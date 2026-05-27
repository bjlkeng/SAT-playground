#!/usr/bin/env python3
"""
Multi-config ablation analysis for the search-control angle.

Reads each <config>/results.csv and <config>/stats.jsonl, computes PAR-2,
work × speed decomposition vs A_baseline, and writes decomp.csv + summary.
"""
from __future__ import annotations
import csv
import json
import sys
from pathlib import Path

ROOT = Path(__file__).parent
CONFIGS = [
    "A_baseline",
    "B_chrono",
]
TIMEOUT = 300


def load_results(cfg_dir: Path) -> dict[str, dict]:
    rows = {}
    csv_path = cfg_dir / "results.csv"
    if not csv_path.exists():
        return rows
    with csv_path.open() as f:
        r = csv.DictReader(f)
        for row in r:
            rows[row["instance"]] = row
    return rows


def load_stats(cfg_dir: Path) -> dict[str, dict]:
    stats = {}
    jsonl = cfg_dir / "stats.jsonl"
    if not jsonl.exists():
        return stats
    with jsonl.open() as f:
        for line in f:
            try:
                obj = json.loads(line)
            except Exception:
                continue
            inst = obj.get("instance") or obj.get("name")
            if inst:
                stats[inst] = obj
    return stats


def par2(rows: dict[str, dict]) -> tuple[float, int, int]:
    total = 0.0
    solved = 0
    timeouts = 0
    for r in rows.values():
        t = float(r["time_s"]) if r["time_s"] else 0.0
        result = (r["result"] or "").upper()
        if result in ("SAT", "UNSAT"):
            total += t
            solved += 1
        else:
            total += 2 * TIMEOUT
            timeouts += 1
    return total, solved, timeouts


def get_stat(s: dict, *keys, default=0):
    cur = s
    for k in keys:
        if not isinstance(cur, dict):
            return default
        cur = cur.get(k)
        if cur is None:
            return default
    return cur


def main():
    summary_rows = []
    decomp_rows = []
    per_cfg = {}
    for cfg in CONFIGS:
        cd = ROOT / cfg
        rows = load_results(cd)
        stats = load_stats(cd)
        p2, solved, to = par2(rows)
        per_cfg[cfg] = (rows, stats, p2)
        summary_rows.append((cfg, len(rows), solved, to, p2))

    base_rows, base_stats, base_p2 = per_cfg[CONFIGS[0]]
    print("Config matrix")
    print(f"{'config':<32} {'#inst':>5} {'solved':>7} {'TO':>4} {'PAR-2':>10} {'Δ%':>6}")
    for cfg, n, solved, to, p2 in summary_rows:
        d = 100.0 * (p2 - base_p2) / base_p2 if base_p2 else 0.0
        print(f"{cfg:<32} {n:>5} {solved:>7} {to:>4} {p2:>10.1f} {d:>+5.1f}%")

    print("\nWork × Speed decomposition vs A_baseline")
    print(f"{'config':<28} {'instance':<48} {'wall_r':>7} {'work_r':>7} {'speed_r':>8} {'net':>6}  {'cause'}")
    for cfg in CONFIGS[1:]:
        rows, stats, _ = per_cfg[cfg]
        for inst, row in rows.items():
            base_row = base_rows.get(inst)
            if not base_row:
                continue
            t_cfg = float(row["time_s"]) or 1e-9
            t_base = float(base_row["time_s"]) or 1e-9
            wall_r = t_cfg / t_base
            s_cfg = stats.get(inst, {})
            s_base = base_stats.get(inst, {})
            conf_cfg = get_stat(s_cfg, "conflicts", default=0) or 0
            conf_base = get_stat(s_base, "conflicts", default=0) or 0
            props_cfg = get_stat(s_cfg, "propagations", default=0) or 0
            props_base = get_stat(s_base, "propagations", default=0) or 0
            if conf_base and conf_cfg:
                work_r = conf_cfg / conf_base
            else:
                work_r = float("nan")
            if props_cfg and props_base and t_cfg and t_base:
                pps_cfg = props_cfg / t_cfg
                pps_base = props_base / t_base
                speed_r = pps_base / pps_cfg if pps_cfg else float("nan")
            else:
                speed_r = float("nan")
            net = work_r * speed_r if work_r == work_r and speed_r == speed_r else float("nan")
            cause = ""
            if wall_r > 1.10:
                if work_r > 1.10 and speed_r < 1.10:
                    cause = "trajectory"
                elif speed_r > 1.10 and work_r < 1.10:
                    cause = "execution"
                elif work_r > 1.10 and speed_r > 1.10:
                    cause = "combined"
            elif wall_r < 0.90:
                if work_r < 0.90 and speed_r > 0.90:
                    cause = "WIN(trajectory)"
                elif speed_r < 0.90 and work_r > 0.90:
                    cause = "WIN(execution)"
                else:
                    cause = "WIN"
            decomp_rows.append({
                "config": cfg,
                "instance": inst,
                "wall_ratio": f"{wall_r:.2f}",
                "work_ratio": f"{work_r:.2f}",
                "speed_ratio": f"{speed_r:.2f}",
                "net": f"{net:.2f}",
                "cause": cause,
                "result_cfg": row.get("result"),
                "result_base": base_row.get("result"),
                "t_cfg": row.get("time_s"),
                "t_base": base_row.get("time_s"),
                "conflicts_cfg": conf_cfg,
                "conflicts_base": conf_base,
            })
            print(f"{cfg:<28} {inst[:46]:<48} {wall_r:>7.2f} {work_r:>7.2f} {speed_r:>8.2f} {net:>6.2f}  {cause}")

    out = ROOT / "decomp.csv"
    with out.open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(decomp_rows[0].keys()) if decomp_rows else [])
        if decomp_rows:
            w.writeheader()
            for r in decomp_rows:
                w.writerow(r)
    print(f"\nWrote {out}")


if __name__ == "__main__":
    main()
