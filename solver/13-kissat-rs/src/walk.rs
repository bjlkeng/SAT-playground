// Port of src/walk.c (kissat 4.0.4).
//
// PORT NOTES:
//  - `struct tagged` (ref:31 + binary:1 bitfield) is a plain u32 with the
//    binary flag in bit 31 (helpers make_tagged/tagged_is_binary/tagged_ref).
//  - The C walker carries a `kissat *solver` back-pointer; the Rust Walker
//    does not — `solver` is threaded as an explicit first argument.
//  - The walker's `litpairs *binaries` pointer becomes an explicit
//    `binaries: &[LitPair]` argument (owned by walking_phase, filled by
//    dense::enter_dense_mode, and passed on to resume_sparse_mode).
//  - `walker->original_values`: C swaps solver->values for a calloc'd LITS
//    array; ported with std::mem::replace and restored in release_walker.
//  - `dereference_literals` returns a pointer into either the binaries stack
//    or the arena; the Rust port copies the literals into the reusable
//    scratch `walker.lits` (capacity never affects semantics) because
//    pick_literal interleaves reads of the literals with &mut Solver calls.
//  - `walker->offset` exists in the C struct but is never used — omitted.
//  - Statistics tiers: walks/walk_steps are COUNTERs (real); flipped and
//    walk_improved are STATISTIC-tier (real fields, never printed);
//    walk_decisions/walk_previous are METRIC (no-op); GET (walks) is real.
//  - %g in kissat_phase messages is approximated with `{}` Display
//    (message-only, verbosity >= 1) as in restart.rs.
//  - CHECK_WALK code is not compiled in the reference build — omitted.

use crate::internal::Solver;
use crate::profile::Prof;
use crate::reference::{Reference, INVALID_REF};
use crate::terminated;
use crate::utilities::{average, percent};
use crate::watch::LitPair;

pub const LD_MAX_WALK_REF: u32 = 31;
pub const MAX_WALK_REF: u32 = (1u32 << LD_MAX_WALK_REF) - 1;

// struct tagged { unsigned ref : 31; bool binary : 1; }
type Tagged = u32;

#[inline]
fn make_tagged(binary: bool, ref_: u32) -> Tagged {
    debug_assert!(ref_ <= MAX_WALK_REF);
    ref_ | ((binary as u32) << 31)
}

#[inline]
fn tagged_is_binary(t: Tagged) -> bool {
    t & (1u32 << 31) != 0
}

#[inline]
fn tagged_ref(t: Tagged) -> u32 {
    t & MAX_WALK_REF
}

// struct counter { unsigned count; unsigned pos; }
#[derive(Clone, Copy, Default)]
struct Counter {
    count: u32,
    pos: u32,
}

const INVALID_BEST_TRAIL_POS: u32 = u32::MAX;

// struct walker
struct Walker {
    best_trail_pos: u32,
    clauses: u32,
    current: u32,
    exponents: u32,
    initial: u32,
    minimum: u32,

    random: crate::random::Generator,

    counters: Vec<Counter>,
    refs: Vec<Tagged>,
    table: Vec<f64>,

    original_values: Vec<i8>,
    best_values: Vec<i8>,

    scores: Vec<f64>,  // doubles scores;
    unsat: Vec<u32>,   // unsigneds unsat;
    trail: Vec<u32>,   // unsigneds trail;
    lits: Vec<u32>,    // scratch — see module PORT NOTES

    size: f64,
    epsilon: f64,

    limit: u64,
    flipped: u64,
    // #ifndef QUIET — kept:
    start: u64,
    report_flipped: u64,
    report_minimum: u32,
}

// static void push_unsat (kissat *, walker *, counter *, unsigned)
fn push_unsat(walker: &mut Walker, counter_ref: u32) {
    debug_assert!(counter_ref < walker.clauses);
    walker.counters[counter_ref as usize].pos = walker.unsat.len() as u32;
    walker.unsat.push(counter_ref);
}

// static bool pop_unsat (kissat *, walker *, counter *, unsigned, unsigned)
fn pop_unsat(walker: &mut Walker, counter_ref: u32, pos: u32) -> bool {
    debug_assert!(walker.current > 0);
    debug_assert!(counter_ref < walker.clauses);
    debug_assert!(walker.counters[counter_ref as usize].pos == pos);
    debug_assert!(walker.current as usize == walker.unsat.len());
    let other_counter_ref = walker.unsat.pop().unwrap();
    walker.current -= 1;
    let mut res = false;
    if counter_ref != other_counter_ref {
        debug_assert!(other_counter_ref < walker.clauses);
        let other_counter = &mut walker.counters[other_counter_ref as usize];
        debug_assert!(other_counter.pos == walker.current);
        debug_assert!(pos < other_counter.pos);
        other_counter.pos = pos;
        walker.unsat[pos as usize] = other_counter_ref;
        res = true;
    }
    res
}

// static double cbvals[][2]
const CBVALS: [[f64; 2]; 6] = [
    [0.0, 2.00],
    [3.0, 2.50],
    [4.0, 2.85],
    [5.0, 3.70],
    [6.0, 5.10],
    [7.0, 7.40],
];

// static double fit_cbval (double size)
fn fit_cbval(size: f64) -> f64 {
    let num_cbvals = CBVALS.len();
    let mut i = 0usize;
    while i + 2 < num_cbvals && (CBVALS[i][0] > size || CBVALS[i + 1][0] < size) {
        i += 1;
    }
    let x2 = CBVALS[i + 1][0];
    let x1 = CBVALS[i][0];
    let y2 = CBVALS[i + 1][1];
    let y1 = CBVALS[i][1];
    let dx = x2 - x1;
    let dy = y2 - y1;
    let res = dy * (size - x1) / dx + y1;
    debug_assert!(res > 0.0);
    res
}

// static void init_score_table (walker *walker)
fn init_score_table(solver: &mut Solver, walker: &mut Walker) {
    let cb = if solver.statistics.walks & 1 != 0 {
        fit_cbval(walker.size)
    } else {
        2.0
    };
    let base = 1.0 / cb;

    let mut exponents: u32 = 0;
    let mut next: f64 = 1.0;
    while next != 0.0 {
        exponents += 1;
        next *= base;
    }

    walker.table = Vec::with_capacity(exponents as usize);

    let mut epsilon: f64;
    let mut next: f64 = 1.0;
    epsilon = next;
    while next != 0.0 {
        epsilon = next;
        walker.table.push(epsilon);
        next = epsilon * base;
    }

    debug_assert!(walker.table.len() == exponents as usize);
    walker.exponents = exponents;
    walker.epsilon = epsilon;

    let walks = solver.statistics.walks;
    crate::print::phase(
        solver,
        "walk",
        walks,
        format_args!("CB {:.2} with inverse {:.2} as base", cb, base),
    );
    crate::print::phase(
        solver,
        "walk",
        walks,
        format_args!("table size {} and epsilon {}", exponents, epsilon),
    );
}

// static unsigned currently_unsatified (walker *walker)
fn currently_unsatified(walker: &Walker) -> u32 {
    walker.unsat.len() as u32
}

// static void import_decision_phases (walker *walker)
fn import_decision_phases(solver: &mut Solver, walker: &mut Walker) {
    // INC (walk_decisions) — METRIC, no-op.
    walker.best_values = vec![0i8; solver.vars as usize]; // kissat_calloc (VARS)
    let mut imported: u32 = 0;
    for idx in 0..solver.vars {
        if !solver.flags[idx as usize].active {
            continue;
        }
        let value = crate::decide::decide_phase(solver, idx) as i8;
        debug_assert!(value != 0);
        walker.best_values[idx as usize] = value;
        let lit = crate::literal::lit(idx);
        let not_lit = crate::literal::not(lit);
        solver.values[lit as usize] = value;
        solver.values[not_lit as usize] = -value;
        imported += 1;
    }
    let walks = solver.statistics.walks;
    let active = solver.active;
    crate::print::phase(
        solver,
        "walk",
        walks,
        format_args!(
            "imported {} decision phases {:.0}%",
            imported,
            percent(imported as f64, active as f64)
        ),
    );
}

// static unsigned connect_binary_counters (walker *walker)
fn connect_binary_counters(solver: &mut Solver, walker: &mut Walker, binaries: &[LitPair]) -> u32 {
    debug_assert!(binaries.len() <= u32::MAX as usize);
    let size = binaries.len() as u32;
    let mut unsat: u32 = 0;
    let mut counter_ref: u32 = 0;

    for binary_ref in 0..size {
        let litpair = &binaries[binary_ref as usize];
        let first = litpair.lits[0];
        let second = litpair.lits[1];
        debug_assert!(first < solver.lits());
        debug_assert!(second < solver.lits());
        let first_value = solver.values[first as usize];
        let second_value = solver.values[second as usize];
        if first_value == 0 || second_value == 0 {
            continue;
        }
        debug_assert!(counter_ref < walker.clauses);
        walker.refs[counter_ref as usize] = make_tagged(true, binary_ref);
        crate::watch::push_large_watch(solver, first, counter_ref);
        crate::watch::push_large_watch(solver, second, counter_ref);
        let count = (first_value > 0) as u32 + (second_value > 0) as u32;
        walker.counters[counter_ref as usize].count = count;
        if count == 0 {
            push_unsat(walker, counter_ref);
            unsat += 1;
        }
        counter_ref += 1;
    }
    let walks = solver.statistics.walks;
    crate::print::phase(
        solver,
        "walk",
        walks,
        format_args!(
            "initially {} unsatisfied binary clauses {:.0}% out of {}",
            unsat,
            percent(unsat as f64, counter_ref as f64),
            counter_ref
        ),
    );
    walker.size += 2.0 * counter_ref as f64;
    counter_ref
}

// static void connect_large_counters (walker *walker, unsigned counter_ref)
fn connect_large_counters(solver: &mut Solver, walker: &mut Walker, mut counter_ref: u32) {
    debug_assert!(solver.level == 0);

    let mut unsat: u32 = 0;
    let mut large: u32 = 0;

    // clause *last_irredundant = kissat_last_irredundant_clause (solver);
    let last_irredundant = solver.last_irredundant;

    let mut cur = crate::vector::PushCursor::load(solver);
    let mut ref_: Reference = 0;
    while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        // if (last_irredundant && c > last_irredundant) break;
        if last_irredundant != INVALID_REF && ref_ > last_irredundant {
            break;
        }
        let (garbage, redundant, size) = {
            let c = solver.arena.clause(ref_);
            (c.garbage(), c.redundant(), c.size())
        };
        if garbage {
            ref_ = next;
            continue;
        }
        if redundant {
            ref_ = next;
            continue;
        }
        let mut continue_with_next_clause = false;
        for i in 0..size {
            let lit = solver.arena.clause(ref_).lit(i);
            let value = walker.original_values[lit as usize];
            if value <= 0 {
                continue;
            }
            crate::clause::mark_clause_as_garbage(solver, ref_);
            continue_with_next_clause = true;
            break;
        }
        if continue_with_next_clause {
            ref_ = next;
            continue;
        }
        large += 1;
        let clause_ref = ref_;
        debug_assert!(clause_ref <= MAX_WALK_REF);
        debug_assert!(counter_ref < walker.clauses);
        walker.refs[counter_ref as usize] = make_tagged(false, clause_ref);
        let mut count: u32 = 0;
        let mut csize: u32 = 0;
        for i in 0..size {
            let lit = solver.arena.clause(ref_).lit(i);
            let value = solver.values[lit as usize]; // local_search_values
            if value == 0 {
                debug_assert!(walker.original_values[lit as usize] < 0);
                continue;
            }
            cur.push(solver, lit, crate::watch::large_watch(counter_ref)); // PUSH_WATCHES
            csize += 1;
            if value > 0 {
                count += 1;
            }
        }
        walker.counters[counter_ref as usize].count = count;

        if count == 0 {
            push_unsat(walker, counter_ref);
            unsat += 1;
        }
        counter_ref += 1;
        walker.size += csize as f64;
        ref_ = next;
    }
    cur.store(solver);
    let walks = solver.statistics.walks;
    crate::print::phase(
        solver,
        "walk",
        walks,
        format_args!(
            "initially {} unsatisfied large clauses {:.0}% out of {}",
            unsat,
            percent(unsat as f64, large as f64),
            large
        ),
    );
}

// #ifndef QUIET
// static void report_initial_minimum (kissat *solver, walker *walker)
fn report_initial_minimum(solver: &Solver, walker: &mut Walker) {
    walker.report_minimum = walker.minimum;
    crate::print::very_verbose(
        solver,
        format_args!("initial minimum of {} unsatisfied clauses", walker.minimum),
    );
}

// static void report_minimum (const char *type, kissat *solver, walker *)
fn report_minimum(type_: &str, solver: &Solver, walker: &mut Walker) {
    debug_assert!(walker.minimum <= walker.report_minimum);
    crate::print::very_verbose(
        solver,
        format_args!(
            "{} minimum of {} unsatisfied clauses after {} flipped literals",
            type_, walker.minimum, walker.flipped
        ),
    );
    walker.report_minimum = walker.minimum;
}

// static void init_walker (kissat *solver, walker *walker, litpairs *binaries)
fn init_walker(solver: &mut Solver, binaries: &[LitPair]) -> Walker {
    let clauses64 = solver.statistics.binirr_clauses(); // BINIRR_CLAUSES
    debug_assert!(clauses64 <= MAX_WALK_REF as u64);
    let clauses = clauses64 as u32;

    // memset (walker, 0, sizeof *walker);
    let mut walker = Walker {
        best_trail_pos: 0,
        clauses,
        current: 0,
        exponents: 0,
        initial: 0,
        minimum: 0,
        random: solver.random ^ solver.statistics.walks,
        counters: Vec::new(),
        refs: Vec::new(),
        table: Vec::new(),
        original_values: Vec::new(),
        best_values: Vec::new(),
        scores: Vec::new(),
        unsat: Vec::new(),
        trail: Vec::new(),
        lits: Vec::new(),
        size: 0.0,
        epsilon: 0.0,
        limit: 0,
        flipped: 0,
        start: 0,
        report_flipped: 0,
        report_minimum: 0,
    };

    // walker->original_values = solver->values;
    // solver->values = kissat_calloc (solver, LITS, 1);
    let lits = solver.lits() as usize;
    walker.original_values = std::mem::replace(&mut solver.values, vec![0i8; lits]);

    import_decision_phases(solver, &mut walker);

    // PORT NOTE: C mallocs counters/refs uninitialized; entries beyond the
    // connected prefix are never read.  Zero-initialized here.
    walker.counters = vec![Counter::default(); clauses as usize];
    walker.refs = vec![0 as Tagged; clauses as usize];

    debug_assert!(walker.size == 0.0);
    let counter_ref = connect_binary_counters(solver, &mut walker, binaries);
    connect_large_counters(solver, &mut walker, counter_ref);

    walker.current = currently_unsatified(&walker);
    walker.initial = walker.current;

    let walks = solver.statistics.walks;
    crate::print::phase(
        solver,
        "walk",
        walks,
        format_args!(
            "initially {} unsatisfied irredundant clauses {:.0}% out of {}",
            walker.initial,
            percent(walker.initial as f64, clauses64 as f64),
            clauses64
        ),
    );

    walker.size = average(walker.size, clauses64 as f64);
    crate::print::phase(
        solver,
        "walk",
        walks,
        format_args!("average clause size {:.2}", walker.size),
    );

    walker.minimum = walker.current;
    init_score_table(solver, &mut walker);

    report_initial_minimum(solver, &mut walker);
    walker
}

// static void init_walker_limit (kissat *solver, walker *walker)
fn init_walker_limit(solver: &mut Solver, walker: &mut Walker) {
    // SET_EFFORT_LIMIT (limit, walk, walk_steps);
    let limit = crate::set_effort_limit!(solver, walk, walkeffort, walk_steps);
    walker.limit = limit;
    walker.flipped = 0;
    // #ifndef QUIET:
    walker.start = solver.statistics.walk_steps;
    walker.report_minimum = u32::MAX;
    walker.report_flipped = 0;
}

// static void release_walker (walker *walker)
fn release_walker(solver: &mut Solver, walker: Walker) {
    // kissat_dealloc table/refs/counters, RELEASE_STACK unsat/scores/trail,
    // kissat_free best_values: all handled by Drop.
    // solver->values = walker->original_values;
    solver.values = walker.original_values.into();
}

// static unsigned break_value (kissat *, walker *, value *, unsigned lit)
fn break_value(solver: &mut Solver, walker: &Walker, lit: u32) -> u32 {
    debug_assert!(solver.values[lit as usize] < 0);
    let not_lit = crate::literal::not(lit);
    let v = solver.watches[not_lit as usize];
    let mut steps: u32 = 1;
    let mut res: u32 = 0;
    let mut p = v.begin;
    while p != v.end {
        steps += 1;
        let watch = solver.vectors.stack[p];
        debug_assert!(!crate::watch::watch_is_binary(watch));
        let counter_ref = crate::watch::watch_ref(watch);
        debug_assert!(counter_ref < walker.clauses);
        res += (walker.counters[counter_ref as usize].count == 1) as u32;
        p += 1;
    }
    solver.statistics.walk_steps += steps as u64; // ADD (walk_steps, steps)
    res
}

// static double scale_score (walker *walker, unsigned breaks)
fn scale_score(walker: &Walker, breaks: u32) -> f64 {
    if breaks < walker.exponents {
        walker.table[breaks as usize]
    } else {
        walker.epsilon
    }
}

// static unsigned pick_literal (kissat *solver, walker *walker)
fn pick_literal(solver: &mut Solver, walker: &mut Walker, binaries: &[LitPair]) -> u32 {
    debug_assert!(walker.current as usize == walker.unsat.len());
    // const unsigned pos = walker->flipped++ % walker->current;
    let pos = (walker.flipped % walker.current as u64) as u32;
    walker.flipped += 1;
    let counter_ref = walker.unsat[pos as usize];

    // dereference_literals into the scratch buffer (see module PORT NOTES).
    walker.lits.clear();
    let tagged = walker.refs[counter_ref as usize];
    if tagged_is_binary(tagged) {
        let binary_ref = tagged_ref(tagged);
        let litpair = &binaries[binary_ref as usize];
        walker.lits.push(litpair.lits[0]);
        walker.lits.push(litpair.lits[1]);
    } else {
        let clause_ref = tagged_ref(tagged);
        let c = solver.arena.clause(clause_ref);
        walker.lits.extend_from_slice(c.lits());
    }

    debug_assert!(walker.scores.is_empty());

    let mut sum: f64 = 0.0;
    let mut picked_lit: u32 = crate::literal::INVALID_LIT;

    for i in 0..walker.lits.len() {
        let lit = walker.lits[i];
        if solver.values[lit as usize] == 0 {
            continue;
        }
        picked_lit = lit;
        let breaks = break_value(solver, walker, lit);
        let score = scale_score(walker, breaks);
        debug_assert!(score > 0.0);
        walker.scores.push(score);
        sum += score;
    }
    debug_assert!(picked_lit != crate::literal::INVALID_LIT);
    debug_assert!(sum > 0.0);

    let random = crate::random::pick_double(&mut walker.random);
    debug_assert!((0.0..1.0).contains(&random));

    let threshold = sum * random;

    // assert (threshold < sum); // NOT TRUE!!!! (C comment)

    let mut scores_cursor = 0usize;
    sum = 0.0;

    for i in 0..walker.lits.len() {
        let lit = walker.lits[i];
        if solver.values[lit as usize] == 0 {
            continue;
        }
        let score = walker.scores[scores_cursor];
        scores_cursor += 1;
        sum += score;
        if threshold < sum {
            picked_lit = lit;
            break;
        }
    }
    debug_assert!(picked_lit != crate::literal::INVALID_LIT);

    walker.scores.clear();

    picked_lit
}

// static void break_clauses (kissat *, walker *, const value *, unsigned)
fn break_clauses(solver: &mut Solver, walker: &mut Walker, flipped: u32) {
    let not_flipped = crate::literal::not(flipped);
    debug_assert!(solver.values[not_flipped as usize] < 0);
    let v = solver.watches[not_flipped as usize];
    let mut steps: u32 = 1;
    let mut p = v.begin;
    while p != v.end {
        steps += 1;
        let watch = solver.vectors.stack[p];
        debug_assert!(!crate::watch::watch_is_binary(watch));
        let counter_ref = crate::watch::watch_ref(watch);
        debug_assert!(counter_ref < walker.clauses);
        let counter = &mut walker.counters[counter_ref as usize];
        debug_assert!(counter.count > 0);
        counter.count -= 1;
        if counter.count == 0 {
            push_unsat(walker, counter_ref);
        }
        p += 1;
    }
    solver.statistics.walk_steps += steps as u64; // ADD (walk_steps, steps)
}

// static void make_clauses (kissat *, walker *, const value *, unsigned)
fn make_clauses(solver: &mut Solver, walker: &mut Walker, flipped: u32) {
    debug_assert!(solver.values[flipped as usize] > 0);
    let v = solver.watches[flipped as usize];
    let mut steps: u32 = 1;
    let mut p = v.begin;
    while p != v.end {
        steps += 1;
        let watch = solver.vectors.stack[p];
        debug_assert!(!crate::watch::watch_is_binary(watch));
        let counter_ref = crate::watch::watch_ref(watch);
        debug_assert!(counter_ref < walker.clauses);
        let count = walker.counters[counter_ref as usize].count;
        walker.counters[counter_ref as usize].count = count + 1;
        if count == 0 {
            let pos = walker.counters[counter_ref as usize].pos;
            if pop_unsat(walker, counter_ref, pos) {
                steps += 1;
            }
        }
        p += 1;
    }
    solver.statistics.walk_steps += steps as u64; // ADD (walk_steps, steps)
}

// static void save_all_values (kissat *solver, walker *walker)
fn save_all_values(solver: &Solver, walker: &mut Walker) {
    debug_assert!(walker.trail.is_empty());
    debug_assert!(walker.best_trail_pos == INVALID_BEST_TRAIL_POS);
    for idx in 0..solver.vars {
        let lit = crate::literal::lit(idx);
        let value = solver.values[lit as usize];
        if value != 0 {
            walker.best_values[idx as usize] = value;
        }
    }
    walker.best_trail_pos = 0;
}

// static void save_walker_trail (kissat *solver, walker *walker, bool keep)
fn save_walker_trail(walker: &mut Walker, keep: bool) {
    // #if defined(LOGGING) || !defined(NDEBUG) block omitted; (void) solver.
    for i in 0..walker.best_trail_pos as usize {
        let lit = walker.trail[i];
        let value: i8 = if crate::literal::negated(lit) != 0 { -1 } else { 1 };
        let idx = crate::literal::idx(lit);
        walker.best_values[idx as usize] = value;
    }
    if !keep {
        return;
    }
    // shift the remaining literals to the front of the trail
    walker.trail.drain(..walker.best_trail_pos as usize);
    walker.best_trail_pos = 0;
}

// static void push_flipped (kissat *solver, walker *walker, unsigned flipped)
fn push_flipped(solver: &Solver, walker: &mut Walker, flipped: u32) {
    if walker.best_trail_pos == INVALID_BEST_TRAIL_POS {
        debug_assert!(walker.trail.is_empty());
    } else {
        let size_trail = walker.trail.len() as u32;
        debug_assert!(walker.best_trail_pos <= size_trail);
        let limit = solver.vars / 4 + 1; // VARS / 4 + 1
        debug_assert!(limit < INVALID_BEST_TRAIL_POS);
        if size_trail < limit {
            walker.trail.push(flipped);
        } else if walker.best_trail_pos != 0 {
            save_walker_trail(walker, true);
            walker.trail.push(flipped);
        } else {
            walker.trail.clear();
            walker.best_trail_pos = INVALID_BEST_TRAIL_POS;
        }
    }
}

// static void flip_literal (kissat *solver, walker *walker, unsigned flip)
fn flip_literal(solver: &mut Solver, walker: &mut Walker, flip: u32) {
    let value = solver.values[flip as usize];
    debug_assert!(value < 0);
    solver.values[flip as usize] = -value;
    solver.values[crate::literal::not(flip) as usize] = value;
    make_clauses(solver, walker, flip);
    break_clauses(solver, walker, flip);
    walker.current = currently_unsatified(walker);
}

// static void update_best (kissat *solver, walker *walker)
fn update_best(solver: &mut Solver, walker: &mut Walker) {
    debug_assert!(walker.current < walker.minimum);
    walker.minimum = walker.current;
    // #ifndef QUIET:
    let verbosity = crate::print::verbosity(solver);
    let mut report = verbosity > 2;
    if verbosity == 2 {
        if walker.flipped / 2 >= walker.report_flipped {
            report = true;
        } else if walker.minimum < 5
            || walker.report_minimum == u32::MAX
            || walker.minimum <= walker.report_minimum / 2
        {
            report = true;
        }
        if report {
            walker.report_minimum = walker.minimum;
            walker.report_flipped = walker.flipped;
        }
    }
    if report {
        report_minimum("new", solver, walker);
    }
    // #endif
    if walker.best_trail_pos == INVALID_BEST_TRAIL_POS {
        save_all_values(solver, walker);
    } else {
        debug_assert!((walker.trail.len() as u32) < INVALID_BEST_TRAIL_POS);
        walker.best_trail_pos = walker.trail.len() as u32;
    }
}

// static void local_search_step (kissat *solver, walker *walker)
fn local_search_step(solver: &mut Solver, walker: &mut Walker, binaries: &[LitPair]) {
    debug_assert!(walker.current > 0);
    solver.statistics.flipped += 1; // INC (flipped) — STATISTIC tier
    debug_assert!(walker.flipped < u64::MAX);
    walker.flipped += 1;
    let lit = pick_literal(solver, walker, binaries);
    flip_literal(solver, walker, lit);
    push_flipped(solver, walker, lit);
    if walker.current < walker.minimum {
        update_best(solver, walker);
    }
}

// static void local_search_round (walker *walker)
fn local_search_round(solver: &mut Solver, walker: &mut Walker, binaries: &[LitPair]) {
    // #ifndef QUIET:
    let before = walker.minimum;
    while walker.minimum != 0 && walker.limit > solver.statistics.walk_steps {
        if terminated!(solver, walk_terminated_1) {
            break;
        }
        local_search_step(solver, walker, binaries);
    }
    // #ifndef QUIET:
    report_minimum("last", solver, walker);
    debug_assert!(solver.statistics.walk_steps >= walker.start);
    let steps = solver.statistics.walk_steps - walker.start;
    crate::print::very_verbose(
        solver,
        format_args!("walking ends with {} unsatisfied clauses", walker.current),
    );
    crate::print::very_verbose(
        solver,
        format_args!(
            "flipping {} literals took {} steps ({:.2} per flipped)",
            walker.flipped,
            steps,
            average(steps as f64, walker.flipped as f64)
        ),
    );
    let after = walker.minimum;
    let walks = solver.statistics.walks;
    crate::print::phase(
        solver,
        "walk",
        walks,
        format_args!(
            "{} minimum {} after {} flips",
            if after < before { "new" } else { "unchanged" },
            after,
            walker.flipped
        ),
    );
}

// static void export_best_values (walker *walker)
fn export_best_values(solver: &mut Solver, walker: &Walker) {
    // memcpy (saved, best, VARS);
    let vars = solver.vars as usize;
    solver.phases.saved[..vars].copy_from_slice(&walker.best_values[..vars]);
}

// static bool save_final_minimum (walker *walker)
fn save_final_minimum(solver: &mut Solver, walker: &mut Walker) -> bool {
    debug_assert!(walker.minimum <= walker.initial);
    let walks = solver.statistics.walks;
    if walker.minimum == walker.initial {
        crate::print::phase(
            solver,
            "walk",
            walks,
            "no improvement thus keeping saved phases",
        );
        return false;
    }

    crate::print::phase(
        solver,
        "walk",
        walks,
        format_args!(
            "saving improved assignment of {} unsatisfied clauses",
            walker.minimum
        ),
    );

    if walker.best_trail_pos == 0 || walker.best_trail_pos == INVALID_BEST_TRAIL_POS {
        // minimum already saved
    } else {
        save_walker_trail(walker, false);
    }

    export_best_values(solver, walker);
    solver.statistics.walk_improved += 1; // INC (walk_improved) — STATISTIC

    true
}

// static void walking_phase (kissat *solver)
fn walking_phase(solver: &mut Solver) {
    solver.statistics.walks += 1; // INC (walks)
    let mut irredundant: Vec<LitPair> = Vec::new(); // litpairs irredundant
    crate::dense::enter_dense_mode(solver, Some(&mut irredundant));
    let mut walker = init_walker(solver, &irredundant);
    init_walker_limit(solver, &mut walker);
    local_search_round(solver, &mut walker, &irredundant);
    save_final_minimum(solver, &mut walker);
    release_walker(solver, walker);
    crate::dense::resume_sparse_mode(solver, false, Some(&mut irredundant));
    // RELEASE_STACK (irredundant) — Drop.
}

/// Port of `kissat_walking`.
pub fn walking(solver: &Solver) -> bool {
    let last_irredundant: u64 = if solver.last_irredundant == INVALID_REF {
        solver.arena.size_wards() // SIZE_STACK (solver->arena)
    } else {
        solver.last_irredundant as u64
    };

    if last_irredundant > MAX_WALK_REF as u64 {
        crate::print::extremely_verbose(
            solver,
            format_args!(
                "can not walk since last irredundant clause reference {} too large",
                last_irredundant
            ),
        );
        return false;
    }

    let clauses = solver.statistics.binirr_clauses(); // BINIRR_CLAUSES
    if clauses > MAX_WALK_REF as u64 {
        crate::print::extremely_verbose(
            solver,
            format_args!(
                "can not walk due to way too many irredundant clauses {}",
                clauses
            ),
        );
        return false;
    }

    true
}

/// Port of `kissat_walk`.
pub fn walk(solver: &mut Solver) {
    debug_assert!(solver.level == 0);
    debug_assert!(!solver.inconsistent);
    debug_assert!(walking(solver));

    let last_irredundant: u64 = if solver.last_irredundant == INVALID_REF {
        solver.arena.size_wards()
    } else {
        solver.last_irredundant as u64
    };

    if last_irredundant > MAX_WALK_REF as u64 {
        let walks = solver.statistics.walks;
        crate::print::phase(
            solver,
            "walk",
            walks,
            format_args!(
                "last irredundant clause reference {} too large",
                last_irredundant
            ),
        );
        return;
    }

    let clauses = solver.statistics.binirr_clauses();
    if clauses > MAX_WALK_REF as u64 {
        let walks = solver.statistics.walks;
        crate::print::phase(
            solver,
            "walk",
            walks,
            format_args!("way too many irredundant clauses {}", clauses),
        );
        return;
    }

    if solver.options.warmup != 0 {
        crate::warmup::warmup(solver);
    }

    // STOP_SEARCH_AND_START_SIMPLIFIER (walking);
    crate::profile::stop_search_and_start_simplifier_checked(solver, Prof::walking);
    walking_phase(solver);
    // STOP_SIMPLIFIER_AND_RESUME_SEARCH (walking);
    crate::profile::stop_simplifier_and_resume_search_checked(solver, Prof::walking);
}
