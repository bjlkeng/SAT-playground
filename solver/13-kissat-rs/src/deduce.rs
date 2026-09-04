// Port of src/deduce.c (kissat 4.0.4) — first-UIP deduction.
//
// PORT NOTES:
//  - `clause *conflict` is the crate::propsearch::Conflict handle
//    (Binary == C's `&solver->conflict`); size/literal reads go through
//    crate::analyze::{conflict_size, conflict_lit}.
//  - The C NULL result ("first-UIP clause deduced into solver->clause") is
//    None; Some(..) is the on-the-fly strengthened clause, which may be
//    Conflict::Binary (binary_on_the_fly_strengthen ends in
//    kissat_binary_conflict) or an arena clause.
//  - C deduce.c has both the *static* `recompute_and_promote` and the
//    public `kissat_recompute_and_promote`; the static one is inlined into
//    mark_clause_as_used here to avoid the post-prefix-drop name collision
//    (identical effect order).
//  - The `resolvent` stack is (LOGGING || !NDEBUG)-only and omitted;
//    resolvent_size/antecedent_size bookkeeping is kept exactly.
//  - ADD (literals_deduced, ..) is METRIC — no-op.

use crate::analyze::{conflict_lit, conflict_size};
use crate::internal::{Solver, DECISION_REASON};
use crate::literal::{self, INVALID_LIT};
use crate::profile::{self, Prof};
use crate::propsearch::Conflict;
use crate::reference::Reference;
use crate::statistics::MAX_GLUE_USED;

// C static inline `mark_clause_as_used` (with the static
// `recompute_and_promote` inlined — see module PORT NOTE).
fn mark_clause_as_used(solver: &mut Solver, ref_: Reference) {
    if !solver.arena.clause(ref_).redundant() {
        return;
    }
    solver.statistics.clauses_used += 1; // INC (clauses_used)
    solver
        .arena
        .clause_mut(ref_)
        .set_used(crate::clause::MAX_USED); // c->used = MAX_USED
    // recompute_and_promote (solver, c):
    {
        let old_glue = solver.arena.clause(ref_).glue();
        let new_glue = crate::promote::recompute_glue(solver, ref_, old_glue);
        if new_glue < old_glue {
            crate::promote::promote_clause(solver, ref_, new_glue);
        }
    }
    // unsigned glue = MIN (c->glue, MAX_GLUE_USED)  (post-promotion glue):
    let glue = solver.arena.clause(ref_).glue().min(MAX_GLUE_USED as u32);
    solver.statistics.used[solver.stable as usize].glue[glue as usize] += 1;
    if solver.stable {
        solver.statistics.clauses_used_stable += 1;
    } else {
        solver.statistics.clauses_used_focused += 1;
    }
}

/// Port of `kissat_recompute_and_promote`.
pub fn recompute_and_promote(solver: &mut Solver, ref_: Reference) -> bool {
    debug_assert!(solver.arena.clause(ref_).redundant());
    let old_glue = solver.arena.clause(ref_).glue();
    let new_glue = crate::promote::recompute_glue(solver, ref_, old_glue);
    if new_glue >= old_glue {
        return false;
    }
    crate::promote::promote_clause(solver, ref_, new_glue);
    true
}

// C static inline `analyze_literal` (the assigned/frames array arguments
// are direct field accesses here).
#[inline]
fn analyze_literal(solver: &mut Solver, lit: u32) -> bool {
    debug_assert!(solver.values[lit as usize] < 0);
    let idx = literal::idx(lit);
    let level = solver.assigned[idx as usize].level;
    if level == 0 {
        return false;
    }
    solver.antecedent_size += 1;
    if solver.assigned[idx as usize].analyzed() {
        return false;
    }
    crate::inline::push_analyzed(solver, idx);
    debug_assert!(level <= solver.level);
    // PUSH_STACK (solver->resolvent, lit): (LOGGING || !NDEBUG)-only.
    solver.resolvent_size += 1;
    if level == solver.level {
        return true;
    }
    solver.clause.push(lit);
    let f = &mut solver.frames[level as usize];
    let used = f.used;
    f.used += 1;
    if used != 0 {
        // if (f->used++) return false;
        return false;
    }
    solver.levels.push(level);
    false
}

/// Port of `kissat_deduce_first_uip_clause` (see module PORT NOTES for the
/// None / Some(Conflict) mapping of the C `clause *` result).
pub fn deduce_first_uip_clause(solver: &mut Solver, conflict: Conflict) -> Option<Conflict> {
    profile::start_checked(solver, Prof::deduce); // START (deduce)
    debug_assert!(solver.analyzed.is_empty());
    debug_assert!(solver.levels.is_empty());
    debug_assert!(solver.clause.is_empty());

    let csize = conflict_size(solver, conflict);
    if csize > 2 {
        let Conflict::Clause(cref) = conflict else {
            unreachable!("size > 2 conflicts are arena clauses");
        };
        mark_clause_as_used(solver, cref);
    }
    solver.clause.push(INVALID_LIT);
    solver.antecedent_size = 0;
    solver.resolvent_size = 0;
    let mut unresolved_on_current_level = 0u32;
    let mut conflict_size = 0u32;
    for i in 0..csize {
        let lit = conflict_lit(solver, conflict, i);
        debug_assert!(solver.values[lit as usize] < 0);
        if solver.assigned[literal::idx(lit) as usize].level != 0 {
            // if (LEVEL (lit)) conflict_size++;
            conflict_size += 1;
        }
        if analyze_literal(solver, lit) {
            unresolved_on_current_level += 1;
        }
    }
    debug_assert!(unresolved_on_current_level > 1);
    debug_assert!(solver.antecedent_size == solver.resolvent_size);

    let otfs = solver.options.otfs != 0;
    let mut t = solver.trail.len(); // END_ARRAY (solver->trail)
    let mut uip;
    let mut resolved = 0u32;
    loop {
        loop {
            debug_assert!(t > 0);
            t -= 1;
            uip = solver.trail[t]; // uip = *--t
            let a = &solver.assigned[literal::idx(uip) as usize];
            if a.analyzed() && a.level == solver.level {
                break;
            }
        }
        if unresolved_on_current_level == 1 {
            break;
        }
        let a = solver.assigned[literal::idx(uip) as usize];
        debug_assert!(a.reason != DECISION_REASON);
        debug_assert!(a.level == solver.level);
        solver.antecedent_size = 1;
        resolved += 1;
        if a.binary() {
            let other = a.reason;
            if analyze_literal(solver, other) {
                unresolved_on_current_level += 1;
            }
        } else {
            let ref_ = a.reason;
            let size = solver.arena.clause(ref_).size();
            for i in 0..size {
                let lit = solver.arena.clause(ref_).lit(i);
                if lit != uip && analyze_literal(solver, lit) {
                    unresolved_on_current_level += 1;
                }
            }
            mark_clause_as_used(solver, ref_);
        }
        debug_assert!(unresolved_on_current_level > 0);
        unresolved_on_current_level -= 1;
        debug_assert!(solver.resolvent_size > 0);
        solver.resolvent_size -= 1;
        // REMOVE_STACK (.., solver->resolvent, ..): (LOGGING || !NDEBUG)-only.
        if otfs
            && solver.antecedent_size > 2
            && solver.resolvent_size < solver.antecedent_size
        {
            debug_assert!(!a.binary());
            debug_assert!(!solver.arena.clause(a.reason).garbage());
            let res = crate::strengthen::on_the_fly_strengthen(solver, a.reason, uip);
            if resolved == 1 && solver.resolvent_size < conflict_size {
                debug_assert!(conflict_size > 2);
                crate::strengthen::on_the_fly_subsume(solver, res, conflict);
            }
            profile::stop_checked(solver, Prof::deduce); // STOP (deduce)
            return Some(res);
        }
    }
    debug_assert!(uip != INVALID_LIT);
    debug_assert!(solver.clause[0] == INVALID_LIT);
    solver.clause[0] = literal::not(uip); // POKE_STACK (solver->clause, 0, NOT (uip))
    if !solver.probing {
        // ADD (literals_deduced, SIZE_STACK (solver->clause)): METRIC — no-op.
    }
    profile::stop_checked(solver, Prof::deduce); // STOP (deduce)
    None
}
