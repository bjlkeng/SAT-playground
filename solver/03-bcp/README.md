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

## Code-Level Optimization Log

### Round 1

Baseline before the first code-level tuning round on 2026-04-15:

- **PAR-2:** `1440.000`
- **Solved:** `0/6`

Measurement note:

- The optimization loop benchmarks in this round were run while a separate long-running `02-cdcl` medium benchmark was active on the machine, so very small sub-point deltas should be treated as approximate.

Successful optimization attempts kept in the solver:

1. **Place the backtrack-level literal in learned-clause slot `1`**
   After conflict analysis, the highest-decision-level non-UIP literal is swapped into position `1`, so newly attached learned clauses start with the standard asserting watch pair.
   PAR-2 improved from `1440.000` to `722.796`.

2. **Raw-pointer assignment loads in `propagate()`**
   The watched-literal propagation hot path now reads assignments through a cached raw pointer instead of repeatedly calling `lit_value()` for watched and candidate literals.
   PAR-2 improved from `722.796` to `722.569`.

3. **`u32` clause indices in watch lists**
   Changed `watchers` from `Vec<Vec<usize>>` to `Vec<Vec<u32>>` to reduce watch-list footprint and improve cache density in propagation.
   PAR-2 improved from `722.569` to `722.523`.

Additional attempts were benchmarked and reverted because they did not improve PAR-2 or broke behavior:

- in-place watch-list reuse/compaction during propagation
- binary-clause fast path
- circular replacement-watch scan from the current watch position
- sorting the full learned-clause tail by decision level
- preallocating initial watch-list capacities

### Round 2

Baseline before the second code-level tuning round on 2026-04-15:

- **PAR-2:** `722.501`
- **Solved:** `3/6`

Successful optimization attempts kept in the solver:

1. **Reuse a scratch watch-list buffer inside `propagate()`**
   Replaced per-literal `retained` allocations with a reusable `watch_scratch` buffer.
   PAR-2 improved from `722.501` to `722.428`.

2. **Pointer-based candidate scans in watched-clause propagation**
   Replaced indexed clause scans with cached watch positions and raw-pointer traversal in the propagation hot path.
   PAR-2 improved from `722.428` to `722.313`.

3. **Direct assignment fast path in `enqueue()`**
   `enqueue()` now checks and writes the assignment slot directly instead of routing through `lit_value()`.
   PAR-2 improved from `722.313` to `722.239`.

4. **Pointer-backed branch-variable scan**
   `pick_branch_lit()` now uses a tight pointer loop over `assignment` rather than a range iterator.
   PAR-2 improved from `722.239` to `722.212`.

5. **Buffered proof streaming for UNSAT output**
   `write_proof()` now streams literals through `BufWriter` instead of building joined strings per clause.
   PAR-2 improved from `722.212` to `722.179`.

6. **`u32` watch positions**
   Shrinking `watch_pos` from `[usize; 2]` to `[u32; 2]` reduced clause-watch metadata in the now pointer-heavy propagation path.
   PAR-2 improved from `722.179` to `722.143`.

7. **`u32` clause metadata**
   Shrinking `ClauseRef` fields from `usize` to `u32` trimmed clause metadata further.
   PAR-2 improved from `722.143` to `722.137`.

Additional attempts were benchmarked and reverted because they did not improve PAR-2:

- early `u32` watch-position shrink on the pre-round-2 baseline
- early `u32` `ClauseRef` conversion on the pre-round-2 baseline
- reusable learned-clause buffer during conflict analysis
- raw-pointer `mark_clause_literals()` traversal
- `u32` reason vector
- generation stamps for `seen` / `resolved`

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
| random_v110_s1 | 3-SAT | UNSAT | 0.150s |
| random_v130_s3 | 3-SAT | SAT | 0.740s |
| random_v140_s1 | 3-SAT | UNSAT | 1.247s |

**PAR-2: 722.137 (3/6 solved)**

This is a `0.364` point improvement over the start of the second tuning round (`722.501`), and a `717.863` point improvement over the original unoptimized `03` baseline (`1440.000`). The kept changes further reduced the random-instance overhead, but the Feistel crypto instances still time out, so additional improvement beyond this point likely needs solver-heuristic or broader algorithmic work rather than more micro-tuning.

Historical note from the pre-optimization baseline:

- **Command:** `bash tools/bench.sh -t 600 -d benchmarks/profiling solver/03-bcp`
- **Result:** all 6 profiling instances timed out before the optimization round
- **PAR-2:** `7200.000`
