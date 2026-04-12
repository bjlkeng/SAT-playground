#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
[[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
RUSTFLAGS="-C target-cpu=native" cargo build --release
