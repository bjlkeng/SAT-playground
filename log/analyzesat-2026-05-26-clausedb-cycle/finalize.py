#!/usr/bin/env python3
"""Finalize the analyzesat clausedb-cycle FINDINGS — emits the data tables that
plug into FINDINGS.md after the ablation finishes."""

import csv
import json
from pathlib import Path

SLUG_DIR = Path(__file__).resolve().parent
CONFIGS = [
    "A_baseline",
    "B_binary_fast",
    "C_lbd_tiered",
    "D_post_reset",
    "E_reuse_trail",
    "F_combined_kissat",
]
PROFILING = [
    "0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12",
    "3746303c659ef65aaa78f3b52cd5de49-6s299b685_Iter30",
    "46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized",
    "557d7d4db5399188f62bc39598c6d868-mp1-Nb7T46",
    "5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7",
    "663bb5659e42c2c75f74354f48895302-SCPC-500-13",
    "6832fe907740af686fde98518067ea3f-velev-pipe-sat-1.0-b7",
    "9af7646fc4a32c6f2744ddc0c4b654b7-brocard_problem_large",
    "ed6d842f96d10f3400bce251f9e95bfb-battleship-16-31-sat",
    "fab2022deb130fe3ad1136a5c71b4109-case9",
]
SHORT = {
    "0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12": "sudoku",
    "3746303c659ef65aaa78f3b52cd5de49-6s299b685_Iter30": "6s299b685",
    "46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized": "REGRandom",
    "557d7d4db5399188f62bc39598c6d868-mp1-Nb7T46": "mp1",
    "5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7": "Kakuro",
    "663bb5659e42c2c75f74354f48895302-SCPC-500-13": "SCPC",
    "6832fe907740af686fde98518067ea3f-velev-pipe-sat-1.0-b7": "velev",
    "9af7646fc4a32c6f2744ddc0c4b654b7-brocard_problem_large": "brocard",
    "ed6d842f96d10f3400bce251f9e95bfb-battleship-16-31-sat": "battleship",
    "fab2022deb130fe3ad1136a5c71b4109-case9": "case9",
}
TIMEOUT_S = 300.0


def load_csv(path):
    out = {}
    if not path.exists():
        return out
    with open(path) as f:
        for row in csv.DictReader(f):
            out[row["instance"]] = row
    return out


def load_stats(path):
    out = {}
    if not path.exists():
        return out
    with open(path) as f:
        for line in f:
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            out[obj.get("instance")] = obj
    return out


def main():
    results = {c: load_csv(SLUG_DIR / c / "results.csv") for c in CONFIGS}
    stats = {c: load_stats(SLUG_DIR / c / "stats.jsonl") for c in CONFIGS}

    # PAR-2 table
    print("## PAR-2 per config (300 s timeout, profiling suite)")
    print()
    print("| Config | Solved | Timeout | PAR-2 | Δ vs A % |")
    print("|---|---:|---:|---:|---:|")
    base_par2 = None
    for cfg in CONFIGS:
        s = 0
        t = 0
        timeouts = 0
        for inst in PROFILING:
            r = results[cfg].get(inst, {})
            res = r.get("result", "")
            try:
                w = float(r.get("time_s", TIMEOUT_S))
            except (ValueError, TypeError):
                w = TIMEOUT_S
            if res in ("SAT", "UNSAT"):
                s += 1
                t += w
            else:
                t += 2 * TIMEOUT_S
                timeouts += 1
        if base_par2 is None:
            base_par2 = t
        delta = (t - base_par2) / base_par2 * 100 if base_par2 else 0
        print(f"| {cfg} | {s} | {timeouts} | {t:.1f} | {delta:+.1f}% |")
    print()

    # Per-instance wall time
    print("## Per-instance wall time (s)")
    print()
    hdr = "| Instance |"
    sep = "|---|"
    for cfg in CONFIGS:
        hdr += f" {cfg.split('_',1)[1] if '_' in cfg else cfg} |"
        sep += "---:|"
    print(hdr)
    print(sep)
    for inst in PROFILING:
        line = f"| {SHORT[inst]} |"
        for cfg in CONFIGS:
            r = results[cfg].get(inst, {})
            res = r.get("result", "")
            try:
                w = float(r.get("time_s", TIMEOUT_S))
            except (ValueError, TypeError):
                w = TIMEOUT_S
            if res in ("SAT", "UNSAT"):
                line += f" {w:.1f} |"
            elif not res:
                line += " -- |"
            else:
                line += f" {res[:7]} |"
        print(line)
    print()

    # Work × speed decomposition
    print("## Work × Speed decomposition (vs A_baseline)")
    print()
    print("Legend: work = conflicts_cfg / conflicts_A, speed = (props/s)_A / (props/s)_cfg, net = work × speed (predicted wall ratio).")
    print()
    print("| Instance | Config | conflicts | props/s | work | speed | net | measured | dominant |")
    print("|---|---|---:|---:|---:|---:|---:|---:|---|")
    for inst in PROFILING:
        ba = stats["A_baseline"].get(inst, {})
        bc = ba.get("conflicts")
        bp = ba.get("propagations")
        bt = ba.get("bench_time_s")
        if not (bc and bp and bt and bt > 0):
            continue
        bpps = bp / bt
        for cfg in CONFIGS[1:]:
            s = stats[cfg].get(inst, {})
            if not s:
                continue
            c = s.get("conflicts")
            p = s.get("propagations")
            t = s.get("bench_time_s")
            if not (c and p and t and t > 0):
                continue
            pps = p / t
            work = c / bc
            speed = bpps / pps
            net = work * speed
            measured = t / bt
            if 0.90 < work < 1.10 and 0.90 < speed < 1.10:
                dom = "noise"
            elif abs(work - 1) > abs(speed - 1):
                dom = "work" if work < 1 else "WORK"
            else:
                dom = "speed" if speed < 1 else "SPEED"
            print(
                f"| {SHORT[inst]} | {cfg} | {c} | {pps:.0f} | {work:.2f} | {speed:.2f} | {net:.2f} | {measured:.2f} | {dom} |"
            )
        print(f"| _A_baseline_ | A_baseline | {bc} | {bpps:.0f} | 1.00 | 1.00 | 1.00 | 1.00 | -- |")
    print()


if __name__ == "__main__":
    main()
