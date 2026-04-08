#!/usr/bin/env bash
# bench.sh — Run a solver against benchmark instances with PAR-2 scoring
#
# Usage:
#   bash tools/bench.sh [OPTIONS] <solver_dir>
#
# Options:
#   -t, --timeout <seconds>   Per-instance time limit (default: 5000)
#   -m, --memory  <MB>        Per-instance memory limit in MB (default: 30720 = 30 GB)
#   -d, --benchdir <path>     Benchmark directory (default: benchmarks/cnf)
#   -j, --jobs <N>            Parallel jobs (default: 1, sequential)
#   -h, --help                Show this help
#
# SAT Competition 2025 scoring:
#   PAR-2 = sum of runtimes for solved instances
#         + 2 * timeout for each unsolved instance
#   Lower is better.

set -euo pipefail

# --- ensure cargo is in PATH ---
if [[ -f "$HOME/.cargo/env" ]]; then
    source "$HOME/.cargo/env"
fi

# --- defaults (SAT Competition 2025 Main Track) ---
TIMEOUT=5000
MEMLIMIT_MB=30720
BENCH_DIR=""
JOBS=1
SOLVER_REL=""

usage() {
    sed -n '2,/^$/s/^# \?//p' "$0"
    exit 0
}

# --- parse arguments ---
while [[ $# -gt 0 ]]; do
    case "$1" in
        -t|--timeout) TIMEOUT="$2"; shift 2 ;;
        -m|--memory)  MEMLIMIT_MB="$2"; shift 2 ;;
        -d|--benchdir) BENCH_DIR="$2"; shift 2 ;;
        -j|--jobs)    JOBS="$2"; shift 2 ;;
        -h|--help)    usage ;;
        -*)           echo "Unknown option: $1" >&2; exit 1 ;;
        *)            SOLVER_REL="$1"; shift ;;
    esac
done

if [[ -z "$SOLVER_REL" ]]; then
    echo "ERROR: solver directory required" >&2
    echo "Usage: bash tools/bench.sh [OPTIONS] solver/NN-name" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOLVER_DIR="$(cd "$REPO_ROOT/$SOLVER_REL" 2>/dev/null && pwd)" || {
    echo "ERROR: solver directory not found: $SOLVER_REL" >&2
    exit 1
}
RUN_SH="$SOLVER_DIR/run.sh"

if [[ ! -f "$RUN_SH" ]]; then
    echo "ERROR: $RUN_SH not found" >&2
    exit 1
fi

# Default benchmark dir
if [[ -z "$BENCH_DIR" ]]; then
    BENCH_DIR="$REPO_ROOT/benchmarks/cnf"
fi

if [[ ! -d "$BENCH_DIR" ]]; then
    echo "ERROR: benchmark directory not found: $BENCH_DIR" >&2
    echo "Download benchmarks first — see benchmarks/README.md or CLAUDE.md" >&2
    exit 1
fi

# --- locate timeout command ---
TIMEOUT_CMD=""
if command -v gtimeout &>/dev/null; then
    TIMEOUT_CMD="gtimeout"
elif command -v timeout &>/dev/null; then
    TIMEOUT_CMD="timeout"
else
    echo "ERROR: neither 'timeout' nor 'gtimeout' found" >&2
    echo "Install coreutils: brew install coreutils (macOS)" >&2
    exit 1
fi

# --- set up log directory ---
TIMESTAMP=$(date +%Y-%m-%d-%H-%M-%S)
SOLVER_NAME=$(basename "$SOLVER_DIR")
LOG_DIR="$REPO_ROOT/log/bench-${SOLVER_NAME}-${TIMESTAMP}"
mkdir -p "$LOG_DIR"

# --- memory limit (KB for ulimit -v) ---
MEMLIMIT_KB=$((MEMLIMIT_MB * 1024))

# --- collect benchmark instances (.cnf or .cnf.gz) ---
mapfile -t CNF_FILES < <(find "$BENCH_DIR" \( -name '*.cnf' -o -name '*.cnf.gz' \) -type f | sort)
TOTAL=${#CNF_FILES[@]}

if [[ $TOTAL -eq 0 ]]; then
    echo "ERROR: no .cnf or .cnf.gz files found in $BENCH_DIR" >&2
    exit 1
fi

# --- temp dir for decompressed files and proof output ---
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

# --- build solver ---
echo "=== Benchmark run: $SOLVER_NAME ==="
echo "    Date:      $(date)"
echo "    Timeout:   ${TIMEOUT}s"
echo "    Memory:    ${MEMLIMIT_MB} MB"
echo "    Instances: $TOTAL"
echo "    Log:       $LOG_DIR"
echo ""

echo "Building solver..."
if ! (cd "$SOLVER_DIR" && bash build.sh) > "$LOG_DIR/build.log" 2>&1; then
    echo "ERROR: build.sh failed (see $LOG_DIR/build.log)" >&2
    exit 1
fi
echo "Build OK"
echo ""

# --- CSV header ---
RESULTS_CSV="$LOG_DIR/results.csv"
echo "instance,result,time_s,timeout,exit_code" > "$RESULTS_CSV"

# --- counters ---
SOLVED=0
SAT_COUNT=0
UNSAT_COUNT=0
UNKNOWN_COUNT=0
TIMEOUT_COUNT=0
ERROR_COUNT=0
TOTAL_TIME=0

# --- error log (only errors get logged) ---
ERRORS_LOG="$LOG_DIR/errors.log"
: > "$ERRORS_LOG"

# --- run each instance ---
run_instance() {
    local cnf="$1"
    local idx="$2"
    local name
    name=$(basename "$cnf" .cnf.gz)
    name=$(basename "$name" .cnf)

    local proof_dir="$TEMP_DIR/proof"
    rm -rf "$proof_dir"
    mkdir -p "$proof_dir"

    # Decompress .cnf.gz to temp file if needed
    local solver_input="$cnf"
    if [[ "$cnf" == *.cnf.gz ]]; then
        solver_input="$TEMP_DIR/$name.cnf"
        gzip -dkc "$cnf" > "$solver_input"
    fi

    # Run solver with timeout and memory limit
    local start_time end_time elapsed
    start_time=$(date +%s.%N 2>/dev/null || date +%s)

    local output=""
    local stderr_output=""
    local exit_code=0
    stderr_output=$(mktemp)
    output=$(
        ulimit -v "$MEMLIMIT_KB" 2>/dev/null
        "$TIMEOUT_CMD" "$TIMEOUT" bash "$RUN_SH" "$solver_input" "$proof_dir" 2>"$stderr_output"
    ) || exit_code=$?

    # Clean up decompressed file and proof output
    [[ "$cnf" == *.cnf.gz ]] && rm -f "$solver_input"
    rm -rf "$proof_dir"

    end_time=$(date +%s.%N 2>/dev/null || date +%s)

    # Compute elapsed time
    if command -v bc &>/dev/null; then
        elapsed=$(echo "$end_time - $start_time" | bc)
    else
        elapsed=$(perl -e "printf '%.3f', $end_time - $start_time")
    fi

    # Extract result
    local s_line result
    s_line=$(echo "$output" | grep '^s ' | head -1) || true

    # exit code 124 = timeout (coreutils timeout)
    if [[ $exit_code -eq 124 ]]; then
        result="TIMEOUT"
    elif [[ -z "$s_line" ]]; then
        result="ERROR"
    else
        case "$s_line" in
            "s SATISFIABLE")   result="SAT" ;;
            "s UNSATISFIABLE") result="UNSAT" ;;
            "s UNKNOWN")       result="UNKNOWN" ;;
            *)                 result="ERROR" ;;
        esac
    fi

    # Log errors only
    if [[ "$result" == "ERROR" ]]; then
        {
            echo "--- $name (exit=$exit_code) ---"
            cat "$stderr_output"
            echo ""
        } >> "$ERRORS_LOG"
    fi
    rm -f "$stderr_output"

    # Clamp elapsed to timeout if it somehow ran over
    local clamped_time
    clamped_time=$(perl -e "printf '%.3f', ($elapsed > $TIMEOUT) ? $TIMEOUT : $elapsed")

    # Write CSV row
    echo "$name,$result,$clamped_time,$TIMEOUT,$exit_code" >> "$RESULTS_CSV"

    # Status line
    local status_icon
    case "$result" in
        SAT)     status_icon="SAT    " ;;
        UNSAT)   status_icon="UNSAT  " ;;
        UNKNOWN) status_icon="UNKNOWN" ;;
        TIMEOUT) status_icon="TIMEOUT" ;;
        ERROR)   status_icon="ERROR  " ;;
    esac
    printf "[%3d/%d] %s %8.2fs  %s\n" "$idx" "$TOTAL" "$status_icon" "$clamped_time" "$name"
}

# Run all instances sequentially
for i in "${!CNF_FILES[@]}"; do
    run_instance "${CNF_FILES[$i]}" "$((i + 1))"
done

echo ""

# --- compute PAR-2 score ---
echo "=== Computing PAR-2 score ==="

# Read results CSV and compute
{
    read -r _header  # skip header
    while IFS=',' read -r name result time_s timeout_val exit_code; do
        case "$result" in
            SAT)
                SAT_COUNT=$((SAT_COUNT + 1))
                SOLVED=$((SOLVED + 1))
                TOTAL_TIME=$(perl -e "printf '%.3f', $TOTAL_TIME + $time_s")
                ;;
            UNSAT)
                UNSAT_COUNT=$((UNSAT_COUNT + 1))
                SOLVED=$((SOLVED + 1))
                TOTAL_TIME=$(perl -e "printf '%.3f', $TOTAL_TIME + $time_s")
                ;;
            UNKNOWN)
                UNKNOWN_COUNT=$((UNKNOWN_COUNT + 1))
                TOTAL_TIME=$(perl -e "printf '%.3f', $TOTAL_TIME + 2 * $timeout_val")
                ;;
            TIMEOUT)
                TIMEOUT_COUNT=$((TIMEOUT_COUNT + 1))
                TOTAL_TIME=$(perl -e "printf '%.3f', $TOTAL_TIME + 2 * $timeout_val")
                ;;
            ERROR)
                ERROR_COUNT=$((ERROR_COUNT + 1))
                TOTAL_TIME=$(perl -e "printf '%.3f', $TOTAL_TIME + 2 * $timeout_val")
                ;;
        esac
    done
} < "$RESULTS_CSV"

UNSOLVED=$((TOTAL - SOLVED))
PAR2="$TOTAL_TIME"

# --- summary ---
SUMMARY="$LOG_DIR/summary.log"
{
    echo "=== Benchmark Results: $SOLVER_NAME ==="
    echo "    Date:      $(date)"
    echo "    Timeout:   ${TIMEOUT}s"
    echo "    Memory:    ${MEMLIMIT_MB} MB"
    echo ""
    echo "    Instances: $TOTAL"
    echo "    Solved:    $SOLVED ($SAT_COUNT SAT + $UNSAT_COUNT UNSAT)"
    echo "    Unsolved:  $UNSOLVED ($TIMEOUT_COUNT timeout + $UNKNOWN_COUNT unknown + $ERROR_COUNT error)"
    echo ""
    echo "    PAR-2:     $PAR2"
    echo ""
    echo "    Results:   $RESULTS_CSV"
    if [[ -s "$ERRORS_LOG" ]]; then
        echo "    Errors:    $ERRORS_LOG"
    fi
} | tee "$SUMMARY"

echo ""
echo "Done."
