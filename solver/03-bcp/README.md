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

- `cargo test` — 12/12 unit tests passed
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

### Round 3

Feistel-targeted tuning round on 2026-04-16:

- **Round baseline:** `722.137`
- **Solved:** `3/6`

Successful optimization attempts kept in the solver:

1. **Dedicated ternary-clause fast path in `propagate()`**
   Since the profiling suite is entirely binary/ternary and the Feistel instances are dominated by ternary clauses, the propagation loop now handles `len == 3` clauses without falling back to the generic candidate-scan loop.
   PAR-2 improved from `722.137` to `722.120`.

2. **Static occurrence-based branch order**
   Replaced raw variable-index branching with a fixed descending-occurrence order computed once from the input CNF. This dramatically improved the random instances, even though the Feistel cases still timed out.
   PAR-2 improved from `722.120` to `720.074`.

Additional attempts were benchmarked and reverted because they did not improve PAR-2:

- dedicated binary implication path on the pre-ternary baseline
- in-place watch-list compaction on top of the ternary fast path
- dedicated binary implication path on top of the ternary fast path
- binary-aware weighted occurrence scoring for the static branch order

### Round 4

Feistel-focused debugging and tuning on 2026-04-16:

- **Round baseline:** `720.074`
- **Solved:** `3/6`

Successful changes kept in the solver:

1. **Restored scratch watch-list reuse**
   The earlier `watch_scratch` reuse had fallen out of the current working tree during subsequent experiments. Restoring it kept the hot path allocation-free again.
   PAR-2 improved from `720.074` to `720.067`.

2. **Phase saving with static sign initialization**
   Branching now reuses each variable's last assigned polarity, with the initial phase biased by literal polarity counts in the input CNF.
   PAR-2 improved from `720.067` to `720.065`.

3. **Stable forward watcher iteration**
   `propagate()` now processes each falsified literal's watcher list in insertion order instead of reverse `pop()` order.
   PAR-2 improved from `720.065` to `720.058`.

4. **Preserve level-0 assignments across `backtrack(0)`**
   The solver was incorrectly clearing root-level assignments when backjumping to decision level 0. On the Feistel CNFs, which contain unit clauses and root-level implications, that bug destroyed the search state and caused the crypto instances to time out. Tracking `root_trail_len` and preserving those level-0 assignments changed the benchmark from three crypto timeouts to a full solve of the profiling suite.
   PAR-2 improved from `720.058` to `115.083`.

Additional attempts were benchmarked and reverted because they did not improve the target behavior:

- low-occurrence-first static branch order for Feistel-style CNFs

This round also added a regression test covering `backtrack(0)` with root-level unit propagation, so the solver now checks that level-0 assignments survive a backjump to zero.

### Round 5

Phase-saving rollback on 2026-04-16:

- **Round baseline:** `115.083`
- **Solved:** `6/6`

Successful changes kept in the solver:

1. **Removed phase saving and static sign initialization**
   Reverted the polarity-memory heuristic while keeping the other Round 4 changes. On this machine that made `feistel_b64_k32_r14` a bit slower, but it improved the hard `feistel_b64_k32_r16` case enough to reduce PAR-2 overall.
   PAR-2 improved from `115.083` to `87.679` on `benchmarks/profiling`, and from `114.255` to `88.832` on `benchmarks/crypto`.

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
| feistel_b64_k32_r12 | crypto | SAT | 0.412s |
| feistel_b64_k32_r14 | crypto | SAT | 1.104s |
| feistel_b64_k32_r16 | crypto | SAT | 86.100s |
| random_v110_s1 | 3-SAT | UNSAT | 0.010s |
| random_v130_s3 | 3-SAT | SAT | 0.022s |
| random_v140_s1 | 3-SAT | UNSAT | 0.026s |

**PAR-2: 87.679 (6/6 solved)**

This is a `634.822` point improvement over the start of the second tuning round (`722.501`), a `634.458` point improvement over the start of the third round (`722.137`), a `632.395` point improvement over the start of the fourth round (`720.074`), a `27.404` point improvement over the end of the fourth round (`115.083`), and a `1352.321` point improvement over the original unoptimized `03` baseline (`1440.000`).

## Crypto Benchmark Results

Environment:

- **CPU:** AMD Ryzen 5 5600 6-Core (12 threads), 3.5 GHz base / 4.47 GHz boost
- **RAM:** 64 GB DDR4
- **OS:** Ubuntu 22.04
- **Benchmark suite:** `benchmarks/crypto`
- **Command:** `bash tools/bench.sh -t 120 -d benchmarks/crypto solver/03-bcp`

Results:

| Instance | Result | Time |
|----------|--------|------|
| feistel_b64_k32_r8 | SAT | 0.008s |
| feistel_b64_k32_r10 | SAT | 0.025s |
| feistel_b64_k32_r12 | SAT | 0.412s |
| feistel_b64_k32_r14 | SAT | 1.112s |
| feistel_b64_k32_r16 | SAT | 87.266s |

**PAR-2: 88.832 (5/5 solved)**

Historical note from the pre-optimization baseline:

- **Command:** `bash tools/bench.sh -t 600 -d benchmarks/profiling solver/03-bcp`
- **Result:** all 6 profiling instances timed out before the optimization round
- **PAR-2:** `7200.000`
