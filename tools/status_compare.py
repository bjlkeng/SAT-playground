#!/usr/bin/env python3
"""Compare two bench.sh results.csv files for baseline-lock checks."""

from __future__ import annotations

import argparse
import csv
import sys
from collections import Counter
from pathlib import Path


SOLVED = {"SAT", "UNSAT"}


def read_results(path: Path) -> dict[str, dict[str, str]]:
    with path.open(newline="") as handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames is None:
            raise SystemExit(f"{path}: missing CSV header")
        required = {"instance", "result"}
        missing = required.difference(reader.fieldnames)
        if missing:
            raise SystemExit(f"{path}: missing columns: {', '.join(sorted(missing))}")
        rows: dict[str, dict[str, str]] = {}
        for row in reader:
            instance = (row.get("instance") or "").strip()
            if not instance:
                raise SystemExit(f"{path}: row with empty instance name")
            if instance in rows:
                raise SystemExit(f"{path}: duplicate instance {instance!r}")
            rows[instance] = {key: (value or "").strip() for key, value in row.items()}
    if not rows:
        raise SystemExit(f"{path}: no result rows")
    return rows


def count_results(rows: dict[str, dict[str, str]]) -> Counter[str]:
    return Counter(row["result"] for row in rows.values())


def solved_count(rows: dict[str, dict[str, str]]) -> int:
    return sum(1 for row in rows.values() if row["result"] in SOLVED)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", required=True, type=Path)
    parser.add_argument("--after", required=True, type=Path)
    args = parser.parse_args()

    before = read_results(args.before)
    after = read_results(args.after)

    before_names = set(before)
    after_names = set(after)
    missing_after = sorted(before_names - after_names)
    extra_after = sorted(after_names - before_names)
    common = sorted(before_names & after_names)

    status_mismatches = [
        (name, before[name]["result"], after[name]["result"])
        for name in common
        if before[name]["result"] != after[name]["result"]
    ]
    verified_mismatches = [
        (name, before[name].get("verified", ""), after[name].get("verified", ""))
        for name in common
        if "verified" in before[name]
        and "verified" in after[name]
        and before[name].get("verified", "") != after[name].get("verified", "")
    ]

    print(f"before={args.before}")
    print(f"after={args.after}")
    print(f"before_rows={len(before)}")
    print(f"after_rows={len(after)}")
    print(f"common_rows={len(common)}")
    print(f"before_counts={dict(sorted(count_results(before).items()))}")
    print(f"after_counts={dict(sorted(count_results(after).items()))}")
    print(f"before_solved={solved_count(before)}")
    print(f"after_solved={solved_count(after)}")
    print(f"missing_after={missing_after}")
    print(f"extra_after={extra_after}")
    print(f"status_mismatches={status_mismatches}")
    print(f"verified_mismatches={verified_mismatches}")

    if missing_after or extra_after or status_mismatches:
        print("verdict=FAIL")
        return 1

    print("verdict=PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
