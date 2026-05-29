#!/bin/bash
# d8p — localize the focused/stable throughput regression (props/s 4.65M->1.22M
# in the 2026-05-23 1.16 matrix) at the symbol level, to prioritize the
# 1.14 throughput subtasks (18.7/18.4/18.9/18.13) by MEASURED impact.
#
# perf-record default vs focused/VMTF on Sudoku (steady-state ~80s window; Sudoku
# TIMEOUTs in focused/VMTF so the window is representative). A light periodic
# search trace (every 200k conflicts) rides along to capture props/s + restart
# rate per config from the same run, without needing completion.
set -u
LOG=/home/bojji/code/SAT-playground/log/d8p-focused-stable-perf
S11=/tmp/sat-worktrees/slate-heron/solver/11-kissat-port/target/release/sat-solver
SUD=/tmp/d8p-cnf/sudoku.cnf
mkdir -p /tmp/d8p-proof
echo "START $(date -Is)" > "$LOG/PROGRESS.txt"

run_rec() {
  local tag="$1"; shift
  local env_str="$*"
  echo "[$(date +%H:%M:%S)] recording $tag ($env_str)" >> "$LOG/PROGRESS.txt"
  timeout 85 perf record -o "$LOG/rec_${tag}.data" -F 1999 -- \
    env SAT_TRACE_SEARCH_INTERVAL=200000 $env_str "$S11" "$SUD" "/tmp/d8p-proof/$tag" \
    2> "$LOG/trace_${tag}.txt" >/dev/null
  perf report -i "$LOG/rec_${tag}.data" --stdio --no-children --sort symbol 2>/dev/null \
    | grep -vE '^#|^$' | head -30 > "$LOG/report_${tag}.txt"
  echo "[$(date +%H:%M:%S)] done $tag" >> "$LOG/PROGRESS.txt"
}

run_rec default
run_rec fvmtf SAT_SEARCH_MODE=focused-stable SAT_USE_LBD=on SAT_MODE_USE_TICKS=on SAT_REDUCE_POLICY=lbd-tiered

echo "ALL DONE $(date -Is)" >> "$LOG/PROGRESS.txt"
