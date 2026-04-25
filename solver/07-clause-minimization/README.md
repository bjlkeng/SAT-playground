# 07-clause-minimization

This iteration takes the `06-clause-storage` watcher/layout rewrite and adds runtime conflict-clause
minimization.

## Current State

`07` includes:

- the full `06` clause storage and blocker-watcher rewrite
- runtime clause minimization modes: `none`, `basic`, and `deep`
- `basic` and `deep` minimization now recurse through learned-clause reasons as well as original
  reasons
- both redundancy walkers skip the reason-clause head by position (`reason_clause[1..]`)
- level-0 parents do not block literal removal during minimization
- backtracking preserves the target decision level, so minimized learned clauses stay asserting
- debug builds assert that learned clauses are still asserting immediately after backtrack
- regression tests cover learned-clause recursion, opposite-polarity reason heads, level-0 parent
  handling, nonzero backtracking, and non-source support escaping the learned source set
- runtime default `ccmin_mode = deep`
- `SAT_CCMIN_MODE=none|basic|deep` override for benchmarking and debugging
- proof checking currently passes on the smoke suite and the profiling UNSAT instances, but proof
  logging still emits only the minimized learned clause rather than an explicit strengthening chain

## What Changed

This pass extends `06` with more aggressive MiniSat-style conflict-clause shrinking:

- added basic and recursive redundancy checks during conflict analysis
- removed the learned-reason guards so `deep` minimization can recurse through learned clauses
- changed both redundancy walkers to skip the reason head by slot instead of comparing literal
  equality
- taught both redundancy walkers to ignore decision-level-0 parents
- fixed `backtrack()` so it keeps assignments at the target decision level
- added debug assertions and regression tests for the learned-reason / asserting-clause invariants
- kept copied proof logging so DRAT output stays stable under in-place clause mutation
- removed the unused clause-abstraction metadata carried over from the storage refactor

The result is a solver that keeps the faster clause/watcher hot path from `06` and gets a further
search reduction from deeper clause minimization.

## Notes on Learned-Reason Minimization

The solver-side part of learned-reason minimization is now implemented. The important invariants
that turned out to matter in practice were:

- skip the reason-clause head by position (`reason_clause[1..]`), not by comparing literal values
- ignore decision-level-0 parents in both redundancy walkers
- preserve the target decision level when backtracking after conflict analysis
- assert in debug builds that the learned clause is still asserting after backtrack

One caveat remains: the proof log still records only the minimized learned clause. That is enough
for the current smoke suite and profiling runs, which both pass DRAT checking, but the solver does
not yet emit explicit strengthening chains. If proof-side minimization ever needs to become more
verbose, the next steps are either:

- keep a conservative proof-visible clause while using a more aggressive internal clause
- emit explicit strengthening clauses in the proof
- or stream proof output to disk instead of buffering it in `Vec<Vec<i32>>`

## Validation

- `cargo test` — `27/27`
- `bash tools/smoke_test.sh solver/07-clause-minimization` — `9/9`

## Profiling Benchmark Results

Current profiling command:

- `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/07-clause-minimization`

Baseline run before the learned-reason deep-minimization changes, on 2026-04-25:

- Result: `PAR-2 74.496`
- Solved: `6/6`
- Log: `log/bench-07-clause-minimization-2026-04-25-10-20-35/results.csv`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r18 | crypto | SAT | 17.364s |
| feistel_b64_k52_r15 | crypto | SAT | 6.234s |
| feistel_b64_k57_r16 | crypto | SAT | 9.063s |
| random_v255_s4 | 3-SAT | UNSAT | 13.021s |
| random_v260_s3 | 3-SAT | UNSAT | 12.401s |
| random_v265_s2 | 3-SAT | UNSAT | 16.413s |

Final run after enabling learned-reason deep minimization, on 2026-04-25:

- Result: `PAR-2 40.878`
- Solved: `6/6`
- Log: `log/bench-07-clause-minimization-2026-04-25-10-25-43/results.csv`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r18 | crypto | SAT | 3.005s |
| feistel_b64_k52_r15 | crypto | SAT | 2.119s |
| feistel_b64_k57_r16 | crypto | SAT | 6.438s |
| random_v255_s4 | 3-SAT | UNSAT | 10.298s |
| random_v260_s3 | 3-SAT | UNSAT | 6.153s |
| random_v265_s2 | 3-SAT | UNSAT | 12.865s |

That is a `33.618` PAR-2 improvement over the same current profiling set, about `45.1%` faster.

The older `2026-04-21` README snapshot used a different profiling mix (`r17/r15/r14` and
`random_v229/v240/v241`), so its `PAR-2 20.736` result is not directly comparable to the current
numbers above.
