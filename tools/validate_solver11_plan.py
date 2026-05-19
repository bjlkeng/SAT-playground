#!/usr/bin/env python3
"""Validate solver 11 architecture-boundary guardrails."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


STAGE_A_MODULES = {
    "config.rs",
    "stats.rs",
    "lit.rs",
    "limits.rs",
    "output.rs",
    "check.rs",
}

LEGACY_EXEMPT_MODULES = {
    "main.rs",
    "simp.rs",
}

PUBLIC_MUT_SOLVER_RE = re.compile(
    r"pub(?:\([^)]*\))?\s+fn\s+[A-Za-z_][A-Za-z0-9_]*\s*\([^)]*&\s*mut\s+Solver",
    re.DOTALL,
)

PUBLIC_RETURNS_MUT_SOLVER_RE = re.compile(
    r"pub(?:\([^)]*\))?\s+fn\s+[A-Za-z_][A-Za-z0-9_]*[^{;]*->\s*&\s*mut\s+Solver",
    re.DOTALL,
)

SOURCE_ANCHORS = {
    "Solver::new": ("src/main.rs", "fn new("),
    "Solver::solve_to_output": ("src/main.rs", "fn solve_to_output("),
    "Solver::solve_with_proof": ("src/main.rs", "fn solve_with_proof("),
    "Solver::propagate": ("src/main.rs", "fn propagate("),
    "Solver::analyze_conflict_to_scratch": ("src/main.rs", "fn analyze_conflict_to_scratch("),
    "Solver::reduce_db": ("src/main.rs", "fn reduce_db("),
    "Solver::eliminate": ("src/simp.rs", "fn eliminate("),
    "ProofLog": ("src/main.rs", "struct ProofLog"),
    "Solver": ("src/main.rs", "struct Solver"),
    "Solver::attach_clause": ("src/main.rs", "fn attach_clause("),
    "Solver::simplify_with_proof": ("src/main.rs", "fn simplify_with_proof("),
    "Solver::garbage_collect": ("src/main.rs", "fn garbage_collect("),
    "parse_cnf": ("src/main.rs", "fn parse_cnf("),
    "main": ("src/main.rs", "fn main("),
}


def fail(errors: list[str], message: str) -> None:
    errors.append(message)


def validate_state_file(solver_dir: Path, errors: list[str]) -> None:
    state_path = solver_dir / "SOLVER11_STATE.md"
    if not state_path.exists():
        fail(errors, f"missing {state_path}")
        return

    text = state_path.read_text()
    for required in [
        "Baseline Source Map",
        "Stage A Modules",
        "Stage B Map",
        "Capability-Based Mutation Rule",
        "unrestricted_mut_solver_exceptions",
        "Extraction Rules",
    ]:
        if required not in text:
            fail(errors, f"{state_path}: missing section {required!r}")
    validate_source_anchors(solver_dir, state_path, text, errors)


def validate_source_anchors(
    solver_dir: Path, state_path: Path, state_text: str, errors: list[str]
) -> None:
    for symbol, (expected_file, expected_needle) in SOURCE_ANCHORS.items():
        row_re = re.compile(
            r"\|\s*`" + re.escape(symbol) + r"`\s*\|\s*`([^`]+):([0-9]+)`\s*\|"
        )
        match = row_re.search(state_text)
        if not match:
            fail(errors, f"{state_path}: missing audited source-map row for {symbol}")
            continue
        actual_file, line_text = match.groups()
        if actual_file != expected_file:
            fail(
                errors,
                f"{state_path}: {symbol} points at {actual_file}, expected {expected_file}",
            )
            continue
        line_no = int(line_text)
        source_path = solver_dir / actual_file
        if not source_path.exists():
            fail(errors, f"{state_path}: {symbol} points at missing {source_path}")
            continue
        source_lines = source_path.read_text().splitlines()
        if line_no < 1 or line_no > len(source_lines):
            fail(errors, f"{state_path}: {symbol} line {line_no} outside {source_path}")
            continue
        source_line = source_lines[line_no - 1]
        if expected_needle not in source_line:
            fail(
                errors,
                f"{state_path}: {symbol} line {line_no} does not contain {expected_needle!r}",
            )


def validate_stage_a_modules(src_dir: Path, errors: list[str]) -> None:
    for module in sorted(STAGE_A_MODULES):
        if not (src_dir / module).exists():
            fail(errors, f"missing Stage A module src/{module}")


def validate_public_mut_solver(src_dir: Path, errors: list[str]) -> None:
    for path in sorted(src_dir.glob("*.rs")):
        if path.name in LEGACY_EXEMPT_MODULES or path.name in STAGE_A_MODULES:
            continue
        text = path.read_text()
        for match in PUBLIC_MUT_SOLVER_RE.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            fail(errors, f"{path}: public pass function takes unrestricted &mut Solver at line {line}")
        for match in PUBLIC_RETURNS_MUT_SOLVER_RE.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            fail(errors, f"{path}: public function returns &mut Solver at line {line}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "solver_dir",
        nargs="?",
        type=Path,
        default=Path("solver/11-kissat-port"),
        help="Path to solver/11-kissat-port",
    )
    args = parser.parse_args()

    solver_dir = args.solver_dir
    src_dir = solver_dir / "src"
    errors: list[str] = []

    if not solver_dir.exists():
        fail(errors, f"missing solver directory {solver_dir}")
    if not src_dir.exists():
        fail(errors, f"missing source directory {src_dir}")
    else:
        validate_stage_a_modules(src_dir, errors)
        validate_public_mut_solver(src_dir, errors)
    validate_state_file(solver_dir, errors)

    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 1

    print(f"solver11 plan validation PASS: {solver_dir}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
