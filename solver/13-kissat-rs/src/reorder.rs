// Port of src/reorder.c (kissat 4.0.4).
//
// PORT NOTES:
//  - `weights` is a calloc'd LITS-sized double array that is deliberately
//    reused/aliased at the end of compute_weights: per-variable weights are
//    written into weights[idx] over the literal-indexed data (same array in
//    C) — quirk ported as-is; the double accumulation order is identical.
//  - SORT_STACK expands to SORT which does START (sort)/STOP (sort) around
//    the quicksort (only when the stack has > 1 element); the profile calls
//    are hoisted around crate::sort::sort_stack per the sort.rs convention.
//  - INC (reordered) is a COUNTER; reordered_focused/_stable are
//    STATISTIC-tier (real never-printed fields).
//  - The C phase message "reorder limit %" PRIu64 " hit a after %" PRIu64
//    " conflicts in %s mode " has a typo ("hit a after") and a trailing
//    space — reproduced verbatim.

use crate::internal::Solver;
use crate::profile::Prof;
use crate::queue::Links;
use crate::update_conflict_limit;

/// Port of `kissat_reordering`.
pub fn reordering(solver: &mut Solver) -> bool {
    if solver.options.reorder == 0 {
        return false;
    }
    if !solver.stable && solver.options.reorder < 2 {
        return false;
    }
    if solver.level != 0 {
        return false;
    }
    solver.statistics.conflicts >= solver.limits.reorder.conflicts
}

// static double *compute_weights (kissat *solver)
fn compute_weights(solver: &mut Solver) -> Vec<f64> {
    let lits = solver.lits() as usize;
    let mut weights = vec![0f64; lits]; // kissat_calloc (LITS)
    let max_size = solver.options.reordermaxsize as u32;
    debug_assert!(max_size >= 2);
    let mut table = vec![0f64; max_size as usize + 1];
    {
        let mut weight: f64 = 1.0;
        for size in 2..=max_size {
            table[size as usize] = weight;
            weight /= 2.0;
        }
    }
    {
        debug_assert!(solver.level == 0);
        // const clause *last = kissat_last_irredundant_clause (solver);
        let last = solver.last_irredundant;
        let mut ref_: crate::reference::Reference = 0;
        while (ref_ as u64) < solver.arena.size_wards() {
            let next = solver.arena.next_clause_ref(ref_);
            // if (last && c > last) break;
            if last != crate::reference::INVALID_REF && ref_ > last {
                break;
            }
            let c = solver.arena.clause(ref_);
            if c.redundant() {
                ref_ = next;
                continue;
            }
            if c.garbage() {
                ref_ = next;
                continue;
            }
            let csize = c.size();
            let mut size: u32 = 0;
            let mut satisfied = false; // goto CONTINUE_WITH_NEXT_CLAUSE
            for i in 0..csize {
                let lit = solver.arena.clause(ref_).lit(i);
                let value = solver.values[lit as usize];
                if value > 0 {
                    satisfied = true;
                    break;
                }
                if value == 0 && size < max_size {
                    size += 1;
                    if size == max_size {
                        break;
                    }
                }
            }
            if !satisfied {
                let weight = table[size as usize];
                for i in 0..csize {
                    let lit = solver.arena.clause(ref_).lit(i);
                    weights[lit as usize] += weight;
                }
            }
            ref_ = next;
        }
    }
    debug_assert!(solver.watching);
    {
        let weight = table[2];
        drop(table); // kissat_dealloc (table)
        let lits = solver.lits();
        for lit in 0..lits {
            let idx = crate::literal::idx(lit);
            if !solver.flags[idx as usize].active {
                continue;
            }
            // for (all_binary_blocking_watches (watch, *watches))
            let v = solver.watches[lit as usize];
            let mut p = v.begin;
            while p != v.end {
                let watch = solver.vectors.stack[p];
                if !crate::watch::watch_is_binary(watch) {
                    p += 2; // skip blocking + reference words
                    continue;
                }
                p += 1;
                let other = crate::watch::watch_lit(watch);
                if other < lit {
                    continue;
                }
                let other_idx = crate::literal::idx(other);
                if !solver.flags[other_idx as usize].active {
                    continue;
                }
                weights[lit as usize] += weight;
                weights[other as usize] += weight;
            }
        }
    }
    for idx in 0..solver.vars {
        if !solver.flags[idx as usize].active {
            continue;
        }
        let lit = crate::literal::lit(idx);
        let not_lit = crate::literal::not(lit);
        let pos = weights[lit as usize];
        let neg = weights[not_lit as usize];
        let max_pos_neg = pos.max(neg); // MAX
        let min_pos_neg = pos.min(neg); // MIN
        let scaled_min_pos_neg = 2.0 * min_pos_neg;
        let weight = max_pos_neg + scaled_min_pos_neg;
        weights[idx as usize] = weight;
    }
    weights
}

// static bool less_focused_order (unsigned a, unsigned b, links *, double *)
fn less_focused_order(a: u32, b: u32, links: &[Links], weights: &[f64]) -> bool {
    let u = weights[a as usize];
    let v = weights[b as usize];
    if u < v {
        return true;
    }
    if u > v {
        return false;
    }
    let s = links[a as usize].stamp;
    let t = links[b as usize].stamp;
    s < t
}

// static bool less_stable_order (unsigned a, unsigned b, heap *, double *)
fn less_stable_order(a: u32, b: u32, scores: &crate::heap::Heap, weights: &[f64]) -> bool {
    let u = weights[a as usize];
    let v = weights[b as usize];
    if u < v {
        return true;
    }
    if u > v {
        return false;
    }
    let s = crate::heap::get_heap_score(scores, a);
    let t = crate::heap::get_heap_score(scores, b);
    if s < t {
        return true;
    }
    if s > t {
        return false;
    }
    b < a
}

// static void sort_active_variables_by_weight (kissat *, unsigneds *, double *)
fn sort_active_variables_by_weight(solver: &mut Solver, weights: &[f64]) -> Vec<u32> {
    let mut sorted: Vec<u32> = Vec::new(); // INIT_STACK (*sorted)
    for idx in 0..solver.vars {
        if solver.flags[idx as usize].active {
            sorted.push(idx);
        }
    }
    // SORT_STACK (unsigned, *sorted, LESS_...): SORT does START (sort)/STOP
    // (sort) when the stack has more than one element.
    if sorted.len() > 1 {
        crate::profile::start_checked(solver, Prof::sort);
        let mut sorter = std::mem::take(&mut solver.sorter);
        if solver.stable {
            let scores = &solver.scores; // SCORES
            crate::sort::sort_stack(&mut sorter, &mut sorted, |&a: &u32, &b: &u32| {
                less_stable_order(a, b, scores, weights)
            });
        } else {
            let links = &solver.links;
            crate::sort::sort_stack(&mut sorter, &mut sorted, |&a: &u32, &b: &u32| {
                less_focused_order(a, b, links, weights)
            });
        }
        solver.sorter = sorter;
        crate::profile::stop_checked(solver, Prof::sort);
    }
    sorted
}

// static void reorder_focused (kissat *solver)
fn reorder_focused(solver: &mut Solver) {
    solver.statistics.reordered_focused += 1; // INC (reordered_focused) — STATISTIC
    debug_assert!(!solver.stable);
    let weights = compute_weights(solver);
    let sorted = sort_active_variables_by_weight(solver, &weights);
    drop(weights); // kissat_dealloc (weights)
    for idx in sorted {
        debug_assert!(solver.flags[idx as usize].active);
        crate::inlinequeue::move_to_front(solver, idx);
    }
    // RELEASE_STACK (sorted) — Drop.
}

// static void reorder_stable (kissat *solver)
fn reorder_stable(solver: &mut Solver) {
    solver.statistics.reordered_stable += 1; // INC (reordered_stable) — STATISTIC
    debug_assert!(solver.stable);
    let weights = compute_weights(solver);
    crate::bump::rescale_scores(solver);
    let mut sorted = sort_active_variables_by_weight(solver, &weights);
    // heap *scores = SCORES;
    while let Some(idx) = sorted.pop() {
        debug_assert!(solver.flags[idx as usize].active);
        let old_score = crate::heap::get_heap_score(&solver.scores, idx);
        let weight = weights[idx as usize];
        let new_score = old_score + weight;
        crate::heap::update_heap(&mut solver.scores, idx, new_score);
    }
    // kissat_dealloc (weights) / RELEASE_STACK (sorted) — Drop.
}

/// Port of `kissat_reorder`.
pub fn reorder(solver: &mut Solver) {
    crate::profile::start_checked(solver, Prof::reorder); // START (reorder)
    solver.statistics.reordered += 1; // INC (reordered)
    debug_assert!(solver.level == 0);
    let reordered = solver.statistics.reordered;
    let limit = solver.limits.reorder.conflicts;
    let conflicts = solver.statistics.conflicts;
    crate::print::phase(
        solver,
        "reorder",
        reordered,
        format!(
            "reorder limit {} hit a after {} conflicts in {} mode ",
            limit,
            conflicts,
            if solver.stable { "stable" } else { "focused" }
        ),
    );
    if solver.stable {
        reorder_stable(solver);
    } else {
        reorder_focused(solver);
    }
    crate::print::phase(
        solver,
        "reorder",
        reordered,
        format!(
            "reordered decisions in {} search mode",
            if solver.stable { "stable" } else { "focused" }
        ),
    );
    // UPDATE_CONFLICT_LIMIT (reorder, reordered, LINEAR, false);
    update_conflict_limit!(
        solver,
        reorder,
        reorderint,
        reordered,
        |n: u64| n as f64, // LINEAR
        false
    );
    crate::report::report(solver, false, 'o'); // REPORT (0, 'o')
    crate::profile::stop_checked(solver, Prof::reorder); // STOP (reorder)
}
