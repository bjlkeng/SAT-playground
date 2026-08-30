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

#[inline]
pub fn mark_removed_literal(solver: &mut Solver, ilit: u32) {
    let idx = literal::idx(ilit) as usize;
    let f = &mut solver.flags[idx];
    if !f.eliminate {
        f.eliminate = true;
        solver.statistics.variables_eliminate += 1;
    }
    if !f.sweep {
        f.sweep = true;
    }
}

#[inline]
pub fn mark_added_literal(solver: &mut Solver, ilit: u32) {
    let idx = literal::idx(ilit) as usize;
    let f = &mut solver.flags[idx];
    if !f.eliminate {
        f.eliminate = true;
        solver.statistics.variables_eliminate += 1;
    }
    if !f.subsume {
        f.subsume = true;
        solver.statistics.variables_subsume += 1;
    }
    if !f.sweep {
        f.sweep = true;
    }
}
