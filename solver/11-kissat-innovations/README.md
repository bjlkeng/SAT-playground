# 11-kissat-innovations

Kissat-inspired solver experiments on top of `10-bve-preprocess`.

This iteration starts as a direct copy of solver `10`, minus the standalone MiniSat `simp` port
notes. The current code is intentionally still the solver-10 implementation: a CDCL core with
MiniSat-style root preprocessing, bounded variable elimination, backward subsumption /
self-subsuming resolution, DRAT proof output, model extension after eliminated variables, and lazy
deleted-clause watcher cleanup.

## Current State

What is present at the fork point:

- the full `10-bve-preprocess` CDCL and preprocessing implementation
- SAT Competition 2025 `build.sh` / `run.sh` interface compatibility
- release-profile and `target-cpu=native` build settings used by previous iterations
- unit and smoke-test coverage inherited from solver `10`
- Phase 0 Kissat-roadmap observability: extended counters, trace output, proof byte/clause metrics,
  GC/deletion timing, and optional `SAT_CHECK_INVARIANTS=1` consistency checks

What is intentionally not present yet:

- no Kissat-specific technique has been added in solver `11`
- no new benchmark run has been recorded for this iteration
- no separate MiniSat `simp` port design document is copied forward

## Direction

Solver `11` is the branch point for measured experiments against ideas from Kissat-family solvers.
Candidate areas should be introduced one at a time and kept only when they improve a concrete
benchmark target while preserving the smoke suite and proof checking:

- search-mode and restart policy changes, including focused/stable-style behavior
- phase-selection and rephasing experiments
- clause-tiering / learned-clause retention policy refinements
- propagation and watch-list layout changes that are motivated by profiler data
- inprocessing ideas that can be validated against residual formula statistics before coding

Follow the repo's code-level optimization workflow for these experiments: pick a target, profile
before coding, measure opportunity size, keep changes only after a meaningful runtime improvement,
and record accepted and important rejected attempts here.

## Validation

Initial validation for the fork on 2026-05-09:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
```

Results:

- `cargo test` in `solver/11-kissat-innovations`: 48 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-09-17-09-03`

Phase 0 observability validation on 2026-05-09:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-innovations
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
```

Results:

- `cargo test`: 50 passed
- normal smoke suite: 9/9 passed, log `log/2026-05-09-17-54-43`
- invariant smoke suite: 9/9 passed, log `log/2026-05-09-17-54-52`
- profiling baseline before instrumentation: PAR-2 `1100.087`, solved 7/11, log
  `log/bench-11-kissat-innovations-2026-05-09-17-38-22`
- profiling after instrumentation: PAR-2 `1098.830`, solved 7/11, log
  `log/bench-11-kissat-innovations-2026-05-09-17-55-01`
- same solved/timeout split; measured runtime difference was within normal run-to-run noise
