# 1.14 EMA Reset On Focused Entry

Date: 2026-05-22
Bead: SAT-playground-5b2.2.24

## Scope

Fixed a focused/stable mode-switch bug where solver 11 carried LBD EMA restart averages from a
stable phase into the next focused phase. Focused mode uses EMA restarts, while stable mode uses
reluctant restarts, so a stable-phase glue distribution should not calibrate the next focused-mode
EMA window.

This follows the bead requirement for Kissat-style mode transitions: entering focused mode starts
the focused restart calibration from a clean EMA window.

## Selection Rationale

`bv --robot-plan` and `bd ready` showed the remaining P2 Phase 1 bug-fix cluster as the highest-value
work before 1.15 and 1.16. This bead was chosen because it is a narrow, low-risk behavioral fix that
pairs directly with the already-closed target-phase preservation fix and unblocks focused/stable
candidate re-evaluation.

## Implementation

- Added `MovingAverage::reset()`, which sets `value = 0.0` and `initialized = false`.
- Updated `maybe_switch_search_mode()` so stable-to-focused transitions reset:
  - `restart_fast_lbd`
  - `restart_slow_lbd`
- Left focused-to-stable transitions unchanged, because stable mode uses reluctant restarts rather
  than the LBD EMA restart condition.
- Preserved existing mode-switch cleanup:
  - restart pending state is cleared
  - restart conflict counters are reset
  - VMTF search cursor resets on focused entry
  - target phase is still preserved across mode switches and reset only by restart handling
- Updated `solver/11-kissat-port/README.md` and `SOLVER11_STATE.md`.

## Example

Before this change:

```text
stable phase records high glue:
  restart_fast_lbd = 13.0
  restart_slow_lbd = 10.0

mode switches stable -> focused

focused EMA restarts begin with:
  restart_fast_lbd = 13.0
  restart_slow_lbd = 10.0
```

After this change:

```text
mode switches stable -> focused

focused EMA restarts begin with:
  restart_fast_lbd.initialized = false
  restart_slow_lbd.initialized = false
```

The first focused-mode conflict initializes the EMA from focused-mode glue instead of inherited
stable-mode glue.

## Fresh-Eyes Review

Reviewed the diff after implementation. The reset is reachable only inside
`SAT_SEARCH_MODE=focused-stable` mode switching. The default profile uses `SAT_SEARCH_MODE=single`,
so `maybe_switch_search_mode()` returns before the new reset branch.

The regression test covers both sides of the transition:

- focused -> stable preserves existing LBD EMA state
- stable -> focused resets the LBD EMA state

No additional bug was found during review.

## Validation

Commands run:

```bash
cargo fmt --check
cargo test mode_switch -- --nocapture
cargo test restart -- --nocapture
cargo test
bash tools/smoke_test.sh solver/11-kissat-port
SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port
SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_RESTART=kissat-ema bash tools/smoke_test.sh solver/11-kissat-port
bash tools/bench.sh -m 16384 -d benchmarks/profiling --log-dir log/1.14-ema-reset/profile-default-300 solver/11-kissat-port
bash tools/bench.sh -m 16384 -d benchmarks/profiling --log-dir log/1.14-ema-reset/profile-default-300-rerun solver/11-kissat-port
```

Results:

- `cargo test mode_switch`: 5 passed.
- `cargo test restart`: 14 passed.
- Full `cargo test`: 250 passed.
- Default smoke: 9/9 passed.
- Invariant smoke: 9/9 passed.
- Focused/stable EMA smoke: 9/9 passed.

## Profile Results

Baseline for comparison:

- `log/1.14-target-phase-preserve/profile-default-300/results.csv`
- 11/11 solved, PAR-2 629.384

First run:

- `log/1.14-ema-reset/profile-default-300/results.csv`
- 11/11 solved, PAR-2 641.120
- No timeouts, unknowns, errors, model failures, or proof failures.

Repeat run:

- `log/1.14-ema-reset/profile-default-300-rerun/results.csv`
- 11/11 solved, PAR-2 634.365
- No timeouts, unknowns, errors, model failures, or proof failures.

Repeat-run delta versus baseline:

```text
feistel_b64_k32_r22: +0.104s
feistel_b64_k52_r17: -0.019s
feistel_b64_k57_r18: -0.010s
sudoku-N30-12: +5.231s
SC25_Timetable: -0.028s
REGRandom-K4-L1-Seed40: -0.102s
mp1-Nb7T46: -0.326s
Kakuro-easy-112: +0.244s
random_v285_s2: -0.020s
random_v292_s4: -0.001s
random_v355_s3: -0.092s
TOTAL: +4.981s
```

## Decision

Keep the fix. The profile runs show no solved-count, status, model, proof, or timeout regression.
The remaining timing delta is concentrated in one long Sudoku row, while most other rows are flat or
slightly faster on the repeat run. The changed behavior is behind opt-in focused/stable mode and is
not reached by the default single-mode profile.
