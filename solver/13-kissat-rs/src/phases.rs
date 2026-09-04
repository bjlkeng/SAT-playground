// Port of src/phases.h + src/phases.c (kissat 4.0.4).
// PORT NOTE: the three C `value *` arrays become Vec<Value>; realloc+memset
// grow becomes `resize(new_size, 0)` (zero-fills the new tail exactly like
// the C memset), shrink becomes `truncate`, release becomes dropping the
// Vecs. Capacity policy never affects semantics (see CONVENTIONS.md).
// PORT NOTE: C static `save_phases (kissat *, value *phases)` reads
// solver->values while writing one of the phase arrays; the Rust helper
// takes the two disjoint slices explicitly (same traversal order:
// phases[i] updated from values[2*i] when nonzero, over VARS entries).
// The BEST/SAVED/TARGET macros are direct indexing at call sites:
// solver.phases.best[idx] etc.

use crate::internal::Solver;
use crate::value::Value;

#[derive(Default)]
pub struct Phases {
    pub best: crate::uvec::UVec<Value>,
    pub saved: crate::uvec::UVec<Value>,
    pub target: crate::uvec::UVec<Value>,
}

pub fn increase_phases(solver: &mut Solver, new_size: u32) {
    let old_size = solver.size;
    debug_assert!(old_size < new_size);
    solver.phases.best.resize(new_size as usize, 0);
    solver.phases.saved.resize(new_size as usize, 0);
    solver.phases.target.resize(new_size as usize, 0);
}

pub fn decrease_phases(solver: &mut Solver, new_size: u32) {
    let old_size = solver.size;
    debug_assert!(old_size > new_size);
    solver.phases.best.truncate(new_size as usize);
    solver.phases.saved.truncate(new_size as usize);
    solver.phases.target.truncate(new_size as usize);
}

pub fn release_phases(solver: &mut Solver) {
    solver.phases.best = Default::default();
    solver.phases.saved = Default::default();
    solver.phases.target = Default::default();
}

// C static `save_phases`.
fn save_phases(values: &[Value], phases: &mut [Value]) {
    for (i, p) in phases.iter_mut().enumerate() {
        let tmp = values[2 * i];
        if tmp != 0 {
            *p = tmp;
        }
    }
}

pub fn save_best_phases(solver: &mut Solver) {
    let vars = solver.vars as usize;
    save_phases(&solver.values, &mut solver.phases.best[..vars]);
}

pub fn save_target_phases(solver: &mut Solver) {
    let vars = solver.vars as usize;
    save_phases(&solver.values, &mut solver.phases.target[..vars]);
}
