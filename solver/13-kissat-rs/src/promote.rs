// Port of src/promote.c + the promote.h inline kissat_recompute_glue
// (kissat 4.0.4).
//
// PORT NOTES:
//  - All INC counters here (clauses_kept1/2/3, clauses_promoted1/2,
//    clauses_improved) are STATISTIC-tier fields (kept, never printed).
//  - C functions take `clause *c`; the port takes the clause Reference.
//  - TIER1/TIER2 macros are solver.tier1()/solver.tier2() (the internal.rs
//    quirk-exact accessors).

use crate::internal::Solver;
use crate::reference::Reference;

/// Port of `kissat_promote_clause`.
pub fn promote_clause(solver: &mut Solver, ref_: Reference, new_glue: u32) {
    if solver.options.promote == 0 {
        return;
    }
    let old_glue = {
        let c = solver.arena.clause(ref_);
        debug_assert!(c.redundant());
        c.glue()
    };
    debug_assert!(new_glue < old_glue);
    let tier1 = solver.tier1();
    let tier2 = solver.tier2().max(tier1); // MAX (TIER2, TIER1)
    if old_glue <= tier1 {
        solver.statistics.clauses_kept1 += 1; // INC (clauses_kept1)
    } else if new_glue <= tier1 {
        debug_assert!(tier1 < old_glue);
        solver.statistics.clauses_promoted1 += 1; // INC (clauses_promoted1)
    } else if tier2 < old_glue && new_glue <= tier2 {
        solver.statistics.clauses_promoted2 += 1; // INC (clauses_promoted2)
    } else if old_glue <= tier2 {
        debug_assert!(tier1 < old_glue);
        debug_assert!(tier1 < new_glue && new_glue <= tier2);
        solver.statistics.clauses_kept2 += 1; // INC (clauses_kept2)
    } else {
        debug_assert!(tier2 < old_glue);
        debug_assert!(tier2 < new_glue);
        solver.statistics.clauses_kept3 += 1; // INC (clauses_kept3)
    }
    solver.statistics.clauses_improved += 1; // INC (clauses_improved)
    solver.arena.clause_mut(ref_).set_glue(new_glue);
}

/// Port of promote.h's inline `kissat_recompute_glue`.
pub fn recompute_glue(solver: &mut Solver, ref_: Reference, limit: u32) -> u32 {
    debug_assert!(limit > 0);
    debug_assert!(solver.promote.is_empty());
    let mut res: u32 = 0;
    for &lit in solver.arena.clause(ref_).lits() {
        debug_assert!(solver.values[lit as usize] != 0);
        let level = solver.assigned[crate::literal::idx(lit) as usize].level; // LEVEL (lit)
        if solver.frames[level as usize].promote {
            continue;
        }
        res += 1;
        if res == limit {
            break;
        }
        solver.frames[level as usize].promote = true;
        solver.promote.push(level);
    }
    for i in 0..solver.promote.len() {
        let level = solver.promote[i];
        debug_assert!(solver.frames[level as usize].promote);
        solver.frames[level as usize].promote = false;
    }
    solver.promote.clear();
    res
}
