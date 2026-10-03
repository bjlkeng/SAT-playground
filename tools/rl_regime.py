#!/usr/bin/env python3
"""rl_regime.py — the regime experiment: does choosing the search regime have any alpha?

Plan: plan/rl-scheduler-solver13-plan.md §11 (2026-10-03). Bead
SAT-playground-p9m.10.17.

The epoch-policy line (rounds 0-1, the two hold pilots, check 1) found no
learnable preference among timing entries. This asks the question one
level up: kissat alternates two search regimes blindly (focused and
stable mode, about half the ticks each); would another regime, chosen per
instance, have paid? It is an oracle with a luck control, so it measures
headroom before any selector is built.

A regime child (solver 13 fork mode, `SAT_POLICY_BRANCH=<D>:regime`,
`SAT_POLICY_BRANCH_REGIMES`) runs one regime in place of kissat's own
choice from the branch decision for its hold, then kissat's alternation
resumes with normal-size stints:

  focused, stable   that mode only
  stay              the mode of the moment, no switch (the swap of the
                    coming stint)
  sat               kissat's alternation with its `--sat` dials
  eager, lazy       kissat's alternation with eager / lazy restarts
  reroll            kissat unchanged on another random stream: what a
                    change of trajectory alone does (the luck control)

`make`: one fork job per training cell, one regime point per job at the
last decision before the first mode switch at or after --point of the
parent's decisions, with three groups of children:

  stint   stay, sat, eager, lazy for K1 decision epochs, K1 the length of
          the stock stint that follows the point (the owner's design: one
          kissat stint, then back to kissat; a stint is 1-4 epochs)
  block   focused, stable, sat, eager, lazy for K2 = --block of the
          parent's decisions (a quarter of the run)
  reroll  --rerolls children, kissat unchanged

Cells: the training cells stock solves in --min-time seconds or more with
at least --min-decisions decisions (at most --per-family per family, the
longest first), every band cell that solves at the band budget, and band
cells that do not (the ones --rescue lists as solved by a longer stock run
first, then one per remaining family, --timeout-n in all).

`report`: from the converted pass, per cell class and overall: solved and
tick PAR-2 (an unsolved run costs twice its budget) for the parent, for
each regime as a constant, for the best of a group's regimes or the
parent per cell (the regime oracle), and for the best of as many rerolls
or the parent (the luck oracle, averaged over the ways to pick them).
Regimes have alpha only where the regime oracle clears the luck oracle;
the comparison (tick PAR-2 ratio, the per-cell cost ratio, the solved
difference) comes with a relabeling p value: within every cell, which
children count as regimes and which as rerolls is shuffled, which is what
the three would read if a regime were only another reroll. A per-family
table follows (solved and tick PAR-2 of the two oracles), and every
cell's arms go to <run>/report/regime_per_cell.tsv.

    ~/.cache/sat13-rl/venv/bin/python tools/rl_regime.py make --stock log/rl-stock2025-<ts> \\
        --band log/rl-band2025-<ts> [--rescue log/rl-band7200-<ts>/results.tsv] --out benchmarks/rl/regime_jobs.tsv
    python3 tools/rl_collect.py table benchmarks/rl/regime_jobs.tsv --suite sat-comp-2025 --name regime --jobs 32 ...
    ~/.cache/sat13-rl/venv/bin/python tools/rl_dataset.py convert log/rl-regime-<ts>
    ~/.cache/sat13-rl/venv/bin/python tools/rl_regime.py report log/rl-regime-<ts>
"""
from __future__ import annotations

import argparse
import csv
import itertools
import math
import random
import statistics
import sys
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from rl_round0 import CELLS, CLASSES, DEFAULT_RATE, cell_class, fnum, load_cells  # noqa: E402
from rl_split import assert_training_only, training_cells  # noqa: E402

JOB_COLUMNS = ("stem", "tag", "flavour", "seed", "limit_ticks", "wall_s", "procs", "peak_rss_mb", "env")
STINT_REGIMES = ("stay", "sat", "eager", "lazy")
BLOCK_REGIMES = ("focused", "stable", "sat", "eager", "lazy")
MAX_HOLD = 1024                      # policy_fork.rs MAX_HOLD
SOLVED = ("SAT", "UNSAT")
CLASS_ORDER = ("slow", "band_solved", "band_timeout")


# ---------------------------------------------------------------------------
# make
# ---------------------------------------------------------------------------

def read_mode_trace(run_dir: Path, stem: str, tag: str = "stock"):
    """The parent trace a regime point is placed on: the row of each
    decision, the work there, and per row the cumulative mode switches."""
    import numpy as np
    import pyarrow.parquet as pq

    f = run_dir / "dataset" / "rows" / f"{stem}.{tag}.parquet"
    if not f.is_file():
        return None
    t = pq.read_table(f, columns=["is_decision", "work", "stable", "tm_mode_fires"]).to_pydict()
    if not t["work"]:
        return None
    dec = np.nonzero(np.asarray(t["is_decision"], dtype=bool))[0]
    return {"n_rows": len(t["work"]), "dec_rows": dec,
            "work": np.asarray(t["work"], dtype=np.float64),
            "stable": np.asarray(t["stable"], dtype=np.float64) > 0.5,
            "fires": np.asarray(t["tm_mode_fires"], dtype=np.int64)}


def place_point(trace, point: float, block: float) -> dict | None:
    """The regime point of one parent: the decision, the stint hold K1 and
    the block hold K2 (decision epochs)."""
    import numpy as np

    dec, fires, n_rows = trace["dec_rows"], trace["fires"], trace["n_rows"]
    n_dec = len(dec)
    if n_dec < 2:
        return None
    per_epoch = max(1.0, float(np.median(np.diff(dec))))       # rows per decision epoch
    d0 = max(1, min(n_dec - 2, round(point * n_dec)))
    # the last decision before a stock mode switch: the first decision at or
    # after d0 whose epoch holds one (within a tenth of the run, else d0)
    d = d0
    for cand in range(d0, min(n_dec - 1, d0 + max(8, n_dec // 10))):
        if fires[dec[cand + 1]] > fires[dec[cand]]:
            d = cand
            break
    # the stint that follows: from the first switch after the point to the next
    after = np.nonzero(fires[dec[d]:] > fires[dec[d]])[0]
    if len(after):
        r1 = dec[d] + int(after[0])
        later = np.nonzero(fires[r1:] > fires[r1])[0]
        r2 = r1 + int(later[0]) if len(later) else n_rows
        k1 = max(1, round((r2 - r1) / per_epoch))
    else:
        k1 = 1                                               # no switch left in the trace
    k1 = min(k1, MAX_HOLD - 1)
    k2 = min(MAX_HOLD, max(k1 + 1, round(block * n_dec)))
    return {"decision": int(d), "k1": int(k1), "k2": int(k2), "work_at": float(trace["work"][dec[d]]),
            "stable_at": bool(trace["stable"][dec[d]]), "n_dec": n_dec}


def regime_list(k1: int, k2: int, rerolls: int) -> str:
    items = [f"{r}@{k1}" for r in STINT_REGIMES] + [f"{r}@{k2}" for r in BLOCK_REGIMES] + ["reroll@1"] * rerolls
    return ",".join(items)


def choose_cells(cells: dict, args) -> list[tuple[str, dict]]:
    train = training_cells()
    by_class: dict[str, list[dict]] = defaultdict(list)
    for stem, c in cells.items():
        if stem in train:
            cls = cell_class(c)
            if cls in CLASS_ORDER:
                by_class[cls].append(c)
    chosen: list[tuple[str, dict]] = []
    # slow: long enough to hold a block, the longest of each family first
    slow = [c for c in by_class["slow"] if (fnum(c.get("stock_time_s")) or 0) >= args.min_time
            and int(c.get("decisions") or 0) >= args.min_decisions]
    fam: dict[str, list[dict]] = defaultdict(list)
    for c in slow:
        fam[c["family"]].append(c)
    for f in sorted(fam):
        best = sorted(fam[f], key=lambda c: (-int(c["decisions"]), c["stem"]))[:args.per_family]
        chosen += [("slow", c) for c in best]
    chosen += [("band_solved", c) for c in sorted(by_class["band_solved"], key=lambda c: c["stem"])]
    # band timeouts: the ones a longer stock run solves, then one per other family
    rescue: set[str] = set()
    if args.rescue:
        with open(args.rescue, newline="") as f:
            rescue = {r["stem"] for r in csv.DictReader(f, dialect="excel-tab")
                      if r.get("result") in SOLVED and r.get("failed", "0") != "1"}
    bt = sorted(by_class["band_timeout"], key=lambda c: c["stem"])
    first = [c for c in bt if c["stem"] in rescue]
    seen = {c["family"] for c in first}
    rest = []
    for c in bt:
        if c["stem"] not in rescue and c["family"] not in seen:
            seen.add(c["family"])
            rest.append(c)
    chosen += [("band_timeout", c) for c in (first + rest)[:args.timeout_n]]
    return chosen


def cmd_make(args) -> int:
    cells = load_cells(Path(args.cells))
    chosen = choose_cells(cells, args)
    assert_training_only([c["stem"] for _, c in chosen])
    traces = {"stock": Path(args.stock).resolve(), "band": Path(args.band).resolve()}
    n_children = len(STINT_REGIMES) + len(BLOCK_REGIMES) + args.rerolls
    jobs, sched, skipped = [], [], Counter()
    for cls, c in chosen:
        spec = CLASSES[cls]
        trace = read_mode_trace(traces[spec["trace"]], c["stem"])
        if trace is None:
            skipped["no converted trace"] += 1
            continue
        pt = place_point(trace, args.point, args.block)
        if pt is None or pt["n_dec"] < args.min_decisions:
            skipped["too few decisions"] += 1
            continue
        budget = int(c[spec["budget"]])
        rate = fnum(c.get("work_per_s")) or DEFAULT_RATE
        rss = max(fnum(c.get("peak_rss_mb")) or 0.0, fnum(c.get("band_peak_rss_mb")) or 0.0)
        waves = math.ceil(n_children / max(args.branch_jobs, 1))
        cap = int(2.0 * (1 + waves) * budget / rate) + 600
        env = (f"SAT_POLICY_BRANCH={pt['decision']}:regime SAT_POLICY_BRANCH_JOBS={args.branch_jobs} "
               f"SAT_POLICY_BRANCH_REGIMES={regime_list(pt['k1'], pt['k2'], args.rerolls)}")
        jobs.append({"stem": c["stem"], "tag": "regime", "flavour": "fork", "seed": int(c.get("seed") or 0),
                     "limit_ticks": budget, "wall_s": cap, "procs": 1 + args.branch_jobs,
                     "peak_rss_mb": f"{rss:.1f}" if rss else "", "env": env})
        solved = cls != "band_timeout"
        parent_work = min(float(c.get("band_work" if cls.startswith("band") else "stock_work") or budget), float(budget))
        to_budget = n_children * max(budget - pt["work_at"], 0.0)
        # a child that keeps the parent's pace ends where the parent does
        expected = n_children * max(parent_work - pt["work_at"], 0.0) * (1.3 if solved else 1.0)
        sched.append({"stem": c["stem"], "class": cls, "family": c.get("family", ""), "decisions": pt["n_dec"],
                      "point": pt["decision"], "stable_at_point": int(pt["stable_at"]), "stint_hold": pt["k1"],
                      "block_hold": pt["k2"], "children": n_children, "budget": budget, "work_per_s": int(rate),
                      "core_s_expected": int((parent_work + expected) / rate),
                      "core_s_to_budget": int((parent_work + to_budget) / rate)})
    out = Path(args.out)
    with open(out, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(JOB_COLUMNS), dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        w.writerows(jobs)
    side = out.with_name(out.name + ".schedule.tsv")
    with open(side, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(sched[0]) if sched else ["stem"], dialect="excel-tab", lineterminator="\n")
        w.writeheader()
        w.writerows(sched)
    print(f"wrote {out} and {side.name}: {len(jobs)} fork jobs, {n_children} children each "
          f"({len(STINT_REGIMES)} stint, {len(BLOCK_REGIMES)} block, {args.rerolls} reroll), "
          f"{args.branch_jobs} live children per parent; skipped {dict(skipped) or 'none'}")
    print(f"{'class':13s} {'cells':>5s} {'families':>8s} {'decisions':>12s} {'stint hold':>11s} {'block hold':>11s} "
          f"{'core-h expected':>15s} {'to budget':>10s}")
    tot_e = tot_b = 0.0
    for cls in CLASS_ORDER:
        rows = [s for s in sched if s["class"] == cls]
        if not rows:
            continue
        e, b = sum(s["core_s_expected"] for s in rows) / 3600, sum(s["core_s_to_budget"] for s in rows) / 3600
        tot_e, tot_b = tot_e + e, tot_b + b
        dec = sorted(s["decisions"] for s in rows)
        print(f"{cls:13s} {len(rows):5d} {len({s['family'] for s in rows}):8d} {dec[0]:5d}-{dec[-1]:<6d} "
              f"{statistics.median(s['stint_hold'] for s in rows):11.0f} {statistics.median(s['block_hold'] for s in rows):11.0f} "
              f"{e:15.0f} {b:10.0f}")
    print(f"{'all':13s} {len(sched):5d} {len({s['family'] for s in sched}):8d} {'':12s} {'':11s} {'':11s} {tot_e:15.0f} {tot_b:10.0f}")
    print(f"at {args.slots} slots: about {tot_e / args.slots:.0f} h expected, {tot_b / args.slots:.0f} h if every child "
          f"ran to its budget")
    return 0


# ---------------------------------------------------------------------------
# report
# ---------------------------------------------------------------------------

def parse_regimes(env: str) -> list[tuple[str, int]]:
    """[(regime, hold)] in child-index order from a job's env column; an
    item without a hold takes SAT_POLICY_BRANCH_HOLD (default 1), as in the
    solver."""
    items = dict(x.split("=", 1) for x in env.split() if "=" in x)
    spec = items.get("SAT_POLICY_BRANCH_REGIMES")
    if not spec:
        return []
    default = int(items.get("SAT_POLICY_BRANCH_HOLD") or 1)
    out = []
    for x in spec.split(","):
        x = x.strip()
        if x:
            name, _, hold = x.partition("@")
            out.append((name.strip(), int(hold) if hold.strip() else default))
    return out


def group_of(regimes: list[tuple[str, int]], index: int) -> str:
    """stint, block or reroll: a job's stint children share the hold of `stay`."""
    name, hold = regimes[index]
    if name == "reroll":
        return "reroll"
    stint_hold = next((h for n, h in regimes if n == "stay"), None)
    return "stint" if hold == stint_hold else "block"


def load_pass(run: Path, cells: dict, partial: bool):
    """Per cell: the parent and its children by (group, regime), behind the
    gates of the fork reports. Returns (cells_out, notes) or exits."""
    import pyarrow.parquet as pq

    with open(run / "jobs.tsv", newline="") as f:
        jobs = {f"{j['stem']}.{j['tag']}": j for j in csv.DictReader(f, dialect="excel-tab")}
    with open(run / "results.tsv", newline="") as f:
        results = {f"{x['stem']}.{x['tag']}": x for x in csv.DictReader(f, dialect="excel-tab")}
    flagged = sorted(k for k, x in results.items() if x.get("failed", "0") == "1")
    if flagged:
        sys.exit(f"*** {len(flagged)} job(s) flagged by the collector (verify, oracle, sibling agreement, premature "
                 f"UNKNOWN or crash), e.g. {flagged[:3]}: a correctness failure, no report")
    missing = sorted(set(jobs) - set(results))
    if missing or not (run / "DONE").is_file():
        print(f"*** pass not complete: {len(missing)} job(s) without a record, "
              f"DONE {'present' if (run / 'DONE').is_file() else 'absent'}")
        if not partial:
            sys.exit(1)
        print("*** --partial: a peek at an unfinished pass, not a readout")
    r = pq.read_table(run / "dataset" / "runs.parquet").to_pydict()
    n = len(r["run_id"])
    bad = sum(1 for x in r["failed"] if x)
    if bad:
        sys.exit(f"*** {bad} run(s) flagged in the dataset: a correctness failure, no report")
    assert_training_only(sorted(set(r["stem"])))
    ds_keys = {r["run_id"][i].split("/")[-1] for i in range(n) if not r["is_child"][i]}
    n_children_ds = Counter(r["parent_run_id"][i].split("/")[-1] for i in range(n) if r["is_child"][i])
    unconverted = sorted(k for k in results if k not in ds_keys)
    unreadable = sum(max(int(x.get("children") or 0) - n_children_ds.get(k, 0), 0)
                     for k, x in results.items() if x.get("flavour") == "fork")
    if unconverted or unreadable:
        sys.exit(f"*** conversion incomplete: {len(unconverted)} collected job(s) without a run in the dataset, "
                 f"{unreadable} forked child(ren) without a row (e.g. {unconverted[:3]}); convert the whole pass first")
    answers: dict[str, set] = defaultdict(set)
    for i in range(n):
        if r["solved"][i] and r["anomaly"][i] != "wall_cap":
            answers[r["stem"][i]].add(str(r["result"][i]).upper()[:3])
    contradictions = sorted(s for s, a in answers.items() if len(a) > 1)
    if contradictions:
        sys.exit(f"*** CORRECTNESS FAILURE: contradicting answers on {contradictions[:5]}")

    def outcome(i):
        """(solved, work, budget) of run i, the last two on the conversion's
        work clock (`work_end` and `limit_ticks_k`, never the job table's
        budget, which is on the solver's own clock): a memory stop is an
        unsolved run, a wall cap no outcome at all"""
        if r["anomaly"][i] == "wall_cap":
            return None
        solved = bool(r["solved"][i]) and r["anomaly"][i] != "cut"
        w, b = r["work_end"][i], r["limit_ticks_k"][i]
        if b is None or b != b or b <= 0:
            sys.exit(f"*** {r['run_id'][i]}: no tick budget on the conversion's work clock (limit_ticks_k)")
        return solved, (float(w) if w is not None and w == w else 0.0), float(b)

    parent_of = {r["run_id"][i]: i for i in range(n) if not r["is_child"][i]}
    out: dict[str, dict] = {}
    notes = Counter()
    for pid, pi in parent_of.items():
        key = pid.split("/")[-1]
        job = jobs.get(key)
        if job is None:
            continue
        regimes = parse_regimes(job.get("env", ""))
        if not regimes:
            continue                                   # not a regime job
        po = outcome(pi)
        if po is None:
            notes["cells dropped: the parent hit the wall cap"] += 1
            continue
        stem = r["stem"][pi]
        c = cells.get(stem) or {}
        out[stem] = {"class": cell_class(c) or "?", "family": c.get("family", ""),
                     "parent": po, "kids": {}, "expected": len(regimes), "regimes": regimes}
    for i in range(n):
        if not r["is_child"][i] or r["branch_knob"][i] != "regime":
            continue
        pi = parent_of.get(r["parent_run_id"][i])
        stem = r["stem"][i]
        if pi is None or stem not in out:
            continue
        idx = int(r["branch_entry"][i])
        regimes = out[stem]["regimes"]
        if not 0 <= idx < len(regimes):
            sys.exit(f"*** {stem}: child entry {idx} is outside the job's regime list")
        o = outcome(i)
        if o is None:
            notes["children dropped: wall cap"] += 1
            continue
        out[stem]["kids"][(group_of(regimes, idx), regimes[idx][0], idx)] = o
    # a cell is compared only when every one of its children has an outcome
    complete = {}
    for stem, c in out.items():
        if len(c["kids"]) == c["expected"]:
            complete[stem] = c
        else:
            notes[f"cells dropped: {'no' if not c['kids'] else 'not every'} child with an outcome"] += 1
    return complete, notes


def cost(o) -> float:
    """tick PAR-2 of one run: its work when solved, else twice its own budget"""
    solved, work, budget = o
    return work if solved else 2.0 * budget


def best_of(options: list) -> tuple[bool, float]:
    """(solved, cost-ordered best): a solve beats a timeout, then the lower work"""
    return min(options, key=lambda sc: (not sc[0], sc[1]))


def cmd_report(args) -> int:
    run = Path(args.run_dir).resolve()
    cells_tab = load_cells(Path(args.cells))
    cells, notes = load_pass(run, cells_tab, args.partial)
    if not cells:
        print("no complete regime cell in this pass")
        return 1
    rng = random.Random(args.seed)
    classes = [c for c in CLASS_ORDER if any(v["class"] == c for v in cells.values())] + ["all"]
    print(f"{run.name}: {len(cells)} cells with every child's outcome"
          + "".join(f"; {k}: {v}" for k, v in sorted(notes.items())))

    def luck(parent, rerolls, k):
        """the best of k rerolls or the parent, averaged over the ways to pick the k"""
        k = min(k, len(rerolls))
        picks = [best_of([parent] + list(sub)) for sub in itertools.combinations(rerolls, k)] if k else [parent]
        return (sum(x[0] for x in picks) / len(picks), sum(x[1] for x in picks) / len(picks))

    # per cell: the parent, the rerolls and each group's regimes as (solved, cost)
    cell_arms = {}
    for stem, c in cells.items():
        sc = lambda o: (o[0], cost(o))
        kids = sorted(c["kids"].items(), key=lambda kv: kv[0][2])
        cell_arms[stem] = {"parent": sc(c["parent"]),
                           "reroll": [sc(o) for (g, name, idx), o in kids if g == "reroll"],
                           "stint": {name: sc(o) for (g, name, idx), o in kids if g == "stint"},
                           "block": {name: sc(o) for (g, name, idx), o in kids if g == "block"}}

    def arms_of(a) -> dict[str, tuple[float, float]]:
        """every arm of one cell as (solved, cost)"""
        arms = {"stock (the parent)": a["parent"]}
        for group in ("stint", "block"):
            for name, v in a[group].items():
                arms[f"{group}: {name}"] = v
            if a[group]:
                arms[f"{group}: regime oracle"] = best_of([a["parent"]] + list(a[group].values()))
                arms[f"{group}: luck oracle"] = luck(a["parent"], a["reroll"], len(a[group]))
        if a["reroll"]:
            n = len(a["reroll"])
            arms["one reroll (mean)"] = (sum(x[0] for x in a["reroll"]) / n, sum(x[1] for x in a["reroll"]) / n)
        return arms

    def versus(stems, group, relabel=False):
        """the regime oracle against the luck oracle over `stems`: (tick PAR-2
        ratio, mean log cost ratio per cell, solved difference). With
        `relabel` the group's regimes and the rerolls of each cell are
        shuffled first: what the three read when a regime is only a reroll."""
        ra = rb = logs = ds = 0.0
        for s in stems:
            a = cell_arms[s]
            regs, rer = list(a[group].values()), list(a["reroll"])
            if relabel:
                pool = regs + rer
                rng.shuffle(pool)
                regs, rer = pool[:len(regs)], pool[len(regs):]
            x = best_of([a["parent"]] + regs)
            y = luck(a["parent"], rer, len(regs))
            ra, rb = ra + x[1], rb + y[1]
            logs += math.log(x[1] / y[1]) if x[1] > 0 and y[1] > 0 else 0.0
            ds += x[0] - y[0]
        return (ra / rb if rb else float("nan")), logs / max(len(stems), 1), ds

    per_cell = {stem: arms_of(a) for stem, a in cell_arms.items()}
    order = ["stock (the parent)", "one reroll (mean)"]
    for group, names in (("stint", STINT_REGIMES), ("block", BLOCK_REGIMES)):
        order += [f"{group}: {n}" for n in names] + [f"{group}: regime oracle", f"{group}: luck oracle"]
    for cls in classes:
        stems = sorted(s for s, c in cells.items() if cls == "all" or c["class"] == cls)
        if not stems:
            continue
        base = sum(per_cell[s]["stock (the parent)"][1] for s in stems)
        print(f"\n{cls}: {len(stems)} cells")
        print(f"  {'arm':<24s} {'solved':>8s} {'tick PAR-2 v stock':>19s}")
        for arm in order:
            if not all(arm in per_cell[s] for s in stems):
                continue
            solved = sum(per_cell[s][arm][0] for s in stems)
            total = sum(per_cell[s][arm][1] for s in stems)
            print(f"  {arm:<24s} {solved:8.1f} {total / base if base else float('nan'):19.3f}")
        for group in ("stint", "block"):
            if not all(cell_arms[s][group] and cell_arms[s]["reroll"] for s in stems):
                continue
            ratio, mlog, ds = versus(stems, group)
            perms = [versus(stems, group, relabel=True) for _ in range(args.perms)]
            p_ratio = sum(1 for x in perms if x[0] <= ratio) / len(perms) if perms else float("nan")
            p_log = sum(1 for x in perms if x[1] <= mlog) / len(perms) if perms else float("nan")
            p_solved = sum(1 for x in perms if x[2] >= ds) / len(perms) if perms else float("nan")
            a, b = f"{group}: regime oracle", f"{group}: luck oracle"
            better = sum(1 for s in stems if per_cell[s][a][1] < 0.99 * per_cell[s][b][1])
            worse = sum(1 for s in stems if per_cell[s][a][1] > 1.01 * per_cell[s][b][1])
            print(f"  {group}: regime oracle v luck oracle: tick PAR-2 ratio {ratio:.3f} (p {p_ratio:.3f}); "
                  f"cost ratio per cell, geometric mean {math.exp(mlog):.3f} (p {p_log:.3f}); solved {ds:+.1f} "
                  f"(p {p_solved:.3f}); cells cheaper {better}, dearer {worse}, within 1 % {len(stems) - better - worse}")
    # per family (the project's reporting rule): where the totals come from.
    # Solved counts and tick PAR-2 of the parent and of the two oracles of
    # each group, so a gain that is one family's win and another's loss shows.
    fams = sorted({c["family"] for c in cells.values()})
    groups = [g for g in ("stint", "block") if all(cell_arms[s][g] and cell_arms[s]["reroll"] for s in cells)]
    print(f"\nper family ({len(fams)} families; solved, and tick PAR-2 v stock, for the regime oracle and the luck oracle)")
    head = f"  {'family':<22s} {'cells':>5s} {'stock':>6s}"
    for g in groups:
        head += f" | {g + ' regime':>13s} {'luck':>6s} {'regime':>7s} {'luck':>6s}"
    print(head)
    lines = []
    for fam in fams:
        stems = sorted(s for s, c in cells.items() if c["family"] == fam)
        base = sum(per_cell[s]["stock (the parent)"][1] for s in stems)
        row = f"  {fam[:22]:<22s} {len(stems):5d} {sum(per_cell[s]['stock (the parent)'][0] for s in stems):6.1f}"
        for g in groups:
            a, b = f"{g}: regime oracle", f"{g}: luck oracle"
            sa, sb = sum(per_cell[s][a][0] for s in stems), sum(per_cell[s][b][0] for s in stems)
            ca, cb = sum(per_cell[s][a][1] for s in stems), sum(per_cell[s][b][1] for s in stems)
            row += (f" | {sa:13.1f} {sb:6.1f} {ca / base if base else float('nan'):7.3f} "
                    f"{cb / base if base else float('nan'):6.3f}")
        lines.append(row)
    print("\n".join(lines))
    out_dir = run / "report"
    out_dir.mkdir(exist_ok=True)
    with open(out_dir / "regime_per_cell.tsv", "w", newline="") as f:
        arms_all = [a for a in order if all(a in per_cell[s] for s in cells)]
        w = csv.writer(f, dialect="excel-tab", lineterminator="\n")
        w.writerow(["stem", "family", "class", "arm", "solved", "cost"])
        for s in sorted(cells):
            for a in arms_all:
                w.writerow([s, cells[s]["family"], cells[s]["class"], a, f"{per_cell[s][a][0]:.3f}", f"{per_cell[s][a][1]:.6g}"])
    print(f"per cell and arm: {out_dir / 'regime_per_cell.tsv'}")
    print(f"\nreading: an oracle picks the best arm per cell after the fact, so it always beats stock; the best of as "
          f"many rerolls does too, by luck alone. Regimes have alpha only where the regime oracle beats the luck "
          f"oracle. Each p is the share of {args.perms} relabelings (within every cell, which children count as "
          f"regimes and which as rerolls is shuffled) that read at least as well as the real labels: a small p "
          f"says the regimes did better than rerolls would have. A constant regime's row is that regime on every cell.")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    m = sub.add_parser("make")
    m.add_argument("--cells", default=str(CELLS))
    m.add_argument("--stock", required=True, help="the stock pass directory (converted)")
    m.add_argument("--band", required=True, help="the 3600 s band pass directory (converted)")
    m.add_argument("--rescue", help="results.tsv of a longer stock run on the band timeouts: its solved cells first")
    m.add_argument("--min-time", type=float, default=300.0, help="slow cells: stock seconds at least this (default 300)")
    m.add_argument("--min-decisions", type=int, default=60, help="decisions of the parent at least this (default 60)")
    m.add_argument("--per-family", type=int, default=3, help="slow cells per family, the longest first (default 3)")
    m.add_argument("--timeout-n", type=int, default=36, help="band cells that do not solve (default 36)")
    m.add_argument("--point", type=float, default=0.30, help="the point, as a share of the parent's decisions")
    m.add_argument("--block", type=float, default=0.25, help="the block hold, as a share of the parent's decisions")
    m.add_argument("--rerolls", type=int, default=5)
    m.add_argument("--branch-jobs", type=int, default=7)
    m.add_argument("--slots", type=int, default=32, help="process slots of the pass (projection only)")
    m.add_argument("--out", required=True)
    m.set_defaults(func=cmd_make)
    r = sub.add_parser("report")
    r.add_argument("run_dir")
    r.add_argument("--cells", default=str(CELLS))
    r.add_argument("--partial", action="store_true", help="peek at a pass that is not DONE (jobs still missing)")
    r.add_argument("--perms", type=int, default=2000, help="relabelings for the p values (default 2000)")
    r.add_argument("--seed", type=int, default=1)
    r.set_defaults(func=cmd_report)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
