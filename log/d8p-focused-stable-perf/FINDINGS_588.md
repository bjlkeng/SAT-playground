# 588 — Root-cause of the focused/stable trajectory pathology — 2026-05-29

**Bead:** `SAT-playground-588` (P1), follow-up to `d8p`. **Agent:** slate-heron via /nextbeads.
Builds on `log/d8p-focused-stable-perf/FINDINGS.md` (d8p showed the regression is
search-trajectory, not per-event overhead). 588 root-causes *which* trajectory effect.

## Executive summary

- The focused/stable regression is **search-trajectory / clause-quality**, and it is
  **instance-dependent** — there is no single universal pathology and no single-knob fix.
- **Largest single fixable contributor: focused-mode VMTF causes pathological deep
  decision dives.** On Sudoku, focused/VMTF runs at avg decision level **31,493**
  (max 307,092); the identical config with **`SAT_VMTF=off` (VSIDS) drops it to avg
  343 (max 2,321)** — a 90× collapse. The 1.16 matrix corroborates: focused/VSIDS
  2.51M props/s vs focused/VMTF 1.22M.
- **Residual beyond VMTF:** even focused/VSIDS loses to default on Sudoku (176.8s vs
  146.7s @200k conflicts) with avg LBD 39 — a **clause-quality** problem (high glue),
  which is Phase-2/inprocessing territory (vivification), matching the `5b2.2.30`
  "search-trajectory/inprocessing-dependent" verdict.
- `max_decision_level` is a real stat (main.rs:4448 = max of `current_level()+1` per
  `decide`), not a bug.

## Evidence — deterministic decision-level + DB-quality stats (equal conflict budget)

`SAT_LIMIT_CONFLICTS`, `SAT_STATS_JSON=on`. Decision-level / conflict / LBD counters
are deterministic (contention-independent); wall/props-s are timing.

| instance / config | conflicts | avg level | focused avg | max level | avg LBD | wall | props/s |
|---|---|---|---|---|---|---|---|
| sudoku default (200k) | 200,001 | 236 | — | 804 | n/a | 146.7s | 7.60M |
| sudoku focused/VMTF (200k) | 200,001 | **31,493** | 31,493 | **307,092** | 27 | 195.7s | 6.63M |
| sudoku focused/VSIDS (200k) | 200,001 | **343** | 343 | **2,321** | 39 | 176.8s | — |
| kakuro default (100k) | 100,001 | 339 | — | 13,043 | n/a | 21.0s | 4.0M |
| kakuro focused/VMTF (100k) | 35,219 (**SAT**) | 401 | 315 | 6,921 | 8.2 | 12.5s | 2.6M |
| 6s299 default (100k) | 3,764 (SAT) | **26,094** | — | 61,630 | n/a | 5.7s | 4.96M |
| 6s299 focused/VMTF (100k) | 5,097 (SAT) | 19,748 | 3,954 | 68,172 | 9.5 | 82.6s | **0.33M** |

Reading:

1. **Sudoku** — focused/VMTF dives 130× deeper than default (avg 31,493 vs 236).
   `SAT_VMTF=off` collapses it to 343 → **VMTF is the deep-dive cause** here.
2. **Kakuro** — focused/VMTF is *fine* and **wins** (SAT at 35,219 conflicts, 12.5s;
   default still UNKNOWN at 100k/21s). No pathology.
3. **6s299** — **default itself** runs at avg level 26,094 (deep is instance-structural,
   not focused-specific), yet focused/VMTF is **15× slower props/s** (0.33M vs 4.96M) at
   similar depth — a *different* focused cost on this instance (clause-DB/propagation
   shape, not depth).

So the regression is **diffuse and instance-dependent**: VMTF deep-dive (Sudoku),
slow high-LBD propagation (6s299), or nothing (Kakuro). Confirms d8p: not code
overhead, and confirms it is not a single-knob policy bug.

## Why the restart policy can't rescue the deep dive (mechanism)

All restart checks (`note_kissat_ema_conflict`, `note_reluctant_conflict`, main.rs:4866/4813)
fire **per conflict**, and the EMA restart condition is **relative**
(`restart_fast_lbd > restart_slow_lbd * margin`, main.rs:4844). When focused mode
uniformly produces high-LBD clauses, both EMAs are uniformly high, so the condition
rarely trips (restart ~every 292 conflicts on Sudoku, not Glucose-aggressive ~50),
and conflict-driven restarts cannot interrupt a conflict-free dive at all. With VMTF
driving deep dives, the relative EMA never sees "worse than usual" → no rescue.

## Disposition / recommendations

1. **Concrete fix bead (opened):** root-cause and fix the focused-mode **VMTF
   deep-dive** — the largest single fixable contributor (Sudoku avg 31,493 → 343 with
   VSIDS). Likely a missing kissat-style VMTF/restart/phase interplay or a queue/guard
   gap in the Rust port. Must be validated on the focused/stable matrix (5b2.2.46
   methodology) + solver-11 promotion gate + shuffle-sensitivity (CLAUDE.md anti-overfit),
   so it is a dedicated bead, not a /nextbeads quick slice. focused/stable is off by
   default, so the default profile bench will NOT show it — validation needs the
   focused matrix.
2. **Residual clause-quality** (high LBD even with VSIDS; 6s299 slow propagation) →
   Phase-2 **vivification/inprocessing** (`SAT_VIVIFY`, parking-lot), per `5b2.2.30`.
3. The 1.14 throughput micro-opts (`18.x`) remain irrelevant to this (d8p).

## Artifacts

- This doc; `log/d8p-focused-stable-perf/FINDINGS.md` (d8p);
  `conf_{kakuro,6s299}_{default,fvmtf}.txt`, `conf_sudoku_fvsids.txt`,
  `json_{default,fvmtf}_200k.txt`.
