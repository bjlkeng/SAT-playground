// Port of src/propbeyond.c (kissat 4.0.4).
//
// propbeyond.c includes proplit.h with CONTINUE_PROPAGATING_AFTER_CONFLICT
// defined: the inner loop does NOT break on conflicts and the outer loop
// counts every conflicting literal (warming propagation during rephasing).
// The template lives in propsearch.rs; this file instantiates
// propagate_literal::<false, true>.
//
// PORT NOTES:
//  - Statistics tiers (reference build): ticks / propagations /
//    warming_propagations are real fields; warming_conflicts is
//    STATISTIC-tier, kept as a real counter per statistics.rs policy.

use crate::internal::Solver;
use crate::profile::Prof;
use crate::propsearch::{propagate_literal, Conflict};
use crate::reference::INVALID_REF;

/// PROPAGATE_LITERAL instantiation for propbeyond.c
/// (propagate_literal_beyond_conflicts).
#[inline]
fn propagate_literal_beyond_conflicts(solver: &mut Solver, lit: u32) -> Option<Conflict> {
    propagate_literal::<false, true>(solver, INVALID_REF, lit)
}

fn update_beyond_propagation_statistics(solver: &mut Solver, saved_propagate: usize) {
    debug_assert!(saved_propagate <= solver.propagate);
    let propagated = (solver.propagate - saved_propagate) as u64;

    solver.statistics.ticks += solver.ticks; // ADD (ticks, ...)

    solver.statistics.propagations += propagated; // ADD (propagations, ...)
    solver.statistics.warming_propagations += propagated; // ADD (warming_propagations, ...)
}

// C static `propagate_literals_beyond_conflicts`.
fn propagate_literals_beyond_conflicts(solver: &mut Solver) {
    let mut propagate = solver.propagate;
    while propagate != solver.trail.len() {
        let lit = solver.trail[propagate];
        propagate += 1;
        if propagate_literal_beyond_conflicts(solver, lit).is_some() {
            solver.statistics.warming_conflicts += 1; // INC (warming_conflicts)
        }
    }
    solver.propagate = propagate;
}

/// kissat_propagate_beyond_conflicts.
pub fn propagate_beyond_conflicts(solver: &mut Solver) {
    debug_assert!(!solver.probing);
    debug_assert!(solver.watching);
    debug_assert!(solver.warming);
    debug_assert!(!solver.inconsistent);

    crate::profile::start_checked(solver, Prof::propagate); // START (propagate)

    solver.ticks = 0;
    let saved_propagate = solver.propagate;
    propagate_literals_beyond_conflicts(solver);
    update_beyond_propagation_statistics(solver, saved_propagate);

    crate::profile::stop_checked(solver, Prof::propagate); // STOP (propagate)
}
