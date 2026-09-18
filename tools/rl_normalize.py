#!/usr/bin/env python3
"""rl_normalize.py — observation normalization and the static-feature runtime predictor.

Plan: plan/rl-scheduler-solver13-plan.md §3 (normalization fitted on the
stock traces and baked into the weights file), §5.2, §6.2 item 3
"diagnostic". Bead SAT-playground-p9m.7.9 (step B.9).

`norm` fits the per-entry mean and standard deviation of the 251-entry
observation vector over the boundary rows of the training-split stock
traces and writes them as benchmarks/rl/obs_norm_2025.json (names, the
layout hash the solver checks, mean, std, counts). `--net <file>` also
writes a stock-biased fixture net that carries them, in the weights-file
format (solver/13-kissat-rs/tools/rl/policy_net.py `write_net`), which
is how a trainer will ship its own model; loading it in the solver at an
infinite margin is the acceptance check (the solver validates the layout
hash and every array length).

`predict` is the diagnostic: how much of the stock solve time do static
features explain before any dynamics? Ridge regression of log10(stock
time) on the standardized static features (the solver's 140 actor-tier
values from the log footers plus the critic-tier estimates of
tools/rl_features.py) on the solved training cells, ridge strength by
5-fold cross-validation inside training, error reported on the solved
validation cells overall and per family; and a logistic "solves within
the wall limit" classifier for the censored cells. A predictor that beats
the constant baseline says the features carry signal (the critic will use
them); its per-family error is the sanity check on the features.

    ~/.cache/sat13-rl/venv/bin/python tools/rl_normalize.py norm log/rl-stock2025-<ts> [--net /path/norm.net.bin]
    ~/.cache/sat13-rl/venv/bin/python tools/rl_normalize.py predict log/rl-stock2025-<ts>
"""
from __future__ import annotations

import argparse
import csv
import json
import math
import sys
import time
from collections import defaultdict
from pathlib import Path

import numpy as np
import pyarrow.parquet as pq

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT / "solver" / "13-kissat-rs" / "tools" / "rl"))
sys.path.insert(0, str(ROOT / "solver" / "13-kissat-rs" / "tools"))
from rl_split import assert_training_only, load_split  # noqa: E402

NORM_OUT = ROOT / "benchmarks" / "rl" / "obs_norm_2025.json"
STATIC = ROOT / "benchmarks" / "rl" / "static_features.tsv"
CRITIC_COLS = ("compression_ratio", "modularity", "treewidth_ub", "communities")


def failed_runs(run: Path) -> set[str]:
    """run_ids to leave out: flagged by the collector (premature UNKNOWN, crash,
    contradiction) or with vectors still in the logged clock (a cut log)."""
    names = pq.read_schema(run / "dataset" / "runs.parquet").names
    cols = ["run_id", "failed"] + (["obs_k_logged"] if "obs_k_logged" in names else [])
    runs = pq.read_table(run / "dataset" / "runs.parquet", columns=cols).to_pydict()
    stale = runs.get("obs_k_logged", [False] * len(runs["run_id"]))
    return {r for r, f, k in zip(runs["run_id"], runs["failed"], stale) if f or k}


# ---------------------------------------------------------------------------
# norm
# ---------------------------------------------------------------------------

def cmd_norm(args) -> int:
    from policy_net import fixture, layout_hash, write_net

    run = Path(args.run_dir).resolve()
    split = load_split()
    files = sorted((run / "dataset" / "rows").glob("*.parquet"))
    failed = failed_runs(run)
    names = None
    s1 = s2 = None
    n = 0
    cells = 0
    t0 = time.time()
    for f in files:
        schema = pq.read_schema(f).names
        obs = [c for c in schema if c.startswith("obs_") and c != "obs_epoch"]
        if names is None:
            names = obs
        elif obs != names:
            raise SystemExit(f"{f}: observation layout differs")
        t = pq.read_table(f, columns=obs + ["boundary", "stem", "is_child", "run_id"])
        if t.num_rows == 0:
            continue                      # a header-only log (killed before its first row)
        stem = t.column("stem")[0].as_py()
        if split.get(stem) != "train" or t.column("run_id")[0].as_py() in failed:
            continue
        assert_training_only([stem])
        d = t.to_pydict()
        keep = np.array(d["boundary"], bool) & ~np.array(d["is_child"], bool)
        X = np.stack([np.array(d[c], dtype=np.float64) for c in obs], axis=1)[keep]
        if s1 is None:
            s1 = np.zeros(X.shape[1])
            s2 = np.zeros(X.shape[1])
        s1 += X.sum(axis=0)
        s2 += (X ** 2).sum(axis=0)
        n += X.shape[0]
        cells += 1
    if not n:
        raise SystemExit("no training rows")
    mean = s1 / n
    var = np.maximum(s2 / n - mean ** 2, 0.0)
    std = np.sqrt(var)
    const = std < 1e-12
    std_safe = np.where(const, 1.0, std)
    entries = [c[len("obs_"):] for c in names]
    out = {"run": str(run), "rows": int(n), "cells": cells, "names": entries,
           "layout_hash": f"{layout_hash(entries):016x}", "mean": mean.tolist(), "std": std_safe.tolist(),
           "constant_entries": [e for e, c in zip(entries, const) if c]}
    Path(args.out).write_text(json.dumps(out) + "\n")
    print(f"{args.out}: {len(entries)} entries from {n} boundary rows of {cells} training cells "
          f"({time.time() - t0:.0f}s); {int(const.sum())} constant entries (std set to 1): "
          f"{out['constant_entries'][:8]}{' ...' if const.sum() > 8 else ''}")
    if args.net:
        m, s, w1, b1, w2, b2, heads = fixture(entries, seed=1, hidden=(32, 16), stock_bias=1000.0)
        write_net(args.net, entries, [float(x) for x in mean], [float(x) for x in std_safe], w1, b1, w2, b2, heads)
        print(f"wrote {args.net}: a stock-biased fixture net carrying the fitted normalization")
    return 0


# ---------------------------------------------------------------------------
# predict
# ---------------------------------------------------------------------------

def ridge(X: np.ndarray, y: np.ndarray, lam: float) -> np.ndarray:
    d = X.shape[1]
    return np.linalg.solve(X.T @ X + lam * np.eye(d), X.T @ y)


def cv_lambda(X: np.ndarray, y: np.ndarray, lams, folds: int = 5, seed: int = 0) -> float:
    rng = np.random.default_rng(seed)
    idx = rng.permutation(len(y))
    best, best_err = lams[0], float("inf")
    for lam in lams:
        err = 0.0
        for k in range(folds):
            test = idx[k::folds]
            train = np.setdiff1d(idx, test)
            w = ridge(X[train], y[train], lam)
            err += float(np.sum((X[test] @ w - y[test]) ** 2))
        if err < best_err:
            best, best_err = lam, err
    return best


def logistic(X: np.ndarray, y: np.ndarray, lam: float, iters: int = 50) -> np.ndarray:
    w = np.zeros(X.shape[1])
    for _ in range(iters):
        p = 1.0 / (1.0 + np.exp(-(X @ w)))
        g = X.T @ (p - y) + lam * w
        H = (X * (p * (1 - p))[:, None]).T @ X + lam * np.eye(X.shape[1])
        step = np.linalg.solve(H, g)
        w -= step
        if np.max(np.abs(step)) < 1e-8:
            break
    return w


def cmd_predict(args) -> int:
    run = Path(args.run_dir).resolve()
    split = load_split()
    runs = pq.read_table(run / "dataset" / "runs.parquet").to_pydict()
    n = len(runs["stem"])
    static_cols = sorted(k for k in runs if k.startswith("s_"))
    critic: dict[str, dict] = {}
    if Path(args.static).is_file():
        with open(args.static, newline="") as f:
            for r in csv.DictReader(f, dialect="excel-tab"):
                if r["suite"] == "sat-comp-2025":
                    critic[r["stem"]] = r
    feat_names = static_cols + [f"c_{c}" for c in CRITIC_COLS] + ["c_log_vars", "c_log_clauses"]
    rows, stems, y_time, solved, wall = [], [], [], [], []
    for i in range(n):
        if runs["is_child"][i] or runs["tag"][i] != "stock" or runs.get("failed", [False] * n)[i]:
            continue
        stem = runs["stem"][i]
        cr = critic.get(stem, {})
        v = [float(runs[c][i]) if runs[c][i] is not None else float("nan") for c in static_cols]
        for c in CRITIC_COLS:
            x = cr.get(c, "")
            v.append(float(x) if x not in ("", None) else float("nan"))
        v.append(math.log10(float(cr["p_vars"])) if cr.get("p_vars") else float("nan"))
        v.append(math.log10(float(cr["p_clauses"])) if cr.get("p_clauses") else float("nan"))
        rows.append(v)
        stems.append(stem)
        solved.append(bool(runs["solved"][i]))
        wall.append(float(runs["wall_ns_end"][i]) / 1e9)
    X = np.array(rows, dtype=np.float64)
    stems = np.array(stems)
    solved = np.array(solved)
    wall = np.array(wall)
    # feature hygiene: drop all-nan / constant columns, fill nan with the training mean, log1p the counts
    is_train = np.array([split.get(s) == "train" for s in stems])
    is_val = np.array([split.get(s) == "val" for s in stems])
    assert_training_only(stems[is_train])
    keep = []
    for j in range(X.shape[1]):
        col = X[is_train, j]
        if np.all(np.isnan(col)) or np.nanstd(col) < 1e-12:
            continue
        keep.append(j)
    X = X[:, keep]
    names = [feat_names[j] for j in keep]
    big = np.nanmax(np.abs(X[is_train]), axis=0) > 1000
    X[:, big] = np.sign(X[:, big]) * np.log1p(np.abs(X[:, big]))
    mu = np.nanmean(X[is_train], axis=0)
    X = np.where(np.isnan(X), mu, X)
    sd = X[is_train].std(axis=0)
    sd[sd < 1e-12] = 1.0
    Z = (X - mu) / sd
    Z = np.concatenate([Z, np.ones((len(Z), 1))], axis=1)
    fam_of = dict(zip(runs["stem"], runs["family"]))
    n_actor = sum(1 for k in keep if k < len(static_cols))
    print(f"{int(is_train.sum())} training cells, {int(is_val.sum())} validation cells, {len(names)} features kept "
          f"({n_actor} of {len(static_cols)} actor-tier from the footers, {len(names) - n_actor} critic-tier; "
          f"constant and all-missing columns dropped)")

    # a. log10 solve time on solved cells
    lams = [0.01, 0.1, 1.0, 3.0, 10.0, 30.0, 100.0, 300.0, 1000.0]
    tr = is_train & solved
    va = is_val & solved
    y = np.log10(np.maximum(wall, 1e-3))
    lam = cv_lambda(Z[tr], y[tr], lams)
    w = ridge(Z[tr], y[tr], lam)
    pred = Z @ w
    base = y[tr].mean()
    rmse = lambda m: float(np.sqrt(np.mean((pred[m] - y[m]) ** 2)))
    rmse0 = lambda m: float(np.sqrt(np.mean((base - y[m]) ** 2)))
    print(f"a. log10(stock time) ridge (lambda {lam} by 5-fold CV on train): "
          f"RMSE train {rmse(tr):.3f} v constant {rmse0(tr):.3f}; val {rmse(va):.3f} v constant {rmse0(va):.3f} "
          f"(1.0 = one decade); val R2 {1 - rmse(va) ** 2 / rmse0(va) ** 2:.3f}")
    per_fam = defaultdict(list)
    for i in np.nonzero(va)[0]:
        per_fam[fam_of[stems[i]]].append(abs(pred[i] - y[i]))
    worst = sorted(per_fam.items(), key=lambda kv: -float(np.mean(kv[1])))
    print("   validation |error| in decades per family (worst first): "
          + ", ".join(f"{f} {np.mean(v):.2f} (n={len(v)})" for f, v in worst[:10]))
    top = np.argsort(-np.abs(w[:-1]))[:10]
    print("   largest |weights|: " + ", ".join(f"{names[j]} {w[j]:+.2f}" for j in top))

    # b. solved-within-the-wall classifier
    lam_c = 10.0
    wc = logistic(Z[is_train], solved[is_train].astype(float), lam_c)
    p = 1.0 / (1.0 + np.exp(-(Z @ wc)))
    acc = lambda m: float(np.mean((p[m] > 0.5) == solved[m]))
    base_c = lambda m: float(max(np.mean(solved[m]), 1 - np.mean(solved[m])))
    print(f"b. solves-within-1800 s logistic (lambda {lam_c}): accuracy train {acc(is_train):.3f} v majority {base_c(is_train):.3f}; "
          f"val {acc(is_val):.3f} v majority {base_c(is_val):.3f}")
    out = {"run": str(run), "features": names, "ridge_lambda": lam, "rmse_train": rmse(tr), "rmse_val": rmse(va),
           "rmse_const_val": rmse0(va), "val_r2": 1 - rmse(va) ** 2 / rmse0(va) ** 2,
           "per_family_val_abs_err": {f: float(np.mean(v)) for f, v in per_fam.items()},
           "classifier_acc_train": acc(is_train), "classifier_acc_val": acc(is_val),
           "classifier_majority_val": base_c(is_val)}
    Path(args.out).write_text(json.dumps(out, indent=1) + "\n")
    print(f"wrote {args.out}")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    n = sub.add_parser("norm")
    n.add_argument("run_dir")
    n.add_argument("--out", default=str(NORM_OUT))
    n.add_argument("--net", default="", help="also write a fixture weights file carrying the normalization")
    n.set_defaults(fn=cmd_norm)
    p = sub.add_parser("predict")
    p.add_argument("run_dir")
    p.add_argument("--static", default=str(STATIC))
    p.add_argument("--out", default=str(ROOT / "benchmarks" / "rl" / "runtime_predictor.json"))
    p.set_defaults(fn=cmd_predict)
    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
