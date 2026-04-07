#!/usr/bin/env bash
# smoke_test.sh — Quick correctness check for a solver iteration
#
# Usage: bash tools/smoke_test.sh solver/NN-name
#
# Runs all tests in tests/cnf/sat/ and tests/cnf/unsat/ against the solver.
# Checks:
#   - SAT instances: correct s-line + assignment satisfies every clause
#   - UNSAT instances: correct s-line + proof.out exists (and runs checker if available)
#
# All outputs are logged to log/<timestamp>/ for debugging.

set -euo pipefail

SOLVER_REL="${1:?Usage: bash tools/smoke_test.sh solver/NN-name}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOLVER_DIR="$(cd "$REPO_ROOT/$SOLVER_REL" 2>/dev/null && pwd)" || {
    echo "ERROR: solver directory not found: $SOLVER_REL" >&2
    exit 1
}
RUN_SH="$SOLVER_DIR/run.sh"

if [[ ! -f "$RUN_SH" ]]; then
    echo "ERROR: $RUN_SH not found" >&2
    exit 1
fi

# --- set up log directory ---
TIMESTAMP=$(date +%Y-%m-%d-%H-%M-%S)
LOG_DIR="$REPO_ROOT/log/$TIMESTAMP"
mkdir -p "$LOG_DIR"

PASS=0
FAIL=0
TOTAL=0

# --- helpers ---

log() {
    echo "$@" | tee -a "$LOG_DIR/summary.log"
}

run_one_test() {
    local cnf="$1"
    local expected="$2"  # SATISFIABLE or UNSATISFIABLE
    local name
    name=$(basename "$cnf" .cnf)
    TOTAL=$((TOTAL + 1))

    local test_dir="$LOG_DIR/$name"
    mkdir -p "$test_dir"

    # Create a proof output dir for this test
    local proof_dir="$test_dir/proof"
    mkdir -p "$proof_dir"

    # Run solver, capture stdout and stderr
    local output=""
    local exit_code=0
    output=$(bash "$RUN_SH" "$cnf" "$proof_dir" 2>"$test_dir/stderr.log") || exit_code=$?
    echo "$output" > "$test_dir/stdout.log"
    echo "$exit_code" > "$test_dir/exit_code.log"

    # Extract s-line
    local s_line
    s_line=$(echo "$output" | grep '^s ' | head -1) || true

    if [[ -z "$s_line" ]]; then
        log "  FAIL  $name — no 's' line in output"
        echo "REASON: no s-line found in stdout" > "$test_dir/failure.log"
        FAIL=$((FAIL + 1))
        return
    fi

    if [[ "$s_line" != "s $expected" ]]; then
        log "  FAIL  $name — expected 's $expected', got '$s_line'"
        echo "REASON: wrong s-line: expected 's $expected', got '$s_line'" > "$test_dir/failure.log"
        FAIL=$((FAIL + 1))
        return
    fi

    # --- SAT-specific checks: verify assignment ---
    if [[ "$expected" == "SATISFIABLE" ]]; then
        local verify_result
        verify_result=$(verify_assignment "$cnf" "$output" 2>&1) || {
            log "  FAIL  $name — $verify_result"
            echo "REASON: $verify_result" > "$test_dir/failure.log"
            FAIL=$((FAIL + 1))
            return
        }
    fi

    # --- UNSAT-specific checks: proof.out must exist ---
    if [[ "$expected" == "UNSATISFIABLE" ]]; then
        if [[ ! -f "$proof_dir/proof.out" ]]; then
            log "  FAIL  $name — no proof.out generated"
            echo "REASON: proof.out not found in $proof_dir" > "$test_dir/failure.log"
            FAIL=$((FAIL + 1))
            return
        fi

        # Log proof file info
        ls -la "$proof_dir/proof.out" > "$test_dir/proof_info.log" 2>&1

        # Run drat-trim if available
        if command -v drat-trim &>/dev/null; then
            local checker_output
            checker_output=$(drat-trim "$cnf" "$proof_dir/proof.out" 2>&1) || {
                log "  FAIL  $name — drat-trim rejected proof"
                echo "$checker_output" > "$test_dir/checker.log"
                echo "REASON: drat-trim rejected proof" > "$test_dir/failure.log"
                FAIL=$((FAIL + 1))
                return
            }
            echo "$checker_output" > "$test_dir/checker.log"
            if ! echo "$checker_output" | grep -qi "VERIFIED\|ACCEPTED"; then
                log "  FAIL  $name — drat-trim did not verify proof"
                echo "REASON: drat-trim output did not contain VERIFIED" > "$test_dir/failure.log"
                FAIL=$((FAIL + 1))
                return
            fi
            log "  PASS  $name (proof verified by drat-trim)"
        else
            log "  PASS  $name (proof.out exists, no checker available)"
        fi

        PASS=$((PASS + 1))
        return
    fi

    log "  PASS  $name"
    PASS=$((PASS + 1))
}

verify_assignment() {
    local cnf="$1"
    local output="$2"

    # Extract assigned literals from v lines
    local -a lits=()
    while IFS= read -r line; do
        for tok in $line; do
            [[ "$tok" == "v" ]] && continue
            [[ "$tok" == "0" ]] && continue
            lits+=("$tok")
        done
    done < <(echo "$output" | grep '^v ')

    if [[ ${#lits[@]} -eq 0 ]]; then
        echo "no v-lines found for SAT result"
        return 1
    fi

    # Check for contradictions (both x and -x assigned)
    declare -A assigned
    for lit in "${lits[@]}"; do
        local var="${lit#-}"
        if [[ -n "${assigned[$var]+x}" ]]; then
            local prev="${assigned[$var]}"
            if [[ "$prev" != "$lit" ]]; then
                echo "contradictory assignment: both $prev and $lit"
                return 1
            fi
        fi
        assigned[$var]="$lit"
    done

    # Build set of true literals for fast lookup
    declare -A true_lits
    for lit in "${lits[@]}"; do
        true_lits[$lit]=1
    done

    # Check each clause in the CNF
    local clause_num=0
    while IFS= read -r line; do
        # Skip comments, header, empty lines
        [[ "$line" =~ ^[[:space:]]*$ ]] && continue
        [[ "$line" =~ ^c ]] && continue
        [[ "$line" =~ ^p ]] && continue

        clause_num=$((clause_num + 1))
        local clause_sat=0
        local clause_lits=""
        for lit in $line; do
            [[ "$lit" == "0" ]] && break
            clause_lits+="$lit "
            if [[ -n "${true_lits[$lit]+x}" ]]; then
                clause_sat=1
            fi
        done

        if [[ $clause_sat -eq 0 && -n "$clause_lits" ]]; then
            echo "clause $clause_num not satisfied: ($clause_lits)"
            return 1
        fi
    done < "$cnf"

    return 0
}

# --- main ---

log "=== Smoke test: $(basename "$SOLVER_DIR") ==="
log "    Date: $(date)"
log "    Log:  $LOG_DIR"
log ""

# Build first
log "Building..."
if ! (cd "$SOLVER_DIR" && bash build.sh) > "$LOG_DIR/build.log" 2>&1; then
    log "ERROR: build.sh failed (see $LOG_DIR/build.log)"
    exit 1
fi
log "Build OK"
log ""

# Run SAT tests
log "--- SAT instances ---"
for cnf in "$REPO_ROOT"/tests/cnf/sat/*.cnf; do
    [[ -f "$cnf" ]] || continue
    run_one_test "$cnf" "SATISFIABLE"
done

log ""

# Run UNSAT tests
log "--- UNSAT instances ---"
for cnf in "$REPO_ROOT"/tests/cnf/unsat/*.cnf; do
    [[ -f "$cnf" ]] || continue
    run_one_test "$cnf" "UNSATISFIABLE"
done

log ""
log "=== Results: $PASS passed, $FAIL failed, $TOTAL total ==="
log "    Log directory: $LOG_DIR"

if [[ $FAIL -gt 0 ]]; then
    exit 1
fi
