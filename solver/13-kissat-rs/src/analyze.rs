// Port of src/analyze.c (kissat 4.0.4) — the conflict-analysis driver.
//
// PORT NOTES:
//  - `clause *conflict` is the crate::propsearch::Conflict handle:
//    Conflict::Binary is C's `&solver->conflict` (its two literals live in
//    solver.conflict.lits[0..2]), Conflict::Clause(ref) an arena clause.
//    The helpers conflict_size/conflict_lit below read through either and
//    are shared with deduce.rs.
//  - kissat_deduce_first_uip_clause's C NULL result is None; on a Some
//    (on-the-fly strengthened) result analysis restarts with that clause,
//    exactly like the C `do { .. } while (!res)` with the reassigned
//    `conflict`.
//  - one_literal_on_conflict_level's watch-fixing loop mutates the arena
//    conflict's literals; kissat_unwatch_blocking runs before the swap and
//    kissat_watch_blocking after, in C effect order.  The `ref =
//    kissat_reference_clause (solver, conflict)` is the handle's own
//    reference.
//  - Backtracking inside this function unassigns literals but leaves their
//    `assigned` entries (level/reason) stale exactly as in C; subsequent
//    level reads here rely on that.
//  - SORT_STACK / RADIX_STACK profile hooks (sort/radix, level 4) are
//    hoisted around the calls per the crate::sort convention.
//  - UPDATE_AVERAGE (trail/level) are `#ifndef QUIET` — kept.  The
//    LOGGING-only resolvent stack is omitted.
//  - ADD (literals_minimized/...deduced) at analyze-chain sites are METRIC —
//    no-ops (see the individual modules).

use crate::internal::{Solver, DECISION_REASON, INVALID_LEVEL, UNIT_REASON};
use crate::kimits::DelayId;
use crate::literal::{self, INVALID_LIT};
use crate::profile::{self, Prof};
use crate::propsearch::Conflict;
use crate::reference::{Reference, INVALID_REF};

/*------------------------------------------------------------------------*/
// Conflict handle accessors (see module PORT NOTES).

/// Size of the conflict clause behind a Conflict handle.
#[inline]
pub fn conflict_size(solver: &Solver, conflict: Conflict) -> u32 {
    match conflict {
        Conflict::Binary => solver.conflict.size,
        Conflict::Clause(ref_) => solver.arena.clause(ref_).size(),
    }
}

/// Literal `i` of the conflict clause behind a Conflict handle.
#[inline]
pub fn conflict_lit(solver: &Solver, conflict: Conflict, i: u32) -> u32 {
    match conflict {
        Conflict::Binary => solver.conflict.lits[i as usize],
        Conflict::Clause(ref_) => solver.arena.clause(ref_).lit(i),
    }
}

/*------------------------------------------------------------------------*/

// C static `one_literal_on_conflict_level`.
fn one_literal_on_conflict_level(
    solver: &mut Solver,
    conflict: Conflict,
    conflict_level_ptr: &mut u32,
) -> bool {
    let conflict_size = conflict_size(solver, conflict);
    debug_assert!(conflict_size > 1);

    let mut jump_level = INVALID_LEVEL;
    let mut conflict_level = INVALID_LEVEL;
    let mut literals_on_conflict_level = 0u32;
    let mut forced_lit = INVALID_LIT;

    for i in 0..conflict_size {
        let lit = conflict_lit(solver, conflict, i);
        debug_assert!(solver.values[lit as usize] < 0);
        let idx = literal::idx(lit);
        let lit_level = solver.assigned[idx as usize].level;
        if conflict_level == INVALID_LEVEL || conflict_level < lit_level {
            literals_on_conflict_level = 1;
            jump_level = conflict_level;
            conflict_level = lit_level;
            forced_lit = lit;
        } else {
            if jump_level == INVALID_LEVEL || jump_level < lit_level {
                jump_level = lit_level;
            }
            if conflict_level == lit_level {
                literals_on_conflict_level += 1;
            }
        }
        if literals_on_conflict_level > 1 && conflict_level == solver.level {
            break;
        }
    }
    debug_assert!(conflict_level != INVALID_LEVEL);
    debug_assert!(literals_on_conflict_level != 0);

    *conflict_level_ptr = conflict_level;

    if conflict_level == 0 {
        solver.inconsistent = true;
        // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
        if solver.proof.is_some() {
            crate::proof::add_empty_to_proof(solver);
        }
        return false;
    }

    if conflict_level < solver.level {
        crate::backtrack::backtrack_after_conflict(solver, conflict_level);
    }

    if conflict_size > 2 {
        let Conflict::Clause(cref) = conflict else {
            unreachable!("size > 2 conflicts are arena clauses");
        };
        for i in 0..2u32 {
            let lit = solver.arena.clause(cref).lit(i);
            let lit_idx = literal::idx(lit);
            let mut highest_position = i;
            let mut highest_literal = lit;
            let mut highest_level = solver.assigned[lit_idx as usize].level;
            for j in (i + 1)..conflict_size {
                let other = solver.arena.clause(cref).lit(j);
                let other_idx = literal::idx(other);
                let level = solver.assigned[other_idx as usize].level;
                if highest_level >= level {
                    continue;
                }
                highest_literal = other;
                highest_position = j;
                highest_level = level;
                if highest_level == conflict_level {
                    break;
                }
            }
            if highest_position == i {
                continue;
            }
            let mut ref_: Reference = INVALID_REF;
            if highest_position > 1 {
                ref_ = cref; // kissat_reference_clause (solver, conflict)
                crate::watch::unwatch_blocking(solver, lit, ref_);
            }
            {
                let mut c = solver.arena.clause_mut(cref);
                c.set_lit(highest_position, lit);
                c.set_lit(i, highest_literal);
            }
            if highest_position > 1 {
                let lit_i = solver.arena.clause(cref).lit(i);
                let lit_other = solver.arena.clause(cref).lit(i ^ 1); // lits[!i]
                crate::watch::watch_blocking(solver, lit_i, lit_other, ref_);
            }
        }
    }

    if literals_on_conflict_level > 1 {
        return false;
    }

    debug_assert!(literals_on_conflict_level == 1);
    debug_assert!(forced_lit != INVALID_LIT);
    debug_assert!(jump_level != INVALID_LEVEL);
    debug_assert!(jump_level < conflict_level);

    let new_level = crate::learn::determine_new_level(solver, jump_level);
    crate::backtrack::backtrack_after_conflict(solver, new_level);

    if conflict_size == 2 {
        debug_assert!(conflict == Conflict::Binary); // conflict == &solver->conflict
        let other = solver.conflict.lits[0] ^ solver.conflict.lits[1] ^ forced_lit;
        crate::assign::assign_binary(solver, forced_lit, other);
    } else {
        let Conflict::Clause(cref) = conflict else {
            unreachable!();
        };
        crate::assign::assign_reference(solver, forced_lit, cref);
    }

    true
}

// C static inline `mark_reason_side_literal`.
#[inline]
fn mark_reason_side_literal(solver: &mut Solver, lit: u32) {
    let idx = literal::idx(lit);
    let a = &solver.assigned[idx as usize];
    if a.level != 0 && !a.analyzed() {
        crate::inline::push_analyzed(solver, idx);
    }
}

// C static inline `analyze_reason_side_literal`.
#[inline]
fn analyze_reason_side_literal(solver: &mut Solver, limit: usize, lit: u32) {
    let idx = literal::idx(lit);
    let a = solver.assigned[idx as usize];
    debug_assert!(a.level != 0);
    debug_assert!(a.analyzed());
    debug_assert!(a.reason != UNIT_REASON);
    if a.reason == DECISION_REASON {
        return;
    }
    if a.binary() {
        let other = a.reason;
        mark_reason_side_literal(solver, other);
    } else {
        let ref_ = a.reason;
        solver.statistics.search_ticks += 1; // INC (search_ticks)
        let not_lit = literal::not(lit);
        let size = solver.arena.clause(ref_).size();
        for i in 0..size {
            let other = solver.arena.clause(ref_).lit(i);
            if other != not_lit {
                debug_assert!(other != lit);
                mark_reason_side_literal(solver, other);
                if solver.analyzed.len() > limit {
                    break;
                }
            }
        }
    }
}

// C static `analyze_reason_side_literals`.
fn analyze_reason_side_literals(solver: &mut Solver) {
    if solver.options.bump == 0 {
        return;
    }
    if solver.options.bumpreasons == 0 {
        return;
    }
    if solver.probing {
        return;
    }
    if crate::kimits::delaying(solver, DelayId::Bumpreasons) {
        // DELAYING (bumpreasons)
        return;
    }
    // AVERAGE (decision_rate):
    let decision_rate = solver.averages[solver.stable as usize].decision_rate.value;
    let decision_rate_limit = solver.options.bumpreasonsrate;
    if decision_rate >= decision_rate_limit as f64 {
        return;
    }
    let saved = solver.analyzed.len();
    let limit = solver.options.bumpreasonslimit as usize * saved;
    for i in 0..solver.clause.len() {
        let lit = solver.clause[i];
        analyze_reason_side_literal(solver, limit, lit);
        if solver.analyzed.len() > limit {
            break;
        }
    }
    if solver.analyzed.len() > limit {
        while solver.analyzed.len() > saved {
            let idx = solver.analyzed.pop().unwrap();
            debug_assert!(solver.assigned[idx as usize].analyzed());
            solver.assigned[idx as usize].set_analyzed(false);
        }
        crate::kimits::bump_delay(solver, DelayId::Bumpreasons); // BUMP_DELAY
    } else {
        crate::kimits::reduce_delay(solver, DelayId::Bumpreasons); // REDUCE_DELAY
    }
}

const RADIX_SORT_LEVELS_LIMIT: usize = 32;

// C static `sort_levels`: RANK_LEVEL (A) = A.
fn sort_levels(solver: &mut Solver) {
    let glue = solver.levels.len();
    if glue < RADIX_SORT_LEVELS_LIMIT {
        // SORT_STACK (unsigned, *levels, SMALLER_LEVEL)
        profile::start_checked(solver, Prof::sort);
        crate::sort::sort_stack(&mut solver.sorter, &mut solver.levels, |a: &u32, b: &u32| {
            a < b
        });
        profile::stop_checked(solver, Prof::sort);
    } else {
        // RADIX_STACK (unsigned, unsigned, *levels, RANK_LEVEL)
        profile::start_checked(solver, Prof::radix);
        crate::sort::radix_stack::<u32, u32, _>(&mut solver.levels, |&l| l);
        profile::stop_checked(solver, Prof::radix);
    }
}

// C static `sort_deduced_clause` — counting sort of solver.clause into
// solver.shadow by decision level (descending), reusing frame->used as the
// per-level write cursor and restoring it afterwards.
fn sort_deduced_clause(solver: &mut Solver) {
    sort_levels(solver);

    let mut pos: u32 = 1;
    for li in (0..solver.levels.len()).rev() {
        let level = solver.levels[li];
        let f = &mut solver.frames[level as usize];
        let used = f.used;
        debug_assert!(used > 0);
        debug_assert!(u32::MAX - used >= pos);
        f.used = pos;
        pos += used;
    }

    let size_clause = solver.clause.len();
    debug_assert!(pos as usize == size_clause);
    debug_assert!(size_clause > 0);

    while solver.shadow.len() < size_clause {
        solver.shadow.push(INVALID_LIT);
    }

    let not_uip = solver.clause[0];
    solver.shadow[0] = not_uip; // POKE_STACK (*shadow, 0, not_uip)

    for i in 1..size_clause {
        let lit = solver.clause[i];
        let idx = literal::idx(lit);
        let level = solver.assigned[idx as usize].level;
        let f = &mut solver.frames[level as usize];
        let p = f.used;
        f.used += 1;
        solver.shadow[p as usize] = lit;
    }

    debug_assert!(size_clause == solver.shadow.len());
    std::mem::swap(&mut solver.clause, &mut solver.shadow); // SWAP (unsigneds, ..)

    let mut pos: u32 = 1;
    for li in (0..solver.levels.len()).rev() {
        let level = solver.levels[li];
        let f = &mut solver.frames[level as usize];
        let end = f.used;
        debug_assert!(pos < end);
        f.used = end - pos;
        pos = end;
    }

    solver.shadow.clear();
}

// C static `reset_levels`.
fn reset_levels(solver: &mut Solver) {
    for i in 0..solver.levels.len() {
        let level = solver.levels[i];
        let f = &mut solver.frames[level as usize];
        debug_assert!(f.used > 0);
        f.used = 0;
    }
    solver.levels.clear();
}

/// Port of `kissat_reset_only_analyzed_literals`.
pub fn reset_only_analyzed_literals(solver: &mut Solver) {
    for i in 0..solver.analyzed.len() {
        let idx = solver.analyzed[i];
        debug_assert!(idx < solver.vars());
        let a = &mut solver.assigned[idx as usize];
        debug_assert!(!a.poisoned());
        debug_assert!(!a.removable());
        debug_assert!(!a.shrinkable());
        a.set_analyzed(false);
    }
    solver.analyzed.clear();
}

// C static `reset_removable`.
fn reset_removable(solver: &mut Solver) {
    for i in 0..solver.removable.len() {
        let idx = solver.removable[i];
        debug_assert!(idx < solver.vars());
        solver.assigned[idx as usize].set_removable(false);
    }
    solver.removable.clear();
}

// C static `reset_analysis_but_not_analyzed_literals`.
fn reset_analysis_but_not_analyzed_literals(solver: &mut Solver) {
    reset_removable(solver);
    reset_levels(solver);
    solver.clause.clear();
}

// C static `update_trail_average` (#ifndef QUIET body — kept).
fn update_trail_average(solver: &mut Solver) {
    debug_assert!(!solver.probing);
    let size = solver.trail.len() as u32; // SIZE_ARRAY (solver->trail)
    let assigned = size - solver.unflushed;
    let active = solver.active;
    let filled = crate::utilities::percent(assigned as f64, active as f64);
    // UPDATE_AVERAGE (trail, filled):
    let stable = solver.stable as usize;
    crate::smooth::update_smooth(&mut solver.averages[stable].trail, filled);
}

// C static `update_decision_rate_average`.
fn update_decision_rate_average(solver: &mut Solver) {
    debug_assert!(!solver.probing);
    let current = solver.statistics.decisions; // DECISIONS
    let stable = solver.stable as usize;
    let previous = solver.averages[stable].saved_decisions;
    debug_assert!(previous <= current);
    let decisions = current - previous;
    solver.averages[stable].saved_decisions = current;
    // UPDATE_AVERAGE (decision_rate, decisions):
    crate::smooth::update_smooth(
        &mut solver.averages[stable].decision_rate,
        decisions as f64,
    );
}

// C static `analyze_failed_literal` — conflict at conflict level one:
// resolve everything on level 1 back to (possibly several) learned units.
// The C `goto DONE` is the labeled break of 'resolve below.
fn analyze_failed_literal(solver: &mut Solver, conflict: Conflict) {
    debug_assert!(solver.level == 1);
    let failed = solver.frames[1].decision; // FRAME (1).decision

    // unsigneds *units = &solver->clause (alias).
    debug_assert!(solver.clause.is_empty());
    debug_assert!(solver.analyzed.is_empty());

    let not_failed = literal::not(failed);
    let mut t = solver.trail.len(); // END_ARRAY (solver->trail)
    let mut unresolved = 0u32;
    let mut unit = INVALID_LIT;

    'resolve: {
        let csize = conflict_size(solver, conflict);
        for i in 0..csize {
            let lit = conflict_lit(solver, conflict, i);
            debug_assert!(lit != failed);
            if lit == not_failed {
                break 'resolve; // goto DONE
            }
            debug_assert!(solver.values[lit as usize] < 0);
            let idx = literal::idx(lit);
            if solver.assigned[idx as usize].level == 0 {
                continue;
            }
            debug_assert!(solver.assigned[idx as usize].level == 1);
            crate::inline::push_analyzed(solver, idx);
            unresolved += 1;
        }

        loop {
            let lit;
            loop {
                debug_assert!(t > 0);
                t -= 1;
                let l = solver.trail[t]; // lit = *--t
                debug_assert!(solver.values[l as usize] > 0);
                if solver.assigned[literal::idx(l) as usize].analyzed() {
                    lit = l;
                    break;
                }
            }
            if unresolved == 1 {
                unit = literal::not(lit);
                solver.clause.push(unit); // PUSH_STACK (*units, unit)
            }
            let a = solver.assigned[literal::idx(lit) as usize];
            if a.binary() {
                let other = a.reason;
                debug_assert!(other != failed);
                debug_assert!(other != unit);
                debug_assert!(solver.values[other as usize] < 0);
                if other == not_failed {
                    break 'resolve; // goto DONE
                }
                let idx = literal::idx(other);
                debug_assert!(solver.assigned[idx as usize].level == 1);
                if !solver.assigned[idx as usize].analyzed() {
                    crate::inline::push_analyzed(solver, idx);
                    unresolved += 1;
                }
            } else {
                debug_assert!(a.reason != UNIT_REASON);
                debug_assert!(a.reason != DECISION_REASON);
                let ref_ = a.reason;
                let size = solver.arena.clause(ref_).size();
                for i in 0..size {
                    let other = solver.arena.clause(ref_).lit(i);
                    debug_assert!(other != literal::not(lit));
                    debug_assert!(other != failed);
                    if other == lit {
                        continue;
                    }
                    if other == unit {
                        continue;
                    }
                    if other == not_failed {
                        break 'resolve; // goto DONE
                    }
                    debug_assert!(solver.values[other as usize] < 0);
                    let idx = literal::idx(other);
                    if solver.assigned[idx as usize].level == 0 {
                        continue;
                    }
                    debug_assert!(solver.assigned[idx as usize].level == 1);
                    if solver.assigned[idx as usize].analyzed() {
                        continue;
                    }
                    crate::inline::push_analyzed(solver, idx);
                    unresolved += 1;
                }
            }
            debug_assert!(unresolved > 0);
            unresolved -= 1;
        }
    }
    // DONE:
    solver.clause.push(not_failed); // PUSH_STACK (*units, not_failed)

    if !solver.probing {
        crate::learn::update_learned(solver, 0, 1);
    }

    crate::backtrack::backtrack_without_updating_phases(solver, 0);

    // for (all_stack (unsigned, lit, *units)) kissat_learned_unit (..):
    // PORT NOTE: units aliases solver.clause; taken out around the loop
    // (kissat_learned_unit never touches solver->clause), restored, then
    // cleared as in C.
    let units = std::mem::take(&mut solver.clause);
    for &lit in &units {
        crate::assign::learned_unit(solver, lit);
    }
    solver.clause = units;
    solver.clause.clear(); // CLEAR_STACK (*units)
    if !solver.probing {
        solver.iterating = true;
        solver.statistics.iterations += 1; // INC (iterations)
    }
}

// C static `update_tier_limits`.
fn update_tier_limits(solver: &mut Solver) {
    solver.statistics.retiered += 1; // INC (retiered)
    crate::tiers::compute_and_set_tier_limits(solver);
    if solver.limits.glue.interval < (1u64 << 16) {
        solver.limits.glue.interval *= 2;
    }
    solver.limits.glue.conflicts = solver.statistics.conflicts + solver.limits.glue.interval;
}

/// Port of `kissat_analyze`.  Returns 0 to continue searching, 20 on
/// (root-level) UNSAT.
pub fn analyze(solver: &mut Solver, conflict: Conflict) -> i32 {
    if solver.inconsistent {
        debug_assert!(solver.level == 0);
        return 20;
    }

    profile::start_checked(solver, Prof::analyze); // START (analyze)
    if !solver.probing {
        update_trail_average(solver);
        update_decision_rate_average(solver);
        // UPDATE_AVERAGE (level, solver->level)  (#ifndef QUIET — kept):
        let level = solver.level;
        let stable = solver.stable as usize;
        crate::smooth::update_smooth(&mut solver.averages[stable].level, level as f64);
    }
    let mut conflict = conflict;
    let mut res: i32;
    loop {
        let mut conflict_level = 0u32;
        if one_literal_on_conflict_level(solver, conflict, &mut conflict_level) {
            res = 1;
        } else if conflict_level == 0 {
            res = -1;
        } else if conflict_level == 1 {
            analyze_failed_literal(solver, conflict);
            res = 1;
        } else if let Some(strengthened) =
            crate::deduce::deduce_first_uip_clause(solver, conflict)
        {
            conflict = strengthened;
            reset_analysis_but_not_analyzed_literals(solver);
            solver.statistics.conflicts += 1; // INC (conflicts)
            if solver.statistics.conflicts > solver.limits.glue.conflicts {
                update_tier_limits(solver);
            }
            res = 0; // And continue with new conflict analysis.
        } else {
            if solver.options.minimize != 0 {
                sort_deduced_clause(solver);
                crate::minimize::minimize_clause(solver);
                if solver.options.shrink != 0 {
                    crate::shrink::shrink_clause(solver);
                }
            }
            analyze_reason_side_literals(solver);
            crate::learn::learn_clause(solver);
            reset_analysis_but_not_analyzed_literals(solver);
            res = 1;
        }
        if !solver.analyzed.is_empty() {
            if !solver.probing && solver.options.bump != 0 {
                crate::bump::bump_analyzed(solver);
            }
            reset_only_analyzed_literals(solver);
        }
        if res != 0 {
            break;
        }
    }
    profile::stop_checked(solver, Prof::analyze); // STOP (analyze)
    if res > 0 {
        0
    } else {
        20
    }
}
