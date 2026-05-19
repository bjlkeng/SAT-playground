#!/usr/bin/env bash
# Fast local gate for solver/11-kissat-port.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOLVER_DIR="$REPO_ROOT/solver/11-kissat-port"
PLAN_PATH="$REPO_ROOT/plan/solver-11-plan.md"
BENCH_TIMEOUT="${SAT_CI_BENCH_TIMEOUT:-120}"
BENCH_MEM_MB="${SAT_CI_BENCH_MEM_MB:-16384}"

cd "$SOLVER_DIR"
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
python3 "$REPO_ROOT/tools/validate_solver11_plan.py" "$PLAN_PATH"

cd "$REPO_ROOT"
python3 -m py_compile \
    tools/status_compare.py \
    tools/validate_solver_result.py \
    tools/select_iter_bench.py \
    tools/compare_bench.py \
    tools/extract_hot_instances.py \
    tools/ci_solver11_overhead.py \
    tools/validate_solver11_plan.py

cargo run --manifest-path tools/sat-bench/Cargo.toml --release -- \
    validate-plan "$PLAN_PATH"

if command -v shellcheck >/dev/null 2>&1; then
    shellcheck tools/*.sh solver/11-kissat-port/*.sh
else
    echo "ci_solver11_fast: shellcheck unavailable; skipped"
fi

bash tools/smoke_test.sh solver/11-kissat-port
SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port
bash tools/bench.sh -t "$BENCH_TIMEOUT" -m "$BENCH_MEM_MB" \
    -d benchmarks/iteration/smoke-plus solver/11-kissat-port
bash tools/ci_reproducibility.sh solver/11-kissat-port
