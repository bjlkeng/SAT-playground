#!/usr/bin/env bash
# run_bench_reference.sh — One-shot wrapper for bench_reference.sh
# Designed to be run from crontab so it survives Claude Code sessions.
#
# Usage: bash tools/run_bench_reference.sh [bench_reference.sh args...]
#
# Logs all output to log/bench_reference_<timestamp>.log
# Creates a sentinel file log/bench_reference_RUNNING while active.
# On completion, creates log/bench_reference_DONE.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# Ensure cargo is in PATH
export PATH="$HOME/.cargo/bin:$PATH"

TIMESTAMP=$(date +%Y-%m-%d-%H-%M-%S)
LOG_FILE="$REPO_ROOT/log/bench_reference_${TIMESTAMP}.log"
RUNNING_SENTINEL="$REPO_ROOT/log/bench_reference_RUNNING"
DONE_SENTINEL="$REPO_ROOT/log/bench_reference_DONE"

mkdir -p "$REPO_ROOT/log"

# Remove old sentinels
rm -f "$DONE_SENTINEL"

# Create running sentinel with PID and args
echo "PID=$$ started=$(date) args=$*" > "$RUNNING_SENTINEL"

{
    echo "=== Reference Solver Benchmark ==="
    echo "Started: $(date)"
    echo "Args: $*"
    echo "PID: $$"
    echo ""

    bash "$REPO_ROOT/tools/bench_reference.sh" "$@"

    echo ""
    echo "Completed: $(date)"
} > "$LOG_FILE" 2>&1

# Signal completion
rm -f "$RUNNING_SENTINEL"
echo "completed=$(date) log=$LOG_FILE args=$*" > "$DONE_SENTINEL"
