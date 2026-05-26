# CLAUDE.md — SAT-playground Development Guide

## Project Overview

This repo builds Boolean SAT solvers iteratively in Rust, one directory per iteration, all conforming to the SAT Competition 2025 interface. Each iteration adds a technique on top of the previous one.

## Build & Run

```bash
# Build any iteration
cd solver/NN-name && bash build.sh        # runs: cargo build --release

# Run on a CNF instance
bash run.sh path/to/instance.cnf /tmp/proof_dir

# Run benchmarks
bash tools/bench.sh solver/NN-name
```

## Website / Docs Workflow

The repo has a static benchmark site under `docs/`, deployed at:

- **Live page:** `https://bjlkeng.io/SAT-playground/`
- **Main page source:** `docs/index.html`
- **Generated benchmark data:** `docs/data/medium-par2.json`
- **Generated README chart asset:** `docs/assets/medium-cumulative.svg`
- **Solver detail pages:** `docs/solvers/01-naive-dpll.html`, `docs/solvers/02-cdcl.html`
- **Shared solver-page stylesheet:** `docs/solver-pages.css`

### Current site conventions

- The hero title is **“Fun with Boolean SAT”**
- The intro paragraph should:
  - include one sentence describing Boolean SAT with a link to the Wikipedia page
  - say the goal is to understand SAT solvers more deeply
  - mention that all code in the repo was generated with AI coding tools
- The benchmark language on the site currently refers to:
  - **100 randomly selected instances from the SAT Competition 2025 main-track benchmark set**
  - local benchmark limits of **1800 seconds** and **16 GB RAM**
  - SAT Competition 2025 output/proof format via the official output page
- The main page theme is now light grey / white with blue and red accents
- Fixed-width / monospace text is intentionally kept for labels, legends, and benchmark metadata
- The main page section formerly called “Benchmark Logs” is now **“Solver Information”**
- The small machine footnote currently describes this host:
  - AMD Ryzen 5 5600 (6 cores / 12 threads)
  - 62 GiB RAM reported by `free -h`

### Site data generation

Regenerate the site payload and README chart with:

```bash
python3 tools/build_site_data.py
```

That script is the canonical source for:

- latest medium-run selection per solver
- benchmark metadata shown in the site notes
- `infoUrl` / `sourceUrl` used by the Solver Information cards
- the static SVG chart embedded at the top of `README.md`

### Current benchmark/site assumptions

- The site uses the **latest available** run per solver matching:
  - `100` instances
  - `1800s` timeout
- Repo solvers currently included in the site:
  - `01-naive-dpll`
  - `02-cdcl`
- `03-bcp` is skipped automatically until a matching medium run exists
- The cumulative chart is interactive:
  - hover with no selection shows an all-solvers overview
  - clicking a line or legend card locks focus to that solver
  - solver-focused hover shows per-instance details and rank on that instance
  - the tooltip disappears when the mouse leaves the chart area

### Solver information links

- Repo solvers (`01`, `02`) should link from the main page to the local detail pages under `docs/solvers/`
- Reference solvers should link to their appropriate external or vendored source page via `infoUrl`
- The `solver source` buttons in the cards should point to GitHub directory **tree** URLs, not `blob` URLs

### Updating solver detail pages

When adding or revising local solver pages:

- Keep them as simple static HTML pages under `docs/solvers/`
- Include:
  - a brief description of the technique(s) implemented
  - links to Wikipedia / papers where appropriate
  - high-level pseudocode
  - a short “code-level optimization diffs” section derived from the solver README
- `01-naive-dpll` and `02-cdcl` already have pages and can be used as the style/template baseline

### README integration

`README.md` now links to the live site and embeds `docs/assets/medium-cumulative.svg` near the top.

If the benchmark sample, chart styling, or site framing changes materially, update all three together:

1. `docs/index.html`
2. `tools/build_site_data.py`
3. `README.md`

### Verification for site changes

At minimum:

```bash
python3 tools/build_site_data.py

python3 - <<'PY'
from pathlib import Path
html = Path('docs/index.html').read_text()
start = html.index('<script>') + len('<script>')
end = html.index('</script>', start)
Path('/tmp/sat-playground-site.js').write_text(html[start:end].strip() + '\n')
PY
node --check /tmp/sat-playground-site.js
```

### Tracking benchmark logs used by the site

If the user wants the exact benchmark logs committed for the site:

- track the exact `summary.log` and `results.csv` used to generate the charts
- `log/` is ignored, so use `git add -f ...` for those files

Those tracked log files are the provenance for the plotted solver runs.

## Solver Interface Contract (SAT Competition 2025)

Every iteration MUST provide `build.sh` and `run.sh` at its top level:

- **`build.sh`**: No arguments. Builds the solver binary.
- **`run.sh <cnf_path> <output_dir>`**: Runs the solver. Prints to stdout. Writes `proof.out` to `<output_dir>` when UNSAT.

### Required stdout format

```
s SATISFIABLE
v 1 -2 3 0
```

or

```
s UNSATISFIABLE
```

or

```
s UNKNOWN
```

Rules:
- Exactly one `s` line per run
- `v` lines only when SAT — space-separated literals, terminated by `0`, max 4096 chars/line
- `c` comment lines are allowed anywhere
- Partial assignments are fine as long as every clause is satisfied

### UNSAT proofs

Write DRAT proof to `<output_dir>/proof.out`. This is required from every iteration.

## Code Conventions

- **Language:** Rust (each iteration is its own Cargo project)
- **Binary name:** `sat-solver` (consistent across iterations for tooling)
- **No external SAT solver dependencies** — the point is to build from scratch
- **Allowed crates:** standard utility crates (clap, anyhow, etc.) are fine; no SAT/SMT libraries
- **Each iteration directory is self-contained** — copy-and-modify from the previous iteration, don't use workspace dependencies between iterations
- **Test with small hand-crafted CNF files first**, then graduate to competition benchmarks

## Development Rules

- **Use red-green TDD for solver changes** — add or update a failing test first when practical, then implement until it passes before moving on.
- **Run smoke tests after every change** to a solver: `bash tools/smoke_test.sh solver/NN-name`
- **Only commit solver changes that pass the smoke test** (all 8 tests green). If a test fails, fix the solver before committing.
- **Never modify `tools/smoke_test.sh`** unless the user explicitly asks for changes to it.
- **Always commit and push** when the user asks — don't skip the push step.
- **Benchmark UNKNOWN is a failure for solver work:** `UNKNOWN` means the solver produced neither
  `SATISFIABLE` nor `UNSATISFIABLE` within the run contract. Treat every new or retained
  `UNKNOWN` on a benchmark/profile row that the baseline solves as a failed experiment, even if
  aggregate PAR-2 happens to improve. Do not describe UNKNOWN churn as harmless.
- **SAT/UNSAT/UNKNOWN result errors are correctness failures:** if a solver run reports the wrong
  status, returns a SAT model that does not satisfy the original CNF, fails to write or validate an
  UNSAT proof, or returns `UNKNOWN` for a row that should produce SAT/UNSAT, stop and debug the
  issue to root cause. Capture the exact repro command/log, analyze the failing code path, and fix
  the correctness issue before continuing promotion, tuning, or downstream work. Do not paper over
  a result error by changing it to `UNKNOWN`, suppressing validation, quarantining flags, or
  rerouting the requested path unless the user explicitly asks for that mitigation.
- **Debug every UNKNOWN before continuing:** when an experiment produces `UNKNOWN`, stop promotion
  work for that path, debug the cause, and fix the actual issue or revert the experiment. Do not
  hide an UNKNOWN-producing path by silently normalizing, quarantining, or rerouting requested
  feature flags unless the user explicitly asks for that mitigation. Rerun the exact affected
  configuration and do not leave the task as complete until that configuration no longer produces
  `UNKNOWN` on the relevant baseline-solved rows.
- **Solver 11 default/fast promotion gate:** before promoting any solver 11 default or fast
  profile change, run a clean solver 10 comparison on the same benchmark set and pass
  `python3 tools/check_solver11_promotion.py --solver10 <solver10-results.csv> --previous <prior-solver11-results.csv> --candidate <candidate-results.csv> --timeout <seconds> --memory-mb <MB>`.
  The gate must record process sanity, matching instance/timeout sets, machine metadata, and the
  explicit decision when a candidate improves prior solver 11 but still loses to solver 10.
- **Do not promote one-instance overfit guards:** if a solver 11 default/profile improvement is
  dominated by one benchmark row, one formula-family classifier, or DIMACS input-order behavior,
  treat it as a diagnostic until a mechanism-level fix explains the win. Do not hard-code guards
  for a single instance shape as a default promotion without shuffled-order validation, a broader
  benchmark sample, and a written revert/rollback plan. Prefer fixing the underlying mechanism
  (for example watch selection, preprocessing budget, or search policy) over preserving a lucky
  trajectory from clause or literal order. Use the shuffle-sensitivity workflow
  (`python3 tools/shuffle_sensitivity.py --instances <cnf...> --seeds <seed-list>`) to record
  per-seed status, runtime, conflicts, decisions, and propagations before treating
  input-order-sensitive wins as promotion evidence.
- **Discord automatic task notices:** For future background ACP/subagent tasks in Discord, suppress automatic `Background task done/failed` channel notices by setting the task notify policy to `silent` once the run/task id exists.
- **Discord notifications:** When reporting background task completion or failures in Discord, @mention bjlkeng as `<@817490773179760662>` so Discord shows a badge notification.

## Iteration Workflow

When creating a new iteration:

1. Copy the previous iteration directory: `cp -r solver/NN-prev/ solver/MM-name/`
2. Update `Cargo.toml` package name
3. Add or update tests first when practical so the change follows a red-green TDD loop
4. Implement the new technique
5. Add unit tests for the new feature
6. Run `bash tools/smoke_test.sh solver/MM-name` — all 8 tests must pass
7. Run against benchmarks and record results in the iteration's `README.md`
8. Ensure `build.sh` and `run.sh` still work

## Testing

```bash
# Unit tests within an iteration
cd solver/NN-name && cargo test

# Smoke test — runs all 8 test instances (4 SAT + 4 UNSAT)
bash tools/smoke_test.sh solver/NN-name
```

### Smoke Test Suite

Located in `tests/cnf/`, these are small hand-crafted instances that run in under a second:

**SAT instances** (`tests/cnf/sat/`):
- `unit.cnf` — single unit clause (trivial)
- `two_clause.cnf` — 2 vars, 2 clauses
- `three_sat.cnf` — 5 vars, 6 clauses (small 3-SAT)
- `all_positive.cnf` — 3 vars, all positive literals

**UNSAT instances** (`tests/cnf/unsat/`):
- `contradiction.cnf` — x AND NOT x
- `empty_clause.cnf` — contains an empty clause
- `pigeonhole_3_2.cnf` — 3 pigeons, 2 holes (classic)
- `chain_unsat.cnf` — implication chain forcing contradiction

The smoke test script (`tools/smoke_test.sh`) builds the solver, runs all instances, checks the `s` line, and verifies SAT assignments satisfy the formula.

## DIMACS CNF Format Reference

```
c optional comment
p cnf <num_vars> <num_clauses>
<lit> <lit> ... 0        ← each line is one clause
```

- Variables: positive integers `1..num_vars`
- Literals: variable or its negation (e.g., `3` or `-3`)
- Clause: list of literals terminated by `0`
- No clause may contain both `x` and `-x`

## Benchmarks

Download from: `https://benchmark-database.de/?track=main_2025&context=cnf`

```bash
cd benchmarks
wget -O track_main_2025.uri "https://benchmark-database.de/?track=main_2025&context=cnf"
wget --content-disposition -i track_main_2025.uri
```

Competition scoring is PAR-2: sum of runtimes for solved instances + 2 × 5000s for each unsolved instance. Lower is better.

## Key SAT Competition 2025 Facts

- **Main Track:** 5000s timeout, 30 GB RAM, 8-core Xeon, PAR-2 scoring
- **Winner:** kissat-sc2024 (PAR-2: 2788, 306/400 solved)
- **Proof formats:** DRAT → LRAT (via drat-trim) → cake_lpr; or DRAT → GRAT (via gratgen) → gratchk; or VeriPB → cake_pb_cnf
- **Benchmark source:** https://benchmark-database.de/?track=main_2025&context=cnf

## Code-Level Optimization Workflow

**Only run this when the user explicitly asks for it.** Do not automatically optimize after implementing a solver.

After implementing a new solver iteration, this optimization loop squeezes out performance. Most
passes should be non-algorithmic, but if the user explicitly asks to optimize using a named solver
idea or benchmark gap, it is acceptable to test that focused feature as part of the loop.

### Procedure

1. **Pick a target**: If the user names a reference solver or competition gap, choose one concrete competition instance where the reference is faster than the repo solver but still short enough for repeated iteration. Create a one-instance benchmark directory for it and keep using that target until the user redirects.
2. **Baseline**: Run `bash tools/bench.sh -d benchmarks/profiling solver/NN-name` and record PAR-2. The profiling-suite default is 300 seconds per instance. For a one-instance target, also run `bash tools/bench.sh -t <seconds> -m 16384 -d <one-instance-dir> solver/NN-name` and record the target-instance runtime. If comparing to MiniSat-style simplification, also capture useful reference variants such as `minisat -no-pre`, `minisat -no-elim`, and full `minisat`.
3. **Profile first**: Use a profiler such as `perf stat` / `perf record` / `perf report` on the target instance before changing code. Use the profile to choose the next implementation slice. If the release binary is stripped and symbols are needed, rebuild for profiling with `CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=1 RUSTFLAGS="-C target-cpu=native" cargo build --release`.
4. **Check opportunity size before coding**: For simplification ideas, do a quick measurement pass over the target formula before implementing. Examples: count duplicate clauses, pure literals, candidate variables, binary subsumption hits, or self-subsuming-resolution opportunities. Do not implement an idea when the measured opportunity is negligible.
5. **Report diagnostic solver stats for every instance analysis**: Capture enough run data to identify whether the bottleneck is simplification, propagation, conflict learning, or search-path sensitivity. At minimum report pre/post-preprocessing variables, clauses, and literal counts; preprocessing time; eliminated variables, resolvents, subsumed clauses, strengthened literals, and root assignments when available; final result and runtime; conflicts, decisions, propagations, restarts, learned-clause count, reduce-DB calls, and any timeout/error status. When profiling, include propagation time or propagation throughput, conflict-analysis time if measured, simplification/proof I/O time if visible, and the profile/log paths used. When testing sensitivity, report the exact mode flags, seed/order/literal-order choices, preprocessing toggles, and per-instance deltas so the cause is not guessed from PAR-2 alone.
6. **Iterate** (at least 10 attempts unless the user asked for a narrower experiment): Make one change at a time, benchmark it against the current accepted baseline, and keep it only if it improves the target metric by more than `3%` without introducing or retaining any `UNKNOWN`/timeout/error row that the accepted baseline solves. Recalculate the accepted baseline and keep threshold after every kept change. Revert changes that improve PAR-2 by `3%` or less, worsen aggregate PAR-2, fail correctness checks, introduce a new unsolved/UNKNOWN row, or only shift time into a slow new implementation.
7. **Stop losers early when safe**: For long target-instance runs, if an experiment has already passed the `>3%` keep cutoff without finishing, stop the run, mark the attempt rejected, and revert it instead of waiting for the full timeout.
8. **Tune algorithmic features carefully**: When testing a focused solver idea such as bounded variable elimination, run small parameter sweeps around the first good result. More simplification can damage CDCL search even when preprocessing is cheap, so validate caps and cost thresholds empirically rather than assuming monotonic improvement.
9. **Retest interactions**: A previously rejected micro-optimization can become worthwhile after a later kept change changes the profile. Retest it only when profiler evidence or timing data suggests the interaction could now clear the `>3%` threshold.
10. **Optimize the new feature if needed**: If a feature is conceptually promising but too slow, profile the modified solver and iterate on that feature's implementation. Keep it only after the optimized version clears the `>3%` improvement threshold.
11. **Correctness checks**: Add or update focused tests when practical before changing solver behavior. Run `cargo test` and the full smoke suite after every kept solver change, and rerun them after reverting failed experiments if the revert touched solver logic.
12. **Record**: Document every *successful* improvement and its PAR-2 or target-instance runtime impact in the solver's `README.md`, including machine environment metadata, benchmark log paths, profile paths, and the profiler evidence that motivated it. Also document important rejected attempts with their measured runtime or reason for skipping so future loops do not repeat them blindly.

### Debugging Optimization Regressions

When a targeted optimization regresses or gives contradictory benchmark results, run a focused
debug pass before changing more code:

1. **Compare modes on the same fixed-time target**: Use identical timeout, proof setting, seed/order
   knobs, and instance file. Capture `SAT_TRACE_PREPROCESS=1` and `SAT_TRACE_SEARCH_INTERVAL=<N>`
   so preprocessing work, conflicts, decisions, propagations, restarts, learned-clause mix, glue,
   reductions, and deleted/gc counters can be compared at similar conflict counts.
2. **Separate preprocessing from search**: If preprocessing is substantial, first confirm both modes
   report identical preprocessing stats. Then use `perf stat -D <milliseconds>` with a delay just
   past preprocessing to collect search-only counters.
3. **Use hardware counters, not just wall time**: At minimum compare `cycles`, `instructions`,
   `branches`, `branch-misses`, `L1-dcache-loads`, `L1-dcache-load-misses`, `dTLB-loads`,
   `dTLB-load-misses`, `cache-references`, and `cache-misses`. Normalize misses by propagations
   or conflicts from trace output; otherwise search-path differences can hide the real cost.
4. **Prefer lower-noise signals before drawing conclusions**: Do not over-index on PAR-2, solved
   count, or one/few instance outcomes when search behavior can be randomized or highly
   path-sensitive. Use more robust signals first: propagation throughput, conflicts per second,
   decisions, propagation count, clause inspections, cache/TLB misses normalized by work, and
   fixed-time trace deltas. Treat instance-level solve/timeout changes as supporting evidence until
   the low-level counters and search statistics explain them.
5. **Sample the suspected event**: Use `perf record -e cache-misses -g --call-graph dwarf` and
   inspect with `perf report --stdio --no-children --sort symbol`. Use `perf annotate --stdio
   --source --symbol '<symbol>'` when the hot function is known. Record exact profile paths.
6. **Check opportunity shape**: For representation changes, count formula features such as binary
   clause percentage, per-literal binary degree distribution, and max degree. High degree or sparse
   occurrence patterns often explain TLB/cache behavior better than aggregate clause counts.
7. **Explain search-path effects separately**: If conflict count, learned glue, learned binary/long
   mix, or root-unit learning diverge, state that separately from microarchitectural overhead. A
   faster local primitive can still lose by producing a worse CDCL trajectory.
8. **Turn the finding into a code hypothesis**: Tie every proposed fix to a measured hot source line
   or data access pattern, such as "avoid arena header loads in the binary implication loop" rather
   than a generic "improve cache locality" note.

### Reference Solver Gap-Closing Strategy

Use this when the user asks to make one solver match or beat a reference solver on a named feature,
such as MiniSat-style simplification. The goal is to explain and reduce the measured work gap, not
just chase one wall-time number.

1. **Measure both implementations on the same inputs first**: Run the repo solver and the reference
   with the same instance set, timeout, memory limit, proof setting, and decompression path. Keep
   the exact `results.csv` paths. Compare solved count, PAR-2, per-instance time, and which
   instances changed when the timeout changes.
2. **Separate feature time from whole-solver time**: If the target is preprocessing, simplification,
   propagation, conflict analysis, or another phase, add phase timing to both implementations when
   possible. Do not assume a full-solve timeout is caused by the phase under investigation; confirm
   whether preprocessing, search trajectory, or proof/checking time is responsible.
3. **Instrument matching work counters in both implementations**: Add comparable counters on each
   side before optimizing. For simplification, useful counters include BSR runs, queue drivers,
   root-unit drivers, driver literals, candidate scans, self/deleted/limit skips, relation calls,
   length rejects, abstraction rejects, subsumed clauses, strengthened literals, occurrence-clean
   calls, occurrence entries scanned/removed, eliminated variables, resolvents, root assignments,
   and post-preprocessing variable/clause/literal counts.
4. **Compare work shape before micro-optimizing**: Large gaps in counters usually indicate a
   semantic, scheduling, data-structure, or ordering mismatch. In the solver 10 vs MiniSat simp
   pass, Rust initially did about `54.0M` BSR drivers and `21.6B` candidates while MiniSat did about
   `17.8M` drivers and `9.8B` candidates. That pointed to wrong work scheduling, not just slow Rust
   inner loops.
5. **Read the reference control flow literally**: Check when the reference queues work, drains work,
   updates heaps, clears marks, and returns to outer loops. Match important scheduling boundaries
   before changing algorithms. The solver 10 fix came from noticing that MiniSat gathers touched
   clauses at the outer simplification-loop boundary, then drains the elimination heap while only
   running BSR on immediate queue/trail work; Rust was gathering touched clauses after every
   eliminated variable, causing repeated extra BSR work.
6. **Test structural parity experiments even if they are not kept**: Try changes that isolate one
   suspected mismatch, such as touched-variable order, heap tie-breaking, update mechanics, or queue
   marking. Keep exact stats for rejected attempts. A MiniSat-style indexed heap reduced solver 10
   work but was slower than the lazy heap, so it was rejected while the loop-structure fix was kept.
7. **Accept fixes by both work counters and wall time**: A strong parity fix should move counters
   toward the reference and improve runtime. After the solver 10 loop fix, BSR drivers moved from
   `54.0M` to `18.3M`, candidate scans from `21.6B` to `9.9B`, and Kakuro preprocessing dropped from
   about `67.5s` to `37.9s` before further cleanup.
8. **Remove diagnostic overhead from normal runs**: Debug counters in hot loops can dominate the
   remaining gap. Keep diagnostics available, but compile or route them out of the normal hot path
   using const generics, feature flags, or separate traced helper variants. In solver 10, disabled
   trace branches left in BSR hot loops accounted for several seconds; compiling them out reduced
   Kakuro preprocessing to about MiniSat's preprocessing time.
9. **Re-run the full target set after each accepted phase fix**: A phase-level win can expose a
   different whole-solver gap. Solver 10 matched MiniSat on Kakuro preprocessing after the loop and
   trace fixes, but the 120s full-solve run still timed out on Kakuro because the remaining problem
   was search, not preprocessing.
10. **Report the residual gap by cause**: End with a table or concise summary that separates phase
    parity from full-solver behavior. Include instances where the repo solver is faster, where the
    reference is faster, timeout-only differences, and the one or two instances that dominate PAR-2.
    This prevents repeating preprocessing work when the next gap is actually search-path sensitivity.

### Investigating Why Ported Features Don't Help

Use this when an opt-in feature (kissat-ema restart, lbd-tiered reducer, focused-stable mode,
chrono backtracking, etc.) has landed but does not improve — or actively regresses — the
benchmark. The goal is to attribute the regression to specific implementation gaps versus
"the feature is wrong for this workload," and to produce per-feature code-level recommendations
rather than parameter-tuning suggestions.

This workflow extends `Reference Solver Gap-Closing Strategy` above by adding multi-config
ablation, source-level diff against the reference, and chaos-aware analysis. Use a separate git
worktree (`git worktree add /tmp/<name> <ref>`) when another agent may be working on the repo.

1. **Build a multi-config ablation matrix, not a one-vs-one comparison**: A single feature flip
   (A vs A+feature) cannot distinguish trajectory effects from execution effects. Define at least
   five configs:
   * `A_baseline`: legacy defaults (no opt-in features)
   * `B_metadata_only`: feature's bookkeeping only (e.g. `SAT_USE_LBD=on` without policy change)
   * `C_one_feature`: just the suspect feature
   * `D_other_feature`: just an orthogonal suspect
   * `E_combined`: both
   * `F_full_stack`: all related opt-ins together
   Confirm `B` matches `A` on conflicts/decisions/propagations to verify bookkeeping is free.
   Anything that diverges past `B` is signal.

2. **Use the full profiling suite, not one instance**: A single instance is a single data point.
   The 10-instance `benchmarks/profiling/` suite is diverse enough to show feature effects across
   SAT/UNSAT, structured/random, and several problem families. A feature is rarely uniformly bad;
   the question is *which instances does it help, which does it hurt, and how do those split.*

3. **Run sequentially in a separate worktree, with `SAT_STATS_JSON=on` and `/usr/bin/time -v`**:
   Capture per-(config, instance) wall time, conflicts, decisions, propagations, restarts,
   `learned_clauses_final`, `reduce_db_calls`, max RSS, and minor page faults. JSON_STATS is
   compact and parseable. Use `SAT_LIMIT_WALL_SEC` plus a `timeout` outer wrapper so timeouts are
   classified consistently.

4. **Decompose every regression as work × speed against `A_baseline`**: For each `(config,
   instance)` row, compute
   * `work_ratio = conflicts_cfg / conflicts_A` (search trajectory effect)
   * `speed_ratio = (props/s)_A / (props/s)_cfg` (per-event execution effect)
   * `net = work × speed` and compare to measured wall ratio
   Trajectory-only features (EMA restart) typically show `speed ≈ 1.0` and `work` ranging widely.
   Execution-only effects (DB bloat, watcher growth) show `work ≈ 1.0` and `speed > 1`. Mixed
   features (combined kissat-style) show both moving. A row where `net ≈ measured wall ratio`
   confirms the decomposition; a large gap suggests a third factor (often GC, allocation, or
   proof writing).

5. **Read the reference source line-by-line for the suspect feature**: Find the vendored
   reference under `benchmarks/reference-solvers/<name>/src/` and read the file that implements
   the feature (e.g. `restart.c`, `reduce.c`, `tiers.c` for kissat). Look for *execution model*
   differences beyond parameter names:
   * what the reference does on each event (restart, reduce, decide, learn)
   * what state survives between events (trail reuse, used counters, tier assignments)
   * which side-effects happen at which boundary (queue drains, heap rebuilds, EMA resets)
   Solver 11 vs kissat: same restart trigger, but kissat does `reuse_focused_trail` while solver
   11 does `backtrack(0)` — same *decision*, completely different *execution*. The reference
   docs/comments rarely call these out; reading the C is non-negotiable.

6. **Quantify each implementation gap against the data**: For every gap you find, predict which
   instance behavior would change if the gap were closed. Verify the prediction matches the
   actual regression pattern. E.g. "no trail reuse on restart" predicts `props/decision` should
   drop sharply under EMA restart — verified on mp1 under `C_lbd_ema` (205 → 44, 5× drop).
   Without this verification, source-diff findings are speculation.

7. **Trace one critical instance under at-least two configs at a fixed interval**: Use
   `SAT_TRACE_SEARCH_INTERVAL=N` (e.g. 20000) to log conflict-aligned snapshots of seconds,
   conflicts, decisions, propagations, restarts, current level, trail length, and
   `live_learned_clause_count`. Compare the trajectories side by side. If the trajectories are
   *identical* through some prefix and diverge late, you have **phase-boundary chaos**: no
   parameter tuning will reliably fix it because the win/loss is determined by which specific
   variable VSIDS picks at one critical decision. Document this as "needs algorithmic
   simplification (inprocessing) or accept as a coin flip" rather than continuing to tune.

8. **Run parameter sweeps that probe specific hypotheses, not blind tuning**: For each suspected
   bottleneck, define a knob change that should rescue the failing instances if the hypothesis
   is right. Sweep on the failing instances at a slightly reduced timeout (e.g. 240s vs 300s) so
   more variants fit. Examples:
   * "EMA restarts fire too often deep in the trail" → sweep `SAT_RESTART_BLOCK_MARGIN`
   * "Reducer deletes too early" → sweep `SAT_REDUCE_DB_INIT`
   * "Phase-saving misleads on HWMCC" → sweep with/without `SAT_PHASE=target-then-saved`
   Sweeps that rescue zero failing instances refute the hypothesis. Sweeps that rescue one or
   two confirm the gap but rarely fix everything — record the trade-off (which instances regress
   when the knob rescues another) so you do not over-promote.

9. **Confirm no single config beats the baseline in aggregate, even after sweeps**: This is the
   honest end-state of most feature investigations. If even the best parameter-tuned variant
   loses to baseline on PAR-2, the conclusion is "the implementation needs the code-level fixes
   identified in step 5/6, parameter tuning is not enough."

10. **Cross-check existing beads before creating new ones**: Use `bd search <keyword>` for every
    feature/file/symbol you plan to write a bead about. The `bd ready` / `bd list --label
    phase1` views are usually 30+ entries deep. Find existing beads for the same area and either
    add a new evidence note (`bd note <id> "..."`) or create a new bead linked to the existing
    one with `bd link <new> <existing> --type related`. Reframe-style discoveries (e.g. "the old
    bead targeted the wrong dimension") deserve a note pointing at the new bead, not silent
    duplication.

11. **Write findings as code-level recommendations, not knob tuning**: Each gap should map to a
    concrete code change (file + function + a brief sketch of the fix) plus a kissat/reference
    citation. Recommendations like "try margin = 1.4" are weaker than "implement
    `reuse_focused_trail` in `perform_restart_if_pending` matching `kissat/src/restart.c:69-84`"
    because the latter survives a parameter sweep failure. Order recommendations by expected
    ROI: implementation gaps that explain the largest measured ratios first (e.g. trail reuse
    when `props/decision` is the dominant drop).

12. **Persist the artifacts under `log/<investigation-name>/`**: Save the ablation script,
    sweep script, raw `results.csv` and `sweep_results.csv`, per-(config, instance)
    `stdout/stderr/stats.json`, an `analysis.py` for the decomposition, and a `FINDINGS.md` and
    `DEEPER_FINDINGS.md` (initial + follow-up). Quote specific stat lines, source line numbers
    in both repos, and the exact env-var combination for each config. Future investigations
    should be able to re-derive your conclusions from these artifacts without re-running
    anything.

### Standard Optimizations (apply to every solver)

**Cargo.toml release profile** (always include):
```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
overflow-checks = false
```

**build.sh** (always include):
```bash
[[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

## Running Full/Medium SAT Competition 2025 Benchmarks

When running benchmarks against `benchmarks/sat-comp-2025/` or `benchmarks/sat-comp-2025-medium/`, **always use a cron job** so the run survives if the Claude session ends. Use `tools/run_bench_reference.sh` as the wrapper (it logs output and manages sentinel files).

Important distinction:

- `tools/run_bench_reference.sh` / `tools/bench_reference.sh` are only for the reference solvers
- For an in-repo solver such as `solver/03-bcp`, use the **same one-shot cron pattern** but invoke `tools/bench.sh` directly and create a custom running/done sentinel if needed
- After the job starts, immediately remove the one-shot cron line so it cannot re-fire

### One-shot cron pattern (preserves existing crontab)

```bash
# 1. Pick a time ~2 min from now
date '+%M %H %d %m'   # e.g. "17 21 11 04"

# 2. Append one-shot entry (MUST preserve existing crontab)
EXISTING=$(crontab -l 2>/dev/null)
echo "$EXISTING
19 21 11 04 * /bin/bash /home/bojji/code/SAT-playground/tools/run_bench_reference.sh -t 1800 -m 16384 -d /home/bojji/code/SAT-playground/benchmarks/sat-comp-2025-medium" | crontab -

# 3. Verify it started (wait ~2 min, check sentinel)
cat log/bench_reference_RUNNING

# 4. IMMEDIATELY clean up the cron entry so it doesn't re-fire
crontab -l | grep -v 'run_bench_reference' | crontab -

# 5. Monitor progress
tail -f log/bench_reference_*.log
# Or check instance counts:
wc -l log/bench-kissat-latest-*/results.csv
```

### Key flags for run_bench_reference.sh

- `-t <seconds>` — per-instance timeout (default: 1800)
- `-m <MB>` — memory limit (default: 16384)
- `-d <path>` — benchmark directory (default: benchmarks/sat-comp-2025)
- Positional args: solver names (default: kissat-latest kissat-sc2024 minisat)

### Monitor progress

- `log/bench_reference_RUNNING` — exists while benchmark is active (contains PID and start time)
- `log/bench_reference_DONE` — created on completion (contains log file path)
- Results CSVs: `log/bench-<solver>-<timestamp>/results.csv`

### Important: kill ALL solver processes when stopping

```bash
pkill -f 'bench_reference'; pkill -f 'run_bench_reference'
pkill -f 'kissat.*\.cnf'; pkill -f 'minisat.*\.cnf'
# Verify: ps aux | grep -E 'kissat|minisat|bench_reference' | grep -v grep
```

Processes spawned by the script can outlive the parent if only the wrapper is killed. Always kill the solver binaries directly too.

## Status Reporting

When the user asks for status (e.g. "status?", "how's it going?", "what's running?"):

1. **CPU usage**: Run `ps aux --sort=-%cpu | head -20` and report SAT solver processes (sat-solver, minisat, kissat, etc.) with their CPU%, runtime, and instance name
2. **Running solvers**: Run `pgrep -a sat-solver; pgrep -a minisat; pgrep -a kissat` to identify active solver processes
3. **Benchmark progress**: Find the most recent active benchmark log:
   - Check `log/bench_reference_RUNNING` for reference solver runs
   - Find the latest `log/bench-*` or `log/bench_reference_*` directory/file
   - Report how many instances are solved vs total, and current instance being worked on
   - Always include a detailed completed-result breakdown for running benchmarks: SAT count, UNSAT count, timeout count, error count, and any other result statuses present in `results.csv`
   - Use `tail` on the log file or `wc -l` on `results.csv` to get progress counts

## Finding Current State

Do not treat this file as the source of truth for the repo's exact current solver lineup, benchmark
state, or iteration status. Instead:

- List the currently available solver iterations with `ls solver`
- Read `solver/NN-name/README.md` for each iteration's actual scope, validation status, and latest recorded benchmark notes
- Inspect `solver/NN-name/src/main.rs` and `Cargo.toml` for the real implementation and package identity
- Use `git status --short` and `git log --oneline -- solver/NN-name` to see local changes and recent history for a solver
- Check `benchmarks/reference-solvers/` for the current vendored reference solvers
- Check `tools/checkers/` and `tools/setup_checkers.sh` for the currently configured proof checkers
- Read `benchmarks/profiling/README.md` for the current 10-instance profile suite (selected from SAT Competition 2025 medium-track, each <300 s on solver 10); the previous 6-instance set is preserved at `benchmarks/profiling/legacy/`
- Check `benchmarks/profiling/`, `benchmarks/crypto/`, `benchmarks/random-3sat/`, and the generator tools under `tools/` for the current benchmark inputs
- Use `tools/bench.sh`, `tools/bench_reference.sh`, and the latest `log/bench-*` directories to see the current benchmark workflow and outputs

## Common Pitfalls

- Forgetting the trailing `0` on `v` lines
- Printing `v` lines when the result is UNSAT
- Not handling empty clauses (immediately UNSAT)
- Not handling unit clauses at the top level
- Off-by-one on variable indexing (DIMACS is 1-based)
- Exceeding 4096 characters on a single `v` line

<!-- BEGIN BEADS CODEX SETUP: generated by bd setup codex -->
## Beads Issue Tracker

Use Beads (`bd`) for durable task tracking in repositories that include it. Use the `beads` skill at `.agents/skills/beads/SKILL.md` (project install) or `~/.agents/skills/beads/SKILL.md` (global install) for Beads workflow guidance, then use the `bd` CLI for issue operations.

### Quick Reference

```bash
bd ready                # Find available work
bd show <id>            # View issue details
bd update <id> --claim  # Claim work
bd close <id>           # Complete work
bd prime                # Refresh Beads context
```

### Beads Viewer (`bv`)

`bv` is installed at `~/.local/bin/bv` as a terminal sidecar for browsing and analyzing Beads. It reads `.beads/beads.jsonl`, so refresh that export after changing beads:

```bash
bd export -o .beads/beads.jsonl
```

For agent/automation use, do **not** run bare `bv` because it opens the interactive TUI. Use robot commands instead:

```bash
bv --robot-help       # list robot commands and contracts
bv --robot-triage     # full triage JSON with top picks, blockers, health
bv --robot-next       # single highest-priority recommendation
bv --robot-plan       # dependency-respecting execution tracks
bv --robot-insights   # graph metrics, bottlenecks, cycles, critical paths
```

Use `bd` as the source of truth for creating, updating, closing, and linking beads. Use `bv` for read-only triage, graph insight, and planning output. Do not hand-edit `.beads/beads.jsonl`; regenerate it with `bd export` when needed.

### Rules

- Use `bd` for all task tracking; do not create markdown TODO lists.
- Run `bd prime` when Beads context is missing or stale.
- Keep persistent project memory in Beads via `bd remember`; do not create ad hoc memory files.
<!-- END BEADS CODEX SETUP -->
