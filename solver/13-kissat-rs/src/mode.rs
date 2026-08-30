// Port of src/mode.h + src/mode.c (kissat 4.0.4).
//
// QUIET is not defined, so the `entered` / `conflicts` bookkeeping fields and
// the verbose reporting are kept; METRICS fields are omitted.

use crate::internal::Solver;

/// Port of `struct mode` (mode.h).
#[derive(Clone, Copy, Default)]
pub struct Mode {
    pub ticks: u64,
    // #ifndef QUIET — kept:
    pub entered: f64,
    pub conflicts: u64,
    // METRICS-only propagations/visits omitted.
}

fn mode_string(solver: &Solver) -> &'static str {
    if solver.stable {
        "stable"
    } else {
        "focused"
    }
}

/// Port of `kissat_init_mode_limit`.
pub fn init_mode_limit(solver: &mut Solver) {
    if solver.options.stable == 1 {
        debug_assert!(!solver.stable);

        let conflicts_delta = solver.options.modeinit as u64;
        let conflicts_limit = solver.statistics.conflicts + conflicts_delta;

        debug_assert!(conflicts_limit != 0);

        solver.limits.mode.conflicts = conflicts_limit;
        solver.limits.mode.ticks = 0;
        solver.limits.mode.count = 0;

        crate::print::very_verbose(
            solver,
            &format!(
                "initial {} mode switching limit at {} after {} conflicts",
                mode_string(solver),
                conflicts_limit,
                conflicts_delta
            ),
        );

        solver.mode.ticks = solver.statistics.search_ticks;
        // #ifndef QUIET:
        solver.mode.conflicts = solver.statistics.conflicts;
        solver.mode.entered = crate::resources::process_time();
        crate::print::very_verbose(
            solver,
            &format!(
                "starting {} mode at {:.2} seconds ({} conflicts, {} ticks)",
                mode_string(solver),
                solver.mode.entered,
                solver.mode.conflicts,
                solver.mode.ticks
            ),
        );
    } else {
        crate::print::very_verbose(
            solver,
            &format!(
                "no need to set mode limit (only {} mode enabled)",
                mode_string(solver)
            ),
        );
    }
}

/// Port of static `update_mode_limit`.
fn update_mode_limit(solver: &mut Solver, delta_ticks: u64) {
    // kissat_init_averages (solver, &AVERAGES);  AVERAGES = averages[stable]
    // PORT NOTE: sibling API guessed as index-based to satisfy the borrow
    // checker (C passes a pointer into solver->averages).
    crate::averages::init_averages(solver, solver.stable as usize);

    debug_assert!(solver.options.stable == 1);

    if solver.limits.mode.count & 1 != 0 {
        solver.limits.mode.ticks = solver.statistics.search_ticks + delta_ticks;
        debug_assert!(solver.stable);
        let limit = solver.limits.mode.ticks;
        // GET (stable_modes) on a METRIC counter yields UINT64_MAX in the
        // reference (non-METRICS) build, which suppresses the phase count.
        let count = u64::MAX;
        crate::print::phase(
            solver,
            "stable",
            count,
            &format!(
                "new stable mode switching limit of {} after {} ticks",
                limit, delta_ticks
            ),
        );
    } else {
        debug_assert!(solver.limits.mode.ticks != 0);
        let interval = solver.options.modeint as u64;
        let count = (solver.statistics.switched + 1) / 2;
        // C: uint64_t scaled = interval * kissat_nlogpown (count, 4);
        // (double product truncated back to uint64_t)
        let scaled = (interval as f64 * crate::kimits::nlogpown(count, 4)) as u64;
        solver.limits.mode.conflicts = solver.statistics.conflicts + scaled;
        debug_assert!(!solver.stable);
        let limit = solver.limits.mode.conflicts;
        // GET (focused_modes): METRIC → UINT64_MAX in the reference build.
        let count = u64::MAX;
        crate::print::phase(
            solver,
            "focused",
            count,
            &format!(
                "new focused mode switching limit of {} after {} conflicts",
                limit, scaled
            ),
        );
    }

    solver.mode.ticks = solver.statistics.search_ticks;
    // #ifndef QUIET:
    solver.mode.conflicts = solver.statistics.conflicts;
}

/// Port of static `report_switching_from_mode`; returns `delta_ticks`
/// (C uses an out-parameter).
fn report_switching_from_mode(solver: &mut Solver) -> u64 {
    let delta_ticks = solver.statistics.search_ticks - solver.mode.ticks;

    // #ifndef QUIET:
    if crate::print::verbosity(solver) < 2 {
        return delta_ticks;
    }

    let current_time = crate::resources::process_time();
    let delta_time = current_time - solver.mode.entered;

    let delta_conflicts = solver.statistics.conflicts - solver.mode.conflicts;
    // PORT NOTE (quirk kept): `mode.entered` is only refreshed when
    // verbosity >= 2, exactly as in C where the store sits after the early
    // verbosity return.
    solver.mode.entered = current_time;

    let stable = if solver.stable { "stable" } else { "focused" };
    crate::print::very_verbose(
        solver,
        &format!(
            "{} mode took {:.2} seconds ({} conflicts, {} ticks)",
            stable, delta_time, delta_conflicts, delta_ticks
        ),
    );

    delta_ticks
}

/// Port of static `switch_to_focused_mode`.
fn switch_to_focused_mode(solver: &mut Solver) {
    debug_assert!(solver.stable);
    let delta = report_switching_from_mode(solver);
    crate::report::report(solver, false, ']'); // REPORT (0, ']')
    crate::profile::stop(solver, crate::profile::Prof::stable); // STOP (stable)
    // INC (focused_modes) is METRIC-only: no-op in the reference build.
    // GET (focused_modes): METRIC → UINT64_MAX in the reference build.
        let count = u64::MAX;
    let conflicts = solver.statistics.conflicts;
    crate::print::phase(
        solver,
        "focus",
        count,
        &format!("switching to focused mode after {} conflicts", conflicts),
    );
    solver.stable = false;
    update_mode_limit(solver, delta);
    crate::profile::start(solver, crate::profile::Prof::focused); // START (focused)
    crate::report::report(solver, false, '{'); // REPORT (0, '{')
    crate::queue::reset_search_of_queue(solver);
    crate::restart::update_focused_restart_limit(solver);
}

/// Port of static `switch_to_stable_mode`.
fn switch_to_stable_mode(solver: &mut Solver) {
    debug_assert!(!solver.stable);
    let delta = report_switching_from_mode(solver);
    crate::report::report(solver, false, '}'); // REPORT (0, '}')
    crate::profile::stop(solver, crate::profile::Prof::focused); // STOP (focused)
    // INC (stable_modes) is METRIC-only: no-op in the reference build.
    solver.stable = true;
    // GET (stable_modes) on a METRIC counter yields UINT64_MAX in the
        // reference (non-METRICS) build, which suppresses the phase count.
        let count = u64::MAX;
    let conflicts = solver.statistics.conflicts;
    crate::print::phase(
        solver,
        "stable",
        count,
        &format!("switched to stable mode after {} conflicts", conflicts),
    );
    update_mode_limit(solver, delta);
    crate::profile::start(solver, crate::profile::Prof::stable); // START (stable)
    crate::report::report(solver, false, '['); // REPORT (0, '[')
    crate::reluctant::init_reluctant(solver);
    crate::bump::update_scores(solver);
}

/// Port of `kissat_switching_search_mode`.
pub fn switching_search_mode(solver: &Solver) -> bool {
    debug_assert!(!solver.inconsistent);

    if solver.options.stable != 1 {
        return false;
    }

    if solver.limits.mode.count & 1 != 0 {
        solver.statistics.search_ticks >= solver.limits.mode.ticks
    } else {
        solver.statistics.conflicts >= solver.limits.mode.conflicts
    }
}

/// Port of `kissat_switch_search_mode`.
pub fn switch_search_mode(solver: &mut Solver) {
    debug_assert!(switching_search_mode(solver));

    solver.statistics.switched += 1; // INC (switched)
    solver.limits.mode.count += 1;

    if solver.stable {
        switch_to_focused_mode(solver);
    } else {
        switch_to_stable_mode(solver);
    }

    solver.averages[solver.stable as usize].saved_decisions = solver.statistics.decisions;

    crate::decide::start_random_sequence(solver);
}
