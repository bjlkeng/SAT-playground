# NEXT PLAN — 2026-08-24 (supersedes 2026-08-16; PRUNED)

One-file plan for the next clear context. SESSIONS 4-13 bodies live in git
history (`git log -p plan/next-plan.md` up to 52a8f95); SESSIONS 14b/14c/14d
bodies were pruned earlier — full text in revisions up to 93ab682. Where this
file contradicts an older revision, THIS file wins.

**START HERE:** read "SESSION 27b" (the sweep-prover quadratic KILL —
promoted, new absolute record 296/400), then "SESSION 27" (the kissat
causal-ablation GRID + the decisive unscoped-escalation NEGATIVE),
then "SESSION 26"
(dive2-scoped elimination-bound escalation PROMOTED — the
causal-ablation method that found it), then "SESSION 25" (dive-scoped
trail reuse), then "SESSION 24 NEGATIVES", then "SESSION 23" (engine
speed — the lit_vals mirror), "SESSION 22" and "SESSION 21" (the
dive-restart latches), then "RANKED PLAN", then "Standing traps".

## SESSION 27b (2026-08-23/24) — sweep-prover QUADRATIC KILLED, PROMOTED (flagless, identity-proven); NEW ABSOLUTE RECORD 296/400 both arms; the 3,100-3,500 s coin band converted to solid margin

**The find:** gdb-as-parent sampling (ptrace_scope=1 workaround; sampler
in scratch, method now proven) on wall-band cell bp4_BC012_CSO put 89%
of its wall (134/150 samples) inside `sweep::prove_facts_budgeted_opts`
— the model store was Vec<Vec<bool>> full snapshots, rescanned per
backbone candidate AND per equivalence pair (O(n² pairs × #models),
models growing on every kitten flip). This engine runs on EVERY
yield-armed sweep cell (the S20 latch class).

**The fix (commit 5027737):** incremental partition refinement over
XOR-normalized model signatures — `same_as_m0` answers the backbone
question, `class_id` equality answers the pair question, O(1) each,
O(n) refinement per new model, no stored models. Identical booleans at
every decision point ⇒ identical kitten call sequence ⇒ bit-exact
yields and trajectories. Flagless (identity-proven, S23 lit_vals
precedent).

**Paired quiet identity+speed proof:** bp4 conflicts/backbones/equivs/
solves ALL digit-identical (1,508,168 / 11 / 373 / 252,872); sweep-
prove wall 1,650 s → 60 s (27.5x); bp4 total 2,415 → 759 s (−69%).
dislog 1,547 s on its digit-exact 4.94M trajectory (−45% v in-gate
2,825 s). HCP-446 2,568 s standalone (−4%; small sweep share).

**Gate (frozen-snapshot method, pre-fix 95f6289 binary as baseline arm;
log/abtest-sweepfix-vs-s27old-2026-08-23-10-44-21, 400x2 @ 3600 s/
16 GB/32 cores): PASS. 296 v 296 solved — BOTH ARMS the highest count
ever recorded on this bench (previous best 294) — with ALL 295
both-solved cells conflict-IDENTICAL (100%), PAR-2 915,096 v 932,820
(−1.9%), zero correctness failures.** Median wall −4.9% (mean −6.1%)
across the 182 identical-trajectory cells >100 s. Monster margins:
manthey 3,256→694 s (−79%), bv_ILA_Piccolo_JALR 2,377→581 (−76%),
bp4_CSO 3,431→1,367 (−60%), oski15 −35%, sted1 −28%, grs-32-64 −26%,
oddball_29/19 −26/−16%, bp4 family −22-23%, RR_n17 3,110→2,868. Wall
coins +reconf10_22 / −valves (net 0). dmu28 reproduced in-gate
(2,453 s). The band cells that flipped as coins in every recent deal
(manthey/bp4/bwo/sqrt169/RR_n17) now carry 400-2,900 s margins —
solved-count on median deals should read ~296-297.

**Method law (add to the toolbox):** profile the WALL BAND, not just
the gap cells — a quadratic in a niche pass taxed 15+ solved cells
invisibly for months (sweep_prove_nanos existed as a stat all along;
nobody compared it to wall). Check `*_nanos` stats against elapsed
before hunting micro-optimizations.

## SESSION 27 (2026-08-22/23) — NO PROMOTION: the kissat mechanism GRID mapped (the session's durable deliverable); unscoped COMPLETE-round escalation measured a decisive bench-scale LOSER (call 286 / cnod 285 v base 291); two standalone first-evers banked

**The grid (reuse it before believing any "needs a port" claim):** 16
kissat-only cells x 12 kissat single-mechanism ablations
(scratch grid_results.tsv; key rows preserved here). ELIMINATE depth is
load-bearing on 9+ cells: b18 / grs-32-128 / pj2016 noelim=TIMEOUT,
SAT_dat 4.8x, goldcrest 2.4x, myciel6 1.8x, oisc/x-epic 1.5x. VIVIFY
secondary (fixedbandwidth 3.0x, ncc_21015 2.6x, goldcrest/oisc 2.0x,
b18 1.9x). CHRONO: nla-dijkstra 2.9x, x-epic 2.2x, pj TO — but OUR
guarded SAT_CHRONO=on does NOT reproduce it (4 standalone probes all
timeout; a faithful chrono re-port is the prerequisite to even test).
uniqinv40 = nosweep 28.5x + nosubst 4.1x (the S20 sweep-port arc,
confirmed and sized). pj2016 is composite (noelim AND nocongr AND
nochrono all TO). oddball_24_4: every kissat mechanism REMOVED makes
kissat FASTER (novivify 0.1x) — over-inprocessed there; not our story.

**The negative (do not re-run — this closes the July question at bench
scale):** SAT_ELIM_BOUND_COMPLETE_ALL (complete-round escalation for
every armed round; b18's congruence-armed trace reproduces the S26
bve_grow=0 stall verbatim, and the mechanism probe was spectacular —
bound 16, 85% eliminated, wall −34% at 2M conflicts). Standalone it
converted ncc_21015 (2,714 s) and grs-32-128 (3,606 s) FIRST-EVER, and
in-gate it converted cfi-rigid-t2 (2,893 s, first-ever). But the 3-arm
gate (log/abtest-call-vs-cnod-vs-base-2026-08-22-12-20-56, 400x3 @
3600 s/16 GB/32 cores) is unambiguous: call +2/−7 = 286, cnod (TT
shielded) +1/−7 = 285, base 291. The losses are the banked fragile
surface itself: VexRiscv (base 2,131 s FAT margin — the wire-cell
casualty July predicted), dislog, sqrt-mitern169 (the S26 capture),
TT496 (call only — the cnod shield DID work), goldcrest-11, ncc-2_17,
RR_n17/bp4 coins. 64/284 both-solved cells conflict-differ (v S26's
1/286). **Law: complete-round escalation is shippable ONLY inside
narrow structural bands; the armed surface at large carries too many
banked captures. The knob stays in tree (on|nodecision) for future
band-scoped reuse.** ncc/grs remain standalone-only capabilities
(wall-marginal in-gate); cfi-rigid-t2 is a one-cell prize with no
structural band yet — do NOT gerrymander one.

Also closed this session: the unarmed-inprocessing hypothesis for the
BMC class — b18 arms at 200k conflicts, SAT_dat at 1, goldcrest arms
too; the flywheel never fires there. Flywheel vivify + binfrac-ceiling
knobs (SAT_UNARMED_FLYWHEEL_VIVIFY / _MAX_BINFRAC) are in tree,
default-inert. TT392 canary byte-identical-fast under escalation.

## SESSION 26 (2026-08-21/22) — dive2-scoped kissat elimination-bound escalation PROMOTED: gate WIN 291 v 288 (PASS, zero correctness failures); dmu28 FIRST-EVER; the elimination-depth gap on miters is CLOSED at its root

**The method that cracked it (reuse this): ablate KISSAT, not just us.**
S24 closed the "elimination-flag vein" saying depth needed a whole-loop
port. Wrong — one causal probe pair settled it in 10 minutes: at a paired
2.5M-conflict horizon on m29, kissat `--eliminatebound=0` drops to 46%
eliminated = EXACTLY our shipped 47%, props 2.1x, wall +30%;
`--definitions=0` costs only ~5pp. The doubling additional-clauses bound
(`set_next_elimination_bound`) IS kissat's depth. Our own machinery
(SAT_ELIM_BOUND_COMPLETE, built 2026-07-18, default off after the
QG7/Pancake/TT casualties) only needed the RIGHT SCOPE: the band-2 dive
latch, which excludes every July casualty by construction. Second
supporting identity: our props-per-active-var equals kissat's (0.147 v
0.155/conf) — the whole miter props gap was active variables.

**Shipped (9a6807e groundwork + promotion): `SAT_ELIM_BOUND_DIVE2=on`
default.** COMPLETE-round escalation 0→1→2→4→8→16 for formulas with
`restart_dive2_armed_at > 0`. Shipped zero-yield rule stalls at 0-2 on
miters (trace: 8 rounds to 2.2M conflicts, 16k count-bound rejects,
budget never exhausted). With escalation m29 eliminates 1,214→1,669 of
2,575 vars (65%; kissat 73%), props −17%, post-elim literals +43%,
per-prop +22% → wall FLAT at fixed conflicts; the win is trajectory
(fewer conflicts on the collapsed formula).

**Gate (`log/abtest-dive2elim-vs-base-2026-08-21-12-43-45`, 400x2 @
3600 s/16 GB/32 NUMA-balanced): WIN 291 v 288, PASS, PAR-2 975,122 v
985,472, zero contradictions. 285/286 both-solved conflict-IDENTICAL —
the delta is exactly the band.** Mechanism: +dmu28 (UNSAT 2,528 s,
1,072 s margin, FIRST-EVER anywhere; S22 verdict "does not convert even
quiet" now dead), +bwo_bit29 (3,035 s; screen conflicts digit-exact
7,308,747), +sqrt-mitern169 (3,600.0 s at-wall), −m29 (in-band
deterministic reroll 8.59M→10.16M conf; keeps converting standalone
2,269 s ⇒ in-band coin upside on quiet deals). Coins +MVRR +ncc
−bp4_BC012 (127 s margin). PvS_6_6 the only conflict-diff both-solved
cell (+1.47M conf, 412 s, 3,188 s margin). 4 checker-timeouts — the
standing miter proof-size watch class, proofs valid.

**Killed this session (do not re-run):**
- Definitions under bound 16 (SAT_ELIM_DEF + DIVE2): +4 eliminations,
  def_gate_eliminated=0, reject_defcap 8,318 — S24's dead verdict holds
  in the new context too.
- Bound caps 4/8 (SAT_ELIM_BOUND_DIVE2_MAX, knob banked in tree): no
  m29 rescue (max4 2,710 s/9.60M; max8 timeout@4,000 under load) — 16
  is the shape; m29's loss is trajectory reroll, not densification.
- Vivify-throughput on miters: 3x armed budget = 5.4x attempts, 3.6x
  strengthened, conflicts DIGIT-IDENTICAL (10,156,558) — vivify volume
  is trajectory-null on this class; kissat's 179k vivified there is not
  load-bearing. Closes ranked-lever "vivify throughput".
- gdb -p sampling: blocked (ptrace_scope=1); run gdb as parent.

**Band-2 membership truth (SAT_DEBUG_DIVE census, 2026-08-21):** all 12
in-bench 16_16 miters latch; BvP_8_4/9_4/8_6 latch; **BvP_7_6 does NOT
(pre_binfrac 0.283 < 0.30)** — the S15 capture is out of scope by
construction; PvS_6_6 latches (the only solved armed in-band cell).
Trap: the DIVE-CHECK line's second field is post-preprocess binfrac; the
latch reads the THIRD (pre_binfrac). Timeout runs emit no stats JSON.

## SESSION 25 (2026-08-20/21) — dive-scoped trail reuse PROMOTED: gate WIN 292 v 290 (+3/−1, m29 captured IN-BAND); the S16-parked lever revived for the floor-2 latch classes

Speed-round follow-ups measured: pooled binary-implication arena
(WatchPool design, order-identical) = NULL (−0.8% on rbsat; Nested
per-list Vecs are parse-order-localized already; banked default-off as
SAT_BIN_POOL), watch min-cap 16 = null. The WIN: the dive latches
restart every ~30 conflicts, so trail re-propagation dominates
(m29 191 props/conf v kissat 107). SAT_DIVE_REUSE_TRAIL=on (now
default) enables reuse on focused AND stable restarts for latched cells
only — the focused-only form measured a wash; the both-mode form
reproduces the global-reuse trajectories DIGIT-EXACT (m29 8,588,278,
ob19 3,173,688). Standalone: m29 −9-13% wall −5% conflicts, bwo_bit29
3,283 -> 2,069 s, ob_19_4 -> 930-979 s. Gate
log/abtest-reuse-vs-base-2026-08-20-19-30-08: WIN 292 v 290, PASS,
+m29 (in-band, 3,317 s in-gate) +cfi-rigid-t2/+ncc coins −RR_n17 coin,
6 in-band conflict-diff cells only. WATCH: m29/bwo reuse-proofs hit
verify=checker-timeout under gate load (valid proofs, drat budget) —
the standing proof-size watch now covers the miter class.

## SESSION 24 FINAL (2026-08-20) — CLEAN RE-BASELINE 294/400 CONFIRMED with the strongest cell composition ever recorded; elimination-flag vein closed

**The clean single-arm quiet re-baseline
(log/abtest-clean-2026-08-19-21-00-35): 294/400** — matching the
best-ever count with, for the FIRST time, every promoted capability on
one deal simultaneously: ob_19_4 1,396 s (S21 latch), **both S22 miters
in-gate (m29 3,389 s, bwo_bit29 3,283 s)**, manthey 3,248 s (S23 speed
capture), MVRR 3,365 s, RoundRobin_n17 2,999 s, bp4_BC012 3,195 s,
dislog 2,424 s, sqrt169, full TT bank (TT395 129 s / TT496 1,108 s —
bug-free). Head-to-head: ours 294 v kissat 294 same-suite, unique sets
43 v 43; the kissat-only miter family is down from 9 to 7. The 285-289
readings of 08-17/18 are CONFIRMED as bug + queue-pressure artifacts.

Next-session lever prepped: **Flat binary-implication layout**
(BinaryImplications::Flat exists, complete for reads, order-identical =
trajectory-safe, but never constructed in production — Nested
pointer-chase runs everywhere). Needs a hybrid overflow segment to cap
Flat add_edge O(n) inserts for learned binaries. Expected 2-6% on
binary-heavy cells; also SAT_BUMP_SORT_CACHE default-off (unmeasured
recently). Combine with a re-profile after lit_vals.

## SESSION 24 NEGATIVES (2026-08-19) — the elimination-depth flag vein is CLOSED; three dead ends measured and recorded

1. **Band-3 dive latch (myciel6/grs/mod4block): KILLED as gerrymander.**
   Trigger-time shapes are heterogeneous (myciel6 density 3.9, mod4block
   206, grs 2.9; grs pre_binfrac is 0.299 not 0.09) and sted2 (the
   never-perturb cell, 0.677/0.004) sits 0.007-collapse from myciel6
   (0.658/0.011). No clean structural band exists. Do not retry without
   a NEW discriminating axis.
2. **Root gate-aware BVE (SAT_GATE_BVE_SCOPED): ALREADY DEFAULT-ACTIVE**
   on small formulas via profile selection — plain-default runs show
   gate_bve_scoped_adopted=1 (m29 e0=963 -> e1=1000 root elims). The
   87-adopter full-bench scan measured the STATUS QUO, not a candidate.
   Trap: config-struct literal defaults are NOT the shipped defaults;
   check profile overrides before re-measuring any flag.
3. **SAT_ELIM_DEF (kitten semantic definitions in armed rounds): DEAD as
   default.** Full probes: uniqinv40prop still TIMEOUT (the definition
   hammer does NOT crack the SESSION-20 flagship), RoundRobin_n18 still
   TIMEOUT, m29 +5% conflicts (9.51M v 9.04M, no gain), HCP-446 rerolled
   (23.5M v 21.9M, still SAT), bp4_BC012's apparent conversion is its
   known near-wall trajectory (conflicts 8,749,492 digit-identical to
   the speed-A/B gain — not an elim_def effect), and **dislog TIMED OUT
   (fragile-bank kill)**. Default-off re-confirmed with fresh evidence.
4. Elimination-depth conclusion: the m29 57% v kissat 74% gap is NOT
   closable by existing flags (root gates on, armed ext-gates on,
   definitions harmful). kissat's depth comes from its
   eliminate/substitute/vivify whole-loop interleave — the ranked
   sweep-port arc, not a knob. BVE-reject trace confirms 100% of
   rejections are the resolvent-count bound (SAT_TRACE_PREPROCESS_DETAILS
   elim_round lines: reject_count_bound=all, clslim/defcap/budget=0).

## SESSION 23 (2026-08-17/18) — lit_vals per-literal value mirror PROMOTED (~9% engine speedup, trajectories digit-exact); SESSION 22's banked miters CONVERTED IN-GATE (m29 3,260 s, bwo_bit29 3,489 s, both first-ever)

The round's brief was speed/efficiency only. Profile (gdb sampler, m29):
propagation ~60% of leaves with the hot loop already saturated (blocking
literals, inline binary tags, flat watch pool, prefetch, kissat
`searched`). The remaining gap was representational: lit_value()
recomputed sign logic with two branches per call vs kissat's values[]
single load. Change: per-literal mirror (pos/neg slots adjacent via
lit_to_index), maintained at the 4 assignment-mutation sites + rebuild
helper at bulk-overwrite sites (capture_sat_model, lucky trials, test
resets — the debug_assert-on-every-call caught both hidden paths during
development; full 761-test debug suite runs with it active). Unchecked
indexed load (bounds proven by construction).

**Measured:** rbsat probe −8.6% (alternating 3x: 14.60-14.83 →
13.31-13.45 s); m29 paired quiet 2,300 → 2,085 s (−9.4%). **Full-bench
(twin identical arms, log/abtest-speed-vs-speedb-2026-08-17-12-17-43):
twins byte-identical (289 v 289, conflicts 580,784,015 = 580,784,015);
vs the pre-change TSV all 285 shared solved cells conflict-IDENTICAL.
Gains vs yesterday: m29 (3,260 s in-gate, conf digit-exact to the
band-2 probe) and bwo_bit29 (3,489 s) — the SESSION 22 bank cashed —
plus MVRR/sqrt169 coins. The 7 raw cross-day losses ALL failed in BOTH
identical twins (deal-wide drift on a weak deal; TT496 documented
flipper, valves/lockchart/ncc/g2 thin margins, TT395/406 giant-cell
placement lottery) — zero candidate-attributable losses, also true by
construction. Mechanical cross-day gate line reads FAIL (289 v 292);
judged PROMOTABLE under "Judging Trades" and recorded as such.** Commit
7874e01.

Fleet effect: every cell in the 1600-1800 s band gains ~150 s of
margin; the 294-class defaults on a median deal should now read
~295-296.

**SESSION 23 FINAL (2026-08-19) — the "host drift" was mostly a BUG,
now FIXED (feeea27); the corrected same-deal old-vs-new A/B confirms
the engine win.** The first lit_vals build missed growing the mirror in
grow_variables: factor-introduced fresh vars made lit_value read OOB
via get_unchecked (UB) and killed the factoring-heavy SC25 Timetable
class — which is why the 08-17/08-18 twin runs (both the buggy binary)
"lost" TT395/406/496/g2 deal-wide and absolute scores read
292 → 289 → 285. Detection: the same-deal old-vs-new A/B
(log/abtest-new-vs-s22-2026-08-18-13-39-28) showed new losing TT395
which s22 solved in 147 s — impossible for a strictly-faster
identical-trajectory binary; quiet reruns confirmed crashes.

**Corrected A/B (log/abtest-newfix-vs-s22-2026-08-19-03-09-59,
same-deal simultaneous, frozen 2d0d071 snapshot as baseline): 290 v
290 solved, 289 both-solved cells with ZERO conflict diffs, newfix
6.4% faster wall (faster on 211/289), PAR-2 WIN 973,924 v 986,228;
bug-class all recovered and faster (TT395 158 v 160 s, TT406 347 v
354, TT496 1,306 v 1,376, g2 3,195 v 3,408); +MVRR (3,397 s, its
second reproduction) / −ncc (92 s-margin coin).** Across the two
same-deal A/Bs the speed change converted in-gate: MVRR (twice),
RoundRobin_n17 (3,244 s), bp4_BC012 (3,393 s), manthey (3,330 s,
kissat-only FIRST-EVER), plus m29 (3,260 s) and bwo_bit29 (3,489 s) on
the 08-17 comparison — all near-wall cells the old engine cannot reach.

**Measurement rules going forward:** (1) same-deal paired arms are the
only valid comparison (the frozen-snapshot method: build old commit in
a git worktree, place binary in an untracked solver/00-*-snapshot dir
with a no-op build.sh, add a temporary CONFIG_MAP entry — the entry is
NOT committed; recreate on demand). (2) Absolute cross-day counts
remain suspect until a quiet-host re-baseline; the pre-bug paired
lineage stands at the 294-class with the S23 engine strictly faster.

## SESSION 22 (2026-08-16/17) — band-2 dive latch PROMOTED (gate PASS 292 v 291): the 16x16 miter class now runs kissat-parity restarts; miter conversions banked standalone, in-gate blocked only by contention wall

Second application of the SESSION 21 method, on ranked item 1 (the 9
kissat-only 16x16 miters). Mechanism measured on m29
(booth_dadda_origin_and_and_dadda_origin_bit29, kissat 587 s / 8.08M
conflicts / restart interval 43, sweep+congruence+factor all negligible):
pure cadence gap. floor 2 + margin 1.10 converts m29 standalone 2,648 s /
10.1M conflicts (trajectory parity) and booth_wallace_origin_bit29
2,987 s under 13-way load. **Band 2** (in maybe_arm_dive_restarts):
collapse in [0.15,0.35] AND parse binfrac in [0.30,0.50] AND initial
clauses <= 30k — exactly 12 in-bench 16_16 miters + 3 BubbleVsPancake
(all base timeouts) + 5 solved small cells (4 solve at 0 conflicts
pre-trigger; PancakeVsSelection_6_6 arms and improves 2.24M -> 1.70M).
SC25 Timetable excluded by the size cap. No slow-EMA window in band 2
(screen: harmful on miters). **Gate
log/abtest-dive2-vs-base-2026-08-16-13-17-06: WIN 292 v 291, PASS, zero
correctness failures. Honest trade note: the solved +2/−1 (valves +,
bp4_BC012_IXA +, MVRR_n14 −) are ALL out-of-band identical-trajectory
wall coins (MVRR baseline margin 64 s, documented flipper family); the
mechanism content is the PvS tier-2 drop + the banked miter class.**

Measured and rejected this session: walk suppression on band-2 armed
cells (m29 3,157 s / 11.0M vs 2,648 s / 10.1M — the walk's warm phases
guide circuit search); slow-EMA window in band 2. dmu28 (kissat 716 s)
does not convert even quiet — the family ratio (~4x kissat wall) puts
kissat<=700s members at our wall; **the next miter lever is throughput,
not cadence** (m29: 191 props/conf vs kissat 107, 22.7k ticks/conf,
180k live learned clauses on a 2.5k-var formula; kissat eliminates 74%
of vars vs our 57%).

STANDING UPSIDE (no action needed): m29 and bwo_bit29 flip in-gate on
quieter deals (the dislog pattern); ob_24_4/26_4, baseballcover12,
3x BubbleVsPancake are additional in-band rolls.

## SESSION 21 (2026-08-14..16) — dive-restart latch PROMOTED: full-bench 293 → 294/400 (gate PASS, +1/−0, oddball_19_4 FIRST-EVER); the restart-cadence gap vs kissat is now MAPPED and the global form is measured DEAD

**Core finding (mechanism, durable):** our focused-mode glucose-EMA
restart constants are all tamer than kissat 4.0.4 — interval floor 50+log
vs ~1, margin 1.20 vs 1.10, slow-EMA window 4096 vs 100,000. On fat-LBD
counting trajectories (oddball-ttf class: avg LBD 24-32 at level 40-52)
this yields ~460 conflicts/restart where kissat runs ~30, deep dives with
59-lit learned clauses, and 37x per-conflict tick cost — the entire 57x
wall gap on oddball_19_4 (kissat 63 s / 2.75M conflicts). Restart parity
closes the trajectory gap to conflict-parity (3.0-3.5M).

**Global parity is DEAD as a default — measured, do not retry:** the
full-bench A/B (log/abtest-rpmf-vs-base-2026-08-14-16-30-16) LOST 286 v
294 (+8/−16). The gains included real targets (HCP-446, oddball_19/56/67,
lockchart-L190, TT495) but the losses gutted the SAT lottery bank
(bp4/lockchart/fsf/RoundRobin/bivium/VDW/mod2c/TT496). LBD fingerprints
at 100k conflicts show NO clean separation between gained and lost SAT
cells (TT495 63.9 in / TT496 61.1 out; lockchart-g1 83 in / g2 115 out;
oddball_56 137 in / _80 128 out) — pure trajectory lottery. Early-window
(100k-conflict) LBD also fails as a discriminator (ob_19_4 measures 20.6
there; the fat signature develops by ~1M).

**What PROMOTED (commit chain e8676d4 → 18aa624 → this): the structural
dive latch, SAT_RESTART_DIVE=on by default.** One-shot check after root
preprocessing: non-binary clause-mass collapse >= 0.77 AND parse-time
binary fraction in [0.50, 0.85]. Trigger-time truth (SAT_DEBUG_DIVE=on):
trio at collapse 0.782-0.834 / binfrac 0.708-0.718; nearest non-members
oddball_80 tto 0.745/0.986, ER_400 0.543/0.987, MVRR 0.308/0.996 — the
SAT-lottery families all carry binfrac >= 0.96 and are excluded by the
ceiling. Full-bench shape-scan (scripted, 400 cells): EXACTLY 10 in band
= 3 target timeouts (ob_19/24/26_4) + baseballcover12 (kissat-unsolved
too) + 3 SAT-at-0-conflicts cells + linked_list (6k conf) + ttf siblings
13_5/17_5. Latch = floor 2 + margin 1.10 + slow window 100k + kissat-style
bias-corrected EMA warmup (alpha_eff = max(alpha, 1/(n+1)), latch-only;
without warmup the pinned slow EMA thrashes: 6.9M vs 3.18M conflicts on
ob_19_4). **Gate (log/abtest-dive-vs-base-2026-08-15-12-18-53): WIN
294 v 293, +1/−0, promotion_gate=PASS, zero correctness failures, 396/400
cells conflict-identical; oddball_19_4 first-ever UNSAT in-gate (3.18M
conflicts), 13_5 improves 1.84M→1.69M, linked_list 6004→3248, 17_5
119k→826k (still 16-24 s, priced).**

Recorded negatives (do not repeat):
- ob_26_4 converts ONLY with latch + unarmed walk-min (2,174 s once);
  floor+margin and full-parity latches both time out even quiet. Walk-min
  as default endangers the walk bank — left out. It remains in-band
  upside on lucky deals.
- ob_24_4 never converted under any variant (kissat 789 s).
- HCP-446 walk-effort bracket (SAT_WALK_EFFORT_YIELD_ARMED, inert knob
  banked): shipped yield-armed effort 50 is already optimal — 1 fails,
  100 = 3,060 s, 250 = timeout vs 2,676 s at 50. The HCP conversion lever
  is NOT walk effort; it remains contention-margin (lower-contention
  scheduling or a faster collapse).
- myciel6 (12.0 LBD) and grs-32-128 (level 483) sit OUTSIDE the band and
  converted only under the global-parity env (standalone 2,690-3,391 s);
  candidates for a second, different discriminator if one exists — do NOT
  widen this band to chase them (VDW at 22.0/33.1 is adjacent).

Free riders in tree (inert): SAT_RESTART_FLOOR / SAT_RESTART_MARGIN
(global restart knobs, defaults unchanged), SAT_WALK_EFFORT_YIELD_ARMED,
SAT_DEBUG_DIVE + SAT_RESTART_DIVE_COLLAPSE/BINFRAC tuning knobs.
Validation: 761+5 tests, smoke 9/9, rbsat 100001/196258/17,758,017
digit-exact dive on AND off, no-fire verified on VDW/MVRR/TT496.

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
one-cell overfit, forbidden. **VERDICT REVISED (SESSION 20g, same day): PROMOTED after the
non-arming verification.** The three calibrated-A/B losses were each
PROVEN non-arming (sqrt169 probe = 7 equivs v the 1000 floor;
oddball_19_4 and reconf10 zero ARMED lines through 3M conflicts) —
byte-identical AND wall-identical trajectories in the cand arm, i.e.
pure contention coins by construction, all three documented flippers.
Under "Judging Trades" (N=3 coins with written justification v
mechanism-validated capability) the trade PROMOTES:
SAT_SWEEP_YIELD_ESCALATE=20 + SAT_SWEEP_YIELD_MIN_EQUIVS=1000 default
ON (probe stays off). dislog_a14 (kissat-only) is the durable capture —
1680 s at the shipped default, in-gate both A/B deals; HCP-446 remains
a standalone-only capability (2676-2730 s, wall-borderline in-gate) and
16_2-class collapses are upside. Fingerprints digit-exact under the
default. Next-session notes unchanged: whole-loop sweep port (uniqinv40
acceptance) and lower-contention scheduling would both convert HCP.

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

## RANKED PLAN (2026-08-23)

SESSIONS 15-26 took the bench 279 → ~295-class; SESSION 27 mapped the
remaining kissat-only set mechanism-by-mechanism and killed the
unscoped-escalation shortcut. The three productive shapes stand: NEW
ENGINES (sweepcount), SCOPED-PARITY LATCHES (dive-restart, dive2-elim),
and CAUSAL KISSAT ABLATION (grid first, build second). Next leads:

1. **Faithful chrono re-port (kissat backtrack.c/decide.c parity).**
   The grid's cleanest untried multi-cell lever: kissat loses 2.9x on
   nla-dijkstra, 2.2x on x-epic, TO on pj2016 without chrono; our
   guarded SAT_CHRONO does not reproduce it (S27: 4 probes dead). A
   faithful port (chronolevels=100 semantics, reuse rather than our
   current−1 guard) is a bounded engine arc with a measured 3-cell
   target list; probe nla/x-epic standalone as acceptance.
2. **kissat sweep.c pair-mechanics port (uniqinv40-class).** Sized by
   the grid: nosweep 28.5x + nosubst 4.1x. uniqinv40 (kissat 51 s) is
   the acceptance test; sweep_repr mid-environment substitution is the
   known missing piece (S20b).
3. **Band-scoped escalation reuse.** The COMPLETE_ALL knob is in tree;
   if a NEW structural discriminator emerges for the b18/grs/ncc
   class (congruence-armed BMC shape: b18 binfrac 0.25, 167k vars),
   escalate inside it only — the S26 shape. Do not re-run unscoped.
4. **Miter family residual (5 kissat-only).** All single-mechanism
   levers now closed (S26/S27). Residual = composite throughput;
   revisit only with a genuinely new engine idea.
5. **Medium-1800 re-baseline (bookkeeping, OVERDUE — eight promotions
   since 74/100 at c469b03).**
6. **Checker-timeout proof-size watch (standing, miter class).**
   dmu28's in-gate proofs verified in S27's gate (35+ min each under
   load) — still valid, still slow.
7. **PARKED/CLOSED: sweepcount generalization; walk vein;
   starved-BMC/XOR; factor.c (DONE, in tree); unscoped escalation
   (S27); definitions (S24+S26); vivify volume on miters (S26);
   flywheel for BMC (S27 — the class arms).**

## Current state

- HEAD: SESSION 27b promotion (sweep-prover quadratic fix, flagless).
  Freshest deal: **296/400 — the best count ever recorded**
  (`log/abtest-sweepfix-vs-s27old-2026-08-23-10-44-21/sweepfix/results.tsv`;
  the pre-fix arm read 296 same deal too — the fix's contribution is
  the −4.9% median wall / the coin band converted to margin; PAR-2
  915,096). kissat same-host reference: 294 (2026-08-10). WE ARE NOW
  AHEAD of kissat on paired-deal counts. Lineage: 261 → 271 → 277 →
  280 → 286 → 290 → 292 → 293 → 294 → ~295 (S26) → 296 (S27b deal).
- kissat 4.0.4 reference: **294/400 same-host 2026-08-10**
  (`log/kissat-full-20260810-073149/results.csv`). Remaining kissat-only
  families after S26: 16x16 miters (5, was 7: dmu28/bwo29/sqrt169 out,
  m29 back in), oddball residue (4), TT (2), lockchart (2), grs (2),
  pj (2), b18/b19 BMC (2), singletons (rook-51, par32-2, cfi-rigid,
  oisc, ER_400, uniqinv40, ...). Lineage: 261 → 271 → 277 → 280 → 286 →
  290 → 292 → 293 → 294 → ~295-class (S26 +dmu28/+bwo/+sqrt169 −m29),
  all paired gated A/Bs.
- Default surface SESSIONS 15-26: SAT_VIVIFY_DEDUCE=on + _ARMED_MIN=500k;
  SAT_REPHASE_UNARMED_MIN=500_000; SAT_WALK_EFFORT_UNARMED=50;
  SAT_WALK_WARMUP_UNARMED=on; SAT_WALK_STALL_GIVEUP=16; SAT_SWEEPCOUNT=on;
  SAT_SWEEP_YIELD_ESCALATE=20 + SAT_SWEEP_YIELD_MIN_EQUIVS=1000;
  SAT_RESTART_DIVE=on (S21); SAT_RESTART_DIVE2=on (S22);
  SAT_DIVE_REUSE_TRAIL=on (S25); **SAT_ELIM_BOUND_DIVE2=on (S26)**;
  SAT_BACKBONE=off; banded sort/tier3/reuse knobs off (closed).
- The deep-unarmed walk class is a managed LOTTERY SURFACE (unchanged);
  the global-restart-parity A/B is the freshest, sharpest measurement of
  that surface: +8/−16 on identical mechanisms. Judge walk members as
  class rebalance, not individual capability.
- **Same-defaults deal variance at 3600 s full bench is ±2-4 solved**;
  the paired A/B inside ONE deal is the real signal. SESSION 21's gate
  is maximally clean by construction: 396/400 cells conflict-identical,
  the delta is exactly the in-band cells.
- **Medium-1800 s baseline: still NEEDS RE-MEASUREMENT (ranked item 5);
  last measured 74/100 at c469b03.**
- Suites: `benchmarks/miterded-2026-08-02` (23 cells, the standard screen
  for late-armed-band candidates — used to pick rpmf in SESSION 21),
  `benchmarks/frontier-2026-07-30` (38), miterarmed-2026-08-01 (18).
- In-band dive cells for future deals (upside, no action needed):
  oddball_24_4 (kissat 789 s), oddball_26_4 (needs walk-min too),
  baseballcover12 (kissat-unsolved; a first-ever candidate).

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

New SESSION 26: **dmu28 = 16_16_default_mapped_ultra_and_and_dadda_mapped_bit28**
(UNSAT 2,528 s in-gate, 1,072 s margin, FIRST-EVER), **bwo_bit29** and
**sqrt-mitern169** (in-gate miter converts; sqrt169 at-wall). **m29 is now
the in-band COIN** (converts standalone 2,269 s; needs a quiet deal
in-gate) — do not read an m29 timeout as a capability loss while
SAT_ELIM_BOUND_DIVE2 is on. Carried: **Circuit_multiplier24** (SAT 1917 s;
kissat-only before), **BubbleVsPancakeSort_7_6** (UNSAT 2274 s, fat
margin; does NOT latch band 2 — protected from dive2-scoped features by
construction). Carried first-evers:
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
