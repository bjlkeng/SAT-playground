// Port of src/bump.c + src/bump.h (kissat 4.0.4).
//
// PORT NOTES:
//  - SORT_STACK / RADIX_STACK carry START/STOP (sort) / (radix) profile
//    hooks (level 4) inside the C macros; they are hoisted around the calls
//    here exactly once per sort, per the crate::sort convention.
//  - INC (rescaled) is a METRIC counter — no-op; GET (rescaled) on a METRIC
//    yields u64::MAX, which kissat_phase renders as "no count" (the Rust
//    print::phase does the same for u64::MAX).
//  - ADD (literals_bumped, ..) is METRIC — no-op.
//  - The heap ops go through crate::heap free functions on solver.scores
//    (the C solver argument to kissat_update_heap etc. is LOG-only).

use crate::heap;
use crate::internal::{Datarank, Solver};
use crate::print;
use crate::profile::{self, Prof};

/// bump.h: `#define MAX_SCORE 1e150`.
pub const MAX_SCORE: f64 = 1e150;

const RADIX_SORT_BUMP_LIMIT: usize = 32;

// C static `sort_bump`: RANK(A) = A.rank, SMALLER by rank.
fn sort_bump(solver: &mut Solver) {
    let size = solver.analyzed.len();
    if size < RADIX_SORT_BUMP_LIMIT {
        // SORT_STACK (datarank, solver->ranks, SMALLER)
        profile::start_checked(solver, Prof::sort);
        crate::sort::sort_stack(&mut solver.sorter, &mut solver.ranks, |a: &Datarank, b: &Datarank| {
            a.rank < b.rank
        });
        profile::stop_checked(solver, Prof::sort);
    } else {
        // RADIX_STACK (datarank, unsigned, solver->ranks, RANK)
        profile::start_checked(solver, Prof::radix);
        crate::sort::radix_stack::<Datarank, u32, _>(&mut solver.ranks, |d: &Datarank| d.rank);
        profile::stop_checked(solver, Prof::radix);
    }
}

/// Port of `kissat_rescale_scores`.
pub fn rescale_scores(solver: &mut Solver) {
    // INC (rescaled): METRIC — no-op.
    let max_score = heap::max_score_on_heap(&solver.scores);
    print::phase(
        solver,
        "rescale",
        u64::MAX, // GET (rescaled): METRIC — prints no count
        format_args!("maximum score {} increment {}", max_score, solver.scinc),
    );
    let rescale = if max_score > solver.scinc {
        max_score
    } else {
        solver.scinc
    }; // MAX (max_score, solver->scinc)
    debug_assert!(rescale > 0.0);
    let factor = 1.0 / rescale;
    heap::rescale_heap(&mut solver.scores, factor);
    solver.scinc *= factor;
    print::phase(
        solver,
        "rescale",
        u64::MAX,
        format_args!("rescaled by factor {}", factor),
    );
}

/// Port of `kissat_bump_score_increment`.
pub fn bump_score_increment(solver: &mut Solver) {
    let old_scinc = solver.scinc;
    let decay = solver.options.decay as f64 * 1e-3;
    debug_assert!((0.0..=0.5).contains(&decay));
    let factor = 1.0 / (1.0 - decay);
    let new_scinc = old_scinc * factor;
    solver.scinc = new_scinc;
    if new_scinc > MAX_SCORE {
        rescale_scores(solver);
    }
}

// C static inline `bump_analyzed_variable_score`.
#[inline]
fn bump_analyzed_variable_score(solver: &mut Solver, idx: u32) {
    let old_score = heap::get_heap_score(&solver.scores, idx);
    let inc = solver.scinc;
    let new_score = old_score + inc;
    heap::update_heap(&mut solver.scores, idx, new_score);
    if new_score > MAX_SCORE {
        rescale_scores(solver);
    }
}

/// Port of `kissat_bump_variable`.
pub fn bump_variable(solver: &mut Solver, idx: u32) {
    bump_analyzed_variable_score(solver, idx);
}

// C static `bump_analyzed_variable_scores`.
fn bump_analyzed_variable_scores(solver: &mut Solver) {
    for i in 0..solver.analyzed.len() {
        let idx = solver.analyzed[i];
        if solver.flags[idx as usize].active() {
            bump_analyzed_variable_score(solver, idx);
        }
    }
    bump_score_increment(solver);
}

// C static `move_analyzed_variables_to_front_of_queue`.
fn move_analyzed_variables_to_front_of_queue(solver: &mut Solver) {
    debug_assert!(solver.ranks.is_empty());
    for i in 0..solver.analyzed.len() {
        let idx = solver.analyzed[i];
        let rank = Datarank {
            data: idx,
            rank: solver.links[idx as usize].stamp,
        };
        solver.ranks.push(rank);
    }

    sort_bump(solver);

    for i in 0..solver.ranks.len() {
        let idx = solver.ranks[i].data;
        if solver.flags[idx as usize].active() {
            crate::inlinequeue::move_to_front(solver, idx);
        }
    }

    solver.ranks.clear();
}

/// Port of `kissat_bump_analyzed`.
pub fn bump_analyzed(solver: &mut Solver) {
    profile::start_checked(solver, Prof::bump);
    let _bumped = solver.analyzed.len() as u64;
    if !solver.stable {
        move_analyzed_variables_to_front_of_queue(solver);
    } else {
        bump_analyzed_variable_scores(solver);
    }
    // ADD (literals_bumped, bumped): METRIC — no-op.
    profile::stop_checked(solver, Prof::bump);
}

/// Port of `kissat_update_scores`.
pub fn update_scores(solver: &mut Solver) {
    debug_assert!(solver.stable);
    for idx in 0..solver.vars() {
        // ACTIVE (idx) && !kissat_heap_contains (scores, idx)
        if solver.flags[idx as usize].active() && !heap::heap_contains(&solver.scores, idx) {
            heap::push_heap(&mut solver.scores, idx);
        }
    }
}
