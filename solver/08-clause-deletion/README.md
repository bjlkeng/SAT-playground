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
- clause deletion followed by compaction of the flat clause storage so deleted literals actually
  release runtime memory pressure
- append-only DRAT proof logging unchanged from `07`

## What Changed

This pass extends `07` with a first learned-clause database policy:

- added a `reason_refcount`-based locked-clause check instead of relying on a fixed asserting
  literal position
- added learned clause activity bump/decay plus a growing learned-database target
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

Profiling run pending. After the implementation commit lands, run:

```bash
bash tools/bench.sh -t 120 -d benchmarks/profiling solver/08-clause-deletion
```
