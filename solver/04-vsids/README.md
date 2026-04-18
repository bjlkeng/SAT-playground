# 04-vsids

First VSIDS iteration built on top of `03-bcp`.

This version keeps the watched-literal CDCL core from `03`, but replaces the static occurrence-order branch picker with a straightforward activity-based heuristic:

- each variable has a floating-point activity score
- variables from the current conflict clause and the learned clause get bumped
- the bump amount decays over time in EVSIDS style
- the next decision picks the highest-activity unassigned variable
- ties fall back to the occurrence-based order inherited from `03`

## Scope of This First Pass

This is a correctness-first VSIDS iteration.

Included:
- watched-literal BCP from `03`
- CDCL learning and non-chronological backtracking from `03`
- simple activity-based branching with conflict bumps and decay

Not included yet:
- heap-backed activity queue
- restarts
- phase saving
- clause deletion / database management

The goal of `04` is to introduce dynamic branching pressure without changing the rest of the search architecture.

## Design Notes

The solver still uses the same state model as `03`:

- `assignment[v]` stores `UNASSIGNED`, `TRUE`, or `FALSE`
- `decision_level[v]` stores the level where `v` was assigned
- `reason[v]` stores the clause index that implied `v`, or a sentinel for decisions
- `trail` stores literals in assignment order
- `trail_limits` stores the trail index where each decision level starts
- watched literals drive propagation incrementally over the trail

VSIDS adds:

- `activity[v]` for each variable
- `activity_inc` for the current bump amount
- `activity_decay` to age out older conflicts

On each conflict, the solver:

1. analyzes the conflict to produce a learned clause
2. bumps variables appearing in the triggering conflict clause
3. bumps variables appearing in the learned clause
4. increases the future bump amount by the decay factor

Decision selection then scans the unassigned variables and picks the highest-activity candidate.

## Validation

Completed checks for this first pass:

- `cargo test` — 14/14 unit tests passed
- `bash tools/smoke_test.sh solver/04-vsids` — 9/9 smoke tests passed
- All 5 UNSAT smoke-test proofs verified with `drat-trim`

The new unit tests added for `04` check that:

- branch selection prefers the highest-activity unassigned variable
- solving a conflict-driven UNSAT instance actually bumps variable activity

## Code-Level Optimization Log

Optimization pass run on 2026-04-17 against the current `benchmarks/profiling` suite for `04-vsids`.

Round baseline before tuning:

- **PAR-2:** `172.268`
- **Solved:** `6/6`

Measurement note:

- A separate long-running `03-bcp` medium benchmark was still active on another core during these measurements, so very small PAR-2 deltas should be treated as approximate.
- Ten one-change optimization attempts were benchmarked in total. Only the successful ones below were kept.

Successful optimization attempts kept in the solver:

1. **Switch VSIDS activity from `f64` to `f32`**
   The activity scores do not need 64-bit precision, and shrinking them improved cache behavior in the decision heuristic enough to materially change performance on the Feistel cases.
   PAR-2 improved from `172.268` to `109.932`.

2. **Raw-pointer activity loads in `pick_branch_lit()`**
   The decision scan now reads `activity[var]` through a cached pointer instead of indexed slice access on every candidate variable.
   PAR-2 improved from `109.932` to `107.783`.

3. **Pointer-style conflict-activity bump loops**
   The loops that bump variables from the conflict clause and learned clause now traverse their literals with pointer-style scans rather than indexed iteration.
   PAR-2 improved from `107.783` to `106.870`.

4. **Shrink `branch_order` from `Vec<usize>` to `Vec<u32>`**
   The VSIDS branch scan walks `branch_order` on every decision, so cutting its footprint improved cache density enough to reduce total time again.
   PAR-2 improved from `106.870` to `105.115`.

Additional attempts were benchmarked and reverted because they did not improve PAR-2:

- removing inherited `branch_rank` / `branch_cursor` bookkeeping
- replacing activity decay division with a precomputed reciprocal multiply
- shrinking `branch_rank` to `u32`
- raw-pointer traversal of `branch_order` itself
- forcing `#[inline(always)]` on the small VSIDS helpers
- shrinking `decision_level` to `u32`

## Profiling Benchmark Results

Environment:

- **CPU:** AMD Ryzen 5 5600 6-Core (12 threads), 3.5 GHz base / 4.47 GHz boost
- **RAM:** 64 GB DDR4
- **OS:** Ubuntu 22.04
- **Benchmark suite:** `benchmarks/profiling`
- **Command:** `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/04-vsids`

Final validation results for the kept code state:

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r17 | crypto | SAT | 7.98s |
| feistel_b64_k49_r15 | crypto | SAT | 4.37s |
| feistel_b64_k57_r14 | crypto | SAT | 23.24s |
| random_v229_s2 | 3-SAT | UNSAT | 18.86s |
| random_v240_s3 | 3-SAT | UNSAT | 22.56s |
| random_v241_s4 | 3-SAT | UNSAT | 29.96s |

**PAR-2: 106.974 (6/6 solved)**

Best observed PAR-2 during the tuning loop was `105.115`; the final revalidation landed at `106.974` under the same background machine load.
