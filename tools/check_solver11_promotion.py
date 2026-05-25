#!/usr/bin/env python3
"""Guard solver 11 default/fast promotions against solver 10 regressions."""

from __future__ import annotations

import argparse
import json
import os
import platform
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

import compare_bench


SOLVER_PROCESS_NAMES = ("sat-solver", "minisat", "kissat")


def read_results(path: Path) -> dict[str, dict[str, str]]:
    return compare_bench.read_rows(path)


def par2(rows: dict[str, dict[str, str]], timeout: float) -> float:
    return compare_bench.par2(rows, timeout)


def solved_count(rows: dict[str, dict[str, str]]) -> int:
    return compare_bench.solved_count(rows)


def result_counts(rows: dict[str, dict[str, str]]) -> dict[str, int]:
    return dict(sorted(Counter(row["result"] for row in rows.values()).items()))


def row_timeout_values(rows: dict[str, dict[str, str]]) -> list[str]:
    return sorted({str(row.get("timeout", "")).strip() for row in rows.values()})


def running_solver_processes() -> list[str]:
    try:
        result = subprocess.run(
            ["pgrep", "-a", "-f", "|".join(SOLVER_PROCESS_NAMES)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except OSError:
        return ["process_check_unavailable"]
    lines = []
    self_pid = str(os.getpid())
    for raw in result.stdout.splitlines():
        line = raw.strip()
        if not line or line.split(maxsplit=1)[0] == self_pid:
            continue
        if "check_solver11_promotion.py" in line or "pgrep -a -f" in line:
            continue
        lines.append(line)
    return lines


def machine_environment(timeout: float, memory_mb: int | None) -> dict[str, str]:
    env = compare_bench.machine_block(timeout)
    env["memory_limit"] = str(memory_mb) if memory_mb is not None else "not_provided"
    env["os"] = platform.platform()
    return env


def same_instance_set(left: dict[str, dict[str, str]], right: dict[str, dict[str, str]]) -> tuple[list[str], list[str]]:
    left_names = set(left)
    right_names = set(right)
    return sorted(left_names - right_names), sorted(right_names - left_names)


def status_regressions_against_solver10(
    solver10: dict[str, dict[str, str]], candidate: dict[str, dict[str, str]]
) -> list[tuple[str, str, str]]:
    regressions = []
    for name in sorted(set(solver10) & set(candidate)):
        before = solver10[name]["result"]
        after = candidate[name]["result"]
        if compare_bench.is_status_regression(before, after):
            regressions.append((name, before, after))
    return regressions


def check_gate(args: argparse.Namespace, process_lines: list[str] | None = None) -> int:
    solver10 = read_results(args.solver10)
    candidate = read_results(args.candidate)
    previous = read_results(args.previous) if args.previous is not None else None

    failures: list[str] = []
    warnings: list[str] = []

    if args.previous is None:
        failures.append("previous_solver11_required")
    if args.memory_mb is None:
        failures.append("memory_mb_required")

    missing_candidate, extra_candidate = same_instance_set(solver10, candidate)
    if missing_candidate or extra_candidate:
        failures.append("candidate_instance_set_differs_from_solver10")

    if previous is not None:
        missing_previous, extra_previous = same_instance_set(previous, candidate)
        if missing_previous or extra_previous:
            failures.append("candidate_instance_set_differs_from_previous_solver11")
    else:
        missing_previous = []
        extra_previous = []

    timeout_sets = {
        "solver10": row_timeout_values(solver10),
        "candidate": row_timeout_values(candidate),
    }
    if previous is not None:
        timeout_sets["previous"] = row_timeout_values(previous)
    if len({tuple(values) for values in timeout_sets.values()}) > 1:
        failures.append("timeout_columns_do_not_match")

    candidate_validation, validation_warnings = compare_bench.read_validation(args.candidate, candidate)
    warnings.extend(validation_warnings)
    failures.extend(compare_bench.correctness_failures(candidate, {}, candidate_validation))

    status_regressions = status_regressions_against_solver10(solver10, candidate)
    if status_regressions:
        failures.append("candidate_status_regresses_solver10")

    process_lines = running_solver_processes() if process_lines is None else process_lines
    if process_lines and not args.allow_running_solvers:
        failures.append("running_solver_processes_detected")

    solver10_par2 = par2(solver10, args.timeout)
    candidate_par2 = par2(candidate, args.timeout)
    previous_par2 = par2(previous, args.timeout) if previous is not None else None
    solver10_margin = max(0.0, solver10_par2 * args.tolerance_fraction)
    candidate_loses_solver10 = candidate_par2 > solver10_par2 + solver10_margin
    candidate_improves_previous = (
        previous_par2 is not None and candidate_par2 < previous_par2 - max(0.0, previous_par2 * args.tolerance_fraction)
    )

    decision_required = "none"
    if candidate_loses_solver10:
        if candidate_improves_previous:
            decision_required = "candidate_improves_previous_solver11_but_loses_solver10"
        else:
            decision_required = "candidate_loses_solver10"
        failures.append(decision_required)

    print(f"solver10={args.solver10}")
    print(f"candidate={args.candidate}")
    print(f"previous={args.previous or 'none'}")
    print(f"timeout_s={args.timeout:g}")
    print(f"memory_mb={args.memory_mb if args.memory_mb is not None else 'not_provided'}")
    print("machine_environment=" + json.dumps(machine_environment(args.timeout, args.memory_mb), sort_keys=True))
    print("running_solver_processes=" + json.dumps(process_lines))
    print("timeout_columns=" + json.dumps(timeout_sets, sort_keys=True))
    print(f"solver10_PAR2={solver10_par2:.3f}")
    if previous_par2 is not None:
        print(f"previous_solver11_PAR2={previous_par2:.3f}")
    print(f"candidate_PAR2={candidate_par2:.3f}")
    print(f"candidate_minus_solver10_PAR2={candidate_par2 - solver10_par2:.3f}")
    if previous_par2 is not None:
        print(f"candidate_minus_previous_solver11_PAR2={candidate_par2 - previous_par2:.3f}")
    print(f"solver10_solved={solved_count(solver10)}")
    if previous is not None:
        print(f"previous_solver11_solved={solved_count(previous)}")
    print(f"candidate_solved={solved_count(candidate)}")
    print("solver10_counts=" + json.dumps(result_counts(solver10), sort_keys=True))
    if previous is not None:
        print("previous_solver11_counts=" + json.dumps(result_counts(previous), sort_keys=True))
    print("candidate_counts=" + json.dumps(result_counts(candidate), sort_keys=True))
    print("missing_from_candidate_vs_solver10=" + json.dumps(missing_candidate))
    print("extra_in_candidate_vs_solver10=" + json.dumps(extra_candidate))
    print("missing_from_candidate_vs_previous=" + json.dumps(missing_previous))
    print("extra_in_candidate_vs_previous=" + json.dumps(extra_previous))
    print("status_regressions_vs_solver10=" + json.dumps(status_regressions))
    print("validation_warnings=" + json.dumps(warnings))
    print("decision_required=" + decision_required)
    print("failures=" + json.dumps(failures))

    if failures:
        print("promotion_gate=FAIL")
        return 1
    print("promotion_gate=PASS")
    return 0


def write_csv(path: Path, rows: list[tuple[str, str, float]]) -> None:
    lines = ["instance,result,verified,time_s,timeout,exit_code"]
    for name, result, seconds in rows:
        lines.append(f"{name},{result},ok,{seconds:.3f},10,0")
    path.write_text("\n".join(lines) + "\n")


def self_test() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        solver10 = root / "solver10.csv"
        previous = root / "previous.csv"
        candidate_loses = root / "candidate-loses.csv"
        candidate_wins = root / "candidate-wins.csv"
        write_csv(solver10, [("a", "SAT", 3.0), ("b", "UNSAT", 4.0)])
        write_csv(previous, [("a", "SAT", 8.0), ("b", "UNSAT", 8.0)])
        write_csv(candidate_loses, [("a", "SAT", 5.0), ("b", "UNSAT", 5.0)])
        write_csv(candidate_wins, [("a", "SAT", 2.0), ("b", "UNSAT", 3.0)])

        base = argparse.Namespace(
            solver10=solver10,
            previous=previous,
            timeout=10.0,
            memory_mb=16384,
            tolerance_fraction=0.0,
            allow_running_solvers=False,
        )
        loses = argparse.Namespace(**{**vars(base), "candidate": candidate_loses})
        if check_gate(loses, process_lines=[]) == 0:
            raise AssertionError("candidate that improves previous but loses solver10 must fail")
        wins = argparse.Namespace(**{**vars(base), "candidate": candidate_wins})
        if check_gate(wins, process_lines=[]) != 0:
            raise AssertionError("candidate that beats solver10 must pass")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--solver10", type=Path)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--previous", type=Path)
    parser.add_argument("--timeout", type=float, default=300.0)
    parser.add_argument("--memory-mb", type=int, default=None)
    parser.add_argument("--tolerance-fraction", type=float, default=0.01)
    parser.add_argument("--allow-running-solvers", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        print("SELFTEST ok")
        return 0

    if args.solver10 is None or args.candidate is None:
        parser.error("--solver10 and --candidate are required unless --self-test is used")
    return check_gate(args)


if __name__ == "__main__":
    sys.exit(main())
