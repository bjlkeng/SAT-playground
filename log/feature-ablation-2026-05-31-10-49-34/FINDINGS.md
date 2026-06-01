# Sweep-2 feature ablation on profile20 — FINDINGS (2026-05-31 → 06-01)

Second feature ablation: combine the **promoted `fstab_lbdtier` default** (focused-stable + LBD +
ticks + lbd-tiered reduce + vmtf-focused + lucky) with other features, on the hypothesis that
interactions could change dramatically vs the isolated single-mode results of sweep-1.

Driver `tools/feature_ablation.py --sweep2`. Conditions: profile20 (20 instances), **600 s**, -j4
pinned to physical cores 0–3, 14 GiB/job. Reference = the new `default` (empty env). Single-stage on
all 20; 3% repeat rule (within ±band → confirming rerun). **Stopped early by user** after all 26
first passes + partial reruns; the verdict was locked (no within-band rerun shifted any verdict).

## Headline

**No feature or combo beats the promoted default beyond the ±295 PAR-2 noise band.** The default
(9835.3 all-20, 13/20 solved) is a robust local optimum on profile20. Best non-default is
`reduce_tier2` at −0.9 — pure noise (it is already implied by the default's lbd-tiered policy).

## Correctness

Clean. All ERROR rows are the single hard instance `2018D_VexRiscv-regch0-20` hitting the 14 GiB/job
cap (OOM → PAR-2-neutral, scored as unsolved like a timeout). Zero non-VexRiscv errors; no wrong
result, no invalid model/proof.

## Results (Δ vs default reference 9835.3; band ±295)

| config | all-20 | easy-10 | hard-10 | Δ | solved | per-instance vs default |
|---|---:|---:|---:|---:|:--:|---|
| reduce_tier2 | 9834.4 | 1027.6 | 8806.8 | −0.9 | 13 | (no-op; already in default) |
| default (ref) | 9835.3 | 1029.3 | 8806.0 | — | 13 | — |
| ema | 9837.5 | 1028.3 | 8809.3 | +2.2 | 13 | — |
| reluctant | 9837.7 | 1029.0 | 8808.7 | +2.3 | 13 | — |
| chrono_ema | 9889.1 | 1084.4 | 8804.7 | +53.8 | 13 | — |
| chrono | 9890.5 | 1083.0 | 8807.5 | +55.2 | 13 | — |
| reuse_trail | 9942.6 | 2013.1 | 7929.6 | +107.3 | 13 | +sqrt-mitern170 −REGRandom |
| ema_reuse | 9956.7 | 2014.2 | 7942.4 | +121.4 | 13 | +sqrt-mitern170 −REGRandom |
| lbd_update_pair | 9973.5 | 1928.5 | 8044.9 | +138.2 | 13 | +PancakeVsSelectionSort −case9 |
| ema_target | 9985.9 | 2155.5 | 7830.4 | +150.6 | 13 | +PancakeVsSelectionSort −REGRandom |
| target_phase | 9987.6 | 2151.8 | 7835.8 | +152.3 | 13 | +PancakeVsSelectionSort −REGRandom |
| best_phase | 9989.4 | 2156.9 | 7832.5 | +154.1 | 13 | +PancakeVsSelectionSort −REGRandom |
| otfs | 10117.7 | 1356.6 | 8761.2 | +282.4 | 13 | — |
| reuse_focused | 10362.3 | 1544.8 | 8817.5 | +527.0 | 13 | — |
| lbd_update_reasons | 10839.7 | 2111.3 | 8728.5 | +1004.4 | 12 | −case9 |
| reorder | 11012.1 | 2202.4 | 8809.6 | +1176.8 | 12 | −case9 |
| rephase | 11015.1 | 2224.3 | 8790.8 | +1179.8 | 12 | −case9 |
| otss | 11026.0 | 2284.3 | 8741.6 | +1190.6 | 12 | −case9 |
| reuse_stable | 11096.9 | 2393.6 | 8703.3 | +1261.5 | 12 | −case9 |
| target_rephase | 11097.7 | 2330.3 | 8767.4 | +1262.4 | 12 | −case9 |
| ema_target_rephase | 11101.4 | 2335.8 | 8765.6 | +1266.1 | 12 | −case9 |
| solver10 (floor) | 11301.1 | 755.6 | 10545.6 | +1465.8 | 12 | — |
| watch_compact | 11912.9 | 3102.4 | 8810.5 | +2077.6 | 11 | −REGRandom −case9 |
| otfs_otss | 12241.9 | 3441.9 | 8800.0 | +2406.6 | 11 | −mp1 −case9 |
| binfast_tier2 | 12448.7 | 3583.5 | 8865.3 | +2613.4 | 11 | −mp1 −case9 |
| binary_fast | 12448.9 | 3590.3 | 8858.6 | +2613.6 | 11 | −mp1 −case9 |

(PAR-2 is higher in absolute terms than sweep-1 only because unsolved rows cost 2×600=1200 at this
timeout; compare within-sweep vs the `default` reference, not across sweeps.)

## Cross-cutting findings

1. **No synergy anywhere.** Every combo ≈ its dominant component: ema_target≈target_phase,
   ema_reuse≈reuse_trail, chrono_ema≈chrono, binfast_tier2≈binary_fast. The combos that add rephase
   inherit rephase's regression.
2. **kissat-ema / reluctant restarts are completely inert** on this base — neutral alone and in every
   combo. Restart *policy* is not the bottleneck on this suite.
3. **The default is a robust local optimum.** Restart/phase/reuse features only **swap** which
   instances solve (Pancake ↔ REGRandom ↔ sqrt170 ↔ case9) at a roughly fixed total; they perturb the
   trajectory without expanding capability. `case9` is the canary — 8 perturbing features lose it.
4. **Interaction flips (sweep-1 → sweep-2):** binary_fast (−43 → +2614), otss (regress → worse),
   watch_compact (~neutral → +2078), lbd_update_reasons (+64 → +1004) all turned strongly harmful on
   the focused-stable + lbd-tiered base. Features that were harmless/neutral on single-mode collide
   with the new default's DB/restart machinery. (Deep-dive on chrono/binary_fast/ema/target_phase —
   trigger-rate + code-level — is the follow-up companion analysis.)

## Conclusion

The promoted `fstab_lbdtier` default stands unchanged; no code change from this sweep. Provenance:
`summary.tsv`, `STAGE1.md` (partial — campaign stopped before sentinel), per-config `<tag>/r<run>/results.csv`.
