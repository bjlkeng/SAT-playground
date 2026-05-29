#!/bin/bash
# run_perf_tax.sh — analyzesat 2026-05-28-perf-tax
#
# Resolve the ALU-vs-cache-layout question for the solver10<->solver11
# identical-work execution tax (bead SAT-playground-5b2.2.62), now that
# perf_event_paranoid=1 (was 4) makes hardware counters available again.
#
# Three binaries, single/no-LBD parity config, on Sudoku + Kakuro:
#   solver10            = legacy baseline           (PAR-2 699.671)
#   solver11-head       = T1-T6 present (35429ab)   (PAR-2 753.236)
#   solver11-candidate  = T1-T3 removed (s11-06 NORMAL_SEARCH diff, PAR-2 734.833)
#
# Counters normalized per propagation. Work counts are identical across all
# three (zero-behaviour-change specialization); the script cross-checks that.
#
# perf event sets chosen to avoid PMU multiplexing on this Zen3 host
# (effective ~5-6 counter slots; NMI watchdog takes one).
set -u

LOG=/home/bojji/code/SAT-playground/log/analyzesat-2026-05-28-perf-tax
S10=/tmp/analyzesat-2026-05-28-perf-tax/solver/10-bve-preprocess/target/release/sat-solver
S11H=$LOG/sat-solver-11-head
S11C=$LOG/sat-solver-11-candidate-rebuilt

SUD=/tmp/perf-tax-cnf/0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12.cnf
KAK=/tmp/perf-tax-cnf/5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7.cnf

# decisive set (no multiplex): ALU (cycles/instructions) + layout (L1/dTLB misses)
EV_A="task-clock,cycles,instructions,L1-dcache-load-misses,dTLB-load-misses"
# branch set (no multiplex): mechanism confirmation for T1-T3 branch removal
EV_B="task-clock,cycles,instructions,branches,branch-misses"

S11ENV="SAT_USE_LBD=off SAT_SEARCH_MODE=single SAT_MODE_USE_TICKS=off SAT_STATS_JSON=on"
S10ENV="SAT_TRACE_SEARCH_INTERVAL=2000000000"   # huge => periodic trace never fires, only final "search done"

PROOF=/tmp/perf-tax-proof
mkdir -p "$PROOF"

echo "START $(date -Is)" > "$LOG/PROGRESS.txt"

run() {
  local tag="$1"; local events="$2"; local env_str="$3"; local bin="$4"; local cnf="$5"
  echo "[$(date +%H:%M:%S)] running $tag ..." >> "$LOG/PROGRESS.txt"
  # perf counters -> perf_<tag>.txt ; solver stderr (stats) -> stats_<tag>.txt ; stdout discarded
  perf stat -o "$LOG/perf_${tag}.txt" -e "$events" \
    env $env_str "$bin" "$cnf" "$PROOF/$tag" > /dev/null 2> "$LOG/stats_${tag}.txt"
  echo "[$(date +%H:%M:%S)] done    $tag" >> "$LOG/PROGRESS.txt"
}

# ---- Pass A (decisive: ALU + layout) on both instances ----
run sudoku_s10_A  "$EV_A" "$S10ENV"  "$S10"  "$SUD"
run sudoku_s11h_A "$EV_A" "$S11ENV"  "$S11H" "$SUD"
run sudoku_s11c_A "$EV_A" "$S11ENV"  "$S11C" "$SUD"
run kakuro_s10_A  "$EV_A" "$S10ENV"  "$S10"  "$KAK"
run kakuro_s11h_A "$EV_A" "$S11ENV"  "$S11H" "$KAK"
run kakuro_s11c_A "$EV_A" "$S11ENV"  "$S11C" "$KAK"

# ---- Pass B (branch mechanism) on Sudoku (largest gap) ----
run sudoku_s10_B  "$EV_B" "$S10ENV"  "$S10"  "$SUD"
run sudoku_s11h_B "$EV_B" "$S11ENV"  "$S11H" "$SUD"
run sudoku_s11c_B "$EV_B" "$S11ENV"  "$S11C" "$SUD"

echo "ALL DONE $(date -Is)" >> "$LOG/PROGRESS.txt"
