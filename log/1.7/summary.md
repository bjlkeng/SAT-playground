# Solver 11 Section 1.7 Decision Heap Cleanup

Bead: SAT-playground-5b2.2.10

## Implementation

- Added explicit decision-heap helpers in `solver/11-kissat-port/src/main.rs`:
  - `heap_contains_var`
  - `heap_remove_assigned_top`
  - `heap_reinsert_unassigned_decision_var`
  - `push_branch_var_if_decision`
- Added a shared `unassigned_decision_candidate` predicate for heap rebuild, insertion, stale-top cleanup, and debug assertions.
- Updated activity bumps to insert an eligible unassigned decision variable when it is not currently in the heap.
- Updated backtracking to reinsert through the named helper.
- Updated branch literal selection to clean assigned/non-decision stale top entries before popping the next decision variable.

## Review Notes

- Fresh-eyes review caught a semantic hazard: this solver's `frozen` flag protects variables from BVE but does not mean "never branch." The final heap predicate keeps frozen variables branchable unless `decision_var` is false.
- Eliminated variables still clear `decision_var` and are removed from the heap in preprocessing, so they are not reinserted.
- Existing branch-rank tie behavior remains deterministic; under the default Minisat branch mode, equal activity ties select lower variable ids.

## Tests Added

- `test_eliminated_var_not_reinserted`
- `test_assigned_heap_top_skipped`
- `test_backtrack_reinserts_unassigned_decision_var`
- `test_activity_bump_percolates`
- `test_heap_push_respects_decision_var`
- `test_heap_tie_break_is_deterministic`
- `test_activity_rescale_preserves_order`
- `test_same_seed_reproduces_decision_prefix_on_small_formula`
- `test_different_seed_changes_only_randomized_policy`

## Validation

- `cargo fmt --check`: pass
- `cargo clippy --all-targets -- -D warnings`: pass
- `cargo test`: pass, 172 tests
- `bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9
- `SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9
- `git diff --check`: pass

## Profile Benchmark

Command:

```bash
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling --log-dir log/1.7/profile-after solver/11-kissat-port
```

Result:

- Solved: 9/11
- SAT: 6
- UNSAT: 3
- Timeouts: 2
- Unknown/errors: 0
- PAR-2: 709.429
- Results CSV: `log/1.7/profile-after/results.csv`

Comparison command:

```bash
python3 tools/compare_bench.py --before log/bs6/profile-after/results.csv --after log/1.7/profile-after/results.csv --timeout 120
```

Comparison result:

- Verdict: PASS
- Status changes: none
- Status regressions: none
- Solved count: 9 before, 9 after
- PAR-2 before: 712.548
- PAR-2 after: 709.429
- PAR-2 delta: -3.119
- Median paired speedup: 0.9959
