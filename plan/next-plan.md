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

## RANKED PLAN for next session

**Ranking CHANGED from the 2026-07-23b aggregate.** The wall-diet/lottery
vein is near-exhausted (9 consecutive diets, 67->70 over ~20 sessions, last
wins are coin flips); the source audit shows items 1-3 below are
deterministic, mechanism-backed work with named target cells.

1. **Fix the sweep seed cursor (NEW #1 — bug, not research).** Advance the
   scan monotonically across rounds, add kissat's leftovers-first +
   occurrence-sorted persistent schedule and the completion/escalation
   ladder, and stop cloning the clause DB per round. Bead
   SAT-playground-5b2.3.39. 450x measured productivity gap behind it;
   touches all nine kissat-only cells. See delta section 2.
2. **Re-denominate the inprocessing budget in ticks** with an effort floor
   (kissat `SET_EFFORT_LIMIT`). Contained change; kills the entire
   "never fires" class. Named beneficiaries: goldcrest, lockchart-group1.
   Must spare sudoku-N30 + bp5 (never-armed but solving). See section 1.
3. **Evaluate what is already built but off** — `SAT_PROBE`,
   `SAT_GATE_BVE`+`SAT_GATE_EXTRACT`, `SAT_ELIM_DEF`. These are GATE RUNS,
   not development. Gate-aware BVE + kitten definitions is exactly how
   kissat reaches 72-88% elimination where we sit at 43-56%. Target cells:
   fixedbandwidth, goldcrest, bp4_TCO_IXA_LP, booth_dadda_mapped (the four
   kissat closes in <=1400 s that 2x time does NOT give us). See section 5.
4. **Small ports:** `backbone.c` (BIG failed literals, 2% effort),
   `transitive.c` (2% effort), vivify tier3 + the 3:3:1:3 budget split.
   See sections 6-7.
5. **Reduce control law (highest ceiling, highest risk).** Fraction-ramp +
   31-step `used` counter vs our literal-budget + 3-step. Best hypothesis
   for the 2.2-9x throughput gap. Measure OFFLINE first (kept clauses +
   ticks/prop on rbsat/Bubble under kissat-style limits,
   SAT_LIMIT_CONFLICTS identity screens — no gate). WARNING: rerolls every
   >=1M-conflict trajectory — needs a deliberate re-luck campaign
   (REROLL-LUCK LAW), not a single gate run. See section 4.
6. **10th wall-diet (DEMOTED from #1, still has a free +1).**
   bp4_TCO_CSO_ZR solves at 1880 s with a kissat-impossible trajectory, so
   **~5% wall is a deterministic capability-backed +1 at the 1800 s gate**
   (conflicts 2,008,325, no reroll). Same diet hardens rbsat (20 s under
   the wire this deal!), sted2, vex, oski15 — five cells in the 1600-1900 s
   band. Proven gate-safe shape (conflicts EXACT tie, wall down). Keep as
   the cheap fallback when items 1-3 stall; do not lead with it.
7. **Giant memory diet (carried).** pj2008 RSS 10.4 GB vs kissat 1.4 GB;
   BVE emits 1.7 GB discarded DRAT in 150 s. Note pj2008 is marginal even
   for kissat (2866 s at 3600 s). Unstarted.
8. **TT class bookkeeping.** TT496 banked and re-confirmed unique at
   3600 s. TT492: kissat needs 2222 s — NOT a 1800 s-gate loss; our old
   draw existed only pre-rf (closed). TT495: nobody solves at 3600 s —
   needs a genuinely new mechanism; low priority standalone.

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
