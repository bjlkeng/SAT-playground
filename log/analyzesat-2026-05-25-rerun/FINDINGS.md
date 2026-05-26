# AnalyzeSAT Findings - 2026-05-25 Rerun

Target: `solver/11-kissat-port` at `a03645a` on branch
`side-01-analyzesat-20260525-234506`.

Benchmark: `benchmarks/profiling`, 10 instances, 300s timeout, 295s internal
wall limit, 16 GB memory.

## Summary

The current default profile solves all 10 profiling instances, but the newly
default-on `SAT_LUCKY` pass is a net regression on this suite. Disabling lucky
keeps 10/10 solved and improves PAR-2 from `799.250` to `759.720` seconds.
Lucky solves only `battleship-16-31-sat` here, saving `22.965s`, but its failed
attempts add `62.495s` across the other rows.

`SAT_USE_LBD=on` as metadata only is not the blocker: it exactly matches the
default search work counters. Standalone `SAT_RESTART=kissat-ema` remains a
hard failure, solving only 8/10 and returning `UNKNOWN` on `mp1` and `case9`,
both solved by the baseline.

The hard failure stopped the matrix before `D_focused_stable`,
`E_focused_ticks`, and `F_full_stack`. Continuing after a baseline-solved
`UNKNOWN` would violate the solver-work gate for this repo.

## Config Results

| Config | Solved | PAR-2 | Unknowns | Notes |
| --- | ---: | ---: | --- | --- |
| `A_default` | 10/10 | 799.250 | none | Current default, `SAT_LUCKY=on` |
| `A_lucky_off` | 10/10 | 759.720 | none | `SAT_LUCKY=off`; best row aggregate in this rerun |
| `B_lbd_metadata` | 10/10 | 764.328 | none | Same conflicts/decisions/props as `A_default` |
| `C_lbd_ema` | 8/10 | 1759.969 | `mp1`, `case9` | Hard stop: baseline-solved rows became `UNKNOWN` |

Derived CSVs:

- `config_summary.csv`
- `config_instance_summary.csv`
- `decomposition.csv`
- `lucky_delta.csv`
- `trace_summary_mp1.csv`
- `reference_failure_summary.csv`

## Lucky Pass Delta

Positive delta means current default is slower than `SAT_LUCKY=off`.

| Instance | Default | Lucky off | Delta |
| --- | ---: | ---: | ---: |
| `sudoku-N30-12` | 207.577 | 188.524 | +19.053 |
| `6s299b685_Iter30` | 20.197 | 16.117 | +4.080 |
| `REGRandom-K4-L1-Seed40` | 61.471 | 57.389 | +4.082 |
| `mp1-Nb7T46` | 46.758 | 44.589 | +2.169 |
| `Kakuro-easy-112` | 236.213 | 214.364 | +21.849 |
| `SCPC-500-13` | 15.319 | 13.502 | +1.817 |
| `velev-pipe-sat-1.0-b7` | 72.847 | 65.479 | +7.368 |
| `brocard_problem_large` | 11.277 | 8.923 | +2.354 |
| `battleship-16-31-sat` | 0.090 | 23.055 | -22.965 |
| `case9` | 127.501 | 127.778 | -0.277 |

Net: `A_default - A_lucky_off = +39.530s`, so default-on lucky loses despite
the battleship win.

## EMA Failure Decomposition

The two `C_lbd_ema` hard failures are not speed-only regressions. Both have a
large search-work increase.

| Instance | Baseline result/time | EMA result/time | Conflict ratio | Prop-rate speed ratio | Net work x speed |
| --- | --- | --- | ---: | ---: | ---: |
| `mp1-Nb7T46` | SAT / 46.758s | UNKNOWN / 296.636s | 6.452 | 1.781 | 11.490 |
| `case9` | SAT / 127.501s | UNKNOWN / 295.341s | 2.086 | 1.111 | 2.318 |

`mp1` trace confirms immediate trajectory divergence. At 20k conflicts:

- default: `88,888` decisions, `12,883,322` propagations, `68` restarts
- EMA: `246,624` decisions, `10,213,716` propagations, `274` restarts

This is not a late phase-boundary coin flip. The restart regime changes the
search shape from the start.

## Reference Checks

The failed rows are solved by reference Kissat binaries:

| Solver | `mp1` | `case9` | `battleship` |
| --- | ---: | ---: | ---: |
| `kissat-latest` | 8.489s SAT | 77.769s SAT | 0.180s SAT |
| `kissat-sc2024` | 225.502s SAT | 32.170s SAT | 7.470s SAT |

This points to integration gaps in solver 11, not to EMA restarts being
inherently unsuitable for these formulas.

## Perf

`perf` could not collect counters on this host:

`perf_event_paranoid setting is 4`

The saved stderr is `perf_C_lbd_ema_mp1.stderr`.

## Artifacts

The raw run directories are under `log/analyzesat-2026-05-25-rerun/` in this
worktree. The committed artifacts preserve the scripts, context, summaries,
reference results, and perf denial message. Raw per-instance stdout/stderr and
proof/model files remain in the worktree for local inspection.
