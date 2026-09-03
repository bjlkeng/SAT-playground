// Port of src/definition.c (kissat 4.0.4).
//
// Kitten-based definition extraction: export the environment (occurrence
// lists of lit / not_lit with the pivot literal removed) to the embedded
// sub-solver; an UNSAT core certifies a definition and its clauses become
// the gate/antecedent split.
//
// PORT NOTE: kitten.rs keeps the C `kitten_` name prefix; the kitten is
// std::mem::take'n out of solver.kitten for the duration of the call so
// `&mut Kitten` and `&mut Solver` can coexist (C uses interior pointers).
// PORT NOTE: the C traverse callbacks take a `void *state` extractor struct;
// the port uses capturing closures.  The proof slices passed to
// add_lits_to_proof / delete_internal_from_proof are copied out of
// solver.added to satisfy the borrow checker — byte-identical proof output.
// PORT NOTE: the `#if !defined(NDEBUG) || !defined(NPROOFS)` lemma-extractor
// block is compiled (NPROOFS off); its `GET_OPTION (check) > 1` disjunct is
// NDEBUG-only, so the condition reduces to `solver->proof`.
// PORT NOTE: definitions_checked / definitions_extracted are METRIC (no-op);
// definition_units is STATISTIC (kept as a real, never-printed field).

use crate::internal::{Solver, INVALID};
use crate::profile::Prof;
use crate::watch::{watch_is_binary, watch_lit, watch_ref};

/// Port of `kissat_find_definition`.
pub fn find_definition(solver: &mut Solver, lit: u32) -> bool {
    if solver.options.definitions == 0 {
        return false;
    }
    crate::profile::start_checked(solver, Prof::definition); // START (definition)
    let mut kitten = solver.kitten.take().expect("solver->kitten"); // assert (kitten)
    crate::kitten::kitten_clear(&mut kitten);
    let not_lit = crate::literal::not(lit);
    // definition_extractor: lit + watches[0] = WATCHES (lit),
    // watches[1] = WATCHES (not_lit) — captured below by the closure.
    crate::kitten::kitten_track_antecedents(&mut kitten);
    let mut exported: u32 = 0;
    // #if !defined(QUIET) || !defined(NDEBUG)
    let mut occs: [u64; 2] = [0, 0];
    let mut clause_buffer: Vec<u32> = Vec::new();
    for sign in 0..2u32 {
        let except = if sign != 0 { not_lit } else { lit };
        let side_lit = if sign != 0 { not_lit } else { lit };
        let v = solver.watches[side_lit as usize];
        for p in v.begin..v.end {
            let watch = solver.vectors.stack[p];
            if watch_is_binary(watch) {
                let other = watch_lit(watch);
                crate::kitten::kitten_clause_with_id_and_exception(
                    &mut kitten,
                    solver,
                    exported,
                    &[other],
                    INVALID, // INVALID_LIT
                );
            } else {
                let ref_ = watch_ref(watch);
                clause_buffer.clear();
                {
                    let c = solver.arena.clause(ref_);
                    clause_buffer.extend_from_slice(c.lits());
                }
                crate::kitten::kitten_clause_with_id_and_exception(
                    &mut kitten,
                    solver,
                    exported,
                    &clause_buffer,
                    except,
                );
            }
            occs[sign as usize] += 1;
            exported += 1;
        }
    }
    let mut res = false;
    // INC (definitions_checked): METRIC, compiled out.
    let limit = solver.options.definitionticks as u64;
    crate::kitten::kitten_set_ticks_limit(&mut kitten, solver, limit);
    let status = crate::kitten::kitten_solve(&mut kitten, solver);
    if status == 20 {
        // sub-solver result UNSAT shows definition exists
        let (mut reduced, _learned) = crate::kitten::kitten_compute_clausal_core(&mut kitten);
        let mut aborted = false;
        let mut i: i32 = 2;
        while i <= solver.options.definitioncores {
            crate::kitten::kitten_shrink_to_clausal_core(&mut kitten);
            crate::kitten::kitten_shuffle_clauses(&mut kitten);
            crate::kitten::kitten_set_ticks_limit(&mut kitten, solver, 10 * limit);
            let tmp = crate::kitten::kitten_solve(&mut kitten, solver);
            debug_assert!(tmp == 0 || tmp == 20);
            if tmp == 0 {
                aborted = true; // goto ABORT
                break;
            }
            let (r, _l) = crate::kitten::kitten_compute_clausal_core(&mut kitten);
            debug_assert!(r <= reduced);
            reduced = r;
            i += 1;
        }
        if !aborted {
            // INC (definitions_extracted): METRIC, compiled out.
            // kitten_traverse_core_ids (kitten, &extractor,
            //                           traverse_definition_core);
            let size_watches0 = solver.watches[lit as usize].size();
            {
                let watches0 = solver.watches[lit as usize];
                let watches1 = solver.watches[not_lit as usize];
                let stack = &solver.vectors.stack;
                let gates = &mut solver.gates;
                crate::kitten::kitten_traverse_core_ids(&kitten, |id| {
                    let id = id as usize;
                    let (sign, watch) = if id < size_watches0 {
                        (0usize, stack[watches0.begin + id])
                    } else {
                        let tmp = id - size_watches0;
                        debug_assert!(tmp < watches1.size());
                        (1usize, stack[watches1.begin + tmp])
                    };
                    gates[sign].push(watch);
                });
            }
            let size0 = solver.gates[0].len();
            let size1 = solver.gates[1].len();
            debug_assert!(reduced as usize == size0 + size1);
            crate::print::extremely_verbose(
                solver,
                format_args!(
                    "definition extracted with core size {} = {} + {} clauses {:.0}% \
                     of {} = {} + {}",
                    reduced,
                    size0,
                    size1,
                    crate::format::percent(reduced as f64, exported as f64),
                    exported,
                    occs[0],
                    occs[1]
                ),
            );
            let mut unit = INVALID;
            if size0 == 0 {
                unit = not_lit;
                debug_assert!(size1 != 0);
            } else if size1 == 0 {
                unit = lit;
            }

            if unit != INVALID {
                solver.statistics.definition_units += 1; // INC: STATISTIC kept

                crate::print::extremely_verbose(
                    solver,
                    "one sided core definition extraction yields failed literal",
                );
                // (GET_OPTION (check) > 1 is NDEBUG-only; condition is proof.)
                if solver.proof.is_some() {
                    // lemma_extractor + traverse_one_sided_core_lemma
                    debug_assert!(solver.added.is_empty());
                    let mut lemmas: u32 = 0;
                    crate::kitten::kitten_traverse_core_clauses(
                        &mut kitten,
                        |learned, lits| {
                            if !learned {
                                return;
                            }
                            debug_assert!(lemmas != 0 || solver.added.is_empty());
                            if !lits.is_empty() {
                                let size = lits.len();
                                solver.added.push(size as u32 + 1);
                                let offset = solver.added.len();
                                solver.added.push(unit);
                                for &other in lits {
                                    solver.added.push(other);
                                }
                                debug_assert!(offset + size + 1 == solver.added.len());
                                // CHECK_AND_ADD_LITS: compiled out (NDEBUG).
                                // ADD_LITS_TO_PROOF (size + 1, extended):
                                let extended: Vec<u32> = solver.added[offset..].to_vec();
                                crate::proof::add_lits_to_proof(solver, &extended);
                            } else {
                                crate::assign::learned_unit(solver, unit);
                                let mut p = 0usize;
                                while p != solver.added.len() {
                                    let size = solver.added[p] as usize;
                                    p += 1;
                                    debug_assert!(p + size <= solver.added.len());
                                    // REMOVE_CHECKER_LITS: compiled out.
                                    // DELETE_LITS_FROM_PROOF (size, p):
                                    let dl: Vec<u32> =
                                        solver.added[p..p + size].to_vec();
                                    crate::proof::delete_internal_from_proof(solver, &dl);
                                    p += size;
                                }
                                solver.added.clear();
                            }
                            lemmas += 1;
                        },
                    );
                } else {
                    crate::assign::learned_unit(solver, unit);
                }
            }
            solver.gate_eliminated = true; // GATE_ELIMINATED (definitions)
            solver.resolve_gate = true;
            res = true;
        }
    }
    // ABORT: sub-solver failed to show that definition exists
    solver.analyzed.clear(); // CLEAR_STACK (solver->analyzed)
    solver.kitten = Some(kitten);
    crate::profile::stop_checked(solver, Prof::definition); // STOP (definition)
    res
}
