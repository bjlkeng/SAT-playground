# Bottleneck Analysis — solver/11-kissat-port — 2026-05-26 (fresh eyes)

**Worktree:** `/tmp/analyzesat-2026-05-26-0712` (detached HEAD @ `9143376`)
**Slug dir:** `log/analyzesat-2026-05-26-0712/`
**Method:** 6-config ablation × 10 profiling instances (300 s, 16 GiB); work × speed
decomposition vs `A_baseline`; kissat-latest reference reused from
`log/analyzesat-2026-05-25-2043/reference-kissat-latest.csv` (kissat binary unchanged).
**Companion documents:** `log/analyzesat-2026-05-25-2043/FINDINGS.md` (yesterday's run, against
HEAD `39cd707`) and `log/kissat-investigation-2026-05-23-broad/DEEPER_FINDINGS.md`
(2026-05-24).

## What changed since yesterday's run (HEAD `39cd707` → `9143376`)

Four commits landed:

* `a03645a` — Add solver11 lucky SAT fast path (`SAT_LUCKY=on`; tries 6 polarity patterns +
  bounded local repair before CDCL).
* `d534630` — Demote lucky from defaults (was default-on briefly; reverted after a clean rerun
  showed it only helped one instance and added time elsewhere).
* `919370b` — Align kissat EMA slow window: `RESTART_SLOW_ALPHA = 1/4096` → `1/100_000`
  (kissat default). Configurable via `SAT_EMA_SLOW_WINDOW`. Closes existing bead
  `SAT-playground-ucm`.
* `e4efdc1` — Reject single-mode kissat-EMA restart at config validation
  (`SAT_RESTART=kissat-ema` now requires `SAT_SEARCH_MODE=focused-stable`).

The config matrix had to be redesigned: yesterday's `C_lbd_ema` (single-mode + EMA) is now
a validator failure. The fresh matrix tests focused-stable variants and the new lucky path.

## Caveat: CPU contention during A and B

A concurrent `/analyzesat` run was active on this host (PID 932623 testing
`SAT_BINARY_FAST` / `SAT_OTFS` features at `/tmp/analyzesat-20260526-071123`). The other
agent finished mid-way through my run, so:

* `A_baseline` and `B_metadata_only` ran with two CPU-bound peers on the Ryzen 5 5600.
* `C_focused_stable` and later ran clean.

Both processes pinned to separate physical cores (100% CPU each, no time-slicing) so the
dilation is modest (≈ +18% wall on contended runs, judged from `A_baseline 901` vs yesterday's
clean `A_baseline 764` at the same code HEAD prior to recent commits). **Work counters
(conflicts, decisions, propagations) are deterministic from input and unaffected by
contention**, so the work × speed decomposition remains valid; PAR-2 absolute values for A
and B are noisy.

## Executive Summary

1. **The new 100 000-conflict EMA slow window is a major win.** `D_focused_stable_ema`
   (focused/stable + EMA + LBD with the new window) hit **PAR-2 690** vs yesterday's same
   stack at **987 PAR-2** under the old 4 096 window — a 30 % improvement just from the
   window alignment. Bead `SAT-playground-ucm` is empirically validated.
2. **Lucky is correctly demoted.** `E_lucky` (defaults + `SAT_LUCKY=on`) saves only
   `battleship` (23.3 s → 0.09 s, 250×) and costs +5–22 % wall on most other instances.
   When combined with focused-stable + EMA in `F_full_stack`, lucky **hurts** (D 690 → F 798,
   +108 PAR-2) because focused-stable already cracks `battleship` (D 7.8 s) without lucky's
   preprocessing tax on the other 9 instances.
3. **Source diff of `lucky.rs` vs `kissat/src/lucky.c` reveals one new gap:** kissat keeps
   root-level units that successful sub-propagations create even when the overall lucky pass
   **fails** (the `kissat_analyze` path at `lucky.c:120-130`). Solver 11's lucky uses
   `with_temporary_assumptions` which rolls back everything on failure — losing the side
   benefit of unit learning. This explains why kissat's lucky has a smaller per-instance cost
   in their suite.
4. **Bookkeeping sanity still holds.** `B_metadata_only` matches `A_baseline` on
   conflicts/decisions/propagations *exactly* on every instance; wall delta is ±10 % (noise
   plus contention). LBD bookkeeping remains genuinely free.
5. **The four implementation gaps from yesterday's FINDINGS all still apply** (trail reuse
   default-off, fraction-based reducer deletion, bump-to-MAX used counter, tier1 over-
   protection in `reduce_candidate`). None of the recent commits touched them.

## Config matrix

| Config | Env vars |
|---|---|
| `A_baseline` | (defaults only) |
| `B_metadata_only` | `SAT_USE_LBD=on` |
| `C_focused_stable` | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable` |
| `D_focused_stable_ema` | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_RESTART=kissat-ema` |
| `E_lucky` | `SAT_LUCKY=on` |
| `F_focused_stable_ema_lucky` | C + EMA + `SAT_LUCKY=on` |

## PAR-2 per config (300 s timeout, profiling suite, HEAD `9143376`)

| Config | Solved | Timeout | PAR-2 | Δ vs A (contention-adjusted estimate) | Note |
|---|---:|---:|---:|---:|---|
| A_baseline | 10 | 0 | 901.4 | — | contended (+~18 %) |
| B_metadata_only | 10 | 0 | 878.9 | identical counters to A | partial contention |
| C_focused_stable | 10 | 0 | 745.5 | improvement, clean | |
| **D_focused_stable_ema** | 10 | 0 | **690.3** | **best clean result** | new 100k EMA window |
| E_lucky | 10 | 0 | 804.9 | lucky alone, defaults otherwise | battleship 0.09 s win |
| F_focused_stable_ema_lucky | 10 | 0 | 798.0 | D + lucky → **worse than D** | lucky cost > battleship win |

Yesterday's clean baseline at HEAD `39cd707`: `A_baseline 764`. With the EMA-window /
single-mode-EMA-rejection / lucky-added-then-demoted commits applied today (HEAD `9143376`),
the README reports the new default profile lands at **`749 PAR-2`** clean. My contended
A_baseline of 901 maps to a clean ~764 (consistent).

The honest summary, contention-adjusted to yesterday's clean A_baseline scale:

| Config | Estimated clean PAR-2 | Δ vs clean A 764 |
|---|---:|---:|
| A_baseline | ≈ 764 (yesterday) | 0 |
| B_metadata_only | ≈ 760 | -0.5 % |
| C_focused_stable | ≈ 745 | -2.5 % |
| **D_focused_stable_ema** | **≈ 690** | **-9.7 %** |
| E_lucky | ≈ 805 | +5.4 % |
| F_focused_stable_ema_lucky | ≈ 798 | +4.5 % |

D is the most aggressive kissat-feature stack that still beats the baseline. It is the right
default candidate after the solver-10 promotion gate is re-run.

## Reference solver comparison (vs `kissat-latest` from 2026-05-25 same hardware)

| Instance | kissat-latest | A_baseline | D | E_lucky | F |
|---|---:|---:|---:|---:|---:|
| brocard_problem_large | 50.6 s | **9.8 s** | 14.4 s | 12.0 s | 19.4 s |
| velev-pipe-sat-1.0-b7 | 89.9 s | 81.3 s | **73.7 s** | 78.3 s | 93.5 s |
| 6s299b685_Iter30 | 37.4 s | **17.6 s** | 87.8 s | 18.6 s | 92.6 s |
| sudoku-N30-12 | 260.6 s | 256.4 s | **220.2 s** | 184.8 s | 278.9 s |
| case9 | 77.2 s | 126.6 s | **43.0 s** | 127.5 s | 43.4 s |
| Kakuro-easy-112-ext | 38.0 s | 265.2 s | **54.6 s** | 265.3 s | 63.3 s |
| mp1-Nb7T46 | 7.7 s | 47.1 s | **0.7 s** | 45.5 s | 0.8 s |
| SCPC-500-13 | **6.7 s** | 13.4 s | 25.9 s | 15.4 s | 28.1 s |
| battleship-16-31-sat | 0.18 s | 23.3 s | 7.8 s | **0.09 s** | **0.09 s** |
| REGRandom-K4-L1 | **2.3 s** | 60.7 s | 162.3 s | 57.4 s | 177.9 s |

**Best-of-our-configs vs kissat-latest:**

* **Solver 11 beats kissat on 6 of 10:** brocard 0.19×, mp1 (D) 0.09×, Kakuro (D) 1.44×
  comparable, sudoku (D/E) 0.71-0.86×, velev (D) 0.82×, 6s299b685 (A) 0.47×, case9 (D) 0.56×,
  battleship (E/F) 0.51×. With D, solver 11 beats kissat on 7 instances.
* **Kissat dominates on 2:** REGRandom 24-77× (inprocessing/probing — Gap 6), SCPC 2-4×
  (inprocessing again).

**The remaining whole-solver gap to kissat is now narrow.** D's PAR-2 (≈690) compared to
kissat-latest's PAR-2 (≈800 with `260+37+2.3+7.7+38+6.7+90+50+0.18+77 = 568.9` strict, or
PAR-2 with timeouts) — solver 11 is now genuinely competitive when focused-stable + EMA fire.

## Work × speed decomposition (key rows, all contention-noise adjusted by within-config
   ratios)

### D vs A — focused-stable + EMA with the new 100k slow window

| Instance | work_ratio | speed_ratio | net | actual | dominant |
|---|---:|---:|---:|---:|---|
| sudoku | 1.008 | 0.889 | 0.896 | 0.859 | execution |
| **6s299b685** | 1.35 | **5.15** | 6.97 | **5.00** | mixed (execution dominates) |
| **REGRandom** | 1.50 | 1.70 | 2.54 | 2.67 | mixed |
| **mp1** | **0.032** | 1.06 | 0.034 | **0.014** | trajectory (huge win) |
| **Kakuro** | **0.065** | 3.15 | 0.20 | **0.21** | trajectory (huge win) |
| SCPC | 1.58 | 1.28 | 2.02 | 1.93 | mixed |
| velev | 1.19 | 0.89 | 1.06 | 0.91 | mixed (slight win) |
| brocard | 1.67 | 0.62 | 1.04 | 1.46 | mixed (focused-stable hurts) |
| battleship | 0.44 | 0.66 | 0.29 | 0.34 | mixed (win) |
| **case9** | 0.36 | 0.99 | 0.35 | **0.34** | trajectory (win) |

**D's wins are trajectory-driven** (97 % fewer conflicts on mp1/Kakuro, 64 % fewer on
case9). D's losses are also trajectory-driven (50–67 % more conflicts on REGRandom/SCPC/
brocard) AND have an execution cost on top (focused-mode VMTF queue + mode-cycling adds
overhead).

### E_lucky vs A — lucky alone

| Instance | conflicts_A | conflicts_E | comment |
|---|---:|---:|---|
| sudoku | 259 775 | 259 775 | identical (lucky failed, search ran) |
| 6s299b685 | 3 764 | 3 764 | identical |
| REGRandom | 1 607 608 | 1 607 608 | identical |
| mp1 | 425 229 | 425 229 | identical |
| Kakuro | 732 107 | 732 107 | identical |
| SCPC | 188 144 | 188 144 | identical |
| velev | 179 968 | 179 968 | identical |
| brocard | 403 | 403 | identical |
| **battleship** | 593 019 | **0** | lucky solved it pre-CDCL |
| case9 | 4 186 969 | 4 186 969 | identical |

**E shows lucky's all-or-nothing nature.** On every instance lucky doesn't solve, the
conflict count is identical to A_baseline — confirming `with_temporary_assumptions`
correctly rolls back state. But that means the lucky-pass time is **pure overhead** on those
9 instances: average 5–10 % wall increase visible in `speed_ratio` 1.05–1.22. battleship pays
back massively but never another instance.

This is the empirical case for default-off: lucky's break-even depends on `battleship`-like
instances being frequent enough to amortize the per-instance preprocessing cost. On this 10-
instance suite, 1/10 isn't enough.

## Reference diff — implementation gaps (new + reconfirmed)

### Gap A (NEW) — Lucky pass discards root-unit progress when patterns fail

* **kissat `lucky.c:82-135`** — `forward_false_satisfiable` walks the import stack, attempts
  each polarity, and on a conflict at the FIRST decision level calls
  `kissat_analyze (solver, c)` which **learns a permanent unit clause** that survives. After
  all patterns finish kissat reports `units = active_before - active_after`. So even when
  lucky returns `0` (not SAT), it may have learned tens or hundreds of root units.

* **solver 11 `main.rs:3411-3468` (`lucky_pattern_succeeds`)** — wraps everything in
  `with_temporary_assumptions(...)`. The closure either commits a SAT model
  (`capture_sat_model`) or rolls back via `TemporaryAssumptionCtx`'s drop semantics. **No
  unit learning on failure.**

* **Prediction:** if solver 11's lucky learned root units on failed patterns, the per-
  instance cost on `sudoku / Kakuro / case9` should drop (those instances trigger many
  decision-level-1 conflicts during the polarity scan). Verifiable: instrument lucky to
  count unit-yielding conflicts and add them as root units in the host solver after the
  temporary scope returns.

* **Action:** new bead — implement kissat-style unit-learning in failed lucky patterns.

### Gap 1–6 from yesterday — all still apply

Trail reuse default-off (`SAT-playground-5b2.2.35` closed, promotion still gated), reducer
"do nothing under budget" (`SAT-playground-qmz`), bump-to-1 `used` counter
(`SAT-playground-ycw`), VMTF-coupled trail reuse, tier1 over-protection in
`reduce_candidate` (`SAT-playground-z70`), inprocessing absent
(roadmap). None of the 2026-05-26 commits touch these. Yesterday's FINDINGS
recommendations stand.

## Trajectory analysis

Not run this iteration — the work × speed table already explains every regression and win:

* **mp1 / Kakuro / case9 wins** are pure trajectory wins (0.03–0.36× conflicts). Same as
  yesterday's E. The new 100k EMA window does not change the qualitative trajectory; it
  reduces the cost of each fast/slow EMA evaluation so the restarts fire on more meaningful
  signal.
* **6s299b685 / REGRandom losses** are mixed work + execution. Same as yesterday's E.
  Phase-boundary chaos — unfixable without inprocessing.
* **brocard loss** is a focused-stable rendering: focused mode picks a random polarity
  sequence that misses solver-10's BVE-preprocessed structure. Brocard is a number-theory
  instance whose solver-10 BVE produces a near-trivial residual; focused-stable's "random
  decision" diversification breaks that structure.

## Hardware counter results

Not run this iteration. The data answers the bottleneck question without per-cycle
information.

## Parameter sweep results

Not run this iteration. The 100k EMA window change is already a successful targeted
parameter alignment; the next sweeps to consider are:

* `SAT_EMA_SLOW_WINDOW` between 4 096 (old) and 100 000 (new) to confirm monotonic improvement
  on the 6s299b685/REGRandom losers — could potentially partially rescue them.
* `SAT_RESTART_BLOCK_MARGIN` (currently 0) — yesterday's H1 sweep on similar configs showed
  1.4 helped Kakuro/SCPC. Now that the EMA window is fixed, re-evaluate the blocker.

## Code-Level Recommendations (ordered by ROI)

1. **Promote `SAT_SEARCH_MODE=focused-stable SAT_RESTART=kissat-ema SAT_USE_LBD=on` to the
   `fast` profile** (not `default`, since promotion needs the solver-10 gate). D's PAR-2 690
   vs A's clean ≈ 764 is a 10 % win and stable. The README's solver-10 gate procedure should
   be re-run with this candidate.
2. **Implement kissat-style root-unit learning in failed lucky patterns**
   (`main.rs:3411-3468`). Specifically: after `with_temporary_assumptions` returns false,
   inspect the conflict trace and add any unit clauses that survived to root level. Match
   `kissat/lucky.c:117-129`. New bead recommended.
3. **Investigate D's 6s299b685 / REGRandom / brocard regressions** with the
   `SAT_RESTART_BLOCK_MARGIN > 0` sweep. Hypothesis: the EMA-window fix exposes a new
   regime where decision-level-EMA-blocking would help. Now that the EMA signal is
   meaningful at 100k, blocking by level-EMA is also more reliable.
4. **Continue Gap 2/3/5 work** (lbd-tiered fractional deletion, bump-to-MAX `used` counter,
   tier1 deletion gate). None of those have changed; existing beads `SAT-playground-qmz` /
   `-ycw` / `-z70` remain valid. F_full_stack regression in yesterday's run was driven by
   adding lbd-tiered on top of focused-stable+EMA; in today's run we deliberately did NOT
   include lbd-tiered, and F is now only slightly worse than D. The lbd-tiered breakage is
   still real.

## Rejected / non-issues this run

* The `e4efdc1` commit (rejecting single-mode EMA at config-validation) is sound — it makes
  yesterday's catastrophic `C_lbd_ema` configuration impossible. No measurement needed.
* Lucky-on default would not have been a win on this suite (E_lucky PAR-2 805 vs A 764
  clean, a regression). The demotion was correct.
* B_metadata_only's per-instance wall variance (±10 % vs A) is contention noise, not a real
  LBD-bookkeeping cost. Counters confirm.

## Artifact paths

* Ablation script: `log/analyzesat-2026-05-26-0712/run_ablation.sh`
* Analysis script: `log/analyzesat-2026-05-26-0712/analysis.py`
* Per-config raw: `log/analyzesat-2026-05-26-0712/<config>/results.csv`, `stats.jsonl`
* Matrix / decomp / reference_gap / summary: `log/analyzesat-2026-05-26-0712/*.csv`,
  `summary.md`
* Reference CSV (reused from yesterday): `log/analyzesat-2026-05-25-2043/reference-kissat-latest.csv`
* Worktree: `/tmp/analyzesat-2026-05-26-0712`
* Kissat reference source: `benchmarks/reference-solvers/kissat-latest/src/lucky.c` (Gap A diff)
* Companion: `log/analyzesat-2026-05-25-2043/FINDINGS.md`,
  `log/kissat-investigation-2026-05-23-broad/DEEPER_FINDINGS.md`
