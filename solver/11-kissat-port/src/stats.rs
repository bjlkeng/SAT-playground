//! Solver counters and structured trace/stat output boundary.
//!
//! The solver keeps these counters as ordinary fields so the hot path does not
//! depend on JSON formatting.  Machine-readable output is built only when
//! `SAT_STATS_JSON=on`.

use std::fs;
use std::io::Read;
use std::path::Path;

use crate::config::{FeatureMaturity, SolverConfig};
use crate::output::{ProofCompleteness, SolveStatus};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum GcReason {
    #[default]
    None,
    LearnedReduction,
    ArenaFragmentation,
    WatcherStaleness,
    EmergencyMemory,
}

impl GcReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::LearnedReduction => "learned-reduction",
            Self::ArenaFragmentation => "arena-fragmentation",
            Self::WatcherStaleness => "watcher-staleness",
            Self::EmergencyMemory => "emergency-memory",
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct SolverStats {
    pub(crate) conflicts: u64,
    pub(crate) propagations: u64,
    pub(crate) decisions: u64,
    pub(crate) restarts: u64,
    pub(crate) simplifications: u64,
    pub(crate) search_ticks: u64,
    pub(crate) reduce_db_calls: u64,
    pub(crate) deleted_clauses: u64,
    pub(crate) garbage_collections: u64,
    pub(crate) gc_last_reason: GcReason,
    pub(crate) gc_words_reclaimed: u64,
    pub(crate) gc_refs_rewritten: u64,
    pub(crate) learned_clauses: u64,
    pub(crate) learned_kept_tier1: u64,
    pub(crate) learned_kept_tier2: u64,
    pub(crate) learned_kept_tier3: u64,
    pub(crate) learned_collected: u64,
    pub(crate) focused_tier1_glue_limit: u64,
    pub(crate) focused_tier2_glue_limit: u64,
    pub(crate) stable_tier1_glue_limit: u64,
    pub(crate) stable_tier2_glue_limit: u64,
    pub(crate) focused_glue_used: Vec<u64>,
    pub(crate) stable_glue_used: Vec<u64>,
    pub(crate) lbd_computed: u64,
    pub(crate) lbd_sum: u64,
    pub(crate) lbd_max: u32,
    pub(crate) lbd_1: u64,
    pub(crate) lbd_2: u64,
    pub(crate) lbd_3_5: u64,
    pub(crate) lbd_6_10: u64,
    pub(crate) lbd_gt_10: u64,
    pub(crate) lbd_improved: u64,
    pub(crate) learned_size_sum: u64,
    pub(crate) learned_size_max: u64,
    pub(crate) luby_restarts: u64,
    pub(crate) glucose_restarts: u64,
    pub(crate) reluctant_restarts: u64,
    pub(crate) restarts_blocked_by_level: u64,
    pub(crate) mode_switches: u64,
    pub(crate) decisions_focused: u64,
    pub(crate) decisions_stable: u64,
    pub(crate) seconds_focused: f64,
    pub(crate) seconds_stable: f64,
    pub(crate) conflicts_focused: u64,
    pub(crate) conflicts_stable: u64,
    pub(crate) lbd_sum_focused: u64,
    pub(crate) lbd_sum_stable: u64,
    pub(crate) lbd_count_focused: u64,
    pub(crate) lbd_count_stable: u64,
    pub(crate) decision_level_sum_focused: u64,
    pub(crate) decision_level_sum_stable: u64,
    pub(crate) chrono_attempts: u64,
    pub(crate) chrono_used: u64,
    pub(crate) chrono_rejected_not_asserting: u64,
    pub(crate) chrono_rejected_delta_too_large: u64,
    pub(crate) chrono_skipped_levels: u64,
    pub(crate) binary_props: u64,
    pub(crate) binary_stale_skips: u64,
    pub(crate) long_props: u64,
    pub(crate) watch_scans: u64,
    pub(crate) watch_stale_skips: u64,
    pub(crate) watch_blocker_hits: u64,
    pub(crate) watch_clause_loads: u64,
    pub(crate) phase_saved_used: u64,
    pub(crate) phase_legacy_used: u64,
    pub(crate) phase_target_used: u64,
    pub(crate) phase_best_used: u64,
    pub(crate) phase_initial_used: u64,
    pub(crate) phase_save_target: u64,
    pub(crate) phase_save_best: u64,
    pub(crate) rephases: u64,
    pub(crate) decision_heap_pops: u64,
    pub(crate) decision_heap_stale_pops: u64,
    pub(crate) decision_heap_inserts: u64,
    pub(crate) avg_decision_level_sum: u64,
    pub(crate) max_decision_level: u64,
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
    pub(crate) preprocess_sec: f64,
    pub(crate) search_sec: f64,
    pub(crate) proof_sec: f64,
}

impl SolverStats {
    pub(crate) fn record_lbd(&mut self, lbd: u32) {
        self.lbd_computed += 1;
        self.lbd_sum += lbd as u64;
        self.lbd_max = self.lbd_max.max(lbd);
        match lbd {
            1 => self.lbd_1 += 1,
            2 => self.lbd_2 += 1,
            3..=5 => self.lbd_3_5 += 1,
            6..=10 => self.lbd_6_10 += 1,
            _ => self.lbd_gt_10 += 1,
        }
    }

    pub(crate) fn record_learned_size(&mut self, size: usize) {
        self.learned_size_sum += size as u64;
        self.learned_size_max = self.learned_size_max.max(size as u64);
    }

    pub(crate) fn record_mode_lbd(&mut self, lbd: u32, focused: bool) {
        if focused {
            self.lbd_sum_focused += lbd as u64;
            self.lbd_count_focused += 1;
        } else {
            self.lbd_sum_stable += lbd as u64;
            self.lbd_count_stable += 1;
        }
    }

    pub(crate) fn record_mode_conflict(&mut self, focused: bool) {
        if focused {
            self.conflicts_focused += 1;
        } else {
            self.conflicts_stable += 1;
        }
    }

    pub(crate) fn record_mode_decision_level(&mut self, level: u64, focused: bool) {
        if focused {
            self.decision_level_sum_focused += level;
        } else {
            self.decision_level_sum_stable += level;
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ProofStats {
    pub(crate) added_clauses: u64,
    pub(crate) deleted_clauses: u64,
    pub(crate) added_literals: u64,
    pub(crate) deleted_literals: u64,
    pub(crate) flushes: u64,
    pub(crate) bytes_written: u64,
    pub(crate) max_clause_len: u64,
    pub(crate) temp_created: bool,
    pub(crate) temp_deleted: bool,
    pub(crate) finalized: bool,
    pub(crate) incomplete: bool,
    pub(crate) write_errors: u64,
    pub(crate) state: &'static str,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InputIdentity {
    pub(crate) path: String,
    pub(crate) sha256: Option<String>,
    pub(crate) compressed_sha256: Option<String>,
    pub(crate) size_bytes: Option<u64>,
}

impl InputIdentity {
    pub(crate) fn from_path(path: &Path) -> Self {
        let sha256 = sha256_file(path);
        let compressed_sha256 = if is_compressed_cnf(path) {
            sha256.clone()
        } else {
            None
        };
        let size_bytes = fs::metadata(path).ok().map(|metadata| metadata.len());
        Self {
            path: path.display().to_string(),
            sha256,
            compressed_sha256,
            size_bytes,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FormulaStats {
    pub(crate) vars: u64,
    pub(crate) original_clauses_initial: u64,
    pub(crate) original_lits_initial: u64,
    pub(crate) original_clauses_after_preprocess: u64,
    pub(crate) original_lits_after_preprocess: u64,
    pub(crate) learned_clauses_final: u64,
    pub(crate) learned_lits_final: u64,
    pub(crate) arena_words_live: u64,
    pub(crate) arena_words_garbage: u64,
    pub(crate) arena_garbage_ratio: f64,
    pub(crate) learned_words_live: u64,
    pub(crate) original_words_live: u64,
    pub(crate) watchers_live: u64,
    pub(crate) watchers_stale: u64,
    pub(crate) deleted_words: u64,
    pub(crate) binary_clauses_final: u64,
    pub(crate) binary_implication_edges_final: u64,
    pub(crate) max_clause_buffer_len: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RunTimings {
    pub(crate) elapsed_sec: f64,
    pub(crate) parse_sec: f64,
    pub(crate) preprocess_sec: f64,
    pub(crate) search_sec: f64,
    pub(crate) proof_sec: f64,
}

pub(crate) struct StatsJsonContext<'a> {
    pub(crate) config: &'a SolverConfig,
    pub(crate) stats: &'a SolverStats,
    pub(crate) proof: &'a ProofStats,
    pub(crate) input: &'a InputIdentity,
    pub(crate) formula: &'a FormulaStats,
    pub(crate) timings: &'a RunTimings,
    pub(crate) status: SolveStatus,
    pub(crate) exit_code: i32,
    pub(crate) status_file_status: Option<&'a str>,
    pub(crate) termination_reason: &'a str,
    pub(crate) unknown_reason: Option<&'a str>,
    pub(crate) limit_hit: bool,
    pub(crate) parse_error_kind: Option<&'a str>,
    pub(crate) model_check_result: &'a str,
    pub(crate) proof_check_result: &'a str,
    pub(crate) proof_completeness: ProofCompleteness,
    pub(crate) output_contract_state: &'a str,
    pub(crate) max_rss_mb: Option<u64>,
}

pub(crate) fn binary_sha256() -> Option<String> {
    std::env::current_exe()
        .ok()
        .and_then(|path| sha256_file(&path))
}

pub(crate) fn max_rss_mb() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        let Some(rest) = line.strip_prefix("VmHWM:") else {
            continue;
        };
        let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
        return Some(kb.div_ceil(1024));
    }
    None
}

pub(crate) fn json_stats_line(ctx: &StatsJsonContext<'_>) -> String {
    let mut json = JsonObject::new();
    json.u64("schema_version", 1);
    json.string("solver", "11-kissat-port");
    json.string(
        "solver_git_sha",
        option_env!("SOLVER_GIT_SHA").unwrap_or("unknown"),
    );
    json.string(
        "rustc_version",
        option_env!("SOLVER_RUSTC_VERSION").unwrap_or("unknown"),
    );
    json.string_opt("binary_sha256", binary_sha256().as_deref());
    json.string("input_path", &ctx.input.path);
    json.string_opt("input_sha256", ctx.input.sha256.as_deref());
    json.string_opt(
        "input_compressed_sha256",
        ctx.input.compressed_sha256.as_deref(),
    );
    json.u64_opt("input_size_bytes", ctx.input.size_bytes);
    json.null("manifest_selection_version");
    json.null("manifest_expected_status");
    json.string("profile", ctx.config.profile_name());
    json.string("proof_policy", ctx.config.proof_policy_name());
    json.bool("hot_diagnostics_enabled", ctx.config.hot_stats);
    json.string("config_hash", &ctx.config.config_hash());
    json.bool("replay_overridden", ctx.config.replay_overridden);
    json.string_array("replay_override_vars", &ctx.config.replay_override_env);
    json.raw(
        "feature_maturity_summary",
        &feature_maturity_summary_json(ctx.config),
    );
    json.u64("seed", ctx.config.deterministic_seed);
    json.string_opt("run_label", ctx.config.run_label.as_deref());
    json.string("result", ctx.status.as_str());
    json.i64("exit_code", ctx.exit_code as i64);
    json.string_opt("status_file_status", ctx.status_file_status);
    json.string("termination_reason", ctx.termination_reason);
    json.string_opt("unknown_reason", ctx.unknown_reason);
    json.bool("limit_hit", ctx.limit_hit);
    json.string_opt("parse_error_kind", ctx.parse_error_kind);
    json.string("model_check_result", ctx.model_check_result);
    json.string("proof_check_result", ctx.proof_check_result);
    json.null("proof_checker");
    json.null("proof_checker_version");
    json.f64("elapsed_sec", ctx.timings.elapsed_sec);
    json.f64("parse_sec", ctx.timings.parse_sec);
    json.f64("preprocess_sec", ctx.timings.preprocess_sec);
    json.f64("search_sec", ctx.timings.search_sec);
    json.f64("proof_sec", ctx.timings.proof_sec);

    json.u64("vars", ctx.formula.vars);
    json.u64(
        "original_clauses_initial",
        ctx.formula.original_clauses_initial,
    );
    json.u64("original_lits_initial", ctx.formula.original_lits_initial);
    json.u64(
        "original_clauses_after_preprocess",
        ctx.formula.original_clauses_after_preprocess,
    );
    json.u64(
        "original_lits_after_preprocess",
        ctx.formula.original_lits_after_preprocess,
    );
    json.u64("learned_clauses_final", ctx.formula.learned_clauses_final);
    json.u64("learned_lits_final", ctx.formula.learned_lits_final);

    json.u64("conflicts", ctx.stats.conflicts);
    json.u64("decisions", ctx.stats.decisions);
    json.u64("propagations", ctx.stats.propagations);
    json.u64("search_ticks", ctx.stats.search_ticks);
    json.u64("restarts", ctx.stats.restarts);
    json.u64("reductions", ctx.stats.reduce_db_calls);
    json.u64("gc_count", ctx.stats.garbage_collections);
    json.string("gc_reason", ctx.stats.gc_last_reason.as_str());
    json.u64("gc_words_reclaimed", ctx.stats.gc_words_reclaimed);
    json.u64("gc_refs_rewritten", ctx.stats.gc_refs_rewritten);
    json.u64("deleted_clauses", ctx.stats.deleted_clauses);
    json.u64("deleted_words", ctx.formula.deleted_words);
    json.u64("arena_words_live", ctx.formula.arena_words_live);
    json.u64("arena_words_garbage", ctx.formula.arena_words_garbage);
    json.f64("arena_garbage_ratio", ctx.formula.arena_garbage_ratio);
    json.u64("learned_words_live", ctx.formula.learned_words_live);
    json.u64("original_words_live", ctx.formula.original_words_live);
    json.u64("watchers_live", ctx.formula.watchers_live);
    json.u64("watchers_stale", ctx.formula.watchers_stale);
    json.u64("learned_kept_tier1", ctx.stats.learned_kept_tier1);
    json.u64("learned_kept_tier2", ctx.stats.learned_kept_tier2);
    json.u64("learned_kept_tier3", ctx.stats.learned_kept_tier3);
    json.u64("learned_collected", ctx.stats.learned_collected);
    json.u64(
        "focused_tier1_glue_limit",
        ctx.stats.focused_tier1_glue_limit,
    );
    json.u64(
        "focused_tier2_glue_limit",
        ctx.stats.focused_tier2_glue_limit,
    );
    json.u64("stable_tier1_glue_limit", ctx.stats.stable_tier1_glue_limit);
    json.u64("stable_tier2_glue_limit", ctx.stats.stable_tier2_glue_limit);
    json.u64_array("focused_glue_used", &ctx.stats.focused_glue_used);
    json.u64_array("stable_glue_used", &ctx.stats.stable_glue_used);
    json.u64("luby_restarts", ctx.stats.luby_restarts);
    json.u64("glucose_restarts", ctx.stats.glucose_restarts);
    json.u64("reluctant_restarts", ctx.stats.reluctant_restarts);
    json.u64(
        "restarts_blocked_by_level",
        ctx.stats.restarts_blocked_by_level,
    );
    json.u64("mode_switches", ctx.stats.mode_switches);
    json.u64("decisions_focused", ctx.stats.decisions_focused);
    json.u64("decisions_stable", ctx.stats.decisions_stable);
    json.f64("seconds_focused", ctx.stats.seconds_focused);
    json.f64("seconds_stable", ctx.stats.seconds_stable);
    json.u64("conflicts_focused", ctx.stats.conflicts_focused);
    json.u64("conflicts_stable", ctx.stats.conflicts_stable);
    json.u64("lbd_sum_focused", ctx.stats.lbd_sum_focused);
    json.u64("lbd_sum_stable", ctx.stats.lbd_sum_stable);
    json.u64("lbd_count_focused", ctx.stats.lbd_count_focused);
    json.u64("lbd_count_stable", ctx.stats.lbd_count_stable);
    json.u64(
        "decision_level_sum_focused",
        ctx.stats.decision_level_sum_focused,
    );
    json.u64(
        "decision_level_sum_stable",
        ctx.stats.decision_level_sum_stable,
    );
    json.u64("chrono_attempts", ctx.stats.chrono_attempts);
    json.u64("chrono_used", ctx.stats.chrono_used);
    json.u64(
        "chrono_rejected_not_asserting",
        ctx.stats.chrono_rejected_not_asserting,
    );
    json.u64(
        "chrono_rejected_delta_too_large",
        ctx.stats.chrono_rejected_delta_too_large,
    );
    json.u64("chrono_skipped_levels", ctx.stats.chrono_skipped_levels);

    json.f64("avg_decision_level", average_decision_level(ctx.stats));
    json.u64("max_decision_level", ctx.stats.max_decision_level);
    json.f64("avg_lbd", average_lbd(ctx.stats));
    json.f64("focused_avg_lbd", focused_average_lbd(ctx.stats));
    json.f64("stable_avg_lbd", stable_average_lbd(ctx.stats));
    json.f64(
        "focused_avg_decision_level",
        focused_average_decision_level(ctx.stats),
    );
    json.f64(
        "stable_avg_decision_level",
        stable_average_decision_level(ctx.stats),
    );
    json.u64("lbd_1", ctx.stats.lbd_1);
    json.u64("lbd_2", ctx.stats.lbd_2);
    json.u64("lbd_3_5", ctx.stats.lbd_3_5);
    json.u64("lbd_6_10", ctx.stats.lbd_6_10);
    json.u64("lbd_gt_10", ctx.stats.lbd_gt_10);
    json.u64("lbd_improved", ctx.stats.lbd_improved);

    json.u64("binary_props", ctx.stats.binary_props);
    json.u64("binary_stale_skips", ctx.stats.binary_stale_skips);
    json.u64("long_props", ctx.stats.long_props);
    json.u64("watch_scans", ctx.stats.watch_scans);
    json.u64("watch_stale_skips", ctx.stats.watch_stale_skips);
    json.u64("watch_blocker_hits", ctx.stats.watch_blocker_hits);
    json.u64("watch_clause_loads", ctx.stats.watch_clause_loads);

    json.u64("phase_saved_used", ctx.stats.phase_saved_used);
    json.u64("phase_legacy_used", ctx.stats.phase_legacy_used);
    json.u64("phase_target_used", ctx.stats.phase_target_used);
    json.u64("phase_best_used", ctx.stats.phase_best_used);
    json.u64("phase_initial_used", ctx.stats.phase_initial_used);
    json.u64("phase_save_target", ctx.stats.phase_save_target);
    json.u64("phase_save_best", ctx.stats.phase_save_best);
    json.u64("rephases", ctx.stats.rephases);
    json.u64("random_decisions", 0);

    json.u64(
        "pre_bve_eliminated_vars",
        ctx.stats.preprocess_eliminated_vars,
    );
    json.u64("pre_bve_resolvents", ctx.stats.preprocess_resolvents);
    json.u64("pre_bsr_subsumed", ctx.stats.preprocess_subsumed_clauses);
    json.u64(
        "pre_bsr_strengthened",
        ctx.stats.preprocess_strengthened_clauses,
    );
    json.u64(
        "pre_occ_clean_scans",
        ctx.stats.occurs_clean_entries_scanned,
    );
    json.u64(
        "pre_occ_clean_removed",
        ctx.stats.occurs_clean_entries_removed,
    );

    json.u64("inprocess_rounds", 0);
    json.u64("vivify_attempts", 0);
    json.u64("vivify_strengthened", 0);
    json.u64("vivify_subsumed", 0);
    json.u64("vivify_removed_literals", 0);
    json.u64("probe_attempts", 0);
    json.u64("probe_failed_literals", 0);
    json.u64("probe_units", 0);
    json.u64("hbr_added_binary", 0);
    json.u64("gate_equivalences_detected", 0);
    json.u64("transitive_removed", 0);
    json.u64("rcheck_attempts", 0);
    json.u64("rcheck_implied", 0);
    json.u64("proof_added_clauses", ctx.proof.added_clauses);
    json.u64("proof_deleted_clauses", ctx.proof.deleted_clauses);
    json.u64("proof_added_literals", ctx.proof.added_literals);
    json.u64("proof_deleted_literals", ctx.proof.deleted_literals);
    json.u64("proof_flushes", ctx.proof.flushes);
    json.u64("proof_bytes_written", ctx.proof.bytes_written);
    json.u64_opt("max_rss_mb", ctx.max_rss_mb);
    json.u64_opt("rss_limit_mb", ctx.config.rss_limit_mb);
    json.u64_opt("learned_lit_limit", ctx.config.learned_lit_limit);
    json.u64_opt("binary_clause_limit", ctx.config.binary_clause_limit);
    json.u64("binary_clauses_final", ctx.formula.binary_clauses_final);
    json.u64("binary_clauses_deleted", 0);
    json.u64(
        "binary_implication_edges_final",
        ctx.formula.binary_implication_edges_final,
    );
    json.u64_opt("extension_bytes_limit", ctx.config.extension_bytes_limit);
    json.u64_opt("proof_bytes_limit", ctx.config.proof_bytes_limit);
    json.bool("proof_temp_created", ctx.proof.temp_created);
    json.bool("proof_temp_deleted", ctx.proof.temp_deleted);
    json.bool("proof_finalized", ctx.proof.finalized);
    json.bool("proof_incomplete", ctx.proof.incomplete);
    json.u64("proof_write_errors", ctx.proof.write_errors);
    json.string("proof_state", ctx.proof.state);
    json.string("proof_completeness", ctx.proof_completeness.as_str());
    json.string("output_contract_state", ctx.output_contract_state);
    json.u64("extension_entries", 0);
    json.u64("extension_lits", 0);
    json.u64("extension_bytes_estimated", 0);
    json.u64("model_replay_steps", 0);
    json.u64("model_replay_conflicts", 0);
    json.u64("scratch_reuses", 0);
    json.u64("scratch_grows", 0);
    json.u64("hot_allocations_estimated", 0);
    json.u64(
        "max_clause_buffer_len",
        ctx.formula
            .max_clause_buffer_len
            .max(ctx.proof.max_clause_len),
    );
    json.u64("max_txn_buffer_lits", 0);

    format!("c JSON_STATS {}", json.finish())
}

pub(crate) fn trace_full_line(stats: &SolverStats, _timings: &RunTimings) -> String {
    format!(
        concat!(
            "c trace_full ",
            "glue_sum={} glue_max={} glue_count={} ",
            "learned_size_sum={} learned_size_max={} ",
            "glucose_restarts={} luby_restarts={} reluctant_restarts={} restarts_blocked_by_level={} ",
            "chrono_attempts={} chrono_backtracks={} non_chrono_backtracks={} chrono_rejected_not_asserting={} chrono_rejected_delta_too_large={} chrono_skipped_levels={} ",
            "vivified_clauses=0 vivified_strengthened=0 vivified_subsumed=0 vivified_ticks=0 ",
            "probe_failed_lits=0 probe_units=0 probe_ticks=0 ",
            "transitive_removed=0 gate_equivs_found=0 appendix_equiv_substitutions=0 ",
            "inprocess_runs=0 inprocess_ticks=0 ",
            "phase_saved_used={} phase_legacy_used={} phase_target_used={} phase_best_used={} phase_initial_used={} ",
            "phase_save_target={} phase_save_best={} rephases={} ",
            "search_ticks={} ",
            "decisions_focused={} decisions_stable={} ",
            "mode_switches={} seconds_focused={:.6} seconds_stable={:.6} ",
            "conflicts_focused={} conflicts_stable={} ",
            "focused_avg_lbd={:.3} stable_avg_lbd={:.3} ",
            "focused_avg_decision_level={:.3} stable_avg_decision_level={:.3} ",
            "learned_kept_tier1={} learned_kept_tier2={} learned_kept_tier3={} learned_collected={} ",
            "focused_tier1_glue_limit={} focused_tier2_glue_limit={} stable_tier1_glue_limit={} stable_tier2_glue_limit={} ",
            "gc_reason={} gc_words_reclaimed={} gc_refs_rewritten={} ",
            "decision_heap_pops={} decision_heap_stale_pops={} decision_heap_inserts={}"
        ),
        stats.lbd_sum,
        stats.lbd_max,
        stats.lbd_computed,
        stats.learned_size_sum,
        stats.learned_size_max,
        stats.glucose_restarts,
        stats.luby_restarts,
        stats.reluctant_restarts,
        stats.restarts_blocked_by_level,
        stats.chrono_attempts,
        stats.chrono_used,
        stats.chrono_attempts.saturating_sub(stats.chrono_used),
        stats.chrono_rejected_not_asserting,
        stats.chrono_rejected_delta_too_large,
        stats.chrono_skipped_levels,
        stats.phase_saved_used,
        stats.phase_legacy_used,
        stats.phase_target_used,
        stats.phase_best_used,
        stats.phase_initial_used,
        stats.phase_save_target,
        stats.phase_save_best,
        stats.rephases,
        stats.search_ticks,
        stats.decisions_focused,
        stats.decisions_stable,
        stats.mode_switches,
        stats.seconds_focused,
        stats.seconds_stable,
        stats.conflicts_focused,
        stats.conflicts_stable,
        focused_average_lbd(stats),
        stable_average_lbd(stats),
        focused_average_decision_level(stats),
        stable_average_decision_level(stats),
        stats.learned_kept_tier1,
        stats.learned_kept_tier2,
        stats.learned_kept_tier3,
        stats.learned_collected,
        stats.focused_tier1_glue_limit,
        stats.focused_tier2_glue_limit,
        stats.stable_tier1_glue_limit,
        stats.stable_tier2_glue_limit,
        stats.gc_last_reason.as_str(),
        stats.gc_words_reclaimed,
        stats.gc_refs_rewritten,
        stats.decision_heap_pops,
        stats.decision_heap_stale_pops,
        stats.decision_heap_inserts,
    )
}

fn average_lbd(stats: &SolverStats) -> f64 {
    if stats.lbd_computed == 0 {
        0.0
    } else {
        stats.lbd_sum as f64 / stats.lbd_computed as f64
    }
}

fn average_decision_level(stats: &SolverStats) -> f64 {
    if stats.decisions == 0 {
        0.0
    } else {
        stats.avg_decision_level_sum as f64 / stats.decisions as f64
    }
}

fn focused_average_lbd(stats: &SolverStats) -> f64 {
    average_from_sum_count(stats.lbd_sum_focused, stats.lbd_count_focused)
}

fn stable_average_lbd(stats: &SolverStats) -> f64 {
    average_from_sum_count(stats.lbd_sum_stable, stats.lbd_count_stable)
}

fn focused_average_decision_level(stats: &SolverStats) -> f64 {
    average_from_sum_count(stats.decision_level_sum_focused, stats.decisions_focused)
}

fn stable_average_decision_level(stats: &SolverStats) -> f64 {
    average_from_sum_count(stats.decision_level_sum_stable, stats.decisions_stable)
}

fn average_from_sum_count(sum: u64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        sum as f64 / count as f64
    }
}

fn feature_maturity_summary_json(config: &SolverConfig) -> String {
    let mut summary = JsonObject::new();
    summary.u64("total", config.feature_statuses.len() as u64);
    summary.u64(
        "enabled",
        config
            .feature_statuses
            .iter()
            .filter(|feature| feature.enabled)
            .count() as u64,
    );
    for maturity in [
        FeatureMaturity::ParkingLot,
        FeatureMaturity::Experimental,
        FeatureMaturity::SmokeSafe,
        FeatureMaturity::OracleSafe,
        FeatureMaturity::ProofValidated,
        FeatureMaturity::DiscriminatingValidated,
        FeatureMaturity::FullSetValidated,
    ] {
        let count = config
            .feature_statuses
            .iter()
            .filter(|feature| feature.maturity == maturity)
            .count() as u64;
        summary.u64(maturity.as_str(), count);
    }
    summary.finish()
}

fn is_compressed_cnf(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext, "gz" | "xz"))
        .unwrap_or(false)
}

struct JsonObject {
    out: String,
    first: bool,
}

impl JsonObject {
    fn new() -> Self {
        Self {
            out: "{".to_string(),
            first: true,
        }
    }

    fn finish(mut self) -> String {
        self.out.push('}');
        self.out
    }

    fn key(&mut self, key: &str) {
        if self.first {
            self.first = false;
        } else {
            self.out.push(',');
        }
        self.out.push('"');
        self.out.push_str(&json_escape(key));
        self.out.push_str("\":");
    }

    fn string(&mut self, key: &str, value: &str) {
        self.key(key);
        self.out.push('"');
        self.out.push_str(&json_escape(value));
        self.out.push('"');
    }

    fn string_opt(&mut self, key: &str, value: Option<&str>) {
        match value {
            Some(value) => self.string(key, value),
            None => self.null(key),
        }
    }

    fn string_array(&mut self, key: &str, values: &[String]) {
        self.key(key);
        self.out.push('[');
        for (idx, value) in values.iter().enumerate() {
            if idx > 0 {
                self.out.push(',');
            }
            self.out.push('"');
            self.out.push_str(&json_escape(value));
            self.out.push('"');
        }
        self.out.push(']');
    }

    fn u64_array(&mut self, key: &str, values: &[u64]) {
        self.key(key);
        self.out.push('[');
        for (idx, value) in values.iter().enumerate() {
            if idx > 0 {
                self.out.push(',');
            }
            self.out.push_str(&value.to_string());
        }
        self.out.push(']');
    }

    fn bool(&mut self, key: &str, value: bool) {
        self.key(key);
        self.out.push_str(if value { "true" } else { "false" });
    }

    fn u64(&mut self, key: &str, value: u64) {
        self.key(key);
        self.out.push_str(&value.to_string());
    }

    fn u64_opt(&mut self, key: &str, value: Option<u64>) {
        match value {
            Some(value) => self.u64(key, value),
            None => self.null(key),
        }
    }

    fn i64(&mut self, key: &str, value: i64) {
        self.key(key);
        self.out.push_str(&value.to_string());
    }

    fn f64(&mut self, key: &str, value: f64) {
        self.key(key);
        if value.is_finite() {
            self.out.push_str(&format!("{value:.6}"));
        } else {
            self.out.push_str("null");
        }
    }

    fn null(&mut self, key: &str) {
        self.key(key);
        self.out.push_str("null");
    }

    fn raw(&mut self, key: &str, value: &str) {
        self.key(key);
        self.out.push_str(value);
    }
}

fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

fn sha256_file(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut state = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        state.update(&buffer[..read]);
    }
    Some(state.finish_hex())
}

#[cfg(test)]
fn sha256_hex(bytes: &[u8]) -> String {
    let mut state = Sha256::new();
    state.update(bytes);
    state.finish_hex()
}

struct Sha256 {
    h: [u32; 8],
    len_bytes: u64,
    buffer: [u8; 64],
    buffer_len: usize,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            len_bytes: 0,
            buffer: [0; 64],
            buffer_len: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.len_bytes = self.len_bytes.wrapping_add(data.len() as u64);
        if self.buffer_len > 0 {
            let take = (64 - self.buffer_len).min(data.len());
            self.buffer[self.buffer_len..self.buffer_len + take].copy_from_slice(&data[..take]);
            self.buffer_len += take;
            data = &data[take..];
            if self.buffer_len == 64 {
                let block = self.buffer;
                self.process_block(&block);
                self.buffer_len = 0;
            }
        }

        while data.len() >= 64 {
            self.process_block(&data[..64]);
            data = &data[64..];
        }

        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffer_len = data.len();
        }
    }

    fn finish_hex(mut self) -> String {
        let bit_len = self.len_bytes.wrapping_mul(8);
        let mut padding = [0u8; 128];
        padding[0] = 0x80;
        let pad_len = if self.buffer_len < 56 {
            56 - self.buffer_len
        } else {
            64 + 56 - self.buffer_len
        };
        self.update(&padding[..pad_len]);
        self.update(&bit_len.to_be_bytes());

        let mut out = String::with_capacity(64);
        for word in self.h {
            out.push_str(&format!("{word:08x}"));
        }
        out
    }

    fn process_block(&mut self, chunk: &[u8]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for (idx, word) in w.iter_mut().take(16).enumerate() {
            let base = idx * 4;
            *word = u32::from_be_bytes([
                chunk[base],
                chunk[base + 1],
                chunk[base + 2],
                chunk[base + 3],
            ]);
        }
        for idx in 16..64 {
            let s0 =
                w[idx - 15].rotate_right(7) ^ w[idx - 15].rotate_right(18) ^ (w[idx - 15] >> 3);
            let s1 = w[idx - 2].rotate_right(17) ^ w[idx - 2].rotate_right(19) ^ (w[idx - 2] >> 10);
            w[idx] = w[idx - 16]
                .wrapping_add(s0)
                .wrapping_add(w[idx - 7])
                .wrapping_add(s1);
        }

        let mut a = self.h[0];
        let mut b = self.h[1];
        let mut c = self.h[2];
        let mut d = self.h[3];
        let mut e = self.h[4];
        let mut f = self.h[5];
        let mut g = self.h[6];
        let mut hh = self.h[7];

        for idx in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[idx])
                .wrapping_add(w[idx]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.h[0] = self.h[0].wrapping_add(a);
        self.h[1] = self.h[1].wrapping_add(b);
        self.h[2] = self.h[2].wrapping_add(c);
        self.h[3] = self.h[3].wrapping_add(d);
        self.h[4] = self.h[4].wrapping_add(e);
        self.h[5] = self.h[5].wrapping_add(f);
        self.h[6] = self.h[6].wrapping_add(g);
        self.h[7] = self.h[7].wrapping_add(hh);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_stats_line_emits_per_mode_diagnostics() {
        let config = SolverConfig::default();
        let stats = SolverStats {
            seconds_focused: 1.25,
            seconds_stable: 2.5,
            conflicts_focused: 3,
            conflicts_stable: 4,
            decisions_focused: 2,
            decisions_stable: 2,
            lbd_sum_focused: 10,
            lbd_sum_stable: 20,
            lbd_count_focused: 2,
            lbd_count_stable: 4,
            decision_level_sum_focused: 7,
            decision_level_sum_stable: 9,
            phase_legacy_used: 5,
            ..SolverStats::default()
        };
        let proof = ProofStats {
            state: "not-created",
            ..ProofStats::default()
        };
        let input = InputIdentity::default();
        let formula = FormulaStats::default();
        let timings = RunTimings::default();
        let ctx = StatsJsonContext {
            config: &config,
            stats: &stats,
            proof: &proof,
            input: &input,
            formula: &formula,
            timings: &timings,
            status: SolveStatus::Sat,
            exit_code: 0,
            status_file_status: Some("SAT"),
            termination_reason: "sat",
            unknown_reason: None,
            limit_hit: false,
            parse_error_kind: None,
            model_check_result: "pass",
            proof_check_result: "not_checked",
            proof_completeness: ProofCompleteness::NotRequested,
            output_contract_state: "complete",
            max_rss_mb: None,
        };

        let line = json_stats_line(&ctx);

        assert!(line.contains("\"seconds_focused\":1.250000"));
        assert!(line.contains("\"seconds_stable\":2.500000"));
        assert!(line.contains("\"conflicts_focused\":3"));
        assert!(line.contains("\"conflicts_stable\":4"));
        assert!(line.contains("\"focused_avg_lbd\":5.000000"));
        assert!(line.contains("\"stable_avg_lbd\":5.000000"));
        assert!(line.contains("\"focused_avg_decision_level\":3.500000"));
        assert!(line.contains("\"stable_avg_decision_level\":4.500000"));
        assert!(line.contains("\"phase_legacy_used\":5"));
    }

    #[test]
    fn test_json_writer_escapes_labels_paths_and_diagnostics() {
        let mut json = JsonObject::new();
        json.string("label", "solver \"alpha\"\nnext");
        json.string("path", r"C:\tmp\sat\weird.cnf");
        json.string(
            "diagnostic",
            "Invalid SAT_RESTART=\"bad\"\nexpected legacy-luby/kissat-ema",
        );
        let encoded = json.finish();

        assert!(encoded.contains("solver \\\"alpha\\\"\\nnext"));
        assert!(encoded.contains(r"C:\\tmp\\sat\\weird.cnf"));
        assert!(encoded.contains("SAT_RESTART=\\\"bad\\\"\\nexpected"));
    }

    #[test]
    fn test_sha256_known_answer() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
