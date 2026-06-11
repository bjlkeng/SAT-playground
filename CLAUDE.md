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

- **Primary objective is the LEXICOGRAPHIC metric over the profile20 suite, measured across seeds.**
  The decision metric for solver work, in strict priority order, is: **(1) solved instances** across
  the seed runs [primary]; **(2) total conflicts** on the instances that tie on solved-count
  [tiebreak, lower is better]; **(3) aggregate PAR-2** [supplemental, breaks a conflicts tie only].
  This is measured over the whole `benchmarks/profile20` set — 10 easy control instances (reused
  verbatim from `benchmarks/profiling`) + 10 hard "headroom" instances solver-10 cannot solve within
  5 min but kissat can — reported as an all-20 aggregate with the easy-10 / hard-10 split.
  (`benchmarks/profiling` is the legacy easy-only control.) **Conflicts rank above PAR-2 on purpose:**
  a faster-per-op change that does *more search* (more conflicts) at equal solved-count is NOT a win,
  even if its wall-clock/PAR-2 looks better — this is the binary_fast lesson (it is 3× faster per
  propagation yet explores a longer search; see `log/feature-deepdive-2026-06-01/`). PAR-2 is
  contention-sensitive; conflicts are deterministic per (config, seed) and contention-immune, which
  is why conflicts are the trustworthy tiebreak and PAR-2 is only supplemental.
- **Every feature measurement that informs a keep/turn-on/promote decision MUST use a multi-seed
  per-instance sweep (default N=10 seeds), never a single run.** Single-seed per-instance verdicts on
  profile20 are unreliable: `case9`, `sudoku-N30-12`, and `REGRandom-K4-L1-Seed40` are seed-fragile
  (e.g. the default solves case9 on only ~1/5 seeds), so n=1 conflates "systematically better/worse"
  with a lucky draw and has produced flat-wrong verdicts. Run the sweep with
  `python3 tools/feature_ablation.py --seedgate --configs <tag> [--seeds 10] [--timeout 900]` per
  config, then decide with `python3 tools/check_solver11_promotion.py --multiseed ...`. Report
  per-instance **solve-rate (X/N seeds)**, median conflicts, **P(feature>default)** stochastic
  dominance (0.50 = no effect / lottery), and compare any aggregate delta against the **default's own
  seed-spread (±stdev)** as the noise floor. Flag any seed-fragile instance (solve-rate < N/N) — its
  single-seed numbers must not silently drive a verdict. Fast single-instance / single-seed runs are
  fine for *mid-iteration exploration while coding*; they are not sufficient for a keep/commit
  decision. (Conflicts-as-tiebreak assumes the solver is deterministic per (config, seed); if a
  feature introduces nondeterminism that breaks that, fall back to repeated PAR-2 runs on quiet cores
  and say so.)
- A change that regresses some instances — including flipping a solved instance to a timeout — is
  acceptable as long as it wins the lexicographic metric beyond seed-noise. Do not reject a
  net-positive change because individual rows got slower, and do not require a feature to help every
  instance. The one thing the metric never overrides is correctness (see the correctness rule below).
  Use kissat and other reference
  solvers for inspiration, hypotheses, and comparison — matching their behavior or implementation
  1-to-1 is **not** a goal; adapt freely and keep whatever wins aggregate PAR-2.
- **Use red-green TDD for solver changes** — add or update a failing test first when practical, then implement until it passes before moving on.
- **Run smoke tests after every change** to a solver: `bash tools/smoke_test.sh solver/NN-name`
- **Only commit solver changes that pass the smoke test** (all 8 tests green). If a test fails, fix the solver before committing.
- **Never modify `tools/smoke_test.sh`** unless the user explicitly asks for changes to it.
- **Always commit and push** when the user asks — don't skip the push step.
- **A new timeout / honest UNKNOWN is a PAR-2 cost, not an automatic failure:** under the
  aggregate-PAR-2 objective, an instance that flips from solved to unsolved — a timeout, or an
  honest resource-limit `UNKNOWN` that actually consumed its time/memory budget — is acceptable as
  long as aggregate PAR-2 over the profile20 suite still improves beyond run-to-run noise. An
  unsolved row is already priced into PAR-2 as `2 × timeout`, so a net-positive change wins on the
  total even when it loses individual rows. Do not reject such a change, and do not describe the
  per-instance regression as a correctness problem. Two carve-outs below still apply.
- **Correctness errors are never acceptable, regardless of PAR-2:** if a solver run reports the
  wrong status (claims SAT/UNSAT incorrectly), returns a SAT model that does not satisfy the
  original CNF, or fails to write/validate a required UNSAT proof, stop and debug to root cause.
  Capture the exact repro command/log, analyze the failing code path, and fix the correctness issue
  before continuing promotion, tuning, or downstream work. Do not paper over a result error by
  changing it to `UNKNOWN`, suppressing validation, quarantining flags, or rerouting the requested
  path unless the user explicitly asks for that mitigation. Aggregate PAR-2 does not buy back a
  wrong answer or a missing/invalid proof.
- **A premature `UNKNOWN` is a bug, not a PAR-2 result:** if a configuration returns `UNKNOWN`
  *without consuming its time/memory budget* (it bails early, normalizes an error to `UNKNOWN`, or
  a feature flag short-circuits the requested path), treat it as a likely bug and root-cause it
  before trusting the run — it is not the same as an honest timeout. Do not hide such a path by
  silently normalizing, quarantining, or rerouting requested feature flags unless the user
  explicitly asks for that mitigation. Distinguish "ran out of budget" (a PAR-2 cost, fine if the
  total wins) from "answered `UNKNOWN` early" (debug it).
- **Solver 11 default/fast promotion gate:** the decision metric is the **LEXICOGRAPHIC
  solved→conflicts→PAR-2 metric over profile20, measured across N=10 seeds**, candidate vs the
  current solver-11 default. Produce one multi-seed TSV per config with
  `python3 tools/feature_ablation.py --seedgate --configs <tag> --seeds 10 [--timeout 900]` (for
  solver10, the prior default, and the candidate), then run the gate:
  `python3 tools/check_solver11_promotion.py --multiseed --solver10 <solver10.tsv> --previous <prior-default.tsv> --candidate <candidate.tsv> --timeout <seconds> --memory-mb <MB>`.
  Promote a candidate that wins lexicographically vs the prior default beyond seed-noise. Solver 10
  is a **lexicographic regression floor**: do not ship a default that loses to solver 10 on the
  lexicographic metric (fewer solved, or equal-solved with more conflicts), but the floor is
  informational pressure, not the keep/revert metric. The floor is **not** a per-instance one: an
  instance that regresses from solved (by solver 10) to a timeout is priced into the lexicographic
  comparison and does **not** by itself fail the gate — only a **SAT↔UNSAT correctness
  contradiction** (per (instance,seed), vs solver 10 or the prior default) fails it (no metric buys
  back a wrong answer). The gate records process sanity, matching (instance,seed) cell sets,
  per-config solved/conflicts/PAR-2 scores, the lexicographic decision vs both the prior default and
  the solver-10 floor, and the explicit note when a candidate beats the prior default but loses the
  floor. **The single-CSV `check_gate` path (aggregate-PAR-2-only, no `--multiseed`) is retained for
  legacy/quick comparisons but is no longer the promotion decision; the multi-seed lexicographic path
  is authoritative.**
- **Do not promote overfit guards or lucky-order wins (distinct from honest per-instance
  regressions):** a real mechanism that improves aggregate PAR-2 while regressing some individual
  rows is a *good* change — keep it. What this rule forbids is a different thing: an apparent
  aggregate win that is actually a hard-coded guard for one instance shape, a one-row formula-family
  classifier, or a fragile DIMACS-input-order / clause-or-literal-order coincidence that will not
  survive a reshuffle. Those are not durable PAR-2 wins; they are noise dressed up as a win. Treat
  such a candidate as a diagnostic until a mechanism-level explanation holds. Do not hard-code
  guards for a single instance shape as a default promotion without shuffled-order validation and a
  written revert/rollback plan. Prefer fixing the underlying mechanism (for example watch selection,
  preprocessing budget, or search policy) over preserving a lucky trajectory from clause or literal
  order. Use the shuffle-sensitivity workflow
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

### Concurrent benchmarking with other agents

You do **not** have to wait for another agent's benchmark to finish before running
your own. Concurrent benchmark runs are acceptable as long as the *combined*
benchmark CPU usage stays below half the machine's cores. On this host (AMD Ryzen 5
5600, 6 cores / 12 threads) that operating threshold is **4 cores** — if you and the
other agents together would keep fewer than 4 cores busy with solver/bench processes,
the timing stays clean enough that concurrent runs do not meaningfully contaminate
each other's PAR-2.

- Each `tools/bench.sh` run is single-instance-at-a-time, so it occupies ~1 core; a
  couple of agents each running one bench is well under the 4-core threshold.
- Before starting, check current usage (`ps aux --sort=-%cpu | grep -E 'sat-solver|kissat|minisat'`)
  and only hold off if launching yours would push total solver/bench cores to 4 or more.
- For tight head-to-head A/B numbers you still want quiet cores, but for routine
  profiling-suite runs the half-cores rule is the bar — do not stall on it.

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

   The optimization target metric is **aggregate PAR-2 over the profile20 suite** (`benchmarks/profile20`):
   10 easy control instances (reused from `benchmarks/profiling`) + 10 hard headroom instances
   (solver-10 times out at 300 s, kissat finishes < 280 s). Report the all-20 aggregate plus the
   easy-10 / hard-10 split. A single instance is only a fast-iteration aid — never the keep/revert metric.

   For solver-11 feature ablations use the driver `tools/feature_ablation.py` (it encodes the matrix,
   the same-binary `SAT_*` toggles, and honors flag `requires` deps). **`--jobs N` sets both the
   number of parallel workers and the physical cores used** (`taskset 0..N-1`, siblings idle).
   Default is 4 jobs (cores 0–3). **For feature iteration on a quiet host, the standard run is
   5 threads × 5 seeds: `--jobs 5 --seeds 5`** (cores 0–4) — faster turnaround while coding; the
   final keep/turn-on/promote decision still REQUIRES the N=10 gate below (5 seeds is for iteration,
   not the authoritative verdict). **Cap `--mem-mb` so `jobs × mem` fits RAM** — at 5 jobs on this
   62 GiB host use ~`--mem-mb 11500` (the 14000 default × 5 overcommits → OOM-kill). **Before
   launching a parallel sweep, check for competing solver/bench processes
   (`ps aux --sort=-%cpu | grep -E 'sat-solver|kissat|feature_ablation|bench'`) and ASK the user
   before proceeding** — the driver also prints a `[preflight]` contention/memory warning at startup,
   but concurrent runs on the same cores still contaminate PAR-2 timing.
   - **Quick screen (exploration only, NOT a decision):** `--stage1` runs every config once at 300 s
     on all 20 to triage obviously-broken or wildly-regressing configs. Single-seed; cheap. Use it to
     pick which configs are worth measuring properly — never to keep/promote.
   - **Measuring a NEW feature (ad-hoc, no `CONFIG_MAP` edit):** put the feature behind a `SAT_*`
     flag (the repo convention), then A/B it with `--seedgate --env`:
     `python3 tools/feature_ablation.py --seedgate --env "SAT_NEWFEAT=on" --tag newfeat --seeds 5 --jobs 5`
     vs the baseline `--env "" --tag baseline ...`, and feed the two TSVs to the gate. `--env ''` is the
     solver default; `--env` works with everything below (5×5 for iteration, N=10 for the verdict).
   - **Keep/turn-on/promote decision (REQUIRED, the authoritative measurement):** the **multi-seed
     `--seedgate`** mode. For each config worth deciding on, run
     `python3 tools/feature_ablation.py --seedgate --configs <tag> --seeds 10 [--timeout 900]`
     (or `--env "SAT_…" --tag …` for an unregistered feature; N=10 seeds per instance via `SAT_SEED`,
     conflicts captured), then decide with the lexicographic
     gate `python3 tools/check_solver11_promotion.py --multiseed ...` (solved→conflicts→PAR-2). This
     is mandatory before keeping a feature or promoting a default, because single-seed verdicts on
     profile20 are unreliable (case9/sudoku/REGRandom are seed-fragile). The old `--stage2` + "3 %
     repeat rule" workflow is superseded by `--seedgate`; the repeat rule no longer applies.

   **Long ablation/seedgate jobs: run detached, report hourly, end with a comparative summary.**
   A full `--seedgate` sweep (20 instances × N seeds at 900 s, or any multi-config matrix) runs for
   hours — do not block the session on it. After the pre-launch contention/memory check and the
   user's go-ahead:
   - **Launch it in the background** so it survives the session — `run_in_background: true` on the
     Bash call for a single sweep, or the one-shot cron pattern under "Running Full/Medium SAT
     Competition 2025 Benchmarks" for very long / multi-day runs. Record the run dir
     (`log/seedgate-<tag>-<ts>/`) and the driver PID.
   - **Post an hourly status report** while it runs. `feature_ablation.py --seedgate` writes
     `results.tsv` only at the very end and deletes each `_work/<idx>` scratch dir per-cell, so
     mid-run progress comes from the live process list, not a partial file: `pgrep -af
     feature_ablation` for liveness, then map the in-flight `_work/<idx>` dirs to instances (idx =
     position in the instance-major × N-seed job list, so the lowest in-flight idx ≈ cells done).
     Report cells-done, current instance, and an ETA against the paired baseline's wall time, and
     schedule the next check ~1 h out (e.g. `ScheduleWakeup`). Stop the cadence when the run
     finishes or the user says so.
   - **Close with a comparative analysis + summary, not just "done".** When `DONE` / `results.tsv`
     land, parse the TSV and run the lexicographic gate against the paired baseline (plus the
     solver-10 floor for a promotion): `python3 tools/check_solver11_promotion.py --multiseed
     --candidate <cand.tsv> --previous <baseline.tsv> [--solver10 <s10.tsv>] --timeout <s>
     --memory-mb <MB>`. Summarize the solved→conflicts→PAR-2 verdict, per-instance solve-rate
     deltas (flag any seed-fragile row), and the keep/promote recommendation.
1. **Pick a target for iteration speed (not for the decision)**: If the user names a reference solver or competition gap, choose one concrete competition instance where the reference is faster than the repo solver but still short enough for repeated iteration, and use it to profile and prototype. Create a one-instance benchmark directory for it and keep using that target until the user redirects. The target instance accelerates the edit/measure loop; the decision to keep or revert is always made on aggregate profile20 PAR-2, so confirm every candidate on the full suite before keeping it.
2. **Baseline**: Run `bash tools/bench.sh -j 4 -d benchmarks/profile20 solver/NN-name` (or the `tools/feature_ablation.py` driver) and record **aggregate PAR-2 over the suite** (all-20, plus the easy-10 / hard-10 split) as the primary baseline metric. The profile20 default is 300 seconds per instance for Stage-1 screening; the hard-10 need a longer Stage-2 timeout to show headroom. For a one-instance target, also run `bash tools/bench.sh -t <seconds> -m 16384 -d <one-instance-dir> solver/NN-name` and record the target-instance runtime as a secondary iteration aid. If comparing to MiniSat-style simplification, also capture useful reference variants such as `minisat -no-pre`, `minisat -no-elim`, and full `minisat`.
3. **Profile first**: Use a profiler such as `perf stat` / `perf record` / `perf report` on the target instance before changing code. Use the profile to choose the next implementation slice. If the release binary is stripped and symbols are needed, rebuild for profiling with `CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=1 RUSTFLAGS="-C target-cpu=native" cargo build --release`.
4. **Check opportunity size before coding**: For simplification ideas, do a quick measurement pass over the target formula before implementing. Examples: count duplicate clauses, pure literals, candidate variables, binary subsumption hits, or self-subsuming-resolution opportunities. Do not implement an idea when the measured opportunity is negligible.
5. **Report diagnostic solver stats for every instance analysis**: Capture enough run data to identify whether the bottleneck is simplification, propagation, conflict learning, or search-path sensitivity. At minimum report pre/post-preprocessing variables, clauses, and literal counts; preprocessing time; eliminated variables, resolvents, subsumed clauses, strengthened literals, and root assignments when available; final result and runtime; conflicts, decisions, propagations, restarts, learned-clause count, reduce-DB calls, and any timeout/error status. When profiling, include propagation time or propagation throughput, conflict-analysis time if measured, simplification/proof I/O time if visible, and the profile/log paths used. When testing sensitivity, report the exact mode flags, seed/order/literal-order choices, preprocessing toggles, and per-instance deltas so the cause is not guessed from PAR-2 alone.
6. **Iterate** (at least 10 attempts unless the user asked for a narrower experiment): Make one change at a time. Use a fast single-instance / single-seed run *while coding* to sanity-check direction, but **the keep/revert decision MUST come from a multi-seed `--seedgate` sweep** judged on the lexicographic solved→conflicts→PAR-2 metric vs the current accepted baseline, beyond the baseline's own seed-spread. **Keep it if it wins the lexicographic metric beyond seed-noise** — regardless of which individual instances got slower or newly timed out. A change that rescues several instances at the cost of regressing others is a keep when the metric wins; a feature does not have to help every row. Recalculate the accepted baseline after every kept change. Revert changes that do not win the lexicographic metric beyond seed-noise, that lose it, or that fail correctness checks (wrong result, invalid model, missing/invalid proof, or a premature non-budget `UNKNOWN`). An honest new timeout is *not* by itself a revert reason — only the lexicographic metric matters. **Do not keep a change on a single-seed result** — that is the n=1 trap that produced wrong verdicts (see `log/feature-deepdive-2026-06-01/`).
7. **Stop losers early when safe**: For long runs, if an experiment clearly cannot improve aggregate PAR-2 (the already-finished rows have lost more PAR-2 than the remaining rows could plausibly recover), stop the run, mark the attempt rejected, and revert it instead of waiting for the full timeout.
8. **Tune algorithmic features carefully**: When testing a focused solver idea such as bounded variable elimination, run small parameter sweeps around the first good result. More simplification can damage CDCL search even when preprocessing is cheap, so validate caps and cost thresholds empirically rather than assuming monotonic improvement.
9. **Retest interactions**: A previously rejected micro-optimization can become worthwhile after a later kept change changes the profile. Retest it only when profiler evidence or timing data suggests the interaction could now improve aggregate PAR-2.
10. **Optimize the new feature if needed**: If a feature is conceptually promising but too slow, profile the modified solver and iterate on that feature's implementation. Keep it only once the optimized version improves aggregate PAR-2 over the suite beyond run-to-run noise.
11. **Correctness checks**: Add or update focused tests when practical before changing solver behavior. Run `cargo test` and the full smoke suite after every kept solver change, and rerun them after reverting failed experiments if the revert touched solver logic.
12. **Record**: Document every *successful* improvement and its aggregate PAR-2 impact (and target-instance runtime where relevant) in the solver's `README.md`, including machine environment metadata, benchmark log paths, profile paths, and the profiler evidence that motivated it. Also document important rejected attempts with their measured runtime or reason for skipping so future loops do not repeat them blindly.

### Scientific bottleneck workflow

See the `/analyzesat` skill for the full scientific bottleneck workflow — multi-config ablation,
work × speed decomposition, reference-source diff, trajectory-trace chaos analysis, and the
debugging-regressions / reference-gap-closing procedures that used to live here.

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

**Re-output command results as markdown in the reply.** Raw tool/command output is not
reliably visible or readable to the user. Whenever command output informs a status report,
comparison, analysis, or decision, re-present the relevant data directly in the reply as
clean GitHub-flavored markdown — markdown tables for tabular data (benchmark rows, per-instance
comparisons, sweep progress), inline numbers for single facts. Do not point at a tool result or
log path as the only presentation of the data.

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
- Read `benchmarks/profile20/README.md` for the current 20-instance optimization/promotion target suite (10 easy controls reused from `benchmarks/profiling` + 10 hard headroom instances; `selection.csv`/`selection.json` record provenance; regenerate with `tools/select_profile20.py`)
- Read `benchmarks/profiling/README.md` for the legacy 10-instance easy-only control suite (selected from SAT Competition 2025 medium-track, each <300 s on solver 10); the previous 6-instance set is preserved at `benchmarks/profiling/legacy/`
- Check `benchmarks/profiling/`, `benchmarks/crypto/`, `benchmarks/random-3sat/`, and the generator tools under `tools/` for the current benchmark inputs
- Use `tools/bench.sh`, `tools/bench_reference.sh`, and the latest `log/bench-*` directories to see the current benchmark workflow and outputs
- **Do NOT consult `solver/11-kissat-port/archive/` (archived pre-2026-05-29 search-feature efficacy verdicts) unless the user explicitly asks.** Search-feature efficacy is being re-evaluated fresh under bead `SAT-playground-gbc`; the archived FEATURES ledger and README validation tables are stale provenance contaminated by measurement artifacts (contention, cold-cache, warming variance) and must not be used as current efficacy evidence.

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

## Agent Mail

### Installation

Clone the repo, set up and install with uv in a python 3.14 venv (install uv if you don't have it already), and then run `scripts/automatically_detect_all_installed_coding_agents_and_install_mcp_agent_mail_in_all.sh`. This will automatically set things up for your various installed coding agent tools and start the MCP server on port 8765. If you want to run the MCP server again in the future, simply run `scripts/run_server_with_token.sh`:

```bash
# Install uv (if you don't have it already)
curl -LsSf https://astral.sh/uv/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"

# Clone the repo
git clone https://github.com/Dicklesworthstone/mcp_agent_mail
cd mcp_agent_mail

# Create a Python 3.14 virtual environment and install dependencies
# Note: If you have an older uv version, run `uv self update` first
uv python install 3.14
uv venv -p 3.14
source .venv/bin/activate
uv sync

# Detect installed coding agents, integrate, and start the MCP server on port 8765
scripts/automatically_detect_all_installed_coding_agents_and_install_mcp_agent_mail_in_all.sh

# Later, to run the MCP server again with the same token
scripts/run_server_with_token.sh

# Change port after installation
uv run python -m mcp_agent_mail.cli config set-port 9000
```

Now, simply launch Codex-CLI or Claude Code or other agent tools in other consoles; they should have the mail tool available.

### MCP Agent Mail: coordination for multi-agent workflows

What it is
- A mail-like layer that lets coding agents coordinate asynchronously via MCP tools and resources.
- Provides identities, inbox/outbox, searchable threads, and advisory file reservations, with human-auditable artifacts in Git.

Why it's useful here
- The hot file for any active solver iteration is `solver/NN-name/src/main.rs` — multiple agents will want to edit it concurrently. Agent Mail lets them coordinate via Beads claims and thread messages instead of locking each other out with exclusive file reservations.
- Keeps cross-agent chatter out of the token budget by storing messages in a per-project archive.
- Fast reads via `resource://inbox/...` and `resource://thread/...`.

#### Coordination workflow (single shared checkout, develop on `main` directly)

This repo runs at most a few agents in parallel. **All agents share the one checkout at
`/home/bojji/code/SAT-playground` and develop on the `main` branch directly** — no git
worktrees, no per-agent feature branches for routine bead work. (This overrides the usual
"branch first before committing to the default branch" default — here, committing to `main`
is the expected flow.) Because there is no working-tree isolation, the coordination rules
below are what keep two agents from clobbering each other: discover who is touching what,
**ask the user before proceeding into a likely conflict**, pre-announce commits, and
pull-rebase-revalidate before every push. Agent Mail is the communication channel throughout.

1) Register and discover what others are doing
   - `ensure_project(human_key="/home/bojji/code/SAT-playground")`, then `register_agent` with a unique `agent_name`. Set `AGENT_NAME` in your shell so the pre-commit guard knows who you are.
   - If a reused identity such as `s11-06` reports `requires registration_token` after a successful `register_agent`, the MCP connector is not preserving Agent Mail's in-memory session binding across tool calls. Work around it by reading the existing local token from `/home/bojji/code/mcp_agent_mail/storage.sqlite3` and passing it explicitly as `registration_token` / `sender_token` on Agent Mail calls. Do **not** paste the token into chat, commits, Beads notes, or AGENTS.md. Example:
     ```bash
     AGENT_MAIL_TOKEN=$(python3 - <<'PY'
     import sqlite3
     con = sqlite3.connect('/home/bojji/code/mcp_agent_mail/storage.sqlite3')
     row = con.execute('''
     select a.registration_token
     from agents a join projects p on p.id = a.project_id
     where p.human_key = ? and lower(a.name) = lower(?)
     ''', ('/home/bojji/code/SAT-playground', 's11-06')).fetchone()
     print(row[0] if row and row[0] else '')
     PY
     )
     ```
     Then call tools with explicit auth, e.g. `fetch_inbox(..., agent_name="s11-06", registration_token=AGENT_MAIL_TOKEN)` or `send_message(..., sender_name="s11-06", sender_token=AGENT_MAIL_TOKEN)`.
   - Before picking up work, list active claims (`bd ready`, `bd list --status in_progress`) and read the shared coordination thread (default `thread_id="coord"`) via `resource://thread/coord?...` to see what other agents have announced. Because everyone shares one checkout, **also inspect the working tree itself** — `git status --short` for in-flight edits another agent left uncommitted, and `ps aux --sort=-%cpu | grep -E 'sat-solver|kissat|bench|feature_ablation'` for live solver/bench processes.
   - Prefer a bead whose scope does **not** overlap the files/regions other agents have already announced or are actively editing. Prefer beads that touch different functions, modules, or solver iterations than the active claims.

2) Develop in the main directory on `main` directly
   - Work in `/home/bojji/code/SAT-playground` on the `main` branch. Do **not** create a worktree or a feature branch for routine bead work. All edits, builds, smoke tests, and benchmarks happen in this one shared checkout.
   - Keep `main` current before you start: `git pull --rebase origin main`.
   - **Conflict gate — ask the user before proceeding:** if another agent is already working on a file you would need to touch — a coord claim naming it, an `in_progress` bead scoped to it, uncommitted edits to it in `git status --short`, or a live process for it — **stop and ask the user for permission before proceeding** instead of editing it anyway. Surface what you saw (which agent, which file, the evidence) so they can decide, and only proceed once they say so. Non-overlapping work needs no permission — just announce it (step 3) and go. (Use Agent Mail to coordinate with the other agent, but the permission gate is the **user's** call.)
   - Commit in small, coherent units so a `git pull --rebase` stays cheap and conflicts stay local.

3) Don't reserve the hot file — claim a bead and announce intent instead
   - Do **not** call `file_reservation_paths(..., exclusive=true)` on `solver/**/src/main.rs`. Exclusive reservations on the hot file defeat the point of parallel agents.
   - Instead: `bd update <id> --claim`, then `send_message(thread_id="coord", subject="claim <id>", body="bead <id>: <one-line scope> — touching <functions/regions> in solver/NN-name/src/main.rs")`.
   - File reservations are still appropriate for **less-contended paths** (e.g. `docs/**`, `tools/**`, `tests/cnf/**`, `benchmarks/profiling/**`) when an edit will span many files there. Reserve those, not `src/main.rs`.

4) Pre-announce every commit with a short objection window
   - Before `git commit`, send `send_message(thread_id="coord", subject="commit <bead-id>", body="files: <list> · regions: <fns/lines> · summary: <one line> · validation: <what you ran> · objections within 2 min")`.
   - Wait ~2 minutes (`fetch_inbox` + `acknowledge_message` for any replies). If no objections, commit locally (push happens in step 5 after a pull-rebase).
   - If another agent replies "I'm about to push overlapping hunks for bead Y", let them push first; you pull-rebase and re-validate (step 5).
   - The 2-minute window is tunable — extend it for risky changes (cross-cutting refactors, new solver iteration scaffolding), keep it tight for narrow bead-scoped edits.

5) Pull-rebase and re-run the bead's validation before every push
   - Always `git pull --rebase origin main` immediately before pushing — with everyone on `main`, a stale local `main` means your push is rejected or you rebase onto surprises. Resolve any conflicts (don't `--skip` or `--abort` away another agent's work).
   - **Re-run whatever validation the bead requires**, in full, on the rebased state:
     - Correctness/fix beads → `bash tools/smoke_test.sh solver/NN-name` (+ `cargo test` if the bead touches tests).
     - Perf/optimization beads → smoke test **plus** `bash tools/bench.sh -j 4 -d benchmarks/profile20 solver/NN-name` so the measured delta is against the new baseline, not the pre-rebase one.
     - Promotion/default-change beads → the full gate described in CLAUDE.md's solver-11 promotion section, with the candidate re-measured on top of the new `main`.
   - If the rebase changes the baseline materially, **re-take the baseline** before reporting the experiment's delta — numbers from a stale baseline are meaningless.
   - If the rebase pulled in new commits, re-announce (step 4) with the updated base SHA in the body, then `git push origin main`. (Commit and push only when the user has asked for it — same as everywhere else.)

6) Closing out
   - `bd close <id>` with a summary, then send a final `send_message(thread_id="coord", subject="closed <id>", body="pushed at <sha>")`. (No worktree to tear down — you were on `main` the whole time. Leave the shared checkout clean: don't leave half-finished uncommitted edits behind for the next agent.)

#### Granular vs macro tools
- Granular (use these in this workflow): `register_agent`, `send_message`, `fetch_inbox`, `acknowledge_message`, `fetch_topic`, `file_reservation_paths` (only for non-hot paths).
- Macros (handy when you don't need fine control): `macro_start_session`, `macro_prepare_thread`, `macro_contact_handshake`. Skip `macro_file_reservation_cycle` for the hot file — it implies exclusive reservation semantics we explicitly don't want there.

#### Common pitfalls
- **Treating `src/main.rs` as exclusive**: don't reserve it. Coordinate via Beads claims + the `coord` thread.
- **Editing a file another agent is actively working on**: the conflict gate (step 2) says stop and **ask the user first** — don't edit a contended file just because your local build is green. Everyone shares one working tree, so two agents editing the same file at once corrupts both their states.
- **Skipping the pre-announce window**: silent commits race. A 2-minute window costs almost nothing and catches nearly all collisions.
- **Pushing after a rebase without re-running validation**: the bead's evidence belongs to the rebased commit, not the pre-rebase one. Re-run the full suite the bead requires, including baseline re-take for perf work.
- **Pushing without a `git pull --rebase` first**: with everyone committing to `main`, a stale local `main` gets your push rejected or rebased onto surprises. Pull-rebase + re-validate right before every push (step 5).
- **Leaving uncommitted edits in the shared checkout**: another agent will trip over them in `git status` and may have to ask you about them. Finish, commit, or revert your changes before stepping away.
- **"from_agent not registered"**: always `register_agent` in the correct `project_key` first.
- **"FILE_RESERVATION_CONFLICT"** on non-hot paths: adjust patterns, wait for expiry, or use a non-exclusive reservation.
- **"requires registration_token" after registering**: tokenless session binding may be broken in this MCP connector. Use the explicit-token workaround in step 1 and pass `registration_token` / `sender_token` on every Agent Mail call.
- **Auth errors**: if JWT+JWKS is enabled, include a bearer token with a `kid` that matches server JWKS; static bearer is used only when JWT is disabled.
