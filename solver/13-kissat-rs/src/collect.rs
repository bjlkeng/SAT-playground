// Port of src/collect.h inline functions (kissat 4.0.4).
// collect.c (dense/sparse GC) lands with the core wave.

use crate::internal::Solver;
use crate::reference::Reference;

#[inline]
pub fn defrag_watches(solver: &mut Solver) {
    crate::vector::defrag_vectors(solver);
}

#[inline]
pub fn defrag_watches_if_needed(solver: &mut Solver) {
    let size = solver.vectors.stack.len();
    let size_limit = solver.options.defragsize as usize;
    if size <= size_limit {
        return;
    }
    let usable = solver.vectors.usable as usize;
    let usable_limit = (size * solver.options.defraglim as usize) / 100;
    if usable <= usable_limit {
        return;
    }
    // INC (vectors_defrags_needed) is METRIC-only: no-op in the reference build.
    defrag_watches(solver);
}

pub fn compacting(solver: &mut Solver) -> bool {
    let _ = solver;
    unimplemented!("collect wave pending")
}

pub fn dense_collect(solver: &mut Solver) {
    let _ = solver;
    unimplemented!("collect wave pending")
}

pub fn sparse_collect(solver: &mut Solver, _compact: bool, _start: Reference) {
    unimplemented!("collect wave pending")
}

pub fn initial_sparse_collect(solver: &mut Solver) {
    let _ = solver;
    unimplemented!("collect wave pending")
}
