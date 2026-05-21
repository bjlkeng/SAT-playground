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

## Reopened 300s Profile Reevaluation

Reopened on 2026-05-21 at user request after changing the profiling-suite default timeout from
120s to 300s.

Fresh command:

```bash
bash tools/bench.sh -m 16384 -d benchmarks/profiling \
  --log-dir log/1.12a/profile-after-300-reopen \
  solver/11-kissat-port
```

This intentionally omits `-t` so `tools/bench.sh` must apply the new `benchmarks/profiling`
default timeout. The harness printed `Timeout: 300s`.

Result:

- Current run: `log/1.12a/profile-after-300-reopen/results.csv`
- Timeout: 300s
- Solved: 11/11 (7 SAT, 4 UNSAT)
- Unsolved: 0
- PAR-2: 624.808
- Correctness failures: none

Comparison against the earlier 120s bead profile (`log/1.12a/profile-after/results.csv`) with
`compare_bench.py --timeout 300`:

- Before: 9/11 solved, PAR-2 713.322
- After: 11/11 solved, PAR-2 624.808
- Delta: -88.514 PAR-2
- Status regressions: none
- Newly solved:
  - `0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12`: TIMEOUT -> UNSAT in 180.975s
  - `5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7`: TIMEOUT -> SAT in 210.712s
- Compare verdict: PASS

Comparison against the earlier 300s default run
(`log/profile-default-300-solver11-2026-05-21/results.csv`):

- Before: 11/11 solved, PAR-2 627.579
- After: 11/11 solved, PAR-2 624.808
- Delta: -2.771 PAR-2
- Status regressions: none
- Compare verdict: PASS

Interpretation:

The 300s profile reevaluation confirms the current default-profile solver completes both rows that
were previously capped at 120s. This improves the profiling-suite solved count from 9/11 to 11/11
under the new default timeout. This does not change the 1.12a advanced-search candidate promotion
decision: the candidate search-core configurations still regress solved rows and remain rejected.

## All-Settings 300s Retest

Reopened again on 2026-05-21 to retest every 1.12a candidate setting under the 300s profiling
timeout and to use same-timeout evidence for the search-core gate.

### Search-Core Matrix

All rows used `benchmarks/iteration/search-core`, timeout `300s`, memory `16384 MB`, `SAT_SEED=0`,
and `SAT_PROOF=drat`.

| Setting | Artifact | Solved | Unsolved | PAR-2 | Verdict |
|---|---|---:|---:|---:|---|
| Saved-phase baseline | `log/1.12a/retest-300-phase-saved/results.csv` | 7/9 | 1 timeout, 1 unknown | 1743.239 | reference |
| Focused/VMTF | `log/1.12a/retest-300-focused-vmtf/results.csv` | 2/9 | 6 timeout, 1 unknown | 4321.673 | fail |
| Stable-SAT, rephase off | `log/1.12a/retest-300-stable-rephase-off/results.csv` | 5/9 | 3 timeout, 1 unknown | 3004.366 | fail |
| Stable-SAT, rephase on | `log/1.12a/retest-300-stable-rephase-on/results.csv` | 3/9 | 5 timeout, 1 unknown | 4024.283 | fail |

Search-core comparisons against the 300s saved-phase baseline:

- Focused/VMTF: `verdict=FAIL`, `promotion_verdict=significant_regression`,
  PAR-2 delta `+2578.434`, lost solved rows `1-TC-256-K-63`,
  `544707209399nw.shuffled-as.sat03-1671`, `SC25_Timetable_C_406`, `case9`, and `mp1-Nb7T46`.
- Stable-SAT, rephase off: `verdict=FAIL`, `promotion_verdict=significant_regression`,
  PAR-2 delta `+1261.127`, newly solved `DLTM_twitter845_79_19`, but lost solved rows
  `1-TC-256-K-63`, `case9`, and `mp1-Nb7T46`.
- Stable-SAT, rephase on: `verdict=FAIL`, `promotion_verdict=significant_regression`,
  PAR-2 delta `+2281.044`, lost solved rows `1-TC-256-K-63`,
  `544707209399nw.shuffled-as.sat03-1671`, `case9`, and `mp1-Nb7T46`.
- Rephase A/B: `log/1.12a/retest-300-stable-rephase-on` regressed against
  `log/1.12a/retest-300-stable-rephase-off` by `+1019.917` PAR-2 and lost the
  `544707209399nw.shuffled-as.sat03-1671` and `DLTM_twitter845_79_19` solves.

No advanced candidate clears the first search-core gate. Downstream promotion suites
(`benchmarks/discriminating`, `benchmarks/iteration/regression-guards`, and holdout) were not rerun
for these candidates because the bead promotion rule requires passing search-core before those
suite-level promotion checks are meaningful.

### Profile Matrix

All profile rows used `benchmarks/profiling`, timeout `300s`, memory `16384 MB`, and DRAT proof
checking through `tools/bench.sh`.

| Setting | Artifact | Solved | Unsolved | PAR-2 | Correctness |
|---|---|---:|---:|---:|---|
| Default baseline | `log/1.12a/profile-after-300-reopen/results.csv` | 11/11 | 0 | 624.808 | pass |
| Saved phase | `log/1.12a/profile-300-phase-saved/results.csv` | 11/11 | 0 | 629.340 | pass |
| Focused/VMTF | `log/1.12a/profile-300-focused-vmtf/results.csv` | 3/11 | 6 timeout, 2 error | 5096.329 | fail |
| Stable-SAT, rephase off | `log/1.12a/profile-300-stable-rephase-off/results.csv` | 4/11 | 5 timeout, 2 error | 4520.623 | fail |
| Stable-SAT, rephase on | `log/1.12a/profile-300-stable-rephase-on/results.csv` | 2/11 | 7 timeout, 2 error | 5610.295 | fail |

Profile comparisons against the default 300s baseline:

- Saved phase remains effectively neutral: 11/11 solved, no status regressions, PAR-2 delta
  `+4.532`, compare verdict PASS.
- Focused/VMTF has correctness failures and major performance regression: 3/11 solved,
  PAR-2 delta `+4471.521`, compare verdict FAIL.
- Stable-SAT, rephase off has correctness failures and major performance regression: 4/11 solved,
  PAR-2 delta `+3895.815`, compare verdict FAIL.
- Stable-SAT, rephase on has correctness failures and major performance regression: 2/11 solved,
  PAR-2 delta `+4985.487`, compare verdict FAIL.
- Profile rephase A/B confirms rephase-on is worse for the current advanced stack:
  `+1089.672` PAR-2 versus rephase-off, with new timeouts on `feistel_b64_k32_r22` and
  `random_v355_s3`.

The advanced profile settings all produced SAT model-check failures on the same two instances:

- `46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized`
- `5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7`

The harness error is: the solver printed `s SATISFIABLE`, but the internal SAT model checker
reported `model_check_result=fail`. Logs:

- `log/1.12a/profile-300-focused-vmtf/errors.log`
- `log/1.12a/profile-300-stable-rephase-off/errors.log`
- `log/1.12a/profile-300-stable-rephase-on/errors.log`

Follow-up bead created: `SAT-playground-5b2.2.20` - "Fix advanced search SAT model-check failures".

### Retest Decision

The all-settings retest strengthens the original rejection:

- The default 300s profile is healthy and remains the only supported profile behavior from this
  bead's evidence.
- `SAT_PHASE=saved` remains an opt-in diagnostic setting with neutral profile behavior, but it is
  not promoted by this bead.
- Focused/VMTF, Stable-SAT rephase-off, and Stable-SAT rephase-on remain rejected for promotion.
- Advanced settings must not be promoted or used as acceptance baselines until the model-check
  failures are fixed and the profile/search-core regressions are re-evaluated.
