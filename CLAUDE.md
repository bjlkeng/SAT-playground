# CLAUDE.md - SAT-playground Development Guide

This file is the high-signal project contract for coding agents. Keep long
procedures in skills or domain reference files, and keep this file focused on
rules that should be in context for most work.

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
- Benchmark operations, cron runs, and reference-solver runs:
  `benchmarks/BENCHMARK_WORKFLOWS.md`
- Solver optimization workflow: `plan/solver-optimization-workflow.md`
- Current benchmark target suite: `benchmarks/profile20/README.md`
- Legacy fast control suite: `benchmarks/profiling/README.md`
- Solver 11 current state and feature surface:
  `solver/11-kissat-search/README.md`, `solver/11-kissat-search/FEATURES.md`,
  and `solver/11-kissat-search/SOLVER11_STATE.md`

Use existing skills instead of duplicating their workflows here:

- `beads`: `.agents/skills/beads/SKILL.md`
- `/nextbeads`: `.codex/skills/nextbeads/SKILL.md`
- `/cleanbeads`: `.codex/skills/cleanbeads/SKILL.md`
- `/analyzesat`: `.codex/skills/analyzesat/SKILL.md`
- Web visualization/debugging: `.codex/skills/debug-web-visualizations/SKILL.md`

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

Standard release profile for solver crates:

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
overflow-checks = false
```

Standard `build.sh`:

```bash
[[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

## Development Rules

- The primary solver decision metric is lexicographic over
  `benchmarks/profile20`: solved count, then total conflicts on tied solved
  cells, then PAR-2 only as a supplemental tie-break. Read
  `benchmarks/profile20/README.md` before interpreting profile20 results.
- Any keep, turn-on, or promotion decision must use a multi-seed sweep, normally
  N=10, and the multiseed gate:
  `python3 tools/check_promotion_gate.py --multiseed ...`. When iterating, run
  the candidate and baseline together as one A/B:
  `python3 tools/feature_ablation.py --arm 'cand:SAT_X=on' --arm 'base:'` — it
  starts both arms simultaneously on shared pinned cores (defaults: 32 cores,
  16 GB, 30 min), emits the per-arm gate TSVs, and prints the
  solved→conflicts→PAR-2 verdict inline.
- Single-seed or one-instance runs are allowed for debugging and iteration only.
  Do not keep or promote a solver feature on that evidence.
- Honest timeouts and budget-consuming `UNKNOWN` results are priced into the
  metric. They are not correctness bugs by themselves.
- Correctness errors are never acceptable: wrong SAT/UNSAT status, invalid SAT
  model, missing/invalid UNSAT proof, or premature non-budget `UNKNOWN` must be
  debugged before tuning or promotion continues.
- A change may regress individual instances if it wins the lexicographic
  aggregate metric beyond seed noise.
- Do not promote hard-coded guards, one-family classifiers, or lucky input-order
  wins without mechanism-level evidence and shuffle-sensitivity validation.
- Use red-green TDD for solver behavior changes when practical.
- Run `bash tools/smoke_test.sh solver/NN-name` after every solver change.
- Only commit solver changes that pass the smoke test.
- Never modify `tools/smoke_test.sh` unless the user explicitly asks.
- Always commit and push when the user asks; do not skip the push.
- For background ACP/subagent tasks in Discord, suppress automatic
  `Background task done/failed` notices by setting the task notify policy to
  `silent`. When manually reporting completion/failure in Discord, mention
  bjlkeng as `<@817490773179760662>`.

## Solver 11 Promotion Gate

Solver 11 default/fast promotion uses the profile20 lexicographic
solved-to-conflicts-to-PAR-2 metric across N=10 seeds. Produce one TSV per
config with:

```bash
python3 tools/feature_ablation.py --seedgate --configs <tag> --seeds 10
```

Or produce the candidate and previous-default TSVs in one fair, simultaneous-start
A/B run (add `--arm solver10` to include the floor arm):

```bash
python3 tools/feature_ablation.py --arm 'candidate:SAT_X=on' --arm 'previous:'
```

Then run:

```bash
python3 tools/check_promotion_gate.py --multiseed \
  --solver10 <solver10.tsv> \
  --previous <prior-default.tsv> \
  --candidate <candidate.tsv> \
  --timeout <seconds> \
  --memory-mb <MB>
```

Solver 10 is the regression floor: do not ship a default that loses to solver 10
lexicographically. The floor is aggregate, not per-instance; only a SAT/UNSAT
correctness contradiction fails regardless of the metric.

## Iteration Workflow

When creating a new iteration:

1. Copy the previous iteration directory.
2. Update `Cargo.toml` package name.
3. Add or update tests first when practical.
4. Implement the new technique.
5. Add unit tests for the new feature.
6. Run `bash tools/smoke_test.sh solver/MM-name`.
7. Record benchmark results and notes in the iteration README.
8. Ensure `build.sh` and `run.sh` still work.

## Testing

```bash
cd solver/NN-name && cargo test
bash tools/smoke_test.sh solver/NN-name
```

Smoke tests live under `tests/cnf/` and cover small SAT and UNSAT instances.
The smoke script builds the solver, checks the `s` line, and validates SAT
assignments.

Common interface pitfalls:

- Missing trailing `0` on `v` lines.
- Printing `v` lines for UNSAT.
- Mishandling empty clauses or top-level unit clauses.
- Off-by-one variable indexing; DIMACS variables are 1-based.
- Exceeding 4096 characters on a single `v` line.

## Benchmarking Rules

- For routine single-worker profiling runs, concurrent benchmarking is allowed
  while combined solver/bench CPU use stays below four cores on this host.
- Before starting benchmarks, check live solver usage:
  `ps aux --sort=-%cpu | grep -E 'sat-solver|kissat|minisat'`.
- For feature-ablation sweeps, follow
  `plan/solver-optimization-workflow.md`; it has stricter preflight and
  long-run reporting rules.
- For full or medium SAT Competition benchmark campaigns, follow
  `benchmarks/BENCHMARK_WORKFLOWS.md` and use the one-shot cron pattern so runs
  survive agent session loss.

## Site And Docs

The static benchmark site is under `docs/` and is deployed at
`https://bjlkeng.io/SAT-playground/`. Use `docs/SITE_WORKFLOW.md` for site
content conventions, generated data, README chart updates, and validation.

For interactive or visual docs, use the `debug-web-visualizations` skill and
verify in a browser rather than relying on static inspection.

## Beads And Multi-Agent Work

Use Beads for durable task tracking in this repo. The workflow lives in the
`beads`, `/nextbeads`, and `/cleanbeads` skills; do not duplicate their command
recipes here. At minimum:

```bash
bd prime
bd ready
bd show <id>
bd update <id> --claim
bd close <id> --reason="Completed"
bd update <id> --assignee ''
```

Run `/nextbeads` for phase-scoped bead work. It owns the full claim, implement,
validate, close, release, commit, and push workflow.

Agents share the main checkout on `main`. Before editing, check for active beads,
uncommitted edits, and live solver/bench processes. If another active agent is
working on a file or region you need, stop and ask the user before proceeding.
Optional Agent Mail coordination details are in `plan/agent-coordination.md`.

## Status Reporting

When reporting command-derived status, show the command and the relevant output
or summary in the reply; do not rely on hidden tool output.

When the user asks for runtime status, check:

```bash
ps aux --sort=-%cpu | head -20
pgrep -a sat-solver; pgrep -a minisat; pgrep -a kissat
```

Also inspect active benchmark sentinels/logs under `log/`, especially
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
