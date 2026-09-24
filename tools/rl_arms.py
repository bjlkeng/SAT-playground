#!/usr/bin/env python3
"""rl_arms.py — tick-deterministic N-arm comparisons on per-cell work budgets.

Plan: plan/rl-scheduler-solver13-plan.md §8 (three arms, tick-budgeted
deterministic candidate selection, per-family reporting), §10 step D.
Beads SAT-playground-p9m.9.3 (D.3, the stock-clone check) and the later
gates that select candidates on the validation split (E.8).

`make` writes a collector job table (tools/rl_collect.py `table`) with one
job per cell and arm, every job stopping on the cell's work budget
(SAT_LIMIT_TICKS = B_cell from benchmarks/rl/cells_2025.tsv, or the band
budget with --band-budget), so a run's result and work are exactly
reproducible under any load. Arms are `name:flavour[:ENV=VALUE ENV=VALUE ...]`
(the environment items are separated by spaces, so a value may hold colons,
as in SAT_POLICY_HORIZON=ticks:1000; quote the whole arm):

    off                       the plain solver (policy off, no log)
    stock                     policy on with the stock action, logging on (the overhead arm)
    net:SAT_POLICY=<file>     the learned policy at SAT_POLICY_MARGIN (default 1)

`report` reads a finished pass and prints, per arm: solved cells, tick
PAR-2 on the work clock W (a solved cell costs its W, an unsolved one
twice its budget), the wall of the solved cells for reference (under a
work budget there is no wall PAR-2), and the same per family; then
the per-cell comparison of every arm against the first: cells whose W
differs, whose result differs, and the largest ratios. Cells the collector
flagged are reported and left out of the sums.

    ~/.cache/sat13-rl/venv/bin/python tools/rl_arms.py make --split val \\
        --arm off:off --arm stock:stock --arm 'clone:net:SAT_POLICY=/abs/clone.net.bin SAT_POLICY_MARGIN=1' \\
        --out /path/jobs-d3.tsv
    python3 tools/rl_collect.py table /path/jobs-d3.tsv --suite sat-comp-2025 --name d3clone \\
        --jobs 4 --cores 14,15,16,17 --force --oracle ...
    python3 tools/rl_arms.py report log/rl-d3clone-<ts>
"""
from __future__ import annotations

import argparse
import csv
import math
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from rl_split import load_split  # noqa: E402

CELLS = ROOT / "benchmarks" / "rl" / "cells_2025.tsv"
JOB_COLUMNS = ("stem", "tag", "flavour", "seed", "limit_ticks", "wall_s", "procs", "peak_rss_mb", "env")
SOLVED = ("SAT", "UNSAT")


def load_cells(path: Path) -> dict[str, dict]:
    with open(path, newline="") as f:
        lines = [ln for ln in f if not ln.startswith("#")]
    return {r["stem"]: r for r in csv.DictReader(lines, dialect="excel-tab")}


def parse_arm(spec: str) -> tuple[str, str, str]:
    # name and flavour end at the first two colons; the rest is the env
    # string as the collector reads it (space-separated KEY=VALUE items,
    # values may hold colons)
    parts = spec.split(":", 2)
    if len(parts) < 2:
        raise SystemExit(f"--arm {spec!r}: expected name:flavour[:ENV=VALUE ...]")
    name, flavour = parts[0], parts[1]
    env = " ".join(parts[2].split()) if len(parts) == 3 else ""
    for item in env.split():
        if "=" not in item:
            raise SystemExit(f"--arm {spec!r}: env item {item!r} is not KEY=VALUE")
    if flavour not in ("off", "stock", "net", "random", "jitter"):
        raise SystemExit(f"--arm {spec!r}: flavour {flavour!r}")
    if flavour == "net" and "SAT_POLICY=" not in env:
        raise SystemExit(f"--arm {spec!r}: a net arm needs SAT_POLICY=<weights file>")
    return name, flavour, env


def cmd_make(args) -> int:
    cells = load_cells(Path(args.cells))
    split = load_split()
    arms = [parse_arm(a) for a in args.arm]
    names = [a[0] for a in arms]
    if len(set(names)) != len(names):
        raise SystemExit("arm names must be distinct")
    wanted = {s for s, k in split.items() if k == args.split} if args.split != "all" else {
        s for s, k in split.items() if k != "shared"}
    rows = []
    skipped = 0
    for stem in sorted(cells):
        c = cells[stem]
        if stem not in wanted or c.get("failed") not in ("0", "", None):
            continue
        key = "B_cell_band" if args.band_budget else "B_cell"
        if not c.get(key):
            skipped += 1
            continue
        budget = int(c[key])
        rate = float(c.get("work_per_s") or 0) or 3e7
        cap = int(args.wall_factor * budget / rate) + 300
        rss = max(float(c.get("peak_rss_mb") or 0), float(c.get("band_peak_rss_mb") or 0))
        for name, flavour, env in arms:
            rows.append({"stem": stem, "tag": name, "flavour": flavour, "seed": int(c.get("seed") or 0),
                         "limit_ticks": budget, "wall_s": cap, "procs": 1, "peak_rss_mb": f"{rss:.1f}" if rss else "",
                         "env": env})
    out = Path(args.out)
    with open(out, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(JOB_COLUMNS), dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        for r in rows:
            w.writerow(r)
    print(f"wrote {out}: {len(rows)} jobs = {len(rows) // max(len(arms), 1)} cells x {len(arms)} arms "
          f"({args.split} split, {'band' if args.band_budget else 'B_cell'} budgets); {skipped} cells without a budget")
    return 0


def cmd_report(args) -> int:
    run = Path(args.run_dir)
    cells = load_cells(Path(args.cells))
    with open(run / "results.tsv", newline="") as f:
        recs = list(csv.DictReader(f, dialect="excel-tab"))
    by_arm: dict[str, dict[str, dict]] = defaultdict(dict)
    for r in recs:
        by_arm[r["tag"]][r["stem"]] = r
    # The arms and jobs the pass was asked to run come from its job table,
    # not from the results: an arm whose jobs never finished must not
    # vanish from a comparison that then looks complete.
    expected: dict[str, set] = defaultdict(set)
    if (run / "jobs.tsv").is_file():
        with open(run / "jobs.tsv", newline="") as f:
            for j in csv.DictReader(f, dialect="excel-tab"):
                expected[j["tag"]].add(j["stem"])
    missing = {a: sorted(stems - set(by_arm.get(a, {}))) for a, stems in expected.items()}
    missing = {a: m for a, m in missing.items() if m}
    if missing:
        print(f"*** pass incomplete: " + ", ".join(f"{a}: {len(m)} job(s) without a record" for a, m in missing.items()))
        if not args.partial:
            return 1
    # the reference arm: the plain solver when there is one, else the stock
    # arm, else the first tag (the collector's tables are sorted by job
    # length, so the table's own order is not kept); --arms overrides
    arms = sorted(set(by_arm) | set(expected), key=lambda a: (a != "off", a != "stock", a))
    arms = [a for a in arms if a in by_arm]
    if args.arms:
        wanted = [a for a in args.arms.split(",") if a]
        absent = [a for a in wanted if a not in by_arm]
        if absent:
            # a requested arm with no record (a typo, or a candidate that never
            # ran) must not vanish from a comparison that then looks complete
            print(f"*** requested arm(s) {absent} have no results in {run} (present: {sorted(by_arm)})")
            return 1
        arms = wanted
    common = set.intersection(*(set(by_arm[a]) for a in arms)) if arms else set()
    # Arms must agree on SAT v UNSAT: the collector checks each run against
    # the oracle and its fork siblings, not against the other arms, so a
    # cell no oracle knows could carry a contradiction only this report
    # sees. It is a solver bug and fails the comparison (CLAUDE.md
    # "Correctness is absolute"). Checked over every cell any arm ran,
    # before any cell is dropped: a capped or missing third arm must not
    # hide a conflict between two completed ones.
    # every completed answer counts, including a run the collector marked
    # wall_cap_after_answer (it had answered and sealed its log before the
    # cap fired); only a run the cap ended (wall_cap) has no answer to check
    every = set().union(*(set(by_arm[a]) for a in arms)) if arms else set()
    contradictions = sorted(
        s for s in every
        if len({by_arm[a][s]["result"] for a in arms if s in by_arm[a] and by_arm[a][s]["result"] in SOLVED
                and by_arm[a][s].get("anomaly") != "wall_cap"}) > 1)
    # every arm of a cell must have run on the same work budget: an unsolved
    # run costs twice its budget, so a smaller budget would look like a gain
    budgets = {s for s in common if len({by_arm[a][s].get("limit_ticks", "") for a in arms}) > 1}
    if budgets:
        print(f"*** {len(budgets)} cell(s) with different work budgets across arms, e.g. {sorted(budgets)[:3]}: "
              f"not comparable")
        return 1
    # A cell leaves the totals only when the collector flagged a run of it
    # (a correctness failure) or a run hit its safety wall cap (no result).
    # A run that ended without a work clock (a memory abort under the
    # ulimit) stays: it is unsolved at twice its budget (CLAUDE.md
    # "Evaluation"), and only the per-cell W comparison skips it.
    failures = {s for a in arms for s, r in by_arm[a].items() if r.get("failed", "0") == "1"}
    flagged = failures | {s for a in arms for s, r in by_arm[a].items() if r.get("anomaly")}
    stems = sorted(common - flagged - set(contradictions))
    no_work = sum(1 for s in stems for a in arms if by_arm[a][s].get("work", "NA") == "NA")
    print(f"{run}: arms {arms}; {len(common)} cells with every arm, {len(flagged)} flagged or capped left out, "
          f"{len(contradictions)} contradicting, {len(stems)} compared ({no_work} runs without a work clock, "
          f"priced as unsolved)")
    if flagged:
        for s in sorted(flagged)[:10]:
            print("  flagged:", s[:60], {a: (by_arm[a].get(s) or {}).get("anomaly") or (by_arm[a].get(s) or {}).get("note", "")[:60] for a in arms})
    for s in contradictions:
        print(f"  *** CORRECTNESS FAILURE: arms disagree on {s}: "
              + ", ".join(f"{a}={by_arm[a][s]['result']}" for a in arms if s in by_arm[a]))
    for s in sorted(failures):
        print(f"  *** CORRECTNESS FAILURE: the collector flagged {s}: "
              + ", ".join(f"{a}: verify={by_arm[a][s].get('verify')} oracle={by_arm[a][s].get('oracle')} "
                          f"note={by_arm[a][s].get('note', '')[:60]}" for a in arms if by_arm[a].get(s)))
    # Under a work budget the wall column of results.tsv is elapsed time and
    # the job's wall_s only a safety cap, so no wall PAR-2 exists: the wall
    # total covers the solved cells alone and is printed for reference.
    budgeted = any(by_arm[a][s].get("limit_ticks", "") != "" for a in arms for s in stems)

    def par2(rs: list[str], arm: str) -> tuple[int, float, float]:
        solved = 0
        tick = 0.0
        wall = 0.0
        for s in rs:
            r = by_arm[arm][s]
            budget = float(r["limit_ticks"])
            if r["result"] in SOLVED and r.get("work", "NA") != "NA":
                solved += 1
                tick += float(r["work"])
                wall += float(r["wall_s"])
            else:
                tick += 2 * budget
        return solved, tick, wall

    base = arms[0]
    wall_label = "wall solved s" if budgeted else "wall PAR-2 s"
    print(f"\n{'arm':12s} {'solved':>6s} {'tick PAR-2':>12s} {'v first':>8s} {wall_label:>13s} {'v first':>8s}")
    ref = par2(stems, base)
    for a in arms:
        s, t, w = par2(stems, a)
        print(f"{a:12s} {s:6d} {t:12.4e} {t / ref[1] if ref[1] else float('nan'):8.4f} {w:13.0f} "
              f"{w / ref[2] if ref[2] else float('nan'):8.4f}")
    # per family
    fam_of = {s: (cells.get(s) or {}).get("family", "?") for s in stems}
    fams = defaultdict(list)
    for s in stems:
        fams[fam_of[s]].append(s)
    print(f"\nper family (cells, then per arm: solved / tick PAR-2 v first arm)")
    for fam in sorted(fams, key=lambda f: -len(fams[f])):
        rs = fams[fam]
        r0 = par2(rs, base)
        cols = []
        for a in arms:
            s, t, _ = par2(rs, a)
            cols.append(f"{a} {s}/{t / r0[1] if r0[1] else float('nan'):.3f}")
        print(f"  {fam[:24]:24s} {len(rs):3d}  " + "  ".join(cols))
    # per-cell comparison against the first arm
    print(f"\nper cell against {base}:")
    for a in arms[1:]:
        same_w = sum(1 for s in stems if by_arm[a][s]["work"] == by_arm[base][s]["work"])
        same_r = sum(1 for s in stems if by_arm[a][s]["result"] == by_arm[base][s]["result"])
        ratios = []
        for s in stems:
            if by_arm[base][s].get("work", "NA") == "NA" or by_arm[a][s].get("work", "NA") == "NA":
                continue
            w0, w1 = float(by_arm[base][s]["work"]), float(by_arm[a][s]["work"])
            if w0 > 0 and w1 > 0:
                ratios.append((w1 / w0, s))
        ratios.sort()
        gm = math.exp(sum(math.log(r) for r, _ in ratios) / len(ratios)) if ratios else float("nan")
        print(f"  {a}: identical W on {same_w}/{len(stems)} cells, same result on {same_r}/{len(stems)}, "
              f"W ratio geomean {gm:.4f} over {len(ratios)}"
              + (f", min {ratios[0][0]:.3f} ({ratios[0][1][:40]}), max {ratios[-1][0]:.3f} ({ratios[-1][1][:40]})"
                 if ratios else ""))
        diff = [s for s in stems if by_arm[a][s]["result"] != by_arm[base][s]["result"]]
        for s in diff[:10]:
            print(f"    result differs: {s[:60]} {base}={by_arm[base][s]['result']} {a}={by_arm[a][s]['result']}")
    if contradictions or failures:
        print(f"*** {len(contradictions)} cell(s) with contradicting answers across arms and {len(failures)} flagged by "
              f"the collector: a correctness failure, the comparison is not valid")
        return 1
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    m = sub.add_parser("make", help="write the job table")
    m.add_argument("--cells", default=str(CELLS))
    m.add_argument("--split", default="val", choices=["train", "val", "all"])
    m.add_argument("--arm", action="append", required=True, help="name:flavour[:ENV=VALUE:...] (repeatable)")
    m.add_argument("--band-budget", action="store_true", help="use B_cell_band (band cells only)")
    m.add_argument("--wall-factor", type=float, default=3.0, help="safety wall cap as a multiple of the expected wall")
    m.add_argument("--out", required=True)
    m.set_defaults(func=cmd_make)
    r = sub.add_parser("report", help="compare the arms of a finished pass")
    r.add_argument("run_dir")
    r.add_argument("--cells", default=str(CELLS))
    r.add_argument("--arms", help="comma list, first is the reference (default: off, stock, then the rest)")
    r.add_argument("--partial", action="store_true", help="report a pass whose jobs are not all done")
    r.set_defaults(func=cmd_report)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
