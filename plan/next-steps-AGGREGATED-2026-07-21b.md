# AGGREGATED next-steps plan — 2026-07-21b (supersedes next-steps-AGGREGATED-2026-07-21.md)

One-file plan for the next session. Folds the 2026-07-21 SAT_PHASE_DELTA
session (gate WIN, promoted) on top of the 2026-07-21 aggregate. Where this
contradicts an older `plan/next-steps-*.md`, THIS file wins.

## Current state (verified 2026-07-21, end of session)

- HEAD: (this session commits SAT_PHASE_DELTA default-on; see git log.)
- **SAT_PHASE_DELTA promoted (9th wall-diet win)**: incremental low-water
  phase-prefix capture. Gate `log/abtest-cand-vs-base-2026-07-21-08-17-47`:
  cand 68 vs base 67 (formal gate PASS, correctness clean), conflicts EXACT
  tie on all 67 both-solved cells, PAR-2 −2481. rbsat-v1375 was SAVED by the
  candidate (solved 1749s; the identical-legacy base arm lost it to load).
  oski15a01b20 timed out in BOTH arms (load lottery, 107.7s margin cell,
  known both-ways flipper) — idle-box confirmation run of the promoted
  config: UNSAT 1400.9s at EXACTLY the lineage 2,663,684 conflicts —
  identical trajectory, 291s faster, margin 108s → 399s. The lineage is
  intact; the gate's 68 was load.
- Baseline TSV for the NEXT A/B:
  `log/abtest-cand-vs-base-2026-07-21-08-17-47/cand/results.tsv` (68/100;
  the 69-lineage is intact modulo the oski15/rbsat load lotteries — treat
  solved-count deltas on {oski15a01b20, rbsat-v1375, sted2, TT492} as noise
  first, mechanism second).
- Kissat 4.0.4 reference: 74/100. Kissat-only cells now: Bubble, booth×2,
  fixedbandwidth, goldcrest (density UNSAT cluster), bp4_TCO_CSO_IXA_LP,
  pj2008 (SAT giants), lockchart-g1, g2-oski15a10b10 (near-unflippable),
  TT406 (41s(!) lottery).

## THE LOAD-BEARING DISCOVERY OF THIS SESSION

**On deep-trail cells, the phase-prefix capture was the dominant search
cost — O(trail) twice per decision — and nobody saw it for 9 sessions
because ticks don't count it.** pj2008 spent 131.6e9 trail-entry walks
(~250s) to produce 4,008 conflicts; with the low-water incremental capture
(byte-identical arrays) the same 250s produced 142,929 conflicts and 1.41G
props at 5.62M props/s — ABOVE kissat's 5.1M on that instance. Wall wins on
solved deep-trail cells: 6s299 −340s(!), VDW −119s, oski15b40 −92s, TT492
−71s, VexRiscv −69s (@300k screen −15.7%).

Method lesson (reusable): when search_sec is large but search_ticks/prop is
small, the sink is OUTSIDE propagation — decompose the per-decision cycle
(capture/heap/backtrack), not the prop loop. `phase_capture_entries` is the
traffic meter; SAT_STATS_JSON + a 300s wall-limited run is the decomposition
harness.

## RANKED PLAN for next session

1. **pj2008 (and oisc) follow-through — now the top prop-bound play.** The
   capture fix made pj2008 propagation kissat-competitive but it still
   timed out: search now reaches ~600-800k conflicts/1800s vs kissat
   solving SAT at ~315k conflicts in 1165s. The remaining gap is
   TRAJECTORY (what kissat's search does), not rate. Measure: SAT_STATS_HOT
   + restarts/mode profile on a 600s run; compare kissat -v phases on
   pj2008 (their preprocessing eliminates far more? their walk?). Cheap
   next probes: (a) SAT_SEARCHED=on for the >=7M-var class latched at
   first conflict (18.normalised solves at ROOT conf=0 → byte-identical;
   pj2008/oisc are timeouts → free reroll; lean giants unaffected);
   (b) preprocess budget profile on 8.6M vars (43s now — fine); (c) memory
   diet items below.
2. **Deep-trail second pass**: the capture fix exposes whatever is next on
   the same class. Re-run the pj2008 decomposition WITH delta on: if
   search_sec/prop is still >2x kissat at equal ticks, candidates are
   branch-heap churn on unassign (40M heap reinserts), enqueue triple-write
   (assignment/level/reason on 3 cache lines vs kissat's 2), binary edge
   locality. All are byte-identical-layout diet candidates (10th diet).
3. **Giant memory diet (plan-21 #2, still open, now secondary)**: RSS
   10.4GB vs kissat 1.4GB on pj2008. Known hogs: clause_abstraction
   u64×arena.len() (~744MB), binary_id_by_clause u32×arena.len() (~372MB),
   occurs Vec<Vec<u32>> (~1GB in preprocessing), nested parse for non-lean
   giants (23M clause Vecs). Trajectory-safe; matters for locality and the
   16GB cap (max_rss 10.4GB leaves little headroom for a longer run).
4. **Density/quality track (unchanged from plan-21 #1)**: learned-clause
   quality on Bubble/booth — tier-budgeted redundant vivify port
   (kissat vivifytier1=3/tier2=3/tier3=1 every probe interval vs our
   LBD<=6 + 6M-delay cursor rotation) + reduce/retention comparison.
   Measure per-tier effort/success first; scope decision after (rerolls!).
5. **The re-luck bundle (unchanged)**: if a real trajectory-rerolling
   throughput mechanism materializes, bundle WITH SAT_SEARCHED=on into ONE
   deliberate reroll gate. Do NOT spend on searched alone (2 gates lost).
6. **Zero-risk deep-arm shape (parked, analyzed this session)**: arming any
   mechanism at conflicts > max-solved-conflicts (7.87M, cell 59-129706,
   EXACT-tied again this gate) provably never touches a solved cell.
   Reachable cells are only the high-conflict-rate timeout cells (density
   UNSAT cluster + ramsey/tseitin/rphp); TT lottery cells never get there.
   Use if a cheap "last-900s regime change" candidate appears.

## Reroll-luck law (inherited, still in force)

The 69/68-lineage embeds banked wall-lottery luck; any global trajectory
reroll is −EV even with real throughput wins. Trajectory-identical diets
(9-for-9 now) and timeout-only-scoped changes are the promotable shapes.
UNSAT statuses never flip on rerolls; SAT-lottery and near-budget wall cells
do. Load ALONE flips oski15a01b20/rbsat/sted2-class cells between gates with
identical binaries — solved-count deltas on those cells are noise first.

## Measured-dead ledger additions (this session)

- (none new measured dead; phase-capture full-walk is now REMOVED-dead.)
- pj2008 did NOT flip from the 35x search-rate improvement alone — its gap
  is now trajectory/quality, not propagation rate.

## Standing traps (additions this session)

- `results.tsv` files are written only at abtest END — monitor progress from
  the launch log's `[cand]/[base]` per-cell lines, completion from "DONE ->".
- Strip `seconds_stable`/`seconds_focused` (mode wall timers) in addition to
  `*_sec`/`max_rss_mb`/`phase_capture_entries` when byte-comparing stats.
- SAT_STATS_JSON needs `=on` (`=1` silently parses false).
- perf is unusable on this box (perf_event_paranoid=4, ptrace_scope=1) —
  decompose with env-gated timers/counters in-tree instead.
- dcg blocks absolute-path shell redirects into $HOME — use repo-relative
  paths for launch logs.
- All 2026-07-21 traps remain in force (no cargo build --release while a
  gate runs; kill watcher tasks before check_promotion_gate; TSV TIMEOUT
  rows carry zero conflicts; abs-path redirections for backgrounded
  subshells; kissat -s/-q exclusive).

## Instrumentation added this session

- `phase_capture_entries` stat (trail entries walked by phase captures).
- `SAT_PHASE_DELTA` knob (on default | off = legacy full-walk A/B arm).
- Fuzz test `phase_capture_delta_is_byte_identical_to_legacy_full_walk`.

## Where the evidence lives

- This session: `plan/next-steps-phasedelta-2026-07-21.md` (full measurement
  chain + gate post-mortem), gate
  `log/abtest-cand-vs-base-2026-07-21-08-17-47` (launch log
  `log/abtest-phasedelta-launch.log`).
- Prior arc: `plan/next-steps-AGGREGATED-2026-07-21.md` (superseded; its
  ledgers/traps remain valid provenance).
- Beads: SAT-playground-pow (this session), SAT-playground-5b2.3.39
  (congruence, in progress), SAT-playground-5b2.3.50 (global-effort cadence
  redesign, open).
