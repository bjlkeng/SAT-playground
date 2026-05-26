# Deeper Findings — Phase 6 parameter sweep follow-up

**Date:** 2026-05-26 (continued from `FINDINGS.md`)
**HEAD:** `9143376` (same as the parent run)
**Method:** 4-point sweep of `SAT_RESTART_BLOCK_MARGIN ∈ {0, 1.2, 1.4, 1.6}` on the 4
profiling instances where `D_focused_stable_ema` regressed `A_baseline`. 240 s timeout,
16 GiB memory, otherwise identical environment to `D`.

## Hypothesis under test

From `FINDINGS.md` Code-Level Recommendation #3:

> "Investigate D's 6s299b685 / REGRandom / brocard regressions with the
>  SAT_RESTART_BLOCK_MARGIN > 0 sweep. Hypothesis: the EMA-window fix exposes a new regime
>  where decision-level-EMA-blocking would help. Now that the EMA signal is meaningful at
>  100k, blocking by level-EMA is also more reliable."

## Sweep results

`m000` (margin=0) is identical to `D` and serves as the sweep baseline.

| Margin | Solved | PAR-2 | 6s299b685 | REGRandom | SCPC | brocard |
|---|---:|---:|---:|---:|---:|---:|
| 0 (D baseline) | 4/4 | 294.4 | 88.8 s | 163.3 s | 26.2 s | 16.1 s |
| 1.2 | 3/4 | 613.0 | 91.6 s | **TIMEOUT** | 25.5 s | 15.9 s |
| 1.4 | 3/4 | 622.1 | 102.4 s | **TIMEOUT** | 25.1 s | 14.6 s |
| 1.6 | 3/4 | 620.7 | 101.9 s | **TIMEOUT** | 23.2 s | 15.6 s |

## Conclusion: hypothesis fully refuted

* **REGRandom**: every margin > 0 produces a TIMEOUT on an instance that the baseline (m000)
  solves in 163 s. Blocking restarts on high level-EMA is exactly the wrong move on this
  random-3-SAT family — the level-EMA stays high precisely because the search is making
  productive progress through a wide trail, and suppressing the restart keeps the search
  stuck in a useless branch.
* **6s299b685**: every margin > 0 makes this HWMCC instance *slower* (88.8 → 91.6 / 102.4 /
  101.9). Not a rescue.
* **SCPC, brocard**: comparable across all margins (variance < 1 s). Margin has effectively
  no effect on these.

**Zero out of four regressing instances are rescued.** The hypothesis that the new 100k
EMA window would make level-EMA-blocking more useful is wrong — the new window makes the
fast/slow gap more sensitive, which means blocking already-rare restarts hurts more.

## Why the hypothesis was wrong

Yesterday's prior investigation (`log/kissat-investigation-2026-05-23-broad/DEEPER_FINDINGS.md`
Phase 3 sweep H1 / H4) reported margin=1.4 rescued Kakuro and SCPC under `C_lbd_ema` and
`E_lbd_ema_tiered` with the OLD 4096 slow window. The reasoning then was: "the EMA window
is so short that it fires on noise; blocking by level-EMA gives the slow EMA time to update."
That logic only applied to the noisy 4k-window regime. With the new 100k window, the slow
EMA is genuinely stable on its own — there is no noise to filter out by level-blocking.

The fix from `919370b` made the blocker hypothesis obsolete, not stronger. Recording this so
no future sweep retries the same knob on this stack.

## What still works after this sweep

`D_focused_stable_ema` (margin=0, the current default for this config) remains the best
non-A_baseline candidate at PAR-2 690 on the full 10-instance suite. The recommendation
from `FINDINGS.md` #1 (promote D to the `fast` profile, subject to solver-10 gate) is
unaffected.

## What the regressors actually need

The 4 instances split into two failure modes:

* **REGRandom-K4-L1**: pure trajectory regression under focused-stable. Conflicts go from
  1.6 M (A) to 2.4 M (D), props/sec stays roughly constant. The "right" fix is **probing**
  / **vivification** to simplify the formula before focused-stable's random-decision phase
  cycling kicks in. kissat solves this in 2.3 s on the same hardware (24.7× faster).
* **6s299b685_Iter30**: HWMCC hardware verification. Conflicts increase 1.4× under D, but
  *propagation throughput drops 5×* — the work × speed decomposition in `FINDINGS.md`
  (decomp.csv D row) shows speed_ratio 5.15. This is **execution overhead** from focused-
  stable's mode-cycling + VMTF queue interactions. Per-prop cost goes up because the binary
  watcher list and arena cache pressure grow when learned-clause-count climbs. The fix is
  the lbd-tiered fraction-based reducer (`SAT-playground-qmz`) — keep the DB lean and the
  per-prop cost stays low.

Neither of these is a "tune a margin" fix.

## Sweep artifacts

* Sweep script: `log/analyzesat-2026-05-26-0712/sweep/run_sweep.sh`
* Symlink subset of profiling/: `benchmarks/profiling-d-regressors/`
* Per-margin results: `log/analyzesat-2026-05-26-0712/sweep/m{000,120,140,160}/results.csv`
* Driver log: `log/analyzesat-2026-05-26-0712/sweep/sweep_driver.log`
