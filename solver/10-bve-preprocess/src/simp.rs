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
        match std::env::var("SAT_FULL_BSR") {
            Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => return true,
                "0" | "false" | "no" | "off" => return false,
                other => {
                    eprintln!("Invalid SAT_FULL_BSR={other}; expected on/off");
                    std::process::exit(2);
                }
            },
            Err(_) => {}
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

    fn clean_occurs(&mut self, var: usize) {
        if var >= self.occurs.len()
            || (!self.occurs_dirty[var] && !self.occurs_membership_dirty[var])
        {
            return;
        }

        let arena = &self.arena;
        let check_membership = self.occurs_membership_dirty[var];
        let occurs = &mut self.occurs[var];
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
        self.occurs_dirty[var] = false;
        self.occurs_membership_dirty[var] = false;
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

    fn gather_touched_clauses(
        &mut self,
        touched: &mut Vec<usize>,
        touched_flags: &mut [bool],
        queue: &mut VecDeque<SubsumptionCandidate>,
        heap: &mut BinaryHeap<Reverse<(u64, usize, u32)>>,
        heap_versions: &mut [u32],
    ) {
        let vars = std::mem::take(touched);
        for var in vars {
            if var < touched_flags.len() {
                touched_flags[var] = false;
            }
            if var == 0 || var >= self.occurs.len() {
                continue;
            }
            self.clean_occurs(var);
            let mut scan_pos = 0usize;
            while scan_pos < self.occurs[var].len() {
                let clause_idx = self.occurs[var][scan_pos] as usize;
                scan_pos += 1;
                self.enqueue_subsumption_clause(queue, clause_idx);
            }
            if var < heap_versions.len() && self.preprocessing_candidate(var) {
                heap_versions[var] = heap_versions[var].wrapping_add(1);
                heap.push(Reverse((
                    self.occurrence_cost(var),
                    var,
                    heap_versions[var],
                )));
            }
        }
    }

    fn mark_occurs_dirty_for_clause(
        &mut self,
        clause_idx: usize,
        _touched: &mut Vec<usize>,
        _touched_flags: &mut Vec<bool>,
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
            let implied_lit = self.clause_lit(clause_idx, 0);
            let var = implied_lit.unsigned_abs() as usize;
            self.reason[var] = NO_REASON;
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

    fn subsumption_relation(
        &self,
        driver: SubsumptionCandidate,
        driver_len: usize,
        driver_abstraction: u64,
        candidate_idx: usize,
    ) -> SubsumptionOutcome {
        if candidate_idx >= self.arena.len() || self.clause_is_deleted(candidate_idx) {
            return SubsumptionOutcome::None;
        }
        let candidate_len = self.clause_len(candidate_idx);
        if driver_len > candidate_len {
            return SubsumptionOutcome::None;
        }
        if (driver_abstraction & !self.original_clause_abstraction(candidate_idx)) != 0 {
            return SubsumptionOutcome::None;
        }

        if self.clauses_sorted_by_var
            && self.inline_original_abstractions
            && driver_len >= SORTED_SUBSUMPTION_MIN_LEN
        {
            return self.sorted_subsumption_relation(driver, driver_len, candidate_idx);
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
            SubsumptionOutcome::Subsumed
        } else {
            SubsumptionOutcome::Strengthen(remove_lit)
        }
    }

    fn sorted_subsumption_relation(
        &self,
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
            SubsumptionOutcome::Subsumed
        } else {
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
        queue: &mut VecDeque<SubsumptionCandidate>,
    ) -> bool {
        if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
            return true;
        }

        let clause_len = self.clause_len(clause_idx);
        let locked_lit = if self.clause_locked(clause_idx) {
            Some(self.clause_lit(clause_idx, 0))
        } else {
            None
        };
        let mut remove_pos = None;
        let mut write_pos = 0usize;
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
            strengthened_abstraction |= 1u64 << (lit.unsigned_abs() & 63);
            if clause_len > 2 && remove_pos.is_some() && write_pos != lit_pos {
                self.set_clause_lit(clause_idx, write_pos, lit);
            }
            write_pos += 1;
        }

        let Some(_remove_pos) = remove_pos else {
            self.scratch_preprocess_clause = strengthened;
            return true;
        };

        proof_log.record_clause(&strengthened);
        self.stats.preprocess_strengthened_clauses += 1;

        if clause_len == 2 {
            let unit_lit = strengthened[0];
            self.scratch_preprocess_clause = strengthened;
            self.remove_original_clause_preprocess(clause_idx, touched, touched_flags);
            if !self.enqueue(unit_lit, NO_REASON) || self.propagate().is_some() {
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
        Self::touch_preprocess_var(touched, touched_flags, remove_var);

        self.detach_clause(clause_idx);

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

        if let Some(lit) = locked_lit {
            if lit == remove_lit {
                let var = lit.unsigned_abs() as usize;
                self.reason[var] = NO_REASON;
            }
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

    fn add_normalized_original_clause(
        &mut self,
        normalized: Vec<i32>,
        proof_log: &mut ProofLog,
        log_proof: bool,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
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
            if !self.enqueue(normalized[0], NO_REASON) || self.propagate().is_some() {
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
            self.arena.push((abstraction >> 32) as u32);
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
                Self::touch_preprocess_var(touched, touched_flags, lit.unsigned_abs() as usize);
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
            subsumption_work,
        )
    }

    pub(super) fn add_initial_original_clauses(&mut self, clauses: Vec<Vec<i32>>, sort: bool) {
        let mut proof_log = ProofLog::disabled();
        let mut touched = Vec::new();
        let mut touched_flags = Vec::new();
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
            SubsumptionCandidate::RootUnit(lit) => 1u64 << (lit.unsigned_abs() & 63),
        }
    }

    fn backward_subsumption_check(
        &mut self,
        seed_all_clauses: bool,
        queue: &mut VecDeque<SubsumptionCandidate>,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
        proof_log: &mut ProofLog,
    ) -> bool {
        if seed_all_clauses {
            let original_clause_ids = self.original_clause_ids.clone();
            for clause_idx in original_clause_ids {
                self.enqueue_subsumption_clause(queue, clause_idx);
            }
        }

        while !queue.is_empty() || self.bwdsub_assigns < self.trail.len() {
            let driver = if let Some(candidate) = queue.pop_front() {
                match candidate {
                    SubsumptionCandidate::Clause(clause_idx) => {
                        if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
                            continue;
                        }
                        if clause_header_mark(self.clause_header(clause_idx)) == 2 {
                            let header = self.clause_header(clause_idx);
                            self.arena[clause_idx] = clause_make_header(
                                clause_header_size(header),
                                clause_header_learnt(header),
                                clause_header_has_extra(header),
                                0,
                                clause_header_reloced(header),
                            );
                        }
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
            let driver_abstraction = self.subsumption_driver_abstraction(driver);

            let mut best_var = self.subsumption_driver_lit(driver, 0).unsigned_abs() as usize;
            for driver_pos in 1..driver_len {
                let var = self
                    .subsumption_driver_lit(driver, driver_pos)
                    .unsigned_abs() as usize;
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
            self.clean_occurs(best_var);

            let mut scan_pos = 0usize;
            while scan_pos < self.occurs[best_var].len() {
                let candidate_idx = self.occurs[best_var][scan_pos] as usize;
                scan_pos += 1;
                if driver == SubsumptionCandidate::Clause(candidate_idx) {
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

                match self.subsumption_relation(
                    driver,
                    driver_len,
                    driver_abstraction,
                    candidate_idx,
                ) {
                    SubsumptionOutcome::None => {}
                    SubsumptionOutcome::Subsumed => {
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
                            queue,
                        ) {
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

    fn merge_size_only(&self, lhs_idx: usize, rhs_idx: usize, var: usize) -> Option<usize> {
        let mut size = 0usize;
        let lhs_len = self.clause_len(lhs_idx);
        let rhs_len = self.clause_len(rhs_idx);

        for lit_pos in 0..lhs_len {
            let lit = self.clause_lit(lhs_idx, lit_pos);
            if lit.unsigned_abs() as usize != var {
                size += 1;
            }
        }

        'rhs_lits: for rhs_pos in 0..rhs_len {
            let lit = self.clause_lit(rhs_idx, rhs_pos);
            if lit.unsigned_abs() as usize == var {
                continue;
            }
            for lhs_pos in 0..lhs_len {
                let existing = self.clause_lit(lhs_idx, lhs_pos);
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

    fn merge_into_vec(
        &self,
        lhs_idx: usize,
        rhs_idx: usize,
        var: usize,
        out: &mut Vec<i32>,
    ) -> bool {
        out.clear();
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
        queue: &mut VecDeque<SubsumptionCandidate>,
        touched: &mut Vec<usize>,
        touched_flags: &mut Vec<bool>,
    ) -> bool {
        self.clean_occurs(var);
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
        for &pos_clause_idx in &pos_clauses {
            for &neg_clause_idx in &neg_clauses {
                let Some(size) = self.merge_size_only(pos_clause_idx, neg_clause_idx, var) else {
                    continue;
                };
                resolvent_count += 1;
                if resolvent_count > occurrence_count as isize + self.bve_grow {
                    return false;
                }
                if self.bve_clause_limit >= 0 && size as isize > self.bve_clause_limit {
                    return false;
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

        for &clause_idx in pos_clauses.iter().chain(neg_clauses.iter()) {
            self.remove_original_clause_preprocess(clause_idx, touched, touched_flags);
        }
        if var < self.occurs.len() {
            self.occurs[var].clear();
            self.occurs_dirty[var] = false;
        }

        for &pos_clause_idx in &pos_clauses {
            for &neg_clause_idx in &neg_clauses {
                let mut resolvent = std::mem::take(&mut self.scratch_preprocess_clause);
                let keep = self.merge_into_vec(pos_clause_idx, neg_clause_idx, var, &mut resolvent);
                if keep {
                    self.stats.preprocess_resolvents += 1;
                    let result = self.add_original_clause_from_slice(
                        &resolvent,
                        proof_log,
                        true,
                        touched,
                        touched_flags,
                        Some(&mut *queue),
                    );
                    if result == OriginalClauseInsertResult::Unsat {
                        self.scratch_preprocess_clause = resolvent;
                        return true;
                    }
                }
                self.scratch_preprocess_clause = resolvent;
            }
        }

        true
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

        let run_full_backward_subsumption = self.should_run_full_backward_subsumption();
        self.build_occurrence_index();
        self.bwdsub_assigns = 0;
        let mut queue = VecDeque::new();
        let mut touched = Vec::new();
        let mut touched_flags = vec![false; self.assignment.len()];
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

        if run_full_backward_subsumption
            && !self.backward_subsumption_check(
                true,
                &mut queue,
                &mut touched,
                &mut touched_flags,
                proof_log,
            )
        {
            self.solver_ok = false;
            return false;
        }

        while self.solver_ok {
            if !touched.is_empty() {
                self.gather_touched_clauses(
                    &mut touched,
                    &mut touched_flags,
                    &mut queue,
                    &mut heap,
                    &mut heap_versions,
                );
                continue;
            }

            if run_full_backward_subsumption
                && (!queue.is_empty() || self.bwdsub_assigns < self.trail.len())
            {
                if !self.backward_subsumption_check(
                    false,
                    &mut queue,
                    &mut touched,
                    &mut touched_flags,
                    proof_log,
                ) {
                    self.solver_ok = false;
                    break;
                }
                continue;
            }

            let Some(Reverse((_, var, version))) = heap.pop() else {
                break;
            };
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

            let eliminated = self.try_eliminate_var(
                var,
                proof_log,
                &mut queue,
                &mut touched,
                &mut touched_flags,
            );
            if !eliminated {
                continue;
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
            self.occurs_membership_dirty.clear();
            self.n_occ.clear();
            self.use_simplification = false;
            self.inline_original_abstractions = false;
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

    fn run_backward_subsumption(s: &mut Solver, seed_all: bool, proof: &mut ProofLog) -> bool {
        let mut queue = VecDeque::new();
        let mut touched = Vec::new();
        let mut touched_flags = vec![false; s.assignment.len()];
        s.backward_subsumption_check(
            seed_all,
            &mut queue,
            &mut touched,
            &mut touched_flags,
            proof,
        )
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
        assert!(s.enqueue(1, NO_REASON));
        assert_eq!(s.propagate(), None);
        s.build_occurrence_index();

        assert!(run_backward_subsumption(&mut s, false, &mut proof));

        assert!(live_original_clauses(&s).contains(&vec![2, 3]));
        assert_eq!(s.stats.preprocess_strengthened_clauses, 1);
    }
}
