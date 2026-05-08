# MiniSat `simp` Port Design For `solver/10-bve-preprocess`

## Goal

Implement MiniSat's `SimpSolver` preprocessing pipeline in `solver/10-bve-preprocess` faithfully
on top of the current `09-root-simp-opts` Rust baseline.

In this context, "faithfully" means:

- keep the current CDCL search core as the post-preprocessing engine
- port MiniSat `simp` preprocessing semantics, data flow, and cleanup behavior, not just the idea
  of BVE
- preserve SAT model correctness by reconstructing eliminated variables
- preserve UNSAT behavior and proof logging expectations already present in this repo

Primary MiniSat references:

- `benchmarks/reference-solvers/minisat/minisat/simp/SimpSolver.h`
- `benchmarks/reference-solvers/minisat/minisat/simp/SimpSolver.cc`
- `benchmarks/reference-solvers/minisat/minisat/core/Solver.cc`
- `benchmarks/reference-solvers/minisat/minisat/simp/Main.cc`

## What MiniSat `SimpSolver` Actually Adds

MiniSat `simp` is not a single BVE routine. It is a preprocessing subsystem layered on top of the
core solver.

### Entry points and control flow

- `solve_()` freezes assumptions temporarily, runs `eliminate()`, then falls back to the core CDCL
  solver, and finally extends the model if SAT.
- `Main.cc` calls `S.eliminate(true)` once after parsing. That means preprocessing normally runs as
  a separate phase and then frees its own heavy bookkeeping before search.
- `eliminate()` itself starts by calling the core `simplify()` pass, then loops while there is
  touched work, pending root assignments for subsumption, or eliminable variables left in the heap.

### Simplification state that exists only for preprocessing

MiniSat adds these persistent structures beyond the core solver:

- `occurs[var]`: occurrence lists of original clauses containing `var`
- `n_occ[lit]`: literal occurrence counts used by the elimination-cost heap
- `elim_heap`: heap ordered by `n_occ[x] * n_occ[~x]`
- `subsumption_queue`: queue of clauses that need backward-subsumption work
- `touched[var]` and `n_touched`: dirty-variable tracking
- `frozen[var]` and `frozen_vars`: variables that must not be eliminated
- `eliminated[var]`: variables removed from the clause database
- `elimclauses`: packed model-extension log for reconstructing eliminated assignments
- `bwdsub_tmpunit`: dummy clause reused to run backward subsumption against new root assignments
- mode flags such as `use_simplification`, `remove_satisfied`, `use_asymm`, `use_rcheck`,
  `use_elim`

### Transformations MiniSat performs

At decision level `0`, `SimpSolver` can:

- check whether a candidate clause is already implied (`use_rcheck`)
- run backward subsumption
- run backward subsumption resolution / clause strengthening
- run asymmetric branching-based clause strengthening (`use_asymm`)
- run bounded variable elimination (`use_elim`)
- remove satisfied clauses and trim root-false literals through the core `simplify()` path
- reconstruct the SAT model through `extendModel()`

### Key behavioral details worth preserving

- assumptions are frozen before preprocessing
- elimination only applies to unassigned, unfrozen, non-eliminated variables
- elimination is bounded by both clause-growth and resolvent-size checks
- new clauses inserted during preprocessing immediately feed back into occurrence lists, touched
  variables, and the subsumption queue
- preprocessing can be turned off permanently after a one-shot run, at which point MiniSat drops
  occurrence/subsumption state and disables extra clause allocator fields
- model extension is required for SAT because eliminated variables are no longer in the live clause
  database

## Current Solver 10 Baseline

The current Rust solver already has a useful subset of MiniSat core behavior:

- arena-based clause storage with a learned-clause extra word
- watched-literal propagation with blocker fast path
- root-level simplify gating through `simplify_assigns` and `simplify_props_remaining`
- learned-clause activity, EVSIDS, restarts, reduction, and GC
- packed clause references that can survive relocation

Relevant current code points:

- solver state: `src/main.rs:184-271`
- root simplify helpers: `src/main.rs:650-743`
- root `simplify()`: `src/main.rs:1262-1285`
- learned-clause insertion/deletion paths: `src/main.rs:1292-1397`
- main solve loop root simplify hook: `src/main.rs:1741-1824`

This is still a `09`-style root simplifier, not a MiniSat `simp` solver:

- there is no original-clause occurrence index
- there is no touched-clause queue
- there is no elimination-cost heap
- there is no frozen/eliminated-variable tracking
- all `add_clause*` helpers currently mean "learned clause", not "general solver clause insertion"
- there is no SAT model extension for eliminated variables
- there is no one-shot `eliminate(true)` preprocessing phase before search

## Gap Analysis: MiniSat vs Current Rust Solver

### 1. Clause ownership and indexing

MiniSat `SimpSolver` preprocessing works over original clauses. The current Rust solver has:

- `original_clause_ids`
- `learned_clause_ids`
- deletion and trimming logic that already treats originals and learned clauses differently

What is missing is a second indexing layer over original clauses:

- per-variable occurrence lists
- literal counts for elimination cost
- touched-variable tracking
- a queue of original clauses awaiting backward-subsumption work

### 2. General clause insertion API

MiniSat's `addClause_()` is the preprocessing entry point for original clauses. It:

- optionally skips the clause if already implied
- inserts it through the core solver
- pushes the new clause into the subsumption queue
- updates `occurs`, `n_occ`, `touched`, and the elimination heap

Current solver 10 has only learned-clause insertion (`add_clause_from_slice()`) and initial parse
construction in `new()`. A faithful port needs a split between:

- original-clause insertion used during parse and preprocessing
- learned-clause insertion used during CDCL conflict analysis

### 3. Clause deletion semantics

MiniSat uses one `removeClause()` path that:

- updates occurrence counts
- smudges occurrence lists
- then delegates to the core clause detach/remove path

Current solver 10 has:

- root-simplify deletion for originals and learneds
- learned-only deletion for database reduction

What is missing is a preprocessing-aware original-clause deletion path that keeps occurrence
metadata consistent.

### 4. Root simplification semantics

MiniSat core `simplify()`:

- propagates
- gates on `simpDB_assigns` and `simpDB_props`
- removes satisfied learned clauses
- conditionally removes/trims original clauses if `remove_satisfied`
- rebuilds the order heap
- refreshes the simplify budget

Current solver 10 is already close here, but it differs in important ways:

- it never has a `remove_satisfied = false` mode
- it does not support released/free variables cleanup
- it trims only originals and intentionally never trims unsatisfied learned clauses
- it has no interaction with preprocessing data structures

For the full `simp` port, the root simplify path should stay intact as the foundation, but its
deletion/trim operations must feed occurrence bookkeeping and variable-release semantics where
relevant.

### 5. Backward subsumption pipeline

MiniSat's backward subsumption loop matters because BVE is not run against a stale database. It:

- drains `subsumption_queue`
- also processes new root assignments via `bwdsub_tmpunit`
- picks the least-populated variable in the candidate clause to limit scans
- can either delete a subsumed clause or strengthen it by removing one literal
- recursively feeds new work back into the queue

Current solver 10 has none of this machinery. Without it, a "BVE port" would not be faithful.

### 6. Asymmetric branching

MiniSat optionally strengthens clauses with asymmetric branching before elimination:

- temporarily assigns negations of all non-`v` literals in a clause
- if propagation conflicts, the remaining `v`-literal is removable
- then it strengthens the clause and reruns backward subsumption

This is separate from BVE. It must be modeled as an optional preprocessing pass over a variable's
occurrence list.

### 7. Variable elimination

MiniSat's elimination loop has three distinct parts:

1. split the occurrence list into positive and negative clause sets
2. estimate whether the cross product is allowed using `grow` and `clause_lim`
3. if allowed:
   - mark the variable eliminated
   - record extension clauses in `elimclauses`
   - delete all old clauses containing the variable
   - add every non-tautological resolvent
   - clear the occurrence list
   - rerun backward subsumption

Current solver 10 has none of this state or flow.

### 8. Model extension

MiniSat stores enough data in `elimclauses` to reconstruct assignments for eliminated variables
after SAT. That is not optional if the competition interface requires a satisfying assignment over
the original variables.

Current solver 10 prints assignments directly from the live search state. If variables are
eliminated without extension, SAT output will be incomplete or wrong.

### 9. One-shot preprocessing cleanup

MiniSat `eliminate(true)` is important operationally:

- frees occurrence lists and queues
- disables simplification-only allocator fields
- restores normal `remove_satisfied`
- records `max_simp_var`
- rebuilds the order heap
- forces a full GC

That cleanup keeps search lean after preprocessing. A faithful port should keep the same split:

- heavy structures during preprocessing
- compact search-only state after preprocessing is done

## Proposed Rust Design

### A. Keep the current CDCL core as the base

Do not rewrite the whole solver around a different architecture. Reuse:

- clause arena
- watcher representation
- conflict analysis
- restarts / reduction
- proof logging

The port should add a preprocessing subsystem around the existing arena and clause IDs.

### B. Add explicit clause kinds and insertion paths

Introduce separate APIs:

- `add_original_clause_from_slice()`
- `add_learned_clause_from_slice()` or keep the current name for learned insertion
- `remove_clause_preprocess()` for original-clause-aware deletion

Reason:

- preprocessing and CDCL currently share too much insertion/deletion surface
- MiniSat updates occurrence metadata only for problem clauses and preprocessing-generated
  resolvents, not for learned clauses

### C. Add preprocessing-only state to `Solver`

Add fields corresponding to MiniSat's preprocessing subsystem:

- `use_simplification: bool`
- `remove_satisfied_originals: bool`
- `frozen: Vec<bool>`
- `frozen_vars: Vec<usize>`
- `eliminated: Vec<bool>`
- `occurs: Vec<Vec<usize>>`
- `occurs_dirty: Vec<bool>` or a smudged/clean bitset
- `n_occ: Vec<usize>` indexed by literal
- `touched: Vec<bool>`
- `n_touched: usize`
- `subsumption_queue: VecDeque<usize>`
- `queued_for_subsumption: Vec<bool>` to avoid gross duplication
- `elim_heap: binary min-heap by n_occ[pos] * n_occ[neg]`
- `elim_heap_pos: Vec<usize>`
- `bwdsub_assigns: usize`
- `bwdsub_tmpunit`: represent as a tiny scratch clause rather than a real arena clause
- `elim_clauses: Vec<u32>` for model extension
- preprocessing options:
  - `use_asymm`
  - `use_rcheck`
  - `use_elim`
  - `grow`
  - `clause_lim`
  - `subsumption_lim`
  - `simp_garbage_frac`

Important design choice:

- keep occurrence lists only for original/preprocessed clauses
- do not index learned clauses in `occurs`

That matches MiniSat and avoids polluting elimination costs with learned clauses.

### D. Separate "deleted in arena" from "present in occurrence list"

Occurrence lists should be lazy-cleaned, as in MiniSat's `OccLists`:

- deletion should mark affected variables dirty instead of eagerly removing the clause from every
  occurrence vector
- consumers should call a `clean_occurs(var)` helper before iterating a variable's occurrence list

Reason:

- eager vector removal inside every clause delete/strengthen path will be expensive
- MiniSat explicitly uses smudged occurrence lists for this reason

### E. Reuse the current simplify budget counters

The existing `simplify_assigns` / `simplify_props_remaining` fields already mirror MiniSat core
`simpDB_assigns` / `simpDB_props` closely enough. Keep them.

What changes:

- `simplify()` must use the preprocessing-aware delete/trim helpers when simplification is still
  enabled
- after `eliminate(true)`, simplify should revert to the cheaper post-preprocessing mode

### F. Add a model-extension stack exactly once

Implement MiniSat-style extension logging:

- `mk_elim_clause(unit)` pushes literal and trailing length `1`
- `mk_elim_clause(var, clause)` pushes the clause with the eliminated-variable literal moved to the
  front, then stores the clause length
- `extend_model()` walks this vector backward after SAT and assigns eliminated variables whose
  defining clause is otherwise falsified

This must integrate with the current SAT output path before any elimination is enabled.

### G. Make preprocessing a distinct top-level phase

Mirror MiniSat's operational flow:

1. parse original clauses
2. enqueue/propagate root units
3. call `eliminate(true)` once before CDCL search
4. if preprocessing finds UNSAT, stop immediately
5. otherwise run the normal CDCL search on the simplified formula
6. if SAT, run `extend_model()` before printing the assignment

This is better than trying to interleave full BVE inside the current solve loop.

## Core Algorithms To Port

### 1. `implied(clause)`

Purpose:

- optional expensive redundancy check before inserting a clause

Rust design:

- require decision level `0`
- enqueue negations of all non-false literals in scratch mode
- if any literal is already true, clause is implied immediately
- run propagation
- backtrack to root without mutating the permanent root assignment

Implementation note:

- the current solver lacks a temporary "push root assumptions and cancel" helper
- add one reusable helper for temporary root-probe propagation

### 2. `gather_touched_clauses()`

Purpose:

- move clauses from touched variables into the subsumption queue

Rust design:

- for each touched variable, clean its occurrence list
- enqueue each live clause once using a `queued_for_subsumption` bit/vector
- clear the touched bit and reset `n_touched`

### 3. `backward_subsumption_check()`

Purpose:

- delete subsumed clauses
- perform backward subsumption resolution by strengthening one literal away

Rust design:

- while queue non-empty or `bwdsub_assigns < root_trail_len`
- when queue empty but new root assignment exists, synthesize a size-1 scratch clause
- choose the smallest occurrence variable from the driver clause
- scan that variable's occurrence list
- classify candidate relation:
  - `subsumed`
  - `strengthen by removing lit`
  - `no action`
- after strengthening/deleting, keep occurrence metadata and queue state consistent

Key implementation constraint:

- the current solver has no clause abstraction bitsets like MiniSat's `Clause::subsumes()` path
- first faithful version should still add a clause-abstraction cache or temporary bitset check,
  otherwise subsumption scans will be too slow and too unlike MiniSat's behavior

### 4. `strengthen_clause()`

Purpose:

- remove one literal from a live original clause at root

Rust design:

- if binary, delete clause and rewrite it as a unit in place of the removed literal
- otherwise:
  - detach watchers
  - remove the literal
  - reattach watchers
  - update occurrence counts
  - update touched bits / queue state
- if clause becomes unit, enqueue it and propagate immediately

Important mismatch to resolve:

- current clause trimming assumes watched literals remain in positions `0` and `1`
- general strengthening can delete any literal, including watched ones
- add a generic "remove literal from clause and restore watched invariant" helper rather than
  reusing `trim_root_false_literals()`

### 5. `asymm()` / `asymm_var()`

Purpose:

- strengthen clauses using asymmetric branching before elimination

Rust design:

- iterate the occurrence list of a variable
- for each clause, temporarily assign negations of all other non-false literals
- if propagation conflicts, strengthen away the variable's literal in that clause
- after finishing the variable, rerun backward subsumption

### 6. `merge()`

Purpose:

- compute resolvent size quickly for the elimination bound
- compute the actual resolvent when elimination proceeds

Rust design:

- implement both forms:
  - `merge_size_only(pos_clause, neg_clause, var) -> tautological? + size`
  - `merge_into_scratch(...) -> tautological?`
- keep the same duplicate/tautology semantics as MiniSat:
  - duplicate literals collapse
  - complementary literals make the resolvent tautological and therefore skipped

### 7. `eliminate_var()`

Purpose:

- bounded variable elimination

Rust design:

1. clean the variable's occurrence list
2. split into `pos` and `neg`
3. estimate whether elimination is allowed:
   - count only non-tautological resolvents
   - reject if `count > occurs(v).len() + grow`
   - reject if any resolvent exceeds `clause_lim` when set
4. if allowed:
   - mark variable eliminated
   - disable it as a decision variable / branch candidate
   - push extension clauses into `elim_clauses`
   - delete every clause containing the variable
   - add all non-tautological resolvents as original clauses
   - clear `occurs[v]`
   - optionally clear watcher vectors for `v` if empty
   - run backward subsumption again

### 8. `extend_model()`

Purpose:

- restore assignments for eliminated variables after SAT

Rust design:

- walk `elim_clauses` backward by stored clause lengths
- if every non-head literal in an extension clause is false in the model, assign the head literal
  true

Output integration:

- call before `print_assignment()`
- ensure eliminated variables appear with concrete truth values in stdout

### 9. `eliminate(turn_off_elim)`

Purpose:

- entire preprocessing phase

Rust design:

1. call root `simplify()`
2. if simplification is already disabled, return
3. loop while any of:
   - touched work remains
   - new root assignments remain for backward subsumption
   - elimination heap non-empty
4. inside the loop:
   - gather touched clauses
   - run backward subsumption if needed
   - pop candidate variables from the elimination heap
   - skip eliminated/assigned variables
   - optionally run asymmetric branching
   - optionally run variable elimination
   - run simplification GC threshold checks
5. cleanup:
   - if `turn_off_elim`, drop preprocessing-only structures, disable extra clause metadata if
     possible, rebuild branch heap, and force full GC
   - else keep preprocessing state alive and only do cheap GC

## Integration Details For This Repo

### Proof logging

The current solver writes DRAT proof additions for learned clauses only. MiniSat `simp` itself does
not emit DRAT in this old codebase.

For this repo, decide explicitly before implementation whether preprocessing additions/deletions
must also be logged for proof correctness. A faithful semantic port of MiniSat simplification is
not automatically a faithful DRAT-producing port.

Recommended approach:

- treat proof logging as a separate acceptance gate
- first get the simplification/search semantics correct behind tests
- then add DRAT support for preprocessing transformations if required by the repo's checker flow

This is the single biggest place where "faithful to MiniSat simp" and "faithful to repo proof
requirements" may diverge.

### Watchers and root units

Current solver 10 stores unit clauses in the watcher structure and tracks original root units in
`root_unit_clauses`. That is compatible with preprocessing, but strengthening/elimination must keep
this list valid across:

- clause deletion
- garbage collection
- unit creation by strengthening

### Branching heap

MiniSat disables eliminated variables as decision variables. In the current Rust solver that should
mean:

- mark them non-branchable permanently
- ensure they are removed or skipped in the branch heap
- do not reinsert them on backtrack

This likely requires a new `decision_var: Vec<bool>` or equivalent flag rather than relying only on
assignment state.

### Assumptions

MiniSat freezes assumptions before elimination. The current CLI solver does not expose assumptions
yet, but the design should still leave room for them:

- add `freeze_var()` / `thaw()` helpers now
- even if assumptions are not used yet, this avoids painting the solver into a corner

## Recommended Implementation Order

The order below is chosen to preserve correctness and keep regressions local.

### Phase 0: metadata and baseline cleanup

1. Rename `Cargo.toml` package metadata from `09` to `10`.
2. Keep current solver behavior unchanged.
3. Add tests that pin the current root simplify behavior and current SAT output shape.

Exit criteria:

- `cargo test`
- `bash tools/smoke_test.sh solver/10-bve-preprocess`

### Phase 1: preprocessing state scaffolding

1. Add preprocessing configuration fields and vectors to `Solver`.
2. Add a decision-variable flag separate from assignment state.
3. Split original-clause insertion from learned-clause insertion.
4. Build occurrence lists and literal counts during initial parse.
5. Add lazy-clean helpers for occurrence lists and queue-dedup state.

Tests first:

- occurrence lists built correctly from parsed clauses
- literal counts update on insertion
- deleted original clauses disappear after `clean_occurs(var)`

### Phase 2: preprocessing-aware deletion and strengthening primitives

1. Implement original-clause-aware `remove_clause_preprocess()`.
2. Implement generic clause literal removal / strengthening with watcher repair.
3. Update root `simplify()` to route through preprocessing-aware deletion when
   `use_simplification` is enabled.

Tests first:

- deleting an original clause updates occurrence counts and touched state
- strengthening a non-binary clause preserves watcher correctness
- strengthening a binary clause yields a unit and propagates it

### Phase 3: touched-clause and backward-subsumption pipeline

1. Implement `gather_touched_clauses()`.
2. Implement scratch-clause handling for new root assignments.
3. Implement backward subsumption and backward subsumption resolution.
4. Optionally add clause abstraction/cache if needed for performance parity.

Tests first:

- clause A subsumes clause B and B is removed
- clause A backward-subsumes clause B up to one literal and B is strengthened
- new root assignments feed the subsumption queue through the scratch unit path

### Phase 4: implied-clause checking and optional asymmetric branching

1. Add temporary root-probe helper for implication checks.
2. Implement `implied()`.
3. Implement `asymm()` and `asymm_var()`.

Tests first:

- implied clause is skipped when `use_rcheck` is enabled
- asymmetric branching removes the target literal on a crafted instance

### Phase 5: elimination heap and bounded variable elimination

1. Implement elimination-cost heap keyed by `n_occ[pos] * n_occ[neg]`.
2. Implement `merge_size_only()` and `merge_into_scratch()`.
3. Implement `eliminate_var()` with:
   - grow cap
   - clause-length cap
   - extension stack writes
   - resolvent insertion
   - occurrence cleanup
4. Re-run backward subsumption after each elimination.

Tests first:

- variable not eliminated when resolvent count exceeds `occurs(v) + grow`
- variable not eliminated when a resolvent exceeds `clause_lim`
- successful elimination deletes old clauses and inserts expected resolvents
- eliminated variable never appears in the branch heap

### Phase 6: model extension

1. Implement `mk_elim_clause_*` helpers.
2. Implement `extend_model()`.
3. Call it on SAT after preprocessing.

Tests first:

- SAT model assigns eliminated variables consistently with the extension stack
- printed assignment still satisfies the original CNF

### Phase 7: one-shot preprocessing entry point

1. Add `eliminate(turn_off_elim)` around the current search entry point.
2. Run `eliminate(true)` before the CDCL loop, matching MiniSat's operational model.
3. Implement cleanup:
   - drop occurrence/subsumption structures
   - disable preprocessing-only paths
   - rebuild branch heap
   - force full GC

Tests first:

- preprocessing-only structures are empty/disabled after `eliminate(true)`
- search still works after preprocessing cleanup
- a formula solved during preprocessing returns UNSAT before entering CDCL

### Phase 8: proof and benchmark hardening

1. Decide and implement proof logging policy for preprocessing transforms.
2. Re-run unit tests, smoke tests, and targeted benchmarks.
3. Compare against MiniSat `-pre`/`-no-pre` behavior on a small regression set.

Tests and checks:

- `cargo test`
- `bash tools/smoke_test.sh solver/10-bve-preprocess`
- proof checker run on UNSAT smoke tests
- benchmark spot-checks against MiniSat on targeted instances

## Minimum Test Matrix To Add

Add targeted unit tests for:

- occurrence-list construction and lazy cleanup
- touched-clause queue dedup
- backward subsumption delete case
- backward subsumption strengthen case
- asymmetric branching strengthen case
- elimination rejection by `grow`
- elimination rejection by `clause_lim`
- successful elimination with expected resolvents
- eliminated variable removed from branching
- model extension after SAT
- preprocessing cleanup after `eliminate(true)`

Keep the existing smoke suite as the final guardrail.

## Main Risks

### 1. Proof correctness risk

Preprocessing transformations change the formula before CDCL. If DRAT logging is not updated,
UNSAT proofs may stop checking even if search is logically correct.

### 2. Watcher corruption risk

General clause strengthening is more invasive than current root trimming. Any bug here will produce
silent propagation errors.

### 3. Model-extension risk

Elimination without extension will produce incomplete SAT assignments.

### 4. Performance risk

Naive eager occurrence maintenance or naive subsumption scans can erase any benefit from BVE.

### 5. Scope creep risk

Trying to implement all of BVE, subsumption, asymm, proof logging, and metadata cleanup in one
patch is likely to fail. The phases above should be kept separate.

## Recommended First Coding Slice

If implementation starts immediately, the highest-value first patch is:

1. fix solver 10 metadata drift
2. add preprocessing state fields
3. split original vs learned clause insertion
4. build occurrence lists and literal counts at parse time
5. add unit tests for occurrence maintenance only

Reason:

- every later `simp` feature depends on having correct original-clause indexing
- it is the lowest-risk slice that materially advances the faithful port
- it does not yet force decisions about proof logging
