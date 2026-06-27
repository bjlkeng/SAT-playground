#!/usr/bin/env bash
# Run the current solver across generated iteration benchmark sets.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOLVER="$("$REPO_ROOT/tools/current_solver.sh")"
TIMEOUT=300
MEMLIMIT_MB=16384
declare -a SETS=(smoke-plus search-core preprocess-core regression-guards)

usage() {
    cat <<'USAGE'
Usage: bash tools/run_matrix.sh [options]

Options:
  --solver <dir>             Solver directory (default: current solver)
  -t, --timeout <seconds>    Per-instance timeout (default: 300)
  -m, --memory <MB>          Memory limit (default: 16384)
  --sets <a,b,c>             Benchmark sets under benchmarks/iteration
  --include-holdout          Include holdout in the run
  --include-stress           Include stress in the run
  --include-killer           Include killer-tests in the run
  -h, --help                 Show this help
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --solver) SOLVER="$("$REPO_ROOT/tools/current_solver.sh" "$2")"; shift 2 ;;
        -t|--timeout) TIMEOUT="$2"; shift 2 ;;
        -m|--memory) MEMLIMIT_MB="$2"; shift 2 ;;
        --sets) IFS=',' read -r -a SETS <<< "$2"; shift 2 ;;
        --include-holdout) SETS+=(holdout); shift ;;
        --include-stress) SETS+=(stress); shift ;;
        --include-killer) SETS+=(killer-tests); shift ;;
        -h|--help) usage; exit 0 ;;
        -*) echo "unknown option: $1" >&2; exit 1 ;;
        *) echo "unexpected argument: $1" >&2; exit 1 ;;
    esac
done

python3 "$REPO_ROOT/tools/select_iter_bench.py" --dry-run

stamp="$(date +%Y-%m-%d-%H-%M-%S)"
for set_name in "${SETS[@]}"; do
    bench_dir="$REPO_ROOT/benchmarks/iteration/$set_name"
    if [[ ! -d "$bench_dir" ]]; then
        echo "missing benchmark set: $bench_dir" >&2
        exit 1
    fi
    log_dir="$REPO_ROOT/log/matrix-${set_name}-${stamp}"
    echo "matrix set=$set_name solver=$SOLVER log=$log_dir"
    bash "$REPO_ROOT/tools/bench.sh" -t "$TIMEOUT" -m "$MEMLIMIT_MB" -d "$bench_dir" --log-dir "$log_dir" "$SOLVER"
done
