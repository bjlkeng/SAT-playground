#!/usr/bin/env python3
"""Guard current-solver default/fast promotions against a reference floor.

The floor is an **aggregate-PAR-2 floor**, not a per-instance floor: an instance that
regresses from solved to a timeout/UNKNOWN is a PAR-2 cost handled by the aggregate
comparison, not a gate failure. The only per-instance floor check the gate still
enforces is a SAT<->UNSAT **correctness contradiction** (PAR-2 can never buy back a
wrong answer). Honest solved->unsolved regressions vs solver 10 are reported for
diagnostics but do not fail the gate.
"""

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
        if "check_promotion_gate.py" in line or "pgrep -a -f" in line:
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
    """All status regressions vs solver 10 (solved->unsolved, ->ERROR, or SAT<->UNSAT).

    Reported for diagnostics only; honest solved->unsolved rows are a PAR-2 cost and do
    NOT fail the gate. The failing subset is computed by status_contradictions_against_solver10.
    """
    regressions = []
    for name in sorted(set(solver10) & set(candidate)):
        before = solver10[name]["result"]
        after = candidate[name]["result"]
        if compare_bench.is_status_regression(before, after):
            regressions.append((name, before, after))
    return regressions


def status_contradictions_against_solver10(
    solver10: dict[str, dict[str, str]], candidate: dict[str, dict[str, str]]
) -> list[tuple[str, str, str]]:
    """SAT<->UNSAT disagreements vs solver 10 — a correctness signal PAR-2 cannot price.

    One of the two solvers is wrong on these instances, so the candidate must not ship
    regardless of aggregate PAR-2. (A solver-10-solved instance the candidate merely fails
    to solve in time is NOT a contradiction; that is priced into PAR-2.)
    """
    contradictions = []
    for name in sorted(set(solver10) & set(candidate)):
        before = solver10[name]["result"]
        after = candidate[name]["result"]
        if before in compare_bench.SOLVED and after in compare_bench.SOLVED and before != after:
            contradictions.append((name, before, after))
    return contradictions


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

    # Solver 10 is an aggregate-PAR-2 floor, not a per-instance floor: a solved->unsolved
    # regression is reported but priced into PAR-2, not a gate failure. Only a SAT<->UNSAT
    # correctness contradiction vs solver 10 fails the gate.
    status_regressions = status_regressions_against_solver10(solver10, candidate)
    status_contradictions = status_contradictions_against_solver10(solver10, candidate)
    if status_contradictions:
        failures.append("candidate_contradicts_solver10")

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
    print("status_regressions_vs_solver10=" + json.dumps(status_regressions) + "  # informational (priced into PAR-2)")
    print("status_contradictions_vs_solver10=" + json.dumps(status_contradictions))
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

        # Solver-10 is an aggregate floor, not a per-instance floor: a candidate that flips
        # a solver-10-solved instance to a timeout but still wins aggregate PAR-2 must PASS.
        solver10b = root / "solver10b.csv"
        previousb = root / "previousb.csv"
        candidate_reg = root / "candidate-regress-but-aggregate-wins.csv"
        candidate_contra = root / "candidate-contradicts.csv"
        write_csv(solver10b, [("a", "SAT", 9.0), ("b", "SAT", 9.0), ("c", "SAT", 9.0)])      # PAR2 27
        write_csv(previousb, [("a", "SAT", 9.5), ("b", "SAT", 9.5), ("c", "SAT", 9.5)])      # PAR2 28.5
        write_csv(candidate_reg, [("a", "TIMEOUT", 10.0), ("b", "SAT", 0.1), ("c", "SAT", 0.1)])  # PAR2 20.2
        write_csv(candidate_contra, [("a", "UNSAT", 1.0), ("b", "SAT", 1.0), ("c", "SAT", 1.0)])  # contradicts a

        base_b = argparse.Namespace(**{**vars(base), "solver10": solver10b, "previous": previousb})
        reg = argparse.Namespace(**{**vars(base_b), "candidate": candidate_reg})
        if check_gate(reg, process_lines=[]) != 0:
            raise AssertionError("solved->timeout regression that wins aggregate PAR-2 must pass")
        contra = argparse.Namespace(**{**vars(base_b), "candidate": candidate_contra})
        if check_gate(contra, process_lines=[]) == 0:
            raise AssertionError("SAT<->UNSAT contradiction vs solver10 must fail")

        _self_test_multiseed(root)


def write_seed_tsv(path: Path, config: str, rows: list[tuple[str, str, str, float, int]]) -> None:
    """rows: (instance, seed, result, time_s, conflicts)."""
    lines = ["config\tinstance\tseed\tresult\ttime_s\tconflicts\tpropagations\tdecisions\ttimeout"]
    for inst, seed, res, t, cf in rows:
        lines.append(f"{config}\t{inst}\t{seed}\t{res}\t{t:.3f}\t{cf}\t0\t0\t600")
    path.write_text("\n".join(lines) + "\n")


def _self_test_multiseed(root: Path) -> None:
    # solver10 + previous both solve a@2seeds, b@2seeds; conflicts modest.
    def mk(name, cfg, spec):
        p = root / name
        write_seed_tsv(p, cfg, spec)
        return p
    seeds = ["0", "1"]
    s10 = mk("ms_s10.tsv", "solver10",
             [("a", s, "SAT", 5, 100) for s in seeds] + [("b", s, "UNSAT", 5, 100) for s in seeds])
    prev = mk("ms_prev.tsv", "default",
              [("a", s, "SAT", 6, 120) for s in seeds] + [("b", s, "UNSAT", 6, 120) for s in seeds])
    base = dict(timeout=600.0, memory_mb=16384, tolerance_fraction=0.0,
                allow_running_solvers=False, multiseed=True, previous=prev, solver10=s10)

    # WIN: candidate solves same, fewer conflicts than previous AND >= solver10 lexicographically.
    cand_win = mk("ms_cand_win.tsv", "cand",
                  [("a", s, "SAT", 6, 90) for s in seeds] + [("b", s, "UNSAT", 6, 90) for s in seeds])
    if check_gate_multiseed(argparse.Namespace(**{**base, "candidate": cand_win}), process_lines=[]) != 0:
        raise AssertionError("multiseed: fewer-conflicts candidate (beats prev, >=solver10) must PASS")

    # FAIL floor: candidate solves FEWER than solver10 (loses the floor lexicographically).
    cand_lose = mk("ms_cand_lose.tsv", "cand",
                   [("a", s, "SAT", 6, 90) for s in seeds] + [("b", s, "TIMEOUT", 600, 0) for s in seeds])
    if check_gate_multiseed(argparse.Namespace(**{**base, "candidate": cand_lose}), process_lines=[]) == 0:
        raise AssertionError("multiseed: candidate solving fewer than solver10 must FAIL the floor")

    # FAIL: binary_fast lesson — same solved, MORE conflicts than solver10 (faster PAR-2 cannot save it).
    cand_moreconf = mk("ms_cand_moreconf.tsv", "cand",
                       [("a", s, "SAT", 1, 500) for s in seeds] + [("b", s, "UNSAT", 1, 500) for s in seeds])
    if check_gate_multiseed(argparse.Namespace(**{**base, "candidate": cand_moreconf}), process_lines=[]) == 0:
        raise AssertionError("multiseed: equal-solved + more-conflicts-than-solver10 must FAIL despite faster PAR-2")

    # FAIL: SAT<->UNSAT contradiction vs solver10.
    cand_contra = mk("ms_cand_contra.tsv", "cand",
                     [("a", s, "UNSAT", 5, 90) for s in seeds] + [("b", s, "UNSAT", 5, 90) for s in seeds])
    if check_gate_multiseed(argparse.Namespace(**{**base, "candidate": cand_contra}), process_lines=[]) == 0:
        raise AssertionError("multiseed: SAT<->UNSAT contradiction vs solver10 must FAIL")


def check_gate_multiseed(args: argparse.Namespace, process_lines: list[str] | None = None) -> int:
    """Multi-seed lexicographic gate (2026-06-02 procedure).

    Inputs are feature_ablation per-(config,instance,seed) TSVs for solver10/previous/candidate.
    Decision metric is LEXICOGRAPHIC solved -> conflicts(both-solved) -> PAR-2, evaluated:
      * candidate vs previous-solver11-default = the promote/keep decision
      * candidate vs solver10 = the floor (must not lose lexicographically)
    All structural guards from the single-CSV gate are preserved: matching (instance,seed) cell sets
    across configs, correctness failures, SAT<->UNSAT contradictions (vs both previous and solver10),
    process sanity, and required --previous/--memory-mb.
    """
    cb = compare_bench
    s10_cells = cb.seed_cells_by_config(cb.read_seed_tsv(args.solver10))
    cand_cells = cb.seed_cells_by_config(cb.read_seed_tsv(args.candidate))
    prev_cells = cb.seed_cells_by_config(cb.read_seed_tsv(args.previous)) if args.previous else None

    # each TSV should contain exactly one config; collapse to its cells
    def only(cells_by_cfg, label):
        if len(cells_by_cfg) != 1:
            failures.append(f"{label}_tsv_must_contain_exactly_one_config")
            # merge anyway so downstream doesn't crash
            merged = {}
            for d in cells_by_cfg.values():
                merged.update(d)
            return merged
        return next(iter(cells_by_cfg.values()))

    failures: list[str] = []
    warnings: list[str] = []
    if args.previous is None:
        failures.append("previous_solver11_required")
    if args.memory_mb is None:
        failures.append("memory_mb_required")

    s10 = only(s10_cells, "solver10")
    cand = only(cand_cells, "candidate")
    prev = only(prev_cells, "previous") if prev_cells is not None else {}

    # matching (instance,seed) cell sets
    if set(cand) != set(s10):
        failures.append("candidate_cells_differ_from_solver10")
    if prev_cells is not None and set(cand) != set(prev):
        failures.append("candidate_cells_differ_from_previous_solver11")

    # correctness: any ERROR/PARSE_ERROR cell in the candidate
    bad = sorted(f"{k[0]}@seed{k[1]}={c['result']}" for k, c in cand.items()
                 if c["result"].upper() in {"ERROR", "PARSE_ERROR"})
    if bad:
        failures.append("candidate_correctness_failures")

    # SAT<->UNSAT contradictions per (instance,seed) vs both references
    contra_s10 = cb.seed_contradictions(cand, s10)
    contra_prev = cb.seed_contradictions(cand, prev) if prev else []
    if contra_s10:
        failures.append("candidate_contradicts_solver10")
    if contra_prev:
        failures.append("candidate_contradicts_previous_solver11")

    process_lines = running_solver_processes() if process_lines is None else process_lines
    if process_lines and not args.allow_running_solvers:
        failures.append("running_solver_processes_detected")

    t = args.timeout
    s10_score = cb.lexicographic_score(s10, t)
    cand_score = cb.lexicographic_score(cand, t)
    prev_score = cb.lexicographic_score(prev, t) if prev else None

    # candidate vs solver10 floor (lexicographic)
    bt_s10 = cb.both_solved_conflict_totals(cand, s10)
    floor_decision, floor_reason = cb.lexicographic_decision(cand_score, s10_score, bt_s10)
    candidate_loses_solver10 = floor_decision == "regress"

    # candidate vs previous default (the keep/promote decision)
    prev_decision = prev_reason = None
    candidate_improves_previous = False
    if prev:
        bt_prev = cb.both_solved_conflict_totals(cand, prev)
        prev_decision, prev_reason = cb.lexicographic_decision(cand_score, prev_score, bt_prev)
        candidate_improves_previous = prev_decision == "win"

    decision_required = "none"
    if candidate_loses_solver10:
        decision_required = ("candidate_improves_previous_solver11_but_loses_solver10"
                             if candidate_improves_previous else "candidate_loses_solver10")
        failures.append(decision_required)

    def fmt(s):
        return None if s is None else {"solved": s["solved"], "conflicts_solved": s["conflicts_solved"],
                                       "par2": round(s["par2"], 3), "cells": s["cells"]}
    print(f"mode=multiseed timeout_s={t:g} memory_mb={args.memory_mb}")
    print("machine_environment=" + json.dumps(machine_environment(t, args.memory_mb), sort_keys=True))
    print("running_solver_processes=" + json.dumps(process_lines))
    print("solver10_score=" + json.dumps(fmt(s10_score)))
    if prev_score is not None:
        print("previous_solver11_score=" + json.dumps(fmt(prev_score)))
    print("candidate_score=" + json.dumps(fmt(cand_score)))
    print(f"candidate_vs_solver10={floor_decision}  # {floor_reason}")
    if prev_decision is not None:
        print(f"candidate_vs_previous_solver11={prev_decision}  # {prev_reason}")
    print("contradictions_vs_solver10=" + json.dumps(contra_s10))
    print("contradictions_vs_previous=" + json.dumps(contra_prev))
    print("candidate_correctness_failures=" + json.dumps(bad))
    print("decision_required=" + decision_required)
    print("failures=" + json.dumps(failures))
    if failures:
        print("promotion_gate=FAIL")
        return 1
    print("promotion_gate=PASS")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--multiseed", action="store_true",
                        help="inputs are feature_ablation per-(config,instance,seed) TSVs; "
                             "decide lexicographically solved->conflicts->PAR-2")
    parser.add_argument("--floor", "--solver10", dest="solver10", type=Path)
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
        parser.error("--floor (or legacy --solver10) and --candidate are required unless --self-test is used")
    return check_gate_multiseed(args) if args.multiseed else check_gate(args)


if __name__ == "__main__":
    sys.exit(main())
