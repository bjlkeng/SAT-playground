# Solver 11 Phase 1.11 Summary

Bead: `SAT-playground-5b2.2.14` - `[1.11] Clause minimization, in-block shrink, and reason-side bumping`

## Implementation

- Added runtime support for `SAT_CLAUSE_MIN=inblock`.
- Made clause minimization binary-reason aware:
  - binary reasons are scanned by literal variable identity instead of assuming the implied literal is stored first;
  - long clause recursive minimization keeps the legacy `reason_lits_except_first` behavior for child clause reasons;
  - explicit `SAT_CLAUSE_MIN` settings are honored with `SAT_BINARY_FAST=on`.
- Added a shared redundancy-check context carrying reason expansion, decision levels, depth limit, and in-block mode.
- Added depth tracking to recursive minimization via `SAT_MINIMIZE_DEPTH_LIMIT`.
- Added in-block shrink behavior by requiring recursive parent expansion to stay within the candidate literal's decision level unless the parent is already included/removable or level 0.
- Kept `SAT_BINARY_FAST=on` env runs conservative: minimization remains off unless `SAT_CLAUSE_MIN` is explicit. The explicit combinations are smoke-safe, but the search-core gate rejected implicit recursive minimization for binary fast.
- Raised the default minimization depth limit to `1_000_000` to preserve the established default profile trajectory while still allowing bounded experiments through `SAT_MINIMIZE_DEPTH_LIMIT`.
- Updated `README.md`, `FEATURES.md`, and `SOLVER11_STATE.md`.

## Fresh-Eyes Review Findings

- Initial implementation enforced the existing depth limit default of `100`, which made profile instance `557d7d4db5399188f62bc39598c6d868-mp1-Nb7T46` time out. A direct probe with `SAT_MINIMIZE_DEPTH_LIMIT=1000000` solved it in `41.88s`, matching the 1.10 profile behavior. Fixed by changing the default limit to `1_000_000` and keeping the explicit limit test.
- Initial recursive reason scanning used position `0` for all child reasons. That is required for binary reasons, but it changed legacy long-clause behavior. Fixed by starting recursive child scans at position `0` for `ReasonRef::Binary` and position `1` for long clause reasons.
- Initial binary-fast search-core gate with implicit recursive minimization was rejected:
  - log: `log/1.11/search-core-binary-fast-explicit-default-min-regression`
  - result: `1/9` solved, PAR-2 `2017.180`
  - prior ccmin-off gate: `3/9` solved, PAR-2 `1556.657`
  - resolution: `SAT_BINARY_FAST=on` keeps minimization off unless `SAT_CLAUSE_MIN` is explicit.

## Tests

- `cargo fmt --check`
- `cargo test -- --nocapture`
  - final: `232 passed`
- `cargo clippy --all-targets -- -D warnings`
- `bash tools/smoke_test.sh solver/11-kissat-port`
  - final: `9 passed, 0 failed`
- `SAT_BINARY_FAST=on bash tools/smoke_test.sh solver/11-kissat-port`
  - final: `9 passed, 0 failed`
- `SAT_BINARY_FAST=on SAT_CLAUSE_MIN=recursive-limited bash tools/smoke_test.sh solver/11-kissat-port`
  - final: `9 passed, 0 failed`
- `SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_REDUCE=lbd-tiered SAT_RESTART=kissat-ema SAT_SEARCH_MODE=focused-stable SAT_VMTF=on SAT_PHASE=best-then-target-then-saved SAT_BINARY_FAST=on SAT_CLAUSE_MIN=inblock bash tools/smoke_test.sh solver/11-kissat-port`
  - final: `9 passed, 0 failed`

## Benchmarks

Default profile:

- command: `bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling --log-dir log/1.11/profile-after solver/11-kissat-port`
- result: `9/11` solved, PAR-2 `713.508`
- comparison: `log/1.10/profile-after` was `9/11` solved, PAR-2 `716.039`
- verdict: pass; no status regressions, PAR-2 delta `-2.531`

Binary-fast search-core gate:

- command: `SAT_BINARY_FAST=on bash tools/bench.sh -t 120 -m 16384 -d benchmarks/iteration/search-core --log-dir log/1.11/search-core-binary-fast solver/11-kissat-port`
- result: `3/9` solved, PAR-2 `1556.749`
- comparison: `log/1.6/search-core-binary-fast-ccmin-off` was `3/9` solved, PAR-2 `1556.657`
- verdict: pass for conservative binary-fast behavior; no status regressions, PAR-2 delta `+0.092`

Promotion decision:

- Do not promote binary fast or implicit recursive minimization on binary fast.
- Keep `SAT_CLAUSE_MIN=inblock` available as an explicit opt-in mode.
