// Port of src/backbone.c (kissat 4.0.4).
//
// Binary-clause backbone computation: probe literals over the binary
// implication graph with a dedicated cheap propagator, learn failed literals
// as units.
//
// PORT NOTES:
//  - The C inline helpers take `unsigned_array *trail`, `value *values`,
//    `assigned *assigned` pointers pulled out of the solver once; the port
//    accesses the solver fields directly (same effect order).
//  - `backbone_assign` deliberately does NOT set `a->trail` (only reason and
//    level) — quirk ported as-is.
//  - backbone_propagate_literal returns the conflict *before* accounting the
//    touched-watches ticks (C early `return kissat_binary_conflict (...)`);
//    ported exactly.
//  - Statistics tiers: backbone_computations / backbone_ticks are COUNTERs,
//    backbone_units is STATISTIC (real, never-printed field per statistics.rs
//    policy); backbone_implied / backbone_probes / backbone_propagations /
//    backbone_rounds are METRIC — compiled out, their INC/ADD sites dropped.
//  - The `#if defined(METRICS)` implied_before/total_implied phase message at
//    the end of compute_backbone is compiled out; the plain (non-METRICS)
//    build prints no success phase line there.
//  - check_large_clauses_watched_after_binary_clauses and the
//    solver->backbone_computing flag are !NDEBUG/METRICS-only — omitted.

use crate::internal::{Solver, DECISION_REASON, UNIT_REASON};
use crate::literal::{idx, lit as lit_of_idx, negated, not, INVALID_LIT};
use crate::profile::Prof;
use crate::propsearch::{binary_conflict, Conflict};
use crate::reference::INVALID_REF;
use crate::terminated;
use crate::utilities::percent;
use crate::watch::{watch_is_binary, watch_lit};

// static schedule_backbone_candidates
fn schedule_backbone_candidates(solver: &mut Solver, candidates: &mut Vec<u32>) {
    let mut not_rescheduled: u32 = 0;
    for idx in 0..solver.vars {
        let f = solver.flags[idx as usize];
        if !f.active {
            continue;
        }
        let lit = lit_of_idx(idx);
        if f.backbone0 {
            candidates.push(lit);
        } else {
            not_rescheduled += 1;
        }
        if f.backbone1 {
            let not_lit = not(lit);
            candidates.push(not_lit);
        } else {
            not_rescheduled += 1;
        }
    }
    // #ifndef QUIET
    let rescheduled = candidates.len();
    let active_literals = 2u64 * solver.active as u64;
    crate::print::very_verbose(
        solver,
        format_args!(
            "rescheduled {} backbone candidate literals {:.0}%",
            rescheduled,
            percent(rescheduled as f64, active_literals as f64)
        ),
    );
    if not_rescheduled != 0 {
        for idx in 0..solver.vars {
            let f = solver.flags[idx as usize];
            if !f.active {
                continue;
            }
            let lit = lit_of_idx(idx);
            if !f.backbone0 {
                candidates.push(lit);
            }
            if !f.backbone1 {
                let not_lit = not(lit);
                candidates.push(not_lit);
            }
        }
    }
    // #ifndef QUIET
    let total = candidates.len();
    crate::print::very_verbose(
        solver,
        format_args!(
            "scheduled {} backbone candidate literals {:.0}% in total",
            total,
            percent(total as f64, active_literals as f64)
        ),
    );
}

// static keep_backbone_candidates
fn keep_backbone_candidates(solver: &mut Solver, candidates: &[u32]) {
    let mut prioritized: usize = 0;
    let mut remain: usize = 0;
    for &lit in candidates.iter() {
        let i = idx(lit);
        let f = solver.flags[i as usize];
        if !f.active {
            continue;
        }
        remain += 1;
        if negated(lit) != 0 {
            prioritized += f.backbone1 as usize;
        } else {
            prioritized += f.backbone0 as usize;
        }
    }
    debug_assert!(prioritized <= remain);
    if remain == 0 {
        crate::print::very_verbose(solver, "no backbone candidates remain");
        return;
    }
    // #ifndef QUIET
    let active_literals = 2u64 * solver.active as u64;
    if prioritized == remain {
        crate::print::very_verbose(
            solver,
            format_args!(
                "keeping all remaining {} backbone candidates {:.0}% prioritized (all were)",
                remain,
                percent(remain as f64, active_literals as f64)
            ),
        );
    } else if prioritized == 0 {
        for &lit in candidates.iter() {
            let i = idx(lit);
            if !solver.flags[i as usize].active {
                continue;
            }
            if negated(lit) != 0 {
                debug_assert!(!solver.flags[i as usize].backbone1);
                solver.flags[i as usize].backbone1 = true;
            } else {
                debug_assert!(!solver.flags[i as usize].backbone0);
                solver.flags[i as usize].backbone0 = true;
            }
        }
        crate::print::very_verbose(
            solver,
            format_args!(
                "keeping all remaining {} backbone candidates {:.0}% prioritized (none was)",
                remain,
                percent(remain as f64, active_literals as f64)
            ),
        );
    } else {
        crate::print::very_verbose(
            solver,
            format_args!(
                "keeping {} backbone candidates {:.0}% prioritized ({:.0}% of remaining {})",
                prioritized,
                percent(prioritized as f64, active_literals as f64),
                percent(prioritized as f64, remain as f64),
                remain
            ),
        );
    }
}

// static inline backbone_assign
#[inline]
fn backbone_assign(solver: &mut Solver, lit: u32, reason: u32) {
    let not_lit = not(lit);
    debug_assert!(solver.values[lit as usize] == 0);
    debug_assert!(solver.values[not_lit as usize] == 0);
    solver.values[lit as usize] = 1;
    solver.values[not_lit as usize] = -1;
    solver.trail.push(lit); // PUSH_ARRAY (*trail, lit)
    let i = idx(lit);
    let a = &mut solver.assigned[i as usize];
    a.reason = reason;
    a.level = solver.level;
}

// static inline backbone_propagate_literal
#[inline]
fn backbone_propagate_literal(
    solver: &mut Solver,
    stop_early: bool,
    lit: u32,
) -> Option<Conflict> {
    debug_assert!(solver.values[lit as usize] > 0);

    let not_lit = not(lit);
    debug_assert!(solver.values[not_lit as usize] < 0);

    let watches = solver.watches[not_lit as usize];
    let begin_watches = watches.begin;
    let end_watches = watches.end;
    let mut p = begin_watches;

    while p != end_watches {
        let watch = solver.vectors.stack[p];
        p += 1;
        if watch_is_binary(watch) {
            let other = watch_lit(watch);
            let value = solver.values[other as usize];
            if value > 0 {
                continue;
            }
            if value < 0 {
                // PORT NOTE: C returns here WITHOUT accounting the ticks of
                // the touched watches — ported exactly.
                return Some(binary_conflict(solver, not_lit, other));
            }
            debug_assert!(value == 0);
            backbone_assign(solver, other, lit);
        } else {
            if stop_early {
                break;
            }
            p += 1;
        }
    }

    let touched = (p - begin_watches) as u64;
    solver.ticks += 1 + crate::utilities::cache_lines(touched, 4);

    None
}

// static inline backbone_propagate
#[inline]
fn backbone_propagate(solver: &mut Solver) -> Option<Conflict> {
    let stop_early = solver.large_clauses_watched_after_binary_clauses;

    let mut conflict: Option<Conflict> = None;
    solver.ticks = 0;

    let mut propagate = solver.propagate;
    while conflict.is_none() && propagate != solver.trail.len() {
        let lit = solver.trail[propagate];
        propagate += 1;
        conflict = backbone_propagate_literal(solver, stop_early, lit);
    }

    debug_assert!(solver.propagate <= propagate);
    let propagated = (propagate - solver.propagate) as u64;
    solver.propagate = propagate;

    // ADD (backbone_propagations, ...) / ADD (probing_propagations, ...):
    // METRIC, compiled out.
    solver.statistics.propagations += propagated; // ADD (propagations, ...)

    let ticks = solver.ticks;

    solver.statistics.backbone_ticks += ticks; // ADD (backbone_ticks, ...)
    solver.statistics.probing_ticks += ticks; // ADD (probing_ticks, ...)
    solver.statistics.ticks += ticks; // ADD (ticks, ...)

    conflict
}

// static inline backbone_backtrack
#[inline]
fn backbone_backtrack(solver: &mut Solver, saved: usize, decision_level: u32) {
    debug_assert!(decision_level <= solver.level);
    let mut end_trail = solver.trail.len();
    debug_assert!(saved != end_trail);
    while saved != end_trail {
        end_trail -= 1;
        let lit = solver.trail[end_trail];
        let not_lit = not(lit);
        debug_assert!(solver.values[lit as usize] > 0);
        debug_assert!(solver.values[not_lit as usize] < 0);
        solver.values[lit as usize] = 0;
        solver.values[not_lit as usize] = 0;
    }
    solver.trail.truncate(saved); // SET_END_OF_ARRAY (solver->trail, saved)
    solver.level = decision_level;
    solver.propagate = saved;
}

// static backbone_analyze
fn backbone_analyze(solver: &mut Solver, conflict: Conflict) -> u32 {
    debug_assert!(crate::analyze::conflict_size(solver, conflict) == 2);

    let c0 = crate::analyze::conflict_lit(solver, conflict, 0);
    let c1 = crate::analyze::conflict_lit(solver, conflict, 1);
    crate::inline::push_analyzed(solver, idx(c0));
    crate::inline::push_analyzed(solver, idx(c1));

    let mut t = solver.trail.len();

    loop {
        debug_assert!(t > 0);
        t -= 1;
        let lit = solver.trail[t];

        let lit_idx = idx(lit);
        if !solver.assigned[lit_idx as usize].analyzed() {
            continue;
        }

        let reason = solver.assigned[lit_idx as usize].reason;
        debug_assert!(reason != UNIT_REASON);
        debug_assert!(reason != DECISION_REASON);
        let reason_idx = idx(reason);
        if !solver.assigned[reason_idx as usize].analyzed() {
            crate::inline::push_analyzed(solver, reason_idx);
        } else {
            crate::analyze::reset_only_analyzed_literals(solver);
            return reason;
        }
    }
}

// static compute_backbone
fn compute_backbone(solver: &mut Solver) -> u32 {
    let mut failed: usize = 0;
    let mut units: Vec<u32> = Vec::new();
    let mut candidates: Vec<u32> = Vec::new();
    schedule_backbone_candidates(solver, &mut candidates);
    // #ifndef QUIET
    let scheduled = candidates.len();

    debug_assert!(solver.propagate == solver.trail.len()); // kissat_propagated

    let mut inconsistent: u32 = INVALID_LIT;

    let ticks_limit = crate::set_effort_limit!(solver, backbone, backboneeffort, backbone_ticks);
    let mut round_limit: u64 = solver.options.backbonerounds as u64;
    debug_assert!(solver.statistics.backbone_computations != 0);
    round_limit *= solver.statistics.backbone_computations;
    let max_rounds = solver.options.backbonemaxrounds as u64;
    if round_limit > max_rounds {
        round_limit = max_rounds;
    }

    let mut round: u64 = 0;

    loop {
        if round >= round_limit {
            crate::print::very_verbose(solver, format_args!("backbone round limit {} hit", round));
            break;
        }
        let ticks = solver.statistics.backbone_ticks;
        if ticks > ticks_limit {
            crate::print::very_verbose(
                solver,
                format_args!("backbone ticks limit {} hit after {} ticks", ticks_limit, ticks),
            );
            break;
        }
        let previous = failed;
        debug_assert!(!solver.inconsistent);
        if terminated!(solver, backbone_terminated_1) {
            break;
        }
        round += 1;
        // INC (backbone_rounds): METRIC, compiled out.
        debug_assert!(solver.level == 0);
        let active_before = solver.active;
        {
            let mut q: usize = 0;
            let mut p: usize = 0;
            let end_candidates = candidates.len();
            while p != end_candidates {
                debug_assert!(!solver.inconsistent);
                let probe = candidates[p];
                candidates[q] = probe;
                q += 1;
                p += 1;
                let value = solver.values[probe as usize];
                if value > 0 {
                    q -= 1;
                    let i = idx(probe);
                    if negated(probe) != 0 {
                        solver.flags[i as usize].backbone1 = false;
                    } else {
                        solver.flags[i as usize].backbone0 = false;
                    }
                    continue;
                }
                if value < 0 {
                    let i = idx(probe);
                    if solver.assigned[i as usize].level != 0 {
                        // skipping falsified backbone probe
                    } else {
                        // removing root-level falsified backbone probe
                        q -= 1;
                    }
                    continue;
                }
                if solver.statistics.backbone_ticks > ticks_limit {
                    break;
                }
                if terminated!(solver, backbone_terminated_2) {
                    break;
                }
                let level = solver.level;
                let saved = solver.trail.len();
                debug_assert!(level != u32::MAX);
                solver.level = level + 1;
                // INC (backbone_probes): METRIC, compiled out.
                backbone_assign(solver, probe, DECISION_REASON);
                let conflict = backbone_propagate(solver);
                if conflict.is_none() {
                    continue;
                }

                failed += 1;
                solver.statistics.backbone_units += 1; // INC (backbone_units): STATISTIC
                q -= 1;

                let uip = backbone_analyze(solver, conflict.unwrap());
                let not_uip = not(uip);
                backbone_backtrack(solver, saved, level);

                units.push(not_uip);
                backbone_assign(solver, not_uip, UNIT_REASON);
                debug_assert!(failed == units.len());

                let conflict = backbone_propagate(solver);
                if conflict.is_some() {
                    inconsistent = not_uip;
                    break;
                }
            }
            // #ifndef QUIET
            let remain = end_candidates - p;
            if remain != 0 {
                crate::print::extremely_verbose(
                    solver,
                    format_args!(
                        "backbone round {} aborted with {} candidates {:.0}% remaining",
                        round,
                        remain,
                        percent(remain as f64, scheduled as f64)
                    ),
                );
            } else {
                crate::print::extremely_verbose(
                    solver,
                    format_args!(
                        "backbone round {} completed with all {} scheduled candidates tried",
                        round, scheduled
                    ),
                );
            }
            while p != end_candidates {
                candidates[q] = candidates[p];
                q += 1;
                p += 1;
            }

            candidates.truncate(q); // SET_END_OF_STACK (candidates, q)
        }
        if inconsistent == INVALID_LIT {
            // flushing satisfied probe candidates
            let mut q: usize = 0;
            let mut p: usize = 0;
            let end_candidates = candidates.len();
            while p != end_candidates {
                let probe = candidates[p];
                candidates[q] = probe;
                q += 1;
                p += 1;
                let value = solver.values[probe as usize];
                if value > 0 {
                    q -= 1;
                    let i = idx(probe);
                    if negated(probe) != 0 {
                        solver.flags[i as usize].backbone1 = false;
                    } else {
                        solver.flags[i as usize].backbone0 = false;
                    }
                    continue;
                }
                if value < 0 {
                    // keeping falsified probe
                    continue;
                }
                debug_assert!(value == 0);
                // keeping unassigned probe
            }
            candidates.truncate(q); // SET_END_OF_STACK (candidates, q)
        }
        if !solver.trail.is_empty() {
            backbone_backtrack(solver, 0, 0);
        }
        if inconsistent == INVALID_LIT && previous < failed {
            let mut broke = false;
            for i in previous..failed {
                let unit = units[i]; // PEEK_STACK (units, i)
                crate::assign::learned_unit(solver, unit);
            }
            if crate::proprobe::probing_propagate(solver, INVALID_REF, true).is_some() {
                broke = true;
            }
            if broke {
                break;
            }
        }
        debug_assert!(solver.active <= active_before);
        let implied = active_before - solver.active;
        // ADD (backbone_implied, implied): METRIC, compiled out.
        // #ifndef QUIET
        let left = candidates.len();
        crate::print::very_verbose(
            solver,
            format_args!(
                "backbone round {} produced {} failed literals {} implied ({} candidates left {:.0}%)",
                round,
                failed - previous,
                implied,
                left,
                percent(left as f64, scheduled as f64)
            ),
        );
        if inconsistent != INVALID_LIT {
            break;
        }
        if candidates.is_empty() {
            break;
        }
    }

    if inconsistent != INVALID_LIT && !solver.inconsistent {
        crate::assign::learned_unit(solver, inconsistent);
        let _ = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
        debug_assert!(solver.inconsistent);
    }
    drop(units); // RELEASE_STACK (units)
    if solver.inconsistent {
        crate::print::phase(
            solver,
            "backbone",
            solver.statistics.backbone_computations, // GET (backbone_computations)
            "inconsistent binary clauses",
        );
    } else {
        keep_backbone_candidates(solver, &candidates);
        // #if defined(METRICS) && !defined(QUIET) success phase message:
        // compiled out in the reference build.
    }
    drop(candidates); // RELEASE_STACK (candidates)
    failed as u32
}

/// Port of `kissat_binary_clauses_backbone`.
pub fn binary_clauses_backbone(solver: &mut Solver) {
    if solver.inconsistent {
        return;
    }
    if solver.options.backbone == 0 {
        return;
    }
    if terminated!(solver, backbone_terminated_3) {
        return;
    }
    debug_assert!(solver.watching);
    debug_assert!(solver.probing);
    debug_assert!(solver.level == 0);
    crate::profile::start_checked(solver, Prof::backbone); // START (backbone)
    solver.statistics.backbone_computations += 1; // INC (backbone_computations)
    let failed = compute_backbone(solver);
    crate::report::report(solver, failed == 0, 'b'); // REPORT (!failed, 'b')
    crate::profile::stop_checked(solver, Prof::backbone); // STOP (backbone)
}
