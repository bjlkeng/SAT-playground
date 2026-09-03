// Port of src/preprocess.c (kissat 4.0.4).
//
// PORT NOTES:
//  - kissat_probe_initially (probe.c) and kissat_fast_variable_elimination
//    (fastel.c) belong to sibling waves; call sites go through
//    crate::probe::probe_initially / crate::fastel::fast_variable_elimination
//    (stubbed in stubs.rs until those waves land).
//  - START/STOP (preprocess) is the level-2 profile.
//  - All #ifndef QUIET verbose accounting is kept (kissat_verbose level 1).

use crate::internal::Solver;
use crate::profile::Prof;
use crate::terminated;
use crate::utilities::percent;

/// Port of `kissat_preprocessing`.
pub fn preprocessing(solver: &mut Solver) -> bool {
    debug_assert!(solver.level == 0);
    debug_assert!(!solver.inconsistent);
    if solver.options.preprocess == 0 {
        return false;
    }
    if solver.options.probe == 0 {
        return false;
    }
    if solver.options.preprocessprobe == 0 {
        return false;
    }
    true
}

/// Port of `kissat_preprocess`.
pub fn preprocess(solver: &mut Solver) -> i32 {
    debug_assert!(preprocessing(solver));
    if !crate::propinitially::initially_propagate(solver) {
        debug_assert!(solver.inconsistent);
        return 20;
    }
    crate::profile::start_checked(solver, Prof::preprocess); // START (preprocess)
    debug_assert!(!solver.preprocessing);
    solver.preprocessing = true;
    crate::report::report(solver, false, '('); // REPORT (0, '(')
    let max_rounds = solver.options.preprocessrounds as u32;
    // #ifndef QUIET:
    let variables_initially = solver.active;
    let clauses_initially = solver.statistics.binirr_clauses(); // BINIRR_CLAUSES
    let variables_originally = solver.import_.len() as u32; // SIZE_STACK (import)
    let clauses_originally = solver.statistics.clauses_original;
    crate::print::verbose(
        solver,
        format!(
            "[preprocess] running at most {} preprocesing rounds",
            max_rounds
        ),
    );
    crate::print::verbose(
        solver,
        format!(
            "[preprocess] initially {} variables {:.0}% and {} clauses {:.0}%",
            variables_initially,
            percent(variables_initially as f64, variables_originally as f64),
            clauses_initially,
            percent(clauses_initially as f64, clauses_originally as f64)
        ),
    );
    // #endif
    crate::collect::initial_sparse_collect(solver);
    let mut round: u32 = 1;
    loop {
        if solver.inconsistent {
            break;
        }
        if terminated!(solver, preprocess_terminated_1) {
            break;
        }
        let variables_before = solver.active;
        let clauses_before = solver.statistics.binirr_clauses();
        crate::print::verbose(
            solver,
            format!(
                "[preprocess-{}] started preprocessing round {}",
                round, round
            ),
        );
        if solver.options.preprocessprobe != 0 {
            crate::probe::probe_initially(solver);
        }
        if solver.options.fastel != 0 {
            crate::fastel::fast_variable_elimination(solver);
        }
        let variables_after = solver.active;
        let clauses_after = solver.statistics.binirr_clauses();
        // #ifndef QUIET:
        if variables_after < variables_before {
            let removed = variables_before - variables_after;
            crate::print::verbose(
                solver,
                format!(
                    "[preprocess-{}] removed {} variables {:.0}% in round {}",
                    round,
                    removed,
                    percent(removed as f64, variables_before as f64),
                    round
                ),
            );
        } else if variables_after > variables_before {
            let added = variables_after - variables_before;
            crate::print::verbose(
                solver,
                format!(
                    "[preprocess-{}] added {} variables {:.0}% in round {}",
                    round,
                    added,
                    percent(added as f64, variables_before as f64),
                    round
                ),
            );
        } else {
            crate::print::verbose(
                solver,
                format!(
                    "[preprocess-{}] number variables {} unchanged in round {}",
                    round, variables_before, round
                ),
            );
        }
        if clauses_after < clauses_before {
            let removed = clauses_before - clauses_after;
            crate::print::verbose(
                solver,
                format!(
                    "[preprocess-{}] removed {} irredundant and binary clauses {:.0}% in round {}",
                    round,
                    removed,
                    percent(removed as f64, clauses_before as f64),
                    round
                ),
            );
        } else if clauses_after > clauses_before {
            let added = clauses_after - clauses_before;
            crate::print::verbose(
                solver,
                format!(
                    "[preprocess-{}] added {} irredundant and binary clauses {:.0}% in round {}",
                    round,
                    added,
                    percent(added as f64, clauses_before as f64),
                    round
                ),
            );
        } else {
            crate::print::verbose(
                solver,
                format!(
                    "[preprocess-{}] number irredundant and binary clauses {} unchanged in round {}",
                    round, clauses_before, round
                ),
            );
        }
        // #endif
        if round == max_rounds {
            break;
        }
        if variables_before == variables_after && clauses_before == clauses_after {
            break;
        }
        round += 1;
    }
    // #ifndef QUIET:
    let variables_finally = solver.active;
    let clauses_finally = solver.statistics.binirr_clauses();
    crate::print::verbose(
        solver,
        format!("[preprocess] finished after {} rounds", round),
    );
    if variables_finally < variables_initially {
        let removed = variables_initially - variables_finally;
        crate::print::verbose(
            solver,
            format!(
                "[preprocess] removed {} variables {:.0}% ({} remain {:.0}%)",
                removed,
                percent(removed as f64, variables_initially as f64),
                variables_finally,
                percent(variables_finally as f64, variables_originally as f64)
            ),
        );
    } else if variables_finally > variables_initially {
        let added = variables_finally - variables_initially;
        crate::print::verbose(
            solver,
            format!(
                "[preprocess] added {} variables {:.0}% ({} remain {:.0}%)",
                added,
                percent(added as f64, variables_initially as f64),
                variables_finally,
                percent(variables_finally as f64, variables_originally as f64)
            ),
        );
    } else {
        crate::print::verbose(
            solver,
            format!(
                "[preprocess] number variables {} unchanged ({} remain {:.0}%)",
                variables_initially,
                variables_finally,
                percent(variables_finally as f64, variables_originally as f64)
            ),
        );
    }
    if clauses_finally < clauses_initially {
        let removed = clauses_initially - clauses_finally;
        crate::print::verbose(
            solver,
            format!(
                "[preprocess] removed {} irredundant and binary clauses {:.0}% ({} remain {:.0}%)",
                removed,
                percent(removed as f64, clauses_initially as f64),
                clauses_finally,
                percent(clauses_finally as f64, clauses_originally as f64)
            ),
        );
    } else if clauses_finally > clauses_initially {
        let added = clauses_finally - clauses_initially;
        crate::print::verbose(
            solver,
            format!(
                "[preprocess] added {} irredundant and binary clauses {:.0}% ({} remain {:.0}%)",
                added,
                percent(added as f64, clauses_initially as f64),
                clauses_finally,
                percent(clauses_finally as f64, clauses_originally as f64)
            ),
        );
    } else {
        crate::print::verbose(
            solver,
            format!(
                "[preprocess] number irredundant and binary clauses {} unchanged ({} remain {:.0}%)",
                clauses_initially,
                clauses_finally,
                percent(clauses_finally as f64, clauses_originally as f64)
            ),
        );
    }
    // #endif
    crate::report::report(solver, false, ')'); // REPORT (0, ')')
    debug_assert!(solver.preprocessing);
    solver.preprocessing = false;
    crate::profile::stop_checked(solver, Prof::preprocess); // STOP (preprocess)
    if solver.inconsistent {
        20
    } else {
        0
    }
}
