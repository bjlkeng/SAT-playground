// Port of src/propdense.c (kissat 4.0.4).
//
// Root-level propagation over full occurrence lists (dense mode).
//
// PORT NOTE: C `unsigned ticks` accumulates truncated
// kissat_cache_lines + per-clause increments and is ADDed at the end;
// kept as u32 for the exact truncation semantics.
// PORT NOTE: ADD (ticks, ...) targets the STATISTIC-tier `ticks` counter
// (kept as a real, never-printed field); ADD (dense_ticks, ...) and
// ADD (dense_propagations, ...) are METRIC — compiled out.

use crate::internal::{Solver, INVALID};
use crate::profile::Prof;
use crate::watch::{watch_is_binary, watch_lit, watch_ref};

// static inline bool non_watching_propagate_literal (kissat *, unsigned lit)
fn non_watching_propagate_literal(solver: &mut Solver, lit: u32) -> bool {
    debug_assert!(!solver.watching);
    debug_assert!(solver.values[lit as usize] > 0);
    let not_lit = crate::literal::not(lit);

    let v = solver.watches[not_lit as usize];
    let size_watches = v.size();
    let mut ticks: u32 =
        1u32.wrapping_add(crate::utilities::cache_lines(size_watches as u64, 4) as u32);

    let begin = v.begin;
    let end = v.end;
    let mut p = begin;
    while p != end {
        let watch = solver.vectors.stack[p];
        p += 1;
        if watch_is_binary(watch) {
            let other = watch_lit(watch);
            let other_value = solver.values[other as usize];
            if other_value > 0 {
                continue;
            }
            if other_value < 0 {
                return false; // conflicting binary
            }
            let other_idx = crate::literal::idx(other);
            if solver.flags[other_idx as usize].eliminated {
                continue;
            }
            debug_assert!(solver.level == 0);
            let probing = solver.probing;
            crate::assign::fast_binary_assign(solver, probing, 0, other, not_lit);
        } else {
            let ref_ = watch_ref(watch);
            debug_assert!(solver.arena.clause(ref_).size() > 2);
            debug_assert!(!solver.arena.clause(ref_).redundant());
            ticks = ticks.wrapping_add(1);
            if solver.arena.clause(ref_).garbage() {
                continue;
            }
            let mut non_false: u32 = 0;
            let mut unit = INVALID;
            let mut satisfied = false;
            for &other in solver.arena.clause(ref_).lits() {
                if other == not_lit {
                    continue;
                }
                let other_value = solver.values[other as usize];
                if other_value < 0 {
                    continue;
                }
                if other_value > 0 {
                    satisfied = true;
                    debug_assert!(solver.level == 0);
                    crate::clause::mark_clause_as_garbage(solver, ref_);
                    break;
                }
                // if (!non_false++) unit = other; else if (non_false > 1) break;
                non_false += 1;
                if non_false == 1 {
                    unit = other;
                } else if non_false > 1 {
                    break;
                }
            }
            if satisfied {
                continue;
            }
            if non_false == 0 {
                return false; // conflicting reference
            }
            if non_false == 1 {
                crate::assign::fast_assign_reference(solver, unit, ref_);
            }
        }
    }

    solver.statistics.ticks += ticks as u64; // ADD (ticks, ticks): STATISTIC kept
    // ADD (dense_ticks, ticks): METRIC, compiled out.

    true
}

/// Port of `kissat_dense_propagate`.
pub fn dense_propagate(solver: &mut Solver) -> bool {
    debug_assert!(solver.level == 0);
    debug_assert!(!solver.watching);
    debug_assert!(!solver.inconsistent);
    crate::profile::start_checked(solver, Prof::propagate); // START (propagate)
    let mut propagate = solver.propagate;
    let mut res = true;
    while res && propagate != solver.trail.len() {
        let lit = solver.trail[propagate];
        propagate += 1;
        res = non_watching_propagate_literal(solver, lit);
    }
    let propagated = (propagate - solver.propagate) as u64;
    solver.propagate = propagate;
    // ADD (dense_propagations, propagated): METRIC, compiled out.
    solver.statistics.propagations += propagated; // ADD (propagations, ...)
    if !res {
        debug_assert!(!solver.inconsistent);
        // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
        if solver.proof.is_some() {
            crate::proof::add_empty_to_proof(solver);
        }
        solver.inconsistent = true;
    }
    crate::profile::stop_checked(solver, Prof::propagate); // STOP (propagate)
    res
}
