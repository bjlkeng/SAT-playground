// Port of src/flags.h + src/flags.c (kissat 4.0.4).
// PORT NOTE: the C struct uses one-bit bitfields -> plain bools, except
// `unsigned factor : 2` which becomes a `u8` holding the same 2-bit mask
// (bit 0 = positive literal marked, bit 1 = negated literal marked; see
// kissat_mark_added_literal in inline.h).
// PORT NOTE: C `deactivate_variable` takes `flags *f` plus the idx; the
// Rust version takes only the idx and re-derives the flags entry, to
// satisfy the borrow checker (same effects, same order).
// PORT NOTE: heap calls (kissat_update_heap / kissat_push_heap /
// kissat_pop_heap / kissat_heap_contains) are assumed to be exposed as
// `crate::heap::{update_heap, push_heap, pop_heap, heap_contains}` taking
// `&mut Heap` / `&Heap` WITHOUT the solver argument (the C solver argument
// is only used for allocation/logging, both irrelevant here).
// PORT NOTE: kissat_enqueue/kissat_dequeue live in inlinequeue.h ->
// `crate::inlinequeue`; kissat_export_literal, kissat_mark_removed_literal
// and kissat_mark_added_literal live in inline.h -> `crate::inline`.

use crate::internal::Solver;

/// C `struct flags` (12 one/two-bit bitfields, sizeof == 2).  Packed into a
/// u16 with accessors: the earlier one-bool-per-field layout was 10 bytes,
/// so every var-indexed scan (factor's schedule_factorization, backbone,
/// sweep, eliminate) streamed 5x the C's cache lines (crusti: 44% of
/// factor's cycles in that scan). Bit layout mirrors the C declaration
/// order; `factor` is the 2-bit mask (bit 0 = positive literal marked,
/// bit 1 = negated literal marked; see kissat_mark_added_literal).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct Flags(u16);

macro_rules! flag_bit {
    ($get:ident, $set:ident, $bit:expr) => {
        #[inline(always)]
        pub fn $get(&self) -> bool {
            self.0 & (1 << $bit) != 0
        }
        #[inline(always)]
        pub fn $set(&mut self, v: bool) {
            self.0 = (self.0 & !(1 << $bit)) | ((v as u16) << $bit);
        }
    };
}

impl Flags {
    flag_bit!(active, set_active, 0);
    flag_bit!(backbone0, set_backbone0, 1);
    flag_bit!(backbone1, set_backbone1, 2);
    flag_bit!(eliminate, set_eliminate, 3);
    flag_bit!(eliminated, set_eliminated, 4);
    // bits 5-6: `unsigned factor : 2`
    flag_bit!(fixed, set_fixed, 7);
    flag_bit!(subsume, set_subsume, 8);
    flag_bit!(sweep, set_sweep, 9);
    flag_bit!(transitive, set_transitive, 10);
    #[inline(always)]
    pub fn factor(&self) -> u8 {
        ((self.0 >> 5) & 3) as u8
    }
    #[inline(always)]
    pub fn set_factor(&mut self, v: u8) {
        debug_assert!(v < 4);
        self.0 = (self.0 & !(3 << 5)) | (((v & 3) as u16) << 5);
    }
    #[inline(always)]
    pub fn factor_or(&mut self, bits: u8) {
        self.set_factor(self.factor() | bits);
    }
    #[inline(always)]
    pub fn factor_and(&mut self, bits: u8) {
        self.set_factor(self.factor() & bits);
    }
}
const _: () = assert!(std::mem::size_of::<Flags>() == 2);

// C static inline `activate_literal` (renamed to avoid colliding with the
// public kissat_activate_literal wrapper below after prefix-dropping).
fn activate_literal_inner(solver: &mut Solver, lit: u32) {
    let idx = crate::literal::idx(lit);
    if solver.flags[idx as usize].active() {
        return;
    }
    let lit = crate::literal::strip(lit);
    solver.flags[idx as usize].set_active(true);
    solver.active += 1;
    solver.statistics.variables_activated += 1;
    crate::inlinequeue::enqueue(solver, idx);
    let score = 1.0 - 1.0 / solver.statistics.variables_activated as f64;
    crate::heap::update_heap(&mut solver.scores, idx, score);
    if solver.stable {
        let lit = crate::literal::lit(idx);
        if solver.values[lit as usize] == 0 {
            crate::heap::push_heap(&mut solver.scores, idx);
        }
    }
    solver.unassigned += 1;
    crate::inline::mark_removed_literal(solver, lit);
    crate::inline::mark_added_literal(solver, lit);
}

// C static inline `deactivate_variable` (flags pointer argument dropped,
// see PORT NOTE above).
fn deactivate_variable(solver: &mut Solver, idx: u32) {
    debug_assert!(solver.flags[idx as usize].active());
    debug_assert!(
        solver.flags[idx as usize].eliminated() || solver.flags[idx as usize].fixed()
    );
    solver.flags[idx as usize].set_active(false);
    debug_assert!(solver.active > 0);
    solver.active -= 1;
    crate::inlinequeue::dequeue(solver, idx);
    if crate::heap::heap_contains(&solver.scores, idx) {
        crate::heap::pop_heap(&mut solver.scores, idx);
    }
}

pub fn activate_literal(solver: &mut Solver, lit: u32) {
    activate_literal_inner(solver, lit);
}

pub fn activate_literals(solver: &mut Solver, size: u32, lits: &[u32]) {
    for i in 0..size as usize {
        activate_literal_inner(solver, lits[i]);
    }
}

pub fn mark_fixed_literal(solver: &mut Solver, lit: u32) {
    debug_assert!(solver.values[lit as usize] > 0);
    let idx = crate::literal::idx(lit);
    debug_assert!(solver.flags[idx as usize].active());
    debug_assert!(!solver.flags[idx as usize].eliminated());
    debug_assert!(!solver.flags[idx as usize].fixed());
    solver.flags[idx as usize].set_fixed(true);
    deactivate_variable(solver, idx);
    solver.statistics.units += 1;
    let elit = crate::inline::export_literal(solver, lit);
    debug_assert!(elit != 0);
    solver.units.push(elit);
}

pub fn mark_eliminated_variable(solver: &mut Solver, idx: u32) {
    let lit = crate::literal::lit(idx);
    debug_assert!(solver.values[lit as usize] == 0);
    debug_assert!(solver.flags[idx as usize].active());
    debug_assert!(!solver.flags[idx as usize].eliminated());
    debug_assert!(!solver.flags[idx as usize].fixed());
    solver.flags[idx as usize].set_eliminated(true);
    deactivate_variable(solver, idx);
    let elit = crate::inline::export_literal(solver, lit);
    debug_assert!(elit != 0);
    debug_assert!(elit != i32::MIN);
    let eidx = elit.unsigned_abs();
    let pos = solver.eliminated.len();
    debug_assert!(pos < (1usize << 30));
    let import = &mut solver.import_[eidx as usize];
    debug_assert!(!import.eliminated);
    debug_assert!(import.imported);
    import.lit = pos as u32;
    import.eliminated = true;
    solver.eliminated.push(0 as crate::value::Value);
    debug_assert!(solver.unassigned > 0);
    solver.unassigned -= 1;
}

pub fn mark_removed_literals(solver: &mut Solver, size: u32, lits: &[u32]) {
    for i in 0..size as usize {
        crate::inline::mark_removed_literal(solver, lits[i]);
    }
}

pub fn mark_added_literals(solver: &mut Solver, size: u32, lits: &[u32]) {
    for i in 0..size as usize {
        crate::inline::mark_added_literal(solver, lits[i]);
    }
}
