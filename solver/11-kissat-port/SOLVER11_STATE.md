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
| `src/branch.rs` | Opt-in VMTF queue state and cursor updates; existing VSIDS heap logic still lives in `src/main.rs`. |
| `src/limits.rs` | Placeholder boundary for future limit checks. |
| `src/output.rs` | SAT Competition model line formatting, SolveStatus, model.txt writing, status.txt/result.json contract emission, and JSON escaping. |
| `src/check.rs` | Debug generation-handle scaffold and tests. |

Audited entry points:

| Entry point | File:line | Notes |
| --- | --- | --- |
| `Solver::new` | `src/main.rs:1644` | Builds branch ordering, root assignments, original clause arena, watchers, occurrence/BVE state, and default solver-10-compatible policy fields. |
| `Solver::solve_to_output` | `src/main.rs:6106` | Creates proof log according to `SAT_PROOF`, runs preprocessing/search, finalizes or discards proof output, and returns proof stats. |
| `Solver::solve_with_proof` | `src/main.rs:6134` | Runs root propagation, optional BVE/simplification, search loop, trace comments, SAT model snapshot, and proof finalization. |
| `Solver::propagate` | `src/main.rs:3384` | Watched-literal BCP over long clauses and units; returns conflicting clause arena offset, always updates propagation count, and updates watcher diagnostics only when `SAT_STATS_HOT=on`. |
| `Solver::analyze_conflict_to_scratch` | `src/main.rs:5879` | Learned clause construction, UIP backtrack target, minimization, and conflict activity updates. |
| `Solver::reduce_db` | `src/main.rs:5235` | Learned-clause reduction by activity with locked/binary preservation and DRAT deletion recording. |
| `Solver::eliminate` | `src/simp.rs:1083` | Preprocessing BVE/BSR driver; owns occurrence cleanup, resolvent generation, proof logging, and extension entries. |

Related implementation anchors:

| Anchor | File:line | Notes |
| --- | --- | --- |
| `ProofLog` | `src/main.rs:786` | DRAT buffering, temp/final proof path lifecycle, and proof stats snapshot; planned for `proof.rs`/`output.rs` split later. |
| `Solver` | `src/main.rs:1015` | Current monolithic state owner; future tasks introduce capability wrappers incrementally. |
| `Solver::attach_clause` | `src/main.rs:3160` | Watcher attachment and empty/unit handling. |
| `Solver::simplify_with_proof` | `src/main.rs:4543` | Top-level simplification and learned/original clause cleanup. |
| `Solver::garbage_collect` | `src/main.rs:5024` | Arena compaction and reference rewriting for current side structures. |
| `parse_cnf` | `src/main.rs:6496` | DIMACS parser used by `main`; returns parse errors so main can emit result.json with PARSE_ERROR. |
| `main` | `src/main.rs:6725` | CLI/run.sh entry point, config parsing/output before CNF parsing, solver construction, result.json/status/model contract emission, JSON_STATS/trace_full stderr emission, internal SAT model check, and SAT Competition stdout. |

Known unpromoted or incomplete feature families at this baseline:

- Full glue/LBD search policy: LBD metadata, focused/stable search, tick-based mode scheduling,
  and the Kissat/Glucose-style focused EMA restart policy are implemented but no longer promoted in
  the default and fast profiles after the clean 2026-05-24 solver 10 comparison showed a default
  PAR-2 regression. Reason-LBD update and LBD-tiered
  reduction remain explicit flags. Kissat-EMA restarts include an optional
  Glucose-style decision-level blocker controlled by `SAT_RESTART_BLOCK_MARGIN`, so high
  fast-vs-slow level EMA ratios can preserve a productive deep prefix instead of restarting
  immediately. The blocker is default-off (`0`) after profile testing showed the `1.4` margin
  regressed the current profile suite. Single-mode `SAT_RESTART=kissat-ema` is rejected because it
  lacks Kissat's focused-mode envelope and restart trail-reuse semantics. The slow LBD EMA window
  defaults to `4096`; the Kissat-style `100000` conflict window remains opt-in through
  `SAT_EMA_SLOW_WINDOW` because it over-restarts this partial focused/stable port and regresses
  profiling `case9` to a timeout.
  The Kissat-style partial-restart experiment remains compiled and env-facing
  `SAT_RESTART_REUSE_TRAIL*` requests are honored. Stable/focused trail reuse is now
  scoped to focused/stable mode, so the solver-10-compatible single-mode Luby path does not retain
  the `mp1` prefix that previously led to `UNKNOWN`.
  Learned clauses now start with the maximum
  `used_recently` value for every LBD tier; later reduce-DB passes age that counter down before
  high-LBD clauses become eviction candidates. Learned-reason LBD recomputation now walks arena
  clauses directly without a temporary literal-vector allocation, and LBD-tiered reduce-DB reuses a
  persistent delete-marker table plus in-place learned-clause-id compaction rather than allocating a
  dense delete vector every pass. The LBD-tiered reducer computes current focused and
  stable tier thresholds from recent glue-use histograms at each reduction pass, using 50% and 90%
  cumulative-use cutoffs with `2/6` as minimum floors, then reclassifies live learned clauses before
  collecting candidates. It also ages `used_recently` on every scanned kept learned clause rather
  than only on protected tier2/tier3 clauses. `SAT_LBD_UPDATE_REASONS=on` keeps reason-side LBD
  improvement scoped to conflict analysis; `SAT_LBD_UPDATE_PROP_REASONS=on` separately enables the
  propagation-time experiment that marks learned propagation reasons recently used in lbd-tiered mode
  and recomputes their LBD after the implied literal is enqueued. The propagation-time experiment
  remains isolated after profile testing regressed the current lbd-tiered feature mode. The reducer
  uses a conflict-count schedule rather than the legacy
  learned-clause-count pressure trigger: first reduce at `SAT_REDUCE_DB_INIT` or 1,000 conflicts,
  then reschedule at current conflicts plus `sqrt(reduce_db_calls) *
  SAT_REDUCE_DB_INTERVAL`; the hard learned-literal budget remains an emergency trigger. To avoid
  high-LBD focused/stable experiments repeatedly reducing every few conflicts, lbd-tiered mode also
  defaults `SAT_REDUCE_MIN_INTERVAL` to `100` conflicts and rejects explicit values below `50`.
- Chronological backtracking is present behind `SAT_CHRONO=on`. It is deliberately guarded: it
  only chooses `current - 1` when the learned clause remains asserting at that level and otherwise
  uses the normal assertion level.
- Saved/target/best phase policies are present behind `SAT_PHASE`. Per-mode focused/stable
  overrides are active behind `SAT_FOCUSED_PHASE` and `SAT_STABLE_PHASE` when
  `SAT_SEARCH_MODE=focused-stable` is enabled. In focused/stable search, target/best phase snapshots
  are captured only in stable mode, matching Kissat's update boundary; target phases are preserved
  across mode switches and restart cycles, then reset when a rephase event starts a new phase block.
  Single-mode target policies still reset target phase on restart because all-mode target
  persistence regressed the profiling suite.
- Focused/stable mode switching and reluctant restarts remain explicit experiments. Env-facing
  `SAT_SEARCH_MODE=focused-stable` and `SAT_MODE_USE_TICKS=on` requests execute the actual
  focused/stable path with LBD enabled and focused-only VMTF by default; `SAT_PROFILE=default` and
  `SAT_PROFILE=fast` keep the old single-mode search path with solver-10 preprocessing enabled.
  Stable-to-focused transitions reset the LBD EMA restart averages so focused restart calibration
  starts from focused-mode glue rather than inherited stable-mode glue. Focused-to-stable
  transitions reset the reluctant restart sequence and rebuild the VSIDS heap from current variable
  activities before stable-mode decisions resume. Stable reluctant restarts use Kissat's
  `reluctantint=1024` scale rather than raw Luby counts; this fixed the case9 focused/stable
  wall-clock UNKNOWN. Focused mode includes Kissat-style random decision sequences and focused
  phase cycling by mode-switch count. JSON/trace diagnostics now attribute search wall time,
  conflicts, learned-clause LBD, and decision level averages separately to focused and stable mode
  so focused/stable triage does not depend on combined averages.
- Kissat-style mode scheduling is compiled internally. Stable mode switches back
  to focused mode by propagation search ticks, focused-mode conflict intervals use Kissat
  `nlogpown(count, 4)` growth, and every mode switch resets all restart EMAs. Focused EMA restart
  windows also grow with the cumulative focused restart count as
  `50 + kissat_logn(focused_restarts) - 1`, and `focused_restarts` is reported in JSON/trace stats.
- VMTF branching is active for the Kissat-faithful focused/stable path. `SAT_SEARCH_MODE=focused-stable`
  defaults to focused-only VMTF unless `SAT_VMTF=off` is explicit. Focused mode uses the VMTF queue
  and move-to-front conflict bumps without updating VSIDS scores, and analyzed variables are moved
  by their existing queue-stamp order to match Kissat; stable mode keeps the VSIDS heap and score
  bumps. `SAT_VMTF=single` remains available as a default-off guarded experimental fallback for the
  single-mode search path, not as a promoted VMTF policy.
- Rephasing is active behind `SAT_REPHASE=on` for focused/stable experiments. It runs only on
  scheduled stable-mode restarts and cycles saved phase data through best, inverted, and original
  polarity sources.
- Binary implication propagation is present behind `SAT_BINARY_FAST=on`; binary clauses keep arena
  representation for proof/model/debug paths while propagation uses stable `BinaryClauseId` reasons.
  The default binary-fast-off propagation path is separately monomorphized so the solver-10-compatible
  profile does not pay a per-propagation binary implication branch.
- Formula classification now runs once after preprocessing and before the main search. It records
  size class, Kissat-style `small`/`bigbig`, binary-clause fraction, average clause size, and
  live-variable density in `SAT_STATS_JSON`; `SAT_TRACE_PREPROCESS=on` prints the same class summary.
  No adaptive feature routing is enabled yet.
- Lucky assignment is available as `SAT_LUCKY=on` and runs after preprocessing before main search.
  It uses the temporary-assumption capability to try all-true/all-false and forward/backward
  false/true propagation probes, then a bounded local repair fallback for small non-eliminated
  formulas. It records `lucky_attempts` / `lucky_solved` and captures a SAT model only after a full
  residual-formula satisfaction check. Default and fast profiles keep it off after the 2026-05-25
  rerun showed the unconditional pass solved only battleship while regressing the rest of the
  profiling suite.
- Clause minimization is binary-reason aware as of 1.11, so explicit `SAT_CLAUSE_MIN` settings are
  honored with `SAT_BINARY_FAST=on`. Binary-fast env runs now preserve the default recursive
  minimization unless `SAT_CLAUSE_MIN=off` is explicit, because implicit min-off produced
  `UNKNOWN` on a baseline-solved Sudoku row. With `SAT_OTFS=on`, clause minimization also runs
  bounded learned-clause subsumption after learned non-unit clauses are added. The pass now follows
  Kissat's eager-subsumption shape by checking only the last four remembered learned clauses, not all
  watcher lists for the learned literals; the watcher-wide variant was too aggressive and moved
  mp1/velev into baseline-solved timeouts. It checks candidates no more than four literals larger
  than the learned clause, deletes only when LBD metadata says the new clause is better, refuses
  live reason clauses, and logs DRAT deletions before tombstoning subsumed learned clauses. Metadata-
  free default runs therefore do not delete clauses under `SAT_OTFS=on`. The feature remains
  default-off after enabled profile regressions.
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
| `src/branch.rs` | `VmtfQueue` linked-list stamps and search cursor. | Later branch tasks can migrate the existing VSIDS heap, phase selection, and rephase state here. |
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
