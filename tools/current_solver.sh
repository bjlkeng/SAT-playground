#!/usr/bin/env bash
# Print the repo-relative solver directory for the solver currently under work.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

candidate="${1:-${SAT_TARGET_SOLVER:-${SAT_CURRENT_SOLVER:-${SAT_SOLVER:-}}}}"

normalize_solver() {
    local solver="$1"
    if [[ "$solver" = /* ]]; then
        realpath --relative-to="$REPO_ROOT" "$solver"
    else
        printf '%s\n' "$solver"
    fi
}

if [[ -n "$candidate" ]]; then
    solver_rel="$(normalize_solver "$candidate")"
    if [[ ! -f "$REPO_ROOT/$solver_rel/build.sh" || ! -f "$REPO_ROOT/$solver_rel/run.sh" ]]; then
        echo "current_solver: invalid solver directory: $candidate" >&2
        exit 2
    fi
    printf '%s\n' "$solver_rel"
    exit 0
fi

latest=""
while IFS= read -r solver_dir; do
    rel="${solver_dir#$REPO_ROOT/}"
    latest="$rel"
done < <(
    find "$REPO_ROOT/solver" -maxdepth 1 -mindepth 1 -type d -printf '%f\t%p\n' \
        | awk -F '\t' '$1 ~ /^[0-9][0-9]-/ { print $0 }' \
        | sort -t "$(printf '\t')" -k1,1V \
        | cut -f2- \
        | while IFS= read -r dir; do
            [[ -f "$dir/build.sh" && -f "$dir/run.sh" ]] && printf '%s\n' "$dir"
        done
)

if [[ -z "$latest" ]]; then
    echo "current_solver: no solver/NN-* directory with build.sh and run.sh found" >&2
    exit 2
fi

printf '%s\n' "$latest"
