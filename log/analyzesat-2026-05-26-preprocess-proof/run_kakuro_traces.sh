#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="$ROOT/log/analyzesat-2026-05-26-preprocess-proof/traces"
TMP="${TMPDIR:-/tmp}/analyzesat-kakuro-trace-$$"
mkdir -p "$OUT" "$TMP"
trap 'rm -rf "$TMP"' EXIT

SRC="$ROOT/benchmarks/profiling/5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7.cnf.xz"
CNF="$TMP/kakuro.cnf"
xz -dkc "$SRC" > "$CNF"

run_trace() {
  local label="$1"
  shift
  local proof_dir="$TMP/proof-$label"
  rm -rf "$proof_dir"
  mkdir -p "$proof_dir"
  echo "trace $label"
  env "$@" \
    SAT_STATS_JSON=on \
    SAT_TRACE_PREPROCESS=1 \
    SAT_TRACE_SEARCH_INTERVAL=20000 \
    SAT_LIMIT_WALL_SEC=295 \
    timeout 300 bash "$ROOT/solver/11-kissat-port/run.sh" "$CNF" "$proof_dir" \
    > "$OUT/$label.stdout" \
    2> "$OUT/$label.stderr"
}

run_trace default
run_trace input_order SAT_INITIAL_CLAUSE_MODE=input-order
