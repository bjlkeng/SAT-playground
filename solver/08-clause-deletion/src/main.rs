use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;

const UNASSIGNED: u8 = 0;
const TRUE: u8 = 1;
const FALSE: u8 = 2;
const NO_REASON: usize = usize::MAX;
const BRANCH_NOT_IN_HEAP: usize = usize::MAX;
const CCMIN_NONE: u8 = 0;
const CCMIN_BASIC: u8 = 1;
const CCMIN_DEEP: u8 = 2;
const REDUNDANT_UNDEF: u8 = 0;
const REDUNDANT_SOURCE: u8 = 1;
const REDUNDANT_REMOVABLE: u8 = 2;
const REDUNDANT_FAILED: u8 = 3;
const CLAUSE_DELETED: u8 = 1;

#[allow(dead_code)]
#[derive(Clone, Copy)]
struct ClauseRef {
    start: u32,
    len: u32,
    learnt: bool,
    mark: u8,
    activity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Watcher {
    clause_idx: u32,
    blocker: i32,
}

struct Solver {
    clauses: Vec<ClauseRef>,
    clause_data: Vec<i32>,
    learnt_clause_indices: Vec<usize>,
    deleted_literals: usize,
    /// clauses currently watching each literal, with blocker fast path
    watchers: Vec<Vec<Watcher>>,
    /// scratch buffer reused when rebuilding a watch list during propagation
    watch_scratch: Vec<Watcher>,
    /// assignment[v] for variable v (1-based, index 0 unused)
    /// 0 = unassigned, 1 = true, 2 = false
    assignment: Vec<u8>,
    /// last assigned polarity for each variable, reused when branching after backtrack/restart
    saved_phase: Vec<u8>,
    /// decision level of each variable assignment
    decision_level: Vec<usize>,
    /// reason clause index for each implied assignment; NO_REASON for decisions/root-unassigned vars
    reason: Vec<usize>,
    /// number of active assignments currently using a clause as their reason
    reason_refcount: Vec<u32>,
    /// assigned literals in chronological order
    trail: Vec<i32>,
    /// number of level-0 assignments that must survive backtrack(0)
    root_trail_len: usize,
    /// trail index where each decision level starts
    trail_limits: Vec<usize>,
    /// next trail entry whose falsified literal still needs watcher processing
    propagate_head: usize,
    /// static tie-break rank derived from descending literal occurrence count
    branch_rank: Vec<usize>,
    /// binary max-heap of candidate branch variables ordered by activity
    branch_heap: Vec<u32>,
    /// current heap index for each variable, or BRANCH_NOT_IN_HEAP
    branch_pos: Vec<usize>,
    /// EVSIDS-style variable activity
    activity: Vec<f32>,
    /// additive bump applied to variables participating in recent conflicts
    activity_inc: f32,
    /// multiplicative decay factor for older activity
    activity_decay: f32,
    /// MiniSat-style learned clause activity bump
    clause_activity_inc: f32,
    /// multiplicative decay factor for clause activity
    clause_activity_decay: f32,
    /// active target for learned clause count before reduction
    max_learnts: usize,
    learntsize_inc: f32,
    learntsize_adjust_inc: f32,
    learntsize_adjust_confl: f32,
    learntsize_adjust_cnt: usize,
    /// number of conflicts seen since the last restart
    restart_conflicts: usize,
    /// base conflict budget multiplied by the current Luby term
    restart_unit: usize,
    /// one-based index into the Luby restart sequence
    restart_luby_index: usize,
    /// active conflict budget for the current restart window
    restart_conflict_limit: usize,
    /// whether a restart should be applied before the next branch
    restart_pending: bool,
    /// original unit clauses that must be enqueued at decision level 0
    root_unit_clauses: Vec<usize>,
    /// whether the formula already contains an empty clause
    has_empty_clause: bool,
    /// scratch buffers reused during conflict analysis
    scratch_seen: Vec<u8>,
    scratch_resolved: Vec<u8>,
    scratch_learned: Vec<i32>,
    scratch_bumped_vars: Vec<usize>,
    scratch_bumped_clauses: Vec<usize>,
    scratch_redundant_state: Vec<u8>,
    scratch_analyze_toclear: Vec<usize>,
    scratch_analyze_stack: Vec<(usize, i32)>,
    /// 0 = none, 1 = basic, 2 = deep
    ccmin_mode: u8,
    /// learned clauses copied in DRAT proof order; clauses mutate in-place for watching
    proof_clauses: Vec<Vec<i32>>,
    /// whether the proof terminates with the empty clause
    proof_has_empty: bool,
}

fn mark_clause_literals(
    decision_level: &[usize],
    clause: &[i32],
    current_level: usize,
    seen: &mut [u8],
    resolved: &[u8],
    learned: &mut Vec<i32>,
    bumped_vars: &mut Vec<usize>,
    current_level_count: &mut usize,
) {
    for &lit in clause {
        let var = lit.unsigned_abs() as usize;
        if seen[var] != 0 || resolved[var] != 0 {
            continue;
        }

        let level = decision_level[var];
        if level == 0 {
            continue;
        }

        seen[var] = 1;
        bumped_vars.push(var);
        if level == current_level {
            *current_level_count += 1;
        } else {
            learned.push(lit);
        }
    }
}

fn basic_lit_redundant(
    lit: i32,
    clauses: &[ClauseRef],
    clause_data: &[i32],
    reason: &[usize],
    state: &[u8],
) -> bool {
    let var = lit.unsigned_abs() as usize;
    let reason_idx = reason[var];
    if reason_idx == NO_REASON || clauses[reason_idx].learnt {
        return false;
    }

    let clause = clauses[reason_idx];
    let start = clause.start as usize;
    let end = start + clause.len as usize;
    for &q in &clause_data[start..end] {
        if q == lit {
            continue;
        }
        let q_var = q.unsigned_abs() as usize;
        if state[q_var] != REDUNDANT_SOURCE && state[q_var] != REDUNDANT_REMOVABLE {
            return false;
        }
    }

    true
}

fn lit_redundant(
    lit: i32,
    clauses: &[ClauseRef],
    clause_data: &[i32],
    reason: &[usize],
    state: &mut [u8],
    toclear: &mut Vec<usize>,
    stack: &mut Vec<(usize, i32)>,
) -> bool {
    let mut lit = lit;
    debug_assert!({
        let var = lit.unsigned_abs() as usize;
        state[var] == REDUNDANT_UNDEF || state[var] == REDUNDANT_SOURCE
    });
    debug_assert!(reason[lit.unsigned_abs() as usize] != NO_REASON);

    stack.clear();
    let mut clause_idx = reason[lit.unsigned_abs() as usize];
    let mut lit_pos = 0usize;

    loop {
        let clause = clauses[clause_idx];
        let start = clause.start as usize;
        let end = start + clause.len as usize;
        if start + lit_pos < end {
            let parent = clause_data[start + lit_pos];
            if parent == lit {
                lit_pos += 1;
                continue;
            }
            let parent_var = parent.unsigned_abs() as usize;
            if state[parent_var] == REDUNDANT_SOURCE
                || state[parent_var] == REDUNDANT_REMOVABLE
            {
                lit_pos += 1;
                continue;
            }

            if reason[parent_var] == NO_REASON
                || clauses[reason[parent_var]].learnt
                || state[parent_var] == REDUNDANT_FAILED
            {
                let lit_var = lit.unsigned_abs() as usize;
                if state[lit_var] == REDUNDANT_UNDEF {
                    state[lit_var] = REDUNDANT_FAILED;
                    toclear.push(lit_var);
                }
                for &(_, stack_lit) in stack.iter() {
                    let stack_var = stack_lit.unsigned_abs() as usize;
                    if state[stack_var] == REDUNDANT_UNDEF {
                        state[stack_var] = REDUNDANT_FAILED;
                        toclear.push(stack_var);
                    }
                }
                stack.clear();
                return false;
            }

            stack.push((lit_pos, lit));
            lit = parent;
            clause_idx = reason[parent_var];
            lit_pos = 0;
            continue;
        }

        let lit_var = lit.unsigned_abs() as usize;
        if state[lit_var] == REDUNDANT_UNDEF {
            state[lit_var] = REDUNDANT_REMOVABLE;
            toclear.push(lit_var);
        }

        if let Some((resume_pos, resume_lit)) = stack.pop() {
            lit = resume_lit;
            clause_idx = reason[lit.unsigned_abs() as usize];
            lit_pos = resume_pos + 1;
        } else {
            return true;
        }
    }
}

#[inline(always)]
fn lit_to_index(lit: i32) -> usize {
    let var = lit.unsigned_abs() as usize;
    let base = (var - 1) * 2;
    if lit > 0 { base } else { base + 1 }
}

impl Solver {
    fn new(num_vars: usize, clauses: Vec<Vec<i32>>) -> Self {
        let original_clause_count = clauses.len();
        let mut occurrence_count = vec![0usize; num_vars + 1];
        for clause in &clauses {
            for &lit in clause {
                let var = lit.unsigned_abs() as usize;
                occurrence_count[var] += 1;
            }
        }
        let mut branch_order: Vec<u32> = (1..=num_vars as u32).collect();
        branch_order.sort_unstable_by(|&lhs, &rhs| {
            occurrence_count[rhs as usize]
                .cmp(&occurrence_count[lhs as usize])
                .then_with(|| lhs.cmp(&rhs))
        });
        let mut branch_rank = vec![0usize; num_vars + 1];
        for (rank, &var) in branch_order.iter().enumerate() {
            branch_rank[var as usize] = rank;
        }

        let total_literals = clauses.iter().map(|clause| clause.len()).sum();
        let mut clause_refs = Vec::with_capacity(original_clause_count);
        let mut clause_data = Vec::with_capacity(total_literals);
        for clause in clauses {
            let start = clause_data.len();
            let len = clause.len();
            clause_data.extend_from_slice(&clause);
            clause_refs.push(ClauseRef {
                start: start as u32,
                len: len as u32,
                learnt: false,
                mark: 0,
                activity: 0.0,
            });
        }
        let mut solver = Solver {
            clauses: clause_refs,
            clause_data,
            learnt_clause_indices: Vec::new(),
            deleted_literals: 0,
            watchers: vec![Vec::new(); num_vars.saturating_mul(2)],
            watch_scratch: Vec::new(),
            assignment: vec![UNASSIGNED; num_vars + 1],
            saved_phase: vec![TRUE; num_vars + 1],
            decision_level: vec![0; num_vars + 1],
            reason: vec![NO_REASON; num_vars + 1],
            reason_refcount: vec![0; original_clause_count],
            trail: Vec::with_capacity(num_vars),
            root_trail_len: 0,
            trail_limits: Vec::new(),
            propagate_head: 0,
            branch_rank,
            branch_heap: Vec::with_capacity(num_vars),
            branch_pos: vec![BRANCH_NOT_IN_HEAP; num_vars + 1],
            activity: vec![0.0; num_vars + 1],
            activity_inc: 1.0,
            activity_decay: 0.95,
            clause_activity_inc: 1.0,
            clause_activity_decay: 0.999,
            max_learnts: ((original_clause_count.max(1) as f32) / 3.0).ceil() as usize,
            learntsize_inc: 1.1,
            learntsize_adjust_inc: 1.5,
            learntsize_adjust_confl: 100.0,
            learntsize_adjust_cnt: 100,
            restart_conflicts: 0,
            restart_unit: 100,
            restart_luby_index: 1,
            restart_conflict_limit: 100,
            restart_pending: false,
            root_unit_clauses: Vec::new(),
            has_empty_clause: false,
            scratch_seen: vec![0; num_vars + 1],
            scratch_resolved: vec![0; num_vars + 1],
            scratch_learned: Vec::with_capacity(16),
            scratch_bumped_vars: Vec::with_capacity(16),
            scratch_bumped_clauses: Vec::with_capacity(16),
            scratch_redundant_state: vec![0; num_vars + 1],
            scratch_analyze_toclear: Vec::with_capacity(16),
            scratch_analyze_stack: Vec::with_capacity(16),
            ccmin_mode: CCMIN_DEEP,
            proof_clauses: Vec::new(),
            proof_has_empty: false,
        };
        solver.max_learnts = solver.max_learnts.max(8);
        for clause_idx in 0..solver.clauses.len() {
            solver.attach_clause(clause_idx, true);
        }
        for &var in &branch_order {
            solver.push_branch_var(var as usize);
        }
        solver
    }

    #[inline(always)]
    fn current_level(&self) -> usize {
        self.trail_limits.len()
    }

    #[inline(always)]
    fn lit_index(&self, lit: i32) -> usize {
        lit_to_index(lit)
    }

    #[inline(always)]
    fn clause_bounds(&self, clause_idx: usize) -> (usize, usize) {
        let clause = self.clauses[clause_idx];
        let start = clause.start as usize;
        (start, start + clause.len as usize)
    }

    #[inline(always)]
    fn clause_slice(&self, clause_idx: usize) -> &[i32] {
        let (start, end) = self.clause_bounds(clause_idx);
        &self.clause_data[start..end]
    }

    #[inline(always)]
    fn is_clause_deleted(&self, clause_idx: usize) -> bool {
        self.clauses[clause_idx].mark == CLAUSE_DELETED
    }

    #[inline(always)]
    fn is_clause_locked(&self, clause_idx: usize) -> bool {
        self.reason_refcount[clause_idx] > 0
    }

    fn push_branch_var(&mut self, var: usize) {
        if self.assignment[var] != UNASSIGNED || self.branch_pos[var] != BRANCH_NOT_IN_HEAP {
            return;
        }

        let idx = self.branch_heap.len();
        self.branch_heap.push(var as u32);
        self.branch_pos[var] = idx;
        self.branch_heap_sift_up(idx);
    }

    fn rebuild_branch_queue(&mut self) {
        self.branch_heap.clear();
        self.branch_pos.fill(BRANCH_NOT_IN_HEAP);
        for var in 1..self.assignment.len() {
            self.push_branch_var(var);
        }
    }

    fn branch_var_better(&self, lhs: usize, rhs: usize) -> bool {
        self.activity[lhs]
            .total_cmp(&self.activity[rhs])
            .then_with(|| self.branch_rank[rhs].cmp(&self.branch_rank[lhs]))
            .is_gt()
    }

    fn branch_heap_swap(&mut self, lhs: usize, rhs: usize) {
        self.branch_heap.swap(lhs, rhs);
        let lhs_var = self.branch_heap[lhs] as usize;
        let rhs_var = self.branch_heap[rhs] as usize;
        self.branch_pos[lhs_var] = lhs;
        self.branch_pos[rhs_var] = rhs;
    }

    fn branch_heap_sift_up(&mut self, mut idx: usize) {
        while idx > 0 {
            let parent = (idx - 1) / 2;
            let var = self.branch_heap[idx] as usize;
            let parent_var = self.branch_heap[parent] as usize;
            if !self.branch_var_better(var, parent_var) {
                break;
            }

            self.branch_heap_swap(idx, parent);
            idx = parent;
        }
    }

    fn branch_heap_sift_down(&mut self, mut idx: usize) {
        let len = self.branch_heap.len();
        loop {
            let left = idx * 2 + 1;
            let right = left + 1;
            if left >= len {
                break;
            }

            let mut best = left;
            if right < len {
                let left_var = self.branch_heap[left] as usize;
                let right_var = self.branch_heap[right] as usize;
                if self.branch_var_better(right_var, left_var) {
                    best = right;
                }
            }

            let var = self.branch_heap[idx] as usize;
            let best_var = self.branch_heap[best] as usize;
            if !self.branch_var_better(best_var, var) {
                break;
            }

            self.branch_heap_swap(idx, best);
            idx = best;
        }
    }

    fn branch_heap_remove(&mut self, var: usize) {
        let idx = self.branch_pos[var];
        if idx == BRANCH_NOT_IN_HEAP {
            return;
        }

        let last_var = self.branch_heap.pop().expect("heap underflow") as usize;
        self.branch_pos[var] = BRANCH_NOT_IN_HEAP;
        if idx == self.branch_heap.len() {
            return;
        }

        self.branch_heap[idx] = last_var as u32;
        self.branch_pos[last_var] = idx;
        if idx > 0 {
            let parent = (idx - 1) / 2;
            let parent_var = self.branch_heap[parent] as usize;
            if self.branch_var_better(last_var, parent_var) {
                self.branch_heap_sift_up(idx);
                return;
            }
        }
        self.branch_heap_sift_down(idx);
    }

    fn branch_heap_pop_best(&mut self) -> Option<usize> {
        if self.branch_heap.is_empty() {
            return None;
        }
        let best_var = self.branch_heap[0] as usize;
        self.branch_heap_remove(best_var);
        Some(best_var)
    }

    fn attach_clause(&mut self, clause_idx: usize, track_root_unit: bool) {
        let clause = self.clauses[clause_idx];
        if clause.mark == CLAUSE_DELETED {
            return;
        }
        match clause.len {
            0 => {
                self.has_empty_clause = true;
            }
            1 => {
                let lit = self.clause_data[clause.start as usize];
                let watch_idx = self.lit_index(lit);
                self.watchers[watch_idx].push(Watcher {
                    clause_idx: clause_idx as u32,
                    blocker: lit,
                });
                if track_root_unit {
                    self.root_unit_clauses.push(clause_idx);
                }
            }
            _ => {
                let first = self.clause_data[clause.start as usize];
                let second = self.clause_data[clause.start as usize + 1];
                let first_watch_idx = self.lit_index(first);
                let second_watch_idx = self.lit_index(second);
                self.watchers[first_watch_idx].push(Watcher {
                    clause_idx: clause_idx as u32,
                    blocker: second,
                });
                self.watchers[second_watch_idx].push(Watcher {
                    clause_idx: clause_idx as u32,
                    blocker: first,
                });
            }
        }
    }

    fn detach_clause(&mut self, clause_idx: usize) {
        let clause = self.clauses[clause_idx];
        if clause.mark == CLAUSE_DELETED {
            return;
        }

        match clause.len {
            0 => {}
            1 => {
                let lit = self.clause_data[clause.start as usize];
                let watch_idx = self.lit_index(lit);
                self.watchers[watch_idx].retain(|w| w.clause_idx as usize != clause_idx);
            }
            _ => {
                let start = clause.start as usize;
                let first = self.clause_data[start];
                let second = self.clause_data[start + 1];
                let first_watch_idx = self.lit_index(first);
                let second_watch_idx = self.lit_index(second);
                self.watchers[first_watch_idx].retain(|w| w.clause_idx as usize != clause_idx);
                if second_watch_idx != first_watch_idx {
                    self.watchers[second_watch_idx].retain(|w| w.clause_idx as usize != clause_idx);
                }
            }
        }
    }

    fn rebuild_watchers(&mut self) {
        self.watchers = vec![Vec::new(); (self.assignment.len() - 1).saturating_mul(2)];
        self.watch_scratch.clear();
        self.root_unit_clauses.clear();
        self.has_empty_clause = false;
        for clause_idx in 0..self.clauses.len() {
            let track_root_unit = !self.clauses[clause_idx].learnt;
            self.attach_clause(clause_idx, track_root_unit);
        }
    }

    fn remove_clause(&mut self, clause_idx: usize) {
        if self.is_clause_deleted(clause_idx) {
            return;
        }
        debug_assert!(
            !self.is_clause_locked(clause_idx),
            "remove_clause called on locked clause {clause_idx}"
        );

        self.detach_clause(clause_idx);
        self.deleted_literals += self.clauses[clause_idx].len as usize;
        self.clauses[clause_idx].mark = CLAUSE_DELETED;
    }

    fn compact_clause_storage(&mut self) {
        if self.deleted_literals == 0 {
            return;
        }

        let old_clauses = std::mem::take(&mut self.clauses);
        let old_clause_data = std::mem::take(&mut self.clause_data);
        let mut remap = vec![usize::MAX; old_clauses.len()];
        let mut new_clauses = Vec::with_capacity(old_clauses.len());
        let mut new_clause_data =
            Vec::with_capacity(old_clause_data.len().saturating_sub(self.deleted_literals));
        let mut new_learnt_clause_indices = Vec::with_capacity(self.learnt_clause_indices.len());

        for (old_idx, clause) in old_clauses.iter().enumerate() {
            if clause.mark == CLAUSE_DELETED {
                continue;
            }

            let start = new_clause_data.len();
            let old_start = clause.start as usize;
            let old_end = old_start + clause.len as usize;
            new_clause_data.extend_from_slice(&old_clause_data[old_start..old_end]);

            let new_idx = new_clauses.len();
            remap[old_idx] = new_idx;
            new_clauses.push(ClauseRef {
                start: start as u32,
                len: clause.len,
                learnt: clause.learnt,
                mark: 0,
                activity: clause.activity,
            });
            if clause.learnt {
                new_learnt_clause_indices.push(new_idx);
            }
        }

        for reason_idx in &mut self.reason {
            if *reason_idx == NO_REASON {
                continue;
            }
            *reason_idx = remap[*reason_idx];
            debug_assert_ne!(*reason_idx, usize::MAX);
        }

        self.clauses = new_clauses;
        self.clause_data = new_clause_data;
        self.learnt_clause_indices = new_learnt_clause_indices;
        self.reason_refcount = vec![0; self.clauses.len()];
        for &reason_idx in &self.reason {
            if reason_idx != NO_REASON {
                self.reason_refcount[reason_idx] += 1;
            }
        }
        self.deleted_literals = 0;
        self.rebuild_watchers();
    }

    #[inline(always)]
    fn lit_value(&self, lit: i32) -> u8 {
        let var = lit.unsigned_abs() as usize;
        let val = self.assignment[var];
        if val == UNASSIGNED {
            return UNASSIGNED;
        }
        if (lit > 0) == (val == TRUE) {
            TRUE
        } else {
            FALSE
        }
    }

    #[inline(always)]
    fn enqueue(&mut self, lit: i32, reason: usize) -> bool {
        let var = lit.unsigned_abs() as usize;
        let target_value = if lit > 0 { TRUE } else { FALSE };
        let current = self.assignment[var];
        if current == UNASSIGNED {
            let current_level = self.current_level();
            self.branch_heap_remove(var);
            self.assignment[var] = target_value;
            self.saved_phase[var] = target_value;
            self.decision_level[var] = current_level;
            self.reason[var] = reason;
            if reason != NO_REASON {
                self.reason_refcount[reason] += 1;
            }
            self.trail.push(lit);
            if current_level == 0 {
                self.root_trail_len += 1;
            }
            true
        } else {
            current == target_value
        }
    }

    fn enqueue_root_units(&mut self) -> bool {
        for idx in 0..self.root_unit_clauses.len() {
            let clause_idx = self.root_unit_clauses[idx];
            let lit = self.clause_slice(clause_idx)[0];
            if !self.enqueue(lit, clause_idx) {
                return false;
            }
        }
        true
    }

    fn propagate(&mut self) -> Option<usize> {
        while self.propagate_head < self.trail.len() {
            let false_lit = -self.trail[self.propagate_head];
            self.propagate_head += 1;
            let watch_idx = self.lit_index(false_lit);
            let pending = std::mem::take(&mut self.watchers[watch_idx]);
            let mut retained = std::mem::take(&mut self.watch_scratch);
            retained.clear();
            if retained.capacity() < pending.len() {
                retained.reserve(pending.len() - retained.capacity());
            }
            let mut pending_idx = 0usize;

            while pending_idx < pending.len() {
                let watcher = pending[pending_idx];
                pending_idx += 1;
                let clause_idx = watcher.clause_idx as usize;
                let clause = self.clauses[clause_idx];
                if clause.mark == CLAUSE_DELETED {
                    continue;
                }
                if clause.len == 1 {
                    let unit_lit = self.clause_data[clause.start as usize];
                    match self.lit_value(unit_lit) {
                        TRUE => retained.push(watcher),
                        FALSE => {
                            retained.push(watcher);
                            retained.extend_from_slice(&pending[pending_idx..]);
                            self.watchers[watch_idx] = retained;
                            self.watch_scratch = pending;
                            self.watch_scratch.clear();
                            return Some(clause_idx);
                        }
                        UNASSIGNED => {
                            if !self.enqueue(unit_lit, clause_idx) {
                                retained.push(watcher);
                                retained.extend_from_slice(&pending[pending_idx..]);
                                self.watchers[watch_idx] = retained;
                                self.watch_scratch = pending;
                                self.watch_scratch.clear();
                                return Some(clause_idx);
                            }
                            retained.push(watcher);
                        }
                        _ => unreachable!(),
                    }
                    continue;
                }

                if self.lit_value(watcher.blocker) == TRUE {
                    retained.push(watcher);
                    continue;
                }

                let clause_start = clause.start as usize;
                let clause_len = clause.len as usize;
                if self.clause_data[clause_start] == false_lit {
                    self.clause_data.swap(clause_start, clause_start + 1);
                }
                if self.clause_data[clause_start + 1] != false_lit {
                    continue;
                }

                let first = self.clause_data[clause_start];
                let updated_watcher = Watcher {
                    clause_idx: watcher.clause_idx,
                    blocker: first,
                };
                if first != watcher.blocker && self.lit_value(first) == TRUE {
                    retained.push(updated_watcher);
                    continue;
                }

                let mut moved_watch = false;
                for lit_pos in 2..clause_len {
                    let candidate = self.clause_data[clause_start + lit_pos];
                    if self.lit_value(candidate) != FALSE {
                        self.clause_data[clause_start + 1] = candidate;
                        self.clause_data[clause_start + lit_pos] = false_lit;
                        let new_watch_idx = self.lit_index(candidate);
                        self.watchers[new_watch_idx].push(updated_watcher);
                        moved_watch = true;
                        break;
                    }
                }

                if moved_watch {
                    continue;
                }

                retained.push(updated_watcher);
                if self.lit_value(first) == FALSE || !self.enqueue(first, clause_idx) {
                    retained.extend_from_slice(&pending[pending_idx..]);
                    self.watchers[watch_idx] = retained;
                    self.watch_scratch = pending;
                    self.watch_scratch.clear();
                    return Some(clause_idx);
                }
            }

            self.watchers[watch_idx] = retained;
            self.watch_scratch = pending;
            self.watch_scratch.clear();
        }

        None
    }

    fn decide(&mut self, lit: i32) {
        self.trail_limits.push(self.trail.len());
        let inserted = self.enqueue(lit, NO_REASON);
        debug_assert!(inserted, "decision literal must be unassigned");
    }

    fn bump_variable_activity(&mut self, var: usize) {
        self.activity[var] += self.activity_inc;
        if self.activity[var] > 1e30 {
            for value in &mut self.activity[1..] {
                *value *= 1e-30;
            }
            self.activity_inc *= 1e-30;
        }
        let idx = self.branch_pos[var];
        if idx != BRANCH_NOT_IN_HEAP {
            self.branch_heap_sift_up(idx);
        }
    }

    fn bump_clause_activity(&mut self, clause_idx: usize) {
        if self.is_clause_deleted(clause_idx) || !self.clauses[clause_idx].learnt {
            return;
        }

        self.clauses[clause_idx].activity += self.clause_activity_inc;
        if self.clauses[clause_idx].activity > 1e20 {
            for &idx in &self.learnt_clause_indices {
                self.clauses[idx].activity *= 1e-20;
            }
            self.clause_activity_inc *= 1e-20;
        }
    }

    fn decay_variable_activity(&mut self) {
        self.activity_inc /= self.activity_decay;
    }

    fn decay_clause_activity(&mut self) {
        self.clause_activity_inc /= self.clause_activity_decay;
    }

    fn bump_analyzed_variable_activity(&mut self) {
        let bumped_vars = std::mem::take(&mut self.scratch_bumped_vars);
        for &var in &bumped_vars {
            self.bump_variable_activity(var);
        }
        self.scratch_bumped_vars = bumped_vars;
        self.scratch_bumped_vars.clear();
    }

    fn bump_analyzed_clause_activity(&mut self) {
        let bumped_clauses = std::mem::take(&mut self.scratch_bumped_clauses);
        for &clause_idx in &bumped_clauses {
            self.bump_clause_activity(clause_idx);
        }
        self.scratch_bumped_clauses = bumped_clauses;
        self.scratch_bumped_clauses.clear();
    }

    fn luby_value(index: usize) -> usize {
        let mut power = 1usize;
        while (1usize << power) - 1 < index {
            power += 1;
        }

        if index == (1usize << power) - 1 {
            return 1usize << (power - 1);
        }

        Self::luby_value(index - (1usize << (power - 1)) + 1)
    }

    fn note_conflict(&mut self) {
        if self.restart_pending {
            return;
        }

        self.restart_conflicts += 1;
        if self.restart_conflicts < self.restart_conflict_limit {
            return;
        }

        self.restart_conflicts = 0;
        self.restart_pending = true;
        self.restart_luby_index += 1;
        self.restart_conflict_limit =
            self.restart_unit
                .saturating_mul(Self::luby_value(self.restart_luby_index));
    }

    fn note_learntsize_adjust(&mut self) {
        self.learntsize_adjust_cnt = self.learntsize_adjust_cnt.saturating_sub(1);
        if self.learntsize_adjust_cnt != 0 {
            return;
        }

        self.learntsize_adjust_confl *= self.learntsize_adjust_inc;
        self.learntsize_adjust_cnt = self.learntsize_adjust_confl.max(1.0) as usize;
        self.max_learnts = ((self.max_learnts as f32) * self.learntsize_inc).ceil() as usize;
        self.max_learnts = self.max_learnts.max(8);
    }

    fn reduce_db(&mut self) {
        if self.learnt_clause_indices.is_empty() {
            return;
        }

        let extra_lim = self.clause_activity_inc / self.learnt_clause_indices.len() as f32;
        self.learnt_clause_indices.sort_unstable_by(|&lhs, &rhs| {
            let lhs_clause = self.clauses[lhs];
            let rhs_clause = self.clauses[rhs];
            let lhs_pref = lhs_clause.len > 2;
            let rhs_pref = rhs_clause.len > 2;
            lhs_pref
                .cmp(&rhs_pref)
                .reverse()
                .then_with(|| lhs_clause.activity.total_cmp(&rhs_clause.activity))
        });

        let learnts = std::mem::take(&mut self.learnt_clause_indices);
        let cutoff = learnts.len() / 2;
        let mut kept = Vec::with_capacity(learnts.len());
        for (i, clause_idx) in learnts.into_iter().enumerate() {
            let clause = self.clauses[clause_idx];
            if clause.mark == CLAUSE_DELETED {
                continue;
            }

            if clause.len > 2
                && !self.is_clause_locked(clause_idx)
                && (i < cutoff || clause.activity < extra_lim)
            {
                self.remove_clause(clause_idx);
            } else {
                kept.push(clause_idx);
            }
        }
        self.learnt_clause_indices = kept;
        self.compact_clause_storage();
    }

    fn pick_branch_lit(&mut self) -> Option<i32> {
        self.branch_heap_pop_best().map(|var| {
            if self.saved_phase[var] == FALSE {
                -(var as i32)
            } else {
                var as i32
            }
        })
    }

    fn backtrack(&mut self, target_level: usize) {
        let new_trail_len = if target_level == 0 {
            self.root_trail_len
        } else {
            self.trail_limits[target_level - 1]
        };

        while self.trail.len() > new_trail_len {
            let lit = self.trail.pop().expect("trail underflow");
            let var = lit.unsigned_abs() as usize;
            let reason_idx = self.reason[var];
            if reason_idx != NO_REASON {
                self.reason_refcount[reason_idx] -= 1;
            }
            self.assignment[var] = UNASSIGNED;
            self.decision_level[var] = 0;
            self.reason[var] = NO_REASON;
            self.push_branch_var(var);
        }

        self.trail_limits.truncate(target_level);
        self.propagate_head = self.propagate_head.min(new_trail_len);
    }

    fn perform_restart_if_pending(&mut self) -> bool {
        if !self.restart_pending {
            return false;
        }

        self.restart_pending = false;
        if self.current_level() == 0 {
            return false;
        }

        self.backtrack(0);
        true
    }

    fn add_clause(&mut self, clause: Vec<i32>) -> usize {
        let start = self.clause_data.len();
        let len = clause.len();
        self.clause_data.extend_from_slice(&clause);
        self.clauses.push(ClauseRef {
            start: start as u32,
            len: len as u32,
            learnt: true,
            mark: 0,
            activity: 0.0,
        });
        self.reason_refcount.push(0);
        let clause_idx = self.clauses.len() - 1;
        self.learnt_clause_indices.push(clause_idx);
        self.attach_clause(clause_idx, false);
        clause_idx
    }

    fn minimize_learned_clause(&mut self, learned_clause: &mut Vec<i32>) {
        if self.ccmin_mode == CCMIN_NONE || learned_clause.len() <= 1 {
            return;
        }

        let state = &mut self.scratch_redundant_state;
        let toclear = &mut self.scratch_analyze_toclear;
        let stack = &mut self.scratch_analyze_stack;
        debug_assert!(toclear.is_empty());
        debug_assert!(stack.is_empty());

        for &lit in &learned_clause[1..] {
            let var = lit.unsigned_abs() as usize;
            if state[var] == REDUNDANT_UNDEF {
                state[var] = REDUNDANT_SOURCE;
                toclear.push(var);
            }
        }

        let clauses = &self.clauses;
        let clause_data = &self.clause_data;
        let reason = &self.reason;
        let mut write = 1usize;
        for read in 1..learned_clause.len() {
            let lit = learned_clause[read];
            let var = lit.unsigned_abs() as usize;
            let keep = if reason[var] == NO_REASON {
                true
            } else if clauses[reason[var]].learnt {
                true
            } else if self.ccmin_mode == CCMIN_BASIC {
                !basic_lit_redundant(lit, clauses, clause_data, reason, state)
            } else {
                !lit_redundant(
                    lit,
                    clauses,
                    clause_data,
                    reason,
                    state,
                    toclear,
                    stack,
                )
            };
            if keep {
                learned_clause[write] = lit;
                write += 1;
            }
        }
        learned_clause.truncate(write);

        for &var in toclear.iter() {
            state[var] = REDUNDANT_UNDEF;
        }
        toclear.clear();
        stack.clear();
    }

    fn analyze_conflict(&mut self, conflict_clause_idx: usize) -> (Vec<i32>, usize) {
        let current_level = self.current_level();
        let decision_level = &self.decision_level;
        let clauses = &self.clauses;
        let clause_data = &self.clause_data;
        let seen = &mut self.scratch_seen;
        let resolved = &mut self.scratch_resolved;
        let learned = &mut self.scratch_learned;
        let bumped_vars = &mut self.scratch_bumped_vars;
        let bumped_clauses = &mut self.scratch_bumped_clauses;
        unsafe {
            std::ptr::write_bytes(seen.as_mut_ptr(), 0, seen.len());
            std::ptr::write_bytes(resolved.as_mut_ptr(), 0, resolved.len());
        }
        learned.clear();
        bumped_vars.clear();
        bumped_clauses.clear();

        if self.clauses[conflict_clause_idx].learnt {
            bumped_clauses.push(conflict_clause_idx);
        }

        let mut current_level_count = 0usize;

        mark_clause_literals(
            decision_level,
            &clause_data[clauses[conflict_clause_idx].start as usize
                ..clauses[conflict_clause_idx].start as usize
                    + clauses[conflict_clause_idx].len as usize],
            current_level,
            seen,
            resolved,
            learned,
            bumped_vars,
            &mut current_level_count,
        );

        debug_assert!(current_level_count > 0);

        let mut trail_index = self.trail.len();
        let uip_lit = loop {
            trail_index -= 1;
            let lit = self.trail[trail_index];
            let var = lit.unsigned_abs() as usize;
            if seen[var] == 0 {
                continue;
            }

            seen[var] = 0;
            resolved[var] = 1;
            current_level_count -= 1;
            if current_level_count == 0 {
                break lit;
            }

                let reason_idx = self.reason[var];
                if reason_idx != NO_REASON {
                    if self.clauses[reason_idx].learnt {
                        bumped_clauses.push(reason_idx);
                    }
                    mark_clause_literals(
                        decision_level,
                        &clause_data[clauses[reason_idx].start as usize
                            ..clauses[reason_idx].start as usize + clauses[reason_idx].len as usize],
                        current_level,
                        seen,
                        resolved,
                        learned,
                    bumped_vars,
                    &mut current_level_count,
                );
            }
        };

        let mut learned_clause = Vec::with_capacity(learned.len() + 1);
        learned_clause.push(-uip_lit);
        learned_clause.extend(learned.iter().copied());
        self.minimize_learned_clause(&mut learned_clause);

        let mut backtrack_level = 0usize;
        let mut backtrack_pos = 1usize;
        for pos in 1..learned_clause.len() {
            let lit = learned_clause[pos];
            let var = lit.unsigned_abs() as usize;
            let level = self.decision_level[var];
            if level > backtrack_level {
                backtrack_level = level;
                backtrack_pos = pos;
            }
        }

        if learned_clause.len() > 2 && backtrack_pos != 1 {
            learned_clause.swap(1, backtrack_pos);
        }

        (learned_clause, backtrack_level)
    }

    fn learned_clause_count(&self) -> usize {
        self.learnt_clause_indices.len()
    }

    fn solve(&mut self) -> bool {
        if self.has_empty_clause || !self.enqueue_root_units() {
            self.proof_has_empty = true;
            return false;
        }

        let mut conflict = self.propagate();

        loop {
            match conflict {
                Some(conflict_clause_idx) => {
                    if self.current_level() == 0 {
                        self.proof_has_empty = true;
                        return false;
                    }

                    let (learned_clause, backtrack_level) =
                        self.analyze_conflict(conflict_clause_idx);
                    self.bump_analyzed_variable_activity();
                    self.bump_analyzed_clause_activity();
                    self.decay_variable_activity();
                    self.decay_clause_activity();
                    self.note_conflict();
                    self.note_learntsize_adjust();
                    let asserting_lit = learned_clause[0];
                    self.proof_clauses.push(learned_clause.clone());
                    let learned_clause_idx = self.add_clause(learned_clause);
                    self.bump_clause_activity(learned_clause_idx);

                    self.backtrack(backtrack_level);
                    let inserted = self.enqueue(asserting_lit, learned_clause_idx);
                    debug_assert!(inserted, "learned clause must be asserting after backtrack");

                    conflict = self.propagate();
                }
                None => {
                    if self.perform_restart_if_pending() {
                        conflict = self.propagate();
                        continue;
                    }

                    if self
                        .learnt_clause_indices
                        .len()
                        .saturating_sub(self.trail.len())
                        >= self.max_learnts
                    {
                        self.reduce_db();
                    }

                    match self.pick_branch_lit() {
                        Some(lit) => {
                            self.decide(lit);
                            conflict = self.propagate();
                        }
                        None => return true,
                    }
                }
            }
        }
    }
}

fn parse_ccmin_mode() -> u8 {
    match env::var("SAT_CCMIN_MODE") {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "0" | "none" => CCMIN_NONE,
            "1" | "basic" => CCMIN_BASIC,
            "2" | "deep" => CCMIN_DEEP,
            other => {
                eprintln!(
                    "Invalid SAT_CCMIN_MODE={other}; expected none/basic/deep or 0/1/2"
                );
                std::process::exit(2);
            }
        },
        Err(_) => CCMIN_DEEP,
    }
}

fn parse_cnf(path: &str) -> (usize, Vec<Vec<i32>>) {
    let file = fs::File::open(path).unwrap_or_else(|e| {
        eprintln!("Error opening {}: {}", path, e);
        std::process::exit(1);
    });
    let reader = io::BufReader::new(file);

    let mut num_vars = 0;
    let mut clauses: Vec<Vec<i32>> = Vec::new();
    let mut current_clause: Vec<i32> = Vec::new();

    for line in reader.lines() {
        let line = line.expect("Failed to read line");
        let line = line.trim();

        if line.is_empty() || line.starts_with('c') {
            continue;
        }

        if line.starts_with('p') {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 && parts[1] == "cnf" {
                num_vars = parts[2].parse().unwrap_or(0);
            }
            continue;
        }

        for token in line.split_whitespace() {
            let lit: i32 = match token.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            if lit == 0 {
                clauses.push(std::mem::take(&mut current_clause));
            } else {
                current_clause.push(lit);
            }
        }
    }

    if !current_clause.is_empty() {
        clauses.push(current_clause);
    }

    (num_vars, clauses)
}

fn write_proof(
    output_dir: &str,
    proof_clauses: &[Vec<i32>],
    proof_has_empty: bool,
) {
    let proof_path = Path::new(output_dir).join("proof.out");
    let file = fs::File::create(&proof_path).unwrap_or_else(|e| {
        eprintln!("Error creating {}: {}", proof_path.display(), e);
        std::process::exit(1);
    });
    let mut writer = io::BufWriter::new(file);

    for clause in proof_clauses {
        for &lit in clause {
            write!(writer, "{} ", lit).expect("Failed to write proof");
        }
        writer.write_all(b"0\n").expect("Failed to write proof");
    }

    if proof_has_empty {
        writer.write_all(b"0\n").expect("Failed to write proof");
    }

    writer.flush().expect("Failed to flush proof");
}

fn print_assignment(assignment: &[u8]) {
    let mut line = String::from("v");
    for var in 1..assignment.len() {
        let lit = if assignment[var] == FALSE {
            -(var as i32)
        } else {
            var as i32
        };
        let token = format!(" {}", lit);
        if line.len() + token.len() > 4090 {
            println!("{}", line);
            line = String::from("v");
        }
        line.push_str(&token);
    }
    line.push_str(" 0");
    println!("{}", line);
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: sat-solver <cnf_path> <output_dir>");
        std::process::exit(1);
    }

    let cnf_path = &args[1];
    let output_dir = &args[2];

    let (num_vars, clauses) = parse_cnf(cnf_path);
    let mut solver = Solver::new(num_vars, clauses);
    solver.ccmin_mode = parse_ccmin_mode();

    if solver.solve() {
        println!("s SATISFIABLE");
        print_assignment(&solver.assignment);
    } else {
        println!("s UNSATISFIABLE");
        write_proof(
            output_dir,
            &solver.proof_clauses,
            solver.proof_has_empty,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_solver(num_vars: usize, clauses: Vec<Vec<i32>>) -> Solver {
        Solver::new(num_vars, clauses)
    }

    fn watched_literals(s: &Solver, clause_idx: usize) -> Option<(i32, i32)> {
        let clause = s.clauses[clause_idx];
        if clause.len < 2 {
            return None;
        }

        let start = clause.start as usize;
        Some((s.clause_data[start], s.clause_data[start + 1]))
    }

    fn install_manual_state(
        s: &mut Solver,
        trail: &[i32],
        trail_limits: &[usize],
        reason_overrides: &[(usize, usize)],
    ) {
        s.assignment.fill(UNASSIGNED);
        s.decision_level.fill(0);
        s.reason.fill(NO_REASON);
        s.reason_refcount.fill(0);
        s.trail.clear();
        s.trail.extend_from_slice(trail);
        s.trail_limits = trail_limits.to_vec();
        s.root_trail_len = 0;
        s.propagate_head = s.trail.len();

        let mut level = 0usize;
        let mut next_limit_idx = 0usize;
        for (trail_idx, &lit) in trail.iter().enumerate() {
            while next_limit_idx < trail_limits.len() && trail_limits[next_limit_idx] == trail_idx {
                level += 1;
                next_limit_idx += 1;
            }

            let var = lit.unsigned_abs() as usize;
            s.assignment[var] = if lit > 0 { TRUE } else { FALSE };
            s.decision_level[var] = level;
        }

        for &(var, reason_idx) in reason_overrides {
            s.reason[var] = reason_idx;
            s.reason_refcount[reason_idx] += 1;
        }
    }

    #[test]
    fn test_unit_clause_sat() {
        let mut s = make_solver(1, vec![vec![1]]);
        assert!(s.solve());
        assert_eq!(s.assignment[1], TRUE);
    }

    #[test]
    fn test_contradiction_unsat() {
        let mut s = make_solver(1, vec![vec![1], vec![-1]]);
        assert!(!s.solve());
    }

    #[test]
    fn test_empty_clause_unsat() {
        let mut s = make_solver(2, vec![vec![1, 2], vec![]]);
        assert!(!s.solve());
    }

    #[test]
    fn test_two_clause_sat() {
        let mut s = make_solver(2, vec![vec![1], vec![2]]);
        assert!(s.solve());
        assert_eq!(s.assignment[1], TRUE);
        assert_eq!(s.assignment[2], TRUE);
    }

    #[test]
    fn test_chain_unsat() {
        let mut s = make_solver(3, vec![vec![1], vec![-1, 2], vec![-2, 3], vec![-3]]);
        assert!(!s.solve());
    }

    #[test]
    fn test_three_sat_instance() {
        let clauses = vec![
            vec![1, 2, 3],
            vec![-1, 2, 4],
            vec![1, -3, 5],
            vec![-2, 4, 5],
            vec![-1, -4, -5],
            vec![3, 4, -2],
        ];
        let mut s = make_solver(5, clauses.clone());
        assert!(s.solve());
        for clause in &clauses {
            let sat = clause.iter().any(|&lit| s.lit_value(lit) == TRUE);
            assert!(sat, "Clause {:?} not satisfied", clause);
        }
    }

    #[test]
    fn test_pigeonhole_3_2_unsat() {
        let clauses = vec![
            vec![1, 2],
            vec![3, 4],
            vec![5, 6],
            vec![-1, -3],
            vec![-1, -5],
            vec![-3, -5],
            vec![-2, -4],
            vec![-2, -6],
            vec![-4, -6],
        ];
        let mut s = make_solver(6, clauses);
        assert!(!s.solve());
    }

    #[test]
    fn test_no_clauses_sat() {
        let mut s = make_solver(3, vec![]);
        assert!(s.solve());
    }

    #[test]
    fn test_bcp_moves_watch_and_then_implies_last_literal() {
        let mut s = make_solver(3, vec![vec![1, 2, 3]]);

        assert_eq!(
            s.watchers[s.lit_index(1)],
            vec![Watcher {
                clause_idx: 0,
                blocker: 2,
            }]
        );
        assert_eq!(
            s.watchers[s.lit_index(2)],
            vec![Watcher {
                clause_idx: 0,
                blocker: 1,
            }]
        );

        s.decide(-1);
        assert_eq!(s.propagate(), None);

        assert_eq!(watched_literals(&s, 0), Some((2, 3)));
        assert_eq!(
            s.watchers[s.lit_index(3)],
            vec![Watcher {
                clause_idx: 0,
                blocker: 2,
            }]
        );

        s.decide(-2);
        assert_eq!(s.propagate(), None);
        assert_eq!(s.lit_value(3), TRUE);
    }

    #[test]
    fn test_basic_clause_minimization_removes_direct_reason_literal() {
        let clauses = vec![vec![5, 3, 4], vec![2, 1, 5], vec![6, 1, 3, 4], vec![2, 6]];
        let mut s = make_solver(6, clauses);
        install_manual_state(
            &mut s,
            &[3, 4, 5, 1, 2, 6],
            &[0, 3],
            &[(5, 0), (2, 1), (6, 2)],
        );

        s.ccmin_mode = CCMIN_NONE;
        let (raw_learned, raw_backtrack) = s.analyze_conflict(3);
        assert_eq!(raw_learned, vec![-1, 3, 4, 5]);
        assert_eq!(raw_backtrack, 1);

        s.ccmin_mode = CCMIN_BASIC;
        let (basic_learned, basic_backtrack) = s.analyze_conflict(3);
        assert_eq!(basic_learned, vec![-1, 3, 4]);
        assert_eq!(basic_backtrack, 1);
    }

    #[test]
    fn test_deep_clause_minimization_removes_recursive_reason_literal() {
        let clauses = vec![
            vec![5, 3, 4],
            vec![7, 5, 6],
            vec![2, 1, 7, 6],
            vec![8, 1, 3, 4],
            vec![2, 8],
        ];
        let mut s = make_solver(8, clauses);
        install_manual_state(
            &mut s,
            &[3, 4, 6, 5, 7, 1, 2, 8],
            &[0, 5],
            &[(5, 0), (7, 1), (2, 2), (8, 3)],
        );

        s.ccmin_mode = CCMIN_NONE;
        let (raw_learned, raw_backtrack) = s.analyze_conflict(4);
        assert_eq!(raw_learned, vec![-1, 3, 4, 7, 6]);
        assert_eq!(raw_backtrack, 1);

        s.ccmin_mode = CCMIN_BASIC;
        let (basic_learned, _) = s.analyze_conflict(4);
        assert_eq!(basic_learned, vec![-1, 3, 4, 7, 6]);

        s.ccmin_mode = CCMIN_DEEP;
        let (deep_learned, deep_backtrack) = s.analyze_conflict(4);
        assert_eq!(deep_learned, vec![-1, 3, 4, 6]);
        assert_eq!(deep_backtrack, 1);
    }

    #[test]
    fn test_clause_minimization_keeps_literal_with_non_source_root_parent() {
        let mut s = make_solver(6, vec![vec![-5, 3, 6]]);
        s.decision_level[1] = 2;
        s.decision_level[3] = 1;
        s.decision_level[5] = 1;
        s.reason[5] = 0;

        let mut learned_clause = vec![-1, 3, 5];

        s.ccmin_mode = CCMIN_BASIC;
        s.minimize_learned_clause(&mut learned_clause);
        assert_eq!(learned_clause, vec![-1, 3, 5]);

        s.ccmin_mode = CCMIN_DEEP;
        s.minimize_learned_clause(&mut learned_clause);
        assert_eq!(learned_clause, vec![-1, 3, 5]);
    }

    #[test]
    fn test_cdcl_learns_clause_on_unsat_instance() {
        let clauses = vec![
            vec![1, 2],
            vec![-1, 2],
            vec![1, -2],
            vec![-1, -2],
        ];
        let mut s = make_solver(2, clauses);
        assert!(!s.solve());
        assert!(s.learned_clause_count() > 0);
    }

    #[test]
    fn test_unsat_proof_logs_learned_clause_before_empty_clause() {
        let clauses = vec![
            vec![1, 2],
            vec![-1, 2],
            vec![1, -2],
            vec![-1, -2],
        ];
        let mut s = make_solver(2, clauses);
        assert!(!s.solve());
        assert!(s.learned_clause_count() > 0);
        assert!(
            !s.proof_clauses.is_empty(),
            "expected proof to contain at least one learned clause before the empty clause",
        );
        assert!(s.proof_has_empty, "expected proof to end with the empty clause");
    }

    #[test]
    fn test_backtrack_to_zero_preserves_root_assignments() {
        let mut s = make_solver(4, vec![vec![1], vec![-1, 2], vec![-2, 3]]);
        assert!(s.enqueue_root_units());
        assert_eq!(s.propagate(), None);
        assert_eq!(s.root_trail_len, 3);
        assert_eq!(s.assignment[1], TRUE);
        assert_eq!(s.assignment[2], TRUE);
        assert_eq!(s.assignment[3], TRUE);

        s.decide(-4);
        s.backtrack(0);

        assert_eq!(s.trail.len(), 3);
        assert_eq!(s.assignment[1], TRUE);
        assert_eq!(s.assignment[2], TRUE);
        assert_eq!(s.assignment[3], TRUE);
        assert_eq!(s.assignment[4], UNASSIGNED);
    }

    #[test]
    fn test_pick_branch_lit_prefers_highest_activity() {
        let mut s = make_solver(3, vec![vec![1, 2], vec![-1, 3]]);
        s.activity[1] = 1.0;
        s.activity[2] = 4.0;
        s.activity[3] = 2.0;
        s.rebuild_branch_queue();

        assert_eq!(s.pick_branch_lit(), Some(2));

        s.decide(2);
        assert_eq!(s.pick_branch_lit(), Some(3));
    }

    #[test]
    fn test_pick_branch_lit_uses_saved_phase_for_selected_variable() {
        let mut s = make_solver(3, vec![vec![1, 2], vec![-1, 3]]);
        s.activity[1] = 1.0;
        s.activity[2] = 4.0;
        s.activity[3] = 2.0;
        s.rebuild_branch_queue();

        assert_eq!(s.pick_branch_lit(), Some(2));

        s.saved_phase[2] = FALSE;
        s.rebuild_branch_queue();
        assert_eq!(s.pick_branch_lit(), Some(-2));
    }

    #[test]
    fn test_saved_phase_survives_backtrack() {
        let mut s = make_solver(2, vec![]);
        s.activity[1] = 5.0;
        s.activity[2] = 1.0;
        s.rebuild_branch_queue();

        s.decide(-1);
        s.backtrack(0);

        assert_eq!(s.assignment[1], UNASSIGNED);
        assert_eq!(s.pick_branch_lit(), Some(-1));
    }

    #[test]
    fn test_backtrack_requeues_variable_into_branch_queue() {
        let mut s = make_solver(2, vec![]);
        s.activity[1] = 3.0;
        s.activity[2] = 1.0;
        s.rebuild_branch_queue();

        assert_eq!(s.pick_branch_lit(), Some(1));

        s.decide(1);
        assert_eq!(s.pick_branch_lit(), Some(2));

        s.backtrack(0);
        assert_eq!(s.pick_branch_lit(), Some(1));
    }

    #[test]
    fn test_conflict_analysis_tracks_intermediate_reason_variables_for_activity() {
        let clauses = vec![
            vec![-1, 5],
            vec![-5, 4],
            vec![-5, 6],
            vec![-4, 2],
            vec![-6, 3],
            vec![-2, -3],
        ];
        let mut s = make_solver(6, clauses);

        s.decide(1);
        let conflict_clause_idx = s.propagate().expect("expected conflict after propagation");
        let (learned_clause, backtrack_level) = s.analyze_conflict(conflict_clause_idx);

        assert_eq!(learned_clause, vec![-5]);
        assert_eq!(backtrack_level, 0);

        let mut bumped_vars = s.scratch_bumped_vars.clone();
        bumped_vars.sort_unstable();
        assert_eq!(bumped_vars, vec![2, 3, 4, 5, 6]);
    }

    #[test]
    fn test_conflict_bumps_variable_activity() {
        let clauses = vec![
            vec![1, 2],
            vec![-1, 2],
            vec![1, -2],
            vec![-1, -2],
        ];
        let mut s = make_solver(2, clauses);

        assert!(!s.solve());
        assert!(s.learned_clause_count() > 0);
        assert!(s.activity[1] > 0.0);
        assert!(s.activity[2] > 0.0);
    }

    #[test]
    fn test_luby_sequence_values() {
        assert_eq!(Solver::luby_value(1), 1);
        assert_eq!(Solver::luby_value(2), 1);
        assert_eq!(Solver::luby_value(3), 2);
        assert_eq!(Solver::luby_value(4), 1);
        assert_eq!(Solver::luby_value(5), 1);
        assert_eq!(Solver::luby_value(6), 2);
        assert_eq!(Solver::luby_value(7), 4);
    }

    #[test]
    fn test_conflict_budget_schedules_restart_and_advances_luby_window() {
        let mut s = make_solver(2, vec![vec![1, 2], vec![-1, -2]]);
        s.restart_unit = 2;
        s.restart_luby_index = 1;
        s.restart_conflict_limit = 2;

        s.note_conflict();
        assert_eq!(s.restart_conflicts, 1);
        assert!(!s.restart_pending);
        assert_eq!(s.restart_conflict_limit, 2);

        s.note_conflict();
        assert_eq!(s.restart_conflicts, 0);
        assert!(s.restart_pending);
        assert_eq!(s.restart_luby_index, 2);
        assert_eq!(s.restart_conflict_limit, 2);

        s.restart_pending = false;
        s.note_conflict();
        s.note_conflict();
        assert!(s.restart_pending);
        assert_eq!(s.restart_luby_index, 3);
        assert_eq!(s.restart_conflict_limit, 4);
    }

    #[test]
    fn test_restart_backtracks_to_root_but_keeps_root_assignments() {
        let mut s = make_solver(4, vec![vec![1], vec![-1, 2], vec![3, 4]]);
        assert!(s.enqueue_root_units());
        assert_eq!(s.propagate(), None);
        s.decide(-3);
        assert_eq!(s.current_level(), 1);
        assert_eq!(s.assignment[1], TRUE);
        assert_eq!(s.assignment[2], TRUE);
        assert_eq!(s.assignment[3], FALSE);
        assert_eq!(s.assignment[4], UNASSIGNED);

        s.restart_pending = true;
        assert!(s.perform_restart_if_pending());

        assert_eq!(s.current_level(), 0);
        assert_eq!(s.assignment[1], TRUE);
        assert_eq!(s.assignment[2], TRUE);
        assert_eq!(s.assignment[3], UNASSIGNED);
        assert_eq!(s.assignment[4], UNASSIGNED);
        assert!(!s.restart_pending);
    }

    #[test]
    fn test_reason_refcount_tracks_enqueue_and_backtrack() {
        let mut s = make_solver(3, vec![vec![1, 2]]);
        let learnt_idx = s.add_clause(vec![3, -1, -2]);

        s.decide(1);
        assert!(s.enqueue(3, learnt_idx));
        assert_eq!(s.reason_refcount[learnt_idx], 1);

        s.backtrack(0);
        assert_eq!(s.reason_refcount[learnt_idx], 0);
    }

    #[test]
    fn test_reduce_db_keeps_locked_and_binary_clauses() {
        let mut s = make_solver(4, vec![vec![1, 2], vec![-1, 3]]);
        let locked_idx = s.add_clause(vec![4, -1, -2]);
        let removable_idx = s.add_clause(vec![4, -2, -3]);
        let binary_idx = s.add_clause(vec![-3, -4]);

        s.clauses[locked_idx].activity = 10.0;
        s.clauses[removable_idx].activity = 0.0;
        s.clauses[binary_idx].activity = 0.0;

        s.decide(1);
        assert!(s.enqueue(4, locked_idx));
        assert_eq!(s.reason_refcount[locked_idx], 1);

        s.reduce_db();

        assert_eq!(s.learnt_clause_indices.len(), 2);
        let learnt_clauses: Vec<Vec<i32>> = s
            .learnt_clause_indices
            .iter()
            .map(|&idx| s.clause_slice(idx).to_vec())
            .collect();
        assert!(learnt_clauses.iter().any(|clause| clause.len() == 2));
        assert!(learnt_clauses.iter().any(|clause| clause.len() == 3));
        assert!(
            !learnt_clauses
                .iter()
                .any(|clause| clause == &vec![4, -2, -3] || clause == &vec![-2, 4, -3])
        );
        assert_eq!(s.reason_refcount.iter().copied().sum::<u32>(), 1);

        let _ = removable_idx;
        let _ = binary_idx;
    }

    #[test]
    fn test_compaction_reclaims_deleted_clause_storage() {
        let mut s = make_solver(4, vec![vec![1, 2], vec![-1, 3]]);
        let doomed_idx = s.add_clause(vec![1, 2, 3, 4]);
        let kept_idx = s.add_clause(vec![-1, -2, -3]);
        let before = s.clause_data.len();

        s.remove_clause(doomed_idx);
        assert!(s.is_clause_deleted(doomed_idx));
        assert!(!s.is_clause_deleted(kept_idx));
        assert_eq!(s.clause_data.len(), before);

        s.compact_clause_storage();

        assert!(s.clause_data.len() < before);
        assert_eq!(s.learnt_clause_indices.len(), 1);
        let remapped_idx = s.learnt_clause_indices[0];
        assert_eq!(s.clause_slice(remapped_idx), &[-1, -2, -3]);
    }
}
