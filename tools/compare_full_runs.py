#!/usr/bin/env python3
"""Paired comparison of two run_kissat_full.sh runs (results.csv schema:
instance,result,time_s,timeout,exit_code). Works on finished runs
(results.csv) or in-progress ones (cells/*.csv), pairing only cells present
in both arms.

  python3 tools/compare_full_runs.py <baseline_run_dir> <candidate_run_dir>
        [--solved-floor 0.98] [--par2-ceiling 1.02] [--band 3000]

Reports solved / PAR-2 per arm with the phase-8 gate (candidate solved >=
floor x baseline, PAR-2 <= ceiling x baseline, zero SAT/UNSAT
contradictions), per-cell wall ratios on both-solved cells, the cells only
one arm solved, and the wall-band cells (baseline solved above --band s).
"""
import argparse
import csv
import glob
import math
import os
import sys

SOLVED = {"SAT", "UNSAT"}


def read_run(d):
    rows = {}
    path = os.path.join(d, "results.csv")
    files = [path] if os.path.exists(path) else sorted(glob.glob(os.path.join(d, "cells", "*.csv")))
    for f in files:
        with open(f, newline="") as h:
            for r in csv.reader(h):
                if not r or r[0] == "instance":
                    continue
                name, result, t, to, code = r[0], r[1], float(r[2]), float(r[3]), int(r[4])
                rows[name] = {"result": result, "time": t, "timeout": to, "exit": code}
    return rows


def par2(rows, names):
    return sum(r["time"] if r["result"] in SOLVED else 2 * r["timeout"]
               for n in names for r in [rows[n]])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("baseline")
    ap.add_argument("candidate")
    ap.add_argument("--solved-floor", type=float, default=0.98)
    ap.add_argument("--par2-ceiling", type=float, default=1.02)
    ap.add_argument("--band", type=float, default=3000.0)
    ap.add_argument("--show", type=int, default=15, help="rows per list")
    a = ap.parse_args()
    b, c = read_run(a.baseline), read_run(a.candidate)
    common = sorted(set(b) & set(c))
    print(f"baseline  {a.baseline}: {len(b)} cells")
    print(f"candidate {a.candidate}: {len(c)} cells")
    print(f"paired: {len(common)} cells")
    if not common:
        sys.exit(1)
    bs = sum(1 for n in common if b[n]["result"] in SOLVED)
    cs = sum(1 for n in common if c[n]["result"] in SOLVED)
    bp, cp = par2(b, common), par2(c, common)
    contra = [n for n in common if b[n]["result"] in SOLVED and c[n]["result"] in SOLVED
              and b[n]["result"] != c[n]["result"]]
    odd = [n for n in common for arm in ("b", "c")
           if (b if arm == "b" else c)[n]["result"] not in SOLVED | {"TIMEOUT"}]
    print(f"\nsolved   baseline {bs}   candidate {cs}   (floor {a.solved_floor}x = {a.solved_floor*bs:.1f})")
    print(f"PAR-2    baseline {bp:.0f}   candidate {cp:.0f}   ratio {cp/bp if bp else float('nan'):.4f}   (ceiling {a.par2_ceiling}x)")
    print(f"SAT/UNSAT contradictions: {len(contra)}  {contra}")
    if odd:
        print(f"non-SAT/UNSAT/TIMEOUT results (UNKNOWN/ERROR): {len(set(odd))}")
        for n in sorted(set(odd))[: a.show]:
            print(f"   {n[:60]:60s} base {b[n]['result']:8s} exit {b[n]['exit']:3d} {b[n]['time']:8.1f}s | cand {c[n]['result']:8s} exit {c[n]['exit']:3d} {c[n]['time']:8.1f}s")
    gate = (not contra) and cs >= a.solved_floor * bs and (bp == 0 or cp <= a.par2_ceiling * bp)
    print(f"GATE: {'PASS' if gate else 'FAIL'}")

    both = [n for n in common if b[n]["result"] in SOLVED and c[n]["result"] in SOLVED and b[n]["time"] >= 1.0]
    if both:
        ratios = [c[n]["time"] / b[n]["time"] for n in both]
        gm = math.exp(sum(math.log(r) for r in ratios) / len(ratios))
        print(f"\nboth-solved cells with baseline >= 1 s: {len(both)}   wall ratio geomean {gm:.4f}   "
              f"total {sum(c[n]['time'] for n in both):.0f} / {sum(b[n]['time'] for n in both):.0f} s")
        worst = sorted(both, key=lambda n: c[n]["time"] / b[n]["time"], reverse=True)
        print("  slowest ratios:")
        for n in worst[: a.show]:
            print(f"   {n[:60]:60s} {b[n]['time']:8.1f} -> {c[n]['time']:8.1f}  {c[n]['time']/b[n]['time']:.3f}")
        print("  fastest ratios:")
        for n in worst[-a.show:][::-1]:
            print(f"   {n[:60]:60s} {b[n]['time']:8.1f} -> {c[n]['time']:8.1f}  {c[n]['time']/b[n]['time']:.3f}")
    only_b = [n for n in common if b[n]["result"] in SOLVED and c[n]["result"] not in SOLVED]
    only_c = [n for n in common if c[n]["result"] in SOLVED and b[n]["result"] not in SOLVED]
    print(f"\nbaseline-only solved: {len(only_b)}")
    for n in only_b:
        print(f"   {n[:60]:60s} base {b[n]['result']:5s} {b[n]['time']:8.1f}s | cand {c[n]['result']} {c[n]['time']:.1f}s")
    print(f"candidate-only solved: {len(only_c)}")
    for n in only_c:
        print(f"   {n[:60]:60s} cand {c[n]['result']:5s} {c[n]['time']:8.1f}s | base {b[n]['result']} {b[n]['time']:.1f}s")
    band = [n for n in common if b[n]["result"] in SOLVED and b[n]["time"] >= a.band]
    print(f"\nwall-band cells (baseline solved >= {a.band:.0f} s): {len(band)}")
    for n in band:
        print(f"   {n[:60]:60s} base {b[n]['time']:8.1f}s | cand {c[n]['result']:7s} {c[n]['time']:8.1f}s")


if __name__ == "__main__":
    main()
