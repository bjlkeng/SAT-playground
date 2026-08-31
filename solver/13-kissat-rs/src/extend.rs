// Port of src/extend.c + src/extend.h (kissat 4.0.4).
//
// PORT NOTE: C walks the extension stack with raw `extension *` cursors from
// END_STACK down to BEGIN_STACK; the port uses a descending index into
// `solver.extend` (the stack is never mutated during the walk, only
// `eliminated`/`etrail` are).  C's do-while `continue` re-tests the
// `!blocking` condition, mirrored by the trailing `if blocking != 0 break`.
// PORT NOTE: `undo_eliminated_assignment` and `extend_assign` in C take a
// cached `value *values = BEGIN_STACK (solver->eliminated)` pointer; the
// port indexes `solver.eliminated` directly (same element writes).

use crate::internal::Solver;
use crate::profile::Prof;

/// C `struct extension` — `signed int lit : 31; bool blocking : 1;`
/// packed into one u32 word for layout fidelity.
#[derive(Clone, Copy)]
pub struct Extension(pub u32);

impl Extension {
    #[inline]
    pub fn new(blocking: bool, lit: i32) -> Extension {
        debug_assert!(lit.unsigned_abs() < (1 << 30));
        Extension((((lit as u32) << 1) >> 1) | ((blocking as u32) << 31))
    }

    #[inline]
    pub fn lit(self) -> i32 {
        // Sign-extend the low 31 bits.
        ((self.0 << 1) as i32) >> 1
    }

    #[inline]
    pub fn blocking(self) -> bool {
        (self.0 >> 31) != 0
    }
}

pub type Extensions = Vec<Extension>;

// undo_eliminated_assignment (static)
fn undo_eliminated_assignment(solver: &mut Solver) {
    let size_etrail = solver.etrail.len();
    if size_etrail == 0 {
        return;
    }
    while let Some(pos) = solver.etrail.pop() {
        debug_assert!((pos as usize) < solver.eliminated.len());
        debug_assert!(solver.eliminated[pos as usize] != 0);
        solver.eliminated[pos as usize] = 0;
    }
}

// extend_assign (static).  Assigns the external literal's slot in the
// eliminated-values stack and records it on the etrail.
fn extend_assign(solver: &mut Solver, lit: i32) {
    debug_assert!(lit != 0);
    debug_assert!(lit != i32::MIN);
    let idx = lit.unsigned_abs();
    let import = solver.import_[idx as usize]; // PEEK_STACK (solver->import, idx)
    debug_assert!(import.eliminated);
    debug_assert!(import.imported);
    let pos = import.lit;
    debug_assert!((pos as usize) < solver.eliminated.len());
    let value: i8 = if lit < 0 { -1 } else { 1 };
    solver.eliminated[pos as usize] = value;
    solver.etrail.push(pos);
}

/// Port of `kissat_extend` — model reconstruction from the extension stack
/// for eliminated/substituted variables.
pub fn extend(solver: &mut Solver) {
    debug_assert!(!solver.extend.is_empty());
    debug_assert!(!solver.extended);

    crate::profile::start_checked(solver, Prof::extend); // START (extend)
    solver.extended = true;

    undo_eliminated_assignment(solver);

    let mut p = solver.extend.len(); // extension const *p = END_STACK (...)

    while p > 0 {
        let mut pos: u32 = u32::MAX;
        let mut satisfied = false;

        let mut eliminated: i32 = 0;
        let mut blocking: i32 = 0;

        // C: do { ... } while (!blocking);
        loop {
            debug_assert!(p > 0);
            p -= 1;
            let ext = solver.extend[p]; // *--p
            let elit = ext.lit();
            if ext.blocking() {
                blocking = elit;
            }

            // C `if (satisfied) continue;` falls through to the do-while
            // condition test, mirrored below.
            if !satisfied {
                debug_assert!(elit != i32::MIN);
                let eidx = elit.unsigned_abs();
                debug_assert!((eidx as usize) < solver.import_.len());
                let import = solver.import_[eidx as usize];
                debug_assert!(import.imported);

                if import.eliminated {
                    let tmp = import.lit;
                    debug_assert!((tmp as usize) < solver.eliminated.len());
                    let mut value = solver.eliminated[tmp as usize];

                    if elit < 0 {
                        value = -value;
                    }

                    if value > 0 {
                        // previously assigned eliminated literal satisfies
                        satisfied = true;
                    } else if value == 0 && (eliminated == 0 || pos < tmp) {
                        eliminated = elit;
                        pos = tmp;
                    }
                } else {
                    let ilit = import.lit;
                    let mut value = solver.values[ilit as usize];
                    debug_assert!(value != 0);

                    if elit < 0 {
                        value = -value;
                    }

                    if value > 0 {
                        // internal literal satisfies clause
                        satisfied = true;
                    }
                }
            }

            if blocking != 0 {
                break;
            }
        }

        if satisfied {
            continue;
        }

        if eliminated != 0 && eliminated != blocking {
            extend_assign(solver, eliminated);
            continue;
        }

        extend_assign(solver, blocking);
    }

    crate::profile::stop_checked(solver, Prof::extend); // STOP (extend)
}
