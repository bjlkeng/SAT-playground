# 06-clause-db-mgmt

This iteration has been reset to a clean `05-restarts` baseline.

The previous clause-database-management experiment was removed so `06` can restart from the
known-good `05` solver and shift focus to a different next step.

## Current Baseline

Right now `06` intentionally matches the `05` solver behavior:

- watched-literal BCP from `03`
- analysis-time EVSIDS variable bumps from `04`
- indexed-heap branch queue with occurrence-order tie-breaks
- Luby restarts with `restart_unit = 100`
- phase saving across backtracks and restarts
- no clause deletion / clause-database management yet
- no MiniSat-style conflict-clause minimization yet

This reset is deliberate. The goal is to preserve the strong `05` search behavior before changing
the internal representation and conflict-analysis details.

## New Focus for `06`

The next `06` implementation pass will not be about learned-clause deletion policy first.

Instead, the planned work is:

- more efficient clause storage, closer to MiniSat's clause arena / watcher-friendly layout
- MiniSat-style conflict-clause minimization after first-UIP analysis
- supporting data-structure changes needed to make the above efficient in Rust

Clause-database reduction may still come later, but it is no longer the first target for this
iteration.

## Design Intent

The reset is based on the debugging results from the earlier `06` experiments:

- simplify-style root cleanup did not appear to be the main regression source
- learned-clause deletion policy was highly sensitive to clause quality and maintenance overhead
- the current solver likely needs better learned clauses and cheaper clause handling before
  aggressive database reduction is worth revisiting

So the plan for `06` is:

1. keep the `05` search loop as the baseline
2. improve clause representation and propagation-local storage
3. add MiniSat-style learned-clause minimization
4. only then revisit learned-clause deletion policy if it is still needed

## Validation

After the reset, `06` should validate exactly like the copied `05` baseline:

- `cargo test`
- `bash tools/smoke_test.sh solver/06-clause-db-mgmt`
