# Search-feature efficacy re-evaluation — 2026-05-29 (bead SAT-playground-gbc)

Fresh, methodologically-sound efficacy measurement of solver-11 search features.
Prior verdicts were archived (see `solver/11-kissat-port/archive/efficacy-reeval-2026-05-29/`,
do not consult unless asked) because they were contaminated by host contention, cold-cache
first-run slowness, the ~7.5% same-binary warming variance, and single-run noise.

## Methodology
- Suite: `benchmarks/profiling` (10 instances), 300 s / 16 GiB per instance.
- One fixed solver-11 binary (HEAD cc55384, chrono-off single-mode default); every config is a
  runtime `SAT_*` env toggle on that SAME binary (no rebuild between configs → isolates the feature
  effect from codegen/layout).
- **Warm-up:** one full discarded run first (absorbs cold-cache / first-run penalty).
- **n=2 via interleaved rounds:** each config is run once in round 1 and once in round 2 (NOT
  back-to-back), so the ~7.5% warming/drift is averaged across configs rather than confounding any
  single config.
- Decision metric: **aggregate PAR-2 over the suite**. A feature/combo is a *candidate win* only if
  its 2-round mean is below the baseline's 2-round mean by more than the run-to-run noise band, AND
  it does not introduce a wrong result / premature non-budget UNKNOWN. Solver-10 (699.671) is an
  informational floor.
- Screening pass: n=2 detects large effects and flags borderline ones; promising configs get an
  n>=3 confirmation follow-up before any promotion (which still requires the full
  `check_solver11_promotion.py` gate).

## Config matrix (same binary, env toggle)
| tag | env |
|---|---|
| baseline | (none — chrono-off single-mode default) |
| chrono | `SAT_CHRONO=on` |
| binfast | `SAT_BINARY_FAST=on` |
| lucky | `SAT_LUCKY=on` |
| inblock | `SAT_CLAUSE_MIN=inblock` |
| fstab | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on` |
| fstab_vsids | fstab + `SAT_VMTF=off` (VMTF ablation) |
| fstab_lbdtier | fstab + `SAT_REDUCE=lbd-tiered` |
| combo_chrono_binfast | `SAT_CHRONO=on SAT_BINARY_FAST=on` |
| fstab_full | fstab + `SAT_REDUCE=lbd-tiered SAT_REPHASE=on` |

## Outputs
- `summary.tsv` — round, tag, par2, solved, unsolved, env (appended as runs finish).
- `<tag>/r<round>/results.csv` — per-instance results for each (config, round).
- Analysis (means, vs-baseline deltas, correctness flags) written to `FINDINGS.md` after completion.
