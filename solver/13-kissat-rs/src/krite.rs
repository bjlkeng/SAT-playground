// Port of src/krite.c (kissat 4.0.4).
//
// PORT NOTES:
//  - C `FILE *file` becomes `&mut dyn std::io::Write` (matches the
//    application.rs '-o' call sites: stdout lock or File).
//  - Write errors are ignored (`let _ =`) exactly as C's unchecked fprintf.
//  - The clause-literal line uses "%d " per literal then "0\n" — the
//    trailing-space-before-0 format is reproduced verbatim.

use crate::internal::Solver;

/// Port of `kissat_write_dimacs`.
pub fn write_dimacs(solver: &mut Solver, file: &mut dyn std::io::Write) {
    let mut imported = solver.import_.len(); // SIZE_STACK (solver->import)
    if imported > 0 {
        imported -= 1;
    }
    let binirr = solver.statistics.binirr_clauses(); // BINIRR_CLAUSES
    let _ = writeln!(file, "p cnf {} {}", imported, binirr);
    debug_assert!(solver.watching);
    if solver.watching {
        for ilit in 0..solver.lits() {
            // for (all_binary_blocking_watches (watch, WATCHES (ilit)))
            let v = solver.watches[ilit as usize];
            let mut p = v.begin;
            while p != v.end {
                let watch = solver.vectors.stack[p];
                if !crate::watch::watch_is_binary(watch) {
                    p += 2; // blocking word + reference word
                    continue;
                }
                p += 1;
                let iother = crate::watch::watch_lit(watch);
                if iother < ilit {
                    continue;
                }
                let elit = crate::inline::export_literal(solver, ilit);
                let eother = crate::inline::export_literal(solver, iother);
                let _ = writeln!(file, "{} {} 0", elit, eother);
            }
        }
    } else {
        for ilit in 0..solver.lits() {
            // for (all_binary_large_watches (watch, WATCHES (ilit)))
            let v = solver.watches[ilit as usize];
            let mut p = v.begin;
            while p != v.end {
                let watch = solver.vectors.stack[p];
                p += 1;
                if !crate::watch::watch_is_binary(watch) {
                    continue;
                }
                let iother = crate::watch::watch_lit(watch);
                if iother < ilit {
                    continue;
                }
                let elit = crate::inline::export_literal(solver, ilit);
                let eother = crate::inline::export_literal(solver, iother);
                let _ = writeln!(file, "{} {} 0", elit, eother);
            }
        }
    }
    // for (all_clauses (c))
    let mut ref_: crate::reference::Reference = 0;
    while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        let (garbage, redundant, size) = {
            let c = solver.arena.clause(ref_);
            (c.garbage(), c.redundant(), c.size())
        };
        if !garbage && !redundant {
            for i in 0..size {
                let ilit = solver.arena.clause(ref_).lit(i);
                let elit = crate::inline::export_literal(solver, ilit);
                let _ = write!(file, "{} ", elit);
            }
            let _ = write!(file, "0\n"); // fputs ("0\n", file)
        }
        ref_ = next;
    }
}
