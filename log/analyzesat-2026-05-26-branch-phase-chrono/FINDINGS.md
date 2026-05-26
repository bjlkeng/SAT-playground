# Bottleneck Analysis - solver/11-kissat-port - 2026-05-26

## Executive Summary

- `SAT_PHASE=target-then-saved` is status-unsafe in single-mode search: Sudoku returned `UNKNOWN` at `295.852s` while default solved `UNSAT`.
- The implementation gap is semantic, not a knob issue: Rust captures and uses target phases in single-mode search, while Kissat gates target use/capture to stable-mode semantics unless the stronger target option is explicitly selected.
- `SAT_BRANCH_MODE=occurrence` is status-safe but globally worse: it helps Kakuro and Sudoku, but destroys mp1 and battleship trajectory, raising PAR-2 from `868.674` to `1059.313`.
- `SAT_PHASE=saved` looked like a broad win in the first full run, but a one-row repeat reversed the Sudoku timing while preserving identical conflicts/decisions/propagations. Treat it as timing noise, not a promotion signal.

## Config Matrix Results

| Config | Env | Solved | PAR-2 | Notes |
|---|---|---:|---:|---|
| `A_default` | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `868.674` | Current baseline at `6d76772`. |
| `B_occurrence` | `SAT_BRANCH_MODE=occurrence` | 10/10 | `1059.313` | Status-safe but regresses aggregate. |
| `C_saved_phase` | `SAT_PHASE=saved` | 10/10 | `764.124` | Full-run timing looked faster, but repeat disproved a stable win. |
| `D_target_phase` | `SAT_PHASE=target-then-saved` | 0/1 before stop | `UNKNOWN` on Sudoku | Baseline-solved failure; run stopped per solver rules. |

Rows `E_chrono`, `F_occurrence_chrono`, and `G_occurrence_saved` were intentionally not run after the `D_target_phase` baseline-solved `UNKNOWN`.

## Reference Solver Live Comparison

Sudoku one-row reference check:

| Solver | Result | Wall | Max RSS |
|---|---|---:|---:|
| solver 11 default trace | `UNSAT` | `218.367s` | `859 MB` |
| solver 11 target-phase trace | `UNKNOWN` | `295.741s` | `859 MB` |
| kissat-sc2024 | `UNSAT` | `187.37s` | `374 MB` |
| kissat-latest | `UNSAT` | `295.18s` | `376 MB` |

This places the target-phase failure behind both the repo default and the sc2024 reference on the same input.

## Work x Speed Decomposition

Important rows from `work_speed.csv`:

| Config / instance | Result | Wall ratio | Work ratio | Speed ratio | Dominant cause |
|---|---|---:|---:|---:|---|
| `B_occurrence` / Sudoku | `UNSAT` | `0.919` | `1.046` | `0.913` | Mixed, small win. |
| `B_occurrence` / Kakuro | `SAT` | `0.480` | `0.399` | `1.217` | Work win despite slower per-prop speed. |
| `B_occurrence` / mp1 | `SAT` | `5.422` | `4.722` | `1.134` | Trajectory loss. |
| `D_target_phase` / Sudoku | `UNKNOWN` | `1.261` before wall limit | `1.216` | `1.186` | Work + DB bloat; status failure. |

The target-phase trace is worse than the baseline trace by:

| Metric | Default trace | Target-phase trace |
|---|---:|---:|
| result | `UNSAT` | `UNKNOWN` |
| conflicts | `259,775` | `347,921` |
| decisions | `6,772,770` | `11,347,958` |
| propagations | `1,312,437,897` | `1,472,258,381` |
| restarts | `617` | `827` |
| learned clauses final | `212,310` | `329,323` |
| learned lits final | `29,406,410` | `55,669,369` |
| proof added literals | `49,620,264` | `75,744,478` |

## Reference Diffs - Implementation Gaps

### Single-mode target phase is not Kissat-faithful

Rust:

- `solver/11-kissat-port/src/main.rs:4439-4447` uses `target_phase` for every `PhasePolicy::TargetThenSaved` decision.
- `solver/11-kissat-port/src/main.rs:4581-4601` captures target phases in single-mode search whenever the current trail is deeper than the previous target prefix.
- `solver/11-kissat-port/src/main.rs:4891-4894` resets target phase on single-mode restart, so this policy repeatedly records and consumes short-lived restart-local target prefixes.

Kissat sc2024:

- `kissat-sc2024/src/decide.c:161-166` only enables target phases if `target` is on and either the solver is in stable mode or the target option is stronger than the default.
- `kissat-sc2024/src/backtrack.c:38-43` returns without updating target/best phases unless `solver->stable`.
- `kissat-sc2024/src/backtrack.c:51-68` saves target/best phases at stable-mode backtrack boundaries, not at every single-mode no-conflict decision boundary.

Prediction: single-mode target policy should overuse stale or locally captured target prefixes, raising decisions/conflicts and learned DB size on trajectory-sensitive UNSAT rows.

Verification: Sudoku target-phase trace used target phase for `10,935,538` decisions and returned `UNKNOWN`; default used legacy phase for `6,772,770` decisions and solved `UNSAT`.

### Occurrence branch order is family-specific

Rust:

- `solver/11-kissat-port/src/main.rs:1739-1746` sorts branch order by descending original literal occurrence count.
- `solver/11-kissat-port/src/main.rs:1751-1754` also flips default phase by branch mode: `minisat` starts false, `occurrence` starts true.

The run shows this is not a pure variable-order experiment; it changes both variable order and initial polarity. That explains the sharp family split and makes a global promotion unsafe.

## Trajectory Analysis

The target-phase and default Sudoku traces already differ at the first `50k` conflicts:

| Conflicts | Default decisions | Target decisions | Default trail | Target trail |
|---:|---:|---:|---:|---:|
| `50k` | `1,068,246` | `1,476,961` | `43,052` | `55,862` |
| `100k` | `2,112,667` | `3,049,638` | `6,836` | `43,642` |
| `200k` | `4,449,171` | `6,506,351` | `54,715` | `44,821` |
| `250k` | `6,545,145` | `8,006,381` | `19,861` | `29,499` |

This is not late phase-boundary chaos. The policy sends the search down a different, deeper decision path almost immediately.

## Hardware Counter Results

`perf stat` was blocked on this host:

```text
perf_event_paranoid setting is 4
Access to performance monitoring and observability operations is limited.
```

The denial is saved at `perf_default_sudoku.stderr`.

## Parameter Sweep Results

No sweep was run after the target-phase `UNKNOWN`. The correct next step is a semantic guard/fix, not tuning.

## Code-Level Recommendations

1. `src/config.rs` / `src/main.rs` - reject or gate `SAT_PHASE=target-then-saved` and `best-then-target-then-saved` outside `SAT_SEARCH_MODE=focused-stable`, or make single-mode target capture/use match Kissat's stable-mode boundaries before allowing the config. Reference: `kissat-sc2024/src/decide.c:161-166`, `backtrack.c:38-68`.
2. `src/main.rs` - keep `SAT_BRANCH_MODE=occurrence` unpromoted and document its formula-family split. If revisited, route it through post-preprocess formula classification and validate with a full profile gate.
3. Do not promote `SAT_PHASE=saved` from this run. It had identical search work to default, and the one-row repeat showed the timing delta is not stable.

## Rejected Sweeps / Non-Issues

- `SAT_PHASE=saved` is not an actionable speed optimization from this evidence. Full-run PAR-2 improved, but the repeated Sudoku one-row run had default at `186.327s` and saved at `195.907s` with identical conflicts/decisions/propagations.
- `SAT_BRANCH_MODE=occurrence` is not a global profile candidate because mp1 regressed from `47.085s` to `255.289s` and battleship from `23.525s` to `78.853s`.

## Beads

- Created `SAT-playground-6ci`: reject or gate single-mode target phase policies.
- Created `SAT-playground-4b4`: keep occurrence branch order diagnostic until formula-gated.
- Added a note to `SAT-playground-5b2.2.18.6` clarifying that its O(delta) phase-capture task is separate from the target-phase semantic/status bug.

## Artifact Paths

- Matrix: `log/analyzesat-2026-05-26-branch-phase-chrono/config_matrix.psv`
- Ablation harness: `log/analyzesat-2026-05-26-branch-phase-chrono/run_ablation.sh`
- Summary tables: `config_summary.csv`, `work_speed.csv`, `sudoku_trace_summary.csv`, `sudoku_repeat_summary.csv`, `reference_sudoku.csv`
- Raw config logs: `A_default/`, `B_occurrence/`, `C_saved_phase/`, `D_target_phase/`
- Traces: `traces/default_sudoku.stderr`, `traces/target_phase_sudoku.stderr`
- Perf denial: `perf_default_sudoku.stderr`
