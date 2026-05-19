#!/usr/bin/env python3
"""Create and validate solver 11 benchmark selection sets."""

from __future__ import annotations

import argparse
import csv
import gzip
import hashlib
import lzma
import os
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_MANIFEST = REPO_ROOT / "benchmarks" / "discriminating" / "MANIFEST.csv"
ITERATION_ROOT = REPO_ROOT / "benchmarks" / "iteration"
SELECTION_VERSION = "solver11-iteration-v1"

MANIFEST_FIELDS = {
    "selection_version",
    "logical_name",
    "path",
    "sha256",
    "compressed_sha256",
    "size_bytes",
    "expected_status",
    "expected_status_source",
    "family",
    "root_cause_tag",
    "kissat_reference_external_time",
    "kissat_reference_local_time",
    "solver10_reference_time",
    "notes",
}

BASELINE_FIELDS = [
    "instance",
    "expected_status",
    "proof_policy",
    "solver10_time",
    "kissat_time",
    "minisat_time",
    "solver10_conflicts",
    "solver10_decisions",
    "solver10_propagations",
    "solver10_preprocess_time",
    "solver10_search_time",
    "category",
    "reason_for_selection",
    "residual_vars_after_preprocess",
    "residual_clauses_after_preprocess",
    "residual_lits_after_preprocess",
    "proof_required",
    "model_required",
    "category_weight",
    "holdout_bucket",
    "benchmark_family",
    "selection_version",
    "selection_confidence",
    "reference_solver_versions",
]

SMOKE_TESTS = [
    ("smoke_sat_unit", "tests/cnf/sat/unit.cnf", "SAT", "single unit clause model output"),
    ("smoke_sat_two_clause", "tests/cnf/sat/two_clause.cnf", "SAT", "small SAT assignment"),
    ("smoke_sat_three_sat", "tests/cnf/sat/three_sat.cnf", "SAT", "small 3-SAT model output"),
    ("smoke_sat_all_positive", "tests/cnf/sat/all_positive.cnf", "SAT", "all-positive model output"),
    ("smoke_unsat_contradiction", "tests/cnf/unsat/contradiction.cnf", "UNSAT", "unit contradiction proof path"),
    ("smoke_unsat_empty_clause", "tests/cnf/unsat/empty_clause.cnf", "UNSAT", "empty clause handling"),
    ("smoke_unsat_pigeonhole", "tests/cnf/unsat/pigeonhole_3_2.cnf", "UNSAT", "small proof-producing pigeonhole"),
    ("smoke_unsat_chain", "tests/cnf/unsat/chain_unsat.cnf", "UNSAT", "implication-chain conflict"),
    ("smoke_unsat_xor_2var", "tests/cnf/unsat/xor_2var.cnf", "UNSAT", "binary-clause parity contradiction"),
]

KILLER_TESTS = [
    ("uip-off-by-one", "tests/cnf/unsat/chain_unsat.cnf", "UNSAT", "UIP cut and first-UIP boundary mistakes"),
    ("learned-clause-rewatcher-staleness", "tests/cnf/sat/three_sat.cnf", "SAT", "watcher movement after learned-clause insertion"),
    ("bve-model-reconstruction-failure", "benchmarks/discriminating/5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7.cnf.xz", "SAT", "model replay after eliminated-variable reconstruction"),
    ("vivification-wrong-strengthening", "benchmarks/discriminating/46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized.cnf.xz", "UNSAT", "clause strengthening soundness under K4-style pressure"),
    ("drat-deletion-misorder", "tests/cnf/unsat/pigeonhole_3_2.cnf", "UNSAT", "DRAT addition/deletion order on proof-heavy small UNSAT"),
    ("binary-clause-gc-reference-drift", "tests/cnf/unsat/xor_2var.cnf", "UNSAT", "binary implication references across GC/reduction"),
    ("chrono-backtrack-reason-corruption", "tests/cnf/unsat/contradiction.cnf", "UNSAT", "reason integrity after non-chronological backtrack"),
    ("gate-extraction-false-positive", "benchmarks/discriminating/849950561ddce887c78fef773dccfa80-circuit_48in64out_with_800gates_4in4out_dist128_seed3.sanitized.cnf.xz", "SAT", "gate-aware simplification must not invent equivalences"),
    ("extension-stack-out-of-order-replay", "benchmarks/discriminating/0205e2dffaef93a90c239df31755f2e1-bp4_CSO_AM_IXA_LP.normalised.cnf.xz", "UNSAT", "extension/replay stack ordering under preprocessing"),
    ("proof-buffering-write-error-mid-finalize", "tests/cnf/unsat/empty_clause.cnf", "UNSAT", "proof finalization and write-error cleanup path"),
]


@dataclass(frozen=True)
class ManifestRow:
    raw: dict[str, str]

    @property
    def logical_name(self) -> str:
        return self.raw["logical_name"]

    @property
    def path(self) -> Path:
        return (REPO_ROOT / self.raw["path"]).resolve()

    @property
    def repo_path(self) -> str:
        return self.raw["path"]

    @property
    def expected_status(self) -> str:
        return self.raw["expected_status"]

    @property
    def root_cause_tag(self) -> str:
        return self.raw["root_cause_tag"]

    @property
    def family(self) -> str:
        return self.raw["family"]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_logical_cnf(path: Path) -> str:
    digest = hashlib.sha256()
    suffixes = path.suffixes
    if suffixes and suffixes[-1] == ".xz":
        opener = lzma.open
    elif suffixes and suffixes[-1] == ".gz":
        opener = gzip.open
    else:
        opener = Path.open
    with opener(path, "rb") as handle:  # type: ignore[arg-type]
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_manifest(path: Path) -> list[ManifestRow]:
    with path.open(newline="") as handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames is None:
            raise ValueError(f"{path}: missing CSV header")
        missing = sorted(MANIFEST_FIELDS.difference(reader.fieldnames))
        if missing:
            raise ValueError(f"{path}: missing manifest fields: {', '.join(missing)}")
        rows = [ManifestRow({k: (v or "").strip() for k, v in row.items()}) for row in reader]
    if len(rows) < 12:
        raise ValueError(f"{path}: expected at least 12 discriminating rows, found {len(rows)}")
    return rows


def cnf_files_under(path: Path) -> set[str]:
    return {
        child.absolute().relative_to(REPO_ROOT).as_posix()
        for child in path.iterdir()
        if child.name.endswith((".cnf", ".cnf.gz", ".cnf.xz"))
    }


def check_manifest(path: Path) -> list[ManifestRow]:
    rows = read_manifest(path)
    seen: set[str] = set()
    versions = {row.raw["selection_version"] for row in rows}
    if len(versions) != 1:
        raise ValueError(f"{path}: expected one selection_version, found {sorted(versions)}")

    manifest_paths: set[str] = set()
    for row in rows:
        if row.repo_path in manifest_paths:
            raise ValueError(f"{path}: duplicate path {row.repo_path}")
        manifest_paths.add(row.repo_path)
        if row.expected_status not in {"SAT", "UNSAT", "UNKNOWN"}:
            raise ValueError(f"{row.repo_path}: invalid expected_status {row.expected_status!r}")
        if row.logical_name in seen:
            raise ValueError(f"{path}: duplicate logical_name {row.logical_name!r}")
        seen.add(row.logical_name)
        if not row.path.is_file():
            raise FileNotFoundError(f"{row.repo_path}: target does not exist")
        raw_stat = row.path.stat()
        if str(raw_stat.st_size) != row.raw["size_bytes"]:
            raise ValueError(f"{row.repo_path}: size_bytes mismatch")
        if sha256_file(row.path) != row.raw["compressed_sha256"]:
            raise ValueError(f"{row.repo_path}: compressed_sha256 mismatch")
        if sha256_logical_cnf(row.path) != row.raw["sha256"]:
            raise ValueError(f"{row.repo_path}: decompressed sha256 mismatch")

    actual = cnf_files_under(path.parent)
    missing_rows = sorted(actual.difference(manifest_paths))
    missing_files = sorted(manifest_paths.difference(actual))
    if missing_rows or missing_files:
        raise ValueError(
            f"{path}: symlink/manifest mismatch missing_rows={missing_rows} missing_files={missing_files}"
        )
    print(
        f"manifest_ok selection_version={next(iter(versions))} rows={len(rows)} "
        f"dir={path.parent.absolute().relative_to(REPO_ROOT)}"
    )
    return rows


def rel_symlink(target: Path, link_path: Path) -> None:
    if link_path.exists() or link_path.is_symlink():
        link_path.unlink()
    link_path.symlink_to(os.path.relpath(target, link_path.parent))


def clear_known_outputs() -> None:
    for dirname in [
        "smoke-plus",
        "search-core",
        "preprocess-core",
        "regression-guards",
        "stress",
        "holdout",
        "killer-tests",
    ]:
        directory = ITERATION_ROOT / dirname
        directory.mkdir(parents=True, exist_ok=True)
        for child in directory.iterdir():
            if child.is_file() or child.is_symlink():
                child.unlink()


def by_logical(rows: Iterable[ManifestRow]) -> dict[str, ManifestRow]:
    return {row.logical_name: row for row in rows}


def materialize_set(
    category: str,
    items: Iterable[tuple[str, Path, str, str, str, str, str]],
    baseline_rows: list[dict[str, str]],
) -> None:
    directory = ITERATION_ROOT / category
    directory.mkdir(parents=True, exist_ok=True)
    for logical, target, status, family, reason, confidence, solver10_time in items:
        if target.name.endswith(".cnf.xz"):
            suffix = ".cnf.xz"
        elif target.name.endswith(".cnf.gz"):
            suffix = ".cnf.gz"
        else:
            suffix = ".cnf"
        link_name = logical if logical.endswith(suffix) else f"{logical}{suffix}"
        link_name = link_name.replace("/", "_")
        link_path = directory / link_name
        rel_symlink(target, link_path)
        baseline_rows.append(
            {
                "instance": link_path.relative_to(REPO_ROOT).as_posix(),
                "expected_status": status,
                "proof_policy": "drat" if status == "UNSAT" else "off",
                "solver10_time": solver10_time,
                "kissat_time": "NA",
                "minisat_time": "NA",
                "solver10_conflicts": "NA",
                "solver10_decisions": "NA",
                "solver10_propagations": "NA",
                "solver10_preprocess_time": "NA",
                "solver10_search_time": "NA",
                "category": category,
                "reason_for_selection": reason,
                "residual_vars_after_preprocess": "NA",
                "residual_clauses_after_preprocess": "NA",
                "residual_lits_after_preprocess": "NA",
                "proof_required": "true" if status == "UNSAT" else "false",
                "model_required": "true" if status == "SAT" else "false",
                "category_weight": "1.0",
                "holdout_bucket": "holdout" if category == "holdout" else "tuning",
                "benchmark_family": family,
                "selection_version": SELECTION_VERSION,
                "selection_confidence": confidence,
                "reference_solver_versions": "kissat_external_table;solver10_plan_table;local_reference_pending",
            }
        )


def manifest_item(row: ManifestRow, reason: str, confidence: str = "high") -> tuple[str, Path, str, str, str, str, str]:
    return (
        row.logical_name,
        row.path,
        row.expected_status,
        row.family,
        reason,
        confidence,
        row.raw["solver10_reference_time"],
    )


def smoke_item(name: str, path: str, status: str, reason: str) -> tuple[str, Path, str, str, str, str, str]:
    return (name, REPO_ROOT / path, status, "smoke", reason, "high", "NA")


def write_iteration_sets(rows: list[ManifestRow]) -> None:
    lookup = by_logical(rows)
    clear_known_outputs()
    baseline_rows: list[dict[str, str]] = []

    materialize_set(
        "smoke-plus",
        [smoke_item(*item) for item in SMOKE_TESTS],
        baseline_rows,
    )
    materialize_set(
        "search-core",
        [
            manifest_item(lookup["battleship-16-31-sat"], "phase and decision-quality gap"),
            manifest_item(lookup["mp1-Nb7T46"], "learned-clause quality gap"),
            manifest_item(lookup["544707209399nw.shuffled-as.sat03-1671"], "phase and restart sensitivity"),
            manifest_item(lookup["SC25_Timetable_C_392"], "Timetable search/restart representative"),
            manifest_item(lookup["SC25_Timetable_C_406"], "hard Timetable phase-saving representative"),
            manifest_item(lookup["DLTM_twitter845_79_19"], "phase-saving search trajectory"),
            manifest_item(lookup["83aa254f-1.normalised"], "search throughput gap"),
            manifest_item(lookup["case9"], "search trajectory gap"),
            manifest_item(lookup["1-TC-256-K-63"], "timeout-level search trajectory gap"),
        ],
        baseline_rows,
    )
    materialize_set(
        "preprocess-core",
        [
            manifest_item(lookup["REGRandom-K4-L1-Seed40"], "K4 preprocessing plus LBD retention pressure"),
            manifest_item(lookup["circuit_48in64out_with_800gates"], "gate-aware BVE and phase gap"),
            manifest_item(lookup["Kakuro-easy-112-ext.xml.hg_7"], "preprocessing throughput representative"),
            manifest_item(lookup["bp4_CSO_IXA_ZR.normalised"], "bp4 preprocessing timeout representative"),
            manifest_item(lookup["bp4_CSO_AM_IXA_LP.normalised"], "bp4 UNSAT preprocessing timeout representative"),
            manifest_item(lookup["brocard_problem_large"], "preprocessing residual pressure"),
        ],
        baseline_rows,
    )
    materialize_set(
        "regression-guards",
        [
            smoke_item(*SMOKE_TESTS[0]),
            smoke_item(*SMOKE_TESTS[4]),
            manifest_item(lookup["mp1-Nb7T46"], "guard learned-clause quality while changing scheduling"),
            manifest_item(lookup["Kakuro-easy-112-ext.xml.hg_7"], "guard MiniSat-style simplification parity work"),
            manifest_item(lookup["div-mitern172"], "guard UNSAT clause DB behavior"),
        ],
        baseline_rows,
    )
    materialize_set(
        "stress",
        [
            manifest_item(lookup["REGRandom-K4-L1-Seed40"], "solver10 timeout stress"),
            manifest_item(lookup["circuit_48in64out_with_800gates"], "solver10 timeout stress"),
            manifest_item(lookup["bp4_CSO_IXA_ZR.normalised"], "preprocessing timeout stress"),
            manifest_item(lookup["bp4_CSO_AM_IXA_LP.normalised"], "proof-heavy UNSAT stress"),
            manifest_item(lookup["1-TC-256-K-63"], "search timeout stress"),
            manifest_item(lookup["SCPC-500-1"], "proof-heavy clause DB stress"),
            manifest_item(lookup["aaai10-planning-pathways-step20"], "UNSAT planning clause DB stress"),
            manifest_item(lookup["sqrt-mitern171"], "miter UNSAT clause DB stress"),
        ],
        baseline_rows,
    )
    materialize_set(
        "holdout",
        [
            manifest_item(lookup["SC25_Timetable_C_406"], "holdout Timetable search check", "medium"),
            manifest_item(lookup["DLTM_twitter845_79_19"], "holdout phase-saving check", "medium"),
            manifest_item(lookup["sqrt-mitern171"], "holdout miter UNSAT check", "medium"),
            manifest_item(lookup["bp4_CSO_AM_IXA_LP.normalised"], "holdout preprocessing UNSAT check", "medium"),
            manifest_item(lookup["brocard_problem_large"], "holdout preprocessing residual check", "medium"),
        ],
        baseline_rows,
    )
    materialize_set(
        "killer-tests",
        [
            (name, REPO_ROOT / path, status, "killer", reason, "medium", "NA")
            for name, path, status, reason in KILLER_TESTS
        ],
        baseline_rows,
    )

    ITERATION_ROOT.mkdir(parents=True, exist_ok=True)
    with (ITERATION_ROOT / "baseline.csv").open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=BASELINE_FIELDS, lineterminator="\n")
        writer.writeheader()
        writer.writerows(baseline_rows)
    with (ITERATION_ROOT / "FLAKY.csv").open("w", newline="") as handle:
        writer = csv.DictWriter(
            handle,
            fieldnames=[
                "instance",
                "category",
                "reason",
                "first_seen_utc",
                "quarantine_status",
                "promotion_policy",
            ],
            lineterminator="\n",
        )
        writer.writeheader()
    print(f"wrote_iteration_sets root={ITERATION_ROOT.relative_to(REPO_ROOT)} rows={len(baseline_rows)}")


def check_iteration_sets() -> None:
    required_dirs = [
        "smoke-plus",
        "search-core",
        "preprocess-core",
        "regression-guards",
        "stress",
        "holdout",
        "killer-tests",
    ]
    counts: dict[str, int] = {}
    for dirname in required_dirs:
        directory = ITERATION_ROOT / dirname
        files = sorted(directory.glob("*.cnf*")) if directory.is_dir() else []
        if not files:
            raise ValueError(f"{directory.relative_to(REPO_ROOT)}: generated benchmark set is empty")
        counts[dirname] = len(files)

    baseline = ITERATION_ROOT / "baseline.csv"
    if not baseline.exists():
        raise FileNotFoundError(f"{baseline.relative_to(REPO_ROOT)} missing")
    with baseline.open(newline="") as handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames is None:
            raise ValueError(f"{baseline}: missing header")
        missing = sorted(set(BASELINE_FIELDS).difference(reader.fieldnames))
        if missing:
            raise ValueError(f"{baseline}: missing fields {missing}")
        rows = list(reader)
    categories = defaultdict(int)
    for row in rows:
        categories[row["category"]] += 1
    for dirname in required_dirs:
        if categories[dirname] == 0:
            raise ValueError(f"{baseline}: no rows for category {dirname}")
    flaky = ITERATION_ROOT / "FLAKY.csv"
    if not flaky.exists():
        raise FileNotFoundError(f"{flaky.relative_to(REPO_ROOT)} missing")
    print(
        "iteration_sets_ok "
        + " ".join(f"{name}={count}" for name, count in sorted(counts.items()))
        + f" baseline_rows={len(rows)}"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-manifest", type=Path, default=None)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()

    try:
        if args.check_manifest is not None:
            check_manifest(args.check_manifest)
            return 0
        rows = check_manifest(args.manifest)
        if args.write:
            write_iteration_sets(rows)
        if args.dry_run or not args.write:
            check_iteration_sets()
        return 0
    except Exception as exc:
        print(f"select_iter_bench FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
