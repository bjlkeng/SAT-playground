#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
LOG_ROOT="$REPO_ROOT/log/analyzesat-2026-05-25-rerun"
INST_DIR="$LOG_ROOT/instances"
TIMEOUT=300
MEM_MB=16384
MEM_KB=$((MEM_MB * 1024))

declare -A REF_BIN=(
    [kissat-latest]="/home/bojji/code/SAT-playground/benchmarks/reference-solvers/kissat-latest/build/kissat"
    [kissat-sc2024]="/home/bojji/code/SAT-playground/benchmarks/reference-solvers/kissat-sc2024/build/kissat"
)

run_one() {
    local solver="$1"
    local inst="$2"
    local cnf="$INST_DIR/$inst.cnf"
    local bin="${REF_BIN[$solver]}"
    local stdout="$LOG_ROOT/reference-${solver}-${inst}.stdout"
    local stderr="$LOG_ROOT/reference-${solver}-${inst}.stderr"

    local start end elapsed exit_code=0 result="ERROR"
    start=$(date +%s.%N)
    (
        ulimit -v "$MEM_KB" 2>/dev/null
        timeout "$TIMEOUT" "$bin" "$cnf" > "$stdout" 2> "$stderr"
    ) || exit_code=$?
    end=$(date +%s.%N)
    elapsed=$(perl -e "printf '%.3f', ($end - $start) > $TIMEOUT ? $TIMEOUT : ($end - $start)")

    if [[ $exit_code -eq 124 ]]; then
        result="TIMEOUT"
    elif [[ $exit_code -eq 10 ]]; then
        result="SAT"
    elif [[ $exit_code -eq 20 ]]; then
        result="UNSAT"
    else
        local s_line
        s_line=$(grep '^s ' "$stdout" | head -1 || true)
        case "$s_line" in
            "s SATISFIABLE") result="SAT" ;;
            "s UNSATISFIABLE") result="UNSAT" ;;
            "s UNKNOWN") result="UNKNOWN" ;;
        esac
    fi

    echo "$inst,$result,$elapsed,$TIMEOUT,$exit_code"
}

for solver in kissat-latest kissat-sc2024; do
    bin="${REF_BIN[$solver]}"
    if [[ ! -x "$bin" ]]; then
        echo "missing reference binary: $bin" >&2
        exit 1
    fi
    out="$LOG_ROOT/reference-failures-${solver}.csv"
    echo "instance,result,time_s,timeout,exit_code" > "$out"
    for inst in mp1 case9 battleship; do
        run_one "$solver" "$inst" >> "$out"
    done
    echo "wrote $out"
done
