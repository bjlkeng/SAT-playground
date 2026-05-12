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
- Phase 6 search-mode infrastructure: stable heap search remains the starting mode, while
  `SAT_SEARCH_MODE=focused` enables a focused recent-conflict decision queue for measurement
- Phase 7 restart/reduction-pressure scaffold: `SAT_RESTART_MODE=glue-ema` enables a fast/slow
  learned-glue EMA restart trigger, and `SAT_REDUCE_LOW_YIELD_COOLDOWN=<conflicts>` enables an
  opt-in cooldown after low-yield learned-clause reduction passes; Kissat-style restart trail reuse
  is now the default with the measured `SAT_RESTART_REUSE_CAP=8` setting; use
  `SAT_RESTART_REUSE=off` for ablations, or `SAT_RESTART_REUSE_CAP=0` for uncapped reuse
- Phase 7 phase-system scaffold: stable search can track `best_phase` and `target_phase` snapshots
  and runs a best/inverted/original rephase cycle by default with `SAT_REPHASE_INTERVAL=10000`; use
  `SAT_REPHASE_INTERVAL=0` to disable while comparing proof/search side effects
- Phase 7 search-control scaffold: stable-mode reluctant doubling restarts are now the default,
  falling back to glue-EMA behavior in focused mode. Root-safe guarded stable/focused switching is
  also enabled by default with `SAT_MODE_SWITCH_INTERVAL=50000` and
  `SAT_MODE_SWITCH_POLICY=stale-stable`; set `SAT_RESTART_MODE=luby` and
  `SAT_MODE_SWITCH_INTERVAL=0` for the previous default-search ablation. Focused phases return to
  stable after a short dwell cap (`SAT_MODE_SWITCH_FOCUSED_CONFLICTS`, automatic default cap
  `1000`)

What is intentionally not present yet:

- no real mid-search inprocessing pass, probing, or local-search walking rephase source yet
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

Restart trail-reuse validation on 2026-05-11:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_RESTART_REUSE=kissat SAT_RESTART_REUSE_CAP=8 SAT_CHECK_INVARIANTS=1 \
  bash tools/smoke_test.sh solver/11-kissat-innovations
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_RESTART_REUSE=kissat bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling \
  solver/11-kissat-innovations
SAT_RESTART_REUSE=kissat SAT_RESTART_REUSE_CAP=8 bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
```

Results:

- `cargo test`: 79 passed
- default stable smoke suite: 9/9 passed, log `log/2026-05-11-01-49-19`
- capped trail-reuse invariant smoke suite: 9/9 passed, log `log/2026-05-11-01-49-35`
- reuse-off profiling baseline after the code change: solved 6/11, PAR-2 `1332.547`, log
  `log/bench-11-kissat-innovations-2026-05-11-00-25-31`
- uncapped stable Kissat-reuse profiling run: solved 6/11, PAR-2 `1301.718`, log
  `log/bench-11-kissat-innovations-2026-05-11-00-46-24`
- capped stable Kissat-reuse profiling run (`SAT_RESTART_REUSE_CAP=8`): solved 6/11, PAR-2
  `1246.545`, log `log/bench-11-kissat-innovations-2026-05-11-01-24-01`

Key profiling deltas, capped reuse versus the reuse-off baseline:

- `feistel_b64_k32_r22`: SAT `27.60s` -> `0.59s`
- `feistel_b64_k52_r17`: SAT `18.42s` -> `4.28s`
- `feistel_b64_k57_r18`: SAT `4.85s` -> `2.43s`
- `SC25_Timetable...`: TIMEOUT -> SAT `15.58s`
- `random_v292_s4`: UNSAT `19.56s` -> `14.94s`
- regression: `random_v355_s3` SAT `53.35s` -> TIMEOUT

Heuristic sweep notes:

- The Kissat-like rule keeps a prefix while kept decision variables are better than the next
  decision candidate. In stable mode "better" means activity; in focused mode it means focused
  recency stamp.
- The now-removed `half`, `quarter`, and fixed-level reuse policies were rejected on
  `feistel_b64_k57_r18`: they reused far more levels and created large search regressions. For
  example stable `quarter` took `57.124s` and stable `half` took `53.417s`, versus `4.452s` with
  reuse off and `1.599s` with uncapped Kissat reuse.
- Uncapped Kissat reuse was useful but too volatile: it solved the timetable case but timed out on
  `random_v355_s3`. Caps were then swept on the sensitive cases. `CAP=8` was the best profile-set
  compromise: it retained the timetable win and Feistel wins, while the smaller `CAP=4` preserved
  `random_v355_s3` but lost the timetable win.
- The stable selector was optimized after the first sweep so it does not scan the whole heap on each
  restart. It now reuses the solver's existing lazy heap cleanup pattern, popping assigned heap roots
  until the best unassigned variable is available. Post-optimization target checks reproduced the
  same search counters on the key cap-8 cases.

Interpretation:

- Kissat-style reuse is now the default, using the measured cap-8 setting (`-86.002` PAR-2 versus
  the reuse-off post-change run). `SAT_RESTART_REUSE=off` remains for controlled comparisons, and
  `SAT_RESTART_REUSE_CAP=0` restores the uncapped Kissat selector.
- The setting is still path-sensitive: the cap-8 profile loses a previously solved random SAT
  instance to timeout while gaining large Feistel and timetable wins.
- The next restart-related step should be either mode switching/rephase support to recover from the
  `random_v355_s3` trajectory, or a conditional reuse guard that disables reuse when restart reuse
  is increasing conflicts/restarts without improving glue or propagation progress.

Default-reuse cleanup validation on 2026-05-11:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_RESTART_REUSE=off SAT_CHECK_INVARIANTS=1 \
  bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-innovations
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
```

Results:

- `cargo test`: 79 passed
- default smoke suite with Kissat reuse and cap 8: 9/9 passed, log `log/2026-05-11-07-33-13`
- reuse-off invariant smoke suite: 9/9 passed, log `log/2026-05-11-07-33-29`
- default invariant smoke suite: 9/9 passed, log `log/2026-05-11-07-33-36`
- default profiling run with Kissat reuse and cap 8: solved 6/11, PAR-2 `1247.040`, log
  `log/bench-11-kissat-innovations-2026-05-11-07-33-52`

The fresh default benchmark reproduced the earlier cap-8 profile within normal run noise:
`feistel_b64_k32_r22` `0.58s`, `feistel_b64_k52_r17` `4.24s`,
`feistel_b64_k57_r18` `2.43s`, timetable `16.48s`, `random_v285_s2` `8.62s`,
`random_v292_s4` `14.69s`, and the known `random_v355_s3` timeout.

Target/best rephase validation on 2026-05-11:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_REPHASE_INTERVAL=1 SAT_CHECK_INVARIANTS=1 \
  bash tools/smoke_test.sh solver/11-kissat-innovations
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_REPHASE_INTERVAL=50000 bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
```

Results:

- `cargo test`: 84 passed
- default smoke suite after the 10000 default-policy change: 9/9 passed, log
  `log/2026-05-11-12-53-25`
- default smoke suite with rephase interval 50000 before the later 10000 default-policy change:
  9/9 passed, log `log/2026-05-11-12-47-54`
- rephase-off invariant smoke suite: 9/9 passed, log `log/2026-05-11-12-48-06`
- default no-rephase profiling run after the implementation: solved 6/11, PAR-2 `1246.720`, log
  `log/bench-11-kissat-innovations-2026-05-11-09-15-19`
- full profiling run with `SAT_REPHASE_INTERVAL=50000`: solved 7/11 by runtime with PAR-2
  `1018.540`, log `log/bench-11-kissat-innovations-2026-05-11-08-48-45`

Implementation notes:

- `best_phase` records the deepest stable-search assignment snapshot seen before a backtrack.
- `target_phase` is the active decision-phase source; stable decisions prefer target phase, then
  saved phase. Focused mode still uses the existing saved-phase behavior.
- Rephase events are stable-mode only and run at root through the maintenance scheduler. The cycle is
  `best -> inverted initial phase -> original initial phase`; the local-search walking source is not
  implemented yet.
- `SAT_REPHASE_INTERVAL=<N>` and the existing `SAT_MAINT_REPHASE_INTERVAL=<N>` both enable rephase.
  Intervals grow using a Kissat-like `N * count * log10(count + 9)^3` schedule.
- `SAT_REPHASE_INTERVAL=0` disables rephase for ablations.

Targeted interval sweep:

- Baseline target set, no rephase: solved 5/6, PAR-2 `278.816`, log
  `log/bench-11-kissat-innovations-2026-05-11-08-13-36`; `random_v355_s3` timed out.
- `SAT_REPHASE_INTERVAL=10000`: solved 6/6, PAR-2 `165.791`, log
  `log/bench-11-kissat-innovations-2026-05-11-08-22-07`; it rescued `random_v355_s3` in `0.50s`
  but regressed `feistel_b64_k32_r22` to `45.49s` and `feistel_b64_k52_r17` to `83.93s`.
- `SAT_REPHASE_INTERVAL=50000`: runtime solved 6/6, PAR-2 `48.226`, log
  `log/bench-11-kissat-innovations-2026-05-11-08-35-06`; it kept the Feistel wins
  (`0.59s`, `4.50s`, `1.22s`), solved `random_v355_s3` in `1.16s`, and solved timetable in
  `13.92s`.
- `40000` and `100000` both timed out `feistel_b64_k52_r17`; `60000` solved the SAT target slice
  but was slower than `50000` on every comparable SAT target.

Analysis:

- The machinery itself is safe for the default path: no-rephase full profiling stayed at 6/11 and
  PAR-2 `1246.720`, matching the previous default cap-8 profile.
- `SAT_REPHASE_INTERVAL=10000` is now the default runtime setting. The targeted sweep showed this
  aggressive interval rescued `random_v355_s3` but substantially perturbed Feistel search paths, so
  future rephase work should add walking/source guards before treating this as fully tuned.
- `SAT_REPHASE_INTERVAL=50000` remains the best measured full-profile interval so far: it improved
  by about `228` PAR-2 and gained `random_v355_s3` (`TIMEOUT` -> `1.17s`).
- On `random_v292_s4`, solver time regressed from `14.85s` to `26.85s`, and the DRAT checker did
  not finish after 14 minutes, so that benchmark row is not verification-complete in the `50000`
  full-profile log. A pre-existing unrelated long-running `drat-trim` process was also using one CPU
  during these experiments, but the proof-checking regression is large enough that
  `SAT_REPHASE_INTERVAL=0` should remain part of future ablations.

Reluctant restart and mode-switch validation on 2026-05-12:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_RESTART_MODE=reluctant SAT_MODE_SWITCH_INTERVAL=1 SAT_CHECK_INVARIANTS=1 \
  bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_SEARCH_MODE=focused SAT_RESTART_MODE=reluctant SAT_MODE_SWITCH_INTERVAL=1 \
  SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-innovations
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_RESTART_MODE=reluctant bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
SAT_RESTART_MODE=reluctant SAT_MODE_SWITCH_INTERVAL=1000 bash tools/bench.sh -t 120 \
  -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_RESTART_MODE=reluctant SAT_MODE_SWITCH_INTERVAL=50000 bash tools/bench.sh -t 120 \
  -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
```

Results:

- `cargo test`: 88 passed.
- default smoke suite: 9/9 passed, log `log/2026-05-12-07-23-42`.
- reluctant restart plus aggressive mode-switch invariant smoke: 9/9 passed, log
  `log/2026-05-12-07-23-56`.
- focused-start reluctant restart plus aggressive mode-switch invariant smoke: 9/9 passed, log
  `log/2026-05-12-07-24-04`.
- default profiling after the implementation: solved 8/11, PAR-2 `898.091`, log
  `log/bench-11-kissat-innovations-2026-05-12-07-24-48`.
- `SAT_RESTART_MODE=reluctant`: solved 8/11, PAR-2 `872.650`, log
  `log/bench-11-kissat-innovations-2026-05-12-07-44-59`.
- `SAT_RESTART_MODE=reluctant SAT_MODE_SWITCH_INTERVAL=1000`: solved 4/11, PAR-2 `1736.112`, log
  `log/bench-11-kissat-innovations-2026-05-12-07-59-37`.
- `SAT_RESTART_MODE=reluctant SAT_MODE_SWITCH_INTERVAL=50000`: runtime solved 8/11, PAR-2
  `998.225`, log `log/bench-11-kissat-innovations-2026-05-12-08-14-53`; `random_v292_s4`
  verification was manually stopped after about 14 minutes of `drat-trim`, so the row is not
  verification-complete.

Implementation notes:

- `SAT_RESTART_MODE=reluctant` uses a Kissat-style reluctant doubling sequence in stable mode with
  `SAT_RELUCTANT_INTERVAL=1024` and `SAT_RELUCTANT_LIMIT=1048576` defaults.
- In focused mode, the same restart mode uses the existing glue-EMA restart signal so mode switching
  has a coherent focused restart policy.
- `SAT_MODE_SWITCH_INTERVAL=<N>` schedules root-safe stable/focused mode switches through the
  maintenance scheduler. Intervals grow as `N * count * log10(count + 9)^4`; the default remains
  disabled.
- Mode switches backtrack to root, rebuild the stable heap/focused queue for the new mode, clear a
  pending restart, and reset the reluctant schedule when returning to stable mode.

Analysis:

- Reluctant-only is a small positive ablation but not enough to become the default under the usual
  `>3%` keep threshold: it improved PAR-2 by `25.441` seconds versus the fresh default
  (`898.091 -> 872.650`, about `2.8%`) with the same solved/timeout split.
- The main reluctant-only win was `feistel_b64_k52_r17` (`82.612s -> 24.079s`). Main regressions
  were `feistel_b64_k32_r22` (`44.988s -> 69.005s`), Timetable (`10.780s -> 20.833s`), and `mp1`
  (`5.019s -> 8.076s`).
- Frequent mode switching is clearly harmful with the current focused queue: interval `1000` timed
  out `feistel_b64_k52_r17`, `mp1`, `random_v285_s2`, and `random_v292_s4`, dropping to 4/11
  solved.
- Rare mode switching is still not ready: interval `50000` improved `feistel_b64_k52_r17` further
  (`11.965s`) but badly regressed Timetable (`109.144s`), slowed both random UNSAT rows, and
  produced another long `random_v292_s4` proof-check failure.
- This raw-switch conclusion was superseded by the guarded mode-switch work below: the default now
  uses reluctant restarts plus stale-stable switching at interval `50000`, while
  `SAT_RESTART_MODE=luby SAT_MODE_SWITCH_INTERVAL=0` preserves the previous default-search
  ablation.

Guarded mode-switch validation on 2026-05-12:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_RESTART_MODE=reluctant SAT_MODE_SWITCH_INTERVAL=10 SAT_MODE_SWITCH_POLICY=stale-stable \
  SAT_MODE_SWITCH_STALE_CONFLICTS=5 SAT_MODE_SWITCH_FOCUSED_CONFLICTS=5 \
  bash tools/smoke_test.sh solver/11-kissat-innovations
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_RESTART_MODE=reluctant SAT_MODE_SWITCH_INTERVAL=1000 SAT_MODE_SWITCH_POLICY=stale-stable \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_RESTART_MODE=reluctant SAT_MODE_SWITCH_INTERVAL=50000 SAT_MODE_SWITCH_POLICY=stale-stable \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
```

Results:

- `cargo test`: 92 passed.
- default smoke suite: 9/9 passed, log `log/2026-05-12-11-10-08`.
- guarded-switch smoke suite with aggressive test intervals: 9/9 passed, log
  `log/2026-05-12-11-10-28`.
- same-turn default profiling anchor: solved 8/11, PAR-2 `905.022`, log
  `log/bench-11-kissat-innovations-2026-05-12-09-59-14`.
- guarded stale-stable interval `1000` before the focused dwell cap was rejected: solved 6/11,
  PAR-2 `1316.666`, log `log/bench-11-kissat-innovations-2026-05-12-10-20-13`.
- guarded stale-stable interval `50000` with the final default focused dwell cap (`1000` conflicts):
  solved 8/11, PAR-2 `849.514`, log
  `log/bench-11-kissat-innovations-2026-05-12-11-10-40`.

Implementation notes:

- `SAT_MODE_SWITCH_POLICY=stale-stable` keeps scheduled mode-switch actions root-safe but refuses
  stable-to-focused transitions while stable mode is still reaching deeper trails. The stale window
  defaults to the mode-switch interval and can be overridden with
  `SAT_MODE_SWITCH_STALE_CONFLICTS=<conflicts>`.
- Focused mode is now a bounded escape phase rather than a one-way trajectory change. When mode
  switching is enabled, focused search returns to stable after
  `SAT_MODE_SWITCH_FOCUSED_CONFLICTS`; the automatic default is
  `min(SAT_MODE_SWITCH_INTERVAL, 1000)`.
- Focused-mode low-yield reduction cooldown is applied automatically with default
  `SAT_FOCUSED_REDUCE_LOW_YIELD_COOLDOWN=100`, while the old global
  `SAT_REDUCE_LOW_YIELD_COOLDOWN` remains available for explicit stable-mode ablations.
- Search traces now report `mode_switch_attempts`, `mode_switch_skipped`, and
  `mode_switch_stale` alongside the existing mode-switch counts.

Analysis:

- The now-default profile (`reluctant + stale-stable interval 50000`) improved PAR-2 by
  `55.508` versus the same-turn default anchor (`905.022 -> 849.514`, about `6.1%`) with the same
  solved/timeout split.
- Main wins versus the same-turn default anchor: `feistel_b64_k52_r17` `88.979s -> 15.059s`,
  `feistel_b64_k32_r22` `45.531s -> 31.229s`, and `feistel_b64_k57_r18` `2.924s -> 1.594s`.
- Main regressions: `mp1-Nb7T46` `5.067s -> 22.274s`, Timetable `11.023s -> 15.432s`,
  `random_v285_s2` `9.552s -> 13.194s`, and `random_v292_s4` `21.434s -> 30.216s`.
- The focused dwell cap was required. A trace on `mp1-Nb7T46` showed the no-cap guarded policy
  switched into focused mode once and was still focused at `315000` conflicts after `27.214s`;
  default stable solved the same decompressed instance after `44025` conflicts and `2.639s` of
  search. With the dwell cap, the guarded policy returned to stable and solved; the `1000` conflict
  cap trace solved in `20.375s` of search.
- The `random_v292_s4` solver result is not proof-verification complete in the profiling logs where
  noted `[VERIFY FAIL]`; in those runs the solver produced UNSAT and `drat-trim` was manually
  stopped after several minutes of verification tail to keep the experiment loop moving. A
  pre-existing unrelated long `drat-trim` process from a solver-10 benchmark was also consuming one
  core throughout these measurements, so the numbers should be treated as directional rather than
  final medium-run evidence.

Default policy:

- The 50k guarded policy is now the built-in default after the 2026-05-12 follow-up request. It
  clears the local `>3%` profiling threshold, but the regression shape shows focused mode is still
  path-sensitive. Keep `SAT_RESTART_MODE=luby SAT_MODE_SWITCH_INTERVAL=0` available for ablations
  against the previous default-search behavior.

Default-policy flip validation on 2026-05-12:

- Built-in defaults now match the measured guarded policy:
  `SAT_RESTART_MODE=reluctant`, `SAT_MODE_SWITCH_POLICY=stale-stable`, and
  `SAT_MODE_SWITCH_INTERVAL=50000`.
- `cargo test`: 92 passed.
- default smoke suite: 9/9 passed, log `log/2026-05-12-11-57-53`.
- previous default-search ablation smoke
  (`SAT_RESTART_MODE=luby SAT_MODE_SWITCH_INTERVAL=0`): 9/9 passed, log
  `log/2026-05-12-11-59-13`.
