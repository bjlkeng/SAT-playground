#!/usr/bin/env bash
# Capture reproducible profiling artifacts for one solver 11 instance.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOLVER_REL="solver/11-kissat-search"
TIMEOUT_SEC=300
MEMLIMIT_MB=16384
RUN_LABEL=""
SAT_PROFILE_ARG=""
SAT_CONFIG_REPLAY_ARG=""
PERF_RECORD_MODE="auto"
BUILD=1
CNF_PATH=""

usage() {
    cat <<'USAGE'
Usage: bash tools/profile_solver11.sh [options] <instance.cnf[.gz|.xz]>

Options:
  --solver <dir>                 Solver directory (default: solver/11-kissat-search)
  -t, --timeout <seconds>        Timeout for each solver run (default: 300)
  -m, --memory <MB>              Virtual-memory limit for solver process (default: 16384)
  --profile <name>               Set SAT_PROFILE for this run
  --config-replay <path>         Set SAT_CONFIG_REPLAY for this run
  --label <text>                 Human run label recorded in artifacts
  --perf-record auto|on|off      Whether to attempt perf record (default: auto)
  --no-build                     Do not run build.sh before profiling
  -h, --help                     Show this help

The script always enables SAT_STATS_JSON=on so stats.jsonl can be captured. It
does not otherwise change SAT_* behavior unless the matching option is provided.
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --solver) SOLVER_REL="$2"; shift 2 ;;
        -t|--timeout) TIMEOUT_SEC="$2"; shift 2 ;;
        -m|--memory) MEMLIMIT_MB="$2"; shift 2 ;;
        --profile) SAT_PROFILE_ARG="$2"; shift 2 ;;
        --config-replay) SAT_CONFIG_REPLAY_ARG="$2"; shift 2 ;;
        --label) RUN_LABEL="$2"; shift 2 ;;
        --perf-record) PERF_RECORD_MODE="$2"; shift 2 ;;
        --no-build) BUILD=0; shift ;;
        -h|--help) usage; exit 0 ;;
        -*) echo "unknown option: $1" >&2; exit 2 ;;
        *)
            if [[ -n "$CNF_PATH" ]]; then
                echo "unexpected extra argument: $1" >&2
                exit 2
            fi
            CNF_PATH="$1"
            shift
            ;;
    esac
done

if [[ -z "$CNF_PATH" ]]; then
    usage >&2
    exit 2
fi

case "$PERF_RECORD_MODE" in
    auto|on|off) ;;
    *) echo "--perf-record must be auto, on, or off" >&2; exit 2 ;;
esac

if [[ "$CNF_PATH" != /* ]]; then
    CNF_PATH="$REPO_ROOT/$CNF_PATH"
fi
if [[ ! -f "$CNF_PATH" ]]; then
    echo "instance not found: $CNF_PATH" >&2
    exit 2
fi

SOLVER_DIR="$REPO_ROOT/$SOLVER_REL"
RUN_SH="$SOLVER_DIR/run.sh"
BUILD_SH="$SOLVER_DIR/build.sh"
BINARY="$SOLVER_DIR/target/release/sat-solver"
if [[ ! -f "$RUN_SH" || ! -f "$BUILD_SH" ]]; then
    echo "invalid solver directory: $SOLVER_REL" >&2
    exit 2
fi

if [[ -f "$HOME/.cargo/env" ]]; then
    source "$HOME/.cargo/env"
fi

TIMEOUT_CMD=""
if command -v gtimeout >/dev/null 2>&1; then
    TIMEOUT_CMD="gtimeout"
elif command -v timeout >/dev/null 2>&1; then
    TIMEOUT_CMD="timeout"
else
    echo "timeout command not found" >&2
    exit 2
fi

if [[ $BUILD -eq 1 ]]; then
    (cd "$SOLVER_DIR" && bash "$BUILD_SH")
fi
if [[ ! -x "$BINARY" ]]; then
    echo "solver binary not found after build: $BINARY" >&2
    exit 2
fi

PROFILE_ROOT="$REPO_ROOT/log/profiles"
mkdir -p "$PROFILE_ROOT"
INSTANCE_BASE="$(basename "$CNF_PATH")"
INSTANCE_BASE="${INSTANCE_BASE%.cnf.xz}"
INSTANCE_BASE="${INSTANCE_BASE%.cnf.gz}"
INSTANCE_BASE="${INSTANCE_BASE%.cnf}"
SAFE_INSTANCE="$(printf '%s' "$INSTANCE_BASE" | tr -c 'A-Za-z0-9_.-' '_')"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
PENDING_DIR="$PROFILE_ROOT/${STAMP}-${SAFE_INSTANCE}-pending-$$"
mkdir -p "$PENDING_DIR"

MEMLIMIT_KB=$((MEMLIMIT_MB * 1024))
declare -a RUN_ENV=(SAT_STATS_JSON=on)
if [[ -n "$SAT_PROFILE_ARG" ]]; then
    RUN_ENV+=("SAT_PROFILE=$SAT_PROFILE_ARG")
fi
if [[ -n "$SAT_CONFIG_REPLAY_ARG" ]]; then
    replay_path="$SAT_CONFIG_REPLAY_ARG"
    [[ "$replay_path" != /* ]] && replay_path="$REPO_ROOT/$replay_path"
    RUN_ENV+=("SAT_CONFIG_REPLAY=$replay_path")
fi

quote_cmd() {
    printf '%q ' "$@"
    printf '\n'
}

run_solver_once() {
    local out_dir="$1"
    local stdout_file="$2"
    local stderr_file="$3"
    local time_file="$4"
    shift 4
    local -a prefix=("$@")
    rm -rf "$out_dir"
    mkdir -p "$out_dir"
    set +e
    if /usr/bin/time -v -o "$time_file" true >/dev/null 2>&1; then
        (
            ulimit -v "$MEMLIMIT_KB" 2>/dev/null || true
            env "${RUN_ENV[@]}" /usr/bin/time -v -o "$time_file" "${prefix[@]}" "$TIMEOUT_CMD" "$TIMEOUT_SEC" bash "$RUN_SH" "$CNF_PATH" "$out_dir"
        ) >"$stdout_file" 2>"$stderr_file"
    else
        echo "GNU /usr/bin/time -v unavailable; wall time only" > "$time_file"
        local start_ns end_ns
        start_ns="$(date +%s%N)"
        (
            ulimit -v "$MEMLIMIT_KB" 2>/dev/null || true
            env "${RUN_ENV[@]}" "${prefix[@]}" "$TIMEOUT_CMD" "$TIMEOUT_SEC" bash "$RUN_SH" "$CNF_PATH" "$out_dir"
        ) >"$stdout_file" 2>"$stderr_file"
        end_ns="$(date +%s%N)"
        awk -v start="$start_ns" -v end="$end_ns" 'BEGIN { printf "Elapsed (wall clock) seconds: %.3f\n", (end - start) / 1000000000 }' >> "$time_file"
    fi
    local status=$?
    set -e
    return "$status"
}

{
    echo "label=${RUN_LABEL:-NA}"
    echo "repo_root=$REPO_ROOT"
    echo "solver=$SOLVER_REL"
    echo "cnf=$CNF_PATH"
    echo "timeout=$TIMEOUT_SEC"
    echo "memory_mb=$MEMLIMIT_MB"
    echo "run_env=$(quote_cmd "${RUN_ENV[@]}")"
    echo "primary_command=$(quote_cmd env "${RUN_ENV[@]}" "$TIMEOUT_CMD" "$TIMEOUT_SEC" bash "$RUN_SH" "$CNF_PATH" "$PENDING_DIR/out")"
} > "$PENDING_DIR/command.txt"

{
    echo "date_utc=$STAMP"
    echo "uname=$(uname -a)"
    echo "solver_binary=$BINARY"
    echo "binary_sha256=$(sha256sum "$BINARY" | awk '{print $1}')"
    echo "SAT_SEED=${SAT_SEED:-unset}"
    echo "SAT_PROFILE=${SAT_PROFILE_ARG:-${SAT_PROFILE:-unset}}"
    echo "SAT_CONFIG_REPLAY=${SAT_CONFIG_REPLAY_ARG:-${SAT_CONFIG_REPLAY:-unset}}"
    env | sort | grep '^SAT_' || true
} > "$PENDING_DIR/env.txt"

PRIMARY_STATUS=0
run_solver_once "$PENDING_DIR/out" "$PENDING_DIR/stdout.log" "$PENDING_DIR/stderr.log" "$PENDING_DIR/time.txt" || PRIMARY_STATUS=$?

grep '^c JSON_STATS ' "$PENDING_DIR/stderr.log" | sed 's/^c JSON_STATS //' > "$PENDING_DIR/stats.jsonl" || true
if [[ ! -s "$PENDING_DIR/stats.jsonl" ]]; then
    echo "JSON_STATS not captured; check stderr.log" >> "$PENDING_DIR/notes.md"
fi

CONFIG_HASH="$(
    python3 - "$PENDING_DIR/stats.jsonl" <<'PY'
import json, sys
path = sys.argv[1]
value = "unknown"
try:
    with open(path) as handle:
        for raw in handle:
            if raw.strip():
                value = json.loads(raw).get("config_hash") or "unknown"
except Exception:
    value = "unknown"
print("".join(ch if ch.isalnum() or ch in "._-" else "_" for ch in value))
PY
)"

if command -v perf >/dev/null 2>&1; then
    PERF_STAT_STATUS=0
    run_solver_once "$PENDING_DIR/out-perf-stat" "$PENDING_DIR/perf-stat-stdout.log" "$PENDING_DIR/perf-stat-stderr.log" "$PENDING_DIR/perf-stat-time.txt" perf stat -o "$PENDING_DIR/perf-stat.txt" || PERF_STAT_STATUS=$?
    if [[ $PERF_STAT_STATUS -ne 0 ]]; then
        {
            echo
            echo "perf stat unavailable or failed; exit_status=$PERF_STAT_STATUS"
            echo "See perf-stat-stderr.log for details."
        } >> "$PENDING_DIR/perf-stat.txt"
    elif [[ ! -s "$PENDING_DIR/perf-stat.txt" ]]; then
        echo "perf stat produced no output despite exit_status=0" > "$PENDING_DIR/perf-stat.txt"
    fi
    if [[ "$PERF_RECORD_MODE" != "off" ]]; then
        PERF_RECORD_STATUS=0
        run_solver_once "$PENDING_DIR/out-perf-record" "$PENDING_DIR/perf-record-stdout.log" "$PENDING_DIR/perf-record-stderr.log" "$PENDING_DIR/perf-record-time.txt" perf record -o "$PENDING_DIR/perf.data" --call-graph dwarf -- || PERF_RECORD_STATUS=$?
        {
            echo "perf_record_exit_status=$PERF_RECORD_STATUS"
            if [[ -s "$PENDING_DIR/perf.data" ]]; then
                echo "perf_data=$PENDING_DIR/perf.data"
            else
                echo "perf record produced no perf.data; see perf-record-stderr.log"
            fi
        } > "$PENDING_DIR/perf-record.txt"
    else
        echo "perf record disabled by --perf-record=off" > "$PENDING_DIR/unavailable.txt"
    fi
else
    echo "perf not found; perf stat and perf record skipped" > "$PENDING_DIR/perf-stat.txt"
    echo "perf not found; perf record skipped" > "$PENDING_DIR/unavailable.txt"
fi

FINAL_DIR_BASE="$PROFILE_ROOT/${STAMP}-${SAFE_INSTANCE}-${CONFIG_HASH}"
FINAL_DIR="$FINAL_DIR_BASE"
suffix=1
while [[ -e "$FINAL_DIR" && "$FINAL_DIR" != "$PENDING_DIR" ]]; do
    FINAL_DIR="${FINAL_DIR_BASE}-${suffix}"
    suffix=$((suffix + 1))
done
if [[ "$FINAL_DIR" != "$PENDING_DIR" ]]; then
    mv "$PENDING_DIR" "$FINAL_DIR"
fi

{
    echo "# Solver 11 Profile"
    echo
    echo "- instance: \`$CNF_PATH\`"
    echo "- solver: \`$SOLVER_REL\`"
    echo "- artifact_dir: \`$FINAL_DIR\`"
    echo "- primary_exit_status: \`$PRIMARY_STATUS\`"
    echo "- config_hash: \`$CONFIG_HASH\`"
    echo "- binary_sha256: \`$(sha256sum "$BINARY" | awk '{print $1}')\`"
    echo "- SAT_SEED: \`${SAT_SEED:-unset}\`"
    echo "- SAT_PROFILE: \`${SAT_PROFILE_ARG:-${SAT_PROFILE:-unset}}\`"
    echo "- SAT_CONFIG_REPLAY: \`${SAT_CONFIG_REPLAY_ARG:-${SAT_CONFIG_REPLAY:-unset}}\`"
    echo
    echo "Use this artifact as evidence for hot-path claims only after checking \`stats.jsonl\`, \`time.txt\`, and perf files or unavailable notes."
} >> "$FINAL_DIR/notes.md"

echo "PROFILE_ARTIFACT=$FINAL_DIR"
echo "PRIMARY_EXIT_STATUS=$PRIMARY_STATUS"
echo "CONFIG_HASH=$CONFIG_HASH"
exit "$PRIMARY_STATUS"
