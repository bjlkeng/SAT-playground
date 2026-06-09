---
name: analyzesat
description: Scientific bottleneck analysis for a SAT solver iteration — runs multi-config ablation across the profiling suite, profiles with perf, decomposes regressions into work × speed, diffs the reference source, writes FINDINGS.md + DEEPER_FINDINGS.md, creates beads for new actionable issues, summarizes findings to screen, and commits the artifacts. Default target is solver/11-kissat-port. Pass an optional solver path argument to target a different iteration (e.g. /analyzesat solver/10-bve-preprocess).
---

# Analyze SAT Solver Bottlenecks

## Purpose

Run a comprehensive, scientific investigation into why ported or new solver features
are not improving — or are actively regressing — **aggregate PAR-2 over the profiling
suite**. The goal is to attribute the aggregate-PAR-2 effect to specific implementation
gaps versus workload mismatch, and to produce per-feature code-level recommendations
(file + function + fix sketch + reference citation) rather than parameter-tuning suggestions.

**The only success metric is aggregate PAR-2 over the whole profiling suite.** A feature
that regresses some instances — including flipping a solved instance to a timeout — while
improving total PAR-2 is a *win*, not a regression to be explained away; analyze it as such
and recommend keeping it. Conversely, a feature that helps a few instances but loses aggregate
PAR-2 is a loss even if its per-instance wins look impressive. Per-instance deltas are
diagnostic detail used to understand *why* the aggregate moved; they are never the verdict.
Honest timeouts/`UNKNOWN` (the solver used its budget and didn't finish) are a priced-in PAR-2
cost; only a *premature* `UNKNOWN` (returns without using the budget) or a correctness error
(wrong status, bad model, missing/invalid proof) is a hard stop-and-debug bug.

**Use kissat (and other reference solvers) for inspiration, hypotheses, and comparison — not
as a 1-to-1 implementation target.** Read kissat to understand *where* and *why* it does less
work or runs faster, then adapt the idea however best fits this codebase. Exact behavioral or
source parity with kissat is explicitly **not** a goal; keep whatever wins aggregate PAR-2.

This skill is the primary tool for "fresh eyes" deep-dives on `solver/11-kissat-port`
or any other iteration. It integrates the workflows from CLAUDE.md sections:
- *Investigating Why Ported Features Don't Help*
- *Debugging Optimization Regressions*
- *Reference Solver Gap-Closing Strategy*

## Invocation

```
/analyzesat [solver/NN-name]
```

Default target: `solver/11-kissat-port`

## Pre-flight

1. **Identify the target solver** from the argument or default.
2. **Read current state**: read `solver/NN-name/README.md`, `FEATURES.md` / `FEATURES.csv`,
   `SOLVER11_STATE.md` (if present), and `src/config.rs` to understand which features exist
   and which are opt-in vs. default.
3. **Read CLAUDE.md** sections for the current iteration and the optimization workflow.
4. **Check existing beads**: run `bd search <feature>` for every feature under investigation.
   Record existing bead IDs so findings are linked, not duplicated.
5. **Work in the main checkout on `main`** (no worktrees — see CLAUDE.md's coordination
   workflow). Before touching any source file, check whether another agent is already
   working on it (coord thread, `bd list --status in_progress`, `git status --short`); if a
   likely conflict exists, **ask the user before proceeding**. An analysis pass that only
   reads + profiles + writes to `log/` rarely conflicts; a profiling rebuild does change the
   shared target binary, so coordinate via the `coord` thread if another agent is mid-build.
   Build the target solver in place:
   ```bash
   cd solver/NN-name && bash build.sh
   ```
6. **Name the investigation**: pick a slug like `analyzesat-YYYY-MM-DD` and create the output
   directory: `log/<slug>/`.

## Phase 1 — Multi-Config Ablation Matrix

Define at least these configs over the full `benchmarks/profiling/` suite (10 instances):

| Config | Env vars | Purpose |
|--------|----------|---------|
| `A_baseline` | (defaults only) | Legacy baseline — solver-10-compatible |
| `B_metadata_only` | `SAT_USE_LBD=on` | Bookkeeping only; should match A on conflicts/decisions/props |
| `C_lbd_ema` | `SAT_USE_LBD=on SAT_RESTART=kissat-ema` | EMA restarts alone |
| `D_focused_stable` | `SAT_SEARCH_MODE=focused-stable` | Full focused/stable path |
| `E_combined` | `SAT_SEARCH_MODE=focused-stable SAT_USE_LBD=on` | Combined |
| `F_full_stack` | all stable opt-ins together | Everything enabled |

Add or drop configs based on what `FEATURES.csv` says is SmokeSafe or Experimental.

Run each config against all 10 profiling instances with:

```bash
tools/bench.sh -t 300 -m 16384 -d benchmarks/profiling solver/NN-name
```

Set env vars via wrapper or by temporarily modifying config defaults; record the exact
`SAT_*` env string for every run in the results so conclusions are reproducible.

**Long matrices run in the background with hourly status.** A full multi-config × 10-instance
matrix at 300 s is a multi-hour job — launch it detached (Bash `run_in_background: true`, or the
one-shot cron pattern for very long runs) instead of blocking the session, post an hourly status
report while it runs (liveness via `pgrep`, cells-done, ETA), and close with the comparative
analysis/summary this skill already produces (the Phase 2–7 decomposition + `FINDINGS.md` + the
screen summary). See CLAUDE.md's "Code-Level Optimization Workflow" for the full
detached-run / hourly-status / end-of-run-summary convention.

Capture per-(config, instance):
- wall time, result (SAT/UNSAT/timeout)
- conflicts, decisions, propagations, restarts (from `SAT_STATS_JSON=on`)
- max RSS, minor page faults (`/usr/bin/time -v`)

Save raw `results.csv` and `stdout/stderr` per config under `log/<slug>/`.

**Bookkeeping sanity check**: confirm `B_metadata_only` matches `A_baseline` on
conflicts/decisions/propagations. Any divergence past `B` is signal.

## Phase 2 — Work × Speed Decomposition

For every `(config, instance)` pair that diverges from `A_baseline`, compute:

```
work_ratio  = conflicts_cfg / conflicts_A          # search trajectory effect
speed_ratio = (props/s)_A / (props/s)_cfg          # per-event execution cost
net         = work_ratio × speed_ratio             # predicted wall ratio
```

Compare `net` to measured wall ratio:
- `net ≈ measured` → decomposition is clean; report `work` vs `speed` cause
- large gap → suspect a third factor (GC, allocation, proof writing, DB bloat)

Trajectory-only features (EMA restart) show `speed ≈ 1.0`.
Execution-only effects (watcher growth, DB bloat) show `work ≈ 1.0`.

## Phase 3 — Reference Solver Live Comparison

Run the vendored kissat binaries on the same profiling instances to establish
empirical targets for conflicts, propagations, decisions, and wall time.

```bash
# Run kissat-latest and kissat-sc2024 on the profiling suite
bash tools/bench_reference.sh -t 300 -m 16384 \
  -d benchmarks/profiling kissat-latest kissat-sc2024
```

Results land in `log/bench-kissat-latest-<ts>/results.csv` and
`log/bench-kissat-sc2024-<ts>/results.csv`. Copy or symlink them under
`log/<slug>/reference-kissat-latest.csv` and `reference-kissat-sc2024.csv`.

For instances where kissat is faster than the repo solver's `A_baseline`, compute
the gap analysis per instance:

```
ref_work_ratio  = conflicts_repo / conflicts_kissat   # how many more conflicts?
ref_speed_ratio = (props/s)_kissat / (props/s)_repo   # how much slower per prop?
```

Classify each instance gap:
- `ref_work_ratio >> 1, ref_speed_ratio ≈ 1` → **trajectory gap** (search quality)
- `ref_work_ratio ≈ 1, ref_speed_ratio >> 1` → **execution gap** (propagation throughput)
- both elevated → **combined gap**
- kissat loses or ties → repo is already competitive on this instance (note it)

This gives a per-instance "what to fix first" signal before reading source code.
Also note instances where kissat **times out** but the repo solver solves — these
are instances where the repo's preprocessing gives an advantage worth preserving.

Add the reference comparison table to `log/<slug>/FINDINGS.md`.

## Phase 4 — Reference Source Diff (for inspiration, not parity)

For a feature whose aggregate-PAR-2 effect you want to understand or improve, read the reference
implementation for ideas about *where it spends or saves work* — **not** to replicate it
line-for-line. Kissat is a source of hypotheses about which work to cut or which search decisions
pay off; this codebase can reach the same PAR-2 effect with a different mechanism. Behavioral or
source parity with kissat is not the goal — a smaller, simpler, or differently-shaped change that
wins aggregate PAR-2 is preferred over a faithful port that does not.

```bash
# Kissat is vendored under benchmarks/reference-solvers/kissat/src/
ls benchmarks/reference-solvers/kissat/src/
```

For each feature, find the C file (e.g. `restart.c`, `reduce.c`, `tiers.c`, `analyze.c`)
and read the *execution model* line by line:
- what the reference does on each event (restart, reduce, decide, learn, propagate)
- what state survives between events (trail reuse, used counters, tier assignments)
- which side-effects happen at which boundary (queue drains, heap rebuilds, EMA resets)

Use the Phase 3 gap classification to prioritize which features to diff first:
trajectory gaps → focus on restart/branching/phase files; execution gaps → focus
on propagation/watcher/arena files.

For each gap found, predict which instances/configs would change if the gap were closed.
Verify the prediction against Phase 1/2/3 data. Only call something a "gap" if the
prediction matches observed regressions.

Kissat ideas worth checking for solver 11 (as inspiration — a similar *effect* is the target,
not an identical implementation):
- `restart.c` — would trail reuse vs `backtrack(0)` change the aggregate?
- `reduce.c` / `tiers.c` — does the Rust tiered reducer keep roughly the right clauses (age/classify intent), even if the order differs?
- `analyze.c` — is conflict analysis finding good UIP/learned clauses (the same *quality*, not the same code path)?
- `decide.c` — is VMTF queue state preserved sensibly across restarts?
- `search.c` — does mode switching happen at a boundary that helps the aggregate?
- A correctness divergence from kissat is still a bug; a *behavioral* divergence that wins aggregate PAR-2 is fine.

## Phase 5 — Trajectory Trace for Critical Instances

Pick 1–2 instances where regression is largest. Run at least two configs with search traces:

```bash
SAT_TRACE_SEARCH_INTERVAL=20000 SAT_STATS_JSON=on \
  timeout 300 ./target/release/sat-solver instance.cnf /tmp/proof \
  2>&1 | tee log/<slug>/trace_<instance>_<config>.txt
```

The trace logs per-interval: seconds, conflicts, decisions, propagations, restarts,
current level, trail length, `live_learned_clause_count`.

Compare trajectories side-by-side. If they are identical through a prefix then diverge
sharply, document this as **phase-boundary chaos** (VSIDS picks a different branching
literal at one critical decision — parameter tuning will not reliably fix it; the
remedy is algorithmic simplification / inprocessing or accepting it as a coin flip).

## Phase 6 — Hardware Performance Counters

For the top 1–2 regressing instances, run `perf stat` on both baseline and the
regressing config:

```bash
perf stat -e cycles,instructions,branches,branch-misses,\
L1-dcache-loads,L1-dcache-load-misses,dTLB-loads,dTLB-load-misses,\
cache-references,cache-misses \
  timeout 300 ./target/release/sat-solver instance.cnf /tmp/proof
```

Normalize cache/TLB misses by propagations or conflicts so search-path differences
do not hide the real per-event cost.

If a hot function is suspected, profile with:
```bash
CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=1 \
  RUSTFLAGS="-C target-cpu=native" cargo build --release
perf record -e cache-misses -g --call-graph dwarf \
  timeout 120 ./target/release/sat-solver instance.cnf /tmp/proof
perf report --stdio --no-children --sort symbol
perf annotate --stdio --source --symbol '<hot_symbol>'
```

## Phase 6 — Targeted Parameter Sweeps

For each suspected bottleneck, define a knob change that should rescue failing instances
if the hypothesis is correct. Sweep at a slightly reduced timeout (e.g. 240s vs 300s)
on just the failing instances. Examples:

- "EMA restarts fire too often" → sweep `SAT_RESTART_BLOCK_MARGIN` (0.0, 1.2, 1.4, 1.6)
- "Reducer deletes too early" → sweep `SAT_REDUCE_DB_INIT` (500, 1000, 2000)
- "Phase-saving misleads" → sweep `SAT_PHASE` (saved, target-then-saved, best)

A sweep that rescues zero failing instances refutes the hypothesis. Document the
trade-off (which instances regress when another is rescued) so the same sweep is
not re-run.

## Phase 7 — Preprocess Stats Check

If solver 11 preprocessing is involved in regressions:

```bash
SAT_TRACE_PREPROCESS=1 SAT_STATS_JSON=on \
  timeout 300 ./target/release/sat-solver instance.cnf /tmp/proof
```

Capture: pre/post variables and clauses, eliminated variables, resolvents, subsumed
clauses, strengthened literals, root assignments, preprocessing time.

Compare to a `SAT_SIMPLIFICATION=off` run and to a reference solver on the same
instance to separate preprocessing regressions from search regressions.

## Synthesis — Findings and Recommendations

### FINDINGS.md

Write `log/<slug>/FINDINGS.md` with this structure:

```markdown
# Bottleneck Analysis — solver/NN-name — YYYY-MM-DD

## Executive Summary
<3–5 bullet points: what was found, what is the dominant cause, what needs code changes>

## Config Matrix Results
<table: config × instance × (wall, conflicts, props/s, result)>

## Reference Solver Live Comparison
<table: instance × (solver-wall, kissat-wall, ref_work_ratio, ref_speed_ratio, gap type)>
<note any instances where repo already beats kissat; flag any kissat timeouts where repo solves>

## Work × Speed Decomposition
<table: (config, instance) × (work_ratio, speed_ratio, net, actual, dominant cause)>

## Reference Diffs — Implementation Gaps
<per-feature: gap description, reference file:line, Rust file:line, prediction, verification>

## Trajectory Analysis
<per-instance trace summary; phase-boundary chaos calls clearly labeled>

## Hardware Counter Results
<perf stat table normalized per propagation>

## Parameter Sweep Results
<per-hypothesis: knob, range swept, rescue count, trade-offs>

## Code-Level Recommendations (ordered by ROI)
1. <file:function — fix sketch — reference citation>
2. ...

## Rejected Sweeps / Non-Issues
<what was ruled out and why>

## Artifact Paths
- Ablation script: log/<slug>/run_ablation.sh
- Raw results: log/<slug>/<config>/results.csv
- Reference solver results: log/<slug>/reference-kissat-latest.csv, reference-kissat-sc2024.csv
- Trace logs: log/<slug>/trace_*.txt
- Profile data: log/<slug>/perf_*.txt
```

If the investigation warrants a follow-up dive, write `DEEPER_FINDINGS.md` for the
second-pass findings.

### Screen Summary

After writing FINDINGS.md, print a concise summary to stdout with:
- Top 3 bottlenecks identified (one sentence each)
- Dominant cause per feature: work (trajectory) vs speed (execution) vs gap (reference diff)
- Number of new beads created
- Any phase-boundary chaos instances (cannot be fixed by parameter tuning)
- What to do next

### Beads

For every new actionable finding:

1. Check for an existing bead first: `bd search <keyword>` (mandatory — do not duplicate).
2. **Classify the finding into a roadmap phase before creating the bead.** Every new solver
   bead MUST be filed under the correct phase epic — both the phase **label** *and*
   `--parent <phase-epic>` — so phase-scoped tooling (`/nextbeads phaseN`, the phase guard, and
   `bv --robot-triage-by-label`) can see it. A standalone bead with no parent epic is invisible
   to a phase-scoped run even when it is squarely that phase's work. Map the finding's primary
   subsystem to a phase:

   | Subsystem the finding's primary code change touches | Phase | Label | Parent epic |
   |-----------------------------------------------------|-------|-------|-------------|
   | Search loop & priority order, decision/branching (VSIDS/VMTF/`SAT_REORDER`), phase saving / rephase, restart / trail-reuse, chronological backtracking, propagation / BCP / watchers / binary fast-path, conflict analysis, clause minimization / shrink (CCMIN), learned-clause DB (reduce-db / lbd-tiered / tiers), lucky pre-search phase patterns | **Phase 1** | `phase1` | `SAT-playground-5b2.2` |
   | Clause simplification, bounded variable elimination (BVE), vivification, subsumption / self-subsuming resolution, probing / hyper-binary resolution, formula rewriting, the inprocessing scheduler, preprocessing | **Phase 2** | `phase2` | `SAT-playground-5b2.3` |

   Rule of thumb: if the change is in the **solve/search loop and does not modify the formula**,
   it is Phase 1; if it **simplifies, rewrites, or eliminates from the formula**, it is Phase 2.
   For a finding that genuinely spans both, file it under the phase of its *primary* code change
   and link the other epic with a dependency. If it fits neither phase, use the matching epic
   instead — `SAT-playground-5b2.4` (governance / invariants / gates), `SAT-playground-5b2.5`
   (milestones / promotion gates), or `SAT-playground-5b2.6` (parking-lot experiments) — but
   **never leave a new solver bead with no parent epic.**

3. If no bead exists, create it under the chosen phase epic (Phase 1 shown; for Phase 2 swap
   `--parent SAT-playground-5b2.3` and label `phase2`):
   ```bash
   bd create --title "<feature>: <specific gap>" \
             --type bug \
             --priority 2 \
             --parent SAT-playground-5b2.2 \
             --no-inherit-labels \
             --labels "solver11,performance,phase1,<subsystem-label>" \
             --description "Gap: <description>. Reference: kissat/src/<file>:<line>. Fix: <sketch>. Evidence: log/<slug>/..."
   ```
   `--no-inherit-labels` keeps the discovered bead from picking up the epic's *planning* labels
   (`plan`, `roadmap`, `task-1-N`); carry only the explicit phase label plus topical subsystem
   labels (e.g. `reduce-db`, `propagation`, `restart`, `clause-minimization`, the
   `analyzesat-<slug>` tag).
4. If a bead exists but has no phase, fix it in place — do not create a duplicate:
   ```bash
   bd update <id> --add-label phase1 --parent SAT-playground-5b2.2   # or phase2 / 5b2.3
   ```
5. If a bead exists, add a note:
   ```bash
   bd note <id> "Confirmed by analyzesat run <slug>: <evidence>"
   ```
6. Link related beads: `bd link <new> <existing> --type related` (or `bd dep add <new> <existing>`
   for a discovered-from / blocks relationship).
7. Export: `bd export -o .beads/beads.jsonl`

## Commit

After writing FINDINGS.md and creating/updating beads:

```bash
git add log/<slug>/FINDINGS.md log/<slug>/DEEPER_FINDINGS.md \
        log/<slug>/run_ablation.sh \
        log/<slug>/reference-kissat-latest.csv \
        log/<slug>/reference-kissat-sc2024.csv \
        .beads/beads.jsonl
git commit -m "analyzesat: <slug> — <one-line summary of top finding>"
```

Do not force-add raw results CSVs unless the user asks for provenance tracking.

## Rules

- **Work on `main` in the main checkout** — no worktrees. Check for another agent on the same files first and **ask the user before proceeding** into a likely conflict (CLAUDE.md coordination workflow).
- **Check beads before creating** — `bd search` is mandatory; never duplicate.
- **File every new bead under its phase epic** — classify the finding (Phase 1 = search/decision/
  phase/restart/learned-clause; Phase 2 = simplification/rewriting/formula modification) and set
  *both* the phase label and `--parent <phase-epic>`. A new solver bead must never be left
  parent-less, or phase-scoped tooling (`/nextbeads phaseN`) cannot see it.
- **Report phase-boundary chaos honestly** — do not suggest parameter tuning when
  the trajectory trace shows single-decision divergence.
- **Tie every recommendation to measured evidence** — avoid generic "improve cache
  locality"; cite the hot symbol and source line.
- **Judge every config by aggregate PAR-2, not per-instance rows** — recommend keeping a config
  that improves total profiling-suite PAR-2 even when it regresses or newly times out individual
  instances. Per-instance deltas explain *why* the aggregate moved; they are not the verdict.
- **Distinguish a priced-in timeout from a bug** — an honest timeout / resource-limit `UNKNOWN`
  (the solver used its budget) is just a PAR-2 cost and a valid data point. A *premature* `UNKNOWN`
  (returns without using the budget) or any correctness error (wrong status, bad model,
  missing/invalid proof) is a bug regardless of PAR-2 — stop and report it before continuing.
- **Commit artifacts** — always commit FINDINGS.md and `.beads/issues.jsonl` at the end.
- **Print a screen summary** — the user cannot see FINDINGS.md until they open it;
  summarize the top findings to stdout before exiting.
