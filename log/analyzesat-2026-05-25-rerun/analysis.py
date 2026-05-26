#!/usr/bin/env python3
import csv
import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parent
TIMEOUT = 300.0
CONFIGS = ["A_default", "A_lucky_off", "B_lbd_metadata", "C_lbd_ema"]
SUMMARY_FIELDS = [
    "config",
    "solved",
    "total",
    "par2",
    "wall_s",
    "conflicts",
    "decisions",
    "propagations",
    "restarts",
    "prop_per_search_s",
    "lucky_attempts",
    "lucky_solved",
    "unknowns",
]
INSTANCE_FIELDS = [
    "config",
    "instance",
    "result",
    "time_s",
    "verified",
    "conflicts",
    "decisions",
    "propagations",
    "restarts",
    "search_sec",
    "prop_per_search_s",
    "lucky_attempts",
    "lucky_solved",
    "unknown_reason",
]


def read_results(config):
    path = ROOT / config / "results.csv"
    rows = {}
    with path.open(newline="") as f:
        for row in csv.DictReader(f):
            row["time_s"] = float(row["time_s"])
            row["timeout"] = float(row["timeout"])
            rows[row["instance"]] = row
    return rows


def read_stats(config):
    path = ROOT / config / "stats.jsonl"
    rows = {}
    with path.open() as f:
        for line in f:
            if not line.strip():
                continue
            row = json.loads(line)
            rows[row["instance"]] = row
    return rows


def flt(value):
    if value is None or value == "":
        return ""
    return f"{float(value):.6f}"


def ints(value):
    if value is None or value == "":
        return ""
    return str(int(value))


def par2_time(row):
    return row["time_s"] if row["result"] in {"SAT", "UNSAT"} else 2.0 * TIMEOUT


def prop_rate(stats):
    search = float(stats.get("search_sec") or 0.0)
    props = float(stats.get("propagations") or 0.0)
    return props / search if search > 0.0 else 0.0


def write_csv(path, fields, rows):
    with path.open("w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def build_config_tables():
    all_results = {config: read_results(config) for config in CONFIGS}
    all_stats = {config: read_stats(config) for config in CONFIGS}

    summary = []
    instance_rows = []
    for config in CONFIGS:
        results = all_results[config]
        stats = all_stats[config]
        solved = sum(1 for row in results.values() if row["result"] in {"SAT", "UNSAT"})
        par2 = sum(par2_time(row) for row in results.values())
        unknowns = sorted(name for name, row in results.items() if row["result"] == "UNKNOWN")
        totals = {
            "conflicts": sum(int(stats[name].get("conflicts") or 0) for name in results),
            "decisions": sum(int(stats[name].get("decisions") or 0) for name in results),
            "propagations": sum(int(stats[name].get("propagations") or 0) for name in results),
            "restarts": sum(int(stats[name].get("restarts") or 0) for name in results),
            "search_sec": sum(float(stats[name].get("search_sec") or 0.0) for name in results),
            "lucky_attempts": sum(int(stats[name].get("lucky_attempts") or 0) for name in results),
            "lucky_solved": sum(int(stats[name].get("lucky_solved") or 0) for name in results),
        }
        summary.append({
            "config": config,
            "solved": solved,
            "total": len(results),
            "par2": flt(par2),
            "wall_s": flt(sum(row["time_s"] for row in results.values())),
            "conflicts": ints(totals["conflicts"]),
            "decisions": ints(totals["decisions"]),
            "propagations": ints(totals["propagations"]),
            "restarts": ints(totals["restarts"]),
            "prop_per_search_s": flt(totals["propagations"] / totals["search_sec"] if totals["search_sec"] else 0.0),
            "lucky_attempts": ints(totals["lucky_attempts"]),
            "lucky_solved": ints(totals["lucky_solved"]),
            "unknowns": ";".join(unknowns),
        })

        for instance, result in sorted(results.items()):
            stat = stats[instance]
            instance_rows.append({
                "config": config,
                "instance": instance,
                "result": result["result"],
                "time_s": flt(result["time_s"]),
                "verified": result["verified"],
                "conflicts": ints(stat.get("conflicts")),
                "decisions": ints(stat.get("decisions")),
                "propagations": ints(stat.get("propagations")),
                "restarts": ints(stat.get("restarts")),
                "search_sec": flt(stat.get("search_sec")),
                "prop_per_search_s": flt(prop_rate(stat)),
                "lucky_attempts": ints(stat.get("lucky_attempts")),
                "lucky_solved": ints(stat.get("lucky_solved")),
                "unknown_reason": stat.get("unknown_reason") or "",
            })

    write_csv(ROOT / "config_summary.csv", SUMMARY_FIELDS, summary)
    write_csv(ROOT / "config_instance_summary.csv", INSTANCE_FIELDS, instance_rows)
    return all_results, all_stats


def build_decomposition(all_results, all_stats):
    baseline = "A_default"
    fields = [
        "config",
        "instance",
        "baseline_result",
        "config_result",
        "baseline_time_s",
        "config_time_s",
        "actual_wall_ratio",
        "work_ratio_conflicts",
        "speed_ratio_prop_rate",
        "net_work_speed_ratio",
        "baseline_conflicts",
        "config_conflicts",
        "baseline_prop_per_search_s",
        "config_prop_per_search_s",
        "baseline_restarts",
        "config_restarts",
    ]
    rows = []
    for config in CONFIGS:
        if config == baseline:
            continue
        for instance in sorted(all_results[baseline]):
            b_result = all_results[baseline][instance]
            c_result = all_results[config][instance]
            b_stats = all_stats[baseline][instance]
            c_stats = all_stats[config][instance]
            b_conf = float(b_stats.get("conflicts") or 0.0)
            c_conf = float(c_stats.get("conflicts") or 0.0)
            b_rate = prop_rate(b_stats)
            c_rate = prop_rate(c_stats)
            work_ratio = c_conf / b_conf if b_conf > 0.0 else None
            speed_ratio = b_rate / c_rate if b_rate > 0.0 and c_rate > 0.0 else None
            net = work_ratio * speed_ratio if work_ratio is not None and speed_ratio is not None else None
            rows.append({
                "config": config,
                "instance": instance,
                "baseline_result": b_result["result"],
                "config_result": c_result["result"],
                "baseline_time_s": flt(b_result["time_s"]),
                "config_time_s": flt(c_result["time_s"]),
                "actual_wall_ratio": flt(c_result["time_s"] / b_result["time_s"] if b_result["time_s"] else None),
                "work_ratio_conflicts": flt(work_ratio),
                "speed_ratio_prop_rate": flt(speed_ratio),
                "net_work_speed_ratio": flt(net),
                "baseline_conflicts": ints(b_stats.get("conflicts")),
                "config_conflicts": ints(c_stats.get("conflicts")),
                "baseline_prop_per_search_s": flt(b_rate),
                "config_prop_per_search_s": flt(c_rate),
                "baseline_restarts": ints(b_stats.get("restarts")),
                "config_restarts": ints(c_stats.get("restarts")),
            })
    write_csv(ROOT / "decomposition.csv", fields, rows)


def build_lucky_delta(all_results, all_stats):
    fields = [
        "instance",
        "default_result",
        "lucky_off_result",
        "default_time_s",
        "lucky_off_time_s",
        "default_minus_lucky_off_s",
        "default_lucky_attempts",
        "default_lucky_solved",
        "lucky_off_lucky_attempts",
        "lucky_off_lucky_solved",
    ]
    rows = []
    for instance in sorted(all_results["A_default"]):
        d = all_results["A_default"][instance]
        o = all_results["A_lucky_off"][instance]
        ds = all_stats["A_default"][instance]
        os = all_stats["A_lucky_off"][instance]
        rows.append({
            "instance": instance,
            "default_result": d["result"],
            "lucky_off_result": o["result"],
            "default_time_s": flt(d["time_s"]),
            "lucky_off_time_s": flt(o["time_s"]),
            "default_minus_lucky_off_s": flt(d["time_s"] - o["time_s"]),
            "default_lucky_attempts": ints(ds.get("lucky_attempts")),
            "default_lucky_solved": ints(ds.get("lucky_solved")),
            "lucky_off_lucky_attempts": ints(os.get("lucky_attempts")),
            "lucky_off_lucky_solved": ints(os.get("lucky_solved")),
        })
    write_csv(ROOT / "lucky_delta.csv", fields, rows)


TRACE_RE = re.compile(
    r"^c search seconds=(?P<seconds>[0-9.]+) conflicts=(?P<conflicts>[0-9]+) "
    r"decisions=(?P<decisions>[0-9]+) propagations=(?P<propagations>[0-9]+) "
    r"restarts=(?P<restarts>[0-9]+) level=(?P<level>[0-9]+) "
    r"trail=(?P<trail>[0-9]+) learned=(?P<learned>[0-9]+) reduce_db=(?P<reduce_db>[0-9]+)"
)


def build_trace_summary():
    fields = [
        "trace",
        "kind",
        "seconds",
        "conflicts",
        "decisions",
        "propagations",
        "restarts",
        "props_per_conflict",
        "decisions_per_conflict",
        "restarts_per_1000_conflicts",
        "level",
        "trail",
        "learned",
        "result",
    ]
    rows = []
    for trace in ["trace_A_default_mp1", "trace_C_lbd_ema_mp1"]:
        with (ROOT / f"{trace}.stderr").open() as f:
            for line in f:
                match = TRACE_RE.match(line.strip())
                if match:
                    row = {k: float(v) if k == "seconds" else int(v) for k, v in match.groupdict().items()}
                    conflicts = row["conflicts"]
                    rows.append({
                        "trace": trace,
                        "kind": "interval",
                        "seconds": flt(row["seconds"]),
                        "conflicts": ints(row["conflicts"]),
                        "decisions": ints(row["decisions"]),
                        "propagations": ints(row["propagations"]),
                        "restarts": ints(row["restarts"]),
                        "props_per_conflict": flt(row["propagations"] / conflicts if conflicts else 0.0),
                        "decisions_per_conflict": flt(row["decisions"] / conflicts if conflicts else 0.0),
                        "restarts_per_1000_conflicts": flt(row["restarts"] * 1000.0 / conflicts if conflicts else 0.0),
                        "level": ints(row["level"]),
                        "trail": ints(row["trail"]),
                        "learned": ints(row["learned"]),
                        "result": "",
                    })
                    continue
                if "c JSON_STATS " in line:
                    stats = json.loads(line.split("c JSON_STATS ", 1)[1])
                    conflicts = int(stats.get("conflicts") or 0)
                    rows.append({
                        "trace": trace,
                        "kind": "final",
                        "seconds": flt(stats.get("search_sec")),
                        "conflicts": ints(conflicts),
                        "decisions": ints(stats.get("decisions")),
                        "propagations": ints(stats.get("propagations")),
                        "restarts": ints(stats.get("restarts")),
                        "props_per_conflict": flt((stats.get("propagations") or 0) / conflicts if conflicts else 0.0),
                        "decisions_per_conflict": flt((stats.get("decisions") or 0) / conflicts if conflicts else 0.0),
                        "restarts_per_1000_conflicts": flt((stats.get("restarts") or 0) * 1000.0 / conflicts if conflicts else 0.0),
                        "level": "",
                        "trail": "",
                        "learned": ints(stats.get("learned_clauses_final")),
                        "result": stats.get("result") or "",
                    })
    write_csv(ROOT / "trace_summary_mp1.csv", fields, rows)


def build_reference_summary():
    fields = ["solver", "instance", "result", "time_s", "exit_code"]
    rows = []
    for solver in ["kissat-latest", "kissat-sc2024"]:
        path = ROOT / f"reference-failures-{solver}.csv"
        with path.open(newline="") as f:
            for row in csv.DictReader(f):
                rows.append({
                    "solver": solver,
                    "instance": row["instance"],
                    "result": row["result"],
                    "time_s": flt(row["time_s"]),
                    "exit_code": row["exit_code"],
                })
    write_csv(ROOT / "reference_failure_summary.csv", fields, rows)


def main():
    all_results, all_stats = build_config_tables()
    build_decomposition(all_results, all_stats)
    build_lucky_delta(all_results, all_stats)
    build_trace_summary()
    build_reference_summary()


if __name__ == "__main__":
    main()
