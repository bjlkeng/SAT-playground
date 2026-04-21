# 06-clause-storage-minimization

This iteration started from a clean `05-restarts` baseline, then rewired the hot clause and
watcher path toward a MiniSat-style layout.

## Current State

Right now `06` includes:

- watched-literal BCP from `03`
- analysis-time EVSIDS variable bumps from `04`
- indexed-heap branch queue with occurrence-order tie-breaks
- Luby restarts with `restart_unit = 100`
- phase saving across backtracks and restarts
- in-clause watched literals stored in clause slots `0` and `1`
- watcher entries with MiniSat-style `{ clause, blocker }` metadata
- clause metadata extended toward MiniSat-style headers
- proof logging by copied learned clauses so in-place watcher swaps do not corrupt DRAT output
- red/green tests for basic and deep clause minimization behavior
- runtime default `ccmin_mode = none` for now

The runtime minimization modes are implemented and unit-tested, but they are not enabled by default
yet because larger profiling proofs were not sound under the first pass. The safe end state of this
round is “MiniSat-style watcher/storage changes landed, clause minimization code present, default
runtime mode still disabled pending a soundness fix.”

## What Changed

This pass focused on the MiniSat-like hot path, not clause-db reduction:

- `watch_pos` was removed in favor of the MiniSat invariant that watched literals live in clause
  positions `0` and `1`
- propagation now uses blocker watchers and the usual MiniSat sequence:
  blocker fast path, normalize false watch into slot `1`, scan from `2..`, then unit/conflict
- clause metadata now carries enough structure to keep moving toward a MiniSat-like clause store
- conflict analysis now has explicit `ccmin_mode = 0/1/2` support with tests for both direct and
  recursive redundancy removal
- proof logging was changed to copy learned clauses at insertion time because the clause bodies now
  mutate during watch movement

The main remaining gap is getting runtime clause minimization sound on larger proofs, then deciding
whether `basic` or `deep` should become the default.

## Validation

Current validation after the watcher/storage refactor:

- `cargo test` — 23/23 unit tests passed
- `bash tools/smoke_test.sh solver/06-clause-storage-minimization` — 9/9 smoke tests passed

## Profiling Benchmark Result

Current safe runtime configuration:

- `ccmin_mode = none`
- MiniSat-style blocker watchers enabled
- copied proof logging enabled

Profiling run on 2026-04-20:

- Command: `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/06-clause-storage-minimization`
- Result: `PAR-2 65.395`
- Solved: `6/6`

Per-instance result:

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r17 | crypto | SAT | 1.00s |
| feistel_b64_k49_r15 | crypto | SAT | 13.15s |
| feistel_b64_k57_r14 | crypto | SAT | 1.50s |
| random_v229_s2 | 3-SAT | UNSAT | 10.06s |
| random_v240_s3 | 3-SAT | UNSAT | 14.87s |
| random_v241_s4 | 3-SAT | UNSAT | 24.81s |

Compared with the earlier reset baseline (`PAR-2 119.334`), this pass improved the profiling suite
substantially through the watcher/storage rewrite alone, even with runtime clause minimization
still disabled by default.
