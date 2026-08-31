// Port of src/proprobe.c (kissat 4.0.4).
//
// proprobe.c is the PROBING_PROPAGATION instantiation of the proplit.h
// propagation template; the shared template lives in propsearch.rs as
// `propagate_literal::<PROBING_PROPAGATION, CONTINUE_PROPAGATING_AFTER_
// CONFLICT>` — here instantiated <true, false> with a real `ignore`
// reference (C's `clause *ignore`, NULL == INVALID_REF).
//
// PORT NOTES:
//  - update_probing_propagation_statistics: probing_propagations is a METRIC
//    counter (compiled out in the reference build), as is the whole
//    backbone/vivify `#if defined(METRICS)` block; propagations (COUNTER),
//    probing_ticks (COUNTER) and ticks (STATISTIC) are real — same tier
//    treatment as propsearch.rs.
//  - kissat_update_conflicts_and_trail is proplit.h template code shared in
//    propsearch.rs; the <true> instantiation skips INC (conflicts).

use crate::internal::Solver;
use crate::profile::Prof;
use crate::propsearch::{propagate_literal, update_conflicts_and_trail, Conflict};
use crate::reference::Reference;

/// PROPAGATE_LITERAL instantiation for proprobe.c
/// (probing_propagate_literal).
#[inline]
fn probing_propagate_literal(
    solver: &mut Solver,
    ignore: Reference,
    lit: u32,
) -> Option<Conflict> {
    propagate_literal::<true, false>(solver, ignore, lit)
}

fn update_probing_propagation_statistics(solver: &mut Solver, propagated: u64) {
    let ticks = solver.ticks;

    solver.statistics.propagations += propagated; // ADD (propagations, ...)
    // ADD (probing_propagations, propagated): METRIC, compiled out.

    // #if defined(METRICS) backbone/vivify propagations/ticks: compiled out.

    solver.statistics.probing_ticks += ticks; // ADD (probing_ticks, ...)
    solver.statistics.ticks += ticks; // ADD (ticks, ...)
}

/// kissat_probing_propagate.  C's `clause *ignore` NULL == INVALID_REF.
pub fn probing_propagate(
    solver: &mut Solver,
    ignore: Reference,
    flush: bool,
) -> Option<Conflict> {
    debug_assert!(solver.probing);
    debug_assert!(solver.watching);
    debug_assert!(!solver.inconsistent);

    crate::profile::start_checked(solver, Prof::propagate); // START (propagate)

    let mut conflict: Option<Conflict> = None;
    let mut propagate = solver.propagate;
    solver.ticks = 0;
    while conflict.is_none() && propagate != solver.trail.len() {
        let lit = solver.trail[propagate];
        propagate += 1;
        conflict = probing_propagate_literal(solver, ignore, lit);
    }

    let propagated = (propagate - solver.propagate) as u64;
    solver.propagate = propagate;
    update_probing_propagation_statistics(solver, propagated);
    update_conflicts_and_trail::<true>(solver, conflict, flush);

    crate::profile::stop_checked(solver, Prof::propagate); // STOP (propagate)

    conflict
}
