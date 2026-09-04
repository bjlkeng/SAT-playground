// Port of src/forward.c (kissat 4.0.4).
//
// Forward subsumption / strengthening during bounded variable elimination.
//
// PORT NOTE: GET (forward_subsumptions) is METRIC in this build — phase
// messages pass u64::MAX so no count is printed; INC (forward_subsumptions)
// is a no-op.  INC (duplicated) is METRIC (no-op); forward_subsumed /
// forward_strengthened are STATISTIC (kept as real, never-printed fields);
// subsumed / strengthened / subsumption_checks / forward_checks /
// forward_steps are COUNTERs (real).
// PORT NOTE: RADIX_SORT is wrapped in START/STOP (radix) profile calls per
// the sort.rs convention (hoisted out of the macro).
// PORT NOTE: the strengthen-to-binary path sets c->garbage directly (C does
// NOT call kissat_mark_clause_as_garbage there because the proof shrink
// already replaced the clause) and adjusts clauses_irredundant /
// clauses_binary by hand exactly as C does.

use crate::internal::{Solver, INVALID};
use crate::profile::Prof;
use crate::reference::Reference;
use crate::terminated;
use crate::watch::{watch_is_binary, watch_lit, watch_ref};

// static size_t remove_duplicated_binaries_with_literal (kissat *, unsigned)
fn remove_duplicated_binaries_with_literal(solver: &mut Solver, lit: u32) -> u64 {
    let v = solver.watches[lit as usize];
    let begin = v.begin;
    let end = v.end;
    let mut q = begin;
    let mut p = begin;

    while p != end {
        // const watch watch = *q++ = *p++;
        let watch = solver.vectors.stack[p];
        solver.vectors.stack[q] = watch;
        q += 1;
        p += 1;
        debug_assert!(watch_is_binary(watch));
        let other = watch_lit(watch);
        let f = solver.flags[crate::literal::idx(other) as usize];
        if !f.active {
            continue;
        }
        if !f.subsume {
            continue;
        }
        let marked = solver.marks[other as usize];
        if marked != 0 {
            q -= 1;
            if lit < other {
                crate::clause::delete_binary(solver, lit, other);
                // INC (duplicated): METRIC, compiled out.
            }
        } else {
            let not_other = crate::literal::not(other);
            if solver.marks[not_other as usize] != 0 {
                // duplicate hyper unary resolution
                solver.delayed.push(lit);
            }
            solver.marks[other as usize] = 1;
        }
    }

    // for (const watch *r = begin; r != q; r++) marks[r->binary.lit] = 0;
    for &watch in &solver.vectors.stack[begin..q] {
        solver.marks[watch_lit(watch) as usize] = 0;
    }

    if q == end {
        return 0;
    }

    let removed = (end - q) as u64;
    crate::vector::resize_vector(solver, lit, q - begin); // SET_END_OF_WATCHES

    removed
}

// static void remove_all_duplicated_binary_clauses (kissat *)
fn remove_all_duplicated_binary_clauses(solver: &mut Solver) {
    // #if !defined(QUIET) || !defined(NDEBUG)
    let mut removed: u64 = 0;
    debug_assert!(solver.delayed.is_empty());

    for idx in 0..solver.vars {
        let flags = solver.flags[idx as usize];
        if !flags.active {
            continue;
        }
        if !flags.subsume {
            continue;
        }
        let lit = crate::literal::lit(idx);
        let not_lit = crate::literal::not(lit);
        removed += remove_duplicated_binaries_with_literal(solver, lit);
        removed += remove_duplicated_binaries_with_literal(solver, not_lit);
    }
    debug_assert!(removed & 1 == 0);

    let units = solver.delayed.len();
    if units != 0 {
        for i in 0..solver.delayed.len() {
            let unit = solver.delayed[i];
            let value = solver.values[unit as usize];
            if value > 0 {
                continue; // skipping satisfied resolved unit
            }
            if value < 0 {
                // found falsified resolved unit
                // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
                if solver.proof.is_some() {
                    crate::proof::add_empty_to_proof(solver);
                }
                solver.inconsistent = true;
                break;
            }
            crate::assign::learned_unit(solver, unit);
        }
        solver.delayed.clear();
        if !solver.inconsistent {
            crate::eliminate::flush_units_while_connected(solver);
        }
    }

    crate::report::report(solver, removed == 0 && units == 0, '2');
}

// static void find_forward_subsumption_candidates (kissat *, references *)
fn find_forward_subsumption_candidates(solver: &mut Solver, candidates: &mut Vec<Reference>) {
    let clslim = solver.options.subsumeclslim as u32;

    let last_irredundant = solver.last_irredundant; // NULL if INVALID_REF

    let mut ref_: Reference = 0;
    while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        // if (last_irredundant && c > last_irredundant) break;
        if last_irredundant != crate::reference::INVALID_REF && ref_ > last_irredundant {
            break;
        }
        if solver.arena.clause(ref_).garbage() {
            ref_ = next;
            continue;
        }
        solver.arena.clause_mut(ref_).set_subsume(false);
        if solver.arena.clause(ref_).redundant() {
            ref_ = next;
            continue;
        }
        let size = solver.arena.clause(ref_).size();
        if size > clslim {
            ref_ = next;
            continue;
        }
        debug_assert!(size > 2);
        let mut subsume: u32 = 0;
        for &lit in solver.arena.clause(ref_).lits() {
            let idx = crate::literal::idx(lit);
            let f = solver.flags[idx as usize];
            if f.subsume {
                subsume += 1;
            }
            if solver.values[lit as usize] > 0 {
                crate::clause::mark_clause_as_garbage(solver, ref_);
                break;
            }
        }
        if solver.arena.clause(ref_).garbage() {
            ref_ = next;
            continue;
        }
        if subsume < 2 {
            ref_ = next;
            continue;
        }
        candidates.push(ref_);
        ref_ = next;
    }
}

// static void sort_forward_subsumption_candidates (kissat *, references *)
fn sort_forward_subsumption_candidates(solver: &mut Solver, candidates: &mut Vec<Reference>) {
    // RADIX_SORT (reference, unsigned, size, references, GET_SIZE_OF_REFERENCE)
    crate::profile::start_checked(solver, Prof::radix); // START (radix)
    {
        let arena = &solver.arena;
        crate::sort::radix_sort::<Reference, u32, _>(candidates, |&r| arena.clause(r).size());
    }
    crate::profile::stop_checked(solver, Prof::radix); // STOP (radix)
}

// static inline bool forward_literal (kissat *, unsigned lit, bool binaries,
//                                     unsigned *remove, unsigned limit)
fn forward_literal(
    solver: &mut Solver,
    lit: u32,
    binaries: bool,
    remove: &mut u32,
    limit: u32,
) -> bool {
    let v = solver.watches[lit as usize];
    let size_watches = v.size();

    if size_watches == 0 {
        return false;
    }

    if size_watches > limit as usize {
        return false;
    }

    let begin = v.begin;
    let end = v.end;
    let mut q = begin;
    let mut p = begin;

    let mut steps: u64 = 1 + crate::utilities::cache_lines(size_watches as u64, 4);
    let mut checks: u64 = 0;

    let mut subsume = false;

    while p != end {
        // const watch watch = *q++ = *p++;
        let watch = solver.vectors.stack[p];
        solver.vectors.stack[q] = watch;
        q += 1;
        p += 1;

        if watch_is_binary(watch) {
            if !binaries {
                continue;
            }

            let other = watch_lit(watch);
            if solver.marks[other as usize] != 0 {
                subsume = true;
                break;
            } else {
                let not_other = crate::literal::not(other);
                if solver.marks[not_other as usize] != 0 {
                    debug_assert!(!subsume);
                    *remove = not_other;
                    break;
                }
            }
        } else {
            let ref_ = watch_ref(watch);
            steps += 1;

            if solver.arena.clause(ref_).garbage() {
                q -= 1;
                continue;
            }

            checks += 1;
            subsume = true;

            let mut candidate = INVALID;

            for &other in solver.arena.clause(ref_).lits() {
                if solver.marks[other as usize] != 0 {
                    continue;
                }
                let value = solver.values[other as usize];
                if value < 0 {
                    continue;
                }
                if value > 0 {
                    crate::clause::mark_clause_as_garbage(solver, ref_);
                    candidate = INVALID;
                    subsume = false;
                    break;
                }
                if !subsume {
                    debug_assert!(candidate != INVALID);
                    candidate = INVALID;
                    break;
                }
                subsume = false;
                let not_other = crate::literal::not(other);
                if solver.marks[not_other as usize] == 0 {
                    debug_assert!(candidate == INVALID);
                    break;
                }
                candidate = not_other;
            }

            if solver.arena.clause(ref_).garbage() {
                debug_assert!(!subsume);
                q -= 1;
                break;
            }

            if subsume {
                break;
            }

            if candidate != INVALID {
                *remove = candidate;
            }
        }
    }

    if p != q {
        while p != end {
            solver.vectors.stack[q] = solver.vectors.stack[p];
            q += 1;
            p += 1;
        }
        crate::vector::resize_vector(solver, lit, q - begin); // SET_END_OF_WATCHES
    }

    solver.statistics.subsumption_checks += checks; // ADD (subsumption_checks, ...)
    solver.statistics.forward_checks += checks; // ADD (forward_checks, ...)
    solver.statistics.forward_steps += steps; // ADD (forward_steps, ...)

    subsume
}

// static inline bool forward_marked_clause (kissat *, clause *c,
//                                           unsigned *remove)
fn forward_marked_clause(solver: &mut Solver, ref_: Reference, remove: &mut u32) -> bool {
    let limit = solver.options.subsumeocclim as u32;
    solver.statistics.forward_steps += 1; // INC (forward_steps)

    let size = solver.arena.clause(ref_).size();
    for i in 0..size {
        let lit = solver.arena.clause(ref_).lit(i);
        let idx = crate::literal::idx(lit);
        if !solver.flags[idx as usize].active {
            continue;
        }

        debug_assert!(solver.values[lit as usize] == 0);

        if forward_literal(solver, lit, true, remove, limit) {
            return true;
        }

        if forward_literal(solver, crate::literal::not(lit), false, remove, limit) {
            return true;
        }
    }
    false
}

// static bool forward_subsumed_clause (kissat *, clause *c,
//                                      bool *strengthened,
//                                      unsigneds *new_binaries)
fn forward_subsumed_clause(
    solver: &mut Solver,
    ref_: Reference,
    strengthened: &mut bool,
    new_binaries: &mut Vec<u32>,
) -> bool {
    debug_assert!(!solver.arena.clause(ref_).garbage());

    let mut non_false: u32 = 0;
    let mut unit = INVALID;

    for &lit in solver.arena.clause(ref_).lits() {
        let value = solver.values[lit as usize];
        if value < 0 {
            continue;
        }
        if value > 0 {
            crate::clause::mark_clause_as_garbage(solver, ref_);
            break;
        }
        solver.marks[lit as usize] = 1;
        // if (non_false++) unit ^= lit; else unit = lit;
        if non_false != 0 {
            unit ^= lit;
        } else {
            unit = lit;
        }
        non_false += 1;
    }

    if solver.arena.clause(ref_).garbage() || non_false <= 1 {
        for &lit in solver.arena.clause(ref_).lits() {
            solver.marks[lit as usize] = 0;
        }
    }

    if solver.arena.clause(ref_).garbage() {
        return false;
    }

    if non_false == 0 {
        // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
        if solver.proof.is_some() {
            crate::proof::add_empty_to_proof(solver);
        }
        solver.inconsistent = true;
        return false;
    }

    if non_false == 1 {
        crate::assign::learned_unit(solver, unit);
        crate::clause::mark_clause_as_garbage(solver, ref_);
        crate::eliminate::flush_units_while_connected(solver);
        return false;
    }

    let mut remove = INVALID;
    let subsume = forward_marked_clause(solver, ref_, &mut remove);

    for &lit in solver.arena.clause(ref_).lits() {
        solver.marks[lit as usize] = 0;
    }

    if subsume {
        crate::clause::mark_clause_as_garbage(solver, ref_);
        solver.statistics.subsumed += 1; // INC (subsumed)
        solver.statistics.forward_subsumed += 1; // INC: STATISTIC kept
    } else if remove != INVALID {
        *strengthened = true;
        solver.statistics.strengthened += 1; // INC (strengthened)
        solver.statistics.forward_strengthened += 1; // INC: STATISTIC kept
        if non_false == 2 {
            unit ^= remove;
            crate::assign::learned_unit(solver, unit);
            crate::clause::mark_clause_as_garbage(solver, ref_);
            crate::eliminate::flush_units_while_connected(solver);
        } else {
            // SHRINK_CLAUSE_IN_PROOF (c, remove, INVALID_LIT)
            if solver.proof.is_some() {
                crate::proof::shrink_clause_in_proof(solver, ref_, remove, INVALID);
            }
            // CHECK_SHRINK_CLAUSE: compiled out (NDEBUG).
            crate::inline::mark_removed_literal(solver, remove);
            if non_false > 3 {
                let mut new_size: u32 = 0;
                let old_size = solver.arena.clause(ref_).size();
                for i in 0..old_size {
                    let lit = solver.arena.clause(ref_).lit(i);
                    if remove == lit {
                        continue;
                    }
                    let value = solver.values[lit as usize];
                    if value < 0 {
                        continue;
                    }
                    debug_assert!(value == 0);
                    solver.arena.clause_mut(ref_).set_lit(new_size, lit);
                    new_size += 1;
                    crate::inline::mark_added_literal(solver, lit);
                }
                debug_assert!(new_size == non_false - 1);
                debug_assert!(new_size > 2);
                {
                    let mut c = solver.arena.clause_mut(ref_);
                    if !c.shrunken() {
                        c.set_shrunken(true);
                        let old = c.size();
                        c.set_lit(old - 1, INVALID); // lits[c->size - 1] = INVALID_LIT
                    }
                    c.set_size(new_size);
                    c.set_searched(2);
                    c.set_subsume(true);
                }
            } else {
                debug_assert!(non_false == 3);
                debug_assert!(!solver.arena.clause(ref_).garbage());
                // ADD (arena_garbage, bytes): METRIC, compiled out.
                solver.arena.clause_mut(ref_).set_garbage(true); // c->garbage = true
                let mut first = INVALID;
                let mut second = INVALID;
                let size = solver.arena.clause(ref_).size();
                for i in 0..size {
                    let lit = solver.arena.clause(ref_).lit(i);
                    if lit == remove {
                        continue;
                    }
                    let value = solver.values[lit as usize];
                    if value < 0 {
                        continue;
                    }
                    debug_assert!(value == 0);
                    if first == INVALID {
                        first = lit;
                    } else {
                        debug_assert!(second == INVALID);
                        second = lit;
                    }
                    crate::inline::mark_added_literal(solver, lit);
                }
                debug_assert!(first != INVALID);
                debug_assert!(second != INVALID);
                crate::watch::watch_other(solver, first, second);
                crate::watch::watch_other(solver, second, first);
                debug_assert!(solver.statistics.clauses_irredundant > 0);
                solver.statistics.clauses_irredundant -= 1;
                debug_assert!(solver.statistics.clauses_binary < u64::MAX);
                solver.statistics.clauses_binary += 1;
                new_binaries.push(first);
                new_binaries.push(second);
            }
        }
    }

    subsume
}

// static void connect_subsuming (kissat *, unsigned occlim, clause *c)
fn connect_subsuming(solver: &mut Solver, occlim: u32, ref_: Reference) {
    debug_assert!(!solver.arena.clause(ref_).garbage());

    let mut min_lit = INVALID;
    let mut min_occs: usize = usize::MAX; // MAX_SIZE_T

    let mut subsume = true;

    for &lit in solver.arena.clause(ref_).lits() {
        let idx = crate::literal::idx(lit);
        let flags = solver.flags[idx as usize];
        if !flags.active {
            continue;
        }
        if !flags.subsume {
            subsume = false;
            break;
        }
        let occs = solver.watches[lit as usize].size();
        if min_lit != INVALID && occs > min_occs {
            continue;
        }
        min_lit = lit;
        min_occs = occs;
    }
    if !subsume {
        return;
    }

    if min_occs > occlim as usize {
        return;
    }
    crate::watch::connect_literal(solver, min_lit, ref_);
}

// static bool forward_subsume_all_clauses (kissat *)
fn forward_subsume_all_clauses(solver: &mut Solver) -> bool {
    let mut candidates: Vec<Reference> = Vec::new(); // INIT_STACK

    find_forward_subsumption_candidates(solver, &mut candidates);
    // #ifndef QUIET
    let scheduled = candidates.len();
    {
        let clauses_irredundant = solver.statistics.clauses_irredundant;
        crate::print::phase(
            solver,
            "forward",
            u64::MAX, // GET (forward_subsumptions): METRIC
            format_args!(
                "scheduled {} irredundant clauses {:.0}%",
                scheduled,
                crate::format::percent(scheduled as f64, clauses_irredundant as f64)
            ),
        );
    }
    sort_forward_subsumption_candidates(solver, &mut candidates);

    let mut p: usize = 0; // reference *p = BEGIN_STACK (candidates)

    // #ifndef QUIET
    let mut subsumed: u64 = 0;
    let mut strengthened: u64 = 0;
    let mut checked: u64 = 0;

    let occlim = solver.options.subsumeocclim as u32;

    let mut new_binaries: Vec<u32> = Vec::new(); // INIT_STACK

    {
        // SET_EFFORT_LIMIT (steps_limit, forward, forward_steps)
        let steps_limit =
            crate::set_effort_limit!(solver, forward, forwardeffort, forward_steps);

        while p != candidates.len() {
            if solver.statistics.forward_steps > steps_limit {
                break;
            }
            if terminated!(solver, forward_terminated_1) {
                break;
            }
            let ref_ = candidates[p];
            p += 1;
            debug_assert!(!solver.arena.clause(ref_).garbage());
            checked += 1;
            let mut not_subsumed_but_strengthened = false;
            if forward_subsumed_clause(
                solver,
                ref_,
                &mut not_subsumed_but_strengthened,
                &mut new_binaries,
            ) {
                subsumed += 1;
            } else if not_subsumed_but_strengthened {
                strengthened += 1;
            }
            if solver.inconsistent {
                break;
            }
            if !solver.arena.clause(ref_).garbage() {
                connect_subsuming(solver, occlim, ref_);
            }
        }
    }
    // #ifndef QUIET
    if subsumed != 0 {
        crate::print::phase(
            solver,
            "forward",
            u64::MAX, // GET (forward_subsumptions): METRIC
            format_args!(
                "subsumed {} clauses {:.2}% of {} checked {:.0}%",
                subsumed,
                crate::format::percent(subsumed as f64, checked as f64),
                checked,
                crate::format::percent(checked as f64, scheduled as f64)
            ),
        );
    }
    if strengthened != 0 {
        crate::print::phase(
            solver,
            "forward",
            u64::MAX,
            format_args!(
                "strengthened {} clauses {:.2}% of {} checked {:.0}%",
                strengthened,
                crate::format::percent(strengthened as f64, checked as f64),
                checked,
                crate::format::percent(checked as f64, scheduled as f64)
            ),
        );
    }
    if subsumed == 0 && strengthened == 0 {
        crate::print::phase(
            solver,
            "forward",
            u64::MAX,
            format_args!(
                "no clause subsumed nor strengthened out of {} checked {:.0}%",
                checked,
                crate::format::percent(checked as f64, scheduled as f64)
            ),
        );
    }

    for idx in 0..solver.vars {
        solver.flags[idx as usize].subsume = false;
    }

    let mut reactivated: u32 = 0;
    // #ifndef QUIET
    let mut remain: u64 = 0;
    for q in 0..candidates.len() {
        let ref_ = candidates[q];
        if solver.arena.clause(ref_).garbage() {
            continue;
        }
        if q < p && !solver.arena.clause(ref_).subsume() {
            continue;
        }
        remain += 1;
        for &lit in solver.arena.clause(ref_).lits() {
            let idx = crate::literal::idx(lit);
            let f = &mut solver.flags[idx as usize];
            if f.subsume {
                continue;
            }
            f.subsume = true;
            debug_assert!(reactivated < u32::MAX);
            reactivated += 1;
        }
    }

    while !new_binaries.is_empty() {
        let mut lits = [0u32; 2];
        lits[1] = new_binaries.pop().unwrap();
        lits[0] = new_binaries.pop().unwrap();
        for i in 0..2 {
            let lit = lits[i];
            let idx = crate::literal::idx(lit);
            let f = &mut solver.flags[idx as usize];
            if f.subsume {
                continue;
            }
            f.subsume = true;
            debug_assert!(reactivated < u32::MAX);
            reactivated += 1;
        }
    }
    drop(new_binaries); // RELEASE_STACK

    {
        let active = solver.active;
        crate::print::very_verbose(
            solver,
            format_args!(
                "marked {} variables {:.0}% to be reconsidered in next forward subsumption",
                reactivated,
                crate::format::percent(reactivated as f64, active as f64)
            ),
        );
    }
    // #ifndef QUIET
    if remain != 0 {
        crate::print::phase(
            solver,
            "forward",
            u64::MAX,
            format_args!(
                "{} unchecked clauses remain {:.0}%",
                remain,
                crate::format::percent(remain as f64, scheduled as f64)
            ),
        );
    } else {
        crate::print::phase(
            solver,
            "forward",
            u64::MAX,
            format_args!("all {} scheduled clauses checked", scheduled),
        );
    }
    drop(candidates); // RELEASE_STACK
    crate::report::report(solver, subsumed == 0, 's');

    let completed = if solver.inconsistent {
        true
    } else if reactivated != 0 {
        false
    } else {
        true
    };
    crate::print::very_verbose(
        solver,
        format_args!(
            "forward subsumption considered {}complete",
            if completed { "" } else { "in" }
        ),
    );
    completed
}

/// Port of `kissat_forward_subsume_during_elimination`.
pub fn forward_subsume_during_elimination(solver: &mut Solver) -> bool {
    crate::profile::start_checked(solver, Prof::subsume); // START (subsume)
    crate::profile::start_checked(solver, Prof::forward); // START (forward)
    debug_assert!(solver.options.forward != 0);
    // INC (forward_subsumptions): METRIC, compiled out.
    debug_assert!(!solver.watching);
    remove_all_duplicated_binary_clauses(solver);
    let mut complete = true;
    if !solver.inconsistent {
        complete = forward_subsume_all_clauses(solver);
    }
    crate::profile::stop_checked(solver, Prof::forward); // STOP (forward)
    crate::profile::stop_checked(solver, Prof::subsume); // STOP (subsume)
    complete
}
