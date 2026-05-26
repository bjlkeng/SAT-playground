#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../" && pwd)"
OUT="$ROOT/log/analyzesat-2026-05-26-lbd-reducer-reasons/traces"
mkdir -p "$OUT"

SRC="$ROOT/benchmarks/profiling/557d7d4db5399188f62bc39598c6d868-mp1-Nb7T46.cnf.xz"
CNF="$OUT/mp1-Nb7T46.cnf"
xz -dkc "$SRC" > "$CNF"

run_trace() {
  local name="$1"
  local envspec="$2"
  local proof_dir="$OUT/$name-proof"
  rm -rf "$proof_dir"
  mkdir -p "$proof_dir"
  echo "=== trace $name ==="
  echo "$envspec"
  read -r -a env_parts <<< "$envspec"
  env "${env_parts[@]}" timeout 130 bash "$ROOT/solver/11-kissat-port/run.sh" \
    "$CNF" "$proof_dir" > "$OUT/$name.stdout" 2> "$OUT/$name.stderr" || true
  grep -E 'TRACE_SEARCH|JSON_STATS|^c |^s ' "$OUT/$name.stdout" "$OUT/$name.stderr" \
    > "$OUT/$name.trace_extract.txt" || true
}

common="SAT_STATS_JSON=on SAT_TRACE_SEARCH_INTERVAL=100000 SAT_LIMIT_WALL_SEC=120"
run_trace A_default "$common"
run_trace D_lbd_tiered "$common SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_REDUCE=lbd-tiered"
