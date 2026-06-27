#!/usr/bin/env bash
# Fast local gate for the current solver.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOLVER_REL="$("$REPO_ROOT/tools/current_solver.sh" "${1:-}")"
SOLVER_DIR="$REPO_ROOT/$SOLVER_REL"
PLAN_PATH="${SAT_CI_PLAN_PATH:-}"
BENCH_TIMEOUT="${SAT_CI_BENCH_TIMEOUT:-120}"
BENCH_MEM_MB="${SAT_CI_BENCH_MEM_MB:-16384}"

if [[ -z "$PLAN_PATH" && -f "$SOLVER_DIR/PLAN.md" ]]; then
    PLAN_PATH="$SOLVER_DIR/PLAN.md"
fi

cd "$SOLVER_DIR"
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
if [[ -n "$PLAN_PATH" ]]; then
    python3 "$REPO_ROOT/tools/validate_solver11_plan.py" "$PLAN_PATH"
else
    echo "ci fast: no SAT_CI_PLAN_PATH or solver-local PLAN.md; skipped plan validation"
fi

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
    --help >/dev/null

if [[ -n "$PLAN_PATH" ]]; then
    cargo run --manifest-path tools/sat-bench/Cargo.toml --release -- \
        validate-plan "$PLAN_PATH"
fi

if command -v shellcheck >/dev/null 2>&1; then
    shellcheck tools/*.sh "$SOLVER_REL"/*.sh
else
    echo "ci fast: shellcheck unavailable; skipped"
fi

bash tools/smoke_test.sh "$SOLVER_REL"
SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh "$SOLVER_REL"
bash tools/bench.sh -t "$BENCH_TIMEOUT" -m "$BENCH_MEM_MB" \
    -d benchmarks/iteration/smoke-plus "$SOLVER_REL"
bash tools/ci_reproducibility.sh "$SOLVER_REL"
