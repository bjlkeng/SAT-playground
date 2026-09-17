#!/usr/bin/env python3
"""Report for the RL plan step A.11 overhead run (bead SAT-playground-p9m.6.11).

Reads a feature_ablation.py --arm run directory with two arms, `base` (policy
off) and `policylog` (policy on, stock action, raw-state log per cell,
observe() every epoch, the wall horizon), and answers the two questions of
the bead:

  1. Is the work clock W identical per cell? It must be on every cell both
     arms solved (the parity claim: the policy plumbing with the stock action
     changes no counter). On cells neither arm solved, W is the work reached
     before the wall kill and differs with the wall overhead itself; it is
     reported separately.
  2. What does the plumbing cost in wall? The geometric mean of the per-cell
     wall ratio policylog / base over the cells both arms solved, with the
     spread and the per-family breakdown, plus solved counts and wall PAR-2
     per arm. Timeouts that flipped between the arms are listed: on a
     borderline cell the overhead can turn a solve into a timeout.

Usage: rl_stepA_overhead_report.py <run dir> [--base base] [--arm policylog]
"""
import csv
import math
import sys
from pathlib import Path


def read_arm(run: Path, arm: str) -> dict:
    rows = {}
    with open(run / arm / "results.tsv") as fh:
        for r in csv.DictReader(fh, delimiter="\t"):
            key = (r["instance"], r["seed"])
            rows[key] = r
    return rows


def family(instance: str) -> str:
    stem = instance.split("-", 1)[1] if "-" in instance else instance
    return stem.split("-")[0].split("_")[0]


def solved(r: dict) -> bool:
    return r["result"] in ("SATISFIABLE", "UNSATISFIABLE")


def geomean(xs):
    xs = [x for x in xs if x > 0]
    return math.exp(sum(math.log(x) for x in xs) / len(xs)) if xs else float("nan")


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    run = Path(argv[1])
    base_name, arm_name = "base", "policylog"
    i = 2
    while i < len(argv):
        if argv[i] == "--base":
            base_name = argv[i + 1]; i += 2
        elif argv[i] == "--arm":
            arm_name = argv[i + 1]; i += 2
        else:
            raise SystemExit(f"unknown argument {argv[i]}")
    base = read_arm(run, base_name)
    arm = read_arm(run, arm_name)
    common = sorted(set(base) & set(arm))
    print(f"run {run}: {len(base)} base cells, {len(arm)} {arm_name} cells, {len(common)} common")
    if not common:
        return 1
    timeout = float(next(iter(base.values()))["timeout"])

    # Correctness: the two arms must agree on every answer.
    contradictions = [k for k in common if solved(base[k]) and solved(arm[k])
                      and base[k]["result"] != arm[k]["result"]]
    if contradictions:
        print(f"CONTRADICTIONS ({len(contradictions)}): " + ", ".join(k[0] for k in contradictions))
        print("the report is unfit for a decision until these are debugged")

    # Solved counts and wall PAR-2 per arm.
    for name, rows in ((base_name, base), (arm_name, arm)):
        n_solved = sum(solved(rows[k]) for k in common)
        par2 = sum(float(rows[k]["time_s"]) if solved(rows[k]) else 2 * timeout for k in common)
        print(f"{name:10s} solved {n_solved}/{len(common)}  wall PAR-2 {par2:.0f}")

    # Question 1: W per cell.
    both = [k for k in common if solved(base[k]) and solved(arm[k])]
    neither = [k for k in common if not solved(base[k]) and not solved(arm[k])]
    flipped = [k for k in common if solved(base[k]) != solved(arm[k])]
    w_diff = [k for k in both if base[k]["work"] != arm[k]["work"]]
    print(f"\nwork clock W on the {len(both)} cells both arms solved: "
          f"{len(both) - len(w_diff)} identical, {len(w_diff)} different")
    for k in w_diff:
        print(f"  W DIFFERS {k[0]}: base {base[k]['work']} {arm_name} {arm[k]['work']}")
    if neither:
        ratios = []
        for k in neither:
            try:
                ratios.append(float(arm[k]["work"]) / float(base[k]["work"]))
            except (ValueError, ZeroDivisionError):
                pass
        print(f"work reached before the kill on the {len(neither)} cells neither solved: "
              f"{arm_name}/base geomean {geomean(ratios):.4f} (wall-dependent, informational)")
    for k in flipped:
        b, a = base[k], arm[k]
        print(f"  FLIPPED {k[0]}: base {b['result']} {float(b['time_s']):.0f}s "
              f"v {arm_name} {a['result']} {float(a['time_s']):.0f}s")

    # Question 2: wall overhead on cells both solved.
    ratios = {k: float(arm[k]["time_s"]) / float(base[k]["time_s"]) for k in both
              if float(base[k]["time_s"]) > 0}
    if ratios:
        vals = sorted(ratios.values())
        print(f"\nwall {arm_name}/base on the {len(ratios)} cells both solved: "
              f"geomean {geomean(vals):.4f}, median {vals[len(vals) // 2]:.4f}, "
              f"min {vals[0]:.3f}, max {vals[-1]:.3f}")
        long_cells = {k: v for k, v in ratios.items() if float(base[k]["time_s"]) >= 60}
        if long_cells:
            print(f"  on the {len(long_cells)} cells base took >= 60 s: geomean "
                  f"{geomean(long_cells.values()):.4f}")
        fams = {}
        for k, v in ratios.items():
            fams.setdefault(family(k[0]), []).append(v)
        print("  per family (cells, geomean ratio):")
        for f in sorted(fams, key=lambda f: -len(fams[f])):
            print(f"    {f:24s} {len(fams[f]):3d}  {geomean(fams[f]):.4f}")
        worst = sorted(ratios.items(), key=lambda kv: -kv[1])[:5]
        print("  five largest ratios:")
        for k, v in worst:
            print(f"    {v:.3f}  {k[0]}  base {float(base[k]['time_s']):.1f}s")
    return 1 if contradictions or w_diff else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
