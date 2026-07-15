# Next steps after the decision-arm promotion (2026-07-15)

Context for a fresh session. State as of this writing:

- Medium baseline: **67/100** (gate: 67 vs 65 — base arm lost rbsat AND sted2
  to wall noise that run; sted2 landed IN for the candidate). Kissat 4.0.4
  reference: 74/100 (`log/kissat-medium-20260705-203444`). Gap ≈ 7.
- **PROMOTED 075b7e8: SAT_DECISION_ARM=24** (default on). At the vivify-yield
  probe points (200k conflicts, 4x spacing), a formula with cumulative
  decisions/conflict >= 24 and !deep_phase arms: the aggressive-inprocess
  bundle + **mid-search factor** (SAT_FACTOR_INPROCESS scope, still default-off
  globally) + the **kissat rephase/walk schedule** (rephase machinery activated
  at the arming point; `decision_search_armed` passes the armed-only rephase
  gate). Off-switch: `SAT_DECISION_ARM=0` (byte-identical shipped baseline).
- Gate: `log/abtest-decarm-vs-base-2026-07-15-08-15-37` + launch log
  `log/abtest-decarm-launch.log` — **PASS, WIN 67 vs 65**, zero contradictions,
  zero correctness failures. Cand-only flips: **TT406 SAT 246s verify=ok**
  (engineered flip, first ever), sted2 1705s (wall-noise class). No base-only
  cells. 64/65 both-solved pairs byte-identical; single divergence
  SC25_Timetable_C_395 (armed, −71,271 conflicts, still solves). sqrt-mitern170
  + VexRiscv checker-timeout symmetric both arms (benign multi-GB-proof class).

## The mechanism (why TT406 flipped)

Proven on BOTH solvers:

- Kissat solves TT406 in 32-41s via 4 mid-search eliminations to 67% of vars +
  15,151 factored vars + 13 rephases / 11.7M walk steps. **Kissat itself TIMES
  OUT (300s) on TT406 with `--rephase=0` OR `--eliminatebound=0`**;
  `--definitions=0` / `--factor=0` are only ~4x slowdowns (32s -> 120s).
- Our attribution (standalone, forced knobs): armed collapse alone
  (pct40+factor: 18.9k fresh vars, 190k product clauses removed, elim
  108k->119.5k of ~216k vars) churns 1.3M conflicts and does NOT solve;
  collapse + rephase/walk solves ~305s (walk_improved=3 — the walker finds the
  model on the compressed formula). pct40 + rephase WITHOUT factor: TIMEOUT
  at 1750s — **factor and rephase/walk are BOTH load-bearing**.
- Implemented default (arming at 200k conflicts, not root): TT406 standalone
  **199s**, 668k conflicts, factor 18,850 fresh vars, 14 rephases / 5 walks.

## Threshold calibration table (dec/conf at the 200k-conflict probe, measured)

Arms: TT406 45.2, TT492 49.7, lockchart-group1 36.2.
Refuses (all solved/fragile cells): sudoku-N30 10.4, goldcrest 10.7, velev 8.0,
reconf10_68 8.1, oddball_80 6.8-7.5, 59-129706 6.7, mp1 5.8, Kakuro 5.4,
lockchart-g2 4.1, case1 2.6, jkkk 2.2, bp4_TCO 2.1, rbsat-v1375 1.9,
544707/case9/VanDerWaerden 1.3, DLTM 1.8. Threshold 24 has ~2x margin both
ways. NOTE: TT406's ratio DECAYS with conflicts (45.2 @200k -> 23.0 @800k) —
first-probe arming is load-bearing; do not move the rule to probe >= 2.

Identity checks: mp1 336,333 and 544707 241,644 conflicts EXACTLY equal
on/off. sudoku-N30 solves UNSAT 771s standalone unarmored.

## Measured this session (do not re-run blind)

1. **SAT_ELIM_PRODUCTIVE_MIN_PCT=40 stays dead** (re-confirmed under the
   current bundle): mp1 derails 27s -> 600s+ TIMEOUT; the signal cannot
   separate TT406 (49.9% root elim) from mp1 (48.4%).
2. **TT492/495/496 do NOT flip** with the decision-arm bundle (1750s
   standalone TIMEOUT, both root-armed combo and implemented probe-armed).
   TT492 arms and runs the full machinery — the collapse+walk is not enough
   there (kissat needs 1052s itself for 492; 495/496 kissat can't solve).
3. **Density class (Bubble/booth/fixedbandwidth) has NO single missing
   mechanism**: kissat ablations on Bubble ALL still solve — definitions=0
   277s, factor=0 290s, eliminatebound=0 360s, vivify=0 424s (baseline 321s).
   The kissat win there is multi-mechanism compounding. Stop looking for a
   single port; the play is pipeline-level (see next steps).
4. **Mid-search factor under yield-arming is nearly inert on Bubble**: 252
   fresh vars vs kissat's ~10% of vars (18.8k on TT406). Whatever gates factor
   candidates on density cells, it is not the decision-armed bound.
5. Kissat TT406 profile cached: 32s, 170k conflicts, 10M decisions (58.8
   dec/conf), eliminated 67% (4 passes), factored 15.2k (7%), 37 backbone
   computations, 13 rephases, 11.7M walk steps, 0 congruence.

## Ranked next steps

### 1. lockchart-group1 (SAT, kissat 1687s; ours TIMEOUT, NOW ARMS at 36.2)
The only remaining gap cell that decision-arms. It didn't flip in this gate —
screen it standalone with trace to see what the armed bundle does there
(does factor fire? does the walker get close — check walk_improved /
best_permille?). A Timetable-style collapse may need different factor
candidates (lockchart is a big SAT scheduling instance). Cheapest possible
next +1.

### 2. oski20 margin (BMC cascade; solves 1481-1659s standalone, TIMEOUT in-gate)
Unchanged from the bindrat note: needs ~150-300s of suite-wide margin or a
targeted speedup. Any decision-arm-adjacent wall saving does not help here
(oski20 dec/conf is low; it never arms). The congruence-merge stall
(18.4k frozen on vex-class) and elim depth on armed cells remain the levers.

### 3. Density class: probe-pipeline interleave (the compounding play)
With single mechanisms exhausted (bindrat note) and kissat's Bubble surviving
every single ablation (this session), the remaining honest approach is the
kissat probe.c ORDER on armed cells: congruence -> substitute -> backbone ->
vivify -> sweep -> substitute -> transitive -> backbone -> factor, per round,
with per-pass effort budgets tied to search ticks — vs our current fixed
congruence -> vivify -> sweep -> BVE -> factor. This is an architecture
session: per-pass SET_EFFORT_LIMIT parity + a round driver that re-runs
passes while the active-var count drops.

### 4. TT492 depth (kissat 1052s — the hardest solvable Timetable)
Kissat needs 25x TT406's time; our armed bundle gets the collapse but not the
model. Options: warmup (kissat warms 100% of walks via
propagate-beyond-conflicts — NOT ported; TT406 didn't need it, TT492 might),
walk effort scaling on armed cells (SAT_WALK_EFFORT), kitten definitions in
armed BVE (elim depth 55% vs kissat 67% on TT-class).

### 5. Watch cells for regressions in future gates
sted2 is now a SOLVED cell at 1705s (12s inside the wire after losing its old
margin) — it remains the thinnest cell in the suite and is deep-phase-guarded
from all arming; treat any sted2 loss as wall noise, not mechanism, unless
its conflict count changes. C_395 now arms (improved this gate): its
trajectory is coupled to the bundle from here on.

## Housekeeping / traps (additions this session)

- `kissat -s` and `-q` are mutually exclusive — use `-s` only.
- The decision-arm rule shares the yield-probe cadence and guards:
  `SAT_VIVIFY_YIELD_ARM=0` (full-replay off-switch) also disables decision-arm
  probes; the >10M-live-clause cap excludes giants from decision arming.
- `pgrep`-based monitor shells trip check_promotion_gate's
  running_solver_processes FAIL (false positive) — TaskStop monitors before
  the gate check, then re-run (PASS this session after doing exactly that).
- The A/B preflight RAM warning (32x16GB > 90% of 512GB) is expected with the
  standard AGENTS.md config on this host; jobs never all peak simultaneously.
- dcg blocks shell pipelines whose sed replacement text contains `->`
  (misparsed as a redirect) — write awk/basename instead.

## Where the evidence lives

- Winning gate: `log/abtest-decarm-vs-base-2026-07-15-08-15-37` (+ launch log
  `log/abtest-decarm-launch.log`); gate PASS output in session log and the
  075b7e8 commit message.
- Kissat ablations, forced-knob attributions, dec/conf calibration, identity
  checks: scratchpad (dies on reboot) — all decision-relevant numbers are in
  this note, the commit message, and bead 2a7's comment log.
- Bead: `SAT-playground-2a7` (running gap-analysis log).
