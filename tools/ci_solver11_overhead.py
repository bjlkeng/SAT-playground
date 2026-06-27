#!/usr/bin/env python3
"""Interleaved reference-floor vs current-target overhead regression gate."""

from __future__ import annotations

import argparse
import csv
import json
import os
import re
import shutil
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from datetime import datetime
from pathlib import Path
from statistics import mean, median


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_INSTANCES = [
    REPO_ROOT / "benchmarks/profiling/legacy/feistel_b64_k57_r18.cnf",
    REPO_ROOT / "benchmarks/profiling/legacy/random_v285_s2.cnf",
]
SOLVERS = {
    "floor": REPO_ROOT / os.environ.get("SAT_REFERENCE_SOLVER", "solver/10-bve-subsume"),
    "target": REPO_ROOT,
}
SEARCH_DONE_RE = re.compile(
    r"c search done result=(?P<result>\S+) seconds=(?P<seconds>[0-9.]+) "
    r"conflicts=(?P<conflicts>\d+) decisions=(?P<decisions>\d+) "
    r"propagations=(?P<propagations>\d+) restarts=(?P<restarts>\d+) "
    r"learned=(?P<learned>\d+) reduce_db=(?P<reduce_db>\d+)"
)


def solver_sort_key(path: Path) -> tuple[int, str]:
    prefix = path.name.split("-", 1)[0]
    try:
        return int(prefix), path.name
    except ValueError:
        return -1, path.name


def resolve_solver(value: str | None) -> Path:
    value = value or os.environ.get("SAT_CURRENT_SOLVER") or os.environ.get("SAT_SOLVER")
    if value:
        path = Path(value)
        return path if path.is_absolute() else REPO_ROOT / path
    candidates = [
        path for path in (REPO_ROOT / "solver").glob("[0-9][0-9]-*")
        if (path / "build.sh").is_file() and (path / "run.sh").is_file()
    ]
    if not candidates:
        raise SystemExit("no solver/NN-* directory with build.sh and run.sh found")
    return sorted(candidates, key=solver_sort_key)[-1]


def configure_solvers(args: argparse.Namespace) -> None:
    global SOLVERS
    SOLVERS = {
        "floor": resolve_solver(args.floor_solver),
        "target": resolve_solver(args.target_solver),
    }
    for label, solver_dir in SOLVERS.items():
        if not (solver_dir / "build.sh").is_file() or not (solver_dir / "run.sh").is_file():
            raise SystemExit(f"{label} solver is invalid: {solver_dir}")


@dataclass
class RunRecord:
    instance: str
    solver: str
    repeat: int
    order_index: int
    seconds: float
    status: str
    returncode: int
    timeout: bool
    stdout_path: str
    stderr_path: str
    output_dir: str


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run an interleaved reference-floor vs target overhead gate."
    )
    parser.add_argument(
        "--repeats",
        type=int,
        default=int(os.environ.get("SAT_OVERHEAD_REPEATS", "3")),
        help="Alternating-order repeats per instance, default: 3.",
    )
    parser.add_argument(
        "--threshold-pct",
        type=float,
        default=float(os.environ.get("SAT_OVERHEAD_THRESHOLD_PCT", "1.5")),
        help="Fail when target median overhead exceeds this percentage, default: 1.5.",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=int(os.environ.get("SAT_OVERHEAD_TIMEOUT", "120")),
        help="Per-run Python timeout in seconds, default: 120.",
    )
    parser.add_argument(
        "--log-dir",
        type=Path,
        default=None,
        help="Artifact directory, default: log/solver-overhead-<target>-<timestamp>.",
    )
    parser.add_argument(
        "--floor-solver",
        default=os.environ.get("SAT_REFERENCE_SOLVER", "solver/10-bve-subsume"),
        help="Reference floor solver directory, default: SAT_REFERENCE_SOLVER or solver/10-bve-subsume.",
    )
    parser.add_argument(
        "--target-solver",
        default=os.environ.get("SAT_TARGET_SOLVER"),
        help="Target solver directory, default: SAT_TARGET_SOLVER, SAT_CURRENT_SOLVER, SAT_SOLVER, or current solver.",
    )
    parser.add_argument(
        "--instance",
        action="append",
        type=Path,
        dest="instances",
        help="CNF instance to run. May be provided multiple times.",
    )
    parser.add_argument(
        "--proof",
        choices=["default", "off", "drat"],
        default=os.environ.get("SAT_OVERHEAD_PROOF", "default"),
        help="SAT_PROOF setting for timing runs, default leaves SAT_PROOF unset.",
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="Use existing release binaries instead of running each build.sh.",
    )
    return parser.parse_args()


def clean_sat_env(extra: dict[str, str] | None = None) -> dict[str, str]:
    env = os.environ.copy()
    for name in list(env):
        if name.startswith("SAT_"):
            env.pop(name)
    if extra:
        env.update(extra)
    return env


def proof_env(proof: str) -> dict[str, str]:
    if proof == "default":
        return clean_sat_env()
    return clean_sat_env({"SAT_PROOF": proof})


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(REPO_ROOT))
    except ValueError:
        return str(path)


def sanitize(text: str) -> str:
    return re.sub(r"[^A-Za-z0-9_.-]+", "_", text)


def build_solvers(log_dir: Path) -> None:
    for name, solver_dir in SOLVERS.items():
        build_log = log_dir / f"build-{name}.log"
        with build_log.open("w", encoding="utf-8") as out:
            proc = subprocess.run(
                ["bash", "build.sh"],
                cwd=solver_dir,
                stdout=out,
                stderr=subprocess.STDOUT,
                text=True,
            )
        if proc.returncode != 0:
            raise SystemExit(f"{name} build failed; see {build_log}")


def extract_status(stdout: str, returncode: int) -> str:
    for line in stdout.splitlines():
        if line.startswith("s "):
            return line[2:].strip()
    return f"rc={returncode}"


def remove_large_proofs(output_dir: Path) -> None:
    for name in ("proof.out", "proof.out.tmp"):
        path = output_dir / name
        if path.exists():
            path.unlink()


def run_solver(
    solver: str,
    instance: Path,
    output_dir: Path,
    stdout_path: Path,
    stderr_path: Path,
    env: dict[str, str],
    timeout: int,
) -> tuple[float, str, int, bool, str, str]:
    run_sh = SOLVERS[solver] / "run.sh"
    start = time.perf_counter()
    timed_out = False
    try:
        proc = subprocess.run(
            ["bash", str(run_sh), str(instance), str(output_dir)],
            cwd=REPO_ROOT,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
        )
        stdout = proc.stdout
        stderr = proc.stderr
        returncode = proc.returncode
    except subprocess.TimeoutExpired as err:
        timed_out = True
        stdout = (err.stdout or "") if isinstance(err.stdout, str) else ""
        stderr = (err.stderr or "") if isinstance(err.stderr, str) else ""
        returncode = 124
    seconds = time.perf_counter() - start
    stdout_path.write_text(stdout, encoding="utf-8")
    stderr_path.write_text(stderr, encoding="utf-8")
    remove_large_proofs(output_dir)
    return seconds, extract_status(stdout, returncode), returncode, timed_out, stdout, stderr


def run_timing_matrix(
    instances: list[Path],
    repeats: int,
    log_dir: Path,
    timeout: int,
    proof: str,
) -> list[RunRecord]:
    records: list[RunRecord] = []
    timing_dir = log_dir / "timing"
    timing_dir.mkdir(parents=True, exist_ok=True)
    env = proof_env(proof)
    solver_order = ["floor", "target"]

    for instance in instances:
        for repeat in range(1, repeats + 1):
            order = solver_order if repeat % 2 == 1 else list(reversed(solver_order))
            for order_index, solver in enumerate(order, start=1):
                run_id = f"{sanitize(instance.stem)}-r{repeat}-{order_index}-{solver}"
                output_dir = timing_dir / run_id / "out"
                output_dir.mkdir(parents=True, exist_ok=True)
                stdout_path = timing_dir / run_id / "stdout.txt"
                stderr_path = timing_dir / run_id / "stderr.txt"
                seconds, status, returncode, timed_out, _, _ = run_solver(
                    solver,
                    instance,
                    output_dir,
                    stdout_path,
                    stderr_path,
                    env,
                    timeout,
                )
                record = RunRecord(
                    instance=rel(instance),
                    solver=solver,
                    repeat=repeat,
                    order_index=order_index,
                    seconds=seconds,
                    status=status,
                    returncode=returncode,
                    timeout=timed_out,
                    stdout_path=rel(stdout_path),
                    stderr_path=rel(stderr_path),
                    output_dir=rel(output_dir),
                )
                records.append(record)
                print(
                    f"{instance.name} repeat={repeat} order={order_index} "
                    f"{solver} {seconds:.6f}s {status}",
                    flush=True,
                )
    return records


def parse_search_done(text: str) -> dict[str, object] | None:
    matches = list(SEARCH_DONE_RE.finditer(text))
    if not matches:
        return None
    match = matches[-1]
    parsed: dict[str, object] = match.groupdict()
    parsed["seconds"] = float(parsed["seconds"])
    for key in ("conflicts", "decisions", "propagations", "restarts", "learned", "reduce_db"):
        parsed[key] = int(parsed[key])
    return parsed


def run_counter_parity(instance: Path, log_dir: Path, timeout: int) -> dict[str, object]:
    parity_dir = log_dir / "counter-parity"
    parity_dir.mkdir(parents=True, exist_ok=True)
    env = clean_sat_env(
        {
            "SAT_PROOF": "off",
            "SAT_TRACE_SEARCH_INTERVAL": "1000000000",
        }
    )
    result: dict[str, object] = {
        "instance": rel(instance),
        "env": {
            "SAT_PROOF": "off",
            "SAT_TRACE_SEARCH_INTERVAL": "1000000000",
        },
        "solvers": {},
        "matching_core_counters": False,
    }
    for solver in ("floor", "target"):
        output_dir = parity_dir / solver / "out"
        output_dir.mkdir(parents=True, exist_ok=True)
        stdout_path = parity_dir / solver / "stdout.txt"
        stderr_path = parity_dir / solver / "stderr.txt"
        seconds, status, returncode, timed_out, stdout, stderr = run_solver(
            solver,
            instance,
            output_dir,
            stdout_path,
            stderr_path,
            env,
            timeout,
        )
        result["solvers"][solver] = {
            "seconds": seconds,
            "status": status,
            "returncode": returncode,
            "timeout": timed_out,
            "stdout_path": rel(stdout_path),
            "stderr_path": rel(stderr_path),
            "output_dir": rel(output_dir),
            "search_done": parse_search_done(stdout + "\n" + stderr),
        }

    left = result["solvers"]["floor"]["search_done"]
    right = result["solvers"]["target"]["search_done"]
    core_keys = ["result", "conflicts", "decisions", "propagations", "restarts"]
    result["matching_core_counters"] = bool(
        left and right and all(left[key] == right[key] for key in core_keys)
    )
    return result


def summarize(records: list[RunRecord], threshold_pct: float) -> dict[str, object]:
    by_instance: dict[str, dict[str, list[RunRecord]]] = {}
    for record in records:
        by_instance.setdefault(record.instance, {}).setdefault(record.solver, []).append(record)

    summary: dict[str, object] = {
        "threshold_pct": threshold_pct,
        "instances": {},
        "failed_instances": [],
    }
    for instance, solver_records in by_instance.items():
        floor_records = solver_records.get("floor", [])
        target_records = solver_records.get("target", [])
        floor_times = [record.seconds for record in floor_records]
        target_times = [record.seconds for record in target_records]
        floor_median = median(floor_times)
        target_median = median(target_times)
        floor_mean = mean(floor_times)
        target_mean = mean(target_times)
        median_delta_pct = ((target_median - floor_median) / floor_median) * 100.0
        mean_delta_pct = ((target_mean - floor_mean) / floor_mean) * 100.0
        status_ok = (
            [record.status for record in floor_records] == [record.status for record in target_records]
            and all(record.returncode == 0 for record in floor_records + target_records)
            and not any(record.timeout for record in floor_records + target_records)
        )
        passes = status_ok and median_delta_pct <= threshold_pct
        if not passes:
            summary["failed_instances"].append(instance)
        summary["instances"][instance] = {
            "floor_times": floor_times,
            "target_times": target_times,
            "floor_statuses": [record.status for record in floor_records],
            "target_statuses": [record.status for record in target_records],
            "floor_median": floor_median,
            "target_median": target_median,
            "median_delta_pct_target_vs_floor": median_delta_pct,
            "floor_mean": floor_mean,
            "target_mean": target_mean,
            "mean_delta_pct_target_vs_floor": mean_delta_pct,
            "status_ok": status_ok,
            "passes_threshold": passes,
        }
    summary["passed"] = not summary["failed_instances"]
    return summary


def write_records(records: list[RunRecord], path: Path) -> None:
    with path.open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(fh, fieldnames=list(asdict(records[0]).keys()))
        writer.writeheader()
        for record in records:
            writer.writerow(asdict(record))


def main() -> int:
    args = parse_args()
    configure_solvers(args)
    if args.repeats <= 0:
        raise SystemExit("--repeats must be positive")
    instances = [path.resolve() for path in (args.instances or DEFAULT_INSTANCES)]
    for instance in instances:
        if not instance.exists():
            raise SystemExit(f"instance not found: {instance}")

    timestamp = datetime.now().strftime("%Y-%m-%d-%H-%M-%S")
    target_name = sanitize(SOLVERS["target"].name)
    log_dir = (args.log_dir or (REPO_ROOT / "log" / f"solver-overhead-{target_name}-{timestamp}")).resolve()
    if log_dir.exists():
        shutil.rmtree(log_dir)
    log_dir.mkdir(parents=True)

    config = {
        "generated_at": datetime.now().isoformat(timespec="seconds"),
        "repeats": args.repeats,
        "threshold_pct": args.threshold_pct,
        "timeout_sec": args.timeout,
        "proof_setting": args.proof,
        "instances": [rel(path) for path in instances],
        "solvers": {name: rel(path) for name, path in SOLVERS.items()},
        "log_dir": rel(log_dir),
    }
    (log_dir / "config.json").write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")

    if not args.skip_build:
        build_solvers(log_dir)

    records = run_timing_matrix(instances, args.repeats, log_dir, args.timeout, args.proof)
    write_records(records, log_dir / "runs.csv")
    summary = summarize(records, args.threshold_pct)
    parity = run_counter_parity(instances[0], log_dir, args.timeout)
    summary["counter_parity"] = parity
    summary["passed"] = bool(summary["passed"] and parity["matching_core_counters"])
    (log_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")

    print("\n=== solver overhead summary ===")
    print(f"Artifacts: {rel(log_dir)}")
    print(f"Floor solver: {rel(SOLVERS['floor'])}")
    print(f"Target solver: {rel(SOLVERS['target'])}")
    print(f"Threshold: {args.threshold_pct:.3f}% median target overhead")
    for instance, item in summary["instances"].items():
        print(
            f"{Path(instance).name}: floor_median={item['floor_median']:.6f}s "
            f"target_median={item['target_median']:.6f}s "
            f"delta={item['median_delta_pct_target_vs_floor']:.3f}% "
            f"mean_delta={item['mean_delta_pct_target_vs_floor']:.3f}% "
            f"status_ok={item['status_ok']}"
        )
    print(f"Counter parity: {parity['matching_core_counters']}")

    if not summary["passed"]:
        print("solver overhead gate FAILED", file=sys.stderr)
        return 1
    print("solver overhead gate PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
