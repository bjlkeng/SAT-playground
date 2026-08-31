// Port of src/propinitially.c (kissat 4.0.4).
//
// propinitially.c includes proplit.h with neither PROBING_PROPAGATION nor
// CONTINUE_PROPAGATING_AFTER_CONFLICT: the template instantiation is
// propagate_literal::<false, false> (identical inner loop to propsearch's;
// the file differs only in statistics and the root-level conflict analysis).
//
// PORT NOTES:
//  - The C static `initially_propagate` collides with
//    kissat_initially_propagate after the kissat_ prefix drop; the static
//    loop is renamed `initially_propagate_all`.
//  - kissat_analyze on the root-level conflict belongs to the analyze wave
//    (currently a stub in stubs.rs); it must set solver.inconsistent and
//    return 20 exactly as asserted in C.

use crate::internal::Solver;
use crate::profile::Prof;
use crate::propsearch::{propagate_literal, update_conflicts_and_trail, Conflict};
use crate::reference::INVALID_REF;

/// PROPAGATE_LITERAL instantiation for propinitially.c
/// (initially_propagate_literal).
#[inline]
fn initially_propagate_literal(solver: &mut Solver, lit: u32) -> Option<Conflict> {
    propagate_literal::<false, false>(solver, INVALID_REF, lit)
}

fn update_initial_propagation_statistics(solver: &mut Solver, saved_propagate: usize) {
    debug_assert!(saved_propagate <= solver.propagate);
    let propagated = (solver.propagate - saved_propagate) as u64;

    solver.statistics.propagations += propagated; // ADD (propagations, ...)
    solver.statistics.ticks += solver.ticks; // ADD (ticks, ...)
}

/// C static `initially_propagate` (renamed, see module PORT NOTES).
fn initially_propagate_all(solver: &mut Solver) -> Option<Conflict> {
    let mut res: Option<Conflict> = None;
    let mut propagate = solver.propagate;
    while res.is_none() && propagate != solver.trail.len() {
        let lit = solver.trail[propagate];
        propagate += 1;
        res = initially_propagate_literal(solver, lit);
    }
    solver.propagate = propagate;
    res
}

/// kissat_initially_propagate.
pub fn initially_propagate(solver: &mut Solver) -> bool {
    debug_assert!(!solver.probing);
    debug_assert!(solver.watching);
    debug_assert!(!solver.inconsistent);

    crate::profile::start_checked(solver, Prof::propagate); // START (propagate)

    solver.ticks = 0;
    let saved_propagate = solver.propagate;
    let conflict = initially_propagate_all(solver);
    update_initial_propagation_statistics(solver, saved_propagate);
    update_conflicts_and_trail::<false>(solver, conflict, true);
    if let Some(conflict) = conflict {
        let res = crate::analyze::analyze(solver, conflict);
        debug_assert!(solver.inconsistent);
        debug_assert!(res == 20);
        let _ = res; // (void) res;
    }

    crate::profile::stop_checked(solver, Prof::propagate); // STOP (propagate)

    conflict.is_none()
}
