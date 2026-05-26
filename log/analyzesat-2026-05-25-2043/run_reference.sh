#!/usr/bin/env bash
# run_reference.sh — Phase 3 kissat live comparison on benchmarks/profiling.
#
# Runs kissat-latest and kissat-sc2024 against the same 10-instance suite,
# then copies the most-recent results.csv into the analyzesat slug directory.
set -euo pipefail

REPO_ROOT="/home/bojji/code/SAT-playground"
SLUG="analyzesat-2026-05-25-2043"
OUT_ROOT="$REPO_ROOT/log/$SLUG"
BENCH_DIR="$REPO_ROOT/benchmarks/profiling"
TIMEOUT_S=300
MEM_MB=16384

mkdir -p "$OUT_ROOT"
cd "$REPO_ROOT"

for solver in kissat-latest kissat-sc2024; do
    dest_csv="$OUT_ROOT/reference-$solver.csv"
    if [[ -f "$dest_csv" ]]; then
        echo "[skip] $solver already at $dest_csv"
        continue
    fi
    echo "=== Running reference: $solver ==="
    # bench_reference.sh writes to log/bench-<solver>-<timestamp>/results.csv.
    bash "$REPO_ROOT/tools/bench_reference.sh" \
        -t "$TIMEOUT_S" -m "$MEM_MB" \
        -d "$BENCH_DIR" \
        "$solver" \
        > "$OUT_ROOT/reference-$solver.stdout" \
        2> "$OUT_ROOT/reference-$solver.stderr" || \
        echo "WARN: $solver exited non-zero"

    # Find the latest results.csv for this solver.
    latest=$(ls -td "$REPO_ROOT/log/bench-$solver-"*/ 2>/dev/null | head -1 || true)
    if [[ -n "$latest" && -f "$latest/results.csv" ]]; then
        cp "$latest/results.csv" "$dest_csv"
        echo "    copied $latest/results.csv -> $dest_csv"
    else
        echo "WARN: no results.csv found for $solver"
    fi
done

echo "=== reference done ==="
