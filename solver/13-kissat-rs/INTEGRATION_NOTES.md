# Resolved cross-module conventions (read AFTER CONVENTIONS.md)

These are the actual APIs as integrated and compiling at the foundation
commit. Write new modules against these — do not guess variants.

- Print (print.rs): `crate::print::{message, verbose, very_verbose,
  extremely_verbose, warning}(solver: &Solver, args: impl std::fmt::Display)`
  — pass `format!(...)` or `format_args!(...)`.
  `phase(solver, name: &str, count: u64, args: impl Display)`;
  `section(solver: &mut Solver, name)`; `verbosity(&Solver) -> i32`;
  `line`, `prefix`.
- Profiles: `crate::profile::Prof` enum, variants are C names verbatim
  lowercase (`Prof::search`, `Prof::focused`, `Prof::walking`...).
  START/STOP macros → `crate::profile::start_checked(solver, Prof::x)` /
  `stop_checked` (they include the `GET_OPTION(profile) >= level` guard).
  Simplifier transitions: `stop_search_and_start_simplifier[_checked]`,
  `stop_simplifier_and_resume_search[_checked]`.
- Statistics: `Statistics` has ONLY the 80 COUNTER fields (build has neither
  METRICS nor STATISTICS defined). `INC/ADD/DEC` on COUNTER →
  `solver.statistics.x += 1/n` (`-=` for DEC). On METRIC/STATISTIC entries:
  INC/ADD are no-ops; `GET` yields `u64::MAX` (kissat_phase then prints no
  count). Check statistics.h for each counter's tier before porting.
- Options: `solver.options.<name>` (i32). Configs via crate::config.
- Literals: `crate::literal::{idx, lit, not, negated, strip}`; NOTE
  `negated(lit) -> u32` (the sign bit, not bool — compare `!= 0`).
  `EXTERNAL_MAX_VAR: i32`. `crate::internal::INVALID = u32::MAX`.
- Heap: free fns over the container: `crate::heap::{push_heap, pop_heap,
  pop_max_heap, update_heap, adjust_heap, rescale_heap, heap_contains,
  get_heap_score, max_heap, ...}(&mut Heap / &Heap, ...)` — no solver arg.
- Queue: `crate::inlinequeue::{enqueue, dequeue, move_to_front,
  update_queue, enqueue_links}(solver, idx)`;
  `dequeue_links(idx, &mut [Links], &mut Queue)`.
  `crate::queue::{init_queue, reset_search_of_queue, reassign_queue_stamps}`.
- Clauses/arena: refs are `crate::reference::Reference` (u32).
  `solver.arena.clause(ref) -> ClauseRef`, `.clause_mut(ref) -> ClauseMut`
  (accessors `glue/size/searched/lit(i)/lits()/redundant/...` + setters),
  `arena.next_clause_ref(ref)`. Constructors in crate::clause:
  `new_original_clause`, `new_redundant_clause`, `new_irredundant_clause`,
  `new_binary_clause`, `new_unwatched_binary_clause`;
  `mark_clause_as_garbage(solver, ref)`, `delete_clause`, `delete_binary`.
- Watches: `type Watch = u32` tagged words; helpers in crate::watch:
  `binary_watch/large_watch/blocking_watch/watch_is_binary/watch_lit/
  watch_ref`, `push_binary_watch/push_large_watch/push_blocking_watch`,
  `watch_other`, `disconnect_binary/disconnect_reference`, `connect_literal`,
  `inlined_connect_clause`, `watch_clause`, `flush_large_watches`,
  `watch_large_clauses`... `solver.watches[lit as usize]` is a
  `Vector` into `solver.vectors` (crate::vector for push/remove/defrag).
- Vector iteration: get `begin/end` word offsets from the Vector, index
  `solver.vectors.stack[...]`; when mutating a watch list while iterating,
  mirror kissat's in-place two-cursor compaction (see flush fns in watch.rs).
- Assigned/flags/values/marks/links/phases live as Vecs on Solver indexed by
  var or lit (`solver.values[lit as usize]`, `solver.assigned[idx as usize]`).
- Frames: `crate::frames::push_frame(solver, decision)`;
  `solver.frames[level as usize]`.
- Proof call sites: `crate::proof::{add_binary_to_proof, add_clause_to_proof,
  delete_clause_from_proof, delete_binary_from_proof, add_empty_to_proof,
  add_lits_to_proof, delete_external_from_proof}` (stub until proof wave;
  keep exact call sites).
- Inline helpers: `crate::inline::{export_literal, mark_removed_literal,
  mark_added_literal}`; `crate::collect::{defrag_watches,
  defrag_watches_if_needed}`.
- Sort: `crate::sort::{radix_sort, sort_literals, sort(sorter, slice, less)}`
  (see sort.rs signatures; solver.sorter is the QUICK_SORT work stack).
- Randomness: `crate::random::{next_random32, next_random64, pick_random,
  pick_bool, pick_double}(&mut solver.random, ...)`.
- Ticks: `solver.ticks` plus per-engine statistics counters; use
  `crate::utilities::cache_lines(n, 4)` exactly at kissat's call sites.
- Temporary stubs pending waves: `src/stubs.rs` re-exports
  backtrack/bump/decide/import/propsearch/resize/restart/search — when your
  wave ports one of these, create the real `src/<name>.rs` and note it; the
  integrator removes the re-export. assign.rs/tiers.rs/proof.rs/extend.rs
  contain marked STUB sections to replace wholesale.
- Borrow-checker patterns in use: `std::mem::take` a Vec field around calls
  that need `&mut Solver` (restore after, preserving C effect order);
  index-based loops instead of held references; split borrows via helper fns
  taking `(&mut [T], &mut U)` where C passed two pointers into solver.
