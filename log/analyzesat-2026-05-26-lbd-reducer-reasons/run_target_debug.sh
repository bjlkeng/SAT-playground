#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../" && pwd)"
OUT="$ROOT/log/analyzesat-2026-05-26-lbd-reducer-reasons"
TIMEOUT_S="${TIMEOUT_S:-300}"
MEM_MB="${MEM_MB:-16384}"
LIMIT_WALL_S="${LIMIT_WALL_S:-295}"

make_target_dir() {
  local target_name="$1"
  local pattern="$2"
  local dir="$OUT/targets/$target_name"
  mkdir -p "$dir"
  rm -f "$dir"/*
  local src
  src=$(find "$ROOT/benchmarks/profiling" -maxdepth 1 -name "$pattern" -type f | head -1)
  if [[ -z "$src" ]]; then
    echo "missing target pattern: $pattern" >&2
    exit 1
  fi
  ln -s "$src" "$dir/$(basename "$src")"
  printf '%s\n' "$dir"
}

run_one() {
  local target="$1"
  local config="$2"
  local envspec="$3"
  local target_dir="$4"
  local log_dir="$OUT/target-$target-$config"
  mkdir -p "$log_dir"
  echo "=== target=$target config=$config ==="
  echo "$envspec"
  read -r -a env_parts <<< "$envspec"
  env "${env_parts[@]}" bash "$ROOT/tools/bench.sh" \
    -t "$TIMEOUT_S" -m "$MEM_MB" -d "$target_dir" \
    --log-dir "$log_dir" solver/11-kissat-port
}

mp1_dir=$(make_target_dir mp1 '557d7d4db5399188f62bc39598c6d868-mp1-Nb7T46.cnf.xz')
regrandom_dir=$(make_target_dir regrandom '46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized.cnf.xz')

base_env="SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=$LIMIT_WALL_S"
lbd_env="$base_env SAT_USE_LBD=on"
reason_env="$lbd_env SAT_LBD_UPDATE_REASONS=on"
tiered_env="$reason_env SAT_REDUCE=lbd-tiered"

run_one mp1 A_default "$base_env" "$mp1_dir"
run_one mp1 C_reason_lbd "$reason_env" "$mp1_dir"
run_one mp1 D_lbd_tiered "$tiered_env" "$mp1_dir"
run_one mp1 I_lbd_tiered_no_reason "$lbd_env SAT_REDUCE=lbd-tiered" "$mp1_dir"
run_one mp1 G_lbd_tiered_delayed "$tiered_env SAT_REDUCE_DB_INIT=100000" "$mp1_dir"
run_one mp1 H_lbd_tiered_slow_interval "$tiered_env SAT_REDUCE_DB_INTERVAL=100000" "$mp1_dir"

run_one regrandom A_default "$base_env" "$regrandom_dir"
run_one regrandom D_lbd_tiered "$tiered_env" "$regrandom_dir"
run_one regrandom G_lbd_tiered_delayed "$tiered_env SAT_REDUCE_DB_INIT=100000" "$regrandom_dir"
run_one regrandom H_lbd_tiered_slow_interval "$tiered_env SAT_REDUCE_DB_INTERVAL=100000" "$regrandom_dir"
