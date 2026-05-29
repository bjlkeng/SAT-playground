#!/bin/bash
# Localize the residual ALU instructions: perf-record symbol breakdown of
# solver10 vs solver11-candidate on Sudoku (steady-state search), to decide
# whether the candidate->solver10 instruction excess is a fixable hotspot
# (reason encode/decode set_reason_ref/ReasonCode) or diffuse codegen.
set -u
LOG=/home/bojji/code/SAT-playground/log/analyzesat-2026-05-28-perf-tax
S10=/tmp/analyzesat-2026-05-28-perf-tax/solver/10-bve-preprocess/target/release/sat-solver
S11C=$LOG/sat-solver-11-candidate-rebuilt
SUD=/tmp/perf-tax-cnf/0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12.cnf

# wait for the perf-stat run to finish (clean cores)
while pgrep -f 'run_perf_tax.sh' >/dev/null 2>&1; do sleep 15; done
sleep 3
echo "REC-START $(date -Is)" >> "$LOG/PROGRESS.txt"

# solver10: defaults; solver11-candidate: single/no-LBD parity config
timeout 80 perf record -o "$LOG/rec_s10.data" -F 1999 -- \
  env SAT_TRACE_SEARCH_INTERVAL=2000000000 "$S10" "$SUD" /tmp/rec_s10 >/dev/null 2>&1
perf report -i "$LOG/rec_s10.data" --stdio --no-children --sort symbol 2>/dev/null | grep -vE '^#|^$' | head -25 > "$LOG/report_s10.txt"

timeout 80 perf record -o "$LOG/rec_s11c.data" -F 1999 -- \
  env SAT_USE_LBD=off SAT_SEARCH_MODE=single SAT_MODE_USE_TICKS=off "$S11C" "$SUD" /tmp/rec_s11c >/dev/null 2>&1
perf report -i "$LOG/rec_s11c.data" --stdio --no-children --sort symbol 2>/dev/null | grep -vE '^#|^$' | head -25 > "$LOG/report_s11c.txt"

echo "REC-DONE $(date -Is)" >> "$LOG/PROGRESS.txt"
