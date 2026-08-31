// Port of src/lucky.c (kissat 4.0.4).
//
// PORT NOTES:
//  - `for (all_clauses (c))` with the `last_irredundant && last_irredundant
//    < c` break becomes reference iteration with next_clause_ref, breaking
//    once ref_ > solver.last_irredundant (same idiom as
//    watch::connect_irredundant_large_clauses).
//  - `all_binary_blocking_watches` (watch.h) advances the cursor by
//    1 + !binary words; ported as an explicit two-word skip for blocking
//    watches.
//  - The C `goto CONTINUE_WITH_NEXT_CLAUSE` is a labeled continue.
//  - `kissat_probing_propagate (solver, 0, true)`: NULL ignore ==
//    INVALID_REF.
//  - The `#ifndef QUIET` conflicts counters are kept (QUIET not defined),
//    as is the `success` flag feeding REPORT.
//  - In kissat_lucky the `#ifndef NDEBUG clause *c = ...` assignment only
//    wraps the assert; the propagate call itself is unconditional.

use crate::internal::Solver;
use crate::profile::Prof;
use crate::reference::{Reference, INVALID_REF};

fn no_all_negative_clauses(solver: &mut Solver) -> bool {
    let last_irredundant = solver.last_irredundant; // kissat_last_irredundant_clause
    let mut ref_: Reference = 0;
    // for (all_clauses (c))
    'clauses: while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        // if (last_irredundant && last_irredundant < c) break;
        if last_irredundant != INVALID_REF && ref_ > last_irredundant {
            break;
        }
        let (redundant, garbage, size) = {
            let c = solver.arena.clause(ref_);
            (c.redundant(), c.garbage(), c.size())
        };
        if redundant || garbage {
            ref_ = next;
            continue;
        }
        for i in 0..size {
            let lit = solver.arena.clause(ref_).lit(i);
            if crate::literal::negated(lit) == 0 && solver.values[lit as usize] >= 0 {
                // goto CONTINUE_WITH_NEXT_CLAUSE
                ref_ = next;
                continue 'clauses;
            }
        }
        crate::print::verbose(solver, "found all negative large clause");
        return false;
    }
    debug_assert!(solver.watching);
    // for (all_variables (idx))
    for idx in 0..solver.vars {
        if !solver.flags[idx as usize].active {
            continue;
        }
        let lit = crate::literal::lit(idx);
        let not_lit = crate::literal::not(lit);
        // for (all_binary_blocking_watches (watch, WATCHES (not_lit)))
        let watches = solver.watches[not_lit as usize];
        let mut p = watches.begin;
        while p != watches.end {
            let watch = solver.vectors.stack[p];
            let binary = crate::watch::watch_is_binary(watch);
            p += if binary { 1 } else { 2 };
            if !binary {
                continue;
            }
            let other = crate::watch::watch_lit(watch); // watch.binary.lit
            if crate::literal::negated(other) != 0
                && solver.flags[crate::literal::idx(other) as usize].active
            {
                crate::print::verbose(solver, "found all negative binary clause");
                return false;
            }
        }
    }
    crate::print::message(solver, "lucky no all-negative clause");
    true
}

fn no_all_positive_clauses(solver: &mut Solver) -> bool {
    let last_irredundant = solver.last_irredundant; // kissat_last_irredundant_clause
    let mut ref_: Reference = 0;
    // for (all_clauses (c))
    'clauses: while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        if last_irredundant != INVALID_REF && ref_ > last_irredundant {
            break;
        }
        let (redundant, garbage, size) = {
            let c = solver.arena.clause(ref_);
            (c.redundant(), c.garbage(), c.size())
        };
        if redundant || garbage {
            ref_ = next;
            continue;
        }
        for i in 0..size {
            let lit = solver.arena.clause(ref_).lit(i);
            if crate::literal::negated(lit) != 0 && solver.values[lit as usize] >= 0 {
                // goto CONTINUE_WITH_NEXT_CLAUSE
                ref_ = next;
                continue 'clauses;
            }
        }
        crate::print::verbose(solver, "found all positive large clause");
        return false;
    }
    debug_assert!(solver.watching);
    for idx in 0..solver.vars {
        if !solver.flags[idx as usize].active {
            continue;
        }
        let lit = crate::literal::lit(idx);
        let watches = solver.watches[lit as usize];
        let mut p = watches.begin;
        while p != watches.end {
            let watch = solver.vectors.stack[p];
            let binary = crate::watch::watch_is_binary(watch);
            p += if binary { 1 } else { 2 };
            if !binary {
                continue;
            }
            let other = crate::watch::watch_lit(watch); // watch.binary.lit
            if crate::literal::negated(other) == 0
                && solver.flags[crate::literal::idx(other) as usize].active
            {
                crate::print::verbose(solver, "found all positive binary clause");
                return false;
            }
        }
    }
    crate::print::message(solver, "lucky no all-positive clause");
    true
}

fn forward_false_satisfiable(solver: &mut Solver) -> i32 {
    debug_assert!(solver.level == 0);
    let mut conflicts: u32 = 0; // #ifndef QUIET
    // for (all_stack (import, import, solver->import))
    for i in 0..solver.import_.len() {
        let import = solver.import_[i];
        if !import.imported {
            continue;
        }
        if import.eliminated {
            continue;
        }
        let lit = import.lit;
        let idx = crate::literal::idx(lit);
        if !solver.flags[idx as usize].active {
            continue;
        }
        if solver.values[lit as usize] != 0 {
            continue;
        }
        let not_lit = crate::literal::not(lit);
        crate::decide::internal_assume(solver, not_lit);
        let c = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
        let Some(c) = c else {
            continue;
        };
        conflicts += 1;
        if solver.level > 1 {
            crate::backtrack::backtrack_without_updating_phases(solver, solver.level - 1);
            crate::decide::internal_assume(solver, lit);
            let d = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
            if d.is_none() {
                continue;
            }
            crate::print::verbose(
                solver,
                format!(
                    "inconsistency after {} conflicts \
                     forward assigning {} variables to false",
                    conflicts, solver.level
                ),
            );
            crate::backtrack::backtrack_without_updating_phases(solver, 0);
            return 0;
        } else {
            let _ = crate::analyze::analyze(solver, c);
            debug_assert!(solver.level == 0);
            let d = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
            if let Some(d) = d {
                let _ = crate::analyze::analyze(solver, d);
                debug_assert!(solver.inconsistent);
                crate::print::verbose(
                    solver,
                    "lucky inconsistency forward assigning to false",
                );
                return 20;
            }
        }
    }

    crate::print::message(solver, "lucky in forward setting literals to false");
    10
}

fn forward_true_satisfiable(solver: &mut Solver) -> i32 {
    debug_assert!(solver.level == 0);
    let mut conflicts: u32 = 0; // #ifndef QUIET
    for i in 0..solver.import_.len() {
        let import = solver.import_[i];
        if !import.imported {
            continue;
        }
        if import.eliminated {
            continue;
        }
        let lit = import.lit;
        let idx = crate::literal::idx(lit);
        if !solver.flags[idx as usize].active {
            continue;
        }
        if solver.values[lit as usize] != 0 {
            continue;
        }
        crate::decide::internal_assume(solver, lit);
        let c = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
        let Some(c) = c else {
            continue;
        };
        conflicts += 1;
        if solver.level > 1 {
            crate::backtrack::backtrack_without_updating_phases(solver, solver.level - 1);
            let not_lit = crate::literal::not(lit);
            crate::decide::internal_assume(solver, not_lit);
            let d = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
            if d.is_none() {
                continue;
            }
            crate::print::verbose(
                solver,
                format!(
                    "inconsistency after {} conflicts \
                     forward assigning {} variables to true",
                    conflicts, solver.level
                ),
            );
            crate::backtrack::backtrack_without_updating_phases(solver, 0);
            return 0;
        } else {
            let _ = crate::analyze::analyze(solver, c);
            debug_assert!(solver.level == 0);
            let d = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
            if let Some(d) = d {
                let _ = crate::analyze::analyze(solver, d);
                debug_assert!(solver.inconsistent);
                crate::print::verbose(
                    solver,
                    "lucky inconsistency forward assigning to true",
                );
                return 20;
            }
        }
    }
    crate::print::message(solver, "lucky in forward setting literals to true");
    10
}

fn backward_false_satisfiable(solver: &mut Solver) -> i32 {
    debug_assert!(solver.level == 0);
    let mut conflicts: u32 = 0; // #ifndef QUIET
    // import *p = end; while (p != begin) { const import import = *--p; ... }
    for i in (0..solver.import_.len()).rev() {
        let import = solver.import_[i];
        if !import.imported {
            continue;
        }
        if import.eliminated {
            continue;
        }
        let lit = import.lit;
        let idx = crate::literal::idx(lit);
        if !solver.flags[idx as usize].active {
            continue;
        }
        if solver.values[lit as usize] != 0 {
            continue;
        }
        let not_lit = crate::literal::not(lit);
        crate::decide::internal_assume(solver, not_lit);
        let c = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
        let Some(c) = c else {
            continue;
        };
        conflicts += 1;
        if solver.level > 1 {
            crate::backtrack::backtrack_without_updating_phases(solver, solver.level - 1);
            crate::decide::internal_assume(solver, lit);
            let d = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
            if d.is_none() {
                continue;
            }
            crate::print::verbose(
                solver,
                format!(
                    "inconsistency after {} conflicts \
                     backward assigning {} variables to false",
                    conflicts, solver.level
                ),
            );
            crate::backtrack::backtrack_without_updating_phases(solver, 0);
            return 0;
        } else {
            let _ = crate::analyze::analyze(solver, c);
            debug_assert!(solver.level == 0);
            let d = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
            if let Some(d) = d {
                let _ = crate::analyze::analyze(solver, d);
                debug_assert!(solver.inconsistent);
                crate::print::verbose(
                    solver,
                    "lucky inconsistency backward assigning to false",
                );
                return 20;
            }
        }
    }
    crate::print::message(solver, "lucky in backward setting literals to false");
    10
}

fn backward_true_satisfiable(solver: &mut Solver) -> i32 {
    debug_assert!(solver.level == 0);
    let mut conflicts: u32 = 0; // #ifndef QUIET
    for i in (0..solver.import_.len()).rev() {
        let import = solver.import_[i];
        if !import.imported {
            continue;
        }
        if import.eliminated {
            continue;
        }
        let lit = import.lit;
        let idx = crate::literal::idx(lit);
        if !solver.flags[idx as usize].active {
            continue;
        }
        if solver.values[lit as usize] != 0 {
            continue;
        }
        crate::decide::internal_assume(solver, lit);
        let c = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
        let Some(c) = c else {
            continue;
        };
        conflicts += 1;
        if solver.level > 1 {
            crate::backtrack::backtrack_without_updating_phases(solver, solver.level - 1);
            let not_lit = crate::literal::not(lit);
            crate::decide::internal_assume(solver, not_lit);
            let d = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
            if d.is_none() {
                continue;
            }
            crate::print::verbose(
                solver,
                format!(
                    "inconsistency after {} conflicts \
                     backward assigning {} variables to true",
                    conflicts, solver.level
                ),
            );
            crate::backtrack::backtrack_without_updating_phases(solver, 0);
            return 0;
        } else {
            let _ = crate::analyze::analyze(solver, c);
            debug_assert!(solver.level == 0);
            let d = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
            if let Some(d) = d {
                let _ = crate::analyze::analyze(solver, d);
                debug_assert!(solver.inconsistent);
                crate::print::verbose(
                    solver,
                    "lucky inconsistency backward assigning to true",
                );
                return 20;
            }
        }
    }
    crate::print::message(solver, "lucky in backward setting literals to true");
    10
}

// kissat_propagated (inline.h), assert-only here.
#[inline]
fn propagated(solver: &Solver) -> bool {
    solver.propagate == solver.trail.len()
}

/// kissat_lucky.
pub fn lucky(solver: &mut Solver) -> i32 {
    if solver.inconsistent {
        return 0;
    }

    if solver.options.lucky == 0 {
        return 0;
    }

    crate::profile::start_checked(solver, Prof::lucky); // START (lucky)
    debug_assert!(solver.level == 0);
    debug_assert!(!solver.probing);
    solver.probing = true;
    debug_assert!(propagated(solver));

    let mut res = 0;

    if no_all_negative_clauses(solver) {
        for idx in 0..solver.vars {
            if !solver.flags[idx as usize].active {
                continue;
            }
            let lit = crate::literal::lit(idx);
            if solver.values[lit as usize] != 0 {
                continue;
            }
            crate::decide::internal_assume(solver, lit);
            let c = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
            debug_assert!(c.is_none());
            let _ = c;
        }
        crate::print::verbose(solver, "set all variables to true");
        debug_assert!(propagated(solver));
        debug_assert!(solver.unassigned == 0);
        res = 10;
    }

    if res == 0 && no_all_positive_clauses(solver) {
        for idx in 0..solver.vars {
            if !solver.flags[idx as usize].active {
                continue;
            }
            let lit = crate::literal::lit(idx);
            if solver.values[lit as usize] != 0 {
                continue;
            }
            let not_lit = crate::literal::not(lit);
            crate::decide::internal_assume(solver, not_lit);
            let c = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
            debug_assert!(c.is_none());
            let _ = c;
        }
        crate::print::verbose(solver, "set all variables to false");
        debug_assert!(propagated(solver));
        debug_assert!(solver.unassigned == 0);
        res = 10;
    }

    let active_before = solver.active;

    if res == 0 {
        res = forward_false_satisfiable(solver);
    }

    if res == 0 {
        res = forward_true_satisfiable(solver);
    }

    if res == 0 {
        res = backward_false_satisfiable(solver);
    }

    if res == 0 {
        res = backward_true_satisfiable(solver);
    }

    let active_after = solver.active;
    let units = active_before - active_after;

    if res == 0 && units != 0 {
        crate::print::message(solver, format!("lucky {} units", units));
    }

    let success = res != 0 || units != 0; // #ifndef QUIET
    debug_assert!(solver.probing);
    solver.probing = false;
    crate::report::report(solver, !success, 'l'); // REPORT (!success, 'l')
    crate::profile::stop_checked(solver, Prof::lucky); // STOP (lucky)

    res
}
