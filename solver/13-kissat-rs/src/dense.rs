// Port of src/dense.c (kissat 4.0.4).
//
// PORT NOTE: dense.c was NOT in the original walk/preprocess/reorder/compact/
// krite cluster assignment, but kissat_enter_dense_mode /
// kissat_resume_sparse_mode are hard dependencies of walk.c (and later of
// eliminate/fastel/sweep/congruence/factor); it is ported here in full so the
// walk path is executable.  One .rs per .c is preserved.
//
// PORT NOTE: C `litpairs *irredundant` NULL/non-NULL becomes
// `Option<&mut Vec<LitPair>>`.
// PORT NOTE: the file-local `flush_large_watches` (dense.c) shadows
// kissat_flush_large_watches (watch.c); it is only ever called with a
// non-NULL `irredundant` (enter_dense_mode dispatches to the watch.c variant
// otherwise), so its C-internal `if (irredundant)` branches are ported with
// `irredundant` always present.
// PORT NOTE: dense.c does `#define INLINE_SORT` + `#include "sort.c"`, giving
// the (values, assigned)-explicit kissat_sort_literals variant — that is
// crate::sort::sort_literals_inline.
// PORT NOTE: LOG-only counters (flushed/collected/deduplicated/resumed_*)
// are omitted (LOGGING not defined).

use crate::internal::Solver;
use crate::reference::{Reference, INVALID_REF};
use crate::watch::LitPair;

// static void flush_large_watches (kissat *solver, litpairs *irredundant)
fn flush_large_watches(solver: &mut Solver, irredundant: &mut Vec<LitPair>) {
    debug_assert!(solver.level == 0);
    debug_assert!(solver.watching);
    let lits = solver.lits();
    // unsigneds *marked = &solver->analyzed;
    for lit in 0..lits {
        let lit_value = solver.values[lit as usize];
        let v = solver.watches[lit as usize];
        let begin = v.begin;
        let end = v.end;
        // C keeps a write cursor `q` for the irredundant == NULL case; this
        // static is only called with irredundant non-NULL, so `q` is never
        // advanced (the watch list is memset below) — cursor omitted.
        let mut p = begin;
        debug_assert!(solver.analyzed.is_empty());
        while p != end {
            // const watch watch = *p++;
            let watch = solver.vectors.stack[p];
            p += 1;
            if crate::watch::watch_is_binary(watch) {
                let other = crate::watch::watch_lit(watch);
                let other_value = solver.values[other as usize];
                if lit_value == 0 && other_value == 0 {
                    let mark = solver.marks[other as usize];
                    if mark != 0 {
                        if lit < other {
                            crate::clause::delete_binary(solver, lit, other);
                        }
                    } else {
                        solver.marks[other as usize] = 1;
                        solver.analyzed.push(other); // PUSH_STACK (*marked, other)
                        // if (irredundant) { ... } else *q++ = watch;
                        // (irredundant always present here — see PORT NOTE)
                        if lit < other {
                            irredundant.push(LitPair { lits: [lit, other] });
                        }
                    }
                } else {
                    debug_assert!(lit_value > 0 || other_value > 0);
                    if lit < other {
                        crate::clause::delete_binary(solver, lit, other);
                    }
                }
            } else {
                // flushed++; p++;
                p += 1;
            }
        }
        // if (irredundant) memset (watches, 0, sizeof *watches);
        solver.watches[lit as usize] = crate::watch::Watches::default();
        let _ = begin;
        // for (all_stack (unsigned, other, *marked)) marks[other] = 0;
        for i in 0..solver.analyzed.len() {
            let other = solver.analyzed[i];
            solver.marks[other as usize] = 0;
        }
        solver.analyzed.clear(); // CLEAR_ARRAY (*marked)
    }
    debug_assert!(solver.analyzed.is_empty());
    // if (irredundant) kissat_release_vectors (solver);
    crate::vector::release_vectors(solver);
}

/// Port of `kissat_enter_dense_mode`.
pub fn enter_dense_mode(solver: &mut Solver, irredundant: Option<&mut Vec<LitPair>>) {
    debug_assert!(solver.level == 0);
    debug_assert!(solver.watching);
    if let Some(irredundant) = irredundant {
        flush_large_watches(solver, irredundant);
    } else {
        crate::watch::flush_large_watches(solver);
    }
    solver.watching = false;
}

// static void resume_watching_irredundant_binaries
fn resume_watching_irredundant_binaries(solver: &mut Solver, binaries: &[LitPair]) {
    for litpair in binaries {
        let first = litpair.lits[0];
        let second = litpair.lits[1];

        debug_assert!(!solver.flags[(first >> 1) as usize].eliminated);
        debug_assert!(!solver.flags[(second >> 1) as usize].eliminated);

        // watch first_watch = kissat_binary_watch (second);
        // PUSH_WATCHES (*first_watches, first_watch);
        crate::watch::push_binary_watch(solver, first, second);
        crate::watch::push_binary_watch(solver, second, first);
    }
}

// static void resume_watching_large_clauses_after_elimination
fn resume_watching_large_clauses_after_elimination(solver: &mut Solver) {
    let mut ref_: Reference = 0;
    while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        if solver.arena.clause(ref_).garbage() {
            ref_ = next;
            continue;
        }
        let size = solver.arena.clause(ref_).size();
        let mut collect = false;
        for &lit in solver.arena.clause(ref_).lits() {
            if solver.values[lit as usize] > 0 {
                collect = true;
                break;
            }
            let idx = lit >> 1;
            if solver.flags[idx as usize].eliminated {
                collect = true;
                break;
            }
        }
        if collect {
            crate::clause::mark_clause_as_garbage(solver, ref_);
            ref_ = next;
            continue;
        }

        debug_assert!(size > 2);

        {
            let arena = &mut solver.arena;
            let values = &solver.values;
            let assigned = &solver.assigned;
            let mut c = arena.clause_mut(ref_);
            crate::sort::sort_literals_inline(values, assigned, size, c.lits_mut());
            c.set_searched(2);
        }

        let (l0, l1) = {
            let c = solver.arena.clause(ref_);
            (c.lit(0), c.lit(1))
        };
        crate::watch::push_blocking_watch(solver, l0, l1, ref_);
        crate::watch::push_blocking_watch(solver, l1, l0, ref_);

        ref_ = next;
    }
}

/// Port of `kissat_resume_sparse_mode`.
pub fn resume_sparse_mode(
    solver: &mut Solver,
    flush_eliminated: bool,
    irredundant: Option<&mut Vec<LitPair>>,
) {
    debug_assert!(solver.level == 0);
    debug_assert!(!solver.watching);
    if solver.inconsistent {
        return;
    }
    crate::watch::flush_large_connected(solver);
    solver.watching = true;
    if let Some(irredundant) = irredundant {
        resume_watching_irredundant_binaries(solver, irredundant);
    }
    if flush_eliminated {
        resume_watching_large_clauses_after_elimination(solver);
    } else {
        crate::watch::watch_large_clauses(solver);
    }
    // kissat_reset_propagate (solver);
    solver.propagate = 0;

    let conflict = if solver.probing {
        crate::proprobe::probing_propagate(solver, INVALID_REF, true).is_some()
    } else {
        crate::propsearch::search_propagate(solver).is_some()
    };
    let _ = conflict; // (void) conflict — NDEBUG
}
