// Not in kissat. `observe()`: the policy's input vector (plan §3, §6.4,
// §7 item 6e; step A.9, bead SAT-playground-p9m.6.9).
//
// The vector is built from the solver's counters at an observation
// boundary, the ring of the last 16 boundary snapshots, the per-pass
// recency state, the static block (policy_static.rs) and the horizon.
// Five blocks, in this order:
//
//   static    a chosen subset of the static features (counts as log1p,
//             fractions raw), plus a validity flag
//   global    the current dynamics: counts as log1p, sizes as ratios,
//             the averages of the current mode, the per-epoch learned
//             clause quality, and the horizon (fraction of budget used)
//   delta     the same dynamics as deltas over the last 1, 4 and 16
//             observation epochs, zero-filled with a validity flag until
//             enough snapshots exist
//   timer     per timer: log of the stock delta, progress towards the
//             stock deadline, fires, stock's would-fire flag
//   pass      per inprocessing pass: epochs since it last ran, times run,
//             yield and cost at its last run
//
// Normalization (mean and std per input) is not applied here: it lives in
// the weights file and the net applies it (policy_net.rs, step A.10).
//
// Purity (CONVENTIONS.md): everything here reads solver state and writes
// only `solver.policy`. No generator is drawn, no solver container is
// touched, no counter a heuristic reads is incremented, and nothing
// allocates after the first call (the output vector keeps its capacity,
// the ring and the recency table are fixed arrays), so the terminal row's
// observation can be built from the signal handler.
//
// Reproducibility: every input of `observe()` is in the log row that
// carries its output (the counters, the averages, the timer state, the
// epoch histogram, `wall_ns`, `work`) or in the header (horizon mode and
// budget) or the footer (the static block), and the ring and recency
// state are functions of the boundary rows before it. tools/policy_obs.py
// recomputes the vector from a log and tests/policy_obs.rs checks that it
// matches the logged one to the float.
//
// Horizon (plan §3.4): `SAT_POLICY_HORIZON=ticks:<B>` gives work / B (the
// deterministic form for parity, replay and fork children); otherwise a
// `SAT_LIMIT_TICKS` run uses work / limit, otherwise `SAT_WALL_LIMIT=<s>`
// gives elapsed wall / limit (the inference form, the one
// non-deterministic input), otherwise the feature is 0 with its validity
// flag 0.

use crate::internal::Solver;
use crate::policy::Timer;
use crate::policy_log::GLUE_BINS;
use crate::statistics::Statistics;

pub const WINDOWS: [usize; 3] = [1, 4, 16];
/// Ring of the last 16 boundary snapshots.
pub const RING: usize = 16;

pub const N_PASSES: usize = 12;
pub const PASS_NAMES: [&str; N_PASSES] = [
    "congruence",
    "substitute",
    "backbone",
    "vivify",
    "sweep",
    "transitive",
    "factor",
    "eliminate",
    "forward",
    "reduce",
    "rephase",
    "walk",
];

/// How the horizon (fraction of budget used) is computed.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Horizon {
    #[default]
    None,
    /// `SAT_POLICY_HORIZON=ticks:<B>`: work / B.
    Ticks(u64),
    /// `SAT_LIMIT_TICKS`: work / limit (the limit as set at start).
    Limit(u64),
    /// `SAT_WALL_LIMIT=<seconds>`: elapsed wall / limit.
    Wall(f64),
}

impl Horizon {
    pub fn name(self) -> &'static str {
        match self {
            Horizon::None => "none",
            Horizon::Ticks(_) => "ticks",
            Horizon::Limit(_) => "limit",
            Horizon::Wall(_) => "wall",
        }
    }

    /// The budget as a number for the header: B, the tick limit, wall
    /// seconds, or 0.
    pub fn budget(self) -> f64 {
        match self {
            Horizon::None => 0.0,
            Horizon::Ticks(b) | Horizon::Limit(b) => b as f64,
            Horizon::Wall(s) => s,
        }
    }
}

/// The raw quantities kept per boundary for the delta block.
#[derive(Clone, Copy, Default, Debug)]
pub struct Snapshot {
    pub conflicts: u64,
    pub decisions: u64,
    pub propagations: u64,
    pub search_ticks: u64,
    pub work: u64,
    pub units: u64,
    pub restarts: u64,
    pub reductions: u64,
    pub learned: u64,
    pub active: u64,
    pub irredundant: u64,
    pub redundant: u64,
    pub switched: u64,
    pub probing_ticks: u64,
    pub eliminate_resolutions: u64,
    /// Per pass: times run, cumulative yield, cumulative cost.
    pub pass_runs: [u64; N_PASSES],
    pub pass_yield: [u64; N_PASSES],
    pub pass_cost: [u64; N_PASSES],
}

/// Per-pass recency (plan §3.3): the boundary index at which the pass last
/// ran (`u64::MAX` = never), times run, and its yield and cost then.
#[derive(Clone, Copy, Debug)]
pub struct PassRecency {
    pub last_epoch: u64,
    pub runs: u64,
    pub last_yield: u64,
    pub last_cost: u64,
}

impl Default for PassRecency {
    fn default() -> Self {
        PassRecency {
            last_epoch: u64::MAX,
            runs: 0,
            last_yield: 0,
            last_cost: 0,
        }
    }
}

/// The observation state kept in `Policy`.
#[derive(Clone, Debug)]
pub struct ObsState {
    pub ring: [Snapshot; RING],
    /// Snapshots pushed so far; the newest is at `(pushed - 1) % RING`.
    pub pushed: u64,
    pub recency: [PassRecency; N_PASSES],
    /// The vector of the last `observe()`; keeps its capacity.
    pub obs: Vec<f32>,
    /// The wall reading (ns since the policy started) the last `observe()`
    /// used, and whether `obs` is fresh for the row about to be written.
    pub obs_wall_ns: u64,
    pub obs_fresh: bool,
    /// The horizon value and validity of the last `observe()`.
    pub horizon_value: f64,
    pub horizon_valid: bool,
}

impl Default for ObsState {
    fn default() -> Self {
        ObsState {
            ring: [Snapshot::default(); RING],
            pushed: 0,
            recency: [PassRecency::default(); N_PASSES],
            obs: Vec::with_capacity(512),
            obs_wall_ns: 0,
            obs_fresh: false,
            horizon_value: 0.0,
            horizon_valid: false,
        }
    }
}

/// Per pass (runs, yield, cost) from the counters. Passes without a tick
/// counter of their own (congruence, reduce, rephase) are costed by the
/// work outside search, `work - search_ticks`.
pub fn pass_counters(st: &Statistics) -> [(u64, u64, u64); N_PASSES] {
    let generic = st.work_clock().saturating_sub(st.search_ticks);
    [
        (st.closures, st.congruent, generic),
        (st.substitutions, st.substituted, st.substitute_ticks),
        (st.backbone_computations, st.backbone_units, st.backbone_ticks),
        (st.vivifications, st.vivified, st.vivify_ticks),
        (st.sweep, st.sweep_equivalences + st.sweep_units, st.kitten_ticks),
        (
            st.transitive_reductions,
            st.transitive_reduced + st.transitive_units,
            st.transitive_ticks,
        ),
        (st.factorizations, st.factored, st.factor_ticks),
        (st.eliminations, st.eliminated, st.eliminate_resolutions),
        (
            st.forward_subsumptions,
            st.forward_subsumed + st.forward_strengthened,
            st.forward_steps,
        ),
        (st.reductions, st.clauses_reduced, generic),
        (st.rephased, 0, generic),
        (st.walks, st.walk_improved, st.walk_steps),
    ]
}

/// The snapshot of the current state.
pub fn snapshot(solver: &Solver) -> Snapshot {
    let st = &solver.statistics;
    let pc = pass_counters(st);
    let mut s = Snapshot {
        conflicts: st.conflicts,
        decisions: st.decisions,
        propagations: st.propagations,
        search_ticks: st.search_ticks,
        work: st.work_clock(),
        units: st.units,
        restarts: st.restarts,
        reductions: st.reductions,
        learned: st.clauses_learned,
        active: solver.active as u64,
        irredundant: st.clauses_irredundant,
        redundant: st.clauses_redundant,
        switched: st.switched,
        probing_ticks: st.probing_ticks,
        eliminate_resolutions: st.eliminate_resolutions,
        pass_runs: [0; N_PASSES],
        pass_yield: [0; N_PASSES],
        pass_cost: [0; N_PASSES],
    };
    for (i, (r, y, c)) in pc.iter().enumerate() {
        s.pass_runs[i] = *r;
        s.pass_yield[i] = *y;
        s.pass_cost[i] = *c;
    }
    s
}

/// Update the per-pass recency from the passes that ran since the last
/// pushed snapshot; `epoch` is the boundary index being observed. Called
/// at every boundary before `observe()`, never on the terminal row.
pub fn update_recency(solver: &mut Solver, epoch: u64) {
    let now = snapshot(solver);
    let o = &mut solver.policy.obs_state;
    if o.pushed == 0 {
        // Before D0 nothing ran inside an epoch; a pass that ran during
        // preprocessing counts as run at D0 with its whole yield and cost.
        for i in 0..N_PASSES {
            if now.pass_runs[i] > 0 {
                o.recency[i] = PassRecency {
                    last_epoch: epoch,
                    runs: now.pass_runs[i],
                    last_yield: now.pass_yield[i],
                    last_cost: now.pass_cost[i],
                };
            }
        }
        return;
    }
    let last = o.ring[((o.pushed - 1) % RING as u64) as usize];
    for i in 0..N_PASSES {
        if now.pass_runs[i] > last.pass_runs[i] {
            o.recency[i] = PassRecency {
                last_epoch: epoch,
                runs: now.pass_runs[i],
                last_yield: now.pass_yield[i].saturating_sub(last.pass_yield[i]),
                last_cost: now.pass_cost[i].saturating_sub(last.pass_cost[i]),
            };
        }
    }
}

/// Push the current state as the newest boundary snapshot.
pub fn push_snapshot(solver: &mut Solver) {
    let s = snapshot(solver);
    let o = &mut solver.policy.obs_state;
    o.ring[(o.pushed % RING as u64) as usize] = s;
    o.pushed += 1;
}

#[inline]
fn lg(x: f64) -> f64 {
    if x > 0.0 {
        x.ln_1p()
    } else {
        0.0
    }
}

#[inline]
fn ratio(a: f64, b: f64) -> f64 {
    if b > 0.0 {
        a / b
    } else {
        0.0
    }
}

/// The horizon feature for the current state: (value, valid).
pub fn horizon(solver: &Solver, wall_ns: u64) -> (f64, bool) {
    let work = solver.statistics.work_clock() as f64;
    match solver.policy.horizon {
        Horizon::Ticks(b) | Horizon::Limit(b) => ((work / (b.max(1)) as f64).min(1.0), true),
        Horizon::Wall(secs) => (((wall_ns as f64 * 1e-9) / secs).min(1.0), true),
        Horizon::None => (0.0, false),
    }
}

/// Emit every observation entry as (name, value). With `want_names ==
/// false` no name is formatted (the row path allocates nothing).
fn build(solver: &Solver, wall_ns: u64, want_names: bool, mut emit: impl FnMut(&str, f64)) {
    let p = &solver.policy;
    let st = &solver.statistics;
    let o = &p.obs_state;
    macro_rules! e {
        ($v:expr, $($name:tt)+) => {{
            let v: f64 = $v;
            if want_names {
                emit(&format!($($name)+), v)
            } else {
                emit("", v)
            }
        }};
    }

    // ---- static block ---------------------------------------------------
    let f = &p.static_;
    e!(f.computed as u8 as f64, "s_valid");
    e!(lg(f.vars), "s_vars");
    e!(lg(f.clauses), "s_clauses");
    e!(lg(f.literals), "s_literals");
    e!(lg(f.clauses_per_var), "s_clauses_per_var");
    e!(lg(f.lits_per_clause), "s_lits_per_clause");
    e!(lg(f.len_max), "s_len_max");
    for i in 0..6 {
        e!(f.len_hist[i], "s_len_hist{}", i);
    }
    e!(f.giant_share, "s_giant_share");
    e!(lg(f.occ_mean), "s_occ_mean");
    e!(lg(f.occ_var), "s_occ_var");
    e!(lg(f.occ_max), "s_occ_max");
    e!(f.occ_entropy, "s_occ_entropy");
    e!(f.near_singleton_frac, "s_near_singleton_frac");
    e!(f.pure_frac, "s_pure_frac");
    e!(f.balance_mean, "s_balance_mean");
    e!(f.clause_pos_frac_mean, "s_clause_pos_frac_mean");
    e!(f.horn_frac, "s_horn_frac");
    e!(f.reverse_horn_frac, "s_reverse_horn_frac");
    e!(f.binary_frac, "s_binary_frac");
    e!(f.big_touched_frac, "s_big_touched_frac");
    e!(lg(f.big_deg_mean), "s_big_deg_mean");
    e!(lg(f.big_deg_max), "s_big_deg_max");
    e!(f.big_root_frac, "s_big_root_frac");
    e!(f.big_leaf_frac, "s_big_leaf_frac");
    e!(f.span_mean, "s_span_mean");
    e!(f.span_median, "s_span_median");
    e!(f.span_small_frac, "s_span_small_frac");
    e!(f.consecutive_frac, "s_consecutive_frac");
    e!(f.class_small, "s_class_small");
    e!(f.class_bigbig, "s_class_bigbig");
    e!(lg(f.scc_count), "s_scc_count");
    e!(lg(f.scc_max), "s_scc_max");
    e!(lg(f.bfs_depth_mean), "s_bfs_depth_mean");
    e!(lg(f.bfs_reach_mean), "s_bfs_reach_mean");
    e!(f.bfs_reach_frac, "s_bfs_reach_frac");
    e!(f.bfs_failed_frac, "s_bfs_failed_frac");
    e!(f.gate_output_frac, "s_gate_output_frac");
    e!(lg(f.matched), "s_gates_matched");
    e!(lg(f.congruent_equivalences), "s_congruent_equivalences");
    e!(lg(f.backbone_units), "s_backbone_units");
    e!((f.backbone_budget_hits > 0.0) as u8 as f64, "s_backbone_budget_hit");
    e!(lg(f.sweep_equivalences + f.sweep_units), "s_sweep_yield");
    e!((f.sweep_budget_hits > 0.0) as u8 as f64, "s_sweep_budget_hit");
    // `kitten_solved` counts every kitten call, UNKNOWN ones included.
    e!(ratio(f.kitten_unknown, f.kitten_solved), "s_kitten_unknown_frac");
    e!(f.fast_eliminated_per_var, "s_fast_eliminated_per_var");
    for i in 0..crate::policy_static::N_LUCKY {
        e!(f.lucky_outcome[i] / 3.0, "s_lucky_outcome_{}", crate::policy_static::LUCKY_NAMES[i]);
        e!(f.lucky_level_frac[i], "s_lucky_level_{}", crate::policy_static::LUCKY_NAMES[i]);
    }
    e!(f.pre_vars_over_original, "s_pre_vars_over_original");
    e!(f.pre_clauses_over_original, "s_pre_clauses_over_original");
    e!(lg(f.pre_probing_ticks), "s_pre_probing_ticks");
    e!(lg(f.pre_kitten_ticks), "s_pre_kitten_ticks");
    e!(lg(f.pre_factor_ticks), "s_pre_factor_ticks");
    e!(lg(f.pre_work), "s_pre_work");

    // ---- global block ---------------------------------------------------
    let now = snapshot(solver);
    let conflicts = now.conflicts as f64;
    let search_ticks = now.search_ticks as f64;
    let work = now.work as f64;
    let vars0 = if f.vars > 0.0 { f.vars } else { solver.active.max(1) as f64 };
    e!(lg(conflicts), "g_conflicts");
    e!(lg(now.decisions as f64), "g_decisions");
    e!(lg(now.propagations as f64), "g_propagations");
    e!(lg(search_ticks), "g_search_ticks");
    e!(lg(work), "g_work");
    e!(lg(ratio(search_ticks, conflicts)), "g_ticks_per_conflict");
    e!(ratio(search_ticks, work), "g_search_frac");
    e!(ratio(now.probing_ticks as f64, work), "g_probing_frac");
    e!(
        ratio(crate::statistics::K_RES as f64 * now.eliminate_resolutions as f64, work),
        "g_eliminate_frac"
    );
    e!(ratio(now.active as f64, vars0), "g_active_frac");
    e!(lg(now.irredundant as f64), "g_irredundant");
    e!(lg(st.clauses_binary as f64), "g_binary");
    e!(lg(now.redundant as f64), "g_redundant");
    e!(ratio(now.redundant as f64, now.irredundant as f64 + st.clauses_binary as f64), "g_redundant_ratio");
    e!(lg(solver.arena.size_wards() as f64), "g_arena_wards");
    e!(lg(now.units as f64), "g_units");
    e!(ratio(solver.trail.len() as f64, solver.active.max(1) as f64), "g_trail_frac");
    e!(lg(solver.level as f64), "g_level");
    e!(ratio(solver.unassigned as f64, solver.active.max(1) as f64), "g_unassigned_frac");
    let a = &solver.averages[solver.stable as usize];
    e!(a.fast_glue.value, "g_avg_fast_glue");
    e!(a.slow_glue.value, "g_avg_slow_glue");
    e!(ratio(a.fast_glue.value, a.slow_glue.value), "g_avg_glue_ratio");
    e!(lg(a.level.value), "g_avg_level");
    e!(lg(a.size.value), "g_avg_size");
    e!(lg(a.trail.value), "g_avg_trail");
    e!(a.decision_rate.value, "g_avg_decision_rate");
    e!(solver.stable as u8 as f64, "g_stable");
    e!(lg(st.search_ticks.saturating_sub(solver.mode.ticks) as f64), "g_ticks_since_switch");
    e!(lg(now.switched as f64), "g_switched");
    e!(lg(now.restarts as f64), "g_restarts");
    e!(ratio(st.restarts_reused_trails as f64, now.restarts as f64), "g_reused_trail_frac");
    e!(lg(now.reductions as f64), "g_reductions");
    e!(lg(now.learned as f64), "g_learned");
    let learned = p.epoch_learned as f64;
    for i in 0..GLUE_BINS {
        e!(ratio(p.epoch_glue[i] as f64, learned), "g_epoch_glue{}", i);
    }
    e!(lg(learned), "g_epoch_learned");
    e!(ratio(p.epoch_learned_size as f64, learned), "g_epoch_size_mean");
    e!(ratio(p.epoch_learned_glue as f64, learned), "g_epoch_glue_mean");
    e!(lg(solver.bounds.eliminate.max_bound_completed as f64), "g_elim_bound");
    let (h, hv) = horizon(solver, wall_ns);
    e!(h, "g_horizon");
    e!(hv as u8 as f64, "g_horizon_valid");

    // ---- delta block ----------------------------------------------------
    for &w in WINDOWS.iter() {
        let valid = o.pushed >= w as u64;
        let prev = if valid {
            o.ring[((o.pushed - w as u64) % RING as u64) as usize]
        } else {
            Snapshot::default()
        };
        let d = |a: u64, b: u64| if valid { a.saturating_sub(b) as f64 } else { 0.0 };
        let dconf = d(now.conflicts, prev.conflicts);
        let ddec = d(now.decisions, prev.decisions);
        let dprop = d(now.propagations, prev.propagations);
        let dst = d(now.search_ticks, prev.search_ticks);
        let dwork = d(now.work, prev.work);
        e!(valid as u8 as f64, "d{}_valid", w);
        e!(lg(dconf), "d{}_conflicts", w);
        e!(lg(ddec), "d{}_decisions", w);
        e!(lg(dprop), "d{}_propagations", w);
        e!(lg(dst), "d{}_search_ticks", w);
        e!(lg(dwork), "d{}_work", w);
        e!(ratio(dconf, dst) * 1e3, "d{}_conflicts_per_kilotick", w);
        e!(lg(ratio(dprop, ddec)), "d{}_propagations_per_decision", w);
        e!(ratio(dst, dwork), "d{}_search_frac", w);
        e!(lg(d(now.units, prev.units)), "d{}_units", w);
        e!(lg(d(now.restarts, prev.restarts)), "d{}_restarts", w);
        e!(lg(d(now.learned, prev.learned)), "d{}_learned", w);
        e!(lg(d(now.reductions, prev.reductions)), "d{}_reductions", w);
        e!(lg(d(now.switched, prev.switched)), "d{}_switched", w);
        let signed = |a: u64, b: u64| if valid { a as f64 - b as f64 } else { 0.0 };
        e!(ratio(signed(now.active, prev.active), vars0), "d{}_active_frac", w);
        e!(
            ratio(signed(now.irredundant, prev.irredundant), (prev.irredundant.max(1)) as f64),
            "d{}_irredundant_ratio",
            w
        );
        e!(
            ratio(signed(now.redundant, prev.redundant), (prev.redundant.max(1)) as f64),
            "d{}_redundant_ratio",
            w
        );
    }

    // ---- timer block ----------------------------------------------------
    for t in Timer::ALL {
        let ts = &p.timers[t as usize];
        let clock = if t == Timer::Mode && solver.limits.mode.count & 1 != 0 {
            st.search_ticks
        } else {
            st.conflicts
        };
        let name = t.name();
        e!(lg(ts.stock_delta as f64), "t_{}_log_delta", name);
        e!(
            ratio(clock.saturating_sub(ts.last_fire) as f64, ts.stock_delta.max(1) as f64).min(4.0),
            "t_{}_progress",
            name
        );
        e!(lg(ts.fires as f64), "t_{}_fires", name);
        e!(
            crate::policy::stock_would_fire(solver, t) as u8 as f64,
            "t_{}_would_fire",
            name
        );
    }

    // ---- pass block -----------------------------------------------------
    let epoch = p.obs_epochs;
    for (i, name) in PASS_NAMES.iter().enumerate() {
        let r = &o.recency[i];
        let never = r.last_epoch == u64::MAX;
        let since = if never { epoch + 1 } else { epoch.saturating_sub(r.last_epoch) };
        e!(never as u8 as f64, "p_{}_never", name);
        e!(lg(since as f64), "p_{}_since", name);
        e!(lg(r.runs as f64), "p_{}_runs", name);
        e!(lg(r.last_yield as f64), "p_{}_yield", name);
        e!(lg(r.last_cost as f64), "p_{}_cost", name);
    }
}

/// The names of every entry, in order (header path; allocates).
pub fn names() -> Vec<String> {
    let s = crate::internal::init();
    let mut v = Vec::with_capacity(512);
    build(&s, 0, true, |n, _| v.push(n.to_string()));
    v
}

/// Compute the observation into `policy.obs_state.obs` for the current
/// state and the given wall reading (ns since the policy started).
pub fn observe(solver: &mut Solver, wall_ns: u64) {
    let mut obs = std::mem::take(&mut solver.policy.obs_state.obs);
    obs.clear();
    build(solver, wall_ns, false, |_, v| obs.push(v as f32));
    let (h, hv) = horizon(solver, wall_ns);
    let o = &mut solver.policy.obs_state;
    o.obs = obs;
    o.obs_wall_ns = wall_ns;
    o.obs_fresh = true;
    o.horizon_value = h;
    o.horizon_valid = hv;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_the_vector_matches_them() {
        let names = names();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate observation name");
        assert!(names.len() > 150 && names.len() < 400, "{} entries", names.len());
        let mut s = crate::internal::init();
        s.statistics.searches = 1;
        s.policy.on = true;
        crate::kimits::init_limits(&mut s);
        observe(&mut s, 0);
        assert_eq!(s.policy.obs_state.obs.len(), names.len());
        assert!(s.policy.obs_state.obs_fresh);
        // Nothing is valid before any snapshot; the horizon is off.
        let idx = |n: &str| names.iter().position(|x| x == n).unwrap_or_else(|| panic!("{}", n));
        let obs = &s.policy.obs_state.obs;
        assert_eq!(obs[idx("d1_valid")], 0.0);
        assert_eq!(obs[idx("d16_valid")], 0.0);
        assert_eq!(obs[idx("g_horizon_valid")], 0.0);
        assert_eq!(obs[idx("s_valid")], 0.0);
        assert_eq!(obs[idx("p_eliminate_never")], 1.0);
        // The second call allocates nothing: capacity is kept.
        let cap = s.policy.obs_state.obs.capacity();
        observe(&mut s, 5);
        assert_eq!(s.policy.obs_state.obs.capacity(), cap);
    }

    #[test]
    fn deltas_and_recency_follow_the_ring() {
        let mut s = crate::internal::init();
        s.statistics.searches = 1;
        s.policy.on = true;
        crate::kimits::init_limits(&mut s);
        let names = names();
        let idx = |n: &str| names.iter().position(|x| x == n).unwrap();
        // D0: nothing ran yet.
        update_recency(&mut s, 0);
        observe(&mut s, 0);
        push_snapshot(&mut s);
        // Epoch 1: 100 conflicts, one elimination with 7 variables.
        s.statistics.conflicts = 100;
        s.statistics.search_ticks = 1000;
        s.statistics.ticks = 1000;
        s.statistics.eliminations = 1;
        s.statistics.eliminated = 7;
        s.statistics.eliminate_resolutions = 50;
        update_recency(&mut s, 1);
        s.policy.obs_epochs = 1;
        observe(&mut s, 0);
        let obs = s.policy.obs_state.obs.clone();
        assert_eq!(obs[idx("d1_valid")], 1.0);
        assert_eq!(obs[idx("d4_valid")], 0.0);
        assert!((obs[idx("d1_conflicts")] - (101f64).ln() as f32).abs() < 1e-6);
        assert_eq!(obs[idx("p_eliminate_never")], 0.0);
        assert_eq!(obs[idx("p_eliminate_since")], 0.0);
        assert!((obs[idx("p_eliminate_yield")] - (8f64).ln() as f32).abs() < 1e-6);
        assert!((obs[idx("p_eliminate_cost")] - (51f64).ln() as f32).abs() < 1e-6);
        push_snapshot(&mut s);
        // Epochs 2..5 without passes: eliminate is 4 epochs old at epoch 5,
        // the 4-window is valid, the 16-window is not.
        for e in 2..=5 {
            s.statistics.conflicts += 10;
            update_recency(&mut s, e);
            s.policy.obs_epochs = e;
            observe(&mut s, 0);
            push_snapshot(&mut s);
        }
        let obs = &s.policy.obs_state.obs;
        assert_eq!(obs[idx("d4_valid")], 1.0);
        assert_eq!(obs[idx("d16_valid")], 0.0);
        assert!((obs[idx("d4_conflicts")] - (41f64).ln() as f32).abs() < 1e-6);
        assert!((obs[idx("p_eliminate_since")] - (5f64).ln() as f32).abs() < 1e-6);
    }

    #[test]
    fn horizon_modes() {
        let mut s = crate::internal::init();
        s.statistics.ticks = 500;
        s.policy.horizon = Horizon::None;
        assert_eq!(horizon(&s, 0), (0.0, false));
        s.policy.horizon = Horizon::Ticks(1000);
        assert_eq!(horizon(&s, 0), (0.5, true));
        // SAT_LIMIT_TICKS is relative to the work at start, so the limit
        // recorded for the horizon is the absolute one.
        crate::internal::set_ticks_limit(&mut s, 2000);
        s.policy.horizon = Horizon::Limit(s.limits.ticks);
        assert_eq!(horizon(&s, 0), (0.2, true));
        s.policy.horizon = Horizon::Wall(10.0);
        let (h, v) = horizon(&s, 2_500_000_000);
        assert!(v && (h - 0.25).abs() < 1e-12, "{}", h);
        assert_eq!(horizon(&s, 25_000_000_000), (1.0, true), "clamped");
    }
}
