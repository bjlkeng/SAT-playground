// Port of src/reduce.c (kissat 4.0.4).
//
// PORT NOTES (ranking):
//  - C builds each reducible's rank as
//        const uint64_t negative_size = ~c->size;          // u32 NOT, zero-ext
//        const uint64_t negative_glue = ~c->glue;          // int NOT, SIGN-ext
//        red.rank = negative_size | (negative_glue << 32);
//    The glue bitfield promotes to int, so ~glue sign-extends to u64; after
//    `<< 32` only its low 32 bits (the 32-bit pattern of !glue) survive in
//    the high half, and the sign-extension bits shift out.  The exact
//    equivalent used here: rank = (!size as u64) | ((!glue as u64) << 32)
//    with size/glue as u32 — bit-for-bit identical to the C value.
//    Radix-sorting ascending on this rank orders by size descending then
//    glue descending, i.e. least-useful first, with kissat's radix sort
//    stability (equal ranks keep arena order).
//  - RADIX_STACK's embedded START/STOP (radix) profile hooks (level 4) are
//    hoisted around the call per sort.rs conventions.
//  - GET (moved) is METRIC (prints as no-count u64::MAX); reductions is a
//    COUNTER.  clauses_reduced* are STATISTIC-tier fields (kept).
//  - reduce.c computes `bytes_to_sweep = sizeof (word) * words_to_sweep`
//    with word == 8 bytes although arena wards are 16 bytes — a C quirk in a
//    message-only value, ported as-is.

use crate::internal::Solver;
use crate::profile::Prof;
use crate::reference::{Reference, INVALID_REF};

/// Port of `kissat_reducing`.
pub fn reducing(solver: &Solver) -> bool {
    if solver.options.reduce == 0 {
        return false;
    }
    if solver.statistics.clauses_redundant == 0 {
        return false;
    }
    if solver.statistics.conflicts < solver.limits.reduce.conflicts {
        return false;
    }
    true
}

// struct reducible
#[derive(Clone, Copy)]
struct Reducible {
    rank: u64,
    ref_: u32,
}

// static collect_reducibles
fn collect_reducibles(
    solver: &mut Solver,
    reds: &mut Vec<Reducible>,
    start_ref: Reference,
) -> bool {
    debug_assert!(start_ref != INVALID_REF);
    debug_assert!((start_ref as u64) <= solver.arena.size_wards());
    let end = solver.arena.size_wards();
    let mut start = start_ref as u64;
    debug_assert!(start < end);
    while start != end && !solver.arena.clause(start as Reference).redundant() {
        start = solver.arena.next_clause_ref(start as Reference) as u64;
    }
    if start == end {
        solver.first_reducible = INVALID_REF;
        return false;
    }
    let redundant = start as Reference;
    solver.first_reducible = redundant;
    let tier1 = solver.tier1();
    let tier2 = tier1.max(solver.tier2()); // MAX (tier1, TIER2)
    debug_assert!(tier1 <= tier2);
    let mut c = start;
    while c != end {
        let ref_ = c as Reference;
        let next = solver.arena.next_clause_ref(ref_) as u64;
        c = next;
        let (redundant, garbage, used, reason, glue, size) = {
            let cl = solver.arena.clause(ref_);
            (
                cl.redundant(),
                cl.garbage(),
                cl.used(),
                cl.reason(),
                cl.glue(),
                cl.size(),
            )
        };
        if !redundant {
            continue;
        }
        if garbage {
            continue;
        }
        if used != 0 {
            solver.arena.clause_mut(ref_).set_used(used - 1);
        }
        if reason {
            continue;
        }
        if glue <= tier1 && used != 0 {
            continue;
        }
        if glue <= tier2 && used >= crate::clause::MAX_USED - 1 {
            continue;
        }
        let rank: u64 = (!size as u64) | ((!glue as u64) << 32);
        reds.push(Reducible { rank, ref_ });
    }
    if reds.is_empty() {
        crate::print::phase(
            solver,
            "reduce",
            solver.statistics.reductions, // GET (reductions)
            "did not find any reducible redundant clause",
        );
        return false;
    }
    true
}

// static sort_reducibles: RADIX_STACK (reducible, uint64_t, *reds, USEFULNESS)
fn sort_reducibles(solver: &mut Solver, reds: &mut Vec<Reducible>) {
    crate::profile::start_checked(solver, Prof::radix); // START (radix)
    crate::sort::radix_stack::<Reducible, u64, _>(reds, |r| r.rank);
    crate::profile::stop_checked(solver, Prof::radix); // STOP (radix)
}

// static mark_less_useful_clauses_as_garbage
fn mark_less_useful_clauses_as_garbage(solver: &mut Solver, reds: &[Reducible]) {
    let high = solver.options.reducehigh as f64 * 0.1;
    let low = solver.options.reducelow as f64 * 0.1;
    let percent = if low < high {
        let delta = high - low;
        high - delta / ((solver.statistics.reductions + 9) as f64).log10()
    } else {
        low
    };
    let fraction = percent / 100.0;
    let size = reds.len();
    let mut target = (size as f64 * fraction) as usize; // size_t = size * fraction
    // #ifndef QUIET
    {
        let clauses =
            solver.statistics.clauses_irredundant + solver.statistics.clauses_redundant;
        crate::print::phase(
            solver,
            "reduce",
            solver.statistics.reductions, // GET (reductions)
            format_args!(
                "reducing {} ({:.0}%) out of {} ({:.0}%) reducible clauses",
                target,
                crate::format::percent(target as f64, size as f64),
                size,
                crate::format::percent(size as f64, clauses as f64)
            ),
        );
    }
    let mut reduced: u64 = 0;
    let mut reduced1: u64 = 0;
    let mut reduced2: u64 = 0;
    let mut reduced3: u64 = 0;
    let tier1 = solver.tier1();
    let tier2 = solver.tier2();
    for p in reds.iter() {
        // for (p = begin; p != end && target--; p++)
        if target == 0 {
            break;
        }
        target -= 1;
        let ref_ = p.ref_;
        let glue = {
            let c = solver.arena.clause(ref_);
            debug_assert!(!c.garbage());
            debug_assert!(!c.reason());
            debug_assert!(c.redundant());
            c.glue()
        };
        crate::clause::mark_clause_as_garbage(solver, ref_);
        reduced += 1;
        if glue <= tier1 {
            reduced1 += 1;
        } else if glue <= tier2 {
            reduced2 += 1;
        } else {
            reduced3 += 1;
        }
    }
    // ADD (...): STATISTIC-tier fields (kept, never printed).
    solver.statistics.clauses_reduced_tier1 += reduced1;
    solver.statistics.clauses_reduced_tier2 += reduced2;
    solver.statistics.clauses_reduced_tier3 += reduced3;
    solver.statistics.clauses_reduced += reduced;
}

/// Port of `kissat_reduce`.
pub fn reduce(solver: &mut Solver) -> i32 {
    crate::profile::start_checked(solver, Prof::reduce); // START (reduce)
    solver.statistics.reductions += 1; // INC (reductions)
    crate::print::phase(
        solver,
        "reduce",
        solver.statistics.reductions,
        format_args!(
            "reduce limit {} hit after {} conflicts",
            solver.limits.reduce.conflicts, solver.statistics.conflicts
        ),
    );
    crate::tiers::compute_and_set_tier_limits(solver);
    let compact = crate::collect::compacting(solver);
    let start: Reference = if compact { 0 } else { solver.first_reducible };
    if start != INVALID_REF {
        // #ifndef QUIET
        {
            let arena_size = solver.arena.size_wards();
            let words_to_sweep = arena_size - start as u64;
            let bytes_to_sweep = 8 * words_to_sweep; // sizeof (word) — C quirk
            crate::print::phase(
                solver,
                "reduce",
                solver.statistics.reductions,
                format_args!("reducing clauses after offset {} in arena", start),
            );
            let bytes_str = crate::format::format_bytes(&mut solver.format, bytes_to_sweep);
            crate::print::phase(
                solver,
                "reduce",
                solver.statistics.reductions,
                format_args!(
                    "reducing {} words {} {:.0}%",
                    words_to_sweep,
                    bytes_str,
                    crate::format::percent(words_to_sweep as f64, arena_size as f64)
                ),
            );
        }
        if crate::trail::flush_and_mark_reason_clauses(solver, start) {
            let mut reds: Vec<Reducible> = Vec::new(); // INIT_STACK (reds)
            if collect_reducibles(solver, &mut reds, start) {
                sort_reducibles(solver, &mut reds);
                mark_less_useful_clauses_as_garbage(solver, &reds);
                drop(reds); // RELEASE_STACK (reds)
                crate::collect::sparse_collect(solver, compact, start);
            } else if compact {
                crate::collect::sparse_collect(solver, compact, start);
            } else {
                crate::trail::unmark_reason_clauses(solver, start);
            }
        } else {
            debug_assert!(solver.inconsistent);
        }
    } else {
        crate::print::phase(
            solver,
            "reduce",
            solver.statistics.reductions,
            "nothing to reduce",
        );
    }
    crate::classify::classify(solver);
    crate::update_conflict_limit!(
        solver,
        reduce,
        reduceint,
        reductions,
        |n| crate::kimits::sqrt(n),
        false
    );
    solver.last.conflicts.reduce = solver.statistics.conflicts;
    crate::report::report(solver, false, '-'); // REPORT (0, '-')
    crate::profile::stop_checked(solver, Prof::reduce); // STOP (reduce)
    if solver.inconsistent {
        20
    } else {
        0
    }
}
