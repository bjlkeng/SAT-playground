# NEXT PLAN — 2026-08-12b (supersedes 2026-08-12; PRUNED)

One-file plan for the next clear context. SESSIONS 4-13 bodies live in git
history (`git log -p plan/next-plan.md` up to 52a8f95); SESSIONS 14b/14c/14d
bodies were pruned earlier — full text in revisions up to 93ab682. Where this
file contradicts an older revision, THIS file wins.

**START HERE:** read "SESSION 20 FINAL VERDICT" below, then "SESSION 19"
(sweepcount), then "RANKED PLAN", then "Standing traps".

## SESSION 20 FINAL VERDICT (2026-08-13) — yield-latch arc closed as a BENCH-WASH after two full A/Bs and per-cell calibration; two standalone first-evers banked as evidence

The complete arc (all default-off in tree, commits 9b78fa8..978cbd6+):
latch + aggressive cadence + wide envs + kitten flips + repr streaming +
fast kitten + calibrated band (abs >= 1000 equivs) + early probe
(SAT_SWEEP_YIELD_PROBE, declines byte-identically).

**Measured outcomes:** STANDALONE conversions of two kissat-only cells —
HCP-446-105 (SAT 2730 s, model independently verified vs all 247,657
clauses; formula collapsed 51% by the cascade) and dislog_a14 (SAT
~2400-2500 s, reproduced in-gate in BOTH A/B deals). But the FULL-BENCH
A/Bs: 20-permille band LOSE 290 v 295
(log/abtest-cand-vs-base-2026-08-12-18-25-30: armed too widely);
calibrated band LOSE 292 v 293
(log/abtest-cand-vs-base-2026-08-13-07-58-47: dislog + bp4 gained, but
HCP cannot beat the in-gate wall — 2730 s standalone + 32-way contention
> 3600 — and sqrt169/oddball-class collateral persists). HCP's yield
develops too late for the early probe (103 equivs at 150k conflicts v
1490 at 810k). Tightening the band further would select exactly dislog =
one-cell overfit, forbidden. **VERDICT: the latch is a deal-wash on THIS
bench (net −1..+1). Keep default-off. It becomes valuable when (a) a
bench carries more equivalence-rich cells, (b) contention drops (fewer
jobs → HCP lands), or (c) the whole-loop sweep port closes the remaining
kitten-throughput gap and pulls solve times down across the class.**
Per-cell arming-yield table and all probes recorded in SESSION 20 items
1-7 above.

## SESSION 20 (2026-08-12) — NO PROMOTION: the uniqinv40/sweep-equivalence arc mapped to its root; miter-congruence definitively killed; yield-escalate latch banked default-off

**Flagship target: uniqinv40prop (kissat 51 s UNSAT, we timeout — a 70x
structural gap).** kissat's measured recipe there: 3,799 sweep equivalences
(30% of vars) over 24 sweeps / 130k kitten solves + 3,108 congruence
matches, then 549k conflicts. Layer-by-layer findings (all measured, none
speculative):

1. **Congruence matching is NOT the entry point:** every one of our 12,092
   extracted AND gates has a DISTINCT input pair — 0 syntactic merges exist
   pristine. kissat's 261 initial congruent vars only arise after its
   substitute→re-extract cascade reaches critical mass. (Also killed for
   the miter class: pristine boothdadda29 extraction = 5,162 gates but 1
   merge — booth/dadda halves share no syntactic gate structure; the plan's
   'congruence blind on miters' hypothesis is DEAD, and the stats-only-
   on-apply artifact that suggested it is noted below.)
2. **SAT_SWEEP_YIELD_ESCALATE latch built (default OFF, commit 9b78fa8):**
   percent-scale equivalence yield latches retire-scan + escalation +
   seed budget 2048 + substitution + aggressive cadence. On uniqinv40 it
   arms at round 1 (375 equivs) and substitutes ~113 distinct vars — then
   the cascade STALLS (~500 distinct equivalences total vs kissat 3,799).
3. **Environment size is NOT the residual:** depth-8/8192-var environments
   yield ZERO (the 2000-solve budget dilutes; pairs are LOCAL).
4. **Transitive pair-waste is NOT the residual:** a union-find skip
   (prove_facts_budgeted_opts, yield-armed rounds only) changed nothing —
   yields identical. The residual is (a) duplicate proving across
   OVERLAPPING environments (whole-env retirement exists only in the tick
   engine) and, deeper, (b) kissat's per-sweep candidate mechanics
   sustaining high yield across 24 sweeps where ours dries up after 2.
   **Closing this needs a faithful kissat sweep.c pair-mechanics port — a
   full session, promoted to ranked item 1.**
5. **SESSION 20b continuation (same day): kitten `flip_literal` PORTED
   (kitten.rs, kissat parity: rewatch-or-fail walk of the true literal's
   watch list; free model-space disproof of backbone/equivalence
   candidates) and wired into the yield-armed sweep (flip pre-tests before
   every solve). Wide-env armed bounds folded into the latch (4096 vars /
   16384 clauses / depth 5 / 64 seeds — probe: round-1 yield 375 → 704 and
   the cascade SUSTAINS ~50/round instead of dying). uniqinv40 still does
   NOT convert at 3600 s (~10x short of kissat's 3,799-equivalence
   critical mass). THE REMAINING PORT CHUNK, precisely: (a) kissat
   `sweep_repr` — substitute proven representatives INSIDE the kitten
   environment mid-sweep so the region collapses while being swept (ours
   applies equivalences only after the round via ELS); (b) kitten solve
   throughput (kissat ~18 µs/solve; profile ours). Acceptance test
   unchanged: uniqinv40 at 3600 s. All SESSION 20/20b knobs default-off;
   defaults byte-identical to SESSION 19.**

par32-2/dubois50 XOR recovery also closed this session (par32's pure-XOR
subsystem is consistent — SAT_GAUSS_MIN_COVERAGE env banked; dubois50's
clause var-sets are all distinct post-transformation). Validation: 761+5
tests, smoke 9/9, rbsat fingerprint digit-exact (all new knobs default
off; defaults byte-identical to the SESSION 19 promotion).

## SESSION 19 (2026-08-11/12) — frontier-sweep counting engine PROMOTED: mchess_20 FIRST-EVER (0.011 s refute, drat-trim VERIFIED); ranked research arc 3 delivered

**Shipped (commit 09a271b + promotion): `src/sweepcount.rs` + SAT_SWEEPCOUNT
default ON.** Pre-search refutation of exactly-one bipartite cover imbalance
(mutilated-chessboard class). The proof design that unblocked the arc: NOT
the H^4 inductive php closer (1.15G lines at H=198, RAT-scan-dead) but a
FRONTIER SWEEP — order cells by bandwidth, keep banded unary counters over
the open-edge frontier (width 21 for mchess_20, not H=198!), advance the
invariant FB−FW=δ per cell via single-pass-RUP lemma batteries, empty
clause when the frontier sweeps out with δ=2. 291k lines, 2.5 MB, verify
115.7 s. The battery engineering (all validated by drat-trim forward
checking on synthetic 4x4/8x8/20x20): definitions RAT-pivot-first; extend
E1-E3 + reverse H0/H1/REV + level-monotone M on the append side; bridge
D1-D5 + per-removed-edge transfer T on the removal side; two-direction
banded invariant on top. KEY LEMMA-ENGINEERING LAWS learned (for the next
proof engine): (1) a lemma is single-pass-RUP only if every case branch is
resolved by an EARLIER lemma — emit per-edge helper batteries BEFORE their
OR-lifted forms; (2) negation of a constant-false counter level is
constant-TRUE (vacuous lemma), never "drop the literal" — conflating these
emits false claims; (3) band saturation needs a 4-state level type
(true/false/var/UNTRACKED) — untracked levels must skip the lemma entirely.

**A/B `log/abtest-cand-vs-base-2026-08-11-19-37-55`:** +mchess_20 (UNSAT
0.05 s in-gate, proof verified ok); ALL 291 shared solved cells
conflict-IDENTICAL — the decline-is-identity claim proven at bench scale;
raw 293 v 294 solely from two documented thin-margin flippers
(valves-gates 33 s, oddball_19_4 103 s) swapping on wall under contention.
Judged per the trade rule: 2 wall coins (test 1, ≤120 s margins, identical
conflicts) v a deterministic first-ever — PROMOTED. Zero contradictions,
zero correctness failures.

**Also measured this session (negatives, recorded):** par32-2's pure-XOR
subsystem is CONSISTENT (gauss's coverage decline at 0.798 was honest —
SAT_GAUSS_MIN_COVERAGE env added, default unchanged); dubois50's clauses
all sit on DISTINCT var sets (transformed instance — no XOR groups to
extract; both stay both-timeout). rook-51/52/56 do NOT fit sweepcount
(P==H balanced rook constraints; their hardness is not color imbalance).

## HEAD-TO-HEAD RE-BASELINE (2026-08-10, user-requested double-check) — solver12 292 v kissat 294 same-host same-deal; gap −2; NUMA-balanced pinning landed

Sequential full-bench runs, 3600 s / 16 GB / 32 NUMA-balanced cores (no
contention between arms):

| solver | solved | PAR-2 | unique | TSV |
|---|:--:|--:|:--:|---|
| solver12 (promoted defaults) | 292/400 | 944,307 | 42 | `log/abtest-solver12-2026-08-10-00-01-22/solver12/results.tsv` |
| kissat 4.0.4 | 294/400 | 930,904 | 44 | `log/kissat-full-20260810-073149/results.csv` |

- **solver12 REPRODUCED its promoted 292 exactly** (verify 288 ok / 4
  checker-timeout / 0 fail — the promotion deal's 7-timeout scare did not
  recur). **kissat scored 294 v its recorded 296** (its own ±2 deal
  variance). Use THIS pair as the reference gap (−2) for same-host
  same-deal comparisons; the 07-29 kissat 296 run predates the balanced
  pinning and is a different deal.
- Unique-set shapes: solver12's 42 = engineered capabilities (php/counting
  x11, RoundRobin/MVRR gate-BVE x10 incl. the walk-giveup first-ever
  n17_d15, oddball_tto_zp x6, xor/tseitin x3, VdW x2, walk-era gains).
  kissat's 44 = 16x16 miters x9 (boothbit29/boothdadda29 flipped BACK to
  kissat this deal — wall-margin swing cells), starved BMC x7, pj giants
  x2, lottery tail. Both-timeout 64.
- **Tooling (commit 2cf3aec): NUMA-balanced worker pinning in
  feature_ablation.py (`numa_balanced_cores`) + run_kissat_full.sh
  (CORE_ORDER_STR, offset = window shift).** Old `range(jobs)` put 18/32
  workers on socket 0; new order alternates sockets over physical cpus
  (16+16 at 32 jobs), SMT spill only past 36. Verified live (taskset
  affinities one-per-socket; order recorded in kissat meta.txt).

## SESSION 18 (2026-08-08/09) — adaptive walk giveup PROMOTED: full-bench 291 → 292/400 (gate PASS, +1, a both-timeout first-ever); the walk vein is now CLOSED and the miter/near-miss levers are mapped exhausted

**Promoted: `SAT_WALK_STALL_GIVEUP=16`.** Walk cannot refute UNSAT; the
latch class mixes SAT walk-targets with UNSAT near-misses. Giveup abandons
walking once the best walk min-unsat stalls K=16 walks (RATE-based: must drop
≥1/64 to count as progress — marginal UNSAT creep counts as a stall),
returning the budget to CDCL. Byte-identical on SAT cells by construction.
A/B `log/abtest-cand-vs-base-2026-08-09-06-42-44` (gate PASS, zero
correctness failures, no SAT regressions): **292 v 291; +RoundRobin_n17_d15
(FIRST-EVER, both-timeout, kissat can't either) +mod2c; −RoundRobin_n18_d15
(same-family 355 s thin-margin wall swap).** Modest (+1, noise-band-adjacent)
but the gain is a deterministic first-ever and the mechanism is safe. Gap to
kissat now −4.

**THE EXHAUSTION MAP (this session's real deliverable — do not re-run these):**
- **Miter family (9 cells, biggest gap): SATURATED for flags.** Mid-search
  PROBE finds 0 units (23,480 attempts); BACKBONE 0; gate-BVE already on;
  vivify volume already at kissat parity (182k attempts) via deduce. Residual
  is pure CDCL trajectory quality (kissat refutes in 6M conflicts, we need
  >20M) — needs a decision/learning-quality mechanism, not a pass.
- **RoundRobin/near-miss via ELIM-ARMING: DANGEROUS, closed.** Forcing
  elim-yield arming (SAT_ELIM_PRODUCTIVE_MIN_PCT=10) on RoundRobin caused an
  UNBOUNDED non-CDCL runaway — probes ran ~14 h with SAT_LIMIT_WALL_SEC never
  firing (wall limit is CDCL-loop-only). Confirms the 2026-07-14 lottery +
  runaway warning; do not re-open without an elimination bound.
- **Walk latch 1M vs 500k: 500k CONFIRMED optimal.** A biased-subset screen
  favored 1M (14/19) but the full bench LOST 286 v 291 — the classic
  screen-doesn't-transfer trap. 500k stays.
- **tseitin_grid: research-scale.** The tseitin engine detects the full
  62,500-node grid component but proved=false — refuting 2D grid cycle
  structure is a proof-engine extension, with checker-cost risk (grid_n400
  already closed under the RAT-scan law).

## SESSION 17 (2026-08-06/07) — walk-latch second wave PROMOTED: full-bench 285 → 290/400 (gate PASS, +11/−6); gap to kissat −6; rbsat walk-solved

**Promoted defaults: `SAT_WALK_WARMUP_UNARMED=on` (new knob — kissat
warmup.c, scoped to never-armed walkers; the 2026-07-17 warmup NEGATIVE was
measured entirely on ARMED walkers, which stay byte-identical) +
`SAT_REPHASE_UNARMED_MIN` 1M → 500k (earlier latch = more walk runway).**

Full-bench A/B `log/abtest-cand-vs-base-2026-08-07-01-51-08` (gate PASS,
zero contradictions/correctness failures): **290 v 285. Gained 11:
ITC2021_Early_12 (834 s; solves in all 4 measured deals/arms since the
latch) + bp4_BC012_CSO_FPBEQ (both former kissat-only);
VanDerWaerden_pd_2-3-27_663 + lockchart-group2 x2 (FIRST-EVERS — nobody
solved these at 3600 s); rbsat-v1375 (the flagship wall-coin flipper of the
whole project, now WALK-SOLVED at ~7.5M conflicts in 4 consecutive
deals/arms — no longer a coin); reconf10 + frb80 (the 16b reroll losses
recovered); sum_of_3_cubes, valves-gates, oddball_57. Lost 6 walk-lottery
classmates (ER_400.apx_2, vmpc_28, oddball_56, bp4_IXA_LPI, mod2c,
oddball_19_4 — every one a documented member of the deep-unarmed rebalance
class; class-level net across 16b+17 = +9). PAR-2 955,537 v 993,612;
tier-2 conflicts flat. Checker-timeouts 3→7 — all big-proof UNSAT solves,
drat-trim BUDGET (none rejected); caveat class, watch it.**

Screen `log/abtest-warm-vs-thresh-vs-warmthresh-vs-base-2026-08-06-23-35-05`
(16 cells): warmthresh 12/16 v base 9/16 with each mechanism confirmed
alone (warm recovered frb80+VdW-23-accel; thresh captured ITC_Early_12 at
408 s + case6). dislog is NOT a latch target (it ARMS and already walks
4.3G steps — its gap is elsewhere). ITC_Late_10 still stands (walks but
does not convert). Validation: 756+5 tests, smoke 9/9, rbsat/MVRR
fingerprints digit-exact (both below the 500k latch).

## SESSION 16b (2026-08-06) — deep-unarmed rephase/walk latch PROMOTED: full-bench 281 → 286/400 (gate PASS, +9/−4, tier-2 −81.8M); SEVEN former kissat-only cells captured

**The discovery:** never-armed formulas structurally could not rephase or
walk — `config.rephase` defaults off and ONLY the arming/endgame paths set
`rephase_enabled`, so the walk-scale SAT class ran ZERO walk steps at any
depth (ITC_Early_12 / ITC_Late_10 / ER_400.apx_1 measured `rephases=0,
walk_steps=0` at 1.2M conflicts while kissat walks 100-360M steps there).
Corollary: `SAT_WALK_EFFORT_UNARMED=200` (promoted 14d) was DEAD CODE —
every rephase-enabled cell is `inprocess_aggressive`, so the unarmed branch
never executed anywhere.

**The promoted shape (commit after d6ea413):**
`SAT_REPHASE_UNARMED_MIN=1_000_000` default ON — enable the kissat-parity
rephase/walk cycle once a never-armed formula reaches 1M conflicts (the
endgame philosophy: perturb only losing trajectories; every unarmed cell
finishing below 1M is byte-identical BY CONSTRUCTION — rbsat
100001/196258/17,758,017 and MVRR 267,199 digit-exact) — plus
`SAT_WALK_EFFORT_UNARMED` default 200 → **50** (kissat walkeffort parity;
the screen measured 200 OVERWALKING: e50 9/14 v e200 6/14 v base 6/14 —
e200 lost vmpc/mod2c/sted2 that e50 wins).

**Full-bench A/B `log/abtest-cand-vs-base-2026-08-06-03-28-37`** (400x2
@3600 s, gate PASS, zero contradictions/correctness failures,
checker-timeouts 5→4): **cand 286 v base 281. Gained 9 (all SAT, all the
deep-unarmed class): ER_400_20_7.apx_1, sted2_0x0_n219, mod2c-rand3bip,
case8, fsf-300-354 x2 — all six former KISSAT-ONLY — plus 170223547
(walk-solves in 51 s right at the latch, was a coin timeout), bp4_BC012_AM,
mp1-Nb7T45. Lost 4: bp4_TCO (184 s, the documented deal coin), VdW-23
(walk-reroll — solved in the screen deal at 3358 s), reconf10_22 + frb80
(reroll losses inside the allowance). Tier-2 conflicts −81.8M across 47
changed both-solved cells; PAR-2 987,867 v 1,028,679.**

Screen (`log/abtest-e200-vs-e50-vs-base-2026-08-06-01-37-34`, suite
`benchmarks/unarmedwalk-2026-08-06`: 5 walk targets + 9 deep-unarmed
coin-class canaries): e50 9/14 v base 6/14, zero losses. ITC x2 and dislog
did NOT fall (still kissat-only) — the latch walks them now but they need
more than phase luck. Validation: 756+5 tests, smoke 9/9.

## SESSION 16 (2026-08-04/06) — NO PROMOTION: the late-armed re-screen space is now mapped; trail reuse PARKED after full evidence; five arms closed with data

**Verdict: defaults unchanged (identity fingerprints digit-exact all
session). The full-bench baseline stays 279/400 promoted; same-config deals
this week scored 276/279/280/281 — the ±2-4 variance calibration holds.**

What was measured (all screens on `benchmarks/miterded-2026-08-02` or
`benchmarks/reusefocused-2026-08-06`, full A/B on sat-comp-2025 400x2):

1. **Profile (gdb SIGINT sampler, boothdadda29 @2.5M conflicts): ~72% of
   wall is `propagate_impl`**; walk negligible; analysis ~14%. Wall/prop is
   only ~1.2x kissat (654 v 537 ns) — the earlier 49-v-26 "ticks/prop" read
   overstated (different accounting units). The real gap is props/conflict
   (194 v 108), dominated by restart re-descent (16,194 restarts / 2.5M
   conflicts, zero reuse) and DB/trajectory quality. SAT_WATCH_POOL and
   SAT_WATCH_INLINE_BIN are ALREADY default-on (stale doc comments say off).
2. **Banded vivify-sort and banded tier3: CLOSED** (screen
   `log/abtest-reuse-vs-sort-vs-tier3-vs-base-2026-08-05-00-20-53`: 7/23
   each v base 8/23 — rerolls without gains, even inside the 500k band with
   deduce active).
3. **Trail reuse (kissat restartreusetrail): PARKED with full evidence.**
   Wiring gap found+fixed (the miters arm via the VIVIFY-YIELD path,
   congruence_merges=0 — the knob only wired through the congruence path;
   commit a726262). Once live: screen WIN 9/23 v 8/23 (boothdadda29
   FIRST-EVER, every UNSAT miter −10-15% conflicts, canaries exact) but the
   full A/B (`log/abtest-cand-vs-base-2026-08-05-08-46-46`) LOST 280 v 281
   with tier-2 +10.9M: the SAME determinism that wins the UNSAT miters
   (boothdadda29 8,759,563 conflicts EXACT across two deals) deterministically
   REROLLS late-armed SAT cells (Circuit_multiplier24 — stable 4,992,637-conf
   trajectory in two deals — and DLTM_twitter774, both fat-margin losses;
   oddball_ttf/ER_400 +2-11M conflicts). The =focused variant (96% of miter
   reuse events are focused-mode) does NOT separate them
   (`log/abtest-focused-vs-both-vs-base-2026-08-05-22-30-51`): Circuit24
   still dies, boothdadda29's gain NEEDS stable-mode reuse, and the miters
   land between base and both. **Law: reuse's per-cell effect is
   deterministic but its sign is per-cell — there is no runtime discriminator
   separating late-armed UNSAT grinders from late-armed SAT-capable cells.
   Shipping it trades ~2 stable SAT cells for ~1 first-ever miter. Knobs
   banked: SAT_RESTART_REUSE_TRAIL_ARMED=on|focused (+_MIN band), both
   paths wired.** The aggressive cadence bundle (floor=1, margin=1.10 +
   reuse) is CLOSED outright (7/23).
4. **Ranked-item hygiene:** SWEEP_SUBST percent-mass (old item 3) PRUNED —
   SESSION 14c already measured SAT_SWEEP_SUBST=on flipping 0/6 on
   miters+uniqinv at 3600 s idle; a safety threshold cannot rescue a
   mechanism that does not fire on its target. mchess_20/rook decode
   (below) moved to a research arc.
5. **mchess_20 decoded (760 domino vars, pairwise AMO, 398 exactly-once
   cells): it IS the direct-php shape** — 200 var-disjoint black-cell covers
   v 198 white-cell AMO holes — but the counting core is PHP(200,198) and
   the inductive closer is ~3/4·H^4 ≈ 1.15G proof lines at H=198:
   infeasible. The family (mchess_20, rook-51/52/56, all nobody-solves
   except rook-51=kissat-only) needs a CARDINALITY-STYLE proof engine
   (totalizer/pseudo-Boolean simulation in DRAT) — a genuine research arc;
   naive totalizer LB/UB groupings do not compose in RUP (the LB needs the
   injective-mapping argument = php again). Park until someone designs the
   proof shape on paper first.

## SESSION 15 (2026-08-02/04) — banded vivify-deduce PROMOTED: full-bench 276 → 279/400 (gate PASS, A/B WIN +5/−2); backbone.c port landed and measured a no-op (free rider, default off)

Full-bench A/B `log/abtest-cand-vs-base-2026-08-03-10-13-35` (400x2 @3600 s
/16 GB/32 cores, simultaneous start, proofs verified, gate PASS, zero
contradictions / zero correctness failures):

| arm | solved | conf(own solved) | PAR-2 |
|---|:--:|--:|--:|
| cand (`SAT_VIVIFY_DEDUCE=on`, banded) | **279/400** | 532.9M | 1,041,267 |
| base (SESSION 14d defaults) | 276/400 | 554.7M | 1,057,324 |

**Gained (+5):** Circuit_multiplier24 (SAT 1917 s, FAT margin — a named
walk-scale gap cell), BubbleVsPancakeSort_7_6 (UNSAT 2274 s, FAT margin — gap
family), valves-gates + bp4_TCO_IXA_FPBLE_ZR + bp4_BC012_IXA_LPI (banked cells
base dropped this deal; retained/recovered). **Lost (−2):**
MVRoundRobin_n14_d10_v2 (base margin 82 s = thin wall coin) and
sum_of_3_cubes_37_bits_87 (REAL SAT reroll: base solved at its stable
894,247-conflict trajectory — identical in 3 prior deals — while deduce
changed cand's deal; expect it to flip back some deals). Tier-2: −14.7M
conflicts across the 37 changed both-solved cells; the mechanism cells all
shortened 10-30% (sqrt-mitern169 −1.43M, lec_mult −1.10M, boothbit29 −0.96M,
oddball_19 −3.94M, PancakeVsSelection_6_8 −3.61M, ER_400 −3.28M; worst
regression case11 +5.0M, still solved).

**What shipped (commits a1bbb5f, 2549801, + the promotion commit):**

1. **`SAT_VIVIFY_DEDUCE` default ON, banded** (the promotion). The kissat
   `vivify_deduce` reason-cone mechanism was built 2026-07-15 and shelved
   after the UNBANDED armed screen lost on EARLY armers (ibm +133% conflicts,
   oski20 +146 s). SESSION 15 added `SAT_VIVIFY_DEDUCE_ARMED_MIN=500_000`
   (the SESSION 14d reduce-law arming-time discriminator): deduce fires only
   where `inprocess_armed_at_conflict >= 500k`, so TT/oski/vex/oddball-class
   banked early armers are byte-identical BY CONSTRUCTION (miterded screen:
   all five canaries conflict-EXACT; identity refs digit-exact). Mechanism:
   boothdadda29 probe @2.5M conflicts — vivify hit rate 14.8% → 28.5%
   (kissat 34%), strengthened 27,491 → 53,823, wall 318 → 311 s.
2. **`src/backbone.rs` — full kissat backbone.c port, default OFF.**
   Stacked-probe failed-literal rounds over a private binary-implication-graph
   propagator, BIG-UIP analysis, RUP units through the learn_lucky path,
   kissat-parity flags/rounds/2%-effort. Tier-1 on the miter class: **ZERO
   units found — and kissat's own backbone finds 2 units there** (its 341k
   backbone ticks are cadence, not content). This re-confirms the 2026-07-15
   "killed without building" verdict buried in commit 038f9c1 — the ranked
   backbone item in earlier plan revisions was STALE. The pass is a
   zero-mutation zero-cost rider (bb arm conflict-identical to base on all
   23 screen cells): keep OFF; only re-arm if a family with a RICH binary
   implication graph (large edge count + failed-literal yield) shows up.
3. **Tier decomposition that found the real lever (boothdadda29, identical
   2.5M-conflict horizon):** solver12 318 s / 23.9G search ticks vs kissat
   145 s / 6.97G — 3.4x ticks (49 v 26 ticks/prop AND 194 v 108
   props/conflict) with kissat vivifying 6.5x more clauses (179,349 v
   27,491) and walking only 0.12% of wall. Deduce closes part of the
   hit-rate hole; the residual rate gap (still ~2x wall on miters) is the
   #1 remaining mechanism target.

Screens: miterded 4-arm (`log/abtest-ded-vs-bbded-vs-bb-vs-base-2026-08-02-
17-45-21`, 23 cells @3600 s): ded 8/23 v base 7/23 (gained sqrt-mitern169;
boothbit29 8.97M → 8.01M conf), bb ≡ base conflict-exact, bbded ≡ ded
conflict-exact (no antagonism, no backbone contribution). New suite:
`benchmarks/miterded-2026-08-02` (23 cells = miterarmed-2026-08-01 + sqrt169
+ lec_mult + boothdadda28/29 + mult16_22). Validation: 756+5 tests (+13 this
session), smoke 9/9, rbsat 100001/196258/17,758,017 + MVRR 267,199
digit-exact both flag states.

## SESSIONS 14b/14c/14d (2026-07-29..08-02) — pruned summaries

- **14d (280/400, +4/−0):** banded `SAT_REDUCE_FRACTION_ARMED` (+ `_MIN=500k`
  arming-time band — the discriminator SESSION 15 reused) un-blinded the
  reduce law on late-armed miters: FIRST-EVER 16x16 miter solve (boothbit29),
  + sqrt-mitern169/lec_mult/shuffling-1. Also `SAT_REPHASE_ARMED_ONLY=off` +
  `SAT_WALK_EFFORT_UNARMED=200`. Full text: rev 93ab682.
- **14c (277/400, +6/−0):** php-detector coverage — inductive PHP proof
  engine (Cook's ER reduction, ~H^4 lines v factorial), direct-php detection,
  AMO-connectivity partition voting, parse-time structure stash: 5 first-ever
  both-timeout hard-core cells (cliquecoloring/clqcl/fphp/rphp). Full text:
  rev d838757.
- **14b (271/400, +10/−4):** three runaway-pass bugs fixed (sweep-kitten
  unlimited budget, gauss ordering spin + 31 GB fill-in, mid-giant BVE 8 GiB
  arena doubling) + `SAT_REDUCE_FRACTION` default ON + thresholded `SAT_ELS`
  ON + root-pass scoping law (percent-mass decline-is-identity gates are the
  ONLY shippable root-pass shape). Full text: rev 416adae.

## RANKED PLAN (2026-08-12)

SESSIONS 15-19 took the bench 279 → 292-294-class with the gap to kissat at
−1 to −4 depending on deal. The flag frontier is mined; SESSION 19 proved
the "new capability engine" path works. Next leads, in order:

1. **kissat sweep.c pair-mechanics port (uniqinv40-class; the measured
   5x-yield ceiling).** See SESSION 20: latch + skip-transitive banked;
   the remaining delta is whole-environment retirement outside the tick
   engine + sustained per-sweep candidate yield. uniqinv40 is the
   acceptance test (kissat 51 s). Also re-screen the latch WITH the port.
2. **Sweepcount generalization (the fresh vein).** The engine handles
   perfect-matching color imbalance. Natural extensions, each a
   detector+battery delta on the now-proven core: (a) relaxed cells
   (at-most-one holes on the minority side); (b) odd-component parity
   (non-bipartite perfect matching — same frontier sweep, parity
   invariant); (c) exactly-k cells via wider bands. Scan any new bench for
   EO-cover shapes before building blind — THIS bench held only mchess_20.
2. **Medium-1800 re-baseline (bookkeeping, OVERDUE — six promotions since
   74/100 at c469b03).** Standard medium A/B, current defaults vs
   all-new-flags-off.
3. **Checker-timeout proof-size watch (4 at-risk UNSAT solves).** Watch
   only; sweepcount adds no exposure (291k lines verify in ~116 s).
4. **Miter CDCL trajectory quality (9 cells; hardest, highest ceiling).**
   Flag levers EXHAUSTED (SESSION 18 map). Needs decision/learning-quality
   work. Deep, risky, no clean probe.
5. **Walk vein: CLOSED** (latch 500k + warmup + effort 50 + giveup 16
   promoted; the deep-unarmed lottery is a managed surface).
6. **Starved hwmcc/BMC + RoundRobin elim-arming + par32/dubois XOR
   recovery:** CLOSED (SESSION 19: par32 honest-decline, dubois structure
   destroyed by transformation; rooks are balanced — not sweepcount-shaped).

## Current state

- HEAD: SESSION 19 promotion (after 09a271b).
  **Full-bench 3600 s baseline: 293/400-class** (freshest TSV =
  `log/abtest-cand-vs-base-2026-08-11-19-37-55/cand/results.tsv`: 293 with
  mchess_20 in and two thin coins out that deal; the same defaults are
  292-294 across recent deals). kissat 4.0.4 reference: **294/400
  same-host same-deal 2026-08-10**
  (`log/kissat-full-20260810-073149/results.csv`) — **gap ≈ −1**
  (was −25 at 14b). kissat can NEVER solve mchess_20, so the unique-set
  edge is now 43 v 43-ish. Lineage: 261 → 271 → 277 → 280 → 286 → 290 →
  292 → +mchess (paired gated A/Bs).
- Default surface SESSIONS 15-19: SAT_VIVIFY_DEDUCE=on + _ARMED_MIN=500k;
  SAT_REPHASE_UNARMED_MIN=500_000; SAT_WALK_EFFORT_UNARMED=50;
  SAT_WALK_WARMUP_UNARMED=on; SAT_WALK_STALL_GIVEUP=16; SAT_SWEEPCOUNT=on;
  SAT_BACKBONE=off; banded sort/tier3/reuse knobs off (closed).
- The deep-unarmed walk class is a managed LOTTERY SURFACE; the giveup
  (K=16) added a UNSAT-aware guard but the RoundRobin n17/n18 pair remains
  a wall-margin swap (~43M conflicts, both at the 3600 s wall). Judge walk
  members as class rebalance, not individual capability.
- **Same-config full-bench deal variance is now measured at 290-292** (four
  A/Bs this week: base arms scored 285/291/291, cand arms 290/286/292).
  A raw +1 is inside noise; the paired in-deal delta + a deterministic
  first-ever is the real signal.
- **Same-defaults deal variance at 3600 s full bench is ±2-4 solved**: the
  14d defaults scored 280 (08-01 deal) and 276 (08-03 deal) on identical
  config — weigh raw full-bench solved deltas accordingly (the paired A/B
  inside ONE deal is the real signal).
- **Medium-1800 s baseline: still NEEDS RE-MEASUREMENT (ranked item 4);
  last measured 74/100 at c469b03 (pre-bundle, pre-deduce).**
- Default surface added this session: SAT_VIVIFY_DEDUCE=on +
  SAT_VIVIFY_DEDUCE_ARMED_MIN=500000; SAT_BACKBONE=off (+ SCOPE/ARMED_MIN/
  EFFORT/TICKS/ROUNDS/MAX_ROUNDS knobs, all inert by default).
- Suites: `benchmarks/miterded-2026-08-02` (23 cells, miter targets + banked
  canaries — the standard screen for late-armed-band candidates),
  `benchmarks/frontier-2026-07-30` (38 cells), miterarmed-2026-08-01 (18).

## Standing traps (updated 2026-08-09 + carried)

- **SESSION 18:** WALL-LIMIT-ONLY-IN-CDCL bites hard — SAT_ELIM_PRODUCTIVE_
  MIN_PCT arming on RoundRobin ran 14 h with no wall stop (stuck in a
  non-CDCL elimination path). Any new mid-search-elimination trigger MUST
  carry a tick/resolvent bound or it can hang the whole bench. When probing
  at a wall limit, sanity-check `ps -o etimes` — a probe past its wall is
  wedged, kill it (bracket-trick pkill: `pkill -9 -f '[s]s pattern'`).
  Biased screens: a subset built from lottery cells will favor the config
  that helps THAT subset (1M latch 14/19) and mislead vs the full bench
  (1M LOST 286 v 291) — screen subsets must include the config's KNOWN
  casualties, and only the full 400-cell A/B decides.
- **SESSION 16b:** REACHABILITY-AUDIT LAW — before tuning any knob, trace
  its enable chain to the class it targets; three separate features this
  week (trail reuse, walk-effort-unarmed, unarmed rephase) were dead code
  on their target class because an upstream gate (arming path,
  rephase_enabled) never fired there. A `*_steps=0` or `rephases=0` stat
  on a cell the feature should touch is the tell. New walk-reroll flipper
  cells at 3600 s: VdW-23, reconf10_22, frb80-14-1 (join bp4_TCO/rbsat/
  case6/170223547* in the coin list; *170223547 now deterministically
  walk-solves at the latch — protect it).
- **SESSION 16:** when a knob screens conflict-IDENTICAL to base across a
  whole suite, suspect WIRING before verdict — trail reuse was only wired
  into the congruence arming path while its target family arms via the
  vivify-yield path. Check WHICH arming path a family takes
  (congruence_merges in the stats JSON) before scoping anything to
  "armed". Screen wins on UNSAT-grind suites do NOT transfer to the full
  bench when the mechanism also touches late-armed SAT cells — put
  known SAT casualties in the screen suite (reusefocused-2026-08-06 is
  the template). Stale doc comments lie about defaults (WATCH_POOL and
  WATCH_INLINE_BIN say "default off", both are ON) — trust env reads in
  Solver::new only.
- **SESSION 15:** the ranked-plan backbone item was STALE — commit 038f9c1
  (2026-07-15) had already killed it with kissat -s profiles; CHECK COMMIT
  MESSAGES of groundwork commits before re-ranking an old idea. Coin list
  additions: sum_of_3_cubes_37_bits_87 (SAT; stable 894,247-conflict
  trajectory when deduce-untouched, rerolls under any late-armed-band
  feature), MVRR_n14_d10_v2 (82-720 s margins at 3600 s, deep grinder at the
  wall). valves-gates is now ALSO a checker-timeout cell (verify caveat).
  4-arm screens at 3600 s on 23 cells run ~10.5 h wall, not ~3 h — plan
  accordingly; 400x2 full A/B ran ~15 h with verification.
- **SESSION 14b (carried):** NEVER `cargo build` the solver dir while ANY
  feature_ablation run is live — build to a scratch CARGO_TARGET_DIR or copy
  the binary out first. `pkill -f` with a self-matching pattern kills your
  own shell — use the `[b]racket` trick. ELS threshold gates ONLY the root
  standalone pass. `SAT_WALK` env name is PARKED (denylist).
- **SESSION 14b (carried):** reduce-law deep-cell coin exposure at 3600 s:
  rbsat/case6/170223547-class. Judge as coins, not capability.
- **SESSION 14 (carried):** full-bench 3600 s and medium-1800 s are separate
  ledgers. `ulimit -v` kills on VIRTUAL memory. rc-6 = allocator abort.
  SAT_LIMIT_WALL_SEC honored only in the CDCL loop.
- **Carried (SESSIONS 4-13):** deal noise ±2 medium; conflicts deterministic
  across load, wall is not; marginal-cell TIMEOUT untrustworthy under 32-way
  contention (solves ARE trustworthy); flipper list rbsat / vex / oski15 /
  VdW-22 (+case6, 170223547, sum_of_3_cubes, MVRR-n14 at 3600 s); activity
  proxies mislead; FEATURES.md/CONFIG_SCHEMA.csv are STALE (read
  src/config.rs + main.rs env reads); results.tsv written at run END; stats
  JSON on stderr, timed-out runs emit none (SAT_LIMIT_CONFLICTS probes);
  heredoc scratch writes flake — use the Write tool; perf blocked (gdb
  SIGINT sampler); `rm -rf` guarded — timestamped scratch dirs.
- **Carried ER/proof laws:** RAT-scan law (verify cost = #definitions x
  maxVar); residue/retry law (never stream an aborted ER attempt);
  deletions are load-bearing; tseitin caps legacy; SAT_TSEITIN_SNAKE off.
- **Carried closed lines (do not reopen without new mechanism):**
  starved-cell tick-cadence pipeline; unscoped root ELS/PROBE/SWEEP_ROOT
  defaults; SAT_ELIM_DEF; vivify tier-split AS A STANDALONE (SESSION 15
  exception: may re-screen as deduce+tier3 inside the late-armed band,
  ranked item 1b); gbve-adopter rounds; units-only transitive; per-mille
  RANKING thresholds (percent-mass decline-is-identity gates are the
  exception); ramsey ER emission; st_659; SAT_BACKBONE default-on (zero
  yield everywhere measured — miters, and 07-15 Bubble/fixedband profiles);
  **SESSION 16 additions:** banded vivify-sort; banded tier3; armed restart
  cadence bundle (floor=1/margin=1.10); trail reuse default-on in ANY mode
  (both-modes AND focused measured — deterministic per-cell sign flips, no
  runtime discriminator); SWEEP_SUBST for uniqinv/miters (0/6 at 3600 s
  idle, 14c — threshold variants pointless when the mechanism never fires).

## solver12's capability edge (protect in rerolls)

New this session: **Circuit_multiplier24** (SAT 1917 s; kissat-only before),
**BubbleVsPancakeSort_7_6** (UNSAT 2274 s, fat margin). Carried first-evers:
MVRoundRobin_n14_d10_v2 (NOW A COIN — protect but expect flips),
RoundRobin_n18_d15, at-least-two-vmpc_28, rphp5_050/085, clqcl_40/50_6_5 + 5
cliquecoloring siblings (SAT_PHP_REFUTE, reroll-immune), xor_op x2
(SAT_GAUSS), tseitin_n188_d3, RoundRobin_n15-n17 + MVRR x3 (gate-BVE),
oddball-tto_zp x4 + TT_C496 + TT_C406 (endgame/arming; protected by the
500k bands), Kakuro-132, HCP-529, frb80-14-1, valves-gates (checker-timeout
caveat), oddball_13_5_ttf, battleship, bivium, gto_p60, contest04,
reconf10_22, blockpuzzle, VdW-23, sted2var, bp4_BC012_IXA + bp4_TCO_IXA
(deal-marginal), boothbit29 + sqrt-mitern169 + lec_mult_CvW + shuffling-1
(14d, now deduce-accelerated 10-16%).

## Where the evidence lives

- SESSION 15: `log/abtest-cand-vs-base-2026-08-03-10-13-35` (THE verdict),
  `log/abtest-ded-vs-bbded-vs-bb-vs-base-2026-08-02-17-45-21` (miterded
  screen), `log/miterded-screen-20260802-174521.log`,
  `log/fullbench-ded-ab-20260803-101334.log`; tier-1 probes in scratch were
  transient — key numbers recorded above and in the solver README entry.
- SESSION 14d/14c/14b: `log/abtest-cand-vs-base-2026-08-01-20-32-12`,
  `log/seedgate-s14c-confirm-2026-08-01-00-07-44`,
  `log/abtest-cand-vs-base-2026-07-31-06-41-31`.
- Mechanism deep dives: `plan/kissat-gaps.md` (NOTE: its backbone/probing
  "small ports" ranking is now measured-refuted for the miter class),
  `plan/gap-read-full-2026-07-30.md`, `plan/gap-read-2026-07-21.md`.
- SESSIONS 4-13 full text: git history of this file (up to 52a8f95);
  14b/c/d full text up to 93ab682.
