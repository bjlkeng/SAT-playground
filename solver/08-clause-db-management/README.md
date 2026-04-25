# 08-clause-db-management

This iteration starts as a direct copy of `07-clause-minimization` and will focus on clause
database management: clause lifetime, deletion policy, storage stability, and eventually
proof-friendly separation between the solver's internal clause store and proof output.

## Current State

`08` currently inherits the full `07` baseline:

- the `06` MiniSat-style clause storage and blocker-watcher rewrite
- runtime conflict-clause minimization modes: `none`, `basic`, and `deep`
- deep minimization through learned-clause reasons
- asserting-clause checks after conflict analysis backtrack
- learned-clause delete bits plus a MiniSat-style bulk-fixup garbage-collection path
- proof logging buffered in memory via `Vec<Vec<i32>>`

`08` now has the first piece of clause-db management in place:

- learned clauses can be marked deleted
- garbage collection compacts live clause storage into a new dense arena
- watcher clause refs and live `reason[var]` refs are fixed up during GC

The solver still does **not** have a deletion policy wired into the search loop yet, and proof
output is still buffered in memory. This iteration is the first storage-management step toward
those follow-on changes.

## Planned Focus

The next set of changes in `08` will target:

- an actual learned-clause deletion policy and GC trigger
- safe cleanup / compaction inside the live search loop
- decoupling proof lifetime from solver-clause lifetime so proof memory does not grow with the
  entire learned-clause history

## What Changed

This first `08` step establishes the clause-db-management substrate:

- copied `07-clause-minimization` to a new self-contained iteration directory
- renamed the package / iteration metadata for `08-clause-db-management`
- added delete bits for learned clauses
- added a garbage-collection pass that compacts live clause storage and rewrites watcher / reason
  refs in bulk
- added regression tests proving that live watcher refs and live `reason[var]` refs survive GC
- kept the solve path otherwise unchanged so the profiling baseline still reflects the inherited
  `07` behavior

## Validation

- `cargo test` — `29/29`
- `bash tools/smoke_test.sh solver/08-clause-db-management` — `9/9`

## Profiling Benchmark Results

Current profiling command:

- `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/08-clause-db-management`

Baseline run before any clause-db-management changes:

- Date: `2026-04-25`
- Result: `PAR-2 41.497`
- Solved: `6/6`
- Log: `log/bench-08-clause-db-management-2026-04-25-17-28-18/results.csv`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r18 | crypto | SAT | 2.986s |
| feistel_b64_k52_r15 | crypto | SAT | 2.132s |
| feistel_b64_k57_r16 | crypto | SAT | 6.462s |
| random_v255_s4 | 3-SAT | UNSAT | 10.368s |
| random_v260_s3 | 3-SAT | UNSAT | 6.270s |
| random_v265_s2 | 3-SAT | UNSAT | 13.279s |

This baseline was recorded while the long `07` medium benchmark was also running in the
background, so re-run it on an otherwise idle machine if we need tighter apples-to-apples
comparison later.

Post-refactor run after adding delete bits and GC fixup infrastructure:

- Date: `2026-04-25`
- Result: `PAR-2 41.820`
- Solved: `6/6`
- Log: `log/bench-08-clause-db-management-2026-04-25-18-37-24/results.csv`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r18 | crypto | SAT | 3.001s |
| feistel_b64_k52_r15 | crypto | SAT | 2.148s |
| feistel_b64_k57_r16 | crypto | SAT | 6.530s |
| random_v255_s4 | 3-SAT | UNSAT | 10.551s |
| random_v260_s3 | 3-SAT | UNSAT | 6.321s |
| random_v265_s2 | 3-SAT | UNSAT | 13.269s |

That is effectively flat versus the pre-refactor baseline, which is the expected result because
the solver still does not run an active deletion policy or invoke GC in the live search loop yet.
