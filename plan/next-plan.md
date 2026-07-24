# NEXT PLAN — 2026-07-24 (supersedes next-steps-AGGREGATED-2026-07-23b.md)

One-file plan for the next clear context. Folds the 2026-07-24 **3600 s / 16 GB
solver12-vs-kissat medium gap read** on top of the 2026-07-23b aggregate
(banded endgame-delta promotion). Where this contradicts an older
`plan/next-steps-*.md`, THIS file wins.

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

## RANKED PLAN for next session

1. **10th wall-diet — now with a deterministic +1 attached (carried #1,
   strengthened).** bp4_TCO_CSO_ZR solves at 1880 s with a kissat-impossible
   trajectory: **~5% wall is a guaranteed, capability-backed +1 at the
   1800 s gate** — no lottery, no reroll (conflicts 2,008,325 deterministic).
   The same diet hardens rbsat (20 s under the wire this deal!), sted2, vex,
   oski15. Proven gate-safe shape: conflicts EXACT tie, wall down
   (watch-pool / closure-diet / round-diet / elim-scratch / hotloop-ptr /
   fastidx / extract-cache / phase-delta / endgame lineage). Profile the
   1600–1900 s five first; pick the fattest shared sink.
2. **Throughput / learned-DB discipline (carried #3, new evidence).** The
   2.2–9x wall ratios above at identical outcomes; Bubble's 9x (20.1 M
   conflicts at ~7 k conf/s) is the new extreme datapoint. Lever = reduce
   policy / kept-clause counts -> shorter watch lists (kissat tier limits).
   Measure OFFLINE first (kept clauses + ticks/prop on rbsat/Bubble under
   kissat-style limits, SAT_LIMIT_CONFLICTS identity screens — no gate).
   WARNING: rerolls every >=1M-conflict trajectory — bundle as a deliberate
   re-luck campaign (REROLL-LUCK LAW), scope by arming time where possible.
3. **Inprocessing capability arc (carried #4/#6, targets now precise).**
   Four cells kissat closes in <=1400 s that 2x time does NOT give us:
   fixedbandwidth, goldcrest, bp4_TCO_IXA_LP, booth_dadda_mapped. The
   2026-07-21 deep dive (still current, `log/gap-read-2026-07-21/deepdive/`)
   ranks the mechanisms: (a) kitten-class SAT-sweeping productivity (ours
   finds 0–826 equivalences where kissat kitten-solves 90 k–18 M; bead
   SAT-playground-5b2.3.39: sweep_round restarts its 512-seed scan at var 1
   every round); (b) elimination depth 72–88% vs our 43–56% on miters +
   equivalence substitution; (c) **time/tick-budgeted inprocessing cadence**
   — goldcrest (474 conf/s) and lockchart (330 conf/s) NEVER reach the 1M
   conflict trigger in a full run. Must spare sudoku-N30 + bp5.
4. **Giant memory diet (carried #5).** pj2008 RSS 10.4 GB vs kissat 1.4 GB;
   BVE emits 1.7 GB discarded DRAT in 150 s. Note pj2008 is marginal even
   for kissat (2866 s at 3600). Unstarted.
5. **TT class bookkeeping.** TT496 banked and re-confirmed unique at 3600 s.
   TT492: kissat needs 2222 s — NOT a 1800 s-gate loss; our old draw existed
   only pre-rf (closed). TT495: nobody solves at 3600 s — needs a genuinely
   new mechanism; low priority standalone.
6. **Carried kissat gaps (unstarted):** tiered vivification, probing/HBR
   parity (SAT_PROBE/SAT_HBR still ParkingLot).

## Current state

- HEAD: b671ae0 (banded-delta promotion). **Medium 1800 s baseline: 70/100**;
  lineage TSV `log/abtest-cand-vs-base-2026-07-23-21-23-54/cand/results.tsv`.
  At 3600 s: 73/100 (this session; solver12 verify clean).
- Endgame surface: SAT_ENDGAME (on), TRIGGER 1, PARTS "rf", MIN_ARMED 100k,
  banded REPHASE_DELTA (decision-armed 48k / yield-armed legacy 50k),
  DELTA_SPLIT 500k.
- Decision metric UNCHANGED: lexicographic solved -> conflicts -> PAR-2 on
  the medium suite at 1800 s, 16 GB, 32 pinned cores. The 3600 s numbers are
  analysis-only — do NOT promote on them.

## Standing traps (carried + this session)

- `results.tsv` written only at run END — monitor per-cell lines in launch
  logs instead.
- `pgrep -f feature_ablation` inside a monitor loop matches ITSELF; use
  `ps aux | grep "[f]eature_ablation.py"`.
- vex UNSAT checker-timeout is historical/symmetric load-lottery, NOT a gate
  failure (verify_fail=0 again this session at 3600 s).
- Conflict counts are EXACTLY deterministic across load; wall is not.
  Digit-exact identity checks (yield-protect + passthrough + default-equiv)
  for every scoped-reroll change.
- Wall-coin cells at the 1800 s gate, updated: **rbsat-v1375 (1780 s),
  bp4_TCO_CSO_ZR (1880 s — just OUT of gate), sted2 (1667 s), vex (1657 s),
  oski15 (1615 s)**. Tier-1 margins under ~120 s are load noise.
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

## solver12's capability edge (protect in rerolls)

xor_op x2, tseitin_n188_d3 (SAT_TSEITIN), oddball_80_5, MVRoundRobin_n16_d10,
SC25_Timetable_C_406 (endgame rf), SC25_Timetable_C_496 (banded d48k, 1076 s
— kissat cannot at 3600 s), **bp4_TCO_CSO_ZR (new: kissat cannot at 3600 s;
ours at 1880 s, 80 s from the gate line)**. Kakuro-easy-132 + case1 are now
speed wins (12x/7x), no longer unique-capability — still gate +1s at 1800 s.

## Where the evidence lives

- This session: result files above; sweep driver pattern in
  `plan/next-plan.md` history; runs were sequential on the idle host.
- Mechanism deep dive (still the reference): `plan/gap-read-2026-07-21.md`,
  `log/gap-read-2026-07-21/deepdive/COMPARISON.txt`.
- Prior aggregates: `plan/next-steps-AGGREGATED-2026-07-23b.md` (and the
  chain below it).
