// Port of src/resolve.c (kissat 4.0.4).
//
// Resolvent generation for bounded variable elimination.
//
// PORT NOTE: C builds stack-local `clause tmp0/tmp1` headers to view binary
// watches as two-literal clauses; the port uses the `WClause` enum instead
// (Tmp holds the two literals, Ref an arena reference).  `c != &tmp0` becomes
// a match on the variant.
// PORT NOTE (quirk ported): in static generate_resolvents C declares
// `unsigned resolved = *resolved_ptr;` — a TRUNCATING read of the uint64_t
// counter into 32 bits — and `++resolved > limit` promotes back to uint64_t.
// The port keeps the u32 with wrapping increment.
// PORT NOTE: the gates/antecedents stacks are std::mem::take'n around the
// static generate_resolvents calls (C passes interior pointers); nothing in
// between reads or writes them, and the C tail-end CLEAR_STACKs happen after
// restore in the same order.

use crate::internal::{Solver, INVALID};
use crate::reference::Reference;
use crate::watch::{watch_is_binary, watch_lit, watch_ref, Watch};

// static inline unsigned occurrences_literal (kissat *, unsigned lit,
//                                             bool *update)
fn occurrences_literal(solver: &mut Solver, lit: u32, update: &mut bool) -> u32 {
    debug_assert!(!solver.watching);

    let clslim = solver.options.eliminateclslim as u32;

    let v = solver.watches[lit as usize];
    let begin = v.begin;
    let end = v.end;
    let mut q = begin;
    let mut p = begin;

    let mut failed = false;
    let mut res: u32 = 0;

    while p != end {
        // const watch head = *q++ = *p++;
        let head = solver.vectors.stack[p];
        solver.vectors.stack[q] = head;
        q += 1;
        p += 1;
        if watch_is_binary(head) {
            let other = watch_lit(head);
            let value = solver.values[other as usize];
            debug_assert!(value >= 0);
            if value > 0 {
                crate::eliminate::eliminate_binary(solver, lit, other);
                q -= 1;
            } else {
                res += 1;
            }
        } else {
            let ref_ = watch_ref(head);
            if solver.arena.clause(ref_).garbage() {
                q -= 1;
            } else if solver.arena.clause(ref_).size() > clslim {
                failed = true;
                break;
            } else {
                res += 1;
            }
        }
    }
    while p != end {
        solver.vectors.stack[q] = solver.vectors.stack[p];
        q += 1;
        p += 1;
    }
    crate::vector::resize_vector(solver, lit, q - begin); // SET_END_OF_WATCHES
    if failed {
        return u32::MAX; // UINT_MAX
    }
    if q != end {
        *update = true;
    }
    res
}

/// C stack-local `clause tmp` view of a watch (see module PORT NOTE).
#[derive(Clone, Copy)]
enum WClause {
    Tmp([u32; 2]),
    Ref(Reference),
}

// static inline clause *watch_to_clause (kissat *, ward *, clause *tmp,
//                                        unsigned lit, watch)
fn watch_to_clause(lit: u32, watch: Watch) -> WClause {
    if watch_is_binary(watch) {
        WClause::Tmp([lit, watch_lit(watch)])
    } else {
        WClause::Ref(watch_ref(watch))
    }
}

impl WClause {
    fn garbage(&self, solver: &Solver) -> bool {
        match *self {
            WClause::Tmp(_) => false,
            WClause::Ref(ref_) => solver.arena.clause(ref_).garbage(),
        }
    }
    fn size(&self, solver: &Solver) -> u32 {
        match *self {
            WClause::Tmp(_) => 2,
            WClause::Ref(ref_) => solver.arena.clause(ref_).size(),
        }
    }
    fn lit(&self, solver: &Solver, i: u32) -> u32 {
        match *self {
            WClause::Tmp(lits) => lits[i as usize],
            WClause::Ref(ref_) => solver.arena.clause(ref_).lit(i),
        }
    }
}

// static bool generate_resolvents (kissat *, unsigned lit,
//                                  statches *watches0, statches *watches1,
//                                  uint64_t *resolved_ptr, uint64_t limit)
fn generate_resolvents_static(
    solver: &mut Solver,
    lit: u32,
    watches0: &[Watch],
    watches1: &[Watch],
    resolved_ptr: &mut u64,
    limit: u64,
) -> bool {
    let not_lit = crate::literal::not(lit);
    let mut resolved: u32 = *resolved_ptr as u32; // QUIRK: truncating read
    let mut failed = false;

    let clslim = solver.options.eliminateclslim as u64;

    'outer: for &watch0 in watches0 {
        let c = watch_to_clause(lit, watch0);

        if c.garbage(solver) {
            debug_assert!(matches!(c, WClause::Ref(_)));
            continue;
        }

        let mut first_antecedent_satisfied = false;

        let c_size = c.size(solver);
        for i in 0..c_size {
            let other = c.lit(solver, i);
            if other == lit {
                continue;
            }
            let value = solver.values[other as usize];
            if value < 0 {
                continue;
            }
            if value > 0 {
                first_antecedent_satisfied = true;
                if let WClause::Ref(ref_) = c {
                    crate::eliminate::eliminate_clause(solver, ref_, other);
                }
                break;
            }
        }

        if first_antecedent_satisfied {
            continue;
        }

        for i in 0..c_size {
            let other = c.lit(solver, i);
            if other == lit {
                continue;
            }
            debug_assert!(solver.marks[other as usize] == 0);
            solver.marks[other as usize] = 1;
        }

        for &watch1 in watches1 {
            let d = watch_to_clause(not_lit, watch1);

            if d.garbage(solver) {
                debug_assert!(matches!(d, WClause::Ref(_)));
                continue;
            }

            let mut resolvent_satisfied_or_tautological = false;
            let saved = solver.resolvents.len();

            solver.statistics.eliminate_resolutions += 1; // INC

            let d_size = d.size(solver);
            for i in 0..d_size {
                let other = d.lit(solver, i);
                if other == not_lit {
                    continue;
                }
                let value = solver.values[other as usize];
                if value < 0 {
                    continue; // dropping falsified literal
                }
                if value > 0 {
                    if let WClause::Ref(ref_) = d {
                        crate::eliminate::eliminate_clause(solver, ref_, other);
                    }
                    resolvent_satisfied_or_tautological = true;
                    break;
                }
                if solver.marks[other as usize] != 0 {
                    continue; // dropping repeated literal
                }
                let not_other = crate::literal::not(other);
                if solver.marks[not_other as usize] != 0 {
                    resolvent_satisfied_or_tautological = true;
                    break;
                }
                solver.resolvents.push(other);
            }

            if resolvent_satisfied_or_tautological {
                solver.resolvents.truncate(saved); // RESIZE_STACK
                continue;
            }

            resolved = resolved.wrapping_add(1); // ++resolved (unsigned)
            if resolved as u64 > limit {
                failed = true;
                break;
            }

            for i in 0..c_size {
                let other = c.lit(solver, i);
                if other == lit {
                    continue;
                }
                let value = solver.values[other as usize];
                debug_assert!(value <= 0);
                if value < 0 {
                    continue; // dropping falsified literal
                }
                solver.resolvents.push(other);
            }

            let size_resolvent = solver.resolvents.len() - saved;

            if size_resolvent == 0 {
                debug_assert!(!solver.inconsistent);
                solver.inconsistent = true;
                // CHECK_AND_ADD_EMPTY: compiled out (NDEBUG).
                if solver.proof.is_some() {
                    crate::proof::add_empty_to_proof(solver);
                }
                failed = true;
                break;
            }

            if size_resolvent == 1 {
                let unit = solver.resolvents[saved]; // PEEK_STACK
                solver.statistics.eliminate_units += 1; // INC: STATISTIC kept
                crate::assign::learned_unit(solver, unit);
                solver.resolvents.truncate(saved); // RESIZE_STACK
                if solver.marks[unit as usize] <= 0 {
                    continue;
                }
                // first antecedent becomes satisfied
                break;
            }

            if size_resolvent as u64 > clslim {
                failed = true;
                break;
            }

            solver.resolvents.push(INVALID); // PUSH_STACK (INVALID_LIT)
        }

        for i in 0..c_size {
            let other = c.lit(solver, i);
            if other == lit {
                continue;
            }
            debug_assert!(solver.marks[other as usize] == 1);
            solver.marks[other as usize] = 0;
        }

        if failed {
            break 'outer;
        }
    }

    *resolved_ptr = resolved as u64;

    !failed
}

/// Port of `kissat_generate_resolvents`.  The C `unsigned *lit_ptr`
/// out-parameter becomes `lit_ptr: &mut u32`.
pub fn generate_resolvents(solver: &mut Solver, idx: u32, lit_ptr: &mut u32) -> bool {
    let mut lit = crate::literal::lit(idx);
    let mut not_lit = crate::literal::not(lit);

    let mut update = false;
    let mut pure_ = false;
    let mut limit: u64;

    {
        let mut pos_count = occurrences_literal(solver, lit, &mut update);
        let mut neg_count = occurrences_literal(solver, not_lit, &mut update);

        if pos_count > neg_count {
            std::mem::swap(&mut lit, &mut not_lit);
            std::mem::swap(&mut pos_count, &mut neg_count);
        }

        let occlim = solver.options.eliminateocclim as u32;
        limit = pos_count as u64 + neg_count as u64;

        if pos_count != 0 && limit > occlim as u64 {
            return false;
        }

        if pos_count != 0 {
            let bound = solver.bounds.eliminate.additional_clauses as u64;
            limit += bound;
        } else {
            // eliminating pure literal
            pure_ = true;
        }
    }

    *lit_ptr = lit;

    solver.statistics.eliminate_attempted += 1; // INC: STATISTIC kept
    if pure_ {
        return true;
    }

    let gates = crate::gates::find_gates(solver, lit); // !pure && ...

    if solver.values[lit as usize] != 0 {
        crate::print::extremely_verbose(solver, "definition produced unit");
        solver.gates[0].clear();
        solver.gates[1].clear();
        return false;
    }

    let mut failed = false;
    let mut resolved: u64 = 0;

    crate::gates::get_antecedents(solver, lit);

    // See module PORT NOTE on the take/restore of the four stacks.
    let gates0 = std::mem::take(&mut solver.gates[0]);
    let gates1 = std::mem::take(&mut solver.gates[1]);
    let antecedents0 = std::mem::take(&mut solver.antecedents[0]);
    let antecedents1 = std::mem::take(&mut solver.antecedents[1]);

    if gates {
        // resolving gates[0] against antecedents[1] clauses
        if !generate_resolvents_static(solver, lit, &gates0, &antecedents1, &mut resolved, limit)
        {
            failed = true;
        } else {
            // resolving gates[1] against antecedents[0] clauses
            if !generate_resolvents_static(
                solver,
                not_lit,
                &gates1,
                &antecedents0,
                &mut resolved,
                limit,
            ) {
                failed = true;
            } else if solver.resolve_gate {
                // need to resolve gates[0] against gates[1] too
                if !generate_resolvents_static(
                    solver,
                    lit,
                    &gates0,
                    &gates1,
                    &mut resolved,
                    limit,
                ) {
                    failed = true;
                }
            }
        }
    } else {
        // no gate extracted thus resolving all clauses
        if !generate_resolvents_static(
            solver,
            lit,
            &antecedents0,
            &antecedents1,
            &mut resolved,
            limit,
        ) {
            failed = true;
        }
    }

    solver.antecedents[0] = antecedents0;
    solver.antecedents[1] = antecedents1;
    solver.antecedents[0].clear(); // CLEAR_STACK (*antecedents0)
    solver.antecedents[1].clear(); // CLEAR_STACK (*antecedents1)

    if failed {
        solver.resolvents.clear();
        if update {
            crate::eliminate::update_variable_score(solver, idx);
        }
    }

    let _ = resolved; // LOG only

    solver.gates[0] = gates0;
    solver.gates[1] = gates1;
    solver.gates[0].clear(); // CLEAR_STACK (*gates0)
    solver.gates[1].clear(); // CLEAR_STACK (*gates1)

    !failed
}
