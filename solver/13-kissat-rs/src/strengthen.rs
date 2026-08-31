// Port of src/strengthen.c (kissat 4.0.4).
//
// PORT NOTES:
//  - C `clause *c` handles become the `Conflict` handle (Clause(ref) for
//    arena clauses, Binary for &solver->conflict) matching deduce.rs.
//  - The fake binary conflict clause has an all-zero header apart from
//    size=2 (internal.c), so its `redundant` reads false here, as in C.
//  - C's binary_on_the_fly_strengthen `break` after finding the second
//    literal is `#ifndef NDEBUG` only — the release build scans the whole
//    clause; ported without break (antecedent_size == 3 guarantees exactly
//    two non-root-level literals, so the result is identical).
//  - INC (on_the_fly_strengthened/subsumed) are STATISTIC-tier: kept as
//    real (never-printed) fields per statistics.rs policy.

use crate::internal::Solver;
use crate::literal::INVALID_LIT;
use crate::propsearch::Conflict;
use crate::reference::Reference;

// static large_on_the_fly_strengthen
fn large_on_the_fly_strengthen(solver: &mut Solver, ref_: Reference, lit: u32) -> Conflict {
    debug_assert!(solver.antecedent_size > 3);
    solver.statistics.on_the_fly_strengthened += 1; // INC (on_the_fly_strengthened)
    {
        let mut c = solver.arena.clause_mut(ref_);
        let l0 = c.lit(0);
        debug_assert!(l0 == lit || c.lit(1) == lit);
        // if (lits[0] == lit) SWAP (unsigned, lits[0], lits[1]);
        if l0 == lit {
            let l1 = c.lit(1);
            c.set_lit(0, l1);
            c.set_lit(1, lit);
        }
    }
    crate::watch::unwatch_blocking(solver, lit, ref_);
    // SHRINK_CLAUSE_IN_PROOF (c, lit, lits[0]);
    if solver.proof.is_some() {
        let keep = solver.arena.clause(ref_).lit(0);
        crate::proof::shrink_clause_in_proof(solver, ref_, lit, keep);
    }
    // CHECK_SHRINK_CLAUSE: compiled out (NDEBUG).
    {
        let old_size = solver.arena.clause(ref_).size();
        let irredundant = !solver.arena.clause(ref_).redundant();
        let mut new_size: u32 = 1;
        for i in 2..old_size {
            let other = solver.arena.clause(ref_).lit(i);
            debug_assert!(solver.values[other as usize] < 0);
            if solver.assigned[crate::literal::idx(other) as usize].level == 0 {
                continue;
            }
            solver.arena.clause_mut(ref_).set_lit(new_size, other);
            new_size += 1;
            if irredundant {
                crate::inline::mark_added_literal(solver, other);
            }
        }
        debug_assert!(new_size > 2);
        {
            let mut c = solver.arena.clause_mut(ref_);
            c.set_size(new_size);
            c.set_searched(2);
        }
        let (redundant, glue) = {
            let c = solver.arena.clause(ref_);
            (c.redundant(), c.glue())
        };
        if redundant && glue >= new_size {
            crate::promote::promote_clause(solver, ref_, new_size - 1);
        }
        if !solver.arena.clause(ref_).shrunken() {
            let mut c = solver.arena.clause_mut(ref_);
            c.set_shrunken(true);
            c.set_lit(old_size - 1, INVALID_LIT);
        }
    }
    {
        // Move the highest-level literal to position 1 and rewatch.
        let size = solver.arena.clause(ref_).size();
        let l1 = solver.arena.clause(ref_).lit(1);
        debug_assert!(solver.values[l1 as usize] < 0);
        let mut highest_pos: u32 = 1;
        let mut highest_level = solver.assigned[crate::literal::idx(l1) as usize].level;
        for i in 2..size {
            let other = solver.arena.clause(ref_).lit(i);
            debug_assert!(solver.values[other as usize] < 0);
            let level = solver.assigned[crate::literal::idx(other) as usize].level;
            if level <= highest_level {
                continue;
            }
            highest_pos = i;
            highest_level = level;
        }
        if highest_pos != 1 {
            let mut c = solver.arena.clause_mut(ref_);
            let a = c.lit(1);
            let b = c.lit(highest_pos);
            c.set_lit(1, b);
            c.set_lit(highest_pos, a);
        }
        let (l0, l1) = {
            let c = solver.arena.clause(ref_);
            (c.lit(0), c.lit(1))
        };
        crate::watch::watch_blocking(solver, l1, l0, ref_);
    }
    {
        // Update the blocking literal of lits[0]'s large watch on ref.
        let (l0, l1) = {
            let c = solver.arena.clause(ref_);
            (c.lit(0), c.lit(1))
        };
        debug_assert!(solver.watching);
        let v = solver.watches[l0 as usize];
        let mut p = v.begin;
        loop {
            debug_assert!(p != v.end);
            let head = solver.vectors.stack[p];
            p += 1;
            if crate::watch::watch_is_binary(head) {
                continue;
            }
            debug_assert!(p != v.end);
            let tail = solver.vectors.stack[p];
            p += 1;
            if crate::watch::watch_ref(tail) == ref_ {
                break;
            }
        }
        // p[-2].blocking.lit = lits[1] (blocking watch word == plain lit).
        solver.vectors.stack[p - 2] = crate::watch::blocking_watch(l1);
    }
    Conflict::Clause(ref_)
}

// static binary_on_the_fly_strengthen
fn binary_on_the_fly_strengthen(solver: &mut Solver, ref_: Reference, lit: u32) -> Conflict {
    debug_assert!(solver.antecedent_size == 3);
    let mut first = INVALID_LIT;
    let mut second = INVALID_LIT;
    let size = solver.arena.clause(ref_).size();
    for i in 0..size {
        let other = solver.arena.clause(ref_).lit(i);
        if other == lit {
            continue;
        }
        debug_assert!(solver.values[other as usize] < 0);
        if solver.assigned[crate::literal::idx(other) as usize].level == 0 {
            continue;
        }
        if first == INVALID_LIT {
            first = other;
        } else {
            second = other;
        }
    }
    debug_assert!(second != INVALID_LIT);
    crate::clause::new_binary_clause(solver, first, second);
    let (l0, l1) = {
        let c = solver.arena.clause(ref_);
        (c.lit(0), c.lit(1))
    };
    crate::watch::unwatch_blocking(solver, l0, ref_);
    crate::watch::unwatch_blocking(solver, l1, ref_);
    crate::clause::mark_clause_as_garbage(solver, ref_);
    crate::propsearch::binary_conflict(solver, first, second)
}

/// Port of `kissat_on_the_fly_strengthen`.
pub fn on_the_fly_strengthen(solver: &mut Solver, ref_: Reference, lit: u32) -> Conflict {
    debug_assert!(!solver.arena.clause(ref_).garbage());
    debug_assert!(solver.antecedent_size > 2);
    if !solver.arena.clause(ref_).redundant() {
        crate::inline::mark_removed_literal(solver, lit);
    }
    if solver.antecedent_size == 3 {
        binary_on_the_fly_strengthen(solver, ref_, lit)
    } else {
        large_on_the_fly_strengthen(solver, ref_, lit)
    }
}

/// Port of `kissat_on_the_fly_subsume`.
pub fn on_the_fly_subsume(solver: &mut Solver, c: Conflict, d: Conflict) {
    let d_ref = match d {
        Conflict::Clause(r) => r,
        // C asserts c != d and c->size <= d->size with conflict_size > 2 at
        // the only call site, so d is always a real arena clause.
        Conflict::Binary => unreachable!("on_the_fly_subsume: d must be an arena clause"),
    };
    debug_assert!(!solver.arena.clause(d_ref).garbage());
    let (c_size, c_redundant, c_glue) = match c {
        // Fake binary conflict clause header: size 2, redundant bit clear.
        Conflict::Binary => (2u32, false, 0u32),
        Conflict::Clause(r) => {
            let cl = solver.arena.clause(r);
            (cl.size(), cl.redundant(), cl.glue())
        }
    };
    debug_assert!(c_size > 1);
    debug_assert!(c_size <= solver.arena.clause(d_ref).size());
    let d_redundant = solver.arena.clause(d_ref).redundant();
    let d_glue = solver.arena.clause(d_ref).glue();
    crate::clause::mark_clause_as_garbage(solver, d_ref);
    solver.statistics.on_the_fly_subsumed += 1; // INC (on_the_fly_subsumed)
    if d_redundant {
        if c_redundant && c_glue > d_glue {
            if let Conflict::Clause(c_ref) = c {
                crate::promote::promote_clause(solver, c_ref, d_glue);
            }
        }
        return;
    }
    if !c_redundant {
        return;
    }
    let c_ref = match c {
        Conflict::Clause(r) => r,
        Conflict::Binary => unreachable!(), // c_redundant is false for Binary
    };
    if c_size > 2 {
        solver.arena.clause_mut(c_ref).set_redundant(false);
        crate::collect::update_last_irredundant(solver, c_ref);
    }
    if c_size > 2 {
        debug_assert!(solver.statistics.clauses_irredundant < u64::MAX);
        solver.statistics.clauses_irredundant += 1;
    } else {
        debug_assert!(solver.statistics.clauses_binary < u64::MAX);
        solver.statistics.clauses_binary += 1;
    }
    debug_assert!(solver.statistics.clauses_redundant > 0);
    solver.statistics.clauses_redundant -= 1;
}
