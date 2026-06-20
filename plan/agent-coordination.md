# Agent Coordination Reference

Routine bead work should use the Beads and `/nextbeads` skills. This file keeps
optional Agent Mail details out of `CLAUDE.md` while preserving the shared-checkout
coordination model.

## Shared Checkout Model

Agents work in `/home/bojji/code/SAT-playground` on `main` directly. Routine bead
work does not use worktrees or per-agent feature branches. Because the checkout is
shared, coordination relies on:

- Beads claims;
- `git status --short` before editing;
- process checks for live solver/bench work;
- optional Agent Mail messages on the shared `coord` thread;
- pull-rebase and validation before push.

Before editing a file another active agent appears to be touching, stop and ask
the user. Surface the evidence: bead id, file, process, uncommitted edit, or coord
message.

## Agent Mail Workflow

If the Agent Mail MCP tools are available:

1. Register in project `/home/bojji/code/SAT-playground`.
2. Read the shared `coord` thread before claiming work.
3. Claim a bead with `bd update <id> --claim`.
4. Announce intent:

   ```text
   subject: claim <id>
   body: bead <id>: <scope>; touching <files/functions>
   ```

5. Before commit, announce files, regions, summary, validation, and a short
   objection window.
6. Pull-rebase, re-run validation, then push.
7. Close the bead and send a final pushed-at-sha message.

Do not use exclusive file reservations on `solver/**/src/main.rs`; that hot file
is too broad for useful exclusive locking. Reservations are reasonable for
less-contended paths like `docs/**`, `tools/**`, `tests/cnf/**`, and benchmark
fixtures.

## Explicit Token Workaround

If a reused Agent Mail identity reports `requires registration_token` after
registration, the connector may not be preserving session binding. Read the local
token from `/home/bojji/code/mcp_agent_mail/storage.sqlite3` and pass it as
`registration_token` or `sender_token` on Agent Mail calls. Do not paste tokens
into chat, commits, Beads notes, or agent instruction files.

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
- Forgetting to release beads on failure or early exit; `/nextbeads` now requires
  all claimed beads to be closed or reopened and unassigned before exit.
