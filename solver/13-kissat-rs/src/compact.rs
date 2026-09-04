// Port of src/compact.c (kissat 4.0.4).
//
// PORT NOTES:
//  - The C `unsigned *mfixed_ptr` out-parameter of kissat_compact_literals
//    becomes the second element of the returned `(vars, mfixed)` tuple
//    (matching the collect.rs call site).
//  - kissat_map_literal (inline.h) is duplicated here as a private fn (it is
//    also private in collect.rs) — same body, same INVALID_LIT propagation.
//  - INC (compacted) is METRIC (no-op) and GET (compacted) yields u64::MAX
//    (kissat_phase then prints no count).
//  - `solver->compacting` only exists under LOGGING — omitted.
//  - compact_queue's C `unsigned *p` cursor (pointing either at queue.first
//    or at links[idx].next) is restructured as an explicit enum over the two
//    write targets, with identical order of effects.
//  - memset of the assigned/flags/values/watches tails clears the ranges to
//    Default (C zeroes the same byte ranges without releasing watch-vector
//    contents — the usable-word leak is ported as-is).
//  - SHRINK_STACK is capacity-only (no semantic effect) — omitted.

use crate::heap::Heap;
use crate::internal::Solver;
use crate::literal::{idx as idx_of, lit as lit_of, not, INVALID_IDX, INVALID_LIT};
use crate::queue::{disconnected, DISCONNECT};
use crate::utilities::percent;

// static void reimport_literal (kissat *solver, unsigned eidx, unsigned mlit)
fn reimport_literal(solver: &mut Solver, eidx: u32, mlit: u32) {
    let import = &mut solver.import_[eidx as usize];
    debug_assert!(import.imported);
    debug_assert!(!import.eliminated);
    import.lit = mlit;
}

/// Port of `kissat_compact_literals` (mfixed out-parameter returned).
pub fn compact_literals(solver: &mut Solver) -> (u32, u32) {
    // INC (compacted) — METRIC, no-op.
    let inactive = solver.vars - solver.active;
    let total_vars = solver.vars;
    crate::print::phase(
        solver,
        "compact",
        u64::MAX, // GET (compacted) — METRIC
        format_args!(
            "compacting garbage collection ({} inactive variables {:.2}%)",
            inactive,
            percent(inactive as f64, total_vars as f64)
        ),
    );
    let mut mfixed: u32 = INVALID_LIT;
    let mut vars: u32 = 0;
    for iidx in 0..solver.vars {
        let flags = solver.flags[iidx as usize];
        if flags.eliminated() {
            continue;
        }
        let ilit = lit_of(iidx);
        let mut mlit: u32;
        if flags.fixed() {
            let value = crate::internal::fixed(solver, ilit);
            debug_assert!(value != 0);
            if mfixed == INVALID_LIT {
                mlit = lit_of(vars);
                mfixed = mlit;
                if value < 0 {
                    mfixed = not(mfixed);
                }
                vars += 1;
            } else if value < 0 {
                mlit = not(mfixed);
            } else {
                mlit = mfixed;
            }
        } else if flags.active() {
            mlit = lit_of(vars);
            vars += 1;
        } else {
            let elit = solver.export_[iidx as usize];
            if elit != 0 {
                let eidx = elit.unsigned_abs(); // ABS (elit)
                let import = &mut solver.import_[eidx as usize];
                debug_assert!(import.imported);
                debug_assert!(!import.eliminated);
                import.imported = false;
                solver.export_[iidx as usize] = 0;
            }
            continue;
        }
        debug_assert!(mlit <= ilit);
        debug_assert!(mlit != not(ilit));
        if mlit == ilit {
            continue;
        }
        let elit = solver.export_[iidx as usize];
        let eidx = elit.unsigned_abs();
        if elit < 0 {
            mlit = not(mlit);
        }
        reimport_literal(solver, eidx, mlit);
    }
    debug_assert!(vars == solver.active || vars == solver.active + 1);
    (vars, mfixed)
}

// static void compact_literal (kissat *, unsigned dst_lit, unsigned src_lit)
fn compact_literal(solver: &mut Solver, dst_lit: u32, src_lit: u32) {
    debug_assert!(dst_lit < src_lit);
    debug_assert!(dst_lit != not(src_lit));
    let dst_idx = idx_of(dst_lit) as usize;
    let src_idx = idx_of(src_lit) as usize;
    debug_assert!(dst_idx != src_idx);
    solver.assigned[dst_idx] = solver.assigned[src_idx];
    solver.flags[dst_idx] = solver.flags[src_idx];

    solver.phases.best[dst_idx] = solver.phases.best[src_idx];
    solver.phases.saved[dst_idx] = solver.phases.saved[src_idx];
    solver.phases.target[dst_idx] = solver.phases.target[src_idx];

    let not_src_lit = not(src_lit) as usize;
    let not_dst_lit = not(dst_lit) as usize;
    solver.values[dst_lit as usize] = solver.values[src_lit as usize];
    solver.values[not_dst_lit] = solver.values[not_src_lit];
}

// inline.h kissat_map_literal — private duplicate (see module PORT NOTES).
fn map_literal(solver: &Solver, ilit: u32, map: bool) -> u32 {
    if !map {
        return ilit;
    }
    let iidx = idx_of(ilit);
    let mut elit = solver.export_[iidx as usize];
    if elit == 0 {
        return INVALID_LIT;
    }
    if crate::literal::negated(ilit) != 0 {
        elit = -elit;
    }
    let eidx = elit.unsigned_abs();
    let import = &solver.import_[eidx as usize];
    if import.eliminated {
        return INVALID_LIT;
    }
    let mut mlit = import.lit;
    if elit < 0 {
        mlit = not(mlit);
    }
    mlit
}

// static unsigned map_idx (kissat *solver, unsigned iidx)
fn map_idx(solver: &Solver, iidx: u32) -> u32 {
    let elit = solver.export_[iidx as usize];
    if elit == 0 {
        return INVALID_IDX;
    }
    let eidx = elit.unsigned_abs();
    debug_assert!(eidx != 0);
    let import = &solver.import_[eidx as usize];
    debug_assert!(import.imported);
    if import.eliminated {
        return INVALID_IDX;
    }
    let mlit = import.lit;
    let midx = idx_of(mlit);
    debug_assert!(midx <= iidx);
    midx
}

// static void compact_queue (kissat *solver)
fn compact_queue(solver: &mut Solver) {
    // unsigned *p = &solver->queue.first, prev = DISCONNECT;
    enum Cursor {
        First,
        Next(u32),
    }
    let mut prev: u32 = DISCONNECT;
    solver.queue.stamp = 0;
    let mut p = Cursor::First;
    loop {
        // for (unsigned idx; !DISCONNECTED (idx = *p); p = &l->next)
        let idx = match p {
            Cursor::First => solver.queue.first,
            Cursor::Next(i) => solver.links[i as usize].next,
        };
        if disconnected(idx) {
            break;
        }
        let midx = map_idx(solver, idx);
        debug_assert!(midx != INVALID_IDX);
        solver.queue.stamp += 1;
        let stamp = solver.queue.stamp;
        {
            let l = &mut solver.links[idx as usize];
            l.prev = prev;
            l.stamp = stamp;
        }
        if idx == solver.queue.search.idx {
            solver.queue.search.idx = midx;
            solver.queue.search.stamp = stamp;
        }
        // *p = prev = midx;
        match p {
            Cursor::First => solver.queue.first = midx,
            Cursor::Next(i) => solver.links[i as usize].next = midx,
        }
        prev = midx;
        p = Cursor::Next(idx);
    }
    solver.queue.last = prev;
    // *p = DISCONNECT;
    match p {
        Cursor::First => solver.queue.first = DISCONNECT,
        Cursor::Next(i) => solver.links[i as usize].next = DISCONNECT,
    }
    for idx in 0..solver.vars {
        let midx = map_idx(solver, idx);
        if midx == INVALID_IDX {
            continue;
        }
        solver.links[midx as usize] = solver.links[idx as usize];
    }
}

// static void compact_stack (kissat *solver, unsigneds *stack)
fn compact_stack(solver: &Solver, stack: &mut Vec<u32>) {
    let mut q = 0usize;
    for p in 0..stack.len() {
        let idx = stack[p];
        let midx = map_idx(solver, idx);
        if midx == INVALID_IDX {
            continue;
        }
        stack[q] = midx;
        q += 1;
    }
    stack.truncate(q); // SET_END_OF_STACK; SHRINK_STACK is capacity-only.
}

// static void compact_scores (kissat *solver, heap *old_scores, unsigned vars)
fn compact_scores(solver: &mut Solver, vars: u32) {
    // heap new_scores; memset 0; kissat_resize_heap (&new_scores, vars);
    let old_scores = std::mem::take(&mut solver.scores); // SCORES
    let mut new_scores = Heap::default();
    crate::heap::resize_heap(&mut new_scores, vars);

    if old_scores.tainted {
        for idx in 0..solver.vars {
            let midx = map_idx(solver, idx);
            if midx == INVALID_IDX {
                continue;
            }
            let score = crate::heap::get_heap_score(&old_scores, idx);
            crate::heap::update_heap(&mut new_scores, midx, score);
        }
    }

    for i in 0..old_scores.stack.len() {
        let idx = old_scores.stack[i];
        let midx = map_idx(solver, idx);
        if midx == INVALID_IDX {
            continue;
        }
        crate::heap::push_heap(&mut new_scores, midx);
    }

    // kissat_release_heap (old_scores); *old_scores = new_scores;
    drop(old_scores);
    solver.scores = new_scores;
}

// static void compact_trail (kissat *solver)
fn compact_trail(solver: &mut Solver) {
    let size = solver.trail.len();
    for i in 0..size {
        let ilit = solver.trail[i];
        let mlit = map_literal(solver, ilit, true);
        debug_assert!(mlit != INVALID_LIT);
        solver.trail[i] = mlit;
        let idx = idx_of(ilit);
        if !solver.assigned[idx as usize].binary() {
            continue;
        }
        let other = solver.assigned[idx as usize].reason;
        let mother = map_literal(solver, other, true);
        debug_assert!(mother != INVALID_LIT);
        solver.assigned[idx as usize].reason = mother;
    }
}

// static void compact_frames (kissat *solver)
fn compact_frames(solver: &mut Solver) {
    let size = solver.frames.len();
    for level in 1..size {
        let ilit = solver.frames[level].decision;
        let mlit = map_literal(solver, ilit, true);
        debug_assert!(mlit != INVALID_LIT);
        solver.frames[level].decision = mlit;
    }
}

// static void compact_export (kissat *solver, unsigned vars)
fn compact_export(solver: &mut Solver, vars: u32) {
    let size = solver.export_.len();
    debug_assert!(size == solver.vars as usize);
    for iidx in 0..size as u32 {
        let elit = solver.export_[iidx as usize];
        if elit == 0 {
            continue;
        }
        let midx = map_idx(solver, iidx);
        if midx == INVALID_IDX {
            continue;
        }
        solver.export_[midx as usize] = elit;
    }
    solver.export_.truncate(vars as usize); // RESIZE_STACK; SHRINK_STACK n/a.
}

// static void compact_units (kissat *solver, unsigned mfixed)
fn compact_units(solver: &mut Solver, mfixed: u32) {
    debug_assert!(crate::internal::fixed(solver, mfixed) > 0);
    for i in 0..solver.units.len() {
        let elit = solver.units[i];
        let eidx = elit.unsigned_abs();
        let mlit = if elit < 0 { not(mfixed) } else { mfixed };
        let import = &solver.import_[eidx as usize];
        debug_assert!(import.imported);
        debug_assert!(!import.eliminated);
        let ilit = import.lit;
        if mlit != ilit {
            reimport_literal(solver, eidx, mlit);
        }
    }
}

// static void compact_best_and_target_values (kissat *solver, unsigned vars)
fn compact_best_and_target_values(solver: &mut Solver, vars: u32) {
    let mut best_assigned: u32 = 0;
    let mut target_assigned: u32 = 0;

    for idx in 0..vars as usize {
        if !solver.flags[idx].active() {
            continue;
        }
        if solver.phases.target[idx] != 0 {
            target_assigned += 1;
        }
        if solver.phases.best[idx] != 0 {
            best_assigned += 1;
        }
    }

    if solver.target_assigned != target_assigned {
        solver.target_assigned = target_assigned;
    }

    if solver.best_assigned != best_assigned {
        solver.best_assigned = best_assigned;
    }
}

/// Port of `kissat_finalize_compacting`.
pub fn finalize_compacting(solver: &mut Solver, vars: u32, mfixed: u32) {
    debug_assert!(vars <= solver.vars);
    if vars == solver.vars {
        return;
    }

    let reduced = solver.vars - vars;

    let mut first = true;
    for iidx in 0..solver.vars {
        let flags = solver.flags[iidx as usize];
        if flags.fixed() && first {
            first = false;
        } else if !flags.active() {
            solver.export_[iidx as usize] = 0;
        }
    }

    compact_trail(solver);

    for iidx in 0..solver.vars {
        let ilit = lit_of(iidx);
        let mlit = map_literal(solver, ilit, true);
        if mlit != INVALID_LIT && ilit != mlit {
            compact_literal(solver, mlit, ilit);
        }
    }

    if mfixed != INVALID_LIT {
        compact_units(solver, mfixed);
    }

    // memset (solver->assigned + vars, 0, reduced * sizeof (assigned)); etc.
    for i in vars..solver.vars {
        solver.assigned[i as usize] = Default::default();
        solver.flags[i as usize] = Default::default();
    }
    for i in (2 * vars)..(2 * solver.vars) {
        solver.values[i as usize] = 0;
        solver.watches[i as usize] = Default::default();
    }
    let _ = reduced;

    compact_queue(solver);
    // compact_stack (solver, &solver->sweep_schedule);
    {
        let mut sweep_schedule = std::mem::take(&mut solver.sweep_schedule);
        compact_stack(solver, &mut sweep_schedule);
        solver.sweep_schedule = sweep_schedule;
    }
    compact_scores(solver, vars); // compact_scores (solver, SCORES, vars)
    compact_frames(solver);
    compact_export(solver, vars);
    compact_best_and_target_values(solver, vars);

    solver.vars = vars;
    crate::resize::decrease_size(solver);
}
