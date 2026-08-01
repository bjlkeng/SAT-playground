# NEXT PLAN — 2026-07-31 (supersedes 2026-07-30; PRUNED)

One-file plan for the next clear context. SESSIONS 4-13 bodies were pruned
from this file — their full text lives in git history (`git log -p
plan/next-plan.md`, revisions up to 52a8f95) and their verdicts survive in
"Standing traps", "Closed lines", and the memory files. Where this file
contradicts an older plan revision, THIS file wins.

**START HERE:** read "SESSION 14b" below, then "RANKED PLAN", then
"Standing traps".

## SESSION 14b (2026-07-31) — FULL-BENCH PROMOTION: 261 → 271/400 at 3600 s (gate WIN +10); three first-ever solves; two runaway-pass bugs fixed; root-pass scoping law confirmed out-of-sample

**Objective (user /goal): improve the FULL-bench (sat-comp-2025, 400
cells) solve count at 3600 s / 16 GB / 32 cores. The medium-1800 s gate
remains the repo's promotion metric for ordinary sessions; this session
promoted on a full-bench 3600 s paired A/B by explicit instruction.**

Final A/B (`log/abtest-cand-vs-base-2026-07-31-06-41-31`, 400 cells x 2
arms, simultaneous start, proofs verified both arms):

| arm | solved | PAR-2 | verdict |
|---|:--:|--:|---|
| cand (new defaults @ f125734) | **271/400** | 1,094,782 | **WIN (+10)** |
| base (fixes only, bundle off) | 261/400 | 1,143,378 | — |

Zero SAT/UNSAT contradictions, zero proof/model failures (267 verified
ok; 4 checker-timeouts, same historical cells). The base arm reproduced
the 2026-07-29 baseline count exactly (261). vs kissat 4.0.4 (296/400 on
the same bench 07-29): gap −35 → **−25**.

**Cells gained (+14 vs the 07-29 baseline):** THREE first-evers that
NOBODY (kissat included) solves at 3600 s — MVRoundRobin_n14_d10_v2
(UNSAT 3465 s), RoundRobin_n18_d15 (UNSAT 2981 s), at-least-two-vmpc_28
(SAT 1534 s) — plus battleship-13-13 (UNSAT 122 s, bug fix + reduce law;
kissat 21 s), bivium-39 (UNSAT 2671 s), gto_p60 (612 s), contest04
(942 s), oddball_13_5_ttf (429 s), bp4_BC012_IXA_LPI (3335 s),
bp4_TCO_IXA_FPBLE_ZR (SAT 3453 s; kissat needs 3466 s), reconf10_22
(2094 s), blockpuzzle (272 s), VdW-23 (1341 s), sted2_0x0_n219-342
(670 s). **Lost (−4):** rbsat (documented coin flipper), case6 (3421 s
thin-margin wall cell), 170223547 + lockchart-group1-L210 (deep-SAT/wall
lottery; lockchart lost in BOTH arms = pure wall coin). Trade: 3
unique-capability first-evers + 10 mechanism-backed flips vs ~2 real
reroll losses + 2 coins — clean under any reading of the trade rule.

### What shipped (all in `.rs`, commits 0f12bd0 + f125734)

1. **`sweep_round` kitten tick budget (`SAT_SWEEP_KITTEN_TICKS`, 200M
   per round).** The legacy `prove_facts` wrapper gave every mid-search
   sweep kitten call an UNLIMITED budget; one environment on
   battleship-13-13 sat in a single exponential kitten solve for the
   cell's whole 3600 s (gdb-confirmed; proof frozen at byte-identical
   150,995,327 bytes across runs). Healthy rounds measure ~31M ticks
   worst-case, so 200M is inert there (budgeted core is
   decision-identical while unexhausted; rbsat/MVRR fingerprints
   digit-exact). battleship: timeout → UNSAT 950-1037 s on the fix
   alone, 122 s with the reduce law.
2. **Gauss work budgets (`SAT_GAUSS_ORDER_WORK`, 100M "touched row
   entries" for min-degree ordering + same-scale combine budget).**
   `try_gauss_refute` fell through to the resolution-only fallback on
   100k-equation XOR systems: `min_degree_order` (gauss.rs:474) spun
   ~25 min in HashMap churn, then elimination fill-in allocated 31.4 GB
   (gdb-confirmed) — tseitin_d3_n100000's rc-6 abort; the cell got zero
   search time. Now declines in ~5.6 s, search runs, peak 1.6 GB.
   xor_op n36/n40 proofs still emit + drat-trim VERIFIED.
3. **Mid-giant BVE resolvent cap (5-20M-var instances, 50M MATERIALIZED
   resolvents, `SAT_GIANT_ELIM_RESOLVENTS`).** Root BVE has no GC inside
   the pass (occurrence lists hold raw clause ids), so pj2016_k100's
   100M-resolvent pass doubled the arena into an exactly-8-GiB mapping,
   peak 17.9 GB virtual → `ulimit -v` kill at 53 s. Attempts do NOT
   separate the classes (solved band cells also exhaust 100M attempts)
   but materialized resolvents DO: solved 5-20M-var cells peak at 8-33M
   resolvents (probed, all byte-identical under the cap by
   construction); pj2016 trips at 50M → peak 9.8 GB, search at 39 s.
   pj2016/pj2008 still don't SOLVE (kissat's SAT wins there are search
   quality, not survival) — the cap is hygiene + enabler.
4. **`SAT_REDUCE_FRACTION` default ON (kissat reduce.c deletion law).**
   Unchanged scoping: activates at first reduce ≥1.3M conflicts AND
   never on `inprocess_aggressive`-armed cells — banked armed cells
   untouched by construction. This carried most of the +10: the
   SESSION-5 "value at >3000 s horizons only" prediction confirmed
   out-of-sample (frontier screen: reduce arm 9/38 vs base 2/38).
5. **`SAT_ELS` default ON with a percent-scale apply threshold
   (`SAT_ELS_MIN_SUBST_PERMILLE`, default 50 = 5%).** The ROOT
   standalone ELS pass computes SCCs, then applies ONLY when merge mass
   ≥ 5% of live vars; below that it declines with ZERO mutation —
   byte-identical to els=off (odd51: declines at 28/44,908 = 0.6‰;
   blockpuzzle: applies at 3,426/50k = 6.9%). Congruence/sweep/round
   substitution through try_els are NOT gated. This is the
   decline-is-identity dry-run shape (gbve/congruence/transitive), NOT
   a ranking threshold — the THRESHOLD-LAW objection does not apply.
6. **`SAT_PROBE` and `SAT_SWEEP_ROOT` stay default OFF.** The union
   bundle's first full-bench A/B (killed at ~9.5 h, 253 paired cells,
   `log/abtest-cand-vs-base-2026-07-30-21-11-*` + log
   `fullbench-ab-final-20260730-211120.log`) measured them NET-NEGATIVE:
   cand 155 v base 168, with base's 23 exclusive wins ALL SATISFIABLE —
   including banked TT496 + all four oddball-tto_zp cells. Rescue
   probes: the oddballs solve again with root passes off. Find-mass
   probes: the root-arm "wins" (Circuit24: 54 edits) and the banked
   losses (TT496: 145 edits) are the SAME tiny-edit phase-lottery —
   only percent-scale mass (blockpuzzle 6.9%, bv_ILA 35%) is mechanism.
   **ROOT-PASS SCOPING LAW (out-of-sample confirmation of
   REROLL-VARIANCE): an unscoped root pass that edits O(100) variables
   on cells where it finds nothing structural is a net-negative SAT
   lottery at ANY wall; only decline-is-identity mass thresholds make
   root passes shippable.**

Validation: 740 tests green (default-expectation tests updated;
`solve_pre_bundle` helper for trajectory tests), smoke 9/9, identity
refs digit-exact after fixes (rbsat 100001/196258/17,758,017; MVRR
267,199). New suite `benchmarks/frontier-2026-07-30` (38 out-of-sample
cells; screens: 4-arm `log/abtest-reduce-vs-inproc-vs-root-vs-base-...`,
union `log/abtest-union-vs-reduce-vs-base-...`).

### Remaining gap analysis (271 vs kissat 296, −25)

Kissat-only classes after this session (approximate, from the 07-29
kissat run joined with A/B2 cand):

- **16x16 multiplier miters (~10 cells + 4 both-timeout): the #1 family
  gap, UNTOUCHED** — no arm flipped any at 3600 s. Probe: kissat wins
  via 74% BVE collapse + sustained 11.6k conf/s over 6.2M conflicts +
  359M walk steps; our elimination parity exists but we need >20M
  conflicts at 7.6k/s. Needs a genuine trajectory-quality mechanism,
  not a flag.
- **Walk-scale SAT cells (~6: Circuit_multiplier24/29, ITC x2, HCP-446,
  ER_400, shuffling-1):** kissat walks 100-360M steps; our armed walk
  does 3-27M. The A/B1 root-arm "wins" here were rerolls, not walk. A
  walk-effort screen is plausible but reroll-lottery-adjacent — use the
  frontier canaries.
- **Starved hwmcc/BMC (goldcrest, fixedbandwidth, x-epic, nla-digbench,
  b18/b19, g2-oski):** tick-cadence inprocessing measured negative at
  1800 s AND the inproc arm flipped none at 3600 s. Genuinely hard.
- **pj SAT giants (pj2016/pj2008):** survive memory now, need SAT
  search luck/quality. **uniqinv40:** needs kissat-scale sweep
  SUBSTITUTION mid-search (SWEEP_SUBST exists; try a percent-mass
  threshold like ELS?). **grs x2, sqrt-mitern169, myciel6, SGI, rook-51,
  lec_mult, SAT_dat.k100, oddball_24_4/26_4/112_5, Bubble_8_4/9_4,
  Timetable_C_492, lockchart-group1 x2, dislog, mod2c/mod4block,
  fsf x2, case8, b19_1, ncc, ER_500, ITC_Late, HCP-446, myciel,
  x-epic...** — long tail, mostly needing the miter/rate mechanisms.
- Both-timeout hard core: ~71 cells (was 75; −3 first-evers, −bivium).

## RANKED PLAN (2026-07-31)

1. **php/counting-detector coverage pass (cheapest capability, proven
   SESSION-11 shape, zero reroll risk).** Decode why these both-timeout
   cells decline: cliquecoloring_n14_k7_c6 (n15/n26 siblings FIRE),
   clqcl_30_9_8, clqcl_30_11_10 (k=9/11 — check the 3-7-longest-covers
   precheck and slot generalization), harder-fphp-016-015 (direct php
   shape), rphp_p25_r25 (check P>H holds first). Nobody solves any at
   3600 s. Expected +2-4 full-bench; medium byte-identical.
2. **Multiplier-miter mechanism hunt (the #1 family, 10-14 cells).**
   Offline first: decompose WHERE kissat's 6.2M-conflict refutation
   beats our >20M (tier limits? vivify quality? reduce law interplay
   now that it ships? re-probe boothbit29 with the new defaults). Any
   flip here generalizes across the family.
3. **SWEEP_SUBST behind a percent-mass threshold (uniqinv40-class).**
   Same decline-is-identity shape as the ELS threshold; uniqinv40 needs
   30% sweep-substitution mass (kissat gets it mid-search). Screen on
   frontier before any default talk.
4. **Medium-1800 re-baseline (bookkeeping, next ordinary session).**
   The new defaults change the medium gate's baseline; run the standard
   medium single-seed A/B (new defaults vs f125734-with-bundle-off) at
   1800 s to re-anchor the 74/100 lineage before any medium-metric
   session. Exposure is small (reduce ≥1.3M conflicts; ELS declines on
   ~all medium cells — verify with the identity refs) but must be
   measured, not assumed.
5. **Walk-effort screen on the SAT frontier cells** (4-arm, frontier
   canaries in every arm; expect lottery — demand mechanism evidence
   like walk-step parity, not just flips).
6. **Giant memory diet phase 2 (17.normalised parse; pj-class search
   RSS)** — only relevant under a 30 GB objective; park.
7. **Checker-timeout proof-size arc (4 at-risk solves)** — downstream
   of trajectory quality; track only.

## Current state

- HEAD: f125734 (SESSION 14b final shape; 0f12bd0 same session).
  **Full-bench 3600 s baseline: 271/400** (A/B2 cand arm TSV =
  `log/abtest-cand-vs-base-2026-07-31-06-41-31/cand/results.tsv`).
  kissat 4.0.4 reference: 296/400 (`log/kissat-full-20260729-210758`).
- **Medium-1800 s baseline: NEEDS RE-MEASUREMENT under the new defaults
  (ranked item 4); last measured 74/100 at c469b03 (pre-bundle).**
  Medium-3600 inside A/B2: cand 75 v base 76 (noise band).
- Default surface added this session: SAT_SWEEP_KITTEN_TICKS=200M,
  SAT_GAUSS_ORDER_WORK=100M, SAT_GIANT_ELIM_RESOLVENTS=50M (5-20M-var
  scope), SAT_REDUCE_FRACTION=on (scoping unchanged), SAT_ELS=on +
  SAT_ELS_MIN_SUBST_PERMILLE=50, SAT_PROBE=off, SAT_SWEEP_ROOT=off.
- Full gap read (07-30) of the pre-session state:
  `plan/gap-read-full-2026-07-30.md` + `log/gap-read-full-2026-07-30/`.
- Tools: `tools/run_kissat_full.sh` (-d suite, -c core offset, -j jobs);
  suite `benchmarks/frontier-2026-07-30` (38 cells).

## Standing traps (updated 2026-07-31 + carried)

- **SESSION 14b:** NEVER `cargo build` the solver dir while ANY
  feature_ablation run is live — later-launched cells silently pick up
  the new binary (this contaminated and killed A/B1's tail). Build to a
  scratch `CARGO_TARGET_DIR` instead. `pkill -f` with a pattern that
  appears in your own command line kills your own shell (exit 144) —
  use the `[b]racket` trick in the PATTERN itself. The ELS threshold
  gates ONLY the root standalone pass via the transient
  `els_apply_min_permille`; congruence/sweep/round substitution must
  never be gated. `SAT_WALK` env name is PARKED (denylist) — walk tuning
  goes through SAT_WALK_EFFORT / SAT_REPHASE_ARMED_ONLY.
- **SESSION 14b:** reduce-law deep-cell coin exposure at 3600 s is
  real but small: rbsat/case6/170223547-class (deep unarmed cells past
  1.3M conflicts). Judge those as coins, not capability, in any future
  full-bench A/B.
- **SESSION 14:** full-bench 3600 s numbers vs medium-1800 s gate —
  keep both ledgers separate; a −35 full-bench read coexisted with a
  WON medium 76v75. `ulimit -v` kills on VIRTUAL memory (RSS
  understates; use /usr/bin/time -v + VmPeak). rc-6 in a seedgate TSV =
  allocator abort. SAT_LIMIT_WALL_SEC is honored ONLY in the CDCL loop
  (sweep-kitten and gauss paths now bounded by ticks/work instead).
- **Carried (from SESSIONS 4-13, verdicts still binding):** deal noise
  is ±2 solved cells (medium); conflicts deterministic across load,
  wall is not; marginal-cell TIMEOUT untrustworthy under 32-way
  contention (solves are trustworthy); wall-coin flipper list rbsat /
  vex / oski15 / VdW-22 (+case6, 170223547 at 3600 s); activity proxies
  mislead — never optimize them; FEATURES.md/CONFIG_SCHEMA.csv are
  STALE — read src/config.rs + env reads in main.rs; results.tsv is
  written only at run END; stats JSON goes to stderr and timed-out runs
  emit none (use SAT_LIMIT_CONFLICTS probes); `pgrep -f
  feature_ablation` in monitors matches itself; heredoc scratch writes
  flake — use the Write tool; perf is blocked (use the gdb
  SIGINT-sampler with `handle SIGINT stop print nopass`; `noprint`
  implies `nostop`); build to scratch CARGO_TARGET_DIR when anything is
  running; `rm -rf` guarded in scratch scripts — use timestamped dirs.
- **Carried ER/proof laws:** RAT-scan law — ER-proof verify cost =
  #definitions x instance maxVar, NOT lemma count (grid_n400
  checker-bound, PERMANENTLY CLOSED under drat-trim). Residue/retry law
  — never stream an aborted ER attempt into the live proof (deletions
  break the retry's RUP; dry-run with sink emitters). Deletions are
  load-bearing for definition-var recycling (no-del proofs do NOT
  verify). tseitin caps stay legacy; SAT_TSEITIN_SNAKE stays OFF at
  1800 s (checker-timeout = gate correctness FAIL).
- **Carried closed lines (do not reopen without new mechanism):**
  starved-cell tick-cadence pipeline (negative at 1800 s AND no flips
  at 3600 s); unscoped root ELS/PROBE/SWEEP_ROOT defaults (SESSION 14b
  A/B1 net-negative — threshold variants only); SAT_ELIM_DEF at any
  budget (fallback bug documented); vivify tier-split; gbve-adopter
  rounds; units-only transitive; per-mille RANKING thresholds for
  adopting root passes (THRESHOLD LAW — decline-is-identity mass gates
  are the exception that works); ramsey ER emission (research-scale);
  st_659 (no certificate family; status UNKNOWN at 4x wall).

## solver12's capability edge (protect in rerolls)

First-evers banked this session (nobody else solves at 3600 s):
**MVRoundRobin_n14_d10_v2, RoundRobin_n18_d15, at-least-two-vmpc_28.**
Carried: rphp5_050/085, clqcl_40/50_6_5 + 5 cliquecoloring siblings
(SAT_PHP_REFUTE, pre-search, reroll-immune), xor_op x2 (SAT_GAUSS),
tseitin_n188_d3, RoundRobin_n15-n17 class + MVRR x3 (gate-BVE),
oddball-tto_zp x4 + TT_C496 + TT_C406 (endgame/arming — CONFIRMED
protected by the final shape; they died under unscoped root passes),
Kakuro-132, HCP-529, frb80-14-1, valves-gates (checker-timeout caveat),
oddball_13_5_ttf + battleship + bivium + gto + contest04 + reconf10_22 +
blockpuzzle + VdW-23 + sted2var + bp4_BC012_IXA + bp4_TCO_IXA (SESSION
14b, reduce/els/fix-backed).

## Where the evidence lives

- This session: `log/abtest-cand-vs-base-2026-07-31-06-41-31` (A/B2,
  THE verdict), `log/abtest-cand-vs-base-2026-07-30-21-11-*` via
  `log/fullbench-ab-final-20260730-211120.log` (A/B1 union bundle,
  killed, attribution data), frontier screens
  `log/abtest-reduce-vs-inproc-vs-root-vs-base-2026-07-30-11-56-05` and
  `log/abtest-union-vs-reduce-vs-base-*`, backtraces + probes in
  `log/gap-read-full-2026-07-30/`.
- Pre-session full-bench gap read: `plan/gap-read-full-2026-07-30.md`.
- Mechanism deep dives: `plan/kissat-gaps.md`,
  `plan/gap-read-2026-07-21.md`.
- SESSIONS 4-13 full text: git history of this file (up to 52a8f95).
