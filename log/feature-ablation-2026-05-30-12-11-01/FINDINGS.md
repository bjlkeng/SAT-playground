# profile20 Stage-1 feature ablation — FINDINGS (2026-05-30/31)

First feature ablation on the new **profile20** suite (10 easy controls reused from
`benchmarks/profiling` + 10 hard "headroom" instances solver-10 can't solve in 5 min but kissat
can). Driver: `tools/feature_ablation.py`. Conditions: 300 s / 14 GiB, 4 workers pinned to physical
cores 0–3 (`taskset`), one `bench.sh -j1` shard each; all-20 aggregate PAR-2 with easy-10 / hard-10
reported separately; 3% repeat rule (within ±3 % of baseline → one confirming rerun; clear win/loss
accepted from n=1).

## Headline

**`fstab_lbdtier` (focused-stable + VMTF + lbd-tiered reduce) wins by 2×: −1155 PAR-2 (5653 vs
baseline 6808), 13/20 solved vs baseline's 10.** The all-easy `benchmarks/profiling` suite could not
have surfaced this — these same configs looked like regressors there; the headroom half flipped the
verdict, validating the profile20 process change.

## Correctness

Clean. 35 ERROR rows across the run, **all** the single hard instance `2018D_VexRiscv-regch0-20`
hitting the 14 GiB/job memory cap (a ~5 GB allocation aborts; honest resource failure, PAR-2-neutral
since it scores as unsolved like a timeout either way). **Zero** non-VexRiscv errors; no wrong
SAT/UNSAT, no invalid model, no premature UNKNOWN.

## Results (means where reruns exist; baseline 6807.6, 3% band ±204)

| config | all-20 | easy-10 | hard-10 | Δ all-20 | solved | verdict |
|---|---:|---:|---:|---:|:--:|---|
| fstab_lbdtier | 5653.1 | 1049.7 | 4603.4 | −1155 | 13 | **WIN** |
| reduce_tier2 | 6298.4 | 1806.8 | 4491.6 | −509 | 11 | WIN |
| lbd_tiered | 6299.4 | 1805.2 | 4494.2 | −508 | 11 | WIN |
| watch_compact | 6313.8 | 1823.0 | 4490.8 | −494 | 11 | WIN |
| fstab | 6433.6 | 1027.3 | 5406.3 | −374 | 11 | WIN |
| fstab_full | 6731.7 | 2138.8 | 4592.9 | −76 | 10 | neutral |
| binary_fast | 6764.8 | 1128.6 | 5636.2 | −43 | 11 | neutral (1 hard crack) |
| solver10 (floor) | 6773.2 | 773.2 | 6000 | −34 | 10 | reference |
| chrono / lucky / restart_reuse_trail / restart_reuse_chrono / lucky_chrono / otfs | ~6798–6803 | — | 6000 | −4…−9 | 10 | neutral |
| baseline | 6807.6 | 807.6 | 6000 | — | 10 | reference |
| fstab_rephase | 6830.6 | 1425.8 | 5404.8 | +23 | 10 | neutral |
| use_lbd / lbd_update_reasons / lbd_update_pair | 6872–6908 | — | 6000 | +64…+101 | 10 | mild regress |
| fstab_novmtf | 7229.7 | 1821.8 | 5407.9 | +422 | 9 | regress |
| reorder | 7246.1 | 1246.1 | 6000 | +439 | 9 | regress |
| otfs_otss | 7776.2 | 1776.2 | 6000 | +969 | 8 | regress |
| otss | 7779.4 | 1779.4 | 6000 | +972 | 8 | regress |

### inblock supplementary pass (separate run, same conditions; baseline re-measured 6799.7)

| config | all-20 | easy-10 | hard-10 | Δ | solved | verdict |
|---|---:|---:|---:|---:|:--:|---|
| inblock | 8157.5 | 2727.1 | 5430.4 | +1358 | 7 | **regress** |
| inblock_otfs_otss | 8662.0 | 3203.2 | 5458.8 | +1862 | 6 | **regress** |

## Mechanism (what actually moves the needle)

- **The win is `SAT_REDUCE=lbd-tiered`.** Alone it cracks **3 hard instances** — `circuit` (SAT 12 s),
  `div-mitern172` (UNSAT 136 s), `sqrt-mitern171` (UNSAT 145 s) → +2 solved, hard 6000→4494.
  `reduce_tier2` and `watch_compact` add nothing measurable on top of plain `lbd_tiered`.
- **`focused-stable + VMTF` is the amplifier.** It cuts the easy-half overhead from ~1805 (lbd_tiered)
  to ~1050, so `fstab_lbdtier` keeps all 3 cracks at ~half the easy-half tax → −1155, 13/20, 2× the
  next best. Cracks circuit even faster (4 s).
- **Confirmed negatives.** VMTF is load-bearing inside focused-stable (removing it: +422, −2 solved);
  rephase consistently hurts (fstab_rephase +23, fstab_full collapses to −76 vs fstab_lbdtier's −1155);
  LBD-update reason flags are pure overhead; `otss`/`otfs_otss` are the worst single features here.
- **`binary_fast`** cracks only `circuit` and regresses the easy half → net neutral (−43); different
  crack path from the tiered configs (reaches nothing they miss at 300 s).
- **`inblock` (even with the SH-A/SH-B fix, 5a01032)** cracks only `circuit` but **destroys the easy
  half** — loses 4 easy instances baseline solves (velev, mp1, sudoku, REGRandom); +otfs/otss loses 5.
  Confirms bug `prz`'s "stays off-default" call on profile20; open shrink follow-ups `8ch`/`0cu` would
  need to land before re-testing.

## Stage-2 shortlist (long timeout, hard-10)

**fstab_lbdtier, lbd_tiered, binary_fast** at 900 s on the hard-10, `-j3 @ 18 GiB` (more memory so
VexRiscv doesn't OOM), to measure how many more instances each reaches with real headroom budget.
inblock is NOT shortlisted (clear regressor).

## Caveats before any promotion

- The 5 band-clearing winners are **n=1** (their Δ exceeded the 3% band so the rule didn't trigger a
  rerun). Stage 2's repeated long-timeout runs are the confirmation.
- This is **screening**, not a promotion decision: no shuffle-sensitivity check, no
  `check_solver11_promotion.py` gate yet. FEATURES.csv verdicts intentionally NOT updated from this
  pass.

Provenance: `summary.tsv` (per-config/per-run PAR-2), `STAGE1.md` (auto-generated table), and the
inblock pass at `log/feature-ablation-2026-05-31-08-04-39/`. Per-instance `results.csv` retained
locally under each `<tag>/r<run>/`.
