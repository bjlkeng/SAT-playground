//! Solver counters and future trace/stat output boundary.
//!
//! Task 0.1 only moves the existing counter bag out of `main.rs`. Task 0.3 and
//! later milestone gates will add structured JSON output, config hashes, and
//! profile-validation metadata here.

#[derive(Clone, Default)]
pub(crate) struct SolverStats {
    pub(crate) conflicts: u64,
    pub(crate) propagations: u64,
    pub(crate) decisions: u64,
    pub(crate) restarts: u64,
    pub(crate) simplifications: u64,
    pub(crate) reduce_db_calls: u64,
    pub(crate) deleted_clauses: u64,
    pub(crate) garbage_collections: u64,
    pub(crate) learned_clauses: u64,
    pub(crate) lbd_computed: u64,
    pub(crate) lbd_sum: u64,
    pub(crate) lbd_max: u32,
    pub(crate) preprocess_eliminated_vars: u64,
    pub(crate) preprocess_resolvents: u64,
    pub(crate) preprocess_subsumed_clauses: u64,
    pub(crate) preprocess_strengthened_clauses: u64,
    pub(crate) bsr_runs: u64,
    pub(crate) bsr_seeded_clauses: u64,
    pub(crate) bsr_drivers: u64,
    pub(crate) bsr_clause_drivers: u64,
    pub(crate) bsr_root_drivers: u64,
    pub(crate) bsr_driver_lits: u64,
    pub(crate) bsr_best_occurs_sum: u64,
    pub(crate) bsr_best_occurs_max: u64,
    pub(crate) bsr_candidates_seen: u64,
    pub(crate) bsr_skip_self: u64,
    pub(crate) bsr_skip_deleted: u64,
    pub(crate) bsr_skip_limit: u64,
    pub(crate) bsr_relation_calls: u64,
    pub(crate) bsr_relation_len_reject: u64,
    pub(crate) bsr_relation_abstraction_reject: u64,
    pub(crate) bsr_relation_sorted_calls: u64,
    pub(crate) bsr_relation_nested_calls: u64,
    pub(crate) bsr_relation_subsumed: u64,
    pub(crate) bsr_relation_strengthen: u64,
    pub(crate) occurs_clean_calls: u64,
    pub(crate) occurs_clean_dirty_calls: u64,
    pub(crate) occurs_clean_membership_calls: u64,
    pub(crate) occurs_clean_entries_scanned: u64,
    pub(crate) occurs_clean_entries_removed: u64,
}
