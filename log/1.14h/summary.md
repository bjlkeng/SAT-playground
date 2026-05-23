# 1.14h LBD Reason Propagation Update

## Bead

- `SAT-playground-5b2.2.29`
- Phase 1 search/LBD metadata task.

## Implementation

Solver 11 previously supported `SAT_LBD_UPDATE_REASONS` during conflict analysis, but learned
clauses used directly as propagation reasons did not refresh their stored LBD in the propagation hot
path.

This change adds `note_clause_used_as_propagation_reason` and calls it after a successful clause
reason enqueue in both watched-clause unit propagation paths. During normal search it:

- skips original, deleted, and non-learned clauses;
- recomputes the reason clause LBD after the implied literal has been enqueued;
- lowers the stored LBD and reclassifies the clause tier when `SAT_LBD_UPDATE_REASONS=on`;
- marks learned propagation reasons recently used when `SAT_REDUCE=lbd-tiered`.

Fresh-eyes review found that temporary-assumption propagation also reuses the normal propagation
code. The helper now receives `normal_search_accounting` and returns early during temporary
assumption contexts so speculative propagations do not mutate the normal learned-clause database.

## Focused Tests

- `test_propagation_reason_lbd_update_uses_implied_literal_level`
- `test_propagation_reason_marks_recent_without_lbd_update_flag`
- `test_temp_assumption_propagation_does_not_update_reason_lbd`

The implied-literal-level test uses learned clause `[2, 1, 3]`, then decides `-3` at level 1 and
`-1` at level 2. Propagation enqueues `2` at level 2, so the recomputed LBD drops from `9` to `2`.
This confirms the update happens after enqueue; updating before enqueue would incorrectly see the
implied literal as unassigned at level 0.

## Validation

- `cargo fmt --check`: PASS.
- `cargo test propagation_reason -- --nocapture`: PASS.
- `cargo test temp_assumption -- --nocapture`: PASS, 8 tests.
- `cargo test lbd_update -- --nocapture`: PASS, 3 tests.
- `cargo test`: PASS, 291 tests.
- `bash tools/smoke_test.sh solver/11-kissat-port`: PASS, 9/9.
- `SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-port`: PASS, 9/9.
- `SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_REDUCE=lbd-tiered bash tools/smoke_test.sh solver/11-kissat-port`: PASS, 9/9.

## Profile Bench

Default profile benchmark settings:

- `SAT_STATS_JSON=on SAT_TRACE_FULL=on`
- `bash tools/bench.sh -t 300 -m 16384 -d benchmarks/profiling solver/11-kissat-port`
- 11 profiling instances, 300 seconds per instance, 16 GiB memory limit.

Before:

- `log/phase1/1.16-matrix/profile-default/results.csv`
- solved 11/11
- PAR-2 `634.478`

After:

- `log/phase1/1.14h-profile-default-after/results.csv`
- solved 11/11
- PAR-2 `639.784`
- `stats.jsonl` rows: 11
- `errors.log`: empty
- `warnings.log`: empty

Comparison:

- no status regressions;
- PAR-2 delta `+5.306s` across the full 11-instance profile set;
- median paired speedup `1.0001`;
- largest regression was `sudoku-N30-12` at `+7.660s`;
- largest improvement was `REGRandom-K4-L1-Seed40` at `-2.101s`.

The default profile does not enable `SAT_LBD_UPDATE_REASONS`, so this small aggregate movement is
treated as run noise rather than a feature-path regression. The opt-in path was exercised by focused
unit tests and an enabled-feature smoke suite.

## Feature-Mode Profile Follow-Up

After the initial default-profile check, the explicit feature mode was benchmarked before and after
the change:

- `SAT_USE_LBD=on`
- `SAT_LBD_UPDATE_REASONS=on`
- `SAT_REDUCE=lbd-tiered`
- `SAT_STATS_JSON=on`
- `SAT_TRACE_FULL=on`
- `bash tools/bench.sh -t 300 -m 16384 -d benchmarks/profiling solver/11-kissat-port`

Before commit `37e8cb4`:

- `log/phase1/1.14h-feature-profile-before/results.csv`
- solved 10/11
- PAR-2 `1119.940`
- timeout: `mp1-Nb7T46`

After commit `b659331`:

- `log/phase1/1.14h-feature-profile-after/results.csv`
- solved 9/11
- PAR-2 `1708.024`
- timeouts: `mp1-Nb7T46`, `random_v355_s3`

Comparison:

- `compare_bench.py` verdict: FAIL / significant regression;
- status regression: `random_v355_s3` changed from `SAT` in `62.025s` to `TIMEOUT`;
- PAR-2 delta: `+588.084s`;
- median paired speedup: `0.9223`;
- largest wins: `random_v292_s4 -10.116s`, `REGRandom-K4-L1-Seed40 -7.848s`;
- largest regressions: `random_v355_s3 +237.975s`, `sudoku-N30-12 +24.672s`,
  `Kakuro-easy-112 +22.071s`, Timetable `+10.154s`, `feistel_b64_k57_r18 +9.398s`.

Conclusion: the default-off path remains safe by the default-profile check, but the explicit
feature mode is not promotable. Follow-up bead `SAT-playground-5b2.2.42` tracks investigation or
tuning/rollback before this propagation-time reason LBD update can be promoted.

## Follow-Up Fix: `SAT-playground-5b2.2.42`

Fresh-eyes review of the failed feature-mode profile found that `SAT_LBD_UPDATE_REASONS=on` had
started doing two separate things:

- conflict-analysis reason-side LBD improvement, which was the older intended scope;
- propagation-time learned reason LBD recomputation and lbd-tiered recent-use marking, which was the
  new behavior that produced the `random_v355_s3` status regression.

The fix restores `SAT_LBD_UPDATE_REASONS=on` to conflict-analysis reason-side LBD improvement only.
The propagation-time behavior is now isolated behind a new experimental
`SAT_LBD_UPDATE_PROP_REASONS=on` flag, which requires `SAT_LBD_UPDATE_REASONS=on`. In solver tests,
`test_reason_update_flag_does_not_touch_propagation_reason_metadata` confirms the restored mode does
not mutate propagation-reason LBD or recent-use metadata, while
`test_propagation_reason_lbd_update_uses_implied_literal_level` keeps coverage for the explicitly
enabled propagation-time experiment.

Validation after the fix:

- `cargo fmt --check`: PASS.
- `cargo test propagation_reason -- --nocapture`: PASS, 2 tests.
- `cargo test temp_assumption -- --nocapture`: PASS, 8 tests.
- `cargo test lbd_reason_update -- --nocapture`: PASS, 2 tests.
- `cargo test`: PASS, 292 tests.
- `bash tools/smoke_test.sh solver/11-kissat-port`: PASS, 9/9.
- `SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-port`: PASS, 9/9.
- `SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_REDUCE=lbd-tiered
  bash tools/smoke_test.sh solver/11-kissat-port`: PASS, 9/9.
- `SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_LBD_UPDATE_PROP_REASONS=on
  SAT_REDUCE=lbd-tiered bash tools/smoke_test.sh solver/11-kissat-port`: PASS, 9/9.

Targeted regression check:

- settings: `SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_REDUCE=lbd-tiered
  SAT_STATS_JSON=on SAT_TRACE_FULL=on SAT_TRACE_SEARCH_INTERVAL=500000 SAT_LIMIT_WALL_SEC=120`
- instance: `benchmarks/profiling/random_v355_s3.cnf`
- log: `log/phase1/1.14h-prop-reason-fix/random_v355_no_prop.stderr`
- result: `SAT` in `63.504s`, with `2,144,818` conflicts, `2,625,013` decisions, and
  `117,452,619` propagations.

Restored feature-mode profile after the fix:

- settings: `SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_REDUCE=lbd-tiered
  SAT_STATS_JSON=on SAT_TRACE_FULL=on`
- command: `bash tools/bench.sh -t 300 -m 16384 -d benchmarks/profiling --log-dir
  log/phase1/1.14h-prop-reason-fix-profile-after solver/11-kissat-port`
- solved `10/11` (`6 SAT`, `4 UNSAT`, `1 TIMEOUT`)
- timeout: `mp1-Nb7T46`
- `random_v355_s3`: `SAT` in `62.636s`
- PAR-2: `1127.546`
- `stats.jsonl` rows: `10`
- `errors.log`: empty
- `warnings.log`: empty

Comparison to the original pre-propagation-update feature run:

- before: `log/phase1/1.14h-feature-profile-before/results.csv`
- after: `log/phase1/1.14h-prop-reason-fix-profile-after/results.csv`
- verdict: PASS
- status regressions: none
- solved before/after: `10/11` -> `10/11`
- PAR-2: `1119.940` -> `1127.546` (`+7.606s`)
- median paired speedup: `0.9922`

Comparison to the regressed propagation-update feature run:

- before: `log/phase1/1.14h-feature-profile-after/results.csv`
- after: `log/phase1/1.14h-prop-reason-fix-profile-after/results.csv`
- verdict: PASS / significant improvement
- newly solved: `random_v355_s3`
- solved before/after: `9/11` -> `10/11`
- PAR-2: `1708.024` -> `1127.546` (`-580.478s`)
- median paired speedup: `1.0770`
