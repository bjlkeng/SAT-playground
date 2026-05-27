#!/usr/bin/env bash
# Reproduction script for analyzesat-2026-05-27-propagation-hotpath.
# This run was an ad-hoc lucky on/off ablation rather than a full multi-config
# matrix — the goal was to characterize the lucky preamble overhead and the
# battleship feature gap, not to compare many search configurations.
#
# Build:
#   git worktree add /tmp/analyzesat-propagation-hotpath HEAD
#   cd /tmp/analyzesat-propagation-hotpath/solver/11-kissat-port
#   CARGO_PROFILE_RELEASE_STRIP=false \
#     CARGO_PROFILE_RELEASE_DEBUG=1 \
#     RUSTFLAGS="-C target-cpu=native" cargo build --release
#
# Instances were decompressed once with xzcat into a scratch dir.
set -euo pipefail

SOLVER=${SOLVER:-/tmp/analyzesat-propagation-hotpath/solver/11-kissat-port/target/release/sat-solver}
KISSAT=${KISSAT:-/home/bojji/code/SAT-playground/benchmarks/reference-solvers/kissat-latest/build/kissat}
PROFILING=${PROFILING:-/home/bojji/code/SAT-playground/benchmarks/profiling}
OUT=${OUT:-/home/bojji/code/SAT-playground/log/analyzesat-2026-05-27-propagation-hotpath}
SCRATCH=${SCRATCH:-/tmp/analyzesat-propagation-scratch}

mkdir -p "$OUT/raw" "$OUT/proof" "$SCRATCH"

decompress() {
  local name="$1" xz="$2"
  [ -f "$SCRATCH/$name.cnf" ] || xzcat "$xz" > "$SCRATCH/$name.cnf"
}

run_solver11() {
  local label="$1" cnf="$2" timeout_s="$3" lucky="$4"
  local env_lucky=""
  [ "$lucky" = "on" ] && env_lucky="SAT_LUCKY=on"
  /usr/bin/env $env_lucky SAT_PROOF=off SAT_STATS_JSON=on \
    /usr/bin/time -v timeout "$timeout_s" "$SOLVER" "$cnf" "$OUT/proof" \
    > "$OUT/raw/solver11_${label}.stdout" \
    2> "$OUT/raw/solver11_${label}.stderr" || true
}

run_kissat() {
  local label="$1" cnf="$2" timeout_s="$3"
  /usr/bin/time -v timeout "$timeout_s" "$KISSAT" --statistics "$cnf" \
    > "$OUT/raw/kissat_${label}.stdout" \
    2> "$OUT/raw/kissat_${label}.stderr" || true
}

decompress battleship "$PROFILING/ed6d842f96d10f3400bce251f9e95bfb-battleship-16-31-sat.cnf.xz"
decompress regrandom "$PROFILING/46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized.cnf.xz"
decompress brocard   "$PROFILING/9af7646fc4a32c6f2744ddc0c4b654b7-brocard_problem_large.cnf.xz"
decompress hw        "$PROFILING/3746303c659ef65aaa78f3b52cd5de49-6s299b685_Iter30.cnf.xz"
decompress mp1       "$PROFILING/557d7d4db5399188f62bc39598c6d868-mp1-Nb7T46.cnf.xz"
decompress kakuro    "$PROFILING/5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7.cnf.xz"
decompress scpc      "$PROFILING/663bb5659e42c2c75f74354f48895302-SCPC-500-13.cnf.xz"
decompress velev     "$PROFILING/6832fe907740af686fde98518067ea3f-velev-pipe-sat-1.0-b7.cnf.xz"
decompress sudoku    "$PROFILING/0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12.cnf.xz"
decompress case9     "$PROFILING/fab2022deb130fe3ad1136a5c71b4109-case9.cnf.xz"

# Baselines (lucky=off, default profile)
run_solver11 battleship      "$SCRATCH/battleship.cnf"   60  off
run_solver11 brocard         "$SCRATCH/brocard.cnf"      300 off
run_solver11 hw              "$SCRATCH/hw.cnf"           300 off
run_solver11 regrandom       "$SCRATCH/regrandom.cnf"    120 off
run_solver11 velev_nolucky   "$SCRATCH/velev.cnf"        200 off

# Lucky=on ablation
run_solver11 battleship_lucky "$SCRATCH/battleship.cnf"  60  on
run_solver11 mp1_lucky        "$SCRATCH/mp1.cnf"         120 on
run_solver11 regrandom_lucky  "$SCRATCH/regrandom.cnf"    60 on
run_solver11 scpc_lucky       "$SCRATCH/scpc.cnf"         30 on
run_solver11 case9_lucky      "$SCRATCH/case9.cnf"       180 on
run_solver11 sudoku_lucky     "$SCRATCH/sudoku.cnf"      300 on
run_solver11 velev_lucky      "$SCRATCH/velev.cnf"       300 on
run_solver11 kakuro_lucky     "$SCRATCH/kakuro.cnf"      300 on

# Reference kissat (re-measured on the gap candidates)
run_kissat battleship "$SCRATCH/battleship.cnf"  60
run_kissat brocard    "$SCRATCH/brocard.cnf"    300
run_kissat hw         "$SCRATCH/hw.cnf"         300
run_kissat regrandom  "$SCRATCH/regrandom.cnf"   30
