#!/usr/bin/env bash
# Build vendored reference solvers and record pinned binary metadata.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REF_ROOT="$REPO_ROOT/benchmarks/reference-solvers"
OUT_ROOT="$REPO_ROOT/log/reference-baselines"

if [[ -f "$HOME/.cargo/env" ]]; then
    # Keep rust/cargo on PATH for environments where reference builds invoke helper tools.
    source "$HOME/.cargo/env"
fi

usage() {
    cat <<'USAGE'
Usage: bash tools/build_reference_solvers.sh [solver...]

Solvers: kissat-latest kissat-sc2024 minisat
If no solver is given, all vendored reference solvers are built.

The script uses the vendored repositories already present under
benchmarks/reference-solvers and records git SHA, build command, binary SHA-256,
and environment metadata under log/reference-baselines/<solver>/.
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
fi

declare -a SOLVERS=("$@")
if [[ ${#SOLVERS[@]} -eq 0 ]]; then
    SOLVERS=(kissat-latest kissat-sc2024 minisat)
fi

build_kissat() {
    local solver="$1"
    local dir="$REF_ROOT/$solver"
    [[ -d "$dir" ]] || { echo "missing $dir" >&2; exit 1; }
    (cd "$dir" && ./configure && make -j"$(nproc 2>/dev/null || echo 1)")
    echo "$dir/build/kissat"
}

build_minisat() {
    local dir="$REF_ROOT/minisat"
    [[ -d "$dir" ]] || { echo "missing $dir" >&2; exit 1; }
    (cd "$dir" && make config prefix="$dir/build/release" && make -j"$(nproc 2>/dev/null || echo 1)")
    echo "$dir/build/release/bin/minisat"
}

record_metadata() {
    local solver="$1"
    local build_command="$2"
    local binary="$3"
    local out_dir="$OUT_ROOT/$solver"
    mkdir -p "$out_dir"
    {
        echo "solver=$solver"
        echo "date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "repo=$(cd "$REF_ROOT/$solver" 2>/dev/null && pwd || true)"
        echo "git_sha=$(git -C "$REF_ROOT/$solver" rev-parse HEAD 2>/dev/null || echo unknown)"
    } > "$out_dir/commit.txt"
    printf '%s\n' "$build_command" > "$out_dir/build-command.txt"
    sha256sum "$binary" > "$out_dir/binary.sha256"
    {
        echo "uname=$(uname -a)"
        echo "cc=$(${CC:-cc} --version 2>/dev/null | head -1 || true)"
        echo "make=$(make --version 2>/dev/null | head -1 || true)"
        echo "nproc=$(nproc 2>/dev/null || echo unknown)"
    } > "$out_dir/environment.txt"
    echo "built $solver binary=$binary metadata=$out_dir"
}

for solver in "${SOLVERS[@]}"; do
    case "$solver" in
        kissat-latest|kissat-sc2024)
            binary="$(build_kissat "$solver")"
            record_metadata "$solver" "cd benchmarks/reference-solvers/$solver && ./configure && make" "$binary"
            ;;
        minisat)
            binary="$(build_minisat)"
            record_metadata "$solver" "cd benchmarks/reference-solvers/minisat && make config prefix=build/release && make" "$binary"
            ;;
        *)
            echo "unknown reference solver: $solver" >&2
            usage >&2
            exit 1
            ;;
    esac
done
