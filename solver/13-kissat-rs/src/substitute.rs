// Port of src/substitute.c (kissat 4.0.4).
//
// Equivalent-literal substitution (SCC decomposition of the binary
// implication graph), run during probing (solver->watching, solver->probing).
//
// PORT NOTE: CHECKING_OR_PROVING is defined in the reference build (NPROOFS
// not defined), so the delayed proof-binary blocks compile; the ADD_/DELETE_
// macros check `solver->proof` internally → `if solver.proof.is_some()`.
// PORT NOTE: C reuses `solver->delayed` (unsigneds) as a `statches *` for the
// delayed substituted watches; Watch is a u32 word here, so the port pushes
// the watch words into solver.delayed directly.
// PORT NOTE: C mallocs `mark`/`reach` (reach uninitialized); the port zeroes
// both — every reach slot is written before read, so this is unobservable.
// PORT NOTE: GET (substitutions) is STATISTIC → phase prints no count
// (u64::MAX); INC (substitutions) / INC (substitute_units) are STATISTIC
// (kept as real, never-printed fields); substituted / substitute_ticks are
// COUNTERs (real).  kissat_check_statistics is NDEBUG-only (omitted).

use crate::internal::{Solver, INVALID};
use crate::profile::Prof;
use crate::reference::{Reference, INVALID_REF};
use crate::terminated;
use crate::watch::{binary_watch, watch_is_binary, watch_lit, LitWatch};

// static void assign_and_propagate_units (kissat *, unsigneds *units)
fn assign_and_propagate_units(solver: &mut Solver, units: &mut Vec<u32>) {
    if units.is_empty() {
        return;
    }
    while !solver.inconsistent && !units.is_empty() {
        let unit = units.pop().unwrap(); // POP_STACK
        let value = solver.values[unit as usize];
        if value > 0 {
            // skipping satisfied unit
        } else if value < 0 {
            // inconsistent unit
            // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
            if solver.proof.is_some() {
                crate::proof::add_empty_to_proof(solver);
            }
            solver.inconsistent = true;
        } else {
            crate::assign::learned_unit(solver, unit);
            solver.statistics.substitute_units += 1; // INC: STATISTIC kept
            debug_assert!(solver.level == 0);
            let _ = crate::proprobe::probing_propagate(solver, INVALID_REF, false);
        }
    }
}

// static void determine_representatives (kissat *, unsigned *repr)
fn determine_representatives(solver: &mut Solver, repr: &mut [u32]) {
    let lits = solver.lits() as usize;
    let mut mark: Vec<u32> = vec![0; lits]; // kissat_calloc
    let mut reach: Vec<u32> = vec![0; lits]; // kissat_malloc (see PORT NOTE)
    let mut reached: u32 = 0;
    let mut scc: Vec<u32> = Vec::new();
    let mut work: Vec<u32> = Vec::new();
    let mut units: Vec<u32> = Vec::new();
    let mut inconsistent = false;
    let mut ticks: u64 = 0;
    for root in 0..lits as u32 {
        if inconsistent {
            break;
        }
        if mark[root as usize] != 0 {
            continue;
        }
        // if (!ACTIVE (IDX (root))) continue;
        if !solver.flags[crate::literal::idx(root) as usize].active {
            continue;
        }
        debug_assert!(scc.is_empty());
        debug_assert!(work.is_empty());
        work.push(root);
        let mut failed = false;
        let mark_root = reached + 1;
        while !inconsistent && !work.is_empty() {
            let mut lit = *work.last().unwrap(); // TOP_STACK
            if lit == INVALID {
                work.pop().unwrap();
                lit = work.pop().unwrap();
                let not_lit = crate::literal::not(lit);
                let mut reach_lit = reach[lit as usize];
                let mark_lit = mark[lit as usize];
                debug_assert!(reach_lit == mark_lit);
                debug_assert!(repr[lit as usize] == INVALID);
                let v = solver.watches[not_lit as usize];
                let size_watches = v.size();
                ticks += 1 + crate::utilities::cache_lines(size_watches as u64, 4);
                // for (all_binary_blocking_watches (watch, *watches))
                let mut p = v.begin;
                while p != v.end {
                    let watch = solver.vectors.stack[p];
                    p += if watch_is_binary(watch) { 1 } else { 2 };
                    if !watch_is_binary(watch) {
                        continue;
                    }
                    let other = watch_lit(watch);
                    let idx_other = crate::literal::idx(other);
                    if !solver.flags[idx_other as usize].active {
                        continue;
                    }
                    debug_assert!(mark[other as usize] != 0);
                    let reach_other = reach[other as usize];
                    if reach_other < reach_lit {
                        reach_lit = reach_other;
                    }
                }
                if reach_lit != mark_lit {
                    reach[lit as usize] = reach_lit;
                    continue;
                }
                // pop the SCC ending at `lit` from `scc`
                let mut begin_scc = scc.len();
                loop {
                    debug_assert!(begin_scc != 0);
                    begin_scc -= 1;
                    if scc[begin_scc] == lit {
                        break;
                    }
                }
                let end_scc = scc.len();
                let size_scc = end_scc - begin_scc;
                let mut min_lit = lit;
                if size_scc > 1 {
                    for &other in &scc[begin_scc..end_scc] {
                        if other < min_lit {
                            min_lit = other;
                        }
                    }
                } else {
                    debug_assert!(min_lit == lit);
                }
                for i in begin_scc..end_scc {
                    let other = scc[i];
                    repr[other as usize] = min_lit;
                    reach[other as usize] = u32::MAX;

                    let not_other = crate::literal::not(other);
                    let repr_not_other = repr[not_other as usize];
                    if repr_not_other == INVALID {
                        continue;
                    }
                    if min_lit == repr_not_other {
                        // clashing literals in same SCC
                        units.push(min_lit);
                        inconsistent = true;
                        break;
                    }
                    debug_assert!(crate::literal::not(min_lit) == repr_not_other);
                    if failed {
                        continue;
                    }
                    let mark_not_other = mark[not_other as usize];
                    debug_assert!(mark_not_other != INVALID);
                    debug_assert!(mark[root as usize] == mark_root);
                    if mark_root > mark_not_other {
                        continue;
                    }
                    // root implies both other and not_other
                    let unit = crate::literal::not(root);
                    units.push(unit);
                    failed = true;
                }
                scc.truncate(begin_scc); // SET_END_OF_STACK (scc, begin_scc)
                if inconsistent {
                    break;
                }
            } else if mark[lit as usize] == 0 {
                work.push(INVALID);
                scc.push(lit);
                reached += 1;
                mark[lit as usize] = reached;
                reach[lit as usize] = reached;
                let not_lit = crate::literal::not(lit);
                let v = solver.watches[not_lit as usize];
                let size_watches = v.size();
                ticks += 1 + crate::utilities::cache_lines(size_watches as u64, 4);
                let mut p = v.begin;
                while p != v.end {
                    let watch = solver.vectors.stack[p];
                    p += if watch_is_binary(watch) { 1 } else { 2 };
                    if !watch_is_binary(watch) {
                        continue;
                    }
                    let other = watch_lit(watch);
                    let idx_other = crate::literal::idx(other);
                    if !solver.flags[idx_other as usize].active {
                        continue;
                    }
                    if mark[other as usize] == 0 {
                        work.push(other);
                    }
                }
            } else {
                work.pop().unwrap();
            }
        }
    }
    drop(work); // RELEASE_STACK
    drop(scc); // RELEASE_STACK
    crate::print::extremely_verbose(
        solver,
        format_args!(
            "determining substitution representatives took {} 'substitute_ticks'",
            ticks
        ),
    );
    solver.statistics.substitute_ticks += ticks; // ADD (substitute_ticks, ticks)
    assign_and_propagate_units(solver, &mut units);
    debug_assert!(!inconsistent || solver.inconsistent);
    drop(units); // RELEASE_STACK
    drop(reach); // kissat_free
    drop(mark); // kissat_free
    for lit in 0..lits {
        if repr[lit] == INVALID {
            repr[lit] = lit as u32;
        }
    }
}

// static bool *add_representative_equivalences (kissat *, unsigned *repr)
fn add_representative_equivalences(solver: &mut Solver, repr: &[u32]) -> Option<Vec<bool>> {
    if solver.inconsistent {
        return None; // C returns NULL
    }
    let mut eliminate = vec![false; solver.vars as usize]; // kissat_calloc
    for idx in 0..solver.vars {
        if !solver.flags[idx as usize].active {
            continue;
        }
        let lit = crate::literal::lit(idx);
        let other = repr[lit as usize];
        if lit == other {
            continue;
        }
        debug_assert!(other < lit);
        // #ifdef CHECKING_OR_PROVING — defined (NPROOFS off):
        let not_lit = crate::literal::not(lit);
        let not_other = crate::literal::not(other);

        // CHECK_AND_ADD_BINARY: compiled out (NDEBUG).
        if solver.proof.is_some() {
            crate::proof::add_binary_to_proof(solver, not_lit, other);
        }
        if solver.proof.is_some() {
            crate::proof::add_binary_to_proof(solver, lit, not_other);
        }
        eliminate[idx as usize] = true;
    }
    Some(eliminate)
}

// static void remove_representative_equivalences (kissat *, unsigned *repr,
//                                                 bool *eliminate)
fn remove_representative_equivalences(
    solver: &mut Solver,
    repr: &[u32],
    eliminate: Option<Vec<bool>>,
) {
    if !solver.inconsistent {
        let eliminate = eliminate.as_ref().unwrap();
        let incremental = solver.options.incremental != 0;
        for idx in 0..solver.vars {
            if !eliminate[idx as usize] {
                continue;
            }

            debug_assert!(solver.flags[idx as usize].active);

            let lit = crate::literal::lit(idx);
            let other = repr[lit as usize];
            let not_lit = crate::literal::not(lit);
            let not_other = crate::literal::not(other);
            debug_assert!(other < lit);
            debug_assert!(not_other < not_lit);

            // REMOVE_CHECKER_BINARY: compiled out (NDEBUG).
            if solver.proof.is_some() {
                crate::proof::delete_binary_from_proof(solver, not_lit, other);
            }
            if solver.proof.is_some() {
                crate::proof::delete_binary_from_proof(solver, lit, not_other);
            }

            solver.statistics.substituted += 1; // INC (substituted)
            crate::flags::mark_eliminated_variable(solver, idx);
            let other_value = solver.values[other as usize];
            if incremental || other_value != 0 {
                if other_value <= 0 {
                    crate::weaken::weaken_binary(solver, not_lit, other);
                }
                if other_value >= 0 {
                    crate::weaken::weaken_binary(solver, lit, not_other);
                }
            } else {
                crate::weaken::weaken_binary(solver, not_lit, other);
                crate::weaken::weaken_unit(solver, lit);
            }
        }
    }
    // if (eliminate) kissat_dealloc (...): Drop.
}

// static void substitute_binaries (kissat *, unsigned *repr)
fn substitute_binaries(solver: &mut Solver, repr: &[u32]) {
    if solver.inconsistent {
        return;
    }
    // statches *delayed_watched = (statches *) &solver->delayed;
    let mut units: Vec<u32> = Vec::new();
    let mut delayed_deleted: Vec<LitWatch> = Vec::new();
    // #ifdef CHECKING_OR_PROVING — defined:
    let mut delayed_removed: Vec<[u32; 2]> = Vec::new(); // litpairs
    let lits = solver.lits();
    for lit in 0..lits {
        let repr_lit = repr[lit as usize];
        let not_repr_lit = crate::literal::not(repr_lit);
        debug_assert!(solver.delayed.is_empty());
        let v = solver.watches[lit as usize];
        let begin = v.begin;
        let end = v.end;
        let mut q = begin;
        let mut p = begin;
        while p != end {
            let src = solver.vectors.stack[p];
            p += 1;
            if !watch_is_binary(src) {
                continue;
            }
            let other = watch_lit(src);
            let repr_other = repr[other as usize];
            let litwatch = LitWatch { lit, watch: src };
            if repr_other == not_repr_lit {
                // becomes tautological
                if lit < other {
                    delayed_deleted.push(litwatch);
                }
            } else if repr_other == repr_lit {
                let unit = repr_lit;
                if lit < other {
                    units.push(unit);
                    delayed_deleted.push(litwatch);
                }
            } else {
                let dst = binary_watch(repr_other); // dst.binary.lit = repr_other
                if lit == repr_lit && other == repr_other {
                    // unchanged
                    solver.vectors.stack[q] = dst;
                    q += 1;
                } else {
                    if lit == repr_lit {
                        // substituted in place
                        solver.vectors.stack[q] = dst;
                        q += 1;
                    } else {
                        // delayed substituted
                        solver.delayed.push(dst); // PUSH_STACK (*delayed_watched, dst)
                    }

                    if lit < other {
                        // #ifdef CHECKING_OR_PROVING:
                        if solver.proof.is_some() {
                            crate::proof::add_binary_to_proof(solver, repr_lit, repr_other);
                        }
                        // CHECK_AND_ADD_BINARY: compiled out (NDEBUG).
                        delayed_removed.push([lit, other]);
                    }
                }
            }
        }
        crate::vector::resize_vector(solver, lit, q - begin); // SET_END_OF_WATCHES
        if lit == repr_lit {
            continue;
        }
        // watches = all_watches + repr_lit;
        // for (all_stack (watch, watch, *delayed_watched))
        //   PUSH_WATCHES (*watches, watch);
        for i in 0..solver.delayed.len() {
            let watch = solver.delayed[i];
            crate::vector::push_vectors(solver, repr_lit, watch);
        }
        solver.delayed.clear(); // CLEAR_STACK (*delayed_watched)
    }
    assign_and_propagate_units(solver, &mut units);
    drop(units); // RELEASE_STACK
    for i in 0..delayed_deleted.len() {
        let litwatch = delayed_deleted[i];
        let lit = litwatch.lit;
        let watch = litwatch.watch;
        debug_assert!(watch_is_binary(watch));
        let other = watch_lit(watch);
        crate::clause::delete_binary(solver, lit, other);
    }
    drop(delayed_deleted); // RELEASE_STACK
    // #ifdef CHECKING_OR_PROVING:
    for i in 0..delayed_removed.len() {
        let [lit, other] = delayed_removed[i];
        if solver.proof.is_some() {
            crate::proof::delete_binary_from_proof(solver, lit, other);
        }
        // REMOVE_CHECKER_BINARY: compiled out (NDEBUG).
    }
    drop(delayed_removed); // RELEASE_STACK
}

// static void substitute_clauses (kissat *, unsigned *repr)
fn substitute_clauses(solver: &mut Solver, repr: &[u32]) {
    if solver.inconsistent {
        return;
    }
    let mut units: Vec<u32> = Vec::new();
    let mut delayed_garbage: Vec<Reference> = Vec::new();
    let mut ref_: Reference = 0;
    while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        if solver.arena.clause(ref_).garbage() {
            ref_ = next;
            continue;
        }
        debug_assert!(solver.clause.is_empty());
        let mut shrink = false;
        let mut satisfied = false;
        let mut substitute = false;
        let mut tautological = false;
        let size = solver.arena.clause(ref_).size();
        for i in 0..size {
            let lit = solver.arena.clause(ref_).lit(i);
            let lit_value = solver.values[lit as usize];
            if lit_value < 0 {
                shrink = true;
                continue;
            }
            if lit_value > 0 {
                satisfied = true;
                break;
            }
            let repr_lit = repr[lit as usize];
            let repr_value = solver.values[repr_lit as usize];
            if repr_value < 0 {
                shrink = true;
                continue;
            }
            if repr_value > 0 {
                satisfied = true;
                break;
            }
            if lit != repr_lit {
                debug_assert!(solver.values[repr_lit as usize] == 0);
                substitute = true;
            }
            if solver.marks[repr_lit as usize] != 0 {
                shrink = true;
                continue; // skipping duplicated
            }
            let not_repr_lit = crate::literal::not(repr_lit);
            if solver.marks[not_repr_lit as usize] != 0 {
                tautological = true;
                break;
            }
            solver.marks[repr_lit as usize] = 1; // marks[repr_lit] = true
            solver.clause.push(repr_lit);
        }
        if satisfied || tautological {
            crate::clause::mark_clause_as_garbage(solver, ref_);
        } else if substitute || shrink {
            let size = solver.clause.len() as u32;
            if size == 0 {
                // simplifies to empty clause
                // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
                if solver.proof.is_some() {
                    crate::proof::add_empty_to_proof(solver);
                }
                solver.inconsistent = true;
                break;
            } else if size == 1 {
                debug_assert!(shrink);
                let unit = solver.clause[0];
                units.push(unit);
                delayed_garbage.push(ref_);
            } else if size == 2 {
                debug_assert!(shrink);
                let first = solver.clause[0];
                let second = solver.clause[1];
                crate::clause::new_binary_clause(solver, first, second);
                crate::clause::mark_clause_as_garbage(solver, ref_);
            } else {
                // ADD_LITS_TO_PROOF (new_size, new_lits);
                // CHECK_AND_ADD_LITS: compiled out (NDEBUG).
                if solver.proof.is_some() {
                    let new_lits = std::mem::take(&mut solver.clause);
                    crate::proof::add_lits_to_proof(solver, &new_lits);
                    solver.clause = new_lits;
                }

                // DELETE_CLAUSE_FROM_PROOF (c);
                // REMOVE_CHECKER_CLAUSE: compiled out (NDEBUG).
                if solver.proof.is_some() {
                    crate::proof::delete_clause_from_proof(solver, ref_);
                }

                let old_size = solver.arena.clause(ref_).size();
                let new_size = solver.clause.len() as u32;

                debug_assert!(new_size <= old_size);
                // memcpy (old_lits, new_lits, new_size * sizeof *old_lits);
                for i in 0..new_size {
                    let l = solver.clause[i as usize];
                    solver.arena.clause_mut(ref_).set_lit(i, l);
                }

                debug_assert!(shrink == (new_size < old_size));
                if new_size < old_size {
                    // PORT NOTE: C sets c->size = new_size BEFORE writing the
                    // shrunken sentinel into lits[old_size-1]; the writes
                    // touch disjoint words, so the sentinel write happens
                    // first here to satisfy set_lit's bounds debug_assert.
                    // The final clause state is identical.
                    let mut c = solver.arena.clause_mut(ref_);
                    if !c.shrunken() {
                        c.set_shrunken(true);
                        c.set_lit(old_size - 1, INVALID); // old_lits[old_size-1]
                    }
                    c.set_size(new_size);
                    c.set_searched(2);
                }
            }
        } else {
            // unchanged
        }
        for i in 0..solver.clause.len() {
            let lit = solver.clause[i];
            solver.marks[lit as usize] = 0;
        }
        solver.clause.clear();
        ref_ = next;
    }
    assign_and_propagate_units(solver, &mut units);
    drop(units); // RELEASE_STACK
    for i in 0..delayed_garbage.len() {
        let ref_ = delayed_garbage[i];
        crate::clause::mark_clause_as_garbage(solver, ref_);
    }
    drop(delayed_garbage); // RELEASE_STACK
}

// static bool substitute_round (kissat *, unsigned round)
fn substitute_round(solver: &mut Solver, round: u32) -> bool {
    debug_assert!(!solver.inconsistent);
    let active = solver.active;
    let lits = solver.lits() as usize;
    let mut repr: Vec<u32> = vec![INVALID; lits]; // memset (repr, 0xff, bytes)
    determine_representatives(solver, &mut repr);
    let eliminate = add_representative_equivalences(solver, &repr);
    substitute_binaries(solver, &repr);
    substitute_clauses(solver, &repr);
    remove_representative_equivalences(solver, &repr, eliminate);
    drop(repr); // kissat_dealloc
    let removed = active - solver.active;
    crate::print::phase(
        solver,
        "substitute",
        u64::MAX, // GET (substitutions): STATISTIC
        format_args!(
            "round {} removed {} variables {:.0}%",
            round,
            removed,
            crate::format::percent(removed as f64, active as f64)
        ),
    );
    // kissat_check_statistics: compiled out (NDEBUG).
    crate::report::report(solver, removed == 0, 'd');
    !solver.inconsistent && removed != 0
}

// static void substitute_rounds (kissat *, bool complete)
fn substitute_rounds(solver: &mut Solver, complete: bool) {
    crate::profile::start_checked(solver, Prof::substitute); // START (substitute)
    solver.statistics.substitutions += 1; // INC (substitutions): STATISTIC kept
    let maxrounds = solver.options.substituterounds as u32;
    for round in 1..=maxrounds {
        let before = solver.statistics.substitute_ticks;
        if !substitute_round(solver, round) {
            break;
        }
        let after = solver.statistics.substitute_ticks;
        let ticks = after - before;
        if !complete {
            let reference = solver.statistics.search_ticks - solver.last.ticks.probe;
            let fraction = solver.options.substituteeffort as f64 * 1e-3;
            let limit = (fraction * reference as f64) as u64;
            if ticks > limit {
                crate::print::extremely_verbose(
                    solver,
                    format_args!(
                        "last substitute round took {} 'substitute_ticks' \
                         > limit {} = {} * {} 'search_ticks'",
                        ticks, limit, fraction, reference
                    ),
                );
                break;
            }
        }
    }
    if !solver.inconsistent {
        crate::watch::watch_large_clauses(solver);
        solver.large_clauses_watched_after_binary_clauses = true;
        // kissat_reset_propagate (solver);
        solver.propagate = 0;
        debug_assert!(solver.level == 0);
        let _ = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
    }
    crate::profile::stop_checked(solver, Prof::substitute); // STOP (substitute)
}

/// Port of `kissat_substitute`.
pub fn substitute(solver: &mut Solver, complete: bool) {
    if solver.inconsistent {
        return;
    }
    debug_assert!(solver.probing);
    debug_assert!(solver.watching);
    debug_assert!(solver.level == 0);
    solver.large_clauses_watched_after_binary_clauses = false;
    if solver.options.substitute == 0 {
        return;
    }
    if terminated!(solver, substitute_terminated_1) {
        return;
    }
    substitute_rounds(solver, complete);
}
