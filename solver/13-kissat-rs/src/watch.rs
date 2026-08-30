// Port of src/watch.h + src/watch.c (kissat 4.0.4), plus the watch/connect
// helpers of src/inline.h (which has no .c of its own).
//
// Watch word encoding (union watch over one u32, little-endian bitfields):
//
//   bits 0..=30  lit / ref   (binary_tagged_literal.lit : 31 /
//                             binary_tagged_reference.ref : 31)
//   bit  31      binary flag (true for binary watches)
//
//   binary watch   = 1 word:  other-lit | BINARY_FLAG
//   blocking watch = 1 word:  blocking-lit (flag clear), followed by
//   large watch    = 1 word:  reference    (flag clear)
//
// In watching mode a large clause occupies TWO consecutive words in a watch
// list (blocking word + reference word); in connected (non-watching) mode
// large clauses are single reference words.
//
// PORT NOTES:
//  - Watch lists are the vectors of vector.rs, identified by their literal;
//    C `watches *` parameters become the owning literal index.
//  - watch.c does `#define INLINE_SORT` and includes sort.c: the explicit
//    (values, assigned) sort_literals variant is crate::sort::
//    sort_literals_inline.
//  - kissat_remove_blocking_watch compares `tail.raw != ref` (full raw word);
//    ported as a raw u32 compare (refs are < 2^31 so the flag bit is clear).
//  - kissat_inlined_connect_clause takes (solver, all_watches, c, ref) in C;
//    all_watches is always solver->watches and c always dereferences ref, so
//    the port takes (solver, ref).
//  - `all_clauses` pointer iteration becomes reference iteration with the
//    successor reference computed *before* the loop body (as the C macro
//    does).

use crate::internal::Solver;
use crate::reference::Reference;
use crate::vector::INVALID_VECTOR_ELEMENT;

pub type Watch = u32; // union watch { ... unsigned raw; }

/// C `typedef vector watches` (one watch list).
pub type Watches = crate::vector::Vector;

pub const BINARY_FLAG: u32 = 1 << 31;

/// kissat_binary_watch.
#[inline]
pub fn binary_watch(lit: u32) -> Watch {
    debug_assert!(lit < BINARY_FLAG);
    lit | BINARY_FLAG
}

/// kissat_large_watch.
#[inline]
pub fn large_watch(ref_: Reference) -> Watch {
    debug_assert!(ref_ < BINARY_FLAG);
    ref_
}

/// kissat_blocking_watch.
#[inline]
pub fn blocking_watch(lit: u32) -> Watch {
    debug_assert!(lit < BINARY_FLAG);
    lit
}

/// watch.type.binary
#[inline]
pub fn watch_is_binary(w: Watch) -> bool {
    w & BINARY_FLAG != 0
}

/// watch.binary.lit / watch.blocking.lit
#[inline]
pub fn watch_lit(w: Watch) -> u32 {
    w & !BINARY_FLAG
}

/// watch.large.ref
#[inline]
pub fn watch_ref(w: Watch) -> Reference {
    w & !BINARY_FLAG
}

/*------------------------------------------------------------------------*/
// litwatch / litpair / litriple (watch.h)

#[derive(Clone, Copy)]
pub struct LitWatch {
    pub lit: u32,
    pub watch: Watch,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LitPair {
    pub lits: [u32; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LitTriple {
    pub lits: [u32; 3],
}

/// kissat_litpair: normalized (smaller literal first).
#[inline]
pub fn litpair(lit: u32, other: u32) -> LitPair {
    LitPair {
        lits: [
            if lit < other { lit } else { other },
            if lit < other { other } else { lit },
        ],
    }
}

/*------------------------------------------------------------------------*/
// inline.h watch/connect helpers

/// kissat_push_large_watch.
#[inline]
pub fn push_large_watch(solver: &mut Solver, lit: u32, ref_: Reference) {
    let watch = large_watch(ref_);
    crate::vector::push_vectors(solver, lit, watch); // PUSH_WATCHES
}

/// kissat_push_binary_watch.
#[inline]
pub fn push_binary_watch(solver: &mut Solver, lit: u32, other: u32) {
    let watch = binary_watch(other);
    crate::vector::push_vectors(solver, lit, watch);
}

/// kissat_push_blocking_watch.
#[inline]
pub fn push_blocking_watch(solver: &mut Solver, lit: u32, blocking: u32, ref_: Reference) {
    debug_assert!(solver.watching);
    let head = blocking_watch(blocking);
    crate::vector::push_vectors(solver, lit, head);
    let tail = large_watch(ref_);
    crate::vector::push_vectors(solver, lit, tail);
}

/// kissat_watch_other: watch `lit` blocking `other` (binary).
#[inline]
pub fn watch_other(solver: &mut Solver, lit: u32, other: u32) {
    push_binary_watch(solver, lit, other);
}

/// kissat_watch_binary.
#[inline]
pub fn watch_binary(solver: &mut Solver, a: u32, b: u32) {
    watch_other(solver, a, b);
    watch_other(solver, b, a);
}

/// kissat_watch_blocking.
#[inline]
pub fn watch_blocking(solver: &mut Solver, lit: u32, blocking: u32, ref_: Reference) {
    debug_assert!(solver.watching);
    push_blocking_watch(solver, lit, blocking, ref_);
}

/// kissat_unwatch_blocking.
#[inline]
pub fn unwatch_blocking(solver: &mut Solver, lit: u32, ref_: Reference) {
    debug_assert!(solver.watching);
    remove_blocking_watch(solver, lit, ref_);
}

/// kissat_disconnect_binary.
#[inline]
pub fn disconnect_binary(solver: &mut Solver, lit: u32, other: u32) {
    debug_assert!(!solver.watching);
    let watch = binary_watch(other);
    crate::vector::remove_from_vector(solver, lit, watch); // REMOVE_WATCHES
}

/// kissat_disconnect_reference.
#[inline]
pub fn disconnect_reference(solver: &mut Solver, lit: u32, ref_: Reference) {
    debug_assert!(!solver.watching);
    let watch = large_watch(ref_);
    crate::vector::remove_from_vector(solver, lit, watch);
}

/// kissat_watch_reference.
#[inline]
pub fn watch_reference(solver: &mut Solver, a: u32, b: u32, ref_: Reference) {
    debug_assert!(solver.watching);
    watch_blocking(solver, a, b, ref_);
    watch_blocking(solver, b, a, ref_);
}

/// kissat_connect_literal.
#[inline]
pub fn connect_literal(solver: &mut Solver, lit: u32, ref_: Reference) {
    debug_assert!(!solver.watching);
    push_large_watch(solver, lit, ref_);
}

/// kissat_inlined_connect_clause (see module PORT NOTES for the signature).
pub fn inlined_connect_clause(solver: &mut Solver, ref_: Reference) {
    debug_assert!(!solver.watching);
    let size = solver.arena.clause(ref_).size();
    for i in 0..size {
        let lit = solver.arena.clause(ref_).lit(i);
        push_large_watch(solver, lit, ref_);
    }
}

/// kissat_watch_clause.
pub fn watch_clause(solver: &mut Solver, ref_: Reference) {
    let (l0, l1) = {
        let c = solver.arena.clause(ref_);
        debug_assert!(c.searched() < c.size());
        (c.lit(0), c.lit(1))
    };
    watch_reference(solver, l0, l1, ref_);
}

/*------------------------------------------------------------------------*/
// watch.c

/// kissat_remove_binary_watch: remove the binary watch on `lit` from the
/// watch list of `watches_of` (C: (solver, watches, lit)), preserving the
/// relative order of the remaining watches.
pub fn remove_binary_watch(solver: &mut Solver, watches_of: u32, lit: u32) {
    let v = solver.watches[watches_of as usize];
    let begin = v.begin;
    let end = v.end;
    {
        let stack = &mut solver.vectors.stack;
        let mut q = begin;
        let mut p = begin;
        while p != end {
            // const watch watch = *q++ = *p++;
            let watch = stack[p];
            stack[q] = watch;
            q += 1;
            p += 1;
            if !watch_is_binary(watch) {
                // *q++ = *p++; (copy the reference word of a large watch)
                stack[q] = stack[p];
                q += 1;
                p += 1;
                continue;
            }
            let other = watch_lit(watch);
            if other != lit {
                continue;
            }
            q -= 1;
        }
        debug_assert!(begin + 1 <= end);
        stack[end - 1] = INVALID_VECTOR_ELEMENT; // end[-1] = empty
    }
    solver.watches[watches_of as usize].end = end - 1; // watches->end -= 1
    solver.vectors.usable += 1;
}

/// kissat_remove_blocking_watch: remove the (blocking, ref) watch pair for
/// `ref_` from the watch list of `watches_of`.
pub fn remove_blocking_watch(solver: &mut Solver, watches_of: u32, ref_: Reference) {
    debug_assert!(solver.watching);
    let v = solver.watches[watches_of as usize];
    let begin = v.begin;
    let end = v.end;
    {
        let stack = &mut solver.vectors.stack;
        let mut q = begin;
        let mut p = begin;
        while p != end {
            // const watch head = *q++ = *p++;
            let head = stack[p];
            stack[q] = head;
            q += 1;
            p += 1;
            if watch_is_binary(head) {
                continue;
            }
            // const watch tail = *q++ = *p++;
            let tail = stack[p];
            stack[q] = tail;
            q += 1;
            p += 1;
            if tail != ref_ {
                // C: tail.raw != ref (raw word compare)
                continue;
            }
            q -= 2;
        }
        debug_assert!(begin + 2 <= end);
        stack[end - 2] = INVALID_VECTOR_ELEMENT; // end[-2] = end[-1] = empty
        stack[end - 1] = INVALID_VECTOR_ELEMENT;
    }
    solver.watches[watches_of as usize].end = end - 2; // watches->end -= 2
    solver.vectors.usable += 2;
}

/// kissat_substitute_large_watch: replace the first occurrence of `src` by
/// `dst` in the watch list of `watches_of`.
pub fn substitute_large_watch(solver: &mut Solver, watches_of: u32, src: Watch, dst: Watch) {
    debug_assert!(!solver.watching);
    let v = solver.watches[watches_of as usize];
    let stack = &mut solver.vectors.stack;
    let mut p = v.begin;
    while p != v.end {
        let head = stack[p];
        if head == src {
            stack[p] = dst;
            break;
        }
        p += 1;
    }
}

/// kissat_flush_all_connected.
pub fn flush_all_connected(solver: &mut Solver) {
    debug_assert!(!solver.watching);
    let lits = 2 * solver.vars; // LITS
    for lit in 0..lits {
        crate::vector::release_vector(solver, lit); // RELEASE_WATCHES
    }
}

/// kissat_flush_large_watches: drop all large (blocking+ref) watch pairs,
/// deduplicate binary watches via marks, deleting duplicated binaries once
/// (from the smaller literal's side).
pub fn flush_large_watches(solver: &mut Solver) {
    debug_assert!(solver.watching);
    let lits = 2 * solver.vars; // LITS
    for lit in 0..lits {
        let v = solver.watches[lit as usize];
        let begin = v.begin;
        let end = v.end;
        let mut q = begin;
        let mut p = begin;
        while p != end {
            // const watch watch = *q++ = *p++;
            let watch = solver.vectors.stack[p];
            solver.vectors.stack[q] = watch;
            q += 1;
            p += 1;
            if !watch_is_binary(watch) {
                // p++, q--; (skip ref word, drop the pair)
                p += 1;
                q -= 1;
            } else {
                let other = watch_lit(watch);
                if solver.marks[other as usize] != 0 {
                    if lit < other {
                        crate::clause::delete_binary(solver, lit, other);
                    }
                    q -= 1;
                } else {
                    solver.marks[other as usize] = 1;
                }
            }
        }
        crate::vector::resize_vector(solver, lit, q - begin); // SET_END_OF_WATCHES
        let mut r = begin;
        while r != q {
            let watch = solver.vectors.stack[r];
            debug_assert!(watch_is_binary(watch));
            solver.marks[watch_lit(watch) as usize] = 0;
            r += 1;
        }
    }
}

/// kissat_watch_large_clauses: (re)watch every non-garbage large clause on
/// its two "smallest" literals after sorting them with the inline
/// sort_literals variant.
pub fn watch_large_clauses(solver: &mut Solver) {
    debug_assert!(solver.watching);
    let mut ref_: Reference = 0;
    // for (all_clauses (c)) — successor computed before the body.
    while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        if !solver.arena.clause(ref_).garbage() {
            {
                let arena = &mut solver.arena;
                let values = &solver.values;
                let assigned = &solver.assigned;
                let mut c = arena.clause_mut(ref_);
                let size = c.size();
                crate::sort::sort_literals_inline(values, assigned, size, c.lits_mut());
                c.set_searched(2);
            }
            let (l0, l1) = {
                let c = solver.arena.clause(ref_);
                (c.lit(0), c.lit(1))
            };
            push_blocking_watch(solver, l0, l1, ref_);
            push_blocking_watch(solver, l1, l0, ref_);
        }
        ref_ = next;
    }
}

/// kissat_connect_irredundant_large_clauses: connect every literal of every
/// non-garbage irredundant large clause (up to last_irredundant), marking
/// root-satisfied clauses garbage instead.
pub fn connect_irredundant_large_clauses(solver: &mut Solver) {
    debug_assert!(!solver.watching);
    let last_irredundant = solver.last_irredundant; // C: NULL if INVALID_REF
    let mut ref_: Reference = 0;
    while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        // if (last_irredundant && c > last_irredundant) break;
        if last_irredundant != crate::clause::INVALID_REF && ref_ > last_irredundant {
            break;
        }
        let (redundant, garbage, size) = {
            let c = solver.arena.clause(ref_);
            (c.redundant(), c.garbage(), c.size())
        };
        if redundant || garbage {
            ref_ = next;
            continue;
        }
        let mut satisfied = false;
        debug_assert!(solver.level == 0);
        for i in 0..size {
            let lit = solver.arena.clause(ref_).lit(i);
            let value = solver.values[lit as usize];
            if value <= 0 {
                continue;
            }
            satisfied = true;
            break;
        }
        if satisfied {
            crate::clause::mark_clause_as_garbage(solver, ref_);
            ref_ = next;
            continue;
        }
        inlined_connect_clause(solver, ref_);
        ref_ = next;
    }
}

/// kissat_flush_large_connected: drop all large clause references from the
/// (connected-mode) watch lists, keeping binary watches.
pub fn flush_large_connected(solver: &mut Solver) {
    debug_assert!(!solver.watching);
    let mut flushed: u64 = 0; // C: size_t (only logged)
    let lits = 2 * solver.vars; // LITS
    for lit in 0..lits {
        let v = solver.watches[lit as usize];
        let begin = v.begin;
        let end = v.end;
        let mut q = begin;
        {
            let stack = &mut solver.vectors.stack;
            let mut p = begin;
            while p != end {
                let head = stack[p];
                p += 1;
                if watch_is_binary(head) {
                    stack[q] = head;
                    q += 1;
                } else {
                    flushed += 1;
                }
            }
        }
        crate::vector::resize_vector(solver, lit, q - begin); // SET_END_OF_WATCHES
    }
    let _ = flushed; // LOG only
}
