#!/usr/bin/env python3
"""Work x speed decomposition for the conflict-analysis ablation."""
from __future__ import annotations

import argparse
import csv
import json
import math
from pathlib import Path

CONFIGS_ORDER = [
    "A_baseline",
    "B_resolved",
    "C_lbd_metadata",
    "D_lbd_resolved",
    "E_focused_stable",
    "F_focused_stable_resolved",
]

PAIRWISE = [
    ("B_resolved", "A_baseline"),
    ("D_lbd_resolved", "C_lbd_metadata"),
    ("F_focused_stable_resolved", "E_focused_stable"),
]


def short_instance(name: str) -> str:
    base = name.rsplit("/", 1)[-1]
    if "-" in base:
        prefix, rest = base.split("-", 1)
        if len(prefix) == 32 and all(c in "0123456789abcdef" for c in prefix):
            base = rest
    for suffix in (".cnf.xz", ".cnf.gz", ".cnf"):
        if base.endswith(suffix):
            return base[: -len(suffix)]
    return base


def load_results(cfg_dir: Path) -> dict[str, dict[str, object]]:
    rows: dict[str, dict[str, object]] = {}
    path = cfg_dir / "results.csv"
    if not path.exists():
        return rows
    with path.open(newline="") as handle:
        for row in csv.DictReader(handle):
            try:
                wall = float(row["time_s"])
            except (KeyError, TypeError, ValueError):
                wall = math.nan
            rows[row["instance"]] = {
                "result": row.get("result", ""),
                "verified": row.get("verified", ""),
                "wall": wall,
                "exit_code": row.get("exit_code", ""),
            }
    return rows


def load_stats(cfg_dir: Path) -> dict[str, dict[str, object]]:
    rows: dict[str, dict[str, object]] = {}
    path = cfg_dir / "stats.jsonl"
    if not path.exists():
        return rows
    with path.open() as handle:
        for raw in handle:
            raw = raw.strip()
            if not raw:
                continue
            try:
                payload = json.loads(raw)
            except json.JSONDecodeError:
                continue
            inst = payload.get("instance") or payload.get("input_basename") or payload.get("input")
            if inst:
                rows[str(inst)] = payload
    return rows


def counter(stats: dict[str, object], *keys: str) -> int | float:
    for key in keys:
        value = stats.get(key)
        if isinstance(value, (int, float)):
            return value
    for group in ("stats", "counters", "search", "result", "formula"):
        sub = stats.get(group)
        if isinstance(sub, dict):
            value = counter(sub, *keys)
            if value:
                return value
    return 0


def par2(results: dict[str, dict[str, object]], timeout_s: float) -> float:
    total = 0.0
    for row in results.values():
        if row.get("result") in {"SAT", "UNSAT"}:
            total += float(row.get("wall", timeout_s * 2))
        else:
            total += timeout_s * 2
    return total


def safe_ratio(num: float, den: float) -> float:
    if den == 0 or math.isnan(den) or math.isnan(num):
        return math.nan
    return num / den


def classify(work: float, speed: float) -> str:
    if math.isnan(work) or math.isnan(speed):
        return "incomparable"
    work_moved = abs(work - 1.0) > 0.10
    speed_moved = abs(speed - 1.0) > 0.05
    if work_moved and speed_moved:
        return "mixed"
    if work_moved:
        return "trajectory"
    if speed_moved:
        return "execution"
    return "noise"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--slug-dir", default=str(Path(__file__).resolve().parent))
    parser.add_argument("--timeout", type=float, default=300.0)
    args = parser.parse_args()

    slug_dir = Path(args.slug_dir)
    configs = {
        cfg: {
            "results": load_results(slug_dir / cfg),
            "stats": load_stats(slug_dir / cfg),
        }
        for cfg in CONFIGS_ORDER
    }
    instances = sorted(
        {inst for data in configs.values() for inst in data["results"].keys()},
        key=short_instance,
    )
    if not instances:
        raise SystemExit(f"no result rows under {slug_dir}")

    matrix_path = slug_dir / "matrix.csv"
    with matrix_path.open("w", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(
            [
                "config",
                "instance",
                "result",
                "verified",
                "wall_s",
                "conflicts",
                "decisions",
                "propagations",
                "props_per_s",
                "restarts",
                "learned_clauses_final",
                "reduce_db_calls",
                "original_clauses_after_preprocess",
                "original_lits_after_preprocess",
                "preprocess_time_s",
            ]
        )
        for cfg in CONFIGS_ORDER:
            for inst in instances:
                result = configs[cfg]["results"].get(inst, {})
                stats = configs[cfg]["stats"].get(inst, {})
                wall = float(result.get("wall", math.nan))
                props = counter(stats, "propagations")
                props_per_s = safe_ratio(float(props), wall)
                writer.writerow(
                    [
                        cfg,
                        short_instance(inst),
                        result.get("result", ""),
                        result.get("verified", ""),
                        f"{wall:.3f}" if not math.isnan(wall) else "",
                        int(counter(stats, "conflicts")),
                        int(counter(stats, "decisions")),
                        int(props),
                        f"{props_per_s:.0f}" if not math.isnan(props_per_s) else "",
                        int(counter(stats, "restarts")),
                        int(counter(stats, "learned_clauses_final")),
                        int(counter(stats, "reduce_db_calls")),
                        int(counter(stats, "original_clauses_after_preprocess")),
                        int(counter(stats, "original_lits_after_preprocess")),
                        f"{counter(stats, 'preprocess_sec'):.6f}",
                    ]
                )

    decomp_path = slug_dir / "decomp.csv"
    baseline = configs["A_baseline"]
    with decomp_path.open("w", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(
            [
                "config",
                "instance",
                "result_A",
                "result_cfg",
                "wall_A",
                "wall_cfg",
                "conf_A",
                "conf_cfg",
                "props_A",
                "props_cfg",
                "work_ratio",
                "speed_ratio",
                "net_pred",
                "actual_wall_ratio",
                "dominant",
            ]
        )
        for cfg in CONFIGS_ORDER[1:]:
            for inst in instances:
                res_a = baseline["results"].get(inst, {})
                res_c = configs[cfg]["results"].get(inst, {})
                stats_a = baseline["stats"].get(inst, {})
                stats_c = configs[cfg]["stats"].get(inst, {})
                wall_a = float(res_a.get("wall", math.nan))
                wall_c = float(res_c.get("wall", math.nan))
                conf_a = float(counter(stats_a, "conflicts"))
                conf_c = float(counter(stats_c, "conflicts"))
                props_a = float(counter(stats_a, "propagations"))
                props_c = float(counter(stats_c, "propagations"))
                pps_a = safe_ratio(props_a, wall_a)
                pps_c = safe_ratio(props_c, wall_c)
                work = safe_ratio(conf_c, conf_a)
                speed = safe_ratio(pps_a, pps_c)
                net = work * speed if not math.isnan(work) and not math.isnan(speed) else math.nan
                actual = safe_ratio(wall_c, wall_a)
                writer.writerow(
                    [
                        cfg,
                        short_instance(inst),
                        res_a.get("result", ""),
                        res_c.get("result", ""),
                        f"{wall_a:.3f}" if not math.isnan(wall_a) else "",
                        f"{wall_c:.3f}" if not math.isnan(wall_c) else "",
                        int(conf_a),
                        int(conf_c),
                        int(props_a),
                        int(props_c),
                        f"{work:.3f}" if not math.isnan(work) else "",
                        f"{speed:.3f}" if not math.isnan(speed) else "",
                        f"{net:.3f}" if not math.isnan(net) else "",
                        f"{actual:.3f}" if not math.isnan(actual) else "",
                        classify(work, speed),
                    ]
                )

    pairwise_path = slug_dir / "resolved_pairwise.csv"
    with pairwise_path.open("w", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(
            [
                "pair",
                "instance",
                "base_result",
                "resolved_result",
                "base_wall_s",
                "resolved_wall_s",
                "wall_ratio",
                "base_conflicts",
                "resolved_conflicts",
                "base_decisions",
                "resolved_decisions",
                "base_propagations",
                "resolved_propagations",
                "same_work",
                "base_props_per_s",
                "resolved_props_per_s",
                "props_per_s_ratio_resolved_over_base",
            ]
        )
        for resolved_cfg, base_cfg in PAIRWISE:
            for inst in instances:
                base_res = configs[base_cfg]["results"].get(inst, {})
                res_res = configs[resolved_cfg]["results"].get(inst, {})
                base_stats = configs[base_cfg]["stats"].get(inst, {})
                res_stats = configs[resolved_cfg]["stats"].get(inst, {})
                base_wall = float(base_res.get("wall", math.nan))
                res_wall = float(res_res.get("wall", math.nan))
                base_conf = int(counter(base_stats, "conflicts"))
                res_conf = int(counter(res_stats, "conflicts"))
                base_dec = int(counter(base_stats, "decisions"))
                res_dec = int(counter(res_stats, "decisions"))
                base_props = int(counter(base_stats, "propagations"))
                res_props = int(counter(res_stats, "propagations"))
                base_pps = safe_ratio(float(base_props), base_wall)
                res_pps = safe_ratio(float(res_props), res_wall)
                same_work = (
                    base_conf == res_conf
                    and base_dec == res_dec
                    and base_props == res_props
                    and int(counter(base_stats, "restarts")) == int(counter(res_stats, "restarts"))
                    and int(counter(base_stats, "learned_clauses_final"))
                    == int(counter(res_stats, "learned_clauses_final"))
                )
                writer.writerow(
                    [
                        f"{resolved_cfg}_vs_{base_cfg}",
                        short_instance(inst),
                        base_res.get("result", ""),
                        res_res.get("result", ""),
                        f"{base_wall:.3f}" if not math.isnan(base_wall) else "",
                        f"{res_wall:.3f}" if not math.isnan(res_wall) else "",
                        f"{safe_ratio(res_wall, base_wall):.3f}",
                        base_conf,
                        res_conf,
                        base_dec,
                        res_dec,
                        base_props,
                        res_props,
                        "yes" if same_work else "no",
                        f"{base_pps:.0f}" if not math.isnan(base_pps) else "",
                        f"{res_pps:.0f}" if not math.isnan(res_pps) else "",
                        f"{safe_ratio(res_pps, base_pps):.3f}"
                        if not math.isnan(base_pps) and not math.isnan(res_pps)
                        else "",
                    ]
                )

    summary_path = slug_dir / "summary.md"
    with summary_path.open("w") as handle:
        handle.write("# Conflict-analysis ablation summary\n\n")
        handle.write(f"Slug dir: `{slug_dir}`\n\n")
        handle.write("| config | solved | timeout | unknown | error | PAR-2 |\n")
        handle.write("|---|---:|---:|---:|---:|---:|\n")
        for cfg in CONFIGS_ORDER:
            results = configs[cfg]["results"]
            solved = sum(1 for row in results.values() if row.get("result") in {"SAT", "UNSAT"})
            timeout = sum(1 for row in results.values() if row.get("result") == "TIMEOUT")
            unknown = sum(1 for row in results.values() if row.get("result") == "UNKNOWN")
            error = sum(
                1
                for row in results.values()
                if row.get("result") not in {"SAT", "UNSAT", "TIMEOUT", "UNKNOWN"}
            )
            handle.write(
                f"| {cfg} | {solved} | {timeout} | {unknown} | {error} | "
                f"{par2(results, args.timeout):.1f} |\n"
            )
        handle.write("\nSee `matrix.csv` and `decomp.csv` for per-instance counters.\n")

    print(f"wrote {matrix_path}")
    print(f"wrote {decomp_path}")
    print(f"wrote {pairwise_path}")
    print(f"wrote {summary_path}")


if __name__ == "__main__":
    main()
