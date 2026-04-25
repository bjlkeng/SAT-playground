# 07-clause-storage-minimization

This iteration takes the `06-clause-storage` watcher/layout rewrite and adds runtime conflict-clause
minimization.

## Current State

`07` includes:

- the full `06` clause storage and blocker-watcher rewrite
- runtime clause minimization modes: `none`, `basic`, and `deep`
- proof-sound minimization restricted to original-clause reason chains
- a regression test that keeps literals whose support escapes the learned source set
- runtime default `ccmin_mode = deep`
- `SAT_CCMIN_MODE=none|basic|deep` override for benchmarking and debugging

## What Changed

This pass extends `06` with MiniSat-style conflict-clause shrinking:

- added basic and recursive redundancy checks during conflict analysis
- made the runtime path proof-sound by refusing to trust learned-clause reasons during
  minimization
- kept copied proof logging so DRAT output stays stable under in-place clause mutation
- removed the unused clause-abstraction metadata carried over from the storage refactor

The result is a solver that keeps the faster clause/watcher hot path from `06` and gets a further
search reduction from safe clause minimization.

## Reimplementation Notes: Deep Simplification Through Learned Clauses

If you want to take a fresh `07` solver and make `deep` conflict-clause minimization recurse
through learned reasons as well as original reasons, the mechanics are straightforward, but there
are several easy ways to silently break the solver.

### High-level change

The existing `07` implementation keeps DRAT proofs sound by refusing to trust learned-clause
reasons during minimization. To make it more aggressive:

- remove the learned-reason guard in the `basic` redundancy check
- remove the learned-reason guard in the recursive `deep` redundancy check
- remove the top-level `if reason[var] is learned => keep` shortcut in
  `minimize_learned_clause()`

That part is small. The important work is the invariants below.

### Gotcha 1: Skip the reason-clause head by position, not by literal equality

This is the bug most likely to blow the solver up.

In the learned clause, literals are stored in their **falsified** form under the current
assignment. For example, if variable `x` is currently assigned `true`, the learned clause stores
`-x`.

But the reason clause for that assignment stores the **assigned** literal at slot `0`. For the same
example, the reason clause head is `x`, not `-x`.

That means this is wrong:

- iterate every literal in the reason clause
- skip only `if parent == lit`

because `lit` is `-x` while the reason head is `x`, so the minimizer recurses back into the same
variable's reason chain and the DFS stack can grow until OOM.

The correct rule is:

- in both `basic` and `deep` minimization, ignore `reason_clause[0]`
- only inspect `reason_clause[1..]`

MiniSat's implementation does exactly this by starting its scan at index `1`.

### Gotcha 2: Ignore decision-level 0 parents

When checking whether a literal is redundant, literals whose variables are assigned at decision
level `0` should not block removal.

So in both redundancy walkers:

- if `decision_level[parent_var] == 0`, continue immediately

Treating level-0 parents as ordinary recursive dependencies makes minimization weaker and can also
mask the real structure of the reason DAG when debugging.

### Gotcha 3: Backtracking must keep the target level, not drop it

This is the other bug that produced invalid learned clauses.

If `trail_limits[level - 1]` stores the start index of decision level `level`, then:

- backtracking to level `0` keeps `root_trail_len`
- backtracking to level `L > 0` must keep assignments at levels `<= L`
- so the new trail length is the start of level `L + 1`

With the current `trail_limits` representation, that means:

- `new_trail_len = trail_limits[target_level]` when `target_level < current_level`
- not `trail_limits[target_level - 1]`

Using `target_level - 1` drops the assignments at the target level as well, which makes freshly
learned clauses non-asserting immediately after conflict analysis.

### Gotcha 4: Learned clauses must still be asserting after minimization

After conflict analysis:

1. compute the simplified learned clause
2. compute the backtrack level as the maximum decision level among literals at positions `1..`
3. backtrack to that level
4. the literal at position `0` must be enqueueable
5. every other literal in the learned clause must evaluate to `FALSE`

If any non-head literal is `UNASSIGNED` after backtracking, either:

- the minimizer removed the wrong literals, or
- the backtrack logic is wrong

This is worth asserting in a debug build while bringing the feature up.

### Gotcha 5: The reason graph must stay acyclic along the current trail

For a currently assigned variable, following `reason[var]` recursively through the non-head
parents should always move to strictly earlier assignments on the trail.

Practical debugging check:

- keep an explicit DFS stack in `lit_redundant()`
- if the stack ever grows past the number of variables, panic and dump the current path

That should be impossible in a correct implementation. If it happens, you almost certainly have one
of:

- the head-skipping bug above
- a non-asserting learned clause being used as a reason
- stale reason pointers after backtracking

### Gotcha 6: Valid DRAT proofs are a separate problem

Solver-side learned-clause minimization through learned reasons is not automatically proof-sound
under `07`'s current proof logging.

Important distinction:

- for the solver: learned-reason minimization is fine if the clause is actually implied
- for the DRAT proof: you cannot simply log the raw learned clause and then also log the shorter
  clause unless you have a valid derivation for the strengthening step

So if you reimplement this on top of a fresh `07` and still want valid UNSAT proofs, you need one
of these approaches:

- keep the proof-visible learned clause conservative while using a more aggressive internal clause
  only inside the solver
- emit a proof chain for the strengthening steps
- or continue to restrict proof-generating minimization to original-clause reason chains only

Do not assume that a shorter solver clause is automatically a valid DRAT addition.

### Recommended implementation checklist

1. Start from the existing `07` `basic_lit_redundant()`, `lit_redundant()`, and
   `minimize_learned_clause()` code.
2. Remove the learned-reason guards.
3. Change both redundancy walkers to skip the reason head by position (`[1..]`), not by comparing
   against the learned literal.
4. Ignore level-0 parents in both walkers.
5. Fix `backtrack()` so it preserves the target level.
6. Add a regression test that uses a learned-clause literal like `-x` with a reason clause headed
   by `x`, and verify that minimization terminates and removes the literal when appropriate.
7. Add a regression test that backtracking to a nonzero level keeps assignments at that level.
8. In a debug build, assert that every learned clause is still asserting after backtrack.
9. Only then benchmark.

### Minimal regression tests worth keeping

- a deep-minimization test where the removable literal's reason is itself a learned clause
- a test where the removable literal is stored as `-x` but the reason head is `x`
- a test showing level-0 parents do not block removal
- a nonzero-backtrack test proving target-level assignments survive
- smoke tests with proof checking, because proof correctness is where this optimization becomes
  tricky

## Validation

- `cargo test` — `24/24`
- `bash tools/smoke_test.sh solver/07-clause-storage-minimization` — `9/9`

## Profiling Benchmark Result

Profiling run on 2026-04-21:

- Command: `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/07-clause-storage-minimization`
- Result: `PAR-2 20.736`
- Solved: `6/6`

| Instance | Type | Result | Time |
|----------|------|--------|------|
| feistel_b64_k32_r17 | crypto | SAT | 1.182s |
| feistel_b64_k49_r15 | crypto | SAT | 3.697s |
| feistel_b64_k57_r14 | crypto | SAT | 0.303s |
| random_v229_s2 | 3-SAT | UNSAT | 4.479s |
| random_v240_s3 | 3-SAT | UNSAT | 4.267s |
| random_v241_s4 | 3-SAT | UNSAT | 6.808s |

Compared with `06`, this iteration cuts the profiling PAR-2 from `58.282` to `20.736`.
