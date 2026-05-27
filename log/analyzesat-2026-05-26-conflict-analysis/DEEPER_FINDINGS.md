# Deeper Findings - conflict-analysis traversal

## What the ablation ruled out

`SAT_CONFLICT_ANALYSIS_MODE=resolved` does not change learned-clause trajectory
on the profiling suite. This is stronger than "PAR-2 got worse": every paired
row has identical:

* conflicts
* decisions
* propagations
* restarts
* final learned-clause count

The conclusion is therefore independent of wall-clock noise. If this mode ever
helps, it is not because it changes search on the current reason-clause layout;
it can only affect local execution cost or protect against a broken clause-order
invariant.

## Code-level interpretation

Solver 11 represents reason clauses in the MiniSat-compatible shape where the
propagated literal is at slot 0. In default mode, conflict analysis skips that
slot when resolving a reason clause. In resolved mode, it scans slot 0 and then
uses `scratch_resolved[var]` to suppress the variable that was just resolved.

That produces identical logical work while adding state and branches:

* `scratch_resolved: Vec<u8>` storage per variable.
* write on every resolved variable.
* extra skip condition in `mark_clause_literals_for_analysis`.
* extra skip condition in `mark_binary_literals_for_analysis`.
* cleanup loop after analysis.

The full-matrix default pair showed a +7.0% geometric wall ratio for
`B_resolved` vs `A_baseline`, but the velev trace rerun reversed wall time while
keeping identical checkpoints. Treat this as "same search, unstable execution
timing" rather than as a robust microarchitectural claim. `perf stat` could not
be collected because the host has `perf_event_paranoid=4`.

## Why this is not the Kissat reason-side bump

Existing bead `SAT-playground-5b2.2.37` is still valid but should be read
precisely. Solver 11 already bumps variables that are reached during normal
1-UIP expansion; the unit test
`test_conflict_analysis_tracks_intermediate_reason_variables_for_activity`
demonstrates variables outside the final learned unit clause entering
`scratch_bumped_vars`.

Kissat's `analyze_reason_side_literals` is a later expansion over reason chains
of literals in the learned clause. It is controlled separately by:

* `bumpreasons`
* `bumpreasonslimit`
* `bumpreasonsrate`
* exponential delay when the expanded set exceeds the limit

That mechanism can change future branching activity without changing the current
learned clause. The resolved traversal does not do that.

## Recommended implementation path

1. Retire or hide `SAT_CONFLICT_ANALYSIS_MODE=resolved`.
   Remove the config/schema entry and the `use_resolved_conflict_analysis`
   hot-path branches, or keep the alternate traversal in a test-only helper that
   asserts equivalence on hand-crafted reason-clause order cases.

2. Add an equivalence regression if the alternate traversal is kept for safety.
   Build a small implication chain where reason clauses are known and assert the
   default and resolved traversals learn the same clause, backtrack level, and
   bumped-variable set.

3. Implement `SAT-playground-5b2.2.37` separately.
   After `analyze_conflict_to_scratch` and before activity decay, optionally walk
   reason chains of the learned-clause literals up to a multiple of the analyzed
   set size. Bump only newly discovered variables, track skip/limit stats, and
   disable the path in temporary accounting contexts.

4. Validate reason-side bump with a matrix that keeps `resolved` off.
   Use configs like default, metadata-only, bump-reasons-only, focused/stable,
   and focused/stable + bump-reasons. The success metric should be trajectory
   movement with explained conflict/decision changes, not wall-only speed noise.

## Residual risk

The suite did not include a case with broken reason-clause ordering. If there is
any code path that stores reason clauses without the propagated literal at slot
0, default mode would be wrong and resolved mode would be a safety net. I did
not find evidence of such a path in this pass, and the same-work result across
all SAT/UNSAT profiling rows is consistent with the invariant holding.
