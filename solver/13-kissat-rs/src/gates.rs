// Port of src/gates.c (kissat 4.0.4).
//
// Gate extraction driver for bounded variable elimination: tries equivalence,
// AND (both phases), if-then-else (both phases) and kitten-based definition
// extraction, then splits the literal's occurrence lists into gate and
// antecedent (non-gate) clauses.
//
// PORT NOTE: GATE_ELIMINATED (NAME) is `true` in the non-METRICS reference
// build, so `solver->gate_eliminated` is a plain bool here (see internal.rs).
// PORT NOTE: get_antecedents' two-cursor merge compares raw watch words
// (`g->raw == watch.raw`); Watch is a u32 word so this is `==`.

use crate::internal::Solver;
use crate::watch::{watch_is_binary, watch_lit};

/// Port of `kissat_mark_binaries`.  Returns the number of newly marked
/// literals (C: size_t).
pub fn mark_binaries(solver: &mut Solver, lit: u32) -> u64 {
    let mut res: u64 = 0;
    let v = solver.watches[lit as usize];
    for &watch in &solver.vectors.stack[v.begin..v.end] {
        if !watch_is_binary(watch) {
            continue;
        }
        let other = watch_lit(watch);
        debug_assert!(solver.values[other as usize] == 0);
        if solver.marks[other as usize] != 0 {
            continue;
        }
        solver.marks[other as usize] = 1;
        res += 1;
    }
    res
}

/// Port of `kissat_unmark_binaries`.
pub fn unmark_binaries(solver: &mut Solver, lit: u32) {
    let v = solver.watches[lit as usize];
    for &watch in &solver.vectors.stack[v.begin..v.end] {
        if watch_is_binary(watch) {
            solver.marks[watch_lit(watch) as usize] = 0;
        }
    }
}

/// Port of `kissat_find_gates`.
pub fn find_gates(solver: &mut Solver, lit: u32) -> bool {
    solver.gate_eliminated = false; // solver->gate_eliminated = 0
    solver.resolve_gate = false;
    if solver.options.extract == 0 {
        return false;
    }
    // INC (gates_checked): METRIC, compiled out.
    let not_lit = crate::literal::not(lit);
    if solver.watches[not_lit as usize].empty() {
        return false;
    }
    let mut res = false;
    if crate::equivalences::find_equivalence_gate(solver, lit) {
        res = true;
    } else if crate::ands::find_and_gate(solver, lit, 0) {
        res = true;
    } else if crate::ands::find_and_gate(solver, not_lit, 1) {
        res = true;
    } else if crate::ifthenelse::find_if_then_else_gate(solver, lit, 0) {
        res = true;
    } else if crate::ifthenelse::find_if_then_else_gate(solver, not_lit, 1) {
        res = true;
    } else if crate::definition::find_definition(solver, lit) {
        res = true;
    }
    // if (res) INC (gates_extracted): METRIC, compiled out.
    res
}

// static get_antecedents (one sign)
fn get_antecedents_one(solver: &mut Solver, lit: u32, negative: u32) {
    debug_assert!(!solver.watching);
    debug_assert!(negative == 0 || negative == 1);
    let negative = negative as usize;
    debug_assert!(solver.antecedents[negative].is_empty());

    let v = solver.watches[lit as usize];
    let mut g = 0usize; // cursor into solver.gates[negative]
    for &watch in &solver.vectors.stack[v.begin..v.end] {
        if g != solver.gates[negative].len() && solver.gates[negative][g] == watch {
            g += 1;
        } else {
            solver.antecedents[negative].push(watch);
        }
    }
    debug_assert!(g == solver.gates[negative].len());
}

/// Port of `kissat_get_antecedents`.
pub fn get_antecedents(solver: &mut Solver, lit: u32) {
    get_antecedents_one(solver, lit, 0);
    get_antecedents_one(solver, crate::literal::not(lit), 1);
}
