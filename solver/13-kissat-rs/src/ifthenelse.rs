// Port of src/ifthenelse.c (kissat 4.0.4).
//
// If-then-else gate extraction: lit = (c ? t : e) from four ternary clauses.
//
// PORT NOTE: C's find_ternary_clause returns a `const watch *` into the watch
// list of `a`; the port returns the word offset into solver.vectors.stack
// (Option<usize>).  The C caller compares the two pointers (`p3 < p4`) to
// order w3/w4 — both point into the same (not_lit) list, so comparing the
// offsets is identical.  Nothing in the search mutates watch lists (only
// clause headers via kissat_eliminate_clause), so the offsets stay valid.

use crate::internal::{Solver, INVALID};
use crate::reference::Reference;
use crate::watch::{watch_is_binary, watch_lit, watch_ref, Watch};

// static bool get_ternary_clause (kissat *, reference, unsigned *, ...)
fn get_ternary_clause(solver: &mut Solver, ref_: Reference) -> Option<(u32, u32, u32)> {
    if solver.arena.clause(ref_).garbage() {
        return None;
    }
    let mut a = INVALID;
    let mut b = INVALID;
    let mut c = INVALID;
    let mut found: u32 = 0;
    for &other in solver.arena.clause(ref_).lits() {
        let value = solver.values[other as usize];
        if value > 0 {
            crate::eliminate::eliminate_clause(solver, ref_, INVALID);
            return None;
        }
        if value < 0 {
            continue;
        }
        found += 1;
        if found == 1 {
            a = other;
        } else if found == 2 {
            b = other;
        } else if found == 3 {
            c = other;
        } else {
            return None;
        }
    }
    if found != 3 {
        return None;
    }
    debug_assert!(a != INVALID);
    debug_assert!(b != INVALID);
    debug_assert!(c != INVALID);
    Some((a, b, c))
}

// static bool match_ternary_ref (kissat *, reference, unsigned a, b, c)
fn match_ternary_ref(solver: &mut Solver, ref_: Reference, a: u32, b: u32, c: u32) -> bool {
    if solver.arena.clause(ref_).garbage() {
        return false;
    }
    let mut found: u32 = 0;
    for &other in solver.arena.clause(ref_).lits() {
        let value = solver.values[other as usize];
        if value > 0 {
            crate::eliminate::eliminate_clause(solver, ref_, INVALID);
            return false;
        }
        if value < 0 {
            continue;
        }
        if a != other && b != other && c != other {
            return false;
        }
        found += 1;
    }
    if found == 3 {
        return true;
    }
    solver.resolve_gate = true;
    true
}

// static bool match_ternary_watch (kissat *, watch, unsigned a, b, c)
fn match_ternary_watch(solver: &mut Solver, watch: Watch, a: u32, b: u32, c: u32) -> bool {
    if watch_is_binary(watch) {
        let other = watch_lit(watch);
        if other != b && other != c {
            return false;
        }
        solver.resolve_gate = true;
        true
    } else {
        let ref_ = watch_ref(watch);
        match_ternary_ref(solver, ref_, a, b, c)
    }
}

// static inline const watch *find_ternary_clause (kissat *, uint64_t *steps,
//                                                 unsigned a, b, c)
fn find_ternary_clause(
    solver: &mut Solver,
    steps: &mut u64,
    a: u32,
    b: u32,
    c: u32,
) -> Option<usize> {
    let v = solver.watches[a as usize];
    for p in v.begin..v.end {
        *steps += 1;
        let watch = solver.vectors.stack[p];
        if match_ternary_watch(solver, watch, a, b, c) {
            return Some(p);
        }
    }
    None
}

/// Port of `kissat_find_if_then_else_gate`.
pub fn find_if_then_else_gate(solver: &mut Solver, lit: u32, negative: u32) -> bool {
    if solver.options.ifthenelse == 0 {
        return false;
    }
    let v = solver.watches[lit as usize];
    let begin = v.begin;
    let end = v.end;
    if begin == end {
        return false;
    }
    let mut large_clauses: u64 = 0;
    for p in begin..end {
        if !watch_is_binary(solver.vectors.stack[p]) {
            large_clauses += 1;
        }
    }
    let limit = solver.options.eliminateocclim as u64;
    if large_clauses * large_clauses > limit {
        return false;
    }
    let last = end - 1;
    let mut steps: u64 = 0;
    let mut p1 = begin;
    while steps < limit && p1 != last {
        let w1 = solver.vectors.stack[p1];
        if watch_is_binary(w1) {
            p1 += 1;
            continue;
        }
        let Some((mut a1, mut b1, mut c1)) = get_ternary_clause(solver, watch_ref(w1)) else {
            p1 += 1;
            continue;
        };
        if b1 == lit {
            std::mem::swap(&mut a1, &mut b1);
        }
        if c1 == lit {
            std::mem::swap(&mut a1, &mut c1);
        }
        debug_assert!(a1 == lit);
        let mut p2 = p1 + 1;
        while steps < limit && p2 != end {
            let w2 = solver.vectors.stack[p2];
            if watch_is_binary(w2) {
                p2 += 1;
                continue;
            }
            let Some((mut a2, mut b2, mut c2)) = get_ternary_clause(solver, watch_ref(w2))
            else {
                p2 += 1;
                continue;
            };
            if b2 == lit {
                std::mem::swap(&mut a2, &mut b2);
            }
            if c2 == lit {
                std::mem::swap(&mut a2, &mut c2);
            }
            debug_assert!(a2 == lit);
            if crate::literal::strip(b1) == crate::literal::strip(c2) {
                std::mem::swap(&mut b2, &mut c2);
            }
            if crate::literal::strip(c1) == crate::literal::strip(c2) {
                p2 += 1;
                continue;
            }
            let not_b2 = crate::literal::not(b2);
            if b1 != not_b2 {
                p2 += 1;
                continue;
            }
            solver.resolve_gate = false;
            let not_lit = crate::literal::not(lit);
            let not_c1 = crate::literal::not(c1);
            let Some(p3) = find_ternary_clause(solver, &mut steps, not_lit, b1, not_c1) else {
                p2 += 1;
                continue;
            };
            let not_c2 = crate::literal::not(c2);
            let Some(p4) = find_ternary_clause(solver, &mut steps, not_lit, b2, not_c2) else {
                p2 += 1;
                continue;
            };
            let w3 = if p3 < p4 {
                solver.vectors.stack[p3]
            } else {
                solver.vectors.stack[p4]
            };
            let w4 = if p3 < p4 {
                solver.vectors.stack[p4]
            } else {
                solver.vectors.stack[p3]
            };
            solver.gate_eliminated = true; // GATE_ELIMINATED (if_then_else)
            solver.gates[negative as usize].push(w1);
            solver.gates[negative as usize].push(w2);
            solver.gates[(1 ^ negative) as usize].push(w3);
            solver.gates[(1 ^ negative) as usize].push(w4);
            // INC (if_then_else_extracted): METRIC, compiled out.
            return true;
        }
        p1 += 1;
    }
    false
}
