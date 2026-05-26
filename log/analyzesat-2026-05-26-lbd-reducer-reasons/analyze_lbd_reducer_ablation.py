#!/usr/bin/env python3
import csv
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parent
BASE = "A_default"
TIMEOUT = 300.0
CONFIG_ORDER = [
    "A_default",
    "B_lbd_metadata",
    "C_reason_lbd",
    "D_lbd_tiered",
    "E_lbd_tiered_prop_reasons",
    "F_lbd_tiered_reset",
    "G_lbd_tiered_delayed",
    "H_lbd_tiered_slow_interval",
]


def read_results(config):
    path = ROOT / config / "results.csv"
    rows = {}
    if not path.exists():
        return rows
    with path.open(newline="") as fh:
        for row in csv.DictReader(fh):
            row["time_s"] = float(row["time_s"])
            rows[row["instance"]] = row
    return rows


def read_stats(config):
    path = ROOT / config / "stats.jsonl"
    rows = {}
    if not path.exists():
        return rows
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        rows[row["instance"]] = row
    return rows


def par2(rows):
    total = 0.0
    solved = 0
    sat = 0
    unsat = 0
    unknown = 0
    timeout = 0
    error = 0
    for row in rows.values():
        result = row["result"]
        if result in ("SAT", "UNSAT"):
            solved += 1
            sat += result == "SAT"
            unsat += result == "UNSAT"
            total += row["time_s"]
        else:
            unknown += result == "UNKNOWN"
            timeout += result == "TIMEOUT"
            error += result not in ("UNKNOWN", "TIMEOUT")
            total += 2 * TIMEOUT
    return {
        "par2": total,
        "solved": solved,
        "sat": sat,
        "unsat": unsat,
        "unknown": unknown,
        "timeout": timeout,
        "error": error,
    }


def num(row, key):
    value = row.get(key)
    if value is None:
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def ratio(a, b):
    if a is None or b is None or b == 0:
        return None
    return a / b


def props_per_s(row):
    props = num(row, "propagations")
    elapsed = num(row, "elapsed_wall_sec") or num(row, "bench_time_s")
    if props is None or elapsed is None or elapsed <= 0:
        return None
    return props / elapsed


def fmt(value, digits=3):
    if value is None or (isinstance(value, float) and (math.isnan(value) or math.isinf(value))):
        return ""
    return f"{value:.{digits}f}"


def main():
    results = {cfg: read_results(cfg) for cfg in CONFIG_ORDER}
    configs = [cfg for cfg in CONFIG_ORDER if results[cfg]]
    stats = {cfg: read_stats(cfg) for cfg in CONFIG_ORDER}
    base_results = results[BASE]
    base_stats = stats[BASE]

    summary_rows = []
    for cfg in configs:
        s = par2(results[cfg])
        s["config"] = cfg
        s["delta_par2"] = s["par2"] - par2(base_results)["par2"]
        summary_rows.append(s)

    with (ROOT / "summary_table.csv").open("w", newline="") as fh:
        fields = ["config", "solved", "sat", "unsat", "unknown", "timeout", "error", "par2", "delta_par2"]
        writer = csv.DictWriter(fh, fields)
        writer.writeheader()
        writer.writerows(summary_rows)

    detail_fields = [
        "config",
        "instance",
        "base_result",
        "result",
        "base_time_s",
        "time_s",
        "wall_ratio",
        "base_conflicts",
        "conflicts",
        "work_ratio",
        "base_props_per_s",
        "props_per_s",
        "speed_ratio",
        "net_work_speed",
        "base_decisions",
        "decisions",
        "base_propagations",
        "propagations",
        "base_reduce_db_calls",
        "reduce_db_calls",
        "base_learned_clauses_final",
        "learned_clauses_final",
        "base_learned_lits_final",
        "learned_lits_final",
        "base_learned_collected",
        "learned_collected",
        "base_gc_count",
        "gc_count",
        "base_max_rss_kb",
        "max_rss_kb",
    ]
    detail = []
    for cfg in configs:
        if cfg == BASE:
            continue
        for inst, base_row in sorted(base_results.items()):
            row = results[cfg].get(inst)
            if row is None:
                continue
            bstat = base_stats.get(inst, {})
            cstat = stats[cfg].get(inst, {})
            base_pps = props_per_s(bstat)
            cfg_pps = props_per_s(cstat)
            work = ratio(num(cstat, "conflicts"), num(bstat, "conflicts"))
            speed = ratio(base_pps, cfg_pps)
            detail.append({
                "config": cfg,
                "instance": inst,
                "base_result": base_row["result"],
                "result": row["result"],
                "base_time_s": fmt(base_row["time_s"]),
                "time_s": fmt(row["time_s"]),
                "wall_ratio": fmt(ratio(row["time_s"], base_row["time_s"])),
                "base_conflicts": fmt(num(bstat, "conflicts"), 0),
                "conflicts": fmt(num(cstat, "conflicts"), 0),
                "work_ratio": fmt(work),
                "base_props_per_s": fmt(base_pps, 1),
                "props_per_s": fmt(cfg_pps, 1),
                "speed_ratio": fmt(speed),
                "net_work_speed": fmt((work * speed) if work is not None and speed is not None else None),
                "base_decisions": fmt(num(bstat, "decisions"), 0),
                "decisions": fmt(num(cstat, "decisions"), 0),
                "base_propagations": fmt(num(bstat, "propagations"), 0),
                "propagations": fmt(num(cstat, "propagations"), 0),
                "base_reduce_db_calls": fmt(num(bstat, "reduce_db_calls"), 0),
                "reduce_db_calls": fmt(num(cstat, "reduce_db_calls"), 0),
                "base_learned_clauses_final": fmt(num(bstat, "learned_clauses_final"), 0),
                "learned_clauses_final": fmt(num(cstat, "learned_clauses_final"), 0),
                "base_learned_lits_final": fmt(num(bstat, "learned_lits_final"), 0),
                "learned_lits_final": fmt(num(cstat, "learned_lits_final"), 0),
                "base_learned_collected": fmt(num(bstat, "learned_collected"), 0),
                "learned_collected": fmt(num(cstat, "learned_collected"), 0),
                "base_gc_count": fmt(num(bstat, "gc_count"), 0),
                "gc_count": fmt(num(cstat, "gc_count"), 0),
                "base_max_rss_kb": fmt(num(bstat, "max_rss_kb"), 0),
                "max_rss_kb": fmt(num(cstat, "max_rss_kb"), 0),
            })

    with (ROOT / "work_speed_detail.csv").open("w", newline="") as fh:
        writer = csv.DictWriter(fh, detail_fields)
        writer.writeheader()
        writer.writerows(detail)

    print("Summary")
    for row in summary_rows:
        print(
            f"{row['config']:32s} solved={row['solved']:2d}/10 "
            f"u/t/e={row['unknown']}/{row['timeout']}/{row['error']} "
            f"PAR2={row['par2']:.3f} delta={row['delta_par2']:+.3f}"
        )

    print("\nLargest solved-row wall regressions vs A_default")
    solved_detail = [
        row for row in detail
        if row["base_result"] in ("SAT", "UNSAT")
        and row["result"] in ("SAT", "UNSAT")
        and row["wall_ratio"]
    ]
    solved_detail.sort(key=lambda r: float(r["wall_ratio"]), reverse=True)
    for row in solved_detail[:20]:
        print(
            f"{row['config']:32s} {row['instance']:60s} "
            f"wall={row['wall_ratio']} work={row['work_ratio']} speed={row['speed_ratio']} "
            f"time={row['base_time_s']}->{row['time_s']} reduce={row['base_reduce_db_calls']}->{row['reduce_db_calls']}"
        )


if __name__ == "__main__":
    main()
