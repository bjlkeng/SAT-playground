#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

SOLVER10="solver/10-bve-preprocess"
SOLVER11="solver/11-kissat-port"
LOG_DIR="log/0.0b"
INSTANCE_DIR="$LOG_DIR/instances"
STDOUT_DIR="$LOG_DIR/lbd-on-stdout"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

rm -rf "$LOG_DIR"
mkdir -p "$INSTANCE_DIR" "$STDOUT_DIR"

link_instance() {
    local source="$1"
    local name="$2"
    ln -sfn "$REPO_ROOT/$source" "$INSTANCE_DIR/$name"
}

link_instance "tests/cnf/sat/three_sat.cnf" "01-small-sat.cnf"
link_instance "tests/cnf/unsat/pigeonhole_3_2.cnf" "02-small-unsat.cnf"
link_instance "benchmarks/profiling/feistel_b64_k57_r18.cnf" "03-medium-sat.cnf"
link_instance "benchmarks/profiling/minisat-simp-five/46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized.cnf.xz" "04-k4-like.cnf.xz"
link_instance "benchmarks/profiling/minisat-simp-five/1d18837c0ced5c18a3a4693993e61728-SC25_Timetable_C_392_E_45_Cl_25_D_7_T_50.normalised.cnf.xz" "05-timetable-like.cnf.xz"

echo "=== 0.0b thin-slice: solver10 baseline ==="
bash tools/bench.sh -t 120 -m 16384 \
    -d "$INSTANCE_DIR" "$SOLVER10" \
    --log-dir "$LOG_DIR/solver10"

echo "=== 0.0b thin-slice: solver11 SAT_USE_LBD=off ==="
SAT_USE_LBD=off bash tools/bench.sh -t 120 -m 16384 \
    -d "$INSTANCE_DIR" "$SOLVER11" \
    --log-dir "$LOG_DIR/solver11-lbd-off"

echo "=== 0.0b thin-slice: solver11 SAT_USE_LBD=on ==="
SAT_USE_LBD=on bash tools/bench.sh -t 120 -m 16384 \
    -d "$INSTANCE_DIR" "$SOLVER11" \
    --log-dir "$LOG_DIR/solver11-lbd-on"

python3 tools/status_compare.py \
    --before "$LOG_DIR/solver10/results.csv" \
    --after "$LOG_DIR/solver11-lbd-off/results.csv" \
    > "$LOG_DIR/status-compare-solver10-vs-lbd-off.txt"

python3 tools/status_compare.py \
    --before "$LOG_DIR/solver11-lbd-off/results.csv" \
    --after "$LOG_DIR/solver11-lbd-on/results.csv" \
    > "$LOG_DIR/status-compare-lbd-off-vs-on.txt"

python3 tools/compare_bench.py \
    --before "$LOG_DIR/solver11-lbd-off/results.csv" \
    --after "$LOG_DIR/solver11-lbd-on/results.csv" \
    --timeout 120 \
    > "$LOG_DIR/compare-lbd-off-vs-on.txt"

lbd_total=0
lbd_max=0
for cnf in "$INSTANCE_DIR"/*; do
    name="$(basename "$cnf")"
    name="${name%.xz}"
    name="${name%.gz}"
    name="${name%.cnf}"
    out_dir="$STDOUT_DIR/$name"
    mkdir -p "$out_dir"
    solver_input="$cnf"
    if [[ "$cnf" == *.cnf.xz ]]; then
        solver_input="$TMP_DIR/$name.cnf"
        xz -dkc "$cnf" > "$solver_input"
    elif [[ "$cnf" == *.cnf.gz ]]; then
        solver_input="$TMP_DIR/$name.cnf"
        gzip -dkc "$cnf" > "$solver_input"
    fi
    SAT_USE_LBD=on bash "$SOLVER11/run.sh" "$solver_input" "$out_dir" > "$out_dir/stdout.log" 2> "$out_dir/stderr.log"
    grep '^s ' "$out_dir/stdout.log" | head -1 > "$out_dir/stdout-status.txt"
    line="$(grep '^c lbd ' "$out_dir/stdout.log" | tail -1 || true)"
    computed="$(printf '%s\n' "$line" | sed -n 's/.*computed=\([0-9][0-9]*\).*/\1/p')"
    max_seen="$(printf '%s\n' "$line" | sed -n 's/.*max=\([0-9][0-9]*\).*/\1/p')"
    computed="${computed:-0}"
    max_seen="${max_seen:-0}"
    lbd_total=$((lbd_total + computed))
    if (( max_seen > lbd_max )); then
        lbd_max="$max_seen"
    fi
done

{
    echo "lbd_total_computed=$lbd_total"
    echo "lbd_max=$lbd_max"
} > "$LOG_DIR/lbd-counters.txt"

if (( lbd_total == 0 )); then
    echo "ERROR: SAT_USE_LBD=on produced zero LBD computations across the thin-slice set" >&2
    exit 1
fi

cat > "$LOG_DIR/findings.md" <<'FINDINGS'
# 0.0b Findings

Found and fixed a harness gap before the accepted run: the manual
SAT_USE_LBD=on stdout/counter pass originally called run.sh directly on
.cnf.xz instance symlinks. tools/bench.sh decompresses compressed instances
before invoking a solver, but run.sh intentionally receives a plain CNF path.
The harness now decompresses .cnf.xz/.cnf.gz inputs into a temporary CNF before
the direct solver pass, keeping the manual counter validation aligned with the
benchmark path.
FINDINGS

echo "Thin-slice artifacts written under $LOG_DIR"
