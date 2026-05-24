#!/usr/bin/env python3
"""PAR-2-first paired comparison for bench.sh results.csv files."""

from __future__ import annotations

import argparse
import csv
import json
import os
import platform
import statistics
import subprocess
import sys
import tempfile
from collections import defaultdict
from collections import Counter
from pathlib import Path


SOLVED = {"SAT", "UNSAT"}
FAIL_RESULTS = {"ERROR", "PARSE_ERROR"}


def strip_cnf_suffix(name: str) -> str:
    for suffix in (".cnf.xz", ".cnf.gz", ".cnf"):
        if name.endswith(suffix):
            return name[: -len(suffix)]
    return name


def read_rows(path: Path) -> dict[str, dict[str, str]]:
    with path.open(newline="") as handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames is None:
            raise SystemExit(f"{path}: missing CSV header")
        required = {"instance", "result", "time_s"}
        missing = required.difference(reader.fieldnames)
        if missing:
            raise SystemExit(f"{path}: missing columns: {', '.join(sorted(missing))}")
        rows: dict[str, dict[str, str]] = {}
        for row in reader:
            name = (row.get("instance") or "").strip()
            if not name:
                raise SystemExit(f"{path}: empty instance name")
            if name in rows:
                raise SystemExit(f"{path}: duplicate instance {name!r}")
            rows[name] = {key: (value or "").strip() for key, value in row.items()}
    if not rows:
        raise SystemExit(f"{path}: no rows")
    return rows


def read_baseline(path: Path | None) -> dict[str, dict[str, str]]:
    if path is None:
        return {}
    with path.open(newline="") as handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames is None:
            raise SystemExit(f"{path}: missing CSV header")
        required = {"instance", "expected_status", "category", "selection_version"}
        missing = required.difference(reader.fieldnames)
        if missing:
            raise SystemExit(f"{path}: missing columns: {', '.join(sorted(missing))}")
        aliases: dict[str, dict[str, str]] = {}
        for row in reader:
            normalized = {key: (value or "").strip() for key, value in row.items()}
            instance = normalized["instance"]
            candidates = {
                instance,
                Path(instance).name,
                strip_cnf_suffix(Path(instance).name),
                strip_cnf_suffix(instance),
            }
            for candidate in candidates:
                aliases.setdefault(candidate, normalized)
        return aliases


def baseline_row_for(baseline: dict[str, dict[str, str]], instance: str) -> dict[str, str]:
    direct = baseline.get(instance) or baseline.get(strip_cnf_suffix(instance))
    if direct is not None:
        return direct
    # Baseline rows often use logical names while bench.sh rows keep SAT Comp hash prefixes.
    for key in sorted(baseline, key=len, reverse=True):
        if key and key in instance:
            return baseline[key]
    return {}


def read_validation(results_csv: Path, rows: dict[str, dict[str, str]]) -> tuple[dict[str, dict], list[str]]:
    validation_path = results_csv.parent / "validation.jsonl"
    warnings: list[str] = []
    if not validation_path.exists():
        warnings.append(f"{validation_path}: validation summary missing")
        return {}, warnings
    if validation_path.stat().st_mtime < results_csv.stat().st_mtime:
        warnings.append(f"{validation_path}: validation summary is older than results.csv")

    records: dict[str, dict] = {}
    with validation_path.open() as handle:
        for line_no, raw in enumerate(handle, start=1):
            line = raw.strip()
            if not line:
                continue
            try:
                payload = json.loads(line)
            except json.JSONDecodeError as exc:
                warnings.append(f"{validation_path}:{line_no}: invalid JSON: {exc}")
                continue
            instance = str(payload.get("instance", "")).strip()
            if not instance:
                warnings.append(f"{validation_path}:{line_no}: missing instance")
                continue
            for alias in {instance, Path(instance).name, strip_cnf_suffix(Path(instance).name)}:
                records[alias] = payload

    missing = [name for name in rows if name not in records and strip_cnf_suffix(name) not in records]
    if missing:
        warnings.append(f"{validation_path}: incomplete validation rows for {len(missing)} instance(s)")
    return records, warnings


def as_float(value: str, default: float = 0.0) -> float:
    try:
        return float(value)
    except ValueError:
        return default


def timeout_for(row: dict[str, str], default: float) -> float:
    return as_float(row.get("timeout", ""), default)


def par2(rows: dict[str, dict[str, str]], timeout: float) -> float:
    total = 0.0
    for row in rows.values():
        if row["result"] in SOLVED:
            total += as_float(row["time_s"])
        else:
            total += 2.0 * timeout_for(row, timeout)
    return total


def solved_count(rows: dict[str, dict[str, str]]) -> int:
    return sum(1 for row in rows.values() if row["result"] in SOLVED)


def validation_for(records: dict[str, dict], instance: str) -> dict | None:
    return records.get(instance) or records.get(strip_cnf_suffix(instance))


def correctness_failures(
    rows: dict[str, dict[str, str]],
    baseline: dict[str, dict[str, str]],
    validation: dict[str, dict],
) -> list[str]:
    failures: list[str] = []
    for name, row in sorted(rows.items()):
        base = baseline_row_for(baseline, name)
        expected = base.get("expected_status", "")
        if expected and expected != "UNKNOWN" and row["result"] in SOLVED and row["result"] != expected:
            failures.append(f"{name}: result {row['result']} differs from expected {expected}")
        if row["result"] in FAIL_RESULTS:
            failures.append(f"{name}: harness/solver correctness failure result={row['result']}")
        record = validation_for(validation, name)
        if record:
            error = str(record.get("validation_error", "") or "").strip()
            if error:
                failures.append(f"{name}: validation_error={error}")
            if row["result"] == "SAT" and record.get("model_check_result") != "pass":
                failures.append(f"{name}: SAT without model_check_result=pass")
            if row["result"] == "UNSAT":
                proof_result = str(record.get("proof_check_result", "") or "").strip()
                if proof_result in {"missing", "unchecked", "checker-timeout", "checker-failed", "fail"}:
                    failures.append(f"{name}: UNSAT proof failure proof_check_result={proof_result}")
    return failures


def is_status_regression(before_result: str, after_result: str) -> bool:
    if before_result == after_result:
        return False
    if after_result in FAIL_RESULTS:
        return True
    if before_result in SOLVED and after_result not in SOLVED:
        return True
    if before_result in SOLVED and after_result in SOLVED:
        return True
    return False


def category_for(baseline: dict[str, dict[str, str]], instance: str) -> str:
    return baseline_row_for(baseline, instance).get("category", "uncategorized")


def result_counts(rows: dict[str, dict[str, str]]) -> dict[str, int]:
    return dict(sorted(Counter(row["result"] for row in rows.values()).items()))


def parse_raw_counts(raw_path: Path) -> tuple[dict[str, int] | None, dict[str, int] | None, dict[str, str]]:
    if not raw_path.exists():
        return None, None, {}
    metadata: dict[str, str] = {}
    before_counts = after_counts = None
    for raw in raw_path.read_text().splitlines():
        if "=" not in raw:
            continue
        key, value = raw.split("=", 1)
        metadata[key] = value
        if key in {"before_counts", "after_counts"}:
            try:
                parsed = json.loads(value.replace("'", '"'))
            except json.JSONDecodeError:
                continue
            if key == "before_counts":
                before_counts = {str(k): int(v) for k, v in parsed.items()}
            else:
                after_counts = {str(k): int(v) for k, v in parsed.items()}
    return before_counts, after_counts, metadata


def baseline_lock_raw_path(args: argparse.Namespace) -> Path | None:
    before_s = args.before.as_posix()
    after_s = args.after.as_posix()
    if "log/baseline-lock/solver10/results.csv" in before_s and "log/baseline-lock/solver11/results.csv" in after_s:
        return Path("solver/11-kissat-port/BASELINE_LOCK.raw.txt")
    return None


def per_category_par2(
    rows: dict[str, dict[str, str]], baseline: dict[str, dict[str, str]], timeout: float
) -> dict[str, float]:
    grouped: dict[str, dict[str, dict[str, str]]] = defaultdict(dict)
    for name, row in rows.items():
        grouped[category_for(baseline, name)][name] = row
    return {category: par2(group_rows, timeout) for category, group_rows in sorted(grouped.items())}


def machine_block(timeout: float) -> dict[str, str]:
    cpu_model = "unknown"
    cpu_count = str(os_cpu_count())
    try:
        for raw in Path("/proc/cpuinfo").read_text(errors="ignore").splitlines():
            if raw.startswith("model name"):
                cpu_model = raw.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    governor = "unknown"
    gov_path = Path("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
    if gov_path.exists():
        governor = gov_path.read_text().strip()
    rustc = "unknown"
    try:
        rustc = subprocess.run(
            ["rustc", "--version"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        ).stdout.strip() or "unknown"
    except OSError:
        pass
    return {
        "cpu_model": cpu_model,
        "core_count": cpu_count,
        "governor": governor,
        "os": platform.platform(),
        "rustc": rustc,
        "binary_sha256": "not_available_from_results_csv",
        "command_line": " ".join(sys.argv),
        "timeout": f"{timeout:g}",
        "memory_limit": "not_available_from_results_csv",
        "SAT_SEED": os.environ.get("SAT_SEED", "unset"),
        "config_hash": "not_available_without_JSON_STATS",
    }


def os_cpu_count() -> int:
    return os.cpu_count() or 0


def compare(args: argparse.Namespace) -> int:
    before = read_rows(args.before)
    after = read_rows(args.after)
    baseline = read_baseline(args.baseline)
    before_validation, before_warnings = read_validation(args.before, before)
    after_validation, after_warnings = read_validation(args.after, after)
    validation_warnings = before_warnings + [w for w in after_warnings if w not in set(before_warnings)]

    common = sorted(set(before) & set(after))
    if not common:
        raise SystemExit("no common instances to compare")

    missing_after = sorted(set(before) - set(after))
    extra_after = sorted(set(after) - set(before))
    status_changes = [
        (name, before[name]["result"], after[name]["result"])
        for name in common
        if before[name]["result"] != after[name]["result"]
    ]
    status_regressions = [
        change for change in status_changes if is_status_regression(change[1], change[2])
    ]
    correctness = correctness_failures(after, baseline, after_validation)
    raw_path = baseline_lock_raw_path(args)
    raw_before_counts = raw_after_counts = None
    raw_metadata: dict[str, str] = {}
    if raw_path is not None:
        raw_before_counts, raw_after_counts, raw_metadata = parse_raw_counts(raw_path)
        if raw_before_counts is not None and raw_before_counts != result_counts(before):
            correctness.append(
                f"{raw_path}: before_counts {raw_before_counts} differ from rich counts {result_counts(before)}"
            )
        if raw_after_counts is not None and raw_after_counts != result_counts(after):
            correctness.append(
                f"{raw_path}: after_counts {raw_after_counts} differ from rich counts {result_counts(after)}"
            )

    deltas = []
    for name in common:
        before_time = as_float(before[name]["time_s"])
        after_time = as_float(after[name]["time_s"])
        deltas.append((name, after_time - before_time, before_time, after_time))

    timeout = args.timeout
    par2_before = par2(before, timeout)
    par2_after = par2(after, timeout)
    total_delta = par2_after - par2_before
    speedups = [before_time / after_time for _, _, before_time, after_time in deltas if after_time > 0]

    newly_solved = sorted(
        name for name in common if before[name]["result"] not in SOLVED and after[name]["result"] in SOLVED
    )
    new_timeouts = sorted(
        name for name in common if before[name]["result"] != "TIMEOUT" and after[name]["result"] == "TIMEOUT"
    )

    print(f"before={args.before}")
    print(f"after={args.after}")
    print(f"baseline={args.baseline or 'none'}")
    print(f"timeout_s={timeout:g}")
    print("correctness_failures=" + json.dumps(correctness))
    print("status_changes=" + json.dumps(status_changes))
    print("status_regressions=" + json.dumps(status_regressions))
    print("validation_warnings=" + json.dumps(validation_warnings))
    print("raw_status_counts_match=" + json.dumps(not any("before_counts" in item or "after_counts" in item for item in correctness)))
    if raw_metadata:
        print("raw_lock_metadata=" + json.dumps(raw_metadata, sort_keys=True))
    print(f"PAR2_before={par2_before:.3f}")
    print(f"PAR2_after={par2_after:.3f}")
    print(f"PAR2_delta={total_delta:.3f}")
    print(f"solved_before={solved_count(before)}")
    print(f"solved_after={solved_count(after)}")
    print("newly_solved=" + json.dumps(newly_solved))
    print("new_timeouts=" + json.dumps(new_timeouts))
    print("missing_after=" + json.dumps(missing_after))
    print("extra_after=" + json.dumps(extra_after))
    print("per_category_PAR2_before=" + json.dumps(per_category_par2(before, baseline, timeout), sort_keys=True))
    print("per_category_PAR2_after=" + json.dumps(per_category_par2(after, baseline, timeout), sort_keys=True))
    if speedups:
        print(f"paired_speedup_median={statistics.median(speedups):.4f}")
    else:
        print("paired_speedup_median=NA")
    print("bootstrap_ci_PAR2_delta=not_computed_single_run")
    print("seed_vs_binary_variance=not_available")
    print("counter_deltas=not_available_without_JSON_STATS_join")
    print("proof_throughput_deltas=not_available_without_proof_validation_join")
    print("machine_environment=" + json.dumps(machine_block(timeout), sort_keys=True))
    print("top_10_wins=" + json.dumps([(n, round(d, 3)) for n, d, _, _ in sorted(deltas, key=lambda x: x[1])[:10]]))
    print(
        "top_10_regressions="
        + json.dumps([(n, round(d, 3)) for n, d, _, _ in sorted(deltas, key=lambda x: x[1], reverse=True)[:10]])
    )
    print("instances_requiring_manual_review=" + json.dumps(sorted(set(new_timeouts + missing_after + extra_after))))

    print("instance,before_result,after_result,before_time_s,after_time_s,delta_s,category")
    for name, delta, before_time, after_time in deltas:
        print(
            f"{name},{before[name]['result']},{after[name]['result']},"
            f"{before_time:.3f},{after_time:.3f},{delta:.3f},{category_for(baseline, name)}"
        )

    if correctness or missing_after or extra_after:
        print("promotion_verdict=significant_regression")
        print("verdict=FAIL")
        return 1
    if total_delta < -0.01 * max(par2_before, 1.0):
        print("promotion_verdict=significant_improvement")
    elif total_delta > 0.01 * max(par2_before, 1.0):
        print("promotion_verdict=significant_regression")
        print("verdict=FAIL")
        return 1
    else:
        print("promotion_verdict=indeterminate")
    print("verdict=PASS")
    return 0


def self_test() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        before_dir = root / "before"
        after_dir = root / "after"
        before_dir.mkdir()
        after_dir.mkdir()
        before_csv = before_dir / "results.csv"
        after_csv = after_dir / "results.csv"
        before_csv.write_text(
            "instance,result,verified,time_s,timeout,exit_code\n"
            "a,SAT,ok,2.000,10,0\n"
            "b,TIMEOUT,skip,10.000,10,124\n"
            "c,SAT,ok,9.000,10,0\n"
        )
        after_csv.write_text(
            "instance,result,verified,time_s,timeout,exit_code\n"
            "a,SAT,ok,1.000,10,0\n"
            "b,UNSAT,ok,0.500,10,0\n"
            "c,TIMEOUT,skip,10.000,10,124\n"
        )
        baseline = root / "baseline.csv"
        baseline.write_text(
            "instance,expected_status,category,selection_version\n"
            "a,SAT,smoke-plus,test\n"
            "b,UNSAT,stress,test\n"
            "c,SAT,stress,test\n"
        )
        namespace = argparse.Namespace(
            before=before_csv,
            after=after_csv,
            baseline=baseline,
            timeout=10.0,
        )
        code = compare(namespace)
        if code != 0:
            raise AssertionError("self-test expected PASS")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--before", type=Path)
    parser.add_argument("--after", type=Path)
    parser.add_argument("--timeout", type=float, default=1800.0)
    parser.add_argument("--baseline", type=Path, default=None)
    args = parser.parse_args()
    if args.self_test:
        self_test()
        print("SELFTEST ok")
        return 0
    if args.before is None or args.after is None:
        parser.error("--before and --after are required unless --self-test is used")
    return compare(args)


if __name__ == "__main__":
    sys.exit(main())
