#!/usr/bin/env python3
"""rl_dataset.py — policy logs of one collector pass to compact parquet tables.

Plan: plan/rl-scheduler-solver13-plan.md §5.1 (what a row carries), §7 item 7,
§12 "Log volume". Bead SAT-playground-p9m.7.8 (step B.7).

A raw policy log (solver/13-kissat-rs/src/policy_log.rs) holds 981 raw
columns per observation epoch, about 60 MB per 1800 s run; a round-0 pass
at fork scale is tens of GB. Training wants a few MB per run in memory.
This tool converts one collector pass (tools/rl_collect.py run directory)
into

    <run>/dataset/runs.parquet          one row per run (parent or child):
                                        bookkeeping, terminal outcome,
                                        censoring, budgets, the solver's
                                        static features, family
    <run>/dataset/rows/<stem>.<tag>.parquet
                                        the rows of that job's parent log
                                        and all its children's logs: keys,
                                        counters and their deltas since the
                                        previous row, the observation
                                        vector, the action in force, the
                                        action chosen at a decision row,
                                        the stock counterfactual flags, and
                                        (with --stock) the stock run's
                                        counters at the same tick-grid
                                        position
    <run>/dataset/MANIFEST.json         per-job status: what was converted
                                        from which log, with which stock
                                        pairing and k_res, so a rerun skips
                                        finished jobs

One run at a time: a log is parsed with numpy straight from its bytes,
written, and dropped. Re-running converts only jobs whose parquet is
missing, whose log changed, or whose pairing or k_res differs from the
manifest (or everything with --force).

Row semantics (checked against real logs, 2026-09-18)
  - `row` counts rows in the log; the final row is written at exit and is
    not an observation boundary (`boundary` false there).
  - The solver writes a boundary row and THEN decides, so the decision
    count rises in the next row. `is_decision` marks the row whose
    observation the decision was made from (the state), and
    `act_taken_*` on that row is the action chosen there: the `dec_*` of
    the next row (not its `act_*`, which a one-shot entry has already
    reset to 1 once its timer fired); `act_*` itself is the action in
    force while the epoch ending at the row ran. D0 is the decision at
    row 0.
  - `d_<counter>` is the counter's change since the previous row of the
    same run. A fork child's first row is the parent's state at the fork,
    which is the parent's row `parent_rows - 1` (the child's epoch counter
    is already one ahead), so that row is the child's predecessor: the
    first deltas are zero except `d_cpu_ns`, which is zero by definition
    because process CPU time restarts at fork. A child's first row carries
    the action the branch put in force, i.e. the action taken at the
    parent's decision row `parent_rows - 1`.
  - `work` is recomputed as ticks + k_res x eliminate_resolutions with one
    k_res for every log in the pass (--k-res, default 7, the step-B fit),
    whatever the binary printed; `work_logged` keeps the printed value.
    Every observation entry derived from the work clock (the log of work,
    the search, probing and eliminate fractions, the per-pass costs, their
    windowed deltas, the preprocessing work) moves with k_res, so when a
    log was written at another k every `obs_*` column is recomputed with
    the reference implementation (solver/13-kissat-rs/tools/policy_obs.py,
    which reproduces the logged vectors bit for bit at the logged k) on
    rows whose `work` and static preprocessing work are re-priced at
    --k-res; a fork child's vectors are replayed after its parent's prefix
    rows. A trace logged at k=11 then feeds a k=7 binary's policy exactly
    the values it will see. The static `s_preprocess_work` of the run
    record is re-priced too.
  - A tick horizon's observation entry is kept as logged: it is the
    fraction of the run's native budget used at that row, which depends
    on nothing after the row, whereas any re-priced budget would depend
    on the run's end and differ between a parent and its children at the
    same state. The run record's `limit_ticks_k` (the budget re-priced by
    the run's own end-of-run ratio, exact for a budget stop) and
    `budget_frac_end` are terminal, per-run quantities: never inputs.
  - A log cut before its footer cannot be rebuilt at another k (the static
    context is gone): its vectors stay as logged and the run record's
    `obs_k_logged` is true; normalization skips such runs.
  - `failed` in runs.parquet carries the collector's correctness flags
    (a premature UNKNOWN, a crash, a contradiction); such runs are not
    data and every consumer skips them.
  - `stock_*` (only with --stock <stock pass dir>) are the paired stock
    run's counters at the same position on the observation grid,
    `search_ticks // X_o`, not at the same observation count: when search
    jumps several grid boundaries in one step the solver writes one row
    and the counts drift apart. `stock_paired` says whether that grid
    position exists in the stock trace; rows at skipped positions and
    final rows stay unpaired.
  - `censored` in runs.parquet is "the run did not solve": a right-
    censored time-to-solve (plan §4), not a zero-information tie.
    `anomaly` is the collector's: a budgeted run that hit its safety wall
    cap (parent or, through its termination, child) is not a result.

    ~/.cache/sat13-rl/venv/bin/python tools/rl_dataset.py convert log/rl-stock2025-<ts>
    ~/.cache/sat13-rl/venv/bin/python tools/rl_dataset.py convert log/rl-round0-<ts> --stock log/rl-stock2025-<ts>
    ~/.cache/sat13-rl/venv/bin/python tools/rl_dataset.py selftest     # fixture log, checks the deltas

Needs numpy and pyarrow (tools/rl/requirements.txt; the venv above).
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import struct
import sys
import tempfile
import time
from pathlib import Path

try:
    import numpy as np
    import pyarrow as pa
    import pyarrow.parquet as pq
except ImportError as e:  # pragma: no cover
    raise SystemExit(f"{e}: run with the RL venv, e.g. ~/.cache/sat13-rl/venv/bin/python (tools/rl/requirements.txt)")

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "solver" / "13-kissat-rs" / "tools"))
from policy_obs import observe_log  # noqa: E402  (the reference recomputation of observe())

MAGIC = b"SAT13POLICYLOG 1"
SENTINEL = (1 << 64) - 1
DEFAULT_FAMILIES = ROOT / "benchmarks" / "rl" / "families.tsv"
DEFAULT_K_RES = 7

# Cumulative counters copied per row and differenced (the work kinds of the
# reward's dense cost, plan §4, plus the headline search counters and the
# clocks). `sweep` has no tick counter of its own: its cost is kitten ticks.
# `work` is recomputed (see the module docstring) and differenced too.
WORK_KINDS = ("search_ticks", "probing_ticks", "backbone_ticks", "transitive_ticks", "factor_ticks",
              "substitute_ticks", "vivify_ticks", "dense_ticks", "kitten_ticks", "walk_steps", "flipped",
              "forward_steps", "eliminate_resolutions", "ticks")
COUNTERS = WORK_KINDS + ("wall_ns", "cpu_ns", "conflicts", "decisions", "propagations", "restarts",
                         "reductions", "rephased", "probings", "eliminations", "switched", "sweep",
                         "vivifications", "walks", "units")
# Raw state copied per row, not differenced.
STATE = ("vars", "active", "clauses_irredundant", "clauses_redundant", "clauses_binary", "level", "trail",
         "unassigned", "stable", "horizon", "horizon_valid", "bound_eliminate_max_completed",
         "arena_garbage", "tier1_focused", "tier1_stable", "tier2_focused", "tier2_stable")
STOCK_PAIR = ("conflicts", "work", "wall_ns", "active", "clauses_irredundant", "clauses_redundant")
PREFIXES = ("obs_", "act_", "dec_", "net_")
TM_SUFFIXES = ("_fires", "_stock_would_fire")


# ---------------------------------------------------------------------------
# reading a log with numpy
# ---------------------------------------------------------------------------

class Log:
    """A parsed policy log: header dict, footer dict (or None), and one
    numpy array per column (u64 counters as uint64, f64 columns as float64)."""

    def __init__(self, path: Path):
        self.path = path
        data = path.read_bytes()
        nl1 = data.index(b"\n")
        if data[:nl1] != MAGIC:
            raise ValueError(f"{path}: not a policy log")
        nl2 = data.index(b"\n", nl1 + 1)
        self.header = json.loads(data[nl1 + 1:nl2])
        cols = self.header["columns"]
        kinds = self.header["kinds"]
        ncol = len(cols)
        body = data[nl2 + 1:]
        nrows_max = len(body) // (8 * ncol)
        arr = np.frombuffer(body[:nrows_max * 8 * ncol], dtype="<u8").reshape(nrows_max, ncol)
        self.footer = None
        end = nrows_max
        sent = np.nonzero(arr[:, 0] == SENTINEL)[0]
        if len(sent):
            end = int(sent[0])
            rest = body[(end + 1) * 8 * ncol:].strip()
            if rest:
                try:
                    self.footer = json.loads(rest)
                except json.JSONDecodeError:
                    self.footer = None
        self.n = end
        self.columns = cols
        self.index = {c: i for i, c in enumerate(cols)}
        self.kinds = kinds
        self._arr = arr[:end]

    def col(self, name: str) -> np.ndarray:
        i = self.index[name]
        raw = self._arr[:, i]
        if self.kinds[i] == "f":
            return raw.view("<f8")
        return raw

    def has(self, name: str) -> bool:
        return name in self.index

    @property
    def obs_ticks(self) -> int:
        return int((self.header.get("policy") or {}).get("obs_ticks") or 0)

    def work(self, k_res: int) -> np.ndarray:
        """W recomputed at one k_res: ticks + k_res x eliminate_resolutions."""
        return self.col("ticks").astype(np.float64) + float(k_res) * self.col("eliminate_resolutions").astype(np.float64)

    def grid(self) -> np.ndarray:
        """Position of each row on the observation grid: search_ticks // X_o (-1 when unknown)."""
        xo = self.obs_ticks
        if xo <= 0 or not self.has("search_ticks"):
            return np.full(self.n, -1, dtype=np.int64)
        return (self.col("search_ticks") // np.uint64(xo)).astype(np.int64)


def flatten_static(static: dict) -> dict[str, float]:
    """The footer's static block ({groups: {group: {name: value}}}) as s_<group>_<name>."""
    out: dict[str, float] = {}
    groups = (static or {}).get("groups") or {}
    for g, feats in groups.items():
        if isinstance(feats, dict):
            for k, v in feats.items():
                out[f"s_{g}_{k}"] = float(v) if isinstance(v, (int, float)) else float("nan")
    return out


# ---------------------------------------------------------------------------
# tables
# ---------------------------------------------------------------------------

def row_state(log: Log, i: int, k_res: int) -> dict[str, float]:
    """The differenced counters of row `i`, as a predecessor for another row."""
    out = {"policy_decisions": float(log.col("policy_decisions")[i]), "work": float(log.work(k_res)[i])}
    for c in COUNTERS:
        if log.has(c):
            out[c] = float(log.col(c)[i])
    return out


def row_table(log: Log, keys: dict, prev_row: dict[str, float] | None, stock: "Log | None",
              k_res: int, prefix: list[dict] | None = None) -> dict[str, np.ndarray]:
    """All row columns of one log as numpy arrays (a dict), given the run's
    key columns, the predecessor of the first row (a parent's fork-state
    row for a child) and the paired stock log."""
    n = log.n
    t: dict[str, np.ndarray] = {}
    for k, v in keys.items():
        t[k] = np.full(n, v, dtype=object) if isinstance(v, str) else np.full(n, v)
    t["row"] = log.col("row").astype(np.int64)
    t["obs_epoch"] = log.col("obs_epoch").astype(np.int64)
    t["grid"] = log.grid()
    boundary = log.col("row_boundary").astype(bool)
    t["boundary"] = boundary
    pd_ = log.col("policy_decisions").astype(np.int64)
    t["policy_decisions"] = pd_
    # the decision made from this row's observation shows up in the next row
    is_dec = np.zeros(n, dtype=bool)
    if n > 1:
        is_dec[:-1] = pd_[1:] > pd_[:-1]
    t["is_decision"] = is_dec
    work = log.work(k_res)
    counters: dict[str, np.ndarray] = {"work": work}
    for c in COUNTERS:
        if log.has(c):
            counters[c] = log.col(c).astype(np.float64)
    t["work_logged"] = log.col("work").astype(np.float64)
    for c, cur in counters.items():
        t[c] = cur
        if n == 0:
            t[f"d_{c}"] = cur
            continue
        if prev_row is not None and c in prev_row:
            prev0 = prev_row[c]
        else:
            prev0 = cur[0]
        t[f"d_{c}"] = cur - np.concatenate([[prev0], cur[:-1]])
    for c in STATE:
        if log.has(c):
            t[c] = log.col(c).astype(np.float64)
    act_cols = []
    for c in log.columns:
        if c.startswith(PREFIXES) and c != "obs_epoch":
            t[c] = log.col(c).astype(np.float32 if log.kinds[log.index[c]] == "f" else np.int64)
            if c.startswith("act_"):
                act_cols.append(c)
        elif c.startswith("tm_") and c.endswith(TM_SUFFIXES):
            t[c] = log.col(c).astype(np.int64)
    if log.header.get("k_res") != k_res and n:
        has_static = bool(((log.footer or {}).get("static") or {}).get("groups"))
        if log.has("avg_f_fast_glue") and log.has("tm_probe_last_fire") and has_static:
            # a real log with its footer: every work-derived entry is rebuilt
            # at k_res (the static block feeds the static entries and the
            # active-variable fraction)
            for c, v in recompute_obs(log, k_res, prefix).items():
                if c in t:
                    t[c] = v
        elif log.has("avg_f_fast_glue"):
            # a log cut before its footer cannot be rebuilt (the static context
            # is gone); its vectors stay as logged, in the logged clock, and
            # the run record says so (`obs_k_logged`) so normalization and
            # training leave it out
            pass
        elif "obs_g_eliminate_frac" in t and log.has("eliminate_resolutions"):
            # a log without the raw columns the reference needs (fixtures):
            # only the direct entry can be re-priced
            elim = log.col("eliminate_resolutions").astype(np.float64)
            frac = np.where(work > 0, float(k_res) * elim / np.where(work > 0, work, 1.0), 0.0)
            t["obs_g_eliminate_frac"] = frac.astype(np.float32)
    # the action chosen at a decision row: the next row's DECISION (`dec_*`).
    # Not its action in force: a one-shot entry (m = 0) whose timer fired
    # before the next row has already been reset to 1 in `act_*`, while
    # `dec_*` keeps what was chosen.
    for c in act_cols:
        src = "dec_" + c[len("act_"):]
        if src not in t:
            src = c
        taken = np.full(n, np.nan, dtype=np.float64)
        if n > 1:
            nxt = t[src][1:].astype(np.float64)
            taken[:-1] = np.where(is_dec[:-1], nxt, np.nan)
        t["act_taken_" + c[len("act_"):]] = taken.astype(np.float32)
    # the paired stock run at the same grid position
    if stock is not None:
        sg = stock.grid()
        sb = stock.col("row_boundary").astype(bool)
        pos_to_row: dict[int, int] = {}
        for i in np.nonzero(sb)[0]:
            pos_to_row.setdefault(int(sg[i]), int(i))   # the first boundary row at a position
        mine = t["grid"]
        idx = np.array([pos_to_row.get(int(g), -1) if (b and g >= 0) else -1 for g, b in zip(mine, boundary)],
                       dtype=np.int64)
        paired = idx >= 0
        t["stock_paired"] = paired
        safe = np.where(paired, idx, 0)
        stock_counters = {"work": stock.work(k_res)}
        for c in STOCK_PAIR:
            if c == "work":
                continue
            if stock.has(c):
                stock_counters[c] = stock.col(c).astype(np.float64)
        for c, v in stock_counters.items():
            vals = v[safe] if stock.n else np.zeros(n)
            t[f"stock_{c}"] = np.where(paired, vals, np.nan)
    return t


def row_dicts(log: Log, k_res: int, upto: int | None = None) -> list[dict]:
    """The log's rows as dicts (column name -> value) with `work` re-priced at k_res."""
    n = log.n if upto is None else min(upto, log.n)
    cols = [log.col(c)[:n].tolist() for c in log.columns]
    wi = log.index["work"]
    ti, ei = log.index["ticks"], log.index["eliminate_resolutions"]
    out = []
    for i in range(n):
        d = {c: cols[j][i] for j, c in enumerate(log.columns)}
        d["work"] = cols[ti][i] + k_res * cols[ei][i]
        out.append(d)
    _ = wi
    return out


def repriced_footer(log: Log, k_res: int) -> dict:
    """The footer with the static preprocessing work re-priced at k_res (the
    `s_pre_work` observation entry reads it)."""
    ft = json.loads(json.dumps(log.footer or {}))
    pre = ((ft.get("static") or {}).get("groups") or {}).get("preprocess")
    if isinstance(pre, dict) and "ticks" in pre and "eliminate_resolutions" in pre:
        pre["work"] = pre["ticks"] + k_res * pre["eliminate_resolutions"]
    return ft


def budget_ratio(log: Log, k_res: int) -> float:
    """work at k_res over the logged work at the end of the run: the factor that
    re-prices a budget given in the logged clock (exact for a run that stopped
    on its budget, proportional otherwise)."""
    if not log.n:
        return 1.0
    logged = float(log.col("work")[-1])
    return float(log.work(k_res)[-1]) / logged if logged > 0 else 1.0


def recompute_obs(log: Log, k_res: int, prefix: list[dict] | None) -> dict[str, np.ndarray]:
    """All `obs_*` columns recomputed at k_res with the reference implementation."""
    header = dict(log.header)
    header["k_res"] = k_res
    header["policy"] = dict(header.get("policy") or {}, k_res=k_res)
    rows = row_dicts(log, k_res)
    names, vectors = observe_log(header, [[r[c] for c in log.columns] for r in rows], repriced_footer(log, k_res),
                                 prefix_rows=prefix)
    out = {}
    arr = np.array(vectors, dtype=np.float32) if vectors else np.zeros((0, len(names or [])), dtype=np.float32)
    for j, name in enumerate(names or []):
        out["obs_" + name] = arr[:, j]
    # A tick horizon is work / budget with the budget in the run's native
    # clock. Its logged value is the fraction of that budget actually used at
    # each row, which depends on nothing after the row; re-pricing the budget
    # would need the run's end (or an assumed tick/resolution mix) and would
    # give a parent and its children different budgets at the same state. So
    # the logged horizon entries are kept.
    if (log.header.get("policy") or {}).get("horizon") not in (None, "wall", "none"):
        for c in ("obs_g_horizon", "obs_g_horizon_valid"):
            if c in out and log.has(c):
                out[c] = log.col(c).astype(np.float32)
    return out


def to_arrow(t: dict[str, np.ndarray]) -> pa.Table:
    cols = {}
    for k, v in t.items():
        if v.dtype == object:
            cols[k] = pa.array(list(v), type=pa.string())
        else:
            cols[k] = pa.array(v)
    return pa.table(cols)


def run_record(log: Log, keys: dict, rec: dict | None, child: dict | None, family: str, pass_name: str,
               k_res: int) -> dict:
    ft = log.footer or {}
    pol = log.header.get("policy", {})
    result = ft.get("result")
    if result is None:
        result = "CUT"                      # no footer: the log was cut off
    solved = result in ("SATISFIABLE", "UNSATISFIABLE")
    n = log.n

    def end_of(name: str) -> float:
        """A terminal value: the footer's if it has one, else the last row's, else nan."""
        if name in ft and ft[name] is not None:
            return float(ft[name])
        if log.has(name) and n:
            return float(log.col(name)[-1])
        return float("nan")

    src = child if child is not None else (rec or {})
    has_static = bool((ft.get("static") or {}).get("groups"))
    out = {
        **keys,
        "pass": pass_name,
        "family": family,
        "log": str(log.path),
        "cnf": log.header.get("cnf"),
        "result": result,
        "solved": solved,
        "censored": not solved,
        "reason": ft.get("reason"),
        "exit": ft.get("exit"),
        "footer": log.footer is not None,
        "rows": n,
        "boundary_rows": int(log.col("row_boundary").sum()) if n else 0,
        "decisions_n": int(log.col("policy_decisions")[-1]) if n else 0,
        "work_end": float(log.work(k_res)[-1]) if n else float("nan"),
        "work_logged_end": end_of("work"),
        "ticks_end": float(log.col("ticks")[-1]) if n and log.has("ticks") else float("nan"),
        "eliminate_resolutions_end": float(log.col("eliminate_resolutions")[-1]) if n and log.has("eliminate_resolutions") else float("nan"),
        "conflicts_end": end_of("conflicts"),
        "wall_ns_end": end_of("wall_ns"),
        "cpu_ns_end": end_of("cpu_ns"),
        "peak_rss_mb": (ft["peak_rss_bytes"] / 2**20) if ft.get("peak_rss_bytes") else float("nan"),
        "static_computed": bool((ft.get("static") or {}).get("computed", False)),
        "policy_mode": pol.get("mode"),
        "obs_ticks": pol.get("obs_ticks"),
        "dec_ticks": pol.get("dec_ticks"),
        "policy_seed": pol.get("seed"),
        "policy_temp": pol.get("temp"),
        "segment_mean": pol.get("segment_mean"),
        "horizon": pol.get("horizon"),
        "horizon_budget": pol.get("horizon_budget"),
        "k_res_logged": log.header.get("k_res"),
        "k_res": k_res,
        # true when the obs_* columns are in the logged clock, not the table's
        # (a log cut before its footer cannot be rebuilt): not for fitting
        "obs_k_logged": bool(log.header.get("k_res") != k_res and not has_static and n),
        "limit_ticks": (rec or {}).get("limit_ticks"),
        # the budget in the table's clock (the logged one re-priced by the
        # run's end-of-run ratio) and the fraction of it used at the end
        "limit_ticks_k": (float((rec or {})["limit_ticks"]) * budget_ratio(log, k_res)
                          if (rec or {}).get("limit_ticks") is not None else None),
        "wall_s": (rec or {}).get("wall_s"),
        "anomaly": src.get("anomaly"),
        "verify": src.get("verify"),
        "oracle": src.get("oracle", ""),
        "collector_result": src.get("result"),
    }
    out["budget_frac_end"] = (out["work_end"] / out["limit_ticks_k"]
                              if out.get("limit_ticks_k") and out["work_end"] == out["work_end"] else None)
    out.update(flatten_static(ft.get("static")))
    if "s_preprocess_ticks" in out and "s_preprocess_eliminate_resolutions" in out:
        out["s_preprocess_work"] = out["s_preprocess_ticks"] + k_res * out["s_preprocess_eliminate_resolutions"]
    out["premature"] = bool(src.get("premature", False))
    out["crash"] = bool(src.get("crash", False))
    out["contradiction"] = bool((rec or {}).get("contradiction", False))
    out["failed"] = out["premature"] or out["crash"] or out["contradiction"]
    return out


def sig_of(path: Path) -> list:
    """A file's identity for the cache: path, size, mtime (None when absent)."""
    try:
        st_ = path.stat()
        return [str(path), st_.st_size, int(st_.st_mtime)]
    except OSError:
        return [str(path), None, None]


def write_manifest(man_path: Path, manifest: dict) -> None:
    tmp = man_path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(manifest, indent=1, sort_keys=True) + "\n")
    os.replace(tmp, man_path)


def load_families(path: Path) -> dict[str, str]:
    fam: dict[str, str] = {}
    if not path.is_file():
        return fam
    with open(path, newline="") as f:
        for r in csv.DictReader(f, dialect="excel-tab"):
            fam[r["stem"]] = r["family"]
    return fam


def convert(run_dir: Path, stock_dir: Path | None, families: Path, force: bool, jobs_filter: set[str] | None,
            k_res: int = DEFAULT_K_RES) -> int:
    run_dir = run_dir.resolve()
    stock_key = str(stock_dir.resolve()) if stock_dir else None
    ds = run_dir / "dataset"
    (ds / "rows").mkdir(parents=True, exist_ok=True)
    man_path = ds / "MANIFEST.json"
    # the manifest is always loaded: --force only bypasses the cache check of
    # the selected jobs, so a forced partial conversion keeps the other runs
    manifest = json.loads(man_path.read_text()) if man_path.is_file() else {"jobs": {}}
    if jobs_filter:
        other = {j.get("k_res") for j in manifest.get("jobs", {}).values() if j.get("status") == "ok"} - {k_res}
        if manifest.get("k_res") not in (None, k_res) or other:
            raise SystemExit(f"{ds}: jobs converted at k_res {sorted(other) or [manifest.get('k_res')]}; a partial "
                             f"conversion (--jobs) at k_res {k_res} would mix clocks in one dataset. Convert everything.")
    fam = load_families(families)
    pass_name = run_dir.name
    cells = sorted((run_dir / "cells").glob("*.json"))
    if not cells:
        raise SystemExit(f"{run_dir}: no cells/*.json (not a finished collector pass?)")
    t0 = time.time()
    n_done = n_skip = 0
    run_rows: dict[str, dict] = {}
    for cell in cells:
        rec = json.loads(cell.read_text())
        key = f"{rec['stem']}.{rec['tag']}"
        prior = manifest["jobs"].get(key)
        if jobs_filter and key not in jobs_filter:
            # keep what an earlier conversion recorded for the jobs not selected now
            for r in (prior or {}).get("runs", []):
                run_rows[r["run_id"]] = r
            continue
        log_path = run_dir / rec["log"]
        out_path = ds / "rows" / f"{key}.parquet"
        if not log_path.is_file():
            manifest["jobs"][key] = {"status": "no-log"}
            out_path.unlink(missing_ok=True)       # no stale rows for a run that is not there
            continue
        stat = log_path.stat()
        # everything the output depends on: the log, the collector's record,
        # every child log, the family table (and the stock pairing and k_res below)
        inputs = [sig_of(cell)] + [sig_of(run_dir / ch["log"]) for ch in rec.get("children") or []] + [sig_of(families)]
        # the stock log's identity is part of the cache key: a re-run with a
        # different pairing (or none) must reconvert
        sp = (stock_dir / "logs" / f"{rec['stem']}.stock.log") if stock_dir is not None else None
        stock_sig = None
        if sp is not None and sp.is_file() and rec["flavour"] != "stock":
            ss = sp.stat()
            # the stock run's collector record is part of the pairing: a run the
            # collector flagged (premature UNKNOWN, crash, contradiction) is no
            # reference, and a later flag must invalidate existing pairs
            srec_path = stock_dir / "cells" / f"{rec['stem']}.stock.json"
            srec = json.loads(srec_path.read_text()) if srec_path.is_file() else {}
            if srec.get("failed") or srec.get("contradiction"):
                print(f"  {key}: not paired, the stock run is flagged ({srec.get('note', '')[:60]})", flush=True)
            else:
                stock_sig = [str(sp), ss.st_size, int(ss.st_mtime), sig_of(srec_path)]
        if (not force and prior and prior.get("status") == "ok" and out_path.is_file()
                and prior.get("log_size") == stat.st_size and prior.get("log_mtime") == int(stat.st_mtime)
                and prior.get("inputs") == inputs
                and prior.get("stock_sig") == stock_sig and prior.get("k_res") == k_res):
            n_skip += 1
            for r in prior.get("runs", []):
                run_rows[r["run_id"]] = r
            continue
        try:
            log = Log(log_path)
        except Exception as e:
            manifest["jobs"][key] = {"status": f"error: {e!r}"}
            out_path.unlink(missing_ok=True)       # a run that cannot be read leaves no rows behind
            print(f"  {key}: {e!r}", flush=True)
            continue
        stock = None
        if stock_sig is not None:
            stock = Log(sp)
            # pairing is by grid position, which only means "the same search
            # ticks" when both runs used the same X_o
            if log.obs_ticks != stock.obs_ticks:
                print(f"  {key}: not paired, obs_ticks {log.obs_ticks} v stock {stock.obs_ticks}", flush=True)
                stock = None
                stock_sig = None
            elif (log.header.get("options") or {}).get("seed") != (stock.header.get("options") or {}).get("seed"):
                # every paired difference is on the same deal (plan section 5.4)
                print(f"  {key}: not paired, seed {(log.header.get('options') or {}).get('seed')} v stock "
                      f"{(stock.header.get('options') or {}).get('seed')}", flush=True)
                stock = None
                stock_sig = None
        run_id = f"{pass_name}/{key}"
        keys = {"run_id": run_id, "stem": rec["stem"], "tag": rec["tag"], "flavour": rec["flavour"],
                "seed": int(rec["seed"]), "is_child": False, "branch_decision": -1, "branch_knob": "",
                "branch_entry": float("nan"), "branch_index": -1, "parent_run_id": ""}
        tables = [to_arrow(row_table(log, keys, None, stock, k_res))]
        runs = [run_record(log, keys, rec, None, fam.get(rec["stem"], ""), pass_name, k_res)]
        parent_dicts = None      # the parent's rows as dicts, built once, for the children's replay
        for child in rec.get("children") or []:
            cpath = run_dir / child["log"]
            if not cpath.is_file():
                continue
            try:
                clog = Log(cpath)
            except Exception as e:      # an empty or truncated child log: skipped, the pass goes on
                print(f"  {key}: child {cpath.name} unreadable ({e!r}), skipped", flush=True)
                manifest.setdefault("unreadable_children", []).append(str(cpath))
                continue
            hb = clog.header.get("branch") or {}
            ckeys = {**keys, "run_id": f"{run_id}.b{child['branch']}.{child['k']}", "is_child": True,
                     "branch_decision": int(hb.get("decision", child["branch"])), "branch_knob": str(hb.get("knob", "")),
                     "branch_entry": float(hb.get("entry", float("nan"))), "branch_index": int(hb.get("index", child["k"])),
                     "parent_run_id": run_id}
            # the fork state is the parent's last row before the fork
            pr = int(hb.get("parent_rows", 0))
            prev = row_state(log, pr - 1, k_res) if 0 < pr <= log.n else None
            if prev is not None and clog.n:
                prev["cpu_ns"] = float(clog.col("cpu_ns")[0])      # restarts at fork: d_cpu_ns 0
            prefix = None
            if clog.header.get("k_res") != k_res and 0 < pr <= log.n and clog.has("avg_f_fast_glue"):
                if parent_dicts is None:
                    parent_dicts = row_dicts(log, k_res)
                prefix = parent_dicts[:pr]
            tables.append(to_arrow(row_table(clog, ckeys, prev, stock, k_res, prefix)))
            runs.append(run_record(clog, ckeys, rec, child, fam.get(rec["stem"], ""), pass_name, k_res))
            del clog
        table = pa.concat_tables(tables, promote_options="default")
        tmp = out_path.with_suffix(".parquet.tmp")
        pq.write_table(table, tmp, compression="zstd")
        os.replace(tmp, out_path)
        for r in runs:
            run_rows[r["run_id"]] = r
        manifest["jobs"][key] = {"status": "ok", "log_size": stat.st_size, "log_mtime": int(stat.st_mtime),
                                 "inputs": inputs,
                                 "stock_sig": stock_sig, "k_res": k_res, "rows": table.num_rows, "runs": runs,
                                 "parquet": str(out_path.relative_to(run_dir))}
        n_done += 1
        if n_done % 20 == 0:
            print(f"  {n_done} converted, {n_skip} skipped, {time.time() - t0:.0f}s", flush=True)
            write_manifest(man_path, manifest)      # a checkpoint: an interrupted pass resumes here
        del log, tables, table
    if not jobs_filter:
        # a job whose collector record is gone leaves no rows behind
        keys = {f"{json.loads(c.read_text())['stem']}.{json.loads(c.read_text())['tag']}" for c in cells}
        for stale in (ds / "rows").glob("*.parquet"):
            if stale.stem not in keys:
                stale.unlink()
                print(f"  pruned {stale.name}: no collector record", flush=True)
        for key in [k for k in manifest["jobs"] if k not in keys]:
            del manifest["jobs"][key]
    # runs.parquet from every job's records (converted now or before)
    if not run_rows:
        (ds / "runs.parquet").unlink(missing_ok=True)      # nothing usable: no stale outcomes either
    if run_rows:
        keys_all: list[str] = []
        for r in run_rows.values():
            for k in r:
                if k not in keys_all:
                    keys_all.append(k)
        cols = {}
        for k in keys_all:
            vals = [r.get(k) for r in run_rows.values()]
            cols[k] = pa.array(vals) if not all(v is None for v in vals) else pa.array(vals, type=pa.string())
        pq.write_table(pa.table(cols), ds / "runs.parquet", compression="zstd")
    manifest["updated"] = time.strftime("%Y-%m-%d %H:%M:%S")
    manifest["stock"] = stock_key
    manifest["k_res"] = k_res
    manifest["families"] = str(families)
    write_manifest(man_path, manifest)
    print(f"{ds}: {n_done} jobs converted, {n_skip} skipped (up to date), {len(run_rows)} runs in runs.parquet, "
          f"{time.time() - t0:.0f}s")
    return 0


# ---------------------------------------------------------------------------
# self-test on a synthetic fixture log
# ---------------------------------------------------------------------------

def write_fixture(path: Path, rows: list[dict], columns: list[str], kinds: str, footer: dict | None,
                  branch: dict | None = None, obs_ticks: int = 100) -> None:
    header = {"format": 1, "solver": "fixture", "k_res": 11, "cnf": "x.cnf", "pid": 1,
              "policy": {"mode": "stock", "obs_ticks": obs_ticks, "dec_ticks": 4 * obs_ticks, "seed": 0, "temp": 1,
                         "segment_mean": 5, "horizon": "ticks", "horizon_budget": 1000, "k_res": 11},
              "options": {}, "branch": branch, "net": None, "static_schema": {},
              "columns": columns, "kinds": kinds, "row_bytes": 8 * len(columns)}
    with open(path, "wb") as f:
        f.write(MAGIC + b"\n" + json.dumps(header).encode() + b"\n")
        for r in rows:
            vals = []
            for c, k in zip(columns, kinds):
                v = r.get(c, 0)
                vals.append(struct.unpack("<Q", struct.pack("<d", float(v)))[0] if k == "f" else int(v))
            f.write(struct.pack("<" + "Q" * len(columns), *vals))
        f.write(struct.pack("<" + "Q" * len(columns), SENTINEL, *([0] * (len(columns) - 1))))
        if footer is not None:
            f.write(json.dumps(footer).encode() + b"\n")


def selftest() -> int:
    cols = ["row", "obs_epoch", "row_boundary", "policy_decisions", "work", "ticks", "search_ticks",
            "eliminate_resolutions", "wall_ns", "cpu_ns", "conflicts", "active", "obs_x", "obs_g_eliminate_frac",
            "act_interval_probe", "dec_interval_probe", "tm_probe_fires", "tm_probe_stock_would_fire"]
    kinds = "".join("f" if c in ("obs_x", "obs_g_eliminate_frac", "act_interval_probe", "dec_interval_probe") else "u"
                    for c in cols)
    # The parent: boundary rows at epochs 0..5 and a final row. Decisions are
    # made from rows 0 (D0) and 4 (X_d = 4 X_o), so the count rises in rows 1
    # and 5. Per epoch 100 ticks and 3 resolutions: the logged work (k = 11)
    # grows by 133, the recomputed work (k = 7) by 121. search_ticks put row e
    # at grid position e.
    parent = []
    for e in range(6):
        parent.append({"row": e, "obs_epoch": e, "row_boundary": 1,
                       "policy_decisions": 0 if e == 0 else (1 if e < 5 else 2),
                       "work": 133 * e, "ticks": 100 * e, "search_ticks": 100 * e + 5, "eliminate_resolutions": 3 * e,
                       "wall_ns": 1000 * e, "cpu_ns": 900 * e, "conflicts": 7 * e, "active": 50 - e, "obs_x": 0.5 * e,
                       "obs_g_eliminate_frac": (11 * 3 * e) / (133 * e) if e else 0.0,
                       "act_interval_probe": 1.0 if e < 5 else 0.5, "dec_interval_probe": 1.0 if e < 5 else 0.0,
                       "tm_probe_fires": e // 2, "tm_probe_stock_would_fire": e % 2})
    parent.append({**parent[-1], "row": 6, "row_boundary": 0, "work": 765, "ticks": 600, "search_ticks": 545,
                   "eliminate_resolutions": 15, "wall_ns": 5400, "cpu_ns": 5000, "conflicts": 36})
    footer = {"result": "SATISFIABLE", "exit": 10, "reason": "solve", "rows": 7, "wall_ns": 5400, "cpu_ns": 5000,
              "peak_rss_bytes": 2 << 20, "work": 765, "conflicts": 36,
              "static": {"computed": True, "cost_ns": 5, "groups": {"shape": {"vars": 50, "clauses": 200},
                                                                    "preprocess": {"ticks": 100, "eliminate_resolutions": 3, "work": 133}}},
              "branches": [], "branch": None}
    # A child forked at the decision made from row 4 (parent_rows = 5): its
    # first row is the parent's row-4 state with the epoch counter one ahead,
    # a fresh CPU clock and the branch action in force; then its own rows.
    child = [dict(parent[4], row=0, obs_epoch=5, policy_decisions=2, cpu_ns=10, act_interval_probe=2.0,
                  dec_interval_probe=2.0)]
    child.append({**child[0], "row": 1, "obs_epoch": 6, "work": 870, "ticks": 650, "search_ticks": 655,
                  "eliminate_resolutions": 20, "wall_ns": 6100, "cpu_ns": 2010, "conflicts": 40, "active": 44})
    child.append({**child[1], "row": 2, "row_boundary": 0, "work": 880, "ticks": 660, "search_ticks": 665,
                  "cpu_ns": 2100, "conflicts": 41})
    cbranch = {"decision": 1, "epoch": 4, "knob": "probe", "entry": 2, "index": 0, "parent_rows": 5}
    cfooter = {**footer, "result": "UNKNOWN", "exit": 0, "rows": 3, "work": 880, "conflicts": 41, "branch": cbranch}
    with tempfile.TemporaryDirectory() as td:
        run = Path(td) / "rl-fixture-pass"
        (run / "logs").mkdir(parents=True)
        (run / "cells").mkdir()
        write_fixture(run / "logs" / "cellA.fork1.log", parent, cols, kinds, footer)
        write_fixture(run / "logs" / "cellA.fork1.log.b1.0", child, cols, kinds, cfooter, branch=cbranch)
        write_fixture(run / "logs" / "cellB.stock.log", parent[:3], cols, kinds, {**footer, "rows": 3})
        stock_run = Path(td) / "rl-fixture-stock"
        (stock_run / "logs").mkdir(parents=True)
        (stock_run / "cells").mkdir()
        # the stock trace: the parent's counters with one more conflict per row,
        # and a jump that skips grid position 3 (rows at positions 0,1,2,4,5)
        stock_rows = [dict(r, conflicts=r["conflicts"] + 1) for r in parent if r["obs_epoch"] != 3 or r["row_boundary"] == 0]
        for i, r in enumerate(stock_rows):
            r["row"] = i
            if r["row_boundary"]:
                r["obs_epoch"] = i
        write_fixture(stock_run / "logs" / "cellA.stock.log", stock_rows, cols, kinds, {**footer, "rows": len(stock_rows)})
        rec = {"stem": "cellA", "tag": "fork1", "flavour": "fork", "seed": 0, "limit_ticks": 1000, "wall_s": 60,
               "log": "logs/cellA.fork1.log", "result": "SAT", "verify": "ok", "oracle": "agree", "anomaly": None,
               "children": [{"branch": 1, "k": 0, "log": "logs/cellA.fork1.log.b1.0", "result": "UNKNOWN",
                             "verify": "skip", "anomaly": "wall_cap"}]}
        (run / "cells" / "cellA.fork1.json").write_text(json.dumps(rec))
        (run / "cells" / "cellB.stock.json").write_text(json.dumps({**rec, "stem": "cellB", "tag": "stock",
                                                                    "flavour": "stock", "log": "logs/cellB.stock.log",
                                                                    "children": []}))
        (stock_run / "cells" / "cellA.stock.json").write_text(json.dumps(
            {**rec, "tag": "stock", "flavour": "stock", "log": "logs/cellA.stock.log", "children": []}))
        convert(run, stock_run, Path("/nonexistent"), True, None, k_res=7)
        t = pq.read_table(run / "dataset" / "rows" / "cellA.fork1.parquet")
        d = {c: t.column(c).to_pylist() for c in t.column_names}
        n = t.num_rows
        assert n == 10, n
        par = [i for i in range(n) if not d["is_child"][i]]
        ch = [i for i in range(n) if d["is_child"][i]]
        assert len(par) == 7 and len(ch) == 3
        # work at the common k = 7, the logged value kept
        assert [d["work"][i] for i in par] == [121.0 * e for e in range(6)] + [705.0]
        assert [d["work_logged"][i] for i in par] == [133.0 * e for e in range(6)] + [765.0]
        assert [d["d_work"][i] for i in par] == [0.0] + [121.0] * 5 + [100.0]
        assert [d["d_ticks"][i] for i in par] == [0.0] + [100.0] * 5 + [100.0]
        assert [d["d_eliminate_resolutions"][i] for i in par] == [0.0] + [3.0] * 5 + [0.0]
        assert [d["d_conflicts"][i] for i in par] == [0.0] + [7.0] * 5 + [1.0]
        assert [d["grid"][i] for i in par] == [0, 1, 2, 3, 4, 5, 5]
        # decisions sit on the state rows 0 and 4; the chosen action is the next row's
        assert [d["is_decision"][i] for i in par] == [True, False, False, False, True, False, False]
        taken = [d["act_taken_interval_probe"][i] for i in par]
        # row 4 chose the one-shot 0 (dec_* of row 5) although row 5's act_* shows the reset 0.5
        assert taken[0] == 1.0 and taken[4] == 0.0 and all(x != x for j, x in enumerate(taken) if j not in (0, 4))
        assert [d["boundary"][i] for i in par] == [True] * 6 + [False]
        assert d["active"][par[3]] == 47.0
        # the child's first row is the parent's row 4 (parent_rows - 1): zero deltas, cpu delta zero by definition
        assert d["work"][ch[0]] == 121.0 * 4 and d["d_work"][ch[0]] == 0.0 and d["d_conflicts"][ch[0]] == 0.0
        assert d["d_wall_ns"][ch[0]] == 0.0 and d["d_cpu_ns"][ch[0]] == 0.0 and d["d_cpu_ns"][ch[1]] == 2000.0
        assert d["d_ticks"][ch[1]] == 250.0 and d["d_conflicts"][ch[1]] == 12.0 and d["d_work"][ch[1]] == 790.0 - 484.0
        assert d["d_work"][ch[2]] == 10.0 and d["boundary"][ch[2]] is False
        assert [d["is_decision"][i] for i in ch] == [False, False, False]
        assert d["act_interval_probe"][ch[0]] == 2.0 and d["act_interval_probe"][par[4]] == 1.0
        assert d["branch_knob"][ch[0]] == "probe" and d["branch_entry"][ch[0]] == 2.0 and d["branch_decision"][ch[0]] == 1
        assert d["parent_run_id"][ch[0]] == "rl-fixture-pass/cellA.fork1" and d["parent_run_id"][par[0]] == ""
        assert d["tm_probe_stock_would_fire"][par[3]] == 1 and d["obs_x"][par[4]] == 2.0
        # the one k-dependent observation entry is recomputed at k = 7: 21e / 121e
        import struct as _st
        f32 = lambda x: _st.unpack("<f", _st.pack("<f", x))[0]
        assert d["obs_g_eliminate_frac"][par[0]] == 0.0 and d["obs_g_eliminate_frac"][par[2]] == f32(42.0 / 242.0)
        # stock pairing by grid position: position 3 is missing in the stock trace, the
        # child's first row pairs with position 4, its later rows (6) and final rows never pair
        assert [d["stock_paired"][i] for i in par] == [True, True, True, False, True, True, False]
        assert d["stock_conflicts"][par[2]] == 15.0 and d["stock_work"][par[2]] == 242.0
        assert d["stock_paired"][ch[0]] is True and d["stock_conflicts"][ch[0]] == 29.0
        assert d["stock_paired"][ch[1]] is False and d["stock_conflicts"][ch[1]] != d["stock_conflicts"][ch[1]]  # nan
        r = pq.read_table(run / "dataset" / "runs.parquet")
        rd = {c: r.column(c).to_pylist() for c in r.column_names}
        rid = dict(zip(rd["run_id"], range(r.num_rows)))
        assert r.num_rows == 3
        p_, c_ = rid["rl-fixture-pass/cellA.fork1"], rid["rl-fixture-pass/cellA.fork1.b1.0"]
        assert rd["solved"][p_] is True and rd["censored"][c_] is True
        assert rd["s_shape_vars"][p_] == 50.0 and rd["work_end"][p_] == 705.0 and rd["work_logged_end"][p_] == 765.0
        assert rd["work_end"][c_] == 800.0 and rd["anomaly"][c_] == "wall_cap" and rd["anomaly"][p_] is None
        assert rd["decisions_n"][p_] == 2 and rd["boundary_rows"][p_] == 6 and rd["branch_knob"][c_] == "probe"
        assert rd["limit_ticks"][p_] == 1000 and rd["peak_rss_mb"][p_] == 2.0 and rd["k_res"][p_] == 7
        assert abs(rd["limit_ticks_k"][p_] - 1000 * 705 / 765) < 1e-9 and abs(rd["budget_frac_end"][p_] - 705 / (1000 * 705 / 765)) < 1e-9
        assert rd["s_preprocess_work"][p_] == 121.0 and rd["failed"][p_] is False
        # the collector's failure flags survive conversion and mark the run as not data
        rec_fail = json.loads((run / "cells" / "cellB.stock.json").read_text())
        rec_fail["premature"] = True
        (run / "cells" / "cellB.stock.json").write_text(json.dumps(rec_fail))
        convert(run, stock_run, Path("/nonexistent"), False, None, k_res=7)
        rd2 = pq.read_table(run / "dataset" / "runs.parquet").to_pydict()
        assert rd2["failed"][dict(zip(rd2["run_id"], range(3)))["rl-fixture-pass/cellB.stock"]] is True
        # resume: nothing to do the second time; a filtered rerun keeps the other runs
        convert(run, stock_run, Path("/nonexistent"), False, None, k_res=7)
        man = json.loads((run / "dataset" / "MANIFEST.json").read_text())
        assert man["jobs"]["cellA.fork1"]["status"] == "ok" and man["jobs"]["cellA.fork1"]["rows"] == 10
        convert(run, stock_run, Path("/nonexistent"), False, {"cellB.stock"}, k_res=7)
        assert pq.read_table(run / "dataset" / "runs.parquet").num_rows == 3
        convert(run, stock_run, Path("/nonexistent"), True, {"cellB.stock"}, k_res=7)
        assert pq.read_table(run / "dataset" / "runs.parquet").num_rows == 3
        try:
            convert(run, stock_run, Path("/nonexistent"), False, {"cellB.stock"}, k_res=11)
            raise AssertionError("a partial conversion at another k_res must be refused")
        except SystemExit:
            pass
        # a different k_res or pairing reconverts instead of reusing the cache
        convert(run, None, Path("/nonexistent"), False, None, k_res=7)
        t2 = pq.read_table(run / "dataset" / "rows" / "cellA.fork1.parquet")
        assert "stock_paired" not in t2.column_names
        convert(run, stock_run, Path("/nonexistent"), False, None, k_res=11)
        t3 = pq.read_table(run / "dataset" / "rows" / "cellA.fork1.parquet")
        assert t3.column("work").to_pylist()[:2] == [0.0, 133.0] and "stock_paired" in t3.column_names
    print("selftest ok")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("convert", help="one collector pass to parquet")
    c.add_argument("run_dir")
    c.add_argument("--stock", default="", help="the stock pass to pair against (round 0 and later)")
    c.add_argument("--families", default=str(DEFAULT_FAMILIES))
    c.add_argument("--k-res", type=int, default=DEFAULT_K_RES, help="k_res used to recompute W for every log")
    c.add_argument("--force", action="store_true", help="reconvert everything")
    c.add_argument("--jobs", default="", help="comma list of <stem>.<tag> keys to convert (default all)")
    c.set_defaults(fn=lambda a: convert(Path(a.run_dir), Path(a.stock) if a.stock else None, Path(a.families),
                                        a.force, set(a.jobs.split(",")) if a.jobs else None, a.k_res))
    s = sub.add_parser("selftest", help="convert a synthetic fixture log and check the deltas")
    s.set_defaults(fn=lambda a: selftest())
    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
