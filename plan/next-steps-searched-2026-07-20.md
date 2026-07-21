# SAT_SEARCHED saved replacement-scan position (2026-07-20/21 night session)

## Outcome — always-on gate FAILED, mechanism validated, armed variant built

Gate #1 (`log/abtest-cand-vs-base-2026-07-20-22-35-43`, `cand:` always-on
vs `base:SAT_SEARCHED=off`): **FAIL — solved 63 vs 69** (PAR-2 156,810 vs
138,962), ZERO contradictions / correctness failures across 200 runs.

The 12 status flips tell the whole story:

- **Gains (3) — ALL kissat-only target cells**: bp4_TCO_CSO_IXA_LP
  (SAT 1018s; kissat 1287s), **TT406 (SAT 250s; kissat 41s — the cell the
  aggregate called "cheapest +1, blocked on a stabilizer")**, oski15a01b20s
  (UNSAT 1766s). The propagation-throughput mechanism reaches exactly the
  propagation-heavy cells it was measured on.
- **Losses (9) — all SAT-lottery / wall-margin rerolls**: oddball_80_5
  (base 286s), jkkk-one-one (base 7s!), case1 (340s), VanDerWaerden
  (1301s), bp4_BC012_CSO_FPBEQ_FPBLE (209s), bp4_TCO_CSO_ZR (1738s),
  rbsat-v1375 (1795s — the documented 5.4s-margin coin-flip), sted2_0x1e3
  (1615s), TT492 (1585s — the TT-class seesaw: TT406 in, TT492 out).

Verdict: the global trajectory reroll (every replacement-watch choice
changes) is the killer, not the mechanism. Response built the same night:
**SAT_SEARCHED=armed (new default)** — the scan latches on per-formula only
when cumulative props/conflict >= SAT_SEARCHED_ARM_PPC (default 512) after
SAT_SEARCHED_ARM_MIN_CONFLICTS (default 300k), checked at restarts. Cells
that never cross stay BYTE-IDENTICAL to the shipped solver.

Per-cell ppc @100k conflicts (off-arm trajectories) for the 12 flips:
gains bp4_IXA_LP 898 / oski15 1063 / TT406 554; losses bp4_BC012F 1435,
bp4_ZR 1098, case1 919, TT492 556, oddball 490, jkkk 434, sted2x 213,
rbsat 178, vdw 30. No ppc threshold separates gains from losses (the bp4
SAT-lottery members have the HIGHEST ppc) — hence the 300k conflict floor:
cells solving before 300k conflicts keep the shipped trajectory outright
(protects bp4_BC012F @239k, oddball @191k, jkkk @30k), and the sub-512-ppc
cells (rbsat, sted2x, vdw, oddball, jkkk) are protected at any depth.
Protected: 6 of the 9 gate-#1 losses. Residual fresh-draw risk: case1,
bp4_ZR, TT492. All three gains still arm (they solved at 810k-2.7M
conflicts in gate #1).

Armed-build validation: jkkk + oddball full solves byte-identical to HEAD
(only diff = the new `searched_armed_at_conflict: 0` stat key itself);
bp4 arms at conflict 300,748 and keeps wall −6.4% vs off (ticks/prop 26.9
vs 34.5) — most of the always-on −10.1% survives the 300k identical
prefix. 658 tests (armed-latch unit test added), smoke 9/9.

GATE #2 RESULT (`log/abtest-cand-vs-base-2026-07-21-02-30-51`): **FAIL —
solved 65 vs 69** (PAR-2 151,427 vs 138,331; base reproduced 69 exactly).
The protection design WORKED — all 6 protected gate-#1 victims (jkkk,
oddball, rbsat, sted2x, vdw, bp4_BC012F) stayed solved — but every armed
cell was a fresh lottery draw: 4 new losses, all armed-cell rerolls
(bp4_CSO_IXA_ZR base 415s, velev-pipe-sat base 579s, lockchart-group2
base 596s, and oski15 **UNKNOWN_rc-6 at 935s = SIGABRT at the 16GB cap**
— the armed trajectory measured 15.9GB RSS standalone where base stays
under 16GB: trajectory rerolls change MEMORY PEAKS, not just walls). None
of gate #1's three gains repeated (fresh draws don't repeat).

FINAL DISPOSITION: SAT_SEARCHED default = OFF (committed as neutral
groundwork; off-arm trajectory-identity vs c8228aa re-verified on the
final build: jkkk full solve + vex @300k differ only by the new
`searched_armed_at_conflict: 0` stat key). The mechanism is parked, not
dead — see the aggregated plan's "re-luck bundle" item for the promotion
path. Two-gate lesson: against a lottery-banked 69 baseline, global
trajectory rerolls are −EV even when the underlying throughput win is
real and the targeted flips demonstrably happen.

## The change (solver/12-kissat-inprocessing)

Port of kissat clause.h `searched`: each clause header now carries a 6-bit
saved replacement-scan resume position (clamped to 63, 0 = unset). The
propagate replacement scan starts at the saved position and wraps to the
2..start prefix only if the tail finds nothing; on success the found
position is stored back into the header. Both the PTR_FAST and legacy scan
paths implement it identically. `SAT_SEARCHED=off` (the base arm) never
reads or writes the field, reproducing the shipped scan-from-2 behavior
verbatim.

Field placement is THE load-bearing decision:

- The 6 bits are carved from the header size field (27 -> 21 bits, shift
  5 -> 11; sizes >= 2^21 now hard-assert instead of silently corrupting —
  no realistic clause comes near it).
- The header word is loaded by the hot loop anyway, and its cache line is
  dirtied by the lits[1] swap on every successful watch move, so the field
  costs ZERO extra memory traffic.
- A first-cut TRAILING-WORD variant (extra arena word after the extras for
  clauses len >= 8) measured a NET WALL REGRESSION on a clean paired bp4
  screen (+3.4% wall despite −16% ticks/prop): the tail-word load/store
  touches an extra cache line per long-clause visit and gives back more
  than the scan saves. Do NOT resurrect the word variant.

Giants (install_preparsed_giant_arena) force `searched_scan = false`: the
field is memory-free, but enabling it rerolls the trajectories of the
memory-fit giant solved cells (00fd8ac / 83aa / ee5). Giants stay
byte-identical in both arms. Enabling searched for pj2008-class giants is
a deliberate follow-up gate (see below).

Touch points (all header rebuilds preserve the field via
`header & CLAUSE_SEARCHED_POS_MASK`): clause_set_deleted, subsumption
mark/unmark (simp.rs), GC in-place + copy paths, abstraction migration.
Shrink sites reset the field to 0 (a stale position clamps back to 2 at
the next scan anyway). Fresh clauses start at 0.

## Why this change (the measurement chain)

1. bp4_TCO_CSO_IXA_LP_ZR (kissat-only cell, 1287s kissat) at matched 2M
   conflicts: ours 790s vs kissat 243s. Decomposition: props/s 2.23M vs
   11.7M (5.2x!), ticks 52/prop vs kissat 3.15 cache-line ticks/prop.
2. SAT_STATS_HOT decomposition on bp4 @300k conflicts:
   **replacement-scan literals = 21.66/prop = 61% of ALL search ticks**,
   avg 12.69 literals scanned per loaded clause; watcher visits 13.78/prop
   (82% resolved by blocker hits — healthy); binary props 0.79/prop.
   The always-from-2 scan rescans the falsified prefix every visit; kissat
   amortizes it with `c->searched`. This was the single biggest measured
   structural gap left in propagation.
3. Bubble (density class) at matched 5M conflicts: ours ~527s vs kissat
   226s; props/conflict 191 vs 103.5, props/s 1.81M vs 2.29M, restart
   interval 528 vs 39, kissat vivified 335k (53% of checks) vs our 43k
   strengthened. (Restart cadence + vivify-hit-rate ports measured no-flip
   in earlier sessions — the remaining density delta is rate/quality.)
4. pj2008 (giant, kissat 1165s): kissat does 200k conflicts in 739s at
   18.9k props/conflict, 5.1M props/s; ours did NOT reach 200k in >2100s
   CPU (killed). Propagation-bound giant — the searched follow-up target.

## Screen evidence (final header-field build)

- CLEAN paired bp4 @1M conflicts (idle box, sequential):
  **wall −10.1% (287.4s vs 319.6s), props/s +11% (3.29M vs 2.97M),
  ticks/prop −29% (24.6 vs 34.5)**, trajectory similar (946M vs 949M props).
- Bubble @1.5M: ticks/prop −9% (36.0 vs 39.5); wall +2.9% but measured
  under 3-way contention — unreliable; density clauses are short so the
  scan win is structurally smaller there.
- Identity screens (SAT_SEARCHED=off NEW binary vs pre-change HEAD c8228aa
  binary, stripped SAT_STATS_JSON byte-compare): **IDENTICAL on ibm (full
  SAT solve), vex @300k conflicts, bubble @1.5M conflicts** — the off arm
  is the shipped solver verbatim.
- 657 unit tests (2 new: header-field roundtrip/isolation; GC +
  delete-mark preservation; plus an on/off random-CNF agreement fuzz test),
  smoke 9/9 with drat-trim.

## Reroll surface / EV prediction (recorded BEFORE the gate)

This is a deliberate global trajectory reroll on all NON-giant cells
(different replacement watches -> different watch-list orders), traded for
a measured propagation-throughput win concentrated on cascade-heavy cells.
Giants are byte-identical. Prediction: solved-count tie or better with
suite-wide wall margins improving on search-heavy cells; known coin-flip
exposure: rbsat (5.4s margin), TT492/TT406 armed-lottery class, oski
wall-lottery cells (oski20 margin 107.7s should absorb noise).

## Follow-ups

1. If the gate WINS: try `SAT_SEARCHED` on giants (one deliberate gate) —
   pj2008 is propagation-bound and the field is memory-free; risk is
   rerolling 00fd8ac/83aa/ee5/18.normalised.
2. bp4 remains 2.4x off kissat AFTER this change (ticks/prop 24.6 vs
   kissat's ~3 cache-line ticks; units differ but the visit count
   13.78/prop with 11.3 blocker hits is the next chunk — kissat's
   watch-list entries for long clauses are 2 words (blocking lit + ref)
   consumed sequentially; ours are 1 struct of 8B — already similar.
   Next measurable: watch-list LENGTH per visit (DB size / tier policy).
3. Density class (Bubble/booth/fixedbandwidth): conflict-rate gap is now
   part-propagation (this change), part learned-clause quality — the
   2026-07-20b aggregate's item #1 (rate measurement + reduce/retention
   comparison) remains open.
