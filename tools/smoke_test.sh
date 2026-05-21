#!/usr/bin/env bash
# smoke_test.sh — Quick correctness check for a solver iteration
#
# Usage: bash tools/smoke_test.sh solver/NN-name
#
# Runs all tests in tests/cnf/sat/ and tests/cnf/unsat/ against the solver.
# Checks:
#   - result.json status contract is present and matches the expected status
#   - SAT instances: correct s-line + assignment satisfies every clause + model.txt exists
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
SMOKE_TIMEOUT="${SAT_SMOKE_TIMEOUT:-600}"

if [[ ! -f "$RUN_SH" ]]; then
    echo "ERROR: $RUN_SH not found" >&2
    exit 1
fi

# --- locate timeout command ---
TIMEOUT_CMD=""
if command -v gtimeout &>/dev/null; then
    TIMEOUT_CMD="gtimeout"
elif command -v timeout &>/dev/null; then
    TIMEOUT_CMD="timeout"
else
    echo "ERROR: neither 'timeout' nor 'gtimeout' found" >&2
    exit 1
fi

# --- locate drat-trim proof checker ---
DRAT_TRIM=""
if [[ -x "$REPO_ROOT/tools/checkers/drat-trim/drat-trim" ]]; then
    DRAT_TRIM="$REPO_ROOT/tools/checkers/drat-trim/drat-trim"
elif command -v drat-trim &>/dev/null; then
    DRAT_TRIM="drat-trim"
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

    # Run solver, capture stdout and stderr
    local output=""
    local exit_code=0
    output=$("$TIMEOUT_CMD" "$SMOKE_TIMEOUT" bash "$RUN_SH" "$cnf" "$test_dir" 2>"$test_dir/stderr.log") || exit_code=$?
    echo "$output" > "$test_dir/stdout.log"
    echo "$exit_code" > "$test_dir/exit_code.log"

    if [[ $exit_code -eq 124 ]]; then
        log "  FAIL  $name — timed out after ${SMOKE_TIMEOUT}s"
        echo "REASON: solver timed out after ${SMOKE_TIMEOUT}s" > "$test_dir/failure.log"
        FAIL=$((FAIL + 1))
        return
    fi

    local result_json="$test_dir/result.json"
    if [[ ! -f "$result_json" ]]; then
        log "  FAIL  $name — missing result.json"
        echo "REASON: result.json not found in $test_dir" > "$test_dir/failure.log"
        FAIL=$((FAIL + 1))
        return
    fi

    local result_status status_file
    read -r result_status status_file < <(python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); print(p["status"], p["status_file"])' "$result_json" 2>"$test_dir/result_parse.err") || {
        log "  FAIL  $name — malformed result.json"
        echo "REASON: could not parse result.json" > "$test_dir/failure.log"
        FAIL=$((FAIL + 1))
        return
    }
    if [[ ! -f "$status_file" ]]; then
        log "  FAIL  $name — result.json points at missing status file"
        echo "REASON: status file not found: $status_file" > "$test_dir/failure.log"
        FAIL=$((FAIL + 1))
        return
    fi
    local status_file_text
    status_file_text=$(tr -d '\r\n' < "$status_file")
    if [[ "$status_file_text" != "$result_status" ]]; then
        log "  FAIL  $name — status file disagrees with result.json"
        echo "REASON: status file says $status_file_text, result.json says $result_status" > "$test_dir/failure.log"
        FAIL=$((FAIL + 1))
        return
    fi

    local expected_status
    case "$expected" in
        SATISFIABLE) expected_status="SAT" ;;
        UNSATISFIABLE) expected_status="UNSAT" ;;
        UNKNOWN) expected_status="UNKNOWN" ;;
        *) expected_status="$expected" ;;
    esac

    if [[ "$result_status" != "$expected_status" ]]; then
        log "  FAIL  $name — expected result status '$expected_status', got '$result_status'"
        echo "REASON: wrong result.json status: expected $expected_status, got $result_status" > "$test_dir/failure.log"
        FAIL=$((FAIL + 1))
        return
    fi

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
        if [[ ! -f "$test_dir/model.txt" ]]; then
            log "  FAIL  $name — SAT result missing model.txt"
            echo "REASON: model.txt not found in $test_dir" > "$test_dir/failure.log"
            FAIL=$((FAIL + 1))
            return
        fi
        local verify_result
        verify_result=$(verify_assignment "$cnf" "$output" 2>&1) || {
            log "  FAIL  $name — $verify_result"
            echo "REASON: $verify_result" > "$test_dir/failure.log"
            FAIL=$((FAIL + 1))
            return
        }
    fi

    # --- UNSAT-specific checks: proof.out + drat-trim verification ---
    if [[ "$expected" == "UNSATISFIABLE" ]]; then
        if [[ ! -f "$test_dir/proof.out" ]]; then
            log "  FAIL  $name — no proof.out generated"
            echo "REASON: proof.out not found in $test_dir" > "$test_dir/failure.log"
            FAIL=$((FAIL + 1))
            return
        fi

        # Log proof file info
        ls -la "$test_dir/proof.out" > "$test_dir/proof_info.log" 2>&1

        # Run drat-trim
        if [[ -n "$DRAT_TRIM" ]]; then
            local checker_output=""
            local checker_exit=0
            local checker_status=""
            checker_output=$("$DRAT_TRIM" "$cnf" "$test_dir/proof.out" 2>&1) || checker_exit=$?
            echo "$checker_output" > "$test_dir/checker.log"
            echo "$checker_exit" > "$test_dir/checker_exit.log"
            checker_status=$(printf '%s\n' "$checker_output" | tr -d '\r')

            if echo "$checker_status" | grep -qx "s VERIFIED"; then
                log "  PASS  $name (proof verified by drat-trim)"
            elif echo "$checker_status" | grep -qx "s ACCEPTED"; then
                log "  PASS  $name (proof accepted by drat-trim)"
            else
                log "  FAIL  $name — drat-trim rejected proof"
                echo "REASON: drat-trim rejected proof (exit=$checker_exit)" > "$test_dir/failure.log"
                FAIL=$((FAIL + 1))
                return
            fi
        else
            log "  WARN  $name — proof.out exists but no drat-trim (run tools/setup_checkers.sh)"
            log "  PASS  $name (proof.out exists, skipped verification)"
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
log "    Timeout: ${SMOKE_TIMEOUT}s"
if [[ -n "$DRAT_TRIM" ]]; then
    log "    Proof checker: $DRAT_TRIM"
else
    log "    Proof checker: NONE (run tools/setup_checkers.sh to install drat-trim)"
fi
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
