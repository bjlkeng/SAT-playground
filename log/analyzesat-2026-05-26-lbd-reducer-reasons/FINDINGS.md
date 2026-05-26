# AnalyzeSAT Findings: LBD Reducer / Reason Metadata

Date: 2026-05-26
Solver: `solver/11-kissat-port`
HEAD: `025fe25083778bad067845835d43606b05489356`
Artifact root: `log/analyzesat-2026-05-26-lbd-reducer-reasons`

## Summary

Fresh area: LBD metadata, reason-LBD updates, and `SAT_REDUCE=lbd-tiered` learned-clause reduction. This avoids the prior restart, branch/phase, lucky-unit, preprocessing, proof, binary-minimization, and clause-order investigations.

The actionable finding is that the current LBD-tiered reducer is still unsafe as an opt-in policy. It turns the clean mp1 target from a 45.384s SAT solve into `UNKNOWN` at the 295s solver wall limit. The failure reproduces with and without reason-side LBD updates, and simple schedule probes do not rescue it. The dominant cause is search work growth from reducer semantics, not LBD bookkeeping alone.

## Profiling Matrix

The full profiling-suite matrix was stopped after `D_lbd_tiered` produced an `UNKNOWN` on mp1, because the repo rules treat a baseline-solved `UNKNOWN` as a failed experiment requiring root-cause debugging.

| Config | Env | Solved | Unknown | PAR-2 | Delta |
| --- | --- | ---: | ---: | ---: | ---: |
| A_default | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | 0 | 853.876 | 0.000 |
| B_lbd_metadata | `SAT_USE_LBD=on` | 10/10 | 0 | 859.259 | +5.383 |
| C_reason_lbd | `SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on` | 10/10 | 0 | 861.813 | +7.937 |
| D_lbd_tiered | `SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_REDUCE=lbd-tiered` | 3/4 run before abort | 1 | 946.494 partial | +92.618 partial |

Raw data:
- `configs.tsv`
- `summary_table.csv`
- `work_speed_detail.csv`
- `A_default/`, `B_lbd_metadata/`, `C_reason_lbd/`, `D_lbd_tiered/`

## Clean Target Reruns

After stopping stale benchmark jobs from earlier sessions, I reran the decisive rows as single-instance targets.

| Target | Config | Result | Time | Wall Ratio | Work Ratio | Speed Ratio | Reductions | Deleted Learned | GC |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| mp1 | A_default | SAT | 45.384 | 1.000 | 1.000 | 1.000 | 0 | 0 | 1 |
| mp1 | C_reason_lbd | SAT | 46.867 | 1.033 | 1.000 | 1.032 | 0 | 0 | 1 |
| mp1 | D_lbd_tiered | UNKNOWN | 296.225 | 6.527 | 5.728 | 1.192 | 237 | 2,245,127 | 58 |
| mp1 | I_lbd_tiered_no_reason | UNKNOWN | 296.142 | 6.525 | 6.170 | 1.140 | 249 | 2,365,573 | 60 |
| mp1 | G_lbd_tiered_delayed | UNKNOWN | 296.200 | 6.527 | 5.544 | 1.220 | 226 | 2,158,076 | 56 |
| mp1 | H_lbd_tiered_slow_interval | UNKNOWN | 296.459 | 6.532 | 5.718 | 1.146 | 42 | 2,320,667 | 17 |
| REGRandom | A_default | UNSAT | 59.179 | 1.000 | 1.000 | 1.000 | 0 | 0 | 1 |
| REGRandom | D_lbd_tiered | UNSAT | 98.385 | 1.662 | 1.700 | 0.997 | 256 | 2,596,793 | 32 |
| REGRandom | G_lbd_tiered_delayed | UNSAT | 82.799 | 1.399 | 1.313 | 1.066 | 209 | 1,951,720 | 28 |
| REGRandom | H_lbd_tiered_slow_interval | UNSAT | 67.910 | 1.148 | 0.918 | 1.209 | 8 | 350,646 | 2 |

Interpretation:
- Reason-LBD is not the root cause. `C_reason_lbd` is a small speed-only cost on mp1, while `D_lbd_tiered` is a 5.7x work explosion.
- No-reason tiered still fails mp1, so the reducer itself is enough to cause `UNKNOWN`.
- Delaying the first reduction and slowing the interval do not rescue mp1. They reduce REGRandom damage but do not make it a win.
- The worst mp1 rows have both work growth and speed loss, but work dominates the failure.

Raw data:
- `target_summary.csv`
- `target-mp1-*/stats.jsonl`
- `target-regrandom-*/stats.jsonl`

## Trace Evidence

Trace files:
- `traces/A_default.trace_extract.txt`
- `traces/D_lbd_tiered.trace_extract.txt`

On mp1 with `SAT_TRACE_SEARCH_INTERVAL=100000 SAT_LIMIT_WALL_SEC=120`, default solves at 425,229 conflicts in 46.723 search seconds. The tiered reducer reaches 400,000 conflicts at 46.771 search seconds but has already run 71 reductions and holds only 125,171 live learned clauses. It then continues past 900,000 conflicts and returns `UNKNOWN` at the 120s trace limit.

This points to a search-trajectory loss after deletion starts, not a preprocessing mismatch. Preprocessing stats match across the target runs: 57,935 variables, 229,320 post-preprocess clauses, and 797,524 post-preprocess literals.

## Source Gap

Local code:
- `solver/11-kissat-port/src/main.rs:5253` triggers LBD-tiered reduction on conflict schedule or hard learned-literal budget.
- `solver/11-kissat-port/src/main.rs:5975` filters candidates by tier and `used_recently`.
- `solver/11-kissat-port/src/main.rs:6032` collects candidates, sorts high-LBD/large candidates first, and then deletes candidates until `projected_lits <= learned_lit_budget`.
- `solver/11-kissat-port/src/main.rs:5323` schedules the next LBD reduction with `sqrt(reduce_db_calls) * SAT_REDUCE_DB_INTERVAL`.

Reference source:
- Upstream Kissat `reduce.c` collects reducible redundant clauses, decrements `used`, skips reason clauses, protects recently-used tier clauses, ranks by high glue and large size, then deletes a computed fraction of reducible candidates rather than deleting until a learned-literal budget is met: https://github.com/arminbiere/kissat/blob/master/src/reduce.c
- Upstream Kissat `tiers.c` computes tier limits from recent glue-use histograms: https://github.com/arminbiere/kissat/blob/master/src/tiers.c

The port has already captured some Kissat mechanics, but the deletion target is structurally different. On mp1, that difference is enough to delete millions of learned clauses and induce a much worse CDCL path.

## Recommendations

1. Prioritize `SAT-playground-qmz`: replace delete-until-learned-literal-budget in `reduce_db_lbd_tiered` with Kissat-style fraction-of-ranked-reducibles. Keep the hard learned-literal budget as an emergency trigger, not the scheduled deletion target.
2. After `SAT-playground-qmz`, retest `SAT-playground-z70` and `SAT-playground-5b2.2.44`. The current data says tier1/tier2 eviction tweaks are second-order until the deletion target is fixed.
3. Add a focused regression test or benchmark guard for mp1-style behavior: `SAT_REDUCE=lbd-tiered` must not turn a default-solved row into `UNKNOWN`.
4. Do not promote `SAT_REDUCE=lbd-tiered`, `SAT_LBD_UPDATE_PROP_REASONS`, or reducer schedule changes from these data. The only safe conclusion is a code-level reducer fix.

## Beads Updated

- `SAT-playground-qmz`: added mp1 and REGRandom evidence supporting fraction-based deletion.
- `SAT-playground-z70`: noted it should remain blocked behind `qmz`.
- `SAT-playground-5b2.2.44`: noted tier2/tier1 eviction work is likely second-order until `qmz` lands.

## Perf

`perf stat` was attempted and blocked by `perf_event_paranoid=4`. See `perf_attempt.txt`.
