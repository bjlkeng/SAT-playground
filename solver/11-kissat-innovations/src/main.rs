use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

mod simp;

const UNASSIGNED: u8 = 0;
const TRUE: u8 = 1;
const FALSE: u8 = 2;
const NO_RELOC: usize = usize::MAX;
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
const LEARNTSIZE_ADJUST_START_CONFL: usize = 100;
const LEARNTSIZE_ADJUST_INC: f64 = 1.5;
const CLAUSE_ACTIVITY_WORDS: usize = 2;
const CLAUSE_MARK_MASK: u32 = 0b11;
const CLAUSE_LEARNT_BIT: u32 = 1 << 2;
const CLAUSE_HAS_EXTRA_BIT: u32 = 1 << 3;
const CLAUSE_RELOCED_BIT: u32 = 1 << 4;
const CLAUSE_SIZE_SHIFT: u32 = 5;
const CLAUSE_DELETED_MARK: u32 = 1;
const DEFAULT_BVE_GROW: isize = 0;
const DEFAULT_BVE_CLAUSE_LIMIT: isize = 20;
const DEFAULT_SUBSUMPTION_LIMIT: isize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Watcher {
    clause_idx: u32,
    blocker: i32,
}

#[derive(Debug, PartialEq, Eq)]
enum OriginalClauseInsertResult {
    Allocated(usize),
    Unit,
    Skipped,
    Unsat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InitialClauseMode {
    CanonicalSorted,
    CanonicalInputOrder,
    Raw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BranchMode {
    Minisat,
    Occurrence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReasonRef(usize);

const REASON_TAG_BITS: usize = 2;
const REASON_TAG_MASK: usize = (1 << REASON_TAG_BITS) - 1;
const REASON_NONE_TAG: usize = 0;
const REASON_UNIT_TAG: usize = 1;
const REASON_CLAUSE_TAG: usize = 2;
const REASON_BINARY_TAG: usize = 3;
const NO_REASON: ReasonRef = ReasonRef::none();

impl ReasonRef {
    #[inline(always)]
    const fn none() -> Self {
        Self(REASON_NONE_TAG)
    }

    #[inline(always)]
    const fn unit() -> Self {
        Self(REASON_UNIT_TAG)
    }

    #[inline(always)]
    fn clause(clause_idx: usize) -> Self {
        debug_assert!(clause_idx <= (usize::MAX >> REASON_TAG_BITS));
        Self((clause_idx << REASON_TAG_BITS) | REASON_CLAUSE_TAG)
    }

    #[inline(always)]
    fn binary(lit: i32) -> Self {
        let lit_idx = lit_to_index(lit);
        debug_assert!(lit_idx <= (usize::MAX >> REASON_TAG_BITS));
        Self((lit_idx << REASON_TAG_BITS) | REASON_BINARY_TAG)
    }

    #[inline(always)]
    fn is_none(self) -> bool {
        self.0 == REASON_NONE_TAG
    }

    #[inline(always)]
    fn is_unit(self) -> bool {
        self.0 == REASON_UNIT_TAG
    }

    #[inline(always)]
    fn is_clause(self) -> bool {
        self.0 & REASON_TAG_MASK == REASON_CLAUSE_TAG
    }

    #[inline(always)]
    fn as_clause(self) -> Option<usize> {
        if self.is_clause() {
            Some(self.0 >> REASON_TAG_BITS)
        } else {
            None
        }
    }

    #[inline(always)]
    fn as_binary(self) -> Option<i32> {
        if self.0 & REASON_TAG_MASK == REASON_BINARY_TAG {
            Some(index_to_lit(self.0 >> REASON_TAG_BITS))
        } else {
            None
        }
    }

    #[inline(always)]
    fn clause_idx(self) -> usize {
        debug_assert!(self.is_clause(), "reason is not an arena clause");
        self.0 >> REASON_TAG_BITS
    }
}

#[derive(Clone, Default)]
struct SolverStats {
    conflicts: u64,
    propagations: u64,
    decisions: u64,
    restarts: u64,
    simplifications: u64,
    reduce_db_calls: u64,
    deleted_clauses: u64,
    garbage_collections: u64,
    learned_clauses: u64,
    learned_clause_literals: u64,
    learned_clause_max_len: usize,
    learned_unit_clauses: u64,
    learned_binary_clauses: u64,
    learned_long_clauses: u64,
    deleted_words: u64,
    shrunk_words: u64,
    gc_copied_words: u64,
    gc_reclaimed_words: u64,
    gc_time_ns: u64,
    reduce_db_time_ns: u64,
    simplify_time_ns: u64,
    preprocess_eliminated_vars: u64,
    preprocess_resolvents: u64,
    preprocess_subsumed_clauses: u64,
    preprocess_strengthened_clauses: u64,
    preprocess_time_ns: u64,
    search_time_ns: u64,
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
    recorded_clauses: u64,
    recorded_literals: u64,
    recorded_bytes: u64,
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

fn ascii_i32_len(value: i32) -> usize {
    let mut len = if value < 0 { 1 } else { 0 };
    let mut value = value.unsigned_abs();
    len += 1;
    while value >= 10 {
        value /= 10;
        len += 1;
    }
    len
}

fn elapsed_ns(start: Instant) -> u64 {
    start.elapsed().as_nanos().min(u64::MAX as u128) as u64
}

impl ProofLog {
    fn disabled() -> Self {
        Self {
            mode: ProofMode::Disabled,
            recorded_clauses: 0,
            recorded_literals: 0,
            recorded_bytes: 0,
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
            recorded_clauses: 0,
            recorded_literals: 0,
            recorded_bytes: 0,
        }
    }

    fn record_clause(&mut self, clause: &[i32]) {
        self.recorded_clauses += 1;
        self.recorded_literals += clause.len() as u64;
        self.recorded_bytes += clause
            .iter()
            .map(|&lit| ascii_i32_len(lit) as u64 + 1)
            .sum::<u64>()
            + 2;
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
                self.recorded_clauses += 1;
                self.recorded_bytes += 2;
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
    /// MiniSat-style word arena: packed clause header, literals, and optional extra word.
    arena: Vec<u32>,
    /// references to original clauses inside `arena`
    original_clause_ids: Vec<usize>,
    /// MiniSat-style variable abstraction for original clauses, indexed by arena clause offset.
    clause_abstraction: Vec<u64>,
    /// live learned-clause ids, mirroring MiniSat's dedicated `learnts` vector
    learned_clause_ids: Vec<usize>,
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
    /// compact reason reference for each implied assignment; NO_REASON for decisions/root-unassigned vars
    reason: Vec<ReasonRef>,
    /// assigned literals in chronological order
    trail: Vec<i32>,
    /// number of level-0 assignments that must survive backtrack(0)
    root_trail_len: usize,
    /// trail index where each decision level starts
    trail_limits: Vec<usize>,
    /// next trail entry whose falsified literal still needs watcher processing
    propagate_head: usize,
    /// static tie-break rank used when activity ties
    branch_rank: Vec<usize>,
    /// binary max-heap of candidate branch variables ordered by activity
    branch_heap: Vec<u32>,
    /// current heap index for each variable, or BRANCH_NOT_IN_HEAP
    branch_pos: Vec<usize>,
    /// variables eligible for search branching; eliminated variables stay permanently false here
    decision_var: Vec<bool>,
    /// EVSIDS-style variable activity
    activity: Vec<f64>,
    /// additive bump applied to variables participating in recent conflicts
    activity_inc: f64,
    /// multiplicative decay factor for older activity
    activity_decay: f64,
    /// additive bump applied to learned clauses participating in recent conflicts
    clause_activity_inc: f64,
    /// multiplicative decay factor for older learned-clause activity
    clause_activity_decay: f64,
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
    /// resize learned-clause budget after preprocessing, matching MiniSat's solve-time setup
    reset_reduce_db_after_preprocess: bool,
    /// current conflict countdown until the next learned-budget adjustment
    learntsize_adjust_cnt: usize,
    /// floating-point copy of the current learned-budget adjustment window
    learntsize_adjust_confl: f64,
    /// current number of non-deleted learned clauses
    live_learned_clause_count: usize,
    /// live literal count in original clauses, maintained incrementally for simplify gating
    original_literals: usize,
    /// live literal count in learned clauses, maintained incrementally for simplify gating
    learned_literals: usize,
    /// total number of arena words currently wasted by deleted or shrunk clauses
    deleted_clause_words: usize,
    /// number of root assignments after the last successful simplify pass
    simplify_assigns: usize,
    /// remaining propagation budget before the next simplify pass should do work again
    simplify_props_remaining: i64,
    /// original unit clauses that must be enqueued at decision level 0
    root_unit_clauses: Vec<usize>,
    /// whether the formula already contains an empty clause
    has_empty_clause: bool,
    /// persistent solver consistency bit used by preprocessing insertions
    solver_ok: bool,
    /// MiniSat-simp preprocessing is available until the one-shot cleanup path turns it off
    use_simplification: bool,
    /// run bounded variable elimination during the one-shot preprocessing phase
    use_elim: bool,
    /// allowed clause-count growth for one variable-elimination step
    bve_grow: isize,
    /// maximum resolvent size allowed during variable elimination; negative means unlimited
    bve_clause_limit: isize,
    /// maximum candidate clause size checked during backward subsumption; negative means unlimited
    subsumption_lim: isize,
    /// number of root assignments already fed through backward subsumption
    bwdsub_assigns: usize,
    /// variables protected from elimination
    frozen: Vec<bool>,
    /// variables already eliminated from the live formula
    eliminated: Vec<bool>,
    /// lazy-cleaned occurrence lists for live original/preprocessed clauses, keyed by variable
    occurs: Vec<Vec<usize>>,
    /// dirty bits for occurrence lists after clause deletion
    occurs_dirty: Vec<bool>,
    /// literal occurrence counts for elimination cost, indexed by `lit_to_index`
    n_occ: Vec<usize>,
    /// packed MiniSat-style model-extension clauses
    elim_clauses: Vec<i32>,
    /// final SAT model snapshot, including assignments reconstructed for eliminated variables
    sat_model: Option<Vec<u8>>,
    /// scratch buffer for preprocessing-generated clauses
    scratch_preprocess_clause: Vec<i32>,
    /// scratch buffers reused during conflict analysis
    scratch_seen: Vec<u8>,
    scratch_resolved: Vec<u8>,
    scratch_learned: Vec<i32>,
    scratch_conflict_clause: Vec<i32>,
    scratch_bumped_vars: Vec<usize>,
    scratch_redundant_state: Vec<u8>,
    scratch_analyze_toclear: Vec<usize>,
    scratch_analyze_stack: Vec<(usize, i32)>,
    /// 0 = none, 1 = basic, 2 = deep
    ccmin_mode: u8,
    /// compatibility fallback for the older solver-10 conflict analyzer
    use_resolved_conflict_analysis: bool,
    /// expensive solver consistency checks, enabled with SAT_CHECK_INVARIANTS
    check_invariants: bool,
    stats: SolverStats,
}

#[inline(always)]
fn clause_make_header(size: usize, learnt: bool, has_extra: bool, mark: u32, reloced: bool) -> u32 {
    debug_assert!(
        size < (1usize << 27),
        "clause too large for packed header: {size}"
    );
    (mark & CLAUSE_MARK_MASK)
        | ((learnt as u32) << 2)
        | ((has_extra as u32) << 3)
        | ((reloced as u32) << 4)
        | ((size as u32) << CLAUSE_SIZE_SHIFT)
}

#[inline(always)]
fn clause_header_mark(header: u32) -> u32 {
    header & CLAUSE_MARK_MASK
}

#[inline(always)]
fn clause_header_learnt(header: u32) -> bool {
    (header & CLAUSE_LEARNT_BIT) != 0
}

#[inline(always)]
fn clause_header_has_extra(header: u32) -> bool {
    (header & CLAUSE_HAS_EXTRA_BIT) != 0
}

#[inline(always)]
fn clause_header_reloced(header: u32) -> bool {
    (header & CLAUSE_RELOCED_BIT) != 0
}

#[inline(always)]
fn clause_header_extra_words(header: u32) -> usize {
    if clause_header_has_extra(header) {
        CLAUSE_ACTIVITY_WORDS
    } else {
        0
    }
}

#[inline(always)]
fn clause_header_size(header: u32) -> usize {
    (header >> CLAUSE_SIZE_SHIFT) as usize
}

#[inline(always)]
fn lit_to_word(lit: i32) -> u32 {
    lit as u32
}

#[inline(always)]
fn word_to_lit(word: u32) -> i32 {
    word as i32
}

#[inline(always)]
fn clause_abstraction_from_lits(lits: &[i32]) -> u64 {
    let mut abstraction = 0u64;
    for &lit in lits {
        abstraction |= 1u64 << (lit.unsigned_abs() & 63);
    }
    abstraction
}

#[inline(always)]
unsafe fn words_as_lits(words: &[u32]) -> &[i32] {
    std::slice::from_raw_parts(words.as_ptr() as *const i32, words.len())
}

#[inline(always)]
fn clause_len_in_arena(arena: &[u32], clause_idx: usize) -> usize {
    clause_header_size(arena[clause_idx])
}

#[inline(always)]
fn clause_activity_in_arena(arena: &[u32], clause_idx: usize) -> f64 {
    debug_assert!(clause_header_has_extra(arena[clause_idx]));
    let extra_idx = clause_idx + 1 + clause_len_in_arena(arena, clause_idx);
    let bits = (arena[extra_idx] as u64) | ((arena[extra_idx + 1] as u64) << 32);
    f64::from_bits(bits)
}

#[inline(always)]
fn clause_lit_in_arena(arena: &[u32], clause_idx: usize, lit_pos: usize) -> i32 {
    debug_assert!(lit_pos < clause_len_in_arena(arena, clause_idx));
    word_to_lit(arena[clause_idx + 1 + lit_pos])
}

fn basic_lit_redundant(
    lit: i32,
    arena: &[u32],
    decision_level: &[usize],
    reason: &[ReasonRef],
    state: &[u8],
) -> bool {
    let var = lit.unsigned_abs() as usize;
    if reason[var].is_none() {
        return false;
    }
    let reason_idx = reason[var].clause_idx();

    let clause_len = clause_len_in_arena(arena, reason_idx);
    for lit_pos in 1..clause_len {
        let q = clause_lit_in_arena(arena, reason_idx, lit_pos);
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
    arena: &[u32],
    decision_level: &[usize],
    reason: &[ReasonRef],
    state: &mut [u8],
    toclear: &mut Vec<usize>,
    stack: &mut Vec<(usize, i32)>,
) -> bool {
    let mut lit = lit;
    debug_assert!({
        let var = lit.unsigned_abs() as usize;
        state[var] == REDUNDANT_UNDEF || state[var] == REDUNDANT_SOURCE
    });
    debug_assert!(!reason[lit.unsigned_abs() as usize].is_none());

    stack.clear();
    let mut clause_idx = reason[lit.unsigned_abs() as usize].clause_idx();
    let mut lit_pos = 1usize;

    loop {
        let clause_len = clause_len_in_arena(arena, clause_idx);
        if lit_pos < clause_len {
            let parent = clause_lit_in_arena(arena, clause_idx, lit_pos);
            if parent == lit {
                lit_pos += 1;
                continue;
            }
            let parent_var = parent.unsigned_abs() as usize;
            if state[parent_var] == REDUNDANT_SOURCE || state[parent_var] == REDUNDANT_REMOVABLE {
                lit_pos += 1;
                continue;
            }

            if decision_level[parent_var] == 0 {
                lit_pos += 1;
                continue;
            }

            if reason[parent_var].is_none() || state[parent_var] == REDUNDANT_FAILED {
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
            clause_idx = reason[parent_var].clause_idx();
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
            clause_idx = reason[lit.unsigned_abs() as usize].clause_idx();
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
    if lit > 0 {
        base
    } else {
        base + 1
    }
}

fn index_to_lit(index: usize) -> i32 {
    let var = (index / 2 + 1) as i32;
    if index % 2 == 0 {
        var
    } else {
        -var
    }
}

impl Solver {
    fn new(num_vars: usize, clauses: Vec<Vec<i32>>) -> Self {
        let original_clause_count = clauses.len();
        let branch_mode = parse_branch_mode();
        let mut occurrence_count = vec![0usize; num_vars + 1];
        for clause in &clauses {
            for &lit in clause {
                let var = lit.unsigned_abs() as usize;
                occurrence_count[var] += 1;
            }
        }
        let mut branch_order: Vec<u32> = (1..=num_vars as u32).collect();
        if branch_mode == BranchMode::Occurrence {
            branch_order.sort_unstable_by(|&lhs, &rhs| {
                occurrence_count[rhs as usize]
                    .cmp(&occurrence_count[lhs as usize])
                    .then_with(|| lhs.cmp(&rhs))
            });
        }
        let mut branch_rank = vec![0usize; num_vars + 1];
        for (rank, &var) in branch_order.iter().enumerate() {
            branch_rank[var as usize] = rank;
        }
        let default_phase = match branch_mode {
            BranchMode::Minisat => FALSE,
            BranchMode::Occurrence => TRUE,
        };

        let total_words: usize = clauses.iter().map(|clause| 1 + clause.len()).sum();
        let arena = Vec::with_capacity(total_words);
        let original_clause_ids = Vec::with_capacity(original_clause_count);
        let mut solver = Solver {
            arena,
            original_clause_ids,
            clause_abstraction: Vec::new(),
            learned_clause_ids: Vec::new(),
            watchers: vec![Vec::new(); num_vars.saturating_mul(2)],
            watch_scratch: Vec::new(),
            assignment: vec![UNASSIGNED; num_vars + 1],
            saved_phase: vec![default_phase; num_vars + 1],
            decision_level: vec![0; num_vars + 1],
            reason: vec![NO_REASON; num_vars + 1],
            trail: Vec::with_capacity(num_vars),
            root_trail_len: 0,
            trail_limits: Vec::new(),
            propagate_head: 0,
            branch_rank,
            branch_heap: Vec::with_capacity(num_vars),
            branch_pos: vec![BRANCH_NOT_IN_HEAP; num_vars + 1],
            decision_var: vec![true; num_vars + 1],
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
            reset_reduce_db_after_preprocess: true,
            learntsize_adjust_cnt: LEARNTSIZE_ADJUST_START_CONFL,
            learntsize_adjust_confl: LEARNTSIZE_ADJUST_START_CONFL as f64,
            live_learned_clause_count: 0,
            original_literals: 0,
            learned_literals: 0,
            deleted_clause_words: 0,
            simplify_assigns: 0,
            simplify_props_remaining: 0,
            root_unit_clauses: Vec::new(),
            has_empty_clause: false,
            solver_ok: true,
            use_simplification: true,
            use_elim: true,
            bve_grow: DEFAULT_BVE_GROW,
            bve_clause_limit: DEFAULT_BVE_CLAUSE_LIMIT,
            subsumption_lim: DEFAULT_SUBSUMPTION_LIMIT,
            bwdsub_assigns: 0,
            frozen: vec![false; num_vars + 1],
            eliminated: vec![false; num_vars + 1],
            occurs: vec![Vec::new(); num_vars + 1],
            occurs_dirty: vec![false; num_vars + 1],
            n_occ: vec![0; num_vars.saturating_mul(2)],
            elim_clauses: Vec::new(),
            sat_model: None,
            scratch_preprocess_clause: Vec::with_capacity(16),
            scratch_seen: vec![0; num_vars + 1],
            scratch_resolved: vec![0; num_vars + 1],
            scratch_learned: Vec::with_capacity(16),
            scratch_conflict_clause: Vec::with_capacity(16),
            scratch_bumped_vars: Vec::with_capacity(16),
            scratch_redundant_state: vec![0; num_vars + 1],
            scratch_analyze_toclear: Vec::with_capacity(16),
            scratch_analyze_stack: Vec::with_capacity(16),
            ccmin_mode: CCMIN_DEEP,
            use_resolved_conflict_analysis: false,
            check_invariants: parse_bool_env("SAT_CHECK_INVARIANTS", false),
            stats: SolverStats::default(),
        };
        match parse_initial_clause_mode() {
            InitialClauseMode::CanonicalSorted => {
                solver.add_initial_original_clauses(clauses, true);
            }
            InitialClauseMode::CanonicalInputOrder => {
                solver.add_initial_original_clauses(clauses, false);
            }
            InitialClauseMode::Raw => {
                solver.add_raw_initial_original_clauses(clauses);
            }
        }
        for &var in &branch_order {
            solver.push_branch_var(var as usize);
        }
        solver.maybe_check_invariants("new");
        solver
    }

    fn add_raw_initial_original_clauses(&mut self, clauses: Vec<Vec<i32>>) {
        for clause in clauses {
            let clause_idx = self.arena.len();
            let clause_len = clause.len();
            self.arena
                .push(clause_make_header(clause_len, false, false, 0, false));
            self.arena.extend(clause.iter().copied().map(lit_to_word));
            self.original_clause_ids.push(clause_idx);
            self.original_literals += clause_len;
            self.attach_clause(clause_idx, true);
        }
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
    fn clause_header(&self, clause_idx: usize) -> u32 {
        self.arena[clause_idx]
    }

    #[inline(always)]
    fn clause_len(&self, clause_idx: usize) -> usize {
        clause_header_size(self.clause_header(clause_idx))
    }

    #[inline(always)]
    fn clause_is_learnt(&self, clause_idx: usize) -> bool {
        clause_header_learnt(self.clause_header(clause_idx))
    }

    #[inline(always)]
    fn clause_is_deleted(&self, clause_idx: usize) -> bool {
        clause_header_mark(self.clause_header(clause_idx)) == CLAUSE_DELETED_MARK
    }

    #[inline(always)]
    fn clause_has_extra(&self, clause_idx: usize) -> bool {
        clause_header_has_extra(self.clause_header(clause_idx))
    }

    #[inline(always)]
    fn clause_word_len(&self, clause_idx: usize) -> usize {
        1 + self.clause_len(clause_idx) + clause_header_extra_words(self.clause_header(clause_idx))
    }

    #[inline(always)]
    fn clause_set_deleted(&mut self, clause_idx: usize, deleted: bool) {
        let header = self.clause_header(clause_idx);
        let mark = if deleted { CLAUSE_DELETED_MARK } else { 0 };
        self.arena[clause_idx] = clause_make_header(
            clause_header_size(header),
            clause_header_learnt(header),
            clause_header_has_extra(header),
            mark,
            clause_header_reloced(header),
        );
    }

    #[inline(always)]
    fn clause_activity(&self, clause_idx: usize) -> f64 {
        debug_assert!(self.clause_has_extra(clause_idx));
        let extra_idx = clause_idx + 1 + self.clause_len(clause_idx);
        let bits = (self.arena[extra_idx] as u64) | ((self.arena[extra_idx + 1] as u64) << 32);
        f64::from_bits(bits)
    }

    #[inline(always)]
    fn set_clause_activity(&mut self, clause_idx: usize, activity: f64) {
        debug_assert!(self.clause_has_extra(clause_idx));
        let extra_idx = clause_idx + 1 + self.clause_len(clause_idx);
        let bits = activity.to_bits();
        self.arena[extra_idx] = bits as u32;
        self.arena[extra_idx + 1] = (bits >> 32) as u32;
    }

    #[inline(always)]
    fn clause_lit(&self, clause_idx: usize, lit_pos: usize) -> i32 {
        debug_assert!(lit_pos < self.clause_len(clause_idx));
        word_to_lit(self.arena[clause_idx + 1 + lit_pos])
    }

    #[inline(always)]
    fn set_clause_lit(&mut self, clause_idx: usize, lit_pos: usize, lit: i32) {
        debug_assert!(lit_pos < self.clause_len(clause_idx));
        self.arena[clause_idx + 1 + lit_pos] = lit_to_word(lit);
    }

    #[inline(always)]
    fn swap_clause_lits(&mut self, clause_idx: usize, lhs: usize, rhs: usize) {
        debug_assert!(lhs < self.clause_len(clause_idx));
        debug_assert!(rhs < self.clause_len(clause_idx));
        self.arena.swap(clause_idx + 1 + lhs, clause_idx + 1 + rhs);
    }

    #[inline(always)]
    fn clause_slice(&self, clause_idx: usize) -> &[i32] {
        debug_assert!(
            !self.clause_is_deleted(clause_idx),
            "attempted to read deleted clause {clause_idx}"
        );
        let start = clause_idx + 1;
        let end = start + self.clause_len(clause_idx);
        unsafe { words_as_lits(&self.arena[start..end]) }
    }

    #[inline(always)]
    fn original_clause_abstraction(&self, clause_idx: usize) -> u64 {
        self.clause_abstraction
            .get(clause_idx)
            .copied()
            .unwrap_or(0)
    }

    fn set_original_clause_abstraction(&mut self, clause_idx: usize, abstraction: u64) {
        if self.clause_abstraction.len() <= clause_idx {
            self.clause_abstraction
                .resize(self.arena.len().max(clause_idx + 1), 0);
        }
        self.clause_abstraction[clause_idx] = abstraction;
    }

    fn ensure_original_clause_abstractions(&mut self) {
        if self.clause_abstraction.len() >= self.arena.len() {
            return;
        }
        self.clause_abstraction.clear();
        self.clause_abstraction.resize(self.arena.len(), 0);
        let original_clause_ids = self.original_clause_ids.clone();
        for clause_idx in original_clause_ids {
            if clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx) {
                self.clause_abstraction[clause_idx] =
                    clause_abstraction_from_lits(self.clause_slice(clause_idx));
            }
        }
    }

    fn clause_satisfied(&self, clause_idx: usize) -> bool {
        let clause_len = self.clause_len(clause_idx);
        for lit_pos in 0..clause_len {
            if self.lit_value(self.clause_lit(clause_idx, lit_pos)) == TRUE {
                return true;
            }
        }
        false
    }

    fn trim_root_false_literals(&mut self, clause_idx: usize) {
        let clause_len = self.clause_len(clause_idx);
        if clause_len <= 2 {
            return;
        }

        debug_assert_ne!(self.lit_value(self.clause_lit(clause_idx, 0)), FALSE);
        debug_assert_ne!(self.lit_value(self.clause_lit(clause_idx, 1)), FALSE);

        let mut write = 2usize;
        for read in 2..clause_len {
            let lit = self.clause_lit(clause_idx, read);
            if self.lit_value(lit) == FALSE {
                continue;
            }
            if write != read {
                self.set_clause_lit(clause_idx, write, lit);
            }
            write += 1;
        }

        if write == clause_len {
            return;
        }

        if self.clause_has_extra(clause_idx) {
            let old_extra_idx = clause_idx + 1 + clause_len;
            let new_extra_idx = clause_idx + 1 + write;
            for offset in 0..CLAUSE_ACTIVITY_WORDS {
                self.arena[new_extra_idx + offset] = self.arena[old_extra_idx + offset];
            }
        }

        let header = self.clause_header(clause_idx);
        self.arena[clause_idx] = clause_make_header(
            write,
            clause_header_learnt(header),
            clause_header_has_extra(header),
            clause_header_mark(header),
            clause_header_reloced(header),
        );
        let removed = clause_len - write;
        if self.clause_is_learnt(clause_idx) {
            self.learned_literals -= removed;
        } else {
            self.original_literals -= removed;
        }
        self.deleted_clause_words += removed;
        self.stats.shrunk_words += removed as u64;
    }

    fn delete_clause_for_simplify(&mut self, clause_idx: usize) {
        if self.clause_locked(clause_idx) {
            let implied_lit = self.clause_lit(clause_idx, 0);
            let var = implied_lit.unsigned_abs() as usize;
            self.reason[var] = NO_REASON;
        }

        let clause_len = self.clause_len(clause_idx);
        self.detach_clause(clause_idx);
        if self.clause_is_learnt(clause_idx) {
            self.learned_literals -= clause_len;
        } else {
            self.original_literals -= clause_len;
        }
        self.deleted_clause_words += self.clause_word_len(clause_idx);
        self.stats.deleted_words += self.clause_word_len(clause_idx) as u64;
        self.clause_set_deleted(clause_idx, true);
        self.stats.deleted_clauses += 1;
    }

    fn simplify_clause_list(&mut self, clause_ids: Vec<usize>) -> Vec<usize> {
        let mut kept = Vec::with_capacity(clause_ids.len());
        for clause_idx in clause_ids {
            if self.clause_is_deleted(clause_idx) {
                continue;
            }
            if self.clause_satisfied(clause_idx) {
                self.delete_clause_for_simplify(clause_idx);
                continue;
            }
            if !self.clause_is_learnt(clause_idx) {
                self.trim_root_false_literals(clause_idx);
            }
            kept.push(clause_idx);
        }
        kept
    }

    fn total_live_clause_literals(&self) -> usize {
        self.original_literals + self.learned_literals
    }

    fn maybe_check_invariants(&self, context: &str) {
        if self.check_invariants {
            self.check_invariants(context);
        }
    }

    fn check_invariants(&self, context: &str) {
        let num_vars = self.assignment.len().saturating_sub(1);
        assert_eq!(
            self.saved_phase.len(),
            self.assignment.len(),
            "{context}: saved phase length mismatch"
        );
        assert_eq!(
            self.decision_level.len(),
            self.assignment.len(),
            "{context}: decision level length mismatch"
        );
        assert_eq!(
            self.reason.len(),
            self.assignment.len(),
            "{context}: reason length mismatch"
        );
        assert_eq!(
            self.decision_var.len(),
            self.assignment.len(),
            "{context}: decision-var length mismatch"
        );
        assert_eq!(
            self.eliminated.len(),
            self.assignment.len(),
            "{context}: eliminated length mismatch"
        );
        assert!(
            self.root_trail_len <= self.trail.len(),
            "{context}: root trail length exceeds trail length"
        );
        assert!(
            self.propagate_head <= self.trail.len(),
            "{context}: propagation head exceeds trail length"
        );

        let current_level = self.current_level();
        let mut previous_limit = self.root_trail_len;
        for (level, &limit) in self.trail_limits.iter().enumerate() {
            assert!(
                limit >= self.root_trail_len && limit <= self.trail.len(),
                "{context}: decision level {} starts outside the trail",
                level + 1
            );
            assert!(
                limit >= previous_limit,
                "{context}: trail limits are not monotonic"
            );
            previous_limit = limit;
            if limit < self.trail.len() {
                let decision_var = self.trail[limit].unsigned_abs() as usize;
                assert_eq!(
                    self.reason[decision_var],
                    NO_REASON,
                    "{context}: decision literal at level {} has a reason",
                    level + 1
                );
            }
        }

        let mut seen_on_trail = vec![false; self.assignment.len()];
        for (trail_pos, &lit) in self.trail.iter().enumerate() {
            let var = lit.unsigned_abs() as usize;
            assert!(
                (1..=num_vars).contains(&var),
                "{context}: trail literal {lit} has invalid variable"
            );
            assert!(
                !seen_on_trail[var],
                "{context}: variable {var} appears twice on the trail"
            );
            seen_on_trail[var] = true;
            assert_eq!(
                self.lit_value(lit),
                TRUE,
                "{context}: trail literal {lit} is not true"
            );
            let level = self.decision_level[var];
            assert!(
                level <= current_level,
                "{context}: variable {var} has invalid decision level {level}"
            );
            if trail_pos < self.root_trail_len {
                assert_eq!(
                    level, 0,
                    "{context}: root trail literal {lit} has non-root level"
                );
            }
        }

        for var in 1..=num_vars {
            if self.assignment[var] == UNASSIGNED {
                assert_eq!(
                    self.decision_level[var], 0,
                    "{context}: unassigned variable {var} has a decision level"
                );
                assert_eq!(
                    self.reason[var], NO_REASON,
                    "{context}: unassigned variable {var} has a reason"
                );
                continue;
            }

            assert!(
                seen_on_trail[var],
                "{context}: assigned variable {var} is missing from the trail"
            );
            let reason_ref = self.reason[var];
            if reason_ref.is_none() || reason_ref.is_unit() {
                continue;
            }
            assert!(
                reason_ref.as_binary().is_none(),
                "{context}: binary reason appeared before binary reason support"
            );
            let reason_idx = reason_ref.clause_idx();
            assert!(
                reason_idx < self.arena.len(),
                "{context}: variable {var} has out-of-range reason {reason_idx}"
            );
            assert!(
                !self.clause_is_deleted(reason_idx),
                "{context}: variable {var} uses deleted reason clause {reason_idx}"
            );
            let assigned_lit = if self.assignment[var] == TRUE {
                var as i32
            } else {
                -(var as i32)
            };
            assert!(
                self.clause_slice(reason_idx).contains(&assigned_lit),
                "{context}: reason clause {reason_idx} does not contain assigned literal {assigned_lit}"
            );
        }

        let mut live_original_literals = 0usize;
        let mut live_learned_literals = 0usize;
        let mut seen_clause_ids = vec![false; self.arena.len()];
        for &clause_idx in &self.original_clause_ids {
            self.check_live_clause_id(context, clause_idx, false, &mut seen_clause_ids);
            live_original_literals += self.clause_len(clause_idx);
        }
        for &clause_idx in &self.learned_clause_ids {
            self.check_live_clause_id(context, clause_idx, true, &mut seen_clause_ids);
            live_learned_literals += self.clause_len(clause_idx);
        }
        assert_eq!(
            live_original_literals, self.original_literals,
            "{context}: original literal count mismatch"
        );
        assert_eq!(
            live_learned_literals, self.learned_literals,
            "{context}: learned literal count mismatch"
        );
        assert_eq!(
            self.live_learned_clause_count,
            self.learned_clause_ids.len(),
            "{context}: live learned clause count mismatch"
        );

        for (watch_index, watch_list) in self.watchers.iter().enumerate() {
            let watched_lit = index_to_lit(watch_index);
            for watcher in watch_list {
                let clause_idx = watcher.clause_idx as usize;
                assert!(
                    clause_idx < self.arena.len(),
                    "{context}: watcher references out-of-range clause {clause_idx}"
                );
                if self.clause_is_deleted(clause_idx) {
                    continue;
                }
                let clause_len = self.clause_len(clause_idx);
                assert!(clause_len > 0, "{context}: watcher on empty clause");
                let first = self.clause_lit(clause_idx, 0);
                if clause_len == 1 {
                    assert_eq!(
                        watched_lit, first,
                        "{context}: unit clause {clause_idx} is watched by wrong literal"
                    );
                    continue;
                }
                let second = self.clause_lit(clause_idx, 1);
                assert!(
                    watched_lit == first || watched_lit == second,
                    "{context}: watcher literal {watched_lit} is not watched by clause {clause_idx}"
                );
                assert!(
                    self.clause_slice(clause_idx).contains(&watcher.blocker),
                    "{context}: blocker {} is not in clause {}",
                    watcher.blocker,
                    clause_idx
                );
            }
        }

        for &clause_idx in self
            .original_clause_ids
            .iter()
            .chain(self.learned_clause_ids.iter())
        {
            let clause_len = self.clause_len(clause_idx);
            if clause_len == 0 {
                continue;
            }
            let first = self.clause_lit(clause_idx, 0);
            let first_watch_count = self.live_watch_count(first, clause_idx);
            if clause_len == 1 {
                assert_eq!(
                    first_watch_count, 1,
                    "{context}: unit clause {clause_idx} does not have exactly one live watch"
                );
            } else {
                let second = self.clause_lit(clause_idx, 1);
                let second_watch_count = self.live_watch_count(second, clause_idx);
                if first == second {
                    assert_eq!(
                        first_watch_count, 2,
                        "{context}: duplicate-watch clause {clause_idx} watch count mismatch"
                    );
                } else {
                    assert_eq!(
                        first_watch_count, 1,
                        "{context}: clause {clause_idx} first watch count mismatch"
                    );
                    assert_eq!(
                        second_watch_count, 1,
                        "{context}: clause {clause_idx} second watch count mismatch"
                    );
                }
            }
        }

        let mut seen_heap_vars = vec![false; self.assignment.len()];
        for (heap_idx, &var_u32) in self.branch_heap.iter().enumerate() {
            let var = var_u32 as usize;
            assert!(
                (1..=num_vars).contains(&var),
                "{context}: branch heap contains invalid variable {var}"
            );
            assert!(
                !seen_heap_vars[var],
                "{context}: branch heap contains duplicate variable {var}"
            );
            seen_heap_vars[var] = true;
            assert_eq!(
                self.branch_pos[var], heap_idx,
                "{context}: branch heap position mismatch for variable {var}"
            );
            assert!(
                self.decision_var[var],
                "{context}: branch heap contains non-decision variable {var}"
            );
        }
        for var in 1..=num_vars {
            if self.branch_pos[var] == BRANCH_NOT_IN_HEAP {
                assert!(
                    !self.decision_var[var] || self.assignment[var] != UNASSIGNED,
                    "{context}: branchable unassigned variable {var} is missing from the heap"
                );
            } else {
                assert!(
                    seen_heap_vars[var],
                    "{context}: branch position set for variable {var} not in heap"
                );
            }
        }

        for &clause_idx in &self.root_unit_clauses {
            assert!(
                clause_idx < self.arena.len(),
                "{context}: root unit clause id out of range"
            );
            if !self.clause_is_deleted(clause_idx) {
                assert_eq!(
                    self.clause_len(clause_idx),
                    1,
                    "{context}: root unit clause list contains non-unit clause"
                );
            }
        }

        if !self.clause_abstraction.is_empty() {
            assert!(
                self.clause_abstraction.len() >= self.arena.len(),
                "{context}: clause abstraction vector is too short"
            );
            for &clause_idx in &self.original_clause_ids {
                assert_eq!(
                    self.clause_abstraction[clause_idx],
                    clause_abstraction_from_lits(self.clause_slice(clause_idx)),
                    "{context}: original clause abstraction mismatch"
                );
            }
        }

        if !self.occurs.is_empty() {
            for var in 1..self.occurs.len().min(self.assignment.len()) {
                for &clause_idx in &self.occurs[var] {
                    assert!(
                        clause_idx < self.arena.len(),
                        "{context}: occurrence list references out-of-range clause"
                    );
                    if self.clause_is_deleted(clause_idx) {
                        continue;
                    }
                    assert!(
                        self.clause_slice(clause_idx)
                            .iter()
                            .any(|&lit| lit.unsigned_abs() as usize == var),
                        "{context}: occurrence list for variable {var} contains unrelated clause {clause_idx}"
                    );
                }
            }
        }

        self.check_elim_clause_stack(context);
    }

    fn check_live_clause_id(
        &self,
        context: &str,
        clause_idx: usize,
        learnt: bool,
        seen_clause_ids: &mut [bool],
    ) {
        assert!(
            clause_idx < self.arena.len(),
            "{context}: live clause id {clause_idx} is out of range"
        );
        assert!(
            !self.clause_is_deleted(clause_idx),
            "{context}: live clause list contains deleted clause {clause_idx}"
        );
        assert_eq!(
            self.clause_is_learnt(clause_idx),
            learnt,
            "{context}: clause {clause_idx} is in the wrong live list"
        );
        assert!(
            !seen_clause_ids[clause_idx],
            "{context}: duplicate live clause id {clause_idx}"
        );
        seen_clause_ids[clause_idx] = true;
    }

    fn live_watch_count(&self, lit: i32, clause_idx: usize) -> usize {
        self.watchers[self.lit_index(lit)]
            .iter()
            .filter(|watcher| {
                watcher.clause_idx as usize == clause_idx
                    && !self.clause_is_deleted(watcher.clause_idx as usize)
            })
            .count()
    }

    fn check_elim_clause_stack(&self, context: &str) {
        let mut end = self.elim_clauses.len();
        while end > 0 {
            let len = self.elim_clauses[end - 1];
            assert!(len > 0, "{context}: zero-length model-extension clause");
            let len = len as usize;
            assert!(
                len < end,
                "{context}: malformed model-extension clause length {len}"
            );
            let start = end - 1 - len;
            for &lit in &self.elim_clauses[start..end - 1] {
                let var = lit.unsigned_abs() as usize;
                assert!(
                    var > 0 && var < self.assignment.len(),
                    "{context}: model-extension literal {lit} has invalid variable"
                );
            }
            end = start;
        }
    }

    fn push_branch_var(&mut self, var: usize) {
        if !self.decision_var[var]
            || self.assignment[var] != UNASSIGNED
            || self.branch_pos[var] != BRANCH_NOT_IN_HEAP
        {
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
            if self.decision_var[var] && self.assignment[var] == UNASSIGNED {
                self.branch_pos[var] = self.branch_heap.len();
                self.branch_heap.push(var as u32);
            }
        }
        for idx in (0..(self.branch_heap.len() / 2)).rev() {
            self.branch_heap_sift_down(idx);
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

    fn detach_clause_watcher_strict(&mut self, lit: i32, clause_idx: usize) {
        let watch_idx = self.lit_index(lit);
        let watch_list = &mut self.watchers[watch_idx];
        if let Some(pos) = watch_list
            .iter()
            .position(|watcher| watcher.clause_idx as usize == clause_idx)
        {
            watch_list.swap_remove(pos);
        } else {
            debug_assert!(
                false,
                "clause {clause_idx} missing watcher for literal {lit}"
            );
        }
    }

    fn detach_clause_strict(&mut self, clause_idx: usize) {
        let clause_len = self.clause_len(clause_idx);
        if self.clause_is_deleted(clause_idx) || clause_len == 0 {
            return;
        }
        self.detach_clause_watcher_strict(self.clause_lit(clause_idx, 0), clause_idx);
        if clause_len > 1 {
            self.detach_clause_watcher_strict(self.clause_lit(clause_idx, 1), clause_idx);
        }
    }

    fn attach_clause(&mut self, clause_idx: usize, track_root_unit: bool) {
        debug_assert!(
            !self.clause_is_deleted(clause_idx),
            "attempted to attach deleted clause {clause_idx}"
        );
        match self.clause_len(clause_idx) {
            0 => {
                self.has_empty_clause = true;
            }
            1 => {
                let lit = self.clause_lit(clause_idx, 0);
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
                let first = self.clause_lit(clause_idx, 0);
                let second = self.clause_lit(clause_idx, 1);
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

    fn detach_clause(&mut self, _clause_idx: usize) {
        // Lazy detach: deleted clauses are compacted out of watch lists during propagation or GC.
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
    fn enqueue(&mut self, lit: i32, reason: ReasonRef) -> bool {
        let var = lit.unsigned_abs() as usize;
        let target_value = if lit > 0 { TRUE } else { FALSE };
        let current = self.assignment[var];
        if current == UNASSIGNED {
            let current_level = self.current_level();
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
            if !self.enqueue(lit, ReasonRef::clause(clause_idx)) {
                return false;
            }
        }
        true
    }

    fn propagate(&mut self) -> Option<usize> {
        let start_head = self.propagate_head;
        while self.propagate_head < self.trail.len() {
            let false_lit = -self.trail[self.propagate_head];
            self.propagate_head += 1;
            self.stats.propagations += 1;
            let watch_idx = self.lit_index(false_lit);
            let mut pending = std::mem::take(&mut self.watchers[watch_idx]);
            let mut read = 0usize;
            let mut write = 0usize;

            while read < pending.len() {
                let watcher = pending[read];
                read += 1;
                let clause_idx = watcher.clause_idx as usize;
                if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
                    continue;
                }
                let clause_len = self.clause_len(clause_idx);
                if clause_len == 1 {
                    let unit_lit = self.clause_lit(clause_idx, 0);
                    match self.lit_value(unit_lit) {
                        TRUE => {
                            pending[write] = watcher;
                            write += 1;
                        }
                        FALSE => {
                            pending[write] = watcher;
                            write += 1;
                            while read < pending.len() {
                                pending[write] = pending[read];
                                write += 1;
                                read += 1;
                            }
                            pending.truncate(write);
                            self.watchers[watch_idx] = pending;
                            return Some(clause_idx);
                        }
                        UNASSIGNED => {
                            if !self.enqueue(unit_lit, ReasonRef::clause(clause_idx)) {
                                pending[write] = watcher;
                                write += 1;
                                while read < pending.len() {
                                    pending[write] = pending[read];
                                    write += 1;
                                    read += 1;
                                }
                                pending.truncate(write);
                                self.watchers[watch_idx] = pending;
                                return Some(clause_idx);
                            }
                            pending[write] = watcher;
                            write += 1;
                        }
                        _ => unreachable!(),
                    }
                    continue;
                }

                if self.lit_value(watcher.blocker) == TRUE {
                    pending[write] = watcher;
                    write += 1;
                    continue;
                }

                if self.clause_lit(clause_idx, 0) == false_lit {
                    self.swap_clause_lits(clause_idx, 0, 1);
                }
                if self.clause_lit(clause_idx, 1) != false_lit {
                    continue;
                }

                let first = self.clause_lit(clause_idx, 0);
                let updated_watcher = Watcher {
                    clause_idx: watcher.clause_idx,
                    blocker: first,
                };
                if first != watcher.blocker && self.lit_value(first) == TRUE {
                    pending[write] = updated_watcher;
                    write += 1;
                    continue;
                }

                let mut moved_watch = false;
                for lit_pos in 2..clause_len {
                    let candidate = self.clause_lit(clause_idx, lit_pos);
                    if self.lit_value(candidate) != FALSE {
                        self.set_clause_lit(clause_idx, 1, candidate);
                        self.set_clause_lit(clause_idx, lit_pos, false_lit);
                        let new_watch_idx = self.lit_index(candidate);
                        self.watchers[new_watch_idx].push(updated_watcher);
                        moved_watch = true;
                        break;
                    }
                }

                if moved_watch {
                    continue;
                }

                pending[write] = updated_watcher;
                write += 1;
                if self.lit_value(first) == FALSE
                    || !self.enqueue(first, ReasonRef::clause(clause_idx))
                {
                    while read < pending.len() {
                        pending[write] = pending[read];
                        write += 1;
                        read += 1;
                    }
                    pending.truncate(write);
                    self.watchers[watch_idx] = pending;
                    return Some(clause_idx);
                }
            }

            pending.truncate(write);
            self.watchers[watch_idx] = pending;
        }

        self.simplify_props_remaining -= (self.propagate_head - start_head) as i64;

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
        if self.activity[var] > 1e100 {
            for value in &mut self.activity[1..] {
                *value *= 1e-100;
            }
            self.activity_inc *= 1e-100;
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
        if clause_idx >= self.arena.len() {
            return;
        }
        if !self.clause_is_learnt(clause_idx) || self.clause_is_deleted(clause_idx) {
            return;
        }

        let new_activity = self.clause_activity(clause_idx) + self.clause_activity_inc;
        self.set_clause_activity(clause_idx, new_activity);
        if new_activity > 1e20 {
            let scale = 1e-20;
            let learned_clause_ids = self.learned_clause_ids.clone();
            for learned_clause_idx in learned_clause_ids {
                let scaled = self.clause_activity(learned_clause_idx) * scale;
                self.set_clause_activity(learned_clause_idx, scaled);
            }
            self.clause_activity_inc *= scale;
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
        self.restart_conflict_limit = self
            .restart_unit
            .saturating_mul(Self::luby_value(self.restart_luby_index));
    }

    fn pick_branch_lit(&mut self) -> Option<i32> {
        while let Some(var) = self.branch_heap_pop_best() {
            if !self.decision_var[var] || self.assignment[var] != UNASSIGNED {
                continue;
            }

            return Some(if self.saved_phase[var] == FALSE {
                -(var as i32)
            } else {
                var as i32
            });
        }
        None
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

    fn debug_assert_clause_asserting_after_backtrack(
        &self,
        learned_clause: &[i32],
        backtrack_level: usize,
    ) {
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

    fn simplify(&mut self) -> bool {
        let simplify_start = Instant::now();
        let result = self.simplify_inner();
        self.stats.simplify_time_ns += elapsed_ns(simplify_start);
        self.maybe_check_invariants("simplify");
        result
    }

    fn simplify_inner(&mut self) -> bool {
        debug_assert_eq!(self.current_level(), 0);
        self.stats.simplifications += 1;

        if self.has_empty_clause || self.propagate().is_some() {
            return false;
        }

        if self.root_trail_len == self.simplify_assigns || self.simplify_props_remaining > 0 {
            return true;
        }

        let learned_clause_ids = std::mem::take(&mut self.learned_clause_ids);
        self.learned_clause_ids = self.simplify_clause_list(learned_clause_ids);
        self.live_learned_clause_count = self.learned_clause_ids.len();

        let original_clause_ids = std::mem::take(&mut self.original_clause_ids);
        self.original_clause_ids = self.simplify_clause_list(original_clause_ids);

        self.maybe_garbage_collect();
        self.rebuild_branch_queue();
        self.simplify_assigns = self.root_trail_len;
        self.simplify_props_remaining = self.total_live_clause_literals() as i64;
        true
    }

    fn add_clause(&mut self, clause: Vec<i32>) -> usize {
        self.add_clause_from_slice(&clause)
    }

    fn add_clause_from_slice(&mut self, clause: &[i32]) -> usize {
        let clause_idx = self.arena.len();
        let clause_len = clause.len();
        self.arena
            .push(clause_make_header(clause_len, true, true, 0, false));
        self.arena.extend(clause.iter().copied().map(lit_to_word));
        let activity_bits = 0.0f64.to_bits();
        self.arena.push(activity_bits as u32);
        self.arena.push((activity_bits >> 32) as u32);
        self.learned_clause_ids.push(clause_idx);
        self.live_learned_clause_count += 1;
        self.learned_literals += clause_len;
        self.stats.learned_clauses += 1;
        self.stats.learned_clause_literals += clause_len as u64;
        self.stats.learned_clause_max_len = self.stats.learned_clause_max_len.max(clause_len);
        match clause_len {
            0 | 1 => self.stats.learned_unit_clauses += 1,
            2 => self.stats.learned_binary_clauses += 1,
            _ => self.stats.learned_long_clauses += 1,
        }
        self.attach_clause(clause_idx, false);
        clause_idx
    }

    fn clause_locked(&self, clause_idx: usize) -> bool {
        if self.clause_is_deleted(clause_idx) || self.clause_len(clause_idx) == 0 {
            return false;
        }
        let implied_lit = self.clause_lit(clause_idx, 0);
        let var = implied_lit.unsigned_abs() as usize;
        self.lit_value(implied_lit) == TRUE && self.reason[var].as_clause() == Some(clause_idx)
    }

    fn reduce_db_enabled(&self) -> bool {
        self.reduce_db_limit != usize::MAX
    }

    fn reset_learned_budget_after_preprocess(&mut self) {
        if !self.reset_reduce_db_after_preprocess || !self.reduce_db_enabled() {
            return;
        }

        self.reduce_db_limit =
            ((self.original_clause_ids.len() as f64) * LEARNTSIZE_FACTOR) as usize;
        self.learntsize_adjust_cnt = LEARNTSIZE_ADJUST_START_CONFL;
        self.learntsize_adjust_confl = LEARNTSIZE_ADJUST_START_CONFL as f64;
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
        debug_assert!(
            clause_idx < self.arena.len(),
            "invalid clause index {clause_idx}"
        );
        debug_assert!(
            self.clause_is_learnt(clause_idx),
            "only learned clauses may be deleted"
        );
        debug_assert!(
            !self.clause_is_deleted(clause_idx),
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
        debug_assert!(
            clause_idx < self.arena.len(),
            "invalid clause index {clause_idx}"
        );
        debug_assert!(
            self.clause_is_learnt(clause_idx),
            "only learned clauses may be deleted"
        );
        debug_assert!(
            !self.clause_is_deleted(clause_idx),
            "clause {clause_idx} already deleted"
        );
        debug_assert!(
            !self
                .reason
                .iter()
                .any(|&reason| reason.as_clause() == Some(clause_idx)),
            "cannot delete clause {clause_idx} while it is still a live reason"
        );
        let learned_pos = self
            .learned_clause_ids
            .iter()
            .position(|&learned_clause_idx| learned_clause_idx == clause_idx)
            .expect("deleted learned clause missing from learned-clause list");
        self.learned_clause_ids.swap_remove(learned_pos);
        self.live_learned_clause_count = self.live_learned_clause_count.saturating_sub(1);
        self.learned_literals -= self.clause_len(clause_idx);
        self.deleted_clause_words += self.clause_word_len(clause_idx);
        self.stats.deleted_words += self.clause_word_len(clause_idx) as u64;
        self.clause_set_deleted(clause_idx, true);
        self.stats.deleted_clauses += 1;
    }

    fn mark_clause_deleted_already_unlinked(&mut self, clause_idx: usize) {
        debug_assert!(
            clause_idx < self.arena.len(),
            "invalid clause index {clause_idx}"
        );
        debug_assert!(
            self.clause_is_learnt(clause_idx),
            "only learned clauses may be deleted"
        );
        debug_assert!(
            !self.clause_is_deleted(clause_idx),
            "clause {clause_idx} already deleted"
        );
        debug_assert!(
            !self
                .reason
                .iter()
                .any(|&reason| reason.as_clause() == Some(clause_idx)),
            "cannot delete clause {clause_idx} while it is still a live reason"
        );
        self.live_learned_clause_count = self.live_learned_clause_count.saturating_sub(1);
        self.learned_literals -= self.clause_len(clause_idx);
        self.deleted_clause_words += self.clause_word_len(clause_idx);
        self.stats.deleted_words += self.clause_word_len(clause_idx) as u64;
        self.clause_set_deleted(clause_idx, true);
        self.stats.deleted_clauses += 1;
    }

    fn maybe_garbage_collect(&mut self) {
        if self.deleted_clause_words == 0 {
            return;
        }
        if self.deleted_clause_words.saturating_mul(3) < self.arena.len() {
            return;
        }
        self.garbage_collect();
    }

    fn garbage_collect(&mut self) {
        let gc_start = Instant::now();
        let old_word_count = self.arena.len();
        self.stats.garbage_collections += 1;
        let mut reloc = vec![NO_RELOC; self.arena.len()];
        let live_clause_count = self.original_clause_ids.len() + self.learned_clause_ids.len();
        let live_word_count: usize = self
            .original_clause_ids
            .iter()
            .chain(self.learned_clause_ids.iter())
            .map(|&clause_idx| self.clause_word_len(clause_idx))
            .sum();

        let mut new_arena = Vec::with_capacity(live_word_count);
        let mut new_original_clause_ids = Vec::with_capacity(self.original_clause_ids.len());
        let mut new_learned_clause_ids = Vec::with_capacity(self.learned_clause_ids.len());
        debug_assert_eq!(
            live_clause_count,
            self.original_clause_ids.len() + self.learned_clause_ids.len()
        );

        let copy_clause = |old_clause_idx: usize,
                           arena: &[u32],
                           new_arena: &mut Vec<u32>,
                           reloc: &mut [usize]| {
            let new_clause_idx = new_arena.len();
            let old_end = old_clause_idx
                + clause_len_in_arena(arena, old_clause_idx)
                + 1
                + clause_header_extra_words(arena[old_clause_idx]);
            reloc[old_clause_idx] = new_clause_idx;
            new_arena.extend_from_slice(&arena[old_clause_idx..old_end]);
            new_clause_idx
        };

        for &old_clause_idx in &self.original_clause_ids {
            debug_assert!(
                !self.clause_is_deleted(old_clause_idx),
                "original clauses must stay live across garbage collection"
            );
            let new_clause_idx =
                copy_clause(old_clause_idx, &self.arena, &mut new_arena, &mut reloc);
            new_original_clause_ids.push(new_clause_idx);
        }
        for &old_clause_idx in &self.learned_clause_ids {
            debug_assert!(
                !self.clause_is_deleted(old_clause_idx),
                "live learned clauses must stay live across garbage collection"
            );
            let new_clause_idx =
                copy_clause(old_clause_idx, &self.arena, &mut new_arena, &mut reloc);
            new_learned_clause_ids.push(new_clause_idx);
        }

        for watch_list in &mut self.watchers {
            let mut write = 0usize;
            for read in 0..watch_list.len() {
                let mut watcher = watch_list[read];
                let new_idx = reloc[watcher.clause_idx as usize];
                if new_idx == NO_RELOC {
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
            if new_idx == NO_RELOC {
                continue;
            }
            watcher.clause_idx = new_idx as u32;
            self.watch_scratch[watch_scratch_write] = watcher;
            watch_scratch_write += 1;
        }
        self.watch_scratch.truncate(watch_scratch_write);

        for reason in &mut self.reason {
            let Some(old_idx) = reason.as_clause() else {
                continue;
            };
            let new_idx = reloc[old_idx];
            debug_assert_ne!(
                new_idx, NO_RELOC,
                "garbage collection removed a clause that is still a live reason"
            );
            *reason = ReasonRef::clause(new_idx);
        }

        let mut root_write = 0usize;
        for read in 0..self.root_unit_clauses.len() {
            let new_idx = reloc[self.root_unit_clauses[read]];
            if new_idx == NO_RELOC {
                continue;
            }
            self.root_unit_clauses[root_write] = new_idx;
            root_write += 1;
        }
        self.root_unit_clauses.truncate(root_write);

        self.arena = new_arena;
        self.original_clause_ids = new_original_clause_ids;
        self.learned_clause_ids = new_learned_clause_ids;
        if !self.clause_abstraction.is_empty() {
            self.clause_abstraction.clear();
            self.clause_abstraction.resize(self.arena.len(), 0);
            let original_clause_ids = self.original_clause_ids.clone();
            for clause_idx in original_clause_ids {
                self.clause_abstraction[clause_idx] =
                    clause_abstraction_from_lits(self.clause_slice(clause_idx));
            }
        }
        self.live_learned_clause_count = self.learned_clause_ids.len();
        self.deleted_clause_words = 0;
        self.stats.gc_copied_words += live_word_count as u64;
        self.stats.gc_reclaimed_words += old_word_count.saturating_sub(live_word_count) as u64;
        self.stats.gc_time_ns += elapsed_ns(gc_start);
        self.maybe_check_invariants("garbage_collect");
    }

    fn reduce_db(&mut self) {
        let reduce_start = Instant::now();
        self.stats.reduce_db_calls += 1;

        let arena = &self.arena;
        self.learned_clause_ids.sort_unstable_by(|&lhs, &rhs| {
            let lhs_short = clause_len_in_arena(arena, lhs) <= 2;
            let rhs_short = clause_len_in_arena(arena, rhs) <= 2;
            lhs_short
                .cmp(&rhs_short)
                .then_with(|| {
                    clause_activity_in_arena(arena, lhs)
                        .total_cmp(&clause_activity_in_arena(arena, rhs))
                })
                .then_with(|| lhs.cmp(&rhs))
        });

        let candidate_count = self.learned_clause_ids.len();
        let extra_lim = if candidate_count == 0 {
            0.0
        } else {
            self.clause_activity_inc / candidate_count as f64
        };
        let half = candidate_count / 2;
        let mut write = 0usize;
        for idx in 0..candidate_count {
            let clause_idx = self.learned_clause_ids[idx];
            if self.clause_len(clause_idx) > 2
                && !self.clause_locked(clause_idx)
                && (idx < half || self.clause_activity(clause_idx) < extra_lim)
            {
                self.detach_clause(clause_idx);
                self.mark_clause_deleted_already_unlinked(clause_idx);
            } else {
                self.learned_clause_ids[write] = clause_idx;
                write += 1;
            }
        }
        self.learned_clause_ids.truncate(write);
        self.live_learned_clause_count = self.learned_clause_ids.len();

        self.maybe_garbage_collect();
        self.stats.reduce_db_time_ns += elapsed_ns(reduce_start);
        self.maybe_check_invariants("reduce_db");
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

        let arena = &self.arena;
        let decision_level = &self.decision_level;
        let reason = &self.reason;
        let mut write = 1usize;
        for read in 1..learned_clause.len() {
            let lit = learned_clause[read];
            let var = lit.unsigned_abs() as usize;
            let keep = if reason[var].is_none() {
                true
            } else if self.ccmin_mode == CCMIN_BASIC {
                !basic_lit_redundant(lit, arena, decision_level, reason, state)
            } else {
                !lit_redundant(lit, arena, decision_level, reason, state, toclear, stack)
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
        start_lit_pos: usize,
        current_level: usize,
        current_level_count: &mut usize,
    ) {
        if self.reduce_db_enabled() {
            self.bump_clause_activity(clause_idx);
        }

        let clause_len = self.clause_len(clause_idx);
        for lit_pos in start_lit_pos..clause_len {
            let lit = self.clause_lit(clause_idx, lit_pos);
            let var = lit.unsigned_abs() as usize;
            if self.scratch_seen[var] != 0
                || (self.use_resolved_conflict_analysis && self.scratch_resolved[var] != 0)
            {
                continue;
            }

            let level = self.decision_level[var];
            if level == 0 {
                continue;
            }

            self.scratch_seen[var] = 1;
            self.scratch_bumped_vars.push(var);
            if level == current_level {
                *current_level_count += 1;
            } else {
                self.scratch_learned.push(lit);
            }
        }
    }

    fn analyze_conflict_to_scratch(&mut self, conflict_clause_idx: usize) -> usize {
        let current_level = self.current_level();
        self.scratch_learned.clear();
        self.scratch_bumped_vars.clear();

        let mut current_level_count = 0usize;

        self.mark_clause_literals_for_analysis(
            conflict_clause_idx,
            0,
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
            if self.use_resolved_conflict_analysis {
                self.scratch_resolved[var] = 1;
            }
            current_level_count -= 1;
            if current_level_count == 0 {
                break lit;
            }

            if !self.reason[var].is_none() {
                let reason_idx = self.reason[var].clause_idx();
                let start_lit_pos = if self.use_resolved_conflict_analysis {
                    0
                } else {
                    1
                };
                self.mark_clause_literals_for_analysis(
                    reason_idx,
                    start_lit_pos,
                    current_level,
                    &mut current_level_count,
                );
            }
        };

        let mut learned_clause = std::mem::take(&mut self.scratch_conflict_clause);
        learned_clause.clear();
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

        self.scratch_conflict_clause = learned_clause;
        for &var in &self.scratch_bumped_vars {
            self.scratch_seen[var] = 0;
            self.scratch_resolved[var] = 0;
        }
        backtrack_level
    }

    fn analyze_conflict(&mut self, conflict_clause_idx: usize) -> (Vec<i32>, usize) {
        let backtrack_level = self.analyze_conflict_to_scratch(conflict_clause_idx);
        let learned_clause = self.scratch_conflict_clause.clone();
        (learned_clause, backtrack_level)
    }

    fn learned_clause_count(&self) -> usize {
        self.learned_clause_ids.len()
    }

    fn live_original_variable_count(&self) -> usize {
        let mut seen = vec![false; self.assignment.len()];
        let mut count = 0usize;
        for &clause_idx in &self.original_clause_ids {
            if clause_idx >= self.arena.len() || self.clause_is_deleted(clause_idx) {
                continue;
            }
            for lit_pos in 0..self.clause_len(clause_idx) {
                let var = self.clause_lit(clause_idx, lit_pos).unsigned_abs() as usize;
                if var < seen.len() && !seen[var] {
                    seen[var] = true;
                    count += 1;
                }
            }
        }
        count
    }

    fn solve(&mut self) -> bool {
        let mut proof_log = ProofLog::disabled();
        self.solve_with_proof(&mut proof_log)
    }

    fn solve_to_output(&mut self, output_dir: &str) -> bool {
        let mut proof_log = if parse_bool_env("SAT_PROOF", true) {
            ProofLog::new(output_dir, PROOF_BUFFER_CAPACITY)
        } else {
            ProofLog::disabled()
        };
        let sat = self.solve_with_proof(&mut proof_log);
        if sat {
            proof_log.finish_sat();
        } else {
            proof_log.finish_unsat();
        }
        sat
    }

    fn solve_with_proof(&mut self, proof_log: &mut ProofLog) -> bool {
        if !self.solver_ok || self.has_empty_clause || !self.enqueue_root_units() {
            return false;
        }

        if self.propagate().is_some() {
            return false;
        }
        self.maybe_check_invariants("root_propagate");

        let preprocess_start = Instant::now();
        if !self.eliminate(true, proof_log) {
            return false;
        }
        let preprocess_ns = elapsed_ns(preprocess_start);
        self.stats.preprocess_time_ns += preprocess_ns;
        self.reset_learned_budget_after_preprocess();
        self.maybe_check_invariants("preprocess");
        if env::var_os("SAT_TRACE_PREPROCESS").is_some() {
            eprintln!(
                "c preprocess seconds={:.3} eliminated={} resolvents={} subsumed={} strengthened={} original_vars={} original_clauses={} original_literals={} root_assigns={} deleted_clauses={} deleted_words={} shrunk_words={} gc={} gc_copied_words={} gc_reclaimed_words={} gc_ms={:.3} proof_clauses={} proof_bytes={} reduce_db_limit={}",
                preprocess_ns as f64 / 1e9,
                self.stats.preprocess_eliminated_vars,
                self.stats.preprocess_resolvents,
                self.stats.preprocess_subsumed_clauses,
                self.stats.preprocess_strengthened_clauses,
                self.live_original_variable_count(),
                self.original_clause_ids.len(),
                self.original_literals,
                self.trail.len(),
                self.stats.deleted_clauses,
                self.stats.deleted_words,
                self.stats.shrunk_words,
                self.stats.garbage_collections,
                self.stats.gc_copied_words,
                self.stats.gc_reclaimed_words,
                self.stats.gc_time_ns as f64 / 1e6,
                proof_log.recorded_clauses,
                proof_log.recorded_bytes,
                self.reduce_db_limit,
            );
        }

        let trace_search_interval = parse_optional_usize_env("SAT_TRACE_SEARCH_INTERVAL") as u64;
        let mut next_search_trace = trace_search_interval;
        let search_start = Instant::now();
        let mut conflict = self.propagate();

        loop {
            match conflict {
                Some(conflict_clause_idx) => {
                    if self.current_level() == 0 {
                        self.stats.search_time_ns += elapsed_ns(search_start);
                        if trace_search_interval > 0 {
                            eprintln!(
                                "c search done result=UNSAT seconds={:.3} conflicts={} decisions={} propagations={} restarts={} learned={} learned_lits={} learned_max={} deleted_clauses={} deleted_words={} gc={} gc_ms={:.3} simplify_ms={:.3} reduce_db={} reduce_ms={:.3} proof_clauses={} proof_bytes={}",
                                self.stats.search_time_ns as f64 / 1e9,
                                self.stats.conflicts,
                                self.stats.decisions,
                                self.stats.propagations,
                                self.stats.restarts,
                                self.live_learned_clause_count,
                                self.stats.learned_clause_literals,
                                self.stats.learned_clause_max_len,
                                self.stats.deleted_clauses,
                                self.stats.deleted_words,
                                self.stats.garbage_collections,
                                self.stats.gc_time_ns as f64 / 1e6,
                                self.stats.simplify_time_ns as f64 / 1e6,
                                self.stats.reduce_db_calls,
                                self.stats.reduce_db_time_ns as f64 / 1e6,
                                proof_log.recorded_clauses,
                                proof_log.recorded_bytes,
                            );
                        }
                        return false;
                    }

                    self.stats.conflicts += 1;
                    if trace_search_interval > 0 && self.stats.conflicts >= next_search_trace {
                        eprintln!(
                            "c search seconds={:.3} conflicts={} decisions={} propagations={} restarts={} level={} trail={} learned={} learned_units={} learned_binary={} learned_long={} learned_max={} reduce_db={} orig_clauses={} orig_literals={} deleted_words={} gc={} proof_bytes={}",
                            search_start.elapsed().as_secs_f64(),
                            self.stats.conflicts,
                            self.stats.decisions,
                            self.stats.propagations,
                            self.stats.restarts,
                            self.current_level(),
                            self.trail.len(),
                            self.live_learned_clause_count,
                            self.stats.learned_unit_clauses,
                            self.stats.learned_binary_clauses,
                            self.stats.learned_long_clauses,
                            self.stats.learned_clause_max_len,
                            self.stats.reduce_db_calls,
                            self.original_clause_ids.len(),
                            self.original_literals,
                            self.stats.deleted_words,
                            self.stats.garbage_collections,
                            proof_log.recorded_bytes,
                        );
                        next_search_trace = next_search_trace.saturating_add(trace_search_interval);
                    }
                    let backtrack_level = self.analyze_conflict_to_scratch(conflict_clause_idx);
                    self.bump_analyzed_variable_activity();
                    self.decay_variable_activity();
                    if self.reduce_db_enabled() {
                        self.decay_clause_activity();
                        self.note_learnt_budget_conflict();
                    }
                    self.note_conflict();
                    let learned_clause = std::mem::take(&mut self.scratch_conflict_clause);
                    let asserting_lit = learned_clause[0];
                    proof_log.record_clause(&learned_clause);
                    if learned_clause.len() == 1 {
                        self.stats.learned_clauses += 1;
                        self.stats.learned_clause_literals += 1;
                        self.stats.learned_clause_max_len =
                            self.stats.learned_clause_max_len.max(1);
                        self.stats.learned_unit_clauses += 1;
                        debug_assert_eq!(backtrack_level, 0);
                        self.backtrack(0);
                        let inserted = self.enqueue(asserting_lit, NO_REASON);
                        if !inserted {
                            return false;
                        }
                        self.scratch_conflict_clause = learned_clause;
                        self.scratch_conflict_clause.clear();
                    } else {
                        let learned_clause_idx = self.add_clause_from_slice(&learned_clause);
                        self.scratch_conflict_clause = learned_clause;
                        self.scratch_conflict_clause.clear();
                        if self.reduce_db_enabled() {
                            self.bump_clause_activity(learned_clause_idx);
                        }

                        self.backtrack(backtrack_level);
                        self.debug_assert_clause_asserting_after_backtrack(
                            self.clause_slice(learned_clause_idx),
                            backtrack_level,
                        );
                        let inserted =
                            self.enqueue(asserting_lit, ReasonRef::clause(learned_clause_idx));
                        debug_assert!(inserted, "learned clause must be asserting after backtrack");
                    }
                    self.maybe_check_invariants("conflict");

                    conflict = self.propagate();
                }
                None => {
                    if self.perform_restart_if_pending() {
                        conflict = self.propagate();
                        continue;
                    }

                    if self.current_level() == 0 && !self.simplify() {
                        return false;
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
                        None => {
                            self.capture_sat_model();
                            self.stats.search_time_ns += elapsed_ns(search_start);
                            if trace_search_interval > 0 {
                                eprintln!(
                                    "c search done result=SAT seconds={:.3} conflicts={} decisions={} propagations={} restarts={} learned={} learned_lits={} learned_max={} deleted_clauses={} deleted_words={} gc={} gc_ms={:.3} simplify_ms={:.3} reduce_db={} reduce_ms={:.3} proof_clauses={} proof_bytes={}",
                                    self.stats.search_time_ns as f64 / 1e9,
                                    self.stats.conflicts,
                                    self.stats.decisions,
                                    self.stats.propagations,
                                    self.stats.restarts,
                                    self.live_learned_clause_count,
                                    self.stats.learned_clause_literals,
                                    self.stats.learned_clause_max_len,
                                    self.stats.deleted_clauses,
                                    self.stats.deleted_words,
                                    self.stats.garbage_collections,
                                    self.stats.gc_time_ns as f64 / 1e6,
                                    self.stats.simplify_time_ns as f64 / 1e6,
                                    self.stats.reduce_db_calls,
                                    self.stats.reduce_db_time_ns as f64 / 1e6,
                                    proof_log.recorded_clauses,
                                    proof_log.recorded_bytes,
                                );
                            }
                            return true;
                        }
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
                eprintln!("Invalid SAT_CCMIN_MODE={other}; expected none/basic/deep or 0/1/2");
                std::process::exit(2);
            }
        },
        Err(_) => CCMIN_DEEP,
    }
}

fn parse_initial_clause_mode() -> InitialClauseMode {
    match env::var("SAT_INITIAL_CLAUSE_MODE") {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "canonical" | "canonical-sorted" | "sorted" | "1" | "true" | "on" => {
                InitialClauseMode::CanonicalSorted
            }
            "input-order" | "canonical-input-order" | "preserve-order" => {
                InitialClauseMode::CanonicalInputOrder
            }
            "raw" | "off" | "0" | "false" => InitialClauseMode::Raw,
            other => {
                eprintln!(
                    "Invalid SAT_INITIAL_CLAUSE_MODE={other}; expected canonical-sorted/input-order/raw"
                );
                std::process::exit(2);
            }
        },
        Err(_) => InitialClauseMode::CanonicalSorted,
    }
}

fn parse_branch_mode() -> BranchMode {
    match env::var("SAT_BRANCH_MODE") {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "minisat" | "mini" | "var-order" | "var_order" | "var" => BranchMode::Minisat,
            "occurrence" | "occ" | "legacy" | "solver10" => BranchMode::Occurrence,
            other => {
                eprintln!("Invalid SAT_BRANCH_MODE={other}; expected minisat/occurrence");
                std::process::exit(2);
            }
        },
        Err(_) => BranchMode::Minisat,
    }
}

fn parse_bool_env(name: &str, default: bool) -> bool {
    match env::var(name) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" | "enabled" => true,
            "0" | "false" | "no" | "off" | "disabled" => false,
            other => {
                eprintln!("Invalid {name}={other}; expected boolean");
                std::process::exit(2);
            }
        },
        Err(_) => default,
    }
}

fn parse_use_resolved_conflict_analysis() -> bool {
    match env::var("SAT_CONFLICT_ANALYSIS_MODE") {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "minisat" | "mini" | "seen" => false,
            "resolved" | "solver10" | "legacy" => true,
            other => {
                eprintln!("Invalid SAT_CONFLICT_ANALYSIS_MODE={other}; expected minisat/resolved");
                std::process::exit(2);
            }
        },
        Err(_) => false,
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

fn parse_optional_usize_env(name: &str) -> usize {
    match env::var(name) {
        Ok(value) => match value.trim().parse::<usize>() {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("Invalid {name}={value:?}: {err}");
                std::process::exit(2);
            }
        },
        Err(_) => 0,
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
        assert_ne!(
            assignment[var], UNASSIGNED,
            "SAT model snapshot left variable {var} unassigned"
        );
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
    solver.use_resolved_conflict_analysis = parse_use_resolved_conflict_analysis();
    let reduce_db_limit_overridden = env::var_os("SAT_REDUCE_DB_INIT").is_some();
    let reduce_db_interval_overridden = env::var_os("SAT_REDUCE_DB_INTERVAL").is_some();
    solver.reduce_db_limit = parse_usize_env("SAT_REDUCE_DB_INIT", solver.reduce_db_limit);
    solver.reset_reduce_db_after_preprocess = parse_bool_env(
        "SAT_POST_PREPROCESS_REDUCE_DB_RESET",
        !(reduce_db_limit_overridden || reduce_db_interval_overridden),
    );
    if let Ok(value) = env::var("SAT_SUBSUMPTION_LIMIT") {
        solver.subsumption_lim = match value.trim().parse::<isize>() {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("Invalid SAT_SUBSUMPTION_LIMIT={value:?}: {err}");
                std::process::exit(2);
            }
        };
    }
    solver.learntsize_adjust_cnt =
        parse_usize_env("SAT_REDUCE_DB_INTERVAL", solver.learntsize_adjust_cnt);
    solver.learntsize_adjust_confl = solver.learntsize_adjust_cnt as f64;

    if solver.solve_to_output(output_dir) {
        println!("s SATISFIABLE");
        let model = solver
            .sat_model
            .as_ref()
            .expect("SAT solver returned without a model snapshot");
        print_assignment(model);
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
        if s.clause_len(clause_idx) < 2 {
            return None;
        }
        Some((s.clause_lit(clause_idx, 0), s.clause_lit(clause_idx, 1)))
    }

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

    fn rewrite_clause_for_test(s: &mut Solver, clause_idx: usize, lits: &[i32]) {
        assert_eq!(s.clause_len(clause_idx), lits.len());
        for (lit_pos, &lit) in lits.iter().enumerate() {
            s.set_clause_lit(clause_idx, lit_pos, lit);
        }
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
            s.reason[var] = ReasonRef::clause(reason_idx);
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
    fn test_reason_ref_is_one_word_and_decodes_tags() {
        assert_eq!(
            std::mem::size_of::<ReasonRef>(),
            std::mem::size_of::<usize>()
        );

        assert!(ReasonRef::none().is_none());
        assert!(ReasonRef::unit().is_unit());

        let clause = ReasonRef::clause(12345);
        assert_eq!(clause.as_clause(), Some(12345));
        assert_eq!(clause.as_binary(), None);

        let binary = ReasonRef::binary(-17);
        assert_eq!(binary.as_binary(), Some(-17));
        assert_eq!(binary.as_clause(), None);
    }

    #[test]
    fn test_invariant_check_accepts_normal_search_state() {
        let mut s = make_solver(4, vec![vec![1, 2, 3], vec![-1, 2], vec![-2, 4]]);
        s.check_invariants = true;

        assert!(s.enqueue_root_units());
        assert_eq!(s.propagate(), None);
        s.decide(-1);
        assert_eq!(s.propagate(), None);
        s.check_invariants("unit-test");
    }

    #[test]
    fn test_constructor_canonicalizes_original_clauses() {
        let s = make_solver(3, vec![vec![2, 1, 2], vec![1, -1, 3], vec![-3, 2, 1]]);

        assert_eq!(live_original_clauses(&s), vec![vec![1, 2], vec![1, 2, -3]]);
        assert_eq!(s.original_literals, 5);
    }

    #[test]
    fn test_constructor_turns_units_into_root_assignments() {
        let s = make_solver(3, vec![vec![1], vec![-1, 2, 3], vec![2, -2, 3]]);

        assert_eq!(s.assignment[1], TRUE);
        assert_eq!(s.root_trail_len, 1);
        assert!(s.root_unit_clauses.is_empty());
        assert_eq!(live_original_clauses(&s), vec![vec![2, 3]]);
    }

    #[test]
    fn test_constructor_detects_contradictory_units() {
        let s = make_solver(1, vec![vec![1], vec![-1]]);

        assert!(!s.solver_ok);
        assert!(s.has_empty_clause);
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
        let reason_clause_ids = [
            s.original_clause_ids[0],
            s.original_clause_ids[1],
            s.original_clause_ids[2],
            s.original_clause_ids[3],
        ];
        rewrite_clause_for_test(&mut s, reason_clause_ids[0], &[5, 3, 4]);
        rewrite_clause_for_test(&mut s, reason_clause_ids[1], &[2, 1, 5]);
        rewrite_clause_for_test(&mut s, reason_clause_ids[2], &[6, 1, 3, 4]);
        rewrite_clause_for_test(&mut s, reason_clause_ids[3], &[2, 6]);
        install_manual_state(
            &mut s,
            &[3, 4, 5, 1, 2, 6],
            &[0, 3],
            &[
                (5, reason_clause_ids[0]),
                (2, reason_clause_ids[1]),
                (6, reason_clause_ids[2]),
            ],
        );

        s.ccmin_mode = CCMIN_NONE;
        let (raw_learned, raw_backtrack) = s.analyze_conflict(reason_clause_ids[3]);
        assert_eq!(raw_learned, vec![-1, 3, 4, 5]);
        assert_eq!(raw_backtrack, 1);

        s.ccmin_mode = CCMIN_BASIC;
        let (basic_learned, basic_backtrack) = s.analyze_conflict(reason_clause_ids[3]);
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
        let reason_clause_ids = [
            s.original_clause_ids[0],
            s.original_clause_ids[1],
            s.original_clause_ids[2],
            s.original_clause_ids[3],
            s.original_clause_ids[4],
        ];
        rewrite_clause_for_test(&mut s, reason_clause_ids[0], &[5, 3, 4]);
        rewrite_clause_for_test(&mut s, reason_clause_ids[1], &[7, 5, 6]);
        rewrite_clause_for_test(&mut s, reason_clause_ids[2], &[2, 1, 7, 6]);
        rewrite_clause_for_test(&mut s, reason_clause_ids[3], &[8, 1, 3, 4]);
        rewrite_clause_for_test(&mut s, reason_clause_ids[4], &[2, 8]);
        install_manual_state(
            &mut s,
            &[3, 4, 6, 5, 7, 1, 2, 8],
            &[0, 5],
            &[
                (5, reason_clause_ids[0]),
                (7, reason_clause_ids[1]),
                (2, reason_clause_ids[2]),
                (8, reason_clause_ids[3]),
            ],
        );

        s.ccmin_mode = CCMIN_NONE;
        let (raw_learned, raw_backtrack) = s.analyze_conflict(reason_clause_ids[4]);
        assert_eq!(raw_learned, vec![-1, 3, 4, 7, 6]);
        assert_eq!(raw_backtrack, 1);

        s.ccmin_mode = CCMIN_BASIC;
        let (basic_learned, _) = s.analyze_conflict(reason_clause_ids[4]);
        assert_eq!(basic_learned, vec![-1, 3, 4, 7, 6]);

        s.ccmin_mode = CCMIN_DEEP;
        let (deep_learned, deep_backtrack) = s.analyze_conflict(reason_clause_ids[4]);
        assert_eq!(deep_learned, vec![-1, 3, 4, 6]);
        assert_eq!(deep_backtrack, 1);
    }

    #[test]
    fn test_deep_clause_minimization_recurses_through_learned_reasons() {
        let mut s = make_solver(7, vec![vec![5, 3]]);
        let learned_reason = s.add_clause(vec![7, -5, 3]);
        s.reason[5] = ReasonRef::clause(0);
        s.reason[7] = ReasonRef::clause(learned_reason);

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
        s.reason[5] = ReasonRef::clause(0);

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
        s.reason[5] = ReasonRef::clause(0);

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
        let _live = s.add_clause(vec![3, 1]);

        s.mark_clause_deleted(dead);
        s.garbage_collect();
        let relocated_live = s.learned_clause_ids[0];

        let watch_clause_ids: Vec<_> = s.watchers[s.lit_index(1)]
            .iter()
            .map(|watcher| watcher.clause_idx)
            .collect();
        assert_eq!(watch_clause_ids, vec![relocated_live as u32]);

        s.decide(-1);
        assert_eq!(s.propagate(), None);
        assert_eq!(s.assignment[3], TRUE);
        assert_eq!(s.reason[3].as_clause(), Some(relocated_live));
        assert_eq!(s.clause_slice(relocated_live), &[3, 1]);
    }

    #[test]
    fn test_clause_gc_relocates_reason_refs_for_live_assignments() {
        let mut s = make_solver(4, vec![]);
        let dead = s.add_clause(vec![4, 1]);
        let live = s.add_clause(vec![3, 1]);

        s.assignment[4] = TRUE;
        s.saved_phase[4] = TRUE;
        s.decision_level[4] = 1;
        s.reason[4] = ReasonRef::clause(live);
        s.trail.push(4);
        s.trail_limits.push(0);

        s.mark_clause_deleted(dead);
        s.garbage_collect();
        let relocated_live = s.learned_clause_ids[0];

        assert_eq!(s.reason[4].as_clause(), Some(relocated_live));
        assert_eq!(s.clause_slice(s.reason[4].clause_idx()), &[3, 1]);
    }

    #[test]
    fn test_delete_clause_lazily_removes_watchers_during_propagation() {
        let mut s = make_solver(3, vec![]);
        let clause_idx = s.add_clause(vec![3, 1, 2]);

        assert!(s.watchers[s.lit_index(3)]
            .iter()
            .any(|watcher| watcher.clause_idx as usize == clause_idx));
        assert!(s.watchers[s.lit_index(1)]
            .iter()
            .any(|watcher| watcher.clause_idx as usize == clause_idx));

        s.delete_clause(clause_idx);

        assert!(s.clause_is_deleted(clause_idx));
        assert_eq!(s.learned_clause_count(), 0);
        assert!(s.learned_clause_ids.is_empty());
        assert_eq!(s.stats.deleted_clauses, 1);
        assert!(s.watchers[s.lit_index(3)]
            .iter()
            .any(|watcher| watcher.clause_idx as usize == clause_idx));
        assert!(s.watchers[s.lit_index(1)]
            .iter()
            .any(|watcher| watcher.clause_idx as usize == clause_idx));

        s.decide(-3);
        assert_eq!(s.propagate(), None);
        assert!(s.watchers[s.lit_index(3)]
            .iter()
            .all(|watcher| watcher.clause_idx as usize != clause_idx));

        s.decide(-1);
        assert_eq!(s.propagate(), None);
        assert!(s.watchers[s.lit_index(1)]
            .iter()
            .all(|watcher| watcher.clause_idx as usize != clause_idx));
    }

    #[test]
    fn test_mark_clause_deleted_removes_clause_from_live_learned_list() {
        let mut s = make_solver(6, vec![]);
        let first = s.add_clause(vec![6, 1, 2]);
        let middle = s.add_clause(vec![5, 1, 3]);
        let tail = s.add_clause(vec![4, 2, 3]);

        assert_eq!(s.learned_clause_ids, vec![first, middle, tail]);

        s.mark_clause_deleted(middle);

        assert_eq!(s.learned_clause_ids.len(), 2);
        assert!(s.clause_is_deleted(middle));
        assert!(s.learned_clause_ids.contains(&first));
        assert!(s.learned_clause_ids.contains(&tail));
        assert!(!s.learned_clause_ids.contains(&middle));

        s.mark_clause_deleted(tail);

        assert_eq!(s.learned_clause_ids, vec![first]);
        assert!(s.clause_is_deleted(tail));
    }

    #[test]
    fn test_enqueue_keeps_assigned_variable_in_branch_heap_for_lazy_skip() {
        let mut s = make_solver(3, vec![]);

        assert_ne!(s.branch_pos[1], BRANCH_NOT_IN_HEAP);
        assert!(s.enqueue(1, NO_REASON));

        assert_ne!(s.branch_pos[1], BRANCH_NOT_IN_HEAP);
        for _ in 0..3 {
            let Some(lit) = s.pick_branch_lit() else {
                break;
            };
            assert_ne!(lit.unsigned_abs(), 1);
        }
        assert_eq!(s.branch_pos[1], BRANCH_NOT_IN_HEAP);
    }

    #[test]
    fn test_reduce_db_keeps_locked_and_binary_learned_clauses() {
        let mut s = make_solver(5, vec![]);
        let removable = s.add_clause(vec![5, 1, 2]);
        let binary = s.add_clause(vec![4, 1]);
        let locked = s.add_clause(vec![3, 1, 2]);

        s.set_clause_activity(removable, 0.0);
        s.set_clause_activity(binary, 0.0);
        s.set_clause_activity(locked, 10.0);

        s.assignment[3] = TRUE;
        s.saved_phase[3] = TRUE;
        s.decision_level[3] = 1;
        s.reason[3] = ReasonRef::clause(locked);
        s.trail.push(3);
        s.trail_limits.push(0);
        s.propagate_head = s.trail.len();

        s.reduce_db();

        assert_eq!(s.learned_clause_count(), 2);
        assert_eq!(s.stats.reduce_db_calls, 1);
        assert_eq!(s.stats.deleted_clauses, 1);
        assert_eq!(s.clause_slice(s.reason[3].clause_idx()), &[3, 1, 2]);

        s.decide(-1);
        assert_eq!(s.propagate(), None);
        assert_eq!(s.assignment[4], TRUE);
        assert!(
            s.watchers[s.lit_index(1)]
                .iter()
                .all(|watcher| !s.clause_is_deleted(watcher.clause_idx as usize)),
            "propagation should drop tombstoned watchers from scanned lists"
        );
    }

    #[test]
    fn test_top_level_simplify_removes_satisfied_clauses_and_trims_only_originals() {
        let mut s = make_solver(7, vec![vec![1, 3], vec![4, -1, 5]]);
        let satisfied_learned = s.add_clause(vec![2, -1]);
        let _trimmed_learned = s.add_clause(vec![6, -1, 7]);

        assert!(s.enqueue(1, NO_REASON));
        assert_eq!(s.propagate(), None);
        assert_eq!(s.assignment[1], TRUE);
        assert_eq!(s.assignment[2], TRUE);
        assert_eq!(s.reason[2].as_clause(), Some(satisfied_learned));

        assert!(s.simplify());

        assert_eq!(s.reason[2], NO_REASON);
        assert_eq!(s.original_clause_ids.len(), 1);
        assert_eq!(s.learned_clause_ids.len(), 1);
        assert_eq!(s.live_learned_clause_count, 1);
        assert_eq!(s.original_literals, 2);
        assert_eq!(s.learned_literals, 3);
        assert_eq!(s.total_live_clause_literals(), 5);

        let mut original_clauses: Vec<Vec<i32>> = s
            .original_clause_ids
            .iter()
            .map(|&clause_idx| s.clause_slice(clause_idx).to_vec())
            .collect();
        original_clauses.sort();
        assert_eq!(original_clauses, vec![vec![4, 5]]);

        let learned_clauses: Vec<Vec<i32>> = s
            .learned_clause_ids
            .iter()
            .map(|&clause_idx| s.clause_slice(clause_idx).to_vec())
            .collect();
        assert_eq!(learned_clauses, vec![vec![6, 7, -1]]);

        assert!(
            !s.learned_clause_ids.contains(&satisfied_learned),
            "satisfied learned clause should be removed from the live learned list"
        );
    }

    #[test]
    fn test_top_level_simplify_is_noop_without_new_root_assignments() {
        let mut s = make_solver(3, vec![vec![1, 2], vec![-1, 3]]);

        assert!(s.simplify());
        let original_ids = s.original_clause_ids.clone();
        let deleted_words = s.deleted_clause_words;

        assert!(s.simplify());
        assert_eq!(s.original_clause_ids, original_ids);
        assert_eq!(s.deleted_clause_words, deleted_words);
    }

    #[test]
    fn test_cdcl_solves_unsat_with_learned_unit_shortcut() {
        let clauses = vec![vec![1, 2], vec![-1, 2], vec![1, -2], vec![-1, -2]];
        let mut s = make_solver(2, clauses);
        s.use_elim = false;
        assert!(!s.solve());
        assert_eq!(
            s.learned_clause_count(),
            0,
            "unit learned clauses should be enqueued at root without storing watched clauses"
        );
        assert!(s.stats.conflicts > 0);
    }

    #[test]
    fn test_bve_eliminates_variable_and_extends_sat_model() {
        let clauses = vec![vec![1, 2, 3], vec![-1, 2, 4]];
        let mut s = make_solver(4, clauses.clone());
        s.frozen[2] = true;
        s.frozen[3] = true;
        s.frozen[4] = true;

        assert!(s.solve());
        assert!(s.eliminated[1], "expected BVE to eliminate x1");
        assert!(
            !s.decision_var[1],
            "eliminated variable must not remain branchable"
        );

        let model = s.sat_model.as_ref().expect("missing SAT model snapshot");
        assert_ne!(model[1], UNASSIGNED);
        assert_ne!(model[2], UNASSIGNED);
        assert_ne!(model[3], UNASSIGNED);
        assert_ne!(model[4], UNASSIGNED);
        for clause in &clauses {
            let sat = clause.iter().any(|&lit| {
                let var = lit.unsigned_abs() as usize;
                (lit > 0 && model[var] == TRUE) || (lit < 0 && model[var] == FALSE)
            });
            assert!(sat, "extended model does not satisfy {clause:?}");
        }
    }

    #[test]
    fn test_bve_can_detect_xor_unsat_before_cdcl_conflicts() {
        let clauses = vec![vec![1, 2], vec![-1, 2], vec![1, -2], vec![-1, -2]];
        let mut s = make_solver(2, clauses);

        assert!(!s.solve());
        assert_eq!(
            s.stats.conflicts, 0,
            "bounded elimination should derive the contradictory units before CDCL search"
        );
    }

    #[test]
    fn test_sat_model_snapshot_assigns_unconstrained_variables() {
        let mut s = make_solver(3, vec![]);

        assert!(s.solve());
        let model = s.sat_model.as_ref().expect("missing SAT model snapshot");
        assert_eq!(&model[1..], &[FALSE, FALSE, FALSE]);
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
    fn test_proof_log_tracks_recorded_clause_metrics() {
        let dir = make_temp_dir("proof-metrics");
        let mut proof = ProofLog::new(&dir, 64);

        proof.record_clause(&[12, -3]);
        proof.record_clause(&[]);

        assert_eq!(proof.recorded_clauses, 2);
        assert_eq!(proof.recorded_literals, 2);
        assert_eq!(proof.recorded_bytes, 10);
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
        let clauses = vec![vec![1, 2], vec![-1, 2], vec![1, -2], vec![-1, -2]];
        let proof_dir = make_temp_dir("solver-unsat-proof");
        let mut s = make_solver(2, clauses);
        assert!(!s.solve_to_output(proof_dir.to_str().expect("utf8 temp dir")));

        let proof_text =
            fs::read_to_string(proof_dir.join("proof.out")).expect("failed to read emitted proof");
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

        assert_eq!(s.pick_branch_lit(), Some(-2));

        s.decide(-2);
        assert_eq!(s.pick_branch_lit(), Some(-3));
    }

    #[test]
    fn test_pick_branch_lit_uses_saved_phase_for_selected_variable() {
        let mut s = make_solver(3, vec![vec![1, 2], vec![-1, 3]]);
        s.activity[1] = 1.0;
        s.activity[2] = 4.0;
        s.activity[3] = 2.0;
        s.rebuild_branch_queue();

        assert_eq!(s.pick_branch_lit(), Some(-2));

        s.saved_phase[2] = TRUE;
        s.rebuild_branch_queue();
        assert_eq!(s.pick_branch_lit(), Some(2));
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

        assert_eq!(s.pick_branch_lit(), Some(-1));

        s.decide(-1);
        assert_eq!(s.pick_branch_lit(), Some(-2));

        s.backtrack(0);
        assert_eq!(s.pick_branch_lit(), Some(-1));
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
        let mut s = make_solver(3, vec![vec![-1, 2]]);
        let learned_reason = s.add_clause(vec![-2, 3]);
        let learned_conflict = s.add_clause(vec![-2, -3]);

        s.decide(1);
        let conflict_clause_idx = s.propagate().expect("expected conflict after propagation");
        assert_eq!(conflict_clause_idx, learned_conflict);
        let (learned_clause, backtrack_level) = s.analyze_conflict(conflict_clause_idx);

        assert_eq!(learned_clause, vec![-2]);
        assert_eq!(backtrack_level, 0);
        assert!(s.clause_activity(learned_conflict) > 0.0);
        assert!(s.clause_activity(learned_reason) > 0.0);
    }

    #[test]
    fn test_conflict_bumps_variable_activity() {
        let clauses = vec![vec![1, 2], vec![-1, 2], vec![1, -2], vec![-1, -2]];
        let mut s = make_solver(2, clauses);
        s.use_elim = false;

        assert!(!s.solve());
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
