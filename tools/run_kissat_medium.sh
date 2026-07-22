#!/usr/bin/env bash
# run_kissat_medium.sh — 32-way parallel kissat-latest sweep over the
# sat-comp-2025-medium suite, matched to the feature_ablation gate conditions
# (32 pinned cores, 16 GB/job ulimit -v, 1800s timeout). Produces a
# results.csv identical in schema to the prior log/kissat-medium-* runs so the
# solver12-vs-kissat gap read is apples-to-apples.
#
# Usage: bash tools/run_kissat_medium.sh [-t timeout_s] [-m mem_mb] [-j jobs]
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TIMEOUT=1800
MEM_MB=16000
JOBS=32
SUITE="$REPO_ROOT/benchmarks/sat-comp-2025-medium"
KISSAT="$REPO_ROOT/benchmarks/reference-solvers/kissat-latest/build/kissat"
SCRATCH="${SAT_KISSAT_SCRATCH:-/tmp/claude-1001/-home-bojji-code/7af02950-fe59-4b78-bf70-b6358cc34e82/scratchpad/kissat-medium-work}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        -t) TIMEOUT="$2"; shift 2 ;;
        -m) MEM_MB="$2"; shift 2 ;;
        -j) JOBS="$2"; shift 2 ;;
        *) echo "unknown arg: $1" >&2; exit 1 ;;
    esac
done

[[ -x "$KISSAT" ]] || { echo "kissat binary not found/executable: $KISSAT" >&2; exit 1; }
KVER="$(cat "$REPO_ROOT/benchmarks/reference-solvers/kissat-latest/VERSION" 2>/dev/null || echo unknown)"

TS="$(date +%Y%m%d-%H%M%S)"
OUT="$REPO_ROOT/log/kissat-medium-${TS}"
mkdir -p "$OUT/cells" "$SCRATCH"
MEM_KB=$((MEM_MB * 1024))

echo "kissat=$KVER instances=$(ls "$SUITE"/*.cnf.xz | wc -l) timeout=${TIMEOUT}s mem=${MEM_MB}MB jobs=${JOBS} out=$OUT" > "$OUT/meta.txt"
cat "$OUT/meta.txt"

# Per-instance worker: $1 = index (for core pinning), $2 = cnf.xz path
run_one() {
    local idx="$1" cnf="$2"
    local name; name="$(basename "$cnf" .cnf.xz)"
    local core=$(( idx % JOBS ))
    local work="$SCRATCH/$name.cnf"
    local cell="$OUT/cells/$name.csv"

    xz -dkc "$cnf" > "$work"

    local start end elapsed exit_code=0 output=""
    start=$(date +%s.%N)
    output=$( ulimit -v "$MEM_KB" 2>/dev/null
              taskset -c "$core" timeout "$TIMEOUT" "$KISSAT" "$work" 2>/dev/null ) || exit_code=$?
    end=$(date +%s.%N)
    elapsed=$(awk "BEGIN{printf \"%.3f\", $end-$start}")
    rm -f "$work"

    local sline result
    sline=$(printf '%s\n' "$output" | grep -m1 '^s ' || true)
    if [[ $exit_code -eq 124 || $exit_code -eq 137 ]]; then
        result=TIMEOUT
    elif [[ "$sline" == *SATISFIABLE* && "$sline" != *UNSATISFIABLE* ]]; then
        result=SAT
    elif [[ "$sline" == *UNSATISFIABLE* ]]; then
        result=UNSAT
    else
        result=UNKNOWN
    fi
    printf '%s,%s,%s,%s,%s\n' "$name" "$result" "$elapsed" "$TIMEOUT" "$exit_code" > "$cell"
    printf '[%3d] %-55s %-8s %8ss  (exit %s)\n' "$idx" "$name" "$result" "$elapsed" "$exit_code"
}
export -f run_one
export OUT SCRATCH KISSAT MEM_KB TIMEOUT JOBS

# Feed instances (deterministic sorted order) to a 32-way xargs pool.
i=0
for cnf in $(ls "$SUITE"/*.cnf.xz | sort); do
    printf '%s\t%s\n' "$i" "$cnf"
    i=$((i+1))
done | xargs -P "$JOBS" -I{} bash -c 'IFS=$'"'"'\t'"'"' read -r idx cnf <<< "{}"; run_one "$idx" "$cnf"'

# Aggregate cells -> results.csv (sorted for stable diff)
echo "instance,result,time_s,timeout,exit_code" > "$OUT/results.csv"
cat "$OUT"/cells/*.csv | sort >> "$OUT/results.csv"

# Summary
echo "=== kissat-medium summary ($OUT) ==="
awk -F, 'NR>1{c[$2]++; tot++} END{for(k in c) printf "%-8s %d\n", k, c[k]; printf "solved   %d/%d\n", c["SAT"]+c["UNSAT"], tot}' "$OUT/results.csv"
touch "$OUT/DONE"
echo "DONE -> $OUT"
