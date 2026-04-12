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
- `reason[v]` stores the clause index that implied `v`, or `None` for decisions
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

- `cargo test` — 9/9 unit tests passed
- `bash tools/smoke_test.sh solver/02-cdcl` — 8/8 smoke tests passed
- All 4 UNSAT smoke-test proofs verified with `drat-trim`

## Profiling Benchmark Baseline

Environment:

- **CPU:** AMD Ryzen 5 5600 6-Core (12 threads), 3.5 GHz base / 4.47 GHz boost
- **RAM:** 64 GB DDR4
- **OS:** Ubuntu 22.04
- **Benchmark suite:** `benchmarks/profiling`
- **Command:** `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/02-cdcl`

Results:

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r10 | crypto | SAT | 0.165s |
| feistel_b64_k32_r12 | crypto | SAT | 4.720s |
| feistel_b64_k32_r8 | crypto | SAT | 0.029s |
| random_v110_s1 | 3-SAT | UNSAT | 3.184s |
| random_v130_s3 | 3-SAT | SAT | 23.620s |
| random_v140_s1 | 3-SAT | UNSAT | 54.132s |

**PAR-2: 85.850 (6/6 solved)**

This already outperforms `01-naive-dpll` by a large margin even without watched literals or heuristic tuning. The remaining work for later iterations should focus on reducing propagation cost and improving branching quality, not reworking the overall search model again.
