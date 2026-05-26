#!/usr/bin/env python3
import csv
import json
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent
TIMEOUT = 300.0
SOLVED = {"SAT", "UNSAT"}


def load_matrix():
    with (ROOT / "matrix_results.csv").open(newline="") as f:
        return list(csv.DictReader(f))


def load_stats():
    stats = {}
    with (ROOT / "all_stats.jsonl").open() as f:
        for line in f:
            if not line.strip():
                continue
            rec = json.loads(line)
            stats[(rec["config"], rec["instance"])] = rec
    return stats


def par2(rows):
    total = 0.0
    for row in rows:
        if row["result"] in SOLVED:
            total += float(row["time_s"])
        else:
            total += 2.0 * float(row["timeout"] or TIMEOUT)
    return total


def prop_rate(rec):
    props = float(rec.get("propagations") or 0.0)
    sec = float(rec.get("search_sec") or rec.get("elapsed_sec") or 0.0)
    return props / sec if props > 0 and sec > 0 else None


def ratio(num, den):
    if den in (0, None) or num is None:
        return None
    return num / den


def fmt(value, digits=3):
    if value is None:
        return "NA"
    return f"{value:.{digits}f}"


def main():
    matrix = load_matrix()
    stats = load_stats()
    by_config = defaultdict(list)
    for row in matrix:
        by_config[row["config"]].append(row)

    default_rows = {row["instance"]: row for row in by_config["default"]}
    default_stats = {inst: stats[("default", inst)] for inst in default_rows}

    lines = []
    lines.append("# AnalyzeSAT Preprocess/Order Matrix")
    lines.append("")
    lines.append("## Config Summary")
    lines.append("")
    lines.append("| config | rows | solved | status regressions | PAR-2 on measured rows | notes |")
    lines.append("|---|---:|---:|---:|---:|---|")
    for config, rows in by_config.items():
        solved = sum(1 for r in rows if r["result"] in SOLVED)
        regressions = sum(1 for r in rows if r["stopped_after_regression"] == "1")
        notes = []
        if any(r["stopped_after_regression"] == "1" for r in rows):
            bad = [r for r in rows if r["stopped_after_regression"] == "1"][0]
            notes.append(f"stopped at {bad['instance']} ({bad['baseline_result']} -> {bad['result']})")
        if config == "proof_off":
            notes.append("diagnostic only: UNSAT rows violate proof requirement")
        lines.append(
            f"| {config} | {len(rows)} | {solved} | {regressions} | {par2(rows):.3f} | {'; '.join(notes)} |"
        )

    lines.append("")
    lines.append("## Full-Suite Deltas vs Default")
    lines.append("")
    lines.append("| config | PAR-2 | delta vs default | solved | largest win | largest loss |")
    lines.append("|---|---:|---:|---:|---|---|")
    default_par2 = par2(by_config["default"])
    for config in ("input_order", "raw_order", "proof_off"):
        rows = by_config[config]
        wins = []
        losses = []
        for row in rows:
            base = default_rows[row["instance"]]
            delta = float(row["time_s"]) - float(base["time_s"]) if row["result"] in SOLVED else 2 * TIMEOUT - float(base["time_s"])
            item = (delta, row["instance"])
            if delta < 0:
                wins.append(item)
            elif delta > 0:
                losses.append(item)
        best = min(wins, default=(0.0, "none"))
        worst = max(losses, default=(0.0, "none"))
        lines.append(
            f"| {config} | {par2(rows):.3f} | {par2(rows) - default_par2:+.3f} | {sum(1 for r in rows if r['result'] in SOLVED)}/10 | "
            f"{best[1]} {best[0]:+.3f}s | {worst[1]} {worst[0]:+.3f}s |"
        )

    lines.append("")
    lines.append("## Work x Speed Decomposition")
    lines.append("")
    lines.append(
        "work_ratio = candidate conflicts / default conflicts. "
        "speed_ratio = default propagation throughput / candidate propagation throughput, using search_sec. "
        "net = work_ratio * speed_ratio; wall_ratio = candidate bench time / default bench time."
    )
    lines.append("")
    lines.append("| config | instance | wall | work | speed | net | conflicts default -> cfg | props/s default -> cfg |")
    lines.append("|---|---|---:|---:|---:|---:|---:|---:|")
    interesting = [
        ("input_order", "5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7"),
        ("raw_order", "5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7"),
        ("input_order", "6832fe907740af686fde98518067ea3f-velev-pipe-sat-1.0-b7"),
        ("raw_order", "6832fe907740af686fde98518067ea3f-velev-pipe-sat-1.0-b7"),
        ("no_full_bsr", "46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized"),
        ("proof_off", "0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12"),
        ("proof_off", "46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized"),
    ]
    rows_by_key = {(r["config"], r["instance"]): r for r in matrix}
    for config, inst in interesting:
        if (config, inst) not in rows_by_key or (config, inst) not in stats:
            continue
        row = rows_by_key[(config, inst)]
        base_row = default_rows[inst]
        rec = stats[(config, inst)]
        base = default_stats[inst]
        work = ratio(float(rec.get("conflicts") or 0), float(base.get("conflicts") or 0))
        b_rate = prop_rate(base)
        c_rate = prop_rate(rec)
        speed = ratio(b_rate, c_rate)
        net = work * speed if work is not None and speed is not None else None
        wall = float(row["time_s"]) / float(base_row["time_s"])
        lines.append(
            f"| {config} | {inst} | {fmt(wall)} | {fmt(work)} | {fmt(speed)} | {fmt(net)} | "
            f"{int(base.get('conflicts') or 0)} -> {int(rec.get('conflicts') or 0)} | "
            f"{fmt(b_rate, 0)} -> {fmt(c_rate, 0)} |"
        )

    lines.append("")
    lines.append("## Preprocess Counters")
    lines.append("")
    lines.append("| config | instance | preprocess_s | search_s | bve_vars | bsr_subsumed | final_original_clauses | final_original_lits | proof_bytes |")
    lines.append("|---|---|---:|---:|---:|---:|---:|---:|---:|")
    for config, inst in [
        ("default", "5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7"),
        ("input_order", "5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7"),
        ("raw_order", "5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7"),
        ("default", "46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized"),
        ("no_full_bsr", "46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized"),
        ("proof_off", "0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12"),
    ]:
        rec = stats.get((config, inst))
        if not rec:
            continue
        lines.append(
            f"| {config} | {inst} | {fmt(rec.get('preprocess_sec'))} | {fmt(rec.get('search_sec'))} | "
            f"{int(rec.get('pre_bve_eliminated_vars') or 0)} | {int(rec.get('pre_bsr_subsumed') or 0)} | "
            f"{int(rec.get('original_clauses_after_preprocess') or 0)} | {int(rec.get('original_lits_after_preprocess') or 0)} | "
            f"{int(rec.get('proof_bytes_written') or 0)} |"
        )

    out = ROOT / "analysis.md"
    out.write_text("\n".join(lines) + "\n")
    print(out)


if __name__ == "__main__":
    main()
