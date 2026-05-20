# M.3 Triage - EMA + Phase + Binary

Date: 2026-05-20
Milestone bead: `SAT-playground-5b2.5.4`
Prior implementation gate: `SAT-playground-5b2.2.11` (`[1.8] Core search default candidate milestone`)
Benchmark commit before this triage artifact: `562dedf`

## Scope

This milestone gates the Phase 1 search work through `1.8`: EMA restarts,
saved/target/best phase selection, the opt-in binary implication fast path,
decision-heap cleanup, and the composed Phase 1.8 candidate configurations.

No solver code was changed for this milestone triage. The artifact work was to
explain why the Phase 1.8 candidate milestone missed, decide keep/tune/revert
for each feature since the previous milestone, identify the next work, and
confirm that the current default profile still has no profile regression.

## Evidence

Primary Phase 1.8 candidate artifacts:

| Run | Results | Solved | PAR-2 | Verdict |
|---|---|---:|---:|---|
| saved-phase reference | `log/1.5/search-core-saved/results.csv` | 5/9 | `1188.569` | reference |
| conservative candidate | `log/1.8/search-core-conservative/results.csv` | 1/9 | `1960.600` | FAIL |
| strong/exploratory candidate | `log/1.8/search-core-strong/results.csv` | 1/9 | `2023.940` | FAIL |
| default profile final | `log/1.8/profile-default-final/results.csv` | 9/11 | `711.492` | PASS vs `1.6` |

Candidate replay hashes:

| Candidate | Config artifact | Config hash | Notes |
|---|---|---|---|
| conservative | `log/1.8/candidates/candidate-conservative.config` | `024e0e9587682af1` | LBD + EMA + LBD-tiered reduce + saved phase; binary fast off |
| strong | `log/1.8/candidates/candidate-strong.config` | `c32d0e3dbd78a31b` | conservative plus reason LBD updates, target phase, binary fast |
| exploratory | `log/1.8/candidates/candidate-exploratory.config` | `c32d0e3dbd78a31b` | identical to strong in the current plan |

Correctness validation from `log/1.8/summary.md`:

- Conservative smoke: 9/9 passed, UNSAT proofs verified.
- Strong smoke: 9/9 passed, UNSAT proofs verified.
- Brute-force oracle tests: 4 passed.
- Full unit test suite: 215 passed.
- Conservative smoke-plus: 9/9 solved, PAR-2 `0.061`.
- Strong smoke-plus: 9/9 solved, PAR-2 `0.059`.

## Top Wins

The meaningful M.3 candidate win is narrow and isolated to the conservative
candidate on one search-core instance. Deltas below use 120s PAR-2 effective
instance time versus the saved-phase reference unless otherwise noted.

| Instance | Before | After | Delta | Candidate | Category |
|---|---:|---:|---:|---|---|
| `544707209399nw.shuffled-as.sat03-1671` | SAT `80.630s` | SAT `40.600s` | `-40.030s` | conservative | restart/phase-policy |
| `feistel_b64_k32_r22` | SAT `51.725s` | SAT `51.626s` | `-0.099s` | default profile vs 1.6 | benchmark-noise |
| `feistel_b64_k52_r17` | SAT `23.249s` | SAT `23.176s` | `-0.073s` | default profile vs 1.6 | benchmark-noise |
| `557d7d4db5399188f62bc39598c6d868-mp1-Nb7T46` | SAT `41.827s` | SAT `41.767s` | `-0.060s` | default profile vs 1.6 | benchmark-noise |
| `random_v355_s3` | SAT `23.321s` | SAT `23.272s` | `-0.049s` | default profile vs 1.6 | benchmark-noise |
| `random_v285_s2` | UNSAT `8.738s` | UNSAT `8.724s` | `-0.014s` | default profile vs 1.6 | benchmark-noise |

There are not ten real wins in the M.3 evidence. Counting unchanged timeout rows
as wins would hide the actual signal. The only robust candidate-level win is the
conservative composition on `544707209399nw.shuffled-as.sat03-1671`; default
profile differences are all far below noise.

## Top Regressions

Candidate regressions below use the saved-phase reference because `SAT_PHASE=saved`
was the best single-feature search-core result before the 1.8 composed gate.

| Instance | Before | Conservative | Strong/exploratory | Candidate impact | Category |
|---|---:|---:|---:|---:|---|
| `SC25_Timetable_C_392` | SAT `3.826s` | TIMEOUT | TIMEOUT | `+116.174s` effective | search-trajectory |
| `battleship-16-31-sat` | SAT `22.390s` | TIMEOUT | TIMEOUT | `+97.610s` effective | restart/phase-policy |
| `mp1-Nb7T46` | SAT `41.263s` | TIMEOUT | TIMEOUT | `+78.737s` effective | learned-clause-quality |
| `SC25_Timetable_C_406` | SAT `80.460s` | TIMEOUT | TIMEOUT | `+39.540s` effective | restart/phase-policy |
| `544707209399nw.shuffled-as.sat03-1671` | SAT `80.630s` | SAT `40.600s` | SAT `103.940s` | strong is `+23.310s` | restart/phase-policy |
| `83aa254f-1.normalised` | UNKNOWN `14.570s` | UNKNOWN `14.716s` | UNKNOWN `14.712s` | about `+0.14s` | benchmark-noise |

There are not ten real regressions in the 9-instance search-core gate. The four
lost solved instances are enough to reject both candidate profiles.

## Lost And Newly Solved Instances

Compared with `log/1.5/search-core-saved/results.csv`:

| Instance | Conservative | Strong/exploratory | Decision |
|---|---|---|---|
| `SC25_Timetable_C_392` | lost solved, SAT -> TIMEOUT | lost solved, SAT -> TIMEOUT | reject candidate profiles |
| `SC25_Timetable_C_406` | lost solved, SAT -> TIMEOUT | lost solved, SAT -> TIMEOUT | reject candidate profiles |
| `battleship-16-31-sat` | lost solved, SAT -> TIMEOUT | lost solved, SAT -> TIMEOUT | reject candidate profiles |
| `mp1-Nb7T46` | lost solved, SAT -> TIMEOUT | lost solved, SAT -> TIMEOUT | reject candidate profiles |

Newly solved instances:

- None in either Phase 1.8 candidate versus the saved-phase reference.

## Status, Proof, And Model Failures

No proof/model/status correctness failure was observed in the solved rows used
for the 1.8 milestone decision.

- Candidate smoke suites passed 9/9 with UNSAT proofs verified.
- Candidate smoke-plus suites passed 9/9.
- Search-core solved rows verified SAT models successfully.
- The search-core `83aa254f-1.normalised` row is `UNKNOWN` under the 1.8
  candidates instead of the older exit-134 `ERROR`; this is not a promotion win
  because the instance remains unsolved and the candidate profiles lose four
  solved rows.

Risk impact:

- R1 SAT model reconstruction: no failure observed.
- R2 UNSAT proof invalidity: no failure observed in solved rows.
- R4 reason corruption after GC/binary deletion: no failure observed, but
  binary-fast plus clause minimization remains intentionally guarded until 1.11.
- R6 benchmark overfit: triggered. One narrow candidate win does not generalize.
- R8 nondeterminism/noise: profile deltas are below noise; candidate regressions
  are status changes and are not noise.

## Dominant Bottleneck Categories

| Instance | M.3 candidate status | Dominant category | Reason |
|---|---|---|---|
| `SC25_Timetable_C_392` | lost solved under both candidates | search-trajectory | extremely fast saved-phase solve becomes timeout when EMA/LBD/reduce composition changes search path |
| `SC25_Timetable_C_406` | lost solved under both candidates | restart/phase-policy | solved by saved phase, lost by composed restart/phase policies |
| `battleship-16-31-sat` | lost solved under both candidates | restart/phase-policy | sensitive SAT trajectory; phase/decision interaction dominates |
| `mp1-Nb7T46` | lost solved under both candidates | learned-clause-quality | learned/reduce/phase composition harms a previously reliable SAT solve |
| `544707209399nw.shuffled-as.sat03-1671` | conservative win, strong slowdown | restart/phase-policy | conservative stack helps; target phase + reason updates + binary fast hurts relative to conservative |
| `case9` | unchanged timeout in 1.8 search-core | search-trajectory | remains a hard SAT search-path instance; binary-fast safety was separately fixed in 1.6 |
| `DLTM_twitter845_79_19` | unchanged timeout in 1.8 search-core | restart/phase-policy | known SAT-side trajectory target for focused/stable/VMTF/rephase work |
| `1-TC-256-K-63` | unchanged timeout in 1.8 search-core | search-trajectory | no evidence the current composed stack improves this class |
| `83aa254f-1.normalised` | UNKNOWN, not solved | memory/GC | no longer an abort in this gate, but still no solved result |

## Holdout Summary

No new M.3 holdout run was required because no candidate was promoted and the
candidate search-core gate already failed by lost solved instances. The current
fixed holdout set remains the five symlinked instances under
`benchmarks/iteration/holdout`:

| Holdout instance | Latest durable evidence | Category |
|---|---|---|
| `DLTM_twitter845_79_19` | TIMEOUT at 300s in `log/M.2/discriminating-2026-05-19-01/results.csv` | restart/phase-policy |
| `SC25_Timetable_C_406` | SAT in `86.269s` at M.2; lost to TIMEOUT under both M.3 candidates at 120s | restart/phase-policy |
| `bp4_CSO_AM_IXA_LP.normalised` | TIMEOUT at 300s in M.2 | preprocessing-shrink |
| `brocard_problem_large` | UNSAT in `8.521s` at M.2 | preprocessing-shrink |
| `sqrt-mitern171` | TIMEOUT at 300s in M.2 | learned-clause-quality |

Holdout interpretation: do not spend the next bead on a full holdout rerun until
there is a promotable candidate. The failed M.3 candidate gate already identifies
search-path sensitivity as the immediate blocker.

## Feature Decisions Since Prior Milestone

| Bead | Feature | Decision | Rationale |
|---|---|---|---|
| `1.4` | `SAT_RESTART=kissat-ema` | Keep, tune, no promotion | Correct and useful infrastructure, but standalone search-core regressed to 1/9 solved and lost four solved rows. Keep opt-in for focused/stable and later VMTF experiments. |
| `1.5` | `SAT_PHASE=saved` | Keep, consider repeated gate before promotion | The saved policy was the only Phase 1.5 search-core mode that passed a single-run gate: 5/9 solved, PAR-2 `1188.569`, no status regressions. The gain was small, so do not promote alone yet. |
| `1.5` | `SAT_PHASE=target-then-saved` and `best-then-target-then-saved` | Keep as scaffolding, tune later, no promotion | Both exercised real phase counters but lost solved instances in search-core. They are needed for stable/rephase experiments but are not default-ready. |
| `1.6` | `SAT_BINARY_FAST=on` | Keep opt-in, tune in 1.11, no promotion | Correctness gates passed with clause minimization disabled, but search-core binary-fast with ccmin off solved only 3/9 and still needs binary-reason-aware minimization before promotion. |
| `1.7` | Decision-heap cleanup | Keep | Behavior-preserving cleanup passed unit/smoke/invariant/profile gates and reduces risk for VMTF; no profile status regression. |
| `1.8` | Conservative candidate composition | Reject profile promotion, keep as experimental replay artifact | One narrow win on `544707...`, but four lost solved instances. |
| `1.8` | Strong/exploratory candidate composition | Reject profile promotion, keep as experimental replay artifact | Same four lost solved instances plus slower than conservative on `544707...`. |

No feature is reverted. All new feature knobs stay off in promoted profiles.

## Profile Changes Accepted Or Rejected

Accepted:

- None.

Rejected or deferred:

- Do not enable `SAT_RESTART=kissat-ema` by default.
- Do not enable `SAT_PHASE=saved` by default from a single-run, small-gain gate.
- Do not enable `SAT_PHASE=target-then-saved` or `best-then-target-then-saved`
  by default.
- Do not enable `SAT_BINARY_FAST=on` by default until 1.11 removes the
  clause-minimization guard safely and the search-core/profile gates pass.
- Do not promote the 1.8 conservative, strong, or exploratory replay files into
  `default` or `fast`.

Config flags to remove:

- None. The current flags are useful for replayable A/B runs and upcoming Phase
  1 work, but they must remain absent from non-experimental profile defaults.

Parking-lot decision:

- Keep walking local search, ELS, BCE, and optional bounded sweeps parked. The
  M.3 miss is explained by search-path sensitivity in active Phase 1 features,
  not by evidence that a parked feature should be unparked.

## Confidence And Noise Decision

Default profile conclusion: PASS / no regression.

- `log/1.8/profile-default-final/results.csv`: 9/11 solved, PAR-2 `711.492`.
- Compared with `log/1.6/profile-after-final/results.csv`: 9/11 solved, PAR-2
  `711.619`, delta `-0.127`.
- Status regressions: none.
- Median paired speedup: `1.0000`.
- The delta is well inside single-run noise and should be treated as no
  regression, not a speed win.

Candidate conclusion: significant regression / milestone miss.

- Conservative candidate: 1/9 solved, PAR-2 `1960.600`; four solved-instance
  losses versus the saved-phase reference.
- Strong/exploratory candidate: 1/9 solved, PAR-2 `2023.940`; same four
  solved-instance losses and no newly solved instances.
- These are status regressions, not timing noise. The Phase 1.8 candidate stack
  does not meet the target larger drop or the rough `-20%` to `-25%`
  discriminating direction.

## Recommended Next Two Beads

1. `SAT-playground-5b2.2.13` - `[1.10] VMTF focused-mode decision queue`.
   Reason: it is the next numeric Phase 1 implementation after the milestone,
   depends on the already-closed focused/stable scaffold, unlocks the 1.12a
   advanced search candidate milestone, and directly tests whether focused-mode
   decision ordering can recover the SAT search-path regressions.

2. `SAT-playground-5b2.2.14` - `[1.11] Clause minimization, in-block shrink, and
   reason-side bumping`.
   Reason: it has slightly higher critical-path weight and fixes the explicit
   binary-fast clause-minimization guard from 1.6, but it is riskier than VMTF
   because it touches conflict analysis and proof/model safety. It should follow
   VMTF unless binary-fast promotion becomes the immediate priority.

Third candidate:

- `SAT-playground-5b2.2.15` - `[1.12] Rephasing hook`, after VMTF and/or
  clause minimization establish the next composed candidate.

## Tasks To Defer Or Delete

Defer:

- `SAT-playground-5b2.2.16` (`[1.12a] Advanced search candidate milestone`)
  until 1.10, 1.11, and 1.12 have produced replayable candidates.
- `SAT-playground-5b2.2.17` (`[1.13] Guarded chronological backtracking`) until
  1.11 lands, because its dependency chain includes clause minimization.
- Phase 2 work until the Phase 1 gate explains whether the search stack has a
  promotable candidate.

Delete:

- None. The miss is a tuning/composition miss, not dead-code evidence.

## Fresh-Eyes Review Notes

- The milestone miss is not caused by a default-profile regression: default
  profile remained 9/11 solved with no status change.
- The miss is not caused by proof/model correctness failures in the candidate
  gates.
- The miss is a search-path sensitivity problem: several SAT instances solved by
  saved phase are lost when restart, reduce, phase, and binary-fast knobs are
  composed.
- The conservative candidate's `544707...` win is useful but insufficient. It
  should be used as a tuning signal for VMTF/rephase rather than as a profile
  promotion argument.
- The strong and exploratory candidates are identical in the current plan. A
  later plan cleanup could differentiate them, but no bead change is required
  because the duplication is now documented in `log/1.8/summary.md`.
