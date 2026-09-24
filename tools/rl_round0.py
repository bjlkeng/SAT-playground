#!/usr/bin/env python3
"""rl_round0.py — the round-0 branch-point schedule and job table.

Plan: plan/rl-scheduler-solver13-plan.md §5.3 (fork branching), §5.4 (run
mix), §6.1c "Round 0", §11 (decisions of 2026-09-18). Bead
SAT-playground-p9m.8.1 (step C.1); the tables feed C.2 and C.3.

Round 0 has no model, so branch points come from the stock trace of each
cell. One stock parent per cell carries every point; at a point the solver
forks one child per alternative entry of one knob's menu (4 for a 5-entry
knob, 2 for a 3-entry one, 3 for sweep effort) and each child holds its
entry for one decision epoch, then runs stock to the cell's work budget.
Two wild runs per cell (one segmented-sticky, one jitter) feed the
offline-RL side line and the flavour rebalancing of C.4.

Hard-cell focus (owner's steer, 2026-09-18). Cells solved under 60 s hold
1 % of all epochs and take at most a few decisions; timeouts hold 69 %.
So the mix is:

  class         cells (train)   budget        points               window
  fast          solved < 60 s   B_cell        min(decisions, 3)    all
  slow          solved >= 60 s  B_cell        decisions / 8 in     10-90 %
                                              [4, 24]
  band_solved   band, solves    B_cell_band   16                   20-90 %
                at 3600 s
  band_timeout  band, timeout   B_cell_band   8                    30-90 %
                at 3600 s

The band cells run only at the 3600 s budget: the stock trajectory is
deterministic, so the 1800 s run is a prefix of the 3600 s one and a
second pass at B_cell would only duplicate it. The window is a share of
the parent's own decisions (a child can only fork where the parent still
runs; on a solved cell the parent ends at a third of B_cell).

Which knob at a point. Knobs are drawn with the step-0 per-knob oracle
gains as weights (plan §11, 2026-09-16: reduce 16.0, mode 15.9, reorder
15.5, eliminate 14.4, probe 14.3, rephase 12.9, margin 12.4, sweep 8.4).
Half of a cell's points are "timer-due": the decision is drawn from those
where the knob's timer fires within the coming decision epoch in the stock
trace (the solver's fire counters per row, cumulative), so the delay
entries (2x, 4x) get labels where they act; the other half are uniform
over the window. Rephase is masked to stock in focused mode and the
restart margin in stable mode (policy.rs), so those knobs are only placed
at decisions where the trace is in the right mode. Sweep effort is due
when probe is; the margin is due in focused mode. Each decision carries
at most one point (the solver refuses repeats).

    ~/.cache/sat13-rl/venv/bin/python tools/rl_round0.py make \\
        --stock log/rl-stock2025-<ts> --band log/rl-band2025-<ts> \\
        [--out jobs-round0.tsv] [--seed 20260918] [--branch-jobs 6] [--slots 28]
    ~/.cache/sat13-rl/venv/bin/python tools/rl_round0.py show jobs-round0.tsv [--cells N]
    python3 tools/rl_collect.py table jobs-round0.tsv --suite sat-comp-2025 --name round0 \\
        --jobs 28 --mem-mb 16000 --mem-total-mb 420000 --oracle ... --oracle ...

Without --out, `make` is a dry run: it prints the schedule per cell and
the totals and writes nothing. With --out it also writes
<out>.schedule.tsv, one row per cell with the class, the decision count,
the points, the children and the projected work.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import math
import random
import sys
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from rl_split import assert_training_only, training_cells  # noqa: E402

CELLS = ROOT / "benchmarks" / "rl" / "cells_2025.tsv"
JOB_COLUMNS = ("stem", "tag", "flavour", "seed", "limit_ticks", "wall_s", "procs", "peak_rss_mb", "env")

# step-0 per-knob oracle tick gain in percent (plan §11, 2026-09-16)
KNOB_WEIGHTS = {"reduce": 16.0, "mode": 15.9, "reorder": 15.5, "eliminate": 14.4, "probe": 14.3,
                "rephase": 12.9, "margin": 12.4, "sweep": 8.4}
KNOBS = tuple(KNOB_WEIGHTS)
# children a point forks: the knob's menu minus the parent's stock entry (policy_fork.rs)
ALT_ENTRIES = {"probe": 4, "eliminate": 4, "reduce": 4, "rephase": 4, "reorder": 4, "mode": 2, "margin": 2,
               "sweep": 3}
# the trace's fire counter a knob's "due" test reads; the margin has no timer
TIMER_OF = {"probe": "probe", "eliminate": "eliminate", "reduce": "reduce", "rephase": "rephase",
            "reorder": "reorder", "mode": "mode", "sweep": "probe", "margin": None}
TIMERS = ("probe", "eliminate", "reduce", "rephase", "reorder", "mode")

# per class: points, window of the parent's decisions, wild runs
CLASSES = {
    "fast": {"budget": "B_cell", "trace": "stock", "window": (0.0, 1.0), "wild": True},
    "slow": {"budget": "B_cell", "trace": "stock", "window": (0.1, 0.9), "wild": True},
    "band_solved": {"budget": "B_cell_band", "trace": "band", "window": (0.2, 0.9), "wild": True},
    "band_timeout": {"budget": "B_cell_band", "trace": "band", "window": (0.3, 0.9), "wild": True},
}
FAST_SECONDS = 60.0
SEGMENT_TEMP = 1.0     # plan §5.3: near-stock sticky segments
SEGMENT_MEAN = 5       # decision epochs per segment (mean)
JITTER_TEMP = 0.5      # plan §5.3: per-epoch resampling at low temperature
DEFAULT_RATE = 3e7     # W per second when a cell has no rate (never on solved cells)


def load_cells(path: Path) -> dict[str, dict]:
    with open(path, newline="") as f:
        lines = [ln for ln in f if not ln.startswith("#")]
    return {r["stem"]: r for r in csv.DictReader(lines, dialect="excel-tab")}


def fnum(x) -> float | None:
    try:
        return float(x)
    except (TypeError, ValueError):
        return None


def cell_class(c: dict) -> str | None:
    """The round-0 class of a cells-table row, or None when it has no budget
    (a failed or memory-aborted run, or a band cell without a band record)."""
    if c.get("failed") not in ("0", "", None):
        return None
    if c.get("band") == "1":
        st = c.get("band_status") or ""
        if not c.get("B_cell_band"):
            return None
        return "band_solved" if st in ("SAT", "UNSAT") else ("band_timeout" if st == "TIMEOUT" else None)
    if c.get("status") not in ("SAT", "UNSAT") or not c.get("B_cell"):
        return None
    t = fnum(c.get("stock_time_s")) or 0.0
    return "fast" if t < FAST_SECONDS else "slow"


def points_for(cls: str, n_dec: int) -> int:
    if cls == "fast":
        return min(n_dec, 3)
    if cls == "slow":
        return min(n_dec, max(4, min(24, round(n_dec / 8))))
    if cls == "band_solved":
        return min(n_dec, 16)
    return min(n_dec, 8)


def read_trace(run_dir: Path, stem: str, tag: str):
    """Per decision of the parent trace: the row index, the work at the
    row, the mode (1 = stable) and the cumulative fires of each timer; plus
    the final row's fires (so the last decision has a 'next' to compare
    with). Returns None when the converted rows are missing."""
    import numpy as np
    import pyarrow.parquet as pq

    f = run_dir / "dataset" / "rows" / f"{stem}.{tag}.parquet"
    if not f.is_file():
        return None
    cols = ["row", "is_decision", "stable", "work"] + [f"tm_{t}_fires" for t in TIMERS]
    t = pq.read_table(f, columns=cols).to_pydict()
    n = len(t["row"])
    if n == 0:
        return None
    isdec = np.asarray(t["is_decision"], dtype=bool)
    idx = np.nonzero(isdec)[0]
    fires = {tm: np.asarray(t[f"tm_{tm}_fires"], dtype=np.int64) for tm in TIMERS}
    return {
        "n_rows": n,
        "dec_rows": idx,
        "work": np.asarray(t["work"], dtype=np.float64)[idx],
        "stable": np.asarray(t["stable"], dtype=np.float64)[idx] > 0.5,
        # fires within the epoch after decision d: counter at the next
        # decision row (or the final row for the last decision) minus at d
        "due": {tm: fires[tm][np.append(idx[1:], n - 1)] - fires[tm][idx] > 0 for tm in TIMERS},
    }


def knob_allowed(knob: str, stable: bool) -> bool:
    if knob == "rephase":
        return stable
    if knob == "margin":
        return not stable
    return True


def draw_schedule(trace, cls: str, rng: random.Random) -> list[tuple[int, str, str]]:
    """[(decision, knob, kind)] with kind 'due' or 'uniform', sorted by decision."""
    n_dec = len(trace["dec_rows"])
    total = points_for(cls, n_dec)
    lo, hi = CLASSES[cls]["window"]
    if cls == "fast":
        window = list(range(n_dec))
    else:
        d_lo = math.ceil(lo * (n_dec - 1))
        d_hi = math.floor(hi * (n_dec - 1))
        window = list(range(d_lo, d_hi + 1))
    if len(window) < total:
        window = list(range(n_dec))
    weights = [KNOB_WEIGHTS[k] for k in KNOBS]
    used: set[int] = set()
    out: list[tuple[int, str, str]] = []
    n_due = total // 2
    # timer-due points first: pick the knob, then a due decision for it
    tries = 0
    while len(out) < n_due and tries < 20 * total:
        tries += 1
        knob = rng.choices(KNOBS, weights)[0]
        timer = TIMER_OF[knob]
        cands = [d for d in window if d not in used and knob_allowed(knob, trace["stable"][d])
                 and (timer is None or trace["due"][timer][d])]
        if not cands:
            continue
        d = rng.choice(cands)
        used.add(d)
        out.append((d, knob, "due"))
    # uniform points: pick the decision, then a knob the mask allows there
    tries = 0
    while len(out) < total and tries < 20 * total:
        tries += 1
        free = [d for d in window if d not in used]
        if not free:
            break
        d = rng.choice(free)
        allowed = [k for k in KNOBS if knob_allowed(k, trace["stable"][d])]
        knob = rng.choices(allowed, [KNOB_WEIGHTS[k] for k in allowed])[0]
        used.add(d)
        out.append((d, knob, "uniform"))
    return sorted(out)


def cell_seed(seed: int, stem: str) -> int:
    h = hashlib.sha256(f"{seed}:{stem}".encode()).digest()
    return int.from_bytes(h[:8], "little")


def make_jobs(args) -> tuple[list[dict], list[dict], dict]:
    cells = load_cells(Path(args.cells))
    train = training_cells()
    stems = sorted(s for s in cells if s in train)
    assert_training_only(stems)
    traces = {"stock": Path(args.stock).resolve(), "band": Path(args.band).resolve()}
    jobs: list[dict] = []
    sched: list[dict] = []
    skipped: Counter = Counter()
    for stem in stems:
        c = cells[stem]
        cls = cell_class(c)
        if cls is None:
            skipped["no budget"] += 1
            continue
        spec = CLASSES[cls]
        trace = read_trace(traces[spec["trace"]], stem, "stock")
        if trace is None:
            skipped["no converted trace"] += 1
            continue
        if len(trace["dec_rows"]) == 0:
            # solved before the first observation epoch: nothing to branch at
            skipped["no decisions"] += 1
            continue
        budget = int(c[spec["budget"]])
        rate = fnum(c.get("work_per_s")) or DEFAULT_RATE
        rss = max(fnum(c.get("peak_rss_mb")) or 0.0, fnum(c.get("band_peak_rss_mb")) or 0.0)
        rng = random.Random(cell_seed(args.seed, stem))
        points = draw_schedule(trace, cls, rng)
        n_children = sum(ALT_ENTRIES[k] for _, k, _ in points)
        # projected work: parent to its own end (or the budget), children
        # from their branch point to the budget, wild runs to the budget
        parent_work = min(float(c.get("band_work" if cls.startswith("band") else "stock_work") or budget),
                          float(budget))
        child_work = sum(ALT_ENTRIES[k] * max(budget - float(trace["work"][d]), 0.0) for d, k, _ in points)
        wild_work = 2.0 * budget if spec["wild"] else 0.0
        waves = math.ceil(n_children / max(args.branch_jobs, 1)) if n_children else 0
        fork_cap = int(2.0 * (1 + waves) * budget / rate) + 600
        branch = ",".join(f"{d}:{k}" for d, k, _ in points)
        seed = int(c.get("seed") or 0)
        env = f"SAT_POLICY_BRANCH={branch} SAT_POLICY_BRANCH_JOBS={args.branch_jobs}"
        jobs.append({"stem": stem, "tag": "fork", "flavour": "fork", "seed": seed, "limit_ticks": budget,
                     "wall_s": fork_cap, "procs": 1 + args.branch_jobs, "peak_rss_mb": f"{rss:.1f}" if rss else "",
                     "env": env})
        if spec["wild"]:
            wild_cap = int(2.0 * budget / rate) + 600
            pseed = rng.randrange(1, 2**31)
            jobs.append({"stem": stem, "tag": "seg", "flavour": "random", "seed": seed, "limit_ticks": budget,
                         "wall_s": wild_cap, "procs": 1, "peak_rss_mb": f"{rss:.1f}" if rss else "",
                         "env": f"SAT_POLICY_SEED={pseed} SAT_POLICY_TEMP={SEGMENT_TEMP} SAT_POLICY_SEGMENT={SEGMENT_MEAN}"})
            jobs.append({"stem": stem, "tag": "jit", "flavour": "jitter", "seed": seed, "limit_ticks": budget,
                         "wall_s": wild_cap, "procs": 1, "peak_rss_mb": f"{rss:.1f}" if rss else "",
                         "env": f"SAT_POLICY_SEED={pseed + 1} SAT_POLICY_TEMP={JITTER_TEMP}"})
        sched.append({"stem": stem, "class": cls, "family": c.get("family", ""), "decisions": len(trace["dec_rows"]),
                      "points": len(points), "due": sum(1 for p in points if p[2] == "due"),
                      "children": n_children, "budget": budget, "work_per_s": int(rate),
                      "parent_work": int(parent_work), "child_work": int(child_work), "wild_work": int(wild_work),
                      "core_s": int((parent_work + child_work + wild_work) / rate),
                      "knobs": " ".join(f"{d}:{k}{'*' if kind == 'due' else ''}" for d, k, kind in points)})
    return jobs, sched, dict(skipped)


def summarize(sched: list[dict], skipped: dict, slots: int, branch_jobs: int) -> str:
    lines = []
    by = defaultdict(list)
    for s in sched:
        by[s["class"]].append(s)
    lines.append(f"{'class':13s} {'cells':>5s} {'points':>6s} {'due':>5s} {'children':>8s} {'core-h':>7s}  "
                 f"{'children/cell':>13s}")
    tot = Counter()
    for cls in CLASSES:
        rs = by.get(cls, [])
        if not rs:
            continue
        pts = sum(r["points"] for r in rs)
        due = sum(r["due"] for r in rs)
        kids = sum(r["children"] for r in rs)
        core_h = sum(r["core_s"] for r in rs) / 3600
        ks = sorted(r["children"] for r in rs)
        lines.append(f"{cls:13s} {len(rs):5d} {pts:6d} {due:5d} {kids:8d} {core_h:7.0f}  "
                     f"median {ks[len(ks) // 2]:3d} range {ks[0]}-{ks[-1]}")
        tot["cells"] += len(rs); tot["points"] += pts; tot["due"] += due; tot["children"] += kids
        tot["core_h"] += core_h
    lines.append(f"{'total':13s} {tot['cells']:5d} {tot['points']:6d} {tot['due']:5d} {tot['children']:8d} "
                 f"{tot['core_h']:7.0f}")
    knobs = Counter()
    for s in sched:
        for item in s["knobs"].split():
            knobs[item.split(":")[1].rstrip("*")] += 1
    lines.append("points per knob: " + ", ".join(f"{k} {knobs[k]}" for k in KNOBS))
    # a fork parent's slot idles while it waits for a child slot, so the
    # useful share of a fork job's cores is about branch_jobs / (1 + branch_jobs)
    eff = slots * branch_jobs / (1 + branch_jobs)
    lines.append(f"projected host time at {slots} slots (about {eff:.0f} busy): {tot['core_h'] / eff / 24:.1f} days"
                 f" ({tot['core_h']:.0f} core-hours; children stop at the cell budget, so ticks not wall decide this)")
    if skipped:
        lines.append("skipped: " + ", ".join(f"{k} {v}" for k, v in skipped.items()))
    return "\n".join(lines)


def cmd_make(args) -> int:
    jobs, sched, skipped = make_jobs(args)
    if args.verbose or not args.out:
        for s in sched:
            print(f"{s['stem'][:48]:48s} {s['class']:12s} dec {s['decisions']:4d} pts {s['points']:2d} "
                  f"(due {s['due']:2d}) children {s['children']:3d} budget {s['budget']:.2e} "
                  f"core-s {s['core_s']:6d}  {s['knobs']}")
    print(summarize(sched, skipped, args.slots, args.branch_jobs))
    if not args.out:
        print("(dry run: no --out, nothing written)")
        return 0
    out = Path(args.out)
    with open(out, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(JOB_COLUMNS), dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        for j in jobs:
            w.writerow(j)
    sp = out.with_name(out.name + ".schedule.tsv")
    with open(sp, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(sched[0].keys()), dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        for s in sched:
            w.writerow(s)
    print(f"wrote {out}: {len(jobs)} jobs on {len(sched)} cells; schedule in {sp}")
    return 0


def cmd_show(args) -> int:
    with open(args.table, newline="") as f:
        rows = list(csv.DictReader(f, dialect="excel-tab"))
    by_stem: dict[str, list[dict]] = defaultdict(list)
    for r in rows:
        by_stem[r["stem"]].append(r)
    n = 0
    for stem, rs in by_stem.items():
        if args.cells and n >= args.cells:
            break
        n += 1
        for r in rs:
            env = dict(kv.split("=", 1) for kv in r["env"].split())
            if r["flavour"] == "fork":
                pts = env["SAT_POLICY_BRANCH"].split(",")
                kids = sum(ALT_ENTRIES[p.split(":")[1]] for p in pts)
                print(f"{stem[:48]:48s} fork  budget {int(r['limit_ticks']):.2e} cap {r['wall_s']:>6s} s "
                      f"procs {r['procs']} points {len(pts)} children {kids}: {' '.join(pts)}")
            else:
                print(f"{stem[:48]:48s} {r['tag']:5s} budget {int(r['limit_ticks']):.2e} cap {r['wall_s']:>6s} s "
                      f"{r['env']}")
    print(f"{len(by_stem)} cells, {len(rows)} jobs")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    m = sub.add_parser("make", help="build the round-0 job table (dry run without --out)")
    m.add_argument("--cells", default=str(CELLS))
    m.add_argument("--stock", required=True, help="the stock pass directory (converted, plan §5.2)")
    m.add_argument("--band", required=True, help="the 3600 s band pass directory (converted)")
    m.add_argument("--out", help="job table to write (plus <out>.schedule.tsv)")
    m.add_argument("--seed", type=int, default=20260918)
    m.add_argument("--branch-jobs", type=int, default=6, help="live children per parent (SAT_POLICY_BRANCH_JOBS)")
    m.add_argument("--slots", type=int, default=28, help="process slots the pass will run on (projection only)")
    m.add_argument("--verbose", action="store_true", help="print the per-cell schedule even with --out")
    m.set_defaults(func=cmd_make)
    s = sub.add_parser("show", help="print the schedule of a job table")
    s.add_argument("table")
    s.add_argument("--cells", type=int, default=0, help="first N cells only")
    s.set_defaults(func=cmd_show)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
