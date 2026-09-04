// Port of the non-watch parts of src/inline.h (kissat 4.0.4).
// (Watch/connect helpers live in watch.rs per the containers cluster.)

use crate::internal::Solver;
use crate::literal;

#[inline]
pub fn export_literal(solver: &Solver, ilit: u32) -> i32 {
    let idx = literal::idx(ilit);
    let elit = solver.export_[idx as usize];
    debug_assert!(elit != 0);
    if literal::negated(ilit) != 0 {
        -elit
    } else {
        elit
    }
}

/// kissat_push_analyzed (inline.h).  PORT NOTE: the C `assigned *assigned`
/// array argument is always solver->assigned — folded into field access
/// (same for push_removable / push_poisoned below).
#[inline]
pub fn push_analyzed(solver: &mut Solver, idx: u32) {
    debug_assert!(idx < solver.vars());
    debug_assert!(!solver.assigned[idx as usize].analyzed());
    solver.assigned[idx as usize].set_analyzed(true);
    solver.analyzed.push(idx);
}

/// kissat_push_removable (inline.h).
#[inline]
pub fn push_removable(solver: &mut Solver, idx: u32) {
    debug_assert!(idx < solver.vars());
    debug_assert!(!solver.assigned[idx as usize].removable());
    solver.assigned[idx as usize].set_removable(true);
    solver.removable.push(idx);
}

/// kissat_push_poisoned (inline.h).
#[inline]
pub fn push_poisoned(solver: &mut Solver, idx: u32) {
    debug_assert!(idx < solver.vars());
    debug_assert!(!solver.assigned[idx as usize].poisoned());
    solver.assigned[idx as usize].set_poisoned(true);
    solver.poisoned.push(idx);
}

#[inline]
pub fn mark_removed_literal(solver: &mut Solver, ilit: u32) {
    let idx = literal::idx(ilit) as usize;
    let f = &mut solver.flags[idx];
    if f.fixed() {
        return;
    }
    if !f.eliminate() {
        f.set_eliminate(true);
        solver.statistics.variables_eliminate += 1;
    }
}

#[inline]
pub fn mark_added_literal(solver: &mut Solver, ilit: u32) {
    let idx = literal::idx(ilit) as usize;
    let negated = literal::negated(ilit);
    let f = &mut solver.flags[idx];
    if !f.subsume() {
        f.set_subsume(true);
        solver.statistics.variables_subsume += 1;
    }
    let bit: u8 = 1u8 << negated;
    if f.factor() & bit == 0 {
        f.factor_or(bit);
        solver.statistics.literals_factor += 1;
    }
}
