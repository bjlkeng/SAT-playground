#!/bin/bash
# Wait for the machine to go quiet (no other agent's profiling bench / sat-solver
# on a .cnf), then run the perf measurement on clean cores. Cap the wait so we
# never hang indefinitely.
set -u
LOG=/home/bojji/code/SAT-playground/log/analyzesat-2026-05-28-perf-tax

busy() {
  # other agent's profiling-suite wrapper?
  pgrep -f 'tools/bench.sh .*benchmarks/profiling' >/dev/null 2>&1 && return 0
  # any sat-solver running on a .cnf that is NOT one of my perf-tax runs?
  ps -eo args | grep -E 'sat-solver' | grep -v 'grep' | grep -v 'perf-tax-proof' | grep -q '\.cnf' && return 0
  return 1
}

waited=0
CAP=1800
echo "WAIT-START $(date -Is)" >> "$LOG/PROGRESS.txt"
while busy; do
  if [ "$waited" -ge "$CAP" ]; then
    echo "WAIT-CAP-REACHED after ${waited}s — proceeding anyway $(date -Is)" >> "$LOG/PROGRESS.txt"
    break
  fi
  sleep 20
  waited=$((waited+20))
done
echo "QUIET after ${waited}s $(date -Is)" >> "$LOG/PROGRESS.txt"

# small settle delay
sleep 3
bash "$LOG/run_perf_tax.sh"
