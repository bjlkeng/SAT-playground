// Port of src/eliminate.c (kissat 4.0.4).
//
// Bounded variable elimination (BVE) driver.
//
// PORT NOTE: the C file-local `eliminate` (static) clashes with the public
// kissat_eliminate after prefix-dropping; it is named `eliminate_inner` here
// (same pattern as flags.rs::activate_literal_inner).
// PORT NOTE (quirk ported): in eliminate_variables the C code declares an
// OUTER `unsigned last_round_eliminated = 0;` which is SHADOWED by a fresh
// `unsigned last_round_eliminated = 0;` inside the round loop, so the final
// `complete = !remain && !last_round_eliminated` always reads 0 from the
// outer one — i.e. completeness is decided by `!remain` alone.  Ported
// exactly (`last_round_eliminated_outer` stays 0).
// PORT NOTE: kissat_check_statistics is `#ifndef NDEBUG` — compiled out.
// PORT NOTE: GET (eliminations) is a COUNTER (real); phase messages print
// the count.  INC (gates_eliminated) targets a STATISTIC-tier counter (kept
// as a real, never-printed field); the METRICS-only *solver->gate_eliminated
// increment is compiled out (gate_eliminated is a bool in this build).

use crate::internal::{Solver, INVALID};
use crate::profile::Prof;
use crate::reference::Reference;
use crate::terminated;
use crate::watch::{watch_is_binary, watch_lit, watch_ref};

/// Port of `kissat_eliminating`.
pub fn eliminating(solver: &mut Solver) -> bool {
    if !solver.enabled.eliminate {
        return false;
    }
    if solver.statistics.clauses_irredundant == 0 {
        return false;
    }
    let conflicts = solver.statistics.conflicts;
    if solver.last.conflicts.reduce == conflicts {
        return false;
    }
    if solver.limits.eliminate.conflicts > conflicts {
        return false;
    }
    if solver.limits.eliminate.variables.eliminate < solver.statistics.variables_eliminate {
        return true;
    }
    solver.limits.eliminate.variables.subsume < solver.statistics.variables_subsume
}

// static inline double variable_score (kissat *, unsigned idx)
fn variable_score(solver: &Solver, idx: u32) -> f64 {
    let lit = crate::literal::lit(idx);
    let not_lit = crate::literal::not(lit);
    let occlim = solver.options.eliminateocclim as u64; // size_t occlim
    let mut pos = solver.watches[lit as usize].size() as u64;
    let mut neg = solver.watches[not_lit as usize].size() as u64;
    if pos > occlim {
        pos = occlim;
    }
    if neg > occlim {
        neg = occlim;
    }
    let prod = (pos * neg) as f64;
    let sum = (pos + neg) as f64;
    let occlim2 = occlim as f64 * occlim as f64;
    debug_assert!(prod <= occlim2);
    let score = prod - sum;
    debug_assert!(score <= occlim2);
    let relevancy = if solver.stable {
        crate::heap::get_heap_score(&solver.scores, idx)
    } else {
        solver.links[idx as usize].stamp as f64
    };
    relevancy + score - occlim2
}

// static inline update_variable_score + kissat_update_variable_score
/// Port of `kissat_update_variable_score`.
pub fn update_variable_score(solver: &mut Solver, idx: u32) {
    debug_assert!(solver.schedule.size != 0);
    let new_score = variable_score(solver, idx);
    crate::heap::update_heap(&mut solver.schedule, idx, -new_score);
}

// static inline void update_after_adding_stack (kissat *, unsigneds *stack)
// — only ever called with &solver->clause; iterated by index.
fn update_after_adding_stack(solver: &mut Solver) {
    debug_assert!(!solver.probing);
    if solver.schedule.size == 0 {
        return;
    }
    for i in 0..solver.clause.len() {
        let lit = solver.clause[i];
        update_variable_score(solver, crate::literal::idx(lit));
    }
}

// static inline void update_after_removing_variable (kissat *, unsigned idx)
fn update_after_removing_variable(solver: &mut Solver, idx: u32) {
    if solver.schedule.size == 0 {
        return;
    }
    debug_assert!(!solver.probing);
    let f = solver.flags[idx as usize];
    if f.fixed {
        return;
    }
    debug_assert!(!f.eliminated);
    update_variable_score(solver, idx);
    if !crate::heap::heap_contains(&solver.schedule, idx) {
        crate::heap::push_heap(&mut solver.schedule, idx);
    }
}

// static inline void update_after_removing_clause (kissat *, clause *c,
//                                                  unsigned except)
fn update_after_removing_clause(solver: &mut Solver, ref_: Reference, except: u32) {
    if solver.schedule.size == 0 {
        return;
    }
    debug_assert!(solver.arena.clause(ref_).garbage());
    let size = solver.arena.clause(ref_).size();
    for i in 0..size {
        let lit = solver.arena.clause(ref_).lit(i);
        if lit != except {
            update_after_removing_variable(solver, crate::literal::idx(lit));
        }
    }
}

/// Port of `kissat_eliminate_binary`.
pub fn eliminate_binary(solver: &mut Solver, lit: u32, other: u32) {
    crate::watch::disconnect_binary(solver, other, lit);
    crate::clause::delete_binary(solver, lit, other);
    update_after_removing_variable(solver, crate::literal::idx(other));
}

/// Port of `kissat_eliminate_clause`.
pub fn eliminate_clause(solver: &mut Solver, ref_: Reference, lit: u32) {
    crate::clause::mark_clause_as_garbage(solver, ref_);
    update_after_removing_clause(solver, ref_, lit);
}

// static unsigned schedule_variables (kissat *)
fn schedule_variables(solver: &mut Solver) -> u32 {
    debug_assert!(solver.schedule.size == 0);

    crate::heap::resize_heap(&mut solver.schedule, solver.vars);

    let mut scheduled: u64 = 0; // size_t
    for idx in 0..solver.vars {
        let flags = solver.flags[idx as usize];
        if !flags.active {
            continue;
        }
        if !flags.eliminate {
            continue;
        }
        scheduled += 1;
        update_after_removing_variable(solver, idx);
    }
    debug_assert!(scheduled == crate::heap::size_heap(&solver.schedule) as u64);
    // #ifndef QUIET
    let active = solver.active;
    crate::print::phase(
        solver,
        "eliminate",
        solver.statistics.eliminations, // GET (eliminations)
        format_args!(
            "scheduled {} variables {:.0}%",
            scheduled,
            crate::format::percent(scheduled as f64, active as f64)
        ),
    );
    scheduled as u32
}

/// Port of `kissat_flush_units_while_connected`.
pub fn flush_units_while_connected(solver: &mut Solver) {
    let propagate = solver.propagate;
    let end_trail = solver.trail.len();
    debug_assert!(propagate <= end_trail);
    let units = end_trail - propagate;
    if units == 0 {
        return;
    }
    if !crate::propdense::dense_propagate(solver) {
        return;
    }
    // marking and flushing unit satisfied clauses
    let end_trail = solver.trail.len();
    let mut propagate = propagate;
    while propagate != end_trail {
        let unit = solver.trail[propagate];
        propagate += 1;
        let v = solver.watches[unit as usize];
        let begin = v.begin;
        let end = v.end;
        if begin == end {
            continue;
        }
        let mut q = begin;
        let mut p = begin;
        while p != end {
            // const watch watch = *q++ = *p++;
            let watch = solver.vectors.stack[p];
            solver.vectors.stack[q] = watch;
            q += 1;
            p += 1;
            if watch_is_binary(watch) {
                let other = watch_lit(watch);
                if solver.values[other as usize] == 0 {
                    update_after_removing_variable(solver, crate::literal::idx(other));
                }
            } else {
                let ref_ = watch_ref(watch);
                if !solver.arena.clause(ref_).garbage() {
                    eliminate_clause(solver, ref_, unit);
                }
                debug_assert!(solver.arena.clause(ref_).garbage());
                q -= 1;
            }
        }
        debug_assert!(q <= end);
        let flushed = end - q;
        if flushed == 0 {
            continue;
        }
        crate::vector::resize_vector(solver, unit, q - begin); // SET_END_OF_WATCHES
    }
}

// static void connect_resolvents (kissat *)
fn connect_resolvents(solver: &mut Solver) {
    debug_assert!(solver.clause.is_empty());
    let mut satisfied = false;
    for i in 0..solver.resolvents.len() {
        let other = solver.resolvents[i];
        if other == INVALID {
            if satisfied {
                satisfied = false;
            } else {
                let size = solver.clause.len();
                if size == 0 {
                    debug_assert!(!solver.inconsistent);
                    // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
                    if solver.proof.is_some() {
                        crate::proof::add_empty_to_proof(solver);
                    }
                    solver.inconsistent = true;
                    break;
                } else if size == 1 {
                    let unit = solver.clause[0]; // PEEK_STACK (solver->clause, 0)
                    crate::assign::learned_unit(solver, unit);
                } else {
                    debug_assert!(size > 1);
                    let _ = crate::clause::new_irredundant_clause(solver);
                    update_after_adding_stack(solver);
                }
            }
            solver.clause.clear();
        } else if !satisfied {
            let value = solver.values[other as usize];
            if value > 0 {
                satisfied = true;
            } else if value < 0 {
                // dropping now falsified literal
            } else {
                solver.clause.push(other);
            }
        }
    }
    solver.resolvents.clear();
}

// static void weaken_clauses (kissat *, unsigned lit)
fn weaken_clauses(solver: &mut Solver, lit: u32) {
    let not_lit = crate::literal::not(lit);

    debug_assert!(solver.values[lit as usize] == 0);

    let v = solver.watches[lit as usize];
    let (begin, end) = (v.begin, v.end);
    let mut p = begin;
    while p != end {
        let watch = solver.vectors.stack[p];
        p += 1;
        if watch_is_binary(watch) {
            let other = watch_lit(watch);
            let value = solver.values[other as usize];
            if value <= 0 {
                crate::weaken::weaken_binary(solver, lit, other);
            }
            eliminate_binary(solver, lit, other);
        } else {
            let ref_ = watch_ref(watch);
            if solver.arena.clause(ref_).garbage() {
                continue;
            }
            let mut satisfied = false;
            for &other in solver.arena.clause(ref_).lits() {
                let value = solver.values[other as usize];
                if value <= 0 {
                    continue;
                }
                satisfied = true;
                break;
            }
            if !satisfied {
                crate::weaken::weaken_clause(solver, lit, ref_);
            }
            eliminate_clause(solver, ref_, lit);
        }
    }
    crate::vector::release_vector(solver, lit); // RELEASE_WATCHES (*pos_watches)

    let optimize = solver.options.incremental == 0;
    let v = solver.watches[not_lit as usize];
    let (begin, end) = (v.begin, v.end);
    let mut p = begin;
    while p != end {
        let watch = solver.vectors.stack[p];
        p += 1;
        if watch_is_binary(watch) {
            let other = watch_lit(watch);
            let value = solver.values[other as usize];
            if !optimize && value <= 0 {
                crate::weaken::weaken_binary(solver, not_lit, other);
            }
            eliminate_binary(solver, not_lit, other);
        } else {
            let ref_ = watch_ref(watch);
            if solver.arena.clause(ref_).garbage() {
                continue;
            }
            let mut satisfied = false;
            for &other in solver.arena.clause(ref_).lits() {
                let value = solver.values[other as usize];
                if value <= 0 {
                    continue;
                }
                satisfied = true;
                break;
            }
            if !optimize && !satisfied {
                crate::weaken::weaken_clause(solver, not_lit, ref_);
            }
            eliminate_clause(solver, ref_, not_lit);
        }
    }
    if optimize && !solver.watches[not_lit as usize].empty() {
        crate::weaken::weaken_unit(solver, not_lit);
    }
    crate::vector::release_vector(solver, not_lit); // RELEASE_WATCHES (*neg_watches)

    flush_units_while_connected(solver);
}

// static void try_to_eliminate_all_variables_again (kissat *)
fn try_to_eliminate_all_variables_again(solver: &mut Solver) {
    for idx in 0..solver.vars {
        solver.flags[idx as usize].eliminate = true;
    }
    solver.limits.eliminate.variables.eliminate = 0;
}

// static void set_next_elimination_bound (kissat *, bool complete)
fn set_next_elimination_bound(solver: &mut Solver, complete: bool) {
    let max_bound = solver.options.eliminatebound as u32;
    let current_bound = solver.bounds.eliminate.additional_clauses;
    debug_assert!(current_bound <= max_bound);

    if complete {
        if current_bound == max_bound {
            crate::print::phase(
                solver,
                "eliminate",
                solver.statistics.eliminations, // GET (eliminations)
                format_args!("completed maximum elimination bound {}", current_bound),
            );
            solver.limits.eliminate.variables.eliminate =
                solver.statistics.variables_eliminate;
            solver.limits.eliminate.variables.subsume = solver.statistics.variables_subsume;
            // #ifndef QUIET
            let first = solver.bounds.eliminate.max_bound_completed == 0;
            solver.bounds.eliminate.max_bound_completed += 1;
            crate::report::report(solver, !first, if first { '!' } else { ':' });
        } else {
            let next_bound = if current_bound == 0 {
                1
            } else {
                std::cmp::min(2 * current_bound, max_bound)
            };
            crate::print::phase(
                solver,
                "eliminate",
                solver.statistics.eliminations,
                format_args!(
                    "completed elimination bound {} next {}",
                    current_bound, next_bound
                ),
            );
            solver.bounds.eliminate.additional_clauses = next_bound;
            try_to_eliminate_all_variables_again(solver);
            crate::report::report(solver, false, '^'); // REPORT (0, '^')
        }
    } else {
        crate::print::phase(
            solver,
            "eliminate",
            solver.statistics.eliminations,
            format_args!("incomplete elimination bound {}", current_bound),
        );
    }
}

// static bool can_eliminate_variable (kissat *, unsigned idx)
fn can_eliminate_variable(solver: &Solver, idx: u32) -> bool {
    let flags = solver.flags[idx as usize];

    if !flags.active {
        return false;
    }
    if !flags.eliminate {
        return false;
    }

    true
}

// static bool eliminate_variable (kissat *, unsigned idx)
fn eliminate_variable(solver: &mut Solver, idx: u32) -> bool {
    debug_assert!(!solver.inconsistent);
    debug_assert!(can_eliminate_variable(solver, idx));

    solver.flags[idx as usize].eliminate = false;

    let mut lit: u32 = 0;
    if !crate::resolve::generate_resolvents(solver, idx, &mut lit) {
        return false;
    }
    connect_resolvents(solver);
    if !solver.inconsistent {
        weaken_clauses(solver, lit);
    }
    solver.statistics.eliminated += 1; // INC (eliminated)
    crate::flags::mark_eliminated_variable(solver, idx);
    if solver.gate_eliminated {
        solver.statistics.gates_eliminated += 1; // INC (gates_eliminated): STATISTIC kept
        // METRICS-only *solver->gate_eliminated += 1 — compiled out.
    }
    true
}

// static void eliminate_variables (kissat *)
fn eliminate_variables(solver: &mut Solver) {
    crate::print::very_verbose(
        solver,
        format_args!(
            "trying to eliminate variables with bound {}",
            solver.bounds.eliminate.additional_clauses
        ),
    );
    debug_assert!(!solver.inconsistent);
    // #ifndef QUIET
    let before = solver.active;
    let mut eliminated: u32 = 0;
    let mut tried: u64 = 0;
    // QUIRK ported: outer counter is shadowed in the loop and stays 0.
    let last_round_eliminated_outer: u32 = 0;

    // SET_EFFORT_LIMIT (resolution_limit, eliminate, eliminate_resolutions)
    let resolution_limit =
        crate::set_effort_limit!(solver, eliminate, eliminateeffort, eliminate_resolutions);

    let mut complete: bool;
    let mut round: i32 = 0;

    let forward = solver.options.forward != 0;

    loop {
        round += 1;

        if forward {
            let propagate = solver.propagate;
            complete = crate::forward::forward_subsume_during_elimination(solver);
            if solver.inconsistent {
                break;
            }
            crate::watch::flush_large_connected(solver);
            crate::watch::connect_irredundant_large_clauses(solver);
            solver.propagate = propagate;
            flush_units_while_connected(solver);
            if solver.inconsistent {
                break;
            }
        } else {
            crate::watch::connect_irredundant_large_clauses(solver);
            complete = true;
        }

        let last_round_scheduled = schedule_variables(solver);
        {
            let active = solver.active;
            crate::print::very_verbose(
                solver,
                format_args!(
                    "scheduled {} variables {:.0}% to eliminate in round {}",
                    last_round_scheduled,
                    crate::format::percent(last_round_scheduled as f64, active as f64),
                    round
                ),
            );
        }

        let mut last_round_eliminated: u32 = 0; // shadows the outer (C quirk)

        while !solver.inconsistent && !crate::heap::empty_heap(&solver.schedule) {
            if terminated!(solver, eliminate_terminated_1) {
                complete = false;
                break;
            }
            let idx = crate::heap::pop_max_heap(&mut solver.schedule);
            if !can_eliminate_variable(solver, idx) {
                continue;
            }
            if solver.statistics.eliminate_resolutions > resolution_limit {
                crate::print::extremely_verbose(
                    solver,
                    format_args!(
                        "eliminate round {} hits resolution limit {} at {} resolutions",
                        round, resolution_limit, solver.statistics.eliminate_resolutions
                    ),
                );
                complete = false;
                break;
            }
            tried += 1;
            if eliminate_variable(solver, idx) {
                last_round_eliminated += 1;
            }
            if solver.inconsistent {
                break;
            }
            flush_units_while_connected(solver);
        }

        if last_round_eliminated != 0 {
            complete = false;
            eliminated += last_round_eliminated;
        }

        if !solver.inconsistent {
            crate::watch::flush_large_connected(solver);
            crate::collect::dense_collect(solver);
        }

        crate::print::phase(
            solver,
            "eliminate",
            solver.statistics.eliminations,
            format_args!(
                "eliminated {} variables {:.0}% in round {}",
                last_round_eliminated,
                crate::format::percent(
                    last_round_eliminated as f64,
                    last_round_scheduled as f64
                ),
                round
            ),
        );
        crate::report::report(solver, last_round_eliminated == 0, 'e');

        if solver.inconsistent {
            break;
        }
        crate::heap::release_heap(&mut solver.schedule);
        if complete {
            break;
        }
        if round == solver.options.eliminaterounds {
            break;
        }
        if solver.statistics.eliminate_resolutions > resolution_limit {
            break;
        }
        if terminated!(solver, eliminate_terminated_2) {
            break;
        }
    }

    let remain = crate::heap::size_heap(&solver.schedule) as u32;
    crate::heap::release_heap(&mut solver.schedule);
    // #ifndef QUIET
    {
        let active = solver.active;
        crate::print::very_verbose(
            solver,
            format_args!(
                "eliminated {} variables {:.0}% of {} tried ({} remain {:.0}%)",
                eliminated,
                crate::format::percent(eliminated as f64, tried as f64),
                tried,
                remain,
                crate::format::percent(remain as f64, active as f64)
            ),
        );
        crate::print::phase(
            solver,
            "eliminate",
            solver.statistics.eliminations,
            format_args!(
                "eliminated {} variables {:.0}% out of {} in {} rounds",
                eliminated,
                crate::format::percent(eliminated as f64, before as f64),
                before,
                round
            ),
        );
    }
    if !solver.inconsistent {
        let complete = remain == 0 && last_round_eliminated_outer == 0;
        set_next_elimination_bound(solver, complete);
        if !complete {
            let mut dropped: u32 = 0;
            for idx in 0..solver.vars {
                let f = &mut solver.flags[idx as usize];
                if f.eliminate {
                    f.eliminate = false;
                    dropped += 1;
                }
            }
            crate::print::very_verbose(
                solver,
                format_args!("dropping {} eliminate candidates", dropped),
            );
        }
    }
}

// static void init_map_and_kitten (kissat *)
fn init_map_and_kitten(solver: &mut Solver) {
    if solver.options.definitions == 0 {
        return;
    }
    debug_assert!(solver.kitten.is_none());
    solver.kitten = Some(crate::kitten::kitten_embedded());
}

// static void reset_map_and_kitten (kissat *)
fn reset_map_and_kitten(solver: &mut Solver) {
    if let Some(kitten) = solver.kitten.take() {
        crate::kitten::kitten_release(kitten);
    }
}

// static void eliminate (kissat *) — renamed, see module PORT NOTE.
fn eliminate_inner(solver: &mut Solver) {
    crate::backtrack::backtrack_propagate_and_flush_trail(solver);
    debug_assert!(!solver.inconsistent);
    // STOP_SEARCH_AND_START_SIMPLIFIER (eliminate)
    crate::profile::stop_search_and_start_simplifier_checked(solver, Prof::eliminate);
    crate::print::phase(
        solver,
        "eliminate",
        solver.statistics.eliminations,
        format_args!(
            "elimination limit of {} conflicts hit",
            solver.limits.eliminate.conflicts
        ),
    );
    init_map_and_kitten(solver);
    crate::dense::enter_dense_mode(solver, None);
    eliminate_variables(solver);
    crate::dense::resume_sparse_mode(solver, true, None);
    reset_map_and_kitten(solver);
    // kissat_check_statistics: compiled out (NDEBUG).
    // STOP_SIMPLIFIER_AND_RESUME_SEARCH (eliminate)
    crate::profile::stop_simplifier_and_resume_search_checked(solver, Prof::eliminate);
}

/// Port of `kissat_eliminate`.
pub fn eliminate(solver: &mut Solver) -> i32 {
    debug_assert!(!solver.inconsistent);
    solver.statistics.eliminations += 1; // INC (eliminations)
    eliminate_inner(solver);
    crate::classify::classify(solver);
    // UPDATE_CONFLICT_LIMIT (eliminate, eliminations, NLOG2N, true)
    crate::update_conflict_limit!(
        solver,
        eliminate,
        eliminateint,
        eliminations,
        |n| crate::kimits::nlogpown(n, 2),
        true
    );
    solver.last.ticks.eliminate = solver.statistics.search_ticks;
    if solver.inconsistent {
        20
    } else {
        0
    }
}
