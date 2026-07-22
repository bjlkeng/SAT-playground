# AGGREGATED next-steps plan — 2026-07-21c (supersedes next-steps-AGGREGATED-2026-07-21b.md)

One-file plan for the next clear context. Folds the 2026-07-21 **gap-read +
code deep-dive** session on top of the 2026-07-21b aggregate (SAT_PHASE_DELTA
promotion). Where this contradicts an older `plan/next-steps-*.md`, THIS file
wins. Full detail of this session: `plan/gap-read-2026-07-21.md`.

## TL;DR — what changed this session

1. **A fresh, fair 32-way paired read says the solver↔kissat gap is ≈TIED,
   not −7.** solver12 **67/100** (PAR-2 141667), kissat 4.0.4 **68/100**
   (PAR-2 142014) — solver12 is actually −347 PAR-2 (better). The banked
   "68–69 vs 74" is **tail-contention lottery**, not capability (see below).
2. **But a code+instrumentation deep-dive found a REAL, mechanism-level
   capability gap: solver12's INPROCESSING doesn't shrink formulas the way
   kissat's does.** The losses are NOT trajectory rerolls. This reframes the
   prior plan: the top lever is **SAT sweeping + inprocessing cadence**, not
   pj2008 propagation trajectory.
3. **solver12 has its own capability edge kissat lacks** (XOR/parity: Gauss +
   pair-abs). The two solvers have *different* frontiers. Don't trade ours away.

## Current lineage state (unchanged provenance from 21b)

- HEAD: `81342c2` (SAT_PHASE_DELTA default-on, 9th wall-diet win). Gate
  `log/abtest-cand-vs-base-2026-07-21-08-17-47`: cand 68 vs base 67, PASS,
  conflicts EXACT tie on 67 both-solved, PAR-2 −2481.
- **Lineage baseline TSV for the next A/B**:
  `log/abtest-cand-vs-base-2026-07-21-08-17-47/cand/results.tsv` (68/100).
  A second fresh baseline from this session (67/100, load-flipped rbsat):
  `log/seedgate-default-2026-07-21-11-37-05/results.tsv`.
- Reroll-luck law still in force (see bottom). 69/68-lineage embeds banked
  wall luck; treat solved-count deltas on {oski15a01b20, rbsat-v1375, sted2,
  TT492} as load noise first, mechanism second.

## The 32-way tail-contention finding (why "kissat 74" was misleading)

kissat is deterministic per instance, so a solved→timeout swing is pure
wall-clock. Under 32-way memory-bandwidth pressure the same computation runs
slower and **marginal-tail cells (prior solve 1050–1760 s) cross 1800 s.**
Six kissat cells the 2026-07-05 idle-ish run solved LATE regressed to TIMEOUT
this run (case1, VanDerWaerden_22, Timetable_C492, g2-oski10, booth_wallace,
pj2008). solver12's tail flips the same way (rbsat/oski15/sted2/TT492). So the
paired 32-way count is a ±3-cell coin flip on the tail. **Report the gap as
≈tied; do NOT chase the −7.** The capability gap below is the real target.

## THE DEEP-DIVE VERDICT — capability gap in inprocessing (NOT trajectory)

Method: kissat `-v` full solve (mechanism ID; deterministic ⇒ contention-
immune) vs solver12 `SAT_STATS_JSON` 300 s window, on identical CNF, for every
loss cell. Raw: `log/gap-read-2026-07-21/deepdive/{*.kissat.out,*.s12.out,
COMPARISON.txt}`.

**kissat wins by SHRINKING the formula (elimination + kitten SAT-sweep +
equivalence substitution) so search runs on a smaller problem; solver12
searches the un-shrunk formula and can't close it even with more conflicts.**

Decisive not-trajectory proof:
- **TT_C406**: solver12 = 1.13M conflicts and FAILS; kissat = 170k conflicts
  and solves (6.6× fewer) after eliminating 67% of vars + 824k kitten solves.
  More search, worse result ⇒ capability.
- **lockchart_g1**: kissat solves in 396k conflicts on 10.9M kitten solves;
  solver12 has 0% elim + 0 sweep, grinding raw.

Evidence table (kissat full solve vs solver12 300 s window):

| cell | k elim% | k kitten | k sweep_solved | k subst | s12 elim% | s12 sweep-finds |
|------|--:|--:|--:|--:|--:|--:|
| Bubble UNSAT | 72 | 90k | 65k | 314 | 43 | 85 |
| booth_dadda UNSAT | 77 | 81k | 59k | 26 | 56 | 319 |
| booth_wallace UNSAT | 77 | 96k | 75k | 40 | 56 | 42 |
| TT_C406 SAT | 67 | 824k | 189k | 136 | 55 | 5 |
| g2_oski10 UNSAT | 88 | 4.5M | 2.4M | 10560 | 82 | 0 |
| goldcrest UNSAT | 85 | 4.7M | 2.0M | 38531 | 54 | 0 |
| oski15a01 UNSAT | 74 | 2.1M | 380k | 71487 | 56 | 826 |
| lockchart_g1 SAT | — | 10.9M | 454k | 0 | 0 | 0 |
| pj2008 SAT giant | 54 | 18.4M | 9.6M | 3.52M | 28 | 0 |
| bp4_TCO SAT | 24 | 649k | 550k | 28686 | 30 | 0 |
| fixedbw UNSAT | 25 | 10k | 8k | 0 | 11 | 0 |
| rbsat1375 SAT | — | 145k | 86k | 0 | 0 | 0 |

## RANKED PLAN for next session (re-ranked by deep-dive evidence)

1. **Make SAT sweeping actually productive (BIGGEST LEVER).** solver12's
   sweep is effectively inert: 0–826 finds vs kissat's 90k–18M kitten solves.
   Even when it fires (oski15a01: 826) it is ~450× less productive than
   kissat (380k). This touches nearly every loss cell (density UNSAT cluster,
   giants, circuit miters). Investigate `src/sweep.rs`: depth/var/clause caps
   (`SAT_SWEEP_DEPTH/MAX_VARS/MAX_CLAUSES`), whether the kitten-equivalent
   sub-solver is under-budgeted or the guard skips too aggressively. This is
   the single highest-value capability to close.
2. **Tick/time-budget the inprocessing cadence (concrete defect).** solver12
   gates inprocessing on a **1M-conflict cadence**; slow-conflict giants
   NEVER reach it in a full run — goldcrest 474 c/s → 0.85M in 1800 s < 1M;
   lockchart 330 c/s → 0.59M < 1M. So they get ZERO inprocessing while kissat
   interleaves substitute/vivify/congruence/sweep continuously (60–111
   substitute rounds, vivify to round ~40). Switch the cadence trigger to a
   tick/time budget (kissat-style) so big formulas get simplified at all.
   Bead SAT-playground-5b2.3.50 (global-effort cadence redesign) is exactly
   this — promote it up the queue.
3. **Deepen gate-aware elimination + equivalence substitution.** kissat
   reaches 72–88% elim on circuit cells vs solver12's 43–56%; kissat
   substitutes 10k–3.5M vars (pj2008 = 3.52M!) where solver12's congruence
   finds ~0 merges on the miters (Bubble/booth). Bead SAT-playground-5b2.3.39
   (congruence, in progress) feeds this. NOTE this REFRAMES the prior
   "pj2008 = trajectory not rate" conclusion: pj2008's kissat gap is
   substantially INPROCESSING (54% elim + 3.5M subst + 18M kitten) that
   solver12 (28% elim, 0 sweep/subst) simply does not do.
4. **Tiered vivification port (was plan-21 #4).** kissat runs
   tier1/tier2/tier3/irredundant vivify every round; solver12 is single
   LBD≤6 + 6M-delay cursor. Measure per-tier effort/success first; scope
   after (rerolls!). Feeds the Bubble/booth circuit-UNSAT cluster.
5. **Land failed-literal probing + HBR.** `SAT_PROBE`/`SAT_HBR` are
   `ParkingLot` (NOT implemented); kissat probes on every cell. New
   capability, not a tuning knob.
6. **Search-heuristic gap on preprocessing-immune cells (small).** fixedbw
   (149 vars): kissat refutes at 12.1M conflicts, solver12 flails past ~50M.
   Not inprocessing — branch/restart/phase quality. Lower priority; only
   ~1–2 cells and no easy port.
7. **Giant memory diet (still open, secondary).** RSS 10.4GB vs kissat 1.4GB
   on pj2008; clause_abstraction/binary_id/occurs hogs. Trajectory-safe;
   matters for the 16GB cap and locality — but item #1/#3 are the real pj2008
   levers now.

## solver12's OWN capability edge (protect in rerolls)

Confirmed uncontended (kissat CANNOT solve any in 600 s; solver12 solves fast):
`xor_op_n36_d3` 1.5 s, `xor_op_n40_d3` 2.2 s (Gauss `SAT_GAUSS` + parity
`SAT_PAIR_ABS_REFUTE`), `oddball_80_5` 272 s, `Kakuro-easy-132` 280 s,
`MVRoundRobin_n16_d10` 181 s. These are genuine algebraic/parity wins kissat's
default lacks. Do NOT trade them away in throughput rerolls.

## Reroll-luck law (inherited, still in force)

The 69/68-lineage embeds banked wall-lottery luck; any global trajectory
reroll is −EV even with real throughput wins. Trajectory-identical diets
(9-for-9) and timeout-only-scoped changes are the promotable shapes. UNSAT
statuses never flip on rerolls; SAT-lottery and near-budget wall cells do.
Load ALONE flips oski15a01b20/rbsat/sted2-class cells between gates with
identical binaries. **Corollary from this session**: the capability items
above (sweep, cadence, elim depth) are formula-shrinking, so they are the
RIGHT kind of change — they can help without a global reroll IF scoped so
already-solved cells stay byte-identical (validate with the usual
trajectory-identical gate).

## Standing traps (carried forward, all still in force)

- `results.tsv`/seedgate TSV written only at run END — monitor progress from
  the launch log per-cell lines; completion from "DONE ->".
- SAT_STATS_JSON needs `=on` (`=1` silently parses false). `SAT_LIMIT_WALL_SEC`
  gives a clean stats-emitting stop for window measurements.
- When byte-comparing stats strip `*_sec`, `seconds_stable/focused`,
  `max_rss_mb`, `phase_capture_entries`.
- 32-way at 16GB/job = 512GB > 90% of 502GB RAM → preflight warns; it's a cap
  not a reservation and prior runs are fine, but watch for OOM on giant waves.
- kissat memory via `ulimit -v` (address space) did NOT strangle it — all 32
  unsolved were exit 124 (real timeout). `tools/run_kissat_medium.sh`
  replicates the gate conditions for kissat.
- perf unusable (perf_event_paranoid=4); decompose with in-tree env timers.
- dcg blocks absolute-path shell redirects into $HOME — repo-relative paths.
- No `cargo build --release` while a gate runs; kill watcher tasks before
  check_promotion_gate; TSV TIMEOUT rows carry zero conflicts; kissat -s/-q
  exclusive; heredoc-to-scratchpad writes flaked this session (use the Write
  tool for scratch scripts).

## Tooling + instrumentation added this session (reusable)

- `tools/run_kissat_medium.sh` — 32-way kissat sweep at gate conditions
  (`ulimit -v` 16GB + timeout 1800), results.csv schema-compatible with prior
  `log/kissat-medium-*`.
- `tools/gap_read.py` — solver TSV vs kissat CSV lexicographic gap report
  (solved, exclusive cells, PAR-2, correctness cross-check).
- `log/gap-read-2026-07-21/` — per_cell_comparison.csv + `deepdive/`
  (kissat `-v` traces, solver12 JSON-stats, COMPARISON.txt).
- Method: `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=N` + kissat `-v` on identical
  CNF is the capability-vs-trajectory decomposition harness. The discriminator
  is: does the winner solve with FEWER conflicts via formula-shrinking
  (elim/sweep/subst)? If yes → capability, not reroll.

## Where the evidence lives

- This session: `plan/gap-read-2026-07-21.md` (headline + capcheck + full deep
  dive), `log/gap-read-2026-07-21/deepdive/COMPARISON.txt`.
- Fresh 32-way reads: solver12
  `log/seedgate-default-2026-07-21-11-37-05/results.tsv`; kissat
  `log/kissat-medium-20260721-130444/results.csv`; capcheck
  `log/gapread-kissat-capcheck.log`.
- Prior arc: `plan/next-steps-AGGREGATED-2026-07-21b.md` (superseded; its
  phasedelta chain + ledgers/traps remain valid provenance),
  `plan/next-steps-phasedelta-2026-07-21.md`.
- Beads: SAT-playground-5b2.3.50 (cadence redesign — now plan #2),
  SAT-playground-5b2.3.39 (congruence — feeds plan #3),
  SAT-playground-pow (phasedelta, done).
