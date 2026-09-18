#!/usr/bin/env python3
"""rl_fit_work.py — fit k_res and the wall weight of every work kind.

Plan: plan/rl-scheduler-solver13-plan.md §3.4 (the work clock), §4 (the
dense cost), §14 item 2. Bead SAT-playground-p9m.7.5 (step B.5).

The work clock is W = ticks + k_res x eliminate_resolutions. `ticks` is
kissat's all-propagation counter (search, probing, backbone, vivify,
transitive, factor, substitute and eliminate's dense propagation all add
to it); eliminate's resolutions, kitten's ticks, the walker's steps and
forward-subsumption steps have no tick equivalent. k_res (K_RES in
solver/13-kissat-rs/src/statistics.rs: 7 since this fit, a provisional
11 before it) is the wall cost of one resolution measured in ticks. This
script measures it from the stock traces: per observation epoch, the wall
spent is regressed on the work of each kind spent in that epoch.

Two fits, both nonnegative least squares without an intercept on the
training split's rows (validation rows are the check):

  1. aggregate: d_wall ~ ticks, eliminate_resolutions, kitten_ticks,
     walk_steps, forward_steps. k_res = w_res / w_ticks. This is the
     number the solver constant, the TSV tooling and SAT_LIMIT_TICKS use.
  2. per kind: the same with `ticks` split into search, probing, backbone,
     transitive, factor, substitute, vivify, dense and the rest, which
     shows whether a tick costs the same wall in every pass. These are
     the reward's dense-cost weights (ns per unit).

Rows are observation epochs of the stock trace; the epoch's wall includes
the row's own logging, which A.11 measured as nil. Rows whose wall is a
clear outlier for their work (the top --trim fraction of |residual|, OS
noise such as page faults at a resize) are dropped once and the fit is
repeated. Per-family residuals say where the linear model is worst.

    ~/.cache/sat13-rl/venv/bin/python tools/rl_fit_work.py log/rl-stock2025-<ts> [--out benchmarks/rl/work_fit.json]
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from collections import defaultdict
from pathlib import Path

import numpy as np
import pyarrow.parquet as pq

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from rl_split import load_split  # noqa: E402

AGG = ("ticks", "eliminate_resolutions", "kitten_ticks", "walk_steps", "forward_steps")
SUB = ("search_ticks", "probing_ticks", "backbone_ticks", "transitive_ticks", "factor_ticks",
       "substitute_ticks", "vivify_ticks", "dense_ticks")
KIND = SUB + ("other_ticks", "eliminate_resolutions", "kitten_ticks", "walk_steps", "forward_steps")


def nnls(A: np.ndarray, b: np.ndarray, iters: int = 5000) -> np.ndarray:
    """Nonnegative least squares by projected gradient on the normal
    equations (small problems: a handful of columns, up to a few million rows)."""
    AtA = A.T @ A
    Atb = A.T @ b
    scale = np.sqrt(np.diag(AtA)) + 1e-12
    AtA_s = AtA / scale[:, None] / scale[None, :]
    Atb_s = Atb / scale
    L = np.linalg.eigvalsh(AtA_s).max() + 1e-12
    x = np.zeros(A.shape[1])
    for _ in range(iters):
        g = AtA_s @ x - Atb_s
        x_new = np.maximum(x - g / L, 0.0)
        if np.max(np.abs(x_new - x)) < 1e-12:
            x = x_new
            break
        x = x_new
    return x / scale


def fit(X: np.ndarray, y: np.ndarray, trim: float) -> tuple[np.ndarray, np.ndarray, float]:
    w = nnls(X, y)
    res = y - X @ w
    if trim > 0:
        keep = np.abs(res) <= np.quantile(np.abs(res), 1.0 - trim)
        w = nnls(X[keep], y[keep])
        res = y - X @ w
    ss_res = float(np.sum(res ** 2))
    ss_tot = float(np.sum((y - y.mean()) ** 2)) or 1.0
    return w, res, 1.0 - ss_res / ss_tot


def load_rows(run_dir: Path, split: dict[str, str]) -> tuple[dict[str, np.ndarray], np.ndarray, np.ndarray]:
    """Deltas of every kind, the wall delta, and the stem/family per row."""
    runs = pq.read_table(run_dir / "dataset" / "runs.parquet").to_pydict()
    fam_of = {s: f for s, f in zip(runs["stem"], runs["family"])}
    failed = {r for r, f in zip(runs["run_id"], runs.get("failed", [False] * len(runs["run_id"]))) if f}
    cols = [f"d_{k}" for k in AGG + SUB] + ["d_wall_ns", "boundary", "row"]
    parts = defaultdict(list)
    stems, fams = [], []
    for f in sorted((run_dir / "dataset" / "rows").glob("*.parquet")):
        t = pq.read_table(f, columns=[c for c in cols if c in pq.read_schema(f).names] + ["stem", "run_id"])
        if t.num_rows == 0 or t.column("run_id")[0].as_py() in failed:
            continue                      # a flagged run is not data
        d = {c: t.column(c).to_numpy() for c in t.column_names if c not in ("stem", "run_id")}
        stem = t.column("stem")[0].as_py()
        # the first row has no predecessor (zero deltas) and the final row is
        # a partial epoch: keep the boundary rows after the first
        keep = d["boundary"] & (d["row"] > 0)
        for c in cols:
            if c in d and c not in ("boundary", "row"):
                parts[c].append(d[c][keep])
        n = int(keep.sum())
        stems += [stem] * n
        fams += [fam_of.get(stem, "")] * n
    out = {c: np.concatenate(v) for c, v in parts.items()}
    return out, np.array(stems), np.array(fams)


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run_dir")
    ap.add_argument("--out", default=str(ROOT / "benchmarks" / "rl" / "work_fit.json"))
    ap.add_argument("--trim", type=float, default=0.01)
    ap.add_argument("--all-cells", action="store_true", help="fit on every cell, not only the training split")
    args = ap.parse_args(argv)
    run_dir = Path(args.run_dir).resolve()
    split = load_split()
    t0 = time.time()
    d, stems, fams = load_rows(run_dir, split)
    y = d["d_wall_ns"].astype(np.float64)
    in_train = np.array([split.get(s) == "train" for s in stems]) if not args.all_cells else np.ones(len(stems), bool)
    in_val = np.array([split.get(s) == "val" for s in stems])       # never the 8 shared (holdout) cells
    print(f"{len(y)} epoch rows from {len(set(stems))} cells; {int(in_train.sum())} rows on "
          f"{len(set(stems[in_train]))} fitting cells ({time.time() - t0:.0f}s to load)")
    result = {"run": str(run_dir), "rows": int(len(y)), "fit_rows": int(in_train.sum()),
              "fit_cells": len(set(stems[in_train])), "trim": args.trim, "unit": "ns per unit of work"}

    # 1. aggregate fit -> k_res
    Xa = np.stack([d[f"d_{k}"].astype(np.float64) for k in AGG], axis=1)
    w, res, r2 = fit(Xa[in_train], y[in_train], args.trim)
    k_res = w[1] / w[0] if w[0] > 0 else float("nan")
    result["aggregate"] = {"weights_ns": dict(zip(AGG, map(float, w))), "r2_train": r2, "k_res": float(k_res)}
    res_all = y - Xa @ w
    print(f"aggregate fit (train R2 {r2:.4f}): " + ", ".join(f"{k} {v:.3f} ns" for k, v in zip(AGG, w)))
    print(f"  k_res = w_res / w_ticks = {k_res:.2f}  (solver constant K_RES = 7 since step B.5, 11 before)")
    for name, mask in (("train", in_train), ("val", in_val)):
        if mask.any():
            r = res_all[mask]
            print(f"  {name}: rows {int(mask.sum())}, median |resid| {np.median(np.abs(r)) / 1e6:.2f} ms, "
                  f"mean wall per row {y[mask].mean() / 1e6:.1f} ms, "
                  f"resid/wall {np.sum(np.abs(r)) / np.sum(y[mask]):.3f}")
            result["aggregate"][f"resid_over_wall_{name}"] = float(np.sum(np.abs(r)) / np.sum(y[mask]))

    # 2. per-kind fit -> dense-cost weights
    sub = np.stack([d[f"d_{k}"].astype(np.float64) for k in SUB], axis=1)
    other = np.maximum(d["d_ticks"].astype(np.float64) - sub.sum(axis=1), 0.0)
    Xk = np.concatenate([sub, other[:, None]] + [d[f"d_{k}"].astype(np.float64)[:, None]
                                                  for k in ("eliminate_resolutions", "kitten_ticks", "walk_steps", "forward_steps")], axis=1)
    wk, resk, r2k = fit(Xk[in_train], y[in_train], args.trim)
    result["per_kind"] = {"weights_ns": dict(zip(KIND, map(float, wk))), "r2_train": r2k,
                          "share_of_ticks_in_subkinds": float(sub.sum() / max(d["d_ticks"].sum(), 1))}
    print(f"per-kind fit (train R2 {r2k:.4f}; sub-kinds cover {100 * sub.sum() / max(d['d_ticks'].sum(), 1):.1f}% of ticks):")
    for k, v in zip(KIND, wk):
        tot = float(Xk[:, KIND.index(k)].sum())
        print(f"  {k:<22} {v:8.3f} ns/unit   total units {tot:.3e}   wall share {100 * v * tot / max(y.sum(), 1):5.1f}%")

    # 3. residuals per family (aggregate fit), on the training and validation rows only
    per_fam = defaultdict(list)
    for f, r, wall, ok in zip(fams, res_all, y, in_train | in_val):
        if ok:
            per_fam[f].append((r, wall))
    rows = []
    for f, v in per_fam.items():
        r = np.array([x[0] for x in v])
        wall = np.array([x[1] for x in v])
        rows.append((f, len(v), float(np.sum(np.abs(r)) / max(np.sum(wall), 1)), float(np.median(r) / 1e6)))
    rows.sort(key=lambda x: -x[2])
    result["per_family"] = [{"family": f, "rows": n, "abs_resid_over_wall": a, "median_resid_ms": m} for f, n, a, m in rows]
    print("worst families (|resid| / wall, median resid ms, rows):")
    for f, n, a, m in rows[:12]:
        print(f"  {f:<24} {a:6.3f}  {m:8.2f}  {n}")
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    Path(args.out).write_text(json.dumps(result, indent=1) + "\n")
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
