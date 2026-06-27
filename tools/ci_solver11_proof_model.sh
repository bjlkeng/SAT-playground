#!/usr/bin/env bash
# Proof/model contract gate for the current solver.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOLVER="$("$REPO_ROOT/tools/current_solver.sh" "${1:-}")"
SOLVER_DIR="$REPO_ROOT/$SOLVER"
OUT_ROOT="${SAT_CI_PROOF_MODEL_OUT:-$REPO_ROOT/log/ci-proof-model-$(basename "$SOLVER")-$(date +%Y-%m-%d-%H-%M-%S)}"
UNKNOWN_CNF="$SOLVER_DIR/testdata/golden/split_clause.cnf"
UNSAT_CNF="$SOLVER_DIR/testdata/golden/unsat_empty_clause.cnf"
if [[ ! -f "$UNKNOWN_CNF" ]]; then
    UNKNOWN_CNF="$REPO_ROOT/tests/cnf/sat/three_sat.cnf"
fi
if [[ ! -f "$UNSAT_CNF" ]]; then
    UNSAT_CNF="$REPO_ROOT/tests/cnf/unsat/contradiction.cnf"
fi

mkdir -p "$OUT_ROOT"

run_smoke() {
    local name="$1"
    shift
    echo "ci proof/model: smoke $name"
    env "$@" bash "$REPO_ROOT/tools/smoke_test.sh" "$SOLVER" \
        > "$OUT_ROOT/$name.log" 2>&1
}

run_smoke baseline_unsat SAT_PROFILE=baseline SAT_PROOF=drat
run_smoke default_unsat SAT_PROOF=drat
run_smoke default_search_strong SAT_PROOF=drat SAT_SEARCH_AXIS=strong
run_smoke fast_unsat SAT_PROFILE=fast SAT_PROOF=drat
run_smoke fast_preprocess_conservative SAT_PROFILE=fast SAT_PREPROCESS_AXIS=conservative SAT_PROOF=drat

echo "ci proof/model: SAT model smoke-plus"
bash "$REPO_ROOT/tools/bench.sh" -t 30 -m 16384 \
    -d "$REPO_ROOT/benchmarks/iteration/smoke-plus" \
    --log-dir "$OUT_ROOT/smoke-plus" "$SOLVER" \
    > "$OUT_ROOT/smoke-plus.log" 2>&1

echo "ci proof/model: UNKNOWN limit contract"
unknown_dir="$OUT_ROOT/unknown-limit"
mkdir -p "$unknown_dir"
SAT_PROFILE=baseline SAT_LIMIT_TICKS=0 SAT_STATS_JSON=on \
    bash "$REPO_ROOT/$SOLVER/run.sh" \
    "$UNKNOWN_CNF" \
    "$unknown_dir" > "$unknown_dir/stdout.log" 2> "$unknown_dir/stderr.log"
python3 "$REPO_ROOT/tools/validate_solver_result.py" \
    --cnf "$UNKNOWN_CNF" \
    --out-dir "$unknown_dir" --expected-status UNKNOWN --proof-policy drat \
    --require-json-stats on

if [[ -f "$SOLVER_DIR/testdata/golden/manifest.tsv" ]]; then
    echo "ci proof/model: generated proof/model golden tests"
    (cd "$SOLVER_DIR" && cargo test oracle_tests::output_contract::test_golden)
else
    echo "ci proof/model: no solver-local golden manifest; skipped generated golden tests"
fi

echo "ci proof/model: LRAT is expected unsupported until the proof-upgrade bead"
lrat_dir="$OUT_ROOT/lrat-unsupported"
mkdir -p "$lrat_dir"
set +e
SAT_PROOF=lrat bash "$REPO_ROOT/$SOLVER/run.sh" \
    "$UNSAT_CNF" \
    "$lrat_dir" > "$lrat_dir/stdout.log" 2> "$lrat_dir/stderr.log"
lrat_rc=$?
set -e
if [[ $lrat_rc -eq 0 ]]; then
    echo "ci proof/model: SAT_PROOF=lrat unexpectedly succeeded" >&2
    exit 1
fi

echo "ci proof/model: proof-checker unavailable path is covered by validate_solver_result.py error semantics"
echo "ci proof/model: artifacts=$OUT_ROOT"
