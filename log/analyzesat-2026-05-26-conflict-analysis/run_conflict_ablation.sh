#!/usr/bin/env bash
# Fresh-eyes AnalyzeSAT pass for solver/11-kissat-port conflict-analysis mode.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../" && pwd)"
SLUG="analyzesat-2026-05-26-conflict-analysis"
OUT_ROOT="$REPO_ROOT/log/$SLUG"
BENCH_DIR="$REPO_ROOT/benchmarks/profiling"
TIMEOUT_S=300
MEM_MB=16384

mkdir -p "$OUT_ROOT"

declare -A CONFIGS=(
    [A_baseline]=""
    [B_resolved]="SAT_CONFLICT_ANALYSIS_MODE=resolved"
    [C_lbd_metadata]="SAT_USE_LBD=on"
    [D_lbd_resolved]="SAT_USE_LBD=on SAT_CONFLICT_ANALYSIS_MODE=resolved"
    [E_focused_stable]="SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable"
    [F_focused_stable_resolved]="SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_CONFLICT_ANALYSIS_MODE=resolved"
)

ORDER=(
    A_baseline
    B_resolved
    C_lbd_metadata
    D_lbd_resolved
    E_focused_stable
    F_focused_stable_resolved
)

cd "$REPO_ROOT"

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
    echo "    env: SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 $env_str"
    echo "    log: $cfg_log_dir"

    (
        export SAT_STATS_JSON=on
        export SAT_LIMIT_WALL_SEC=295
        # shellcheck disable=SC2086
        for kv in $env_str; do
            export "$kv"
        done
        /usr/bin/time -v bash "$REPO_ROOT/tools/bench.sh" \
            -t "$TIMEOUT_S" -m "$MEM_MB" \
            -d "$BENCH_DIR" \
            --log-dir "$cfg_log_dir" \
            solver/11-kissat-port \
            > "$cfg_log_dir/bench_stdout.log" 2> "$cfg_log_dir/bench_stderr.log"
    )

    {
        echo "config: $cfg"
        echo "env: SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 $env_str"
        echo "timeout_s: $TIMEOUT_S"
        echo "memory_mb: $MEM_MB"
        echo "bench_dir: $BENCH_DIR"
        echo "worktree: $REPO_ROOT"
        echo "rev: $(git rev-parse HEAD)"
        echo "date: $(date -Iseconds)"
    } > "$cfg_log_dir/run_meta.txt"

    echo "    summary: $(grep -E 'PAR-2|Solved|Timeout|Unknown|Errors' "$cfg_log_dir/bench_stdout.log" | tr '\n' ' | ')"
done

echo "=== conflict-analysis ablation done ==="
