#!/usr/bin/env python3

from __future__ import annotations

import csv
import json
import re
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
LOG_DIR = REPO_ROOT / "log"
OUTPUT_PATH = REPO_ROOT / "docs" / "data" / "medium-par2.json"
GITHUB_BLOB_BASE = "https://github.com/bjlkeng/SAT-playground/blob/main"


@dataclass(frozen=True)
class SolverSpec:
    slug: str
    label: str
    family: str
    source_url: str


SOLVERS = [
    SolverSpec(
        slug="01-naive-dpll",
        label="01 naive-dpll",
        family="repo",
        source_url=f"{GITHUB_BLOB_BASE}/solver/01-naive-dpll",
    ),
    SolverSpec(
        slug="02-cdcl",
        label="02 cdcl",
        family="repo",
        source_url=f"{GITHUB_BLOB_BASE}/solver/02-cdcl",
    ),
    SolverSpec(
        slug="minisat",
        label="MiniSat",
        family="reference",
        source_url=f"{GITHUB_BLOB_BASE}/benchmarks/reference-solvers/minisat",
    ),
    SolverSpec(
        slug="kissat-sc2024",
        label="Kissat sc2024",
        family="reference",
        source_url=f"{GITHUB_BLOB_BASE}/benchmarks/reference-solvers/kissat-sc2024",
    ),
    SolverSpec(
        slug="kissat-latest",
        label="Kissat latest",
        family="reference",
        source_url=f"{GITHUB_BLOB_BASE}/benchmarks/reference-solvers/kissat-latest",
    ),
]


DATE_RE = re.compile(r"^\s*Date:\s+(?P<value>.+?)\s*$")
SOLVED_RESULTS = {"SAT", "UNSAT"}


def repo_relative(path: Path) -> str:
    return path.relative_to(REPO_ROOT).as_posix()


def github_blob_url(path: Path) -> str:
    return f"{GITHUB_BLOB_BASE}/{repo_relative(path)}"


def parse_summary_date(summary_path: Path) -> str | None:
    for line in summary_path.read_text().splitlines():
        match = DATE_RE.match(line)
        if match:
            return match.group("value")
    return None


def build_cumulative_curve(solved_times: list[float], timeout_s: int) -> list[dict[str, float | int]]:
    points: list[dict[str, float | int]] = [{"time": 0.0, "solved": 0}]
    solved = 0

    for elapsed in sorted(solved_times):
        rounded = round(elapsed, 3)
        points.append({"time": rounded, "solved": solved})
        solved += 1
        points.append({"time": rounded, "solved": solved})

    if points[-1]["time"] < timeout_s:
        points.append({"time": float(timeout_s), "solved": solved})

    return points


def parse_results(results_path: Path) -> dict[str, float | int | list]:
    solved = sat = unsat = timeouts = unknown = errors = 0
    par2 = 0.0
    instances = 0
    timeout_s = None
    rows = []
    solved_times = []

    with results_path.open(newline="") as handle:
        reader = csv.DictReader(handle)
        for row in reader:
            instance = row["instance"].strip()
            result = row["result"].strip()
            elapsed = float(row["time_s"])
            timeout_value = int(float(row["timeout"]))
            timeout_s = timeout_value if timeout_s is None else timeout_s
            instances += 1
            solved_row = result in SOLVED_RESULTS

            rows.append(
                {
                    "instance": instance,
                    "result": result,
                    "time": round(elapsed, 3),
                    "solved": solved_row,
                }
            )

            if result == "SAT":
                sat += 1
                solved += 1
                par2 += elapsed
                solved_times.append(elapsed)
            elif result == "UNSAT":
                unsat += 1
                solved += 1
                par2 += elapsed
                solved_times.append(elapsed)
            elif result == "TIMEOUT":
                timeouts += 1
                par2 += 2 * timeout_value
            elif result == "UNKNOWN":
                unknown += 1
                par2 += 2 * timeout_value
            else:
                errors += 1
                par2 += 2 * timeout_value

    if timeout_s is None:
        raise ValueError(f"no rows found in {results_path}")

    return {
        "instances": instances,
        "timeout_s": timeout_s,
        "par2": round(par2, 3),
        "solved": solved,
        "sat": sat,
        "unsat": unsat,
        "timeouts": timeouts,
        "unknown": unknown,
        "errors": errors,
        "curve": build_cumulative_curve(solved_times, timeout_s),
        "rows": rows,
    }


def find_latest_medium_run(spec: SolverSpec) -> dict | None:
    candidates = []
    pattern = f"bench-{spec.slug}-*/summary.log"
    for summary_path in LOG_DIR.glob(pattern):
        results_path = summary_path.parent / "results.csv"
        if not results_path.exists():
            continue

        metrics = parse_results(results_path)
        if metrics["instances"] != 100 or metrics["timeout_s"] != 1800:
            continue

        candidates.append(
            {
                "summary_path": summary_path,
                "results_path": results_path,
                "metrics": metrics,
                "date": parse_summary_date(summary_path),
            }
        )

    if not candidates:
        return None

    latest = max(candidates, key=lambda item: item["summary_path"].parent.name)
    entry = {
        "slug": spec.slug,
        "label": spec.label,
        "family": spec.family,
        "sourceUrl": spec.source_url,
        "summaryPath": repo_relative(latest["summary_path"]),
        "resultsPath": repo_relative(latest["results_path"]),
        "summaryUrl": github_blob_url(latest["summary_path"]),
        "resultsUrl": github_blob_url(latest["results_path"]),
        "date": latest["date"],
        **latest["metrics"],
    }
    return entry


def build_payload() -> dict:
    entries = []
    missing = []

    for spec in SOLVERS:
        entry = find_latest_medium_run(spec)
        if entry is None:
            missing.append(spec.slug)
        else:
            entries.append(entry)

    entries.sort(key=lambda item: (item["par2"], item["label"]))
    timeout_s = entries[0]["timeout_s"] if entries else 1800
    virtual_best = build_virtual_best(entries, timeout_s) if entries else None

    for entry in entries:
        entry.pop("rows", None)

    payload = {
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "benchmark": {
            "name": "SAT Competition 2025 Medium",
            "instances": 100,
            "timeoutSeconds": timeout_s,
            "metric": "PAR-2",
            "note": "Latest available medium run per solver with 100 instances and a 1800s timeout.",
            "curveNote": "Curves show cumulative solved instances over runtime; the virtual best line uses the fastest solved time per instance across the plotted solvers.",
        },
        "entries": entries,
        "missing": missing,
    }
    if virtual_best is not None:
        payload["virtualBest"] = virtual_best
    return payload


def build_virtual_best(entries: list[dict], timeout_s: int) -> dict:
    best_by_instance: dict[str, float] = {}

    for entry in entries:
        for row in entry["rows"]:
            if not row["solved"]:
                continue

            instance = row["instance"]
            time_value = row["time"]
            best = best_by_instance.get(instance)
            if best is None or time_value < best:
                best_by_instance[instance] = time_value

    solved_times = list(best_by_instance.values())
    return {
        "slug": "virtual-best",
        "label": "Virtual Best Solver",
        "family": "virtual-best",
        "instances": entries[0]["instances"],
        "timeout_s": timeout_s,
        "solved": len(solved_times),
        "curve": build_cumulative_curve(solved_times, timeout_s),
    }


def main() -> None:
    payload = build_payload()
    OUTPUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT_PATH.write_text(json.dumps(payload, indent=2) + "\n")
    print(f"Wrote {OUTPUT_PATH}")


if __name__ == "__main__":
    main()
