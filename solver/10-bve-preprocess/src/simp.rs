use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

use super::*;

enum SubsumptionCandidate {
    Clause(usize),
    RootUnit(i32),
}

enum SubsumptionOutcome {
    None,
    Subsumed,
    Strengthen(i32),
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
    }

    fn occurrence_cost(&self, var: usize) -> u64 {
        let pos = lit_to_index(var as i32);
        let neg = lit_to_index(-(var as i32));
        (self.n_occ[pos] as u64) * (self.n_occ[neg] as u64)
    }

    fn should_run_full_backward_subsumption(&self) -> bool {
        let vars = self.variable_count();
        vars > 0 && vars <= 10_000 && self.original_clause_ids.len() >= vars.saturating_mul(50)
    }

    fn build_occurrence_index(&mut self) {
        let num_vars = self.variable_count();
        self.occurs.clear();
        self.occurs.resize_with(num_vars + 1, Vec::new);
        self.occurs_dirty.clear();
        self.occurs_dirty.resize(num_vars + 1, false);
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
            self.occurs[var].push(clause_idx);
            self.n_occ[lit_to_index(lit)] += 1;
        }
    }

    fn clean_occurs(&mut self, var: usize) {
        if var >= self.occurs.len() || !self.occurs_dirty[var] {
            return;
        }

        let old = std::mem::take(&mut self.occurs[var]);
        let mut cleaned = Vec::with_capacity(old.len());
        for clause_idx in old {
            if clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx) {
                cleaned.push(clause_idx);
            }
        }
        self.occurs[var] = cleaned;
        self.occurs_dirty[var] = false;
    }

    fn enqueue_subsumption_clause(
        &self,
        queue: &mut VecDeque<SubsumptionCandidate>,
        queued: &mut Vec<bool>,
        clause_idx: usize,
    ) {
        if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
            return;
        }
        if clause_idx >= queued.len() {
            queued.resize(clause_idx + 1, false);
        }
        if queued[clause_idx] {
            return;
        }
        queued[clause_idx] = true;
        queue.push_back(SubsumptionCandidate::Clause(clause_idx));
    }

    fn gather_touched_clauses(
        &mut self,
        touched: &[usize],
        queue: &mut VecDeque<SubsumptionCandidate>,
        queued: &mut Vec<bool>,
    ) {
        for &var in touched {
            if var == 0 || var >= self.occurs.len() {
                continue;
            }
            self.clean_occurs(var);
            let clauses = self.occurs[var].clone();
            for clause_idx in clauses {
                self.enqueue_subsumption_clause(queue, queued, clause_idx);
            }
        }
    }

    fn mark_occurs_dirty_for_clause(&mut self, clause_idx: usize, touched: &mut Vec<usize>) {
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
            touched.push(var);
        }
    }

    fn remove_original_clause_preprocess(&mut self, clause_idx: usize, touched: &mut Vec<usize>) {
        if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
            return;
        }
        debug_assert!(!self.clause_is_learnt(clause_idx));

        if self.clause_locked(clause_idx) {
            let implied_lit = self.clause_lit(clause_idx, 0);
            let var = implied_lit.unsigned_abs() as usize;
            self.reason[var] = NO_REASON;
        }

        self.mark_occurs_dirty_for_clause(clause_idx, touched);
        let clause_len = self.clause_len(clause_idx);
        self.detach_clause(clause_idx);
        self.original_literals = self.original_literals.saturating_sub(clause_len);
        self.deleted_clause_words += self.clause_word_len(clause_idx);
        self.clause_set_deleted(clause_idx, true);
        self.stats.deleted_clauses += 1;
    }

    fn subsumption_relation(
        &self,
        driver: &[i32],
        driver_abstraction: u64,
        candidate_idx: usize,
        marks: &mut [u32],
        signs: &mut [u8],
        stamp: u32,
    ) -> SubsumptionOutcome {
        if candidate_idx >= self.arena.len() || self.clause_is_deleted(candidate_idx) {
            return SubsumptionOutcome::None;
        }
        let candidate_len = self.clause_len(candidate_idx);
        if driver.len() > candidate_len {
            return SubsumptionOutcome::None;
        }
        if (driver_abstraction & !self.original_clause_abstraction(candidate_idx)) != 0 {
            return SubsumptionOutcome::None;
        }

        for candidate_pos in 0..candidate_len {
            let candidate_lit = self.clause_lit(candidate_idx, candidate_pos);
            let var = candidate_lit.unsigned_abs() as usize;
            debug_assert!(var < marks.len());
            if marks[var] != stamp {
                marks[var] = stamp;
                signs[var] = 0;
            }
            signs[var] |= if candidate_lit > 0 { 1 } else { 2 };
        }

        let mut remove_lit = None;
        for &lit in driver {
            let var = lit.unsigned_abs() as usize;
            if var >= marks.len() || marks[var] != stamp {
                return SubsumptionOutcome::None;
            }

            let same_sign = if lit > 0 { 1 } else { 2 };
            if signs[var] & same_sign != 0 {
                continue;
            }

            let complement_sign = if lit > 0 { 2 } else { 1 };
            if signs[var] & complement_sign != 0 && remove_lit.is_none() {
                remove_lit = Some(-lit);
                continue;
            }

            return SubsumptionOutcome::None;
        }

        match remove_lit {
            Some(lit) => SubsumptionOutcome::Strengthen(lit),
            None => SubsumptionOutcome::Subsumed,
        }
    }

    fn strengthen_original_clause_preprocess(
        &mut self,
        clause_idx: usize,
        remove_lit: i32,
        proof_log: &mut ProofLog,
        touched: &mut Vec<usize>,
        queue: &mut VecDeque<SubsumptionCandidate>,
        queued: &mut Vec<bool>,
    ) -> bool {
        if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
            return true;
        }

        let mut strengthened = Vec::with_capacity(self.clause_len(clause_idx).saturating_sub(1));
        let mut removed = false;
        for lit_pos in 0..self.clause_len(clause_idx) {
            let lit = self.clause_lit(clause_idx, lit_pos);
            if lit == remove_lit && !removed {
                removed = true;
                continue;
            }
            strengthened.push(lit);
        }

        if !removed {
            return true;
        }

        self.remove_original_clause_preprocess(clause_idx, touched);
        if clause_idx < queued.len() {
            queued[clause_idx] = false;
        }

        self.stats.preprocess_strengthened_clauses += 1;
        match self.add_original_clause_from_slice(&strengthened, proof_log, true, touched) {
            OriginalClauseInsertResult::Allocated(new_idx) => {
                self.enqueue_subsumption_clause(queue, queued, new_idx);
                true
            }
            OriginalClauseInsertResult::Unit | OriginalClauseInsertResult::Skipped => true,
            OriginalClauseInsertResult::Unsat => false,
        }
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

    fn add_original_clause_from_slice(
        &mut self,
        clause: &[i32],
        proof_log: &mut ProofLog,
        log_proof: bool,
        touched: &mut Vec<usize>,
    ) -> OriginalClauseInsertResult {
        if !self.solver_ok {
            return OriginalClauseInsertResult::Unsat;
        }

        let Some(normalized) = self.normalize_original_clause(clause) else {
            return OriginalClauseInsertResult::Skipped;
        };

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
            if !self.enqueue(normalized[0], NO_REASON) || self.propagate().is_some() {
                self.solver_ok = false;
                return OriginalClauseInsertResult::Unsat;
            }
            return OriginalClauseInsertResult::Unit;
        }

        let clause_idx = self.arena.len();
        self.arena
            .push(clause_make_header(normalized.len(), false, false, 0, false));
        self.arena
            .extend(normalized.iter().copied().map(lit_to_word));
        self.original_clause_ids.push(clause_idx);
        self.original_literals += normalized.len();
        self.attach_clause(clause_idx, false);

        if self.use_simplification {
            if !self.clause_abstraction.is_empty() {
                self.set_original_clause_abstraction(
                    clause_idx,
                    clause_abstraction_from_lits(&normalized),
                );
            }
            self.index_original_clause(clause_idx);
            for &lit in &normalized {
                touched.push(lit.unsigned_abs() as usize);
            }
        }

        OriginalClauseInsertResult::Allocated(clause_idx)
    }

    fn backward_subsumption_check(
        &mut self,
        seed_all_clauses: bool,
        seed_touched: Vec<usize>,
        proof_log: &mut ProofLog,
    ) -> Option<Vec<usize>> {
        let mut queue = VecDeque::new();
        let mut queued = vec![false; self.arena.len()];
        let mut touched = seed_touched;
        self.ensure_original_clause_abstractions();
        let mut relation_marks = vec![0u32; self.assignment.len()];
        let mut relation_signs = vec![0u8; self.assignment.len()];
        let mut relation_stamp = 1u32;

        if seed_all_clauses {
            let original_clause_ids = self.original_clause_ids.clone();
            for clause_idx in original_clause_ids {
                self.enqueue_subsumption_clause(&mut queue, &mut queued, clause_idx);
            }
        }
        self.gather_touched_clauses(&touched, &mut queue, &mut queued);

        while !queue.is_empty() || self.bwdsub_assigns < self.trail.len() {
            let candidate = if let Some(candidate) = queue.pop_front() {
                candidate
            } else {
                let lit = self.trail[self.bwdsub_assigns];
                self.bwdsub_assigns += 1;
                SubsumptionCandidate::RootUnit(lit)
            };

            let (driver, driver_clause_idx) = match candidate {
                SubsumptionCandidate::Clause(clause_idx) => {
                    if clause_idx < queued.len() {
                        queued[clause_idx] = false;
                    }
                    if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
                        continue;
                    }
                    (self.clause_slice(clause_idx).to_vec(), Some(clause_idx))
                }
                SubsumptionCandidate::RootUnit(lit) => (vec![lit], None),
            };

            if driver.is_empty() {
                continue;
            }
            let driver_abstraction = clause_abstraction_from_lits(&driver);

            let mut best_var = driver[0].unsigned_abs() as usize;
            self.clean_occurs(best_var);
            for &lit in &driver[1..] {
                let var = lit.unsigned_abs() as usize;
                self.clean_occurs(var);
                if var < self.occurs.len()
                    && best_var < self.occurs.len()
                    && self.occurs[var].len() < self.occurs[best_var].len()
                {
                    best_var = var;
                }
            }

            if best_var >= self.occurs.len() {
                continue;
            }

            let candidates = self.occurs[best_var].clone();
            for candidate_idx in candidates {
                if Some(candidate_idx) == driver_clause_idx {
                    continue;
                }
                if candidate_idx >= self.arena.len() || self.clause_is_deleted(candidate_idx) {
                    continue;
                }
                if self.subsumption_lim >= 0
                    && self.clause_len(candidate_idx) as isize >= self.subsumption_lim
                {
                    continue;
                }

                relation_stamp = relation_stamp.wrapping_add(1);
                if relation_stamp == 0 {
                    relation_marks.fill(0);
                    relation_stamp = 1;
                }

                match self.subsumption_relation(
                    &driver,
                    driver_abstraction,
                    candidate_idx,
                    &mut relation_marks,
                    &mut relation_signs,
                    relation_stamp,
                ) {
                    SubsumptionOutcome::None => {}
                    SubsumptionOutcome::Subsumed => {
                        self.remove_original_clause_preprocess(candidate_idx, &mut touched);
                        if candidate_idx < queued.len() {
                            queued[candidate_idx] = false;
                        }
                        self.stats.preprocess_subsumed_clauses += 1;
                    }
                    SubsumptionOutcome::Strengthen(remove_lit) => {
                        if !self.strengthen_original_clause_preprocess(
                            candidate_idx,
                            remove_lit,
                            proof_log,
                            &mut touched,
                            &mut queue,
                            &mut queued,
                        ) {
                            return None;
                        }
                    }
                }
            }
        }

        Some(touched)
    }

    fn merge_size_only(&self, lhs: &[i32], rhs: &[i32], var: usize) -> Option<usize> {
        let mut size = 0usize;
        for &lit in lhs {
            if lit.unsigned_abs() as usize != var {
                size += 1;
            }
        }

        'rhs_lits: for &lit in rhs {
            if lit.unsigned_abs() as usize == var {
                continue;
            }
            for &existing in lhs {
                if existing.unsigned_abs() == lit.unsigned_abs() {
                    if existing == -lit {
                        return None;
                    }
                    continue 'rhs_lits;
                }
            }
            size += 1;
        }
        Some(size)
    }

    fn merge_into_vec(&self, lhs: &[i32], rhs: &[i32], var: usize, out: &mut Vec<i32>) -> bool {
        out.clear();
        for &lit in lhs {
            if lit.unsigned_abs() as usize != var {
                out.push(lit);
            }
        }

        'rhs_lits: for &lit in rhs {
            if lit.unsigned_abs() as usize == var {
                continue;
            }
            for &existing in out.iter() {
                if existing.unsigned_abs() == lit.unsigned_abs() {
                    if existing == -lit {
                        out.clear();
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

    fn push_elim_clause(&mut self, var: usize, clause: &[i32]) {
        let start = self.elim_clauses.len();
        let mut var_pos = None;
        for &lit in clause {
            if lit.unsigned_abs() as usize == var {
                var_pos = Some(self.elim_clauses.len());
            }
            self.elim_clauses.push(lit);
        }

        let var_pos = var_pos.expect("elimination extension clause missing eliminated variable");
        self.elim_clauses.swap(start, var_pos);
        self.elim_clauses.push(clause.len() as i32);
    }

    fn try_eliminate_var(&mut self, var: usize, proof_log: &mut ProofLog) -> Option<Vec<usize>> {
        self.clean_occurs(var);
        let occurrence_ids = self.occurs.get(var)?.clone();
        if occurrence_ids.is_empty() {
            return None;
        }

        let mut pos_clauses = Vec::new();
        let mut neg_clauses = Vec::new();
        for clause_idx in occurrence_ids {
            if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
                continue;
            }

            let clause = self.clause_slice(clause_idx).to_vec();
            if clause.iter().any(|&lit| lit == var as i32) {
                pos_clauses.push((clause_idx, clause));
            } else if clause.iter().any(|&lit| lit == -(var as i32)) {
                neg_clauses.push((clause_idx, clause));
            }
        }

        let occurrence_count = pos_clauses.len() + neg_clauses.len();
        if occurrence_count == 0 {
            return None;
        }

        let mut resolvent_count = 0isize;
        for (_, pos_clause) in &pos_clauses {
            for (_, neg_clause) in &neg_clauses {
                let Some(size) = self.merge_size_only(pos_clause, neg_clause, var) else {
                    continue;
                };
                resolvent_count += 1;
                if resolvent_count > occurrence_count as isize + self.bve_grow {
                    return None;
                }
                if self.bve_clause_limit >= 0 && size as isize > self.bve_clause_limit {
                    return None;
                }
            }
        }

        if pos_clauses.len() > neg_clauses.len() {
            for (_, clause) in &neg_clauses {
                self.push_elim_clause(var, clause);
            }
            self.push_elim_unit(var as i32);
        } else {
            for (_, clause) in &pos_clauses {
                self.push_elim_clause(var, clause);
            }
            self.push_elim_unit(-(var as i32));
        }

        self.eliminated[var] = true;
        self.decision_var[var] = false;
        self.branch_heap_remove(var);
        self.stats.preprocess_eliminated_vars += 1;

        let mut touched = Vec::new();
        for (clause_idx, _) in pos_clauses.iter().chain(neg_clauses.iter()) {
            self.remove_original_clause_preprocess(*clause_idx, &mut touched);
        }
        if var < self.occurs.len() {
            self.occurs[var].clear();
            self.occurs_dirty[var] = false;
        }

        for (_, pos_clause) in &pos_clauses {
            for (_, neg_clause) in &neg_clauses {
                let mut resolvent = std::mem::take(&mut self.scratch_preprocess_clause);
                let keep = self.merge_into_vec(pos_clause, neg_clause, var, &mut resolvent);
                if keep {
                    self.stats.preprocess_resolvents += 1;
                    let result = self.add_original_clause_from_slice(
                        &resolvent,
                        proof_log,
                        true,
                        &mut touched,
                    );
                    if result == OriginalClauseInsertResult::Unsat {
                        self.scratch_preprocess_clause = resolvent;
                        return Some(touched);
                    }
                }
                self.scratch_preprocess_clause = resolvent;
            }
        }

        Some(touched)
    }

    pub(super) fn eliminate(&mut self, turn_off_elim: bool, proof_log: &mut ProofLog) -> bool {
        if !self.solver_ok {
            return false;
        }
        if !self.use_simplification || !self.use_elim {
            return self.simplify();
        }
        if !self.simplify() {
            self.solver_ok = false;
            return false;
        }

        self.build_occurrence_index();
        self.bwdsub_assigns = 0;
        let use_full_backward_subsumption = self.should_run_full_backward_subsumption();
        if use_full_backward_subsumption {
            if self
                .backward_subsumption_check(true, Vec::new(), proof_log)
                .is_none()
            {
                self.solver_ok = false;
                return false;
            }
        }

        if use_full_backward_subsumption {
            let mut heap = BinaryHeap::new();
            let mut heap_versions = vec![0u32; self.assignment.len()];
            for var in 1..=self.variable_count() {
                if self.preprocessing_candidate(var) {
                    heap.push(Reverse((
                        self.occurrence_cost(var),
                        var,
                        heap_versions[var],
                    )));
                }
            }

            while let Some(Reverse((_, var, version))) = heap.pop() {
                if var >= heap_versions.len() || version != heap_versions[var] {
                    continue;
                }
                if !self.preprocessing_candidate(var) {
                    continue;
                }
                self.clean_occurs(var);
                if self.occurs[var].is_empty() {
                    continue;
                }

                let Some(touched) = self.try_eliminate_var(var, proof_log) else {
                    continue;
                };
                if !self.solver_ok {
                    break;
                }

                let Some(touched) = self.backward_subsumption_check(false, touched, proof_log)
                else {
                    self.solver_ok = false;
                    break;
                };

                for touched_var in touched {
                    if touched_var < heap_versions.len()
                        && self.preprocessing_candidate(touched_var)
                    {
                        heap_versions[touched_var] = heap_versions[touched_var].wrapping_add(1);
                        heap.push(Reverse((
                            self.occurrence_cost(touched_var),
                            touched_var,
                            heap_versions[touched_var],
                        )));
                    }
                }
            }
        } else {
            let mut candidates: Vec<usize> = (1..=self.variable_count())
                .filter(|&var| self.preprocessing_candidate(var))
                .collect();
            candidates.sort_unstable_by(|&lhs, &rhs| {
                self.occurrence_cost(lhs)
                    .cmp(&self.occurrence_cost(rhs))
                    .then_with(|| lhs.cmp(&rhs))
            });

            let mut queue: VecDeque<usize> = candidates.into();
            let mut queued = vec![false; self.assignment.len()];
            for &var in &queue {
                queued[var] = true;
            }

            while let Some(var) = queue.pop_front() {
                queued[var] = false;
                if !self.preprocessing_candidate(var) {
                    continue;
                }
                self.clean_occurs(var);
                if self.occurs[var].is_empty() {
                    continue;
                }

                let Some(touched) = self.try_eliminate_var(var, proof_log) else {
                    continue;
                };
                if !self.solver_ok {
                    break;
                }

                for touched_var in touched {
                    if touched_var < queued.len()
                        && !queued[touched_var]
                        && self.preprocessing_candidate(touched_var)
                    {
                        queued[touched_var] = true;
                        queue.push_back(touched_var);
                    }
                }
            }
        }

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
            self.n_occ.clear();
            self.use_simplification = false;
            self.rebuild_branch_queue();
            self.garbage_collect();
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
        for var in 1..model.len() {
            if model[var] == UNASSIGNED && !self.eliminated[var] {
                model[var] = TRUE;
            }
        }
        self.extend_model_snapshot(&mut model);
        for var in 1..model.len() {
            if model[var] == UNASSIGNED {
                model[var] = TRUE;
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

    #[test]
    fn backward_subsumption_removes_subsumed_original_clause() {
        let mut s = Solver::new(3, vec![vec![1, 2], vec![1, 2, 3]]);
        let mut proof = ProofLog::disabled();
        s.build_occurrence_index();

        assert!(s
            .backward_subsumption_check(true, Vec::new(), &mut proof)
            .is_some());

        assert_eq!(live_original_clauses(&s), vec![vec![1, 2]]);
        assert_eq!(s.stats.preprocess_subsumed_clauses, 1);
    }

    #[test]
    fn backward_subsumption_resolution_strengthens_original_clause() {
        let mut s = Solver::new(3, vec![vec![1, 2], vec![1, -2, 3]]);
        let mut proof = ProofLog::disabled();
        s.build_occurrence_index();

        assert!(s
            .backward_subsumption_check(true, Vec::new(), &mut proof)
            .is_some());

        assert_eq!(live_original_clauses(&s), vec![vec![1, 2], vec![1, 3]]);
        assert_eq!(s.stats.preprocess_strengthened_clauses, 1);
    }

    #[test]
    fn root_assignment_subsumption_trims_false_literal() {
        let mut s = Solver::new(3, vec![vec![1], vec![-1, 2, 3]]);
        let mut proof = ProofLog::disabled();
        assert!(s.enqueue_root_units());
        assert_eq!(s.propagate(), None);
        s.build_occurrence_index();

        assert!(s
            .backward_subsumption_check(false, Vec::new(), &mut proof)
            .is_some());

        assert!(live_original_clauses(&s).contains(&vec![2, 3]));
        assert_eq!(s.stats.preprocess_strengthened_clauses, 1);
    }
}
