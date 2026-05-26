#!/usr/bin/env bash
# run_ablation.sh — multi-config ablation over benchmarks/profiling for solver 11.
#
# Reads from the analyzesat worktree, writes per-(config, instance) artifacts
# under log/analyzesat-2026-05-25-2043/<config>/.
#
# All configs use SAT_STATS_JSON=on so per-run counters are captured in the
# stats.jsonl that bench.sh writes alongside results.csv.
set -euo pipefail

REPO_ROOT="/home/bojji/code/SAT-playground"
WORKTREE="/tmp/analyzesat-2026-05-25-2043"
SLUG="analyzesat-2026-05-25-2043"
OUT_ROOT="$REPO_ROOT/log/$SLUG"
BENCH_DIR="$REPO_ROOT/benchmarks/profiling"
TIMEOUT_S=300
MEM_MB=16384

mkdir -p "$OUT_ROOT"

declare -A CONFIGS=(
    [A_baseline]=""
    [B_metadata_only]="SAT_USE_LBD=on"
    [C_lbd_ema]="SAT_USE_LBD=on SAT_RESTART=kissat-ema"
    [D_focused_stable]="SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable"
    [E_combined]="SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_RESTART=kissat-ema"
    [F_full_stack]="SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on SAT_RESTART=kissat-ema SAT_REDUCE=lbd-tiered SAT_REPHASE=on SAT_CHRONO=on SAT_BINARY_FAST=on"
)

ORDER=(A_baseline B_metadata_only C_lbd_ema D_focused_stable E_combined F_full_stack)

cd "$WORKTREE"

for cfg in "${ORDER[@]}"; do
    cfg_log_dir="$OUT_ROOT/$cfg"
    if [[ -f "$cfg_log_dir/results.csv" ]]; then
        echo "[skip] $cfg already complete at $cfg_log_dir"
        continue
    fi
    rm -rf "$cfg_log_dir"
    mkdir -p "$cfg_log_dir"
    env_str="${CONFIGS[$cfg]}"
    echo "=== Running config: $cfg ==="
    echo "    env: $env_str"
    echo "    log: $cfg_log_dir"

    # Run bench.sh with SAT_STATS_JSON=on plus the per-config env. The
    # per-config env vars are exported into bench.sh's environment so they
    # are inherited by run.sh and the solver binary.
    (
        export SAT_STATS_JSON=on
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

    # Save the env string used for reproducibility.
    {
        echo "config: $cfg"
        echo "env: SAT_STATS_JSON=on $env_str"
        echo "timeout_s: $TIMEOUT_S"
        echo "memory_mb: $MEM_MB"
        echo "bench_dir: $BENCH_DIR"
        echo "worktree: $WORKTREE"
        echo "rev: $(git -C "$WORKTREE" rev-parse HEAD)"
        echo "date: $(date -Iseconds)"
    } > "$cfg_log_dir/run_meta.txt"

    echo "    summary: $(grep -E 'PAR-2|Solved|Timeout|Errors' "$cfg_log_dir/bench_stdout.log" | tr '\n' ' | ')"
done

echo "=== ablation done ==="
