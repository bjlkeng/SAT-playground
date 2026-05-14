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
  `SAT_SEARCH_MODE=focused` enables a focused recent-conflict decision queue for measurement.
  Kissat-exact focused queue-cursor and focused phase-override experiments are available as
  `SAT_FOCUSED_DECISION=kissat` and `SAT_FOCUSED_PHASE=kissat`, but both are non-default after the
  2026-05-13 profiling rejection; the accepted defaults are `SAT_FOCUSED_DECISION=pop-front` and
  `SAT_FOCUSED_PHASE=saved`.
- Phase 7 restart/reduction-pressure scaffold: `SAT_RESTART_MODE=glue-ema` enables a fast/slow
  learned-glue EMA restart trigger, and `SAT_REDUCE_LOW_YIELD_COOLDOWN=<conflicts>` enables an
  opt-in cooldown after low-yield learned-clause reduction passes; Kissat-style restart trail reuse
  is now the default with the measured `SAT_RESTART_REUSE_CAP=8` setting; use
  `SAT_RESTART_REUSE=off` for ablations, or `SAT_RESTART_REUSE_CAP=0` for uncapped reuse. Dynamic
  restart-reuse guarding is also default-on as `SAT_RESTART_REUSE_GUARD=progress`; it disables
  reuse briefly when a reused restart window is short and learned-clause glue worsens. Use
  `SAT_RESTART_REUSE_GUARD=off` for guard-only ablations.
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
- Phase 7 conflict-analysis controls: chronological backtracking is enabled globally by default
  with `SAT_CHRONO_LEVELS=100`; use `SAT_CHRONO_LEVELS=off` to disable it for ablations. Extra
  reason-side variable bumping is controlled by
  `SAT_REASON_SIDE_BUMP_MODE=off|traversal|one-hop`, defaulting to one-hop with unlimited
  immediate reason-side variables after the 2026-05-12 follow-up request. Learned-clause variables
  are still bumped when extra reason-side bumping is off. Use
  `SAT_REASON_SIDE_BUMP_MODE=traversal SAT_REASON_SIDE_BUMP_LIMIT=unlimited` to restore the
  previous full-UIP-traversal analyzed-variable bumping behavior, or
  `SAT_REASON_SIDE_BUMP_MODE=off` to return to the capped/no-extra-reason-side default-search
  ablation.
- Phase 8 lucky SAT shortcut: `SAT_LUCKY=shortcut` is the default and checks all-true/all-false
  candidate models after preprocessing but before CDCL search. Use `SAT_LUCKY=off` for ablations.
- Phase 8 clause-weight reorder default: `SAT_REORDER=kissat` is now the default delayed
  mode-aware variant after the 2026-05-12 follow-up request. It starts at `SAT_REORDER_INIT`
  conflicts, repeats with linearly growing
  `SAT_REORDER_INTERVAL` windows, rescales stable scores before adding weights, and reorders the
  focused queue by weight plus existing recency. Use `SAT_REORDER=off` for the no-reorder
  ablation or `SAT_REORDER=stable-weight` for the pre-search stable-heap experiment.
- Phase 7 phase-source defaults: bounded pre-search warmup and scheduled local-search walking are
  now enabled by default after the 2026-05-13 follow-up confirmation run. Warmup makes ordinary
  decisions, propagates, saves phases through the normal enqueue path, then backtracks without
  updating target/best snapshots. Scheduled walking uses the Kissat-style
  `best -> walk -> inverted -> best -> walk -> original` stable rephase source cycle with
  `SAT_WALK_STEPS=100` and `SAT_WALK_RANDOM_PERCENT=0`. Use `SAT_WARMUP=off` or `SAT_WALK=off`
  for ablations. `SAT_WALK_INITIAL=1` remains opt-in and runs the same local-search phase source
  once before CDCL search.
- Full backward-subsumption / self-subsuming-resolution sweep is off by default after the same
  follow-up request; use `SAT_FULL_BSR=on` to restore the previous full-BSR preprocessing behavior.

What is intentionally not present yet:

- no full lucky probing, real mid-search inprocessing pass, or probing yet
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

Chronological backtracking and reason-side bump limit validation on 2026-05-12:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_CHRONO_LEVELS=2 SAT_REASON_SIDE_BUMP_LIMIT=2 SAT_CHECK_INVARIANTS=1 \
  bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_CHRONO_LEVELS=0 SAT_REASON_SIDE_BUMP_LIMIT=0 SAT_CHECK_INVARIANTS=1 \
  bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_PROOF=0 bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling \
  solver/11-kissat-innovations
SAT_PROOF=0 SAT_CHRONO_LEVELS=100 bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_REASON_SIDE_BUMP_LIMIT=0 bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_REASON_SIDE_BUMP_LIMIT=0 SAT_CHRONO_LEVELS=100 \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling \
  solver/11-kissat-innovations
```

Results:

- `cargo test`: 95 passed.
- default smoke suite before the default flip: 9/9 passed, log `log/2026-05-12-12-23-27`.
- opt-in chrono/reason smoke (`SAT_CHRONO_LEVELS=2 SAT_REASON_SIDE_BUMP_LIMIT=2` with
  invariants): 9/9 passed, log `log/2026-05-12-12-23-39`.
- aggressive knob smoke (`SAT_CHRONO_LEVELS=0 SAT_REASON_SIDE_BUMP_LIMIT=0` with invariants):
  9/9 passed, log `log/2026-05-12-12-23-46`.
- no-proof default profiling baseline: solved 8/11, PAR-2 `848.544`, log
  `log/bench-11-kissat-innovations-2026-05-12-12-46-12`.
- no-proof conservative chrono-only (`SAT_CHRONO_LEVELS=100`): solved 8/11, PAR-2 `848.613`,
  log `log/bench-11-kissat-innovations-2026-05-12-12-56-42`.
- no-proof accepted reason-side cap (`SAT_REASON_SIDE_BUMP_LIMIT=0`): solved 8/11, PAR-2
  `817.695`, log `log/bench-11-kissat-innovations-2026-05-12-13-05-06`.
- no-proof reason cap plus conservative chrono (`SAT_REASON_SIDE_BUMP_LIMIT=0
  SAT_CHRONO_LEVELS=100`), now the default policy after the follow-up chrono request: solved 8/11,
  PAR-2 `817.653`, log
  `log/bench-11-kissat-innovations-2026-05-12-13-17-51`.
- final built-in default after the reason-side cap flip: solved 8/11, PAR-2 `817.875`, log
  `log/bench-11-kissat-innovations-2026-05-12-13-38-18`.
- final default smoke suite after the cap flip: 9/9 passed, log `log/2026-05-12-13-37-52`.
- final default invariant smoke suite after the cap flip: 9/9 passed, log
  `log/2026-05-12-13-38-05`.
- follow-up built-in default after enabling global `SAT_CHRONO_LEVELS=100`: solved 8/11, PAR-2
  `816.106`, log `log/bench-11-kissat-innovations-2026-05-12-14-30-48`.
- follow-up default smoke after enabling global chrono: 9/9 passed, log
  `log/2026-05-12-14-30-26`.
- follow-up invariant smoke after enabling global chrono: 9/9 passed, log
  `log/2026-05-12-14-30-39`.

Important rejected attempts:

- `SAT_CHRONO_LEVELS=2` with unlimited reason-side bumping was stopped early after
  `feistel_b64_k32_r22` regressed from `31.33s` to `63.39s`.
- `SAT_REASON_SIDE_BUMP_LIMIT=4` was stopped early after timing out
  `feistel_b64_k52_r17` and slowing `feistel_b64_k57_r18` to `23.45s`.
- `SAT_REASON_SIDE_BUMP_LIMIT=0 SAT_CHRONO_LEVELS=2` completed but regressed badly: solved 8/11,
  PAR-2 `921.638`, log `log/bench-11-kissat-innovations-2026-05-12-13-25-44`.

Analysis:

- The accepted change is the reason-side cap, not chronological backtracking. Limiting extra
  reason-side bumps to zero improved the no-proof profiling PAR-2 by `30.669` to `30.849` seconds
  depending on same-turn rerun (`848.544 -> 817.875` built-in default, `848.544 -> 817.695`
  opt-in cap), about `3.6%` with the same solved/timeout split.
- Main wins versus the no-proof baseline: `feistel_b64_k32_r22` `31.33s -> 27.63s`,
  `feistel_b64_k52_r17` `15.12s -> 11.68s`, `feistel_b64_k57_r18` `1.59s -> 0.44s`,
  Timetable `15.05s -> 10.25s`, `mp1-Nb7T46` `21.69s -> 16.50s`, `random_v285_s2`
  `13.14s -> 8.56s`, and `random_v292_s4` `30.10s -> 15.14s`.
- The main regression was `random_v355_s3` (`0.52s -> 7.49s`), which is a search-path sensitivity
  warning but not enough to erase the profile-set gain.
- Direct no-proof trace on `random_v292_s4` explains the main random-UNSAT win: the accepted
  default solved in `15.256s` with `1816741` conflicts, `2268059` decisions, `84916180`
  propagations, `589` restarts, and `2692` reduce-DB calls. The legacy
  `SAT_REASON_SIDE_BUMP_LIMIT=unlimited` mode took `30.215s` with `3310953` conflicts,
  `3957454` decisions, `154902824` propagations, `960` restarts, and `4496` reduce-DB calls.
- Direct no-proof trace on the `random_v355_s3` regression shows the tradeoff: the accepted default
  took `7.475s` with `840045` conflicts and `46920823` propagations, while the legacy unlimited
  mode solved in `0.514s` with `63957` conflicts and `3585194` propagations. The cap improves the
  aggregate profile set but can still damage individual SAT trajectories.
- Conservative chronological backtracking (`SAT_CHRONO_LEVELS=100`) was neutral on top of the
  accepted reason-side cap, but it now matches Kissat's global default threshold and remains
  ablatable with `SAT_CHRONO_LEVELS=off`. The follow-up default profiling confirmation was slightly
  better than the previous cap-only default (`817.875 -> 816.106` PAR-2), but this is still best
  treated as neutral/noise rather than a separate chrono win.
- These profiling runs used `SAT_PROOF=0` so the timings isolate solver search from the long
  `drat-trim` tail observed on `random_v292_s4`. Proof correctness remains covered by the smoke
  suite; the no-proof benchmark rows intentionally report UNSAT rows as `[no proof]`.

One-hop reason-side bump validation on 2026-05-12:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_REASON_SIDE_BUMP_MODE=one-hop SAT_REASON_SIDE_BUMP_LIMIT=10 SAT_CHECK_INVARIANTS=1 \
  bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_REASON_SIDE_BUMP_MODE=traversal SAT_REASON_SIDE_BUMP_LIMIT=unlimited \
  SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_PROOF=0 bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling \
  solver/11-kissat-innovations
SAT_PROOF=0 SAT_REASON_SIDE_BUMP_MODE=off \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling \
  solver/11-kissat-innovations
SAT_PROOF=0 SAT_REASON_SIDE_BUMP_MODE=one-hop SAT_REASON_SIDE_BUMP_LIMIT=10 \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling \
  solver/11-kissat-innovations
SAT_PROOF=0 SAT_REASON_SIDE_BUMP_MODE=one-hop SAT_REASON_SIDE_BUMP_LIMIT=1 \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling \
  solver/11-kissat-innovations
SAT_PROOF=0 SAT_REASON_SIDE_BUMP_MODE=one-hop SAT_REASON_SIDE_BUMP_LIMIT=unlimited \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling \
  solver/11-kissat-innovations
```

Results:

- `cargo test`: 96 passed.
- default smoke suite: 9/9 passed, log `log/2026-05-12-14-57-14`.
- one-hop cap `10` invariant smoke: 9/9 passed, log `log/2026-05-12-14-57-28`.
- legacy traversal invariant smoke: 9/9 passed, log `log/2026-05-12-14-57-36`.
- post-change no-proof prior-default anchor, reproducible now with `SAT_REASON_SIDE_BUMP_MODE=off`:
  solved 8/11, PAR-2 `816.217`, log
  `log/bench-11-kissat-innovations-2026-05-12-14-57-59`.
- bounded one-hop cap `10` experiment (`SAT_REASON_SIDE_BUMP_MODE=one-hop
  SAT_REASON_SIDE_BUMP_LIMIT=10`, implicit before the final default-limit change) was stopped
  early:
  first three Feistel rows were `42.431s`, `14.987s`, and `2.242s`, versus default `27.78s`,
  `11.67s`, and `0.44s`; partial log
  `log/bench-11-kissat-innovations-2026-05-12-15-05-55`.
- one-hop minimal cap (`SAT_REASON_SIDE_BUMP_MODE=one-hop SAT_REASON_SIDE_BUMP_LIMIT=1`) was also
  stopped early: it improved `feistel_b64_k52_r17` (`11.67s -> 0.964s`) but regressed
  `feistel_b64_k32_r22` (`27.78s -> 51.903s`) and `feistel_b64_k57_r18` (`0.44s -> 29.504s`);
  partial log `log/bench-11-kissat-innovations-2026-05-12-15-07-48`.
- one-hop unlimited completed per follow-up request: solved 8/11, PAR-2 `794.645`, log
  `log/bench-11-kissat-innovations-2026-05-12-15-41-28`.
- final built-in default after flipping one-hop unlimited on by default: `cargo test` passed 96
  tests, default smoke passed 9/9 (`log/2026-05-12-16-02-25`), invariant smoke passed 9/9
  (`log/2026-05-12-16-02-34`), and the no-proof profiling confirmation solved 8/11 with PAR-2
  `794.641` (`log/bench-11-kissat-innovations-2026-05-12-16-02-42`).

Analysis:

- The implemented one-hop pass is not recursive: after the final learned clause is known, it
  inspects only each learned literal's immediate binary or long-clause reason and bumps side
  variables up to `SAT_REASON_SIDE_BUMP_LIMIT`.
- One-hop unlimited is now the default after the follow-up request. The completed unlimited run
  improved the no-proof profile PAR-2 by `21.572` seconds (`816.217 -> 794.645`, about `2.6%`) with
  the same solved/timeout split. This is below the usual `>3%` threshold, but it is intentionally
  kept as the default to match the requested policy.
- The final no-env default confirmation reproduced that result after the code default changed:
  PAR-2 `794.641`, solved 8/11, same timeout split.
- One-hop unlimited wins: `feistel_b64_k32_r22` `27.78s -> 7.01s`,
  `feistel_b64_k52_r17` `11.67s -> 5.12s`, `mp1-Nb7T46` `12.32s -> 3.63s`,
  `random_v285_s2` `8.53s -> 3.57s`, and `random_v292_s4` `15.18s -> 7.76s`.
- One-hop unlimited regressions: `feistel_b64_k57_r18` `0.44s -> 2.18s`, Timetable
  `12.83s -> 28.84s`, and `random_v355_s3` `7.47s -> 16.53s`.
- Direct traces show this is a search-trajectory effect, not local overhead. On
  `feistel_b64_k52_r17`, one-hop cap `1` solved in `0.940s` with `63241` conflicts and `5196216`
  propagations, versus default `11.573s`, `530808` conflicts, and `52921576` propagations. On
  `feistel_b64_k32_r22`, the same cap regressed to `51.661s`, `1200333` conflicts, and
  `213032231` propagations, versus default `27.728s`, `677717` conflicts, and `112518055`
  propagations.
- Direct traces for one-hop unlimited show the same trajectory tradeoff. On `random_v292_s4`, it
  solved in `7.768s` with `948357` conflicts and `43605767` propagations, versus default
  `15.125s`, `1816741` conflicts, and `84916180` propagations. On the Timetable regression, it
  took `19.543s` with `906640` conflicts and `51525605` propagations on a decompressed direct trace,
  versus default `4.475s`, `169292` conflicts, and `22497680` propagations on the same file.

Lucky SAT shortcut validation on 2026-05-12:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_TRACE_PREPROCESS=1 SAT_PROOF=0 \
  bash solver/11-kissat-innovations/run.sh tests/cnf/sat/all_positive.cnf \
  /tmp/sat-lucky-all-positive
SAT_PROOF=0 bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling \
  solver/11-kissat-innovations
SAT_PROOF=0 SAT_LUCKY=off bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
```

Results:

- `cargo test`: 100 passed.
- default smoke suite: 9/9 passed, log `log/2026-05-12-16-36-04`.
- invariant smoke suite: 9/9 passed, log `log/2026-05-12-16-36-13`.
- traced all-positive smoke instance returned SAT before CDCL decisions with
  `lucky=1/1/1/0` and model `1 2 3`.
- no-proof default shortcut profiling: solved 8/11, PAR-2 `794.191`, log
  `log/bench-11-kissat-innovations-2026-05-12-16-36-29`.
- no-proof `SAT_LUCKY=off` ablation: solved 8/11, PAR-2 `793.828`, log
  `log/bench-11-kissat-innovations-2026-05-12-16-44-02`.

Analysis:

- The shortcut validates the actual full candidate assignment against all live original and learned
  clauses after preprocessing. This is slightly more general than a raw sign-only check, because it
  respects root assignments and clauses already removed or trimmed by simplification.
- The profiling set showed no meaningful shortcut opportunity. The `SAT_LUCKY=off` ablation was
  `0.363s` faster in PAR-2, about `0.05%`, which is timing noise on this 11-instance sample.
- The feature is kept as a default foundational shortcut rather than as a measured profiling-set
  speed win. It creates a safe pre-CDCL SAT return path with model extension, counters, and an
  ablation knob for the later four-pass lucky probing work.

Stable clause-weight reorder validation on 2026-05-12:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_REORDER=stable-weight SAT_CHECK_INVARIANTS=1 \
  bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_TRACE_PREPROCESS=1 SAT_PROOF=0 SAT_REORDER=stable-weight \
  bash solver/11-kissat-innovations/run.sh benchmarks/profiling/feistel_b64_k32_r22.cnf \
  /tmp/sat-reorder-feistel-k32
SAT_TRACE_PREPROCESS=1 SAT_PROOF=0 SAT_SEARCH_MODE=focused SAT_REORDER=stable-weight \
  timeout 5s bash solver/11-kissat-innovations/run.sh \
  benchmarks/profiling/feistel_b64_k32_r22.cnf /tmp/sat-reorder-focused-k32
SAT_PROOF=0 SAT_REORDER=stable-weight bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling \
  solver/11-kissat-innovations
```

Results:

- `cargo test`: 104 passed.
- default smoke suite: 9/9 passed, log `log/2026-05-12-20-38-11`.
- default invariant smoke suite: 9/9 passed, log `log/2026-05-12-20-38-22`.
- opt-in reorder invariant smoke suite: 9/9 passed, log `log/2026-05-12-20-51-26`.
- traced `feistel_b64_k32_r22` preprocessing with `SAT_REORDER=stable-weight`: scanned `8111`
  live clauses and `29701` literals, boosted `1200` variables, and spent `0.088ms` in reorder.
- default-off profiling confirmation after the implementation: solved 8/11, PAR-2 `794.914`, log
  `log/bench-11-kissat-innovations-2026-05-12-20-41-47`.
- full opt-in reorder profiling: solved 8/11, PAR-2 `940.312`, log
  `log/bench-11-kissat-innovations-2026-05-12-21-07-19`. Solved count was unchanged, but PAR-2
  worsened by `145.398s`: `feistel_b64_k32_r22` `7.137s -> 20.349s`,
  `feistel_b64_k52_r17` `5.210s -> 51.128s`, `feistel_b64_k57_r18`
  `2.212s -> 42.423s`, timetable `28.566s -> 48.914s`, and `mp1`
  `3.529s -> 27.559s`. The only wins were small: `random_v285_s2`
  `3.609s -> 3.431s` and `random_v355_s3` `16.799s -> 14.538s`.
- focused-mode opt-in trace confirmed the pass is skipped outside stable search:
  `reorder=1/0/1`, `reorder_scanned=0/0/0`.

Analysis:

- The implementation is cheap locally but harmful as a search policy on the profiling set. The
  direct `feistel_b64_k32_r22` trace with `SAT_REORDER=off` solved in `6.997s` with `187452`
  conflicts, `209944` decisions, and `32638486` propagations. The stable-weight run took
  `20.434s` with `506505` conflicts, `561757` decisions, and `94834877` propagations.
- Keep `SAT_REORDER=stable-weight` as an opt-in foundation and maintenance-hook implementation, but
  do not make it the default. It should not affect focused mode in the current implementation:
  focused search uses a different queue policy, and the stable-weight hook intentionally skips when
  `search_mode != Stable`. A focused reorder would need a separate queue/stamp policy and separate
  validation rather than reusing this stable heap activity boost.

Delayed Kissat-style reorder validation on 2026-05-12:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_REORDER=kissat SAT_REORDER_INIT=0 SAT_REORDER_INTERVAL=1 SAT_CHECK_INVARIANTS=1 \
  bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_PROOF=0 SAT_REORDER=kissat bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_REORDER=kissat SAT_TRACE_REORDER=1 SAT_TRACE_SEARCH_INTERVAL=1000000000 \
  bash solver/11-kissat-innovations/run.sh benchmarks/profiling/feistel_b64_k32_r22.cnf \
  /tmp/sat-reorder-kissat-delayed-k32
```

Results:

- `cargo test`: 109 passed.
- default smoke suite: 9/9 passed, log `log/2026-05-12-22-26-19`.
- default invariant smoke suite: 9/9 passed, log `log/2026-05-12-22-26-23`.
- forced delayed reorder invariant smoke suite: 9/9 passed, log `log/2026-05-12-22-26-31`.
- full `SAT_REORDER=kissat` profiling: solved 8/11, PAR-2 `917.395`, log
  `log/bench-11-kissat-innovations-2026-05-12-22-11-59`. This improves on the rejected
  pre-search `stable-weight` PAR-2 `940.312`, but still regresses badly against default-off
  `794.914`.
- Against default-off, the largest regressions were `feistel_b64_k32_r22`
  `7.137s -> 22.147s`, `feistel_b64_k52_r17` `5.210s -> 48.971s`,
  `feistel_b64_k57_r18` `2.212s -> 15.443s`, timetable `28.566s -> 79.095s`,
  and `mp1` `3.529s -> 12.617s`. Wins were limited to `random_v292_s4`
  `7.852s -> 7.011s` and `random_v355_s3` `16.799s -> 7.416s`.
- `feistel_b64_k32_r22` trace: 11 stable-mode reorder applications fired at roughly
  `10k, 20k, 40k, 70k, 110k, 160k, 220k, 290k, 370k, 460k, 560k` conflicts. Most changed the
  front branch variable; each pass scanned `8111` clauses and `29701` literals, boosted `1200`
  variables, and cost about `0.16ms`. Final search grew to `616120` conflicts, `698970`
  decisions, and `107241644` propagations.
- One-instance `k32` sensitivity checks did not rescue the policy: `SAT_REORDER_INIT=50000`
  / `SAT_REORDER_INTERVAL=50000` took `56.30s`; `100000/100000` took `26.91s`; starting in
  focused mode with `SAT_REORDER=kissat` took `103.17s`.

Analysis:

- The smarter integration reduced some of the cold pre-search damage, but the structural signal is
  still not aligned with solver 11's current search dynamics. The pass remains cheap; the loss is
  search-path quality.
- Keep both reorder modes opt-in for diagnostics. The next foundational work should move away from
  clause-weight decision reordering and toward a precursor that changes the formula/search
  substrate, such as binary transitive reduction or scheduled probing, before trying reorder again.

Full-BSR-off reorder interaction experiment on 2026-05-12:

```bash
SAT_FULL_BSR=off SAT_PROOF=0 \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_FULL_BSR=off SAT_PROOF=0 SAT_REORDER=stable-weight \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_FULL_BSR=off SAT_PROOF=0 SAT_REORDER=kissat \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_FULL_BSR=off SAT_PROOF=0 SAT_REORDER=stable-weight SAT_TRACE_PREPROCESS=1 \
  SAT_TRACE_SEARCH_INTERVAL=1000000000 \
  bash solver/11-kissat-innovations/run.sh benchmarks/profiling/feistel_b64_k52_r17.cnf \
  /tmp/sat-fullbsr-off-stable-k52
```

Results:

- `SAT_FULL_BSR=off`, no reorder: solved 7/11, PAR-2 `1072.183`, log
  `log/bench-11-kissat-innovations-2026-05-12-22-33-58`.
- `SAT_FULL_BSR=off SAT_REORDER=stable-weight`: solved 9/11, PAR-2 `614.511`, log
  `log/bench-11-kissat-innovations-2026-05-12-22-44-31`.
- `SAT_FULL_BSR=off SAT_REORDER=kissat`: solved 9/11, PAR-2 `797.994`, log
  `log/bench-11-kissat-innovations-2026-05-12-22-51-24`.
- The pre-search `stable-weight` reorder recovered large no-full-BSR losses:
  `feistel_b64_k52_r17` `TIMEOUT -> 2.205s`, `feistel_b64_k57_r18`
  `30.130s -> 15.215s`, timetable `TIMEOUT -> 10.333s`, and Kakuro
  `26.927s -> 22.725s`. It regressed `mp1` from `1.925s` to `28.296s`
  and `random_v292_s4` from `7.761s` to `11.992s`.
- The delayed `kissat` mode also improved over the no-reorder baseline, but less strongly:
  `feistel_b64_k57_r18` `30.130s -> 2.637s`, timetable `TIMEOUT -> 87.020s`,
  `random_v355_s3` `16.475s -> 7.368s`, while `k52` barely solved at `114.849s`.
- The traced no-full-BSR `stable-weight` `k52` run showed preprocessing with no full BSR left
  `6464` live original clauses and `23808` literals. Reorder scanned them in `0.114ms`, boosted
  `948` variables, and the solver finished in `2.180s` with `108475` conflicts and `11207334`
  propagations.

Analysis:

- Turning off full BSR changes the role of reorder completely. With full BSR on, clause-weight
  reorder fights the simplified search trajectory. With full BSR off, the pre-search structural
  ordering replaces some of the guidance that full BSR had been providing and wins decisively on
  this 11-instance profiling set.
- `SAT_FULL_BSR=off SAT_REORDER=stable-weight` is the best profiling-set result so far, but it
  wins by trading a simplification policy for a search-path heuristic and may be brittle. Per the
  2026-05-12 follow-up request, full BSR is now default-off and the delayed `SAT_REORDER=kissat`
  policy is now the default; the stronger pre-search `stable-weight` combination remains an
  explicit experiment to validate on a larger benchmark slice.

Warmup and local-search phase source validation on 2026-05-13:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_WARMUP=1 SAT_WARMUP_DECISIONS=32 SAT_WALK_INITIAL=1 SAT_WALK=1 \
  SAT_CHECK_INVARIANTS=1 \
  bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_PROOF=0 bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling \
  solver/11-kissat-innovations
SAT_PROOF=0 SAT_WARMUP=1 bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_WALK_INITIAL=1 bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_WALK=1 bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_WARMUP=1 SAT_WALK=1 bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_WALK=1 SAT_WALK_STEPS=100 SAT_WALK_RANDOM_PERCENT=0 \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_WALK=1 SAT_WALK_STEPS=100 bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_WALK=1 SAT_TRACE_PREPROCESS=1 SAT_TRACE_SEARCH_INTERVAL=1000000000 \
  solver/11-kissat-innovations/target/release/sat-solver /tmp/sat-playground-timetable.cnf \
  /tmp/sat-trace-walk > /tmp/sat-trace-walk.stdout
SAT_PROOF=0 SAT_TRACE_PREPROCESS=1 SAT_TRACE_SEARCH_INTERVAL=1000000000 \
  solver/11-kissat-innovations/target/release/sat-solver /tmp/sat-playground-timetable.cnf \
  /tmp/sat-trace-baseline > /tmp/sat-trace-baseline.stdout
```

Results:

- `cargo test`: 113 passed.
- default smoke suite: 9/9 passed, log `log/2026-05-13-00-55-02`.
- all-new-flags invariant smoke suite: 9/9 passed, log `log/2026-05-13-00-55-10`.
- no-new-flag no-proof baseline: solved 9/11, PAR-2 `793.182`, log
  `log/bench-11-kissat-innovations-2026-05-12-23-36-05`.
- `SAT_WARMUP=1`: solved 9/11, PAR-2 `795.985`, log
  `log/bench-11-kissat-innovations-2026-05-12-23-45-58`.
- `SAT_WALK_INITIAL=1`: solved 8/11, PAR-2 `962.377`, log
  `log/bench-11-kissat-innovations-2026-05-12-23-55-52`.
- pre-tuning `SAT_WALK=1` with variable-scaled walk steps: solved 9/11, PAR-2 `846.795`, log
  `log/bench-11-kissat-innovations-2026-05-13-00-04-47`.
- tuned `SAT_WALK=1 SAT_WALK_STEPS=100 SAT_WALK_RANDOM_PERCENT=0`: solved 9/11, PAR-2
  `662.263`, log `log/bench-11-kissat-innovations-2026-05-13-00-15-41`.
- same 100-step cap with the old 1% random setting: solved 9/11, PAR-2 `727.151`, log
  `log/bench-11-kissat-innovations-2026-05-13-00-23-24`.
- final `SAT_WALK=1` after changing walk defaults to 100 deterministic steps: solved 9/11,
  PAR-2 `667.472`, log `log/bench-11-kissat-innovations-2026-05-13-00-32-50`.
- final `SAT_WARMUP=1 SAT_WALK=1`: solved 9/11, PAR-2 `665.956`, log
  `log/bench-11-kissat-innovations-2026-05-13-00-40-36`.
- follow-up confirmation before the default flip with the same settings: solved 9/11, PAR-2
  `667.671`, log `log/bench-11-kissat-innovations-2026-05-13-10-52-55`.
- default-policy confirmation after making warmup plus scheduled walking default-on: solved 9/11,
  PAR-2 `665.670`, log `log/bench-11-kissat-innovations-2026-05-13-11-14-32`.
- default-policy validation after the flip: `cargo test` passed 113 tests, and the smoke suite
  passed 9/9 with UNSAT proofs verified, log `log/2026-05-13-11-14-17`.

Analysis:

- The implementation is proof-safe because warmup and walk only change phase arrays and never add or
  delete clauses. Warmup backtracks through a no-phase-snapshot path so the pass leaves saved
  phases but does not pollute best/target snapshots; it also stops immediately after a
  decision-level warmup conflict.
- Warmup alone is neutral-to-slightly-negative on this slice: it preserved solved count but moved
  PAR-2 from `793.182` to `795.985`. It remains separately ablatable with `SAT_WARMUP=off`.
- Initial walking is rejected. It helped `feistel_b64_k52_r17`, `mp1`, and `random_v355_s3`, but
  slowed `k32`, `k57`, and Kakuro, lost the timetable solve, and added a third timeout.
- Scheduled rephase walking is useful only with a small deterministic cap. The original
  variable-scaled step budget improved `k52` and timetable but over-walked other instances,
  regressing PAR-2 to `846.795`.
- The accepted walking policy is `SAT_WALK=1` with default `SAT_WALK_STEPS=100` and
  `SAT_WALK_RANDOM_PERCENT=0`. The final walk-only confirmation improved PAR-2 by `125.710s`
  (`15.9%`) over the no-walk baseline. Main deltas: `k32` `29.707s -> 15.620s`, `k52`
  `114.906s -> 79.411s`, `k57` `2.641s -> 0.881s`, timetable `87.846s -> 12.804s`, Kakuro
  `48.496s -> 40.278s`, with regressions on `mp1` `10.487s -> 19.355s` and `random_v355_s3`
  `7.403s -> 7.891s`.
- Keeping the old 1% random flip rate with the 100-step cap still improved baseline but was worse
  than deterministic walking (`727.151` versus `662.263` in the tuning run), so the default random
  rate is now zero.
- Warmup plus accepted rephase walking was only `1.516s` better than walk-only on the final
  confirmation, far below the local 3% keep threshold. After the follow-up request and rerun, this
  warmup-plus-walk policy is the built-in default because it reproduced the same 9/11 solved split
  and PAR-2 within normal run-to-run noise, including a no-env default run at `665.670`.
- Direct timetable traces used identical preprocessing stats: `105987` eliminated variables,
  `334841` resolvents, `567515` live original clauses, `1966590` original literals, `3992` root
  assignments, and about `3.1s` preprocessing. Search changed from `81.655s`, `2661411`
  conflicts, `6200853` decisions, `244378607` propagations, `747` restarts, `16` rephases, and no
  walks to `9.832s`, `542423` conflicts, `1348218` decisions, `38983177` propagations, `183`
  restarts, `8` rephases, and `3` successful walks (`300` total steps, best unsat
  `616 -> 529`). This is a search-trajectory win, not a preprocessing difference.

Dynamic restart-reuse guard validation on 2026-05-13:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_PROOF=0 bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling \
  solver/11-kissat-innovations
SAT_PROOF=0 SAT_RESTART_REUSE_GUARD=off bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_RESTART_REUSE=off bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_TRACE_SEARCH_INTERVAL=1000000000 \
  bash solver/11-kissat-innovations/run.sh benchmarks/profiling/feistel_b64_k52_r17.cnf \
  /tmp/sat-rrg-k52
```

Results:

- `cargo test`: 116 passed.
- default smoke suite: 9/9 passed, including UNSAT proof checking, log
  `log/2026-05-13-18-42-51`.
- default no-proof profiling with the guard on: solved 9/11, PAR-2 `629.226`, log
  `log/bench-11-kissat-innovations-2026-05-13-18-15-12`.
- guard-off ablation: solved 9/11, PAR-2 `669.566`, log
  `log/bench-11-kissat-innovations-2026-05-13-18-22-21`.
- reuse-off ablation: solved 9/11, PAR-2 `722.942`, log
  `log/bench-11-kissat-innovations-2026-05-13-18-33-10`.
- traced default `feistel_b64_k52_r17`: SAT in `41.024s`, with
  `restart_reuse_guard=436/2/1`.

Analysis:

- The accepted guard watches restart windows after reused restarts. If the next restart arrives
  within `SAT_RESTART_REUSE_GUARD_MIN_CONFLICTS=128` conflicts and the learned-clause average glue
  is at least `1.05x` worse than the prior restart window, it skips reuse until
  `SAT_RESTART_REUSE_GUARD_COOLDOWN=1024` more conflicts pass.
- The profiling-set gain is `40.340s` PAR-2 versus the guard-off ablation with the same solved
  split. The main movement is `feistel_b64_k52_r17`, which changed from `80.721s` guard-off to
  `40.977s` guard-on. Other solved instances were within small timing noise.
- Reuse remains valuable with the guard: default guard-on is `93.716s` PAR-2 faster than
  `SAT_RESTART_REUSE=off`. No-reuse happened to solve `k52` faster (`32.533s` versus `40.977s`),
  but lost more time on `k32`, timetable, Kakuro, and the random UNSAT rows.
- The traced `k52` run confirms the guard is not dead code: it checked 436 restart windows, skipped
  reuse twice, and entered cooldown once on the winning trajectory.

Focused queue and phase exactness validation on 2026-05-13:

```bash
cd solver/11-kissat-innovations && cargo test
bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_SEARCH_MODE=focused SAT_FOCUSED_DECISION=kissat SAT_FOCUSED_PHASE=kissat \
  SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-innovations
SAT_PROOF=0 bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling \
  solver/11-kissat-innovations
SAT_PROOF=0 SAT_FOCUSED_DECISION=kissat SAT_FOCUSED_PHASE=kissat \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_FOCUSED_DECISION=pop-front SAT_FOCUSED_PHASE=saved \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_FOCUSED_DECISION=pop-front \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
SAT_PROOF=0 SAT_FOCUSED_PHASE=saved \
  bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/11-kissat-innovations
```

Results:

- `cargo test`: 120 passed.
- default smoke suite: 9/9 passed, including UNSAT proof checking, log
  `log/2026-05-13-20-06-16`.
- opt-in exact focused invariant smoke suite: 9/9 passed, including UNSAT proof checking, log
  `log/2026-05-13-19-57-08`.
- final no-env default profiling confirmation: solved 9/11, PAR-2 `622.999`, log
  `log/bench-11-kissat-innovations-2026-05-13-19-57-15`.
- exact focused queue plus exact focused phase: solved 9/11, PAR-2 `699.284`, log
  `log/bench-11-kissat-innovations-2026-05-13-19-23-21`.
- legacy focused queue plus saved focused phase: solved 9/11, PAR-2 `622.784`, log
  `log/bench-11-kissat-innovations-2026-05-13-19-31-41`.
- exact focused phase only (`SAT_FOCUSED_DECISION=pop-front`): solved 8/11, PAR-2 `868.899`, log
  `log/bench-11-kissat-innovations-2026-05-13-19-38-46`.
- exact focused queue only (`SAT_FOCUSED_PHASE=saved`): solved 9/11, PAR-2 `714.800`, log
  `log/bench-11-kissat-innovations-2026-05-13-19-47-52`.

Analysis:

- The implemented exact queue cursor mirrors Kissat's focused decision path more closely: focused
  decisions keep variables in the queue and advance a search cursor over assigned entries instead of
  popping variables out. Backtracking only updates the cursor when a newly unassigned variable has a
  newer focused stamp. It is available with `SAT_FOCUSED_DECISION=kissat`.
- The implemented exact focused phase override mirrors Kissat's focused `decide_phase` pattern:
  selected focused-mode switch windows force the initial or inverted initial phase before falling
  back to saved phases. It is available with `SAT_FOCUSED_PHASE=kissat`.
- Both exact pieces are rejected as default policies on this profiling set. The phase override is
  the larger isolated regression: it lost the `feistel_b64_k52_r17` solve and moved PAR-2 to
  `868.899`. The exact queue cursor alone kept the solved split but regressed PAR-2 to `714.800`,
  mainly by slowing `k32`, `k52`, and timetable.
- The accepted built-in default remains the previous focused behavior:
  `SAT_FOCUSED_DECISION=pop-front SAT_FOCUSED_PHASE=saved`. The final no-env confirmation
  reproduced the accepted 9/11 solved split with PAR-2 `622.999`; the small improvement over the
  previous `629.226` guard-on run is treated as run-to-run noise, not a new default win.
