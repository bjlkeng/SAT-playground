# Deeper Findings - Branch / Phase / Chrono AnalyzeSAT

## 1. The Target-Phase Failure Is A Semantic Port Gap

`SAT_PHASE=target-then-saved` is available in the runtime schema as a general phase policy, but the implementation it activates is not equivalent to Kissat's target phase execution model.

Kissat separates three concerns:

- Target phase selection is conditional in `decide.c`: target phases are used only when the target option is enabled and either the solver is stable or the stronger target option is selected.
- Target and best phase snapshots are updated in `backtrack.c` only when `solver->stable` is true.
- Rephase copies saved phases into target phases and resets target assignment counters at scheduled stable-mode rephase boundaries.

Solver 11 single-mode target phase instead captures a target prefix whenever the trail grows deeper in the no-conflict scheduling path, then resets it at every single-mode restart. On Sudoku this creates a bad feedback loop:

- `phase_save_target=527,257`
- `phase_target_used=10,935,538`
- learned literals grow from `29.4M` to `55.7M`
- the solver reaches `347,921` conflicts and returns `UNKNOWN`

This is too large to treat as ordinary SAT trajectory variance. The feature should either be rejected outside focused/stable mode or implemented with Kissat's stable-only capture/use semantics.

## 2. Saved Phase Did Not Change Search Work

The full-suite saved-phase row looked attractive: PAR-2 `764.124` versus default `868.674`. But every row had identical conflicts, decisions, propagations, and restarts against default. That means the phase policy did not change the actual search trajectory.

The Sudoku repeat confirms the full-run speed delta was not stable:

| Run | Result | Elapsed | Conflicts | Decisions | Propagations |
|---|---|---:|---:|---:|---:|
| default repeat | `UNSAT` | `186.327s` | `259,775` | `6,772,770` | `1,312,437,897` |
| saved repeat | `UNSAT` | `195.907s` | `259,775` | `6,772,770` | `1,312,437,897` |

The right conclusion is "do not promote based on one noisy PAR-2 run." If saved phase is revisited, use lower-noise counters or multiple repeats.

## 3. Occurrence Branch Order Is A Formula-Family Split

Occurrence branch order is valuable evidence, but not a profile candidate:

- Kakuro: `247.447s -> 118.695s`, work ratio `0.399`
- Sudoku: `234.665s -> 215.667s`
- mp1: `47.085s -> 255.289s`, work ratio `4.722`
- battleship: `23.525s -> 78.853s`

The implementation changes variable order and default polarity together: `BranchMode::Occurrence` sorts variables by original occurrence count and sets `default_phase=TRUE`. Any future experiment should split those axes:

1. Minisat order + true initial phase.
2. Occurrence order + false initial phase.
3. Occurrence order + true initial phase.

Then compare against post-preprocess formula stats to see whether Kakuro-like rows can be routed safely.

## 4. Reference Comparison

On the same decompressed Sudoku input:

- solver 11 default trace solved in `218.367s` elapsed / `214.064s` search.
- kissat-sc2024 solved in `187.37s`.
- kissat-latest solved in `295.18s`.
- solver 11 target-phase returned `UNKNOWN` at `295.741s`.

The reference result does not justify copying target phase into single-mode. It argues the opposite: Kissat's target phase is coupled to its stable/focused mode model, backtrack boundary updates, and rephase schedule.

## 5. Actionable Follow-Up Order

1. Fix `SAT_PHASE=target-then-saved` single-mode safety first (`SAT-playground-6ci`). This is a status failure.
2. Keep the existing phase-capture optimization bead scoped to capture cost only after semantics are safe (`SAT-playground-5b2.2.18.6`).
3. Treat `SAT_BRANCH_MODE=occurrence` as a future adaptive-routing experiment, not a default candidate (`SAT-playground-4b4`).
