# 09-root-simp-opts

This iteration starts as a direct copy of `08-clause-db-management` and adds MiniSat-style
root-level simplification plus a set of profiled hot-path optimizations.

## Current State

`09` currently inherits the full `08` baseline:

- watched-literal BCP with blocker fast paths
- EVSIDS-style variable activity and saved-phase branching
- conflict-clause minimization modes: `none`, `basic`, and `deep`
- deep minimization through learned-clause reasons
- MiniSat-style learned-clause activity bumps and learned-clause reduction thresholds
- a MiniSat-style packed clause arena with stable clause refs and relocating GC
- streamed proof logging through a fixed 16 MiB byte buffer into `proof.out.tmp`

On top of that, `09` now adds a root-level simplify pass:

- runs only at decision level `0`
- re-propagates before simplifying and returns UNSAT immediately on a root conflict
- deletes satisfied learned clauses
- deletes satisfied original clauses too
- trims literals falsified at level `0` from positions `2..` inside remaining original clauses
- intentionally leaves unsatisfied learned clauses untrimmed, after profiling showed learned-clause
  trimming perturbed CDCL search badly on the current crypto SAT cases
- clears `reason[var]` if simplification deletes the clause that was justifying a root assignment
- rebuilds the branch heap after simplification
- uses a MiniSat-style gate so repeated level-0 visits do not rescan the database unless root
  assignments changed or enough propagation work has happened since the last pass

The same source delta also includes several code-level optimizations that profiling made visible:

- lazy branch-heap cleanup instead of removing every assigned variable during propagation
- bottom-up branch-heap rebuilds after root simplification
- narrower watcher detachment with `swap_remove`
- in-place watcher-list compaction during propagation
- in-place learned-clause reduction without cloning the learned list
- scratch-buffer conflict analysis to avoid a fresh learned-clause allocation per conflict
- a learned-unit shortcut that records the DRAT clause and enqueues the root literal without storing
  a watched learned unit

## What Changed

This `09` step adds root simplification and hot-path cleanup on top of `08`:

- copied `08-clause-db-management` into a new self-contained iteration directory
- renamed the package / iteration metadata for `09-root-simp-opts`
- added `simplify()` to the level-0 search path
- added in-place original-clause trimming for root-false literals while keeping the packed arena
  layout
- generalized clause deletion during simplification so both original and learned clauses can be
  detached, tombstoned, and later reclaimed by GC
- stopped trimming learned clauses at root after ablation showed that deleting satisfied learned
  clauses was useful but strengthening unsatisfied learned clauses caused a large search regression
- added regression tests for:
  - removing satisfied clauses at level `0`
  - trimming root-false literals from surviving original clauses while keeping surviving learned
    clauses intact
  - treating a second simplify call as a no-op when no new root work has happened
  - lazy branch-heap skipping
  - learned-unit shortcut behavior

## Deep Diff vs `08`

The source delta from `08-clause-db-management` is concentrated in `src/main.rs`. `Cargo.toml` and
`Cargo.lock` only rename the package from `sat-solver-08-clause-db-management` to
`sat-solver-09-root-simp-opts`; `build.sh` and `run.sh` are unchanged.

At the data-model level, `09` changes the solver state so original clauses are no longer assumed to
be permanently live:

- removes the fixed `original_clause_count` field and uses `original_clause_ids.len()` when checking
  the live arena during garbage collection
- adds `original_literals` and `learned_literals`, maintained incrementally as clauses are added,
  shortened, deleted, or collected
- adds `simplify_assigns` and `simplify_props_remaining` as the MiniSat-style gate for avoiding
  repeated full database scans at level `0`
- adds `stats.simplifications` so simplify calls can be measured separately from conflicts,
  propagations, reductions, and GC
- adds `scratch_conflict_clause`, a reusable conflict-clause buffer used by the optimized conflict
  analysis path

The new top-level simplification path is implemented through a small group of helpers:

- `clause_satisfied(clause_idx)` scans a clause under the current root assignment and identifies
  clauses that can be removed permanently
- `trim_root_false_literals(clause_idx)` removes literals falsified at level `0` from positions
  `2..` of surviving original clauses; it preserves the two watched positions, rewrites the packed
  clause header size, moves the extra activity word if present, updates live literal counts, and
  counts the removed arena words as reclaimable garbage
- `delete_clause_for_simplify(clause_idx)` handles satisfied original or learned clauses; if the
  clause is a root-level reason, it clears the affected variable's `reason` entry before detaching
  and tombstoning the clause
- `simplify_clause_list(clause_ids)` rebuilds the original and learned clause vectors by deleting
  satisfied clauses and trimming only original clauses that survive
- `simplify()` runs only at decision level `0`, first calls `propagate()` to catch root conflicts,
  skips the scan if no new root assignments or propagation budget justify it, simplifies learned
  clauses and then original clauses, optionally garbage-collects, rebuilds the branch heap, and
  resets the simplify gate

The learned-clause behavior is intentionally asymmetric. `09` deletes satisfied learned clauses, but
does not trim root-false literals from unsatisfied learned clauses. A previous ablation showed that
strengthening learned clauses at root could perturb the CDCL search order enough to lose far more
time than it saved on the current crypto SAT profiling cases.

Several hot-path changes landed alongside the simplify work because profiling made their overhead
visible:

- `enqueue()` no longer removes the assigned variable from the branch heap immediately; instead,
  `pick_branch_lit()` lazily pops and skips assigned variables. This avoids heap mutation on every
  propagation and confines cleanup to branching.
- `rebuild_branch_queue()` now bulk-loads unassigned variables and heapifies bottom-up instead of
  repeatedly calling `push_branch_var()` and sifting each insertion.
- `detach_clause()` now removes watchers with `swap_remove` from the two watched literal lists via
  `detach_clause_watcher()`. `08` used `retain()` and also scanned `watch_scratch`; `09` keeps
  detachment narrower and cheaper.
- `propagate()` now compacts the current watch list in place with read/write indices. `08` moved
  watchers into a separate retained vector and shuffled `watch_scratch` on every scanned watch list.
  The new path keeps the same watched-literal semantics while reducing vector traffic.
- `propagate()` also decrements `simplify_props_remaining` by the number of processed trail entries,
  which gives `simplify()` a cheap budget signal without another counter.
- `reduce_db()` sorts `learned_clause_ids` in place, uses arena-only helpers for clause length and
  activity during sorting, and writes survivors back into the same vector. `08` cloned the learned
  list into `candidates` and deleted clauses one-by-one with a list search inside `mark_clause_deleted`.
- `mark_clause_deleted_already_unlinked()` supports that in-place reduction path by tombstoning a
  learned clause after `reduce_db()` has already decided not to keep it in `learned_clause_ids`.
- conflict analysis now has `analyze_conflict_to_scratch()`, which writes the learned clause into
  `scratch_conflict_clause`, clears only variables touched during analysis, and lets the main solve
  loop move the learned clause out without allocating a fresh `Vec` for every conflict.
- the solve loop special-cases unit learned clauses: it records the DRAT clause, backtracks to root,
  enqueues the asserting literal with `NO_REASON`, and avoids storing a watched learned unit clause.

The important behavioral integration point is in the no-conflict branch of `solve_with_proof()`:
after pending restarts are handled and before learned-clause reduction or branching, `09` calls
`simplify()` whenever the solver is at decision level `0`. A root conflict from that pass returns
UNSAT immediately; otherwise the search continues with the simplified database.

The regression tests added or changed for the diff cover:

- lazy branch-heap skipping of variables assigned by `enqueue()`
- propagation dropping tombstoned watchers when a reduced database leaves deleted learned clauses
  behind
- deletion of satisfied root clauses plus original-only trimming of root-false literals
- no-op repeated simplification when no new root assignments or propagation budget exist
- the learned-unit shortcut, where an UNSAT example now records conflicts but does not retain a
  learned unit clause in the clause database

## Validation

- `cargo test` — `39/39`
- `bash tools/smoke_test.sh solver/09-root-simp-opts` — `9/9`

`AGENTS.md` already contained the required red-green TDD and post-change smoke-test rules, so no
instruction-file change was needed for this step.

## Profiling Benchmark Results

Current profiling command:

- `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/09-root-simp-opts`

The historical benchmark logs below were produced before the directory was renamed from
`09-top-level-simplify` to `09-root-simp-opts`, so their log paths retain the old slug.

Baseline run on the unchanged `08` copy, before adding `simplify()`:

- Date: `2026-04-28`
- Result: `PAR-2 89.177`
- Solved: `6/6`
- Log: `log/bench-09-top-level-simplify-2026-04-28-10-35-51/results.csv`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r22 | crypto | SAT | 15.324s |
| feistel_b64_k52_r17 | crypto | SAT | 17.019s |
| feistel_b64_k57_r18 | crypto | SAT | 12.974s |
| random_v285_s2 | 3-SAT | UNSAT | 10.253s |
| random_v292_s4 | 3-SAT | UNSAT | 22.530s |
| random_v355_s3 | 3-SAT | SAT | 11.077s |

Post-change run after enabling the level-0 simplify pass:

- Date: `2026-04-28`
- Result: `PAR-2 108.066`
- Solved: `6/6`
- Log: `log/bench-09-top-level-simplify-2026-04-28-10-48-32/results.csv`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r22 | crypto | SAT | 10.915s |
| feistel_b64_k52_r17 | crypto | SAT | 23.464s |
| feistel_b64_k57_r18 | crypto | SAT | 29.641s |
| random_v285_s2 | 3-SAT | UNSAT | 10.325s |
| random_v292_s4 | 3-SAT | UNSAT | 22.617s |
| random_v355_s3 | crypto/random SAT | SAT | 11.104s |

The initial net result was a regression on the current profiling set:

- `89.177` -> `108.066`
- slower by `18.889` PAR-2
- about `21.2%` worse overall

The simplify pass helped `feistel_b64_k32_r22`, but it hurt the other two crypto SAT cases much
more, especially `feistel_b64_k57_r18`.

Follow-up run after disabling root-level learned-clause trimming while keeping satisfied-clause
deletion and original-clause trimming:

- Date: `2026-04-28`
- Result: `PAR-2 69.321`
- Solved: `6/6`
- Log: `log/bench-09-top-level-simplify-2026-04-28-11-38-35/results.csv`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r22 | crypto | SAT | 10.960s |
| feistel_b64_k52_r17 | crypto | SAT | 13.510s |
| feistel_b64_k57_r18 | crypto | SAT | 0.817s |
| random_v285_s2 | 3-SAT | UNSAT | 10.306s |
| random_v292_s4 | 3-SAT | UNSAT | 22.558s |
| random_v355_s3 | crypto/random SAT | SAT | 11.170s |

Same profiling set comparison from the loaded-machine run:

| Solver | PAR-2 | Solved | Log |
|--------|------:|-------:|-----|
| `09-root-simp-opts` | 69.321 | 6/6 | `log/bench-09-top-level-simplify-2026-04-28-11-38-35/results.csv` |
| `08-clause-db-management` | 88.908 | 6/6 | `log/bench-08-clause-db-management-2026-04-28-11-46-08/results.csv` |
| `minisat` | 111.928 | 6/6 | `log/bench-minisat-2026-04-28-11-53-52/results.csv` |

These runs shared the host with an unrelated long-running `08` solver process, so use them as
loaded-machine comparisons rather than isolated timing measurements.

## Medium Benchmark Result

Latest medium run used by the static site:

- Date: `2026-05-02`
- Command: `bash tools/bench.sh -t 1800 -m 16384 -d benchmarks/sat-comp-2025-medium solver/09-top-level-simplify`
- Result: `PAR-2 208534.668`
- Solved: `46/100` (`29 SAT + 17 UNSAT`)
- Unsolved: `54` timeouts, `0` unknown, `0` errors
- Log: `log/bench-09-top-level-simplify-2026-04-30-20-00-01/results.csv`

That run also predates the directory rename; it is still the current benchmark result for the code
now named `09-root-simp-opts`.
