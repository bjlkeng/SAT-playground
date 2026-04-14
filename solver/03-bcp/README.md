# 03-bcp

First watched-literal BCP iteration built on top of `02-cdcl`.

This version keeps the same overall CDCL structure from `02`, but replaces full-clause-scan propagation with a basic incremental Boolean constraint propagation implementation:

- two watched literals per non-unit clause
- per-literal watch lists
- trail-head driven propagation over newly assigned literals only
- explicit handling for root-level unit clauses and empty clauses

## Scope of This First Pass

This is a correctness-first BCP iteration.

Included:
- Watched-literal propagation for original and learned clauses
- Incremental propagation using the assignment trail
- Existing CDCL learning and non-chronological backtracking from `02`

Not included yet:
- Specialized binary-clause fast paths
- Watch-list compaction or other propagation tuning
- Better branching heuristics
- Restarts
- Clause deletion / database management

The goal of `03` is to make propagation event-driven before doing any deeper performance work.

## Design Notes

The solver still uses the `02` CDCL state model:

- `assignment[v]` stores `UNASSIGNED`, `TRUE`, or `FALSE`
- `decision_level[v]` stores the level where `v` was assigned
- `reason[v]` stores the clause index that implied `v`, or a sentinel for decisions
- `trail` stores literals in assignment order
- `trail_limits` stores the trail index where each decision level starts

BCP now adds:

- `watch_pos[clause]` to remember which literal positions are currently watched
- `watchers[lit]` to track which clauses are watching each literal
- `propagate_head` so propagation only touches new trail entries

When a literal is assigned, the solver only visits clauses watching the now-false literal. Each such clause either:

- keeps its current watch because the other watched literal already satisfies the clause
- moves the falsified watch to another literal that is not false
- becomes unit and implies the other watched literal
- reports conflict if both watched literals are false and no replacement exists

## Interface Contract

Like every iteration in this repo:

- `build.sh` builds the solver
- `run.sh <cnf_path> <output_dir>` runs the solver
- stdout follows SAT Competition 2025 format
- UNSAT writes `proof.out` into the given output directory

## Validation

Completed checks for this first pass:

- `cargo test` — 11/11 unit tests passed
- `bash tools/smoke_test.sh solver/03-bcp` — 9/9 smoke tests passed
- All 5 UNSAT smoke-test proofs verified with `drat-trim`

## Profiling Benchmark Results

Environment:

- **CPU:** AMD Ryzen 5 5600 6-Core (12 threads), 3.5 GHz base / 4.47 GHz boost
- **RAM:** 64 GB DDR4
- **OS:** Ubuntu 22.04
- **Benchmark suite:** `benchmarks/profiling`
- **Command:** `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/03-bcp`

Results:

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r12 | crypto | TIMEOUT | 120.000s |
| feistel_b64_k32_r14 | crypto | TIMEOUT | 120.000s |
| feistel_b64_k32_r16 | crypto | TIMEOUT | 120.000s |
| random_v110_s1 | 3-SAT | TIMEOUT | 120.000s |
| random_v130_s3 | 3-SAT | TIMEOUT | 120.000s |
| random_v140_s1 | 3-SAT | TIMEOUT | 120.000s |

**PAR-2: 1440.000 (0/6 solved)**

This confirms that the first watched-literal BCP pass is a correctness baseline only. The current implementation regresses substantially against `02-cdcl` and needs targeted follow-up work on watcher handling, clause access, and propagation hot paths before it is competitive.
