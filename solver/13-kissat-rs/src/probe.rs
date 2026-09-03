// Port of src/probe.c (kissat 4.0.4).
//
// Driver of the probing pipeline: congruence / substitute / backbone /
// vivify / sweep / transitive / factor sequencing, the simplifier profile
// transitions and the probe conflict-limit update.
//
// PORT NOTES:
//  - The C static functions `probe` and `probe_initially` collide with the
//    public kissat_probe / kissat_probe_initially after kissat_ prefix
//    dropping; the statics are named probe_round / probe_initially_round
//    here (bodies verbatim).
//  - congruence.c / substitute.c / sweep.c / factor.c are ported by
//    concurrent waves; until they land, marked stubs live in src/stubs.rs
//    (crate::congruence::congruence, crate::substitute::substitute,
//    crate::sweep::sweep, crate::factor::factor).
//  - UPDATE_CONFLICT_LIMIT (probe, probings, NLOGN, true): NLOGN (COUNT) is
//    kissat_nlogpown (COUNT, 1) per kimits.h.

use crate::internal::Solver;
use crate::profile::Prof;

/// Port of `kissat_probing`.
pub fn probing(solver: &mut Solver) -> bool {
    if !solver.enabled.probe {
        return false;
    }
    let conflicts = solver.statistics.conflicts;
    if solver.last.conflicts.reduce == conflicts {
        return false;
    }
    solver.limits.probe.conflicts < conflicts
}

// static probe (renamed, see PORT NOTES).
fn probe_round(solver: &mut Solver) {
    crate::backtrack::backtrack_propagate_and_flush_trail(solver);
    debug_assert!(!solver.inconsistent);
    // STOP_SEARCH_AND_START_SIMPLIFIER (probe)
    crate::profile::stop_search_and_start_simplifier_checked(solver, Prof::probe);
    crate::print::phase(
        solver,
        "probe",
        solver.statistics.probings, // GET (probings)
        format!(
            "probing limit hit after {} conflicts",
            solver.limits.probe.conflicts
        ),
    );
    let _ = crate::congruence::congruence(solver);
    crate::substitute::substitute(solver, false);
    crate::backbone::binary_clauses_backbone(solver);
    crate::vivify::vivify(solver);
    let _ = crate::sweep::sweep(solver);
    crate::substitute::substitute(solver, false);
    crate::transitive::transitive_reduction(solver);
    crate::backbone::binary_clauses_backbone(solver);
    crate::factor::factor(solver);
    // STOP_SIMPLIFIER_AND_RESUME_SEARCH (probe)
    crate::profile::stop_simplifier_and_resume_search_checked(solver, Prof::probe);
}

// static probe_initially (renamed, see PORT NOTES).
fn probe_initially_round(solver: &mut Solver) {
    debug_assert!(solver.level == 0);
    debug_assert!(!solver.inconsistent);
    crate::print::phase(
        solver,
        "probe",
        solver.statistics.probings, // GET (probings)
        "initial probing",
    );
    let mut substitute_at_the_end = true;
    if solver.options.preprocesscongruence != 0 {
        if crate::congruence::congruence(solver) {
            crate::substitute::substitute(solver, true);
            substitute_at_the_end = false;
        }
    }
    if solver.options.preprocessbackbone != 0 {
        crate::backbone::binary_clauses_backbone(solver);
    }
    if solver.options.preprocessweep != 0 {
        if crate::sweep::sweep(solver) {
            crate::substitute::substitute(solver, true);
            substitute_at_the_end = false;
        }
    }
    if substitute_at_the_end {
        crate::substitute::substitute(solver, false);
    }
    if solver.options.preprocessfactor != 0 {
        crate::factor::factor(solver);
    }
}

/// Port of `kissat_probe`.
pub fn probe(solver: &mut Solver) -> i32 {
    debug_assert!(!solver.inconsistent);
    solver.statistics.probings += 1; // INC (probings)
    debug_assert!(!solver.probing);
    solver.probing = true;
    let max_rounds = solver.options.proberounds as u32;
    for _round in 0..max_rounds {
        let before = solver.active;
        probe_round(solver);
        if solver.inconsistent {
            break;
        }
        if before == solver.active {
            break;
        }
    }
    crate::classify::classify(solver);
    // UPDATE_CONFLICT_LIMIT (probe, probings, NLOGN, true)
    crate::update_conflict_limit!(
        solver,
        probe,
        probeint,
        probings,
        |n| crate::kimits::nlogpown(n, 1),
        true
    );
    solver.last.ticks.probe = solver.statistics.search_ticks;
    debug_assert!(solver.probing);
    solver.probing = false;
    if solver.inconsistent {
        20
    } else {
        0
    }
}

/// Port of `kissat_probe_initially`.
pub fn probe_initially(solver: &mut Solver) -> i32 {
    debug_assert!(solver.level == 0);
    debug_assert!(!solver.inconsistent);
    solver.statistics.probings += 1; // INC (probings)
    crate::profile::start_checked(solver, Prof::probe); // START (probe)
    debug_assert!(!solver.probing);
    solver.probing = true;
    probe_initially_round(solver);
    debug_assert!(solver.probing);
    solver.probing = false;
    crate::profile::stop_checked(solver, Prof::probe); // STOP (probe)
    if solver.inconsistent {
        20
    } else {
        0
    }
}
