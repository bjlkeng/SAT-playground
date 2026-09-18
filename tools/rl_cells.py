#!/usr/bin/env python3
"""rl_cells.py — the per-cell table: B_cell, peak RSS, family, status, stock time.

Plan: plan/rl-scheduler-solver13-plan.md §3.4 (B_cell), §5.4 (stratified
budgets), §7 item 6d (child memory). Bead SAT-playground-p9m.7.7 (step B.6).

One table the collector's job builders and the evaluation scripts read:
benchmarks/rl/cells_2025.tsv, one row per cell of sat-comp-2025, built
from the stock-trace pass (tools/rl_collect.py `stock`, converted by
tools/rl_dataset.py) and, when it exists, the timeout-band pass at 3600 s.

    stem, name, family, split          benchmarks/rl/families.tsv, split_2025.tsv
    seed                               the stock run's --seed; every perturbed run
                                       of the cell uses it (plan section 5.4)
    status                             SAT | UNSAT | TIMEOUT from the stock trace
    failed                             1 when the collector flagged the run (premature
                                       UNKNOWN, crash, contradiction): no B_cell
    stock_time_s, stock_work, stock_conflicts, rows, decisions
                                       the stock run's wall, W at exit, conflicts,
                                       log rows, policy decisions (X_d = 2^27)
    work_per_s                         W / wall: the cell's tick rate
    B_cell                             the work budget of every perturbed run
                                       (plan §5.4): the W the stock run had
                                       reached at clamp(3 x solve time, 120 s,
                                       1800 s) of its own wall. A solved run
                                       ends before that point, so W there is
                                       extrapolated at the run's own rate:
                                       B_cell = W_end x clamp(3t, 120, 1800) / t.
                                       A timeout cell's B_cell is W at the kill.
                                       W is recomputed as ticks + k_res x
                                       eliminate_resolutions with --k-res (default
                                       7, the step-B fit), whatever k_res the
                                       binary that made the trace printed: a
                                       budget must be in the units of the binary
                                       that will run under it.
    band                               1 when stock did not solve at 1800 s
    band_time_s, band_status, band_work, B_cell_band
                                       from the 3600 s pass (--band): the band's
                                       budget is W at 3600 s (or at the solve,
                                       times 3600 / t, if it solved)
    peak_rss_mb, band_peak_rss_mb      getrusage peak from the log footer, for the
                                       collector's memory admission
    verify, oracle                     the collector's correctness columns

    ~/.cache/sat13-rl/venv/bin/python tools/rl_cells.py --stock log/rl-stock2025-<ts> [--band log/rl-band2025-<ts>]
"""
from __future__ import annotations

import argparse
import csv
import sys
import time
from pathlib import Path

import pyarrow.parquet as pq

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from rl_split import load_split  # noqa: E402

FAMILIES = ROOT / "benchmarks" / "rl" / "families.tsv"
OUT = ROOT / "benchmarks" / "rl" / "cells_2025.tsv"
COLUMNS = ("stem", "name", "family", "split", "seed", "status", "failed", "stock_time_s", "stock_work", "stock_conflicts", "rows",
           "decisions", "work_per_s", "B_cell", "band", "band_status", "band_time_s", "band_work", "B_cell_band",
           "peak_rss_mb", "band_peak_rss_mb", "verify", "oracle")


def clamp(x: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, x))


def load_pass(run_dir: Path, tag: str) -> dict[str, dict]:
    """Per stem: the collector record (results.tsv) and the run record (runs.parquet)."""
    out: dict[str, dict] = {}
    with open(run_dir / "results.tsv", newline="") as f:
        for r in csv.DictReader(f, dialect="excel-tab"):
            if r["tag"] == tag:
                out[r["stem"]] = {"res": r}
    runs_path = run_dir / "dataset" / "runs.parquet"
    if runs_path.is_file():
        runs = pq.read_table(runs_path).to_pydict()
        for i, stem in enumerate(runs["stem"]):
            if runs["tag"][i] == tag and not runs["is_child"][i] and stem in out:
                out[stem]["run"] = {k: runs[k][i] for k in runs}
    return out


def work_of(r: dict, k_res: int) -> float:
    """W = ticks + k_res x eliminate_resolutions from a results.tsv row (NA when the solver printed no exit line)."""
    if r.get("ticks") in ("NA", "", None) or r.get("eliminate_resolutions") in ("NA", "", None):
        return float("nan")
    return float(r["ticks"]) + k_res * float(r["eliminate_resolutions"])


def status_of(r: dict) -> str:
    return {"SAT": "SAT", "UNSAT": "UNSAT", "TIMEOUT": "TIMEOUT"}.get(r["result"], r["result"])


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--stock", required=True)
    ap.add_argument("--band", default="")
    ap.add_argument("--timeout", type=float, default=1800.0)
    ap.add_argument("--band-timeout", type=float, default=3600.0)
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--k-res", type=int, default=7, help="k_res for W = ticks + k_res x eliminate_resolutions (default 7)")
    args = ap.parse_args(argv)
    stock_dir = Path(args.stock).resolve()
    band_dir = Path(args.band).resolve() if args.band else None
    stock = load_pass(stock_dir, "stock")
    band = load_pass(band_dir, "stock") if band_dir else {}
    split = load_split()
    with open(FAMILIES, newline="") as f:
        fam = {r["stem"]: r for r in csv.DictReader(f, dialect="excel-tab") if r["suite"] == "sat-comp-2025"}
    rows = []
    missing = []
    n_failed = 0
    for stem in sorted(fam):
        if stem not in stock:
            missing.append(stem)
            continue
        r = stock[stem]["res"]
        run = stock[stem].get("run") or {}
        status = status_of(r)
        t = float(r["wall_s"])
        w = work_of(r, args.k_res)
        rate = w / t if t > 0 else float("nan")
        # a run the collector flagged (premature UNKNOWN, crash, contradiction)
        # is not a result: no budget is derived from it (CLAUDE.md "Correctness
        # is absolute"); the flag comes from the converted runs table
        failed = bool(run.get("failed", False)) or r.get("failed", "0") == "1"
        if failed:
            n_failed += 1
            b_cell = float("nan")
        elif status in ("SAT", "UNSAT"):
            b_cell = w * clamp(3.0 * t, 120.0, args.timeout) / t if t > 0 else w
        else:
            b_cell = w
        row = {
            "stem": stem, "name": fam[stem]["name"], "family": fam[stem]["family"], "split": split.get(stem, ""),
            "seed": r["seed"], "status": status, "failed": int(failed), "stock_time_s": f"{t:.3f}", "stock_work": int(w) if w == w else "",
            "stock_conflicts": r["conflicts"], "rows": r["rows"], "decisions": run.get("decisions_n", ""),
            "work_per_s": f"{rate:.0f}" if rate == rate else "", "B_cell": int(b_cell) if b_cell == b_cell else "",
            "band": int(status not in ("SAT", "UNSAT")), "band_status": "", "band_time_s": "", "band_work": "",
            "B_cell_band": "", "peak_rss_mb": r["peak_rss_mb"], "band_peak_rss_mb": "",
            "verify": r["verify"], "oracle": r["oracle"],
        }
        if stem in band:
            br = band[stem]["res"]
            bt = float(br["wall_s"])
            bw = work_of(br, args.k_res)
            bstatus = status_of(br)
            bfailed = bool((band[stem].get("run") or {}).get("failed", False)) or br.get("failed", "0") == "1"
            row.update({"band_status": bstatus, "band_time_s": f"{bt:.3f}", "band_work": int(bw) if bw == bw else "",
                        "band_peak_rss_mb": br["peak_rss_mb"]})
            if bfailed:
                n_failed += 1
                row["band_status"] = "FAILED"
            elif bw == bw:
                row["B_cell_band"] = int(bw * args.band_timeout / bt) if bstatus in ("SAT", "UNSAT") and bt > 0 else int(bw)
        rows.append(row)
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with open(out, "w", newline="") as f:
        f.write(f"# per-cell table of sat-comp-2025; stock pass {stock_dir.name}"
                + (f"; band pass {band_dir.name}" if band_dir else "; no band pass yet")
                + f"; W = ticks + {args.k_res} x eliminate_resolutions"
                + f"; built {time.strftime('%Y-%m-%d %H:%M')} by tools/rl_cells.py\n")
        w = csv.writer(f, dialect="excel-tab", lineterminator="\n")
        w.writerow(COLUMNS)
        for row in rows:
            w.writerow([row[c] for c in COLUMNS])
    n_band = sum(r["band"] for r in rows)
    print(f"{out}: {len(rows)} cells ({len(missing)} without a stock record), {n_band} on the band, "
          f"{sum(1 for r in rows if r['band_status'])} with a band record")
    if n_failed:
        print(f"  *** {n_failed} run(s) carry a correctness failure and got no budget; debug the pass before using it")
    if missing:
        print("  missing:", missing[:5])
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
