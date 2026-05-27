#!/usr/bin/env bash
# Trace identical-work default vs resolved conflict-analysis runs on velev.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../" && pwd)"
OUT_ROOT="$SCRIPT_DIR/traces/velev"
INSTANCE_XZ="$REPO_ROOT/benchmarks/profiling/6832fe907740af686fde98518067ea3f-velev-pipe-sat-1.0-b7.cnf.xz"
INSTANCE_CNF="$OUT_ROOT/velev-pipe-sat-1.0-b7.cnf"

mkdir -p "$OUT_ROOT"
xz -dkc "$INSTANCE_XZ" > "$INSTANCE_CNF"

run_case() {
    local name="$1"
    shift
    local dir="$OUT_ROOT/$name"
    rm -rf "$dir"
    mkdir -p "$dir/proof"
    (
        cd "$REPO_ROOT"
        export SAT_STATS_JSON=on
        export SAT_TRACE_SEARCH_INTERVAL=20000
        export SAT_LIMIT_WALL_SEC=120
        for kv in "$@"; do
            export "$kv"
        done
        /usr/bin/time -v \
            bash solver/11-kissat-port/run.sh "$INSTANCE_CNF" "$dir/proof" \
            > "$dir/stdout.log" 2> "$dir/stderr.log"
    ) || true
    grep '^c JSON_STATS ' "$dir/stderr.log" | sed 's/^c JSON_STATS //' > "$dir/stats.jsonl" || true
    {
        echo "name: $name"
        echo "env: SAT_STATS_JSON=on SAT_TRACE_SEARCH_INTERVAL=20000 SAT_LIMIT_WALL_SEC=120 $*"
        echo "instance: $INSTANCE_XZ"
        echo "rev: $(git -C "$REPO_ROOT" rev-parse HEAD)"
        echo "date: $(date -Iseconds)"
    } > "$dir/run_meta.txt"
}

run_case A_baseline
run_case B_resolved SAT_CONFLICT_ANALYSIS_MODE=resolved

echo "wrote traces to $OUT_ROOT"
