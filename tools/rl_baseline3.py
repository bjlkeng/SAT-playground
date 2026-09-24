#!/usr/bin/env python3
"""rl_baseline3.py — baseline 3: one constant per instance, chosen at t = 0 from static features.

Plan: plan/rl-scheduler-solver13-plan.md §6.1 step 3 (the algorithm-
configuration baseline), §8 (splits, tick-deterministic comparison), §13
"Beyond the current purview". Bead SAT-playground-p9m.9.1 (step D.1).

The question: before any epoch policy, how much of the step-0 headroom
does a per-instance choice of ONE constant capture, when the choice is made
once, before search, from the static features the solver computes itself?
If most of it, the epoch machinery is a follow-up; if none, the headroom
(if any) lives in the dynamics.

Data. The step-0 sweeps (solver README "Step-0 constant-knob sweeps"): 32
constants of 16 knobs plus stock, each run on the 100 cells of
benchmarks/sat-comp-2025-medium at 1800 s. Every cell has every arm, so
the realized cost of any choice is known without a new run. Cost is tick
PAR-2 on the work clock at k_res = 7 (recomputed from the sweep's ticks
and eliminate_resolutions): W when solved, 2 x the W reached at the kill
when not. Each arm's cost is taken relative to its own stage's stock run.
The features are computed after stock preprocessing (plan D0), so the
selector can only be deployed for options that act after it. Kissat's
preprocessing (preprocess.c, probe.c `probe_initially`) runs congruence,
backbone, sweep, substitute, factor and fast elimination, so the sweep,
backbone-effort and factor-effort arms are left out by default: their
sweep outcomes had the option on during preprocessing too, which a D0
choice cannot reproduce. --all-arms puts them back as a diagnostic.

Model. For each arm a ridge regression of log(cost_arm / cost_stock) on
the standardized actor-tier static features: the block the solver itself
computes before search (the `s_*` values of the stock trace footers, the
same hygiene as tools/rl_normalize.py `predict`). The critic-tier
estimates of tools/rl_features.py (compression ratio, modularity,
treewidth, communities) are offline-only and cost extraction time the
solver never pays, so they stay out unless --critic asks for them as a
diagnostic. The selector picks the arm
with the lowest predicted ratio when it beats stock by more than a margin
delta (in log cost), else stock. Ridge strength and delta are chosen by
leave-one-out over the training cells of the medium suite (78 of the 100;
the split of benchmarks/rl/split_2025.tsv; the 3 shared cells are never
used): the held-out cell's choice is scored with its realized sweep cost,
so the LOO number is an honest estimate of what the selector would do on
a new cell of this distribution.

Evaluation. Two views. (1) The 19 medium-suite cells in the validation
split: realized from the sweeps. (2) The whole 99-cell validation split:
the chosen constant per cell is run tick-deterministically at B_cell
(`fit --jobs` writes the collector table; cells whose choice is stock
reuse the D.3 pass's plain-solver arm), next to the best global constant
of step 0 (reorderint 20000, baseline 2), and `report` prints solved and
tick PAR-2 against stock per family.

    ~/.cache/sat13-rl/venv/bin/python tools/rl_baseline3.py fit \\
        --stage1 log/abtest-rl-step0-stage1-<ts> --stage2 log/abtest-rl-step0-stage2-<ts> \\
        --stock log/rl-stock2025-<ts> --out benchmarks/rl/baseline3.json --jobs /path/jobs-b3.tsv
    python3 tools/rl_collect.py table /path/jobs-b3.tsv --suite sat-comp-2025 --name b3 --jobs 4 --cores 14,15,16,17 --force ...
    ~/.cache/sat13-rl/venv/bin/python tools/rl_baseline3.py report --d3 log/rl-d3clone-<ts> --run log/rl-b3-<ts>
"""
from __future__ import annotations

import argparse
import csv
import json
import math
import os
import sys
from collections import defaultdict
from pathlib import Path

# the ridge fits are tiny; keep numpy's BLAS off the cores a running sweep uses
os.environ.setdefault("OMP_NUM_THREADS", "2")
os.environ.setdefault("OPENBLAS_NUM_THREADS", "2")
import numpy as np  # noqa: E402
import pyarrow.parquet as pq

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from rl_split import assert_training_only, load_split  # noqa: E402
from rl_sweep_report import failed as sweep_failed, is_solved as sweep_solved  # noqa: E402

CELLS = ROOT / "benchmarks" / "rl" / "cells_2025.tsv"
STATIC = ROOT / "benchmarks" / "rl" / "static_features.tsv"
MEDIUM = ROOT / "benchmarks" / "sat-comp-2025-medium"
CRITIC_COLS = ("compression_ratio", "modularity", "treewidth_ub", "communities")
K_RES = 7
SOLVED = ("SATISFIABLE", "UNSATISFIABLE", "SAT", "UNSAT")
# step-0 arm tag -> kissat options (comma-separated, the collector's SAT_EXTRA_ARGS form)
ARM_OPTIONS = {
    "probeint50": "--probeint=50", "probeint200": "--probeint=200",
    "eliminateint250": "--eliminateint=250", "eliminateint1000": "--eliminateint=1000",
    "reduceint500": "--reduceint=500", "reduceint2000": "--reduceint=2000",
    "rephaseint500": "--rephaseint=500", "rephaseint2000": "--rephaseint=2000",
    "reorderint5000": "--reorderint=5000", "reorderint20000": "--reorderint=20000",
    "modeint500": "--modeint=500", "modeint2000": "--modeint=2000",
    "restartmargin5": "--restartmargin=5", "restartmargin20": "--restartmargin=20",
    "sweepoff": "--sweep=0", "sweepeffort50": "--sweepeffort=50", "sweepeffort200": "--sweepeffort=200",
    "vivifyeffort50": "--vivifyeffort=50", "vivifyeffort200": "--vivifyeffort=200",
    "eliminateeffort50": "--eliminateeffort=50", "eliminateeffort200": "--eliminateeffort=200",
    "backboneeffort10": "--backboneeffort=10", "backboneeffort40": "--backboneeffort=40",
    "factoreffort25": "--factoreffort=25", "factoreffort100": "--factoreffort=100",
    "forwardeffort50": "--forwardeffort=50", "forwardeffort200": "--forwardeffort=200",
    "transitiveeffort10": "--transitiveeffort=10", "transitiveeffort40": "--transitiveeffort=40",
    "walkeffort25": "--walkeffort=25", "walkeffort100": "--walkeffort=100",
    "reducefrachalf": "--reducelow=250,--reducehigh=450",
}
# arms whose option acts during kissat's preprocessing (before the features exist)
PREPROCESS_ARMS = {"sweepoff", "sweepeffort50", "sweepeffort200", "backboneeffort10", "backboneeffort40",
                   "factoreffort25", "factoreffort100"}
BEST_GLOBAL = "reorderint20000"     # baseline 2 (plan §11, 2026-09-16)
LAMBDAS = (1.0, 3.0, 10.0, 30.0, 100.0, 300.0, 1000.0, 3000.0)
DELTAS = (0.0, 0.05, 0.1, 0.2, 0.3, 0.5)
JOB_COLUMNS = ("stem", "tag", "flavour", "seed", "limit_ticks", "wall_s", "procs", "peak_rss_mb", "env")


# ---------------------------------------------------------------------------
# data
# ---------------------------------------------------------------------------

def load_cells(path: Path) -> dict[str, dict]:
    with open(path, newline="") as f:
        lines = [ln for ln in f if not ln.startswith("#")]
    return {r["stem"]: r for r in csv.DictReader(lines, dialect="excel-tab")}


def load_sweep(run: Path) -> dict[str, dict[str, dict]]:
    """arm tag -> stem -> row of the sweep's results.tsv (seed 0 only). A
    row the sweep report classes as a correctness failure (a model that
    fails verification, a missing proof, a panic, a premature UNKNOWN)
    stops the fit: such a sweep is debugged, not learned from."""
    arms: dict[str, dict[str, dict]] = {}
    bad = []
    for d in sorted(run.iterdir()):
        f = d / "results.tsv"
        if not d.is_dir() or not f.is_file():
            continue
        with open(f, newline="") as fh:
            rows = [r for r in csv.DictReader(fh, dialect="excel-tab") if r.get("seed", "0") == "0"]
        for r in rows:
            why = sweep_failed(r)
            if why:
                bad.append(f"{d.name}/{r['instance']}: {why}")
        arms[d.name] = {r["instance"]: r for r in rows}
    if bad:
        raise SystemExit(f"{run}: {len(bad)} correctness failure(s) in the sweep, e.g. {bad[:3]}; "
                         f"CLAUDE.md 'Correctness is absolute': debug before fitting")
    return arms


def solved(r: dict) -> bool:
    """A verified, complete answer (rl_sweep_report.is_solved): an answer cut
    short or failing verification is never a solve."""
    return sweep_solved(r)


def cost7(r: dict) -> float:
    """Tick PAR-2 cost at k_res = 7: W when solved, twice the W at the kill when not."""
    w = float(r["ticks"]) + K_RES * float(r["eliminate_resolutions"])
    return w if solved(r) else 2.0 * w


def raw_matrix(stock: Path, static_tsv: Path, stems: list[str], critic_tier: bool = False):
    """The raw static features of `stems` (missing as nan) and their names."""
    runs = pq.read_table(stock / "dataset" / "runs.parquet").to_pydict()
    n = len(runs["stem"])
    static_cols = sorted(k for k in runs if k.startswith("s_"))
    by_stem = {}
    for i in range(n):
        if runs["is_child"][i] or runs["tag"][i] != "stock" or runs.get("failed", [False] * n)[i]:
            continue
        by_stem[runs["stem"][i]] = i
    critic: dict[str, dict] = {}
    if critic_tier and static_tsv.is_file():
        with open(static_tsv, newline="") as f:
            for r in csv.DictReader(f, dialect="excel-tab"):
                if r["suite"] == "sat-comp-2025":
                    critic[r["stem"]] = r
    names = static_cols + ([f"c_{c}" for c in CRITIC_COLS] + ["c_log_vars", "c_log_clauses"] if critic_tier else [])
    rows = []
    for s in stems:
        i = by_stem.get(s)
        if i is None:
            raise SystemExit(f"{s}: no stock run in {stock}")
        cr = critic.get(s, {})
        v = [float(runs[c][i]) if runs[c][i] is not None else float("nan") for c in static_cols]
        if critic_tier:
            for c in CRITIC_COLS:
                x = cr.get(c, "")
                v.append(float(x) if x not in ("", None) else float("nan"))
            v.append(math.log10(float(cr["p_vars"])) if cr.get("p_vars") else float("nan"))
            v.append(math.log10(float(cr["p_clauses"])) if cr.get("p_clauses") else float("nan"))
        rows.append(v)
    return np.array(rows, dtype=np.float64), names


def hygiene(X_raw: np.ndarray, names: list[str], fit: np.ndarray):
    """Standardized features (with an intercept), the hygiene fitted on the
    rows `fit` marks exactly as tools/rl_normalize.py `predict` does:
    constant and all-missing columns dropped, counts above 1000 log1p'd,
    missing values filled with the fitted mean, then scaled. Rows outside
    `fit` are only transformed, so a held-out cell never shapes its own
    features."""
    X = X_raw.copy()
    keep = [j for j in range(X.shape[1])
            if not np.all(np.isnan(X[fit, j])) and np.nanstd(X[fit, j]) >= 1e-12]
    X = X[:, keep]
    kept = [names[j] for j in keep]
    big = np.nanmax(np.abs(X[fit]), axis=0) > 1000
    X[:, big] = np.sign(X[:, big]) * np.log1p(np.abs(X[:, big]))
    mu = np.nanmean(X[fit], axis=0)
    X = np.where(np.isnan(X), mu, X)
    sd = X[fit].std(axis=0)
    sd[sd < 1e-12] = 1.0
    Z = np.concatenate([(X - mu) / sd, np.ones((len(X), 1))], axis=1)
    return Z, kept, {"keep": keep, "big": big.tolist(), "mu": mu.tolist(), "sd": sd.tolist()}


# ---------------------------------------------------------------------------
# the selector
# ---------------------------------------------------------------------------

def ridge_all(Z: np.ndarray, Y: np.ndarray, lam: float) -> np.ndarray:
    """One ridge fit per column of Y (arms); the intercept is not penalized."""
    d = Z.shape[1]
    P = lam * np.eye(d)
    P[-1, -1] = 0.0
    return np.linalg.solve(Z.T @ Z + P, Z.T @ Y)


def choose(pred: np.ndarray, delta: float) -> np.ndarray:
    """Per row the arm index with the lowest predicted log ratio when it is
    below -delta, else -1 (stock)."""
    best = np.argmin(pred, axis=1)
    return np.where(pred[np.arange(len(pred)), best] < -delta, best, -1)


def realized(choice: np.ndarray, costs: np.ndarray, base_cost: np.ndarray, sol: np.ndarray,
             base_sol: np.ndarray) -> tuple[float, float, int, int]:
    """(tick PAR-2 of the choices, of stock, solved by the choices, by stock)."""
    c = np.where(choice >= 0, costs[np.arange(len(choice)), np.maximum(choice, 0)], base_cost)
    s = np.where(choice >= 0, sol[np.arange(len(choice)), np.maximum(choice, 0)], base_sol)
    return float(c.sum()), float(base_cost.sum()), int(s.sum()), int(base_sol.sum())


def loo_predictions(X_raw: np.ndarray, names: list[str], Y: np.ndarray, lams) -> dict[float, np.ndarray]:
    """Per ridge strength, the held-out prediction of every training row:
    the feature hygiene and the regression are both fitted on the other
    rows, so the held-out cell shapes nothing about its own prediction."""
    n = len(X_raw)
    out = {lam: np.empty((n, Y.shape[1])) for lam in lams}
    for i in range(n):
        m = np.arange(n) != i
        Z, _, _ = hygiene(X_raw, names, m)
        for lam in lams:
            W = ridge_all(Z[m], Y[m], lam)
            out[lam][i] = Z[i] @ W
    return out


def cmd_fit(args) -> int:
    split = load_split()
    cells = load_cells(Path(args.cells))
    medium = sorted(p.name[:-len(".cnf.xz")] for p in MEDIUM.glob("*.cnf.xz"))
    sweeps = [load_sweep(Path(args.stage1)), load_sweep(Path(args.stage2))]
    arms: list[str] = []
    arm_rows: dict[str, dict[str, dict]] = {}
    base_rows: dict[str, dict[str, dict]] = {}
    for sw in sweeps:
        if "base" not in sw:
            raise SystemExit("a sweep has no base arm")
        for tag in sorted(sw):
            if tag == "base" or (tag in PREPROCESS_ARMS and not args.all_arms):
                continue
            if tag not in ARM_OPTIONS:
                raise SystemExit(f"arm {tag}: no option mapping")
            arms.append(tag)
            arm_rows[tag] = sw[tag]
            base_rows[tag] = sw["base"]
    # every arm of both sweeps must agree on SAT v UNSAT per instance: a
    # contradiction is a solver bug and no training label (CLAUDE.md
    # "Correctness is absolute")
    contradictions = []
    for stem in medium:
        answers = {r["result"].upper()[:3] for sw in sweeps for tag in sw
                   for r in [sw[tag].get(stem)] if r is not None and solved(r)}
        if len(answers) > 1:
            contradictions.append(f"{stem}: {sorted(answers)}")
    if contradictions:
        raise SystemExit(f"{len(contradictions)} instance(s) with contradicting answers across the sweep arms, e.g. "
                         f"{contradictions[:3]}; debug before fitting")
    # a row without a work measurement (an out-of-memory abort or a forced
    # kill leaves ticks NA) cannot be priced: the cell leaves the fit, in
    # every arm, and is reported
    def has_work(row):
        return all(row.get(c, "NA") not in ("", "NA") for c in ("ticks", "eliminate_resolutions"))
    unpriced = sorted(s for s in medium if any(s in arm_rows[a] and s in base_rows[a]
                                               and not (has_work(arm_rows[a][s]) and has_work(base_rows[a][s]))
                                               for a in arms))
    if unpriced:
        print(f"  {len(unpriced)} cell(s) without a work measurement in some arm left out: {unpriced[:5]}")
    stems = [s for s in medium if all(s in arm_rows[a] and s in base_rows[a] for a in arms)
             and split.get(s) in ("train", "val") and s not in unpriced]
    is_train = np.array([split[s] == "train" for s in stems])
    is_val = ~is_train
    assert_training_only([s for s, t in zip(stems, is_train) if t])
    # costs and labels
    costs = np.array([[cost7(arm_rows[a][s]) for a in arms] for s in stems])
    base_cost = np.array([[cost7(base_rows[a][s]) for a in arms] for s in stems])
    sol = np.array([[solved(arm_rows[a][s]) for a in arms] for s in stems])
    base_sol = np.array([[solved(base_rows[a][s]) for a in arms] for s in stems])
    Y = np.log(costs / base_cost)
    # stock's own cost per cell: the stage-1 base (identical W on solved cells across stages)
    stock_cost = base_cost[:, 0]
    stock_sol = base_sol[:, 0]
    X_raw, raw_names = raw_matrix(Path(args.stock), Path(args.static), stems, args.critic)
    Z, names, hyg = hygiene(X_raw, raw_names, is_train)
    n_tr = int(is_train.sum())
    print(f"{len(stems)} medium-suite cells with every arm ({n_tr} train, {int(is_val.sum())} val; shared left out), "
          f"{len(arms)} arms ({'preprocessing-time arms included' if args.all_arms else 'the 7 preprocessing-time arms left out'}), "
          f"{len(names)} features")
    # the sweeps' own baselines on the training cells
    def summary(label, choice, mask):
        c, b, s, bs = realized(choice[mask], costs[mask], stock_cost[mask], sol[mask], stock_sol[mask])
        return f"{label}: solved {s} v stock {bs}, tick PAR-2 {c / b:.4f}x stock"
    # the oracle: solved first, then cost (an arm that solves beats an
    # unsolved stock whatever its work; among solves the cheaper wins)
    best = np.argmin(np.where(sol, costs, np.inf), axis=1)
    rows_ = np.arange(len(stems))
    arm_solves = sol[rows_, best]
    oracle = np.where(arm_solves & (~stock_sol | (costs[rows_, best] < stock_cost)), best, -1)
    print("  " + summary(f"per-cell oracle over the {len(arms)} constants (train)", oracle, is_train))
    gi = arms.index(BEST_GLOBAL)
    print("  " + summary(f"best global constant {BEST_GLOBAL} (train)", np.full(len(stems), gi), is_train))
    # leave-one-out over lambda and delta (hygiene and regression refitted per fold)
    Ztr, Ytr = Z[is_train], Y[is_train]
    preds = loo_predictions(X_raw[is_train], raw_names, Ytr, LAMBDAS)
    best = None
    table = []
    for lam in LAMBDAS:
        for delta in DELTAS:
            ch = choose(preds[lam], delta)
            c, b, s, bs = realized(ch, costs[is_train], stock_cost[is_train], sol[is_train], stock_sol[is_train])
            n_dev = int((ch >= 0).sum())
            table.append((lam, delta, c / b, s, bs, n_dev))
            # the project's metric order: solved cells, then tick PAR-2; a
            # tie goes to the larger margin (fewer cells moved off stock)
            key = (-s, c / b, -delta)
            if best is None or key < best[0]:
                best = (key, lam, delta, c / b, s, bs, n_dev)
    print(f"  leave-one-out on the {n_tr} training cells (tick PAR-2 v stock; solved v stock; cells moved off stock):")
    for lam, delta, r, s, bs, nd in table:
        mark = " <-" if (lam, delta) == (best[1], best[2]) else ""
        print(f"    lambda {lam:7.1f} delta {delta:.2f}: {r:.4f}x, {s} v {bs}, {nd} moved{mark}")
    _, lam, delta, loo_ratio, loo_s, loo_bs, loo_dev = best
    print(f"  chosen: lambda {lam}, delta {delta}: LOO tick PAR-2 {loo_ratio:.4f}x stock, solved {loo_s} v {loo_bs}, "
          f"{loo_dev} of {n_tr} cells moved off stock")
    # final fit on all training cells; the 19 validation medium cells, realized
    W = ridge_all(Ztr, Ytr, lam)
    pred = Z @ W
    choice = choose(pred, delta)
    print("  " + summary("validation medium cells, realized from the sweeps", choice, is_val))
    print("  " + summary(f"validation medium cells, {BEST_GLOBAL}", np.full(len(stems), gi), is_val))
    fam_of = {s: cells.get(s, {}).get("family", "?") for s in stems}
    per_fam = defaultdict(list)
    for i in np.nonzero(is_val)[0]:
        per_fam[fam_of[stems[i]]].append(i)
    for fam, idx in sorted(per_fam.items(), key=lambda kv: -len(kv[1])):
        m = np.zeros(len(stems), bool)
        m[idx] = True
        print("    " + summary(f"{fam} ({len(idx)})", choice, m))
    val_choices = {stems[i]: (arms[choice[i]] if choice[i] >= 0 else "stock") for i in np.nonzero(is_val)[0]}
    # the whole validation split: predict for every val cell with a stock
    # trace, through the hygiene fitted on the training cells
    val_all = sorted(s for s, k in split.items() if k == "val" and cells.get(s, {}).get("B_cell"))
    extra = [s for s in val_all if s not in stems]
    Xv_raw, _ = raw_matrix(Path(args.stock), Path(args.static), stems + extra, args.critic)
    Zv, _, _ = hygiene(Xv_raw, raw_names, np.concatenate([is_train, np.zeros(len(extra), bool)]))
    pred_v = Zv[len(stems):] @ W
    choice_v = choose(pred_v, delta)
    all_choices = dict(val_choices)
    for s, c in zip(extra, choice_v):
        all_choices[s] = arms[c] if c >= 0 else "stock"
    moved = {s: a for s, a in all_choices.items() if a != "stock"}
    counts = defaultdict(int)
    for a in moved.values():
        counts[a] += 1
    # the constant the selector picks most often, as a global arm of its own:
    # what the per-instance choice adds over it is baseline 3's real margin
    modal = max(counts, key=counts.get) if counts else None
    print(f"  validation split ({len(all_choices)} cells): {len(moved)} moved off stock: "
          + ", ".join(f"{a} {n}" for a, n in sorted(counts.items(), key=lambda kv: -kv[1])))
    top = np.argsort(-np.abs(W[:-1, gi]))[:6]
    print(f"  largest |weights| for {BEST_GLOBAL}: " + ", ".join(f"{names[j]} {W[j, gi]:+.3f}" for j in top))
    out = {"stage1": str(args.stage1), "stage2": str(args.stage2), "stock": str(args.stock), "k_res": K_RES,
           "critic_tier": bool(args.critic), "all_arms": bool(args.all_arms),
           "arms": arms, "options": {a: ARM_OPTIONS[a] for a in arms}, "features": names, "hygiene": hyg,
           "lambda": lam, "delta": delta, "weights": W.tolist(),
           "loo": {"tick_par2_ratio": loo_ratio, "solved": loo_s, "stock_solved": loo_bs, "moved": loo_dev,
                   "train_cells": n_tr, "grid": [{"lambda": l, "delta": d, "ratio": r, "solved": s, "moved": nd}
                                                 for l, d, r, s, bs, nd in table]},
           "train_cells": [s for s, t in zip(stems, is_train) if t],
           "val_medium_cells": [s for s, t in zip(stems, is_val) if t],
           "val_choices": all_choices, "modal": modal}
    Path(args.out).write_text(json.dumps(out, indent=1) + "\n")
    print(f"wrote {args.out}")
    if args.jobs:
        rows = []
        for s in val_all:
            c = cells[s]
            budget = int(c["B_cell"])
            rate = float(c.get("work_per_s") or 0) or 3e7
            cap = int(3.0 * budget / rate) + 300
            rss = c.get("peak_rss_mb", "")
            common = {"stem": s, "seed": int(c.get("seed") or 0), "limit_ticks": budget, "wall_s": cap, "procs": 1,
                      "peak_rss_mb": rss}
            if all_choices[s] != "stock":
                rows.append({**common, "tag": "b3", "flavour": "off",
                             "env": f"SAT_EXTRA_ARGS={ARM_OPTIONS[all_choices[s]]}"})
            rows.append({**common, "tag": "reorder20k", "flavour": "off",
                         "env": f"SAT_EXTRA_ARGS={ARM_OPTIONS[BEST_GLOBAL]}"})
            if modal and modal != BEST_GLOBAL:
                rows.append({**common, "tag": "modal", "flavour": "off",
                             "env": f"SAT_EXTRA_ARGS={ARM_OPTIONS[modal]}"})
        with open(args.jobs, "w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=list(JOB_COLUMNS), dialect="excel-tab", lineterminator="\n")
            w.writeheader()
            for r in rows:
                w.writerow(r)
        print(f"wrote {args.jobs}: {len(rows)} jobs ({len(moved)} b3 cells off stock, {len(val_all)} reorder20k"
              + (f", {len(val_all)} modal = {modal}" if modal and modal != BEST_GLOBAL else "") + ")")
    return 0


# ---------------------------------------------------------------------------
# report
# ---------------------------------------------------------------------------

def read_results(run: Path) -> dict[str, dict[str, dict]]:
    with open(run / "results.tsv", newline="") as f:
        recs = list(csv.DictReader(f, dialect="excel-tab"))
    out: dict[str, dict[str, dict]] = defaultdict(dict)
    for r in recs:
        out[r["tag"]][r["stem"]] = r
    return out


def cmd_report(args) -> int:
    cells = load_cells(Path(args.cells))
    model = json.loads(Path(args.model).read_text())
    d3 = read_results(Path(args.d3))
    b3 = read_results(Path(args.run))
    stock = d3.get(args.stock_arm) or {}
    if not stock:
        raise SystemExit(f"{args.d3}: no arm {args.stock_arm}")
    choices = model["val_choices"]

    def ok(r):
        # flagged or capped runs leave the totals; a run without a work
        # clock (a memory abort) stays, priced as unsolved
        return r and r.get("failed", "0") != "1" and not r.get("anomaly")

    failures = sorted({s for t in b3 for s, r in b3[t].items() if r.get("failed", "0") == "1"}
                      | {s for s, r in stock.items() if r.get("failed", "0") == "1"})
    for s in failures:
        print(f"*** CORRECTNESS FAILURE: the collector flagged {s}")

    def is_solved(r):
        return r["result"] in ("SAT", "UNSAT") and r.get("work", "NA") != "NA"

    def cost(r):
        return float(r["work"]) if is_solved(r) else 2.0 * float(r["limit_ticks"])

    # The arms and jobs the pass was asked to run come from its job table
    # (as rl_arms.py does), so a constant whose jobs never finished cannot
    # vanish from a comparison that then looks complete.
    expected: dict[str, set] = defaultdict(set)
    for run_dir, tags in ((Path(args.run), None), (Path(args.d3), {args.stock_arm})):
        jt = run_dir / "jobs.tsv"
        if jt.is_file():
            with open(jt, newline="") as f:
                for j in csv.DictReader(f, dialect="excel-tab"):
                    if tags is None or j["tag"] in tags:
                        expected[j["tag"]].add(j["stem"])
    have = {t: set(b3[t]) for t in b3}
    have[args.stock_arm] = set(stock)
    missing = {t: len(stems_ - have.get(t, set())) for t, stems_ in expected.items()}
    missing = {t: m for t, m in missing.items() if m}
    if missing:
        print(f"*** pass incomplete: " + ", ".join(f"{t}: {m} job(s) without a record" for t, m in missing.items()))
        if not args.partial:
            return 1
    others = sorted((set(b3) | set(expected)) - {"b3", args.stock_arm})
    others = [t for t in others if t in b3]
    names = ["stock", "b3"] + others
    # arms must agree on SAT v UNSAT (CLAUDE.md "Correctness is absolute"):
    # checked over every answer of every cell before any cell is dropped,
    # whether or not the run left a work reading, so neither a capped, a
    # missing nor an unpriced arm can hide a conflict
    all_recs: dict[str, dict[str, dict]] = defaultdict(dict)
    for s, rr in stock.items():
        all_recs[s]["stock"] = rr
    for t in b3:
        for s, rr in b3[t].items():
            all_recs[s][t] = rr
    contradictions = sorted(
        s for s, recs in all_recs.items()
        if len({rr["result"] for rr in recs.values() if rr["result"] in ("SAT", "UNSAT")
                and rr.get("anomaly") != "wall_cap"}) > 1)
    for s in contradictions:
        print(f"*** CORRECTNESS FAILURE: arms disagree on {s}: "
              + ", ".join(f"{a}={rr['result']}" for a, rr in all_recs[s].items()))
    arms: dict[str, dict[str, dict]] = {a: {} for a in names}
    budget_mismatch = []
    for s in choices:
        if s in contradictions:
            continue
        rec = {"stock": stock.get(s), "b3": b3.get("b3", {}).get(s) if choices[s] != "stock" else stock.get(s)}
        for t in others:
            rec[t] = b3.get(t, {}).get(s)
        if all(ok(rec[a]) for a in names):
            # the two passes must have run the cell on the same work budget:
            # an unsolved run costs twice its budget, so a smaller budget in
            # one pass would pass for a gain
            if len({rec[a].get("limit_ticks", "") for a in names}) > 1:
                budget_mismatch.append(s)
                continue
            for a in names:
                arms[a][s] = rec[a]
    if budget_mismatch:
        print(f"*** {len(budget_mismatch)} cell(s) with different work budgets between the passes, e.g. "
              f"{budget_mismatch[:3]}: not comparable")
        return 1
    stems = sorted(arms["stock"])
    print(f"{len(stems)} validation cells with every arm {names} ({len(choices) - len(stems)} missing, flagged or "
          f"contradicting); modal constant {model.get('modal')}")

    def line(label, rs):
        out = [f"{label:28s} n={len(rs):3d}"]
        base = sum(cost(arms['stock'][s]) for s in rs)
        for a in names:
            sv = sum(1 for s in rs if is_solved(arms[a][s]))
            c = sum(cost(arms[a][s]) for s in rs)
            out.append(f"{a} {sv:3d} / {c / base if base else float('nan'):.4f}")
        return "  ".join(out)
    print(line("all (solved / tick PAR-2 v stock)", stems))
    fams = defaultdict(list)
    for s in stems:
        fams[cells.get(s, {}).get("family", "?")].append(s)
    for fam, rs in sorted(fams.items(), key=lambda kv: -len(kv[1])):
        print(line(fam[:28], rs))
    moved = [s for s in stems if choices[s] != "stock"]
    better = sum(1 for s in moved if cost(arms["b3"][s]) < cost(arms["stock"][s]))
    worse = sum(1 for s in moved if cost(arms["b3"][s]) > cost(arms["stock"][s]))
    print(f"b3 moved {len(moved)} cells off stock: cheaper on {better}, dearer on {worse}, "
          f"solved gained {sum(1 for s in moved if is_solved(arms['b3'][s]) and not is_solved(arms['stock'][s]))}, "
          f"lost {sum(1 for s in moved if not is_solved(arms['b3'][s]) and is_solved(arms['stock'][s]))}")
    if contradictions or failures:
        print(f"*** {len(contradictions)} cell(s) with contradicting answers and {len(failures)} flagged by the "
              f"collector: a correctness failure, the comparison is not valid")
        return 1
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("fit")
    f.add_argument("--stage1", required=True)
    f.add_argument("--stage2", required=True)
    f.add_argument("--stock", required=True, help="the converted stock pass (static features in its footers)")
    f.add_argument("--static", default=str(STATIC))
    f.add_argument("--critic", action="store_true",
                   help="also use the critic-tier estimates (offline-only; a diagnostic, not the baseline)")
    f.add_argument("--all-arms", action="store_true",
                   help="also offer the sweep, backbone-effort and factor-effort arms, which act during preprocessing")
    f.add_argument("--cells", default=str(CELLS))
    f.add_argument("--out", default=str(ROOT / "benchmarks" / "rl" / "baseline3.json"))
    f.add_argument("--jobs", help="collector job table for the validation-split run")
    f.set_defaults(func=cmd_fit)
    r = sub.add_parser("report")
    r.add_argument("--d3", required=True, help="the D.3 pass (its plain-solver arm is stock)")
    r.add_argument("--run", required=True, help="the baseline-3 pass (arms b3 and reorder20k)")
    r.add_argument("--stock-arm", default="off")
    r.add_argument("--model", default=str(ROOT / "benchmarks" / "rl" / "baseline3.json"))
    r.add_argument("--cells", default=str(CELLS))
    r.add_argument("--partial", action="store_true", help="report passes whose jobs are not all done")
    r.set_defaults(func=cmd_report)
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
