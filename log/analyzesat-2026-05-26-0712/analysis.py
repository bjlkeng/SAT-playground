#!/usr/bin/env python3
"""analysis.py — work × speed decomposition + kissat reference gap.

Copied from analyzesat-2026-05-25-2043 with config-list adapted for this run.
"""
from __future__ import annotations

import argparse
import csv
import json
import sys
from pathlib import Path

DEFAULT_SLUG_DIR = Path("/home/bojji/code/SAT-playground/log/analyzesat-2026-05-26-0712")
CONFIGS_ORDER = ["A_baseline", "B_metadata_only", "C_focused_stable",
                 "D_focused_stable_ema", "E_lucky", "F_focused_stable_ema_lucky"]


def short_instance(name: str) -> str:
    base = name.rsplit("/", 1)[-1]
    if "-" in base:
        parts = base.split("-", 1)
        if len(parts[0]) == 32 and all(c in "0123456789abcdef" for c in parts[0]):
            base = parts[1]
    for suf in (".cnf.xz", ".cnf.gz", ".cnf"):
        if base.endswith(suf):
            base = base[: -len(suf)]
    return base


def load_results(cfg_dir: Path):
    results_csv = cfg_dir / "results.csv"
    if not results_csv.exists():
        return {}
    rows = {}
    with results_csv.open() as fh:
        for r in csv.DictReader(fh):
            inst = r["instance"]
            try:
                wall = float(r["time_s"])
            except (TypeError, ValueError):
                wall = float("nan")
            rows[inst] = {
                "wall": wall,
                "result": r.get("result", ""),
                "timeout": r.get("timeout", ""),
                "verified": r.get("verified", ""),
                "exit_code": r.get("exit_code", ""),
            }
    return rows


def load_stats(cfg_dir: Path):
    stats_jl = cfg_dir / "stats.jsonl"
    if not stats_jl.exists():
        return {}
    out = {}
    with stats_jl.open() as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            inst = obj.get("instance") or obj.get("input_basename") or obj.get("input")
            if inst is None:
                continue
            out[inst] = obj
    return out


def get_counter(stats: dict, *keys, default=0):
    if not stats:
        return default
    for k in keys:
        if k in stats:
            return stats[k]
        for sub in ("counters", "stats", "search"):
            if sub in stats and isinstance(stats[sub], dict) and k in stats[sub]:
                return stats[sub][k]
    return default


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--slug-dir", default=str(DEFAULT_SLUG_DIR))
    ap.add_argument("--reference-csv", default="")
    args = ap.parse_args()

    slug_dir = Path(args.slug_dir)

    configs = {}
    for cfg in CONFIGS_ORDER:
        cfg_dir = slug_dir / cfg
        configs[cfg] = {
            "results": load_results(cfg_dir),
            "stats": load_stats(cfg_dir),
        }

    instances = sorted({inst for cfg in configs.values() for inst in cfg["results"].keys()})
    if not instances:
        print("ERROR: no results", file=sys.stderr)
        sys.exit(1)

    matrix_csv = slug_dir / "matrix.csv"
    with matrix_csv.open("w") as fh:
        w = csv.writer(fh)
        w.writerow(["config", "instance", "result", "wall_s", "conflicts",
                    "decisions", "propagations", "props_per_s", "restarts",
                    "lucky_solved"])
        for cfg in CONFIGS_ORDER:
            results = configs[cfg]["results"]
            stats = configs[cfg]["stats"]
            for inst in instances:
                r = results.get(inst, {})
                s = stats.get(inst, {})
                wall = r.get("wall", float("nan"))
                conf = get_counter(s, "conflicts")
                deci = get_counter(s, "decisions")
                props = get_counter(s, "propagations")
                rest = get_counter(s, "restarts")
                lucky = get_counter(s, "lucky_solved", "lucky_calls")
                pps = props / wall if (isinstance(wall, (int, float)) and wall > 0 and props) else 0
                w.writerow([cfg, short_instance(inst), r.get("result", ""),
                            f"{wall:.3f}" if isinstance(wall, (int, float)) else "",
                            conf, deci, props, f"{pps:.0f}", rest, lucky])
    print(f"wrote {matrix_csv}")

    decomp_csv = slug_dir / "decomp.csv"
    baseline = configs["A_baseline"]
    with decomp_csv.open("w") as fh:
        w = csv.writer(fh)
        w.writerow(["config", "instance", "result_A", "result_cfg",
                    "wall_A", "wall_cfg", "conf_A", "conf_cfg",
                    "props_A", "props_cfg",
                    "work_ratio", "speed_ratio", "net_pred",
                    "actual_wall_ratio", "dominant"])
        for cfg in CONFIGS_ORDER[1:]:
            results = configs[cfg]["results"]
            stats = configs[cfg]["stats"]
            for inst in instances:
                rA = baseline["results"].get(inst, {})
                rC = results.get(inst, {})
                sA = baseline["stats"].get(inst, {})
                sC = stats.get(inst, {})
                wA = rA.get("wall", float("nan"))
                wC = rC.get("wall", float("nan"))
                cA = get_counter(sA, "conflicts") or 0
                cC = get_counter(sC, "conflicts") or 0
                pA = get_counter(sA, "propagations") or 0
                pC = get_counter(sC, "propagations") or 0
                ppsA = pA / wA if wA and wA > 0 else 0
                ppsC = pC / wC if wC and wC > 0 else 0
                work = (cC / cA) if cA > 0 else float("nan")
                speed = (ppsA / ppsC) if ppsC > 0 else float("nan")
                net = work * speed if work == work and speed == speed else float("nan")
                actual = (wC / wA) if wA and wA > 0 else float("nan")
                if work == work and speed == speed:
                    if abs(work - 1) > 0.10 and abs(speed - 1) < 0.05:
                        dom = "trajectory"
                    elif abs(speed - 1) > 0.05 and abs(work - 1) < 0.10:
                        dom = "execution"
                    elif abs(work - 1) > 0.10 and abs(speed - 1) > 0.05:
                        dom = "mixed"
                    else:
                        dom = "noise"
                else:
                    dom = "incomparable"
                w.writerow([cfg, short_instance(inst), rA.get("result", ""),
                            rC.get("result", ""),
                            f"{wA:.3f}" if isinstance(wA, (int, float)) else "",
                            f"{wC:.3f}" if isinstance(wC, (int, float)) else "",
                            cA, cC, pA, pC,
                            f"{work:.3f}" if work == work else "",
                            f"{speed:.3f}" if speed == speed else "",
                            f"{net:.3f}" if net == net else "",
                            f"{actual:.3f}" if actual == actual else "",
                            dom])
    print(f"wrote {decomp_csv}")

    if args.reference_csv:
        ref_rows = {}
        with open(args.reference_csv) as fh:
            for r in csv.DictReader(fh):
                try:
                    wall = float(r["time_s"])
                except (TypeError, ValueError):
                    wall = float("nan")
                ref_rows[r["instance"]] = {"wall": wall, "result": r["result"]}
        ref_csv = slug_dir / "reference_gap.csv"
        with ref_csv.open("w") as fh:
            w = csv.writer(fh)
            w.writerow(["instance", "kissat_result", "repo_result",
                        "kissat_wall", "repo_wall", "wall_ratio_repo_over_kissat",
                        "classification"])
            for inst in instances:
                ref = ref_rows.get(inst, {})
                rA = baseline["results"].get(inst, {})
                kw = ref.get("wall", float("nan"))
                rw = rA.get("wall", float("nan"))
                ratio = (rw / kw) if (kw and kw > 0) else float("nan")
                if ratio != ratio:
                    cls = "incomparable"
                elif ratio > 2.0:
                    cls = "repo_much_slower"
                elif ratio > 1.2:
                    cls = "repo_slower"
                elif ratio < 0.83:
                    cls = "repo_faster"
                else:
                    cls = "comparable"
                w.writerow([short_instance(inst),
                            ref.get("result", ""), rA.get("result", ""),
                            f"{kw:.3f}" if isinstance(kw, (int, float)) else "",
                            f"{rw:.3f}" if isinstance(rw, (int, float)) else "",
                            f"{ratio:.2f}" if ratio == ratio else "",
                            cls])
        print(f"wrote {ref_csv}")

    summary_md = slug_dir / "summary.md"
    with summary_md.open("w") as fh:
        fh.write(f"# analyzesat-2026-05-26-0712 summary\n\nSlug dir: {slug_dir}\n\n")
        fh.write("## PAR-2 per config (300s timeout, profiling suite, HEAD 9143376)\n\n")
        fh.write("| config | solved | timeout | unknown | error | PAR-2 |\n")
        fh.write("|---|---:|---:|---:|---:|---:|\n")
        for cfg in CONFIGS_ORDER:
            results = configs[cfg]["results"]
            solved = sum(1 for r in results.values()
                         if r["result"] in ("SAT", "UNSAT"))
            to = sum(1 for r in results.values() if r["result"] == "TIMEOUT")
            unk = sum(1 for r in results.values() if r["result"] == "UNKNOWN")
            err = sum(1 for r in results.values()
                      if r["result"] not in ("SAT", "UNSAT", "TIMEOUT", "UNKNOWN"))
            par2 = sum(
                (r["wall"] if r["result"] in ("SAT", "UNSAT") else 600.0)
                for r in results.values()
            )
            fh.write(f"| {cfg} | {solved} | {to} | {unk} | {err} | {par2:.1f} |\n")
        fh.write("\nSee matrix.csv, decomp.csv, reference_gap.csv.\n")
    print(f"wrote {summary_md}")


if __name__ == "__main__":
    main()
