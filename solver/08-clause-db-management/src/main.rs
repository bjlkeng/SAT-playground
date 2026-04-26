use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

const UNASSIGNED: u8 = 0;
const TRUE: u8 = 1;
const FALSE: u8 = 2;
const NO_REASON: usize = usize::MAX;
const NO_LEARNED_POS: usize = usize::MAX;
const BRANCH_NOT_IN_HEAP: usize = usize::MAX;
const CCMIN_NONE: u8 = 0;
const CCMIN_BASIC: u8 = 1;
const CCMIN_DEEP: u8 = 2;
const REDUNDANT_UNDEF: u8 = 0;
const REDUNDANT_SOURCE: u8 = 1;
const REDUNDANT_REMOVABLE: u8 = 2;
const REDUNDANT_FAILED: u8 = 3;
const PROOF_BUFFER_CAPACITY: usize = 16 * 1024 * 1024;
const LEARNTSIZE_FACTOR: f64 = 1.0 / 3.0;
const LEARNTSIZE_INC: f64 = 1.1;
const LEARNTSIZE_ADJUST_START_CONFL: usize = 50;
const LEARNTSIZE_ADJUST_INC: f64 = 1.5;

#[allow(dead_code)]
#[derive(Clone, Copy)]
struct ClauseRef {
    start: u32,
    len: u32,
    learnt: bool,
    deleted: bool,
    activity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Watcher {
    clause_idx: u32,
    blocker: i32,
}

#[derive(Clone, Default)]
struct SolverStats {
    conflicts: u64,
    propagations: u64,
    decisions: u64,
    restarts: u64,
    reduce_db_calls: u64,
    deleted_clauses: u64,
    garbage_collections: u64,
    learned_clauses: u64,
}

enum ProofMode {
    Disabled,
    Stream(ProofStream),
}

struct ProofStream {
    final_path: PathBuf,
    temp_path: PathBuf,
    file: fs::File,
    buffer: Vec<u8>,
    capacity: usize,
}

struct ProofLog {
    mode: ProofMode,
}

fn append_u32_ascii(buffer: &mut Vec<u8>, mut value: u32) {
    if value == 0 {
        buffer.push(b'0');
        return;
    }

    let mut digits = [0u8; 10];
    let mut len = 0;
    while value > 0 {
        digits[len] = (value % 10) as u8;
        value /= 10;
        len += 1;
    }

    for idx in (0..len).rev() {
        buffer.push(b'0' + digits[idx]);
    }
}

fn append_i32_ascii(buffer: &mut Vec<u8>, value: i32) {
    if value < 0 {
        buffer.push(b'-');
    }
    append_u32_ascii(buffer, value.unsigned_abs());
}

impl ProofLog {
    fn disabled() -> Self {
        Self {
            mode: ProofMode::Disabled,
        }
    }

    fn new<P: AsRef<Path>>(output_dir: P, capacity: usize) -> Self {
        let output_dir = output_dir.as_ref();
        fs::create_dir_all(output_dir).unwrap_or_else(|e| {
            eprintln!("Error creating {}: {}", output_dir.display(), e);
            std::process::exit(1);
        });

        let temp_path = output_dir.join("proof.out.tmp");
        let final_path = output_dir.join("proof.out");
        let file = fs::File::create(&temp_path).unwrap_or_else(|e| {
            eprintln!("Error creating {}: {}", temp_path.display(), e);
            std::process::exit(1);
        });

        Self {
            mode: ProofMode::Stream(ProofStream {
                final_path,
                temp_path,
                file,
                buffer: Vec::with_capacity(capacity),
                capacity,
            }),
        }
    }

    fn record_clause(&mut self, clause: &[i32]) {
        if let ProofMode::Stream(stream) = &mut self.mode {
            stream.buffer.reserve(clause.len() * 12 + 2);
            for &lit in clause {
                append_i32_ascii(&mut stream.buffer, lit);
                stream.buffer.push(b' ');
            }
            stream.buffer.extend_from_slice(b"0\n");
            if stream.buffer.len() >= stream.capacity {
                Self::flush_stream(stream);
            }
        }
    }

    fn finish_sat(&mut self) {
        match std::mem::replace(&mut self.mode, ProofMode::Disabled) {
            ProofMode::Disabled => {}
            ProofMode::Stream(stream) => {
                drop(stream.file);
                let _ = fs::remove_file(&stream.temp_path);
            }
        }
    }

    fn finish_unsat(&mut self) {
        match std::mem::replace(&mut self.mode, ProofMode::Disabled) {
            ProofMode::Disabled => {}
            ProofMode::Stream(mut stream) => {
                stream
                    .buffer
                    .write_all(b"0\n")
                    .expect("Failed to buffer empty proof clause");
                Self::flush_stream(&mut stream);
                stream.file.flush().expect("Failed to flush proof file");
                drop(stream.file);
                fs::rename(&stream.temp_path, &stream.final_path).unwrap_or_else(|e| {
                    eprintln!(
                        "Error renaming {} to {}: {}",
                        stream.temp_path.display(),
                        stream.final_path.display(),
                        e
                    );
                    std::process::exit(1);
                });
            }
        }
    }

    fn flush_stream(stream: &mut ProofStream) {
        if stream.buffer.is_empty() {
            return;
        }
        stream
            .file
            .write_all(&stream.buffer)
            .expect("Failed to write proof buffer");
        stream.buffer.clear();
    }
}

struct Solver {
    clauses: Vec<ClauseRef>,
    clause_data: Vec<i32>,
    original_clause_count: usize,
    /// live learned-clause ids, mirroring MiniSat's dedicated `learnts` vector
    learned_clause_ids: Vec<usize>,
    /// reverse map from clause index to learned-clause position, or NO_LEARNED_POS
    learned_clause_pos: Vec<usize>,
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
    /// additive bump applied to learned clauses participating in recent conflicts
    clause_activity_inc: f32,
    /// multiplicative decay factor for older learned-clause activity
    clause_activity_decay: f32,
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
    /// learned-clause budget threshold for running a database reduction pass
    reduce_db_limit: usize,
    /// current conflict countdown until the next learned-budget adjustment
    learntsize_adjust_cnt: usize,
    /// floating-point copy of the current learned-budget adjustment window
    learntsize_adjust_confl: f64,
    /// current number of non-deleted learned clauses
    live_learned_clause_count: usize,
    /// total number of literal slots currently wasted by tombstoned learned clauses
    deleted_clause_literals: usize,
    /// original unit clauses that must be enqueued at decision level 0
    root_unit_clauses: Vec<usize>,
    /// whether the formula already contains an empty clause
    has_empty_clause: bool,
    /// scratch buffers reused during conflict analysis
    scratch_seen: Vec<u8>,
    scratch_resolved: Vec<u8>,
    scratch_learned: Vec<i32>,
    scratch_bumped_vars: Vec<usize>,
    scratch_redundant_state: Vec<u8>,
    scratch_analyze_toclear: Vec<usize>,
    scratch_analyze_stack: Vec<(usize, i32)>,
    /// 0 = none, 1 = basic, 2 = deep
    ccmin_mode: u8,
    stats: SolverStats,
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
    decision_level: &[usize],
    reason: &[usize],
    state: &[u8],
) -> bool {
    let var = lit.unsigned_abs() as usize;
    let reason_idx = reason[var];
    if reason_idx == NO_REASON {
        return false;
    }

    let clause = clauses[reason_idx];
    let start = clause.start as usize;
    let end = start + clause.len as usize;
    for &q in &clause_data[start + 1..end] {
        let q_var = q.unsigned_abs() as usize;
        if decision_level[q_var] == 0 {
            continue;
        }
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
    decision_level: &[usize],
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
    let mut lit_pos = 1usize;

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

            if decision_level[parent_var] == 0 {
                lit_pos += 1;
                continue;
            }

            if reason[parent_var] == NO_REASON || state[parent_var] == REDUNDANT_FAILED {
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
            debug_assert!(
                stack.len() <= reason.len(),
                "redundancy DFS exceeded variable count while checking literal {lit}"
            );
            lit = parent;
            clause_idx = reason[parent_var];
            lit_pos = 1;
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
                deleted: false,
                activity: 0.0,
            });
        }
        let mut solver = Solver {
            clauses: clause_refs,
            clause_data,
            original_clause_count,
            learned_clause_ids: Vec::new(),
            learned_clause_pos: vec![NO_LEARNED_POS; original_clause_count],
            watchers: vec![Vec::new(); num_vars.saturating_mul(2)],
            watch_scratch: Vec::new(),
            assignment: vec![UNASSIGNED; num_vars + 1],
            saved_phase: vec![TRUE; num_vars + 1],
            decision_level: vec![0; num_vars + 1],
            reason: vec![NO_REASON; num_vars + 1],
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
            restart_conflicts: 0,
            restart_unit: 100,
            restart_luby_index: 1,
            restart_conflict_limit: 100,
            restart_pending: false,
            reduce_db_limit: ((original_clause_count as f64) * LEARNTSIZE_FACTOR) as usize,
            learntsize_adjust_cnt: LEARNTSIZE_ADJUST_START_CONFL,
            learntsize_adjust_confl: LEARNTSIZE_ADJUST_START_CONFL as f64,
            live_learned_clause_count: 0,
            deleted_clause_literals: 0,
            root_unit_clauses: Vec::new(),
            has_empty_clause: false,
            scratch_seen: vec![0; num_vars + 1],
            scratch_resolved: vec![0; num_vars + 1],
            scratch_learned: Vec::with_capacity(16),
            scratch_bumped_vars: Vec::with_capacity(16),
            scratch_redundant_state: vec![0; num_vars + 1],
            scratch_analyze_toclear: Vec::with_capacity(16),
            scratch_analyze_stack: Vec::with_capacity(16),
            ccmin_mode: CCMIN_DEEP,
            stats: SolverStats::default(),
        };
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
        debug_assert!(!clause.deleted, "attempted to read deleted clause {clause_idx}");
        let start = clause.start as usize;
        (start, start + clause.len as usize)
    }

    #[inline(always)]
    fn clause_slice(&self, clause_idx: usize) -> &[i32] {
        let (start, end) = self.clause_bounds(clause_idx);
        &self.clause_data[start..end]
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
        debug_assert!(!clause.deleted, "attempted to attach deleted clause {clause_idx}");
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

    fn remove_clause_watchers(watch_list: &mut Vec<Watcher>, clause_idx: usize) {
        watch_list.retain(|watcher| watcher.clause_idx as usize != clause_idx);
    }

    fn detach_clause(&mut self, clause_idx: usize) {
        let clause = self.clauses[clause_idx];
        if clause.deleted || clause.len == 0 {
            return;
        }

        let start = clause.start as usize;
        let first_watch_idx = self.lit_index(self.clause_data[start]);
        Self::remove_clause_watchers(&mut self.watchers[first_watch_idx], clause_idx);
        if clause.len >= 2 {
            let second_watch_idx = self.lit_index(self.clause_data[start + 1]);
            if second_watch_idx != first_watch_idx {
                Self::remove_clause_watchers(&mut self.watchers[second_watch_idx], clause_idx);
            } else {
                Self::remove_clause_watchers(&mut self.watchers[second_watch_idx], clause_idx);
            }
        }
        Self::remove_clause_watchers(&mut self.watch_scratch, clause_idx);
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
            self.stats.propagations += 1;
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
        self.stats.decisions += 1;
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

    fn decay_variable_activity(&mut self) {
        self.activity_inc /= self.activity_decay;
    }

    fn bump_clause_activity(&mut self, clause_idx: usize) {
        if clause_idx >= self.clauses.len() {
            return;
        }
        let clause = &mut self.clauses[clause_idx];
        if !clause.learnt || clause.deleted {
            return;
        }

        clause.activity += self.clause_activity_inc;
        if clause.activity > 1e20 {
            for learned in &mut self.clauses[self.original_clause_count..] {
                learned.activity *= 1e-20;
            }
            self.clause_activity_inc *= 1e-20;
        }
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
        let current_level = self.current_level();
        debug_assert!(target_level <= current_level);
        if target_level == current_level {
            return;
        }

        let new_trail_len = if target_level == 0 {
            self.root_trail_len
        } else {
            self.trail_limits[target_level]
        };

        while self.trail.len() > new_trail_len {
            let lit = self.trail.pop().expect("trail underflow");
            let var = lit.unsigned_abs() as usize;
            self.assignment[var] = UNASSIGNED;
            self.decision_level[var] = 0;
            self.reason[var] = NO_REASON;
            self.push_branch_var(var);
        }

        self.trail_limits.truncate(target_level);
        self.propagate_head = self.propagate_head.min(new_trail_len);
    }

    fn debug_assert_clause_asserting_after_backtrack(&self, learned_clause: &[i32], backtrack_level: usize) {
        debug_assert_eq!(self.current_level(), backtrack_level);
        debug_assert_eq!(
            self.lit_value(learned_clause[0]),
            UNASSIGNED,
            "asserting literal must be unassigned after backtrack"
        );
        for &lit in &learned_clause[1..] {
            debug_assert_eq!(
                self.lit_value(lit),
                FALSE,
                "non-head learned literal {lit} must be false after backtrack"
            );
        }
    }

    fn perform_restart_if_pending(&mut self) -> bool {
        if !self.restart_pending {
            return false;
        }

        self.restart_pending = false;
        if self.current_level() == 0 {
            return false;
        }

        self.stats.restarts += 1;
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
            deleted: false,
            activity: 0.0,
        });
        let clause_idx = self.clauses.len() - 1;
        let learned_pos = self.learned_clause_ids.len();
        self.learned_clause_ids.push(clause_idx);
        self.learned_clause_pos.push(learned_pos);
        self.live_learned_clause_count += 1;
        self.stats.learned_clauses += 1;
        self.attach_clause(clause_idx, false);
        clause_idx
    }

    fn clause_locked(&self, clause_idx: usize) -> bool {
        let clause = self.clauses[clause_idx];
        if clause.deleted || clause.len == 0 {
            return false;
        }
        let implied_lit = self.clause_data[clause.start as usize];
        let var = implied_lit.unsigned_abs() as usize;
        self.lit_value(implied_lit) == TRUE && self.reason[var] == clause_idx
    }

    fn reduce_db_enabled(&self) -> bool {
        self.reduce_db_limit != usize::MAX
    }

    fn note_learnt_budget_conflict(&mut self) {
        if !self.reduce_db_enabled() || self.learntsize_adjust_cnt == 0 {
            return;
        }
        self.learntsize_adjust_cnt -= 1;
        if self.learntsize_adjust_cnt == 0 {
            self.learntsize_adjust_confl *= LEARNTSIZE_ADJUST_INC;
            self.learntsize_adjust_cnt = self.learntsize_adjust_confl as usize;
            self.reduce_db_limit = ((self.reduce_db_limit as f64) * LEARNTSIZE_INC) as usize;
        }
    }

    fn delete_clause(&mut self, clause_idx: usize) {
        debug_assert!(clause_idx < self.clauses.len(), "invalid clause index {clause_idx}");
        debug_assert!(
            self.clauses[clause_idx].learnt,
            "only learned clauses may be deleted"
        );
        debug_assert!(
            !self.clauses[clause_idx].deleted,
            "clause {clause_idx} already deleted"
        );
        debug_assert!(
            !self.clause_locked(clause_idx),
            "cannot delete clause {clause_idx} while it is still a live reason"
        );

        self.detach_clause(clause_idx);
        self.mark_clause_deleted(clause_idx);
    }

    fn mark_clause_deleted(&mut self, clause_idx: usize) {
        debug_assert!(clause_idx < self.clauses.len(), "invalid clause index {clause_idx}");
        debug_assert!(
            self.clauses[clause_idx].learnt,
            "only learned clauses may be deleted"
        );
        debug_assert!(
            !self.clauses[clause_idx].deleted,
            "clause {clause_idx} already deleted"
        );
        debug_assert!(
            !self.reason.iter().any(|&reason_idx| reason_idx == clause_idx),
            "cannot delete clause {clause_idx} while it is still a live reason"
        );
        let learned_pos = self.learned_clause_pos[clause_idx];
        debug_assert_ne!(
            learned_pos, NO_LEARNED_POS,
            "deleted learned clause {clause_idx} missing from learned-clause list"
        );
        let moved_clause_idx = self.learned_clause_ids.last().copied().unwrap();
        self.learned_clause_ids.swap_remove(learned_pos);
        if learned_pos < self.learned_clause_ids.len() {
            self.learned_clause_pos[moved_clause_idx] = learned_pos;
        }
        self.learned_clause_pos[clause_idx] = NO_LEARNED_POS;
        self.live_learned_clause_count = self.live_learned_clause_count.saturating_sub(1);
        self.deleted_clause_literals += self.clauses[clause_idx].len as usize;
        self.clauses[clause_idx].deleted = true;
        self.stats.deleted_clauses += 1;
    }

    fn maybe_garbage_collect(&mut self) {
        if self.deleted_clause_literals == 0 {
            return;
        }
        if self.deleted_clause_literals.saturating_mul(3) < self.clause_data.len() {
            return;
        }
        self.garbage_collect();
    }

    fn garbage_collect(&mut self) {
        self.stats.garbage_collections += 1;
        let mut reloc = vec![NO_REASON; self.clauses.len()];
        let live_clause_count = self.clauses.iter().filter(|clause| !clause.deleted).count();
        let live_literal_count = self
            .clauses
            .iter()
            .filter(|clause| !clause.deleted)
            .map(|clause| clause.len as usize)
            .sum();

        let mut new_clauses = Vec::with_capacity(live_clause_count);
        let mut new_clause_data = Vec::with_capacity(live_literal_count);
        let mut new_learned_clause_ids = Vec::with_capacity(self.learned_clause_ids.len());
        let mut new_learned_clause_pos = Vec::with_capacity(live_clause_count);

        for old_idx in 0..self.clauses.len() {
            let clause = self.clauses[old_idx];
            if clause.deleted {
                continue;
            }

            let start = new_clause_data.len();
            let old_start = clause.start as usize;
            let old_end = old_start + clause.len as usize;
            new_clause_data.extend_from_slice(&self.clause_data[old_start..old_end]);

            let new_idx = new_clauses.len();
            reloc[old_idx] = new_idx;
            new_clauses.push(ClauseRef {
                start: start as u32,
                len: clause.len,
                learnt: clause.learnt,
                deleted: false,
                activity: clause.activity,
            });
            new_learned_clause_pos.push(NO_LEARNED_POS);
            if clause.learnt {
                let learned_pos = new_learned_clause_ids.len();
                new_learned_clause_ids.push(new_idx);
                new_learned_clause_pos[new_idx] = learned_pos;
            }
        }

        debug_assert!(
            self.clauses[..self.original_clause_count]
                .iter()
                .all(|clause| !clause.deleted),
            "original clauses must stay live across garbage collection"
        );

        for watch_list in &mut self.watchers {
            let mut write = 0usize;
            for read in 0..watch_list.len() {
                let mut watcher = watch_list[read];
                let new_idx = reloc[watcher.clause_idx as usize];
                if new_idx == NO_REASON {
                    continue;
                }
                watcher.clause_idx = new_idx as u32;
                watch_list[write] = watcher;
                write += 1;
            }
            watch_list.truncate(write);
        }

        let mut watch_scratch_write = 0usize;
        for read in 0..self.watch_scratch.len() {
            let mut watcher = self.watch_scratch[read];
            let new_idx = reloc[watcher.clause_idx as usize];
            if new_idx == NO_REASON {
                continue;
            }
            watcher.clause_idx = new_idx as u32;
            self.watch_scratch[watch_scratch_write] = watcher;
            watch_scratch_write += 1;
        }
        self.watch_scratch.truncate(watch_scratch_write);

        for reason_idx in &mut self.reason {
            if *reason_idx == NO_REASON {
                continue;
            }
            let new_idx = reloc[*reason_idx];
            debug_assert_ne!(
                new_idx, NO_REASON,
                "garbage collection removed a clause that is still a live reason"
            );
            *reason_idx = new_idx;
        }

        let mut root_write = 0usize;
        for read in 0..self.root_unit_clauses.len() {
            let new_idx = reloc[self.root_unit_clauses[read]];
            if new_idx == NO_REASON {
                continue;
            }
            self.root_unit_clauses[root_write] = new_idx;
            root_write += 1;
        }
        self.root_unit_clauses.truncate(root_write);

        self.clauses = new_clauses;
        self.clause_data = new_clause_data;
        self.learned_clause_ids = new_learned_clause_ids;
        self.learned_clause_pos = new_learned_clause_pos;
        self.live_learned_clause_count = self.learned_clause_ids.len();
        self.deleted_clause_literals = 0;
    }

    fn reduce_db(&mut self) {
        self.stats.reduce_db_calls += 1;
        let mut candidates = self.learned_clause_ids.clone();

        candidates.sort_unstable_by(|&lhs, &rhs| {
            let lhs_clause = self.clauses[lhs];
            let rhs_clause = self.clauses[rhs];
            let lhs_short = lhs_clause.len <= 2;
            let rhs_short = rhs_clause.len <= 2;
            lhs_short
                .cmp(&rhs_short)
                .then_with(|| lhs_clause.activity.total_cmp(&rhs_clause.activity))
                .then_with(|| lhs.cmp(&rhs))
        });

        let extra_lim = if candidates.is_empty() {
            0.0
        } else {
            self.clause_activity_inc / candidates.len() as f32
        };
        let half = candidates.len() / 2;
        for (idx, &clause_idx) in candidates.iter().enumerate() {
            let clause = self.clauses[clause_idx];
            if clause.len > 2
                && !self.clause_locked(clause_idx)
                && (idx < half || clause.activity < extra_lim)
            {
                self.delete_clause(clause_idx);
            }
        }

        self.maybe_garbage_collect();
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
        let decision_level = &self.decision_level;
        let reason = &self.reason;
        let mut write = 1usize;
        for read in 1..learned_clause.len() {
            let lit = learned_clause[read];
            let var = lit.unsigned_abs() as usize;
            let keep = if reason[var] == NO_REASON {
                true
            } else if self.ccmin_mode == CCMIN_BASIC {
                !basic_lit_redundant(lit, clauses, clause_data, decision_level, reason, state)
            } else {
                !lit_redundant(
                    lit,
                    clauses,
                    clause_data,
                    decision_level,
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

    fn mark_clause_literals_for_analysis(
        &mut self,
        clause_idx: usize,
        current_level: usize,
        current_level_count: &mut usize,
    ) {
        if self.reduce_db_enabled() {
            self.bump_clause_activity(clause_idx);
        }

        let clause = self.clauses[clause_idx];
        let start = clause.start as usize;
        let end = start + clause.len as usize;
        let clause_lits = &self.clause_data[start..end];
        mark_clause_literals(
            &self.decision_level,
            clause_lits,
            current_level,
            &mut self.scratch_seen,
            &self.scratch_resolved,
            &mut self.scratch_learned,
            &mut self.scratch_bumped_vars,
            current_level_count,
        );
    }

    fn analyze_conflict(&mut self, conflict_clause_idx: usize) -> (Vec<i32>, usize) {
        let current_level = self.current_level();
        unsafe {
            std::ptr::write_bytes(self.scratch_seen.as_mut_ptr(), 0, self.scratch_seen.len());
            std::ptr::write_bytes(
                self.scratch_resolved.as_mut_ptr(),
                0,
                self.scratch_resolved.len(),
            );
        }
        self.scratch_learned.clear();
        self.scratch_bumped_vars.clear();

        let mut current_level_count = 0usize;

        self.mark_clause_literals_for_analysis(
            conflict_clause_idx,
            current_level,
            &mut current_level_count,
        );

        debug_assert!(current_level_count > 0);

        let mut trail_index = self.trail.len();
        let uip_lit = loop {
            trail_index -= 1;
            let lit = self.trail[trail_index];
            let var = lit.unsigned_abs() as usize;
            if self.scratch_seen[var] == 0 {
                continue;
            }

            self.scratch_seen[var] = 0;
            self.scratch_resolved[var] = 1;
            current_level_count -= 1;
            if current_level_count == 0 {
                break lit;
            }

            let reason_idx = self.reason[var];
            if reason_idx != NO_REASON {
                self.mark_clause_literals_for_analysis(
                    reason_idx,
                    current_level,
                    &mut current_level_count,
                );
            }
        };

        let mut learned_clause = Vec::with_capacity(self.scratch_learned.len() + 1);
        learned_clause.push(-uip_lit);
        learned_clause.extend(self.scratch_learned.iter().copied());
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
        self.learned_clause_ids.len()
    }

    fn solve(&mut self) -> bool {
        let mut proof_log = ProofLog::disabled();
        self.solve_with_proof(&mut proof_log)
    }

    fn solve_to_output(&mut self, output_dir: &str) -> bool {
        let mut proof_log = ProofLog::new(output_dir, PROOF_BUFFER_CAPACITY);
        let sat = self.solve_with_proof(&mut proof_log);
        if sat {
            proof_log.finish_sat();
        } else {
            proof_log.finish_unsat();
        }
        sat
    }

    fn solve_with_proof(&mut self, proof_log: &mut ProofLog) -> bool {
        if self.has_empty_clause || !self.enqueue_root_units() {
            return false;
        }

        let mut conflict = self.propagate();

        loop {
            match conflict {
                Some(conflict_clause_idx) => {
                    if self.current_level() == 0 {
                        return false;
                    }

                    self.stats.conflicts += 1;
                    let (learned_clause, backtrack_level) =
                        self.analyze_conflict(conflict_clause_idx);
                    self.bump_analyzed_variable_activity();
                    self.decay_variable_activity();
                    if self.reduce_db_enabled() {
                        self.decay_clause_activity();
                        self.note_learnt_budget_conflict();
                    }
                    self.note_conflict();
                    let asserting_lit = learned_clause[0];
                    proof_log.record_clause(&learned_clause);
                    let learned_clause_idx = self.add_clause(learned_clause);
                    if self.reduce_db_enabled() {
                        self.bump_clause_activity(learned_clause_idx);
                    }

                    self.backtrack(backtrack_level);
                    self.debug_assert_clause_asserting_after_backtrack(
                        self.clause_slice(learned_clause_idx),
                        backtrack_level,
                    );
                    let inserted = self.enqueue(asserting_lit, learned_clause_idx);
                    debug_assert!(inserted, "learned clause must be asserting after backtrack");

                    conflict = self.propagate();
                }
                None => {
                    if self.perform_restart_if_pending() {
                        conflict = self.propagate();
                        continue;
                    }

                    if self.reduce_db_enabled()
                        && self
                            .live_learned_clause_count
                            .saturating_sub(self.trail.len())
                            >= self.reduce_db_limit
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

fn parse_usize_env(name: &str, default: usize) -> usize {
    match env::var(name) {
        Ok(value) => match value.trim().parse::<usize>() {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("Invalid {name}={value:?}: {err}");
                std::process::exit(2);
            }
        },
        Err(_) => default,
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
    solver.reduce_db_limit = parse_usize_env("SAT_REDUCE_DB_INIT", solver.reduce_db_limit);
    solver.learntsize_adjust_cnt =
        parse_usize_env("SAT_REDUCE_DB_INTERVAL", solver.learntsize_adjust_cnt);
    solver.learntsize_adjust_confl = solver.learntsize_adjust_cnt as f64;

    if solver.solve_to_output(output_dir) {
        println!("s SATISFIABLE");
        print_assignment(&solver.assignment);
    } else {
        println!("s UNSATISFIABLE");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_solver(num_vars: usize, clauses: Vec<Vec<i32>>) -> Solver {
        Solver::new(num_vars, clauses)
    }

    fn make_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sat-playground-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("failed to create temp dir");
        path
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
    fn test_deep_clause_minimization_recurses_through_learned_reasons() {
        let mut s = make_solver(7, vec![vec![5, 3], vec![7, -5, 3]]);
        s.clauses[1].learnt = true;
        s.reason[5] = 0;
        s.reason[7] = 1;

        let mut learned_clause = vec![-1, 3, -7];
        s.ccmin_mode = CCMIN_DEEP;
        s.minimize_learned_clause(&mut learned_clause);

        assert_eq!(learned_clause, vec![-1, 3]);
    }

    #[test]
    fn test_clause_minimization_ignores_level_zero_parents() {
        let mut s = make_solver(5, vec![vec![5, 3, 4]]);
        s.decision_level[1] = 2;
        s.decision_level[3] = 1;
        s.decision_level[5] = 1;
        s.reason[5] = 0;

        let mut basic_clause = vec![-1, 3, 5];
        s.ccmin_mode = CCMIN_BASIC;
        s.minimize_learned_clause(&mut basic_clause);
        assert_eq!(basic_clause, vec![-1, 3]);

        let mut deep_clause = vec![-1, 3, 5];
        s.ccmin_mode = CCMIN_DEEP;
        s.minimize_learned_clause(&mut deep_clause);
        assert_eq!(deep_clause, vec![-1, 3]);
    }

    #[test]
    fn test_clause_minimization_keeps_literal_with_non_source_root_parent() {
        let mut s = make_solver(6, vec![vec![-5, 3, 6]]);
        s.decision_level[1] = 2;
        s.decision_level[3] = 1;
        s.decision_level[5] = 1;
        s.decision_level[6] = 1;
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
    fn test_clause_gc_relocates_watchers_for_live_learned_clause() {
        let mut s = make_solver(3, vec![]);
        let dead = s.add_clause(vec![2, 1]);
        let live = s.add_clause(vec![3, 1]);
        let relocated_live = live - 1;

        s.mark_clause_deleted(dead);
        s.garbage_collect();

        let watch_clause_ids: Vec<_> = s.watchers[s.lit_index(1)]
            .iter()
            .map(|watcher| watcher.clause_idx)
            .collect();
        assert_eq!(watch_clause_ids, vec![relocated_live as u32]);

        s.decide(-1);
        assert_eq!(s.propagate(), None);
        assert_eq!(s.assignment[3], TRUE);
        assert_eq!(s.reason[3], relocated_live);
        assert_eq!(s.clause_slice(relocated_live), &[3, 1]);
    }

    #[test]
    fn test_clause_gc_relocates_reason_refs_for_live_assignments() {
        let mut s = make_solver(4, vec![]);
        let dead = s.add_clause(vec![4, 1]);
        let live = s.add_clause(vec![3, 1]);
        let relocated_live = live - 1;

        s.assignment[4] = TRUE;
        s.saved_phase[4] = TRUE;
        s.decision_level[4] = 1;
        s.reason[4] = live;
        s.trail.push(4);
        s.trail_limits.push(0);

        s.mark_clause_deleted(dead);
        s.garbage_collect();

        assert_eq!(s.reason[4], relocated_live);
        assert_eq!(s.clause_slice(s.reason[4]), &[3, 1]);
    }

    #[test]
    fn test_delete_clause_detaches_watchers_immediately() {
        let mut s = make_solver(3, vec![]);
        let clause_idx = s.add_clause(vec![3, 1, 2]);

        assert!(
            s.watchers[s.lit_index(3)]
                .iter()
                .any(|watcher| watcher.clause_idx as usize == clause_idx)
        );
        assert!(
            s.watchers[s.lit_index(1)]
                .iter()
                .any(|watcher| watcher.clause_idx as usize == clause_idx)
        );

        s.delete_clause(clause_idx);

        assert!(s.clauses[clause_idx].deleted);
        assert_eq!(s.learned_clause_count(), 0);
        assert!(s.learned_clause_ids.is_empty());
        assert_eq!(s.stats.deleted_clauses, 1);
        assert!(
            s.watchers[s.lit_index(3)]
                .iter()
                .all(|watcher| watcher.clause_idx as usize != clause_idx)
        );
        assert!(
            s.watchers[s.lit_index(1)]
                .iter()
                .all(|watcher| watcher.clause_idx as usize != clause_idx)
        );
    }

    #[test]
    fn test_mark_clause_deleted_updates_swapped_learned_clause_position() {
        let mut s = make_solver(6, vec![]);
        let first = s.add_clause(vec![6, 1, 2]);
        let middle = s.add_clause(vec![5, 1, 3]);
        let tail = s.add_clause(vec![4, 2, 3]);

        assert_eq!(s.learned_clause_ids, vec![first, middle, tail]);
        assert_eq!(s.learned_clause_pos[first], 0);
        assert_eq!(s.learned_clause_pos[middle], 1);
        assert_eq!(s.learned_clause_pos[tail], 2);

        s.mark_clause_deleted(middle);

        assert_eq!(s.learned_clause_ids, vec![first, tail]);
        assert_eq!(s.learned_clause_pos[first], 0);
        assert_eq!(s.learned_clause_pos[middle], NO_LEARNED_POS);
        assert_eq!(s.learned_clause_pos[tail], 1);

        s.mark_clause_deleted(tail);

        assert_eq!(s.learned_clause_ids, vec![first]);
        assert_eq!(s.learned_clause_pos[tail], NO_LEARNED_POS);
        assert_eq!(s.learned_clause_pos[first], 0);
    }

    #[test]
    fn test_reduce_db_keeps_locked_and_binary_learned_clauses() {
        let mut s = make_solver(5, vec![]);
        let removable = s.add_clause(vec![5, 1, 2]);
        let binary = s.add_clause(vec![4, 1]);
        let locked = s.add_clause(vec![3, 1, 2]);

        s.clauses[removable].activity = 0.0;
        s.clauses[binary].activity = 0.0;
        s.clauses[locked].activity = 10.0;

        s.assignment[3] = TRUE;
        s.saved_phase[3] = TRUE;
        s.decision_level[3] = 1;
        s.reason[3] = locked;
        s.trail.push(3);
        s.trail_limits.push(0);
        s.propagate_head = s.trail.len();

        s.reduce_db();

        assert_eq!(s.learned_clause_count(), 2);
        assert_eq!(s.stats.reduce_db_calls, 1);
        assert_eq!(s.stats.deleted_clauses, 1);
        assert_eq!(s.clause_slice(s.reason[3]), &[3, 1, 2]);
        assert!(
            s.watchers.iter().flatten().all(|watcher| {
                !s.clauses[watcher.clause_idx as usize].deleted
            }),
            "watch lists should not keep deleted clauses after reduction"
        );

        s.decide(-1);
        assert_eq!(s.propagate(), None);
        assert_eq!(s.assignment[4], TRUE);
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
    fn test_proof_log_flushes_temp_file_when_buffer_fills() {
        let dir = make_temp_dir("proof-flush");
        let temp_path = dir.join("proof.out.tmp");
        let mut proof = ProofLog::new(&dir, 32);

        for _ in 0..4 {
            proof.record_clause(&[123456789, -123456789, 42]);
        }

        assert!(temp_path.exists(), "expected temp proof file to exist");
        assert!(
            fs::metadata(&temp_path)
                .expect("failed to stat temp proof file")
                .len()
                > 0,
            "expected proof buffer flush to write bytes before finalization"
        );
    }

    #[test]
    fn test_append_i32_ascii_formats_signed_values() {
        let mut buffer = Vec::new();
        append_i32_ascii(&mut buffer, -2147483648);
        buffer.push(b' ');
        append_i32_ascii(&mut buffer, -17);
        buffer.push(b' ');
        append_i32_ascii(&mut buffer, 0);
        buffer.push(b' ');
        append_i32_ascii(&mut buffer, 42);

        assert_eq!(
            std::str::from_utf8(&buffer).expect("ascii digits"),
            "-2147483648 -17 0 42"
        );
    }

    #[test]
    fn test_proof_log_finalizes_unsat_and_discards_sat_temp_file() {
        let unsat_dir = make_temp_dir("proof-unsat");
        let mut unsat_proof = ProofLog::new(&unsat_dir, 32);
        unsat_proof.record_clause(&[1, -2]);
        unsat_proof.finish_unsat();

        let unsat_path = unsat_dir.join("proof.out");
        assert!(unsat_path.exists(), "expected UNSAT proof file to exist");
        assert!(
            !unsat_dir.join("proof.out.tmp").exists(),
            "expected temp proof file to be renamed away"
        );
        let unsat_text = fs::read_to_string(&unsat_path).expect("failed to read UNSAT proof");
        assert!(
            unsat_text.contains("1 -2 0\n"),
            "expected learned clause to be serialized before finalization"
        );
        assert!(
            unsat_text.ends_with("0\n"),
            "expected UNSAT proof to end with the empty clause"
        );

        let sat_dir = make_temp_dir("proof-sat");
        let mut sat_proof = ProofLog::new(&sat_dir, 32);
        sat_proof.record_clause(&[1, 2, 3]);
        sat_proof.finish_sat();

        assert!(
            !sat_dir.join("proof.out").exists(),
            "did not expect SAT run to leave a final proof file behind"
        );
        assert!(
            !sat_dir.join("proof.out.tmp").exists(),
            "did not expect SAT run to leave a temp proof file behind"
        );
    }

    #[test]
    fn test_unsat_proof_logs_learned_clause_before_empty_clause() {
        let clauses = vec![
            vec![1, 2],
            vec![-1, 2],
            vec![1, -2],
            vec![-1, -2],
        ];
        let proof_dir = make_temp_dir("solver-unsat-proof");
        let mut s = make_solver(2, clauses);
        assert!(!s.solve_to_output(proof_dir.to_str().expect("utf8 temp dir")));
        assert!(
            s.learned_clause_count() > 0,
            "expected solver to learn at least one clause on this UNSAT instance"
        );

        let proof_text = fs::read_to_string(proof_dir.join("proof.out"))
            .expect("failed to read emitted proof");
        let proof_lines: Vec<_> = proof_text.lines().collect();
        assert!(
            proof_lines.len() >= 2,
            "expected proof to contain at least one learned clause before the empty clause",
        );
        assert_eq!(
            proof_lines.last().copied(),
            Some("0"),
            "expected proof to end with the empty clause"
        );
        assert!(
            proof_lines[..proof_lines.len() - 1]
                .iter()
                .any(|line| *line != "0"),
            "expected at least one non-empty learned clause before the final empty clause"
        );
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
    fn test_backtrack_to_nonzero_preserves_target_level_assignments() {
        let mut s = make_solver(3, vec![]);
        s.decide(1);
        s.decide(2);
        s.decide(-3);

        s.backtrack(2);

        assert_eq!(s.trail, vec![1, 2]);
        assert_eq!(s.assignment[1], TRUE);
        assert_eq!(s.assignment[2], TRUE);
        assert_eq!(s.assignment[3], UNASSIGNED);
        assert_eq!(s.current_level(), 2);
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
    fn test_conflict_analysis_bumps_learned_reason_clause_activity() {
        let clauses = vec![vec![-1, 2], vec![-2, 3], vec![-2, -3]];
        let mut s = make_solver(3, clauses);
        s.clauses[1].learnt = true;
        s.clauses[2].learnt = true;

        s.decide(1);
        let conflict_clause_idx = s.propagate().expect("expected conflict after propagation");
        let (learned_clause, backtrack_level) = s.analyze_conflict(conflict_clause_idx);

        assert_eq!(learned_clause, vec![-2]);
        assert_eq!(backtrack_level, 0);
        assert!(s.clauses[2].activity > 0.0);
        assert!(s.clauses[1].activity > 0.0);
        assert_eq!(s.clauses[0].activity, 0.0);
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
}
