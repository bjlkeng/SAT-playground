#!/usr/bin/env python3

from __future__ import annotations

import csv
import html
import json
import re
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
LOG_DIR = REPO_ROOT / "log"
OUTPUT_PATH = REPO_ROOT / "docs" / "data" / "medium-par2.json"
SVG_OUTPUT_PATH = REPO_ROOT / "docs" / "assets" / "medium-cumulative.svg"
GITHUB_BLOB_BASE = "https://github.com/bjlkeng/SAT-playground/blob/main"
GITHUB_TREE_BASE = "https://github.com/bjlkeng/SAT-playground/tree/main"
COMPETITION_URL = "https://satcompetition.github.io/2025/"
BENCHMARK_URL = "https://satcompetition.github.io/2025/downloads.html"
OUTPUT_FORMAT_URL = "https://satcompetition.github.io/2025/output.html"
SITE_URL = "https://bjlkeng.io/SAT-playground/"
COLORS = {
    "01-naive-dpll": "#5f83ff",
    "02-cdcl": "#163d8f",
    "minisat": "#ecab4e",
    "kissat-sc2024": "#c7631f",
    "kissat-latest": "#8d3613",
    "virtual-best": "#0d8a72",
}


@dataclass(frozen=True)
class SolverSpec:
    slug: str
    label: str
    family: str
    source_url: str
    info_url: str


SOLVERS = [
    SolverSpec(
        slug="01-naive-dpll",
        label="01 naive-dpll",
        family="repo",
        source_url=f"{GITHUB_TREE_BASE}/solver/01-naive-dpll",
        info_url="./solvers/01-naive-dpll.html",
    ),
    SolverSpec(
        slug="02-cdcl",
        label="02 cdcl",
        family="repo",
        source_url=f"{GITHUB_TREE_BASE}/solver/02-cdcl",
        info_url="./solvers/02-cdcl.html",
    ),
    SolverSpec(
        slug="minisat",
        label="MiniSat",
        family="reference",
        source_url=f"{GITHUB_TREE_BASE}/benchmarks/reference-solvers/minisat",
        info_url="https://minisat.se/",
    ),
    SolverSpec(
        slug="kissat-sc2024",
        label="Kissat sc2024",
        family="reference",
        source_url=f"{GITHUB_TREE_BASE}/benchmarks/reference-solvers/kissat-sc2024",
        info_url=BENCHMARK_URL,
    ),
    SolverSpec(
        slug="kissat-latest",
        label="Kissat latest",
        family="reference",
        source_url=f"{GITHUB_TREE_BASE}/benchmarks/reference-solvers/kissat-latest",
        info_url="https://github.com/arminbiere/kissat",
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
    solved_events = []

    with results_path.open(newline="") as handle:
        reader = csv.DictReader(handle)
        for row in reader:
            instance = row["instance"].strip()
            result = row["result"].strip()
            elapsed = float(row["time_s"])
            timeout_value = int(float(row["timeout"]))
            verified = row.get("verified", "skip").strip()
            timeout_s = timeout_value if timeout_s is None else timeout_s
            instances += 1
            solved_row = result in SOLVED_RESULTS
            rounded = round(elapsed, 3)

            rows.append(
                {
                    "instance": instance,
                    "result": result,
                    "time": rounded,
                    "verified": verified,
                    "solved": solved_row,
                }
            )

            if result == "SAT":
                sat += 1
                solved += 1
                par2 += elapsed
                solved_events.append(
                    {
                        "instance": instance,
                        "result": result,
                        "time": rounded,
                        "verified": verified,
                    }
                )
            elif result == "UNSAT":
                unsat += 1
                solved += 1
                par2 += elapsed
                solved_events.append(
                    {
                        "instance": instance,
                        "result": result,
                        "time": rounded,
                        "verified": verified,
                    }
                )
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

    solved_events.sort(key=lambda item: (item["time"], item["instance"]))

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
        "curve": build_cumulative_curve([event["time"] for event in solved_events], timeout_s),
        "events": solved_events,
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
        "infoUrl": spec.info_url,
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

    payload = {
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "benchmark": {
            "name": "SAT Competition 2025 Medium",
            "instances": 100,
            "timeoutSeconds": timeout_s,
            "memoryLimitGb": 16,
            "metric": "PAR-2",
            "sampleDescription": "100 randomly selected instances from the SAT Competition 2025 main-track benchmark set.",
            "competitionUrl": COMPETITION_URL,
            "benchmarkUrl": BENCHMARK_URL,
            "outputUrl": OUTPUT_FORMAT_URL,
            "siteUrl": SITE_URL,
            "note": "These local runs use 100 randomly selected SAT Competition 2025 main-track instances, a 1800-second timeout, and a 16 GB memory limit per solver.",
            "curveNote": "Curves show cumulative solved instances over runtime. Output and proof handling follow the SAT Competition 2025 format; the virtual best line uses the fastest solved time per instance across the plotted solvers.",
        },
        "entries": entries,
        "missing": missing,
    }
    if virtual_best is not None:
        payload["virtualBest"] = virtual_best
    return payload


def build_virtual_best(entries: list[dict], timeout_s: int) -> dict:
    best_by_instance: dict[str, dict] = {}

    for entry in entries:
        for row in entry["rows"]:
            if not row["solved"]:
                continue

            instance = row["instance"]
            time_value = row["time"]
            best = best_by_instance.get(instance)
            if best is None or time_value < best["time"]:
                best_by_instance[instance] = {
                    "instance": instance,
                    "result": row["result"],
                    "time": time_value,
                    "verified": row["verified"],
                    "solverSlug": entry["slug"],
                    "solverLabel": entry["label"],
                }

    events = sorted(best_by_instance.values(), key=lambda item: (item["time"], item["instance"]))
    return {
        "slug": "virtual-best",
        "label": "Virtual Best Solver",
        "family": "virtual-best",
        "instances": entries[0]["instances"],
        "timeout_s": timeout_s,
        "solved": len(events),
        "curve": build_cumulative_curve([event["time"] for event in events], timeout_s),
        "events": events,
    }


def build_svg_chart(payload: dict) -> str:
    width = 1080
    height = 620
    top = 88
    left = 82
    right = 30
    bottom = 88
    plot_width = width - left - right
    plot_height = height - top - bottom
    timeout_s = payload["benchmark"]["timeoutSeconds"]
    entries = list(payload["entries"])
    virtual_best = payload.get("virtualBest")
    curves = entries + ([virtual_best] if virtual_best else [])
    max_solved = max((entry["solved"] for entry in curves), default=10)
    y_step = 5 if max_solved <= 40 else 10
    y_max = max(y_step, ((max_solved + y_step - 1) // y_step) * y_step)

    def x_scale(time_value: float) -> float:
        return left + (time_value / timeout_s) * plot_width

    def y_scale(solved: int) -> float:
        return top + plot_height - (solved / y_max) * plot_height

    def line_path(curve: list[dict]) -> str:
        segments = []
        for index, point in enumerate(curve):
            cmd = "M" if index == 0 else "L"
            segments.append(f"{cmd} {x_scale(point['time']):.2f} {y_scale(point['solved']):.2f}")
        return " ".join(segments)

    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" fill="none">',
        '<rect width="100%" height="100%" fill="#f3f5f8"/>',
        '<rect x="18" y="18" width="1044" height="584" rx="24" fill="#ffffff" stroke="rgba(22,32,43,0.10)"/>',
        '<text x="42" y="56" fill="#16202b" font-family="IBM Plex Mono, monospace" font-size="14">SAT Playground · Cumulative Solved vs Time</text>',
        f'<text x="42" y="78" fill="#5c6676" font-family="IBM Plex Mono, monospace" font-size="12">{html.escape(payload["benchmark"]["sampleDescription"])} 1800s · 16 GB</text>',
    ]

    for tick in range(7):
        value = timeout_s * (tick / 6)
        x = x_scale(value)
        parts.append(f'<line x1="{x:.2f}" y1="{top}" x2="{x:.2f}" y2="{top + plot_height}" stroke="rgba(22,32,43,0.10)" stroke-width="1"/>')
        anchor = "start" if tick == 0 else "end" if tick == 6 else "middle"
        parts.append(
            f'<text x="{x:.2f}" y="{height - 40}" fill="#5c6676" font-family="IBM Plex Mono, monospace" font-size="12" text-anchor="{anchor}">{int(value):,}</text>'
        )

    for value in range(0, y_max + y_step, y_step):
        y = y_scale(value)
        parts.append(f'<line x1="{left}" y1="{y:.2f}" x2="{left + plot_width}" y2="{y:.2f}" stroke="rgba(22,32,43,0.10)" stroke-width="1"/>')
        parts.append(
            f'<text x="{left - 12}" y="{y + 4:.2f}" fill="#5c6676" font-family="IBM Plex Mono, monospace" font-size="12" text-anchor="end">{value}</text>'
        )

    parts.append(
        f'<text x="{left + plot_width / 2:.2f}" y="{height - 16}" fill="#5c6676" font-family="IBM Plex Mono, monospace" font-size="12" text-anchor="middle">Runtime (seconds)</text>'
    )
    parts.append(
        f'<text x="22" y="{top + plot_height / 2:.2f}" fill="#5c6676" font-family="IBM Plex Mono, monospace" font-size="12" text-anchor="middle" transform="rotate(-90 22 {top + plot_height / 2:.2f})">Solved instances</text>'
    )

    for entry in entries:
        color = COLORS[entry["slug"]]
        marker = entry["curve"][-2] if len(entry["curve"]) >= 2 and entry["curve"][-1]["time"] == timeout_s else entry["curve"][-1]
        parts.append(
            f'<path d="{line_path(entry["curve"])}" stroke="{color}" stroke-width="3" stroke-linejoin="round" stroke-linecap="round"/>'
        )
        parts.append(
            f'<circle cx="{x_scale(marker["time"]):.2f}" cy="{y_scale(marker["solved"]):.2f}" r="3.4" fill="{color}"/>'
        )

    if virtual_best is not None:
        color = COLORS["virtual-best"]
        marker = virtual_best["curve"][-2] if len(virtual_best["curve"]) >= 2 and virtual_best["curve"][-1]["time"] == timeout_s else virtual_best["curve"][-1]
        parts.append(
            f'<path d="{line_path(virtual_best["curve"])}" stroke="{color}" stroke-width="3.5" stroke-linejoin="round" stroke-linecap="round" stroke-dasharray="10 7"/>'
        )
        parts.append(
            f'<circle cx="{x_scale(marker["time"]):.2f}" cy="{y_scale(marker["solved"]):.2f}" r="4" fill="{color}"/>'
        )

    legend_y = height - 56
    legend_x = 48
    for entry in ([virtual_best] if virtual_best else []) + entries:
        color = COLORS[entry["slug"]]
        dash = ' stroke-dasharray="10 7"' if entry["slug"] == "virtual-best" else ""
        parts.append(
            f'<line x1="{legend_x}" y1="{legend_y}" x2="{legend_x + 24}" y2="{legend_y}" stroke="{color}" stroke-width="4" stroke-linecap="round"{dash}/>'
        )
        parts.append(
            f'<text x="{legend_x + 34}" y="{legend_y + 4}" fill="#16202b" font-family="IBM Plex Mono, monospace" font-size="12">{html.escape(entry["label"])}</text>'
        )
        legend_x += 34 + max(88, len(entry["label"]) * 8)

    parts.append("</svg>")
    return "\n".join(parts)


def main() -> None:
    payload = build_payload()
    OUTPUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT_PATH.write_text(json.dumps(payload, indent=2) + "\n")
    SVG_OUTPUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    SVG_OUTPUT_PATH.write_text(build_svg_chart(payload) + "\n")
    print(f"Wrote {OUTPUT_PATH}")
    print(f"Wrote {SVG_OUTPUT_PATH}")


if __name__ == "__main__":
    main()
