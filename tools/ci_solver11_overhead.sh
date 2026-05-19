#!/usr/bin/env bash
# Lightweight solver 10 vs solver 11 overhead gate.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$REPO_ROOT/tools/ci_solver11_overhead.py" "$@"
