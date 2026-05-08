# 10-bve-preprocess

This iteration is intended to become the MiniSat `simp`-style preprocessing solver on top of
`09-root-simp-opts`. The code in `src/main.rs` is currently still the `09` root-level
simplification baseline plus its profiling-driven micro-optimizations, so `10` should be treated as
the staging area for the next faithful preprocessing port rather than as a completed BVE solver.

## Current State

What is already present in the current `10` tree:

- the full `09` CDCL baseline with watched-literal BCP, EVSIDS, saved phase, Luby restarts,
  learned-clause minimization, learned-clause reduction, arena-based clause storage, and streamed
  DRAT output
- MiniSat-style level-0 `simplify()` gating through `simplify_assigns` and
  `simplify_props_remaining`
- deletion of satisfied learned and original clauses at root
- trimming of root-false literals from surviving original clauses only
- garbage collection and branch-heap rebuild after simplification

What is not present yet:

- occurrence lists for original clauses
- MiniSat's backward subsumption / backward subsumption resolution queue
- asymmetric branching clause strengthening
- bounded variable elimination with resolvent growth and clause-size caps
- model extension for eliminated variables
- freeze / eliminated-variable bookkeeping
- `turn_off_elim` cleanup semantics from `SimpSolver::eliminate(true)`

The stale README copied from `09` has been replaced, but the implementation is still the `09`
baseline and should be treated that way until the preprocessing port lands.

## Design Doc

The implementation design and step-by-step port plan are in
[MINISAT_SIMP_PORT.md](/home/bojji/code/SAT-playground/solver/10-bve-preprocess/MINISAT_SIMP_PORT.md).

That document is the source of truth for:

- what MiniSat `SimpSolver` actually does
- the exact gap between the current Rust baseline and the target solver
- the data-structure changes needed for a faithful port
- the recommended implementation order, with test checkpoints

## Recommended Scope

The next implementation should aim to match the core MiniSat `simp` pipeline, not just "some BVE":

1. preserve the current `09` search behavior when simplification features are disabled
2. add MiniSat-style occurrence and touched-clause infrastructure for original clauses
3. add backward subsumption / strengthening at decision level `0`
4. add bounded variable elimination with faithful cost checks and resolvent generation
5. add elimination-stack model extension so SAT output still reconstructs eliminated assignments
6. add the one-shot `turn_off_elim` cleanup path used before search

## Validation Expectations For The Port

When implementation starts, follow the repo rules from `AGENTS.md`:

- add failing tests first when practical
- run `cargo test` in `solver/10-bve-preprocess`
- run `bash tools/smoke_test.sh solver/10-bve-preprocess`
- only treat the iteration as ready once the full smoke suite passes

## Historical Note

`git log -- solver/10-bve-preprocess` shows earlier local BVE experiments, but the current tree was
reset to the `09` baseline in commit `aa525b3` (`Reset solver 10 to solver 09 baseline`). The new
design doc therefore plans the MiniSat-faithful port from the actual current code, not from those
discarded intermediate attempts.
