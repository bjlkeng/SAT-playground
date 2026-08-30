// STUB — full port of src/proof.c (kissat 4.0.4) lands with the proof wave.
// Call-site surface only, guarded by solver.proof.is_some(); no-ops for now.

use crate::internal::Solver;
use crate::reference::Reference;

pub struct Proof {
    // Placeholder; the real port carries the file, binary flag, buffers.
    _private: (),
}

pub fn add_binary_to_proof(solver: &mut Solver, _a: u32, _b: u32) {
    debug_assert!(solver.proof.is_none() || true);
}

pub fn add_clause_to_proof(_solver: &mut Solver, _ref_: Reference) {}

pub fn delete_clause_from_proof(_solver: &mut Solver, _ref_: Reference) {}

pub fn delete_binary_from_proof(_solver: &mut Solver, _a: u32, _b: u32) {}

pub fn add_empty_to_proof(_solver: &mut Solver) {}

pub fn add_lits_to_proof(_solver: &mut Solver, _lits: &[u32]) {}

pub fn delete_external_from_proof(_solver: &mut Solver, _elits: &[i32]) {}

pub fn print_proof_statistics(_solver: &mut Solver, _verbose: bool) {}
