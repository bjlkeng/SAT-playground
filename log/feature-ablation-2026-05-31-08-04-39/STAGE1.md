# Stage-1 feature ablation on profile20 (2026-05-31-08-04-39)

Baseline (solver-11 default) all-20 PAR-2 = **6799.7**; 3% band = +/-204.0. (solver-10 floor not in this subset run)

| config | all-20 Δ | all-20 | easy-10 | hard-10 | n |
|---|---:|---:|---:|---:|---:|
| baseline | +0.0 — | 6799.7 | 799.7 | 6000.0 | 2 |
| inblock | +1357.9 regress | 8157.5 | 2727.1 | 5430.4 | 1 |
| inblock_otfs_otss | +1862.3 regress | 8662.0 | 3203.2 | 5458.8 | 1 |

## Proposed Stage-2 shortlist (long-timeout, hard-10)
inblock, inblock_otfs_otss
