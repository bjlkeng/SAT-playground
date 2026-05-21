# Solver 11 Phase 1.12 Summary

Bead: `SAT-playground-5b2.2.15` - `[1.12] Rephasing hook`

## Implementation

- Added opt-in runtime support for `SAT_REPHASE=on`.
- Kept the feature constrained to focused/stable search:
  `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_REPHASE=on`.
- Added rephase scheduler state to the solver:
  - `rephase_index`
  - `rephase_at_conflicts`
  - `rephase_conflicts`
  - reuse of the existing `original_phase`
- Rephase fires only on a real stable-mode restart when the global conflict schedule is due.
  Focused-mode restarts, temporary-assumption work, root-level no-op restarts, and default single-mode
  search do not rephase.
- Implemented the default three-step cycle:
  - `best`: copy assigned `best_phase` values into `saved_phase`, using `original_phase` for variables
    outside the captured best prefix
  - `inverted`: flip every concrete saved phase, using `original_phase` as the source for unassigned
    saved entries
  - `original`: restore `saved_phase` from `original_phase`
- Added `phase_save_target`, `phase_save_best`, and `rephases` counters to JSON stats and
  `SAT_TRACE_FULL`.
- Updated the config schema, feature ledger, README, and solver state docs. `SAT_REPHASE` is now
  `SmokeSafe` and remains absent from promoted profiles.

## Fresh-Eyes Review Findings

- Initial smoke checks were mistakenly launched in parallel and collided on the same timestamped log
  directory; one run reported a false missing-output failure. Both smoke configurations were rerun
  sequentially with clean logs and passed.
- Reviewed the restart integration after implementation. The hook runs after `restart_pending` is
  consumed and after target phase reset, but before backtracking to root, so it sees stable phase
  buffers while still preserving restart semantics.
- Reviewed temporary-assumption and root-level paths. `rephase_due_on_stable_restart` explicitly
  rejects temporary accounting, and `perform_restart_if_pending` returns before rephasing when the
  current level is already root.
- Reviewed feature metadata and config validation. The runtime validator no longer rejects
  `SAT_REPHASE=on`, but it does reject enabling rephase outside focused/stable search so benchmark
  artifacts cannot claim a no-op rephase configuration.

## Tests

- `cargo fmt --check`
- `cargo test rephase -- --nocapture`
  - `7 passed`
- `cargo test -- --nocapture`
  - `239 passed`
- `cargo clippy --all-targets -- -D warnings`
- `bash tools/smoke_test.sh solver/11-kissat-port`
  - final clean rerun: `9 passed, 0 failed`
  - log: `log/2026-05-20-23-25-05`
- `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_REPHASE=on bash tools/smoke_test.sh solver/11-kissat-port`
  - final clean rerun: `9 passed, 0 failed`
  - log: `log/2026-05-20-23-25-12`
- `SAT_PROFILE=experimental SAT_PROOF=drat SAT_SEED=0 SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_RESTART=kissat-ema SAT_REDUCE=lbd-tiered SAT_PHASE=best-then-target-then-saved SAT_BINARY_FAST=on SAT_SEARCH_MODE=focused-stable SAT_CLAUSE_MIN=recursive-limited SAT_VMTF=on SAT_REPHASE=on bash tools/smoke_test.sh solver/11-kissat-port`
  - `9 passed, 0 failed`
  - log: `log/2026-05-20-23-25-18`
- `SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port`
  - `9 passed, 0 failed`
  - log: `log/2026-05-20-23-25-26`
- `SAT_STATS_JSON=on SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_REPHASE=on bash tools/smoke_test.sh solver/11-kissat-port`
  - `9 passed, 0 failed`
  - log: `log/2026-05-20-23-36-08`
  - confirmed JSON contains `phase_save_target`, `phase_save_best`, and `rephases`

## Benchmarks

Default profile:

- command: `bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling --log-dir log/1.12/profile-after solver/11-kissat-port`
- result: `9/11` solved, PAR-2 `713.617`
- comparison: `log/1.11/profile-after` was `9/11` solved, PAR-2 `713.508`
- paired compare:
  - status changes: none
  - status regressions: none
  - PAR-2 delta: `+0.109`
  - median paired speedup: `0.9989`
  - verdict: `PASS`

Promotion decision:

- Do not promote rephase into default or fast profiles from this bead.
- Keep `SAT_REPHASE=on` as SmokeSafe opt-in infrastructure for the 1.12a advanced search candidate.
