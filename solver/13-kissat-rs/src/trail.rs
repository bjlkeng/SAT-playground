// Port of src/trail.c (kissat 4.0.4).
//
// PORT NOTES:
//  - kissat_propagated (inline.h) is the `solver.propagate ==
//    solver.trail.len()` cursor check (trail is a Vec + index cursor, see
//    internal.rs); kissat_reset_propagate is `solver.propagate = 0`.
//  - The reason-clause loops cache `ward *arena = BEGIN_STACK (...)` in C;
//    here each marked clause is addressed through arena.clause_mut(ref) —
//    same memory, the arena cannot move inside the loop.
//  - kissat_backtrack_propagate_and_flush_trail lives in backtrack.c
//    (backtrack wave; currently a stub in stubs.rs).

use crate::internal::{Solver, DECISION_REASON, UNIT_REASON};
use crate::reference::Reference;

/// kissat_flush_trail.
pub fn flush_trail(solver: &mut Solver) {
    debug_assert!(solver.level == 0);
    debug_assert!(solver.unflushed != 0);
    debug_assert!(!solver.inconsistent);
    debug_assert!(solver.propagate == solver.trail.len()); // kissat_propagated
    debug_assert!(solver.trail.len() == solver.unflushed as usize);
    solver.trail.clear(); // CLEAR_ARRAY (solver->trail)
    solver.propagate = 0; // kissat_reset_propagate
    solver.unflushed = 0;
}

/// kissat_mark_reason_clauses.
pub fn mark_reason_clauses(solver: &mut Solver, start: Reference) {
    debug_assert!(solver.unflushed == 0);
    for i in 0..solver.trail.len() {
        let lit = solver.trail[i];
        let a = solver.assigned[crate::literal::idx(lit) as usize];
        debug_assert!(a.level > 0);
        if a.binary {
            continue;
        }
        let ref_ = a.reason;
        debug_assert!(ref_ != UNIT_REASON);
        if ref_ == DECISION_REASON {
            continue;
        }
        if ref_ < start {
            continue;
        }
        solver.arena.clause_mut(ref_).set_reason(true);
    }
}

/// kissat_flush_and_mark_reason_clauses.
pub fn flush_and_mark_reason_clauses(solver: &mut Solver, start: Reference) -> bool {
    debug_assert!(solver.watching);
    debug_assert!(!solver.inconsistent);
    debug_assert!(solver.propagate == solver.trail.len()); // kissat_propagated

    if solver.unflushed != 0 {
        crate::backtrack::backtrack_propagate_and_flush_trail(solver);
    } else {
        mark_reason_clauses(solver, start);
    }

    true
}

/// kissat_unmark_reason_clauses.
pub fn unmark_reason_clauses(solver: &mut Solver, start: Reference) {
    debug_assert!(solver.unflushed == 0);
    for i in 0..solver.trail.len() {
        let lit = solver.trail[i];
        let a = solver.assigned[crate::literal::idx(lit) as usize];
        debug_assert!(a.level > 0);
        if a.binary {
            continue;
        }
        let ref_ = a.reason;
        debug_assert!(ref_ != UNIT_REASON);
        if ref_ == DECISION_REASON {
            continue;
        }
        if ref_ < start {
            continue;
        }
        debug_assert!(solver.arena.clause(ref_).reason());
        solver.arena.clause_mut(ref_).set_reason(false);
    }
}
