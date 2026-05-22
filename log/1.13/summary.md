# 1.13 Guarded Chronological Backtracking

## Scope

Implemented the conservative `SAT_CHRONO=on` experiment for solver 11. The default remains off.

The chooser only considers chronological backtracking when:

- the current level is above the normal assertion level
- `current_level - assertion_level <= SAT_CHRONO_MAX_DELTA`
- the learned clause would still be asserting after backtracking to `current_level - 1`

If any guard fails, the solver uses the normal first-UIP assertion level. No trail splicing or
`enqueue_at_level` support was added.

## Code Changes

- `solver/11-kissat-port/src/main.rs`
  - Stores `chrono_backtrack` and `chrono_max_delta` in `Solver`.
  - Adds `learned_clause_asserts_at_level`.
  - Adds `choose_backtrack_level`.
  - Applies the chooser between conflict analysis and `backtrack()` only when `SAT_CHRONO=on`.
  - Keeps the default-off path on the existing assertion level without calling the chooser.
- `solver/11-kissat-port/src/stats.rs`
  - Adds `chrono_attempts`, `chrono_used`, `chrono_rejected_not_asserting`,
    `chrono_rejected_delta_too_large`, and `chrono_skipped_levels`.
  - Emits those counters in JSON stats and `SAT_TRACE_FULL`.
- `solver/11-kissat-port/src/config.rs`
  - Removes `SAT_CHRONO` from parking-lot runtime rejection.
  - Marks it `SmokeSafe` in feature metadata.
- `solver/11-kissat-port/FEATURES.*`, `CONFIG_SCHEMA.csv`, `README.md`,
  and `SOLVER11_STATE.md`
  - Document the opt-in feature and validation artifact.

## Tests

- `cargo fmt --check`
- `cargo test chrono -- --nocapture`
  - 8 chrono/config tests passed.
- `cargo test`
  - 248 passed.
- `bash tools/smoke_test.sh solver/11-kissat-port`
  - 9 passed, 0 failed.
- `SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port`
  - 9 passed, 0 failed.
- `SAT_CHRONO=on SAT_USE_LBD=on SAT_RESTART=kissat-ema SAT_REDUCE=lbd-tiered bash tools/smoke_test.sh solver/11-kissat-port`
  - 9 passed, 0 failed.

## Benchmark Results

Chrono gate configuration:

```bash
SAT_CHRONO=on SAT_USE_LBD=on SAT_RESTART=kissat-ema SAT_REDUCE=lbd-tiered \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/iteration/search-core \
    --log-dir log/1.13/search-core-chrono-gate solver/11-kissat-port
```

Result:

- `0/9` solved
- 8 timeout, 1 unknown
- PAR-2 `2160.000`
- results: `log/1.13/search-core-chrono-gate/results.csv`

This does not meet the promotion gate. Chronological backtracking remains opt-in and is not promoted
to any profile.

Default profile regression check:

```bash
bash tools/bench.sh -m 16384 -d benchmarks/profiling \
  --log-dir log/1.13/profile-default-300-final-rerun solver/11-kissat-port
```

Result:

- timeout `300s`
- `11/11` solved
- 7 SAT, 4 UNSAT
- PAR-2 `628.149`
- results: `log/1.13/profile-default-300-final-rerun/results.csv`

Recent comparable baseline:

- `log/1.12a/reretest-profile-300-default/results.csv`
- PAR-2 `628.815`

The default profile shows no regression.

## Fresh-Eyes Review Notes

Review found and fixed two implementation issues before final validation:

- the default-off conflict path now avoids calling the chrono chooser
- `SAT_TRACE_FULL` non-chrono backtrack accounting now counts all chrono attempts that fell back

No correctness, proof, model, or invariant issues remained after rerunning the tests and smoke
suites above.
