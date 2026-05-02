# 10-simp-foundation

This iteration starts as a direct copy of `09-root-simp-opts`. It is a clean baseline for the next
MiniSat-style simplification or preprocessing experiment.

## Current State

`10` currently has no algorithmic changes relative to `09`. It inherits:

- watched-literal BCP with blocker fast paths
- EVSIDS-style variable activity and saved-phase branching
- conflict-clause minimization modes: `none`, `basic`, and `deep`
- deep minimization through learned-clause reasons
- MiniSat-style learned-clause activity bumps and learned-clause reduction thresholds
- a MiniSat-style packed clause arena with stable clause refs and relocating GC
- streamed proof logging through a fixed 16 MiB byte buffer into `proof.out.tmp`
- root-level `simplify()` that deletes satisfied clauses and trims root-false literals from
  surviving original clauses
- the profiled `09` hot-path cleanup: lazy branch-heap cleanup, bottom-up heap rebuilds, in-place
  watcher compaction, in-place learned-clause reduction, scratch-buffer conflict analysis, and the
  learned-unit shortcut

## What Changed

- copied `09-root-simp-opts` into a new self-contained iteration directory
- renamed the package / iteration metadata for `10-simp-foundation`
- left `src/main.rs`, `build.sh`, and `run.sh` behavior unchanged

## Intended Focus

The next useful work is to close selected gaps between `09` and MiniSat `simp`, starting with a
small, measurable simplification pass before attempting full bounded variable elimination.

Candidate directions:

- maintain occurrence lists for original clauses
- add backward subsumption and simple self-subsuming resolution
- add bounded variable elimination for low-cost variables
- normalize clauses during parsing before the arena is built
- add benchmark instrumentation for simplification impact on active variables, clauses, literals,
  and propagation rate

## Validation

- `cargo test` — `39/39`
- `bash tools/smoke_test.sh solver/10-simp-foundation` — `9/9`

## Benchmark Baseline

No dedicated `10` benchmark has been run yet. Since this is a direct copy of `09`, use the latest
`09-root-simp-opts` benchmark results as the current behavioral baseline until `10` diverges.
