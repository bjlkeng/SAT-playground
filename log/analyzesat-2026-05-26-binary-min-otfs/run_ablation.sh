#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/log/analyzesat-2026-05-26-binary-min-otfs"
SOLVER="solver/11-kissat-port"
BENCH="benchmarks/profiling"
TIMEOUT=300
MEMORY=16384

run_config() {
    local label="$1"
    local env_string="$2"
    local log_dir="$OUT/$label"
    mkdir -p "$log_dir"
    printf '%s\n' "$env_string" > "$log_dir/env.txt"
    echo "=== $label ==="
    echo "$env_string"
    (
        cd "$ROOT"
        # shellcheck disable=SC2086
        env $env_string /usr/bin/time -v -o "$log_dir/time-v.log" \
            bash tools/bench.sh -t "$TIMEOUT" -m "$MEMORY" -d "$BENCH" \
            --log-dir "$log_dir" "$SOLVER"
    ) > "$log_dir/bench.stdout.log" 2>&1
}

check_against_baseline() {
    local label="$1"
    python3 - "$OUT/A_default/results.csv" "$OUT/$label/results.csv" "$OUT/$label/baseline_solved_failures.csv" <<'PY'
import csv
import sys

base_path, cfg_path, out_path = sys.argv[1:]
base = {}
with open(base_path, newline="") as f:
    for row in csv.DictReader(f):
        base[row["instance"]] = row

failures = []
with open(cfg_path, newline="") as f:
    for row in csv.DictReader(f):
        b = base.get(row["instance"])
        if b and b["result"] in {"SAT", "UNSAT"} and row["result"] not in {"SAT", "UNSAT"}:
            failures.append(row)

with open(out_path, "w", newline="") as f:
    writer = csv.DictWriter(
        f,
        fieldnames=["instance", "result", "verified", "time_s", "timeout", "exit_code"],
        lineterminator="\n",
    )
    writer.writeheader()
    writer.writerows(failures)

if failures:
    for row in failures:
        print(
            f"baseline-solved failure: {row['instance']} -> {row['result']} "
            f"after {row['time_s']}s",
            file=sys.stderr,
        )
    sys.exit(2)
PY
}

mapfile -t CONFIG_LINES < <(awk -F'|' 'NR > 1 { print $1 "|" $2 }' "$OUT/config_matrix.psv")

for entry in "${CONFIG_LINES[@]}"; do
    label="${entry%%|*}"
    env_string="${entry#*|}"
    run_config "$label" "$env_string"
    if [[ "$label" != "A_default" ]]; then
        check_against_baseline "$label"
    fi
done
