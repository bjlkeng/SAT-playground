# 09-top-level-simplify

This iteration starts as a direct copy of `08-clause-db-management` and adds a MiniSat-style
level-0 `simplify()` pass.

## Current State

`09` currently inherits the full `08` baseline:

- watched-literal BCP with blocker fast paths
- EVSIDS-style variable activity and saved-phase branching
- conflict-clause minimization modes: `none`, `basic`, and `deep`
- deep minimization through learned-clause reasons
- MiniSat-style learned-clause activity bumps and learned-clause reduction thresholds
- a MiniSat-style packed clause arena with stable clause refs and relocating GC
- streamed proof logging through a fixed 16 MiB byte buffer into `proof.out.tmp`

On top of that, `09` now adds a top-level simplify pass:

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

## What Changed

This `09` step adds the first MiniSat-style simplify path on top of `08`:

- copied `08-clause-db-management` into a new self-contained iteration directory
- renamed the package / iteration metadata for `09-top-level-simplify`
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

## Validation

- `cargo test` — `38/38`
- `bash tools/smoke_test.sh solver/09-top-level-simplify` — `9/9`

`AGENTS.md` already contained the required red-green TDD and post-change smoke-test rules, so no
instruction-file change was needed for this step.

## Profiling Benchmark Results

Current profiling command:

- `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/09-top-level-simplify`

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
| `09-top-level-simplify` | 69.321 | 6/6 | `log/bench-09-top-level-simplify-2026-04-28-11-38-35/results.csv` |
| `08-clause-db-management` | 88.908 | 6/6 | `log/bench-08-clause-db-management-2026-04-28-11-46-08/results.csv` |
| `minisat` | 111.928 | 6/6 | `log/bench-minisat-2026-04-28-11-53-52/results.csv` |

These runs shared the host with an unrelated long-running `08` solver process, so use them as
loaded-machine comparisons rather than isolated timing measurements.
