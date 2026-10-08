#!/usr/bin/env bash
# setup_checkers.sh — Download and build UNSAT proof checkers
#
# Usage: bash tools/setup_checkers.sh
#
# Builds drat-trim into tools/checkers/drat-trim/

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECKERS_DIR="$SCRIPT_DIR/checkers"

mkdir -p "$CHECKERS_DIR"

# --- drat-trim ---
if [[ -x "$CHECKERS_DIR/drat-trim/drat-trim" ]]; then
    echo "drat-trim already built at $CHECKERS_DIR/drat-trim/drat-trim"
else
    echo "Cloning and building drat-trim..."
    cd "$CHECKERS_DIR"
    if [[ ! -d drat-trim ]]; then
        git clone https://github.com/marijnheule/drat-trim.git
    fi
    cd drat-trim
    make
    echo "drat-trim built: $CHECKERS_DIR/drat-trim/drat-trim"
fi

# --- VeriPB (the structure pass's stage-1 checker, plan section 11 2026-10-07) ---
if [[ -x "$CHECKERS_DIR/VeriPB/target/release/veripb" ]]; then
    echo "VeriPB already built at $CHECKERS_DIR/VeriPB/target/release/veripb"
else
    echo "Cloning and building VeriPB (needs cargo)..."
    cd "$CHECKERS_DIR"
    if [[ ! -d VeriPB ]]; then
        git clone https://gitlab.com/MIAOresearch/software/VeriPB.git
    fi
    cd VeriPB
    cargo build --release
    echo "VeriPB built: $CHECKERS_DIR/VeriPB/target/release/veripb"
fi
