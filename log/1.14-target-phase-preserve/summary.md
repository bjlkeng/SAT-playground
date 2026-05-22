# 1.14 Target Phase Preservation Fix

Date: 2026-05-22
Bead: SAT-playground-5b2.2.23

## Scope

Fixed a focused/stable mode-switch bug where solver 11 cleared `target_phase` on every
`maybe_switch_search_mode()` transition. Kissat preserves target phases across mode changes and
clears them through restart handling, so the solver was discarding useful target-phase information
too early for `SAT_PHASE=target-then-saved` and `best-then-target-then-saved` runs.

## Selection Rationale

`bv` and `bd ready` showed several Phase 1 fix beads ready. This bead was chosen first because it
is a small P2 behavioral bug, directly affects the advanced focused/stable candidate stack, and
blocks reliable re-evaluation of the 1.12a candidate modes.

## Implementation

- Removed `reset_target_phase()` from `maybe_switch_search_mode()`.
- Kept restart-state cleanup on mode switches unchanged:
  - `restart_pending = false`
  - `restart_conflicts = 0`
  - `restart_conflicts_since_last = 0`
  - `restart_next_check_conflict` reset to the next conflict
- Added `test_mode_switch_preserves_target_phase`.
- Updated the existing restart-pending mode-switch test so it checks restart state only, not target
  phase clearing.
- Updated `solver/11-kissat-port/README.md` and `SOLVER11_STATE.md` to document that target phases
  persist across focused/stable mode switches and are reset only by restart handling.

## Example

Before this change, if a focused epoch captured:

```text
target_phase[1] = TRUE
target_phase[2] = FALSE
target_assigned = 2
```

then a focused-to-stable mode switch cleared the target phase vector back to `UNASSIGNED` and set
`target_assigned = 0`.

After this change, the same mode switch preserves those target phases. A restart still owns target
phase reset behavior.

## Fresh-Eyes Review

Reviewed the full code diff after implementation. The only solver behavior change is removing the
mode-switch target reset. Restart pending/counter cleanup still happens on each mode switch, and the
existing restart tests plus the new preservation test cover the intended ownership split.

No additional bugs were found during review.

## Validation

Commands run:

```bash
cargo fmt --check
cargo test mode_switch -- --nocapture
cargo test target_phase -- --nocapture
cargo test
bash tools/smoke_test.sh solver/11-kissat-port
SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port
SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_PHASE=target-then-saved bash tools/smoke_test.sh solver/11-kissat-port
bash tools/bench.sh -m 16384 -d benchmarks/profiling --log-dir log/1.14-target-phase-preserve/profile-default-300 solver/11-kissat-port
```

Results:

- `cargo test mode_switch`: 4 passed.
- `cargo test target_phase`: 5 passed.
- Full `cargo test`: 249 passed.
- Default smoke: 9/9 passed.
- Invariant smoke: 9/9 passed.
- Focused/stable target-phase smoke: 9/9 passed.
- Profile benchmark: 11/11 solved, 7 SAT + 4 UNSAT, PAR-2 629.384.

## Profile Comparison

Compared against the immediate prior default profile baseline:

- Baseline: `log/1.13/profile-default-300-final-rerun/results.csv`, PAR-2 628.149.
- New run: `log/1.14-target-phase-preserve/profile-default-300/results.csv`, PAR-2 629.384.
- Delta: +1.235 seconds total across 11 instances.

Per-instance deltas were mixed and small. There were no solved-count, result-status, model, proof,
or timeout regressions.

## Decision

Keep the fix. This is correctness/parity work for target-phase ownership, and the profile delta is
normal benchmark noise for the default profile because default search does not use focused/stable
target-phase selection.
