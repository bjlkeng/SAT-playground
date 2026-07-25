# solver12 vs kissat 4.0.4 — full gap analysis (2026-07-24)

Standalone reference doc. Combines a fresh paired 3600 s benchmark with a
**source-level audit of both codebases** (solver12: 46k lines Rust; kissat
4.0.4: 39k lines C). Written to be the durable answer to "where exactly are
we behind kissat, is it code or configuration, and is more iteration worth
it."

Companion files: `plan/next-plan.md` (ranked next steps, folds this in),
`plan/gap-read-2026-07-21.md` (prior mechanism deep dive, still valid on
elimination %/kitten productivity numbers).

---

# PART 1 — MEASUREMENT

## 1.1 Run conditions

Both solvers over the full 100-instance `benchmarks/sat-comp-2025-medium`
suite at **3600 s timeout, 16 GB/job (`ulimit -v`), 32 pinned physical
cores**, run **sequentially** so each had the otherwise-idle 72-core host to
itself (no cross-solver contention). Same methodology as the 2026-07-21 gap
read, so the numbers chain back to prior sessions.

- solver12 @ HEAD `b671ae0`, via
  `feature_ablation.py --seedgate --configs default --suite sat-comp-2025-medium --seeds 1 --timeout 3600 --mem-mb 16000 --jobs 32`
  (verification ON).
- kissat 4.0.4 via `tools/run_kissat_medium.sh -t 3600 -m 16000 -j 32`.

Result files:

- solver12 TSV: `log/seedgate-default-2026-07-24-07-24-29/results.tsv`
- kissat CSV:   `log/kissat-medium-20260724-102838/results.csv`
- per-cell join (both solvers x both timeouts):
  `log/gap-read-2026-07-24/per_cell_comparison.csv`
- report tool: `python3 tools/gap_read.py --solver <tsv> --kissat <csv> --timeout 3600`

## 1.2 Headline

| solver (3600 s)     | solved | SAT | UNSAT | PAR-2      |
|---------------------|:------:|:---:|:-----:|-----------:|
| solver12 @ b671ae0  | **73** | 41  | 32    | **225 298** |
| kissat 4.0.4        | **75** | 42  | 33    | 226 904     |

kissat +2 solved; solver12 better PAR-2 by 1606. **Correctness clean**: zero
SAT/UNSAT contradictions across all 66 both-solved cells, `verify_fail = 0`
on the solver12 side (DRAT proofs checked in-gate).

## 1.3 THE decision number — same-deal truncation curve

Truncating **this one 3600 s deal** at virtual cutoffs. Because it is a
single run, there is zero deal-to-deal wall lottery — this is the cleanest
solver12-vs-kissat comparison the project has produced.

| cutoff | solver12 | kissat | delta |
|-------:|---------:|-------:|:-----:|
|  300 s | 45 | 39 | −6 |
|  600 s | 55 | 54 | −1 |
|  900 s | 59 | 57 | −2 |
| 1200 s | 66 | 60 | −6 |
| **1800 s** | **71** | **67** | **−4** |
| 2400 s | 72 | 70 | −2 |
| 3000 s | 73 | 73 | 0 |
| 3600 s | 73 | 75 | +2 |

**solver12 leads at every cutoff through 2400 s and only crosses over at
~3000 s.** We are not "behind kissat" — we are a *different shape*: faster
to close what we can close, thinner in the long tail.

This reframes every strategy question. At our own 1800 s promotion gate we
already win by 4. The deficit is purely a >3000 s tail phenomenon, so it
only matters if the target is competition-realistic (5000 s) scoring.

## 1.4 What the extra hour bought each solver

solver12 **+3** over the 70/100 1800 s lineage (nothing lost):

| cell | result | time | note |
|------|--------|-----:|------|
| bp4_TCO_CSO_ZR | SAT | 1880 s | 2 008 325 conflicts, deterministic |
| BubbleVsPancakeSort_7_6 | UNSAT | 2880 s | **FIRST-EVER solver12 solve**, 20.1M conflicts |
| rbsat-v1375 | SAT | 1780 s | the documented wall coin, landed IN this deal |

kissat **+7** over its 68/100 1800 s run: Kakuro-132 (3259 s), case1
(1917 s), VanDerWaerden-22 (2565 s), TT492 (2222 s), booth_wallace
(3131 s), booth_dadda_origin (1864 s), pj2008 (2866 s).

**kissat's marginal tail is roughly twice as fat as ours.** That is the
mechanism behind the crossover in 1.3, and it is why capability work beats
cadence-coin tuning if the horizon is long timeouts.

## 1.5 Capability map at 3600 s

Doubling the timeout is a clean discriminator: a cell that stays unsolved at
2x time is a capability wall, not a timeout artifact.

**solver12-only (7)** — kissat times out at 3600 s:

| cell | time | mechanism |
|------|-----:|-----------|
| xor_op_n36_d3 | 1.4 s | SAT_GAUSS + SAT_PAIR_ABS_REFUTE |
| xor_op_n40_d3 | 2.0 s | SAT_GAUSS + SAT_PAIR_ABS_REFUTE |
| tseitin_n188_d3 | 43.7 s | SAT_TSEITIN (extended resolution) |
| MVRoundRobin_n16_d10 | 172.9 s | — |
| oddball_80_5 | 253.9 s | — |
| SC25_Timetable_C_496 | 1076.0 s | SAT_ENDGAME banded d48k |
| **bp4_TCO_CSO_ZR** | **1880.3 s** | **NEW this session** |

Dropped OFF the unique list: Kakuro-easy-132 and case1 — kissat gets both
with 2x time (3259 s / 1917 s), though we remain 12x / 7x faster. They are
still gate +1s at 1800 s, just not capability wins.

**kissat-only (9)**, split by what they actually need:

*Pure capability gap — kissat ≤1400 s, we are dead even at 3600 s. These
four are the sharpest inprocessing targets:*

| cell | kissat |
|------|-------:|
| fixedbandwidth-eq-37 | 1153 s |
| goldcrest-and-14 | 1185 s |
| bp4_TCO_CSO_IXA_LP_ZR | 1187 s |
| booth_dadda_mapped | 1372 s |

*kissat itself needed >1800 s (not 1800 s-gate losses, same mechanism
class):* booth_dadda_origin 1864 s, TT492 2222 s, lockchart-group1 2770 s,
pj2008 2866 s, booth_wallace 3131 s.

**Both-timeout hard core (18):** TT495 (**nobody solves it, even at
3600 s**), TT7F-33-24B, ramsey_3_6_19, ramsey_4_4_18, clqcl_40_6_5,
clqcl_50_6_5, rphp5_050, rphp5_085, VanDerWaerden-27, RoundRobin_n16_d13,
lockchart-group3, rbsat-v945, g2-hwmcc15deep-oski15a10b10s-k20,
bp4_LPI_FPBEQ_ZR, st_659_37_25_686, oisc-subrv-and-nested-11, stp212,
tseitin_grid_n400 (arc CLOSED — do not revisit).

## 1.6 Throughput on identical-outcome cells

kissat faster on 41 of 66 both-solved cells (total 27 468 s vs 29 154 s —
close in aggregate, but the dense band is lopsided):

| cell | solver12 | kissat | ratio | s12 conf/s | s12 props/s |
|------|---------:|-------:|:-----:|-----------:|------------:|
| BubbleVsPancakeSort | 2880 s | 319 s | **9.0x** | 6 995 | 1.34M |
| sted2 | 1667 s | 468 s | 3.6x | 2 640 | 0.66M |
| rbsat-v1375 | 1780 s | 569 s | 3.1x | 3 516 | 0.64M |
| oski15a01b20s | 1615 s | 574 s | 2.8x | 1 649 | 1.01M |
| vex (VexRiscv) | 1657 s | 755 s | 2.2x | 1 795 | 2.32M |

**FIVE solver12 cells now sit in the 1600–1900 s band** of the 1800 s gate
(bp4_TCO_CSO_ZR 1880, rbsat 1780, sted2 1667, vex 1657, oski15 1615). Each
is an exactly-deterministic trajectory whose solve/no-solve at the gate is a
pure wall coin.

Note propagation throughput is NOT the problem — see 2.4.

---

# PART 2 — IMPLEMENTATION DELTA (source-audited 2026-07-24)

**Headline finding: the remaining gap is mostly configuration plus one
scheduling bug — NOT missing algorithms.** Much more is implemented in
solver12 than the project docs claim.

## 2.0 DOC WARNING — read source, never FEATURES.md

`FEATURES.md`, `FEATURES.csv`, and `CONFIG_SCHEMA.csv` are **stale**.
Authoritative defaults live in:

- `src/config.rs` — `impl Default for SolverConfig` (~L705-840) and
  `apply_profile_defaults` (L895-1094)
- `src/main.rs` — raw `env_bool_or_default("SAT_*", …)` reads (~L3600-3970)

Known doc errors:

- `SAT_CONGRUENCE`, `SAT_CONGRUENCE_XOR`, `SAT_FACTOR` are **default-ON** in
  source, documented as Experimental / default-off.
- `SAT_TSEITIN`, `SAT_ENDGAME`, `SAT_SWEEP`, `SAT_WALK`, `SAT_ELIM_*` and
  the entire arming layer appear in **no** doc file.
- **`SAT_PROBE` is implemented and works** — it is NOT in the runtime
  rejection list. Prior plans calling it "ParkingLot" were repeating the
  stale doc, and that error propagated for multiple sessions.

## 2.1 The one real architectural difference: what the budget clock counts

This is the deepest divergence and the root of the "never fires" failures.

**kissat denominates every simplifier in TICKS** — propagation work, defined
as `(watchlist_bytes >> 7) + 1` per propagated literal. Each technique gets
a fixed per-mille share of the search ticks accumulated *since that
technique last ran*, floored at 10M ticks (`mineffort = 10`), via
`kimits.h SET_EFFORT_LIMIT`:

```
REFERENCE = search_ticks − last.ticks.{probe|eliminate}
if REFERENCE < 1e7: REFERENCE = 1e7
LIMIT = statistics.START + (NAMEeffort / 1000) · REFERENCE
```

| technique | option | per mille | share |
|---|---|--:|--:|
| eliminate | `eliminateeffort` | 100 | 10% |
| vivify | `vivifyeffort` | 100 | 10% |
| forward subsume | `forwardeffort` | 100 | 10% |
| sweep | `sweepeffort` | 100 | 10% |
| factor | `factoreffort` | 50 | 5% |
| walk | `walkeffort` | 50 | 5% |
| backbone | `backboneeffort` | 20 | 2% |
| transitive | `transitiveeffort` | 20 | 2% |
| substitute | `substituteeffort` | 10 | 1% |

**solver12 denominates in CONFLICTS**: `inprocess_interval_conflicts =
1_000_000` flat (dropping to a 10k first round only for arming-flagged
formulas).

The consequence is structural, not a tuning delta. kissat keeps accruing
ticks on a slow-conflict instance and therefore keeps inprocessing; we
simply never fire. **goldcrest (474 conf/s) and lockchart-group1
(330 conf/s) reach ZERO inprocessing rounds in a full 1800 s run** — they
cannot reach 1M conflicts.

**CORRECTION to earlier project notes:** kissat's `eliminateinit=500` and
`probeint=100` are NOT raw conflict counts. `kimits.c kissat_scale_delta`
multiplies them by a formula-size factor of **at least 25** (quadratic in
`log10(BINIRR_CLAUSES)`), so the real first fire is ~12 500 conflicts for
BVE and ~2 500 for probing, then growing `NLOG2N` / `NLOGN` respectively.
Any prior claim of a "1000x cadence difference" was wrong; the honest
figure at first fire is ~75x, and the *denomination* is the real issue.

### kissat's scheduling substrate (for reference when porting)

`search.c kissat_search` dispatches in strict priority per propagation
fixpoint: `reduce → mode switch → restart → reorder → rephase → probe →
eliminate → limits → decide`. Inprocessing is not on a separate clock — it
fires only when nothing higher-priority is due. Probe and eliminate are both
*blocked on the exact conflict where a reduce happened*.

Conflict-limit rescheduling uses
`UPDATE_CONFLICT_LIMIT(NAME, COUNT, F, SCALE)`: next trigger =
`CONFLICTS + scale_delta(NAMEint · F(invocations))`, with `SQRT` for reduce,
`NLOGN` for probe, `NLOG2N` for eliminate, `NLOG3N` for rephase.

An adaptive **delay** governor (`kimits.c`, `BUMP_DELAY` / `REDUCE_DELAY`)
skips low-yield techniques: on poor yield `current += 1` (skip that many
next invocations), on good yield `current /= 2`. Applied to `bumpreasons`,
`congruence`, `sweep`, `vivifyirr`.

Probe pipeline order within one round (`proberounds=2`):
`congruence → substitute → backbone → vivify → sweep → substitute →
transitive_reduction → backbone → factor`.

## 2.2 Sweeping — crippled by SCHEDULING, the sub-solver is fine

We have kitten and it works: `src/kitten.rs` (868 lines) vs kissat
`kitten.c` (2877). It is used by sweep (default-on) and by definition-based
elimination (off).

**kissat `sweep.c schedule_sweeping` keeps a PERSISTENT schedule:**

1. Variables left over from the previous sweep go to the **front**.
2. All other eligible vars (both polarities present, occurrence counts under
   the limit) are **radix-sorted by total occurrence count ascending** and
   appended.
3. Per-variable `sweep` flags track completion; when none remain incomplete,
   `sweep_completed++`.

**And the bounds escalate on each completed sweep:**

| bound | initial | escalation | cap |
|---|--:|---|--:|
| environment variables | `sweepvars=256` | `<<= completed` | `sweepmaxvars=8192` |
| environment clauses | `sweepclauses=1024` | `<<= completed` | `sweepmaxclauses=32768` |
| environment depth | `sweepdepth=2` | `+= completed` | `sweepmaxdepth=3` |
| ticks | `sweepeffort=100` per mille | 10% of search ticks | — |

It produces backbone units (cheap `kitten_fixed` first, then a model-flip
test, then assumption solves) and equivalences (partition refinement over
the model, flip-pruning, then two assumption solves per candidate pair),
feeding `substitute.c` immediately after.

**solver12 `sweep_round` (`main.rs:10468`) does none of this:**

```rust
for seed in 1..=nvars as i32 {
    if seeds_done >= SWEEP_SEED_BUDGET { break; }   // = 512
```

It **restarts the seed scan at variable 1 on every round**, capped at 512
seeds, with no completion tracking and no escalation ladder. It also clones
the entire original clause DB into a per-round snapshot (`snap` + `occ`,
with `clauses_of` cloning vectors per query).

On a 100k-variable instance this re-sweeps the same ~512 lowest-numbered
variables forever. **This is the 450x productivity gap**: we find 0–826
equivalences per instance where kissat kitten-solves 90k–18M times
(`plan/gap-read-2026-07-21.md` table). Bead SAT-playground-5b2.3.39.

This is a scheduling bug with a known-good target design — ordinary
engineering, not research.

### 2.2a MEASURED 2026-07-24 — a bare cursor is NOT a uniform win

Implementing the plain cursor and measuring it (paired probe, 200k conflict
limit, `SAT_INPROCESS_INTERVAL_CONFLICTS=20000` so rounds actually fire —
note the shipped 1M cadence produces ZERO sweep rounds at 200k conflicts,
which is itself section 2.1 in miniature):

| cell | legacy `sweep_eq` | cursor `sweep_eq` | direction |
|------|------------------:|------------------:|-----------|
| booth_dadda_mapped | 792 | **7 303** | cursor 9.2x better |
| BubbleVsPancakeSort | 77 | **316** | cursor 4.1x better (bb 1 -> 93) |
| VexRiscv | **10 330** | 3 291 | cursor 3.1x WORSE |
| oski15a01b20s | **1 005** | 234 | cursor 4.3x WORSE |
| bp4_TCO_IXA_LP | **1 486** | 882 | cursor 1.7x WORSE |
| fixedbandwidth | 0 | 0 | tie (sweep finds nothing either way) |

**Why:** restarting at variable 1 accidentally re-visits a PRODUCTIVE
frontier. `try_els` merges one miter layer, which exposes the next layer's
equivalences **at the same variables** (this is documented in the
`sweep_round` code comment). A bare cursor walks away from that frontier
before it is exhausted, trading depth of exploitation for breadth of
coverage. Miters (booth/Bubble) want breadth; BMC/circuit cells
(VexRiscv/oski/bp4) want the repeated frontier.

**The fix is kissat's actual design, which does BOTH:** retire only
*exhausted* seeds, keep leftovers at the front. Implemented as
`SAT_SWEEP_SCHED=retire` — per-variable barren flags, skipped on later
rounds, cleared wholesale on a completed pass so the escalated environment
gets a fresh crack (kissat's `try_to_eliminate_all_variables_again`
analogue). Productive seeds are never flagged, so they stay in rotation
while the budget reaches deeper variables.

**Lesson worth carrying:** "this is obviously a bug, just fix it" was wrong
here. The legacy behaviour was load-bearing on three cells. Always measure
the mechanism before assuming a defect is pure loss.

### 2.1a IMPLEMENTED 2026-07-24 — `SAT_INPROCESS_TICK_CADENCE` (default-off)

Tick-denominated inprocessing trigger, added alongside the conflict one.

**Measured tick vs conflict accumulation (240 s windows, idle host):**

| cell | conf/s | ticks/s | ticks/conflict |
|------|-------:|--------:|---------------:|
| lockchart-group1 | 92 | 4.3M | **47 123** |
| pj2008 | 389 | 9.2M | **23 671** |
| goldcrest | 452 | 9.5M | **20 925** |
| VexRiscv | 401 | 6.0M | **15 033** |
| case7 (healthy) | 648 | 5.8M | 9 012 |
| sqrt-mitern170 | 3 705 | 33.9M | 9 155 |
| fixedbandwidth | 23 390 | 102M | 4 362 |

**Why the trigger MUST be ratio-scoped, not a flat tick interval.** Tick rate
spans only 23x across cells; conflict rate spans 254x. So any flat tick
interval that fires on lockchart also fires ~19x on sqrt-mitern170 — and
section 2.2c already proved that piling inprocessing onto already-solving
miters LOSES them. **Ticks-per-conflict is the discriminator** and the
measured gap is clean: starved 15k-47k, healthy 4k-9.2k. Floor set at 12 000,
with a 20k-conflict warmup so early propagation-heavy search cannot
misclassify. Mechanism (not a family classifier): high ticks/conflict means
much propagation work per learned clause, i.e. an unsimplified formula
relative to search progress.

**Verified inert where it must be:** sqrt-mitern170 bit-identical with the
flag on (889 421 conflicts / 177 390 vivify attempts / 83 equivalences),
case7 identical (155 717). **Verified live where it should be:** goldcrest
first-ever sweep facts (0 -> 351 equivalences, 0 -> 11 backbones), VexRiscv
vivify +50%, and VexRiscv/pj2008 do 29%/24% MORE conflicts in the same wall
(formula shrank, search sped up).

**TUNING RESULT: no +1.** Full 1800 s runs at intervals 0.5e9 / 1.5e9 / 4e9
across goldcrest, pj2008, lockchart, VexRiscv, booth_dadda, bp4_TCO_IXA —
**all 24 runs TIMEOUT, no cell flipped at any interval.**

**Why: this is a magnitude problem, not a scheduling one.** kissat cracks
goldcrest with 85% elimination and 4.7M kitten solves; a few extra rounds buy
us 351 sweep equivalences. The cadence fix was NECESSARY (52 of 73 solved
cells inprocess zero times) but is nowhere near SUFFICIENT. Classification:
validated-neutral groundwork that unblocks depth work, not a metric mover on
its own. The follow-up is rounds x DEPTH — the built-but-off passes of
section 2.6 (`SAT_GATE_BVE`+`SAT_GATE_EXTRACT`, `SAT_ELIM_DEF`) running on
cells that now actually get rounds.

**Methodological trap learned the hard way:** marginal-cell timing is
impossible while a 32-way gate runs. VexRiscv (2.3M props/s over 723k vars)
timed out at 1801 s in BOTH arms on "free" cores 40/42 while the gate
saturated memory bandwidth on cores 0-31, though it solves at ~1500 s idle.
**Under contention a SOLVE is still trustworthy; a TIMEOUT is not.** Schedule
marginal-cell measurements on a quiet host.

### 2.6a BREAKTHROUGH 2026-07-24 — gate-aware BVE solves a kissat-only cell

Depth probe, full 1800 s, 5 starved/kissat-only cells x 6 arms
(base / tick / gbve / edef / tick+gbve / tick+gbve+edef):

| cell | base | tick | **gbve** | edef | tick+gbve | tick+gbve+edef |
|------|------|------|----------|------|-----------|----------------|
| **bp4_TCO_CSO_IXA_LP_ZR** | TO | TO | **SAT 425 s** | TO | TO | TO |
| booth_dadda_mapped | TO | TO | TO | TO | TO | TO |
| goldcrest | TO | TO | TO | TO | TO | TO |
| fixedbandwidth | TO | TO | TO | TO | TO | TO |
| pj2008 | TO | TO | TO | TO | TO | TO |

**`SAT_GATE_EXTRACT=on SAT_GATE_BVE=on` solves bp4_TCO_CSO_IXA_LP_ZR in 425 s
— a FIRST-EVER solve of a kissat-only cell (kissat needs 1187 s), with a
1375 s margin.** Solves under contention are trustworthy (see the contention
trap in 2.1a), so this is real signal.

**Two findings of equal importance:**

1. **Depth is the lever, not frequency.** `gbve` (deeper elimination per
   round) flipped a cell that no cadence interval and no sweep schedule could
   touch. This is the third consecutive result pointing the same way:
   scheduling/frequency changes (sweep cursor 2.2c, tick cadence 2.1a) are
   neutral-to-negative, while a DEPTH change flips a cell on its first try.
   Section 3.3's ordering was wrong — the built-but-off depth passes should
   have been #1, not #3.
2. **tick+gbve TIMED OUT where gbve alone solved.** Adding inprocessing
   rounds destroyed the win. Consistent with 2.2c (extra inprocessing on a
   cell that can otherwise finish is a net loss). **Do not bundle the cadence
   with the depth passes** — they are antagonistic on this cell.

`edef` (`SAT_ELIM_DEF`, kitten definition extraction) flipped nothing alone
and did not add to gbve.

Next: reproduce + model-verify bp4 (first-ever solves are exactly where latent
model/proof bugs surface), then gate `SAT_GATE_EXTRACT+SAT_GATE_BVE` ALONE.
Note FEATURES.md claims gate-BVE was "rejected for default" by an earlier
session — that verdict predates the current feature set, and this is now a
measured +1 candidate, so re-gate rather than trust the note.

### 2.2b The seed budget is the real scaling defect

`SWEEP_SEED_BUDGET` is a FIXED 512 seeds/round, but the suite spans three
orders of magnitude in variable count, so the same constant means wildly
different coverage:

| cell | variables | 512 seeds covers |
|------|----------:|-----------------:|
| booth_dadda | 2 948 | 17% per round |
| oski15a01b20s | 488 500 | 0.10% |
| VexRiscv | 723 395 | 0.07% |

On VexRiscv a cursor needs ~1 400 rounds merely to wrap once, so it never
returns to the frontier AND never completes a pass to trigger escalation.
kissat has no seed count at all — sweep is budgeted at `sweepeffort=100` per
mille of search ticks. **This is the tick-vs-count denomination problem of
section 2.1 appearing independently in a second place.**

Raising the budget multiplies measured productivity (`sweep_equivalences` at
200k conflicts / 20k interval, legacy -> retire@2048): booth_dadda 792 ->
41 460 (52x), bp4_TCO_IXA 1 486 -> 72 984 (49x), oski15 1 005 -> 4 336,
VexRiscv 10 330 -> 11 928. At b=8192 oski15 reaches 26 512 but several cells
start timing out.

### 2.2c GATE RESULT — retire@2048 FAILED, and why it matters

**Gate (medium 100, single seed, 1800 s): cand 64 v base 69 — LOSE.**
Artifacts `log/abtest-cand-vs-base-2026-07-24-15-48-41`.

- **Lost 6:** sqrt-mitern170 (base UNSAT 1151 s), sqrt-mitern171 (389 s),
  div-mitern172 (220 s), PancakeVsSelectionSort_6_7 (639 s), **TT496
  (1133 s — banked unique capability)**, VexRiscv (1502 s).
- **Gained 1:** oski15a01b20s (1745 s, wall coin).
- Both-solved wall **19 751 s vs 17 855 s (+10.6%)**; worst deltas TT406
  +848 s, 59-129706 +623 s, oddball_24 +393 s.

**THE MITERS REGRESSED** — the exact class SAT sweeping exists to crack.
b=2048 quadruples kitten solves per round; the extra equivalences do not pay
for their wall cost, and cells already solving in 200-1200 s get pushed past
1800 s.

**The load-bearing lesson: sweep productivity is NOT solving improvement.**
The 49-52x equivalence multipliers were bought with wall time the gate
charges for. Any future sweep work must be measured in solved cells and
wall, never in `sweep_equivalences` alone — that metric is actively
misleading as an optimisation target.

Follow-up gate (schedule change isolated at constant cost,
`SAT_SWEEP_SCHED=retire` with the budget left at 512):
`log/abtest-sweepretire512-launch-2026-07-24.log`.

## 2.3 Variable elimination

**kissat's bound escalation** (`eliminate.c set_next_elimination_bound`):
`bounds.eliminate.additional_clauses` starts at **0** (pure non-increasing
BVE) and only advances `0 → 1 → 2 → 4 → 8 → 16` when a full round
*completes*, re-flagging **all** variables on each step.

Per-variable limits: `eliminateocclim=2000` (skip if pos+neg exceeds),
resolvent-count limit `pos + neg + bound`, `eliminateclslim=100`.
Round structure: `eliminaterounds=2`, each preceded by forward subsumption
(`forwardeffort=100` per mille, `subsumeclslim=1000`, `subsumeocclim=1000`),
scheduled from a max-heap on
`relevancy + (pos·neg − pos − neg) − occlim²`.

**Gate/definition extraction** (`gates.c kissat_find_gates`), first hit wins:

1. `equivalences.c` — binary pair ⇒ `lit ≡ x`.
2. `ands.c` — marked binaries + large clause ⇒ AND gate.
3. `ifthenelse.c` — matched ternary pairs ⇒ ITE gate.
4. `definition.c` — **the kitten-based one**: export all occurrences of
   `lit` / `¬lit` into kitten with the pivot as an exception, solve. UNSAT ⇒
   a functional definition exists. Budget `definitionticks=1e6`, then
   `definitioncores=2` core-minimization passes at **10x budget** each. If
   one side's core is empty the pivot is a **failed literal** ⇒ unit learned
   with DRAT lemmas replayed from the kitten core.

When a gate is found only `gate × antecedent` resolvents are generated —
this is what makes BVE affordable on circuit instances, and it is why kissat
reaches 72–88% elimination on the miters where we sit at 43–56%.

**solver12:** root BVE is `grow=0`, `clause_lim=20`, occ limit 2000, with 3e9
tick / 1e8 resolution budgets. Mid-search BVE rounds run for *armed*
formulas with the same `0→1→2→4→8→16` ladder (`SAT_ELIM_ARMED_BOUNDS`, on).
Congruence (AND/OR + ITE + XOR, union-find to fixpoint, feeding ELS
substitution) runs at root and every inprocess round, gated by a dry-run
threshold (`CONGRUENCE_MIN_APPLY_MERGES = 3000`) so non-circuit formulas
stay byte-identical.

**But `SAT_GATE_BVE` + `SAT_GATE_EXTRACT` (Plaisted-Greenbaum gate-aware
BVE) and `SAT_ELIM_DEF` (kitten definition extraction) are BUILT AND OFF.**
See 2.6.

## 2.4 Already at PARITY — do not spend effort here

- **Tier limits are dynamic in both.** kissat `tiers.c` derives tier1/tier2
  from a glue-usage histogram at the 50%/90% percentiles
  (`tier1relative=500`, `tier2relative=900` per mille) with the static 2/6
  only as fallback. solver12 does the same:
  `compute_tier_limits_from_histogram` (`main.rs:11357`),
  `TIER1_RELATIVE = 1/2`, `TIER2_RELATIVE = 9/10`, floors 2/6.
- **BVE bound ladder** `0→1→2→4→8→16` on round completion (ours for armed
  formulas via `SAT_ELIM_ARMED_BOUNDS`).
- **Propagation throughput.** Measured 5.62M props/s vs kissat 5.1M on
  pj2008 — we are *faster* per propagation. The rate gap is clause-DB size,
  not the propagator. (This closed the "prop throughput" arc in the
  2026-07-18 session; do not reopen it.)

## 2.5 Genuinely different control law: REDUCE

Best current hypothesis for the 2.2–9x throughput gap in 1.6.

**kissat `reduce.c`** deletes a **fraction** of reducible clauses, ramping
50% → 90% as `high − (high−low)/log10(reductions+9)` with
`reducelow=500`, `reducehigh=900` per mille. Retention rules: keep if it's a
reason, or `glue ≤ tier1 && used > 0`, or `glue ≤ tier2 && used ≥ 30`,
where `used` is a **5-bit counter, `MAX_USED = 31`**, decremented on every
reduce. Ranking for deletion: `(~glue << 32) | ~size` — worst glue first,
then largest size. Trigger `reduceinit = reduceint = 1000`, next at
`1000 · sqrt(reductions)`.

**solver12 `reduce_db_lbd_tiered` (`main.rs:11511`)** deletes down to a
**literal budget** (`learned_lit_budget`), sorting worst-LBD-first then
largest-size, with **`MAX_USED_RECENTLY = 3`**.

Two differences that compound over long runs:

1. A budget-driven law and a fraction-driven law diverge as the DB grows —
   the fraction law is self-limiting, the budget law is not.
2. A 3-step usage counter is a far coarser retention signal than a 31-step
   one, so clause "usefulness" is much more weakly discriminated.

Longer clause DB ⇒ longer watch lists ⇒ more work per propagation, which is
exactly the shape of a slowdown at *identical conflict counts*.

## 2.6 Built but SWITCHED OFF (gate runs, not development)

| flag | what it is | kissat counterpart |
|---|---|---|
| `SAT_PROBE` | root failed-literal probing, `main.rs:6318`, proportional 5M–100M tick budget | `probe.c` pipeline |
| `SAT_GATE_BVE` + `SAT_GATE_EXTRACT` | gate-aware / Plaisted-Greenbaum BVE (implemented, DRAT-verified) | `gates.c` |
| `SAT_ELIM_DEF` | kitten definition extraction (ticks 50k, cores 2) | `definition.c` |
| `SAT_ELS` | ELS as a standalone root pass (engine is live via congruence/sweep) | `substitute.c` |
| `SAT_FACTOR_INPROCESS` | mid-search BVA | `factor.c` |

These need **benchmark gate runs, not implementation**. That is a very
different cost profile from a port.

## 2.7 Actually ABSENT (runtime-rejected, `config.rs:1752`)

`SAT_HBR`, `SAT_TRANSITIVE`, `SAT_FORWARD_SUBSUME`, `SAT_RCHECK` — the
validator hard-fails with "implementation bead has not landed". Plus BCE
(`SAT_BCE` is a denylisted name only) and asymmetric branching.

**Important scoping note:** kissat has **no standalone HBR module either**.
The failed-literal role is split across `backbone.c` (binary-implication-
graph only, own mini-propagator, `backboneeffort=20` per mille = 2%,
`backbonerounds=100 × computations` capped at 1000) and `transitive.c`
(removes transitively-implied binaries, 2% effort, `transitivekeep=1`).
These are small cheap passes, not big ports.

## 2.8 Vivification granularity

**kissat `vivify.c`** runs **four rounds per invocation** in fixed order —
tier1, tier2, tier3, irredundant — splitting one `SET_EFFORT_LIMIT` budget
by relative weights `vivifytier1=3`, `vivifytier2=3`, `vivifytier3=1`,
`vivifyirr=3` (**3:3:1:3**), with **unspent slack carried forward** to the
next round. Candidates are scheduled with prioritized (`c->vivify`-flagged)
clauses on top of the stack; unfinished candidates keep their flag for the
next invocation. `vivifyfocusedtiers=1` means the focused-mode tier limits
are used regardless of current mode.

**solver12** vivifies originals always, plus learned candidates restricted
to tier1/tier2, delayed to 6M conflicts, and suppressed entirely when
post-preprocess binary fraction ≥ 0.85. **tier3 is never vivified.**

## 2.9 Restart / rephase / walk / mode

Mostly at parity in shape; recorded for completeness.

- **Modes.** kissat alternates stable/focused with an asymmetric law:
  focused length in conflicts (`1000 · log10(n+9)⁴`), stable length in
  *ticks* mirroring the preceding focused phase. solver12 has
  `search_mode_policy = FocusedStable` with `mode_use_ticks = true`.
- **Restarts.** kissat: focused = Glucose EMA (`restartmargin=10` ⇒ 1.10x,
  `emafast=33`, `emaslow=1e5`), stable = reluctant doubling
  (`reluctantint=1024`). solver12 mirrors this via
  `effective_restart_policy()` (`main.rs:7632`). **Trail reuse is ON in
  kissat (`restartreusetrail=1`) and OFF in solver12** — a small unexplored
  delta.
- **Rephase.** kissat: stable mode only, fixed 6-slot cycle
  `best, walk, inverted, best, walk, original`, `rephaseinit=1000` growing
  `NLOG3N`. solver12: config-level off but **self-enables at runtime** via
  the arming latch and `SAT_ENDGAME`.
- **Walk.** kissat reaches it only as the `W` slot of the rephase cycle
  (`walkinitially=0`), budget `walkeffort=50` per mille. solver12's
  `walk.rs` (ProbSAT port) is likewise only reachable from rephase slots,
  so it is effectively active only on armed/endgame formulas. Note
  `SAT_WALK` is on the PARKING_LOT_DENYLIST — setting the env var *aborts
  the run* even though the feature is live by default.

## 2.10 Our side of the ledger — no kissat counterpart at all

1. **`SAT_GAUSS`** — GF(2) Gaussian refutation of extracted XOR systems,
   coverage-gated at ≥90%, emitting a **pure-resolution DRAT proof**
   (drat-trim verified, 0 RAT lemmas). kissat has no XOR engine.
2. **`SAT_TSEITIN`** — closed-Tseitin component detection with a
   width-bounded **extension-variable** DRAT proof. This is extended
   resolution, **outside kissat's proof system entirely**.
3. **`SAT_PAIR_ABS_REFUTE`** — adjacent-pair parity abstraction: recognizes
   complete pair-XOR expansions, introduces fresh parity variables,
   resolution-lifts every abstract clause, solves the compact abstract CNF,
   maps the UNSAT proof back.
4. **`SAT_ENDGAME`** — late-arming latch (formulas armed at ≥100k conflicts)
   pinning a flat rephase/walk delta (48k decision-armed / 50k yield-armed)
   plus a restart floor.
5. **The arming / routing layer generally** — `SAT_DECISION_ARM`,
   `SAT_VIVIFY_YIELD_ARM` dry-run probes, the deep-phase sweep guard, the
   congruence dry-run threshold. An adaptive per-formula router kissat has
   no analogue for.

Protect all of these in any reroll campaign.

---

# PART 3 — IS MORE ITERATION WORTH IT?

## 3.1 The returns picture

126 commits since 2026-07-01 took the medium baseline from 55/100
(2026-07-05) to 70/100 (2026-07-23) — a real arc. But the recent slope is
flat: **67 (07-14) → 70 (07-23)**, roughly +3 over ~20 promotion sessions,
and the wins in that window are wall diets and lottery flips worth about +1
each with genuine reroll risk. Nine consecutive wall diets have banked
maybe +5 total. rbsat cleared the 1800 s line by 20 seconds in this deal.

That specific vein — cadence constants and wall-coin hunting — is close to
exhausted, and the 2026-07-23 delta sweep already proved the point
empirically (TT496 solved at exactly 2 of 12 tested deltas with no
smoothness; adjacent deltas differ 3x in conflicts).

## 3.2 Why the answer is still yes

The source audit changes the cost side of the ledger substantially. The
remaining gap is **not** a port backlog. It is:

- **one scheduling bug** (2.2) with a 450x measured productivity gap behind
  it,
- **one clock change** (2.1) that eliminates a whole class of "never fires"
  failures,
- **three features already written and switched off** (2.6) that need gate
  runs rather than development,
- and only **two small genuinely-missing passes** (2.7), each of which kissat
  itself budgets at 2% effort.

That is a much cheaper path than "reimplement kissat's inprocessing."

## 3.3 Recommended order

1. **Fix the sweep seed cursor.** Advance monotonically across rounds; add
   the leftovers-first + occurrence-sorted persistent schedule and the
   completion/escalation ladder; stop cloning the clause DB per round.
   Touches all nine kissat-only cells. (2.2)
2. **Re-denominate the inprocessing budget in ticks** with an effort floor.
   Named beneficiaries goldcrest and lockchart-group1. Must spare
   sudoku-N30 and bp5 (never-armed but currently solving). (2.1)
3. **Gate-run what is already built** — `SAT_PROBE`, `SAT_GATE_BVE` +
   `SAT_GATE_EXTRACT`, `SAT_ELIM_DEF`. Target cells: fixedbandwidth,
   goldcrest, bp4_TCO_IXA_LP, booth_dadda_mapped. (2.6)
4. **Small ports:** `backbone.c`, `transitive.c`, vivify tier3 + the 3:3:1:3
   split. (2.7, 2.8)
5. **Reduce control law** — highest ceiling, highest risk. Measure offline
   first (kept-clause counts + ticks/prop on rbsat/Bubble under kissat-style
   limits, `SAT_LIMIT_CONFLICTS` identity screens, no gate). Rerolls every
   ≥1M-conflict trajectory, so it needs a deliberate re-luck campaign under
   the REROLL-LUCK LAW, not a single gate run. (2.5)
6. **10th wall-diet** — demoted but still carries a free +1:
   bp4_TCO_CSO_ZR at 1880 s is kissat-impossible and deterministic, so ~5%
   wall is a capability-backed gate +1 with no reroll. Keep as the cheap
   fallback when 1–3 stall; do not lead with it.

## 3.4 What to stop doing

Tuning cadence constants and hunting wall-coin flips. The class is a
lottery; only fat-margin draws (>600 s) are promotable, and every reroll
risks banked luck elsewhere in the 70-lineage.

## 3.5 The framing that should drive the call

**At the 1800 s promotion gate, solver12 already beats kissat 71 to 67 on
the same deal.** The deficit is a >3000 s tail phenomenon. So this is not
catch-up work — it is a decision about whether the target is the project's
own gate (already won) or competition-realistic 5000 s scoring (where the
tail, and therefore items 1–3, is the whole game).

---

# PART 4 — PROVENANCE

- Benchmarks: files listed in 1.1; runs sequential on the idle 72-core host
  2026-07-24 07:24–12:23 EDT.
- Truncation curve: computed from the single 3600 s deal, both result files.
- solver12 source audit: `src/config.rs`, `src/main.rs`, `src/simp.rs`,
  `src/sweep.rs`, `src/kitten.rs`, `FEATURES.md` (found stale).
- kissat source audit: `benchmarks/reference-solvers/kissat-latest/src/` —
  `options.h`, `kimits.{c,h}`, `search.c`, `sweep.c`, `kitten.c`,
  `eliminate.c`, `gates.c`, `definition.c`, `vivify.c`, `probe.c`,
  `backbone.c`, `transitive.c`, `substitute.c`, `reduce.c`, `tiers.c`,
  `restart.c`, `rephase.c`, `walk.c`, `mode.c`, `preprocess.c`.
- Prior mechanism numbers (elimination %, kitten solve counts, sweep finds
  per cell): `plan/gap-read-2026-07-21.md` and
  `log/gap-read-2026-07-21/deepdive/COMPARISON.txt` — still valid.
- Ranked next steps derived from this doc: `plan/next-plan.md`.
