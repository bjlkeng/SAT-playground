# CLAUDE.md - SAT-playground Development Guide

This file is the high-signal project contract for coding agents. It carries the
measured facts and decision rules that are not derivable from the repo — above
all the evaluation metric and promotion gate, which live here in full because
almost every change is judged against them. Everything else should be a pointer.

## Project Overview

This repo builds Boolean SAT solvers iteratively in Rust, one directory per
iteration, all conforming to the SAT Competition solver interface. Each
iteration is self-contained and builds on the previous one.

## Build And Run

```bash
# Build any iteration
cd solver/NN-name && bash build.sh

# Run on a CNF instance
bash run.sh path/to/instance.cnf /tmp/proof_dir

# Run benchmarks
bash tools/bench.sh solver/NN-name
```

## Canonical References

- Site workflow: `docs/SITE_WORKFLOW.md`
- Pinned reference solvers and baseline provenance:
  `benchmarks/REFERENCE_SOLVERS.md`
- Current decision/gate suite: `benchmarks/sat-comp-2025-medium/` (100
  instances, single default seed; run via
  `feature_ablation.py --suite sat-comp-2025-medium --seeds 1`)
- Prior profile20 target suite (provenance/background): `benchmarks/profile20/README.md`
- Legacy fast control suite: `benchmarks/profiling/README.md`
- Solver 11 current state and feature surface:
  `solver/11-kissat-search/README.md`, `solver/11-kissat-search/FEATURES.md`,
  and `solver/11-kissat-search/SOLVER11_STATE.md`


## Solver Interface Contract

Every iteration must provide these files at its top level:

- `build.sh`: no arguments; builds the solver binary.
- `run.sh <cnf_path> <output_dir>`: runs the solver, prints to stdout, and
  writes `<output_dir>/proof.out` when reporting UNSAT.

Required stdout:

```text
s SATISFIABLE
v 1 -2 3 0
```

or:

```text
s UNSATISFIABLE
```

or:

```text
s UNKNOWN
```

Rules:

- Print exactly one `s` line per run.
- Print `v` lines only for SAT; literals are space-separated and terminated by
  `0`, with at most 4096 characters per line.
- `c` comment lines are allowed anywhere.
- Partial SAT assignments are fine if every original clause is satisfied.
- UNSAT requires a DRAT proof at `<output_dir>/proof.out`.

## Code Conventions

- Language: Rust.
- Binary name: `sat-solver`.
- No external SAT/SMT solver dependencies. Standard utility crates are fine.
- Each iteration is self-contained; copy-and-modify rather than introducing
  workspace dependencies between iterations.
- Test with small hand-crafted CNFs before competition-sized benchmarks.
- Match the existing iterations' release profile and `build.sh` (fat LTO, one
  codegen unit, `target-cpu=native`); copy them from the previous iteration
  rather than reinventing.
- The release profile sets `strip = true`, so profiling needs an explicit
  override to get symbols:

  ```bash
  CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=1 \
  RUSTFLAGS="-C target-cpu=native" cargo build --release
  ```

## Development Rules

- The primary solver decision metric is lexicographic over the 100-instance
  `benchmarks/sat-comp-2025-medium` suite at the single default seed: solved
  count, then total conflicts on tied solved cells, then PAR-2 only as a
  supplemental tie-break.
- Any keep, turn-on, or promotion decision must use the full medium single-seed
  A/B and the gate — see "Promotion Gate". Screen the idea on a cheaper tier
  first ("Candidate Triage Tiers"); subset and single-instance runs are never
  promotion evidence on their own, and always say which tier a number came from.
- Honest timeouts and budget-consuming `UNKNOWN` results are priced into the
  metric. They are not correctness bugs by themselves.
- Correctness errors are never acceptable: wrong SAT/UNSAT status, invalid SAT
  model, missing/invalid UNSAT proof, or premature non-budget `UNKNOWN` must be
  debugged before tuning or promotion continues.
- A change may regress individual instances if it wins the lexicographic
  aggregate metric beyond seed noise. **Do not revert on any loss — judge the
  trade explicitly** ("Judging Trades"), and keep iterating until the metric
  actually moves.
- Do not promote hard-coded guards, one-family classifiers, or lucky input-order
  wins without mechanism-level evidence and shuffle-sensitivity validation.
- Treat stale "rejected for default" notes as hypotheses to re-measure, not
  settled verdicts — the context may have changed since they were written.
- Record improvements in the solver README with benchmark log paths, machine
  metadata, and measured impact, and record important rejected attempts too so
  future loops do not repeat them.
- Use red-green TDD for solver behavior changes when practical.
- Run `bash tools/smoke_test.sh solver/NN-name` after every solver change, and
  only commit changes that pass it. Never modify `tools/smoke_test.sh` unless
  the user explicitly asks.
- Always commit and push when the user asks; do not skip the push.
- For background ACP/subagent tasks in Discord, suppress automatic
  `Background task done/failed` notices by setting the task notify policy to
  `silent`. When manually reporting completion/failure in Discord, mention
  bjlkeng as `<@817490773179760662>`.

## Promotion Gate

Default/fast promotion uses the `sat-comp-2025-medium` lexicographic
solved-to-conflicts-to-PAR-2 metric over all 100 instances at the single default
seed. Produce one TSV per config with:

```bash
python3 tools/feature_ablation.py --seedgate --configs <tag> --suite sat-comp-2025-medium --seeds 1
```

Or produce the candidate and baseline TSVs in one fair, simultaneous-start
before/after A/B run:

```bash
python3 tools/feature_ablation.py --arm 'candidate:SAT_X=on' --arm 'baseline:' --suite sat-comp-2025-medium --seeds 1
```

Both arms start simultaneously on shared pinned cores, so there is no host drift
between them. Standard parallelism is `--jobs 32 --mem-mb 16000` at a 1800 s
wall: 32 and not all 36 physical cores, to leave system headroom. The run emits
one gate TSV per arm and prints the solved→conflicts→PAR-2 verdict inline.

Then confirm with:

```bash
python3 tools/check_promotion_gate.py --multiseed \
  --candidate <candidate.tsv> \
  --baseline <pre-change-baseline.tsv> \
  --timeout <seconds> \
  --memory-mb <MB>
```

The gate is a before/after A/B with no external floor. It makes only two kinds of
comparison: candidate correctness (invalid model/proof, ERROR/PARSE_ERROR, and
SAT/UNSAT contradictions against the baseline) and the candidate versus the
pre-change baseline on the lexicographic metric.

Correctness failures are absolute: they fail the gate, always, no trade.

For the performance comparison the gate output is an input to a judgement, not
the verdict itself. See "Judging Trades".

## Judging Trades

A raw lexicographic regression does not automatically mean revert. Classify
every changed cell before deciding, because a solved-count delta mixes two very
different things:

- **Wall-coin cell** — either test qualifies it:
  1. *Thin margin*: `timeout - baseline_time <= ~120 s` (at the 1800 s gate, a
     baseline solve at >= ~1680 s).
  2. *Documented flipper*: the cell has been observed to flip solved/unsolved
     across deals **at an identical conflict count**. This test is the stronger
     one — conflicts are exactly deterministic across load while wall is not, so
     identical conflicts with a different outcome is proof the cell is pure
     wall luck, whatever its margin. The current flipper list lives in the
     newest `plan/next-steps-*.md` / `plan/next-plan.md` "Standing traps".
- **Capability cell** — the baseline solved it with real margin and a stable
  trajectory, or the candidate solves something previously unsolved. Signal.

Test 2 matters because margins alone mislead: a known flipper can post a
300-700 s margin in one deal and time out in the next on the same trajectory.
Check the conflict counts before calling a loss a capability loss.

Calibration: across three gates on one host on 2026-07-24 the *baseline* scored
67, 69, and 71 on the same suite and commit. **±2 solved cells is deal noise.**
Weigh a raw solved-count delta of 1-2 accordingly, and lean on tier-2 conflicts,
wall, and mechanism evidence to break those ties.

The flexible rule:

> A candidate may lose up to **N = 2** wall-coin cells (3 with written
> justification) and still be promotable, **provided** it gains
> mechanism-validated capability elsewhere. Judge and record the trade
> explicitly; do not revert on any loss, and do not promote on coin wins alone.

"Mechanism-validated" means the gain is explained and reproducible, not a lucky
draw: a first-ever solve, a fat margin (well clear of the timeout), a
digit-exact identity check showing untouched cells, or a measured mechanism
(elimination depth, propagation rate, proof size) that accounts for the win.

Trades that FAIL this rule:

- Losing a cell whose baseline margin was large (a real capability loss).
- Winning only wall coins while tier-2 conflicts and wall regress — that is a
  reroll lottery, not an improvement. Prefer the arm that wins on mechanism.

Write the trade into the promotion note: cells gained, cells lost with their
baseline margins, the mechanism evidence, and the tier-2/PAR-2 movement. If the
trade is genuinely ambiguous, say so and ask.

## Candidate Triage Tiers

Do not spend a 100-instance gate on an unscreened idea. Escalate:

1. **Probe (minutes).** 1-15 cells chosen to exercise the mechanism, short
   walls, mechanism counters via `SAT_STATS_JSON=on` plus `SAT_LIMIT_CONFLICTS`
   or `SAT_LIMIT_WALL_SEC`. Answers "does it do anything at all".
2. **Triage subset (tens of minutes).** Either the ~20-cell
   `benchmarks/discriminating` set, a ~15-cell hand-picked probe set, or the
   ~30-cell timeout subset drawn from the newest medium results (the cells that
   currently time out — that is where capability gains appear). Short walls
   (300-900 s). Answers "which variant is best, and is it worth a gate".
3. **Promotion gate (hours).** Full 100-instance medium single-seed A/B at
   1800 s / 16 GB / 32 pinned cores. The ONLY promotion evidence.

Build the timeout subset from the latest gate or seedgate TSV, e.g. select rows
whose `result` is not SAT/UNSAT and pass those stems via a `--suite` directory of
symlinks. Use judgement on subset choice: match the subset to the mechanism
(timeout cells for capability work, the 1600-1800 s margin band for wall work,
miters/circuits for elimination work).

Never quote a subset result as a promotion decision, and always say which tier a
number came from.

## Multi-Arm Sweeps

Run **up to 4 candidate variants per sweep and promote the best arm.** One
invocation, simultaneous start, shared pinned cores — that is a fair paired
comparison and it costs little more than a single A/B:

```bash
python3 tools/feature_ablation.py \
  --arm 'v1:SAT_X=on' \
  --arm 'v2:SAT_X=on,SAT_X_TUNE=2048' \
  --arm 'v3:SAT_X=aggressive' \
  --arm 'base:' \
  --suite sat-comp-2025-medium --seeds 1
```

Guidance:

- Always include a `base:` arm; without it there is no baseline to judge against.
- 4 arms total (3 candidates + base) is the cap — more arms means each arm's
  cells contend and marginal cells get noisier.
- Vary ONE axis per sweep (a tuning parameter, a policy choice) so the winner is
  interpretable. Do not mix unrelated features into different arms.
- Prefer this to sequential A/Bs when tuning a knob: it removes host drift
  between arms entirely.
- Pick the winning arm by the same "Judging Trades" rules, then re-gate the
  winner alone if the sweep was run on a triage subset.
- Beware antagonistic combinations: two individually-good features can lose
  together. If you bundle, also run each alone as its own arm.

## Testing

```bash
cd solver/NN-name && cargo test
bash tools/smoke_test.sh solver/NN-name
```

Smoke tests live under `tests/cnf/` and cover small SAT and UNSAT instances.
The smoke script builds the solver, checks the `s` line, and validates SAT
assignments — it catches the usual output-format mistakes, so run it before
hand-auditing the interface.

## Benchmarking Rules

- For routine single-worker profiling runs, concurrent benchmarking is allowed
  while combined solver/bench CPU use stays below four cores on this host.
- Before starting benchmarks, check live solver usage and ask before launching a
  parallel sweep if competing solver/bench processes are live. Cap memory so
  `jobs * mem` fits RAM.
- `feature_ablation.py --seedgate` writes `results.tsv` only at the end;
  mid-run progress has to come from live processes and `_work/<idx>` scratch
  dirs. Long sweeps run for hours — launch them detached so they survive session
  loss, and record the run directory and PID.
- **Marginal-cell timing is invalid while another 32-way sweep runs.** A gate
  saturates memory bandwidth, so cells near the timeout report TIMEOUT in every
  arm even on nominally free cores. Under contention a SOLVE is trustworthy but
  a TIMEOUT is not — schedule margin measurements on a quiet host.
- When killing a run, kill the wrappers *and* the solver binaries — children
  outlive the parent:
  `pkill -f 'bench_reference'; pkill -f 'kissat.*\.cnf'`, then verify with `ps`.
- Reference-solver runs use `tools/run_bench_reference.sh` (`-t` timeout, `-m`
  memory MB, `-d` benchmark dir; solvers default to
  `kissat-latest kissat-sc2024 minisat`). It writes the
  `log/bench_reference_RUNNING` / `_DONE` sentinels and
  `log/bench-<solver>-<timestamp>/results.csv`. See
  `benchmarks/REFERENCE_SOLVERS.md` for pinned versions and baseline provenance.

SAT Competition 2025 main track, for calibration: 5000 s timeout, 30 GB RAM,
8-core competition host; the winner was kissat-sc2024 at PAR-2 2788 and 306/400
solved. Competition scoring is PAR-2 — runtime for solved instances plus twice
the timeout for each unsolved one, lower better. Instances came from
`https://benchmark-database.de/?track=main_2025&context=cnf`.

## Site And Docs

The static benchmark site is under `docs/` and is deployed at
`https://bjlkeng.io/SAT-playground/`. Use `docs/SITE_WORKFLOW.md` for site
content conventions, generated data, README chart updates, and validation.

For interactive or visual docs, use the `debug-web-visualizations` skill and
verify in a browser rather than relying on static inspection.

## Multi-Agent Work

Agents share the main checkout on `main`. Before editing, check for uncommitted
edits and live solver/bench processes. If another active agent is working on a
file or region you need, stop and ask the user before proceeding. Optional Agent
Mail coordination details are in `plan/agent-coordination.md`.

## Status Reporting

When reporting command-derived status, show the command and the relevant output
or summary in the reply; do not rely on hidden tool output.

For runtime status, check live `sat-solver`/`minisat`/`kissat` processes and the
benchmark sentinels and logs under `log/` — especially
`log/bench_reference_RUNNING`, the newest `log/bench-*`, and any relevant
`results.csv`. Report SAT/UNSAT/timeout/error counts when a benchmark is running.

## Finding Current State

Do not treat this file as the source of truth for exact current solver lineup,
benchmark state, or feature efficacy. Instead:

- List solver iterations with `ls solver`.
- Read the relevant solver `README.md`, `Cargo.toml`, and source.
- Check `git status --short` and recent `git log`.
- Read `benchmarks/profile20/README.md` and `benchmarks/profiling/README.md`
  for current benchmark-suite provenance.
- Check `benchmarks/reference-solvers/`, `benchmarks/REFERENCE_SOLVERS.md`,
  `tools/checkers/`, and `tools/setup_checkers.sh` for references and proof
  checkers.
- Use `tools/bench.sh`, `tools/bench_reference.sh`, and latest `log/bench-*`
  directories for real benchmark outputs.
- Do not consult `solver/11-kissat-search/archive/` for current search-feature
  efficacy unless the user explicitly asks. It is archived pre-2026-05-29
  provenance and is known to include stale, noise-contaminated verdicts.

## Session Completion

When ending a work session, work is not complete until `git push` succeeds. Run
the quality gates for whatever changed (smoke test, `cargo test`), commit, then:

```bash
git pull --rebase
git push
git status  # must show "up to date with origin"
```

Never stop before pushing, and never say "ready to push when you are" — push it.
If the push fails, resolve and retry until it succeeds. Then hand off with
context for the next session, recording follow-up work in `plan/next-plan.md`.
