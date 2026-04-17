# 04-vsids

First VSIDS iteration built on top of `03-bcp`.

This version keeps the watched-literal CDCL core from `03`, but replaces the static occurrence-order branch picker with a straightforward activity-based heuristic:

- each variable has a floating-point activity score
- variables from the current conflict clause and the learned clause get bumped
- the bump amount decays over time in EVSIDS style
- the next decision picks the highest-activity unassigned variable
- ties fall back to the occurrence-based order inherited from `03`

## Scope of This First Pass

This is a correctness-first VSIDS iteration.

Included:
- watched-literal BCP from `03`
- CDCL learning and non-chronological backtracking from `03`
- simple activity-based branching with conflict bumps and decay

Not included yet:
- heap-backed activity queue
- restarts
- phase saving
- clause deletion / database management

The goal of `04` is to introduce dynamic branching pressure without changing the rest of the search architecture.

## Design Notes

The solver still uses the same state model as `03`:

- `assignment[v]` stores `UNASSIGNED`, `TRUE`, or `FALSE`
- `decision_level[v]` stores the level where `v` was assigned
- `reason[v]` stores the clause index that implied `v`, or a sentinel for decisions
- `trail` stores literals in assignment order
- `trail_limits` stores the trail index where each decision level starts
- watched literals drive propagation incrementally over the trail

VSIDS adds:

- `activity[v]` for each variable
- `activity_inc` for the current bump amount
- `activity_decay` to age out older conflicts

On each conflict, the solver:

1. analyzes the conflict to produce a learned clause
2. bumps variables appearing in the triggering conflict clause
3. bumps variables appearing in the learned clause
4. increases the future bump amount by the decay factor

Decision selection then scans the unassigned variables and picks the highest-activity candidate.

## Validation

Completed checks for this first pass:

- `cargo test` — 14/14 unit tests passed
- `bash tools/smoke_test.sh solver/04-vsids` — 9/9 smoke tests passed
- All 5 UNSAT smoke-test proofs verified with `drat-trim`

The new unit tests added for `04` check that:

- branch selection prefers the highest-activity unassigned variable
- solving a conflict-driven UNSAT instance actually bumps variable activity
