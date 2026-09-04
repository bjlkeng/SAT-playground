// Port of src/transitive.c (kissat 4.0.4).
//
// Transitive reduction of the binary implication graph.
//
// PORT NOTES:
//  - C iterates watch lists via `watch *` pointers; the port uses word
//    offsets into solver.vectors.stack.  The C `if (p == q) continue;`
//    pointer-identity test (which skips the currently probed src watch when
//    the propagated literal's watch list IS the src list, i.e. not_lit ==
//    src) becomes an offset equality test — identical semantics because both
//    index the same shared stack.
//  - REMOVE_WATCHES (*dst_watches, dst_watch) is
//    kissat_remove_from_vector on the raw watch word -> crate::vector::
//    remove_from_vector (solver, dst, word).
//  - Statistics tiers: transitive_ticks is a COUNTER; transitive_probes /
//    transitive_propagations / transitive_reduced / transitive_reductions /
//    transitive_units are METRIC — compiled out, INC/ADD sites dropped.
//    ADD (propagations)/ADD (probing_ticks) are COUNTERs, ADD (ticks) is a
//    kept STATISTIC field.
//  - less_stable_transitive / less_focused_transitive apply IDX() to the
//    *variable indices* stored in the probes stack (halving them) — a kissat
//    quirk ported as-is.
//  - solver->transitive_reducing flag is !NDEBUG/METRICS-only — omitted.

use crate::internal::Solver;
use crate::literal::ILLEGAL_LIT;
use crate::profile::Prof;
use crate::reference::INVALID_REF;
use crate::terminated;
use crate::utilities::percent;
use crate::watch::{binary_watch, watch_is_binary, watch_lit, Watch};

// static transitive_assign
fn transitive_assign(solver: &mut Solver, lit: u32) {
    let not_lit = crate::literal::not(lit);
    debug_assert!(solver.values[lit as usize] == 0);
    debug_assert!(solver.values[not_lit as usize] == 0);
    solver.values[lit as usize] = 1;
    solver.values[not_lit as usize] = -1;
    solver.trail.push(lit); // PUSH_ARRAY (solver->trail, lit)
}

// static transitive_backtrack
fn transitive_backtrack(solver: &mut Solver, saved: usize) {
    let mut end_trail = solver.trail.len();
    debug_assert!(saved <= end_trail);

    while end_trail != saved {
        end_trail -= 1;
        let lit = solver.trail[end_trail];
        let not_lit = crate::literal::not(lit);
        debug_assert!(solver.values[lit as usize] > 0);
        debug_assert!(solver.values[not_lit as usize] < 0);
        solver.values[lit as usize] = 0;
        solver.values[not_lit as usize] = 0;
    }

    solver.trail.truncate(saved); // SET_END_OF_ARRAY (solver->trail, saved)
    solver.propagate = saved;
    solver.level = 0;
}

// static prioritize_binaries
fn prioritize_binaries(solver: &mut Solver) {
    debug_assert!(solver.watching);
    let mut large: Vec<Watch> = Vec::new(); // statches large
    let lits = solver.lits();
    for lit in 0..lits {
        debug_assert!(large.is_empty());
        let v = solver.watches[lit as usize];
        let begin_watches = v.begin;
        let end_watches = v.end;
        let mut q = begin_watches;
        let mut p = begin_watches;
        while p != end_watches {
            let head = solver.vectors.stack[p];
            solver.vectors.stack[q] = head;
            q += 1;
            p += 1;
            if watch_is_binary(head) {
                continue;
            }
            let tail = solver.vectors.stack[p];
            p += 1;
            large.push(head);
            large.push(tail);
            q -= 1;
        }
        for &w in large.iter() {
            solver.vectors.stack[q] = w;
            q += 1;
        }
        debug_assert!(q == end_watches);
        large.clear();
    }
    drop(large); // RELEASE_STACK (large)
}

// static transitive_reduce
fn transitive_reduce(
    solver: &mut Solver,
    src: u32,
    limit: u64,
    reduced_ptr: &mut u64,
    units: &mut u32,
) -> bool {
    let mut res = false;
    debug_assert!(solver.values[src as usize] == 0);
    let src_vector = solver.watches[src as usize];
    let begin_src = src_vector.begin;
    let end_src = src_vector.end;
    let size_src_watches = (end_src - begin_src) as u64;
    let src_ticks = 1 + crate::utilities::cache_lines(size_src_watches, 4);
    solver.statistics.transitive_ticks += src_ticks; // ADD (transitive_ticks, ...)
    solver.statistics.probing_ticks += src_ticks; // ADD (probing_ticks, ...)
    solver.statistics.ticks += src_ticks; // ADD (ticks, ...)
    // INC (transitive_probes): METRIC, compiled out.
    let not_src = crate::literal::not(src);
    let mut reduced: u32 = 0;
    let mut failed = false;
    let mut p = begin_src;
    while p != end_src {
        let src_watch = solver.vectors.stack[p];
        if !watch_is_binary(src_watch) {
            break;
        }
        let dst = watch_lit(src_watch);
        if dst < src {
            p += 1;
            continue;
        }
        if solver.values[dst as usize] != 0 {
            p += 1;
            continue;
        }
        debug_assert!(solver.propagate == solver.trail.len()); // kissat_propagated
        let saved = solver.propagate;
        debug_assert!(solver.level == 0);
        solver.level = 1;
        transitive_assign(solver, not_src);
        let mut transitive = false;
        let mut inner_ticks: u64 = 0;
        let mut propagate = solver.propagate;
        while !transitive && !failed && propagate != solver.trail.len() {
            let lit = solver.trail[propagate];
            propagate += 1;
            debug_assert!(solver.values[lit as usize] > 0);
            let not_lit = crate::literal::not(lit);
            let lit_vector = solver.watches[not_lit as usize];
            let begin_lit = lit_vector.begin;
            let end_lit = lit_vector.end;
            let size_lit_watches = (end_lit - begin_lit) as u64;
            inner_ticks += 1 + crate::utilities::cache_lines(size_lit_watches, 4);
            let mut q = begin_lit;
            while q != end_lit {
                if p == q {
                    q += 1;
                    continue;
                }
                let lit_watch = solver.vectors.stack[q];
                if !watch_is_binary(lit_watch) {
                    break;
                }
                if not_lit == src && watch_lit(lit_watch) == ILLEGAL_LIT {
                    q += 1;
                    continue;
                }
                let other = watch_lit(lit_watch);
                if other == dst {
                    transitive = true;
                    break;
                }
                let value = solver.values[other as usize];
                if value < 0 {
                    failed = true;
                    break;
                }
                if value == 0 {
                    transitive_assign(solver, other);
                }
                q += 1;
            }
        }

        debug_assert!(solver.probing);

        debug_assert!(solver.propagate <= propagate);
        let propagated = (propagate - solver.propagate) as u64;

        // ADD (transitive_propagations, ...): METRIC, compiled out.
        // ADD (probing_propagations, ...): METRIC, compiled out.
        solver.statistics.propagations += propagated; // ADD (propagations, ...)

        solver.statistics.transitive_ticks += inner_ticks;
        solver.statistics.probing_ticks += inner_ticks;
        solver.statistics.ticks += inner_ticks;

        transitive_backtrack(solver, saved);

        if transitive {
            // INC (transitive_reduced): METRIC, compiled out.
            debug_assert!(watch_lit(src_watch) == dst);
            let dst_watch = binary_watch(src); // dst_watch.binary.lit = src
            crate::vector::remove_from_vector(solver, dst, dst_watch); // REMOVE_WATCHES
            crate::clause::delete_binary(solver, src, dst);
            // p->binary.lit = ILLEGAL_LIT (binary flag kept)
            solver.vectors.stack[p] = binary_watch(ILLEGAL_LIT);
            reduced += 1;
            res = true;
        }

        if failed {
            break;
        }
        if solver.statistics.transitive_ticks > limit {
            break;
        }
        if terminated!(solver, transitive_terminated_1) {
            break;
        }
        p += 1;
    }

    if reduced != 0 {
        *reduced_ptr += reduced as u64;
        debug_assert!(begin_src == solver.watches[src as usize].begin);
        debug_assert!(end_src == solver.watches[src as usize].end);
        let mut q = begin_src;
        let mut p = begin_src;
        while p != end_src {
            let src_watch = solver.vectors.stack[p];
            solver.vectors.stack[q] = src_watch;
            q += 1;
            if !watch_is_binary(src_watch) {
                p += 1;
                solver.vectors.stack[q] = solver.vectors.stack[p];
                q += 1;
                p += 1;
                continue;
            }
            p += 1;
            if watch_lit(src_watch) == ILLEGAL_LIT {
                q -= 1;
            }
        }
        debug_assert!(end_src - q == reduced as usize);
        crate::vector::resize_vector(solver, src, q - begin_src); // SET_END_OF_WATCHES
    }

    if failed {
        // INC (transitive_units): METRIC, compiled out.
        *units += 1;
        res = true;

        crate::assign::learned_unit(solver, src);

        debug_assert!(solver.level == 0);
        let _ = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
    }

    res
}

// static inline less_stable_transitive
// PORT NOTE: IDX() applied to variable indices — quirk ported (see header).
#[inline]
fn less_stable_transitive(
    flags: &[crate::flags::Flags],
    scores: &crate::heap::Heap,
    a: u32,
    b: u32,
) -> bool {
    let i = crate::literal::idx(a);
    let j = crate::literal::idx(b);
    let p = flags[i as usize].transitive();
    let q = flags[j as usize].transitive();
    if !p && q {
        return true;
    }
    if p && !q {
        return false;
    }
    let s = crate::heap::get_heap_score(scores, i);
    let t = crate::heap::get_heap_score(scores, j);
    if s < t {
        return true;
    }
    if s > t {
        return false;
    }
    i < j
}

// static inline less_focused_transitive
#[inline]
fn less_focused_transitive(
    flags: &[crate::flags::Flags],
    links: &[crate::queue::Links],
    a: u32,
    b: u32,
) -> bool {
    let i = crate::literal::idx(a);
    let j = crate::literal::idx(b);
    let p = flags[i as usize].transitive();
    let q = flags[j as usize].transitive();
    if !p && q {
        return true;
    }
    if p && !q {
        return false;
    }
    let s = links[i as usize].stamp;
    let t = links[j as usize].stamp;
    s < t
}

// static sort_stable_transitive / sort_focused_transitive / sort_transitive
fn sort_transitive(solver: &mut Solver, probes: &mut Vec<u32>) {
    let mut sorter = std::mem::take(&mut solver.sorter);
    // SORT_STACK carries START/STOP (sort) — hoisted per sort.rs convention.
    crate::profile::start_checked(solver, Prof::sort);
    if solver.stable {
        let flags = &solver.flags;
        let scores = &solver.scores;
        crate::sort::sort_stack(&mut sorter, probes, |&a, &b| {
            less_stable_transitive(flags, scores, a, b)
        });
    } else {
        let flags = &solver.flags;
        let links = &solver.links;
        crate::sort::sort_stack(&mut sorter, probes, |&a, &b| {
            less_focused_transitive(flags, links, a, b)
        });
    }
    crate::profile::stop_checked(solver, Prof::sort);
    solver.sorter = sorter;
}

// static schedule_transitive
fn schedule_transitive(solver: &mut Solver, probes: &mut Vec<u32>) {
    debug_assert!(probes.is_empty());
    for idx in 0..solver.vars {
        if solver.flags[idx as usize].active() {
            probes.push(idx);
        }
    }
    sort_transitive(solver, probes);
    crate::print::very_verbose(
        solver,
        format_args!("scheduled {} transitive probes", probes.len()),
    );
}

/// Port of `kissat_transitive_reduction`.
pub fn transitive_reduction(solver: &mut Solver) {
    if solver.inconsistent {
        return;
    }
    debug_assert!(solver.watching);
    debug_assert!(solver.probing);
    debug_assert!(solver.level == 0);
    if solver.options.transitive == 0 {
        return;
    }
    if terminated!(solver, transitive_terminated_2) {
        return;
    }
    crate::profile::start_checked(solver, Prof::transitive); // START (transitive)
    // INC (transitive_reductions): METRIC, compiled out.
    prioritize_binaries(solver);
    let mut success = false;
    let mut reduced: u64 = 0;
    let mut units: u32 = 0;

    let limit = crate::set_effort_limit!(solver, transitive, transitiveeffort, transitive_ticks);

    // #ifndef QUIET
    let active = solver.active;
    let old_ticks = solver.statistics.transitive_ticks;
    crate::print::extremely_verbose(
        solver,
        format_args!("starting with {} transitive ticks", old_ticks),
    );
    let mut probed: u32 = 0;

    let mut probes: Vec<u32> = Vec::new();
    schedule_transitive(solver, &mut probes);
    let mut terminate = false;
    while !terminate && !probes.is_empty() {
        let idx = probes.pop().unwrap(); // POP_STACK (probes)
        solver.flags[idx as usize].set_transitive(false);
        if !solver.flags[idx as usize].active() {
            continue;
        }
        let mut sign = 0;
        while !terminate && sign < 2 {
            let lit = 2 * idx + sign;
            sign += 1;
            if solver.values[lit as usize] != 0 {
                continue;
            }
            probed += 1; // #ifndef QUIET
            if transitive_reduce(solver, lit, limit, &mut reduced, &mut units) {
                success = true;
            }
            if solver.inconsistent {
                terminate = true;
            } else if solver.statistics.transitive_ticks > limit {
                terminate = true;
            } else if terminated!(solver, transitive_terminated_3) {
                terminate = true;
            }
        }
    }
    let remain = probes.len();
    if remain != 0 {
        if solver.options.transitivekeep == 0 {
            crate::print::very_verbose(
                solver,
                format_args!("dropping remaining {} transitive candidates", remain),
            );
            while let Some(idx) = probes.pop() {
                solver.flags[idx as usize].set_transitive(false);
            }
        }
    } else {
        crate::print::very_verbose(solver, "transitive reduction complete");
    }
    drop(probes); // RELEASE_STACK (probes)

    // #ifndef QUIET
    let new_ticks = solver.statistics.transitive_ticks;
    let delta_ticks = new_ticks - old_ticks;
    crate::print::extremely_verbose(
        solver,
        format_args!(
            "finished at {} after {} transitive ticks",
            new_ticks, delta_ticks
        ),
    );
    crate::print::phase(
        solver,
        "transitive",
        solver.statistics.probings, // GET (probings)
        format_args!(
            "probed {} ({:.0}%): reduced {}, units {}",
            probed,
            percent(probed as f64, (2 * active) as f64),
            reduced,
            units
        ),
    );

    crate::report::report(solver, !success, 't'); // REPORT (!success, 't')
    crate::profile::stop_checked(solver, Prof::transitive); // STOP (transitive)
}
