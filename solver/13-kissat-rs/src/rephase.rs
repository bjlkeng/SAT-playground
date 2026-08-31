// Port of src/rephase.c (kissat 4.0.4).
//
// PORT NOTE: `kissat_reset_best_assigned` / `kissat_reset_target_assigned`
// are *static* in rephase.c despite the `kissat_` prefix; they keep their
// names (minus prefix) as private fns here.
// PORT NOTE: INC (rephased_best/_inverted/_original/_walking) are METRIC
// counters — no-ops in the reference (non-METRICS) build.
// PORT NOTE: `kissat_walking`/`kissat_walk` live in the walk wave
// (crate::walk, stubbed in stubs.rs until it lands).

use crate::internal::Solver;
use crate::profile::Prof;
use crate::update_conflict_limit;

// static void kissat_reset_best_assigned (kissat *solver)
fn reset_best_assigned(solver: &mut Solver) {
    if solver.best_assigned == 0 {
        return;
    }
    let best_assigned = solver.best_assigned;
    crate::print::extremely_verbose(
        solver,
        format!(
            "resetting best assigned trail height {} to 0",
            best_assigned
        ),
    );
    solver.best_assigned = 0;
}

// static void kissat_reset_target_assigned (kissat *solver)
fn reset_target_assigned(solver: &mut Solver) {
    if solver.target_assigned == 0 {
        return;
    }
    let target_assigned = solver.target_assigned;
    crate::print::extremely_verbose(
        solver,
        format!(
            "resetting target assigned trail height {} to 0",
            target_assigned
        ),
    );
    solver.target_assigned = 0;
}

/// Port of `kissat_rephasing`.
pub fn rephasing(solver: &Solver) -> bool {
    if solver.options.rephase == 0 {
        return false;
    }
    if !solver.stable {
        return false;
    }
    solver.statistics.conflicts > solver.limits.rephase.conflicts
}

// static char rephase_best (kissat *solver)
fn rephase_best(solver: &mut Solver) -> char {
    let vars = solver.vars as usize;
    // for (s = saved, b = best; b != end_of_best; s++, b++)
    //   if ((tmp = *b)) *s = tmp;
    for i in 0..vars {
        let tmp = solver.phases.best[i];
        if tmp != 0 {
            solver.phases.saved[i] = tmp;
        }
    }
    // INC (rephased_best) — METRIC, no-op.
    'B'
}

// static char rephase_original (kissat *solver)
fn rephase_original(solver: &mut Solver) -> char {
    // INITIAL_PHASE (decide.h): GET_OPTION (phase) ? 1 : -1
    let initial_phase: i8 = if solver.options.phase != 0 { 1 } else { -1 };
    let vars = solver.vars as usize;
    for s in solver.phases.saved[..vars].iter_mut() {
        *s = initial_phase;
    }
    // INC (rephased_original) — METRIC, no-op.
    'O'
}

// static char rephase_inverted (kissat *solver)
fn rephase_inverted(solver: &mut Solver) -> char {
    let inverted_initial_phase: i8 = if solver.options.phase != 0 { -1 } else { 1 };
    let vars = solver.vars as usize;
    for s in solver.phases.saved[..vars].iter_mut() {
        *s = inverted_initial_phase;
    }
    // INC (rephased_inverted) — METRIC, no-op.
    'I'
}

// static char rephase_walking (kissat *solver)
fn rephase_walking(solver: &mut Solver) -> char {
    debug_assert!(crate::walk::walking(solver));
    crate::profile::stop_checked(solver, Prof::rephase); // STOP (rephase)
    crate::walk::walk(solver);
    crate::profile::start_checked(solver, Prof::rephase); // START (rephase)
    // INC (rephased_walking) — METRIC, no-op.
    'W'
}

// static char (*rephase_schedule[]) (kissat *)
const REPHASE_SCHEDULE: [fn(&mut Solver) -> char; 6] = [
    rephase_best,
    rephase_walking,
    rephase_inverted,
    rephase_best,
    rephase_walking,
    rephase_original,
];

// #ifndef QUIET — kept:
fn rephase_type_as_string(type_: char) -> &'static str {
    if type_ == 'B' {
        return "best";
    }
    if type_ == 'I' {
        return "inverted";
    }
    if type_ == 'O' {
        return "original";
    }
    debug_assert!(type_ == 'W');
    "walking"
}

// static char reset_phases (kissat *solver)
fn reset_phases(solver: &mut Solver) -> char {
    let count = solver.statistics.rephased; // GET (rephased)
    debug_assert!(count > 0);
    let select = ((count - 1) % REPHASE_SCHEDULE.len() as u64) as usize;
    let type_ = REPHASE_SCHEDULE[select](solver);
    let rephased = solver.statistics.rephased;
    crate::print::phase(
        solver,
        "rephase",
        rephased,
        format!(
            "{} phases in {} search mode",
            rephase_type_as_string(type_),
            if solver.stable { "stable" } else { "focused" }
        ),
    );
    // memcpy (solver->phases.target, solver->phases.saved, VARS)
    let vars = solver.vars as usize;
    solver.phases.target[..vars].copy_from_slice(&solver.phases.saved[..vars]);
    // UPDATE_CONFLICT_LIMIT (rephase, rephased, NLOG3N, false)
    update_conflict_limit!(
        solver,
        rephase,
        rephaseint,
        rephased,
        |n| crate::kimits::nlogpown(n, 3),
        false
    );
    reset_target_assigned(solver);
    if type_ == 'B' {
        reset_best_assigned(solver);
    }
    type_
}

/// Port of `kissat_rephase`.
pub fn rephase(solver: &mut Solver) {
    crate::backtrack::backtrack_propagate_and_flush_trail(solver);
    debug_assert!(!solver.inconsistent);
    crate::profile::start_checked(solver, Prof::rephase); // START (rephase)
    solver.statistics.rephased += 1; // INC (rephased)
    let type_ = reset_phases(solver);
    crate::report::report(solver, false, type_); // REPORT (0, type)
    crate::profile::stop_checked(solver, Prof::rephase); // STOP (rephase)
}
