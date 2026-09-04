// Port of src/warmup.c (kissat 4.0.4).

use crate::internal::Solver;
use crate::profile::Prof;
use crate::terminated;

/// Port of `kissat_warmup`.
pub fn warmup(solver: &mut Solver) {
    debug_assert!(solver.level == 0);
    debug_assert!(solver.watching);
    debug_assert!(!solver.inconsistent);
    debug_assert!(solver.options.warmup != 0);
    crate::profile::start_checked(solver, Prof::warmup); // START (warmup)
    debug_assert!(!solver.warming);
    solver.warming = true;
    solver.statistics.warmups += 1; // INC (warmups)
    // #ifndef QUIET — kept:
    let mut propagations = solver.statistics.warming_propagations;
    let mut decisions = solver.statistics.warming_decisions;
    while solver.unassigned != 0 {
        if terminated!(solver, warmup_terminated_1) {
            break;
        }
        crate::decide::decide(solver);
        crate::propbeyond::propagate_beyond_conflicts(solver);
    }
    debug_assert!(!solver.inconsistent);
    // #ifndef QUIET — kept:
    decisions = solver.statistics.warming_decisions - decisions;
    propagations = solver.statistics.warming_propagations - propagations;

    crate::print::very_verbose(
        solver,
        format_args!(
            "warming-up needed {} decisions and {} propagations",
            decisions, propagations
        ),
    );

    let level = solver.level;
    if solver.unassigned != 0 {
        crate::print::verbose(
            solver,
            format_args!(
                "reached decision level {} during warming-up saved phases",
                level
            ),
        );
    } else {
        crate::print::verbose(
            solver,
            format_args!(
                "all variables assigned at decision level {} during warming-up saved phases",
                level
            ),
        );
    }
    crate::backtrack::backtrack_without_updating_phases(solver, 0);
    debug_assert!(solver.warming);
    solver.warming = false;
    crate::profile::stop_checked(solver, Prof::warmup); // STOP (warmup)
}
