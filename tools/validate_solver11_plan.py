#!/usr/bin/env python3
"""Validate solver 11 architecture-boundary guardrails."""

from __future__ import annotations

import argparse
import csv
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

REQUIRED_CONFIG_ENVS = {
    "SAT_STATS_JSON",
    "SAT_TRACE_FULL",
    "SAT_CHECK_INVARIANTS",
    "SAT_SEED",
    "SAT_PROFILE",
    "SAT_SEARCH_AXIS",
    "SAT_PREPROCESS_AXIS",
    "SAT_PROOF",
    "SAT_CONFIG_DUMP",
    "SAT_CONFIG_OUT",
    "SAT_CONFIG_REPLAY",
    "SAT_CONFIG_REPLAY_ALLOW_OVERRIDES",
    "SAT_STRICT_CONFIG",
    "SAT_RUN_LABEL",
    "SAT_LIMIT_CONFLICTS",
    "SAT_LIMIT_PROPAGATIONS",
    "SAT_LIMIT_TICKS",
    "SAT_LIMIT_WALL_SEC",
    "SAT_LIMIT_RSS_MB",
    "SAT_LIMIT_LEARNED_LITS",
    "SAT_LIMIT_BINARY_CLAUSES",
    "SAT_LIMIT_EXTENSION_BYTES",
    "SAT_LIMIT_PROOF_BYTES",
    "SAT_USE_LBD",
    "SAT_LBD_UPDATE_REASONS",
    "SAT_RESTART",
    "SAT_REDUCE",
    "SAT_PHASE",
    "SAT_SEARCH_MODE",
    "SAT_CHRONO",
    "SAT_BINARY_FAST",
    "SAT_CLAUSE_MIN",
    "SAT_VMTF",
    "SAT_REPHASE",
    "SAT_MINIMIZE_DEPTH_LIMIT",
    "SAT_CHRONO_MAX_DELTA",
    "SAT_MODE_INIT_CONFLICTS",
    "SAT_MODE_INTERVAL_SCALE",
    "SAT_REPHASE_INIT_CONFLICTS",
    "SAT_SIMPLIFICATION",
    "SAT_BVE",
    "SAT_FULL_BSR",
    "SAT_INPROCESS",
    "SAT_VIVIFY",
    "SAT_PROBE",
    "SAT_HBR",
    "SAT_TRANSITIVE",
    "SAT_FORWARD_SUBSUME",
    "SAT_GATE_EXTRACT",
    "SAT_GATE_BVE",
    "SAT_RCHECK",
    "SAT_INPROCESS_INTERVAL_CONFLICTS",
    "SAT_INPROCESS_MAX_ROUNDS",
    "SAT_VIVIFY_TICKS",
    "SAT_VIVIFY_MAX_CLAUSE_LEN",
    "SAT_PROBE_TICKS",
    "SAT_ELIMINATE_TICKS",
    "SAT_TRANSITIVE_MAX_DEPTH",
    "SAT_TRANSITIVE_TICKS_PER_SOURCE",
    "SAT_TRANSITIVE_MAX_REMOVED_PER_ROUND",
    "SAT_RCHECK_TICKS",
}

REQUIRED_FEATURE_FLAGS = {
    "SAT_USE_LBD",
    "SAT_LBD_UPDATE_REASONS",
    "SAT_CHRONO",
    "SAT_BINARY_FAST",
    "SAT_VMTF",
    "SAT_REPHASE",
    "SAT_SIMPLIFICATION",
    "SAT_BVE",
    "SAT_FULL_BSR",
    "SAT_INPROCESS",
    "SAT_VIVIFY",
    "SAT_PROBE",
    "SAT_HBR",
    "SAT_TRANSITIVE",
    "SAT_FORWARD_SUBSUME",
    "SAT_GATE_EXTRACT",
    "SAT_GATE_BVE",
    "SAT_RCHECK",
}

PARKING_LOT_DENYLIST = {"SAT_WALK", "SAT_SWEEP", "SAT_ELS", "SAT_BCE"}
PLAN_ENV_REFERENCE_EXEMPTIONS = {
    # Documentation-only shorthands or explicitly rejected legacy aliases.
    "SAT_CONFIG",
    "SAT_ELIMINATE_INPROCESS",
    "SAT_LIMIT_",
}
OPTIONAL_DAG_TASK_IDS = {
    # These headings intentionally document templates or parked/reserved features
    # and are not executable nodes in the dependency-respecting implementation DAG.
    "0.7",
    "2.7",
    "2.13",
}
TASK_ID_RE = re.compile(r"(?<![A-Za-z0-9])(?:[0-9]+\.[0-9]+[a-z]?|[A-Z]\.[0-9]+[a-z]?)(?![A-Za-z0-9])")
TASK_HEADING_RE = re.compile(r"^##\s+([0-9]+\.[0-9]+[a-z]?)\b", re.MULTILINE)


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


def read_csv_rows(path: Path, errors: list[str]) -> list[dict[str, str]]:
    if not path.exists():
        fail(errors, f"missing {path}")
        return []
    try:
        with path.open(newline="") as handle:
            return list(csv.DictReader(handle))
    except csv.Error as exc:
        fail(errors, f"{path}: invalid CSV: {exc}")
        return []


def validate_config_artifacts(solver_dir: Path, errors: list[str]) -> None:
    schema_path = solver_dir / "CONFIG_SCHEMA.csv"
    features_path = solver_dir / "FEATURES.csv"
    features_md_path = solver_dir / "FEATURES.md"
    config_rs_path = solver_dir / "src" / "config.rs"

    schema_rows = read_csv_rows(schema_path, errors)
    feature_rows = read_csv_rows(features_path, errors)
    if not features_md_path.exists():
        fail(errors, f"missing {features_md_path}")
    if not config_rs_path.exists():
        fail(errors, f"missing {config_rs_path}")
        config_text = ""
    else:
        config_text = config_rs_path.read_text()

    schema_envs = {row.get("env_var", "") for row in schema_rows}
    missing_envs = sorted(REQUIRED_CONFIG_ENVS - schema_envs)
    if missing_envs:
        fail(errors, f"{schema_path}: missing required env rows {missing_envs}")

    feature_flags = {row.get("feature_flag", "") for row in feature_rows}
    missing_features = sorted(REQUIRED_FEATURE_FLAGS - feature_flags)
    if missing_features:
        fail(errors, f"{features_path}: missing required feature rows {missing_features}")

    for denied in sorted(PARKING_LOT_DENYLIST):
        if denied in schema_envs:
            fail(errors, f"{schema_path}: parked flag {denied} must not be an active schema row")

    for required_text in [
        "pub(crate) struct SolverConfig",
        "fn validate_runtime_support",
        "fn replay_override_env",
        "pub(crate) fn config_hash",
        "pub(crate) fn json_stats_line",
    ]:
        if required_text not in config_text:
            fail(errors, f"{config_rs_path}: missing config contract text {required_text!r}")


def task_region(plan_text: str) -> str:
    start = plan_text.find("## 0.0 Fork")
    end = plan_text.find("# 6. Milestones")
    if start == -1:
        return ""
    if end == -1 or end <= start:
        end = len(plan_text)
    return plan_text[start:end]


def validate_plan_dag(plan_path: Path, solver_dir: Path, errors: list[str]) -> None:
    if not plan_path.exists():
        fail(errors, f"missing plan file {plan_path}")
        return

    plan_text = plan_path.read_text()
    region = task_region(plan_text)
    if not region:
        fail(errors, f"{plan_path}: missing task region starting at '## 0.0 Fork'")
        return

    task_ids = TASK_HEADING_RE.findall(region)
    seen: set[str] = set()
    duplicates: set[str] = set()
    for task_id in task_ids:
        if task_id in seen:
            duplicates.add(task_id)
        seen.add(task_id)
    if duplicates:
        fail(errors, f"{plan_path}: duplicate task IDs {sorted(duplicates)}")

    task_id_set = set(task_ids)
    dag_marker = "# 7. Dependency-respecting DAG summary"
    dag_start = plan_text.find(dag_marker)
    if dag_start == -1:
        fail(errors, f"{plan_path}: missing dependency-respecting DAG summary")
        dag_text = ""
    else:
        code_start = plan_text.find("```text", dag_start)
        code_end = plan_text.find("```", code_start + 1)
        dag_text = plan_text[code_start:code_end] if code_start != -1 and code_end != -1 else ""
        if not dag_text:
            fail(errors, f"{plan_path}: DAG summary is missing its text code block")

    dag_ids = set(TASK_ID_RE.findall(dag_text))
    for task_id in sorted(task_id_set - dag_ids - OPTIONAL_DAG_TASK_IDS):
        fail(errors, f"{plan_path}: task {task_id} missing from DAG summary")
    for dag_id in sorted(dag_ids - task_id_set):
        if not dag_id.startswith("A."):
            fail(errors, f"{plan_path}: DAG summary references unknown task {dag_id}")

    dag_positions = {task_id: dag_text.find(task_id) for task_id in dag_ids}

    if "0.5a profiling" in dag_text and "0.0a rich baseline" in dag_text:
        if dag_text.find("0.0a rich baseline") < dag_text.find("0.5a profiling"):
            fail(errors, f"{plan_path}: DAG appears lexically ordered; 0.0a must remain after 0.5a")

    sections = re.split(r"^##\s+", region, flags=re.MULTILINE)
    for raw_section in sections[1:]:
        heading, _, body = raw_section.partition("\n")
        match = re.match(r"([0-9]+\.[0-9]+[a-z]?)\b", heading)
        if not match:
            continue
        task_id = match.group(1)
        dep_match = re.search(
            r"### Dependenc(?:y|ies)\s*\n\n(?P<body>.*?)(?=\n### |\n## |\n# |\n---\n|\Z)",
            body,
            re.DOTALL,
        )
        if not dep_match:
            continue
        dep_text = dep_match.group("body")
        for dep_id in TASK_ID_RE.findall(dep_text):
            if dep_id == task_id or ".." in dep_text[max(0, dep_text.find(dep_id) - 2): dep_text.find(dep_id) + len(dep_id) + 2]:
                continue
            if dep_id not in task_id_set and not dep_id.startswith("A."):
                fail(errors, f"{plan_path}: task {task_id} has unknown dependency {dep_id}")
                continue
            if dep_id in OPTIONAL_DAG_TASK_IDS or task_id in OPTIONAL_DAG_TASK_IDS:
                continue
            dep_position = dag_positions.get(dep_id)
            task_position = dag_positions.get(task_id)
            if dep_position is None or task_position is None:
                continue
            if dep_position > task_position:
                fail(
                    errors,
                    f"{plan_path}: dependency {dep_id} appears after dependent task {task_id} in DAG summary",
                )

    validate_sat_env_cross_references(plan_path, plan_text, solver_dir, errors)


def validate_sat_env_cross_references(
    plan_path: Path, plan_text: str, solver_dir: Path, errors: list[str]
) -> None:
    schema_rows = read_csv_rows(solver_dir / "CONFIG_SCHEMA.csv", errors)
    schema_envs = {row.get("env_var", "") for row in schema_rows}
    readme_path = solver_dir / "README.md"
    readme_text = readme_path.read_text() if readme_path.exists() else ""
    mentioned = set(re.findall(r"\bSAT_[A-Z0-9_]+\b", plan_text + "\n" + readme_text))
    missing = sorted(
        env
        for env in mentioned - schema_envs
        if env not in PARKING_LOT_DENYLIST and env not in PLAN_ENV_REFERENCE_EXEMPTIONS
    )
    if missing:
        fail(errors, f"{plan_path}/{readme_path}: SAT_* variables missing from CONFIG_SCHEMA.csv {missing}")

    if readme_text and "SAT_PROFILE=baseline|default|fast|experimental" not in readme_text:
        fail(errors, f"{readme_path}: README profile example drifted from documented profile set")

    generated_schema = solver_dir / "target" / "generated" / "CONFIG_SCHEMA.csv"
    generated_features = solver_dir / "target" / "generated" / "FEATURES.csv"
    for checked_in, generated in [
        (solver_dir / "CONFIG_SCHEMA.csv", generated_schema),
        (solver_dir / "FEATURES.csv", generated_features),
    ]:
        if generated.exists() and checked_in.read_text() != generated.read_text():
            fail(errors, f"{checked_in}: checked-in CSV differs from {generated}")


def validate_env_boundary(src_dir: Path, errors: list[str]) -> None:
    env_read_re = re.compile(r"\benv::vars?\b|\benv::var_os\b|\bstd::env::vars?\b|\bstd::env::var_os\b")
    for path in sorted(src_dir.glob("*.rs")):
        if path.name == "config.rs":
            continue
        text = path.read_text()
        for match in env_read_re.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            fail(errors, f"{path}: env config read outside src/config.rs at line {line}")


def validate_result_contract(repo_root: Path, solver_dir: Path, errors: list[str]) -> None:
    output_rs = solver_dir / "src" / "output.rs"
    smoke = repo_root / "tools" / "smoke_test.sh"
    bench = repo_root / "tools" / "bench.sh"
    validator = repo_root / "tools" / "validate_solver_result.py"
    readme = solver_dir / "README.md"

    required_by_file = {
        output_rs: [
            "pub(crate) enum SolveStatus",
            "pub(crate) const RESULT_JSON",
            "write_result_contract",
            "\\\"status\\\":",
            "\\\"config_hash\\\":",
        ],
        smoke: ["result.json", "status_file", "model.txt"],
        bench: ["result.json", "status_file", "PARSE_ERROR"],
        validator: ["RESULT_JSON = \"result.json\"", "REQUIRED_RESULT_JSON", "read_result_json"],
        readme: ["Solver 11 Result Contract", "result.json", "PARSE_ERROR"],
    }
    for path, needles in required_by_file.items():
        if not path.exists():
            fail(errors, f"missing {path}")
            continue
        text = path.read_text()
        for needle in needles:
            if needle not in text:
                fail(errors, f"{path}: missing result-contract text {needle!r}")

    validator_text = validator.read_text() if validator.exists() else ""
    for forbidden in ["status.json", "status.txt, or stdout.log"]:
        if forbidden in validator_text:
            fail(errors, f"{validator}: legacy status fallback still present: {forbidden!r}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "path",
        nargs="?",
        type=Path,
        default=Path("solver/11-kissat-port"),
        help="Path to solver/11-kissat-port or plan/solver-11-plan.md",
    )
    args = parser.parse_args()

    plan_path = Path("plan/solver-11-plan.md")
    solver_dir = args.path
    repo_root = Path.cwd()
    if args.path.is_file():
        plan_path = args.path.resolve()
        repo_root = plan_path.parent.parent
        solver_dir = repo_root / "solver" / "11-kissat-port"
    else:
        solver_dir = args.path.resolve()
        if solver_dir.name == "11-kissat-port" and solver_dir.parent.name == "solver":
            repo_root = solver_dir.parent.parent
    src_dir = solver_dir / "src"
    errors: list[str] = []

    if not solver_dir.exists():
        fail(errors, f"missing solver directory {solver_dir}")
    if not src_dir.exists():
        fail(errors, f"missing source directory {src_dir}")
    else:
        validate_stage_a_modules(src_dir, errors)
        validate_public_mut_solver(src_dir, errors)
        validate_env_boundary(src_dir, errors)
        validate_config_artifacts(solver_dir, errors)
        validate_result_contract(repo_root, solver_dir, errors)
        validate_plan_dag(plan_path, solver_dir, errors)
    validate_state_file(solver_dir, errors)

    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 1

    print(f"solver11 plan validation PASS: {solver_dir}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
