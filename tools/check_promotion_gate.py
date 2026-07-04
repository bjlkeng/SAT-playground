#!/usr/bin/env python3
"""Guard a default/fast promotion with correctness + an A/B against the pre-change baseline.

There is no external reference floor. The gate makes exactly two kinds of comparison:

  1. Candidate correctness — invalid SAT model, invalid UNSAT proof, ERROR/PARSE_ERROR
     cells, and SAT<->UNSAT contradictions against the baseline (one side is wrong; the
     metric can never buy back a wrong answer).
  2. Candidate vs the ORIGINAL BASELINE before the change (before/after A/B) on the
     lexicographic metric solved -> conflicts(both-solved) -> PAR-2. A lexicographic
     regression against the baseline fails the gate.

Both the candidate and the baseline must be measured on the same instance/seed set. A
candidate that ties or improves the baseline (and is correctness-clean) passes.
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


def status_contradictions_against_baseline(
    baseline: dict[str, dict[str, str]], candidate: dict[str, dict[str, str]]
) -> list[tuple[str, str, str]]:
    """SAT<->UNSAT disagreements vs the baseline — a correctness signal the metric cannot price.

    One of the two runs is wrong on these instances, so the candidate must not ship regardless
    of the aggregate metric. (A baseline-solved instance the candidate merely fails to solve in
    time is NOT a contradiction; that is an honest solved->unsolved regression priced into the
    lexicographic comparison.)
    """
    contradictions = []
    for name in sorted(set(baseline) & set(candidate)):
        before = baseline[name]["result"]
        after = candidate[name]["result"]
        if before in compare_bench.SOLVED and after in compare_bench.SOLVED and before != after:
            contradictions.append((name, before, after))
    return contradictions


def check_gate(args: argparse.Namespace, process_lines: list[str] | None = None) -> int:
    candidate = read_results(args.candidate)
    baseline = read_results(args.baseline) if args.baseline is not None else None

    failures: list[str] = []
    warnings: list[str] = []

    if args.baseline is None:
        failures.append("baseline_required")
    if args.memory_mb is None:
        failures.append("memory_mb_required")

    if baseline is not None:
        missing_baseline, extra_baseline = same_instance_set(baseline, candidate)
        if missing_baseline or extra_baseline:
            failures.append("candidate_instance_set_differs_from_baseline")
    else:
        missing_baseline = []
        extra_baseline = []

    timeout_sets = {"candidate": row_timeout_values(candidate)}
    if baseline is not None:
        timeout_sets["baseline"] = row_timeout_values(baseline)
    if len({tuple(values) for values in timeout_sets.values()}) > 1:
        failures.append("timeout_columns_do_not_match")

    candidate_validation, validation_warnings = compare_bench.read_validation(args.candidate, candidate)
    warnings.extend(validation_warnings)
    failures.extend(compare_bench.correctness_failures(candidate, {}, candidate_validation))

    contradictions = (
        status_contradictions_against_baseline(baseline, candidate) if baseline is not None else []
    )
    if contradictions:
        failures.append("candidate_contradicts_baseline")

    process_lines = running_solver_processes() if process_lines is None else process_lines
    if process_lines and not args.allow_running_solvers:
        failures.append("running_solver_processes_detected")

    candidate_par2 = par2(candidate, args.timeout)
    candidate_solved = solved_count(candidate)
    baseline_par2 = par2(baseline, args.timeout) if baseline is not None else None
    baseline_solved = solved_count(baseline) if baseline is not None else None

    # Candidate vs baseline (before/after A/B): solved-count first, PAR-2 as the tie-break.
    # A regression fails the gate; a tie or improvement passes.
    decision = "none"
    reason = ""
    if baseline is not None:
        margin = max(0.0, baseline_par2 * args.tolerance_fraction)
        if candidate_solved != baseline_solved:
            decision = "win" if candidate_solved > baseline_solved else "regress"
            reason = f"solved {candidate_solved} vs {baseline_solved}"
        elif candidate_par2 > baseline_par2 + margin:
            decision = "regress"
            reason = f"equal solved; PAR-2 {candidate_par2:.1f} vs {baseline_par2:.1f}"
        elif candidate_par2 < baseline_par2 - margin:
            decision = "win"
            reason = f"equal solved; PAR-2 {candidate_par2:.1f} vs {baseline_par2:.1f}"
        else:
            decision = "tie"
            reason = f"equal solved; PAR-2 {candidate_par2:.1f} vs {baseline_par2:.1f} (within tolerance)"
        if decision == "regress":
            failures.append("candidate_regresses_baseline")

    print(f"candidate={args.candidate}")
    print(f"baseline={args.baseline or 'none'}")
    print(f"timeout_s={args.timeout:g}")
    print(f"memory_mb={args.memory_mb if args.memory_mb is not None else 'not_provided'}")
    print("machine_environment=" + json.dumps(machine_environment(args.timeout, args.memory_mb), sort_keys=True))
    print("running_solver_processes=" + json.dumps(process_lines))
    print("timeout_columns=" + json.dumps(timeout_sets, sort_keys=True))
    if baseline_par2 is not None:
        print(f"baseline_PAR2={baseline_par2:.3f}")
    print(f"candidate_PAR2={candidate_par2:.3f}")
    if baseline_par2 is not None:
        print(f"candidate_minus_baseline_PAR2={candidate_par2 - baseline_par2:.3f}")
    if baseline_solved is not None:
        print(f"baseline_solved={baseline_solved}")
    print(f"candidate_solved={candidate_solved}")
    if baseline is not None:
        print("baseline_counts=" + json.dumps(result_counts(baseline), sort_keys=True))
    print("candidate_counts=" + json.dumps(result_counts(candidate), sort_keys=True))
    print("missing_from_candidate_vs_baseline=" + json.dumps(missing_baseline))
    print("extra_in_candidate_vs_baseline=" + json.dumps(extra_baseline))
    print(f"candidate_vs_baseline={decision}" + (f"  # {reason}" if reason else ""))
    print("contradictions_vs_baseline=" + json.dumps(contradictions))
    print("validation_warnings=" + json.dumps(warnings))
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
        baseline = root / "baseline.csv"
        candidate_wins = root / "candidate-wins.csv"
        candidate_regresses = root / "candidate-regresses.csv"
        candidate_ties = root / "candidate-ties.csv"
        candidate_contra = root / "candidate-contradicts.csv"
        write_csv(baseline, [("a", "SAT", 8.0), ("b", "UNSAT", 8.0)])
        write_csv(candidate_wins, [("a", "SAT", 2.0), ("b", "UNSAT", 3.0)])
        write_csv(candidate_regresses, [("a", "TIMEOUT", 10.0), ("b", "UNSAT", 8.0)])
        write_csv(candidate_ties, [("a", "SAT", 8.0), ("b", "UNSAT", 8.0)])
        write_csv(candidate_contra, [("a", "UNSAT", 1.0), ("b", "UNSAT", 1.0)])

        base = argparse.Namespace(
            baseline=baseline,
            timeout=10.0,
            memory_mb=16384,
            tolerance_fraction=0.0,
            allow_running_solvers=False,
        )
        wins = argparse.Namespace(**{**vars(base), "candidate": candidate_wins})
        if check_gate(wins, process_lines=[]) != 0:
            raise AssertionError("candidate that beats the baseline must pass")
        reg = argparse.Namespace(**{**vars(base), "candidate": candidate_regresses})
        if check_gate(reg, process_lines=[]) == 0:
            raise AssertionError("candidate that solves fewer than the baseline must fail")
        tie = argparse.Namespace(**{**vars(base), "candidate": candidate_ties})
        if check_gate(tie, process_lines=[]) != 0:
            raise AssertionError("candidate that ties the baseline must pass")
        contra = argparse.Namespace(**{**vars(base), "candidate": candidate_contra})
        if check_gate(contra, process_lines=[]) == 0:
            raise AssertionError("SAT<->UNSAT contradiction vs the baseline must fail")

        _self_test_multiseed(root)


def write_seed_tsv(path: Path, config: str, rows: list[tuple[str, str, str, float, int]]) -> None:
    """rows: (instance, seed, result, time_s, conflicts)."""
    lines = ["config\tinstance\tseed\tresult\ttime_s\tconflicts\tpropagations\tdecisions\ttimeout"]
    for inst, seed, res, t, cf in rows:
        lines.append(f"{config}\t{inst}\t{seed}\t{res}\t{t:.3f}\t{cf}\t0\t0\t600")
    path.write_text("\n".join(lines) + "\n")


def _self_test_multiseed(root: Path) -> None:
    def mk(name, cfg, spec):
        p = root / name
        write_seed_tsv(p, cfg, spec)
        return p
    seeds = ["0", "1"]
    baseline = mk("ms_baseline.tsv", "baseline",
                  [("a", s, "SAT", 6, 120) for s in seeds] + [("b", s, "UNSAT", 6, 120) for s in seeds])
    base = dict(timeout=600.0, memory_mb=16384, tolerance_fraction=0.0,
                allow_running_solvers=False, multiseed=True, baseline=baseline)

    # WIN: candidate solves the same with fewer conflicts than the baseline.
    cand_win = mk("ms_cand_win.tsv", "cand",
                  [("a", s, "SAT", 6, 90) for s in seeds] + [("b", s, "UNSAT", 6, 90) for s in seeds])
    if check_gate_multiseed(argparse.Namespace(**{**base, "candidate": cand_win}), process_lines=[]) != 0:
        raise AssertionError("multiseed: fewer-conflicts candidate (beats baseline) must PASS")

    # WIN: candidate solves MORE than the baseline.
    cand_more = mk("ms_cand_more.tsv", "cand",
                   [("a", s, "SAT", 6, 120) for s in seeds] + [("b", s, "UNSAT", 6, 120) for s in seeds]
                   + [("c", s, "SAT", 6, 120) for s in seeds])
    baseline_more = mk("ms_baseline_more.tsv", "baseline",
                       [("a", s, "SAT", 6, 120) for s in seeds] + [("b", s, "UNSAT", 6, 120) for s in seeds]
                       + [("c", s, "TIMEOUT", 600, 0) for s in seeds])
    if check_gate_multiseed(argparse.Namespace(**{**base, "baseline": baseline_more, "candidate": cand_more}),
                            process_lines=[]) != 0:
        raise AssertionError("multiseed: candidate solving more than the baseline must PASS")

    # FAIL: candidate solves FEWER than the baseline (lexicographic regression).
    cand_lose = mk("ms_cand_lose.tsv", "cand",
                   [("a", s, "SAT", 6, 90) for s in seeds] + [("b", s, "TIMEOUT", 600, 0) for s in seeds])
    if check_gate_multiseed(argparse.Namespace(**{**base, "candidate": cand_lose}), process_lines=[]) == 0:
        raise AssertionError("multiseed: candidate solving fewer than the baseline must FAIL")

    # FAIL: same solved, MORE conflicts than the baseline (faster PAR-2 cannot save it).
    cand_moreconf = mk("ms_cand_moreconf.tsv", "cand",
                       [("a", s, "SAT", 1, 500) for s in seeds] + [("b", s, "UNSAT", 1, 500) for s in seeds])
    if check_gate_multiseed(argparse.Namespace(**{**base, "candidate": cand_moreconf}), process_lines=[]) == 0:
        raise AssertionError("multiseed: equal-solved + more-conflicts-than-baseline must FAIL despite faster PAR-2")

    # PASS: identical to the baseline (a tie is not a regression).
    cand_tie = mk("ms_cand_tie.tsv", "cand",
                  [("a", s, "SAT", 6, 120) for s in seeds] + [("b", s, "UNSAT", 6, 120) for s in seeds])
    if check_gate_multiseed(argparse.Namespace(**{**base, "candidate": cand_tie}), process_lines=[]) != 0:
        raise AssertionError("multiseed: candidate that ties the baseline must PASS")

    # FAIL: SAT<->UNSAT contradiction vs the baseline.
    cand_contra = mk("ms_cand_contra.tsv", "cand",
                     [("a", s, "UNSAT", 5, 90) for s in seeds] + [("b", s, "UNSAT", 5, 90) for s in seeds])
    if check_gate_multiseed(argparse.Namespace(**{**base, "candidate": cand_contra}), process_lines=[]) == 0:
        raise AssertionError("multiseed: SAT<->UNSAT contradiction vs the baseline must FAIL")


def check_gate_multiseed(args: argparse.Namespace, process_lines: list[str] | None = None) -> int:
    """Multi-seed lexicographic before/after gate.

    Inputs are feature_ablation per-(config,instance,seed) TSVs for the candidate and the
    pre-change baseline. The decision metric is LEXICOGRAPHIC solved -> conflicts(both-solved)
    -> PAR-2, evaluated as candidate vs baseline (the before/after A/B). Structural guards:
    matching (instance,seed) cell sets, candidate correctness, SAT<->UNSAT contradictions vs the
    baseline, process sanity, and required --baseline / --memory-mb.
    """
    cb = compare_bench
    cand_cells = cb.seed_cells_by_config(cb.read_seed_tsv(args.candidate))
    base_cells = cb.seed_cells_by_config(cb.read_seed_tsv(args.baseline)) if args.baseline else None

    failures: list[str] = []
    warnings: list[str] = []

    # each TSV should contain exactly one config; collapse to its cells
    def only(cells_by_cfg, label):
        if len(cells_by_cfg) != 1:
            failures.append(f"{label}_tsv_must_contain_exactly_one_config")
            merged = {}
            for d in cells_by_cfg.values():
                merged.update(d)
            return merged
        return next(iter(cells_by_cfg.values()))

    if args.baseline is None:
        failures.append("baseline_required")
    if args.memory_mb is None:
        failures.append("memory_mb_required")

    cand = only(cand_cells, "candidate")
    base = only(base_cells, "baseline") if base_cells is not None else {}

    # matching (instance,seed) cell sets
    if base_cells is not None and set(cand) != set(base):
        failures.append("candidate_cells_differ_from_baseline")

    # correctness: any ERROR/PARSE_ERROR cell in the candidate
    bad = sorted(f"{k[0]}@seed{k[1]}={c['result']}" for k, c in cand.items()
                 if c["result"].upper() in {"ERROR", "PARSE_ERROR"})
    if bad:
        failures.append("candidate_correctness_failures")

    # SAT<->UNSAT contradictions per (instance,seed) vs the baseline
    contra = cb.seed_contradictions(cand, base) if base else []
    if contra:
        failures.append("candidate_contradicts_baseline")

    process_lines = running_solver_processes() if process_lines is None else process_lines
    if process_lines and not args.allow_running_solvers:
        failures.append("running_solver_processes_detected")

    t = args.timeout
    cand_score = cb.lexicographic_score(cand, t)
    base_score = cb.lexicographic_score(base, t) if base else None

    # candidate vs baseline (the before/after keep/promote decision)
    decision = reason = None
    if base:
        bt = cb.both_solved_conflict_totals(cand, base)
        decision, reason = cb.lexicographic_decision(cand_score, base_score, bt)
        if decision == "regress":
            failures.append("candidate_regresses_baseline")

    def fmt(s):
        return None if s is None else {"solved": s["solved"], "conflicts_solved": s["conflicts_solved"],
                                       "par2": round(s["par2"], 3), "cells": s["cells"]}
    print(f"mode=multiseed timeout_s={t:g} memory_mb={args.memory_mb}")
    print("machine_environment=" + json.dumps(machine_environment(t, args.memory_mb), sort_keys=True))
    print("running_solver_processes=" + json.dumps(process_lines))
    if base_score is not None:
        print("baseline_score=" + json.dumps(fmt(base_score)))
    print("candidate_score=" + json.dumps(fmt(cand_score)))
    if decision is not None:
        print(f"candidate_vs_baseline={decision}  # {reason}")
    print("contradictions_vs_baseline=" + json.dumps(contra))
    print("candidate_correctness_failures=" + json.dumps(bad))
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
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--baseline", "--previous", dest="baseline", type=Path,
                        help="the pre-change baseline (before/after A/B reference)")
    parser.add_argument("--timeout", type=float, default=300.0)
    parser.add_argument("--memory-mb", type=int, default=None)
    parser.add_argument("--tolerance-fraction", type=float, default=0.01)
    parser.add_argument("--allow-running-solvers", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        print("SELFTEST ok")
        return 0

    if args.candidate is None or args.baseline is None:
        parser.error("--candidate and --baseline are required unless --self-test is used")
    return check_gate_multiseed(args) if args.multiseed else check_gate(args)


if __name__ == "__main__":
    sys.exit(main())
