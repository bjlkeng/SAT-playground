// Not in kissat. RL scheduler plumbing for solver 13:
// plan/rl-scheduler-solver13-plan.md, step A (beads SAT-playground-p9m.6.*).
//
// What this is. The scheduler takes over kissat's *timing* layer: when the
// six timers (probe, eliminate, reduce, rephase, reorder, mode switch) fire,
// how hard focused-mode restarts are, and, in stage 1, how much effort sweep
// gets. Every mechanism stays byte-identical. Decisions happen at epoch
// boundaries on `statistics.search_ticks` (plan §1): a log row every X_o
// ticks and an action every X_d ticks, plus one decision D0 before the first
// `decide()` (plan §2.4).
//
// Off by default. With `SAT_POLICY` unset `Policy::on` is false and nothing
// in this module runs on the search path beyond the `if solver.policy.on`
// branch at each chokepoint. The chokepoints are:
//
//   reduce::reducing, mode::switching_search_mode, reorder::reordering,
//   rephase::rephasing, probe::probing, eliminate::eliminating -> effective_limit
//   restart::restarting                                         -> restart_margin
//   sweep::init_sweeper, sweep::sweep                           -> scale_effort_limit, skip_effort
//   kimits::init_conflict_limit, update_conflict_limit!,
//   mode::init_mode_limit, mode::update_mode_limit              -> record_limit (stock delta)
//   search::search, head of the if-chain                        -> epoch_due, epoch
//
// Invariants (plan §2.3). Policy off: no added line executes. Policy on with
// `act == STOCK`: every fire predicate equals the stock value, because after
// every fire `last_fire + 1 x stock_delta == limits[T]` and the multiplier
// 1 reproduces `stock_delta` exactly. tools/parity.py checks both against
// the C binary (20/20 at 100 k conflicts); tests/policy.rs checks them
// against the stock binary on generated formulas.
//
// Purity. Nothing here draws from `solver.random`, touches the arena, or
// increments a counter a heuristic reads. The policy has its own generator
// (`Policy::rng`, same LCG as random.rs, separate state).
//
// Environment (plan §7 item 6; the solver README has the table):
//   SAT_POLICY=stock|random|jitter   on, with the stock action / segmented
//                                    sticky random actions / per-decision
//                                    jitter. A weights-file path is step A.10.
//   SAT_POLICY_EPOCH_TICKS=X_o[,X_d] observation and decision epochs in
//                                    search ticks (default 2^23 and 2^27).
//   SAT_POLICY_SEED=n                the policy's own generator seed (0).
//   SAT_POLICY_TEMP=t                spread of the random menus around
//                                    stock; small = near stock, large =
//                                    uniform (1.0).
//   SAT_POLICY_SEGMENT=m             mean sticky segment length in decision
//                                    epochs (5).

use crate::internal::Solver;

// ---------------------------------------------------------------------------
// Timers, efforts, the action and its menus (plan §2.1, §2.2)
// ---------------------------------------------------------------------------

pub const N_TIMERS: usize = 6;

/// The six timers whose next fire the policy moves. The first five are on
/// conflicts; `Mode` is on conflicts in focused mode and on search ticks in
/// stable mode, exactly as kissat keeps `limits.mode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Timer {
    Probe = 0,
    Eliminate = 1,
    Reduce = 2,
    Rephase = 3,
    Reorder = 4,
    Mode = 5,
}

impl Timer {
    pub const ALL: [Timer; N_TIMERS] = [
        Timer::Probe,
        Timer::Eliminate,
        Timer::Reduce,
        Timer::Rephase,
        Timer::Reorder,
        Timer::Mode,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Timer::Probe => "probe",
            Timer::Eliminate => "eliminate",
            Timer::Reduce => "reduce",
            Timer::Rephase => "rephase",
            Timer::Reorder => "reorder",
            Timer::Mode => "mode",
        }
    }

    /// The timer behind a kissat limit name as used by
    /// `INIT_CONFLICT_LIMIT` / `UPDATE_CONFLICT_LIMIT`; `None` for limits the
    /// policy does not move (`randec`, `restart`).
    pub fn from_name(name: &str) -> Option<Timer> {
        Some(match name {
            "probe" => Timer::Probe,
            "eliminate" => Timer::Eliminate,
            "reduce" => Timer::Reduce,
            "rephase" => Timer::Rephase,
            "reorder" => Timer::Reorder,
            "mode" => Timer::Mode,
            _ => return None,
        })
    }
}

pub const N_EFFORTS: usize = 8;

/// Per-pass effort knobs (plan §2.2). Only `Sweep` is consulted in stage 1;
/// the rest are carried in the action so the log and the menu are fixed
/// before round 0 (plan §6.4, invariants across rounds).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Effort {
    Sweep = 0,
    Vivify = 1,
    Eliminate = 2,
    Backbone = 3,
    Factor = 4,
    Forward = 5,
    Transitive = 6,
    Walk = 7,
}

/// Elimination-bound choice (stage 2, plan §2.2); stage 1 keeps `Stock`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ElimBound {
    Hold,
    Stock,
    Escalate,
}

/// The action in force for one decision epoch (plan §2.3). Multipliers are
/// relative to the stock value; all ones plus `ElimBound::Stock` is stock.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Action {
    /// probe, eliminate, reduce, rephase, reorder, mode (`Timer` order).
    pub interval_mult: [f32; N_TIMERS],
    /// Scales `restartmargin`; focused mode only.
    pub restart_margin: f32,
    /// `Effort` order; 0 skips the pass for this round.
    pub effort_mult: [f32; N_EFFORTS],
    pub elim_bound: ElimBound,
    pub reduce_fraction: f32,
}

pub const STOCK: Action = Action {
    interval_mult: [1.0; N_TIMERS],
    restart_margin: 1.0,
    effort_mult: [1.0; N_EFFORTS],
    elim_bound: ElimBound::Stock,
    reduce_fraction: 1.0,
};

impl Default for Action {
    fn default() -> Self {
        STOCK
    }
}

/// Menus (plan §2.1). The index of the stock entry is next to each menu.
pub const INTERVAL_MENU: [f32; 5] = [0.0, 0.5, 1.0, 2.0, 4.0];
pub const INTERVAL_STOCK: usize = 2;
pub const MODE_MENU: [f32; 3] = [0.5, 1.0, 2.0];
pub const MODE_STOCK: usize = 1;
pub const MARGIN_MENU: [f32; 3] = [0.5, 1.0, 2.0];
pub const MARGIN_STOCK: usize = 1;
pub const SWEEP_MENU: [f32; 4] = [0.0, 0.5, 1.0, 2.0];
pub const SWEEP_STOCK: usize = 2;

/// Cap on the probability of the one-shot `m = 0` entry in random mode
/// (plan §5.3), so wild runs do not fire every timer every few epochs.
pub const ZERO_CAP: f64 = 0.05;

// ---------------------------------------------------------------------------
// Policy state
// ---------------------------------------------------------------------------

/// How actions are chosen. `Net` (a weights file) is step A.10.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// The stock action every epoch: measures logging and epoch overhead.
    #[default]
    Stock,
    /// Segmented sticky random actions (plan §5.3 flavour 2).
    Random,
    /// Near-stock jitter: resample every decision (plan §5.3 flavour 3).
    Jitter,
}

/// Per-timer bookkeeping (plan §2.1): the clock reading at the last fire
/// (conflicts, or search ticks for the mode timer in stable mode) and the
/// delta stock computed then, so that `last_fire + stock_delta` is the stock
/// deadline and `last_fire + m x stock_delta` the policy's.
#[derive(Clone, Copy, Default, Debug)]
pub struct TimerState {
    pub last_fire: u64,
    pub stock_delta: u64,
    /// Fires so far, not counting the initial limit.
    pub fires: u64,
}

pub const DEFAULT_OBS_TICKS: u64 = 1 << 23;
pub const DEFAULT_DEC_TICKS: u64 = 1 << 27;

pub struct Policy {
    pub on: bool,
    pub mode: Mode,
    /// X_o and X_d in search ticks.
    pub obs_ticks: u64,
    pub dec_ticks: u64,
    /// Next epoch boundaries; both 0 initially so the first hook call is D0.
    pub next_obs: u64,
    pub next_decision: u64,
    /// Observation rows logged and decisions taken so far.
    pub obs_epochs: u64,
    pub decisions: u64,
    pub act: Action,
    pub timers: [TimerState; N_TIMERS],
    /// The policy's own generator (random.rs LCG, separate state).
    pub rng: u64,
    pub seed: u64,
    pub temp: f64,
    pub segment_mean: f64,
    /// Decision epochs left in the current sticky segment.
    pub segment_left: u64,
    /// The action sampled at the start of the current sticky segment,
    /// re-issued (then masked) at every decision of the segment. Kept apart
    /// from `act`, which masking and a consumed one-shot modify.
    pub segment_act: Action,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            on: false,
            mode: Mode::Stock,
            obs_ticks: DEFAULT_OBS_TICKS,
            dec_ticks: DEFAULT_DEC_TICKS,
            next_obs: 0,
            next_decision: 0,
            obs_epochs: 0,
            decisions: 0,
            act: STOCK,
            timers: [TimerState::default(); N_TIMERS],
            rng: 0,
            seed: 0,
            temp: 1.0,
            segment_mean: 5.0,
            segment_left: 0,
            segment_act: STOCK,
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration from the environment
// ---------------------------------------------------------------------------

fn env_nonempty(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    }
}

/// Parse the `SAT_POLICY*` variables into `solver.policy`. `Ok(())` with the
/// policy off when `SAT_POLICY` is unset or empty; `Err(text)` for a value
/// the solver cannot honour, so the caller exits 1 the way it does for a bad
/// option.
pub fn init_from_env(solver: &mut Solver) -> Result<(), String> {
    let mut p = Policy::default();
    let mode = match env_nonempty("SAT_POLICY") {
        None => {
            solver.policy = p;
            return Ok(());
        }
        Some(m) => m,
    };
    p.mode = match mode.as_str() {
        "stock" => Mode::Stock,
        "random" => Mode::Random,
        "jitter" => Mode::Jitter,
        other => {
            return Err(format!(
                "SAT_POLICY='{}': expected stock, random or jitter (a weights file is plan step A.10)",
                other
            ))
        }
    };
    if let Some(v) = env_nonempty("SAT_POLICY_EPOCH_TICKS") {
        let mut it = v.split(',');
        let xo = it
            .next()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .filter(|&x| x >= 1)
            .ok_or_else(|| format!("SAT_POLICY_EPOCH_TICKS='{}': expected X_o[,X_d] with X_o >= 1", v))?;
        // Decisions are only checked at observation boundaries, so X_d must
        // be a multiple of X_o for the decision grid to be honoured exactly.
        let xd = match it.next() {
            None => xo.saturating_mul(16),
            Some(s) => s
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|&x| x >= xo && x % xo == 0)
                .ok_or_else(|| {
                    format!(
                        "SAT_POLICY_EPOCH_TICKS='{}': X_d must be an integer multiple of X_o",
                        v
                    )
                })?,
        };
        if it.next().is_some() {
            return Err(format!("SAT_POLICY_EPOCH_TICKS='{}': at most two values", v));
        }
        p.obs_ticks = xo;
        p.dec_ticks = xd;
    }
    if let Some(v) = env_nonempty("SAT_POLICY_SEED") {
        p.seed = v
            .parse::<u64>()
            .map_err(|_| format!("SAT_POLICY_SEED='{}': expected a non-negative integer", v))?;
    }
    if let Some(v) = env_nonempty("SAT_POLICY_TEMP") {
        p.temp = v
            .parse::<f64>()
            .ok()
            .filter(|t| t.is_finite() && *t > 0.0)
            .ok_or_else(|| format!("SAT_POLICY_TEMP='{}': expected a positive number", v))?;
    }
    if let Some(v) = env_nonempty("SAT_POLICY_SEGMENT") {
        p.segment_mean = v
            .parse::<f64>()
            .ok()
            .filter(|m| m.is_finite() && *m >= 1.0)
            .ok_or_else(|| format!("SAT_POLICY_SEGMENT='{}': expected a number >= 1", v))?;
    }
    p.rng = p.seed;
    p.on = true;
    solver.policy = p;
    Ok(())
}

/// The `[ policy ]` section printed after the limits when the policy is on
/// (nothing is printed when it is off, so stock output is unchanged).
pub fn print_configuration(solver: &mut Solver) {
    if !solver.policy.on {
        return;
    }
    crate::print::section(solver, "policy");
    let mode = match solver.policy.mode {
        Mode::Stock => "stock action every epoch",
        Mode::Random => "segmented sticky random actions",
        Mode::Jitter => "per-decision jitter",
    };
    crate::print::message(solver, format!("policy mode: {}", mode));
    crate::print::message(
        solver,
        format!(
            "observation epoch {} and decision epoch {} search ticks",
            solver.policy.obs_ticks, solver.policy.dec_ticks
        ),
    );
    if solver.policy.mode != Mode::Stock {
        crate::print::message(
            solver,
            format!(
                "policy seed {} temperature {} mean segment {} decision epochs",
                solver.policy.seed, solver.policy.temp, solver.policy.segment_mean
            ),
        );
    }
}

// ---------------------------------------------------------------------------
// Timer bookkeeping and the fire predicates (plan §2.1, §2.3)
// ---------------------------------------------------------------------------

/// Record a stock limit computation for timer `t`: the clock reading `base`
/// it was set at (`limits[T] = base + delta`) and the stock `delta`. Called
/// from `INIT_CONFLICT_LIMIT` (`fired == false`) and from every
/// `UPDATE_CONFLICT_LIMIT` / mode-limit update (`fired == true`). A one-shot
/// multiplier (`m == 0`) on a timer that has just fired reverts to 1 here,
/// so it fires exactly once per decision epoch (plan §2.1).
pub fn record_limit(solver: &mut Solver, t: Timer, base: u64, delta: u64, fired: bool) {
    let p = &mut solver.policy;
    let i = t as usize;
    p.timers[i].last_fire = base;
    p.timers[i].stock_delta = delta;
    if fired {
        p.timers[i].fires += 1;
        if p.act.interval_mult[i] == 0.0 {
            p.act.interval_mult[i] = 1.0;
        }
    }
}

/// `record_limit` keyed by kissat's limit name (see `Timer::from_name`);
/// names the policy does not move are ignored.
pub fn record_limit_by_name(solver: &mut Solver, name: &str, base: u64, delta: u64, fired: bool) {
    if let Some(t) = Timer::from_name(name) {
        record_limit(solver, t, base, delta, fired);
    }
}

/// The rail on any effective interval: 0.1 x stock_delta (plan §2.1).
#[inline]
pub fn interval_floor(stock_delta: u64) -> u64 {
    stock_delta / 10
}

/// The deadline the fire predicate of timer `t` compares against:
/// `last_fire + max(floor, m x stock_delta)`, or `last_fire + floor` for the
/// one-shot `m == 0`. With `m == 1` this is exactly the stock limit.
#[inline]
pub fn effective_limit(solver: &Solver, t: Timer) -> u64 {
    let i = t as usize;
    let st = &solver.policy.timers[i];
    let m = solver.policy.act.interval_mult[i];
    let delta = if m == 1.0 {
        st.stock_delta
    } else if m == 0.0 {
        interval_floor(st.stock_delta)
    } else {
        let scaled = (m as f64 * st.stock_delta as f64) as u64;
        scaled.max(interval_floor(st.stock_delta))
    };
    st.last_fire.saturating_add(delta)
}

/// `restartmargin` scaled by the action (focused mode; plan §2.1). Returns
/// the option value itself under the stock action.
#[inline]
pub fn restart_margin(solver: &Solver) -> f64 {
    solver.options.restartmargin as f64 * solver.policy.act.restart_margin as f64
}

/// Scale the delta of an effort limit computed by `set_effort_limit!`
/// (`limit == start + delta`) by the action's multiplier for pass `e`.
#[inline]
pub fn scale_effort_limit(solver: &Solver, e: Effort, start: u64, limit: u64) -> u64 {
    let m = solver.policy.act.effort_mult[e as usize];
    if m == 1.0 {
        return limit;
    }
    let delta = limit.saturating_sub(start);
    start.saturating_add((delta as f64 * m as f64) as u64)
}

/// True when the action says pass `e` is skipped this round (`m == 0`).
#[inline]
pub fn skip_effort(solver: &Solver, e: Effort) -> bool {
    solver.policy.act.effort_mult[e as usize] == 0.0
}

// ---------------------------------------------------------------------------
// Masking (plan §2.1)
// ---------------------------------------------------------------------------

/// Force knobs that cannot act in the solver's current configuration and
/// mode to their stock value, so a random or learned action is canonical:
/// rephase only runs in stable mode, the restart margin only in focused
/// mode, reorder per `reorder` (2 = both modes, 1 = stable only), the mode
/// switch only with `stable == 1`, and every disabled pass stays stock.
pub fn mask(solver: &Solver, mut act: Action) -> Action {
    let o = &solver.options;
    if !solver.stable || o.rephase == 0 {
        act.interval_mult[Timer::Rephase as usize] = 1.0;
    }
    if solver.stable || o.restart == 0 {
        act.restart_margin = 1.0;
    }
    if o.reorder == 0 || (o.reorder < 2 && !solver.stable) {
        act.interval_mult[Timer::Reorder as usize] = 1.0;
    }
    if o.stable != 1 {
        act.interval_mult[Timer::Mode as usize] = 1.0;
    }
    if o.reduce == 0 {
        act.interval_mult[Timer::Reduce as usize] = 1.0;
    }
    if !solver.enabled.probe {
        act.interval_mult[Timer::Probe as usize] = 1.0;
    }
    if !solver.enabled.eliminate {
        act.interval_mult[Timer::Eliminate as usize] = 1.0;
    }
    if o.sweep == 0 || !solver.enabled.probe {
        act.effort_mult[Effort::Sweep as usize] = 1.0;
    }
    act
}

// ---------------------------------------------------------------------------
// Random actions (plan §5.3 flavours 2 and 3; bead A.4)
// ---------------------------------------------------------------------------

/// Draw an index of an `n`-entry menu from a categorical centred on
/// `stock`: weight exp(-|i - stock| / temp), so a small temperature stays
/// within one step of stock and a large one is uniform. With `cap_zero` the
/// probability of entry 0 (the one-shot `m = 0`) is capped at `ZERO_CAP`.
pub fn sample_index(rng: &mut u64, n: usize, stock: usize, temp: f64, cap_zero: bool) -> usize {
    debug_assert!(n >= 1 && stock < n);
    let mut w = [0f64; 8];
    debug_assert!(n <= w.len());
    let mut total = 0.0;
    for (i, wi) in w.iter_mut().enumerate().take(n) {
        let dist = (i as f64 - stock as f64).abs();
        *wi = (-dist / temp).exp();
        total += *wi;
    }
    for wi in w.iter_mut().take(n) {
        *wi /= total;
    }
    if cap_zero && n > 1 && w[0] > ZERO_CAP {
        let rest: f64 = w[1..n].iter().sum();
        let scale = (1.0 - ZERO_CAP) / rest;
        for wi in w[1..n].iter_mut() {
            *wi *= scale;
        }
        w[0] = ZERO_CAP;
    }
    let u = crate::random::pick_double(rng);
    let mut acc = 0.0;
    for (i, wi) in w.iter().enumerate().take(n) {
        acc += *wi;
        if u < acc {
            return i;
        }
    }
    n - 1
}

/// One full random action at the run's temperature (before masking).
pub fn sample_action(rng: &mut u64, temp: f64) -> Action {
    let mut act = STOCK;
    for t in [Timer::Probe, Timer::Eliminate, Timer::Reduce, Timer::Rephase, Timer::Reorder] {
        let k = sample_index(rng, INTERVAL_MENU.len(), INTERVAL_STOCK, temp, true);
        act.interval_mult[t as usize] = INTERVAL_MENU[k];
    }
    let k = sample_index(rng, MODE_MENU.len(), MODE_STOCK, temp, false);
    act.interval_mult[Timer::Mode as usize] = MODE_MENU[k];
    let k = sample_index(rng, MARGIN_MENU.len(), MARGIN_STOCK, temp, false);
    act.restart_margin = MARGIN_MENU[k];
    let k = sample_index(rng, SWEEP_MENU.len(), SWEEP_STOCK, temp, true);
    act.effort_mult[Effort::Sweep as usize] = SWEEP_MENU[k];
    act
}

/// A geometric segment length with the given mean (>= 1), in decision
/// epochs: 1 + floor(ln(1 - u) / ln(1 - 1/mean)).
pub fn sample_segment_length(rng: &mut u64, mean: f64) -> u64 {
    if mean <= 1.0 {
        return 1;
    }
    let p = 1.0 / mean;
    let u = crate::random::pick_double(rng);
    let extra = ((1.0 - u).ln() / (1.0 - p).ln()).floor();
    1 + if extra.is_finite() && extra > 0.0 {
        extra.min(1e9) as u64
    } else {
        0
    }
}

/// The unmasked action for this decision under the policy's mode.
fn choose(solver: &mut Solver) -> Action {
    let p = &mut solver.policy;
    match p.mode {
        Mode::Stock => STOCK,
        Mode::Jitter => sample_action(&mut p.rng, p.temp),
        Mode::Random => {
            if p.segment_left == 0 {
                p.segment_left = sample_segment_length(&mut p.rng, p.segment_mean);
                // At each segment start the action is stock with
                // probability one half, otherwise a fresh draw.
                p.segment_act = if crate::random::pick_bool(&mut p.rng) {
                    STOCK
                } else {
                    sample_action(&mut p.rng, p.temp)
                };
            }
            p.segment_left -= 1;
            // Re-issue the segment's action at every decision: masking is
            // redone for the current mode, and a one-shot `m = 0` re-arms
            // once per decision epoch (plan §2.1), so nothing that
            // happened to `act` during the last epoch leaks into this one.
            p.segment_act
        }
    }
}

// ---------------------------------------------------------------------------
// The epoch hook (plan §1, §2.3)
// ---------------------------------------------------------------------------

/// Polled at the head of the search-loop if-chain, after the policy-on test.
#[inline]
pub fn epoch_due(solver: &Solver) -> bool {
    solver.statistics.search_ticks >= solver.policy.next_obs
}

/// One observation epoch: a log row (step A.5), and every X_d a decision.
/// Both boundaries are kept on their fixed grids (X_d is a multiple of X_o,
/// enforced by `init_from_env`, so every decision boundary is also an
/// observation boundary); if the search ran past several boundaries since
/// the last poll, one row and one decision are taken and the grids catch
/// up.
pub fn epoch(solver: &mut Solver) {
    let ticks = solver.statistics.search_ticks;
    log_row(solver);
    solver.policy.obs_epochs += 1;
    while solver.policy.next_obs <= ticks {
        solver.policy.next_obs = solver.policy.next_obs.saturating_add(solver.policy.obs_ticks);
    }
    if ticks >= solver.policy.next_decision {
        decide(solver);
        while solver.policy.next_decision <= ticks {
            solver.policy.next_decision = solver
                .policy
                .next_decision
                .saturating_add(solver.policy.dec_ticks);
        }
    }
}

/// Take a decision: choose an action, mask it for the current mode and
/// configuration, and put it in force.
pub fn decide(solver: &mut Solver) {
    let act = choose(solver);
    let act = mask(solver, act);
    solver.policy.act = act;
    solver.policy.decisions += 1;
    if crate::print::verbosity(solver) >= 2 {
        report_decision(solver);
    }
}

fn report_decision(solver: &Solver) {
    let p = &solver.policy;
    let a = &p.act;
    crate::print::very_verbose(
        solver,
        format_args!(
            "policy decision {} at {} search ticks {} conflicts: intervals {:?} margin {} sweep {} ({})",
            p.decisions,
            solver.statistics.search_ticks,
            solver.statistics.conflicts,
            a.interval_mult,
            a.restart_margin,
            a.effort_mult[Effort::Sweep as usize],
            if solver.stable { "stable" } else { "focused" }
        ),
    );
}

/// One observation row (step A.5 fills this in). Must stay pure with respect
/// to solver state: no RNG draws, no allocation the arena can see, no
/// counter a heuristic reads.
pub fn log_row(_solver: &mut Solver) {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A solver at search start: limits armed as `init_limits` arms them,
    /// with the policy on and the stock action.
    fn armed_solver() -> Solver {
        let mut s = crate::internal::init();
        s.statistics.searches = 1;
        s.policy.on = true;
        s.policy.mode = Mode::Stock;
        crate::kimits::init_limits(&mut s);
        s
    }

    fn stock_limit(s: &Solver, t: Timer) -> u64 {
        match t {
            Timer::Probe => s.limits.probe.conflicts,
            Timer::Eliminate => s.limits.eliminate.conflicts,
            Timer::Reduce => s.limits.reduce.conflicts,
            Timer::Rephase => s.limits.rephase.conflicts,
            Timer::Reorder => s.limits.reorder.conflicts,
            Timer::Mode => {
                if s.limits.mode.count & 1 != 0 {
                    s.limits.mode.ticks
                } else {
                    s.limits.mode.conflicts
                }
            }
        }
    }

    #[test]
    fn stock_action_reproduces_every_initial_limit() {
        let s = armed_solver();
        for t in Timer::ALL {
            assert_eq!(effective_limit(&s, t), stock_limit(&s, t), "{}", t.name());
            assert_eq!(s.policy.timers[t as usize].fires, 0);
        }
        // The scaled initial deltas (probe, eliminate) were recorded scaled.
        assert!(s.policy.timers[Timer::Probe as usize].stock_delta > s.options.probeinit as u64);
    }

    #[test]
    fn stock_action_reproduces_every_updated_limit() {
        let mut owned = armed_solver();
        let s = &mut owned; // the macro takes `&mut Solver`, as at its call sites
        // Advance and fire every conflict timer through the real macro.
        s.statistics.conflicts = 5000;
        s.statistics.reductions = 3;
        crate::update_conflict_limit!(s, reduce, reduceint, reductions, |n| crate::kimits::sqrt(n), false);
        s.statistics.conflicts = 6000;
        s.statistics.rephased = 2;
        crate::update_conflict_limit!(s, rephase, rephaseint, rephased, |n| crate::kimits::nlogpown(n, 3), false);
        s.statistics.conflicts = 7000;
        s.statistics.reordered = 1;
        crate::update_conflict_limit!(s, reorder, reorderint, reordered, |n: u64| n as f64, false);
        s.statistics.conflicts = 8000;
        s.statistics.probings = 4;
        crate::update_conflict_limit!(s, probe, probeint, probings, |n| crate::kimits::nlogpown(n, 1), true);
        s.statistics.conflicts = 9000;
        s.statistics.eliminations = 2;
        crate::update_conflict_limit!(s, eliminate, eliminateint, eliminations, |n| crate::kimits::nlogpown(n, 2), true);
        for t in [Timer::Reduce, Timer::Rephase, Timer::Reorder, Timer::Probe, Timer::Eliminate] {
            assert_eq!(effective_limit(s, t), stock_limit(s, t), "{}", t.name());
            assert_eq!(s.policy.timers[t as usize].fires, 1, "{}", t.name());
            let st = s.policy.timers[t as usize];
            assert_eq!(st.last_fire + st.stock_delta, stock_limit(s, t));
        }
        // The randec limit goes through the same macro and is not a timer.
        s.statistics.random_sequences = 1;
        crate::update_conflict_limit!(s, randec, randecint, random_sequences, |n| crate::kimits::logn(n), false);
        assert_eq!(s.policy.timers.iter().map(|t| t.fires).sum::<u64>(), 5);
    }

    #[test]
    fn multipliers_scale_the_stock_delta_with_a_floor() {
        let mut s = armed_solver();
        let i = Timer::Reduce as usize;
        let st = s.policy.timers[i];
        assert_eq!(st.last_fire, 0);
        assert_eq!(st.stock_delta, s.options.reduceinit as u64);
        s.policy.act.interval_mult[i] = 2.0;
        assert_eq!(effective_limit(&s, Timer::Reduce), 2 * st.stock_delta);
        s.policy.act.interval_mult[i] = 0.5;
        assert_eq!(effective_limit(&s, Timer::Reduce), st.stock_delta / 2);
        // Anything below the 0.1 x floor is held at the floor.
        s.policy.act.interval_mult[i] = 0.01;
        assert_eq!(effective_limit(&s, Timer::Reduce), interval_floor(st.stock_delta));
        s.policy.act.interval_mult[i] = 0.0;
        assert_eq!(effective_limit(&s, Timer::Reduce), interval_floor(st.stock_delta));
    }

    #[test]
    fn zero_multiplier_is_one_shot() {
        let mut owned = armed_solver();
        let s = &mut owned;
        let i = Timer::Probe as usize;
        s.policy.act.interval_mult[i] = 0.0;
        let floor = interval_floor(s.policy.timers[i].stock_delta);
        assert_eq!(effective_limit(s, Timer::Probe), floor);
        // The fire (through the real macro) consumes the one-shot.
        s.statistics.conflicts = floor + 1;
        s.statistics.probings = 1;
        crate::update_conflict_limit!(s, probe, probeint, probings, |n| crate::kimits::nlogpown(n, 1), true);
        assert_eq!(s.policy.act.interval_mult[i], 1.0);
        assert_eq!(effective_limit(s, Timer::Probe), s.limits.probe.conflicts);
        // A second fire in the same epoch is therefore at the stock interval.
        assert!(effective_limit(s, Timer::Probe) > s.statistics.conflicts + floor);
    }

    #[test]
    fn stock_margin_and_effort_are_exact() {
        let mut s = armed_solver();
        assert_eq!(restart_margin(&s), s.options.restartmargin as f64);
        assert_eq!(scale_effort_limit(&s, Effort::Sweep, 100, 1_000_000), 1_000_000);
        assert!(!skip_effort(&s, Effort::Sweep));
        s.policy.act.restart_margin = 2.0;
        assert_eq!(restart_margin(&s), 2.0 * s.options.restartmargin as f64);
        s.policy.act.effort_mult[Effort::Sweep as usize] = 0.5;
        assert_eq!(scale_effort_limit(&s, Effort::Sweep, 100, 1_000_100), 500_100);
        s.policy.act.effort_mult[Effort::Sweep as usize] = 0.0;
        assert!(skip_effort(&s, Effort::Sweep));
    }

    #[test]
    fn masking_forces_stock_where_a_knob_cannot_act() {
        let mut s = armed_solver();
        let mut act = STOCK;
        for m in act.interval_mult.iter_mut() {
            *m = 2.0;
        }
        act.restart_margin = 2.0;
        act.effort_mult[Effort::Sweep as usize] = 0.5;
        // Focused mode (init_limits starts focused with stable == 1).
        assert!(!s.stable);
        let m = mask(&s, act);
        assert_eq!(m.interval_mult[Timer::Rephase as usize], 1.0, "rephase is stable-only");
        assert_eq!(m.restart_margin, 2.0, "margin acts in focused mode");
        assert_eq!(m.interval_mult[Timer::Reorder as usize], 2.0, "reorder=2 runs in both modes");
        assert_eq!(m.interval_mult[Timer::Mode as usize], 2.0);
        assert_eq!(m.effort_mult[Effort::Sweep as usize], 0.5);
        // Stable mode flips rephase and the margin.
        s.stable = true;
        let m = mask(&s, act);
        assert_eq!(m.interval_mult[Timer::Rephase as usize], 2.0);
        assert_eq!(m.restart_margin, 1.0, "margin is a no-op in stable mode");
        // reorder=1 is stable-only.
        s.stable = false;
        s.options.reorder = 1;
        assert_eq!(mask(&s, act).interval_mult[Timer::Reorder as usize], 1.0);
        // Disabled passes and single-mode search stay stock.
        s.options.stable = 2;
        s.enabled.probe = false;
        s.enabled.eliminate = false;
        s.options.reduce = 0;
        s.options.sweep = 0;
        let m = mask(&s, act);
        assert_eq!(m.interval_mult[Timer::Mode as usize], 1.0);
        assert_eq!(m.interval_mult[Timer::Probe as usize], 1.0);
        assert_eq!(m.interval_mult[Timer::Eliminate as usize], 1.0);
        assert_eq!(m.interval_mult[Timer::Reduce as usize], 1.0);
        assert_eq!(m.effort_mult[Effort::Sweep as usize], 1.0);
    }

    #[test]
    fn epoch_clock_fires_d0_then_every_x_o_and_x_d() {
        let mut s = armed_solver();
        s.policy.obs_ticks = 100;
        s.policy.dec_ticks = 400;
        assert!(epoch_due(&s), "D0 is due before the first decide");
        epoch(&mut s);
        assert_eq!((s.policy.obs_epochs, s.policy.decisions), (1, 1));
        assert_eq!((s.policy.next_obs, s.policy.next_decision), (100, 400));
        s.statistics.search_ticks = 99;
        assert!(!epoch_due(&s));
        s.statistics.search_ticks = 100;
        assert!(epoch_due(&s));
        epoch(&mut s);
        assert_eq!((s.policy.obs_epochs, s.policy.decisions), (2, 1));
        assert_eq!(s.policy.next_obs, 200);
        // A jump over several boundaries takes one row and one decision and
        // realigns both grids.
        s.statistics.search_ticks = 1250;
        epoch(&mut s);
        assert_eq!((s.policy.obs_epochs, s.policy.decisions), (3, 2));
        assert_eq!((s.policy.next_obs, s.policy.next_decision), (1300, 1600));
    }

    #[test]
    fn random_sampling_is_seeded_centred_and_capped() {
        // Same seed, same draws.
        let mut a = 7u64;
        let mut b = 7u64;
        for _ in 0..50 {
            assert_eq!(sample_action(&mut a, 1.0), sample_action(&mut b, 1.0));
        }
        // Low temperature stays at stock; the zero entry never exceeds its cap.
        let mut rng = 3u64;
        let n = 20_000;
        let mut zeros = 0;
        let mut stock = 0;
        for _ in 0..n {
            let k = sample_index(&mut rng, INTERVAL_MENU.len(), INTERVAL_STOCK, 0.2, true);
            if k == 0 {
                zeros += 1;
            }
            if k == INTERVAL_STOCK {
                stock += 1;
            }
        }
        assert!(stock as f64 > 0.95 * n as f64, "stock share {}", stock);
        assert!(zeros as f64 <= (ZERO_CAP + 0.01) * n as f64, "zeros {}", zeros);
        // High temperature is close to uniform, except the capped zero entry.
        let mut counts = [0usize; 5];
        for _ in 0..n {
            counts[sample_index(&mut rng, 5, INTERVAL_STOCK, 1e6, true)] += 1;
        }
        assert!((counts[0] as f64) < 0.07 * n as f64, "{:?}", counts);
        for &c in &counts[1..] {
            assert!((c as f64) > 0.20 * n as f64 && (c as f64) < 0.28 * n as f64, "{:?}", counts);
        }
        // Segment lengths: mean about the parameter.
        let mut total = 0u64;
        for _ in 0..n {
            total += sample_segment_length(&mut rng, 5.0);
        }
        let mean = total as f64 / n as f64;
        assert!((4.6..5.4).contains(&mean), "mean segment {}", mean);
        assert_eq!(sample_segment_length(&mut rng, 1.0), 1);
    }

    #[test]
    fn sticky_mode_holds_an_action_for_a_segment() {
        let mut s = armed_solver();
        s.policy.mode = Mode::Random;
        s.policy.rng = 11;
        s.policy.segment_mean = 4.0;
        let mut changes = 0;
        let mut last = None;
        for _ in 0..400 {
            decide(&mut s);
            let a = s.policy.act;
            if last.is_some() && last != Some(a) {
                changes += 1;
            }
            last = Some(a);
        }
        // Far fewer changes than decisions (segments), but not zero.
        assert!(changes > 20 && changes < 200, "changes {}", changes);
    }

    #[test]
    fn sticky_segment_survives_masking_and_a_consumed_one_shot() {
        // A segment whose sampled action has a one-shot probe multiplier
        // and a non-stock restart margin: after the one-shot fires and
        // after a decision taken in stable mode (margin masked), the next
        // decision in focused mode re-issues both.
        let mut owned = armed_solver();
        let s = &mut owned;
        s.policy.mode = Mode::Random;
        s.policy.segment_left = 10;
        let mut sampled = STOCK;
        sampled.interval_mult[Timer::Probe as usize] = 0.0;
        sampled.restart_margin = 2.0;
        s.policy.segment_act = sampled;
        assert!(!s.stable);
        decide(s);
        assert_eq!(s.policy.act.interval_mult[Timer::Probe as usize], 0.0);
        assert_eq!(s.policy.act.restart_margin, 2.0);
        // The one-shot fires (real macro) and is consumed for this epoch.
        s.statistics.conflicts = 1;
        s.statistics.probings = 1;
        crate::update_conflict_limit!(s, probe, probeint, probings, |n| crate::kimits::nlogpown(n, 1), true);
        assert_eq!(s.policy.act.interval_mult[Timer::Probe as usize], 1.0);
        // A stable-mode decision masks the margin but keeps the segment.
        s.stable = true;
        decide(s);
        assert_eq!(s.policy.act.restart_margin, 1.0);
        assert_eq!(s.policy.act.interval_mult[Timer::Probe as usize], 0.0, "one-shot re-armed");
        // Back in focused mode the sampled margin is in force again.
        s.stable = false;
        decide(s);
        assert_eq!(s.policy.act.restart_margin, 2.0);
        assert_eq!(s.policy.segment_act, sampled, "the sampled action itself never changes");
    }
}
