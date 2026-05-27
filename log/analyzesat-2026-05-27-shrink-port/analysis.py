#!/usr/bin/env python3
"""Work × speed decomposition for analyzesat-2026-05-27-shrink-port.

Loads A_baseline (from this run's directory OR from the in-flight nextbeads CSV)
and D_inblock, joins on instance, and prints per-instance:
- result A, result D
- wall A, wall D, ratio
- conflicts A, conflicts D, work_ratio
- props/s A, props/s D, speed_ratio (A/D, >1 means D is slower per event)
- net = work_ratio * speed_ratio
"""
from __future__ import annotations

import csv
from pathlib import Path

ROOT = Path(__file__).parent
BENCHMARK_DIR = ROOT.parent.parent / "benchmarks" / "profiling"


def load_csv(path: Path):
    rows = []
    with path.open() as f:
        reader = csv.DictReader(f)
        for r in reader:
            rows.append(r)
    return {r["instance"]: r for r in rows}


def parse_float(x, default=0.0):
    try:
        return float(x)
    except (TypeError, ValueError):
        return default


def parse_int(x, default=0):
    try:
        return int(float(x))
    except (TypeError, ValueError):
        return default


def main():
    a_path = ROOT / "A_baseline" / "results.csv"
    d_path = ROOT / "D_inblock" / "results.csv"
    nextbeads_path = ROOT.parent / "nextbeads-2026-05-27-s11-04d-before" / "results.csv"
    ref_path = ROOT / "reference-kissat-latest.csv"

    if a_path.exists():
        a = load_csv(a_path)
        a_source = "local A_baseline"
    elif nextbeads_path.exists():
        a = load_csv(nextbeads_path)
        a_source = "nextbeads in-flight"
    else:
        print("No A baseline found.")
        return

    if not d_path.exists():
        print("D_inblock not run yet.")
        return

    d = load_csv(d_path)
    ref = load_csv(ref_path)

    instances = sorted(set(a.keys()) & set(d.keys()))

    print(f"# Work × Speed decomposition  (A baseline source: {a_source})")
    print(f"# {len(instances)} matched instances\n")

    fmt_header = (
        "instance  res_A  res_D  wall_A  wall_D  Δwall%  "
        "conf_A  conf_D  workΔ%  props/s_A  props/s_D  speedΔ%  net%"
    )
    print(fmt_header)
    print("-" * len(fmt_header))

    par2_a = par2_d = 0.0
    par2_timeout = 600.0  # 2 * 300s
    for inst in instances:
        ra = a[inst]
        rd = d[inst]

        wall_a = parse_float(ra.get("time_s"))
        wall_d = parse_float(rd.get("time_s"))
        res_a = ra.get("result", "?")
        res_d = rd.get("result", "?")

        conf_a = parse_int(ra.get("conflicts", 0))
        conf_d = parse_int(rd.get("conflicts", 0))
        prop_a = parse_int(ra.get("propagations", 0))
        prop_d = parse_int(rd.get("propagations", 0))

        if res_a in ("SAT", "UNSAT"):
            par2_a += wall_a
        else:
            par2_a += par2_timeout
        if res_d in ("SAT", "UNSAT"):
            par2_d += wall_d
        else:
            par2_d += par2_timeout

        delta_wall = 100.0 * (wall_d - wall_a) / wall_a if wall_a > 0 else 0.0

        if conf_a > 0 and conf_d > 0:
            work_ratio = conf_d / conf_a
            work_pct = 100.0 * (work_ratio - 1.0)
        else:
            work_ratio = float("nan")
            work_pct = float("nan")

        pps_a = prop_a / wall_a if wall_a > 0 else 0.0
        pps_d = prop_d / wall_d if wall_d > 0 else 0.0
        if pps_d > 0 and pps_a > 0:
            speed_ratio = pps_a / pps_d  # > 1 if D is slower
            speed_pct = 100.0 * (speed_ratio - 1.0)
        else:
            speed_ratio = float("nan")
            speed_pct = float("nan")

        if work_ratio == work_ratio and speed_ratio == speed_ratio:
            net_pct = 100.0 * (work_ratio * speed_ratio - 1.0)
        else:
            net_pct = float("nan")

        # Trim instance hash prefix for display
        inst_short = inst.split("-", 1)[1] if "-" in inst else inst
        inst_short = inst_short[:30]

        print(
            f"{inst_short:<30}  {res_a:<6}  {res_d:<6}  "
            f"{wall_a:>6.1f}  {wall_d:>6.1f}  {delta_wall:+6.1f}%  "
            f"{conf_a:>9}  {conf_d:>9}  {work_pct:+6.1f}%  "
            f"{pps_a:>10.0f}  {pps_d:>10.0f}  {speed_pct:+6.1f}%  {net_pct:+6.1f}%"
        )

    print(f"\nPAR-2 A: {par2_a:.1f}")
    print(f"PAR-2 D: {par2_d:.1f}")
    delta_par2 = 100.0 * (par2_d - par2_a) / par2_a if par2_a > 0 else 0.0
    print(f"Δ PAR-2: {delta_par2:+.2f}%")

    # Reference comparison (kissat-latest) for any instance solver did SAT/UNSAT
    print("\n# Reference comparison (kissat-latest)")
    print("instance  wall_A  wall_D  wall_kissat  A-vs-ref  D-vs-ref")
    print("-" * 70)
    for inst in instances:
        if inst not in ref:
            continue
        rk = ref[inst]
        wall_a = parse_float(a[inst].get("time_s"))
        wall_d = parse_float(d[inst].get("time_s"))
        wall_k = parse_float(rk.get("time_s"))
        if wall_k <= 0:
            continue
        ratio_a = wall_a / wall_k
        ratio_d = wall_d / wall_k
        inst_short = inst.split("-", 1)[1] if "-" in inst else inst
        inst_short = inst_short[:30]
        print(
            f"{inst_short:<30}  {wall_a:>6.1f}  {wall_d:>6.1f}  "
            f"{wall_k:>6.1f}  {ratio_a:>5.1f}x  {ratio_d:>5.1f}x"
        )


if __name__ == "__main__":
    main()
