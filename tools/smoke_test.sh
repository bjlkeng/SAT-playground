#!/usr/bin/env bash
# smoke_test.sh — Quick correctness check for a solver iteration
#
# Usage: bash tools/smoke_test.sh solver/NN-name
#
# Runs all tests in tests/cnf/sat/ and tests/cnf/unsat/ against the solver,
# checking that it reports the correct result and (for SAT) a valid assignment.

set -euo pipefail

SOLVER_DIR="${1:?Usage: bash tools/smoke_test.sh solver/NN-name}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOLVER_DIR="$(cd "$REPO_ROOT/$SOLVER_DIR" 2>/dev/null && pwd)" || {
    echo "ERROR: solver directory not found: $1" >&2
    exit 1
}
RUN_SH="$SOLVER_DIR/run.sh"

if [[ ! -x "$RUN_SH" && ! -f "$RUN_SH" ]]; then
    echo "ERROR: $RUN_SH not found" >&2
    exit 1
fi

PASS=0
FAIL=0
ERRORS=""
PROOF_DIR=$(mktemp -d)
trap 'rm -rf "$PROOF_DIR"' EXIT

# --- helpers ---

run_solver() {
    local cnf="$1"
    bash "$RUN_SH" "$cnf" "$PROOF_DIR" 2>/dev/null
}

check_sat_result() {
    local cnf="$1"
    local expected="$2"  # SATISFIABLE or UNSATISFIABLE
    local output
    output=$(run_solver "$cnf") || true

    local s_line
    s_line=$(echo "$output" | grep '^s ' | head -1)

    if [[ -z "$s_line" ]]; then
        FAIL=$((FAIL + 1))
        ERRORS+="  FAIL $(basename "$cnf"): no 's' line in output\n"
        return
    fi

    if [[ "$s_line" != "s $expected" ]]; then
        FAIL=$((FAIL + 1))
        ERRORS+="  FAIL $(basename "$cnf"): expected 's $expected', got '$s_line'\n"
        return
    fi

    # For SAT results, verify the assignment satisfies the formula
    if [[ "$expected" == "SATISFIABLE" ]]; then
        if ! verify_assignment "$cnf" "$output"; then
            FAIL=$((FAIL + 1))
            ERRORS+="  FAIL $(basename "$cnf"): assignment does not satisfy formula\n"
            return
        fi
    fi

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
        # No v lines for a SAT result is a failure
        return 1
    fi

    # Build a set of true literals
    declare -A true_lits
    for lit in "${lits[@]}"; do
        true_lits[$lit]=1
    done

    # Check each clause: at least one literal must be in the assignment
    while IFS= read -r line; do
        # Skip comments and header
        [[ "$line" =~ ^c ]] && continue
        [[ "$line" =~ ^p ]] && continue
        [[ -z "$line" ]] && continue

        local clause_sat=0
        for lit in $line; do
            [[ "$lit" == "0" ]] && break
            if [[ -n "${true_lits[$lit]+x}" ]]; then
                clause_sat=1
                break
            fi
        done
        if [[ $clause_sat -eq 0 ]]; then
            return 1
        fi
    done < "$cnf"

    return 0
}

# --- main ---

echo "=== Smoke test: $(basename "$SOLVER_DIR") ==="
echo ""

# Build first
echo "Building..."
(cd "$SOLVER_DIR" && bash build.sh) >/dev/null 2>&1 || {
    echo "ERROR: build.sh failed" >&2
    exit 1
}
echo ""

# Run SAT tests
echo "--- SAT instances ---"
for cnf in "$REPO_ROOT"/tests/cnf/sat/*.cnf; do
    [[ -f "$cnf" ]] || continue
    printf "  %-30s " "$(basename "$cnf")"
    check_sat_result "$cnf" "SATISFIABLE"
    if [[ $((PASS + FAIL)) -eq $PASS ]]; then
        # Last test passed (PASS just incremented and FAIL didn't)
        echo "PASS"
    else
        echo "FAIL"
    fi
done

echo ""

# Run UNSAT tests
echo "--- UNSAT instances ---"
for cnf in "$REPO_ROOT"/tests/cnf/unsat/*.cnf; do
    [[ -f "$cnf" ]] || continue
    printf "  %-30s " "$(basename "$cnf")"
    local_pass=$PASS
    check_sat_result "$cnf" "UNSATISFIABLE"
    if [[ $PASS -gt $local_pass ]]; then
        echo "PASS"
    else
        echo "FAIL"
    fi
done

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="

if [[ $FAIL -gt 0 ]]; then
    echo ""
    echo "Failures:"
    echo -e "$ERRORS"
    exit 1
fi
