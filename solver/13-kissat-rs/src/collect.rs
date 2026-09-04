// Port of src/collect.c + src/collect.h inline functions (kissat 4.0.4).
//
// PORT NOTES (GC ordering):
//  - The mark-and-sweep is an in-place forward compaction over the arena:
//    clause order is preserved exactly (src/dst cursors, dst <= src, all
//    copies are forward memmoves), then — only when redundant clauses ended
//    up before irredundant ones — move_redundant_clauses_to_the_end copies
//    redundant clauses into a side buffer and re-emits irredundant first,
//    redundant after, both in original relative order, exactly as C.
//  - C clause pointers become ward offsets (u64); NULL-vs-begin distinctions
//    (first/last_irredundant/first_reducible/first_redundant locals) become
//    Option<u64>.  `first == begin` in the C last_irredundant fixups is
//    equivalent to `start == 0` (first = begin iff start == 0).
//  - INC (garbage_collections) / INC (sparse_gcs) /
//    INC (dense_garbage_collections) / INC (moved) / ADD (flushed) are
//    METRIC counters: no-ops; GET of them in kissat_phase prints as
//    "no count" (u64::MAX).  kissat_check_statistics is a no-op (NDEBUG).
//  - CHECKING_OR_PROVING is defined in this build (NPROOFS undefined); the
//    solver->added/removed bookkeeping is compiled in and gated at runtime
//    by kissat_checking_or_proving == solver.proof.is_some().
//  - kissat_map_literal is an inline.h helper; it is defined here (private)
//    over the export_/import_ fields because inline.rs's export_literal
//    asserts a nonzero export entry while map_literal needs the 0 case.
//  - In flush_watched_clauses_by_literal C computes
//    `mlit = kissat_map_literal (solver, lit, true)` with map=true even when
//    not compacting (only used under compact) — quirk ported as-is.
//  - The binary-reason rewrite in the sweep stores the ORIGINAL (unmapped)
//    other literal (`a->reason = other`), and update_large_reason uses the
//    ORIGINAL forced literal from the walk — both exactly as C (compaction
//    of assigned happens later in kissat_finalize_compacting).

use crate::internal::{Assigned, Solver, INVALID_LEVEL};
use crate::literal::INVALID_LIT;
use crate::profile::Prof;
use crate::reference::{Reference, INVALID_REF};

/*------------------------------------------------------------------------*/
// collect.h inline functions (pre-existing).

#[inline]
pub fn defrag_watches(solver: &mut Solver) {
    crate::vector::defrag_vectors(solver);
}

#[inline]
pub fn defrag_watches_if_needed(solver: &mut Solver) {
    let size = solver.vectors.stack.len();
    let size_limit = solver.options.defragsize as usize;
    if size <= size_limit {
        return;
    }
    let usable = solver.vectors.usable as usize;
    let usable_limit = (size * solver.options.defraglim as usize) / 100;
    if usable <= usable_limit {
        return;
    }
    // INC (vectors_defrags_needed) is METRIC-only: no-op in the reference build.
    defrag_watches(solver);
}

/*------------------------------------------------------------------------*/
// Helpers.

/// wards occupied by a non-shrunken clause of `size` literals.
#[inline]
fn wards_of_clause(size: u32) -> u64 {
    (crate::clause::words_of_clause(size) / crate::arena::WORDS_PER_WARD) as u64
}

// inline.h kissat_map_literal over the split-out export/import arrays.
fn map_literal_parts(
    export_: &[i32],
    import_: &[crate::internal::Import],
    ilit: u32,
    map: bool,
) -> u32 {
    if !map {
        return ilit;
    }
    // kissat_export_literal with the elit == 0 case propagated:
    let iidx = crate::literal::idx(ilit);
    let mut elit = export_[iidx as usize];
    if elit == 0 {
        return INVALID_LIT;
    }
    if crate::literal::negated(ilit) != 0 {
        elit = -elit;
    }
    let eidx = elit.unsigned_abs();
    let import = &import_[eidx as usize];
    if import.eliminated {
        return INVALID_LIT;
    }
    let mut mlit = import.lit;
    if elit < 0 {
        mlit = crate::literal::not(mlit);
    }
    mlit
}

fn map_literal(solver: &Solver, ilit: u32, map: bool) -> u32 {
    map_literal_parts(&solver.export_, &solver.import_, ilit, map)
}

/*------------------------------------------------------------------------*/
// collect.c statics.

// static flush_watched_clauses_by_literal
fn flush_watched_clauses_by_literal(
    solver: &mut Solver,
    lit: u32,
    compact: bool,
    start: Reference,
) {
    debug_assert!(start != INVALID_REF);

    let lit_value = solver.values[lit as usize];
    let lit_level = solver.assigned[crate::literal::idx(lit) as usize].level;
    let lit_fixed: i8 = if lit_value != 0 && lit_level == 0 {
        lit_value
    } else {
        0
    };
    let mlit = map_literal(solver, lit, true);

    let v = solver.watches[lit as usize];
    let begin = v.begin;
    let end = v.end;
    let mut q = begin;
    let mut p = begin;

    while p != end {
        let head = solver.vectors.stack[p];
        p += 1;
        if crate::watch::watch_is_binary(head) {
            let other = crate::watch::watch_lit(head);
            let other_idx = crate::literal::idx(other);
            let other_value = solver.values[other as usize];
            let other_fixed: i8 =
                if other_value != 0 && solver.assigned[other_idx as usize].level == 0 {
                    other_value
                } else {
                    0
                };
            let mother = map_literal(solver, other, compact);
            if lit_fixed > 0 || other_fixed > 0 || mother == INVALID_LIT {
                if lit < other {
                    crate::clause::delete_binary(solver, lit, other);
                }
            } else {
                debug_assert!(lit_fixed == 0);
                debug_assert!(other_fixed == 0);
                // head.binary.lit = mother; *q++ = head;
                solver.vectors.stack[q] = crate::watch::binary_watch(mother);
                q += 1;
            }
        } else {
            debug_assert!(solver.watching);
            let tail = solver.vectors.stack[p];
            p += 1;
            if lit_fixed == 0 {
                let ref_ = crate::watch::watch_ref(tail);
                if ref_ < start {
                    solver.vectors.stack[q] = head;
                    q += 1;
                    solver.vectors.stack[q] = tail;
                    q += 1;
                }
            }
        }
    }

    debug_assert!(lit_fixed == 0 || q == begin);
    crate::vector::resize_vector(solver, lit, q - begin); // SET_END_OF_WATCHES

    if !compact {
        return;
    }

    if mlit == INVALID_LIT {
        return;
    }

    if lit_fixed != 0 {
        debug_assert!(solver.watches[mlit as usize].empty());
    } else if mlit < lit {
        // *mlit_watches = *lit_watches; memset (lit_watches, 0, ...);
        solver.watches[mlit as usize] = solver.watches[lit as usize];
        solver.watches[lit as usize] = crate::vector::Vector::default();
    } else {
        debug_assert!(mlit == lit);
    }
}

// static flush_all_watched_clauses
fn flush_all_watched_clauses(solver: &mut Solver, compact: bool, start: Reference) {
    debug_assert!(solver.watching);
    for idx in 0..solver.vars {
        let lit = crate::literal::lit(idx);
        flush_watched_clauses_by_literal(solver, lit, compact, start);
        let not_lit = crate::literal::not(lit);
        flush_watched_clauses_by_literal(solver, not_lit, compact, start);
    }
}

// static update_large_reason (split-borrow form: assigned + arena words).
fn update_large_reason(assigned: &mut [Assigned], forced: u32, dst_ref: Reference, words: &mut [u32]) {
    debug_assert!(words[dst_ref as usize * 4] & crate::clause::REASON_BIT != 0);
    debug_assert!(forced != INVALID_LIT);
    let forced_idx = crate::literal::idx(forced) as usize;
    let a = &mut assigned[forced_idx];
    debug_assert!(!a.binary());
    if a.reason != dst_ref {
        a.reason = dst_ref;
    }
    // dst->reason = false;
    words[dst_ref as usize * 4] &= !crate::clause::REASON_BIT;
}

// static get_forced
fn get_forced(values: &[i8], words: &[u32], dst_ref: Reference) -> u32 {
    let base = dst_ref as usize * 4;
    debug_assert!(words[base] & crate::clause::REASON_BIT != 0);
    let size = words[base + 2] as usize;
    let mut forced = INVALID_LIT;
    for i in 0..size {
        let lit = words[base + 3 + i];
        let value = values[lit as usize];
        if value <= 0 {
            continue;
        }
        forced = lit;
        break;
    }
    debug_assert!(forced != INVALID_LIT);
    forced
}

// static get_forced_and_update_large_reason
fn get_forced_and_update_large_reason(
    assigned: &mut [Assigned],
    values: &[i8],
    dst_ref: Reference,
    words: &mut [u32],
) {
    let forced = get_forced(values, words, dst_ref);
    update_large_reason(assigned, forced, dst_ref, words);
}

// static update_first_reducible (end/first_reducible in ward offsets).
fn update_first_reducible_inner(solver: &mut Solver, end: u64, first_reducible: Option<u64>) {
    match first_reducible {
        Some(off) if off >= end => {
            // first reducible after end of arena
            solver.first_reducible = INVALID_REF;
        }
        Some(off) => {
            solver.first_reducible = off as Reference;
        }
        None => {
            solver.first_reducible = INVALID_REF;
        }
    }
}

// static update_last_irredundant
fn update_last_irredundant_inner(solver: &mut Solver, end: u64, last_irredundant: Option<u64>) {
    match last_irredundant {
        None => {
            solver.last_irredundant = INVALID_REF;
        }
        Some(off) if end <= off => {
            solver.last_irredundant = INVALID_REF;
        }
        Some(off) => {
            solver.last_irredundant = off as Reference;
        }
    }
}

/// Port of `kissat_update_first_reducible` (public; takes the clause ref).
pub fn update_first_reducible(solver: &mut Solver, reducible: Reference) {
    {
        let c = solver.arena.clause(reducible);
        debug_assert!(!c.garbage());
        debug_assert!(c.redundant());
    }
    if solver.first_reducible != INVALID_REF && reducible >= solver.first_reducible {
        // no need to update larger first reducible
        return;
    }
    let end = solver.arena.size_wards();
    update_first_reducible_inner(solver, end, Some(reducible as u64));
}

/// Port of `kissat_update_last_irredundant` (public; takes the clause ref).
pub fn update_last_irredundant(solver: &mut Solver, irredundant: Reference) {
    {
        let c = solver.arena.clause(irredundant);
        debug_assert!(!c.garbage());
        debug_assert!(!c.redundant());
    }
    if solver.last_irredundant != INVALID_REF && irredundant <= solver.last_irredundant {
        // no need to update smaller last irredundant
        return;
    }
    let end = solver.arena.size_wards();
    update_last_irredundant_inner(solver, end, Some(irredundant as u64));
}

// static move_redundant_clauses_to_the_end
fn move_redundant_clauses_to_the_end(solver: &mut Solver, ref_: Reference) {
    // INC (moved): METRIC, no-op.
    debug_assert!(ref_ != INVALID_REF);
    let end = solver.arena.size_wards();
    debug_assert!(ref_ as u64 <= end);
    let begin = ref_ as u64;
    let bytes_redundant = (end - begin) * crate::arena::BYTES_PER_WARD as u64;
    let bytes_str = crate::format::format_bytes(&mut solver.format, bytes_redundant);
    crate::print::phase(
        solver,
        "move",
        u64::MAX, // GET (moved): METRIC
        format!("moving redundant clauses of {} to the end", bytes_str),
    );
    crate::trail::mark_reason_clauses(solver, ref_);
    // clause *redundant = kissat_malloc (solver, bytes_redundant);
    let mut redundant_buf: Vec<u32> = Vec::with_capacity(((end - begin) * 4) as usize);

    let mut last_irredundant: Option<u64> = if solver.last_irredundant == INVALID_REF {
        None
    } else {
        Some(solver.last_irredundant as u64)
    };
    let mut first_reducible: Option<u64> = None;
    let q_final: u64;

    {
        let Solver {
            arena,
            values,
            assigned,
            ..
        } = &mut *solver;
        let words = arena.words_mut();
        let end_w = (end * 4) as usize;
        let mut p = (begin * 4) as usize;
        let mut q = p;
        while p != end_w {
            debug_assert!(words[p] & crate::clause::SHRUNKEN_BIT == 0);
            let size = words[p + 2];
            let wds = crate::clause::words_of_clause(size);
            if words[p] & crate::clause::REDUNDANT_BIT != 0 {
                // memcpy (r, p, bytes)
                redundant_buf.extend_from_slice(&words[p..p + wds]);
            } else {
                // memmove (q, p, bytes)
                words.copy_within(p..p + wds, q);
                last_irredundant = Some((q / 4) as u64);
                if words[q] & crate::clause::REASON_BIT != 0 {
                    let dst_ref = (q / 4) as Reference;
                    get_forced_and_update_large_reason(assigned, values, dst_ref, words);
                }
                q += wds;
            }
            p += wds;
        }
        // copy the redundant clauses back after the irredundant ones
        let mut r = 0usize;
        while q != end_w {
            let size = redundant_buf[r + 2];
            let wds = crate::clause::words_of_clause(size);
            words[q..q + wds].copy_from_slice(&redundant_buf[r..r + wds]);
            if words[q] & crate::clause::REASON_BIT != 0 {
                let dst_ref = (q / 4) as Reference;
                get_forced_and_update_large_reason(assigned, values, dst_ref, words);
            }
            debug_assert!(words[q] & crate::clause::REDUNDANT_BIT != 0);
            if first_reducible.is_none() {
                first_reducible = Some((q / 4) as u64);
            }
            r += wds;
            q += wds;
        }
        debug_assert!(r <= redundant_buf.len());
        q_final = (q / 4) as u64;
    }
    // kissat_free (solver, redundant, bytes_redundant): drop.
    drop(redundant_buf);

    debug_assert!(first_reducible.is_none() || first_reducible.unwrap() < q_final);

    update_first_reducible_inner(solver, q_final, first_reducible);
    update_last_irredundant_inner(solver, q_final, last_irredundant);
    crate::internal::reset_last_learned(solver);
}

// static sparse_sweep_garbage_clauses
fn sparse_sweep_garbage_clauses(
    solver: &mut Solver,
    compact: bool,
    start: Reference,
) -> Reference {
    debug_assert!(solver.watching);
    let checking_or_proving = solver.proof.is_some(); // kissat_checking_or_proving
    debug_assert!(solver.added.is_empty());
    debug_assert!(solver.removed.is_empty());

    let mut flushed_garbage_clauses: u64 = 0;
    let mut flushed_satisfied_clauses: u64 = 0;
    let mut flushed: u64 = 0;

    let end = solver.arena.size_wards();

    // first = start ? dereference (start) : begin — same offset either way;
    // `first == begin` below is equivalent to start == 0.
    let first: u64 = start as u64;
    let first_is_begin = start == 0;
    let mut src: u64 = first;
    let mut dst: u64 = first;

    let mut first_redundant: Option<u64> = None;
    let mut first_reducible: Option<u64> = None;
    let mut last_irredundant: Option<u64> = if start != 0 {
        if solver.last_irredundant == INVALID_REF {
            None
        } else {
            Some(solver.last_irredundant as u64)
        }
    } else {
        None
    };

    while src != end {
        let src_ref = src as Reference;
        if solver.arena.clause(src_ref).garbage() {
            let next = crate::clause::delete_clause(solver, src_ref) as u64;
            flushed_garbage_clauses += 1;
            if last_irredundant == Some(src) {
                last_irredundant = if first_is_begin { None } else { Some(first) };
            }
            src = next;
            continue;
        }

        debug_assert!(solver.arena.clause(src_ref).size() > 1);
        let next = solver.arena.next_clause_ref(src_ref) as u64;

        let old_size: u32;
        let new_size: u32;
        let mut mfirst = INVALID_LIT;
        let mut msecond = INVALID_LIT;
        let mut forced = INVALID_LIT;
        let mut other = INVALID_LIT;
        let mut non_false: u32 = 0;
        let mut satisfied = false;

        {
            let Solver {
                arena,
                values,
                assigned,
                export_,
                import_,
                added,
                removed,
                ..
            } = &mut *solver;
            let words = arena.words_mut();
            let src_w = src as usize * 4;
            let dst_w = dst as usize * 4;
            // *(unsigned *) dst = *(unsigned *) src;  (header word)
            words[dst_w] = words[src_w];
            old_size = words[src_w + 2];
            let mut kept: usize = 0; // q - dst->lits

            for i in 0..old_size as usize {
                let lit = words[src_w + 3 + i];
                if checking_or_proving {
                    removed.push(lit);
                }
                if satisfied {
                    continue;
                }
                let tmp = values[lit as usize];
                let idx = crate::literal::idx(lit);
                let level = if tmp != 0 {
                    assigned[idx as usize].level
                } else {
                    INVALID_LEVEL
                };
                if tmp < 0 && level == 0 {
                    flushed += 1;
                } else if tmp > 0 && level == 0 {
                    debug_assert!(!satisfied);
                    debug_assert!(words[dst_w] & crate::clause::REASON_BIT == 0);
                    satisfied = true;
                } else {
                    let mlit = map_literal_parts(export_, import_, lit, compact);

                    if tmp > 0 {
                        debug_assert!(level != 0);
                        forced = if non_false != 0 { INVALID_LIT } else { lit };
                        non_false += 1;
                    } else if tmp < 0 {
                        other = lit;
                    }

                    if mfirst == INVALID_LIT {
                        mfirst = mlit;
                    } else if msecond == INVALID_LIT {
                        msecond = mlit;
                    }

                    words[dst_w + 3 + kept] = mlit; // *q++ = mlit;
                    kept += 1;

                    if checking_or_proving {
                        added.push(lit);
                    }
                }
            }
            new_size = kept as u32;
        }

        if satisfied {
            if solver.arena.clause(dst as Reference).redundant() {
                debug_assert!(solver.statistics.clauses_redundant > 0);
                solver.statistics.clauses_redundant -= 1; // DEC
            } else {
                debug_assert!(solver.statistics.clauses_irredundant > 0);
                solver.statistics.clauses_irredundant -= 1; // DEC
            }
            flushed_satisfied_clauses += 1;
            if checking_or_proving {
                // REMOVE_CHECKER_STACK: no-op (NDEBUG).
                // DELETE_STACK_FROM_PROOF (solver->removed):
                let removed = std::mem::take(&mut solver.removed);
                crate::proof::delete_internal_from_proof(solver, &removed);
                solver.removed = removed;
                solver.added.clear();
                solver.removed.clear();
            }
            if last_irredundant == Some(src) {
                last_irredundant = if first_is_begin { None } else { Some(first) };
            }
            src = next;
            continue;
        }

        debug_assert!(new_size <= old_size);
        debug_assert!(new_size > 1);

        if new_size == 2 {
            debug_assert!(mfirst != INVALID_LIT);
            debug_assert!(msecond != INVALID_LIT);

            debug_assert!(solver.statistics.clauses_binary < u64::MAX);
            solver.statistics.clauses_binary += 1;
            let dst_redundant = solver.arena.clause(dst as Reference).redundant();
            let mut redundant = dst_redundant;
            if redundant {
                debug_assert!(solver.statistics.clauses_redundant > 0);
                solver.statistics.clauses_redundant -= 1;
                redundant = false;
            } else {
                debug_assert!(solver.statistics.clauses_irredundant > 0);
                solver.statistics.clauses_irredundant -= 1;
            }
            crate::watch::watch_binary(solver, mfirst, msecond);

            if solver.arena.clause(dst as Reference).reason() {
                debug_assert!(non_false == 1);
                debug_assert!(other != INVALID_LIT);
                debug_assert!(forced != INVALID_LIT);

                let forced_idx = crate::literal::idx(forced) as usize;
                let a = &mut solver.assigned[forced_idx];
                debug_assert!(!a.binary());

                a.set_binary(true);
                a.reason = other;
            }

            if !redundant && last_irredundant == Some(src) {
                last_irredundant = if first_is_begin { None } else { Some(first) };
            }
        } else {
            debug_assert!(new_size > 2);

            {
                let Solver {
                    arena,
                    values,
                    assigned,
                    ..
                } = &mut *solver;
                let words = arena.words_mut();
                let dst_w = dst as usize * 4;
                words[dst_w + 2] = new_size; // dst->size = new_size;
                words[dst_w] &= !crate::clause::SHRUNKEN_BIT; // dst->shrunken = false;
                words[dst_w + 1] = 2; // dst->searched = 2;

                if words[dst_w] & crate::clause::REASON_BIT != 0 {
                    update_large_reason(assigned, forced, dst as Reference, words);
                }
                let _ = values;
            }

            let next_dst = dst + wards_of_clause(new_size);

            if solver.arena.clause(dst as Reference).redundant() {
                if first_reducible.is_none() {
                    first_reducible = Some(dst);
                }
                if first_redundant.is_none() {
                    first_redundant = Some(dst);
                }
            } else {
                last_irredundant = Some(dst);
            }

            dst = next_dst;
        }

        if checking_or_proving {
            if new_size != old_size {
                debug_assert!(new_size > 1);
                debug_assert!(new_size < old_size);

                // CHECK_AND_ADD_STACK: no-op (NDEBUG).
                // ADD_STACK_TO_PROOF (solver->added):
                let added = std::mem::take(&mut solver.added);
                crate::proof::add_lits_to_proof(solver, &added);
                solver.added = added;

                // REMOVE_CHECKER_STACK: no-op (NDEBUG).
                // DELETE_STACK_FROM_PROOF (solver->removed):
                let removed = std::mem::take(&mut solver.removed);
                crate::proof::delete_internal_from_proof(solver, &removed);
                solver.removed = removed;
            }
            solver.added.clear();
            solver.removed.clear();
        }

        src = next;
    }

    update_first_reducible_inner(solver, dst, first_reducible);
    update_last_irredundant_inner(solver, dst, last_irredundant);
    crate::internal::reset_last_learned(solver);

    // #ifndef QUIET
    {
        let bytes = (end - dst) * crate::arena::BYTES_PER_WARD as u64;
        if flushed != 0 {
            crate::print::phase(
                solver,
                "collect",
                u64::MAX, // GET (garbage_collections): METRIC
                format!("flushed {} falsified literals in large clauses", flushed),
            );
        }
        let flushed_clauses = flushed_satisfied_clauses + flushed_garbage_clauses;
        if flushed_satisfied_clauses != 0 {
            crate::print::phase(
                solver,
                "collect",
                u64::MAX,
                format!(
                    "flushed {} satisfied large clauses {:.0}%",
                    flushed_satisfied_clauses,
                    crate::format::percent(
                        flushed_satisfied_clauses as f64,
                        flushed_clauses as f64
                    )
                ),
            );
        }
        if flushed_garbage_clauses != 0 {
            crate::print::phase(
                solver,
                "collect",
                u64::MAX,
                format!(
                    "flushed {} large garbage clauses {:.0}%",
                    flushed_garbage_clauses,
                    crate::format::percent(
                        flushed_garbage_clauses as f64,
                        flushed_clauses as f64
                    )
                ),
            );
        }
        let bytes_str = crate::format::format_bytes(&mut solver.format, bytes);
        crate::print::phase(
            solver,
            "collect",
            u64::MAX,
            format!("collected {} in total", bytes_str),
        );
    }
    // ADD (flushed, flushed): METRIC, no-op.

    let mut res: Reference = INVALID_REF;

    if let (Some(fr), Some(li)) = (first_redundant, last_irredundant) {
        if fr < li {
            debug_assert!(fr < dst);
            res = fr as Reference;
            debug_assert!(res != INVALID_REF);
        }
    }

    // SET_END_OF_STACK (solver->arena, (ward *) dst);
    solver.arena.truncate_wards(dst);
    crate::arena::shrink_arena(solver);

    res
}

// static rewatch_clauses
fn rewatch_clauses(solver: &mut Solver, start: Reference) {
    debug_assert!(solver.watching);
    let end = solver.arena.size_wards();
    let mut c: u64 = start as u64;
    debug_assert!(c <= end);
    while c != end {
        let ref_ = c as Reference;
        let next = solver.arena.next_clause_ref(ref_) as u64;

        {
            let Solver {
                arena,
                values,
                assigned,
                ..
            } = &mut *solver;
            let mut cm = arena.clause_mut(ref_);
            let size = cm.size();
            crate::sort::sort_literals_inline(values, assigned, size, cm.lits_mut());
            cm.set_searched(2);
        }

        let (l0, l1) = {
            let cl = solver.arena.clause(ref_);
            (cl.lit(0), cl.lit(1))
        };
        crate::watch::push_blocking_watch(solver, l0, l1, ref_);
        crate::watch::push_blocking_watch(solver, l1, l0, ref_);

        c = next;
    }
}

/*------------------------------------------------------------------------*/
// collect.c public API.

/// Port of `kissat_sparse_collect`.
pub fn sparse_collect(solver: &mut Solver, compact: bool, start: Reference) {
    debug_assert!(solver.watching);
    crate::profile::start_checked(solver, Prof::collect); // START (collect)
    // INC (garbage_collections) / INC (sparse_gcs): METRIC, no-op.
    crate::report::report(solver, true, 'G'); // REPORT (1, 'G')
    let (vars, mfixed) = if compact {
        crate::compact::compact_literals(solver)
    } else {
        (solver.vars, INVALID_LIT)
    };
    flush_all_watched_clauses(solver, compact, start);
    let move_ = sparse_sweep_garbage_clauses(solver, compact, start);
    if compact {
        crate::compact::finalize_compacting(solver, vars, mfixed);
    }
    if move_ != INVALID_REF {
        move_redundant_clauses_to_the_end(solver, move_);
    }
    rewatch_clauses(solver, start);
    crate::report::report(solver, true, 'C'); // REPORT (1, 'C')
    // kissat_check_statistics: no-op (NDEBUG).
    crate::profile::stop_checked(solver, Prof::collect); // STOP (collect)
}

/// Port of `kissat_compacting`.
pub fn compacting(solver: &mut Solver) -> bool {
    if solver.options.compact == 0 {
        return false;
    }
    let inactive = solver.vars - solver.active;
    // unsigned limit = GET_OPTION (compactlim) / 1e2 * solver->vars;
    let limit = (solver.options.compactlim as f64 / 1e2 * solver.vars as f64) as u32;
    inactive > limit
}

/// Port of `kissat_initial_sparse_collect`.
pub fn initial_sparse_collect(solver: &mut Solver) {
    debug_assert!(solver.level == 0);
    debug_assert!(!solver.inconsistent);
    debug_assert!(solver.watching);
    if solver.statistics.units != 0 {
        let compact = solver.options.compact != 0;
        sparse_collect(solver, compact, 0);
    }
    crate::report::report(solver, false, '.'); // REPORT (0, '.')
}

// static dense_sweep_garbage_clauses
fn dense_sweep_garbage_clauses(solver: &mut Solver) {
    debug_assert!(solver.level == 0);
    debug_assert!(!solver.watching);

    let mut flushed_garbage_clauses: u64 = 0;
    let mut first_reducible: Option<u64> = None;
    let mut last_irredundant: Option<u64> = None;

    let end = solver.arena.size_wards();
    let mut src: u64 = 0;
    let mut dst: u64 = 0;

    while src != end {
        let src_ref = src as Reference;
        if solver.arena.clause(src_ref).garbage() {
            let next = crate::clause::delete_clause(solver, src_ref) as u64;
            flushed_garbage_clauses += 1;
            src = next;
            continue;
        }
        debug_assert!(solver.arena.clause(src_ref).size() > 1);
        let next = solver.arena.next_clause_ref(src_ref) as u64;
        {
            let words = solver.arena.words_mut();
            let sw = src as usize * 4;
            let dw = dst as usize * 4;
            words[dw] = words[sw]; // header word
            words[dw + 1] = words[sw + 1]; // dst->searched = src->searched;
            let size = words[sw + 2];
            words[dw + 2] = size; // dst->size = src->size;
            words[dw] &= !crate::clause::SHRUNKEN_BIT; // dst->shrunken = false;
            // memmove (dst->lits, src->lits, src->size * sizeof (unsigned)):
            words.copy_within(sw + 3..sw + 3 + size as usize, dw + 3);
            if words[dw] & crate::clause::REDUNDANT_BIT == 0 {
                last_irredundant = Some(dst);
            } else if first_reducible.is_none() {
                first_reducible = Some(dst);
            }
            dst += wards_of_clause(size); // dst = kissat_next_clause (dst);
        }
        src = next;
    }

    update_first_reducible_inner(solver, dst, first_reducible);
    update_last_irredundant_inner(solver, dst, last_irredundant);
    crate::internal::reset_last_learned(solver);

    let bytes = (end - dst) * crate::arena::BYTES_PER_WARD as u64;
    crate::print::phase(
        solver,
        "collect",
        u64::MAX, // GET (garbage_collections): METRIC
        format!("flushed {} large garbage clauses", flushed_garbage_clauses),
    );
    let bytes_str = crate::format::format_bytes(&mut solver.format, bytes);
    crate::print::phase(
        solver,
        "collect",
        u64::MAX,
        format!("collected {} in total", bytes_str),
    );

    // SET_END_OF_STACK (solver->arena, (ward *) dst);
    solver.arena.truncate_wards(dst);
    crate::arena::shrink_arena(solver);
}

/// Port of `kissat_dense_collect`.
pub fn dense_collect(solver: &mut Solver) {
    debug_assert!(!solver.watching);
    debug_assert!(solver.level == 0);
    crate::profile::start_checked(solver, Prof::collect); // START (collect)
    // INC (garbage_collections) / INC (dense_garbage_collections): METRIC.
    crate::report::report(solver, true, 'G'); // REPORT (1, 'G')
    dense_sweep_garbage_clauses(solver);
    crate::report::report(solver, true, 'C'); // REPORT (1, 'C')
    crate::profile::stop_checked(solver, Prof::collect); // STOP (collect)
}
