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

## Profiling Benchmark Results

Environment:

- **CPU:** AMD Ryzen 5 5600 6-Core (12 threads), 3.5 GHz base / 4.47 GHz boost
- **RAM:** 64 GB DDR4
- **OS:** Ubuntu 22.04
- **Benchmark suite:** `benchmarks/profiling`
- **Command:** `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/05-restarts`

Current validation results for the phase-saving version:

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r17 | crypto | SAT | 1.28s |
| feistel_b64_k49_r15 | crypto | SAT | 32.92s |
| feistel_b64_k57_r14 | crypto | SAT | 1.24s |
| random_v229_s2 | 3-SAT | UNSAT | 53.99s |
| random_v240_s3 | 3-SAT | UNSAT | 40.30s |
| random_v241_s4 | 3-SAT | UNSAT | 43.22s |

**PAR-2: 172.954 (6/6 solved)**

For comparison during this iteration:

- first geometric restart pass: `PAR-2 324.750`
- Luby-only restart pass: `PAR-2 223.194`
- Luby + phase saving: `PAR-2 172.954`
