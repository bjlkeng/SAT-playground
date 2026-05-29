---
name: nextbeads
description: Work through up to N highest-priority unblocked beads within a single phase (default phase1), one bead at a time — claim, implement, fresh-eyes review, fix, run smoke + cargo test, update the bead, commit, push. Run the profiling-suite benchmark before the first bead and after the last to confirm no regression. Never crosses into the next phase. Reports beads worked on, what was implemented (with examples), the before/after profile-bench delta, and the next logical beads to tackle.
---

# /nextbeads — Work Phase-Scoped Beads In Priority Order

## Multi-agent coordination — read this first

Multiple agents may be working in this repo at the same time. Before you touch any
bead, assume another agent is already running and verify otherwise. The rules below
exist because **almost all solver logic lives in a single file** (`solver/11-kissat-port/src/main.rs`,
~13K lines), so two agents editing concurrently will collide even on logically
unrelated beads. File-level claims do not solve this — only the rules below do.

### Required pre-claim check

Run both of these before calling `bd update --claim` on anything:

```bash
# 1. Is the bead — or another shrink/inblock/<your-area> bead — already in progress?
bd list --status in_progress
bd search <area-keyword>     # e.g. shrink, propagation, reduce-db, restart

# 2. Is another agent actively running cargo / solver / benchmark processes?
ps aux | grep -E 'cargo|sat-solver|bench\.sh|run_ablation|nextbeads' | grep -v grep
```

If either check shows activity in your area, **stop and tell the user**. Do not race.
Pick a different bead, or wait for the other agent to close.

### Claim discipline

When you start work on a bead:

```bash
bd update <id> --claim                    # sets owner = you
bd update <id> --status in_progress       # makes it visible to other agents
```

When you finish or revert:

```bash
bd close <id>                             # or bd update <id> --status open if reverting
```

Beads tracks *declared* state. There is no heartbeat or file lock — if you do not
claim, no one else can see you are working.

### Worktree isolation

Always work in a dedicated worktree, never the main checkout:

```bash
git worktree add /tmp/nextbeads-<slug>-$(date +%s) HEAD
cd /tmp/nextbeads-<slug>-<ts>
```

This isolates your build artifacts and uncommitted edits from other agents. Conflicts
resolve at merge / rebase time rather than during editing.

### Pick non-overlapping work — don't wait on `src/main.rs`

The solver-11 monolith file (`solver/11-kissat-port/src/main.rs`, ~13K lines) is the
contention hotspot. **Do not block waiting for it to free up.** Instead, pick a bead
whose change surface does not overlap with any open / `in_progress` bead.

Use the pre-claim check to enumerate what's already taken:

```bash
bd list --status in_progress
bd list --label solver11 --status=open      # plus any in_progress
```

Then choose a bead that:

- Touches a *different subsystem keyword* than the in-flight ones (e.g. if `shrink`
  / `clause-minimization` is in flight, prefer `propagation`, `restart`, `reduce-db`,
  `vmtf`, `bve`, `rephase`, `chrono`, `lucky`, etc.).
- Edits a *different function or hot-path region* in `main.rs`. Two agents can both
  edit `main.rs` if they touch disjoint line ranges — read each in-flight bead's
  "Fix sketch" / file:line citations to predict the overlap before claiming.
- Or lives in a file other than `main.rs` entirely (`src/config.rs`, `src/simp.rs`,
  `src/stats.rs`, `src/branch.rs`, `tools/*`, `docs/*`, `log/*`, `README.md`,
  `FEATURES.md`, etc.).
- Independent work — benchmarks, docs, `FINDINGS.md` writeups, reference reads,
  `bd remember` updates — always runs safely in parallel.

If every ready bead overlaps with current in-flight work, prefer the *least
overlapping* one and accept the merge cost rather than stalling. Surface the
overlap risk to the user in your run summary.

The long-term mitigation is the **Stage B module split** (`src/arena.rs`,
`src/trail.rs`, `src/watch.rs`, `src/proof.rs`, `src/model.rs`, `src/branch.rs`,
`src/search.rs`, `src/inprocess.rs`) outlined in `SOLVER11_STATE.md`. Until those
land, expect occasional `main.rs` merge work even with good bead-picking.

### Single-file caveat — what file claims can't fix

Even if Beads supported file-level claims (it doesn't), they would not help here:
two beads addressing unrelated features (e.g. shrink and propagation) still both
end up patching `main.rs`. The defenses are claim discipline + worktrees + careful
non-overlapping bead selection, not file metadata.

### Pre-commit rebase + re-test

Other agents may have pushed in the time between your claim and your commit.
**Before every commit, sync with the remote and re-verify the change still works.**

```bash
# 1. Fetch the latest main without merging yet.
git fetch origin main

# 2. If origin/main is ahead of HEAD, rebase your in-progress branch on top.
if [ "$(git rev-list --count HEAD..origin/main)" -gt 0 ]; then
    git rebase origin/main           # resolve conflicts as needed
fi

# 3. Re-run the validation suite. Hooks may pass but logic can still break
#    when someone else's change touches a related code path.
cd solver/NN-name && bash build.sh
bash tools/smoke_test.sh solver/NN-name
# For solver-changing beads, also re-run the focused benchmark / cargo test you
# used to validate the bead originally.

# 4. Only commit + push if smoke + targeted tests still pass.
```

If the rebase produced conflicts you cannot resolve confidently, **stop and tell
the user**. Do not paper over conflicts with `git checkout --theirs` / `--ours`.
If the smoke or targeted tests fail after a clean rebase, the other agent's
change interacted with yours — investigate the interaction before committing.

## Invocation

```
/nextbeads [N] [phaseLabel]
```

- `N` — hard cap on how many beads to complete in this run. **Default: 5.** Stop after N even if more are ready.
- `phaseLabel` — which Beads phase label to scope to (e.g. `phase1`, `phase2`). **Default: `phase1`.** The skill will refuse to work beads outside this label.

Examples:
- `/nextbeads` → up to 5 beads in `phase1`
- `/nextbeads 3` → up to 3 beads in `phase1`
- `/nextbeads 5 phase2` → up to 5 beads in `phase2`

## Preamble — Re-read context before every run

**A prior compaction may have evicted the project rules. Do not skip this — re-read every file before touching beads.** Compaction is silent; the only safe assumption is that none of the project conventions are in your active context.

Read in full, in this order:

1. `CLAUDE.md` — the project guide. The sections under *Development Rules*, *Code-Level Optimization Workflow*, *Debugging Optimization Regressions*, *Investigating Why Ported Features Don't Help*, *Solver Interface Contract*, *Status Reporting*, and *Beads Issue Tracker* are load-bearing for this skill. The **Solver 11 default/fast promotion gate**, **UNKNOWN as failure**, **SAT/UNSAT/UNKNOWN result errors are correctness failures**, and **smoke-test-before-commit** rules are non-negotiable.
2. `.agents/skills/beads/SKILL.md` — the Beads workflow (`bd ready`, `bd show`, `bd update --claim`, `bd close`, `bd note`, `bd link`, `bd remember`). Use `bd`, not the `bv` TUI, for mutations. Use `bv --robot-*` for read-only triage.
3. The target solver's `README.md` — find the active iteration with `ls solver/` and pick the latest (`solver/NN-name/README.md`). Also read `FEATURES.md` / `FEATURES.csv` and `SOLVER11_STATE.md` if they exist.
4. Any `log/<investigation>/FINDINGS.md` or `DEEPER_FINDINGS.md` referenced by the beads you are about to work — these are usually how the bead's *why* is documented.

Do not paraphrase from memory. Open each file with the Read tool.

## Pre-flight

Run all of these before claiming any bead.

1. **Refresh bead context**
   ```bash
   bd prime
   bd export -o .beads/beads.jsonl
   ```

2. **Identify the active phase scope.** Default is `phase1` unless the user passed another label. Confirm beads with that label exist:
   ```bash
   bd list --label <phaseLabel> --status=open
   bd list --label <phaseLabel> --status=in_progress
   ```
   If there is no open or ready work for the label, stop and tell the user — do not silently fall through to another phase.

3. **Analyze the bead graph for the phase.** Use `bv` robot commands only (the bare `bv` TUI is interactive and not usable here):
   ```bash
   bv --robot-triage
   bv --robot-plan
   bv --robot-insights
   ```
   Restrict reasoning to beads carrying `<phaseLabel>`. Discard any rankings that pull in cross-phase work.

4. **Build the work order.** Pick the highest-value, currently-unblocked bead inside `<phaseLabel>`, optimizing for:
   - dependencies (blocked beads cannot run)
   - explicit blockers from `bd show`
   - priority field
   - risk (prefer reversible / well-scoped changes when priority is tied)
   - implementation leverage (one bead fixing the root cause of several)
   - downstream unblock count (use `bv --robot-insights` critical-path data)

   Record this ordered list. You will work through it top-down, stopping at N beads or at the last unblocked bead in the phase, whichever comes first.

5. **Baseline profile-bench (before).** Build the active solver and run the profiling suite once to anchor the before/after comparison:
   ```bash
   cd solver/NN-name && bash build.sh && cd -
   bash tools/bench.sh -d benchmarks/profiling solver/NN-name
   ```
   Capture the resulting `log/bench-*/results.csv` path. This is the "before" baseline for the whole run. Do not re-baseline between beads — the comparison is run-level, not per-bead.

## Per-bead loop

For each bead in the work order, up to N times:

1. **Re-confirm scope.** `bd show <id>` and verify the bead still carries `<phaseLabel>` and is still unblocked. If it is no longer in scope, drop it and move to the next.

2. **Claim it.**
   ```bash
   bd update <id> --claim
   ```
   Add a starting note that links to the work order:
   ```bash
   bd note <id> "Claimed via /nextbeads. Work order position M of K."
   ```

3. **Plan the implementation.** From the bead description, the linked FINDINGS docs, and the source files involved, decide what to change. State it in one or two sentences before touching code.

4. **Implement.** Use red-green TDD where the bead is testable: add or update the failing test first, then implement until it passes. Otherwise, make the minimum focused change the bead describes.

5. **Keep the bead updated as you work.** Use `bd note <id> "..."` for material progress points, decisions, or surprises. Do not narrate every line of code — note things a future agent would need to resume.

6. **Switch to fresh-eyes review mode.** Read every file you added or modified in this bead end-to-end, as if you had not written it. Look specifically for:
   - bugs and off-by-ones
   - wrong status reporting (SAT/UNSAT/UNKNOWN correctness — this is a correctness failure per CLAUDE.md)
   - edge cases (empty clauses, unit clauses, max-line-length on `v` lines)
   - confusing logic or surprising control flow
   - broken assumptions about caller invariants
   - integration problems with adjacent modules
   - reliability issues (panics, unwraps in hot paths, allocation in tight loops)
   - anything inconsistent with the bead's acceptance criteria

   Fix every finding before continuing. Use your strongest reasoning mode for this pass.

7. **Run the required checks.** All must pass before closing:
   ```bash
   cd solver/NN-name && cargo test && cd -
   bash tools/smoke_test.sh solver/NN-name
   ```
   If the bead touches the solver hot path, also run a fast sanity check on one profiling instance with the same env vars the bead targets.

8. **Update the bead with what you actually changed.**
   ```bash
   bd note <id> "Implemented: <summary>. Reviewed and fixed: <findings>. Tests: cargo test ✓, smoke ✓."
   ```

9. **Close the bead.**
   ```bash
   bd close <id> --reason="Completed via /nextbeads"
   bd export -o .beads/beads.jsonl
   ```

10. **Commit and push** (per-bead, per the user's preference):
    ```bash
    git add -A
    git commit -m "$(cat <<'EOF'
    <bead-id>: <short title>

    <one-paragraph what+why pulled from the bead and your implementation>

    Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
    EOF
    )"
    git push
    ```
    Stage only the files you actually changed when feasible; avoid `git add -A` if there are untracked files unrelated to the bead.

## Failure handling — stop the loop

A bead is considered failed if any of the following happens:
- `cargo test` fails after the implementation and you cannot fix it inside the bead's scope
- `tools/smoke_test.sh` fails on any of the 8 instances
- the bead's hot-path config produces `UNKNOWN` on a row the baseline solves (this is a correctness failure, never silently quarantined)
- the SAT/UNSAT result, model, or proof is wrong on any test instance

On failure:

1. **Stop the loop immediately.** Do not move to the next bead.
2. **Revert the in-progress changes** so the working tree matches the last clean commit:
   ```bash
   git restore --staged --worktree -- <files-changed-in-this-bead>
   ```
   Or, if changes are spread widely, hard-reset only files this bead touched. Never `git reset --hard` blindly — confirm scope first.
3. **Re-open the bead** with a detailed failure note:
   ```bash
   bd update <id> --status=open
   bd note <id> "Reverted by /nextbeads. Failure: <exact failing command and output>. Hypothesis: <one sentence>."
   ```
4. **Run the after-bench step below anyway** so the user sees the current state.
5. **Report and exit.** Do not silently continue.

## Phase guard — never cross phases

Stop the loop when any of these is true:
- You have completed N beads.
- There are no remaining unblocked beads carrying `<phaseLabel>`.
- The only remaining ready beads carry a different phase label.

**Do not pick up `phase2` work because `phase1` ran dry.** That is the user's call, not the skill's.

## Post-loop — after-bench and report

Run these once, after the last bead (or after a failure):

1. **Rebuild and run the profiling-suite benchmark (after).**
   ```bash
   cd solver/NN-name && bash build.sh && cd -
   bash tools/bench.sh -d benchmarks/profiling solver/NN-name
   ```
   Note the new `log/bench-*/results.csv` path.

2. **Compare before vs after.** Build a small table from the two `results.csv` files showing, per instance:
   - status before / after (SAT / UNSAT / TIMEOUT / UNKNOWN / ERROR)
   - wall time before / after
   - delta and percent change
   - any new ERROR / wrong-result / premature-UNKNOWN rows (these are correctness bugs — hard fails per CLAUDE.md, regardless of PAR-2). A new honest TIMEOUT or budget-consuming UNKNOWN is *not* a failure on its own — it is a priced-in PAR-2 cost.

3. **Analyze the result.** The verdict is **aggregate PAR-2 over the suite**, not per-instance rows. Call out:
   - aggregate PAR-2 before vs after — this is the decision metric; an improvement beyond the noise floor is a win even if some rows regressed or newly timed out
   - which rows regressed / improved (diagnostic detail explaining the aggregate move, not a verdict)
   - any new ERROR, wrong result, or premature `UNKNOWN` (returns without using its budget) — these block regardless of PAR-2
   - the relevant config / env-var combo for the changes you made (so the user can re-run)

## Final report — required format

End the run with a single message containing:

1. **Beads worked on** — a table:

   | Order | Bead ID | Title | Status |
   |-------|---------|-------|--------|
   | 1     | sat-123 | …     | closed |

2. **What was implemented** — for each bead, a paragraph or two covering: the problem it addressed, the change at file:function granularity, and any non-obvious decision. Include a minimal *example* of how it works when applicable (a short command, a code snippet, a CNF trace, a stats diff — whatever demonstrates the change).

3. **Profile-bench before/after** — the comparison table from step 2 of post-loop, plus a one-paragraph analysis of the delta. State whether the changes improve or regress **aggregate PAR-2** over the suite (the decision metric); per-instance regressions/new timeouts are acceptable when the aggregate wins. If a new ERROR, wrong result, or premature non-budget `UNKNOWN` appeared, flag it explicitly as a correctness failure per CLAUDE.md (these block regardless of PAR-2).

4. **Next logical beads** — the top 3 highest-value unblocked beads remaining inside `<phaseLabel>`, ranked by the same criteria from pre-flight step 4 (dependencies, blockers, priority, risk, implementation leverage, downstream unblock count). For each, give:
   - bead ID and title
   - one-sentence reason it is the next logical pick
   - what it would unblock if completed

   If `<phaseLabel>` has no remaining ready work, say so and name the *first* `phase(N+1)` bead that becomes the natural follow-up — but do not start working it.

## Notes and guardrails

- **Use `bd`, not `bv`, for any mutation.** `bv` is a read-only sidecar; use only its `--robot-*` commands for triage.
- **Re-run `bd export -o .beads/beads.jsonl` after every bead close** so `bv` sees current state.
- **Never modify `tools/smoke_test.sh`.**
- **Never use `--no-verify` or `--no-gpg-sign`** when committing.
- **Never force-push.** The per-bead commits go to the current branch and are pushed with a plain `git push`.
- **Convert relative dates in bead notes to absolute** (`Thursday` → `2026-05-28`) so notes remain interpretable later.
- **One bead at a time.** Do not interleave implementations across beads — the fresh-eyes review and the smoke run must apply to a single bead's diff.
- **If `bd prime` reports no workspace,** stop and tell the user. Do not invent beads or fall back to TODO files.
