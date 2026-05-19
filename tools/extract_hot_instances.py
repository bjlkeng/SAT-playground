#!/usr/bin/env python3
"""Extract slow, regressed, or newly interesting instances from benchmark CSVs."""

from __future__ import annotations

import argparse
import csv
import sys
from pathlib import Path


def read_results(path: Path) -> dict[str, dict[str, str]]:
    with path.open(newline="") as handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames is None:
            raise SystemExit(f"{path}: missing CSV header")
        required = {"instance", "result", "time_s"}
        missing = required.difference(reader.fieldnames)
        if missing:
            raise SystemExit(f"{path}: missing columns: {', '.join(sorted(missing))}")
        return {
            row["instance"].strip(): {key: (value or "").strip() for key, value in row.items()}
            for row in reader
            if row.get("instance", "").strip()
        }


def as_float(value: str) -> float:
    try:
        return float(value)
    except ValueError:
        return 0.0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", required=True, type=Path)
    parser.add_argument("--before", type=Path)
    parser.add_argument("--top", type=int, default=10)
    parser.add_argument("--include-solved", action="store_true")
    args = parser.parse_args()

    rows = read_results(args.results)
    before = read_results(args.before) if args.before else {}
    ranked: list[tuple[float, str, str]] = []
    for name, row in rows.items():
        result = row["result"]
        elapsed = as_float(row["time_s"])
        if before and name in before:
            delta = elapsed - as_float(before[name]["time_s"])
            reason = f"delta_s={delta:.3f} before={before[name]['result']} after={result}"
            score = delta
        else:
            reason = f"time_s={elapsed:.3f} result={result}"
            score = elapsed
        if args.include_solved or result not in {"SAT", "UNSAT"} or score > 0:
            ranked.append((score, name, reason))

    print("instance,score,reason")
    for score, name, reason in sorted(ranked, reverse=True)[: args.top]:
        print(f"{name},{score:.3f},{reason}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
