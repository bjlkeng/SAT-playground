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


def fail(errors: list[str], message: str) -> None:
    errors.append(message)


def validate_state_file(solver_dir: Path, errors: list[str]) -> None:
    state_path = solver_dir / "SOLVER11_STATE.md"
    if not state_path.exists():
        fail(errors, f"missing {state_path}")
        return

    text = state_path.read_text()
    for required in [
        "Stage A Modules",
        "Stage B Map",
        "Capability-Based Mutation Rule",
        "unrestricted_mut_solver_exceptions",
        "Extraction Rules",
    ]:
        if required not in text:
            fail(errors, f"{state_path}: missing section {required!r}")


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
