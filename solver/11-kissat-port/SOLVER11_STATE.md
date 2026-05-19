# Solver 11 State

Task 0.1 establishes the architecture boundary map for `solver/11-kissat-port`.
This file is durable handoff state for future agents; keep it current when a
task moves code, introduces a capability, or changes ownership of an invariant.

## Stage A Modules

These modules are present before Phase 1 algorithmic work:

| Module | Owns now | Future growth |
| --- | --- | --- |
| `src/config.rs` | Existing environment parsing helpers. Defaults are unchanged from the fork. | Full `SolverConfig`, schema, dump, replay, profile/default selection in 0.3. |
| `src/stats.rs` | Existing `SolverStats` counters. | JSON stats, timers, trace output, feature maturity records. |
| `src/lit.rs` | Literal-to-index and raw arena word conversion helpers. | Typed literal/newtype helpers if later tasks need them. |
| `src/limits.rs` | Documented limit-check boundary. | Conflict, propagation, tick, wall-clock, RSS, learned-lit, binary, extension, and proof byte limits. |
| `src/output.rs` | SAT Competition assignment-line formatting. | Status/model/proof-path contract helpers as output handling is extracted. |
| `src/check.rs` | Debug generation-handle scaffold and tests. | Runtime invariant checks for typed clause, binary, reason, trail, and watch handles. |

## Stage B Map

Move these boundaries only when a task needs the seam. Do not move a whole
subsystem just to satisfy the map.

| Module | Planned ownership |
| --- | --- |
| `src/arena.rs` | Clause arena, `ClauseRef`, clause header/meta helpers, GC reference rewriting, debug generation checks. |
| `src/trail.rs` | Assignment values, decision levels, trail frames, reason references. |
| `src/watch.rs` | Long watcher mutation, binary implication adjacency, propagation conflict representation. |
| `src/proof.rs` | DRAT add/delete output, proof buffering, proof temp-file lifecycle, proof counters. |
| `src/model.rs` | Original-CNF model check, extension-stack replay, assignment repair. |
| `src/branch.rs` | Branch heap, VMTF, saved/target/best phase selection, rephase state. |
| `src/search.rs` | CDCL search loop, restart policy, reduce policy, search-mode glue. |
| `src/simp.rs` | Existing occurrence lists, backward subsumption, BVE, model extension until split by a later task. |
| `src/inprocess.rs` | Scheduler and pass orchestration only. |

## Capability-Based Mutation Rule

New pass modules must receive the narrowest capability object that can do the
job. A pass must not expose a public function that takes unrestricted
`&mut Solver` unless the task records an explicit exception below.

Rules:

- Existing `Solver::*` methods inherited from solver 10 are exempt until a task
  explicitly refactors them.
- Capability fields remain private.
- Capability objects must not be stored beyond a single call frame.
- No public capability method may return `&mut Solver`.
- The first capability seam is expected in 1.0a (`TemporaryAssumptionCtx`), but
  this is informative rather than normative.
- Later capability tasks must update this file and migrate the call sites they
  touch in the same patch.

Expected rollout:

| Task | Capability |
| --- | --- |
| 1.0a | `TemporaryAssumptionCtx` |
| 1.6 | Propagation-level capability if binary fast path needs it |
| 2.0 | `InprocessCtx` |
| 2.1 | `ProofCtx` |
| 2.1a | `ModelCtx` |
| 2.6+ | Occurrence-list capability if BVE scheduling needs it |

## unrestricted_mut_solver_exceptions

This table intentionally starts empty. Add a row only when a future task cannot
avoid a temporary public pass-module entry point taking unrestricted
`&mut Solver`.

| Module | Function | Reason | Expiry task |
| --- | --- | --- | --- |

## Extraction Rules

- Keep extraction patches behavior-preserving.
- Do not change feature defaults during extraction.
- Do not tune algorithms in extraction patches.
- Expose narrow methods rather than public fields.
- Document the invariants owned by every module.
- Add facade methods first, then move internals when the next task needs the seam.
- Every extraction patch must pass the solver-10-vs-solver-11 smoke-plus status comparison.
