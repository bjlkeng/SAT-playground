# Stage-1 feature ablation on profile20 (2026-05-30-12-11-01)

Baseline (solver-11 default) all-20 PAR-2 = **6807.6**; 3% band = +/-204.2. solver-10 floor all-20 = 6773.2.

| config | all-20 Δ | all-20 | easy-10 | hard-10 | n |
|---|---:|---:|---:|---:|---:|
| fstab_lbdtier | -1154.5 WIN | 5653.1 | 1049.7 | 4603.4 | 1 |
| reduce_tier2 | -509.2 WIN | 6298.4 | 1806.8 | 4491.6 | 1 |
| lbd_tiered | -508.2 WIN | 6299.4 | 1805.2 | 4494.2 | 1 |
| watch_compact | -493.8 WIN | 6313.8 | 1823.0 | 4490.8 | 1 |
| fstab | -374.0 WIN | 6433.6 | 1027.3 | 5406.3 | 1 |
| fstab_full | -75.9 neutral | 6731.7 | 2138.8 | 4592.9 | 2 |
| binary_fast | -42.7 neutral | 6764.8 | 1128.6 | 5636.2 | 2 |
| solver10 | -34.4 neutral | 6773.2 | 773.2 | 6000.0 | 1 |
| chrono | -9.1 neutral | 6798.5 | 798.5 | 6000.0 | 2 |
| lucky | -6.5 neutral | 6801.1 | 801.1 | 6000.0 | 2 |
| restart_reuse_trail | -5.9 neutral | 6801.7 | 801.7 | 6000.0 | 2 |
| restart_reuse_chrono | -5.3 neutral | 6802.3 | 802.3 | 6000.0 | 2 |
| lucky_chrono | -5.3 neutral | 6802.3 | 802.3 | 6000.0 | 2 |
| otfs | -4.3 neutral | 6803.3 | 803.3 | 6000.0 | 2 |
| baseline | +0.0 — | 6807.6 | 807.6 | 6000.0 | 2 |
| fstab_rephase | +23.0 neutral | 6830.6 | 1425.8 | 5404.8 | 2 |
| use_lbd | +64.2 neutral | 6871.8 | 871.8 | 6000.0 | 2 |
| lbd_update_reasons | +73.4 neutral | 6881.0 | 881.0 | 6000.0 | 2 |
| lbd_update_pair | +100.6 neutral | 6908.2 | 908.2 | 6000.0 | 2 |
| fstab_novmtf | +422.1 regress | 7229.7 | 1821.8 | 5407.9 | 1 |
| reorder | +438.5 regress | 7246.1 | 1246.1 | 6000.0 | 1 |
| otfs_otss | +968.6 regress | 7776.2 | 1776.2 | 6000.0 | 1 |
| otss | +971.8 regress | 7779.4 | 1779.4 | 6000.0 | 1 |

## Proposed Stage-2 shortlist (long-timeout, hard-10)
binary_fast, lbd_tiered, reduce_tier2, watch_compact, fstab, fstab_novmtf, fstab_lbdtier, fstab_rephase, fstab_full
