// Port of src/decide.c (kissat 4.0.4).
//
// PORT NOTES:
//  - INC (score_decisions), INC (target_decisions), INC (saved_decisions),
//    INC (initial_decisions), INC (stable_decisions), INC (focused_decisions)
//    are METRIC counters: no-ops in the reference build.
//    INC (queue_decisions) / INC (random_decisions) are STATISTIC-tier fields
//    (kept, never printed); decisions / warming_decisions / random_sequences
//    are COUNTERs.
//  - C `const unsigned length = GET_OPTION (randeclength) * LOGN (count);`
//    is int * double -> double -> unsigned truncation, ported exactly.
//  - FORMAT_COUNT arguments of kissat_very_verbose are evaluated
//    unconditionally in C (before the verbosity check inside the callee);
//    the port computes the strings first for the same rotating-buffer usage.

use crate::internal::Solver;
use crate::literal::{lit, not, INVALID_IDX};
use crate::profile::Prof;

// INITIAL_PHASE (decide.h): GET_OPTION (phase) ? 1 : -1.
#[inline]
pub fn initial_phase(solver: &Solver) -> i8 {
    if solver.options.phase != 0 {
        1
    } else {
        -1
    }
}

// static last_enqueued_unassigned_variable
fn last_enqueued_unassigned_variable(solver: &mut Solver) -> u32 {
    debug_assert!(solver.unassigned > 0);
    let mut res = solver.queue.search.idx;
    if solver.values[lit(res) as usize] != 0 {
        loop {
            res = solver.links[res as usize].prev;
            debug_assert!(!crate::queue::disconnected(res));
            if solver.values[lit(res) as usize] == 0 {
                break;
            }
        }
        crate::inlinequeue::update_queue(solver, res);
    }
    res
}

// static largest_score_unassigned_variable
fn largest_score_unassigned_variable(solver: &mut Solver) -> u32 {
    let mut res = crate::heap::max_heap(&solver.scores);
    while solver.values[lit(res) as usize] != 0 {
        crate::heap::pop_max_heap(&mut solver.scores);
        res = crate::heap::max_heap(&solver.scores);
    }
    res
}

/// Port of `kissat_start_random_sequence`.
pub fn start_random_sequence(solver: &mut Solver) {
    if solver.options.randec == 0 {
        return;
    }

    if solver.stable && solver.options.randecstable == 0 {
        return;
    }

    if !solver.stable && solver.options.randecfocused == 0 {
        return;
    }

    if solver.randec != 0 {
        let conflicts =
            crate::format::format_count(&mut solver.format, solver.statistics.conflicts);
        crate::print::very_verbose(
            solver,
            format!(
                "continuing random decision sequence at {} conflicts",
                conflicts
            ),
        );
    } else {
        solver.statistics.random_sequences += 1; // INC (random_sequences)
        let count = solver.statistics.random_sequences;
        let length = (solver.options.randeclength as f64 * crate::kimits::logn(count)) as u32;
        let conflicts_str =
            crate::format::format_count(&mut solver.format, solver.statistics.conflicts);
        let length_str = crate::format::format_count(&mut solver.format, length as u64);
        crate::print::very_verbose(
            solver,
            format!(
                "starting random decision sequence at {} conflicts for {} conflicts",
                conflicts_str, length_str
            ),
        );
        solver.randec = length;

        crate::update_conflict_limit!(
            solver,
            randec,
            randecint,
            random_sequences,
            |n| crate::kimits::logn(n),
            false
        );
    }
}

// static next_random_decision
fn next_random_decision(solver: &mut Solver) -> u32 {
    if solver.vars == 0 {
        return INVALID_IDX;
    }

    if solver.warming {
        return INVALID_IDX;
    }

    if solver.options.randec == 0 {
        return INVALID_IDX;
    }

    if solver.stable && solver.options.randecstable == 0 {
        return INVALID_IDX;
    }

    if !solver.stable && solver.options.randecfocused == 0 {
        return INVALID_IDX;
    }

    if solver.randec == 0 {
        debug_assert!(solver.level > 0);
        if solver.level > 1 {
            return INVALID_IDX;
        }

        let conflicts = solver.statistics.conflicts;
        if conflicts < solver.limits.randec.conflicts {
            return INVALID_IDX;
        }

        start_random_sequence(solver);
    }

    loop {
        let idx = crate::random::next_random32(&mut solver.random) % solver.vars;
        if !solver.flags[idx as usize].active {
            continue;
        }
        let l = lit(idx);
        if solver.values[l as usize] != 0 {
            continue;
        }
        return idx;
    }
}

/// Port of `kissat_next_decision_variable`.
pub fn next_decision_variable(solver: &mut Solver) -> u32 {
    let mut res = next_random_decision(solver);
    if res == INVALID_IDX {
        if solver.stable {
            res = largest_score_unassigned_variable(solver);
            // INC (score_decisions): METRIC, no-op.
        } else {
            res = last_enqueued_unassigned_variable(solver);
            solver.statistics.queue_decisions += 1; // INC (queue_decisions)
        }
    } else {
        solver.statistics.random_decisions += 1; // INC (random_decisions)
    }
    res
}

/// Port of `kissat_decide_phase`.
pub fn decide_phase(solver: &mut Solver, idx: u32) -> i32 {
    let force = solver.options.forcephase != 0;

    let use_target = if force {
        false
    } else if solver.options.target == 0 {
        false
    } else {
        solver.stable || solver.options.target > 1
    };

    let use_saved = if force {
        false
    } else {
        solver.options.phasesaving != 0
    };

    let mut res: i8 = 0;

    if !solver.stable {
        match (solver.statistics.switched >> 1) & 7 {
            1 => res = initial_phase(solver),
            3 => res = -initial_phase(solver),
            _ => {}
        }
    }

    if res == 0 && use_target {
        res = solver.phases.target[idx as usize];
        // if (res) INC (target_decisions): METRIC, no-op.
    }

    if res == 0 && use_saved {
        res = solver.phases.saved[idx as usize];
        // if (res) INC (saved_decisions): METRIC, no-op.
    }

    if res == 0 {
        res = initial_phase(solver);
        // INC (initial_decisions): METRIC, no-op.
    }
    debug_assert!(res != 0);

    if res < 0 {
        -1
    } else {
        1
    }
}

/// Port of `kissat_decide`.
pub fn decide(solver: &mut Solver) {
    crate::profile::start_checked(solver, Prof::decide); // START (decide)
    debug_assert!(solver.unassigned > 0);
    if solver.warming {
        solver.statistics.warming_decisions += 1; // INC (warming_decisions)
    } else {
        solver.statistics.decisions += 1; // INC (decisions)
        // INC (stable_decisions) / INC (focused_decisions): METRIC, no-op.
    }
    solver.level += 1;
    debug_assert!(solver.level != crate::internal::INVALID_LEVEL);
    let idx = next_decision_variable(solver);
    let value = decide_phase(solver, idx);
    let mut l = lit(idx);
    if value < 0 {
        l = not(l);
    }
    crate::frames::push_frame(solver, l);
    debug_assert!((solver.level as usize) < solver.frames.len());
    crate::assign::assign_decision(solver, l);
    crate::profile::stop_checked(solver, Prof::decide); // STOP (decide)
}

/// Port of `kissat_internal_assume`.
pub fn internal_assume(solver: &mut Solver, lit_: u32) {
    debug_assert!(solver.unassigned > 0);
    debug_assert!(solver.values[lit_ as usize] == 0);
    solver.level += 1;
    debug_assert!(solver.level != crate::internal::INVALID_LEVEL);
    crate::frames::push_frame(solver, lit_);
    debug_assert!((solver.level as usize) < solver.frames.len());
    crate::assign::assign_decision(solver, lit_);
}
