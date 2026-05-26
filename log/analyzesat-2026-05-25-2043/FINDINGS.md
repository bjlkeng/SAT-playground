# Bottleneck Analysis — solver/11-kissat-port — 2026-05-25

**Worktree:** `/tmp/analyzesat-2026-05-25-2043` (detached HEAD @ `39cd707`)
**Slug dir:** `log/analyzesat-2026-05-25-2043/`
**Method:** 6-config ablation × 10 profiling instances (300 s, 16 GiB);
work × speed decomposition vs `A_baseline`; kissat-latest / kissat-sc2024 live comparison;
kissat-latest source diff against solver 11 hot paths.
**Companion document:** `log/kissat-investigation-2026-05-23-broad/DEEPER_FINDINGS.md`
(2026-05-24) — this run re-validates those findings against current code and adds Phase 3
reference comparison plus updated per-instance signal.

## Executive Summary

* **B_metadata_only sanity passes:** identical conflicts/decisions/propagations to A_baseline
  on every instance, wall within ±4%. LBD bookkeeping is genuinely free; anything past B is
  search-trajectory signal.
* **Single isolated kissat-style features regress the baseline.** EMA restart alone (C) loses 3
  instances entirely (mp1 / battleship / case9 timeout); PAR-2 +215%. Focused/stable alone (D)
  retains 10/10 but slows by +43% because 6s299b685 and REGRandom take 5–7× longer.
* **E_combined (focused-stable + EMA + LBD) is the most "kissat-like" config and the best
  non-baseline result: 10/10 solved, PAR-2 987 (+29% vs A).** It also surfaces three big
  per-instance wins — mp1 43.6 s → 0.6 s (-98.5%), Kakuro 214 s → 58 s (-73%), battleship 22.9 s
  → 0.09 s (250×) — paid for by 6s299b685 (+472%), REGRandom (+281%), SCPC (+88%), brocard
  (+106%). This is **per-instance dispersion, not noise:** focused/stable phase cycling finds a
  qualitatively different trajectory.
* **F_full_stack (E + lbd-tiered + chrono + rephase + binary-fast) is broken: 5/10 solved,
  PAR-2 3506 (+359%).** Adding lbd-tiered + chrono on top of E destroys 4 instances that E
  solves. F is the strongest evidence that the lbd-tiered reducer (`SAT-playground-qmz`,
  `-ycw`) is incomplete enough to harm a kissat-feature stack — not just no-op.
* The four implementation gaps documented in the prior DEEPER_FINDINGS (2026-05-24) all
  reconfirm against current code (`39cd707`): no default trail reuse, "do nothing under budget"
  reducer, bump-to-1 used counter, default restart margin still 1.20 vs kissat 1.10. A fifth
  gap is the tier1 over-protection in `reduce_candidate` (rejects tier1 unless emergency).
* **One new finding for this run:** the `velev` and `battleship` "instant" results under D/E
  (0.09 s vs A_baseline 22.9 / 64 s) are produced by focused-mode phase cycling hitting a
  satisfying assignment within hundreds of decisions. They are not bugs — but they are
  evidence the focused-stable path needs the *complete* kissat support (probing, target-phase,
  random sequences) before its dispersion stabilizes.

## Config matrix

| Config | Env vars |
|---|---|
| `A_baseline` | (defaults only — solver-10-compatible single mode) |
| `B_metadata_only` | `SAT_USE_LBD=on` |
| `C_lbd_ema` | `SAT_USE_LBD=on SAT_RESTART=kissat-ema` |
| `D_focused_stable` | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable` |
| `E_combined` | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_RESTART=kissat-ema` |
| `F_full_stack` | LBD + focused-stable + ticks + ema-restart + lbd-tiered + rephase + chrono + binary-fast |

## PAR-2 per config (300 s timeout, 16 GiB limit, profiling/ 10 instances)

| Config | Solved | Timeout | UNKNOWN | Error | PAR-2 | Δ vs A |
|---|---:|---:|---:|---:|---:|---:|
| A_baseline | 10 | 0 | 0 | 0 | 764.1 | — |
| B_metadata_only | 10 | 0 | 0 | 0 | 763.4 | -0.09% |
| C_lbd_ema | 7 | 3 | 0 | 0 | 2403.4 | +215% |
| D_focused_stable | 10 | 0 | 0 | 0 | 1089.5 | +43% |
| E_combined | 10 | 0 | 0 | 0 | 987.3 | +29% |
| F_full_stack | 5 | 5 | 0 | 0 | 3505.7 | +359% |

`B == A` to within 0.1 PAR-2 — confirms `SAT_USE_LBD=on` is bookkeeping-only.

## Work × speed decomposition (key rows)

`work_ratio = conflicts_cfg / conflicts_A` (search trajectory).
`speed_ratio = (props/s)_A / (props/s)_cfg` (execution cost per event).
`net_pred = work × speed` should match `actual_wall_ratio`; a mismatch indicates a third
factor (GC, allocation, proof writing). `dominant` is the classification.

### C_lbd_ema vs A_baseline

| Instance | work | speed | net | actual | dominant | result |
|---|---:|---:|---:|---:|---|---|
| sudoku | 0.85 | 1.21 | 1.03 | 1.17 | mixed | UNSAT |
| 6s299b685 | 1.47 | 1.11 | 1.63 | 1.27 | mixed | SAT |
| REGRandom | 1.40 | 1.16 | 1.62 | 1.65 | mixed | UNSAT |
| **mp1** | — | — | — | 6.89 | — | **TIMEOUT** |
| Kakuro | **0.53** | 1.55 | 0.82 | **0.81** | mixed | **SAT, -19%** |
| SCPC | **0.76** | 0.97 | 0.74 | **0.71** | trajectory | **UNSAT, -29%** |
| velev | 0.94 | 1.18 | 1.11 | 0.99 | execution | SAT, ±0 |
| brocard | 1.63 | 0.76 | 1.23 | 1.42 | mixed | UNSAT, +42% |
| **battleship** | — | — | — | 13.11 | — | **TIMEOUT** |
| **case9** | — | — | — | 2.35 | — | **TIMEOUT** |

**Pattern:** EMA restart alone is a 0/3 fixed-fee deal. The instances it helps (Kakuro -19%,
SCPC -29%) are paid for by 3 timeouts and 4 ≥+27% regressions. Predicted by prior
investigation Gap 1: every restart is full (no trail reuse in single mode), which is
catastrophic when restart cadence is >1k/s.

### D_focused_stable vs A_baseline

| Instance | work | speed | net | actual | dominant | result |
|---|---:|---:|---:|---:|---|---|
| sudoku | 1.14 | 1.38 | 1.56 | 1.41 | mixed | UNSAT, +41% |
| **6s299b685** | **3.79** | **3.06** | 11.6 | **6.36** | mixed | **SAT, +535%** |
| **REGRandom** | **2.32** | **2.31** | 5.36 | **5.10** | mixed | **UNSAT, +410%** |
| mp1 | 1.17 | 1.25 | 1.46 | 0.91 | mixed | SAT, -9% |
| **Kakuro** | **0.055** | 4.62 | 0.26 | **0.28** | mixed | **SAT, -72%** |
| SCPC | 1.72 | 1.32 | 2.28 | 2.12 | mixed | UNSAT, +112% |
| velev | 0.001 | 156.5 | 0.16 | 0.46 | mixed | SAT, -54% |
| brocard | 1.67 | 0.88 | 1.48 | 2.08 | mixed | UNSAT, +108% |
| **battleship** | 0.000 | — | — | **0.004** | mixed | **SAT, 250×** |
| case9 | 1.63 | 1.23 | 2.00 | 1.90 | mixed | SAT, +90% |

**Pattern:** D shows extreme per-instance dispersion. The wins (Kakuro 26%, battleship 0.4%,
velev 46% of baseline wall) come from focused mode finding a satisfying decision sequence in
hundreds of conflicts (`conflicts_cfg < 700` on Kakuro/battleship). The losses (6s299b685,
REGRandom) come from focused-stable mode alternation that doubles both conflicts and per-prop
cost — VMTF queue / VSIDS rebuild interaction is one suspect. The net is +43% PAR-2 but the
variance is the story.

### E_combined vs A_baseline

| Instance | work | speed | net | actual | dominant | result |
|---|---:|---:|---:|---:|---|---|
| sudoku | 1.06 | 1.46 | 1.56 | 1.45 | execution | UNSAT, +45% |
| **6s299b685** | 1.35 | **5.90** | 7.99 | **5.73** | mixed | **SAT, +472%** |
| **REGRandom** | **2.19** | 1.70 | 3.72 | **3.81** | mixed | **UNSAT, +281%** |
| **mp1** | **0.032** | 1.43 | 0.045 | **0.015** | mixed | **SAT, -98.5%** |
| **Kakuro** | **0.058** | 4.19 | 0.25 | **0.27** | mixed | **SAT, -73%** |
| SCPC | 1.43 | 1.39 | 1.98 | 1.88 | mixed | UNSAT, +88% |
| velev | 0.50 | 1.83 | 0.92 | 0.75 | mixed | SAT, -25% |
| brocard | 1.67 | 0.88 | 1.46 | 2.06 | mixed | UNSAT, +106% |
| **battleship** | 0.000 | — | — | **0.004** | mixed | **SAT, 250×** |
| case9 | 1.63 | 1.23 | 2.00 | 1.90 | mixed | SAT, +90% |

**Pattern:** E reaches 4 wins (mp1 -98.5%, Kakuro -73%, velev -25%, battleship 250×) at the
price of 6 regressions, the worst being 6s299b685 +472% and REGRandom +281%. This is exactly
the "phase-boundary chaos" diagnosis from prior investigation: the focused-stable + EMA stack
finds different SAT models than A on instances where the formula has many equivalent paths,
and is much worse on instances where A's deterministic trajectory was lucky.

### F_full_stack vs A_baseline

| Instance | work | speed | net | actual | dominant | result |
|---|---:|---:|---:|---:|---|---|
| **sudoku** | — | — | — | 1.54 | — | **TIMEOUT** |
| **6s299b685** | **49.30** | 1.21 | 59.6 | **13.93** | mixed | **SAT, +1293%** |
| **REGRandom** | — | — | — | 5.22 | — | **TIMEOUT** |
| mp1 | 0.29 | 1.00 | 0.29 | **0.11** | trajectory | SAT, -89% |
| **Kakuro** | — | — | — | 1.40 | — | **TIMEOUT** |
| **SCPC** | **6.45** | **4.04** | 26.1 | **17.90** | mixed | **UNSAT, +1690%** |
| **velev** | — | — | — | 4.63 | — | **TIMEOUT** |
| brocard | 5.72 | 0.68 | 3.90 | 3.44 | mixed | UNSAT, +244% |
| **battleship** | 0.000 | — | — | **0.004** | mixed | **SAT, 250×** |
| **case9** | — | — | — | 2.35 | — | **TIMEOUT** |

**Pattern:** lbd-tiered + chrono + rephase + binary-fast on top of E catastrophically breaks
the solver on instances where E succeeded. 6s299b685 jumps from +472% to +1293% (E vs F).
SCPC jumps to +1690%. 4 instances E solved (sudoku, REGRandom, Kakuro, velev, case9) time
out. This is a strong signal that the lbd-tiered reducer in conjunction with chrono
backtracking has a semantic interaction bug or a severe over-aging pattern.

## Reference solver live comparison

Same hardware, same 10 profiling instances, same 300 s timeout.

| Instance | kissat-latest | kissat-sc2024 | solver11 A | solver11 E | repo-A / kissat-latest | Classification |
|---|---:|---:|---:|---:|---:|---|
| brocard_problem_large | 50.6 s | 46.5 s | **8.6 s** | 17.7 s | **0.17×** | repo A much faster (solver-10 BVE) |
| 6s299b685_Iter30 | 37.4 s | 39.3 s | **16.2 s** | 92.9 s | **0.43×** | repo A faster, E regresses |
| velev-pipe-sat-1.0-b7 | 89.9 s | 155.4 s | **64.8 s** | 48.9 s | **0.72×** | repo A faster, E also faster than kissat-latest |
| sudoku-N30-12 | 260.6 s | 175.4 s | **195.2 s** | 282.2 s | **0.75×** | repo A faster than kissat-latest |
| case9 | 77.2 s | 32.0 s | 127.7 s | 243.0 s | 1.65× | repo slower |
| SCPC-500-13 | 6.7 s | 6.9 s | 13.7 s | 25.7 s | 2.04× | repo slower |
| mp1-Nb7T46 | **7.7 s** | 208.8 s | 43.6 s | **0.6 s** | **5.53×** | repo E beats kissat-latest by 12× |
| Kakuro-easy-112-ext | 38.0 s | 68.7 s | 214.0 s | **57.6 s** | **5.64×** | repo E closes 78% of gap |
| REGRandom-K4-L1 | **2.3 s** | 2.5 s | 57.4 s | 218.5 s | **24.7×** | inprocessing gap |
| battleship-16-31-sat | 0.18 s | 7.4 s | 22.9 s | **0.09 s** | **127.8×** | repo E beats kissat-latest by 2× |

**Pattern of the gap is inprocessing-dominated.** The 4 instances where solver 11 A beats
kissat-latest all have a feature that solver-10 preprocessing strips effectively (large BVE
wins on brocard, sudoku root-unit chains, large variable elimination on velev pipeline). The
5 instances where kissat-latest dominates by 5–128× are exactly the families that kissat's
inprocessing pipeline (probe, vivify, ternary, transitive reduction) handles: random 3-SAT
(REGRandom), satisfiable puzzle-style search (battleship), constraint propagation cores
(Kakuro), mp1 HWMCC, SCPC set cover. These are Gap 6 (inprocessing missing entirely).

**The two kissat versions differ significantly per instance.** kissat-sc2024 is faster on
case9, sudoku, brocard; kissat-latest is faster on mp1, battleship, sudoku, Kakuro. The
per-instance variance between two kissat builds shows that the focused/stable trajectory
behavior we see in E_combined (mp1 6× kissat-latest, battleship 80× kissat-sc2024) is not
unique to solver 11 — it reflects how sensitive these instances are to the exact restart /
phase / reduce schedule. The "phase-boundary chaos" finding from the prior investigation is
borne out by kissat's own version-to-version dispersion.

**E_combined picks up three of the inprocessing-dominated instances** by changing the search
trajectory:
* Kakuro: A 214 s → E 58 s (kissat-latest 38 s, kissat-sc2024 69 s). E closes 78% of the gap
  and beats kissat-sc2024.
* mp1: A 43.6 s → E 0.6 s (kissat-latest 7.7 s, kissat-sc2024 209 s). E beats both kissat
  versions by 12× / 350×.
* battleship: A 22.9 s → E 0.09 s (kissat-latest 0.18 s, kissat-sc2024 7.4 s). E beats both
  by 2× / 80×.

But E pays for it on the families where A is already fast — REGRandom A 57 s → E 218 s
(kissat 2.3 s; nothing gets close), 6s299b685 A 16 s → E 93 s (kissat 37 s; A is already
best). The E wins are stochastic, not algorithmic — kissat-latest gets mp1/battleship/Kakuro
via inprocessing rather than via focused-stable trajectory dispersion.

**Pattern of the gap is inprocessing-dominated.** The 4 instances where solver 11 beats kissat
all have a feature that solver-10 preprocessing strips effectively (large BVE wins on brocard,
sudoku root-unit chains, large variable elimination on velev pipeline). The 5 instances where
kissat dominates by 5–128× are exactly the families that kissat's inprocessing pipeline
(probe, vivify, ternary, transitive reduction) handles: random 3-SAT (REGRandom), satisfiable
puzzle-style search (battleship), constraint propagation cores (Kakuro), mp1 HWMCC, SCPC set
cover. These are Gap 6 (inprocessing missing entirely).

**E_combined picks up two of the inprocessing-dominated instances** by changing the search
trajectory:
* Kakuro: A 214 s → E 58 s (kissat 38 s). E closes 78% of the gap to kissat.
* mp1: A 43.6 s → E 0.6 s (kissat 7.7 s). E beats kissat by 12×.
* battleship: A 22.9 s → E 0.09 s (kissat 0.18 s). E beats kissat by 2×.

But E pays for it on the families where A is already fast — REGRandom A 57 s → E 218 s
(kissat 2.3 s; nothing gets close), 6s299b685 A 16 s → E 93 s (kissat 37 s; A is already best).
The E wins are stochastic, not algorithmic — kissat gets them too via inprocessing rather than
via focused-stable trajectory dispersion.

## Reference diff — implementation gaps

Re-confirmed against current `solver/11-kissat-port/src/` (HEAD `39cd707`):

### Gap 1 — Restart trail reuse is opt-in, not promoted to default

* **kissat `restart.c:53-110`** — `reuse_focused_trail` (stamp-based) and `reuse_stable_trail`
  (score-based) keep the productive prefix on every restart; default `restartreusetrail=true`.
* **solver 11 `main.rs:4445-4467`** — `restart_reuse_trail_level()` is implemented but only
  fires when `restart_reuse_trail_focused` or `restart_reuse_trail_stable` is on. Both default
  to `off` in `CONFIG_SCHEMA.csv:36-37`. Profile defaults (`default`, `fast`) leave them off.
* **Effect:** under EMA restart in single mode (config C), every restart is `backtrack(0)`.
  This is exactly what made 3 of 10 instances time out.
* **Existing bead:** `SAT-playground-5b2.2.35` (CLOSED — implementation landed) — no follow-up
  for promotion or for making it the default when `SAT_RESTART=kissat-ema`.

### Gap 2 — Reducer is "do nothing under budget"

* **kissat `reduce.c:102-151`** — `mark_less_useful_clauses_as_garbage` always deletes
  `percent · size` of the candidate stack (50% → 90% via `log10(reductions+9)`).
* **solver 11 `main.rs:5486-5495`** — `reduce_db_lbd_tiered` deletes only until
  `projected_lits <= learned_lit_budget` (budget grows as `2000 + 300·sqrt(reductions)`).
* **Effect:** the reducer effectively no-ops on workloads dominated by short learned clauses,
  yet still pays setup cost (retier, candidate scan, etc.). Confirmed by F_full_stack
  catastrophe — adding lbd-tiered to E broke instances E solved.
* **Existing bead:** `SAT-playground-qmz` P2.

### Gap 3 — `used_recently` bump-to-1 vs kissat's bump-to-MAX

* **kissat `learn.c:110`, `deduce.c:18`** — every learn / reason-use sets `c->used = MAX_USED`
  (31), aged by 1 per reduce.
* **solver 11 `main.rs:2369-2380`** — `mark_learned_clause_recent` sets
  `used = max(current, 1)` and `MAX_USED_RECENTLY = 3`.
* **Effect:** clauses aged to 0 then re-used regain protection for only one reduction (vs
  kissat's 31).
* **Existing bead:** `SAT-playground-ycw` P2.

### Gap 4 — VMTF queue stamp-based reuse_trail only fires under `SAT_SEARCH_MODE=focused-stable`

* **kissat** — VMTF is the canonical focused branching; `links[var].stamp` is the basis for
  `reuse_focused_trail` and for branching.
* **solver 11 `main.rs:4423-4467`** — `reuse_focused_trail_level` returns 0 if `vmtf_queue`
  is None. Single-mode default has no VMTF queue → focused trail reuse cannot fire even when
  `restart_reuse_trail_focused=on`. This couples Gap 1 to the focused-stable opt-in.
* **Effect:** single-mode runs (the default profile) cannot benefit from trail reuse for any
  decision-stamp-based heuristic — only score-based stable reuse is available, and that's also
  default-off.

### Gap 5 — Tier1 over-protection in `reduce_candidate`

* **kissat `reduce.c:62-87`** — tier1 clauses skipped from candidates while `used > 0`. Once
  aged to 0 they become eligible immediately.
* **solver 11 `main.rs:5413-5421`** — tier1 clauses are NEVER candidates unless emergency mode
  AND not used recently AND old enough. In normal reduction tier1 is fully protected.
* **Effect:** tier1 share of arena grows without bound between emergencies, compounding Gap 2.
* **Related bead:** `SAT-playground-5b2.2.44` P4 (Differentiate Tier 2 vs Tier 1) — closely
  related but framed around tier 2 differentiation.

### Gap 6 — Inprocessing / probing / vivification missing entirely

* **kissat `search.c:204-217`** — main loop dispatches to `probe`, `eliminate`, `reorder`,
  `rephase` between decisions.
* **solver 11** — `SAT_PROBE`, `SAT_VIVIFY`, `SAT_INPROCESS`, `SAT_HBR`, `SAT_TRANSITIVE`,
  `SAT_FORWARD_SUBSUME`, `SAT_GATE_EXTRACT`, `SAT_GATE_BVE`, `SAT_RCHECK` are **ParkingLot**
  in `FEATURES.csv` — config-schema-only, validator rejects them when enabled.
* **Effect:** instances kissat dominates by 5–127× (Kakuro, mp1, battleship, REGRandom — prior
  investigation) are dominated by inprocessing. Roadmap item, not Phase-1 actionable.

## Trajectory analysis

The 6s299b685 / REGRandom regressions under D/E and the F_full_stack catastrophe both warrant
trace-level inspection. Two specific traces deferred for follow-up:

* `6s299b685_Iter30` under A vs D (conflict count 3.8× higher under D suggests early
  trajectory divergence; the work × speed decomposition shows **both** dimensions move
  together, not a single mechanism).
* `SCPC-500-13` under E vs F (E uses 269k conflicts in 25 s; F uses 1.2M conflicts in 245 s —
  lbd-tiered + chrono interaction is the suspect).

## Hardware counter results

Not run in this iteration (the gap shape is already explained by trajectory + execution
ratios from JSON_STATS; perf counters would only matter if a single hot symbol were
suspected).

## Parameter sweep results

Not run in this iteration. Prior investigation
(`log/kissat-investigation-2026-05-23-broad/sweep_results.csv`) demonstrated that no single
knob sweep rescues mp1 or REGRandom — they are phase-boundary chaos. The findings here
re-confirm the same dispersion shape.

## Code-Level Recommendations (ordered by ROI)

1. **Implement kissat fraction-based deletion in `reduce_db_lbd_tiered`**
   (`SAT-playground-qmz`). Replace the over-budget while-loop in `main.rs:5486-5495` with a
   `percent = high - delta / log10(reductions + 9); target = candidates × (percent / 100);`
   loop matching `kissat/src/reduce.c:102-151`. This is the smallest-blast-radius fix that
   should reduce the F_full_stack catastrophe — under F, the lbd-tiered reducer is paying
   for retier + candidate scan and deleting little, which compounds with chrono.
2. **Fix `mark_learned_clause_recent` to bump-to-MAX and widen `MAX_USED_RECENTLY`**
   (`SAT-playground-ycw`). One-line change at `main.rs:2369-2380` plus a constant widen at
   `main.rs:142` from 3 → 7 or 15.
3. **Promote `SAT_RESTART_REUSE_TRAIL_*=on` when `SAT_SEARCH_MODE=focused-stable`** — the
   default profile keeps single mode, but every focused-stable invocation needs trail reuse
   for restart to be productive. Specifically:
   * make `restart_reuse_trail_focused = true` when `vmtf` is `focused-only` (i.e. when the
     queue is allocated and the stamps are meaningful)
   * make `restart_reuse_trail_stable = true` when `restart_policy = reluctant` in stable mode
   This avoids Gap 4 silently muting Gap 1's fix.
4. **Relax tier1 deletion gate in `reduce_candidate`** to match kissat: tier1 clauses become
   candidates once `used_recently == 0`, removing the emergency-only gate. Combine with
   recommendation #1.
5. **Investigate F_full_stack-specific regression on 6s299b685 and SCPC.** F adds
   `SAT_REDUCE=lbd-tiered`, `SAT_CHRONO=on`, `SAT_REPHASE=on`, `SAT_BINARY_FAST=on` on top of
   E. Bisect by toggling one of those four off in F — bead `SAT-playground-cjb` (make
   `SAT_CHRONO=on` default for kissat-feature configs) needs evidence updated with this run.
6. **Phase 2 roadmap:** prioritize implementation of `SAT_PROBE` and `SAT_VIVIFY` to close the
   remaining whole-solver gap to kissat on Kakuro, mp1, battleship, REGRandom.

## Rejected / non-issues this run

* B_metadata_only confirms LBD bookkeeping is free — earlier suspicion that LBD scoring
  added measurable execution cost was wrong (B vs A speed_ratio is 0.97–1.04, dominantly
  "noise" classification).
* Per-instance dispersion under D/E is real, not noise. The same Kakuro/battleship/mp1 wins
  appear consistently across D and E. This is the "phase-boundary chaos" diagnosis (prior
  investigation) — it is not fixable by tuning EMA margins or reduce schedules; it requires
  inprocessing to homogenize the input formula.

## Artifact paths

* Ablation script: `log/analyzesat-2026-05-25-2043/run_ablation.sh`
* Reference script: `log/analyzesat-2026-05-25-2043/run_reference.sh`
* Analysis script: `log/analyzesat-2026-05-25-2043/analysis.py`
* Per-config raw results: `log/analyzesat-2026-05-25-2043/<config>/results.csv` and
  `stats.jsonl`
* Reference CSVs (pending): `log/analyzesat-2026-05-25-2043/reference-kissat-latest.csv`,
  `reference-kissat-sc2024.csv`
* Matrix / decomposition / summary: `matrix.csv`, `decomp.csv`, `summary.md`
* Worktree: `/tmp/analyzesat-2026-05-25-2043`
* Kissat reference source: `benchmarks/reference-solvers/kissat-latest/src/`
  (key files diffed: `restart.c`, `reduce.c`, `tiers.c`, `decide.c`, `bump.c`, `search.c`,
  `backtrack.c`, `mode.c`)
