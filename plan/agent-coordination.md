# Agent Coordination Reference

This file keeps optional Agent Mail details out of `CLAUDE.md` while preserving
the shared-checkout coordination model.

## Shared Checkout Model

Agents work in `/home/bojji/code/SAT-playground` on `main` directly — no worktrees
or per-agent feature branches. Because the checkout is shared, coordination relies
on:

- `git status --short` before editing;
- process checks for live solver/bench work;
- optional Agent Mail messages on the shared `coord` thread;
- pull-rebase and validation before push.

Before editing a file another active agent appears to be touching, stop and ask
the user. Surface the evidence: file, process, uncommitted edit, or coord message.

## The one exception: branch `assumptions`

The owner's call, 2026-11-15, for epic `SAT-playground-bp3` only. That epic
changes the heart of the search, so its work happens on branch `assumptions`
in its own worktree:

    /home/bojji/code/SAT-playground-assumptions

The main checkout stays on `main`, and main's solver code is not touched until
the owner merges the branch (bead `bp3.10`). Everyone else keeps working on
`main` as above.

What is different in the worktree (checked 2026-11-15):

- **The tracker.** `bd` run from the worktree uses the main checkout's
  database (`bd where` prints it). A commit on the branch carries no tracker
  export: the pre-commit hook writes `.beads/issues.jsonl` in the *main*
  checkout and stages it there, so the next commit on `main` picks it up. The
  post-checkout and post-merge hooks do not read the worktree's own, older
  copy of that file (tested with a made-up issue in it: it never reached the
  database). The branch never edits `.beads`; at the merge, main's file is
  kept.
- **Local-only files.** Benchmarks, reference solvers and proof checkers are
  not in git, so a fresh worktree has none. `bash tools/worktree_links.sh
  /home/bojji/code/SAT-playground` (on the branch) links them in.
- **Frozen builds** (`solver/13-j..-snapshot`) are untracked and live only in
  the main checkout: name them by full path. Name a new one `13-jN-...`,
  never `13-k-...`, and check `tools/current_solver.sh` afterwards.
- **Runs** follow the same resource cap as everything else: the worktree is on
  the same host.
- **When main's solver code moves**, the branch merges `main`, and its
  baselines are recorded again on the new main build (bead `bp3.1`).

Remove the worktree with `git worktree remove` once the branch is merged or
deleted, and delete this section then.

## Agent Mail Workflow

If the Agent Mail MCP tools are available:

1. Register in project `/home/bojji/code/SAT-playground`.
2. Read the shared `coord` thread before claiming work.
3. Announce intent:

   ```text
   subject: claim <scope>
   body: <scope>; touching <files/functions>
   ```

4. Before commit, announce files, regions, summary, validation, and a short
   objection window.
5. Pull-rebase, re-run validation, then push.
6. Send a final pushed-at-sha message.

Do not use exclusive file reservations on `solver/**/src/main.rs`; that hot file
is too broad for useful exclusive locking. Reservations are reasonable for
less-contended paths like `docs/**`, `tools/**`, `tests/cnf/**`, and benchmark
fixtures.

## Explicit Token Workaround

If a reused Agent Mail identity reports `requires registration_token` after
registration, the connector may not be preserving session binding. Read the local
token from `/home/bojji/code/mcp_agent_mail/storage.sqlite3` and pass it as
`registration_token` or `sender_token` on Agent Mail calls. Do not paste tokens
into chat, commits, or agent instruction files.

```bash
AGENT_MAIL_TOKEN=$(python3 - <<'PY'
import sqlite3
con = sqlite3.connect('/home/bojji/code/mcp_agent_mail/storage.sqlite3')
row = con.execute('''
select a.registration_token
from agents a join projects p on p.id = a.project_id
where p.human_key = ? and lower(a.name) = lower(?)
''', ('/home/bojji/code/SAT-playground', 'agent-name')).fetchone()
print(row[0] if row and row[0] else '')
PY
)
```

## Common Pitfalls

- Treating `src/main.rs` as exclusively reservable.
- Editing a contended file without asking the user.
- Skipping pre-commit announcement when Agent Mail is in use.
- Pushing after rebase without rerunning validation.
- Leaving uncommitted edits in the shared checkout.
