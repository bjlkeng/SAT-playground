# Solver 11 Section 1.4 EMA Restart Policy

Bead: SAT-playground-5b2.2.7

## Implementation

- Added a `MovingAverage` helper and EMA restart state to `solver/11-kissat-port/src/main.rs`.
- Routed restart scheduling through `RestartPolicy` while preserving the default legacy Luby behavior.
- Implemented opt-in Kissat/Glucose-style LBD EMA restarts behind:
  - `SAT_USE_LBD=on`
  - `SAT_RESTART=kissat-ema`
- Kept `SAT_RESTART=reluctant` rejected as unsupported until the focused/stable-mode bead.
- Added `glucose_restarts` to stats, JSON stats, and full trace output.
- Updated solver 11 documentation to list the opt-in restart policy and note that it is not promoted to default behavior.

## Fresh-Eyes Review Notes

- Checked that temporary assumption accounting does not mutate restart EMAs or restart counters.
- Checked that level-zero conflicts do not schedule EMA restarts.
- Checked that legacy Luby restart cadence remains unchanged and ignores the EMA minimum interval.
- Checked that the first implementation still backtracks to level 0; trail reuse remains out of scope for this bead.
- Checked that the policy remains opt-in because the search-core gate regressed significantly.

## Tests Added Or Updated

- `test_kissat_ema_restart_is_runtime_supported_with_lbd`
- `test_temp_assumption_does_not_update_restart_ema`
- `test_no_restart_at_level_zero`
- `test_lbd_ema_fast_reacts_faster_than_slow`
- `test_restart_triggers_when_fast_exceeds_slow_by_margin`
- `test_restart_blocked_during_min_interval`
- `test_restart_policy_legacy_unchanged_when_selected`
- `test_restart_backtracks_and_preserves_root_units`

## Validation

- `cargo fmt --check`: pass
- `cargo clippy --all-targets -- -D warnings`: pass
- `cargo test`: pass, 178 tests
- `cargo test restart -- --nocapture`: pass, 8 tests
- `cargo test ema -- --nocapture`: pass, 6 tests
- `bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9
- `SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9
- `SAT_USE_LBD=on SAT_RESTART=kissat-ema SAT_REDUCE=lbd-tiered bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9

## Default Profile Benchmark

Command:

```bash
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling --log-dir log/1.4/profile-after solver/11-kissat-port
```

Result:

- Solved: 9/11
- SAT: 6
- UNSAT: 3
- Timeouts: 2
- Unknown/errors: 0
- PAR-2: 708.182
- Results CSV: `log/1.4/profile-after/results.csv`

Comparison command:

```bash
python3 tools/compare_bench.py --before log/1.7/profile-after/results.csv --after log/1.4/profile-after/results.csv --timeout 120
```

Comparison result:

- Verdict: PASS
- Status changes: none
- Status regressions: none
- Solved count: 9 before, 9 after
- PAR-2 before: 709.429
- PAR-2 after: 708.182
- PAR-2 delta: -1.247
- Median paired speedup: 1.0062

## Opt-In Search-Core Gate

Command:

```bash
SAT_USE_LBD=on SAT_RESTART=kissat-ema SAT_REDUCE=lbd-tiered SAT_STATS_JSON=on \
  bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/search-core \
  --log-dir log/1.4/search-core-ema-1 solver/11-kissat-port
```

Result:

- Solved: 1/9
- SAT: 1
- UNSAT: 0
- Timeouts: 7
- Unknown: 1
- Errors: 0
- PAR-2: 2008.719
- Results CSV: `log/1.4/search-core-ema-1/results.csv`

Comparison against the prior search-core baseline `log/bench-s11-1.3a-search-core-2026-05-19-1709/results.csv`:

- Verdict: FAIL
- Promotion verdict: significant regression
- Status regressions:
  - `544707209399nw.shuffled-as.sat03-1671`: SAT -> TIMEOUT
  - `SC25_Timetable_C_406`: SAT -> TIMEOUT
  - `battleship-16-31-sat`: SAT -> TIMEOUT
  - `mp1-Nb7T46`: SAT -> TIMEOUT
- PAR-2 before: 1195.842
- PAR-2 after: 2008.719
- PAR-2 delta: +812.877

The median-of-three promotion gate was stopped after the first run because the first run had large status regressions and a clear PAR-2 regression. The implementation remains available for A/B testing, but is intentionally not promoted to default behavior.

## Preserved Counter Sample

Manual bounded stats capture:

```bash
xz -dkc benchmarks/iteration/search-core/544707209399nw.shuffled-as.sat03-1671.cnf.xz \
  > log/1.4/manual-stats-544/input.cnf
SAT_USE_LBD=on SAT_RESTART=kissat-ema SAT_REDUCE=lbd-tiered SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=5 \
  bash solver/11-kissat-port/run.sh \
  log/1.4/manual-stats-544/input.cnf \
  log/1.4/manual-stats-544/proof
```

The temporary decompressed input was removed after capture; the source `.cnf.xz`, stdout, stderr, and result contract remain.

Captured from `log/1.4/manual-stats-544/stderr.txt`:

- Result: UNKNOWN
- Termination: wall-clock-limit
- Elapsed: 5.006492s
- Conflicts: 24074
- Decisions: 35353
- Propagations: 19698059
- Restarts: 148
- Luby restarts: 0
- Glucose restarts: 148
- Avg LBD: 13.191202
- Avg decision level: 18.849885
