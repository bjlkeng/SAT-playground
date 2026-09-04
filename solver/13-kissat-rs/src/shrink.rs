// Port of src/shrink.c (kissat 4.0.4) — Feng/Biere all-UIP shrinking.
//
// PORT NOTES:
//  - The C block pointers (begin_block / end_block into solver->clause) are
//    usize indices here; all reads and writes go through solver.clause[i]
//    in the identical order.
//  - The trail walk in shrink_block uses an isize cursor because the C
//    pointer `t` may be decremented past the block's lowest trail position
//    only after the loop has already terminated (the do-while reads before
//    the check); the port reads solver.trail[t] then decrements, exactly as
//    `uip = *t--`.
//  - shrink_literal calls the public kissat_minimize_literal (crate::
//    minimize::minimize_literal) when option shrink > 2, corner case: this
//    can push poisoned/removable entries mid-shrink; kissat_reset_poisoned
//    at the end of kissat_shrink_clause clears the poisoned ones, matching
//    the C lifetime.
//  - ADD (literals_shrunken / literals_minshrunken) are METRIC — no-ops.

use crate::internal::{Solver, DECISION_REASON, INVALID_LEVEL};
use crate::literal::{self, INVALID_LIT};
use crate::profile::{self, Prof};
use crate::reference::Reference;

// C static `reset_shrinkable`.
fn reset_shrinkable(solver: &mut Solver) {
    while let Some(idx) = solver.shrinkable.pop() {
        debug_assert!(solver.assigned[idx as usize].shrinkable());
        solver.assigned[idx as usize].set_shrinkable(false);
    }
}

// C static `mark_shrinkable_as_removable`.
fn mark_shrinkable_as_removable(solver: &mut Solver) {
    while let Some(idx) = solver.shrinkable.pop() {
        debug_assert!(solver.assigned[idx as usize].shrinkable());
        solver.assigned[idx as usize].set_shrinkable(false);
        debug_assert!(!solver.assigned[idx as usize].poisoned());
        if solver.assigned[idx as usize].removable() {
            continue;
        }
        crate::inline::push_removable(solver, idx);
    }
}

// C static inline `shrink_literal`.
fn shrink_literal(solver: &mut Solver, level: u32, lit: u32) -> i32 {
    debug_assert!(solver.values[lit as usize] < 0);

    let idx = literal::idx(lit);
    let a = solver.assigned[idx as usize];
    debug_assert!(a.level <= level);
    if a.level == 0 {
        return 0; // root level assigned
    }
    if a.shrinkable() {
        return 0; // already shrinkable
    }
    if a.level < level {
        if a.removable() {
            return 0; // removable thus shrinkable
        }
        let always_minimize_on_lower_level = solver.options.shrink > 2;
        if always_minimize_on_lower_level && crate::minimize::minimize_literal(solver, lit, false)
        {
            return 0; // minimized thus shrinkable
        }
        return -1; // lower level, not removable/shrinkable
    }
    solver.assigned[idx as usize].set_shrinkable(true);
    solver.shrinkable.push(idx);
    1
}

// C static inline `shrunken_block`.
fn shrunken_block(
    solver: &mut Solver,
    _level: u32,
    begin_block: usize,
    end_block: usize,
    uip: u32,
) -> u32 {
    debug_assert!(uip != INVALID_LIT);
    let not_uip = literal::not(uip);

    debug_assert!(begin_block < end_block);

    let mut block_shrunken = 0u32;

    for p in begin_block..end_block {
        let lit = solver.clause[p];
        if lit == INVALID_LIT {
            continue;
        }
        solver.clause[p] = INVALID_LIT;
        block_shrunken += 1;
    }
    solver.clause[begin_block] = not_uip; // *begin_block = not_uip
    debug_assert!(block_shrunken != 0);
    block_shrunken -= 1;

    let uip_idx = literal::idx(uip);
    if !solver.assigned[uip_idx as usize].analyzed() {
        crate::inline::push_analyzed(solver, uip_idx);
    }

    mark_shrinkable_as_removable(solver);
    block_shrunken
}

// C static inline `push_literals_of_block`.
fn push_literals_of_block(solver: &mut Solver, begin_block: usize, end_block: usize, level: u32) {
    for p in begin_block..end_block {
        let lit = solver.clause[p];
        if lit == INVALID_LIT {
            continue;
        }
        let tmp = shrink_literal(solver, level, lit);
        debug_assert!(tmp > 0);
        let _ = tmp;
    }
}

// C static inline `shrink_along_binary`.
fn shrink_along_binary(solver: &mut Solver, level: u32, _uip: u32, other: u32) -> u32 {
    debug_assert!(solver.values[other as usize] < 0);
    let tmp = shrink_literal(solver, level, other);
    (tmp > 0) as u32
}

// C static inline `shrink_along_large`.
fn shrink_along_large(
    solver: &mut Solver,
    level: u32,
    uip: u32,
    ref_: Reference,
    failed: &mut bool,
) -> u32 {
    let mut open = 0u32;
    if solver.options.minimizeticks != 0 {
        solver.statistics.search_ticks += 1; // INC (search_ticks)
    }
    let size = solver.arena.clause(ref_).size();
    for i in 0..size {
        let other = solver.arena.clause(ref_).lit(i);
        if other == uip {
            continue;
        }
        debug_assert!(solver.values[other as usize] < 0);
        let tmp = shrink_literal(solver, level, other);
        if tmp < 0 {
            *failed = true;
            break;
        }
        if tmp > 0 {
            open += 1;
        }
    }
    open
}

// C static inline `shrink_along_reason`.
fn shrink_along_reason(
    solver: &mut Solver,
    level: u32,
    uip: u32,
    resolve_large_clauses: bool,
    failed: &mut bool,
) -> u32 {
    let uip_idx = literal::idx(uip);
    let a = solver.assigned[uip_idx as usize];
    debug_assert!(a.shrinkable());
    debug_assert!(a.level == level);
    debug_assert!(a.reason != DECISION_REASON);
    if a.binary() {
        let other = a.reason;
        shrink_along_binary(solver, level, uip, other)
    } else {
        let ref_ = a.reason;
        if resolve_large_clauses {
            shrink_along_large(solver, level, uip, ref_, failed)
        } else {
            *failed = true;
            0
        }
    }
}

// C static inline `shrink_block`.
fn shrink_block(
    solver: &mut Solver,
    begin_block: usize,
    end_block: usize,
    level: u32,
    max_trail: u32,
) -> u32 {
    debug_assert!(level < solver.level);

    let mut open = (end_block - begin_block) as u32;

    push_literals_of_block(solver, begin_block, end_block, level);

    debug_assert!(solver.shrinkable.len() == open as usize);

    let resolve_large_clauses = solver.options.shrink > 1;
    let mut uip = INVALID_LIT;
    let mut failed = false;

    let mut t: isize = max_trail as isize;

    while !failed {
        // do uip = *t--; while (!assigned[IDX (uip)].shrinkable);
        loop {
            debug_assert!(t >= 0);
            uip = solver.trail[t as usize];
            t -= 1;
            if solver.assigned[literal::idx(uip) as usize].shrinkable() {
                break;
            }
        }
        if open == 1 {
            break;
        }
        open += shrink_along_reason(solver, level, uip, resolve_large_clauses, &mut failed);
        debug_assert!(open > 1);
        open -= 1;
    }

    let mut block_shrunken = 0;
    if failed {
        reset_shrinkable(solver);
    } else {
        block_shrunken = shrunken_block(solver, level, begin_block, end_block, uip);
    }

    block_shrunken
}

// C static `next_block` — returns (begin_block, level, max_trail).
fn next_block(solver: &Solver, begin_lits: usize, end_block: usize) -> (usize, u32, u32) {
    let mut level = INVALID_LEVEL;
    let mut max_trail = 0u32;

    let mut begin_block = end_block;

    while begin_lits < begin_block {
        let lit = solver.clause[begin_block - 1];
        debug_assert!(lit != INVALID_LIT);
        let idx = literal::idx(lit);
        let a = &solver.assigned[idx as usize];
        let lit_level = a.level;
        if level == INVALID_LEVEL {
            level = lit_level;
        } else {
            debug_assert!(lit_level >= level);
            if lit_level > level {
                break;
            }
        }
        begin_block -= 1;
        let trail = a.trail;
        if trail > max_trail {
            max_trail = trail;
        }
    }

    (begin_block, level, max_trail)
}

// C static `minimize_block`.
fn minimize_block(solver: &mut Solver, begin_block: usize, end_block: usize) -> u32 {
    let mut minimized = 0u32;

    for p in begin_block..end_block {
        let lit = solver.clause[p];
        debug_assert!(lit != INVALID_LIT);
        if !crate::minimize::minimize_literal(solver, lit, true) {
            continue;
        }
        solver.clause[p] = INVALID_LIT;
        minimized += 1;
    }

    minimized
}

// C static inline `minimize_and_shrink_block` — returns the new end_block
// (the found begin_block).
fn minimize_and_shrink_block(
    solver: &mut Solver,
    begin_lits: usize,
    end_block: usize,
    total_shrunken: &mut u32,
    total_minimized: &mut u32,
) -> usize {
    debug_assert!(solver.shrinkable.is_empty());

    let (begin_block, level, max_trail) = next_block(solver, begin_lits, end_block);

    let open = end_block - begin_block;
    debug_assert!(open > 0);

    let mut block_shrunken = 0u32;
    let mut block_minimized = 0u32;

    if open < 2 {
        // only one literal on this level
    } else {
        block_shrunken = shrink_block(solver, begin_block, end_block, level, max_trail);
        if block_shrunken == 0 {
            block_minimized = minimize_block(solver, begin_block, end_block);
        }
    }

    block_shrunken += block_minimized;

    *total_minimized += block_minimized;
    *total_shrunken += block_shrunken;

    begin_block
}

/// Port of `kissat_shrink_clause`.
pub fn shrink_clause(solver: &mut Solver) {
    debug_assert!(solver.options.minimize > 0);
    debug_assert!(solver.options.shrink > 0);
    debug_assert!(!solver.clause.is_empty());

    profile::start_checked(solver, Prof::shrink);

    let mut total_shrunken = 0u32;
    let mut total_minimized = 0u32;

    let begin_lits = 0usize;
    let end_lits = solver.clause.len();

    let mut end_block = solver.clause.len();

    while end_block != begin_lits {
        end_block = minimize_and_shrink_block(
            solver,
            begin_lits,
            end_block,
            &mut total_shrunken,
            &mut total_minimized,
        );
    }

    let mut q = begin_lits;
    for p in q..end_lits {
        let lit = solver.clause[p];
        if lit != INVALID_LIT {
            solver.clause[q] = lit;
            q += 1;
        }
    }
    debug_assert!(q + total_shrunken as usize == end_lits);
    solver.clause.truncate(q); // SET_END_OF_STACK (solver->clause, q)

    // ADD (literals_shrunken, total_shrunken): METRIC — no-op.
    // ADD (literals_minshrunken, total_minimized): METRIC — no-op.
    let _ = total_minimized;

    crate::minimize::reset_poisoned(solver);

    profile::stop_checked(solver, Prof::shrink);
}
