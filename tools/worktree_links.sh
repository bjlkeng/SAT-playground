#!/usr/bin/env bash
# Link the main checkout's local-only folders into this worktree.
#
# Benchmark files, reference solvers and proof checkers are not in git, so a
# fresh worktree has none of them and most tools and some tests cannot run.
# This script makes symlinks to the main checkout's copies. It never touches a
# tracked file and never copies data.
#
# usage: bash tools/worktree_links.sh /path/to/main/checkout
#
# Safe to run again: existing links are left alone.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: bash tools/worktree_links.sh /path/to/main/checkout" >&2
    exit 2
fi

MAIN="$(cd "$1" && pwd)"
HERE="$(cd "$(dirname "$0")/.." && pwd)"

if [[ "$MAIN" == "$HERE" ]]; then
    echo "worktree_links: this is the main checkout itself; nothing to do" >&2
    exit 2
fi
if [[ ! -d "$MAIN/benchmarks" || ! -d "$MAIN/tools" ]]; then
    echo "worktree_links: $MAIN does not look like a checkout of this repo" >&2
    exit 2
fi

made=0
kept=0

link() {
    # link <target in main> <path here>
    local target="$1" path="$2"
    if [[ -L "$path" || -e "$path" ]]; then
        kept=$((kept + 1))
        return
    fi
    ln -s "$target" "$path"
    made=$((made + 1))
}

# Whole folders that git ignores with a trailing slash. A symlink is not a
# folder to git, so these two also go into the shared exclude file.
exclude="$(cd "$HERE" && git rev-parse --git-common-dir)/info/exclude"
for name in tools/checkers benchmarks/gbd; do
    if [[ -d "$MAIN/$name" ]]; then
        link "$MAIN/$name" "$HERE/$name"
        if ! grep -qxF "/$name" "$exclude" 2>/dev/null; then
            printf '/%s\n' "$name" >> "$exclude"
        fi
    fi
done

# Folders whose tracked files stay and whose other entries are ignored: link
# every entry of the main checkout that git does not track here.
for dir in benchmarks/reference-solvers benchmarks/sat-comp-*; do
    dir="${dir%/}"
    [[ -d "$MAIN/$dir" ]] || continue
    mkdir -p "$HERE/$dir"
    tracked="$(cd "$HERE" && git ls-files -- "$dir")"
    while IFS= read -r -d '' entry; do
        name="$(basename "$entry")"
        if grep -qxF -- "$dir/$name" <<< "$tracked"; then
            continue
        fi
        if [[ -L "$entry" ]]; then
            # main's own symlink (the medium suite points into the full
            # suite with a relative link): copy the link as it is
            if [[ ! -L "$HERE/$dir/$name" && ! -e "$HERE/$dir/$name" ]]; then
                cp -P "$entry" "$HERE/$dir/$name"
                made=$((made + 1))
            else
                kept=$((kept + 1))
            fi
        else
            link "$entry" "$HERE/$dir/$name"
        fi
    done < <(find "$MAIN/$dir" -mindepth 1 -maxdepth 1 -print0)
done

echo "worktree_links: $made links made, $kept already there"
