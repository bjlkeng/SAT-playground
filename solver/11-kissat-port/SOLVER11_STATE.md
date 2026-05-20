# Solver 11 State

Task 0.1 establishes the architecture boundary map for `solver/11-kissat-port`.
This file is durable handoff state for future agents; keep it current when a
task moves code, introduces a capability, or changes ownership of an invariant.

## Baseline Source Map

Audited for task 0.2 after commit `1c692eb` and before Phase 1 feature work.
Line numbers are intentionally recorded so later agents can detect drift before
editing the solver.

Known source files:

| File | Current ownership |
| --- | --- |
| `src/main.rs` | Clause arena helpers, watcher attachment/propagation, conflict analysis, restarts, learned-clause reduction, top-level simplify/search loop, DRAT stream implementation, DIMACS parsing, process entry point. |
| `src/simp.rs` | Occurrence lists, backward subsumption/subsumption resolution, BVE, preprocessing model extension. |
| `src/config.rs` | SolverConfig, profile/axis defaults, strict SAT_* parsing, config dump/replay/hash, feature maturity records, and legacy env compatibility. |
| `src/stats.rs` | Solver counters, run/proof/input/formula snapshots, local JSON writer, streaming SHA-256 helper, JSON_STATS line emission, and SAT_TRACE_FULL summary formatting. |
| `src/lit.rs` | Raw `i32` literal word conversion and literal-index mapping. |
| `src/limits.rs` | Placeholder boundary for future limit checks. |
| `src/output.rs` | SAT Competition model line formatting, SolveStatus, model.txt writing, status.txt/result.json contract emission, and JSON escaping. |
| `src/check.rs` | Debug generation-handle scaffold and tests. |

Audited entry points:

| Entry point | File:line | Notes |
| --- | --- | --- |
| `Solver::new` | `src/main.rs:736` | Builds branch ordering, root assignments, original clause arena, watchers, occurrence/BVE state, and default solver-10-compatible policy fields. |
| `Solver::solve_to_output` | `src/main.rs:2557` | Creates proof log according to `SAT_PROOF`, runs preprocessing/search, finalizes or discards proof output, and returns proof stats. |
| `Solver::solve_with_proof` | `src/main.rs:2585` | Runs root propagation, optional BVE/simplification, search loop, trace comments, SAT model snapshot, and proof finalization. |
| `Solver::propagate` | `src/main.rs:1583` | Watched-literal BCP over long clauses and units; returns conflicting clause arena offset, always updates propagation count, and updates watcher diagnostics only when `SAT_STATS_HOT=on`. |
| `Solver::analyze_conflict_to_scratch` | `src/main.rs:2388` | Learned clause construction, UIP backtrack target, minimization, and conflict activity updates. |
| `Solver::reduce_db` | `src/main.rs:2256` | Learned-clause reduction by activity with locked/binary preservation and DRAT deletion recording. |
| `Solver::eliminate` | `src/simp.rs:1092` | Preprocessing BVE/BSR driver; owns occurrence cleanup, resolvent generation, proof logging, and extension entries. |

Related implementation anchors:

| Anchor | File:line | Notes |
| --- | --- | --- |
| `ProofLog` | `src/main.rs:100` | DRAT buffering, temp/final proof path lifecycle, and proof stats snapshot; planned for `proof.rs`/`output.rs` split later. |
| `Solver` | `src/main.rs:329` | Current monolithic state owner; future tasks introduce capability wrappers incrementally. |
| `Solver::attach_clause` | `src/main.rs:1496` | Watcher attachment and empty/unit handling. |
| `Solver::simplify_with_proof` | `src/main.rs:1918` | Top-level simplification and learned/original clause cleanup. |
| `Solver::garbage_collect` | `src/main.rs:2096` | Arena compaction and reference rewriting for current side structures. |
| `parse_cnf` | `src/main.rs:2901` | DIMACS parser used by `main`; returns parse errors so main can emit result.json with PARSE_ERROR. |
| `main` | `src/main.rs:3014` | CLI/run.sh entry point, config parsing/output before CNF parsing, solver construction, result.json/status/model contract emission, JSON_STATS/trace_full stderr emission, internal SAT model check, and SAT Competition stdout. |

Known missing or incomplete feature families at this baseline:

- Full glue/LBD search policy: LBD metadata, reason-LBD update, LBD-tiered reduction, and the
  opt-in Kissat/Glucose-style EMA restart policy are present behind explicit flags, but they are
  not promoted as default-profile behavior yet.
- Chronological backtracking.
- Saved/target/best phase policies are present behind `SAT_PHASE`, but they are not promoted as
  default-profile behavior yet.
- Focused/stable mode switching.
- VMTF queue.
- Rephasing.
- Vivification.
- Failed literal probing.
- Hyper-binary resolution.
- Equivalent literal substitution.
- Transitive reduction.
- Gate extraction.
- Walking local search.
- DRAT deletion coverage for future binary-clause and formula-edit transaction paths beyond the existing learned/original simplification deletions.

## Stage A Modules

These modules are present before Phase 1 algorithmic work:

| Module | Owns now | Future growth |
| --- | --- | --- |
| `src/config.rs` | SolverConfig, schema-backed env parsing, replay/dump/hash, feature maturity records, profile/axis selection, and fail-fast validation. | Later tasks add implementation support for currently parked feature flags and lift validator rejections when behavior lands. |
| `src/stats.rs` | `SolverStats`, proof/input/formula/timing snapshot types, streaming SHA-256, local JSON writer, JSON_STATS line builder, and SAT_TRACE_FULL line builder. | Later tasks populate currently-zero future counters as features land. |
| `src/lit.rs` | Literal-to-index and raw arena word conversion helpers. | Typed literal/newtype helpers if later tasks need them. |
| `src/limits.rs` | Documented limit-check boundary. | Conflict, propagation, tick, wall-clock, RSS, learned-lit, binary, extension, and proof byte limits. |
| `src/output.rs` | SAT Competition assignment-line formatting and minimal 0.3a status/model/result contract helpers. | Fuller OutputContract checks in 0.8 may add proof/model finalization validation without changing status strings. |
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
