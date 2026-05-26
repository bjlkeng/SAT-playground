#!/usr/bin/env bash
# run_ablation.sh — fresh-eyes 6-config ablation over benchmarks/profiling
# for solver/11-kissat-port HEAD 9143376 (2026-05-26).
#
# Differences vs the 2026-05-25 ablation:
#   * old C_lbd_ema (single mode + kissat-ema) is now rejected by config validator —
#     replaced with C_focused_stable (still uses LBD as required by SAT_SEARCH_MODE).
#   * old D_focused_stable now folds in here.
#   * new E_lucky tests SAT_LUCKY=on alone (default-off opt-in since 2026-05-26).
#   * new F_focused_stable_ema_lucky combines lucky with the best non-baseline stack.
set -euo pipefail

REPO_ROOT="/home/bojji/code/SAT-playground"
WORKTREE="/tmp/analyzesat-2026-05-26-0712"
SLUG="analyzesat-2026-05-26-0712"
OUT_ROOT="$REPO_ROOT/log/$SLUG"
BENCH_DIR="$REPO_ROOT/benchmarks/profiling"
TIMEOUT_S=300
MEM_MB=16384

mkdir -p "$OUT_ROOT"

declare -A CONFIGS=(
    [A_baseline]=""
    [B_metadata_only]="SAT_USE_LBD=on"
    [C_focused_stable]="SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable"
    [D_focused_stable_ema]="SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_RESTART=kissat-ema"
    [E_lucky]="SAT_LUCKY=on"
    [F_focused_stable_ema_lucky]="SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_RESTART=kissat-ema SAT_LUCKY=on"
)

ORDER=(A_baseline B_metadata_only C_focused_stable D_focused_stable_ema E_lucky F_focused_stable_ema_lucky)

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
