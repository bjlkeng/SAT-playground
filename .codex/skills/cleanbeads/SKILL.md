---
name: cleanbeads
description: Audit the beads tracker for stale or incorrectly-marked work — in_progress claims no agent is actually working, beads stuck in_progress that should be blocked, beads stuck blocked whose dependencies have closed, deferred beads whose defer date has passed, and ready-queue beads with an assignee set but never claimed. Cross-references active processes, git worktrees, and the agent-mail coord thread to tell the difference between "actively running" and "stale." Asks per-bead before applying any state change. Default staleness threshold is 48 hours; pass an arg like `/cleanbeads 24h` or `/cleanbeads 7d` to override.
---

# /cleanbeads — Audit beads for stale or incorrectly-marked state

## What this does

Bead state drifts. Agents crash, sessions get killed, claims get forgotten, dependencies close without anyone reopening dependents. This skill runs a five-check audit and proposes — but does not silently apply — corrections.

Checks performed:

1. **Stale `in_progress`** — claimed beads whose `Updated:` timestamp is older than the threshold (default 48h) AND that have no live evidence of active work (no matching process, no matching worktree, no fresh coord-thread message).
2. **`in_progress` that should be `blocked`** — beads marked `in_progress` whose `DEPENDS ON` list contains an open or in_progress bead. The dependency means no work can land; status should reflect that.
3. **`blocked` that should be `open`** — beads marked `blocked` whose declared blockers are all closed/superseded. Free them to the ready queue.
4. **`deferred` past defer date** — beads with a `defer` date in the past. Surface for re-triage.
5. **Owner-set but never claimed** — open beads where `Assignee:` is set but status is still `open` (not `in_progress`). Half-assigned beads that nobody is actually working.

For each finding the skill asks the user per-bead what to do, then applies the change with `bd update` and re-exports `bd export -o .beads/beads.jsonl`.

## When to invoke

Run this when:
- The in_progress list looks too long relative to running agents.
- Another agent says "the ready queue is empty" but you know there's claimable work.
- After a crash, OOM, or forced restart where claims may have been orphaned.
- Periodically — once a week is usually enough on a single-user repo, more on multi-agent days.

Do **not** run this in the middle of an active `/nextbeads` or `/analyzesat` run that you launched in this session — your own work will look stale to itself.

## Procedure

### Step 0 — parse the optional threshold arg

Accept arguments like `24h`, `48h`, `7d`, `2d`. Default: `48h`. Convert to a seconds-ago cutoff for comparison with bead `Updated:` timestamps.

If the user passes no arg, use 48h and say so in the opening status line.

### Step 1 — gather live-work signals

Run these in parallel:

```bash
bd list --status in_progress
bd list --status blocked
bd list --status deferred
bd list --status open --json     # for owner-set-but-not-claimed scan
```

```bash
# Active processes — what's actually running right now
ps aux | grep -E 'sat-solver|cargo|bench\.sh|claude' | grep -v grep
```

```bash
# Active worktrees — who has a branch checked out
git worktree list
ls -lat /tmp/sat-worktrees/ 2>/dev/null
```

```bash
# Coord thread — recent agent announcements (project-specific to SAT-playground)
# Uses MCP Agent Mail; safe to call even if empty.
```

Then call `mcp__mcp_agent_mail__fetch_topic` with `project_key="/home/bojji/code/SAT-playground"`, `topic_name="coord"`, `limit=20` to see any recent claim announcements.

### Step 2 — for each in_progress bead, classify

For every bead returned by `bd list --status in_progress`:

```bash
bd show <id>
```

Extract: `Updated:`, `Assignee:`, `DEPENDS ON:`, and the most recent `NOTES` entry.

Classify into one of:

- **Active**: matches a running process command line, worktree branch name (e.g. `agent/<owner>/<bead-id-suffix>`), or coord-thread message newer than the staleness threshold. Leave alone.
- **Should-be-blocked**: `DEPENDS ON` has at least one entry that is not closed. Propose status change to `blocked`.
- **Stale**: `Updated:` older than threshold AND no live-work match AND no open dependency. Propose release to `open`.
- **Recent**: `Updated:` within threshold and no live-work signal — ambiguous. Don't propose anything; mention in the report so the user can decide.

### Step 3 — for each blocked bead, check the blocker graph

For every bead from `bd list --status blocked`:

```bash
bd show <id>
```

Look at `DEPENDS ON`. If **all** listed dependencies are `✓ closed`, propose status change to `open` with a note pointing to the closed blockers.

### Step 4 — for each deferred bead, check the defer date

```bash
bd list --status deferred --json | jq '.[] | {id, defer_until, title}'
```

(Or parse `bd show <id>` if `--json` doesn't include defer-until.) For any bead whose defer date is in the past, propose either reopening (`--status open`) or extending the defer date — leave the decision to the user.

### Step 5 — for owner-set-but-not-claimed

From `bd list --status open --json`, find beads with `assignee` set but `status == "open"`. Propose either claiming (`--status in_progress`) or clearing the assignee (`--assignee ""`).

### Step 6 — present findings, ask per-bead

Print a short summary table grouped by check:

```
Stale in_progress (>48h, no active signal):
  - SAT-playground-xxx [P1] · last 2026-05-25 · "<title>"
  - ...

Should be blocked (depends on non-closed):
  - SAT-playground-yyy [P0] · depends on SAT-playground-zzz (in_progress) · "<title>"

Should be unblocked (all blockers closed):
  - ...

Deferred past defer date:
  - ...

Owner set but not claimed:
  - ...
```

Then use `AskUserQuestion` to ask per-bead. Group questions so a single `AskUserQuestion` call covers at most 4 beads (the tool's max); call it repeatedly if there are more.

For **stale in_progress**, the three options are:
- "Release back to open" — `bd update <id> --status open --append-notes "Released claim YYYY-MM-DD: <reason>. <handoff note>"`
- "Keep claimed, add 'paused' note" — `bd update <id> --append-notes "Paused YYYY-MM-DD: <reason>. <checkpoint>"`
- "Leave as-is"

For **should-be-blocked**, two options:
- "Mark blocked (Recommended)"
- "Leave in_progress"

For **should-be-unblocked**, two options:
- "Reopen to open (Recommended)"
- "Leave blocked" (rare — only if there's an undeclared blocker)

For **deferred past date**, two options:
- "Reopen to open" — `bd update <id> --status open --defer ""`
- "Extend defer date" — ask the user for a new date and apply `--defer <date>`

For **owner-set-but-not-claimed**, two options:
- "Claim it (mark in_progress)" — `bd update <id> --claim`
- "Clear assignee" — `bd update <id> --assignee ""`

### Step 7 — apply and re-export

After each batch of decisions, run the `bd update` commands the user approved.

When all decisions are applied:

```bash
bd export -o .beads/beads.jsonl
```

Then print a one-paragraph summary of what changed: how many released, how many relabeled, how many left alone, and the new ready-queue count (`bd ready | wc -l`).

## Multi-agent safety

This skill **modifies bead state**, which is shared across all agents. Before applying any change, the calling agent must:

- Not be running inside another `/nextbeads` or `/analyzesat` session itself.
- Confirm that any in_progress bead it proposes to release does not match an active process or worktree (Step 2 already does this — don't skip it).
- Not touch the `5b2.2.36`-style entries that match a *fresh* `/nextbeads` worktree just created in the last hour. The pre-bead profiling bench can take 30+ minutes; an agent that just claimed a bead and started profiling looks "stale" by raw timestamp but isn't.

If you cannot positively classify a bead as stale, propose "Leave as-is" or skip it from the findings entirely. **It is always safe to leave a claim alone; it is sometimes destructive to release one.**

## Output discipline

- Open with one line stating the threshold being used and how many beads were inspected: `Auditing 4 in_progress + 12 blocked + 2 deferred + 38 open with threshold 48h.`
- Group findings by check (use the table format above).
- Don't restate bead descriptions in full — title + ID + one-line reason is enough.
- After the AskUserQuestion round, print only what changed and the new ready-queue count. Skip per-bead "applied" confirmations — the bd output already shows them.

## What this skill is NOT

- It does **not** create or close beads. Use `bd create` / `bd close` for that.
- It does **not** auto-claim ready beads for you. Use `/nextbeads` for that.
- It does **not** edit bead descriptions, acceptance criteria, or labels. Only status + notes + assignee + defer date.
- It does **not** push or commit. The only artifact it writes outside `bd` is the re-exported `.beads/beads.jsonl`, which the user commits when they want to.
