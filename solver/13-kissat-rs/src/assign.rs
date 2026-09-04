// Port of src/assign.c + the function parts of src/assign.h, plus
// src/inlineassign.h and src/fastassign.h (kissat 4.0.4).
//
// The `Assigned` struct and the DECISION_REASON / UNIT_REASON /
// INVALID_LEVEL / INVALID_TRAIL constants live in internal.rs (the struct
// part of assign.h).
//
// PORT NOTES:
//  - C compiles kissat_assign twice: plain (inlineassign.h, used by
//    assign.c) and as kissat_fast_assign (FAST_ASSIGN via fastassign.h,
//    used by the propagation inner loops), whose ONLY difference is that
//    the values/assigned base pointers are passed in by the caller instead
//    of reloaded from solver.  Passing aliasing raw pointers alongside
//    `&mut Solver` is UB-prone in Rust and reloading a Vec base pointer is
//    a single field load, so both collapse to the one `assign` below; the
//    fast_* wrappers keep the C call-site names, argument order (minus the
//    pointer pair) and semantics.
//  - The `#if !defined(PROBING_PROPAGATION)` guard around the saved-phase
//    store in inlineassign.h is a compile-time specialization for
//    proprobe.c only; there `solver->probing` is always true, so the
//    runtime `if !probing` below is exactly equivalent.
//  - `__builtin_prefetch (w, 0, 1)` on the watch list of NOT(lit) is ported
//    as _mm_prefetch::<_MM_HINT_T2> on x86_64 (pure performance hint, no
//    semantics).
//  - kissat_assign_unit keeps the C `const char *reason` parameter for
//    call-site fidelity; it is LOGGING-only and unused here.
//  - kissat_assignment_level / kissat_assign_reference take the clause as
//    its Reference; the C `clause *` is always re-derivable from it (see
//    the internal.rs call-site PORT NOTE).
//  - INC (jumped_reasons) is STATISTIC-tier: compiled out in the reference
//    build but kept as a real counter per statistics.rs policy.

use crate::internal::{Assigned, Solver, DECISION_REASON, UNIT_REASON};
use crate::reference::Reference;

/// kissat_assign (inlineassign.h) == kissat_fast_assign (fastassign.h).
#[inline(always)]
pub fn assign(
    solver: &mut Solver,
    probing: bool,
    level: u32,
    mut binary: bool,
    lit: u32,
    mut reason: u32,
) {
    let not_lit = crate::literal::not(lit);

    // watches watches = WATCHES (not_lit);
    // if (!kissat_empty_vector (&watches)) __builtin_prefetch (w, 0, 1);
    {
        let w = solver.watches[not_lit as usize];
        if !w.empty() {
            #[cfg(target_arch = "x86_64")]
            unsafe {
                use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T2};
                _mm_prefetch::<{ _MM_HINT_T2 }>(
                    solver.vectors.stack.as_ptr().add(w.begin) as *const i8
                );
            }
        }
    }

    debug_assert!(solver.values[lit as usize] == 0);
    debug_assert!(solver.values[not_lit as usize] == 0);
    unsafe {
        *solver.values.get_unchecked_mut(lit as usize) = 1;
        *solver.values.get_unchecked_mut(not_lit as usize) = -1;
    }

    debug_assert!(solver.unassigned > 0);
    solver.unassigned -= 1;

    if level == 0 {
        crate::flags::mark_fixed_literal(solver, lit);
        debug_assert!(solver.unflushed < u32::MAX);
        solver.unflushed += 1;
        if reason != UNIT_REASON {
            // CHECK_AND_ADD_UNIT: compiled out (NDEBUG).
            // ADD_UNIT_TO_PROOF:
            if solver.proof.is_some() {
                crate::proof::add_unit_to_proof(solver, lit);
            }
            reason = UNIT_REASON;
            binary = false;
        }
    }

    let trail = solver.trail.len() as u32; // SIZE_ARRAY (solver->trail)
    // PUSH_ARRAY (solver->trail, lit): unchecked, like the C's `*end++ = lit`
    // — resize.rs keeps capacity >= size and a variable is assigned at most
    // once, so len < size <= capacity holds here.
    debug_assert!((trail as usize) < solver.trail.capacity());
    unsafe {
        std::ptr::write(solver.trail.as_mut_ptr().add(trail as usize), lit);
        solver.trail.set_len(trail as usize + 1);
    }

    let idx = crate::literal::idx(lit);

    // #if !defined(PROBING_PROPAGATION) — see module PORT NOTES.
    if !probing {
        let negated = crate::literal::negated(lit) != 0;
        let new_value = crate::value::bool_to_value(negated);
        // *saved = new_value;  (SAVED (idx))
        unsafe {
            *solver.phases.saved.get_unchecked_mut(idx as usize) = new_value;
        }
    }

    let b = Assigned::new(level, trail, binary, reason);
    unsafe {
        *solver.assigned.get_unchecked_mut(idx as usize) = b;
    }
}

/// kissat_assign_unit.  `_reason` is the C LOGGING-only description string.
pub fn assign_unit(solver: &mut Solver, lit: u32, _reason: &str) {
    let probing = solver.probing;
    assign(solver, probing, 0, false, lit, UNIT_REASON);
}

/// kissat_learned_unit.
pub fn learned_unit(solver: &mut Solver, lit: u32) {
    assign_unit(solver, lit, "learned reason");
    // CHECK_AND_ADD_UNIT: compiled out (NDEBUG).
    // ADD_UNIT_TO_PROOF:
    if solver.proof.is_some() {
        crate::proof::add_unit_to_proof(solver, lit);
    }
}

/// kissat_original_unit.
pub fn original_unit(solver: &mut Solver, lit: u32) {
    assign_unit(solver, lit, "original reason");
}

/// kissat_assign_decision.
pub fn assign_decision(solver: &mut Solver, lit: u32) {
    let probing = solver.probing;
    let level = solver.level;
    assign(solver, probing, level, false, lit, DECISION_REASON);
}

/// kissat_assign_binary (assign.c).  NOTE: unlike the fast path below, the
/// reason-jump here does NOT check classification.bigbig — quirk ported.
pub fn assign_binary(solver: &mut Solver, lit: u32, mut other: u32) {
    debug_assert!(solver.values[other as usize] < 0);
    let other_idx = crate::literal::idx(other);
    let a = solver.assigned[other_idx as usize];
    let level = a.level;
    if solver.options.jumpreasons != 0 && level != 0 && a.binary() {
        solver.statistics.jumped_reasons += 1; // INC (jumped_reasons)
        other = a.reason;
    }
    // kissat_assign (solver, solver->probing, a->level, true, lit, other):
    let probing = solver.probing;
    assign(solver, probing, level, true, lit, other);
}

/// kissat_assignment_level (inlineassign.h).  C signature is
/// (solver, values, assigned, lit, clause *reason); values/assigned always
/// alias solver's own arrays and the clause comes in as its reference.
#[inline(always)]
pub fn assignment_level(solver: &Solver, lit: u32, ref_: Reference) -> u32 {
    let c = solver.arena.clause(ref_);
    let mut res: u32 = 0;
    for &other in c.lits() {
        if other == lit {
            continue;
        }
        debug_assert!(solver.values[other as usize] < 0);
        let other_idx = crate::literal::idx(other);
        let level = unsafe { solver.assigned.get_unchecked(other_idx as usize).level };
        if res < level {
            res = level;
        }
    }
    res
}

/// kissat_assign_reference (assign.c).
pub fn assign_reference(solver: &mut Solver, lit: u32, ref_: Reference) {
    let level = assignment_level(solver, lit, ref_);
    debug_assert!(level <= solver.level);
    debug_assert!(ref_ != DECISION_REASON);
    debug_assert!(ref_ != UNIT_REASON);
    let probing = solver.probing;
    assign(solver, probing, level, false, lit, ref_);
}

/// kissat_fast_binary_assign (fastassign.h) — the propagation inner-loop
/// binary assign.  NOTE: reason jumping here additionally requires
/// solver.classification.bigbig (unlike kissat_assign_binary above).
#[inline(always)]
pub fn fast_binary_assign(
    solver: &mut Solver,
    probing: bool,
    level: u32,
    lit: u32,
    mut other: u32,
) {
    if solver.options.jumpreasons != 0 && level != 0 && solver.classification.bigbig {
        let other_idx = crate::literal::idx(other);
        let a = unsafe { *solver.assigned.get_unchecked(other_idx as usize) };
        if a.binary() {
            solver.statistics.jumped_reasons += 1; // INC (jumped_reasons)
            other = a.reason;
        }
    }
    assign(solver, probing, level, true, lit, other);
}

/// kissat_fast_assign_reference (fastassign.h).
#[inline(always)]
pub fn fast_assign_reference(solver: &mut Solver, lit: u32, ref_: Reference) {
    let level = assignment_level(solver, lit, ref_);
    debug_assert!(level <= solver.level);
    debug_assert!(ref_ != DECISION_REASON);
    debug_assert!(ref_ != UNIT_REASON);
    let probing = solver.probing;
    assign(solver, probing, level, false, lit, ref_);
}
