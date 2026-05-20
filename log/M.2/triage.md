# M.2 Triage - LBD + Tiered Reduce + DB/GC Policy

Date: 2026-05-19
Benchmark commit before this triage artifact: `bf0aafc`
Milestone bead: `SAT-playground-5b2.5.3`

## Scope

This milestone gates the work through `1.3a`: learned-clause LBD metadata, optional reason-side LBD updates, LBD-tiered learned-clause reduction, and clause database / GC budget telemetry.

No solver code was changed for this milestone. The artifact work was to run the gate, compare current evidence, classify remaining failures, and decide whether the `1.1` through `1.3a` slice should be kept, tuned, reverted, promoted, or used only as infrastructure for the next Phase 1 beads.

## Evidence

Primary milestone run:

```bash
bash tools/bench.sh -t 300 -m 16384 \
  -d benchmarks/discriminating solver/11-kissat-port \
  --log-dir log/M.2/discriminating-2026-05-19-01
```

Result:

| Set | Timeout | Instances | Solved | SAT | UNSAT | Timeout | Error | PAR-2 | Results |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| discriminating | 300s | 20 | 11 | 8 | 3 | 7 | 2 | 6434.211 | `log/M.2/discriminating-2026-05-19-01/results.csv` |

Profile/no-regression evidence from the completed `1.3a` implementation:

| Comparison | Before | After | Solved | PAR-2 before | PAR-2 after | Delta | Verdict |
|---|---|---|---:|---:|---:|---:|---|
| pre-`1.3a` solver11 vs post-`1.3a` solver11 | `log/profile-compare-solver11-2026-05-19-1222/results.csv` | `log/profile-compare-solver11-2026-05-19-1647/results.csv` | 9/11 both | 712.728 | 710.454 | -2.274 | PASS |
| same-day solver10 vs post-`1.3a` solver11 | `log/profile-compare-solver10-2026-05-19-1202/results.csv` | `log/profile-compare-solver11-2026-05-19-1647/results.csv` | 9/11 both | 711.024 | 710.454 | -0.570 | PASS |

Important caveat: the discriminating comparison below uses the manifest's `solver10_reference_time` as a directional reference, not a fresh same-run paired solver10 benchmark. Values marked `TIMEOUT` or greater than the 300s M.2 timeout are treated as unsolved for effective PAR-2 comparison.

## Top 10 Wins

Wins are sorted by effective delta versus the manifest solver10 reference under the 300s gate. Negative is better for solver11.

| Instance | Current result | Current time | Manifest solver10 reference | Effective delta | Family | Bottleneck tag |
|---|---:|---:|---:|---:|---|---|
| `brocard_problem_large` | UNSAT | 8.521s | 479.9s | -591.479s | brocard | preprocess-residual |
| `mp1-Nb7T46` | SAT | 43.467s | TIMEOUT | -556.533s | planning | learned-clause-quality |
| `REGRandom-K4-L1-Seed40` | UNSAT | 56.140s | TIMEOUT | -543.860s | random-k | preprocess-lbd-retention |
| `SC25_Timetable_C_406` | SAT | 86.269s | 772.8s | -513.731s | timetable | search-phase-saving |
| `1-TC-256-K-63` | SAT | 194.970s | TIMEOUT | -405.030s | tc | search-trajectory |
| `battleship-16-31-sat` | SAT | 22.383s | 169.47s | -147.087s | battleship | phase-decision-quality |
| `case9` | SAT | 121.078s | 215.1s | -94.022s | case | search |
| `SCPC-500-1` | UNSAT | 190.775s | 270.8s | -80.025s | scpc | clause-db-lbd |
| `SC25_Timetable_C_392` | SAT | 4.316s | 72.8s | -68.484s | timetable | search-restart-strategy |
| `544707209399nw.shuffled-as.sat03-1671` | SAT | 80.144s | 105.5s | -25.356s | sat03-shuffled | phase-restarts |

## Top Regressions And Failures

The strongest regressions are not slow solved cases; they are solver aborts that miss the solver11 result contract.

| Instance | Current result | Current time | Manifest solver10 reference | Effective delta | Family | Classification |
|---|---:|---:|---:|---:|---|---|
| `83aa254f-1.normalised` | ERROR | 61.263s | 66.6s | +533.400s | normalised | memory/GC, missing `result.json`; reproduced as pre-existing against solver10 in `log/bench-s11-1.3a-83aa-solver10-2026-05-19-1721` |
| `ee5fb3e-11.normalised` | ERROR | 27.874s | 166.0s | +434.000s | normalised | memory/GC, missing `result.json`; newly tracked as `SAT-playground-bs6` |
| `bp4_CSO_AM_IXA_LP.normalised` | TIMEOUT | 300.000s | TIMEOUT | 0.000s | bp4 | preprocessing-shrink |
| `aaai10-planning-pathways-step20` | TIMEOUT | 300.000s | 381.7s | 0.000s | planning | learned-clause-quality |
| `sqrt-mitern171` | TIMEOUT | 300.000s | 464.0s | 0.000s | miters | learned-clause-quality |
| `bp4_CSO_IXA_ZR.normalised` | TIMEOUT | 300.000s | TIMEOUT | 0.000s | bp4 | preprocessing-shrink |
| `circuit_48in64out_with_800gates` | TIMEOUT | 300.000s | TIMEOUT | 0.000s | circuit | preprocessing-shrink / gate-aware-BVE |
| `DLTM_twitter845_79_19` | TIMEOUT | 300.000s | 362.6s | 0.000s | dltm | restart/phase-policy |
| `div-mitern172` | TIMEOUT | 300.000s | 657.9s | 0.000s | miters | learned-clause-quality |
| `Kakuro-easy-112-ext.xml.hg_7` | SAT | 226.148s | 240.6s | -14.452s | kakuro | preprocessing-shrink; solved under M.2 but still much slower than Kissat's 42.2s table time |

## Lost And Newly Solved Instances

Compared with the manifest solver10 reference under an effective 300s gate:

Newly solved or materially recovered:

| Instance | Current | Reference context |
|---|---|---|
| `brocard_problem_large` | UNSAT in 8.521s | manifest solver10 reference 479.9s, effectively unsolved at 300s |
| `mp1-Nb7T46` | SAT in 43.467s | manifest solver10 reference TIMEOUT |
| `REGRandom-K4-L1-Seed40` | UNSAT in 56.140s | manifest solver10 reference TIMEOUT |
| `SC25_Timetable_C_406` | SAT in 86.269s | manifest solver10 reference 772.8s, effectively unsolved at 300s |
| `1-TC-256-K-63` | SAT in 194.970s | manifest solver10 reference TIMEOUT |

Lost solved instances or contract failures:

| Instance | Current | Reference context | Decision |
|---|---|---|---|
| `83aa254f-1.normalised` | ERROR, exit 134, missing `result.json` | manifest solver10 reference 66.6s; isolated solver10 also aborts under the 16 GB task gate | Track but do not attribute to `1.3a`; investigate before Phase 1 promotion |
| `ee5fb3e-11.normalised` | ERROR, exit 134, missing `result.json` | manifest solver10 reference 166.0s | New follow-up bug `SAT-playground-bs6`; blocks `1.15` promotion |

## Status, Proof, And Model Failures

All 11 solved rows were verified by the harness:

- SAT model checks: 8 `ok`
- UNSAT DRAT checks: 3 `ok`
- Wrong-status mismatches: none observed in the solved rows
- Timeouts: 7, verification skipped
- Solver/harness errors: 2, both missing `result.json` because the process aborted with exit 134 before writing the solver11 output contract

`log/M.2/discriminating-2026-05-19-01/errors.log` records:

```text
83aa254f...1.normalised: memory allocation of 128 bytes failed
ee5fb3e...11.normalised: memory allocation of 2589149280 bytes failed
```

Risk impact:

- R1 SAT model reconstruction: no failure on solved SAT instances.
- R2 UNSAT proof invalidity: no failure on solved UNSAT instances.
- R3 stale watcher reads: no direct failure observed.
- R4 reason corruption after GC/binary deletion: no direct failure observed.
- R6 benchmark overfit: do not promote based on this mixed run.
- R7 memory blowup: triggered by the two normalised-instance aborts.
- R8 nondeterminism/noise: profile deltas are single-run and within expected noise.

## Dominant Bottleneck Categories

Hard means timeout, error, or solved but still clearly slow relative to the Kissat table reference.

| Instance | M.2 status | Dominant category | Reason |
|---|---|---|---|
| `bp4_CSO_AM_IXA_LP.normalised` | TIMEOUT | preprocessing-shrink | BP4 class still needs stronger simplification/gate-aware work; not fixed by LBD/GC telemetry |
| `aaai10-planning-pathways-step20` | TIMEOUT | learned-clause-quality | Clause DB/LBD work is not enough alone; needs later restart/minimization/phase interactions |
| `sqrt-mitern171` | TIMEOUT | learned-clause-quality | UNSAT miter remains hard despite LBD metadata |
| `bp4_CSO_IXA_ZR.normalised` | TIMEOUT | preprocessing-shrink | Same BP4 family gap as above |
| `83aa254f-1.normalised` | ERROR | memory/GC | Allocation abort before result contract; pre-existing on solver10 under isolated task gate |
| `circuit_48in64out_with_800gates` | TIMEOUT | preprocessing-shrink | Circuit/gate-aware BVE target remains a Phase 2 issue |
| `DLTM_twitter845_79_19` | TIMEOUT | restart/phase-policy | SAT-side trajectory problem; points at phase/restart/decision work |
| `ee5fb3e-11.normalised` | ERROR | memory/GC | Allocation abort before result contract; follow-up bug created |
| `div-mitern172` | TIMEOUT | learned-clause-quality | Clause DB/LBD alone did not solve the miter gap |
| `Kakuro-easy-112-ext.xml.hg_7` | SAT in 226.148s | preprocessing-shrink | Solved at 300s, timeout at 120s profile, still far behind Kissat |
| `SCPC-500-1` | UNSAT in 190.775s | learned-clause-quality | Solved but slow; useful as a later reduce/minimize tuning target |
| `1-TC-256-K-63` | SAT in 194.970s | search-trajectory | Solved but still slow; search policy remains important |
| `case9` | SAT in 121.078s | search-trajectory | Solved but still slower than the reference table |

## Holdout Summary

The current holdout directory has five instances, all included in the M.2 discriminating manifest. No separate duplicate run was needed.

| Holdout instance | M.2 result |
|---|---|
| `DLTM_twitter845_79_19` | TIMEOUT at 300s |
| `SC25_Timetable_C_406` | SAT in 86.269s |
| `bp4_CSO_AM_IXA_LP.normalised` | TIMEOUT at 300s |
| `brocard_problem_large` | UNSAT in 8.521s |
| `sqrt-mitern171` | TIMEOUT at 300s |

Holdout aggregate from those rows: 2/5 solved, 1 SAT, 1 UNSAT, 3 timeouts, effective PAR-2 1894.790 at 300s.

## Feature Decisions Since Prior Milestone

| Bead | Feature | Decision | Rationale |
|---|---|---|---|
| `1.1` | LBD/glue metadata | Keep, no default promotion | Profile comparison has no status regression; metadata is needed by later restart/reduce/minimization work. Keep `SAT_USE_LBD` default off in normal profile until combined candidates pass discriminating/regression guards. |
| `1.2` | Optional reason-side LBD updates | Keep as default-off diagnostic/experimental path | It provides the needed hook for later learned-quality tuning. Do not promote `SAT_LBD_UPDATE_REASONS=on` yet. |
| `1.3` | LBD-tiered reduce | Keep, tune later, no default promotion | It is structurally required for Phase 1 candidates, but M.2 did not show enough standalone discriminating improvement to make it a default. Budget and tier tuning should wait until restart/phase/binary interactions exist. |
| `1.3a` | Clause DB budget and GC policy | Keep | It adds required observability and root-safe GC behavior with profile no-regression evidence. The milestone found memory aborts, but one is pre-existing and the other is now tracked separately; do not revert the GC policy. |

No feature is promoted to `default` or `fast` from M.2. No feature is reverted. No profile changes are accepted.

## Profile Changes Accepted Or Rejected

Accepted:

- None.

Rejected or deferred:

- Do not enable `SAT_USE_LBD=on` by default yet.
- Do not enable `SAT_LBD_UPDATE_REASONS=on` by default.
- Do not enable `SAT_REDUCE=lbd-tiered` by default.
- Do not change `default` or `fast` profile composition from this milestone.

Config flags to remove:

- None.

Parking-lot decision:

- Keep ELS, BCE, walking local search, and optional bounded sweep parked. M.2 does not provide the counter evidence required to unpark them.

## Confidence And Noise Decision

Profile-no-regression conclusion: PASS.

- Post-`1.3a` profile PAR-2 is 710.454.
- Versus pre-`1.3a` solver11, PAR-2 improved by 2.274s with identical solved count and no status/verification mismatch.
- Versus same-day solver10, PAR-2 improved by 0.570s with identical solved count and no status/verification mismatch.
- These are small single-run deltas and should be treated as no regression, not a robust speed win.

Discriminating conclusion: mixed and not promotion-quality.

- Current discriminating PAR-2 is 6434.211 at 300s.
- Directional effective solver10-reference PAR-2 from the manifest is 8506.870, but that is not a fresh paired run.
- The target direction for M.2 was a meaningful drop and roughly -10% on discriminating. The directional result is encouraging, but the two result-contract errors and seven timeouts prevent promotion.

Decision: keep the implemented `1.1` through `1.3a` work as infrastructure and observability, do not promote it as a performance milestone, and proceed to the next Phase 1 work with the memory-abort follow-up tracked.

## Recommended Next Two Beads

1. `SAT-playground-bs6` - Investigate solver11 normalised-instance memory aborts.
   Reason: two M.2 rows abort before writing `result.json`; this is a reliability/promotion blocker and has been added as a dependency of `SAT-playground-5b2.2.19` (`1.15`).

2. `SAT-playground-5b2.2.10` - `[1.7] Decision heap cleanup`.
   Reason: it is ready, on the Phase 1 path, reduces decision-selection risk, unlocks phase/VMTF work, and is a safer next feature slice than jumping directly into a large binary implication rewrite.

After those, `SAT-playground-5b2.2.9` (`1.6` Binary implication fast path) is the highest direct performance-leverage bead because M.2 still shows search-throughput and propagation-sensitive normalised failures.

## Follow-Up: `SAT-playground-bs6`

Date: 2026-05-20

`SAT-playground-bs6` was resolved by adding a pre-solve memory admission guard to
solver 11. The guard uses `SAT_LIMIT_RSS_MB` when present and otherwise reads the
process address-space cap from `/proc/self/limits`, which captures the standard
`tools/bench.sh -m 16384` ulimit. It estimates the mandatory dense solver state
and one-shot preprocessing peak before `Solver::new_with_config`; if the
estimated peak reaches 90% of the effective cap, solver 11 writes a complete
`UNKNOWN` contract with `unknown_reason=memory-preflight-limit`.

Final-code evidence:

| Instance | Before | After |
|---|---|---|
| `83aa254f-1.normalised` | `ERROR`, exit 134, missing `result.json`, allocation of 128 bytes failed (`log/bs6/repro-83aa-before/results.csv`) | `UNKNOWN`, no harness error, 14.29s (`log/bs6/repro-83aa-after2/results.csv`) |
| `ee5fb3e-11.normalised` | `ERROR`, exit 134, missing `result.json`, allocation of 2589149280 bytes failed (`log/bs6/repro-ee5-before/results.csv`) | `UNKNOWN`, no harness error, 26.91s (`log/bs6/repro-ee5-after2/results.csv`) |

The allocation phase is now classified separately:

- `ee5` failed immediately on dense watcher header allocation:
  `2 * 53940610 * size_of::<Vec<Watcher>>() = 2589149280` bytes.
- `83aa` failed during later predictable preprocessing peak allocation, so the
  final guard also accounts for watcher entries, occurrence entries, and
  inline-abstraction arena/relocation peak memory.

Profile no-regression evidence:

| Before | After | Solved | PAR-2 before | PAR-2 after | Delta | Verdict |
|---|---|---:|---:|---:|---:|---|
| `log/profile-compare-solver11-2026-05-19-1647/results.csv` | `log/bs6/profile-after/results.csv` | 9/11 both | 710.454 | 712.548 | +2.094 | PASS, no status changes |

Decision: memory aborts under the standard 16 GB gate are no longer unexplained
missing-result-contract failures. They are clean `UNKNOWN` results and should not
block Phase 1 promotion on output-contract reliability grounds, though they still
count as unsolved rows in performance gates.
