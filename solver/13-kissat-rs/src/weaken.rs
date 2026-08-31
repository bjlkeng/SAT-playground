// Port of src/weaken.c + src/weaken.h (kissat 4.0.4).
//
// Pushes weakened clauses onto the extension (reconstruction) stack with a
// blocking witness literal, used by elimination and friends.
// PORT NOTE: INC (weakened) is a METRIC counter (statistics.h), compiled
// out in the reference build — no-op here per INTEGRATION_NOTES.
// PORT NOTE: `kissat_weaken_clause` takes `clause *c` in C; the crate
// convention passes the arena Reference, and the literal loop re-derives
// the clause accessor per iteration (arena borrow vs `&mut Solver`, same
// pattern as clause.rs::mark_clause_as_garbage).

use crate::extend::Extension;
use crate::internal::Solver;
use crate::reference::Reference;

// push_witness_literal (static)
fn push_witness_literal(solver: &mut Solver, ilit: u32) {
    debug_assert!(solver.values[ilit as usize] == 0); // !VALUE (ilit)
    let elit = crate::inline::export_literal(solver, ilit);
    debug_assert!(elit != 0);
    let ext = Extension::new(true, elit); // kissat_extension (true, elit)
    solver.extend.push(ext);
}

// push_clause_literal (static)
fn push_clause_literal(solver: &mut Solver, ilit: u32) {
    let value = solver.values[ilit as usize]; // VALUE (ilit)
    debug_assert!(value <= 0);
    if value < 0 {
        // not pushing internal falsified clause literal
    } else {
        let elit = crate::inline::export_literal(solver, ilit);
        debug_assert!(elit != 0);
        let ext = Extension::new(false, elit); // kissat_extension (false, elit)
        solver.extend.push(ext);
    }
}

/// Port of `kissat_weaken_clause`.
pub fn weaken_clause(solver: &mut Solver, lit: u32, ref_: Reference) {
    // INC (weakened): METRIC, compiled out.
    push_witness_literal(solver, lit);
    let size = solver.arena.clause(ref_).size();
    for i in 0..size {
        let other = solver.arena.clause(ref_).lit(i);
        if lit != other {
            push_clause_literal(solver, other);
        }
    }
}

/// Port of `kissat_weaken_binary`.
pub fn weaken_binary(solver: &mut Solver, lit: u32, other: u32) {
    // INC (weakened): METRIC, compiled out.
    push_witness_literal(solver, lit);
    push_clause_literal(solver, other);
}

/// Port of `kissat_weaken_unit`.
pub fn weaken_unit(solver: &mut Solver, lit: u32) {
    // INC (weakened): METRIC, compiled out.
    push_witness_literal(solver, lit);
}
