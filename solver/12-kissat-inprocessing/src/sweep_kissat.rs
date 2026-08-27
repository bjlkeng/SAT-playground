//! Faithful kissat `sweep.c` port (SAT_SWEEP_FAITHFUL, SESSION 29).
//!
//! The existing `sweep_round` (SESSION 20 lineage) builds seed environments,
//! proves facts in a kitten, and applies equivalences only AFTER the round via
//! ELS — the substitute→re-extract cascade dies at ~500-1,700 equivalences on
//! uniqinv40 where kissat reaches 3,799 (S20/S28b measurements; three "nibble"
//! ports failed to close it). This module is the no-more-nibbles full port of
//! kissat's per-variable sweeping loop:
//!
//!   - variables are swept off a persistent occurrence-sorted schedule
//!     (doubly-linked list; incomplete schedules survive across calls;
//!     completed passes double the environment limits — kissat
//!     `sweepvars/sweepclauses <<= completed`, `depth += completed`),
//!   - each variable gets a fresh kitten environment built by BFS over the
//!     REAL clause DB occurrence lists,
//!   - backbone candidates and an equivalence partition are refined with
//!     kitten models and `flip_literal` pre-tests (kissat `sweepfliprounds`),
//!   - a proven equivalence is substituted into the real clause DB
//!     IMMEDIATELY (`substitute_connected_clauses` parity): the two
//!     equivalence binaries are installed as irredundant clauses (RUP from the
//!     emitted kitten core lemmas), every irredundant occurrence of the dead
//!     literal is rewritten in place, and the surviving representative is
//!     rescheduled — later environments see the collapsed formula, which is
//!     what sustains kissat's cascade,
//!   - the whole call is budgeted by kitten ticks (kissat `sweepeffort` 100‰
//!     of search ticks since the last call, min 10M) and self-throttles with
//!     kissat's delay counter (yield < 0.001 per swept variable).
//!
//! Everything is gated behind `SAT_SWEEP_FAITHFUL` (default off): with the
//! flag off no code here runs and the shipped defaults are byte-identical.

use super::*;

use crate::fxhash::FxHashSet;
use crate::kitten::{Kitten, KittenResult};

pub(crate) const FSWEEP_INVALID: u32 = u32::MAX;
const FSWEEP_VARS_BASE: u64 = 256; // kissat sweepvars
const FSWEEP_VARS_MAX: u64 = 8192; // kissat sweepmaxvars
const FSWEEP_CLAUSES_BASE: u64 = 1024; // kissat sweepclauses
const FSWEEP_CLAUSES_MAX: u64 = 32768; // kissat sweepmaxclauses
const FSWEEP_DEPTH_BASE: u32 = 2; // kissat sweepdepth
const FSWEEP_DEPTH_MAX: u32 = 3; // kissat sweepmaxdepth
const FSWEEP_MIN_EFFORT_TICKS: u64 = 10_000_000; // kissat mineffort (10M)
const FSWEEP_MAX_COMPLETED: u32 = 32; // kissat max_completed

/// Cross-call and scratch state for the faithful sweep (kissat `sweeper` +
/// the `solver->sweep_*` persistence fields).
pub(crate) struct FaithfulSweepState {
    /// Variables retained from an interrupted pass (kissat sweep_schedule).
    pub(crate) schedule_saved: Vec<u32>,
    /// Per-variable "still to sweep in the current incomplete pass" flag
    /// (kissat flags[idx].sweep).
    pub(crate) flags: Vec<bool>,
    /// A pass over all variables is in flight (kissat sweep_incomplete).
    pub(crate) incomplete: bool,
    /// Completed full passes; drives the environment-limit doubling.
    pub(crate) completed: u32,
    /// kissat delay machinery: skip `delay_count` future calls; interval
    /// grows +1 on barren calls and halves on yield.
    pub(crate) delay_current: u32,
    pub(crate) delay_count: u32,
    /// Lifetime kitten ticks consumed here (kissat statistics.kitten_ticks).
    pub(crate) kitten_ticks: u64,
    /// search_ticks at the last call (effort reference).
    pub(crate) last_search_ticks: u64,
    /// Deterministic LCG for kitten phase randomization.
    pub(crate) rng: u64,
    /// Cumulative equivalences (drives the ported yield-arming latch).
    pub(crate) equivs_total: u64,

    // ---- per-call scratch (allocation reuse) ----
    /// Literal-indexed occurrence lists over live irredundant clauses
    /// (kissat dense-mode watches).
    occ: Vec<Vec<u32>>,
    /// Var-indexed BFS depth + 1; 0 = not in the current environment.
    depths: Vec<u32>,
    /// Var-indexed signed representative literal (kissat reprs, both
    /// polarities folded into one signed entry). repr[v] == v ⇒ self.
    repr: Vec<i32>,
    /// Schedule doubly-linked list (kissat prev/next/first/last).
    prev: Vec<u32>,
    next: Vec<u32>,
    first: u32,
    last: u32,
    /// Var -> kitten var (1-based); 0 = unmapped. Cleared per environment.
    env_kvar: Vec<i32>,
    /// Kitten var - 1 -> outer var (reverse map).
    kmap_vars: Vec<u32>,
    /// Environment variables in BFS order (kissat sweeper->vars).
    env_vars: Vec<u32>,
    /// Clause refs already encoded into the current environment (c->swept).
    swept: FxHashSet<u32>,
    /// Kitten learned-clause indices already emitted to the outer proof.
    emitted: FxHashSet<usize>,
    /// SAT_DEBUG_FSWEEP_VARS: comma-separated outer vars — every fact,
    /// rewrite, and deletion touching one of them is logged to stderr.
    debug_vars: Option<Vec<u32>>,
    /// Backbone candidates (kissat sweeper->backbone).
    backbone: Vec<i32>,
    /// Equivalence-candidate partition, classes 0-terminated (kissat
    /// sweeper->partition; 0 plays INVALID_LIT).
    partition: Vec<i32>,
}

impl Default for FaithfulSweepState {
    fn default() -> Self {
        FaithfulSweepState {
            schedule_saved: Vec::new(),
            flags: Vec::new(),
            incomplete: false,
            completed: 0,
            delay_current: 0,
            delay_count: 0,
            kitten_ticks: 0,
            last_search_ticks: 0,
            rng: 0x9E3779B97F4A7C15,
            equivs_total: 0,
            occ: Vec::new(),
            depths: Vec::new(),
            repr: Vec::new(),
            prev: Vec::new(),
            next: Vec::new(),
            first: FSWEEP_INVALID,
            last: FSWEEP_INVALID,
            env_kvar: Vec::new(),
            kmap_vars: Vec::new(),
            env_vars: Vec::new(),
            swept: FxHashSet::default(),
            emitted: FxHashSet::default(),
            debug_vars: None,
            backbone: Vec::new(),
            partition: Vec::new(),
        }
    }
}

/// Shared loader for the SAT_DEBUG_MODEL_FILE reference model.
pub(crate) fn fs_debug_model() -> Option<&'static std::collections::HashMap<usize, bool>> {
    use std::sync::OnceLock;
    static MODEL: OnceLock<Option<std::collections::HashMap<usize, bool>>> = OnceLock::new();
    MODEL
        .get_or_init(|| {
            let path = std::env::var("SAT_DEBUG_MODEL_FILE").ok()?;
            let text = std::fs::read_to_string(path).ok()?;
            let mut m = std::collections::HashMap::new();
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("v ") {
                    for tok in rest.split_whitespace() {
                        if let Ok(x) = tok.parse::<i64>() {
                            if x != 0 {
                                m.insert(x.unsigned_abs() as usize, x > 0);
                            }
                        }
                    }
                }
            }
            Some(m)
        })
        .as_ref()
}

fn fs_debug_hit(debug_vars: &Option<Vec<u32>>, lits: &[i32]) -> bool {
    match debug_vars {
        None => false,
        Some(vs) => lits.iter().any(|&l| vs.contains(&l.unsigned_abs())),
    }
}

/// Per-call environment/effort limits (kissat sweeper->limit).
struct FsLimits {
    vars: usize,
    clauses: usize,
    depth: u32,
    ticks: u64,
}

impl Solver {
    /// One faithful sweep call (kissat `kissat_sweep`). Returns `false` iff
    /// the formula was proven UNSAT during the call.
    pub(crate) fn faithful_sweep(&mut self, proof_log: &mut ProofLog) -> bool {
        if self.current_level() != 0 || self.has_empty_clause || !self.solver_ok {
            return true;
        }
        if self.binary_fast_path {
            return true;
        }
        // Inline-tag contract: this pass rewrites clauses in place, which is
        // unsound under blindly-trusted tagged binary watchers — strip the
        // tags (lazy untagged validation takes over) before the first edit.
        if self.watch_inline_tags_active {
            self.deactivate_watch_inline_tags();
        }
        // kissat DELAYING(sweep)
        if self.fsweep.delay_count > 0 {
            self.fsweep.delay_count -= 1;
            return true;
        }
        self.stats.fsweep_calls += 1;
        self.fsweep.debug_vars = std::env::var("SAT_DEBUG_FSWEEP_VARS").ok().map(|s| {
            s.split(',')
                .filter_map(|t| t.trim().parse().ok())
                .collect::<Vec<u32>>()
        });

        let nv = self.assignment.len().saturating_sub(1);
        self.fs_resize_state(nv);

        // kissat init_sweeper limit computation.
        let completed = self.fsweep.completed.min(FSWEEP_MAX_COMPLETED);
        let vars_limit = (FSWEEP_VARS_BASE << completed).min(FSWEEP_VARS_MAX) as usize;
        let clauses_limit = (FSWEEP_CLAUSES_BASE << completed).min(FSWEEP_CLAUSES_MAX) as usize;
        let depth_limit = (FSWEEP_DEPTH_BASE + completed).min(FSWEEP_DEPTH_MAX);
        // kissat SET_EFFORT_LIMIT(sweep): effort‰ of search ticks since the
        // last call, floored at mineffort, added to lifetime kitten ticks.
        let now_ticks = self.stats.search_ticks;
        let reference = now_ticks
            .saturating_sub(self.fsweep.last_search_ticks)
            .max(FSWEEP_MIN_EFFORT_TICKS);
        self.fsweep.last_search_ticks = now_ticks;
        let delta = reference.saturating_mul(self.sweep_faithful_effort_permille) / 1000;
        let lim = FsLimits {
            vars: vars_limit.max(2),
            clauses: clauses_limit.max(2),
            depth: depth_limit.max(1),
            ticks: self.fsweep.kitten_ticks.saturating_add(delta.max(1)),
        };

        let call_no = self.stats.fsweep_calls;
        self.fs_check_watch_invariants(&format!("sweep-entry#{call_no}"));
        self.fs_build_occ();
        let _scheduled = self.fs_schedule_sweeping(lim.clauses);

        let units0 = self.stats.fsweep_units;
        let equivs0 = self.stats.fsweep_equivalences;
        let mut swept: u64 = 0;
        loop {
            if self.has_empty_clause || !self.solver_ok {
                break;
            }
            if self.fsweep.kitten_ticks >= lim.ticks {
                break;
            }
            let Some(idx) = self.fs_next_scheduled() else {
                break;
            };
            self.fsweep.flags[idx] = false;
            self.fs_sweep_variable(idx, &lim, proof_log);
            swept += 1;
            self.stats.fsweep_vars += 1;
            // kissat: dense propagate after each swept variable.
            if !self.has_empty_clause && self.propagate().is_some() {
                proof_log.record_clause(&[]);
                self.has_empty_clause = true;
                self.solver_ok = false;
            }
        }
        self.fs_unschedule();

        self.fs_check_watch_invariants(&format!("sweep-exit#{call_no}"));
        let units = self.stats.fsweep_units - units0;
        let equivs = self.stats.fsweep_equivalences - equivs0;
        let eliminated = units + equivs;
        // kissat BUMP_DELAY / REDUCE_DELAY on eliminated-per-swept < 0.001.
        let average = if swept > 0 {
            eliminated as f64 / swept as f64
        } else {
            0.0
        };
        if average < 0.001 {
            self.fsweep.delay_current = self.fsweep.delay_current.saturating_add(1);
        } else {
            self.fsweep.delay_current /= 2;
        }
        self.fsweep.delay_count = self.fsweep.delay_current;

        // Ported yield-arming latch (SESSION 20g machinery): the broader armed
        // inprocessing ecology (aggressive cadence, armed technique set) keys
        // off sweep_yield_armed; keep the identical arming rule so the banked
        // capture class (dislog et al.) retains its cadence under the port.
        self.fsweep.equivs_total = self.fsweep.equivs_total.saturating_add(equivs);
        if !self.sweep_yield_armed && self.sweep_yield_escalate_permille > 0 && equivs > 0 {
            let live = self.count_active_vars() as u64;
            let min_abs: u64 = std::env::var("SAT_SWEEP_YIELD_MIN_EQUIVS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1000);
            if self.stats.conflicts >= 100_000
                && self.fsweep.equivs_total
                    >= min_abs
                        .max(live.saturating_mul(self.sweep_yield_escalate_permille) / 1000)
            {
                self.sweep_yield_armed = true;
                if !self.inprocess_aggressive {
                    if self.stats.inprocess_armed_at_conflict == 0 {
                        self.stats.inprocess_armed_at_conflict = self.stats.conflicts.max(1);
                    }
                    self.inprocess_aggressive = true;
                    self.inprocess_aggressive_interval = INPROCESS_AGGRESSIVE_FIRST_INTERVAL;
                    self.next_inprocess_conflicts = self
                        .stats
                        .conflicts
                        .saturating_add(INPROCESS_AGGRESSIVE_FIRST_INTERVAL);
                }
            }
        }

        if std::env::var("SAT_DEBUG_SWEEP").is_ok() {
            eprintln!(
                "c fsweep call={} swept={} units={} equivs={} completed={} incomplete={} kticks={} limit={} delay={}",
                self.stats.fsweep_calls,
                swept,
                units,
                equivs,
                self.fsweep.completed,
                self.fsweep.incomplete,
                self.fsweep.kitten_ticks,
                lim.ticks,
                self.fsweep.delay_current,
            );
        }

        !self.has_empty_clause
    }

    /// Debug: dump the full analysis context when a search-learned clause is
    /// falsified by the reference model (SAT_DEBUG_MODEL_FILE). Names the
    /// poisoned reason clause directly.
    pub(crate) fn fs_debug_dump_false_learning(&self, learned: &[i32], conflict: &Conflict) {
        let Some(model) = fs_debug_model() else { return };
        if model.is_empty() || learned.is_empty() {
            return;
        }
        let m_sat = |slice: &[i32]| {
            slice.iter().any(|&l| {
                model
                    .get(&(l.unsigned_abs() as usize))
                    .map(|&v| v == (l > 0))
                    .unwrap_or(true)
            })
        };
        if m_sat(learned) {
            return;
        }
        eprintln!("c FALSE-LEARN learned={learned:?} conflict={conflict:?}");
        match conflict {
            Conflict::Clause(cref) => {
                eprintln!(
                    "c FALSE-LEARN conflict clause {} = {:?} model_sat={} deleted={}",
                    cref,
                    if self.clause_is_deleted(*cref) { vec![] } else { self.clause_slice(*cref).to_vec() },
                    if self.clause_is_deleted(*cref) { false } else { m_sat(self.clause_slice(*cref)) },
                    self.clause_is_deleted(*cref)
                );
            }
            other => eprintln!("c FALSE-LEARN conflict {other:?}"),
        }
        // dump reasons of every trail literal whose var appears in the learned clause,
        // plus the last 40 trail entries with reasons
        // The lethal ingredient of a false 1-UIP resolvent over true clauses is
        // a false "root" value (level-0-marked literal dropped during analysis).
        let nv = self.assignment.len().saturating_sub(1);
        for v in 1..=nv {
            if self.assignment[v] == UNASSIGNED || self.decision_level[v] != 0 {
                continue;
            }
            if let Some(&mv) = model.get(&v) {
                let av = self.assignment[v] == TRUE;
                if av != mv {
                    let detail = match self.reason[v].as_ref_unchecked() {
                        ReasonRef::Clause(cref) => {
                            if self.clause_is_deleted(cref) {
                                format!("Clause({cref}) DELETED")
                            } else {
                                let lit_status: Vec<String> = self
                                    .clause_slice(cref)
                                    .iter()
                                    .map(|&l| {
                                        let lv = l.unsigned_abs() as usize;
                                        format!(
                                            "{l}(a={} lvl={} elim={} M={:?})",
                                            self.assignment[lv],
                                            self.decision_level[lv],
                                            self.eliminated[lv],
                                            model.get(&lv)
                                        )
                                    })
                                    .collect();
                                format!(
                                    "Clause({cref}) learnt={} {:?} :: {}",
                                    self.clause_is_learnt(cref),
                                    self.clause_slice(cref),
                                    lit_status.join(" ")
                                )
                            }
                        }
                        ReasonRef::Binary(id) => format!("Binary({id:?})"),
                        ReasonRef::None => "None".to_string(),
                    };
                    eprintln!(
                        "c FALSE-LEARN POISONED-ROOT var={v} assigned={av} model={mv} elim={} reason={detail}",
                        self.eliminated[v]
                    );
                }
            }
        }
        let start = self.trail.len().saturating_sub(60);
        for i in start..self.trail.len() {
            let lit = self.trail[i];
            let v = lit.unsigned_abs() as usize;
            let detail = match self.reason[v].as_ref_unchecked() {
                ReasonRef::Clause(cref) => {
                    if self.clause_is_deleted(cref) {
                        format!("Clause({cref}) DELETED")
                    } else {
                        let s = self.clause_slice(cref);
                        format!(
                            "Clause({cref}) {:?} model_sat={} contains_lit={}",
                            s,
                            m_sat(s),
                            s.contains(&lit)
                        )
                    }
                }
                ReasonRef::Binary(id) => format!("Binary({id:?})"),
                ReasonRef::None => "None".to_string(),
            };
            eprintln!(
                "c FALSE-LEARN trail[{i}] lit={lit} level={} reason={detail}",
                self.decision_level[v]
            );
        }
    }

    /// Debug validator (SAT_DEBUG_MODEL_FILE=<path to a `v `-line model>):
    /// scan every live clause (original + learned) against a known-good model
    /// of the ORIGINAL formula; any live clause the model falsifies proves
    /// solver-state corruption at this boundary. Panics with full context.
    pub(crate) fn fs_debug_check_model(&mut self, when: &str) {
        let Some(model) = fs_debug_model() else { return };
        let check = |slice: &[i32]| -> bool {
            // satisfied, or contains a var outside the model (fresh factor var)
            slice.iter().any(|&l| {
                model
                    .get(&(l.unsigned_abs() as usize))
                    .map(|&v| v == (l > 0))
                    .unwrap_or(true)
            })
        };
        let oids: Vec<u32> = self.original_clause_ids.clone();
        for &cid in &oids {
            let cref = cid as usize;
            if self.clause_is_deleted(cref) {
                continue;
            }
            let slice = self.clause_slice(cref);
            assert!(
                check(slice),
                "MODEL-AUDIT [{when}]: live ORIGINAL clause {cref} {slice:?} falsified by reference model"
            );
        }
        let lids: Vec<usize> = self.learned_clause_ids.clone();
        for cref in lids {
            if self.clause_is_deleted(cref) {
                continue;
            }
            let slice = self.clause_slice(cref);
            assert!(
                check(slice),
                "MODEL-AUDIT [{when}]: live LEARNED clause {cref} {slice:?} falsified by reference model"
            );
        }
        // root values must agree with the model too
        let nv = self.assignment.len().saturating_sub(1);
        for v in 1..=nv {
            let a = self.assignment[v];
            if a == UNASSIGNED {
                continue;
            }
            if let Some(&mv) = model.get(&v) {
                // only audit ROOT assignments
                if self.current_level() == 0 {
                    assert!(
                        (a == TRUE) == mv || self.eliminated[v],
                        "MODEL-AUDIT [{when}]: root value of var {v} = {} contradicts reference model {mv}",
                        a == TRUE
                    );
                }
            }
        }
    }

    /// Debug validator (SAT_FSWEEP_INVARIANTS=on): every live original clause
    /// must be watched exactly on its first two literals, and every watcher
    /// must point at a live clause actually containing its watch literal in a
    /// watched slot. Panics with context on the first violation.
    pub(crate) fn fs_check_watch_invariants(&mut self, when: &str) {
        if std::env::var("SAT_FSWEEP_INVARIANTS").is_err() {
            return;
        }
        let ids: Vec<u32> = self.original_clause_ids.clone();
        for &cid in &ids {
            let cref = cid as usize;
            if self.clause_is_deleted(cref) {
                continue;
            }
            let len = self.clause_len(cref);
            if len < 2 {
                continue;
            }
            for pos in 0..2 {
                let lit = self.clause_lit(cref, pos);
                let wi = self.lit_index(lit);
                let found = self
                    .watch_list(wi)
                    .iter()
                    .any(|w| watcher_untagged_idx(w.clause_idx) == cref);
                assert!(
                    found,
                    "fsweep invariant [{when}]: clause {cref} {:?} not watched on {lit}",
                    self.clause_slice(cref)
                );
            }
        }
        let nlists = self.watch_lists_len();
        for wi in 0..nlists {
            let entries: Vec<u32> = self
                .watch_list(wi)
                .iter()
                .map(|w| w.clause_idx)
                .collect();
            for raw in entries {
                let cref = watcher_untagged_idx(raw);
                if self.clause_is_deleted(cref) {
                    continue; // lazy-deleted watcher, cleaned on traversal
                }
                let len = self.clause_len(cref);
                if len < 2 {
                    continue;
                }
                let l0 = lit_to_index(self.clause_lit(cref, 0));
                let l1 = lit_to_index(self.clause_lit(cref, 1));
                assert!(
                    wi == l0 || wi == l1,
                    "fsweep invariant [{when}]: watcher list {wi} holds clause {cref} {:?} whose watched slots are {l0}/{l1}",
                    self.clause_slice(cref)
                );
            }
        }
    }

    fn fs_resize_state(&mut self, nv: usize) {
        let want = nv + 1;
        self.fsweep.flags.resize(want, false);
        self.fsweep.depths.clear();
        self.fsweep.depths.resize(want, 0);
        self.fsweep.repr.clear();
        self.fsweep.repr.reserve(want);
        for v in 0..want as i32 {
            self.fsweep.repr.push(v);
        }
        self.fsweep.prev.clear();
        self.fsweep.prev.resize(want, FSWEEP_INVALID);
        self.fsweep.next.clear();
        self.fsweep.next.resize(want, FSWEEP_INVALID);
        self.fsweep.first = FSWEEP_INVALID;
        self.fsweep.last = FSWEEP_INVALID;
        self.fsweep.env_kvar.clear();
        self.fsweep.env_kvar.resize(want, 0);
        if self.fsweep.occ.len() < 2 * want {
            self.fsweep.occ.resize_with(2 * want, Vec::new);
        }
        self.fsweep.env_vars.clear();
        self.fsweep.kmap_vars.clear();
        self.fsweep.swept.clear();
        self.fsweep.emitted.clear();
        self.fsweep.backbone.clear();
        self.fsweep.partition.clear();
    }

    fn fs_var_active(&self, v: usize) -> bool {
        v < self.assignment.len() && self.assignment[v] == UNASSIGNED && !self.eliminated[v]
    }

    /// kissat sweep_repr: chase the representative chain with full path
    /// compression, keeping both polarities consistent.
    fn fs_find_repr(&mut self, lit: i32) -> i32 {
        let read = |repr: &[i32], l: i32| -> i32 {
            let r = repr[l.unsigned_abs() as usize];
            if l > 0 {
                r
            } else {
                -r
            }
        };
        let mut root = lit;
        loop {
            let r = read(&self.fsweep.repr, root);
            if r == root {
                break;
            }
            root = r;
        }
        if root == lit {
            return root;
        }
        let mut p = lit;
        while p != root {
            let nextp = read(&self.fsweep.repr, p);
            let v = p.unsigned_abs() as usize;
            self.fsweep.repr[v] = if p > 0 { root } else { -root };
            p = nextp;
        }
        root
    }

    fn fs_set_repr(&mut self, gone: i32, keep: i32) {
        let v = gone.unsigned_abs() as usize;
        self.fsweep.repr[v] = if gone > 0 { keep } else { -keep };
    }

    /// Literal-indexed occurrence index over live irredundant clauses
    /// (kissat enter_dense_mode + connect_irredundant_large_clauses; our
    /// binaries are ordinary length-2 arena clauses so one uniform index).
    fn fs_build_occ(&mut self) {
        let want = 2 * (self.assignment.len().saturating_sub(1) + 1);
        for list in self.fsweep.occ.iter_mut().take(want) {
            list.clear();
        }
        let ids = std::mem::take(&mut self.original_clause_ids);
        for &cref_w in &ids {
            let cref = cref_w as usize;
            if self.clause_is_deleted(cref) {
                continue;
            }
            let len = self.clause_len(cref);
            for pos in 0..len {
                let lit = self.clause_lit(cref, pos);
                self.fsweep.occ[lit_to_index(lit)].push(cref_w);
            }
        }
        self.original_clause_ids = ids;
    }

    // ---- schedule linked list (kissat schedule_inner/outer/next_scheduled) ----

    fn fs_scheduled(&self, v: usize) -> bool {
        self.fsweep.prev[v] != FSWEEP_INVALID || self.fsweep.first == v as u32
    }

    /// Append `v` at the BACK (dequeued next; kissat schedule_inner).
    fn fs_schedule_inner(&mut self, v: usize) {
        if !self.fs_var_active(v) {
            return;
        }
        let vu = v as u32;
        let nxt = self.fsweep.next[v];
        if nxt != FSWEEP_INVALID {
            // unlink, then append at back
            let prv = self.fsweep.prev[v];
            self.fsweep.prev[nxt as usize] = prv;
            if prv == FSWEEP_INVALID {
                self.fsweep.first = nxt;
            } else {
                self.fsweep.next[prv as usize] = nxt;
            }
            let lst = self.fsweep.last;
            if lst == FSWEEP_INVALID {
                self.fsweep.first = vu;
            } else {
                self.fsweep.next[lst as usize] = vu;
            }
            self.fsweep.prev[v] = lst;
            self.fsweep.next[v] = FSWEEP_INVALID;
            self.fsweep.last = vu;
        } else if self.fsweep.last != vu {
            let lst = self.fsweep.last;
            if lst == FSWEEP_INVALID {
                self.fsweep.first = vu;
            } else {
                self.fsweep.next[lst as usize] = vu;
            }
            self.fsweep.prev[v] = lst;
            self.fsweep.last = vu;
        }
    }

    /// Prepend `v` at the FRONT (dequeued last; kissat schedule_outer).
    fn fs_schedule_outer(&mut self, v: usize) {
        let vu = v as u32;
        let fst = self.fsweep.first;
        if fst == FSWEEP_INVALID {
            self.fsweep.last = vu;
        } else {
            self.fsweep.prev[fst as usize] = vu;
        }
        self.fsweep.next[v] = fst;
        self.fsweep.first = vu;
    }

    /// Dequeue from the BACK (kissat next_scheduled).
    fn fs_next_scheduled(&mut self) -> Option<usize> {
        let res = self.fsweep.last;
        if res == FSWEEP_INVALID {
            return None;
        }
        let v = res as usize;
        let prv = self.fsweep.prev[v];
        self.fsweep.prev[v] = FSWEEP_INVALID;
        if prv == FSWEEP_INVALID {
            self.fsweep.first = FSWEEP_INVALID;
        } else {
            self.fsweep.next[prv as usize] = FSWEEP_INVALID;
        }
        self.fsweep.last = prv;
        Some(v)
    }

    fn fs_scheduable(&self, v: usize, max_occurrences: usize) -> Option<usize> {
        let pos = self.fsweep.occ[lit_to_index(v as i32)].len();
        if pos == 0 || pos > max_occurrences {
            return None;
        }
        let neg = self.fsweep.occ[lit_to_index(-(v as i32))].len();
        if neg == 0 || neg > max_occurrences {
            return None;
        }
        Some(pos + neg)
    }

    /// kissat schedule_sweeping: rescheduled-from-last-call first (back of the
    /// queue = swept first), then all remaining candidates sorted ascending by
    /// occurrence count and prepended (fewest occurrences swept first).
    fn fs_schedule_sweeping(&mut self, max_occurrences: usize) -> usize {
        // reschedule_previously_remaining
        let saved = std::mem::take(&mut self.fsweep.schedule_saved);
        let mut rescheduled = 0usize;
        for &vu in &saved {
            let v = vu as usize;
            if v >= self.fsweep.flags.len() || !self.fs_var_active(v) {
                continue;
            }
            if self.fs_scheduled(v) {
                continue;
            }
            if self.fs_scheduable(v, max_occurrences).is_none() {
                self.fsweep.flags[v] = false;
                continue;
            }
            self.fs_schedule_inner(v);
            rescheduled += 1;
        }
        // schedule_all_other_not_scheduled_yet
        let nv = self.assignment.len().saturating_sub(1);
        let incomplete_pass = self.fsweep.incomplete;
        let mut fresh: Vec<(u32, u32)> = Vec::new();
        for v in 1..=nv {
            if !self.fs_var_active(v) {
                continue;
            }
            if incomplete_pass && !self.fsweep.flags[v] {
                continue;
            }
            if self.fs_scheduled(v) {
                continue;
            }
            match self.fs_scheduable(v, max_occurrences) {
                None => {
                    self.fsweep.flags[v] = false;
                }
                Some(occ) => fresh.push((occ.min(u32::MAX as usize) as u32, v as u32)),
            }
        }
        fresh.sort_by_key(|&(occ, _)| occ); // stable = kissat radix order
        let fresh_count = fresh.len();
        for &(_, vu) in &fresh {
            self.fs_schedule_outer(vu as usize);
        }
        let scheduled = fresh_count + rescheduled;
        // incomplete bookkeeping (kissat schedule_sweeping tail)
        let incomplete = (1..=nv)
            .filter(|&v| self.fs_var_active(v) && self.fsweep.flags[v])
            .count();
        if incomplete == 0 {
            if self.fsweep.incomplete {
                self.fsweep.completed += 1;
                self.stats.fsweep_completed = self.fsweep.completed as u64;
            }
            // mark_incomplete: flag every scheduled variable
            let mut cur = self.fsweep.first;
            while cur != FSWEEP_INVALID {
                let v = cur as usize;
                self.fsweep.flags[v] = true;
                cur = self.fsweep.next[v];
            }
            self.fsweep.incomplete = true;
        }
        scheduled
    }

    /// kissat unschedule_sweeping: retain the untried schedule for next call.
    fn fs_unschedule(&mut self) {
        let mut saved = std::mem::take(&mut self.fsweep.schedule_saved);
        saved.clear();
        let mut cur = self.fsweep.first;
        while cur != FSWEEP_INVALID {
            let v = cur as usize;
            let nxt = self.fsweep.next[v];
            if self.fs_var_active(v) {
                saved.push(cur);
            }
            self.fsweep.prev[v] = FSWEEP_INVALID;
            self.fsweep.next[v] = FSWEEP_INVALID;
            cur = nxt;
        }
        self.fsweep.first = FSWEEP_INVALID;
        self.fsweep.last = FSWEEP_INVALID;
        self.fsweep.schedule_saved = saved;
        let nv = self.assignment.len().saturating_sub(1);
        let incomplete = (1..=nv)
            .filter(|&v| self.fs_var_active(v) && self.fsweep.flags[v])
            .count();
        if incomplete == 0 {
            self.fsweep.incomplete = false;
            self.fsweep.completed += 1;
            self.stats.fsweep_completed = self.fsweep.completed as u64;
        }
    }

    // ---- environment construction (kissat sweep_variable part 1) ----

    fn fs_kvar(&mut self, kitten: &mut Kitten, v: usize) -> i32 {
        let kv = self.fsweep.env_kvar[v];
        if kv != 0 {
            return kv;
        }
        self.fsweep.kmap_vars.push(v as u32);
        let kv = self.fsweep.kmap_vars.len() as i32;
        self.fsweep.env_kvar[v] = kv;
        kitten.ensure_num_vars(kv as usize);
        kv
    }

    fn fs_klit(&self, lit: i32) -> i32 {
        let kv = self.fsweep.env_kvar[lit.unsigned_abs() as usize];
        debug_assert!(kv != 0);
        if lit > 0 {
            kv
        } else {
            -kv
        }
    }

    fn fs_add_env_var(&mut self, depth: u32, lit: i32) {
        let r = self.fs_find_repr(lit);
        if r != lit {
            return;
        }
        let v = lit.unsigned_abs() as usize;
        if self.fsweep.depths[v] != 0 {
            return;
        }
        self.fsweep.depths[v] = depth + 1;
        self.fsweep.env_vars.push(v as u32);
    }

    /// Encode one clause into the environment (kissat sweep_binary /
    /// sweep_reference + sweep_clause). Returns true if it was encoded.
    fn fs_encode_clause(
        &mut self,
        kitten: &mut Kitten,
        depth: u32,
        cref: usize,
    ) -> bool {
        if self.clause_is_deleted(cref) || self.fsweep.swept.contains(&(cref as u32)) {
            return false;
        }
        let len = self.clause_len(cref);
        if len == 2 {
            // kissat sweep_binary parity: only representative endpoints — this
            // is what keeps proven-equivalence definition binaries out of
            // later environments.
            let a = self.clause_lit(cref, 0);
            let b = self.clause_lit(cref, 1);
            if self.fs_find_repr(a) != a || self.fs_find_repr(b) != b {
                return false;
            }
        }
        let mut buf: Vec<i32> = Vec::with_capacity(len);
        for pos in 0..len {
            let lit = self.clause_lit(cref, pos);
            match self.lit_value(lit) {
                TRUE => return false, // satisfied: skip (kissat garbages it)
                FALSE => continue,
                _ => buf.push(lit),
            }
        }
        self.fsweep.swept.insert(cref as u32);
        for &lit in &buf {
            self.fs_add_env_var(depth, lit);
        }
        let mut klits: Vec<i32> = Vec::with_capacity(buf.len());
        for &lit in &buf {
            let v = lit.unsigned_abs() as usize;
            let kv = self.fs_kvar(kitten, v);
            klits.push(if lit > 0 { kv } else { -kv });
        }
        kitten.add_clause(&klits);
        true
    }

    // ---- kitten drivers with tick accounting ----

    fn fs_solve(
        &mut self,
        kitten: &mut Kitten,
        assumptions: &[i32],
        lim: &FsLimits,
    ) -> Option<KittenResult> {
        let remaining = lim.ticks.saturating_sub(self.fsweep.kitten_ticks);
        if remaining == 0 {
            return None;
        }
        // kissat sweep_solve: randomize phases before every solve.
        self.fsweep.rng = self
            .fsweep
            .rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        kitten.queue_randomize_phases(self.fsweep.rng);
        let res = kitten.solve_budgeted(assumptions, remaining);
        self.fsweep.kitten_ticks = self.fsweep.kitten_ticks.saturating_add(kitten.ticks());
        self.stats.fsweep_solves += 1;
        self.stats.fsweep_ticks = self.fsweep.kitten_ticks;
        res
    }

    fn fs_flip(&mut self, kitten: &mut Kitten, var: usize) -> bool {
        let before = kitten.ticks();
        let res = kitten.flip_literal(var);
        self.fsweep.kitten_ticks = self
            .fsweep
            .kitten_ticks
            .saturating_add(kitten.ticks().saturating_sub(before));
        res
    }

    // ---- proof emission ----

    /// Emit not-yet-emitted kitten core lemmas (ascending = derivation order)
    /// as outer RUP additions. Every lemma's antecedents are in the core, so
    /// the emitted subsequence is a self-contained RUP chain over the outer
    /// formula (environment clauses are outer clauses minus root-false
    /// literals, which propagate identically under the outer root units).
    fn fs_emit_core_list(
        &mut self,
        kitten: &Kitten,
        core_learned: &[usize],
        proof_log: &mut ProofLog,
    ) {
        let mut out: Vec<i32> = Vec::new();
        for &ci in core_learned {
            if !self.fsweep.emitted.insert(ci) {
                continue;
            }
            out.clear();
            for &d in kitten.learned_lemma_dimacs(ci) {
                let kv = d.unsigned_abs() as usize;
                let outer_var = self.fsweep.kmap_vars[kv - 1] as i32;
                out.push(if d > 0 { outer_var } else { -outer_var });
            }
            proof_log.record_clause(&out);
        }
    }

    fn fs_emit_core(&mut self, kitten: &Kitten, proof_log: &mut ProofLog) {
        let core: Vec<usize> = kitten.core_learned().to_vec();
        self.fs_emit_core_list(kitten, &core, proof_log);
    }

    // ---- candidate refinement (kissat sweep_refine*) ----

    fn fs_refine(&mut self, kitten: &Kitten) {
        // Backbone: keep outer-unassigned candidates the new model still
        // makes true.
        let mut backbone = std::mem::take(&mut self.fsweep.backbone);
        backbone.retain(|&lit| {
            if self.lit_value(lit) != UNASSIGNED {
                return false;
            }
            let kl = self.fs_klit(lit);
            match kitten.value(kl.unsigned_abs() as usize) {
                Some(b) => b == (kl > 0),
                None => false,
            }
        });
        self.fsweep.backbone = backbone;
        // Partition: split every class by the new model's value.
        let old = std::mem::take(&mut self.fsweep.partition);
        let mut newp: Vec<i32> = Vec::with_capacity(old.len());
        let mut class_start = 0usize;
        for i in 0..old.len() {
            if old[i] != 0 {
                continue;
            }
            let class = &old[class_start..i];
            class_start = i + 1;
            for want_true in [true, false] {
                let mark = newp.len();
                let mut count = 0usize;
                for &other in class {
                    if self.fs_find_repr(other) != other {
                        continue;
                    }
                    if self.lit_value(other) != UNASSIGNED {
                        continue;
                    }
                    let kl = self.fs_klit(other);
                    let val = kitten.value(kl.unsigned_abs() as usize);
                    let model_true = match val {
                        Some(b) => b == (kl > 0),
                        None => continue,
                    };
                    if model_true == want_true {
                        newp.push(other);
                        count += 1;
                    }
                }
                if count < 2 {
                    newp.truncate(mark);
                } else {
                    newp.push(0);
                }
            }
        }
        self.fsweep.partition = newp;
    }

    // ---- flip pre-tests (kissat flip_backbone_literals / flip_partition_literals,
    //      sweepfliprounds = 1) ----

    fn fs_flip_backbone(&mut self, kitten: &mut Kitten, lim: &FsLimits) {
        if kitten.status() != 10 {
            return;
        }
        let backbone = std::mem::take(&mut self.fsweep.backbone);
        let mut kept = Vec::with_capacity(backbone.len());
        for (i, &lit) in backbone.iter().enumerate() {
            self.stats.fsweep_flip_tests += 1;
            let kl = self.fs_klit(lit);
            if self.fs_flip(kitten, kl.unsigned_abs() as usize) {
                self.stats.fsweep_flipped += 1;
            } else {
                kept.push(lit);
            }
            if self.fsweep.kitten_ticks >= lim.ticks {
                kept.extend_from_slice(&backbone[i + 1..]);
                break;
            }
        }
        self.fsweep.backbone = kept;
    }

    fn fs_flip_partition(&mut self, kitten: &mut Kitten) {
        if kitten.status() != 10 {
            return;
        }
        let old = std::mem::take(&mut self.fsweep.partition);
        let mut newp: Vec<i32> = Vec::with_capacity(old.len());
        let mut class_start = 0usize;
        for i in 0..old.len() {
            if old[i] != 0 {
                continue;
            }
            let class = &old[class_start..i];
            class_start = i + 1;
            let mut size = class.len();
            let mark = newp.len();
            for &lit in class {
                self.stats.fsweep_flip_tests += 1;
                let kl = self.fs_klit(lit);
                if self.fs_flip(kitten, kl.unsigned_abs() as usize) {
                    self.stats.fsweep_flipped += 1;
                    size -= 1;
                    if size < 2 {
                        break; // kissat drops the unvisited tail too
                    }
                } else {
                    newp.push(lit);
                }
            }
            if size > 1 {
                newp.push(0);
            } else {
                newp.truncate(mark);
            }
        }
        self.fsweep.partition = newp;
    }

    // ---- backbone candidates (kissat sweep_backbone_candidate) ----

    fn fs_backbone_candidate(
        &mut self,
        kitten: &mut Kitten,
        lit: i32,
        lim: &FsLimits,
        proof_log: &mut ProofLog,
    ) -> bool {
        let kl = self.fs_klit(lit);
        if kitten.fixed_lit(kl) != 0 {
            return false;
        }
        self.stats.fsweep_flip_tests += 1;
        if kitten.status() == 10 && self.fs_flip(kitten, kl.unsigned_abs() as usize) {
            self.stats.fsweep_flipped += 1;
            return false;
        }
        match self.fs_solve(kitten, &[-kl], lim) {
            Some(KittenResult::Sat) => {
                self.fs_refine(kitten);
                false
            }
            None => false,
            Some(KittenResult::Unsat) => {
                self.fs_emit_core(kitten, proof_log);
                self.stats.fsweep_units += 1;
                if fs_debug_hit(&self.fsweep.debug_vars, &[lit]) {
                    eprintln!("c FSDBG backbone-unit {lit} conflicts={}", self.stats.conflicts);
                }
                if !self.learn_lucky_failed_literal_units(&[lit], proof_log) {
                    self.solver_ok = false;
                }
                true
            }
        }
    }

    // ---- equivalence candidates (kissat sweep_equivalence_candidates) ----

    fn fs_equivalence_candidates(
        &mut self,
        kitten: &mut Kitten,
        lit: i32,
        other: i32,
        lim: &FsLimits,
        proof_log: &mut ProofLog,
    ) -> bool {
        let n = self.fsweep.partition.len();
        debug_assert!(n >= 3);
        debug_assert_eq!(self.fsweep.partition[n - 1], 0);
        debug_assert_eq!(self.fsweep.partition[n - 3], lit);
        debug_assert_eq!(self.fsweep.partition[n - 2], other);
        let third = if n == 3 { 0 } else { self.fsweep.partition[n - 4] };
        let kl = self.fs_klit(lit);
        let ko = self.fs_klit(other);
        if kitten.status() == 10 {
            self.stats.fsweep_flip_tests += 1;
            if self.fs_flip(kitten, kl.unsigned_abs() as usize) {
                self.stats.fsweep_flipped += 1;
                if third == 0 {
                    self.fsweep.partition.truncate(n - 3);
                } else {
                    self.fsweep.partition[n - 3] = other;
                    self.fsweep.partition[n - 2] = 0;
                    self.fsweep.partition.truncate(n - 1);
                }
                return false;
            }
            self.stats.fsweep_flip_tests += 1;
            if self.fs_flip(kitten, ko.unsigned_abs() as usize) {
                self.stats.fsweep_flipped += 1;
                if third == 0 {
                    self.fsweep.partition.truncate(n - 3);
                } else {
                    self.fsweep.partition[n - 2] = 0;
                    self.fsweep.partition.truncate(n - 1);
                }
                return false;
            }
        }
        // First implication: other -> lit, i.e. refute (¬lit ∧ other).
        let core1: Vec<usize> = match self.fs_solve(kitten, &[-kl, ko], lim) {
            Some(KittenResult::Sat) => {
                self.fs_refine(kitten);
                return false;
            }
            None => return false,
            Some(KittenResult::Unsat) => kitten.core_learned().to_vec(),
        };
        // Second implication: lit -> other, i.e. refute (lit ∧ ¬other).
        let core2: Vec<usize> = match self.fs_solve(kitten, &[kl, -ko], lim) {
            Some(KittenResult::Sat) => {
                self.fs_refine(kitten);
                return false;
            }
            None => return false,
            Some(KittenResult::Unsat) => kitten.core_learned().to_vec(),
        };
        // Bank the equivalence: cores make each binary RUP.
        self.fs_emit_core_list(kitten, &core1, proof_log);
        self.fs_add_equiv_binary(lit, -other, proof_log);
        self.fs_emit_core_list(kitten, &core2, proof_log);
        self.fs_add_equiv_binary(-lit, other, proof_log);
        self.stats.fsweep_equivalences += 1;
        self.stats.sweep_equivalences += 1;

        let (keep, gone) = if lit_to_index(lit) < lit_to_index(other) {
            (lit, other)
        } else {
            (other, lit)
        };
        self.fs_set_repr(gone, keep);
        self.fs_substitute(gone, keep, proof_log);
        self.fs_substitute(-gone, -keep, proof_log);
        self.fs_partition_remove(gone);
        self.fs_schedule_inner(keep.unsigned_abs() as usize);
        true
    }

    fn fs_add_equiv_binary(&mut self, a: i32, b: i32, proof_log: &mut ProofLog) {
        if fs_debug_hit(&self.fsweep.debug_vars, &[a, b]) {
            eprintln!("c FSDBG equiv-binary a=({a} {b}) conflicts={}", self.stats.conflicts);
        }
        proof_log.record_clause(&[a, b]);
        let cref = self.els_install_original_clause(&[a, b]);
        self.original_clause_ids.push(cref as u32);
        self.fsweep.occ[lit_to_index(a)].push(cref as u32);
        self.fsweep.occ[lit_to_index(b)].push(cref as u32);
    }

    /// kissat sweep_remove: drop `gone` from its partition class; squash the
    /// class if it shrinks below 2.
    fn fs_partition_remove(&mut self, gone: i32) {
        let partition = &mut self.fsweep.partition;
        let Some(p) = partition.iter().position(|&l| l == gone) else {
            return;
        };
        let mut begin = p;
        while begin > 0 && partition[begin - 1] != 0 {
            begin -= 1;
        }
        let mut end = p;
        while partition[end] != 0 {
            end += 1;
        }
        let size = end - begin;
        if size <= 2 {
            // squash the whole class including its terminator
            partition.drain(begin..=end);
        } else {
            partition.remove(p);
        }
    }

    // ---- immediate real-DB substitution (kissat substitute_connected_clauses) ----

    fn fs_substitute(&mut self, from: i32, to: i32, proof_log: &mut ProofLog) {
        // Debug isolation knob: bank equivalence binaries only (ELS applies
        // them later), no immediate rewrite.
        if std::env::var("SAT_SWEEP_FAITHFUL_NOSUBST").is_ok() {
            return;
        }
        if self.has_empty_clause || !self.solver_ok {
            return;
        }
        if self.lit_value(from) != UNASSIGNED || self.lit_value(to) != UNASSIGNED {
            return;
        }
        debug_assert!(from.unsigned_abs() != to.unsigned_abs());
        let li = lit_to_index(from);
        let list = std::mem::take(&mut self.fsweep.occ[li]);
        let mut kept: Vec<u32> = Vec::new();
        let mut delayed: Vec<u32> = Vec::new();
        let mut new_lits: Vec<i32> = Vec::new();
        let mut stopped_at: Option<usize> = None;
        'walk: for (i, &cref_w) in list.iter().enumerate() {
            let cref = cref_w as usize;
            if self.clause_is_deleted(cref) {
                continue;
            }
            let slice = self.clause_slice(cref);
            if !slice.contains(&from) {
                continue; // stale entry (literal dropped by an earlier rewrite)
            }
            let len = slice.len();
            // The equivalence-definition binary (from ∨ ¬to): keep untouched
            // (kissat binary path `other == NOT (repr) → continue`).
            if len == 2 && (slice[0] == -to || slice[1] == -to) {
                kept.push(cref_w);
                continue;
            }
            new_lits.clear();
            let mut satisfied = false;
            let mut to_present = false;
            for &l in slice {
                let l2 = if l == from { to } else { l };
                if l2 == -to {
                    // substituted clause is a tautology (to ∨ ¬to …);
                    // kissat marks it garbage
                    satisfied = true;
                    break;
                }
                if l2 == to {
                    if to_present {
                        continue; // dedup repr
                    }
                    to_present = true;
                    new_lits.push(to);
                    continue;
                }
                match self.lit_value(l2) {
                    TRUE => {
                        satisfied = true;
                        break;
                    }
                    FALSE => continue,
                    _ => new_lits.push(l2),
                }
            }
            if fs_debug_hit(&self.fsweep.debug_vars, slice)
                || fs_debug_hit(&self.fsweep.debug_vars, &new_lits)
            {
                eprintln!(
                    "c FSDBG subst from={from} to={to} old={:?} new={:?} sat={satisfied} conflicts={}",
                    self.clause_slice(cref),
                    new_lits,
                    self.stats.conflicts
                );
            }
            if satisfied {
                self.delete_clause_for_simplify(cref, proof_log);
                continue;
            }
            match new_lits.len() {
                0 => {
                    proof_log.record_clause(&[]);
                    self.has_empty_clause = true;
                    self.solver_ok = false;
                    stopped_at = Some(i + 1);
                    break 'walk;
                }
                1 => {
                    // kissat: assign the unit and stop the walk; the remaining
                    // occurrences are handled by root propagation.
                    let unit = new_lits[0];
                    self.stats.fsweep_units += 1;
                    if !self.learn_lucky_failed_literal_units(&[unit], proof_log) {
                        self.solver_ok = false;
                    }
                    kept.push(cref_w);
                    stopped_at = Some(i + 1);
                    break 'walk;
                }
                _ => {
                    self.fs_rewrite_clause(cref, &new_lits, proof_log);
                    self.stats.fsweep_substituted += 1;
                    delayed.push(cref_w);
                }
            }
        }
        if let Some(stop) = stopped_at {
            kept.extend_from_slice(&list[stop..]);
        }
        self.fsweep.occ[li] = kept;
        self.fsweep.occ[lit_to_index(to)].extend_from_slice(&delayed);
    }

    /// In-place clause rewrite at root with proof (inprocess_strengthen_clause
    /// sibling that also allows equal size — pure literal replacement).
    fn fs_rewrite_clause(&mut self, cref: usize, new_lits: &[i32], proof_log: &mut ProofLog) {
        debug_assert_eq!(self.current_level(), 0);
        debug_assert!(!self.clause_is_deleted(cref));
        debug_assert!(new_lits.len() >= 2);
        debug_assert!(!self.clause_is_learnt(cref));
        let old_len = self.clause_len(cref);
        let write = new_lits.len();
        debug_assert!(write <= old_len);
        // SAT_EXTRACT_CACHE hook (mirror of install/delete): an in-place
        // rewrite changes the gate neighborhood of every var the clause
        // mentions, old and new (len<=3 only — see delete_clause_for_simplify).
        if self.extract_cache.recording && (old_len <= 3 || write <= 3) {
            for pos in 0..old_len {
                let v = self.clause_lit(cref, pos).unsigned_abs() as usize;
                self.extract_cache.mark_touched(v);
            }
            for &l in new_lits {
                self.extract_cache.mark_touched(l.unsigned_abs() as usize);
            }
        }
        proof_log.record_clause(new_lits);
        proof_log.record_deletion(self.clause_slice(cref));
        self.detach_clause_for_rewrite(cref);
        let header = self.clause_header(cref);
        if clause_header_has_extra(header) {
            let extra_words = clause_header_extra_words(header);
            let old_extra = cref + 1 + old_len;
            let new_extra = cref + 1 + write;
            for o in 0..extra_words {
                self.arena[new_extra + o] = self.arena[old_extra + o];
            }
            // The first extra word on originals is the literal abstraction —
            // recompute it for the new literal set.
            if extra_words > 0 {
                self.arena[new_extra] = clause_abstraction_from_lits(new_lits) as u32;
            }
        }
        for (pos, &l) in new_lits.iter().enumerate() {
            self.set_clause_lit(cref, pos, l);
        }
        self.arena[cref] = clause_make_header(
            write,
            clause_header_learnt(header),
            clause_header_has_extra(header),
            clause_header_mark(header),
            clause_header_reloced(header),
        );
        if write < old_len {
            let removed = old_len - write;
            self.original_literals -= removed;
            self.deleted_clause_words += removed;
        }
        self.reattach_clause_after_rewrite(cref);
    }

    // ---- per-variable driver (kissat sweep_variable) ----

    fn fs_sweep_variable(&mut self, idx: usize, lim: &FsLimits, proof_log: &mut ProofLog) -> bool {
        if !self.fs_var_active(idx) {
            return false;
        }
        let start = idx as i32;
        if self.fs_find_repr(start) != start {
            return false;
        }
        // fresh environment
        for &v in &self.fsweep.env_vars {
            self.fsweep.depths[v as usize] = 0;
        }
        self.fsweep.env_vars.clear();
        for &v in &self.fsweep.kmap_vars {
            self.fsweep.env_kvar[v as usize] = 0;
        }
        self.fsweep.kmap_vars.clear();
        self.fsweep.swept.clear();
        self.fsweep.emitted.clear();
        self.fsweep.backbone.clear();
        self.fsweep.partition.clear();

        let mut kitten = Kitten::new();
        kitten.set_fast_mode(true);
        kitten.set_assumption_complete(true);

        self.fs_add_env_var(0, start);
        let mut expand = 0usize;
        let mut next_bound = 1usize;
        let mut depth = 1u32;
        let mut encoded = 0usize;
        let mut limit_reached = false;
        while !limit_reached {
            if encoded >= lim.clauses {
                limit_reached = true;
                break;
            }
            if expand == next_bound {
                if depth >= lim.depth {
                    break;
                }
                next_bound = self.fsweep.env_vars.len();
                if expand == next_bound {
                    break;
                }
                depth += 1;
            }
            if expand >= self.fsweep.env_vars.len() {
                break;
            }
            let v = self.fsweep.env_vars[expand] as usize;
            'signs: for sign in 0..2 {
                let lit = if sign == 0 { v as i32 } else { -(v as i32) };
                let li = lit_to_index(lit);
                let occ_list = std::mem::take(&mut self.fsweep.occ[li]);
                for &cref_w in &occ_list {
                    if self.fs_encode_clause(&mut kitten, depth, cref_w as usize) {
                        encoded += 1;
                    }
                    if self.fsweep.env_vars.len() >= lim.vars {
                        limit_reached = true;
                        self.fsweep.occ[li] = occ_list;
                        break 'signs;
                    }
                    if encoded >= lim.clauses {
                        limit_reached = true;
                        self.fsweep.occ[li] = occ_list;
                        break 'signs;
                    }
                }
                self.fsweep.occ[li] = occ_list;
            }
            expand += 1;
        }
        self.stats.fsweep_envs += 1;
        self.stats.fsweep_env_clauses += encoded as u64;

        let mut success = false;
        match self.fs_solve(&mut kitten, &[], lim) {
            None => {}
            Some(KittenResult::Unsat) => {
                // Environment (⊆ formula) is UNSAT outright.
                self.fs_emit_core(&kitten, proof_log);
                proof_log.record_clause(&[]);
                self.has_empty_clause = true;
                self.solver_ok = false;
            }
            Some(KittenResult::Sat) => {
                // init_backbone_and_partition
                let env_vars = self.fsweep.env_vars.clone();
                for &vu in &env_vars {
                    let v = vu as usize;
                    if !self.fs_var_active(v) {
                        continue;
                    }
                    let kv = self.fsweep.env_kvar[v];
                    if kv == 0 {
                        continue;
                    }
                    let candidate = match kitten.value(kv as usize) {
                        Some(true) => v as i32,
                        Some(false) => -(v as i32),
                        None => continue,
                    };
                    self.fsweep.backbone.push(candidate);
                    self.fsweep.partition.push(candidate);
                }
                self.fsweep.partition.push(0);

                // backbone phase
                while !self.fsweep.backbone.is_empty() {
                    if self.has_empty_clause
                        || !self.solver_ok
                        || self.fsweep.kitten_ticks >= lim.ticks
                    {
                        break;
                    }
                    self.fs_flip_backbone(&mut kitten, lim);
                    if self.fsweep.kitten_ticks >= lim.ticks {
                        break;
                    }
                    let Some(lit) = self.fsweep.backbone.pop() else {
                        break;
                    };
                    if !self.fs_var_active(lit.unsigned_abs() as usize) {
                        continue;
                    }
                    if self.fs_backbone_candidate(&mut kitten, lit, lim, proof_log) {
                        success = true;
                    }
                }

                // equivalence phase
                while !self.fsweep.partition.is_empty() {
                    if self.has_empty_clause
                        || !self.solver_ok
                        || self.fsweep.kitten_ticks >= lim.ticks
                    {
                        break;
                    }
                    self.fs_flip_partition(&mut kitten);
                    if self.fsweep.kitten_ticks >= lim.ticks {
                        break;
                    }
                    let n = self.fsweep.partition.len();
                    if n > 2 {
                        let lit = self.fsweep.partition[n - 3];
                        let other = self.fsweep.partition[n - 2];
                        if self.fs_equivalence_candidates(&mut kitten, lit, other, lim, proof_log)
                        {
                            success = true;
                        }
                    } else {
                        self.fsweep.partition.clear();
                    }
                }
            }
        }
        success
    }
}
