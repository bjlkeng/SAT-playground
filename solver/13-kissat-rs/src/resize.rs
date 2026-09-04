// Port of src/resize.c (kissat 4.0.4).
//
// PORT NOTES:
//  - CREALLOC_* (calloc + copy) → Vec::resize with zeroed/default tails;
//    NREALLOC_* (realloc, tail uninitialized in C, always written before
//    read) → Vec::resize with default tails too (semantically identical).
//    Capacity policy is free per CONVENTIONS.md; only the enlarge cadence of
//    kissat_enlarge_variables (power-of-two doubling) is observable and is
//    ported exactly.
//  - reallocate_trail: the C trail is a fixed preallocated array with a
//    `propagate` cursor pointer; the port's trail is a Vec plus the
//    `propagate` index (internal.rs), so reallocation preserves the cursor
//    by construction and needs no code.
//  - decrease_size truncates (NREALLOC to the smaller size keeps the prefix).

use crate::internal::Solver;

/// Port of `kissat_increase_size`.
pub fn increase_size(solver: &mut Solver, new_size: u32) {
    debug_assert!(solver.vars <= new_size);
    let old_size = solver.size;
    if old_size >= new_size {
        return;
    }

    let n = new_size as usize;

    // CREALLOC_VARIABLE_INDEXED (assigned, assigned);
    solver.assigned.resize(n, Default::default());
    // CREALLOC_VARIABLE_INDEXED (flags, flags);
    solver.flags.resize(n, Default::default());
    // NREALLOC_VARIABLE_INDEXED (links, links);
    solver.links.resize(n, Default::default());

    // CREALLOC_LITERAL_INDEXED (mark, marks);
    solver.marks.resize(2 * n, 0);
    // CREALLOC_LITERAL_INDEXED (value, values);
    solver.values.resize(2 * n, 0);
    // CREALLOC_LITERAL_INDEXED (watches, watches);
    solver.watches.resize(2 * n, Default::default());

    // reallocate_trail (solver, old_size, new_size): the C trail is a fixed
    // array of `size` entries so PUSH_ARRAY is an unchecked store; keep the
    // Vec's capacity at >= size so assign.rs can push unchecked too.
    if solver.trail.capacity() < n {
        let len = solver.trail.len();
        solver.trail.reserve_exact(n - len);
    }
    debug_assert!(solver.trail.capacity() >= n);
    crate::heap::resize_heap(&mut solver.scores, new_size); // kissat_resize_heap (SCORES)
    crate::phases::increase_phases(solver, new_size);

    solver.size = new_size;
}

/// Port of `kissat_decrease_size`.
pub fn decrease_size(solver: &mut Solver) {
    let old_size = solver.size;
    let new_size = solver.vars;
    let _ = old_size;

    let n = new_size as usize;

    // NREALLOC_VARIABLE_INDEXED (assigned/flags/links);
    solver.assigned.truncate(n);
    solver.flags.truncate(n);
    solver.links.truncate(n);

    // NREALLOC_LITERAL_INDEXED (marks/values/watches);
    solver.marks.truncate(2 * n);
    solver.values.truncate(2 * n);
    solver.watches.truncate(2 * n);

    // reallocate_trail (solver, old_size, new_size): no-op (see PORT NOTES).
    crate::heap::resize_heap(&mut solver.scores, new_size); // grow-only: no-op here
    crate::phases::decrease_phases(solver, new_size);

    solver.size = new_size;
}

/// Port of `kissat_enlarge_variables`.
pub fn enlarge_variables(solver: &mut Solver, new_vars: u32) {
    if solver.vars >= new_vars {
        return;
    }
    debug_assert!(new_vars <= crate::literal::INTERNAL_MAX_VAR + 1);
    let old_size = solver.size as u64;
    if old_size < new_vars as u64 {
        let new_size: u64;
        if old_size == 0 {
            new_size = new_vars as u64;
        } else {
            // kissat_is_power_of_two (old_size)
            let mut ns: u64 = if old_size & (old_size - 1) == 0 {
                debug_assert!(old_size <= u32::MAX as u64 / 2);
                2 * old_size
            } else {
                debug_assert!(old_size > 1);
                2
            };
            while ns < new_vars as u64 {
                debug_assert!(ns <= u32::MAX as u64 / 2);
                ns *= 2;
            }
            new_size = ns;
        }
        increase_size(solver, new_size as u32);
    }
    solver.vars = new_vars;
}
