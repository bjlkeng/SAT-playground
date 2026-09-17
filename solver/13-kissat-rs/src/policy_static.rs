// Not in kissat. Actor-tier static features for the RL scheduler (plan
// §2.4, §3.5, §3.6 and Appendix A; step A.7, bead SAT-playground-p9m.6.7).
//
// One pass over the active variables and the non-garbage irredundant
// clauses, run once after preprocessing and `classify()` (search.rs), plus
// a snapshot of the preprocessing yields kissat's own passes already paid
// for. The result is a flat list of (group, name, value) features:
//
//   identity groups  shape, occurrence, big, locality, classify
//   response groups  scc, bfs, gates, backbone, sweep, kitten, fastel,
//                    lucky, warmup, preprocess
//
// Every group is named in the log header (`static_schema`) so the offline
// tools can mask it in an ablation; `locality` is the one group that is
// not invariant under variable renaming (plan §3.6). The values go into
// the log footer (they exist only after preprocessing, which is after the
// header is written) and into `observe()` at D0 (step A.9).
//
// Purity (CONVENTIONS.md): this module reads solver state and writes only
// to `solver.policy`. It draws nothing from `solver.random` (the BFS
// sample uses its own generator seeded from the policy seed), touches no
// solver container, and increments no counter a heuristic reads. Its cost
// is wall only: no tick counter is charged, so the work clock of a run
// with the policy on equals the work clock of the same run with it off.
//
// The `Notes` accumulators are fed by two-line `if solver.policy.on`
// hooks in lucky.rs (outcome per pattern), warmup.rs (assigned fraction),
// backbone.rs and sweep.rs (effort-budget hits) and substitute.rs (the SCC
// sizes of its Tarjan walk), all pure increments.

use crate::internal::Solver;
use crate::literal::{idx, not};
use crate::watch::{watch_is_binary, watch_lit};

/// Exact clause-length histogram up to this length; longer clauses go into
/// log2 buckets. Both feed the "longest 1 % of clauses" share.
const LEN_EXACT: usize = 1024;
const LEN_LOG_BINS: usize = 40;
/// Span histogram bins for the locality median.
const SPAN_BINS: usize = 64;
/// BFS sample: top-degree literals, minimum and maximum random literals,
/// the relative standard error that stops sampling, and the edge cap.
const BFS_TOP: usize = 32;
const BFS_RANDOM_MIN: usize = 16;
const BFS_RANDOM_MAX: usize = 256;
const BFS_RELATIVE_SE: f64 = 0.1;
const BFS_EDGE_CAP: u64 = 1_000_000;

// ---------------------------------------------------------------------------
// Notes: pure accumulators fed by hooks in the passes
// ---------------------------------------------------------------------------

/// The lucky patterns of lucky.rs, in the order the driver tries them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum LuckyPattern {
    AllTrue = 0,
    AllFalse = 1,
    ForwardFalse = 2,
    ForwardTrue = 3,
    BackwardFalse = 4,
    BackwardTrue = 5,
}

pub const N_LUCKY: usize = 6;

pub const LUCKY_NAMES: [&str; N_LUCKY] = [
    "all_true",
    "all_false",
    "forward_false",
    "forward_true",
    "backward_false",
    "backward_true",
];

/// What a lucky pattern did: 0 = not tried (or aborted on the work
/// budget), 1 = conflict, 2 = solved (SAT), 3 = refuted (UNSAT at root).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LuckyNote {
    pub outcome: u8,
    /// Decision level reached when the pattern stopped: how far the
    /// assignment got before a conflict, or all of it when it solved.
    pub level: u32,
    /// Active variables when the pattern ran (the level's denominator).
    pub active: u32,
    /// Conflicts seen by the pattern before it stopped.
    pub conflicts: u32,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Notes {
    pub lucky: [LuckyNote; N_LUCKY],
    /// Lucky passes run so far (early and late).
    pub lucky_runs: u64,
    /// Warmups run, and the assigned fraction and level at the end of the
    /// latest one.
    pub warmups: u64,
    pub warmup_assigned_frac: f64,
    pub warmup_level: u64,
    /// Times a pass stopped on its effort budget.
    pub backbone_budget_hits: u64,
    pub sweep_budget_hits: u64,
    /// Non-trivial SCCs found by substitute's walk: count, largest, summed
    /// size, and a size histogram (2, 3-4, 5-16, 17+).
    pub scc_count: u64,
    pub scc_max: u64,
    pub scc_sum: u64,
    pub scc_hist: [u64; 4],
}

/// Hook for lucky.rs: the outcome of one pattern (see `LuckyNote`).
#[inline]
pub fn note_lucky(solver: &mut Solver, pattern: LuckyPattern, outcome: u8, level: u32, conflicts: u32) {
    let active = solver.active;
    let n = &mut solver.policy.notes.lucky[pattern as usize];
    n.outcome = outcome;
    n.level = level;
    n.active = active;
    n.conflicts = conflicts;
}

#[inline]
pub fn note_lucky_run(solver: &mut Solver) {
    solver.policy.notes.lucky_runs += 1;
}

/// Hook for warmup.rs, before its backtrack: the assigned fraction and the
/// level the warm-up reached.
#[inline]
pub fn note_warmup(solver: &mut Solver) {
    let active = solver.active as f64;
    let assigned = solver.active.saturating_sub(solver.unassigned) as f64;
    let n = &mut solver.policy.notes;
    n.warmups += 1;
    n.warmup_assigned_frac = if active > 0.0 { assigned / active } else { 0.0 };
    n.warmup_level = solver.level as u64;
}

#[inline]
pub fn note_backbone_budget_hit(solver: &mut Solver) {
    solver.policy.notes.backbone_budget_hits += 1;
}

#[inline]
pub fn note_sweep_budget_hit(solver: &mut Solver) {
    solver.policy.notes.sweep_budget_hits += 1;
}

/// Hook for substitute.rs: one non-trivial SCC of `size` literals.
#[inline]
pub fn note_scc(solver: &mut Solver, size: usize) {
    let n = &mut solver.policy.notes;
    let size = size as u64;
    n.scc_count += 1;
    n.scc_sum += size;
    if size > n.scc_max {
        n.scc_max = size;
    }
    let bin = match size {
        0..=2 => 0,
        3..=4 => 1,
        5..=16 => 2,
        _ => 3,
    };
    n.scc_hist[bin] += 1;
}

// ---------------------------------------------------------------------------
// The features
// ---------------------------------------------------------------------------

/// Every static feature, as f64. Counts are exact up to 2^53. The
/// `visit` function below is the single definition of names, groups and
/// values, so the header schema, the footer JSON and `observe()` cannot
/// drift from each other.
#[derive(Clone, Debug, Default)]
pub struct StaticFeatures {
    pub computed: bool,
    pub cost_ns: u64,
    // shape (identity)
    pub vars: f64,
    pub active_frac: f64,
    pub clauses: f64,
    pub literals: f64,
    pub clauses_per_var: f64,
    pub lits_per_clause: f64,
    pub len_var: f64,
    pub len_max: f64,
    pub len_hist: [f64; 6],
    pub giant_share: f64,
    pub duplicated: f64,
    // occurrence and polarity (identity)
    pub occ_mean: f64,
    pub occ_var: f64,
    pub occ_max: f64,
    pub occ_entropy: f64,
    pub near_singleton_frac: f64,
    pub pure_frac: f64,
    pub unused_frac: f64,
    pub balance_mean: f64,
    pub balance_hist: [f64; 4],
    pub clause_pos_frac_mean: f64,
    pub horn_frac: f64,
    pub reverse_horn_frac: f64,
    // binary implication graph (identity)
    pub binary_frac: f64,
    pub big_touched_frac: f64,
    pub big_deg_mean: f64,
    pub big_deg_var: f64,
    pub big_deg_max: f64,
    pub big_root_frac: f64,
    pub big_leaf_frac: f64,
    // locality (identity, not shuffle-invariant)
    pub span_mean: f64,
    pub span_median: f64,
    pub span_small_frac: f64,
    pub consecutive_frac: f64,
    // classify (identity)
    pub class_small: f64,
    pub class_bigbig: f64,
    // sccs from substitute (response)
    pub scc_count: f64,
    pub scc_max: f64,
    pub scc_sum: f64,
    pub scc_hist: [f64; 4],
    pub substituted: f64,
    // bounded BFS over the BIG (response)
    pub bfs_samples: f64,
    pub bfs_random: f64,
    pub bfs_depth_mean: f64,
    pub bfs_depth_max: f64,
    pub bfs_reach_mean: f64,
    pub bfs_reach_max: f64,
    pub bfs_reach_frac: f64,
    pub bfs_failed_frac: f64,
    pub bfs_edges: f64,
    pub bfs_capped: f64,
    // congruence gate census (response)
    pub gates: f64,
    pub gates_ands: f64,
    pub gates_xors: f64,
    pub gates_ites: f64,
    pub gate_output_frac: f64,
    pub matched: f64,
    pub matched_ands: f64,
    pub matched_xors: f64,
    pub matched_ites: f64,
    pub congruent_equivalences: f64,
    pub congruent_units: f64,
    pub congruent_arity_ands: f64,
    pub congruent_arity_xors: f64,
    pub closures: f64,
    // backbone (response)
    pub backbone_computations: f64,
    pub backbone_units: f64,
    pub backbone_units_per_var: f64,
    pub backbone_probes: f64,
    pub backbone_implied: f64,
    pub backbone_rounds: f64,
    pub backbone_ticks: f64,
    pub backbone_budget_hits: f64,
    // sweep (response)
    pub sweeps: f64,
    pub sweep_completed: f64,
    pub sweep_variables: f64,
    pub sweep_equivalences: f64,
    pub sweep_units: f64,
    pub sweep_solved: f64,
    pub sweep_sat: f64,
    pub sweep_unsat: f64,
    pub sweep_budget_hits: f64,
    // kitten (response)
    pub kitten_solved: f64,
    pub kitten_sat: f64,
    pub kitten_unsat: f64,
    pub kitten_unknown: f64,
    pub kitten_ticks: f64,
    // fast elimination and factoring (response)
    pub fast_eliminated: f64,
    pub fast_eliminated_per_var: f64,
    pub fast_strengthened: f64,
    pub fast_subsumed: f64,
    pub factored: f64,
    // lucky (response): per pattern outcome, level fraction, conflicts
    pub lucky_runs: f64,
    pub lucky_outcome: [f64; N_LUCKY],
    pub lucky_level_frac: [f64; N_LUCKY],
    pub lucky_conflicts: [f64; N_LUCKY],
    // warmup (response)
    pub warmups: f64,
    pub warmup_assigned_frac: f64,
    pub warmup_level: f64,
    // preprocessing yields and cost by pass (response); the two ratios
    // are over the original counts and can exceed 1 (factoring adds
    // variables and clauses)
    pub pre_vars_over_original: f64,
    pub pre_clauses_over_original: f64,
    pub pre_units: f64,
    pub pre_probing_ticks: f64,
    pub pre_backbone_ticks: f64,
    pub pre_substitute_ticks: f64,
    pub pre_transitive_ticks: f64,
    pub pre_factor_ticks: f64,
    pub pre_kitten_ticks: f64,
    pub pre_vivify_ticks: f64,
    pub pre_dense_ticks: f64,
    pub pre_eliminate_resolutions: f64,
    pub pre_walk_steps: f64,
    pub pre_ticks: f64,
    pub pre_work: f64,
}

pub const N_GROUPS: usize = 15;

/// Group name and kind (identity v response, plan §3.6), in visit order.
pub const GROUPS: [(&str, &str); N_GROUPS] = [
    ("shape", "identity"),
    ("occurrence", "identity"),
    ("big", "identity"),
    ("locality", "identity"),
    ("classify", "identity"),
    ("scc", "response"),
    ("bfs", "response"),
    ("gates", "response"),
    ("backbone", "response"),
    ("sweep", "response"),
    ("kitten", "response"),
    ("fastel", "response"),
    ("lucky", "response"),
    ("warmup", "response"),
    ("preprocess", "response"),
];

impl StaticFeatures {
    /// Visit every feature as (group, name, value), in a fixed order. Names
    /// are static strings, so a visit allocates nothing.
    pub fn visit(&self, mut f: impl FnMut(&'static str, &'static str, f64)) {
        const LEN_NAMES: [&str; 6] = ["len_1", "len_2", "len_3", "len_4_8", "len_9_32", "len_33_up"];
        const BAL_NAMES: [&str; 4] = ["balance_q1", "balance_q2", "balance_q3", "balance_q4"];
        const SCC_NAMES: [&str; 4] = ["scc_size_2", "scc_size_3_4", "scc_size_5_16", "scc_size_17_up"];
        const LUCKY_OUTCOME: [&str; N_LUCKY] = [
            "outcome_all_true",
            "outcome_all_false",
            "outcome_forward_false",
            "outcome_forward_true",
            "outcome_backward_false",
            "outcome_backward_true",
        ];
        const LUCKY_LEVEL: [&str; N_LUCKY] = [
            "level_frac_all_true",
            "level_frac_all_false",
            "level_frac_forward_false",
            "level_frac_forward_true",
            "level_frac_backward_false",
            "level_frac_backward_true",
        ];
        const LUCKY_CONFLICTS: [&str; N_LUCKY] = [
            "conflicts_all_true",
            "conflicts_all_false",
            "conflicts_forward_false",
            "conflicts_forward_true",
            "conflicts_backward_false",
            "conflicts_backward_true",
        ];
        let g = "shape";
        f(g, "vars", self.vars);
        f(g, "active_frac", self.active_frac);
        f(g, "clauses", self.clauses);
        f(g, "literals", self.literals);
        f(g, "clauses_per_var", self.clauses_per_var);
        f(g, "lits_per_clause", self.lits_per_clause);
        f(g, "len_var", self.len_var);
        f(g, "len_max", self.len_max);
        for (i, n) in LEN_NAMES.iter().enumerate() {
            f(g, n, self.len_hist[i]);
        }
        f(g, "giant_share", self.giant_share);
        f(g, "duplicated", self.duplicated);
        let g = "occurrence";
        f(g, "occ_mean", self.occ_mean);
        f(g, "occ_var", self.occ_var);
        f(g, "occ_max", self.occ_max);
        f(g, "occ_entropy", self.occ_entropy);
        f(g, "near_singleton_frac", self.near_singleton_frac);
        f(g, "pure_frac", self.pure_frac);
        f(g, "unused_frac", self.unused_frac);
        f(g, "balance_mean", self.balance_mean);
        for (i, n) in BAL_NAMES.iter().enumerate() {
            f(g, n, self.balance_hist[i]);
        }
        f(g, "clause_pos_frac_mean", self.clause_pos_frac_mean);
        f(g, "horn_frac", self.horn_frac);
        f(g, "reverse_horn_frac", self.reverse_horn_frac);
        let g = "big";
        f(g, "binary_frac", self.binary_frac);
        f(g, "touched_frac", self.big_touched_frac);
        f(g, "deg_mean", self.big_deg_mean);
        f(g, "deg_var", self.big_deg_var);
        f(g, "deg_max", self.big_deg_max);
        f(g, "root_frac", self.big_root_frac);
        f(g, "leaf_frac", self.big_leaf_frac);
        let g = "locality";
        f(g, "span_mean", self.span_mean);
        f(g, "span_median", self.span_median);
        f(g, "span_small_frac", self.span_small_frac);
        f(g, "consecutive_frac", self.consecutive_frac);
        let g = "classify";
        f(g, "small", self.class_small);
        f(g, "bigbig", self.class_bigbig);
        let g = "scc";
        f(g, "count", self.scc_count);
        f(g, "max", self.scc_max);
        f(g, "sum", self.scc_sum);
        for (i, n) in SCC_NAMES.iter().enumerate() {
            f(g, n, self.scc_hist[i]);
        }
        f(g, "substituted", self.substituted);
        let g = "bfs";
        f(g, "samples", self.bfs_samples);
        f(g, "random", self.bfs_random);
        f(g, "depth_mean", self.bfs_depth_mean);
        f(g, "depth_max", self.bfs_depth_max);
        f(g, "reach_mean", self.bfs_reach_mean);
        f(g, "reach_max", self.bfs_reach_max);
        f(g, "reach_frac", self.bfs_reach_frac);
        f(g, "failed_frac", self.bfs_failed_frac);
        f(g, "edges", self.bfs_edges);
        f(g, "capped", self.bfs_capped);
        let g = "gates";
        f(g, "gates", self.gates);
        f(g, "ands", self.gates_ands);
        f(g, "xors", self.gates_xors);
        f(g, "ites", self.gates_ites);
        f(g, "output_frac", self.gate_output_frac);
        f(g, "matched", self.matched);
        f(g, "matched_ands", self.matched_ands);
        f(g, "matched_xors", self.matched_xors);
        f(g, "matched_ites", self.matched_ites);
        f(g, "equivalences", self.congruent_equivalences);
        f(g, "units", self.congruent_units);
        f(g, "arity_ands", self.congruent_arity_ands);
        f(g, "arity_xors", self.congruent_arity_xors);
        f(g, "closures", self.closures);
        let g = "backbone";
        f(g, "computations", self.backbone_computations);
        f(g, "units", self.backbone_units);
        f(g, "units_per_var", self.backbone_units_per_var);
        f(g, "probes", self.backbone_probes);
        f(g, "implied", self.backbone_implied);
        f(g, "rounds", self.backbone_rounds);
        f(g, "ticks", self.backbone_ticks);
        f(g, "budget_hits", self.backbone_budget_hits);
        let g = "sweep";
        f(g, "sweeps", self.sweeps);
        f(g, "completed", self.sweep_completed);
        f(g, "variables", self.sweep_variables);
        f(g, "equivalences", self.sweep_equivalences);
        f(g, "units", self.sweep_units);
        f(g, "solved", self.sweep_solved);
        f(g, "sat", self.sweep_sat);
        f(g, "unsat", self.sweep_unsat);
        f(g, "budget_hits", self.sweep_budget_hits);
        let g = "kitten";
        f(g, "solved", self.kitten_solved);
        f(g, "sat", self.kitten_sat);
        f(g, "unsat", self.kitten_unsat);
        f(g, "unknown", self.kitten_unknown);
        f(g, "ticks", self.kitten_ticks);
        let g = "fastel";
        f(g, "eliminated", self.fast_eliminated);
        f(g, "eliminated_per_var", self.fast_eliminated_per_var);
        f(g, "strengthened", self.fast_strengthened);
        f(g, "subsumed", self.fast_subsumed);
        f(g, "factored", self.factored);
        let g = "lucky";
        f(g, "runs", self.lucky_runs);
        for i in 0..N_LUCKY {
            f(g, LUCKY_OUTCOME[i], self.lucky_outcome[i]);
            f(g, LUCKY_LEVEL[i], self.lucky_level_frac[i]);
            f(g, LUCKY_CONFLICTS[i], self.lucky_conflicts[i]);
        }
        let g = "warmup";
        f(g, "warmups", self.warmups);
        f(g, "assigned_frac", self.warmup_assigned_frac);
        f(g, "level", self.warmup_level);
        let g = "preprocess";
        f(g, "vars_over_original", self.pre_vars_over_original);
        f(g, "clauses_over_original", self.pre_clauses_over_original);
        f(g, "units", self.pre_units);
        f(g, "probing_ticks", self.pre_probing_ticks);
        f(g, "backbone_ticks", self.pre_backbone_ticks);
        f(g, "substitute_ticks", self.pre_substitute_ticks);
        f(g, "transitive_ticks", self.pre_transitive_ticks);
        f(g, "factor_ticks", self.pre_factor_ticks);
        f(g, "kitten_ticks", self.pre_kitten_ticks);
        f(g, "vivify_ticks", self.pre_vivify_ticks);
        f(g, "dense_ticks", self.pre_dense_ticks);
        f(g, "eliminate_resolutions", self.pre_eliminate_resolutions);
        f(g, "walk_steps", self.pre_walk_steps);
        f(g, "ticks", self.pre_ticks);
        f(g, "work", self.pre_work);
    }

    /// Number of features (the length of `visit`'s output).
    pub fn len() -> usize {
        let mut n = 0;
        StaticFeatures::default().visit(|_, _, _| n += 1);
        n
    }

    /// (group, name) of every feature, in visit order.
    pub fn names() -> Vec<(&'static str, &'static str)> {
        let mut v = Vec::with_capacity(160);
        StaticFeatures::default().visit(|g, n, _| v.push((g, n)));
        v
    }

    /// Every value, in visit order.
    pub fn values(&self) -> Vec<f64> {
        let mut v = Vec::with_capacity(160);
        self.visit(|_, _, x| v.push(x));
        v
    }

    /// The footer object: `{"computed":..,"cost_ns":..,"groups":{"shape":{"vars":..},..}}`.
    pub fn json(&self) -> String {
        let mut s = String::with_capacity(8 << 10);
        s.push_str(&format!(
            "{{\"computed\":{},\"cost_ns\":{},\"groups\":{{",
            self.computed, self.cost_ns
        ));
        let mut current = "";
        let mut first_in_group = true;
        let mut first_group = true;
        self.visit(|g, n, x| {
            if g != current {
                if !first_group {
                    s.push('}');
                    s.push(',');
                }
                first_group = false;
                s.push_str(&format!("\"{}\":{{", g));
                current = g;
                first_in_group = true;
            }
            if !first_in_group {
                s.push(',');
            }
            first_in_group = false;
            s.push_str(&format!("\"{}\":{}", n, json_number(x)));
        });
        if !first_group {
            s.push('}');
        }
        s.push_str("}}");
        s
    }
}

/// A finite JSON number for `x` (NaN and infinities become 0, which the
/// pass never produces for a well-formed formula).
fn json_number(x: f64) -> String {
    if !x.is_finite() {
        return "0".to_string();
    }
    if x == x.trunc() && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        format!("{}", x)
    }
}

/// The header schema: every group with its kind and feature names, so a
/// tool can mask a group by name without knowing the values.
pub fn schema_json() -> String {
    let names = StaticFeatures::names();
    let mut s = String::with_capacity(4 << 10);
    s.push('{');
    for (gi, (group, kind)) in GROUPS.iter().enumerate() {
        if gi > 0 {
            s.push(',');
        }
        s.push_str(&format!("\"{}\":{{\"kind\":\"{}\",\"features\":[", group, kind));
        let mut first = true;
        for (g, n) in &names {
            if g == group {
                if !first {
                    s.push(',');
                }
                first = false;
                s.push_str(&format!("\"{}\"", n));
            }
        }
        s.push_str("]}");
    }
    s.push('}');
    s
}

// ---------------------------------------------------------------------------
// The pass
// ---------------------------------------------------------------------------

#[inline]
fn len_bin(size: u64) -> usize {
    match size {
        0 | 1 => 0,
        2 => 1,
        3 => 2,
        4..=8 => 3,
        9..=32 => 4,
        _ => 5,
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

/// Accumulators of the clause pass.
struct ClausePass {
    clauses: u64,
    literals: u64,
    len_sq: f64,
    len_max: u64,
    len_hist: [u64; 6],
    len_count: Vec<u64>,
    len_lits: Vec<u64>,
    big_count: [u64; LEN_LOG_BINS],
    big_lits: [u64; LEN_LOG_BINS],
    horn: u64,
    reverse_horn: u64,
    pos_frac_sum: f64,
    span_sum: f64,
    span_hist: [u64; SPAN_BINS],
    span_small: u64,
    consecutive: u64,
    binaries: u64,
}

impl ClausePass {
    fn new() -> ClausePass {
        ClausePass {
            clauses: 0,
            literals: 0,
            len_sq: 0.0,
            len_max: 0,
            len_hist: [0; 6],
            len_count: vec![0; LEN_EXACT + 1],
            len_lits: vec![0; LEN_EXACT + 1],
            big_count: [0; LEN_LOG_BINS],
            big_lits: [0; LEN_LOG_BINS],
            horn: 0,
            reverse_horn: 0,
            pos_frac_sum: 0.0,
            span_sum: 0.0,
            span_hist: [0; SPAN_BINS],
            span_small: 0,
            consecutive: 0,
            binaries: 0,
        }
    }

    /// One clause of `size` unassigned literals, `pos` of them positive,
    /// over variables `min_var..=max_var`; `vars` is the index space.
    #[inline]
    fn account(&mut self, size: u64, pos: u64, min_var: u32, max_var: u32, vars: f64) {
        self.clauses += 1;
        self.literals += size;
        self.len_sq += (size as f64) * (size as f64);
        if size > self.len_max {
            self.len_max = size;
        }
        self.len_hist[len_bin(size)] += 1;
        if size == 2 {
            self.binaries += 1;
        }
        if (size as usize) <= LEN_EXACT {
            self.len_count[size as usize] += 1;
            self.len_lits[size as usize] += size;
        } else {
            let bin = (63 - size.leading_zeros()) as usize;
            let bin = bin.min(LEN_LOG_BINS - 1);
            self.big_count[bin] += 1;
            self.big_lits[bin] += size;
        }
        if pos <= 1 {
            self.horn += 1;
        }
        if size - pos <= 1 {
            self.reverse_horn += 1;
        }
        self.pos_frac_sum += pos as f64 / size as f64;
        let span = (max_var - min_var) as f64 / vars;
        self.span_sum += span;
        let bin = ((span * SPAN_BINS as f64) as usize).min(SPAN_BINS - 1);
        self.span_hist[bin] += 1;
        if span < 0.01 {
            self.span_small += 1;
        }
        if (max_var - min_var) as u64 + 1 == size {
            self.consecutive += 1;
        }
    }

    /// Share of all literals held by the longest 1 % of clauses (at least
    /// one clause), from the exact and the log2 histograms.
    fn giant_share(&self) -> f64 {
        if self.clauses == 0 || self.literals == 0 {
            return 0.0;
        }
        let target = (self.clauses / 100).max(1);
        let mut left = target;
        let mut lits: f64 = 0.0;
        let mut take = |count: u64, sum: u64, left: &mut u64| -> bool {
            if count == 0 {
                return false;
            }
            if count <= *left {
                lits += sum as f64;
                *left -= count;
                *left == 0
            } else {
                // A partial bin contributes its proportional share.
                lits += sum as f64 * (*left as f64 / count as f64);
                *left = 0;
                true
            }
        };
        let mut done = false;
        for b in (0..LEN_LOG_BINS).rev() {
            if take(self.big_count[b], self.big_lits[b], &mut left) {
                done = true;
                break;
            }
        }
        if !done {
            for n in (1..=LEN_EXACT).rev() {
                if take(self.len_count[n], self.len_lits[n], &mut left) {
                    break;
                }
            }
        }
        lits / self.literals as f64
    }

    fn span_median(&self) -> f64 {
        if self.clauses == 0 {
            return 0.0;
        }
        let half = self.clauses.div_ceil(2);
        let mut seen = 0;
        for (b, n) in self.span_hist.iter().enumerate() {
            seen += n;
            if seen >= half {
                return (b as f64 + 0.5) / SPAN_BINS as f64;
            }
        }
        1.0
    }
}

/// Compute the static features into `solver.policy.static_` and the
/// preformatted footer text `solver.policy.static_json`. Called once
/// after `classify()` when the policy is on; a second call recomputes.
pub fn compute(solver: &mut Solver) {
    let t0 = crate::policy_log::wall_ns();
    let mut f = StaticFeatures::default();
    let notes = solver.policy.notes;
    let mut rng: u64 = solver.policy.seed ^ 0x5341_5431_3353_5441;
    {
        let s: &Solver = &*solver;
        let vars = s.vars as usize;
        let lits = 2 * vars;
        let vars_f = (vars.max(1)) as f64;
        let mut occ_pos: Vec<u32> = vec![0; vars];
        let mut occ_neg: Vec<u32> = vec![0; vars];
        let mut bin_occ: Vec<u32> = vec![0; lits];
        let mut pass = ClausePass::new();
        let mut active: u64 = 0;

        // Binary clauses live only in the watch lists in watching mode
        // (after preprocessing, before search). Each is counted once, from
        // its smaller literal; every binary occurrence counts for the
        // implication graph.
        debug_assert!(s.watching);
        for i in 0..vars {
            if !s.flags[i].active() {
                continue;
            }
            active += 1;
            for lit in [2 * i as u32, 2 * i as u32 + 1] {
                let w = s.watches[lit as usize];
                let mut p = w.begin;
                while p != w.end {
                    let watch = s.vectors.stack[p];
                    let binary = watch_is_binary(watch);
                    p += if binary { 1 } else { 2 };
                    if !binary {
                        continue;
                    }
                    let other = watch_lit(watch);
                    if !s.flags[idx(other) as usize].active() || s.values[other as usize] != 0 {
                        continue;
                    }
                    bin_occ[lit as usize] += 1;
                    if other < lit {
                        continue;
                    }
                    let (a, b) = (idx(lit), idx(other));
                    let pos = (lit & 1 == 0) as u64 + (other & 1 == 0) as u64;
                    if lit & 1 == 0 {
                        occ_pos[a as usize] += 1;
                    } else {
                        occ_neg[a as usize] += 1;
                    }
                    if other & 1 == 0 {
                        occ_pos[b as usize] += 1;
                    } else {
                        occ_neg[b as usize] += 1;
                    }
                    pass.account(2, pos, a.min(b), a.max(b), vars_f);
                }
            }
        }

        // Large irredundant clauses in the arena (lucky.rs's iteration
        // idiom). Root-false literals are skipped, root-satisfied clauses
        // are skipped whole.
        let last_irredundant = s.last_irredundant;
        let mut ref_: crate::reference::Reference = 0;
        while (ref_ as u64) < s.arena.size_wards() {
            let next = s.arena.next_clause_ref(ref_);
            if last_irredundant != crate::reference::INVALID_REF && ref_ > last_irredundant {
                break;
            }
            let c = s.arena.clause(ref_);
            if c.redundant() || c.garbage() {
                ref_ = next;
                continue;
            }
            let mut size: u64 = 0;
            let mut pos: u64 = 0;
            let mut min_var = u32::MAX;
            let mut max_var = 0u32;
            let mut satisfied = false;
            for &lit in c.lits() {
                let v = s.values[lit as usize];
                if v > 0 {
                    satisfied = true;
                    break;
                }
                if v < 0 {
                    continue;
                }
                size += 1;
                let i = idx(lit);
                if lit & 1 == 0 {
                    pos += 1;
                }
                if i < min_var {
                    min_var = i;
                }
                if i > max_var {
                    max_var = i;
                }
            }
            if !satisfied && size > 0 {
                for &lit in c.lits() {
                    if s.values[lit as usize] != 0 {
                        continue;
                    }
                    let i = idx(lit) as usize;
                    if lit & 1 == 0 {
                        occ_pos[i] += 1;
                    } else {
                        occ_neg[i] += 1;
                    }
                }
                pass.account(size, pos, min_var, max_var, vars_f);
            }
            ref_ = next;
        }

        // Shape.
        let active_f = active as f64;
        f.vars = active_f;
        f.active_frac = ratio(active_f, s.vars as f64);
        f.clauses = pass.clauses as f64;
        f.literals = pass.literals as f64;
        f.clauses_per_var = ratio(f.clauses, active_f);
        f.lits_per_clause = ratio(f.literals, f.clauses);
        f.len_var = if pass.clauses > 0 {
            (pass.len_sq / f.clauses - f.lits_per_clause * f.lits_per_clause).max(0.0)
        } else {
            0.0
        };
        f.len_max = pass.len_max as f64;
        for i in 0..6 {
            f.len_hist[i] = ratio(pass.len_hist[i] as f64, f.clauses);
        }
        f.giant_share = pass.giant_share();
        f.duplicated = s.statistics.duplicated as f64;

        // Occurrence and polarity over active variables.
        let mut occ_sum = 0.0f64;
        let mut occ_sq = 0.0f64;
        let mut occ_max = 0u64;
        let mut ent = 0.0f64;
        let mut near_singleton = 0u64;
        let mut pure = 0u64;
        let mut unused = 0u64;
        let mut balance_sum = 0.0f64;
        let mut balance_hist = [0u64; 4];
        let mut touched = 0u64;
        for i in 0..vars {
            if !s.flags[i].active() {
                continue;
            }
            let p = occ_pos[i] as u64;
            let n = occ_neg[i] as u64;
            let occ = p + n;
            occ_sum += occ as f64;
            occ_sq += (occ as f64) * (occ as f64);
            if occ > occ_max {
                occ_max = occ;
            }
            if occ > 0 {
                ent += (occ as f64) * (occ as f64).ln();
                if p == 0 || n == 0 {
                    pure += 1;
                }
                let bal = (p as f64 - n as f64).abs() / occ as f64;
                balance_sum += bal;
                balance_hist[((bal * 4.0) as usize).min(3)] += 1;
            } else {
                unused += 1;
            }
            if occ <= 2 {
                near_singleton += 1;
            }
            if bin_occ[2 * i] + bin_occ[2 * i + 1] > 0 {
                touched += 1;
            }
        }
        f.occ_mean = ratio(occ_sum, active_f);
        f.occ_var = if active > 0 {
            (occ_sq / active_f - f.occ_mean * f.occ_mean).max(0.0)
        } else {
            0.0
        };
        f.occ_max = occ_max as f64;
        // Entropy of the occurrence distribution, normalized to [0, 1] by
        // the uniform entropy ln(active); 1 = every variable occurs equally.
        f.occ_entropy = if occ_sum > 0.0 && active > 1 {
            let h = occ_sum.ln() - ent / occ_sum;
            (h / active_f.ln()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        f.near_singleton_frac = ratio(near_singleton as f64, active_f);
        f.pure_frac = ratio(pure as f64, active_f);
        f.unused_frac = ratio(unused as f64, active_f);
        f.balance_mean = ratio(balance_sum, (active - unused) as f64);
        for i in 0..4 {
            f.balance_hist[i] = ratio(balance_hist[i] as f64, (active - unused) as f64);
        }
        f.clause_pos_frac_mean = ratio(pass.pos_frac_sum, f.clauses);
        f.horn_frac = ratio(pass.horn as f64, f.clauses);
        f.reverse_horn_frac = ratio(pass.reverse_horn as f64, f.clauses);

        // Binary implication graph: out-degree of literal l is the number
        // of binary clauses holding its negation; in-degree the number
        // holding l itself.
        let mut deg_sum = 0.0f64;
        let mut deg_sq = 0.0f64;
        let mut deg_max = 0u32;
        let mut roots = 0u64;
        let mut leaves = 0u64;
        let mut big_lits: u64 = 0;
        for i in 0..vars {
            if !s.flags[i].active() {
                continue;
            }
            for lit in [2 * i, 2 * i + 1] {
                let out = bin_occ[lit ^ 1];
                let inn = bin_occ[lit];
                big_lits += 1;
                deg_sum += out as f64;
                deg_sq += (out as f64) * (out as f64);
                if out > deg_max {
                    deg_max = out;
                }
                if out > 0 && inn == 0 {
                    roots += 1;
                }
                if out == 0 && inn > 0 {
                    leaves += 1;
                }
            }
        }
        let big_lits_f = big_lits as f64;
        f.binary_frac = ratio(pass.binaries as f64, f.clauses);
        f.big_touched_frac = ratio(touched as f64, active_f);
        f.big_deg_mean = ratio(deg_sum, big_lits_f);
        f.big_deg_var = if big_lits > 0 {
            (deg_sq / big_lits_f - f.big_deg_mean * f.big_deg_mean).max(0.0)
        } else {
            0.0
        };
        f.big_deg_max = deg_max as f64;
        f.big_root_frac = ratio(roots as f64, big_lits_f);
        f.big_leaf_frac = ratio(leaves as f64, big_lits_f);

        // Locality.
        f.span_mean = ratio(pass.span_sum, f.clauses);
        f.span_median = pass.span_median();
        f.span_small_frac = ratio(pass.span_small as f64, f.clauses);
        f.consecutive_frac = ratio(pass.consecutive as f64, f.clauses);

        // Bounded BFS over the implication graph from a sample of
        // literals: the top out-degree ones and random ones with an
        // out-edge, the random count set by the standard error of the
        // mean reach.
        bfs(s, &bin_occ, &mut rng, &mut f, active);
    }

    // Classification bits and the response groups from the counters.
    let st = &solver.statistics;
    f.class_small = solver.classification.small as u8 as f64;
    f.class_bigbig = solver.classification.bigbig as u8 as f64;
    f.scc_count = notes.scc_count as f64;
    f.scc_max = notes.scc_max as f64;
    f.scc_sum = notes.scc_sum as f64;
    for i in 0..4 {
        f.scc_hist[i] = notes.scc_hist[i] as f64;
    }
    f.substituted = st.substituted as f64;
    f.gates = st.congruent_gates as f64;
    f.gates_ands = st.congruent_gates_ands as f64;
    f.gates_xors = st.congruent_gates_xors as f64;
    f.gates_ites = st.congruent_gates_ites as f64;
    f.gate_output_frac = ratio(st.congruent_gates as f64, solver.active as f64).min(1.0);
    f.matched = st.congruent_matched as f64;
    f.matched_ands = st.congruent_matched_ands as f64;
    f.matched_xors = st.congruent_matched_xors as f64;
    f.matched_ites = st.congruent_matched_ites as f64;
    f.congruent_equivalences = st.congruent_equivalences as f64;
    f.congruent_units = st.congruent_units as f64;
    f.congruent_arity_ands = st.congruent_arity_ands as f64;
    f.congruent_arity_xors = st.congruent_arity_xors as f64;
    f.closures = st.closures as f64;
    f.backbone_computations = st.backbone_computations as f64;
    f.backbone_units = st.backbone_units as f64;
    f.backbone_units_per_var = ratio(st.backbone_units as f64, st.variables_original as f64);
    f.backbone_probes = st.backbone_probes as f64;
    f.backbone_implied = st.backbone_implied as f64;
    f.backbone_rounds = st.backbone_rounds as f64;
    f.backbone_ticks = st.backbone_ticks as f64;
    f.backbone_budget_hits = notes.backbone_budget_hits as f64;
    f.sweeps = st.sweep as f64;
    f.sweep_completed = st.sweep_completed as f64;
    f.sweep_variables = st.sweep_variables as f64;
    f.sweep_equivalences = st.sweep_equivalences as f64;
    f.sweep_units = st.sweep_units as f64;
    f.sweep_solved = st.sweep_solved as f64;
    f.sweep_sat = st.sweep_sat as f64;
    f.sweep_unsat = st.sweep_unsat as f64;
    f.sweep_budget_hits = notes.sweep_budget_hits as f64;
    f.kitten_solved = st.kitten_solved as f64;
    f.kitten_sat = st.kitten_sat as f64;
    f.kitten_unsat = st.kitten_unsat as f64;
    f.kitten_unknown = st.kitten_unknown as f64;
    f.kitten_ticks = st.kitten_ticks as f64;
    f.fast_eliminated = st.fast_eliminated as f64;
    f.fast_eliminated_per_var = ratio(st.fast_eliminated as f64, st.variables_original as f64);
    f.fast_strengthened = st.fast_strengthened as f64;
    f.fast_subsumed = st.fast_subsumed as f64;
    f.factored = st.factored as f64;
    f.lucky_runs = notes.lucky_runs as f64;
    for i in 0..N_LUCKY {
        let n = notes.lucky[i];
        f.lucky_outcome[i] = n.outcome as f64;
        f.lucky_level_frac[i] = ratio(n.level as f64, n.active as f64).min(1.0);
        f.lucky_conflicts[i] = n.conflicts as f64;
    }
    f.warmups = notes.warmups as f64;
    f.warmup_assigned_frac = notes.warmup_assigned_frac;
    f.warmup_level = notes.warmup_level as f64;
    f.pre_vars_over_original = ratio(solver.active as f64, st.variables_original as f64);
    f.pre_clauses_over_original = ratio(f.clauses, st.clauses_original as f64);
    f.pre_units = st.units as f64;
    f.pre_probing_ticks = st.probing_ticks as f64;
    f.pre_backbone_ticks = st.backbone_ticks as f64;
    f.pre_substitute_ticks = st.substitute_ticks as f64;
    f.pre_transitive_ticks = st.transitive_ticks as f64;
    f.pre_factor_ticks = st.factor_ticks as f64;
    f.pre_kitten_ticks = st.kitten_ticks as f64;
    f.pre_vivify_ticks = st.vivify_ticks as f64;
    f.pre_dense_ticks = st.dense_ticks as f64;
    f.pre_eliminate_resolutions = st.eliminate_resolutions as f64;
    f.pre_walk_steps = st.walk_steps as f64;
    f.pre_ticks = st.ticks as f64;
    f.pre_work = st.work_clock() as f64;

    f.computed = true;
    f.cost_ns = crate::policy_log::wall_ns().saturating_sub(t0);
    let json = f.json();
    {
        // The footer reads `static_json` from the signal handler: the swap
        // of the string happens with the handled signals blocked.
        let _guard = crate::policy_log::SignalGuard::block();
        solver.policy.static_json = json;
        solver.policy.static_ = f;
    }
    if crate::print::verbosity(solver) >= 2 {
        let f = &solver.policy.static_;
        crate::print::very_verbose(
            solver,
            format_args!(
                "static features: {} values in {} groups, {} active variables, {} clauses, binary fraction {:.3}, bfs {} samples, {:.1} ms",
                StaticFeatures::len(),
                N_GROUPS,
                f.vars,
                f.clauses,
                f.binary_frac,
                f.bfs_samples,
                f.cost_ns as f64 * 1e-6
            ),
        );
    }
}

/// Breadth-first implication walks from a sample of literals (plan
/// Appendix A): depth is the longest forced chain, reach the number of
/// literals forced; a walk that forces both x and not-x is a failed
/// literal. Edge visits are capped over the whole sample.
fn bfs(s: &Solver, bin_occ: &[u32], rng: &mut u64, f: &mut StaticFeatures, active: u64) {
    let vars = s.vars as usize;
    let lits = 2 * vars;
    if lits == 0 {
        return;
    }
    // Literals with at least one out-edge: candidates for the sample.
    let has_out = |lit: usize| s.flags[lit >> 1].active() && bin_occ[lit ^ 1] > 0;
    let mut n_candidates: u64 = 0;
    // Top out-degree literals by a bounded min-heap.
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::with_capacity(BFS_TOP + 1);
    for lit in 0..lits {
        if !has_out(lit) {
            continue;
        }
        n_candidates += 1;
        let deg = bin_occ[lit ^ 1];
        if heap.len() < BFS_TOP {
            heap.push(Reverse((deg, lit as u32)));
        } else if let Some(&Reverse((min_deg, _))) = heap.peek() {
            if deg > min_deg {
                heap.pop();
                heap.push(Reverse((deg, lit as u32)));
            }
        }
    }
    if n_candidates == 0 {
        return;
    }
    let mut seen: Vec<u32> = vec![0; lits];
    let mut queue: Vec<u32> = Vec::with_capacity(1024);
    let mut stamp: u32 = 0;
    let mut edges: u64 = 0;
    let mut capped = false;
    let mut samples: u64 = 0;
    let mut random_n: u64 = 0;
    let mut depth_sum = 0.0f64;
    let mut depth_max = 0u64;
    let mut reach_sum = 0.0f64;
    let mut reach_max = 0u64;
    let mut failed = 0u64;
    // Random-sample statistics for the stopping rule.
    let mut r_sum = 0.0f64;
    let mut r_sq = 0.0f64;

    let mut walk = |start: u32, seen: &mut Vec<u32>, queue: &mut Vec<u32>, edges: &mut u64| -> (u64, u64, bool) {
        stamp += 1;
        queue.clear();
        queue.push(start);
        seen[start as usize] = stamp;
        let mut depth: u64 = 0;
        let mut head = 0usize;
        let mut level_end = 1usize;
        let mut failed_here = false;
        'outer: while head < queue.len() {
            let u = queue[head];
            head += 1;
            let w = s.watches[not(u) as usize];
            let mut p = w.begin;
            while p != w.end {
                let watch = s.vectors.stack[p];
                let binary = watch_is_binary(watch);
                p += if binary { 1 } else { 2 };
                if !binary {
                    continue;
                }
                *edges += 1;
                if *edges > BFS_EDGE_CAP {
                    break 'outer;
                }
                let m = watch_lit(watch);
                if !s.flags[idx(m) as usize].active() || s.values[m as usize] != 0 {
                    continue;
                }
                if seen[m as usize] == stamp {
                    continue;
                }
                seen[m as usize] = stamp;
                if seen[not(m) as usize] == stamp {
                    failed_here = true;
                }
                queue.push(m);
            }
            if head == level_end && head < queue.len() {
                depth += 1;
                level_end = queue.len();
            }
        }
        (depth, queue.len() as u64 - 1, failed_here)
    };

    let mut record = |depth: u64, reach: u64, failed_here: bool| {
        samples += 1;
        depth_sum += depth as f64;
        reach_sum += reach as f64;
        if depth > depth_max {
            depth_max = depth;
        }
        if reach > reach_max {
            reach_max = reach;
        }
        if failed_here {
            failed += 1;
        }
    };

    for Reverse((_, lit)) in heap.into_sorted_vec() {
        if edges > BFS_EDGE_CAP {
            capped = true;
            break;
        }
        let (d, r, fl) = walk(lit, &mut seen, &mut queue, &mut edges);
        record(d, r, fl);
    }
    // Random literals with an out-edge, by rejection sampling; the
    // relative standard error of the mean reach stops the sample.
    let mut tries: u64 = 0;
    while (random_n as usize) < BFS_RANDOM_MAX && tries < 64 * BFS_RANDOM_MAX as u64 {
        if edges > BFS_EDGE_CAP {
            capped = true;
            break;
        }
        tries += 1;
        let lit = crate::random::pick_random(rng, 0, lits as u32) as usize;
        if !has_out(lit) {
            continue;
        }
        let (d, r, fl) = walk(lit as u32, &mut seen, &mut queue, &mut edges);
        record(d, r, fl);
        random_n += 1;
        r_sum += r as f64;
        r_sq += (r as f64) * (r as f64);
        if random_n as usize >= BFS_RANDOM_MIN {
            let n = random_n as f64;
            let mean = r_sum / n;
            let var = (r_sq / n - mean * mean).max(0.0);
            let se = (var / n).sqrt();
            if mean <= 0.0 || se <= BFS_RELATIVE_SE * mean {
                break;
            }
        }
    }
    if edges > BFS_EDGE_CAP {
        capped = true;
    }
    f.bfs_samples = samples as f64;
    f.bfs_random = random_n as f64;
    f.bfs_depth_mean = ratio(depth_sum, samples as f64);
    f.bfs_depth_max = depth_max as f64;
    f.bfs_reach_mean = ratio(reach_sum, samples as f64);
    f.bfs_reach_max = reach_max as f64;
    f.bfs_reach_frac = ratio(f.bfs_reach_mean, active as f64).min(1.0);
    f.bfs_failed_frac = ratio(failed as f64, samples as f64);
    f.bfs_edges = edges.min(BFS_EDGE_CAP) as f64;
    f.bfs_capped = capped as u8 as f64;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_grouped_and_match_the_schema() {
        let names = StaticFeatures::names();
        assert!(names.len() > 100, "{} features", names.len());
        assert_eq!(names.len(), StaticFeatures::len());
        let mut full: Vec<String> = names.iter().map(|(g, n)| format!("{}.{}", g, n)).collect();
        full.sort();
        full.dedup();
        assert_eq!(full.len(), names.len(), "duplicate feature name");
        // Every visited group is a declared group, and groups are
        // contiguous in visit order.
        let mut last = "";
        let mut seen: Vec<&str> = Vec::new();
        for (g, _) in &names {
            assert!(GROUPS.iter().any(|(name, _)| name == g), "group {}", g);
            if *g != last {
                assert!(!seen.contains(g), "group {} is not contiguous", g);
                seen.push(g);
                last = g;
            }
        }
        assert_eq!(seen.len(), N_GROUPS);
        let schema = schema_json();
        assert!(schema.starts_with("{\"shape\":{\"kind\":\"identity\",\"features\":[\"vars\","));
        assert!(schema.contains("\"locality\":{\"kind\":\"identity\""));
        assert!(schema.contains("\"lucky\":{\"kind\":\"response\""));
        let f = StaticFeatures::default();
        let json = f.json();
        assert!(json.starts_with("{\"computed\":false,\"cost_ns\":0,\"groups\":{\"shape\":{\"vars\":0,"));
        assert!(json.ends_with("}}"));
        assert_eq!(f.values().len(), names.len());
    }

    #[test]
    fn json_numbers_are_finite_and_short() {
        assert_eq!(json_number(3.0), "3");
        assert_eq!(json_number(0.25), "0.25");
        assert_eq!(json_number(f64::NAN), "0");
        assert_eq!(json_number(f64::INFINITY), "0");
        assert_eq!(json_number(-2.0), "-2");
    }

    #[test]
    fn giant_share_and_median_from_histograms() {
        let mut p = ClausePass::new();
        // 99 binary clauses and one clause of 2000 literals: the longest
        // 1 % is that one clause, holding 2000 of 2198 literals.
        for _ in 0..99 {
            p.account(2, 1, 0, 1, 10.0);
        }
        p.account(2000, 1000, 0, 9, 10.0);
        let share = p.giant_share();
        assert!((share - 2000.0 / 2198.0).abs() < 1e-9, "{}", share);
        assert_eq!(p.clauses, 100);
        assert_eq!(p.binaries, 99);
        assert_eq!(p.horn, 99);
        assert_eq!(p.len_max, 2000);
        assert_eq!(p.len_hist[1], 99);
        assert_eq!(p.len_hist[5], 1);
        // The median span is in the bin of the 99 small clauses.
        assert!(p.span_median() < 0.2, "{}", p.span_median());
        assert_eq!(p.span_small, 0);
        assert_eq!(p.consecutive, 99);
    }

    #[test]
    fn notes_accumulate() {
        let mut s = crate::internal::init();
        s.policy.on = true;
        note_scc(&mut s, 2);
        note_scc(&mut s, 17);
        note_backbone_budget_hit(&mut s);
        note_lucky(&mut s, LuckyPattern::ForwardTrue, 1, 7, 3);
        note_lucky_run(&mut s);
        let n = s.policy.notes;
        assert_eq!((n.scc_count, n.scc_max, n.scc_sum), (2, 17, 19));
        assert_eq!(n.scc_hist, [1, 0, 0, 1]);
        assert_eq!(n.backbone_budget_hits, 1);
        assert_eq!(n.lucky[LuckyPattern::ForwardTrue as usize].level, 7);
        assert_eq!(n.lucky_runs, 1);
    }
}
