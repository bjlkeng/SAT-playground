# CLAUDE.md — SAT-playground

High-signal contract for coding agents. It carries only what is *not*
derivable from the repo: the current project, the invariants, and the rules
for judging a change. Everything else is a pointer.

## What this repo is

Boolean SAT solvers built iteratively in Rust, one directory per iteration,
all conforming to the SAT Competition solver interface. Iterations 01-12 are
finished history — read them only when a task actually points there. Current
work is **solver 13** and the RL scheduler on top of it.

## Current work

- **`solver/13-kissat-rs`** — faithful Rust reimplementation of kissat 4.0.4
  (reference: `benchmarks/reference-solvers/kissat-latest/`). All engines
  ported, exact counter parity. Third paired 400-instance run (2026-09-06):
  **313 v 312 solved, PAR-2 0.9932x, both-solved wall geomean 0.9875x, zero
  contradictions** — i.e. at or slightly ahead of the C. Details, log paths
  and residual families: `solver/13-kissat-rs/README.md`.
  `solver/13-kissat-rs/CONVENTIONS.md` is **binding** for any port edit.
- **`plan/rl-scheduler-solver13-plan.md`** — the active project. Replace
  kissat's *timing* layer (when to probe / eliminate / reduce / rephase /
  reorder / switch mode, how hard to restart, later per-pass effort) with a
  small CPU MLP consulted every X search ticks. Every mechanism stays
  byte-identical; train offline from logged and fork-branched trajectories
  first. §10 is the work order, §8 the evaluation discipline, §11-15 the
  decision log and review passes. Read it before starting any
  `src/policy.rs` work — that module does not exist yet; plan step A
  creates it.
- **`plan/next-plan.md`** — running session log and handoff notes (newest
  session at the top).

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

## Parity is the solver-13 invariant

`python3 solver/13-kissat-rs/tools/parity.py --corpus default --conflicts N`
diffs every deterministic `-s` counter and the status line against the
reference binary; `--phases` also diffs the `-v` phase lines, which is what
catches watch-stack *layout* divergence the counters cannot see. A faithful
tree matches exactly.

Consequences for new work:

- Any change that is not meant to alter the trajectory must keep 20/20
  parity. RL work is additive and default-off: **policy-off, and policy-on
  with `act == STOCK`, must both stay at exact parity.**
- Do not add counters to the printed `-s` block — `parity.py` compares every
  line, so a new counter breaks the oracle. Put observation counters behind a
  flag or outside that block.
- Port behavior, not intent: where kissat's C does something that looks like
  a bug, reproduce the behavior and leave a `PORT NOTE`.

## Evaluation

The decision metric is lexicographic: **solved count → total conflicts on
tied solved cells → PAR-2** as a supplemental tie-break. Escalate candidates;
never quote a cheap tier as a promotion decision, and always say which tier a
number came from.

1. **Probe (minutes).** 1-15 cells from `benchmarks/discriminating`, short
   walls, mechanism counters from the binary's own `-s` block under
   `--conflicts=N` / `--time=N`. Answers "does it do anything at all".
2. **Triage subset (tens of minutes).** `benchmarks/discriminating`, a
   hand-picked ~15-cell set, or the timeout cells from the newest medium
   results. Match the subset to the mechanism. Answers "which variant, and is
   it worth a gate".
3. **Gate (hours).** 100-instance `benchmarks/sat-comp-2025-medium` single
   default seed, 1800 s / 16 GB / 32 pinned cores:

   ```bash
   python3 tools/feature_ablation.py --arm 'cand:SAT_EXTRA_ARGS=--eliminateint=1000' \
     --arm 'base:' --suite sat-comp-2025-medium --seeds 1 --jobs 32 --mem-mb 16000
   python3 tools/check_promotion_gate.py --multiseed --candidate <cand.tsv> \
     --baseline <base.tsv> --timeout 1800 --memory-mb 16000
   ```

   Both arms start simultaneously on shared pinned cores, so there is no host
   drift between them. Up to 4 arms (3 candidates + a mandatory `base:`),
   varying **one** axis per sweep.

   Note: solver 13 has **no `SAT_*` feature toggles** — it is a kissat CLI
   port, and every knob is a kissat option. An arm's env therefore does
   nothing until the one-line `SAT_EXTRA_ARGS` passthrough in `run.sh` lands
   (plan §7 step 5b / §10 step 0); the older `--arm 'x:SAT_FEATURE=on'` form
   in the git history targeted solvers 11-12.

4. **Paired 400-cell run** — the solver-13-scale check against kissat:
   `tools/run_kissat_full.sh` (`-k` to run our binary, `-n` to name the log
   dir) for each arm on disjoint pinned cores, then
   `python3 tools/compare_full_runs.py <baseline_dir> <candidate_dir>`.
   ~10-12 h at 3600 s.

For RL candidates specifically (plan §8): ±2 solved is noise on 100 cells, so
a +3 policy is invisible on the medium suite — decide on the 400-cell run
plus a **tick-budgeted deterministic comparison** (`SAT_LIMIT_TICKS`, to be
built in plan step A), and keep the medium gate as the in-distribution
check. The medium suite is 100% inside sat-comp-2025 and therefore a
*training-set* gate;
`benchmarks/sat-comp-2026` is the true holdout and is consulted **at most
once per promoted candidate**. (Solver 12's lesson: 296 v 292 on 2025, 160 v
197 on 2026.) Every RL gate carries three arms: stock, policy-on-with-STOCK
(isolates logging/inference overhead), and the candidate.

## Judging trades

A raw lexicographic regression does not automatically mean revert. Classify
every changed cell first:

- **Wall-coin cell** — either test qualifies it: the baseline solved within
  ~120 s of the timeout, *or* the cell is a documented flipper (observed to
  flip solved/unsolved across deals at an *identical conflict count*).
  Conflicts are exactly deterministic across load while wall is not, so
  identical conflicts with a different outcome is proof of pure wall luck
  whatever the margin. The flipper list lives in `plan/next-plan.md`
  "Standing traps".
- **Capability cell** — a solve with real margin and a stable trajectory, or
  a first-ever solve. Signal.

Calibration: three gates on one host on 2026-07-24 scored the *same* baseline
67, 69, and 71. **±2 solved cells is deal noise.**

> A candidate may lose up to **2** wall-coin cells (3 with written
> justification) and still be promotable, **provided** it gains
> mechanism-validated capability elsewhere.

"Mechanism-validated" means explained and reproducible: a first-ever solve, a
fat margin, a digit-exact identity check on untouched cells, or a measured
mechanism (elimination depth, propagation rate, proof size) that accounts for
the win. Fails the rule: losing a large-margin cell (real capability loss);
winning only wall coins while conflicts and wall regress (a reroll lottery).

Do not revert on any loss — judge the trade explicitly and write it into the
promotion note (cells gained, cells lost with baseline margins, mechanism
evidence, conflicts/PAR-2 movement). If genuinely ambiguous, say so and ask.
Validate suspicious wins against variable-renamed / clause-shuffled copies
(`tools/shuffle_cnf.py`, `tools/shuffle_sensitivity.py`) before believing
them; do not promote hard-coded guards or one-family classifiers.

## Correctness is absolute

Wrong SAT/UNSAT status, an invalid SAT model, a missing or invalid UNSAT
proof, or a premature non-budget `UNKNOWN` fails the gate always, no trade,
and must be debugged before tuning continues. Honest timeouts and
budget-consuming `UNKNOWN`s are priced into the metric and are not bugs.

## Benchmarking operations

- Check for live solver/bench processes before launching anything parallel,
  and ask if a sweep is already running. Cap memory so `jobs * mem` fits RAM.
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

Work is not done until `git push` succeeds. Run the gates for whatever
changed (smoke test, `cargo test`), commit, then `git pull --rebase &&
git push && git status`. Never stop at "ready to push when you are". Record
follow-up work in `plan/next-plan.md`.
