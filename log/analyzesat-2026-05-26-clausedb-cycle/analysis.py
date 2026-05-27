#!/usr/bin/env python3
"""Analyze ablation matrix: build per-(config, instance) work × speed decomposition vs A_baseline."""
import csv
import json
import sys
from pathlib import Path

SLUG_DIR = Path(__file__).resolve().parent
CONFIGS = ["A_baseline", "B_binary_fast", "C_lbd_tiered", "D_post_reset", "E_reuse_trail", "F_combined_kissat"]
TIMEOUT_S = 300.0


def load_results(cfg):
    path = SLUG_DIR / cfg / "results.csv"
    if not path.exists():
        return {}
    rows = {}
    with path.open() as f:
        for row in csv.DictReader(f):
            rows[row["instance"]] = row
    return rows


def load_stats(cfg):
    """Stats per instance from stats.jsonl emitted by bench.sh."""
    path = SLUG_DIR / cfg / "stats.jsonl"
    out = {}
    if not path.exists():
        return out
    with path.open() as f:
        for line in f:
            try:
                payload = json.loads(line)
            except json.JSONDecodeError:
                continue
            instance = payload.get("instance")
            if instance is None:
                continue
            out[instance] = payload
    return out


def fmt(v):
    if v is None or v == "":
        return "--"
    try:
        x = float(v)
        if abs(x) >= 100:
            return f"{x:.1f}"
        if abs(x) >= 10:
            return f"{x:.2f}"
        return f"{x:.3f}"
    except (ValueError, TypeError):
        return str(v)


def main():
    results = {cfg: load_results(cfg) for cfg in CONFIGS}
    stats = {cfg: load_stats(cfg) for cfg in CONFIGS}

    if not results.get("A_baseline"):
        print("ERROR: A_baseline results.csv missing or empty")
        return 1

    instances = sorted(results["A_baseline"].keys())

    par2 = {}
    solved = {}
    for cfg in CONFIGS:
        s = 0
        p = 0.0
        for inst in instances:
            row = results[cfg].get(inst, {})
            result = row.get("result", "TIMEOUT")
            try:
                t = float(row.get("time_s", TIMEOUT_S))
            except (ValueError, TypeError):
                t = TIMEOUT_S
            if result in ("SAT", "UNSAT"):
                p += t
                s += 1
            else:
                p += 2 * TIMEOUT_S
        par2[cfg] = p
        solved[cfg] = s

    print("=" * 80)
    print("PAR-2 per config (300 s, profiling suite)")
    print("=" * 80)
    print(f"{'Config':<22}{'Solved':>8}{'PAR-2':>12}{'Δ vs A %':>14}")
    base = par2["A_baseline"]
    for cfg in CONFIGS:
        delta = (par2[cfg] - base) / base * 100 if base else 0
        print(f"{cfg:<22}{solved[cfg]:>8}{par2[cfg]:>12.1f}{delta:>13.1f}%")
    print()

    # Per-instance wall time table
    print("=" * 80)
    print("Per-instance wall time (s)")
    print("=" * 80)
    short = {inst: inst.split("-", 1)[1].split(".")[0][:30] for inst in instances}
    hdr = f"{'Instance':<32}"
    for cfg in CONFIGS:
        hdr += f"{cfg.split('_',1)[1][:11]:>13}"
    print(hdr)
    for inst in instances:
        row = f"{short[inst]:<32}"
        for cfg in CONFIGS:
            r = results[cfg].get(inst, {})
            t = r.get("time_s", "")
            res = r.get("result", "")
            if res in ("TIMEOUT", "UNKNOWN", "ERROR"):
                row += f"{res[:11]:>13}"
            else:
                row += f"{fmt(t):>13}"
        print(row)
    print()

    # Work × speed decomposition
    print("=" * 80)
    print("Work × Speed decomposition (vs A_baseline, work=conflicts ratio, speed=props/s ratio)")
    print("=" * 80)
    print(f"{'Instance':<32}{'Config':<20}{'work':>10}{'speed':>10}{'net':>10}{'measured':>10}")
    for inst in instances:
        base_stat = stats["A_baseline"].get(inst, {})
        base_confl = base_stat.get("conflicts")
        base_props = base_stat.get("propagations")
        base_time = base_stat.get("bench_time_s")
        if not (base_confl and base_props and base_time and base_time > 0):
            continue
        base_pps = base_props / base_time
        for cfg in CONFIGS[1:]:
            s = stats[cfg].get(inst, {})
            if not s:
                continue
            confl = s.get("conflicts")
            props = s.get("propagations")
            time = s.get("bench_time_s")
            if not (confl and props and time and time > 0):
                continue
            work = confl / base_confl if base_confl else 0
            speed = base_pps / (props / time)
            net = work * speed
            measured = time / base_time if base_time else 0
            print(
                f"{short[inst]:<32}{cfg:<20}{work:>10.2f}{speed:>10.2f}{net:>10.2f}{measured:>10.2f}"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main())
