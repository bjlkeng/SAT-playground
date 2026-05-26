#!/usr/bin/env python3
from __future__ import annotations

import csv
import json
import math
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
OUT = Path(__file__).resolve().parent
MATRIX = OUT / "config_matrix.psv"


def read_matrix() -> list[dict[str, str]]:
    rows: list[dict[str, str]] = []
    with MATRIX.open() as f:
        header = f.readline().rstrip("\n").split("|")
        for line in f:
            if not line.strip():
                continue
            values = line.rstrip("\n").split("|")
            rows.append(dict(zip(header, values)))
    return rows


def read_results(label: str) -> list[dict[str, str]]:
    path = OUT / label / "results.csv"
    if not path.exists():
        return []
    with path.open(newline="") as f:
        return list(csv.DictReader(f))


def read_stats(label: str) -> dict[str, dict]:
    path = OUT / label / "stats.jsonl"
    stats: dict[str, dict] = {}
    if not path.exists():
        return stats
    with path.open() as f:
        for line in f:
            if not line.strip():
                continue
            row = json.loads(line)
            stats[row["instance"]] = row
    return stats


def read_summary(label: str) -> dict[str, str]:
    path = OUT / label / "summary.log"
    data: dict[str, str] = {}
    if not path.exists():
        return data
    for line in path.read_text().splitlines():
        match = re.match(r"\s*(Solved|Unsolved|PAR-2):\s+(.+)$", line)
        if match:
            data[match.group(1).lower().replace("-", "_")] = match.group(2).strip()
    return data


def fnum(value, default=0.0) -> float:
    try:
        if value is None or value == "":
            return default
        return float(value)
    except (TypeError, ValueError):
        return default


def inum(value, default=0) -> int:
    try:
        if value is None or value == "":
            return default
        return int(value)
    except (TypeError, ValueError):
        return default


def ratio(num: float, den: float) -> str:
    if den == 0 or math.isnan(num) or math.isnan(den):
        return ""
    return f"{num / den:.6f}"


def props_per_sec(stats: dict, result: dict) -> float:
    props = fnum(stats.get("propagations"))
    elapsed = fnum(stats.get("elapsed_seconds"), fnum(result.get("time_s")))
    if elapsed <= 0:
        return 0.0
    return props / elapsed


def main() -> None:
    matrix = read_matrix()
    labels = [row["label"] for row in matrix]
    results = {label: read_results(label) for label in labels}
    stats = {label: read_stats(label) for label in labels}

    summary_rows: list[dict[str, str]] = []
    for row in matrix:
        label = row["label"]
        rrows = results[label]
        solved = sum(1 for r in rrows if r.get("result") in {"SAT", "UNSAT"})
        unknown = sum(1 for r in rrows if r.get("result") == "UNKNOWN")
        timeout = sum(1 for r in rrows if r.get("result") == "TIMEOUT")
        error = sum(1 for r in rrows if r.get("result") == "ERROR")
        par2 = ""
        summary = read_summary(label)
        if "par_2" in summary:
            par2 = summary["par_2"]
        elif rrows:
            total = 0.0
            for r in rrows:
                if r.get("result") in {"SAT", "UNSAT"}:
                    total += fnum(r.get("time_s"))
                else:
                    total += 2.0 * fnum(r.get("timeout"), 300.0)
            par2 = f"{total:.3f}"
        summary_rows.append(
            {
                "label": label,
                "solved": str(solved),
                "unknown": str(unknown),
                "timeout": str(timeout),
                "error": str(error),
                "par2": par2,
                "env": row["env"],
            }
        )

    with (OUT / "config_summary.csv").open("w", newline="") as f:
        writer = csv.DictWriter(
            f, fieldnames=["label", "solved", "unknown", "timeout", "error", "par2", "env"]
        )
        writer.writeheader()
        writer.writerows(summary_rows)

    base_results = {r["instance"]: r for r in results.get("A_default", [])}
    base_stats = stats.get("A_default", {})
    rows: list[dict[str, str]] = []
    for label in labels:
        if label == "A_default":
            continue
        for r in results[label]:
            instance = r["instance"]
            brow = base_results.get(instance)
            if not brow:
                continue
            s = stats[label].get(instance, {})
            bs = base_stats.get(instance, {})
            conflicts = fnum(s.get("conflicts"))
            base_conflicts = fnum(bs.get("conflicts"))
            pps = props_per_sec(s, r)
            base_pps = props_per_sec(bs, brow)
            wall = fnum(r.get("time_s"))
            base_wall = fnum(brow.get("time_s"))
            rows.append(
                {
                    "label": label,
                    "instance": instance,
                    "result": r.get("result", ""),
                    "base_result": brow.get("result", ""),
                    "time_s": f"{wall:.3f}",
                    "base_time_s": f"{base_wall:.3f}",
                    "wall_ratio": ratio(wall, base_wall),
                    "conflicts": str(inum(s.get("conflicts"))),
                    "base_conflicts": str(inum(bs.get("conflicts"))),
                    "work_ratio": ratio(conflicts, base_conflicts),
                    "props_per_sec": f"{pps:.3f}",
                    "base_props_per_sec": f"{base_pps:.3f}",
                    "speed_ratio": ratio(base_pps, pps),
                    "net_work_speed": ratio(conflicts * base_pps, base_conflicts * pps),
                    "decisions": str(inum(s.get("decisions"))),
                    "base_decisions": str(inum(bs.get("decisions"))),
                    "restarts": str(inum(s.get("restarts"))),
                    "base_restarts": str(inum(bs.get("restarts"))),
                    "chrono_used": str(inum(s.get("chrono_used"))),
                    "phase_legacy_used": str(inum(s.get("phase_legacy_used"))),
                    "phase_saved_used": str(inum(s.get("phase_saved_used"))),
                    "phase_target_used": str(inum(s.get("phase_target_used"))),
                    "phase_best_used": str(inum(s.get("phase_best_used"))),
                }
            )

    with (OUT / "work_speed.csv").open("w", newline="") as f:
        fieldnames = [
            "label",
            "instance",
            "result",
            "base_result",
            "time_s",
            "base_time_s",
            "wall_ratio",
            "conflicts",
            "base_conflicts",
            "work_ratio",
            "props_per_sec",
            "base_props_per_sec",
            "speed_ratio",
            "net_work_speed",
            "decisions",
            "base_decisions",
            "restarts",
            "base_restarts",
            "chrono_used",
            "phase_legacy_used",
            "phase_saved_used",
            "phase_target_used",
            "phase_best_used",
        ]
        writer = csv.DictWriter(f, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)


if __name__ == "__main__":
    main()
