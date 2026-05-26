#!/usr/bin/env bash
# Preprocessing-focused ablation: decompose where solver 11 BVE/BSR helps vs hurts.
# A different angle from prior analyzesat runs (which focused on search policy).
set -euo pipefail

REPO_ROOT="/home/bojji/code/SAT-playground"
SLUG="analyzesat-2026-05-26-preprocess"
OUT_ROOT="$REPO_ROOT/log/$SLUG"
BENCH_DIR="$REPO_ROOT/benchmarks/profiling"
TIMEOUT_S=300
MEM_MB=16384

mkdir -p "$OUT_ROOT"

declare -A CONFIGS=(
    [A_default]=""
    [B_no_simp]="SAT_SIMPLIFICATION=off"
    [C_no_bve]="SAT_BVE=off"
    [D_no_bsr]="SAT_FULL_BSR=off"
)

ORDER=(A_default B_no_simp C_no_bve D_no_bsr)

for cfg in "${ORDER[@]}"; do
    cfg_log_dir="$OUT_ROOT/$cfg"
    if [[ -f "$cfg_log_dir/results.csv" ]] && [[ $(wc -l < "$cfg_log_dir/results.csv") -gt 1 ]]; then
        echo "[skip] $cfg already complete"
        continue
    fi
    rm -rf "$cfg_log_dir"
    mkdir -p "$cfg_log_dir"
    env_str="${CONFIGS[$cfg]}"
    echo "=== $cfg ==="
    echo "    env: $env_str"

    (
        export SAT_STATS_JSON=on
        export SAT_TRACE_PREPROCESS=on
        # shellcheck disable=SC2086
        for kv in $env_str; do
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
        echo "config: $cfg"
        echo "env: SAT_STATS_JSON=on SAT_TRACE_PREPROCESS=on $env_str"
        echo "timeout: $TIMEOUT_S"
        echo "rev: $(git -C "$REPO_ROOT" rev-parse HEAD)"
    } > "$cfg_log_dir/run_meta.txt"

    echo "    $(grep -E 'PAR-2|Solved' "$cfg_log_dir/bench_stdout.log" | tr '\n' ' | ')"
done

echo "=== done ==="
