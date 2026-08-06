# NEXT PLAN — 2026-08-06b (supersedes 2026-08-06; PRUNED)

One-file plan for the next clear context. SESSIONS 4-13 bodies live in git
history (`git log -p plan/next-plan.md` up to 52a8f95); SESSIONS 14b/14c/14d
bodies were pruned earlier — full text in revisions up to 93ab682. Where this
file contradicts an older revision, THIS file wins.

**START HERE:** read "SESSION 16b" (the promotion), then "SESSION 16" (the
mapping session whose negatives set it up), then "RANKED PLAN", then
"Standing traps".

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

## RANKED PLAN (2026-08-06b)

1. **Medium-1800 re-baseline (bookkeeping, OVERDUE — THREE promotions since
   74/100 at c469b03).** Run the standard medium single-seed A/B (current
   defaults vs all-three-off) at 1800 s before any medium-metric work.
   Exposure: deduce needs >=500k arming, the walk latch needs 1M conflicts
   unarmed — most medium cells reach neither; verify with identity refs.
2. **Walk-latch second wave.** The latch captured 7 kissat-only cells but
   ITC x2 / dislog / Circuit_multiplier29 still stand, and VdW-23 +
   reconf10 + frb80 became walk-reroll flippers. Candidate refinements to
   SCREEN (unarmedwalk suite + the three new flippers as canaries): latch
   threshold sweep (500k / 2M — is 1M optimal?), rephase-cycle delta for
   the unarmed class, walk size cap (ITC at 208k vars may be
   warmup-bound — profile ONE ITC walk first). Lottery-adjacent: demand
   walk-step/improvement evidence, not just flips.
3. **Cardinality proof engine research arc (mchess_20 + rook-51/52/56).**
   mchess_20 decoded as direct-php P=200/H=198 — blocked on proof SIZE
   (closer is H^4; RAT-scan law kills even H^3 variants at maxVar~2.6M).
   Needs a genuinely new DRAT-emittable cardinality argument designed ON
   PAPER first. Potential +2-4 first-evers, zero reroll risk.
4. **Miter family, remaining levers:** band-re-screen space EXHAUSTED
   (SESSION 16); residual is structural (props/conflict net of restarts,
   DB/decision quality). New decomposition probe required; LOW slope.
5. **Starved hwmcc/BMC class:** CLOSED at current mechanism level.
6. **Checker-timeout proof-size arc (4 at-risk solves)** — track only.

## Current state

- HEAD: SESSION 16b promotion commit (after d6ea413).
  **Full-bench 3600 s baseline: 286/400 promoted** (cand TSV =
  `log/abtest-cand-vs-base-2026-08-06-03-28-37/cand/results.tsv`).
  kissat 4.0.4 reference: 296/400 (`log/kissat-full-20260729-210758`) —
  **gap −10** (was −25 at 14b, −17 at SESSION 15). kissat-only 48 /
  solver12-only 38 / both-timeout 66. Lineage this month: 261 → 271 →
  277 → 280 → 279* → 286 (*deal variance; paired A/Bs all WINs).
- Default surface added SESSIONS 15-16b: SAT_VIVIFY_DEDUCE=on +
  _ARMED_MIN=500k; SAT_REPHASE_UNARMED_MIN=1_000_000;
  SAT_WALK_EFFORT_UNARMED=50 (was 200 = dead code); SAT_BACKBONE=off;
  banded sort/tier3/reuse knobs off (closed).
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

## Standing traps (updated 2026-08-06b + carried)

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
