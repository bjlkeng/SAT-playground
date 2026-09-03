// Port of src/vivify.c (kissat 4.0.4).
//
// Clause vivification: assume the negations of a candidate clause's literals
// in count-sorted order under probing propagation, then subsume / shrink /
// instantiate based on the resulting conflict analysis.
//
// PORT NOTES:
//  - The C `struct vivifier` keeps a back-pointer to the solver; the port
//    passes `&mut Solver` alongside `&mut Vivifier` (counts/schedule/
//    countrefs/sorted are vivifier-owned so they borrow-split from solver).
//  - vivify_deduce pushes LITERALS onto solver->analyzed (unlike the
//    idx-based kissat_push_analyzed protocol); reset_vivify_analyzed
//    matches.  Ported as-is.
//  - swap_first_literal_with_best_watch: the C loop condition reads the
//    OUTER `value` (of the first literal) which is shadowed, not updated, by
//    the inner `value` — so the loop runs fully iff the first literal is
//    falsified.  Quirk ported exactly.
//  - vivify_learn_large marks added literals over `c->size` which is still
//    the OLD size after the memcpy of the shrunken literals (the stale tail
//    is marked too).  Quirk ported exactly.
//  - Statistics tiers: vivifications / vivified / vivify_checks /
//    vivify_probes / vivify_reused are COUNTERs; every vivified_* breakdown
//    plus vivify_units is STATISTIC (kept as real, never-printed fields per
//    statistics.rs policy); probing_ticks is the COUNTER driving the effort
//    limits.
//  - SORT/SORT_STACK carry START/STOP (sort) and RADIX_STACK carries
//    START/STOP (radix) inside the C macros; hoisted around the calls per
//    the sort.rs convention.  START (vivifysort) wraps candidate sorting.
//  - The `#ifndef QUIET` remain-accounting block at the end of vivify_round
//    CONSUMES the schedule stack (POP_STACK loop) — it is compiled into the
//    reference build and kept.
//  - solver->vivifying flag is !NDEBUG/METRICS-only — omitted.
//  - LOG/LOGCLS/LOGCOUNTEDCLS/LOGCOUNTEDREFLITS and `#ifdef LOGGING` blocks
//    are compiled out.

use crate::internal::{Solver, DECISION_REASON, INVALID_TRAIL};
use crate::kimits::DelayId;
use crate::literal::{idx, not, INVALID_LIT};
use crate::profile::Prof;
use crate::propsearch::{binary_conflict, Conflict};
use crate::reference::{Reference, INVALID_REF};
use crate::terminated;
use crate::utilities::percent;

// static inline more_occurrences
#[inline]
fn more_occurrences(counts: &[u32], a: u32, b: u32) -> bool {
    let s = counts[a as usize];
    let t = counts[b as usize];
    // ((t - s) | ((b - a) & ~(s - t))) >> 31  (unsigned wrap-around)
    ((t.wrapping_sub(s)) | ((b.wrapping_sub(a)) & !(s.wrapping_sub(t)))) >> 31 != 0
}

// static vivify_sort_lits_by_counts (SORT carries START/STOP (sort)).
fn vivify_sort_lits_by_counts(solver: &mut Solver, lits: &mut [u32], counts: &[u32]) {
    let mut sorter = std::mem::take(&mut solver.sorter);
    crate::profile::start_checked(solver, Prof::sort); // START (sort)
    crate::sort::sort(&mut sorter, lits, |&a, &b| more_occurrences(counts, a, b));
    crate::profile::stop_checked(solver, Prof::sort); // STOP (sort)
    solver.sorter = sorter;
}

// static vivify_sort_clause_by_counts
fn vivify_sort_clause_by_counts(solver: &mut Solver, c_ref: Reference, counts: &[u32]) {
    let mut sorter = std::mem::take(&mut solver.sorter);
    crate::profile::start_checked(solver, Prof::sort); // START (sort)
    {
        let mut c = solver.arena.clause_mut(c_ref);
        let lits = c.lits_mut();
        crate::sort::sort(&mut sorter, lits, |&a, &b| more_occurrences(counts, a, b));
    }
    crate::profile::stop_checked(solver, Prof::sort); // STOP (sort)
    solver.sorter = sorter;
}

// static count_literal
#[inline]
fn count_literal(lit: u32, counts: &mut [u32]) {
    let c = counts[lit as usize];
    counts[lit as usize] = c + ((c < i32::MAX as u32) as u32);
}

// static count_clause (reads the clause through the arena per literal so the
// counts array can be vivifier-owned).
fn count_clause(solver: &Solver, c_ref: Reference, counts: &mut [u32]) {
    let size = solver.arena.clause(c_ref).size();
    for i in 0..size {
        let lit = solver.arena.clause(c_ref).lit(i);
        count_literal(lit, counts);
    }
}

// static simplify_vivification_candidate
fn simplify_vivification_candidate(solver: &mut Solver, c_ref: Reference) -> bool {
    debug_assert!(solver.level == 0);
    solver.clause.clear();

    let size = solver.arena.clause(c_ref).size();
    for i in 0..size {
        let lit = solver.arena.clause(c_ref).lit(i);
        let value = solver.values[lit as usize];
        if value > 0 {
            crate::clause::mark_clause_as_garbage(solver, c_ref);
            return true;
        }
        if value == 0 {
            solver.clause.push(lit);
        }
    }

    let non_false = solver.clause.len() as u32;
    debug_assert!(non_false <= size);
    if non_false == size {
        return false;
    }

    if non_false == 2 {
        let first = solver.clause[0];
        let second = solver.clause[1];
        crate::clause::new_binary_clause(solver, first, second);
        crate::clause::mark_clause_as_garbage(solver, c_ref);
        return true;
    }

    debug_assert!(non_false > 2);

    // CHECK_AND_ADD_STACK: compiled out (NDEBUG).
    // ADD_STACK_TO_PROOF (solver->clause):
    if solver.proof.is_some() {
        let lits = std::mem::take(&mut solver.clause);
        crate::proof::add_lits_to_proof(solver, &lits);
        solver.clause = lits;
    }
    // REMOVE_CHECKER_CLAUSE: compiled out (NDEBUG).
    // DELETE_CLAUSE_FROM_PROOF (c):
    if solver.proof.is_some() {
        crate::proof::delete_clause_from_proof(solver, c_ref);
    }

    let old_size = size;
    let mut new_size: u32 = 0;
    for i in 0..old_size {
        let lit = solver.arena.clause(c_ref).lit(i);
        let value = solver.values[lit as usize];
        debug_assert!(value <= 0);
        if value == 0 {
            solver.arena.clause_mut(c_ref).set_lit(new_size, lit);
            new_size += 1;
        }
    }

    debug_assert!(new_size > 2);
    debug_assert!(new_size == non_false);
    debug_assert!(new_size < old_size);

    {
        let mut c = solver.arena.clause_mut(c_ref);
        c.set_size(new_size);
        c.set_searched(2);
    }

    let (redundant, glue) = {
        let c = solver.arena.clause(c_ref);
        (c.redundant(), c.glue())
    };
    if redundant && glue >= new_size {
        crate::promote::promote_clause(solver, c_ref, new_size - 1);
    }
    if !solver.arena.clause(c_ref).shrunken() {
        let mut c = solver.arena.clause_mut(c_ref);
        c.set_shrunken(true);
        c.set_lit(old_size - 1, INVALID_LIT);
    }

    false
}

// static vivify_tier1_limit
fn vivify_tier1_limit(solver: &Solver) -> u32 {
    if solver.options.vivifyfocusedtiers != 0 {
        solver.tier1[0]
    } else {
        solver.tier1() // TIER1 == solver->tier1[0]
    }
}

// static vivify_tier2_limit
fn vivify_tier2_limit(solver: &Solver) -> u32 {
    if solver.options.vivifyfocusedtiers != 0 {
        solver.tier2[0]
    } else {
        solver.tier2() // TIER2 == solver->tier2[1] (quirk, see internal.rs)
    }
}

const COUNTREF_COUNTS: usize = 1;

/// C `struct countref` (vivify:1 / size:31 bitfields become plain fields;
/// size is always < 2^31 so the ranking math is identical).
#[derive(Clone, Copy)]
struct Countref {
    vivify: bool,
    size: u32,
    count: [u32; COUNTREF_COUNTS],
    ref_: Reference,
}

/// C `struct vivifier` minus the solver back-pointer (see PORT NOTES).
pub struct Vivifier {
    counts: Vec<u32>, // kissat_calloc (solver, LITS, sizeof (unsigned))
    schedule: Vec<Reference>,
    scheduled: usize,
    tried: usize,
    vivified: usize,
    countrefs: Vec<Countref>,
    sorted: Vec<u32>,
    // #ifndef QUIET
    mode: &'static str,
    name: &'static str,
    tag: char,
    tier: i32,
}

// static init_vivifier
fn init_vivifier(solver: &Solver) -> Vivifier {
    Vivifier {
        counts: vec![0u32; solver.lits() as usize],
        schedule: Vec::new(),
        scheduled: 0,
        tried: 0,
        vivified: 0,
        countrefs: Vec::new(),
        sorted: Vec::new(),
        mode: "",
        name: "",
        tag: ' ',
        tier: 0,
    }
}

// static set_vivifier_mode
fn set_vivifier_mode(vivifier: &mut Vivifier, tier: i32) {
    vivifier.tier = tier;
    match tier {
        1 => {
            vivifier.mode = "vivify-tier1";
            vivifier.name = "tier1";
            vivifier.tag = 'u';
        }
        2 => {
            vivifier.mode = "vivify-tier2";
            vivifier.name = "tier2";
            vivifier.tag = 'v';
        }
        3 => {
            vivifier.mode = "vivify-tier3";
            vivifier.name = "tier3";
            vivifier.tag = 'w';
        }
        _ => {
            debug_assert!(tier == 0);
            vivifier.mode = "vivify-irredundant";
            vivifier.name = "irredundant";
            vivifier.tag = 'x';
        }
    }
}

// static clear_vivifier
fn clear_vivifier(vivifier: &mut Vivifier) {
    for c in vivifier.counts.iter_mut() {
        *c = 0; // memset (counts, 0, LITS * sizeof *counts)
    }
    vivifier.schedule.clear();
    vivifier.countrefs.clear();
    vivifier.sorted.clear();
}

// static schedule_vivification_candidates
fn schedule_vivification_candidates(solver: &mut Solver, vivifier: &mut Vivifier) {
    let tier = vivifier.tier;
    let tier1 = vivify_tier1_limit(solver);
    let tier2 = std::cmp::max(tier1, vivify_tier2_limit(solver));
    debug_assert!(tier1 <= tier2);
    let (lower_glue_limit, upper_glue_limit): (u32, u32) = match tier {
        1 => (0, tier1),
        2 => (if tier1 < tier2 { tier1 + 1 } else { 0 }, tier2),
        3 => (tier2 + 1, u32::MAX),
        _ => {
            debug_assert!(tier == 0);
            (0, u32::MAX)
        }
    };
    debug_assert!(lower_glue_limit <= upper_glue_limit);
    let mut prioritized: usize = 0;
    for prioritize in 0..2u32 {
        // for (all_clauses (c)) — successor computed before the body.
        let mut ref_: Reference = 0;
        while (ref_ as u64) < solver.arena.size_wards() {
            let next = solver.arena.next_clause_ref(ref_);
            'clause: {
                if solver.arena.clause(ref_).garbage() {
                    break 'clause;
                }
                if prioritize != 0 {
                    count_clause(solver, ref_, &mut vivifier.counts);
                }
                let (redundant, glue, vivify_flag) = {
                    let c = solver.arena.clause(ref_);
                    (c.redundant(), c.glue(), c.vivify())
                };
                if tier != 0 {
                    if !redundant {
                        break 'clause;
                    }
                    if glue < lower_glue_limit {
                        break 'clause;
                    }
                    if glue > upper_glue_limit {
                        break 'clause;
                    }
                } else if redundant {
                    break 'clause;
                }
                if vivify_flag != (prioritize != 0) {
                    break 'clause;
                }
                if simplify_vivification_candidate(solver, ref_) {
                    break 'clause;
                }
                if prioritize != 0 {
                    prioritized += 1;
                }
                vivifier.schedule.push(ref_);
            }
            ref_ = next;
        }
    }
    solver.clause.clear(); // CLEAR_STACK (solver->clause)
    let scheduled = vivifier.schedule.len();
    if prioritized != 0 {
        crate::print::phase(
            solver,
            vivifier.mode,
            solver.statistics.vivifications, // GET (vivifications)
            format!(
                "prioritized {} clauses {:.0}%",
                prioritized,
                percent(prioritized as f64, scheduled as f64)
            ),
        );
    } else {
        crate::print::phase(
            solver,
            vivifier.mode,
            solver.statistics.vivifications,
            format!("prioritizing all {} scheduled clauses", scheduled),
        );
        for i in 0..vivifier.schedule.len() {
            let ref_ = vivifier.schedule[i];
            solver.arena.clause_mut(ref_).set_vivify(true);
        }
    }
    vivifier.scheduled = scheduled;
    vivifier.tried = 0;
    vivifier.vivified = 0;
}

// static inline worse_candidate
#[inline]
fn worse_candidate(solver: &Solver, counts: &[u32], r: Reference, s: Reference) -> bool {
    let c = solver.arena.clause(r);
    let d = solver.arena.clause(s);

    if !c.vivify() && d.vivify() {
        return true;
    }
    if c.vivify() && !d.vivify() {
        return false;
    }

    let cl = c.lits();
    let dl = d.lits();

    let mut a = INVALID_LIT;
    let mut b = INVALID_LIT;

    let mut p = 0usize;
    let mut q = 0usize;
    while p != cl.len() && q != dl.len() {
        a = cl[p];
        p += 1;
        b = dl[q];
        q += 1;
        let u = counts[a as usize];
        let v = counts[b as usize];
        if u < v {
            return true;
        }
        if u > v {
            return false;
        }
    }

    if p != cl.len() && q == dl.len() {
        return true;
    }
    if p == cl.len() && q != dl.len() {
        return false;
    }

    debug_assert!(p == cl.len() && q == dl.len());

    if a < b {
        return true;
    }
    if a > b {
        return false;
    }

    r < s
}

// static sort_vivification_candidates_after_sorting_literals
fn sort_vivification_candidates_after_sorting_literals(
    solver: &mut Solver,
    vivifier: &mut Vivifier,
) {
    let mut sorter = std::mem::take(&mut solver.sorter);
    crate::profile::start_checked(solver, Prof::sort); // START (sort) in SORT_STACK
    {
        let counts = &vivifier.counts;
        let solver_ref: &Solver = solver;
        crate::sort::sort_stack(&mut sorter, &mut vivifier.schedule, |&a, &b| {
            worse_candidate(solver_ref, counts, a, b)
        });
    }
    crate::profile::stop_checked(solver, Prof::sort); // STOP (sort)
    solver.sorter = sorter;
}

// static sort_scheduled_candidate_literals
fn sort_scheduled_candidate_literals(solver: &mut Solver, vivifier: &mut Vivifier) {
    for i in 0..vivifier.schedule.len() {
        let ref_ = vivifier.schedule[i];
        vivify_sort_clause_by_counts(solver, ref_, &vivifier.counts);
    }
}

// static inline init_countref
fn init_countref(solver: &Solver, counts: &[u32], ref_: Reference) -> Countref {
    let c = solver.arena.clause(ref_);
    debug_assert!(COUNTREF_COUNTS as u32 <= c.size());
    let mut cr = Countref {
        vivify: c.vivify(),
        size: c.size(),
        count: [0; COUNTREF_COUNTS],
        ref_,
    };
    let mut lits_sel = [INVALID_LIT; COUNTREF_COUNTS];
    for i in 0..COUNTREF_COUNTS {
        let mut best = INVALID_LIT;
        let mut best_count: u32 = 0;
        'literals: for &lit in c.lits() {
            for j in 0..i {
                if lits_sel[j] == lit {
                    continue 'literals; // goto CONTINUE_WITH_NEXT_LITERAL
                }
            }
            let lit_count = counts[lit as usize];
            debug_assert!(lit_count != 0);
            if lit_count <= best_count {
                continue;
            }
            best_count = lit_count;
            best = lit;
        }
        debug_assert!(best != INVALID_LIT);
        debug_assert!(best_count != 0);
        cr.count[i] = best_count;
        lits_sel[i] = best;
    }
    cr
}

// static init_countrefs
fn init_countrefs(solver: &mut Solver, vivifier: &mut Vivifier) {
    debug_assert!(vivifier.countrefs.is_empty());
    for i in 0..vivifier.schedule.len() {
        let ref_ = vivifier.schedule[i];
        let cr = init_countref(solver, &vivifier.counts, ref_);
        vivifier.countrefs.push(cr);
    }
    vivifier.schedule.clear(); // RELEASE_STACK (*schedule)
}

// static rank_vivification_candidates (RADIX_STACK carries START/STOP (radix)).
fn rank_vivification_candidates(solver: &mut Solver, vivifier: &mut Vivifier) {
    crate::profile::start_checked(solver, Prof::radix);
    // RANK_COUNTREF_BY_INVERSE_SIZE: (unsigned) ~(CR).size
    crate::sort::radix_stack::<Countref, u32, _>(&mut vivifier.countrefs, |cr| !cr.size);
    crate::profile::stop_checked(solver, Prof::radix);
    for i in 0..COUNTREF_COUNTS {
        crate::profile::start_checked(solver, Prof::radix);
        crate::sort::radix_stack::<Countref, u32, _>(&mut vivifier.countrefs, |cr| {
            cr.count[COUNTREF_COUNTS - 1 - i]
        });
        crate::profile::stop_checked(solver, Prof::radix);
    }
    crate::profile::start_checked(solver, Prof::radix);
    crate::sort::radix_stack::<Countref, u32, _>(&mut vivifier.countrefs, |cr| cr.vivify as u32);
    crate::profile::stop_checked(solver, Prof::radix);
}

// static copy_countrefs
fn copy_countrefs(vivifier: &mut Vivifier) {
    debug_assert!(vivifier.schedule.is_empty());
    for i in 0..vivifier.countrefs.len() {
        let cr = vivifier.countrefs[i];
        vivifier.schedule.push(cr.ref_);
    }
    vivifier.countrefs.clear(); // RELEASE_STACK (*countrefs)
}

// static sort_vivification_candidates
fn sort_vivification_candidates(solver: &mut Solver, vivifier: &mut Vivifier) {
    crate::profile::start_checked(solver, Prof::vivifysort); // START (vivifysort)
    if vivifier.tier != 0 {
        crate::print::extremely_verbose(
            solver,
            format!(
                "sorting {} vivification candidates precisely",
                vivifier.name
            ),
        );
        sort_scheduled_candidate_literals(solver, vivifier);
        sort_vivification_candidates_after_sorting_literals(solver, vivifier);
    } else {
        crate::print::extremely_verbose(
            solver,
            format!(
                "sorting {} vivification candidates imprecisely by first {} literals",
                vivifier.name, COUNTREF_COUNTS as u32
            ),
        );
        init_countrefs(solver, vivifier);
        rank_vivification_candidates(solver, vivifier);
        copy_countrefs(vivifier);
    }
    crate::profile::stop_checked(solver, Prof::vivifysort); // STOP (vivifysort)
}

// `reason->redundant` for either kind of conflict handle.
#[inline]
fn conflict_redundant(solver: &Solver, c: Conflict) -> bool {
    match c {
        Conflict::Binary => solver.conflict.header & crate::clause::REDUNDANT_BIT != 0,
        Conflict::Clause(r) => solver.arena.clause(r).redundant(),
    }
}

// `subsuming->glue` for either kind of conflict handle.
#[inline]
fn conflict_glue(solver: &Solver, c: Conflict) -> u32 {
    match c {
        Conflict::Binary => solver.conflict.header & crate::clause::GLUE_MASK,
        Conflict::Clause(r) => solver.arena.clause(r).glue(),
    }
}

// static vivify_deduce.  Returns (subsuming, redundant); on a subsuming
// early return the C caller's `redundant` stays false, matching the tuple.
fn vivify_deduce(
    solver: &mut Solver,
    cand_ref: Reference,
    conflict: Option<Conflict>,
    implied: u32,
) -> (Option<Conflict>, bool) {
    let mut redundant = false;
    let mut subsumes;

    debug_assert!(solver.level != 0);
    debug_assert!(solver.clause.is_empty());
    debug_assert!(solver.analyzed.is_empty());

    if implied != INVALID_LIT {
        let not_implied = not(implied);
        let a = &mut solver.assigned[idx(not_implied) as usize];
        debug_assert!(a.level != 0);
        debug_assert!(!a.analyzed);
        a.analyzed = true;
        solver.analyzed.push(not_implied); // literal, not idx (see PORT NOTES)
        solver.clause.push(implied);
    } else {
        let reason: Conflict = conflict.unwrap_or(Conflict::Clause(cand_ref));
        debug_assert!(!matches!(reason, Conflict::Clause(r) if solver.arena.clause(r).garbage()));
        if conflict_redundant(solver, reason) {
            redundant = true;
        }
        subsumes = conflict.is_some(); // (reason != candidate)
        let reason_size = crate::analyze::conflict_size(solver, reason);
        for i in 0..reason_size {
            let other = crate::analyze::conflict_lit(solver, reason, i);
            debug_assert!(solver.values[other as usize] < 0);
            let value = crate::internal::fixed(solver, other);
            if value < 0 {
                continue;
            }
            debug_assert!(value == 0);
            let a = &mut solver.assigned[idx(other) as usize];
            debug_assert!(a.level != 0);
            debug_assert!(!a.analyzed);
            a.analyzed = true;
            solver.analyzed.push(other);
            if solver.marks[other as usize] <= 0 {
                subsumes = false;
            }
        }
        if conflict.is_some() && conflict_redundant(solver, reason) {
            if let Conflict::Clause(r) = reason {
                crate::deduce::recompute_and_promote(solver, r);
            }
        }
        if subsumes {
            return (Some(reason), false);
        }
    }

    let mut analyzed = 0usize;
    while analyzed < solver.analyzed.len() {
        let not_lit = solver.analyzed[analyzed];
        let lit = not(not_lit);
        debug_assert!(solver.values[lit as usize] > 0);
        analyzed += 1;
        let a = solver.assigned[idx(lit) as usize];
        debug_assert!(a.level != 0);
        debug_assert!(a.analyzed);
        if a.reason == DECISION_REASON {
            solver.clause.push(not_lit);
        } else if a.binary {
            let other = a.reason;
            if solver.marks[lit as usize] > 0 && solver.marks[other as usize] > 0 {
                let subsuming = binary_conflict(solver, lit, other);
                return (Some(subsuming), false);
            }
            debug_assert!(solver.values[other as usize] < 0);
            let b_idx = idx(other) as usize;
            debug_assert!(solver.assigned[b_idx].level != 0);
            if solver.assigned[b_idx].analyzed {
                continue;
            }
            solver.assigned[b_idx].analyzed = true;
            solver.analyzed.push(other);
        } else {
            let ref_ = a.reason;
            let reason_redundant = solver.arena.clause(ref_).redundant();
            debug_assert!(ref_ != cand_ref);
            if reason_redundant {
                redundant = true;
            }
            subsumes = true;
            let size = solver.arena.clause(ref_).size();
            for i in 0..size {
                let other = solver.arena.clause(ref_).lit(i);
                if solver.marks[other as usize] <= 0 {
                    subsumes = false;
                }
                if other == lit {
                    continue;
                }
                debug_assert!(other != not_lit);
                debug_assert!(solver.values[other as usize] < 0);
                let b_idx = idx(other) as usize;
                if solver.assigned[b_idx].level == 0 {
                    continue;
                }
                if solver.assigned[b_idx].analyzed {
                    continue;
                }
                solver.assigned[b_idx].analyzed = true;
                solver.analyzed.push(other);
            }
            if reason_redundant {
                crate::deduce::recompute_and_promote(solver, ref_);
            }
            if subsumes {
                return (Some(Conflict::Clause(ref_)), false);
            }
        }
    }

    (None, redundant)
}

// static reset_vivify_analyzed
fn reset_vivify_analyzed(solver: &mut Solver) {
    for i in 0..solver.analyzed.len() {
        let lit = solver.analyzed[i];
        let a = &mut solver.assigned[idx(lit) as usize];
        a.analyzed = false;
    }
    solver.analyzed.clear();
    solver.clause.clear();
}

// static vivify_shrinkable
fn vivify_shrinkable(solver: &Solver, sorted: &[u32], conflict: bool) -> bool {
    debug_assert!(solver.clause.len() <= sorted.len());
    if solver.clause.len() == sorted.len() {
        return false;
    }
    let mut count_implied: u32 = 0;
    for &lit in sorted.iter() {
        let value = solver.values[lit as usize];
        if value == 0 {
            return true; // unassigned thus shrinking
        }
        if value > 0 {
            if conflict {
                return true; // implied literal with conflict thus shrinking
            }
            let previous = count_implied;
            count_implied += 1;
            if previous != 0 {
                return true; // at least two implied literals thus shrinking
            }
        } else {
            let a = &solver.assigned[idx(lit) as usize];
            debug_assert!(a.level != 0);
            if !a.analyzed {
                return true; // non-analyzed thus shrinking
            }
            if a.reason != DECISION_REASON {
                return true; // implied falsified thus shrinking
            }
        }
    }
    false
}

// static vivify_learn_unit
fn vivify_learn_unit(solver: &mut Solver, c_ref: Reference) {
    debug_assert!(solver.clause.len() == 1);
    crate::backtrack::backtrack_without_updating_phases(solver, 0);
    let unit = solver.clause[0];
    crate::assign::learned_unit(solver, unit);
    solver.iterating = true;
    crate::clause::mark_clause_as_garbage(solver, c_ref);
    debug_assert!(solver.level == 0);
    let conflict = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
    debug_assert!(conflict.is_none() || solver.inconsistent);
    solver.statistics.vivify_units += 1; // INC (vivify_units): STATISTIC
    let _ = conflict;
}

// static vivify_learn_binary
fn vivify_learn_binary(solver: &mut Solver, c_ref: Reference) {
    crate::backtrack::backtrack_without_updating_phases(solver, 0);
    debug_assert!(solver.clause.len() == 2);
    if solver.arena.clause(c_ref).redundant() {
        let _ = crate::clause::new_redundant_clause(solver, 1);
    } else {
        let _ = crate::clause::new_irredundant_clause(solver);
    }
    crate::clause::mark_clause_as_garbage(solver, c_ref);
}

// static swap_first_literal_with_best_watch, operating on the clause's
// literal array starting at `offset` (C: lits pointer / lits + 1).
fn swap_first_literal_with_best_watch(
    solver: &mut Solver,
    c_ref: Reference,
    offset: u32,
    size: u32,
) {
    debug_assert!(size != 0);
    let first = solver.arena.clause(c_ref).lit(offset);
    let mut best_pos: u32 = 0;
    let mut best = first;
    // PORT NOTE: C's loop condition reads this outer `value`, which the inner
    // declaration shadows and never updates — quirk ported (see header).
    let value = solver.values[best as usize];
    let mut best_level = solver.assigned[idx(best) as usize].level; // LEVEL (best)
    let mut p: u32 = 1;
    while value < 0 && p != size {
        let lit = solver.arena.clause(c_ref).lit(offset + p);
        let v = solver.values[lit as usize];
        if v < 0 {
            let level = solver.assigned[idx(lit) as usize].level;
            if level <= best_level {
                p += 1;
                continue;
            }
            best_level = level;
        }
        best_pos = p;
        best = lit;
        p += 1;
    }
    if best_pos == 0 {
        return;
    }
    let mut c = solver.arena.clause_mut(c_ref);
    c.set_lit(offset + best_pos, first);
    c.set_lit(offset, best);
}

// static vivify_unwatch_clause
fn vivify_unwatch_clause(solver: &mut Solver, c_ref: Reference) {
    let (l0, l1) = {
        let c = solver.arena.clause(c_ref);
        (c.lit(0), c.lit(1))
    };
    crate::watch::unwatch_blocking(solver, l0, c_ref);
    crate::watch::unwatch_blocking(solver, l1, c_ref);
}

// static vivify_watch_clause
fn vivify_watch_clause(solver: &mut Solver, c_ref: Reference) {
    let size = solver.arena.clause(c_ref).size();
    swap_first_literal_with_best_watch(solver, c_ref, 0, size);
    swap_first_literal_with_best_watch(solver, c_ref, 1, size - 1);
    let (l0, l1) = {
        let c = solver.arena.clause(c_ref);
        (c.lit(0), c.lit(1))
    };
    crate::watch::watch_blocking(solver, l0, l1, c_ref);
    crate::watch::watch_blocking(solver, l1, l0, c_ref);
}

// static vivify_learn_large
fn vivify_learn_large(solver: &mut Solver, c_ref: Reference, implied: u32) {
    debug_assert!(!solver.arena.clause(c_ref).garbage());

    // CHECK_AND_ADD_STACK: compiled out (NDEBUG).
    // ADD_STACK_TO_PROOF (solver->clause):
    if solver.proof.is_some() {
        let lits = std::mem::take(&mut solver.clause);
        crate::proof::add_lits_to_proof(solver, &lits);
        solver.clause = lits;
    }
    // REMOVE_CHECKER_CLAUSE: compiled out (NDEBUG).
    // DELETE_CLAUSE_FROM_PROOF (c):
    if solver.proof.is_some() {
        crate::proof::delete_clause_from_proof(solver, c_ref);
    }

    vivify_unwatch_clause(solver, c_ref);

    let irredundant = !solver.arena.clause(c_ref).redundant();

    if irredundant {
        // TODO comment in C: "this could be made more precise."
        let size = solver.arena.clause(c_ref).size();
        for i in 0..size {
            let lit = solver.arena.clause(c_ref).lit(i);
            crate::inline::mark_removed_literal(solver, lit);
        }
    }

    let old_size = solver.arena.clause(c_ref).size();
    let new_size = solver.clause.len() as u32;
    debug_assert!(new_size <= old_size);

    // memcpy (lits, BEGIN_STACK (*learned), new_size * sizeof *lits)
    for i in 0..new_size {
        let l = solver.clause[i as usize];
        solver.arena.clause_mut(c_ref).set_lit(i, l);
    }

    if irredundant {
        // PORT NOTE: c->size is still old_size here — the stale tail beyond
        // new_size is marked added too (C quirk, see header).
        for i in 0..old_size {
            let lit = solver.arena.clause(c_ref).lit(i);
            crate::inline::mark_added_literal(solver, lit);
        }
    }

    debug_assert!(new_size < old_size);
    if !solver.arena.clause(c_ref).shrunken() {
        let mut c = solver.arena.clause_mut(c_ref);
        c.set_shrunken(true);
        c.set_lit(old_size - 1, INVALID_LIT);
    }
    solver.arena.clause_mut(c_ref).set_size(new_size);
    let glue = solver.arena.clause(c_ref).glue();
    if !irredundant && glue >= new_size {
        crate::promote::promote_clause(solver, c_ref, new_size - 1);
    }
    solver.arena.clause_mut(c_ref).set_searched(2);

    if implied == INVALID_LIT {
        // vivified shrunken after conflict
        crate::backtrack::backtrack_without_updating_phases(solver, new_size - 2);
    } else {
        // vivified shrunken after implied
        debug_assert!(solver.level >= new_size - 1);
    }

    vivify_watch_clause(solver, c_ref);
}

// static vivify_learn
fn vivify_learn(solver: &mut Solver, c_ref: Reference, implied: u32) {
    let size = solver.clause.len();
    if size == 1 {
        vivify_learn_unit(solver, c_ref);
    } else if size == 2 {
        vivify_learn_binary(solver, c_ref);
    } else {
        vivify_learn_large(solver, c_ref, implied);
    }
}

// static binary_strengthen_after_instantiation
fn binary_strengthen_after_instantiation(solver: &mut Solver, c_ref: Reference, remove: u32) {
    debug_assert!(solver.level == 3);

    let mut first = INVALID_LIT;
    let mut second = INVALID_LIT;
    let size = solver.arena.clause(c_ref).size();
    for i in 0..size {
        let lit = solver.arena.clause(c_ref).lit(i);
        if lit != remove {
            debug_assert!(solver.values[lit as usize] < 0);
            if solver.assigned[idx(lit) as usize].level != 0 {
                if first == INVALID_LIT {
                    first = lit;
                } else {
                    debug_assert!(second == INVALID_LIT);
                    second = lit;
                }
            }
        }
    }
    debug_assert!(first != INVALID_LIT);
    debug_assert!(second != INVALID_LIT);
    solver.clause.clear();
    solver.clause.push(first);
    solver.clause.push(second);
    if solver.arena.clause(c_ref).redundant() {
        let _ = crate::clause::new_redundant_clause(solver, 1);
    } else {
        let _ = crate::clause::new_irredundant_clause(solver);
    }

    crate::clause::mark_clause_as_garbage(solver, c_ref);
    crate::backtrack::backtrack_without_updating_phases(solver, 0);
}

// static large_strengthen_after_instantiation
fn large_strengthen_after_instantiation(solver: &mut Solver, c_ref: Reference, remove: u32) {
    debug_assert!(solver.level > 3);

    // SHRINK_CLAUSE_IN_PROOF (c, remove, INVALID_LIT):
    if solver.proof.is_some() {
        crate::proof::shrink_clause_in_proof(solver, c_ref, remove, INVALID_LIT);
    }
    // CHECK_SHRINK_CLAUSE: compiled out (NDEBUG).

    vivify_unwatch_clause(solver, c_ref);

    let irredundant = !solver.arena.clause(c_ref).redundant();
    let old_size = solver.arena.clause(c_ref).size();
    debug_assert!(old_size > 3);
    let mut new_size: u32 = 0;
    for i in 0..old_size {
        let lit = solver.arena.clause(c_ref).lit(i);
        if lit == remove {
            if irredundant {
                crate::inline::mark_removed_literal(solver, lit);
            }
        } else if crate::internal::fixed(solver, lit) >= 0 {
            solver.arena.clause_mut(c_ref).set_lit(new_size, lit);
            new_size += 1;
            if irredundant {
                crate::inline::mark_added_literal(solver, lit);
            }
        }
    }
    debug_assert!(new_size > 2);
    debug_assert!(new_size < old_size);
    if !solver.arena.clause(c_ref).shrunken() {
        let mut c = solver.arena.clause_mut(c_ref);
        c.set_shrunken(true);
        c.set_lit(old_size - 1, INVALID_LIT);
    }
    solver.arena.clause_mut(c_ref).set_size(new_size);
    let glue = solver.arena.clause(c_ref).glue();
    if !irredundant && glue >= new_size {
        crate::promote::promote_clause(solver, c_ref, new_size - 1);
    }
    solver.arena.clause_mut(c_ref).set_searched(2);

    crate::backtrack::backtrack_without_updating_phases(solver, solver.level - 2);
    vivify_watch_clause(solver, c_ref);
}

// static vivify_strengthen_after_instantiation
fn vivify_strengthen_after_instantiation(solver: &mut Solver, c_ref: Reference, remove: u32) {
    debug_assert!(solver.level >= 3);
    debug_assert!(solver.values[remove as usize] > 0);
    debug_assert!(solver.assigned[idx(remove) as usize].level == solver.level);

    if solver.level == 3 {
        binary_strengthen_after_instantiation(solver, c_ref, remove);
    } else {
        large_strengthen_after_instantiation(solver, c_ref, remove);
    }
}

// static vivify_mark_sorted_literals
fn vivify_mark_sorted_literals(solver: &mut Solver, sorted: &[u32]) {
    for &lit in sorted.iter() {
        debug_assert!(solver.marks[lit as usize] == 0);
        solver.marks[lit as usize] = 1;
        solver.marks[not(lit) as usize] = -1;
    }
}

// static vivify_unmark_sorted_literals
fn vivify_unmark_sorted_literals(solver: &mut Solver, sorted: &[u32]) {
    for &lit in sorted.iter() {
        debug_assert!(solver.marks[lit as usize] > 0);
        debug_assert!(solver.marks[not(lit) as usize] < 0);
        solver.marks[lit as usize] = 0;
        solver.marks[not(lit) as usize] = 0;
    }
}

// static reestablish_watch_invariant_for_candidate
fn reestablish_watch_invariant_for_candidate(solver: &mut Solver, cand_ref: Reference) {
    if solver.level == 0 {
        return;
    }
    if solver.arena.clause(cand_ref).garbage() {
        return;
    }
    let (first, second) = {
        let c = solver.arena.clause(cand_ref);
        (c.lit(0), c.lit(1))
    };
    let first_val = solver.values[first as usize];
    let second_val = solver.values[second as usize];
    let first_level = if first_val != 0 {
        solver.assigned[idx(first) as usize].level
    } else {
        0
    };
    let second_level = if second_val != 0 {
        solver.assigned[idx(second) as usize].level
    } else {
        0
    };
    let new_level;
    if first_val >= 0 && second_val >= 0 {
        return;
    }
    if first_val < 0 && second_val == 0 {
        new_level = first_level;
    } else if first_val < 0 && second_val > 0 {
        if first_level >= second_level {
            return;
        }
        new_level = first_level;
    } else if second_val < 0 && first_val == 0 {
        new_level = second_level;
    } else if second_val < 0 && first_val > 0 {
        if second_level >= first_level {
            return;
        }
        new_level = second_level;
    } else {
        debug_assert!(first_val < 0 && second_val < 0);
        new_level = std::cmp::min(first_level, second_level);
    }
    debug_assert!(new_level != 0);
    crate::backtrack::backtrack_without_updating_phases(solver, new_level - 1);
}

// static vivify_clause
#[allow(unused_assignments)] // `conflict = None` mirrors C's dead `conflict = 0;`
fn vivify_clause(solver: &mut Solver, vivifier: &mut Vivifier, cand_ref: Reference) -> bool {
    debug_assert!(!solver.arena.clause(cand_ref).garbage());
    debug_assert!(solver.probing);
    debug_assert!(solver.watching);
    debug_assert!(!solver.inconsistent);

    vivifier.sorted.clear();

    {
        let size = solver.arena.clause(cand_ref).size();
        for i in 0..size {
            let lit = solver.arena.clause(cand_ref).lit(i);
            let value = crate::internal::fixed(solver, lit);
            if value < 0 {
                continue;
            }
            if value > 0 {
                crate::clause::mark_clause_as_garbage(solver, cand_ref);
                return false;
            }
            vivifier.sorted.push(lit);
        }
    }

    debug_assert!(!solver.arena.clause(cand_ref).garbage());

    let non_false = vivifier.sorted.len() as u32;

    debug_assert!(non_false > 1);
    debug_assert!(non_false <= solver.arena.clause(cand_ref).size());

    if non_false == 2 {
        return false; // skipping actually binary
    }

    solver.statistics.vivify_checks += 1; // INC (vivify_checks)

    let mut unit = INVALID_LIT;
    {
        let size = solver.arena.clause(cand_ref).size();
        for i in 0..size {
            let lit = solver.arena.clause(cand_ref).lit(i);
            let value = solver.values[lit as usize];
            if value < 0 {
                continue;
            }
            if value == 0 {
                unit = INVALID_LIT;
                break;
            }
            debug_assert!(value > 0);
            if unit != INVALID_LIT {
                unit = INVALID_LIT;
                break;
            }
            unit = lit;
        }
    }
    if unit != INVALID_LIT {
        let a = solver.assigned[idx(unit) as usize];
        debug_assert!(a.level != 0);
        if a.binary {
            unit = INVALID_LIT;
        } else if a.reason != cand_ref {
            unit = INVALID_LIT;
        }
    }
    if unit != INVALID_LIT {
        // candidate is the reason of `unit`: forced to backtrack
        let level = solver.assigned[idx(unit) as usize].level;
        debug_assert!(level > 0);
        crate::backtrack::backtrack_without_updating_phases(solver, level - 1);
    }

    debug_assert!(solver.analyzed.is_empty());
    debug_assert!(solver.clause.is_empty());

    // vivify_sort_stack_by_counts (solver, sorted, counts)
    vivify_sort_lits_by_counts(solver, &mut vivifier.sorted, &vivifier.counts);

    vivify_mark_sorted_literals(solver, &vivifier.sorted);

    let mut implied = INVALID_LIT;
    let mut conflict: Option<Conflict> = None;
    let mut level: u32 = 0;

    'assumptions: for si in 0..vivifier.sorted.len() {
        let lit = vivifier.sorted[si];
        let old_level = level; // C: if (level++ < solver->level)
        level += 1;
        if old_level < solver.level {
            let frame_decision = solver.frames[level as usize].decision; // FRAME (level)
            let not_lit = not(lit);
            if frame_decision == not_lit {
                // reusing assumption
                solver.statistics.vivify_reused += 1; // INC (vivify_reused)
                solver.statistics.vivify_probes += 1; // INC (vivify_probes)
                debug_assert!(solver.values[lit as usize] < 0);
                continue;
            }

            // forced to backtrack to decision level `level - 1`
            crate::backtrack::backtrack_without_updating_phases(solver, level - 1);
        }

        let value = solver.values[lit as usize];
        debug_assert!(value == 0 || solver.assigned[idx(lit) as usize].level <= level);

        if value < 0 {
            // literal already falsified
            continue;
        }

        if value > 0 {
            // literal already initially satisfied
            implied = lit;
            break;
        }

        debug_assert!(value == 0);

        let not_lit = not(lit);
        solver.statistics.vivify_probes += 1; // INC (vivify_probes)

        crate::decide::internal_assume(solver, not_lit);
        debug_assert!(solver.level >= 1);

        let p = solver.propagate;
        conflict = crate::proprobe::probing_propagate(solver, cand_ref, true);
        if conflict.is_some() {
            break;
        }

        let end = solver.trail.len();
        let mut q = p;
        while q != end {
            let other = solver.trail[q];
            q += 1;
            let mark = solver.marks[other as usize];
            if mark > 0 {
                // literal already implied satisfied
                implied = other;
                break 'assumptions; // goto EXIT_ASSUMPTION_LOOP
            }
        }
    }
    // EXIT_ASSUMPTION_LOOP:

    debug_assert!(conflict.is_none() || implied == INVALID_LIT);

    if implied != INVALID_LIT {
        let mut better_implied = INVALID_LIT;
        let mut better_implied_trail = INVALID_TRAIL;
        for si in 0..vivifier.sorted.len() {
            let lit = vivifier.sorted[si];
            if solver.values[lit as usize] <= 0 {
                continue;
            }
            let lit_trail = solver.assigned[idx(lit) as usize].trail; // TRAIL (lit)
            if lit_trail > better_implied_trail {
                continue;
            }
            better_implied_trail = lit_trail;
            better_implied = lit;
        }
        if better_implied != implied {
            implied = better_implied;
        }
    }

    let level_after_assumptions = solver.level;
    debug_assert!(level_after_assumptions != 0);

    let (subsuming, redundant) = vivify_deduce(solver, cand_ref, conflict, implied);

    vivify_unmark_sorted_literals(solver, &vivifier.sorted);

    let res;

    if let Some(subsuming) = subsuming {
        debug_assert!(!matches!(subsuming, Conflict::Clause(r) if solver.arena.clause(r).garbage()));
        let subsuming_redundant = conflict_redundant(solver, subsuming);
        let (cand_redundant, cand_glue) = {
            let c = solver.arena.clause(cand_ref);
            (c.redundant(), c.glue())
        };
        if cand_redundant {
            crate::clause::mark_clause_as_garbage(solver, cand_ref);
            if subsuming_redundant && cand_glue < conflict_glue(solver, subsuming) {
                // vivify candidate with smaller glue than subsuming clause
                let sub_ref = match subsuming {
                    Conflict::Clause(r) => r,
                    Conflict::Binary => unreachable!("fake binary conflict is never redundant"),
                };
                crate::promote::promote_clause(solver, sub_ref, cand_glue);
            }
            solver.statistics.vivified_subred += 1; // INC: STATISTIC
            solver.statistics.vivified_subsumed += 1; // INC: STATISTIC
            res = true;
        } else if !subsuming_redundant {
            debug_assert!(!cand_redundant);
            crate::clause::mark_clause_as_garbage(solver, cand_ref);
            solver.statistics.vivified_subirr += 1; // INC: STATISTIC
            solver.statistics.vivified_subsumed += 1; // INC: STATISTIC
            res = true;
        } else {
            debug_assert!(!cand_redundant);
            crate::clause::mark_clause_as_garbage(solver, cand_ref);
            let sub_ref = match subsuming {
                Conflict::Clause(r) => r,
                Conflict::Binary => unreachable!("fake binary conflict is never redundant"),
            };
            solver.arena.clause_mut(sub_ref).set_redundant(false);
            debug_assert!(solver.statistics.clauses_redundant > 0);
            solver.statistics.clauses_redundant -= 1;
            debug_assert!(solver.statistics.clauses_irredundant < u64::MAX);
            solver.statistics.clauses_irredundant += 1;
            solver.statistics.vivified_promoted += 1; // INC: STATISTIC
            // vivification promoted from redundant to irredundant
            crate::collect::update_last_irredundant(solver, sub_ref);
            // kissat_mark_added_literals (solver, subsuming->size, subsuming->lits)
            let lits: Vec<u32> = solver.arena.clause(sub_ref).lits().to_vec();
            crate::flags::mark_added_literals(solver, lits.len() as u32, &lits);
            solver.statistics.vivified_subirr += 1; // INC: STATISTIC
            solver.statistics.vivified_subsumed += 1; // INC: STATISTIC
            res = true;
        }
    } else if vivify_shrinkable(solver, &vivifier.sorted, conflict.is_some()) {
        vivify_learn(solver, cand_ref, implied);
        solver.statistics.vivified_shrunken += 1; // INC: STATISTIC
        if solver.arena.clause(cand_ref).redundant() {
            solver.statistics.vivified_shrunkred += 1; // INC: STATISTIC
        } else {
            solver.statistics.vivified_shrunkirr += 1; // INC: STATISTIC
        }
        res = true;
    } else if implied != INVALID_LIT && solver.arena.clause(cand_ref).redundant() {
        crate::clause::mark_clause_as_garbage(solver, cand_ref);
        solver.statistics.vivified_implied += 1; // INC: STATISTIC
        res = true;
    } else if (conflict.is_some() || implied != INVALID_LIT)
        && !solver.arena.clause(cand_ref).redundant()
        && !redundant
    {
        // vivification asymmetric tautology
        crate::clause::mark_clause_as_garbage(solver, cand_ref);
        solver.statistics.vivified_asym += 1; // INC: STATISTIC
        res = true;
    } else if implied != INVALID_LIT {
        // no vivification instantiation with implied literal
        debug_assert!(!solver.arena.clause(cand_ref).redundant());
        debug_assert!(redundant);
        res = false;
    } else {
        debug_assert!(solver.level > 2);
        debug_assert!(solver.level as usize == vivifier.sorted.len());
        let lit = *vivifier.sorted.last().unwrap(); // TOP_STACK (*sorted)
        debug_assert!(solver.values[lit as usize] < 0);
        debug_assert!(solver.assigned[idx(lit) as usize].level == solver.level);
        crate::backtrack::backtrack_without_updating_phases(solver, solver.level - 1);
        conflict = None;
        debug_assert!(solver.values[lit as usize] == 0);
        crate::decide::internal_assume(solver, lit);
        conflict = crate::proprobe::probing_propagate(solver, cand_ref, true);
        if conflict.is_some() {
            // vivification instantiation succeeded
            vivify_strengthen_after_instantiation(solver, cand_ref, lit);
            solver.statistics.vivified_instantiated += 1; // INC: STATISTIC
            if solver.arena.clause(cand_ref).redundant() {
                solver.statistics.vivified_instred += 1; // INC: STATISTIC
            } else {
                solver.statistics.vivified_instirr += 1; // INC: STATISTIC
            }
            res = true;
        } else {
            // vivification instantiation failed
            crate::backtrack::backtrack_without_updating_phases(solver, solver.level - 2);
            res = false;
        }
    }

    reset_vivify_analyzed(solver);
    if conflict.is_some() && solver.level == level_after_assumptions {
        // forcing backtracking at least one level after conflict
        crate::backtrack::backtrack_without_updating_phases(solver, solver.level - 1);
    }
    reestablish_watch_invariant_for_candidate(solver, cand_ref);

    if res {
        solver.statistics.vivified += 1; // INC (vivified)
        match vivifier.tier {
            1 => solver.statistics.vivified_tier1 += 1, // STATISTIC
            2 => solver.statistics.vivified_tier2 += 1, // STATISTIC
            3 => solver.statistics.vivified_tier3 += 1, // STATISTIC
            _ => {
                debug_assert!(vivifier.tier == 0);
                solver.statistics.vivified_irredundant += 1; // STATISTIC
            }
        }
    }

    res
}

// static vivify_round
fn vivify_round(solver: &mut Solver, vivifier: &mut Vivifier, limit: u64) {
    let tier = vivifier.tier;

    if tier != 0 && solver.statistics.clauses_redundant == 0 {
        return; // REDUNDANT_CLAUSES == 0
    }

    debug_assert!((0..=3).contains(&tier));
    debug_assert!(solver.watching);
    debug_assert!(solver.probing);

    crate::watch::flush_large_watches(solver);

    schedule_vivification_candidates(solver, vivifier);
    if solver.options.vivifysort != 0 {
        if tier != 0
            || solver.statistics.clauses_irredundant / 10 <= solver.statistics.clauses_redundant
        {
            sort_vivification_candidates(solver, vivifier);
        } else {
            crate::print::extremely_verbose(
                solver,
                format!("not sorting {} vivification candidates", vivifier.name),
            );
        }
    }

    crate::watch::watch_large_clauses(solver);

    // #ifndef QUIET
    let start = solver.statistics.probing_ticks;
    let delta = limit.wrapping_sub(start);
    crate::print::very_verbose(
        solver,
        format!(
            "vivification {} effort limit {} = {} + {} 'probing_ticks'",
            vivifier.name, limit, start, delta
        ),
    );
    let total = if tier != 0 {
        solver.statistics.clauses_redundant
    } else {
        solver.statistics.clauses_irredundant
    };
    let scheduled = vivifier.schedule.len();
    crate::print::phase(
        solver,
        vivifier.mode,
        solver.statistics.vivifications, // GET (vivifications)
        format!(
            "scheduled {} clauses {:.0}% of {}",
            scheduled,
            percent(scheduled as f64, total as f64),
            total
        ),
    );

    debug_assert!(vivifier.vivified == 0);
    debug_assert!(vivifier.tried == 0);
    while !vivifier.schedule.is_empty() {
        let probing_ticks = solver.statistics.probing_ticks;
        if probing_ticks > limit {
            crate::print::extremely_verbose(
                solver,
                format!(
                    "vivification {} ticks limit {} hit after {} 'probing_ticks'",
                    vivifier.name, limit, probing_ticks
                ),
            );
            break;
        }
        if terminated!(solver, vivify_terminated_1) {
            break;
        }
        let ref_ = vivifier.schedule.pop().unwrap(); // POP_STACK
        debug_assert!(!solver.arena.clause(ref_).garbage());
        vivifier.tried += 1;
        if vivify_clause(solver, vivifier, ref_) {
            vivifier.vivified += 1;
        }
        if solver.inconsistent {
            break;
        }
        solver.arena.clause_mut(ref_).set_vivify(false);
    }
    if solver.level != 0 {
        crate::backtrack::backtrack_without_updating_phases(solver, 0);
    }
    // #ifndef QUIET
    crate::print::phase(
        solver,
        vivifier.mode,
        solver.statistics.vivifications,
        format!(
            "vivified {} clauses {:.0}% out of {} tried",
            vivifier.vivified,
            percent(vivifier.vivified as f64, vivifier.tried as f64),
            vivifier.tried
        ),
    );
    if !solver.inconsistent {
        let remain = vivifier.schedule.len();
        if remain != 0 {
            crate::print::phase(
                solver,
                vivifier.mode,
                solver.statistics.vivifications,
                format!(
                    "{} clauses remain {:.0}% out of {} scheduled",
                    remain,
                    percent(remain as f64, scheduled as f64),
                    scheduled
                ),
            );

            let mut prioritized: usize = 0;
            while let Some(ref_) = vivifier.schedule.pop() {
                if solver.arena.clause(ref_).vivify() {
                    prioritized += 1;
                }
            }
            if prioritized == 0 {
                crate::print::phase(
                    solver,
                    vivifier.mode,
                    solver.statistics.vivifications,
                    "no remaining prioritized clauses",
                );
            } else {
                crate::print::phase(
                    solver,
                    vivifier.mode,
                    solver.statistics.vivifications,
                    format!(
                        "keeping all {} remaining clauses prioritized {:.0}%",
                        prioritized,
                        percent(prioritized as f64, remain as f64)
                    ),
                );
            }
        } else {
            crate::print::phase(
                solver,
                vivifier.mode,
                solver.statistics.vivifications,
                "no untried clauses remain",
            );
        }
    }
    crate::report::report(solver, vivifier.vivified == 0, vivifier.tag); // REPORT
}

// static vivify_tier1
fn vivify_tier1(solver: &mut Solver, vivifier: &mut Vivifier, limit: u64) {
    crate::profile::start_checked(solver, Prof::vivify1); // START (vivify1)
    set_vivifier_mode(vivifier, 1);
    vivify_round(solver, vivifier, limit);
    crate::profile::stop_checked(solver, Prof::vivify1); // STOP (vivify1)
}

// static vivify_tier2
fn vivify_tier2(solver: &mut Solver, vivifier: &mut Vivifier, limit: u64) {
    crate::profile::start_checked(solver, Prof::vivify2); // START (vivify2)
    clear_vivifier(vivifier);
    set_vivifier_mode(vivifier, 2);
    vivify_round(solver, vivifier, limit);
    crate::profile::stop_checked(solver, Prof::vivify2); // STOP (vivify2)
}

// static vivify_tier3
fn vivify_tier3(solver: &mut Solver, vivifier: &mut Vivifier, limit: u64) {
    crate::profile::start_checked(solver, Prof::vivify3); // START (vivify3)
    clear_vivifier(vivifier);
    set_vivifier_mode(vivifier, 3);
    vivify_round(solver, vivifier, limit);
    crate::profile::stop_checked(solver, Prof::vivify3); // STOP (vivify3)
}

// static vivify_irredundant
fn vivify_irredundant(solver: &mut Solver, vivifier: &mut Vivifier, limit: u64) {
    crate::profile::start_checked(solver, Prof::vivify0); // START (vivify0)
    clear_vivifier(vivifier);
    set_vivifier_mode(vivifier, 0);
    vivify_round(solver, vivifier, limit);
    crate::profile::stop_checked(solver, Prof::vivify0); // STOP (vivify0)
}

/// Port of `kissat_vivify`.
pub fn vivify(solver: &mut Solver) {
    if solver.inconsistent {
        return;
    }
    debug_assert!(solver.level == 0);
    debug_assert!(solver.probing);
    debug_assert!(solver.watching);
    if solver.options.vivify == 0 {
        return;
    }
    if terminated!(solver, vivify_terminated_2) {
        return;
    }
    let irr_budget: f64 = if crate::kimits::delaying(solver, DelayId::Vivifyirr) {
        0.0
    } else {
        solver.options.vivifyirr as f64
    };
    let mut tier1_budget: f64 = solver.options.vivifytier1 as f64;
    let mut tier2_budget: f64 = solver.options.vivifytier2 as f64;
    let tier3_budget: f64 = if solver.statistics.clauses_redundant == 0 {
        tier1_budget = 0.0;
        tier2_budget = 0.0;
        0.0
    } else {
        solver.options.vivifytier3 as f64 // tier3_buget in C
    };
    let sum = irr_budget + tier1_budget + tier2_budget + tier3_budget;
    if sum == 0.0 {
        return;
    }

    crate::profile::start_checked(solver, Prof::vivify); // START (vivify)
    solver.statistics.vivifications += 1; // INC (vivifications)

    let mut limit = crate::set_effort_limit!(solver, vivify, vivifyeffort, probing_ticks);
    let total = limit - solver.statistics.probing_ticks;
    limit = solver.statistics.probing_ticks;
    let tier1_limit = vivify_tier1_limit(solver);
    let tier2_limit = vivify_tier2_limit(solver);
    let mut tier1_budget = tier1_budget;
    let mut tier2_budget = tier2_budget;
    if tier1_budget != 0.0 && tier2_budget != 0.0 && tier1_limit == tier2_limit {
        // vivification tier1 matches tier2: use tier2 budget for tier1
        tier1_budget += tier2_budget;
        tier2_budget = 0.0;
    }

    {
        let mut vivifier = init_vivifier(solver);
        if tier1_budget != 0.0 {
            // limit += (total * tier1_budget) / sum  (double arithmetic)
            limit = (limit as f64 + (total as f64 * tier1_budget) / sum) as u64;
            vivify_tier1(solver, &mut vivifier, limit);
        }
        if tier2_budget != 0.0
            && !solver.inconsistent
            && !terminated!(solver, vivify_terminated_3)
        {
            limit = (limit as f64 + (total as f64 * tier2_budget) / sum) as u64;
            vivify_tier2(solver, &mut vivifier, limit);
        }
        if tier3_budget != 0.0
            && !solver.inconsistent
            && !terminated!(solver, vivify_terminated_4)
        {
            limit = (limit as f64 + (total as f64 * tier3_budget) / sum) as u64;
            vivify_tier3(solver, &mut vivifier, limit);
        }
        if irr_budget != 0.0 && !solver.inconsistent && !terminated!(solver, vivify_terminated_5)
        {
            limit = (limit as f64 + (total as f64 * irr_budget) / sum) as u64;
            vivify_irredundant(solver, &mut vivifier, limit);
            if crate::utilities::average(vivifier.vivified as f64, vivifier.tried as f64) < 0.01 {
                crate::kimits::bump_delay(solver, DelayId::Vivifyirr); // BUMP_DELAY
            } else {
                crate::kimits::reduce_delay(solver, DelayId::Vivifyirr); // REDUCE_DELAY
            }
        }
        drop(vivifier); // release_vivifier
    }

    // #ifndef QUIET
    if solver.statistics.probing_ticks < limit {
        let delta = limit - solver.statistics.probing_ticks;
        crate::print::phase(
            solver,
            "vivify-limit",
            solver.statistics.vivifications, // GET (vivifications)
            format!(
                "has {} ticks left {:.2}%",
                delta,
                percent(delta as f64, total as f64)
            ),
        );
    } else {
        let delta = solver.statistics.probing_ticks - limit;
        crate::print::phase(
            solver,
            "vivify-limit",
            solver.statistics.vivifications,
            format!(
                "exceeded by {} ticks {:.2}%",
                delta,
                percent(delta as f64, total as f64)
            ),
        );
    }
    crate::profile::stop_checked(solver, Prof::vivify); // STOP (vivify)
}
