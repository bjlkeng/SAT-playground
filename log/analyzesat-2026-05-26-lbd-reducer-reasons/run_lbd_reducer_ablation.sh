#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../" && pwd)"
OUT="$ROOT/log/analyzesat-2026-05-26-lbd-reducer-reasons"
mkdir -p "$OUT"

TIMEOUT_S="${TIMEOUT_S:-300}"
MEM_MB="${MEM_MB:-16384}"
LIMIT_WALL_S="${LIMIT_WALL_S:-295}"

declare -a CONFIGS=(
  "A_default|SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=$LIMIT_WALL_S"
  "B_lbd_metadata|SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=$LIMIT_WALL_S SAT_USE_LBD=on"
  "C_reason_lbd|SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=$LIMIT_WALL_S SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on"
  "D_lbd_tiered|SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=$LIMIT_WALL_S SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_REDUCE=lbd-tiered"
  "E_lbd_tiered_prop_reasons|SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=$LIMIT_WALL_S SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_LBD_UPDATE_PROP_REASONS=on SAT_REDUCE=lbd-tiered"
  "F_lbd_tiered_reset|SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=$LIMIT_WALL_S SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_REDUCE=lbd-tiered SAT_POST_PREPROCESS_REDUCE_DB_RESET=on"
  "G_lbd_tiered_delayed|SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=$LIMIT_WALL_S SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_REDUCE=lbd-tiered SAT_REDUCE_DB_INIT=100000"
  "H_lbd_tiered_slow_interval|SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=$LIMIT_WALL_S SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_REDUCE=lbd-tiered SAT_REDUCE_DB_INTERVAL=100000"
)

: > "$OUT/configs.tsv"
printf 'config\tenv\n' >> "$OUT/configs.tsv"

for entry in "${CONFIGS[@]}"; do
  IFS='|' read -r name envspec <<< "$entry"
  log_dir="$OUT/$name"
  mkdir -p "$log_dir"
  printf '%s\t%s\n' "$name" "$envspec" >> "$OUT/configs.tsv"
  echo "=== $name ==="
  echo "$envspec"
  read -r -a env_parts <<< "$envspec"
  env "${env_parts[@]}" bash "$ROOT/tools/bench.sh" \
    -t "$TIMEOUT_S" -m "$MEM_MB" -d "$ROOT/benchmarks/profiling" \
    --log-dir "$log_dir" solver/11-kissat-port
done
