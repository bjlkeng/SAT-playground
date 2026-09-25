#!/usr/bin/env python3
"""rl_round1.py — DAgger round 1: cells, the parent's dry run, the active fork schedule.

Plan: plan/rl-scheduler-solver13-plan.md §5.3 (one parent per cell, the
whole menu of one knob per point), §6.1c (active branching: ensemble
disagreement × timer-due stakes, a quarter of the points random), §6.2
item 4 (each round runs the current policy as the parent, forks at the
states it visits, and aggregates), §11 (2026-09-24: the round-1 design).
Bead SAT-playground-p9m.10.5 (E.5, round 1).

Three steps, each a subcommand:

  cells     The round-1 cell table from round 0's outcome. Only cells that
            yield labels: the training cells stock solves in 60-1800 s
            (class slow, budget B_cell), the band cells stock solves at
            3600 s (band_solved, B_cell_band), the band timeouts some
            round-0 child or wild run solved (rescuable, 1.5 × B_cell_band,
            points late in the run) and the band timeouts the 7200 s stock
            probe solved (probe_solved, 1.5 × the probe's work at the
            solve). Fast cells and the never-solved timeouts are out.

  dryrun    A collector job table that runs the parent policy (the net
            exported by tools/rl/rank.py --export, at the collection
            margin) once per cell with logging and no forks, on the cell's
            budget. The parent is deterministic, so this run IS the prefix
            every fork child of the round will share; its decision rows are
            the states the schedule is drawn from.

  schedule  From the converted dry run: at every decision of the parent,
            per knob, the disagreement of the round-0 bootstrap rankers
            (the spread across members of each alternative's gap over
            stock, the largest alternative counting) times the stakes (1
            when the knob's timer fires within the coming epoch, else
            --idle-stakes). Three quarters of a cell's points are active,
            shared equally among the knobs (their ensembles differ in
            regularization, so raw disagreement is not comparable across
            knobs): each knob takes its highest-scoring free decisions;
            the last quarter is random for coverage. Then the fork job
            table with the same parent policy and budgets.

    ~/.cache/sat13-rl/venv/bin/python tools/rl_round1.py cells --round0 log/rl-round0-<ts> \\
        --probe log/rl-band7200-<ts> --out benchmarks/rl/round1_cells.tsv
    ~/.cache/sat13-rl/venv/bin/python tools/rl_round1.py dryrun --cells benchmarks/rl/round1_cells.tsv \\
        --net /abs/round0_rankers.net.bin --margin 0.5 --out /path/jobs-r1dry.tsv
    python3 tools/rl_collect.py table /path/jobs-r1dry.tsv --suite sat-comp-2025 --name r1dry --jobs 32 ...
    ~/.cache/sat13-rl/venv/bin/python tools/rl_dataset.py convert log/rl-r1dry-<ts>
    ~/.cache/sat13-rl/venv/bin/python tools/rl_round1.py schedule --cells benchmarks/rl/round1_cells.tsv \\
        --dryrun log/rl-r1dry-<ts> --ensemble log/rl-round0-<ts>/dataset/rankers_ensemble.npz \\
        --net /abs/round0_rankers.net.bin --margin 0.5 --out benchmarks/rl/round1_jobs.tsv
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
CHILDREN = ROOT / "benchmarks" / "rl" / "round0_children.tsv"
NORM = ROOT / "benchmarks" / "rl" / "obs_norm_2025.json"
JOB_COLUMNS = ("stem", "tag", "flavour", "seed", "limit_ticks", "wall_s", "procs", "peak_rss_mb", "env")
CELL_COLUMNS = ("stem", "family", "cls", "budget", "work_per_s", "peak_rss_mb", "points", "window_lo", "window_hi",
                "seed", "source")
KNOBS = ("reorder", "reduce", "probe", "rephase", "mode")      # round 1's knobs (plan §11, 2026-09-24)
ALT_ENTRIES = {"probe": 4, "eliminate": 4, "reduce": 4, "rephase": 4, "reorder": 4, "mode": 2, "margin": 2, "sweep": 3}
TIMER_OF = {"probe": "probe", "eliminate": "eliminate", "reduce": "reduce", "rephase": "rephase",
            "reorder": "reorder", "mode": "mode", "sweep": "probe", "margin": None}
TIMERS = ("probe", "eliminate", "reduce", "rephase", "reorder", "mode")
MENU_OF = {"probe": [0.0, 0.5, 1.0, 2.0, 4.0], "eliminate": [0.0, 0.5, 1.0, 2.0, 4.0],
           "reduce": [0.0, 0.5, 1.0, 2.0, 4.0], "rephase": [0.0, 0.5, 1.0, 2.0, 4.0],
           "reorder": [0.0, 0.5, 1.0, 2.0, 4.0], "mode": [0.5, 1.0, 2.0], "margin": [0.5, 1.0, 2.0],
           "sweep": [0.0, 0.5, 1.0, 2.0]}
# per class: the window of the parent's decisions; the point targets are
# the schedule's --points-* options (recorded in the schedule table)
CLASS_WINDOW = {"slow": (0.1, 0.9), "band_solved": (0.2, 0.9), "rescuable": (0.4, 0.9), "probe_solved": (0.2, 0.9)}


def point_rules(args) -> dict:
    """Point target per class from the parent's decision count."""
    return {
        "slow": lambda dec: min(dec, max(8, min(args.points_slow_max, round(dec / args.points_slow_div)))),
        "band_solved": lambda dec: min(dec, args.points_band),
        "rescuable": lambda dec: min(dec, args.points_rescue),
        "probe_solved": lambda dec: min(dec, args.points_probe),
    }
RESCUE_BUDGET_FACTOR = 1.5      # the band budget, extended: round-0 rescues came at 48-100 % of it
PROBE_BUDGET_FACTOR = 1.5       # over the probe's work at the solve, like B_cell's 3 × t clamp but tighter
DEFAULT_RATE = 3e7


def load_tsv(path: Path) -> list[dict]:
    with open(path, newline="") as f:
        return list(csv.DictReader((ln for ln in f if not ln.startswith("#")), dialect="excel-tab"))


def fnum(x):
    try:
        return float(x)
    except (TypeError, ValueError):
        return None


def cell_seed(seed: int, stem: str) -> int:
    return int.from_bytes(hashlib.sha256(f"{seed}:{stem}".encode()).digest()[:8], "little")


# ---------------------------------------------------------------------------
# cells
# ---------------------------------------------------------------------------

def cmd_cells(args) -> int:
    cells = {r["stem"]: r for r in load_tsv(Path(args.cells_table))}
    train = training_cells()
    children = load_tsv(Path(args.children))
    # band timeouts some round-0 run solved: children from the labels table,
    # wild runs from the pass's results
    rescued = {r["stem"] for r in children if r["cls"] == "band_timeout" and r["child_solved"] == "1"}
    # the input passes must be clean: a job the collector flagged (an invalid
    # model, a contradiction, a premature UNKNOWN) is a solver bug and stops
    # the round (CLAUDE.md "Correctness is absolute")
    def clean_results(run_dir: Path) -> list[dict]:
        with open(run_dir / "results.tsv", newline="") as f:
            recs = list(csv.DictReader(f, dialect="excel-tab"))
        bad = [x for x in recs if x.get("failed", "0") == "1"]
        if bad:
            raise SystemExit(f"*** {run_dir}: {len(bad)} job(s) flagged by the collector, e.g. "
                             f"{[x['stem'] + '.' + x['tag'] for x in bad[:3]]}: a correctness failure, no cell table")
        return recs
    # every answer of a cell across the input passes must agree (the
    # collector checks within a pass, not between passes): the round-0
    # parents, children and wild runs (from its converted dataset), the
    # probe, and the stock passes' statuses in the cells table
    answers: dict[str, set] = defaultdict(set)
    for stem, c in cells.items():
        for st in (c.get("status"), c.get("band_status")):
            if st in ("SAT", "UNSAT"):
                answers[stem].add(st)
    r0_runs = Path(args.round0) / "dataset" / "runs.parquet"
    if r0_runs.is_file():
        import pyarrow.parquet as pq
        rr = pq.read_table(r0_runs, columns=["stem", "result", "solved", "anomaly"]).to_pydict()
        for stem, res, sol, an in zip(rr["stem"], rr["result"], rr["solved"], rr["anomaly"]):
            if sol and an != "wall_cap":
                answers[stem].add(str(res).upper()[:3] if str(res).upper()[:3] in ("SAT", "UNS") else str(res))
    for x in clean_results(Path(args.round0)):
        if x["result"] in ("SAT", "UNSAT") and not x.get("anomaly"):
            answers[x["stem"]].add(x["result"])
        if x["flavour"] in ("random", "jitter") and x["result"] in ("SAT", "UNSAT") \
                and cells.get(x["stem"], {}).get("band_status") == "TIMEOUT":
            rescued.add(x["stem"])
    # band timeouts the 7200 s stock probe solved: budget from its work
    probe_work: dict[str, float] = {}
    if args.probe:
        for x in clean_results(Path(args.probe)):
            if x["result"] in ("SAT", "UNSAT") and not x.get("anomaly"):
                answers[x["stem"]].add(x["result"])
            if x["result"] in ("SAT", "UNSAT") and x.get("work", "NA") != "NA":
                probe_work[x["stem"]] = float(x["work"])
    contradictions = {s_: sorted(a) for s_, a in answers.items() if len({v[:3] for v in a}) > 1}
    if contradictions:
        for s_, a in list(contradictions.items())[:10]:
            print(f"*** CORRECTNESS FAILURE: {s_}: answers {a} across the input passes")
        raise SystemExit(f"*** {len(contradictions)} cell(s) with contradicting answers across the passes: a solver "
                         f"bug, no cell table")
    rows = []
    for stem in sorted(cells):
        c = cells[stem]
        if stem not in train or c.get("failed") not in ("0", "", None):
            continue
        rate = fnum(c.get("work_per_s")) or DEFAULT_RATE
        rss = max(fnum(c.get("peak_rss_mb")) or 0.0, fnum(c.get("band_peak_rss_mb")) or 0.0)
        common = {"stem": stem, "family": c.get("family", ""), "work_per_s": int(rate), "peak_rss_mb": f"{rss:.1f}",
                  "seed": int(c.get("seed") or 0)}
        t = fnum(c.get("stock_time_s")) or 0.0
        if c.get("band") == "1":
            if not c.get("B_cell_band"):
                continue
            if c.get("band_status") in ("SAT", "UNSAT"):
                rows.append({**common, "cls": "band_solved", "budget": int(c["B_cell_band"]), "source": "band pass"})
            elif stem in probe_work:
                rows.append({**common, "cls": "probe_solved", "budget": int(PROBE_BUDGET_FACTOR * probe_work[stem]),
                             "source": "7200 s probe"})
            elif stem in rescued:
                rows.append({**common, "cls": "rescuable", "budget": int(RESCUE_BUDGET_FACTOR * int(c["B_cell_band"])),
                             "source": "round-0 child or wild solve"})
        elif c.get("status") in ("SAT", "UNSAT") and t >= 60.0 and c.get("B_cell"):
            rows.append({**common, "cls": "slow", "budget": int(c["B_cell"]), "source": "stock pass"})
    for r in rows:
        lo, hi = CLASS_WINDOW[r["cls"]]
        r["window_lo"], r["window_hi"] = lo, hi
        r["points"] = ""          # set by `schedule` from the parent's decision count
    assert_training_only([r["stem"] for r in rows])
    with open(args.out, "w", newline="") as f:
        f.write(f"# round-1 cells (tools/rl_round1.py cells): classes slow / band_solved / rescuable / probe_solved; "
                f"budgets in work units; points set by the schedule\n")
        w = csv.DictWriter(f, fieldnames=list(CELL_COLUMNS), dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        for r in rows:
            w.writerow(r)
    by = Counter(r["cls"] for r in rows)
    print(f"wrote {args.out}: {len(rows)} cells: " + ", ".join(f"{k} {v}" for k, v in sorted(by.items()))
          + f"; {len(probe_work)} probe solves, {len(rescued)} rescuable band timeouts")
    return 0


# ---------------------------------------------------------------------------
# dryrun
# ---------------------------------------------------------------------------

def parent_env(net: str, margin: float, net_sha: str, epochs: str | None = None) -> str:
    """The parent's settings, with the weights file's checksum as an extra
    variable the solver ignores and the collector records per job, so a
    later step can check it ran the same weights; the decision cadence
    too when the dry run set one."""
    env = f"SAT_POLICY={net} SAT_POLICY_MARGIN={margin} RL_NET_SHA256={net_sha}"
    if epochs:
        env += f" SAT_POLICY_EPOCH_TICKS={epochs}"
    return env


def sha256_of(path: str) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def cmd_dryrun(args) -> int:
    import json

    rows = load_tsv(Path(args.cells))
    net = str(Path(args.net).resolve())
    net_sha = sha256_of(net)
    jobs = []
    for r in rows:
        budget = int(r["budget"])
        rate = float(r["work_per_s"]) or DEFAULT_RATE
        jobs.append({"stem": r["stem"], "tag": "r1dry", "flavour": "net", "seed": int(r["seed"]), "limit_ticks": budget,
                     "wall_s": int(3.0 * budget / rate) + 300, "procs": 1, "peak_rss_mb": r["peak_rss_mb"],
                     "env": parent_env(net, args.margin, net_sha, args.epochs)})
    with open(args.out, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(JOB_COLUMNS), dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        for j in jobs:
            w.writerow(j)
    meta = {"net": net, "net_sha256": net_sha, "margin": args.margin, "cells": str(Path(args.cells).resolve()),
            "cells_sha256": sha256_of(args.cells), "epochs": args.epochs}
    Path(args.out + ".meta.json").write_text(json.dumps(meta, indent=1) + "\n")
    print(f"wrote {args.out}: {len(jobs)} dry-run jobs (parent {net} at margin {args.margin}); settings in {args.out}.meta.json")
    return 0


# ---------------------------------------------------------------------------
# schedule
# ---------------------------------------------------------------------------

def cmd_schedule(args) -> int:
    import json

    import numpy as np
    import pyarrow.parquet as pq

    rows = load_tsv(Path(args.cells))
    dry = Path(args.dryrun).resolve()
    ens = np.load(args.ensemble)
    norm = json.loads(Path(NORM).read_text())
    names = norm["names"]
    if list(ens["names"]) != names:
        raise SystemExit(f"{args.ensemble}: entry names differ from {NORM}")
    mean = np.asarray(norm["mean"], dtype=np.float64)
    std = np.asarray(norm["std"], dtype=np.float64)
    cols = [f"obs_{n}" for n in names]
    knobs = [k for k in KNOBS if f"{k}_W" in ens]
    print(f"knobs with a round-0 ensemble: {knobs}")
    # The dry run's records. A cell the collector flagged (an invalid model, a
    # contradiction, a premature UNKNOWN) is a solver bug and stops the round
    # (CLAUDE.md "Correctness is absolute"); a capped or missing cell is only
    # left out. The fork jobs must run exactly the parent the dry run ran
    # (same weights file, same margin, same seed and budget per cell), or
    # the decision numbers would name states the parent never visits.
    with open(dry / "results.tsv", newline="") as f:
        dres = {x["stem"]: x for x in csv.DictReader(f, dialect="excel-tab") if x["tag"] == "r1dry"}
    failed = sorted(s_ for s_, x in dres.items() if x.get("failed", "0") == "1")
    if failed:
        print(f"*** {len(failed)} dry-run cell(s) flagged by the collector, e.g. {failed[:3]}: a correctness failure, "
              f"no schedule from this run")
        return 1
    net = str(Path(args.net).resolve())
    net_sha = sha256_of(net)
    manifest = json.loads((dry / "manifest.json").read_text())
    with open(dry / "jobs.tsv", newline="") as f:
        djobs = {j["stem"]: j for j in csv.DictReader(f, dialect="excel-tab") if j["tag"] == "r1dry"}
    # the decision cadence the dry run ran: the job's own env first (a table
    # made with `dryrun --epochs`), else the collector's --epochs from the
    # manifest, else the solver's default; the fork jobs carry the same
    cadences = {(dict(kv.split("=", 1) for kv in j["env"].split()).get("SAT_POLICY_EPOCH_TICKS")
                 or manifest.get("epochs") or None) for j in djobs.values()}
    if len(cadences) > 1:
        print(f"*** the dry run's jobs ran at different decision cadences {sorted(map(str, cadences))}: refusing")
        return 1
    epochs = next(iter(cadences)) if cadences else (manifest.get("epochs") or None)
    # the weights the dry run ran: the checksum its jobs carried, else the
    # one recorded beside the run (a dry run launched before the checksum
    # travelled in the env, written by hand after checking the file
    # predates the run); a weights file with another checksum is refused
    meta_path = dry / "jobs.tsv.meta.json"
    meta_sha = json.loads(meta_path.read_text()).get("net_sha256") if meta_path.is_file() else None
    mismatches = []
    for r in rows:
        j = djobs.get(r["stem"])
        if j is None:
            continue
        env = dict(kv.split("=", 1) for kv in j["env"].split())
        ran_sha = env.get("RL_NET_SHA256") or meta_sha
        if env.get("SAT_POLICY") != net or float(env.get("SAT_POLICY_MARGIN", "nan")) != args.margin:
            mismatches.append(f"{r['stem']}: parent {env.get('SAT_POLICY')} at {env.get('SAT_POLICY_MARGIN')}")
        elif ran_sha != net_sha:
            mismatches.append(f"{r['stem']}: weights {str(ran_sha)[:16]} v {net_sha[:16]}")
        elif int(j["seed"]) != int(r["seed"]) or int(j["limit_ticks"]) != int(r["budget"]):
            mismatches.append(f"{r['stem']}: seed/budget {j['seed']}/{j['limit_ticks']} v {r['seed']}/{r['budget']}")
    if mismatches:
        print(f"*** {len(mismatches)} cell(s) whose dry run used other settings, e.g. {mismatches[:3]}: refusing")
        return 1
    rng = random.Random(args.seed)
    rules = point_rules(args)
    jobs, sched = [], []
    skipped: Counter = Counter()
    for r in rows:
        stem = r["stem"]
        x = dres.get(stem)
        if not x:
            skipped["no dry-run record"] += 1
            continue
        if x.get("anomaly"):
            skipped["dry run capped"] += 1
            continue
        f = dry / "dataset" / "rows" / f"{stem}.r1dry.parquet"
        if not f.is_file():
            skipped["dry run not converted"] += 1
            continue
        t = pq.read_table(f, columns=cols + ["is_decision", "stable", "work"] + [f"tm_{tm}_fires" for tm in TIMERS]).to_pydict()
        n = len(t["is_decision"])
        idx = np.nonzero(np.array(t["is_decision"], dtype=bool))[0]
        n_dec = len(idx)
        if n_dec == 0:
            skipped["no decision"] += 1
            continue
        Z = (np.stack([np.array(t[c], dtype=np.float64) for c in cols], axis=1)[idx] - mean) / std
        stable = np.array(t["stable"], dtype=np.float64)[idx] > 0.5
        nxt = np.append(idx[1:], n - 1)
        due = {tm: (np.array(t[f"tm_{tm}_fires"])[nxt] - np.array(t[f"tm_{tm}_fires"])[idx]) > 0 for tm in TIMERS}
        # per knob: the members' disagreement on each alternative's gap over
        # stock (the spread across members per fixed entry, then the largest
        # over entries, so members that back different alternatives count
        # as disagreeing), times the stakes
        score = np.zeros((n_dec, len(knobs)))
        for kk, knob in enumerate(knobs):
            W, b = ens[f"{knob}_W"], ens[f"{knob}_b"]           # (B, E, n_in), (B, E)
            stock_i = MENU_OF[knob].index(1.0)
            S = np.einsum("dn,ben->bde", Z, W) + b[:, None, :]  # (B, dec, E)
            gaps = np.delete(S, stock_i, axis=2) - S[:, :, stock_i][:, :, None]   # (B, dec, E-1)
            dis = gaps.std(axis=0).max(axis=1)
            tm = TIMER_OF[knob]
            stakes = np.where(due[tm], 1.0, args.idle_stakes) if tm else np.where(stable, args.idle_stakes, 1.0)
            if knob == "rephase":
                stakes = np.where(stable, stakes, 0.0)          # masked to stock in focused mode
            score[:, kk] = dis * stakes
        lo, hi = float(r["window_lo"]), float(r["window_hi"])
        window = list(range(math.ceil(lo * (n_dec - 1)), math.floor(hi * (n_dec - 1)) + 1)) or list(range(n_dec))
        # the target never exceeds the window, so the random quarter survives on short runs
        target = min(rules[r["cls"]](n_dec), len(window))
        n_active = round(0.75 * target)
        # active points: an equal share per knob, each knob taking its
        # highest-scoring free decisions (one point per decision); a knob
        # short of scored decisions leaves its share to the others
        chosen: dict[int, str] = {}
        ranked = {k: sorted((d for d in window if score[d, kk] > 0), key=lambda d: -score[d, kk])
                  for kk, k in enumerate(knobs)}
        order = list(knobs)
        rng.shuffle(order)
        while len(chosen) < n_active and any(ranked[k] for k in order):
            for k in order:
                while ranked[k] and ranked[k][0] in chosen:
                    ranked[k].pop(0)
                if ranked[k] and len(chosen) < n_active:
                    chosen[ranked[k].pop(0)] = k
        active_set = set(chosen)
        # random points: only decisions where some fitted knob may act
        # (rephase is masked in focused mode; an ensemble fitted for rephase
        # alone leaves focused-mode decisions with nothing to fork)
        def allowed_at(d):
            return [k for k in knobs if not (k == "rephase" and not stable[d])]
        rest = [d for d in window if d not in chosen and allowed_at(d)]
        rng.shuffle(rest)
        for d in rest[:max(target - len(chosen), 0)]:
            chosen[d] = rng.choice(allowed_at(d))
        if not chosen:
            skipped["no point"] += 1
            continue
        points = sorted(chosen.items())
        n_children = sum(ALT_ENTRIES[k] for _, k in points)
        budget = int(r["budget"])
        rate = float(r["work_per_s"]) or DEFAULT_RATE
        waves = math.ceil(n_children / max(args.branch_jobs, 1))
        cap = int(2.0 * (1 + waves) * budget / rate) + 600
        branch = ",".join(f"{d}:{k}" for d, k in points)
        jobs.append({"stem": stem, "tag": "fork", "flavour": "fork", "seed": int(r["seed"]), "limit_ticks": budget,
                     "wall_s": cap, "procs": 1 + args.branch_jobs, "peak_rss_mb": r["peak_rss_mb"],
                     "env": f"{parent_env(net, args.margin, net_sha, epochs)} SAT_POLICY_BRANCH={branch} "
                            f"SAT_POLICY_BRANCH_JOBS={args.branch_jobs}"})
        work_at = np.array(t["work"], dtype=np.float64)[idx]
        sched.append({"stem": stem, "class": r["cls"], "decisions": n_dec, "points": len(points),
                      "active": len(active_set), "children": n_children, "budget": budget,
                      "core_s": int((min(float(x.get("work") or budget), budget)
                                     + sum(ALT_ENTRIES[k] * max(budget - work_at[d], 0.0) for d, k in points)) / rate),
                      "knobs": " ".join(f"{d}:{k}{'*' if d in active_set else ''}" for d, k in points)})
    out = Path(args.out)
    if not sched:
        print("no cell got a branch point (" + ", ".join(f"{k} {v}" for k, v in skipped.items()) + "): nothing written")
        return 2
    with open(out, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(JOB_COLUMNS), dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        for j in jobs:
            w.writerow(j)
    with open(out.with_name(out.name + ".schedule.tsv"), "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(sched[0].keys()), dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        for s_ in sched:
            w.writerow(s_)
    by_cls: dict[str, Counter] = defaultdict(Counter)
    for s_ in sched:
        by_cls[s_["class"]].update({"cells": 1, "points": s_["points"], "children": s_["children"], "core_s": s_["core_s"]})
    print(f"{'class':13s} {'cells':>5s} {'points':>6s} {'children':>8s} {'core-h':>7s}")
    tot = Counter()
    for cls, c in by_cls.items():
        print(f"{cls:13s} {c['cells']:5d} {c['points']:6d} {c['children']:8d} {c['core_s'] / 3600:7.0f}")
        tot.update(c)
    print(f"{'total':13s} {tot['cells']:5d} {tot['points']:6d} {tot['children']:8d} {tot['core_s'] / 3600:7.0f}"
          f"   (about {tot['core_s'] / 3600 / (args.slots * args.branch_jobs / (1 + args.branch_jobs)) / 24:.1f} days at {args.slots} slots)")
    kc = Counter(k for s_ in sched for item in s_["knobs"].split() for k in [item.split(':')[1].rstrip('*')])
    print("points per knob: " + ", ".join(f"{k} {kc[k]}" for k in knobs))
    if skipped:
        print("skipped: " + ", ".join(f"{k} {v}" for k, v in skipped.items()))
    print(f"wrote {out} ({len(jobs)} fork jobs) and {out.with_name(out.name + '.schedule.tsv')}")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("cells")
    c.add_argument("--cells-table", default=str(CELLS))
    c.add_argument("--children", default=str(CHILDREN))
    c.add_argument("--round0", required=True, help="the round-0 pass (wild-run solves on band timeouts)")
    c.add_argument("--probe", help="the 7200 s stock probe pass on the band timeouts")
    c.add_argument("--out", required=True)
    c.set_defaults(func=cmd_cells)
    d = sub.add_parser("dryrun")
    d.add_argument("--cells", required=True)
    d.add_argument("--net", required=True)
    d.add_argument("--margin", type=float, default=0.5)
    d.add_argument("--epochs", help="SAT_POLICY_EPOCH_TICKS for the parent (default: the solver's)")
    d.add_argument("--out", required=True)
    d.set_defaults(func=cmd_dryrun)
    s = sub.add_parser("schedule")
    s.add_argument("--cells", required=True)
    s.add_argument("--dryrun", required=True, help="the converted dry-run pass")
    s.add_argument("--ensemble", required=True, help="rankers_ensemble.npz of the previous round")
    s.add_argument("--net", required=True)
    s.add_argument("--margin", type=float, default=0.5)
    s.add_argument("--idle-stakes", type=float, default=0.2)
    s.add_argument("--branch-jobs", type=int, default=6)
    s.add_argument("--slots", type=int, default=28)
    s.add_argument("--seed", type=int, default=20260924)
    s.add_argument("--points-slow-div", type=float, default=4.0, help="slow cells: decisions / this, clamped to [8, max]")
    s.add_argument("--points-slow-max", type=int, default=48)
    s.add_argument("--points-band", type=int, default=32, help="points per band solver")
    s.add_argument("--points-rescue", type=int, default=16, help="points per rescuable band timeout")
    s.add_argument("--points-probe", type=int, default=16, help="points per probe-solved band timeout")
    s.add_argument("--out", required=True)
    s.set_defaults(func=cmd_schedule)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
