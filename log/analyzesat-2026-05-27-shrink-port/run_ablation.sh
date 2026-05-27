#!/usr/bin/env bash
# Ablation runner for analyzesat-2026-05-27-shrink-port
#
# Axis under test: SAT_CLAUSE_MIN learned-clause-shrink modes
#
# Configs:
#   A_baseline  : SAT_CLAUSE_MIN=recursive-limited (current default)
#   D_inblock   : SAT_CLAUSE_MIN=inblock (new Kissat shrink port, commit b65815c)
#   F_inblock_off : SAT_CLAUSE_MIN=off (no minimization at all — sanity bound)
#
# Each config runs the 10-instance profiling suite at 300 s, 16 GiB.
# Per-instance: result, wall time, conflicts, decisions, propagations, learned
# count, restarts. Stats come from SAT_STATS_JSON=on JSON line emitted by the
# solver after every run.
#
# Host contention caveat: another nextbeads benchmark may be running in
# parallel on the same host. Work counters (conflicts, decisions, propagations)
# are deterministic and unaffected. Wall-time may be 1.5-2x inflated.

set -uo pipefail

SLUG_DIR="/home/bojji/code/SAT-playground/log/analyzesat-2026-05-27-shrink-port"
WORKTREE="/tmp/analyzesat-shrink-1779916112"
SOLVER_BIN="${WORKTREE}/solver/11-kissat-port/target/release/sat-solver"
BENCH_DIR="/home/bojji/code/SAT-playground/benchmarks/profiling"
TIMEOUT_S=300
WALL_LIMIT=$((TIMEOUT_S + 5))

run_config () {
  local cfg="$1"
  local clause_min="$2"
  local outdir="${SLUG_DIR}/${cfg}"
  mkdir -p "$outdir"

  echo "=== Config $cfg: SAT_CLAUSE_MIN=$clause_min ===" | tee -a "$outdir/run.log"
  local csv="$outdir/results.csv"
  echo "instance,result,time_s,conflicts,decisions,propagations,restarts,learned_final,exit_code" > "$csv"

  for cnf_xz in "$BENCH_DIR"/*.cnf.xz; do
    local base
    base=$(basename "$cnf_xz" .cnf.xz)
    local cnf="/tmp/shrink-${cfg}-${base}.cnf"
    local proof_dir="/tmp/shrink-${cfg}-${base}-proof"
    mkdir -p "$proof_dir"
    if ! xz -dkc "$cnf_xz" > "$cnf"; then
      echo "[$cfg] decompress failed: $base" | tee -a "$outdir/run.log"
      continue
    fi

    local stderr_path="$outdir/${base}.stderr"
    local stdout_path="$outdir/${base}.stdout"
    local t_start t_end elapsed result_line conflicts decisions props restarts learned exit_code

    t_start=$(date +%s.%N)
    SAT_PROFILE=default SAT_CLAUSE_MIN="$clause_min" SAT_STATS_JSON=on \
      SAT_LIMIT_WALL_SEC="$TIMEOUT_S" \
      timeout --kill-after=5 "$WALL_LIMIT" \
      "$SOLVER_BIN" "$cnf" "$proof_dir" \
      > "$stdout_path" 2> "$stderr_path"
    exit_code=$?
    t_end=$(date +%s.%N)
    elapsed=$(awk "BEGIN{printf \"%.3f\", ${t_end} - ${t_start}}")

    local sline
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

    # Extract stats from SAT_STATS_JSON line in stderr
    local json_line
    json_line=$(grep -E "^c JSON_STATS " "$stderr_path" | tail -1 || true)
    conflicts=$(echo "$json_line" | grep -oE '"conflicts":[0-9]+' | head -1 | cut -d: -f2)
    decisions=$(echo "$json_line" | grep -oE '"decisions":[0-9]+' | head -1 | cut -d: -f2)
    props=$(echo "$json_line" | grep -oE '"propagations":[0-9]+' | head -1 | cut -d: -f2)
    restarts=$(echo "$json_line" | grep -oE '"restarts":[0-9]+' | head -1 | cut -d: -f2)
    learned=$(echo "$json_line" | grep -oE '"learned_clauses_final":[0-9]+' | head -1 | cut -d: -f2)

    : "${conflicts:=0}" "${decisions:=0}" "${props:=0}" "${restarts:=0}" "${learned:=0}"

    echo "${base},${result_line},${elapsed},${conflicts},${decisions},${props},${restarts},${learned},${exit_code}" >> "$csv"
    echo "[$cfg] $base -> $result_line in ${elapsed}s (conf=${conflicts}, dec=${decisions}, prop=${props})" | tee -a "$outdir/run.log"

    rm -f "$cnf"
    rm -rf "$proof_dir"
  done
  echo "=== Config $cfg done ===" | tee -a "$outdir/run.log"
}

CFG="${1:-all}"
case "$CFG" in
  A) run_config A_baseline recursive-limited ;;
  D) run_config D_inblock inblock ;;
  F) run_config F_inblock_off off ;;
  all)
    run_config A_baseline recursive-limited
    run_config D_inblock inblock
    ;;
  *)
    echo "Unknown config: $CFG"
    exit 2
    ;;
esac
