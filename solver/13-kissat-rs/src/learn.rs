// Port of src/learn.c (kissat 4.0.4).
//
// PORT NOTES:
//  - kissat_assign_reference in C receives the dereferenced clause pointer
//    alongside the reference; the Rust assign::assign_reference re-derives
//    the clause from the reference (crate::assign convention).
//  - ADD (literals_learned, size) is METRIC — no-op.  UPDATE_AVERAGE (size,
//    size) is `#ifndef QUIET` — kept.
//  - eagerly_subsume_last_learned ports the C quirk exactly: in
//    `if (marks[lit] && !--needed) break; else if (--remain < needed) break;`
//    the `else` arm runs (and decrements `remain`) whenever the first
//    condition is false — including for marked literals whose `--needed`
//    stayed non-zero.
//  - INC (eagerly_subsumed) is STATISTIC-tier: the field exists in the Rust
//    Statistics struct (never printed), incremented per crate policy.

use crate::internal::Solver;
use crate::literal;
use crate::reference::{Reference, INVALID_REF};

// C static `backjump_limit`.
fn backjump_limit(solver: &Solver) -> u32 {
    if solver.options.chrono != 0 {
        solver.options.chronolevels as u32
    } else {
        u32::MAX
    }
}

/// Port of `kissat_determine_new_level` — the chronological-backtracking
/// decision: jump non-chronologically unless the backjump distance exceeds
/// `chronolevels` (option `chrono`), in which case backtrack by one level.
pub fn determine_new_level(solver: &mut Solver, jump: u32) -> u32 {
    debug_assert!(solver.level != 0);
    let back = solver.level - 1;
    debug_assert!(jump <= back);

    let delta = back - jump;
    let limit = backjump_limit(solver);

    if delta == 0 {
        jump
    } else if delta > limit {
        solver.statistics.chronological += 1; // INC (chronological)
        back
    } else {
        jump
    }
}

// C static `learn_unit`.
fn learn_unit(solver: &mut Solver, not_uip: u32) {
    debug_assert!(not_uip == solver.clause[0]);
    let new_level = determine_new_level(solver, 0);
    crate::backtrack::backtrack_after_conflict(solver, new_level);
    crate::assign::learned_unit(solver, not_uip);
    if !solver.probing {
        solver.iterating = true;
        solver.statistics.iterations += 1; // INC (iterations)
    }
}

// C static `learn_binary`.
fn learn_binary(solver: &mut Solver, not_uip: u32) {
    let other = solver.clause[1]; // PEEK_STACK (solver->clause, 1)
    let jump_level = solver.assigned[literal::idx(other) as usize].level; // LEVEL (other)
    let new_level = determine_new_level(solver, jump_level);
    crate::backtrack::backtrack_after_conflict(solver, new_level);
    let ref_ = crate::clause::new_redundant_clause(solver, 1);
    debug_assert!(ref_ == INVALID_REF);
    let _ = ref_;
    crate::assign::assign_binary(solver, not_uip, other);
}

// C static `insert_last_learned`.
fn insert_last_learned(solver: &mut Solver, ref_: Reference) {
    let end = solver.options.eagersubsume as usize;
    let mut prev = ref_;
    for p in 0..end {
        let tmp = solver.last_learned[p];
        solver.last_learned[p] = prev;
        prev = tmp;
    }
}

// C static `learn_reference`.
fn learn_reference(solver: &mut Solver, not_uip: u32, glue: u32) -> Reference {
    debug_assert!(solver.level > 1);
    debug_assert!(solver.clause.len() > 2);
    debug_assert!(solver.clause[0] == not_uip);

    let mut q = 1usize;
    let mut jump_lit = solver.clause[q];
    let mut jump_level = solver.assigned[literal::idx(jump_lit) as usize].level;
    let end = solver.clause.len();
    let backtrack_level = solver.level - 1;
    for p in 2..end {
        let lit = solver.clause[p];
        let idx = literal::idx(lit);
        let level = solver.assigned[idx as usize].level;
        if jump_level >= level {
            continue;
        }
        jump_level = level;
        jump_lit = lit;
        q = p;
        if level == backtrack_level {
            break;
        }
    }
    solver.clause[q] = solver.clause[1]; // *q = lits[1]
    solver.clause[1] = jump_lit; // lits[1] = jump_lit

    let ref_ = crate::clause::new_redundant_clause(solver, glue);
    debug_assert!(ref_ != INVALID_REF);
    solver
        .arena
        .clause_mut(ref_)
        .set_used(crate::clause::MAX_USED); // c->used = MAX_USED
    let new_level = determine_new_level(solver, jump_level);
    crate::backtrack::backtrack_after_conflict(solver, new_level);
    crate::assign::assign_reference(solver, not_uip, ref_);
    ref_
}

/// Port of `kissat_update_learned`.
pub fn update_learned(solver: &mut Solver, glue: u32, size: u32) {
    debug_assert!(!solver.probing);
    solver.statistics.clauses_learned += 1; // INC (clauses_learned)
    if solver.stable {
        crate::reluctant::tick_reluctant(&mut solver.reluctant);
    }
    // ADD (literals_learned, size): METRIC — no-op.
    let stable = solver.stable as usize;
    // UPDATE_AVERAGE (size, size)  (#ifndef QUIET — kept):
    crate::smooth::update_smooth(&mut solver.averages[stable].size, size as f64);
    // UPDATE_AVERAGE (fast_glue, glue) / (slow_glue, glue):
    crate::smooth::update_smooth(&mut solver.averages[stable].fast_glue, glue as f64);
    crate::smooth::update_smooth(&mut solver.averages[stable].slow_glue, glue as f64);
}

// C static `flush_last_learned`.
fn flush_last_learned(solver: &mut Solver) {
    let end = solver.options.eagersubsume as usize;
    let mut q = 0usize;
    for p in 0..end {
        let ref_ = solver.last_learned[p];
        if ref_ != INVALID_REF {
            solver.last_learned[q] = ref_;
            q += 1;
        }
    }
    while q != end {
        solver.last_learned[q] = INVALID_REF;
        q += 1;
    }
}

// C static `eagerly_subsume_last_learned`.
fn eagerly_subsume_last_learned(solver: &mut Solver) {
    for i in 0..solver.clause.len() {
        let lit = solver.clause[i];
        debug_assert!(solver.marks[lit as usize] == 0);
        solver.marks[lit as usize] = 1;
    }
    let clause_size = solver.clause.len() as u32;
    let mut subsumed = 0u32;
    let end = solver.options.eagersubsume as usize;
    let mut p = 0usize;
    while p != end {
        let ref_ = solver.last_learned[p];
        p += 1;
        if ref_ == INVALID_REF {
            continue;
        }
        let (garbage, redundant, c_size) = {
            let c = solver.arena.clause(ref_);
            (c.garbage(), c.redundant(), c.size())
        };
        if garbage {
            continue;
        }
        if !redundant {
            continue;
        }
        if c_size <= clause_size {
            continue;
        }
        let mut needed = clause_size;
        let mut remain = c_size;
        {
            let c = solver.arena.clause(ref_);
            for &lit in c.lits() {
                if solver.marks[lit as usize] != 0 && {
                    needed -= 1;
                    needed == 0
                } {
                    break;
                } else {
                    remain -= 1;
                    if remain < needed {
                        break;
                    }
                }
            }
        }
        if needed != 0 {
            continue;
        }
        crate::clause::mark_clause_as_garbage(solver, ref_);
        solver.last_learned[p - 1] = INVALID_REF;
        subsumed += 1;
        solver.statistics.eagerly_subsumed += 1; // INC (eagerly_subsumed): STATISTIC
    }
    for i in 0..solver.clause.len() {
        let lit = solver.clause[i];
        solver.marks[lit as usize] = 0;
    }
    if subsumed != 0 {
        flush_last_learned(solver);
    }
}

/// Port of `kissat_learn_clause`.
pub fn learn_clause(solver: &mut Solver) {
    let not_uip = solver.clause[0]; // PEEK_STACK (solver->clause, 0)
    let size = solver.clause.len() as u32;
    let glue = solver.levels.len() as u32; // SIZE_STACK (solver->levels)
    if !solver.probing {
        update_learned(solver, glue, size);
    }
    debug_assert!(size > 0);
    let mut ref_ = INVALID_REF;
    if size == 1 {
        learn_unit(solver, not_uip);
    } else if size == 2 {
        learn_binary(solver, not_uip);
    } else {
        ref_ = learn_reference(solver, not_uip, glue);
    }
    if solver.options.eagersubsume != 0 {
        eagerly_subsume_last_learned(solver);
        if ref_ != INVALID_REF {
            insert_last_learned(solver, ref_);
        }
    }
}
