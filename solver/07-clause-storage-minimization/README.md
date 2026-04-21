# 07-clause-storage-minimization

This iteration takes the `06-clause-storage` watcher/layout rewrite and adds runtime conflict-clause
minimization.

## Current State

`07` includes:

- the full `06` clause storage and blocker-watcher rewrite
- runtime clause minimization modes: `none`, `basic`, and `deep`
- proof-sound minimization restricted to original-clause reason chains
- a regression test that keeps literals whose support escapes the learned source set
- runtime default `ccmin_mode = deep`
- `SAT_CCMIN_MODE=none|basic|deep` override for benchmarking and debugging

## What Changed

This pass extends `06` with MiniSat-style conflict-clause shrinking:

- added basic and recursive redundancy checks during conflict analysis
- made the runtime path proof-sound by refusing to trust learned-clause reasons during
  minimization
- kept copied proof logging so DRAT output stays stable under in-place clause mutation

The result is a solver that keeps the faster clause/watcher hot path from `06` and gets a further
search reduction from safe clause minimization.

## Validation

- `cargo test` — `24/24`
- `bash tools/smoke_test.sh solver/07-clause-storage-minimization` — `9/9`

## Profiling Benchmark Result

Profiling run on 2026-04-20:

- Command: `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/07-clause-storage-minimization`
- Result: `PAR-2 22.953`
- Solved: `6/6`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r17 | crypto | SAT | 1.220s |
| feistel_b64_k49_r15 | crypto | SAT | 4.087s |
| feistel_b64_k57_r14 | crypto | SAT | 0.314s |
| random_v229_s2 | 3-SAT | UNSAT | 4.850s |
| random_v240_s3 | 3-SAT | UNSAT | 4.649s |
| random_v241_s4 | 3-SAT | UNSAT | 7.834s |

Compared with `06`, this iteration cuts the profiling PAR-2 from `68.668` to `22.953`.
