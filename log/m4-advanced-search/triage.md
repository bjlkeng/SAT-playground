# M.4 Mode/VMTF/Minimize/Rephase Advanced-Search Triage

Milestone bead: `SAT-playground-5b2.5.5`

Date: 2026-05-21

## Scope

This milestone reviews the Phase 1 advanced-search stack after:

- `1.9` focused/stable mode scaffold and reluctant restarts
- `1.10` focused-mode VMTF queue
- `1.11` clause minimization / in-block shrink / reason-side bumping
- `1.12` rephasing hook
- `1.12a` advanced candidate matrix
- the follow-up binary-fast remap correctness fix

The purpose is to decide whether any advanced-search configuration should be promoted, tuned,
parked, or reverted before starting the next feature family.

## Artifacts Reviewed

Primary written summary:

- `log/1.12a/summary.md`

Fresh post-fix search-core reruns:

- `log/1.12a/reretest-300-phase-saved/results.csv`
- `log/1.12a/reretest-300-focused-vmtf/results.csv`
- `log/1.12a/reretest-300-stable-rephase-off/results.csv`
- `log/1.12a/reretest-300-stable-rephase-on/results.csv`

Fresh post-fix profile reruns:

- `log/1.12a/reretest-profile-300-default/results.csv`
- `log/1.12a/reretest-profile-300-phase-saved/results.csv`
- `log/1.12a/reretest-profile-300-focused-vmtf/results.csv`
- `log/1.12a/reretest-profile-300-stable-rephase-off/results.csv`
- `log/1.12a/reretest-profile-300-stable-rephase-on/results.csv`

Correctness logs:

- all nine `log/1.12a/reretest*/errors.log` files are empty

## Candidate Summary

### Search-Core, 300s

| Variation | Solved | Unsolved | PAR-2 | Decision |
|---|---:|---:|---:|---|
| Saved phase baseline | `7/9` | `1 timeout`, `1 unknown` | `1744.881` | Keep as reference |
| Focused/VMTF | `2/9` | `6 timeout`, `1 unknown` | `4316.538` | Reject for promotion |
| Stable-SAT rephase off | `5/9` | `3 timeout`, `1 unknown` | `2986.432` | Reject for promotion; tune later |
| Stable-SAT rephase on | `3/9` | `5 timeout`, `1 unknown` | `4007.975` | Reject for promotion |

### Profile, 300s

| Variation | Solved | Unsolved | PAR-2 | Decision |
|---|---:|---:|---:|---|
| Default | `11/11` | `0` | `628.815` | Keep as default |
| Saved phase | `11/11` | `0` | `629.709` | Neutral; no profile change |
| Focused/VMTF | `4/11` | `7 timeout` | `4532.312` | Reject for promotion |
| Stable-SAT rephase off | `5/11` | `6 timeout` | `3972.872` | Reject for promotion; tune later |
| Stable-SAT rephase on | `3/11` | `8 timeout` | `5065.088` | Reject for promotion |

## Top Regressions

Measured against the profile default rerun. Deltas use PAR-2 accounting, so a 300s timeout counts
as 600s.

| Rank | Variation | Instance | Before | After | PAR-2 delta | Bottleneck category |
|---:|---|---|---:|---:|---:|---|
| 1 | Focused/VMTF | `random_v285_s2` | `UNSAT 8.789s` | `TIMEOUT` | `+591.211s` | search-trajectory |
| 2 | Stable off | `random_v285_s2` | `UNSAT 8.789s` | `TIMEOUT` | `+591.211s` | search-trajectory |
| 3 | Stable on | `random_v285_s2` | `UNSAT 8.789s` | `TIMEOUT` | `+591.211s` | restart/phase-policy |
| 4 | Focused/VMTF | `random_v292_s4` | `UNSAT 14.227s` | `TIMEOUT` | `+585.773s` | search-trajectory |
| 5 | Stable off | `random_v292_s4` | `UNSAT 14.227s` | `TIMEOUT` | `+585.773s` | search-trajectory |
| 6 | Stable on | `random_v292_s4` | `UNSAT 14.227s` | `TIMEOUT` | `+585.773s` | restart/phase-policy |
| 7 | Focused/VMTF | `feistel_b64_k52_r17` | `SAT 23.290s` | `TIMEOUT` | `+576.710s` | restart/phase-policy |
| 8 | Stable off | `feistel_b64_k52_r17` | `SAT 23.290s` | `TIMEOUT` | `+576.710s` | restart/phase-policy |
| 9 | Stable on | `feistel_b64_k52_r17` | `SAT 23.290s` | `TIMEOUT` | `+576.710s` | restart/phase-policy |
| 10 | Stable on | `random_v355_s3` | `SAT 23.468s` | `TIMEOUT` | `+576.532s` | restart/phase-policy |

Additional broad regressions:

- all advanced variants timeout on `REGRandom-K4-L1-Seed40.sanitized`
- all advanced variants timeout on profile `mp1-Nb7T46`
- focused/VMTF and stable-on timeout on `feistel_b64_k32_r22`
- all advanced variants timeout on Sudoku `0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12`

## Top Wins

Measured against the profile default rerun unless noted.

| Rank | Variation | Instance | Before | After | Delta | Bottleneck category |
|---:|---|---|---:|---:|---:|---|
| 1 | Focused/VMTF | `Kakuro-easy-112-ext.xml.hg_7` | `SAT 209.421s` | `SAT 58.863s` | `-150.558s` | search-trajectory |
| 2 | Stable off | `Kakuro-easy-112-ext.xml.hg_7` | `SAT 209.421s` | `SAT 61.638s` | `-147.783s` | search-trajectory |
| 3 | Stable on | `Kakuro-easy-112-ext.xml.hg_7` | `SAT 209.421s` | `SAT 66.392s` | `-143.029s` | search-trajectory |
| 4 | Stable off | `feistel_b64_k32_r22` | `SAT 52.123s` | `SAT 28.409s` | `-23.714s` | restart/phase-policy |
| 5 | Focused/VMTF, search-core | `battleship-16-31-sat` | `SAT 22.995s` | `SAT 0.778s` | `-22.217s` | phase-policy |
| 6 | Stable off, search-core | `DLTM_twitter845_79_19` | `TIMEOUT` | `SAT 118.900s` | newly solved | search-trajectory |
| 7 | Stable off, search-core | `battleship-16-31-sat` | `SAT 22.995s` | `SAT 6.420s` | `-16.575s` | phase-policy |

These wins are real enough to justify later targeted tuning, but not enough to override the broad
status regressions.

## Lost Solved Instances

Profile lost solves against default:

| Variation | Lost solved instances |
|---|---|
| Focused/VMTF | `feistel_b64_k32_r22`, `feistel_b64_k52_r17`, Sudoku, REGRandom K4, `mp1-Nb7T46`, `random_v285_s2`, `random_v292_s4` |
| Stable off | `feistel_b64_k52_r17`, Sudoku, REGRandom K4, `mp1-Nb7T46`, `random_v285_s2`, `random_v292_s4` |
| Stable on | `feistel_b64_k32_r22`, `feistel_b64_k52_r17`, Sudoku, REGRandom K4, `mp1-Nb7T46`, `random_v285_s2`, `random_v292_s4`, `random_v355_s3` |

Search-core lost solves against saved phase:

| Variation | Lost solved instances |
|---|---|
| Focused/VMTF | `1-TC-256-K-63`, `544707209399nw.shuffled-as.sat03-1671`, `SC25_Timetable_C_406`, `case9`, `mp1-Nb7T46` |
| Stable off | `1-TC-256-K-63`, `case9`, `mp1-Nb7T46` |
| Stable on | `1-TC-256-K-63`, `544707209399nw.shuffled-as.sat03-1671`, `case9`, `mp1-Nb7T46` |

## Newly Solved Instances

Search-core newly solved against saved phase:

- Stable off solves `DLTM_twitter845_79_19` in `118.900s`; saved phase timed out.

Profile newly solved against default:

- none, because default already solved `11/11`.

## Status, Proof, and Model Failures

No current proof/model/status failures remain in the post-fix reruns:

- every rerun `errors.log` is empty
- SAT rows were model-checked by `tools/bench.sh`
- UNSAT rows were proof-checked where applicable
- the earlier invalid SAT model failure was fixed by remapping binary-fast metadata during inline
  original-abstraction arena migration

The remaining advanced-search problem is performance/search trajectory, not correctness.

## Hard-Instance Bottleneck Classification

| Instance or family | Observed behavior | Dominant bottleneck category |
|---|---|---|
| Kakuro profile | all advanced variants solve roughly 3.1x-3.6x faster than default | search-trajectory win |
| `feistel_b64_k32_r22` | stable off improves; focused and stable-on do not preserve this win | restart/phase-policy |
| `feistel_b64_k52_r17` | every advanced variant times out from a fast default solve | restart/phase-policy |
| random UNSAT rows | every advanced variant times out from fast default solves | learned-clause-quality / search-trajectory |
| REGRandom K4 | every advanced variant times out; prior correctness bug fixed | search-trajectory |
| `mp1-Nb7T46` | every advanced variant times out from a default solve | restart/phase-policy |
| Search-core `DLTM_twitter845_79_19` | stable off newly solves | search-trajectory win |
| Search-core `battleship-16-31-sat` | focused and stable off improve sharply | phase-policy win |

## Keep, Tune, Revert, or Park Decisions

| Feature | Decision | Reason |
|---|---|---|
| Focused/stable mode scaffold | Keep opt-in; tune later | Needed for later search experiments and has class wins, but current bundle is not promotable. |
| Reluctant stable restarts | Keep opt-in; tune later | Required by focused/stable scaffold; not accepted as default. |
| VMTF focused queue | Keep opt-in; tune later | Useful for targeted experiments; broad regressions prevent promotion. |
| Clause minimization in advanced stack | Keep implementation; do not default-enable in binary-fast runs | Binary-reason correctness is fixed, but recursive/basic combinations are not promotion evidence. |
| Rephase hook | Keep opt-in but do not tune first | Rephase-on is worse than rephase-off in both search-core and profile. |
| Binary implication fast path | Keep opt-in | Correctness blocker is fixed; current advanced bundles still show trajectory regressions. |
| Stable off advanced bundle | Best future tuning baseline | It has the best advanced profile/search-core rows and the `feistel_b64_k32_r22` plus Kakuro wins. |
| Focused/VMTF bundle | Park as non-baseline experiment | Strong Kakuro/battleship wins, but too many timeouts. |
| Stable-on bundle | Park until rephase-specific evidence appears | Consistently worse than stable-off. |

No feature is deleted in this milestone because all implementations remain tested, opt-in, and useful
for diagnostics. No feature is promoted into `default` or `fast`.

## Profile Changes

Accepted:

- none

Rejected:

- do not replace default with focused/VMTF
- do not replace default with stable off
- do not replace default with stable on
- do not enable rephase by default
- do not enable recursive clause minimization in binary-fast candidate profiles by default

Config flags to remove from candidate profiles:

- none removed from the schema or code
- remove `SAT_REPHASE=on` from the next active tuning baseline
- keep `SAT_CHRONO=off` until the guarded chrono bead produces evidence

## Holdout and Discriminating Summary

No additional holdout or discriminating run was launched for this milestone because every advanced
candidate failed the mandatory first gate on search-core, and all advanced profile reruns showed
severe solved-count regressions against the default profile.

This is a deliberate rejection-path shortcut, not missing evidence for promotion:

- the 1.12a promotion rule requires beating search-core before promotion
- no advanced candidate beat search-core
- profile evidence independently confirms large regressions
- running larger suites would not make a failed first gate promotable

Future Phase 1 gate work must still run the required discriminating/holdout suites if a candidate
survives search-core and profile screening.

## Noise and Confidence Decision

The rejection is not a timing-noise call:

- search-core PAR-2 deltas are `+1241.551s` to `+2571.657s` against saved phase
- profile PAR-2 deltas are `+3344.057s` to `+4436.273s` against default
- advanced variants introduce 6-8 profile timeouts from an `11/11` default baseline
- the same regression families repeat across variants

The local wins should be treated as real signals for targeted tuning, but the promotion rejection is
well outside normal single-run noise.

## Recommended Next Two Beads

1. `SAT-playground-5b2.2.17` - `[1.13] Guarded chronological backtracking`
   - Implement only the conservative bounded chrono form.
   - Use stable-off evidence as the tuning baseline where applicable.
   - Accept only class-specific wins without proof/model regressions.

2. `SAT-playground-s11-1-14a` - `[1.14a] Inline-blocker propagation specialization`
   - Start throughput work after the chrono skip/implementation decision.
   - Measure propagation counters and ensure no K4/Kakuro/Timetable regression.

## Tasks To Defer Or Delete

Defer:

- rephase-on tuning until a rephase-specific counter or benchmark-family signal exists
- recursive minimization promotion in binary-fast profiles until search-core no longer regresses
- larger discriminating/holdout advanced-candidate runs until a first-gate candidate survives

Delete:

- none

## Milestone Decision

M.4 missed its target. The advanced search stack is correctness-clean after the binary-fast remap
fix, but it is not promotable. Keep the implementations as opt-in diagnostic/tuning components,
reject all advanced profile changes, and proceed to guarded chrono / throughput work with
stable-off as the most informative tuning baseline and default as the safety baseline.
