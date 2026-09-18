#!/usr/bin/env python3
"""rl_xd_sweep.py — the decision-epoch (X_d) sweep: job table and report.

Plan: plan/rl-scheduler-solver13-plan.md §1 (epoch clock), §10 step B.
Bead SAT-playground-p9m.7.10 (step B.10); feeds the decision bead
SAT-playground-p9m.12 (X_d in {2^26, 2^27, 2^28}).

What the decision needs, and where each number comes from:

  1. How many stock timer fires one decision epoch spans, and decisions
     per run, per cell and X_d. From the stock traces alone: the log has
     the fire count of every timer per observation epoch (`tm_*_fires`)
     and a decision epoch is X_d / X_o observation epochs, so `report`
     aggregates the converted stock pass for all three X_d without a run.
  2. The size of the paired difference one deviating decision epoch
     produces. A fork pass on the discriminating cells: per cell and X_d
     one parent with three branch points (probe, reduce and mode menus at
     about 20 %, 40 % and 60 % of the run's decisions), children on the
     cell's stock budget B_cell. `make` writes that job table;
     tools/rl_collect.py runs it (tick-deterministic, so load does not
     matter); `report` compares every child to its parent at the end:
     work, conflicts and outcome.
  3. Logging and decide overhead. Logging is per observation epoch and was
     measured in step A.11 (nil); a random-mode decision is a few draws
     and a net decision 50 us (A.10), so per X_d it is 2^-26 of nothing.
     Reported as a note, not measured again.

    ~/.cache/sat13-rl/venv/bin/python tools/rl_xd_sweep.py make --cells benchmarks/rl/cells_2025.tsv \\
        --suite discriminating --out /path/jobs-xd.tsv
    python3 tools/rl_collect.py table /path/jobs-xd.tsv --suite discriminating --name xdsweep --jobs 32
    ~/.cache/sat13-rl/venv/bin/python tools/rl_dataset.py convert log/rl-xdsweep-<ts> --stock log/rl-stock2025-<ts>
    ~/.cache/sat13-rl/venv/bin/python tools/rl_xd_sweep.py report --stock log/rl-stock2025-<ts> [--sweep log/rl-xdsweep-<ts>]
"""
from __future__ import annotations

import argparse
import csv
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from rl_split import load_split  # noqa: E402

X_O = 1 << 23
XDS = {"xd26": 1 << 26, "xd27": 1 << 27, "xd28": 1 << 28}
TIMERS = ("probe", "eliminate", "reduce", "rephase", "reorder", "mode")
BRANCH_KNOBS = ("probe", "reduce", "mode")
# alternative entries per knob (the menu minus the parent's stock entry), as policy_fork forks them
BRANCH_MENU_SIZES = {"probe": (0, 0.5, 2, 4), "eliminate": (0, 0.5, 2, 4), "reduce": (0, 0.5, 2, 4),
                     "rephase": (0, 0.5, 2, 4), "reorder": (0, 0.5, 2, 4), "mode": (0.5, 2), "margin": (0.5, 2),
                     "sweep": (0, 0.5, 2)}
BRANCH_FRACTIONS = (0.2, 0.4, 0.6)


def load_cells(path: Path) -> dict[str, dict]:
    with open(path, newline="") as f:
        lines = [ln for ln in f if not ln.startswith("#")]
    return {r["stem"]: r for r in csv.DictReader(lines, dialect="excel-tab")}


def cmd_make(args) -> int:
    cells = load_cells(Path(args.cells))
    suite = ROOT / "benchmarks" / args.suite
    stems = sorted(p.name[:-len(".cnf.xz")] for p in suite.glob("*.cnf.xz"))
    rows = []
    skipped = []
    for stem in stems:
        c = cells.get(stem)
        if not c or not c.get("B_cell") or not c.get("decisions"):
            skipped.append(stem)
            continue
        b_cell = int(c["B_cell"])
        budget = min(b_cell, args.max_budget)
        rate = float(c["work_per_s"] or 0) or 1e7
        # the parent's wall covers its own run plus its children in waves of
        # --children: with three branch points the menus give 4 + 4 + 2
        # children, so the cap is twice that many single-run times
        n_children = sum(len(BRANCH_MENU_SIZES.get(k, ())) for k in BRANCH_KNOBS)
        waves = -(-n_children // max(args.children, 1))
        wall_cap = int(2 * (1 + waves) * budget / rate) + 120
        # decisions the stock run took at X_d = 2^27 within this budget: the
        # cells table counts them over the stock run's own work, which is what
        # the budget cuts (a timeout cell's work is its B_cell)
        stock_work = int(c.get("stock_work") or b_cell)
        dec27 = max(int(c["decisions"]) * min(1.0, budget / max(stock_work, 1)), 1.0)
        for tag, xd in XDS.items():
            n_dec = max(round(dec27 * (1 << 27) / xd), 1)
            points = sorted({max(1, round(f * n_dec)) for f in BRANCH_FRACTIONS})
            # one knob per branch point; distinct decisions (the solver refuses repeats)
            branch = ",".join(f"{d}:{k}" for d, k in zip(points, BRANCH_KNOBS))
            env = f"SAT_POLICY_EPOCH_TICKS={X_O},{xd} SAT_POLICY_BRANCH={branch} SAT_POLICY_BRANCH_JOBS={args.children}"
            rows.append({"stem": stem, "tag": tag, "flavour": "fork", "seed": int(c.get("seed") or 0), "limit_ticks": budget,
                         "wall_s": wall_cap, "procs": 1 + args.children, "peak_rss_mb": c.get("peak_rss_mb", ""),
                         "env": env})
    out = Path(args.out)
    with open(out, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=["stem", "tag", "flavour", "seed", "limit_ticks", "wall_s", "procs",
                                          "peak_rss_mb", "env"], dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        for r in rows:
            w.writerow(r)
    print(f"{out}: {len(rows)} jobs on {len(rows) // max(len(XDS), 1)} cells; skipped {len(skipped)} without B_cell/decisions")
    return 0


def cmd_report(args) -> int:
    import numpy as np
    import pyarrow.parquet as pq

    stock = Path(args.stock).resolve()
    split = load_split()
    shared = {s for s, k in split.items() if k == "shared"}   # in the 2026 holdout too: never evidence
    runs = pq.read_table(stock / "dataset" / "runs.parquet").to_pydict()
    fam = dict(zip(runs["stem"], runs["family"]))
    solved = dict(zip(runs["stem"], runs["solved"]))
    failed_stems = {s for s, f in zip(runs["stem"], runs.get("failed", [False] * len(runs["stem"]))) if f}
    dec_ticks_of = dict(zip(runs["stem"], runs.get("dec_ticks", [None] * len(runs["stem"]))))
    obs_ticks_of = dict(zip(runs["stem"], runs.get("obs_ticks", [None] * len(runs["stem"]))))
    decn_of = dict(zip(runs["stem"], runs.get("decisions_n", [None] * len(runs["stem"]))))
    if failed_stems:
        print(f"   WARNING: {len(failed_stems)} stock run(s) carry a correctness failure and are left out of part 1")
    # 1. fires per decision epoch and decisions per run, from the stock traces
    per_xd = {tag: defaultdict(list) for tag in XDS}
    n_cells = 0
    n_shared = 0
    logged_xd = None
    validation = []
    for f in sorted((stock / "dataset" / "rows").glob("*.parquet")):
        cols = ["boundary", "row", "grid"] + [f"tm_{t}_fires" for t in TIMERS]
        names = pq.read_schema(f).names
        t = pq.read_table(f, columns=[c for c in cols if c in names] + ["stem"])
        if t.num_rows == 0:
            continue
        d = t.to_pydict()
        stem = d["stem"][0]
        if stem in shared or stem in failed_stems:
            n_shared += stem in shared
            continue
        x_o = int(obs_ticks_of.get(stem) or 0)
        if x_o != X_O:
            # the grid is in the run's own X_o; the three X_d are multiples of 2^23
            print(f"   {stem[:40]}: left out, observation epoch {x_o} is not {X_O}")
            continue
        b = np.array(d["boundary"], bool)
        grid = np.array(d["grid"], dtype=np.int64)[b]
        n_cells += 1
        for tag, xd in XDS.items():
            k = xd // x_o
            # a decision is taken at every distinct decision-grid position a
            # boundary row lands on (D0 at position 0); the solver may skip
            # observation boundaries, so this is not the row count / k
            n_dec = len(set((grid // k).tolist())) if len(grid) else 0
            per_xd[tag]["decisions"].append(n_dec)
            for tmr in TIMERS:
                col = f"tm_{tmr}_fires"
                if col not in d:
                    continue
                fires = np.array(d[col], dtype=np.int64)[b]
                total = int(fires[-1] - fires[0]) if len(fires) else 0
                per_xd[tag][f"fires_{tmr}"].append(total / n_dec if n_dec else float("nan"))
        # the logged run took its decisions at its own X_d: the grid count must match its counter
        if dec_ticks_of.get(stem) and decn_of.get(stem) is not None:
            logged_xd = int(dec_ticks_of[stem])
            k = logged_xd // x_o
            validation.append(len(set((grid // k).tolist())) - int(decn_of[stem]))
    print(f"1. stock traces ({n_cells} cells; {n_shared} shared with the 2026 holdout left out), per X_d: "
          f"decisions per run and stock timer fires per decision epoch")
    if validation:
        v = np.array(validation)
        print(f"   (grid count v the logged decision count at the logged X_d = 2^{int(np.log2(logged_xd))}: "
              f"equal on {int(np.sum(v == 0))} of {len(v)} cells, max |difference| {int(np.max(np.abs(v)))})")
    print(f"   {'X_d':<6} {'dec/run med':>11} {'dec/run p10':>11} {'cells<10 dec':>12}  " + "  ".join(f"{t:>9}" for t in TIMERS))
    for tag, xd in XDS.items():
        dec = np.array(per_xd[tag]["decisions"], float)
        fires = []
        for tmr in TIMERS:
            v = np.array(per_xd[tag].get(f"fires_{tmr}", [float("nan")]), float)
            v = v[~np.isnan(v)]
            fires.append(float(np.median(v)) if len(v) else float("nan"))
        print(f"   {tag:<6} {np.median(dec):11.0f} {np.percentile(dec, 10):11.0f} {int((dec < 10).sum()):12d}  "
              + "  ".join(f"{x:9.2f}" for x in fires))
    print("   (fires per decision epoch: median over cells of the run's total fires / decision epochs)")
    if not args.sweep:
        print("2. no --sweep pass given: the paired-difference table needs the fork pass (make + rl_collect.py table)")
        return 0
    # 2. paired differences child v parent
    sweep = Path(args.sweep).resolve()
    sr = pq.read_table(sweep / "dataset" / "runs.parquet").to_pydict()
    by_id = {sr["run_id"][i]: {k: sr[k][i] for k in sr} for i in range(len(sr["run_id"]))}
    print(f"2. fork pass {sweep.name}: each child holds one knob entry for one decision epoch, then returns to")
    print("   stock; child and parent stop on the same budget. 'no-op' = the child ended with the parent's work and")
    print("   conflicts (its epoch held no fire of that timer, so nothing changed); the rest diverged.")
    print(f"   {'X_d':<6} {'knob':<8} {'children':>8} {'no-op':>6} {'diverged':>8} {'|dW|/W med':>10} {'|dW|/W p90':>10} "
          f"{'flips':>5} {'par solved':>10} {'child lost':>10} {'child won':>9}")
    n_failed = sum(1 for r in by_id.values() if r.get("failed"))
    if n_failed:
        print(f"   WARNING: {n_failed} run(s) carry a correctness failure (premature UNKNOWN, crash or contradiction) "
              f"and are left out; the pass needs debugging before its data is used")
    for tag in XDS:
        parents = [r for r in by_id.values() if r["tag"] == tag and not r["is_child"] and r["stem"] not in shared
                   and not r.get("failed")]
        rows_knob = defaultdict(list)
        n_anom = 0
        for p in parents:
            for c in by_id.values():
                if c["parent_run_id"] == p["run_id"]:
                    if p.get("anomaly") or c.get("anomaly") or c.get("failed"):
                        n_anom += 1          # a safety-cap stop is not a result (plan section 5.4)
                        continue
                    rows_knob[c["branch_knob"]].append((p, c))
        if n_anom:
            print(f"   {tag}: {n_anom} child(ren) excluded for a wall-cap or cut anomaly")
        for knob in list(BRANCH_KNOBS) + ["all"]:
            pairs = rows_knob[knob] if knob != "all" else [pc for k in rows_knob for pc in rows_knob[k]]
            if not pairs:
                continue
            noop = [(p, c) for p, c in pairs if c["work_end"] == p["work_end"] and c["conflicts_end"] == p["conflicts_end"]]
            div = [(p, c) for p, c in pairs if (p, c) not in noop]
            dw = np.array([abs(c["work_end"] - p["work_end"]) / p["work_end"] for p, c in div if p["work_end"]] or [np.nan])
            flips = sum(1 for p, c in div if bool(c["solved"]) != bool(p["solved"]))
            par_solved = sum(1 for p, c in div if p["solved"])
            lost = sum(1 for p, c in div if p["solved"] and not c["solved"])
            won = sum(1 for p, c in div if c["solved"] and not p["solved"])
            print(f"   {tag:<6} {knob:<8} {len(pairs):8d} {len(noop):6d} {len(div):8d} {np.nanmedian(dw):10.3f} "
                  f"{np.nanpercentile(dw, 90):10.3f} {flips:5d} {par_solved:10d} {lost:10d} {won:9d}")
    print("   (|dW|/W over diverged pairs; a child that lost its parent's solve ended at the budget, so its dW is the")
    print("    budget minus the parent's solve work)")
    print("3. overhead: logging per X_o measured nil in A.11; a decision is a few draws (random) or 50 us (net) per X_d.")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    m = sub.add_parser("make")
    m.add_argument("--cells", default=str(ROOT / "benchmarks" / "rl" / "cells_2025.tsv"))
    m.add_argument("--suite", default="discriminating")
    m.add_argument("--children", type=int, default=4, help="live children per parent (SAT_POLICY_BRANCH_JOBS)")
    m.add_argument("--max-budget", type=int, default=2_000_000_000,
                   help="cap on the tick budget per run (B_cell above it is cut; a timeout cell's B_cell is 1800 s of work)")
    m.add_argument("--out", required=True)
    m.set_defaults(fn=cmd_make)
    r = sub.add_parser("report")
    r.add_argument("--stock", required=True)
    r.add_argument("--sweep", default="")
    r.set_defaults(fn=cmd_report)
    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
