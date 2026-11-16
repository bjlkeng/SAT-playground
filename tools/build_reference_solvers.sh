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

Solvers: kissat-latest kissat-sc2024 minisat cadical
If no solver is given, all vendored reference solvers are built
(cadical only when asked for by name).

cadical is fetched at its pinned release (rel-2.1.3) when its folder is
missing; it is a second, independent solver for checking answers (the
assumptions epic, plan/assumptions-plan.md).

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

CADICAL_TAG="rel-2.1.3"
CADICAL_SHA="f13d74439a5b5c963ac5b02d05ce93a8098018b8"

build_cadical() {
    local dir="$REF_ROOT/cadical"
    if [[ ! -d "$dir/.git" ]]; then
        git clone --quiet https://github.com/arminbiere/cadical.git "$dir" >&2
    fi
    git -C "$dir" -c advice.detachedHead=false checkout --quiet "$CADICAL_TAG" >&2
    local head
    head="$(git -C "$dir" rev-parse HEAD)"
    if [[ "$head" != "$CADICAL_SHA" ]]; then
        echo "cadical: $CADICAL_TAG is at $head, expected $CADICAL_SHA" >&2
        exit 1
    fi
    (cd "$dir" && ./configure >&2 && make -j"$(nproc 2>/dev/null || echo 1)" >&2)
    echo "$dir/build/cadical"
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
        cadical)
            binary="$(build_cadical)"
            record_metadata "$solver" "cd benchmarks/reference-solvers/cadical && git checkout $CADICAL_TAG && ./configure && make" "$binary"
            ;;
        *)
            echo "unknown reference solver: $solver" >&2
            usage >&2
            exit 1
            ;;
    esac
done
