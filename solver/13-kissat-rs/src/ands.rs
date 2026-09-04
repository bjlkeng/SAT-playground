// Port of src/ands.c (kissat 4.0.4).
//
// AND-gate extraction: `lit = (a & b & ...)` shows up as binary clauses
// (lit -> a), (lit -> b), ... plus a base clause (-lit a' b' ...) whose
// remaining literals are all negations of marked binaries.
//
// PORT NOTE: C tracks the base clause with a `clause *base`; the port uses a
// Reference with INVALID as NULL plus a `found` flag for the per-clause scan.

use crate::internal::{Solver, INVALID};
use crate::reference::Reference;
use crate::watch::{binary_watch, large_watch, watch_is_binary, watch_lit, watch_ref};

/// Port of `kissat_find_and_gate`.
pub fn find_and_gate(solver: &mut Solver, lit: u32, negative: u32) -> bool {
    if solver.options.ands == 0 {
        return false;
    }
    let marked = crate::gates::mark_binaries(solver, lit);
    if marked == 0 {
        return false;
    }
    if marked < 2 {
        crate::gates::unmark_binaries(solver, lit);
        return false;
    }

    let not_lit = crate::literal::not(lit);

    let mut base: Reference = INVALID; // clause *base = 0
    let v = solver.watches[not_lit as usize];
    for p in v.begin..v.end {
        let watch = solver.vectors.stack[p];
        if watch_is_binary(watch) {
            continue;
        }
        let ref_ = watch_ref(watch);
        debug_assert!(!solver.arena.clause(ref_).garbage());
        let mut candidate = true; // base = c
        for &other in solver.arena.clause(ref_).lits() {
            if other == not_lit {
                continue;
            }
            let value = solver.values[other as usize];
            if value > 0 {
                crate::eliminate::eliminate_clause(solver, ref_, INVALID);
                candidate = false; // base = 0
                break;
            }
            if value < 0 {
                continue;
            }
            let not_other = crate::literal::not(other);
            let mark = solver.marks[not_other as usize];
            if mark != 0 {
                continue;
            }
            candidate = false; // base = 0
            break;
        }
        if candidate {
            base = ref_;
            break;
        }
    }
    if base == INVALID {
        crate::gates::unmark_binaries(solver, lit);
        return false;
    }

    // Unmark the negations of the base clause literals.
    for &other in solver.arena.clause(base).lits() {
        if other == not_lit {
            continue;
        }
        if solver.values[other as usize] != 0 {
            continue;
        }
        let not_other = crate::literal::not(other);
        debug_assert!(solver.marks[not_other as usize] != 0);
        solver.marks[not_other as usize] = 0;
    }

    // Binary watches of `lit` still marked are NOT part of the gate; the
    // unmarked ones are the gate binaries.
    let v = solver.watches[lit as usize];
    for &watch in &solver.vectors.stack[v.begin..v.end] {
        if !watch_is_binary(watch) {
            continue;
        }
        let other = watch_lit(watch);
        debug_assert!(solver.values[other as usize] == 0);
        if solver.marks[other as usize] != 0 {
            solver.marks[other as usize] = 0;
            continue;
        }
        solver.gates[negative as usize].push(binary_watch(other));
    }
    solver.gates[(1 ^ negative) as usize].push(large_watch(base)); // gates[!negative]
    solver.gate_eliminated = true; // GATE_ELIMINATED (ands)
    // INC (ands_extracted): METRIC, compiled out.
    true
}
