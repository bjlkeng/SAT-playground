#!/usr/bin/env bash
# Run pinned reference solvers on the standard solver 11 benchmark suites.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT_ROOT="$REPO_ROOT/log/reference-baselines"
TIMEOUT=1800
MEMLIMIT_MB=16384
declare -a SOLVERS=(kissat-latest minisat)
declare -a SUITES=(
    "calibration:$REPO_ROOT/tests/cnf/sat"
    "profiling:$REPO_ROOT/benchmarks/profiling"
    "discriminating:$REPO_ROOT/benchmarks/discriminating"
    "sat-comp-2025:$REPO_ROOT/benchmarks/sat-comp-2025"
)

usage() {
    cat <<'USAGE'
Usage: bash tools/run_reference_baseline.sh [options] [solver...]

Options:
  -t, --timeout <seconds>    Per-instance timeout (default: 1800)
  -m, --memory <MB>          Memory limit (default: 16384)
  --suite <name:path>        Add a suite to run; may be repeated
  -h, --help                 Show this help

If no solver is given, runs kissat-latest and minisat.
Results are copied to log/reference-baselines/<solver>/<suite>/.
USAGE
}

declare -a ARGS_SOLVERS=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        -t|--timeout) TIMEOUT="$2"; shift 2 ;;
        -m|--memory) MEMLIMIT_MB="$2"; shift 2 ;;
        --suite) SUITES+=("$2"); shift 2 ;;
        -h|--help) usage; exit 0 ;;
        -*) echo "unknown option: $1" >&2; exit 1 ;;
        *) ARGS_SOLVERS+=("$1"); shift ;;
    esac
done
if [[ ${#ARGS_SOLVERS[@]} -gt 0 ]]; then
    SOLVERS=("${ARGS_SOLVERS[@]}")
fi

mkdir -p "$OUT_ROOT"

latest_log_dir() {
    local solver="$1"
    find "$REPO_ROOT/log" -maxdepth 1 -type d -name "bench-${solver}-*" | sort | tail -1
}

for suite in "${SUITES[@]}"; do
    suite_name="${suite%%:*}"
    suite_path="${suite#*:}"
    if [[ ! -d "$suite_path" ]]; then
        echo "skip suite=$suite_name missing_dir=$suite_path" >&2
        continue
    fi
    for solver in "${SOLVERS[@]}"; do
        echo "running reference solver=$solver suite=$suite_name timeout=${TIMEOUT}s memory=${MEMLIMIT_MB}MB"
        before="$(latest_log_dir "$solver" || true)"
        bash "$REPO_ROOT/tools/bench_reference.sh" -t "$TIMEOUT" -m "$MEMLIMIT_MB" -d "$suite_path" "$solver"
        after="$(latest_log_dir "$solver")"
        if [[ -z "$after" || "$after" == "$before" ]]; then
            echo "failed to locate new log dir for $solver on $suite_name" >&2
            exit 1
        fi
        dest="$OUT_ROOT/$solver/$suite_name"
        mkdir -p "$dest"
        cp "$after/results.csv" "$dest/results.csv"
        cp "$after/summary.log" "$dest/summary.log"
        {
            echo "date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
            echo "solver=$solver"
            echo "suite=$suite_name"
            echo "suite_path=$suite_path"
            echo "source_log=$after"
            echo "timeout=$TIMEOUT"
            echo "memory_mb=$MEMLIMIT_MB"
        } > "$dest/run-metadata.txt"
    done
done
