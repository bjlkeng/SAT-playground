#!/usr/bin/env python3
"""Interleaved solver 10 vs solver 11 overhead regression gate."""

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
    "solver10": REPO_ROOT / "solver/10-bve-subsume",
    "solver11": REPO_ROOT / "solver/11-kissat-search",
}
SEARCH_DONE_RE = re.compile(
    r"c search done result=(?P<result>\S+) seconds=(?P<seconds>[0-9.]+) "
    r"conflicts=(?P<conflicts>\d+) decisions=(?P<decisions>\d+) "
    r"propagations=(?P<propagations>\d+) restarts=(?P<restarts>\d+) "
    r"learned=(?P<learned>\d+) reduce_db=(?P<reduce_db>\d+)"
)


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
        description="Run an interleaved solver 10 vs solver 11 overhead gate."
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
        help="Fail when solver 11 median overhead exceeds this percentage, default: 1.5.",
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
        help="Artifact directory, default: log/solver11-overhead-<timestamp>.",
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
    solver_order = ["solver10", "solver11"]

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
    for solver in ("solver10", "solver11"):
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

    left = result["solvers"]["solver10"]["search_done"]
    right = result["solvers"]["solver11"]["search_done"]
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
        s10 = solver_records.get("solver10", [])
        s11 = solver_records.get("solver11", [])
        times10 = [record.seconds for record in s10]
        times11 = [record.seconds for record in s11]
        med10 = median(times10)
        med11 = median(times11)
        mean10 = mean(times10)
        mean11 = mean(times11)
        median_delta_pct = ((med11 - med10) / med10) * 100.0
        mean_delta_pct = ((mean11 - mean10) / mean10) * 100.0
        status_ok = (
            [record.status for record in s10] == [record.status for record in s11]
            and all(record.returncode == 0 for record in s10 + s11)
            and not any(record.timeout for record in s10 + s11)
        )
        passes = status_ok and median_delta_pct <= threshold_pct
        if not passes:
            summary["failed_instances"].append(instance)
        summary["instances"][instance] = {
            "solver10_times": times10,
            "solver11_times": times11,
            "solver10_statuses": [record.status for record in s10],
            "solver11_statuses": [record.status for record in s11],
            "median10": med10,
            "median11": med11,
            "median_delta_pct_solver11_vs_10": median_delta_pct,
            "mean10": mean10,
            "mean11": mean11,
            "mean_delta_pct_solver11_vs_10": mean_delta_pct,
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
    if args.repeats <= 0:
        raise SystemExit("--repeats must be positive")
    instances = [path.resolve() for path in (args.instances or DEFAULT_INSTANCES)]
    for instance in instances:
        if not instance.exists():
            raise SystemExit(f"instance not found: {instance}")

    timestamp = datetime.now().strftime("%Y-%m-%d-%H-%M-%S")
    log_dir = (args.log_dir or (REPO_ROOT / "log" / f"solver11-overhead-{timestamp}")).resolve()
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

    print("\n=== solver11 overhead summary ===")
    print(f"Artifacts: {rel(log_dir)}")
    print(f"Threshold: {args.threshold_pct:.3f}% median solver11 overhead")
    for instance, item in summary["instances"].items():
        print(
            f"{Path(instance).name}: median10={item['median10']:.6f}s "
            f"median11={item['median11']:.6f}s "
            f"delta={item['median_delta_pct_solver11_vs_10']:.3f}% "
            f"mean_delta={item['mean_delta_pct_solver11_vs_10']:.3f}% "
            f"status_ok={item['status_ok']}"
        )
    print(f"Counter parity: {parity['matching_core_counters']}")

    if not summary["passed"]:
        print("solver11 overhead gate FAILED", file=sys.stderr)
        return 1
    print("solver11 overhead gate PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
