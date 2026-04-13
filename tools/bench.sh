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
TIMEOUT=1800
MEMLIMIT_MB=16384
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
    BENCH_DIR="$REPO_ROOT/benchmarks/sat-comp-2025"
fi

if [[ ! -d "$BENCH_DIR" ]]; then
    echo "ERROR: benchmark directory not found: $BENCH_DIR" >&2
    echo "Download benchmarks first — see benchmarks/README.md or CLAUDE.md" >&2
    exit 1
fi

# --- locate verification tools ---
VERIFY_SAT="$REPO_ROOT/tools/verify_sat.py"
DRAT_TRIM=""
if [[ -x "$REPO_ROOT/tools/checkers/drat-trim/drat-trim" ]]; then
    DRAT_TRIM="$REPO_ROOT/tools/checkers/drat-trim/drat-trim"
elif command -v drat-trim &>/dev/null; then
    DRAT_TRIM="drat-trim"
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

# --- collect benchmark instances (.cnf, .cnf.gz, or .cnf.xz) ---
mapfile -t CNF_FILES < <(find -L "$BENCH_DIR" \( -name '*.cnf' -o -name '*.cnf.gz' -o -name '*.cnf.xz' \) -type f | sort)
TOTAL=${#CNF_FILES[@]}

if [[ $TOTAL -eq 0 ]]; then
    echo "ERROR: no .cnf, .cnf.gz, or .cnf.xz files found in $BENCH_DIR" >&2
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
echo "    SAT checker: $VERIFY_SAT"
if [[ -n "$DRAT_TRIM" ]]; then
    echo "    DRAT checker: $DRAT_TRIM"
else
    echo "    DRAT checker: NONE (run tools/setup_checkers.sh)"
fi
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
echo "instance,result,verified,time_s,timeout,exit_code" > "$RESULTS_CSV"

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
    name=$(basename "$cnf" .cnf.xz)
    name=$(basename "$name" .cnf.gz)
    name=$(basename "$name" .cnf)

    local proof_dir="$TEMP_DIR/proof"
    rm -rf "$proof_dir"
    mkdir -p "$proof_dir"

    # Decompress compressed files to temp if needed
    local solver_input="$cnf"
    if [[ "$cnf" == *.cnf.gz ]]; then
        solver_input="$TEMP_DIR/$name.cnf"
        gzip -dkc "$cnf" > "$solver_input"
    elif [[ "$cnf" == *.cnf.xz ]]; then
        solver_input="$TEMP_DIR/$name.cnf"
        xz -dkc "$cnf" > "$solver_input"
    fi

    # Run solver with timeout and memory limit
    local start_time end_time elapsed
    local output=""
    local stderr_output=""
    local exit_code=0
    stderr_output=$(mktemp)

    start_time=$(date +%s.%N 2>/dev/null || date +%s)
    output=$(
        ulimit -v "$MEMLIMIT_KB" 2>/dev/null
        "$TIMEOUT_CMD" "$TIMEOUT" bash "$RUN_SH" "$solver_input" "$proof_dir" 2>"$stderr_output"
    ) || exit_code=$?
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

    # --- Verify correctness ---
    local verified="skip"

    if [[ "$result" == "SAT" ]]; then
        # Verify assignment satisfies the formula
        local stdout_file="$TEMP_DIR/stdout.tmp"
        echo "$output" > "$stdout_file"
        if python3 "$VERIFY_SAT" "$solver_input" "$stdout_file" >/dev/null 2>&1; then
            verified="ok"
        else
            verified="FAIL"
            {
                echo "--- $name: SAT verification failed ---"
                python3 "$VERIFY_SAT" "$solver_input" "$stdout_file" 2>&1 || true
                echo ""
            } >> "$ERRORS_LOG"
        fi
        rm -f "$stdout_file"
    elif [[ "$result" == "UNSAT" ]]; then
        # Verify DRAT proof
        if [[ -f "$proof_dir/proof.out" ]]; then
            if [[ -n "$DRAT_TRIM" ]]; then
                local checker_output=""
                local checker_status=""
                checker_output=$("$DRAT_TRIM" "$solver_input" "$proof_dir/proof.out" 2>&1) || true
                checker_status=$(printf '%s\n' "$checker_output" | tr -d '\r')
                if echo "$checker_status" | grep -qx "s VERIFIED"; then
                    verified="ok"
                elif echo "$checker_status" | grep -qx "s ACCEPTED"; then
                    verified="ok"
                else
                    verified="FAIL"
                    {
                        echo "--- $name: DRAT proof rejected ---"
                        echo "$checker_output" | tail -5
                        echo ""
                    } >> "$ERRORS_LOG"
                fi
            else
                verified="no-checker"
            fi
        else
            verified="no-proof"
            {
                echo "--- $name: UNSAT but no proof.out ---"
                echo ""
            } >> "$ERRORS_LOG"
        fi
    fi

    # Clean up decompressed file and proof output
    [[ "$cnf" == *.cnf.gz || "$cnf" == *.cnf.xz ]] && rm -f "$solver_input"
    rm -rf "$proof_dir"

    # Log errors
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
    echo "$name,$result,$verified,$clamped_time,$TIMEOUT,$exit_code" >> "$RESULTS_CSV"

    # Status line
    local status_icon verified_icon
    case "$result" in
        SAT)     status_icon="SAT    " ;;
        UNSAT)   status_icon="UNSAT  " ;;
        UNKNOWN) status_icon="UNKNOWN" ;;
        TIMEOUT) status_icon="TIMEOUT" ;;
        ERROR)   status_icon="ERROR  " ;;
    esac
    case "$verified" in
        ok)         verified_icon="" ;;
        skip)       verified_icon="" ;;
        FAIL)       verified_icon=" [VERIFY FAIL]" ;;
        no-checker) verified_icon=" [no checker]" ;;
        no-proof)   verified_icon=" [no proof]" ;;
    esac
    printf "[%3d/%d] %s %8.2fs  %s%s\n" "$idx" "$TOTAL" "$status_icon" "$clamped_time" "$name" "$verified_icon"
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
    while IFS=',' read -r name result verified time_s timeout_val exit_code; do
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
