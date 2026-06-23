#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

SOLVER10="solver/10-bve-subsume"
SOLVER11="solver/11-kissat-search"
PROFILE_BENCH_TIMEOUT="${PROFILE_BENCH_TIMEOUT:-300}"
LOCK_DIR="log/baseline-lock"
SMOKE_VALIDATE_DIR="$LOCK_DIR/solver11-smoke-validation"
RAW_LOCK="$SOLVER11/BASELINE_LOCK.raw.txt"

rm -rf "$LOCK_DIR"
mkdir -p "$SMOKE_VALIDATE_DIR"

echo "=== Solver 11 baseline lock ==="
echo "repo=$REPO_ROOT"
echo "solver10=$SOLVER10"
echo "solver11=$SOLVER11"

echo "=== Unit tests and smoke ==="
(
    cd "$SOLVER11"
    cargo test
    cargo build --release
)
bash tools/smoke_test.sh "$SOLVER11"

run_and_validate() {
    local cnf="$1"
    local expected="$2"
    local name
    name="$(basename "$cnf" .cnf)"

    local out_dir="$SMOKE_VALIDATE_DIR/$name"
    rm -rf "$out_dir"
    mkdir -p "$out_dir"

    local exit_code=0
    bash "$SOLVER11/run.sh" "$cnf" "$out_dir" > "$out_dir/stdout.log" 2> "$out_dir/stderr.log" || exit_code=$?
    echo "$exit_code" > "$out_dir/exit_code.log"
    grep '^s ' "$out_dir/stdout.log" | head -1 > "$out_dir/stdout-status.txt" || true

    local proof_policy="off"
    if [[ "$expected" == "UNSAT" ]]; then
        proof_policy="drat"
    fi

    python3 tools/validate_solver_result.py \
        --cnf "$cnf" \
        --out-dir "$out_dir" \
        --expected-status "$expected" \
        --proof-policy "$proof_policy" \
        --require-json-stats off
}

echo "=== Validator pass over smoke instances ==="
for cnf in tests/cnf/sat/*.cnf; do
    run_and_validate "$cnf" SAT
done
for cnf in tests/cnf/unsat/*.cnf; do
    run_and_validate "$cnf" UNSAT
done

echo "=== Profiling baseline comparison ==="
bash tools/bench.sh -t "$PROFILE_BENCH_TIMEOUT" -m 16384 \
    -d benchmarks/profiling "$SOLVER10" \
    --log-dir "$LOCK_DIR/solver10"

bash tools/bench.sh -t "$PROFILE_BENCH_TIMEOUT" -m 16384 \
    -d benchmarks/profiling "$SOLVER11" \
    --log-dir "$LOCK_DIR/solver11"

python3 tools/status_compare.py \
    --before "$LOCK_DIR/solver10/results.csv" \
    --after "$LOCK_DIR/solver11/results.csv" \
    > "$RAW_LOCK"

{
    echo "solver10_dir=$SOLVER10"
    echo "solver11_dir=$SOLVER11"
    echo "profile_bench_timeout=$PROFILE_BENCH_TIMEOUT"
    echo "date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "rustc=$(rustc --version 2>/dev/null || true)"
    echo "cargo=$(cargo --version 2>/dev/null || true)"
    echo "uname=$(uname -a)"
    echo "solver10_binary_sha256=$(sha256sum "$SOLVER10/target/release/sat-solver" 2>/dev/null | awk '{print $1}')"
    echo "solver11_binary_sha256=$(sha256sum "$SOLVER11/target/release/sat-solver" 2>/dev/null | awk '{print $1}')"
    echo "env_SAT=$(env | sort | grep '^SAT_' || true)"
    echo "solver10_log=$LOCK_DIR/solver10/results.csv"
    echo "solver11_log=$LOCK_DIR/solver11/results.csv"
    echo "solver11_smoke_validation_dir=$SMOKE_VALIDATE_DIR"
} >> "$RAW_LOCK"

echo "Baseline lock written to $RAW_LOCK"
