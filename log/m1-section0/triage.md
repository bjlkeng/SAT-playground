# M.1 Section 0 Baseline and Default-Profile Triage

Date: 2026-05-19

Scope: Section 0 completed through local CI, feature ledger, reproducibility scaffolding, solver 10 parity preservation, and the solver 10 vs solver 11 overhead gate.

Primary evidence:

- `log/solver11-overhead-2026-05-19-09-16-46/summary.json`
- `log/solver11-overhead-2026-05-19-09-16-46/config.json`
- `log/profile-compare-solver10-2026-05-19/results.csv`
- `log/profile-compare-solver11-2026-05-19/results.csv`
- `log/profile-compare-solver10-2026-05-19/summary.log`
- `log/profile-compare-solver11-2026-05-19/summary.log`

## Summary

Decision: keep the Section 0 infrastructure and proceed to Phase 1 bead `SAT-playground-5b2.2.1` (`[1.0] Reason and propagation scaffold`).

Section 0 did not intentionally change search policy, preprocessing semantics, proof output, or model reconstruction. The current default profile is suitable as the Phase 1 baseline. No Section 0 feature is promoted as a performance feature, and no config/profile change is accepted for speed.

## Status, proof, and model failures

No status, proof, or model failures are currently attributed to Section 0.

The latest repeated overhead gate reported matching statuses for solver 10 and solver 11 on both guard instances and matching core search counters on the counter-parity target:

- conflicts: `551925` for both solvers
- decisions: `679410` for both solvers
- propagations: `43725178` for both solvers
- restarts: `1149` for both solvers
- learned clauses: `9631` for both solvers
- reduce-db calls: `138` for both solvers

## Wins

Repeated overhead gate wins, solver 11 vs solver 10 median runtime:

1. `benchmarks/profiling/feistel_b64_k57_r18.cnf`: `8.433s` vs `8.504s`, `-0.84%`.
2. `benchmarks/profiling/random_v285_s2.cnf`: `8.330s` vs `8.387s`, `-0.68%`.

The broader single-run profile comparison did not show per-instance runtime wins for solver 11. That single-run result is treated as lower-confidence than the repeated overhead gate for Section 0 overhead assessment.

## Regressions

Single-run 11-instance profile comparison, solver 11 vs solver 10:

1. `feistel_b64_k57_r18`: `8.879s` vs `8.470s`, `+4.83%`.
2. `feistel_b64_k52_r17`: `23.325s` vs `22.313s`, `+4.54%`.
3. `random_v292_s4`: `14.161s` vs `13.595s`, `+4.16%`.
4. `random_v355_s3`: `23.471s` vs `22.543s`, `+4.12%`.
5. `feistel_b64_k32_r22`: `51.640s` vs `49.626s`, `+4.06%`.
6. `random_v285_s2`: `8.745s` vs `8.439s`, `+3.63%`.
7. `46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized`: `56.201s` vs `54.357s`, `+3.39%`.
8. `557d7d4db5399188f62bc39598c6d868-mp1-Nb7T46`: `41.962s` vs `41.081s`, `+2.14%`.
9. `1d18837c0ced5c18a3a4693993e61728-SC25_Timetable_C_392_E_45_Cl_25_D_7_T_50.normalised`: `3.926s` vs `3.854s`, `+1.87%`.
10. `0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12`: both timed out at `120s`, no solved-status delta.

Interpretation: this single-run profile comparison is useful as a reminder to keep profile-noise discipline, but it is not enough to reject Section 0. The repeated overhead gate uses three repeats, checks statuses, and verifies core counter parity. Its median deltas are inside the `1.5%` overhead threshold and slightly favor solver 11 on the two guard instances.

## Lost and newly solved instances

Lost solved instances: none.

Newly solved instances: none.

The broader single-run profile comparison solved `9/11` for both solvers with `6 SAT`, `3 UNSAT`, and `2` timeouts. The timeouts were the same two instances:

- `0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12`
- `5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7`

## Bottleneck categories

- `feistel_b64_k57_r18`: benchmark-noise / search-trajectory. Core counters match exactly in the overhead gate, so no Section 0 algorithmic bottleneck is indicated.
- `random_v285_s2`: benchmark-noise / search-trajectory. Repeated gate shows solver 11 within threshold and slightly faster; single-run profile compare shows a small slowdown.
- `0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12`: search-trajectory, still timed out in both solver 10 and solver 11.
- `5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7`: search-trajectory after preprocessing parity; both solver 10 and solver 11 timed out at `120s`.

No evidence currently points to proof-throughput, model reconstruction, occurrence-list cost, memory/GC, or preprocessing-shrink regressions from Section 0.

## Keep, tune, revert decisions

- Keep Section 0 config/result/stats/oracle infrastructure.
- Keep the feature ledger and default-off future flags.
- Keep the overhead regression gate and its `1.5%` repeated-median threshold.
- Keep the local CI scripts and reproducibility scaffolding.
- Tune nothing before Phase 1; the default profile is the baseline, not a performance profile.
- Revert nothing from Section 0.

## Profile changes

Accepted profile changes: none.

Rejected profile changes: none.

Config flags to remove from candidate profiles: none. Future-feature flags remain present but default off or parked according to `solver/11-kissat-port/FEATURES.md`.

## Holdout summary

No holdout promotion run is claimed for M.1. The `benchmarks/iteration/holdout` directory currently contains no instance files, and Section 0 introduced no promoted performance feature that should be tuned or accepted on holdout data.

## Confidence and noise decision

The Section 0 overhead finding is within the measured confidence/noise band. The repeated overhead gate passed all configured checks:

- threshold: `1.5%`
- repeats: `3`
- timeout: `120s`
- failed instances: none
- status parity: true
- core counter parity: true

The single-run 11-instance profile comparison reports PAR-2 `712.310` for solver 11 vs `704.278` for solver 10, a `+1.14%` solver 11 delta with identical solved count. Because it is single-run and the repeated overhead gate shows no core-counter divergence, this is treated as noise rather than a Section 0 blocker.

## Next two beads

1. `SAT-playground-5b2.2.1` (`[1.0] Reason and propagation scaffold`): highest-leverage dependency; unlocks LBD metadata, binary-fast representation work, decision heap cleanup, and several downstream Phase 1 tasks.
2. `SAT-playground-5b2.2.2` (`[1.0a] Temporary-assumption propagation context`): should follow immediately after the reason scaffold so probes, vivification, HBR, and transitive reduction do not reuse ad hoc root-trail mutation.

Do not begin LBD policy work before the reason scaffold is complete and verified.
