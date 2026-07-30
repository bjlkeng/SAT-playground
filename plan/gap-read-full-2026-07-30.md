# FULL-BENCH GAP READ — solver12 vs kissat 4.0.4, sat-comp-2025 (400 instances, 3600 s, 16 GB)

First-ever full-competition-bench comparison (2026-07-29/30). Prior gap
reads covered only the 100-cell medium suite; 300 of these cells had
never been run by solver12 at any wall.

## Setup

- solver12 @ HEAD c469b03 (defaults, 74/100 medium lineage), via
  `feature_ablation.py --seedgate --suite sat-comp-2025 --timeout 3600
  --mem-mb 16000 --jobs 18`, proofs REQUIRED and verified (drat-trim,
  2x-wall checker budget; SAT models verified). Cores 0-17 (socket 0).
- kissat 4.0.4 (kissat-latest) via new `tools/run_kissat_full.sh`
  (parallel runner generalized from run_kissat_medium.sh with `-d`
  suite / `-c` core-offset), no proofs. 14 jobs, cores 18-31 (socket 1).
- Concurrent, disjoint sockets, 3600 s / 16 GB (`ulimit -v`) per cell,
  ~11.4 h wall. Both arms share host load symmetrically; wall times on
  marginal cells carry the usual contention caveat, conflicts do not.
- Result files:
  - solver12 TSV: `log/seedgate-solver12-full3600-2026-07-29-21-07-58/results.tsv`
  - kissat CSV: `log/kissat-full-20260729-210758/results.csv`
  - analysis artifacts: `log/gap-read-full-2026-07-30/` (gap_read.txt,
    analyze.py, analysis.txt)

## Headline

| solver (3600 s, 400 cells) | solved | SAT | UNSAT | PAR-2 |
|---|:--:|:--:|:--:|--:|
| solver12 @ c469b03 | **261** | 133 | 128 | 1,136,656 |
| kissat 4.0.4       | **296** | 150 | 146 |   923,215 |

- **Gap: kissat +35.** Zero SAT/UNSAT contradictions on 232 both-solved
  cells; solver12 verify: 257 ok, 0 FAIL, 4 checker-timeout, 139 skip
  (timeouts). Calibration: kissat-sc2024 won SC25 at 306/400 under
  5000 s / 30 GB; kissat-latest posts 296 at 3600 s / 16 GB here.
- Exclusive cells: kissat-only 64, solver12-only 29, both-timeout 75.
- **Medium-100 inside this run: solver12 WINS 76 v 75** (was 73 v 75 on
  2026-07-24 pre-scoped-BVE/php). **The entire +35 lives in the 300
  out-of-sample cells: 185 v 221 (−36).**

## Truncation curve (same-deal virtual cutoffs, full 400)

| cutoff | s12 | kissat | delta |
|--:|--:|--:|:--:|
| 300 s | 154 | 166 | +12 |
| 600 s | 195 | 209 | +14 |
| 1200 s | 225 | 242 | +17 |
| **1800 s** | **239** | **266** | **+27** |
| 2400 s | 249 | 277 | +28 |
| 3000 s | 257 | 287 | +30 |
| 3600 s | 261 | 296 | +35 |

Unlike the medium suite (where s12 led every cutoff through 2400 s),
kissat leads at EVERY cutoff on the full bench. This is NOT a tail-only
phenomenon: 23 of the 64 kissat-only cells solve in <=600 s. The medium
suite over-represents solver12's engineered strengths — 8+ sessions of
scoping decisions were fit to exactly those 100 cells.

## Family decomposition

**kissat-only (64):** multiplier-miter-16x16 x10 (+4 more both-timeout;
kissat 580-3162 s), misc x12 (battleship 21 s, uniqinv40prop 51 s,
gto 62 s, shuffling-1 109 s, case8 217 s, contest04 284 s,
sted2_0x0_n219-342 303 s, myciel6 1868 s, SGI 3080 s ...), oddball-ttf
x5, crypto-arith x4 (mod2c, bivium, dislog, mod4block), hwmcc-bmc x4
(nla-digbench 523 s, x-epic 577 s, goldcrest 1173 s, g2-oski15a10b10s
1680 s), bitvector-bp x4, circuit-multiplier x3 (Circuit_multiplier24
174 s SAT, lec_mult 512 s, Circuit_multiplier29 3382 s), pj-giants x2
(pj2016_k100 = OUR MEMORY ABORT, kissat solves SAT 1568 s; pj2008_k200
1157 s), grs x2, sorting-networks x2 (BubbleVsPancakeSort_7_6 368 s —
we solved it 2880 s on 07-24, lost to rerolls; _8_4 1967 s), itc x2,
fsf x2, + singletons (rook-51 1904 s, b18 360 s, b19_1 820 s, VdW-23
1197 s, TT_C492 967 s SAT, lockchart-group1-L190 2816 s, sqrt-mitern169,
HCP-446, ER_400, SAT_dat.k100, reconf10_22, ncc_none_2_18, battleship,
mp1-blockpuzzle).

**solver12-only (29):** roundrobin x8 (RoundRobin n15-n17 d13/d14 +
MVRoundRobin x3 — the gate-BVE capability class GENERALIZED),
cliquecoloring x7 (SAT_PHP_REFUTE fired on 5 UNSEEN family members:
clqcl_100_6_5, clqcl_30_7_6, cliquecoloring_n26_k7_c6, n32_k5_c4,
n15_k7_c6 — all <2 s), oddball-tto_zp SAT x4 (267-572 s; kissat
times out on all), xor_op x2 (SAT_GAUSS; kissat 3600 s on n38!), rphp
x2, tseitin_n188 (44 s), Kakuro-132, TT_C496 (1130 s), HCP-529 (62 s),
frb80-14-1 (SAT 3325 s), valves-gates-1-k617 (UNSAT 3400 s,
checker-timeout).

**The promoted special-refutation + arming portfolio is worth ~29
unique cells out-of-sample (13 on medium)** — it is real capability,
not suite overfit. Detection generalized to never-seen instances with
zero false fires and zero proof failures.

## Mechanism probes (300 s stats probes + kissat -s, 2026-07-30)

| cell (kissat time) | kissat mechanism | solver12 state |
|---|---|---|
| bv_ILA_Piccolo_BEQ (8 s!) | substituted 35% of vars + 22% units via probing, 8.4k conflicts total | 9.07M conflicts, 994 s (root ELS off, probe adopter-scoped) |
| n320p5q2_n.apx_16 (16 s) | round BVE 272% cumulative + 15 probing rounds | 1.39M conflicts, 1086 s |
| 170223547 (4.4 s) | NO preprocessing — pure search, 29.6k conf/s | 7.39M conflicts @ 13k conf/s, 567 s |
| uniqinv40prop (51 s) | sweep_equivalences 3799 = 30% of vars, substituted 33% | sweep finds 361 (learned-binary shape, never substituted), 4.3k conf/s |
| mp1-blockpuzzle (25 s SAT) | substituted 46% (ELS) | sweep_eq 31,253 + els 3426 but 3.5k conf/s, timeout |
| oddball_13_5_ttf (26 s) | 49.5k conf/s, walk 18M steps, 1.29M conflicts total | 10.4k conf/s, 3.1M conflicts in 300 s, no walk — rate AND quality gap |
| boothbit29 miter (533 s) | eliminated 74%, 11.6k conf/s sustained over 6.2M conflicts, walk 359M | similar elim %, 7.6k conf/s, >20M conflicts without solving |
| Circuit_multiplier24 (174 s SAT) | walk_steps 155M | walk_flips 3M in 300 s |
| sted2_0x0_n219-342 (303 s SAT) | eliminated 49%, walk 100M steps | gate-elim 8221 but 2.7k conf/s, zero walk in 300 s |
| battleship-13-13 (21 s) | 31.5k conf/s + walk 31M | STUCK: no proof growth for 8+ min inside solve, wall-limit not honored (see anomalies) |

## The three gap mechanisms (out-of-sample)

1. **Scoping over-fit (root/round inprocessing).** kissat runs
   ELS-substitution, probing, sweep, round-BVE ALWAYS-ON everywhere;
   solver12's equivalents exist but are adopter-scoped/thresholded/off
   because every scope was chosen to protect medium-100 coins
   (SESSIONS 6-10). Out-of-sample, instant-collapse cells (bv_ILA x2,
   n320p5q2, uniqinv40, blockpuzzle, likely several more of misc) go
   to kissat ~for free.
2. **Long-horizon trajectory quality (the REDUCE-law / tick-cadence
   bundle).** Multiplier miters (14 cells), oddball-ttf (5), sorting
   networks: kissat sustains 11-50k conf/s and needs 2-10x fewer
   conflicts at horizon >1M conflicts; solver12's DB grows (3-step used
   counter, literal-budget reduce) and rates sag. This is exactly the
   ">3000 s bundle" measured real-but-unpromotable at 1800 s medium
   (SESSIONS 5/9/10/12).
3. **Walk/rephase SAT capability.** kissat walks 100-360M steps on the
   SAT wins (circuit-multiplier, sted2var, ITC, HCP-446); solver12's
   walk is armed-scoped and fires 10-50x fewer flips. (Counter-view:
   OUR oddball-tto_zp x4 + TT496 wins come from the endgame/banded
   rephase — the classes split by structure.)

## Anomalies and correctness surface

- **ZERO proof/model failures across 261 solved cells.** vex-class
  checker-timeouts x4 (valves-gates 3400 s, ncc_none_21015 2950 s [kissat
  472 s], grs-160-48 2046 s [kissat 1161 s], VexRiscv-regch0-20-p1
  1635 s [kissat 512 s]) — proofs too big for the 7200 s drat-trim
  budget; in a proof-required competition these 4 solves are at risk.
- **pj2016_k100 (8.8M vars / 23M clauses): solver12 rc-6 abort at 53 s —
  virtual footprint crosses 16 GB `ulimit -v` during parse/setup; RSS
  12.7 GB at 150 s unlimited. kissat solves it SAT in 1568 s in-budget.
  A concrete +1 behind the giant memory diet.**
- pj2002_k500 (2.6 GB text): the mirror image — kissat OOM-aborts at
  122 s (exit 134), solver12 parses fine and runs the full 3600 s. The
  giant-arena parse diet already beats kissat here; the deficit is
  peak SEARCH-phase memory, not parse.
- 17.normalised (7.1 GB text): both solvers OOM-abort. Unreachable at
  16 GB; only relevant under 30 GB competition limits.
- **tseitin_d3_n100000 (8.7 MB formula!) — ROOT CAUSE FOUND (gdb
  backtrace, `log/gap-read-full-2026-07-30/gauss_spin_backtrace.log`):
  `SAT_GAUSS` pre-search refutation. `gauss::min_degree_order`
  (gauss.rs:474, HashMap-entry churn) spins ~25 min on the 100k-equation
  XOR system with NO wall/limit checks, then the elimination fill-in
  allocates unboundedly — 31.4 GB RSS measured before kill; the bench
  run's rc-6 at 1711 s was this crossing 16 GB virtual. The cell got
  ZERO search time.** Likely also taxes tseitin_grid_n250/n400 and any
  large XOR-heavy timeout cell. kissat also cannot solve it, so the fix
  is a bug fix (size cap + tick budget on the gauss path), not a +1.
- **battleship-13-13-unsat — ROOT CAUSE FOUND (gdb backtrace,
  `log/gap-read-full-2026-07-30/sweep_kitten_spin_backtrace.log`):
  mid-search `sweep_round` → `sweep::prove_facts` (the
  unlimited-budget wrapper, sweep.rs:256) → `Kitten::solve_budgeted`
  stuck in `propagate` — one kitten sub-solve with no effective budget
  consumed the cell's entire 3600 s. Deterministic (proof frozen at
  byte-identical 150,995,327 in two idle runs). kissat solves the cell
  in 21 s — bounding the legacy sweep kitten call may make it
  winnable.** SAT_LIMIT_WALL_SEC is only checked in the CDCL loop, so
  both this and the gauss spin also explain the wall-cap misses.
- rc-6 aborts print no `s UNKNOWN` line (allocator abort). Cosmetic
  under competition scoring, but the harness records them as UNKNOWN_rc-6.

## Regressions vs the 2026-07-24 medium 3600 s run (same 100 cells)

Net +3 (73 → 76): gained rphp x2 + clqcl x2 (SAT_PHP_REFUTE),
RoundRobin_n16_d13 + bp4_TCO_CSO_IXA_LP_ZR (scoped gate-BVE); lost
bp4_TCO_CSO_ZR (documented SESSION-4 casualty), BubbleVsPancakeSort_7_6
(was 2880 s marginal; kissat does it in 368 s), and
**bp4_BC012_CSO_FPBEQ_FPBLE_ZR (was SAT 205 s → now 3600 s timeout — an
undocumented reroll casualty of the post-07-24 promotion chain; its
sibling bp4_BC012_CSO_AM_FPBEQ_FPBLE_ZR is kissat-only at 2537 s).**

## php-detector near-misses (both-timeout, structure-adjacent)

cliquecoloring_n14_k7_c6, clqcl_30_9_8, clqcl_30_11_10 declined
detection while five family siblings fired; harder-fphp-016-015
(direct pigeonhole, sat05 shuffle) and rphp_p25_r25 also sit in the
both-timeout core. Decoding WHY each declines is the cheapest
capability lead in the whole read (the proven SESSION-11 shape).
