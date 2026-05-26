#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="$ROOT/log/analyzesat-2026-05-26-preprocess-proof"
TMP="${TMPDIR:-/tmp}/analyzesat-preprocess-proof-$$"
mkdir -p "$OUT/runs" "$TMP/bench"
trap 'rm -rf "$TMP"' EXIT

TIMEOUT="${TIMEOUT:-300}"
MEM_MB="${MEM_MB:-16384}"
SOLVER="solver/11-kissat-port"
BENCH="$ROOT/benchmarks/profiling"

CONFIGS=(
  default
  no_bve
  no_full_bsr
  no_simplification
  input_order
  raw_order
  proof_off
)

ENV_default=(SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 SAT_TRACE_PREPROCESS=1)
ENV_no_bve=(SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 SAT_TRACE_PREPROCESS=1 SAT_BVE=off)
ENV_no_full_bsr=(SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 SAT_TRACE_PREPROCESS=1 SAT_FULL_BSR=off)
ENV_no_simplification=(SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 SAT_TRACE_PREPROCESS=1 SAT_SIMPLIFICATION=off)
ENV_input_order=(SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 SAT_TRACE_PREPROCESS=1 SAT_INITIAL_CLAUSE_MODE=input-order)
ENV_raw_order=(SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 SAT_TRACE_PREPROCESS=1 SAT_INITIAL_CLAUSE_MODE=raw)
ENV_proof_off=(SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 SAT_TRACE_PREPROCESS=1 SAT_PROOF=off)

safe_name() {
  local name="$1"
  name="${name%.cnf.xz}"
  name="${name%.cnf.gz}"
  name="${name%.cnf}"
  printf '%s' "$name"
}

read_result_row() {
  local csv="$1"
  tail -n +2 "$csv" | head -1
}

append_stats() {
  local config="$1"
  local stats="$2"
  [[ -s "$stats" ]] || return 0
  python3 - "$config" "$stats" "$OUT/all_stats.jsonl" <<'PY'
import json
import sys

config, stats_path, out_path = sys.argv[1:]
with open(out_path, "a", encoding="utf-8") as out:
    with open(stats_path, encoding="utf-8") as src:
        for line in src:
            if not line.strip():
                continue
            record = json.loads(line)
            record["config"] = config
            print(json.dumps(record, sort_keys=True, separators=(",", ":")), file=out)
PY
}

mapfile -t INSTANCES < <(find -L "$BENCH" -maxdepth 1 \( -name '*.cnf' -o -name '*.cnf.gz' -o -name '*.cnf.xz' \) -type f | sort)
if [[ "${#INSTANCES[@]}" -eq 0 ]]; then
  echo "no profiling instances found under $BENCH" >&2
  exit 1
fi

: > "$OUT/all_stats.jsonl"
printf 'config,instance,result,verified,time_s,timeout,exit_code,baseline_result,baseline_verified,log_dir,stopped_after_regression\n' > "$OUT/matrix_results.csv"

declare -A BASE_RESULT=()
declare -A BASE_VERIFIED=()

for config in "${CONFIGS[@]}"; do
  echo "=== config: $config ==="
  stopped=0
  for idx in "${!INSTANCES[@]}"; do
    cnf="${INSTANCES[$idx]}"
    base="$(basename "$cnf")"
    inst="$(safe_name "$base")"
    singleton="$TMP/bench/$inst"
    mkdir -p "$singleton"
    ln -sf "$cnf" "$singleton/$base"

    run_dir="$OUT/runs/$config/$inst"
    mkdir -p "$(dirname "$run_dir")"

    env_name="ENV_$config[@]"
    echo "[$config $((idx + 1))/${#INSTANCES[@]}] $inst"
    env "${!env_name}" bash "$ROOT/tools/bench.sh" -t "$TIMEOUT" -m "$MEM_MB" -d "$singleton" --log-dir "$run_dir" "$SOLVER"

    row="$(read_result_row "$run_dir/results.csv")"
    IFS=',' read -r name result verified time_s timeout_s exit_code <<<"$row"
    append_stats "$config" "$run_dir/stats.jsonl"

    baseline_result="${BASE_RESULT[$inst]:-}"
    baseline_verified="${BASE_VERIFIED[$inst]:-}"
    if [[ "$config" == "default" ]]; then
      BASE_RESULT[$inst]="$result"
      BASE_VERIFIED[$inst]="$verified"
    fi

    printf '%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
      "$config" "$inst" "$result" "$verified" "$time_s" "$timeout_s" "$exit_code" \
      "$baseline_result" "$baseline_verified" "$run_dir" "0" >> "$OUT/matrix_results.csv"

    if [[ "$config" != "default" && "$baseline_result" =~ ^(SAT|UNSAT)$ ]]; then
      if [[ "$result" != "$baseline_result" || "$result" == "TIMEOUT" || "$result" == "UNKNOWN" || "$result" == "ERROR" ]]; then
        echo "stopping $config after baseline-solved regression on $inst: baseline=$baseline_result candidate=$result"
        python3 - "$OUT/matrix_results.csv" "$config" "$inst" <<'PY'
import csv
import sys
from pathlib import Path

path = Path(sys.argv[1])
config = sys.argv[2]
instance = sys.argv[3]
rows = list(csv.DictReader(path.open(newline="")))
for row in rows:
    if row["config"] == config and row["instance"] == instance:
        row["stopped_after_regression"] = "1"
with path.open("w", newline="") as f:
    writer = csv.DictWriter(f, fieldnames=rows[0].keys(), lineterminator="\n")
    writer.writeheader()
    writer.writerows(rows)
PY
        stopped=1
        break
      fi
    fi
  done
  if [[ "$stopped" -eq 1 ]]; then
    echo "=== config stopped: $config ==="
  fi
done
