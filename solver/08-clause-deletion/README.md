# 08-clause-deletion

This iteration builds on `07-clause-storage-minimization` and adds MiniSat-style learned clause
database reduction.

## Current State

`08` includes:

- the full `07` watched-literal CDCL core, EVSIDS variable branching, Luby restarts, and safe
  conflict-clause minimization
- an explicit learned-clause list with per-clause activity scores
- clause-activity bumping for learned clauses that participate in conflict analysis
- MiniSat-style `reduce_db()` behavior that keeps binary clauses and currently locked reason clauses
- a conservative learned-clause budget (`8x` original clause count) so reduction does not
  destabilize search on the profiling set
- `SAT_MAX_LEARNTS_OVERRIDE=<n>` override for benchmarking and deletion-policy tuning
- lazy watcher cleanup for deleted clauses, with watcher lists rebuilt during compaction instead of
  scanned on every clause removal
- clause deletion followed by compaction of the flat clause storage so deleted literals actually
  release runtime memory pressure
- append-only DRAT proof logging unchanged from `07`

## What Changed

This pass extends `07` with a first learned-clause database policy:

- added a `reason_refcount`-based locked-clause check instead of relying on a fixed asserting
  literal position
- added learned clause activity bump/decay plus a growing learned-database target
- raised the initial learned-clause budget well above MiniSat's default because this solver's
  proof-sound minimization keeps weaker learned clauses and regressed badly under aggressive early
  reduction
- stopped eagerly filtering watcher vectors during clause deletion; deleted clauses now stay
  tombstoned until the next compaction pass rebuilds the watcher lists
- added learned-clause reduction that deletes low-activity unlocked clauses while preserving binary
  learned clauses
- added clause-storage compaction so the append-only `clause_data` buffer is rebuilt after
  deletions, remapping reasons and watchers onto the compacted clause indices

The main goal of `08` is to keep the learned database from growing without bound, which is the
next structural step after `07`'s faster clause storage and clause minimization.

## Validation

- `cargo test` — `27/27`
- `bash tools/smoke_test.sh solver/08-clause-deletion` — `9/9`
- All 5 UNSAT smoke-test proofs verified with `drat-trim`

## Profiling Benchmark Result

Profiling run on 2026-04-23:

- Command: `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/08-clause-deletion`
- Result: `PAR-2 15.261`
- Solved: `6/6`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r17 | crypto | SAT | 1.259s |
| feistel_b64_k49_r15 | crypto | SAT | 4.257s |
| feistel_b64_k57_r14 | crypto | SAT | 0.317s |
| random_v229_s2 | 3-SAT | UNSAT | 3.852s |
| random_v240_s3 | 3-SAT | UNSAT | 2.669s |
| random_v241_s4 | 3-SAT | UNSAT | 2.906s |

Compared with `07`, this iteration cuts the profiling PAR-2 from `20.736` to `15.261`.
