# FINDINGS — SAT-playground-0s0: single-mode restart trail-reuse (Gap CD-4)

**Agent:** VioletRidge (`/nextbeads 1`, phase1)
**Date:** 2026-05-29
**Base:** `origin/main` @ `2f466ac`
**Worktree:** `/tmp/sat-worktrees/VioletRidge-1780072366`
**Verdict:** EVALUATED — regresses aggregate PAR-2; **reverted** (nothing merged). CD-4 is a
correct deliberate design, not a fixable gap.

## What was implemented

`restart_reuse_trail_level` (main.rs) short-circuited to `0` for any non-`FocusedStable`
policy, so the **default single-mode profile never reused the trail** — every restart was a
full `backtrack(0)`, re-deriving lower-level propagations (sudoku ~617 restarts, REGRandom
~3069, mp1 ~1020). The bead (Gap CD-4) proposed making single-mode honour
`SAT_RESTART_REUSE_TRAIL=on` via the VSIDS activity cutoff (kissat `reuse_stable_trail`),
since single-mode branches by the same VSIDS heap as stable mode.

Change (default-off, opt-in):
- New `Solver.restart_reuse_trail_single` (from `config.restart_reuse_trail`).
- `restart_reuse_trail_level` single-mode branch: `if !restart_reuse_trail_single { 0 } else
  { reuse_stable_trail_level(current_level) }`.
- Updated the old `test_trail_reuse_stable_does_not_apply_to_single_mode_luby_path` into two
  tests: default-off keeps full-backtrack(0); opt-in reuses (level 2 for the high-activity
  prefix).

Validation: `cargo test` 433 PASS, smoke 9/9. Mechanism confirmed active and correct on
brocard: `reused_trails` 0→3, `reused_levels` 0→9, both runs UNSAT-correct (403→622
conflicts — trajectory-affecting, as expected).

## Aggregate-PAR-2 A/B (profiling suite, 300s, same binary, flag-off vs `SAT_RESTART_REUSE_TRAIL=on`)

`flag-off` = `log/nb-0s0-off` (complete), `flag-on` = `log/nb-0s0-on` (stopped at 4/10 per
stop-losers-early).

| instance | flag-off | flag-on | ΔPAR-2 |
|---|---|---|---|
| sudoku (UNSAT) | 202.5 | 212.8 | +10.3 |
| 6s299 (SAT) | 17.2 | 16.4 | −0.8 |
| REGRandom (UNSAT) | 59.0 | 47.4 | **−11.5** (high-restart UNSAT benefits) |
| **mp1 (SAT)** | **45.2** | **TIMEOUT 600** | **+554.8** |
| Kakuro / SCPC / velev / brocard / battleship / case9 | (sum 462.6) | not measured | run stopped |
| **flag-off total** | **786.5 (10/10)** | — | — |
| 4-row subtotal | 323.9 | 876.6 | **+552.8** |

**Conclusive reject without finishing:** mp1's **+554.8** alone exceeds the **462.6** total
PAR-2 of all six unmeasured rows, so even if every remaining instance dropped to 0s under
flag-on the aggregate could not reach parity. Realistically the unmeasured SAT instances
(Kakuro/case9/battleship) would regress like mp1. Per the skill's "stop losers early when
safe," the flag-on run was halted at 4/10.

- **mp1 SAT→TIMEOUT** is the dominant term — the same trajectory-fragile instance that broke
  `1oo` and that `4a3` documented (its SAT solution depends on a fragile trajectory). Trail
  reuse preserves a high-activity prefix across restarts that derails mp1's path. This
  **confirms the README's documented reason** single-mode trail-reuse is deliberately off:
  *"stable-mode reuse is deliberately not applied to the solver-10-compatible single-mode
  Luby path because that hybrid preserved a harmful SAT prefix on mp1."*
- **REGRandom improved** (−11.5) — high-restart UNSAT instances do benefit from avoiding the
  full re-propagation, exactly as the bead predicted. But it is far from enough to offset mp1.
- **No correctness failures** — mp1 is an honest 300s budget-consuming TIMEOUT (exit 124), not
  a wrong result or premature non-budget UNKNOWN.

## Disposition

CD-4 is a **correct deliberate design**, not a bug: single-mode trail-reuse regresses
aggregate PAR-2, so the default correctly stays full-backtrack(0). Bead closed; code reverted
(no source merged); `bd remember` `solver11-0s0-single-mode-trail-reuse-2026-05-29`. mp1
trajectory fragility is the recurring blocker for default trajectory changes (`1oo`, `0s0`) —
the durable lever is Phase-2 clause-quality (inprocessing), not Phase-1 restart/propagation
tweaks.
