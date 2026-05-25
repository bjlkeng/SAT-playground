use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[cfg(test)]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(test)]
use std::cell::Cell;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

mod branch;
mod check;
mod config;
mod limits;
mod lit;
mod output;
mod simp;
mod stats;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod oracle_tests;

use branch::VmtfQueue;
use config::{
    BranchMode, ClauseMinMode, InitialClauseMode, PhasePolicy, ProofPolicy, ReducePolicy,
    RestartPolicy, SearchModePolicy, SolverConfig, VmtfMode,
};
use limits::{effective_memory_limit_bytes, LimitHit, RuntimeLimits};
use lit::{lit_to_index, lit_to_word, word_to_lit};
use output::{
    prepare_output_contract_dir, print_assignment, write_model_file, write_result_contract,
    OutputContract, OutputContractState, ProofCompleteness, ResultContractFields, SolveStatus,
    PROOF_OUT,
};
use stats::{
    json_stats_line, max_rss_mb, trace_full_line, FormulaStats, GcReason, InputIdentity,
    ProofStats, RunTimings, SolverStats, StatsJsonContext,
};

#[cfg(test)]
static TEST_ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
thread_local! {
    static TEST_COUNT_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
struct CountingAllocator;

#[cfg(test)]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        TEST_COUNT_ALLOCATIONS.with(|enabled| {
            if enabled.get() {
                TEST_ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            }
        });
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        TEST_COUNT_ALLOCATIONS.with(|enabled| {
            if enabled.get() {
                TEST_ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            }
        });
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: CountingAllocator = CountingAllocator;

#[cfg(test)]
fn reset_test_allocations() {
    TEST_ALLOCATIONS.store(0, Ordering::SeqCst);
    TEST_COUNT_ALLOCATIONS.with(|enabled| enabled.set(true));
}

#[cfg(test)]
fn stop_test_allocations() {
    TEST_COUNT_ALLOCATIONS.with(|enabled| enabled.set(false));
}

#[cfg(test)]
fn test_allocation_count() -> usize {
    TEST_ALLOCATIONS.load(Ordering::SeqCst)
}

const UNASSIGNED: u8 = 0;
const TRUE: u8 = 1;
const FALSE: u8 = 2;
const BRANCH_NOT_IN_HEAP: usize = usize::MAX;
const CCMIN_NONE: u8 = 0;
const CCMIN_BASIC: u8 = 1;
const CCMIN_DEEP: u8 = 2;
const CCMIN_INBLOCK: u8 = 3;
const REDUNDANT_UNDEF: u8 = 0;
const REDUNDANT_SOURCE: u8 = 1;
const REDUNDANT_REMOVABLE: u8 = 2;
const REDUNDANT_FAILED: u8 = 3;
const PROOF_BUFFER_CAPACITY: usize = 16 * 1024 * 1024;
const LEARNTSIZE_FACTOR: f64 = 1.0 / 3.0;
const LEARNTSIZE_INC: f64 = 1.1;
const LEARNTSIZE_ADJUST_START_CONFL: usize = 100;
const LEARNTSIZE_ADJUST_INC: f64 = 1.5;
const LBD_REDUCE_DB_INIT_CONFLICTS: usize = 1_000;
const LBD_REDUCE_DB_INTERVAL_CONFLICTS: usize = 1_000;
const LBD_REDUCE_DB_MIN_INTERVAL_CONFLICTS: u64 = 100;
const CLAUSE_ACTIVITY_WORDS: usize = 2;
const ORIGINAL_ABSTRACTION_WORDS: usize = 1;
const CLAUSE_MARK_MASK: u32 = 0b11;
const CLAUSE_LEARNT_BIT: u32 = 1 << 2;
const CLAUSE_HAS_EXTRA_BIT: u32 = 1 << 3;
const CLAUSE_RELOCED_BIT: u32 = 1 << 4;
const CLAUSE_SIZE_SHIFT: u32 = 5;
const CLAUSE_DELETED_MARK: u32 = 1;
const DEFAULT_BVE_GROW: isize = 0;
const DEFAULT_BVE_CLAUSE_LIMIT: isize = 20;
const DEFAULT_SUBSUMPTION_LIMIT: isize = 1000;
const OTFS_MAX_LEARNED_LEN: usize = 20;
const OTFS_MAX_EXTRA_LITS: usize = 4;
const INLINE_ABSTRACTION_CLAUSE_THRESHOLD: usize = 750_000;
const LAZY_DETACH_SMALL_CLAUSE_THRESHOLD: usize = 50_000;
const SORTED_SUBSUMPTION_MIN_LEN: usize = 2;
const TIER1_MAX_GLUE: u16 = 2;
const TIER2_MAX_GLUE: u16 = 6;
const TIER1_RELATIVE_NUMERATOR: u64 = 1;
const TIER1_RELATIVE_DENOMINATOR: u64 = 2;
const TIER2_RELATIVE_NUMERATOR: u64 = 9;
const TIER2_RELATIVE_DENOMINATOR: u64 = 10;
const MAX_USED_RECENTLY: u8 = 3;
const LEARNED_LIT_BUDGET_BASE: usize = 2_000;
const LEARNED_LIT_BUDGET_FACTOR: usize = 300;
const LBD_HARD_LEARNED_LIT_BUDGET_FACTOR: usize = 64;
const LBD_HARD_LEARNED_LIT_FORMULA_FACTOR: usize = 64;
const EMERGENCY_TIER1_MIN_AGE_CONFLICTS: u64 = 1_000;
const RESTART_FAST_ALPHA: f64 = 1.0 / 32.0;
const RESTART_SLOW_ALPHA: f64 = 1.0 / 4096.0;
const KISSAT_EMA_RESTART_MIN_CONFLICTS: u64 = 50;
const KISSAT_EMA_RESTART_MARGIN: f64 = 1.20;
const GC_GARBAGE_RATIO_NUMERATOR: usize = 1;
const GC_GARBAGE_RATIO_DENOMINATOR: usize = 3;
const GC_WATCHER_STALE_MIN: usize = 1_024;
const GC_WATCHER_STALE_RATIO_NUMERATOR: usize = 1;
const GC_WATCHER_STALE_RATIO_DENOMINATOR: usize = 10;

type ClauseRef = usize;

const NO_CLAUSE_REF: ClauseRef = usize::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BinaryClauseId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BinaryEdge {
    implied: i32,
    clause_id: BinaryClauseId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum BinaryOrigin {
    Original,
    LearnedConflict,
    Hbr,
    Transitive,
    Gate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
struct BinaryClause {
    clause_ref: ClauseRef,
    a: i32,
    b: i32,
    redundant: bool,
    deleted: bool,
    proof_logged: bool,
    origin: BinaryOrigin,
    used_count: u32,
    last_used_conflict: u64,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum BinaryImplications {
    Nested(Vec<Vec<BinaryEdge>>),
    Flat {
        edges: Vec<BinaryEdge>,
        offsets: Vec<u32>,
        dirty: bool,
    },
}

impl BinaryImplications {
    fn nested(lit_count: usize) -> Self {
        Self::Nested(vec![Vec::new(); lit_count])
    }

    fn lit_index(lit: i32) -> usize {
        lit_to_index(lit)
    }

    #[allow(dead_code)]
    fn edges_for(&self, lit: i32) -> &[BinaryEdge] {
        let idx = Self::lit_index(lit);
        match self {
            Self::Nested(edges) => edges.get(idx).map(Vec::as_slice).unwrap_or(&[]),
            Self::Flat { edges, offsets, .. } => {
                let Some(&start) = offsets.get(idx) else {
                    return &[];
                };
                let end = offsets.get(idx + 1).copied().unwrap_or(edges.len() as u32);
                &edges[start as usize..end as usize]
            }
        }
    }

    fn len_for(&self, lit: i32) -> usize {
        self.edges_for(lit).len()
    }

    fn edge_for(&self, lit: i32, idx: usize) -> BinaryEdge {
        self.edges_for(lit)[idx]
    }

    fn add_edge(&mut self, antecedent: i32, edge: BinaryEdge) {
        let idx = Self::lit_index(antecedent);
        match self {
            Self::Nested(edges) => {
                if let Some(list) = edges.get_mut(idx) {
                    list.push(edge);
                }
            }
            Self::Flat {
                edges,
                offsets,
                dirty,
            } => {
                let insert_at = offsets
                    .get(idx + 1)
                    .copied()
                    .map(|offset| offset as usize)
                    .unwrap_or(edges.len());
                edges.insert(insert_at, edge);
                for offset in offsets.iter_mut().skip(idx + 1) {
                    *offset = offset.saturating_add(1);
                }
                *dirty = true;
            }
        }
    }

    fn mark_deleted(&mut self, _id: BinaryClauseId) {
        if let Self::Flat { dirty, .. } = self {
            *dirty = true;
        }
    }

    #[allow(dead_code)]
    fn rebuild_flat_if_needed(&mut self) {
        if let Self::Flat { dirty, .. } = self {
            *dirty = false;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReasonRef {
    None,
    Clause(ClauseRef),
    Binary(BinaryClauseId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum Conflict {
    Clause(ClauseRef),
    Binary(BinaryClauseId),
    RootUnit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum SearchAccountingMode {
    NormalSearch,
    TemporaryAssumption {
        update_phase: bool,
        update_branch_stats: bool,
        update_restart_stats: bool,
        count_as_decision: bool,
    },
}

#[allow(dead_code)]
impl SearchAccountingMode {
    fn from_temporary_options(opts: TemporaryAssumptionOptions) -> Self {
        Self::TemporaryAssumption {
            update_phase: opts.update_phase,
            update_branch_stats: opts.update_branch_stats,
            update_restart_stats: opts.update_restart_stats,
            count_as_decision: opts.count_as_decision,
        }
    }

    fn is_temporary(self) -> bool {
        matches!(self, Self::TemporaryAssumption { .. })
    }

    fn update_phase(self) -> bool {
        match self {
            Self::NormalSearch => true,
            Self::TemporaryAssumption { update_phase, .. } => update_phase,
        }
    }

    fn update_branch_stats(self) -> bool {
        match self {
            Self::NormalSearch => true,
            Self::TemporaryAssumption {
                update_branch_stats,
                ..
            } => update_branch_stats,
        }
    }

    fn update_restart_stats(self) -> bool {
        match self {
            Self::NormalSearch => true,
            Self::TemporaryAssumption {
                update_restart_stats,
                ..
            } => update_restart_stats,
        }
    }

    fn count_as_decision(self) -> bool {
        match self {
            Self::NormalSearch => true,
            Self::TemporaryAssumption {
                count_as_decision, ..
            } => count_as_decision,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[allow(dead_code)]
struct TemporaryAssumptionOptions {
    update_phase: bool,
    update_branch_stats: bool,
    update_restart_stats: bool,
    count_as_decision: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TemporaryAssumptionStats {
    enqueues: u64,
    propagations: u64,
    conflicts: u64,
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
struct TemporaryAssumptionGuard {
    start_trail: usize,
    start_level: usize,
    start_root_trail_len: usize,
    start_propagate_head: usize,
    saved_accounting_mode: SearchAccountingMode,
}

#[allow(dead_code)]
struct TemporaryAssumptionCtx<'a> {
    solver: &'a mut Solver,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum EnqueueResult {
    Enqueued,
    AlreadyAssigned,
    Conflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
struct Budget {
    remaining: Option<u64>,
}

#[allow(dead_code)]
impl Budget {
    fn from_ticks(ticks: u64) -> Self {
        Self {
            remaining: Some(ticks),
        }
    }

    fn exhausted(self) -> bool {
        self.remaining == Some(0)
    }

    fn consume(&mut self, ticks: u64) {
        if let Some(remaining) = &mut self.remaining {
            *remaining = remaining.saturating_sub(ticks);
        }
    }
}

#[allow(dead_code)]
impl<'a> TemporaryAssumptionCtx<'a> {
    fn enqueue(&mut self, lit: i32) -> EnqueueResult {
        let var = lit.unsigned_abs() as usize;
        let target_value = if lit > 0 { TRUE } else { FALSE };
        match self.solver.assignment[var] {
            UNASSIGNED => {
                if self.solver.accounting_mode.count_as_decision() {
                    self.solver.stats.decisions += 1;
                }
                let inserted = self.solver.enqueue(lit, ReasonRef::None);
                debug_assert!(inserted, "temporary literal was checked as unassigned");
                self.solver.temporary_stats.enqueues += 1;
                EnqueueResult::Enqueued
            }
            current if current == target_value => EnqueueResult::AlreadyAssigned,
            _ => EnqueueResult::Conflict,
        }
    }

    fn propagate_budgeted(&mut self, budget: &mut Budget) -> Option<Conflict> {
        if budget.exhausted() {
            return None;
        }
        let before = self.solver.temporary_stats.propagations;
        let conflict = self.solver.propagate();
        let spent = self
            .solver
            .temporary_stats
            .propagations
            .saturating_sub(before);
        budget.consume(spent);
        if conflict.is_some() {
            self.solver.temporary_stats.conflicts += 1;
        }
        conflict
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LearnedId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LearnedMeta {
    lbd: u16,
    tier: u8,
    used_recently: u8,
    removable: bool,
    vivified: bool,
    created_at_conflict: u64,
}

impl Default for LearnedMeta {
    fn default() -> Self {
        Self {
            lbd: u16::MAX,
            tier: 2,
            used_recently: 0,
            removable: true,
            vivified: false,
            created_at_conflict: 0,
        }
    }
}

#[derive(Clone, Debug)]
struct MovingAverage {
    value: f64,
    initialized: bool,
    alpha: f64,
}

impl MovingAverage {
    fn new(alpha: f64) -> Self {
        Self {
            value: 0.0,
            initialized: false,
            alpha,
        }
    }

    fn update(&mut self, x: f64) {
        if !self.initialized {
            self.value = x;
            self.initialized = true;
        } else {
            self.value += self.alpha * (x - self.value);
        }
    }

    fn reset(&mut self) {
        self.value = 0.0;
        self.initialized = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchMode {
    Focused,
    Stable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reluctant {
    u: u64,
    v: u64,
}

impl Reluctant {
    fn new() -> Self {
        Self { u: 1, v: 1 }
    }

    fn current(self) -> u64 {
        self.v.max(1)
    }

    fn advance(&mut self) {
        self.u = self.u.saturating_add(1);
        self.v = Solver::luby_value_u64(self.u);
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct ReasonPinSet {
    pinned_clauses: Vec<ClauseRef>,
    pinned_binaries: Vec<BinaryClauseId>,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReduceCand {
    clause_idx: ClauseRef,
    lbd: u16,
    size: usize,
    used_recently: u8,
    activity_rank: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TierLimits {
    tier1_max_glue: u16,
    tier2_max_glue: u16,
}

impl TierLimits {
    const fn static_defaults() -> Self {
        Self {
            tier1_max_glue: TIER1_MAX_GLUE,
            tier2_max_glue: TIER2_MAX_GLUE,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ClauseDbMeasurement {
    arena_words_live: usize,
    arena_words_garbage: usize,
    learned_words_live: usize,
    original_words_live: usize,
    watchers_live: usize,
    watchers_stale: usize,
}

impl ClauseDbMeasurement {
    fn arena_garbage_ratio(self) -> f64 {
        let total = self
            .arena_words_live
            .saturating_add(self.arena_words_garbage);
        if total == 0 {
            0.0
        } else {
            self.arena_words_garbage as f64 / total as f64
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReasonCodeError {
    InvalidTag,
    ClauseOverflow,
    BinaryOverflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReasonCode(usize);

impl ReasonCode {
    const TAG_SHIFT: usize = usize::BITS as usize - 2;
    const TAG_MASK: usize = 0b11usize << Self::TAG_SHIFT;
    const PAYLOAD_MASK: usize = !Self::TAG_MASK;
    const CLAUSE_TAG: usize = 0usize << Self::TAG_SHIFT;
    const BINARY_TAG: usize = 1usize << Self::TAG_SHIFT;
    #[cfg(test)]
    const INVALID_TAG: usize = 2usize << Self::TAG_SHIFT;
    const NONE: Self = Self(usize::MAX);

    fn from_ref(reason: ReasonRef) -> Result<Self, ReasonCodeError> {
        match reason {
            ReasonRef::None => Ok(Self::NONE),
            ReasonRef::Clause(clause_idx) => {
                if clause_idx > Self::PAYLOAD_MASK {
                    Err(ReasonCodeError::ClauseOverflow)
                } else {
                    Ok(Self(Self::CLAUSE_TAG | clause_idx))
                }
            }
            ReasonRef::Binary(binary_id) => {
                let payload = binary_id.0 as usize;
                if payload > Self::PAYLOAD_MASK {
                    Err(ReasonCodeError::BinaryOverflow)
                } else {
                    Ok(Self(Self::BINARY_TAG | payload))
                }
            }
        }
    }

    fn as_ref(self) -> Result<ReasonRef, ReasonCodeError> {
        if self == Self::NONE {
            return Ok(ReasonRef::None);
        }
        let payload = self.0 & Self::PAYLOAD_MASK;
        match self.0 & Self::TAG_MASK {
            Self::CLAUSE_TAG => Ok(ReasonRef::Clause(payload)),
            Self::BINARY_TAG => {
                let payload =
                    u32::try_from(payload).map_err(|_| ReasonCodeError::BinaryOverflow)?;
                Ok(ReasonRef::Binary(BinaryClauseId(payload)))
            }
            _ => Err(ReasonCodeError::InvalidTag),
        }
    }

    fn as_ref_unchecked(self) -> ReasonRef {
        self.as_ref().expect("invalid encoded reason")
    }

    fn is_none(self) -> bool {
        self == Self::NONE
    }

    #[cfg(test)]
    fn from_raw(raw: usize) -> Self {
        Self(raw)
    }
}

const NO_REASON: ReasonCode = ReasonCode::NONE;

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

enum ProofMode {
    Disabled,
    Stream(ProofStream),
}

struct ProofStream {
    final_path: PathBuf,
    temp_path: PathBuf,
    file: fs::File,
    buffer: Vec<u8>,
    scratch: Vec<i32>,
    capacity: usize,
    trace: bool,
    clause_count: u64,
    literal_count: u64,
    deletion_count: u64,
    deletion_literal_count: u64,
    max_clause_len: usize,
    bytes_written: u64,
    flush_count: u64,
}

struct ProofLog {
    mode: ProofMode,
    stats: ProofStats,
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
            stats: ProofStats {
                state: "disabled",
                ..ProofStats::default()
            },
        }
    }

    fn new<P: AsRef<Path>>(output_dir: P, capacity: usize, trace: bool) -> Self {
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
                scratch: Vec::new(),
                capacity,
                trace,
                clause_count: 0,
                literal_count: 0,
                deletion_count: 0,
                deletion_literal_count: 0,
                max_clause_len: 0,
                bytes_written: 0,
                flush_count: 0,
            }),
            stats: ProofStats {
                temp_created: true,
                state: "streaming",
                ..ProofStats::default()
            },
        }
    }

    fn record_clause(&mut self, clause: &[i32]) {
        if let ProofMode::Stream(stream) = &mut self.mode {
            stream.clause_count += 1;
            stream.literal_count += clause.len() as u64;
            stream.max_clause_len = stream.max_clause_len.max(clause.len());
            Self::write_clause_line(stream, b"", clause);
        }
    }

    fn record_deletion(&mut self, clause: &[i32]) {
        if let ProofMode::Stream(stream) = &mut self.mode {
            stream.deletion_count += 1;
            stream.deletion_literal_count += clause.len() as u64;
            stream.max_clause_len = stream.max_clause_len.max(clause.len());
            Self::write_clause_line(stream, b"d ", clause);
        }
    }

    fn write_clause_line(stream: &mut ProofStream, prefix: &[u8], clause: &[i32]) {
        stream.scratch.clear();
        stream.scratch.extend_from_slice(clause);
        stream.scratch.sort_unstable_by(|&lhs, &rhs| {
            lhs.unsigned_abs()
                .cmp(&rhs.unsigned_abs())
                .then_with(|| lhs.cmp(&rhs))
        });

        stream.buffer.reserve(prefix.len() + clause.len() * 12 + 2);
        stream.buffer.extend_from_slice(prefix);
        for idx in 0..stream.scratch.len() {
            append_i32_ascii(&mut stream.buffer, stream.scratch[idx]);
            stream.buffer.push(b' ');
        }
        stream.buffer.extend_from_slice(b"0\n");
        if stream.buffer.len() >= stream.capacity {
            Self::flush_stream(stream);
        }
    }

    fn finish_sat(&mut self) {
        match std::mem::replace(&mut self.mode, ProofMode::Disabled) {
            ProofMode::Disabled => {}
            ProofMode::Stream(stream) => {
                self.stats = proof_stats_from_stream(&stream, "discarded");
                drop(stream.file);
                self.stats.temp_deleted = fs::remove_file(&stream.temp_path).is_ok();
            }
        }
    }

    fn finish_unknown(&mut self) {
        match std::mem::replace(&mut self.mode, ProofMode::Disabled) {
            ProofMode::Disabled => {}
            ProofMode::Stream(stream) => {
                self.stats = proof_stats_from_stream(&stream, "discarded-incomplete");
                self.stats.incomplete = true;
                drop(stream.file);
                self.stats.temp_deleted = fs::remove_file(&stream.temp_path).is_ok();
            }
        }
    }

    fn finish_unsat(&mut self) {
        match std::mem::replace(&mut self.mode, ProofMode::Disabled) {
            ProofMode::Disabled => {}
            ProofMode::Stream(mut stream) => {
                stream.clause_count += 1;
                stream
                    .buffer
                    .write_all(b"0\n")
                    .expect("Failed to buffer empty proof clause");
                Self::flush_stream(&mut stream);
                stream.file.flush().expect("Failed to flush proof file");
                let mut stats = proof_stats_from_stream(&stream, "finalized");
                stats.finalized = true;
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
                self.stats = stats;
                if stream.trace {
                    let proof_bytes = fs::metadata(&stream.final_path)
                        .map(|metadata| metadata.len())
                        .unwrap_or(stream.bytes_written);
                    eprintln!(
                        "c proof_detail additions={} addition_literals={} deletions={} deletion_literals={} max_clause_lits={} bytes={} flushes={} path={}",
                        stream.clause_count,
                        stream.literal_count,
                        stream.deletion_count,
                        stream.deletion_literal_count,
                        stream.max_clause_len,
                        proof_bytes,
                        stream.flush_count,
                        stream.final_path.display(),
                    );
                }
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
        stream.bytes_written += stream.buffer.len() as u64;
        stream.flush_count += 1;
        stream.buffer.clear();
    }

    fn snapshot(&self) -> ProofStats {
        self.stats.clone()
    }

    fn bytes_written_estimate(&self) -> u64 {
        match &self.mode {
            ProofMode::Disabled => self.stats.bytes_written,
            ProofMode::Stream(stream) => stream
                .bytes_written
                .saturating_add(stream.buffer.len() as u64),
        }
    }
}

fn proof_stats_from_stream(stream: &ProofStream, state: &'static str) -> ProofStats {
    ProofStats {
        added_clauses: stream.clause_count,
        deleted_clauses: stream.deletion_count,
        added_literals: stream.literal_count,
        deleted_literals: stream.deletion_literal_count,
        flushes: stream.flush_count,
        bytes_written: stream.bytes_written,
        max_clause_len: stream.max_clause_len as u64,
        temp_created: true,
        state,
        ..ProofStats::default()
    }
}

struct Solver {
    /// MiniSat-style word arena: packed clause header, literals, and optional extra word.
    arena: Vec<u32>,
    /// references to original clauses inside `arena`
    original_clause_ids: Vec<usize>,
    /// MiniSat-style variable abstraction for original clauses, indexed by arena clause offset.
    clause_abstraction: Vec<u64>,
    /// Store original-clause abstractions inline during large preprocessing passes.
    inline_original_abstractions: bool,
    /// All original/preprocessing clauses keep MiniSat-style variable/sign order.
    clauses_sorted_by_var: bool,
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
    /// target assignment polarity captured from the deepest unconflicted prefix in this phase block
    target_phase: Vec<u8>,
    /// best full-solve assignment polarity captured from the deepest unconflicted prefix
    best_phase: Vec<u8>,
    /// initial polarity used when no saved/target/best phase is available
    original_phase: Vec<u8>,
    /// deepest unconflicted trail length captured for target phase in the current phase block
    target_assigned: usize,
    /// deepest unconflicted trail length captured for best phase over the whole solve
    best_assigned: usize,
    /// monotonic phase-capture counter used by tests and future rephase scheduling
    phase_ticks: u64,
    /// selected phase policy; legacy preserves solver-10-compatible saved-phase branching
    phase_policy: PhasePolicy,
    /// opt-in stable-mode rephase hook for focused/stable search experiments
    rephase_enabled: bool,
    /// current step in the default best -> inverted -> original rephase cycle
    rephase_index: u8,
    /// global conflict count at which the next stable-mode restart may rephase
    rephase_at_conflicts: u64,
    /// conflict interval between scheduled rephase opportunities
    rephase_conflicts: u64,
    /// decision level of each variable assignment
    decision_level: Vec<usize>,
    /// encoded reason for each implied assignment; NONE for decisions/root-unassigned vars
    reason: Vec<ReasonCode>,
    /// binary reason literals, indexed by stable BinaryClauseId
    binary_reason_lits: Vec<[i32; 2]>,
    /// stable binary-clause metadata and proof/model/debug traceability
    binary_clauses: Vec<BinaryClause>,
    /// binary implication adjacency indexed by the assigned-true antecedent literal
    binary_implications: BinaryImplications,
    /// arena clause offset to stable BinaryClauseId + 1; 0 means not represented as binary
    binary_id_by_clause: Vec<u32>,
    /// scratch stamps reserved for generated binary deduplication in later HBR/transitive passes
    #[allow(dead_code)]
    binary_dedup_seen: Vec<u32>,
    #[allow(dead_code)]
    binary_dedup_stamp: u32,
    /// opt-in switch for binary implication propagation; default off for solver-10 parity
    binary_fast_path: bool,
    /// controls whether the current propagation/accounting path mutates normal search counters
    accounting_mode: SearchAccountingMode,
    /// counters for temporary assumptions, kept separate from normal search stats
    temporary_stats: TemporaryAssumptionStats,
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
    /// opt-in VMTF branch queue used by configured search modes
    vmtf_queue: Option<VmtfQueue>,
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
    /// selected restart policy; legacy Luby remains the default for solver-10 parity
    restart_policy: RestartPolicy,
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
    /// true after learning a unit clause; skips one post-propagation scheduling pass
    iterating: bool,
    /// fast EMA of recent learned-clause LBD values for Kissat/Glucose-style restarts
    restart_fast_lbd: MovingAverage,
    /// slow EMA of learned-clause LBD values for restart baseline
    restart_slow_lbd: MovingAverage,
    /// fast EMA of conflict decision levels, exposed for diagnostics and future policies
    restart_fast_level: MovingAverage,
    /// slow EMA of conflict decision levels, exposed for diagnostics and future policies
    restart_slow_level: MovingAverage,
    /// minimum conflicts between EMA restarts
    restart_min_conflicts: u64,
    /// next global conflict count where EMA restart conditions are checked
    restart_next_check_conflict: u64,
    /// threshold multiplier for fast LBD EMA over slow LBD EMA
    restart_margin: f64,
    /// threshold multiplier for blocking restarts when fast level EMA is high
    restart_block_margin: f64,
    /// opt-in Kissat-style partial restart that keeps the best decision-prefix trail
    restart_reuse_trail: bool,
    /// conflicts since the last EMA restart
    restart_conflicts_since_last: u64,
    /// current high-level search mode for focused/stable experiments
    search_mode: SearchMode,
    /// selected search-mode policy; single keeps solver-10-compatible behavior
    search_mode_policy: SearchModePolicy,
    /// selected VMTF activation mode; default keeps solver-10-compatible VSIDS branching
    vmtf_mode: VmtfMode,
    /// dynamic focused-mode glue thresholds computed from recent clause-use histograms
    focused_tier_limits: TierLimits,
    /// dynamic stable-mode glue thresholds computed from recent clause-use histograms
    stable_tier_limits: TierLimits,
    /// focused-mode clause-use histogram since the last focused retier pass
    focused_glue_recent: Vec<u64>,
    /// stable-mode clause-use histogram since the last stable retier pass
    stable_glue_recent: Vec<u64>,
    /// opt-in Kissat-style stable-mode gate based on propagation search ticks
    mode_use_ticks: bool,
    /// conflict count when the current search mode started
    mode_start_conflicts: u64,
    /// search tick count when the current search mode started
    mode_start_ticks: u64,
    /// decision count when the current search mode started
    mode_start_decisions: u64,
    /// wall-clock start for the currently active search-mode segment
    mode_wall_start: Instant,
    /// whether search-mode wall-clock attribution is currently active
    mode_wall_active: bool,
    /// number of focused/stable mode switches performed
    mode_switches: u64,
    /// absolute conflict count at which the next mode switch should happen
    mode_switch_at_conflicts: u64,
    /// absolute search tick count at which stable mode should switch back
    mode_switch_at_ticks: u64,
    /// base focused/stable interval before sqrt scaling
    mode_init_conflicts: u64,
    /// current conflict interval between focused/stable switches
    mode_interval: u64,
    /// multiplier applied to sqrt-scaled mode intervals
    mode_interval_scale: f64,
    /// state for reluctant restart scheduling
    reluctant: Reluctant,
    /// learned-clause budget threshold for running a database reduction pass
    reduce_db_limit: usize,
    /// minimum conflicts that must elapse between learned-clause database reductions
    reduce_db_min_interval: u64,
    /// global conflict count at which the last database reduction ran
    reduce_db_last_conflicts: Option<u64>,
    /// target learned-literal budget for LBD-tiered reduction
    learned_lit_budget: usize,
    /// hard learned-literal budget that allows emergency low-LBD demotion
    hard_learned_lit_budget: usize,
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
    /// emit detailed preprocessing counters when SAT_TRACE_PREPROCESS_DETAILS is set
    trace_preprocess_details: bool,
    /// run bounded variable elimination during the one-shot preprocessing phase
    use_elim: bool,
    /// run full backward subsumption rather than queue-only root/touched work
    full_bsr: bool,
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
    occurs: Vec<Vec<u32>>,
    /// dirty bits for occurrence lists after clause deletion
    occurs_dirty: Vec<bool>,
    /// dirty bits for occurrence lists after a clause is strengthened and loses a variable
    occurs_membership_dirty: Vec<bool>,
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
    otfs_delete_candidates: Vec<ClauseRef>,
    scratch_conflict_clause: Vec<i32>,
    scratch_bumped_vars: Vec<usize>,
    scratch_redundant_state: Vec<u8>,
    scratch_analyze_toclear: Vec<usize>,
    scratch_analyze_stack: Vec<(usize, i32, u32)>,
    /// 0 = none, 1 = basic, 2 = deep, 3 = in-block shrink
    ccmin_mode: u8,
    /// opt-in bounded learned-clause-only OTFS pass after learning; default off after profiling.
    otfs_enabled: bool,
    minimize_depth_limit: u32,
    /// compatibility fallback for the older solver-10 conflict analyzer
    use_resolved_conflict_analysis: bool,
    /// early Section 0 LBD instrumentation slice; default off and policy-neutral
    use_lbd: bool,
    /// opt-in reason-side LBD improvement; default off until LBD-tiered reduction is stable
    update_reason_lbd: bool,
    /// opt-in propagation-time reason LBD refresh; isolated after profile regression testing
    update_propagation_reason_lbd: bool,
    /// learned-clause reduction policy selected by validated configuration
    reduce_policy: ReducePolicy,
    /// opt-in guarded chronological backtracking; default off for solver-10 parity
    chrono_backtrack: bool,
    /// maximum current/assertion-level gap where chronological backtracking is considered
    chrono_max_delta: usize,
    /// stable learned-clause metadata, keyed by LearnedId rather than moving arena offsets
    learned_meta: Vec<LearnedMeta>,
    /// current arena clause reference for each stable learned id
    learned_clause_by_id: Vec<ClauseRef>,
    /// arena-offset to stable LearnedId map; stores id + 1 so 0 means absent
    learned_id_by_clause: Vec<u32>,
    /// scratch stamps used for allocation-free LBD/glue computation
    lbd_seen: Vec<u32>,
    lbd_stamp: u32,
    last_conflict_lbd: u16,
    sum_lbd: u64,
    num_lbd: u64,
    lbd_hist_1: u64,
    lbd_hist_2: u64,
    lbd_hist_3_5: u64,
    lbd_hist_6_10: u64,
    lbd_hist_gt_10: u64,
    reason_pin_generation: u64,
    reduce_delete_generation: u64,
    reduce_delete_mark: Vec<u64>,
    reduce_candidates: Vec<ReduceCand>,
    gc_pending_reason: GcReason,
    track_gc_detail_stats: bool,
    /// opt-in hot-path watcher diagnostics; default off for solver-10 parity
    hot_stats: bool,
    stats: SolverStats,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SolveOutcome {
    status: SolveStatus,
    unknown_reason: Option<&'static str>,
}

impl SolveOutcome {
    fn sat() -> Self {
        Self {
            status: SolveStatus::Sat,
            unknown_reason: None,
        }
    }

    fn unsat() -> Self {
        Self {
            status: SolveStatus::Unsat,
            unknown_reason: None,
        }
    }

    fn unknown(reason: &'static str) -> Self {
        Self {
            status: SolveStatus::Unknown,
            unknown_reason: Some(reason),
        }
    }

    fn termination_reason(self) -> &'static str {
        self.unknown_reason
            .unwrap_or_else(|| self.status.termination_reason())
    }
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
    if !clause_header_has_extra(header) {
        0
    } else if clause_header_learnt(header) {
        CLAUSE_ACTIVITY_WORDS
    } else {
        ORIGINAL_ABSTRACTION_WORDS
    }
}

#[inline(always)]
fn clause_header_size(header: u32) -> usize {
    (header >> CLAUSE_SIZE_SHIFT) as usize
}

#[inline(always)]
fn clause_abstraction_from_lits(lits: &[i32]) -> u64 {
    let mut abstraction = 0u64;
    for &lit in lits {
        abstraction |= 1u64 << ((lit.unsigned_abs() - 1) & 31);
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

#[inline(always)]
fn clause_contains_var_in_arena(arena: &[u32], clause_idx: usize, var: usize) -> bool {
    let clause_len = clause_len_in_arena(arena, clause_idx);
    for lit_pos in 0..clause_len {
        if clause_lit_in_arena(arena, clause_idx, lit_pos).unsigned_abs() as usize == var {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy)]
struct ReasonExpansionContext<'a> {
    arena: &'a [u32],
    binary_reasons: &'a [[i32; 2]],
}

#[derive(Clone, Copy)]
struct RedundancyCheckContext<'a> {
    reasons: ReasonExpansionContext<'a>,
    decision_level: &'a [usize],
    reason: &'a [ReasonCode],
    max_depth: u32,
    same_level_only: bool,
}

fn reason_len_in_arena(reasons: ReasonExpansionContext<'_>, reason_ref: ReasonRef) -> usize {
    match reason_ref {
        ReasonRef::None => 0,
        ReasonRef::Clause(clause_idx) => clause_len_in_arena(reasons.arena, clause_idx),
        ReasonRef::Binary(binary_id) => {
            debug_assert!(
                (binary_id.0 as usize) < reasons.binary_reasons.len(),
                "invalid binary reason id {:?}",
                binary_id
            );
            2
        }
    }
}

fn reason_lit_in_arena(
    reasons: ReasonExpansionContext<'_>,
    reason_ref: ReasonRef,
    lit_pos: usize,
) -> i32 {
    match reason_ref {
        ReasonRef::None => panic!("attempted to read literal from empty reason"),
        ReasonRef::Clause(clause_idx) => clause_lit_in_arena(reasons.arena, clause_idx, lit_pos),
        ReasonRef::Binary(binary_id) => {
            let lits = reasons
                .binary_reasons
                .get(binary_id.0 as usize)
                .expect("invalid binary reason id");
            lits[lit_pos]
        }
    }
}

fn rewrite_reason_ref(
    reason_ref: ReasonRef,
    reloc: &[ClauseRef],
    removed_clause_message: &str,
) -> Result<ReasonCode, ReasonCodeError> {
    match reason_ref {
        ReasonRef::None => ReasonCode::from_ref(ReasonRef::None),
        ReasonRef::Clause(clause_idx) => {
            let new_idx = reloc.get(clause_idx).copied().unwrap_or(NO_CLAUSE_REF);
            debug_assert_ne!(new_idx, NO_CLAUSE_REF, "{removed_clause_message}");
            ReasonCode::from_ref(ReasonRef::Clause(new_idx))
        }
        ReasonRef::Binary(binary_id) => ReasonCode::from_ref(ReasonRef::Binary(binary_id)),
    }
}

fn basic_lit_redundant(
    lit: i32,
    reasons: ReasonExpansionContext<'_>,
    decision_level: &[usize],
    reason: &[ReasonCode],
    state: &[u8],
) -> bool {
    let var = lit.unsigned_abs() as usize;
    let reason_ref = reason[var].as_ref_unchecked();
    let (ReasonRef::Clause(_) | ReasonRef::Binary(_)) = reason_ref else {
        return false;
    };

    let clause_len = reason_len_in_arena(reasons, reason_ref);
    for lit_pos in 0..clause_len {
        let q = reason_lit_in_arena(reasons, reason_ref, lit_pos);
        let q_var = q.unsigned_abs() as usize;
        if q_var == var {
            continue;
        }
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
    context: RedundancyCheckContext<'_>,
    state: &mut [u8],
    toclear: &mut Vec<usize>,
    stack: &mut Vec<(usize, i32, u32)>,
) -> bool {
    let mut lit = lit;
    debug_assert!({
        let var = lit.unsigned_abs() as usize;
        state[var] == REDUNDANT_UNDEF || state[var] == REDUNDANT_SOURCE
    });
    debug_assert!(!context.reason[lit.unsigned_abs() as usize].is_none());

    stack.clear();
    let mut reason_ref = context.reason[lit.unsigned_abs() as usize].as_ref_unchecked();
    let mut lit_pos = 0usize;
    let target_level = context.decision_level[lit.unsigned_abs() as usize];
    let mut depth = 0u32;

    loop {
        let clause_len = reason_len_in_arena(context.reasons, reason_ref);
        if lit_pos < clause_len {
            let parent = reason_lit_in_arena(context.reasons, reason_ref, lit_pos);
            let parent_var = parent.unsigned_abs() as usize;
            if parent_var == lit.unsigned_abs() as usize {
                lit_pos += 1;
                continue;
            }
            if state[parent_var] == REDUNDANT_SOURCE || state[parent_var] == REDUNDANT_REMOVABLE {
                lit_pos += 1;
                continue;
            }

            if context.decision_level[parent_var] == 0 {
                lit_pos += 1;
                continue;
            }

            if (context.same_level_only && context.decision_level[parent_var] != target_level)
                || depth >= context.max_depth
                || context.reason[parent_var].is_none()
                || state[parent_var] == REDUNDANT_FAILED
            {
                let lit_var = lit.unsigned_abs() as usize;
                if state[lit_var] == REDUNDANT_UNDEF {
                    state[lit_var] = REDUNDANT_FAILED;
                    toclear.push(lit_var);
                }
                for &(_, stack_lit, _) in stack.iter() {
                    let stack_var = stack_lit.unsigned_abs() as usize;
                    if state[stack_var] == REDUNDANT_UNDEF {
                        state[stack_var] = REDUNDANT_FAILED;
                        toclear.push(stack_var);
                    }
                }
                stack.clear();
                return false;
            }

            stack.push((lit_pos, lit, depth));
            debug_assert!(
                stack.len() <= context.reason.len(),
                "redundancy DFS exceeded variable count while checking literal {lit}"
            );
            lit = parent;
            reason_ref = context.reason[parent_var].as_ref_unchecked();
            lit_pos = match reason_ref {
                ReasonRef::Binary(_) => 0,
                ReasonRef::Clause(_) | ReasonRef::None => 1,
            };
            depth += 1;
            continue;
        }

        let lit_var = lit.unsigned_abs() as usize;
        if state[lit_var] == REDUNDANT_UNDEF {
            state[lit_var] = REDUNDANT_REMOVABLE;
            toclear.push(lit_var);
        }

        if let Some((resume_pos, resume_lit, resume_depth)) = stack.pop() {
            lit = resume_lit;
            reason_ref = context.reason[lit.unsigned_abs() as usize].as_ref_unchecked();
            lit_pos = resume_pos + 1;
            depth = resume_depth;
        } else {
            return true;
        }
    }
}

fn ccmin_mode_from_config(mode: ClauseMinMode) -> u8 {
    match mode {
        ClauseMinMode::Off => CCMIN_NONE,
        ClauseMinMode::Basic => CCMIN_BASIC,
        ClauseMinMode::RecursiveLimited => CCMIN_DEEP,
        ClauseMinMode::InBlockShrink => CCMIN_INBLOCK,
    }
}

impl Solver {
    #[cfg(test)]
    fn new(num_vars: usize, clauses: Vec<Vec<i32>>) -> Self {
        let config = SolverConfig::default();
        Self::new_with_config(num_vars, clauses, &config)
    }

    fn new_with_config(num_vars: usize, clauses: Vec<Vec<i32>>, config: &SolverConfig) -> Self {
        let original_clause_count = clauses.len();
        let branch_mode = config.branch_mode;
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
        let phase_policy = config.phase_policy;
        let search_mode_policy = config.search_mode_policy;
        let focused_stable_mode = search_mode_policy == SearchModePolicy::FocusedStable;
        let mode_use_ticks = config.mode_use_ticks && focused_stable_mode;
        let phase_buffers_enabled =
            phase_policy != PhasePolicy::Legacy || focused_stable_mode || config.rephase;
        let initial_saved_phase = if phase_buffers_enabled {
            UNASSIGNED
        } else {
            default_phase
        };
        let search_mode = if focused_stable_mode {
            SearchMode::Focused
        } else {
            SearchMode::Stable
        };
        let mode_interval = config.mode_init_conflicts.max(1);
        let mode_switch_at_conflicts = if focused_stable_mode {
            mode_interval
        } else {
            u64::MAX
        };
        let ccmin_mode = ccmin_mode_from_config(config.clause_min_mode);
        let vmtf_mode = config.vmtf;
        let vmtf_queue = vmtf_mode
            .enabled()
            .then(|| VmtfQueue::new(num_vars, &branch_order));
        let rephase_conflicts = config.rephase_init_conflicts.max(1);
        let rephase_at_conflicts = if config.rephase {
            rephase_conflicts
        } else {
            u64::MAX
        };

        let total_words: usize = clauses.iter().map(|clause| 1 + clause.len()).sum();
        let arena = Vec::with_capacity(total_words);
        let original_clause_ids = Vec::with_capacity(original_clause_count);
        let initial_clause_mode = config.initial_clause_mode;
        let mut solver = Solver {
            arena,
            original_clause_ids,
            clause_abstraction: Vec::new(),
            inline_original_abstractions: false,
            clauses_sorted_by_var: initial_clause_mode == InitialClauseMode::CanonicalSorted,
            learned_clause_ids: Vec::new(),
            learned_meta: Vec::new(),
            learned_clause_by_id: Vec::new(),
            learned_id_by_clause: Vec::new(),
            watchers: vec![Vec::new(); num_vars.saturating_mul(2)],
            watch_scratch: Vec::new(),
            assignment: vec![UNASSIGNED; num_vars + 1],
            saved_phase: vec![initial_saved_phase; num_vars + 1],
            target_phase: if phase_buffers_enabled {
                vec![UNASSIGNED; num_vars + 1]
            } else {
                Vec::new()
            },
            best_phase: if phase_buffers_enabled {
                vec![UNASSIGNED; num_vars + 1]
            } else {
                Vec::new()
            },
            original_phase: if phase_buffers_enabled {
                vec![default_phase; num_vars + 1]
            } else {
                Vec::new()
            },
            target_assigned: 0,
            best_assigned: 0,
            phase_ticks: 0,
            phase_policy,
            rephase_enabled: config.rephase,
            rephase_index: 0,
            rephase_at_conflicts,
            rephase_conflicts,
            decision_level: vec![0; num_vars + 1],
            reason: vec![NO_REASON; num_vars + 1],
            binary_reason_lits: Vec::new(),
            binary_clauses: Vec::new(),
            binary_implications: BinaryImplications::nested(num_vars.saturating_mul(2)),
            binary_id_by_clause: Vec::new(),
            binary_dedup_seen: vec![0; num_vars.saturating_mul(2)],
            binary_dedup_stamp: 0,
            binary_fast_path: config.binary_fast_path,
            accounting_mode: SearchAccountingMode::NormalSearch,
            temporary_stats: TemporaryAssumptionStats::default(),
            trail: Vec::with_capacity(num_vars),
            root_trail_len: 0,
            trail_limits: Vec::new(),
            propagate_head: 0,
            branch_rank,
            branch_heap: Vec::with_capacity(num_vars),
            branch_pos: vec![BRANCH_NOT_IN_HEAP; num_vars + 1],
            vmtf_queue,
            decision_var: vec![true; num_vars + 1],
            activity: vec![0.0; num_vars + 1],
            activity_inc: 1.0,
            activity_decay: 0.95,
            clause_activity_inc: 1.0,
            clause_activity_decay: 0.999,
            restart_policy: config.restart_policy,
            restart_conflicts: 0,
            restart_unit: 100,
            restart_luby_index: 1,
            restart_conflict_limit: 100,
            restart_pending: false,
            iterating: false,
            restart_fast_lbd: MovingAverage::new(RESTART_FAST_ALPHA),
            restart_slow_lbd: MovingAverage::new(RESTART_SLOW_ALPHA),
            restart_fast_level: MovingAverage::new(RESTART_FAST_ALPHA),
            restart_slow_level: MovingAverage::new(RESTART_SLOW_ALPHA),
            restart_min_conflicts: KISSAT_EMA_RESTART_MIN_CONFLICTS,
            restart_next_check_conflict: 0,
            restart_margin: KISSAT_EMA_RESTART_MARGIN,
            restart_block_margin: config.restart_block_margin,
            restart_reuse_trail: config.restart_reuse_trail,
            restart_conflicts_since_last: 0,
            search_mode,
            search_mode_policy,
            vmtf_mode,
            focused_tier_limits: TierLimits::static_defaults(),
            stable_tier_limits: TierLimits::static_defaults(),
            focused_glue_recent: Vec::new(),
            stable_glue_recent: Vec::new(),
            mode_use_ticks,
            mode_start_conflicts: 0,
            mode_start_ticks: 0,
            mode_start_decisions: 0,
            mode_wall_start: Instant::now(),
            mode_wall_active: false,
            mode_switches: 0,
            mode_switch_at_conflicts,
            mode_switch_at_ticks: u64::MAX,
            mode_init_conflicts: mode_interval,
            mode_interval,
            mode_interval_scale: config.mode_interval_scale,
            reluctant: Reluctant::new(),
            reduce_db_limit: ((original_clause_count as f64) * LEARNTSIZE_FACTOR) as usize,
            reduce_db_min_interval: 0,
            reduce_db_last_conflicts: None,
            learned_lit_budget: LEARNED_LIT_BUDGET_BASE,
            hard_learned_lit_budget: LEARNED_LIT_BUDGET_BASE.saturating_mul(2),
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
            use_simplification: config.simplification,
            trace_preprocess_details: config.trace_preprocess_details,
            use_elim: config.bve,
            full_bsr: config.full_bsr,
            bve_grow: DEFAULT_BVE_GROW,
            bve_clause_limit: DEFAULT_BVE_CLAUSE_LIMIT,
            subsumption_lim: DEFAULT_SUBSUMPTION_LIMIT,
            bwdsub_assigns: 0,
            frozen: vec![false; num_vars + 1],
            eliminated: vec![false; num_vars + 1],
            occurs: vec![Vec::new(); num_vars + 1],
            occurs_dirty: vec![false; num_vars + 1],
            occurs_membership_dirty: vec![false; num_vars + 1],
            n_occ: vec![0; num_vars.saturating_mul(2)],
            elim_clauses: Vec::new(),
            sat_model: None,
            scratch_preprocess_clause: Vec::with_capacity(16),
            scratch_seen: vec![0; num_vars + 1],
            scratch_resolved: vec![0; num_vars + 1],
            scratch_learned: Vec::with_capacity(16),
            otfs_delete_candidates: Vec::new(),
            scratch_conflict_clause: Vec::with_capacity(16),
            scratch_bumped_vars: Vec::with_capacity(16),
            scratch_redundant_state: vec![0; num_vars + 1],
            scratch_analyze_toclear: Vec::with_capacity(16),
            scratch_analyze_stack: Vec::with_capacity(16),
            ccmin_mode,
            otfs_enabled: config.otfs,
            minimize_depth_limit: config.minimize_depth_limit,
            use_resolved_conflict_analysis: config.use_resolved_conflict_analysis,
            use_lbd: config.use_lbd,
            update_reason_lbd: config.update_reason_lbd,
            update_propagation_reason_lbd: config.update_propagation_reason_lbd,
            reduce_policy: config.reduce_policy,
            chrono_backtrack: config.chrono_backtrack,
            chrono_max_delta: config.chrono_max_delta,
            lbd_seen: vec![0; num_vars + 1],
            lbd_stamp: 0,
            last_conflict_lbd: 0,
            sum_lbd: 0,
            num_lbd: 0,
            lbd_hist_1: 0,
            lbd_hist_2: 0,
            lbd_hist_3_5: 0,
            lbd_hist_6_10: 0,
            lbd_hist_gt_10: 0,
            reason_pin_generation: 0,
            reduce_delete_generation: 0,
            reduce_delete_mark: Vec::new(),
            reduce_candidates: Vec::new(),
            gc_pending_reason: GcReason::None,
            track_gc_detail_stats: config.stats_json || config.trace_full,
            hot_stats: config.hot_stats,
            stats: SolverStats::default(),
        };
        solver.sync_tier_limit_stats();
        let reduce_db_limit_overridden = config.reduce_db_init.is_some();
        let reduce_db_interval_overridden = config.reduce_db_interval.is_some();
        if solver.reduce_policy == ReducePolicy::LbdTiered {
            solver.reduce_db_limit = config
                .reduce_db_init
                .unwrap_or(LBD_REDUCE_DB_INIT_CONFLICTS);
            solver.reduce_db_min_interval = config
                .reduce_min_interval
                .unwrap_or(LBD_REDUCE_DB_MIN_INTERVAL_CONFLICTS as usize)
                as u64;
            let interval = config
                .reduce_db_interval
                .unwrap_or(LBD_REDUCE_DB_INTERVAL_CONFLICTS);
            solver.learntsize_adjust_cnt = interval.max(1);
            solver.learntsize_adjust_confl = interval.max(1) as f64;
            solver.reset_reduce_db_after_preprocess =
                config.post_preprocess_reduce_db_reset.unwrap_or(false);
        } else {
            if let Some(limit) = config.reduce_db_init {
                solver.reduce_db_limit = limit;
            }
            solver.reduce_db_min_interval = config.reduce_min_interval.unwrap_or(0) as u64;
            solver.reset_reduce_db_after_preprocess = config
                .post_preprocess_reduce_db_reset
                .unwrap_or(!(reduce_db_limit_overridden || reduce_db_interval_overridden));
            if let Some(interval) = config.reduce_db_interval {
                solver.learntsize_adjust_cnt = interval;
                solver.learntsize_adjust_confl = interval as f64;
            }
        }
        solver.refresh_learned_lit_budgets();
        if let Some(limit) = config.subsumption_limit {
            solver.subsumption_lim = limit;
        }
        match initial_clause_mode {
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
        solver.refresh_learned_lit_budgets();
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
        if deleted {
            self.mark_binary_clause_deleted_for_clause(clause_idx);
        }
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
    fn clause_extra_idx(&self, clause_idx: usize) -> usize {
        clause_idx + 1 + self.clause_len(clause_idx)
    }

    fn ensure_binary_id_map_len(&mut self, clause_idx: ClauseRef) {
        if self.binary_id_by_clause.len() <= clause_idx {
            self.binary_id_by_clause.resize(clause_idx + 1, 0);
        }
    }

    fn try_binary_id_for_clause(&self, clause_idx: ClauseRef) -> Option<BinaryClauseId> {
        let encoded = self.binary_id_by_clause.get(clause_idx).copied()?;
        if encoded == 0 {
            return None;
        }
        Some(BinaryClauseId(encoded - 1))
    }

    #[cfg(test)]
    fn binary_id_for_clause(&self, clause_idx: ClauseRef) -> BinaryClauseId {
        self.try_binary_id_for_clause(clause_idx)
            .expect("binary clause is missing stable BinaryClauseId")
    }

    fn register_binary_clause(&mut self, clause_idx: ClauseRef) -> BinaryClauseId {
        debug_assert!(self.binary_fast_path);
        debug_assert_eq!(self.clause_len(clause_idx), 2);
        if let Some(id) = self.try_binary_id_for_clause(clause_idx) {
            return id;
        }

        let a = self.clause_lit(clause_idx, 0);
        let b = self.clause_lit(clause_idx, 1);
        let id = BinaryClauseId(
            self.binary_clauses
                .len()
                .try_into()
                .expect("too many binary clauses for stable BinaryClauseId"),
        );
        let origin = if self.clause_is_learnt(clause_idx) {
            BinaryOrigin::LearnedConflict
        } else {
            BinaryOrigin::Original
        };
        self.binary_reason_lits.push([a, b]);
        self.binary_clauses.push(BinaryClause {
            clause_ref: clause_idx,
            a,
            b,
            redundant: self.clause_is_learnt(clause_idx),
            deleted: false,
            proof_logged: self.clause_is_learnt(clause_idx),
            origin,
            used_count: 0,
            last_used_conflict: 0,
        });
        self.ensure_binary_id_map_len(clause_idx);
        self.binary_id_by_clause[clause_idx] = id.0.saturating_add(1);
        self.binary_implications.add_edge(
            -a,
            BinaryEdge {
                implied: b,
                clause_id: id,
            },
        );
        self.binary_implications.add_edge(
            -b,
            BinaryEdge {
                implied: a,
                clause_id: id,
            },
        );
        id
    }

    fn mark_binary_clause_deleted_for_clause(&mut self, clause_idx: ClauseRef) {
        let Some(id) = self.try_binary_id_for_clause(clause_idx) else {
            return;
        };
        if let Some(binary) = self.binary_clauses.get_mut(id.0 as usize) {
            binary.deleted = true;
        }
        self.binary_implications.mark_deleted(id);
        if clause_idx < self.binary_id_by_clause.len() {
            self.binary_id_by_clause[clause_idx] = 0;
        }
    }

    fn mark_binary_clause_used(&mut self, binary_id: BinaryClauseId) {
        if let Some(binary) = self.binary_clauses.get_mut(binary_id.0 as usize) {
            binary.used_count = binary.used_count.saturating_add(1);
            binary.last_used_conflict = self.stats.conflicts;
        }
    }

    fn binary_clause_is_deleted(&self, binary_id: BinaryClauseId) -> bool {
        self.binary_clauses
            .get(binary_id.0 as usize)
            .map(|binary| binary.deleted)
            .unwrap_or(true)
    }

    fn reason_ref_for_clause(&self, clause_idx: ClauseRef) -> ReasonRef {
        if self.binary_fast_path && self.clause_len(clause_idx) == 2 {
            if let Some(binary_id) = self.try_binary_id_for_clause(clause_idx) {
                return ReasonRef::Binary(binary_id);
            }
        }
        ReasonRef::Clause(clause_idx)
    }

    #[cfg(test)]
    fn binary_clause_lits_for_test(&self, binary_id: BinaryClauseId) -> [i32; 2] {
        self.binary_reason_lits[binary_id.0 as usize]
    }

    #[cfg(test)]
    fn generated_binary_pair_is_duplicate_for_test(&self, a: i32, b: i32) -> bool {
        let mut needle = [a, b];
        needle.sort_unstable();
        self.binary_clauses.iter().enumerate().any(|(idx, binary)| {
            if binary.deleted {
                return false;
            }
            let mut existing = self.binary_reason_lits[idx];
            existing.sort_unstable();
            existing == needle
        })
    }

    #[inline]
    fn begin_lbd_measurement(&mut self) {
        self.lbd_stamp = self.lbd_stamp.wrapping_add(1);
        if self.lbd_stamp == 0 {
            self.lbd_seen.fill(0);
            self.lbd_stamp = 1;
        }
    }

    #[inline]
    fn count_lbd_level(&mut self, level: usize, count: &mut u32) {
        if level >= self.lbd_seen.len() {
            self.lbd_seen.resize(level + 1, 0);
        }
        if self.lbd_seen[level] != self.lbd_stamp {
            self.lbd_seen[level] = self.lbd_stamp;
            *count += 1;
        }
    }

    #[inline(always)]
    fn finish_lbd_measurement(count: u32) -> u16 {
        count.min(u16::MAX as u32) as u16
    }

    #[inline]
    fn compute_lbd_from_lits(&mut self, lits: &[i32]) -> u16 {
        if lits.is_empty() {
            return 0;
        }

        self.begin_lbd_measurement();

        let mut count = 0u32;
        for &lit in lits {
            let var = lit.unsigned_abs() as usize;
            let level = self.decision_level.get(var).copied().unwrap_or(0);
            self.count_lbd_level(level, &mut count);
        }
        Self::finish_lbd_measurement(count)
    }

    fn record_lbd_measurement(&mut self, lbd: u16) {
        self.sum_lbd += u64::from(lbd);
        self.num_lbd += 1;
        match lbd {
            1 => self.lbd_hist_1 += 1,
            2 => self.lbd_hist_2 += 1,
            3..=5 => self.lbd_hist_3_5 += 1,
            6..=10 => self.lbd_hist_6_10 += 1,
            _ => self.lbd_hist_gt_10 += 1,
        }
        self.stats.record_lbd(u32::from(lbd));
    }

    fn compute_lbd_for_clause(&mut self, clause_idx: ClauseRef) -> u16 {
        let clause_len = self.clause_len(clause_idx);
        if clause_len == 0 {
            return 0;
        }

        self.begin_lbd_measurement();

        let mut count = 0u32;
        for lit_pos in 0..clause_len {
            let lit = self.clause_lit(clause_idx, lit_pos);
            let var = lit.unsigned_abs() as usize;
            let level = self.decision_level.get(var).copied().unwrap_or(0);
            self.count_lbd_level(level, &mut count);
        }
        Self::finish_lbd_measurement(count)
    }

    fn ensure_learned_id_map_len(&mut self, clause_idx: ClauseRef) {
        if self.learned_id_by_clause.len() <= clause_idx {
            self.learned_id_by_clause.resize(clause_idx + 1, 0);
        }
    }

    fn allocate_learned_id(&mut self, clause_idx: ClauseRef) -> LearnedId {
        let id = LearnedId(
            self.learned_meta
                .len()
                .try_into()
                .expect("too many learned clauses for stable LearnedId"),
        );
        self.learned_meta.push(LearnedMeta::default());
        self.learned_clause_by_id.push(clause_idx);
        self.ensure_learned_id_map_len(clause_idx);
        self.learned_id_by_clause[clause_idx] = id.0.saturating_add(1);
        id
    }

    fn try_learned_id_for_clause(&self, clause_idx: ClauseRef) -> Option<LearnedId> {
        let encoded = self.learned_id_by_clause.get(clause_idx).copied()?;
        if encoded == 0 {
            return None;
        }
        Some(LearnedId(encoded - 1))
    }

    fn learned_id_for_clause(&self, clause_idx: ClauseRef) -> LearnedId {
        self.try_learned_id_for_clause(clause_idx)
            .expect("learned clause is missing stable metadata id")
    }

    fn learned_meta(&self, clause_idx: ClauseRef) -> Option<LearnedMeta> {
        let id = self.try_learned_id_for_clause(clause_idx)?;
        self.learned_meta.get(id.0 as usize).copied()
    }

    fn learned_meta_mut_by_id(&mut self, id: LearnedId) -> &mut LearnedMeta {
        &mut self.learned_meta[id.0 as usize]
    }

    fn set_learnt_lbd(&mut self, clause_idx: ClauseRef, lbd: u16) {
        let id = self.learned_id_for_clause(clause_idx);
        self.learned_meta_mut_by_id(id).lbd = lbd;
    }

    fn initialize_learnt_lbd(&mut self, clause_idx: ClauseRef, lbd: u16) {
        self.set_learnt_lbd(clause_idx, lbd);
        self.classify_learnt_clause(clause_idx);
        self.set_learnt_used_recently(clause_idx, MAX_USED_RECENTLY);
        let id = self.learned_id_for_clause(clause_idx);
        self.learned_meta_mut_by_id(id).created_at_conflict = self.stats.conflicts;
    }

    fn learnt_lbd(&self, clause_idx: ClauseRef) -> u16 {
        self.learned_meta(clause_idx)
            .map(|meta| meta.lbd)
            .expect("learned clause is missing LBD metadata")
    }

    fn maybe_improve_lbd(&mut self, clause_idx: ClauseRef, new_lbd: u16) {
        if !self.use_lbd || clause_idx >= self.arena.len() {
            return;
        }
        if !self.clause_is_learnt(clause_idx) || self.clause_is_deleted(clause_idx) {
            return;
        }
        let old_lbd = self.learnt_lbd(clause_idx);
        if new_lbd < old_lbd {
            self.set_learnt_lbd(clause_idx, new_lbd);
            self.classify_learnt_clause(clause_idx);
            self.stats.lbd_improved += 1;
        }
    }

    #[cfg(test)]
    fn learned_clause_lbd(&self, clause_idx: ClauseRef) -> Option<u16> {
        self.learned_meta(clause_idx).map(|meta| meta.lbd)
    }

    fn set_learnt_tier(&mut self, clause_idx: ClauseRef, tier: u8) {
        let id = self.learned_id_for_clause(clause_idx);
        self.learned_meta_mut_by_id(id).tier = tier;
    }

    fn current_tier_limits(&self) -> TierLimits {
        match self.search_mode {
            SearchMode::Focused => self.focused_tier_limits,
            SearchMode::Stable => self.stable_tier_limits,
        }
    }

    fn sync_tier_limit_stats(&mut self) {
        self.stats.focused_tier1_glue_limit = self.focused_tier_limits.tier1_max_glue as u64;
        self.stats.focused_tier2_glue_limit = self.focused_tier_limits.tier2_max_glue as u64;
        self.stats.stable_tier1_glue_limit = self.stable_tier_limits.tier1_max_glue as u64;
        self.stats.stable_tier2_glue_limit = self.stable_tier_limits.tier2_max_glue as u64;
    }

    fn classify_learnt_clause(&mut self, clause_idx: ClauseRef) {
        let lbd = self.learnt_lbd(clause_idx);
        let limits = self.current_tier_limits();
        let tier = if lbd <= limits.tier1_max_glue {
            0
        } else if lbd <= limits.tier2_max_glue {
            1
        } else {
            2
        };
        self.set_learnt_tier(clause_idx, tier);
    }

    fn learnt_used_recently(&self, clause_idx: ClauseRef) -> u8 {
        self.learned_meta(clause_idx)
            .map(|meta| meta.used_recently)
            .expect("learned clause is missing used_recently metadata")
    }

    fn set_learnt_used_recently(&mut self, clause_idx: ClauseRef, value: u8) {
        let id = self.learned_id_for_clause(clause_idx);
        self.learned_meta_mut_by_id(id).used_recently = value;
    }

    fn mark_learned_clause_recent(&mut self, clause_idx: ClauseRef) {
        if !self.use_lbd
            || clause_idx >= self.arena.len()
            || !self.clause_is_learnt(clause_idx)
            || self.clause_is_deleted(clause_idx)
        {
            return;
        }
        self.record_current_mode_glue_use(self.learnt_lbd(clause_idx));
        let recent = self.learnt_used_recently(clause_idx).max(1);
        self.set_learnt_used_recently(clause_idx, recent);
    }

    fn note_clause_used_as_propagation_reason(
        &mut self,
        clause_idx: ClauseRef,
        normal_search_accounting: bool,
    ) {
        if !normal_search_accounting
            || !self.use_lbd
            || !self.update_reason_lbd
            || !self.update_propagation_reason_lbd
            || clause_idx >= self.arena.len()
            || !self.clause_is_learnt(clause_idx)
            || self.clause_is_deleted(clause_idx)
        {
            return;
        }

        let lbd = self.compute_lbd_for_clause(clause_idx);
        self.maybe_improve_lbd(clause_idx, lbd);

        if self.reduce_policy == ReducePolicy::LbdTiered {
            self.mark_learned_clause_recent(clause_idx);
        }
    }

    fn increment_glue_histogram(hist: &mut Vec<u64>, glue: u16) {
        let idx = glue as usize;
        if idx >= hist.len() {
            hist.resize(idx + 1, 0);
        }
        hist[idx] = hist[idx].saturating_add(1);
    }

    fn record_current_mode_glue_use(&mut self, glue: u16) {
        if !self.use_lbd {
            return;
        }
        match self.search_mode {
            SearchMode::Focused => {
                Self::increment_glue_histogram(&mut self.focused_glue_recent, glue);
                Self::increment_glue_histogram(&mut self.stats.focused_glue_used, glue);
            }
            SearchMode::Stable => {
                Self::increment_glue_histogram(&mut self.stable_glue_recent, glue);
                Self::increment_glue_histogram(&mut self.stats.stable_glue_used, glue);
            }
        }
    }

    fn rebuild_reason_pinset(&mut self) -> ReasonPinSet {
        self.reason_pin_generation = self.reason_pin_generation.wrapping_add(1).max(1);
        let mut pinned_clauses = Vec::new();
        let mut pinned_binaries = Vec::new();
        for &reason in &self.reason {
            match reason.as_ref_unchecked() {
                ReasonRef::Clause(clause_idx) => pinned_clauses.push(clause_idx),
                ReasonRef::Binary(binary_id) => pinned_binaries.push(binary_id),
                ReasonRef::None => {}
            }
        }
        pinned_clauses.sort_unstable();
        pinned_clauses.dedup();
        pinned_binaries.sort_unstable_by_key(|binary_id| binary_id.0);
        pinned_binaries.dedup_by_key(|binary_id| binary_id.0);
        ReasonPinSet {
            pinned_clauses,
            pinned_binaries,
            generation: self.reason_pin_generation,
        }
    }

    fn clause_is_reason_pinned(&self, pins: &ReasonPinSet, clause_idx: ClauseRef) -> bool {
        pins.pinned_clauses.binary_search(&clause_idx).is_ok()
    }

    #[allow(dead_code)]
    fn binary_is_reason_pinned(&self, pins: &ReasonPinSet, binary_id: BinaryClauseId) -> bool {
        pins.pinned_binaries
            .binary_search_by_key(&binary_id.0, |pinned| pinned.0)
            .is_ok()
    }

    fn clear_learned_clause_metadata_ref(&mut self, clause_idx: ClauseRef) {
        let Some(id) = self.try_learned_id_for_clause(clause_idx) else {
            return;
        };
        self.learned_clause_by_id[id.0 as usize] = NO_CLAUSE_REF;
        if clause_idx < self.learned_id_by_clause.len() {
            self.learned_id_by_clause[clause_idx] = 0;
        }
    }

    fn remap_learned_metadata_clause_refs(
        &mut self,
        reloc: &[usize],
        new_arena_len: usize,
        count_rewrites: bool,
    ) -> u64 {
        if self.learned_clause_by_id.is_empty() {
            return 0;
        }
        let mut refs_rewritten = 0u64;
        self.learned_id_by_clause.clear();
        self.learned_id_by_clause.resize(new_arena_len, 0);
        for id_idx in 0..self.learned_clause_by_id.len() {
            let old_clause_idx = self.learned_clause_by_id[id_idx];
            if old_clause_idx == NO_CLAUSE_REF || old_clause_idx >= reloc.len() {
                self.learned_clause_by_id[id_idx] = NO_CLAUSE_REF;
                continue;
            }
            let new_clause_idx = reloc[old_clause_idx];
            if new_clause_idx != NO_CLAUSE_REF {
                if count_rewrites && new_clause_idx != old_clause_idx {
                    refs_rewritten += 1;
                }
                self.learned_clause_by_id[id_idx] = new_clause_idx;
                self.learned_id_by_clause[new_clause_idx] =
                    u32::try_from(id_idx + 1).expect("too many learned ids for metadata map");
            } else {
                self.learned_clause_by_id[id_idx] = NO_CLAUSE_REF;
            }
        }
        refs_rewritten
    }

    fn remap_binary_clause_refs(
        &mut self,
        reloc: &[usize],
        new_arena_len: usize,
        count_rewrites: bool,
    ) -> u64 {
        if self.binary_clauses.is_empty() {
            self.binary_id_by_clause.clear();
            return 0;
        }
        let mut refs_rewritten = 0u64;
        self.binary_id_by_clause.clear();
        self.binary_id_by_clause.resize(new_arena_len, 0);
        for id_idx in 0..self.binary_clauses.len() {
            if self.binary_clauses[id_idx].deleted {
                continue;
            }
            let old_clause_idx = self.binary_clauses[id_idx].clause_ref;
            if old_clause_idx >= reloc.len() {
                self.binary_clauses[id_idx].deleted = true;
                continue;
            }
            let new_clause_idx = reloc[old_clause_idx];
            if new_clause_idx == NO_CLAUSE_REF {
                self.binary_clauses[id_idx].deleted = true;
                continue;
            }
            if count_rewrites && new_clause_idx != old_clause_idx {
                refs_rewritten += 1;
            }
            self.binary_clauses[id_idx].clause_ref = new_clause_idx;
            self.binary_id_by_clause[new_clause_idx] =
                u32::try_from(id_idx + 1).expect("too many binary ids for metadata map");
        }
        refs_rewritten
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
        debug_assert!(!self.clause_is_learnt(clause_idx));
        let header = self.arena[clause_idx];
        if clause_header_has_extra(header) {
            let extra_idx = clause_idx + 1 + clause_header_size(header);
            return self.arena[extra_idx] as u64;
        }
        self.clause_abstraction
            .get(clause_idx)
            .copied()
            .unwrap_or(0)
    }

    fn set_original_clause_abstraction(&mut self, clause_idx: usize, abstraction: u64) {
        debug_assert!(!self.clause_is_learnt(clause_idx));
        if self.inline_original_abstractions
            && clause_idx < self.arena.len()
            && self.clause_has_extra(clause_idx)
        {
            let extra_idx = self.clause_extra_idx(clause_idx);
            self.arena[extra_idx] = abstraction as u32;
            return;
        }
        if self.clause_abstraction.len() <= clause_idx {
            self.clause_abstraction.resize(clause_idx + 1, 0);
        }
        self.clause_abstraction[clause_idx] = abstraction;
    }

    fn should_inline_original_abstractions(&self) -> bool {
        self.original_clause_ids.len() >= INLINE_ABSTRACTION_CLAUSE_THRESHOLD
    }

    fn should_lazy_detach_preprocess_originals(&self) -> bool {
        self.inline_original_abstractions
            || self.original_clause_ids.len() < LAZY_DETACH_SMALL_CLAUSE_THRESHOLD
    }

    fn ensure_original_clause_abstractions(&mut self) {
        if !self.should_inline_original_abstractions() {
            self.inline_original_abstractions = false;
            self.clause_abstraction.clear();
            self.clause_abstraction.resize(self.arena.len(), 0);
            let original_clause_ids = self.original_clause_ids.clone();
            for clause_idx in original_clause_ids {
                if clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx) {
                    self.clause_abstraction[clause_idx] =
                        clause_abstraction_from_lits(self.clause_slice(clause_idx));
                }
            }
            return;
        }

        self.inline_original_abstractions = true;
        let needs_inline_abstraction = self.original_clause_ids.iter().any(|&clause_idx| {
            clause_idx < self.arena.len()
                && !self.clause_is_deleted(clause_idx)
                && !self.clause_has_extra(clause_idx)
        });
        if !needs_inline_abstraction {
            self.clause_abstraction.clear();
            return;
        }

        let mut reloc = vec![NO_CLAUSE_REF; self.arena.len()];
        let original_live_word_count: usize = self
            .original_clause_ids
            .iter()
            .filter(|&&clause_idx| {
                clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx)
            })
            .map(|&clause_idx| {
                self.clause_word_len(clause_idx)
                    + if self.clause_has_extra(clause_idx) {
                        0
                    } else {
                        ORIGINAL_ABSTRACTION_WORDS
                    }
            })
            .sum();
        let learned_live_word_count: usize = self
            .learned_clause_ids
            .iter()
            .filter(|&&clause_idx| {
                clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx)
            })
            .map(|&clause_idx| self.clause_word_len(clause_idx))
            .sum();
        let mut new_arena = Vec::with_capacity(original_live_word_count + learned_live_word_count);
        let mut new_original_clause_ids = Vec::with_capacity(self.original_clause_ids.len());
        let mut new_learned_clause_ids = Vec::with_capacity(self.learned_clause_ids.len());

        for &old_clause_idx in &self.original_clause_ids {
            if old_clause_idx >= self.arena.len() || self.clause_is_deleted(old_clause_idx) {
                continue;
            }
            let header = self.clause_header(old_clause_idx);
            let clause_len = clause_header_size(header);
            let new_clause_idx = new_arena.len();
            reloc[old_clause_idx] = new_clause_idx;
            new_arena.push(clause_make_header(
                clause_len,
                false,
                true,
                clause_header_mark(header),
                clause_header_reloced(header),
            ));
            let lits_start = old_clause_idx + 1;
            let lits_end = lits_start + clause_len;
            new_arena.extend_from_slice(&self.arena[lits_start..lits_end]);
            let abstraction = if clause_header_has_extra(header) {
                let extra_idx = lits_end;
                self.arena[extra_idx] as u64
            } else {
                clause_abstraction_from_lits(unsafe {
                    words_as_lits(&self.arena[lits_start..lits_end])
                })
            };
            new_arena.push(abstraction as u32);
            new_original_clause_ids.push(new_clause_idx);
        }

        for &old_clause_idx in &self.learned_clause_ids {
            if old_clause_idx >= self.arena.len() || self.clause_is_deleted(old_clause_idx) {
                continue;
            }
            let new_clause_idx = new_arena.len();
            let old_end = old_clause_idx + self.clause_word_len(old_clause_idx);
            reloc[old_clause_idx] = new_clause_idx;
            new_arena.extend_from_slice(&self.arena[old_clause_idx..old_end]);
            new_learned_clause_ids.push(new_clause_idx);
        }

        for watch_list in &mut self.watchers {
            let mut write = 0usize;
            for read in 0..watch_list.len() {
                let mut watcher = watch_list[read];
                let old_idx = watcher.clause_idx as usize;
                if old_idx >= reloc.len() {
                    continue;
                }
                let new_idx = reloc[old_idx];
                if new_idx == NO_CLAUSE_REF {
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
            let old_idx = watcher.clause_idx as usize;
            if old_idx >= reloc.len() {
                continue;
            }
            let new_idx = reloc[old_idx];
            if new_idx == NO_CLAUSE_REF {
                continue;
            }
            watcher.clause_idx = new_idx as u32;
            self.watch_scratch[watch_scratch_write] = watcher;
            watch_scratch_write += 1;
        }
        self.watch_scratch.truncate(watch_scratch_write);

        for reason_code in &mut self.reason {
            *reason_code = rewrite_reason_ref(
                reason_code.as_ref_unchecked(),
                &reloc,
                "inline abstraction migration removed a live reason clause",
            )
            .expect("reason rewrite failed during inline abstraction migration");
        }

        let mut root_write = 0usize;
        for read in 0..self.root_unit_clauses.len() {
            let old_idx = self.root_unit_clauses[read];
            if old_idx >= reloc.len() {
                continue;
            }
            let new_idx = reloc[old_idx];
            if new_idx == NO_CLAUSE_REF {
                continue;
            }
            self.root_unit_clauses[root_write] = new_idx;
            root_write += 1;
        }
        self.root_unit_clauses.truncate(root_write);

        let new_arena_len = new_arena.len();
        let _ = self.remap_learned_metadata_clause_refs(&reloc, new_arena_len, false);
        let _ = self.remap_binary_clause_refs(&reloc, new_arena_len, false);
        self.arena = new_arena;
        self.original_clause_ids = new_original_clause_ids;
        self.learned_clause_ids = new_learned_clause_ids;
        self.live_learned_clause_count = self.learned_clause_ids.len();
        self.deleted_clause_words = 0;
        self.clause_abstraction.clear();
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

    fn trim_root_false_literals_with_proof(&mut self, clause_idx: usize, proof_log: &mut ProofLog) {
        let clause_len = self.clause_len(clause_idx);
        if clause_len <= 2 {
            return;
        }

        let mut trimmed = Vec::with_capacity(clause_len);
        for read in 0..clause_len {
            let lit = self.clause_lit(clause_idx, read);
            if self.lit_value(lit) == FALSE {
                continue;
            }
            trimmed.push(lit);
        }

        let write = trimmed.len();
        if write == clause_len {
            return;
        }

        proof_log.record_clause(&trimmed);
        if self.clause_has_extra(clause_idx) {
            let old_extra_idx = clause_idx + 1 + clause_len;
            let new_extra_idx = clause_idx + 1 + write;
            let extra_words = clause_header_extra_words(self.clause_header(clause_idx));
            for offset in 0..extra_words {
                self.arena[new_extra_idx + offset] = self.arena[old_extra_idx + offset];
            }
        }

        for (lit_pos, lit) in trimmed.into_iter().enumerate() {
            self.set_clause_lit(clause_idx, lit_pos, lit);
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
    }

    fn delete_clause_for_simplify(&mut self, clause_idx: usize) {
        if self.clause_locked(clause_idx) {
            self.clear_reason_for_locked_clause(clause_idx);
        }

        let clause_len = self.clause_len(clause_idx);
        self.detach_clause(clause_idx);
        if self.clause_is_learnt(clause_idx) {
            self.clear_learned_clause_metadata_ref(clause_idx);
            self.learned_literals -= clause_len;
        } else {
            self.original_literals -= clause_len;
        }
        self.deleted_clause_words += self.clause_word_len(clause_idx);
        self.clause_set_deleted(clause_idx, true);
        self.stats.deleted_clauses += 1;
    }

    fn simplify_clause_list(
        &mut self,
        clause_ids: Vec<usize>,
        proof_log: &mut ProofLog,
    ) -> Vec<usize> {
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
                self.trim_root_false_literals_with_proof(clause_idx, proof_log);
            }
            kept.push(clause_idx);
        }
        kept
    }

    fn total_live_clause_literals(&self) -> usize {
        self.original_literals + self.learned_literals
    }

    fn heap_contains_var(&self, var: usize) -> bool {
        if var == 0 || var >= self.branch_pos.len() {
            return false;
        }
        let idx = self.branch_pos[var];
        idx != BRANCH_NOT_IN_HEAP
            && idx < self.branch_heap.len()
            && self.branch_heap[idx] as usize == var
    }

    fn unassigned_decision_candidate(&self, var: usize) -> bool {
        var > 0
            && var < self.assignment.len()
            && self.decision_var[var]
            && !self.eliminated[var]
            && self.assignment[var] == UNASSIGNED
    }

    fn push_branch_var_if_decision(&mut self, var: usize) {
        if !self.unassigned_decision_candidate(var) {
            return;
        }
        if self.branch_pos[var] != BRANCH_NOT_IN_HEAP {
            debug_assert!(self.heap_contains_var(var));
            return;
        }

        let idx = self.branch_heap.len();
        self.branch_heap.push(var as u32);
        if self.accounting_mode.update_branch_stats() {
            self.stats.decision_heap_inserts += 1;
        }
        self.branch_pos[var] = idx;
        self.branch_heap_sift_up(idx);
    }

    fn push_branch_var(&mut self, var: usize) {
        self.push_branch_var_if_decision(var);
    }

    fn heap_reinsert_unassigned_decision_var(&mut self, var: usize) {
        self.push_branch_var_if_decision(var);
    }

    fn vmtf_note_unassigned_decision_var(&mut self, var: usize) {
        if !self.unassigned_decision_candidate(var) {
            return;
        }
        if let Some(queue) = self.vmtf_queue.as_mut() {
            queue.note_unassigned(var);
        }
    }

    fn heap_remove_assigned_top(&mut self) {
        while let Some(&var_word) = self.branch_heap.first() {
            let var = var_word as usize;
            if self.unassigned_decision_candidate(var) {
                break;
            }
            let _ = self.branch_heap_pop_best();
            if self.accounting_mode.update_branch_stats() {
                self.stats.decision_heap_stale_pops += 1;
            }
        }
    }

    fn rebuild_branch_queue(&mut self) {
        self.branch_heap.clear();
        self.branch_pos.fill(BRANCH_NOT_IN_HEAP);
        for var in 1..self.assignment.len() {
            if self.unassigned_decision_candidate(var) {
                self.branch_pos[var] = self.branch_heap.len();
                self.branch_heap.push(var as u32);
            }
        }
        for idx in (0..(self.branch_heap.len() / 2)).rev() {
            self.branch_heap_sift_down(idx);
        }
    }

    fn refresh_stable_branch_heap_scores(&mut self) {
        self.rebuild_branch_queue();
    }

    fn branch_var_better(&self, lhs: usize, rhs: usize) -> bool {
        self.activity[lhs]
            .total_cmp(&self.activity[rhs])
            .then_with(|| self.branch_rank[rhs].cmp(&self.branch_rank[lhs]))
            .is_gt()
    }

    fn vmtf_branching_active(&self) -> bool {
        if self.vmtf_queue.is_none() {
            return false;
        }

        match self.vmtf_mode {
            VmtfMode::Off => false,
            VmtfMode::FocusedOnly => {
                self.search_mode_policy == SearchModePolicy::FocusedStable
                    && self.search_mode == SearchMode::Focused
            }
            VmtfMode::Single => self.search_mode_policy == SearchModePolicy::Single,
        }
    }

    fn vmtf_stamp_analyzed_var(&mut self, var: usize) {
        if var == 0
            || var >= self.decision_var.len()
            || !self.decision_var[var]
            || self.eliminated[var]
        {
            return;
        }

        let unassigned_candidate = self.unassigned_decision_candidate(var);
        if let Some(queue) = self.vmtf_queue.as_mut() {
            queue.stamp_and_move_to_front(var);
            if unassigned_candidate {
                queue.note_unassigned(var);
            }
        }
    }

    fn pick_vmtf_branch_var(&mut self) -> Option<usize> {
        let assignment = &self.assignment;
        let decision_var = &self.decision_var;
        let eliminated = &self.eliminated;
        let queue = self.vmtf_queue.as_mut()?;
        let mut picked = queue.pick(|var| {
            var > 0
                && var < assignment.len()
                && decision_var[var]
                && !eliminated[var]
                && assignment[var] == UNASSIGNED
        });
        if picked.is_none() {
            queue.reset_search_to_head();
            picked = queue.pick(|var| {
                var > 0
                    && var < assignment.len()
                    && decision_var[var]
                    && !eliminated[var]
                    && assignment[var] == UNASSIGNED
            });
        }

        if let Some(var) = picked {
            self.branch_heap_remove(var);
        }
        picked
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
        if self.accounting_mode.update_branch_stats() {
            self.stats.decision_heap_pops += 1;
        }
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
            2 if self.binary_fast_path => {
                self.register_binary_clause(clause_idx);
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
    fn reason_ref(&self, var: usize) -> ReasonRef {
        self.reason[var].as_ref_unchecked()
    }

    #[inline(always)]
    fn set_reason_ref(&mut self, var: usize, reason: ReasonRef) {
        self.reason[var] = ReasonCode::from_ref(reason).expect("reason encoding failed");
    }

    fn clause_used_as_reason(&self, clause_idx: ClauseRef) -> bool {
        let binary_id = self.try_binary_id_for_clause(clause_idx);
        self.reason
            .iter()
            .any(|&reason| match reason.as_ref_unchecked() {
                ReasonRef::Clause(reason_clause_idx) => reason_clause_idx == clause_idx,
                ReasonRef::Binary(reason_binary_id) => {
                    binary_id.map(|id| id == reason_binary_id).unwrap_or(false)
                }
                ReasonRef::None => false,
            })
    }

    #[allow(dead_code)]
    fn begin_temporary_assumptions(
        &mut self,
        opts: TemporaryAssumptionOptions,
    ) -> TemporaryAssumptionGuard {
        assert_eq!(
            self.current_level(),
            0,
            "temporary assumptions must start at root level"
        );
        let guard = TemporaryAssumptionGuard {
            start_trail: self.trail.len(),
            start_level: self.current_level(),
            start_root_trail_len: self.root_trail_len,
            start_propagate_head: self.propagate_head,
            saved_accounting_mode: self.accounting_mode,
        };
        self.accounting_mode = SearchAccountingMode::from_temporary_options(opts);
        guard
    }

    #[allow(dead_code)]
    fn end_temporary_assumptions(&mut self, guard: TemporaryAssumptionGuard) {
        while self.trail.len() > guard.start_trail {
            let lit = self.trail.pop().expect("temporary trail underflow");
            let var = lit.unsigned_abs() as usize;
            self.assignment[var] = UNASSIGNED;
            self.decision_level[var] = 0;
            self.set_reason_ref(var, ReasonRef::None);
            self.push_branch_var(var);
        }
        self.trail_limits.truncate(guard.start_level);
        self.root_trail_len = guard.start_root_trail_len;
        self.propagate_head = guard.start_propagate_head;
        self.accounting_mode = guard.saved_accounting_mode;
        debug_assert_eq!(self.current_level(), guard.start_level);
        debug_assert_eq!(self.trail.len(), guard.start_trail);
        debug_assert_eq!(self.root_trail_len, guard.start_root_trail_len);
        debug_assert_eq!(self.propagate_head, guard.start_propagate_head);
    }

    #[allow(dead_code)]
    fn with_temporary_assumptions<R>(
        &mut self,
        opts: TemporaryAssumptionOptions,
        f: impl FnOnce(&mut TemporaryAssumptionCtx<'_>) -> R,
    ) -> R {
        let guard = self.begin_temporary_assumptions(opts);
        let result = {
            let mut ctx = TemporaryAssumptionCtx { solver: self };
            f(&mut ctx)
        };
        self.end_temporary_assumptions(guard);
        result
    }

    #[cfg(test)]
    fn reason_clause_for_test(&self, var: usize) -> ClauseRef {
        match self.reason_ref(var) {
            ReasonRef::Clause(clause_idx) => clause_idx,
            other => panic!("expected clause reason for var {var}, got {other:?}"),
        }
    }

    #[cfg(test)]
    fn add_binary_reason_for_test(&mut self, lits: [i32; 2]) -> BinaryClauseId {
        let id = BinaryClauseId(
            self.binary_reason_lits
                .len()
                .try_into()
                .expect("too many test binary reasons"),
        );
        self.binary_reason_lits.push(lits);
        id
    }

    #[cfg(test)]
    fn reason_lits_for_test(&self, reason: ReasonRef) -> Vec<i32> {
        match reason {
            ReasonRef::None => Vec::new(),
            ReasonRef::Clause(clause_idx) => self.clause_slice(clause_idx).to_vec(),
            ReasonRef::Binary(binary_id) => self.binary_reason_lits[binary_id.0 as usize].to_vec(),
        }
    }

    #[cfg(test)]
    fn conflict_lits_for_test(&self, conflict: Conflict) -> Vec<i32> {
        match conflict {
            Conflict::Clause(clause_idx) => self.clause_slice(clause_idx).to_vec(),
            Conflict::Binary(binary_id) => self.binary_reason_lits[binary_id.0 as usize].to_vec(),
            Conflict::RootUnit => Vec::new(),
        }
    }

    fn record_propagation_accounting(&mut self) {
        if self.accounting_mode.is_temporary() {
            self.temporary_stats.propagations += 1;
        } else {
            self.stats.propagations += 1;
        }
    }

    #[inline]
    fn record_search_ticks<const MODE_TICKS: bool>(&mut self, ticks: u64) {
        if MODE_TICKS && !self.accounting_mode.is_temporary() {
            self.stats.search_ticks = self.stats.search_ticks.saturating_add(ticks);
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
            if self.accounting_mode.update_phase() {
                self.saved_phase[var] = target_value;
            }
            self.decision_level[var] = current_level;
            self.set_reason_ref(var, reason);
            self.trail.push(lit);
            if current_level == 0 && !self.accounting_mode.is_temporary() {
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
            if !self.enqueue(lit, ReasonRef::Clause(clause_idx)) {
                return false;
            }
        }
        true
    }

    fn propagate(&mut self) -> Option<Conflict> {
        match (self.hot_stats, self.mode_use_ticks, self.binary_fast_path) {
            (true, true, true) => self.propagate_impl::<true, true, true>(),
            (true, true, false) => self.propagate_impl::<true, true, false>(),
            (true, false, true) => self.propagate_impl::<true, false, true>(),
            (true, false, false) => self.propagate_impl::<true, false, false>(),
            (false, true, true) => self.propagate_impl::<false, true, true>(),
            (false, true, false) => self.propagate_impl::<false, true, false>(),
            (false, false, true) => self.propagate_impl::<false, false, true>(),
            (false, false, false) => self.propagate_impl::<false, false, false>(),
        }
    }

    #[inline(always)]
    fn propagate_binary_implications<
        const HOT_STATS: bool,
        const MODE_TICKS: bool,
        const BINARY_FAST: bool,
    >(
        &mut self,
        lit: i32,
        normal_search_accounting: bool,
    ) -> Option<Conflict> {
        if !BINARY_FAST {
            return None;
        }

        let edge_count = self.binary_implications.len_for(lit);
        for edge_idx in 0..edge_count {
            self.record_search_ticks::<MODE_TICKS>(1);
            let edge = self.binary_implications.edge_for(lit, edge_idx);
            if self.binary_clause_is_deleted(edge.clause_id) {
                if HOT_STATS && normal_search_accounting {
                    self.stats.binary_stale_skips += 1;
                }
                continue;
            }
            match self.lit_value(edge.implied) {
                TRUE => {}
                FALSE => {
                    self.mark_binary_clause_used(edge.clause_id);
                    return Some(Conflict::Binary(edge.clause_id));
                }
                UNASSIGNED => {
                    self.mark_binary_clause_used(edge.clause_id);
                    if !self.enqueue(edge.implied, ReasonRef::Binary(edge.clause_id)) {
                        return Some(Conflict::Binary(edge.clause_id));
                    }
                    if HOT_STATS && normal_search_accounting {
                        self.stats.binary_props += 1;
                    }
                }
                _ => unreachable!(),
            }
        }
        None
    }

    fn propagate_impl<const HOT_STATS: bool, const MODE_TICKS: bool, const BINARY_FAST: bool>(
        &mut self,
    ) -> Option<Conflict> {
        let start_head = self.propagate_head;
        let normal_search_accounting = !self.accounting_mode.is_temporary();
        while self.propagate_head < self.trail.len() {
            let trail_lit = self.trail[self.propagate_head];
            let false_lit = -trail_lit;
            self.propagate_head += 1;
            self.record_propagation_accounting();
            if let Some(conflict) = self
                .propagate_binary_implications::<HOT_STATS, MODE_TICKS, BINARY_FAST>(
                    trail_lit,
                    normal_search_accounting,
                )
            {
                return Some(conflict);
            }
            let watch_idx = self.lit_index(false_lit);
            let mut pending = std::mem::take(&mut self.watchers[watch_idx]);
            let mut read = 0usize;
            let mut write = 0usize;

            while read < pending.len() {
                let watcher = pending[read];
                read += 1;
                self.record_search_ticks::<MODE_TICKS>(1);
                if HOT_STATS && normal_search_accounting {
                    self.stats.watch_scans += 1;
                }
                let clause_idx = watcher.clause_idx as usize;
                if clause_idx >= self.arena.len() {
                    if HOT_STATS && normal_search_accounting {
                        self.stats.watch_stale_skips += 1;
                    }
                    continue;
                }
                if self.lit_value(watcher.blocker) == TRUE {
                    if HOT_STATS && normal_search_accounting {
                        self.stats.watch_blocker_hits += 1;
                    }
                    pending[write] = watcher;
                    write += 1;
                    continue;
                }
                if self.clause_is_deleted(clause_idx) {
                    if HOT_STATS && normal_search_accounting {
                        self.stats.watch_stale_skips += 1;
                    }
                    continue;
                }
                if HOT_STATS && normal_search_accounting {
                    self.stats.watch_clause_loads += 1;
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
                            return Some(Conflict::Clause(clause_idx));
                        }
                        UNASSIGNED => {
                            if !self.enqueue(unit_lit, ReasonRef::Clause(clause_idx)) {
                                pending[write] = watcher;
                                write += 1;
                                while read < pending.len() {
                                    pending[write] = pending[read];
                                    write += 1;
                                    read += 1;
                                }
                                pending.truncate(write);
                                self.watchers[watch_idx] = pending;
                                return Some(Conflict::Clause(clause_idx));
                            }
                            self.note_clause_used_as_propagation_reason(
                                clause_idx,
                                normal_search_accounting,
                            );
                            pending[write] = watcher;
                            write += 1;
                        }
                        _ => unreachable!(),
                    }
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
                    self.record_search_ticks::<MODE_TICKS>(1);
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
                if self.lit_value(first) == FALSE {
                    while read < pending.len() {
                        pending[write] = pending[read];
                        write += 1;
                        read += 1;
                    }
                    pending.truncate(write);
                    self.watchers[watch_idx] = pending;
                    return Some(Conflict::Clause(clause_idx));
                }
                if !self.enqueue(first, ReasonRef::Clause(clause_idx)) {
                    while read < pending.len() {
                        pending[write] = pending[read];
                        write += 1;
                        read += 1;
                    }
                    pending.truncate(write);
                    self.watchers[watch_idx] = pending;
                    return Some(Conflict::Clause(clause_idx));
                }
                self.note_clause_used_as_propagation_reason(clause_idx, normal_search_accounting);
                if clause_len == 2 {
                    if HOT_STATS && normal_search_accounting {
                        self.stats.binary_props += 1;
                    }
                } else {
                    if HOT_STATS && normal_search_accounting {
                        self.stats.long_props += 1;
                    }
                }
            }

            pending.truncate(write);
            self.watchers[watch_idx] = pending;
        }

        if normal_search_accounting {
            self.simplify_props_remaining -= (self.propagate_head - start_head) as i64;
        }

        None
    }

    fn decide(&mut self, lit: i32) {
        self.stats.decisions += 1;
        let level = self.current_level() as u64 + 1;
        match self.search_mode {
            SearchMode::Focused => {
                self.stats.decisions_focused += 1;
                self.stats.record_mode_decision_level(level, true);
            }
            SearchMode::Stable => {
                self.stats.decisions_stable += 1;
                self.stats.record_mode_decision_level(level, false);
            }
        }
        self.stats.avg_decision_level_sum += level;
        self.stats.max_decision_level = self.stats.max_decision_level.max(level);
        self.trail_limits.push(self.trail.len());
        let inserted = self.enqueue(lit, ReasonRef::None);
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
        } else {
            self.push_branch_var_if_decision(var);
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
            if !self.accounting_mode.is_temporary() && self.vmtf_branching_active() {
                self.vmtf_stamp_analyzed_var(var);
            }
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

    fn luby_value_u64(index: u64) -> u64 {
        let index = usize::try_from(index).unwrap_or(usize::MAX);
        Self::luby_value(index.max(1)) as u64
    }

    fn mode_switching_enabled(&self) -> bool {
        self.search_mode_policy == SearchModePolicy::FocusedStable
            && !self.accounting_mode.is_temporary()
    }

    fn begin_search_mode_timing(&mut self) {
        if self.accounting_mode.is_temporary() {
            return;
        }
        self.mode_wall_start = Instant::now();
        self.mode_wall_active = true;
    }

    fn record_current_mode_seconds(&mut self, seconds: f64) {
        if seconds <= 0.0 || !seconds.is_finite() {
            return;
        }
        match self.search_mode {
            SearchMode::Focused => self.stats.seconds_focused += seconds,
            SearchMode::Stable => self.stats.seconds_stable += seconds,
        }
    }

    fn account_current_mode_wall_time(&mut self) {
        if !self.mode_wall_active {
            return;
        }
        let elapsed = self.mode_wall_start.elapsed().as_secs_f64();
        self.record_current_mode_seconds(elapsed);
        self.mode_wall_start = Instant::now();
    }

    fn finish_search_timing(&mut self, search_start: Instant) {
        self.stats.search_sec = search_start.elapsed().as_secs_f64();
        if self.mode_wall_active {
            self.account_current_mode_wall_time();
            self.mode_wall_active = false;
        }
    }

    fn record_search_conflict_mode(&mut self) {
        if self.accounting_mode.is_temporary() {
            return;
        }
        self.stats
            .record_mode_conflict(self.search_mode == SearchMode::Focused);
    }

    fn record_current_mode_lbd(&mut self, lbd: u16) {
        if self.accounting_mode.is_temporary() {
            return;
        }
        self.stats
            .record_mode_lbd(u32::from(lbd), self.search_mode == SearchMode::Focused);
    }

    fn effective_restart_policy(&self) -> RestartPolicy {
        if self.search_mode_policy == SearchModePolicy::FocusedStable {
            match self.search_mode {
                SearchMode::Focused => RestartPolicy::KissatEma,
                SearchMode::Stable => RestartPolicy::Reluctant,
            }
        } else {
            self.restart_policy
        }
    }

    fn effective_phase_policy(&self) -> PhasePolicy {
        if self.search_mode_policy == SearchModePolicy::FocusedStable {
            match self.search_mode {
                SearchMode::Focused => match self.phase_policy {
                    PhasePolicy::Legacy | PhasePolicy::Saved => PhasePolicy::Saved,
                    PhasePolicy::TargetThenSaved | PhasePolicy::BestThenTargetThenSaved => {
                        PhasePolicy::TargetThenSaved
                    }
                },
                SearchMode::Stable => PhasePolicy::BestThenTargetThenSaved,
            }
        } else {
            self.phase_policy
        }
    }

    fn next_mode_interval(&self) -> u64 {
        let scaled = (self.mode_switches.saturating_add(1) as f64).sqrt()
            * self.mode_interval_scale
            * self.mode_init_conflicts.max(1) as f64;
        if !scaled.is_finite() || scaled < 1.0 {
            1
        } else if scaled > u64::MAX as f64 {
            u64::MAX
        } else {
            scaled.round() as u64
        }
    }

    fn nlogpown(count: u64, exponent: u32) -> f64 {
        debug_assert!(count > 0);
        let base = (count as f64 + 9.0).log10();
        let mut factor = 1.0;
        for _ in 0..exponent {
            factor *= base;
        }
        count as f64 * factor
    }

    fn kissat_logn(count: u64) -> u64 {
        if count == 0 {
            0
        } else {
            u64::BITS as u64 - u64::from(count.leading_zeros())
        }
    }

    fn focused_restart_interval(&self) -> u64 {
        KISSAT_EMA_RESTART_MIN_CONFLICTS
            .saturating_add(Self::kissat_logn(self.stats.focused_restarts).saturating_sub(1))
    }

    fn next_focused_mode_interval(&self) -> u64 {
        let focused_count = self.mode_switches.saturating_add(1) / 2;
        let scaled =
            Self::nlogpown(focused_count.max(1), 4) * self.mode_init_conflicts.max(1) as f64;
        if !scaled.is_finite() || scaled < 1.0 {
            1
        } else if scaled > u64::MAX as f64 {
            u64::MAX
        } else {
            scaled as u64
        }
    }

    fn reset_mode_sensitive_averages(&mut self) {
        self.restart_fast_lbd.reset();
        self.restart_slow_lbd.reset();
        self.restart_fast_level.reset();
        self.restart_slow_level.reset();
    }

    fn maybe_switch_search_mode(&mut self) {
        if !self.mode_switching_enabled() {
            return;
        }
        if self.mode_use_ticks {
            match self.search_mode {
                SearchMode::Focused if self.stats.conflicts < self.mode_switch_at_conflicts => {
                    return;
                }
                SearchMode::Stable if self.stats.search_ticks < self.mode_switch_at_ticks => {
                    return;
                }
                _ => {}
            }
        } else if self.stats.conflicts < self.mode_switch_at_conflicts {
            return;
        }

        let delta_ticks = self
            .stats
            .search_ticks
            .saturating_sub(self.mode_start_ticks)
            .max(1);
        self.account_current_mode_wall_time();
        self.search_mode = match self.search_mode {
            SearchMode::Focused => SearchMode::Stable,
            SearchMode::Stable => SearchMode::Focused,
        };
        self.mode_switches = self.mode_switches.saturating_add(1);
        self.stats.mode_switches = self.mode_switches;
        self.mode_start_conflicts = self.stats.conflicts;
        self.mode_start_ticks = self.stats.search_ticks;
        self.mode_start_decisions = self.stats.decisions;
        if self.mode_use_ticks {
            match self.search_mode {
                SearchMode::Focused => {
                    self.mode_interval = self.next_focused_mode_interval();
                    self.mode_switch_at_conflicts =
                        self.stats.conflicts.saturating_add(self.mode_interval);
                    self.mode_switch_at_ticks = u64::MAX;
                }
                SearchMode::Stable => {
                    self.mode_interval = delta_ticks;
                    self.mode_switch_at_conflicts = u64::MAX;
                    self.mode_switch_at_ticks = self.stats.search_ticks.saturating_add(delta_ticks);
                }
            }
        } else {
            self.mode_interval = self.next_mode_interval();
            self.mode_switch_at_conflicts = self.stats.conflicts.saturating_add(self.mode_interval);
        }
        self.restart_pending = false;
        self.restart_conflicts = 0;
        self.restart_conflicts_since_last = 0;
        self.restart_next_check_conflict = self.stats.conflicts.saturating_add(1);
        if self.mode_use_ticks {
            self.reset_mode_sensitive_averages();
        }
        match self.search_mode {
            SearchMode::Focused => {
                if !self.mode_use_ticks {
                    self.restart_fast_lbd.reset();
                    self.restart_slow_lbd.reset();
                }
                if let Some(queue) = self.vmtf_queue.as_mut() {
                    queue.reset_search_to_head();
                }
            }
            SearchMode::Stable => {
                self.refresh_stable_branch_heap_scores();
            }
        }
    }

    fn legacy_luby_restart_due(&self) -> bool {
        self.restart_conflicts >= self.restart_conflict_limit
    }

    fn note_legacy_luby_conflict(&mut self) {
        self.restart_conflicts += 1;
        if !self.legacy_luby_restart_due() {
            return;
        }

        self.restart_conflicts = 0;
        self.restart_pending = true;
        self.restart_luby_index += 1;
        self.stats.luby_restarts += 1;
        self.restart_conflict_limit = self
            .restart_unit
            .saturating_mul(Self::luby_value(self.restart_luby_index));
    }

    fn reluctant_restart_due(&self) -> bool {
        self.current_level() > 0 && self.restart_conflicts_since_last >= self.reluctant.current()
    }

    fn note_reluctant_conflict(&mut self) {
        self.restart_conflicts_since_last = self.restart_conflicts_since_last.saturating_add(1);
        if !self.reluctant_restart_due() {
            return;
        }

        self.restart_pending = true;
        self.restart_conflicts_since_last = 0;
        self.reluctant.advance();
        self.stats.reluctant_restarts += 1;
    }

    fn update_restart_ema(&mut self) {
        let lbd = if self.last_conflict_lbd == 0 {
            1.0
        } else {
            f64::from(self.last_conflict_lbd)
        };
        let level = self.current_level().max(1) as f64;
        self.restart_fast_lbd.update(lbd);
        self.restart_slow_lbd.update(lbd);
        self.restart_fast_level.update(level);
        self.restart_slow_level.update(level);
    }

    fn kissat_ema_restart_candidate_due(&self) -> bool {
        self.current_level() > 0
            && self.restart_conflicts_since_last >= self.restart_min_conflicts
            && self.stats.conflicts >= self.restart_next_check_conflict
            && self.restart_fast_lbd.initialized
            && self.restart_slow_lbd.initialized
            && self.restart_fast_lbd.value > self.restart_slow_lbd.value * self.restart_margin
    }

    fn restart_blocked_by_level_ema(&self) -> bool {
        self.restart_block_margin > 0.0
            && self.restart_fast_level.initialized
            && self.restart_slow_level.initialized
            && self.restart_fast_level.value
                > self.restart_slow_level.value * self.restart_block_margin
    }

    #[cfg(test)]
    fn should_restart(&self) -> bool {
        match self.effective_restart_policy() {
            RestartPolicy::LegacyLuby => self.legacy_luby_restart_due(),
            RestartPolicy::KissatEma => {
                self.kissat_ema_restart_candidate_due() && !self.restart_blocked_by_level_ema()
            }
            RestartPolicy::Reluctant => self.reluctant_restart_due(),
        }
    }

    fn note_kissat_ema_conflict(&mut self) {
        self.update_restart_ema();
        self.restart_conflicts_since_last = self.restart_conflicts_since_last.saturating_add(1);
        if !self.kissat_ema_restart_candidate_due() {
            return;
        }
        if self.restart_blocked_by_level_ema() {
            self.stats.restarts_blocked_by_level =
                self.stats.restarts_blocked_by_level.saturating_add(1);
            return;
        }

        self.restart_pending = true;
        self.restart_conflicts_since_last = 0;
        self.restart_next_check_conflict = self.stats.conflicts.saturating_add(1);
        self.stats.glucose_restarts += 1;
        if self.search_mode_policy == SearchModePolicy::FocusedStable
            && self.search_mode == SearchMode::Focused
        {
            self.stats.focused_restarts = self.stats.focused_restarts.saturating_add(1);
            self.restart_min_conflicts = self.focused_restart_interval();
        }
    }

    fn note_conflict(&mut self) {
        if !self.accounting_mode.update_restart_stats() {
            return;
        }
        if self.restart_pending {
            return;
        }

        match self.effective_restart_policy() {
            RestartPolicy::LegacyLuby => self.note_legacy_luby_conflict(),
            RestartPolicy::KissatEma => self.note_kissat_ema_conflict(),
            RestartPolicy::Reluctant => self.note_reluctant_conflict(),
        }
    }

    fn initial_phase(&self, var: usize) -> u8 {
        self.original_phase.get(var).copied().unwrap_or(FALSE)
    }

    fn saved_or_initial_phase(&mut self, var: usize) -> u8 {
        let saved = self.saved_phase[var];
        if saved != UNASSIGNED {
            self.stats.phase_saved_used += 1;
            saved
        } else {
            self.stats.phase_initial_used += 1;
            self.initial_phase(var)
        }
    }

    #[inline]
    fn pick_branch_phase(&mut self, var: usize) -> bool {
        let phase = match self.effective_phase_policy() {
            PhasePolicy::Legacy => {
                self.stats.phase_legacy_used += 1;
                self.saved_phase[var]
            }
            PhasePolicy::Saved => self.saved_or_initial_phase(var),
            PhasePolicy::TargetThenSaved => {
                let target = self.target_phase[var];
                if target != UNASSIGNED {
                    self.stats.phase_target_used += 1;
                    target
                } else {
                    self.saved_or_initial_phase(var)
                }
            }
            PhasePolicy::BestThenTargetThenSaved => {
                let best = self.best_phase[var];
                if best != UNASSIGNED {
                    self.stats.phase_best_used += 1;
                    best
                } else {
                    let target = self.target_phase[var];
                    if target != UNASSIGNED {
                        self.stats.phase_target_used += 1;
                        target
                    } else {
                        self.saved_or_initial_phase(var)
                    }
                }
            }
        };
        phase == TRUE
    }

    fn pick_branch_lit(&mut self) -> Option<i32> {
        let var = if self.vmtf_branching_active() {
            self.pick_vmtf_branch_var().or_else(|| {
                self.heap_remove_assigned_top();
                self.branch_heap_pop_best()
            })?
        } else {
            self.heap_remove_assigned_top();
            self.branch_heap_pop_best()?
        };
        debug_assert!(self.unassigned_decision_candidate(var));

        Some(if self.pick_branch_phase(var) {
            var as i32
        } else {
            -(var as i32)
        })
    }

    fn reset_target_phase(&mut self) {
        if self.target_phase.is_empty() {
            return;
        }
        self.target_phase.fill(UNASSIGNED);
        self.target_assigned = 0;
    }

    fn preserve_target_phase_across_restart(&self) -> bool {
        self.search_mode_policy == SearchModePolicy::FocusedStable
    }

    fn capture_target_phase(&mut self, assigned: usize) {
        for &lit in &self.trail {
            let var = lit.unsigned_abs() as usize;
            let value = self.assignment[var];
            if value != UNASSIGNED {
                self.target_phase[var] = value;
            }
        }
        self.target_assigned = assigned;
        self.phase_ticks = self.phase_ticks.saturating_add(1);
        self.stats.phase_save_target = self.stats.phase_save_target.saturating_add(1);
    }

    fn capture_best_phase(&mut self, assigned: usize) {
        for &lit in &self.trail {
            let var = lit.unsigned_abs() as usize;
            let value = self.assignment[var];
            if value != UNASSIGNED {
                self.best_phase[var] = value;
            }
        }
        self.best_assigned = assigned;
        self.phase_ticks = self.phase_ticks.saturating_add(1);
        self.stats.phase_save_best = self.stats.phase_save_best.saturating_add(1);
    }

    fn maybe_capture_phase_prefix(&mut self) {
        if self.accounting_mode.is_temporary() {
            return;
        }

        let capture_best = if self.search_mode_policy == SearchModePolicy::FocusedStable {
            true
        } else {
            match self.effective_phase_policy() {
                PhasePolicy::BestThenTargetThenSaved => true,
                PhasePolicy::TargetThenSaved => false,
                PhasePolicy::Legacy | PhasePolicy::Saved => return,
            }
        };
        let assigned = self.trail.len();
        if assigned > self.target_assigned {
            self.capture_target_phase(assigned);
        }
        if capture_best && assigned > self.best_assigned {
            self.capture_best_phase(assigned);
        }
    }

    fn rephase_due_on_stable_restart(&self) -> bool {
        self.rephase_enabled
            && !self.accounting_mode.is_temporary()
            && self.search_mode_policy == SearchModePolicy::FocusedStable
            && self.search_mode == SearchMode::Stable
            && self.stats.conflicts >= self.rephase_at_conflicts
    }

    fn saved_phase_value_for_rephase(&self, var: usize) -> u8 {
        match self.saved_phase[var] {
            TRUE | FALSE => self.saved_phase[var],
            UNASSIGNED => self.initial_phase(var),
            _ => unreachable!("invalid saved phase value"),
        }
    }

    fn invert_phase(value: u8) -> u8 {
        match value {
            TRUE => FALSE,
            FALSE => TRUE,
            UNASSIGNED => UNASSIGNED,
            _ => unreachable!("invalid phase value"),
        }
    }

    fn rephase_to_best(&mut self) {
        if self.saved_phase.is_empty() {
            return;
        }
        for var in 1..self.saved_phase.len() {
            let best = self.best_phase.get(var).copied().unwrap_or(UNASSIGNED);
            self.saved_phase[var] = if best == UNASSIGNED {
                self.initial_phase(var)
            } else {
                best
            };
        }
    }

    fn rephase_to_inverted(&mut self) {
        if self.saved_phase.is_empty() {
            return;
        }
        for var in 1..self.saved_phase.len() {
            let phase = self.saved_phase_value_for_rephase(var);
            self.saved_phase[var] = Self::invert_phase(phase);
        }
    }

    fn rephase_to_original(&mut self) {
        if self.saved_phase.is_empty() {
            return;
        }
        for var in 1..self.saved_phase.len() {
            self.saved_phase[var] = self.initial_phase(var);
        }
    }

    fn apply_rephase(&mut self) {
        match self.rephase_index % 3 {
            0 => self.rephase_to_best(),
            1 => self.rephase_to_inverted(),
            2 => self.rephase_to_original(),
            _ => unreachable!(),
        }
        self.reset_target_phase();
        self.rephase_index = (self.rephase_index + 1) % 3;
        self.rephase_at_conflicts = self
            .stats
            .conflicts
            .saturating_add(self.rephase_conflicts.max(1));
        self.phase_ticks = self.phase_ticks.saturating_add(1);
        self.stats.rephases = self.stats.rephases.saturating_add(1);
    }

    fn maybe_rephase_on_stable_restart(&mut self) {
        if self.rephase_due_on_stable_restart() {
            self.apply_rephase();
        }
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
            self.set_reason_ref(var, ReasonRef::None);
            self.heap_reinsert_unassigned_decision_var(var);
            self.vmtf_note_unassigned_decision_var(var);
        }

        self.trail_limits.truncate(target_level);
        self.propagate_head = self.propagate_head.min(new_trail_len);
    }

    fn learned_clause_asserts_at_level(&self, learned_clause: &[i32], target_level: usize) -> bool {
        if learned_clause.is_empty() {
            return false;
        }

        let mut unassigned_after_backtrack = 0usize;
        for &lit in learned_clause {
            let var = lit.unsigned_abs() as usize;
            if self.decision_level[var] > target_level {
                unassigned_after_backtrack += 1;
                if unassigned_after_backtrack > 1 {
                    return false;
                }
            } else if self.lit_value(lit) != FALSE {
                return false;
            }
        }

        unassigned_after_backtrack == 1
    }

    fn choose_backtrack_level(&mut self, assertion_level: usize, learned_clause: &[i32]) -> usize {
        if !self.chrono_backtrack {
            return assertion_level;
        }

        let current_level = self.current_level();
        if current_level == 0 {
            return 0;
        }
        if assertion_level >= current_level {
            return assertion_level;
        }

        self.stats.chrono_attempts += 1;
        let delta = current_level - assertion_level;
        if delta > self.chrono_max_delta {
            self.stats.chrono_rejected_delta_too_large += 1;
            return assertion_level;
        }

        let chrono_level = current_level - 1;
        if !self.learned_clause_asserts_at_level(learned_clause, chrono_level) {
            self.stats.chrono_rejected_not_asserting += 1;
            return assertion_level;
        }

        if chrono_level > assertion_level {
            self.stats.chrono_used += 1;
            self.stats.chrono_skipped_levels += (chrono_level - assertion_level) as u64;
        }
        chrono_level
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

    fn decision_variable_at_level(&self, level: usize) -> Option<usize> {
        if level == 0 || level > self.current_level() {
            return None;
        }
        let trail_idx = *self.trail_limits.get(level - 1)?;
        let lit = *self.trail.get(trail_idx)?;
        Some(lit.unsigned_abs() as usize)
    }

    fn vmtf_restart_candidate_eligible(&self, var: usize) -> bool {
        var > 0
            && var < self.assignment.len()
            && self.decision_var[var]
            && !self.eliminated[var]
            && self.assignment[var] == UNASSIGNED
    }

    fn peek_vmtf_branch_var(&self) -> Option<usize> {
        let queue = self.vmtf_queue.as_ref()?;
        queue
            .peek(|var| self.vmtf_restart_candidate_eligible(var))
            .or_else(|| queue.peek_from_head(|var| self.vmtf_restart_candidate_eligible(var)))
    }

    fn reuse_stable_trail_level(&mut self, current_level: usize) -> usize {
        self.heap_remove_assigned_top();
        let Some(&limit_word) = self.branch_heap.first() else {
            return current_level;
        };
        let limit_var = limit_word as usize;
        debug_assert!(self.unassigned_decision_candidate(limit_var));
        let limit_score = self.activity[limit_var];

        let mut reused_level = 0usize;
        for level in 1..=current_level {
            let Some(var) = self.decision_variable_at_level(level) else {
                break;
            };
            if self.activity[var].total_cmp(&limit_score).is_le() {
                break;
            }
            reused_level = level;
        }
        reused_level
    }

    fn reuse_focused_trail_level(&self, current_level: usize) -> usize {
        let Some(limit_var) = self.peek_vmtf_branch_var() else {
            return current_level;
        };
        let Some(queue) = self.vmtf_queue.as_ref() else {
            return 0;
        };
        let limit_stamp = queue.stamp(limit_var);

        let mut reused_level = 0usize;
        for level in 1..=current_level {
            let Some(var) = self.decision_variable_at_level(level) else {
                break;
            };
            if queue.stamp(var) <= limit_stamp {
                break;
            }
            reused_level = level;
        }
        reused_level
    }

    fn restart_reuse_trail_level(&mut self) -> usize {
        if !self.restart_reuse_trail {
            return 0;
        }

        let current_level = self.current_level();
        if current_level == 0 {
            return 0;
        }

        if self.vmtf_branching_active() {
            self.reuse_focused_trail_level(current_level)
        } else {
            self.reuse_stable_trail_level(current_level)
        }
    }

    fn perform_restart_if_pending(&mut self) -> bool {
        if !self.restart_pending {
            return false;
        }

        self.restart_pending = false;
        if !self.preserve_target_phase_across_restart() {
            self.reset_target_phase();
        }
        if self.current_level() == 0 {
            return false;
        }

        let backtrack_level = self.restart_reuse_trail_level();
        if self.accounting_mode.update_restart_stats() {
            self.stats.restarts += 1;
            if backtrack_level > 0 {
                self.stats.restarts_reused_trails =
                    self.stats.restarts_reused_trails.saturating_add(1);
                self.stats.restarts_reused_levels = self
                    .stats
                    .restarts_reused_levels
                    .saturating_add(backtrack_level as u64);
            }
        }
        self.maybe_rephase_on_stable_restart();
        self.backtrack(backtrack_level);
        true
    }

    fn take_iterating(&mut self) -> bool {
        let iterating = self.iterating;
        self.iterating = false;
        iterating
    }

    fn maybe_switch_search_mode_after_conflict(&mut self) {
        if !self.iterating {
            self.maybe_switch_search_mode();
        }
    }

    fn run_post_propagation_scheduling(&mut self) -> bool {
        let skip_search_scheduling = self.take_iterating();
        if !skip_search_scheduling && self.mode_use_ticks {
            self.maybe_switch_search_mode();
        }

        !skip_search_scheduling && self.perform_restart_if_pending()
    }

    #[cfg(test)]
    fn simplify(&mut self) -> bool {
        let mut proof_log = ProofLog::disabled();
        self.simplify_with_proof(&mut proof_log)
    }

    fn simplify_with_proof(&mut self, proof_log: &mut ProofLog) -> bool {
        debug_assert_eq!(self.current_level(), 0);
        self.stats.simplifications += 1;

        if self.has_empty_clause || self.propagate().is_some() {
            return false;
        }

        if self.root_trail_len == self.simplify_assigns || self.simplify_props_remaining > 0 {
            return true;
        }

        let learned_clause_ids = std::mem::take(&mut self.learned_clause_ids);
        self.learned_clause_ids = self.simplify_clause_list(learned_clause_ids, proof_log);
        self.live_learned_clause_count = self.learned_clause_ids.len();

        let original_clause_ids = std::mem::take(&mut self.original_clause_ids);
        self.original_clause_ids = self.simplify_clause_list(original_clause_ids, proof_log);

        self.maybe_garbage_collect(GcReason::ArenaFragmentation);
        self.rebuild_branch_queue();
        self.simplify_assigns = self.root_trail_len;
        self.simplify_props_remaining = self.total_live_clause_literals() as i64;
        true
    }

    #[cfg(test)]
    fn add_clause(&mut self, clause: Vec<i32>) -> usize {
        self.add_clause_from_slice(&clause)
    }

    #[allow(dead_code)]
    fn add_clause_from_slice(&mut self, clause: &[i32]) -> usize {
        if !self.use_lbd {
            return self.add_clause_from_slice_plain(clause);
        }
        let lbd = self.compute_lbd_from_lits(clause);
        self.record_lbd_measurement(lbd);
        self.add_clause_from_slice_with_lbd(clause, lbd)
    }

    fn add_analyzed_clause_from_slice(&mut self, clause: &[i32]) -> usize {
        if !self.use_lbd {
            return self.add_clause_from_slice_plain(clause);
        }
        let lbd = self.last_conflict_lbd;
        self.record_current_mode_glue_use(lbd);
        self.add_clause_from_slice_with_lbd(clause, lbd)
    }

    fn add_clause_from_slice_plain(&mut self, clause: &[i32]) -> usize {
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
        self.stats.record_learned_size(clause_len);
        self.attach_clause(clause_idx, false);
        clause_idx
    }

    fn add_clause_from_slice_with_lbd(&mut self, clause: &[i32], lbd: u16) -> usize {
        let clause_idx = self.arena.len();
        let clause_len = clause.len();
        self.arena
            .push(clause_make_header(clause_len, true, true, 0, false));
        self.arena.extend(clause.iter().copied().map(lit_to_word));
        let activity_bits = 0.0f64.to_bits();
        self.arena.push(activity_bits as u32);
        self.arena.push((activity_bits >> 32) as u32);
        self.allocate_learned_id(clause_idx);
        self.learned_clause_ids.push(clause_idx);
        self.live_learned_clause_count += 1;
        self.learned_literals += clause_len;
        self.stats.learned_clauses += 1;
        self.stats.record_learned_size(clause_len);
        self.initialize_learnt_lbd(clause_idx, lbd);
        self.attach_clause(clause_idx, false);
        clause_idx
    }

    fn clause_locked(&self, clause_idx: usize) -> bool {
        if self.clause_is_deleted(clause_idx) || self.clause_len(clause_idx) == 0 {
            return false;
        }
        if let Some(binary_id) = self.try_binary_id_for_clause(clause_idx) {
            let check_len = self.clause_len(clause_idx).min(2);
            for lit_pos in 0..check_len {
                let lit = self.clause_lit(clause_idx, lit_pos);
                let var = lit.unsigned_abs() as usize;
                if self.lit_value(lit) == TRUE
                    && self.reason_ref(var) == ReasonRef::Binary(binary_id)
                {
                    return true;
                }
            }
        }
        let implied_lit = self.clause_lit(clause_idx, 0);
        let var = implied_lit.unsigned_abs() as usize;
        self.lit_value(implied_lit) == TRUE && self.reason_ref(var) == ReasonRef::Clause(clause_idx)
    }

    fn clear_reason_for_locked_clause(&mut self, clause_idx: usize) {
        if self.clause_is_deleted(clause_idx) || self.clause_len(clause_idx) == 0 {
            return;
        }
        let binary_id = self.try_binary_id_for_clause(clause_idx);
        let check_len = self.clause_len(clause_idx).min(2);
        for lit_pos in 0..check_len {
            let lit = self.clause_lit(clause_idx, lit_pos);
            let var = lit.unsigned_abs() as usize;
            let reason = self.reason_ref(var);
            if reason == ReasonRef::Clause(clause_idx)
                || binary_id
                    .map(|id| reason == ReasonRef::Binary(id))
                    .unwrap_or(false)
            {
                self.set_reason_ref(var, ReasonRef::None);
            }
        }
    }

    fn reduce_db_enabled(&self) -> bool {
        self.reduce_db_limit != usize::MAX
    }

    fn reduce_db_min_interval_elapsed(&self) -> bool {
        if self.reduce_db_min_interval == 0 {
            return true;
        }
        self.reduce_db_last_conflicts
            .map(|last| self.stats.conflicts.saturating_sub(last) >= self.reduce_db_min_interval)
            .unwrap_or(true)
    }

    fn should_reduce_db(&self) -> bool {
        if !self.reduce_db_enabled() {
            return false;
        }
        if self.reduce_policy == ReducePolicy::LbdTiered {
            let budget_due = self.stats.conflicts >= self.reduce_db_limit as u64
                || self.learned_literals > self.hard_learned_lit_budget;
            return budget_due && self.reduce_db_min_interval_elapsed();
        }
        let learned_clause_pressure = self
            .live_learned_clause_count
            .saturating_sub(self.trail.len())
            >= self.reduce_db_limit;
        learned_clause_pressure && self.reduce_db_min_interval_elapsed()
    }

    fn refresh_learned_lit_budgets(&mut self) {
        if !self.reduce_db_enabled() {
            self.learned_lit_budget = usize::MAX;
            self.hard_learned_lit_budget = usize::MAX;
            return;
        }
        let reduction_count = self.stats.reduce_db_calls as usize;
        let schedule_budget = LEARNED_LIT_BUDGET_BASE.saturating_add(
            LEARNED_LIT_BUDGET_FACTOR.saturating_mul((reduction_count as f64).sqrt() as usize),
        );
        if self.reduce_policy == ReducePolicy::LbdTiered {
            self.learned_lit_budget = schedule_budget;
            self.hard_learned_lit_budget =
                self.lbd_hard_learned_lit_budget(self.learned_lit_budget);
            return;
        }
        let clause_budget = self.reduce_db_limit.max(1).saturating_mul(8);
        self.learned_lit_budget = schedule_budget.max(clause_budget);
        self.hard_learned_lit_budget = self.learned_lit_budget.saturating_mul(2);
    }

    fn reset_learned_budget_after_preprocess(&mut self) {
        if !self.reduce_db_enabled() {
            return;
        }

        if self.reduce_policy == ReducePolicy::LbdTiered {
            if self.reset_reduce_db_after_preprocess {
                self.reduce_db_limit =
                    (self.stats.conflicts as usize).saturating_add(self.reduce_db_limit);
            }
            self.refresh_learned_lit_budgets();
            return;
        }

        if !self.reset_reduce_db_after_preprocess {
            return;
        }

        self.reduce_db_limit =
            ((self.original_clause_ids.len() as f64) * LEARNTSIZE_FACTOR) as usize;
        self.learntsize_adjust_cnt = LEARNTSIZE_ADJUST_START_CONFL;
        self.learntsize_adjust_confl = LEARNTSIZE_ADJUST_START_CONFL as f64;
        self.refresh_learned_lit_budgets();
    }

    fn lbd_hard_learned_lit_budget(&self, learned_lit_budget: usize) -> usize {
        let soft_guard = learned_lit_budget.saturating_mul(LBD_HARD_LEARNED_LIT_BUDGET_FACTOR);
        let formula_guard = self
            .original_literals
            .saturating_mul(LBD_HARD_LEARNED_LIT_FORMULA_FACTOR);
        soft_guard.max(formula_guard)
    }

    fn schedule_next_lbd_reduce_db(&mut self) {
        debug_assert_eq!(self.reduce_policy, ReducePolicy::LbdTiered);
        if !self.reduce_db_enabled() {
            self.refresh_learned_lit_budgets();
            return;
        }
        let reductions = self.stats.reduce_db_calls.max(1) as f64;
        let base_interval = self.learntsize_adjust_confl.max(1.0);
        let interval = (base_interval * reductions.sqrt()) as usize;
        self.reduce_db_limit = (self.stats.conflicts as usize).saturating_add(interval.max(1));
        self.refresh_learned_lit_budgets();
    }

    fn note_learnt_budget_conflict(&mut self) {
        if !self.reduce_db_enabled()
            || self.reduce_policy == ReducePolicy::LbdTiered
            || self.learntsize_adjust_cnt == 0
        {
            return;
        }
        self.learntsize_adjust_cnt -= 1;
        if self.learntsize_adjust_cnt == 0 {
            self.learntsize_adjust_confl *= LEARNTSIZE_ADJUST_INC;
            self.learntsize_adjust_cnt = self.learntsize_adjust_confl as usize;
            self.reduce_db_limit = ((self.reduce_db_limit as f64) * LEARNTSIZE_INC) as usize;
            self.refresh_learned_lit_budgets();
        }
    }

    #[cfg(test)]
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
            !self.clause_used_as_reason(clause_idx),
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
        self.clear_learned_clause_metadata_ref(clause_idx);
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
            !self.clause_used_as_reason(clause_idx),
            "cannot delete clause {clause_idx} while it is still a live reason"
        );
        self.live_learned_clause_count = self.live_learned_clause_count.saturating_sub(1);
        self.learned_literals -= self.clause_len(clause_idx);
        self.deleted_clause_words += self.clause_word_len(clause_idx);
        self.clear_learned_clause_metadata_ref(clause_idx);
        self.clause_set_deleted(clause_idx, true);
        self.stats.deleted_clauses += 1;
    }

    fn live_clause_word_len_for_gc(&self, clause_idx: ClauseRef, strip_extra: bool) -> usize {
        let word_len = self.clause_word_len(clause_idx);
        if strip_extra && self.clause_has_extra(clause_idx) {
            word_len - ORIGINAL_ABSTRACTION_WORDS
        } else {
            word_len
        }
    }

    fn watcher_liveness_counts(&self) -> (usize, usize) {
        let mut live = 0usize;
        let mut stale = 0usize;
        for watch_list in &self.watchers {
            for watcher in watch_list {
                let clause_idx = watcher.clause_idx as usize;
                if clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx) {
                    live += 1;
                } else {
                    stale += 1;
                }
            }
        }
        (live, stale)
    }

    fn clause_db_measurement(&self) -> ClauseDbMeasurement {
        let strip_original_extra = !self.use_simplification;
        let original_words_live: usize = self
            .original_clause_ids
            .iter()
            .copied()
            .filter(|&clause_idx| {
                clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx)
            })
            .map(|clause_idx| self.live_clause_word_len_for_gc(clause_idx, strip_original_extra))
            .sum();
        let learned_words_live: usize = self
            .learned_clause_ids
            .iter()
            .copied()
            .filter(|&clause_idx| {
                clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx)
            })
            .map(|clause_idx| self.clause_word_len(clause_idx))
            .sum();
        let (watchers_live, watchers_stale) = self.watcher_liveness_counts();
        let arena_words_live = original_words_live.saturating_add(learned_words_live);
        ClauseDbMeasurement {
            arena_words_live,
            arena_words_garbage: self.arena.len().saturating_sub(arena_words_live),
            learned_words_live,
            original_words_live,
            watchers_live,
            watchers_stale,
        }
    }

    fn has_gc_fragmentation_pressure_words(
        arena_words_garbage: usize,
        arena_words_total: usize,
    ) -> bool {
        arena_words_garbage > 0
            && arena_words_garbage.saturating_mul(GC_GARBAGE_RATIO_DENOMINATOR)
                >= arena_words_total.saturating_mul(GC_GARBAGE_RATIO_NUMERATOR)
    }

    fn has_gc_watcher_staleness_pressure_counts(
        watchers_live: usize,
        watchers_stale: usize,
    ) -> bool {
        let watchers_total = watchers_live.saturating_add(watchers_stale);
        watchers_stale >= GC_WATCHER_STALE_MIN
            && watchers_stale.saturating_mul(GC_WATCHER_STALE_RATIO_DENOMINATOR)
                >= watchers_total.saturating_mul(GC_WATCHER_STALE_RATIO_NUMERATOR)
    }

    fn prune_stale_watchers(&mut self) -> usize {
        let arena = &self.arena;
        let mut pruned = 0usize;
        for watch_list in &mut self.watchers {
            let mut write = 0usize;
            for read in 0..watch_list.len() {
                let watcher = watch_list[read];
                let clause_idx = watcher.clause_idx as usize;
                if clause_idx < arena.len()
                    && clause_header_mark(arena[clause_idx]) != CLAUSE_DELETED_MARK
                {
                    watch_list[write] = watcher;
                    write += 1;
                } else {
                    pruned += 1;
                }
            }
            watch_list.truncate(write);
        }
        pruned
    }

    fn remember_gc_pending(&mut self, reason: GcReason) {
        if reason == GcReason::None {
            return;
        }
        self.gc_pending_reason = match (self.gc_pending_reason, reason) {
            (GcReason::EmergencyMemory, _) | (_, GcReason::EmergencyMemory) => {
                GcReason::EmergencyMemory
            }
            (GcReason::WatcherStaleness, _) | (_, GcReason::WatcherStaleness) => {
                GcReason::WatcherStaleness
            }
            (GcReason::LearnedReduction, _) | (_, GcReason::LearnedReduction) => {
                GcReason::LearnedReduction
            }
            _ => GcReason::ArenaFragmentation,
        };
    }

    fn maybe_garbage_collect(&mut self, requested_reason: GcReason) -> bool {
        let requested_reason = if self.gc_pending_reason != GcReason::None {
            self.gc_pending_reason
        } else {
            requested_reason
        };
        let fragmentation =
            Self::has_gc_fragmentation_pressure_words(self.deleted_clause_words, self.arena.len());
        let watcher_staleness_candidate =
            self.stats.watch_stale_skips >= GC_WATCHER_STALE_MIN as u64;
        if self.current_level() > 0 {
            if fragmentation || watcher_staleness_candidate {
                self.prune_stale_watchers();
                self.remember_gc_pending(requested_reason);
            }
            return false;
        }
        let watcher_staleness = if watcher_staleness_candidate {
            let (watchers_live, watchers_stale) = self.watcher_liveness_counts();
            Self::has_gc_watcher_staleness_pressure_counts(watchers_live, watchers_stale)
        } else {
            false
        };
        let reason = if watcher_staleness {
            GcReason::WatcherStaleness
        } else if fragmentation && requested_reason == GcReason::EmergencyMemory {
            GcReason::EmergencyMemory
        } else if fragmentation && requested_reason == GcReason::LearnedReduction {
            GcReason::LearnedReduction
        } else if fragmentation {
            GcReason::ArenaFragmentation
        } else {
            GcReason::None
        };
        if reason == GcReason::None {
            if !fragmentation && !watcher_staleness_candidate {
                self.gc_pending_reason = GcReason::None;
            }
            return false;
        }
        self.gc_pending_reason = GcReason::None;
        self.garbage_collect_with_reason(reason);
        true
    }

    fn garbage_collect(&mut self) {
        self.garbage_collect_with_reason(GcReason::ArenaFragmentation);
    }

    fn garbage_collect_with_reason(&mut self, reason: GcReason) {
        self.stats.garbage_collections += 1;
        self.stats.gc_last_reason = reason;
        let old_arena_words = self.arena.len();
        let track_gc_detail_stats = self.track_gc_detail_stats;
        let mut refs_rewritten = 0u64;
        let pins = self.rebuild_reason_pinset();
        let strip_original_extra = !self.use_simplification;
        let mut reloc = vec![NO_CLAUSE_REF; self.arena.len()];
        let live_clause_count = self.original_clause_ids.len() + self.learned_clause_ids.len();
        let original_live_word_count: usize = self
            .original_clause_ids
            .iter()
            .map(|&clause_idx| {
                let word_len = self.clause_word_len(clause_idx);
                if strip_original_extra && self.clause_has_extra(clause_idx) {
                    word_len - ORIGINAL_ABSTRACTION_WORDS
                } else {
                    word_len
                }
            })
            .sum();
        let learned_live_word_count: usize = self
            .learned_clause_ids
            .iter()
            .map(|&clause_idx| self.clause_word_len(clause_idx))
            .sum();
        let live_word_count = original_live_word_count + learned_live_word_count;

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
                           reloc: &mut [usize],
                           strip_extra: bool| {
            let new_clause_idx = new_arena.len();
            reloc[old_clause_idx] = new_clause_idx;
            let header = arena[old_clause_idx];
            let clause_len = clause_header_size(header);
            let has_extra = clause_header_has_extra(header) && !strip_extra;
            new_arena.push(clause_make_header(
                clause_len,
                clause_header_learnt(header),
                has_extra,
                clause_header_mark(header),
                clause_header_reloced(header),
            ));
            let lits_start = old_clause_idx + 1;
            let lits_end = lits_start + clause_len;
            new_arena.extend_from_slice(&arena[lits_start..lits_end]);
            if has_extra {
                let extra_end = lits_end + clause_header_extra_words(header);
                new_arena.extend_from_slice(&arena[lits_end..extra_end]);
            }
            new_clause_idx
        };

        for &old_clause_idx in &self.original_clause_ids {
            debug_assert!(
                !self.clause_is_deleted(old_clause_idx),
                "original clauses must stay live across garbage collection"
            );
            let new_clause_idx = copy_clause(
                old_clause_idx,
                &self.arena,
                &mut new_arena,
                &mut reloc,
                strip_original_extra,
            );
            new_original_clause_ids.push(new_clause_idx);
        }
        for &old_clause_idx in &self.learned_clause_ids {
            debug_assert!(
                !self.clause_is_deleted(old_clause_idx),
                "live learned clauses must stay live across garbage collection"
            );
            let new_clause_idx = copy_clause(
                old_clause_idx,
                &self.arena,
                &mut new_arena,
                &mut reloc,
                false,
            );
            new_learned_clause_ids.push(new_clause_idx);
        }

        for watch_list in &mut self.watchers {
            let mut write = 0usize;
            for read in 0..watch_list.len() {
                let mut watcher = watch_list[read];
                let old_idx = watcher.clause_idx as usize;
                if old_idx >= reloc.len() {
                    continue;
                }
                let new_idx = reloc[old_idx];
                if new_idx == NO_CLAUSE_REF {
                    continue;
                }
                if track_gc_detail_stats && new_idx != old_idx {
                    refs_rewritten += 1;
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
            let old_idx = watcher.clause_idx as usize;
            if old_idx >= reloc.len() {
                continue;
            }
            let new_idx = reloc[old_idx];
            if new_idx == NO_CLAUSE_REF {
                continue;
            }
            if track_gc_detail_stats && new_idx != old_idx {
                refs_rewritten += 1;
            }
            watcher.clause_idx = new_idx as u32;
            self.watch_scratch[watch_scratch_write] = watcher;
            watch_scratch_write += 1;
        }
        self.watch_scratch.truncate(watch_scratch_write);

        for reason_code in &mut self.reason {
            let old_reason = reason_code.as_ref_unchecked();
            let new_code = rewrite_reason_ref(
                old_reason,
                &reloc,
                "garbage collection removed a clause that is still a live reason",
            )
            .expect("reason rewrite failed during garbage collection");
            if let (ReasonRef::Clause(old_idx), ReasonRef::Clause(new_idx)) =
                (old_reason, new_code.as_ref_unchecked())
            {
                if track_gc_detail_stats && old_idx != new_idx {
                    refs_rewritten += 1;
                }
            }
            *reason_code = new_code;
        }
        for &pinned_clause in &pins.pinned_clauses {
            debug_assert!(
                pinned_clause < reloc.len() && reloc[pinned_clause] != NO_CLAUSE_REF,
                "garbage collection removed a reason-pinned clause"
            );
        }

        let mut root_write = 0usize;
        for read in 0..self.root_unit_clauses.len() {
            let old_idx = self.root_unit_clauses[read];
            if old_idx >= reloc.len() {
                continue;
            }
            let new_idx = reloc[old_idx];
            if new_idx == NO_CLAUSE_REF {
                continue;
            }
            if track_gc_detail_stats && new_idx != old_idx {
                refs_rewritten += 1;
            }
            self.root_unit_clauses[root_write] = new_idx;
            root_write += 1;
        }
        self.root_unit_clauses.truncate(root_write);

        let new_arena_len = new_arena.len();
        refs_rewritten += self.remap_learned_metadata_clause_refs(
            &reloc,
            new_arena_len,
            self.track_gc_detail_stats,
        );
        refs_rewritten +=
            self.remap_binary_clause_refs(&reloc, new_arena_len, self.track_gc_detail_stats);
        self.arena = new_arena;
        self.original_clause_ids = new_original_clause_ids;
        self.learned_clause_ids = new_learned_clause_ids;
        if !self.clause_abstraction.is_empty() {
            self.clause_abstraction.clear();
            if self.use_simplification {
                self.clause_abstraction.resize(self.arena.len(), 0);
                let original_clause_ids = self.original_clause_ids.clone();
                for clause_idx in original_clause_ids {
                    self.clause_abstraction[clause_idx] =
                        clause_abstraction_from_lits(self.clause_slice(clause_idx));
                }
            }
        }
        self.live_learned_clause_count = self.learned_clause_ids.len();
        self.deleted_clause_words = 0;
        self.gc_pending_reason = GcReason::None;
        self.stats.gc_words_reclaimed += old_arena_words.saturating_sub(new_arena_len) as u64;
        self.stats.gc_refs_rewritten += refs_rewritten;
    }

    #[cfg(test)]
    fn reduce_db(&mut self) {
        let mut proof_log = ProofLog::disabled();
        self.reduce_db_with_proof(&mut proof_log);
    }

    fn reduce_db_with_proof(&mut self, proof_log: &mut ProofLog) {
        self.stats.reduce_db_calls += 1;
        self.reduce_db_last_conflicts = Some(self.stats.conflicts);
        match self.reduce_policy {
            ReducePolicy::LbdTiered => {
                self.reduce_db_lbd_tiered(proof_log);
                self.schedule_next_lbd_reduce_db();
            }
            ReducePolicy::LegacyActivity | ReducePolicy::Activity => {
                self.reduce_db_legacy_activity(proof_log);
                self.refresh_learned_lit_budgets();
            }
        }
    }

    fn reduce_db_legacy_activity(&mut self, proof_log: &mut ProofLog) {
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
                proof_log.record_deletion(self.clause_slice(clause_idx));
                self.detach_clause(clause_idx);
                self.mark_clause_deleted_already_unlinked(clause_idx);
            } else {
                self.learned_clause_ids[write] = clause_idx;
                write += 1;
            }
        }
        self.learned_clause_ids.truncate(write);
        self.live_learned_clause_count = self.learned_clause_ids.len();

        self.maybe_garbage_collect(GcReason::LearnedReduction);
    }

    fn reduce_candidate_activity_rank(&self, clause_idx: ClauseRef) -> u32 {
        (self.clause_activity(clause_idx).to_bits() >> 32) as u32
    }

    fn glue_limit_for_target(hist: &[u64], target: u64, fallback: u16) -> u16 {
        if target == 0 {
            return fallback;
        }
        let mut cumulative = 0u64;
        let mut last_seen = fallback;
        for (glue, &count) in hist.iter().enumerate() {
            if count == 0 {
                continue;
            }
            last_seen = (glue.min(u16::MAX as usize)) as u16;
            cumulative = cumulative.saturating_add(count);
            if cumulative >= target {
                return last_seen;
            }
        }
        last_seen
    }

    fn relative_target(total: u64, numerator: u64, denominator: u64) -> u64 {
        debug_assert!(denominator > 0);
        let scaled = (total as u128).saturating_mul(numerator as u128);
        scaled
            .saturating_add(denominator.saturating_sub(1) as u128)
            .saturating_div(denominator as u128) as u64
    }

    fn compute_tier_limits_from_histogram(hist: &[u64], fallback: TierLimits) -> TierLimits {
        let total = hist.iter().copied().fold(0u64, u64::saturating_add);
        if total == 0 {
            return fallback;
        }
        let tier1_target =
            Self::relative_target(total, TIER1_RELATIVE_NUMERATOR, TIER1_RELATIVE_DENOMINATOR);
        let tier2_target =
            Self::relative_target(total, TIER2_RELATIVE_NUMERATOR, TIER2_RELATIVE_DENOMINATOR);
        let tier1 = Self::glue_limit_for_target(hist, tier1_target, fallback.tier1_max_glue)
            .max(TIER1_MAX_GLUE);
        let tier2 = Self::glue_limit_for_target(hist, tier2_target, fallback.tier2_max_glue)
            .max(TIER2_MAX_GLUE)
            .max(tier1);
        TierLimits {
            tier1_max_glue: tier1,
            tier2_max_glue: tier2,
        }
    }

    fn retier_current_mode_from_glue_histogram(&mut self) {
        match self.search_mode {
            SearchMode::Focused => {
                let limits = Self::compute_tier_limits_from_histogram(
                    &self.focused_glue_recent,
                    self.focused_tier_limits,
                );
                self.focused_tier_limits = limits;
                self.focused_glue_recent.clear();
            }
            SearchMode::Stable => {
                let limits = Self::compute_tier_limits_from_histogram(
                    &self.stable_glue_recent,
                    self.stable_tier_limits,
                );
                self.stable_tier_limits = limits;
                self.stable_glue_recent.clear();
            }
        }
        self.sync_tier_limit_stats();
        self.reclassify_live_learned_clause_tiers();
    }

    fn reclassify_live_learned_clause_tiers(&mut self) {
        for idx in 0..self.learned_clause_ids.len() {
            let clause_idx = self.learned_clause_ids[idx];
            if clause_idx < self.arena.len()
                && !self.clause_is_deleted(clause_idx)
                && self.clause_is_learnt(clause_idx)
                && self.learned_meta(clause_idx).is_some()
            {
                self.classify_learnt_clause(clause_idx);
            }
        }
    }

    fn is_old_enough_for_emergency_demote(&self, meta: LearnedMeta) -> bool {
        self.stats
            .conflicts
            .saturating_sub(meta.created_at_conflict)
            >= EMERGENCY_TIER1_MIN_AGE_CONFLICTS
    }

    fn reduce_candidate(
        &self,
        clause_idx: ClauseRef,
        pins: &ReasonPinSet,
        emergency: bool,
    ) -> Option<ReduceCand> {
        if clause_idx >= self.arena.len()
            || self.clause_is_deleted(clause_idx)
            || !self.clause_is_learnt(clause_idx)
            || self.clause_len(clause_idx) <= 2
            || self.clause_locked(clause_idx)
            || self.clause_is_reason_pinned(pins, clause_idx)
        {
            return None;
        }

        let meta = self.learned_meta(clause_idx)?;
        if !meta.removable {
            return None;
        }

        let over_budget = self.learned_literals > self.learned_lit_budget;
        match meta.tier {
            0 => {
                if !emergency
                    || meta.used_recently > 0
                    || !self.is_old_enough_for_emergency_demote(meta)
                {
                    return None;
                }
            }
            1 => {
                if !over_budget || meta.used_recently > 0 {
                    return None;
                }
            }
            _ => {
                if !over_budget || meta.used_recently > 0 {
                    return None;
                }
            }
        }

        Some(ReduceCand {
            clause_idx,
            lbd: meta.lbd,
            size: self.clause_len(clause_idx),
            used_recently: meta.used_recently,
            activity_rank: self.reduce_candidate_activity_rank(clause_idx),
        })
    }

    fn begin_reduce_delete_marking(&mut self) {
        self.reduce_delete_generation = self.reduce_delete_generation.wrapping_add(1);
        if self.reduce_delete_generation == 0 {
            self.reduce_delete_mark.fill(0);
            self.reduce_delete_generation = 1;
        }
        if self.reduce_delete_mark.len() < self.arena.len() {
            self.reduce_delete_mark.resize(self.arena.len(), 0);
        }
    }

    fn mark_reduce_delete_candidate(&mut self, clause_idx: ClauseRef) {
        debug_assert!(clause_idx < self.reduce_delete_mark.len());
        self.reduce_delete_mark[clause_idx] = self.reduce_delete_generation;
    }

    fn reduce_delete_candidate_marked(&self, clause_idx: ClauseRef) -> bool {
        clause_idx < self.reduce_delete_mark.len()
            && self.reduce_delete_mark[clause_idx] == self.reduce_delete_generation
    }

    fn reduce_db_lbd_tiered(&mut self, proof_log: &mut ProofLog) {
        self.retier_current_mode_from_glue_histogram();
        let pins = self.rebuild_reason_pinset();
        debug_assert!(pins.generation > 0);
        let emergency = self.learned_literals > self.hard_learned_lit_budget;
        self.reduce_candidates.clear();
        for idx in 0..self.learned_clause_ids.len() {
            let clause_idx = self.learned_clause_ids[idx];
            if let Some(candidate) = self.reduce_candidate(clause_idx, &pins, emergency) {
                self.reduce_candidates.push(candidate);
            }
        }

        self.reduce_candidates.sort_unstable_by(|lhs, rhs| {
            rhs.lbd
                .cmp(&lhs.lbd)
                .then_with(|| rhs.size.cmp(&lhs.size))
                .then_with(|| lhs.used_recently.cmp(&rhs.used_recently))
                .then_with(|| lhs.activity_rank.cmp(&rhs.activity_rank))
                .then_with(|| lhs.clause_idx.cmp(&rhs.clause_idx))
        });

        let mut projected_lits = self.learned_literals;
        self.begin_reduce_delete_marking();
        for idx in 0..self.reduce_candidates.len() {
            let cand = self.reduce_candidates[idx];
            if projected_lits <= self.learned_lit_budget {
                break;
            }
            self.mark_reduce_delete_candidate(cand.clause_idx);
            projected_lits = projected_lits.saturating_sub(cand.size);
        }
        self.reduce_candidates.clear();

        let mut write = 0usize;
        let learned_len = self.learned_clause_ids.len();
        for read in 0..learned_len {
            let clause_idx = self.learned_clause_ids[read];
            if self.reduce_delete_candidate_marked(clause_idx) {
                proof_log.record_deletion(self.clause_slice(clause_idx));
                self.detach_clause(clause_idx);
                self.mark_clause_deleted_already_unlinked(clause_idx);
                self.stats.learned_collected += 1;
            } else {
                self.record_lbd_tier_kept(clause_idx);
                self.age_learned_clause_on_reduce(clause_idx);
                self.learned_clause_ids[write] = clause_idx;
                write += 1;
            }
        }
        self.learned_clause_ids.truncate(write);
        self.live_learned_clause_count = self.learned_clause_ids.len();

        let gc_reason = if emergency {
            GcReason::EmergencyMemory
        } else {
            GcReason::LearnedReduction
        };
        self.maybe_garbage_collect(gc_reason);
    }

    fn record_lbd_tier_kept(&mut self, clause_idx: ClauseRef) {
        if clause_idx >= self.arena.len()
            || self.clause_is_deleted(clause_idx)
            || !self.clause_is_learnt(clause_idx)
        {
            return;
        }
        let Some(meta) = self.learned_meta(clause_idx) else {
            return;
        };
        match meta.tier {
            0 => self.stats.learned_kept_tier1 += 1,
            1 => self.stats.learned_kept_tier2 += 1,
            _ => self.stats.learned_kept_tier3 += 1,
        }
    }

    fn age_learned_clause_on_reduce(&mut self, clause_idx: ClauseRef) {
        if clause_idx >= self.arena.len()
            || self.clause_is_deleted(clause_idx)
            || !self.clause_is_learnt(clause_idx)
            || self.clause_len(clause_idx) <= 2
        {
            return;
        }
        let Some(meta) = self.learned_meta(clause_idx) else {
            return;
        };
        if meta.removable && meta.used_recently > 0 {
            self.set_learnt_used_recently(clause_idx, meta.used_recently - 1);
        }
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
        let reason_context = ReasonExpansionContext {
            arena,
            binary_reasons: &self.binary_reason_lits,
        };
        let redundancy_context = RedundancyCheckContext {
            reasons: reason_context,
            decision_level,
            reason,
            max_depth: self.minimize_depth_limit,
            same_level_only: self.ccmin_mode == CCMIN_INBLOCK,
        };
        let mut write = 1usize;
        for read in 1..learned_clause.len() {
            let lit = learned_clause[read];
            let var = lit.unsigned_abs() as usize;
            let keep = if reason[var].is_none() {
                true
            } else if self.ccmin_mode == CCMIN_BASIC {
                !basic_lit_redundant(lit, reason_context, decision_level, reason, state)
            } else {
                !lit_redundant(lit, redundancy_context, state, toclear, stack)
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

    fn learned_clause_subsumes_watched_clause(
        &self,
        learned_clause: &[i32],
        candidate_idx: ClauseRef,
    ) -> bool {
        if candidate_idx >= self.arena.len() || self.clause_is_deleted(candidate_idx) {
            return false;
        }
        let candidate_len = self.clause_len(candidate_idx);
        if candidate_len < learned_clause.len()
            || candidate_len <= 1
            || candidate_len > learned_clause.len().saturating_add(OTFS_MAX_EXTRA_LITS)
        {
            return false;
        }

        learned_clause.iter().copied().all(|needle| {
            (0..candidate_len).any(|lit_pos| self.clause_lit(candidate_idx, lit_pos) == needle)
        })
    }

    fn delete_clause_subsumed_by_otfs(
        &mut self,
        clause_idx: ClauseRef,
        proof_log: &mut ProofLog,
    ) -> bool {
        if clause_idx >= self.arena.len()
            || self.clause_is_deleted(clause_idx)
            || !self.clause_is_learnt(clause_idx)
            || self.clause_used_as_reason(clause_idx)
        {
            return false;
        }

        proof_log.record_deletion(self.clause_slice(clause_idx));
        self.detach_clause(clause_idx);
        self.mark_clause_deleted(clause_idx);
        self.stats.otfs_subsumed_learned += 1;
        self.stats.otfs_subsumed_clauses += 1;
        true
    }

    fn otfs_subsume_watched_clauses(
        &mut self,
        learned_clause: &[i32],
        protected_clause_idx: Option<ClauseRef>,
        proof_log: &mut ProofLog,
    ) -> usize {
        if !self.otfs_enabled
            || self.ccmin_mode == CCMIN_NONE
            || learned_clause.len() <= 1
            || learned_clause.len() > OTFS_MAX_LEARNED_LEN
        {
            return 0;
        }

        let mut candidates = std::mem::take(&mut self.otfs_delete_candidates);
        candidates.clear();
        for &lit in learned_clause {
            let watch_idx = self.lit_index(lit);
            let watch_len = self.watchers[watch_idx].len();
            for watcher_idx in 0..watch_len {
                self.stats.otfs_watch_scans += 1;
                let clause_idx = self.watchers[watch_idx][watcher_idx].clause_idx as ClauseRef;
                if Some(clause_idx) == protected_clause_idx
                    || clause_idx >= self.arena.len()
                    || self.clause_is_deleted(clause_idx)
                    || !self.clause_is_learnt(clause_idx)
                {
                    continue;
                }
                let candidate_len = self.clause_len(clause_idx);
                if candidate_len < learned_clause.len()
                    || candidate_len > learned_clause.len().saturating_add(OTFS_MAX_EXTRA_LITS)
                {
                    continue;
                }
                self.stats.otfs_candidate_checks += 1;
                if self.learned_clause_subsumes_watched_clause(learned_clause, clause_idx) {
                    candidates.push(clause_idx);
                }
            }
        }

        candidates.sort_unstable();
        candidates.dedup();
        let mut deleted = 0usize;
        for &clause_idx in &candidates {
            if Some(clause_idx) == protected_clause_idx {
                continue;
            }
            if self.learned_clause_subsumes_watched_clause(learned_clause, clause_idx)
                && self.delete_clause_subsumed_by_otfs(clause_idx, proof_log)
            {
                deleted += 1;
            }
        }
        candidates.clear();
        self.otfs_delete_candidates = candidates;
        deleted
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

    fn mark_binary_literals_for_analysis(
        &mut self,
        binary_id: BinaryClauseId,
        skip_var: Option<usize>,
        current_level: usize,
        current_level_count: &mut usize,
    ) {
        self.mark_binary_clause_used(binary_id);
        let lits = *self
            .binary_reason_lits
            .get(binary_id.0 as usize)
            .expect("invalid binary reason id");
        for &lit in &lits {
            let var = lit.unsigned_abs() as usize;
            if skip_var == Some(var) {
                continue;
            }
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

    fn mark_reason_literals_for_analysis<const LBD_META: bool>(
        &mut self,
        reason_ref: ReasonRef,
        resolved_var: usize,
        start_lit_pos: usize,
        current_level: usize,
        current_level_count: &mut usize,
    ) {
        match reason_ref {
            ReasonRef::None => {}
            ReasonRef::Clause(clause_idx) => {
                if LBD_META && self.reduce_policy == ReducePolicy::LbdTiered {
                    self.mark_learned_clause_recent(clause_idx);
                }
                if LBD_META
                    && self.update_reason_lbd
                    && clause_idx < self.arena.len()
                    && self.clause_is_learnt(clause_idx)
                    && !self.clause_is_deleted(clause_idx)
                {
                    let lbd = self.compute_lbd_for_clause(clause_idx);
                    self.maybe_improve_lbd(clause_idx, lbd);
                }
                self.mark_clause_literals_for_analysis(
                    clause_idx,
                    start_lit_pos,
                    current_level,
                    current_level_count,
                );
            }
            ReasonRef::Binary(binary_id) => self.mark_binary_literals_for_analysis(
                binary_id,
                Some(resolved_var),
                current_level,
                current_level_count,
            ),
        }
    }

    fn mark_conflict_literals_for_analysis(
        &mut self,
        conflict: Conflict,
        current_level: usize,
        current_level_count: &mut usize,
    ) {
        match conflict {
            Conflict::Clause(clause_idx) => self.mark_clause_literals_for_analysis(
                clause_idx,
                0,
                current_level,
                current_level_count,
            ),
            Conflict::Binary(binary_id) => self.mark_binary_literals_for_analysis(
                binary_id,
                None,
                current_level,
                current_level_count,
            ),
            Conflict::RootUnit => {}
        }
    }

    fn conflict_max_decision_level(&self, conflict: Conflict) -> usize {
        match conflict {
            Conflict::Clause(clause_idx) => {
                let clause_len = self.clause_len(clause_idx);
                let mut max_level = 0usize;
                for lit_pos in 0..clause_len {
                    let var = self.clause_lit(clause_idx, lit_pos).unsigned_abs() as usize;
                    max_level = max_level.max(self.decision_level[var]);
                }
                max_level
            }
            Conflict::Binary(binary_id) => self
                .binary_reason_lits
                .get(binary_id.0 as usize)
                .map(|lits| {
                    lits.iter()
                        .map(|lit| self.decision_level[lit.unsigned_abs() as usize])
                        .max()
                        .unwrap_or(0)
                })
                .unwrap_or(0),
            Conflict::RootUnit => 0,
        }
    }

    fn analyze_conflict_to_scratch(&mut self, conflict: Conflict) -> usize {
        if self.use_lbd {
            self.analyze_conflict_to_scratch_impl::<true>(conflict)
        } else {
            self.analyze_conflict_to_scratch_impl::<false>(conflict)
        }
    }

    fn analyze_conflict_to_scratch_impl<const LBD_META: bool>(
        &mut self,
        conflict: Conflict,
    ) -> usize {
        let current_level = self.current_level();
        self.scratch_learned.clear();
        self.scratch_bumped_vars.clear();

        let mut current_level_count = 0usize;

        self.mark_conflict_literals_for_analysis(conflict, current_level, &mut current_level_count);

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

            let reason_ref = self.reason[var].as_ref_unchecked();
            if reason_ref != ReasonRef::None {
                let start_lit_pos = if self.use_resolved_conflict_analysis {
                    0
                } else {
                    1
                };
                self.mark_reason_literals_for_analysis::<LBD_META>(
                    reason_ref,
                    var,
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
        if LBD_META {
            let lbd = self.compute_lbd_from_lits(&learned_clause);
            self.last_conflict_lbd = lbd;
            self.record_lbd_measurement(lbd);
            self.record_current_mode_lbd(lbd);
        } else {
            self.last_conflict_lbd = 0;
        }

        let mut backtrack_level = 0usize;
        let mut backtrack_pos = 1usize;
        for (pos, &lit) in learned_clause.iter().enumerate().skip(1) {
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

        self.iterating = learned_clause.len() == 1;
        self.scratch_conflict_clause = learned_clause;
        for &var in &self.scratch_bumped_vars {
            self.scratch_seen[var] = 0;
            self.scratch_resolved[var] = 0;
        }
        backtrack_level
    }

    #[cfg(test)]
    fn analyze_conflict(&mut self, conflict: Conflict) -> (Vec<i32>, usize) {
        let backtrack_level = self.analyze_conflict_to_scratch(conflict);
        let learned_clause = self.scratch_conflict_clause.clone();
        (learned_clause, backtrack_level)
    }

    #[cfg(test)]
    fn learned_clause_count(&self) -> usize {
        self.learned_clause_ids.len()
    }

    fn live_original_clause_count(&self) -> usize {
        self.original_clause_ids
            .iter()
            .copied()
            .filter(|&clause_idx| {
                clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx)
            })
            .count()
    }

    fn binary_clause_count_final(&self) -> usize {
        if self.binary_fast_path {
            return self
                .binary_clauses
                .iter()
                .filter(|binary| !binary.deleted)
                .count();
        }
        self.original_clause_ids
            .iter()
            .chain(self.learned_clause_ids.iter())
            .copied()
            .filter(|&clause_idx| {
                clause_idx < self.arena.len()
                    && !self.clause_is_deleted(clause_idx)
                    && self.clause_len(clause_idx) == 2
            })
            .count()
    }

    fn binary_implication_edge_count_final(&self) -> usize {
        if self.binary_fast_path {
            self.binary_clauses
                .iter()
                .filter(|binary| !binary.deleted)
                .count()
                .saturating_mul(2)
        } else {
            self.binary_clause_count_final().saturating_mul(2)
        }
    }

    fn formula_stats_snapshot(
        &self,
        vars: usize,
        original_clauses_initial: usize,
        original_lits_initial: u64,
    ) -> FormulaStats {
        let binary_clauses_final = self.binary_clause_count_final() as u64;
        let clause_db = self.clause_db_measurement();
        FormulaStats {
            vars: vars as u64,
            original_clauses_initial: original_clauses_initial as u64,
            original_lits_initial,
            original_clauses_after_preprocess: self.live_original_clause_count() as u64,
            original_lits_after_preprocess: self.original_literals as u64,
            learned_clauses_final: self.live_learned_clause_count as u64,
            learned_lits_final: self.learned_literals as u64,
            arena_words_live: clause_db.arena_words_live as u64,
            arena_words_garbage: clause_db.arena_words_garbage as u64,
            arena_garbage_ratio: clause_db.arena_garbage_ratio(),
            learned_words_live: clause_db.learned_words_live as u64,
            original_words_live: clause_db.original_words_live as u64,
            watchers_live: clause_db.watchers_live as u64,
            watchers_stale: clause_db.watchers_stale as u64,
            deleted_words: self.deleted_clause_words as u64,
            binary_clauses_final,
            binary_implication_edges_final: self.binary_implication_edge_count_final() as u64,
            max_clause_buffer_len: self.scratch_conflict_clause.len() as u64,
        }
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

    fn record_live_original_clauses_for_proof(&self, proof_log: &mut ProofLog) {
        for &clause_idx in &self.original_clause_ids {
            if clause_idx < self.arena.len() && !self.clause_is_deleted(clause_idx) {
                proof_log.record_clause(self.clause_slice(clause_idx));
            }
        }
    }

    #[cfg(test)]
    fn solve(&mut self) -> bool {
        let mut proof_log = ProofLog::disabled();
        let config = SolverConfig::default();
        self.solve_with_proof(&mut proof_log, &config)
    }

    fn solve_to_output(
        &mut self,
        output_dir: &str,
        config: &SolverConfig,
    ) -> (SolveOutcome, ProofStats) {
        let mut proof_log = match config.proof_policy {
            ProofPolicy::Off => ProofLog::disabled(),
            ProofPolicy::Drat => {
                ProofLog::new(output_dir, PROOF_BUFFER_CAPACITY, config.trace_proof)
            }
            ProofPolicy::Lrat => {
                eprintln!("SAT_PROOF=lrat is not implemented yet");
                std::process::exit(2);
            }
        };
        let outcome = self.solve_status_with_proof(&mut proof_log, config);
        let proof_start = Instant::now();
        match outcome.status {
            SolveStatus::Sat => proof_log.finish_sat(),
            SolveStatus::Unsat => proof_log.finish_unsat(),
            SolveStatus::Unknown => proof_log.finish_unknown(),
            SolveStatus::ParseError => unreachable!("parse errors do not enter solve_to_output"),
        }
        self.stats.proof_sec = proof_start.elapsed().as_secs_f64();
        (outcome, proof_log.snapshot())
    }

    #[cfg(test)]
    fn solve_with_proof(&mut self, proof_log: &mut ProofLog, config: &SolverConfig) -> bool {
        self.solve_status_with_proof(proof_log, config).status == SolveStatus::Sat
    }

    fn limit_hit(
        &self,
        limits: &RuntimeLimits,
        solve_start: Instant,
        proof_log: &ProofLog,
    ) -> Option<LimitHit> {
        if let Some(limit) = limits.conflict_limit {
            if self.stats.conflicts > limit {
                return Some(LimitHit::solve("conflict-limit"));
            }
        }
        if let Some(limit) = limits.propagation_limit {
            if self.stats.propagations > limit {
                return Some(LimitHit::solve("propagation-limit"));
            }
        }
        if let Some(limit) = limits.tick_limit {
            let ticks = self
                .stats
                .conflicts
                .saturating_add(self.stats.decisions)
                .saturating_add(self.stats.propagations);
            if ticks > limit {
                return Some(LimitHit::solve("tick-limit"));
            }
        }
        if let Some(limit) = limits.wall_limit_sec {
            if solve_start.elapsed().as_secs_f64() >= limit {
                return Some(LimitHit::solve("wall-clock-limit"));
            }
        }
        if let Some(limit) = limits.rss_limit_mb {
            if max_rss_mb().is_some_and(|rss| rss >= limit) {
                return Some(LimitHit::emergency_memory("rss-limit"));
            }
        }
        if let Some(limit) = limits.learned_lit_limit {
            if (self.learned_literals as u64) > limit {
                return Some(LimitHit::solve("learned-literal-limit"));
            }
        }
        if let Some(limit) = limits.binary_clause_limit {
            if (self.binary_clause_count_final() as u64) > limit {
                return Some(LimitHit::solve("binary-clause-limit"));
            }
        }
        if let Some(limit) = limits.extension_bytes_limit {
            let extension_bytes = (self.elim_clauses.len() as u64).saturating_mul(4);
            if extension_bytes > limit {
                return Some(LimitHit::solve("extension-bytes-limit"));
            }
        }
        if let Some(limit) = limits.proof_bytes_limit {
            if proof_log.bytes_written_estimate() > limit {
                return Some(LimitHit::solve("proof-bytes-limit"));
            }
        }
        None
    }

    fn solve_status_with_proof(
        &mut self,
        proof_log: &mut ProofLog,
        config: &SolverConfig,
    ) -> SolveOutcome {
        let solve_start = Instant::now();
        let runtime_limits = RuntimeLimits::from_config(config);
        let limits_active = runtime_limits.is_active();
        if !self.solver_ok || self.has_empty_clause || !self.enqueue_root_units() {
            return SolveOutcome::unsat();
        }

        if self.propagate().is_some() {
            return SolveOutcome::unsat();
        }

        self.record_live_original_clauses_for_proof(proof_log);
        if limits_active {
            if let Some(limit) = self.limit_hit(&runtime_limits, solve_start, proof_log) {
                let _class = limit.class.as_str();
                return SolveOutcome::unknown(limit.reason);
            }
        }

        let preprocess_start = Instant::now();
        if !self.eliminate(true, proof_log) {
            self.stats.preprocess_sec = preprocess_start.elapsed().as_secs_f64();
            return SolveOutcome::unsat();
        }
        self.stats.preprocess_sec = preprocess_start.elapsed().as_secs_f64();
        if limits_active {
            if let Some(limit) = self.limit_hit(&runtime_limits, solve_start, proof_log) {
                let _class = limit.class.as_str();
                return SolveOutcome::unknown(limit.reason);
            }
        }
        self.reset_learned_budget_after_preprocess();
        if config.trace_preprocess {
            eprintln!(
                "c preprocess seconds={:.3} eliminated={} resolvents={} subsumed={} strengthened={} original_vars={} original_clauses={} original_literals={} root_assigns={} deleted_clauses={} reduce_db_limit={}",
                preprocess_start.elapsed().as_secs_f64(),
                self.stats.preprocess_eliminated_vars,
                self.stats.preprocess_resolvents,
                self.stats.preprocess_subsumed_clauses,
                self.stats.preprocess_strengthened_clauses,
                self.live_original_variable_count(),
                self.original_clause_ids.len(),
                self.original_literals,
                self.trail.len(),
                self.stats.deleted_clauses,
                self.reduce_db_limit,
            );
        }
        if self.trace_preprocess_details {
            let avg_best_occurs = if self.stats.bsr_drivers == 0 {
                0.0
            } else {
                self.stats.bsr_best_occurs_sum as f64 / self.stats.bsr_drivers as f64
            };
            eprintln!(
                "c preprocess_detail bsr_runs={} seeded={} drivers={} clause_drivers={} root_drivers={} driver_lits={} candidates={} skip_self={} skip_deleted={} skip_limit={} relation_calls={} len_reject={} abstraction_reject={} sorted_calls={} nested_calls={} relation_subsumed={} relation_strengthen={} avg_best_occurs={:.3} max_best_occurs={} clean_calls={} clean_dirty={} clean_membership={} clean_scanned={} clean_removed={}",
                self.stats.bsr_runs,
                self.stats.bsr_seeded_clauses,
                self.stats.bsr_drivers,
                self.stats.bsr_clause_drivers,
                self.stats.bsr_root_drivers,
                self.stats.bsr_driver_lits,
                self.stats.bsr_candidates_seen,
                self.stats.bsr_skip_self,
                self.stats.bsr_skip_deleted,
                self.stats.bsr_skip_limit,
                self.stats.bsr_relation_calls,
                self.stats.bsr_relation_len_reject,
                self.stats.bsr_relation_abstraction_reject,
                self.stats.bsr_relation_sorted_calls,
                self.stats.bsr_relation_nested_calls,
                self.stats.bsr_relation_subsumed,
                self.stats.bsr_relation_strengthen,
                avg_best_occurs,
                self.stats.bsr_best_occurs_max,
                self.stats.occurs_clean_calls,
                self.stats.occurs_clean_dirty_calls,
                self.stats.occurs_clean_membership_calls,
                self.stats.occurs_clean_entries_scanned,
                self.stats.occurs_clean_entries_removed,
            );
        }

        let trace_search_interval = config.trace_search_interval as u64;
        let mut next_search_trace = trace_search_interval;
        let search_start = Instant::now();
        self.begin_search_mode_timing();
        let mut conflict = self.propagate();

        loop {
            match conflict {
                Some(conflict_event) => {
                    if self.current_level() == 0 {
                        self.finish_search_timing(search_start);
                        if trace_search_interval > 0 {
                            eprintln!(
                                "c search done result=UNSAT seconds={:.3} conflicts={} decisions={} propagations={} restarts={} learned={} reduce_db={}",
                                search_start.elapsed().as_secs_f64(),
                                self.stats.conflicts,
                                self.stats.decisions,
                                self.stats.propagations,
                                self.stats.restarts,
                                self.live_learned_clause_count,
                                self.stats.reduce_db_calls,
                            );
                        }
                        return SolveOutcome::unsat();
                    }
                    if self.binary_fast_path {
                        let conflict_level = self.conflict_max_decision_level(conflict_event);
                        if conflict_level == 0 {
                            self.finish_search_timing(search_start);
                            return SolveOutcome::unsat();
                        }
                        if conflict_level < self.current_level() {
                            self.backtrack(conflict_level);
                            conflict = Some(conflict_event);
                            continue;
                        }
                    }

                    self.stats.conflicts += 1;
                    self.record_search_conflict_mode();
                    if limits_active {
                        if let Some(limit) = self.limit_hit(&runtime_limits, solve_start, proof_log)
                        {
                            self.finish_search_timing(search_start);
                            let _class = limit.class.as_str();
                            return SolveOutcome::unknown(limit.reason);
                        }
                    }
                    if trace_search_interval > 0 && self.stats.conflicts >= next_search_trace {
                        eprintln!(
                            "c search seconds={:.3} conflicts={} decisions={} propagations={} restarts={} level={} trail={} learned={} reduce_db={} orig_clauses={} orig_literals={}",
                            search_start.elapsed().as_secs_f64(),
                            self.stats.conflicts,
                            self.stats.decisions,
                            self.stats.propagations,
                            self.stats.restarts,
                            self.current_level(),
                            self.trail.len(),
                            self.live_learned_clause_count,
                            self.stats.reduce_db_calls,
                            self.original_clause_ids.len(),
                            self.original_literals,
                        );
                        next_search_trace = next_search_trace.saturating_add(trace_search_interval);
                    }
                    let assertion_level = self.analyze_conflict_to_scratch(conflict_event);
                    self.bump_analyzed_variable_activity();
                    self.decay_variable_activity();
                    if self.reduce_db_enabled() {
                        self.decay_clause_activity();
                        self.note_learnt_budget_conflict();
                    }
                    self.note_conflict();
                    self.maybe_switch_search_mode_after_conflict();
                    let learned_clause = std::mem::take(&mut self.scratch_conflict_clause);
                    let asserting_lit = learned_clause[0];
                    proof_log.record_clause(&learned_clause);
                    if learned_clause.len() == 1 {
                        debug_assert_eq!(assertion_level, 0);
                        self.backtrack(0);
                        let inserted = self.enqueue(asserting_lit, ReasonRef::None);
                        if !inserted {
                            self.finish_search_timing(search_start);
                            return SolveOutcome::unsat();
                        }
                        self.otfs_subsume_watched_clauses(&learned_clause, None, proof_log);
                        self.scratch_conflict_clause = learned_clause;
                        self.scratch_conflict_clause.clear();
                    } else {
                        let backtrack_level = if self.chrono_backtrack {
                            self.choose_backtrack_level(assertion_level, &learned_clause)
                        } else {
                            assertion_level
                        };
                        let learned_clause_idx =
                            self.add_analyzed_clause_from_slice(&learned_clause);
                        if self.reduce_db_enabled() {
                            self.bump_clause_activity(learned_clause_idx);
                        }

                        self.backtrack(backtrack_level);
                        self.debug_assert_clause_asserting_after_backtrack(
                            self.clause_slice(learned_clause_idx),
                            backtrack_level,
                        );
                        let inserted = self.enqueue(
                            asserting_lit,
                            self.reason_ref_for_clause(learned_clause_idx),
                        );
                        debug_assert!(inserted, "learned clause must be asserting after backtrack");
                        self.otfs_subsume_watched_clauses(
                            &learned_clause,
                            Some(learned_clause_idx),
                            proof_log,
                        );
                        if self.reduce_policy == ReducePolicy::LbdTiered {
                            self.mark_learned_clause_recent(learned_clause_idx);
                        }
                        self.scratch_conflict_clause = learned_clause;
                        self.scratch_conflict_clause.clear();
                    }

                    conflict = self.propagate();
                    if limits_active {
                        if let Some(limit) = self.limit_hit(&runtime_limits, solve_start, proof_log)
                        {
                            self.finish_search_timing(search_start);
                            let _class = limit.class.as_str();
                            return SolveOutcome::unknown(limit.reason);
                        }
                    }
                }
                None => {
                    if self.run_post_propagation_scheduling() {
                        conflict = self.propagate();
                        continue;
                    }

                    self.maybe_capture_phase_prefix();

                    if self.current_level() == 0 {
                        if self.gc_pending_reason != GcReason::None {
                            self.maybe_garbage_collect(GcReason::ArenaFragmentation);
                        }
                        if !self.simplify_with_proof(proof_log) {
                            self.finish_search_timing(search_start);
                            return SolveOutcome::unsat();
                        }
                    }

                    if self.reduce_db_enabled() && self.should_reduce_db() {
                        self.reduce_db_with_proof(proof_log);
                    }

                    if limits_active {
                        if let Some(limit) = self.limit_hit(&runtime_limits, solve_start, proof_log)
                        {
                            self.finish_search_timing(search_start);
                            let _class = limit.class.as_str();
                            return SolveOutcome::unknown(limit.reason);
                        }
                    }

                    match self.pick_branch_lit() {
                        Some(lit) => {
                            self.decide(lit);
                            conflict = self.propagate();
                            if limits_active {
                                if let Some(limit) =
                                    self.limit_hit(&runtime_limits, solve_start, proof_log)
                                {
                                    self.finish_search_timing(search_start);
                                    let _class = limit.class.as_str();
                                    return SolveOutcome::unknown(limit.reason);
                                }
                            }
                        }
                        None => {
                            self.capture_sat_model();
                            self.finish_search_timing(search_start);
                            if trace_search_interval > 0 {
                                eprintln!(
                                    "c search done result=SAT seconds={:.3} conflicts={} decisions={} propagations={} restarts={} learned={} reduce_db={}",
                                    search_start.elapsed().as_secs_f64(),
                                    self.stats.conflicts,
                                    self.stats.decisions,
                                    self.stats.propagations,
                                    self.stats.restarts,
                                    self.live_learned_clause_count,
                                    self.stats.reduce_db_calls,
                                );
                            }
                            return SolveOutcome::sat();
                        }
                    }
                }
            }
        }
    }
}

fn parse_cnf(path: &str) -> Result<(usize, Vec<Vec<i32>>), String> {
    let file = fs::File::open(path).map_err(|e| format!("Error opening {path}: {e}"))?;
    let reader = io::BufReader::new(file);

    let mut num_vars: Option<usize> = None;
    let mut clauses: Vec<Vec<i32>> = Vec::new();
    let mut current_clause: Vec<i32> = Vec::new();

    for (line_idx, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("{path}:{}: read error: {e}", line_idx + 1))?;
        let line = line.trim();

        if line.is_empty() || line.starts_with('c') {
            continue;
        }

        if line.starts_with('p') {
            if num_vars.is_some() {
                return Err(format!("{path}:{}: duplicate problem line", line_idx + 1));
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() != 4 || parts[0] != "p" || parts[1] != "cnf" {
                return Err(format!(
                    "{path}:{}: malformed problem line, expected 'p cnf <vars> <clauses>'",
                    line_idx + 1
                ));
            }
            let parsed_vars = parts[2].parse().map_err(|e| {
                format!(
                    "{path}:{}: invalid variable count {:?}: {e}",
                    line_idx + 1,
                    parts[2]
                )
            })?;
            let _declared_clauses: usize = parts[3].parse().map_err(|e| {
                format!(
                    "{path}:{}: invalid clause count {:?}: {e}",
                    line_idx + 1,
                    parts[3]
                )
            })?;
            num_vars = Some(parsed_vars);
            continue;
        }

        let Some(declared_vars) = num_vars else {
            return Err(format!(
                "{path}:{}: literal data before problem line",
                line_idx + 1
            ));
        };

        for token in line.split_whitespace() {
            let lit: i32 = token
                .parse()
                .map_err(|e| format!("{path}:{}: invalid literal {token:?}: {e}", line_idx + 1))?;
            if lit == 0 {
                clauses.push(std::mem::take(&mut current_clause));
            } else {
                let var = lit.unsigned_abs() as usize;
                if var == 0 || var > declared_vars {
                    return Err(format!(
                        "{path}:{}: literal {lit} uses variable {var}, beyond declared bound {declared_vars}",
                        line_idx + 1
                    ));
                }
                current_clause.push(lit);
            }
        }
    }

    if !current_clause.is_empty() {
        return Err(format!("{path}: missing terminal 0 for final clause"));
    }

    let Some(num_vars) = num_vars else {
        return Err(format!("{path}: missing problem line"));
    };

    Ok((num_vars, clauses))
}

fn initial_lit_count(clauses: &[Vec<i32>]) -> u64 {
    clauses.iter().map(|clause| clause.len() as u64).sum()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MemoryPreflight {
    estimated_peak_bytes: u64,
    limit_bytes: u64,
    threshold_bytes: u64,
}

fn bytes_for_elems(count: u64, elem_size: usize) -> u64 {
    count.saturating_mul(elem_size as u64)
}

fn solver_construction_extra_bytes(num_vars: usize, clauses: &[Vec<i32>]) -> u64 {
    let vars = num_vars as u64;
    let vars_with_zero = vars.saturating_add(1);
    let lits = initial_lit_count(clauses);
    let clause_count = clauses.len() as u64;
    let literal_slots = lits.saturating_add(clause_count);

    let mut bytes = 0u64;
    bytes = bytes.saturating_add(bytes_for_elems(
        vars_with_zero,
        std::mem::size_of::<usize>(),
    )); // occurrence_count
    bytes = bytes.saturating_add(bytes_for_elems(vars, std::mem::size_of::<u32>())); // branch_order
    bytes = bytes.saturating_add(bytes_for_elems(
        vars_with_zero,
        std::mem::size_of::<usize>(),
    )); // branch_rank
    bytes = bytes.saturating_add(bytes_for_elems(literal_slots, std::mem::size_of::<u32>())); // arena
    bytes = bytes.saturating_add(bytes_for_elems(clause_count, std::mem::size_of::<usize>())); // original_clause_ids
    bytes = bytes.saturating_add(bytes_for_elems(
        vars.saturating_mul(2),
        std::mem::size_of::<Vec<Watcher>>(),
    )); // watchers
    bytes = bytes.saturating_add(bytes_for_elems(
        clause_count.saturating_mul(2),
        std::mem::size_of::<Watcher>(),
    )); // watcher entries
    bytes = bytes.saturating_add(bytes_for_elems(vars_with_zero, std::mem::size_of::<u8>())); // assignment
    bytes = bytes.saturating_add(bytes_for_elems(vars_with_zero, std::mem::size_of::<u8>())); // saved_phase
    bytes = bytes.saturating_add(bytes_for_elems(
        vars_with_zero,
        std::mem::size_of::<usize>(),
    )); // decision_level
    bytes = bytes.saturating_add(bytes_for_elems(
        vars_with_zero,
        std::mem::size_of::<ReasonCode>(),
    )); // reason
    bytes = bytes.saturating_add(bytes_for_elems(vars, std::mem::size_of::<i32>())); // trail
    bytes = bytes.saturating_add(bytes_for_elems(vars, std::mem::size_of::<u32>())); // branch_heap
    bytes = bytes.saturating_add(bytes_for_elems(
        vars_with_zero,
        std::mem::size_of::<usize>(),
    )); // branch_pos
    bytes = bytes.saturating_add(bytes_for_elems(vars_with_zero, std::mem::size_of::<bool>())); // decision_var
    bytes = bytes.saturating_add(bytes_for_elems(vars_with_zero, std::mem::size_of::<f64>())); // activity
    bytes = bytes.saturating_add(bytes_for_elems(vars_with_zero, std::mem::size_of::<bool>())); // frozen
    bytes = bytes.saturating_add(bytes_for_elems(vars_with_zero, std::mem::size_of::<bool>())); // eliminated
    bytes = bytes.saturating_add(bytes_for_elems(
        vars_with_zero,
        std::mem::size_of::<Vec<u32>>(),
    )); // occurs
    bytes = bytes.saturating_add(bytes_for_elems(lits, std::mem::size_of::<u32>())); // occurs entries
    bytes = bytes.saturating_add(bytes_for_elems(vars_with_zero, std::mem::size_of::<bool>())); // occurs_dirty
    bytes = bytes.saturating_add(bytes_for_elems(vars_with_zero, std::mem::size_of::<bool>())); // occurs_membership_dirty
    bytes = bytes.saturating_add(bytes_for_elems(
        vars.saturating_mul(2),
        std::mem::size_of::<usize>(),
    )); // n_occ
    bytes = bytes.saturating_add(bytes_for_elems(vars_with_zero, std::mem::size_of::<u8>())); // scratch_seen
    bytes = bytes.saturating_add(bytes_for_elems(vars_with_zero, std::mem::size_of::<u8>())); // scratch_resolved
    bytes = bytes.saturating_add(bytes_for_elems(vars_with_zero, std::mem::size_of::<u8>())); // scratch_redundant_state
    bytes = bytes.saturating_add(bytes_for_elems(vars_with_zero, std::mem::size_of::<u32>())); // lbd_seen
    if clause_count >= INLINE_ABSTRACTION_CLAUSE_THRESHOLD as u64 {
        bytes = bytes.saturating_add(bytes_for_elems(literal_slots, std::mem::size_of::<usize>())); // inline-abstraction reloc
        bytes = bytes.saturating_add(bytes_for_elems(
            literal_slots.saturating_add(clause_count),
            std::mem::size_of::<u32>(),
        )); // inline-abstraction rebuilt arena
    }
    bytes
}

fn memory_preflight(
    num_vars: usize,
    clauses: &[Vec<i32>],
    config: &SolverConfig,
) -> Option<MemoryPreflight> {
    let limit_bytes = effective_memory_limit_bytes(config)?;
    let current_rss_bytes = max_rss_mb()
        .unwrap_or(0)
        .saturating_mul(1024)
        .saturating_mul(1024);
    memory_preflight_with_limit(num_vars, clauses, limit_bytes, current_rss_bytes)
}

fn memory_preflight_with_limit(
    num_vars: usize,
    clauses: &[Vec<i32>],
    limit_bytes: u64,
    current_rss_bytes: u64,
) -> Option<MemoryPreflight> {
    let estimated_peak_bytes =
        current_rss_bytes.saturating_add(solver_construction_extra_bytes(num_vars, clauses));
    let threshold_bytes = limit_bytes.saturating_mul(9) / 10;
    (estimated_peak_bytes >= threshold_bytes).then_some(MemoryPreflight {
        estimated_peak_bytes,
        limit_bytes,
        threshold_bytes,
    })
}

fn mb_ceil(bytes: u64) -> u64 {
    bytes.div_ceil(1024 * 1024)
}

fn verify_model_against_clauses(clauses: &[Vec<i32>], assignment: &[u8]) -> bool {
    for clause in clauses {
        let mut satisfied = false;
        for &lit in clause {
            let var = lit.unsigned_abs() as usize;
            let Some(&value) = assignment.get(var) else {
                return false;
            };
            if (lit > 0 && value == TRUE) || (lit < 0 && value == FALSE) {
                satisfied = true;
                break;
            }
        }
        if !satisfied {
            return false;
        }
    }
    true
}

fn verify_model_against_cnf_path(path: &str, assignment: &[u8]) -> bool {
    let Ok((_, clauses)) = parse_cnf(path) else {
        return false;
    };
    verify_model_against_clauses(&clauses, assignment)
}

fn main() {
    let run_start = Instant::now();
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: sat-solver <cnf_path> <output_dir>");
        std::process::exit(1);
    }

    let cnf_path = &args[1];
    let output_dir = &args[2];
    let output_path = Path::new(output_dir);

    let config = SolverConfig::from_env();
    config.emit_requested_outputs();
    let input_identity = InputIdentity::from_path(Path::new(cnf_path));
    prepare_output_contract_dir(output_path);
    let parse_start = Instant::now();
    let (num_vars, clauses) = match parse_cnf(cnf_path) {
        Ok(parsed) => parsed,
        Err(message) => {
            let parse_sec = parse_start.elapsed().as_secs_f64();
            eprintln!("{message}");
            let output_contract = OutputContract {
                status: SolveStatus::ParseError,
                proof_completeness: ProofCompleteness::None,
                model_written: false,
                proof_written: output_path.join(PROOF_OUT).exists(),
                stats_written: config.stats_json,
                result_json_written: true,
                output_contract_state: OutputContractState::Complete,
            };
            if let Err(reason) = output_contract.validate() {
                eprintln!("internal output contract validation failed: {reason}");
                std::process::exit(2);
            }
            let fields = ResultContractFields::new(
                Some(&message),
                input_identity.sha256.as_deref(),
                "not_applicable",
                "not_applicable",
                output_contract.output_contract_state.as_str(),
            );
            write_result_contract(
                output_path,
                SolveStatus::ParseError,
                &config,
                &fields,
                None,
                ProofCompleteness::None,
            );
            if config.stats_json {
                let timings = RunTimings {
                    elapsed_sec: run_start.elapsed().as_secs_f64(),
                    parse_sec,
                    ..RunTimings::default()
                };
                let formula = FormulaStats::default();
                let stats = SolverStats::default();
                let proof = ProofStats {
                    state: "not-created",
                    ..ProofStats::default()
                };
                let ctx = StatsJsonContext {
                    config: &config,
                    stats: &stats,
                    proof: &proof,
                    input: &input_identity,
                    formula: &formula,
                    timings: &timings,
                    status: SolveStatus::ParseError,
                    exit_code: SolveStatus::ParseError.exit_code(),
                    status_file_status: Some(SolveStatus::ParseError.as_str()),
                    termination_reason: SolveStatus::ParseError.termination_reason(),
                    unknown_reason: Some(&message),
                    limit_hit: false,
                    parse_error_kind: Some("dimacs-parse-error"),
                    model_check_result: "not_applicable",
                    proof_check_result: "not_applicable",
                    proof_completeness: ProofCompleteness::None,
                    output_contract_state: "complete",
                    max_rss_mb: max_rss_mb(),
                };
                eprintln!("{}", json_stats_line(&ctx));
            }
            println!("{}", SolveStatus::ParseError.s_line());
            std::process::exit(SolveStatus::ParseError.exit_code());
        }
    };
    let parse_sec = parse_start.elapsed().as_secs_f64();
    let original_clause_count = clauses.len();
    let original_lits_initial = initial_lit_count(&clauses);
    if let Some(preflight) = memory_preflight(num_vars, &clauses, &config) {
        eprintln!(
            "c memory_preflight result=UNKNOWN reason=memory-preflight-limit vars={} clauses={} literals={} estimated_peak_mb={} limit_mb={} threshold_mb={}",
            num_vars,
            original_clause_count,
            original_lits_initial,
            mb_ceil(preflight.estimated_peak_bytes),
            mb_ceil(preflight.limit_bytes),
            mb_ceil(preflight.threshold_bytes),
        );
        let proof_completeness = match config.proof_policy {
            ProofPolicy::Off => ProofCompleteness::NotRequested,
            ProofPolicy::Drat | ProofPolicy::Lrat => ProofCompleteness::Incomplete,
        };
        let output_contract = OutputContract {
            status: SolveStatus::Unknown,
            proof_completeness,
            model_written: false,
            proof_written: output_path.join(PROOF_OUT).exists(),
            stats_written: config.stats_json,
            result_json_written: true,
            output_contract_state: OutputContractState::Complete,
        };
        if let Err(reason) = output_contract.validate() {
            eprintln!("internal output contract validation failed: {reason}");
            std::process::exit(2);
        }
        let fields = ResultContractFields::new(
            Some("memory-preflight-limit"),
            input_identity.sha256.as_deref(),
            "not_applicable",
            "not_applicable",
            output_contract.output_contract_state.as_str(),
        )
        .with_termination_reason(SolveStatus::Unknown.termination_reason());
        write_result_contract(
            output_path,
            SolveStatus::Unknown,
            &config,
            &fields,
            None,
            proof_completeness,
        );
        if config.stats_json {
            let timings = RunTimings {
                elapsed_sec: run_start.elapsed().as_secs_f64(),
                parse_sec,
                ..RunTimings::default()
            };
            let formula = FormulaStats {
                vars: num_vars as u64,
                original_clauses_initial: original_clause_count as u64,
                original_lits_initial,
                ..FormulaStats::default()
            };
            let stats = SolverStats::default();
            let proof = ProofStats {
                state: "not-created",
                ..ProofStats::default()
            };
            let ctx = StatsJsonContext {
                config: &config,
                stats: &stats,
                proof: &proof,
                input: &input_identity,
                formula: &formula,
                timings: &timings,
                status: SolveStatus::Unknown,
                exit_code: SolveStatus::Unknown.exit_code(),
                status_file_status: Some(SolveStatus::Unknown.as_str()),
                termination_reason: SolveStatus::Unknown.termination_reason(),
                unknown_reason: Some("memory-preflight-limit"),
                limit_hit: true,
                parse_error_kind: None,
                model_check_result: "not_applicable",
                proof_check_result: "not_applicable",
                proof_completeness,
                output_contract_state: "complete",
                max_rss_mb: max_rss_mb(),
            };
            eprintln!("{}", json_stats_line(&ctx));
        }
        println!("{}", SolveStatus::Unknown.s_line());
        std::process::exit(SolveStatus::Unknown.exit_code());
    }
    let mut solver = Solver::new_with_config(num_vars, clauses, &config);

    let (outcome, proof_stats) = solver.solve_to_output(output_dir, &config);
    let status = outcome.status;
    let model_path = if status == SolveStatus::Sat {
        let model = solver
            .sat_model
            .as_ref()
            .expect("SAT solver returned without a model snapshot");
        Some(write_model_file(output_path, model))
    } else {
        None
    };
    let model_check_result = if status == SolveStatus::Sat {
        let model = solver
            .sat_model
            .as_ref()
            .expect("SAT solver returned without a model snapshot");
        if !config.check_invariants {
            "not_checked"
        } else if verify_model_against_cnf_path(cnf_path, model) {
            "pass"
        } else {
            "fail"
        }
    } else {
        "not_applicable"
    };
    if model_check_result == "fail" {
        eprintln!("internal model check failed for {}", cnf_path);
    }
    let proof_completeness = match (status, config.proof_policy) {
        (SolveStatus::Unsat, ProofPolicy::Drat) => ProofCompleteness::Complete,
        (SolveStatus::Unsat, ProofPolicy::Off) => ProofCompleteness::NotRequested,
        (SolveStatus::Sat, ProofPolicy::Drat) => ProofCompleteness::Incomplete,
        (SolveStatus::Sat, ProofPolicy::Off) => ProofCompleteness::NotRequested,
        (SolveStatus::Unknown, ProofPolicy::Drat) => ProofCompleteness::Incomplete,
        (SolveStatus::Unknown, ProofPolicy::Off) => ProofCompleteness::NotRequested,
        _ => ProofCompleteness::None,
    };
    let output_contract = OutputContract {
        status,
        proof_completeness,
        model_written: model_path.is_some(),
        proof_written: output_path.join(PROOF_OUT).exists(),
        stats_written: config.stats_json,
        result_json_written: true,
        output_contract_state: OutputContractState::Complete,
    };
    if let Err(reason) = output_contract.validate() {
        eprintln!("internal output contract validation failed: {reason}");
        std::process::exit(2);
    }
    let fields = ResultContractFields::new(
        outcome.unknown_reason,
        input_identity.sha256.as_deref(),
        model_check_result,
        "not_checked",
        output_contract.output_contract_state.as_str(),
    )
    .with_termination_reason(outcome.termination_reason());
    write_result_contract(
        output_path,
        status,
        &config,
        &fields,
        model_path.as_deref(),
        proof_completeness,
    );
    let timings = RunTimings {
        elapsed_sec: run_start.elapsed().as_secs_f64(),
        parse_sec,
        preprocess_sec: solver.stats.preprocess_sec,
        search_sec: solver.stats.search_sec,
        proof_sec: solver.stats.proof_sec,
    };
    if config.stats_json {
        let formula =
            solver.formula_stats_snapshot(num_vars, original_clause_count, original_lits_initial);
        let ctx = StatsJsonContext {
            config: &config,
            stats: &solver.stats,
            proof: &proof_stats,
            input: &input_identity,
            formula: &formula,
            timings: &timings,
            status,
            exit_code: status.exit_code(),
            status_file_status: Some(status.as_str()),
            termination_reason: outcome.termination_reason(),
            unknown_reason: outcome.unknown_reason,
            limit_hit: status == SolveStatus::Unknown && outcome.unknown_reason.is_some(),
            parse_error_kind: None,
            model_check_result,
            proof_check_result: "not_checked",
            proof_completeness,
            output_contract_state: "complete",
            max_rss_mb: max_rss_mb(),
        };
        eprintln!("{}", json_stats_line(&ctx));
    }
    if config.trace_full {
        eprintln!("{}", trace_full_line(&solver.stats, &timings));
    }
    if solver.use_lbd {
        println!(
            "c lbd computed={} sum={} max={}",
            solver.stats.lbd_computed, solver.stats.lbd_sum, solver.stats.lbd_max
        );
    }
    if status == SolveStatus::Sat {
        println!("{}", status.s_line());
        let model = solver
            .sat_model
            .as_ref()
            .expect("SAT solver returned without a model snapshot");
        print_assignment(model);
    } else {
        println!("{}", status.s_line());
    }
    std::process::exit(status.exit_code());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn make_solver(num_vars: usize, clauses: Vec<Vec<i32>>) -> Solver {
        Solver::new(num_vars, clauses)
    }

    fn make_solver_with_config(
        num_vars: usize,
        clauses: Vec<Vec<i32>>,
        config: &SolverConfig,
    ) -> Solver {
        Solver::new_with_config(num_vars, clauses, config)
    }

    fn binary_fast_config() -> SolverConfig {
        SolverConfig {
            binary_fast_path: true,
            ..Default::default()
        }
    }

    fn binary_fast_hot_stats_config() -> SolverConfig {
        SolverConfig {
            binary_fast_path: true,
            hot_stats: true,
            ..Default::default()
        }
    }

    fn chrono_config(max_delta: usize) -> SolverConfig {
        SolverConfig {
            chrono_backtrack: true,
            chrono_max_delta: max_delta,
            ..Default::default()
        }
    }

    #[test]
    fn test_binary_fast_path_keeps_configured_clause_minimization() {
        let fast_config = SolverConfig {
            binary_fast_path: true,
            clause_min_mode: ClauseMinMode::RecursiveLimited,
            ..Default::default()
        };
        let fast = make_solver_with_config(2, vec![vec![1, 2]], &fast_config);

        assert_eq!(fast.ccmin_mode, CCMIN_DEEP);

        let legacy_config = SolverConfig {
            clause_min_mode: ClauseMinMode::Basic,
            ..Default::default()
        };
        let legacy = make_solver_with_config(2, vec![vec![1, 2]], &legacy_config);

        assert_eq!(legacy.ccmin_mode, CCMIN_BASIC);
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

    fn decision_prefix(mut solver: Solver, limit: usize) -> Vec<i32> {
        let mut prefix = Vec::new();
        for _ in 0..limit {
            let Some(lit) = solver.pick_branch_lit() else {
                break;
            };
            prefix.push(lit);
            solver.decide(lit);
        }
        prefix
    }

    #[test]
    fn memory_preflight_accounts_for_dense_watchers() {
        let clauses = vec![vec![1, 2], vec![-1, 3]];
        let estimated = solver_construction_extra_bytes(10, &clauses);
        let watcher_headers = bytes_for_elems(20, std::mem::size_of::<Vec<Watcher>>());

        assert!(estimated >= watcher_headers);
    }

    #[test]
    fn memory_preflight_trips_before_dense_allocation_exceeds_limit() {
        let clauses = vec![vec![1, -2, 3]; 100];
        let extra = solver_construction_extra_bytes(10_000, &clauses);
        let current = 64 * 1024 * 1024;
        let limit = current + extra;

        let preflight = memory_preflight_with_limit(10_000, &clauses, limit, current)
            .expect("90% threshold should trip before allocation reaches the cap");

        assert_eq!(preflight.limit_bytes, limit);
        assert!(preflight.estimated_peak_bytes >= preflight.threshold_bytes);
    }

    #[test]
    fn memory_preflight_allows_small_instance_under_limit() {
        let clauses = vec![vec![1, -2, 3]; 10];
        let limit = 1024 * 1024 * 1024;

        assert!(memory_preflight_with_limit(10, &clauses, limit, 0).is_none());
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
            s.set_reason_ref(var, ReasonRef::Clause(reason_idx));
        }
    }

    #[test]
    fn test_lbd_single_level_clause_is_1() {
        let mut s = make_solver(2, vec![]);
        s.decision_level[1] = 4;
        s.decision_level[2] = 4;

        assert_eq!(s.compute_lbd_from_lits(&[1, -2]), 1);
    }

    #[test]
    fn test_lbd_binary_across_two_levels_is_2() {
        let mut s = make_solver(2, vec![]);
        s.decision_level[1] = 1;
        s.decision_level[2] = 2;

        assert_eq!(s.compute_lbd_from_lits(&[1, -2]), 2);
    }

    #[test]
    fn test_lbd_ignores_duplicate_decision_levels() {
        let mut s = make_solver(5, vec![]);
        s.decision_level[1] = 1;
        s.decision_level[2] = 1;
        s.decision_level[3] = 2;
        s.decision_level[4] = 0;
        s.decision_level[5] = 3;

        assert_eq!(s.compute_lbd_from_lits(&[]), 0);
        assert_eq!(s.compute_lbd_from_lits(&[1, -2, 3, -4, 5]), 4);
    }

    #[test]
    fn test_lbd_stamp_wrap_clears_seen() {
        let mut s = make_solver(2, vec![]);
        s.decision_level[1] = 1;
        s.decision_level[2] = 2;
        s.lbd_seen[1] = u32::MAX;
        s.lbd_stamp = u32::MAX;

        assert_eq!(s.compute_lbd_from_lits(&[1, 2]), 2);
        assert_eq!(s.lbd_stamp, 1);
        assert_eq!(s.lbd_seen[1], 1);
        assert_eq!(s.lbd_seen[2], 1);
    }

    #[test]
    fn test_compute_lbd_for_clause_matches_compute_lbd_from_lits() {
        let mut s = make_solver(5, vec![]);
        s.decision_level[1] = 1;
        s.decision_level[2] = 3;
        s.decision_level[3] = 3;
        s.decision_level[4] = 0;
        s.decision_level[5] = 2;
        let clause_idx = s.add_clause(vec![1, -2, 3, -4, 5]);

        let expected = s.compute_lbd_from_lits(&[1, -2, 3, -4, 5]);
        let actual = s.compute_lbd_for_clause(clause_idx);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_compute_lbd_for_clause_no_allocation() {
        let mut s = make_solver(5, vec![]);
        s.decision_level[1] = 1;
        s.decision_level[2] = 3;
        s.decision_level[3] = 3;
        s.decision_level[4] = 0;
        s.decision_level[5] = 2;
        let clause_idx = s.add_clause(vec![1, -2, 3, -4, 5]);

        reset_test_allocations();
        let lbd = s.compute_lbd_for_clause(clause_idx);
        let allocations = test_allocation_count();
        stop_test_allocations();

        assert_eq!(lbd, 4);
        assert_eq!(allocations, 0);
    }

    #[test]
    fn test_lbd_reason_analysis_hot_path_no_allocation() {
        let mut s = make_solver(5, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.update_reason_lbd = true;
        s.decision_level[1] = 1;
        s.decision_level[2] = 3;
        s.decision_level[3] = 3;
        s.decision_level[4] = 0;
        s.decision_level[5] = 2;
        let reason = s.add_clause(vec![1, -2, 3, -4, 5]);
        set_lbd_meta_for_test(&mut s, reason, 9, 0);
        s.focused_glue_recent.resize(10, 0);
        s.stats.focused_glue_used.resize(10, 0);
        s.stable_glue_recent.resize(10, 0);
        s.stats.stable_glue_used.resize(10, 0);

        let mut current_level_count = 0;
        reset_test_allocations();
        s.mark_reason_literals_for_analysis::<true>(
            ReasonRef::Clause(reason),
            1,
            0,
            3,
            &mut current_level_count,
        );
        let allocations = test_allocation_count();
        stop_test_allocations();

        assert_eq!(allocations, 0);
        assert_eq!(s.learnt_lbd(reason), 4);
        assert_eq!(current_level_count, 2);
    }

    #[test]
    fn test_reason_none_for_decision() {
        let mut s = make_solver(2, vec![]);

        s.decide(1);

        assert_eq!(s.reason_ref(1), ReasonRef::None);
    }

    #[test]
    fn test_reason_clause_expands_lits() {
        let mut s = make_solver(3, vec![]);
        let clause_idx = s.add_clause(vec![2, -1, 3]);

        assert_eq!(
            s.reason_lits_for_test(ReasonRef::Clause(clause_idx)),
            vec![2, -1, 3]
        );
    }

    #[test]
    fn test_reason_binary_expands_lits() {
        let mut s = make_solver(3, vec![]);
        let binary_id = s.add_binary_reason_for_test([2, -1]);

        assert_eq!(
            s.reason_lits_for_test(ReasonRef::Binary(binary_id)),
            vec![2, -1]
        );
    }

    #[test]
    fn test_conflict_binary_expands_lits() {
        let mut s = make_solver(3, vec![]);
        let binary_id = s.add_binary_reason_for_test([-2, -1]);

        assert_eq!(
            s.conflict_lits_for_test(Conflict::Binary(binary_id)),
            vec![-2, -1]
        );
    }

    #[test]
    fn test_binary_implication_flat_add_edge_preserves_edge() {
        let mut implications = BinaryImplications::Flat {
            edges: Vec::new(),
            offsets: vec![0; 5],
            dirty: false,
        };
        let first = BinaryEdge {
            implied: 2,
            clause_id: BinaryClauseId(0),
        };
        let second = BinaryEdge {
            implied: -2,
            clause_id: BinaryClauseId(1),
        };

        implications.add_edge(1, first);
        implications.add_edge(-1, second);

        assert_eq!(implications.edges_for(1), &[first]);
        assert_eq!(implications.edges_for(-1), &[second]);
        assert!(matches!(
            implications,
            BinaryImplications::Flat { dirty: true, .. }
        ));
    }

    #[test]
    fn test_binary_propagates_implied_literal() {
        let config = binary_fast_hot_stats_config();
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        let clause_idx = s.original_clause_ids[0];
        let binary_id = s.binary_id_for_clause(clause_idx);

        assert_eq!(s.binary_implications.len_for(-1), 1);
        assert!(s.enqueue(-1, ReasonRef::None));
        assert!(s.propagate().is_none());

        assert_eq!(s.assignment[2], TRUE);
        assert_eq!(s.reason_ref(2), ReasonRef::Binary(binary_id));
        assert_eq!(s.stats.binary_props, 1);
    }

    #[test]
    fn test_binary_conflict_detected() {
        let config = binary_fast_config();
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        let clause_idx = s.original_clause_ids[0];
        let binary_id = s.binary_id_for_clause(clause_idx);

        assert!(s.enqueue(-1, ReasonRef::None));
        assert!(s.enqueue(-2, ReasonRef::None));

        assert_eq!(s.propagate(), Some(Conflict::Binary(binary_id)));
    }

    #[test]
    fn test_binary_conflict_level_uses_reason_literals() {
        let config = binary_fast_config();
        let mut s = make_solver_with_config(3, vec![vec![1, 2]], &config);
        let binary_id = s.binary_id_for_clause(s.original_clause_ids[0]);

        assert!(s.enqueue(-1, ReasonRef::None));
        assert!(s.enqueue(-2, ReasonRef::None));
        s.decide(3);

        assert_eq!(s.current_level(), 1);
        assert_eq!(
            s.conflict_max_decision_level(Conflict::Binary(binary_id)),
            0
        );
        s.backtrack(0);
        assert_eq!(s.current_level(), 0);
    }

    #[test]
    fn test_binary_reason_expands_in_analyze() {
        let config = binary_fast_config();
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        let binary_id = s.binary_id_for_clause(s.original_clause_ids[0]);
        assert!(s.enqueue(-1, ReasonRef::None));
        s.decide(-2);

        let (learned, backtrack_level) = s.analyze_conflict(Conflict::Binary(binary_id));

        assert_eq!(learned, vec![2]);
        assert_eq!(backtrack_level, 0);
        assert!(s.binary_clauses[binary_id.0 as usize].used_count > 0);
    }

    #[test]
    fn test_binary_reason_analysis_skips_resolved_variable_by_identity() {
        let config = binary_fast_config();
        let mut s = make_solver_with_config(3, vec![vec![1, 2], vec![1, 3], vec![-2, -3]], &config);

        s.decide(-1);
        let conflict = s.propagate().expect("expected binary conflict");
        let (learned, backtrack_level) = s.analyze_conflict(conflict);

        assert_eq!(learned, vec![1]);
        assert_eq!(backtrack_level, 0);
    }

    #[test]
    fn test_binary_original_clause_preserved_for_model_check() {
        let config = binary_fast_config();
        let s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        let clause_idx = s.original_clause_ids[0];

        assert_eq!(s.clause_slice(clause_idx), &[1, 2]);
        assert_eq!(
            s.binary_clause_lits_for_test(s.binary_id_for_clause(clause_idx)),
            [1, 2]
        );
    }

    #[test]
    fn test_binary_refs_survive_inline_abstraction_migration_and_gc() {
        let config = binary_fast_config();
        let mut s = make_solver_with_config(5, vec![vec![3, 4, 5], vec![1, 2]], &config);
        let old_binary_clause_idx = s.original_clause_ids[1];
        let binary_id = s.binary_id_for_clause(old_binary_clause_idx);

        // Force the large-formula inline-abstraction migration without constructing
        // a huge distinct formula.
        s.original_clause_ids
            .resize(INLINE_ABSTRACTION_CLAUSE_THRESHOLD, old_binary_clause_idx);
        s.ensure_original_clause_abstractions();

        let migrated_clause_idx = s.binary_clauses[binary_id.0 as usize].clause_ref;
        assert_ne!(migrated_clause_idx, old_binary_clause_idx);
        assert_eq!(
            s.try_binary_id_for_clause(migrated_clause_idx),
            Some(binary_id)
        );

        s.garbage_collect();

        assert!(!s.binary_clause_is_deleted(binary_id));
        assert!(s.enqueue(-1, ReasonRef::None));
        assert!(s.propagate().is_none());
        assert_eq!(s.assignment[2], TRUE);
    }

    #[test]
    fn test_binary_fast_and_legacy_same_result_on_small_oracle() {
        let clauses = vec![vec![1, 2], vec![-1, 2], vec![1, -2], vec![-1, -2]];
        let mut legacy = make_solver(2, clauses.clone());
        let config = binary_fast_config();
        let mut fast = make_solver_with_config(2, clauses, &config);

        assert_eq!(legacy.solve(), fast.solve());
    }

    #[test]
    fn test_binary_fast_path_sets_assignment() {
        let config = binary_fast_config();
        let mut s = make_solver_with_config(2, vec![vec![-1, 2]], &config);

        assert!(s.enqueue(1, ReasonRef::None));
        assert!(s.propagate().is_none());

        assert_eq!(s.assignment[2], TRUE);
    }

    #[test]
    fn test_binary_generated_clause_has_proof_id() {
        let config = binary_fast_config();
        let mut s = make_solver_with_config(2, vec![], &config);
        let clause_idx = s.add_clause(vec![1, 2]);
        let binary_id = s.binary_id_for_clause(clause_idx);
        let binary = &s.binary_clauses[binary_id.0 as usize];

        assert_eq!(binary.origin, BinaryOrigin::LearnedConflict);
        assert!(binary.redundant);
        assert!(binary.proof_logged);
    }

    #[test]
    fn test_binary_delete_marks_edge_stale_until_rebuild() {
        let config = binary_fast_hot_stats_config();
        let mut s = make_solver_with_config(2, vec![], &config);
        let clause_idx = s.add_clause(vec![1, 2]);
        let binary_id = s.binary_id_for_clause(clause_idx);

        s.delete_clause(clause_idx);
        assert!(s.binary_clause_is_deleted(binary_id));
        assert_eq!(s.binary_implications.len_for(-1), 1);

        assert!(s.enqueue(-1, ReasonRef::None));
        assert!(s.propagate().is_none());
        assert_eq!(s.assignment[2], UNASSIGNED);
        assert_eq!(s.stats.binary_stale_skips, 1);
    }

    #[test]
    fn test_binary_dedup_prevents_duplicate_hbr_edge() {
        let config = binary_fast_config();
        let mut s = make_solver_with_config(2, vec![], &config);
        let _first = s.add_clause(vec![1, 2]);
        let before_edges = s.binary_implications.len_for(-1);

        assert!(s.generated_binary_pair_is_duplicate_for_test(2, 1));
        if !s.generated_binary_pair_is_duplicate_for_test(2, 1) {
            s.add_clause(vec![2, 1]);
        }
        assert_eq!(s.binary_implications.len_for(-1), before_edges);
    }

    #[test]
    fn test_generated_redundant_binary_deleted_through_formula_edit() {
        let config = binary_fast_config();
        let mut s = make_solver_with_config(2, vec![], &config);
        let clause_idx = s.add_clause(vec![1, 2]);
        let binary_id = s.binary_id_for_clause(clause_idx);

        s.delete_clause(clause_idx);

        assert!(s.binary_clauses[binary_id.0 as usize].deleted);
        assert!(s.clause_is_deleted(clause_idx));
    }

    #[test]
    fn test_original_binary_not_deleted_by_binary_budget() {
        let config = binary_fast_config();
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        let clause_idx = s.original_clause_ids[0];

        s.reduce_db();

        assert!(!s.clause_is_deleted(clause_idx));
        assert!(!s.binary_clause_is_deleted(s.binary_id_for_clause(clause_idx)));
    }

    #[test]
    fn test_binary_usage_counter_updates_on_reason() {
        let config = binary_fast_config();
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        let binary_id = s.binary_id_for_clause(s.original_clause_ids[0]);

        assert!(s.enqueue(-1, ReasonRef::None));
        assert!(s.propagate().is_none());

        assert_eq!(s.binary_clauses[binary_id.0 as usize].used_count, 1);
        assert_eq!(s.binary_clauses[binary_id.0 as usize].last_used_conflict, 0);
    }

    #[test]
    fn test_reason_code_roundtrip_clause() {
        let code = ReasonCode::from_ref(ReasonRef::Clause(123)).expect("valid clause reason");

        assert_eq!(code.as_ref(), Ok(ReasonRef::Clause(123)));
    }

    #[test]
    fn test_reason_code_roundtrip_binary() {
        let binary_id = BinaryClauseId(7);
        let code = ReasonCode::from_ref(ReasonRef::Binary(binary_id)).expect("valid binary reason");

        assert_eq!(code.as_ref(), Ok(ReasonRef::Binary(binary_id)));
    }

    #[test]
    fn test_reason_code_rejects_invalid_tag_or_overflow() {
        let invalid_tag = ReasonCode::from_raw(ReasonCode::INVALID_TAG | 17);
        assert_eq!(invalid_tag.as_ref(), Err(ReasonCodeError::InvalidTag));
        assert_eq!(
            ReasonCode::from_ref(ReasonRef::Clause(ReasonCode::PAYLOAD_MASK + 1)),
            Err(ReasonCodeError::ClauseOverflow)
        );
        if usize::BITS > u32::BITS {
            let too_large_binary =
                ReasonCode::from_raw(ReasonCode::BINARY_TAG | ((u32::MAX as usize) + 1));
            assert_eq!(
                too_large_binary.as_ref(),
                Err(ReasonCodeError::BinaryOverflow)
            );
        }
    }

    #[test]
    fn test_gc_rewrites_reason_ref() {
        let mut s = make_solver(4, vec![]);
        let dead = s.add_clause(vec![4, 1]);
        let live = s.add_clause(vec![3, 1]);

        s.assignment[3] = TRUE;
        s.saved_phase[3] = TRUE;
        s.decision_level[3] = 1;
        s.set_reason_ref(3, ReasonRef::Clause(live));
        s.trail.push(3);
        s.trail_limits.push(0);

        s.mark_clause_deleted(dead);
        s.garbage_collect();

        let relocated_live = s.learned_clause_ids[0];
        assert_eq!(s.reason_ref(3), ReasonRef::Clause(relocated_live));
        assert_eq!(s.clause_slice(s.reason_clause_for_test(3)), &[3, 1]);
    }

    #[test]
    fn test_gc_not_run_above_root_level() {
        let mut s = make_solver(6, vec![]);
        let dead = s.add_clause(vec![1, 2, 3, 4, 5]);
        let live = s.add_clause(vec![1, 2]);
        s.mark_clause_deleted(dead);
        s.decide(6);

        let arena_words_before = s.arena.len();
        let deleted_words_before = s.deleted_clause_words;

        assert!(!s.maybe_garbage_collect(GcReason::LearnedReduction));
        assert_eq!(s.stats.garbage_collections, 0);
        assert_eq!(s.arena.len(), arena_words_before);
        assert_eq!(s.deleted_clause_words, deleted_words_before);
        assert_eq!(s.learned_clause_ids, vec![live]);
        assert_eq!(s.gc_pending_reason, GcReason::LearnedReduction);
    }

    #[test]
    fn test_gc_deferred_above_root_runs_at_next_root_safe_point() {
        let mut s = make_solver(6, vec![]);
        let dead = s.add_clause(vec![1, 2, 3, 4, 5]);
        let live = s.add_clause(vec![1, 2]);
        s.mark_clause_deleted(dead);
        s.decide(6);

        assert!(!s.maybe_garbage_collect(GcReason::LearnedReduction));
        s.backtrack(0);

        assert!(s.maybe_garbage_collect(GcReason::ArenaFragmentation));
        assert_eq!(s.stats.gc_last_reason, GcReason::LearnedReduction);
        assert_eq!(s.gc_pending_reason, GcReason::None);
        assert_eq!(s.deleted_clause_words, 0);
        assert_eq!(s.clause_slice(s.learned_clause_ids[0]), &[1, 2]);
        assert_ne!(s.learned_clause_ids[0], live);
    }

    #[test]
    fn test_gc_reclaims_deleted_learned_words() {
        let mut s = make_solver(6, vec![]);
        let dead = s.add_clause(vec![1, 2, 3, 4, 5]);
        let live = s.add_clause(vec![1, 2]);
        let dead_words = s.clause_word_len(dead);
        s.mark_clause_deleted(dead);

        let arena_words_before = s.arena.len();

        assert!(s.maybe_garbage_collect(GcReason::LearnedReduction));
        assert!(s.arena.len() < arena_words_before);
        assert_eq!(s.deleted_clause_words, 0);
        assert_eq!(s.learned_clause_ids.len(), 1);
        assert_eq!(s.clause_slice(s.learned_clause_ids[0]), &[1, 2]);
        assert!(s.stats.gc_words_reclaimed >= dead_words as u64);
        assert_eq!(s.stats.gc_last_reason, GcReason::LearnedReduction);
        assert_eq!(s.live_learned_clause_count, 1);
        assert_eq!(s.learned_literals, s.clause_len(s.learned_clause_ids[0]));
        assert_ne!(s.learned_clause_ids[0], live);
    }

    #[test]
    fn test_gc_rewrites_all_registered_refs() {
        let mut s = make_solver(4, vec![]);
        s.track_gc_detail_stats = true;
        enable_lbd_tiered_for_test(&mut s, 16);
        let dead = s.add_clause(vec![4, 1, 2]);
        s.add_raw_initial_original_clauses(vec![vec![1], vec![2, 3]]);
        let live = s.add_clause(vec![4, 2, 3]);
        set_lbd_meta_for_test(&mut s, live, 4, 1);

        s.set_reason_ref(4, ReasonRef::Clause(live));
        s.assignment[4] = TRUE;
        s.mark_clause_deleted(dead);

        let old_reason = s.reason_ref(4);
        let old_root = s.root_unit_clauses[0];

        s.garbage_collect();

        let relocated_live = s.learned_clause_ids[0];
        let relocated_root = s.root_unit_clauses[0];
        assert_ne!(s.reason_ref(4), old_reason);
        assert_eq!(s.reason_ref(4), ReasonRef::Clause(relocated_live));
        assert_ne!(relocated_root, old_root);
        assert_eq!(s.clause_slice(relocated_root), &[1]);
        assert_eq!(s.learnt_lbd(relocated_live), 4);
        assert_eq!(s.learnt_used_recently(relocated_live), 1);
        for watch_list in &s.watchers {
            for watcher in watch_list {
                let clause_idx = watcher.clause_idx as usize;
                assert!(clause_idx < s.arena.len());
                assert!(!s.clause_is_deleted(clause_idx));
            }
        }
        assert!(s.stats.gc_refs_rewritten >= 3);
    }

    #[test]
    fn test_gc_preserves_original_clause_model_check_refs() {
        let mut s = make_solver(3, vec![vec![1, 2], vec![-1, 3]]);
        let dead = s.add_clause(vec![1, 2, 3]);
        s.mark_clause_deleted(dead);

        s.garbage_collect();

        let original_clauses = live_original_clauses(&s);
        let mut model = vec![UNASSIGNED; 4];
        model[1] = TRUE;
        model[2] = TRUE;
        model[3] = TRUE;

        assert_eq!(original_clauses, vec![vec![-1, 3], vec![1, 2]]);
        assert!(verify_model_against_clauses(&original_clauses, &model));
    }

    #[test]
    fn test_gc_reason_recorded_in_stats() {
        let mut s = make_solver(6, vec![]);
        let dead = s.add_clause(vec![1, 2, 3, 4, 5]);
        let dead_words = s.clause_word_len(dead);
        let _live = s.add_clause(vec![1, 2]);
        s.mark_clause_deleted(dead);

        assert!(s.maybe_garbage_collect(GcReason::EmergencyMemory));
        assert_eq!(s.stats.garbage_collections, 1);
        assert_eq!(s.stats.gc_last_reason, GcReason::EmergencyMemory);
        assert!(s.stats.gc_words_reclaimed >= dead_words as u64);
    }

    #[test]
    fn test_gc_watcher_staleness_reason_after_skip_pressure() {
        let mut s = make_solver(1, vec![]);
        let watch_idx = s.lit_index(1);
        s.watchers[watch_idx].resize(
            GC_WATCHER_STALE_MIN,
            Watcher {
                clause_idx: u32::MAX,
                blocker: 1,
            },
        );
        s.stats.watch_stale_skips = GC_WATCHER_STALE_MIN as u64;

        assert!(s.maybe_garbage_collect(GcReason::LearnedReduction));

        assert_eq!(s.stats.gc_last_reason, GcReason::WatcherStaleness);
        assert_eq!(s.stats.garbage_collections, 1);
        assert!(s.watchers[watch_idx].is_empty());
    }

    #[test]
    fn test_legacy_reason_path_unchanged_when_binary_fast_off() {
        let mut s = make_solver(2, vec![vec![2, 1]]);

        s.decide(-1);
        assert_eq!(s.propagate(), None);

        let clause_idx = s.original_clause_ids[0];
        assert_eq!(s.reason_ref(2), ReasonRef::Clause(clause_idx));
        assert!(s.binary_reason_lits.is_empty());
        assert_eq!(s.reason_lits_for_test(s.reason_ref(2)), vec![2, 1]);
    }

    #[test]
    fn test_temp_assumption_guard_restores_root() {
        let mut s = make_solver(2, vec![]);
        let start_trail = s.trail.len();
        let start_root = s.root_trail_len;

        s.with_temporary_assumptions(TemporaryAssumptionOptions::default(), |ctx| {
            assert_eq!(ctx.enqueue(1), EnqueueResult::Enqueued);
            assert_eq!(ctx.solver.assignment[1], TRUE);
            assert_eq!(ctx.solver.root_trail_len, start_root);
        });

        assert_eq!(s.current_level(), 0);
        assert_eq!(s.trail.len(), start_trail);
        assert_eq!(s.root_trail_len, start_root);
        assert_eq!(s.assignment[1], UNASSIGNED);
        assert_eq!(s.reason_ref(1), ReasonRef::None);
    }

    #[test]
    fn test_temp_assumption_does_not_update_saved_phase() {
        let mut s = make_solver(1, vec![]);
        s.saved_phase[1] = TRUE;

        s.with_temporary_assumptions(TemporaryAssumptionOptions::default(), |ctx| {
            assert_eq!(ctx.enqueue(-1), EnqueueResult::Enqueued);
            assert_eq!(ctx.solver.assignment[1], FALSE);
            assert_eq!(ctx.solver.saved_phase[1], TRUE);
        });

        assert_eq!(s.saved_phase[1], TRUE);
        assert_eq!(s.assignment[1], UNASSIGNED);
    }

    #[test]
    fn test_temp_assumption_does_not_update_target_or_best_phase() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::BestThenTargetThenSaved,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(1, vec![], &config);
        let phase_initial = s.stats.phase_initial_used;
        let saved_phase = s.saved_phase[1];
        let target_phase = s.target_phase[1];
        let best_phase = s.best_phase[1];
        let target_assigned = s.target_assigned;
        let best_assigned = s.best_assigned;
        let phase_ticks = s.phase_ticks;

        s.with_temporary_assumptions(TemporaryAssumptionOptions::default(), |ctx| {
            assert_eq!(ctx.enqueue(-1), EnqueueResult::Enqueued);
            ctx.solver.maybe_capture_phase_prefix();
        });

        assert_eq!(s.stats.phase_initial_used, phase_initial);
        assert_eq!(s.saved_phase[1], saved_phase);
        assert_eq!(s.target_phase[1], target_phase);
        assert_eq!(s.best_phase[1], best_phase);
        assert_eq!(s.target_assigned, target_assigned);
        assert_eq!(s.best_assigned, best_assigned);
        assert_eq!(s.phase_ticks, phase_ticks);
    }

    #[test]
    fn test_temp_assumption_does_not_bump_vmtf_or_heap_stats() {
        let config = focused_stable_vmtf_config();
        let mut s = make_solver_with_config(2, vec![], &config);
        let heap_inserts = s.stats.decision_heap_inserts;
        let heap_pops = s.stats.decision_heap_pops;
        let heap_stale = s.stats.decision_heap_stale_pops;
        let vmtf_stamp = s.vmtf_queue.as_ref().unwrap().stamp_for_test(1);

        s.with_temporary_assumptions(TemporaryAssumptionOptions::default(), |ctx| {
            assert_eq!(ctx.enqueue(1), EnqueueResult::Enqueued);
            ctx.solver.scratch_bumped_vars.push(1);
            ctx.solver.bump_analyzed_variable_activity();
        });

        assert_eq!(s.stats.decision_heap_inserts, heap_inserts);
        assert_eq!(s.stats.decision_heap_pops, heap_pops);
        assert_eq!(s.stats.decision_heap_stale_pops, heap_stale);
        assert_eq!(s.vmtf_queue.as_ref().unwrap().stamp_for_test(1), vmtf_stamp);
    }

    #[test]
    fn test_temp_assumption_does_not_update_restart_ema() {
        let config = SolverConfig {
            use_lbd: true,
            restart_policy: RestartPolicy::KissatEma,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(2, vec![], &config);
        let restart_conflicts = s.restart_conflicts;
        let restart_limit = s.restart_conflict_limit;
        let restart_conflicts_since_last = s.restart_conflicts_since_last;
        let fast_lbd = s.restart_fast_lbd.value;
        let slow_lbd = s.restart_slow_lbd.value;
        let restarts = s.stats.restarts;
        let luby_restarts = s.stats.luby_restarts;
        let glucose_restarts = s.stats.glucose_restarts;
        let focused_restarts = s.stats.focused_restarts;

        s.with_temporary_assumptions(TemporaryAssumptionOptions::default(), |ctx| {
            assert_eq!(ctx.enqueue(1), EnqueueResult::Enqueued);
            ctx.solver.last_conflict_lbd = 4;
            ctx.solver.note_conflict();
        });

        assert_eq!(s.restart_conflicts, restart_conflicts);
        assert_eq!(s.restart_conflict_limit, restart_limit);
        assert_eq!(s.restart_conflicts_since_last, restart_conflicts_since_last);
        assert_eq!(s.restart_fast_lbd.value, fast_lbd);
        assert_eq!(s.restart_slow_lbd.value, slow_lbd);
        assert_eq!(s.stats.restarts, restarts);
        assert_eq!(s.stats.luby_restarts, luby_restarts);
        assert_eq!(s.stats.glucose_restarts, glucose_restarts);
        assert_eq!(s.stats.focused_restarts, focused_restarts);
    }

    #[test]
    fn test_temp_assumption_conflict_restores_propagate_head() {
        let mut s = make_solver(2, vec![vec![2, 1], vec![-2, 1]]);
        let start_head = s.propagate_head;
        let start_normal_props = s.stats.propagations;
        let mut saw_conflict = false;

        s.with_temporary_assumptions(TemporaryAssumptionOptions::default(), |ctx| {
            assert_eq!(ctx.enqueue(-1), EnqueueResult::Enqueued);
            let mut budget = Budget::from_ticks(10);
            saw_conflict = ctx.propagate_budgeted(&mut budget).is_some();
            assert!(saw_conflict);
        });

        assert_eq!(s.propagate_head, start_head);
        assert_eq!(s.stats.propagations, start_normal_props);
        assert_eq!(s.temporary_stats.conflicts, 1);
        assert_eq!(s.assignment[1], UNASSIGNED);
        assert_eq!(s.assignment[2], UNASSIGNED);
    }

    #[test]
    fn test_temp_assumption_propagation_does_not_update_reason_lbd() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.update_reason_lbd = true;
        s.update_propagation_reason_lbd = true;
        let reason = s.add_clause(vec![2, 1, 3]);
        set_lbd_meta_for_test(&mut s, reason, 9, 0);

        s.with_temporary_assumptions(TemporaryAssumptionOptions::default(), |ctx| {
            assert_eq!(ctx.enqueue(-3), EnqueueResult::Enqueued);
            assert_eq!(ctx.enqueue(-1), EnqueueResult::Enqueued);
            let mut budget = Budget::from_ticks(10);
            assert_eq!(ctx.propagate_budgeted(&mut budget), None);
            assert_eq!(ctx.solver.assignment[2], TRUE);
            assert_eq!(ctx.solver.reason_ref(2), ReasonRef::Clause(reason));
        });

        assert_eq!(s.assignment[2], UNASSIGNED);
        assert_eq!(s.reason_ref(2), ReasonRef::None);
        assert_eq!(s.learnt_lbd(reason), 9);
        assert_eq!(s.learnt_used_recently(reason), 0);
        assert_eq!(s.stats.lbd_improved, 0);
    }

    #[test]
    fn test_temp_assumption_closure_restores_on_early_return() {
        let mut s = make_solver(2, vec![]);
        let start_head = s.propagate_head;

        let result = s.with_temporary_assumptions(TemporaryAssumptionOptions::default(), |ctx| {
            assert_eq!(ctx.enqueue(1), EnqueueResult::Enqueued);
            "early-return-value"
        });

        assert_eq!(result, "early-return-value");
        assert_eq!(s.trail.len(), 0);
        assert_eq!(s.propagate_head, start_head);
        assert_eq!(s.accounting_mode, SearchAccountingMode::NormalSearch);
        assert_eq!(s.assignment[1], UNASSIGNED);
    }

    #[test]
    fn test_lbd_stored_and_read_from_learned_clause() {
        let mut s = Solver::new(5, vec![]);
        s.use_lbd = true;
        s.decision_level[1] = 1;
        s.decision_level[2] = 1;
        s.decision_level[3] = 2;
        s.decision_level[4] = 2;
        s.decision_level[5] = 3;

        let clause_idx = s.add_clause(vec![1, -2, 3, -4, 5]);

        assert_eq!(s.learned_clause_lbd(clause_idx), Some(3));
        assert_eq!(s.stats.lbd_computed, 1);
        assert_eq!(s.stats.lbd_sum, 3);
        assert_eq!(s.stats.lbd_max, 3);
        assert_eq!(s.sum_lbd, 3);
        assert_eq!(s.num_lbd, 1);
        assert_eq!(s.lbd_hist_3_5, 1);
    }

    #[test]
    fn test_learned_clause_initial_used_recently_is_max_for_all_lbd_tiers() {
        let mut s = Solver::new(9, vec![]);
        s.use_lbd = true;
        for var in 1..=9 {
            s.decision_level[var] = var;
        }

        let tier1 = s.add_clause(vec![1, 2]);
        let tier2 = s.add_clause(vec![1, 2, 3, 4]);
        let tier3 = s.add_clause(vec![1, 2, 3, 4, 5, 6, 7]);

        assert_eq!(s.learned_meta(tier1).unwrap().tier, 0);
        assert_eq!(s.learned_meta(tier2).unwrap().tier, 1);
        assert_eq!(s.learned_meta(tier3).unwrap().tier, 2);
        assert_eq!(s.learnt_used_recently(tier1), MAX_USED_RECENTLY);
        assert_eq!(s.learnt_used_recently(tier2), MAX_USED_RECENTLY);
        assert_eq!(s.learnt_used_recently(tier3), MAX_USED_RECENTLY);
    }

    #[test]
    fn test_lbd_side_table_records_empty_learned_clause_metadata() {
        let mut s = Solver::new(1, vec![]);
        s.use_lbd = true;

        let clause_idx = s.add_clause(vec![]);

        assert_eq!(s.learned_clause_lbd(clause_idx), Some(0));
        assert_eq!(s.stats.lbd_computed, 1);
        assert_eq!(s.stats.lbd_sum, 0);
        assert_eq!(s.stats.lbd_max, 0);
    }

    #[test]
    fn test_lbd_metadata_does_not_touch_original_clause_layout() {
        let mut s = Solver::new(2, vec![vec![1, 2]]);
        let original_idx = s.original_clause_ids[0];
        let original_header = s.clause_header(original_idx);
        let original_word_len = s.clause_word_len(original_idx);
        s.use_lbd = true;
        s.decision_level[1] = 1;
        s.decision_level[2] = 2;

        let learned_idx = s.add_clause(vec![1, -2]);

        assert_eq!(s.clause_header(original_idx), original_header);
        assert_eq!(s.clause_word_len(original_idx), original_word_len);
        assert!(s.try_learned_id_for_clause(original_idx).is_none());
        assert_eq!(s.learned_clause_lbd(learned_idx), Some(2));
    }

    #[test]
    fn test_lbd_side_table_remaps_across_garbage_collection() {
        let mut s = Solver::new(5, vec![]);
        s.use_lbd = true;
        s.decision_level[1] = 1;
        s.decision_level[2] = 2;
        s.decision_level[3] = 3;
        let dead = s.add_clause(vec![1, 2]);
        let live = s.add_clause(vec![-1, 2, 3]);

        assert_eq!(s.learned_clause_lbd(dead), Some(2));
        assert_eq!(s.learned_clause_lbd(live), Some(3));

        s.mark_clause_deleted(dead);
        s.garbage_collect();

        let relocated_live = s.learned_clause_ids[0];
        assert_eq!(s.learned_clause_lbd(relocated_live), Some(3));
        assert_eq!(
            s.learned_clause_by_id[s.learned_id_for_clause(relocated_live).0 as usize],
            relocated_live
        );
    }

    #[test]
    fn test_lbd_improvement_only_lowers() {
        let mut s = Solver::new(3, vec![]);
        s.use_lbd = true;
        let clause_idx = s.add_clause(vec![1, 2, 3]);
        s.set_learnt_lbd(clause_idx, 4);

        s.maybe_improve_lbd(clause_idx, 5);
        assert_eq!(s.learnt_lbd(clause_idx), 4);
        assert_eq!(s.stats.lbd_improved, 0);

        s.maybe_improve_lbd(clause_idx, 2);
        assert_eq!(s.learnt_lbd(clause_idx), 2);
        assert_eq!(s.stats.lbd_improved, 1);
    }

    #[test]
    fn test_original_clause_lbd_not_touched() {
        let mut s = Solver::new(2, vec![vec![1, 2]]);
        s.use_lbd = true;
        let original_idx = s.original_clause_ids[0];

        s.maybe_improve_lbd(original_idx, 1);

        assert!(s.try_learned_id_for_clause(original_idx).is_none());
        assert_eq!(s.stats.lbd_improved, 0);
    }

    #[test]
    fn test_reason_clause_lbd_update_preserves_activity() {
        let mut s = Solver::new(3, vec![]);
        s.use_lbd = true;
        let clause_idx = s.add_clause(vec![1, 2, 3]);
        s.set_learnt_lbd(clause_idx, 4);
        s.set_clause_activity(clause_idx, 123.5);

        s.maybe_improve_lbd(clause_idx, 2);

        assert_eq!(s.learnt_lbd(clause_idx), 2);
        assert_eq!(s.clause_activity(clause_idx), 123.5);
    }

    #[test]
    fn test_analyze_stores_last_conflict_lbd() {
        let mut s = make_solver(2, vec![vec![2, 1], vec![-2, 1]]);
        s.use_lbd = true;

        s.decide(-1);
        let conflict = s.propagate().expect("expected conflict");
        let (learned, backtrack_level) = s.analyze_conflict(conflict);

        assert_eq!(learned, vec![1]);
        assert_eq!(backtrack_level, 0);
        assert_eq!(s.last_conflict_lbd, 1);
        assert_eq!(s.stats.lbd_computed, 1);
        assert_eq!(s.num_lbd, 1);
    }

    fn enable_lbd_tiered_for_test(s: &mut Solver, learned_lit_budget: usize) {
        s.use_lbd = true;
        s.reduce_policy = ReducePolicy::LbdTiered;
        s.learned_lit_budget = learned_lit_budget;
        s.hard_learned_lit_budget = learned_lit_budget.saturating_mul(2);
    }

    fn set_lbd_meta_for_test(s: &mut Solver, clause_idx: ClauseRef, lbd: u16, used_recently: u8) {
        s.set_learnt_lbd(clause_idx, lbd);
        s.classify_learnt_clause(clause_idx);
        s.set_learnt_used_recently(clause_idx, used_recently);
    }

    fn record_glue_uses_for_test(s: &mut Solver, glue: u16, count: usize) {
        for _ in 0..count {
            s.record_current_mode_glue_use(glue);
        }
    }

    #[test]
    fn test_glue_histogram_records_learned_and_reason_uses_by_mode() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.search_mode = SearchMode::Focused;
        s.last_conflict_lbd = 8;

        let learned = s.add_analyzed_clause_from_slice(&[1, 2, 3]);
        s.mark_learned_clause_recent(learned);

        assert_eq!(s.stats.focused_glue_used[8], 2);
        assert!(s.stats.stable_glue_used.is_empty());

        s.search_mode = SearchMode::Stable;
        s.mark_learned_clause_recent(learned);

        assert_eq!(s.stats.focused_glue_used[8], 2);
        assert_eq!(s.stats.stable_glue_used[8], 1);
    }

    #[test]
    fn test_dynamic_tier_limits_use_current_mode_histogram() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.search_mode = SearchMode::Stable;
        record_glue_uses_for_test(&mut s, 4, 5);
        record_glue_uses_for_test(&mut s, 8, 4);
        record_glue_uses_for_test(&mut s, 20, 1);

        s.retier_current_mode_from_glue_histogram();

        assert_eq!(
            s.stable_tier_limits,
            TierLimits {
                tier1_max_glue: 4,
                tier2_max_glue: 8,
            }
        );
        assert_eq!(s.stats.stable_tier1_glue_limit, 4);
        assert_eq!(s.stats.stable_tier2_glue_limit, 8);
        assert!(s.stable_glue_recent.is_empty());
        assert_eq!(s.stats.stable_glue_used[4], 5);
        assert_eq!(s.stats.stable_glue_used[8], 4);
        assert_eq!(s.stats.stable_glue_used[20], 1);
    }

    #[test]
    fn test_dynamic_tier_limits_reclassify_live_learned_clauses() {
        let mut s = Solver::new(6, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.search_mode = SearchMode::Stable;
        let glue4 = s.add_clause(vec![1, 2, 3]);
        let glue8 = s.add_clause(vec![1, 2, 3, 4]);
        let glue20 = s.add_clause(vec![1, 2, 3, 4, 5]);
        set_lbd_meta_for_test(&mut s, glue4, 4, 0);
        set_lbd_meta_for_test(&mut s, glue8, 8, 0);
        set_lbd_meta_for_test(&mut s, glue20, 20, 0);
        assert_eq!(s.learned_meta(glue4).unwrap().tier, 1);
        assert_eq!(s.learned_meta(glue8).unwrap().tier, 2);

        record_glue_uses_for_test(&mut s, 4, 5);
        record_glue_uses_for_test(&mut s, 8, 4);
        record_glue_uses_for_test(&mut s, 20, 1);
        s.retier_current_mode_from_glue_histogram();

        assert_eq!(s.learned_meta(glue4).unwrap().tier, 0);
        assert_eq!(s.learned_meta(glue8).unwrap().tier, 1);
        assert_eq!(s.learned_meta(glue20).unwrap().tier, 2);
    }

    #[test]
    fn test_reduce_db_ages_all_scanned_learned_clauses_after_protecting_this_pass() {
        let mut s = Solver::new(6, vec![]);
        enable_lbd_tiered_for_test(&mut s, 0);
        s.hard_learned_lit_budget = 0;
        let tier1 = s.add_clause(vec![1, 2, 3]);
        let tier2 = s.add_clause(vec![1, 2, 3, 4]);
        let tier3 = s.add_clause(vec![1, 2, 3, 4, 5]);
        set_lbd_meta_for_test(&mut s, tier1, 1, 2);
        set_lbd_meta_for_test(&mut s, tier2, 4, 2);
        set_lbd_meta_for_test(&mut s, tier3, 9, 2);

        s.reduce_db();

        assert!(!s.clause_is_deleted(tier1));
        assert!(!s.clause_is_deleted(tier2));
        assert!(!s.clause_is_deleted(tier3));
        assert_eq!(s.learnt_used_recently(tier1), 1);
        assert_eq!(s.learnt_used_recently(tier2), 1);
        assert_eq!(s.learnt_used_recently(tier3), 1);
    }

    #[test]
    fn test_reduce_candidate_activity_rank_is_integer_monotonic_for_positive_activity() {
        let mut s = Solver::new(4, vec![]);
        s.use_lbd = true;
        let older = s.add_clause(vec![1, 2, 3]);
        let newer = s.add_clause(vec![1, 2, 4]);
        s.set_clause_activity(older, 1.0);
        s.set_clause_activity(newer, 2.0);

        assert!(s.reduce_candidate_activity_rank(older) < s.reduce_candidate_activity_rank(newer));
    }

    #[test]
    fn test_lbd_tiered_reduce_uses_conflict_limit_not_clause_or_soft_lit_budget() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.reduce_db_limit = 1_000;
        s.stats.conflicts = 999;
        s.live_learned_clause_count = 10_000;
        s.learned_literals = 11;

        assert!(!s.should_reduce_db());

        s.stats.conflicts = 1_000;

        assert!(s.should_reduce_db());
    }

    #[test]
    fn test_lbd_tiered_reduce_hard_lit_budget_is_emergency_trigger() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.reduce_db_limit = 1_000;
        s.stats.conflicts = 0;
        s.learned_literals = 21;

        assert!(s.should_reduce_db());
    }

    #[test]
    fn test_lbd_tiered_reduce_min_interval_blocks_hard_budget_repeats() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.reduce_db_limit = 1_000;
        s.reduce_db_min_interval = 100;
        s.reduce_db_last_conflicts = Some(1_000);
        s.stats.conflicts = 1_050;
        s.learned_literals = 21;

        assert!(!s.should_reduce_db());

        s.stats.conflicts = 1_100;

        assert!(s.should_reduce_db());
    }

    #[test]
    fn test_reduce_db_records_last_conflict_for_interval_guard() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.stats.conflicts = 42;

        s.reduce_db();

        assert_eq!(s.reduce_db_last_conflicts, Some(42));
    }

    #[test]
    fn test_lbd_tiered_conflict_accounting_does_not_mutate_reduce_limit() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.reduce_db_limit = 1_000;
        s.learntsize_adjust_cnt = 1;
        s.learntsize_adjust_confl = 1.0;

        s.note_learnt_budget_conflict();

        assert_eq!(s.reduce_db_limit, 1_000);
        assert_eq!(s.learntsize_adjust_cnt, 1);
    }

    #[test]
    fn test_lbd_tiered_reduce_reschedules_with_sqrt_conflict_interval() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.reduce_db_limit = 1_000;
        s.learntsize_adjust_confl = 300.0;
        s.stats.conflicts = 1_500;
        s.stats.reduce_db_calls = 4;

        s.schedule_next_lbd_reduce_db();

        assert_eq!(s.reduce_db_limit, 2_100);
        assert_eq!(s.learned_lit_budget, 2_600);
        assert_eq!(
            s.hard_learned_lit_budget,
            2_600usize.saturating_mul(LBD_HARD_LEARNED_LIT_BUDGET_FACTOR)
        );
    }

    #[test]
    fn test_lbd_tiered_config_uses_kissat_style_reduce_schedule_defaults() {
        let config = SolverConfig {
            use_lbd: true,
            reduce_policy: ReducePolicy::LbdTiered,
            ..SolverConfig::default()
        };

        let s = make_solver_with_config(3, vec![], &config);

        assert_eq!(s.reduce_db_limit, LBD_REDUCE_DB_INIT_CONFLICTS);
        assert_eq!(
            s.reduce_db_min_interval,
            LBD_REDUCE_DB_MIN_INTERVAL_CONFLICTS
        );
        assert_eq!(s.learntsize_adjust_cnt, LBD_REDUCE_DB_INTERVAL_CONFLICTS);
        assert_eq!(
            s.learntsize_adjust_confl,
            LBD_REDUCE_DB_INTERVAL_CONFLICTS as f64
        );
        assert!(!s.reset_reduce_db_after_preprocess);
        assert_eq!(s.learned_lit_budget, LEARNED_LIT_BUDGET_BASE);
        assert_eq!(
            s.hard_learned_lit_budget,
            LEARNED_LIT_BUDGET_BASE.saturating_mul(LBD_HARD_LEARNED_LIT_BUDGET_FACTOR)
        );
    }

    #[test]
    fn test_lbd_tiered_config_honors_reduce_db_schedule_overrides() {
        let config = SolverConfig {
            use_lbd: true,
            reduce_policy: ReducePolicy::LbdTiered,
            reduce_db_init: Some(50),
            reduce_db_interval: Some(25),
            ..SolverConfig::default()
        };

        let s = make_solver_with_config(3, vec![], &config);

        assert_eq!(s.reduce_db_limit, 50);
        assert_eq!(s.reduce_db_min_interval, 100);
        assert_eq!(s.learntsize_adjust_cnt, 25);
        assert_eq!(s.learntsize_adjust_confl, 25.0);
    }

    #[test]
    fn test_lbd_tiered_config_honors_reduce_min_interval_override() {
        let config = SolverConfig {
            use_lbd: true,
            reduce_policy: ReducePolicy::LbdTiered,
            reduce_min_interval: Some(250),
            ..SolverConfig::default()
        };

        let s = make_solver_with_config(3, vec![], &config);

        assert_eq!(s.reduce_db_min_interval, 250);
    }

    #[test]
    fn test_lbd_tiered_hard_budget_scales_with_formula_size() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.original_literals = 1_000_000;

        s.refresh_learned_lit_budgets();

        assert_eq!(s.learned_lit_budget, LEARNED_LIT_BUDGET_BASE);
        assert_eq!(
            s.hard_learned_lit_budget,
            1_000_000usize.saturating_mul(LBD_HARD_LEARNED_LIT_FORMULA_FACTOR)
        );
    }

    #[test]
    fn test_lbd_tiered_preprocess_refresh_preserves_default_schedule() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.reduce_db_limit = 1_000;
        s.reset_reduce_db_after_preprocess = false;
        s.stats.conflicts = 123;
        s.original_literals = 1_000_000;

        s.reset_learned_budget_after_preprocess();

        assert_eq!(s.reduce_db_limit, 1_000);
        assert_eq!(
            s.hard_learned_lit_budget,
            1_000_000usize.saturating_mul(LBD_HARD_LEARNED_LIT_FORMULA_FACTOR)
        );
    }

    #[test]
    fn test_reason_pinset_contains_all_clause_reasons() {
        let mut s = make_solver(3, vec![]);
        let reason_clause = s.add_clause(vec![2, 1, 3]);
        s.assignment[1] = TRUE;
        s.set_reason_ref(1, ReasonRef::Clause(reason_clause));

        let pins = s.rebuild_reason_pinset();

        assert!(s.clause_is_reason_pinned(&pins, reason_clause));
    }

    #[test]
    fn test_reason_pinset_contains_all_binary_reasons() {
        let mut s = make_solver(3, vec![]);
        let binary_id = s.add_binary_reason_for_test([2, -1]);
        s.assignment[2] = TRUE;
        s.set_reason_ref(2, ReasonRef::Binary(binary_id));

        let pins = s.rebuild_reason_pinset();

        assert!(s.binary_is_reason_pinned(&pins, binary_id));
    }

    #[test]
    fn test_reduce_db_consults_reason_pinset() {
        let mut s = Solver::new(4, vec![]);
        enable_lbd_tiered_for_test(&mut s, 0);
        let pinned = s.add_clause(vec![4, 1, 2, 3]);
        set_lbd_meta_for_test(&mut s, pinned, 12, 0);
        s.assignment[1] = TRUE;
        s.set_reason_ref(1, ReasonRef::Clause(pinned));

        s.reduce_db();

        assert!(!s.clause_is_deleted(pinned));
    }

    #[test]
    fn test_gc_preserves_reason_pinned_clauses() {
        let mut s = Solver::new(4, vec![]);
        enable_lbd_tiered_for_test(&mut s, 0);
        let dead = s.add_clause(vec![4, 1, 2]);
        let pinned = s.add_clause(vec![3, 1, 2]);
        s.assignment[1] = TRUE;
        s.set_reason_ref(1, ReasonRef::Clause(pinned));
        s.mark_clause_deleted(dead);

        s.garbage_collect();

        let relocated = s.reason_clause_for_test(1);
        assert_eq!(s.clause_slice(relocated), &[3, 1, 2]);
        assert!(!s.clause_is_deleted(relocated));
    }

    #[test]
    fn test_reduce_never_deletes_binary() {
        let mut s = Solver::new(2, vec![]);
        enable_lbd_tiered_for_test(&mut s, 0);
        let binary = s.add_clause(vec![1, 2]);
        set_lbd_meta_for_test(&mut s, binary, 20, 0);

        s.reduce_db();

        assert!(!s.clause_is_deleted(binary));
    }

    #[test]
    fn test_reduce_never_deletes_unit() {
        let mut s = Solver::new(1, vec![]);
        enable_lbd_tiered_for_test(&mut s, 0);
        let unit = s.add_clause(vec![1]);
        set_lbd_meta_for_test(&mut s, unit, 20, 0);

        s.reduce_db();

        assert!(!s.clause_is_deleted(unit));
    }

    #[test]
    fn test_reduce_never_deletes_reason_clause() {
        let mut s = Solver::new(4, vec![]);
        enable_lbd_tiered_for_test(&mut s, 0);
        let reason = s.add_clause(vec![4, 1, 2, 3]);
        set_lbd_meta_for_test(&mut s, reason, 20, 0);
        s.assignment[4] = TRUE;
        s.set_reason_ref(4, ReasonRef::Clause(reason));

        s.reduce_db();

        assert!(!s.clause_is_deleted(reason));
    }

    #[test]
    fn test_reduce_db_protects_glue_one_clauses() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 0);
        let glue_one = s.add_clause(vec![1, 2, 3]);
        set_lbd_meta_for_test(&mut s, glue_one, 1, 0);

        s.reduce_db();

        assert!(!s.clause_is_deleted(glue_one));
    }

    #[test]
    fn test_reduce_db_protects_tier2_with_used_recently() {
        let mut s = Solver::new(4, vec![]);
        enable_lbd_tiered_for_test(&mut s, 0);
        let tier2 = s.add_clause(vec![1, 2, 3, 4]);
        set_lbd_meta_for_test(&mut s, tier2, 4, 1);

        s.reduce_db();

        assert!(!s.clause_is_deleted(tier2));
        assert_eq!(s.learnt_used_recently(tier2), 0);
    }

    #[test]
    fn test_reduce_db_protects_tier3_with_used_recently() {
        let mut s = Solver::new(4, vec![]);
        enable_lbd_tiered_for_test(&mut s, 0);
        let tier3 = s.add_clause(vec![1, 2, 3, 4]);
        set_lbd_meta_for_test(&mut s, tier3, 9, 1);

        s.reduce_db();

        assert!(!s.clause_is_deleted(tier3));
        assert_eq!(s.learnt_used_recently(tier3), 0);
    }

    #[test]
    fn test_reduce_db_drops_high_glue_unused_large_clauses_first() {
        let mut s = Solver::new(5, vec![]);
        enable_lbd_tiered_for_test(&mut s, 3);
        let better = s.add_clause(vec![1, 2, 3]);
        let worse = s.add_clause(vec![1, 2, 3, 4]);
        set_lbd_meta_for_test(&mut s, better, 7, 0);
        set_lbd_meta_for_test(&mut s, worse, 12, 0);

        s.reduce_db();

        assert_eq!(s.learned_clause_ids.len(), 1);
        assert_eq!(s.clause_slice(s.learned_clause_ids[0]), &[1, 2, 3]);
    }

    #[test]
    fn test_reduce_updates_live_learned_counts() {
        let mut s = Solver::new(4, vec![]);
        enable_lbd_tiered_for_test(&mut s, 0);
        let delete_me = s.add_clause(vec![1, 2, 3, 4]);
        set_lbd_meta_for_test(&mut s, delete_me, 12, 0);

        s.reduce_db();

        assert_eq!(s.live_learned_clause_count, 0);
        assert_eq!(s.learned_clause_ids.len(), 0);
        assert_eq!(s.learned_literals, 0);
        assert_eq!(s.stats.learned_collected, 1);
    }

    #[test]
    fn test_reduce_delete_marker_handles_arena_growth() {
        let mut s = Solver::new(3, vec![]);
        let clause_idx = s.add_clause(vec![1, 2, 3]);

        s.begin_reduce_delete_marking();
        s.mark_reduce_delete_candidate(clause_idx);

        assert!(s.reduce_delete_mark.len() >= s.arena.len());
        assert!(s.reduce_delete_candidate_marked(clause_idx));
    }

    #[test]
    fn test_reduce_delete_marker_generation_wraparound() {
        let mut s = Solver::new(2, vec![]);
        s.reduce_delete_mark.resize(4, u64::MAX);
        s.reduce_delete_generation = u64::MAX;

        s.begin_reduce_delete_marking();

        assert_eq!(s.reduce_delete_generation, 1);
        assert!(s.reduce_delete_mark.iter().all(|&mark| mark == 0));
    }

    #[test]
    fn test_reduce_db_lbd_tiered_no_per_call_allocation_when_marker_reused() {
        let mut s = Solver::new(4, vec![]);
        enable_lbd_tiered_for_test(&mut s, 100);
        let first = s.add_clause(vec![1, 2, 3]);
        let second = s.add_clause(vec![1, 2, 3, 4]);
        set_lbd_meta_for_test(&mut s, first, 8, 0);
        set_lbd_meta_for_test(&mut s, second, 9, 0);
        s.begin_reduce_delete_marking();
        let mut proof_log = ProofLog::disabled();

        reset_test_allocations();
        s.reduce_db_lbd_tiered(&mut proof_log);
        s.reduce_db_lbd_tiered(&mut proof_log);
        let allocations = test_allocation_count();
        stop_test_allocations();

        assert_eq!(allocations, 0);
        assert_eq!(s.learned_clause_ids.len(), 2);
        assert_eq!(s.live_learned_clause_count, 2);
    }

    #[test]
    fn test_reduce_deleted_watchers_are_skipped_by_propagation() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 0);
        let delete_me = s.add_clause(vec![2, 1, 3]);
        set_lbd_meta_for_test(&mut s, delete_me, 12, 0);
        s.reduce_db();

        s.decide(-1);

        assert_eq!(s.propagate(), None);
        assert_eq!(s.assignment[2], UNASSIGNED);
    }

    #[test]
    fn test_reason_use_marks_learned_clause_recent() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        let reason = s.add_clause(vec![2, 1, 3]);
        set_lbd_meta_for_test(&mut s, reason, 9, 0);

        let mut current_level_count = 0;
        s.decision_level[1] = 1;
        s.decision_level[2] = 1;
        s.mark_reason_literals_for_analysis::<true>(
            ReasonRef::Clause(reason),
            2,
            1,
            1,
            &mut current_level_count,
        );

        assert_eq!(s.learnt_used_recently(reason), 1);
    }

    #[test]
    fn test_propagation_reason_lbd_update_uses_implied_literal_level() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.update_reason_lbd = true;
        s.update_propagation_reason_lbd = true;
        let reason = s.add_clause(vec![2, 1, 3]);
        set_lbd_meta_for_test(&mut s, reason, 9, 0);

        s.decide(-3);
        s.decide(-1);

        assert_eq!(s.propagate(), None);
        assert_eq!(s.assignment[2], TRUE);
        assert_eq!(s.decision_level[2], 2);
        assert_eq!(s.reason_ref(2), ReasonRef::Clause(reason));
        assert_eq!(s.learnt_lbd(reason), 2);
        assert_eq!(s.learned_meta(reason).unwrap().tier, 0);
        assert_eq!(s.learnt_used_recently(reason), 1);
        assert_eq!(s.stats.lbd_improved, 1);
    }

    #[test]
    fn test_reason_update_flag_does_not_touch_propagation_reason_metadata() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 10);
        s.update_reason_lbd = true;
        let reason = s.add_clause(vec![2, 1, 3]);
        set_lbd_meta_for_test(&mut s, reason, 9, 0);

        s.decide(-3);
        s.decide(-1);

        assert_eq!(s.propagate(), None);
        assert_eq!(s.assignment[2], TRUE);
        assert_eq!(s.learnt_lbd(reason), 9);
        assert_eq!(s.learnt_used_recently(reason), 0);
        assert_eq!(s.stats.lbd_improved, 0);
    }

    #[test]
    fn test_lbd_improvement_reclassifies_clause_tier() {
        let mut s = Solver::new(3, vec![]);
        s.use_lbd = true;
        let clause_idx = s.add_clause(vec![1, 2, 3]);
        set_lbd_meta_for_test(&mut s, clause_idx, 8, 0);

        s.maybe_improve_lbd(clause_idx, 2);

        assert_eq!(s.learnt_lbd(clause_idx), 2);
        assert_eq!(s.learned_meta(clause_idx).unwrap().tier, 0);
    }

    #[test]
    fn test_reduce_respects_learned_lit_budget() {
        let mut s = Solver::new(6, vec![]);
        enable_lbd_tiered_for_test(&mut s, 4);
        let first = s.add_clause(vec![1, 2, 3]);
        let second = s.add_clause(vec![1, 2, 3, 4]);
        set_lbd_meta_for_test(&mut s, first, 8, 0);
        set_lbd_meta_for_test(&mut s, second, 9, 0);

        s.reduce_db();

        assert!(s.learned_literals <= 4);
    }

    #[test]
    fn test_reduce_emergency_can_demote_old_unused_tier1() {
        let mut s = Solver::new(3, vec![]);
        enable_lbd_tiered_for_test(&mut s, 0);
        s.hard_learned_lit_budget = 0;
        s.stats.conflicts = EMERGENCY_TIER1_MIN_AGE_CONFLICTS + 1;
        let tier1 = s.add_clause(vec![1, 2, 3]);
        set_lbd_meta_for_test(&mut s, tier1, 1, 0);
        s.learned_meta_mut_by_id(s.learned_id_for_clause(tier1))
            .created_at_conflict = 0;

        s.reduce_db();

        assert!(s.learned_clause_ids.is_empty());
    }

    #[test]
    fn test_reduce_emergency_never_deletes_locked_binary_or_unit() {
        let mut s = Solver::new(4, vec![]);
        enable_lbd_tiered_for_test(&mut s, 0);
        s.hard_learned_lit_budget = 0;
        s.stats.conflicts = EMERGENCY_TIER1_MIN_AGE_CONFLICTS + 1;
        let unit = s.add_clause(vec![1]);
        let binary = s.add_clause(vec![2, 3]);
        let locked = s.add_clause(vec![4, 1, 2]);
        set_lbd_meta_for_test(&mut s, unit, 1, 0);
        set_lbd_meta_for_test(&mut s, binary, 1, 0);
        set_lbd_meta_for_test(&mut s, locked, 1, 0);
        s.assignment[4] = TRUE;
        s.set_reason_ref(4, ReasonRef::Clause(locked));

        s.reduce_db();

        assert!(!s.clause_is_deleted(unit));
        assert!(!s.clause_is_deleted(binary));
        assert!(!s.clause_is_deleted(locked));
    }

    #[test]
    fn test_gc_preserves_learned_meta_after_reduction() {
        let mut s = Solver::new(5, vec![]);
        enable_lbd_tiered_for_test(&mut s, 3);
        let live = s.add_clause(vec![1, 2, 3]);
        let deleted = s.add_clause(vec![1, 2, 3, 4]);
        set_lbd_meta_for_test(&mut s, live, 4, 1);
        set_lbd_meta_for_test(&mut s, deleted, 12, 0);

        s.reduce_db();
        s.garbage_collect();

        let relocated_live = s.learned_clause_ids[0];
        assert_eq!(s.learnt_lbd(relocated_live), 4);
        assert_eq!(s.learnt_used_recently(relocated_live), 0);
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
    fn test_hot_watch_stats_are_opt_in() {
        let clauses = vec![vec![1, 2]];
        let mut default_solver = make_solver(2, clauses.clone());
        default_solver.decide(-1);
        assert_eq!(default_solver.propagate(), None);
        assert_eq!(default_solver.stats.propagations, 2);
        assert_eq!(default_solver.stats.watch_scans, 0);
        assert_eq!(default_solver.stats.watch_clause_loads, 0);
        assert_eq!(default_solver.stats.binary_props, 0);

        let config = SolverConfig {
            hot_stats: true,
            ..SolverConfig::default()
        };
        let mut diagnostic_solver = make_solver_with_config(2, clauses, &config);
        diagnostic_solver.decide(-1);
        assert_eq!(diagnostic_solver.propagate(), None);
        assert_eq!(
            diagnostic_solver.stats.propagations,
            default_solver.stats.propagations
        );
        assert!(diagnostic_solver.stats.watch_scans > 0);
        assert!(diagnostic_solver.stats.watch_clause_loads > 0);
        assert!(diagnostic_solver.stats.binary_props > 0);
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
        let (raw_learned, raw_backtrack) =
            s.analyze_conflict(Conflict::Clause(reason_clause_ids[3]));
        assert_eq!(raw_learned, vec![-1, 3, 4, 5]);
        assert_eq!(raw_backtrack, 1);

        s.ccmin_mode = CCMIN_BASIC;
        let (basic_learned, basic_backtrack) =
            s.analyze_conflict(Conflict::Clause(reason_clause_ids[3]));
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
        let (raw_learned, raw_backtrack) =
            s.analyze_conflict(Conflict::Clause(reason_clause_ids[4]));
        assert_eq!(raw_learned, vec![-1, 3, 4, 7, 6]);
        assert_eq!(raw_backtrack, 1);

        s.ccmin_mode = CCMIN_BASIC;
        let (basic_learned, _) = s.analyze_conflict(Conflict::Clause(reason_clause_ids[4]));
        assert_eq!(basic_learned, vec![-1, 3, 4, 7, 6]);

        s.ccmin_mode = CCMIN_DEEP;
        let (deep_learned, deep_backtrack) =
            s.analyze_conflict(Conflict::Clause(reason_clause_ids[4]));
        assert_eq!(deep_learned, vec![-1, 3, 4, 6]);
        assert_eq!(deep_backtrack, 1);
    }

    #[test]
    fn test_deep_clause_minimization_recurses_through_learned_reasons() {
        let mut s = make_solver(7, vec![vec![5, 3]]);
        let learned_reason = s.add_clause(vec![7, -5, 3]);
        s.set_reason_ref(5, ReasonRef::Clause(0));
        s.set_reason_ref(7, ReasonRef::Clause(learned_reason));

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
        s.set_reason_ref(5, ReasonRef::Clause(0));

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
        s.set_reason_ref(5, ReasonRef::Clause(0));

        let mut learned_clause = vec![-1, 3, 5];

        s.ccmin_mode = CCMIN_BASIC;
        s.minimize_learned_clause(&mut learned_clause);
        assert_eq!(learned_clause, vec![-1, 3, 5]);

        s.ccmin_mode = CCMIN_DEEP;
        s.minimize_learned_clause(&mut learned_clause);
        assert_eq!(learned_clause, vec![-1, 3, 5]);
    }

    #[test]
    fn test_clause_minimization_binary_reason_skips_resolved_variable_by_identity() {
        let mut s = make_solver(5, vec![]);
        let binary_id = s.add_binary_reason_for_test([3, 5]);
        s.decision_level[1] = 2;
        s.decision_level[3] = 1;
        s.decision_level[5] = 1;
        s.set_reason_ref(5, ReasonRef::Binary(binary_id));

        let mut basic_clause = vec![-1, -5];
        s.ccmin_mode = CCMIN_BASIC;
        s.minimize_learned_clause(&mut basic_clause);
        assert_eq!(basic_clause, vec![-1, -5]);

        let mut deep_clause = vec![-1, -5];
        s.ccmin_mode = CCMIN_DEEP;
        s.minimize_learned_clause(&mut deep_clause);
        assert_eq!(deep_clause, vec![-1, -5]);
    }

    #[test]
    fn test_binary_fast_clause_minimization_handles_binary_reason_order() {
        let config = SolverConfig {
            binary_fast_path: true,
            clause_min_mode: ClauseMinMode::Basic,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(5, vec![], &config);
        let binary_id = s.add_binary_reason_for_test([3, 5]);
        s.decision_level[1] = 2;
        s.decision_level[3] = 1;
        s.decision_level[5] = 1;
        s.set_reason_ref(5, ReasonRef::Binary(binary_id));

        let mut learned_clause = vec![-1, 3, -5];
        s.minimize_learned_clause(&mut learned_clause);

        assert_eq!(learned_clause, vec![-1, 3]);
    }

    #[test]
    fn test_recursive_minimization_descends_into_binary_reason_by_identity() {
        let mut s = make_solver(7, vec![vec![7, 5]]);
        let binary_id = s.add_binary_reason_for_test([3, 5]);
        s.decision_level[1] = 2;
        s.decision_level[3] = 1;
        s.decision_level[5] = 1;
        s.decision_level[7] = 1;
        s.set_reason_ref(5, ReasonRef::Binary(binary_id));
        s.set_reason_ref(7, ReasonRef::Clause(0));

        let mut learned_clause = vec![-1, 7];
        s.ccmin_mode = CCMIN_DEEP;
        s.minimize_learned_clause(&mut learned_clause);

        assert_eq!(learned_clause, vec![-1, 7]);
    }

    #[test]
    fn test_minimization_does_not_remove_decision_literal() {
        let mut s = make_solver(5, vec![]);
        s.decision_level[1] = 2;
        s.decision_level[5] = 1;

        let mut learned_clause = vec![-1, 5];
        s.ccmin_mode = CCMIN_DEEP;
        s.minimize_learned_clause(&mut learned_clause);

        assert_eq!(learned_clause, vec![-1, 5]);
    }

    #[test]
    fn test_recursive_limit_prevents_unbounded_walk() {
        let mut s = make_solver(7, vec![vec![5, 3], vec![7, 5]]);
        let first = s.original_clause_ids[0];
        let second = s.original_clause_ids[1];
        s.decision_level[1] = 2;
        s.decision_level[3] = 1;
        s.decision_level[5] = 1;
        s.decision_level[7] = 1;
        s.set_reason_ref(5, ReasonRef::Clause(first));
        s.set_reason_ref(7, ReasonRef::Clause(second));

        let mut limited_clause = vec![-1, 3, 7];
        s.ccmin_mode = CCMIN_DEEP;
        s.minimize_depth_limit = 0;
        s.minimize_learned_clause(&mut limited_clause);
        assert_eq!(limited_clause, vec![-1, 3, 7]);

        let mut recursive_clause = vec![-1, 3, 7];
        s.minimize_depth_limit = 2;
        s.minimize_learned_clause(&mut recursive_clause);
        assert_eq!(recursive_clause, vec![-1, 3]);
    }

    #[test]
    fn test_minimized_clause_still_asserting() {
        let mut s = make_solver(5, vec![vec![3, 4]]);
        s.decision_level[1] = 2;
        s.decision_level[3] = 1;
        s.decision_level[4] = 1;
        s.set_reason_ref(3, ReasonRef::Clause(0));

        let mut learned_clause = vec![-1, 4, 3];
        s.ccmin_mode = CCMIN_BASIC;
        s.minimize_learned_clause(&mut learned_clause);

        assert_eq!(learned_clause[0], -1);
        assert_eq!(learned_clause, vec![-1, 4]);
    }

    #[test]
    fn test_shrink_removes_block_covered_literal() {
        let mut s = make_solver(5, vec![vec![5, 3]]);
        s.decision_level[1] = 2;
        s.decision_level[3] = 1;
        s.decision_level[5] = 1;
        s.set_reason_ref(5, ReasonRef::Clause(0));

        let mut learned_clause = vec![-1, 3, 5];
        s.ccmin_mode = CCMIN_INBLOCK;
        s.minimize_learned_clause(&mut learned_clause);

        assert_eq!(learned_clause, vec![-1, 3]);
    }

    #[test]
    fn test_shrink_does_not_cross_decision_level_blocks() {
        let mut s = make_solver(7, vec![vec![5, 3], vec![7, 5]]);
        let first = s.original_clause_ids[0];
        let second = s.original_clause_ids[1];
        s.decision_level[1] = 3;
        s.decision_level[3] = 1;
        s.decision_level[5] = 1;
        s.decision_level[7] = 2;
        s.set_reason_ref(5, ReasonRef::Clause(first));
        s.set_reason_ref(7, ReasonRef::Clause(second));

        let mut learned_clause = vec![-1, 3, 7];
        s.ccmin_mode = CCMIN_INBLOCK;
        s.minimize_learned_clause(&mut learned_clause);

        assert_eq!(learned_clause, vec![-1, 3, 7]);
    }

    #[test]
    fn test_shrink_leaves_uip_at_pos_zero() {
        let mut s = make_solver(5, vec![vec![5, 3]]);
        s.decision_level[1] = 2;
        s.decision_level[3] = 1;
        s.decision_level[5] = 1;
        s.set_reason_ref(5, ReasonRef::Clause(0));

        let mut learned_clause = vec![-1, 3, 5];
        s.ccmin_mode = CCMIN_INBLOCK;
        s.minimize_learned_clause(&mut learned_clause);

        assert_eq!(learned_clause.first().copied(), Some(-1));
    }

    #[test]
    fn test_otfs_default_off_leaves_subsumed_watcher() {
        let mut s = make_solver(3, vec![]);
        let subsumed = s.add_clause(vec![1, 2, 3]);
        let mut proof_log = ProofLog::disabled();

        let deleted = s.otfs_subsume_watched_clauses(&[1, 2], None, &mut proof_log);

        assert_eq!(deleted, 0);
        assert!(!s.clause_is_deleted(subsumed));
        assert_eq!(s.stats.otfs_subsumed_clauses, 0);
    }

    #[test]
    fn test_otfs_removes_subsumed_watcher() {
        let mut s = make_solver(3, vec![]);
        s.otfs_enabled = true;
        let subsumed = s.add_clause(vec![1, 2, 3]);
        let mut proof_log = ProofLog::disabled();

        let deleted = s.otfs_subsume_watched_clauses(&[1, 2], None, &mut proof_log);

        assert_eq!(deleted, 1);
        assert!(s.clause_is_deleted(subsumed));
        assert!(s.learned_clause_ids.is_empty());
        assert_eq!(s.live_learned_clause_count, 0);
        assert_eq!(s.learned_literals, 0);
        assert_eq!(s.stats.deleted_clauses, 1);
        assert_eq!(s.stats.otfs_subsumed_clauses, 1);
        assert_eq!(s.stats.otfs_subsumed_learned, 1);
    }

    #[test]
    fn test_otfs_does_not_remove_non_subsumed() {
        let mut s = make_solver(7, vec![]);
        s.otfs_enabled = true;
        let first = s.add_clause(vec![1, -2, 3]);
        let too_long = s.add_clause(vec![1, 2, 3, 4, 5, 6, 7]);
        let mut proof_log = ProofLog::disabled();

        let deleted = s.otfs_subsume_watched_clauses(&[1, 2], None, &mut proof_log);

        assert_eq!(deleted, 0);
        assert!(!s.clause_is_deleted(first));
        assert!(!s.clause_is_deleted(too_long));
        assert_eq!(s.learned_clause_ids.len(), 2);
        assert_eq!(s.stats.deleted_clauses, 0);
        assert_eq!(s.stats.otfs_subsumed_clauses, 0);
    }

    #[test]
    fn test_otfs_does_not_remove_original_clause() {
        let mut s = make_solver(3, vec![vec![1, 2, 3]]);
        s.otfs_enabled = true;
        let original = s.original_clause_ids[0];
        let mut proof_log = ProofLog::disabled();

        let deleted = s.otfs_subsume_watched_clauses(&[1, 2], None, &mut proof_log);

        assert_eq!(deleted, 0);
        assert!(!s.clause_is_deleted(original));
        assert_eq!(s.live_original_clause_count(), 1);
        assert_eq!(s.original_literals, 3);
        assert_eq!(s.stats.otfs_subsumed_clauses, 0);
    }

    #[test]
    fn test_otfs_proof_logs_deletion() {
        let mut s = make_solver(3, vec![]);
        s.otfs_enabled = true;
        let subsumed = s.add_clause(vec![1, 2, 3]);
        let dir = make_temp_dir("otfs-proof");
        let mut proof_log = ProofLog::new(&dir, 32, false);
        proof_log.record_clause(&[1, 2, 3]);
        proof_log.record_clause(&[1, 2]);

        let deleted = s.otfs_subsume_watched_clauses(&[1, 2], None, &mut proof_log);
        proof_log.finish_unsat();

        assert_eq!(deleted, 1);
        assert!(s.clause_is_deleted(subsumed));
        assert!(s.learned_clause_ids.is_empty());
        assert_eq!(s.live_learned_clause_count, 0);
        assert_eq!(s.learned_literals, 0);
        assert_eq!(s.stats.otfs_subsumed_learned, 1);

        let proof_text =
            fs::read_to_string(dir.join("proof.out")).expect("failed to read OTFS proof");
        assert!(
            proof_text.contains("d 1 2 3 0\n"),
            "expected OTFS deletion in proof, got:\n{proof_text}"
        );
    }

    #[test]
    fn test_otfs_does_not_remove_live_reason_clause() {
        let mut s = make_solver(3, vec![]);
        s.otfs_enabled = true;
        let reason = s.add_clause(vec![3, 1, 2]);
        s.assignment[3] = TRUE;
        s.decision_level[3] = 1;
        s.set_reason_ref(3, ReasonRef::Clause(reason));
        s.trail.push(3);
        let mut proof_log = ProofLog::disabled();

        let deleted = s.otfs_subsume_watched_clauses(&[1, 2], None, &mut proof_log);

        assert_eq!(deleted, 0);
        assert!(!s.clause_is_deleted(reason));
        assert_eq!(s.learned_clause_ids, vec![reason]);
        assert_eq!(s.stats.otfs_subsumed_clauses, 0);
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
        assert_eq!(s.reason_ref(3), ReasonRef::Clause(relocated_live));
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
        s.set_reason_ref(4, ReasonRef::Clause(live));
        s.trail.push(4);
        s.trail_limits.push(0);

        s.mark_clause_deleted(dead);
        s.garbage_collect();
        let relocated_live = s.learned_clause_ids[0];

        assert_eq!(s.reason_ref(4), ReasonRef::Clause(relocated_live));
        assert_eq!(s.clause_slice(s.reason_clause_for_test(4)), &[3, 1]);
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
        assert!(s.enqueue(1, ReasonRef::None));

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
    fn test_eliminated_var_not_reinserted() {
        let mut s = make_solver(2, vec![]);

        assert!(s.heap_contains_var(1));
        s.branch_heap_remove(1);
        s.eliminated[1] = true;
        s.decision_var[1] = false;

        s.heap_reinsert_unassigned_decision_var(1);

        assert!(!s.heap_contains_var(1));
        assert_eq!(s.branch_pos[1], BRANCH_NOT_IN_HEAP);
    }

    #[test]
    fn test_assigned_heap_top_skipped() {
        let mut s = make_solver(3, vec![]);
        s.activity[1] = 10.0;
        s.activity[2] = 2.0;
        s.activity[3] = 1.0;
        s.rebuild_branch_queue();
        assert_eq!(s.branch_heap[0] as usize, 1);

        s.assignment[1] = TRUE;
        s.stats.decision_heap_pops = 0;
        s.stats.decision_heap_stale_pops = 0;

        s.heap_remove_assigned_top();

        assert!(!s.heap_contains_var(1));
        assert_eq!(s.branch_heap[0] as usize, 2);
        assert_eq!(s.stats.decision_heap_pops, 1);
        assert_eq!(s.stats.decision_heap_stale_pops, 1);
    }

    #[test]
    fn test_backtrack_reinserts_unassigned_decision_var() {
        let mut s = make_solver(2, vec![]);
        s.activity[1] = 3.0;
        s.activity[2] = 1.0;
        s.rebuild_branch_queue();

        let lit = s.pick_branch_lit().expect("expected branch literal");
        assert_eq!(lit.unsigned_abs(), 1);
        s.decide(lit);
        assert!(!s.heap_contains_var(1));

        s.backtrack(0);

        assert_eq!(s.assignment[1], UNASSIGNED);
        assert!(s.heap_contains_var(1));
    }

    #[test]
    fn test_activity_bump_percolates() {
        let mut s = make_solver(3, vec![]);
        s.activity[1] = 1.0;
        s.activity[2] = 2.0;
        s.activity[3] = 3.0;
        s.activity_inc = 4.0;
        s.rebuild_branch_queue();
        assert_eq!(s.branch_heap[0] as usize, 3);

        s.bump_variable_activity(1);

        assert_eq!(s.branch_heap[0] as usize, 1);
    }

    #[test]
    fn test_heap_push_respects_decision_var() {
        let mut s = make_solver(1, vec![]);
        s.branch_heap_remove(1);

        s.decision_var[1] = false;
        s.push_branch_var_if_decision(1);
        assert!(!s.heap_contains_var(1));

        s.decision_var[1] = true;
        s.eliminated[1] = true;
        s.push_branch_var_if_decision(1);
        assert!(!s.heap_contains_var(1));

        s.frozen[1] = true;
        s.eliminated[1] = false;
        s.decision_var[1] = false;
        s.push_branch_var_if_decision(1);
        assert!(!s.heap_contains_var(1));

        s.decision_var[1] = true;
        s.frozen[1] = false;
        s.push_branch_var_if_decision(1);
        assert!(s.heap_contains_var(1));
    }

    #[test]
    fn test_heap_tie_break_is_deterministic() {
        let mut s = make_solver(3, vec![]);
        s.activity[1] = 1.0;
        s.activity[2] = 1.0;
        s.activity[3] = 1.0;
        s.rebuild_branch_queue();

        assert_eq!(s.pick_branch_lit(), Some(-1));
        assert_eq!(s.pick_branch_lit(), Some(-2));
        assert_eq!(s.pick_branch_lit(), Some(-3));
    }

    #[test]
    fn test_activity_rescale_preserves_order() {
        let mut s = make_solver(3, vec![]);
        s.activity[1] = 6.0e99;
        s.activity[2] = 3.0e99;
        s.activity[3] = 1.0e99;
        s.activity_inc = 6.0e99;
        s.rebuild_branch_queue();

        s.bump_variable_activity(1);

        assert_eq!(s.pick_branch_lit(), Some(-1));
        assert_eq!(s.pick_branch_lit(), Some(-2));
        assert_eq!(s.pick_branch_lit(), Some(-3));
    }

    #[test]
    fn test_same_seed_reproduces_decision_prefix_on_small_formula() {
        let config = SolverConfig {
            deterministic_seed: 17,
            ..SolverConfig::default()
        };
        let clauses = vec![vec![1, 2], vec![-1, 3], vec![4, -2]];

        let first = make_solver_with_config(4, clauses.clone(), &config);
        let second = make_solver_with_config(4, clauses, &config);

        assert_eq!(decision_prefix(first, 4), decision_prefix(second, 4));
    }

    #[test]
    fn test_different_seed_changes_only_randomized_policy() {
        let clauses = vec![vec![1, 2], vec![-1, 3], vec![4, -2]];
        let first_config = SolverConfig {
            deterministic_seed: 11,
            ..SolverConfig::default()
        };
        let second_config = SolverConfig {
            deterministic_seed: 29,
            ..SolverConfig::default()
        };

        let first = make_solver_with_config(4, clauses.clone(), &first_config);
        let second = make_solver_with_config(4, clauses, &second_config);

        assert_eq!(decision_prefix(first, 4), decision_prefix(second, 4));
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
        s.set_reason_ref(3, ReasonRef::Clause(locked));
        s.trail.push(3);
        s.trail_limits.push(0);
        s.propagate_head = s.trail.len();

        s.reduce_db();

        assert_eq!(s.learned_clause_count(), 2);
        assert_eq!(s.stats.reduce_db_calls, 1);
        assert_eq!(s.stats.deleted_clauses, 1);
        assert_eq!(s.clause_slice(s.reason_clause_for_test(3)), &[3, 1, 2]);

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

        assert!(s.enqueue(1, ReasonRef::None));
        assert_eq!(s.propagate(), None);
        assert_eq!(s.assignment[1], TRUE);
        assert_eq!(s.assignment[2], TRUE);
        assert_eq!(s.reason_ref(2), ReasonRef::Clause(satisfied_learned));

        assert!(s.simplify());

        assert_eq!(s.reason_ref(2), ReasonRef::None);
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
        let mut proof = ProofLog::new(&dir, 32, false);

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
        let mut unsat_proof = ProofLog::new(&unsat_dir, 32, false);
        unsat_proof.record_clause(&[1, -2]);
        unsat_proof.record_deletion(&[3, -4]);
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
            unsat_text.contains("d 3 -4 0\n"),
            "expected deleted clause to be serialized in DRAT deletion format"
        );
        assert!(
            unsat_text.ends_with("0\n"),
            "expected UNSAT proof to end with the empty clause"
        );

        let sat_dir = make_temp_dir("proof-sat");
        let mut sat_proof = ProofLog::new(&sat_dir, 32, false);
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
        let config = SolverConfig::default();
        assert_eq!(
            s.solve_to_output(proof_dir.to_str().expect("utf8 temp dir"), &config)
                .0
                .status,
            SolveStatus::Unsat
        );

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
    fn test_chrono_off_uses_assertion_level() {
        let mut s = make_solver(2, vec![]);
        s.decide(1);
        s.decide(2);

        let chosen = s.choose_backtrack_level(0, &[-2, -1]);

        assert_eq!(chosen, 0);
        assert_eq!(s.stats.chrono_attempts, 0);
        assert_eq!(s.stats.chrono_used, 0);
    }

    #[test]
    fn test_chrono_rejects_large_delta() {
        let config = chrono_config(1);
        let mut s = make_solver_with_config(3, vec![], &config);
        s.decide(1);
        s.decide(2);
        s.decide(3);

        let chosen = s.choose_backtrack_level(0, &[-3, -1]);

        assert_eq!(chosen, 0);
        assert_eq!(s.stats.chrono_attempts, 1);
        assert_eq!(s.stats.chrono_used, 0);
        assert_eq!(s.stats.chrono_rejected_delta_too_large, 1);
    }

    #[test]
    fn test_chrono_allows_small_delta_when_asserting() {
        let config = chrono_config(3);
        let mut s = make_solver_with_config(3, vec![], &config);
        s.decide(1);
        s.decide(2);
        s.decide(3);

        let chosen = s.choose_backtrack_level(0, &[-3, -1]);

        assert_eq!(chosen, 2);
        assert_eq!(s.stats.chrono_attempts, 1);
        assert_eq!(s.stats.chrono_used, 1);
        assert_eq!(s.stats.chrono_skipped_levels, 2);
    }

    #[test]
    fn test_chrono_rejects_non_asserting_level() {
        let config = chrono_config(3);
        let mut s = make_solver_with_config(4, vec![], &config);
        s.decide(1);
        s.decide(2);
        s.decide(3);
        assert!(s.enqueue(4, ReasonRef::None));

        let chosen = s.choose_backtrack_level(0, &[-3, -4, -1]);

        assert_eq!(chosen, 0);
        assert_eq!(s.stats.chrono_attempts, 1);
        assert_eq!(s.stats.chrono_used, 0);
        assert_eq!(s.stats.chrono_rejected_not_asserting, 1);
    }

    #[test]
    fn test_chrono_backtrack_preserves_reason_invariant() {
        let config = chrono_config(3);
        let mut s = make_solver_with_config(3, vec![], &config);
        s.decide(1);
        s.decide(2);
        s.decide(3);
        let learned_clause = vec![-3, -2, -1];
        let backtrack_level = s.choose_backtrack_level(1, &learned_clause);
        assert_eq!(backtrack_level, 2);

        let learned_clause_idx = s.add_analyzed_clause_from_slice(&learned_clause);
        s.backtrack(backtrack_level);
        s.debug_assert_clause_asserting_after_backtrack(
            s.clause_slice(learned_clause_idx),
            backtrack_level,
        );
        assert!(s.enqueue(-3, s.reason_ref_for_clause(learned_clause_idx)));

        assert_eq!(s.current_level(), 2);
        assert_eq!(s.assignment[1], TRUE);
        assert_eq!(s.assignment[2], TRUE);
        assert_eq!(s.assignment[3], FALSE);
        assert_eq!(s.decision_level[3], 2);
        assert_eq!(s.reason_ref(3), ReasonRef::Clause(learned_clause_idx));
        assert_eq!(s.lit_value(-1), FALSE);
        assert_eq!(s.lit_value(-2), FALSE);
        assert_eq!(s.lit_value(-3), TRUE);
    }

    #[test]
    fn test_chrono_root_conflict_unchanged() {
        let config = chrono_config(3);
        let mut s = make_solver_with_config(1, vec![], &config);

        let chosen = s.choose_backtrack_level(0, &[1]);

        assert_eq!(chosen, 0);
        assert_eq!(s.stats.chrono_attempts, 0);
        assert_eq!(s.stats.chrono_used, 0);
    }

    #[test]
    fn test_chrono_does_not_break_smoke_unsat_proof() {
        let config = chrono_config(100);
        let clauses = vec![vec![1, 2], vec![-1, 2], vec![1, -2], vec![-1, -2]];
        let proof_dir = make_temp_dir("chrono-unsat-proof");
        let mut s = make_solver_with_config(2, clauses, &config);

        assert_eq!(
            s.solve_to_output(proof_dir.to_str().expect("utf8 temp dir"), &config)
                .0
                .status,
            SolveStatus::Unsat
        );

        let proof_text =
            fs::read_to_string(proof_dir.join("proof.out")).expect("failed to read emitted proof");
        assert!(
            proof_text.ends_with("0\n"),
            "expected chrono-enabled UNSAT proof to end with the empty clause"
        );
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
    fn test_pick_branch_legacy_increments_correct_counter() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::Legacy,
            ..single_mode_config()
        };
        let mut s = make_solver_with_config(1, vec![], &config);
        s.saved_phase[1] = TRUE;

        assert_eq!(s.pick_branch_lit(), Some(1));
        assert_eq!(s.stats.phase_legacy_used, 1);
        assert_eq!(s.stats.phase_initial_used, 0);
        assert_eq!(s.stats.phase_saved_used, 0);
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
    fn test_phase_falls_back_to_initial() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::Saved,
            ..single_mode_config()
        };
        let mut s = make_solver_with_config(1, vec![], &config);

        assert_eq!(s.saved_phase[1], UNASSIGNED);
        assert_eq!(s.pick_branch_lit(), Some(-1));
        assert_eq!(s.stats.phase_initial_used, 1);
        assert_eq!(s.stats.phase_saved_used, 0);
    }

    #[test]
    fn test_saved_phase_used_when_no_target() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::TargetThenSaved,
            ..single_mode_config()
        };
        let mut s = make_solver_with_config(1, vec![], &config);
        s.saved_phase[1] = TRUE;

        assert_eq!(s.pick_branch_lit(), Some(1));
        assert_eq!(s.stats.phase_saved_used, 1);
        assert_eq!(s.stats.phase_target_used, 0);
    }

    #[test]
    fn test_target_phase_precedes_saved() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::TargetThenSaved,
            ..single_mode_config()
        };
        let mut s = make_solver_with_config(1, vec![], &config);
        s.saved_phase[1] = TRUE;
        s.target_phase[1] = FALSE;

        assert_eq!(s.pick_branch_lit(), Some(-1));
        assert_eq!(s.stats.phase_target_used, 1);
        assert_eq!(s.stats.phase_saved_used, 0);
    }

    #[test]
    fn test_best_phase_precedes_target_when_policy_selected() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::BestThenTargetThenSaved,
            ..single_mode_config()
        };
        let mut s = make_solver_with_config(1, vec![], &config);
        s.saved_phase[1] = FALSE;
        s.target_phase[1] = FALSE;
        s.best_phase[1] = TRUE;

        assert_eq!(s.pick_branch_lit(), Some(1));
        assert_eq!(s.stats.phase_best_used, 1);
        assert_eq!(s.stats.phase_target_used, 0);
        assert_eq!(s.stats.phase_saved_used, 0);
    }

    #[test]
    fn test_target_phase_captured_at_new_deep_prefix() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::TargetThenSaved,
            ..single_mode_config()
        };
        let mut s = make_solver_with_config(3, vec![], &config);

        s.decide(1);
        s.maybe_capture_phase_prefix();
        assert_eq!(s.target_assigned, 1);
        assert_eq!(s.target_phase[1], TRUE);
        assert_eq!(s.phase_ticks, 1);

        s.backtrack(0);
        s.decide(-2);
        s.maybe_capture_phase_prefix();
        assert_eq!(s.target_assigned, 1);
        assert_eq!(
            s.target_phase[2], UNASSIGNED,
            "equal-depth prefixes should not replace the captured target"
        );

        s.decide(3);
        s.maybe_capture_phase_prefix();
        assert_eq!(s.target_assigned, 2);
        assert_eq!(s.target_phase[2], FALSE);
        assert_eq!(s.target_phase[3], TRUE);
        assert_eq!(s.phase_ticks, 2);
    }

    #[test]
    fn test_focused_stable_target_phase_survives_restart() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::TargetThenSaved,
            ..focused_stable_config()
        };
        let mut s = make_solver_with_config(2, vec![], &config);

        s.decide(1);
        s.maybe_capture_phase_prefix();
        assert_eq!(s.target_phase[1], TRUE);
        assert_eq!(s.target_assigned, 1);

        s.restart_pending = true;
        assert!(s.perform_restart_if_pending());

        assert_eq!(s.target_assigned, 1);
        assert_eq!(s.target_phase[1], TRUE);
        assert_eq!(s.current_level(), 0);
    }

    #[test]
    fn test_single_mode_target_phase_resets_on_restart() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::TargetThenSaved,
            ..single_mode_config()
        };
        let mut s = make_solver_with_config(2, vec![], &config);

        s.decide(1);
        s.maybe_capture_phase_prefix();
        assert_eq!(s.target_phase[1], TRUE);
        assert_eq!(s.target_assigned, 1);

        s.restart_pending = true;
        assert!(s.perform_restart_if_pending());

        assert_eq!(s.target_assigned, 0);
        assert_eq!(s.target_phase[1], UNASSIGNED);
        assert_eq!(s.current_level(), 0);
    }

    #[test]
    fn test_focused_stable_target_phase_survives_pending_restart_already_at_root() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::TargetThenSaved,
            ..focused_stable_config()
        };
        let mut s = make_solver_with_config(1, vec![], &config);
        s.target_phase[1] = TRUE;
        s.target_assigned = 1;
        s.restart_pending = true;

        assert!(!s.perform_restart_if_pending());

        assert!(!s.restart_pending);
        assert_eq!(s.target_assigned, 1);
        assert_eq!(s.target_phase[1], TRUE);
        assert_eq!(s.stats.restarts, 0);
    }

    #[test]
    fn test_single_mode_target_phase_resets_when_pending_restart_already_at_root() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::TargetThenSaved,
            ..single_mode_config()
        };
        let mut s = make_solver_with_config(1, vec![], &config);
        s.target_phase[1] = TRUE;
        s.target_assigned = 1;
        s.restart_pending = true;

        assert!(!s.perform_restart_if_pending());

        assert!(!s.restart_pending);
        assert_eq!(s.target_assigned, 0);
        assert_eq!(s.target_phase[1], UNASSIGNED);
        assert_eq!(s.stats.restarts, 0);
    }

    #[test]
    fn test_target_assigned_monotone_across_restarts() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::TargetThenSaved,
            ..focused_stable_config()
        };
        let mut s = make_solver_with_config(3, vec![], &config);

        s.decide(1);
        s.maybe_capture_phase_prefix();
        assert_eq!(s.target_assigned, 1);
        assert_eq!(s.target_phase[1], TRUE);

        s.restart_pending = true;
        assert!(s.perform_restart_if_pending());
        assert_eq!(s.target_assigned, 1);
        assert_eq!(s.target_phase[1], TRUE);

        s.decide(1);
        s.decide(-2);
        s.maybe_capture_phase_prefix();
        assert_eq!(s.target_assigned, 2);
        assert_eq!(s.target_phase[1], TRUE);
        assert_eq!(s.target_phase[2], FALSE);

        s.restart_pending = true;
        assert!(s.perform_restart_if_pending());
        assert_eq!(s.target_assigned, 2);
        assert_eq!(s.target_phase[2], FALSE);

        s.decide(1);
        s.decide(-2);
        s.decide(3);
        s.maybe_capture_phase_prefix();
        assert_eq!(s.target_assigned, 3);
        assert_eq!(s.target_phase[3], TRUE);
    }

    #[test]
    fn test_pick_branch_phase_uses_target_after_restart() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::TargetThenSaved,
            ..focused_stable_config()
        };
        let mut s = make_solver_with_config(1, vec![], &config);
        s.saved_phase[1] = TRUE;
        s.target_phase[1] = FALSE;
        s.target_assigned = 1;

        s.decide(1);
        s.restart_pending = true;
        assert!(s.perform_restart_if_pending());

        assert!(!s.pick_branch_phase(1));
        assert_eq!(s.stats.phase_target_used, 1);
        assert_eq!(s.stats.phase_saved_used, 0);
    }

    #[test]
    fn test_best_phase_only_grows_monotonically() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::BestThenTargetThenSaved,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(3, vec![], &config);

        s.decide(1);
        s.maybe_capture_phase_prefix();
        assert_eq!(s.best_assigned, 1);
        assert_eq!(s.best_phase[1], TRUE);

        s.backtrack(0);
        s.decide(-2);
        s.maybe_capture_phase_prefix();
        assert_eq!(s.best_assigned, 1);
        assert_eq!(s.best_phase[2], UNASSIGNED);

        s.decide(3);
        s.maybe_capture_phase_prefix();
        assert_eq!(s.best_assigned, 2);
        assert_eq!(s.best_phase[2], FALSE);
        assert_eq!(s.best_phase[3], TRUE);
    }

    #[test]
    fn test_target_phase_reset_on_rephase() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::TargetThenSaved,
            rephase: true,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(2, vec![], &config);
        s.target_phase[1] = TRUE;
        s.target_phase[2] = FALSE;
        s.target_assigned = 2;

        s.apply_rephase();

        assert_eq!(s.target_assigned, 0);
        assert_eq!(s.target_phase[1], UNASSIGNED);
        assert_eq!(s.target_phase[2], UNASSIGNED);
        assert_eq!(s.stats.rephases, 1);
    }

    #[test]
    fn test_rephase_best_writes_best_phase_into_saved() {
        let config = SolverConfig {
            rephase: true,
            ..focused_stable_config()
        };
        let mut s = make_solver_with_config(3, vec![], &config);
        s.best_phase[1] = TRUE;
        s.best_phase[2] = FALSE;
        s.saved_phase[3] = TRUE;

        s.apply_rephase();

        assert_eq!(s.saved_phase[1], TRUE);
        assert_eq!(s.saved_phase[2], FALSE);
        assert_eq!(
            s.saved_phase[3], FALSE,
            "variables outside the best prefix fall back to the original phase"
        );
        assert_eq!(s.rephase_index, 1);
        assert_eq!(s.stats.rephases, 1);
    }

    #[test]
    fn test_rephase_inverted_flips_all_saved_phases() {
        let config = SolverConfig {
            rephase: true,
            ..focused_stable_config()
        };
        let mut s = make_solver_with_config(3, vec![], &config);
        s.rephase_index = 1;
        s.saved_phase[1] = TRUE;
        s.saved_phase[2] = FALSE;
        s.saved_phase[3] = UNASSIGNED;

        s.apply_rephase();

        assert_eq!(s.saved_phase[1], FALSE);
        assert_eq!(s.saved_phase[2], TRUE);
        assert_eq!(
            s.saved_phase[3], TRUE,
            "unassigned saved phases invert from the original phase"
        );
        assert_eq!(s.rephase_index, 2);
    }

    #[test]
    fn test_rephase_original_restores_original_phase() {
        let config = SolverConfig {
            rephase: true,
            ..focused_stable_config()
        };
        let mut s = make_solver_with_config(3, vec![], &config);
        s.rephase_index = 2;
        s.saved_phase[1] = TRUE;
        s.saved_phase[2] = TRUE;
        s.saved_phase[3] = FALSE;

        s.apply_rephase();

        assert_eq!(s.saved_phase[1], FALSE);
        assert_eq!(s.saved_phase[2], FALSE);
        assert_eq!(s.saved_phase[3], FALSE);
        assert_eq!(s.rephase_index, 0);
    }

    #[test]
    fn test_rephase_advances_index_on_each_call() {
        let config = SolverConfig {
            rephase: true,
            ..focused_stable_config()
        };
        let mut s = make_solver_with_config(1, vec![], &config);

        s.apply_rephase();
        assert_eq!(s.rephase_index, 1);
        s.apply_rephase();
        assert_eq!(s.rephase_index, 2);
        s.apply_rephase();
        assert_eq!(s.rephase_index, 0);
        assert_eq!(s.stats.rephases, 3);
    }

    #[test]
    fn test_rephase_cycle_excludes_walk_by_default() {
        let config = SolverConfig {
            rephase: true,
            ..focused_stable_config()
        };
        let mut s = make_solver_with_config(2, vec![], &config);
        s.best_phase[1] = TRUE;
        s.best_phase[2] = FALSE;

        s.apply_rephase();
        s.apply_rephase();
        s.apply_rephase();
        s.saved_phase[1] = FALSE;
        s.saved_phase[2] = TRUE;
        s.apply_rephase();

        assert_eq!(s.saved_phase[1], TRUE);
        assert_eq!(s.saved_phase[2], FALSE);
        assert_eq!(s.rephase_index, 1);
        assert_eq!(s.stats.rephases, 4);
    }

    #[test]
    fn test_rephase_only_runs_on_due_stable_restart() {
        let config = SolverConfig {
            rephase: true,
            rephase_init_conflicts: 3,
            ..focused_stable_config()
        };
        let mut s = make_solver_with_config(2, vec![], &config);
        s.best_phase[1] = TRUE;
        s.stats.conflicts = 3;
        s.decide(1);
        s.restart_pending = true;

        assert!(s.perform_restart_if_pending());
        assert_eq!(
            s.stats.rephases, 0,
            "focused-mode restarts must not rephase"
        );

        s.search_mode = SearchMode::Stable;
        s.stats.conflicts = 2;
        s.decide(1);
        s.restart_pending = true;
        assert!(s.perform_restart_if_pending());
        assert_eq!(
            s.stats.rephases, 0,
            "stable restarts before the schedule must not rephase"
        );

        s.stats.conflicts = 3;
        s.decide(1);
        s.restart_pending = true;
        assert!(s.perform_restart_if_pending());

        assert_eq!(s.stats.rephases, 1);
        assert_eq!(s.saved_phase[1], TRUE);
        assert_eq!(s.rephase_index, 1);
        assert_eq!(s.rephase_at_conflicts, 6);
    }

    #[test]
    fn test_phase_saving_survives_backtrack() {
        let config = SolverConfig {
            phase_policy: PhasePolicy::Saved,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(1, vec![], &config);

        s.decide(-1);
        s.backtrack(0);

        assert_eq!(s.assignment[1], UNASSIGNED);
        assert_eq!(s.saved_phase[1], FALSE);
        assert_eq!(s.pick_branch_lit(), Some(-1));
        assert_eq!(s.stats.phase_saved_used, 1);
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
        let conflict = s.propagate().expect("expected conflict after propagation");
        let (learned_clause, backtrack_level) = s.analyze_conflict(conflict);

        assert_eq!(learned_clause, vec![-5]);
        assert_eq!(backtrack_level, 0);
        assert!(s.iterating);

        let mut bumped_vars = s.scratch_bumped_vars.clone();
        bumped_vars.sort_unstable();
        assert_eq!(bumped_vars, vec![2, 3, 4, 5, 6]);
    }

    #[test]
    fn test_conflict_analysis_clears_iterating_for_non_unit_learned_clause() {
        let mut s = make_solver(2, vec![vec![-1, -2]]);
        s.iterating = true;

        s.decide(1);
        s.decide(2);
        let conflict = s.propagate().expect("expected conflict after decisions");
        let (learned_clause, backtrack_level) = s.analyze_conflict(conflict);

        assert_eq!(learned_clause.len(), 2);
        assert_eq!(backtrack_level, 1);
        assert!(!s.iterating);
    }

    #[test]
    fn test_iterating_skips_due_conflict_mode_switch() {
        let config = focused_stable_config();
        let mut s = make_solver_with_config(2, vec![], &config);
        s.iterating = true;
        s.stats.conflicts = s.mode_switch_at_conflicts;

        s.maybe_switch_search_mode_after_conflict();

        assert_eq!(s.search_mode, SearchMode::Focused);
        assert_eq!(s.mode_switches, 0);
        assert!(s.iterating);
    }

    #[test]
    fn test_iterating_skips_post_propagation_restart_and_tick_switch_once() {
        let config = focused_stable_tick_config();
        let mut s = make_solver_with_config(2, vec![], &config);
        s.decide(1);
        s.iterating = true;
        s.restart_pending = true;
        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.stats.search_ticks = s.mode_switch_at_ticks;

        assert!(!s.run_post_propagation_scheduling());

        assert!(!s.iterating);
        assert_eq!(s.search_mode, SearchMode::Focused);
        assert_eq!(s.mode_switches, 0);
        assert!(s.restart_pending);
        assert_eq!(s.current_level(), 1);
    }

    #[test]
    fn test_iterating_defers_restart_until_next_scheduling_pass() {
        let mut s = make_solver(2, vec![]);
        s.decide(1);
        s.iterating = true;
        s.restart_pending = true;

        assert!(!s.run_post_propagation_scheduling());
        assert!(s.restart_pending);
        assert_eq!(s.current_level(), 1);

        assert!(s.run_post_propagation_scheduling());
        assert!(!s.restart_pending);
        assert_eq!(s.current_level(), 0);
    }

    #[test]
    fn test_conflict_analysis_bumps_learned_reason_clause_activity() {
        let mut s = make_solver(3, vec![vec![-1, 2]]);
        let learned_reason = s.add_clause(vec![-2, 3]);
        let learned_conflict = s.add_clause(vec![-2, -3]);

        s.decide(1);
        let conflict = s.propagate().expect("expected conflict after propagation");
        assert_eq!(conflict, Conflict::Clause(learned_conflict));
        let (learned_clause, backtrack_level) = s.analyze_conflict(conflict);

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
    fn test_kissat_logn_matches_focused_restart_schedule() {
        assert_eq!(Solver::kissat_logn(0), 0);
        assert_eq!(Solver::kissat_logn(1), 1);
        assert_eq!(Solver::kissat_logn(2), 2);
        assert_eq!(Solver::kissat_logn(3), 2);
        assert_eq!(Solver::kissat_logn(4), 3);
        assert_eq!(Solver::kissat_logn(7), 3);
        assert_eq!(Solver::kissat_logn(8), 4);
    }

    #[test]
    fn test_reluctant_sequence_matches_expected_prefix() {
        let mut reluctant = Reluctant::new();
        let mut values = Vec::new();
        for _ in 0..7 {
            values.push(reluctant.current());
            reluctant.advance();
        }

        assert_eq!(values, vec![1, 1, 2, 1, 1, 2, 4]);
    }

    #[test]
    fn test_reluctant_restart_policy_schedules_restart() {
        let config = SolverConfig {
            restart_policy: RestartPolicy::Reluctant,
            ..single_mode_config()
        };
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        s.decide(1);

        s.note_conflict();

        assert!(s.restart_pending);
        assert_eq!(s.stats.reluctant_restarts, 1);
        assert_eq!(s.restart_conflicts_since_last, 0);
        assert_eq!(s.reluctant.current(), 1);
    }

    fn focused_stable_config() -> SolverConfig {
        SolverConfig {
            use_lbd: true,
            search_mode_policy: SearchModePolicy::FocusedStable,
            mode_use_ticks: false,
            mode_init_conflicts: 2,
            mode_interval_scale: 1.0,
            ..SolverConfig::default()
        }
    }

    fn focused_stable_vmtf_config() -> SolverConfig {
        SolverConfig {
            vmtf: VmtfMode::FocusedOnly,
            ..focused_stable_config()
        }
    }

    fn focused_stable_tick_config() -> SolverConfig {
        SolverConfig {
            mode_use_ticks: true,
            ..focused_stable_config()
        }
    }

    fn single_mode_config() -> SolverConfig {
        SolverConfig {
            search_mode_policy: SearchModePolicy::Single,
            mode_use_ticks: false,
            ..SolverConfig::default()
        }
    }

    fn single_mode_vmtf_config() -> SolverConfig {
        SolverConfig {
            vmtf: VmtfMode::Single,
            ..single_mode_config()
        }
    }

    #[test]
    fn test_vmtf_recently_analyzed_variable_pops_first() {
        let config = focused_stable_vmtf_config();
        let mut s = make_solver_with_config(4, vec![], &config);

        s.vmtf_stamp_analyzed_var(2);

        assert_eq!(s.pick_branch_lit(), Some(-2));
    }

    #[test]
    fn test_vmtf_search_pointer_resets_on_relevant_backtrack() {
        let config = focused_stable_vmtf_config();
        let mut s = make_solver_with_config(4, vec![], &config);

        s.vmtf_stamp_analyzed_var(2);
        assert_eq!(s.pick_branch_lit(), Some(-2));
        s.decide(-2);
        s.decide(-3);
        s.vmtf_stamp_analyzed_var(3);
        assert_ne!(s.vmtf_queue.as_ref().unwrap().search_for_test(), 3);

        s.backtrack(1);

        assert_eq!(s.vmtf_queue.as_ref().unwrap().search_for_test(), 3);
        assert_eq!(s.pick_branch_lit(), Some(-3));
    }

    #[test]
    fn test_vmtf_does_not_pick_assigned_variables() {
        let config = focused_stable_vmtf_config();
        let mut s = make_solver_with_config(3, vec![], &config);

        assert!(s.enqueue(-3, ReasonRef::None));

        assert_eq!(s.pick_branch_lit(), Some(-2));
    }

    #[test]
    fn test_vmtf_does_not_pick_eliminated_variables() {
        let config = focused_stable_vmtf_config();
        let mut s = make_solver_with_config(3, vec![], &config);

        s.eliminated[3] = true;
        s.decision_var[3] = false;

        assert_eq!(s.pick_branch_lit(), Some(-2));
    }

    #[test]
    fn test_vmtf_tie_break_is_deterministic() {
        let config = focused_stable_vmtf_config();
        let first = decision_prefix(make_solver_with_config(4, vec![], &config), 4);
        let second = decision_prefix(make_solver_with_config(4, vec![], &config), 4);

        assert_eq!(first, second);
        assert_eq!(first, vec![-4, -3, -2, -1]);
    }

    #[test]
    fn test_vmtf_keeps_stable_mode_on_vsids_heap() {
        let config = focused_stable_vmtf_config();
        let mut s = make_solver_with_config(3, vec![], &config);
        s.search_mode = SearchMode::Stable;
        s.activity[1] = 10.0;
        s.activity[2] = 1.0;
        s.activity[3] = 0.5;
        s.rebuild_branch_queue();

        assert_eq!(s.pick_branch_lit(), Some(-1));
    }

    #[test]
    fn test_vmtf_single_mode_activates_without_focused_stable() {
        let config = single_mode_vmtf_config();
        let mut s = make_solver_with_config(4, vec![], &config);

        assert_eq!(s.search_mode, SearchMode::Stable);
        assert!(s.vmtf_branching_active());

        s.vmtf_stamp_analyzed_var(2);

        assert_eq!(s.pick_branch_lit(), Some(-2));
    }

    #[test]
    fn test_vmtf_single_mode_conflict_bump_updates_queue() {
        let config = single_mode_vmtf_config();
        let mut s = make_solver_with_config(4, vec![], &config);

        s.scratch_bumped_vars.push(2);
        s.bump_analyzed_variable_activity();

        assert_eq!(s.pick_branch_lit(), Some(-2));
    }

    #[test]
    fn test_mode_starts_focused_default() {
        let config = focused_stable_config();
        let s = make_solver_with_config(2, vec![vec![1, 2]], &config);

        assert_eq!(s.search_mode, SearchMode::Focused);
        assert_eq!(s.mode_start_conflicts, 0);
        assert_eq!(s.mode_start_decisions, 0);
        assert_eq!(s.mode_interval, 2);
        assert_eq!(s.mode_switch_at_conflicts, 2);
        assert_eq!(s.effective_restart_policy(), RestartPolicy::KissatEma);
        assert_eq!(s.effective_phase_policy(), PhasePolicy::Saved);
    }

    #[test]
    fn test_focused_stable_phase_policy_override_table() {
        for (configured, focused_effective) in [
            (PhasePolicy::Legacy, PhasePolicy::Saved),
            (PhasePolicy::Saved, PhasePolicy::Saved),
            (PhasePolicy::TargetThenSaved, PhasePolicy::TargetThenSaved),
            (
                PhasePolicy::BestThenTargetThenSaved,
                PhasePolicy::TargetThenSaved,
            ),
        ] {
            let config = SolverConfig {
                use_lbd: true,
                search_mode_policy: SearchModePolicy::FocusedStable,
                phase_policy: configured,
                ..SolverConfig::default()
            };
            let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);

            s.search_mode = SearchMode::Focused;
            assert_eq!(s.effective_phase_policy(), focused_effective);

            s.search_mode = SearchMode::Stable;
            assert_eq!(
                s.effective_phase_policy(),
                PhasePolicy::BestThenTargetThenSaved
            );
        }

        let single = SolverConfig {
            phase_policy: PhasePolicy::TargetThenSaved,
            ..SolverConfig::default()
        };
        let s = make_solver_with_config(2, vec![vec![1, 2]], &single);
        assert_eq!(s.effective_phase_policy(), PhasePolicy::TargetThenSaved);
    }

    #[test]
    fn test_mode_switch_after_budget() {
        let config = focused_stable_config();
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);

        s.stats.conflicts = 1;
        s.maybe_switch_search_mode();
        assert_eq!(s.search_mode, SearchMode::Focused);

        s.stats.conflicts = 2;
        s.maybe_switch_search_mode();

        assert_eq!(s.search_mode, SearchMode::Stable);
        assert_eq!(s.mode_switches, 1);
        assert_eq!(s.stats.mode_switches, 1);
        assert_eq!(s.mode_start_conflicts, 2);
        assert!(s.mode_switch_at_conflicts > 2);
        assert_eq!(s.effective_restart_policy(), RestartPolicy::Reluctant);
        assert_eq!(
            s.effective_phase_policy(),
            PhasePolicy::BestThenTargetThenSaved
        );
    }

    #[test]
    fn test_mode_switch_back_preserves_heap() {
        let config = focused_stable_config();
        let mut s = make_solver_with_config(3, vec![], &config);
        s.activity[1] = 3.0;
        s.activity[2] = 2.0;
        s.rebuild_branch_queue();
        let heap_before = s.branch_heap.clone();
        let pos_before = s.branch_pos.clone();

        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();
        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();

        assert_eq!(s.search_mode, SearchMode::Focused);
        assert_eq!(s.branch_heap, heap_before);
        assert_eq!(s.branch_pos, pos_before);
    }

    #[test]
    fn test_mode_switch_to_stable_refreshes_vsids_heap_scores() {
        let config = focused_stable_vmtf_config();
        let mut s = make_solver_with_config(3, vec![], &config);
        s.activity[1] = 1.0;
        s.activity[2] = 3.0;
        s.activity[3] = 2.0;
        s.rebuild_branch_queue();
        assert_eq!(s.branch_heap[0] as usize, 2);

        s.activity[1] = 10.0;
        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();

        assert_eq!(s.search_mode, SearchMode::Stable);
        assert_eq!(s.branch_heap[0] as usize, 1);
        assert_eq!(s.pick_branch_lit(), Some(-1));
    }

    #[test]
    fn test_mode_switch_preserves_target_phase() {
        let config = focused_stable_config();
        let mut s = make_solver_with_config(2, vec![], &config);
        s.target_phase[1] = TRUE;
        s.target_phase[2] = FALSE;
        s.target_assigned = 2;

        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();

        assert_eq!(s.search_mode, SearchMode::Stable);
        assert_eq!(s.target_assigned, 2);
        assert_eq!(s.target_phase[1], TRUE);
        assert_eq!(s.target_phase[2], FALSE);
    }

    #[test]
    fn test_mode_switch_resets_restart_pending() {
        let config = focused_stable_config();
        let mut s = make_solver_with_config(2, vec![], &config);
        s.restart_pending = true;
        s.restart_conflicts = 7;
        s.restart_conflicts_since_last = 9;

        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();

        assert!(!s.restart_pending);
        assert_eq!(s.restart_conflicts, 0);
        assert_eq!(s.restart_conflicts_since_last, 0);
    }

    #[test]
    fn test_mode_switch_resets_lbd_ema_only_when_entering_focused() {
        let config = focused_stable_config();
        let mut s = make_solver_with_config(1, vec![], &config);
        s.restart_fast_lbd.update(13.0);
        s.restart_slow_lbd.update(10.0);

        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();

        assert_eq!(s.search_mode, SearchMode::Stable);
        assert!(s.restart_fast_lbd.initialized);
        assert!(s.restart_slow_lbd.initialized);
        assert_eq!(s.restart_fast_lbd.value, 13.0);
        assert_eq!(s.restart_slow_lbd.value, 10.0);

        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();

        assert_eq!(s.search_mode, SearchMode::Focused);
        assert!(!s.restart_fast_lbd.initialized);
        assert!(!s.restart_slow_lbd.initialized);
        assert_eq!(s.restart_fast_lbd.value, 0.0);
        assert_eq!(s.restart_slow_lbd.value, 0.0);
    }

    #[test]
    fn test_mode_tick_accounting_is_opt_in() {
        let clauses = vec![vec![1, 2]];
        let mut legacy = make_solver_with_config(2, clauses.clone(), &focused_stable_config());
        legacy.decide(-1);
        assert_eq!(legacy.propagate(), None);
        assert_eq!(legacy.stats.search_ticks, 0);

        let mut ticked = make_solver_with_config(2, clauses, &focused_stable_tick_config());
        ticked.decide(-1);
        assert_eq!(ticked.propagate(), None);
        assert!(ticked.stats.search_ticks > 0);
    }

    #[test]
    fn test_stable_mode_switch_uses_ticks_when_enabled() {
        let config = focused_stable_tick_config();
        let mut s = make_solver_with_config(2, vec![], &config);

        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();
        assert_eq!(s.search_mode, SearchMode::Stable);
        let stable_tick_limit = s.mode_switch_at_ticks;

        s.stats.conflicts = u64::MAX;
        s.stats.search_ticks = stable_tick_limit.saturating_sub(1);
        s.maybe_switch_search_mode();
        assert_eq!(s.search_mode, SearchMode::Stable);

        s.stats.search_ticks = stable_tick_limit;
        s.maybe_switch_search_mode();
        assert_eq!(s.search_mode, SearchMode::Focused);
    }

    #[test]
    fn test_tick_mode_switches_after_nonconflicting_propagation() {
        let config = focused_stable_tick_config();
        let mut s = make_solver_with_config(2, vec![], &config);
        s.search_mode = SearchMode::Stable;
        s.mode_switch_at_ticks = s.stats.search_ticks;

        let mut proof_log = ProofLog::disabled();
        assert!(s.solve_with_proof(&mut proof_log, &SolverConfig::default()));

        assert_eq!(s.search_mode, SearchMode::Focused);
        assert_eq!(s.stats.mode_switches, 1);
    }

    #[test]
    fn test_tick_mode_uses_kissat_nlogpown_for_focused_intervals() {
        let config = SolverConfig {
            mode_init_conflicts: 10,
            mode_use_ticks: true,
            ..focused_stable_config()
        };
        let mut s = make_solver_with_config(2, vec![], &config);

        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();
        s.stats.search_ticks = s.mode_switch_at_ticks;
        s.maybe_switch_search_mode();
        assert_eq!(s.search_mode, SearchMode::Focused);
        assert_eq!(s.mode_interval, 10);

        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.stats.search_ticks = s.stats.search_ticks.saturating_add(7);
        s.maybe_switch_search_mode();
        s.stats.search_ticks = s.mode_switch_at_ticks;
        s.maybe_switch_search_mode();

        let expected = (10.0 * Solver::nlogpown(2, 4)) as u64;
        assert_eq!(s.search_mode, SearchMode::Focused);
        assert_eq!(s.mode_interval, expected);
        assert!(s.mode_interval > 10);
    }

    #[test]
    fn test_tick_mode_resets_all_restart_emas_on_every_switch() {
        let config = focused_stable_tick_config();
        let mut s = make_solver_with_config(1, vec![], &config);
        s.restart_fast_lbd.update(13.0);
        s.restart_slow_lbd.update(10.0);
        s.restart_fast_level.update(9.0);
        s.restart_slow_level.update(4.0);

        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();

        assert_eq!(s.search_mode, SearchMode::Stable);
        assert!(!s.restart_fast_lbd.initialized);
        assert!(!s.restart_slow_lbd.initialized);
        assert!(!s.restart_fast_level.initialized);
        assert!(!s.restart_slow_level.initialized);

        s.restart_fast_lbd.update(8.0);
        s.restart_slow_lbd.update(5.0);
        s.restart_fast_level.update(6.0);
        s.restart_slow_level.update(3.0);
        s.stats.search_ticks = s.mode_switch_at_ticks;
        s.maybe_switch_search_mode();

        assert_eq!(s.search_mode, SearchMode::Focused);
        assert!(!s.restart_fast_lbd.initialized);
        assert!(!s.restart_slow_lbd.initialized);
        assert!(!s.restart_fast_level.initialized);
        assert!(!s.restart_slow_level.initialized);
    }

    #[test]
    fn test_mode_stats_count_switches() {
        let config = focused_stable_config();
        let mut s = make_solver_with_config(3, vec![], &config);

        s.decide(1);
        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();
        s.decide(2);
        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();

        assert_eq!(s.mode_switches, 2);
        assert_eq!(s.stats.mode_switches, 2);
        assert_eq!(s.stats.decisions_focused, 1);
        assert_eq!(s.stats.decisions_stable, 1);
        assert_eq!(s.mode_start_decisions, 2);
    }

    #[test]
    fn test_mode_wall_time_records_focused_and_stable_segments() {
        let config = focused_stable_config();
        let mut s = make_solver_with_config(2, vec![], &config);

        s.begin_search_mode_timing();
        s.mode_wall_start = Instant::now()
            .checked_sub(Duration::from_millis(10))
            .expect("test duration should be representable");
        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();

        assert_eq!(s.search_mode, SearchMode::Stable);
        assert!(s.stats.seconds_focused > 0.0);
        assert_eq!(s.stats.seconds_stable, 0.0);

        s.mode_wall_start = Instant::now()
            .checked_sub(Duration::from_millis(10))
            .expect("test duration should be representable");
        let search_start = Instant::now()
            .checked_sub(Duration::from_millis(20))
            .expect("test duration should be representable");
        s.finish_search_timing(search_start);

        assert!(s.stats.seconds_stable > 0.0);
        assert!(!s.mode_wall_active);
    }

    #[test]
    fn test_mode_specific_lbd_conflict_and_decision_stats() {
        let config = focused_stable_config();
        let mut s = make_solver_with_config(3, vec![], &config);

        s.decide(1);
        s.record_search_conflict_mode();
        s.record_current_mode_lbd(4);

        s.stats.conflicts = s.mode_switch_at_conflicts;
        s.maybe_switch_search_mode();
        s.decide(2);
        s.record_search_conflict_mode();
        s.record_current_mode_lbd(10);

        assert_eq!(s.stats.conflicts_focused, 1);
        assert_eq!(s.stats.conflicts_stable, 1);
        assert_eq!(s.stats.lbd_count_focused, 1);
        assert_eq!(s.stats.lbd_sum_focused, 4);
        assert_eq!(s.stats.lbd_count_stable, 1);
        assert_eq!(s.stats.lbd_sum_stable, 10);
        assert_eq!(s.stats.decision_level_sum_focused, 1);
        assert_eq!(s.stats.decision_level_sum_stable, 2);
    }

    #[test]
    fn test_no_restart_at_level_zero() {
        let config = SolverConfig {
            use_lbd: true,
            restart_policy: RestartPolicy::KissatEma,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        s.restart_conflicts_since_last = s.restart_min_conflicts;
        s.restart_fast_lbd.update(10.0);
        s.restart_slow_lbd.update(1.0);

        assert_eq!(s.current_level(), 0);
        assert!(!s.should_restart());
    }

    #[test]
    fn test_lbd_ema_fast_reacts_faster_than_slow() {
        let mut fast = MovingAverage::new(RESTART_FAST_ALPHA);
        let mut slow = MovingAverage::new(RESTART_SLOW_ALPHA);

        fast.update(4.0);
        slow.update(4.0);
        fast.update(20.0);
        slow.update(20.0);

        assert!(fast.value > slow.value);
        assert!((fast.value - 4.0) > (slow.value - 4.0));
    }

    #[test]
    fn test_restart_triggers_when_fast_exceeds_slow_by_margin() {
        let config = SolverConfig {
            use_lbd: true,
            restart_policy: RestartPolicy::KissatEma,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        s.decide(1);
        s.restart_fast_lbd.update(13.0);
        s.restart_slow_lbd.update(10.0);
        s.restart_conflicts_since_last = s.restart_min_conflicts - 1;
        s.stats.conflicts = 100;
        s.last_conflict_lbd = 13;

        s.note_conflict();

        assert!(s.restart_pending);
        assert_eq!(s.restart_conflicts_since_last, 0);
        assert_eq!(s.stats.glucose_restarts, 1);
    }

    #[test]
    fn test_focused_restart_interval_grows_with_focused_restarts() {
        let config = focused_stable_config();
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        s.decide(1);
        s.stats.conflicts = 100;
        s.restart_fast_lbd.update(13.0);
        s.restart_slow_lbd.update(10.0);
        s.restart_conflicts_since_last = s.restart_min_conflicts - 1;
        s.last_conflict_lbd = 13;

        s.note_conflict();

        assert!(s.restart_pending);
        assert_eq!(s.stats.glucose_restarts, 1);
        assert_eq!(s.stats.focused_restarts, 1);
        assert_eq!(s.restart_min_conflicts, KISSAT_EMA_RESTART_MIN_CONFLICTS);

        s.restart_pending = false;
        s.stats.conflicts = s.restart_next_check_conflict;
        s.restart_fast_lbd.update(13.0);
        s.restart_slow_lbd.update(10.0);
        s.restart_conflicts_since_last = s.restart_min_conflicts - 1;
        s.last_conflict_lbd = 13;

        s.note_conflict();

        assert!(s.restart_pending);
        assert_eq!(s.stats.glucose_restarts, 2);
        assert_eq!(s.stats.focused_restarts, 2);
        assert_eq!(
            s.restart_min_conflicts,
            KISSAT_EMA_RESTART_MIN_CONFLICTS + 1
        );
    }

    #[test]
    fn test_single_mode_ema_restart_does_not_grow_focused_interval() {
        let config = SolverConfig {
            use_lbd: true,
            restart_policy: RestartPolicy::KissatEma,
            search_mode_policy: SearchModePolicy::Single,
            mode_use_ticks: false,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        s.decide(1);
        s.stats.conflicts = 100;
        s.restart_fast_lbd.update(13.0);
        s.restart_slow_lbd.update(10.0);
        s.restart_conflicts_since_last = s.restart_min_conflicts - 1;
        s.last_conflict_lbd = 13;

        s.note_conflict();

        assert!(s.restart_pending);
        assert_eq!(s.stats.glucose_restarts, 1);
        assert_eq!(s.stats.focused_restarts, 0);
        assert_eq!(s.restart_min_conflicts, KISSAT_EMA_RESTART_MIN_CONFLICTS);
    }

    #[test]
    fn test_blocking_restart_suppresses_when_level_high() {
        let config = SolverConfig {
            use_lbd: true,
            restart_policy: RestartPolicy::KissatEma,
            restart_block_margin: 1.4,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        s.decide(1);
        s.restart_fast_lbd.update(13.0);
        s.restart_slow_lbd.update(10.0);
        s.restart_fast_level.update(15.0);
        s.restart_slow_level.update(10.0);
        s.restart_conflicts_since_last = s.restart_min_conflicts;
        s.stats.conflicts = 100;

        assert!(s.kissat_ema_restart_candidate_due());
        assert!(s.restart_blocked_by_level_ema());
        assert!(!s.should_restart());
    }

    #[test]
    fn test_blocking_restart_no_effect_when_level_low() {
        let config = SolverConfig {
            use_lbd: true,
            restart_policy: RestartPolicy::KissatEma,
            restart_block_margin: 1.4,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        s.decide(1);
        s.restart_fast_lbd.update(13.0);
        s.restart_slow_lbd.update(10.0);
        s.restart_fast_level.update(12.0);
        s.restart_slow_level.update(10.0);
        s.restart_conflicts_since_last = s.restart_min_conflicts;
        s.stats.conflicts = 100;

        assert!(s.kissat_ema_restart_candidate_due());
        assert!(!s.restart_blocked_by_level_ema());
        assert!(s.should_restart());
    }

    #[test]
    fn test_blocking_restart_default_margin_disables_blocker() {
        let config = SolverConfig {
            use_lbd: true,
            restart_policy: RestartPolicy::KissatEma,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        s.decide(1);
        s.restart_fast_lbd.update(13.0);
        s.restart_slow_lbd.update(10.0);
        s.restart_fast_level.update(100.0);
        s.restart_slow_level.update(1.0);
        s.restart_conflicts_since_last = s.restart_min_conflicts;
        s.stats.conflicts = 100;

        assert_eq!(s.restart_block_margin, 0.0);
        assert!(s.kissat_ema_restart_candidate_due());
        assert!(!s.restart_blocked_by_level_ema());
        assert!(s.should_restart());
    }

    #[test]
    fn test_blocking_restart_uninitialized_emas_do_not_suppress() {
        let config = SolverConfig {
            use_lbd: true,
            restart_policy: RestartPolicy::KissatEma,
            restart_block_margin: 1.4,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        s.decide(1);
        s.restart_fast_lbd.update(13.0);
        s.restart_slow_lbd.update(10.0);
        s.restart_conflicts_since_last = s.restart_min_conflicts;
        s.stats.conflicts = 100;

        assert!(s.kissat_ema_restart_candidate_due());
        assert!(!s.restart_fast_level.initialized);
        assert!(!s.restart_slow_level.initialized);
        assert!(!s.restart_blocked_by_level_ema());
        assert!(s.should_restart());
    }

    #[test]
    fn test_blocking_restart_disabled_for_legacy_luby_and_reluctant() {
        let mut luby =
            make_solver_with_config(2, vec![vec![1, 2], vec![-1, -2]], &single_mode_config());
        luby.restart_fast_level.update(20.0);
        luby.restart_slow_level.update(10.0);
        luby.restart_unit = 1;
        luby.restart_conflict_limit = 1;
        luby.note_conflict();

        assert!(luby.restart_pending);
        assert_eq!(luby.stats.luby_restarts, 1);
        assert_eq!(luby.stats.restarts_blocked_by_level, 0);

        let config = SolverConfig {
            restart_policy: RestartPolicy::Reluctant,
            ..single_mode_config()
        };
        let mut reluctant = make_solver_with_config(2, vec![vec![1, 2]], &config);
        reluctant.decide(1);
        reluctant.restart_fast_level.update(20.0);
        reluctant.restart_slow_level.update(10.0);
        reluctant.note_conflict();

        assert!(reluctant.restart_pending);
        assert_eq!(reluctant.stats.reluctant_restarts, 1);
        assert_eq!(reluctant.stats.restarts_blocked_by_level, 0);
    }

    #[test]
    fn test_blocking_restart_counter_increments_on_suppression() {
        let config = SolverConfig {
            use_lbd: true,
            restart_policy: RestartPolicy::KissatEma,
            restart_block_margin: 1.4,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        s.decide(1);
        s.restart_fast_lbd.update(13.0);
        s.restart_slow_lbd.update(10.0);
        s.restart_fast_level.update(20.0);
        s.restart_slow_level.update(10.0);
        s.restart_conflicts_since_last = s.restart_min_conflicts - 1;
        s.stats.conflicts = 100;
        s.last_conflict_lbd = 13;

        s.note_conflict();

        assert!(!s.restart_pending);
        assert_eq!(s.restart_conflicts_since_last, s.restart_min_conflicts);
        assert_eq!(s.stats.glucose_restarts, 0);
        assert_eq!(s.stats.restarts_blocked_by_level, 1);
    }

    #[test]
    fn test_restart_blocked_during_min_interval() {
        let config = SolverConfig {
            use_lbd: true,
            restart_policy: RestartPolicy::KissatEma,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(2, vec![vec![1, 2]], &config);
        s.decide(1);
        s.restart_fast_lbd.update(13.0);
        s.restart_slow_lbd.update(10.0);
        s.restart_conflicts_since_last = s.restart_min_conflicts - 2;
        s.stats.conflicts = 100;
        s.last_conflict_lbd = 13;

        s.note_conflict();

        assert!(!s.restart_pending);
        assert_eq!(s.restart_conflicts_since_last, s.restart_min_conflicts - 1);
        assert_eq!(s.stats.glucose_restarts, 0);
    }

    #[test]
    fn test_restart_policy_legacy_unchanged_when_selected() {
        let mut s =
            make_solver_with_config(2, vec![vec![1, 2], vec![-1, -2]], &single_mode_config());
        s.restart_min_conflicts = 50;
        s.restart_unit = 2;
        s.restart_luby_index = 1;
        s.restart_conflict_limit = 2;

        s.note_conflict();
        assert_eq!(s.restart_conflicts, 1);
        assert!(!s.restart_pending);

        s.note_conflict();
        assert_eq!(s.restart_conflicts, 0);
        assert!(s.restart_pending);
        assert_eq!(s.restart_luby_index, 2);
        assert_eq!(s.stats.luby_restarts, 1);
        assert_eq!(s.stats.glucose_restarts, 0);
        assert_eq!(s.stats.restarts_blocked_by_level, 0);
    }

    #[test]
    fn test_conflict_budget_schedules_restart_and_advances_luby_window() {
        let mut s =
            make_solver_with_config(2, vec![vec![1, 2], vec![-1, -2]], &single_mode_config());
        s.restart_unit = 2;
        s.restart_luby_index = 1;
        s.restart_conflict_limit = 2;
        assert_eq!(s.stats.luby_restarts, 0);

        s.note_conflict();
        assert_eq!(s.restart_conflicts, 1);
        assert!(!s.restart_pending);
        assert_eq!(s.restart_conflict_limit, 2);
        assert_eq!(s.stats.luby_restarts, 0);

        s.note_conflict();
        assert_eq!(s.restart_conflicts, 0);
        assert!(s.restart_pending);
        assert_eq!(s.restart_luby_index, 2);
        assert_eq!(s.restart_conflict_limit, 2);
        assert_eq!(s.stats.luby_restarts, 1);

        s.restart_pending = false;
        s.note_conflict();
        s.note_conflict();
        assert!(s.restart_pending);
        assert_eq!(s.restart_luby_index, 3);
        assert_eq!(s.restart_conflict_limit, 4);
        assert_eq!(s.stats.luby_restarts, 2);
    }

    #[test]
    fn test_restart_backtracks_and_preserves_root_units() {
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
    fn test_trail_reuse_stable_keeps_high_score_prefix() {
        let config = SolverConfig {
            restart_reuse_trail: true,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(4, vec![], &config);
        s.decide(1);
        s.decide(2);
        s.decide(3);
        s.activity[1] = 10.0;
        s.activity[2] = 8.0;
        s.activity[3] = 1.0;
        s.activity[4] = 5.0;
        s.rebuild_branch_queue();

        assert_eq!(s.restart_reuse_trail_level(), 2);

        s.restart_pending = true;
        assert!(s.perform_restart_if_pending());

        assert_eq!(s.current_level(), 2);
        assert_eq!(s.assignment[1], TRUE);
        assert_eq!(s.assignment[2], TRUE);
        assert_eq!(s.assignment[3], UNASSIGNED);
        assert_eq!(s.stats.restarts, 1);
        assert_eq!(s.stats.restarts_reused_trails, 1);
        assert_eq!(s.stats.restarts_reused_levels, 2);
    }

    #[test]
    fn test_trail_reuse_focused_keeps_high_stamp_prefix() {
        let config = SolverConfig {
            restart_reuse_trail: true,
            ..focused_stable_vmtf_config()
        };
        let mut s = make_solver_with_config(4, vec![], &config);
        s.decide(1);
        s.decide(2);
        s.decide(3);
        s.vmtf_stamp_analyzed_var(1);
        s.vmtf_stamp_analyzed_var(2);

        assert_eq!(
            s.vmtf_queue.as_ref().unwrap().stamp_for_test(1),
            s.vmtf_queue.as_ref().unwrap().stamp_for_test(4) + 1
        );
        assert_eq!(s.restart_reuse_trail_level(), 2);

        s.restart_pending = true;
        assert!(s.perform_restart_if_pending());

        assert_eq!(s.current_level(), 2);
        assert_eq!(s.assignment[1], TRUE);
        assert_eq!(s.assignment[2], TRUE);
        assert_eq!(s.assignment[3], UNASSIGNED);
        assert_eq!(s.stats.restarts_reused_trails, 1);
        assert_eq!(s.stats.restarts_reused_levels, 2);
    }

    #[test]
    fn test_trail_reuse_does_not_reuse_at_zero() {
        let config = SolverConfig {
            restart_reuse_trail: true,
            ..SolverConfig::default()
        };
        let mut s = make_solver_with_config(2, vec![], &config);

        assert_eq!(s.restart_reuse_trail_level(), 0);
        s.restart_pending = true;
        assert!(!s.perform_restart_if_pending());
        assert_eq!(s.stats.restarts, 0);
        assert_eq!(s.stats.restarts_reused_trails, 0);
        assert_eq!(s.stats.restarts_reused_levels, 0);
    }
}
