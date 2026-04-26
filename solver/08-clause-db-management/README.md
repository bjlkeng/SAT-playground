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
- proof logging streamed through a fixed 16 MiB byte buffer into `proof.out.tmp`
- MiniSat-style learned-clause reduction thresholds enabled by default

`08` now has the first piece of clause-db management in place:

- learned clauses can be marked deleted
- learned clauses can be detached from watch lists before deletion
- garbage collection compacts live clause storage into a new dense arena
- watcher clause refs and live `reason[var]` refs are fixed up during GC

`08` now has the first end-to-end learned-clause cleanup path in place:

- stable watcher / reason fixup when clauses move during GC
- proof output that no longer retains the full learned-clause history in RAM
- live learned-clause counting, deletion, and database-reduction helpers
- a dedicated live learned-clause index so `reduce_db()` no longer rescans the full clause table
- O(1) locked-clause checks through the head literal's live `reason[var]`
- internal counters for conflicts, propagations, decisions, restarts, reductions, deletions, GCs,
  and learned clauses

The automatic reducer now runs with MiniSat-style size thresholds and conflict-window growth.
That fixes the earlier "disabled by default" gap, but it still regresses the profiling set versus
the no-auto-reduce snapshot, so the remaining work is to improve clause-quality selection and
overall search behavior rather than keep reducer overhead on the hot path.

## Planned Focus

The next set of changes in `08` will target:

- a better learned-clause scoring policy so automatic reduction can be enabled by default
- more MiniSat-like reduction quality signals and garbage-collection scheduling
- keeping the proof stream sound and low-overhead once internal clause cleanup is active

## What Changed

This `08` step extends the clause-db-management substrate into a safe first reduction path:

- copied `07-clause-minimization` to a new self-contained iteration directory
- renamed the package / iteration metadata for `08-clause-db-management`
- added delete bits for learned clauses
- added a garbage-collection pass that compacts live clause storage and rewrites watcher / reason
  refs in bulk
- added regression tests proving that live watcher refs and live `reason[var]` refs survive GC
- replaced in-memory proof storage with a 16 MiB clause-output buffer that flushes to
  `proof.out.tmp` when full and finalizes to `proof.out` on UNSAT
- added regression tests for periodic proof-buffer flush, UNSAT finalization, SAT temp-file
  cleanup, and end-to-end UNSAT proof emission
- replaced per-literal `write!` formatting in proof logging with direct ASCII integer append into
  the byte buffer
- added watcher detachment for learned-clause deletion
- added live learned-clause counting and wasted-literal tracking so GC decisions are O(1) on the
  hot path
- added a first `reduce_db()` pass that skips locked and binary clauses and deletes low-priority
  unlocked learned clauses
- added regression tests for immediate watcher detachment and locked/binary-clause preservation
- switched to MiniSat-style learned-clause budgets and conflict-window growth for automatic
  reduction
- replaced full clause-table scans inside `reduce_db()` with a dedicated live learned-clause list
  plus reverse positions
- replaced the O(num_vars) locked-clause check with an O(1) head-literal `reason[var]` check
- added internal solver counters for search and clause-db events
- added a regression test that catches stale learned-clause positions after `swap_remove()`

## Validation

- `cargo test` — `35/35`
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

Post-proof-streaming run after replacing in-memory proof storage with the 16 MiB flush buffer:

- Date: `2026-04-25`
- Result: `PAR-2 41.784`
- Solved: `6/6`
- Log: `log/bench-08-clause-db-management-2026-04-25-18-58-03/results.csv`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r18 | crypto | SAT | 2.991s |
| feistel_b64_k52_r15 | crypto | SAT | 2.139s |
| feistel_b64_k57_r16 | crypto | SAT | 6.594s |
| random_v255_s4 | 3-SAT | UNSAT | 10.484s |
| random_v260_s3 | 3-SAT | UNSAT | 6.266s |
| random_v265_s2 | 3-SAT | UNSAT | 13.310s |

That run is also effectively flat relative to the earlier `08` measurements, which is a good
outcome here: proof memory no longer scales with the learned-clause history, and the buffered
disk path did not introduce a visible regression on the profiling set. Like the other `08`
profiling runs today, this benchmark shared the machine with the long `07` medium benchmark that
was already running in the background.

Post-integer-formatting micro-optimization run after replacing `write!` with direct ASCII append:

- Date: `2026-04-25`
- Result: `PAR-2 41.663`
- Solved: `6/6`
- Log: `log/bench-08-clause-db-management-2026-04-25-20-36-15/results.csv`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r18 | crypto | SAT | 3.016s |
| feistel_b64_k52_r15 | crypto | SAT | 2.128s |
| feistel_b64_k57_r16 | crypto | SAT | 6.556s |
| random_v255_s4 | 3-SAT | UNSAT | 10.558s |
| random_v260_s3 | 3-SAT | UNSAT | 6.284s |
| random_v265_s2 | 3-SAT | UNSAT | 13.121s |

This is a small improvement over the earlier proof-buffer run (`41.784`), which is about what we
would expect from removing formatting overhead from the proof fast path without changing the core
search algorithm.

Post-live-deletion/GC-plumbing run with automatic reduction disabled by default:

- Date: `2026-04-25`
- Result: `PAR-2 42.959`
- Solved: `6/6`
- Log: `log/bench-08-clause-db-management-2026-04-25-21-20-45/results.csv`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r18 | crypto | SAT | 3.015s |
| feistel_b64_k52_r15 | crypto | SAT | 2.174s |
| feistel_b64_k57_r16 | crypto | SAT | 6.687s |
| random_v255_s4 | 3-SAT | UNSAT | 10.829s |
| random_v260_s3 | 3-SAT | UNSAT | 6.363s |
| random_v265_s2 | 3-SAT | UNSAT | 13.891s |

This is slightly slower than the prior `08` snapshot (`41.663`), but it lands the deletion /
detachment / GC plumbing cleanly without leaving the regressed eager reducer enabled in the
default solve path. Like the other `08` profiling runs today, this benchmark shared the machine
with the long `07` medium benchmark that was already running in the background.

First MiniSat-style automatic-reduction run after enabling learned-clause cleanup by default:

- Date: `2026-04-25`
- Result: `PAR-2 52.957`
- Solved: `6/6`
- Log: `log/bench-08-clause-db-management-2026-04-25-22-16-18/results.csv`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r18 | crypto | SAT | 2.404s |
| feistel_b64_k52_r15 | crypto | SAT | 18.074s |
| feistel_b64_k57_r16 | crypto | SAT | 9.646s |
| random_v255_s4 | 3-SAT | UNSAT | 8.298s |
| random_v260_s3 | 3-SAT | UNSAT | 5.215s |
| random_v265_s2 | 3-SAT | UNSAT | 9.320s |

This confirmed that simply turning on MiniSat-style thresholds was not enough on top of the
current implementation: the search work dropped on some UNSAT random instances, but the reducer's
own bookkeeping and the clause-selection quality were still too costly overall.

Post-reduce-db-bookkeeping run after switching `reduce_db()` to a dedicated learned-clause list,
O(1) locked-clause checks, and internal counters:

- Date: `2026-04-25`
- Result: `PAR-2 51.997`
- Solved: `6/6`
- Log: `log/bench-08-clause-db-management-2026-04-25-23-18-05/results.csv`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r18 | crypto | SAT | 2.350s |
| feistel_b64_k52_r15 | crypto | SAT | 17.734s |
| feistel_b64_k57_r16 | crypto | SAT | 9.327s |
| random_v255_s4 | 3-SAT | UNSAT | 8.242s |
| random_v260_s3 | 3-SAT | UNSAT | 5.150s |
| random_v265_s2 | 3-SAT | UNSAT | 9.194s |

This improves slightly on the first MiniSat-style automatic-reduction run (`52.957`) by removing
most of the reducer's own bookkeeping overhead, but it is still materially worse than the
no-auto-reduce run (`42.959`). That tells us the remaining gap is mostly search quality and clause
selection, not the raw cost of scanning for clauses to delete.
