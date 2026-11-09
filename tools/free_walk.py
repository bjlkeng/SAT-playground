#!/usr/bin/env python3
"""Free-choice walk (bead SAT-playground-d96.2.2, G.8.2): a prototype search
over a formula's free choices, and the runs that compare it with the solver.

The search is tools/free-walk (Rust, standalone, no solver code; `cargo build
--release` in that folder).  This script gives it the role reading of
tools/free_census.py and checks every model it finds against the original
formula with tools/verify_sat.py, the gate's checker.

  export CNF OUT.fwr        write the reading for the walker (format below)
  walk CNF [--seed N] [--seconds S] [--keep FILE] [--walker-args '...']
                            export to a scratch file, walk, check the model
  run --out DIR [--cells REGEX ...] [--seeds 0,1,2] [--seconds 1800]
      [--jobs 32] [--export-jobs 8] [--mem-mb 16000] [--walker-args '...']
                            the prototype arm on 2025 cells: every reading once
                            (timed), then one walk per cell and seed on its own
                            core, then every model checked; DIR/results.tsv
  report --walk RESULTS.tsv --arm NAME=RESULTS.tsv ... [--readings DIR] [--tsv OUT]
                            the per-cell table over the arms (markdown; the
                            cells all arms ran), and with --tsv one row per
                            cell, arm and seed
  selftest                  toys: the walker's computed values against the
                            census's own evaluation, and a model found and
                            checked on every satisfiable toy

The reading file is FWR1 and then 32-bit little-endian integers: nv; the
units (count, then variable and value pairs); the groups (count, then per
group: none_ok, size, member literals); the free variables (count,
variables); the computed variables in computing order (count, then per
variable a kind and its data: 4 an OR: output literal, count, input literals,
the output literal true when an input is; 5 a gate: variable, support size,
support variables, truth table indexed by the support values with the first
support the lowest bit; 6 a helper: helper literal, count of its clauses,
then per clause the count and its other literals, the helper literal true
when one of its clauses has every other literal false); the constraint
clauses (count, then per clause the count and its literals).

The 2026 suite is not read (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import json
import lzma
import os
import queue
import random
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
from array import array
from concurrent.futures import ProcessPoolExecutor, ThreadPoolExecutor, as_completed
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOLS))
import free_census as fc  # noqa: E402

ROOT = TOOLS.parent
SUITE = ROOT / "benchmarks" / "sat-comp-2025"
WALKER = TOOLS / "free-walk" / "target" / "release" / "free-walk"
VERIFY_SAT = TOOLS / "verify_sat.py"
MAGIC = b"FWR1"
# the cells of the bead: lockchart groups 1 and 2, oddball tto, and the two mp1
# circuits the census named (timetable's counters read as one-way helpers, not
# definitions, so by the bead's own condition it is not a target)
TARGETS = (r"lockchart-group[12]-", r"oddball_\d+_5_tto", r"mp1-Nb7T4[56]$")
RESULT_COLUMNS = ["stem", "seed", "result", "search_s", "wall_s", "read_s", "steps", "trials", "flips",
                  "best", "start_false", "verified", "note"]


# ---------------------------------------------------------------------------
# the reading
# ---------------------------------------------------------------------------

def reading_array(fixed, rd):
    """The reading of read_roles() as the walker's integers (after the magic)."""
    red = rd.cnf
    lits, starts = red.lits, red.starts
    out = array("i")
    out.append(red.nv)
    out.append(len(fixed))
    for v in sorted(fixed):
        out.append(v)
        out.append(1 if fixed[v] else 0)
    out.append(len(rd.groups))
    for mem, none_ok in zip(rd.groups, rd.none_ok):
        out.append(1 if none_ok else 0)
        out.append(len(mem))
        out.extend(mem)
    free = [v for v in rd.order if rd.role[v] == fc.FREE]
    out.append(len(free))
    out.extend(free)
    nodes = [v for v in rd.order if rd.role[v] in (fc.DEF, fc.GATE, fc.HELPER)]
    out.append(len(nodes))
    for v in nodes:
        i = rd.acc[v]
        r = rd.role[v]
        if r == fc.DEF:
            c, x = rd.c_a[i], rd.c_b[i]
            ins = [y for y in lits[starts[c]:starts[c + 1]] if y != x]
            out.append(4)
            out.append(-x)                    # -x is the OR of the clause's other literals
            out.append(len(ins))
            out.extend(ins)
        elif r == fc.GATE:
            sup, tt, _cl = rd.gates[rd.c_a[i]]
            out.append(5)
            out.append(v)
            out.append(len(sup))
            out.extend(sup)
            out.append(tt)
        else:
            y = rd.c_a[i]
            occ = red.occs(y)
            out.append(6)
            out.append(y)
            out.append(len(occ))
            for c in occ:
                cl = [z for z in lits[starts[c]:starts[c + 1]] if z != y]
                out.append(len(cl))
                out.extend(cl)
    cons = [c for c in range(red.m) if not rd.cover[c]]
    out.append(len(cons))
    for c in cons:
        out.append(starts[c + 1] - starts[c])
        out.extend(lits[starts[c]:starts[c + 1]])
    return out


def write_reading(path, data):
    if sys.byteorder != "little":
        data.byteswap()
    with open(path, "wb") as f:
        f.write(MAGIC)
        data.tofile(f)


def export(cnf_path, out_path, deadline=None):
    """Read the cell, write its reading; returns the timings and sizes."""
    if "sat-comp-2026" in str(Path(cnf_path).resolve()):
        raise SystemExit("the 2026 suite is the holdout and is not read")
    t0 = time.time()
    cnf, truncated = fc.read_cnf(cnf_path)
    if truncated:
        raise SystemExit(f"{cnf_path}: more clauses than the census reads whole; the walk needs the whole formula")
    nclauses = cnf.m
    t1 = time.time()
    fixed, rd, _nsat = fc.read_roles(cnf, deadline)
    t2 = time.time()
    info = {"cnf": str(cnf_path), "clauses": nclauses, "parse_s": round(t1 - t0, 1), "roles_s": round(t2 - t1, 1)}
    if rd is None:
        info.update(refuted=True, read_s=round(time.time() - t0, 1))
        return info
    data = reading_array(fixed, rd)
    write_reading(out_path, data)
    t3 = time.time()
    s = rd.summary()
    info.update(refuted=False, write_s=round(t3 - t2, 1), read_s=round(t3 - t0, 1), fixed=len(fixed),
                groups=s["groups"], amo_groups=s["amo_groups"], free=s["free"], defs=s["defs"], gates=s["gates"],
                helpers=s["helpers"], constraints=s["constraints"], bits=s["bits"], stages=rd.times)
    return info


def _export_job(cnf_path, out_path):
    try:
        return export(cnf_path, out_path)
    except BaseException as e:            # noqa: BLE001 (a worker's failure is a row, not a crash)
        return {"cnf": cnf_path, "error": f"{type(e).__name__}: {e}"}


# ---------------------------------------------------------------------------
# one walk
# ---------------------------------------------------------------------------

def parse_walk(stdout):
    """(result, stats) from the walker's output."""
    res = next((ln.split()[1] for ln in stdout.splitlines() if ln.startswith("s ")), None)
    stats = {}
    for ln in stdout.splitlines():
        if ln.startswith("c free-walk: result="):
            for kv in ln[len("c free-walk: "):].split():
                k, _, v = kv.partition("=")
                stats[k] = v
        m = re.search(r"(\d+) clauses false at the start", ln)
        if m:
            stats["start_false"] = m.group(1)
    return res, stats


def walk_cmd(reading, seed, seconds, extra=()):
    return [str(WALKER), str(reading), "--seed", str(seed), "--seconds", str(seconds), "--progress", "60", *extra]


def plain_cnf(cnf_path, scratch):
    """A plain-text copy of an .xz cell for the checker (verify_sat.py reads plain or gz)."""
    p = Path(cnf_path)
    with open(p, "rb") as fh:
        xz = fh.read(6) == b"\xfd7zXZ\x00"
    if not xz:
        return p
    dst = Path(scratch) / (p.name[:-3] if p.name.endswith(".xz") else p.name + ".cnf")
    if not dst.exists():
        with lzma.open(p, "rb") as src, open(dst, "wb") as out:
            shutil.copyfileobj(src, out, 1 << 24)
    return dst


def verify(cnf_plain, out_file, timeout=3600):
    """ok / FAIL / checker-timeout, by tools/verify_sat.py on the original formula."""
    try:
        p = subprocess.run([sys.executable, "-I", str(VERIFY_SAT), str(cnf_plain), str(out_file)],
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return "checker-timeout"
    return "ok" if p.returncode == 0 and "VERIFIED" in p.stdout else "FAIL"


def cmd_export(args):
    info = export(args.cnf, args.out)
    print(json.dumps(info))
    return 0


def cmd_walk(args):
    with tempfile.TemporaryDirectory() as tmp:
        reading = args.keep or os.path.join(tmp, "reading.fwr")
        if not (args.keep and os.path.exists(args.keep)):
            info = export(args.cnf, reading)
            print("c reading:", json.dumps(info), flush=True)
            if info.get("refuted"):
                print("s UNSATISFIABLE (unit propagation refutes it)")
                return 0
        cmd = walk_cmd(reading, args.seed, args.seconds, shlex.split(args.walker_args))
        t0 = time.time()
        p = subprocess.run(cmd, stdout=subprocess.PIPE, text=True)
        wall = time.time() - t0
        res, st = parse_walk(p.stdout)
        print("\n".join(ln for ln in p.stdout.splitlines() if ln.startswith("c ")))
        print(f"c walk wall {wall:.2f} s, exit {p.returncode}")
        if res == "SATISFIABLE":
            out = os.path.join(tmp, "model.out")
            with open(out, "w") as fh:
                fh.write(p.stdout)
            ver = verify(plain_cnf(args.cnf, tmp), out)
            print(f"s SATISFIABLE (model check: {ver})")
            return 0 if ver == "ok" else 1
        print(f"s {res}")
        return 0


# ---------------------------------------------------------------------------
# the prototype arm
# ---------------------------------------------------------------------------

def target_cells(patterns):
    stems = sorted(p.name[:-len(".cnf.xz")] for p in SUITE.glob("*.cnf.xz"))
    pats = [re.compile(x) for x in (patterns or TARGETS)]
    return [s for s in stems if any(p.search(s) for p in pats)]


def numa_cores(jobs):
    """The harness's core order (tools/feature_ablation.py numa_balanced_cores)."""
    import feature_ablation as fa  # noqa: E402  (imported late: it reads its own globals at import)
    return fa.numa_balanced_cores(jobs)


def _limit_memory(mb):
    import resource
    b = mb * 1024 * 1024
    resource.setrlimit(resource.RLIMIT_AS, (b, b))


def cmd_run(args):
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    if not WALKER.exists():
        raise SystemExit(f"build the walker first: cargo build --release in {WALKER.parents[2]}")
    cells = target_cells(args.cells)
    seeds = [int(x) for x in args.seeds.split(",")]
    (out / "cells.txt").write_text("\n".join(cells) + "\n")
    # the settings every walk in the folder shares: a folder is resumed (more seeds or cells,
    # or the walks a stop cut short) only with the same ones
    settings = {"seconds": args.seconds, "walker_args": shlex.split(args.walker_args), "mem_mb": args.mem_mb}
    sfile = out / "settings.json"
    if sfile.exists():
        old = json.loads(sfile.read_text())
        if old != settings:
            raise SystemExit(f"{out} holds walks with other settings ({old}, now {settings}); use a new --out")
    else:
        sfile.write_text(json.dumps(settings))
    walker_copy = out / "free-walk"
    if not walker_copy.exists():
        shutil.copy2(WALKER, walker_copy)           # the binary of record, frozen for the run
    print(f"[run] {len(cells)} cells x {len(seeds)} seeds, {args.seconds} s each, {args.jobs} cores; "
          f"walker {walker_copy}", flush=True)
    rdir = out / "readings"
    rdir.mkdir(exist_ok=True)
    infos = {}
    todo = []
    for s in cells:
        js = rdir / f"{s}.json"
        if js.exists():
            infos[s] = json.loads(js.read_text())
        else:
            todo.append(s)
    if todo:
        print(f"[run] reading {len(todo)} cells, {args.export_jobs} at a time", flush=True)
        with ProcessPoolExecutor(args.export_jobs, initializer=_limit_memory, initargs=(args.mem_mb,)) as ex:
            futs = {ex.submit(_export_job, str(SUITE / f"{s}.cnf.xz"), str(rdir / f"{s}.fwr")): s for s in todo}
            for f in as_completed(futs):
                s = futs[f]
                info = f.result()
                infos[s] = info
                (rdir / f"{s}.json").write_text(json.dumps(info))
                print(f"[read] {s[:60]}: {info.get('error') or str(info.get('read_s')) + ' s'}", flush=True)
    rdir_runs = out / "runs"
    rdir_runs.mkdir(exist_ok=True)
    jobs = []
    for seed in seeds:
        for s in cells:
            js = rdir_runs / f"{s}.s{seed}.json"
            if infos[s].get("error") or infos[s].get("refuted"):
                # no reading, so no walk: the cell stays in the results as a failure row
                failed = "READ_ERROR" if infos[s].get("error") else "REFUTED"
                js.write_text(json.dumps({"stem": s, "seed": seed, "result": failed, "search_s": "NA", "wall_s": "NA",
                                          "read_s": infos[s].get("read_s", "NA"), "steps": "NA", "trials": "NA",
                                          "flips": "NA", "best": "NA", "start_false": "NA", "verified": "skip",
                                          "note": infos[s].get("error") or "unit propagation refutes it"}))
                continue
            if not js.exists():
                jobs.append((s, seed))
    random.Random(1).shuffle(jobs)            # the big and small cells spread over the run
    cores = numa_cores(args.jobs)
    pool = queue.Queue()
    for c in cores:
        pool.put(c)
    extra = shlex.split(args.walker_args)
    print(f"[run] {len(jobs)} walks on cores {cores}", flush=True)

    def one(job):
        s, seed = job
        core = pool.get()
        try:
            argv = [str(walker_copy), str(rdir / f"{s}.fwr"), "--seed", str(seed), "--seconds", str(args.seconds),
                    "--progress", "60", *extra]
            cmd = ["taskset", "-c", str(core), "bash", "-c",
                   f"ulimit -v {args.mem_mb * 1024}; exec timeout {args.seconds + 120} "
                   + " ".join(shlex.quote(a) for a in argv)]
            t0 = time.time()
            with open(rdir_runs / f"{s}.s{seed}.out", "w") as fo, open(rdir_runs / f"{s}.s{seed}.err", "w") as fe:
                p = subprocess.run(cmd, stdout=fo, stderr=fe)
            wall = time.time() - t0
        finally:
            pool.put(core)
        text = (rdir_runs / f"{s}.s{seed}.out").read_text()
        res, st = parse_walk(text)
        row = {"stem": s, "seed": seed, "result": {"SATISFIABLE": "SAT", "UNKNOWN": "UNKNOWN"}.get(res, f"ERROR_rc{p.returncode}"),
               "search_s": st.get("search_seconds", "NA"), "wall_s": round(wall, 2), "read_s": infos[s].get("read_s"),
               "steps": st.get("steps", "NA"), "trials": st.get("trials", "NA"), "flips": st.get("flips", "NA"),
               "best": st.get("best", "NA"), "start_false": st.get("start_false", "NA"), "verified": "pending", "note": ""}
        (rdir_runs / f"{s}.s{seed}.json").write_text(json.dumps(row))
        print(f"[walk] {s[:56]} s{seed} core {core}: {row['result']} {row['search_s']} s best {row['best']}", flush=True)
        return row

    with ThreadPoolExecutor(len(cores)) as ex:
        list(ex.map(one, jobs))
    # every model, checked after the walks (off the timed path)
    scratch = out / "cnf-scratch"
    scratch.mkdir(exist_ok=True)
    rows = []
    for s in cells:
        for seed in seeds:
            js = rdir_runs / f"{s}.s{seed}.json"
            if not js.exists():
                continue
            row = json.loads(js.read_text())
            if row["result"] == "SAT" and row["verified"] == "pending":
                row["verified"] = verify(plain_cnf(SUITE / f"{s}.cnf.xz", scratch), rdir_runs / f"{s}.s{seed}.out")
                js.write_text(json.dumps(row))
                print(f"[check] {s[:56]} s{seed}: {row['verified']}", flush=True)
            elif row["result"] != "SAT":
                row["verified"] = "skip"
            rows.append(row)
        for f in scratch.glob("*"):
            f.unlink()
    with open(out / "results.tsv", "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=RESULT_COLUMNS, delimiter="\t", extrasaction="ignore")
        w.writeheader()
        for r in rows:
            w.writerow(r)
    bad = [r for r in rows if r["verified"] not in ("ok", "skip")]
    unread = sorted({r["stem"] for r in rows if r["result"] == "READ_ERROR"})
    # a walk that ended without its status line (a crash, a bad option, a kill) is a failure, not a miss
    broken = [r for r in rows if r["result"] not in ("SAT", "UNKNOWN", "READ_ERROR", "REFUTED")]
    walks = sum(1 for r in rows if r["result"] not in ("READ_ERROR", "REFUTED"))
    print(f"[run] DONE {sum(r['result'] == 'SAT' for r in rows)} models of {walks} walks; "
          f"check failures {len(bad)}; walks that failed {len(broken)}; cells not read {len(unread)} -> "
          f"{out / 'results.tsv'}", flush=True)
    for s in unread:
        print(f"[run] NOT READ: {s}: {infos[s].get('error')}", flush=True)
    for r in broken:
        print(f"[run] WALK FAILED: {r['stem']} seed {r['seed']}: {r['result']} (see runs/*.err)", flush=True)
    return 1 if bad or unread or broken else 0


# ---------------------------------------------------------------------------
# the table over the arms
# ---------------------------------------------------------------------------

def _family(stem):
    s = stem[33:] if re.match(r"[0-9a-f]{32}-", stem) else stem
    for pat, name in ((r"lockchart-group1", "lockchart group 1"), (r"lockchart-group2", "lockchart group 2"),
                      (r"oddball_.*_tto", "oddball tto"), (r"mp1-", "mp1")):
        if re.search(pat, s):
            return name
    return "other"


def _short(stem):
    s = stem[33:] if re.match(r"[0-9a-f]{32}-", stem) else stem
    s = s.replace(".normalised", "").replace("lockchart-group1-", "g1 ").replace("lockchart-group2-rnd0.3-", "g2 ")
    return s.replace("-p8d4j1", "").replace("-P8D4J1", "").replace("_tto_zp", "")


def load_arm(path, walk=False):
    """{(stem, seed): (seconds, verdict) for a model, None for no model}.  A
    model counts when its check says ok; one the harness's checker gave up on
    (checker-timeout: verify_sat.py takes longer than its 600 s on the oddball
    formulas) counts too and is marked; a model whose check failed does not."""
    out = {}
    with open(path) as fh:
        for r in csv.DictReader(fh, delimiter="\t"):
            if walk:
                stem, seed, sat, t = r["stem"], int(r["seed"]), r["result"] == "SAT", r["search_s"]
            else:
                stem, seed, sat, t = r["instance"], int(r["seed"]), r["result"] == "SATISFIABLE", r["time_s"]
            ver = r.get("verified", "")
            if walk and r["result"] not in ("SAT", "UNKNOWN"):
                print(f"WARNING: {path}: {stem} seed {seed}: the walk ended {r['result']!r} ({r.get('note', '')}); "
                      "shown as no model", file=sys.stderr)
            if sat and ver not in ("ok", "checker-timeout"):
                print(f"WARNING: {path}: {stem} seed {seed}: a model whose check says {ver!r}; not counted",
                      file=sys.stderr)
            out[(stem, seed)] = (float(t), ver) if sat and ver in ("ok", "checker-timeout") else None
    return out


def cmd_report(args):
    arms = [("prototype", load_arm(args.walk, walk=True))]
    for spec in args.arm or []:
        name, _, path = spec.partition("=")
        arms.append((name, load_arm(path)))
    reads = {}
    if args.readings:
        for js in Path(args.readings).glob("*.json"):
            info = json.loads(js.read_text())
            reads[js.name[:-5]] = info.get("read_s")
    stems = sorted(set.intersection(*({k[0] for k in a} for _n, a in arms)), key=lambda s: (_family(s), _short(s)))
    seeds = sorted({k[1] for _n, a in arms for k in a})

    def cell(v):
        if v is None:
            return "—"
        t, ver = v
        txt = f"{t:.0f}" if t >= 10 else f"{t:.1f}"
        return txt + ("*" if ver != "ok" else "")

    head = "| cell | read s | " + " | ".join(f"{n} ({'/'.join(map(str, seeds))})" for n, _a in arms) + " |"
    print(head)
    print("|" + "---|" * (2 + len(arms)))
    for s in stems:
        parts = []
        for _n, a in arms:
            parts.append(" / ".join(cell(a.get((s, sd))) if (s, sd) in a else "·" for sd in seeds))
        rs = reads.get(s)
        print(f"| `{_short(s)}` | {rs if rs is not None else '·'} | " + " | ".join(parts) + " |")
    print()
    fams = sorted({_family(s) for s in stems})
    print("| family | cells | " + " | ".join(f"{n}: runs with a model, cells ever" for n, _a in arms) + " |")
    print("|" + "---|" * (2 + len(arms)))
    for f in fams + ["all"]:
        fs = [s for s in stems if f == "all" or _family(s) == f]
        parts = []
        for _n, a in arms:
            runs = sum(1 for s in fs for sd in seeds if a.get((s, sd)) is not None)
            tot = sum(1 for s in fs for sd in seeds if (s, sd) in a)
            ever = sum(1 for s in fs if any(a.get((s, sd)) is not None for sd in seeds))
            parts.append(f"{runs}/{tot}, {ever}")
        print(f"| {f} | {len(fs)} | " + " | ".join(parts) + " |")
    if args.tsv:
        with open(args.tsv, "w", newline="") as fh:
            w = csv.writer(fh, delimiter="\t", lineterminator="\n")
            w.writerow(["stem", "family", "read_s", "arm", "seed", "model_s", "check"])
            for s in stems:
                for n, a in arms:
                    for sd in seeds:
                        if (s, sd) not in a:
                            continue
                        v = a[(s, sd)]
                        w.writerow([s, _family(s), reads.get(s, ""), n, sd, "" if v is None else f"{v[0]:.1f}",
                                    "" if v is None else v[1]])
    shown = set(stems)
    marked = sum(1 for _n, a in arms for k, v in a.items() if k[0] in shown and v is not None and v[1] != "ok")
    if marked:
        print(f"\n`*`: {marked} models the harness's checker gave up on (checker-timeout), counted as found.")
    return 0


# ---------------------------------------------------------------------------
# self-test
# ---------------------------------------------------------------------------

def _walk_toy(nv, cl, tmp, name, extra):
    """Export the toy, walk it; (result, stats, assignment {var: bool} or None, rd, fixed)."""
    path = os.path.join(tmp, name + ".cnf")
    with open(path, "w") as fh:
        fh.write(f"p cnf {nv} {len(cl)}\n")
        for c in cl:
            fh.write(" ".join(map(str, c)) + " 0\n")
    cnf, _t = fc.read_cnf(path)
    fixed, rd, _ = fc.read_roles(cnf)
    reading = os.path.join(tmp, name + ".fwr")
    write_reading(reading, reading_array(fixed, rd))
    p = subprocess.run([str(WALKER), reading, *extra], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    res, st = parse_walk(p.stdout)
    assign = None
    if any(ln.startswith("v") for ln in p.stdout.splitlines()):
        assign = {}
        for ln in p.stdout.splitlines():
            if ln.startswith("v"):
                for t in ln.split()[1:]:
                    z = int(t)
                    if z:
                        assign[abs(z)] = z > 0
    with open(os.path.join(tmp, name + ".out"), "w") as fh:
        fh.write(p.stdout)
    return res, st, assign, rd, fixed, path, p


def _agrees(rd, fixed, assign):
    """Does the walker's assignment equal the census's evaluation from the
    same choices (the true member per group, the free values)?"""
    idx = []
    for mem, none_ok in zip(rd.groups, rd.none_ok):
        t = [j for j, x in enumerate(mem) if assign[abs(x)] == (x > 0)]
        if len(t) == 0 and none_ok:
            t = [len(mem)]
        if len(t) != 1:
            return False, "a group without exactly one true member"
        idx.append(t[0])
    free = {v: assign[v] for v in rd.order if rd.role[v] == fc.FREE}
    ref = rd.full_assignment(idx, free)
    for v, b in ref.items():
        if assign[v] != b:
            return False, f"variable {v} ({fc.ROLE_NAMES[rd.role[v]]}): walker {assign[v]}, census {b}"
    for v, b in fixed.items():
        if assign[v] != b:
            return False, f"unit {v}"
    return True, ""


def cmd_selftest(_args):
    if not WALKER.exists():
        raise SystemExit(f"build the walker first: cargo build --release in {WALKER.parents[2]}")
    ok = True
    toys = [
        ("tseitin", fc.toy_tseitin()[:2], True),
        ("tseitin-one-way", fc.toy_tseitin(one_way=True, seed=6)[:2], True),
        ("csp-pairwise", fc.toy_csp()[:2], True),
        ("csp-ladder", fc.toy_csp(amo="ladder", seed=7)[:2], True),
        ("lockchart", fc.toy_lockchart()[:2], True),
        ("lockchart-big", fc.toy_lockchart(n_keys=24, n_locks=20, pins=6, depths=4, seed=11)[:2], True),
        ("guarded", fc.toy_guarded()[:2], True),
        ("cycle", fc.toy_cycle(), True),
        ("random3-easy", fc.toy_random3(n=120, ratio=3.0, seed=9), True),
    ]
    with tempfile.TemporaryDirectory() as tmp:
        for name, (nv, cl), sat in toys:
            # the start: the walker's values against the census's evaluation, over a few seeds
            agree = True
            why = ""
            for seed in range(5):
                res, st, assign, rd, fixed, _path, p = _walk_toy(nv, cl, tmp, name, ["--seed", str(seed), "--steps", "0", "--dump"])
                if assign is None or rd is None:
                    agree, why = False, f"no assignment (exit {p.returncode}: {p.stderr.strip()[:200]})"
                    break
                a, why = _agrees(rd, fixed, assign)
                if not a:
                    agree = False
                    break
            ok &= fc._check(f"{name}: the walker computes every variable as the census does", agree, why)
            # the search, with the state checked against a recomputation every step
            res, st, assign, rd, fixed, path, p = _walk_toy(nv, cl, tmp, name, ["--seed", "1", "--seconds", "20", "--check", "1"])
            if sat:
                found = res == "SATISFIABLE" and p.returncode == 10
                ok &= fc._check(f"{name}: a model found (state checked every step)", found,
                                f"{res} rc {p.returncode} {p.stderr.strip()[-300:]}")
                if found:
                    ver = verify(path, os.path.join(tmp, name + ".out"), timeout=120)
                    ok &= fc._check(f"{name}: verify_sat.py accepts it", ver == "ok", ver)
                    ok &= fc._check(f"{name}: the model agrees with the census's evaluation", _agrees(rd, fixed, assign)[0])
        # clause weights: the weighted cost checked against a recomputation every step
        nv, cl = fc.toy_lockchart(n_keys=24, n_locks=20, pins=6, depths=4, seed=11)[:2]
        for wargs in (["--weights", "clause", "--smooth", "5"], ["--weights", "all", "--smooth", "3"]):
            res, st, assign, rd, fixed, path, p = _walk_toy(nv, cl, tmp, "weights", ["--seed", "2", "--seconds", "20", "--check", "1",
                                                                                     "--noise", "0.2", *wargs])
            found = res == "SATISFIABLE" and p.returncode == 10
            ok &= fc._check(f"weights {' '.join(wargs[1:])}: a model found, the weighted state checked every step", found,
                            f"{res} rc {p.returncode} {p.stderr.strip()[-300:]}")
            if found:
                ok &= fc._check(f"weights {' '.join(wargs[1:])}: verify_sat.py accepts it",
                                verify(path, os.path.join(tmp, "weights.out"), timeout=120) == "ok")
        # a move's break: switching a group from value a to b leaves (x_a | x_b) true, though it
        # is false midway (x_a cleared before x_b is set); every trial's break is checked against
        # the clauses true before and false after.  Six 4-valued groups, each limited to two values
        # by such a clause, with nogoods between neighbours that spare a planted solution.
        rng = random.Random(13)
        xs = [[4 * i + a + 1 for a in range(4)] for i in range(6)]
        plant = [rng.randrange(4) for _ in range(6)]
        cl = []
        for r in xs:
            cl.append(tuple(r))
            cl += [(-r[a], -r[b]) for a in range(4) for b in range(a + 1, 4)]
        for i, r in enumerate(xs):
            other = rng.choice([a for a in range(4) if a != plant[i]])
            cl.append((r[plant[i]], r[other]))
            j = (i + 1) % 6
            for a in range(4):
                for b in range(4):
                    if (a, b) != (plant[i], plant[j]) and rng.random() < 0.3:
                        cl.append((-r[a], -xs[j][b]))
        res, st, assign, rd, fixed, path, p = _walk_toy(24, cl, tmp, "break", ["--seed", "3", "--seconds", "20", "--check", "1",
                                                                             "--score", "break", "--weights", "none"])
        same = 0
        if rd is not None:
            gid = {abs(x): g for g, mem in enumerate(rd.groups) for x in mem}
            red = rd.cnf
            for c in range(red.m):
                lits = red.clause(c)
                if not rd.cover[c] and len(lits) == 2 and all(z > 0 for z in lits) and gid.get(abs(lits[0])) == gid.get(abs(lits[1])) is not None:
                    same += 1
        ok &= fc._check("the break toy has constraint clauses over two members of one group", same == 6, str(same))
        ok &= fc._check("break scoring: every trial's break as the clauses true before and false after, a model found",
                        res == "SATISFIABLE" and p.returncode == 10, f"{res} rc {p.returncode} {p.stderr.strip()[-300:]}")
        # a toy the walk cannot satisfy: four pigeons, three holes (groups and their nogoods, no unit)
        x = [[3 * i + a + 1 for a in range(3)] for i in range(4)]
        cl = [tuple(r) for r in x] + [(-r[a], -r[b]) for r in x for a in range(3) for b in range(a + 1, 3)]
        cl += [(-x[i][a], -x[j][a]) for i in range(4) for j in range(i + 1, 4) for a in range(3)]
        res, st, assign, rd, fixed, path, p = _walk_toy(12, cl, tmp, "unsat", ["--seed", "1", "--steps", "20000"])
        ok &= fc._check("an unsatisfiable toy: UNKNOWN at the step limit, no model", res == "UNKNOWN" and p.returncode == 0,
                        f"{res} rc {p.returncode}")
        # a corrupt reading is refused
        bad = os.path.join(tmp, "bad.fwr")
        with open(bad, "wb") as fh:
            fh.write(MAGIC + array("i", [3, 0, 1, 0, 2, 1, 5]).tobytes())
        p = subprocess.run([str(WALKER), bad], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        ok &= fc._check("a group member out of range is refused", p.returncode == 2 and "out of range" in p.stderr, p.stderr)
        bad2 = os.path.join(tmp, "bad2.fwr")
        # nv 2, no units, no groups, no free, one OR node reading variable 2 before it has a role
        with open(bad2, "wb") as fh:
            fh.write(MAGIC + array("i", [2, 0, 0, 0, 1, 4, 1, 1, 2, 0]).tobytes())
        p = subprocess.run([str(WALKER), bad2], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        ok &= fc._check("a node that reads an unsettled variable is refused", p.returncode == 2 and "before it is settled" in p.stderr,
                        p.stderr)
    print("all ok" if ok else "SOME CHECKS FAILED")
    return 0 if ok else 1


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("export")
    p.add_argument("cnf")
    p.add_argument("out")
    p = sub.add_parser("walk")
    p.add_argument("cnf")
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--seconds", type=float, default=60)
    p.add_argument("--keep", default=None, help="the reading file: written if missing, reused if present")
    p.add_argument("--walker-args", default="", help="more walker options, one string")
    p = sub.add_parser("run")
    p.add_argument("--out", required=True)
    p.add_argument("--cells", nargs="*", default=None, help="regexes over the 2025 stems (default: the bead's cells)")
    p.add_argument("--seeds", default="0,1,2")
    p.add_argument("--seconds", type=int, default=1800)
    p.add_argument("--jobs", type=int, default=32)
    p.add_argument("--export-jobs", type=int, default=8)
    p.add_argument("--mem-mb", type=int, default=16000)
    p.add_argument("--walker-args", default="")
    p = sub.add_parser("report")
    p.add_argument("--walk", required=True)
    p.add_argument("--arm", action="append")
    p.add_argument("--readings", default=None, help="the run's readings folder: the read time per cell")
    p.add_argument("--tsv", default=None, help="also write the table, one row per cell, arm and seed")
    sub.add_parser("selftest")
    args = ap.parse_args(argv)
    return {"export": cmd_export, "walk": cmd_walk, "run": cmd_run, "report": cmd_report,
            "selftest": cmd_selftest}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
