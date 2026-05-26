#!/usr/bin/env python3
import csv
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent

TARGETS = {
    "mp1": [
        "A_default",
        "C_reason_lbd",
        "D_lbd_tiered",
        "I_lbd_tiered_no_reason",
        "G_lbd_tiered_delayed",
        "H_lbd_tiered_slow_interval",
    ],
    "regrandom": [
        "A_default",
        "D_lbd_tiered",
        "G_lbd_tiered_delayed",
        "H_lbd_tiered_slow_interval",
    ],
}

FIELDS = [
    "target",
    "config",
    "result",
    "time_s",
    "wall_ratio",
    "conflicts",
    "work_ratio",
    "decisions",
    "propagations",
    "props_per_s",
    "speed_ratio",
    "net_work_speed",
    "reductions",
    "learned_collected",
    "learned_clauses_final",
    "learned_lits_final",
    "gc_count",
    "gc_refs_rewritten",
    "gc_words_reclaimed",
    "proof_bytes_written",
    "max_rss_mb",
    "termination_reason",
    "unknown_reason",
]


def read_stats(target, config):
    path = ROOT / f"target-{target}-{config}" / "stats.jsonl"
    if not path.exists():
        return None
    lines = [line for line in path.read_text().splitlines() if line.strip()]
    if not lines:
        return None
    return json.loads(lines[-1])


def props_per_s(row):
    elapsed = row.get("elapsed_sec") or row.get("bench_time_s")
    if not elapsed:
        return None
    return row.get("propagations", 0) / elapsed


def ratio(lhs, rhs):
    if lhs is None or rhs in (None, 0):
        return None
    return lhs / rhs


def fmt(value, digits=3):
    if value is None:
        return ""
    if isinstance(value, float):
        return f"{value:.{digits}f}"
    return value


def main():
    rows = []
    for target, configs in TARGETS.items():
        base = read_stats(target, "A_default")
        if base is None:
            continue
        base_pps = props_per_s(base)
        for config in configs:
            row = read_stats(target, config)
            if row is None:
                continue
            pps = props_per_s(row)
            work = ratio(row.get("conflicts"), base.get("conflicts"))
            speed = ratio(base_pps, pps)
            rows.append({
                "target": target,
                "config": config,
                "result": row.get("result"),
                "time_s": fmt(row.get("bench_time_s")),
                "wall_ratio": fmt(ratio(row.get("bench_time_s"), base.get("bench_time_s"))),
                "conflicts": row.get("conflicts"),
                "work_ratio": fmt(work),
                "decisions": row.get("decisions"),
                "propagations": row.get("propagations"),
                "props_per_s": fmt(pps, 1),
                "speed_ratio": fmt(speed),
                "net_work_speed": fmt((work * speed) if work is not None and speed is not None else None),
                "reductions": row.get("reductions"),
                "learned_collected": row.get("learned_collected"),
                "learned_clauses_final": row.get("learned_clauses_final"),
                "learned_lits_final": row.get("learned_lits_final"),
                "gc_count": row.get("gc_count"),
                "gc_refs_rewritten": row.get("gc_refs_rewritten"),
                "gc_words_reclaimed": row.get("gc_words_reclaimed"),
                "proof_bytes_written": row.get("proof_bytes_written"),
                "max_rss_mb": row.get("max_rss_mb"),
                "termination_reason": row.get("termination_reason"),
                "unknown_reason": row.get("unknown_reason"),
            })

    with (ROOT / "target_summary.csv").open("w", newline="") as fh:
        writer = csv.DictWriter(fh, FIELDS)
        writer.writeheader()
        writer.writerows(rows)

    for row in rows:
        print(
            f"{row['target']:9s} {row['config']:28s} {row['result']:7s} "
            f"time={row['time_s']:>8s} wall={row['wall_ratio']:>6s} "
            f"work={row['work_ratio']:>6s} speed={row['speed_ratio']:>6s} "
            f"red={row['reductions']} del={row['learned_collected']} gc={row['gc_count']}"
        )


if __name__ == "__main__":
    main()
