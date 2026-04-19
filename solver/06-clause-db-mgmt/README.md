# 06-clause-db-mgmt

First clause-database-management iteration built on top of `05-restarts`.

This version keeps the watched-literal CDCL core, VSIDS branching, Luby restarts, and phase saving from `05`, then adds a basic learned-clause management policy driven by LBD scoring:

- each learned clause gets an `LBD` value when it is created
- learned clauses are bucketed into `core`, `mid`, or `local` tiers
- only `local` learned clauses are eligible for deletion
- periodic reduction passes mark removable local clauses as deleted
- clauses that are currently locking a trail assignment are never deleted

## Scope of This First Pass

This is a correctness-first implementation of clause-database management.

Included:

- learned-clause `LBD` computation from distinct non-root decision levels
- simple three-tier clause classification
- a periodic learned-clause reduction pass
- conservative protection for locked clauses that are still acting as reasons

Not included yet:

- LBD refresh / promotion of older clauses
- physical compaction of deleted clauses out of storage
- watch-list cleanup during deletion
- benchmark tuning of reduction thresholds or tier cutoffs

The goal of `06` is to introduce the core policy decisions first: which learned clauses are considered important, and which ones can be dropped without breaking correctness.

## Design Notes

### LBD scoring

For each learned clause, `06` counts how many distinct non-root decision levels appear among its literals at learning time.

- lower `LBD` clauses are treated as more reusable
- binaries are kept as `core` automatically
- current tier thresholds are:
  - `core`: clause length `<= 2` or `LBD <= 2`
  - `mid`: `LBD <= 6`
  - `local`: everything else

### Deletion policy

The current reducer is intentionally simple:

1. every learned clause starts active
2. after every 32 learned clauses added, the solver runs a reduction pass
3. the pass scans learned clauses and marks deletable `local` clauses inactive
4. `core`, `mid`, original, and locked clauses are retained

Deleted clauses are left in clause storage and proof history, but propagation skips them. That keeps the first implementation small and safe while still preventing inactive local clauses from participating in future search.

## Validation

Completed checks for this first pass:

- `cargo test` — 21/21 unit tests passed
- `bash tools/smoke_test.sh solver/06-clause-db-mgmt` — 9/9 smoke tests passed

The new unit tests added for `06` check that:

- `LBD` counts distinct non-root decision levels rather than raw clause length
- the reducer keeps `core` clauses
- the reducer keeps locked `local` clauses that still justify a trail assignment
- removable `local` clauses are marked deleted
