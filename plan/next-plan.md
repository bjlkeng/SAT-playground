# NEXT PLAN — 2026-07-24 (supersedes next-steps-AGGREGATED-2026-07-23b.md)

One-file plan for the next clear context. Folds the 2026-07-24 **3600 s / 16 GB
solver12-vs-kissat medium gap read** plus the same-day **source audit and
three-gate depth campaign** on top of the 2026-07-23b aggregate (banded
endgame-delta promotion). Where this contradicts an older
`plan/next-steps-*.md`, THIS file wins.

**START HERE:** read "SESSION 2" then the RANKED PLAN. Item 1 (scoped
gate-aware BVE) has a measured discriminator and a projected gate WIN
(69 -> 72); it is the highest-value open item in the project. Companion
deep-dive: `plan/kissat-gaps.md`.

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

## RANKED PLAN for next session

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

- HEAD: b671ae0 (banded-delta promotion). **Medium 1800 s baseline: 70/100**;
  lineage TSV `log/abtest-cand-vs-base-2026-07-23-21-23-54/cand/results.tsv`.
  At 3600 s: 73/100 (this session; solver12 verify clean).
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
  bp4_TCO_CSO_ZR (1880 s — just OUT of gate), sted2 (1667-1791 s), vex
  (1476-1664 s), oski15 (1597-1657 s), VanDerWaerden_pd_2-3-22 (1718 s)**.
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

## solver12's capability edge (protect in rerolls)

**NEW 2026-07-24 (gate-BVE, needs scoping to bank):** `RoundRobin_n16_d13`
UNSAT 80.7 s — nobody solves it, kissat included, even at 3600 s;
`bp4_TCO_CSO_IXA_LP_ZR` SAT 238.9 s (kissat 1187 s).

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
