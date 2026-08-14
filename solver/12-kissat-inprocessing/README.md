# 12-kissat-inprocessing

Solver 12 starts as a direct copy of `11-kissat-search`, including the MiniSat `simp`-style bounded variable elimination baseline and the Kissat-style focused/stable search engine, so new Kissat-class inprocessing work can proceed without losing solver 11 fixes.

This iteration keeps the `09` CDCL core and adds a one-shot preprocessing phase before search. The
preprocessor is intentionally isolated in `src/simp.rs` so it can keep evolving toward the full
MiniSat `SimpSolver` design described in
[MINISAT_SIMP_PORT.md](/home/bojji/code/SAT-playground/solver/12-kissat-inprocessing/MINISAT_SIMP_PORT.md).

> ⚠️ **Search-feature efficacy verdicts are under fresh re-evaluation (2026-05-29, bead `SAT-playground-gbc`).**
> Efficacy claims embedded in the "Current State" prose below and in the feature ledger may be stale.
> The prior documented results were archived to `archive/efficacy-reeval-2026-05-29/`
> (**do not consult unless explicitly asked**). Treat any per-feature "helps/hurts PAR-2" statement as
> provisional until the re-evaluation completes.

## Current State

> **Full-bench default promotion (2026-08-13, SESSION 20g): sweep
> yield-escalate latch (`SAT_SWEEP_YIELD_ESCALATE=20`,
> `SAT_SWEEP_YIELD_MIN_EQUIVS=1000`).** Once a conflict-cadence sweep round
> proves >= max(1000, 2% of live vars) distinct equivalences past 100k
> conflicts, the run latches kissat-parity sweep completion: retire-mode
> scanning, the escalation ladder, wide environments (4096/16384/depth-5,
> 64 seeds), equivalence SUBSTITUTION, the fast kitten (phase saving +
> flip-literal candidate pruning + in-round repr streaming), and the
> aggressive inprocess cadence. Non-arming formulas pay nothing — the check
> is a post-round comparison, and rbsat/MVRR fingerprints are digit-exact
> under the default. **Capability: dislog_a14_x14_n24 (kissat-only,
> discrete-log) is cascade-solved — arming yield 111,962 equivalences
> (8256 permille), SAT at 1680/2398/2481 s across three runs, in-gate in
> BOTH full A/B deals.** HCP-446-105 additionally solves STANDALONE (SAT
> 2676-2730 s, model verified against all 247,657 clauses; formula
> collapsed 51%) but is wall-borderline under 32-way in-gate contention.
> Promotion trade (A/B `log/abtest-cand-vs-base-2026-08-13-07-58-47`,
> judged per "Judging Trades" with written justification): gained dislog +
> bp4_BC012 return; the raw 292 v 293 deficit is carried entirely by THREE
> PROVEN-NON-ARMING cells (sqrt-mitern169 probe = 7 equivs v the 1000
> floor; oddball_19_4 and reconf10 zero ARMED lines — byte-identical
> trajectories, pure contention coins, all documented flippers; N=3
> allowance). The 20-permille un-floored variant measured LOSE 290 v 295
> (`log/abtest-cand-vs-base-2026-08-12-18-25-30`) — the abs-1000
> calibration is load-bearing. SAT_SWEEP_YIELD_PROBE stays default-off
> (measured useless for HCP; costs wall on decliners). 761+5 tests, smoke
> 9/9. Detail: `plan/next-plan.md` SESSION 20 items 1-8.

> **Full-bench default promotion (2026-08-12, SESSION 19): frontier-sweep
> counting engine (`SAT_SWEEPCOUNT` default ON, `src/sweepcount.rs`).** A new
> pre-search refutation engine for exactly-one bipartite cover imbalance —
> the mutilated-chessboard class, exponentially hard for resolution/CDCL
> (neither solver12 nor kissat has ever solved mchess_20 at 3600 s). Detect:
> all-positive cover clauses + complete pairwise AMO binaries, every var in
> exactly two cells, 2-colorable cell graph, unequal color classes. Refute:
> sweep cells in a low-bandwidth order (input order first, BFS fallback)
> maintaining banded unary counters over the open-edge frontier; each cell's
> exactly-one advances the invariant FB−FW=δ by single-pass RUP through nine
> explicit lemma batteries (extend E1-E3, reverse H0/H1/REV, level-monotone
> M, removal bridge D1-D5, transfer T), ending in the empty clause at the
> swept-out boundary. Validated bottom-up with drat-trim forward checking on
> 4x4/8x8/20x20 synthetic boards (all s VERIFIED in-tests). **Real mchess_20:
> REFUTED in 0.011 s, 291,102-line proof, drat-trim VERIFIED (115.7 s
> standalone; verified 'ok' in-gate).** A/B
> `log/abtest-cand-vs-base-2026-08-11-19-37-55`: +mchess_20; ALL 291 shared
> solved cells conflict-IDENTICAL (the decline-is-identity construction
> proven at bench scale); raw count 293 v 294 because two documented
> thin-margin flippers (valves-gates 33 s, oddball_19_4 103 s) flipped on
> wall under contention — judged wall coins per the trade rule (N=2
> allowance, mechanism-validated capability gained; zero
> contradictions/correctness failures). Gap analysis vs kissat unchanged
> except mchess_20 moves from both-timeout to solver12-only. Also landed:
> `SAT_GAUSS_MIN_COVERAGE` env (default 900 unchanged; par32-2 measured
> honest-decline — its pure-XOR subsystem is consistent). 761+5 tests,
> smoke 9/9, identity refs digit-exact. Detail: `plan/next-plan.md`
> SESSION 19.

> **Full-bench default promotion (2026-08-09, SESSION 18): adaptive walk
> giveup (`SAT_WALK_STALL_GIVEUP` default 16).** Walking cannot refute UNSAT,
> but the SESSION 16b/17 latch gave walks to a class that mixes SAT
> walk-targets with UNSAT near-misses (RoundRobin/lockchart). The giveup
> abandons walking on the latch class once the best walk min-unsat stalls for
> K=16 consecutive walks (rate-based: a walk must lower it by ≥1/64 of the
> current best to count as progress, so an UNSAT-bound cell's marginal
> local-minimum creep still counts as a stall), returning the walk budget to
> CDCL, which is what actually refutes it. Byte-identical on SAT cells by
> construction (they drive min-unsat toward 0 and never stall) — the mechanism
> can only ever affect UNSAT-plateau walk waste, so the SESSION-17 SAT wins are
> untouched. Screen (`log/abtest-k4-vs-k8-vs-k16-vs-base-2026-08-09-04-16-51`):
> k8/k16 both 14/19 v base 13/19, K=16 the gentler winner. Full-bench A/B
> (`log/abtest-cand-vs-base-2026-08-09-06-42-44`, gate PASS, zero
> contradictions/correctness failures, NO SAT regressions): **cand 292 v base
> 291 WIN; gained RoundRobin_n17_d15 (UNSAT 2942 s — a FIRST-EVER, was
> both-timeout, kissat cannot solve it either) + mod2c-rand3bip; lost
> RoundRobin_n18_d15 (a same-family ~43M-conflict UNSAT grinder, 355 s
> thin-margin wall swap — both siblings land at the 3600 s wall and the
> walk-budget change flips which one makes it). PAR-2 945,714 v 949,626.**
> Full-bench lineage 290 → 292/400; gap to kissat 4.0.4 (296) now −4. Identity
> refs digit-exact (the giveup never trips on SAT/early-refuting cells).
> Detail: `plan/next-plan.md` SESSION 18.

> **Full-bench default promotion (2026-08-07, SESSION 17): walk-latch second
> wave — `SAT_WALK_WARMUP_UNARMED` default ON + `SAT_REPHASE_UNARMED_MIN`
> 1M → 500k.** Warmup (kissat warmup.c parity: complete the root assignment
> by decide+propagate-beyond-conflicts before each walk) was measured
> negative 2026-07-17 ON ARMED walkers only — the never-armed latch class
> could not walk then; the unarmed scope gives it to exactly the class the
> negative never covered, byte-identical armed walkers by construction. The
> 500k latch gives deep-unarmed cells more walk runway (ITC_Early_12 falls
> at 408-834 s from a 1.0-1.8M-conflict trajectory). Screen
> (`log/abtest-warm-vs-thresh-vs-warmthresh-vs-base-2026-08-06-23-35-05`):
> warmthresh 12/16 v base 9/16, each mechanism confirmed alone. Full-bench
> A/B (`log/abtest-cand-vs-base-2026-08-07-01-51-08`, gate PASS, zero
> contradictions): **cand 290 v base 285 WIN (+11/−6, PAR-2 955,537 v
> 993,612); gained ITC2021_Early_12 + bp4_BC012_CSO_FPBEQ (kissat-only),
> VanDerWaerden_pd_2-3-27_663 + lockchart-group2 x2 (FIRST-EVERS,
> both-timeout before), rbsat-v1375 (the flagship wall-coin flipper, now
> walk-solved in 4 consecutive deals/arms), reconf10 + frb80 (the 16b
> reroll losses recovered) + sum_of_3_cubes + valves + oddball_57; lost 6
> walk-lottery classmates (ER_400.apx_2, vmpc_28, oddball_56, bp4_IXA_LPI,
> mod2c, oddball_19_4 — all documented rebalance members, class-level net
> +9 across SESSIONS 16b+17). Checker-timeouts 3→7 (big-proof UNSAT cells,
> drat-trim budget — caveat class, none rejected.** Full-bench lineage
> 286 → 290/400; gap to kissat 4.0.4 (296) now −6. Identity refs
> digit-exact below the latch. Detail: `plan/next-plan.md` SESSION 17.

> **Full-bench default promotion (2026-08-06, SESSION 16b): deep-unarmed
> rephase/walk latch (`SAT_REPHASE_UNARMED_MIN=1_000_000` default ON,
> `SAT_WALK_EFFORT_UNARMED` default 200→50).** DISCOVERY: never-armed formulas
> structurally could not rephase or walk — `config.rephase` defaults off and
> only the arming/endgame paths enable the cycle, so the walk-scale SAT class
> ran ZERO walk steps forever (ITC/ER_400/mod2c measured rephases=0 at 1.2M
> conflicts; kissat walks 100-360M steps there), and the previously-promoted
> effort-200 unarmed knob was dead code (every rephase-enabled cell is
> `inprocess_aggressive`). The latch enables the kissat-parity rephase/walk
> cycle once a never-armed formula reaches 1M conflicts; unarmed cells
> finishing earlier stay byte-identical by construction (rbsat
> 100001/196258/17,758,017 and MVRR 267,199 digit-exact). Effort 50 is
> kissat walkeffort parity — the unarmedwalk screen measured 200 overwalking
> (e50 9/14 v e200 6/14 v base 6/14). Full-bench A/B
> (`log/abtest-cand-vs-base-2026-08-06-03-28-37`, 400x2 @3600 s, gate PASS,
> zero contradictions/correctness failures, checker-timeouts 5→4): **cand 286
> v base 281 WIN (+9/−4); gained ER_400_20_7.apx_1, sted2_0x0_n219, mchess-
> class mod2c, case8, fsf-300 x2 (ALL former kissat-only), 170223547
> (walk-solves in 51 s at the latch), bp4_BC012_AM_FPBEQ_ZR, mp1-Nb7T45;
> lost bp4_TCO (184 s coin), VdW-23 (walk-reroll, solved in the screen
> deal), reconf10_22, frb80; tier-2 conflicts −81.8M over the 47 changed
> both-solved cells, PAR-2 987,867 v 1,028,679.** Full-bench lineage
> 279 → 286/400; gap to kissat 4.0.4 (296) now −10. Detail:
> `plan/next-plan.md` SESSION 16b.

> **Full-bench default promotion (2026-08-03, SESSION 15): banded vivify-deduce
> (`SAT_VIVIFY_DEDUCE` default ON inside the late-armed band).** The kissat
> `vivify_deduce` mechanism (reason-cone shrink on conflict, implied-TRUE
> strengthen — built 2026-07-15, shelved after the UNBANDED armed screen lost on
> early armers) is promoted behind `SAT_VIVIFY_DEDUCE_ARMED_MIN=500_000`: it
> fires only on formulas whose `inprocess_aggressive` arming latched at >=500k
> conflicts (the SESSION 14d reduce-law discriminator), so every early-armed
> banked cell (TT/oski/vex/oddball class) keeps a byte-identical trajectory by
> construction. Mechanism (tier-1 probe, boothdadda29 @2.5M conflicts): vivify
> hit rate 14.8% -> 28.5% (kissat 34%), strengthened 27,491 -> 53,823, wall
> 318 -> 311 s. Miterded screen (23 cells, 4 arms, 3600 s,
> `log/abtest-ded-vs-bbded-vs-bb-vs-base-2026-08-02-17-45-21`): ded 8/23 v base
> 7/23, boothbit29 conflicts 8.97M -> 8.01M, lec_mult 8.46M -> 7.35M, all five
> early-armed canaries conflict-EXACT. Full-bench A/B
> (`log/abtest-cand-vs-base-2026-08-03-10-13-35`, 400x2 @3600 s, gate PASS,
> zero contradictions/correctness failures): **cand 279 v base 276 WIN; gained
> Circuit_multiplier24 (SAT 1917 s, fat margin) + BubbleVsPancakeSort_7_6
> (UNSAT 2274 s, fat margin) + bp4_BC012/bp4_TCO/valves retentions; lost
> MVRoundRobin_n14_d10_v2 (82 s base margin = documented-thin wall coin) +
> sum_of_3_cubes_37_bits_87 (SAT reroll; base solved at its stable 894,247-
> conflict trajectory, cand's changed deal lost it); tier-2 conflicts −14.7M
> across the 37 changed both-solved cells, PAR-2 1.041M v 1.057M.** Also landed
> (default OFF): `SAT_BACKBONE` — a full kissat backbone.c port
> (`src/backbone.rs`: stacked-probe binary-implication-graph failed-literal
> rounds, RUP units); tier-1 measured ZERO units on the miter class (kissat's
> own backbone finds 2 there), re-confirming the 2026-07-15 "killed without
> building" verdict — kept as a zero-mutation groundwork rider with
> `SAT_BACKBONE_SCOPE/_ARMED_MIN/_EFFORT/_TICKS/_ROUNDS/_MAX_ROUNDS` knobs.
> Validation: 756+5 tests, smoke 9/9, identity refs digit-exact both flag
> states (rbsat 100001/196258/17,758,017; MVRR 267,199). Detail:
> `plan/next-plan.md` SESSION 15.

> **Medium-suite default promotion (2026-07-28): pigeonhole-counting ER refutation (`SAT_PHP_REFUTE`).**
> Detects two relativized-pigeonhole clause shapes at the root — the `rphp` family
> (P pigeons -> N resting places -> H holes with used-place guards) and the `clqcl`
> family (P clique slots -> N vertices with existential edge literals -> H colors) —
> by strict all-or-nothing structural matching: literal-based (invariant under
> shuffling and per-variable polarity flips), every clause required for the counting
> argument verified by exact lookup, which makes detection itself the soundness
> anchor (a matching formula with P > H is UNSAT by construction). Refutes with an
> extended-resolution DRAT proof: fresh definitions `W[p][r][h] ~ a & hole` and
> `G[p][h] ~ OR_r W` (RAT on the leading fresh literal), pairwise blocking lemmas at
> the W then G level (RUP), per-pigeon covers, and an injective-assignment DFS over
> the (H+1)xH G-matrix ending in the empty clause. Frontend BVA is held off matching
> formulas (factoring rewrites the clqcl adjacency ternaries and hides the shape).
> The histogram precheck (3-7 strictly-longest covers >= 10 wide, all else <= 8
> literals) passes on EXACTLY the 4 family cells of the 100-cell medium suite, so
> the other 96 pay one length scan and stay byte-identical (rbsat 100k fingerprint
> 100001/196258/17,758,017 and MVRR 267,199 digit-exact both flag states).
> Gate `log/abtest-cand-vs-base-2026-07-28-08-08-20`: **WIN 74 vs 71 solved**
> (+rphp5_050 +rphp5_085 +clqcl_40_6_5 +clqcl_50_6_5, all FIRST-EVER solves, all
> UNSAT in <= 0.3 s with drat-trim-verified proofs in-gate — kissat 4.0.4 solves
> none of them even at 3600 s; -oski15a01b20s, the documented wall-coin flipper,
> lost at its exact reference conflict count 2,663,684 = pure wall luck), 70
> both-solved cells ALL conflict-identical, PAR-2 115260.5 vs 128218.6;
> `promotion_gate=PASS`, zero contradictions, zero correctness failures (vex
> checker-timeout symmetric both arms as documented). Proof sizes 0.6-1.8 MB
> (106k-300k lemmas), emission < 0.08 s. Disable with `SAT_PHP_REFUTE=off`.
> Detail: `plan/next-plan.md` SESSION 11.

> **Medium-suite default promotion (2026-07-22): closed-Tseitin ER refutation (`SAT_TSEITIN`).**
> Detects closed Tseitin XOR components (every variable in exactly two extracted XOR
> constraints; union-find + odd rhs-parity charge ⇒ UNSAT) and emits a width-bounded
> extension-variable DRAT proof: equations are summed in a greedy min-cut-growth connected
> order, the partial-sum row is compressed into prefix-accumulator chains ordered by next
> use (forward pointer-pair consumption, pointer parking, aggressive deletion lines for
> drat-trim backward-check speed). Capped at 20k-equation components / 6M proof lemmas so
> every emitted proof stays verifiable under the harness's 1800 s drat-trim budget.
> Gate `log/abtest-cand-vs-base-2026-07-22-14-52-12`: **WIN 70 vs 68 solved**
> (tseitin_n188_d3 TIMEOUT → UNSAT 44 s — kissat 4.0.4 cannot solve it either;
> plus the documented oski15 load-lottery flip), both-solved conflicts EXACT tie
> (trajectory-identical elsewhere), PAR-2 139761.9 → 134611.0; `promotion_gate=PASS`.
> tseitin_grid_n12_m12 also drops 5.65 s → ~1.5 s (proof 1.26M → 10.7k clauses).
> tseitin_grid_n400_m400 is provable (14.6M lemmas, 22 s) but deliberately left
> unsolved: its proof cannot be backward-verified within the checker cap and a
> `checker-timeout` on UNSAT is a gate correctness failure. Disable with `SAT_TSEITIN=off`.
> Detail: `plan/next-steps-tseitin-2026-07-22.md` (bead `SAT-playground-kk8`).

> **Default profile (2026-05-31): `fstab_lbdtier` promoted.** The `default`/`fast` profiles now run
> focused-stable search + LBD + tick mode-switching + LBD-tiered reduction (VMTF auto → focused-only),
> plus `lucky` (promoted earlier, 70h). Promoted from the first feature ablation on the new
> `benchmarks/profile20` suite (10 easy controls + 10 hard headroom instances): fstab_lbdtier wins
> aggregate PAR-2 by 2x over the prior single-mode default (5653 vs 6808, 13/20 vs 10/20 solved) and
> clears the solver-10 floor (6773). The win is `SAT_REDUCE=lbd-tiered` (cracks 3 hard headroom
> instances), amplified by focused-stable+VMTF halving the easy-half overhead. Promoted on the
> Stage-1 (n=1 screening) decision; a formal `check_promotion_gate.py` gate run was not recorded.
> Provenance: `log/feature-ablation-2026-05-30-12-11-01/FINDINGS.md`. To recover the old behavior:
> `SAT_PROFILE=baseline`, or `SAT_SEARCH_MODE=single SAT_MODE_USE_TICKS=off SAT_REDUCE=legacy SAT_USE_LBD=off`.

> **Medium-suite default promotion (2026-07-09): adjacent-pair parity abstraction.**
> `SAT_PAIR_ABS_REFUTE` is enabled in the `default` and `fast` profiles after the
> `sat-comp-2025-medium` single-seed A/B gate improved solved count from `55/100` to `58/100`
> with identical both-solved conflicts and PAR-2 `180659.0 -> 170286.2`.
> The feature targets complete expanded clauses over adjacent pair parities, introduces fresh
> parity variables, emits a pure-resolution DRAT lift from the concrete expansion to the compact
> abstract CNF, solves that abstract CNF, and maps its UNSAT proof back to the fresh variables.
> Evidence: `log/abtest-pairabs-vs-base-2026-07-09-08-20-53`; `check_promotion_gate.py`
> reported `promotion_gate=PASS`. Disable with `SAT_PAIR_ABS_REFUTE=off`.

> **Medium-suite default promotion (2026-07-11): learned-vivify binary gate.**
> The `default`/`fast` profiles run guarded 1M-conflict inprocessing with SAT sweeping and
> vivification. Learned-clause vivification stays delayed until 6M conflicts, but now becomes
> originals-only on post-preprocess binary-dominated formulas (`binary_fraction >= 0.85`). This
> preserves the low-binary `sted2_0x1e3-216` solve that needs learned vivify while recovering the
> binary-heavy `59-129706` trajectory where learned candidates churned the implication graph.
> Full `sat-comp-2025-medium` single-seed A/B:
> `log/abtest-cand-vs-base-2026-07-11-08-54-35` kept solved count tied at `61/100` and improved
> both-solved conflicts `57,134,586 -> 52,607,869`; `check_promotion_gate.py` reported
> `promotion_gate=PASS`. Disable learned candidates entirely with `SAT_VIVIFY_LEARNED=off`; disable
> only the binary-fraction gate for before/after replay with `SAT_VIVIFY_LEARNED_BINARY_GATE=off`.

What is present:

- original-clause occurrence lists and literal occurrence counts during preprocessing
- a separate decision-variable flag so eliminated variables do not re-enter the branch heap
- bounded variable elimination with MiniSat-style `grow = 0` and `clause_lim = 20`, enabled by
  default and disabled with `SAT_BVE=off`
- MiniSat-style backward subsumption / BSR, enabled by default and disabled with
  `SAT_FULL_BSR=off`
- preprocessing can be bypassed for comparison runs with `SAT_SIMPLIFICATION=off`
- 64-bit clause abstraction prefiltering for preprocessing subsumption checks
- large-formula preprocessing stores original-clause abstractions inline to avoid sparse arena-indexed
  side-table loads in hot BSR scans
- sorted-clause subsumption relation for long clauses on large canonical formulas
- in-place original-clause strengthening during BSR
- one-pass strengthened-clause compaction/proof logging metadata updates
- lazy occurrence-list membership cleanup for large formulas after clause strengthening
- lazy preprocessing watcher detach on small formulas and the large inline-preprocessing path
- a persistent preprocessing loop over touched variables, root assignments, queued subsumption
  clauses, and a dynamic elimination heap
- resolvent insertion through a preprocessing original-clause path, with generated clauses queued
  for immediate subsumption work
- parse-time canonical original-clause insertion for input clauses: duplicate literals are removed,
  tautologies / already-satisfied clauses are skipped, root units are enqueued immediately, and
  surviving clauses use the same normalized representation as preprocessing-generated clauses
- `SAT_INITIAL_CLAUSE_MODE` switch for initial clause loading experiments:
  `canonical-sorted` (default/baseline), `input-order`, `kissat-watch`, `raw`, or `auto`
  (currently an alias for `canonical-sorted`)
- DRAT logging for preprocessing-generated resolvents/units
- DRAT-verified adjacent-pair parity abstraction refutation for expanded `xor_op`-style UNSAT
  formulas (`SAT_PAIR_ABS_REFUTE`, default/fast)
- MiniSat-style elimination stack entries and SAT model extension
- SAT output from a complete model snapshot instead of the mutable live assignment vector
- one-shot cleanup after preprocessing: drop occurrence metadata, rebuild branch heap, and force GC
- lazy deleted-clause watcher cleanup during propagation, with strict detach retained where
  preprocessing removes an original clause before tombstoning it
- opt-in LBD metadata and LBD-tiered reduction state. Newly learned clauses initialize their
  `used_recently` counter to the solver's maximum for every LBD tier, matching Kissat's
  maximum initial learned-clause retention semantics before reduce-DB aging decides eviction.
  `SAT_LBD_UPDATE_REASONS=on` keeps reason-side LBD improvement scoped to conflict analysis.
  Learned-reason LBD recomputation now walks arena clauses directly instead of allocating a
  temporary literal vector per reason. LBD-tiered reduce-DB also reuses a persistent delete-marker
  table and compacts `learned_clause_ids` in place instead of allocating a dense delete vector for
  every reduction pass.
  `SAT_LBD_UPDATE_PROP_REASONS=on` is a separate experimental extension that recomputes learned
  propagation-reason LBD after the implied literal is enqueued, lowers the stored LBD/tier if the
  current assignment gives a better glue value, and marks learned propagation reasons recently used
  in lbd-tiered runs. The propagation-time extension remains isolated after profile testing showed
  that enabling it in the lbd-tiered feature mode regressed the current profiling suite.
  The LBD-tiered reducer now computes focused-mode and stable-mode tier thresholds from recent
  glue-use histograms at the start of each LBD reduction pass: tier 1 covers the first 50% of
  recently used learned-clause glue counts, tier 2 covers the first 90%, and both thresholds keep
  the original `2/6` constants as minimum floors. The same pass reclassifies live learned clauses
  under the current mode's thresholds and ages `used_recently` for every scanned kept learned
  clause, so clauses are protected for the current pass but do not stay protected indefinitely.
  `SAT_REDUCE=lbd-tiered` uses a Kissat-style conflict-count reduction schedule: the first
  reduction is scheduled at `SAT_REDUCE_DB_INIT` or 1,000 conflicts, later reductions are scheduled
  at the current conflict count plus `sqrt(reduce_db_calls) * SAT_REDUCE_DB_INTERVAL`, and the hard
  learned-literal budget is retained only as an emergency trigger. A minimum conflict interval
  guard prevents repeated high-LBD emergency reductions from firing more often than
  `SAT_REDUCE_MIN_INTERVAL` conflicts; lbd-tiered mode defaults this guard to `100`.
- an LBD EMA restart policy (`SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable
  SAT_RESTART=kissat-ema`) for focused/stable Phase 1 search experiments. Single-mode
  `SAT_RESTART=kissat-ema` is rejected because that path lacks Kissat's focused-mode envelope and
  restart trail-reuse semantics. An optional Glucose-style decision-level blocker can suppress EMA
  restarts when the recent decision-level EMA is high relative to the slow baseline; set
  `SAT_RESTART_BLOCK_MARGIN` above `0` to enable it. The blocker is default-off after profile
  testing showed the `1.4` margin regressed the current profiling suite. The slow LBD EMA window
  defaults to `4096` conflicts; a Kissat-style `100000` window remains available as an explicit
  `SAT_EMA_SLOW_WINDOW` experiment, but it over-restarts the current partial focused/stable port
  and times out on the profiling `case9` instance.
  Restart trail reuse is available for the focused/stable search path; stable-mode reuse is
  deliberately not applied to the solver-10-compatible single-mode Luby path because that hybrid
  preserved a harmful SAT prefix on `mp1`.
  Single-mode search still defaults to legacy Luby restarts for solver-10 parity. Built-in profiles intentionally do not bundle
  `SAT_RESTART=kissat-ema` with target-phase policies; `SAT_PHASE=target-then-saved` remains an
  explicit opt-in when EMA restarts are active because HWMCC-style instances have regressed under
  that combination.
- opt-in saved/target/best phase selection policies via `SAT_PHASE`, with legacy saved-phase
  branching kept as the default for solver-10 parity. Target/best phase policies require
  `SAT_SEARCH_MODE=focused-stable`; single-mode search accepts only `legacy` and `saved`.
  In focused/stable search, target/best phase
  snapshots are captured only while stable mode is active, matching Kissat's update boundary; target
  phases persist across mode switches and restart cycles, then reset when a rephase event starts a
  new phase block. Single-mode target policies are rejected after a current Sudoku repro produced
  `UNKNOWN` where the default profile solves.
- focused/stable search-mode scaffolding with focused EMA restarts and stable reluctant restarts.
  Env-facing `SAT_SEARCH_MODE=focused-stable` now enters the actual focused/stable path and defaults
  to focused-only VMTF unless `SAT_VMTF=off` is explicitly requested for an ablation. This matches
  Kissat's focused-mode branching model instead of the rejected VSIDS-in-focused hybrid.
  The `default` and `fast` profiles use the focused-stable search path with LBD, tick-based
  mode-switching, and LBD-tiered reduction (the "fstab_lbdtier" config), promoted from the profile20
  Stage-1 ablation on 2026-05-30/31 (see the "Default profile" note in Current State). The earlier
  single-mode default was demoted after profile20's headroom half showed fstab_lbdtier wins
  aggregate PAR-2 by 2x. The `baseline` profile remains the solver-10-compatible single-mode path.
  Entering focused mode resets the LBD EMA restart averages so focused-mode restart calibration
  does not inherit stable-mode glue. Entering stable mode refreshes the VSIDS heap from current
  variable activities before stable-mode decisions resume. Focused EMA restarts now use Kissat's
  soft throttle: after each focused restart, the minimum EMA interval becomes
  `50 + kissat_logn(focused_restarts) - 1`.
  Stable reluctant restarts now use Kissat's `reluctantint=1024` scale instead of raw Luby counts,
  which fixed the focused/stable case9 wall-clock UNKNOWN caused by hundreds of thousands of stable
  restarts. Focused mode also has Kissat-style random decision sequences and the focused phase cycle
  that periodically forces initial and inverted-initial phases by mode-switch count.
- Kissat-style mode scheduling is available for focused/stable search. The implementation keeps
  focused-mode switches conflict-gated with
  `nlogpown(count, 4)` interval growth, but gates stable-mode duration on propagation search ticks.
  Tick mode also resets all restart EMAs on every mode switch.
- Variable-Move-To-Front branching. Focused mode uses the VMTF queue and move-to-front conflict
  bumps without updating VSIDS scores, while stable mode uses the VSIDS heap and score bumps.
  Conflict-bumped variables are now moved in existing queue-stamp order, matching Kissat's
  `sort_bump` behavior; the old scratch-order movement could invert the focused queue and produced
  `UNKNOWN` on Sudoku. `SAT_VMTF=single` remains a default-off experimental fallback for the
  solver-10-compatible single-mode path; it is bounded by fixed budgets and should not be treated as
  the promoted VMTF policy.
- opt-in periodic decision-order reordering (`SAT_REORDER=on`). Stable mode rebuilds the VSIDS
  heap from current variable activities; VMTF mode rebuilds the queue in the same activity order.
  The interval is controlled by `SAT_REORDER_INTERVAL_CONFLICTS` and the default profiles leave it
  off until a promotion-safe interval is validated.
- focused/stable rephasing is available behind `SAT_REPHASE=on`. It runs only on scheduled stable-mode restarts and cycles saved phases through
  best, inverted, and original polarity sources
- opt-in guarded chronological backtracking (`SAT_CHRONO=on`) that keeps only `current - 1`
  instead of the normal assertion level when the learned clause remains asserting there; it falls
  back to ordinary non-chronological backtracking unless the level gap exceeds
  `SAT_CHRONO_MAX_DELTA` (default `5000`) or the learned clause would stop being unit. The larger
  default keeps chrono opt-in conservative on solver 12's deep-level trajectories.
- opt-in binary implication fast path (`SAT_BINARY_FAST=on`) that keeps binary clauses in the arena
  for proof/model/debug traceability while propagating them through stable binary IDs and implication
  edges; default propagation remains the legacy watched-clause path. Clause minimization is
  binary-reason aware and remains controlled separately by `SAT_CLAUSE_MIN`; binary-fast env runs
  preserve the configured clause minimization mode unless `SAT_CLAUSE_MIN=off` is explicit, because
  disabling minimization silently can move baseline-solved rows to `UNKNOWN`. The default
  `inblock-late` mode keeps recursive minimization active and runs Kissat-style level-block shrink
  only after `SAT_INBLOCK_DELAY_CONFLICTS` conflicts on formulas whose pre-preprocess binary-clause
  fraction is at least `SAT_INBLOCK_BINARY_MIN`. Eager `inblock` remains available as an opt-in
  diagnostic for shrink experiments.
  With `SAT_OTFS=on` and clause minimization enabled, newly learned non-unit clauses also run a
  bounded recent-clause subsumption pass modeled on Kissat's eager learned-clause window: only the
  last four remembered learned clauses are candidates, and candidate clauses must be within four
  extra literals of the new learned clause. Deletion is also gated on LBD metadata, so metadata-free
  default runs do not discard clauses whose quality cannot be compared. The pass refuses live reason
  clauses and emits DRAT deletion records before tombstoning subsumed learned clauses. It
  intentionally does not scan all watcher lists; the earlier watcher-wide version was too aggressive
  and moved mp1/velev into baseline-solved timeouts. The feature remains default-off after enabled
  profiling regressions.
- post-preprocess formula classification in `SAT_STATS_JSON` and `SAT_TRACE_PREPROCESS` output:
  solver 12 records size class, Kissat-style `small`/`bigbig` flags, binary-clause fraction,
  average clause size, and live-variable density. The class now drives adaptive defaults such as
  BSR formula gating and the learned-vivify binary-dominated gate.
- guarded inprocessing in the `default`/`fast` profiles. The scheduler fires at a 1M-conflict
  cadence, runs SAT sweeping unless the deep-phase guard sees a nearly complete SAT-looking prefix,
  and runs vivification. Learned-clause vivification is enabled only on long searches (6M-conflict
  delay), restricted to low-LBD tier1/tier2 clauses, and suppressed on binary-dominated formulas so
  original-clause vivification still runs without learned-clause churn.
- an opt-in pre-search lucky assignment pass (`SAT_LUCKY=on`) that runs after preprocessing and
  before CDCL search. It tries all-true/all-false and forward/backward false/true temporary
  propagation probes, then a bounded small-formula local repair fallback. It captures a SAT model
  only after a full residual-formula satisfaction check and restores temporary propagation state
  before returning. The pass is **on by default in the `default`/`fast` profiles** (promoted
  2026-05-30, bead `SAT-playground-70h`): the fresh re-evaluation measured a net −12 PAR-2 over the
  profiling suite (n≥5, lucky-on entirely below lucky-off), and a shuffle test showed lucky robustly
  solves the order-fragile battleship instance (0.08 s vs lucky-off 18–904 s / timeouts). `baseline`
  keeps it off.

Still incomplete:

- asymmetric branching clause strengthening
- `use_rcheck` implied-clause checks
- MiniSat's CDCL implementation details; this solver still keeps the repo's `09` search core

So this is now a working BVE preprocessing iteration, but it is not yet a complete MiniSat `simp`
port.

## Solver 12 Configuration Contract

Solver 12 now parses all `SAT_*` runtime controls through one `SolverConfig`
object before reading the CNF. Search, propagation, simplification, and proof
code should not call `std::env::var` directly. The checked-in schema and
feature ledger are:

- `CONFIG_SCHEMA.csv`: environment variable, config field, type, defaults,
  feature/limit/legacy classification, conflicts, requirements, and task owner
- `FEATURES.csv`: machine-readable feature maturity, validation state, promoted
  profiles, validation artifact, risk IDs, and last task that changed the row
- `FEATURES.md`: human-readable notes for the same feature maturity records

Important config controls:

```bash
SAT_PROFILE=baseline|default|fast|experimental
SAT_SEARCH_AXIS=safe|validated|strong
SAT_PREPROCESS_AXIS=off|conservative|gate-aware
SAT_PROOF=off|drat|lrat
SAT_CONFIG_DUMP=on
SAT_CONFIG_OUT=log/solver11.config
SAT_CONFIG_REPLAY=log/solver11.config
SAT_STRICT_CONFIG=on
SAT_STATS_JSON=on
SAT_STATS_HOT=on
SAT_USE_LBD=on
SAT_RESTART=legacy-luby|kissat-ema|reluctant  # kissat-ema requires focused-stable search
SAT_RESTART_BLOCK_MARGIN=<f64>  # 0 disables the level blocker
SAT_EMA_SLOW_WINDOW=<u64>       # default 4096
SAT_RESTART_REUSE_TRAIL=on|off
SAT_RESTART_REUSE_TRAIL_FOCUSED=on|off
SAT_RESTART_REUSE_TRAIL_STABLE=on|off
SAT_REDUCE=legacy|lbd-tiered
SAT_REDUCE_MIN_INTERVAL=<usize>  # lbd-tiered default is 100, values must be >= 50
SAT_CLAUSE_MIN=off|basic|recursive-limited|inblock|inblock-late
SAT_INBLOCK_DELAY_CONFLICTS=<u64>  # default 1000000
SAT_INBLOCK_BINARY_MIN=<f64>       # default 0.85, valid range 0.0..=1.0
SAT_OTFS=on|off
SAT_MINIMIZE_DEPTH_LIMIT=<u32>  # default 1000
SAT_PHASE=legacy|saved|target-then-saved|best-then-target-then-saved  # target/best require focused-stable
SAT_FOCUSED_PHASE=auto|legacy|saved|target-then-saved|best-then-target-then-saved
SAT_STABLE_PHASE=auto|legacy|saved|target-then-saved|best-then-target-then-saved
SAT_SEARCH_MODE=single|focused-stable
SAT_MODE_USE_TICKS=on
SAT_VAR_DECAY_FOCUSED=<f64>  # focused-stable only, default 0.95
SAT_VAR_DECAY_STABLE=<f64>   # focused-stable only, default 0.95
SAT_LUCKY=on|off
SAT_CHRONO=on
SAT_CHRONO_MAX_DELTA=<usize>  # default 5000
SAT_VMTF=off|focused-only|single  # focused-stable defaults to focused-only unless explicit
SAT_REORDER=on|off
SAT_REORDER_INTERVAL_CONFLICTS=<u64>  # default 10000
SAT_REPHASE=on
SAT_BINARY_FAST=on
SAT_INPROCESS=on|off
SAT_INPROCESS_INTERVAL_CONFLICTS=<u64>  # default profile uses 1000000
SAT_INPROCESS_MAX_ROUNDS=<u64>          # 0 = unlimited
SAT_SWEEP=on|off                        # direct runtime override for SAT sweeping
SAT_VIVIFY=on|off
SAT_VIVIFY_LEARNED=on|off               # direct runtime override for learnt-clause vivify
SAT_VIVIFY_LEARNED_BINARY_GATE=on|off   # off restores pre-2026-07-11 learned-vivify behavior
SAT_VIVIFY_PERMILLE=<u64>               # default 20 when SAT_VIVIFY_TICKS=0
SAT_VIVIFY_TICKS=<u64>                  # absolute per-round vivify budget; 0 = proportional
SAT_VIVIFY_MAX_CLAUSE_LEN=<usize>       # 0 = default cap
SAT_ELIMINATE_TICKS=<u64>        # default 3000000000; BSR/BVE work budget for the eliminate pass; 0 = unlimited
SAT_ELIMINATE_RESOLUTIONS=<u64>  # default 100000000; BVE resolution-attempt budget; 0 = unlimited
SAT_ELIMINATE_OCCLIM=<u64>       # 0 means unlimited BVE occurrence scan
SAT_BSR_OCCLIM=<u64>             # default 0; 0 means unlimited BSR best-occurrence scan
```

The `default` and `fast` profiles currently keep `SAT_USE_LBD=off`,
`SAT_SEARCH_MODE=single`, and `SAT_MODE_USE_TICKS=off` while retaining the solver-10 preprocessing
stack. `SAT_LUCKY` is now **on by default in `default`/`fast`** (promoted 2026-05-30, bead 70h;
`baseline` keeps it off) after the fresh re-eval measured a net −12 PAR-2 win. The focused/stable search stack remains opt-in until it beats the clean solver 10
profiling baseline. `baseline` keeps both LBD/focused-stable search, lucky assignment, and
preprocessing off.

When `SAT_SEARCH_MODE=focused-stable` is enabled, `SAT_PHASE` acts as an input preference rather
than the literal phase policy used in every mode: focused mode maps `legacy` and `saved` to `saved`,
and maps `target-then-saved` and `best-then-target-then-saved` to `target-then-saved`; stable mode
always uses `best-then-target-then-saved`. Target/best phase snapshots are updated only in stable
mode. `SAT_FOCUSED_PHASE` and `SAT_STABLE_PHASE` override those per-mode effective policies for
focused/stable matrix tests; `auto` or an empty value keeps the default mapping. In single-mode
search, `SAT_PHASE=legacy|saved` is used directly and target/best policies are rejected at config
parse time so invalid benchmark settings fail fast instead of running to `UNKNOWN`; the per-mode
overrides are inert.

`SAT_CONFIG_OUT` writes a deterministic replay file with `schema_version`,
effective profile/axes, proof policy, every config field, feature maturity
records, legacy aliases used, and a stable `config_hash`. `SAT_CONFIG_REPLAY`
loads that file before CNF parsing. By default replay allows only the documented
runtime overrides (`SAT_CONFIG_OUT`, `SAT_RUN_LABEL`, `SAT_STATS_JSON`,
`SAT_STATS_HOT`, `SAT_TRACE_FULL`, `SAT_TRACE_PROOF`, `SAT_TRACE_PREPROCESS`,
`SAT_TRACE_PREPROCESS_DETAILS`, `SAT_TRACE_SEARCH_INTERVAL`,
`SAT_LIMIT_WALL_SEC`, `SAT_LIMIT_RSS_MB`); set
`SAT_CONFIG_REPLAY_ALLOW_OVERRIDES=on` only for explicit experiments.

Phase-selection telemetry distinguishes the default legacy saved-phase path
from explicit phase-policy fallbacks: `phase_legacy_used` counts
`SAT_PHASE=legacy`, `phase_saved_used` counts explicit saved-phase policy use,
and `phase_initial_used` counts true initial-phase fallbacks.

Unpromoted Phase 1 and Phase 2 feature flags default off. Flags whose implementation
bead has not landed are represented in the schema but fail fast if enabled, so
benchmark artifacts cannot accidentally record no-op feature claims. `lrat`
still fails fast. `SAT_LIMIT_*` values are parsed into the config contract and
solve-ending limit hits now return structured `UNKNOWN` results while deleting
any temporary proof artifact.

## Solver 12 Result Contract

Every normally exited solver 12 run writes the minimal 0.3a output contract into
the output directory passed as the second `run.sh` argument:

- `result.json`: mandatory machine-readable result contract
- `status.txt`: exact status string, one of `SAT`, `UNSAT`, `UNKNOWN`, or
  `PARSE_ERROR`
- `model.txt`: emitted for `SAT`
- `proof.out`: emitted for `UNSAT` when `SAT_PROOF=drat`

`result.json` includes:

```text
schema_version, status, exit_code, termination_reason, unknown_reason,
status_file, model_file, proof_file, proof_completeness,
model_check_result, proof_check_result, config_hash, input_sha256, profile,
proof_policy, output_contract_state, stats_json_seen
```

The solver keeps SAT Competition stdout compatibility: exactly one `s` line is
printed, and `v` lines are printed only for SAT. `PARSE_ERROR` is represented as
`s UNKNOWN` on stdout with `status=PARSE_ERROR` in `result.json`. `SAT`,
`UNSAT`, and `UNKNOWN` currently exit `0`; `PARSE_ERROR` exits `2`. When
`SAT_STATS_JSON=on`, the final `c JSON_STATS {...}` record is emitted on stderr
and `result.json` records `stats_json_seen=true`. The JSON stats line includes
config identity, input and binary hashes, result/status fields, timing buckets,
formula sizes, clause-database/GC budget counters, CDCL/preprocessing/watch/LBD/proof
counters, focused/stable mode timing, per-mode conflict/LBD/decision-level
diagnostics, focused restart interval diagnostics, output-contract state, and explicit zero/null placeholders for planned
Phase 1/2 counters that are not implemented yet. High-frequency watcher diagnostics (`watch_scans`,
`watch_blocker_hits`, `watch_clause_loads`, `watch_stale_skips`,
`binary_props`, and `long_props`) default to zero to keep the release hot path
solver-10-equivalent; set `SAT_STATS_HOT=on` for profiling runs that need those
counters. SAT model files are written on every SAT result; the expensive
internal reparse-and-check pass is only run with `SAT_CHECK_INVARIANTS=on`, so
normal runs record `model_check_result=not_checked` and rely on the smoke/bench
harness validation of emitted assignments. The JSON line records
`hot_diagnostics_enabled` so artifacts are explicit about this choice. Formula-classification
fields are emitted as `formula_size_class`, `formula_kissat_small`, `formula_kissat_bigbig`,
`formula_binary_fraction`, `formula_avg_clause_size`, and `formula_variable_density`.
`SAT_TRACE_FULL=on` emits an additional
human-readable `c trace_full ...` line with glue, restart, phase, inprocess,
learned-clause, GC, and branch-heap counters.
Lower-level diagnostics can be enabled independently with `SAT_TRACE_PROOF=on`,
`SAT_TRACE_PREPROCESS=on`, `SAT_TRACE_PREPROCESS_DETAILS=on`, and
`SAT_TRACE_SEARCH_INTERVAL=N` for periodic and final `c search ...` counters.

## Validation

> **Search-feature efficacy is being RE-EVALUATED fresh as of 2026-05-29** (bead `SAT-playground-gbc`).
> The prior validation tables were contaminated by measurement artifacts (host contention,
> cold-cache/first-run slowness, ~7.5% same-binary warming variance, single-run noise) and have
> been **archived** to `archive/efficacy-reeval-2026-05-29/README-validation-archived.md`.
> **Do not consult the archived tables unless explicitly asked** — they do not reflect current
> efficacy. Fresh results (warm-up + n≥2, same-binary toggles vs the chrono-off single-mode
> baseline, aggregate PAR-2 beyond noise, solver-10 floor) will be recorded here when complete.

### Fresh re-eval results (2026-05-29/30)

- **`SAT_LUCKY` → PROMOTED on by default (`default`/`fast`)** — 2026-05-30, bead `SAT-playground-70h`.
  n≥5 confirmation: lucky-on (721.6–726.0 PAR-2) is *entirely below* lucky-off (731.8–737.7), net
  ≈ −12 PAR-2 over the profiling suite; correctness clean (10/10, all proofs verified). Shuffle gate
  passed decisively: lucky-on solves the order-fragile battleship instance in 0.08 s on every shuffle
  seed while lucky-off ranges 18–904 s (would time out on 3/5). Solver-10 gate:
  `candidate_improves_previous_solver11_but_loses_solver10` (−12 vs prior default; still +23 vs
  solver-10, narrowing the gap). `baseline` keeps lucky off. Evidence:
  `log/lucky-confirm-2026-05-30/`, `log/efficacy-reeval-2026-05-29/FINDINGS.md`.
- All other tested search singles/combos from the original May sweep (chrono neutral; binary-fast /
  focused-stable / lbd-tiered / eager inblock / rephase / combos regress) were **not** promoted —
  see `FINDINGS.md`.

### Medium Clause-Minimization Promotion (2026-07-10)

`SAT_CLAUSE_MIN=inblock-late` is now the default. The promoted mode keeps the recursive minimizer
on every learned clause, then enables Kissat-style in-block shrink only after one million conflicts
and only on binary-dominated formulas (`SAT_INBLOCK_BINARY_MIN=0.85`). The guard was chosen after
the eager in-block mode solved useful gap cells but regressed SAT-sensitive rows on the full medium
suite.

Focused screen:
`log/abtest-latebin-vs-base-2026-07-10-00-58-15` improved the targeted 13-row set from 10/13 to
12/13. Full medium gate:
`log/abtest-latebin-vs-base-2026-07-10-01-28-43` improved the default from 58/100 to 60/100 solved,
with no lost solved cells and `promotion_gate=PASS`. The two gained cells were
`296fd43...oddball_24_5_ttf` and `f1973c...lockchart-group2`.

Smoke/correctness (independent of efficacy) is verified per change via
`bash tools/smoke_test.sh solver/12-kissat-inprocessing` (9/9) and `cargo test`.

### BSR occurrence-cap experiment (2026-06-08)

- **`SAT_BSR_OCCLIM=1000` → REJECTED as a default** — bead `SAT-playground-5b2.3.29`.
  A 5-seed profile20 seedgate showed fewer solved cells than the unlimited baseline:
  unlimited (`SAT_BSR_OCCLIM=0`) solved 65/100 with PAR-2 74926.668, while the capped candidate
  solved 61/100 with PAR-2 79865.034. The loss is primarily `REGRandom-K4-L1-Seed40`, where 5/5
  previously solved seeds become timeouts; the only solved-count gain is one seed on `bp4_CSO_IXA_ZR`.
  Keep the knob diagnostic/default-off unless a later parameter or adaptive rule clears the
  solved-count gate. Evidence:
  `log/seedgate-bsr_occlim_unlimited-2026-06-08-16-33-14/results.tsv`,
  `log/seedgate-bsr_occlim_1000-2026-06-08-20-44-54/results.tsv`, and
  `tools/check_promotion_gate.py --multiseed ...` gate output
  (`promotion_gate=FAIL`, candidate 61 solved vs previous 65).

### Auto-enable SAT_BINARY_FAST under SAT_FULL_BSR (analyzesat PRE-2, 2026-06-09)

- **Coupling `binary_fast_path` on whenever `full_bsr` is on → REJECTED as a default** — bead
  `SAT-playground-5b2.3.27`. Hypothesis (analyzesat PRE-2): BSR strengthens long clauses into
  binaries that, with the binary fast path off, fall onto the slow long-clause watcher list, so
  defaulting `SAT_BINARY_FAST=on` alongside BSR should recover Kakuro/velev. A 5-seed profile20
  seedgate (candidate `--env ""` with the coupling vs baseline `--env "SAT_BINARY_FAST=off"`,
  900s) **loses the lexicographic metric on the primary axis**: candidate solved **61/100**
  PAR-2 82149 vs baseline **65/100** PAR-2 74433 (baseline = the current default; its 65/100 ≈
  the BSR-occlim run's 65/100, cross-run consistent).
- **Why it loses:** the binary fast path is a propagation-order change (binaries propagate first,
  via `binary_implications`, before the long-clause watch scan in `propagate_impl`), which reshapes
  reasons → learned clauses → the whole search trajectory — faster per propagation but a *longer*
  search on formulas that are not binary-propagation-bound. The premise only half-held:
  **Kakuro improved** (−61% conflicts on commonly-solved seeds) but **velev got worse** (+155%
  conflicts, both still 5/5 at 900s), and the longer search tipped three borderline instances over
  the timeout: `sqrt-mitern170` (0/5 vs 2/5), `mp1-Nb7T46` (3/5 vs 5/5, +10657% conflicts on the
  seeds both solve), `REGRandom-K4-L1-Seed40` (4/5 vs 5/5). Only `bp4_CSO_IXA_ZR` gained (1/5 vs 0/5).
- **1800s confirmation (the reject is a conflicts loss, not a budget artifact):** to rule out the
  900s timeout being the cause, the 10 instances that timed out in the screen were re-run at a 30-min
  (1800s) timeout, N=5, both configs (`SAT_BINARY_FAST=on` reproduces the coupling on the reverted
  binary; `=off` is the baseline). With the extra budget the 900s solved-count gap **mostly closed** —
  `sqrt-mitern170` (0/5→5/5), `REGRandom` (4/5→5/5) and `case9` (3/5→5/5) were pure budget artifacts
  that recover in the candidate, and `bp4` genuinely *helps* (cand 2/5 vs base 0/5, both SAT seeds the
  baseline cannot crack even at 1800s). Combining the 1800s timeout-rows with the 900s always-solved
  rows for a full-20 read: candidate **solved 72/100 vs baseline 71/100** (cand +1, a single cell,
  within seed-noise and driven by seed-fragile `case9`), i.e. **solved-count effectively ties at a
  generous budget**. The decision therefore falls to the **conflicts tiebreak, where the candidate
  does +24.9% more search** (226.3M vs 181.1M conflicts over 63 commonly-solved cells). Per the repo
  metric — a faster-per-op change that does more search at equal solved-count is **not** a win — this
  is a clean reject on the genuine mechanism, not the 900s budget cliff. Per-instance conflicts split:
  helps Kakuro −61% / circuit −18% / 6s299b685 −13%; hurts mp1 **+10657%** / velev +155% / case9 +137%
  / sqrt-mitern171 +14% / brocard +11%; the big regressions (mp1/velev/case9) dominate the aggregate.
  `mp1-Nb7T46` is the one genuine solved-count loss (cand 3/5 vs base 5/5 even at 1800s).
- **Takeaway:** binary-fast-under-BSR is instance-shape-dependent (helps truly binary-bound formulas
  like Kakuro/bp4, hurts arithmetic/long-clause miters like velev/mitern/mp1), which argues for a
  *gated* coupling rather than an unconditional default — feed this into the BSR/binary-fast
  formula-gate beads (`SAT-playground-5b2.3.28` / `5b2.3.31`), not a blanket default flip.
  `SAT_BINARY_FAST=on` remains available as an explicit opt-in. The 5-seed screen + the 1800s
  timeout-instance rerun together (solved ties at generous budget, conflicts −25% for the baseline)
  were decisive enough to revert without an N=10 gate. Evidence:
  `log/seedgate-binfast_cand-2026-06-09-07-33-05/results.tsv`,
  `log/seedgate-binfast_base-2026-06-09-10-16-38/results.tsv`,
  `log/seedgate-binfast_to_on-2026-06-09-17-38-40/results.tsv`,
  `log/seedgate-binfast_to_off-2026-06-09-21-37-23/results.tsv`,
  screen log `log/binfast-5x5-screen.log`, rerun log `log/binfast-timeout-rerun.log`.

## 2026-05-15 Profile Simplification Optimization Pass

Target: close the solver-10 preprocessing gap to MiniSat `simp` on `benchmarks/profiling` while
keeping the search algorithm unchanged.

Accepted code-level changes:

- changed simplification occurrence refs from `usize` to `u32` and manually compact dirty
  occurrence lists
- moved propagation's blocker-satisfied fast path before deleted-clause/header checks
- for large formulas, store original-clause abstractions inline with original clauses during
  preprocessing, then strip the preprocessing-only words before search GC
- gate inline abstractions to formulas with at least `750000` original clauses so small/medium
  solved instances keep their previous search trajectory
- use a sorted subsumption relation for short clauses (`len >= 2`) only when clauses are known to be
  canonical-sorted and inline abstractions are active
- match MiniSat's removal scheduling more closely by smudging occurrence lists on deleted clauses
  without treating every removed-clause variable as a backward-subsumption touch
- read inline original-clause abstractions from one loaded header in the subsumption hot path
- reuse the preprocessing scratch buffer for strengthened clauses, compute the strengthened
  abstraction during that same scan, and compact the arena clause in one pass
- replace per-strengthen occurrence-list `position()` removal with lazy membership cleaning on the
  large inline path only
- use lazy preprocessing watcher detach on small formulas and the large inline path; keep strict
  detach for mid-sized formulas where lazy detach perturbed the search path

Key focused K4 movement:

| Build | Preprocess | Total | Notes |
|---|---:|---:|---|
| previous accepted | `103.796s` | `159.31s` | `log/diagnostics/current-best-k4-2026-05-15-21-25/k4.stderr` |
| inline abstractions only | `97.181s` | `158.96s` | `log/diagnostics/simp-inline-strip-2026-05-15-19-42-00/k4.stderr` |
| gated inline + sorted relation | `77.708s` | `132.38s` | `log/diagnostics/simp-inline-gated-sorted9-2026-05-15-20-03-00/k4.stderr` |
| one-pass strengthen updates | `67.996s` | `122.68s` | `log/diagnostics/simp-strength-onepass-compact-2026-05-15-20-50-00/k4.stderr` |
| lazy occurrence membership cleanup | `53.524s` | `108.40s` | `log/diagnostics/simp-lazy-occ-membership-2026-05-15-20-56-00/k4.stderr` |

The final profile bench solves K4 in `95.300s` including proof verification. A follow-up MiniSat
parity pass found that solver 10 was incorrectly using clause deletion as a backward-subsumption
touch source; after matching MiniSat's removal scheduling, focused Kakuro preprocessing improved
from `99.419s` to `71.201s` (`SAT_TRACE_PREPROCESS=1`, `/tmp/kakuro-112.cnf`, 2026-05-15).
MiniSat's verbose run on the same decompressed input reports `28.09s` simplification, so the
remaining gap is now about `2.5x` on preprocessing rather than `3.4x`.

Full `benchmarks/profiling` result:

```bash
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/10-bve-subsume
```

| Solver/run | Solved | SAT | UNSAT | Timeouts | PAR-2 | Results |
|---|---:|---:|---:|---:|---:|---|
| solver 10 previous accepted | 7/11 | 5 | 2 | 4 | `1087.869` | `log/bench-10-bve-preprocess-2026-05-15-18-39-10/results.csv` |
| solver 10 after this pass | 9/11 | 6 | 3 | 2 | `679.222` | `log/bench-10-bve-preprocess-2026-05-15-21-32-49/results.csv` |
| MiniSat `simp` | 10/11 | 6 | 4 | 1 | `559.646` | `log/bench-minisat-2026-05-14-13-31-31/results.csv` |

The accepted pass flips `REGRandom-K4` from timeout to verified UNSAT (`95.300s`) and `random_v355`
from timeout to SAT (`21.270s`) while preserving `mp1`. Remaining non-search gap is still visible on
Kakuro (`71s` focused preprocessing versus MiniSat's `28.09s` simplification).

Rejected experiments from this pass:

- all-formula inline abstractions: K4 preprocessing improved, but Feistel search roughly doubled
  because the migration perturbed small-instance search paths
- direct inline abstraction loads: K4 preprocessing regressed (`79.259s` versus `77.708s`)
- sorted-relation threshold `5`: tied threshold `9` on K4 and was not worth extra risk
- cleaning every driver occurrence list on large formulas: exceeded the prior Kakuro preprocessing
  time before producing a trace line
- ungated lazy occurrence-membership cleanup: solved K4, but regressed Timetable; gating it to the
  large inline path kept K4 and restored Timetable
- globally lazy preprocessing watcher detach: improved K4/Kakuro and made `random_v355` solve, but
  moved `mp1` to timeout; the accepted policy applies it only to small formulas and the large inline
  path
- side-table abstractions on Kakuro: regressed focused preprocessing to `121.529s`; inline
  abstractions remain faster despite the header arithmetic
- specialized split subsumption hot loop: regressed focused Kakuro preprocessing to `112.626s`
- MiniSat-style contiguous `Vec` subsumption queue: regressed focused Kakuro preprocessing to
  `80.179s`
- MiniSat-style gather-time queue marking: regressed focused Kakuro preprocessing to `72.853s`

## MiniSat-Simp Five-Instance Benchmark

Command:

```bash
bash tools/bench.sh -t 600 -m 16384 -d benchmarks/profiling/minisat-simp-five solver/10-bve-subsume
```

Result log:

- `log/bench-10-bve-preprocess-2026-05-08-15-56-37/results.csv`

Summary:

- 5 instances
- 5 solved: 3 SAT, 2 UNSAT
- 0 timeouts
- PAR-2: `540.550`

Comparison against matching harness runs:

| Solver | Solved | SAT | UNSAT | Timeouts | PAR-2 | Results |
|---|---:|---:|---:|---:|---:|---|
| `09-root-simp-opts` | 3/5 | 2 | 1 | 2 | `3195.921` | `log/bench-09-root-simp-opts-2026-05-08-09-58-03/results.csv` |
| `10-bve-subsume` before gated BSR | 4/5 | 3 | 1 | 1 | `1532.975` | `log/bench-10-bve-preprocess-2026-05-08-13-08-41/results.csv` |
| `10-bve-subsume` gated BSR | 5/5 | 3 | 2 | 0 | `540.550` | `log/bench-10-bve-preprocess-2026-05-08-15-56-37/results.csv` |
| `minisat` | 5/5 | 3 | 2 | 0 | `453.343` | `log/bench-minisat-2026-05-08-09-58-03/results.csv` |

Per-instance notes for the gated-BSR run:

- `sudoku-N30-12`: `184.240s`, roughly equal to previous solver `10` and much faster than `09`
- `SC25_Timetable...`: `89.200s`, still far slower than MiniSat's `18.545s`
- `REGRandom-K4...`: now solves UNSAT in `205.600s`; previous solver `10` timed out at `600s`
- `mp1-Nb7T46`: `43.110s`, still faster than MiniSat's `75.054s`
- `Kakuro...`: `18.400s`, still faster than MiniSat's `80.111s`

The remaining gap to MiniSat is now mostly preprocessing speed on K4 and CDCL/search behavior on the
Timetable SAT instance. A direct MiniSat K4 run reported `39.65s` simplification and `61.26s` total
CPU time; gated solver `10` reaches the same K4 residual formula but spent about `117.05s` in
preprocessing during trace runs.

## Fresh MiniSat-Gap Debugging Notes

The 2026-05-08 fresh five-instance rerun showed solver `10` solving 3/5 while MiniSat solved all 5.
The accepted change from the follow-up debugging loop is a larger-formula full-BSR gate:

- `9af7...brocard_problem_large`: baseline solver `10` solved UNSAT in `163.160s`; with full BSR
  enabled by the new large-formula gate it solved in about `42.3s` (`34.9s` preprocessing +
  `7.4s` search).
- MiniSat's dumped residual for brocard had `4,086,123` clauses and `13,124,041` literals. The new
  large-formula BSR path produces essentially the same residual before search.
- Running solver `10` directly on MiniSat's brocard residual solved in `9.7s`, confirming the
  brocard gap was mostly preprocessing residual quality rather than CDCL search.

Rejected or incomplete hypotheses from that loop:

- Initial negative branching phase did not solve `bp4`, brocard, or Timetable within the tested
  bounds.
- MiniSat-style variable-order activity tie-breaking plus negative phase was worse on brocard and
  Timetable than the existing occurrence tie.
- MiniSat-style backtrack-only phase saving plus negative phase did not fix the SAT-side gaps.
- Forced full BSR matches MiniSat-like residuals on `bp4` and Timetable, but it does not make solver
  `10` solve those SAT targets quickly; running solver `10` on MiniSat's own residual formulas still
  timed out under the tested `90s` bound. Those remain CDCL/search-core gaps.

## MiniSat-Loop Refactor Follow-up

The next refactor implemented the remaining MiniSat `simp` work-loop differences:

- full BSR became force-runnable instead of using the earlier formula-size gate; current default is
  off, with `SAT_FULL_BSR=on` retained for targeted comparison runs
- preprocessing now loops over touched variables, root assignments, queued subsumption clauses, and
  elimination-heap variables until all work is drained
- BSR strengthens original clauses in place
- variable occurrence-cost updates feed a dynamic elimination heap broadly after clause
  add/delete/strengthen events
- generated resolvents are queued immediately for subsumption, and touched variables continuously
  enqueue their occurrence clauses

Direct `600s` checks on the fresh MiniSat-gap instances after this refactor:

| Instance | Result | Notes |
|---|---:|---|
| `849950...circuit_48in64out...` | SAT `208.1s` | Slower than the previous gated path (`49.4s`). |
| `98e8...bp4_TCO_CSO_IXA_LP_ZR` | TIMEOUT | Preprocessing `7.0s`; search still did not find SAT. |
| `9af7...brocard_problem_large` | UNSAT `~15.3s` | Improved from `42.3s` after the large-formula gate and `163.2s` before it. |
| `f17d...SC25_Timetable...` | TIMEOUT | Preprocessing `5.3s`; search still did not find SAT. |
| `f25a...1-TC-256-K-63` | TIMEOUT | With `SAT_FULL_BSR=off`, the same code still solves in `375.4s`; full MiniSat-like preprocessing changes the search trajectory. |

MiniSat enters search on `1-TC` with the same residual size (`422669` clauses / `930421` literals)
and solves in `162.8s`, so the remaining `1-TC`, `bp4`, and Timetable gap is no longer explained
by these `simp` work-loop differences alone.

Matching harness rerun:

- `10-bve-subsume`: `log/bench-10-bve-preprocess-2026-05-08-22-51-56/results.csv`
- solved `2/5` (`1 SAT`, `1 UNSAT`, `3` timeouts)
- PAR-2: `3823.879`

Compared with the previous gated-BSR run on this same set, the refactor improves Brocard
dramatically (`163.160s -> 16.277s`) but regresses the overall benchmark (`3/5`, PAR-2
`2986.963` -> `2/5`, PAR-2 `3823.879`) because circuit slows down and `1-TC` becomes a timeout.

## Parse-Time Canonical Insertion Follow-up

On 2026-05-09, initial parsed clauses were routed through the same MiniSat-style original-clause
normalization path used by preprocessing-generated resolvents. Validation:

- `cargo test` in `solver/10-bve-subsume`: 48 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-09-07-33-09`

Benchmark rerun:

- `log/bench-10-bve-preprocess-2026-05-09-00-21-53/results.csv`
- 5/5 solved, PAR-2 `946.556`

Diff versus the previous accepted `minisat-simp-five` run
(`log/bench-10-bve-preprocess-2026-05-08-15-56-37/results.csv`):

| Instance | Before | After | Delta |
|---|---:|---:|---:|
| `sudoku-N30-12` | `184.240s` | `357.536s` | `+173.296s` |
| `SC25_Timetable...392...` | `89.198s` | `29.561s` | `-59.637s` |
| `REGRandom-K4...` | `205.602s` | `201.044s` | `-4.558s` |
| `mp1-Nb7T46` | `43.106s` | `45.757s` | `+2.651s` |
| `Kakuro...` | `18.404s` | `312.658s` | `+294.254s` |

Follow-up Kakuro isolation runs:

| Mode | Full BSR | Time | Results |
|---|---:|---:|---|
| `canonical-sorted` | on | `312.658s` | `log/bench-10-bve-preprocess-2026-05-09-00-21-53/results.csv` |
| `raw` | on | `454.667s` | `log/bench-10-bve-preprocess-2026-05-09-07-20-27/results.csv` |
| `canonical-sorted` | off | `95.817s` | `log/bench-10-bve-preprocess-2026-05-09-07-28-51/results.csv` |
| `raw` | off | `19.140s` | `log/bench-10-bve-preprocess-2026-05-09-07-31-04/results.csv` |
| `input-order` | off | `19.508s` | `log/bench-10-bve-preprocess-2026-05-09-07-32-00/results.csv` |

Conclusion: parse-time canonical insertion closes a real MiniSat `addClause_()` semantic gap and
keeps correctness intact, but it should not be considered a default performance win yet. The Kakuro
regression is a compound search-path sensitivity: full BSR/work-loop policy is the largest factor,
and sorted canonical literal order adds another large slowdown. Canonical semantics that preserve
input literal order recover the old fast behavior when full BSR is disabled, so duplicate removal,
tautology skipping, and immediate root units are not the observed Kakuro problem by themselves.

### Rejected guarded initial clause order auto mode (2026-05-26)

`SAT_INITIAL_CLAUSE_MODE=auto` was briefly promoted for the default and fast profiles, using only
input-shape data available before initial clause insertion. The policy selected `input-order` for
Kakuro-like formulas:

- at least `10,000,000` input clauses
- input binary-clause fraction at most `0.05`
- input average clause length between `3.0` and `4.0`
- input literal/variable density at least `300`

That gate was reverted because the apparent PAR-2 gain was dominated by one Kakuro row and depended
on preserving DIMACS literal order. This is an overfit trajectory workaround rather than a
mechanism-level solver improvement. Default, fast, baseline, and `auto` now resolve to
`canonical-sorted` until the underlying mechanism is addressed, likely by decoupling initial watch
selection from physical literal sorting. Explicit `SAT_INITIAL_CLAUSE_MODE=input-order`,
`kissat-watch`, and `raw` remain available as diagnostics. The `kissat-watch` mode mirrors the
Kissat import step by selecting the first two watched literals from the normalized input-order
clause while keeping the remaining literals canonicalized; because this breaks globally sorted
physical clause order, the sorted-subsumption fast path is disabled for that diagnostic mode.

Follow-up diagnostic comparison after adding the explicit `kissat-watch` mode:

| Initial clause mode | Results | PAR-2 | Solved | Notes |
|---|---:|---:|---:|---|
| `canonical-sorted` | `log/bench-11-kissat-port-2026-05-26-18-09-15/results.csv` | `746.711` | `10/10` | Default-safe baseline after reverting the overfit auto gate |
| `input-order` | `log/bench-11-kissat-port-2026-05-26-18-25-42/results.csv` | `644.540` | `10/10` | Big Kakuro win (`48.985s`) but Velev and REGRandom regress, so still diagnostic-only |
| `kissat-watch` | `log/bench-11-kissat-port-2026-05-26-18-40-32/results.csv` | `822.083` | `10/10` | Status-safe but slower than sorted; not a replacement for canonical sorting |

Conclusion: selecting Kissat-style initial watches alone does not explain the input-order Kakuro
trajectory win. The diagnostic path is useful evidence, but promotion needs a stronger mechanism
than moving the first two watched literals while disabling sorted-subsumption.

Shuffle-sensitivity validation for the overfit concern:

```bash
python3 tools/shuffle_sensitivity.py \
  --instances \
  benchmarks/profiling/5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7.cnf.xz \
  benchmarks/profiling/6832fe907740af686fde98518067ea3f-velev-pipe-sat-1.0-b7.cnf.xz \
  --seeds 1 \
  --modes canonical-sorted,input-order,kissat-watch \
  --timeout 300 \
  --memory-mb 16384 \
  --work-dir log/phase1/dq9-shuffle-kakuro-velev-seed1 \
  --force
```

| Mode | Shuffled Kakuro seed 1 | Shuffled Velev seed 1 | Summary |
|---|---:|---:|---|
| `canonical-sorted` | `TIMEOUT` | `SAT 131.729s` | `log/phase1/dq9-shuffle-kakuro-velev-seed1/summary.csv` |
| `input-order` | `SAT 295.490s` | `SAT 209.618s` | Same summary |
| `kissat-watch` | `TIMEOUT` | `SAT 204.459s` | Same summary |

The unshuffled `input-order` Kakuro win (`48.985s`) does not survive a single deterministic
literal/clause shuffle; the same mode needs `295.490s` on the shuffled Kakuro row and regresses
shuffled Velev. Treat future input-order wins as trajectory-sensitive until they pass
multi-seed shuffled validation.

Rejected alternatives:

- Blindly defaulting to `raw` or `input-order`: AnalyzeSAT evidence showed those modes were
  status-safe on the profiling suite, but they regressed Velev (`canonical-sorted 77.545s`,
  `input-order 109.516s`, `raw 122.182s`), so they remain diagnostics until a mechanism-level fix
  explains the trajectory movement.
- A broader low-binary or high-density rule: Brocard and REGRandom share some individual features
  with Kakuro, but not the full high-clause/high-density shape. They stay canonical until a broader
  family has direct evidence.
- Promoting a Sudoku-shaped rule: the referenced evidence directory for a Sudoku-specific win was
  absent from this checkout, and the 2026-05-26 guard validation kept Sudoku canonical.

Historical validation for the rejected gate:

| Run | Results | PAR-2 | Solved | Notes |
|---|---:|---:|---:|---|
| Prior solver 12 default | `log/bench-11-kissat-port-2026-05-26-14-31-59/results.csv` | `865.611` | `10/10` | fresh `/nextbeads` baseline |
| Guarded auto candidate | `log/bench-11-kissat-port-2026-05-26-14-59-03/results.csv` | `666.213` | `10/10` | no UNKNOWN/error/status regressions |
| Solver 10 comparison | `log/phase1/solver10-default-300-vs-solver11-clean/results.csv` | `699.671` | `10/10` | same 300s profiling set |

Dominant per-instance delta was Kakuro `255.845s -> 51.891s` (`-203.954s`). Velev stayed
status-safe and canonical-shaped (`81.823s -> 85.904s` in this noisy paired run). The required
solver 12 promotion gate passed with candidate PAR-2 `33.458s` better than solver 10 and `199.398s`
better than the prior solver 12 baseline.

## MiniSat CDCL Compatibility Follow-up

On 2026-05-09, the CDCL core was moved closer to MiniSat in five targeted areas:

- learned-clause budget adjustment now starts at 100 conflicts and is reset after preprocessing
  from the residual original-clause count unless `SAT_REDUCE_DB_INIT`,
  `SAT_REDUCE_DB_INTERVAL`, or `SAT_POST_PREPROCESS_REDUCE_DB_RESET` override it
- conflict analysis defaults to MiniSat's `seen`-only behavior and skips literal position 0 in
  reason clauses; the older solver-10 `scratch_resolved` mode was retired after profiling showed
  identical CDCL trajectories and only extra hot-loop/config surface
- variable and learned-clause activities now use `f64`; learned-clause activity uses two arena
  words
- proof generation remains enabled by default, with `SAT_PROOF=off` available only as a diagnostic
  mode
- branch defaults are MiniSat-like: variable-order tie-breaking and negative initial polarity;
  the previous occurrence-count ordering is available with `SAT_BRANCH_MODE=occurrence`

Validation:

- `cargo test` in `solver/10-bve-subsume`: 48 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-09-10-42-44`

Benchmark command:

```bash
bash tools/bench.sh -t 600 -m 16384 -d benchmarks/profiling/minisat-simp-five solver/10-bve-subsume
```

Before/after logs:

- before: `log/bench-10-bve-preprocess-2026-05-09-10-19-52/results.csv`
- after: `log/bench-10-bve-preprocess-2026-05-09-10-43-00/results.csv`

| Instance | Before | After | Delta |
|---|---:|---:|---:|
| `sudoku-N30-12` | `340.515s` | `353.204s` | `+12.689s` |
| `SC25_Timetable...392...` | `53.842s` | `32.634s` | `-21.208s` |
| `REGRandom-K4...` | `204.339s` | `226.317s` | `+21.978s` |
| `mp1-Nb7T46` | `44.788s` | `46.763s` | `+1.975s` |
| `Kakuro...112...` | `303.744s` | `288.306s` | `-15.438s` |
| **PAR-2** | **`947.228`** | **`947.224`** | **`-0.004`** |

Timetable trace stats, with default proof generation:

| Metric | Before | After |
|---|---:|---:|
| Preprocess time | `4.656s` | `4.665s` |
| Eliminated vars | `106138` | `106138` |
| Resolvents | `334136` | `334136` |
| Subsumed clauses | `57400` | `57400` |
| Strengthened clauses | `126577` | `126577` |
| Search time | `49.085s` | `27.818s` |
| Conflicts | `412742` | `292899` |
| Decisions | `8314512` | `5734322` |
| Propagations | `140891805` | `92303633` |
| Restarts | `1017` | `700` |

The Timetable improvement is therefore search-path driven: preprocessing produced identical counts,
but the MiniSat-like CDCL defaults reduced conflicts by about 29% and decisions by about 31%.
The aggregate five-instance score is effectively flat because the same search-path changes regress
the two UNSAT instances and slightly regress `mp1`.

Proof-off diagnostic on Timetable:

- command added `SAT_PROOF=off` with the same trace settings
- elapsed time changed from `32.731s` to `32.096s`
- search time changed from `27.818s` to `27.243s`
- conflicts/decisions/propagations were unchanged
- no `proof.out` or `proof.out.tmp` was written

Conclusion: proof streaming has measurable but small SAT-side overhead on this target. The larger
effect is the CDCL search trajectory change from MiniSat-compatible analysis/branching defaults.

## Lazy Deleted-Clause Watcher Follow-up

On 2026-05-09, five smaller CDCL/code-level changes from the MiniSat comparison were tested one at
a time against the current solver-10 baseline. The three-instance diagnostic set was Timetable,
K4, and `mp1`, using `SAT_TRACE_PREPROCESS=1`, a very high search trace interval, and a `600s`
per-instance cap.

Diagnostic logs:

- individual-change matrix: `log/diagnostics/individual-2026-05-09/summary.tsv`
- lazy-detach Sudoku/Kakuro validation: `log/diagnostics/individual-2026-05-09/candidate_remaining.tsv`

Three-instance totals:

| Change | Elapsed delta | Search delta | Outcome |
|---|---:|---:|---|
| Trim root-false literals from learned clauses | `-1.326s` | `-0.110s` | Same search counters; noise. |
| Store learned-clause activity as `f32` | `-1.070s` | `+0.132s` | Same search counters; noise. |
| Lazy detach deleted watchers | `-15.821s` | `-15.015s` | Only clear isolated win. |
| MiniSat positive-before-negative literal tie sort | `+8.058s` | `+0.024s` | Worse preprocessing on K4. |
| Attach learned clause after backtrack | `-1.330s` | `+0.304s` | Same search counters; noise. |

The kept change is lazy deleted-clause watcher cleanup:

- ordinary `detach_clause()` is now lazy; deleted or stale watchers are skipped and compacted out
  when the relevant watch list is scanned during propagation
- `detach_clause_strict()` remains available for places that still need eager unlinking
- preprocessing original-clause removal uses strict detach before marking the clause deleted
- propagation tolerates watcher entries whose clause was deleted or whose watched literal moved
  during in-place strengthening

Full five-instance trace validation for the kept change:

| Instance | Baseline elapsed | Lazy detach elapsed | Search delta | Counter movement |
|---|---:|---:|---:|---|
| `sudoku-N30-12` | `359.334s` | `317.959s` | `-41.524s` | conflicts `-16.4%`, decisions `-18.6%`, propagations `-7.7%` |
| `SC25_Timetable...392...` | `32.762s` | `21.599s` | `-11.080s` | conflicts `-28.8%`, decisions `-23.6%`, propagations `-24.5%` |
| `REGRandom-K4...` | `227.351s` | `225.398s` | `-1.244s` | conflicts `+5.9%`, decisions `+5.6%`, propagations `+3.7%` |
| `mp1-Nb7T46` | `46.989s` | `44.284s` | `-2.691s` | same conflicts/decisions/propagations; faster throughput |
| `Kakuro...112...` | `289.194s` | `352.258s` | `+61.406s` | conflicts `+66.0%`, decisions `+49.2%`, propagations `+64.0%` |

Aggregate trace totals:

- elapsed: `955.630s -> 961.498s` (`+5.868s`, `+0.6%`)
- search: `477.397s -> 482.264s` (`+4.867s`, `+1.0%`)

Conclusion: lazy detach is a useful implementation simplification and a real win on Sudoku,
Timetable, and `mp1`, but it is still a search-path tradeoff rather than an aggregate
five-instance performance win. The large `mp1` regression seen in the combined experimental patch
did not reproduce for any single change, so it was an interaction effect and the other four changes
were not kept.

## Full-BSR Code-Level Optimization Pass

On 2026-05-15, a focused code-level pass targeted the full-BSR preprocessing gap on the current
`benchmarks/profiling` set. The main target was
`46355...REGRandom-K4-L1-Seed40`, where profiling showed almost all proof-off preprocessing time in
`backward_subsumption_check`, especially `subsumption_relation`.

Accepted changes:

- mark each driver clause once per BSR driver instead of rebuilding candidate-side marks for every
  candidate relation check
- pass the already-known driver length into `subsumption_relation`
- scan candidate clauses through `clause_slice()` after length and abstraction filters pass

K4 proof-off preprocessing trace, with identical residual stats
(`eliminated=512`, `resolvents=8192`, `strengthened=348160`):

| Step | Time | Log |
|---|---:|---|
| Baseline | `116.316s` | `log/opt-10-simp-baseline-2026-05-14-22-26-45` |
| Prepared driver marks | `104.752s` | `log/opt-10-simp-prepared-driver-2026-05-14-22-48-48` |
| Candidate slice scan | `94.317s` | `log/opt-10-simp-clause-slice-2026-05-14-22-54-51` |
| Delayed slice creation | `91.222s` | `log/opt-10-simp-delayed-slice-2026-05-14-23-02-40` |

The final proof-on K4 trace still needs about `95.060s` preprocessing plus `64.068s` search
(`log/opt-10-simp-k4-proof-on-current-2026-05-15-00-16-06`), so K4 remains outside a `120s`
profile-bench cap even after the preprocessing speedup.

Final profile benchmark:

```bash
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/10-bve-subsume
```

- `10-bve-subsume`: 7/11 solved, PAR-2 `1093.858`
- results: `log/bench-10-bve-preprocess-2026-05-15-00-25-35/results.csv`
- comparison MiniSat run: 10/11 solved, PAR-2 `559.646`
  (`log/bench-minisat-2026-05-14-13-31-31/results.csv`)

Rejected code-level attempts from this pass:

- sorted original clauses plus two-pointer relation: K4 regressed to `116.874s`
- candidate metadata hoisting: K4 regressed to `99.654s`
- removing duplicate live-candidate checks: K4 regressed to `95.931s`
- direct abstraction indexing: K4 regressed to `98.516s`
- unchecked candidate scanning: K4 regressed to `100.302s`
- early relation exit: K4 improved only to `90.752s`, below the 3% keep threshold
- occurrence-count best-variable cleanup: Kakuro improved from `166.964s` to `135.785s`, but K4
  regressed to `102.500s`
- manual occurrence-list compaction: K4 regressed to `95.788s`; Kakuro regressed to `170.497s`
- gated/const-generic occurrence cleanup: still regressed or timed out on K4
- direct driver-slice marking: K4 regressed to `93.521s`
- branchless relation-sign encoding: K4 regressed to `98.359s`
- custom literal-variable helper: K4 regressed to `99.772s`
- driver-clause sentinel instead of enum equality: K4 improved only to `90.455s`, below the 3%
  keep threshold

Validation for the accepted changes:

- `cargo test` in `solver/10-bve-subsume`: 48 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-15-00-25-25`

## Kakuro MiniSat-Simp Parity Pass

On 2026-05-15, a focused MiniSat comparison pass targeted
`5e933a...Kakuro-easy-112-ext.xml.hg_7.cnf.xz`, where MiniSat full simplification finishes in
about `28.09s` and solver 10 was still spending `71.201s` in preprocessing.

Accepted change:

- Store inline original-clause abstractions as one 32-bit word, matching MiniSat's non-learnt
  clause `abst` field, while keeping learned clauses on two activity words. This removes one
  arena word per live original clause and turns the abstraction filter into the same 32-bit
  variable mask shape MiniSat uses.

Kakuro focused trace:

| Step | Preprocess Time | Notes |
|---|---:|---|
| Previous solver 10 baseline | `71.201s` | pushed baseline from `130ed5c` |
| 32-bit inline original abstraction word | `67.529s` | `eliminated=56052`, `subsumed=4868640`, same residual formula stats |
| MiniSat simp reference | `28.09s` | verbose MiniSat simplification time on the same decompressed input |

The accepted change closes about `5.2%` of the Kakuro preprocessing runtime versus the previous
solver 10 baseline, but the remaining gap is still about `2.4x` versus MiniSat on this instance.

Fresh profile after the accepted change:

- `log/diagnostics/kakuro-abs32-symbols-perf/perf.data`
- `backward_subsumption_check`: about `79%` of sampled cycles
- `original_clause_abstraction` under `subsumption_relation`: about `15%`
- `clean_occurs`: about `7%`

Rejected fresh-pass attempts:

- unmarked initial BSR seeding: `71.806s`, worse than the previous baseline
- delayed heap-update parity: `71.905s`, worse despite MiniSat-like heap stats
- direct short-clause sorted relation: `71.224s`, effectively tied and below the keep threshold
- candidate length/abstraction hoisting: `72.303s`, branch/register pressure regressed
- no-clone BSR seed loop: `71.423s`, below the keep threshold
- end-to-end `u32` abstraction plumbing: `68.645s`, worse than the accepted `67.529s`
- preallocating the BSR queue: `66.846s`, only about `1.0%` faster than the accepted baseline,
  below the 3% keep threshold
- direct inline abstraction loads in the candidate loop: `68.004s`, worse than the helper path
- raw-pointer occurrence scanning: did not finish preprocessing within the `110s` cap

Validation after the accepted change:

- `cargo test` in `solver/10-bve-subsume`: 48 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-15-23-40-34`

## 2026-06-17 Propagation Watch-Prefetch Pass (bead 5b2.8.1, `SAT_PREFETCH`)

`perf record` on `case9` showed `Solver::propagate` is **~83% of self-time**, bottlenecked on
random `arena[clause_idx]` loads: the watch-list slots are sequential (hardware-prefetched) but
each watcher's clause sits at an unpredictable arena offset, so every clause inspection is a likely
last-level-cache miss that stalls the core.

**Accepted change:** software-prefetch the *next* watcher's clause (`_mm_prefetch`, `_MM_HINT_T0`)
while the loop processes the current watcher — classic software pipelining of a pointer-chasing
loop. Gated behind `SAT_PREFETCH` (config `prefetch_watched_clauses`), promoted to **default + fast**
(off in raw/baseline). The prefetch is a pure microarchitectural hint: it never reads a value or
touches solver state, so it is **conflict-preserving** — it cannot change which clauses propagate,
which conflicts arise, the learned clauses, or the DRAT proof.

Evidence:

- **Conflict-preservation, proven at scale:** the 5×5/900s seedgate's 75 shared-solved cells have
  **byte-identical conflicts** (98,219,572; 0 mismatches), and every single-instance A/B is
  conflict-identical.
- **Single-instance (quiet cores — the competition scenario):** faster-or-neutral on **all 20**
  profile20 instances, conflict-capped at 200k/50k conflicts: `SCPC −20.9%`, `case9 −13.8%`,
  `Kakuro −10.0%`, `oddball −9.0%`, `circuit −7.2%`, `sqrt171 −4.4%`, `BubblePancake −4.2%`,
  `bp4 −3.7%`, `sqrt170 −3.4%`, `sudoku −3.3%`, `brocard −2.7%`, `Pancake −2.6%`, `div −2.6%`,
  `mp1 −2.0%`, `6s299 −1.5%`, `tseitin −1.1%`, `velev −0.6%`, `REGRandom −0.1%`,
  `VexRiscv +0.3%` (noise, 0.14s/43s), `battleship` (too fast to measure).
- **5×5/900s seedgate (parallel):** solved `75 = 75` (tied), conflicts identical on all shared-solved
  cells, PAR-2 `−0.14%` (`61782.8 → 61697.7`). The tiny aggregate is a **5-parallel memory-bandwidth
  artifact** — with 5 cells running concurrently the extra prefetch traffic contends for bandwidth,
  even inverting the gain *under parallel load* (`sqrt170 +8%`, `bp4 +5%`, `6s299 +7%` in parallel,
  yet all faster single-instance). The competition/bench runs single-instance, where the benefit is
  real; the doc itself notes PAR-2 is contention-sensitive and conflicts are the contention-immune
  metric (here exactly tied).

Validation after the accepted change:

- `cargo test`: 459 passed (incl. `test_prefetch_watched_clauses_is_parsed_and_replayable`)
- smoke suite: 9/9 passed, including DRAT verification with `SAT_PREFETCH` on (`s VERIFIED`)
- baseline run: `log/seedgate-pref-baseline-2026-06-17-17-25-59/`
- candidate run: `log/seedgate-pref-on-2026-06-17-19-42-46/`
- profile: `propagate` 83% self-time (perf on `case9`, conflict-capped)

## 2026-07-28 SAT-Sweeping Productivity Pass (SAT_SWEEP_ROOT, SAT_SWEEP_SUBST — both default-off)

Session goal: attack the largest measured mechanism gap vs kissat — sweep productivity
(kissat kitten-solves 90k-18M times per instance with up to 3.5M substitutions on the
starved timeout cells; solver12 posted 0 sweep finds there — `plan/gap-read-2026-07-21.md`).

**Defect found (permanent fact):** `sweep_round` added its proven equivalences as LEARNED
binaries (`inprocess_add_clause` → `add_clause_from_slice`), but `try_els` harvests its
implication graph from `original_clause_ids` only — **sweep-proven equivalences were never
substituted, on any cell, ever**. Measured on booth_dadda_mapped: 352 equivalences found
across rounds, 4 ELS calls, 0 substitutions. The finds are also duplicate-inflated by
overlapping environments (352 finds = 11 distinct pairs).

**`SAT_SWEEP_SUBST` (default-off):** installs sweep equivalence binaries as ORIGINAL
clauses (deduped, skip-assigned) so the ELS merge actually fires;
`SAT_SWEEP_SUBST_MIN_EQUIVS` keeps low-yield rounds (TT class finds ~5) on the shipped
learned shape byte-identically. Result: **kissat-parity substitution mass on the BMC
class** — oski15a01b20s 69,347 substituted vars (kissat 71,487), VexRiscv 76,062 — and a
large wall win (oski15b20 1642 s idle baseline → 1281 s under load), but the decisive
conflicts tier LOSES: vex 2,975,066 → 3,412,420 (+437k), oski15b20 2,663,684 → 2,832,881
(+169k). Booth x3 / Bubble / stp212 / g2 timeout cells: zero flips. Verdict: the
REDUCE-law shape — real mechanism, negative at the 1800 s metric; >3000 s-horizon item.

**`SAT_SWEEP_ROOT` (default-off):** kissat-parity escalating root sweep — whole-environment
completion flags (without them 91/102 booth probe finds were re-proofs), cross-pass dedup,
kitten-tick budget (`SAT_SWEEP_ROOT_TICKS`, default lits*200 clamped 200M..2G), bound
escalation (256→8192 vars / 1024→32768 clauses / depth 2→3), pass-1 dry-run probe
(`SAT_SWEEP_ROOT_PROBE_ENVS`=2000 or 10% budget) with an all-or-nothing yield threshold
(`SAT_SWEEP_ROOT_MIN_YIELD_PERMILLE`=20) and a live-var cap (`SAT_SWEEP_ROOT_MAX_VARS`=400k);
adopters get units + original equivalence binaries + ELS + a final eliminate(true).
Result: **zero flips** on 14 under-cap timeout cells at 1800 s idle (stp212 applied 520
units + 13,518 equivalences — 8% of live vars — and still timed out, even at 3e9 ticks
with SWEEP_SUBST stacked); the probe-yield threshold separates the wrong way across
classes (TT_C392 adopts at 139‰ while goldcrest sits at 31‰ — the SESSION 6 threshold
law), so no promotable scope exists. Kept as identity-safe groundwork.

Validation: 715 unit tests (+9), smoke 9/9, drat-trim VERIFIED on every UNSAT smoke
formula in all flag combinations, byte-identity of defaults re-verified digit-exact
(rbsat 100k: 100001 conflicts / 196258 decisions / 17,758,017 propagations; MVRR 267,199
conflicts). Full session detail and screen artifacts: `plan/next-plan.md` SESSION 10.
