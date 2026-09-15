# CLAUDE.md — SAT-playground

What a coding agent needs to know before touching this repo: what we are
working on, how we measure it, and the rules to follow. It holds only what
you cannot work out by reading the code. Everything else is a pointer.

## How to explain

**Always use simple, plain language.** Short sentences, ordinary words. When
a term cannot be avoided (PAR-2, tick, inprocessing, epoch), define it once
in a few words and move on. Say what happened and what it means, not how
clever the method was. This applies everywhere: replies, commit messages,
plan documents, READMEs, and code comments.

## What this repo is

Boolean SAT solvers built iteratively in Rust, one directory per iteration,
all conforming to the SAT Competition solver interface. Iterations 01-12 are
finished history — read them only when a task actually points there. Current
work is **solver 13** and the RL scheduler on top of it.

## Current work

- **`solver/13-kissat-rs`** — faithful Rust reimplementation of kissat 4.0.4
  (reference: `benchmarks/reference-solvers/kissat-latest/`). All engines
  ported. Third paired 400-instance run (2026-09-06):
  **313 v 312 solved, PAR-2 0.9932x, both-solved wall geomean 0.9875x, zero
  contradictions** — i.e. at or slightly ahead of the C. Details, log paths
  and residual families: `solver/13-kissat-rs/README.md`.
  `solver/13-kissat-rs/CONVENTIONS.md` is **binding** for any port edit.
  Exact counter parity with the C was the port's acceptance criterion and is
  now a *starting point*, not a standing invariant — the RL scheduler changes
  the trajectory on purpose. `solver/13-kissat-rs/tools/parity.py` is still
  the tool to reach for when the stock path regresses unintentionally.
- **`plan/rl-scheduler-solver13-plan.md`** — the active project. Replace
  kissat's *timing* layer (when to probe / eliminate / reduce / rephase /
  reorder / switch mode, how hard to restart, later per-pass effort) with a
  small CPU MLP consulted every X search ticks. Every mechanism stays
  byte-identical; train offline from logged and fork-branched trajectories
  first. §10 is the work order, §8 the evaluation discipline, §11-15 the
  decision log and review passes. Read it before starting any
  `src/policy.rs` work — that module does not exist yet; plan step A
  creates it.

## Build, run, test

```bash
cd solver/13-kissat-rs && bash build.sh          # builds target/release/sat-solver
bash run.sh path/to/instance.cnf /tmp/outdir     # competition interface
cargo test
bash tools/smoke_test.sh solver/13-kissat-rs     # run after EVERY solver change
```

Solver 13's `build.sh` is deliberately **generic x86-64** — measured
2026-09-05, `-C target-cpu=native` (AVX-512 codegen) is 8-11% *slower* on
search-bound cells, and the reference kissat is itself generic `-O3`. Do not
re-add `target-cpu=native` here. The release profile sets `strip = true`, so
profiling needs `CARGO_PROFILE_RELEASE_STRIP=false
CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release`.

`tools/current_solver.sh` resolves "the current solver" to the
highest-numbered `solver/NN-*` with `build.sh` + `run.sh`, so the harness
targets solver 13 by default; override with `SAT_TARGET_SOLVER`.

## Solver interface contract

Every iteration provides `build.sh` (no args) and `run.sh <cnf> <out_dir>`.
`run.sh` prints exactly one `s` line — `s SATISFIABLE` (plus `v` lines of
space-separated literals terminated by `0`, ≤4096 chars per line),
`s UNSATISFIABLE` (plus a DRAT proof at `<out_dir>/proof.out`), or
`s UNKNOWN`. `c` comments allowed anywhere; partial SAT assignments are fine
if every original clause is satisfied.

## Evaluation

The decision metric, in order:

1. **Solved instances.**
2. **Tick PAR-2** — the same PAR-2 sum, but counted in work units instead
   of seconds: a solved cell costs the work it used, an unsolved cell costs
   twice the work budget it was given. A work unit is the plan's work clock
   **W = `statistics.ticks` + k_res × `eliminate_resolutions`** (plan §3.4).
   Use W, **not `search_ticks`**: search ticks leave out probing and
   elimination, which are exactly the passes the scheduler is allowed to
   move, so a policy could look cheap just by pushing work out of search.
   W is deterministic, so this number comes out the same on a loaded host
   and an idle one.
3. **Wall PAR-2** — the competition metric, reported alongside.

W is not in the harness output yet. Plan step A′ adds per-cell W to
`feature_ablation.py`'s results TSV and to `run_kissat_full.sh`'s
`results.csv`; do that before the first RL comparison, and add
`SAT_LIMIT_TICKS` (plan step A), which limits W, so a work budget can be
enforced the way `--conflicts` is. Note `statistics.ticks` is never printed
by kissat today and `k_res` has to be fitted once from the stock traces.

**Splits** (plan §8). `benchmarks/sat-comp-2025-medium` (100 cells) and the
full `benchmarks/sat-comp-2025` (400) are training / in-distribution. Hold a
family-stratified validation split out of all fitting and select checkpoints
on it. `benchmarks/sat-comp-2026` is the true holdout, consulted **at most
once per promoted candidate** — solver 12 scored 296 v 292 on 2025 and 160 v
197 on 2026, which is what an over-fitted candidate looks like.

Every RL comparison carries three arms: stock (policy off), policy-on with
`act == STOCK` (isolates logging and inference overhead), and the candidate.
Report per-family. ±2 solved is deal noise on 100 cells, so the decision
evidence is the 400-cell run plus the tick-deterministic comparison, not a
single medium-suite delta.

Runs:

```bash
# A/B or N-way, simultaneous start on shared pinned cores (no host drift)
python3 tools/feature_ablation.py --arm 'cand:...' --arm 'base:' \
  --suite sat-comp-2025-medium --seeds 1 --jobs 32 --mem-mb 16000

# 400-cell paired run against kissat: two arms, disjoint pinned cores
# (-k picks the binary, -n names log/<name>-<timestamp>, -c is the core offset)
bash tools/run_kissat_full.sh -n kissat-<tag>   -j 16 -c 0  -t 3600 -m 16000 &
bash tools/run_kissat_full.sh -n solver13-<tag> -j 16 -c 18 -t 3600 -m 16000 \
  -k solver/13-kissat-rs/target/release/sat-solver &
python3 tools/compare_full_runs.py <kissat_log_dir> <solver13_log_dir>
```

Solver 13 has **no `SAT_*` feature toggles** — it is a kissat CLI port and
every knob is a kissat option, so an arm's env does nothing until the
one-line `SAT_EXTRA_ARGS` passthrough in `run.sh` lands (plan §7 step 5b /
§10 step 0).

## Correctness is absolute

Wrong SAT/UNSAT status, an invalid SAT model, a missing or invalid UNSAT
proof, or a premature non-budget `UNKNOWN` fails the gate always, no trade,
and must be debugged before tuning continues. Honest timeouts and
budget-consuming `UNKNOWN`s are priced into the metric and are not bugs.

## Benchmarking operations

**Resource cap: at most 32 concurrent solver processes and 16 GB per
process.** The host has 36 physical cores (72 threads) and 502 GB RAM; the
4-core and RAM headroom keeps the machine usable and keeps marginal-cell
timing honest. `--jobs 32 --mem-mb 16000` for `feature_ablation.py`, `-j`/`-m`
for the `run_kissat_*` scripts; the memory number is a per-process ulimit, not
a reservation. Two paired arms split the budget (16 + 16 on disjoint pinned
physical cores), never 32 each.

- Check for live solver/bench processes before launching anything parallel,
  and ask if a sweep is already running.
- **Marginal-cell timing is invalid while another 32-way sweep runs.** Under
  contention a SOLVE is trustworthy but a TIMEOUT is not. Schedule margin and
  wall measurements on a quiet host.
- Long sweeps run for hours and `--seedgate` writes `results.tsv` only at the
  end — launch detached, record the run directory and PID, read progress from
  `_work/<idx>` scratch dirs.
- Killing a run means killing the wrappers *and* the solver children:
  `pkill -f 'bench_reference'; pkill -f 'kissat.*\.cnf'`, then verify with
  `ps`.
- Reference-solver runs go through `tools/run_bench_reference.sh`; pinned
  versions and baseline provenance are in `benchmarks/REFERENCE_SOLVERS.md`.

Calibration: SAT Competition 2025 main track was 5000 s / 30 GB / 8 cores;
kissat-sc2024 won at PAR-2 2788 and 306/400. PAR-2 = runtime for solved
instances plus twice the timeout for each unsolved one, lower better.

## Conventions

- Rust; binary name `sat-solver`; no external SAT/SMT solver dependencies.
- Each iteration is self-contained — copy-and-modify, never a workspace
  dependency between iterations.
- Red-green TDD for behavior changes where practical. Test small hand-crafted
  CNFs before competition-sized ones.
- Never modify `tools/smoke_test.sh` unless explicitly asked.
- Record improvements *and* important rejected attempts in the solver README
  with log paths, machine metadata, and measured impact, so future loops do
  not repeat them.
- Treat stale "rejected for default" notes as hypotheses to re-measure, not
  settled verdicts.
- When reporting command-derived status, show the command and its relevant
  output in the reply; do not rely on hidden tool output.
- Agents share the main checkout on `main`: check `git status --short` and
  live processes before editing, and stop and ask if another agent is in the
  file you need. Coordination details: `plan/agent-coordination.md`.
- For background subagent tasks in Discord, set the task notify policy to
  `silent`; when reporting manually there, mention bjlkeng as
  `<@817490773179760662>`.
- The static benchmark site under `docs/` deploys to
  `https://bjlkeng.io/SAT-playground/`; conventions in `docs/SITE_WORKFLOW.md`.
  Use the `debug-web-visualizations` skill for anything visual.

## Session completion

This repo is **team-maintainer** in the Beads sense: closing beads, running
the gates, committing, and pushing are part of session close, and the Beads
block's conservative default does not apply here.

1. File beads for remaining work; close what is finished.
2. Run the gates for whatever changed (`bash tools/smoke_test.sh
   solver/13-kissat-rs`, `cargo test`).
3. Run the Codex review loop below until it is clean.
4. Nothing to do for beads: the `pre-commit` git hook exports the tracker
   to `.beads/issues.jsonl` and stages it on every commit. Auto-export alone
   is not enough — it is throttled to once a minute, so between exports the
   file lags the database (measured 2026-09-15: database 5 issues, file 2).
   bd also reads that file back in, so a stale export silently restores
   issues you deleted (measured 2026-09-14: 309 deleted issues came
   straight back). There is no Dolt remote; the tracker syncs through git.
   `bd remember` memories are **not** exported at all — if a lesson matters,
   write it into `plan/` or a solver README, which is where this project's
   durable knowledge lives.
5. `git pull --rebase && git push && git status`.

Work is not done until `git push` succeeds. Never stop at "ready to push when
you are".

## Code review

**No manual PR review happens here.** Every change is reviewed by Codex on
GPT-6-Astra at maximum reasoning effort, and Claude applies the fixes:

```bash
codex review --uncommitted -c model="gpt-6-astra" -c model_reasoning_effort="max"
# or, against a base branch / a commit:
codex review --base main   -c model="gpt-6-astra" -c model_reasoning_effort="max"
codex review --commit <sha> -c model="gpt-6-astra" -c model_reasoning_effort="max"
```

Loop: review → Claude fixes the findings → re-review, until a pass returns no
significant findings. What counts as significant, in priority order:

1. **Correctness** — anything that could produce a wrong SAT/UNSAT answer, an
   invalid model or proof, UB, a data race, or a panic on a valid instance.
2. **Metric impact** — anything that could move solved count, tick PAR-2, or
   wall PAR-2 the wrong way, including accidental trajectory changes in
   code that is supposed to be behavior-neutral.
3. Everything else (style, naming, taste) is optional; do not spend loop
   iterations on it.

Record the review verdict in the bead for the work and in the commit message
when the review changed the patch.


<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:6cd5cc61 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

Profile for this repo: **team-maintainer** — see "Session completion"
above, which overrides the Beads block's conservative git default.

The bd git hooks are installed (`bd hooks install --beads`; scripts live in
`.beads/hooks` and `core.hooksPath` points there). `pre-commit` exports the
tracker and stages it, `post-merge` and `post-checkout` import it back. If
you pull this repo into an older checkout and bd reports `workspace identity
mismatch`, that local database predates the 2026-09-15 re-init: delete
`.beads/embeddeddolt` and let bd rebuild it from `.beads/issues.jsonl`.

Note that `bd setup claude` and `bd init` rewrite this block and
`.codex/hooks.json`. After running either, re-check this section and the
Codex `SessionStart` matcher (it must include `compact`).
<!-- END BEADS INTEGRATION -->
