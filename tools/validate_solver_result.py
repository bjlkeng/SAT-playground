#!/usr/bin/env python3
"""Validate one captured SAT solver run directory against a CNF input."""

from __future__ import annotations

import argparse
import gzip
import json
import lzma
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Iterable


STATUS_FROM_S_LINE = {
    "s SATISFIABLE": "SAT",
    "s UNSATISFIABLE": "UNSAT",
    "s UNKNOWN": "UNKNOWN",
}
RESULT_JSON = "result.json"
FINAL_PROOF = "proof.out"
TEMP_PROOF = "proof.out.tmp"
REQUIRED_RESULT_JSON = {
    "schema_version",
    "status",
    "exit_code",
    "termination_reason",
    "unknown_reason",
    "status_file",
    "model_file",
    "proof_file",
    "proof_completeness",
    "model_check_result",
    "proof_check_result",
    "config_hash",
    "input_sha256",
    "profile",
    "proof_policy",
    "output_contract_state",
    "stats_json_seen",
}


def open_text(path: Path):
    suffixes = path.suffixes
    if suffixes and suffixes[-1] == ".gz":
        return gzip.open(path, "rt")
    if suffixes and suffixes[-1] == ".xz":
        return lzma.open(path, "rt")
    return path.open("rt")


def parse_cnf(path: Path) -> list[list[int]]:
    clauses: list[list[int]] = []
    current: list[int] = []
    with open_text(path) as handle:
        for raw in handle:
            line = raw.strip()
            if not line or line.startswith("c") or line.startswith("p"):
                continue
            for tok in line.split():
                lit = int(tok)
                if lit == 0:
                    clauses.append(current)
                    current = []
                else:
                    current.append(lit)
    if current:
        clauses.append(current)
    return clauses


def parse_stdout(stdout_path: Path) -> tuple[str, list[int], list[str]]:
    s_lines: list[str] = []
    assignment: list[int] = []
    v_lines: list[str] = []
    with stdout_path.open() as handle:
        for raw in handle:
            line = raw.strip()
            if line.startswith("s "):
                s_lines.append(line)
            elif line.startswith("v "):
                v_lines.append(line)
                for tok in line.split()[1:]:
                    lit = int(tok)
                    if lit != 0:
                        assignment.append(lit)
    if len(s_lines) != 1:
        raise ValueError(f"expected exactly one s-line in {stdout_path}, found {len(s_lines)}")
    if s_lines[0] not in STATUS_FROM_S_LINE:
        raise ValueError(f"unsupported status line in {stdout_path}: {s_lines[0]!r}")
    return STATUS_FROM_S_LINE[s_lines[0]], assignment, v_lines


def read_result_json(out_dir: Path) -> tuple[str, Path, dict]:
    result_path = out_dir / RESULT_JSON
    if not result_path.exists():
        raise FileNotFoundError(f"{result_path}: mandatory solver result contract is missing")
    payload = json.loads(result_path.read_text())
    missing = sorted(REQUIRED_RESULT_JSON.difference(payload))
    if missing:
        raise ValueError(f"{result_path}: missing required fields {missing}")
    status = payload.get("status")
    if not isinstance(status, str):
        raise ValueError(f"{result_path}: missing string status")
    status_file = payload.get("status_file")
    if not isinstance(status_file, str):
        raise ValueError(f"{result_path}: missing string status_file")
    status_path = Path(status_file)
    if not status_path.exists():
        raise FileNotFoundError(f"{result_path}: status_file does not exist: {status_path}")
    status_text = status_path.read_text().strip()
    if normalize_status(status_text) != normalize_status(status):
        raise ValueError(
            f"{result_path}: status_file says {status_text!r}, result.json says {status!r}"
        )
    return normalize_status(status), result_path, payload


def normalize_status(status: str) -> str:
    status = status.strip()
    if status in STATUS_FROM_S_LINE:
        return STATUS_FROM_S_LINE[status]
    upper = status.upper()
    aliases = {
        "SATISFIABLE": "SAT",
        "UNSATISFIABLE": "UNSAT",
        "PARSEERROR": "PARSE_ERROR",
        "PARSE_ERROR": "PARSE_ERROR",
    }
    return aliases.get(upper, upper)


def verify_assignment(clauses: Iterable[list[int]], assignment: list[int]) -> None:
    if not assignment:
        raise ValueError("SAT result has no assignment literals")
    seen: dict[int, int] = {}
    true_lits = set()
    for lit in assignment:
        var = abs(lit)
        prior = seen.get(var)
        if prior is not None and prior != lit:
            raise ValueError(f"contradictory assignment for variable {var}: {prior} and {lit}")
        seen[var] = lit
        true_lits.add(lit)
    for idx, clause in enumerate(clauses, start=1):
        if not any(lit in true_lits for lit in clause):
            body = " ".join(str(lit) for lit in clause)
            raise ValueError(f"clause {idx} not satisfied: ({body})")


def find_drat_trim() -> str | None:
    repo_tool = Path(__file__).resolve().parent / "checkers" / "drat-trim" / "drat-trim"
    if repo_tool.exists() and repo_tool.is_file():
        return str(repo_tool)
    return shutil.which("drat-trim")


def maybe_decompress_cnf(cnf: Path, temp_dir: Path) -> Path:
    if cnf.suffix == ".gz":
        target = temp_dir / cnf.with_suffix("").name
        with gzip.open(cnf, "rb") as src, target.open("wb") as dst:
            shutil.copyfileobj(src, dst)
        return target
    if cnf.suffix == ".xz":
        target = temp_dir / cnf.with_suffix("").name
        with lzma.open(cnf, "rb") as src, target.open("wb") as dst:
            shutil.copyfileobj(src, dst)
        return target
    return cnf


def verify_drat(cnf: Path, proof: Path) -> None:
    checker = find_drat_trim()
    if checker is None:
        raise ValueError("proof-policy=drat requires drat-trim, but none was found")
    with tempfile.TemporaryDirectory() as tmp:
        cnf_for_checker = maybe_decompress_cnf(cnf, Path(tmp))
        proc = subprocess.run(
            [checker, str(cnf_for_checker), str(proof)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
        )
    normalized = [line.strip().replace("\r", "") for line in proc.stdout.splitlines()]
    if not any(line in {"s VERIFIED", "s ACCEPTED"} for line in normalized):
        tail = "\n".join(normalized[-10:])
        raise ValueError(f"drat-trim rejected proof {proof}:\n{tail}")


def check_json_stats(out_dir: Path, payload: dict) -> None:
    if not payload.get("stats_json_seen"):
        raise ValueError(f"{out_dir / RESULT_JSON}: stats_json_seen is false")


def validate(args: argparse.Namespace) -> tuple[str, dict]:
    cnf = args.cnf
    out_dir = args.out_dir
    if not cnf.exists():
        raise FileNotFoundError(f"CNF not found: {cnf}")
    if not out_dir.is_dir():
        raise FileNotFoundError(f"output directory not found: {out_dir}")

    status, status_source, result_payload = read_result_json(out_dir)
    expected = normalize_status(args.expected_status)
    if expected != "ANY" and status != expected:
        raise ValueError(f"expected {expected}, got {status} from {status_source}")

    stdout_path = out_dir / "stdout.log"
    assignment: list[int] = []
    v_lines: list[str] = []
    if stdout_path.exists():
        stdout_status, assignment, v_lines = parse_stdout(stdout_path)
        expected_stdout_status = "UNKNOWN" if status == "PARSE_ERROR" else status
        if stdout_status != expected_stdout_status:
            raise ValueError(f"status source says {status}, stdout says {stdout_status}")

    proof_path = out_dir / FINAL_PROOF
    temp_proof_path = out_dir / TEMP_PROOF
    has_model = bool(v_lines or assignment)

    if status == "SAT":
        if result_payload.get("model_check_result") != "pass":
            raise ValueError(
                f"{status_source}: SAT result requires model_check_result=pass, got "
                f"{result_payload.get('model_check_result')!r}"
            )
        if not stdout_path.exists():
            raise FileNotFoundError("SAT validation requires stdout.log with v-lines")
        model_file = result_payload.get("model_file")
        if not isinstance(model_file, str) or not Path(model_file).exists():
            raise FileNotFoundError(f"SAT result missing model_file from {status_source}")
        verify_assignment(parse_cnf(cnf), assignment)
    elif status == "UNSAT":
        if has_model:
            raise ValueError("UNSAT result must not contain v-lines")
        if args.proof_policy == "drat":
            if result_payload.get("proof_completeness") != "complete":
                raise ValueError(
                    f"{status_source}: UNSAT with proof-policy=drat requires "
                    "proof_completeness=complete"
                )
            proof_file = result_payload.get("proof_file")
            proof_for_check = Path(proof_file) if isinstance(proof_file, str) else proof_path
            if not proof_for_check.exists():
                raise FileNotFoundError(f"UNSAT result missing {proof_for_check}")
            verify_drat(cnf, proof_for_check)
    elif status in {"UNKNOWN", "PARSE_ERROR"}:
        if result_payload.get("proof_completeness") == "complete":
            raise ValueError(f"{status} result must not claim proof_completeness=complete")
        if proof_path.exists() or temp_proof_path.exists():
            raise ValueError(f"{status} result must not leave finalized or temp proof files")
        if has_model:
            raise ValueError(f"{status} result must not contain v-lines")
    else:
        raise ValueError(f"unsupported normalized status: {status}")

    if args.require_json_stats == "on":
        check_json_stats(out_dir, result_payload)

    print(f"VALIDATED status={status} source={status_source} out_dir={out_dir}")
    return status, result_payload


def append_validation_summary(
    path: Path,
    args: argparse.Namespace,
    status: str,
    payload: dict | None,
    error: str,
) -> None:
    record = {
        "instance": str(args.cnf) if args.cnf is not None else "",
        "out_dir": str(args.out_dir) if args.out_dir is not None else "",
        "status": status,
        "expected_status": normalize_status(args.expected_status),
        "model_check_result": "",
        "proof_check_result": "",
        "proof_checker": "",
        "proof_checker_exit_status": "",
        "output_contract_state": "",
        "validation_error": error,
    }
    if payload is not None:
        record.update(
            {
                "model_check_result": payload.get("model_check_result", ""),
                "proof_check_result": payload.get("proof_check_result", ""),
                "proof_checker": payload.get("proof_checker", ""),
                "proof_checker_exit_status": payload.get("proof_checker_exit_status", ""),
                "output_contract_state": payload.get("output_contract_state", ""),
            }
        )
    with path.open("a") as handle:
        handle.write(json.dumps(record, sort_keys=True) + "\n")


def self_test() -> None:
    clauses = [[1, -2], [2]]
    verify_assignment(clauses, [1, 2])
    try:
        verify_assignment(clauses, [-1, 2])
    except ValueError:
        pass
    else:
        raise AssertionError("expected unsatisfied assignment to fail")
    assert normalize_status("s SATISFIABLE") == "SAT"
    assert normalize_status("parse_error") == "PARSE_ERROR"
    print("SELFTEST ok")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--cnf", type=Path)
    parser.add_argument("--out-dir", type=Path)
    parser.add_argument(
        "--expected-status",
        default="any",
        choices=["SAT", "UNSAT", "UNKNOWN", "PARSE_ERROR", "any"],
    )
    parser.add_argument("--proof-policy", choices=["off", "drat"], default="off")
    parser.add_argument("--require-json-stats", choices=["on", "off"], default="off")
    parser.add_argument("--validation-jsonl", type=Path)
    args = parser.parse_args()

    try:
        if args.self_test:
            self_test()
            return 0
        if args.cnf is None or args.out_dir is None:
            parser.error("--cnf and --out-dir are required unless --self-test is used")
        status, payload = validate(args)
        if args.validation_jsonl is not None:
            append_validation_summary(args.validation_jsonl, args, status, payload, "")
        return 0
    except Exception as exc:
        if "args" in locals() and getattr(args, "validation_jsonl", None) is not None:
            try:
                append_validation_summary(args.validation_jsonl, args, "VALIDATION_ERROR", None, str(exc))
            except Exception:
                pass
        print(f"VALIDATION FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
