#!/usr/bin/env bash
# Ablation matrix — clause-DB lifecycle + restart-execution + propagation-primitive axis.
# Investigator angle: prior runs covered restart-policy, focused-stable, LBD bookkeeping, BVE/BSR,
# conflict minimization, OTFS, VMTF. This run looks at the POST-CONFLICT pipeline:
#   - clause DB lifetime (reduce policy, post-preprocess reset)
#   - restart execution (trail reuse vs backtrack(0))
#   - propagation primitive (binary fast path)
#
# Worktree: $WORKTREE
# Slug:     analyzesat-2026-05-26-clausedb-cycle

set -euo pipefail

source /tmp/analyzesat_wt.env

SLUG_DIR="/home/bojji/code/SAT-playground/log/analyzesat-2026-05-26-clausedb-cycle"
SOLVER_REL="solver/11-kissat-port"
BENCH_DIR="${WORKTREE}/benchmarks/profiling"

mkdir -p "$SLUG_DIR"

run_config () {
  local name="$1"; shift
  local env_str="$1"; shift
  echo "== ${name} =="
  echo "ENV: ${env_str}"
  local out="${SLUG_DIR}/${name}"
  mkdir -p "$out"
  # Run the bench from the worktree so it uses worktree's binary
  (
    cd "${WORKTREE}"
    env $env_str \
      SAT_STATS_JSON=on \
      bash tools/bench.sh \
        -t 300 -m 16384 \
        -d "${BENCH_DIR}" \
        --log-dir "${out}" \
        "${SOLVER_REL}" \
      2>&1 | tee "${out}/bench.log"
  )
}

# A_baseline: defaults — legacy reduce, no trail reuse, binary_fast=off, single-mode search.
run_config A_baseline ""

# B_binary_fast: isolated propagation-primitive change.
run_config B_binary_fast "SAT_BINARY_FAST=on"

# C_lbd_tiered: tiered LBD reducer alone (single-mode search) — clause DB policy change.
run_config C_lbd_tiered "SAT_USE_LBD=on SAT_REDUCE=lbd-tiered"

# D_post_reset: post-preprocess DB reset (flush after preprocessing).
run_config D_post_reset "SAT_POST_PREPROCESS_REDUCE_DB_RESET=on"

# E_reuse_trail: kissat-style restart with trail reuse (requires focused-stable + LBD + kissat-ema).
run_config E_reuse_trail "SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_RESTART=kissat-ema SAT_RESTART_REUSE_TRAIL=on"

# F_combined_kissat: combine the four (intended kissat-parity stable stack).
run_config F_combined_kissat "SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_RESTART=kissat-ema SAT_RESTART_REUSE_TRAIL=on SAT_REDUCE=lbd-tiered SAT_BINARY_FAST=on SAT_POST_PREPROCESS_REDUCE_DB_RESET=on"

echo "== DONE =="
