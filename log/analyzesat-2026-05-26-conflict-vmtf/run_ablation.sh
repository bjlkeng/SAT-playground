#!/usr/bin/env bash
# analyzesat-2026-05-26-conflict-vmtf — Multi-config ablation over the conflict
# analysis / clause minimization / OTFS / VMTF feature axis. This is a different
# place from prior runs (restart, lucky, focused-stable, BVE/BSR).
#
# Worktree: /tmp/analyzesat-2026-05-26-conflict-vmtf  (HEAD e7ec1f8)
# Solver:   solver/11-kissat-port
# Timeout:  300 s per instance, 16 GiB memory
# Suite:    benchmarks/profiling/ (10 instances)

set -euo pipefail

ROOT="/tmp/analyzesat-2026-05-26-conflict-vmtf"
BENCH_DIR="$ROOT/benchmarks/profiling"
OUT_ROOT="/home/bojji/code/SAT-playground/log/analyzesat-2026-05-26-conflict-vmtf"

TIMEOUT_S=300
MEM_MB=16384

# 7 configs, fresh angle on conflict analysis primitives
declare -A CONFIGS=(
  [A_baseline]=""
  [B_ccmin_off]="SAT_CLAUSE_MIN=off"
  [C_ccmin_basic]="SAT_CLAUSE_MIN=basic"
  [D_ccmin_inblock]="SAT_CLAUSE_MIN=inblock"
  [E_otfs_on]="SAT_OTFS=on"
  [F_resolved]="SAT_CONFLICT_ANALYSIS_MODE=resolved"
  [G_deep_min]="SAT_MINIMIZE_DEPTH_LIMIT=1000000000"
)

ORDER=(A_baseline B_ccmin_off C_ccmin_basic D_ccmin_inblock E_otfs_on F_resolved G_deep_min)

echo "[ablation] starting at $(date -Iseconds)"
echo "[ablation] worktree=$ROOT"
echo "[ablation] bench=$BENCH_DIR ($(ls "$BENCH_DIR"/*.cnf.xz | wc -l) instances)"

for cfg in "${ORDER[@]}"; do
  envstr="${CONFIGS[$cfg]}"
  cfg_dir="$OUT_ROOT/$cfg"
  mkdir -p "$cfg_dir"
  echo
  echo "=== config $cfg :: env=\"SAT_STATS_JSON=on $envstr\" ==="
  echo "$envstr" > "$cfg_dir/env.txt"

  # Use bench.sh with custom log-dir; pass env vars via env(1) preserving inheritance.
  set +u
  if [[ -n "$envstr" ]]; then
    env_arr=()
    for kv in $envstr; do env_arr+=("$kv"); done
  else
    env_arr=()
  fi
  set -u

  env SAT_STATS_JSON=on "${env_arr[@]}" \
    bash "$ROOT/tools/bench.sh" \
      -t "$TIMEOUT_S" \
      -m "$MEM_MB" \
      -d "$BENCH_DIR" \
      --log-dir "$cfg_dir" \
      solver/11-kissat-port 2>&1 | tee "$cfg_dir/bench.log" | tail -30

  if [[ -f "$cfg_dir/results.csv" ]]; then
    python3 - <<PY
import csv
rows = list(csv.DictReader(open("$cfg_dir/results.csv")))
par2 = 0.0
solved = 0
to = 0
err = 0
unk = 0
for r in rows:
    try:
        t = float(r.get("time_s", 0) or 0)
    except Exception:
        t = 0
    res = r.get("result", "")
    if res in ("SAT", "UNSAT"):
        par2 += t
        solved += 1
    elif res == "TIMEOUT":
        par2 += 2 * $TIMEOUT_S
        to += 1
    elif res == "UNKNOWN":
        par2 += 2 * $TIMEOUT_S
        unk += 1
    else:
        par2 += 2 * $TIMEOUT_S
        err += 1
print(f"[$cfg] PAR2={par2:.1f}  solved={solved}  timeout={to}  unknown={unk}  err={err}")
PY
  else
    echo "[$cfg] NO results.csv found!"
  fi
done

echo
echo "[ablation] done at $(date -Iseconds)"
