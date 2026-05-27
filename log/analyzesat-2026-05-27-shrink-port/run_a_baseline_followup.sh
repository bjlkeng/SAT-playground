#!/usr/bin/env bash
# Follow-up A_baseline run with JSON_STATS so we can complete the decomposition.
# Runs on the same instances D_inblock already finished, in isolation (no other
# benchmarks should be running for clean comparison).

set -uo pipefail

SLUG_DIR="/home/bojji/code/SAT-playground/log/analyzesat-2026-05-27-shrink-port"
WORKTREE="/tmp/analyzesat-shrink-1779916112"
SOLVER_BIN="${WORKTREE}/solver/11-kissat-port/target/release/sat-solver"
BENCH_DIR="/home/bojji/code/SAT-playground/benchmarks/profiling"
TIMEOUT_S=300

outdir="${SLUG_DIR}/A_baseline"
mkdir -p "$outdir"
csv="$outdir/results.csv"
echo "instance,result,time_s,conflicts,decisions,propagations,restarts,learned_final,exit_code" > "$csv"

# Restrict to fast instances first for quick data
# Then any unfinished D_inblock instances
TARGETS=(
  "3746303c659ef65aaa78f3b52cd5de49-6s299b685_Iter30"
  "46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized"
  "557d7d4db5399188f62bc39598c6d868-mp1-Nb7T46"
  "663bb5659e42c2c75f74354f48895302-SCPC-500-13"
  "9af7646fc4a32c6f2744ddc0c4b654b7-brocard_problem_large"
  "ed6d842f96d10f3400bce251f9e95bfb-battleship-16-31-sat"
  "0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12"
)

for base in "${TARGETS[@]}"; do
  cnf_xz="${BENCH_DIR}/${base}.cnf.xz"
  if [ ! -f "$cnf_xz" ]; then
    continue
  fi
  cnf="/tmp/A-${base}.cnf"
  proof_dir="/tmp/A-${base}-proof"
  mkdir -p "$proof_dir"
  xz -dkc "$cnf_xz" > "$cnf" || continue

  stderr_path="$outdir/${base}.stderr"
  stdout_path="$outdir/${base}.stdout"
  t_start=$(date +%s.%N)
  SAT_PROFILE=default SAT_CLAUSE_MIN=recursive-limited SAT_STATS_JSON=on \
    SAT_LIMIT_WALL_SEC="$TIMEOUT_S" \
    timeout --kill-after=5 305 "$SOLVER_BIN" "$cnf" "$proof_dir" \
    > "$stdout_path" 2> "$stderr_path"
  exit_code=$?
  t_end=$(date +%s.%N)
  elapsed=$(awk "BEGIN{printf \"%.3f\", ${t_end} - ${t_start}}")

  sline=$(grep -E "^s " "$stdout_path" | tail -1 || true)
  case "$sline" in
    "s SATISFIABLE")   result_line="SAT" ;;
    "s UNSATISFIABLE") result_line="UNSAT" ;;
    "s UNKNOWN")       result_line="UNKNOWN" ;;
    *)
      if [ "$exit_code" = "124" ] || [ "$exit_code" = "137" ]; then
        result_line="TIMEOUT"
      else
        result_line="ERROR"
      fi
      ;;
  esac

  json_line=$(grep -E "^c JSON_STATS " "$stderr_path" | tail -1 || true)
  conflicts=$(echo "$json_line" | grep -oE '"conflicts":[0-9]+' | head -1 | cut -d: -f2)
  decisions=$(echo "$json_line" | grep -oE '"decisions":[0-9]+' | head -1 | cut -d: -f2)
  props=$(echo "$json_line" | grep -oE '"propagations":[0-9]+' | head -1 | cut -d: -f2)
  restarts=$(echo "$json_line" | grep -oE '"restarts":[0-9]+' | head -1 | cut -d: -f2)
  learned=$(echo "$json_line" | grep -oE '"learned_clauses_final":[0-9]+' | head -1 | cut -d: -f2)
  : "${conflicts:=0}" "${decisions:=0}" "${props:=0}" "${restarts:=0}" "${learned:=0}"
  echo "${base},${result_line},${elapsed},${conflicts},${decisions},${props},${restarts},${learned},${exit_code}" >> "$csv"
  echo "[A_baseline] $base -> $result_line in ${elapsed}s (conf=${conflicts}, prop=${props})"
  rm -f "$cnf"
  rm -rf "$proof_dir"
done
