# Fresh search-feature efficacy re-evaluation — 2026-05-29/30 (bead SAT-playground-gbc)

Clean re-measurement of solver-11 search features after the prior (contaminated) verdicts were
archived to `solver/11-kissat-port/archive/efficacy-reeval-2026-05-29/` (do not consult unless asked).

## Methodology
- Suite `benchmarks/profiling` (10 instances), 300 s / 16 GiB. One fixed binary (HEAD cc55384,
  chrono-off single-mode default); every config is a runtime `SAT_*` toggle on that **same binary**.
- **Warm-up discarded** + **n=2 interleaved rounds** (each config once per round, not back-to-back)
  to average out the ~7.5 % / ~57-PAR-2 same-binary warming variance documented in bd memory
  `profiling-suite-longinstance-variance`.
- Decision metric: **aggregate PAR-2**. Baseline n=2 = 737.7 / 734.8 → **mean 736.2, spread 2.9**
  (tight → noise band ≈ 3 PAR-2). A config is a real effect only if its delta exceeds that band.

## Correctness
**Clean.** All solved rows verified (`ok`/`no-checker`); every unsolved row is an **honest TIMEOUT**
(no wrong SAT/UNSAT, no ERROR, no premature non-budget UNKNOWN). All regressions are priced-in PAR-2
costs, not correctness failures.

## Results (n=2 mean PAR-2, Δ vs baseline 736.2)
| config | env | r1 | r2 | mean | Δ | timeouts | verdict |
|---|---|---:|---:|---:|---:|---:|---|
| baseline | (chrono-off single-mode) | 737.7 | 734.8 | 736.2 | — | 0 | reference |
| **lucky** | `SAT_LUCKY=on` | 726.0 | 723.4 | **724.7** | **−11.5** | 0 | **candidate WIN (confirm)** |
| chrono | `SAT_CHRONO=on` | 734.0 | 740.7 | 737.4 | +1.2 | 0 | neutral |
| fstab_lbdtier | focused-stable + `SAT_REDUCE=lbd-tiered` | 918.2 | 914.7 | 916.5 | +180.3 | 0 | regress |
| fstab | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on` | 928.0 | 925.0 | 926.5 | +190.3 | 0 | regress |
| binfast | `SAT_BINARY_FAST=on` | 1099.4 | 1099.7 | 1099.6 | +363.3 | 0 | regress |
| combo_chrono_binfast | `SAT_CHRONO=on SAT_BINARY_FAST=on` | 1517.5 | 1520.1 | 1518.8 | +782.6 | 1/1 | regress |
| fstab_vsids | fstab + `SAT_VMTF=off` | 1770.7 | 1770.5 | 1770.6 | +1034.4 | 2/2 | regress |
| fstab_full | fstab + `SAT_REDUCE=lbd-tiered SAT_REPHASE=on` | 1844.8 | 1841.6 | 1843.2 | +1107.0 | 2/2 | regress |
| inblock | `SAT_CLAUSE_MIN=inblock` | 3409.5 | 3407.4 | 3408.4 | +2672.2 | 5/5 | catastrophic |

## Per-feature verdicts
- **`SAT_LUCKY=on` — the ONLY net win (−11.5 PAR-2), but narrow.** Per-instance: battleship
  22.9 s → **0.1 s (−22.8, deterministic lucky solve)**, every other instance **+0.0…+2.6** (probe
  overhead), totalling +11.3. The net win exists *only because* a lucky-solvable instance
  (battleship) is in the suite; on a suite without one, lucky would be net-negative. Under the
  aggregate-PAR-2-only policy this is a legitimate win on **this** suite, but it is
  **suite-composition-dependent / single-instance-driven** — promote only after **n≥3 confirmation,
  a shuffle/suite-robustness check, and the solver-10 + `check_solver11_promotion.py` gate**. (This
  reverses the archived "demoted" verdict, which used the retired per-instance-no-regression bar.)
- **`SAT_CHRONO=on` — neutral** (+1.2, within noise). Reconfirms bead 59l.
- **`SAT_BINARY_FAST=on` — clear reproducible regression** (+363; both runs 1099).
- **`SAT_CLAUSE_MIN=inblock` — catastrophic** (+2672, 5/5 timeouts both rounds). Matches the open
  bug `prz` (CCMIN_INBLOCK `same_level_only=true`). Inblock is unusable on this suite as-is.
- **Focused-stable stack (`fstab`) — regresses ~+190** vs single-mode (10/10 solved). Within it:
  - **`SAT_VMTF` HELPS focused-stable** (fstab 926.5 with VMTF vs fstab_vsids 1770.6 without, +2 TO) —
    VMTF is beneficial, but only inside a mode that itself loses to the single-mode default.
  - `SAT_REDUCE=lbd-tiered` ≈ neutral within focused-stable (916.5 vs 926.5).
  - **`SAT_REPHASE` destabilizes** (fstab_full 1843 vs fstab_lbdtier 916.5 = +927 + 2 TO).
- **Combos do not help.** chrono+binfast (1518.8) is worse than either single; fstab_full is worse
  than its parts. No combination beat any of its components, and none approached baseline.

## Conclusion
**The chrono-off single-mode default (≈736 PAR-2) is the best configuration tested.** Of the
implemented search features, only `SAT_LUCKY` produces a net aggregate-PAR-2 improvement, and it is
narrow/suite-dependent; chrono is neutral; every other single and every combination regresses,
several catastrophically (inblock, the focused-stable + rephase/VSIDS variants). This cleanly
reconfirms the long-standing result that the kissat-class feature stack loses to single-mode on this
suite — now with sound methodology rather than contaminated single runs.

## Next steps
1. **lucky:** n≥3 confirmation + shuffle/suite-robustness + solver-10/promotion gate; decide whether
   a battleship-driven net win is worth promoting given suite-composition fragility.
2. **inblock:** fix bug `prz` before any inblock re-evaluation.
3. Record these verdicts in the live `FEATURES.csv` (done). Untested rows remain `ReevalPending`.

Raw: `summary.tsv`, `<tag>/r<round>/results.csv`. Driver/plan: `run_ablation.sh`, `PLAN.md`.
