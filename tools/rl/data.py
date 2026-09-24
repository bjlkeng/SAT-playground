"""tools/rl/data.py — decision rows of converted collector passes, for training.

Plan: plan/rl-scheduler-solver13-plan.md §5.1 (what a row carries), §6.2,
§6.4, §8 (splits). Bead SAT-playground-p9m.9.2 (step D.2).

A converted pass (tools/rl_dataset.py) holds one row table per job with the
251-entry observation vector (`obs_*` columns) on every observation-epoch
row. The policy decides only on the rows `is_decision` marks (the state a
decision was made from, D0 first), so those are the training states. This
module loads them as one float32 matrix plus a small metadata table, in
the entry order the solver uses (the normalization file's `names`), and
applies the split discipline: `load_decisions(..., split="train")` refuses
any cell outside the training split (rl_split.assert_training_only).

    from data import load_norm, load_decisions
    norm = load_norm()                                  # names, mean, std, layout hash
    X, meta = load_decisions(run_dir, "train", norm)    # X: (n, 251) float32
    meta["stable"]                                      # the mode at each decision (1 = stable)
    meta["stem"], meta["run_id"], meta["row"], meta["is_child"]

Runs the collector flagged (`failed`) and runs whose vectors are still in
the logged work clock (`obs_k_logged`) are left out, as rl_normalize does.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np
import pyarrow.parquet as pq

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT / "solver" / "13-kissat-rs" / "tools" / "rl"))
sys.path.insert(0, str(ROOT / "solver" / "13-kissat-rs" / "tools"))
from rl_split import assert_training_only, load_split  # noqa: E402
from policy_net import layout_hash  # noqa: E402

NORM = ROOT / "benchmarks" / "rl" / "obs_norm_2025.json"
META_COLUMNS = ("stem", "run_id", "row", "is_child", "stable")


class Norm:
    """The observation layout and its standardization (benchmarks/rl/obs_norm_2025.json)."""

    def __init__(self, d: dict):
        self.names: list[str] = list(d["names"])
        self.mean = np.asarray(d["mean"], dtype=np.float64)
        self.std = np.asarray(d["std"], dtype=np.float64)
        self.std = np.where(self.std > 0.0, self.std, 1.0)
        self.layout_hash = layout_hash(self.names)
        if f"{self.layout_hash:016x}" != d["layout_hash"]:
            raise ValueError(f"normalization file: layout hash {d['layout_hash']} does not match its names")

    @property
    def n_in(self) -> int:
        return len(self.names)

    def columns(self) -> list[str]:
        return [f"obs_{n}" for n in self.names]


def load_norm(path: Path = NORM) -> Norm:
    return Norm(json.loads(Path(path).read_text()))


def excluded_runs(run: Path) -> set[str]:
    """run_ids the collector flagged or whose vectors were not re-priced."""
    names = pq.read_schema(run / "dataset" / "runs.parquet").names
    cols = ["run_id", "failed"] + (["obs_k_logged"] if "obs_k_logged" in names else [])
    runs = pq.read_table(run / "dataset" / "runs.parquet", columns=cols).to_pydict()
    stale = runs.get("obs_k_logged", [False] * len(runs["run_id"]))
    return {r for r, f, k in zip(runs["run_id"], runs["failed"], stale) if f or k}


def load_decisions(run_dir: Path | str, split: str, norm: Norm, *, children: bool = False,
                   limit_cells: int | None = None) -> tuple[np.ndarray, dict[str, np.ndarray]]:
    """The decision rows of every run of `run_dir` whose cell is in `split`
    ("train", "val" or "all"; "all" still leaves the 8 shared cells out).
    Fork children are included only with `children=True`. Returns the
    float32 observation matrix in the normalization's entry order and the
    metadata arrays of META_COLUMNS."""
    run = Path(run_dir).resolve()
    if split not in ("train", "val", "all"):
        raise ValueError(f"split {split!r}: expected train, val or all")
    split_of = load_split()
    out_rows = set()
    if split == "train":
        out_rows = {s for s, k in split_of.items() if k != "train"}
    elif split == "val":
        out_rows = {s for s, k in split_of.items() if k != "val"}
    else:
        out_rows = {s for s, k in split_of.items() if k == "shared"}
    excluded = excluded_runs(run)
    cols = norm.columns()
    X_parts: list[np.ndarray] = []
    meta_parts: dict[str, list] = {k: [] for k in META_COLUMNS}
    cells = 0
    for f in sorted((run / "dataset" / "rows").glob("*.parquet")):
        schema = pq.read_schema(f).names
        missing = [c for c in cols if c not in schema]
        if missing:
            raise ValueError(f"{f}: {len(missing)} observation columns missing (layout differs), e.g. {missing[:3]}")
        t = pq.read_table(f, columns=cols + ["is_decision"] + list(META_COLUMNS))
        if t.num_rows == 0:
            continue
        stem = t.column("stem")[0].as_py()
        if stem in out_rows or split_of.get(stem) is None:
            continue
        if split == "train":
            assert_training_only([stem])
        d = t.to_pydict()
        keep = np.asarray(d["is_decision"], dtype=bool)
        keep &= ~np.isin(np.asarray(d["run_id"]), list(excluded))
        if not children:
            keep &= ~np.asarray(d["is_child"], dtype=bool)
        if not keep.any():
            continue
        X = np.stack([np.asarray(d[c], dtype=np.float32) for c in cols], axis=1)[keep]
        X_parts.append(X)
        for k in META_COLUMNS:
            v = np.asarray(d[k])
            meta_parts[k].append(v[keep])
        cells += 1
        if limit_cells and cells >= limit_cells:
            break
    if not X_parts:
        raise ValueError(f"{run}: no decision rows for split {split!r}")
    meta = {k: np.concatenate(v) for k, v in meta_parts.items()}
    meta["stable"] = meta["stable"].astype(np.float64) > 0.5
    return np.concatenate(X_parts, axis=0), meta


def standardize(norm: Norm, X: np.ndarray) -> np.ndarray:
    """(x - mean) / std in float64, as the solver's forward pass does."""
    return (X.astype(np.float64) - norm.mean) / norm.std
