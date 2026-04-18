# 05-restarts

First restart iteration built on top of `04-vsids`.

This version keeps the watched-literal CDCL core and VSIDS branching from `04`, then adds a simple Luby restart policy plus phase saving:

- conflicts are counted globally within each restart window
- after a fixed number of conflicts, the solver schedules a restart
- the next restart budget is chosen from the Luby sequence
- restarting backtracks to decision level 0 while keeping learned clauses
- each variable remembers its last assigned polarity and reuses it when branching again

## Scope of This First Pass

This is a correctness-first restart iteration.

Included:
- watched-literal BCP from `03`
- VSIDS branching from `04`
- simple Luby restarts on top of the existing CDCL loop
- saved variable polarities reused after backtrack and restart

Not included yet:
- clause deletion / database management

The goal of `05` is to add a minimal standard restart policy without otherwise changing the search architecture.

## Design Notes

`05` keeps the same propagation, conflict analysis, proof logging, and VSIDS activity scheme as `04`.

The search-loop changes are restart scheduling plus saved polarities:

1. every non-root conflict increments a per-window conflict counter
2. once the counter reaches the current budget, the solver marks a pending restart
3. the next restart budget advances along the Luby sequence
4. before the next branch decision, the solver backtracks to level 0 if a restart is pending
5. the next branch on a variable uses that variable's remembered last value instead of always choosing positive

This is deliberately simple:

- learned clauses are retained across restarts
- root assignments are preserved by the existing `backtrack(0)` logic
- saved polarity is updated whenever a variable is assigned
- no clause-database policy is added yet

The implementation is meant to isolate the effect of “try the search again from the top with the clauses you just learned” before adding more advanced restart machinery.

## Validation

Completed checks for this first pass:

- `cargo test` — 19/19 unit tests passed
- `bash tools/smoke_test.sh solver/05-restarts` — 9/9 smoke tests passed

The new unit tests added for `05` check that:

- the Luby helper produces the expected `1, 1, 2, 1, 1, 2, 4, ...` sequence
- hitting the restart conflict budget schedules a restart and advances the next Luby window
- applying a pending restart backtracks to level 0 while preserving root assignments
- branch selection reuses the saved polarity for the chosen variable
- saved polarity survives backtrack and is reused on the next decision

## Code-Level Optimization Log

Optimization pass run on 2026-04-18 against the current `benchmarks/profiling` suite for `05-restarts`.

Round baseline before tuning:

- **PAR-2:** `172.954`
- **Solved:** `6/6`

Measurement note:

- Ten one-change optimization attempts were benchmarked in total.
- Only the successful changes below were kept in the current solver state.

Successful optimization attempts kept in the solver:

1. **Removed dead `branch_rank` / `branch_cursor` bookkeeping**
   `05` still inherited reverse-order bookkeeping from `04`, but the current branch picker scans all of `branch_order` anyway. Removing that maintenance work from backtracking produced a small win.
   PAR-2 improved from `172.954` to `171.863`.

2. **Cached `current_level()` inside `enqueue()`**
   Assignments were reading the current level twice on every enqueue. Reusing a single local copy reduced hot-path overhead enough to matter on the profiling suite.
   PAR-2 improved from `171.863` to `166.188`.

3. **Cleared conflict-analysis scratch buffers with `write_bytes`**
   The `scratch_seen` and `scratch_resolved` arrays are zeroed on every conflict analysis. Replacing two slice `fill(0)` calls with direct `write_bytes` produced the final improvement.
   PAR-2 improved from `166.188` to `164.794`.

Additional attempts were benchmarked and reverted because they did not improve PAR-2:

- raw-pointer `saved_phase` loads in `pick_branch_lit()`
- raw-pointer traversal of `branch_order`
- shrinking `decision_level` to `u32`
- shrinking `trail_limits` to `u32`
- shrinking `reason` to `u32`
- forcing `#[inline(always)]` on the small restart helpers
- shrinking `proof_clause_indices` to `u32`

## Restart-Unit Sweep

Follow-up tuning run on 2026-04-18 varied the base Luby restart unit after the code-level cleanup above.

Results on `benchmarks/profiling`:

- `restart_unit = 8` -> `PAR-2 172.298`
- `restart_unit = 16` -> `PAR-2 164.791`
- `restart_unit = 32` -> `PAR-2 173.182`
- `restart_unit = 64` -> `PAR-2 151.453`
- `restart_unit = 128` -> `PAR-2 213.119`
- `restart_unit = 1_000_000_000` -> `PAR-2 134.319`

The best observed setting on the current profiling suite was an effectively disabled restart schedule (`restart_unit = 1_000_000_000`).

For comparison against `04-vsids`, the solver was then reset to the original `restart_unit = 32` configuration before the latest validation run below.

## Profiling Benchmark Results

Environment:

- **CPU:** AMD Ryzen 5 5600 6-Core (12 threads), 3.5 GHz base / 4.47 GHz boost
- **RAM:** 64 GB DDR4
- **OS:** Ubuntu 22.04
- **Benchmark suite:** `benchmarks/profiling`
- **Command:** `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/05-restarts`

Current validation results for the restored restart + optimized phase-saving version:

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r17 | crypto | SAT | 1.25s |
| feistel_b64_k49_r15 | crypto | SAT | 28.46s |
| feistel_b64_k57_r14 | crypto | SAT | 1.23s |
| random_v229_s2 | 3-SAT | UNSAT | 51.49s |
| random_v240_s3 | 3-SAT | UNSAT | 39.03s |
| random_v241_s4 | 3-SAT | UNSAT | 44.25s |

**PAR-2: 165.717 (6/6 solved)**

For comparison during this iteration:

- first geometric restart pass: `PAR-2 324.750`
- Luby-only restart pass: `PAR-2 223.194`
- Luby + phase saving baseline: `PAR-2 172.954`
- optimized Luby + phase saving: `PAR-2 164.794`
- tuned Luby + phase saving (`restart_unit = 64`): `PAR-2 151.453`
- effectively disabled restarts (`restart_unit = 1_000_000_000`): best observed `PAR-2 134.319`, final revalidation `134.909`
- restored original restart schedule (`restart_unit = 32`): final revalidation `165.717`
