// Port of src/factor.c (kissat 4.0.4).
//
// Bounded variable addition (clause factoring): pick a factor literal, grow
// quotients of clauses sharing it, and when the best quotient reduces the
// clause count beyond the eliminate bound, introduce a fresh variable.
//
// PORT NOTE (quotient list): C heap-allocates `struct quotient` in a doubly
// linked list built strictly by appending (new_quotient), so ids equal
// creation order.  Ported as a Vec<Quotient> where the index IS the id;
// `prev` is index-1 and `next` index+1.  All C traversals (forward from
// `first`, backward via `prev`) become index loops.  `struct quotient`'s
// `matched` field is never used in C and is omitted.
//
// PORT NOTE (eagerly_remove_watch): C memmoves the watch out and just moves
// the vector end pointer down (no `usable` accounting, unlike
// kissat_remove_from_vector) — ported identically by decrementing
// `Vector::end` directly.
//
// PORT NOTE (count/score resizing): C reallocs `count` and the per-hop
// `score` arrays with doubling capacity and zero/-1-fills the fresh tail;
// entries in [size, allocated) are never read, so Vec::resize with the same
// fill values is behaviourally identical.
//
// PORT NOTE (dense mode): kissat_enter_dense_mode / kissat_resume_sparse_mode
// route through crate::dense (stub in stubs.rs until the dense wave lands;
// kissat_factor is only reachable from the eliminate driver, also pending).

use crate::internal::{Solver, INVALID};
use crate::literal::{idx, lit as make_lit, negated, not};
use crate::profile::Prof;
use crate::reference::Reference;
use crate::terminated;
use crate::utilities::{cache_lines, percent};
use crate::watch::{binary_watch, watch_is_binary, watch_lit, watch_ref, Watch};

const FACTOR: i8 = 1;
const QUOTIENT: i8 = 2;
const NOUNTED: i8 = 4;

// struct quotient
struct Quotient {
    factor: u32,
    clauses: Vec<Watch>,  // statches clauses;
    matches: Vec<usize>,  // sizes matches;
}

// struct scores
struct Scores {
    score: Vec<f64>,
    scored: Vec<u32>, // unsigneds scored;
}

// struct factoring
struct Factoring {
    size: usize,
    allocated: usize,
    initial: u32,
    count: Vec<u32>,
    scores: Vec<Scores>, // scores *scores; (hops entries)
    hops: u32,
    bound: u32, // solver->bounds.eliminate.additional_clauses
    fresh: Vec<u32>,
    counted: Vec<u32>,
    nounted: Vec<u32>,
    qlauses: Vec<Reference>,
    limit: u64,
    quotients: Vec<Quotient>,
    schedule: crate::heap::Heap,
}

// static void init_factoring
fn init_factoring(solver: &mut Solver, limit: u64) -> Factoring {
    let lits = solver.lits() as usize;
    let mut hops = 0u32;
    if solver.options.factorstructural != 0 {
        hops = solver.options.factorhops as u32;
    }
    let mut scores = Vec::new();
    if hops != 0 {
        for _ in 0..hops {
            scores.push(Scores {
                score: vec![-1.0; solver.vars as usize],
                scored: Vec::new(),
            });
        }
    }
    #[cfg(debug_assertions)]
    for l in 0..lits {
        debug_assert!(solver.marks[l] == 0);
    }
    Factoring {
        size: lits,
        allocated: lits,
        initial: lits as u32,
        count: vec![0u32; lits],
        scores,
        hops,
        bound: solver.bounds.eliminate.additional_clauses,
        fresh: Vec::new(),
        counted: Vec::new(),
        nounted: Vec::new(),
        qlauses: Vec::new(),
        limit,
        quotients: Vec::new(),
        schedule: crate::heap::Heap::new(),
    }
}

// static void release_quotients
fn release_quotients(solver: &mut Solver, factoring: &mut Factoring) {
    for q in &factoring.quotients {
        let factor = q.factor;
        debug_assert!(solver.marks[factor as usize] == FACTOR);
        solver.marks[factor as usize] = 0;
    }
    factoring.quotients.clear();
    let hops = factoring.hops;
    if hops != 0 {
        for i in 0..hops as usize {
            let scores = &mut factoring.scores[i];
            while let Some(v) = scores.scored.pop() {
                scores.score[v as usize] = -1.0;
            }
        }
    }
}

// static void release_factoring
fn release_factoring(solver: &mut Solver, factoring: &mut Factoring) {
    debug_assert!(solver.analyzed.is_empty());
    debug_assert!(factoring.counted.is_empty());
    debug_assert!(factoring.nounted.is_empty());
    debug_assert!(factoring.qlauses.is_empty());
    factoring.count = Vec::new();
    factoring.counted = Vec::new();
    factoring.nounted = Vec::new();
    factoring.fresh = Vec::new();
    factoring.qlauses = Vec::new();
    release_quotients(solver, factoring);
    crate::heap::release_heap(&mut factoring.schedule);
    factoring.scores = Vec::new();
    #[cfg(debug_assertions)]
    for l in 0..solver.lits() as usize {
        debug_assert!(solver.marks[l] == 0);
    }
}

// static void update_candidate
fn update_candidate(solver: &mut Solver, factoring: &mut Factoring, lit: u32) {
    let cands = &mut factoring.schedule;
    let size = solver.watches[lit as usize].size();
    if size > 1 {
        crate::heap::adjust_heap(cands, lit);
        crate::heap::update_heap(cands, lit, size as f64);
        if !crate::heap::heap_contains(cands, lit) {
            crate::heap::push_heap(cands, lit);
        }
    } else if crate::heap::heap_contains(cands, lit) {
        crate::heap::pop_heap(cands, lit);
    }
}

// static void schedule_factorization
fn schedule_factorization(solver: &mut Solver, factoring: &mut Factoring) {
    for i in 0..solver.vars {
        if solver.flags[i as usize].active {
            let f_factor = solver.flags[i as usize].factor;
            let l = make_lit(i);
            let not_lit = not(l);
            if f_factor & 1 != 0 {
                update_candidate(solver, factoring, l);
            }
            if f_factor & 2 != 0 {
                update_candidate(solver, factoring, not_lit);
            }
        }
    }
    let size_cands = crate::heap::size_heap(&factoring.schedule);
    crate::print::very_verbose(
        solver,
        format_args!(
            "scheduled {} factorization candidate literals {:.0} %",
            size_cands,
            percent(size_cands as f64, solver.lits() as f64)
        ),
    );
}

// static quotient *new_quotient — returns the index (== C id).
fn new_quotient(solver: &mut Solver, factoring: &mut Factoring, factor: u32) -> usize {
    debug_assert!(solver.marks[factor as usize] == 0);
    solver.marks[factor as usize] = FACTOR;
    let res = factoring.quotients.len();
    factoring.quotients.push(Quotient {
        factor,
        clauses: Vec::new(),
        matches: Vec::new(),
    });
    res
}

// static size_t first_factor
fn first_factor(solver: &mut Solver, factoring: &mut Factoring, factor: u32) -> usize {
    debug_assert!(factoring.quotients.is_empty());
    let quotient = new_quotient(solver, factoring, factor);
    let mut ticks: u64 = 0;
    let v = solver.watches[factor as usize];
    for wi in v.begin..v.end {
        let watch = solver.vectors.stack[wi];
        factoring.quotients[quotient].clauses.push(watch);
        ticks += 1;
    }
    let res = factoring.quotients[quotient].clauses.len();
    debug_assert!(res > 1);
    solver.statistics.factor_ticks += ticks;
    res
}

// static void clear_nounted
fn clear_nounted(solver: &mut Solver, nounted: &mut Vec<u32>) {
    for &l in nounted.iter() {
        debug_assert!(solver.marks[l as usize] & NOUNTED != 0);
        solver.marks[l as usize] &= !NOUNTED;
    }
    nounted.clear();
}

// static void clear_qlauses
fn clear_qlauses(solver: &mut Solver, qlauses: &mut Vec<Reference>) {
    for &ref_ in qlauses.iter() {
        let mut c = solver.arena.clause_mut(ref_);
        debug_assert!(c.quotient());
        c.set_quotient(false);
    }
    qlauses.clear();
}

// static double distinct_paths
fn distinct_paths(
    solver: &mut Solver,
    factoring: &mut Factoring,
    src_lit: u32,
    dst_idx: u32,
    hops: u32,
) -> f64 {
    let src_idx = idx(src_lit);
    let matched = src_idx == dst_idx;
    if hops == 0 {
        return if matched { 1.0 } else { 0.0 };
    }
    let next_hops = hops - 1;
    {
        let scores = &factoring.scores[next_hops as usize];
        let res = scores.score[src_idx as usize];
        if res >= 0.0 {
            return res;
        }
    }
    let mut res: f64 = if matched { 1.0 } else { 0.0 };
    for sign in 0..2u32 {
        let signed_src_lit = src_lit ^ sign;
        let v = solver.watches[signed_src_lit as usize];
        let mut ticks: u64 = 1 + cache_lines((v.end - v.begin) as u64, 4);
        for wi in v.begin..v.end {
            let watch = solver.vectors.stack[wi];
            if watch_is_binary(watch) {
                let other = watch_lit(watch);
                res += distinct_paths(solver, factoring, other, dst_idx, next_hops);
            } else {
                let ref_ = watch_ref(watch);
                ticks += 1;
                let size = solver.arena.clause(ref_).size();
                for j in 0..size {
                    let other = solver.arena.clause(ref_).lit(j);
                    if other != signed_src_lit {
                        res += distinct_paths(solver, factoring, other, dst_idx, next_hops);
                    }
                }
            }
        }
        solver.statistics.factor_ticks += ticks;
    }
    debug_assert!(res >= 0.0);
    let scores = &mut factoring.scores[next_hops as usize];
    scores.score[src_idx as usize] = res;
    scores.scored.push(src_idx);
    res
}

// static double structural_score
fn structural_score(solver: &mut Solver, factoring: &mut Factoring, lit: u32) -> f64 {
    debug_assert!(!factoring.quotients.is_empty());
    let first_factor = factoring.quotients[0].factor;
    let first_factor_idx = idx(first_factor);
    let hops = factoring.hops;
    distinct_paths(solver, factoring, lit, first_factor_idx, hops)
}

// static double watches_score
fn watches_score(solver: &Solver, lit: u32) -> f64 {
    solver.watches[lit as usize].size() as f64
}

// static double tied_next_factor_score
fn tied_next_factor_score(solver: &mut Solver, factoring: &mut Factoring, lit: u32) -> f64 {
    if factoring.hops != 0 {
        structural_score(solver, factoring, lit)
    } else {
        watches_score(solver, lit)
    }
}

// static unsigned next_factor
fn next_factor(solver: &mut Solver, factoring: &mut Factoring) -> (u32, u32) {
    debug_assert!(!factoring.quotients.is_empty());
    let last_quotient = factoring.quotients.len() - 1;
    debug_assert!(factoring.counted.is_empty());
    debug_assert!(factoring.qlauses.is_empty());
    let initial = factoring.initial;
    let mut ticks: u64 = 1
        + cache_lines(
            factoring.quotients[last_quotient].clauses.len() as u64,
            4,
        );
    let num_clauses = factoring.quotients[last_quotient].clauses.len();
    'clauses: for ci in 0..num_clauses {
        let quotient_watch = factoring.quotients[last_quotient].clauses[ci];
        if watch_is_binary(quotient_watch) {
            let q = watch_lit(quotient_watch);
            let qv = solver.watches[q as usize];
            ticks += 1 + cache_lines((qv.end - qv.begin) as u64, 4);
            for wi in qv.begin..qv.end {
                let next_watch = solver.vectors.stack[wi];
                if !watch_is_binary(next_watch) {
                    continue;
                }
                let next = watch_lit(next_watch);
                if next > initial {
                    continue;
                }
                if solver.marks[next as usize] & FACTOR != 0 {
                    continue;
                }
                let next_idx = idx(next);
                if !solver.flags[next_idx as usize].active {
                    continue;
                }
                if factoring.count[next as usize] == 0 {
                    factoring.counted.push(next);
                }
                factoring.count[next as usize] += 1;
            }
        } else {
            let c_ref = watch_ref(quotient_watch);
            debug_assert!(!solver.arena.clause(c_ref).quotient());
            let mut min_lit = INVALID;
            let mut factors = 0u32;
            let mut min_size = 0usize;
            ticks += 1;
            let c_size = solver.arena.clause(c_ref).size();
            for j in 0..c_size {
                let other = solver.arena.clause(c_ref).lit(j);
                if solver.marks[other as usize] & FACTOR != 0 {
                    let prev = factors;
                    factors += 1;
                    if prev != 0 {
                        break;
                    }
                } else {
                    debug_assert!(solver.marks[other as usize] & QUOTIENT == 0);
                    solver.marks[other as usize] |= QUOTIENT;
                    let other_size = solver.watches[other as usize].size();
                    if min_lit != INVALID && min_size <= other_size {
                        continue;
                    }
                    min_lit = other;
                    min_size = other_size;
                }
            }
            debug_assert!(factors > 0);
            if factors == 1 {
                debug_assert!(min_lit != INVALID);
                let c_size_field = solver.arena.clause(c_ref).size();
                debug_assert!(factoring.nounted.is_empty());
                let mv = solver.watches[min_lit as usize];
                ticks += 1 + cache_lines((mv.end - mv.begin) as u64, 4);
                'min_watches: for wi in mv.begin..mv.end {
                    let min_watch = solver.vectors.stack[wi];
                    if watch_is_binary(min_watch) {
                        continue;
                    }
                    let d_ref = watch_ref(min_watch);
                    if c_ref == d_ref {
                        continue;
                    }
                    ticks += 1;
                    if solver.arena.clause(d_ref).quotient() {
                        continue;
                    }
                    if solver.arena.clause(d_ref).size() != c_size_field {
                        continue;
                    }
                    let mut next = INVALID;
                    let d_size = solver.arena.clause(d_ref).size();
                    for j in 0..d_size {
                        let other = solver.arena.clause(d_ref).lit(j);
                        let mark = solver.marks[other as usize];
                        if mark & QUOTIENT != 0 {
                            continue;
                        }
                        if mark & FACTOR != 0 {
                            continue 'min_watches;
                        }
                        if mark & NOUNTED != 0 {
                            continue 'min_watches;
                        }
                        if next != INVALID {
                            continue 'min_watches;
                        }
                        next = other;
                    }
                    debug_assert!(next != INVALID);
                    if next > initial {
                        continue;
                    }
                    let next_idx = idx(next);
                    if !solver.flags[next_idx as usize].active {
                        continue;
                    }
                    debug_assert!(solver.marks[next as usize] & (FACTOR | NOUNTED) == 0);
                    solver.marks[next as usize] |= NOUNTED;
                    factoring.nounted.push(next);
                    solver.arena.clause_mut(d_ref).set_quotient(true);
                    factoring.qlauses.push(d_ref);
                    if factoring.count[next as usize] == 0 {
                        factoring.counted.push(next);
                    }
                    factoring.count[next as usize] += 1;
                }
                let mut nounted = std::mem::take(&mut factoring.nounted);
                clear_nounted(solver, &mut nounted);
                factoring.nounted = nounted;
            }
            let c_size2 = solver.arena.clause(c_ref).size();
            for j in 0..c_size2 {
                let other = solver.arena.clause(c_ref).lit(j);
                solver.marks[other as usize] &= !QUOTIENT;
            }
        }
        solver.statistics.factor_ticks += ticks;
        ticks = 0;
        if solver.statistics.factor_ticks > factoring.limit {
            break 'clauses;
        }
    }
    let mut qlauses = std::mem::take(&mut factoring.qlauses);
    clear_qlauses(solver, &mut qlauses);
    factoring.qlauses = qlauses;
    let mut next_count = 0u32;
    let mut next = INVALID;
    if solver.statistics.factor_ticks <= factoring.limit {
        let mut ties = 0u32;
        for i in 0..factoring.counted.len() {
            let l = factoring.counted[i];
            let lit_count = factoring.count[l as usize];
            if lit_count < next_count {
                continue;
            }
            if lit_count == next_count {
                debug_assert!(lit_count > 0);
                ties += 1;
            } else {
                debug_assert!(lit_count > next_count);
                next_count = lit_count;
                next = l;
                ties = 1;
            }
        }
        if next_count < 2 {
            next = INVALID;
        } else if ties > 1 {
            let mut next_score = -1.0f64;
            for i in 0..factoring.counted.len() {
                let l = factoring.counted[i];
                let lit_count = factoring.count[l as usize];
                if lit_count != next_count {
                    continue;
                }
                let lit_score = tied_next_factor_score(solver, factoring, l);
                debug_assert!(lit_score >= 0.0);
                if lit_score <= next_score {
                    continue;
                }
                next_score = lit_score;
                next = l;
            }
            debug_assert!(next_score >= 0.0);
            debug_assert!(next != INVALID);
        } else {
            debug_assert!(ties == 1);
        }
    }
    for i in 0..factoring.counted.len() {
        let l = factoring.counted[i];
        factoring.count[l as usize] = 0;
    }
    factoring.counted.clear();
    debug_assert!(next == INVALID || next_count > 1);
    (next, next_count)
}

// static void factorize_next
fn factorize_next(solver: &mut Solver, factoring: &mut Factoring, next: u32, expected_next_count: u32) {
    let last_quotient = factoring.quotients.len() - 1;
    let next_quotient = new_quotient(solver, factoring, next);

    debug_assert!(factoring.qlauses.is_empty());

    let mut ticks: u64 = 1
        + cache_lines(
            factoring.quotients[last_quotient].clauses.len() as u64,
            4,
        );

    let num_clauses = factoring.quotients[last_quotient].clauses.len();
    for i in 0..num_clauses {
        let last_watch = factoring.quotients[last_quotient].clauses[i];
        if watch_is_binary(last_watch) {
            let q = watch_lit(last_watch);
            let qv = solver.watches[q as usize];
            ticks += 1 + cache_lines((qv.end - qv.begin) as u64, 4);
            for wi in qv.begin..qv.end {
                let q_watch = solver.vectors.stack[wi];
                if watch_is_binary(q_watch) && watch_lit(q_watch) == next {
                    factoring.quotients[next_quotient].clauses.push(last_watch);
                    factoring.quotients[next_quotient].matches.push(i);
                    break;
                }
            }
        } else {
            let c_ref = watch_ref(last_watch);
            debug_assert!(!solver.arena.clause(c_ref).quotient());
            let mut min_lit = INVALID;
            let mut factors = 0u32;
            let mut min_size = 0usize;
            ticks += 1;
            let c_size = solver.arena.clause(c_ref).size();
            for j in 0..c_size {
                let other = solver.arena.clause(c_ref).lit(j);
                if solver.marks[other as usize] & FACTOR != 0 {
                    let prev = factors;
                    factors += 1;
                    if prev != 0 {
                        break;
                    }
                } else {
                    debug_assert!(solver.marks[other as usize] & QUOTIENT == 0);
                    solver.marks[other as usize] |= QUOTIENT;
                    let other_size = solver.watches[other as usize].size();
                    if min_lit != INVALID && min_size <= other_size {
                        continue;
                    }
                    min_lit = other;
                    min_size = other_size;
                }
            }
            debug_assert!(factors > 0);
            if factors == 1 {
                debug_assert!(min_lit != INVALID);
                let c_size_field = solver.arena.clause(c_ref).size();
                let mv = solver.watches[min_lit as usize];
                ticks += 1 + cache_lines((mv.end - mv.begin) as u64, 4);
                'min_watches: for wi in mv.begin..mv.end {
                    let min_watch = solver.vectors.stack[wi];
                    if watch_is_binary(min_watch) {
                        continue;
                    }
                    let d_ref = watch_ref(min_watch);
                    if c_ref == d_ref {
                        continue;
                    }
                    ticks += 1;
                    if solver.arena.clause(d_ref).quotient() {
                        continue;
                    }
                    if solver.arena.clause(d_ref).size() != c_size_field {
                        continue;
                    }
                    let d_size = solver.arena.clause(d_ref).size();
                    for j in 0..d_size {
                        let other = solver.arena.clause(d_ref).lit(j);
                        let mark = solver.marks[other as usize];
                        if mark & QUOTIENT != 0 {
                            continue;
                        }
                        if other != next {
                            continue 'min_watches;
                        }
                    }
                    factoring.quotients[next_quotient].clauses.push(min_watch);
                    factoring.quotients[next_quotient].matches.push(i);
                    factoring.qlauses.push(d_ref);
                    solver.arena.clause_mut(d_ref).set_quotient(true);
                    break;
                }
            }
            let c_size2 = solver.arena.clause(c_ref).size();
            for j in 0..c_size2 {
                let other = solver.arena.clause(c_ref).lit(j);
                solver.marks[other as usize] &= !QUOTIENT;
            }
        }
    }

    let mut qlauses = std::mem::take(&mut factoring.qlauses);
    clear_qlauses(solver, &mut qlauses);
    factoring.qlauses = qlauses;
    solver.statistics.factor_ticks += ticks;

    debug_assert!(expected_next_count as usize <= factoring.quotients[next_quotient].clauses.len());
    let _ = expected_next_count;
}

// static quotient *best_quotient — returns (index, reduction).
fn best_quotient(factoring: &Factoring) -> (Option<usize>, usize) {
    let mut factors = 1usize;
    let mut best_reduction = 0usize;
    let mut best: Option<usize> = None;
    for (qi, q) in factoring.quotients.iter().enumerate() {
        let quotients = q.clauses.len();
        let before_factorization = quotients * factors;
        let after_factorization = quotients + factors;
        if before_factorization > after_factorization {
            let delta = before_factorization - after_factorization;
            if best.is_none() || best_reduction < delta {
                best_reduction = delta;
                best = Some(qi);
            }
        }
        factors += 1;
    }
    if best.is_none() {
        return (None, 0);
    }
    (best, best_reduction)
}

// static void resize_factoring
fn resize_factoring(solver: &mut Solver, factoring: &mut Factoring, lit: u32) {
    debug_assert!(lit > not(lit));
    let old_size = factoring.size;
    debug_assert!(lit as usize > old_size);
    let old_allocated = factoring.allocated;
    let new_size = lit as usize + 1;
    if new_size > old_allocated {
        let mut new_allocated = 2 * old_allocated;
        while new_size > new_allocated {
            new_allocated *= 2;
        }
        factoring.count.resize(new_allocated, 0);
        debug_assert!(old_allocated % 2 == 0);
        debug_assert!(new_allocated % 2 == 0);
        let new_allocated_score = new_allocated / 2;
        for i in 0..factoring.hops as usize {
            let scores = &mut factoring.scores[i];
            scores.score.resize(new_allocated_score, -1.0);
        }
        factoring.allocated = new_allocated;
    }
    factoring.size = new_size;
    let _ = solver;
}

// static void flush_unmatched_clauses
fn flush_unmatched_clauses(_solver: &mut Solver, factoring: &mut Factoring, qi: usize) {
    debug_assert!(qi > 0);
    let (left, right) = factoring.quotients.split_at_mut(qi);
    let prev = &mut left[qi - 1];
    let q = &right[0];
    let n = q.clauses.len();
    debug_assert!(n == q.matches.len());
    let prev_is_first = qi - 1 == 0; // !prev->id
    for i in 0..n {
        let j = q.matches[i];
        debug_assert!(i <= j);
        if !prev_is_first {
            let matches = prev.matches[j];
            prev.matches[i] = matches;
        }
        let watch = prev.clauses[j];
        prev.clauses[i] = watch;
    }
    if !prev_is_first {
        prev.matches.truncate(n);
    }
    prev.clauses.truncate(n);
}

// static void add_factored_divider
fn add_factored_divider(solver: &mut Solver, factoring: &mut Factoring, qi: usize, fresh: u32) {
    let factor = factoring.quotients[qi].factor;
    crate::clause::new_binary_clause(solver, fresh, factor);
    solver.statistics.clauses_factored += 1;
    solver.statistics.literals_factored += 2;
}

// static void add_factored_quotient
fn add_factored_quotient(solver: &mut Solver, factoring: &mut Factoring, qi: usize, not_fresh: u32) {
    let num = factoring.quotients[qi].clauses.len();
    for i in 0..num {
        let watch = factoring.quotients[qi].clauses[i];
        if watch_is_binary(watch) {
            let other = watch_lit(watch);
            crate::clause::new_binary_clause(solver, not_fresh, other);
            solver.statistics.literals_factored += 2;
        } else {
            let c_ref = watch_ref(watch);
            debug_assert!(solver.clause.is_empty());
            let factor = factoring.quotients[qi].factor;
            solver.clause.push(not_fresh);
            let c_size = solver.arena.clause(c_ref).size();
            for j in 0..c_size {
                let other = solver.arena.clause(c_ref).lit(j);
                if other == factor {
                    continue;
                }
                solver.clause.push(other);
            }
            solver.statistics.literals_factored += c_size as u64;
            crate::clause::new_irredundant_clause(solver);
            solver.clause.clear();
        }
        solver.statistics.clauses_factored += 1;
    }
}

// static void eagerly_remove_watch
fn eagerly_remove_watch(solver: &mut Solver, watches_of: u32, needle: Watch) {
    let v = solver.watches[watches_of as usize];
    debug_assert!(v.begin != v.end);
    let mut p = v.begin;
    while solver.vectors.stack[p] != needle {
        p += 1;
        debug_assert!(p != v.end);
    }
    let last = v.end - 1;
    if p != last {
        solver.vectors.stack.copy_within(p + 1..v.end, p);
    }
    solver.watches[watches_of as usize].end = last; // SET_END_OF_WATCHES
}

// static void eagerly_remove_binary
fn eagerly_remove_binary(solver: &mut Solver, watches_of: u32, lit: u32) {
    let needle = binary_watch(lit);
    eagerly_remove_watch(solver, watches_of, needle);
}

// static void delete_unfactored
fn delete_unfactored(solver: &mut Solver, factoring: &mut Factoring, qi: usize) {
    let factor = factoring.quotients[qi].factor;
    let num = factoring.quotients[qi].clauses.len();
    for i in 0..num {
        let watch = factoring.quotients[qi].clauses[i];
        if watch_is_binary(watch) {
            let other = watch_lit(watch);
            eagerly_remove_binary(solver, other, factor);
            eagerly_remove_binary(solver, factor, other);
            crate::clause::delete_binary(solver, factor, other);
            solver.statistics.literals_unfactored += 2;
        } else {
            let ref_ = watch_ref(watch);
            let c_size = solver.arena.clause(ref_).size();
            for j in 0..c_size {
                let l = solver.arena.clause(ref_).lit(j);
                eagerly_remove_watch(solver, l, watch);
            }
            crate::clause::mark_clause_as_garbage(solver, ref_);
            solver.statistics.literals_unfactored += c_size as u64;
        }
        solver.statistics.clauses_unfactored += 1;
    }
}

// static void update_factored
fn update_factored(solver: &mut Solver, factoring: &mut Factoring, qi: usize) {
    let factor = factoring.quotients[qi].factor;
    update_candidate(solver, factoring, factor);
    update_candidate(solver, factoring, not(factor));
    let num = factoring.quotients[qi].clauses.len();
    for i in 0..num {
        let watch = factoring.quotients[qi].clauses[i];
        if watch_is_binary(watch) {
            let other = watch_lit(watch);
            update_candidate(solver, factoring, other);
        } else {
            let ref_ = watch_ref(watch);
            let c_size = solver.arena.clause(ref_).size();
            for j in 0..c_size {
                let l = solver.arena.clause(ref_).lit(j);
                if l != factor {
                    update_candidate(solver, factoring, l);
                }
            }
        }
    }
}

// static bool apply_factoring
fn apply_factoring(solver: &mut Solver, factoring: &mut Factoring, qi: usize) -> bool {
    let fresh = crate::import::fresh_literal(solver);
    if fresh == INVALID {
        return false;
    }
    solver.statistics.factored += 1;
    factoring.fresh.push(fresh);
    // for (quotient *p = q; p->prev; p = p->prev)
    let mut p = qi;
    while p > 0 {
        flush_unmatched_clauses(solver, factoring, p);
        p -= 1;
    }
    // for (quotient *p = q; p; p = p->prev)
    let mut p = qi as isize;
    while p >= 0 {
        add_factored_divider(solver, factoring, p as usize, fresh);
        p -= 1;
    }
    let not_fresh = not(fresh);
    add_factored_quotient(solver, factoring, qi, not_fresh);
    let mut p = qi as isize;
    while p >= 0 {
        delete_unfactored(solver, factoring, p as usize);
        p -= 1;
    }
    let mut p = qi as isize;
    while p >= 0 {
        update_factored(solver, factoring, p as usize);
        p -= 1;
    }
    debug_assert!(fresh < not_fresh);
    resize_factoring(solver, factoring, not_fresh);
    true
}

// static void adjust_scores_and_phases_of_fresh_variables
fn adjust_scores_and_phases_of_fresh_variables(solver: &mut Solver, factoring: &mut Factoring) {
    {
        // unbump fresh variables (reverse order)
        for p in (0..factoring.fresh.len()).rev() {
            let l = factoring.fresh[p];
            let i = idx(l);
            let score = 0.0;
            crate::heap::update_heap(&mut solver.scores, i, score);
        }
    }
    {
        let mut links = std::mem::take(&mut solver.links);
        let mut queue = solver.queue;
        for p in 0..factoring.fresh.len() {
            let l = factoring.fresh[p];
            let i = idx(l);
            crate::inlinequeue::dequeue_links(i, &mut links, &mut queue);
        }
        for p in 0..factoring.fresh.len() {
            let l = factoring.fresh[p];
            let i = idx(l);
            if crate::queue::disconnected(queue.first) {
                debug_assert!(crate::queue::disconnected(queue.last));
                queue.last = i;
            } else {
                let first = queue.first;
                debug_assert!(crate::queue::disconnected(links[first as usize].prev));
                links[first as usize].prev = i;
            }
            links[i as usize].next = queue.first;
            queue.first = i;
            debug_assert!(crate::queue::disconnected(links[i as usize].prev));
        }
        queue.stamp = 0;
        let mut i = queue.first;
        while !crate::queue::disconnected(i) {
            queue.stamp += 1;
            links[i as usize].stamp = queue.stamp;
            i = links[i as usize].next;
        }
        queue.search.idx = queue.last;
        queue.search.stamp = queue.stamp;
        solver.links = links;
        solver.queue = queue;
        // kissat_check_queue: !NDEBUG only.
    }
}

// static bool run_factorization
fn run_factorization(solver: &mut Solver, limit: u64) -> bool {
    let mut factoring = init_factoring(solver, limit);
    schedule_factorization(solver, &mut factoring);
    let mut done = false;
    let mut factored = 0u32;
    crate::print::extremely_verbose(
        solver,
        format_args!(
            "factorization limit of {} ticks",
            limit.wrapping_sub(solver.statistics.factor_ticks)
        ),
    );
    while !done && !crate::heap::empty_heap(&factoring.schedule) {
        let first = crate::heap::pop_max_heap(&mut factoring.schedule);
        let first_idx = idx(first);
        if !solver.flags[first_idx as usize].active {
            continue;
        }
        if solver.statistics.factor_ticks > limit {
            crate::print::very_verbose(solver, "factorization ticks limit hit");
            break;
        }
        if terminated!(solver, factor_terminated_1) {
            break;
        }
        let bit = 1u8 << negated(first);
        if solver.flags[first_idx as usize].factor & bit == 0 {
            continue;
        }
        solver.flags[first_idx as usize].factor &= !bit;
        let first_count = first_factor(solver, &mut factoring, first);
        if first_count > 1 {
            loop {
                let (next, next_count) = next_factor(solver, &mut factoring);
                if next == INVALID {
                    break;
                }
                debug_assert!(next_count > 1);
                if next_count < 2 {
                    break;
                }
                factorize_next(solver, &mut factoring, next, next_count);
            }
            let (q, reduction) = best_quotient(&factoring);
            if let Some(qi) = q {
                if reduction > factoring.bound as usize {
                    if apply_factoring(solver, &mut factoring, qi) {
                        factored += 1;
                    } else {
                        done = true;
                    }
                }
            }
        }
        release_quotients(solver, &mut factoring);
    }
    let completed = crate::heap::empty_heap(&factoring.schedule);
    adjust_scores_and_phases_of_fresh_variables(solver, &mut factoring);
    release_factoring(solver, &mut factoring);
    crate::report::report(solver, factored == 0, 'f');
    completed
}

// static void connect_clauses_to_factor
fn connect_clauses_to_factor(solver: &mut Solver) {
    let size_limit = solver.options.factorsize as u32;
    if size_limit < 3 {
        crate::print::extremely_verbose(solver, "only factorizing binary clauses");
        return;
    }
    crate::print::very_verbose(
        solver,
        format_args!("factorizing clauses of maximum size {}", size_limit),
    );
    let last_irredundant = solver.last_irredundant;
    let lits_count = solver.lits() as usize;
    let mut bincount = vec![0u32; lits_count];
    for l in 0..solver.lits() {
        if !solver.flags[idx(l) as usize].active {
            continue;
        }
        let v = solver.watches[l as usize];
        for wi in v.begin..v.end {
            let watch = solver.vectors.stack[wi];
            debug_assert!(watch_is_binary(watch));
            let other = watch_lit(watch);
            if l > other {
                continue;
            }
            bincount[l as usize] += 1;
            bincount[other as usize] += 1;
        }
    }
    let mut largecount = vec![0u32; lits_count];
    let mut initial_candidates = 0usize;
    let mut ref_: Reference = 0;
    while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        let c = solver.arena.clause(ref_);
        if c.garbage() {
            ref_ = next;
            continue;
        }
        if last_irredundant != crate::reference::INVALID_REF && ref_ > last_irredundant {
            break;
        }
        if c.redundant() {
            ref_ = next;
            continue;
        }
        if c.size() > size_limit {
            ref_ = next;
            continue;
        }
        let c_size = c.size();
        for j in 0..c_size {
            let l = solver.arena.clause(ref_).lit(j);
            largecount[l as usize] += 1;
        }
        initial_candidates += 1;
        ref_ = next;
    }
    crate::print::very_verbose(
        solver,
        format_args!(
            "initially found {} large clause candidates",
            initial_candidates
        ),
    );
    let mut candidates = initial_candidates;
    let rounds = solver.options.factorcandrounds;
    for round in 1..=rounds {
        let mut new_candidates = 0usize;
        let mut newlargecount = vec![0u32; lits_count];
        let mut ref_: Reference = 0;
        'clauses1: while (ref_ as u64) < solver.arena.size_wards() {
            let next = solver.arena.next_clause_ref(ref_);
            let c = solver.arena.clause(ref_);
            if c.garbage() {
                ref_ = next;
                continue;
            }
            if last_irredundant != crate::reference::INVALID_REF && ref_ > last_irredundant {
                break;
            }
            if c.redundant() {
                ref_ = next;
                continue;
            }
            if c.size() > size_limit {
                ref_ = next;
                continue;
            }
            let c_size = c.size();
            for j in 0..c_size {
                let l = solver.arena.clause(ref_).lit(j);
                if bincount[l as usize] + largecount[l as usize] < 2 {
                    ref_ = next;
                    continue 'clauses1; // goto CONTINUE_WITH_NEXT_CLAUSE1
                }
            }
            for j in 0..c_size {
                let l = solver.arena.clause(ref_).lit(j);
                newlargecount[l as usize] += 1;
            }
            new_candidates += 1;
            ref_ = next;
        }
        largecount = newlargecount;
        if candidates == new_candidates {
            crate::print::very_verbose(
                solver,
                format_args!(
                    "no large factorization candidate clauses reduction in round {}",
                    round
                ),
            );
            break;
        }
        candidates = new_candidates;
        crate::print::very_verbose(
            solver,
            format_args!(
                "reduced to {} large factorization candidate clauses {:.0}% in round {}",
                candidates,
                percent(candidates as f64, initial_candidates as f64),
                round
            ),
        );
    }
    let mut connected = 0usize;
    let mut ref_: Reference = 0;
    'clauses2: while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        let c = solver.arena.clause(ref_);
        if c.garbage() {
            ref_ = next;
            continue;
        }
        if last_irredundant != crate::reference::INVALID_REF && ref_ > last_irredundant {
            break;
        }
        if c.redundant() {
            ref_ = next;
            continue;
        }
        if c.size() > size_limit {
            ref_ = next;
            continue;
        }
        let c_size = c.size();
        for j in 0..c_size {
            let l = solver.arena.clause(ref_).lit(j);
            if bincount[l as usize] + largecount[l as usize] < 2 {
                ref_ = next;
                continue 'clauses2; // goto CONTINUE_WITH_NEXT_CLAUSE2
            }
        }
        crate::watch::inlined_connect_clause(solver, ref_);
        connected += 1;
        ref_ = next;
    }
    drop(largecount);
    drop(bincount);
    crate::print::very_verbose(
        solver,
        format_args!(
            "connected {} large factorization candidate clauses {:.0}%",
            connected,
            percent(candidates as f64, initial_candidates as f64)
        ),
    );
}

// static bool kissat_factoring — the C static's name loses only the prefix.
fn factoring(solver: &mut Solver) -> bool {
    if solver.options.factor == 0 {
        return false;
    }
    if solver.active == 0 {
        return false;
    }
    let active = solver.active;
    let log_active = (active as f64).log10() as usize; // size_t log_active = log10 (active)
    let eliminations = solver.statistics.eliminations as usize;
    let delay = solver.options.factordelay as usize;
    let limit = eliminations + delay;
    if log_active <= limit {
        return true;
    }
    crate::print::very_verbose(
        solver,
        format_args!(
            "delaying factorization as '{} = log10(variables) = log10 ({})  > eliminations + delay = {} + {} = {}",
            log_active, active, eliminations, delay, limit
        ),
    );
    false
}

// void kissat_factor
pub fn factor(solver: &mut Solver) {
    debug_assert!(solver.level == 0);
    if solver.inconsistent {
        return;
    }
    if !factoring(solver) {
        return;
    }
    if solver.limits.factor.marked >= solver.statistics.literals_factor {
        crate::print::extremely_verbose(
            solver,
            format_args!(
                "factorization skipped as no literals have been marked to be added ({} < {}",
                solver.limits.factor.marked, solver.statistics.literals_factor
            ),
        );
        return;
    }
    crate::profile::start_checked(solver, Prof::factor);
    solver.statistics.factorizations += 1;
    let factorizations = solver.statistics.factorizations;
    crate::print::phase(
        solver,
        "factorization",
        factorizations,
        "binary clause bounded variable addition",
    );
    let mut limit = solver.options.factoriniticks as u64;
    if solver.statistics.factorizations > 1 {
        let tmp = crate::set_effort_limit!(solver, factor, factoreffort, factor_ticks);
        limit = tmp;
    } else {
        crate::print::very_verbose(
            solver,
            format_args!(
                "initially limiting to {} million factorization ticks",
                limit
            ),
        );
        limit = (limit as f64 * 1e6) as u64;
        limit += solver.statistics.factor_ticks;
    }
    // #ifndef QUIET (kept):
    let before_variables =
        (solver.statistics.variables_extension + solver.statistics.variables_original) as i64;
    let before_binary = solver.statistics.clauses_binary as i64;
    let before_clauses = solver.statistics.clauses_irredundant as i64;
    let before_ticks = solver.statistics.factor_ticks as i64;
    crate::dense::enter_dense_mode(solver, None);
    connect_clauses_to_factor(solver);
    let completed = run_factorization(solver, limit);
    crate::dense::resume_sparse_mode(solver, false, None);
    let after_variables =
        (solver.statistics.variables_extension + solver.statistics.variables_original) as i64;
    let after_binary = solver.statistics.clauses_binary as i64;
    let after_clauses = solver.statistics.clauses_irredundant as i64;
    let after_ticks = solver.statistics.factor_ticks as i64;
    let delta_variables = after_variables - before_variables;
    let delta_binary = before_binary - after_binary;
    let delta_clauses = before_clauses - after_clauses;
    let delta_ticks = after_ticks - before_ticks;
    crate::print::very_verbose(
        solver,
        format_args!(
            "used {:.6} million factorization ticks",
            delta_ticks as f64 * 1e-6
        ),
    );
    crate::print::phase(
        solver,
        "factorization",
        factorizations,
        format_args!(
            "introduced {} extension variables {:.0}%",
            delta_variables,
            percent(delta_variables as f64, before_variables as f64)
        ),
    );
    crate::print::phase(
        solver,
        "factorization",
        factorizations,
        format_args!(
            "removed {} binary clauses {:.0}%",
            delta_binary,
            percent(delta_binary as f64, before_binary as f64)
        ),
    );
    crate::print::phase(
        solver,
        "factorization",
        factorizations,
        format_args!(
            "removed {} large clauses {:.0}%",
            delta_clauses,
            percent(delta_clauses as f64, before_clauses as f64)
        ),
    );
    if completed {
        solver.limits.factor.marked = solver.statistics.literals_factor;
    }
    crate::profile::stop_checked(solver, Prof::factor);
}
