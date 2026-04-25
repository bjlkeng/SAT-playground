# 06-clause-storage

This iteration starts from the `05-restarts` solver and focuses only on the MiniSat-style clause
storage and watcher rewrite.

## Current State

`06` includes:

- watched literals stored directly in clause slots `0` and `1`
- watcher entries with MiniSat-style `{ clause, blocker }` metadata
- blocker-based propagation with in-place watch swapping
- copied proof logging so clause-body mutation does not corrupt DRAT output
- the `05` restart/phase-saving/heap baseline otherwise unchanged

This iteration intentionally stops before runtime clause minimization. The follow-on minimization
work lives in `07-clause-minimization`, and `06` no longer carries a runtime ccmin mode.

## What Changed

The main storage changes are:

- removed `watch_pos` and made watched literals part of the clause body layout
- changed watcher lists from bare clause ids to blocker watchers
- rewrote propagation around the standard MiniSat flow:
  blocker fast path, normalize false watch into slot `1`, scan from `2..`, then unit/conflict
- switched proof logging to copy learned clauses on insertion because clause bodies now mutate
- removed the unused clause-abstraction metadata carried over from the initial MiniSat-shaped port

## Validation

- `cargo test` — `21/21`
- `bash tools/smoke_test.sh solver/06-clause-storage` — `9/9`

## Profiling Benchmark Result

Profiling run on 2026-04-21:

- Command: `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/06-clause-storage`
- Result: `PAR-2 58.282`
- Solved: `6/6`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r17 | crypto | SAT | 0.963s |
| feistel_b64_k49_r15 | crypto | SAT | 11.822s |
| feistel_b64_k57_r14 | crypto | SAT | 1.441s |
| random_v229_s2 | 3-SAT | UNSAT | 9.340s |
| random_v240_s3 | 3-SAT | UNSAT | 13.614s |
| random_v241_s4 | 3-SAT | UNSAT | 21.102s |

This is the storage-only baseline for comparing `07`.
