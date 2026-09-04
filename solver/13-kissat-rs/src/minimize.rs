// Port of src/minimize.c (kissat 4.0.4).
//
// PORT NOTES:
//  - C has both the *static* `minimize_literal` (the recursive worker) and
//    the public `kissat_minimize_literal`; after the kissat_ prefix drop
//    they would collide, so the static worker is `minimize_literal_rec`
//    here and the public entry keeps the name `minimize_literal`.
//  - The `assigned *` / `frame *` array arguments C threads through are
//    always solver->assigned / solver->frames; folded into direct field
//    access (identical reads/writes, same order).
//  - Recursion depth is capped by option `minimizedepth` exactly as in C
//    (same recursion shape).
//  - ADD (literals_minimized, ..) is METRIC — no-op.

use crate::internal::{Solver, DECISION_REASON, UNIT_REASON};
use crate::literal::{self, INVALID_LIT};
use crate::profile::{self, Prof};

// C static inline `minimized_index` (lit argument is LOG-only; the C
// `assigned *a` parameter is re-derived from idx).
#[inline]
fn minimized_index(solver: &Solver, minimizing: bool, idx: u32, depth: u32) -> i32 {
    let a = &solver.assigned[idx as usize];
    if a.level == 0 {
        return 1; // root level literal
    }
    if a.removable() && depth != 0 {
        return 1; // already removable
    }
    debug_assert!(a.reason != UNIT_REASON);
    if a.reason == DECISION_REASON {
        return -1; // can not remove decision literal
    }
    if a.poisoned() {
        return -1; // can not remove poisoned literal
    }
    if minimizing || depth == 0 {
        let frame = &solver.frames[a.level as usize];
        if frame.used <= 1 {
            return -1; // singleton frame literal
        }
    }
    0
}

// C static inline `minimize_reference`.
fn minimize_reference(
    solver: &mut Solver,
    minimizing: bool,
    ref_: crate::reference::Reference,
    lit: u32,
    depth: u32,
) -> bool {
    let next_depth = if depth == u32::MAX { depth } else { depth + 1 };
    let not_lit = literal::not(lit);
    if solver.options.minimizeticks != 0 {
        solver.statistics.search_ticks += 1; // INC (search_ticks)
    }
    let size = solver.arena.clause(ref_).size();
    for i in 0..size {
        let other = solver.arena.clause(ref_).lit(i);
        if other != not_lit
            && !minimize_literal_rec(solver, minimizing, other, next_depth)
        {
            return false;
        }
    }
    true
}

// C static inline `minimize_binary` — walks the binary-reason chain
// iteratively, caching visited indices on solver->minimize until the
// chain's fate is known, then marks them all removable or poisoned.
fn minimize_binary(solver: &mut Solver, minimizing: bool, lit: u32, depth: u32) -> bool {
    let saved = solver.minimize.len();
    let res;
    let mut next = lit;
    loop {
        let next_idx = literal::idx(next);
        let tmp = minimized_index(solver, minimizing, next_idx, 1);
        if tmp != 0 {
            res = tmp > 0;
            break;
        }
        solver.minimize.push(next_idx);
        let a = solver.assigned[next_idx as usize];
        if !a.binary() {
            let next_depth = if depth == u32::MAX { depth } else { depth + 1 };
            res = minimize_reference(solver, minimizing, a.reason, next, next_depth);
            break;
        }
        next = a.reason;
    }
    if res {
        for i in saved..solver.minimize.len() {
            let idx = solver.minimize[i];
            crate::inline::push_removable(solver, idx);
        }
    } else {
        for i in saved..solver.minimize.len() {
            let idx = solver.minimize[i];
            crate::inline::push_poisoned(solver, idx);
        }
    }
    solver.minimize.truncate(saved); // SET_END_OF_STACK (solver->minimize, begin)
    res
}

// C static `minimize_literal` (renamed — see module PORT NOTE).
fn minimize_literal_rec(solver: &mut Solver, minimizing: bool, lit: u32, depth: u32) -> bool {
    debug_assert!(solver.values[lit as usize] < 0);
    debug_assert!(depth != 0 || solver.minimize.is_empty());
    debug_assert!(solver.options.minimizedepth > 0);
    if depth >= solver.options.minimizedepth as u32 {
        return false;
    }
    let idx = literal::idx(lit);
    let tmp = minimized_index(solver, minimizing, idx, depth);
    if tmp > 0 {
        return true;
    }
    if tmp < 0 {
        return false;
    }
    let a = solver.assigned[idx as usize];
    let res = if a.binary() {
        minimize_binary(solver, minimizing, a.reason, depth)
    } else {
        minimize_reference(solver, minimizing, a.reason, lit, depth)
    };
    if depth == 0 {
        return res;
    }
    if !res {
        crate::inline::push_poisoned(solver, idx);
    } else if !solver.assigned[idx as usize].removable() {
        crate::inline::push_removable(solver, idx);
    }
    res
}

/// Port of `kissat_minimize_literal`.
pub fn minimize_literal(solver: &mut Solver, lit: u32, lit_in_clause: bool) -> bool {
    debug_assert!(solver.minimize.is_empty());
    minimize_literal_rec(solver, false, lit, if lit_in_clause { 0 } else { 1 })
}

/// Port of `kissat_reset_poisoned`.
pub fn reset_poisoned(solver: &mut Solver) {
    for i in 0..solver.poisoned.len() {
        let idx = solver.poisoned[i];
        debug_assert!(idx < solver.vars());
        debug_assert!(solver.assigned[idx as usize].poisoned());
        solver.assigned[idx as usize].set_poisoned(false);
    }
    solver.poisoned.clear();
}

/// Port of `kissat_minimize_clause`.
pub fn minimize_clause(solver: &mut Solver) {
    profile::start_checked(solver, Prof::minimize);

    debug_assert!(solver.minimize.is_empty());
    debug_assert!(solver.removable.is_empty());
    debug_assert!(solver.poisoned.is_empty());
    debug_assert!(!solver.clause.is_empty());

    let end = solver.clause.len();

    for p in 0..end {
        let idx = literal::idx(solver.clause[p]);
        crate::inline::push_removable(solver, idx);
    }

    if solver.options.shrink > 2 {
        profile::stop_checked(solver, Prof::minimize);
        return;
    }

    let mut minimized = 0u32;

    // for (unsigned *p = end; --p > lits;) — indices end-1 down to 1.
    for p in (1..end).rev() {
        let lit = solver.clause[p];
        if minimize_literal_rec(solver, true, lit, 0) {
            solver.clause[p] = INVALID_LIT;
            minimized += 1;
        }
    }

    let mut q = 0usize;
    for p in 0..end {
        let lit = solver.clause[p];
        if lit != INVALID_LIT {
            solver.clause[q] = lit;
            q += 1;
        }
    }
    debug_assert!(q + minimized as usize == end);
    solver.clause.truncate(q); // SET_END_OF_STACK (solver->clause, q)

    debug_assert!(!solver.probing);
    // ADD (literals_minimized, minimized): METRIC — no-op.
    let _ = minimized;

    reset_poisoned(solver);

    profile::stop_checked(solver, Prof::minimize);
}
