#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SLUG="analyzesat-2026-05-26-branch-phase-chrono"
OUT="$ROOT/log/$SLUG"
MATRIX="$OUT/config_matrix.psv"
BASELINE_RESULTS="$OUT/A_default/results.csv"

cd "$ROOT"
mkdir -p "$OUT"

run_config() {
    local label="$1"
    local env_string="$2"
    local purpose="$3"
    local dir="$OUT/$label"

    mkdir -p "$dir"
    {
        echo "label=$label"
        echo "env=$env_string"
        echo "purpose=$purpose"
        echo "started=$(date -Is)"
    } > "$dir/run.meta"

    echo "=== $label ==="
    echo "env: $env_string"
    echo "purpose: $purpose"

    # shellcheck disable=SC2086
    env $env_string /usr/bin/time -v \
        bash tools/bench.sh -t 300 -m 16384 -d benchmarks/profiling \
        --log-dir "log/$SLUG/$label" solver/11-kissat-port \
        > "$dir/bench.stdout" 2> "$dir/bench.stderr"

    echo "finished=$(date -Is)" >> "$dir/run.meta"
}

check_baseline_failures() {
    local label="$1"
    local dir="$OUT/$label"
    local failures="$dir/baseline_solved_failures.csv"

    if [[ "$label" == "A_default" || ! -f "$BASELINE_RESULTS" ]]; then
        return 0
    fi

    python3 - "$BASELINE_RESULTS" "$dir/results.csv" "$failures" <<'PY'
import csv
import sys

baseline_path, candidate_path, failures_path = sys.argv[1:]
solved = {"SAT", "UNSAT"}

with open(baseline_path, newline="") as f:
    base = {row["instance"]: row for row in csv.DictReader(f)}
with open(candidate_path, newline="") as f:
    cand = {row["instance"]: row for row in csv.DictReader(f)}

rows = []
for instance, brow in base.items():
    crow = cand.get(instance)
    if not crow:
        continue
    if brow["result"] in solved and crow["result"] not in solved:
        rows.append(crow)

if rows:
    with open(failures_path, "w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=rows[0].keys())
        writer.writeheader()
        writer.writerows(rows)
    for row in rows:
        print(f"{row['instance']}: baseline solved, candidate {row['result']} in {row['time_s']}s")
    sys.exit(2)
PY
}

while IFS='|' read -r label env_string purpose; do
    [[ "$label" == "label" ]] && continue
    [[ -z "$label" ]] && continue

    run_config "$label" "$env_string" "$purpose"
    check_baseline_failures "$label"
done < "$MATRIX"

python3 "$OUT/analysis.py"
