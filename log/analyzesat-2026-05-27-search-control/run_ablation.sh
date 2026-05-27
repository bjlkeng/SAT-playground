#!/usr/bin/env bash
# Multi-config ablation for the "search-control axis" angle.
#
# Tests three knobs not previously ablated in analyzesat investigations:
#   - SAT_CHRONO + SAT_CHRONO_MAX_DELTA (chronological backtracking)
#   - SAT_BRANCH_MODE (minisat vs occurrence initial branch order)
#   - SAT_INITIAL_CLAUSE_MODE (clause iteration order at setup)
#
# Worktree: /tmp/analyzesat-2026-05-27-search-control-1779879326
# Output:   /home/bojji/code/SAT-playground/log/analyzesat-2026-05-27-search-control/<config>/
#
# Method: run each config sequentially against benchmarks/profiling (10 instances)
# at 300s timeout, 16 GiB memory, with SAT_STATS_JSON=on so JSON_STATS rows are
# captured per instance for work × speed decomposition.

set -euo pipefail

WT="/tmp/analyzesat-2026-05-27-search-control-1779879326"
SLUG_DIR="/home/bojji/code/SAT-playground/log/analyzesat-2026-05-27-search-control"
BENCH="/home/bojji/code/SAT-playground/benchmarks/profiling"
SOLVER_RELDIR="solver/11-kissat-port"
TIMEOUT=300
MEM=16384

run_cfg() {
    local label="$1"
    shift
    local env_str="$*"
    local out="$SLUG_DIR/$label"
    mkdir -p "$out"
    echo "=== $label  env: $env_str ==="
    (
        cd "$WT"
        # shellcheck disable=SC2086
        env $env_str SAT_STATS_JSON=on \
            bash tools/bench.sh -t $TIMEOUT -m $MEM -d "$BENCH" --log-dir "$out" "$SOLVER_RELDIR" \
            > "$out/driver.log" 2>&1 || true
    )
    echo "    done -> $out"
}

run_cfg A_baseline ''
run_cfg B_chrono 'SAT_CHRONO=on'
# Reduced scope: skip C/D/E/F because host contention is high.
# Add the rest by re-running the script after manually moving the existing dirs.
# run_cfg C_chrono_aggressive 'SAT_CHRONO=on SAT_CHRONO_MAX_DELTA=10000'
# run_cfg D_branch_occurrence 'SAT_BRANCH_MODE=occurrence'
# run_cfg E_initial_kissat_watch 'SAT_INITIAL_CLAUSE_MODE=kissat-watch'
# run_cfg F_search_control_combined 'SAT_CHRONO=on SAT_INITIAL_CLAUSE_MODE=kissat-watch'

echo "ALL DONE: $SLUG_DIR"
