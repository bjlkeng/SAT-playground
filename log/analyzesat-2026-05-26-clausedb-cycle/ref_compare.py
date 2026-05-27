#!/usr/bin/env python3
"""Build per-instance kissat-vs-solver-11-A_baseline gap table."""
import csv
import json
from pathlib import Path

SLUG_DIR = Path(__file__).resolve().parent
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


def load_csv_first(path):
    """Return dict of first occurrence per instance."""
    out = {}
    with open(path) as f:
        for row in csv.DictReader(f):
            inst = row["instance"]
            if inst not in out:
                out[inst] = row
    return out


def load_stats(path):
    out = {}
    with open(path) as f:
        for line in f:
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            out[obj.get("instance")] = obj
    return out


def main():
    kl = load_csv_first(SLUG_DIR / "reference-kissat-latest.csv")
    ks = load_csv_first(SLUG_DIR / "reference-kissat-sc2024.csv")
    a_res = load_csv_first(SLUG_DIR / "A_baseline" / "results.csv")
    a_stats = load_stats(SLUG_DIR / "A_baseline" / "stats.jsonl")

    print(f"{'Instance':<40}{'A':>10}{'kissat-l':>10}{'kissat-sc':>10}{'win':>10}")
    print("-" * 80)
    for inst in PROFILING:
        a = a_res.get(inst, {})
        klr = kl.get(inst, {})
        ksr = ks.get(inst, {})
        a_t = a.get("time_s", "?")
        kl_t = klr.get("time_s", "?")
        ks_t = ksr.get("time_s", "?")
        a_r = a.get("result", "?")
        try:
            best_ref = min(float(kl_t), float(ks_t))
            ratio = float(a_t) / best_ref if a_r in ("SAT", "UNSAT") and best_ref > 0 else None
        except (ValueError, TypeError):
            ratio = None
        short = inst.split("-", 1)[1].split(".")[0][:38]
        ratio_str = f"{ratio:.1f}×" if ratio is not None else "TO" if a_r == "TIMEOUT" else "?"
        print(f"{short:<40}{a_t:>10}{kl_t:>10}{ks_t:>10}{ratio_str:>10}")

    print()
    print("Per-instance solver-11 stats (work counters):")
    print(f"{'Instance':<40}{'conflicts':>12}{'props':>14}{'props/s':>14}{'restarts':>10}")
    print("-" * 90)
    for inst in PROFILING:
        s = a_stats.get(inst, {})
        if not s:
            continue
        c = s.get("conflicts", 0)
        p = s.get("propagations", 0)
        t = s.get("bench_time_s", 1)
        r = s.get("restarts", 0)
        pps = p / t if t else 0
        short = inst.split("-", 1)[1].split(".")[0][:38]
        print(f"{short:<40}{c:>12}{p:>14}{pps:>14.0f}{r:>10}")


if __name__ == "__main__":
    main()
