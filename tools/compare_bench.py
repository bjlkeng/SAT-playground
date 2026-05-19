#!/usr/bin/env python3
"""Minimal paired bench comparison shim for early solver 11 slices."""

from __future__ import annotations

import argparse
import csv
import sys
from pathlib import Path


def read_rows(path: Path) -> dict[str, dict[str, str]]:
    with path.open(newline="") as handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames is None:
            raise SystemExit(f"{path}: missing CSV header")
        required = {"instance", "result", "time_s"}
        missing = required.difference(reader.fieldnames)
        if missing:
            raise SystemExit(f"{path}: missing columns: {', '.join(sorted(missing))}")
        rows: dict[str, dict[str, str]] = {}
        for row in reader:
            name = (row.get("instance") or "").strip()
            if not name:
                raise SystemExit(f"{path}: empty instance name")
            rows[name] = {key: (value or "").strip() for key, value in row.items()}
    return rows


def as_float(value: str) -> float:
    try:
        return float(value)
    except ValueError:
        return 0.0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", required=True, type=Path)
    parser.add_argument("--after", required=True, type=Path)
    parser.add_argument("--timeout", type=float, default=None)
    parser.add_argument("--baseline", type=Path, default=None)
    args = parser.parse_args()

    before = read_rows(args.before)
    after = read_rows(args.after)
    common = sorted(set(before) & set(after))
    if not common:
        raise SystemExit("no common instances to compare")

    missing_after = sorted(set(before) - set(after))
    extra_after = sorted(set(after) - set(before))
    status_mismatches = []
    total_delta = 0.0

    print(f"before={args.before}")
    print(f"after={args.after}")
    print(f"common_rows={len(common)}")
    print("instance,before_result,after_result,before_time_s,after_time_s,delta_s")
    for name in common:
        before_row = before[name]
        after_row = after[name]
        before_time = as_float(before_row["time_s"])
        after_time = as_float(after_row["time_s"])
        delta = after_time - before_time
        total_delta += delta
        if before_row["result"] != after_row["result"]:
            status_mismatches.append((name, before_row["result"], after_row["result"]))
        print(
            f"{name},{before_row['result']},{after_row['result']},"
            f"{before_time:.3f},{after_time:.3f},{delta:.3f}"
        )

    print(f"total_delta_s={total_delta:.3f}")
    print(f"missing_after={missing_after}")
    print(f"extra_after={extra_after}")
    print(f"status_mismatches={status_mismatches}")
    if missing_after or extra_after or status_mismatches:
        print("verdict=FAIL")
        return 1
    print("verdict=PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
