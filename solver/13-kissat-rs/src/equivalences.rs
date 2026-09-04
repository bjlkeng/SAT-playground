// Port of src/equivalences.c (kissat 4.0.4).
//
// Equivalence-gate extraction: `lit = replace` from the binary clauses
// (-lit replace) and (lit -replace).

use crate::internal::{Solver, INVALID};
use crate::watch::{binary_watch, watch_is_binary, watch_lit};

/// Port of `kissat_find_equivalence_gate`.
pub fn find_equivalence_gate(solver: &mut Solver, lit: u32) -> bool {
    if solver.options.equivalences == 0 {
        return false;
    }
    if crate::gates::mark_binaries(solver, lit) == 0 {
        return false;
    }
    let not_lit = crate::literal::not(lit);
    let mut replace = INVALID;
    let v = solver.watches[not_lit as usize];
    for &watch in &solver.vectors.stack[v.begin..v.end] {
        if !watch_is_binary(watch) {
            continue;
        }
        let other = watch_lit(watch);
        let not_other = crate::literal::not(other);
        if solver.marks[not_other as usize] == 0 {
            continue;
        }
        replace = other;
        break;
    }
    crate::gates::unmark_binaries(solver, lit);
    if replace == INVALID {
        return false;
    }

    let watch1 = binary_watch(replace);
    solver.gates[1].push(watch1);

    let watch0 = binary_watch(crate::literal::not(replace));
    solver.gates[0].push(watch0);
    solver.gate_eliminated = true; // GATE_ELIMINATED (equivalences)
    // INC (equivalences_extracted): METRIC, compiled out.
    true
}
