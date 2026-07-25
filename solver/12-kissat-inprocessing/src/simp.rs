use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};

use crate::kitten::{Kitten, KittenResult};

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubsumptionCandidate {
    Clause(usize),
    RootUnit(i32),
}

enum SubsumptionOutcome {
    None,
    Subsumed,
    Strengthen(i32),
}

/// Persistent `eliminate()` round workspaces (SAT_ROUND_DIET): the subsumption
/// queue, touched/BSR-touched worklists + flags, the elimination candidate heap,
/// and the heap version stamps are reused across rounds instead of being
/// reallocated (three `vec![_; vars]` fills plus heap growth) on every root or
/// mid-search armed round.
///
/// Trajectory-identity argument: the flags obey `flag[v] == true ⟺ v` present in
/// the paired worklist, so clearing the listed entries restores the all-false
/// round-entry state on every exit; the queue is fully drained by
/// `clear_subsumption_queue_marks` on every exit; the heap is cleared at round
/// entry; and the carried-over version stamps only ever compare between entries
/// of the SAME variable, where relative order (older < newer) is preserved
/// because per-variable stamps only grow.
#[derive(Default)]
pub(crate) struct ElimRoundWs {
    queue: VecDeque<SubsumptionCandidate>,
    touched: Vec<usize>,
    touched_flags: Vec<bool>,
    bsr_touched: Vec<usize>,
    bsr_touched_flags: Vec<bool>,
    heap: BinaryHeap<Reverse<(u64, usize, u32)>>,
    heap_versions: Vec<u32>,
}

enum PreprocessBudgetKind {
    Resolution,
    Tick,
}

/// A detected functional definition of a pivot variable as an AND/OR gate, used by
/// gate-aware BVE (`SAT_GATE_BVE`). The pivot's clauses are partitioned into the gate
/// clauses (the definition `x <-> OR(o1..ok)`: one base clause + k binaries) and the
/// remaining non-gate clauses, on each polarity side. By Plaisted-Greenbaum substitution
/// only gate-vs-nongate resolvents are needed; nongate-vs-nongate resolvents are implied
/// by the gate definition and are sound to skip. For AND/OR (and equivalence) gates the
/// gate-vs-gate resolvents are tautologies, so they are skipped too (`resolve_gate=false`).
struct GatePartition {
    /// clauses containing +pivot that form the gate definition
    gate_pos: Vec<usize>,
    /// clauses containing -pivot that form the gate definition
    gate_neg: Vec<usize>,
    /// clauses containing +pivot that are not part of the gate definition
    nongate_pos: Vec<usize>,
    /// clauses containing -pivot that are not part of the gate definition
    nongate_neg: Vec<usize>,
    /// which detector produced the definition (stats attribution only)
    kind: ElimGateKind,
}

/// Which gate detector recognized the elimination pivot's definition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ElimGateKind {
    AndOr,
    Equivalence,
    Ite,
    /// Semantic definition found by the kitten sub-solver (kissat definition.c).
    /// Unlike the syntactic kinds, gate-vs-gate resolvents are NOT necessarily
    /// tautologies, so the resolution loop must include them (kissat `resolve_gate`).
    Definition,
}

const MARKED_SUBSUMPTION_MIN_PRODUCT: usize = 32;

/// Env-gated (SAT_TRACE_ELIM=1) fine-grained wall counters for try_eliminate_var.
/// eprintln-only measurement aid; zero effect on solver behavior.
mod elim_trace {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::OnceLock;
    pub static SETUP_NS: AtomicU64 = AtomicU64::new(0);
    pub static PARTITION_NS: AtomicU64 = AtomicU64::new(0);
    pub static GATE_NS: AtomicU64 = AtomicU64::new(0);
    pub static RESOLVE_NS: AtomicU64 = AtomicU64::new(0);
    pub static APPLY_NS: AtomicU64 = AtomicU64::new(0);
    pub static APPLY_PUSHELIM_NS: AtomicU64 = AtomicU64::new(0);
    pub static APPLY_PROOFSNAP_NS: AtomicU64 = AtomicU64::new(0);
    pub static APPLY_REMOVE_NS: AtomicU64 = AtomicU64::new(0);
    pub static APPLY_ADD_NS: AtomicU64 = AtomicU64::new(0);
    pub static APPLY_PROOFDEL_NS: AtomicU64 = AtomicU64::new(0);
    pub static ADD_NORM_NS: AtomicU64 = AtomicU64::new(0);
    pub static ADD_PROOF_NS: AtomicU64 = AtomicU64::new(0);
    pub static ADD_ARENA_NS: AtomicU64 = AtomicU64::new(0);
    pub static ADD_ATTACH_NS: AtomicU64 = AtomicU64::new(0);
    pub static ADD_INDEX_NS: AtomicU64 = AtomicU64::new(0);
    pub static ADD_ENQ_NS: AtomicU64 = AtomicU64::new(0);
    pub fn enabled() -> bool {
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var("SAT_TRACE_ELIM").is_ok())
    }
    /// A timing token: `None` when tracing is off, so the disabled path costs
    /// one predictable branch and never calls `Instant::now()`.
    #[derive(Clone, Copy)]
    pub struct T(pub Option<std::time::Instant>);
    impl T {
        pub fn elapsed_opt(self) -> Option<std::time::Duration> {
            self.0.map(|s| s.elapsed())
        }
    }
    pub fn start(enabled: bool) -> T {
        T(if enabled {
            Some(std::time::Instant::now())
        } else {
            None
        })
    }
    pub fn add(counter: &AtomicU64, t: T) {
        if let Some(s) = t.0 {
            counter.fetch_add(s.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
    }
    pub fn secs(counter: &AtomicU64) -> f64 {
        counter.load(Ordering::Relaxed) as f64 / 1e9
    }
}

impl Solver {
    fn variable_count(&self) -> usize {
        self.assignment.len().saturating_sub(1)
    }

    fn preprocessing_candidate(&self, var: usize) -> bool {
        var > 0
            && var < self.assignment.len()
            && self.use_elim
            && !self.frozen[var]
            && !self.eliminated[var]
            && self.assignment[var] == UNASSIGNED
            && self.within_eliminate_occurrence_limit(var)
    }

    /// kissat `eliminateocclim` (options.h:47, eliminate.c:44-50): skip variables occurring in
    /// more than `eliminate_occurrence_limit` clauses per polarity, to bound the O(pos*neg)
    /// resolvent gate loop in `try_eliminate_var`. `0` = unlimited (the shipped default), so
    /// default behavior is unchanged. Bead SAT-playground-5b2.3.22 (root-cause fix for the
    /// unbounded BVE pass that OOMs at 14GB on VexRiscv).
    fn within_eliminate_occurrence_limit(&self, var: usize) -> bool {
        let limit = self.eliminate_occurrence_limit;
        if limit == 0 {
            return true;
        }
        let pos = self.n_occ[lit_to_index(var as i32)] as u64;
        let neg = self.n_occ[lit_to_index(-(var as i32))] as u64;
        pos <= limit && neg <= limit
    }

    fn occurrence_cost(&self, var: usize) -> u64 {
        let pos = lit_to_index(var as i32);
        let neg = lit_to_index(-(var as i32));
        (self.n_occ[pos] as u64) * (self.n_occ[neg] as u64)
    }

    fn live_occurrence_count(&self, var: usize) -> usize {
        if var == 0 || var >= self.occurs.len() {
            return usize::MAX;
        }
        if self.n_occ.is_empty() || var > i32::MAX as usize {
            return self.occurs[var].len();
        }
        let lit = var as i32;
        let pos = lit_to_index(lit);
        let neg = lit_to_index(-lit);
        match (self.n_occ.get(pos), self.n_occ.get(neg)) {
            (Some(&pos_count), Some(&neg_count)) => {
                (pos_count as usize).saturating_add(neg_count as usize)
            }
            _ => self.occurs[var].len(),
        }
    }

    fn should_run_full_backward_subsumption(&self) -> bool {
        if !self.full_bsr {
            return false;
        }
        if self.bsr_formula_gate && self.pre_preprocess_class_skips_bsr() {
            return false;
        }
        true
    }

    // Adaptive rule from log/analyzesat-2026-05-26-preprocess/FINDINGS.md Gap PRE-3:
    // full backward subsumption is a net loss on large, sparse, low-binary formulas
    // (Kakuro -79%, velev -79%) but stays useful on random 3-SAT and brocard. Only
    // consulted when SAT_BSR_FORMULA_GATE is on; the pre-preprocess snapshot is taken
    // in main.rs right before `eliminate()` is called.
    fn pre_preprocess_class_skips_bsr(&self) -> bool {
        let class = self.pre_preprocess_formula_class;
        matches!(class.size_class, FormulaSizeClass::Large)
            && class.binary_fraction < 0.05
            && class.variable_density > 100.0
    }

    fn note_preprocess_budget_hit(&mut self, kind: PreprocessBudgetKind) {
        if !self.preprocess_budget_exhausted {
            match kind {
                PreprocessBudgetKind::Resolution => {
                    self.stats.preprocess_resolution_budget_hits += 1;
                }
                PreprocessBudgetKind::Tick => {
                    self.stats.preprocess_tick_budget_hits += 1;
                }
            }
        }
        self.preprocess_budget_exhausted = true;
    }

    fn note_preprocess_bsr_tick_budget_hit(&mut self) {
        if !self.preprocess_bsr_budget_exhausted {
            self.stats.preprocess_bsr_tick_budget_hits += 1;
        }
        self.preprocess_bsr_budget_exhausted = true;
    }

    fn consume_eliminate_tick(&mut self) -> bool {
        if self.eliminate_ticks_budget == 0 {
            return true;
        }
        if self.stats.preprocess_eliminate_ticks >= self.eliminate_ticks_budget {
            self.note_preprocess_budget_hit(PreprocessBudgetKind::Tick);
            return false;
        }
        self.stats.preprocess_eliminate_ticks =
            self.stats.preprocess_eliminate_ticks.saturating_add(1);
        true
    }

    fn consume_bsr_tick(&mut self) -> bool {
        if self.eliminate_ticks_budget == 0 {
            return true;
        }
        if self.stats.preprocess_bsr_ticks >= self.eliminate_ticks_budget {
            self.note_preprocess_bsr_tick_budget_hit();
            return false;
        }
        self.stats.preprocess_bsr_ticks = self.stats.preprocess_bsr_ticks.saturating_add(1);
        true
    }

    fn consume_eliminate_resolution_attempt(&mut self) -> bool {
        if self.eliminate_resolution_budget != 0
            && self.stats.preprocess_resolution_attempts >= self.eliminate_resolution_budget
        {
            self.note_preprocess_budget_hit(PreprocessBudgetKind::Resolution);
            return false;
        }
        if !self.consume_eliminate_tick() {
            return false;
        }
        if self.eliminate_resolution_budget != 0 {
            self.stats.preprocess_resolution_attempts =
                self.stats.preprocess_resolution_attempts.saturating_add(1);
        }
        true
    }

    /// Giant formulas (tens of millions of vars) OOM when the full occurrence index is
    /// materialized: the occ-lists are multi-GB working memory stacked on an already ~13-15GB
    /// base (e.g. 00fd8ac: 23.4M vars, base VmPeak 15.6GB). `Some(cap)` selects a PARTIAL
    /// occurrence index that materializes `occurs[var]` only for vars whose per-polarity
    /// degree is `<= cap`; high-degree vars (never good BVE candidates — `occurrence_limit`
    /// skips them anyway) are left unindexed. `None` = build the full index (the shipped
    /// behavior). Gated far above every normal medium instance, so they are byte-for-byte
    /// unaffected. Tunable via SAT_PARTIAL_OCC_MIN_VARS / SAT_PARTIAL_OCC_CAP.
    fn partial_occurrence_cap(&self, num_vars: usize) -> Option<u64> {
        let threshold = std::env::var("SAT_PARTIAL_OCC_MIN_VARS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(20_000_000usize);
        if num_vars <= threshold {
            return None;
        }
        let cap = std::env::var("SAT_PARTIAL_OCC_CAP")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(10_000u64);
        Some(cap)
    }

    fn build_occurrence_index(&mut self) {
        self.ensure_original_clause_abstractions();
        let num_vars = self.variable_count();
        self.occurs.clear();
        self.occurs.resize_with(num_vars + 1, Vec::new);
        self.occurs_dirty.clear();
        self.occurs_dirty.resize(num_vars + 1, false);
        self.occurs_membership_dirty.clear();
        self.occurs_membership_dirty.resize(num_vars + 1, false);
        self.n_occ.clear();
        self.n_occ.resize(num_vars.saturating_mul(2), 0);

        let original_clause_ids = self.original_clause_ids.clone();

        if let Some(cap) = self.partial_occurrence_cap(num_vars) {
            // Pass 1: degrees only (cheap; no per-var occurrence storage).
            for &clause_idx in &original_clause_ids {
                let clause_idx = clause_idx as usize;
                if self.clause_is_deleted(clause_idx) {
                    continue;
                }
                let clause_len = self.clause_len(clause_idx);
                for lit_pos in 0..clause_len {
                    let lit = self.clause_lit(clause_idx, lit_pos);
                    let idx = lit_to_index(lit);
                    if idx < self.n_occ.len() {
                        self.n_occ[idx] += 1;
                    }
                }
            }
            // Cap BVE candidate selection to the materialized (low-degree) set so we never
            // attempt to eliminate a var whose occurs list is intentionally incomplete.
            if self.eliminate_occurrence_limit == 0 || self.eliminate_occurrence_limit > cap {
                self.eliminate_occurrence_limit = cap;
            }
            self.stats.partial_occurrence_index = 1;
            // Pass 2: materialize occurs only for low-degree vars. Sound: any var we later
            // eliminate has a complete occurs list; subsumption over the empty high-degree
            // lists simply finds fewer candidates (never a wrong removal).
            for &clause_idx in &original_clause_ids {
                let clause_idx = clause_idx as usize;
                if self.clause_is_deleted(clause_idx) {
                    continue;
                }
                let clause_len = self.clause_len(clause_idx);
                for lit_pos in 0..clause_len {
                    let lit = self.clause_lit(clause_idx, lit_pos);
                    let var = lit.unsigned_abs() as usize;
                    if var == 0 || var >= self.occurs.len() {
                        continue;
                    }
                    let pos = self.n_occ[lit_to_index(var as i32)] as u64;
                    let neg = self.n_occ[lit_to_index(-(var as i32))] as u64;
                    if pos <= cap && neg <= cap {
                        self.occurs[var].push(clause_idx as u32);
                    }
                }
            }
        } else {
            for clause_idx in original_clause_ids {
                let clause_idx = clause_idx as usize;
                if !self.clause_is_deleted(clause_idx) {
                    self.index_original_clause(clause_idx);
                }
            }
        }
    }

    fn index_original_clause(&mut self, clause_idx: usize) {
        debug_assert!(!self.clause_is_learnt(clause_idx));
        let clause_len = self.clause_len(clause_idx);
        for lit_pos in 0..clause_len {
            let lit = self.clause_lit(clause_idx, lit_pos);
            let var = lit.unsigned_abs() as usize;
            if var == 0 || var >= self.occurs.len() {
                continue;
            }
            self.occurs[var].push(clause_idx as u32);
            self.n_occ[lit_to_index(lit)] += 1;
        }
    }

    fn clean_occurs<const TRACE: bool>(&mut self, var: usize) {
        if TRACE {
            self.stats.occurs_clean_calls += 1;
        }
        if var >= self.occurs.len()
            || (!self.occurs_dirty[var] && !self.occurs_membership_dirty[var])
        {
            return;
        }

        if TRACE {
            self.stats.occurs_clean_dirty_calls += 1;
        }
        let arena = &self.arena;
        let check_membership = self.occurs_membership_dirty[var];
        if TRACE && check_membership {
            self.stats.occurs_clean_membership_calls += 1;
        }
        let occurs = &mut self.occurs[var];
        let old_len = occurs.len();
        if TRACE {
            self.stats.occurs_clean_entries_scanned += old_len as u64;
        }
        let mut write = 0usize;
        for read in 0..occurs.len() {
            let clause_idx = occurs[read] as usize;
            if clause_idx < arena.len()
                && clause_header_mark(arena[clause_idx]) != CLAUSE_DELETED_MARK
                && (!check_membership || clause_contains_var_in_arena(arena, clause_idx, var))
            {
                occurs[write] = clause_idx as u32;
                write += 1;
            }
        }
        occurs.truncate(write);
        if TRACE {
            self.stats.occurs_clean_entries_removed += old_len.saturating_sub(write) as u64;
        }
        self.occurs_dirty[var] = false;
        self.occurs_membership_dirty[var] = false;
    }

    fn clean_occurs_dynamic(&mut self, var: usize) {
        if self.trace_preprocess_details {
            self.clean_occurs::<true>(var);
        } else {
            self.clean_occurs::<false>(var);
        }
    }

    fn enqueue_subsumption_clause(
        &mut self,
        queue: &mut VecDeque<SubsumptionCandidate>,
        clause_idx: usize,
    ) {
        if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
            return;
        }
        if clause_header_mark(self.clause_header(clause_idx)) != 0 {
            return;
        }
        let header = self.clause_header(clause_idx);
        self.arena[clause_idx] = clause_make_header(
            clause_header_size(header),
            clause_header_learnt(header),
            clause_header_has_extra(header),
            2,
            clause_header_reloced(header),
        ) | (header & CLAUSE_SEARCHED_POS_MASK);
        queue.push_back(SubsumptionCandidate::Clause(clause_idx));
    }

    fn clear_subsumption_clause_mark(&mut self, clause_idx: usize) {
        if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
            return;
        }
        if clause_header_mark(self.clause_header(clause_idx)) != 2 {
            return;
        }
        let header = self.clause_header(clause_idx);
        self.arena[clause_idx] = clause_make_header(
            clause_header_size(header),
            clause_header_learnt(header),
            clause_header_has_extra(header),
            0,
            clause_header_reloced(header),
        ) | (header & CLAUSE_SEARCHED_POS_MASK);
    }

    fn clear_subsumption_queue_marks(&mut self, queue: &mut VecDeque<SubsumptionCandidate>) {
        while let Some(candidate) = queue.pop_front() {
            if let SubsumptionCandidate::Clause(clause_idx) = candidate {
                self.clear_subsumption_clause_mark(clause_idx);
            }
        }
    }

    fn touch_preprocess_var(touched: &mut Vec<usize>, touched_flags: &mut Vec<bool>, var: usize) {
        if var == 0 {
            return;
        }
        if var >= touched_flags.len() {
            touched_flags.resize(var + 1, false);
        }
        if touched_flags[var] {
            return;
        }
        touched_flags[var] = true;
        touched.push(var);
    }

    fn touch_preprocess_var_and_bsr(
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
        bsr_touched: &mut Vec<usize>,
        bsr_touched_flags: &mut Vec<bool>,
        var: usize,
    ) {
        Self::touch_preprocess_var(touched, touched_flags, var);
        Self::touch_preprocess_var(bsr_touched, bsr_touched_flags, var);
    }

    fn gather_touched_clauses(
        &mut self,
        touched: &mut Vec<usize>,
        touched_flags: &mut [bool],
        bsr_touched: &mut Vec<usize>,
        bsr_touched_flags: &mut [bool],
        queue: &mut VecDeque<SubsumptionCandidate>,
        heap: &mut BinaryHeap<Reverse<(u64, usize, u32)>>,
        heap_versions: &mut [u32],
        enqueue_subsumption_work: bool,
    ) {
        let vars = std::mem::take(touched);
        for var in vars {
            if var < touched_flags.len() {
                touched_flags[var] = false;
            }
            if var == 0 || var >= self.occurs.len() {
                continue;
            }
            self.clean_occurs_dynamic(var);
            if var < heap_versions.len() && self.preprocessing_candidate(var) {
                heap_versions[var] = heap_versions[var].wrapping_add(1);
                heap.push(Reverse((
                    self.occurrence_cost(var),
                    var,
                    heap_versions[var],
                )));
            }
        }

        let bsr_vars = std::mem::take(bsr_touched);
        for var in bsr_vars {
            if var < bsr_touched_flags.len() {
                bsr_touched_flags[var] = false;
            }
            if !enqueue_subsumption_work || var == 0 || var >= self.occurs.len() {
                continue;
            }
            self.clean_occurs_dynamic(var);
            let mut scan_pos = 0usize;
            while scan_pos < self.occurs[var].len() {
                let clause_idx = self.occurs[var][scan_pos] as usize;
                scan_pos += 1;
                self.enqueue_subsumption_clause(queue, clause_idx);
            }
        }
    }

    fn mark_occurs_dirty_for_clause(
        &mut self,
        clause_idx: usize,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
    ) {
        let clause_len = self.clause_len(clause_idx);
        for lit_pos in 0..clause_len {
            let lit = self.clause_lit(clause_idx, lit_pos);
            let var = lit.unsigned_abs() as usize;
            if var == 0 || var >= self.occurs_dirty.len() {
                continue;
            }
            let lit_idx = lit_to_index(lit);
            self.n_occ[lit_idx] = self.n_occ[lit_idx].saturating_sub(1);
            self.occurs_dirty[var] = true;
            Self::touch_preprocess_var(touched, touched_flags, var);
        }
    }

    fn remove_original_clause_preprocess(
        &mut self,
        clause_idx: usize,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
    ) {
        if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
            return;
        }
        debug_assert!(!self.clause_is_learnt(clause_idx));

        if self.clause_locked(clause_idx) {
            self.clear_reason_for_locked_clause(clause_idx);
        }

        self.mark_occurs_dirty_for_clause(clause_idx, touched, touched_flags);
        let clause_len = self.clause_len(clause_idx);
        if self.should_lazy_detach_preprocess_originals() {
            self.detach_clause(clause_idx);
        } else {
            self.detach_clause_strict(clause_idx);
        }
        self.original_literals = self.original_literals.saturating_sub(clause_len);
        self.deleted_clause_words += self.clause_word_len(clause_idx);
        self.clause_set_deleted(clause_idx, true);
        self.stats.deleted_clauses += 1;
    }

    fn subsumption_relation<const TRACE: bool>(
        &mut self,
        driver: SubsumptionCandidate,
        driver_len: usize,
        driver_abstraction: u64,
        candidate_idx: usize,
        relation_marks: &mut Vec<u32>,
        relation_mark_stamp: &mut u32,
    ) -> SubsumptionOutcome {
        if candidate_idx >= self.arena.len() || self.clause_is_deleted(candidate_idx) {
            return SubsumptionOutcome::None;
        }
        if TRACE {
            self.stats.bsr_relation_calls += 1;
        }
        let candidate_len = self.clause_len(candidate_idx);
        if driver_len > candidate_len {
            if TRACE {
                self.stats.bsr_relation_len_reject += 1;
            }
            return SubsumptionOutcome::None;
        }
        if (driver_abstraction & !self.original_clause_abstraction(candidate_idx)) != 0 {
            if TRACE {
                self.stats.bsr_relation_abstraction_reject += 1;
            }
            return SubsumptionOutcome::None;
        }

        if self.clauses_sorted_by_var
            && self.inline_original_abstractions
            && driver_len >= SORTED_SUBSUMPTION_MIN_LEN
        {
            if TRACE {
                self.stats.bsr_relation_sorted_calls += 1;
            }
            return self.sorted_subsumption_relation::<TRACE>(driver, driver_len, candidate_idx);
        }

        if driver_len.saturating_mul(candidate_len) >= MARKED_SUBSUMPTION_MIN_PRODUCT {
            if TRACE {
                self.stats.bsr_relation_marked_calls += 1;
            }
            return self.marked_subsumption_relation::<TRACE>(
                driver,
                driver_len,
                candidate_idx,
                relation_marks,
                relation_mark_stamp,
            );
        }

        if TRACE {
            self.stats.bsr_relation_nested_calls += 1;
        }
        let candidate_lits = self.clause_slice(candidate_idx);

        let mut remove_lit = 0i32;
        for driver_pos in 0..driver_len {
            let driver_lit = self.subsumption_driver_lit(driver, driver_pos);
            let mut found = false;
            for &candidate_lit in candidate_lits {
                if driver_lit == candidate_lit {
                    found = true;
                    break;
                }
                if remove_lit == 0 && driver_lit == -candidate_lit {
                    remove_lit = candidate_lit;
                    found = true;
                    break;
                }
            }
            if !found {
                return SubsumptionOutcome::None;
            }
        }

        if remove_lit == 0 {
            if TRACE {
                self.stats.bsr_relation_subsumed += 1;
            }
            SubsumptionOutcome::Subsumed
        } else {
            if TRACE {
                self.stats.bsr_relation_strengthen += 1;
            }
            SubsumptionOutcome::Strengthen(remove_lit)
        }
    }

    fn marked_subsumption_relation<const TRACE: bool>(
        &mut self,
        driver: SubsumptionCandidate,
        driver_len: usize,
        candidate_idx: usize,
        relation_marks: &mut Vec<u32>,
        relation_mark_stamp: &mut u32,
    ) -> SubsumptionOutcome {
        let lit_slots = self.variable_count().saturating_mul(2);
        if relation_marks.len() < lit_slots {
            relation_marks.resize(lit_slots, 0);
        }
        *relation_mark_stamp = relation_mark_stamp.wrapping_add(1);
        if *relation_mark_stamp == 0 {
            relation_marks.fill(0);
            *relation_mark_stamp = 1;
        }
        let mark = *relation_mark_stamp;

        for &candidate_lit in self.clause_slice(candidate_idx) {
            relation_marks[lit_to_index(candidate_lit)] = mark;
        }

        let mut remove_lit = 0i32;
        for driver_pos in 0..driver_len {
            let driver_lit = self.subsumption_driver_lit(driver, driver_pos);
            if relation_marks[lit_to_index(driver_lit)] == mark {
                continue;
            }

            let candidate_complement = -driver_lit;
            if relation_marks[lit_to_index(candidate_complement)] != mark || remove_lit != 0 {
                return SubsumptionOutcome::None;
            }
            remove_lit = candidate_complement;
        }

        if remove_lit == 0 {
            if TRACE {
                self.stats.bsr_relation_subsumed += 1;
            }
            SubsumptionOutcome::Subsumed
        } else {
            if TRACE {
                self.stats.bsr_relation_strengthen += 1;
            }
            SubsumptionOutcome::Strengthen(remove_lit)
        }
    }

    fn sorted_subsumption_relation<const TRACE: bool>(
        &mut self,
        driver: SubsumptionCandidate,
        driver_len: usize,
        candidate_idx: usize,
    ) -> SubsumptionOutcome {
        let candidate_lits = self.clause_slice(candidate_idx);
        let mut candidate_pos = 0usize;
        let mut remove_lit = 0i32;

        for driver_pos in 0..driver_len {
            let driver_lit = self.subsumption_driver_lit(driver, driver_pos);
            let driver_var = driver_lit.unsigned_abs();
            let mut found = false;

            while candidate_pos < candidate_lits.len() {
                let candidate_lit = candidate_lits[candidate_pos];
                let candidate_var = candidate_lit.unsigned_abs();
                if candidate_var < driver_var {
                    candidate_pos += 1;
                    continue;
                }
                if candidate_var > driver_var {
                    return SubsumptionOutcome::None;
                }
                candidate_pos += 1;
                if candidate_lit == driver_lit {
                    found = true;
                    break;
                }
                if remove_lit == 0 && candidate_lit == -driver_lit {
                    remove_lit = candidate_lit;
                    found = true;
                    break;
                }
                return SubsumptionOutcome::None;
            }

            if !found {
                return SubsumptionOutcome::None;
            }
        }

        if remove_lit == 0 {
            if TRACE {
                self.stats.bsr_relation_subsumed += 1;
            }
            SubsumptionOutcome::Subsumed
        } else {
            if TRACE {
                self.stats.bsr_relation_strengthen += 1;
            }
            SubsumptionOutcome::Strengthen(remove_lit)
        }
    }

    fn strengthen_original_clause_preprocess(
        &mut self,
        clause_idx: usize,
        remove_lit: i32,
        proof_log: &mut ProofLog,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
        bsr_touched: &mut Vec<usize>,
        bsr_touched_flags: &mut Vec<bool>,
        queue: &mut VecDeque<SubsumptionCandidate>,
    ) -> bool {
        if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
            return true;
        }

        let clause_len = self.clause_len(clause_idx);
        let was_locked = self.clause_locked(clause_idx);
        let mut remove_pos = None;
        let mut strengthened = std::mem::take(&mut self.scratch_preprocess_clause);
        strengthened.clear();
        strengthened.reserve(clause_len.saturating_sub(1));
        let mut strengthened_abstraction = 0u64;
        for lit_pos in 0..clause_len {
            let lit = self.clause_lit(clause_idx, lit_pos);
            if lit == remove_lit && remove_pos.is_none() {
                remove_pos = Some(lit_pos);
                continue;
            }
            strengthened.push(lit);
            strengthened_abstraction |= 1u64 << ((lit.unsigned_abs() - 1) & 31);
        }

        let Some(_remove_pos) = remove_pos else {
            self.scratch_preprocess_clause = strengthened;
            return true;
        };

        proof_log.record_clause(&strengthened);
        proof_log.record_deletion(self.clause_slice(clause_idx));
        self.stats.preprocess_strengthened_clauses += 1;

        if clause_len == 2 {
            let unit_lit = strengthened[0];
            self.scratch_preprocess_clause = strengthened;
            self.remove_original_clause_preprocess(clause_idx, touched, touched_flags);
            if !self.enqueue(unit_lit, ReasonRef::None) || self.propagate().is_some() {
                self.solver_ok = false;
                return false;
            }
            return true;
        }

        let remove_var = remove_lit.unsigned_abs() as usize;
        if self.inline_original_abstractions && remove_var < self.occurs_membership_dirty.len() {
            self.occurs_membership_dirty[remove_var] = true;
        } else if remove_var < self.occurs.len() {
            if let Some(pos) = self.occurs[remove_var]
                .iter()
                .position(|&idx| idx as usize == clause_idx)
            {
                self.occurs[remove_var].swap_remove(pos);
            }
        }
        if remove_lit != 0 && !self.n_occ.is_empty() {
            let lit_idx = lit_to_index(remove_lit);
            self.n_occ[lit_idx] = self.n_occ[lit_idx].saturating_sub(1);
        }
        Self::touch_preprocess_var_and_bsr(
            touched,
            touched_flags,
            bsr_touched,
            bsr_touched_flags,
            remove_var,
        );

        self.detach_clause(clause_idx);
        for (lit_pos, &lit) in strengthened.iter().enumerate() {
            self.set_clause_lit(clause_idx, lit_pos, lit);
        }

        let header = self.clause_header(clause_idx);
        self.arena[clause_idx] = clause_make_header(
            clause_len - 1,
            clause_header_learnt(header),
            clause_header_has_extra(header),
            clause_header_mark(header),
            clause_header_reloced(header),
        );
        self.original_literals = self.original_literals.saturating_sub(1);
        self.deleted_clause_words += 1;
        self.set_original_clause_abstraction(clause_idx, strengthened_abstraction);
        self.scratch_preprocess_clause = strengthened;

        if was_locked {
            self.clear_reason_for_locked_clause(clause_idx);
        }

        self.attach_clause(clause_idx, false);
        self.enqueue_subsumption_clause(queue, clause_idx);

        true
    }

    fn normalize_original_clause(&self, clause: &[i32]) -> Option<Vec<i32>> {
        let mut normalized = Vec::with_capacity(clause.len());
        if self.normalize_original_clause_into(clause, &mut normalized) {
            Some(normalized)
        } else {
            None
        }
    }

    /// `normalize_original_clause` writing into a caller-owned reusable buffer
    /// (allocation-free in steady state). Returns `false` where the allocating
    /// variant returns `None` (satisfied or tautological — caller skips the
    /// clause); returns `true` with `out` holding the normalized literals
    /// otherwise, including the defensive empty-clause case for out-of-range or
    /// eliminated variables (`Some(Vec::new())` in the allocating variant).
    fn normalize_original_clause_into(&self, clause: &[i32], out: &mut Vec<i32>) -> bool {
        out.clear();
        for &lit in clause {
            let var = lit.unsigned_abs() as usize;
            if var == 0 || var >= self.assignment.len() || self.eliminated[var] {
                out.clear();
                return true;
            }

            match self.lit_value(lit) {
                TRUE => return false,
                FALSE => {}
                UNASSIGNED => out.push(lit),
                _ => unreachable!(),
            }
        }

        out.sort_unstable_by(|&lhs, &rhs| {
            lhs.unsigned_abs()
                .cmp(&rhs.unsigned_abs())
                .then_with(|| lhs.cmp(&rhs))
        });

        let mut write = 0usize;
        let mut prev_lit = 0i32;
        for read in 0..out.len() {
            let lit = out[read];
            if write > 0 {
                if lit == prev_lit {
                    continue;
                }
                if lit == -prev_lit {
                    return false;
                }
            }
            out[write] = lit;
            write += 1;
            prev_lit = lit;
        }
        out.truncate(write);
        true
    }

    fn normalize_original_clause_input_order(&self, clause: &[i32]) -> Option<Vec<i32>> {
        let mut normalized = Vec::with_capacity(clause.len());
        'lits: for &lit in clause {
            let var = lit.unsigned_abs() as usize;
            if var == 0 || var >= self.assignment.len() || self.eliminated[var] {
                return Some(Vec::new());
            }

            match self.lit_value(lit) {
                TRUE => return None,
                FALSE => continue,
                UNASSIGNED => {}
                _ => unreachable!(),
            }

            for &existing in &normalized {
                if existing == lit {
                    continue 'lits;
                }
                if existing == -lit {
                    return None;
                }
            }
            normalized.push(lit);
        }
        Some(normalized)
    }

    fn move_kissat_initial_watch_to_front(
        &self,
        lits: &mut [i32],
        start: usize,
        satisfied_is_enough: bool,
    ) -> u8 {
        debug_assert!(lits.len() > 1);
        debug_assert!(start < lits.len());

        let current_lit = lits[start];
        let mut current_value = self.lit_value(current_lit);
        if current_value == UNASSIGNED || (current_value == TRUE && satisfied_is_enough) {
            return current_value;
        }

        let mut best_pos = start;
        let mut best_level = self.decision_level[current_lit.unsigned_abs() as usize];
        for (pos, &candidate_lit) in lits.iter().enumerate().skip(start + 1) {
            let candidate_value = self.lit_value(candidate_lit);
            if candidate_value == UNASSIGNED || (candidate_value == TRUE && satisfied_is_enough) {
                best_pos = pos;
                current_value = candidate_value;
                break;
            }

            let candidate_level = self.decision_level[candidate_lit.unsigned_abs() as usize];
            let better = match (current_value, candidate_value) {
                (FALSE, TRUE) => true,
                (TRUE, FALSE) => false,
                (FALSE, FALSE) => best_level < candidate_level,
                (TRUE, TRUE) => candidate_level > best_level,
                _ => false,
            };
            if better {
                best_pos = pos;
                current_value = candidate_value;
                best_level = candidate_level;
            }
        }

        if best_pos != start {
            lits.swap(start, best_pos);
        }
        current_value
    }

    pub(super) fn order_kissat_initial_watches(&self, lits: &mut [i32]) {
        if lits.len() < 2 {
            return;
        }
        let first_value = self.move_kissat_initial_watch_to_front(lits, 0, false);
        if lits.len() > 2 {
            self.move_kissat_initial_watch_to_front(lits, 1, first_value == TRUE);
        }
    }

    fn promote_initial_watches_preserving_sorted_tail(
        sorted: &mut [i32],
        first_watch: i32,
        second_watch: i32,
    ) {
        debug_assert!(sorted.len() >= 2);
        debug_assert_ne!(first_watch, second_watch);
        debug_assert!(sorted.contains(&first_watch));
        debug_assert!(sorted.contains(&second_watch));

        let mut write = sorted.len();
        for read in (0..sorted.len()).rev() {
            let lit = sorted[read];
            if lit == first_watch || lit == second_watch {
                continue;
            }
            write -= 1;
            sorted[write] = lit;
        }
        debug_assert_eq!(write, 2);
        sorted[0] = first_watch;
        sorted[1] = second_watch;
    }

    pub(super) fn normalize_original_clause_kissat_watch(
        &self,
        clause: &[i32],
    ) -> Option<Vec<i32>> {
        let mut normalized = self.normalize_original_clause(clause)?;
        if normalized.len() < 2 {
            return Some(normalized);
        }

        let mut watch_order = self.normalize_original_clause_input_order(clause)?;
        debug_assert_eq!(normalized.len(), watch_order.len());
        self.order_kissat_initial_watches(&mut watch_order);
        Self::promote_initial_watches_preserving_sorted_tail(
            &mut normalized,
            watch_order[0],
            watch_order[1],
        );
        Some(normalized)
    }

    fn add_normalized_original_clause(
        &mut self,
        normalized: &[i32],
        proof_log: &mut ProofLog,
        log_proof: bool,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
        bsr_touched: &mut Vec<usize>,
        bsr_touched_flags: &mut Vec<bool>,
        mut subsumption_work: Option<&mut VecDeque<SubsumptionCandidate>>,
    ) -> OriginalClauseInsertResult {
        if normalized.is_empty() {
            if log_proof {
                proof_log.record_clause(&[]);
            }
            self.solver_ok = false;
            self.has_empty_clause = true;
            return OriginalClauseInsertResult::Unsat;
        }

        let tr = elim_trace::enabled();
        if log_proof {
            let t = elim_trace::start(tr);
            proof_log.record_clause(normalized);
            elim_trace::add(&elim_trace::ADD_PROOF_NS, t);
        }

        if normalized.len() == 1 {
            if !self.enqueue(normalized[0], ReasonRef::None) || self.propagate().is_some() {
                self.solver_ok = false;
                return OriginalClauseInsertResult::Unsat;
            }
            return OriginalClauseInsertResult::Unit;
        }

        let t_arena = elim_trace::start(tr);
        let clause_idx = self.arena.len();
        let store_abstraction_inline = self.use_simplification && self.inline_original_abstractions;
        self.arena.push(clause_make_header(
            normalized.len(),
            false,
            store_abstraction_inline,
            0,
            false,
        ));
        self.arena
            .extend(normalized.iter().copied().map(lit_to_word));
        if store_abstraction_inline {
            let abstraction = clause_abstraction_from_lits(normalized);
            self.arena.push(abstraction as u32);
        }
        debug_assert!(clause_idx < u32::MAX as usize);
        self.original_clause_ids.push(clause_idx as u32);
        self.original_literals += normalized.len();
        elim_trace::add(&elim_trace::ADD_ARENA_NS, t_arena);
        let t_attach = elim_trace::start(tr);
        self.attach_clause(clause_idx, false);
        elim_trace::add(&elim_trace::ADD_ATTACH_NS, t_attach);

        if self.use_simplification {
            let t_index = elim_trace::start(tr);
            if !store_abstraction_inline && !self.clause_abstraction.is_empty() {
                self.set_original_clause_abstraction(
                    clause_idx,
                    clause_abstraction_from_lits(normalized),
                );
            }
            self.index_original_clause(clause_idx);
            for &lit in normalized {
                Self::touch_preprocess_var_and_bsr(
                    touched,
                    touched_flags,
                    bsr_touched,
                    bsr_touched_flags,
                    lit.unsigned_abs() as usize,
                );
            }
            elim_trace::add(&elim_trace::ADD_INDEX_NS, t_index);
        }

        if let Some(queue) = subsumption_work.as_mut() {
            let t_enq = elim_trace::start(tr);
            self.enqueue_subsumption_clause(queue, clause_idx);
            elim_trace::add(&elim_trace::ADD_ENQ_NS, t_enq);
        }

        OriginalClauseInsertResult::Allocated(clause_idx)
    }

    fn add_original_clause_from_slice(
        &mut self,
        clause: &[i32],
        proof_log: &mut ProofLog,
        log_proof: bool,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
        bsr_touched: &mut Vec<usize>,
        bsr_touched_flags: &mut Vec<bool>,
        subsumption_work: Option<&mut VecDeque<SubsumptionCandidate>>,
    ) -> OriginalClauseInsertResult {
        if !self.solver_ok {
            return OriginalClauseInsertResult::Unsat;
        }

        if !self.elim_scratch {
            // Legacy allocating path (SAT_ELIM_SCRATCH=off): fresh normalize Vec per clause.
            let Some(normalized) = self.normalize_original_clause(clause) else {
                return OriginalClauseInsertResult::Skipped;
            };
            return self.add_normalized_original_clause(
                &normalized,
                proof_log,
                log_proof,
                touched,
                touched_flags,
                bsr_touched,
                bsr_touched_flags,
                subsumption_work,
            );
        }

        let tr = elim_trace::enabled();
        let t_norm = elim_trace::start(tr);
        let mut normalized = std::mem::take(&mut self.norm_scratch);
        let keep = self.normalize_original_clause_into(clause, &mut normalized);
        elim_trace::add(&elim_trace::ADD_NORM_NS, t_norm);
        if !keep {
            self.norm_scratch = normalized;
            return OriginalClauseInsertResult::Skipped;
        }

        let result = self.add_normalized_original_clause(
            &normalized,
            proof_log,
            log_proof,
            touched,
            touched_flags,
            bsr_touched,
            bsr_touched_flags,
            subsumption_work,
        );
        self.norm_scratch = normalized;
        result
    }

    pub(super) fn add_initial_original_clauses(&mut self, clauses: Vec<Vec<i32>>, sort: bool) {
        let mut proof_log = ProofLog::disabled();
        let mut touched = Vec::new();
        let mut touched_flags = Vec::new();
        let mut bsr_touched = Vec::new();
        let mut bsr_touched_flags = Vec::new();
        let use_simplification = self.use_simplification;
        self.use_simplification = false;
        for clause in clauses {
            if !self.solver_ok {
                break;
            }
            let normalized = if sort {
                self.normalize_original_clause(&clause)
            } else {
                self.normalize_original_clause_input_order(&clause)
            };
            let Some(normalized) = normalized else {
                continue;
            };
            let _ = self.add_normalized_original_clause(
                &normalized,
                &mut proof_log,
                false,
                &mut touched,
                &mut touched_flags,
                &mut bsr_touched,
                &mut bsr_touched_flags,
                None,
            );
        }
        self.use_simplification = use_simplification;
    }

    pub(super) fn add_initial_original_clauses_kissat_watch(&mut self, clauses: Vec<Vec<i32>>) {
        let mut proof_log = ProofLog::disabled();
        let mut touched = Vec::new();
        let mut touched_flags = Vec::new();
        let mut bsr_touched = Vec::new();
        let mut bsr_touched_flags = Vec::new();
        let use_simplification = self.use_simplification;
        self.use_simplification = false;
        for clause in clauses {
            if !self.solver_ok {
                break;
            }
            let Some(normalized) = self.normalize_original_clause_kissat_watch(&clause) else {
                continue;
            };
            let _ = self.add_normalized_original_clause(
                &normalized,
                &mut proof_log,
                false,
                &mut touched,
                &mut touched_flags,
                &mut bsr_touched,
                &mut bsr_touched_flags,
                None,
            );
        }
        self.use_simplification = use_simplification;
    }

    #[inline(always)]
    fn subsumption_driver_len(&self, driver: SubsumptionCandidate) -> usize {
        match driver {
            SubsumptionCandidate::Clause(clause_idx) => self.clause_len(clause_idx),
            SubsumptionCandidate::RootUnit(_) => 1,
        }
    }

    #[inline(always)]
    fn subsumption_driver_lit(&self, driver: SubsumptionCandidate, pos: usize) -> i32 {
        match driver {
            SubsumptionCandidate::Clause(clause_idx) => self.clause_lit(clause_idx, pos),
            SubsumptionCandidate::RootUnit(lit) => {
                debug_assert_eq!(pos, 0);
                lit
            }
        }
    }

    #[inline(always)]
    fn subsumption_driver_abstraction(&self, driver: SubsumptionCandidate) -> u64 {
        match driver {
            SubsumptionCandidate::Clause(clause_idx) => {
                self.original_clause_abstraction(clause_idx)
            }
            SubsumptionCandidate::RootUnit(lit) => 1u64 << ((lit.unsigned_abs() - 1) & 31),
        }
    }

    fn backward_subsumption_check<const TRACE: bool>(
        &mut self,
        seed_all_clauses: bool,
        queue: &mut VecDeque<SubsumptionCandidate>,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
        bsr_touched: &mut Vec<usize>,
        bsr_touched_flags: &mut Vec<bool>,
        proof_log: &mut ProofLog,
    ) -> bool {
        // Persist the stamped relation-marks buffer across calls (SAT_ELIM_SCRATCH):
        // the legacy per-call fresh Vec re-zeroed 2*vars u32 (~5.8MB on vex-class) on
        // every BSR entry. Stamp semantics make persistence behavior-identical: only
        // equality with the current stamp is ever read, and the stamp keeps
        // monotonically increasing across calls (wraparound clears as before).
        let (mut relation_marks, mut relation_mark_stamp) = if self.elim_scratch {
            (
                std::mem::take(&mut self.bsr_relation_marks_scratch),
                self.bsr_relation_stamp,
            )
        } else {
            (Vec::new(), 0u32)
        };
        let result = self.backward_subsumption_check_body::<TRACE>(
            seed_all_clauses,
            queue,
            touched,
            touched_flags,
            bsr_touched,
            bsr_touched_flags,
            proof_log,
            &mut relation_marks,
            &mut relation_mark_stamp,
        );
        if self.elim_scratch {
            self.bsr_relation_marks_scratch = relation_marks;
            self.bsr_relation_stamp = relation_mark_stamp;
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn backward_subsumption_check_body<const TRACE: bool>(
        &mut self,
        seed_all_clauses: bool,
        queue: &mut VecDeque<SubsumptionCandidate>,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
        bsr_touched: &mut Vec<usize>,
        bsr_touched_flags: &mut Vec<bool>,
        proof_log: &mut ProofLog,
        relation_marks: &mut Vec<u32>,
        relation_mark_stamp: &mut u32,
    ) -> bool {
        if TRACE {
            self.stats.bsr_runs += 1;
        }
        if seed_all_clauses {
            let mut original_clause_ids = self.original_clause_ids.clone();
            original_clause_ids.sort_by_key(|&clause_idx| {
                let clause_idx = clause_idx as usize;
                if clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx) {
                    (self.clause_len(clause_idx), clause_idx)
                } else {
                    (usize::MAX, clause_idx)
                }
            });
            for clause_idx in original_clause_ids {
                self.enqueue_subsumption_clause(queue, clause_idx as usize);
                if TRACE {
                    self.stats.bsr_seeded_clauses += 1;
                }
            }
        }

        while !queue.is_empty() || self.bwdsub_assigns < self.trail.len() {
            let driver = if let Some(candidate) = queue.pop_front() {
                match candidate {
                    SubsumptionCandidate::Clause(clause_idx) => {
                        if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
                            continue;
                        }
                        self.clear_subsumption_clause_mark(clause_idx);
                    }
                    SubsumptionCandidate::RootUnit(_) => {}
                }
                candidate
            } else {
                let lit = self.trail[self.bwdsub_assigns];
                self.bwdsub_assigns += 1;
                SubsumptionCandidate::RootUnit(lit)
            };

            let driver_len = self.subsumption_driver_len(driver);
            if driver_len == 0 {
                continue;
            }
            if TRACE {
                self.stats.bsr_drivers += 1;
                self.stats.bsr_driver_lits += driver_len as u64;
                match driver {
                    SubsumptionCandidate::Clause(_) => self.stats.bsr_clause_drivers += 1,
                    SubsumptionCandidate::RootUnit(_) => self.stats.bsr_root_drivers += 1,
                }
            }
            let driver_abstraction = self.subsumption_driver_abstraction(driver);

            let mut best_var = 0usize;
            let mut best_live_occurs = usize::MAX;
            for driver_pos in 0..driver_len {
                let var = self
                    .subsumption_driver_lit(driver, driver_pos)
                    .unsigned_abs() as usize;
                if var == 0 || var >= self.occurs.len() {
                    continue;
                }
                let live_occurs = self.live_occurrence_count(var);
                if live_occurs < best_live_occurs {
                    best_var = var;
                    best_live_occurs = live_occurs;
                }
            }

            if best_var == 0 {
                continue;
            }
            self.clean_occurs::<TRACE>(best_var);
            let best_occurs_len = self.occurs[best_var].len() as u64;
            if TRACE {
                self.stats.bsr_best_occurs_sum += best_occurs_len;
                self.stats.bsr_best_occurs_max =
                    self.stats.bsr_best_occurs_max.max(best_occurs_len);
            }
            if self.bsr_occurrence_limit != 0 && best_occurs_len > self.bsr_occurrence_limit {
                if TRACE {
                    self.stats.bsr_skip_occurrence_limit += 1;
                }
                continue;
            }

            let mut scan_pos = 0usize;
            while scan_pos < self.occurs[best_var].len() {
                if self.eliminate_ticks_budget != 0 && !self.consume_bsr_tick() {
                    self.clear_subsumption_queue_marks(queue);
                    return true;
                }
                let candidate_idx = self.occurs[best_var][scan_pos] as usize;
                scan_pos += 1;
                if TRACE {
                    self.stats.bsr_candidates_seen += 1;
                }
                if driver == SubsumptionCandidate::Clause(candidate_idx) {
                    if TRACE {
                        self.stats.bsr_skip_self += 1;
                    }
                    continue;
                }
                if candidate_idx >= self.arena.len() || self.clause_is_deleted(candidate_idx) {
                    if TRACE {
                        self.stats.bsr_skip_deleted += 1;
                    }
                    continue;
                }
                if self.subsumption_lim >= 0
                    && self.clause_len(candidate_idx) as isize >= self.subsumption_lim
                {
                    if TRACE {
                        self.stats.bsr_skip_limit += 1;
                    }
                    continue;
                }

                match self.subsumption_relation::<TRACE>(
                    driver,
                    driver_len,
                    driver_abstraction,
                    candidate_idx,
                    &mut *relation_marks,
                    &mut *relation_mark_stamp,
                ) {
                    SubsumptionOutcome::None => {}
                    SubsumptionOutcome::Subsumed => {
                        proof_log.record_deletion(self.clause_slice(candidate_idx));
                        self.remove_original_clause_preprocess(
                            candidate_idx,
                            touched,
                            touched_flags,
                        );
                        self.stats.preprocess_subsumed_clauses += 1;
                    }
                    SubsumptionOutcome::Strengthen(remove_lit) => {
                        if !self.strengthen_original_clause_preprocess(
                            candidate_idx,
                            remove_lit,
                            proof_log,
                            touched,
                            touched_flags,
                            bsr_touched,
                            bsr_touched_flags,
                            queue,
                        ) {
                            self.clear_subsumption_queue_marks(queue);
                            return false;
                        }
                        if remove_lit.unsigned_abs() as usize == best_var {
                            scan_pos = scan_pos.saturating_sub(1);
                        }
                    }
                }
            }
        }

        true
    }

    fn backward_subsumption_check_dynamic(
        &mut self,
        seed_all_clauses: bool,
        queue: &mut VecDeque<SubsumptionCandidate>,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
        bsr_touched: &mut Vec<usize>,
        bsr_touched_flags: &mut Vec<bool>,
        proof_log: &mut ProofLog,
    ) -> bool {
        if self.trace_preprocess_details {
            self.backward_subsumption_check::<true>(
                seed_all_clauses,
                queue,
                touched,
                touched_flags,
                bsr_touched,
                bsr_touched_flags,
                proof_log,
            )
        } else {
            self.backward_subsumption_check::<false>(
                seed_all_clauses,
                queue,
                touched,
                touched_flags,
                bsr_touched,
                bsr_touched_flags,
                proof_log,
            )
        }
    }

    fn append_resolvent_into_vec(
        &self,
        lhs_idx: usize,
        rhs_idx: usize,
        var: usize,
        out: &mut Vec<i32>,
    ) -> bool {
        let start = out.len();
        if self.clauses_sorted_by_var {
            if self.append_sorted_resolvent_into_vec(lhs_idx, rhs_idx, var, out) {
                true
            } else {
                out.truncate(start);
                false
            }
        } else {
            if self.append_nested_resolvent_into_vec(lhs_idx, rhs_idx, var, out) {
                true
            } else {
                out.truncate(start);
                false
            }
        }
    }

    fn append_sorted_resolvent_into_vec(
        &self,
        lhs_idx: usize,
        rhs_idx: usize,
        var: usize,
        out: &mut Vec<i32>,
    ) -> bool {
        let lhs_len = self.clause_len(lhs_idx);
        let rhs_len = self.clause_len(rhs_idx);
        let mut lhs_pos = 0usize;
        let mut rhs_pos = 0usize;

        loop {
            while lhs_pos < lhs_len {
                let lit = self.clause_lit(lhs_idx, lhs_pos);
                if lit.unsigned_abs() as usize != var {
                    break;
                }
                lhs_pos += 1;
            }
            while rhs_pos < rhs_len {
                let lit = self.clause_lit(rhs_idx, rhs_pos);
                if lit.unsigned_abs() as usize != var {
                    break;
                }
                rhs_pos += 1;
            }

            match (lhs_pos < lhs_len, rhs_pos < rhs_len) {
                (false, false) => return true,
                (true, false) => {
                    out.push(self.clause_lit(lhs_idx, lhs_pos));
                    lhs_pos += 1;
                }
                (false, true) => {
                    out.push(self.clause_lit(rhs_idx, rhs_pos));
                    rhs_pos += 1;
                }
                (true, true) => {
                    let lhs_lit = self.clause_lit(lhs_idx, lhs_pos);
                    let rhs_lit = self.clause_lit(rhs_idx, rhs_pos);
                    match lhs_lit.unsigned_abs().cmp(&rhs_lit.unsigned_abs()) {
                        std::cmp::Ordering::Less => {
                            out.push(lhs_lit);
                            lhs_pos += 1;
                        }
                        std::cmp::Ordering::Greater => {
                            out.push(rhs_lit);
                            rhs_pos += 1;
                        }
                        std::cmp::Ordering::Equal => {
                            if lhs_lit == -rhs_lit {
                                return false;
                            }
                            out.push(lhs_lit);
                            lhs_pos += 1;
                            rhs_pos += 1;
                        }
                    }
                }
            }
        }
    }

    fn append_nested_resolvent_into_vec(
        &self,
        lhs_idx: usize,
        rhs_idx: usize,
        var: usize,
        out: &mut Vec<i32>,
    ) -> bool {
        let resolvent_start = out.len();
        let lhs_len = self.clause_len(lhs_idx);
        let rhs_len = self.clause_len(rhs_idx);

        for lit_pos in 0..lhs_len {
            let lit = self.clause_lit(lhs_idx, lit_pos);
            if lit.unsigned_abs() as usize != var {
                out.push(lit);
            }
        }

        'rhs_lits: for rhs_pos in 0..rhs_len {
            let lit = self.clause_lit(rhs_idx, rhs_pos);
            if lit.unsigned_abs() as usize == var {
                continue;
            }
            for &existing in &out[resolvent_start..] {
                if existing.unsigned_abs() == lit.unsigned_abs() {
                    if existing == -lit {
                        return false;
                    }
                    continue 'rhs_lits;
                }
            }
            out.push(lit);
        }

        true
    }

    fn push_elim_unit(&mut self, lit: i32) {
        self.elim_clauses.push(lit);
        self.elim_clauses.push(1);
    }

    fn push_elim_clause(&mut self, var: usize, clause_idx: usize) {
        let start = self.elim_clauses.len();
        let mut var_pos = None;
        let clause_len = self.clause_len(clause_idx);
        for lit_pos in 0..clause_len {
            let lit = self.clause_lit(clause_idx, lit_pos);
            if lit.unsigned_abs() as usize == var {
                var_pos = Some(self.elim_clauses.len());
            }
            self.elim_clauses.push(lit);
        }

        let var_pos = var_pos.expect("elimination extension clause missing eliminated variable");
        self.elim_clauses.swap(start, var_pos);
        self.elim_clauses.push(clause_len as i32);
    }

    /// Resolve one (pos, neg) clause pair for variable elimination, appending the
    /// resolvent (if not tautological) to `resolvent_lits`/`resolvent_ranges` and
    /// charging the resolution budget. `pos_idx` must contain `+var`, `neg_idx` `-var`.
    /// Returns `false` if the elimination must be rejected (budget exhausted, the
    /// resolvent count exceeded `occurrence_count + bve_grow`, or a resolvent exceeded
    /// `bve_clause_limit`); on rejection any partial resolvent is truncated. Tautological
    /// resolvents are skipped and return `true`. This is the shared body of both the naive
    /// all-pairs loop and the gate-restricted loop, so they apply identical accounting.
    fn resolve_elim_pair(
        &mut self,
        pos_idx: usize,
        neg_idx: usize,
        var: usize,
        occurrence_count: usize,
        resolvent_count: &mut isize,
        resolvent_lits: &mut Vec<i32>,
        resolvent_ranges: &mut Vec<(usize, usize)>,
    ) -> bool {
        self.resolve_elim_pair_capped(
            pos_idx,
            neg_idx,
            var,
            occurrence_count,
            resolvent_count,
            resolvent_lits,
            resolvent_ranges,
            None,
        )
    }

    /// `resolve_elim_pair` with an optional per-resolvent length cap on top of
    /// `bve_clause_limit`. Definition-gate eliminations pass the longer parent's
    /// length: a semantic-core elimination may never produce a resolvent longer
    /// than the clauses it replaces, because densifying resolvents compound —
    /// measured on oski40, unrestricted definition eliminations doubled the live
    /// arena, tripled learned literals, and cost 5.6x search ticks (+700s wall)
    /// for a 14% conflict reduction.
    #[allow(clippy::too_many_arguments)]
    fn resolve_elim_pair_capped(
        &mut self,
        pos_idx: usize,
        neg_idx: usize,
        var: usize,
        occurrence_count: usize,
        resolvent_count: &mut isize,
        resolvent_lits: &mut Vec<i32>,
        resolvent_ranges: &mut Vec<(usize, usize)>,
        max_len: Option<usize>,
    ) -> bool {
        if (self.eliminate_resolution_budget != 0 || self.eliminate_ticks_budget != 0)
            && !self.consume_eliminate_resolution_attempt()
        {
            self.elim_reject_budget += 1;
            return false;
        }
        let start = resolvent_lits.len();
        if !self.append_resolvent_into_vec(pos_idx, neg_idx, var, resolvent_lits) {
            return true;
        }
        *resolvent_count += 1;
        if *resolvent_count > occurrence_count as isize + self.bve_grow {
            self.elim_reject_count_bound += 1;
            resolvent_lits.truncate(start);
            return false;
        }
        let size = resolvent_lits.len() - start;
        if self.bve_clause_limit >= 0 && size as isize > self.bve_clause_limit {
            self.elim_reject_clslim += 1;
            resolvent_lits.truncate(start);
            return false;
        }
        if let Some(cap) = max_len {
            if size > cap {
                self.elim_reject_defcap += 1;
                resolvent_lits.truncate(start);
                return false;
            }
        }
        resolvent_ranges.push((start, size));
        true
    }

    /// Detect whether `var` is functionally defined by an AND/OR gate over its clauses
    /// (`pos_clauses` contain `+var`, `neg_clauses` contain `-var`). Returns a
    /// `GatePartition` splitting each side into gate vs non-gate clauses, or `None` if no
    /// gate is found. Tries the gate with `+var` as the defined side, then `-var` (the
    /// AND/OR dual). Sound-by-construction: if any clause involved contains a root-assigned
    /// literal, returns `None` and the caller falls back to naive all-pairs BVE (a stale
    /// root-satisfied clause could otherwise yield a spurious gate and an unsound skip).
    fn detect_and_or_gate(
        &mut self,
        var: usize,
        pos_clauses: &[usize],
        neg_clauses: &[usize],
    ) -> Option<GatePartition> {
        for &ci in pos_clauses.iter().chain(neg_clauses.iter()) {
            let len = self.clause_len(ci);
            for k in 0..len {
                let v = self.clause_lit(ci, k).unsigned_abs() as usize;
                if v < self.assignment.len() && self.assignment[v] != UNASSIGNED {
                    return None;
                }
            }
        }
        let mut marks = std::mem::take(&mut self.gate_marks);
        let lit_slots = self.variable_count().saturating_mul(2);
        if marks.len() < lit_slots {
            marks.resize(lit_slots, 0);
        }
        let result = self
            .detect_and_or_gate_side(var, pos_clauses, neg_clauses, true, &mut marks)
            .or_else(|| self.detect_and_or_gate_side(var, pos_clauses, neg_clauses, false, &mut marks));
        self.gate_marks = marks;
        result
    }

    /// One side of AND/OR gate detection: tries to read `var` (when `l_is_pos`) or `-var`
    /// as the literal `L` defined by `L <-> OR(o1..ok)`, encoded as the base clause
    /// `(¬L ∨ o1 ∨ .. ∨ ok)` (on the opposite side) plus binaries `(L ∨ ¬oi)` (on L's side).
    fn detect_and_or_gate_side(
        &mut self,
        var: usize,
        pos_clauses: &[usize],
        neg_clauses: &[usize],
        l_is_pos: bool,
        marks: &mut [u32],
    ) -> Option<GatePartition> {
        let (l_clauses, base_clauses): (&[usize], &[usize]) = if l_is_pos {
            (pos_clauses, neg_clauses)
        } else {
            (neg_clauses, pos_clauses)
        };
        let l = if l_is_pos { var as i32 } else { -(var as i32) };
        let not_l = -l;

        // Step 1: mark the partner literal of every binary (L ∨ other) on L's side.
        self.gate_mark_stamp = self.gate_mark_stamp.wrapping_add(1);
        if self.gate_mark_stamp == 0 {
            for m in marks.iter_mut() {
                *m = 0;
            }
            self.gate_mark_stamp = 1;
        }
        let stamp = self.gate_mark_stamp;
        let mut marked = 0usize;
        for &ci in l_clauses {
            if self.clause_len(ci) != 2 {
                continue;
            }
            let a = self.clause_lit(ci, 0);
            let b = self.clause_lit(ci, 1);
            let other = if a == l {
                b
            } else if b == l {
                a
            } else {
                continue;
            };
            let idx = lit_to_index(other);
            if marks[idx] != stamp {
                marks[idx] = stamp;
                marked += 1;
            }
        }
        if marked < 2 {
            return None;
        }

        // Step 2: find a base clause (¬L ∨ o1 ∨ .. ∨ ok), k >= 2, on the opposite side such
        // that every binary (L ∨ ¬oi) exists (i.e. marks[idx(¬oi)] is set).
        for &base in base_clauses {
            let blen = self.clause_len(base);
            if blen < 3 {
                continue;
            }
            let mut all_partners_marked = true;
            for k in 0..blen {
                let o = self.clause_lit(base, k);
                if o == not_l {
                    continue;
                }
                if marks[lit_to_index(-o)] != stamp {
                    all_partners_marked = false;
                    break;
                }
            }
            if !all_partners_marked {
                continue;
            }

            // Step 3: partition. A binary (L ∨ x) is a gate clause iff x is a base partner
            // (x == ¬oi for some base literal oi != ¬L). Re-stamp the partner set, then split.
            self.gate_mark_stamp = self.gate_mark_stamp.wrapping_add(1);
            if self.gate_mark_stamp == 0 {
                for m in marks.iter_mut() {
                    *m = 0;
                }
                self.gate_mark_stamp = 1;
            }
            let pstamp = self.gate_mark_stamp;
            for k in 0..blen {
                let o = self.clause_lit(base, k);
                if o == not_l {
                    continue;
                }
                marks[lit_to_index(-o)] = pstamp;
            }
            let mut gate_l = Vec::new();
            let mut nongate_l = Vec::new();
            for &ci in l_clauses {
                let mut is_gate = false;
                if self.clause_len(ci) == 2 {
                    let a = self.clause_lit(ci, 0);
                    let b = self.clause_lit(ci, 1);
                    let other = if a == l { b } else { a };
                    if marks[lit_to_index(other)] == pstamp {
                        is_gate = true;
                    }
                }
                if is_gate {
                    gate_l.push(ci);
                } else {
                    nongate_l.push(ci);
                }
            }
            let gate_base = vec![base];
            let nongate_base: Vec<usize> =
                base_clauses.iter().copied().filter(|&c| c != base).collect();

            return Some(if l_is_pos {
                GatePartition {
                    gate_pos: gate_l,
                    gate_neg: gate_base,
                    nongate_pos: nongate_l,
                    nongate_neg: nongate_base,
                    kind: ElimGateKind::AndOr,
                }
            } else {
                GatePartition {
                    gate_pos: gate_base,
                    gate_neg: gate_l,
                    nongate_pos: nongate_base,
                    nongate_neg: nongate_l,
                    kind: ElimGateKind::AndOr,
                }
            });
        }
        None
    }

    /// Shared safety check for elimination gate detection: every clause of the pivot
    /// must be free of root-assigned literals, otherwise a stale root-satisfied clause
    /// could yield a spurious gate definition and an unsound resolvent skip.
    fn elim_gate_clauses_clean(&self, pos_clauses: &[usize], neg_clauses: &[usize]) -> bool {
        for &ci in pos_clauses.iter().chain(neg_clauses.iter()) {
            let len = self.clause_len(ci);
            for k in 0..len {
                let v = self.clause_lit(ci, k).unsigned_abs() as usize;
                if v < self.assignment.len() && self.assignment[v] != UNASSIGNED {
                    return false;
                }
            }
        }
        true
    }

    /// Detect an equivalence definition `var ≡ r` from two exact binary clauses
    /// `(v ∨ ¬r)` and `(¬v ∨ r)` (kissat equivalences.c). Eliminating an
    /// equivalence-defined pivot resolves each side's non-gate clauses against the
    /// single opposite gate binary — i.e. substitution by resolution — producing
    /// `occurrences − 2` resolvents, always within the acceptance bound. The
    /// gate-vs-gate resolvent `(¬r ∨ r)` is a tautology, so omitting it is sound.
    fn detect_equivalence_gate(
        &mut self,
        var: usize,
        pos_clauses: &[usize],
        neg_clauses: &[usize],
    ) -> Option<GatePartition> {
        if !self.elim_gate_clauses_clean(pos_clauses, neg_clauses) {
            return None;
        }
        let l = var as i32;
        let mut marks = std::mem::take(&mut self.gate_marks);
        let lit_slots = self.variable_count().saturating_mul(2);
        if marks.len() < lit_slots {
            marks.resize(lit_slots, 0);
        }
        self.gate_mark_stamp = self.gate_mark_stamp.wrapping_add(1);
        if self.gate_mark_stamp == 0 {
            for m in marks.iter_mut() {
                *m = 0;
            }
            self.gate_mark_stamp = 1;
        }
        let stamp = self.gate_mark_stamp;
        let mut any = false;
        for &ci in pos_clauses {
            if self.clause_len(ci) != 2 {
                continue;
            }
            let a = self.clause_lit(ci, 0);
            let b = self.clause_lit(ci, 1);
            let other = if a == l {
                b
            } else if b == l {
                a
            } else {
                continue;
            };
            marks[lit_to_index(other)] = stamp;
            any = true;
        }
        let mut found: Option<(usize, i32)> = None;
        if any {
            for &cj in neg_clauses {
                if self.clause_len(cj) != 2 {
                    continue;
                }
                let a = self.clause_lit(cj, 0);
                let b = self.clause_lit(cj, 1);
                let other = if a == -l {
                    b
                } else if b == -l {
                    a
                } else {
                    continue;
                };
                if other.unsigned_abs() as usize == var {
                    continue;
                }
                if marks[lit_to_index(-other)] == stamp {
                    found = Some((cj, other));
                    break;
                }
            }
        }
        self.gate_marks = marks;
        let (neg_gate_ci, r) = found?;
        let mut pos_gate_ci = None;
        for &ci in pos_clauses {
            if self.clause_len(ci) != 2 {
                continue;
            }
            let a = self.clause_lit(ci, 0);
            let b = self.clause_lit(ci, 1);
            let other = if a == l {
                b
            } else if b == l {
                a
            } else {
                continue;
            };
            if other == -r {
                pos_gate_ci = Some(ci);
                break;
            }
        }
        let pos_gate_ci = pos_gate_ci?;
        Some(GatePartition {
            gate_pos: vec![pos_gate_ci],
            gate_neg: vec![neg_gate_ci],
            nongate_pos: pos_clauses
                .iter()
                .copied()
                .filter(|&c| c != pos_gate_ci)
                .collect(),
            nongate_neg: neg_clauses
                .iter()
                .copied()
                .filter(|&c| c != neg_gate_ci)
                .collect(),
            kind: ElimGateKind::Equivalence,
        })
    }

    /// Detect an if-then-else definition `var = ITE(c, t, e)` from its four exact
    /// ternary Tseitin clauses (kissat ifthenelse.c, exact-shape case):
    /// `(v ∨ ¬c ∨ ¬t)`, `(v ∨ c ∨ ¬e)` on the positive side and `(¬v ∨ ¬c ∨ t)`,
    /// `(¬v ∨ c ∨ e)` on the negative side. All four gate-vs-gate resolvents on the
    /// pivot are tautologies (each pair shares `c` in opposite polarity or resolves a
    /// branch literal against itself), so restricting resolution to gate-vs-nongate
    /// pairs is the standard substitution-by-definition argument. The four variables
    /// `v, c, t, e` must be pairwise distinct (degenerate shapes are skipped —
    /// conservative and sound).
    fn detect_ite_gate(
        &mut self,
        var: usize,
        pos_clauses: &[usize],
        neg_clauses: &[usize],
    ) -> Option<GatePartition> {
        const ITE_MAX_TERNARIES: usize = 64;
        const ITE_MAX_PAIRS: usize = 2048;
        if !self.elim_gate_clauses_clean(pos_clauses, neg_clauses) {
            return None;
        }
        let l = var as i32;
        let collect_ternaries = |s: &Self, lit: i32, clauses: &[usize]| -> Option<Vec<(usize, [i32; 2])>> {
            let mut out: Vec<(usize, [i32; 2])> = Vec::new();
            for &ci in clauses {
                if s.clause_len(ci) != 3 {
                    continue;
                }
                let mut others = [0i32; 2];
                let mut n = 0usize;
                let mut has_lit = false;
                for k in 0..3 {
                    let cl = s.clause_lit(ci, k);
                    if cl == lit {
                        has_lit = true;
                    } else if n < 2 {
                        others[n] = cl;
                        n += 1;
                    }
                }
                if !has_lit || n != 2 {
                    continue;
                }
                out.push((ci, others));
                if out.len() > ITE_MAX_TERNARIES {
                    return None;
                }
            }
            Some(out)
        };
        let pos_tern = collect_ternaries(self, l, pos_clauses)?;
        if pos_tern.len() < 2 {
            return None;
        }
        let neg_tern_list = collect_ternaries(self, -l, neg_clauses)?;
        if neg_tern_list.len() < 2 {
            return None;
        }
        let sorted_pair = |a: i32, b: i32| if a <= b { (a, b) } else { (b, a) };
        let mut neg_tern: HashMap<(i32, i32), usize> = HashMap::new();
        for &(cj, o) in &neg_tern_list {
            neg_tern.entry(sorted_pair(o[0], o[1])).or_insert(cj);
        }
        let vv = var as u32;
        let mut pairs = 0usize;
        for i in 0..pos_tern.len() {
            for j in (i + 1)..pos_tern.len() {
                pairs += 1;
                if pairs > ITE_MAX_PAIRS {
                    return None;
                }
                let (ci1, o1) = pos_tern[i];
                let (ci2, o2) = pos_tern[j];
                for a in 0..2 {
                    for b in 0..2 {
                        // Candidate roles: clause i = (v ∨ x1 ∨ nt) as (v ∨ ¬c ∨ ¬t),
                        // clause j = (v ∨ x2 ∨ ne) as (v ∨ c ∨ ¬e), i.e. c = ¬x1 = x2.
                        let x1 = o1[a];
                        let nt = o1[1 - a];
                        let x2 = o2[b];
                        let ne = o2[1 - b];
                        if x1 != -x2 {
                            continue;
                        }
                        let cv = x1.unsigned_abs();
                        let tv = nt.unsigned_abs();
                        let ev = ne.unsigned_abs();
                        if cv == tv || cv == ev || tv == ev || cv == vv || tv == vv || ev == vv {
                            continue;
                        }
                        let Some(&c3) = neg_tern.get(&sorted_pair(x1, -nt)) else {
                            continue;
                        };
                        let Some(&c4) = neg_tern.get(&sorted_pair(x2, -ne)) else {
                            continue;
                        };
                        if c3 == c4 {
                            continue;
                        }
                        let gate_pos = vec![ci1, ci2];
                        let gate_neg = vec![c3, c4];
                        let nongate_pos: Vec<usize> = pos_clauses
                            .iter()
                            .copied()
                            .filter(|&c| c != ci1 && c != ci2)
                            .collect();
                        let nongate_neg: Vec<usize> = neg_clauses
                            .iter()
                            .copied()
                            .filter(|&c| c != c3 && c != c4)
                            .collect();
                        return Some(GatePartition {
                            gate_pos,
                            gate_neg,
                            nongate_pos,
                            nongate_neg,
                            kind: ElimGateKind::Ite,
                        });
                    }
                }
            }
        }
        None
    }

    /// kissat definition.c parity: semantic definition extraction via the kitten
    /// sub-solver, the LAST fallback after the syntactic detectors (kissat gates.c
    /// order: equivalence → AND/OR → ITE → definition). Export every occurrence of
    /// the pivot with the pivot literal removed; if that pivot-free environment is
    /// UNSAT, the pivot is functionally defined by its neighbor variables, and the
    /// refutation's clausal core selects the defining clause subset (the "gate").
    /// This finds definitions the syntactic detectors cannot: XOR chains, majority/
    /// threshold shapes, and irregular multi-clause encodings left behind by
    /// strengthening — the dominant definition shapes on arithmetic-circuit (booth/
    /// Bubble) and BMC formulas where our congruence closure finds zero merges.
    ///
    /// Soundness: every emitted resolvent is an ordinary RUP resolvent of two live
    /// clauses, so the DRAT stream needs no extra lemmas; omitting the
    /// nongate-vs-nongate resolvents is the standard substitution-by-definition
    /// argument (Eén–Biere), which for a semantic core additionally requires the
    /// gate-vs-gate resolvents — kissat sets `resolve_gate` for definition cores
    /// (resolve.c:340-345) and `try_eliminate_var` mirrors that via
    /// `ElimGateKind::Definition`. One-sided cores (kissat's failed-literal bonus)
    /// are skipped conservatively: a unit clause derived from a one-sided core is
    /// not necessarily RUP, and the elimination win does not need it.
    ///
    /// Budget: `SAT_ELIM_DEF_TICKS` kitten ticks per check (kissat
    /// `definitionticks`, default 1e6); consumed ticks are charged against the
    /// armed-BVE eliminate tick budget so definition probing cannot extend a round
    /// past its existing wall budget.
    /// Charge kitten definition-check work to the definition stats and (mirroring
    /// `consume_eliminate_tick`) to the eliminate tick budget so definition probing
    /// and core refinement can never extend an armed round past its existing bound.
    fn charge_def_kitten_ticks(&mut self, ticks: u64) {
        self.stats.preprocess_def_gate_ticks =
            self.stats.preprocess_def_gate_ticks.saturating_add(ticks);
        if self.eliminate_ticks_budget != 0 {
            self.stats.preprocess_eliminate_ticks = self
                .stats
                .preprocess_eliminate_ticks
                .saturating_add(ticks);
            if self.stats.preprocess_eliminate_ticks >= self.eliminate_ticks_budget {
                self.note_preprocess_budget_hit(PreprocessBudgetKind::Tick);
            }
        }
    }

    fn detect_kitten_definition(
        &mut self,
        var: usize,
        pos_clauses: &[usize],
        neg_clauses: &[usize],
    ) -> Option<GatePartition> {
        // The environment export is linear in clauses+literals; these caps bound the
        // per-candidate setup cost on pathological occurrence lists (the tick budget
        // only bounds the solve). Generous vs kissat's typical envs.
        // Tight caps: a definition only converts to an elimination when its core's
        // gate-restricted resolvents fit the occurrence bound, which never happens
        // for cores beyond a few dozen clauses — and non-definable (SAT) envs burn
        // the full tick budget. Circuit gate definitions (XOR/adder/mux) live in
        // envs of a handful of clauses. Measured on oski40: env 1024/lits 8192 +
        // 1M ticks cost ~700s of kitten wall for 1.2k conversions.
        const ELIM_DEF_MAX_ENV_CLAUSES: usize = 64;
        const ELIM_DEF_MAX_ENV_LITS: usize = 512;
        // Formula-adaptive cutoff: if tens of thousands of checks never found a
        // single definition, this formula's pivots are not kitten-definable
        // (measured: Timetable_492 1.7M checks / 0 found, lockchart-group1 5.0M / 0
        // — pure wall burn inside armed rounds), so stop probing it. Cells where
        // definitions exist find them almost immediately (found/checks 60-99% on
        // oski/ibm/booth/Bubble screens).
        const ELIM_DEF_PROBE_CHECKS: u64 = 20_000;
        if self.stats.preprocess_def_gate_checks >= ELIM_DEF_PROBE_CHECKS
            && self.stats.preprocess_def_gate_found == 0
        {
            return None;
        }
        if pos_clauses.is_empty() || neg_clauses.is_empty() {
            return None;
        }
        let total = pos_clauses.len() + neg_clauses.len();
        if total > ELIM_DEF_MAX_ENV_CLAUSES {
            return None;
        }
        // Per-variable re-check memo (see the field doc): only kitten-solve a pivot
        // again when its occurrence counts or the armed growth bound moved since the
        // last non-eliminating check, and give up on a pivot whose found definition
        // was bound-rejected ELIM_DEF_MAX_FAILS times at the current bound (amnesty
        // when the armed bound escalates 0 -> 1 -> ... -> 16).
        const ELIM_DEF_MAX_FAILS: u8 = 2;
        let pos_n = self.n_occ[lit_to_index(var as i32)];
        let neg_n = self.n_occ[lit_to_index(-(var as i32))];
        let bound = self.bve_grow as i32;
        if self.elim_def_last_probe.len() <= var {
            self.elim_def_last_probe
                .resize(var + 1, (u32::MAX, u32::MAX, 0, 0));
        }
        let entry = self.elim_def_last_probe[var];
        let fails = if entry.2 == bound { entry.3 } else { 0 };
        if fails >= ELIM_DEF_MAX_FAILS {
            return None;
        }
        if (entry.0, entry.1, entry.2) == (pos_n, neg_n, bound) {
            return None;
        }
        self.elim_def_last_probe[var] = (pos_n, neg_n, bound, fails);
        if !self.elim_gate_clauses_clean(pos_clauses, neg_clauses) {
            return None;
        }
        self.stats.preprocess_def_gate_checks += 1;

        // Map outer variables to dense kitten DIMACS variables and export each
        // occurrence with the pivot literal removed. Outer clauses are canonical
        // (no duplicate variables), so the pivot-free export can never be a
        // tautology or contain duplicates — kitten input indices stay positional.
        let mut var_map: HashMap<usize, i32> = HashMap::new();
        let mut next_kitten_var = 1i32;
        let mut kitten = Kitten::new();
        let mut buf: Vec<i32> = Vec::new();
        let mut total_lits = 0usize;
        // Pivot-free exports kept for core-refinement re-solves (only saved when
        // refinement is enabled).
        let mut exports: Vec<Vec<i32>> = Vec::new();
        for &ci in pos_clauses.iter().chain(neg_clauses.iter()) {
            let len = self.clause_len(ci);
            if len <= 1 {
                return None; // unit pivot clause: root-assignment race, bail
            }
            total_lits += len - 1;
            if total_lits > ELIM_DEF_MAX_ENV_LITS {
                return None;
            }
            buf.clear();
            for k in 0..len {
                let lit = self.clause_lit(ci, k);
                let v = lit.unsigned_abs() as usize;
                if v == var {
                    continue;
                }
                let kv = *var_map.entry(v).or_insert_with(|| {
                    let kv = next_kitten_var;
                    next_kitten_var += 1;
                    kv
                });
                buf.push(if lit > 0 { kv } else { -kv });
            }
            if buf.is_empty() {
                return None; // defensive: pivot-only clause
            }
            kitten.add_clause(&buf);
            if self.elim_def_cores > 1 {
                exports.push(buf.clone());
            }
        }
        debug_assert_eq!(kitten.num_input_clauses(), total);

        let result = kitten.solve_budgeted(&[], self.elim_def_ticks);
        self.charge_def_kitten_ticks(kitten.ticks());
        if result != Some(KittenResult::Unsat) {
            return None; // SAT (no definition) or budget exhausted
        }

        // kissat definition.c `definitioncores` parity: refine the core before
        // converting it into a gate. Each extra round re-exports ONLY the current
        // core clauses into a fresh kitten with shuffled variable numbering
        // (decision order) and clause order (watch order), re-solves at 10x the
        // check budget, and keeps the recomputed (weakly smaller) core. A budget
        // exhaustion during refinement drops the definition entirely (kissat
        // ABORT parity), keeping the work calibrated.
        let mut core_members: Vec<usize> = kitten
            .core()
            .iter()
            .copied()
            .filter(|&idx| idx < total)
            .collect();
        core_members.sort_unstable();
        core_members.dedup();
        let num_kitten_vars = (next_kitten_var - 1) as usize;
        let mut rng_state =
            (var as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
        // Deterministic splitmix64 stream: shuffles must not depend on host state.
        let mut next_rand = move || -> u64 {
            rng_state = rng_state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = rng_state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        for _round in 2..=self.elim_def_cores {
            // A two-clause core (one per side) is already minimal for a
            // two-sided definition; skip the re-solve.
            if core_members.len() <= 2 {
                break;
            }
            let mut var_perm: Vec<i32> = (1..=num_kitten_vars as i32).collect();
            for i in (1..var_perm.len()).rev() {
                let j = (next_rand() % (i as u64 + 1)) as usize;
                var_perm.swap(i, j);
            }
            let mut order: Vec<usize> = (0..core_members.len()).collect();
            for i in (1..order.len()).rev() {
                let j = (next_rand() % (i as u64 + 1)) as usize;
                order.swap(i, j);
            }
            let mut refine = Kitten::new();
            let mut shuffled: Vec<i32> = Vec::new();
            for &pos in &order {
                let member = core_members[pos];
                shuffled.clear();
                for &lit in &exports[member] {
                    let mapped = var_perm[(lit.unsigned_abs() - 1) as usize];
                    shuffled.push(if lit > 0 { mapped } else { -mapped });
                }
                refine.add_clause(&shuffled);
            }
            self.stats.preprocess_def_refine_solves += 1;
            let refined = refine.solve_budgeted(&[], self.elim_def_ticks.saturating_mul(10));
            self.charge_def_kitten_ticks(refine.ticks());
            match refined {
                Some(KittenResult::Unsat) => {
                    let mut new_members: Vec<usize> = refine
                        .core()
                        .iter()
                        .filter(|&&p| p < order.len())
                        .map(|&p| core_members[order[p]])
                        .collect();
                    new_members.sort_unstable();
                    new_members.dedup();
                    if new_members.len() < core_members.len() {
                        self.stats.preprocess_def_refine_shrunk += 1;
                    }
                    core_members = new_members;
                }
                Some(KittenResult::Sat) => {
                    // A subset of a refutation support cannot be SAT; a SAT answer
                    // means the previous core was not a true support. Do not
                    // eliminate on a broken core.
                    debug_assert!(false, "definition core re-solved SAT");
                    return None;
                }
                None => return None, // budget exhausted: kissat ABORT parity
            }
        }

        let pos_len = pos_clauses.len();
        let mut in_core = vec![false; total];
        for &idx in &core_members {
            in_core[idx] = true;
        }
        let mut gate_pos = Vec::new();
        let mut nongate_pos = Vec::new();
        for (i, &ci) in pos_clauses.iter().enumerate() {
            if in_core[i] {
                gate_pos.push(ci);
            } else {
                nongate_pos.push(ci);
            }
        }
        let mut gate_neg = Vec::new();
        let mut nongate_neg = Vec::new();
        for (j, &ci) in neg_clauses.iter().enumerate() {
            if in_core[pos_len + j] {
                gate_neg.push(ci);
            } else {
                nongate_neg.push(ci);
            }
        }
        if gate_pos.is_empty() || gate_neg.is_empty() {
            return None; // one-sided core: unit/failed-literal case, skipped (see doc)
        }
        self.stats.preprocess_def_gate_found += 1;
        Some(GatePartition {
            gate_pos,
            gate_neg,
            nongate_pos,
            nongate_neg,
            kind: ElimGateKind::Definition,
        })
    }

    /// Workspace-reusing wrapper: hands the persistent scratch buffers to the body so
    /// the ~O(vars) pivot attempts per eliminate round are allocation-free in steady
    /// state. Behavior-identical to the former locals-allocating version.
    #[allow(clippy::too_many_arguments)]
    fn try_eliminate_var(
        &mut self,
        var: usize,
        proof_log: &mut ProofLog,
        enqueue_subsumption_work: bool,
        queue: &mut VecDeque<SubsumptionCandidate>,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
        bsr_touched: &mut Vec<usize>,
        bsr_touched_flags: &mut Vec<bool>,
    ) -> bool {
        if !self.elim_scratch {
            return self.try_eliminate_var_legacy(
                var,
                proof_log,
                enqueue_subsumption_work,
                queue,
                touched,
                touched_flags,
                bsr_touched,
                bsr_touched_flags,
            );
        }
        let mut pos_clauses = std::mem::take(&mut self.elim_pos_scratch);
        let mut neg_clauses = std::mem::take(&mut self.elim_neg_scratch);
        let mut resolvent_lits = std::mem::take(&mut self.elim_resolvent_lits_scratch);
        let mut resolvent_ranges = std::mem::take(&mut self.elim_resolvent_ranges_scratch);
        let mut proof_del_lits = std::mem::take(&mut self.elim_proof_del_lits_scratch);
        let mut proof_del_ranges = std::mem::take(&mut self.elim_proof_del_ranges_scratch);
        let result = self.try_eliminate_var_inner(
            var,
            proof_log,
            enqueue_subsumption_work,
            queue,
            touched,
            touched_flags,
            bsr_touched,
            bsr_touched_flags,
            &mut pos_clauses,
            &mut neg_clauses,
            &mut resolvent_lits,
            &mut resolvent_ranges,
            &mut proof_del_lits,
            &mut proof_del_ranges,
        );
        self.elim_pos_scratch = pos_clauses;
        self.elim_neg_scratch = neg_clauses;
        self.elim_resolvent_lits_scratch = resolvent_lits;
        self.elim_resolvent_ranges_scratch = resolvent_ranges;
        self.elim_proof_del_lits_scratch = proof_del_lits;
        self.elim_proof_del_ranges_scratch = proof_del_ranges;
        result
    }

    /// The pre-scratch (2026-07-19) implementation VERBATIM, selected by
    /// SAT_ELIM_SCRATCH=off as the fair simultaneous A/B baseline arm.
    #[allow(clippy::too_many_arguments)]
    fn try_eliminate_var_legacy(
        &mut self,
        var: usize,
        proof_log: &mut ProofLog,
        enqueue_subsumption_work: bool,
        queue: &mut VecDeque<SubsumptionCandidate>,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
        bsr_touched: &mut Vec<usize>,
        bsr_touched_flags: &mut Vec<bool>,
    ) -> bool {
        self.elim_attempted_vars += 1;
        self.clean_occurs_dynamic(var);
        let Some(occurrence_ids) = self.occurs.get(var).cloned() else {
            return false;
        };
        if occurrence_ids.is_empty() {
            return false;
        }

        let mut pos_clauses = Vec::new();
        let mut neg_clauses = Vec::new();
        for clause_ref in occurrence_ids {
            let clause_idx = clause_ref as usize;
            if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
                continue;
            }

            let mut has_pos = false;
            let mut has_neg = false;
            for lit_pos in 0..self.clause_len(clause_idx) {
                let lit = self.clause_lit(clause_idx, lit_pos);
                if lit == var as i32 {
                    has_pos = true;
                } else if lit == -(var as i32) {
                    has_neg = true;
                }
            }
            if has_pos {
                pos_clauses.push(clause_idx);
            } else if has_neg {
                neg_clauses.push(clause_idx);
            }
        }

        let occurrence_count = pos_clauses.len() + neg_clauses.len();
        if occurrence_count == 0 {
            return false;
        }

        // When `var` is functionally defined by an AND/OR gate, restrict resolution to
        // gate-vs-nongate pairs (Plaisted-Greenbaum): the nongate-vs-nongate resolvents are
        // implied by the gate definition and gate-vs-gate resolvents are tautologies, so both
        // are sound to omit. This produces far fewer resolvents, so gate-defined variables
        // pass the `resolvent_count <= occurrence_count + bve_grow` bound that naive all-pairs
        // BVE rejects. The acceptance bound and DRAT add/delete ordering below are unchanged;
        // the resolvents are still ordinary (RUP) resolvents of two live source clauses.
        // Extended detectors run only in ARMED mid-search rounds (`inprocess_aggressive`):
        // root elimination stays byte-identical on every formula, so the blast radius is
        // exactly the congruence-armed miter/BMC cells whose collapse flywheel needs the
        // extra elimination yield. (Root-level extended gate BVE was measured a suite
        // regression in the SAT_GATE_BVE provenance — do not widen this without a gate.)
        let gates_ext = self.elim_gates_ext
            && (self.inprocess_aggressive || self.unarmed_flywheel_round_active);
        let gate = {
            let mut g = None;
            // Kissat gates.c detection order: equivalence → AND/OR → if-then-else.
            if gates_ext {
                g = self.detect_equivalence_gate(var, &pos_clauses, &neg_clauses);
            }
            if g.is_none() && (self.gate_bve || gates_ext) {
                g = self.detect_and_or_gate(var, &pos_clauses, &neg_clauses);
            }
            if g.is_none() && gates_ext {
                g = self.detect_ite_gate(var, &pos_clauses, &neg_clauses);
            }
            if g.is_none() && gates_ext && self.elim_def {
                g = self.detect_kitten_definition(var, &pos_clauses, &neg_clauses);
            }
            g
        };

        let mut resolvent_count = 0isize;
        let mut resolvent_lits = Vec::new();
        let mut resolvent_ranges = Vec::new();
        if let Some(g) = &gate {
            let mut rejected = false;
            // Definition kind: cap each resolvent at its longer parent's length
            // (see resolve_elim_pair_capped doc). Syntactic kinds keep the
            // promoted unlimited behavior.
            // Kissat has NO parent-length cap on definition resolvents — clslim is its
            // only limit. SAT_ELIM_DEF_NOCAP=on restores that parity; default keeps the
            // parent-length cap (the oski40 densification guard: unrestricted definition
            // eliminations doubled the live arena there, +700s wall).
            let def_nocap = self.elim_def_nocap;
            let def_cap = move |s: &Self, p: usize, n: usize| -> Option<usize> {
                if g.kind == ElimGateKind::Definition && !def_nocap {
                    Some(s.clause_len(p).max(s.clause_len(n)))
                } else {
                    None
                }
            };
            'gate_pos_nongate_neg: for &p in &g.gate_pos {
                for &n in &g.nongate_neg {
                    let cap = def_cap(self, p, n);
                    if !self.resolve_elim_pair_capped(
                        p,
                        n,
                        var,
                        occurrence_count,
                        &mut resolvent_count,
                        &mut resolvent_lits,
                        &mut resolvent_ranges,
                        cap,
                    ) {
                        rejected = true;
                        break 'gate_pos_nongate_neg;
                    }
                }
            }
            if !rejected {
                'nongate_pos_gate_neg: for &p in &g.nongate_pos {
                    for &n in &g.gate_neg {
                        let cap = def_cap(self, p, n);
                        if !self.resolve_elim_pair_capped(
                            p,
                            n,
                            var,
                            occurrence_count,
                            &mut resolvent_count,
                            &mut resolvent_lits,
                            &mut resolvent_ranges,
                            cap,
                        ) {
                            rejected = true;
                            break 'nongate_pos_gate_neg;
                        }
                    }
                }
            }
            // Semantic definition cores additionally need the gate-vs-gate resolvents
            // (kissat `resolve_gate`): unlike AND/OR/eq/ITE, they are not tautologies.
            if !rejected && g.kind == ElimGateKind::Definition {
                'gate_pos_gate_neg: for &p in &g.gate_pos {
                    for &n in &g.gate_neg {
                        let cap = def_cap(self, p, n);
                        if !self.resolve_elim_pair_capped(
                            p,
                            n,
                            var,
                            occurrence_count,
                            &mut resolvent_count,
                            &mut resolvent_lits,
                            &mut resolvent_ranges,
                            cap,
                        ) {
                            rejected = true;
                            break 'gate_pos_gate_neg;
                        }
                    }
                }
            }
            if rejected {
                if g.kind == ElimGateKind::Definition && var < self.elim_def_last_probe.len() {
                    let e = &mut self.elim_def_last_probe[var];
                    e.3 = e.3.saturating_add(1);
                }
                return false;
            }
            self.stats.preprocess_gate_eliminated_vars += 1;
            match g.kind {
                ElimGateKind::AndOr => {}
                ElimGateKind::Equivalence => self.stats.preprocess_eq_gate_eliminated_vars += 1,
                ElimGateKind::Ite => self.stats.preprocess_ite_gate_eliminated_vars += 1,
                ElimGateKind::Definition => self.stats.preprocess_def_gate_eliminated_vars += 1,
            }
        } else {
            for &pos_clause_idx in &pos_clauses {
                for &neg_clause_idx in &neg_clauses {
                    if !self.resolve_elim_pair(
                        pos_clause_idx,
                        neg_clause_idx,
                        var,
                        occurrence_count,
                        &mut resolvent_count,
                        &mut resolvent_lits,
                        &mut resolvent_ranges,
                    ) {
                        return false;
                    }
                }
            }
        }

        if pos_clauses.len() > neg_clauses.len() {
            for &clause_idx in &neg_clauses {
                self.push_elim_clause(var, clause_idx);
            }
            self.push_elim_unit(var as i32);
        } else {
            for &clause_idx in &pos_clauses {
                self.push_elim_clause(var, clause_idx);
            }
            self.push_elim_unit(-(var as i32));
        }

        self.eliminated[var] = true;
        self.decision_var[var] = false;
        self.branch_heap_remove(var);
        self.stats.preprocess_eliminated_vars += 1;

        // DRAT ordering invariant: the eliminated source clauses must be DELETED in the
        // proof *after* the resolvents are added (each resolvent is a RAT clause on the
        // eliminated variable, so the checker needs the source clauses still present when
        // it validates the resolvent). The source clauses must also leave the live
        // occurrence/watch structures *before* resolvents are inserted, so immediate
        // subsumption cannot see stale eliminated clauses during resolvent insertion and
        // materially change the post-BVE formula. Those two orderings conflict, so snapshot
        // the source-clause literals now while they are still live and emit their proof
        // deletions after the resolvent loop below. (Emitting them here, before the
        // resolvents, strips the RAT support and drat-trim rejects the proof.)
        let proof_deletions: Vec<Vec<i32>> = if proof_log.is_enabled() {
            pos_clauses
                .iter()
                .chain(neg_clauses.iter())
                .map(|&clause_idx| self.clause_slice(clause_idx).to_vec())
                .collect()
        } else {
            Vec::new()
        };

        for &clause_idx in pos_clauses.iter().chain(neg_clauses.iter()) {
            self.remove_original_clause_preprocess(clause_idx, touched, touched_flags);
        }
        if var < self.occurs.len() {
            self.occurs[var].clear();
            self.occurs_dirty[var] = false;
        }

        for &(start, len) in &resolvent_ranges {
            self.stats.preprocess_resolvents += 1;
            let subsumption_work =
                if enqueue_subsumption_work && !self.preprocess_bsr_budget_exhausted {
                    Some(&mut *queue)
                } else {
                    None
                };
            let result = self.add_original_clause_from_slice(
                &resolvent_lits[start..start + len],
                proof_log,
                true,
                touched,
                touched_flags,
                bsr_touched,
                bsr_touched_flags,
                subsumption_work,
            );
            if result == OriginalClauseInsertResult::Unsat {
                return true;
            }
        }

        // Now that the resolvents have been recorded, delete the eliminated source clauses
        // from the proof (snapshotted above before structure removal). See the DRAT
        // ordering invariant comment near the snapshot.
        for deletion in &proof_deletions {
            proof_log.record_deletion(deletion);
        }

        true
    }

    #[allow(clippy::too_many_arguments)]
    fn try_eliminate_var_inner(
        &mut self,
        var: usize,
        proof_log: &mut ProofLog,
        enqueue_subsumption_work: bool,
        queue: &mut VecDeque<SubsumptionCandidate>,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
        bsr_touched: &mut Vec<usize>,
        bsr_touched_flags: &mut Vec<bool>,
        pos_clauses: &mut Vec<usize>,
        neg_clauses: &mut Vec<usize>,
        resolvent_lits: &mut Vec<i32>,
        resolvent_ranges: &mut Vec<(usize, usize)>,
        proof_del_lits: &mut Vec<i32>,
        proof_del_ranges: &mut Vec<(usize, usize)>,
    ) -> bool {
        let tr = elim_trace::enabled();
        let t_setup = elim_trace::start(tr);
        self.elim_attempted_vars += 1;
        self.clean_occurs_dynamic(var);
        if var >= self.occurs.len() {
            return false;
        }
        let occ_len = self.occurs[var].len();
        elim_trace::add(&elim_trace::SETUP_NS, t_setup);
        if occ_len == 0 {
            return false;
        }

        let t_part = elim_trace::start(tr);
        pos_clauses.clear();
        neg_clauses.clear();
        for occ_pos in 0..occ_len {
            let clause_idx = self.occurs[var][occ_pos] as usize;
            if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
                continue;
            }

            let mut has_pos = false;
            let mut has_neg = false;
            for lit_pos in 0..self.clause_len(clause_idx) {
                let lit = self.clause_lit(clause_idx, lit_pos);
                if lit == var as i32 {
                    has_pos = true;
                } else if lit == -(var as i32) {
                    has_neg = true;
                }
            }
            if has_pos {
                pos_clauses.push(clause_idx);
            } else if has_neg {
                neg_clauses.push(clause_idx);
            }
        }
        elim_trace::add(&elim_trace::PARTITION_NS, t_part);

        let occurrence_count = pos_clauses.len() + neg_clauses.len();
        if occurrence_count == 0 {
            return false;
        }

        // When `var` is functionally defined by an AND/OR gate, restrict resolution to
        // gate-vs-nongate pairs (Plaisted-Greenbaum): the nongate-vs-nongate resolvents are
        // implied by the gate definition and gate-vs-gate resolvents are tautologies, so both
        // are sound to omit. This produces far fewer resolvents, so gate-defined variables
        // pass the `resolvent_count <= occurrence_count + bve_grow` bound that naive all-pairs
        // BVE rejects. The acceptance bound and DRAT add/delete ordering below are unchanged;
        // the resolvents are still ordinary (RUP) resolvents of two live source clauses.
        // Extended detectors run only in ARMED mid-search rounds (`inprocess_aggressive`):
        // root elimination stays byte-identical on every formula, so the blast radius is
        // exactly the congruence-armed miter/BMC cells whose collapse flywheel needs the
        // extra elimination yield. (Root-level extended gate BVE was measured a suite
        // regression in the SAT_GATE_BVE provenance — do not widen this without a gate.)
        let gates_ext = self.elim_gates_ext
            && (self.inprocess_aggressive || self.unarmed_flywheel_round_active);
        let t_gate = elim_trace::start(tr);
        let gate = {
            let mut g = None;
            // Kissat gates.c detection order: equivalence → AND/OR → if-then-else.
            if gates_ext {
                g = self.detect_equivalence_gate(var, pos_clauses, neg_clauses);
            }
            if g.is_none() && (self.gate_bve || gates_ext) {
                g = self.detect_and_or_gate(var, pos_clauses, neg_clauses);
            }
            if g.is_none() && gates_ext {
                g = self.detect_ite_gate(var, pos_clauses, neg_clauses);
            }
            if g.is_none() && gates_ext && self.elim_def {
                g = self.detect_kitten_definition(var, pos_clauses, neg_clauses);
            }
            g
        };
        elim_trace::add(&elim_trace::GATE_NS, t_gate);
        let t_resolve = elim_trace::start(tr);

        let mut resolvent_count = 0isize;
        resolvent_lits.clear();
        resolvent_ranges.clear();
        if let Some(g) = &gate {
            let mut rejected = false;
            // Definition kind: cap each resolvent at its longer parent's length
            // (see resolve_elim_pair_capped doc). Syntactic kinds keep the
            // promoted unlimited behavior.
            // Kissat has NO parent-length cap on definition resolvents — clslim is its
            // only limit. SAT_ELIM_DEF_NOCAP=on restores that parity; default keeps the
            // parent-length cap (the oski40 densification guard: unrestricted definition
            // eliminations doubled the live arena there, +700s wall).
            let def_nocap = self.elim_def_nocap;
            let def_cap = move |s: &Self, p: usize, n: usize| -> Option<usize> {
                if g.kind == ElimGateKind::Definition && !def_nocap {
                    Some(s.clause_len(p).max(s.clause_len(n)))
                } else {
                    None
                }
            };
            'gate_pos_nongate_neg: for &p in &g.gate_pos {
                for &n in &g.nongate_neg {
                    let cap = def_cap(self, p, n);
                    if !self.resolve_elim_pair_capped(
                        p,
                        n,
                        var,
                        occurrence_count,
                        &mut resolvent_count,
                        &mut *resolvent_lits,
                        &mut *resolvent_ranges,
                        cap,
                    ) {
                        rejected = true;
                        break 'gate_pos_nongate_neg;
                    }
                }
            }
            if !rejected {
                'nongate_pos_gate_neg: for &p in &g.nongate_pos {
                    for &n in &g.gate_neg {
                        let cap = def_cap(self, p, n);
                        if !self.resolve_elim_pair_capped(
                            p,
                            n,
                            var,
                            occurrence_count,
                            &mut resolvent_count,
                            &mut *resolvent_lits,
                            &mut *resolvent_ranges,
                            cap,
                        ) {
                            rejected = true;
                            break 'nongate_pos_gate_neg;
                        }
                    }
                }
            }
            // Semantic definition cores additionally need the gate-vs-gate resolvents
            // (kissat `resolve_gate`): unlike AND/OR/eq/ITE, they are not tautologies.
            if !rejected && g.kind == ElimGateKind::Definition {
                'gate_pos_gate_neg: for &p in &g.gate_pos {
                    for &n in &g.gate_neg {
                        let cap = def_cap(self, p, n);
                        if !self.resolve_elim_pair_capped(
                            p,
                            n,
                            var,
                            occurrence_count,
                            &mut resolvent_count,
                            &mut *resolvent_lits,
                            &mut *resolvent_ranges,
                            cap,
                        ) {
                            rejected = true;
                            break 'gate_pos_gate_neg;
                        }
                    }
                }
            }
            if rejected {
                if g.kind == ElimGateKind::Definition && var < self.elim_def_last_probe.len() {
                    let e = &mut self.elim_def_last_probe[var];
                    e.3 = e.3.saturating_add(1);
                }
                elim_trace::add(&elim_trace::RESOLVE_NS, t_resolve);
                return false;
            }
            self.stats.preprocess_gate_eliminated_vars += 1;
            match g.kind {
                ElimGateKind::AndOr => {}
                ElimGateKind::Equivalence => self.stats.preprocess_eq_gate_eliminated_vars += 1,
                ElimGateKind::Ite => self.stats.preprocess_ite_gate_eliminated_vars += 1,
                ElimGateKind::Definition => self.stats.preprocess_def_gate_eliminated_vars += 1,
            }
        } else {
            for &pos_clause_idx in pos_clauses.iter() {
                for &neg_clause_idx in neg_clauses.iter() {
                    if !self.resolve_elim_pair(
                        pos_clause_idx,
                        neg_clause_idx,
                        var,
                        occurrence_count,
                        &mut resolvent_count,
                        &mut *resolvent_lits,
                        &mut *resolvent_ranges,
                    ) {
                        elim_trace::add(&elim_trace::RESOLVE_NS, t_resolve);
                        return false;
                    }
                }
            }
        }
        elim_trace::add(&elim_trace::RESOLVE_NS, t_resolve);
        let t_apply = elim_trace::start(tr);

        if pos_clauses.len() > neg_clauses.len() {
            for &clause_idx in neg_clauses.iter() {
                self.push_elim_clause(var, clause_idx);
            }
            self.push_elim_unit(var as i32);
        } else {
            for &clause_idx in pos_clauses.iter() {
                self.push_elim_clause(var, clause_idx);
            }
            self.push_elim_unit(-(var as i32));
        }

        self.eliminated[var] = true;
        self.decision_var[var] = false;
        self.branch_heap_remove(var);
        self.stats.preprocess_eliminated_vars += 1;
        elim_trace::add(&elim_trace::APPLY_PUSHELIM_NS, t_apply);
        let t_snap = elim_trace::start(tr);

        // DRAT ordering invariant: the eliminated source clauses must be DELETED in the
        // proof *after* the resolvents are added (each resolvent is a RAT clause on the
        // eliminated variable, so the checker needs the source clauses still present when
        // it validates the resolvent). The source clauses must also leave the live
        // occurrence/watch structures *before* resolvents are inserted, so immediate
        // subsumption cannot see stale eliminated clauses during resolvent insertion and
        // materially change the post-BVE formula. Those two orderings conflict, so snapshot
        // the source-clause literals now while they are still live and emit their proof
        // deletions after the resolvent loop below. (Emitting them here, before the
        // resolvents, strips the RAT support and drat-trim rejects the proof.)
        proof_del_lits.clear();
        proof_del_ranges.clear();
        if proof_log.is_enabled() {
            for &clause_idx in pos_clauses.iter().chain(neg_clauses.iter()) {
                let start = proof_del_lits.len();
                proof_del_lits.extend_from_slice(self.clause_slice(clause_idx));
                proof_del_ranges.push((start, proof_del_lits.len() - start));
            }
        }
        elim_trace::add(&elim_trace::APPLY_PROOFSNAP_NS, t_snap);

        let t_remove = elim_trace::start(tr);
        for &clause_idx in pos_clauses.iter().chain(neg_clauses.iter()) {
            self.remove_original_clause_preprocess(clause_idx, touched, touched_flags);
        }
        if var < self.occurs.len() {
            self.occurs[var].clear();
            self.occurs_dirty[var] = false;
        }
        elim_trace::add(&elim_trace::APPLY_REMOVE_NS, t_remove);

        let t_add = elim_trace::start(tr);
        for &(start, len) in resolvent_ranges.iter() {
            self.stats.preprocess_resolvents += 1;
            let subsumption_work =
                if enqueue_subsumption_work && !self.preprocess_bsr_budget_exhausted {
                    Some(&mut *queue)
                } else {
                    None
                };
            let result = self.add_original_clause_from_slice(
                &resolvent_lits[start..start + len],
                proof_log,
                true,
                touched,
                touched_flags,
                bsr_touched,
                bsr_touched_flags,
                subsumption_work,
            );
            if result == OriginalClauseInsertResult::Unsat {
                return true;
            }
        }
        elim_trace::add(&elim_trace::APPLY_ADD_NS, t_add);

        // Now that the resolvents have been recorded, delete the eliminated source clauses
        // from the proof (snapshotted above before structure removal). See the DRAT
        // ordering invariant comment near the snapshot.
        let t_pdel = elim_trace::start(tr);
        for &(start, len) in proof_del_ranges.iter() {
            proof_log.record_deletion(&proof_del_lits[start..start + len]);
        }
        elim_trace::add(&elim_trace::APPLY_PROOFDEL_NS, t_pdel);

        elim_trace::add(&elim_trace::APPLY_NS, t_apply);
        true
    }

    pub(super) fn eliminate(&mut self, turn_off_elim: bool, proof_log: &mut ProofLog) -> bool {
        if !self.solver_ok {
            return false;
        }
        self.preprocess_budget_exhausted = false;
        self.preprocess_bsr_budget_exhausted = false;
        if !self.use_simplification || !self.use_elim {
            return self.simplify_with_proof(proof_log);
        }
        if !self.simplify_with_proof(proof_log) {
            self.solver_ok = false;
            return false;
        }

        // Env-gated wall decomposition (SAT_TRACE_ELIM=1): eprintln-only, measures where
        // root-eliminate wall goes (occurrence build vs BSR vs BVE vs touched-gather).
        let trace_elim = std::env::var("SAT_TRACE_ELIM").is_ok();
        let elim_t0 = std::time::Instant::now();
        let mut t_bsr = std::time::Duration::ZERO;
        let mut t_bve = std::time::Duration::ZERO;
        let mut t_gather = std::time::Duration::ZERO;
        let mut n_bsr_calls = 0u64;
        let mut n_bve_calls = 0u64;

        let run_full_backward_subsumption = self.should_run_full_backward_subsumption();
        let t_occ_start = std::time::Instant::now();
        self.build_occurrence_index();
        let t_occ = t_occ_start.elapsed();
        self.bwdsub_assigns = 0;
        let use_ws = self.round_diet;
        let mut ws = if use_ws {
            // Persistent workspaces: reset to the legacy round-entry state (empty
            // queue/worklists/heap, all-false flags; see ElimRoundWs for the
            // identity argument), keeping the allocations.
            let mut ws = std::mem::take(&mut self.elim_round_ws);
            ws.heap.clear();
            ws.queue.clear();
            for &var in &ws.touched {
                if var < ws.touched_flags.len() {
                    ws.touched_flags[var] = false;
                }
            }
            ws.touched.clear();
            for &var in &ws.bsr_touched {
                if var < ws.bsr_touched_flags.len() {
                    ws.bsr_touched_flags[var] = false;
                }
            }
            ws.bsr_touched.clear();
            ws.touched_flags.resize(self.assignment.len(), false);
            ws.bsr_touched_flags.resize(self.assignment.len(), false);
            ws.heap_versions.resize(self.assignment.len(), 0);
            ws
        } else {
            // Legacy per-round allocations (SAT_ROUND_DIET=off) verbatim.
            ElimRoundWs {
                queue: VecDeque::new(),
                touched: Vec::new(),
                touched_flags: vec![false; self.assignment.len()],
                bsr_touched: Vec::new(),
                bsr_touched_flags: vec![false; self.assignment.len()],
                heap: BinaryHeap::new(),
                heap_versions: vec![0u32; self.assignment.len()],
            }
        };

        let t_heap_build_start = std::time::Instant::now();
        for (var, &version) in ws
            .heap_versions
            .iter()
            .enumerate()
            .take(self.variable_count() + 1)
            .skip(1)
        {
            if self.preprocessing_candidate(var) {
                ws.heap.push(Reverse((self.occurrence_cost(var), var, version)));
            }
        }
        let t_heap_build = t_heap_build_start.elapsed();

        if run_full_backward_subsumption && !self.preprocess_bsr_budget_exhausted {
            let t = elim_trace::start(trace_elim);
            let ok = self.backward_subsumption_check_dynamic(
                true,
                &mut ws.queue,
                &mut ws.touched,
                &mut ws.touched_flags,
                &mut ws.bsr_touched,
                &mut ws.bsr_touched_flags,
                proof_log,
            );
            if let Some(d) = t.elapsed_opt() {
                t_bsr += d;
            }
            n_bsr_calls += 1;
            if !ok {
                self.clear_subsumption_queue_marks(&mut ws.queue);
                self.solver_ok = false;
                if use_ws {
                    self.elim_round_ws = ws;
                }
                return false;
            }
        }

        while self.solver_ok
            && !self.preprocess_budget_exhausted
            && (!ws.touched.is_empty()
                || !ws.bsr_touched.is_empty()
                || (run_full_backward_subsumption
                    && !self.preprocess_bsr_budget_exhausted
                    && (!ws.queue.is_empty() || self.bwdsub_assigns < self.trail.len()))
                || !ws.heap.is_empty())
        {
            if !ws.touched.is_empty() || !ws.bsr_touched.is_empty() {
                let t = elim_trace::start(trace_elim);
                self.gather_touched_clauses(
                    &mut ws.touched,
                    &mut ws.touched_flags,
                    &mut ws.bsr_touched,
                    &mut ws.bsr_touched_flags,
                    &mut ws.queue,
                    &mut ws.heap,
                    &mut ws.heap_versions,
                    run_full_backward_subsumption && !self.preprocess_bsr_budget_exhausted,
                );
                if let Some(d) = t.elapsed_opt() {
                    t_gather += d;
                }
                continue;
            }

            if run_full_backward_subsumption
                && !self.preprocess_bsr_budget_exhausted
                && (!ws.queue.is_empty() || self.bwdsub_assigns < self.trail.len())
            {
                let t = elim_trace::start(trace_elim);
                let ok = self.backward_subsumption_check_dynamic(
                    false,
                    &mut ws.queue,
                    &mut ws.touched,
                    &mut ws.touched_flags,
                    &mut ws.bsr_touched,
                    &mut ws.bsr_touched_flags,
                    proof_log,
                );
                if let Some(d) = t.elapsed_opt() {
                    t_bsr += d;
                }
                n_bsr_calls += 1;
                if !ok {
                    self.solver_ok = false;
                    break;
                }
            }
            if self.preprocess_budget_exhausted {
                break;
            }

            while let Some(Reverse((_, var, version))) = ws.heap.pop() {
                if var >= ws.heap_versions.len() || version != ws.heap_versions[var] {
                    continue;
                }
                if !self.preprocessing_candidate(var) {
                    continue;
                }
                self.clean_occurs_dynamic(var);
                if self.occurs[var].is_empty() {
                    continue;
                }

                let t = elim_trace::start(trace_elim);
                let eliminated = self.try_eliminate_var(
                    var,
                    proof_log,
                    run_full_backward_subsumption && !self.preprocess_bsr_budget_exhausted,
                    &mut ws.queue,
                    &mut ws.touched,
                    &mut ws.touched_flags,
                    &mut ws.bsr_touched,
                    &mut ws.bsr_touched_flags,
                );
                if let Some(d) = t.elapsed_opt() {
                    t_bve += d;
                }
                n_bve_calls += 1;
                if !eliminated {
                    if self.preprocess_budget_exhausted {
                        break;
                    }
                    continue;
                }

                if run_full_backward_subsumption
                    && !self.bsr_drain_batched
                    && !self.preprocess_bsr_budget_exhausted
                    && (!ws.queue.is_empty() || self.bwdsub_assigns < self.trail.len())
                {
                    let t = elim_trace::start(trace_elim);
                    let ok = self.backward_subsumption_check_dynamic(
                        false,
                        &mut ws.queue,
                        &mut ws.touched,
                        &mut ws.touched_flags,
                        &mut ws.bsr_touched,
                        &mut ws.bsr_touched_flags,
                        proof_log,
                    );
                    if let Some(d) = t.elapsed_opt() {
                        t_bsr += d;
                    }
                    n_bsr_calls += 1;
                    if !ok {
                        self.solver_ok = false;
                        break;
                    }
                }
                if self.preprocess_budget_exhausted {
                    break;
                }
            }
        }

        self.clear_subsumption_queue_marks(&mut ws.queue);
        if use_ws {
            self.elim_round_ws = ws;
        }

        if trace_elim {
            eprintln!(
                "c trace_elim total={:.3} occ_build={:.3} bsr={:.3} bve={:.3} gather={:.3} heap_build={:.3} other={:.3} bsr_calls={} bve_calls={}",
                elim_t0.elapsed().as_secs_f64(),
                t_occ.as_secs_f64(),
                t_bsr.as_secs_f64(),
                t_bve.as_secs_f64(),
                t_gather.as_secs_f64(),
                t_heap_build.as_secs_f64(),
                (elim_t0.elapsed() - t_occ - t_bsr - t_bve - t_gather).as_secs_f64(),
                n_bsr_calls,
                n_bve_calls,
            );
            eprintln!(
                "c trace_elim_bve setup={:.3} partition={:.3} gate={:.3} resolve={:.3} apply={:.3}",
                elim_trace::secs(&elim_trace::SETUP_NS),
                elim_trace::secs(&elim_trace::PARTITION_NS),
                elim_trace::secs(&elim_trace::GATE_NS),
                elim_trace::secs(&elim_trace::RESOLVE_NS),
                elim_trace::secs(&elim_trace::APPLY_NS),
            );
            eprintln!(
                "c trace_elim_apply pushelim={:.3} proofsnap={:.3} remove={:.3} add={:.3} proofdel={:.3}",
                elim_trace::secs(&elim_trace::APPLY_PUSHELIM_NS),
                elim_trace::secs(&elim_trace::APPLY_PROOFSNAP_NS),
                elim_trace::secs(&elim_trace::APPLY_REMOVE_NS),
                elim_trace::secs(&elim_trace::APPLY_ADD_NS),
                elim_trace::secs(&elim_trace::APPLY_PROOFDEL_NS),
            );
            eprintln!(
                "c trace_elim_add norm={:.3} proof={:.3} arena={:.3} attach={:.3} index={:.3} enq={:.3}",
                elim_trace::secs(&elim_trace::ADD_NORM_NS),
                elim_trace::secs(&elim_trace::ADD_PROOF_NS),
                elim_trace::secs(&elim_trace::ADD_ARENA_NS),
                elim_trace::secs(&elim_trace::ADD_ATTACH_NS),
                elim_trace::secs(&elim_trace::ADD_INDEX_NS),
                elim_trace::secs(&elim_trace::ADD_ENQ_NS),
            );
        }

        if self.trace_preprocess_details {
            eprintln!(
                "c elim_round attempted={} eliminated_total={} reject_count_bound={} reject_clslim={} reject_defcap={} reject_budget={} bve_grow={} clslim={} conflicts={}",
                self.elim_attempted_vars,
                self.stats.preprocess_eliminated_vars,
                self.elim_reject_count_bound,
                self.elim_reject_clslim,
                self.elim_reject_defcap,
                self.elim_reject_budget,
                self.bve_grow,
                self.bve_clause_limit,
                self.stats.conflicts,
            );
        }

        let original_clause_ids = std::mem::take(&mut self.original_clause_ids);
        self.original_clause_ids = original_clause_ids
            .into_iter()
            .filter(|&clause_idx| {
                let clause_idx = clause_idx as usize;
                clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx)
            })
            .collect();

        if turn_off_elim {
            // Only on the true OOM giants (>20M vars) do we aggressively free the elimination
            // working sets BEFORE the post-preprocessing GC. `clear()` keeps each Vec's
            // capacity, so on a giant ~0.8-2GB of occurrence-list / n_occ / abstraction /
            // clause-id-map address space would stay reserved right when the GC's relocation
            // map + new arena transient needs headroom under the 16GB `ulimit -v` cap. Dropping
            // to empty Vecs frees that virtual memory so the compaction fits. Non-giants keep
            // the exact original `.clear()` behavior (byte-identical) — this reclaim is pure
            // memory management and is behavior-preserving, but restricting it to giants keeps
            // any allocator-timing difference off the instances the baseline already solves.
            let is_giant = self.assignment.len().saturating_sub(1) > 20_000_000;
            if is_giant {
                self.occurs = Vec::new();
                self.occurs_dirty = Vec::new();
                self.occurs_membership_dirty = Vec::new();
                self.n_occ = Vec::new();
                self.clause_abstraction = Vec::new();
                self.learned_id_by_clause = Vec::new();
                self.binary_id_by_clause = Vec::new();
                // The SAT_ROUND_DIET persistent workspaces are ~O(vars) too, and
                // eliminate/ELS never run again after turn-off; free them so the
                // GC relocation transient keeps its headroom (same rationale as
                // the reclaims above, same giant-only scoping).
                self.elim_round_ws = ElimRoundWs::default();
                self.els_active_scratch = Vec::new();
                self.els_binaries_scratch = Vec::new();
                self.els_csr_ws.release();
                self.extract_cache.release();
            } else {
                self.occurs.clear();
                self.occurs_dirty.clear();
                self.occurs_membership_dirty.clear();
                self.n_occ.clear();
            }
            self.use_simplification = false;
            self.inline_original_abstractions = false;
            self.rebuild_branch_queue();
            if is_giant {
                // BVE re-grows per-literal watch-list doubling slack during preprocessing;
                // reclaim it right before the GC so the relocation map (arena-sized, ~1GB on
                // 00fd8ac) fits under the 16GB `ulimit -v` cap. Behavior-preserving.
                self.shrink_watch_lists_if_large();
            }
            self.garbage_collect();
            if !is_giant {
                self.clause_abstraction.clear();
                self.clause_abstraction.shrink_to_fit();
            }
        }

        self.solver_ok
    }

    fn model_lit_value(model: &[u8], lit: i32) -> u8 {
        let var = lit.unsigned_abs() as usize;
        let val = model[var];
        if val == UNASSIGNED {
            return UNASSIGNED;
        }
        if (lit > 0) == (val == TRUE) {
            TRUE
        } else {
            FALSE
        }
    }

    fn extend_model_snapshot(&self, model: &mut [u8]) {
        let mut end = self.elim_clauses.len();
        while end > 0 {
            end -= 1;
            let len = self.elim_clauses[end] as usize;
            let start = end - len;
            let head = self.elim_clauses[start];
            let mut clause_is_falsified = true;
            for idx in (start + 1)..end {
                if Self::model_lit_value(model, self.elim_clauses[idx]) != FALSE {
                    clause_is_falsified = false;
                    break;
                }
            }

            if clause_is_falsified {
                let var = head.unsigned_abs() as usize;
                model[var] = if head > 0 { TRUE } else { FALSE };
            }

            end = start;
        }
    }

    pub(super) fn capture_sat_model(&mut self) {
        let mut model = self.assignment.clone();
        for (var, value) in model.iter_mut().enumerate().skip(1) {
            if *value == UNASSIGNED && !self.eliminated[var] {
                *value = TRUE;
            }
        }
        self.extend_model_snapshot(&mut model);
        for value in model.iter_mut().skip(1) {
            if *value == UNASSIGNED {
                *value = TRUE;
            }
        }
        self.assignment.clone_from(&model);
        self.sat_model = Some(model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_original_clauses(s: &Solver) -> Vec<Vec<i32>> {
        let mut clauses: Vec<Vec<i32>> = s
            .original_clause_ids
            .iter()
            .map(|&clause_idx| clause_idx as usize)
            .filter(|&clause_idx| !s.clause_is_deleted(clause_idx))
            .map(|clause_idx| s.clause_slice(clause_idx).to_vec())
            .collect();
        clauses.sort();
        clauses
    }

    fn assert_no_subsumption_queue_marks(s: &Solver) {
        for &clause_idx in &s.original_clause_ids {
            let clause_idx = clause_idx as usize;
            if clause_idx < s.arena.len() && !s.clause_is_deleted(clause_idx) {
                assert_ne!(
                    clause_header_mark(s.clause_header(clause_idx)),
                    2,
                    "live original clause {clause_idx} kept a BSR queue mark"
                );
            }
        }
    }

    fn run_backward_subsumption(s: &mut Solver, seed_all: bool, proof: &mut ProofLog) -> bool {
        let mut queue = VecDeque::new();
        let mut touched = Vec::new();
        let mut touched_flags = vec![false; s.assignment.len()];
        let mut bsr_touched = Vec::new();
        let mut bsr_touched_flags = vec![false; s.assignment.len()];
        s.backward_subsumption_check_dynamic(
            seed_all,
            &mut queue,
            &mut touched,
            &mut touched_flags,
            &mut bsr_touched,
            &mut bsr_touched_flags,
            proof,
        )
    }

    #[test]
    fn marked_subsumption_relation_detects_subsumed_clause() {
        let mut s = Solver::new(
            10,
            vec![vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10], vec![2, 4, 6, 8]],
        );
        let candidate = s.original_clause_ids[0] as usize ;
        let driver_idx = s.original_clause_ids[1] as usize ;
        let driver = SubsumptionCandidate::Clause(driver_idx);
        let mut marks = Vec::new();
        let mut stamp = 0;

        let relation = s.subsumption_relation::<true>(
            driver,
            s.subsumption_driver_len(driver),
            s.subsumption_driver_abstraction(driver),
            candidate,
            &mut marks,
            &mut stamp,
        );

        assert!(matches!(relation, SubsumptionOutcome::Subsumed));
        assert_eq!(s.stats.bsr_relation_marked_calls, 1);
        assert_eq!(stamp, 1);
    }

    #[test]
    fn marked_subsumption_relation_detects_strengthen_literal() {
        let mut s = Solver::new(
            10,
            vec![vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10], vec![2, -4, 6, 8]],
        );
        let candidate = s.original_clause_ids[0] as usize ;
        let driver_idx = s.original_clause_ids[1] as usize ;
        let driver = SubsumptionCandidate::Clause(driver_idx);
        let mut marks = Vec::new();
        let mut stamp = 0;

        let relation = s.subsumption_relation::<true>(
            driver,
            s.subsumption_driver_len(driver),
            s.subsumption_driver_abstraction(driver),
            candidate,
            &mut marks,
            &mut stamp,
        );

        assert!(matches!(relation, SubsumptionOutcome::Strengthen(4)));
        assert_eq!(s.stats.bsr_relation_marked_calls, 1);
    }

    #[test]
    fn marked_subsumption_relation_rejects_two_complements() {
        let mut s = Solver::new(
            10,
            vec![vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10], vec![-2, -4, 6, 8]],
        );
        let candidate = s.original_clause_ids[0] as usize ;
        let driver_idx = s.original_clause_ids[1] as usize ;
        let driver = SubsumptionCandidate::Clause(driver_idx);
        let mut marks = Vec::new();
        let mut stamp = 0;

        let relation = s.subsumption_relation::<true>(
            driver,
            s.subsumption_driver_len(driver),
            s.subsumption_driver_abstraction(driver),
            candidate,
            &mut marks,
            &mut stamp,
        );

        assert!(matches!(relation, SubsumptionOutcome::None));
        assert_eq!(s.stats.bsr_relation_marked_calls, 1);
    }

    #[test]
    fn backward_subsumption_removes_subsumed_original_clause() {
        let mut s = Solver::new(3, vec![vec![1, 2], vec![1, 2, 3]]);
        let mut proof = ProofLog::disabled();
        s.build_occurrence_index();

        assert!(run_backward_subsumption(&mut s, true, &mut proof));

        assert_eq!(live_original_clauses(&s), vec![vec![1, 2]]);
        assert_eq!(s.stats.preprocess_subsumed_clauses, 1);
    }

    #[test]
    fn backward_subsumption_resolution_strengthens_original_clause() {
        let mut s = Solver::new(3, vec![vec![1, 2], vec![1, -2, 3]]);
        let mut proof = ProofLog::disabled();
        s.build_occurrence_index();

        assert!(run_backward_subsumption(&mut s, true, &mut proof));

        assert_eq!(live_original_clauses(&s), vec![vec![1, 2], vec![1, 3]]);
        assert_eq!(s.stats.preprocess_strengthened_clauses, 1);
    }

    #[test]
    fn root_assignment_subsumption_trims_false_literal() {
        let mut s = Solver::new(3, vec![vec![-1, 2, 3]]);
        let mut proof = ProofLog::disabled();
        assert!(s.enqueue(1, ReasonRef::None));
        assert_eq!(s.propagate(), None);
        s.build_occurrence_index();

        assert!(run_backward_subsumption(&mut s, false, &mut proof));

        assert!(live_original_clauses(&s).contains(&vec![2, 3]));
        assert_eq!(s.stats.preprocess_strengthened_clauses, 1);
    }

    #[test]
    fn removed_original_clause_vars_are_touched_for_preprocess_retry() {
        let mut s = Solver::new(4, vec![vec![1, 2, -3], vec![1, 3], vec![2, 4]]);
        s.build_occurrence_index();
        let clause_idx = s.original_clause_ids[0] as usize ;
        let mut touched = Vec::new();
        let mut touched_flags = vec![false; s.assignment.len()];

        s.remove_original_clause_preprocess(clause_idx, &mut touched, &mut touched_flags);
        touched.sort_unstable();

        assert_eq!(touched, vec![1, 2, 3]);
        assert!(touched_flags[1]);
        assert!(touched_flags[2]);
        assert!(touched_flags[3]);
        assert!(s.occurs_dirty[1]);
        assert!(s.occurs_dirty[2]);
        assert!(s.occurs_dirty[3]);
    }

    #[test]
    fn heap_only_touched_vars_refresh_bve_heap_without_bsr_queue() {
        let mut s = Solver::new(3, vec![vec![1, 2], vec![1, 3]]);
        s.build_occurrence_index();
        let mut touched = vec![1];
        let mut touched_flags = vec![false; s.assignment.len()];
        touched_flags[1] = true;
        let mut bsr_touched = Vec::new();
        let mut bsr_touched_flags = vec![false; s.assignment.len()];
        let mut queue = VecDeque::new();
        let mut heap = BinaryHeap::new();
        let mut heap_versions = vec![0u32; s.assignment.len()];

        s.gather_touched_clauses(
            &mut touched,
            &mut touched_flags,
            &mut bsr_touched,
            &mut bsr_touched_flags,
            &mut queue,
            &mut heap,
            &mut heap_versions,
            true,
        );

        assert!(touched.is_empty());
        assert!(!touched_flags[1]);
        assert!(bsr_touched.is_empty());
        assert!(queue.is_empty());
        assert_eq!(heap_versions[1], 1);
        assert!(heap
            .iter()
            .any(|Reverse((_, var, version))| *var == 1 && *version == 1));
    }

    #[test]
    fn bsr_touched_vars_enqueue_subsumption_work() {
        let mut s = Solver::new(3, vec![vec![1, 2], vec![1, 3]]);
        s.build_occurrence_index();
        let mut touched = Vec::new();
        let mut touched_flags = vec![false; s.assignment.len()];
        let mut bsr_touched = vec![1];
        let mut bsr_touched_flags = vec![false; s.assignment.len()];
        bsr_touched_flags[1] = true;
        let mut queue = VecDeque::new();
        let mut heap = BinaryHeap::new();
        let mut heap_versions = vec![0u32; s.assignment.len()];

        s.gather_touched_clauses(
            &mut touched,
            &mut touched_flags,
            &mut bsr_touched,
            &mut bsr_touched_flags,
            &mut queue,
            &mut heap,
            &mut heap_versions,
            true,
        );

        assert!(bsr_touched.is_empty());
        assert!(!bsr_touched_flags[1]);
        assert_eq!(queue.len(), 2);
        assert!(heap.is_empty());
    }

    #[test]
    fn backward_subsumption_occurrence_limit_skips_dense_driver_scan() {
        let config = SolverConfig {
            bsr_occurrence_limit: 1,
            trace_preprocess_details: true,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(2, vec![vec![1, 2], vec![1, 2]], &config);
        let mut proof = ProofLog::disabled();
        s.build_occurrence_index();

        assert!(run_backward_subsumption(&mut s, true, &mut proof));

        assert_eq!(live_original_clauses(&s), vec![vec![1, 2], vec![1, 2]]);
        assert_eq!(s.stats.bsr_skip_occurrence_limit, 2);
        assert_eq!(s.stats.bsr_candidates_seen, 0);
        assert_eq!(s.stats.preprocess_subsumed_clauses, 0);
    }

    #[test]
    fn backward_subsumption_occurrence_limit_zero_keeps_unlimited_scan() {
        let config = SolverConfig {
            bsr_occurrence_limit: 0,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(2, vec![vec![1, 2], vec![1, 2]], &config);
        let mut proof = ProofLog::disabled();
        s.build_occurrence_index();

        assert!(run_backward_subsumption(&mut s, true, &mut proof));

        assert_eq!(live_original_clauses(&s), vec![vec![1, 2]]);
        assert_eq!(s.stats.preprocess_subsumed_clauses, 1);
    }

    #[test]
    fn backward_subsumption_seed_all_runs_short_clause_before_tick_budget_hits() {
        let config = SolverConfig {
            eliminate_ticks_budget: 1,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(3, vec![vec![1, 2, 3], vec![1, 2]], &config);
        let mut proof = ProofLog::disabled();
        s.build_occurrence_index();

        assert!(run_backward_subsumption(&mut s, true, &mut proof));

        assert_eq!(live_original_clauses(&s), vec![vec![1, 2]]);
        assert_eq!(s.stats.preprocess_bsr_ticks, 1);
        assert_eq!(s.stats.preprocess_subsumed_clauses, 1);
        assert!(s.preprocess_bsr_budget_exhausted);
        assert_no_subsumption_queue_marks(&s);
    }

    #[test]
    fn backward_subsumption_best_var_ignores_stale_occurs_tombstones() {
        let config = SolverConfig {
            bsr_occurrence_limit: 2,
            trace_preprocess_details: true,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(
            9,
            vec![
                vec![1, 2],
                vec![1, 2, 3],
                vec![2, 4],
                vec![3, 5],
                vec![3, 6],
                vec![1, 7],
                vec![1, 8],
                vec![1, 9],
            ],
            &config,
        );
        let mut proof = ProofLog::disabled();
        s.build_occurrence_index();

        let driver = s.original_clause_ids[0] as usize ;
        let mut touched = Vec::new();
        let mut touched_flags = vec![false; s.assignment.len()];
        for clause_idx in s.original_clause_ids[5..].to_vec() {
            s.remove_original_clause_preprocess(
                clause_idx as usize,
                &mut touched,
                &mut touched_flags,
            );
        }

        assert!(s.occurs_dirty[1]);
        assert!(s.occurs[1].len() > s.occurs[2].len());
        assert_eq!(s.n_occ[lit_to_index(1)], 2);
        assert_eq!(s.n_occ[lit_to_index(2)], 3);
        assert_eq!(s.n_occ[lit_to_index(3)], 3);

        let mut queue = VecDeque::new();
        queue.push_back(SubsumptionCandidate::Clause(driver));
        let mut bsr_touched = Vec::new();
        let mut bsr_touched_flags = vec![false; s.assignment.len()];
        assert!(s.backward_subsumption_check_dynamic(
            false,
            &mut queue,
            &mut touched,
            &mut touched_flags,
            &mut bsr_touched,
            &mut bsr_touched_flags,
            &mut proof,
        ));

        assert_eq!(
            live_original_clauses(&s),
            vec![vec![1, 2], vec![2, 4], vec![3, 5], vec![3, 6]]
        );
        assert_eq!(s.stats.preprocess_subsumed_clauses, 1);
        assert_eq!(s.stats.bsr_skip_occurrence_limit, 0);
    }

    #[test]
    fn eliminate_respects_resolution_budget_before_variable_edit() {
        let config = SolverConfig {
            eliminate_resolution_budget: 1,
            full_bsr: false,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(4, vec![vec![1, 2], vec![1, 3], vec![-1, 4]], &config);
        s.frozen[2] = true;
        s.frozen[3] = true;
        s.frozen[4] = true;
        let mut proof = ProofLog::disabled();

        assert!(s.eliminate(false, &mut proof));

        assert!(
            !s.eliminated[1],
            "budget must stop before partial BVE edits"
        );
        assert!(s.preprocess_budget_exhausted);
        assert_eq!(s.stats.preprocess_resolution_attempts, 1);
        assert_eq!(s.stats.preprocess_resolution_budget_hits, 1);
        assert_eq!(
            live_original_clauses(&s),
            vec![vec![-1, 4], vec![1, 2], vec![1, 3]]
        );
    }

    #[test]
    fn eliminate_occurrence_limit_skips_high_degree_variable() {
        // var 1: 2 positive occurrences (1,2),(1,3) + 2 negative (-1,4),(-1,5). With occlim=1 it
        // exceeds the per-polarity cap (kissat eliminateocclim) and must be skipped, not eliminated.
        let config = SolverConfig {
            eliminate_occurrence_limit: 1,
            full_bsr: false,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(
            5,
            vec![vec![1, 2], vec![1, 3], vec![-1, 4], vec![-1, 5]],
            &config,
        );
        s.frozen[2] = true;
        s.frozen[3] = true;
        s.frozen[4] = true;
        s.frozen[5] = true;
        let mut proof = ProofLog::disabled();
        assert!(s.eliminate(false, &mut proof));
        assert!(
            !s.eliminated[1],
            "var 1 (2 pos + 2 neg occ) must be skipped under occlim=1"
        );
    }

    #[test]
    fn eliminate_occurrence_limit_zero_is_unlimited() {
        // Same formula; occlim=0 (the shipped default) -> var 1 IS eliminated (4 non-growing resolvents).
        let config = SolverConfig {
            eliminate_occurrence_limit: 0,
            full_bsr: false,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(
            5,
            vec![vec![1, 2], vec![1, 3], vec![-1, 4], vec![-1, 5]],
            &config,
        );
        s.frozen[2] = true;
        s.frozen[3] = true;
        s.frozen[4] = true;
        s.frozen[5] = true;
        let mut proof = ProofLog::disabled();
        assert!(s.eliminate(false, &mut proof));
        assert!(
            s.eliminated[1],
            "var 1 must be eliminated when occlim=0 (unlimited)"
        );
    }

    #[test]
    fn eliminate_turn_off_drops_dead_clause_abstraction_table() {
        let mut s = Solver::new(3, vec![vec![1, 2], vec![-1, 3]]);
        s.build_occurrence_index();
        assert!(!s.inline_original_abstractions);
        assert!(!s.clause_abstraction.is_empty());
        assert!(s.clause_abstraction.capacity() > 0);
        let mut proof = ProofLog::disabled();

        assert!(s.eliminate(true, &mut proof));

        assert!(!s.use_simplification);
        assert!(s.clause_abstraction.is_empty());
        assert_eq!(s.clause_abstraction.capacity(), 0);
    }

    #[test]
    fn eliminate_respects_tick_budget_during_bsr() {
        let config = SolverConfig {
            eliminate_ticks_budget: 1,
            ..SolverConfig::default()
        };
        let mut s =
            Solver::new_with_config(4, vec![vec![1, 2], vec![1, 2, 3], vec![1, 2, 4]], &config);
        let mut proof = ProofLog::disabled();

        assert!(s.eliminate(false, &mut proof));

        assert!(s.preprocess_bsr_budget_exhausted);
        assert!(!s.preprocess_budget_exhausted);
        assert_eq!(s.stats.preprocess_bsr_ticks, 1);
        assert_eq!(s.stats.preprocess_bsr_tick_budget_hits, 1);
        assert_eq!(s.stats.preprocess_tick_budget_hits, 0);
        assert!(s.solver_ok);
        assert_no_subsumption_queue_marks(&s);
    }

    #[test]
    fn bsr_budget_exhaustion_clears_pending_queue_marks() {
        let config = SolverConfig {
            eliminate_ticks_budget: 1,
            ..SolverConfig::default()
        };
        let mut s =
            Solver::new_with_config(4, vec![vec![1, 2], vec![1, 2, 3], vec![1, 2, 4]], &config);
        let mut proof = ProofLog::disabled();
        let mut queue = VecDeque::new();
        let mut touched = Vec::new();
        let mut touched_flags = vec![false; s.assignment.len()];
        let mut bsr_touched = Vec::new();
        let mut bsr_touched_flags = vec![false; s.assignment.len()];

        s.build_occurrence_index();
        assert!(s.backward_subsumption_check_dynamic(
            true,
            &mut queue,
            &mut touched,
            &mut touched_flags,
            &mut bsr_touched,
            &mut bsr_touched_flags,
            &mut proof,
        ));

        assert!(s.preprocess_bsr_budget_exhausted);
        assert!(
            queue.is_empty(),
            "BSR queue should be drained after budget exit"
        );
        assert_no_subsumption_queue_marks(&s);
    }

    #[test]
    fn bve_resolvents_are_not_bsr_marked_when_full_bsr_is_off() {
        let config = SolverConfig {
            full_bsr: false,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(3, vec![vec![1, 2], vec![-1, 3]], &config);
        s.frozen[2] = true;
        s.frozen[3] = true;
        let mut proof = ProofLog::disabled();

        assert!(s.eliminate(false, &mut proof));

        assert!(s.eliminated[1]);
        assert_eq!(live_original_clauses(&s), vec![vec![2, 3]]);
        assert_no_subsumption_queue_marks(&s);
    }

    #[test]
    fn bve_sorted_resolvent_merge_deduplicates_and_preserves_order() {
        let mut s = Solver::new(8, vec![vec![1, 5, 7], vec![-1, 2, 5, 8]]);
        s.build_occurrence_index();
        let lhs = s.original_clause_ids[0] as usize ;
        let rhs = s.original_clause_ids[1] as usize ;
        let mut resolvent = Vec::new();

        assert!(s.append_resolvent_into_vec(lhs, rhs, 1, &mut resolvent));
        assert_eq!(resolvent, vec![2, 5, 7, 8]);
    }

    #[test]
    fn bve_sorted_resolvent_merge_rejects_tautology() {
        let mut s = Solver::new(4, vec![vec![1, 2, 4], vec![-1, -2, 3]]);
        s.build_occurrence_index();
        let lhs = s.original_clause_ids[0] as usize ;
        let rhs = s.original_clause_ids[1] as usize ;
        let mut resolvent = Vec::new();

        assert!(!s.append_resolvent_into_vec(lhs, rhs, 1, &mut resolvent));
        assert!(resolvent.is_empty());
    }

    #[test]
    fn bsr_tick_budget_exhaustion_does_not_stop_bve_heap() {
        let config = SolverConfig {
            eliminate_ticks_budget: 1,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(2, vec![vec![1, 2], vec![-1, 2]], &config);
        s.frozen[2] = true;
        let mut proof = ProofLog::disabled();

        assert!(s.eliminate(false, &mut proof));

        assert!(
            s.eliminated[1],
            "BVE should still run after BSR consumes its own tick budget"
        );
        assert!(
            !s.preprocess_budget_exhausted,
            "BSR tick exhaustion must not stop the BVE budget"
        );
    }

    #[test]
    fn zero_eliminate_budget_disables_counting_and_stays_unlimited() {
        // Explicit 0 budgets are the unlimited opt-out: no counting in the hot loop.
        let config = SolverConfig {
            eliminate_ticks_budget: 0,
            eliminate_resolution_budget: 0,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(4, vec![vec![1, 2], vec![1, 3], vec![-1, 4]], &config);
        s.frozen[2] = true;
        s.frozen[3] = true;
        s.frozen[4] = true;
        let mut proof = ProofLog::disabled();

        assert!(s.eliminate(false, &mut proof));

        assert_eq!(s.stats.preprocess_eliminate_ticks, 0);
        assert_eq!(s.stats.preprocess_bsr_ticks, 0);
        assert_eq!(s.stats.preprocess_resolution_attempts, 0);
        assert_eq!(s.stats.preprocess_resolution_budget_hits, 0);
        assert_eq!(s.stats.preprocess_tick_budget_hits, 0);
        assert_eq!(s.stats.preprocess_bsr_tick_budget_hits, 0);
    }

    #[test]
    fn default_eliminate_budget_is_active_and_spares_small_formulas() {
        // bead 5b2.3.24: the shipped default budgets are non-zero, so eliminate work
        // is counted by default — but they are generous enough that a small formula
        // completes its full pass without exhausting them.
        let mut s = Solver::new(4, vec![vec![1, 2], vec![1, 3], vec![-1, 4]]);
        s.frozen[2] = true;
        s.frozen[3] = true;
        s.frozen[4] = true;
        let mut proof = ProofLog::disabled();

        assert!(s.eliminate(false, &mut proof));

        assert!(
            s.stats.preprocess_eliminate_ticks > 0,
            "default budgets must count eliminate work"
        );
        assert!(
            !s.preprocess_budget_exhausted,
            "small formula must not exhaust the default budget"
        );
        assert!(
            s.eliminated[1],
            "BVE must proceed normally under the default budget"
        );
    }

    #[test]
    fn partial_budgeted_elimination_still_solves_safely() {
        let config = SolverConfig {
            eliminate_resolution_budget: 1,
            full_bsr: false,
            ..SolverConfig::default()
        };
        let clauses = vec![vec![1, 2], vec![-1, 2], vec![3, 4], vec![3, 5], vec![-3, 6]];
        let mut s = Solver::new_with_config(6, clauses.clone(), &config);
        for var in [2, 4, 5, 6] {
            s.frozen[var] = true;
        }

        assert!(s.solve());
        assert!(
            s.eliminated[1],
            "first variable should finish before budget hit"
        );
        assert!(
            !s.eliminated[3],
            "second variable should be left for search"
        );

        let model = s.sat_model.as_ref().expect("missing SAT model snapshot");
        for clause in clauses {
            assert!(
                clause.iter().any(|&lit| {
                    let var = lit.unsigned_abs() as usize;
                    (lit > 0 && model[var] == TRUE) || (lit < 0 && model[var] == FALSE)
                }),
                "extended model does not satisfy {clause:?}"
            );
        }
    }

    // x=1 <-> (a=2 OR b=3) encoded as gate clauses (1∨-2),(1∨-3),(-1∨2∨3), plus
    // `pos_extra` clauses (1∨pi) and `neg_extra` clauses (-1∨qj). Naive BVE generates
    // ~(2+m)(1+n) - tautologies resolvents incl. the m*n nongate×nongate cross terms;
    // gate-aware BVE drops the m*n terms.
    fn or_gate_with_extras(pos_extra: &[i32], neg_extra: &[i32]) -> Vec<Vec<i32>> {
        let mut c = vec![vec![1, -2], vec![1, -3], vec![-1, 2, 3]];
        for &p in pos_extra {
            c.push(vec![1, p]);
        }
        for &q in neg_extra {
            c.push(vec![-1, q]);
        }
        c
    }

    #[test]
    fn gate_bve_eliminates_var_that_naive_bve_rejects() {
        // 3 pos + 3 neg extras: naive non-taut resolvents = 6 (gatebin×negextra) +
        // 3 (posextra×base) + 9 (posextra×negextra) = 18 > occ(9); gate-aware = 6 + 3 = 9 <= 9.
        let clauses = or_gate_with_extras(&[4, 5, 6], &[7, 8, 9]);
        let run = |gate_on: bool| {
            let config = SolverConfig {
                gate_extract: gate_on,
                gate_bve: gate_on,
                full_bsr: false,
                ..SolverConfig::default()
            };
            let mut s = Solver::new_with_config(9, clauses.clone(), &config);
            for var in 2..=9 {
                s.frozen[var] = true;
            }
            let sat = s.solve();
            (s, sat)
        };

        let (s_off, sat_off) = run(false);
        assert!(
            !s_off.eliminated[1],
            "naive BVE must reject x (18 resolvents > 9 occurrences)"
        );
        assert_eq!(s_off.stats.preprocess_gate_eliminated_vars, 0);

        let (s_on, sat_on) = run(true);
        assert!(
            s_on.eliminated[1],
            "gate-aware BVE must eliminate x (9 gate-restricted resolvents <= 9 occurrences)"
        );
        assert_eq!(
            s_on.stats.preprocess_gate_eliminated_vars, 1,
            "exactly one gate elimination expected"
        );

        // Soundness: both SAT, and the gate-on extended model satisfies every original clause.
        assert!(sat_off && sat_on, "instance is satisfiable in both configs");
        let model = s_on.sat_model.as_ref().expect("missing SAT model");
        for clause in &clauses {
            assert!(
                clause.iter().any(|&lit| {
                    let v = lit.unsigned_abs() as usize;
                    (lit > 0 && model[v] == TRUE) || (lit < 0 && model[v] == FALSE)
                }),
                "gate-eliminated model violates original clause {clause:?}"
            );
        }
    }

    #[test]
    fn scoped_gate_bve_adopts_on_net_elimination_gain() {
        // Same OR-gate formula as gate_bve_eliminates_var_that_naive_bve_rejects:
        // plain BVE rejects the pivot (E0=0), gate-aware BVE eliminates it (E1=1),
        // so the scoped dry-run must adopt and the real run eliminates the var.
        let clauses = or_gate_with_extras(&[4, 5, 6], &[7, 8, 9]);
        let config = SolverConfig {
            gate_bve_scoped: true,
            full_bsr: false,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(9, clauses, &config);
        for var in 2..=9 {
            s.frozen[var] = true;
        }
        let mut proof = ProofLog::disabled();
        assert!(s.solve_with_proof(&mut proof, &config));
        assert_eq!(s.stats.gate_bve_dryrun_e0, 0);
        assert_eq!(s.stats.gate_bve_dryrun_e1, 1);
        assert_eq!(s.stats.gate_bve_scoped_adopted, 1);
        assert!(s.eliminated[1], "adopted gate-aware BVE must eliminate x");
        assert_eq!(s.stats.preprocess_gate_eliminated_vars, 1);
    }

    #[test]
    fn scoped_gate_bve_size_cap_keeps_plain_path() {
        // Same formula, but the var cap is below the formula size: the dry-run must
        // not fire and the run stays on the plain path (pivot not eliminated).
        let clauses = or_gate_with_extras(&[4, 5, 6], &[7, 8, 9]);
        let config = SolverConfig {
            gate_bve_scoped: true,
            gate_bve_scoped_max_vars: 3,
            full_bsr: false,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(9, clauses, &config);
        for var in 2..=9 {
            s.frozen[var] = true;
        }
        let mut proof = ProofLog::disabled();
        assert!(s.solve_with_proof(&mut proof, &config));
        assert_eq!(s.stats.gate_bve_dryrun_e0, 0);
        assert_eq!(s.stats.gate_bve_dryrun_e1, 0);
        assert_eq!(s.stats.gate_bve_scoped_adopted, 0);
        assert!(!s.eliminated[1], "capped scoped run must keep plain BVE");
        assert_eq!(s.stats.preprocess_gate_eliminated_vars, 0);
    }

    #[test]
    fn scoped_gate_bve_keeps_plain_when_gates_add_nothing() {
        // No gate structure: plain and gated dry-runs eliminate the same count
        // (E1 == E0 > 0), the 2% threshold rejects, and gate_bve stays off while
        // the plain elimination still happens.
        let clauses = vec![vec![1, 2], vec![-1, 3]];
        let config = SolverConfig {
            gate_bve_scoped: true,
            full_bsr: false,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(3, clauses, &config);
        s.frozen[2] = true;
        s.frozen[3] = true;
        let mut proof = ProofLog::disabled();
        assert!(s.solve_with_proof(&mut proof, &config));
        assert_eq!(s.stats.gate_bve_dryrun_e0, s.stats.gate_bve_dryrun_e1);
        assert!(s.stats.gate_bve_dryrun_e0 > 0);
        assert_eq!(s.stats.gate_bve_scoped_adopted, 0);
        assert!(!s.gate_bve, "threshold reject must leave gate_bve off");
        assert!(s.eliminated[1], "plain BVE must still eliminate the var");
        assert_eq!(s.stats.preprocess_gate_eliminated_vars, 0);
    }

    #[test]
    fn eq_gate_bve_eliminates_var_that_naive_bve_rejects() {
        // x=1 ≡ a=2 via (1,-2),(-1,2), plus 3 pos and 3 neg extra binaries.
        // Naive non-taut resolvents = 3 + 3 + 9 = 15 > occ(8); EQ-aware = 3 + 3 = 6 <= 8.
        let clauses = vec![
            vec![1, -2],
            vec![-1, 2],
            vec![1, 4],
            vec![1, 5],
            vec![1, 6],
            vec![-1, 7],
            vec![-1, 8],
            vec![-1, 9],
        ];
        let run = |ext_on: bool| {
            let config = SolverConfig {
                full_bsr: false,
                ..SolverConfig::default()
            };
            let mut s = Solver::new_with_config(9, clauses.clone(), &config);
            s.elim_gates_ext = ext_on;
            s.inprocess_aggressive = true;
            for var in 2..=9 {
                s.frozen[var] = true;
            }
            let sat = s.solve();
            (s, sat)
        };

        let (s_off, sat_off) = run(false);
        assert!(
            !s_off.eliminated[1],
            "naive BVE must reject x (15 resolvents > 8 occurrences)"
        );
        assert_eq!(s_off.stats.preprocess_eq_gate_eliminated_vars, 0);

        let (s_on, sat_on) = run(true);
        assert!(
            s_on.eliminated[1],
            "EQ-gate BVE must eliminate x (6 gate-restricted resolvents <= 8 occurrences)"
        );
        assert_eq!(s_on.stats.preprocess_eq_gate_eliminated_vars, 1);

        assert!(sat_off && sat_on, "instance is satisfiable in both configs");
        let model = s_on.sat_model.as_ref().expect("missing SAT model");
        for clause in &clauses {
            assert!(
                clause.iter().any(|&lit| {
                    let v = lit.unsigned_abs() as usize;
                    (lit > 0 && model[v] == TRUE) || (lit < 0 && model[v] == FALSE)
                }),
                "EQ-eliminated model violates original clause {clause:?}"
            );
        }
    }

    #[test]
    fn ite_gate_bve_eliminates_var_that_naive_bve_rejects() {
        // x=1 = ITE(c=2, t=3, e=4): the four Tseitin ternaries, plus 2 pos and 2 neg
        // extra binaries chosen to match no equivalence or AND/OR pattern.
        // Naive non-taut resolvents = 4 + 4 + 4 = 12 > occ(8); ITE-aware = 4 + 4 = 8 <= 8.
        let clauses = vec![
            vec![1, -2, -3],
            vec![1, 2, -4],
            vec![-1, -2, 3],
            vec![-1, 2, 4],
            vec![1, 5],
            vec![1, 6],
            vec![-1, 7],
            vec![-1, 8],
        ];
        let run = |ext_on: bool| {
            let config = SolverConfig {
                full_bsr: false,
                ..SolverConfig::default()
            };
            let mut s = Solver::new_with_config(8, clauses.clone(), &config);
            s.elim_gates_ext = ext_on;
            s.inprocess_aggressive = true;
            for var in 2..=8 {
                s.frozen[var] = true;
            }
            let sat = s.solve();
            (s, sat)
        };

        let (s_off, sat_off) = run(false);
        assert!(
            !s_off.eliminated[1],
            "naive BVE must reject x (12 resolvents > 8 occurrences)"
        );
        assert_eq!(s_off.stats.preprocess_ite_gate_eliminated_vars, 0);

        let (s_on, sat_on) = run(true);
        assert!(
            s_on.eliminated[1],
            "ITE-gate BVE must eliminate x (8 gate-restricted resolvents <= 8 occurrences)"
        );
        assert_eq!(s_on.stats.preprocess_ite_gate_eliminated_vars, 1);

        assert!(sat_off && sat_on, "instance is satisfiable in both configs");
        let model = s_on.sat_model.as_ref().expect("missing SAT model");
        for clause in &clauses {
            assert!(
                clause.iter().any(|&lit| {
                    let v = lit.unsigned_abs() as usize;
                    (lit > 0 && model[v] == TRUE) || (lit < 0 && model[v] == FALSE)
                }),
                "ITE-eliminated model violates original clause {clause:?}"
            );
        }
    }

    #[test]
    fn elim_gates_ext_preserves_unsat() {
        // x=1 = ITE(c=2, t=3, e=4) plus x→¬t, x→¬e, ¬x→t, ¬x→e: in either branch of c
        // the ITE forces x ≡ branch while the binaries force x ≠ branch. UNSAT.
        // The extra binaries also form equivalence definitions (x ≡ ¬t), so this covers
        // both new detectors' resolvent-restriction soundness on an UNSAT pivot.
        let clauses = vec![
            vec![1, -2, -3],
            vec![1, 2, -4],
            vec![-1, -2, 3],
            vec![-1, 2, 4],
            vec![-1, -3],
            vec![-1, -4],
            vec![1, 3],
            vec![1, 4],
        ];
        let run = |ext_on: bool| {
            let config = SolverConfig {
                full_bsr: false,
                ..SolverConfig::default()
            };
            let mut s = Solver::new_with_config(4, clauses.clone(), &config);
            s.elim_gates_ext = ext_on;
            s.inprocess_aggressive = true;
            for var in 2..=4 {
                s.frozen[var] = true;
            }
            s.solve()
        };
        assert!(!run(false), "baseline must report UNSAT");
        assert!(
            !run(true),
            "extended gate BVE must preserve UNSAT (must not drop a load-bearing resolvent)"
        );
    }

    #[test]
    fn def_gate_bve_eliminates_xor_defined_var_that_naive_bve_rejects() {
        // x=1 = a=2 XOR b=3 via the four XOR Tseitin clauses — a definition none of
        // the syntactic detectors (eq/AND-OR/ITE) can match — plus 2 pos and 2 neg
        // extra binaries. Naive non-taut resolvents = 4 + 4 + 4 = 12 > occ(8);
        // definition-aware = gate_pos×nongate_neg (4) + nongate_pos×gate_neg (4)
        // + gate×gate (all 4 tautological, skipped) = 8 <= 8.
        let clauses = vec![
            vec![1, 2, -3],
            vec![1, -2, 3],
            vec![-1, 2, 3],
            vec![-1, -2, -3],
            vec![1, 4],
            vec![1, 5],
            vec![-1, 6],
            vec![-1, 7],
        ];
        let run = |def_on: bool| {
            let config = SolverConfig {
                full_bsr: false,
                ..SolverConfig::default()
            };
            let mut s = Solver::new_with_config(7, clauses.clone(), &config);
            s.elim_gates_ext = true;
            s.elim_def = def_on;
            s.inprocess_aggressive = true;
            for var in 2..=7 {
                s.frozen[var] = true;
            }
            let sat = s.solve();
            (s, sat)
        };

        let (s_off, sat_off) = run(false);
        assert!(
            !s_off.eliminated[1],
            "syntactic-only BVE must reject x (XOR definition is not eq/AND-OR/ITE)"
        );
        assert_eq!(s_off.stats.preprocess_def_gate_eliminated_vars, 0);

        let (s_on, sat_on) = run(true);
        assert!(
            s_on.eliminated[1],
            "definition-aware BVE must eliminate x (8 restricted resolvents <= 8 occurrences)"
        );
        assert_eq!(s_on.stats.preprocess_def_gate_eliminated_vars, 1);
        assert!(s_on.stats.preprocess_def_gate_checks >= 1);
        assert!(s_on.stats.preprocess_def_gate_found >= 1);

        assert!(sat_off && sat_on, "instance is satisfiable in both configs");
        let model = s_on.sat_model.as_ref().expect("missing SAT model");
        for clause in &clauses {
            assert!(
                clause.iter().any(|&lit| {
                    let v = lit.unsigned_abs() as usize;
                    (lit > 0 && model[v] == TRUE) || (lit < 0 && model[v] == FALSE)
                }),
                "definition-eliminated model violates original clause {clause:?}"
            );
        }
    }

    #[test]
    fn elim_def_core_refinement_preserves_definition_and_runs() {
        // Same XOR-defined pivot as def_gate_bve_eliminates_xor_defined_var_...:
        // the definition core is exactly the four XOR clauses (any proper subset
        // is SAT), so refinement re-solves MUST reproduce the same two-sided core
        // at any SAT_ELIM_DEF_CORES level, and the elimination must survive.
        let clauses = vec![
            vec![1, 2, -3],
            vec![1, -2, 3],
            vec![-1, 2, 3],
            vec![-1, -2, -3],
            vec![1, 4],
            vec![1, 5],
            vec![-1, 6],
            vec![-1, 7],
        ];
        for cores in [1u32, 2, 4] {
            let config = SolverConfig {
                full_bsr: false,
                ..SolverConfig::default()
            };
            let mut s = Solver::new_with_config(7, clauses.clone(), &config);
            s.elim_gates_ext = true;
            s.elim_def = true;
            s.elim_def_cores = cores;
            s.inprocess_aggressive = true;
            for var in 2..=7 {
                s.frozen[var] = true;
            }
            let sat = s.solve();
            assert!(sat, "cores={cores}: instance is satisfiable");
            assert!(
                s.eliminated[1],
                "cores={cores}: definition-aware BVE must still eliminate x"
            );
            assert_eq!(s.stats.preprocess_def_gate_eliminated_vars, 1);
            if cores > 1 {
                assert!(
                    s.stats.preprocess_def_refine_solves >= 1,
                    "cores={cores}: refinement must have re-solved (4-clause core > 2)"
                );
            } else {
                assert_eq!(s.stats.preprocess_def_refine_solves, 0);
            }
            let model = s.sat_model.as_ref().expect("missing SAT model");
            for clause in &clauses {
                assert!(
                    clause.iter().any(|&lit| {
                        let v = lit.unsigned_abs() as usize;
                        (lit > 0 && model[v] == TRUE) || (lit < 0 && model[v] == FALSE)
                    }),
                    "cores={cores}: model violates original clause {clause:?}"
                );
            }
        }
    }

    #[test]
    fn elim_def_preserves_unsat() {
        // x=1 = a=2 XOR b=3, a ≡ b forces x false, while (x ∨ 4), (x ∨ ¬4) force x
        // true. UNSAT. The definition core is the four XOR clauses (two-sided);
        // gate-restricted elimination must keep the resolvents that carry the
        // contradiction (dropping a load-bearing resolvent would flip to SAT).
        let clauses = vec![
            vec![1, 2, -3],
            vec![1, -2, 3],
            vec![-1, 2, 3],
            vec![-1, -2, -3],
            vec![-2, 3],
            vec![2, -3],
            vec![1, 4],
            vec![1, -4],
        ];
        let run = |def_on: bool| {
            let config = SolverConfig {
                full_bsr: false,
                ..SolverConfig::default()
            };
            let mut s = Solver::new_with_config(4, clauses.clone(), &config);
            s.elim_gates_ext = true;
            s.elim_def = def_on;
            s.inprocess_aggressive = true;
            for var in 2..=4 {
                s.frozen[var] = true;
            }
            s.solve()
        };
        assert!(!run(false), "baseline must report UNSAT");
        assert!(
            !run(true),
            "definition-gate BVE must preserve UNSAT (must not drop a load-bearing resolvent)"
        );
    }

    #[test]
    fn elim_def_fuzz_against_brute_force() {
        let mut state: u64 = 0x243F6A8885A308D3;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };
        for round in 0..4000 {
            let nv = 4 + (next() % 5) as usize; // 4..=8 vars
            let nc = 6 + (next() % 18) as usize; // 6..=23 clauses
            let mut clauses: Vec<Vec<i32>> = Vec::new();
            for _ in 0..nc {
                let clen = 2 + (next() % 3) as usize; // 2..=4 lits
                let mut c: Vec<i32> = Vec::new();
                for _ in 0..clen {
                    let v = 1 + (next() % nv as u32) as i32;
                    let lit = if next() & 1 == 0 { v } else { -v };
                    if !c.contains(&lit) && !c.contains(&-lit) {
                        c.push(lit);
                    }
                }
                if c.len() >= 2 {
                    clauses.push(c);
                }
            }
            if clauses.is_empty() {
                continue;
            }
            let expect_sat = {
                let mut sat = false;
                'outer: for mask in 0u32..(1u32 << nv) {
                    for c in &clauses {
                        if !c.iter().any(|&d| {
                            let bit = (mask >> (d.unsigned_abs() - 1)) & 1 == 1;
                            if d > 0 { bit } else { !bit }
                        }) {
                            continue 'outer;
                        }
                    }
                    sat = true;
                    break;
                }
                sat
            };
            let config = SolverConfig {
                full_bsr: false,
                ..SolverConfig::default()
            };
            let mut s = Solver::new_with_config(nv, clauses.clone(), &config);
            s.elim_gates_ext = true;
            s.elim_def = true;
            s.inprocess_aggressive = true;
            let got = s.solve();
            assert_eq!(
                got, expect_sat,
                "round {round}: elim_def status mismatch (expect sat={expect_sat}) on {clauses:?}"
            );
            if got {
                let model = s.sat_model.as_ref().expect("missing model");
                for c in &clauses {
                    assert!(
                        c.iter().any(|&lit| {
                            let v = lit.unsigned_abs() as usize;
                            (lit > 0 && model[v] == TRUE) || (lit < 0 && model[v] == FALSE)
                        }),
                        "round {round}: model violates {c:?} in {clauses:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn elim_def_repro_684() {
        let clauses: Vec<Vec<i32>> = vec![
            vec![-1, -3, 5], vec![5, 1, 2], vec![-3, -1], vec![5, 1, -2],
            vec![-3, -1], vec![1, 5, -2], vec![5, 2, -3], vec![1, -3, 5],
            vec![2, -5, -3], vec![5, -3], vec![-4, -3], vec![5, 3],
            vec![-1, -2, 5, 3], vec![-4, -3], vec![-3, 5], vec![2, 1, 3, 5],
            vec![-3, -1, 4], vec![-3, 5], vec![-5, -2, -4], vec![5, 3, -2],
            vec![-5, -4, 2], vec![4, 3],
        ];
        let config = SolverConfig { full_bsr: false, ..SolverConfig::default() };
        let mut s = Solver::new_with_config(5, clauses.clone(), &config);
        s.elim_gates_ext = true;
        s.elim_def = true;
        s.inprocess_aggressive = true;
        let got = s.solve();
        assert!(got, "formula is SAT");
        let model = s.sat_model.as_ref().expect("missing model");
        for c in &clauses {
            assert!(
                c.iter().any(|&lit| {
                    let v = lit.unsigned_abs() as usize;
                    (lit > 0 && model[v] == TRUE) || (lit < 0 && model[v] == FALSE)
                }),
                "model violates {c:?}"
            );
        }
    }

    #[test]
    fn gate_bve_preserves_unsat() {
        // x<->(a∨b) with (¬x∨¬a),(¬x∨¬b),(a∨b): a∨b => x => ¬a∧¬b => contradicts a∨b. UNSAT.
        // No root units, so x=1 stays a live gate pivot. The classic gate-BVE bug turns this
        // UNSAT into SAT by dropping a needed resolvent — assert it does not.
        let clauses = vec![
            vec![1, -2],
            vec![1, -3],
            vec![-1, 2, 3],
            vec![-1, -2],
            vec![-1, -3],
            vec![2, 3],
        ];
        let run = |gate_on: bool| {
            let config = SolverConfig {
                gate_extract: gate_on,
                gate_bve: gate_on,
                full_bsr: false,
                ..SolverConfig::default()
            };
            let mut s = Solver::new_with_config(3, clauses.clone(), &config);
            s.frozen[2] = true;
            s.frozen[3] = true;
            s.solve()
        };
        assert!(!run(false), "baseline must report UNSAT");
        assert!(
            !run(true),
            "gate-aware BVE must preserve UNSAT (must not drop a load-bearing resolvent)"
        );
    }

    fn make_matching_pre_class() -> FormulaClass {
        FormulaClass {
            size_class: FormulaSizeClass::Large,
            kissat_small: false,
            kissat_bigbig: false,
            binary_fraction: 0.04,
            avg_clause_size: 3.0,
            variable_density: 150.0,
        }
    }

    fn make_nonmatching_pre_class() -> FormulaClass {
        FormulaClass {
            size_class: FormulaSizeClass::Medium,
            kissat_small: false,
            kissat_bigbig: false,
            binary_fraction: 0.5,
            avg_clause_size: 3.0,
            variable_density: 50.0,
        }
    }

    #[test]
    fn should_run_full_backward_subsumption_default_behavior_unchanged() {
        // Default config: full_bsr=true, bsr_formula_gate=false. Regardless of
        // pre_preprocess class, should_run_full_backward_subsumption returns true.
        let mut s = Solver::new(2, vec![]);
        assert!(s.full_bsr);
        assert!(!s.bsr_formula_gate);
        s.pre_preprocess_formula_class = make_matching_pre_class();
        assert!(s.should_run_full_backward_subsumption());
    }

    #[test]
    fn bsr_formula_gate_skips_bsr_on_large_low_binary_dense_formula() {
        let config = SolverConfig {
            bsr_formula_gate: true,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(2, vec![], &config);
        assert!(s.full_bsr);
        assert!(s.bsr_formula_gate);
        s.pre_preprocess_formula_class = make_matching_pre_class();
        assert!(!s.should_run_full_backward_subsumption());
    }

    #[test]
    fn bsr_formula_gate_runs_bsr_when_class_does_not_match() {
        let config = SolverConfig {
            bsr_formula_gate: true,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(2, vec![], &config);
        s.pre_preprocess_formula_class = make_nonmatching_pre_class();
        assert!(s.should_run_full_backward_subsumption());
    }

    #[test]
    fn bsr_formula_gate_off_runs_bsr_even_when_class_matches() {
        // Default (gate off): the matching class must NOT change behavior.
        let mut s = Solver::new(2, vec![]);
        assert!(!s.bsr_formula_gate);
        s.pre_preprocess_formula_class = make_matching_pre_class();
        assert!(s.should_run_full_backward_subsumption());
    }

    #[test]
    fn bsr_formula_gate_respects_full_bsr_off() {
        // If full_bsr is already off, the gate is a no-op (return false either way).
        let config = SolverConfig {
            full_bsr: false,
            bsr_formula_gate: true,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(2, vec![], &config);
        s.pre_preprocess_formula_class = make_matching_pre_class();
        assert!(!s.should_run_full_backward_subsumption());
        s.pre_preprocess_formula_class = make_nonmatching_pre_class();
        assert!(!s.should_run_full_backward_subsumption());
    }

    #[test]
    fn bsr_formula_gate_requires_all_three_conditions() {
        let config = SolverConfig {
            bsr_formula_gate: true,
            ..SolverConfig::default()
        };
        let mut s = Solver::new_with_config(2, vec![], &config);

        // Large + low binary but density too low → run BSR.
        s.pre_preprocess_formula_class = FormulaClass {
            size_class: FormulaSizeClass::Large,
            binary_fraction: 0.01,
            variable_density: 50.0,
            ..FormulaClass::default()
        };
        assert!(s.should_run_full_backward_subsumption());

        // Large + dense but binary too high → run BSR.
        s.pre_preprocess_formula_class = FormulaClass {
            size_class: FormulaSizeClass::Large,
            binary_fraction: 0.5,
            variable_density: 200.0,
            ..FormulaClass::default()
        };
        assert!(s.should_run_full_backward_subsumption());

        // Boundary: binary_fraction=0.05 (strictly less than required) → run BSR.
        s.pre_preprocess_formula_class = FormulaClass {
            size_class: FormulaSizeClass::Large,
            binary_fraction: 0.05,
            variable_density: 200.0,
            ..FormulaClass::default()
        };
        assert!(s.should_run_full_backward_subsumption());

        // Boundary: variable_density=100.0 (strictly greater than required) → run BSR.
        s.pre_preprocess_formula_class = FormulaClass {
            size_class: FormulaSizeClass::Large,
            binary_fraction: 0.01,
            variable_density: 100.0,
            ..FormulaClass::default()
        };
        assert!(s.should_run_full_backward_subsumption());
    }
}
