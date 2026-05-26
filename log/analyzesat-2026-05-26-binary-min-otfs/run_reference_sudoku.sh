#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/log/analyzesat-2026-05-26-binary-min-otfs"
INSTANCE="$OUT/instances/sudoku.cnf"
TIMEOUT=300

printf 'solver,instance,result,time_s,exit_code\n' > "$OUT/reference_sudoku.csv"

run_ref() {
    local label="$1"
    local bin="$2"
    local stdout="$OUT/reference/${label}-sudoku.stdout"
    local stderr="$OUT/reference/${label}-sudoku.stderr"
    local time_log="$OUT/reference/${label}-sudoku.time"
    local start end elapsed code result

    start=$(date +%s.%N)
    set +e
    /usr/bin/time -v -o "$time_log" timeout "$TIMEOUT" "$bin" "$INSTANCE" > "$stdout" 2> "$stderr"
    code=$?
    set -e
    end=$(date +%s.%N)
    elapsed=$(python3 - "$start" "$end" <<'PY'
import sys
print(f"{float(sys.argv[2]) - float(sys.argv[1]):.3f}")
PY
)
    case "$code" in
        10) result="SAT" ;;
        20) result="UNSAT" ;;
        124) result="TIMEOUT" ;;
        *) result="ERROR" ;;
    esac
    printf '%s,sudoku,%s,%s,%s\n' "$label" "$result" "$elapsed" "$code" >> "$OUT/reference_sudoku.csv"
}

run_ref kissat-latest "/home/bojji/code/SAT-playground/benchmarks/reference-solvers/kissat-latest/build/kissat"
run_ref kissat-sc2024 "/home/bojji/code/SAT-playground/benchmarks/reference-solvers/kissat-sc2024/build/kissat"
