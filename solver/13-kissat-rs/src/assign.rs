// STUB — full port of src/assign.c (kissat 4.0.4) lands with the core wave.

use crate::internal::Solver;
use crate::reference::Reference;

pub fn assign_unit(solver: &mut Solver, _lit: u32) {
    let _ = solver;
    unimplemented!("assign wave pending")
}

pub fn assign_reference(solver: &mut Solver, _lit: u32, _ref_: Reference) {
    let _ = solver;
    unimplemented!("assign wave pending")
}

pub fn assign_binary(solver: &mut Solver, _lit: u32, _other: u32) {
    let _ = solver;
    unimplemented!("assign wave pending")
}

pub fn original_unit(solver: &mut Solver, _lit: u32) {
    let _ = solver;
    unimplemented!("assign wave pending")
}
