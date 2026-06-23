---
name: nextbeads
description: Work through up to N highest-priority unblocked beads drawn from phase1 or phase2 (search work plus inprocessing / rewriting kissat features; pass a single phase label to narrow), one bead at a time. Claims, implements, validates, closes or releases every claimed bead, commits, pushes, and reports. Benchmarks are launched detached, validated after launch, then polled hourly instead of being waited on continuously.
---

# /nextbeads - Phase-Scoped Bead Work

## Purpose

Work through up to `N` ready beads drawn from `phase1` or `phase2`, ranked
together by priority. `phase1` is search, decision, restart, and learned-clause
work; `phase2` is the inprocessing / rewriting kissat features: clause
simplification, rewriting, and formula modification (for example the
inprocessing scheduler, vivification, probing, equivalent-literal substitution,
and gate-aware BVE). Pass a single phase label to narrow to just that phase. Use
this skill when the user asks for `/nextbeads` or asks an agent to take ready
Beads work in priority order.

## Non-Negotiables

- One bead at a time. Do not interleave implementations.
- Pick only from `phase1` and `phase2` (or the single phase requested). Do not
  pull beads from other epics / work-areas (e.g. governance, hot-path) unless the
  user explicitly asks.
- Claim before implementation and release every claimed bead before exit.
- Work in `/home/bojji/code/SAT-playground` on `main`, not in worktrees or
  routine feature branches.
- Before touching files, check active beads, `git status --short`, and live
  solver/bench processes. If another active agent appears to be editing the file
  or region you need, stop and ask the user before proceeding.
- For solver changes, cargo tests and smoke tests must pass before commit.
- Never modify `tools/smoke_test.sh` unless the user explicitly asks.
- Never force-push and never use `--no-verify` or `--no-gpg-sign`.

## Invocation

```text
/nextbeads [N] [phaseLabel]
```

- `N`: maximum beads to complete. Default: `5`.
- `phaseLabel`: optional single phase to narrow to (`phase1` or `phase2`).
  Default: both — pick from `phase1` (search) and `phase2` (inprocessing /
  rewriting kissat features: clause simplification, rewriting, formula
  modification), ranked together.

Examples:

- `/nextbeads`           # up to 5 from phase1 or phase2
- `/nextbeads 3`         # up to 3 from phase1 or phase2
- `/nextbeads 5 phase2`  # narrow to phase2 only
- `/nextbeads 5 phase1`  # narrow to phase1 only

## Required Reading

Read these before selecting work. A compaction may have evicted project rules.

1. `CLAUDE.md`
2. `plan/solver-optimization-workflow.md`
3. `benchmarks/BENCHMARK_WORKFLOWS.md`
4. `.agents/skills/beads/SKILL.md`
5. Target solver README and feature/state files:
   `solver/NN-name/README.md`, plus `FEATURES.md`, `FEATURES.csv`, or a
   `SOLVERNN_STATE.md` (e.g. `SOLVER12_STATE.md`) when present.
6. Any `log/<investigation>/FINDINGS.md` or `DEEPER_FINDINGS.md` referenced by
   candidate beads.

## Benchmark Discipline

Benchmark commands are measurement runs, not tests. This applies to
`tools/bench.sh`, `tools/bench_reference.sh`, `tools/run_bench_reference.sh`,
`tools/feature_ablation.py`, and similar profiling or ablation commands. Unit
tests, smoke tests, and tiny direct solver repros may still run synchronously.

### Launch

After the initial benchmark launch, immediately validate that it is running.
Do not run benchmark commands in the foreground with a blocking wait.

Use a detached command with a durable log, for example:

```bash
mkdir -p log/nextbeads
stamp=$(date +%Y%m%d-%H%M%S)
run_log="log/nextbeads/<tag>-$stamp.log"
nohup bash -lc '<benchmark command>' > "$run_log" 2>&1 &
pid=$!
echo "$pid" > "$run_log.pid"
```

Record the command, PID, log path, start time, expected output directory or TSV,
and benchmark purpose in your status note.

### Validate Startup

Within 30-120 seconds of launch, verify one of these is true:

- the PID is still alive:
  ```bash
  ps -p "$pid" -o pid,etime,cmd
  ```
- the benchmark finished quickly and produced an expected `results.csv`,
  `results.tsv`, `DONE`, or equivalent artifact.

Also inspect the log tail:

```bash
tail -40 "$run_log"
```

If the process exited without expected output, stop and report the failure. Do
not claim or close beads using missing benchmark evidence.

### Hourly Polling

Once the benchmark is confirmed running, poll it about once per hour. Do not keep
a tool call open just to wait. Use a scheduler/wakeup facility if available; if
not, report the next poll time and resume when prompted.

Each poll should report:

- whether the PID is alive;
- current solver/bench processes:
  ```bash
  pgrep -af 'sat-solver|kissat|minisat|bench|feature_ablation'
  ```
- latest log tail;
- progress from `results.csv` / `results.tsv` row count or benchmark-specific
  scratch dirs, when available;
- ETA when there is enough information to estimate it.

If the benchmark is still running, leave it running and schedule/report the next
hourly check. If it is done, parse results and continue the workflow.

### Dependency On Results

Do not continue into a step that depends on benchmark results until those results
exist. For example:

- The before benchmark must finish before using it as the comparison baseline.
- The after benchmark must finish before making a before/after PAR-2 claim.
- A seedgate must finish before a keep/promote/revert decision.

If a required benchmark is still running, release any claimed beads, report the
PID/log/progress, and stop the current turn cleanly.

## Preflight

1. Refresh context:
   ```bash
   bd prime
   bd export -o .beads/beads.jsonl
   git status --short
   bd list --status=in_progress
   ps aux | grep -E 'cargo|sat-solver|bench\.sh|feature_ablation|kissat|minisat' | grep -v grep
   ```

2. Confirm phase scope. By default that is both `phase1` and `phase2`; if a
   single `phaseLabel` was given, use only that one:
   ```bash
   for ph in phase1 phase2; do   # or just the requested phaseLabel
     bd list --label "$ph" --status=open
     bd list --label "$ph" --status=in_progress
   done
   ```
   If no ready/open work exists in scope, stop and say so.

3. Use `bv` only for read-only graph analysis:
   ```bash
   bv --robot-triage
   bv --robot-plan
   bv --robot-insights
   ```

4. Build the work order from ready beads across the in-scope phases (`phase1`
   and `phase2`, or the single requested phase), ranked together. Rank by
   blockers/dependencies, priority, scope risk, implementation leverage, and
   downstream unblock count. Drop recommendations outside the in-scope phases.

5. Start the before benchmark for the whole run using the benchmark discipline
   above:
   ```bash
   cd solver/NN-name && bash build.sh && cd -
   bash tools/bench.sh -d benchmarks/profiling solver/NN-name
   ```
   Wait only via hourly polling. Do not start bead implementation until this
   baseline has completed and the `results.csv` path is known.

## Per-Bead Loop

For each bead in the work order, up to `N`:

1. Re-confirm scope:
   ```bash
   bd show <id>
   ```
   Verify the bead is still in scope (`phase1` / `phase2`) and unblocked.

2. Claim and note:
   ```bash
   bd update <id> --claim
   bd note <id> "Claimed via /nextbeads. Work order position M of K."
   ```

3. Plan briefly from the bead, referenced findings, and source. State the edit
   target before touching code.

4. Implement the minimum focused change. Use red-green TDD when practical.

5. Keep durable progress in the bead with `bd note <id> "..."` for decisions,
   surprises, and resume context.

6. Fresh-eyes review every file changed for this bead. Check correctness,
   edge cases, invariants, and integration with adjacent modules. Fix findings
   before validation.

7. Run required validation:
   ```bash
   cd solver/NN-name && cargo test && cd -
   bash tools/smoke_test.sh solver/NN-name
   ```
   If extra benchmark validation is needed, launch it with the benchmark
   discipline above and wait by hourly polling until results exist.

8. Update the bead:
   ```bash
   bd note <id> "Implemented: <summary>. Reviewed: <findings>. Tests: cargo test PASS, smoke PASS."
   ```

9. Close and release:
   ```bash
   bd close <id> --reason="Completed via /nextbeads"
   bd update <id> --assignee ''
   bd export -o .beads/beads.jsonl
   ```

10. Commit and push this bead:
    ```bash
    git add <files actually changed>
    git commit -m "<bead-id>: <short title>"
    git pull --rebase origin main
    # If rebase moved the base or produced conflicts, resolve and re-run required validation.
    git push
    ```
    Avoid `git add -A` when unrelated files exist.

## Failure Or Early Stop

Stop the loop immediately on:

- failing cargo tests or smoke tests that cannot be fixed in scope;
- wrong SAT/UNSAT status, invalid SAT model, or missing/invalid UNSAT proof;
- premature non-budget `UNKNOWN`;
- benchmark evidence required for the next decision still running;
- merge conflict or rebase interaction you cannot resolve confidently.

Before exiting:

1. Revert only files touched by the failed bead when a revert is appropriate:
   ```bash
   git restore --staged --worktree -- <files-touched-by-this-bead>
   ```
   Never run `git reset --hard` blindly.

2. Reopen and release unfinished beads:
   ```bash
   bd update <id> --status=open --assignee ''
   bd note <id> "Released by /nextbeads YYYY-MM-DD: <reason and handoff>."
   bd export -o .beads/beads.jsonl
   ```

3. If a benchmark is still running, report PID, log path, current progress, and
   next hourly poll time.

## Post-Loop

1. Verify every bead claimed by this run is either closed or reopened and has no
   assignee. Do not alter beads claimed by other active agents.

2. Start the after benchmark using the benchmark discipline:
   ```bash
   cd solver/NN-name && bash build.sh && cd -
   bash tools/bench.sh -d benchmarks/profiling solver/NN-name
   ```
   Poll hourly until `results.csv` exists. Do not make before/after claims while
   it is still running.

3. Compare before and after results. Report per-instance status/time deltas,
   aggregate PAR-2, any new ERROR/wrong-result/premature-UNKNOWN rows, and the
   config/env used.

## Final Report

When all required results exist, finish with:

1. Beads worked on: order, bead ID, title, final status.
2. What changed: file/function granularity and any non-obvious decisions.
3. Validation: cargo, smoke, and benchmark logs/results.
4. Profile-bench before/after: aggregate PAR-2 verdict plus notable rows.
5. Next logical beads: top three ready beads in scope (`phase1` / `phase2`) and why.

If a required benchmark is still running, do not give the final comparison.
Instead give a monitoring report with PID/log/progress and the next poll time.

## Guardrails

- Use `bd`, not bare `bv`, for mutations.
- Re-export `.beads/beads.jsonl` after Beads changes.
- Convert relative dates in bead notes to absolute dates.
- Leave unrelated working-tree changes untouched.
- If `bd prime` reports no workspace, stop and tell the user.
