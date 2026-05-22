# Stable Entry VSIDS Heap Refresh

## Bead

`SAT-playground-e22` - Fix: `kissat_update_scores` not called when entering stable mode.

## Change

Solver 11 focused/stable mode uses VMTF in focused mode when `SAT_VMTF=on`, but stable mode makes
decisions from the VSIDS heap. Before this change, switching from focused to stable did not refresh
the VSIDS heap, so stable mode could begin from stale heap ordering after variable activities changed
during the focused phase.

The fix adds a stable-entry refresh path:

```text
focused -> stable:
  rebuild VSIDS branch heap from current variable activities
  include only unassigned decision variables

stable -> focused:
  keep existing EMA reset and VMTF search reset behavior
```

Implementation detail:

- Added `refresh_stable_branch_heap_scores()`, currently implemented with the existing
  `rebuild_branch_queue()` helper.
- Called it in `maybe_switch_search_mode()` only when the new mode is `Stable`.
- Left default single-mode search untouched because `maybe_switch_search_mode()` returns unless
  `SAT_SEARCH_MODE=focused-stable`.

## Example

Before the switch:

```text
heap order was built when activity[2] = 3.0 and activity[1] = 1.0
focused search later bumps activity[1] to 10.0
```

Before this fix, entering stable mode could still pick variable 2 first from the stale heap. After
the fix, stable entry rebuilds the heap and picks variable 1 first.

## Fresh-Eyes Review

Reviewed the mode switch branch, heap rebuild helper, VMTF pick path, stable VSIDS pick path,
backtrack reinsertion, default-mode reachability, docs, and plan text.

Findings:

- The refresh is intentionally limited to stable entry. It does not change focused-entry restart
  EMA reset or VMTF cursor reset behavior.
- `rebuild_branch_queue()` filters through `unassigned_decision_candidate`, so assigned variables,
  eliminated variables, and non-decision variables are not reintroduced into the heap.
- Default profile remains unaffected by code reachability because it uses
  `SAT_SEARCH_MODE=single`.
- The docs and plan text needed updates so future agents see stable-entry heap refresh as part of
  the current focused/stable behavior.

No additional implementation bugs were found during review.

## Validation

Commands:

```bash
cargo fmt --check
cargo test test_mode_switch_to_stable_refreshes_vsids_heap_scores -- --nocapture
cargo test mode_switch -- --nocapture
cargo test heap -- --nocapture
cargo test
bash tools/smoke_test.sh solver/11-kissat-port
SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port
SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_RESTART=kissat-ema SAT_VMTF=on \
  bash tools/smoke_test.sh solver/11-kissat-port
SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_RESTART=kissat-ema \
  SAT_REDUCE=lbd-tiered SAT_PHASE=target-then-saved SAT_BINARY_FAST=on \
  SAT_SEARCH_MODE=focused-stable SAT_CLAUSE_MIN=basic SAT_VMTF=on \
  bash tools/smoke_test.sh solver/11-kissat-port
bash tools/bench.sh -m 16384 -d benchmarks/profiling \
  --log-dir log/1.14-stable-heap-refresh/profile-default-300 solver/11-kissat-port
bash tools/bench.sh -m 16384 -d benchmarks/profiling \
  --log-dir log/1.14-stable-heap-refresh/profile-default-300-rerun solver/11-kissat-port
```

Results:

```text
cargo fmt --check: passed
new focused regression test: passed
mode_switch tests: 6/6 passed
heap tests: 8/8 passed
cargo test: 252/252 passed
standard smoke: 9/9 passed
invariant smoke: 9/9 passed
focused/stable VMTF smoke: 9/9 passed
advanced focused/VMTF smoke: 9/9 passed
```

Profile artifacts:

- `log/1.14-stable-heap-refresh/profile-default-300/results.csv`
- `log/1.14-stable-heap-refresh/profile-default-300/summary.log`
- `log/1.14-stable-heap-refresh/profile-default-300-rerun/results.csv`
- `log/1.14-stable-heap-refresh/profile-default-300-rerun/summary.log`

Profile results:

```text
first run:  11/11 solved, PAR-2 642.264
rerun:      11/11 solved, PAR-2 627.429
```

The first run was status-clean but slower than the previous default profile, mostly due to the known
high-variance Sudoku row, so a rerun was taken. The rerun is slightly better than the immediately
previous default profile:

```text
log/1.14-learned-used-init/profile-default-300:          PAR-2 631.833
log/1.14-stable-heap-refresh/profile-default-300-rerun:  PAR-2 627.429
```

This confirms no default-profile regression. A direct default-mode speedup is not expected because
the changed branch is only reachable in focused/stable mode.
