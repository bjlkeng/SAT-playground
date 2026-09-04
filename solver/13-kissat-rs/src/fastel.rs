// Port of src/fastel.c (kissat 4.0.4).
//
// Fast bounded variable elimination during preprocessing.
//
// PORT NOTE (quirk ported): in kissat_fast_variable_elimination the C code
// computes `const unsigned not_lit = LIT (pivot);` — NOT `NOT (LIT (pivot))`
// — so both flush_occurrences calls in candidate gathering use the POSITIVE
// literal and `score = pos + neg` is really twice the positive count.
// PORT NOTE (quirk ported): in flush_occurrences the satisfied-literal check
// inside the clause-literal loop does `q--; continue;` PER satisfied literal
// (the `continue` targets the inner literal loop), so a clause with two
// satisfied literals is marked garbage twice and drops two watch slots;
// ported exactly.
// PORT NOTE: fast_subsumed / fast_eliminated / fast_strengthened /
// eliminated / subsumed / strengthened are COUNTERs (real);
// eliminate_units is STATISTIC (kept as real, never-printed field).
// PORT NOTE: RADIX_STACK is wrapped in START/STOP (radix) per the sort.rs
// convention.

use crate::internal::{Solver, INVALID};
use crate::profile::Prof;
use crate::reference::Reference;
use crate::terminated;
use crate::watch::{watch_is_binary, watch_lit, watch_ref, Watch};

// static bool fast_forward_subsumed (kissat *, clause *c)
fn fast_forward_subsumed(solver: &mut Solver, c_ref: Reference) -> bool {
    debug_assert!(!solver.arena.clause(c_ref).garbage());
    debug_assert!(!solver.arena.clause(c_ref).redundant());
    let mut max_occurring = INVALID;
    let mut max_occurrence: usize = 0;
    let c_size = solver.arena.clause(c_ref).size();
    for &other in solver.arena.clause(c_ref).lits() {
        let other_idx = crate::literal::idx(other);
        if !solver.flags[other_idx as usize].active {
            continue;
        }
        let other_occurrence = solver.watches[other as usize].size();
        if other_occurrence <= max_occurrence {
            continue;
        }
        max_occurrence = other_occurrence;
        max_occurring = other;
        solver.marks[other as usize] = 1;
    }
    let mut subsumed = false;
    let fasteloccs = solver.options.fasteloccs as usize;
    'outer: for i in 0..c_size {
        let other = solver.arena.clause(c_ref).lit(i);
        if other == max_occurring {
            continue;
        }
        let other_idx = crate::literal::idx(other);
        if !solver.flags[other_idx as usize].active {
            continue;
        }
        let v = solver.watches[other as usize];
        let size_other_watches = v.size();
        if size_other_watches > fasteloccs {
            continue;
        }
        for p in v.begin..v.end {
            let watch = solver.vectors.stack[p];
            if watch_is_binary(watch) {
                let other2 = watch_lit(watch); // watch.type.lit
                if solver.marks[other2 as usize] != 0 {
                    subsumed = true;
                    break;
                }
            } else {
                let d_ref = watch_ref(watch);
                if d_ref == c_ref {
                    continue;
                }
                if solver.arena.clause(d_ref).garbage() {
                    continue;
                }
                if solver.arena.clause(d_ref).size() > solver.arena.clause(c_ref).size() {
                    continue;
                }
                debug_assert!(!solver.arena.clause(d_ref).redundant());
                subsumed = true;
                for &other2 in solver.arena.clause(d_ref).lits() {
                    if solver.values[other2 as usize] < 0 {
                        continue;
                    }
                    if solver.marks[other2 as usize] == 0 {
                        subsumed = false;
                        break;
                    }
                }
            }
        }
        if subsumed {
            break 'outer;
        }
    }
    for &other in solver.arena.clause(c_ref).lits() {
        solver.marks[other as usize] = 0;
    }
    if subsumed {
        crate::clause::mark_clause_as_garbage(solver, c_ref);
        solver.statistics.subsumed += 1; // INC (subsumed)
        solver.statistics.fast_subsumed += 1; // INC (fast_subsumed)
    }
    subsumed
}

// static size_t flush_occurrences (kissat *, unsigned lit)
fn flush_occurrences(solver: &mut Solver, lit: u32) -> usize {
    let fasteloccs = solver.options.fasteloccs as usize;
    let fastelclslim = solver.options.fastelclslim as usize;
    let fastelsub = solver.options.fastelsub;
    let v = solver.watches[lit as usize];
    let begin = v.begin;
    let end = v.end;
    let mut q = begin;
    let mut p = begin;
    let mut res: usize = 0;
    while p != end {
        // const watch watch = *q++ = *p++;
        let watch = solver.vectors.stack[p];
        solver.vectors.stack[q] = watch;
        q += 1;
        p += 1;
        if watch_is_binary(watch) {
            let other = watch_lit(watch);
            if solver.values[other as usize] > 0 {
                continue; // satisfied: keep the watch, do not count it
            }
            let other_idx = crate::literal::idx(other);
            if solver.flags[other_idx as usize].eliminated {
                q -= 1;
                continue;
            }
        } else {
            let ref_ = watch_ref(watch);
            if solver.arena.clause(ref_).garbage() {
                q -= 1;
                continue;
            }
            if solver.arena.clause(ref_).size() as usize > fastelclslim {
                res = fasteloccs + 1;
                break;
            }
            let size = solver.arena.clause(ref_).size();
            for i in 0..size {
                let other = solver.arena.clause(ref_).lit(i);
                let other_value = solver.values[other as usize];
                if other_value > 0 {
                    // QUIRK ported: per-satisfied-literal garbage + q--
                    crate::clause::mark_clause_as_garbage(solver, ref_);
                    q -= 1;
                    continue;
                }
            }
            if fastelsub != 0 && fast_forward_subsumed(solver, ref_) {
                q -= 1;
                continue;
            }
        }
        res += 1;
        if res > fasteloccs {
            break;
        }
    }
    if q < p {
        while p != end {
            solver.vectors.stack[q] = solver.vectors.stack[p];
            q += 1;
            p += 1;
        }
        crate::vector::resize_vector(solver, lit, q - begin); // SET_END_OF_WATCHES
    }
    res
}

// static void do_fast_resolve_binary_binary (kissat *, unsigned pivot,
//                                            unsigned clit, unsigned dlit)
fn do_fast_resolve_binary_binary(solver: &mut Solver, _pivot: u32, clit: u32, dlit: u32) {
    debug_assert!(!solver.flags[crate::literal::idx(clit) as usize].eliminated);
    debug_assert!(!solver.flags[crate::literal::idx(dlit) as usize].eliminated);
    if clit == crate::literal::not(dlit) {
        return; // resolvent tautological
    }
    let cval = solver.values[clit as usize];
    if cval > 0 {
        return; // 1st antecedent satisfied
    }
    let dval = solver.values[dlit as usize];
    if dval > 0 {
        return; // 2nd antecedent satisfied
    }
    if cval < 0 && dval < 0 {
        debug_assert!(!solver.inconsistent);
        solver.inconsistent = true;
        // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
        if solver.proof.is_some() {
            crate::proof::add_empty_to_proof(solver);
        }
        return;
    }
    if cval < 0 {
        solver.statistics.eliminate_units += 1; // INC: STATISTIC kept
        crate::assign::learned_unit(solver, dlit);
        return;
    }
    if dval < 0 {
        solver.statistics.eliminate_units += 1;
        crate::assign::learned_unit(solver, clit);
        return;
    }
    if clit == dlit {
        solver.statistics.eliminate_units += 1;
        crate::assign::learned_unit(solver, clit);
        return;
    }
    debug_assert!(cval == 0);
    debug_assert!(dval == 0);
    debug_assert!(solver.clause.is_empty());
    solver.clause.push(clit);
    solver.clause.push(dlit);
    crate::clause::new_irredundant_clause(solver);
    solver.clause.clear();
}

// static void do_fast_resolve_binary_large (kissat *, unsigned pivot,
//                                           unsigned lit, clause *c)
fn do_fast_resolve_binary_large(solver: &mut Solver, pivot: u32, lit: u32, c_ref: Reference) {
    debug_assert!(!solver.flags[crate::literal::idx(lit) as usize].eliminated);
    if solver.arena.clause(c_ref).garbage() {
        return;
    }
    debug_assert!(!solver.arena.clause(c_ref).redundant());
    let lit_val = solver.values[lit as usize];
    if lit_val > 0 {
        return; // binary clause antecedent satisfied
    }
    debug_assert!(solver.clause.is_empty());
    if lit_val == 0 {
        solver.clause.push(lit);
    }
    let mut satisfied = false;
    let mut tautological = false;
    let not_lit = crate::literal::not(lit);
    for &other in solver.arena.clause(c_ref).lits() {
        let idx_other = crate::literal::idx(other);
        if idx_other == pivot {
            continue;
        }
        if other == lit {
            continue;
        }
        if other == not_lit {
            tautological = true;
            break;
        }
        let other_val = solver.values[other as usize];
        if other_val < 0 {
            continue;
        }
        if other_val > 0 {
            crate::clause::mark_clause_as_garbage(solver, c_ref);
            satisfied = true;
            break;
        }
        solver.clause.push(other);
    }
    if satisfied || tautological {
        solver.clause.clear();
        return;
    }
    let size = solver.clause.len();
    if size == 0 {
        debug_assert!(!solver.inconsistent);
        solver.inconsistent = true;
        // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
        if solver.proof.is_some() {
            crate::proof::add_empty_to_proof(solver);
        }
        return;
    }
    if size == 1 {
        let unit = solver.clause[0];
        solver.clause.clear();
        solver.statistics.eliminate_units += 1;
        crate::assign::learned_unit(solver, unit);
        return;
    }
    crate::clause::new_irredundant_clause(solver);
    solver.clause.clear();
}

// static void do_fast_resolve_large_large (kissat *, unsigned pivot,
//                                          clause *c, clause *d)
fn do_fast_resolve_large_large(
    solver: &mut Solver,
    pivot: u32,
    c_ref: Reference,
    d_ref: Reference,
) {
    if solver.arena.clause(c_ref).garbage() {
        return;
    }
    if solver.arena.clause(d_ref).garbage() {
        return;
    }
    debug_assert!(!solver.arena.clause(c_ref).redundant());
    debug_assert!(!solver.arena.clause(d_ref).redundant());
    debug_assert!(solver.clause.is_empty());
    let mut satisfied = false;
    let mut tautological = false;
    for &other in solver.arena.clause(c_ref).lits() {
        let idx_other = crate::literal::idx(other);
        if idx_other == pivot {
            continue;
        }
        let other_val = solver.values[other as usize];
        if other_val < 0 {
            continue;
        }
        if other_val > 0 {
            satisfied = true;
            break;
        }
        solver.clause.push(other);
        solver.marks[other as usize] = 1;
    }
    if satisfied || tautological {
        for i in 0..solver.clause.len() {
            let other = solver.clause[i];
            solver.marks[other as usize] = 0;
        }
        solver.clause.clear();
        return;
    }
    let marked = solver.clause.len();
    for &other in solver.arena.clause(d_ref).lits() {
        let idx_other = crate::literal::idx(other);
        if idx_other == pivot {
            continue;
        }
        let other_val = solver.values[other as usize];
        if other_val < 0 {
            continue;
        }
        if other_val > 0 {
            satisfied = true;
            break;
        }
        let mark_other = solver.marks[other as usize];
        if mark_other != 0 {
            continue;
        }
        let not_other = crate::literal::not(other);
        let mark_not_other = solver.marks[not_other as usize];
        if mark_not_other != 0 {
            tautological = true;
            break;
        }
        solver.clause.push(other);
    }
    if satisfied || tautological {
        for i in 0..solver.clause.len() {
            let other = solver.clause[i];
            solver.marks[other as usize] = 0;
        }
        solver.clause.clear();
        return;
    }
    let size = solver.clause.len();
    if size == 0 {
        debug_assert!(!solver.inconsistent);
        solver.inconsistent = true;
        // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
        if solver.proof.is_some() {
            crate::proof::add_empty_to_proof(solver);
        }
        return;
    }
    if size == 1 {
        let unit = solver.clause[0];
        solver.clause.clear();
        solver.marks[unit as usize] = 0;
        solver.statistics.eliminate_units += 1;
        crate::assign::learned_unit(solver, unit);
        return;
    }
    crate::clause::new_irredundant_clause(solver);
    solver.clause.truncate(marked); // RESIZE_STACK (*clause, marked)
    for i in 0..solver.clause.len() {
        let other = solver.clause[i];
        solver.marks[other as usize] = 0;
    }
    solver.clause.clear();
}

// static void do_fast_resolve (kissat *, unsigned pivot, watch c, watch d)
fn do_fast_resolve(solver: &mut Solver, pivot: u32, cwatch: Watch, dwatch: Watch) {
    debug_assert!(!solver.inconsistent);
    let clit = watch_lit(cwatch);
    let dlit = watch_lit(dwatch);
    let cref = watch_ref(cwatch);
    let dref = watch_ref(dwatch);
    let cbin = watch_is_binary(cwatch);
    let dbin = watch_is_binary(dwatch);
    if cbin && dbin {
        do_fast_resolve_binary_binary(solver, pivot, clit, dlit);
    } else if cbin && !dbin {
        do_fast_resolve_binary_large(solver, pivot, clit, dref);
    } else if !cbin && dbin {
        do_fast_resolve_binary_large(solver, pivot, dlit, cref);
    } else {
        debug_assert!(!cbin && !dbin);
        do_fast_resolve_large_large(solver, pivot, cref, dref);
    }
}

// static void fast_delete_and_weaken_clauses (kissat *, unsigned lit)
fn fast_delete_and_weaken_clauses(solver: &mut Solver, lit: u32) {
    let v = solver.watches[lit as usize];
    let (begin, end) = (v.begin, v.end);
    let mut p = begin;
    while p != end {
        let watch = solver.vectors.stack[p];
        p += 1;
        if watch_is_binary(watch) {
            let other = watch_lit(watch);
            let value = solver.values[other as usize];
            if value <= 0 {
                crate::weaken::weaken_binary(solver, lit, other);
            }
            crate::clause::delete_binary(solver, lit, other);
        } else {
            let ref_ = watch_ref(watch);
            if !solver.arena.clause(ref_).garbage() {
                let mut satisfied = false;
                for &other in solver.arena.clause(ref_).lits() {
                    if solver.values[other as usize] > 0 {
                        satisfied = true;
                        break;
                    }
                }
                if !satisfied {
                    crate::weaken::weaken_clause(solver, lit, ref_);
                }
                crate::clause::mark_clause_as_garbage(solver, ref_);
            }
        }
    }
    crate::vector::release_vector(solver, lit); // RELEASE_WATCHES (*lit_watches)
}

// static void do_fast_eliminate (kissat *, unsigned pivot)
fn do_fast_eliminate(solver: &mut Solver, pivot: u32) {
    let lit = crate::literal::lit(pivot);
    let not_lit = crate::literal::not(lit);
    // C re-derives begin/end pointers after every do_fast_resolve; the port
    // re-reads the Vector each iteration instead.
    let mut i: usize = 0;
    loop {
        let lw = solver.watches[lit as usize];
        if i >= lw.size() {
            break;
        }
        let mut j: usize = 0;
        loop {
            let lw = solver.watches[lit as usize];
            let nw = solver.watches[not_lit as usize];
            if j >= nw.size() {
                break;
            }
            let cwatch = solver.vectors.stack[lw.begin + i];
            let dwatch = solver.vectors.stack[nw.begin + j];
            do_fast_resolve(solver, pivot, cwatch, dwatch);
            if solver.inconsistent {
                return;
            }
            j += 1;
        }
        i += 1;
    }
    debug_assert!(!solver.inconsistent);
    solver.statistics.eliminated += 1; // INC (eliminated)
    solver.statistics.fast_eliminated += 1; // INC (fast_eliminated)
    crate::flags::mark_eliminated_variable(solver, pivot);
    fast_delete_and_weaken_clauses(solver, lit);
    fast_delete_and_weaken_clauses(solver, not_lit);
}

// static bool can_fast_resolve_binary_binary (kissat *, unsigned, unsigned)
fn can_fast_resolve_binary_binary(solver: &mut Solver, clit: u32, dlit: u32) -> bool {
    debug_assert!(!solver.flags[crate::literal::idx(clit) as usize].eliminated);
    debug_assert!(!solver.flags[crate::literal::idx(dlit) as usize].eliminated);
    if clit == crate::literal::not(dlit) {
        return false;
    }
    let cval = solver.values[clit as usize];
    if cval > 0 {
        return false;
    }
    let dval = solver.values[dlit as usize];
    if dval > 0 {
        return false;
    }
    if cval < 0 && dval < 0 {
        debug_assert!(!solver.inconsistent);
        solver.inconsistent = true;
        // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
        if solver.proof.is_some() {
            crate::proof::add_empty_to_proof(solver);
        }
        return false;
    }
    if cval < 0 {
        solver.statistics.eliminate_units += 1;
        crate::assign::learned_unit(solver, dlit);
        return false;
    }
    if dval < 0 {
        solver.statistics.eliminate_units += 1;
        crate::assign::learned_unit(solver, clit);
        return false;
    }
    if clit == dlit {
        solver.statistics.eliminate_units += 1;
        crate::assign::learned_unit(solver, clit);
        return false;
    }
    debug_assert!(cval == 0);
    debug_assert!(dval == 0);
    true
}

// static bool can_fast_resolve_binary_large (kissat *, unsigned pivot,
//                                            unsigned lit, clause *c)
fn can_fast_resolve_binary_large(
    solver: &mut Solver,
    pivot: u32,
    lit: u32,
    c_ref: Reference,
) -> bool {
    debug_assert!(!solver.flags[crate::literal::idx(lit) as usize].eliminated);
    if solver.arena.clause(c_ref).garbage() {
        return false;
    }
    debug_assert!(!solver.arena.clause(c_ref).redundant());
    let lit_val = solver.values[lit as usize];
    if lit_val > 0 {
        return false;
    }
    let not_lit = crate::literal::not(lit);
    let mut found_lit = false;
    for &other in solver.arena.clause(c_ref).lits() {
        if other == lit {
            found_lit = true;
        }
        if other == not_lit {
            return false;
        }
        let other_val = solver.values[other as usize];
        if other_val > 0 {
            crate::clause::mark_clause_as_garbage(solver, c_ref);
            return false;
        }
    }
    if found_lit {
        debug_assert!(solver.clause.is_empty());
        for &other in solver.arena.clause(c_ref).lits() {
            let idx = crate::literal::idx(other);
            if idx == pivot {
                continue;
            }
            let value = solver.values[other as usize];
            if value < 0 {
                continue;
            }
            debug_assert!(value == 0);
            solver.clause.push(other);
        }
        solver.statistics.strengthened += 1; // INC (strengthened)
        solver.statistics.fast_strengthened += 1; // INC (fast_strengthened)
        let size = solver.clause.len();
        if size == 0 {
            debug_assert!(!solver.inconsistent);
            solver.inconsistent = true;
            // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
            if solver.proof.is_some() {
                crate::proof::add_empty_to_proof(solver);
            }
        } else if size == 1 {
            let unit = solver.clause[0];
            solver.statistics.eliminate_units += 1;
            crate::assign::learned_unit(solver, unit);
        } else {
            crate::clause::new_irredundant_clause(solver);
        }
        solver.clause.clear();
        // c = kissat_dereference_clause (solver, ref): c_ref still valid.
        crate::clause::mark_clause_as_garbage(solver, c_ref);
        return false;
    }
    true
}

// static bool can_fast_resolve_large_large (kissat *, unsigned pivot,
//                                           clause *c, clause *d)
fn can_fast_resolve_large_large(
    solver: &mut Solver,
    pivot: u32,
    c_ref: Reference,
    d_ref: Reference,
) -> bool {
    if solver.arena.clause(c_ref).garbage() {
        return false;
    }
    if solver.arena.clause(d_ref).garbage() {
        return false;
    }
    debug_assert!(!solver.arena.clause(c_ref).redundant());
    debug_assert!(!solver.arena.clause(d_ref).redundant());
    let mut satisfied = false;
    debug_assert!(solver.clause.is_empty());
    for &other in solver.arena.clause(c_ref).lits() {
        let idx_other = crate::literal::idx(other);
        if idx_other == pivot {
            continue;
        }
        let other_val = solver.values[other as usize];
        if other_val < 0 {
            continue;
        }
        if other_val > 0 {
            satisfied = true;
            crate::clause::mark_clause_as_garbage(solver, c_ref);
            break;
        }
        debug_assert!(solver.marks[other as usize] == 0);
        solver.marks[other as usize] = 1;
        solver.clause.push(other);
    }
    let mut tautological = false;
    if !satisfied {
        for &other in solver.arena.clause(d_ref).lits() {
            let idx_other = crate::literal::idx(other);
            if idx_other == pivot {
                continue;
            }
            let other_val = solver.values[other as usize];
            if other_val < 0 {
                continue;
            }
            if other_val > 0 {
                satisfied = true;
                crate::clause::mark_clause_as_garbage(solver, d_ref);
                break;
            }
            let not_other = crate::literal::not(other);
            let mark_not_other = solver.marks[not_other as usize];
            if mark_not_other != 0 {
                tautological = true;
                break;
            }
            let other_mark = solver.marks[other as usize];
            if other_mark != 0 {
                continue;
            }
            solver.clause.push(other);
        }
    }
    // for (all_literals_in_clause (other, c)) marks[other] = 0;
    for &other in solver.arena.clause(c_ref).lits() {
        solver.marks[other as usize] = 0;
    }
    let mut strengthened = false;
    if !satisfied && !tautological {
        let size = solver.clause.len();
        if size == 0 {
            debug_assert!(!solver.inconsistent);
            solver.inconsistent = true;
            // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
            if solver.proof.is_some() {
                crate::proof::add_empty_to_proof(solver);
            }
            strengthened = true;
        } else if size == 1 {
            let unit = solver.clause[0];
            solver.statistics.eliminate_units += 1;
            crate::assign::learned_unit(solver, unit);
            strengthened = true;
        } else {
            let mut c_subsumed = false;
            let mut d_subsumed = false;
            let mut marked = false;
            if size < solver.arena.clause(c_ref).size() as usize {
                marked = true;
                for i in 0..solver.clause.len() {
                    let other = solver.clause[i];
                    solver.marks[other as usize] = 1;
                }
                let mut count: usize = 0;
                for &other in solver.arena.clause(c_ref).lits() {
                    if solver.marks[other as usize] != 0 {
                        count += 1;
                    }
                }
                c_subsumed = count >= size;
            }
            if size < solver.arena.clause(d_ref).size() as usize {
                if !marked {
                    marked = true;
                    for i in 0..solver.clause.len() {
                        let other = solver.clause[i];
                        solver.marks[other as usize] = 1;
                    }
                }
                let mut count: usize = 0;
                for &other in solver.arena.clause(d_ref).lits() {
                    if solver.marks[other as usize] != 0 {
                        count += 1;
                    }
                }
                d_subsumed = count >= size;
            }
            if marked {
                for i in 0..solver.clause.len() {
                    let other = solver.clause[i];
                    solver.marks[other as usize] = 0;
                }
            }
            if c_subsumed || d_subsumed {
                solver.statistics.strengthened += 1; // INC (strengthened)
                solver.statistics.fast_strengthened += 1; // INC (fast_strengthened)
                crate::clause::new_irredundant_clause(solver);
                strengthened = true;
                if c_subsumed {
                    crate::clause::mark_clause_as_garbage(solver, c_ref);
                }
                if d_subsumed {
                    crate::clause::mark_clause_as_garbage(solver, d_ref);
                }
                if c_subsumed && d_subsumed {
                    solver.statistics.fast_subsumed += 1; // INC (fast_subsumed)
                }
            }
        }
    }
    solver.clause.clear();
    !satisfied && !tautological && !strengthened
}

// static bool can_fast_resolve (kissat *, unsigned pivot, watch c, watch d)
fn can_fast_resolve(solver: &mut Solver, pivot: u32, cwatch: Watch, dwatch: Watch) -> bool {
    debug_assert!(!solver.inconsistent);
    let clit = watch_lit(cwatch);
    let dlit = watch_lit(dwatch);
    let cref = watch_ref(cwatch);
    let dref = watch_ref(dwatch);
    let cbin = watch_is_binary(cwatch);
    let dbin = watch_is_binary(dwatch);
    if cbin && dbin {
        return can_fast_resolve_binary_binary(solver, clit, dlit);
    }
    if cbin && !dbin {
        return can_fast_resolve_binary_large(solver, pivot, clit, dref);
    }
    if !cbin && dbin {
        return can_fast_resolve_binary_large(solver, pivot, dlit, cref);
    }
    debug_assert!(!cbin && !dbin);
    can_fast_resolve_large_large(solver, pivot, cref, dref)
}

// static bool resolvents_limited (kissat *, unsigned pivot, size_t limit)
fn resolvents_limited(solver: &mut Solver, pivot: u32, limit: usize) -> bool {
    let lit = crate::literal::lit(pivot);
    let not_lit = crate::literal::not(lit);
    let mut resolved: usize = 0;
    let mut i: usize = 0;
    loop {
        let lw = solver.watches[lit as usize];
        if i >= lw.size() {
            break;
        }
        let mut j: usize = 0;
        loop {
            let lw = solver.watches[lit as usize];
            let nw = solver.watches[not_lit as usize];
            if j >= nw.size() {
                break;
            }
            let cwatch = solver.vectors.stack[lw.begin + i];
            let dwatch = solver.vectors.stack[nw.begin + j];
            if can_fast_resolve(solver, pivot, cwatch, dwatch) {
                resolved += 1;
                if resolved > limit {
                    return false;
                }
            }
            if solver.inconsistent {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

// static bool try_to_fast_eliminate (kissat *, unsigned pivot)
fn try_to_fast_eliminate(solver: &mut Solver, pivot: u32) -> bool {
    debug_assert!(!solver.inconsistent);
    if !solver.flags[pivot as usize].active {
        return false;
    }
    let lit = crate::literal::lit(pivot);
    let not_lit = crate::literal::not(lit);
    let fasteloccs = solver.options.fasteloccs as usize;
    let pos = flush_occurrences(solver, lit);
    if pos > fasteloccs {
        return false;
    }
    let neg = flush_occurrences(solver, not_lit);
    if neg > fasteloccs {
        return false;
    }
    let sum = pos + neg;
    let product = pos * neg;
    if sum > fasteloccs {
        return false;
    }
    let fastelim = solver.options.fastelim as usize;
    if product <= fastelim {
        do_fast_eliminate(solver, pivot);
        return true;
    }
    if resolvents_limited(solver, pivot, fastelim) {
        do_fast_eliminate(solver, pivot);
        return true;
    }
    false
}

// static void flush_eliminated_binary_clauses_of_literal (kissat *, unsigned)
fn flush_eliminated_binary_clauses_of_literal(solver: &mut Solver, lit: u32) {
    let v = solver.watches[lit as usize];
    let begin = v.begin;
    let end = v.end;
    let mut q = begin;
    let mut p = begin;
    while p != end {
        let watch = solver.vectors.stack[p];
        solver.vectors.stack[q] = watch;
        q += 1;
        p += 1;
        if !watch_is_binary(watch) {
            continue;
        }
        let other = watch_lit(watch);
        let other_idx = crate::literal::idx(other);
        if solver.flags[other_idx as usize].eliminated {
            q -= 1;
        }
    }
    crate::vector::resize_vector(solver, lit, q - begin); // SET_END_OF_WATCHES
}

// static void flush_eliminated_binary_clauses (kissat *)
fn flush_eliminated_binary_clauses(solver: &mut Solver) {
    for idx in 0..solver.vars {
        let lit = crate::literal::lit(idx);
        let not_lit = crate::literal::not(lit);
        flush_eliminated_binary_clauses_of_literal(solver, lit);
        flush_eliminated_binary_clauses_of_literal(solver, not_lit);
    }
}

// struct candidate { unsigned pivot; unsigned score; };
#[derive(Clone, Copy)]
struct Candidate {
    pivot: u32,
    score: u32,
}

/// Port of `kissat_fast_variable_elimination`.
pub fn fast_variable_elimination(solver: &mut Solver) {
    if solver.inconsistent {
        return;
    }
    if solver.options.fastel == 0 {
        return;
    }
    // #ifndef QUIET
    let variables_before = solver.active;
    debug_assert!(solver.level == 0);
    crate::profile::start_checked(solver, Prof::fastel); // START (fastel)
    crate::dense::enter_dense_mode(solver, None);
    crate::watch::connect_irredundant_large_clauses(solver);
    let fastelrounds = solver.options.fastelrounds as u32;
    let fasteloccs = solver.options.fasteloccs as usize;
    // #ifndef QUIET
    let mut eliminated: u32 = 0;
    let mut round: u32 = 0;
    let mut candidates: Vec<Candidate> = Vec::new(); // INIT_STACK
    let mut done = false;
    loop {
        if round >= fastelrounds {
            break;
        }
        round += 1;
        crate::print::extremely_verbose(
            solver,
            format_args!("gathering candidates for fast elimination round {}", round),
        );
        debug_assert!(candidates.is_empty());
        for pivot in 0..solver.vars {
            let pivot_flags = solver.flags[pivot as usize];
            if !pivot_flags.active {
                continue;
            }
            if !pivot_flags.eliminate {
                continue;
            }
            let lit = crate::literal::lit(pivot);
            let pos = flush_occurrences(solver, lit);
            if pos > fasteloccs {
                continue;
            }
            // QUIRK ported: C computes `not_lit = LIT (pivot)` (not NOT).
            let not_lit = crate::literal::lit(pivot);
            let neg = flush_occurrences(solver, not_lit);
            if neg > fasteloccs {
                continue;
            }
            let score = (pos + neg) as u32;
            if score as usize > fasteloccs {
                continue;
            }
            candidates.push(Candidate { pivot, score });
        }
        // #ifndef QUIET
        let size_candidates = candidates.len();
        {
            let active_variables = solver.active;
            crate::print::extremely_verbose(
                solver,
                format_args!(
                    "gathered {} candidates {:.0}% in elimination round {}",
                    size_candidates,
                    crate::format::percent(size_candidates as f64, active_variables as f64),
                    round
                ),
            );
        }
        // RADIX_STACK (candidate, unsigned, candidates, RANK_CANDIDATE)
        crate::profile::start_checked(solver, Prof::radix);
        crate::sort::radix_stack::<Candidate, u32, _>(&mut candidates, |c| c.score);
        crate::profile::stop_checked(solver, Prof::radix);
        let mut eliminated_this_round: u32 = 0;
        for i in 0..candidates.len() {
            let pivot = candidates[i].pivot;
            let pivot_flags = solver.flags[pivot as usize];
            if !pivot_flags.active {
                continue;
            }
            if !pivot_flags.eliminate {
                continue;
            }
            if terminated!(solver, fastel_terminated_1) {
                done = true;
                break;
            }
            if try_to_fast_eliminate(solver, pivot) {
                eliminated_this_round += 1;
            }
            if solver.inconsistent {
                done = true;
                break;
            }
            solver.flags[pivot as usize].eliminate = false;
            crate::eliminate::flush_units_while_connected(solver);
            if solver.inconsistent {
                done = true;
                break;
            }
        }
        candidates.clear(); // CLEAR_STACK
        // #ifndef QUIET
        eliminated += eliminated_this_round;
        crate::print::very_verbose(
            solver,
            format_args!(
                "fast eliminated {} of {} candidates {:.0}% in round {}",
                eliminated_this_round,
                size_candidates,
                crate::format::percent(eliminated_this_round as f64, size_candidates as f64),
                round
            ),
        );
        if eliminated_this_round == 0 {
            done = true;
        }
        if done {
            break;
        }
    }
    drop(candidates); // RELEASE_STACK
    for idx in 0..solver.vars {
        solver.flags[idx as usize].eliminate = true;
    }
    flush_eliminated_binary_clauses(solver);
    crate::dense::resume_sparse_mode(solver, true, None);
    // #ifndef QUIET
    {
        let original_variables = solver.statistics.variables_original;
        let variables_after = solver.active;
        crate::print::verbose(
            solver,
            format_args!(
                "[fastel] fast elimination of {} variables {:.0}% ({} remain {:.0}%)",
                eliminated,
                crate::format::percent(eliminated as f64, variables_before as f64),
                variables_after,
                crate::format::percent(variables_after as f64, original_variables as f64)
            ),
        );
    }
    crate::profile::stop_checked(solver, Prof::fastel); // STOP (fastel)
    crate::report::report(solver, eliminated == 0, 'e');
}
