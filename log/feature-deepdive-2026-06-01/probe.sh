#!/usr/bin/env bash
# Instrumented single-instance probe: run a config with SAT_STATS_JSON and extract key counters.
set -u
SOLVER=/home/bojji/code/SAT-playground/solver/11-kissat-port/target/release/sat-solver
INST_DIR=/tmp/p20deep
OUT=/tmp/p20deep/probe.tsv
TIMEOUT=${TIMEOUT:-300}

declare -A CFG=(
  [default]=""
  [chrono]="SAT_CHRONO=on"
  [binary_fast]="SAT_BINARY_FAST=on"
  [ema]="SAT_RESTART=kissat-ema"
  [target_phase]="SAT_PHASE=target-then-saved"
)
ORDER=(default chrono binary_fast ema target_phase)

keys='result time_s conflicts propagations decisions restarts luby_restarts glucose_restarts focused_restarts reluctant_restarts mode_switches chrono_attempts chrono_used chrono_rejected_not_asserting chrono_rejected_delta_small chrono_skipped_levels phase_saved_used phase_legacy_used phase_target_used phase_best_used phase_initial_used binary_props binary_stale_skips rephases search_ticks learned_clauses'

printf 'instance\tconfig' > "$OUT"
for k in $keys; do printf '\t%s' "$k" >> "$OUT"; done
printf '\n' >> "$OUT"

for inst in "$@"; do
  cnf="$INST_DIR/$inst.cnf"
  [ -f "$cnf" ] || { echo "missing $cnf"; continue; }
  for tag in "${ORDER[@]}"; do
    env_extra="${CFG[$tag]}"
    od=$(mktemp -d)
    t0=$(date +%s.%N)
    # shellcheck disable=SC2086
    out=$(timeout "$TIMEOUT" env $env_extra SAT_STATS_JSON=on "$SOLVER" "$cnf" "$od" 2>"$od/err" )
    rc=$?
    t1=$(date +%s.%N)
    res=$(printf '%s' "$out" | grep -oE '^s [A-Z]+' | awk '{print $2}'); [ -z "$res" ] && res="TIMEOUT/$rc"
    js=$(grep -oE '\{.*\}' "$od/err" | tail -1)
    printf '%s\t%s' "$inst" "$tag" >> "$OUT"
    for k in $keys; do
      if [ "$k" = "result" ]; then printf '\t%s' "$res" >> "$OUT"
      elif [ "$k" = "time_s" ]; then printf '\t%.1f' "$(echo "$t1-$t0"|bc)" >> "$OUT"
      else
        v=$(printf '%s' "$js" | grep -oE "\"$k\":[0-9.eE+-]+" | head -1 | cut -d: -f2)
        printf '\t%s' "${v:-NA}" >> "$OUT"
      fi
    done
    printf '\n' >> "$OUT"
    echo "[$inst/$tag] $res $(echo "$t1-$t0"|bc)s"
    rm -rf "$od"
  done
done
echo "wrote $OUT"
