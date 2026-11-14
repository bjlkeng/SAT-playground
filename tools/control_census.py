#!/usr/bin/env python3
"""Control census (bead SAT-playground-d96.3): which inputs steer a circuit's muxes.

A CNF written by a circuit encoder (Bitwuzla, AIGER-to-CNF, Tseitin) is read back
as gates:

  AND   o = x AND y           clauses (-o, x), (-o, y), (o, -x, -y), any signs on x, y
  XOR   o = x XOR y           the four ternary clauses of the parity table
  ITE   o = s ? a : b         the four ternary clauses (-s,-a,o), (-s,a,-o), (s,-b,o), (s,b,-o)

A gate's output must be numbered after its inputs (encoders number outputs last);
variables no gate defines are the circuit's inputs.  A mux is an ITE gate, or the AND
form OR(AND(s, a), AND(-s, b)).  For every mux the census follows its select back to
the inputs (the select's support, capped).  An input's weight is the number of muxes
whose select support holds it; the top K inputs by weight are the candidate control
set, and `cover K` is the share of all muxes whose select support lies inside it.

On oisc-subrv-sll-nested-8 eight inputs steer 77% of the 1.9M muxes, and fixing
those eight bits leaves the solver almost nothing to do (solver README, "Reading
the bp, oisc and smt2 cells").  The census asks where else that shape occurs.

Usage:
  python3 tools/control_census.py cell FILE [--cap 200]
  python3 tools/control_census.py census --out TABLE.tsv [--stems a,b] [--workers 6]
                                        [--max-clauses 30000000] [--seconds 1800]
  python3 tools/control_census.py cube FILE --inputs 10,11,12 --value 5 --out CUBE.cnf
  python3 tools/control_census.py selftest

Limits: the select-support walk has no shared memo, so a deep circuit with few inputs
(goldcrest-and-11: 1,408 s; goldcrest-and-14 hit the 30-minute cell limit) walks its
whole cone once per select; a file is read up to --max-clauses (30M by default), so a
bigger one gets a partial row (`truncated` 1).  STP's clause shapes are not read (the
smt2 cell comes out as mostly inputs).

The 2026 suite is not analysed (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import lzma
import os
import resource
import signal
import sys
import time
from collections import Counter, defaultdict
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rl_structure import parse_prefix  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
SUITE = ROOT / "benchmarks" / "sat-comp-2025"
CAP = 200             # select supports larger than this are not followed
KS = (8, 16, 32)      # control-set sizes reported
FIELDS = ["stem", "vars", "clauses", "truncated", "and_gates", "xor_gates", "ite_gates",
          "inputs", "input_frac", "order_bad", "muxes", "selects", "top_select_muxes",
          "small_select_muxes"] + [f"cover{k}" for k in KS] + ["top_inputs", "seconds", "error"]


# ---------------------------------------------------------------------------
# reading
# ---------------------------------------------------------------------------


def read_gates(clauses):
    """(gates, kinds): gates[o] = tuple of input literals (AND: x, y; XOR: x, y;
    ITE: s, a, b); kinds[o] in {'and', 'xor', 'ite'}.  Outputs come after their
    inputs (a candidate whose output is not the largest index is dropped)."""
    binc = set()
    tri = defaultdict(list)
    for c in clauses:
        if len(c) == 2:
            binc.add((c[0], c[1]))
            binc.add((c[1], c[0]))
        elif len(c) == 3:
            tri[frozenset(abs(l) for l in c)].append(c)
    gates, kinds = {}, {}
    # AND: one ternary clause (o, -x, -y) with the two binaries (-o, x), (-o, y)
    for cs in tri.values():
        for c in cs:
            for k in range(3):
                o = c[k]
                if o < 0:
                    continue
                x, y = [-l for j, l in enumerate(c) if j != k]
                if (-o, x) in binc and (-o, y) in binc and o > abs(x) and o > abs(y):
                    if o not in gates:
                        gates[o] = (x, y)
                        kinds[o] = "and"
                    break
    # XOR: the four parity clauses over one variable set
    for vs, cs in tri.items():
        if len(cs) != 4 or len(vs) != 3:
            continue
        odd = {sum(1 for l in c if l < 0) % 2 for c in cs}
        if len(odd) != 1 or len({tuple(sorted(c)) for c in cs}) != 4:
            continue
        o = max(vs)
        if o in gates:
            continue
        x, y = sorted(vs - {o})
        gates[o] = (x, y)
        kinds[o] = "xor"
    # ITE: two clause pairs; a pair on one variable set shares one literal p and has
    # the other two flipped, so -p -> (x <-> y) or (x <-> -y)
    halves = defaultdict(list)
    for vs, cs in tri.items():
        if len(cs) < 2 or len(vs) != 3:
            continue
        for i in range(len(cs)):
            for j in range(i + 1, len(cs)):
                c1, c2 = set(cs[i]), set(cs[j])
                shared = c1 & c2
                if len(shared) != 1:
                    continue
                p = next(iter(shared))
                rest = c1 - shared
                if {-l for l in rest} != c2 - shared:
                    continue
                x, y = sorted(abs(l) for l in rest)
                halves[x].append((-p, y))
                halves[y].append((-p, x))
    for o, hs in halves.items():
        if o in gates:
            continue
        by_sel = defaultdict(dict)
        for g, other in hs:
            by_sel[abs(g)][g > 0] = other
        for s, d in by_sel.items():
            if True in d and False in d:
                a, b = d[True], d[False]
                if s < o and a < o and b < o:
                    gates[o] = (s, a, b)
                    kinds[o] = "ite"
                    break
    return gates, kinds


def mux_selects(gates, kinds):
    """Counter: select variable -> number of muxes it drives (ITE gates, and the AND
    form -AND(-AND(s, a), -AND(-s, b)))."""
    sel = Counter()
    for o, k in kinds.items():
        g = gates[o]
        if k == "ite":
            sel[abs(g[0])] += 1
        elif k == "and":
            x, y = g
            if x < 0 and y < 0 and kinds.get(-x) == "and" and kinds.get(-y) == "and":
                p, q = gates[-x], gates[-y]
                for s in p:
                    if -s in q:
                        sel[abs(s)] += 1
                        break
    return sel


def support(v, gates, cap, memo):
    """Input support of v, or None when it has more than cap inputs."""
    if v in memo:
        return memo[v]
    seen, stack, sup = set(), [v], set()
    while stack:
        u = stack.pop()
        if u in seen:
            continue
        seen.add(u)
        if u in gates:
            for l in gates[u]:
                stack.append(abs(l))
        else:
            sup.add(u)
            if len(sup) > cap:
                memo[v] = None
                return None
    res = frozenset(sup)
    memo[v] = res
    return res


def analyse_clauses(clauses, cap=CAP):
    t0 = time.time()
    gates, kinds = read_gates(clauses)
    used = set()
    for c in clauses:
        for l in c:
            used.add(abs(l))
    inputs = [v for v in used if v not in gates]
    order_bad = sum(1 for o, g in gates.items() if any(abs(l) >= o for l in g))
    sel = mux_selects(gates, kinds)
    memo = {}
    weight = Counter()
    sups = []
    for s, n in sel.items():
        sp = support(s, gates, cap, memo)
        if sp is None:
            continue
        sups.append((sp, n))
        for u in sp:
            weight[u] += n
    kc = Counter(kinds.values())
    total = sum(sel.values())
    row = {
        "and_gates": kc.get("and", 0), "xor_gates": kc.get("xor", 0), "ite_gates": kc.get("ite", 0),
        "inputs": len(inputs), "input_frac": round(len(inputs) / max(1, len(used)), 4),
        "order_bad": order_bad, "muxes": total, "selects": len(sel),
        "top_select_muxes": max(sel.values()) if sel else 0,
        "small_select_muxes": sum(n for _sp, n in sups),
    }
    ranked = [u for u, _w in weight.most_common()]
    for k in KS:
        top = set(ranked[:k])
        covered = sum(n for sp, n in sups if sp <= top)
        row[f"cover{k}"] = round(covered / total, 4) if total else 0.0
    row["top_inputs"] = ",".join(str(u) for u in ranked[:8])
    row["seconds"] = round(time.time() - t0, 1)
    return row, ranked


# ---------------------------------------------------------------------------
# commands
# ---------------------------------------------------------------------------


def _alarm(_sig, _frm):
    raise TimeoutError("cell time limit")


def census_one(job):
    stem, path, max_clauses, seconds, mem_mb = job
    row = {f: "" for f in FIELDS}
    row["stem"] = stem
    t0 = time.time()
    try:
        if mem_mb:
            lim = mem_mb * 1024 * 1024
            resource.setrlimit(resource.RLIMIT_AS, (lim, lim))
        signal.signal(signal.SIGALRM, _alarm)
        signal.alarm(seconds)
        n_vars, n_cls, clauses, truncated = parse_prefix(Path(path), max_clauses)
        row.update({"vars": n_vars, "clauses": n_cls, "truncated": int(truncated)})
        res, _ranked = analyse_clauses(clauses)
        row.update(res)
    except TimeoutError:
        row["error"] = "timeout"
    except MemoryError:
        row["error"] = "memory"
    except Exception as e:  # noqa: BLE001 - a census row records any failure
        row["error"] = f"{type(e).__name__}: {e}"[:200]
    finally:
        signal.alarm(0)     # never leave the alarm armed for this worker's next cell
    row["seconds"] = round(time.time() - t0, 1)
    return row


def cmd_census(a):
    files = sorted(SUITE.glob("*.cnf.xz"))
    if a.stems:
        keys = a.stems.split(",")
        files = [f for f in files if any(k in f.name for k in keys)]
    jobs = [(f.name[: -len(".cnf.xz")], str(f), a.max_clauses, a.seconds, a.mem_mb) for f in files]
    # big files first so the long jobs start early
    jobs.sort(key=lambda j: -os.path.getsize(j[1]))
    out = Path(a.out)
    done = set()
    if out.exists():
        with open(out) as f:
            done = {r["stem"] for r in csv.DictReader(f, delimiter="\t")}
    jobs = [j for j in jobs if j[0] not in done]
    new = not out.exists()
    with open(out, "a", newline="") as fh:
        w = csv.DictWriter(fh, FIELDS, delimiter="\t")
        if new:
            w.writeheader()
        with ProcessPoolExecutor(a.workers) as ex:
            futs = [ex.submit(census_one, j) for j in jobs]
            for fu in as_completed(futs):
                r = fu.result()
                w.writerow(r)
                fh.flush()
                print(f"{r['stem'][:60]:60s} muxes {r['muxes']!s:>8} cover8 {r['cover8']!s:>6} "
                      f"cover32 {r['cover32']!s:>6} {r['seconds']}s {r['error']}", flush=True)


def cmd_cell(a):
    n_vars, n_cls, clauses, truncated = parse_prefix(Path(a.file), a.max_clauses)
    row, ranked = analyse_clauses(clauses, a.cap)
    print(f"vars {n_vars} clauses {n_cls} truncated {truncated}")
    for k in FIELDS:
        if k in row:
            print(f"  {k}: {row[k]}")
    print(f"  top 32 inputs: {ranked[:32]}")


def cmd_cube(a):
    lits = []
    inputs = [int(x) for x in a.inputs.split(",")]
    for i, v in enumerate(inputs):
        lits.append(v if (a.value >> i) & 1 else -v)
    path = Path(a.file)
    with open(path, "rb") as fh:
        xz = fh.read(6) == b"\xfd7zXZ\x00"
    opener = lzma.open if xz else open
    with opener(path, "rt", encoding="ascii", errors="replace") as fin, open(a.out, "w") as fout:
        for line in fin:
            if line.startswith("p "):
                parts = line.split()
                fout.write(f"p cnf {parts[2]} {int(parts[3]) + len(lits)}\n")
                continue
            fout.write(line if line.endswith("\n") else line + "\n")
        for l in lits:
            fout.write(f"{l} 0\n")
    print(f"wrote {a.out}: {len(lits)} unit clauses {lits}")


def cmd_split(a):
    """Solve every assignment of the control inputs as its own run (cube and conquer
    from outside the solver): the file's clauses plus one unit per control input,
    streamed through a named pipe so no cube is written to disk.  The cell is
    unsatisfiable exactly when every case is; the first satisfiable case stops the
    run.  With --ticks every case runs under that work limit (SAT_LIMIT_TICKS, the
    work clock W); with --then, a case that ends without an answer is split again on
    the next group of inputs (groups separated by ';'), as and-nested-11's value 63
    was by hand.  With --group the first level is a choice group instead of bits:
    one case per member set true and one with all of them false.  Results go to OUT/results.tsv (case, status, seconds, conflicts,
    work), a case named by its value at each level joined by '/', e.g. 63/5."""
    import json
    import re
    import subprocess
    import tempfile
    import threading

    def ints(text):
        return [int(x) for x in text.split(",") if x.strip()]

    # the first level is either input bits (every value) or a choice group:
    # one case per member set true, and one with every member false, so the
    # cases cover every assignment whether or not the formula says that one
    # member must be true (if it does, that last case falls at once)
    group = ints(a.group) if a.group else []
    if bool(group) == bool(a.inputs):
        sys.exit("give the first level as --inputs or as --group, not both")
    levels = [ints(a.inputs) if a.inputs else [abs(l) for l in group]] + \
        [ints(g) for g in a.then.split(";") if g.strip()]
    fixed = ints(a.fix) if a.fix else []
    for k, grp in enumerate(levels):
        if not grp:
            sys.exit(f"level {k} has no inputs")
        if len(grp) > 24 and not (k == 0 and (a.sample or group)):
            sys.exit(f"level {k}: {len(grp)} inputs make {1 << len(grp)} cases: "
                     "use fewer inputs (or --sample at the first level)")
    flat = [v for grp in levels for v in grp]
    if any(v <= 0 for v in flat):
        sys.exit("split inputs are variables (positive numbers)")
    if len(set(flat)) != len(flat) or set(flat) & {abs(l) for l in fixed}:
        sys.exit("the split inputs must be distinct across levels and not among --fix")
    n_first = len(group) + 1 if group else 1 << len(levels[0])
    if a.ticks < 0:
        sys.exit("--ticks must be 0 (no limit) or positive")
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    src = Path(a.file).resolve()
    st = src.stat()
    config = {"file": str(src), "size": st.st_size, "mtime_ns": st.st_mtime_ns,
              "inputs": levels[0], "group": group, "then": levels[1:], "fix": fixed, "seed": a.seed,
              "ticks": a.ticks, "solver": str(Path(a.solver).resolve())}
    cfg_path = out / "config.json"
    body = out / "body.cnf"
    res_path = out / "results.tsv"
    if cfg_path.exists():
        old = json.loads(cfg_path.read_text())
        # folders written before --group, --then and --ticks existed: those
        # runs had no group, no deeper level and no work limit
        old.setdefault("group", [])
        old.setdefault("then", [])
        old.setdefault("ticks", 0)
        if old != config:
            sys.exit(f"{out} holds a split of another problem or setting:\n  {old}\n  now {config}\n"
                     "use a fresh --out")
    elif body.exists() or res_path.exists():
        sys.exit(f"{out} has results but no config.json: use a fresh --out")
    else:
        # copy the clause lines as written; count clauses by their terminating 0
        count = 0
        with open(src, "rb") as fh:
            xz = fh.read(6) == b"\xfd7zXZ\x00"
        opener = lzma.open if xz else open
        n_vars = None
        with opener(src, "rt", encoding="ascii", errors="replace") as fin, open(body, "w") as fb:
            for line in fin:
                if not line.strip() or line[0] in "c%":
                    continue
                if line[0] == "p":
                    n_vars = int(line.split()[2])
                    continue
                fb.write(line if line.endswith("\n") else line + "\n")
                count += sum(1 for tok in line.split() if tok == "0")
        if n_vars is None:
            sys.exit(f"{src}: no header line")
        (out / "header.txt").write_text(f"p cnf {n_vars} {count}")
        cfg_path.write_text(json.dumps(config))
    header = (out / "header.txt").read_text().split()
    n_vars = int(header[2])
    base_cls = int(header[3]) + len(fixed)
    if max(flat + [abs(l) for l in fixed]) > n_vars:
        sys.exit(f"an input or fixed literal is past the header's {n_vars} variables")

    def units_of(level, value):
        if level == 0 and group:
            return [group[value]] if value < len(group) else [-l for l in group]
        return [v if (value >> i) & 1 else -v for i, v in enumerate(levels[level])]

    answered = ("UNSATISFIABLE", "SATISFIABLE")
    done = {}
    if res_path.exists():
        for line in res_path.read_text().splitlines():
            q = line.split("\t")
            done[q[0]] = q[1]

    # the work list: (name, units, level); a finished case without an answer is
    # expanded into its sub-cases when a deeper level exists (also on a resume)
    todo = []

    def expand(name, units, level):
        if name not in done:
            todo.append((name, units, level))
        elif done[name] not in answered and level + 1 < len(levels):
            for w in range(1 << len(levels[level + 1])):
                expand(f"{name}/{w}", units + units_of(level + 1, w), level + 1)

    if a.sample:
        import random
        rng = random.Random(a.sample_seed)
        n = min(a.sample, n_first)
        first = sorted(rng.sample(range(n_first), n))
    else:
        first = range(n_first)
    for v in first:
        expand(str(v), units_of(0, v), 0)
    if any(s == "SATISFIABLE" for s in done.values()):
        todo.clear()            # a model was found before: the cell is decided

    cores = []
    for part in a.cores.split(","):
        if "-" in part:
            lo, hi = part.split("-")
            cores += list(range(int(lo), int(hi) + 1))
        else:
            cores.append(int(part))
    cv = threading.Condition()
    stop = threading.Event()
    in_flight = [0]
    tmp = Path(tempfile.mkdtemp(prefix="split-", dir=out))
    mem = a.mem_mb * 1024 * 1024
    env = dict(os.environ)
    env.pop("SAT_LIMIT_TICKS", None)
    if a.ticks:
        env["SAT_LIMIT_TICKS"] = str(a.ticks)

    def limit():
        resource.setrlimit(resource.RLIMIT_AS, (mem, mem))

    def feed(fifo, units):
        tail = "".join(f"{l} 0\n" for l in units + fixed)
        try:
            with open(fifo, "w") as f, open(body) as fb:
                f.write(f"p cnf {n_vars} {base_cls + len(units)}\n")
                while True:
                    chunk = fb.read(1 << 22)
                    if not chunk:
                        break
                    f.write(chunk)
                f.write(tail)
        except BrokenPipeError:
            pass                # the solver stopped reading (it ended early)

    def run_case(core, attempt, units):
        # a fresh pipe for every attempt, so a writer left over from an attempt
        # whose solver never opened its input cannot write into the next one
        fifo = tmp / f"cube{core}_{attempt}.fifo"
        os.mkfifo(fifo, 0o600)
        t0 = time.time()
        th = threading.Thread(target=feed, args=(fifo, units), daemon=True)
        th.start()
        cmd = ["timeout", str(a.timeout), "taskset", "-c", str(core), a.solver, f"--seed={a.seed}", str(fifo)]
        p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                           preexec_fn=limit, env=env)
        if th.is_alive():
            # the writer may be blocked opening the pipe (the solver never opened
            # it) or writing into it: open the read end ourselves and close it, so
            # its open returns and its writes fail with a broken pipe
            try:
                fd = os.open(fifo, os.O_RDONLY | os.O_NONBLOCK)
                os.close(fd)
            except OSError:
                pass
            th.join(timeout=60)
        try:
            os.unlink(fifo)
        except OSError:
            pass
        txt = p.stdout
        m = re.search(r"^s (SATISFIABLE|UNSATISFIABLE|UNKNOWN)", txt, re.M)
        status = m.group(1) if m else f"NONE(rc={p.returncode})"
        m = re.search(r"^c workclock .*?\bconflicts=(\d+).*?\bwork=(\d+)", txt, re.M)
        conflicts, work = (m.group(1), m.group(2)) if m else ("", "")
        return status, time.time() - t0, conflicts, work

    def worker(core):
        attempt = 0
        while True:
            with cv:
                while not todo and in_flight[0] > 0 and not stop.is_set():
                    cv.wait()
                if stop.is_set() or not todo:
                    cv.notify_all()
                    return
                name, units, level = todo.pop(0)
                in_flight[0] += 1
            attempt += 1
            result = None
            try:
                result = run_case(core, attempt, units)
            finally:
                with cv:
                    in_flight[0] -= 1
                    if result is not None:
                        status, secs, conflicts, work = result
                        with open(res_path, "a") as fr:
                            fr.write(f"{name}\t{status}\t{secs:.1f}\t{conflicts}\t{work}\n")
                        done[name] = status
                        if status != "UNSATISFIABLE":
                            print(f"case {name}: {status}", flush=True)
                        if status == "SATISFIABLE":
                            stop.set()
                        elif status not in answered and level + 1 < len(levels):
                            for w in range(1 << len(levels[level + 1])):
                                todo.append((f"{name}/{w}", units + units_of(level + 1, w), level + 1))
                    cv.notify_all()

    threads = [threading.Thread(target=worker, args=(c,)) for c in cores]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    rows = [line.split("\t") for line in res_path.read_text().splitlines()]
    status = {r[0]: r[1] for r in rows}

    def decided(name, level):
        """True when this case is refuted, directly or through all its sub-cases."""
        s = status.get(name)
        if s == "UNSATISFIABLE":
            return True
        if s is None or level + 1 >= len(levels):
            return False
        return all(decided(f"{name}/{w}", level + 1) for w in range(1 << len(levels[level + 1])))

    for k in range(len(levels)):
        at = [r for r in rows if r[0].count("/") == k]
        if not at:
            continue
        works = sorted(int(r[4]) for r in at if r[4])
        conf = sorted(int(r[3]) for r in at if r[3])
        what = f"a group of {len(group)} and none" if k == 0 and group else f"{len(levels[k])} inputs"
        line = f"level {k} ({what}): {len(at)} cases {dict(Counter(r[1] for r in at))}"
        if works:
            line += (f"; work per case min {works[0]}, median {works[len(works) // 2]}, max {works[-1]}, "
                     f"sum {sum(works):.4g}; conflicts max {conf[-1] if conf else ''}")
        print(line)
    total = sum(int(r[4]) for r in rows if r[4])
    no_work = sum(1 for r in rows if not r[4])
    secs = sum(float(r[2]) for r in rows)
    if no_work == len(rows):
        work_txt = "work NA (no case printed a work count)"
    elif no_work:
        work_txt = f"work {total:.4g} over the {len(rows) - no_work} cases with a count ({no_work} without: a partial sum)"
    else:
        work_txt = f"work {total:.4g}"
    print(f"all cases: {len(rows)}, {work_txt}, seconds summed {secs:.0f}")
    if any(r[1] == "SATISFIABLE" for r in rows):
        print("a case is satisfiable: so is the cell")
    elif not a.sample:
        if not all(decided(str(v), 0) for v in first):
            print("not decided: some cases have no answer at the last level")
        elif fixed:
            # the cases cover only the part of the formula --fix selects
            print("every case refuted: the formula is unsatisfiable under the --fix literals "
                  "(the rest of the formula is not covered)")
        else:
            print("every case refuted: the cell is unsatisfiable")


def selftest():
    """A two-bit control input steering a 4-way word mux, built by hand."""
    # inputs: s0=1, s1=2, data a=3..6 (four 1-bit words), plus a free input 7
    # m1 = s0 ? a1 : a0 (8), m2 = s0 ? a3 : a2 (9), out = s1 ? m2 : m1 (10)
    # g = AND(7, 10) (11); unit 11
    def ite(o, s, a, b):
        return [(-s, -a, o), (-s, a, -o), (s, -b, o), (s, b, -o)]
    cl = ite(8, 1, 4, 3) + ite(9, 1, 6, 5) + ite(10, 2, 9, 8)
    cl += [(-11, 7), (-11, 10), (11, -7, -10), (11,)]
    row, ranked = analyse_clauses(cl)
    assert row["ite_gates"] == 3 and row["and_gates"] == 1, row
    assert row["inputs"] == 7, row
    assert row["muxes"] == 3 and row["cover8"] == 1.0, row
    assert set(ranked[:2]) == {1, 2}, ranked
    # a XOR gate is read as a gate, not a mux
    xr = [(-12, 1, 2), (-12, -1, -2), (12, -1, 2), (12, 1, -2)]
    row2, _ = analyse_clauses(xr)
    assert row2["xor_gates"] == 1 and row2["muxes"] == 0, row2
    print("selftest ok")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("census")
    c.add_argument("--out", required=True)
    c.add_argument("--stems")
    c.add_argument("--workers", type=int, default=6)
    c.add_argument("--max-clauses", type=int, default=30_000_000)
    c.add_argument("--seconds", type=int, default=1800)
    c.add_argument("--mem-mb", type=int, default=40000)
    o = sub.add_parser("cell")
    o.add_argument("file")
    o.add_argument("--cap", type=int, default=CAP)
    o.add_argument("--max-clauses", type=int, default=100_000_000)
    u = sub.add_parser("cube")
    u.add_argument("file")
    u.add_argument("--inputs", required=True)
    u.add_argument("--value", type=int, required=True)
    u.add_argument("--out", required=True)
    s = sub.add_parser("split")
    s.add_argument("file")
    s.add_argument("--inputs", default="", help="the first level's input bits (every value is a case)")
    s.add_argument("--group", default="",
                   help="the first level as a choice group: one case per member literal set true, "
                        "plus the case with every member false")
    s.add_argument("--out", required=True, help="a directory: body.cnf, header.txt, results.tsv")
    s.add_argument("--solver", required=True)
    s.add_argument("--cores", default="0-15")
    s.add_argument("--timeout", type=int, default=600)
    s.add_argument("--seed", type=int, default=0)
    s.add_argument("--sample", type=int, default=0, help="solve only this many random values")
    s.add_argument("--fix", default="", help="literals added as units to every cube (a sub-split)")
    s.add_argument("--mem-mb", type=int, default=16000, help="address-space limit per solver run")
    s.add_argument("--sample-seed", type=int, default=1)
    s.add_argument("--ticks", type=int, default=0,
                   help="work limit per case (SAT_LIMIT_TICKS, the work clock W); 0 = none")
    s.add_argument("--then", default="",
                   help="inputs to split a case on again when it ends without an answer; levels separated by ';'")
    sub.add_parser("selftest")
    a = ap.parse_args(argv)
    if a.cmd == "census":
        cmd_census(a)
    elif a.cmd == "cell":
        cmd_cell(a)
    elif a.cmd == "cube":
        cmd_cube(a)
    elif a.cmd == "split":
        cmd_split(a)
    else:
        selftest()


if __name__ == "__main__":
    main()
