// Port of src/backtrack.c (kissat 4.0.4).
//
// PORT NOTES:
//  - C `static void kissat_update_target_and_best_phases` is a *static*
//    function despite its kissat_ prefix; ported as the private
//    `update_target_and_best_phases`.
//  - The C two-cursor trail compaction walks `p` over the old trail while
//    writing through `q`; here both are indices into solver.trail with the
//    exact same order of reads/writes.
//  - SET_END_OF_STACK (solver->frames, new_frame) keeps the frame at index
//    `new_level + 1` readable through the (dangling-but-valid) C pointer
//    `new_frame` after the truncation; the port reads `new_frame.trail`
//    before truncating.
//  - INC (target_saved) / INC (best_saved) are METRIC counters — no-ops in
//    the reference build.
//  - kissat_backtrack_propagate_and_flush_trail: the NDEBUG build drops the
//    `clause *conflict =` binding and the trailing asserts but keeps the
//    propagation call itself (side effects).  kissat_probing_propagate is
//    stubbed until the proprobe wave lands (crate::proprobe).

use crate::heap;
use crate::internal::{assigned as kissat_assigned, Solver};
use crate::literal;
use crate::print;

// C static `unassign` (values array argument folded into direct field
// access; call sites inline the same effect order).
#[inline]
fn unassign(solver: &mut Solver, lit: u32) {
    debug_assert!(solver.values[lit as usize] > 0);
    let not_lit = literal::not(lit);
    unsafe {
        *solver.values.get_unchecked_mut(lit as usize) = 0;
        *solver.values.get_unchecked_mut(not_lit as usize) = 0;
    }
    debug_assert!(solver.unassigned < solver.vars());
    solver.unassigned += 1;
}

// C static `add_unassigned_variable_back_to_queue`.
#[inline]
fn add_unassigned_variable_back_to_queue(solver: &mut Solver, lit: u32) {
    debug_assert!(!solver.stable);
    let idx = literal::idx(lit);
    if solver.links[idx as usize].stamp > solver.queue.search.stamp {
        crate::inlinequeue::update_queue(solver, idx);
    }
}

// C static `add_unassigned_variable_back_to_heap`.
#[inline]
fn add_unassigned_variable_back_to_heap(solver: &mut Solver, lit: u32) {
    debug_assert!(solver.stable);
    let idx = literal::idx(lit);
    if !heap::heap_contains(&solver.scores, idx) {
        heap::push_heap(&mut solver.scores, idx);
    }
}

// C static `kissat_update_target_and_best_phases` (see PORT NOTE).
fn update_target_and_best_phases(solver: &mut Solver) {
    if solver.probing {
        return;
    }
    if !solver.stable {
        return;
    }

    let assigned = kissat_assigned(solver);

    if solver.target_assigned < assigned {
        print::extremely_verbose(
            solver,
            format_args!(
                "updating target assigned trail height from {} to {}",
                solver.target_assigned, assigned
            ),
        );
        solver.target_assigned = assigned;
        crate::phases::save_target_phases(solver);
        // INC (target_saved): METRIC — no-op.
    }

    if solver.best_assigned < assigned {
        print::extremely_verbose(
            solver,
            format_args!(
                "updating best assigned trail height from {} to {}",
                solver.best_assigned, assigned
            ),
        );
        solver.best_assigned = assigned;
        crate::phases::save_best_phases(solver);
        // INC (best_saved): METRIC — no-op.
    }
}

/// Port of `kissat_backtrack_without_updating_phases`.
pub fn backtrack_without_updating_phases(solver: &mut Solver, new_level: u32) {
    debug_assert!(solver.level >= new_level);
    if solver.level == new_level {
        return;
    }

    // frame *new_frame = &FRAME (new_level + 1);
    // SET_END_OF_STACK (solver->frames, new_frame);
    let new_frame_trail = solver.frames[(new_level + 1) as usize].trail;
    solver.frames.truncate((new_level + 1) as usize);

    let new_end = new_frame_trail as usize;
    let old_end = solver.trail.len();

    let mut q = new_end;
    if solver.stable {
        for p in new_end..old_end {
            let lit = unsafe { *solver.trail.get_unchecked(p) };
            let idx = literal::idx(lit);
            debug_assert!(idx < solver.vars());
            let a = unsafe { solver.assigned.get_unchecked_mut(idx as usize) };
            let level = a.level;
            if level <= new_level {
                let new_trail = q as u32;
                debug_assert!(new_trail <= a.trail);
                a.trail = new_trail;
                unsafe { *solver.trail.get_unchecked_mut(q) = lit };
                q += 1;
            } else {
                unassign(solver, lit);
                add_unassigned_variable_back_to_heap(solver, lit);
            }
        }
    } else {
        for p in new_end..old_end {
            let lit = unsafe { *solver.trail.get_unchecked(p) };
            let idx = literal::idx(lit);
            debug_assert!(idx < solver.vars());
            let a = unsafe { solver.assigned.get_unchecked_mut(idx as usize) };
            let level = a.level;
            if level <= new_level {
                let new_trail = q as u32;
                debug_assert!(new_trail <= a.trail);
                a.trail = new_trail;
                unsafe { *solver.trail.get_unchecked_mut(q) = lit };
                q += 1;
            } else {
                unassign(solver, lit);
                add_unassigned_variable_back_to_queue(solver, lit);
            }
        }
    }
    solver.trail.truncate(q); // SET_END_OF_ARRAY (solver->trail, q)

    solver.level = new_level;

    // solver->propagate = new_end (propagation resumes there).
    debug_assert!(new_end <= solver.trail.len());
    solver.propagate = new_end;

    debug_assert!(!solver.extended);
}

/// Port of `kissat_backtrack_in_consistent_state`.
pub fn backtrack_in_consistent_state(solver: &mut Solver, new_level: u32) {
    update_target_and_best_phases(solver);
    backtrack_without_updating_phases(solver, new_level);
}

/// Port of `kissat_backtrack_after_conflict`.
pub fn backtrack_after_conflict(solver: &mut Solver, new_level: u32) {
    if solver.level != 0 {
        backtrack_without_updating_phases(solver, solver.level - 1);
    }
    update_target_and_best_phases(solver);
    backtrack_without_updating_phases(solver, new_level);
}

/// Port of `kissat_backtrack_propagate_and_flush_trail`.
pub fn backtrack_propagate_and_flush_trail(solver: &mut Solver) {
    if solver.level != 0 {
        debug_assert!(solver.watching);
        backtrack_in_consistent_state(solver, 0);
        // NDEBUG drops the `clause *conflict =` binding but keeps the call:
        let conflict = if solver.probing {
            crate::proprobe::probing_propagate(solver, crate::reference::INVALID_REF, true)
        } else {
            crate::propsearch::search_propagate(solver)
        };
        debug_assert!(conflict.is_none());
        let _ = conflict;
    }
    // assert (kissat_propagated / kissat_trail_flushed): NDEBUG — omitted.
}
