#!/usr/bin/env bash
# bench_reference.sh — Run reference solvers against benchmark instances with PAR-2 scoring
#
# Usage:
#   bash tools/bench_reference.sh [OPTIONS] [solver1 solver2 ...]
#
# If no solvers specified, runs all three: kissat-latest kissat-sc2024 minisat
#
# Options:
#   -t, --timeout <seconds>   Per-instance time limit (default: 1800)
#   -m, --memory  <MB>        Per-instance memory limit in MB (default: 16384 = 16 GB)
#   -d, --benchdir <path>     Benchmark directory (default: benchmarks/sat-comp-2025)
#   -h, --help                Show this help

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# --- defaults ---
TIMEOUT=1800
MEMLIMIT_MB=16384
BENCH_DIR=""
declare -a SOLVERS=()

usage() {
    sed -n '2,/^$/s/^# \?//p' "$0"
    exit 0
}

# --- parse arguments ---
while [[ $# -gt 0 ]]; do
    case "$1" in
        -t|--timeout)  TIMEOUT="$2"; shift 2 ;;
        -m|--memory)   MEMLIMIT_MB="$2"; shift 2 ;;
        -d|--benchdir) BENCH_DIR="$2"; shift 2 ;;
        -h|--help)     usage ;;
        -*)            echo "Unknown option: $1" >&2; exit 1 ;;
        *)             SOLVERS+=("$1"); shift ;;
    esac
done

# Default solvers
if [[ ${#SOLVERS[@]} -eq 0 ]]; then
    SOLVERS=(kissat-latest kissat-sc2024 minisat)
fi

# Default benchmark dir
if [[ -z "$BENCH_DIR" ]]; then
    BENCH_DIR="$REPO_ROOT/benchmarks/sat-comp-2025"
fi

if [[ ! -d "$BENCH_DIR" ]]; then
    echo "ERROR: benchmark directory not found: $BENCH_DIR" >&2
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
    exit 1
fi

# --- resolve solver binaries ---
declare -A SOLVER_BIN
REF_DIR="$REPO_ROOT/benchmarks/reference-solvers"

for s in "${SOLVERS[@]}"; do
    case "$s" in
        kissat-latest)
            bin="$REF_DIR/kissat-latest/build/kissat"
            ;;
        kissat-sc2024)
            bin="$REF_DIR/kissat-sc2024/build/kissat"
            ;;
        minisat)
            bin="$REF_DIR/minisat/build/release/bin/minisat"
            ;;
        *)
            echo "ERROR: unknown reference solver: $s" >&2
            echo "Available: kissat-latest, kissat-sc2024, minisat" >&2
            exit 1
            ;;
    esac
    if [[ ! -x "$bin" ]]; then
        echo "ERROR: $s binary not found at $bin — build it first" >&2
        exit 1
    fi
    SOLVER_BIN[$s]="$bin"
done

# --- collect benchmark instances (prefer .cnf, fall back to .cnf.xz/.cnf.gz) ---
declare -A INSTANCES
while IFS= read -r f; do
    base="${f%.xz}"
    base="${base%.gz}"
    # Only keep the base .cnf path; prefer uncompressed if it exists
    if [[ "$f" == *.cnf ]] || [[ ! -v "INSTANCES[$base]" ]]; then
        INSTANCES["$base"]="$f"
    fi
done < <(find -L "$BENCH_DIR" \( -name '*.cnf' -o -name '*.cnf.gz' -o -name '*.cnf.xz' \) -type f | sort)

# Sort instance keys
mapfile -t SORTED_KEYS < <(printf '%s\n' "${!INSTANCES[@]}" | sort)
TOTAL=${#SORTED_KEYS[@]}

if [[ $TOTAL -eq 0 ]]; then
    echo "ERROR: no benchmark instances found in $BENCH_DIR" >&2
    exit 1
fi

# --- memory limit ---
MEMLIMIT_KB=$((MEMLIMIT_MB * 1024))

# --- run one solver across all instances ---
run_solver() {
    local solver_name="$1"
    local solver_bin="$2"

    local timestamp
    timestamp=$(date +%Y-%m-%d-%H-%M-%S)
    local log_dir="$REPO_ROOT/log/bench-${solver_name}-${timestamp}"
    mkdir -p "$log_dir"

    local results_csv="$log_dir/results.csv"
    echo "instance,result,time_s,timeout,exit_code" > "$results_csv"

    local temp_dir
    temp_dir=$(mktemp -d)

    local solved=0 sat_count=0 unsat_count=0 unknown_count=0 timeout_count=0 error_count=0
    local total_time=0
    local idx=0

    echo "=== $solver_name === ($TOTAL instances, ${TIMEOUT}s timeout, ${MEMLIMIT_MB}MB)"

    for base in "${SORTED_KEYS[@]}"; do
        idx=$((idx + 1))
        local cnf="${INSTANCES[$base]}"
        local name
        name=$(basename "$cnf" .cnf.xz)
        name=$(basename "$name" .cnf.gz)
        name=$(basename "$name" .cnf)

        # Decompress if needed
        local solver_input="$cnf"
        if [[ "$cnf" == *.cnf.xz ]]; then
            solver_input="$temp_dir/$name.cnf"
            xz -dkc "$cnf" > "$solver_input"
        elif [[ "$cnf" == *.cnf.gz ]]; then
            # kissat and minisat can both read plain cnf; minisat can read .gz natively
            # but for consistency, decompress
            solver_input="$temp_dir/$name.cnf"
            gzip -dkc "$cnf" > "$solver_input"
        fi

        local start_time end_time elapsed
        local output=""
        local exit_code=0

        start_time=$(date +%s.%N 2>/dev/null || date +%s)
        output=$(
            ulimit -v "$MEMLIMIT_KB" 2>/dev/null
            "$TIMEOUT_CMD" "$TIMEOUT" "$solver_bin" "$solver_input" 2>/dev/null
        ) || exit_code=$?
        end_time=$(date +%s.%N 2>/dev/null || date +%s)

        if command -v bc &>/dev/null; then
            elapsed=$(echo "$end_time - $start_time" | bc)
        else
            elapsed=$(perl -e "printf '%.3f', $end_time - $start_time")
        fi

        # Extract result
        local s_line result
        s_line=$(echo "$output" | grep '^s ' | head -1) || true

        if [[ $exit_code -eq 124 ]]; then
            result="TIMEOUT"
        elif [[ -z "$s_line" ]]; then
            # kissat uses exit code 10=SAT, 20=UNSAT
            if [[ $exit_code -eq 10 ]]; then
                result="SAT"
            elif [[ $exit_code -eq 20 ]]; then
                result="UNSAT"
            else
                result="ERROR"
            fi
        else
            case "$s_line" in
                "s SATISFIABLE")   result="SAT" ;;
                "s UNSATISFIABLE") result="UNSAT" ;;
                "s UNKNOWN")       result="UNKNOWN" ;;
                *)                 result="ERROR" ;;
            esac
        fi

        # Clamp elapsed
        local clamped_time
        clamped_time=$(perl -e "printf '%.3f', ($elapsed > $TIMEOUT) ? $TIMEOUT : $elapsed")

        # Clean up temp file
        [[ "$cnf" != "$solver_input" ]] && rm -f "$solver_input"

        echo "$name,$result,$clamped_time,$TIMEOUT,$exit_code" >> "$results_csv"

        # Update counters
        case "$result" in
            SAT)
                sat_count=$((sat_count + 1))
                solved=$((solved + 1))
                total_time=$(perl -e "printf '%.3f', $total_time + $clamped_time")
                ;;
            UNSAT)
                unsat_count=$((unsat_count + 1))
                solved=$((solved + 1))
                total_time=$(perl -e "printf '%.3f', $total_time + $clamped_time")
                ;;
            UNKNOWN)
                unknown_count=$((unknown_count + 1))
                total_time=$(perl -e "printf '%.3f', $total_time + 2 * $TIMEOUT")
                ;;
            TIMEOUT)
                timeout_count=$((timeout_count + 1))
                total_time=$(perl -e "printf '%.3f', $total_time + 2 * $TIMEOUT")
                ;;
            ERROR)
                error_count=$((error_count + 1))
                total_time=$(perl -e "printf '%.3f', $total_time + 2 * $TIMEOUT")
                ;;
        esac

        local status_icon
        case "$result" in
            SAT)     status_icon="SAT    " ;;
            UNSAT)   status_icon="UNSAT  " ;;
            UNKNOWN) status_icon="UNKNOWN" ;;
            TIMEOUT) status_icon="TIMEOUT" ;;
            ERROR)   status_icon="ERROR  " ;;
        esac
        printf "[$solver_name] [%3d/%d] %s %8.2fs  %s\n" "$idx" "$TOTAL" "$status_icon" "$clamped_time" "$name"
    done

    rm -rf "$temp_dir"

    local unsolved=$((TOTAL - solved))

    # Write summary
    local summary="$log_dir/summary.log"
    {
        echo "=== Benchmark Results: $solver_name ==="
        echo "    Date:      $(date)"
        echo "    Timeout:   ${TIMEOUT}s"
        echo "    Memory:    ${MEMLIMIT_MB} MB"
        echo ""
        echo "    Instances: $TOTAL"
        echo "    Solved:    $solved ($sat_count SAT + $unsat_count UNSAT)"
        echo "    Unsolved:  $unsolved ($timeout_count timeout + $unknown_count unknown + $error_count error)"
        echo ""
        echo "    PAR-2:     $total_time"
        echo ""
        echo "    Results:   $results_csv"
    } | tee "$summary"

    echo ""
}

# --- launch all solvers in parallel ---
declare -a PIDS=()
for s in "${SOLVERS[@]}"; do
    run_solver "$s" "${SOLVER_BIN[$s]}" &
    PIDS+=($!)
    echo "Launched $s (PID ${PIDS[-1]})"
done

echo ""
echo "Waiting for all solvers to finish..."
echo ""

# Wait for all and collect exit codes
FAIL=0
for pid in "${PIDS[@]}"; do
    if ! wait "$pid"; then
        FAIL=1
    fi
done

echo ""
echo "=== All reference solver runs complete ==="
if [[ $FAIL -ne 0 ]]; then
    echo "WARNING: one or more solvers had errors"
fi
