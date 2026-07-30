# NEXT PLAN — 2026-07-30 (supersedes the 2026-07-28d revision)

One-file plan for the next clear context. Folds SESSION 14 (2026-07-30:
first-ever FULL-BENCH 3600 s comparison — solver12 261/400 vs kissat
296/400; medium-100 inside the same run WON 76 v 75; the whole −35 is
out-of-sample; three gap mechanisms identified with probes; ranked
incremental ideas rebuilt from full-bench evidence) on top of
SESSIONS 4-13. Where this contradicts an older `plan/next-steps-*.md`,
THIS file wins.

**START HERE:** read "SESSION 14" below, then "RANKED PLAN (2026-07-30)",
then "SESSION 13".

## SESSION 14 (2026-07-30) — FULL-BENCH 3600 s GAP READ: solver12 261/400 vs kissat 296/400; medium-100 WON 76 v 75; the −35 is entirely out-of-sample; zero code changes

Full deep-dive: `plan/gap-read-full-2026-07-30.md`. Artifacts:
`log/gap-read-full-2026-07-30/` (gap_read.txt, analyze.py, analysis.txt),
`log/seedgate-solver12-full3600-2026-07-29-21-07-58/results.tsv`,
`log/kissat-full-20260729-210758/results.csv`. New tool:
`tools/run_kissat_full.sh` (parallel kissat runner, `-d` suite /
`-c` core-offset / `-j` jobs). Run shape: concurrent disjoint sockets
(s12 18 jobs cores 0-17 WITH proofs+verify; kissat 14 jobs cores 18-31),
3600 s / 16 GB, ~11.4 h. Zero correctness failures: 257 verified ok,
0 FAIL, 0 contradictions on 232 both-solved.

**Headline facts (all analysis-tier, NOT promotion evidence):**

1. **solver12 261/400 (SAT 133 / UNSAT 128, PAR-2 1,136,656); kissat
   296/400 (150/146, 923,215). Medium-100 inside the run: 76 v 75 WIN
   (was 73 v 75 on 07-24). Non-medium-300: 185 v 221 (−36) — the
   ENTIRE gap is out-of-sample.** Kissat leads the full-bench
   truncation curve at every cutoff (+12@300 s → +27@1800 s →
   +35@3600 s); 23 of its 64 exclusive cells solve in <=600 s, so this
   is capability, not only tail.
2. **The special-refutation + arming portfolio GENERALIZED: 29
   solver12-only cells (13 on medium)** — roundrobin x8 (gate-BVE
   class), cliquecoloring x7 (SAT_PHP_REFUTE fired on 5 never-seen
   siblings, all <2 s), oddball-tto_zp SAT x4 (endgame rephase class),
   xor_op x2, rphp x2, tseitin_n188, Kakuro-132, TT_C496, HCP-529,
   frb80 (SAT 3325 s), valves-gates (UNSAT 3400 s, checker-timeout).
   Zero false fires, zero proof failures out-of-sample.
3. **Three gap mechanisms, probe-confirmed (kissat -s vs SAT_STATS_JSON
   300 s probes, table in the gap-read doc):**
   (a) *scoping over-fit* — kissat's always-on ELS-substitution/probing
   units/round-BVE instantly collapse cells our scoped/off passes skip
   (bv_ILA_Piccolo: kissat substitutes 35% of vars + 22% units and
   finishes in 8 s / 8.4k conflicts; s12 grinds 9.07M conflicts, 994 s;
   same story n320p5q2 16 s, uniqinv40 51 s via sweep-substitution 30%
   of vars, blockpuzzle 25 s via ELS 46%);
   (b) *long-horizon trajectory quality* — the REDUCE-law/tick-cadence
   shape: on booth-bit29 kissat sustains 11.6k conf/s and finishes in
   6.2M conflicts where s12 does 7.6k/s and >20M without solving; the
   16x16 multiplier-miter family alone is 10 kissat-only cells (+4
   both-timeout); oddball-ttf: kissat 49.5k conf/s vs our 10.4k;
   (c) *walk/rephase SAT capability* — kissat walks 100-360M steps on
   its SAT wins (Circuit_multiplier24/29, sted2_0x0_n219-342, ITC,
   HCP-446); our armed walk fires 3-27M flips on the same cells.
4. **Memory is now METRIC-MOVING: pj2016_k100 (8.8M vars) is a real
   lost cell** — s12 rc-6 OOM-abort at 53 s (virtual >16 GB during
   parse/setup; RSS 12.7 GB @150 s unlimited idle), kissat solves it
   SAT 1568 s in-budget. Mirror win: pj2002_k500 — KISSAT OOM-aborts
   (exit 134 @122 s) while s12 parses fine (giant-arena diet) and runs
   full wall. 17.normalised (7.1 GB text): both abort.
   tseitin_d3_n100000 (8.7 MB formula!): s12 OOM at 1711 s via a
   LATE-phase explosion (RSS still 0.12 GB at 540 s idle re-run);
   kissat times out too, so hygiene not +1.
5. **TWO unbounded-pass bugs found and ROOT-CAUSED with gdb backtraces
   (archived in `log/gap-read-full-2026-07-30/`):**
   (a) *battleship-13-13 spin* = mid-search `sweep_round` →
   `sweep::prove_facts` (unlimited-budget wrapper, sweep.rs:256) →
   kitten `solve_budgeted`/`propagate` — ONE kitten sub-solve with no
   effective budget ate the whole 3600 s (deterministic; proof frozen
   at byte-identical 150,995,327 across runs). kissat solves the cell
   in 21 s — bounding this may make it winnable.
   (b) *tseitin_d3_n100000 rc-6 OOM* = pre-search `try_gauss_refute` →
   `gauss::min_degree_order` (gauss.rs:474) — ~25 min of HashMap churn
   on a 100k-equation XOR system, then unbounded elimination fill-in
   (31.4 GB RSS measured); the bench cell got ZERO search time.
   Neither path checks SAT_LIMIT_WALL_SEC (it lives in the CDCL loop
   only). External `timeout` still bounds bench runs; gates are
   unaffected, but SAT_LIMIT_WALL_SEC screens under-read any cell
   entering these phases.
6. **Regressions vs the 07-24 medium 3600 s run: net +3 (73→76), but
   bp4_BC012_CSO_FPBEQ_FPBLE_ZR went SAT@205 s → TIMEOUT** — an
   undocumented reroll casualty of the post-07-24 promotion chain
   (sibling bp4_BC012_CSO_AM... is kissat-only at 2537 s). Bubble_7_6
   also lost (was the documented 2880 s marginal).
7. **php-detector near-misses in the both-timeout core:**
   cliquecoloring_n14_k7_c6 declined while n15_k7_c6/n26_k7_c6 fired
   (boundary condition?); clqcl_30_9_8 and clqcl_30_11_10 declined
   (k>7 — likely the 3-7-longest-covers precheck or slot-count bound);
   harder-fphp-016-015 (direct, non-relativized php) and rphp_p25_r25
   unmatched. Five capability leads in the proven SESSION-11 shape.

## SESSION 13 (2026-07-28 night) — grid-n400 CLOSED PERMANENTLY (RAT-scan law); snake proof layout landed default-off; st_659 identified; zero promotions

HEAD 97d80a6 defaults byte-identical (rbsat 100k fingerprint
100001/196258/17,758,017 digit-exact; MVRR 267,199 digit-exact; 735 unit
tests (+2 snake tests, +php race fix); smoke 9/9). No 100-cell gate
spent — correctly: the session's candidate died at triage tier 1.
Artifacts: `log/tseitin-snake-2026-07-28/measurements.txt`.

**1. `SAT_TSEITIN_SNAKE` (default-off groundwork) — the SESSION 12 #1
item (grid proof shrink) implemented and it WORKS as a proof-size
mechanism: grid_n400 ER proof 14,629,730 -> 7,622,504 lemmas (1.92x,
238MB, gen 17.4s, cell wall 34.2s incl. dry-run, RSS 275MB).** Three
structural levers, all in `gauss.rs tseitin_refute_mode`: (a)
boustrophedon (snake) walk via an adjacent-to-last tie-break in the
greedy frontier order — each grid row's cut is then consumed in exact
LIFO order; (b) "valley" (bitonic next-use) chains consumed by
single-slot TOP POPS (P holds one pointer per chain for its whole life;
the `{z_end, z_base}` pair never opens; a bitonic window's earliest-due
var is always at an end, so consumption never needs interior
extraction); (c) raw compress target 1. Steady P width 5 -> 3. Emission
model VALIDATED to the digit: per-step cost = one stage per shared var
per combine, each stage ~2^(width-1) clauses (measured 91/step legacy,
47.6/step snake — the model's 48). Mode selected by DETERMINISTIC
DRY-RUN with sink emitters; irregular components (n188 expander needs
~10 live chains = width cap) fall back to the byte-identical legacy
layout (n188: 4,714,240 lemmas, VERIFIED 197s, solve wall 32.7s).

**2. THE RAT-SCAN LAW (permanent, checker-side — the arc-killer):
`drat-trim.c checkRAT` loops over the ENTIRE literal space
`for (i = -maxVar..maxVar)` and walks every watch list, for EVERY
core RAT lemma. ER-proof verify cost ~ #core-RAT-lemmas x (2 maxVar +
live watch mass) — proof LENGTH is nearly irrelevant.** Measured on a
synthetic grid ladder (same instance, both layouts, defs equal, RUP
lemmas halved): n60 2.0 vs 2.75s; n120 34.8 vs 41.0s — HALVING the
lemmas buys only -15%; n200 375s (5.0k lemmas/s vs 82k/s at n60).
grid_n400 (7.62M lemmas, 637k core RAT lemmas over 319k instance vars):
**idle drat-trim s VERIFIED in 13,327.8 s = 3.7x the 3,600 s in-gate
verify budget** (so the OLD 14.6M proof, killed unfinished at 3,717 s,
was only ~15% done — its total was ~26,000 s+). The snake proof is
VALID; it is verifiability inside the gate that is refuted. Definition count is WALK-INDEPENDENT (~1 per grid
vertex — every vertical edge must be chained once under any linear
walk) and pure-resolution grid-Tseitin is exponential (Urquhart), so
**no .rs-side proof shape can fix this. tseitin_grid_n400 is
CHECKER-BOUND and PERMANENTLY CLOSED under `drat-trim <cnf> <proof>`.
Do not revisit without a different checker (out of scope) — the
SESSION 12 "~2.5x shrink restores the fast rate" hypothesis is REFUTED
(rate is var-count-driven, not length-driven).** Corollary for ALL
future ER families: viability metric = #definitions x instance maxVar
(php/n188 verify fast because their var spaces are tiny, NOT because
their proofs are short). Caps stay effectively legacy (snake flag off);
with the flag on, caps are 200k/8M — safe only because the cell solves
in 34s and verify failure would be a gate correctness FAIL, so the flag
must stay OFF at 1800s.

**3. RESIDUE/RETRY LAW (proof-emission trap, measured): an aborted ER
attempt DELETES the axiom clauses it consumed, so a follow-up attempt
streamed into the same live proof lacks its RUP premises — composite
proof is `s NOT VERIFIED`** (n188: 365k-line snake residue + legacy
retry). Any multi-layout ER engine must select the layout by dry-run
with sink emitters (deterministic replay), never stream-and-retry.
Also fixed: php.rs unit-test temp files raced under the parallel
runner (plain vs shuffled rphp share (pid, nv) — now sequence-tagged;
this was the source of rare flaky `s NOT VERIFIED` test failures).

**4. st_659 IDENTIFIED (structure fully mapped, refutation hunt still
closed): it is a digraph kernel / stable-model existence encoding.**
1977 vars = 3 layers x 659 items (x=in-kernel, y=absorbed,
z=unfounded-escape); digraph E, 11,833 edges, only 322 symmetric;
x_a -> y_b for (a,b) in E (+ AMO => kernel independence);
y_a -> OR_{b in succ(a)} x_b (ALL 655 support sets equal succ(a)
exactly — absorption); z_a -> OR_{b in succ(a)} z_b (unfounded
chains); x_a -> not z_b for (b,a) in E; covers: >=1 x overall, >=1 x
and >=1 y among the 575 non-unit items; 84 forced units. Kernel
non-existence is NP-complete with no known polynomial certificate
family, so there is NO ER-refutation shape to implement without the
generator's argument — and the instance may simply be SAT. **4h idle
status probes CONCLUDED (2026-07-29): BOTH kissat --sat and solver12
TIMED OUT at 14,400 s with no verdict.** st_659's status remains
UNKNOWN at 4x the gate wall; the kernel-existence identification is
the only new fact. The cell stays in the hard core.

**5. What this changes in the ranking:** the last "concrete +1" from
the 07-28c plan is gone. The special-refutation axis is now closed on
ALL 14 hard-core cells with per-cell mechanism verdicts (ramsey:
proof-size scale; grid n400: checker-bound; st_659: no certificate
family known / status unknown; the rest: SAT-likely or no counting
core). The search/inprocessing axis was closed in SESSIONS 9/10/12.
The next +1 at 1800s needs either a genuinely new mechanism class or
new suite cells.

## SESSION 12 (2026-07-28 evening) — starved-class kissat-pipeline bundle NEGATIVE (the last search-side axis closed); grid n400 verify measured; zero promotions, baseline stays 74/100

HEAD defaults byte-identical (rbsat 100k fingerprint 100001/196258/
17,758,017 digit-exact; MVRR 267,199 digit-exact; 738 unit tests (+5);
smoke 9/9 both flag states). No 100-cell gate spent.

**1. Hard-core structure read (SESSION 11 item 1) — NO tractable
special-refutation family remains.** Full clause-length/role decode of
the 14-cell hard core: ramsey_4_4_18 (K18, 2x3060 len-6 clauses) has a
mathematically complete counting+parity proof (Greenwood-Gleason
R(4,4)<=R(3,4)+R(4,3), handshake parity for R(3,4)<=9) but the DRAT
emission needs ~C(17,9)=24k per-subset R(3,4) sub-proofs x thousands of
lemmas = ~100M+ lemmas vs the ~6M checker cap — research-scale, not a
session. ramsey_3_6_19 worse. VdW-27: no poly refutation known, status
possibly SAT. TT495/TT7F/lockchart/rbsat-v945: SAT-likely or no
counting core. st_659 decoded (NEW): 3 layers x 659 item-vars
(x/y/z_i), 655 at-least-one ternaries (i, i+659, i+1318), per-item AMO
(1965 negneg binaries at dist 659/1318), ~11.8k cross-item conflict
binaries, 1309 wide all-positive per-layer covers (len 10-26, npos
15-20), 96 forced units — a 3-period timetabling/partition shape, no
strict php-core over-constraint visible, UNSAT status unknown. Item 1
is CLOSED unless a new suite arrives.

**2. `SAT_SWEEP_TICK_ROUNDS` + `SAT_ELIM_TICK_ROUNDS` (default-off
groundwork) — the full kissat starved-cell pipeline, implemented and
DEFINITIVELY NEGATIVE at 1800 s.** This was the one unmeasured
combination left from SESSIONS 9-10: tick-cadence-triggered rounds
running the FIXED sweep engine (kitten-tick-share budget 100‰ since
last sweep clamped 10M..1G, whole-env completion flags resuming across
rounds, escalation on completed passes, persistent cross-round dedup,
substitution as ORIGINAL binaries + ELS) plus bounded unarmed eliminate
(armed-bounds effort path) — everything scoped to `round_via_tick` so
conflict-cadence rounds and non-starved cells are byte-identical by
construction. Mechanism CONFIRMED on goldcrest (300 s idle: 3 tick
rounds, 7197 envs, 2358 equivalences, 1293 ELS-substituted vars vs 0
baseline, elim rounds firing) — and the metric still loses everywhere:

- 4-arm 1800 s screen on 14 starved cells
  (`log/abtest-full-vs-sweep-vs-elim-vs-base-2026-07-28-17-17-33`,
  suite `benchmarks/starved-screen-2026-07-28`): **base 3/14 solved
  (oski15b20 1538 s at reference conflicts 2,663,684; vex at reference
  2,975,066; rbsat 1652 s), full 0/14, elim 0/14, sweep 1/14 (rbsat
  conflict-identical 6,257,890 — its tick trigger never armed in-run).
  ZERO timeout flips; every candidate arm loses the in-class coins.**
- Idle 1800 s re-check (contention caveat closed): goldcrest, st_659,
  TT7F, lockchart-group1 ALL still TIMEOUT under the full bundle
  (`log/starved-arc-2026-07-28/idle_full_bundle_4cells.txt`).
- 100-cell ticks/conflict scan at 100k conflicts
  (`log/starved-arc-2026-07-28/ticks_per_conflict_scan.txt`): the
  >=12k-ratio class is ~44 cells including ~30 SOLVED (bp5_CSO 172k,
  oddball 122k, velev 71k, sudoku 45k, oski15b40 44k, Kakuros, jkkk,
  aaai10, reconf10 x2, vex 15.2k, oski15b20 31.6k) — the early-search
  proxy OVERSTATES in-run arming (rbsat 19k proxy never armed; session
  9's lockchart-group2 29k proxy never armed), but vex/oski15 DO arm
  and DO lose their coins. There is no promotable scope: the class
  either contains the coins or fires nothing.
- booth x3 / Bubble / fixedbandwidth sit at 2.2-4.9k ticks/conflict —
  NOT starved; the tick trigger structurally cannot reach the density
  class at any sane floor.

**VERDICT: the starved-cell inprocessing axis is now CLOSED at 1800 s
with the complete kissat design measured** (session 9 cadence-only,
session 10 root-pass-only, session 12 full pipeline). kissat's
goldcrest/booth wins at ~1200-1400 s do not transfer at our search
shape inside 1800 s. Both flags stay default-off groundwork; do not
re-enable without a >3000 s objective.

**3. tseitin_grid_n400 checker arc RE-MEASURED — still blocked, but
the blocking mechanism is now precise and the reopen target concrete
(`log/starved-arc-2026-07-28/grid_n400_checker_measurements.txt`).**
- Engine reproduces the proof with caps lifted: 14,629,730 lemmas /
  528 MB / 23.3 s gen (breakdown main=7.6M shift=2.5M append=3.8M
  defs=0.64M). In-gate solve would be ~25 s = fat-margin +1 that
  NOBODY (kissat included) solves at 3600 s.
- `SAT_TSEITIN_COMPRESS` sweep: default target=2 IS the floor (c=3
  doubles to 28.6M, c=4 quadruples — 2^(w-1) materialization
  dominates).
- drat-trim WITH deletions: **killed unfinished at >3717 s idle vs the
  3600 s harness verify budget (2 x 1800 s)** — unverifiable in-gate;
  the old 6M-lemma cap stays correct.
- drat-trim WITHOUT deletions (new `SAT_TSEITIN_NO_DEL` measurement
  knob): **s NOT VERIFIED in 11 s** — definition-variable RECYCLING is
  RAT-sound ONLY because deletions clear the recycled var's old
  clauses. no-del + no-recycling = the originally-documented
  unverifiable variant (var-space blowup). Deletions are load-bearing;
  do not strip them.
- **The reopen lever is superlinearity: n188_d3 checks at 25k
  lemmas/s (4.71M/187 s) while grid checks at ~4.1k/s — rate degrades
  ~6x for a 3x bigger proof. A ~2.5x proof shrink (to ~6M lemmas)
  likely restores the fast rate => ~240-600 s verify, comfortably
  in-budget.** Candidate shrink: fold the constraint's new far-use
  vars into the chain inside the SAME combine as the main addition
  (append+main fusion — append is 3.8M of the 14.6M), and/or fuse
  shift+main. This is a redesign of the chain machinery in
  `gauss.rs tseitin_refute_with_proof` — one focused session, high
  correctness bar (proof-critical), offline-testable in ~40 s per
  iteration (23 s gen + drat-trim on a prefix).

**4. Bookkeeping.** New default-off flags: `SAT_SWEEP_TICK_ROUNDS`,
`SAT_SWEEP_TICK_PERMILLE` (100), `SAT_ELIM_TICK_ROUNDS`,
`SAT_TSEITIN_NO_DEL` (measurement-only). New stats: sweep_tick_rounds/
envs/ticks, elim_tick_rounds. New triage suite:
`benchmarks/starved-screen-2026-07-28` (14 cells). The 2026-07-24
tick-ratio calibration ("healthy <=9.2k / starved >=15k") is
SUPERSEDED by the 100-cell scan — the class boundary is much broader
and includes solved coins; any future tick-scoped pass must enumerate
arming on the FULL suite, not a 10-cell probe.

## SESSION 11 context (unchanged)

The 1800 s "measured local
optimum" of SESSION 9 was real for the *search/inprocessing* axis but
not for the *special-refutation* axis: a brand-new capability mechanism
(the SAT_TSEITIN promotion shape — universal-timeout + unique-capability
+ scoped-firing) took FOUR cells (rphp5_050/085, clqcl_40/50_6_5) that
NOBODY — kissat included — solves at 3600 s, each in <0.3 s with
drat-trim-verified extended-resolution counting proofs. The hard core is
now 14 cells. Companion deep-dive: `plan/kissat-gaps.md`.

## SESSION 11 (2026-07-28) — SAT_PHP_REFUTE PROMOTED, gate WIN 74v71 (+4 FIRST-EVER capability, −1 documented coin), baseline now 74/100

**PROMOTED: `SAT_PHP_REFUTE` default-on — pigeonhole-counting
extended-resolution refutation (php.rs, ~700 lines + 14 tests).** Gate:
`log/abtest-cand-vs-base-2026-07-28-08-08-20`, `promotion_gate=PASS`,
**74 v 71 solved**, 70 both-solved cells ALL conflict-identical (zero
diff cells — complete trajectory identity), PAR-2 115,260.5 vs
128,218.6, zero contradictions/correctness failures (vex
checker-timeout symmetric as always).

**Mechanism.** Two clause shapes reduce to one abstract counting core
(P pigeon literals a[p][r] over N places, hole literals hole[r][h],
H < P shared holes, with F1/F2/F3 unit-propagation conflict guarantees):

- `rphp` (relativized PHP, SHUFFLED + sign-flipped instances): P
  var-disjoint pigeon covers (the strictly longest clauses), per-place
  at-most-one binary cliques, pigeon→used binaries, used→hole covers,
  and pairwise used-places-can't-share-a-hole 4-clauses. rphp5 = 5
  pigeons → 50/85 places → 4 holes.
- `clqcl` (clique-coloring with EXISTENTIAL edge vars): 6 slot covers,
  AMO-per-vertex cliques, slots force edge literals via ternaries, edges
  forbid equal colors, unconditional H-color covers. 6 slots → 40/50
  vertices → 5 colors.

Detection is literal-based (shuffle/polarity invariant) and
all-or-nothing: EVERY clause the counting argument needs is verified by
exact lookup, so detection itself is the soundness anchor (matching ∧
P>H ⇒ UNSAT by construction; a proof bug could only produce a
checker-rejected proof, never a wrong answer on a SAT instance). The
proof: fresh W[p][r][h] ~ a∧hole and G[p][h] ~ OR_r W definitions (RAT,
pivot-first), N²-scale pairwise W blocking lemmas (RUP via F2/F3), G
lifting, per-pigeon covers (RUP via F1), and an injective-assignment
DFS over the (H+1)×H G matrix ending in the empty clause. Proofs
106k-300k lemmas / 0.6-1.8 MB, emitted in <0.08 s, drat-trim VERIFIED
on all four real cells (in-gate `verified=ok` AND standalone).

**Key implementation facts:**

- Frontend BVA had to be held off matching formulas
  (`php::formula_matches` consulted before the factor block in main.rs):
  factoring rewrites the clqcl adjacency ternaries and hides the
  structure — clqcl declined until this was found. rphp survives factor
  untouched, clqcl does not.
- The histogram precheck (3-7 strictly-longest covers ≥10 wide,
  everything else ≤8 literals, ≤400k clauses) passes on EXACTLY the 4
  family cells of the 100-cell suite (measured offline over all 100) —
  the other 96 pay one length scan; full detection never runs there.
  Byte-identity verified: rbsat 100k fingerprint 100001/196258/
  17,758,017 and MVRR 267,199 digit-exact in BOTH flag states, in-gate
  70/70 both-solved conflict-identical.
- The −1 is `oski15a01b20s`: base solved 1602.1 s of 1800 at conflicts
  2,663,684 — its EXACT documented reference trajectory (the standing
  wall-coin flipper test: identical conflicts, different outcome = pure
  wall lottery under load). The pass provably never touches it. Trade:
  4 mechanism-validated capability gains vs 1 documented coin — clean
  under the flexible rule.
- Validation: 729 unit tests (14 new: RUP/RAT per-line proof checks,
  shuffle/flip detection, SAT-variant/missing-clause declines, drat-trim
  end-to-end on synthetics), smoke 9/9 both flag states, shipped
  defaults reproduce all four solves + both identity references
  digit-exact.

**Take-aways:**

1. The SESSION 9 "local optimum" verdict applies to the
   search/inprocessing axis only. The special-refutation ledger
   (SAT_GAUSS → SAT_PAIR_ABS → SAT_TSEITIN → SAT_PHP_REFUTE) is now
   4-for-4 promoted, each banking unique capability kissat cannot reach
   at any wall. Structure detection + ER proof emission is solver12's
   highest-yield promotion shape: zero reroll risk by construction
   (fires pre-search, strict decline elsewhere), so a gate can only
   confirm capability, not lose banked cells (modulo deal-noise coins).
2. When hunting the next such family, reverse-engineer the timeout
   cells' clause-length histograms + variable roles FIRST (the rphp
   structure was fully decoded from the shuffled instances in minutes
   offline, before any solver work).
3. drat-trim RAT-with-fresh-variables + RUP-lemma streams remain fully
   checker-compatible at 300k lemmas — the TSEITIN infrastructure
   generalizes.

## SESSION 10 (2026-07-27 night → 07-28) — SAT-sweeping productivity arc: two features landed default-off, sweep-substitution defect FOUND and fixed, zero promotions, baseline stays 72/100

Both features are committed default-off with byte-identical defaults
(rbsat 100k fingerprint 100001/196258/17,758,017 digit-exact; MVRR
267,199 digit-exact; 715 unit tests (+9); smoke 9/9; drat-trim VERIFIED
on every UNSAT smoke formula in all flag combinations).

**1. THE DEFECT (permanent mechanism fact): `sweep_round` adds its proven
equivalences as LEARNED binaries via `inprocess_add_clause`, but
`try_els` harvests its implication graph from `original_clause_ids`
ONLY — so sweep-proven equivalences have NEVER been substituted, on any
cell, ever.** Measured on booth_dadda_mapped (20k-interval probe, 100k
conflicts): 352 sweep equivalences found across rounds, 4 ELS calls, 0
substitutions, every ELS call logging "no equivalences". The
"ELS merge cascade" comment in sweep_round described machinery that never
ran. Additionally the finds are massively DUPLICATED (the same pair
re-proven by overlapping environments): booth's 352 finds are 11 distinct
pairs. All historical `sweep_equivalences` stats (vex 10,330 /
oski15 1,005 / booth 792 / TT_C406 5) are duplicate-inflated counts of
never-substituted facts — they only ever acted as extra learned binaries.

**2. `SAT_SWEEP_SUBST` (default-off) — the fix: install sweep equivalence
binaries as ORIGINAL clauses (deduped, skip-assigned), so `try_els`
actually merges them; `SAT_SWEEP_SUBST_MIN_EQUIVS` scopes it to rounds
proving >= N distinct pairs (low-yield rounds keep the learned shape
byte-identically — the TT class finds ~5, so N=100 protects every banked
timetable cell by construction).** Screen results (1800 s idle-ish, scan
load background; conflicts are load-immune):

- **Substitution mass is REAL and kissat-scale: vex 76,062 substituted
  vars, oski15b20 69,347 (kissat: 71,487 — parity).** A handful of sweep
  binaries close huge implication cycles in the BMC binary graph.
- **But the metric loses at the decisive tier: vex conflicts 2,975,066 →
  3,412,420 (+437k), oski15b20 2,663,684 → 2,832,881 (+169k) — 2-cell
  aggregate +606k WORSE.** Both still solve (both UNSAT verified).
  Wall moves the OTHER way and hard: oski15b20 1642 s (idle baseline) →
  1281 s UNDER LOAD (−22%+); vex 1469 → 1524 s under load (≈neutral-ish).
- Timeout cells: booth x3, Bubble, stp212 (3 arms incl. 3e9-tick root
  budget + subst), g2-oski — ALL stayed TIMEOUT. No capability flip.
- **Verdict: the REDUCE-law shape exactly — real throughput/collapse
  mechanism, negative on the conflicts tier at 1800 s single-deal.
  DO NOT enable at 1800 s without new evidence. It belongs in the
  >3000 s-horizon bundle** (wall-dominated scoring; the substitution
  mass shrinks the formula permanently, which compounds over long runs).

**3. `SAT_SWEEP_ROOT` (default-off) — kissat-parity ROOT sweep pass:
occurrence-driven environments over the post-BVE/probe/transitive root
formula, per-environment completion marking (kissat's whole-env sweep
flags — without it 91/102 booth probe finds were duplicates and a 90 s
budget re-proved the same facts forever), cross-pass dedup, kitten-tick
budget (lits x200 clamped 200M..2G), escalation ladder (256->8192 vars,
1024->32768 clauses, depth 2->3, re-flag on completed pass), pass-1
dry-run probe (2000 envs or 10% budget) with a yield-per-swept-env
adoption threshold (default 20 permille), all-or-nothing: rejecting cells
pay only the bounded probe (0.4M-40M ticks, sub-second to ~4 s) and stay
byte-identical. Adopters get units + original-clause equivalence binaries
+ ELS merge + a final eliminate(true) re-run.** Screen results:

- 1800 s idle on 14 under-cap timeout cells: **ZERO flips.** stp212
  applied the largest mass by far (520 units + 13,518 equivalences = ~8%
  of 172k live vars, probe yield 515 permille) and still TIMEOUT, even at
  a 3e9-tick budget and with SWEEP_SUBST stacked. goldcrest applied
  131+163 (its 321k live vars need ~20x the budget for full coverage —
  the root-only pass cannot reach kissat's 38k-substitution mass there;
  kissat gets it by sweeping 10% of ticks ALL RUN LONG on a
  progressively-collapsed formula). booth/Bubble reach fixpoint fast but
  their distinct equivalence structure at root is tiny (11 / 5 pairs).
- 100-cell probe scan (SAT_LIMIT_CONFLICTS=100, final tally 36 adopt /
  42 reject / 12 var-cap-skip / 10 solved-pre-sweep): at the 20-permille
  yield threshold **36 cells adopt, including the ENTIRE banked TT class
  (C392/393/492/495/496), sted2, ibm, both oski15s, both Kakuros, jkkk,
  twitter, the sqrt-miters and Pancake** — a huge solved-cell reroll
  surface. And the yield ranking separates the WRONG WAY: the intended
  capability target goldcrest probes at **31‰ — the LOWEST of all
  measured adopters** — versus TT496 91‰, sted2 104‰, TT_C392 139‰,
  Kakuro-132 326‰, aaai10 339‰ (SESSION 6's THRESHOLD LAW, now proven
  for probe-yield too). Only stp212 (515‰) separates cleanly above
  everything, and stp212 does not flip.
- **Verdict: no promotable scope exists — the pass either adopts nothing
  that moves, or rerolls protected solved cells. Stays default-off
  groundwork.** Its real contribution: the fixed sweep engine
  (env-marking + dedup) and the probe/scan numbers above.

**4. Engine facts worth keeping:** solver12's sweep environment cost is
~6-10 ms per env on dense cells (2000-solve budget, ~30 ticks/solve —
tiny kitten solves, per-solve overhead dominated); goldcrest live vars
after root simp = 321,259 with only 4.5% probe coverage per 856M ticks.
kissat's sweep productivity is NOT one bug but three compounding designs:
tick-share budget over the whole run, whole-env completion flags, and
substitution that actually fires. We now have all three implemented, but
only as a root pass (cadence-starved cells still get zero mid-search
rounds — the SESSION 9 tick-cadence verdict stands).

**5. Screen artifacts (preserved):** `log/sweep-arc-2026-07-28/` —
`scan_results.txt` (100-cell probe map: per-cell probe units/equivs/
envs/yield/adoption + skip reasons), `triage1_summary.txt` (14-cell
SWEEP_ROOT idle, all TIMEOUT), `triage2_summary.txt` (SWEEP_SUBST:
booth x3/Bubble TIMEOUT, vex/oski15 solved — conflict/wall numbers
above), `triage3_summary.txt` (stp212 3 arms, all TIMEOUT), plus the
vex/oski15 stats JSONs with the substitution-mass counters.

## SESSION 9 (2026-07-27 evening) — five lines screened NEGATIVE, zero promotions, baseline stays 72/100

HEAD 69ec5eb (groundwork commit, defaults byte-identical to fe82400 —
rbsat props 17,758,017 / MVRR 267,199 digit-exact, 706 unit tests (+6),
smoke 9/9 both flag states). No 100-cell gate was spent; every verdict
below is triage-tier idle-screen evidence, which twice this month matched
the gate to the digit.

**1. `SAT_TRANSITIVE_UNITS_ONLY` (ranked 3a) — implemented default-off,
screen LOSE 15v16. Item 3a CLOSED.** The decision scan (100 cells,
SAT_LIMIT_CONFLICTS=100, `transitive_found_units`) found the exposure is
21 non-adopter cells, 3.5x SESSION 6's remembered list — including the
three giants (00fd8ac/83aa/ee5, 363 units each), all three Kakuros, and
four bp4s. 21-cell 1800 s A/B: the units arm TIMED OUT
bp4_TCO_CSO_IXA_LP_ZR (base SAT 233 s — the banked kissat-only capability
cell) and lost both-solved conflicts ~+1.0M (jkkk +1.70M, twitter +345k,
Kakuro-132 +283k vs reconf10_70 −147k, Pancake −395k...). Root units are
just another uncontrolled trajectory reroll — REROLL-VARIANCE, exactly as
SESSION 6 predicted.

**2. `SAT_INPROCESS_TICK_CADENCE` re-measure (stale-context hypothesis:
its old rejection was bundled with unconditional gate-BVE) — ZERO flips,
CLOSED at 1800 s.** 10-cell starved-class A/B (goldcrest, lockchart x3,
pj2008, vex, stp212, oisc, booth_dadda_mapped, fixedbandwidth): every
timeout cell stayed TIMEOUT in both arms; lockchart-group2 solved both
arms conflict-identical (1,239,136 — ratio never armed, scope confirmed).
Mechanism probe: rounds DO fire (goldcrest on-arm sweep_equivalences 351
vs 0 off-arm at 100k conflicts, 1.66e9 ticks > the 1.5e9 interval;
goldcrest is 41,622 ticks/conflict — deeply starved). Conclusion is
mechanism-solid: tick-fired inprocessing rounds run and still flip
NOTHING at 1800 s. Stays the >3000 s-horizon candidate only.

**3. GBVE-adopter round extension (`SAT_TRANSITIVE_INPROCESS_GBVE` /
`SAT_PROBE_INPROCESS_GBVE`, default-off) — the root-adopter shape's 3rd
reuse FAILED; the shape is now measured MINED OUT.** Scoped-gate-BVE is a
root pass with a deterministic 19-cell adopter class (enumerated via
`gate_bve_scoped_adopted`, matches SESSION 3's list; sted2 is also a
transitive adopter = flag no-op there). 12 movable cells (7 solved with
>=1M conflicts + 5 timeouts; 6 adopters sit under the 1M round interval,
auto-protected). 4-arm screen (tr/pr/both/base): **ALL LOSE** — tr +109k,
pr +3.18M, both +403k both-solved conflicts, zero timeout flips,
RoundRobin_n16_d13 conflict-identical in all four arms (its single 1M
round finds nothing). Trace autopsy of the tr arm: the miter "wins"
(sqrt171 −110k conflicts on FIFTEEN total removals, Pancake −205k on ~30)
are pure reroll luck, while the one high-yield cell (bp4_BC012: rounds
removed 11,855 then 3,366 binaries at 38.9‰/11.6‰) LOST +519k — a
round-yield threshold separates in the WRONG DIRECTION (it would keep the
loser and drop the lucky winners). Unlike sted2/ibm (whose rounds remove
30-90k binaries against structure that keeps regenerating edges), the
gbve class has no mid-search edge mass: the shape only pays where the
ROOT pass itself found percent-scale binary redundancy.

**4. Root ELS enumeration (`SAT_ELS=on` scan; stats JSON now emits
`els_substituted_vars`/`els_rewritten_clauses` — the fields existed but
were never emitted) — CLOSED WITHOUT A SCREEN.** Substitution mass lives
exclusively on solved cells with nothing to gain or coins to lose:
6s299b685 624k vars (but SAT 186 s / 10,887 conflicts), 18.normalised
492k (SAT 93 s, 0 conflicts), ibm 175k (adopter — would reroll its tuned
trajectory), **oski15 x2 58k each (the 1154 s/1642 s UNSAT wall-coins)**,
vex 5.8k (coin), sudoku-N30 5.3k. Every timeout cell carries <2k vars
(bp4s 35-42, Bubble 160, stp212 1.9k). No adopter class with upside
exists; enabling root ELS is a pure coin-reroll lottery.

**5. Bookkeeping:** the tickcad probe re-confirmed vex times out in BOTH
arms under 20-way load (the documented contention artifact — not signal).
New triage suites committed: `benchmarks/units-screen-2026-07-27` (21),
`benchmarks/tickcad-screen-2026-07-27` (10),
`benchmarks/gbverounds-screen-2026-07-27` (12). Screen artifacts:
`log/abtest-units-vs-base-2026-07-27-17-23-24`,
`log/abtest-tickcad-vs-base-2026-07-27-18-*`,
`log/abtest-tr-vs-pr-vs-both-vs-base-2026-07-27-18-51-20`.

**SESSION 9 take-away: the 1800 s metric is at a measured local optimum.**
Every cheap candidate from the 2026-07-27b ranking is now closed with
mechanism evidence. What remains: (a) the giant memory diet
(capability-adjacent, not metric-moving at 1800 s), (b) the >3000 s
horizon bundle (REDUCE law + tick cadence + low transitive thresholds —
all measured real but valueless at 1800 s), (c) elim_def behind its
fallback fix (low priority, yield 3 orders short), and (d) waiting for a
genuinely new mechanism idea — the next +1 likely needs a new root pass
with percent-scale find mass on a timeout cell, which nothing currently
built provides.

## SESSION 8 (2026-07-27 afternoon) — SAT_PROBE_INPROCESS PROMOTED, gate WIN 72v72 (conflicts −121.6k); ELS rounds NEGATIVE

**PROMOTED fe82400: `SAT_PROBE_INPROCESS` default-on — failed-literal
probing every inprocessing round (kissat probe.c parity:
binary_clauses_backbone fires each probe interval), scoped to
ROOT-TRANSITIVE-ADOPTERS — the first reuse of SESSION 7's root-adopter
shape on another pass, exactly ranked follow-up 3d.** Gate:
`log/abtest-cand-vs-base-2026-07-27-11-58-13`, `promotion_gate=PASS`,
72 v 72 with IDENTICAL solved sets, both-solved conflicts **66,592,856 vs
66,714,464 (−121,608)**, PAR-2 125,937.5 vs 125,917.3 (+20.2, never
reached — decided at the conflicts tier; the delta is load noise), zero
correctness failures (vex checker-timeout symmetric as always).

Mechanism (~40 lines in solver12 `.rs` + 3 tests):

- `inprocess_round_pass` computes `root_adopter = config.transitive &&
  transitive_adopted == 1` and runs the EXISTING
  `probe_root_failed_literals` (tick-budgeted, lits*20 clamped 5M..100M,
  deterministic) at round start when `probe_inprocess && root_adopter`.
  Root SAT_PROBE stays off/independent. New stat `probe_inprocess_rounds`.
- Also landed default-OFF groundwork `SAT_ELS_INPROCESS` (+
  `els_inprocess_rounds` stat): standalone `try_els` right after the
  congruence block (kissat parity: substitute-after-congruence), same
  adopter scope. Identity-verified; see the negative screen below.

Results (deterministic, digit-exact idle → gate → shipped default):

- **sted2: 1,492,091 → 1,246,166 conflicts (−16.5%), 476 → 318 s in-gate,
  1 probe round.** ibm-2004-23: 638,674 → 762,991 (+124k, 208 → 270 s,
  7 rounds) — the aggregate wins the tier. Exactly these 2 cells moved
  in-gate (98/100 conflict-identical pairs).
- rbsat non-adopter identity digit-exact (props 17,758,017 at 100k);
  MVRR (267,199 conf) and gm16 (62) below the 1M round interval — zero
  rounds, byte-identical, verified.

**The 4-arm idle screen (the only two movable cells = complete gate
forecast, again confirmed to the digit):**

| arm | ibm conf | sted2 conf | 2-cell Δ vs base |
|---|---:|---:|:---:|
| base | 638,674 | 1,492,091 | — |
| +ELS | 346,718 | 1,828,373 | +44k WORSE |
| +probe | 762,991 | 1,246,166 | **−122k (promoted)** |
| +both | 250,867 | 4,486,785 | +2.6M MUCH WORSE |

**Take-aways:**

- ELS-only and ELS+probe LOSE despite ELS being great on ibm (−292k):
  per-cell signs are uncontrollable reroll draws (the REROLL-VARIANCE law
  inside the adopter class), but the ADOPTER-SCOPE screen prices the whole
  gate in one 4×2-cell idle sweep — ~25 CPU-minutes to kill two arms and
  bank the third. `SAT_ELS_INPROCESS=on` is a measured NEGATIVE default;
  do not enable without a new mechanism argument.
- The root-adopter shape is now 2-for-2 (transitive rounds, probe rounds).
  Remaining same-shape candidates are thinner: sweep/factor rounds already
  run via arming; vivify is covered. The shape is likely mined out unless
  a new root pass creates a new adopter class.

Validation: 700 unit tests (3 new), smoke 9/9 both flag states, shipped
defaults reproduce the gate candidate digit-exact on all 4 probe cells.

## SESSION 7 (2026-07-27) — SAT_TRANSITIVE_INPROCESS PROMOTED, gate WIN 72v72 (conflicts −385k)

**PROMOTED ecdf632: `SAT_TRANSITIVE_INPROCESS` default-on — inprocessing-
round transitive reduction (ranked follow-up 3b), scoped to ROOT-ADOPTERS
only.** Gate: `log/abtest-cand-vs-base-2026-07-26-23-11-04`,
`promotion_gate=PASS`, 72 v 72 solved with **IDENTICAL solved sets**,
both-solved conflicts **66,714,464 vs 67,099,722 (−385,258)**, PAR-2
125,500.8 vs 125,972.3 (−471.5), zero correctness failures/contradictions
(vex checker-timeout symmetric-documented, ~60 CPU-min per arm).

Mechanism (~60 lines in solver12 `.rs` + 2 tests):

- `inprocess_round_pass` calls `try_transitive_reduce` right after the
  probe step, gated on `config.transitive && config.transitive_inprocess
  && self.stats.transitive_adopted == 1`. The `transitive_adopted` flag is
  set ONLY by the root pass crossing `SAT_TRANSITIVE_MIN_REMOVED_PERMILLE`
  (100‰), so the 96 non-adopter cells never even scan mid-search —
  byte-identity by construction, verified digit-exact on rbsat (props
  17,758,017) and 98/100 conflict-identical pairs in-gate.
- Round threshold `SAT_TRANSITIVE_INPROCESS_MIN_REMOVED_PERMILLE` default
  0 = kissat parity (apply everything found): the adopter's trajectory is
  already rerolled by the root edits, and each round's finds are fresh
  edges exposed by mid-search BVE/congruence/units.
- `try_transitive_reduce` now takes `(min_removed_permille, inprocess)`
  params; new stat `transitive_inprocess_rounds` counts APPLIED rounds.
  Deletion of inline-tagged binaries mid-search is sound: every deletion
  routes through `clause_set_deleted`, which untags watchers in place.

Results (deterministic, reproduced digit-exact idle → gate → shipped
default):

- **sted2: 1,761,498 → 1,492,091 conflicts (−15.3%), 517 → 420 s**,
  1 round fired (its total is ~1.5M conflicts, one 1M-interval round).
- **ibm-2004-23: 754,525 → 638,674 (−15.4%), 262 → 212 s, 5 rounds** —
  93,353 binaries removed vs 57,892 root-only (+35k found mid-search!)
  and 114 vs 94 units. Mid-search rounds keep finding NEW transitive
  edges; the root pass is nowhere near closure on this class. Note ibm
  was the root promotion's +408k reroll loser — the rounds more than
  paid it back.
- gm16sparrc (62 conf) and MVRoundRobin (267,199 conf) finish below the
  1M-conflict round interval: zero rounds, byte-identical (verified).
- Both wall-coin documented flippers behaved: rbsat solved both arms
  (byte-identical trajectory), no solved-set movement anywhere.

**Take-aways:**

- The root-adopter scope is a REUSABLE gate-safe shape for any
  "extend a promoted root pass into rounds" idea: reroll risk confined to
  cells that already rerolled at promotion time, all of them fat-margin
  solved cells.
- A cell's round count is its conflict total over the 1M interval —
  adopters below 1M conflicts are automatically protected. Only sted2/ibm
  could ever move, which made the idle 4-cell probe a complete gate
  forecast (and it was: gate deltas matched the probe to the digit).

Validation: 697 unit tests (2 new), smoke 9/9, shipped defaults reproduce
the gate candidate digit-exact on both touched cells.

## SESSION 6 (2026-07-26 evening) — SAT_TRANSITIVE PROMOTED, gate WIN 70v70 (conflicts −3.34M)

**PROMOTED ab592d2: `SAT_TRANSITIVE` default-on with
`SAT_TRANSITIVE_MIN_REMOVED_PERMILLE=100` — the last small kissat port
(transitive.c), implemented as a root-scoped, threshold-gated dry-run.**
Gate: `log/abtest-candidate-vs-baseline-2026-07-26-17-59-06`,
`promotion_gate=PASS`, 70 v 70 solved with **IDENTICAL solved sets**,
both-solved conflicts **58,178,148 vs 61,520,922 (−3,342,774)**, PAR-2
129,564 vs 130,389 (−825), zero correctness failures/contradictions
(vex checker-timeout symmetric-documented as always).

Mechanism (all in solver12 `.rs`, ~230 lines + tests):

- Root pass in `try_transitive_reduce` (main.rs), called right after
  `probe_root_failed_literals`: collect live binaries over unassigned vars,
  build a CSR implication graph (edge-labeled by clause id), then probe each
  binary clause (src ∨ dst) ONCE from its smaller-index literal side — BFS
  from ¬src with the clause itself excluded (by id, both orientations).
  Reaching dst ⇒ clause implied by the rest ⇒ delete (`d` proof line via
  `delete_clause_for_simplify`); reaching ¬dst or two contradictory literals
  ⇒ failed literal ⇒ unit src (RUP, `learn_lucky_failed_literal_units`,
  units emitted BEFORE deletions — RUP is monotone in the clause set).
  Removals are discovered sequentially against the already-reduced graph, so
  a mutually-redundant duplicate pair never loses both copies and graph
  reachability never shrinks.
- **The whole scan is a DRY-RUN; edits apply only when
  removed ≥ 10% (100‰) of live binaries.** Below threshold NOTHING is
  touched: rbsat-v1375 (19.1‰), vex (0.97‰), oski15 (0.03‰) verified
  digit-exact (conflicts/props/decisions) at 100k conflicts, and 96/100
  cells were trajectory-identical in-gate. Deterministic (tick-budgeted,
  wall-free): SAT_TRANSITIVE_TICKS default = literals*20 clamped 10M..100M.
- Adopting cells: **4 of 100** — sted2 (121‰ removable, −1.80M conflicts,
  1210→524 s), MVRoundRobin (276‰, −1.95M, 179→25 s), gm16sparrc (311‰,
  −254), ibm-2004-23 (120‰, +408k, 160→260 s — fat margin, inseparable
  from sted2 at 120.9‰). All four reproduced digit-exact across idle screen
  and both gate deals.

**THRESHOLD AUTOPSY (the session's key scoping result):**

- **T=25‰ KILLS TT496** (banked, kissat-impossible): 1800 s idle screen
  TIMEOUT under its 3.0% reroll. And the five timeout targets that adopt at
  low thresholds (TT492, TT495, bp4_TCO_CSO_ZR, stp212, rbsat-v945) ALL
  stay TIMEOUT at idle — **transitive reduction flips NO timeout cell;
  conflicts are deterministic, so an idle-timeout cell cannot flip in-gate
  either. Deep-cell rerolls under this pass are pure downside.**
- T=40‰ adds Kakuro-132 +288k / ibm +408k / Kakuro-112 +55k for only −21k
  (Kakuro-115) back. T=100‰ keeps only the four winners.
- bp4_TCO_CSO_IXA_LP_ZR measured +994k conflicts at T≤36 (solves 738 s idle
  — survives but regresses); the TT band sits at 27-30‰; both are protected
  below the shipped 100‰ threshold. The bp4/TT structure classes contain
  banked winners and timeout targets at NEARLY IDENTICAL removable-permille
  (TT496 29.79‰ vs TT492 29.79‰) — permille CANNOT separate within a class,
  only across classes.
- First gate deal LOSE 70v72 (`log/abtest-...-2026-07-26-15-19-21`): both
  losses were wall coins on BYTE-IDENTICAL trajectories (rbsat-v1375 base
  1757.8 s = 42 s margin; VdW-22 base 1603 s, 0 of 77 binaries removable —
  zero formula edits, the documented flipper). Second deal: both symmetric
  (VdW solved both arms — cand 56 s faster; rbsat TIMEOUT both arms).
  Textbook ±2 deal-noise; the re-gate was justified by the byte-identity
  proof and won cleanly.
- Failed-literal units are real but scoped out below threshold: reconf10
  x2 have 1074 units each (0.4-0.5‰), goldcrest 18, g2-oski15a10 122,
  jkkk 84, twitter 25. All those cells are SOLVED (reconf10/jkkk/twitter,
  fat margins) or timeout-and-stay-timeout (goldcrest/g2) — a units-only
  adoption arm would reroll 5 solved SAT cells for unproven gain. Screen
  before ever trying it.

Validation: 695 unit tests (6 new: removal, failed-literal unit, threshold
identity, duplicate-pair survivor, no-op, end-to-end SAT), smoke 9/9 both
flag states, shipped defaults reproduce the gate candidate digit-exact
(sted2 1,761,498 conflicts adopted; rbsat 17,758,017 props untouched).

## SESSION 5 (2026-07-26) — REDUCE law ported, mechanism REAL, gate LOSE 70v71, CLOSED at 1800 s

HEAD b8495b7 (groundwork commit, defaults byte-identical to ac0e675 —
digit-exact conflicts/props/ticks verified on rbsat+booth at 400k).
Baseline stays 72/100 (lineage) / 71 in this session's gate deal.

**`SAT_REDUCE_FRACTION` — kissat reduce.c deletion law (fraction-ramp
50→90% of candidates via `high-(high-low)/log10(reductions+9)` + 31-step
`used` counter, born/bumped 31, −1 per reduce, tier1 exempt while used>0,
tier2 while used>=30, tier3 always candidate; no budget gate; hard-budget
emergency trigger dropped while active). Scoped activation: first reduce
>= `SAT_REDUCE_FRACTION_MIN_CONFLICTS` (1.3M) AND never
`inprocess_aggressive`-armed; warm-starts all live used counters to 31 on
activation. Cadence was already kissat-parity (1000·sqrt(reductions)) —
only the deletion law differed.**

- **Mechanism is REAL.** Fixed-conflict idle probes: learned DB 2–4.5x
  smaller in literals everywhere, RSS −20..−40%, wall −19.3% rbsat /
  −17.3% sted2 / −14.6% bubble at identical conflict counts; vex/oski15
  wall-neutral (props/conflict grows to offset shorter watch lists).
  In-gate: 59-129706 −295 s, VdW −209 s *despite* +497k conflicts,
  case20 −121 s. The 2.2–9x throughput-gap hypothesis (clause-DB size →
  watch-list length) is CONFIRMED as mechanism.
- **Gate LOSE 70v71** (`log/abtest-candidate-vs-baseline-2026-07-26-08-19-42`,
  zero correctness failures, vex checker-timeout symmetric-documented,
  64/70 both-solved trajectory-identical — the scope banked TT/armed/
  sub-1.3M cells digit-exact as designed, verified in-gate). Tier-2
  +1.85M (ENTIRELY sted2's reroll +2.22M; excluding it tier-2 improves
  −372k), PAR-2 +1941. The −1 is rbsat: baseline SAT 1757.4 s (43 s
  margin, the documented thinnest coin), candidate's post-1.3M reroll
  lost the SAT draw. Zero capability gained → no trade available.
- **REROLL-VARIANCE LAW (new, the session's key result):** deep SAT
  cells reroll with ±2x conflict variance PER DRAW under any reduce-law
  change, and the draw sign is not controllable by scoping axis. Sweep
  of T ∈ {1.3M, 4M, 7M} × {cold, warm-start} (all deterministic idle
  probes): rbsat LOST its draw in 3/3 rerolls that touched it (TIMEOUT
  at 9.4–9.9M conf); 59-129706 swung from −1.65M (T=1.3M cold) to
  +8.5M (7M cold) to +11.7M (1.3M warm); sted2 +2.2M/+2.85M in both
  draws; VdW −1.28M warm after +497k cold. UNSAT rerolls by contrast
  are conflict-neutral-to-good with wall gains (MVRR −22k warm,
  bp4_BC012 +67k warm, both wall-faster in-gate). The warm-start
  (anti-mass-eviction) fix is mechanically correct but does NOT tame
  SAT-draw variance.
- **Verdict: promotion at the 1800 s gate is a lottery purchase — any
  winning shape rests on one favorable SAT draw (the class the
  searched-reroll session banned). CLOSED at 1800 s.** The law's value
  is at >3000 s horizons (competition scoring) where the deep-tail
  throughput compounds and single-deal coins wash out; revisit ONLY
  under a 3600 s/5000 s objective, or as insurance-only with
  T >= 8M (fires exclusively past every solved cell's conflict count —
  a strict no-op at 1800 s, deep-tail insurance beyond it).
- Surface: `SAT_REDUCE_FRACTION` (off), `SAT_REDUCE_FRACTION_MIN_CONFLICTS`
  (1.3M), `SAT_REDUCE_LOW`/`SAT_REDUCE_HIGH` (500/900 per mille),
  `reduce_fraction_activated_at` in stats JSON. 689 unit tests (7 new),
  smoke 9/9.

## SESSION 4 (2026-07-25 evening → 07-26) — three negatives, zero promotions, baseline stays 72/100

HEAD fd68696 (groundwork commit, defaults unchanged). Nothing promoted; the
value of this session is three closed lines with mechanism evidence, one
standing-trap correction, and a committed triage suite
(`benchmarks/timeout-subset-2026-07-25`, 28 cells, relative symlinks).

**1. `SAT_ELIM_DEF` — CLOSED at ANY tick budget. The ranked-item-1 framing
("20x budget gap was an unfair test") was WRONG; arming and the resolvent
cap are the blockers, not budget.** Probes (400k/4M-conflict, digit-exact):

- The density-class timeout targets (booth x2, Bubble, fixedbandwidth,
  goldcrest, g2-oski) do NOT arm by 400k conflicts; booth/Bubble arm at the
  SECOND yield probe (~800k). goldcrest (474 conf/s) reaches 800k conflicts
  only at ~1700 s — the mechanism never fires there in a gate.
- Timetable class: TT492 runs exactly 20,000 checks / 0 found → the
  formula-adaptive probe-cutoff stops it (pivots not kitten-definable).
- Once armed, elim_def FINDS definitions at 99% hit rate (booth 2383/2399,
  Bubble 2938/2950) but converts ZERO under the default parent-length cap —
  and a found-but-rejected definition BLOCKS the naive-BVE fallback for that
  pivot, so capped elim_def eliminates FEWER vars than base (booth 1581 vs
  1662). The default-off shape is actively harmful, not just useless.
- `SAT_ELIM_DEF_NOCAP=on` converts, but the yield is 3 orders of magnitude
  short (+56/+109 def elims, net +43/+4 total at 4M conflicts, vs kissat's
  72-77% collapse) and NOCAP is the documented oski40 killer.
- Tick budget 50k vs 1e6: BYTE-IDENTICAL runs (same propagation count);
  whole-run def kitten spend is 34-138k ticks. Budget was never binding.
- If elim_def is ever revisited: fix the rejected-definition fallback first
  (fall through to naive BVE on bound rejection), then re-measure.

**2. `SAT_VIVIFY_TIER_SPLIT` (kissat 3:3:1:3 tier schedule, armed-scoped) —
implemented, mechanism REAL, gate LOSE 70v70 on conflicts; DO NOT enable.**
Artifacts: `log/abtest-cand-vs-base-2026-07-25-22-05-56`, zero correctness
failures (vex checker-timeout symmetric both arms, the documented event).

- Mechanism probe (fixed conflicts): tier3 — which the legacy single pass
  NEVER vivifies (learned cap LBD<=6) — hits 28-63% of scanned candidates:
  booth 20k → 197k total strengthenings (10x), oski15b20 +24%, TT492 +26%,
  vex +15%.
- Gate: 70 v 70 solved, both-solved conflicts +1.05M WORSE, PAR-2 +2354
  WORSE, and the solved-set swap is a FAILING trade: LOST TT496 (base SAT
  1111 s, fat margin, the kissat-impossible banked capability cell — it is
  decision-armed, so tier-split rerolled exactly it) for oski15a01b20s
  (documented wall coin, kissat 574 s).
- LESSON (repeat of the sweep-budget lesson, now with vivify): raw yield is
  an ACTIVITY PROXY. 10x strengthenings churned armed trajectories
  net-negative. Any future vivify-depth idea must protect decision-armed
  banked cells (TT406/TT496) by scoping, and be judged on conflicts, never
  on strengthening counts.

**3. `SAT_BUMP_SORT_CACHE` (10th wall-diet attempt) — identity-exact but
wall-NEUTRAL; the visible sort cost is irreducible by key-caching.**

- perf is blocked on this host (perf_event_paranoid=4, no sudo). Built a
  gdb SIGINT-sampler instead (works under ptrace_scope=1 because gdb is the
  parent; `handle SIGINT stop print nopass` + external kill -INT loop —
  note `noprint` implies `nostop`, which silently yields ZERO samples).
- 400/173-sample profiles of bp4_TCO_CSO_ZR / rbsat-v1375: 75-85% propagate,
  9-13% analyze, no side-pass fat left. The one non-primitive item:
  `bump_analyzed_variable_activity`'s per-conflict
  `sort_unstable_by_key(stamp)` = 4.3% / 6.9% of wall.
- The diet (cache (stamp,var) pairs; >=2 zero stamps → legacy call verbatim
  so tie permutations cannot diverge) is digit-exact on 4 cells but wall is
  NOISE (booth +1.9%, bp4 −1.1%, rbsat +3.1%, TT492 +6.2%): the cost is the
  sort's own compare/swap work — which kissat also pays (radix) — not the
  key recomputation. Committed default-off; do not gate.
- Conclusion for the wall-diet arc: after nine diets the band cells'
  profiles are propagation-bound with primitives already lean. The 2.2-9x
  throughput gap vs kissat is the clause-DB size / REDUCE control law
  (ranked item below), not per-instruction overhead. The cheap 10th diet
  probably does not exist.

**4. STANDING-TRAP CORRECTION — bp4_TCO_CSO_ZR's 1880 s trajectory is GONE.**
Idle 3600 s re-run at HEAD (scoped gate-BVE adopts it, +3.16% elimination):
TIMEOUT. The reroll lost the cell even at 2x the gate wall (it was also
TIMEOUT in both arms of this session's gate and in the aa9f4d6 winning
deal). Ranked-plan items built on "bp4_TCO_CSO_ZR solves at 1880 s
deterministically / free +1 via 5% wall diet" are DEAD. If the cell is ever
wanted back, the lever is its gate-BVE dry-run decision (adopt threshold or
a per-cell arming-time discriminator), not wall.

Validation state at HEAD: 688 unit tests (5 new this session), smoke 9/9,
both new flags default-off with shipped trajectories byte-identical.

## SESSION 3 (2026-07-25) — SCOPED GATE-BVE PROMOTED, baseline now 72/100

**PROMOTED aa9f4d6: `SAT_GATE_BVE_SCOPED` default-on — the 2026-07-24 ranked
item 1, implemented exactly as projected, gate WIN 72 v 71.**
Artifacts: `log/abtest-cand-vs-base-2026-07-25-13-59-16` (cand/base TSVs),
`promotion_gate=PASS`, zero contradictions, zero correctness failures.

Mechanism (all in solver12 `.rs`, ~300 lines):

- Two-phase root pass in `maybe_scope_gate_bve` (main.rs, called right before
  the real `eliminate(true)`): snapshot the live root-simplified clauses
  (satisfied clauses skipped, false literals stripped), build TWO throwaway
  sub-solvers via `Solver::new_with_config`, run `eliminate(false)` with
  `ProofLog::disabled()` — plain (E0) and gate-aware (E1) — then set
  `self.gate_bve = true` for the real run only when
  `(E1-E0)*100 >= E0*SAT_GATE_BVE_MIN_GAIN_PCT` (default 2%).
- `SAT_GATE_BVE_SCOPED_MAX_VARS=100k` cap: bigger formulas skip the dry-run
  entirely. This is TRIPLE protection: zero dry-run wall on big marginal
  cells (vex, oski15, TT406), hard byte-identity for the gate-3 reroll
  casualties (TT496 260k, bp5_CSO 380k, VexRiscv 723k vars), and a memory
  guard. Verified in-gate: bp5_CSO conflicts EXACT-identical both arms.
- Decisions are tick-budgeted, wall-free → deterministic across load/deals.
  Dry-run cost: ≤5.4s (bp4_BC012), typical <1s, 0.1s on rbsat.
- Explicit `SAT_GATE_BVE=on` supersedes scoped mode (normalized in env parse,
  no config error) so the unconditional variant stays usable for A/B arms.
- Observability: `gate_bve_dryrun_e0/e1`, `gate_bve_scoped_adopted` in stats
  JSON; `c gate_bve_scoped e0=... adopt=...` line under SAT_TRACE_PREPROCESS.

Gate result detail (single deal, 1800 s/16 GB/32 pinned):

- **GAINED 2 (both fat-margin capability):** `RoundRobin_n16_d13` UNSAT
  **119.4 s** (FIRST-EVER in-gate solve; kissat cannot at 3600 s; proof
  drat-trim VERIFIED standalone, 90MB) and `bp4_TCO_CSO_IXA_LP_ZR` SAT
  **237.0 s** (kissat-only cell, kissat needs 1187 s).
- **LOST 1:** `bp4_BC012_CSO_FPBEQ_FPBLE_ZR` (base SAT 211.5 s, real margin —
  a genuine capability loss). This is EXACTLY the casualty the 2.6c
  discriminator predicted (+48% gain yet still dies); pre-judged a
  defensible trade (+2 fat capability −1) per the CLAUDE.md flexible rule.
- Tier-2: both-solved conflicts **−1,088,186** over 70 cells
  (58/70 trajectory-identical vs gate 3's 40/67); wall −481 s;
  PAR-2 126512.9 vs 130449.2 (−3936). Every lexicographic tier improves.
- Decision scan (100 cells, SAT_LIMIT_CONFLICTS=100): **19 adopt** — the two
  winners, the whole bp4 family (+2.8–57%), sted2 (+180%), aaai10 (+21.5%),
  both sqrt-miters, div172, Pancake/Bubble, jkkk, twitter, TC-256, circuit,
  booth_wallace_mapped (+2.6%); 42 dry-run-but-reject, 39 cap-skip.
- Reroll winners besides the flips: aaai10 −411 s, **sted2 1555→1199 s (OUT
  of the 1600–1800 coin band — hardened)**, twitter −266 s, bp4_CSO_IXA
  −194 s. Reroll losers (all still solved, fat margins): jkkk +327 s
  (7→334 s), TC-256 +191 s, sqrt-miters +20 s.
- vex verify=checker-timeout in BOTH arms — the documented
  historical/symmetric event, not a gate failure.

Validation chain: 680/680 unit tests (3 new: adopt-on-gain, size-cap,
reject-when-no-gain), smoke 9/9, dry-run reproduces every 2.6c measured
number digit-exact (RoundRobin 116/223, bp4_TCO 25353/26065, bp4_BC012
16106/23852), TT496+rbsat digit-exact identity at 100k conflicts.

## TL;DR — what happened this session (2026-07-24)

Ran both solvers over the full 100-instance medium suite at **2x the gate
timeout (3600 s), 16 GB/job, 32 pinned cores, sequential** (each solver had
the idle host to itself; same methodology as `plan/gap-read-2026-07-21.md`).

| solver (3600 s)     | solved | SAT | UNSAT | PAR-2      |
|---------------------|:------:|:---:|:-----:|-----------:|
| solver12 @ b671ae0  | **73** | 41  | 32    | **225298** |
| kissat 4.0.4        | **75** | 42  | 33    | 226904     |

- **kissat +2 solved; solver12 better PAR-2 (−1606).** Zero SAT/UNSAT
  contradictions on 66 both-solved cells; solver12 verify_fail = 0.
- solver12 gained **+3** from the extra hour over the 70/100 lineage:
  bp4_TCO_CSO_ZR (SAT 1880 s), **BubbleVsPancakeSort_7_6 (UNSAT 2880 s,
  20.1 M conflicts — FIRST-EVER solver12 solve)**, rbsat-v1375 (SAT 1780 s —
  its usual coin, in this deal). Nothing lost.
- kissat gained **+7** from the extra hour over its 68/100 1800 s run:
  Kakuro-132 (3259 s), case1 (1917 s), VdW-22 (2565 s), TT492 (2222 s),
  booth_wallace (3131 s), booth_dadda_origin (1864 s), pj2008 (2866 s).
  **kissat's marginal tail is ~2x fatter than ours** — at SAT-comp 5000 s
  conditions the gap widens; capability work beats coin-tuning long-term.

### THE decision number — same-deal truncation curve

Truncating THIS 3600 s deal at virtual cutoffs (one run, so zero
deal-to-deal lottery — the cleanest s12-vs-kissat comparison we have):

| cutoff | solver12 | kissat | delta |
|-------:|---------:|-------:|:-----:|
|  300 s | 45 | 39 | −6 |
|  600 s | 55 | 54 | −1 |
| 1200 s | 66 | 60 | −6 |
| **1800 s** | **71** | **67** | **−4** |
| 2400 s | 72 | 70 | −2 |
| 3000 s | 73 | 73 | 0 |
| 3600 s | 73 | 75 | +2 |

**solver12 is AHEAD at every cutoff through 2400 s and only crosses over at
~3000 s.** We are not behind kissat — we are a different shape: faster to
close what we can close, thinner in the long tail. Framing for any
"is more iteration worth it" call: at the 1800 s gate we already win; the
deficit is purely a >3000 s tail phenomenon, i.e. it only matters if the
target is competition-realistic (5000 s) scoring.

### Result files
- solver12 TSV: `log/seedgate-default-2026-07-24-07-24-29/results.tsv`
- kissat CSV:   `log/kissat-medium-20260724-102838/results.csv`
- per-cell:     `log/gap-read-2026-07-24/per_cell_comparison.csv`
  (also joins the 1800 s runs: lineage `log/abtest-cand-vs-base-2026-07-23-21-23-54/cand`
  and `log/kissat-medium-20260721-130444`)
- report cmd:   `python3 tools/gap_read.py --solver <tsv> --kissat <csv> --timeout 3600`

### Capability map at 3600 s (time-immunity now measured, not inferred)

**solver12-only (7):** xor_op n36/n40 (1–2 s), tseitin_n188 (44 s),
MVRoundRobin_n16_d10 (173 s), oddball_80 (254 s), TT496 (1076 s — kissat
cannot even at 3600 s), **bp4_TCO_CSO_ZR (1880 s — NEW unique cell; kissat
times out at 3600 s)**. NOTE: Kakuro-132 and case1 dropped OFF the unique
list — kissat solves them with 2x time (3259 s / 1917 s), though solver12 is
still 12x / 7x faster there.

**kissat-only (9), split by what they need:**
- *Pure capability gap (kissat ≤1400 s, solver12 dead even at 3600 s):*
  fixedbandwidth (1153 s), goldcrest-and-14 (1185 s), bp4_TCO_IXA_LP
  (1187 s), booth_dadda_mapped (1372 s). These four are the sharpest
  inprocessing-gap targets.
- *kissat itself needed >1800 s:* booth_dadda_origin (1864 s), TT492
  (2222 s), lockchart-group1 (2770 s), pj2008 (2866 s), booth_wallace
  (3131 s). Not 1800 s-gate losses; same mechanism class though.

**both-timeout hard core (18):** TT495 (NOBODY solves, even at 3600 s),
TT7F-33-24B, ramsey x2, clqcl x2, rphp5 x2, VdW-27, RoundRobin_n16_d13,
lockchart-group3, rbsat-v945, g2-oski15a10-k20, bp4_LPI_FPBEQ, st_659,
oisc-subrv, stp212, tseitin_grid_n400 (arc CLOSED — do not revisit).

### The throughput gap, re-measured on identical-outcome cells

kissat faster on 41 of 66 both-solved cells. The dense/margin band:

| cell | s12 | kissat | ratio |
|------|----:|-------:|:-----:|
| BubbleVsPancakeSort | 2880 s | 319 s | **9.0x** |
| sted2 | 1667 s | 468 s | 3.6x |
| rbsat-v1375 | 1780 s | 569 s | 3.1x |
| oski15a01b20s | 1615 s | 574 s | 2.8x |
| vex (VexRiscv) | 1657 s | 755 s | 2.2x |

**FIVE solver12 cells now sit in the 1600–1900 s band of the 1800 s gate**
(bp4_TCO_CSO_ZR 1880, rbsat 1780, sted2 1667, vex 1657, oski15 1615). Each
is an exactly-deterministic trajectory whose solve is a wall coin at 1800 s.

## IMPLEMENTATION DELTA vs kissat 4.0.4 (source-audited 2026-07-24)

Full audit of both codebases this session (solver12 46k lines Rust; kissat
39k lines C). **The remaining gap is mostly configuration plus one
scheduling bug — NOT missing algorithms.** This section replaces the vaguer
"carried kissat gaps" bullets in prior aggregates.

### DOC WARNING — read source, not FEATURES.md

`FEATURES.md`, `FEATURES.csv`, `CONFIG_SCHEMA.csv` are STALE. Authoritative
defaults are `src/config.rs` (`impl Default for SolverConfig` ~L705-840 +
`apply_profile_defaults` L895-1094) and raw `env_bool_or_default` reads in
`src/main.rs` ~L3600-3970. Specifically wrong in the docs:
`SAT_CONGRUENCE`/`SAT_CONGRUENCE_XOR`/`SAT_FACTOR` are **default-ON** in
source but documented Experimental; `SAT_TSEITIN`, `SAT_ENDGAME`,
`SAT_SWEEP`, `SAT_WALK`, `SAT_ELIM_*` and the whole arming layer are in
**no** doc file. **`SAT_PROBE` is IMPLEMENTED and works** (see below) —
prior plans calling it ParkingLot were repeating the stale doc.

### 1. The one real architectural difference: what the budget clock counts

kissat denominates every simplifier in **ticks** (propagation work,
`(watchlist_bytes >> 7) + 1` per literal), taken as a fixed per-mille share
of search ticks since that technique last ran, floored at 10M ticks
(`mineffort`), via `kimits.h SET_EFFORT_LIMIT`:

| technique | per mille | share |
|---|--:|--:|
| eliminate / vivify / sweep / forward-subsume | 100 | 10% |
| factor / walk | 50 | 5% |
| backbone / transitive | 20 | 2% |
| substitute | 10 | 1% |

solver12 denominates in **conflicts**: `inprocess_interval_conflicts =
1_000_000` flat (10k first round only for arming-flagged formulas).
Consequence is structural, not a tuning delta: kissat keeps accruing ticks
on slow-conflict instances and still inprocesses; we simply never fire.
goldcrest (474 conf/s) and lockchart (330 conf/s) reach ZERO inprocessing
rounds in a full run.

CORRECTION to earlier notes: kissat's `eliminateinit=500` / `probeint=100`
are NOT raw conflict counts — `kimits.c kissat_scale_delta` multiplies by
>=25 (formula-size quadratic in log10 clauses), so real first fire is
~12,500 conflicts (BVE) and ~2,500 (probe), then growing NLOG2N / NLOGN.

### 2. Sweeping — crippled by SCHEDULING, sub-solver is fine

kitten exists and works (`kitten.rs` 868 lines vs kissat `kitten.c` 2877).
kissat `sweep.c schedule_sweeping` keeps a PERSISTENT schedule: leftovers
from the previous round to the front, all other candidates radix-sorted by
occurrence count ascending, per-variable `sweep` flags for completion, and
on each COMPLETED sweep the bounds escalate — env vars double 256->8192,
clauses 1024->32768, depth 2->3 (`sweepvars/maxvars/clauses/maxclauses/
depth/maxdepth`), budget `sweepeffort=100` per mille of kitten_ticks.

solver12 `sweep_round` (`main.rs:10468`): `for seed in 1..=nvars` capped at
`SWEEP_SEED_BUDGET = 512`, **restarting at variable 1 every round**, no
completion tracking, no escalation ladder, and it clones the entire
original clause DB into a snapshot per round. On a 100k-var instance it
re-sweeps the same ~512 lowest-numbered vars forever. THIS is the 450x
productivity gap (0-826 equivalences vs 90k-18M kitten solves).

### 3. Already at PARITY — do not spend effort here

- **Tier limits**: both derive tier1/tier2 from a glue-usage histogram at
  the 50%/90% percentiles with 2/6 floors (ours:
  `compute_tier_limits_from_histogram`, `main.rs:11357`; kissat `tiers.c`).
- **BVE bound ladder** 0->1->2->4->8->16 on round completion (ours via
  `SAT_ELIM_ARMED_BOUNDS`, armed formulas only).
- **Propagation throughput**: 5.62M props/s vs kissat 5.1M on pj2008.

### 4. Genuinely different control law: REDUCE (best throughput hypothesis)

kissat `reduce.c` deletes a FRACTION of reducibles, ramping 50% -> 90% as
`high - (high-low)/log10(reductions+9)` (`reducelow=500`, `reducehigh=900`
per mille), keeping anything with `glue<=tier1 && used>0` or
`glue<=tier2 && used>=30`, where `used` is a 5-bit counter (`MAX_USED=31`).

solver12 `reduce_db_lbd_tiered` (`main.rs:11511`) deletes down to a LITERAL
BUDGET (`learned_lit_budget`), worst-LBD-first, with
`MAX_USED_RECENTLY = 3`. A budget-driven law and a fraction-driven law
diverge on long runs, and a 3-step vs 31-step usage counter is a much
coarser retention signal -> longer DB -> longer watch lists. This is the
shape of the observed 2.2-9x slowdown at identical conflict counts.

### 5. Built but SWITCHED OFF (gate runs, not development)

`SAT_PROBE` (root failed-literal probing, `main.rs:6318`, proportional
5M-100M tick budget — NOT in the runtime rejection list, it works),
`SAT_GATE_BVE`+`SAT_GATE_EXTRACT` (gate-aware/Plaisted-Greenbaum BVE),
`SAT_ELIM_DEF` (kitten definition extraction = kissat `definition.c`),
`SAT_ELS` as a standalone root pass, `SAT_FACTOR_INPROCESS`.

### 6. Actually ABSENT (runtime-rejected, `config.rs:1752`)

`SAT_HBR`, `SAT_TRANSITIVE`, `SAT_FORWARD_SUBSUME`, `SAT_RCHECK`; plus BCE
(denylisted name only). NOTE kissat has no standalone HBR module either —
the failed-literal role lives in `backbone.c` (binary-implication-graph
only, 2% effort) and `transitive.c` (2% effort). Small cheap passes, not
big ports.

### 7. Vivification granularity

kissat runs FOUR rounds per invocation (tier1, tier2, tier3, irredundant)
splitting one budget 3:3:1:3 (`vivifytier1/2/3=3/3/1`, `vivifyirr=3`) with
unspent slack carried forward. solver12 does originals + tier1/tier2
learned only, learned delayed to 6M conflicts. **tier3 is never vivified.**

### 8. Our side of the ledger (no kissat counterpart at all)

GF(2) Gaussian refutation w/ pure-resolution DRAT (`SAT_GAUSS`),
closed-Tseitin extended resolution (`SAT_TSEITIN` — outside kissat's proof
system entirely), adjacent-pair parity abstraction
(`SAT_PAIR_ABS_REFUTE`), the endgame rephase latch (`SAT_ENDGAME`), and the
per-formula arming/routing layer (`SAT_DECISION_ARM`,
`SAT_VIVIFY_YIELD_ARM`, deep-phase sweep guard, congruence dry-run
threshold).

## SESSION 2 (2026-07-24 evening) — three gates, one breakthrough

Everything below supersedes the earlier ranking in this same file. Full detail:
`plan/kissat-gaps.md` sections 2.1a, 2.2a-c, 2.6a-c.

**THE RESULT: `RoundRobin_n16_d13` UNSAT 80.7 s under gate-aware BVE. It was in
the both-timeout hard core — NOBODY solved it, kissat included, even at 3600 s.
First solve in project history and an outright capability win over kissat.**
Alongside it `bp4_TCO_CSO_IXA_LP_ZR` SAT 238.9 s (kissat-only, kissat 1187 s),
reproduced twice with the SAT model drat/model-VERIFIED.

Three gates run, none promoted:

| gate | candidate | verdict | why not promoted |
|------|-----------|---------|------------------|
| 1 | `SAT_SWEEP_SCHED=retire` + budget 2048 | **LOSE 64 v 69** | killed the miters (sqrt170/171, div172, Pancake) + TT496; both-solved wall +10.6% |
| 2 | `SAT_SWEEP_SCHED=retire` (budget 512) | WIN 69 v 67 **but rejected** | all 4 flips are wall coins (sted2 landed 1791 s of 1800); tier-2 conflicts +2.6M, wall +5.6%, only 54/66 trajectory-identical |
| 3 | `SAT_GATE_EXTRACT+SAT_GATE_BVE` | LOSE 69 v 71 | 2 real capability gains, but 4 reroll casualties with LARGE margins; only 40/67 trajectory-identical |

**THE LESSON, three results agreeing: DEPTH IS THE LEVER, FREQUENCY IS NOT.**
Sweep re-scheduling and the tick cadence are neutral-to-negative; the first
DEPTH change flipped two cells on its first attempt. The earlier ranking in
this file (sweep #1, cadence #2, depth #3) was WRONG — depth was always #1.

**Deal-noise calibration (important):** the same baseline scored **67, 69, and
71** across the three gates, same host, same commit, same suite. **±2 solved
cells is deal noise.** Do not read a 1-2 cell delta as signal without tier-2
conflicts, wall, and a mechanism.

Also landed: `CLAUDE.md` + `plan/solver-optimization-workflow.md` now carry the
flexible trade rule (lose up to N=2 wall coins for mechanism-validated
capability; wall coin = margin <=~120 s OR flipped across deals at an IDENTICAL
conflict count), tiered triage (probe -> subset -> 100-cell gate for promotion
only), and 4-arm sweeps (promote the best arm).

## RANKED PLAN (updated 2026-07-30 — rebuilt from the full-bench read)

The medium-1800 s gate stays THE promotion metric (unchanged). But the
full-bench read reopens ranked work that the medium suite could not see:
out-of-sample capability, memory, and detector coverage. Items 1-3 are
medium-gate-safe by construction (additive detection / correctness /
memory — identity elsewhere); items 4+ are analysis or need new scoping.

1. **php/counting-detector coverage pass (cheapest capability, proven
   SESSION-11 shape, near-zero reroll risk).** Decode offline why these
   both-timeout cells decline: cliquecoloring_n14_k7_c6 (n15/n26
   siblings FIRE — boundary bug?), clqcl_30_9_8, clqcl_30_11_10 (k=9/11
   — the histogram precheck allows 3-7 longest covers; check slot-count
   generalization), harder-fphp-016-015 (direct php shape, not
   relativized — new but simpler matcher), rphp_p25_r25 (P=H? then NOT
   a counting core — check before building). Each decode is minutes
   (SESSION-11 method: clause-length histogram + literal roles on the
   shuffled instance). Expected +2-4 full-bench; medium unaffected;
   gate = identity check + additive wins. NOTE: nobody (kissat incl.)
   solves any of these at 3600 s — same unique-capability shape as
   TSEITIN/PHP promotions.
2. **Giant memory diet — NOW +1-METRIC-MOVING (full-bench), was
   capability-adjacent.** Concrete target: pj2016_k100 parse/setup
   virtual footprint (8.8M vars / 23M clauses: >16 GB virtual at 53 s,
   RSS 12.7 GB @150 s; kissat solves SAT 1568 s in-budget). Measure the
   parse→root-simp transient (VmPeak vs post-parse steady RSS) first;
   the fix is likely a transient doubling (occurrence build / arena
   growth policy), not steady-state. Secondary: the
   tseitin_d3_n100000 late-phase explosion (RSS trajectory probe
   running 2026-07-30; suspect learned-DB/inprocessing pass on an 8.7 MB
   formula — hygiene + possibly protects other cells). pj2008-class RSS
   (10.4 GB vs kissat 1.4 GB) is the same arc.
3. **Bound the two runaway passes (bug fixes, root-caused, backtraces
   archived).** (a) `sweep_round`'s legacy `prove_facts` path enters
   kitten with an effectively unlimited budget (sweep.rs:256 wrapper) —
   give it the same tick budget as the budgeted core and check
   wall/limits between kitten solves; then re-probe battleship
   (kissat 21 s / 665k conflicts — plausible +1 once the solver
   actually searches). Identity risk: bounding CHANGES trajectories on
   any cell whose sweep kitten call previously ran long — screen with
   the digit-exact identity refs (rbsat/MVRR) plus the frontier suite;
   cells where the old budget was effectively never hit stay
   byte-identical. (b) `try_gauss_refute` needs a size cap (decline
   >~30k XOR equations) and a tick budget + wall checks inside
   `min_degree_order` and the elimination loop (gauss.rs:474/533) —
   fixes the tseitin_d3_n100000 rc-6 and stops burning root time on
   large XOR cells (grid_n250/n400 likely pay this tax too). Both are
   correctness-adjacent (an OOM abort in a proofs-required setting is
   a lost cell, and one is a whole-cell wall sink).
4. **Out-of-sample scoping question — run the >3000 s bundle A/B on the
   full bench (analysis-only, zero gate risk).** Arms: default vs
   REDUCE_FRACTION+SWEEP_SUBST+TICK_CADENCE(+root ELS, +unscoped probe)
   at 3600 s on a ~60-cell frontier subset (see item 5). The probes say
   these exact mechanisms are why 20+ cells go to kissat (bv_ILA x2,
   n320p5q2, uniqinv40, blockpuzzle, oddball-ttf x5, miter x14 class).
   If the bundle recovers >=10, the promotion path is NOT enabling it
   at 1800 s (measured negative there) but designing per-formula
   STRUCTURAL scoping (e.g. arm by formula class detected at root, not
   by suite-tuned yield thresholds) — the medium coins stay protected
   by their existing byte-identity scopes.
5. **Build `benchmarks/frontier-2026-07-30` (~60 cells) as the standing
   out-of-sample triage suite:** the 23 kissat-only <=600 s cells + the
   14-cell multiplier-miter class + oddball-ttf x5 + the 5 php
   near-misses + 8-10 protected solver12-only cells (regression canary:
   RoundRobin/clqcl/oddball-tto_zp/TT496 representatives). Purpose:
   every future scoping decision gets an out-of-sample read, so we stop
   overfitting medium-100 (three sessions of scope-tuning produced a
   suite that we WIN 76v75 while losing the bench 261v296).
6. **bp4_BC012_CSO_FPBEQ_FPBLE_ZR regression autopsy (medium cell,
   gate-relevant).** Was SAT 205 s on 07-24; now 3600 s TIMEOUT at
   3600 s idle-equivalent. Bisect the promotion chain (aa9f4d6 →
   d46f988) on this cell at 3600 s; check its scoped-gbve dry-run
   adoption decision. If it is a gbve adoption casualty like
   bp4_TCO_CSO_ZR (SESSION 4), the two cells together justify
   revisiting the adoption threshold/arming-time discriminator with a
   2-cell recovery target.
7. **Walk scale-up screen (SAT frontier class).** kissat walks 100-360M
   steps on Circuit_multiplier24/29, sted2_0x0_n219-342, ITC2021,
   HCP-446 SAT wins; our armed walk fires 3-27M on the same cells.
   4-arm triage on those 6-8 SAT cells at 3600 s: default /
   SAT_WALK unscoped / walk_effort x4 / + warmup. Known trap:
   SAT_WALK_WARMUP measured negative on medium (kills TT-class
   lottery) — scope any candidate away from decision-armed cells.
8. **Checker-timeout proof-size risk (4 at-risk solves):** valves-gates
   (3400 s), ncc_none_21015 (2950 s vs kissat 472 s), grs-160-48
   (2046 s vs 1161 s), VexRiscv (1635 s vs 512 s). Mostly a downstream
   symptom of item 4 (shorter search → smaller proof); no standalone
   work beyond tracking, unless the objective becomes
   proof-required competition scoring.
9. **Class decodes for later:** b18/b19 (giant ISCAS UNSAT, kissat
   360/820 s; s12 shows 24 decisions/conflict — density-armed shape),
   sorting-networks (Bubble_8_4/9_4/8_6), grs-32/256, crypto-arith x4,
   SGI/myciel/contest04 one-offs. Only after items 1-4.

## RANKED PLAN for next session (updated 2026-07-28d)

0. **DONE in SESSION 13 (zero promotions, zero gates spent):** the
   07-28c #1 (grid-n400 proof shrink) implemented — the shrink WORKED
   (1.92x, snake/valley layout, default-off `SAT_TSEITIN_SNAKE`) and
   the arc is nonetheless CLOSED PERMANENTLY: verification is
   checker-bound by the RAT-scan law (defs x maxVar, walk-independent
   ~160k defs on a 319k-var instance — idle verify exceeds the 3600 s
   budget even at half the lemmas). st_659 identified (digraph
   kernel/stable-model existence — no known certificate family). DONE
   in SESSION 12: refutation-family hunt closed; starved-cell pipeline
   negative; search/inprocessing axis closed (sessions 9/10/12).
1. **No concrete +1 is currently visible at 1800 s.** Both axes
   (search/inprocessing, special refutation) are closed with per-cell
   mechanism verdicts. The honest next moves, in order of expected
   value:
   a. **Watch for st_659 status** (probes this session; if kissat
      finds SAT, the cell formally leaves the refutation-target list).
   b. **Giant memory diet (carried).** pj2008 RSS 10.4 GB vs kissat
      1.4 GB; capability-adjacent, not metric-moving at 1800 s.
   c. **The 3600 s / competition-horizon bundle (carried, grown).**
      REDUCE law + tick cadence + SWEEP_SUBST + low transitive
      thresholds + tick-round sweep/elim — all measured real but
      valueless at 1800 s; revisit only under a >3000 s objective.
      NOTE: `SAT_TSEITIN_SNAKE=on` belongs here too ONLY IF a future
      checker change lands (its 7.6M-lemma grid proof would need
      ~2.5-3x more verify budget than 3600 s).
   d. **elim_def revisit ONLY behind the fallback fix (carried, low).**
2. **For any future ER family: budget by the RAT-scan law FIRST** —
   #definitions x instance maxVar decides verifiability, not lemma
   count. A family on a small-var instance (php-class, n188-class) is
   fine; anything grid-n400-scale (10^5+ vars x 10^5+ defs) is dead on
   arrival under drat-trim backward checking.

Previous ranking (2026-07-28a), kept for provenance:

0. **DONE in SESSION 10 (zero promotions, zero gates spent): the
   SAT-sweeping productivity arc.** SAT_SWEEP_ROOT (kissat-parity root
   sweep, all three kissat sweep designs implemented) — zero timeout
   flips on 14 idle 1800 s cells, no promotable adopter scope (yield
   threshold separates the wrong way, TT_C392 adopts at 139‰ while
   goldcrest sits at 31‰). SAT_SWEEP_SUBST (fixes the never-fired sweep
   substitution defect, kissat-parity substitution mass on BMC) —
   conflicts tier LOSES on the 2-coin forecast (+606k), wall wins big;
   the REDUCE-law verdict: CLOSED at 1800 s, filed in the >3000 s
   bundle. DONE in SESSION 9 (all NEGATIVE): units-only 3a, tick-cadence
   re-measure, gbve-adopter rounds, root ELS. DONE in SESSION 8: probe
   rounds (PROMOTED fe82400). DONE in SESSION 7: transitive rounds
   (PROMOTED ecdf632). DONE in SESSION 6: `transitive.c` root pass
   (PROMOTED ab592d2). The small-port class stays EXHAUSTED. CLOSED in
   SESSIONS 4-5: REDUCE law at 1800 s, `SAT_ELIM_DEF` (any budget),
   vivify tier3/3:3:1:3, 10th wall-diet, bp4_TCO_CSO_ZR free +1. Sweep
   schedule stays CLOSED — and with SESSION 10, sweep DEPTH at 1800 s is
   now closed too.

1. **Giant memory diet (unstarted).** pj2008 RSS 10.4 GB vs kissat 1.4 GB;
   BVE emits 1.7 GB discarded DRAT in 150 s. pj2008 is marginal even for
   kissat (2866 s at 3600 s), so this is capability-adjacent, not urgent.
   SESSION 5 note: SAT_REDUCE_FRACTION cuts search-phase RSS 20-40% on
   band cells if a memory-pressure (not metric) motivation ever appears.
2. **The 3600 s / competition-horizon question.** SESSION 5 makes the
   split concrete: the reduce law is worth ~15-20% deep-tail wall but
   cannot be promoted under the 1800 s single-deal metric. If the target
   ever shifts toward SAT-comp scoring (5000 s), re-run the truncation
   analysis with SAT_REDUCE_FRACTION=on at 3600 s — that is where its
   value lives (kissat's marginal tail is 2x fatter than ours; the law
   attacks exactly that). SESSION 6 adds: SAT_TRANSITIVE thresholds below
   100‰ (25-40‰) trade banked cells for deep-tail throughput — the SAME
   >3000 s-horizon shape; a lower threshold may pay at 3600 s where
   TT/bp4 reroll variance has room to wash out. SESSION 7 adds: the
   round pass would fire on MANY more cells at a lower root threshold —
   any future threshold experiment now sweeps both knobs together.
3. **Transitive follow-ups ALL CLOSED as of SESSION 9:** (a) units-only
   arm — screened LOSE 15v16 (SESSION 9.1), CLOSED; (b) DONE (SESSION 7);
   (d) DONE (SESSION 8); the "new root pass creates a new adopter class"
   corollary was tested on the scoped-gate-BVE class and FAILED
   (SESSION 9.3) — the shape requires percent-scale regenerating edge
   mass, not just a deterministic adopter set; (c) Kakuro-115 remains not
   worth a per-cell scope.
4. **elim_def revisit ONLY behind the fallback fix:** on bound-rejection,
   fall through to naive all-pairs BVE for that pivot (the current shape
   strictly loses eliminations vs base — SESSION 4 autopsy). Low priority;
   even NOCAP yield was 3 orders of magnitude short of the target class.
5. **TT496/bp4_TCO_CSO_ZR bookkeeping.** Both are RerollED cells: TT496
   still solves on the shipped default (banked — SESSION 6 re-confirmed
   the protection: it is byte-identical under the shipped T=100‰ scope,
   and DIES at T=25‰; protect it in every future armed-path or
   binary-graph change; SESSION 7's round pass cannot touch it — it is a
   non-adopter). bp4_TCO_CSO_ZR is LOST at 3600 s idle under the adopted
   gate-BVE trajectory — recovering it means revisiting its dry-run
   adoption decision (threshold/arming-time discriminator), not wall
   diets.

Historical detail of the promoted item (kept for provenance):

1. **SCOPED gate-aware BVE — THE #1 ITEM, projected 69 -> 72 (gate WIN).**
   Gate 3's losses are reroll casualties, not mechanism failures, and the
   discriminator is MEASURED (kissat-gaps 2.6c). Decisive datum: **bp5_CSO
   gate-eliminates 56 646 vars while its TOTAL elimination is byte-identical
   (122 262 -> 122 262)** — gate-BVE reaching the same vars by another route,
   pure trajectory churn for zero benefit. TT496 (+0.16%) and VexRiscv (+1.5%)
   are near-zero likewise, while the two winners sit at **+92%** and **+2.8%**.
   A **2% net-elimination-gain threshold** keeps both wins and skips 3 of the 4
   casualties.
   *Implementation:* two-phase root pass — plain BVE to completion recording
   E0, re-run from the ORIGINAL formula with gates on recording E1, apply the
   gated result only when `E1/E0 - 1 >= threshold`, else keep the plain result
   byte-identical. Root BVE is cheap (bp4_TCO_IXA spends 7.6M eliminate ticks),
   so the doubled cost is affordable. This is the established gate-safe shape
   (`CONGRUENCE_MIN_APPLY_MERGES=3000` all-or-nothing dry-run).
   *Tune with a 4-arm sweep:* thresholds 1% / 2% / 5% + base, on the ~30-cell
   timeout subset first, then gate the winner. Bead
   SAT-playground-5b2.3 child "Gate-aware BVE ... re-gate for default".
2. **Protect the reroll casualties explicitly.** bp4_BC012 gains +48%
   elimination and STILL dies — so gain does not predict success monotonically;
   the threshold works by filtering DEGENERATE cases, not by ranking. If
   bp4_BC012 remains a loss after scoping, that is 1 capability loss against 2
   capability gains: judge the trade per the new CLAUDE.md rule (it is
   defensible, but write it out). Consider also scoping by arming time, the
   trick that saved the endgame-delta promotion.
3. **`SAT_ELIM_DEF` (kitten definition extraction) — still unexplored.** It
   flipped nothing alone and added nothing on top of gbve in the depth probe,
   but it was only tested at default budgets (`SAT_ELIM_DEF_TICKS=50k`,
   `_CORES=2`). kissat gives definition extraction `definitionticks=1e6` and
   **10x that** for its 2 core-minimisation passes. Retry with budgets raised
   to kissat parity — a 20x budget gap is not a fair test. 4-arm sweep on
   ticks: 50k / 500k / 1e6 + base.
4. **DO NOT bundle the tick cadence with the depth passes — they are
   ANTAGONISTIC.** `tick+gbve` TIMED OUT where `gbve` alone solved. Same shape
   as gate 1. `SAT_INPROCESS_TICK_CADENCE` is implemented, correct, identity-
   verified and default-off; treat it as groundwork that is currently a dead
   end for the metric, and do not re-litigate it without a depth win first.
5. **Small ports (unstarted):** `backbone.c` (binary-implication-graph failed
   literals, 2% effort in kissat), `transitive.c` (2% effort), vivify tier3 +
   the 3:3:1:3 budget split. Cheap, additive, low reroll risk.
6. **Reduce control law (highest ceiling, highest risk).** Fraction-ramp
   50%->90% + 31-step `used` counter vs our literal-budget + 3-step. Best
   hypothesis for the 2.2-9x throughput gap. Measure OFFLINE first; rerolls
   every >=1M-conflict trajectory so it needs a deliberate re-luck campaign,
   not one gate.
7. **10th wall-diet (cheap fallback, still has a free +1).** bp4_TCO_CSO_ZR
   solves at 1880 s deterministically (2 008 325 conflicts) and kissat cannot
   do it at 3600 s, so ~5% wall is a capability-backed +1 with no reroll. Also
   hardens rbsat/sted2/vex/oski15 in the 1600-1900 s band. Use when items 1-3
   stall; do not lead with it.
8. **Sweep schedule — CLOSED for now.** `SAT_SWEEP_SCHED=retire` is
   implemented, identity-verified (legacy digit-exact on 7/7 fingerprints) and
   default-off. It is CORRECT but earns no default flip: gate 2's win was pure
   coin. Do not spend more time here unless a depth win changes the context.
   The seed budget is a genuine scaling defect (512 fixed = 17% coverage on a
   2948-var formula vs 0.07% on 723k; kissat uses `sweepeffort` per-mille of
   ticks, no seed count) but raising it LOST gate 1 badly.
9. **Giant memory diet (carried, unstarted).** pj2008 RSS 10.4 GB vs kissat
   1.4 GB; BVE emits 1.7 GB discarded DRAT in 150 s. pj2008 is marginal even
   for kissat (2866 s at 3600 s).
10. **TT class bookkeeping.** TT496 banked, re-confirmed kissat-impossible at
    3600 s — and it is a gate-BVE casualty, so protect it. TT492: kissat needs
    2222 s, not an 1800 s-gate loss. TT495: nobody solves at 3600 s.

## Current state

- HEAD: 97d80a6 SESSION 13 groundwork commit — defaults byte-identical
  to 1912628/d46f988 (rbsat 100k = 100001 conf / 196258 dec /
  17,758,017 props; MVRR 267,199 digit-exact; 735 unit tests; smoke
  9/9). New default-off flag SAT_TSEITIN_SNAKE (snake/valley ER proof
  layout, dry-run mode selection, legacy caps when off); php.rs test
  temp-file race fixed; new tests tseitin_proof_snake_grids_verified +
  tseitin_snake_shrinks_grid_proof. Artifacts:
  log/tseitin-snake-2026-07-28/. **Medium 1800 s baseline: 74/100,
  unchanged.**
- Prior HEAD: SESSION 12 groundwork commit — defaults byte-identical to
  d46f988 (rbsat 100k = 100001 conf / 196258 dec / 17,758,017 props;
  MVRR 267,199 digit-exact; 738 unit tests; smoke 9/9 both flag
  states). New default-off flags SAT_SWEEP_TICK_ROUNDS /
  SAT_SWEEP_TICK_PERMILLE / SAT_ELIM_TICK_ROUNDS / SAT_TSEITIN_NO_DEL;
  new stats sweep_tick_* x3 + elim_tick_rounds; new suite
  benchmarks/starved-screen-2026-07-28; artifacts
  log/starved-arc-2026-07-28 + the 4-arm screen dir. **Medium 1800 s
  baseline: 74/100, unchanged.**
- Prior HEAD: SESSION 11 promotion commit d46f988 — `SAT_PHP_REFUTE` default-on
  (php.rs detector + ER proof engine; frontend BVA held off matching
  formulas; feature table FullSetValidated). **Medium 1800 s baseline:
  74/100**, candidate TSV
  `log/abtest-cand-vs-base-2026-07-28-08-08-20/cand/results.tsv` (that
  deal's base arm posted 71 — rbsat/VdW/oski15 coins OUT, textbook ±2
  noise band around the 72 lineage). Both-timeout hard core now 14
  cells (was 18): rphp5 x2 + clqcl x2 REMOVED. Identity refs at HEAD
  unchanged: rbsat 100k = 100001 conf / 196258 dec / 17,758,017 props;
  MVRR 267,199 conflicts; 729 unit tests; smoke 9/9.
- Prior HEAD: ba928c1 SESSION 10 groundwork commit (defaults byte-identical to
  69ec5eb/fe82400 — new default-off flags SAT_SWEEP_ROOT (+
  _MIN_YIELD_PERMILLE/_MAX_VARS/_TICKS/_PROBE_ENVS), SAT_SWEEP_SUBST (+
  _MIN_EQUIVS), both measured non-promotable at 1800 s; new stats
  sweep_root_* x8; sweep.rs prove_facts refactored to a budgeted core
  with the unlimited-budget wrapper decision-identical). Identity refs
  re-verified at HEAD: rbsat 100k = 100001 conf / 196258 dec /
  17,758,017 props; MVRR 267,199 conflicts. **Medium 1800 s baseline:
  72/100 lineage, unchanged**; newest candidate TSV remains
  `log/abtest-cand-vs-base-2026-07-27-11-58-13/cand/results.tsv`.
- Prior HEAD: 69ec5eb (SESSION 9: groundwork only, defaults
  byte-identical to fe82400 — SAT_TRANSITIVE_UNITS_ONLY,
  SAT_TRANSITIVE_INPROCESS_GBVE, SAT_PROBE_INPROCESS_GBVE, all measured
  negative; stats JSON emits els_substituted_vars / els_rewritten_clauses
  + transitive_units_only_applied).
- Prior HEAD: fe82400 (SESSION 8: SAT_PROBE_INPROCESS default-on,
  root-adopter scope — only sted2 (−246k, 1 round) and ibm (+124k,
  7 rounds) moved, aggregate −121.6k; SAT_ELS_INPROCESS landed
  default-off, measured negative). Gate 72v72 identical sets, conflicts
  −121,608; that gate's deal posted 72 in BOTH arms.
- Prior HEAD: ecdf632 (SESSION 7: SAT_TRANSITIVE_INPROCESS default-on,
  root-adopter scope — only sted2/ibm trajectories moved, both improved;
  98/100 cells conflict-identical to ab592d2 behavior in-gate; gate
  72v72, conflicts −385k, PAR-2 −471.5, TSV
  `log/abtest-cand-vs-base-2026-07-26-23-11-04/cand/results.tsv`).
- Transitive surface: SAT_TRANSITIVE (on), MIN_REMOVED_PERMILLE 100,
  SAT_TRANSITIVE_INPROCESS (on), INPROCESS_MIN_REMOVED_PERMILLE 0,
  SAT_TRANSITIVE_TICKS 0 = proportional. Round pass fires only where
  `transitive_adopted=1` (stats JSON: `transitive_inprocess_rounds`).
  NEW (SESSION 8): SAT_PROBE_INPROCESS (on) and SAT_ELS_INPROCESS (off)
  share the same adopter scope (stats JSON: `probe_inprocess_rounds`,
  `els_inprocess_rounds`). Adopter trajectories at HEAD: sted2 1,246,166
  conflicts / ibm 762,991 / MVRR 267,199 / gm16 62 (digit-exact
  references for future identity checks).
- Prior HEAD: ab592d2 (SESSION 6: SAT_TRANSITIVE root default-on, T=100‰ —
  4 cells adopt, 96 byte-identical to b8495b7 behavior). Its gate deals
  posted base 72 (rbsat+VdW IN) and base 70 (both OUT) — ±2 deal noise on
  exactly the documented coins. The SESSION 5 gate's base arm posted
  71/100; SESSION 4's posted 70/100.
- New default-off flags this session: `SAT_BUMP_SORT_CACHE`,
  `SAT_VIVIFY_TIER_SPLIT` (+ `vivify_tier3_attempts`/`_strengthened` stats).
  New triage suite: `benchmarks/timeout-subset-2026-07-25` (28 cells).
- Scoped gate-BVE surface: SAT_GATE_BVE_SCOPED (on), MIN_GAIN_PCT 2,
  SCOPED_MAX_VARS 100k; explicit SAT_GATE_BVE=on supersedes scoped.
  19/100 cells adopt (see SESSION 3 list); 81 byte-identical to b671ae0
  behavior.
- Prior state (b671ae0): 70/100, lineage
  `log/abtest-cand-vs-base-2026-07-23-21-23-54/cand/results.tsv`.
  At 3600 s: 73/100 (2026-07-24 session; solver12 verify clean — PRE-scoped;
  re-measure at 3600 s only if a gap read is needed).
- Endgame surface: SAT_ENDGAME (on), TRIGGER 1, PARTS "rf", MIN_ARMED 100k,
  banded REPHASE_DELTA (decision-armed 48k / yield-armed legacy 50k),
  DELTA_SPLIT 500k.
- Decision metric UNCHANGED: lexicographic solved -> conflicts -> PAR-2 on
  the medium suite at 1800 s, 16 GB, 32 pinned cores. The 3600 s numbers are
  analysis-only — do NOT promote on them.
- **Decision PROCESS changed 2026-07-24** (`CLAUDE.md` "Judging Trades",
  "Candidate Triage Tiers", "Multi-Arm Sweeps"): do not revert on any loss —
  classify cells and judge the trade (lose up to N=2 wall coins for
  mechanism-validated capability); triage on a probe/subset before spending a
  100-cell gate; run up to 4 arms per sweep and promote the best.
- New default-off flags added this session (all identity-verified, none
  promoted): `SAT_SWEEP_SCHED` (legacy|cursor|retire), `SAT_SWEEP_SEED_BUDGET`,
  `SAT_INPROCESS_TICK_CADENCE`, `SAT_INPROCESS_TICK_INTERVAL`,
  `SAT_INPROCESS_TICKS_PER_CONF_MIN`.
- Gate artifacts this session: `log/abtest-cand-vs-base-2026-07-24-15-48-41`
  (sweep retire+2048, LOSE 64v69), `...-2026-07-24-18-28-40` (sweep retire@512,
  coin WIN 69v67, rejected), `...-2026-07-24-21-17-01` (gate-BVE, LOSE 69v71,
  the two capability gains).

## Standing traps (carried + this session)

- **SESSION 14:** the full-bench 3600 s numbers are ANALYSIS-ONLY — the
  promotion metric stays medium-1800 s (a −35 full-bench read coexists
  with a WON medium suite 76v75; do not "fix" full-bench cells by
  rerolling medium coins without the frontier-suite canary). `ulimit
  -v` kills on VIRTUAL memory — RSS readings understate the kill
  criterion; measure with /usr/bin/time -v + VmPeak. rc-6 results in a
  seedgate TSV = allocator abort (no `s` line), not a solver bug per
  se. SAT_LIMIT_WALL_SEC is honored ONLY in the CDCL loop — the sweep
  kitten path (sweep.rs:256 unlimited wrapper) and the gauss
  refutation path (gauss.rs min_degree_order/elimination) can run
  unbounded past it (battleship: whole 3600 s in one kitten solve;
  tseitin_d3_n100000: 25 min ordering + 31 GB fill-in). Screens
  relying on it under-read such cells; external `timeout` still
  bounds gates.
  `tools/run_kissat_full.sh` exists for full-bench kissat sweeps
  (`-d`/`-c`/`-j`); marginal-cell WALL times from the 07-29 concurrent
  run carry contention caveats, conflicts do not.
- **SESSION 13:** `SAT_TSEITIN_SNAKE=on` raises the tseitin caps to
  200k components / 8M lemmas — its grid_n400 proof GENERATES fine
  (34 s) but CANNOT be verified inside the 3600 s harness budget
  (checker-bound), so enabling it at 1800 s would convert that cell's
  timeout into a gate-level correctness FAIL (checker-timeout). Keep it
  OFF unless the checker changes. Never judge ER-proof viability by
  lemma count — use #defs x maxVar (the RAT-scan law). And never
  stream an aborted ER attempt into the live proof and retry: the
  aborted attempt's deletions of consumed axioms break the retry's RUP
  premises (`s NOT VERIFIED`, measured) — dry-run with sink emitters
  to select a layout.
- **SESSION 12:** `SAT_SWEEP_TICK_ROUNDS` / `SAT_ELIM_TICK_ROUNDS` are
  MEASURED NEGATIVE defaults at 1800 s (4-arm screen: 0-1/14 vs base
  3/14 — they reroll vex/oski15 coins away and flip nothing, confirmed
  idle). `SAT_TSEITIN_NO_DEL` is measurement-only and produces
  NOT-VERIFIED proofs by design (definition-var recycling needs the
  deletions for RAT soundness — deletions are load-bearing in every
  tseitin/gauss ER proof). And the ticks/conflict EARLY-SEARCH ratio
  (100k-conflict scan) OVERSTATES in-run tick arming: rbsat 19k proxy
  and lockchart-group2 29k proxy never arm in a full run, while vex
  15.2k does — enumerate arming with the real trigger counters, never
  the proxy.
- **SESSION 10:** `sweep_equivalences` in ANY historical stats output is a
  duplicate-inflated count of never-substituted facts (the learned-binary
  defect) — never read it as substitution mass; distinct-pair counts are
  1-2 orders smaller (booth 352 -> 11 distinct). `els_substituted_vars`
  is the real merge counter. And `SAT_SWEEP_SUBST=on` is a MEASURED
  NEGATIVE default at 1800 s (2-coin conflicts forecast +606k) despite
  kissat-parity substitution mass — do not enable without a >3000 s
  objective or a new scoping argument.
- **SESSION 10:** a root-sweep-style adoption threshold on probe yield
  separates the WRONG WAY across classes (TT_C392 139‰ > goldcrest 31‰)
  — the THRESHOLD LAW (SESSION 6) generalizes: per-mille find-mass
  thresholds cannot rank capability targets above protected solved cells,
  in either direction, for ANY root pass measured so far.
- **SESSION 9:** all three new flags (`SAT_TRANSITIVE_UNITS_ONLY`,
  `SAT_TRANSITIVE_INPROCESS_GBVE`, `SAT_PROBE_INPROCESS_GBVE`) are
  MEASURED NEGATIVE defaults — identity-verified groundwork only; do not
  enable without a new mechanism argument. Tiny formula edits (a dozen
  binary deletions) can swing a solved cell ±100-400k conflicts — never
  read a small-edit per-cell improvement as mechanism; demand find-mass
  (SESSION 7's 30-90k binaries/round) before believing a round pass.
- **SESSION 9:** the stats JSON did NOT emit `els_substituted_vars` /
  `els_rewritten_clauses` before 69ec5eb — any older scan that grepped
  them silently measured nothing (this cost one wasted 10-min scan).
  Check the emission list in `stats.rs` before scripting a stats grep.
- **SESSION 8:** `SAT_ELS_INPROCESS=on` is a MEASURED NEGATIVE default
  (idle 2-cell forecast +44k conflicts alone, +2.6M with probe — sted2
  ballooned to 4.49M conflicts / 1065 s under both). It exists as
  identity-verified groundwork only. Within the adopter class, per-cell
  reroll signs are uncontrollable (ELS helped ibm −292k while hurting
  sted2 +336k); judge any adopter-scoped pass ONLY on the 2-cell
  aggregate idle forecast, which has now matched the gate to the digit
  twice (SESSIONS 7 and 8).
- **SESSION 6:** a permille-of-removable-binaries threshold CANNOT separate
  cells WITHIN a structure class (TT496 vs TT492 both 29.79‰) — only across
  classes. And transitive-style trajectory rerolls flip NO timeout cell
  (conflicts deterministic: idle TIMEOUT ⇒ in-gate TIMEOUT); their only
  value is conflict/wall improvement on already-solved adopters. Judge any
  future binary-graph edit on that shape.
- **SESSION 6:** a gate deal can lose BOTH documented wall coins in one arm
  on byte-identical trajectories (deal 15-19-21: rbsat 42 s margin + VdW
  flipper, zero formula edits on either). When every flipped cell is
  proven byte-identical (digit-exact identity + zero dry-run adoption),
  one re-gate is the correct move, not a revert.

- **`FEATURES.md`/`FEATURES.csv`/`CONFIG_SCHEMA.csv` are STALE — never quote
  them for a default or a "not implemented" claim. Read `src/config.rs` +
  the raw env reads in `src/main.rs`. This trap already cost one wrong
  "SAT_PROBE is ParkingLot" line in a prior plan.**
- `results.tsv` written only at run END — monitor per-cell lines in launch
  logs instead.
- `pgrep -f feature_ablation` inside a monitor loop matches ITSELF; use
  `ps aux | grep "[f]eature_ablation.py"`.
- vex UNSAT checker-timeout is historical/symmetric load-lottery, NOT a gate
  failure (verify_fail=0 again this session at 3600 s).
- Conflict counts are EXACTLY deterministic across load; wall is not.
  Digit-exact identity checks (yield-protect + passthrough + default-equiv)
  for every scoped-reroll change.
- Wall-coin cells at the 1800 s gate, updated 2026-07-26: **rbsat-v1375
  (1780 s), vex (1476-1664 s), oski15 (1597-1663 s; b20s flipped IN only in
  the tier-split cand arm at 1316 s — still a coin), VanDerWaerden_pd_2-3-22
  (1718 s)**. sted2 LEFT the band under scoped gate-BVE (1199 s).
  **bp4_TCO_CSO_ZR left the COIN class entirely: it is now a 3600 s-idle
  TIMEOUT under the adopted gate-BVE trajectory (SESSION 4 re-measure) — do
  not count it as a margin cell.**
- Scoped gate-BVE dry-run decisions are DETERMINISTIC (tick-budgeted, no
  wall): the 19-cell adopt list is stable across deals; only their search
  trajectories reroll deal-to-deal, exactly like any other solved cell.
  Margins under ~120 s are load noise — but note vex/sted2/oski15 swing by
  100-300 s across deals, so the STRONGER coin test is "flipped across deals at
  an IDENTICAL conflict count" (see CLAUDE.md "Judging Trades").
- **DEAL NOISE IS ±2 SOLVED CELLS.** The same baseline scored 67, 69 and 71
  across three gates on 2026-07-24, same host/commit/suite. Never read a 1-2
  cell delta as signal without tier-2 conflicts, wall, and a mechanism.
- **Marginal-cell timing is INVALID while another 32-way sweep runs.** VexRiscv
  timed out in BOTH arms on "free" cores 40/42 while a gate saturated memory
  bandwidth on 0-31, though it solves ~1500 s idle. Under contention a SOLVE is
  trustworthy; a TIMEOUT is not.
- **Activity proxies mislead — never optimise them.** `sweep_equivalences` rose
  49-52x under a bigger seed budget and the gate LOST 64 v 69. Measure solved
  cells and wall.
- sqrt-mitern170 produced `verify=checker-timeout` in gate 2's cand arm (first
  time on that cell; drat-trim resource limit on a large proof, not an invalid
  proof — same class as the documented vex case, but watch it).
- `inprocess_rounds` is hardcoded to 0 in the stats JSON — useless as a proxy.
  Use `vivify_attempts` / `sweep_equivalences` / `gate_eliminated_vars` instead.
  Elimination keys are `pre_bve_eliminated_vars` and `gate_eliminated_vars`
  (NOT the Rust field names).
- Build to a scratch `CARGO_TARGET_DIR` when a gate is running, so the gate's
  binary is not swapped underneath it.
- `rm -rf` in scratch scripts is blocked by a guard — use fresh timestamped
  dirs instead of deleting.
- Arming times (idle, re-confirmed): instant: vex, oski15 x2. ~200k: TT406
  (200,057), TT492 (200,057), TT395 (200,191), TT496 (200,013). ~800k:
  sqrt170/171, pancake, QG7, aaai10, oddball24, div172.
- SAT_STATS_JSON=on emits to STDERR; timed-out runs emit NO stats JSON — use
  SAT_LIMIT_CONFLICTS (~400k) for arming stats on timeout cells.
- No `cargo build` while a gate runs; `pgrep -a sat-solver` before gates.
  Heredoc scratch writes flake — use the Write tool. A/B launcher: cd to
  repo root first.
- Kissat 3600 s sweeps: `tools/run_kissat_medium.sh -t 3600 -m 16000 -j 32`
  (~1.9 h); solver12 via seedgate `--timeout 3600` (~3 h incl. verify).
- perf is BLOCKED on this host (perf_event_paranoid=4, no passwordless
  sudo). Working substitute: gdb SIGINT-sampler (gdb is the parent →
  ptrace_scope=1 ok; `handle SIGINT stop print nopass` + N x `bt 40` +
  `continue` in a -batch script, external `kill -INT <inferior>` loop).
  Trap: `noprint` implies `nostop` — that variant yields ZERO samples.
  Script pattern in scratchpad gdb_sample.sh (SESSION 4).
- 900 s subset walls on TIMEOUT cells discriminate NOTHING under 32-way
  contention (SESSION 4's elim_def 4-arm sweep: 0/28 in every arm incl.
  base). For timeout-cell triage use SAT_LIMIT_CONFLICTS probes with stats
  JSON, not short-wall subset sweeps.

## solver12's capability edge (protect in rerolls)

**NEW 2026-07-28 (SAT_PHP_REFUTE, banked at promotion):** `rphp5_050`,
`rphp5_085`, `clqcl_40_6_5`, `clqcl_50_6_5` — all UNSAT <0.3 s,
drat-trim-verified ER counting proofs; nobody else solves any of them
at 3600 s. Reroll-immune by construction (pre-search refutation, strict
structural detection — but protect the php.rs detector invariants and
the frontend-BVA hold-off in any parse/factor refactor).

**NEW 2026-07-24 (gate-BVE, needs scoping to bank):** `RoundRobin_n16_d13`
UNSAT 80.7 s — nobody solves it, kissat included, even at 3600 s;
`bp4_TCO_CSO_IXA_LP_ZR` SAT 238.9 s (kissat 1187 s).

xor_op x2, tseitin_n188_d3 (SAT_TSEITIN), oddball_80_5, MVRoundRobin_n16_d10,
SC25_Timetable_C_406 (endgame rf), SC25_Timetable_C_496 (banded d48k,
1076-1111 s — kissat cannot at 3600 s; **decision-armed, the tier-split
gate's casualty — protect it in every armed-path change**),
RoundRobin_n16_d13 (gate-BVE, 119 s). **bp4_TCO_CSO_ZR is REVOKED from this
list (SESSION 4): the adopted gate-BVE trajectory times out at 3600 s idle;
neither we nor kissat solve it now.** Kakuro-easy-132 + case1 are
speed wins (12x/7x), no longer unique-capability — still gate +1s at 1800 s.

## Where the evidence lives

- This session: result files above; sweep driver pattern in
  `plan/next-plan.md` history; runs were sequential on the idle host.
- Mechanism deep dive (still the reference): `plan/gap-read-2026-07-21.md`,
  `log/gap-read-2026-07-21/deepdive/COMPARISON.txt`.
- Prior aggregates: `plan/next-steps-AGGREGATED-2026-07-23b.md` (and the
  chain below it).
