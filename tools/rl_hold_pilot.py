#!/usr/bin/env python3
"""rl_hold_pilot.py — does a deviation held for several epochs carry a consistent label?

Plan: plan/rl-scheduler-solver13-plan.md §9 (the parked "hold delay entries
for several epochs" option), §11 (2026-09-28: the round-1 result). Bead
SAT-playground-p9m.16 (the go/no-go after round 1).

Rounds 0 and 1 showed that which one-epoch deviation wins is not
predictable from the state: a one-epoch change is a nudge, the runs then
drift apart like two seeds, and the winner is a coin flip with a negative
mean. A change held for many epochs is a strategy difference with a much
larger effect (the segmented runs changed 62-72 % of outcomes against 49 %
for a one-epoch child), and its sign may be consistent for a kind of
state. This pilot measures that before any full round.

`make`: 20 training cells stock solves in 60-1800 s with at least
--min-decisions decisions and a budget at most --max-budget, one per
family (the cell whose decision count is nearest the median of its
family), each with the same six branch points: two nearby points per knob
for probe, reduce and mode (at 30 and 34, 50 and 54, 70 and 74 % of the
stock run's decisions), and three fork jobs per cell holding the entry
for 1, 4 and 16 decision epochs (`SAT_POLICY_BRANCH_HOLD`). Same stock
parent, same budget B_cell, so the three holds differ only in how long a
child keeps its entry.

`report`: from the converted pass (refused unless the pass is DONE, every
job has its run and every forked child its row in the dataset, so that a
subset conversion cannot pass for the whole), per hold: how often a
child's outcome or work differs from the parent's, beats or loses, and the spread of the
work ratio (the effect size); and the consistency of the label between
the two nearby points of the same cell and knob: for each entry, whether
the two points give the same label (better / worse / tie / censored)
and, among both-solved pairs, whether the work ratios have the same sign.
Each agreement is printed with its chance level (the same statistic after
re-pairing the second point's rows across the cells of the same knob,
which keeps how often the entry helps or hurts at that hold) and the
excess over it, because a hold that hurts more often raises raw agreement
by itself. If the excess rises with the hold, a longer hold produces
learnable labels; if it stays near zero, it does not.

    ~/.cache/sat13-rl/venv/bin/python tools/rl_hold_pilot.py make --stock log/rl-stock2025-<ts> \\
        --out benchmarks/rl/holdpilot_jobs.tsv
    python3 tools/rl_collect.py table benchmarks/rl/holdpilot_jobs.tsv --suite sat-comp-2025 --name holdpilot --jobs 28 ...
    ~/.cache/sat13-rl/venv/bin/python tools/rl_dataset.py convert log/rl-holdpilot-<ts>
    ~/.cache/sat13-rl/venv/bin/python tools/rl_hold_pilot.py report log/rl-holdpilot-<ts>
"""
from __future__ import annotations

import argparse
import csv
import math
import random
import statistics
import sys
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from rl_split import assert_training_only, training_cells  # noqa: E402

CELLS = ROOT / "benchmarks" / "rl" / "cells_2025.tsv"
JOB_COLUMNS = ("stem", "tag", "flavour", "seed", "limit_ticks", "wall_s", "procs", "peak_rss_mb", "env")
HOLDS = (1, 4, 16)
POINTS = (("probe", 0.30), ("probe", 0.34), ("reduce", 0.50), ("reduce", 0.54), ("mode", 0.70), ("mode", 0.74))
ALT_ENTRIES = {"probe": 4, "reduce": 4, "mode": 2}
SOLVED = ("SATISFIABLE", "UNSATISFIABLE", "SAT", "UNSAT")


def load_cells(path: Path) -> dict[str, dict]:
    with open(path, newline="") as f:
        lines = [ln for ln in f if not ln.startswith("#")]
    return {r["stem"]: r for r in csv.DictReader(lines, dialect="excel-tab")}


def cmd_make(args) -> int:
    cells = load_cells(Path(args.cells))
    train = training_cells()
    pool = [c for s, c in cells.items() if s in train and c.get("status") in ("SAT", "UNSAT")
            and c.get("failed") in ("0", "", None) and 60.0 <= float(c.get("stock_time_s") or 0) < 1800.0
            and int(c.get("decisions") or 0) >= args.min_decisions and 0 < int(c.get("B_cell") or 0) <= args.max_budget]
    by_family: dict[str, list[dict]] = defaultdict(list)
    for c in pool:
        by_family[c["family"]].append(c)
    # one cell per family, the one nearest its family's median decision count;
    # families with the most candidates first, up to --cells
    chosen = []
    for fam, cs in sorted(by_family.items(), key=lambda kv: (-len(kv[1]), kv[0])):
        med = statistics.median(int(c["decisions"]) for c in cs)
        chosen.append(min(cs, key=lambda c: (abs(int(c["decisions"]) - med), c["stem"])))
        if len(chosen) >= args.cells_n:
            break
    assert_training_only([c["stem"] for c in chosen])
    jobs = []
    for c in chosen:
        n_dec = int(c["decisions"])
        budget = int(c["B_cell"])
        rate = float(c.get("work_per_s") or 0) or 3e7
        used = set()
        points = []
        for knob, frac in POINTS:
            d = max(1, min(n_dec - 1, round(frac * n_dec)))
            while d in used:
                d += 1
            used.add(d)
            points.append((d, knob))
        points.sort()
        branch = ",".join(f"{d}:{k}" for d, k in points)
        n_children = sum(ALT_ENTRIES[k] for _, k in points)
        waves = math.ceil(n_children / args.branch_jobs)
        cap = int(2.0 * (1 + waves) * budget / rate) + 600
        for hold in HOLDS:
            jobs.append({"stem": c["stem"], "tag": f"h{hold}", "flavour": "fork", "seed": int(c.get("seed") or 0),
                         "limit_ticks": budget, "wall_s": cap, "procs": 1 + args.branch_jobs,
                         "peak_rss_mb": c.get("peak_rss_mb", ""),
                         "env": f"SAT_POLICY_BRANCH={branch} SAT_POLICY_BRANCH_JOBS={args.branch_jobs} "
                                f"SAT_POLICY_BRANCH_HOLD={hold}"})
    with open(args.out, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(JOB_COLUMNS), dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        for j in jobs:
            w.writerow(j)
    core_h = sum(int(j["limit_ticks"]) * (1 + 0.6 * 20) / (float(cells[j["stem"]].get("work_per_s") or 3e7))
                 for j in jobs) / 3600
    print(f"wrote {args.out}: {len(jobs)} fork jobs on {len(chosen)} cells ({len(pool)} candidates in "
          f"{len(by_family)} families), holds {HOLDS}, {len(POINTS)} points and 20 children per job; "
          f"about {core_h:.0f} core-hours if every child ran to the budget")
    for c in chosen:
        print(f"  {c['family']:22s} {c['stem'][:40]:40s} dec {int(c['decisions']):4d} t {float(c['stock_time_s']):6.0f} s "
              f"B {float(c['B_cell']):.1e}")
    return 0


# ---------------------------------------------------------------------------
# report
# ---------------------------------------------------------------------------

def label(child_solved, child_work, parent_solved, parent_work, margin=0.01) -> str:
    if child_solved and not parent_solved:
        return "better"
    if parent_solved and not child_solved:
        return "worse"
    if not child_solved:
        return "censored"
    if child_work < (1 - margin) * parent_work:
        return "better"
    if child_work > (1 + margin) * parent_work:
        return "worse"
    return "tie"


def cmd_report(args) -> int:
    import pyarrow.parquet as pq

    run = Path(args.run_dir).resolve()
    # The collector's own records first (as tools/rl_round0_report.py): a
    # job that failed before writing a readable log has no run in the
    # dataset, so completeness is checked on results.tsv against the job
    # table. Then the conversion's coverage: a `convert --jobs` subset or a
    # child log the converter could not read would leave the report on a
    # subset that can differ by hold (a memory-stopped child, say), and the
    # hold comparison would be biased, so that is refused outright.
    with open(run / "jobs.tsv", newline="") as f:
        job_keys = {f"{j['stem']}.{j['tag']}" for j in csv.DictReader(f, dialect="excel-tab")}
    with open(run / "results.tsv", newline="") as f:
        results = {f"{x['stem']}.{x['tag']}": x for x in csv.DictReader(f, dialect="excel-tab")}
    failed_jobs = sorted(k for k, x in results.items() if x.get("failed", "0") == "1")
    if failed_jobs:
        print(f"*** {len(failed_jobs)} job(s) flagged by the collector (verify, oracle, sibling agreement, premature "
              f"UNKNOWN or crash), e.g. {failed_jobs[:3]}: a correctness failure, no report")
        return 1
    missing_jobs = sorted(job_keys - set(results))
    if missing_jobs or not (run / "DONE").is_file():
        print(f"*** pass not complete: {len(missing_jobs)} job(s) without a record, "
              f"DONE {'present' if (run / 'DONE').is_file() else 'absent'}")
        if not args.partial:
            return 1
        print("*** --partial: a peek at an unfinished pass, the holds may be unevenly covered; not a readout")
    r = pq.read_table(run / "dataset" / "runs.parquet").to_pydict()
    n = len(r["run_id"])
    flagged = sum(1 for x in r["failed"] if x)
    if flagged:
        print(f"*** {flagged} run(s) flagged in the dataset: a correctness failure, no report")
        return 1
    ds_keys = {r["run_id"][i].split("/")[-1] for i in range(n) if not r["is_child"][i]}
    n_children_ds = Counter(r["parent_run_id"][i].split("/")[-1] for i in range(n) if r["is_child"][i])
    unconverted = sorted(k for k in results if k not in ds_keys)
    unreadable = sum(max(int(x.get("children") or 0) - n_children_ds.get(k, 0), 0)
                     for k, x in results.items() if x.get("flavour") == "fork")
    if unconverted or unreadable:
        print(f"*** conversion incomplete: {len(unconverted)} collected job(s) without a run in the dataset, "
              f"{unreadable} forked child(ren) without a row (e.g. {unconverted[:3]}); convert the whole pass first")
        return 1
    parent = {r["run_id"][i]: i for i in range(n) if not r["is_child"][i]}
    # answers must agree across every run of a cell
    answers: dict[str, set] = defaultdict(set)
    for i in range(n):
        if r["solved"][i] and r["anomaly"][i] != "wall_cap":
            answers[r["stem"][i]].add(str(r["result"][i]).upper()[:3])
    contradictions = [s for s, a in answers.items() if len(a) > 1]
    if contradictions:
        print(f"*** CORRECTNESS FAILURE: contradicting answers on {contradictions[:5]}")
        return 1
    # per (hold, stem, decision, knob, entry): the child's label against the
    # parent. A run the safety wall cap ended (anomaly wall_cap) has no
    # outcome at the budget: such a child is dropped, and a capped parent
    # drops its whole set. A child stopped by memory (anomaly cut, no
    # failure flag) is an unsolved outcome, like a budget stop.
    def finite(x):
        return float(x) if x is not None and x == x else 0.0
    rows: dict[tuple, dict] = {}
    for i in range(n):
        if not r["is_child"][i] or r["anomaly"][i] == "wall_cap":
            continue
        p = parent[r["parent_run_id"][i]]
        if r["anomaly"][p] == "wall_cap":
            continue
        child_solved = bool(r["solved"][i]) and r["anomaly"][i] != "cut"
        parent_solved = bool(r["solved"][p]) and r["anomaly"][p] != "cut"
        hold = int(r["tag"][i][1:])
        key = (hold, r["stem"][i], int(r["branch_decision"][i]), r["branch_knob"][i], float(r["branch_entry"][i]))
        cw, pw = finite(r["work_end"][i]), finite(r["work_end"][p])
        rows[key] = {"label": label(child_solved, cw, parent_solved, pw),
                     "ratio": math.log(cw / pw) if child_solved and parent_solved and cw > 0 and pw > 0 else None,
                     "parent_solved": parent_solved}
    print(f"{run.name}: {len(rows)} children with a known outcome, {len({k[1] for k in rows})} cells\n")
    # 1. effect size per hold
    print(f"{'hold':>4s} {'children':>8s} {'moved%':>7s} {'better%':>8s} {'worse%':>7s} {'tie%':>5s} {'cens%':>6s} "
          f"{'|logW| median':>14s} {'p90':>6s} {'rescue':>9s}")
    for hold in HOLDS:
        sel = [v for k, v in rows.items() if k[0] == hold]
        if not sel:
            continue
        lab = Counter(v["label"] for v in sel)
        ordered = len(sel) - lab["censored"]
        abs_ratio = sorted(abs(v["ratio"]) for v in sel if v["ratio"] is not None)
        unsolved = [v for v in sel if not v["parent_solved"]]
        rescued = sum(1 for v in unsolved if v["label"] == "better")
        med = abs_ratio[len(abs_ratio) // 2] if abs_ratio else float("nan")
        p90 = abs_ratio[int(0.9 * (len(abs_ratio) - 1))] if abs_ratio else float("nan")
        print(f"{hold:4d} {len(sel):8d} {100 * (lab['better'] + lab['worse']) / max(ordered, 1):7.1f} "
              f"{100 * lab['better'] / max(ordered, 1):8.1f} {100 * lab['worse'] / max(ordered, 1):7.1f} "
              f"{100 * lab['tie'] / max(ordered, 1):5.1f} {100 * lab['censored'] / max(len(sel), 1):6.1f} "
              f"{med:14.3f} {p90:6.3f} {rescued:4d}/{len(unsolved):<4d}")
    # 2. consistency between the two nearby points of the same cell and knob,
    # per hold. Every statistic is printed next to its chance level: the
    # same statistic after re-pairing the second point's rows across the
    # cells with the same knob (`--shuffles` random re-pairings), which
    # keeps every marginal rate (how often the entry helps or hurts at a
    # hold) and removes only the link between the two points of one cell.
    # A held change that hurts more often raises the raw agreement by
    # itself; only the excess over chance says the two nearby states agree
    # on the direction. `p` is the share of re-pairings at or above the
    # observed value.
    log_margin = math.log(1.01)

    def best(x):
        # the best entry at a point: the smallest work ratio among solved children, else none
        cand = [(v["ratio"], e) for e, v in x.items() if v["ratio"] is not None]
        return min(cand)[1] if cand else None

    def consistency(pairs):
        c = Counter()
        for a, b in pairs:
            for e in set(a) & set(b):
                la, lb = a[e]["label"], b[e]["label"]
                c["pairs"] += 1
                c["same"] += la == lb
                if la != "tie" and lb != "tie" and "censored" not in (la, lb):
                    c["nontie"] += 1
                    c["same_nontie"] += la == lb
                # the direction statistic: both solved at both points and
                # neither within the tie margin, so a no-effect branch cannot
                # pass for agreement with an improving one
                ra, rb = a[e]["ratio"], b[e]["ratio"]
                if ra is not None and rb is not None and abs(ra) > log_margin and abs(rb) > log_margin:
                    c["solved2"] += 1
                    c["same_sign"] += (ra > 0) == (rb > 0)
            ba, bb = best(a), best(b)
            if ba is not None and bb is not None:
                c["best_pairs"] += 1
                c["best_same"] += ba == bb
        return c

    STATS = (("same label", "same", "pairs"), ("same non-tie label", "same_nontie", "nontie"),
             ("same ratio sign", "same_sign", "solved2"), ("best entry agrees", "best_same", "best_pairs"))
    rng = random.Random(args.seed)
    print(f"\nconsistency of the label between the two nearby points of one cell and knob (same entry), "
          f"against chance ({args.shuffles} re-pairings across cells within a knob):")
    print(f"{'hold':>4s} {'statistic':<19s} {'n':>5s} {'observed%':>9s} {'chance%':>8s} {'excess':>7s} {'p':>6s}")
    for hold in HOLDS:
        by = defaultdict(dict)   # (stem, knob) -> {decision: {entry: row}}
        for (h, stem, d, knob, e), v in rows.items():
            if h == hold:
                by[(stem, knob)].setdefault(d, {})[e] = v
        groups = defaultdict(list)   # knob -> [(first point rows, second point rows)] over cells
        for (stem, knob), decs in by.items():
            ds = sorted(decs)
            if len(ds) >= 2:
                groups[knob].append((decs[ds[0]], decs[ds[1]]))
        pairs = [ab for g in groups.values() for ab in g]
        if not pairs:
            continue
        obs = consistency(pairs)
        shuffled = []
        for _ in range(args.shuffles):
            re_paired = []
            for g in groups.values():
                seconds = [b for _, b in g]
                rng.shuffle(seconds)
                re_paired.extend(zip((a for a, _ in g), seconds))
            shuffled.append(consistency(re_paired))
        for name, num, den in STATS:
            o = obs[num] / obs[den] if obs[den] else float("nan")
            ch = [c[num] / c[den] for c in shuffled if c[den]]
            mean = sum(ch) / len(ch) if ch else float("nan")
            pv = sum(1 for x in ch if x >= o) / len(ch) if ch and obs[den] else float("nan")
            print(f"{hold:4d} {name:<19s} {obs[den]:5d} {100 * o:9.1f} {100 * mean:8.1f} {100 * (o - mean):+7.1f} {pv:6.2f}")
    print("\nreading: 'excess' is the agreement the two nearby points have beyond what the hold's own helps/hurts "
          "rates give at random. Near zero (p not small) at every hold means the two points disagree at random on "
          "whether the entry helps; an excess that grows with the hold means a held change has a consistent direction.")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    m = sub.add_parser("make")
    m.add_argument("--cells", default=str(CELLS))
    m.add_argument("--stock", help="unused; the stock pass's cells table is read (kept for the recipe)")
    m.add_argument("--cells-n", dest="cells_n", type=int, default=20)
    m.add_argument("--min-decisions", type=int, default=30)
    m.add_argument("--max-budget", type=float, default=2e10)
    m.add_argument("--branch-jobs", type=int, default=6)
    m.add_argument("--out", required=True)
    m.set_defaults(func=cmd_make)
    r = sub.add_parser("report")
    r.add_argument("run_dir")
    r.add_argument("--partial", action="store_true", help="peek at a pass that is not DONE (jobs still missing)")
    r.add_argument("--shuffles", type=int, default=2000, help="re-pairings for the chance level (default 2000)")
    r.add_argument("--seed", type=int, default=1, help="seed of the re-pairings (default 1)")
    r.set_defaults(func=cmd_report)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
