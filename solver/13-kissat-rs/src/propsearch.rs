// Port of src/propsearch.c + the shared propagation template src/proplit.h
// (kissat 4.0.4).
//
// C structure: proplit.h is a textual template included by propsearch.c,
// propbeyond.c, propinitially.c and proprobe.c after #defining
//   PROPAGATE_LITERAL                       (the generated function name),
//   CONTINUE_PROPAGATING_AFTER_CONFLICT     (propbeyond.c only), and
//   PROBING_PROPAGATION                     (proprobe.c only: extra `ignore`
//                                            clause parameter, ignored-
//                                            conflict handling, no
//                                            INC (conflicts) in
//                                            kissat_update_conflicts_and_trail,
//                                            no saved-phase store in
//                                            kissat_fast_assign).
// Here the template is ONE generic inner function `propagate_literal` with
// two const-generic flags plus an `ignore` reference argument (INVALID_REF
// when unused).  propbeyond.rs instantiates <false, true>, propinitially.rs
// <false, false>, and the probe wave instantiates <true, false> for
// proprobe.c (the saved-phase difference is covered by the runtime
// `!probing` check in assign.rs — see its PORT NOTES).
// kissat_update_conflicts_and_trail and the delayed-watch helpers are also
// proplit.h template code and live here, shared the same way.
//
// PORT NOTES:
//  - C propagation returns `clause *`: NULL, &solver->conflict (the embedded
//    fake binary-conflict header filled by kissat_binary_conflict), or an
//    arena clause.  Ported as Option<Conflict> with
//    Conflict::{Binary, Clause(Reference)}; kissat_binary_conflict
//    (inline.h) is defined here because the handle type is.
//  - The C static `search_propagate` collides with kissat_search_propagate
//    once the kissat_ prefix is dropped; the static loop is renamed
//    `search_propagate_all` (same for propinitially.rs).
//  - Hot-loop pointer arithmetic (the p/q watch-list cursors, arena clause
//    field access, the replacement search from `searched`) is index
//    arithmetic with unsafe unchecked indexing; every read/write happens in
//    C effect order.
//  - `q[-2].blocking.lit = other` writes the whole watch word: the binary
//    flag of a blocking watch is clear, and blocking_watch(other) == other,
//    so the full-word store equals the C bitfield store.
//  - SET_END_OF_WATCHES == kissat_resize_vector (watch.h): poisons the freed
//    suffix and counts it usable — crate::vector::resize_vector.
//  - update_search_propagation_statistics: search_propagations and the
//    whole stable/focused propagations/ticks block are METRIC counters,
//    compiled out in the reference build; propagations, ticks and
//    search_ticks are real (COUNTER/STATISTIC tiers per statistics.rs).
//  - kissat_watch_large_delayed iterates solver->delayed while pushing into
//    other literals' watch lists; the stack is mem::take'n around the loop
//    (nothing reads `delayed` during the pushes) and cleared before being
//    put back, preserving C effect order and CLEAR_STACK capacity behavior.

use crate::arena::WORDS_PER_WARD;
use crate::clause::{GARBAGE_BIT, HEADER_OFFSET, LITS_OFFSET, SEARCHED_OFFSET, SIZE_OFFSET};
use crate::internal::Solver;
use crate::literal::INVALID_LIT;
use crate::profile::Prof;
use crate::reference::{Reference, INVALID_REF};
use crate::watch::Watch;

/*------------------------------------------------------------------------*/

/// The non-NULL `clause *` results of propagation (see module PORT NOTES).
/// `Binary` is C's `&solver->conflict`; its two literals live in
/// solver.conflict.lits[0..2].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Conflict {
    Binary,
    Clause(Reference),
}

/// kissat_binary_conflict (inline.h): fills the embedded fake binary
/// conflict clause header and returns the handle to it.
pub fn binary_conflict(solver: &mut Solver, a: u32, b: u32) -> Conflict {
    let res = &mut solver.conflict;
    res.size = 2;
    res.lits[0] = a;
    res.lits[1] = b;
    Conflict::Binary
}

/*------------------------------------------------------------------------*/
// proplit.h template code (shared with propbeyond/propinitially/proprobe).

/// kissat_delay_watching_large (proplit.h).
#[inline]
fn delay_watching_large(solver: &mut Solver, lit: u32, other: u32, ref_: Reference) {
    let watch = crate::watch::blocking_watch(other);
    solver.delayed.push(lit);
    solver.delayed.push(watch);
    solver.delayed.push(ref_);
}

/// kissat_watch_large_delayed (proplit.h).
#[inline]
fn watch_large_delayed(solver: &mut Solver) {
    let delayed = std::mem::take(&mut solver.delayed);
    let end_delayed = delayed.len();
    let mut d = 0;
    while d != end_delayed {
        let lit = delayed[d];
        d += 1;
        debug_assert!(d != end_delayed);
        let watch = delayed[d];
        d += 1;
        debug_assert!(!crate::watch::watch_is_binary(watch));
        debug_assert!(d != end_delayed);
        let ref_ = delayed[d];
        d += 1;
        let blocking = crate::watch::watch_lit(watch); // watch.blocking.lit
        crate::watch::push_blocking_watch(solver, lit, blocking, ref_);
    }
    let mut delayed = delayed;
    delayed.clear(); // CLEAR_STACK (*delayed)
    solver.delayed = delayed;
}

/// PROPAGATE_LITERAL (proplit.h) — the shared inner propagation loop.
/// `ignore` is only inspected under PROBING_PROPAGATION (pass INVALID_REF
/// otherwise); C compares clause pointers, references are equivalent.
#[inline]
pub(crate) fn propagate_literal<
    const PROBING_PROPAGATION: bool,
    const CONTINUE_PROPAGATING_AFTER_CONFLICT: bool,
>(
    solver: &mut Solver,
    ignore: Reference,
    lit: u32,
) -> Option<Conflict> {
    debug_assert!(solver.watching);
    debug_assert!(solver.values[lit as usize] > 0);
    debug_assert!(solver.delayed.is_empty());

    let not_lit = crate::literal::not(lit);
    debug_assert!(not_lit < solver.lits());

    let watches = solver.watches[not_lit as usize];
    let begin_watches = watches.begin;
    let end_watches = watches.end;

    let mut q = begin_watches;
    let mut p = q;

    let size_watches = end_watches - begin_watches;
    let mut ticks: u64 = 1 + crate::utilities::cache_lines(size_watches as u64, 4);
    let idx = crate::literal::idx(lit);
    let level = unsafe { solver.assigned.get_unchecked(idx as usize).level };
    let probing = solver.probing;
    let mut res: Option<Conflict> = None;

    while p != end_watches {
        // const watch head = *q++ = *p++;
        let head = unsafe { *solver.vectors.stack.get_unchecked(p) };
        unsafe { *solver.vectors.stack.get_unchecked_mut(q) = head };
        q += 1;
        p += 1;
        let blocking = crate::watch::watch_lit(head); // head.blocking.lit
        let blocking_value = unsafe { *solver.values.get_unchecked(blocking as usize) };
        let binary = crate::watch::watch_is_binary(head);
        let mut tail: Watch = 0;
        if !binary {
            // tail = *q++ = *p++;
            tail = unsafe { *solver.vectors.stack.get_unchecked(p) };
            unsafe { *solver.vectors.stack.get_unchecked_mut(q) = tail };
            q += 1;
            p += 1;
        }
        if blocking_value > 0 {
            continue;
        }
        if binary {
            if blocking_value < 0 {
                res = Some(binary_conflict(solver, not_lit, blocking));
                if !CONTINUE_PROPAGATING_AFTER_CONFLICT {
                    break;
                }
            } else {
                debug_assert!(blocking_value == 0);
                crate::assign::fast_binary_assign(solver, probing, level, blocking, not_lit);
                ticks += 1;
            }
        } else {
            let ref_: Reference = tail; // tail.raw
            debug_assert!((ref_ as u64) < solver.arena.size_wards());
            let c = ref_ as usize * WORDS_PER_WARD; // clause word offset
            ticks += 1;
            let header = unsafe { *solver.arena.words().get_unchecked(c + HEADER_OFFSET) };
            if header & GARBAGE_BIT != 0 {
                q -= 2;
                continue;
            }
            let lits = c + LITS_OFFSET; // BEGIN_LITS (c)
            let lit0 = unsafe { *solver.arena.words().get_unchecked(lits) };
            let lit1 = unsafe { *solver.arena.words().get_unchecked(lits + 1) };
            let other = lit0 ^ lit1 ^ not_lit;
            debug_assert!(lit0 != lit1);
            debug_assert!(not_lit != other);
            debug_assert!(lit != other);
            let other_value = unsafe { *solver.values.get_unchecked(other as usize) };
            if other_value > 0 {
                // q[-2].blocking.lit = other; (see module PORT NOTES)
                unsafe {
                    *solver.vectors.stack.get_unchecked_mut(q - 2) =
                        crate::watch::blocking_watch(other)
                };
                continue;
            }
            let size =
                unsafe { *solver.arena.words().get_unchecked(c + SIZE_OFFSET) } as usize;
            let searched =
                unsafe { *solver.arena.words().get_unchecked(c + SEARCHED_OFFSET) } as usize;
            debug_assert!(2 <= searched);
            debug_assert!(searched < size);
            let mut r = searched;
            let mut replacement: u32 = INVALID_LIT;
            let mut replacement_value: i8 = -1;
            while r != size {
                replacement = unsafe { *solver.arena.words().get_unchecked(lits + r) };
                replacement_value =
                    unsafe { *solver.values.get_unchecked(replacement as usize) };
                if replacement_value >= 0 {
                    break;
                }
                r += 1;
            }
            if replacement_value < 0 {
                r = 2;
                while r != searched {
                    replacement = unsafe { *solver.arena.words().get_unchecked(lits + r) };
                    replacement_value =
                        unsafe { *solver.values.get_unchecked(replacement as usize) };
                    if replacement_value >= 0 {
                        break;
                    }
                    r += 1;
                }
            }

            if replacement_value >= 0 {
                // c->searched = r - lits;
                unsafe {
                    *solver.arena.words_mut().get_unchecked_mut(c + SEARCHED_OFFSET) = r as u32
                };
                debug_assert!(replacement != INVALID_LIT);
                q -= 2;
                unsafe {
                    *solver.arena.words_mut().get_unchecked_mut(lits) = other;
                    *solver.arena.words_mut().get_unchecked_mut(lits + 1) = replacement;
                    *solver.arena.words_mut().get_unchecked_mut(lits + r) = not_lit;
                }
                delay_watching_large(solver, replacement, other, ref_);
                ticks += 1;
            } else if other_value != 0 {
                debug_assert!(replacement_value < 0);
                debug_assert!(blocking_value < 0);
                debug_assert!(other_value < 0);
                if PROBING_PROPAGATION && ref_ == ignore {
                    continue; // conflicting but ignored
                }
                res = Some(Conflict::Clause(ref_));
                if !CONTINUE_PROPAGATING_AFTER_CONFLICT {
                    break;
                }
            } else {
                debug_assert!(replacement_value < 0);
                if PROBING_PROPAGATION && ref_ == ignore {
                    continue; // forcing but ignored
                }
                crate::assign::fast_assign_reference(solver, other, ref_);
                ticks += 1;
            }
        }
    }
    solver.ticks += ticks;

    while p != end_watches {
        // *q++ = *p++;
        let w = unsafe { *solver.vectors.stack.get_unchecked(p) };
        unsafe { *solver.vectors.stack.get_unchecked_mut(q) = w };
        q += 1;
        p += 1;
    }
    crate::vector::resize_vector(solver, not_lit, q - begin_watches); // SET_END_OF_WATCHES

    watch_large_delayed(solver);

    res
}

/// kissat_update_conflicts_and_trail (proplit.h).  Under PROBING_PROPAGATION
/// (proprobe.c) the INC (conflicts) is compiled out.
pub(crate) fn update_conflicts_and_trail<const PROBING_PROPAGATION: bool>(
    solver: &mut Solver,
    conflict: Option<Conflict>,
    flush: bool,
) {
    if conflict.is_some() {
        if !PROBING_PROPAGATION {
            solver.statistics.conflicts += 1; // INC (conflicts)
        }
        if solver.level == 0 {
            solver.inconsistent = true;
            // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
            // ADD_EMPTY_TO_PROOF:
            if solver.proof.is_some() {
                crate::proof::add_empty_to_proof(solver);
            }
        }
    } else if flush && solver.level == 0 && solver.unflushed != 0 {
        crate::trail::flush_trail(solver);
    }
}

/*------------------------------------------------------------------------*/
// propsearch.c proper.

/// PROPAGATE_LITERAL instantiation for propsearch.c
/// (search_propagate_literal).
#[inline]
fn search_propagate_literal(solver: &mut Solver, lit: u32) -> Option<Conflict> {
    propagate_literal::<false, false>(solver, INVALID_REF, lit)
}

fn update_search_propagation_statistics(solver: &mut Solver, saved_propagate: usize) {
    debug_assert!(saved_propagate <= solver.propagate);
    let propagated = (solver.propagate - saved_propagate) as u64;

    solver.statistics.propagations += propagated; // ADD (propagations, ...)
    solver.statistics.ticks += solver.ticks; // ADD (ticks, ...)

    // ADD (search_propagations, propagated): METRIC, compiled out.
    solver.statistics.search_ticks += solver.ticks; // ADD (search_ticks, ...)

    // if (solver->stable) { ADD (stable_propagations/stable_ticks) } else
    // { ADD (focused_propagations/focused_ticks) }: all METRIC, compiled out.
}

/// C static `search_propagate` (renamed, see module PORT NOTES).
fn search_propagate_all(solver: &mut Solver) -> Option<Conflict> {
    let mut res: Option<Conflict> = None;
    let mut propagate = solver.propagate;
    while res.is_none() && propagate != solver.trail.len() {
        let lit = solver.trail[propagate];
        propagate += 1;
        res = search_propagate_literal(solver, lit);
    }
    solver.propagate = propagate;
    res
}

/// kissat_search_propagate.
pub fn search_propagate(solver: &mut Solver) -> Option<Conflict> {
    debug_assert!(!solver.probing);
    debug_assert!(solver.watching);
    debug_assert!(!solver.inconsistent);

    crate::profile::start_checked(solver, Prof::propagate); // START (propagate)

    solver.ticks = 0;
    let saved_propagate = solver.propagate;
    let conflict = search_propagate_all(solver);
    update_search_propagation_statistics(solver, saved_propagate);
    update_conflicts_and_trail::<false>(solver, conflict, true);
    if conflict.is_some() && solver.randec != 0 {
        // if (!--solver->randec) ...
        solver.randec -= 1;
        if solver.randec == 0 {
            crate::print::very_verbose(solver, "last random decision conflict");
        } else if solver.randec == 1 {
            crate::print::very_verbose(solver, "one more random decision conflict to go");
        } else {
            let count =
                crate::format::format_count(&mut solver.format, solver.randec as u64);
            crate::print::very_verbose(
                solver,
                format!("{} more random decision conflicts to go", count),
            );
        }
    }

    crate::profile::stop_checked(solver, Prof::propagate); // STOP (propagate)

    conflict
}
