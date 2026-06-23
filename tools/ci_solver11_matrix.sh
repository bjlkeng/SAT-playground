#!/usr/bin/env bash
# Bounded solver 11 feature-interaction matrix.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOLVER="${1:-solver/11-kissat-search}"
TIMEOUT="${SAT_CI_MATRIX_TIMEOUT:-30}"
OUT_ROOT="${SAT_CI_MATRIX_OUT:-$REPO_ROOT/log/ci_solver11_matrix-$(date +%Y-%m-%d-%H-%M-%S)}"
INSTANCE="$REPO_ROOT/solver/11-kissat-search/testdata/golden/sat_tiny.cnf"

mkdir -p "$OUT_ROOT"
SUMMARY="$OUT_ROOT/summary.tsv"
printf 'row\texpected\tactual\tconfig_hash\tlog_dir\n' > "$SUMMARY"

run_row() {
    local name="$1"
    local expected="$2"
    local env_line="$3"
    local row_dir="$OUT_ROOT/$name"
    mkdir -p "$row_dir"

    set +e
    # shellcheck disable=SC2086 # env_line is a controlled matrix row of VAR=value tokens.
    env SAT_STATS_JSON=on $env_line timeout "$TIMEOUT" \
        bash "$REPO_ROOT/$SOLVER/run.sh" "$INSTANCE" "$row_dir" \
        > "$row_dir/stdout.log" 2> "$row_dir/stderr.log"
    local rc=$?
    set -e

    local actual="ERROR"
    local config_hash="unavailable"
    if [[ -f "$row_dir/result.json" ]]; then
        IFS=$'\t' read -r actual config_hash < <(
            python3 -c 'import json,sys
p=json.load(open(sys.argv[1]))
print(p.get("status","ERROR"), p.get("config_hash","unavailable"), sep="\t")' \
                "$row_dir/result.json"
        )
    elif [[ $rc -ne 0 && "$expected" == "unsupported" ]]; then
        actual="UNSUPPORTED"
    fi

    printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$expected" "$actual" "$config_hash" "$row_dir" \
        | tee -a "$SUMMARY"

    case "$expected:$actual" in
        pass:SAT|pass:UNSAT|pass:UNKNOWN) ;;
        unsupported:UNSUPPORTED|unsupported:ERROR) ;;
        *)
            echo "ci_solver11_matrix: unexpected result for $name expected=$expected actual=$actual" >&2
            return 1
            ;;
    esac
}

run_row baseline pass 'SAT_PROFILE=baseline'
run_row lbd_legacy pass 'SAT_USE_LBD=on SAT_REDUCE=legacy SAT_RESTART=legacy-luby'
run_row lbd_tiered unsupported 'SAT_USE_LBD=on SAT_REDUCE=lbd-tiered SAT_RESTART=legacy-luby'
run_row lbd_tiered_kissat_ema unsupported 'SAT_USE_LBD=on SAT_REDUCE=lbd-tiered SAT_RESTART=kissat-ema'
run_row binary_fast unsupported 'SAT_BINARY_FAST=on'
run_row inprocess_shell unsupported 'SAT_INPROCESS=on SAT_VIVIFY=off SAT_PROBE=off SAT_HBR=off'

echo "ci_solver11_matrix: summary=$SUMMARY"
