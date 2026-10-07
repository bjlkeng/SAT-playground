#!/usr/bin/env python3
"""lymphosat_probe.py — run the LymphoSAT specialists over our cells.

LymphoSAT (SAT Competition 2026, https://c.mov/lymphosat/) is a bundle of
126 family-specialist solvers, each with a detector that recognises one
problem family's encoding from the clauses and a method for that family,
with kissat as the fallback. The registry page publishes every specialist's
source. This tool asks which of them recognise and answer our cells.

    python3 tools/lymphosat_probe.py fetch   --work <dir>
        downloads the registry JSON, writes one source file per specialist
        under <dir>/src, compiles the C++ ones into <dir>/bin
        (g++ -O2 -std=c++20 -march=native -pthread)
    python3 tools/lymphosat_probe.py matrix  --work <dir> --cnf <dir of plain .cnf> --out <tsv> [--timeout 10] [--jobs 32]
        runs every specialist on every cell (`solver <cnf> <proof>`), under a
        memory cap, and records the answer line, exit code and seconds;
        resumes from an existing output
    python3 tools/lymphosat_probe.py verify  --work <dir> --cnf <dir> --matrix <tsv> --out <tsv> --veripb <binary> [--timeout 120]
        re-runs every answer given on a cell our stock pass did not solve,
        with a real proof file: a SAT model is checked against every clause,
        an UNSAT proof with VeriPB (-u)
    python3 tools/lymphosat_probe.py report  --matrix <tsv> [--claims <tsv>]
        the specialists that contradicted a known answer (dropped from
        every count), the targeted matches per family, and the unsolved
        cells with a verified answer

A specialist prints `s UNKNOWN` when the instance is not its family; the
`c detected ...` lines are not reliable (several print one on every input),
so only an answer counts. Specialists that answer a cell with a known
status wrongly are unsound outside their family and are dropped entirely.
Results of 2026-10-06 are in the solver README and plan §11.
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import resource
import subprocess
import sys
import time
import urllib.request
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CELLS = ROOT / "benchmarks" / "rl" / "cells_2025.tsv"
REGISTRY_URL = "https://c.mov/lymphosat/solver-explorer-data.json"


def load_cells():
    with open(CELLS, newline="") as fh:
        return {r["stem"]: r for r in csv.DictReader([l for l in fh if not l.startswith("#")], dialect="excel-tab")}


def specialists(work: Path):
    """(name, command prefix) for every runnable specialist."""
    out = []
    for f in sorted((work / "bin").iterdir()) if (work / "bin").is_dir() else []:
        if f.suffix != ".err" and os.access(f, os.X_OK):
            out.append((f.name, [str(f)]))
    for f in sorted((work / "src").glob("*.py")):
        out.append((f.name, [sys.executable, str(f)]))
    return out


# ---------------------------------------------------------------------------
# fetch
# ---------------------------------------------------------------------------

def cmd_fetch(args) -> int:
    work = Path(args.work)
    (work / "src").mkdir(parents=True, exist_ok=True)
    (work / "bin").mkdir(exist_ok=True)
    reg = work / "solver-explorer-data.json"
    if not reg.exists():
        urllib.request.urlretrieve(REGISTRY_URL, reg)
    d = json.load(open(reg))
    items = d["solvers"]
    print(f"{len(items)} specialists in the registry ({Counter(i['language'] for i in items)})")
    for it in items:
        ext = "cpp" if it["language"] == "cpp" else "py"
        (work / "src" / f"{it['family']}__{it['name']}.{ext}").write_text(it["code"])
        (work / "src" / f"{it['family']}__{it['name']}.txt").write_text(f"{it['description']}\n\n{it['longDescription']}\n\n{it['techniques']}\n")

    def build(src: Path):
        exe = work / "bin" / src.stem
        if exe.exists():
            return src.stem, True
        r = subprocess.run(["g++", "-O2", "-std=c++20", "-march=native", "-pthread", "-o", str(exe), str(src)], capture_output=True, text=True)
        (work / "bin" / f"{src.stem}.err").write_text(r.stderr)
        return src.stem, r.returncode == 0

    failed = []
    with ThreadPoolExecutor(max_workers=args.jobs) as ex:
        for name, ok in ex.map(build, sorted((work / "src").glob("*.cpp"))):
            if not ok:
                failed.append(name)
    print(f"compiled {len(list((work / 'src').glob('*.cpp'))) - len(failed)} C++ specialists; failed: {failed}")
    return 0


# ---------------------------------------------------------------------------
# matrix
# ---------------------------------------------------------------------------

def limits(mem: int):
    def f():
        resource.setrlimit(resource.RLIMIT_AS, (mem, mem))
        os.setsid()
    return f


def run_one(cmd, cnf: Path, proof: str, timeout: float, mem: int):
    """(stdout, exit) of one specialist run; exit is 'timeout' when killed."""
    try:
        p = subprocess.run(cmd + [str(cnf), proof], capture_output=True, text=True, timeout=timeout,
                           preexec_fn=limits(mem), errors="replace")
        return p.stdout, p.returncode
    except subprocess.TimeoutExpired as e:
        out = e.stdout.decode(errors="replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        return out, "timeout"


def cmd_matrix(args) -> int:
    work, cnfdir = Path(args.work), Path(args.cnf)
    specs = specialists(work)
    cells = sorted(cnfdir.glob("*.cnf"))
    done = set()
    fields = ["specialist", "cell", "exit", "verdict", "seconds"]
    if os.path.exists(args.out) and os.path.getsize(args.out) > 0:
        with open(args.out, newline="") as fh:
            rd = csv.DictReader(fh, dialect="excel-tab")
            done = {(r["specialist"], r["cell"]) for r in rd}
            if rd.fieldnames:
                if any(f not in rd.fieldnames for f in fields):
                    raise SystemExit(f"{args.out} lacks a column the matrix writes: {rd.fieldnames}")
                fields = list(rd.fieldnames)          # keep the saved schema on resume
    jobs = [(s, cmd, c) for s, cmd in specs for c in cells if (s, c.stem) not in done]
    print(f"{len(specs)} specialists, {len(cells)} cells, {len(jobs)} runs to do", flush=True)
    mem = args.mem_mb << 20

    def run(job):
        s, cmd, c = job
        t0 = time.time()
        out, code = run_one(cmd, c, "/dev/null", args.timeout, mem)
        verdict = ""
        for line in out.splitlines():
            if line.startswith("s "):
                verdict = line[2:].strip()
        return {"specialist": s, "cell": c.stem, "exit": code, "verdict": verdict, "seconds": round(time.time() - t0, 2)}

    new = not os.path.exists(args.out) or os.path.getsize(args.out) == 0
    with open(args.out, "a", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=fields, dialect="excel-tab", lineterminator="\n", restval="")
        if new:
            w.writeheader()
        t0 = time.time()
        ex = ThreadPoolExecutor(max_workers=args.jobs)
        futs = [ex.submit(run, j) for j in jobs]
        try:
            for n, f in enumerate(as_completed(futs), 1):
                w.writerow(f.result())
                if n % 500 == 0:
                    fh.flush()
                    print(f"  {n}/{len(jobs)} runs, {time.time() - t0:.0f} s", flush=True)
        except KeyboardInterrupt:
            # drop the queue; the runs already started finish within their timeout
            # and are not recorded, so a resume redoes them
            ex.shutdown(wait=False, cancel_futures=True)
            fh.flush()
            print("interrupted: queued runs cancelled, resume with the same --out", flush=True)
            return 130
        ex.shutdown(wait=True)
    print("done", flush=True)
    return 0


# ---------------------------------------------------------------------------
# verify
# ---------------------------------------------------------------------------

def answered(r) -> bool:
    """A complete answer: an answer line from a run that did not hit the
    timeout (a specialist may print its status before the model)."""
    return r["verdict"] in ("SATISFIABLE", "UNSATISFIABLE") and r["exit"] != "timeout"


def unsound_specialists(rows, cells):
    """Specialists with an answer that contradicts a cell's known status."""
    bad = set()
    for r in rows:
        if not answered(r):
            continue
        v, ours = r["verdict"], cells.get(r["cell"], {}).get("status", "")
        if v in ("SATISFIABLE", "UNSATISFIABLE") and ours in ("SAT", "UNSAT") and v[:3] != ours[:3]:
            bad.add(r["specialist"])
    return bad


def check_model(cnf: Path, lits) -> str:
    """Every clause of the CNF satisfied by the printed model? A model that
    sets a variable both ways, or a formula with an empty clause, fails."""
    truth = set(lits)
    if not truth:
        return "NO MODEL PRINTED"
    if any(-l in truth for l in truth):
        return "MODEL CONTRADICTS ITSELF"
    cur: list[int] = []
    with open(cnf) as f:
        for line in f:
            if not line or line[0] in "cp%":
                continue
            for tok in line.split():
                v = int(tok)
                if v == 0:
                    if not any(l in truth for l in cur):
                        return "MODEL FAILS a clause"
                    cur = []
                else:
                    cur.append(v)
    if cur and not any(l in truth for l in cur):
        return "MODEL FAILS a clause"
    return "MODEL OK"


def cmd_verify(args) -> int:
    work, cnfdir = Path(args.work), Path(args.cnf)
    cells = load_cells()
    with open(args.matrix, newline="") as fh:
        rows = list(csv.DictReader(fh, dialect="excel-tab"))
    bad = unsound_specialists(rows, cells)
    cmds = dict(specialists(work))
    pairs = [(r["specialist"], r["cell"]) for r in rows
             if r["specialist"] not in bad and answered(r)
             and cells.get(r["cell"], {}).get("status") == "TIMEOUT" and r["specialist"] in cmds]
    print(f"{len(pairs)} answers on unsolved cells to check ({len(bad)} unsound specialists dropped)", flush=True)
    claims = Path(args.out).with_suffix("") .as_posix() + "-proofs"
    os.makedirs(claims, exist_ok=True)
    mem = args.mem_mb << 20

    def one(pair):
        s, c = pair
        cnf = cnfdir / f"{c}.cnf"
        proof = os.path.join(claims, f"{s}__{c[:40]}.pbp")
        for stale in (proof, str(Path(proof).with_suffix(".model"))):
            if os.path.exists(stale):
                os.remove(stale)
        t0 = time.time()
        out, code = run_one(cmds[s], cnf, proof, args.timeout, mem)
        solve_s = round(time.time() - t0, 1)
        claim = ""
        for line in out.splitlines():
            if line.startswith("s "):
                claim = line[2:].strip()
        t1 = time.time()
        if code == "timeout":
            check = "-"
            claim = "timeout"
        elif claim == "SATISFIABLE":
            lits = [int(t) for line in out.splitlines() if line.startswith("v ") for t in line[2:].split() if t != "0"]
            Path(proof).with_suffix(".model").write_text(" ".join(map(str, lits)) + "\n")
            check = check_model(cnf, lits)
        elif claim == "UNSATISFIABLE":
            if os.path.exists(proof) and os.path.getsize(proof) > 0:
                try:
                    r = subprocess.run([args.veripb, "-u", str(cnf), proof], capture_output=True, text=True, timeout=1800,
                                       preexec_fn=limits(mem))
                    verified = r.returncode == 0 and any(l.strip() == "s VERIFIED UNSATISFIABLE" for l in (r.stdout or "").splitlines())
                    tail = [l.strip() for l in ((r.stdout or "") + (r.stderr or "")).splitlines() if l.strip()]
                    check = "VERIFIED UNSATISFIABLE" if verified else "PROOF REJECTED: " + (tail[-1] if tail else "")[:100]
                except subprocess.TimeoutExpired:
                    check = "veripb timeout"
            else:
                check = "NO PROOF WRITTEN"
        else:
            check = "-"
        return {"specialist": s, "cell": c, "family": cells[c]["family"], "claim": claim or "none", "check": check,
                "solve_s": solve_s, "check_s": round(time.time() - t1, 1)}

    with open(args.out, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=["specialist", "cell", "family", "claim", "check", "solve_s", "check_s"], dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        ex = ThreadPoolExecutor(max_workers=args.jobs)
        try:
            for f in as_completed([ex.submit(one, p) for p in pairs]):
                r = f.result()
                w.writerow(r)
                fh.flush()
                print(f"  {r['family']} {r['cell'][33:60]} {r['specialist'].split('__')[0]}: {r['claim']} -> {r['check']} "
                      f"({r['solve_s']} s, check {r['check_s']} s)", flush=True)
        except KeyboardInterrupt:
            ex.shutdown(wait=False, cancel_futures=True)
            print("interrupted: queued checks cancelled; the output holds the checks that finished", flush=True)
            return 130
        ex.shutdown(wait=True)
    print("done", flush=True)
    return 0


# ---------------------------------------------------------------------------
# report
# ---------------------------------------------------------------------------

# LymphoSAT family names that are the same problem as one of our families
# (by the generators behind the names). An answer from any other specialist
# came from the general solver it carries as a fallback, not from its
# structure; the report labels the two apart.
SAME_FAMILY = {
    "battleship": {"battleship"}, "clqcl": {"clique-coloring", "coloring-clique"},
    "clique-coloring": {"clique-coloring", "coloring-clique"}, "php": {"pigeon-hole", "relativized-pigeon-hole", "binary-pigeon-hole"},
    "roundrobin": {"pigeon-hole"}, "tseitin": {"tseitin-formulas", "xor-chain"}, "parity": {"tseitin-formulas", "xor-chain"},
    "xor-op": {"xor_op", "ordering-principle-xor"}, "ramsey": {"ramsey", "ramsey-numbers"}, "chess-puzzles": {"rooks", "mutilated-chessboard"},
    "kakuro": {"hypertree-decomposition"}, "lockchart": {"mechanical-master-key"}, "frb": {"random-csp", "rbsat"}, "rbsat": {"rbsat", "random-csp"},
    "em": {"edge-matching"}, "sted": {"stedman-triples"}, "gp": {"graceful-production", "profitable-robust-production"},
    "bits-fast": {"multiplier-circuits", "multiplier-verification"}, "reconf": {"independent-set-reconfiguration"},
    "et-tc": {"independent-set"}, "ncc": {"core-based-generator"}, "sum-of-cubes": {"sum-of-3-cubes", "sat-x"}, "ktf": {"ktf"},
    "fsf": {"fixed-shape-random"}, "factoring": {"fermat"}, "oddball": {"oddball-weighing"}, "edp": {"erdos-discrepancy"},
    "vdw": {"waerden"}, "scpc": {"set-covering"}, "sgi": {"subgraph-isomorphism"}, "hcp": {"hamiltonian-cycle"},
    "dltm": {"influence-maximization"}, "planning": {"planning"}, "simon": {"cryptography-simon"},
    "sort-equivalence": {"algorithm-equivalence-checking"}, "lksat": {"clustered-random"}, "regrandom": {"uniform-random", "random"},
    "multiplier-16x16": {"multiplier-verification", "multiplier-circuits"}, "mult-miter-bits": {"multiplier-verification"},
    "circuit-multiplier": {"multiplier-circuits", "multiplier-verification"}, "lec-mult": {"multiplier-verification"},
    "dimacs-coloring": {"coloring-mycielski-graph"}, "snw": {"sorting-networks"}, "arles": {"p-center"}, "linked-list": {"software-verification"},
    "singletons": {"discrete-logarithm", "pythagorean-triples", "cryptography-simon", "diagnosis"},
}


def same_family(our: str, spec: str) -> bool:
    return spec in SAME_FAMILY.get(our, set())


def cmd_report(args) -> int:
    cells = load_cells()
    with open(args.matrix, newline="") as fh:
        rows = list(csv.DictReader(fh, dialect="excel-tab"))
    print(f"{len(rows)} runs; exits: {Counter(r['exit'] for r in rows).most_common(6)}")
    bad = unsound_specialists(rows, cells)
    wrong = Counter(r["specialist"] for r in rows if r["specialist"] in bad and answered(r)
                    and cells.get(r["cell"], {}).get("status", "") in ("SAT", "UNSAT")
                    and cells[r["cell"]]["status"][:3] != r["verdict"][:3])
    print("dropped as unsound (a wrong answer on a cell with a known status): "
          + ", ".join(f"{s.split('__')[0]} ({n} wrong)" for s, n in wrong.items()))
    rows = [r for r in rows if r["specialist"] not in bad]
    ans = defaultdict(set)
    for r in rows:
        if answered(r) and r["cell"] in cells:
            ans[r["specialist"].split("__")[0]].add(r["cell"])
    generic = {s for s, cs in ans.items() if len(cs) >= 15 and len({cells[c]["family"] for c in cs}) >= 8}
    print("\ngeneric specialists (a general solver inside; answered 15+ cells over 8+ families, not counted as matches): "
          + ", ".join(sorted(generic)))
    sizes = Counter(c["family"] for c in cells.values())
    per = defaultdict(lambda: defaultdict(set))
    for s, cs in ans.items():
        if s in generic:
            continue
        for c in cs:
            per[cells[c]["family"]][s].add(c)
    print("\nfamilies answered by a specialist for the same problem (answered / size; * = a cell we time out on);"
          "\n  in brackets: other specialists that answered, through the general solver they carry as a fallback:")
    for f in sorted(per, key=lambda x: (-sizes[x], x)):
        same = [(s, cs) for s, cs in per[f].items() if same_family(f, s)]
        other = [(s, cs) for s, cs in per[f].items() if not same_family(f, s)]
        if not same:
            continue
        fmt = lambda s, cs: f"{s} {len(cs)}{'*' if any(cells[c]['status'] == 'TIMEOUT' for c in cs) else ''}"
        print(f"  {f:<20} {sizes[f]:3d}: " + ", ".join(fmt(s, cs) for s, cs in sorted(same, key=lambda kv: -len(kv[1])))
              + (f"   [{', '.join(fmt(s, cs) for s, cs in sorted(other, key=lambda kv: -len(kv[1]))[:5])}{' ...' if len(other) > 5 else ''}]" if other else ""))
    print("\nfamilies with no same-problem specialist answering: " + ", ".join(
        f"{f} {sizes[f]}" for f in sorted(sizes, key=lambda x: (-sizes[x], x)) if not any(same_family(f, s) for s in per.get(f, {}))))
    if args.claims:
        with open(args.claims, newline="") as fh:
            cl = [r for r in csv.DictReader(fh, dialect="excel-tab") if r["specialist"] not in bad]
        print(f"\nanswers on unsolved cells, checked: {Counter(r['check'][:24] for r in cl)}")
        ver = defaultdict(list)
        for r in cl:
            if r["check"] in ("MODEL OK", "VERIFIED UNSATISFIABLE"):
                spec = r["specialist"].split("__")[0]
                ver[r["cell"]].append((spec, r["claim"][:5], r["solve_s"], same_family(r["family"], spec)))
        fams = defaultdict(list)
        for c, v in ver.items():
            v.sort(key=lambda x: not x[3])           # a same-problem specialist first
            fams[cells[c]["family"]].append((c[33:60], v[0]))
        n_same = sum(1 for v in ver.values() if v[0][3])
        print(f"{len(ver)} unsolved cells with a verified answer, {n_same} from a specialist for the same problem, "
              f"{len(ver) - n_same} only from another specialist's general fallback solver (marked 'fallback'):")
        for f in sorted(fams, key=lambda x: (-len(fams[x]), x)):
            print(f"  {f:<16} {len(fams[f])}: " + "; ".join(f"{c} ({s} {a} {t} s{'' if same else ', fallback'})" for c, (s, a, t, same) in fams[f]))
        only = {r["cell"]: r for r in cl if r["cell"] not in ver}
        print("claimed, not verified: " + ", ".join(f"{r['family']} {c[33:55]} ({r['check'][:40]})" for c, r in only.items()))
    return 0


def main(argv) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("fetch")
    f.add_argument("--work", required=True)
    f.add_argument("--jobs", type=int, default=16)
    f.set_defaults(func=cmd_fetch)
    m = sub.add_parser("matrix")
    m.add_argument("--work", required=True)
    m.add_argument("--cnf", required=True, help="directory of decompressed .cnf files named by stem")
    m.add_argument("--out", required=True)
    m.add_argument("--timeout", type=float, default=10.0)
    m.add_argument("--jobs", type=int, default=32)
    m.add_argument("--mem-mb", type=int, default=8192)
    m.set_defaults(func=cmd_matrix)
    v = sub.add_parser("verify")
    v.add_argument("--work", required=True)
    v.add_argument("--cnf", required=True)
    v.add_argument("--matrix", required=True)
    v.add_argument("--out", required=True)
    v.add_argument("--veripb", required=True)
    v.add_argument("--timeout", type=float, default=120.0)
    v.add_argument("--jobs", type=int, default=16)
    v.add_argument("--mem-mb", type=int, default=16384)
    v.set_defaults(func=cmd_verify)
    r = sub.add_parser("report")
    r.add_argument("--matrix", required=True)
    r.add_argument("--claims")
    r.set_defaults(func=cmd_report)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
