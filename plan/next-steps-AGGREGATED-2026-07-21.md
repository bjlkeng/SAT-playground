# AGGREGATED next-steps plan — 2026-07-21 (supersedes next-steps-AGGREGATED-2026-07-20b.md)

One-file plan for the next session. Folds the 2026-07-20/21 SAT_SEARCHED
session (two full gates, both FAIL, mechanism validated and parked
default-off) on top of the 2026-07-20b aggregate. Where this contradicts an
older `plan/next-steps-*.md`, THIS file wins.

## Current state (verified 2026-07-21, end of session)

- HEAD: (see git log — this session commits the SAT_SEARCHED
  infrastructure DEFAULT-OFF as neutral groundwork on top of c8228aa; the
  tree with SAT_SEARCHED=off is byte-identical-trajectory to c8228aa,
  verified on ibm full-solve, vex @300k, bubble @1.5M stats byte-compare.)
- Solved lineage: **69/100** (realized in
  `log/abtest-cand-vs-base-2026-07-20-12-03-06/cand/results.tsv`, which
  REMAINS the baseline TSV for the next A/B). Kissat 4.0.4 reference:
  74/100. Net gap 5.
- Gate #1 this session `log/abtest-cand-vs-base-2026-07-20-22-35-43`
  (always-on searched): FAIL 63v69. Gate #2
  `log/abtest-cand-vs-base-2026-07-21-02-30-51` (armed@512ppc/300k):
  FAIL (fresh SAT-lottery losses + oski15 16GB OOM; see below).
- Wall-lottery margins (from the 69-lineage gate, unchanged): rbsat 5.4s
  (coin-flip, NEVER build on it), oski20 107.7s, sted2 1636s-class,
  oski40, vex. Gate #1/#2 base arms confirmed these cells' outcomes are
  load-sensitive lotteries (oski15 flipped base-solved/base-timeout
  between two gates SIX HOURS apart).

## THE LOAD-BEARING DISCOVERY OF THIS SESSION (read this first)

**The 69-solved baseline embeds accumulated wall-lottery luck, and ANY
global trajectory reroll regresses that luck to the mean.** The evidence:

- SAT_SEARCHED (kissat clause.h saved replacement-scan position) is a REAL
  throughput win: clean paired bp4 screen wall −10.1%, props/s +11%,
  ticks/prop −29%; measured basis: 61% of bp4 search ticks were
  replacement-scan literal rescans (21.66 scanned lits/prop, avg 12.7 per
  loaded clause).
- Always-on gate: the 3 gains were EXACTLY the targeted kissat-only cells
  (bp4_TCO_CSO_IXA_LP SAT 1018s, TT406 SAT 250s(!), oski15 UNSAT 1766s) —
  but 9 SAT-lottery cells rolled out (jkkk base 7s!, bc012f 209s, oddball
  286s, case1 340s, VDW 1301s, TT492 1585s, sted2x 1615s, bp4_ZR 1738s,
  rbsat 1795s). Net −6.
- Armed variant (ppc>=512 after 300k conflicts, latched at restarts;
  sub-threshold cells byte-identical): still lost fresh victims (cells
  that survived gate #1's reroll lost their luck in gate #2's different
  draw), plus oski15 aborted at the 16GB cap — armed trajectory ballooned
  to 15.9GB RSS where base stays under. Trajectory rerolls change MEMORY
  peaks too.
- ppc CANNOT separate winners from victims: gains span 554-1063
  props/conflict, losses span 434-1435 (the bp4-family SAT lotteries are
  the most propagation-heavy of all). goldcrest (ppc 3421) did NOT flip
  under always-on searched, so a ppc>=2000 surgical variant has no
  expected gains either.

Consequences for ALL future work:

1. The trajectory-identical wall-diet discipline (8-for-8) was not a
   style choice — it is the ONLY promotable shape while the margin
   structure is lottery-banked.
2. Trajectory-rerolling improvements (searched, canonicalization,
   restart/vivify changes...) can only land (a) bundled into a gate that
   rerolls anyway, (b) after a deliberate "re-luck" campaign that re-banks
   margins around the new default, or (c) scoped so tightly that no
   currently-solved cell's trajectory changes.
3. Solve-time variance on this suite's SAT cells is enormous: base-arm
   oski15 was TIMEOUT in gate #1 and UNSAT 1638s in gate #2 with an
   IDENTICAL binary. Single-cell flips near the budget are load noise as
   much as mechanism. (Both gates also re-confirm: UNSAT statuses never
   flipped on rerolls — only SAT-lottery and near-budget wall cells did.)

## In-tree but default-off: SAT_SEARCHED (this session's artifact)

- 6-bit resume position in spare clause-header bits (size field 27->21
  bits, hard assert on overflow; header line is loaded/dirtied by the hot
  loop anyway — ZERO extra memory/cache traffic). Wraparound scan in both
  PTR_FAST and legacy paths. All header rebuild sites preserve the field
  (CLAUSE_SEARCHED_POS_MASK); shrink sites reset it. Giants force it off.
- Env: SAT_SEARCHED=off (default) | armed | on; SAT_SEARCHED_ARM_PPC
  (512), SAT_SEARCHED_ARM_MIN_CONFLICTS (300k); stat
  `searched_armed_at_conflict` in JSON.
- TRAP (measured, do not resurrect): a trailing-word variant of the same
  port (extra arena word after the extras) is a NET wall REGRESSION
  (+3.4% clean bp4) despite −16% ticks — the tail-line touch costs more
  than the scan saves. Kissat's placement (header) is load-bearing.
- The always-on flip evidence (bp4_IXA_LP/TT406/oski15) makes this the
  single strongest known mechanism for the propagation-bound kissat-only
  cells — it is parked, not dead.

## Session measurement library (new, reusable)

- bp4_TCO_CSO_IXA_LP @2M conflicts: ours 790s vs kissat 243s; props/s
  2.23M vs 11.7M (5.2x); ticks/prop ours 52 vs kissat 3.15 (their units:
  cache lines). SAT_STATS_HOT decomposition @300k: watcher visits
  13.78/prop (82% blocker hits), clause loads 1.71/prop, replacement-scan
  lits 21.66/prop (61% of ticks) — killed by searched (24.6 ticks/prop,
  −29%). AFTER searched, bp4 is still ~2.4x off kissat: next chunks are
  visit count (watch-list length / DB size / tier policy) and cache-line
  organization.
- Bubble @5M conflicts: ours 527s vs kissat 226s; props/conflict 191 vs
  103.5 (1.85x — SEARCH QUALITY gap, restart interval 528 vs 39, kissat
  vivified 335k=53% of checks vs our 43k strengthened); props/s 1.81M vs
  2.29M (1.27x). Restart-cadence and vivify-hit-rate single ports measured
  no-flip in prior sessions; what remains is learned-clause quality
  (kissat's tier-budgeted vivify of REDUNDANT clauses every probe
  interval) + reduce/retention policy differences (kissat: interval-based,
  50-90% fraction deletion; ours: literal-budget-driven).
- pj2008 (giant 8.6M vars): kissat 200k conflicts in 739s, 18.9k
  props/conflict, 5.1M props/s, RSS 1.4GB; ours did not reach 200k in
  >2100s CPU, RSS 6.7GB (4.8x kissat's memory!). Propagation-bound AND
  memory-layout-bound. goldcrest ppc 3421 (same class, non-giant).
- Clause-length profile bp4: originals mostly len 2-3 with 100k len 7-11;
  learned avg LBD 26 — long learned clauses carry the scan cost.

## RANKED PLAN for next session

1. **Density/quality track (now the top play again)**: learned-clause
   QUALITY, not cadence — measure our vivify's per-tier effort/success on
   Bubble/booth vs kissat's vivifytier1=3/tier2=3/tier3=1+irr=3 budget
   split (kissat vivifies redundant tiers EVERY probe interval at
   tick-relative effort; our learned vivify is cursor-rotated with
   LBD<=6 gate and 6M-conflict delay on non-armed formulas). A
   tier-budgeted learned-vivify port is trajectory-REROLLING on armed
   cells only if it changes edits — measure first, decide scope after.
   Reduce/retention comparison (kissat reduce fractions vs our
   literal-budget) belongs to the same measurement session.
2. **pj2008 memory-layout measurement** (cheap, no code): ours 6.7GB vs
   kissat 1.4GB on the same formula is a 4.8x LIVE-footprint gap —
   decompose (arena vs watchers vs per-var arrays vs binary index) with
   SAT_STATS_JSON memory fields + /proc sampling. A giant-scoped memory
   diet is trajectory-safe (byte-identical trajectories, less RSS) and
   may flip pj2008 wall by locality alone.
3. **bp4 watch-visit reduction**: after searched, visits (13.78/prop) are
   the next bp4 chunk. Mechanisms that DON'T reroll: none known (blocker
   choice, tier policy, DB size all reroll). Mechanisms that reroll →
   bundle with SAT_SEARCHED=on into ONE deliberate re-luck gate (see 4).
4. **The re-luck bundle (deliberate, ONE gate, priced as a campaign)**:
   if/when a second real throughput mechanism materializes (e.g. #3, or
   canonicalization+cross-invocation extract-cache from plan-20b#3),
   bundle it WITH SAT_SEARCHED=on into a single reroll gate and accept
   1-2 gates of variance to re-bank margins around the faster core. Do
   NOT spend this on searched alone (2 gates already spent: −6 and ~−2).
   Prediction discipline: enumerate the current gate's marginal cells
   (>1500s solves) as expected-loss candidates; the throughput wins must
   exceed that count.
5. **TT406 stabilizer** — note the new evidence: TT406 flipped IN under
   always-on searched (250s) while TT492 flipped OUT; the TT class
   seesaws under ANY reroll. Unchanged verdict: blind rerolls are −EV
   while TT492 is in; a mechanism hypothesis is still missing.
6. **Wall-diet arc**: still DONE. Remaining chunks (closure occ
   Vec-of-Vec ~0.36s/round, sweep snapshot clones <2s/cell) — bundle
   only into a gate happening anyway.

## Measured-dead ledger additions (this session)

- SAT_SEARCHED always-on default: 63v69 (9 SAT-lottery losses; gains were
  real but outnumbered). Armed@512/300k: fresh lottery losses + oski15
  16GB OOM. ppc thresholds cannot separate gain cells from loss cells.
- Trailing-word searched layout: +3.4% wall clean (cache-line cost).
- ppc>=2000 surgical arming: no expected gains (goldcrest didn't flip
  under always-on).
- (Inherited, unchanged: density inprocessing ensemble; restart
  floors/margins; vivify-deduce/sort; rephase/walk; backbone; elim-def;
  trail reuse; walk warmup; lit-indexed values; congruence-learned; etc.
  — see 2026-07-20b for the full list.)

## Standing traps (additions this session)

- `pgrep -f feature_ablation` in monitor shells self-matches the monitor's
  OWN cmdline — completion-detect from the launch log ("DONE ->" line),
  and kill watcher tasks by id BEFORE check_promotion_gate (its
  running_solver_processes check flags them; gate #1's formal output has
  a cosmetic failure line from exactly this).
- Trajectory rerolls change MEMORY peaks, not just walls (oski15 armed:
  15.9GB vs base under 16GB). Any reroll gate on 16GB jobs can turn a
  solved cell into UNKNOWN_rc-6 (SIGABRT on alloc failure).
- cargo build --release while a gate is running would swap the binary
  under the ablation (it references target/release) — cargo check only
  until the gate's solvers are done.
- The new stats key (`searched_armed_at_conflict`) shows up as a 1-key
  diff when byte-comparing new-binary stats vs pre-change-binary stats —
  strip it (like *_sec fields) when doing cross-binary identity screens.
- All 2026-07-20b traps remain in force (abs-path redirections for
  backgrounded subshells, TSV TIMEOUT rows carry zero conflicts, kissat
  -s/-q exclusive, etc.).

## Instrumentation added this session

- `SAT_STATS_HOT=1` (existing but now load-bearing): watch_scans /
  watch_blocker_hits / watch_clause_loads / binary_props decomposition —
  the tool that found the 61% scan share.
- `searched_armed_at_conflict` stat; SAT_SEARCHED/SAT_SEARCHED_ARM_PPC/
  SAT_SEARCHED_ARM_MIN_CONFLICTS knobs.
- Off-switch A/B knobs inherited: SAT_EXTRACT_CACHE, SAT_CLOSURE_DIET,
  SAT_ROUND_DIET, SAT_ELIM_SCRATCH, SAT_CONGRUENCE_FASTIDX, ...

## Where the evidence lives

- This session: `plan/next-steps-searched-2026-07-20.md` (full
  measurement chain + both gate post-mortems), gates
  `log/abtest-cand-vs-base-2026-07-20-22-35-43` (always-on, launch log
  `log/abtest-searched-launch.log`) and
  `log/abtest-cand-vs-base-2026-07-21-02-30-51` (armed, launch log
  `log/abtest-searched-armed-launch.log`).
- **Baseline TSV for the NEXT A/B (unchanged)**:
  `log/abtest-cand-vs-base-2026-07-20-12-03-06/cand/results.tsv` (69/100).
- Prior arc: `plan/next-steps-AGGREGATED-2026-07-20b.md` (superseded but
  its ledgers/traps remain valid provenance).
- Beads: SAT-playground-5b2.3.39 (congruence, in progress),
  SAT-playground-5b2.3.50 (global-effort cadence redesign, open).
