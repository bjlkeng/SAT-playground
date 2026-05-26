#!/usr/bin/env python3
import csv
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent
TIMEOUT = 300.0

FULL_CONFIGS = ["A_default", "B_no_min"]
SUDOKU_RUNS = [
    ("default_recursive", "A_default", "0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12"),
    ("min_off", "B_no_min", "0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12"),
    ("basic", "S_basic_sudoku", "sudoku"),
    ("inblock", "S_inblock_sudoku", "sudoku"),
    ("recursive_depth1000", "S_depth1000_sudoku", "sudoku"),
    ("binary_fast_auto_min_off", "S_binary_fast_sudoku", "sudoku"),
    ("binary_fast_recursive", "S_binary_fast_min_sudoku", "sudoku"),
]


def read_results(config):
    rows = {}
    path = ROOT / config / "results.csv"
    if not path.exists():
        return rows
    with path.open(newline="") as f:
        for row in csv.DictReader(f):
            row["time_s"] = float(row["time_s"])
            rows[row["instance"]] = row
    return rows


def read_stats(config):
    rows = {}
    path = ROOT / config / "stats.jsonl"
    if not path.exists():
        return rows
    with path.open() as f:
        for line in f:
            if line.strip():
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


def prop_rate(stats):
    search = float(stats.get("search_sec") or 0.0)
    props = float(stats.get("propagations") or 0.0)
    return props / search if search else 0.0


def par2_time(result):
    return result["time_s"] if result["result"] in {"SAT", "UNSAT"} else 2.0 * TIMEOUT


def write_csv(name, fields, rows):
    with (ROOT / name).open("w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def full_config_summary():
    rows = []
    fields = [
        "config",
        "rows_completed",
        "solved",
        "unknown",
        "par2_on_completed_rows",
        "wall_s_on_completed_rows",
        "unknowns",
    ]
    for config in FULL_CONFIGS:
        results = read_results(config)
        solved = sum(1 for row in results.values() if row["result"] in {"SAT", "UNSAT"})
        unknowns = sorted(name for name, row in results.items() if row["result"] == "UNKNOWN")
        rows.append({
            "config": config,
            "rows_completed": len(results),
            "solved": solved,
            "unknown": len(unknowns),
            "par2_on_completed_rows": flt(sum(par2_time(row) for row in results.values())),
            "wall_s_on_completed_rows": flt(sum(row["time_s"] for row in results.values())),
            "unknowns": ";".join(unknowns),
        })
    write_csv("config_summary.csv", fields, rows)


def sudoku_summary():
    baseline_stats = read_stats("A_default")["0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12"]
    baseline_result = read_results("A_default")["0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12"]
    b_conf = float(baseline_stats.get("conflicts") or 0)
    b_rate = prop_rate(baseline_stats)
    b_time = baseline_result["time_s"]
    b_learned_lits = float(baseline_stats.get("learned_lits_final") or 0)

    fields = [
        "run",
        "result",
        "time_s",
        "actual_wall_ratio",
        "conflicts",
        "work_ratio_conflicts",
        "propagations",
        "prop_per_search_s",
        "speed_ratio_vs_default",
        "net_work_speed_ratio",
        "decisions",
        "restarts",
        "learned_clauses_final",
        "learned_lits_final",
        "learned_lits_ratio",
        "max_clause_buffer_len",
        "proof_added_literals",
        "proof_bytes_written",
        "limit_hit",
        "termination_reason",
        "unknown_reason",
    ]
    rows = []
    for label, config, instance in SUDOKU_RUNS:
        result = read_results(config)[instance]
        stats = read_stats(config)[instance]
        conf = float(stats.get("conflicts") or 0)
        rate = prop_rate(stats)
        work_ratio = conf / b_conf if b_conf else None
        speed_ratio = b_rate / rate if b_rate and rate else None
        net = work_ratio * speed_ratio if work_ratio is not None and speed_ratio is not None else None
        learned_lits = float(stats.get("learned_lits_final") or 0)
        rows.append({
            "run": label,
            "result": result["result"],
            "time_s": flt(result["time_s"]),
            "actual_wall_ratio": flt(result["time_s"] / b_time if b_time else None),
            "conflicts": ints(stats.get("conflicts")),
            "work_ratio_conflicts": flt(work_ratio),
            "propagations": ints(stats.get("propagations")),
            "prop_per_search_s": flt(rate),
            "speed_ratio_vs_default": flt(speed_ratio),
            "net_work_speed_ratio": flt(net),
            "decisions": ints(stats.get("decisions")),
            "restarts": ints(stats.get("restarts")),
            "learned_clauses_final": ints(stats.get("learned_clauses_final")),
            "learned_lits_final": ints(stats.get("learned_lits_final")),
            "learned_lits_ratio": flt(learned_lits / b_learned_lits if b_learned_lits else None),
            "max_clause_buffer_len": ints(stats.get("max_clause_buffer_len")),
            "proof_added_literals": ints(stats.get("proof_added_literals")),
            "proof_bytes_written": ints(stats.get("proof_bytes_written")),
            "limit_hit": str(bool(stats.get("limit_hit"))).lower(),
            "termination_reason": stats.get("termination_reason") or "",
            "unknown_reason": stats.get("unknown_reason") or "",
        })
    write_csv("sudoku_clause_min_summary.csv", fields, rows)


def reference_summary():
    fields = ["solver", "instance", "result", "time_s", "exit_code"]
    rows = []
    with (ROOT / "reference_sudoku.csv").open(newline="") as f:
        for row in csv.DictReader(f):
            rows.append(row)
    write_csv("reference_summary.csv", fields, rows)


def main():
    full_config_summary()
    sudoku_summary()
    reference_summary()


if __name__ == "__main__":
    main()
