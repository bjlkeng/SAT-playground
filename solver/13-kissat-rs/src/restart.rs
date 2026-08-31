// Port of src/restart.c (kissat 4.0.4).
//
// PORT NOTES:
//  - INC (stable_restarts) / INC (focused_restarts) are METRIC counters:
//    no-ops in the reference build.  restarts is a COUNTER;
//    restarts_levels / restarts_reused_trails / restarts_reused_levels are
//    STATISTIC-tier fields (kept, never printed).
//  - C `delta += kissat_logn (restarts) - 1;` promotes delta to double for
//    the addition and truncates back to uint64_t — ported exactly.
//  - C's %g float formatting in the extremely-verbose message is
//    approximated with Rust `{}` Display (message-only, verbosity >= 3).

use crate::internal::Solver;
use crate::profile::Prof;

/// Port of `kissat_restarting`.
pub fn restarting(solver: &mut Solver) -> bool {
    debug_assert!(solver.unassigned > 0);
    if solver.options.restart == 0 {
        return false;
    }
    if solver.level == 0 {
        return false;
    }
    if solver.statistics.conflicts < solver.limits.restart.conflicts {
        return false;
    }
    if solver.stable {
        return crate::reluctant::reluctant_triggered(&mut solver.reluctant);
    }
    let averages = &solver.averages[solver.stable as usize];
    let fast = averages.fast_glue.value; // AVERAGE (fast_glue)
    let slow = averages.slow_glue.value; // AVERAGE (slow_glue)
    let margin = (100.0 + solver.options.restartmargin as f64) / 100.0;
    let limit = margin * slow;
    crate::print::extremely_verbose(
        solver,
        format!(
            "restart glue limit {} = {:.2} * {} (slow glue) {} {} (fast glue)",
            limit,
            margin,
            slow,
            if limit > fast {
                '>'
            } else if limit == fast {
                '='
            } else {
                '<'
            },
            fast
        ),
    );
    limit <= fast
}

/// Port of `kissat_update_focused_restart_limit`.
pub fn update_focused_restart_limit(solver: &mut Solver) {
    debug_assert!(!solver.stable);
    let restarts = solver.statistics.restarts;
    let mut delta: u64 = solver.options.restartint as u64;
    if restarts != 0 {
        // delta += kissat_logn (restarts) - 1;  (double arithmetic, truncated)
        delta = (delta as f64 + (crate::kimits::logn(restarts) - 1.0)) as u64;
    }
    solver.limits.restart.conflicts = solver.statistics.conflicts + delta;
    crate::print::extremely_verbose(
        solver,
        format!(
            "focused restart limit at {} after {} conflicts ",
            solver.limits.restart.conflicts, delta
        ),
    );
}

// static reuse_stable_trail
fn reuse_stable_trail(solver: &mut Solver) -> u32 {
    let next_idx = crate::decide::next_decision_variable(solver);
    let limit = crate::heap::get_heap_score(&solver.scores, next_idx);
    let level = solver.level;
    let mut res: u32 = 0;
    while res < level {
        let decision = solver.frames[(res + 1) as usize].decision; // FRAME (res + 1)
        let idx = crate::literal::idx(decision);
        let score = crate::heap::get_heap_score(&solver.scores, idx);
        if score <= limit {
            break;
        }
        res += 1;
    }
    res
}

// static reuse_focused_trail
fn reuse_focused_trail(solver: &mut Solver) -> u32 {
    let next_idx = crate::decide::next_decision_variable(solver);
    let limit = solver.links[next_idx as usize].stamp;
    let level = solver.level;
    let mut res: u32 = 0;
    while res < level {
        let decision = solver.frames[(res + 1) as usize].decision; // FRAME (res + 1)
        let idx = crate::literal::idx(decision);
        let score = solver.links[idx as usize].stamp;
        if score <= limit {
            break;
        }
        res += 1;
    }
    res
}

// static reuse_trail
fn reuse_trail(solver: &mut Solver) -> u32 {
    debug_assert!(solver.level > 0);
    debug_assert!(!solver.trail.is_empty());

    if solver.options.restartreusetrail == 0 {
        return 0;
    }

    let res = if solver.stable {
        reuse_stable_trail(solver)
    } else {
        reuse_focused_trail(solver)
    };

    if res != 0 {
        solver.statistics.restarts_reused_trails += 1; // INC
        solver.statistics.restarts_reused_levels += res as u64; // ADD
    }

    res
}

/// Port of `kissat_restart`.
pub fn restart(solver: &mut Solver) {
    crate::profile::start_checked(solver, Prof::restart); // START (restart)
    solver.statistics.restarts += 1; // INC (restarts)
    solver.statistics.restarts_levels += solver.level as u64; // ADD (restarts_levels)
    // INC (stable_restarts) / INC (focused_restarts): METRIC, no-op.
    let level = reuse_trail(solver);
    crate::print::extremely_verbose(
        solver,
        format!(
            "restarting after {} conflicts (limit {})",
            solver.statistics.conflicts, solver.limits.restart.conflicts
        ),
    );
    crate::backtrack::backtrack_in_consistent_state(solver, level);
    if !solver.stable {
        update_focused_restart_limit(solver);
    }
    crate::report::report(solver, true, 'R'); // REPORT (1, 'R')
    crate::profile::stop_checked(solver, Prof::restart); // STOP (restart)
}
