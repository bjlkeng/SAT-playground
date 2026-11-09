#!/usr/bin/env python3
"""Checks of the solver's free-choice reading, solver/13-kissat-rs/src/freeread.rs
(bead SAT-playground-d96.2.4.1, G.8.4a; epic SAT-playground-d96.2).

The Rust reading is a port of tools/free_census.py and must give the same
roles: the same groups, definitions, gates, helpers, free variables, cycle
breaks and constraint clauses, cell by cell.

  fixtures   write solver/13-kissat-rs/tests/fixtures/freeread_toys.txt: the
             census's toys (its selftest's, plus the same encoders at other
             seeds and a copy with repeated literals, tautologies and
             duplicate clauses), each as a file the census reads, with the
             census's own counts; cargo test compares the Rust reading with
             them
  real       run the solver (--freeread=2 --conflicts=0, the other structure
             passes off) on the 2025 cells and compare its counts with the
             census table benchmarks/rl/free_2025.tsv; writes a table with
             one row per cell
  report     summarise such a table: matches, differences and their reasons,
             reading time and work

Usage:
  python3 -I tools/free_read_check.py fixtures
  python3 -I tools/free_read_check.py real --out DIR [--stems a,b] [--jobs 32] [--mem-mb 16000]
  python3 -I tools/free_read_check.py report DIR/results.tsv [AGAIN/results.tsv ...] [--write TABLE.tsv]

The 2026 suite is not read (plan section 8: it is the holdout).
"""
from __future__ import annotations

import argparse
import csv
import os
import queue
import re
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOLS))
import free_census as fc  # noqa: E402

ROOT = TOOLS.parent
SOLVER = ROOT / "solver" / "13-kissat-rs" / "target" / "release" / "sat-solver"
FIXTURES = ROOT / "solver" / "13-kissat-rs" / "tests" / "fixtures" / "freeread_toys.txt"
CENSUS = ROOT / "benchmarks" / "rl" / "free_2025.tsv"
SUITE = ROOT / "benchmarks" / "sat-comp-2025"
# every other structure pass off: the reading alone (it runs last anyway)
OTHERS_OFF = ["--structsym=0", "--structparity=0", "--structcount=0", "--structarith=0", "--structsort=0",
              "--structdiv=0", "--structdd=0", "--gauss=0", "--card=0", "--structpoints=0"]
# the count columns that must be equal, and the shares compared to rounding
INT_FIELDS = ["clauses", "fixed", "satisfied", "present", "groups", "group_vars", "chain_groups", "amo_groups",
              "overlap_groups", "derived_groups", "defs", "wide_defs", "def_inputs_max", "gates", "helpers", "free",
              "cycle_breaks", "cov_group", "cov_def", "cov_gate", "cov_helper", "constraints", "con_choice"]
LIST_FIELDS = ["group_sizes", "con_lens"]
FLOAT_FIELDS = {"bits": 0.051, "choice_share": 0.00006, "constraint_share": 0.00006}
LINE = re.compile(r"^c freeread status=(\w+)(.*)$")


# ---------------------------------------------------------------------------
# fixtures
# ---------------------------------------------------------------------------


def _dimacs(nv, clauses):
    lines = [f"p cnf {nv} {len(clauses)}"]
    lines += [" ".join(str(x) for x in c) + (" 0" if c else "0") for c in clauses]
    return "\n".join(lines) + "\n"


def _census_row(text):
    """The census's reading of a DIMACS text, read as it reads files."""
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "toy.cnf")
        with open(path, "w") as fh:
            fh.write(text)
        cnf, _trunc = fc.read_cnf(path)
    row = {"clauses": cnf.m}
    row, rd = fc.analyse_cnf(cnf, row)
    return row, rd


def _messy(clauses, seed=9):
    """The same formula with repeated literals, tautologies and duplicate
    clauses mixed in (the census and the structure pass both drop repeats
    and tautologies; duplicates stay)."""
    import random
    rng = random.Random(seed)
    out = []
    for i, c in enumerate(clauses):
        c = list(c)
        if i % 7 == 3 and c:
            c.insert(rng.randrange(len(c) + 1), c[rng.randrange(len(c))])
        out.append(tuple(c))
        if i % 11 == 5 and c:
            v = abs(c[0])
            out.append((v, -v) + tuple(c[1:]))
        if i % 13 == 8:
            out.append(tuple(c))
    return out


def _toys():
    """(name, nv, clauses, true literals of a planted model or None)."""
    toys = []
    nv, cl, ins, outs, val = fc.toy_tseitin()
    toys.append(("tseitin", nv, cl, [v for v, b in val.items() if b]))
    nv, cl, ins, outs, val = fc.toy_tseitin(unit=False)
    toys.append(("tseitin-no-unit", nv, cl, [v for v, b in val.items() if b]))
    nv, cl, ins, outs, val = fc.toy_tseitin(one_way=True, seed=6)
    toys.append(("tseitin-one-way", nv, cl, [v for v, b in val.items() if b]))
    nv, cl, x, plant, _nog = fc.toy_csp()
    toys.append(("csp-pairwise", nv, cl, [x[i][plant[i]] for i in range(len(x))]))
    nv, cl, x, plant, _nog = fc.toy_csp(amo="ladder", seed=7)
    toys.append(("csp-ladder", nv, cl, [x[i][plant[i]] for i in range(len(x))]))
    nv, cl, key, plant, _exp = fc.toy_lockchart()
    toys.append(("lockchart", nv, cl, [key[k][p][plant[k][p]] for k in range(len(key)) for p in range(len(key[0]))]))
    nv, cl, val, _exp = fc.toy_guarded()
    toys.append(("guarded", nv, cl, [v for v, b in val.items() if b]))
    toys.append(("root-conflict", 3, [(1,), (-1, 2), (-2, 3), (-3,)], None))
    toys.append(("empty-clause", 2, [(1, 2), ()], None))
    nv, cl = fc.toy_random3()
    toys.append(("random3", nv, cl, None))
    nv, cl = fc.toy_cycle()
    toys.append(("cycle", nv, cl, None))
    # the same encoders at other seeds and sizes
    for s in (11, 12, 13):
        nv, cl, ins, outs, val = fc.toy_tseitin(seed=s, n_gates=90)
        toys.append((f"tseitin-s{s}", nv, cl, [v for v, b in val.items() if b]))
        nv, cl, ins, outs, val = fc.toy_tseitin(seed=s, one_way=True)
        toys.append((f"tseitin-one-way-s{s}", nv, cl, [v for v, b in val.items() if b]))
    for s in (21, 22):
        for amo in ("pairwise", "ladder"):
            nv, cl, x, plant, _nog = fc.toy_csp(seed=s, amo=amo, n=12, d=4)
            toys.append((f"csp-{amo}-s{s}", nv, cl, [x[i][plant[i]] for i in range(len(x))]))
    for s in (31, 32):
        nv, cl, key, plant, _exp = fc.toy_lockchart(seed=s, n_keys=12, n_locks=9)
        toys.append((f"lockchart-s{s}", nv, cl, [key[k][p][plant[k][p]] for k in range(len(key)) for p in range(len(key[0]))]))
    nv, cl, val, _exp = fc.toy_guarded(seed=41, n=12, d=4)
    toys.append(("guarded-s41", nv, cl, [v for v, b in val.items() if b]))
    for s, ratio in ((51, 3.0), (52, 4.5)):
        nv, cl = fc.toy_random3(seed=s, n=120, ratio=ratio)
        toys.append((f"random3-s{s}", nv, cl, None))
    nv, cl = fc.toy_cycle(n=7)
    toys.append(("cycle-7", nv, cl, None))
    # repeated literals, tautologies and duplicate clauses
    nv, cl, key, plant, _exp = fc.toy_lockchart()
    toys.append(("lockchart-messy", nv, _messy(cl), [key[k][p][plant[k][p]] for k in range(len(key)) for p in range(len(key[0]))]))
    nv, cl, ins, outs, val = fc.toy_tseitin(seed=3)
    toys.append(("tseitin-messy", nv, _messy(cl, seed=4), [v for v, b in val.items() if b]))
    return toys


def _expect(row):
    out = {}
    for k in INT_FIELDS:
        out[k] = str(row[k])
    for k in LIST_FIELDS:
        out[k] = row[k].replace(" ", ",") if row[k] else "-"
    for k in FLOAT_FIELDS:
        out[k] = str(row[k])
    return out


def cmd_fixtures(args):
    out = []
    for name, nv, cl, plant in _toys():
        text = _dimacs(nv, cl)
        row, rd = _census_row(text)
        if rd is None:
            expect = {"status": "refuted"}
        else:
            if row["sim_bad"]:
                raise SystemExit(f"{name}: the census's own simulation finds a covered clause false")
            expect = _expect(row)
        out.append(f"toy {name}")
        out.append("expect " + " ".join(f"{k}={v}" for k, v in expect.items()))
        if plant is not None and rd is not None:
            out.append("plant " + " ".join(str(x) for x in sorted(plant)))
        out.append(text.rstrip("\n"))
        out.append("end")
        print(f"{name:24} " + (" ".join(f"{k}={expect[k]}" for k in ("groups", "defs", "gates", "helpers", "free", "constraints"))
                               if rd is not None else "refuted"))
    Path(args.out).write_text("\n".join(out) + "\n")
    print(f"wrote {args.out}")
    return 0


# ---------------------------------------------------------------------------
# real cells
# ---------------------------------------------------------------------------


def _cores(n):
    """n cores for pinned jobs, the harness's layout: 0-15, then 18-33, then
    the rest."""
    first = list(range(0, 16)) + list(range(18, 34))
    rest = [c for c in range(os.cpu_count() or 1) if c not in first]
    return (first + rest)[:n]


def parse_line(text):
    """The `c freeread` line of a run: (status, fields) or (None, {})."""
    for line in text.splitlines():
        m = LINE.match(line)
        if m:
            fields = {}
            for kv in m.group(2).split():
                if "=" in kv:
                    k, v = kv.split("=", 1)
                    fields[k] = v
            return m.group(1), fields
    return None, {}


def run_cell(solver, path, args, pool):
    """One cell on a core of its own, under a memory limit (ulimit -v: the
    address space, as the harness's)."""
    core = pool.get()
    try:
        cmd = [solver, "-q", *OTHERS_OFF, "--freeread=2", f"--freereadticks={args.ticks}",
               f"--structclauses={args.structclauses}", "--conflicts=0", str(path)]
        wrapped = ["taskset", "-c", str(core), "bash", "-c", f'ulimit -v {args.mem_mb * 1024}; exec "$@"', "_", *cmd]
        t0 = time.time()
        try:
            p = subprocess.run(wrapped, capture_output=True, text=True, timeout=args.timeout)
            status, fields = parse_line(p.stdout)
            rc = p.returncode
            lines = p.stderr.strip().splitlines()
            # the line that says what went wrong, else the last one
            key = [ln for ln in lines if "memory allocation" in ln or "panicked" in ln or "error" in ln.lower()]
            err = (key[0] if key else lines[-1] if lines else "")[:200]
        except subprocess.TimeoutExpired:
            status, fields, rc, err = "timeout", {}, -1, "timeout"
        return status, fields, rc, round(time.time() - t0, 1), err
    finally:
        pool.put(core)


def compare(census, status, fields, err=""):
    """(verdict, differing fields): verdict 'match', 'differ', or a reason."""
    if census.get("error") == "root-conflict":
        return ("match", []) if status == "refuted" else ("differ", ["status"])
    if status == "refuted":
        return "differ", ["status"]
    if status == "overflow":
        return ("overflow-prefix" if census.get("truncated") == "1" else "overflow"), []
    if not status and "memory allocation" in (err or ""):
        # the solver could not load the cell within the memory limit; the
        # census read a prefix of the cells too big to read whole
        return ("memory-prefix" if census.get("truncated") == "1" else "memory"), []
    if status != "read":
        return status or "no-line", []
    if census.get("truncated") == "1":
        return "census-prefix", []
    diff = []
    if fields.get("sim_bad", "0") != "0":
        diff.append("sim_bad")       # the reading's own simulation found a claimed clause false
    for k in INT_FIELDS:
        if str(census.get(k, "")) != fields.get(k):
            diff.append(k)
    for k in LIST_FIELDS:
        want = (census.get(k) or "").replace(" ", ",") or "-"
        if fields.get(k) != want:
            diff.append(k)
    for k, tol in FLOAT_FIELDS.items():
        try:
            if abs(float(census.get(k) or 0) - float(fields.get(k))) > tol:
                diff.append(k)
        except (TypeError, ValueError):
            diff.append(k)
    return ("match" if not diff else "differ"), diff


RESULT_COLUMNS = ["stem", "verdict", "diff", "status", "work", "ms", "sim_ms", "wall_s", "rc", "sim_bad", "sim_false",
                  *INT_FIELDS, *LIST_FIELDS, *FLOAT_FIELDS, "vars", "topo", "census_status", "err"]


def cmd_real(args):
    solver = str(Path(args.solver).resolve())
    if not os.access(solver, os.X_OK):
        raise SystemExit(f"no solver at {solver}")
    files = [Path(p) for p in args.files] if args.files else sorted(SUITE.glob("*.cnf.xz"))
    if any("sat-comp-2026" in str(f) for f in files):
        raise SystemExit("the 2026 suite is the holdout and is not read")
    stems = args.stems.split(",") if args.stems else None
    census = {r["stem"]: r for r in csv.DictReader(open(CENSUS), delimiter="\t")}
    jobs = []
    for f in files:
        stem = fc._stem(f)
        if stems and not any(s in stem for s in stems):
            continue
        jobs.append((stem, f))
    jobs.sort(key=lambda j: -j[1].stat().st_size)  # the long tail starts early
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    table = out_dir / "results.tsv"
    print(f"{len(jobs)} cells, {args.jobs} at once, solver {solver}", flush=True)
    rows = []
    pool = queue.Queue()
    for c in _cores(args.jobs):
        pool.put(c)
    with open(table, "w", newline="") as fh, ThreadPoolExecutor(max_workers=args.jobs) as ex:
        w = csv.DictWriter(fh, fieldnames=RESULT_COLUMNS, delimiter="\t", extrasaction="ignore")
        w.writeheader()
        fh.flush()
        futs = {ex.submit(run_cell, solver, f, args, pool): stem for stem, f in jobs}
        for fut in as_completed(futs):
            stem = futs[fut]
            status, fields, rc, wall, err = fut.result()
            c = census.get(stem, {})
            verdict, diff = compare(c, status, fields, err)
            row = {"stem": stem, "verdict": verdict, "diff": ",".join(diff), "status": status, "wall_s": wall, "rc": rc,
                   "census_status": c.get("error") or ("prefix" if c.get("truncated") == "1" else "read"), "err": err}
            row.update({k: v for k, v in fields.items() if k in RESULT_COLUMNS})
            w.writerow(row)
            fh.flush()
            rows.append(row)
            print(f"{stem[:60]:60} {verdict:16} {row['diff'][:40]:40} work={fields.get('work', '-')} "
                  f"ms={fields.get('ms', '-')} wall={wall}", flush=True)
    bad = [r for r in rows if r["verdict"] not in GOOD]
    print(f"{len(rows)} cells: " + ", ".join(f"{k} {sum(1 for r in rows if r['verdict'] == k)}"
                                             for k in sorted({r['verdict'] for r in rows})))
    return 1 if bad else 0


# verdicts that are not a fault of the reading: the census read a prefix of
# the cells too big to read whole, and the solver either overflows its
# structure buffer on them or cannot load them at all
GOOD = ("match", "overflow-prefix", "memory-prefix")


def cmd_report(args):
    # later tables replace the rows of earlier ones (a cell run again)
    by_stem = {}
    for t in args.tables:
        for r in csv.DictReader(open(t), delimiter="\t"):
            by_stem[r["stem"]] = r
    rows = list(by_stem.values())
    census = {r["stem"]: r for r in csv.DictReader(open(CENSUS), delimiter="\t")}
    for r in rows:
        # the verdicts again, by the current rules, from what the run recorded
        fields = {k: r[k] for k in RESULT_COLUMNS if r.get(k) not in (None, "")}
        r["verdict"], diff = compare(census.get(r["stem"], {}), r["status"], fields, r.get("err", ""))
        r["diff"] = ",".join(diff)
    print(f"{len(rows)} cells")
    from collections import Counter
    print("verdicts:", dict(Counter(r["verdict"] for r in rows)))
    fields = Counter(k for r in rows if r["verdict"] == "differ" for k in r["diff"].split(",") if k)
    print("differing fields:", dict(fields) or "none")
    for r in rows:
        if r["verdict"] != "match":
            print(f"  {r['stem'][:70]:70} {r['verdict']} {r['diff']} status={r['status']} rc={r['rc']} {r['err'][:60]}")
    read = [r for r in rows if r["status"] == "read"]
    if read:
        ms = sorted(int(r["ms"]) for r in read)
        work = sorted(int(r["work"]) for r in read)
        def q(xs, p):
            return xs[min(len(xs) - 1, int(p * len(xs)))]
        print(f"reading time ms: median {q(ms, 0.5)}, 90% {q(ms, 0.9)}, max {ms[-1]}; "
              f"work: median {q(work, 0.5)}, 90% {q(work, 0.9)}, max {work[-1]}")
        bad = [r["stem"] for r in read if r.get("sim_bad") not in ("0", "", None)]
        print("cells where the simulation finds a covered clause false:", bad or "none")
    if args.write:
        with open(args.write, "w", newline="") as fh:
            w = csv.DictWriter(fh, fieldnames=RESULT_COLUMNS, delimiter="\t", extrasaction="ignore")
            w.writeheader()
            for r in sorted(rows, key=lambda r: r["stem"]):
                w.writerow(r)
        print(f"wrote {args.write}")
    return 0 if all(r["verdict"] in GOOD for r in rows) else 1


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("fixtures")
    f.add_argument("--out", default=str(FIXTURES))
    f.set_defaults(func=cmd_fixtures)
    r = sub.add_parser("real")
    r.add_argument("--out", required=True)
    r.add_argument("--files", nargs="*")
    r.add_argument("--stems", help="comma-separated substrings of stems (default: every 2025 cell)")
    r.add_argument("--solver", default=str(SOLVER))
    r.add_argument("--jobs", type=int, default=32)
    r.add_argument("--mem-mb", type=int, default=16000)
    r.add_argument("--timeout", type=int, default=3600)
    r.add_argument("--ticks", type=int, default=2_147_483_647, help="--freereadticks (K)")
    r.add_argument("--structclauses", type=int, default=50_000_000, help="the structure buffer (the census read 50M whole)")
    r.set_defaults(func=cmd_real)
    p = sub.add_parser("report")
    p.add_argument("tables", nargs="+", help="results tables; a later one's rows replace an earlier one's")
    p.add_argument("--write", help="write the merged table, verdicts by the current rules, sorted by stem")
    p.set_defaults(func=cmd_report)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
