#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
LOG_ROOT="$REPO_ROOT/log/analyzesat-2026-05-25-rerun"
MATRIX="$LOG_ROOT/config_matrix.psv"
SOLVER="solver/11-kissat-port"
BENCH_DIR="benchmarks/profiling"
TIMEOUT=300
MEM_MB=16384

mkdir -p "$LOG_ROOT"

BASELINE_RESULTS=""
baseline_solved_file="$LOG_ROOT/A_default/solved_instances.txt"

check_against_baseline() {
    local label="$1"
    local results="$LOG_ROOT/$label/results.csv"
    if [[ "$label" == "A_default" ]]; then
        awk -F, 'NR > 1 && ($2 == "SAT" || $2 == "UNSAT") { print $1 }' "$results" > "$baseline_solved_file"
        return 0
    fi

    local failures="$LOG_ROOT/$label/baseline_solved_failures.csv"
    python3 - "$baseline_solved_file" "$results" "$failures" <<'PY'
import csv
import sys
from pathlib import Path

baseline_solved = set(Path(sys.argv[1]).read_text().splitlines())
results_path = Path(sys.argv[2])
failures_path = Path(sys.argv[3])

rows = []
with results_path.open(newline="") as fh:
    for row in csv.DictReader(fh):
        if row["instance"] in baseline_solved and row["result"] not in {"SAT", "UNSAT"}:
            rows.append(row)

if rows:
    with failures_path.open("w", newline="") as out:
        writer = csv.DictWriter(out, fieldnames=rows[0].keys(), lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    print(f"baseline-solved failures: {failures_path}", file=sys.stderr)
    for row in rows:
        print(f"{row['instance']},{row['result']},{row['time_s']}", file=sys.stderr)
    sys.exit(2)

failures_path.write_text("")
PY
}

while IFS='|' read -r label env_line purpose; do
    if [[ "$label" == "label" || -z "$label" ]]; then
        continue
    fi

    out_dir="$LOG_ROOT/$label"
    mkdir -p "$out_dir"
    {
        echo "label=$label"
        echo "purpose=$purpose"
        echo "env=$env_line"
    } > "$out_dir/env.txt"

    echo "=== $label ==="
    read -r -a env_parts <<< "$env_line"
    env SAT_RUN_LABEL="$label" "${env_parts[@]}" \
        bash "$REPO_ROOT/tools/bench.sh" \
        -t "$TIMEOUT" -m "$MEM_MB" -d "$BENCH_DIR" \
        --log-dir "log/analyzesat-2026-05-25-rerun/$label" \
        "$SOLVER" | tee "$out_dir/bench.stdout.log"

    check_against_baseline "$label"
done < "$MATRIX"

echo "ablation complete: $LOG_ROOT"
