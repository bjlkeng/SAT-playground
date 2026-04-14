# 02-cdcl

First clean CDCL iteration built on top of `01-naive-dpll`.

This version replaces recursive DPLL backtracking with an explicit iterative CDCL loop:

- clause database containing original and learned clauses
- trail-based assignment state
- decision levels per variable
- per-variable reason clauses for implied assignments
- conflict analysis with clause learning
- non-chronological backtracking

## Scope of This First Pass

This is intentionally a correctness-first CDCL implementation, not an optimized one.

Included:
- Full-clause-scan unit propagation to fixpoint
- First-unassigned branching with fixed positive polarity
- Learned clause insertion
- Non-chronological backjumping

Not included yet:
- Watched literals
- VSIDS or any dynamic branching heuristic
- Restarts
- Clause deletion / database management
- Performance tuning

That separation is deliberate: `02` establishes the CDCL state model cleanly, and later iterations can optimize it without changing the overall solver architecture again.

## Design Notes

The solver keeps explicit mutable state instead of using recursion:

- `assignment[v]` stores `UNASSIGNED`, `TRUE`, or `FALSE`
- `decision_level[v]` stores the level where `v` was assigned
- `reason[v]` stores the clause index that implied `v`, or a sentinel for decisions
- `trail` stores literals in assignment order
- `trail_limits` stores the trail index where each decision level starts

### Propagation

Propagation is still naive in this iteration: the solver scans every clause until it reaches a fixed point.

That is slower than watched literals, but it keeps the CDCL implementation straightforward:

- satisfied clause => skip
- all literals false => conflict
- exactly one unassigned literal and the rest false => imply that literal

### Conflict Analysis

On conflict, the solver analyzes the current conflict clause and resolves backwards through reason clauses until only one current-level literal remains in the learned clause.

The resulting learned clause is:

- added to the clause database
- used to compute the backjump level
- made asserting by enqueueing its first literal after backtracking

This is the key difference from `01`: the solver no longer blindly flips the most recent branch. It learns from the conflict and jumps directly to the highest useful earlier level.

## Interface Contract

Like every iteration in this repo:

- `build.sh` builds the solver
- `run.sh <cnf_path> <output_dir>` runs the solver
- stdout follows SAT Competition 2025 format
- UNSAT writes `proof.out` into the given output directory

The outer interface is unchanged from `01`, so the shared tooling (`tools/smoke_test.sh`, `tools/bench.sh`) can run this solver exactly the same way.

## Validation

Completed checks for this iteration:

- `cargo test` — 10/10 unit tests passed
- `bash tools/smoke_test.sh solver/02-cdcl` — 9/9 smoke tests passed
- All 5 UNSAT smoke-test proofs verified with `drat-trim`

## Code-Level Optimization Log

The profiling suite was refreshed before optimization so the Feistel instances were no longer trivial:

- `feistel_b64_k32_r12`
- `feistel_b64_k32_r14`
- `feistel_b64_k32_r16`
- `random_v110_s1`
- `random_v130_s3`
- `random_v140_s1`

Baseline before code-level tuning:

- **PAR-2:** `384.161`
- **Solved:** `5/6`

Successful optimization attempts kept in the solver:

1. **Reason sentinel instead of `Option<usize>`**
   Replaced per-variable `Option<usize>` reason tracking with a plain `usize` plus `NO_REASON` sentinel.
   PAR-2 improved from `384.161` to `382.088`.

2. **Reusable conflict-analysis scratch buffers**
   Moved `seen`, `resolved`, and `learned` scratch storage into the solver so conflict analysis stops allocating fresh vectors on every conflict.
   PAR-2 improved from `382.088` to `379.009`.

3. **Direct clause scanning inside `propagate()`**
   Removed the extra `ClauseState` layer in the propagation hot path and scanned clauses directly inside `propagate()`.
   PAR-2 improved from `379.009` to `367.450`.

4. **Branch cursor for first-unassigned selection**
   Cached the lowest plausible unassigned variable index and reset it on backtrack, preserving the same branching rule while avoiding repeated scans from variable `1`.
   PAR-2 improved from `367.450` to `364.486`.

Additional attempts were benchmarked and reverted because they did not improve PAR-2. Those included:

- inlining `enqueue` logic
- narrowing `decision_level` to `u32`
- advancing the branch cursor past the chosen variable
- preallocating `trail_limits`
- enlarging the learned-clause scratch buffer
- forcing `mark_clause_literals()` inline

Additional optimization round on 2026-04-13, starting from the existing optimized solver:

- **Round baseline:** `375.663` PAR-2, `5/6` solved

5. **Flat clause storage in one contiguous literal buffer**
   Replaced `Vec<Box<[i32]>>` with lightweight clause refs into a shared `clause_data` buffer, reducing clause allocation overhead and improving scan locality.
   PAR-2 improved from `375.663` to `370.471`.

6. **Proof clause indices instead of cloned proof clauses**
   Logged learned-clause indices plus an empty-clause flag for DRAT output so proof generation no longer clones learned clauses.
   PAR-2 improved from `370.471` to `369.990`.

7. **Raw-pointer assignment loads in `propagate()`**
   Switched the propagation inner loop to read assignments through a cached raw pointer while walking clause literals with pointer arithmetic.
   PAR-2 improved from `369.990` to `364.905`.

8. **Larger reusable learned-clause scratch buffer**
   Increased the reusable `scratch_learned` capacity from `8` to `16` to reduce capacity growth during conflict analysis.
   PAR-2 improved from `364.905` to `364.794`.

Additional attempts in this round were benchmarked and reverted because they did not improve PAR-2. Those included:

- raw-pointer decision/reason loads in conflict analysis
- raw-pointer backtrack clears
- preallocating `trail_limits`
- advancing the branch cursor past the chosen variable
- forcing `mark_clause_literals()` inline
- scanning `pick_branch_lit()` through a raw pointer

## Profiling Benchmark Results

Environment:

- **CPU:** AMD Ryzen 5 5600 6-Core (12 threads), 3.5 GHz base / 4.47 GHz boost
- **RAM:** 64 GB DDR4
- **OS:** Ubuntu 22.04
- **Benchmark suite:** `benchmarks/profiling`
- **Command:** `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/02-cdcl`

Results:

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r12 | crypto | SAT | 3.879s |
| feistel_b64_k32_r14 | crypto | SAT | 49.055s |
| feistel_b64_k32_r16 | crypto | TIMEOUT | 120.000s |
| random_v110_s1 | 3-SAT | UNSAT | 2.922s |
| random_v130_s3 | 3-SAT | SAT | 21.302s |
| random_v140_s1 | 3-SAT | UNSAT | 47.636s |

**PAR-2: 364.794 (5/6 solved)**

This is a `19.367` point PAR-2 improvement over the refreshed-suite baseline, about `5.0%` better without changing the algorithmic structure of the solver. The 2026-04-13 optimization round alone improved PAR-2 by `10.869` points from its measured starting point. The remaining work for later iterations should focus on reducing propagation cost further and improving branching quality.
