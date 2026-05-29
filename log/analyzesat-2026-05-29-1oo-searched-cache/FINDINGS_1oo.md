# FINDINGS — SAT-playground-1oo: long-clause watcher `searched` cache (Gap CD-2)

**Agent:** VioletRidge (`/nextbeads 1`, phase1)
**Date:** 2026-05-29
**Base:** `origin/main` @ `64cc69c`
**Worktree:** `/tmp/sat-worktrees/VioletRidge-1780068978`
**Verdict:** REJECTED — net aggregate-PAR-2 regression +548.0. Code reverted (nothing merged).

## What was implemented

Kissat's `c->searched` optimization (proplit.h): cache the last successful
watcher-replacement position on the clause and resume the next scan from there
(wrapping `[searched..len)` then `[2..searched)`), to skip re-scanning the
false prefix.

Implementation (learnt clauses only — they always carry activity extra words and
are the long-clause-heavy population the bead targets):

- `CLAUSE_SEARCHED_WORDS = 1`, `CLAUSE_LEARNT_EXTRA_WORDS = CLAUSE_ACTIVITY_WORDS + 1`.
- `clause_header_extra_words` learnt branch: `2 -> 3` (single source of truth; GC
  copies the extra span via `clause_word_len`, so both GC paths auto-handle it).
- `searched` word appended **after** the 2 activity words, so the existing
  activity offsets (`clause_idx + 1 + len`, `+1`) are untouched.
- Accessors `clause_searched` / `set_clause_searched` at `clause_idx + 1 + len + 2`.
- Init `searched = 2` at both learnt-creation sites.
- Propagation scan (`propagate_impl`): for learnt clauses with `len > 3`, resume at
  `searched` (clamped to `[2, len)`, with stale-shrink guard) and wrap; non-learnt /
  short / fresh (`searched == 2`) clauses keep the byte-identical scan-from-2 fast
  path. When no replacement is found, all of `[2, len)` is still scanned (identical
  fall-through to conflict/unit; no missed conflicts, identical tick count).

Validation: `cargo test` 433 unit + 4 config CLI PASS (incl. a new round-trip + GC
preservation test `test_clause_searched_roundtrips_and_survives_gc`); smoke 9/9
(DRAT proofs verified).

## Why it was rejected (the bead premise was wrong)

The bead framed CD-2 as "avoid re-scanning early positions" → a pure speedup. **It is
not a pure speedup.** Resuming the scan from `searched` instead of 2 selects a
*different* non-false literal as the new watched literal, which changes the order in
which clauses are revisited as literals later become false — i.e. it changes
conflict-detection order and therefore the **search trajectory**. The saving comes
*precisely* from picking a different replacement; there is no variant that both skips
the prefix and preserves the watched-literal choice.

## Aggregate-PAR-2 A/B (profiling suite, 300s timeout → unsolved = 600)

before = `log/nb-1oo-before/results.csv`, after = `log/nb-1oo-after/results.csv`.

| instance | before | after | ΔPAR-2 |
|---|---|---|---|
| sudoku (UNSAT) | 193.6 | 199.7 | +6.1 |
| 6s299 (SAT) | 16.6 | 14.2 | −2.4 |
| REGRandom (UNSAT) | 57.3 | 62.8 | +5.5 |
| **mp1 (SAT)** | **43.4** | **TIMEOUT 600** | **+556.6** |
| Kakuro (SAT) | 208.9 | 104.4 | **−104.6** |
| SCPC (UNSAT) | 13.3 | 11.3 | −2.1 |
| velev (SAT) | 67.6 | 90.6 | +23.0 |
| brocard (UNSAT) | 8.7 | 10.8 | +2.2 |
| battleship (SAT) | 22.9 | 76.8 | +53.9 |
| case9 (SAT) | 125.3 | 135.1 | +9.8 |
| **PAR-2** | **757.7** | **1305.7** | **+548.0** |

- The Kakuro win (−104.6) confirms the cache *does* help long-clause-heavy instances
  (the CD-2 prediction). But the trajectory perturbation flips **mp1** SAT→TIMEOUT
  (+556.6) — mp1 is the trajectory-fragile instance from SAT-playground-4a3 (its SAT
  solution depends on a fragile clause-retention/propagation trajectory) — and slows
  battleship (+53.9) and velev (+23.0). Net **+548.0**, a clear reject.
- No correctness failures: every row is a correct SAT/UNSAT except mp1, which is an
  **honest 300s budget-consuming TIMEOUT** (exit 124), not a wrong result or a
  premature non-budget UNKNOWN.
- Concurrent benching (AmberFinch, 1 core) during the after-run is within CLAUDE.md's
  4-core clean-timing threshold (2 cores total); the +548 swing is far beyond any
  contention noise, and the dominant term (mp1 SAT→TIMEOUT) is work-bound, not timing.

## Disposition

`SAT-playground-1oo` closed as rejected. The straightforward kissat port is
net-negative on aggregate PAR-2 and there is no pure-speedup variant. A future retry
would have to avoid perturbing fragile SAT trajectories — likely only via formula/
instance gating (overfit risk per CLAUDE.md's anti-lucky-order rule) or by revisiting
once Phase-2 inprocessing changes clause quality. Recorded via `bd remember`
(`solver11-1oo-searched-cache-2026-05-29`) so future loops do not re-attempt it as a
"free speedup."
