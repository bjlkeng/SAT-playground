#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
[[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
# No -C target-cpu=native here (the other iterations use it): measured
# 2026-09-05, the AVX-512 codegen it enables (797 zmm instructions) is 8-11%
# SLOWER than the generic x86-64 build on search-bound cells (crafted 114.9 v
# 106.2 s, SCPC 4.75 v 4.28 s; x86-64-v3 +1.7%, x86-64-v2 +6%), and the
# reference kissat is itself built generic (-O3, no -march).  Both paired
# 400-instance runs before this date used native binaries.  See README.
RUSTFLAGS="" cargo build --release
