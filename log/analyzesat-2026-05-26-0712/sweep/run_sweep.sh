#!/usr/bin/env bash
# Phase 6 — sweep SAT_RESTART_BLOCK_MARGIN on D_focused_stable_ema's 4 regressing
# instances. Hypothesis: now that EMA window is 100k, decision-level blocking
# rescues 6s299b685 / REGRandom / SCPC / brocard.
set -euo pipefail

REPO_ROOT="/home/bojji/code/SAT-playground"
SLUG_DIR="$REPO_ROOT/log/analyzesat-2026-05-26-0712"
OUT_ROOT="$SLUG_DIR/sweep"
BENCH_DIR="$REPO_ROOT/benchmarks/profiling-d-regressors"
TIMEOUT_S=240
MEM_MB=16384

BASE_ENV="SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_RESTART=kissat-ema"

declare -A MARGINS=(
    [m000]="0"
    [m120]="1.2"
    [m140]="1.4"
    [m160]="1.6"
)

ORDER=(m000 m120 m140 m160)

cd "$REPO_ROOT"

for label in "${ORDER[@]}"; do
    cfg_log_dir="$OUT_ROOT/$label"
    if [[ -f "$cfg_log_dir/results.csv" ]]; then
        echo "[skip] $label"
        continue
    fi
    rm -rf "$cfg_log_dir"
    mkdir -p "$cfg_log_dir"
    margin="${MARGINS[$label]}"
    full_env="$BASE_ENV SAT_RESTART_BLOCK_MARGIN=$margin"
    echo "=== $label (margin=$margin) ==="
    (
        export SAT_STATS_JSON=on
        # shellcheck disable=SC2086
        for kv in $full_env; do
            export "$kv"
        done
        bash "$REPO_ROOT/tools/bench.sh" \
            -t "$TIMEOUT_S" -m "$MEM_MB" \
            -d "$BENCH_DIR" \
            --log-dir "$cfg_log_dir" \
            solver/11-kissat-port \
            > "$cfg_log_dir/bench_stdout.log" 2> "$cfg_log_dir/bench_stderr.log"
    )
    {
        echo "label: $label"
        echo "env: $full_env"
        echo "timeout: $TIMEOUT_S"
        echo "rev: $(git rev-parse HEAD)"
    } > "$cfg_log_dir/run_meta.txt"
    echo "    summary: $(grep -E 'PAR-2|Solved|Timeout' "$cfg_log_dir/bench_stdout.log" | tr '\n' ' | ')"
done

echo "=== sweep done ==="
