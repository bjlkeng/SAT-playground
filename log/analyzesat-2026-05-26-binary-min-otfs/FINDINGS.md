# Bottleneck Analysis - solver/11-kissat-port - 2026-05-26

Target: `solver/11-kissat-port` at `9143376` on branch
`side-01-analyzesat-20260526-071123`.

Focus: binary implication fast path, clause minimization, and learned-clause
OTFS. This run intentionally looked away from the just-covered lucky and
single-mode Kissat EMA findings.

## Executive Summary

- `SAT_CLAUSE_MIN=off` is not a harmless ablation. On Sudoku, current default
  recursive minimization solves UNSAT in `207.092s`, while minimization off
  returns `UNKNOWN` at `295.880s`.
- `SAT_BINARY_FAST=on` currently inherits that unsafe path because
  `src/config.rs:1052-1053` silently forces `clause_min_mode=off` unless
  `SAT_CLAUSE_MIN` is explicit. On Sudoku, `SAT_BINARY_FAST=on` returns
  `UNKNOWN`; `SAT_BINARY_FAST=on SAT_CLAUSE_MIN=recursive-limited` solves in
  `256.316s`.
- The existing Kissat-depth alignment idea is supported on this critical row:
  `SAT_MINIMIZE_DEPTH_LIMIT=1000` solves Sudoku in `207.935s` with identical
  conflicts/propagations/learned-literal count to default.
- `FEATURES.csv` and `FEATURES.md` still claim `SAT_LUCKY` is promoted for
  default/fast even though code, tests, and README now say it is opt-in.

## Config Matrix Results

The full matrix hard-stopped after `B_no_min` produced a baseline-solved
`UNKNOWN`, per repo policy.

| Config | Rows completed | Solved | Unknown | Completed-row PAR-2 | Notes |
| --- | ---: | ---: | ---: | ---: | --- |
| `A_default` | 10 | 10 | 0 | 845.424 | Current default, lucky off |
| `B_no_min` | 2 | 1 | 1 | 618.649 | Stopped after Sudoku `UNKNOWN`; Iter30 completed |

Baseline per-instance highlights:

| Instance | Result | Time |
| --- | --- | ---: |
| Sudoku | UNSAT | 207.092 |
| Kakuro | SAT | 258.264 |
| velev | SAT | 87.526 |
| case9 | SAT | 127.386 |

## Sudoku Minimization Sweep

| Run | Result | Time | Conflict ratio | Speed ratio | Learned-lits ratio | Max learned buffer |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| default recursive | UNSAT | 207.092 | 1.000 | 1.000 | 1.000 | 1,698 |
| min off | UNKNOWN | 295.880 | 1.225 | 1.165 | 2.244 | 7,068 |
| basic | UNSAT | 253.482 | 1.199 | 1.081 | 1.739 | 4,381 |
| inblock | UNSAT | 237.822 | 1.013 | 1.085 | 1.098 | 2,165 |
| recursive depth 1000 | UNSAT | 207.935 | 1.000 | 1.004 | 1.000 | 1,698 |
| binary-fast auto min-off | UNKNOWN | 295.937 | 0.942 | 1.424 | 1.543 | 7,447 |
| binary-fast recursive | UNSAT | 256.316 | 0.887 | 1.414 | 0.882 | 1,781 |

Interpretation:

- Turning minimization off grows learned clauses dramatically: final learned
  literals increase from `29.4M` to `66.0M`, max learned buffer from `1,698` to
  `7,068`, proof-added literals from `49.6M` to `85.2M`, and the row times out.
- Basic and in-block minimization rescue correctness/status but remain slower
  than recursive. Recursive minimization is the quality boundary on this row.
- Binary-fast with recursive minimization reduces conflicts and learned literals,
  but it is still slower because propagation throughput drops by about `41%`
  (`speed_ratio=1.414`). That points to binary-fast execution overhead, not a
  worse search trajectory.

## Reference Solver Live Comparison

| Solver | Sudoku result | Time |
| --- | --- | ---: |
| solver 11 default | UNSAT | 207.092 |
| kissat-latest | TIMEOUT | 300.022 |
| kissat-sc2024 | UNSAT | 202.951 |

Solver 11 default is competitive with `kissat-sc2024` and beats
`kissat-latest` on this row, but only when recursive minimization is active.
This is an advantage to preserve while tuning binary propagation.

## Work x Speed Decomposition

For `SAT_CLAUSE_MIN=off`, measured wall ratio vs default is `1.429`; the
work-speed product is `1.427`, so the slowdown is cleanly explained by more
conflicts plus lower propagation throughput. No hidden third factor is needed.

For `SAT_BINARY_FAST=on SAT_CLAUSE_MIN=recursive-limited`, conflict work improves
(`0.887x` default), but propagation throughput loses (`1.414x` slower), giving a
net `1.254x` predicted slowdown and `1.238x` measured slowdown. This confirms
binary-fast has an execution-cost problem even when search quality improves.

## Reference Diffs - Implementation Gaps

Rust:

- `solver/11-kissat-port/src/config.rs:1052-1053` forces
  `clause_min_mode=Off` when `SAT_BINARY_FAST=on` and `SAT_CLAUSE_MIN` is not
  explicit.
- `solver/11-kissat-port/src/main.rs:5970-6026` implements basic, recursive, and
  in-block learned-clause minimization.
- `solver/11-kissat-port/src/main.rs:6352` minimizes every learned conflict
  clause before LBD/restart accounting.

Kissat:

- `kissat-sc2024/src/options.h:80-82` enables minimization by default with
  `minimizedepth=1000`.
- `kissat-sc2024/src/minimize.c:95-140` recursively minimizes literals through
  binary and long reasons with a depth cap.
- `kissat-sc2024/src/analyze.c:560-564` sorts, minimizes, and shrinks the
  learned clause before learning it.

The actionable mismatch is not that solver 11 has recursive minimization; it is
that one opt-in path (`SAT_BINARY_FAST=on`) silently turns that essential
mechanism off.

## Hardware Counter Results

`perf` is unavailable on this host:

`perf_event_paranoid setting is 4`

The saved stderr is `perf_clause_min_off_sudoku.stderr`.

## Beads

New:

- `SAT-playground-a0f`: do not let `SAT_BINARY_FAST` silently disable required
  minimization.
- `SAT-playground-otk`: sync the `SAT_LUCKY` feature ledger after default
  demotion.

Updated:

- `SAT-playground-k25`: added evidence that `SAT_MINIMIZE_DEPTH_LIMIT=1000` is
  safe on the Sudoku row where recursive minimization is essential.

## Artifact Paths

- Ablation script: `log/analyzesat-2026-05-26-binary-min-otfs/run_ablation.sh`
- Reference script: `log/analyzesat-2026-05-26-binary-min-otfs/run_reference_sudoku.sh`
- Derived summary: `log/analyzesat-2026-05-26-binary-min-otfs/sudoku_clause_min_summary.csv`
- Baseline results: `log/analyzesat-2026-05-26-binary-min-otfs/A_default/results.csv`
- Hard-fail results: `log/analyzesat-2026-05-26-binary-min-otfs/B_no_min/results.csv`
- Reference results: `log/analyzesat-2026-05-26-binary-min-otfs/reference_summary.csv`
