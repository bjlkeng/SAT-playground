// Port of src/kimits.h + src/kimits.c (kissat 4.0.4).
//
// PORT NOTE: kimits.h declares `typedef struct changes changes;` plus
// kissat_changes/kissat_changed, but kissat 4.0.4 never defines the struct or
// the functions anywhere in src/ — dead declarations, not ported.
//
// PORT NOTE: the C delay functions take a `delay *` pointer into
// solver->delays; Rust borrow rules make that awkward alongside `&mut Solver`,
// so they take a `DelayId` selector instead.  Call-site macros map as:
//   DELAYING (NAME)     → kimits::delaying (solver, DelayId::Name)
//   BUMP_DELAY (NAME)   → kimits::bump_delay (solver, DelayId::Name)
//   REDUCE_DELAY (NAME) → kimits::reduce_delay (solver, DelayId::Name)

use crate::internal::Solver;

// ---------------------------------------------------------------------------
// Structs (kimits.h)
// ---------------------------------------------------------------------------

/// `struct bounds`.
#[derive(Clone, Copy, Default)]
pub struct Bounds {
    pub eliminate: BoundsEliminate,
}

#[derive(Clone, Copy, Default)]
pub struct BoundsEliminate {
    pub max_bound_completed: u64,
    pub additional_clauses: u32,
}

/// `struct limits`.
#[derive(Clone, Copy, Default)]
pub struct Limits {
    pub conflicts: u64,
    pub decisions: u64,
    pub reports: u64,

    pub mode: ModeLimits,

    pub eliminate: EliminateLimits,

    pub factor: FactorLimits,

    // C: struct { uint64_t conflicts; } probe, randec, reduce, reorder,
    //    rephase, restart;
    pub probe: ConflictLimit,
    pub randec: ConflictLimit,
    pub reduce: ConflictLimit,
    pub reorder: ConflictLimit,
    pub rephase: ConflictLimit,
    pub restart: ConflictLimit,

    pub glue: GlueLimits,
}

#[derive(Clone, Copy, Default)]
pub struct ModeLimits {
    pub count: u64,
    pub ticks: u64,
    pub conflicts: u64,
}

#[derive(Clone, Copy, Default)]
pub struct EliminateLimits {
    pub variables: EliminateVariableLimits,
    pub conflicts: u64,
}

#[derive(Clone, Copy, Default)]
pub struct EliminateVariableLimits {
    pub eliminate: u64,
    pub subsume: u64,
}

#[derive(Clone, Copy, Default)]
pub struct FactorLimits {
    pub marked: u64,
}

#[derive(Clone, Copy, Default)]
pub struct ConflictLimit {
    pub conflicts: u64,
}

#[derive(Clone, Copy, Default)]
pub struct GlueLimits {
    pub conflicts: u64,
    pub interval: u64,
}

/// `struct limited`.
#[derive(Clone, Copy, Default)]
pub struct Limited {
    pub conflicts: bool,
    pub decisions: bool,
}

/// `struct enabled`.
#[derive(Clone, Copy, Default)]
pub struct Enabled {
    pub eliminate: bool,
    pub focus: bool,
    pub mode: bool,
    pub probe: bool,
}

/// `struct delay`.
#[derive(Clone, Copy, Default)]
pub struct Delay {
    pub count: u32,
    pub current: u32,
}

/// `struct delays`.
#[derive(Clone, Copy, Default)]
pub struct Delays {
    pub bumpreasons: Delay,
    pub congruence: Delay,
    pub sweep: Delay,
    pub vivifyirr: Delay,
}

/// `struct remember` (field `last` in struct kissat).
#[derive(Clone, Copy, Default)]
pub struct Remember {
    pub ticks: RememberTicks,
    pub conflicts: RememberConflicts,
}

#[derive(Clone, Copy, Default)]
pub struct RememberTicks {
    pub eliminate: u64,
    pub probe: u64,
}

#[derive(Clone, Copy, Default)]
pub struct RememberConflicts {
    pub reduce: u64,
}

// ---------------------------------------------------------------------------
// kimits.c
// ---------------------------------------------------------------------------

/// Port of `kissat_logn`.  Exact math: log10 (count + 9).
pub fn logn(count: u64) -> f64 {
    debug_assert!(count > 0);
    let res = ((count + 9) as f64).log10();
    debug_assert!(res >= 1.0);
    res
}

/// Port of `kissat_sqrt`.
pub fn sqrt(count: u64) -> f64 {
    debug_assert!(count > 0);
    let res = (count as f64).sqrt();
    debug_assert!(res >= 1.0);
    res
}

/// Port of `kissat_nlogpown`: count * log10(count + 9)^exponent.
pub fn nlogpown(count: u64, exponent: u32) -> f64 {
    debug_assert!(count > 0);
    let tmp = ((count + 9) as f64).log10();
    let mut factor = 1.0;
    let mut exponent = exponent;
    while exponent > 0 {
        factor *= tmp;
        exponent -= 1;
    }
    debug_assert!(factor >= 1.0);
    let res = count as f64 * factor;
    debug_assert!(res >= 1.0);
    res
}

/// Port of `kissat_scale_delta`:
/// scaled = (4.5 * (log10 (BINIRR_CLAUSES + 1 + 9) - 5)^2 + 25) * delta,
/// truncated to u64 exactly as the C double→uint64_t conversion.
pub fn scale_delta(solver: &mut Solver, pretty: &str, delta: u64) -> u64 {
    // BINIRR_CLAUSES = clauses_binary + clauses_irredundant
    let c = solver.statistics.clauses_binary + solver.statistics.clauses_irredundant;
    let f = logn(c + 1) - 5.0;
    let ff = f * f;
    debug_assert!(ff >= 0.0);
    let fff = 4.5 * ff + 25.0;
    let scaled = (fff * delta as f64) as u64;
    debug_assert!(delta <= scaled);
    crate::print::very_verbose(
        solver,
        &format!(
            "scaled {} delta {} = {} * {} = (4.5 (log10({}) - 5)^2 + 25) * {}",
            pretty, scaled, fff, delta, c, delta
        ),
    );
    scaled
}

/// Port of static `init_enabled`.
fn init_enabled(solver: &mut Solver) {
    let probe = if solver.options.simplify == 0 {
        false
    } else if solver.options.probe == 0 {
        false
    } else if solver.options.substitute != 0 {
        true
    } else if solver.options.sweep != 0 {
        true
    } else if solver.options.vivify != 0 {
        true
    } else {
        false
    };
    crate::print::very_verbose(
        solver,
        &format!("probing {}abled", if probe { "en" } else { "dis" }),
    );
    solver.enabled.probe = probe;

    let eliminate = if solver.options.simplify == 0 {
        false
    } else if solver.options.eliminate == 0 {
        false
    } else {
        true
    };
    crate::print::very_verbose(
        solver,
        &format!("eliminate {}abled", if eliminate { "en" } else { "dis" }),
    );
    solver.enabled.eliminate = eliminate;
}

/// Body of the `INIT_CONFLICT_LIMIT (NAME, SCALE)` macro: computes the scaled
/// initial limit; the caller stores it into `limits.NAME.conflicts`.
fn init_conflict_limit(solver: &mut Solver, name: &str, delta: u64, scale: bool) -> u64 {
    let scaled = if !scale {
        delta
    } else {
        scale_delta(solver, name, delta)
    };
    let limit = solver.statistics.conflicts + scaled;
    crate::print::very_verbose(
        solver,
        &format!("initial {} limit of {} conflicts", name, limit),
    );
    limit
}

/// Port of `kissat_init_limits`.
pub fn init_limits(solver: &mut Solver) {
    debug_assert!(solver.statistics.searches == 1);

    init_enabled(solver);

    if solver.options.randec != 0 {
        let delta = solver.options.randecinit as u64;
        solver.limits.randec.conflicts = init_conflict_limit(solver, "randec", delta, false);
    }

    if solver.options.reduce != 0 {
        let delta = solver.options.reduceinit as u64;
        solver.limits.reduce.conflicts = init_conflict_limit(solver, "reduce", delta, false);
    }

    if solver.options.reorder != 0 {
        let delta = solver.options.reorderinit as u64;
        solver.limits.reorder.conflicts = init_conflict_limit(solver, "reorder", delta, false);
    }

    if solver.options.rephase != 0 {
        let delta = solver.options.rephaseinit as u64;
        solver.limits.rephase.conflicts = init_conflict_limit(solver, "rephase", delta, false);
    }

    if !solver.stable {
        crate::restart::update_focused_restart_limit(solver);
    }

    crate::mode::init_mode_limit(solver);

    if solver.enabled.eliminate {
        let delta = solver.options.eliminateinit as u64;
        solver.limits.eliminate.conflicts = init_conflict_limit(solver, "eliminate", delta, true);
        solver.bounds.eliminate.max_bound_completed = 0;
        solver.bounds.eliminate.additional_clauses = 0;
        crate::print::very_verbose(solver, "reset elimination bound to zero");
    }

    if solver.enabled.probe {
        let delta = solver.options.probeinit as u64;
        solver.limits.probe.conflicts = init_conflict_limit(solver, "probe", delta, true);
    }
}

// ---------------------------------------------------------------------------
// Delays
// ---------------------------------------------------------------------------

/// Selector replacing the C `delay *` argument (see PORT NOTE at top).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DelayId {
    Bumpreasons,
    Congruence,
    Sweep,
    Vivifyirr,
}

fn delay_mut<'a>(solver: &'a mut Solver, which: DelayId) -> &'a mut Delay {
    match which {
        DelayId::Bumpreasons => &mut solver.delays.bumpreasons,
        DelayId::Congruence => &mut solver.delays.congruence,
        DelayId::Sweep => &mut solver.delays.sweep,
        DelayId::Vivifyirr => &mut solver.delays.vivifyirr,
    }
}

/// Port of static `delay_description` (QUIET off).
fn delay_description(which: DelayId) -> &'static str {
    match which {
        DelayId::Bumpreasons => "bumping reason side literals",
        DelayId::Congruence => "congruence closure",
        DelayId::Sweep => "sweeping",
        DelayId::Vivifyirr => "vivifying irredundant clauses",
    }
}

// VERY_VERBOSE_IF_NOT_BUMPREASONS: for bumpreasons the C macro degrades to
// LOG (a no-op in this build), so bumpreasons prints nothing.
fn very_verbose_if_not_bumpreasons(solver: &mut Solver, which: DelayId, msg: &str) {
    if which == DelayId::Bumpreasons {
        return; // LOG (...) — no-op without LOGGING
    }
    crate::print::very_verbose(solver, msg);
}

/// Port of `kissat_reduce_delay`.
pub fn reduce_delay(solver: &mut Solver, which: DelayId) {
    let delay = delay_mut(solver, which);
    if delay.current == 0 {
        return;
    }
    delay.current /= 2;
    let current = delay.current;
    delay.count = current;
    very_verbose_if_not_bumpreasons(
        solver,
        which,
        &format!(
            "{} delay interval decreased to {}",
            delay_description(which),
            current
        ),
    );
}

/// Port of `kissat_bump_delay`.
pub fn bump_delay(solver: &mut Solver, which: DelayId) {
    let delay = delay_mut(solver, which);
    delay.current += (delay.current < u32::MAX) as u32;
    let current = delay.current;
    delay.count = current;
    very_verbose_if_not_bumpreasons(
        solver,
        which,
        &format!(
            "{} delay interval increased to {}",
            delay_description(which),
            current
        ),
    );
}

/// Port of `kissat_delaying`.
pub fn delaying(solver: &mut Solver, which: DelayId) -> bool {
    let delay = delay_mut(solver, which);
    if delay.count != 0 {
        delay.count -= 1;
        let current = delay.current;
        very_verbose_if_not_bumpreasons(
            solver,
            which,
            &format!(
                "{} still delayed ({} more times)",
                delay_description(which),
                current
            ),
        );
        true
    } else {
        very_verbose_if_not_bumpreasons(
            solver,
            which,
            &format!("{} not delayed", delay_description(which)),
        );
        false
    }
}

// ---------------------------------------------------------------------------
// Macros (kimits.h) for sibling modules
// ---------------------------------------------------------------------------

/// Port of the `UPDATE_CONFLICT_LIMIT (NAME, COUNT, SCALE_COUNT_FUNCTION,
/// SCALE_DELTA)` macro.  Because Rust macros cannot paste `NAME##int`, the
/// caller passes the option field and count field explicitly:
///
/// C: `UPDATE_CONFLICT_LIMIT (eliminate, eliminations, NLOGN, true);`
/// Rust: `update_conflict_limit!(solver, eliminate, eliminateint,
///        eliminations, |n| crate::kimits::nlogpown (n, 1), true);`
#[macro_export]
macro_rules! update_conflict_limit {
    ($solver:expr, $name:ident, $int_opt:ident, $count:ident, $scale_count:expr,
     $scale_delta:expr) => {{
        if !$solver.inconsistent {
            debug_assert!($solver.statistics.$count > 0);
            let mut delta: u64 = $solver.options.$int_opt as u64;
            let scaling: f64 = ($scale_count)($solver.statistics.$count);
            debug_assert!(scaling >= 1.0);
            delta = (delta as f64 * scaling) as u64; // DELTA *= SCALING
            let scaled: u64 = if !($scale_delta) {
                delta
            } else {
                $crate::kimits::scale_delta($solver, stringify!($name), delta)
            };
            $solver.limits.$name.conflicts = $solver.statistics.conflicts + scaled;
            let count = $solver.statistics.$count;
            let limit = $solver.limits.$name.conflicts;
            $crate::print::phase(
                $solver,
                stringify!($name),
                count,
                &format!("new limit of {} after {} conflicts", limit, scaled),
            );
        }
    }};
}

/// Port of the `SET_EFFORT_LIMIT (LIMIT, NAME, START)` macro.  Evaluates to
/// the new limit (C declares `uint64_t LIMIT`); pass the effort option field
/// (`NAME##effort`) and the statistics field for `START` explicitly:
///
/// C: `SET_EFFORT_LIMIT (limit, vivify, vivify_ticks);`
/// Rust: `let limit = set_effort_limit!(solver, vivify, vivifyeffort,
///        vivify_ticks);`
#[macro_export]
macro_rules! set_effort_limit {
    ($solver:expr, $name:ident, $effort_opt:ident, $start:ident) => {{
        let old_limit: u64 = $solver.statistics.$start;
        let ticks: u64 = $solver.statistics.search_ticks;
        let last: u64 = if $solver.probing {
            $solver.last.ticks.probe
        } else {
            $solver.last.ticks.eliminate
        };
        let mut reference: u64 = ticks - last;
        let mineffort: u64 = (1e6 * $solver.options.mineffort as f64) as u64;
        if reference < mineffort {
            reference = mineffort;
            $crate::print::extremely_verbose(
                $solver,
                &format!(
                    concat!(stringify!($name), " effort reference {} set to 'mineffort'"),
                    reference
                ),
            );
        } else {
            $crate::print::extremely_verbose(
                $solver,
                &format!(
                    concat!(
                        stringify!($name),
                        " effort reference {} = {} - {} 'search_ticks'"
                    ),
                    reference, ticks, last
                ),
            );
        }
        let effort: f64 = $solver.options.$effort_opt as f64 * 1e-3;
        let delta: u64 = (effort * reference as f64) as u64;
        $crate::print::extremely_verbose(
            $solver,
            &format!(
                concat!(
                    stringify!($name),
                    " effort delta {} = {} * {} '",
                    stringify!($start),
                    "'"
                ),
                delta, effort, reference
            ),
        );
        let new_limit: u64 = old_limit + delta;
        $crate::print::very_verbose(
            $solver,
            &format!(
                concat!(
                    stringify!($name),
                    " effort limit {} = {} + {} '",
                    stringify!($start),
                    "'"
                ),
                new_limit, old_limit, delta
            ),
        );
        new_limit
    }};
}
