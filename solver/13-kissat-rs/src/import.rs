// Port of src/import.c (kissat 4.0.4).
//
// PORT NOTES:
//  - variables_extension / variables_original are COUNTERs; fresh is a
//    STATISTIC-tier field (kept, never printed).
//  - The C non-tumble do-while `do { ilit = import_literal (solver, other,
//    false); } while (other++ < eidx);` (post-increment in the condition) is
//    ported with an explicit old-value test.
//  - VALID_INTERNAL_LITERAL / VALID_EXTERNAL_LITERAL asserts are NDEBUG-out.

use crate::internal::{Import, Solver};
use crate::literal::{EXTERNAL_MAX_VAR, INVALID_LIT};

// static adjust_imports_for_external_literal
fn adjust_imports_for_external_literal(solver: &mut Solver, eidx: u32) {
    while eidx as usize >= solver.import_.len() {
        solver.import_.push(Import {
            lit: 0,
            extension: false,
            imported: false,
            eliminated: false,
        });
    }
}

// static adjust_exports_for_external_literal
fn adjust_exports_for_external_literal(solver: &mut Solver, eidx: u32, extension: bool) {
    let iidx = solver.vars;
    crate::resize::enlarge_variables(solver, iidx + 1);
    let ilit = 2 * iidx;
    {
        let import = &mut solver.import_[eidx as usize];
        import.extension = extension;
        import.imported = true;
    }
    if extension {
        solver.statistics.variables_extension += 1; // INC (variables_extension)
    } else {
        solver.statistics.variables_original += 1; // INC (variables_original)
    }
    debug_assert!(!solver.import_[eidx as usize].eliminated);
    solver.import_[eidx as usize].lit = ilit;
    while iidx as usize >= solver.export_.len() {
        solver.export_.push(0);
    }
    solver.export_[iidx as usize] = eidx as i32; // POKE_STACK (solver->export, iidx, eidx)
}

// static inline import_literal (the file-local one)
fn import_literal_inner(solver: &mut Solver, elit: i32, extension: bool) -> u32 {
    let eidx = elit.unsigned_abs();
    adjust_imports_for_external_literal(solver, eidx);
    if solver.import_[eidx as usize].eliminated {
        return INVALID_LIT;
    }
    if !solver.import_[eidx as usize].imported {
        adjust_exports_for_external_literal(solver, eidx, extension);
    }
    debug_assert!(solver.import_[eidx as usize].imported);
    let mut ilit = solver.import_[eidx as usize].lit;
    if elit < 0 {
        ilit = crate::literal::not(ilit);
    }
    ilit
}

/// Port of `kissat_import_literal`.
pub fn import_literal(solver: &mut Solver, elit: i32) -> u32 {
    debug_assert!(crate::literal::valid_external_literal(elit));
    if solver.options.tumble != 0 {
        return import_literal_inner(solver, elit, false);
    }
    let eidx = elit.unsigned_abs();
    debug_assert!(solver.import_.len() <= u32::MAX as usize);
    let mut other = solver.import_.len() as u32;
    if eidx < other {
        return import_literal_inner(solver, elit, false);
    }
    if other == 0 {
        adjust_imports_for_external_literal(solver, other);
        other += 1;
    }

    let mut ilit: u32;
    loop {
        // do { ilit = import_literal (solver, other, false); } while (other++ < eidx);
        ilit = import_literal_inner(solver, other as i32, false);
        let old = other;
        other += 1;
        if old >= eidx {
            break;
        }
    }

    if elit < 0 {
        ilit = crate::literal::not(ilit);
    }

    ilit
}

/// Port of `kissat_fresh_literal`.
pub fn fresh_literal(solver: &mut Solver) -> u32 {
    let imported = solver.import_.len();
    debug_assert!(imported <= EXTERNAL_MAX_VAR as usize);
    if imported == EXTERNAL_MAX_VAR as usize {
        // can not get another external variable
        return INVALID_LIT;
    }
    debug_assert!(imported <= i32::MAX as usize);
    let eidx = imported as i32;
    let res = import_literal_inner(solver, eidx, true);
    solver.statistics.fresh += 1; // INC (fresh): STATISTIC
    crate::flags::activate_literal(solver, res);
    res
}
