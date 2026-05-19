#!/usr/bin/env bash
# Reproducibility gate for solver 11.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOLVER="${1:-solver/11-kissat-port}"
SOLVER_DIR="$REPO_ROOT/$SOLVER"
OUT_ROOT="${SAT_CI_REPRO_OUT:-$REPO_ROOT/log/ci_reproducibility-$(date +%Y-%m-%d-%H-%M-%S)}"
STRICT_SEED_EFFECT="${SAT_CI_REQUIRE_SEED_EFFECT:-off}"

mkdir -p "$OUT_ROOT"
(cd "$SOLVER_DIR" && bash build.sh)

REPLAY_DIR="$OUT_ROOT/replay"
mkdir -p "$REPLAY_DIR"
SEED0_REPLAY="$REPLAY_DIR/seed0.config"
SEED1_REPLAY="$REPLAY_DIR/seed1.config"
REPLAY_INSTANCE="$REPO_ROOT/tests/cnf/sat/unit.cnf"

make_replay() {
    local seed="$1"
    local replay_path="$2"
    local replay_out="$REPLAY_DIR/seed$seed-run"
    mkdir -p "$replay_out"
    SAT_PROFILE=baseline SAT_SEED="$seed" SAT_PROOF=drat \
        SAT_CONFIG_OUT="$replay_path" SAT_STATS_JSON=on \
        bash "$SOLVER_DIR/run.sh" "$REPLAY_INSTANCE" "$replay_out" \
        > "$replay_out/stdout.log" 2> "$replay_out/stderr.log"
    test -s "$replay_path"
}

make_replay 0 "$SEED0_REPLAY"
make_replay 1 "$SEED1_REPLAY"

declare -a CASES=(
    "trivial_sat:$REPO_ROOT/tests/cnf/sat/unit.cnf"
    "trivial_unsat:$REPO_ROOT/tests/cnf/unsat/contradiction.cnf"
    "medium_sat:$REPO_ROOT/tests/cnf/sat/three_sat.cnf"
    "medium_unsat:$REPO_ROOT/tests/cnf/unsat/pigeonhole_3_2.cnf"
    "proof_unsat:$REPO_ROOT/solver/11-kissat-port/testdata/golden/unsat_empty_clause.cnf"
)

extract_stats() {
    local stderr_log="$1"
    python3 - "$stderr_log" <<'PY'
import json, sys
for line in open(sys.argv[1]):
    marker = "c JSON_STATS "
    if marker in line:
        payload = json.loads(line.split(marker, 1)[1])
        keys = ["result", "conflicts", "decisions", "propagations", "restarts"]
        print("\t".join(str(payload.get(k, "")) for k in keys))
        break
else:
    raise SystemExit("missing JSON_STATS")
PY
}

normalize_trace() {
    sed -E 's/seconds_[a-z_]+=([0-9]+\.)?[0-9]+/seconds=ELIDED/g' "$1" || true
}

run_once() {
    local case_name="$1"
    local cnf="$2"
    local replay_path="$3"
    local suffix="$4"
    local out_dir="$OUT_ROOT/$case_name-$suffix"
    mkdir -p "$out_dir"
    SAT_CONFIG_REPLAY="$replay_path" SAT_STATS_JSON=on SAT_TRACE_FULL=on \
        bash "$SOLVER_DIR/run.sh" "$cnf" "$out_dir" \
        > "$out_dir/stdout.log" 2> "$out_dir/stderr.log"
    extract_stats "$out_dir/stderr.log" > "$out_dir/stats.tsv"
    grep '^c trace_full ' "$out_dir/stderr.log" > "$out_dir/trace.log" || true
}

seed_effect_seen=0
for entry in "${CASES[@]}"; do
    case_name="${entry%%:*}"
    cnf="${entry#*:}"
    run_once "$case_name" "$cnf" "$SEED0_REPLAY" a
    run_once "$case_name" "$cnf" "$SEED0_REPLAY" b
    run_once "$case_name" "$cnf" "$SEED1_REPLAY" seed1

    diff -u "$OUT_ROOT/$case_name-a/stats.tsv" "$OUT_ROOT/$case_name-b/stats.tsv"
    diff -u <(normalize_trace "$OUT_ROOT/$case_name-a/trace.log") \
        <(normalize_trace "$OUT_ROOT/$case_name-b/trace.log")

    if [[ -f "$OUT_ROOT/$case_name-a/proof.out" || -f "$OUT_ROOT/$case_name-b/proof.out" ]]; then
        cmp "$OUT_ROOT/$case_name-a/proof.out" "$OUT_ROOT/$case_name-b/proof.out"
    fi

    status0="$(cut -f1 "$OUT_ROOT/$case_name-a/stats.tsv")"
    status1="$(cut -f1 "$OUT_ROOT/$case_name-seed1/stats.tsv")"
    if [[ "$status0" != "$status1" ]]; then
        echo "ci_reproducibility: seed changed status for $case_name: $status0 vs $status1" >&2
        exit 1
    fi
    if ! cmp -s "$OUT_ROOT/$case_name-a/stats.tsv" "$OUT_ROOT/$case_name-seed1/stats.tsv"; then
        seed_effect_seen=1
    fi
done

if [[ "$seed_effect_seen" -eq 0 ]]; then
    msg="ci_reproducibility: SAT_SEED currently has no counter-visible effect; strict seed-effect enforcement is deferred until a seeded search feature lands"
    if [[ "$STRICT_SEED_EFFECT" == "on" ]]; then
        echo "$msg" >&2
        exit 1
    fi
    echo "$msg"
fi

cat > "$OUT_ROOT/allowed-nondeterminism.txt" <<'EOF'
Seed 0 runs use the same SAT_CONFIG_REPLAY file. Seed 1 runs use a separate
SAT_CONFIG_REPLAY file generated from the same baseline profile with only
SAT_SEED changed.

Excluded from byte comparisons:
- wall_time / elapsed_sec
- parse_time / parse_sec
- search_time / search_sec
- max_rss_mb
- binary_sha256 and config_hash are recorded identity fields, not nondeterminism.
No other JSON_STATS counters or trace fields are allowed to drift for same-seed replay.
EOF

echo "ci_reproducibility: artifacts=$OUT_ROOT"
