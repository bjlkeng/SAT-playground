# 11-kissat-innovations

Kissat-inspired solver experiments on top of `10-bve-preprocess`.

This iteration started as a direct copy of solver `10`, minus the standalone MiniSat `simp` port
notes. It is now the development branch for Kissat-inspired infrastructure experiments on top of
the solver-10 CDCL core: MiniSat-style root preprocessing, bounded variable elimination, backward
subsumption / self-subsuming resolution, DRAT proof output, model extension after eliminated
variables, compact reason references, binary-clause metadata, and aggressive binary-first
propagation experiments.

## Current State

What is present:

- the full `10-bve-preprocess` CDCL and preprocessing implementation
- SAT Competition 2025 `build.sh` / `run.sh` interface compatibility
- release-profile and `target-cpu=native` build settings used by previous iterations
- unit and smoke-test coverage inherited from solver `10`
- Phase 0 Kissat-roadmap observability: extended counters, trace output, proof byte/clause metrics,
  GC/deletion timing, and optional `SAT_CHECK_INVARIANTS=1` consistency checks
- aggressive binary-first propagation, with arena binary clauses kept for proof/conflict material
- production auto-enabled static original-binary implication segments for binary-heavy formulas;
  use `SAT_BINARY_STATIC_SEGMENTS=off` to disable or `on` to force them
- Phase 3 root-maintenance scaffolding: a central scheduler, a proof-safe
  `return_to_root_for_maintenance` transition, and diagnostic no-op hooks for future reorder,
  rephase, probe, and eliminate passes
- Phase 4 dense/sparse simplification boundary around the existing upfront MiniSat-style
  simplifier: explicit `enter_simplification_mode` / `resume_search_mode_after_simplification`
  hooks, dense occurrence-view lifetime counters, and sparse-search cleanup
- Phase 5 learned-clause lifecycle policy: glue/tier/used-based reduction is the default; use
  `SAT_REDUCE_MODE=activity` for the previous activity-based reducer fallback
- Phase 6 search-mode infrastructure: stable heap search remains the default, while
  `SAT_SEARCH_MODE=focused` enables a focused recent-conflict decision queue for measurement
- Phase 7 restart/reduction-pressure scaffold: `SAT_RESTART_MODE=glue-ema` enables a fast/slow
  learned-glue EMA restart trigger, and `SAT_REDUCE_LOW_YIELD_COOLDOWN=<conflicts>` enables an
  opt-in cooldown after low-yield learned-clause reduction passes; stable/Luby remains the default

What is intentionally not present yet:

- no automatic Kissat search-mode switching, restart trail reuse, real mid-search inprocessing pass,
  probing, or rephasing yet
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

Static original-binary implication validation on 2026-05-10:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
```

Results:

- `cargo test`: 61 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-10-16-04-06`
- forced static-segment invariant smoke:
  `SAT_BINARY_STATIC_SEGMENTS=on SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-innovations`,
  9/9 passed, log `log/2026-05-10-16-05-06`
- fixed-window perf checks are recorded in `kissat.md`; high-binary Velev enabled static segments
  automatically and reached the same 200k-conflict trace about 8% faster, while mixed/no-binary
  guard instances stayed disabled

Root-maintenance scheduler validation on 2026-05-10:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
```

Results:

- `cargo test`: 64 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-10-17-20-27`
- fresh pre-change profiling baseline: solved 5/11, PAR-2 `1561.642`, log
  `log/bench-11-kissat-innovations-2026-05-10-17-01-46`
- post-change profiling run: solved 5/11, PAR-2 `1559.767`, log
  `log/bench-11-kissat-innovations-2026-05-10-17-20-43`
- same solved/timeout split; per-instance deltas were small and consistent with run-to-run noise

Dense/sparse simplification boundary validation on 2026-05-10:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-innovations
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
```

Results:

- `cargo test`: 67 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-10-18-34-16`
- invariant smoke suite: 9/9 passed, log `log/2026-05-10-18-53-12`
- fresh pre-change profiling baseline: solved 5/11, PAR-2 `1559.776`, log
  `log/bench-11-kissat-innovations-2026-05-10-18-15-21`
- post-change profiling run: solved 5/11, PAR-2 `1559.650`, log
  `log/bench-11-kissat-innovations-2026-05-10-18-34-31`
- same solved/timeout split; all non-timeout per-instance deltas were under `0.1s`

Glue-tiered learned reduction validation on 2026-05-10:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_REDUCE_MODE=activity bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_REDUCE_MODE=activity bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_REDUCE_MODE=glue-tiered bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
```

Results:

- `cargo test`: 70 passed
- default glue-tiered smoke suite: 9/9 passed, log `log/2026-05-10-21-08-46`
- activity fallback smoke suite: 9/9 passed, log `log/2026-05-10-21-08-55`
- glue-tiered invariant smoke suite: 9/9 passed, log `log/2026-05-10-20-41-12`
- activity fallback profiling run: solved 5/11, PAR-2 `1559.855`, log
  `log/bench-11-kissat-innovations-2026-05-10-19-29-34`
- glue-tiered/default profiling run: solved 6/11, PAR-2 `1328.592`, log
  `log/bench-11-kissat-innovations-2026-05-10-20-17-39`
- net profile-set improvement: `-231.263` PAR-2 and one extra solved instance

Key per-instance deltas versus activity fallback mode:

- `feistel_b64_k32_r22`: SAT `90.071s` -> `27.015s`
- `random_v355_s3`: TIMEOUT -> SAT `53.410s`
- regressions: `feistel_b64_k52_r17` `+4.290s`, `feistel_b64_k57_r18` `+2.314s`,
  `random_v285_s2` `+0.746s`, `random_v292_s4` `+11.033s`
- timeout-heavy structured instances stayed timeouts under both modes

Important rejected/adjusted attempt:

- The first glue-tiered cut deleted only half of the immediately eligible clauses. On
  `random_v292_s4`, direct trace showed reduction churn: activity mode used `991` reduce passes
  and solved in `8.557s`, while the primary-only glue cut used about `75k` reduce passes and took
  over `21s`.
- The kept version uses budget pressure: primary reducibles are still tier `>= 3` or unused tier-2
  clauses, but if they cannot bring the learned DB back toward budget, used tier-2 clauses become
  pressure candidates. The focused trace after that change solved `random_v292_s4` in `19.066s`
  with `3261` reductions. That is still a search-path regression on this instance, but avoids the
  pathological reducer churn and the full profiling set improves materially.

Focused search-mode infrastructure validation on 2026-05-10:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_SEARCH_MODE=focused SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-innovations
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_SEARCH_MODE=focused bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
```

Results:

- `cargo test`: 73 passed
- default stable smoke suite: 9/9 passed, log `log/2026-05-10-21-31-59`
- focused invariant smoke suite: 9/9 passed, log `log/2026-05-10-21-32-04`
- default stable profiling run: solved 6/11, PAR-2 `1329.605`, log
  `log/bench-11-kissat-innovations-2026-05-10-21-32-20`
- focused profiling run: solved 2/11, PAR-2 `2288.046`, log
  `log/bench-11-kissat-innovations-2026-05-10-21-52-31`
- stable default stayed in line with the previous glue-default profile, so the focused queue
  infrastructure did not show an obvious default hot-path regression

Focused-mode analysis:

- Focused-only is not ready as a default policy. It timed out on `feistel_b64_k32_r22`,
  `feistel_b64_k52_r17`, `random_v285_s2`, and `random_v292_s4`, all solved by stable mode.
- On `feistel_b64_k57_r18`, direct no-proof trace shows stable solved in `3.992s` with `243188`
  conflicts, `304380` decisions, `18090356` propagations, `90` reduce passes, and `112.844ms`
  reduction time.
- The same direct trace in focused mode solved in `61.275s` with `933578` conflicts, `1062833`
  decisions, `81748523` propagations, `191417` reduce passes, and `32077.625ms` reduction time.
- That points to a search trajectory and learned-DB pressure problem, not just local queue overhead.
  The next focused-related work should be glue-EMA restarts/trail reuse or a reduction throttle
  before considering focused mode as anything other than an opt-in experiment.

Glue-EMA restart and reduction-pressure scaffold validation on 2026-05-10:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_SEARCH_MODE=focused SAT_RESTART_MODE=glue-ema SAT_REDUCE_LOW_YIELD_COOLDOWN=100 \
  SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-innovations
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_SEARCH_MODE=focused SAT_RESTART_MODE=glue-ema SAT_REDUCE_LOW_YIELD_COOLDOWN=100 \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
```

Results:

- `cargo test`: 76 passed
- default stable smoke suite: 9/9 passed, log `log/2026-05-10-23-55-40`
- focused glue-EMA invariant smoke suite: 9/9 passed, log `log/2026-05-10-23-55-49`
- default stable profiling run: solved 6/11, PAR-2 `1328.185`, log
  `log/bench-11-kissat-innovations-2026-05-10-23-11-30`
- focused glue-EMA plus low-yield cooldown profiling run: solved 2/11, PAR-2 `2185.711`, log
  `log/bench-11-kissat-innovations-2026-05-10-23-32-47`
- default stable remains in line with the previous stable run (`1329.605`), so the additional
  counters and default-off restart/cooldown hooks did not show an obvious stable hot-path
  regression

Focused glue-EMA analysis:

- The focused stack improved versus the previous focused-only profile (`2288.046` -> `2185.711`
  PAR-2) but did not improve solved count. It still timed out on `feistel_b64_k32_r22`,
  `feistel_b64_k52_r17`, `random_v285_s2`, and `random_v292_s4`, all of which stable solved.
- It materially improved the two focused-solved instances: `feistel_b64_k57_r18` `61.879s` ->
  `9.344s`, and `random_v355_s3` `66.167s` -> `16.367s`.
- Direct trace on `feistel_b64_k57_r18`:
  - stable default: `4.034s`, `243188` conflicts, `304380` decisions, `18090356` propagations,
    `90` reduce passes, `113.286ms` reduction time
  - focused Luby plus cooldown only: `28.092s`, `881956` conflicts, `1006564` decisions,
    `75918462` propagations, `2987` reduce passes, `1008.760ms` reduction time
  - focused glue-EMA without cooldown: `13.519s`, `363740` conflicts, `408549` decisions,
    `31012613` propagations, `33768` reduce passes, `4135.205ms` reduction time
  - focused glue-EMA plus cooldown: `9.355s`, `349623` conflicts, `392584` decisions,
    `30174167` propagations, `551` reduce passes, `224.440ms` reduction time
- Interpretation: glue-EMA restarts fix a substantial part of focused mode's search trajectory on
  at least one Feistel case, and the cooldown fixes the reduction churn exposed by that better
  trajectory. The remaining gap to stable is still search quality: focused does more conflicts,
  decisions, and propagations even when reduction overhead is controlled.
