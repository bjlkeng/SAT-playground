use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

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

enum PreprocessBudgetKind {
    Resolution,
    Tick,
}

const MARKED_SUBSUMPTION_MIN_PRODUCT: usize = 32;

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
            (Some(&pos_count), Some(&neg_count)) => pos_count.saturating_add(neg_count),
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
        for clause_idx in original_clause_ids {
            if !self.clause_is_deleted(clause_idx) {
                self.index_original_clause(clause_idx);
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
        );
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
        );
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
        for &lit in clause {
            let var = lit.unsigned_abs() as usize;
            if var == 0 || var >= self.assignment.len() || self.eliminated[var] {
                return Some(Vec::new());
            }

            match self.lit_value(lit) {
                TRUE => return None,
                FALSE => {}
                UNASSIGNED => normalized.push(lit),
                _ => unreachable!(),
            }
        }

        normalized.sort_unstable_by(|&lhs, &rhs| {
            lhs.unsigned_abs()
                .cmp(&rhs.unsigned_abs())
                .then_with(|| lhs.cmp(&rhs))
        });

        let mut write = 0usize;
        let mut prev_lit = 0i32;
        for read in 0..normalized.len() {
            let lit = normalized[read];
            if write > 0 {
                if lit == prev_lit {
                    continue;
                }
                if lit == -prev_lit {
                    return None;
                }
            }
            normalized[write] = lit;
            write += 1;
            prev_lit = lit;
        }
        normalized.truncate(write);
        Some(normalized)
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
        normalized: Vec<i32>,
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

        if log_proof {
            proof_log.record_clause(&normalized);
        }

        if normalized.len() == 1 {
            if !self.enqueue(normalized[0], ReasonRef::None) || self.propagate().is_some() {
                self.solver_ok = false;
                return OriginalClauseInsertResult::Unsat;
            }
            return OriginalClauseInsertResult::Unit;
        }

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
            let abstraction = clause_abstraction_from_lits(&normalized);
            self.arena.push(abstraction as u32);
        }
        self.original_clause_ids.push(clause_idx);
        self.original_literals += normalized.len();
        self.attach_clause(clause_idx, false);

        if self.use_simplification {
            if !store_abstraction_inline && !self.clause_abstraction.is_empty() {
                self.set_original_clause_abstraction(
                    clause_idx,
                    clause_abstraction_from_lits(&normalized),
                );
            }
            self.index_original_clause(clause_idx);
            for &lit in &normalized {
                Self::touch_preprocess_var_and_bsr(
                    touched,
                    touched_flags,
                    bsr_touched,
                    bsr_touched_flags,
                    lit.unsigned_abs() as usize,
                );
            }
        }

        if let Some(queue) = subsumption_work.as_mut() {
            self.enqueue_subsumption_clause(queue, clause_idx);
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

        let Some(normalized) = self.normalize_original_clause(clause) else {
            return OriginalClauseInsertResult::Skipped;
        };

        self.add_normalized_original_clause(
            normalized,
            proof_log,
            log_proof,
            touched,
            touched_flags,
            bsr_touched,
            bsr_touched_flags,
            subsumption_work,
        )
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
                normalized,
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
                normalized,
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
        if TRACE {
            self.stats.bsr_runs += 1;
        }
        if seed_all_clauses {
            let mut original_clause_ids = self.original_clause_ids.clone();
            original_clause_ids.sort_by_key(|&clause_idx| {
                if clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx) {
                    (self.clause_len(clause_idx), clause_idx)
                } else {
                    (usize::MAX, clause_idx)
                }
            });
            for clause_idx in original_clause_ids {
                self.enqueue_subsumption_clause(queue, clause_idx);
                if TRACE {
                    self.stats.bsr_seeded_clauses += 1;
                }
            }
        }

        let mut relation_marks = Vec::new();
        let mut relation_mark_stamp = 0u32;

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
                    &mut relation_marks,
                    &mut relation_mark_stamp,
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

        let mut resolvent_count = 0isize;
        let mut resolvent_lits = Vec::new();
        let mut resolvent_ranges = Vec::new();
        for &pos_clause_idx in &pos_clauses {
            for &neg_clause_idx in &neg_clauses {
                if (self.eliminate_resolution_budget != 0 || self.eliminate_ticks_budget != 0)
                    && !self.consume_eliminate_resolution_attempt()
                {
                    return false;
                }
                let start = resolvent_lits.len();
                if !self.append_resolvent_into_vec(
                    pos_clause_idx,
                    neg_clause_idx,
                    var,
                    &mut resolvent_lits,
                ) {
                    continue;
                }
                resolvent_count += 1;
                if resolvent_count > occurrence_count as isize + self.bve_grow {
                    resolvent_lits.truncate(start);
                    return false;
                }
                let size = resolvent_lits.len() - start;
                if self.bve_clause_limit >= 0 && size as isize > self.bve_clause_limit {
                    resolvent_lits.truncate(start);
                    return false;
                }
                resolvent_ranges.push((start, size));
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

        for &clause_idx in pos_clauses.iter().chain(neg_clauses.iter()) {
            proof_log.record_deletion(self.clause_slice(clause_idx));
        }

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

        let run_full_backward_subsumption = self.should_run_full_backward_subsumption();
        self.build_occurrence_index();
        self.bwdsub_assigns = 0;
        let mut queue = VecDeque::new();
        let mut touched = Vec::new();
        let mut touched_flags = vec![false; self.assignment.len()];
        let mut bsr_touched = Vec::new();
        let mut bsr_touched_flags = vec![false; self.assignment.len()];
        let mut heap = BinaryHeap::new();
        let mut heap_versions = vec![0u32; self.assignment.len()];

        for (var, &version) in heap_versions
            .iter()
            .enumerate()
            .take(self.variable_count() + 1)
            .skip(1)
        {
            if self.preprocessing_candidate(var) {
                heap.push(Reverse((self.occurrence_cost(var), var, version)));
            }
        }

        if run_full_backward_subsumption
            && !self.preprocess_bsr_budget_exhausted
            && !self.backward_subsumption_check_dynamic(
                true,
                &mut queue,
                &mut touched,
                &mut touched_flags,
                &mut bsr_touched,
                &mut bsr_touched_flags,
                proof_log,
            )
        {
            self.clear_subsumption_queue_marks(&mut queue);
            self.solver_ok = false;
            return false;
        }

        while self.solver_ok
            && !self.preprocess_budget_exhausted
            && (!touched.is_empty()
                || !bsr_touched.is_empty()
                || (run_full_backward_subsumption
                    && !self.preprocess_bsr_budget_exhausted
                    && (!queue.is_empty() || self.bwdsub_assigns < self.trail.len()))
                || !heap.is_empty())
        {
            if !touched.is_empty() || !bsr_touched.is_empty() {
                self.gather_touched_clauses(
                    &mut touched,
                    &mut touched_flags,
                    &mut bsr_touched,
                    &mut bsr_touched_flags,
                    &mut queue,
                    &mut heap,
                    &mut heap_versions,
                    run_full_backward_subsumption && !self.preprocess_bsr_budget_exhausted,
                );
                continue;
            }

            if run_full_backward_subsumption
                && !self.preprocess_bsr_budget_exhausted
                && (!queue.is_empty() || self.bwdsub_assigns < self.trail.len())
                && !self.backward_subsumption_check_dynamic(
                    false,
                    &mut queue,
                    &mut touched,
                    &mut touched_flags,
                    &mut bsr_touched,
                    &mut bsr_touched_flags,
                    proof_log,
                )
            {
                self.solver_ok = false;
                break;
            }
            if self.preprocess_budget_exhausted {
                break;
            }

            while let Some(Reverse((_, var, version))) = heap.pop() {
                if var >= heap_versions.len() || version != heap_versions[var] {
                    continue;
                }
                if !self.preprocessing_candidate(var) {
                    continue;
                }
                self.clean_occurs_dynamic(var);
                if self.occurs[var].is_empty() {
                    continue;
                }

                let eliminated = self.try_eliminate_var(
                    var,
                    proof_log,
                    run_full_backward_subsumption && !self.preprocess_bsr_budget_exhausted,
                    &mut queue,
                    &mut touched,
                    &mut touched_flags,
                    &mut bsr_touched,
                    &mut bsr_touched_flags,
                );
                if !eliminated {
                    if self.preprocess_budget_exhausted {
                        break;
                    }
                    continue;
                }

                if run_full_backward_subsumption
                    && !self.bsr_drain_batched
                    && !self.preprocess_bsr_budget_exhausted
                    && (!queue.is_empty() || self.bwdsub_assigns < self.trail.len())
                    && !self.backward_subsumption_check_dynamic(
                        false,
                        &mut queue,
                        &mut touched,
                        &mut touched_flags,
                        &mut bsr_touched,
                        &mut bsr_touched_flags,
                        proof_log,
                    )
                {
                    self.solver_ok = false;
                    break;
                }
                if self.preprocess_budget_exhausted {
                    break;
                }
            }
        }

        self.clear_subsumption_queue_marks(&mut queue);

        let original_clause_ids = std::mem::take(&mut self.original_clause_ids);
        self.original_clause_ids = original_clause_ids
            .into_iter()
            .filter(|&clause_idx| {
                clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx)
            })
            .collect();

        if turn_off_elim {
            self.occurs.clear();
            self.occurs_dirty.clear();
            self.occurs_membership_dirty.clear();
            self.n_occ.clear();
            self.use_simplification = false;
            self.inline_original_abstractions = false;
            self.rebuild_branch_queue();
            self.garbage_collect();
            self.clause_abstraction.clear();
            self.clause_abstraction.shrink_to_fit();
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
            .copied()
            .filter(|&clause_idx| !s.clause_is_deleted(clause_idx))
            .map(|clause_idx| s.clause_slice(clause_idx).to_vec())
            .collect();
        clauses.sort();
        clauses
    }

    fn assert_no_subsumption_queue_marks(s: &Solver) {
        for &clause_idx in &s.original_clause_ids {
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
        let candidate = s.original_clause_ids[0];
        let driver_idx = s.original_clause_ids[1];
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
        let candidate = s.original_clause_ids[0];
        let driver_idx = s.original_clause_ids[1];
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
        let candidate = s.original_clause_ids[0];
        let driver_idx = s.original_clause_ids[1];
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
        let clause_idx = s.original_clause_ids[0];
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

        let driver = s.original_clause_ids[0];
        let mut touched = Vec::new();
        let mut touched_flags = vec![false; s.assignment.len()];
        for clause_idx in s.original_clause_ids[5..].to_vec() {
            s.remove_original_clause_preprocess(clause_idx, &mut touched, &mut touched_flags);
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
        let lhs = s.original_clause_ids[0];
        let rhs = s.original_clause_ids[1];
        let mut resolvent = Vec::new();

        assert!(s.append_resolvent_into_vec(lhs, rhs, 1, &mut resolvent));
        assert_eq!(resolvent, vec![2, 5, 7, 8]);
    }

    #[test]
    fn bve_sorted_resolvent_merge_rejects_tautology() {
        let mut s = Solver::new(4, vec![vec![1, 2, 4], vec![-1, -2, 3]]);
        s.build_occurrence_index();
        let lhs = s.original_clause_ids[0];
        let rhs = s.original_clause_ids[1];
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
