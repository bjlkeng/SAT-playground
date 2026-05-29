# ibq — kissat-faithful focused restart: REJECTED — 2026-05-29

**Bead:** `SAT-playground-ibq` (P1). **Agent:** slate-heron via /nextbeads.
Attempted fix for the focused-mode VMTF deep-dive (`588`). **Result: rejected** —
reverted, no code change kept.

## Hypothesis

`588` root-caused the deep dive partly to a kissat-faithful divergence: the Rust
focused EMA restart uses base `KISSAT_EMA_RESTART_MIN_CONFLICTS=50` + margin `1.20`,
vs kissat `restartint=1` (`RESTARTINT_DEFAULT`) + `restartmargin=10` (margin 1.10),
checked every conflict. Hypothesis: making it kissat-faithful (1 / 1.10) fixes the
deep dive and improves focused/stable.

## Experiment + full focused-stable matrix A/B (profiling suite, 300s)

Set the two consts to 1 / 1.10, rebuilt, and ran the **full focused-stable matrix**
(`SAT_SEARCH_MODE=focused-stable SAT_USE_LBD=on SAT_MODE_USE_TICKS=on
SAT_REDUCE_POLICY=lbd-tiered`) for orig (50/1.20) vs KF (1/1.10):

| | solved | PAR-2 |
|---|---|---|
| single-mode **default** (context) | 10/10 | **750.260** |
| focused/stable **orig** (50/1.20) | 10/10 | **991.750** |
| focused/stable **KF** (1/1.10) | **9/10** | **1326.824** |

Per-instance (orig → KF):

| instance | orig | KF | |
|---|---|---|---|
| sudoku | UNSAT 287.9s | UNSAT 222.4s | ✅ −66s |
| 6s299 | SAT 87.3s | SAT 76.6s | ✅ −11s |
| REGRandom | UNSAT 259.7s | UNSAT 92.1s | ✅ −168s |
| **mp1** | **SAT 0.98s** | **TIMEOUT 300s** | ❌ **NEW TIMEOUT** |
| battleship | SAT 3.0s | SAT 76.7s | ❌ +74s |
| velev | SAT 55.3s | SAT 86.3s | ❌ +31s |
| Kakuro | SAT 54.4s | SAT 67.8s | ❌ +13s |
| SCPC | UNSAT 28.7s | UNSAT 33.6s | ❌ +5s |
| brocard | UNSAT 15.1s | UNSAT 22.2s | ❌ +7s |
| case9 | SAT (orig 10/10) | SAT 49.2s | ~ |

## Verdict: REJECTED

- **New timeout on mp1** (orig solves in 0.98s → KF times out at 300s): a CLAUDE.md
  hard-fail (new timeout on a baseline-solved row), regardless of aggregate PAR-2.
- **Net worse**: KF PAR-2 1326.8 (9/10) vs orig 991.8 (10/10), +335 PAR-2.
- The Rust port's conservative base 50 / margin 1.20 is **better-tuned for this
  suite than literal kissat values**. Aggressive restarts are a tradeoff that
  rescues deep-divers (Sudoku/6s299/REGRandom) but wrecks instances that benefit
  from deeper search (mp1, battleship). Reverted; no code change kept.

## Two corrections to the d8p/588 record

1. **Focused/stable does NOT "diverge" on Sudoku.** Orig focused/stable SOLVES
   Sudoku at 287.9s (within the 300s timeout). The earlier "diverge/UNKNOWN" was a
   **conflict-limit artifact** (the 200k `SAT_LIMIT_CONFLICTS` cap cut it off before
   its ~260k-conflict solve). The deep dive makes focused/stable *slow*, not
   divergent (on this build/suite).
2. **The VMTF deep dive is real but not the gating issue for the default goal.**
   Even orig focused/stable (991.8, 10/10) and every focused variant measured
   (focused/VSIDS, 1.16 matrix) **lose to single-mode default (750.3)**. Fixing the
   deep dive alone will not make the focused/stable feature default beat
   single-mode/solver10 — that gap is **Phase-2 clause-quality / inprocessing**
   dependent (`5b2.3.18`), confirming `5b2.2.30` yet again.

## Recommendations

1. **Reject the restart-cadence approach** (done). Do not chase a single global
   restart parameter for focused/stable.
2. **ibq reopened** — if the focused-mode VMTF deep dive is pursued, it needs a
   *surgical* fix (e.g., focused-mode VSIDS, which `588` showed avoids the dive; or
   a depth-triggered restart that fires only on pathological depth without touching
   mp1/battleship) — NOT a global restart change. But see (3).
3. **P0 `5b2.2.53` (default decision):** the evidence across all focused variants
   shows focused/stable loses to single-mode default. **Ship single-mode default
   near-term** (~7% slower than solver10, accepted in `5b2.2.56`); the focused
   feature default is **Phase-2-dependent** (`5b2.3.18` inprocessing/vivification),
   not a Phase-1 restart/deep-dive fix.

## Artifacts

- orig results: `/tmp/sat-worktrees/sh-orig/log/bench-11-kissat-port-2026-05-29-00-38-32/results.csv`
- KF results: `/tmp/sat-worktrees/slate-heron/log/bench-11-kissat-port-2026-05-29-00-38-37/results.csv`
- diagnostic + generalization JSON: `log/d8p-focused-stable-perf/`
