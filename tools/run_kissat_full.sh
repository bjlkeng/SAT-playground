#!/usr/bin/env bash
# run_kissat_full.sh — parallel kissat-latest sweep over an arbitrary suite,
# generalized from run_kissat_medium.sh with a core-offset so it can share the
# host with a concurrent feature_ablation run on disjoint physical cores.
# No proof emission (reference arm). Produces results.csv in the same schema
# as log/kissat-medium-* runs.
#
# Usage: bash tools/run_kissat_full.sh [-t timeout_s] [-m mem_mb] [-j jobs]
#                                      [-c core_offset] [-d suite_dir]
#                                      [-k solver_binary] [-n run_name]
# -k runs another kissat-CLI-compatible binary (e.g. solver/13-kissat-rs's
#    sat-solver) under the identical methodology; -n names the log dir
#    (log/<run_name>-<timestamp>, default kissat-full).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TIMEOUT=3600
MEM_MB=16000
JOBS=14
CORE_OFFSET=18
SUITE="$REPO_ROOT/benchmarks/sat-comp-2025"
KISSAT="$REPO_ROOT/benchmarks/reference-solvers/kissat-latest/build/kissat"
RUN_NAME=kissat-full
SCRATCH="${SAT_KISSAT_SCRATCH:-/tmp/kissat-full-work-$$}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        -t) TIMEOUT="$2"; shift 2 ;;
        -m) MEM_MB="$2"; shift 2 ;;
        -j) JOBS="$2"; shift 2 ;;
        -c) CORE_OFFSET="$2"; shift 2 ;;
        -d) SUITE="$2"; shift 2 ;;
        -k) KISSAT="$2"; shift 2 ;;
        -n) RUN_NAME="$2"; shift 2 ;;
        *) echo "unknown arg: $1" >&2; exit 1 ;;
    esac
done

[[ -x "$KISSAT" ]] || { echo "kissat binary not found/executable: $KISSAT" >&2; exit 1; }
KVER="$(cat "$REPO_ROOT/benchmarks/reference-solvers/kissat-latest/VERSION" 2>/dev/null || echo unknown)"

TS="$(date +%Y%m%d-%H%M%S)"
OUT="$REPO_ROOT/log/${RUN_NAME}-${TS}"
mkdir -p "$OUT/cells" "$SCRATCH"
MEM_KB=$((MEM_MB * 1024))

echo "binary=$KISSAT sha256=$(sha256sum "$KISSAT" | cut -c1-16)" > "$OUT/meta.txt"
echo "kissat=$KVER suite=$SUITE instances=$(ls "$SUITE"/*.cnf.xz | wc -l) timeout=${TIMEOUT}s mem=${MEM_MB}MB jobs=${JOBS} core_offset=${CORE_OFFSET} out=$OUT" >> "$OUT/meta.txt"
cat "$OUT/meta.txt"

# Socket-balanced core order (mirrors feature_ablation.py numa_balanced_cores
# — keep in sync): alternate sockets over PHYSICAL cpus first, then SMT
# siblings, so any JOBS-sized window splits evenly across both packages.
# CORE_OFFSET is now an index shift into this order (still gives disjoint
# cores versus a concurrent ablation using window [0, jobs)), not a raw
# first-CPU id. Falls back to identity order if lscpu parsing fails.
CORE_ORDER_STR=$(python3 - <<'PYEOF'
import subprocess
try:
    out = subprocess.run(["lscpu", "-p=CPU,CORE,SOCKET"],
                         capture_output=True, text=True, check=True).stdout
    first, rest = {}, {}
    for line in out.splitlines():
        if line.startswith('#') or not line.strip():
            continue
        cpu, core, sock = (int(x) for x in line.split(',')[:3])
        key = (sock, core)
        if key not in first:
            first[key] = cpu
        else:
            rest.setdefault(key, []).append(cpu)
    sockets = sorted({s for s, _ in first})
    phys = {s: sorted(c for (sk, _), c in first.items() if sk == s) for s in sockets}
    sibs = {s: sorted(c for (sk, _), cs in rest.items() if sk == s for c in cs)
            for s in sockets}
    order = []
    for pool in (phys, sibs):
        idx = {s: 0 for s in sockets}
        remaining = sum(len(v) for v in pool.values())
        while remaining:
            for s in sockets:
                if idx[s] < len(pool[s]):
                    order.append(pool[s][idx[s]])
                    idx[s] += 1
                    remaining -= 1
    print(' '.join(map(str, order)))
except Exception:
    print(' '.join(str(i) for i in range(72)))
PYEOF
)
echo "core_order=[$CORE_ORDER_STR]" >> "$OUT/meta.txt"

# Per-instance worker: $1 = index (for core pinning), $2 = cnf.xz path
run_one() {
    local idx="$1" cnf="$2"
    local name; name="$(basename "$cnf" .cnf.xz)"
    local -a _ord=($CORE_ORDER_STR)
    # Acquire a free pinning slot (mkdir is atomic). Pinning by idx % JOBS
    # doubled solvers up on one core whenever xargs refilled a freed slot
    # with an index congruent to a still-running one (found 2026-09-04);
    # a slot pool guarantees exactly one solver per pinned core.
    local slot="" s
    while [[ -z "$slot" ]]; do
        for ((s = 0; s < JOBS; s++)); do
            if mkdir "$SCRATCH/slot.$s" 2>/dev/null; then slot=$s; break; fi
        done
        [[ -z "$slot" ]] && sleep 0.2
    done
    local core=${_ord[$(( (CORE_OFFSET + slot) % ${#_ord[@]} ))]}
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
    rmdir "$SCRATCH/slot.$slot"

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
export OUT SCRATCH KISSAT MEM_KB TIMEOUT JOBS CORE_OFFSET CORE_ORDER_STR

# Feed instances (deterministic sorted order) to the worker pool.
i=0
for cnf in $(ls "$SUITE"/*.cnf.xz | sort); do
    printf '%s\t%s\n' "$i" "$cnf"
    i=$((i+1))
done | xargs -P "$JOBS" -I{} bash -c 'IFS=$'"'"'\t'"'"' read -r idx cnf <<< "{}"; run_one "$idx" "$cnf"'

# Aggregate cells -> results.csv (sorted for stable diff)
echo "instance,result,time_s,timeout,exit_code" > "$OUT/results.csv"
cat "$OUT"/cells/*.csv | sort >> "$OUT/results.csv"

rm -rf "$SCRATCH"

# Summary
echo "=== kissat-full summary ($OUT) ==="
awk -F, 'NR>1{c[$2]++; tot++} END{for(k in c) printf "%-8s %d\n", k, c[k]; printf "solved   %d/%d\n", c["SAT"]+c["UNSAT"], tot}' "$OUT/results.csv"
touch "$OUT/DONE"
echo "DONE -> $OUT"
