# 1.12a Advanced Search Candidate Milestone

Bead: `SAT-playground-5b2.2.16`

Date: 2026-05-21

## Scope

This was a performance-gate bead, not a solver-code bead. The goal was to evaluate whether the
composed Phase 1 search stack should replace the earlier 1.8 core profile after the focused/stable
mode, VMTF, clause minimization, binary-fast propagation, and rephase hook existed.

No solver source code was changed for this bead.

## Graph Selection

`bv --robot-plan` placed `SAT-playground-5b2.2.16` as the next dependency-respecting Phase 1 item.
`bv --robot-insights` showed it directly blocks `SAT-playground-5b2.5.5` (`[M.4]
Mode/VMTF/minimize/rephase advanced-search milestone`). Its dependencies (`1.8`, `1.9`, `1.10`,
`1.11`, and `1.12`) were already closed.

## Candidate Results

### Focused/VMTF Candidate

Command artifact:

`log/1.12a/candidate-focused-vmtf/results.csv`

Configuration artifact:

`log/1.12a/candidate-focused-vmtf.config`

Key settings:

- `SAT_PROFILE=experimental`
- `SAT_RESTART=kissat-ema`
- `SAT_REDUCE=lbd-tiered`
- `SAT_PHASE=target-then-saved`
- `SAT_BINARY_FAST=on`
- `SAT_SEARCH_MODE=focused-stable`
- `SAT_CLAUSE_MIN=basic`
- `SAT_VMTF=on`
- `SAT_REPHASE=off`
- `SAT_CHRONO=off`

Result on `benchmarks/iteration/search-core` with 120s timeout:

- Solved: 1/9 (`battleship-16-31-sat`)
- Unsolved: 8/9 (7 timeout, 1 unknown)
- PAR-2: 1920.781

Comparison against the saved-phase reference (`log/1.5/search-core-saved/results.csv`):

- Saved-phase: 5/9 solved, PAR-2 1188.569
- Focused/VMTF: 1/9 solved, PAR-2 1920.781
- Delta: +732.212 PAR-2
- Status regressions:
  - `544707209399nw.shuffled-as.sat03-1671`: SAT -> TIMEOUT
  - `SC25_Timetable_C_392`: SAT -> TIMEOUT
  - `SC25_Timetable_C_406`: SAT -> TIMEOUT
  - `mp1-Nb7T46`: SAT -> TIMEOUT
- Compare verdict: FAIL, `promotion_verdict=significant_regression`

Comparison against the earlier VMTF run (`log/1.10/search-core-vmtf/results.csv`):

- 1.10 VMTF: 4/9 solved, PAR-2 1258.395
- Focused/VMTF: 1/9 solved, PAR-2 1920.781
- Delta: +662.386 PAR-2
- Status regressions:
  - `544707209399nw.shuffled-as.sat03-1671`: SAT -> TIMEOUT
  - `DLTM_twitter845_79_19`: SAT -> TIMEOUT
  - `mp1-Nb7T46`: SAT -> TIMEOUT
- Compare verdict: FAIL, `promotion_verdict=significant_regression`

Comparison against the 1.8 conservative search-core run
(`log/1.8/search-core-conservative/results.csv`):

- 1.8 conservative: 1/9 solved, PAR-2 1960.600
- Focused/VMTF: 1/9 solved, PAR-2 1920.781
- Delta: -39.819 PAR-2
- Status changed in both directions:
  - `battleship-16-31-sat`: TIMEOUT -> SAT
  - `544707209399nw.shuffled-as.sat03-1671`: SAT -> TIMEOUT
- Compare verdict: FAIL because a prior solved row regressed.

### Stable-SAT Exploratory Candidate

The stable-SAT exploratory candidate was covered by the direct rephase A/B artifacts:

- Rephase off: `log/1.12a/ab-rephase-off/results.csv`
- Rephase on: `log/1.12a/ab-rephase-on/results.csv`

The rephase-on run matches the planned exploratory stack except for the artifact path:

- `SAT_PHASE=best-then-target-then-saved`
- `SAT_BINARY_FAST=on`
- `SAT_SEARCH_MODE=focused-stable`
- `SAT_CLAUSE_MIN=recursive-limited`
- `SAT_VMTF=on`
- `SAT_REPHASE=on`
- `SAT_CHRONO=off`

Result on `benchmarks/iteration/search-core` with 120s timeout:

- Rephase off: 1/9 solved, PAR-2 1926.486
- Rephase on: 0/9 solved, PAR-2 2160.000
- Delta: +233.514 PAR-2
- Status regression:
  - `battleship-16-31-sat`: SAT in 6.486s -> TIMEOUT
- Compare verdict: FAIL, `promotion_verdict=significant_regression`

Comparison against the saved-phase reference:

- Saved-phase: 5/9 solved, PAR-2 1188.569
- Stable-SAT exploratory: 0/9 solved, PAR-2 2160.000
- Delta: +971.431 PAR-2
- Status regressions:
  - `544707209399nw.shuffled-as.sat03-1671`: SAT -> TIMEOUT
  - `SC25_Timetable_C_392`: SAT -> TIMEOUT
  - `SC25_Timetable_C_406`: SAT -> TIMEOUT
  - `battleship-16-31-sat`: SAT -> TIMEOUT
  - `mp1-Nb7T46`: SAT -> TIMEOUT

## Promotion Decision

Do not promote either advanced candidate.

Rationale:

- Focused/VMTF does not beat the saved-phase reference on search-core.
- Focused/VMTF regresses solved instances versus both saved-phase and the earlier VMTF-only run.
- Stable-SAT exploratory with rephase is worse than the same stack with rephase disabled.
- Both candidates fail the bead's "without proof/model failures and no solved-row regressions" rule
  at the first required search-core gate.

Because the search-core gate failed, discriminating and regression-guard suites were intentionally
not run for these candidates. Running downstream promotion suites after a first-gate solved-row
regression would not change the promotion decision.

This also means the 1.13 chronological-backtracking bead should not be skipped under its documented
skip rule: 1.12a did not meet the Phase 1 target on search-core, discriminating, and
regression-guards.

## Fresh-Eyes Review

- The candidate benchmark did not modify solver code, so there was no implementation diff to
  inspect.
- The solved candidate rows had no SAT model failures or proof failures in `tools/bench.sh`.
- `compare_bench.py` reported no correctness failures for all candidate comparisons.
- The rephase A/B compared only the intended rephase toggle and artifact-path/config-hash changes.
- The main issue is search-trajectory sensitivity: rephase-on loses the `battleship-16-31-sat`
  solve, while Focused/VMTF trades away several earlier solved rows for one very fast battleship
  solve. That is not acceptable for profile promotion.

## Validation

Unit and smoke checks:

- `cargo test` in `solver/11-kissat-port`: 239 passed, 0 failed.
- `bash tools/smoke_test.sh solver/11-kissat-port`: 9 passed, 0 failed.
- `SAT_CHECK_INVARIANTS=1 bash tools/smoke_test.sh solver/11-kissat-port`: 9 passed, 0 failed.

Profile no-regression benchmark:

- Current run: `log/1.12a/profile-after/results.csv`
- Previous comparison point: `log/1.12/profile-after/results.csv`
- Before: 9/11 solved, PAR-2 713.617
- After: 9/11 solved, PAR-2 713.322
- Delta: -0.295 PAR-2
- Status regressions: none
- Compare verdict: PASS

The default profile remains non-regressing. The rejected advanced candidates remain opt-in and are
not promoted into the default profile.
