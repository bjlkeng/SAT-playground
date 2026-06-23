#!/usr/bin/env python3
"""Run initial-clause order sensitivity checks on shuffled CNF seeds."""

from __future__ import annotations

import argparse
import csv
import json
import os
import shutil
import subprocess
import sys
from datetime import datetime
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MODES = "canonical-sorted,input-order,kissat-watch"
SUMMARY_FIELDS = [
    "mode",
    "seed",
    "instance",
    "source",
    "result",
    "verified",
    "time_s",
    "conflicts",
    "decisions",
    "propagations",
    "unknown_reason",
    "bench_log_dir",
]


def parse_csv_list(value: str) -> list[str]:
    return [item.strip() for item in value.split(",") if item.strip()]


def instance_label(path: Path, index: int) -> str:
    name = path.name
    if name.endswith(".xz"):
        name = name[:-3]
    if name.endswith(".cnf"):
        name = name[:-4]
    return f"{index:02d}-{name}"


def run_command(
    cmd: list[str],
    *,
    env: dict[str, str] | None = None,
    cwd: Path = REPO_ROOT,
    dry_run: bool = False,
) -> None:
    printable = " ".join(cmd)
    if dry_run:
        print(printable)
        return
    subprocess.run(cmd, cwd=cwd, env=env, check=True)


def read_results(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as fh:
        return list(csv.DictReader(fh))


def read_stats(path: Path) -> dict[str, dict[str, object]]:
    stats: dict[str, dict[str, object]] = {}
    if not path.exists():
        return stats
    with path.open(encoding="utf-8") as fh:
        for line in fh:
            if not line.strip():
                continue
            item = json.loads(line)
            instance = str(item.get("instance", ""))
            if instance:
                stats[instance] = item
    return stats


def write_summary(
    path: Path,
    rows: list[dict[str, object]],
) -> None:
    with path.open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(fh, fieldnames=SUMMARY_FIELDS)
        writer.writeheader()
        for row in rows:
            writer.writerow({field: row.get(field, "") for field in SUMMARY_FIELDS})


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--solver", default="solver/11-kissat-search")
    parser.add_argument("--instances", nargs="+", required=True, type=Path)
    parser.add_argument("--seeds", default="1,2,3")
    parser.add_argument("--modes", default=DEFAULT_MODES)
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("--memory-mb", type=int, default=16384)
    parser.add_argument("--work-dir", type=Path)
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)

    seeds = [int(seed) for seed in parse_csv_list(args.seeds)]
    modes = parse_csv_list(args.modes)
    timestamp = datetime.now().strftime("%Y-%m-%d-%H-%M-%S")
    work_dir = args.work_dir or REPO_ROOT / "log" / f"shuffle-sensitivity-{timestamp}"
    if not work_dir.is_absolute():
        work_dir = REPO_ROOT / work_dir
    if work_dir.exists() and args.force and not args.dry_run:
        shutil.rmtree(work_dir)
    if work_dir.exists() and any(work_dir.iterdir()) and not args.force and not args.dry_run:
        raise SystemExit(f"{work_dir} already exists and is non-empty; pass --force")

    shuffled_dir = work_dir / "shuffled"
    manifest_rows: list[dict[str, object]] = []
    for index, source in enumerate(args.instances, 1):
        source = source if source.is_absolute() else REPO_ROOT / source
        label = instance_label(source, index)
        for seed in seeds:
            output = shuffled_dir / f"{label}.seed-{seed}.cnf"
            run_command(
                [
                    sys.executable,
                    str(REPO_ROOT / "tools" / "shuffle_cnf.py"),
                    "--seed",
                    str(seed),
                    str(source),
                    str(output),
                ],
                dry_run=args.dry_run,
            )
            manifest_rows.append(
                {
                    "instance": output.stem,
                    "seed": seed,
                    "source": str(source.relative_to(REPO_ROOT)),
                    "path": str(output.relative_to(REPO_ROOT)),
                }
            )

    if args.dry_run:
        for mode in modes:
            print(
                "SAT_STATS_JSON=on "
                f"SAT_INITIAL_CLAUSE_MODE={mode} "
                f"bash tools/bench.sh -t {args.timeout} -m {args.memory_mb} "
                f"-d {shuffled_dir} --log-dir {work_dir / 'bench' / mode} {args.solver}"
            )
        return 0

    work_dir.mkdir(parents=True, exist_ok=True)
    manifest_path = work_dir / "manifest.csv"
    with manifest_path.open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(fh, fieldnames=["instance", "seed", "source", "path"])
        writer.writeheader()
        writer.writerows(manifest_rows)
    manifest_by_instance = {str(row["instance"]): row for row in manifest_rows}

    summary_rows: list[dict[str, object]] = []
    for mode in modes:
        bench_dir = work_dir / "bench" / mode
        env = os.environ.copy()
        env["SAT_STATS_JSON"] = "on"
        env["SAT_INITIAL_CLAUSE_MODE"] = mode
        run_command(
            [
                "bash",
                "tools/bench.sh",
                "-t",
                str(args.timeout),
                "-m",
                str(args.memory_mb),
                "-d",
                str(shuffled_dir),
                "--log-dir",
                str(bench_dir),
                args.solver,
            ],
            env=env,
        )
        stats_by_instance = read_stats(bench_dir / "stats.jsonl")
        for result in read_results(bench_dir / "results.csv"):
            instance = result["instance"]
            manifest = manifest_by_instance.get(instance, {})
            stats = stats_by_instance.get(instance, {})
            summary_rows.append(
                {
                    "mode": mode,
                    "seed": manifest.get("seed", ""),
                    "instance": instance,
                    "source": manifest.get("source", ""),
                    "result": result.get("result", ""),
                    "verified": result.get("verified", ""),
                    "time_s": result.get("time_s", ""),
                    "conflicts": stats.get("conflicts", ""),
                    "decisions": stats.get("decisions", ""),
                    "propagations": stats.get("propagations", ""),
                    "unknown_reason": stats.get("unknown_reason", ""),
                    "bench_log_dir": str(bench_dir.relative_to(REPO_ROOT)),
                }
            )

    write_summary(work_dir / "summary.csv", summary_rows)
    print(f"wrote {work_dir / 'summary.csv'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
