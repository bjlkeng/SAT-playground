// Port of src/search.c (kissat 4.0.4).
//
// PORT NOTE: C `kissat_search_propagate` returns `clause *` (0 = no
// conflict); the landed crate convention is `Option<propsearch::Conflict>`
// (`Conflict::Binary` = C's `&solver->conflict` fake header,
// `Conflict::Clause(ref)` = an arena clause).  `kissat_analyze` takes that
// `Conflict` value.
// PORT NOTE: `kissat_propagated` is a static inline in inline.h with no
// module of its own; ported here as the private `propagated` helper
// (solver.propagate cursor == trail length, exactly END_ARRAY semantics).

use crate::internal::Solver;
use crate::profile::Prof;
use crate::terminated;

// static void init_tiers (kissat *solver)
fn init_tiers(solver: &mut Solver) {
    for stable in 0..2usize {
        if solver.tier1[stable] == 0 {
            debug_assert!(solver.tier2[stable] == 0);
            solver.tier1[stable] = solver.options.tier1 as u32;
            solver.tier2[stable] = solver.options.tier2 as u32;
            if solver.tier2[stable] <= solver.tier1[stable] {
                solver.tier2[stable] = solver.tier1[stable];
            }
            if solver.limits.glue.interval == 0 {
                solver.limits.glue.interval = 2;
            }
        }
    }
}

// static void start_search (kissat *solver)
fn start_search(solver: &mut Solver) {
    crate::profile::start_checked(solver, Prof::search); // START (search)
    solver.statistics.searches += 1; // INC (searches)

    let stable = solver.options.stable == 2;

    solver.stable = stable;
    let searches = solver.statistics.searches;
    let conflicts = solver.statistics.conflicts;
    crate::print::phase(
        solver,
        "search",
        searches,
        format_args!(
            "initializing {} search after {} conflicts",
            if stable { "stable" } else { "focus" },
            conflicts
        ),
    );

    // kissat_init_averages (solver, &AVERAGES)
    crate::averages::init_averages(solver, solver.stable as usize);

    crate::classify::classify(solver);

    if solver.stable {
        crate::reluctant::init_reluctant(solver);
        crate::bump::update_scores(solver);
    }

    init_tiers(solver);

    crate::kimits::init_limits(solver);

    let seed = solver.options.seed as u32; // unsigned seed = GET_OPTION (seed)
    solver.random = seed as u64;

    // #ifndef QUIET — kept:
    let limited_conflicts = solver.limited.conflicts;
    let limited_decisions = solver.limited.decisions;
    let conflicts_limit = solver.limits.conflicts;
    let decisions_limit = solver.limits.decisions;
    if !limited_conflicts && !limited_decisions {
        crate::print::very_verbose(solver, "starting unlimited search");
    } else if limited_conflicts && !limited_decisions {
        crate::print::very_verbose(
            solver,
            format_args!(
                "starting search with conflicts limited to {}",
                conflicts_limit
            ),
        );
    } else if !limited_conflicts && limited_decisions {
        crate::print::very_verbose(
            solver,
            format_args!(
                "starting search with decisions limited to {}",
                decisions_limit
            ),
        );
    } else {
        crate::print::very_verbose(
            solver,
            format_args!(
                "starting search with decisions limited to {} and conflicts limited to {}",
                decisions_limit, conflicts_limit
            ),
        );
    }
    if stable {
        crate::profile::start_checked(solver, Prof::stable); // START (stable)
        crate::report::report(solver, false, '['); // REPORT (0, '[')
    } else {
        crate::profile::start_checked(solver, Prof::focused); // START (focused)
        crate::report::report(solver, false, '{'); // REPORT (0, '{')
    }
}

// static void stop_search (kissat *solver)
fn stop_search(solver: &mut Solver) {
    if solver.limited.conflicts {
        solver.limited.conflicts = false;
    }

    if solver.limited.decisions {
        solver.limited.decisions = false;
    }

    if solver
        .termination
        .flagged
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        crate::print::very_verbose(solver, "termination forced externally");
        solver
            .termination
            .flagged
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    if solver.stable {
        crate::report::report(solver, false, ']'); // REPORT (0, ']')
        crate::profile::stop_checked(solver, Prof::stable); // STOP (stable)
        solver.stable = false;
    } else {
        crate::report::report(solver, false, '}'); // REPORT (0, '}')
        crate::profile::stop_checked(solver, Prof::focused); // STOP (focused)
    }
    crate::profile::stop_checked(solver, Prof::search); // STOP (search)
}

// static void report_search_result (kissat *solver, int res)
fn report_search_result(solver: &mut Solver, res: i32) {
    // #ifndef QUIET — kept:
    let type_ = if res == 10 {
        '1'
    } else if res == 20 {
        '0'
    } else {
        '?'
    };
    crate::report::report(solver, false, type_); // REPORT (0, type)
}

// static void iterate (kissat *solver)
fn iterate(solver: &mut Solver) {
    debug_assert!(solver.iterating);
    solver.iterating = false;
    crate::report::report(solver, false, 'i'); // REPORT (0, 'i')
}

// static bool conflict_limit_hit (kissat *solver)
fn conflict_limit_hit(solver: &mut Solver) -> bool {
    if !solver.limited.conflicts {
        return false;
    }
    if solver.limits.conflicts > solver.statistics.conflicts {
        return false;
    }
    let limit = solver.limits.conflicts;
    let conflicts = solver.statistics.conflicts;
    crate::print::very_verbose(
        solver,
        format_args!("conflict limit {} hit after {} conflicts", limit, conflicts),
    );
    true
}

// static bool decision_limit_hit (kissat *solver)
fn decision_limit_hit(solver: &mut Solver) -> bool {
    if !solver.limited.decisions {
        return false;
    }
    if solver.limits.decisions > solver.statistics.decisions {
        return false;
    }
    let limit = solver.limits.decisions;
    let decisions = solver.statistics.decisions;
    crate::print::very_verbose(
        solver,
        format_args!("decision limit {} hit after {} decisions", limit, decisions),
    );
    true
}

// kissat_propagated (inline.h) — see PORT NOTE at the top.
#[inline]
fn propagated(solver: &Solver) -> bool {
    debug_assert!(solver.propagate <= solver.trail.len());
    solver.propagate == solver.trail.len()
}

// static bool searching (kissat *solver)
fn searching(solver: &mut Solver) -> bool {
    if !propagated(solver) {
        return true;
    }
    if solver.options.probeinit == 0 {
        return true;
    }
    if solver.options.eliminateinit == 0 {
        return true;
    }
    if conflict_limit_hit(solver) {
        return false;
    }
    true
}

/// Port of `kissat_search`.
pub fn search(solver: &mut Solver) -> i32 {
    crate::report::report(solver, false, '*'); // REPORT (0, '*')
    let mut res = 0;
    if solver.inconsistent {
        res = 20;
    }
    if res == 0 && solver.options.luckyearly != 0 {
        res = crate::lucky::lucky(solver);
    }
    if res == 0 && crate::preprocess::preprocessing(solver) {
        res = crate::preprocess::preprocess(solver);
    }
    if res == 0 && solver.options.luckylate != 0 {
        res = crate::lucky::lucky(solver);
    }
    if res == 0 {
        crate::classify::classify(solver);
    }
    if res == 0 && searching(solver) {
        start_search(solver);
        while res == 0 {
            let conflict = crate::propsearch::search_propagate(solver);
            if let Some(conflict) = conflict {
                res = crate::analyze::analyze(solver, conflict);
            } else if solver.iterating {
                iterate(solver);
            } else if solver.unassigned == 0 {
                res = 10;
            } else if terminated!(solver, search_terminated_1) {
                break;
            } else if crate::reduce::reducing(solver) {
                res = crate::reduce::reduce(solver);
            } else if crate::mode::switching_search_mode(solver) {
                crate::mode::switch_search_mode(solver);
            } else if crate::restart::restarting(solver) {
                crate::restart::restart(solver);
            } else if crate::reorder::reordering(solver) {
                crate::reorder::reorder(solver);
            } else if crate::rephase::rephasing(solver) {
                crate::rephase::rephase(solver);
            } else if crate::probe::probing(solver) {
                res = crate::probe::probe(solver);
            } else if crate::eliminate::eliminating(solver) {
                res = crate::eliminate::eliminate(solver);
            } else if conflict_limit_hit(solver) {
                break;
            } else if decision_limit_hit(solver) {
                break;
            } else {
                crate::decide::decide(solver);
            }
        }
        stop_search(solver);
    }
    report_search_result(solver, res);
    res
}
