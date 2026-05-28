//! Configuration parsing boundary for solver 11.
//!
//! The solver proper receives a fully parsed `SolverConfig`.  This keeps
//! feature selection, legacy compatibility variables, profile defaults, config
//! replay, and validation out of propagation/search/simplification hot paths.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const CONFIG_SCHEMA_VERSION: u32 = 1;
const DEFAULT_DETERMINISTIC_SEED: u64 = 0;
const DEFAULT_MINIMIZE_DEPTH_LIMIT: u32 = 1_000;
const DEFAULT_CHRONO_MAX_DELTA: usize = 5_000;
const DEFAULT_MODE_INIT_CONFLICTS: u64 = 2000;
const DEFAULT_MODE_INTERVAL_SCALE: f64 = 1.5;
const DEFAULT_REPHASE_INIT_CONFLICTS: u64 = 1000;
const DEFAULT_REORDER_INTERVAL_CONFLICTS: u64 = 10_000;
const DEFAULT_RESTART_BLOCK_MARGIN: f64 = 0.0;
const DEFAULT_RESTART_SLOW_WINDOW: u64 = 4_096;
const DEFAULT_VAR_DECAY_FOCUSED: f64 = 0.95;
const DEFAULT_VAR_DECAY_STABLE: f64 = 0.95;

const PARKING_LOT_DENYLIST: &[&str] = &["SAT_WALK", "SAT_SWEEP", "SAT_ELS", "SAT_BCE"];
const REMOVED_ALIASES: &[&str] = &["SAT_ELIMINATE_INPROCESS"];

const REPLAY_ALWAYS_ALLOWED: &[&str] = &[
    "SAT_CONFIG_REPLAY",
    "SAT_CONFIG_REPLAY_ALLOW_OVERRIDES",
    "SAT_STRICT_CONFIG",
];

const REPLAY_DEFAULT_ALLOWED_OVERRIDES: &[&str] = &[
    "SAT_CONFIG_OUT",
    "SAT_RUN_LABEL",
    "SAT_STATS_JSON",
    "SAT_STATS_HOT",
    "SAT_TRACE_FULL",
    "SAT_TRACE_PROOF",
    "SAT_TRACE_PREPROCESS",
    "SAT_TRACE_PREPROCESS_DETAILS",
    "SAT_TRACE_SEARCH_INTERVAL",
    "SAT_LIMIT_WALL_SEC",
    "SAT_LIMIT_RSS_MB",
];

#[cfg(test)]
pub(crate) const CONFIG_SCHEMA_CSV: &str = include_str!("../CONFIG_SCHEMA.csv");
#[cfg(test)]
pub(crate) const FEATURES_CSV: &str = include_str!("../FEATURES.csv");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InitialClauseMode {
    Auto,
    CanonicalSorted,
    CanonicalInputOrder,
    KissatWatch,
    Raw,
}

impl InitialClauseMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::CanonicalSorted => "canonical-sorted",
            Self::CanonicalInputOrder => "input-order",
            Self::KissatWatch => "kissat-watch",
            Self::Raw => "raw",
        }
    }

    fn parse(value: &str, env_name: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" | "guarded" | "formula-gated" | "formula_gated" => Self::Auto,
            "canonical" | "canonical-sorted" | "canonical_sorted" | "sorted" | "1" | "true"
            | "on" => Self::CanonicalSorted,
            "input-order"
            | "canonical-input-order"
            | "canonical-input"
            | "canonical_input"
            | "input"
            | "preserve-order" => Self::CanonicalInputOrder,
            "kissat-watch"
            | "kissat_watch"
            | "canonical-kissat-watch"
            | "canonical_kissat_watch"
            | "watch-select"
            | "watch_selection" => Self::KissatWatch,
            "raw" | "solver10" | "legacy" | "off" | "0" | "false" => Self::Raw,
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected auto/canonical-sorted/input-order/kissat-watch/raw"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BranchMode {
    Minisat,
    Occurrence,
}

impl BranchMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Minisat => "minisat",
            Self::Occurrence => "occurrence",
        }
    }

    fn parse(value: &str, env_name: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "minisat" | "mini" | "var-order" | "var_order" | "var" => Self::Minisat,
            "occurrence" | "occ" | "legacy" | "solver10" => Self::Occurrence,
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected minisat/occurrence"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SolverProfile {
    Baseline,
    Default,
    Fast,
    Experimental,
}

impl SolverProfile {
    fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Default => "default",
            Self::Fast => "fast",
            Self::Experimental => "experimental",
        }
    }

    fn parse(value: &str, env_name: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "baseline" => Self::Baseline,
            "default" => Self::Default,
            "fast" => Self::Fast,
            "experimental" => Self::Experimental,
            "search-conservative" | "inprocess-conservative" => Self::Default,
            "search-strong" | "inprocess-gate-aware" => Self::Fast,
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected baseline/default/fast/experimental"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchAxis {
    Safe,
    Validated,
    Strong,
}

impl SearchAxis {
    fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Validated => "validated",
            Self::Strong => "strong",
        }
    }

    fn parse(value: &str, env_name: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "safe" => Self::Safe,
            "validated" => Self::Validated,
            "strong" => Self::Strong,
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected safe/validated/strong"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreprocessAxis {
    Off,
    Conservative,
    GateAware,
}

impl PreprocessAxis {
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Conservative => "conservative",
            Self::GateAware => "gate-aware",
        }
    }

    fn parse(value: &str, env_name: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Self::Off,
            "conservative" => Self::Conservative,
            "gate-aware" | "gate_aware" | "gateaware" => Self::GateAware,
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected off/conservative/gate-aware"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProfileAxes {
    pub(crate) search: SearchAxis,
    pub(crate) preprocess: PreprocessAxis,
}

impl ProfileAxes {
    fn for_profile(profile: SolverProfile) -> Self {
        match profile {
            SolverProfile::Baseline => Self {
                search: SearchAxis::Safe,
                preprocess: PreprocessAxis::Off,
            },
            SolverProfile::Default => Self {
                search: SearchAxis::Validated,
                preprocess: PreprocessAxis::Conservative,
            },
            SolverProfile::Fast => Self {
                search: SearchAxis::Strong,
                preprocess: PreprocessAxis::GateAware,
            },
            SolverProfile::Experimental => Self {
                search: SearchAxis::Validated,
                preprocess: PreprocessAxis::Conservative,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProofPolicy {
    Off,
    Drat,
    Lrat,
}

impl ProofPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Drat => "drat",
            Self::Lrat => "lrat",
        }
    }

    fn parse(value: &str, env_name: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" | "enabled" | "drat" => Self::Drat,
            "0" | "false" | "no" | "off" | "disabled" => Self::Off,
            "lrat" => Self::Lrat,
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected off/drat/lrat"
            )),
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FeatureMaturity {
    ParkingLot,
    Experimental,
    SmokeSafe,
    OracleSafe,
    ProofValidated,
    DiscriminatingValidated,
    FullSetValidated,
}

impl FeatureMaturity {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ParkingLot => "ParkingLot",
            Self::Experimental => "Experimental",
            Self::SmokeSafe => "SmokeSafe",
            Self::OracleSafe => "OracleSafe",
            Self::ProofValidated => "ProofValidated",
            Self::DiscriminatingValidated => "DiscriminatingValidated",
            Self::FullSetValidated => "FullSetValidated",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FeatureStatus {
    pub(crate) name: &'static str,
    pub(crate) enabled: bool,
    pub(crate) maturity: FeatureMaturity,
    pub(crate) proof_validated: bool,
    pub(crate) model_validated: bool,
    pub(crate) full_set_validated: bool,
    pub(crate) validation_artifact: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestartPolicy {
    LegacyLuby,
    KissatEma,
    Reluctant,
}

impl RestartPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::LegacyLuby => "legacy-luby",
            Self::KissatEma => "kissat-ema",
            Self::Reluctant => "reluctant",
        }
    }

    fn parse(value: &str, env_name: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "legacy-luby" | "legacy" | "luby" => Self::LegacyLuby,
            "kissat-ema" | "ema" => Self::KissatEma,
            "reluctant" => Self::Reluctant,
            "minisat" => {
                fail_config("Invalid SAT_RESTART=minisat; use legacy-luby/kissat-ema/reluctant")
            }
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected legacy-luby/kissat-ema/reluctant"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReducePolicy {
    LegacyActivity,
    Activity,
    LbdTiered,
}

impl ReducePolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::LegacyActivity => "legacy",
            Self::Activity => "activity",
            Self::LbdTiered => "lbd-tiered",
        }
    }

    fn parse(value: &str, env_name: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "legacy" | "legacy-activity" => Self::LegacyActivity,
            "activity" => Self::Activity,
            "lbd-tiered" | "lbd_tiered" => Self::LbdTiered,
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected legacy/activity/lbd-tiered"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PhasePolicy {
    Legacy,
    Saved,
    TargetThenSaved,
    BestThenTargetThenSaved,
}

impl PhasePolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Saved => "saved",
            Self::TargetThenSaved => "target-then-saved",
            Self::BestThenTargetThenSaved => "best-then-target-then-saved",
        }
    }

    fn parse(value: &str, env_name: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "legacy" => Self::Legacy,
            "saved" => Self::Saved,
            "target-then-saved" | "target_then_saved" => Self::TargetThenSaved,
            "best-then-target-then-saved" | "best_then_target_then_saved" => {
                Self::BestThenTargetThenSaved
            }
            "target" | "best" | "negative" | "kissat" => fail_config(&format!(
                "Invalid {env_name}={value}; use target-then-saved or best-then-target-then-saved"
            )),
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected legacy/saved/target-then-saved/best-then-target-then-saved"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchModePolicy {
    Single,
    FocusedStable,
}

impl SearchModePolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::FocusedStable => "focused-stable",
        }
    }

    fn parse(value: &str, env_name: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "single" => Self::Single,
            "focused-stable" | "focused_stable" => Self::FocusedStable,
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected single/focused-stable"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VmtfMode {
    Off,
    FocusedOnly,
    Single,
}

impl VmtfMode {
    pub(crate) fn enabled(self) -> bool {
        self != Self::Off
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::FocusedOnly => "focused-only",
            Self::Single => "single",
        }
    }

    fn parse(value: &str, env_name: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "0" | "off" | "false" | "no" => Self::Off,
            "1" | "on" | "true" | "yes" | "focused" | "focused-only" | "focused_only" => {
                Self::FocusedOnly
            }
            "single" => Self::Single,
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected off/focused-only/single"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClauseMinMode {
    Off,
    Basic,
    RecursiveLimited,
    InBlockShrink,
}

impl ClauseMinMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Basic => "basic",
            Self::RecursiveLimited => "recursive-limited",
            Self::InBlockShrink => "inblock",
        }
    }

    fn parse(value: &str, env_name: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "0" => Self::Off,
            "basic" | "1" => Self::Basic,
            "recursive-limited" | "recursive_limited" | "recursive" | "deep" | "2" => {
                Self::RecursiveLimited
            }
            "inblock" | "in-block" | "in_block" => Self::InBlockShrink,
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected off/basic/recursive-limited/inblock"
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SolverConfig {
    pub(crate) schema_version: u32,
    pub(crate) profile: SolverProfile,
    pub(crate) axes: ProfileAxes,
    pub(crate) proof_policy: ProofPolicy,
    pub(crate) feature_statuses: Vec<FeatureStatus>,
    pub(crate) config_dump: bool,
    pub(crate) config_out: Option<PathBuf>,
    pub(crate) config_replay: Option<PathBuf>,
    pub(crate) config_replay_allow_overrides: bool,
    pub(crate) strict_config: bool,
    pub(crate) run_label: Option<String>,

    pub(crate) stats_json: bool,
    pub(crate) hot_stats: bool,
    pub(crate) trace_full: bool,
    pub(crate) trace_proof: bool,
    pub(crate) trace_preprocess: bool,
    pub(crate) trace_preprocess_details: bool,
    pub(crate) trace_search_interval: usize,
    pub(crate) check_invariants: bool,
    pub(crate) deterministic_seed: u64,

    pub(crate) conflict_limit: Option<u64>,
    pub(crate) propagation_limit: Option<u64>,
    pub(crate) tick_limit: Option<u64>,
    pub(crate) wall_limit_sec: Option<f64>,
    pub(crate) rss_limit_mb: Option<u64>,
    pub(crate) learned_lit_limit: Option<u64>,
    pub(crate) binary_clause_limit: Option<u64>,
    pub(crate) extension_bytes_limit: Option<u64>,
    pub(crate) proof_bytes_limit: Option<u64>,

    pub(crate) use_lbd: bool,
    pub(crate) update_reason_lbd: bool,
    pub(crate) update_propagation_reason_lbd: bool,
    pub(crate) restart_policy: RestartPolicy,
    pub(crate) restart_block_margin: f64,
    pub(crate) restart_slow_window: u64,
    pub(crate) restart_reuse_trail: bool,
    pub(crate) restart_reuse_trail_focused: bool,
    pub(crate) restart_reuse_trail_stable: bool,
    pub(crate) reduce_policy: ReducePolicy,
    pub(crate) phase_policy: PhasePolicy,
    pub(crate) focused_phase_policy: Option<PhasePolicy>,
    pub(crate) stable_phase_policy: Option<PhasePolicy>,
    pub(crate) search_mode_policy: SearchModePolicy,
    pub(crate) mode_use_ticks: bool,
    pub(crate) lucky: bool,
    pub(crate) warmup: bool,
    pub(crate) bump_reasons: bool,
    pub(crate) bump_reasons_limit_multiplier: u32,
    pub(crate) chrono_backtrack: bool,
    pub(crate) binary_fast_path: bool,
    pub(crate) clause_min_mode: ClauseMinMode,
    pub(crate) otfs: bool,
    /// Opt-in OTSS: on-the-fly self-subsuming resolution. After conflict analysis
    /// produces a learned clause, scan participating reason clauses (those whose
    /// literals were marked during analyze) and delete any that are strict
    /// supersets of the learned clause. Off by default. Bead SAT-playground-5b2.2.39.
    pub(crate) otss: bool,
    pub(crate) vmtf: VmtfMode,
    pub(crate) rephase: bool,
    pub(crate) reorder: bool,
    pub(crate) minimize_depth_limit: u32,
    pub(crate) chrono_max_delta: usize,
    pub(crate) mode_init_conflicts: u64,
    pub(crate) mode_interval_scale: f64,
    pub(crate) focused_activity_decay: f64,
    pub(crate) stable_activity_decay: f64,
    pub(crate) rephase_init_conflicts: u64,
    pub(crate) reorder_interval_conflicts: u64,

    pub(crate) simplification: bool,
    pub(crate) bve: bool,
    pub(crate) full_bsr: bool,
    // When true, skip full backward subsumption on formulas classified pre-preprocess as
    // (size_class=Large, binary_fraction<0.05, variable_density>100). Per
    // log/analyzesat-2026-05-26-preprocess/FINDINGS.md Gap PRE-3: BSR is a net loss on
    // such formulas (Kakuro -79%, velev -79%) but useful on random 3-SAT / brocard.
    // Default off; the first concrete adaptive-routing rule under bead f06.
    pub(crate) bsr_formula_gate: bool,
    pub(crate) use_resolved_conflict_analysis: bool,
    pub(crate) initial_clause_mode: InitialClauseMode,
    pub(crate) branch_mode: BranchMode,
    pub(crate) reduce_db_init: Option<usize>,
    pub(crate) reduce_db_interval: Option<usize>,
    pub(crate) reduce_min_interval: Option<usize>,
    pub(crate) post_preprocess_reduce_db_reset: Option<bool>,
    pub(crate) subsumption_limit: Option<isize>,

    pub(crate) inprocess: bool,
    pub(crate) vivify: bool,
    pub(crate) probe: bool,
    pub(crate) hbr: bool,
    pub(crate) transitive: bool,
    pub(crate) forward_subsume: bool,
    pub(crate) gate_extract: bool,
    pub(crate) gate_bve: bool,
    pub(crate) rcheck: bool,
    pub(crate) inprocess_interval_conflicts: u64,
    pub(crate) inprocess_max_rounds: u64,
    pub(crate) vivify_ticks_budget: u64,
    pub(crate) vivify_max_clause_len: usize,
    pub(crate) probe_ticks_budget: u64,
    pub(crate) eliminate_ticks_budget: u64,
    pub(crate) eliminate_resolution_budget: u64,
    pub(crate) transitive_max_depth: u32,
    pub(crate) transitive_ticks_per_source: u64,
    pub(crate) transitive_max_removed_per_round: u64,
    pub(crate) rcheck_ticks_budget: u64,

    pub(crate) replay_overridden: bool,
    pub(crate) replay_override_env: Vec<String>,
    pub(crate) legacy_aliases_used: Vec<String>,
}

impl Default for SolverConfig {
    fn default() -> Self {
        let mut config = Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            profile: SolverProfile::Default,
            axes: ProfileAxes::for_profile(SolverProfile::Default),
            proof_policy: ProofPolicy::Drat,
            feature_statuses: Vec::new(),
            config_dump: false,
            config_out: None,
            config_replay: None,
            config_replay_allow_overrides: false,
            strict_config: false,
            run_label: None,

            stats_json: false,
            hot_stats: false,
            trace_full: false,
            trace_proof: false,
            trace_preprocess: false,
            trace_preprocess_details: false,
            trace_search_interval: 0,
            check_invariants: false,
            deterministic_seed: DEFAULT_DETERMINISTIC_SEED,

            conflict_limit: None,
            propagation_limit: None,
            tick_limit: None,
            wall_limit_sec: None,
            rss_limit_mb: None,
            learned_lit_limit: None,
            binary_clause_limit: None,
            extension_bytes_limit: None,
            proof_bytes_limit: None,

            use_lbd: false,
            update_reason_lbd: false,
            update_propagation_reason_lbd: false,
            restart_policy: RestartPolicy::LegacyLuby,
            restart_block_margin: DEFAULT_RESTART_BLOCK_MARGIN,
            restart_slow_window: DEFAULT_RESTART_SLOW_WINDOW,
            restart_reuse_trail: false,
            restart_reuse_trail_focused: false,
            restart_reuse_trail_stable: false,
            reduce_policy: ReducePolicy::LegacyActivity,
            phase_policy: PhasePolicy::Legacy,
            focused_phase_policy: None,
            stable_phase_policy: None,
            search_mode_policy: SearchModePolicy::Single,
            mode_use_ticks: false,
            lucky: false,
            warmup: false,
            bump_reasons: false,
            bump_reasons_limit_multiplier: 10,
            chrono_backtrack: false,
            binary_fast_path: false,
            clause_min_mode: ClauseMinMode::RecursiveLimited,
            otfs: false,
            otss: false,
            vmtf: VmtfMode::Off,
            rephase: false,
            reorder: false,
            minimize_depth_limit: DEFAULT_MINIMIZE_DEPTH_LIMIT,
            chrono_max_delta: DEFAULT_CHRONO_MAX_DELTA,
            mode_init_conflicts: DEFAULT_MODE_INIT_CONFLICTS,
            mode_interval_scale: DEFAULT_MODE_INTERVAL_SCALE,
            focused_activity_decay: DEFAULT_VAR_DECAY_FOCUSED,
            stable_activity_decay: DEFAULT_VAR_DECAY_STABLE,
            rephase_init_conflicts: DEFAULT_REPHASE_INIT_CONFLICTS,
            reorder_interval_conflicts: DEFAULT_REORDER_INTERVAL_CONFLICTS,

            simplification: true,
            bve: true,
            full_bsr: true,
            bsr_formula_gate: false,
            use_resolved_conflict_analysis: false,
            initial_clause_mode: InitialClauseMode::CanonicalSorted,
            branch_mode: BranchMode::Minisat,
            reduce_db_init: None,
            reduce_db_interval: None,
            reduce_min_interval: None,
            post_preprocess_reduce_db_reset: None,
            subsumption_limit: None,

            inprocess: false,
            vivify: false,
            probe: false,
            hbr: false,
            transitive: false,
            forward_subsume: false,
            gate_extract: false,
            gate_bve: false,
            rcheck: false,
            inprocess_interval_conflicts: 0,
            inprocess_max_rounds: 0,
            vivify_ticks_budget: 0,
            vivify_max_clause_len: 0,
            probe_ticks_budget: 0,
            eliminate_ticks_budget: 0,
            eliminate_resolution_budget: 0,
            transitive_max_depth: 0,
            transitive_ticks_per_source: 0,
            transitive_max_removed_per_round: 0,
            rcheck_ticks_budget: 0,

            replay_overridden: false,
            replay_override_env: Vec::new(),
            legacy_aliases_used: Vec::new(),
        };
        config.refresh_feature_statuses();
        config
    }
}

impl SolverConfig {
    pub(crate) fn from_env() -> Self {
        let env_map: BTreeMap<String, String> = env::vars()
            .filter(|(name, _)| name.starts_with("SAT_"))
            .collect();
        Self::from_env_map(&env_map)
    }

    pub(crate) fn from_env_map(env_map: &BTreeMap<String, String>) -> Self {
        validate_removed_and_parked_vars(env_map);
        let strict_config = parse_bool_map(env_map, "SAT_STRICT_CONFIG", false);
        if strict_config {
            validate_unknown_sat_vars(env_map);
        }
        validate_legacy_conflicts(env_map, strict_config);

        let mut config = if let Some(replay_path) = env_map.get("SAT_CONFIG_REPLAY") {
            let allow_overrides =
                parse_bool_map(env_map, "SAT_CONFIG_REPLAY_ALLOW_OVERRIDES", false);
            let mut replayed = Self::from_replay_file(Path::new(replay_path));
            replayed.config_replay = Some(PathBuf::from(replay_path));
            replayed.config_replay_allow_overrides = allow_overrides;
            replayed.strict_config = strict_config || replayed.strict_config;
            let overrides = replay_override_env(env_map, allow_overrides);
            replayed.apply_env_overrides(env_map, &overrides);
            replayed.replay_overridden = !overrides.is_empty();
            replayed.replay_override_env = overrides;
            replayed
        } else {
            let requested_profile = env_map
                .get("SAT_PROFILE")
                .map(|value| SolverProfile::parse(value, "SAT_PROFILE"))
                .unwrap_or(SolverProfile::Default);
            let mut fresh = Self::default();
            fresh.apply_profile_defaults(requested_profile);
            fresh.apply_env_overrides(env_map, &all_sat_env_keys(env_map));
            fresh
        };

        if config.trace_full {
            config.trace_preprocess = true;
            config.trace_preprocess_details = true;
        }
        config.refresh_feature_statuses();
        config.validate_runtime_support();
        config
    }

    fn apply_profile_defaults(&mut self, requested: SolverProfile) {
        let axes = ProfileAxes::for_profile(requested);
        self.profile = requested;
        self.axes = axes;
        match requested {
            SolverProfile::Baseline => {
                self.use_lbd = false;
                self.search_mode_policy = SearchModePolicy::Single;
                self.mode_use_ticks = false;
                self.lucky = false;
                self.initial_clause_mode = InitialClauseMode::CanonicalSorted;
            }
            SolverProfile::Default | SolverProfile::Fast => {
                self.use_lbd = false;
                self.search_mode_policy = SearchModePolicy::Single;
                self.mode_use_ticks = false;
                self.lucky = false;
                self.initial_clause_mode = InitialClauseMode::CanonicalSorted;
            }
            SolverProfile::Experimental => {
                self.use_lbd = true;
                self.search_mode_policy = SearchModePolicy::FocusedStable;
                self.mode_use_ticks = true;
                self.lucky = true;
            }
        }
        if requested == SolverProfile::Baseline || axes.preprocess == PreprocessAxis::Off {
            self.simplification = false;
            self.bve = false;
            self.full_bsr = false;
        }
    }

    fn apply_env_overrides(&mut self, env_map: &BTreeMap<String, String>, keys: &[String]) {
        let key_set: BTreeSet<&str> = keys.iter().map(String::as_str).collect();

        if let Some(value) = get_selected(env_map, &key_set, "SAT_PROFILE") {
            self.apply_profile_defaults(SolverProfile::parse(value, "SAT_PROFILE"));
        }
        let mut axes = self.axes;
        if let Some(value) = get_selected(env_map, &key_set, "SAT_SEARCH_AXIS") {
            axes.search = SearchAxis::parse(value, "SAT_SEARCH_AXIS");
        }
        if let Some(value) = get_selected(env_map, &key_set, "SAT_PREPROCESS_AXIS") {
            axes.preprocess = PreprocessAxis::parse(value, "SAT_PREPROCESS_AXIS");
        }
        self.axes = axes;
        self.profile = normalize_profile(self.profile, axes);
        if axes.preprocess == PreprocessAxis::Off {
            self.simplification = false;
            self.bve = false;
            self.full_bsr = false;
        }

        self.proof_policy = parse_enum_selected(
            env_map,
            &key_set,
            "SAT_PROOF",
            self.proof_policy,
            ProofPolicy::parse,
        );
        self.config_dump =
            parse_bool_selected(env_map, &key_set, "SAT_CONFIG_DUMP", self.config_dump);
        self.config_out =
            parse_path_selected(env_map, &key_set, "SAT_CONFIG_OUT", self.config_out.take());
        self.config_replay = parse_path_selected(
            env_map,
            &key_set,
            "SAT_CONFIG_REPLAY",
            self.config_replay.take(),
        );
        self.config_replay_allow_overrides = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_CONFIG_REPLAY_ALLOW_OVERRIDES",
            self.config_replay_allow_overrides,
        );
        self.strict_config =
            parse_bool_selected(env_map, &key_set, "SAT_STRICT_CONFIG", self.strict_config);
        self.run_label =
            parse_string_selected(env_map, &key_set, "SAT_RUN_LABEL", self.run_label.take());

        self.stats_json = parse_bool_selected(env_map, &key_set, "SAT_STATS_JSON", self.stats_json);
        self.hot_stats = parse_bool_selected(env_map, &key_set, "SAT_STATS_HOT", self.hot_stats);
        self.trace_full = parse_bool_selected(env_map, &key_set, "SAT_TRACE_FULL", self.trace_full);
        self.trace_proof =
            parse_bool_selected(env_map, &key_set, "SAT_TRACE_PROOF", self.trace_proof);
        self.trace_preprocess = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_TRACE_PREPROCESS",
            self.trace_preprocess,
        );
        self.trace_preprocess_details = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_TRACE_PREPROCESS_DETAILS",
            self.trace_preprocess_details,
        );
        self.trace_search_interval = parse_usize_selected(
            env_map,
            &key_set,
            "SAT_TRACE_SEARCH_INTERVAL",
            self.trace_search_interval,
        );
        self.check_invariants = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_CHECK_INVARIANTS",
            self.check_invariants,
        );
        self.deterministic_seed =
            parse_u64_selected(env_map, &key_set, "SAT_SEED", self.deterministic_seed);

        self.conflict_limit = parse_option_u64_selected(
            env_map,
            &key_set,
            "SAT_LIMIT_CONFLICTS",
            self.conflict_limit,
        );
        self.propagation_limit = parse_option_u64_selected(
            env_map,
            &key_set,
            "SAT_LIMIT_PROPAGATIONS",
            self.propagation_limit,
        );
        self.tick_limit =
            parse_option_u64_selected(env_map, &key_set, "SAT_LIMIT_TICKS", self.tick_limit);
        self.wall_limit_sec =
            parse_option_f64_selected(env_map, &key_set, "SAT_LIMIT_WALL_SEC", self.wall_limit_sec);
        self.rss_limit_mb =
            parse_option_u64_selected(env_map, &key_set, "SAT_LIMIT_RSS_MB", self.rss_limit_mb);
        self.learned_lit_limit = parse_option_u64_selected(
            env_map,
            &key_set,
            "SAT_LIMIT_LEARNED_LITS",
            self.learned_lit_limit,
        );
        self.binary_clause_limit = parse_option_u64_selected(
            env_map,
            &key_set,
            "SAT_LIMIT_BINARY_CLAUSES",
            self.binary_clause_limit,
        );
        self.extension_bytes_limit = parse_option_u64_selected(
            env_map,
            &key_set,
            "SAT_LIMIT_EXTENSION_BYTES",
            self.extension_bytes_limit,
        );
        self.proof_bytes_limit = parse_option_u64_selected(
            env_map,
            &key_set,
            "SAT_LIMIT_PROOF_BYTES",
            self.proof_bytes_limit,
        );

        self.use_lbd = parse_bool_selected(env_map, &key_set, "SAT_USE_LBD", self.use_lbd);
        self.update_reason_lbd = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_LBD_UPDATE_REASONS",
            self.update_reason_lbd,
        );
        self.update_propagation_reason_lbd = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_LBD_UPDATE_PROP_REASONS",
            self.update_propagation_reason_lbd,
        );
        self.restart_policy = parse_enum_selected(
            env_map,
            &key_set,
            "SAT_RESTART",
            self.restart_policy,
            RestartPolicy::parse,
        );
        self.restart_block_margin = parse_f64_selected(
            env_map,
            &key_set,
            "SAT_RESTART_BLOCK_MARGIN",
            self.restart_block_margin,
        );
        self.restart_slow_window = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_EMA_SLOW_WINDOW",
            self.restart_slow_window,
        );
        self.restart_reuse_trail = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_RESTART_REUSE_TRAIL",
            self.restart_reuse_trail,
        );
        self.restart_reuse_trail_focused = self.restart_reuse_trail;
        self.restart_reuse_trail_stable = self.restart_reuse_trail;
        self.restart_reuse_trail_focused = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_RESTART_REUSE_TRAIL_FOCUSED",
            self.restart_reuse_trail_focused,
        );
        self.restart_reuse_trail_stable = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_RESTART_REUSE_TRAIL_STABLE",
            self.restart_reuse_trail_stable,
        );
        self.reduce_policy = parse_enum_selected(
            env_map,
            &key_set,
            "SAT_REDUCE",
            self.reduce_policy,
            ReducePolicy::parse,
        );
        self.phase_policy = parse_enum_selected(
            env_map,
            &key_set,
            "SAT_PHASE",
            self.phase_policy,
            PhasePolicy::parse,
        );
        self.focused_phase_policy = parse_option_phase_policy_selected(
            env_map,
            &key_set,
            "SAT_FOCUSED_PHASE",
            self.focused_phase_policy,
        );
        self.stable_phase_policy = parse_option_phase_policy_selected(
            env_map,
            &key_set,
            "SAT_STABLE_PHASE",
            self.stable_phase_policy,
        );
        self.search_mode_policy = parse_enum_selected(
            env_map,
            &key_set,
            "SAT_SEARCH_MODE",
            self.search_mode_policy,
            SearchModePolicy::parse,
        );
        self.mode_use_ticks =
            parse_bool_selected(env_map, &key_set, "SAT_MODE_USE_TICKS", self.mode_use_ticks);
        self.lucky = parse_bool_selected(env_map, &key_set, "SAT_LUCKY", self.lucky);
        self.warmup = parse_bool_selected(env_map, &key_set, "SAT_WARMUP", self.warmup);
        self.bump_reasons =
            parse_bool_selected(env_map, &key_set, "SAT_BUMP_REASONS", self.bump_reasons);
        self.bump_reasons_limit_multiplier = parse_u32_selected(
            env_map,
            &key_set,
            "SAT_BUMP_REASONS_LIMIT",
            self.bump_reasons_limit_multiplier,
        );
        self.chrono_backtrack =
            parse_bool_selected(env_map, &key_set, "SAT_CHRONO", self.chrono_backtrack);
        self.binary_fast_path =
            parse_bool_selected(env_map, &key_set, "SAT_BINARY_FAST", self.binary_fast_path);
        self.clause_min_mode = parse_enum_selected(
            env_map,
            &key_set,
            "SAT_CLAUSE_MIN",
            self.clause_min_mode,
            ClauseMinMode::parse,
        );
        self.otfs = parse_bool_selected(env_map, &key_set, "SAT_OTFS", self.otfs);
        self.otss = parse_bool_selected(env_map, &key_set, "SAT_OTSS", self.otss);
        let vmtf_explicit = get_selected(env_map, &key_set, "SAT_VMTF").is_some();
        self.vmtf = parse_enum_selected(env_map, &key_set, "SAT_VMTF", self.vmtf, VmtfMode::parse);
        self.rephase = parse_bool_selected(env_map, &key_set, "SAT_REPHASE", self.rephase);
        self.reorder = parse_bool_selected(env_map, &key_set, "SAT_REORDER", self.reorder);
        self.minimize_depth_limit = parse_u32_selected(
            env_map,
            &key_set,
            "SAT_MINIMIZE_DEPTH_LIMIT",
            self.minimize_depth_limit,
        );
        self.chrono_max_delta = parse_usize_selected(
            env_map,
            &key_set,
            "SAT_CHRONO_MAX_DELTA",
            self.chrono_max_delta,
        );
        self.mode_init_conflicts = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_MODE_INIT_CONFLICTS",
            self.mode_init_conflicts,
        );
        self.mode_interval_scale = parse_f64_selected(
            env_map,
            &key_set,
            "SAT_MODE_INTERVAL_SCALE",
            self.mode_interval_scale,
        );
        self.focused_activity_decay = parse_f64_selected(
            env_map,
            &key_set,
            "SAT_VAR_DECAY_FOCUSED",
            self.focused_activity_decay,
        );
        self.stable_activity_decay = parse_f64_selected(
            env_map,
            &key_set,
            "SAT_VAR_DECAY_STABLE",
            self.stable_activity_decay,
        );
        self.rephase_init_conflicts = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_REPHASE_INIT_CONFLICTS",
            self.rephase_init_conflicts,
        );
        self.reorder_interval_conflicts = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_REORDER_INTERVAL_CONFLICTS",
            self.reorder_interval_conflicts,
        );

        self.simplification =
            parse_bool_selected(env_map, &key_set, "SAT_SIMPLIFICATION", self.simplification);
        self.bve = parse_bool_selected(env_map, &key_set, "SAT_BVE", self.bve);
        self.full_bsr = parse_bool_selected(env_map, &key_set, "SAT_FULL_BSR", self.full_bsr);
        self.bsr_formula_gate = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_BSR_FORMULA_GATE",
            self.bsr_formula_gate,
        );
        self.use_resolved_conflict_analysis = parse_conflict_analysis_selected(
            env_map,
            &key_set,
            self.use_resolved_conflict_analysis,
        );
        self.initial_clause_mode = parse_enum_selected(
            env_map,
            &key_set,
            "SAT_INITIAL_CLAUSE_MODE",
            self.initial_clause_mode,
            InitialClauseMode::parse,
        );
        self.branch_mode = parse_enum_selected(
            env_map,
            &key_set,
            "SAT_BRANCH_MODE",
            self.branch_mode,
            BranchMode::parse,
        );
        if get_selected(env_map, &key_set, "SAT_CCMIN_MODE").is_some()
            && get_selected(env_map, &key_set, "SAT_CLAUSE_MIN").is_none()
        {
            self.clause_min_mode = ClauseMinMode::parse(
                get_selected(env_map, &key_set, "SAT_CCMIN_MODE").unwrap(),
                "SAT_CCMIN_MODE",
            );
        }
        self.reduce_db_init = parse_option_usize_selected(
            env_map,
            &key_set,
            "SAT_REDUCE_DB_INIT",
            self.reduce_db_init,
        );
        self.reduce_db_interval = parse_option_usize_selected(
            env_map,
            &key_set,
            "SAT_REDUCE_DB_INTERVAL",
            self.reduce_db_interval,
        );
        self.reduce_min_interval = parse_option_usize_selected(
            env_map,
            &key_set,
            "SAT_REDUCE_MIN_INTERVAL",
            self.reduce_min_interval,
        );
        self.post_preprocess_reduce_db_reset = parse_option_bool_selected(
            env_map,
            &key_set,
            "SAT_POST_PREPROCESS_REDUCE_DB_RESET",
            self.post_preprocess_reduce_db_reset,
        );
        self.subsumption_limit = parse_option_isize_selected(
            env_map,
            &key_set,
            "SAT_SUBSUMPTION_LIMIT",
            self.subsumption_limit,
        );

        self.inprocess = parse_bool_selected(env_map, &key_set, "SAT_INPROCESS", self.inprocess);
        self.vivify = parse_bool_selected(env_map, &key_set, "SAT_VIVIFY", self.vivify);
        self.probe = parse_bool_selected(env_map, &key_set, "SAT_PROBE", self.probe);
        self.hbr = parse_bool_selected(env_map, &key_set, "SAT_HBR", self.hbr);
        self.transitive = parse_bool_selected(env_map, &key_set, "SAT_TRANSITIVE", self.transitive);
        self.forward_subsume = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_FORWARD_SUBSUME",
            self.forward_subsume,
        );
        self.gate_extract =
            parse_bool_selected(env_map, &key_set, "SAT_GATE_EXTRACT", self.gate_extract);
        self.gate_bve = parse_bool_selected(env_map, &key_set, "SAT_GATE_BVE", self.gate_bve);
        self.rcheck = parse_bool_selected(env_map, &key_set, "SAT_RCHECK", self.rcheck);
        self.inprocess_interval_conflicts = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_INPROCESS_INTERVAL_CONFLICTS",
            self.inprocess_interval_conflicts,
        );
        self.inprocess_max_rounds = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_INPROCESS_MAX_ROUNDS",
            self.inprocess_max_rounds,
        );
        self.vivify_ticks_budget = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_VIVIFY_TICKS",
            self.vivify_ticks_budget,
        );
        self.vivify_max_clause_len = parse_usize_selected(
            env_map,
            &key_set,
            "SAT_VIVIFY_MAX_CLAUSE_LEN",
            self.vivify_max_clause_len,
        );
        self.probe_ticks_budget = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_PROBE_TICKS",
            self.probe_ticks_budget,
        );
        self.eliminate_ticks_budget = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_ELIMINATE_TICKS",
            self.eliminate_ticks_budget,
        );
        self.eliminate_resolution_budget = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_ELIMINATE_RESOLUTIONS",
            self.eliminate_resolution_budget,
        );
        self.transitive_max_depth = parse_u32_selected(
            env_map,
            &key_set,
            "SAT_TRANSITIVE_MAX_DEPTH",
            self.transitive_max_depth,
        );
        self.transitive_ticks_per_source = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_TRANSITIVE_TICKS_PER_SOURCE",
            self.transitive_ticks_per_source,
        );
        self.transitive_max_removed_per_round = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_TRANSITIVE_MAX_REMOVED_PER_ROUND",
            self.transitive_max_removed_per_round,
        );
        self.rcheck_ticks_budget = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_RCHECK_TICKS",
            self.rcheck_ticks_budget,
        );

        self.apply_focused_stable_defaults(vmtf_explicit);
        self.record_legacy_aliases(env_map);
    }

    fn apply_focused_stable_defaults(&mut self, vmtf_explicit: bool) {
        if self.search_mode_policy == SearchModePolicy::FocusedStable
            && !vmtf_explicit
            && self.vmtf == VmtfMode::Off
        {
            self.vmtf = VmtfMode::FocusedOnly;
        }
    }

    fn record_legacy_aliases(&mut self, env_map: &BTreeMap<String, String>) {
        for name in [
            "SAT_SIMPLIFICATION",
            "SAT_BVE",
            "SAT_FULL_BSR",
            "SAT_CCMIN_MODE",
            "SAT_CONFLICT_ANALYSIS_MODE",
            "SAT_INITIAL_CLAUSE_MODE",
            "SAT_BRANCH_MODE",
            "SAT_REDUCE_DB_INIT",
            "SAT_REDUCE_DB_INTERVAL",
            "SAT_POST_PREPROCESS_REDUCE_DB_RESET",
            "SAT_SUBSUMPTION_LIMIT",
        ] {
            if env_map.contains_key(name) && !self.legacy_aliases_used.iter().any(|key| key == name)
            {
                self.legacy_aliases_used.push(name.to_string());
            }
        }
        self.legacy_aliases_used.sort();
    }

    fn refresh_feature_statuses(&mut self) {
        self.feature_statuses = feature_metadata(self);
    }

    fn validate_runtime_support(&self) {
        if self.proof_policy == ProofPolicy::Lrat {
            fail_config("Invalid SAT_PROOF=lrat: LRAT output is not implemented yet");
        }
        if self.reduce_policy == ReducePolicy::LbdTiered && !self.use_lbd {
            fail_config("Invalid config: SAT_REDUCE=lbd-tiered requires SAT_USE_LBD=on");
        }
        if self.update_reason_lbd && !self.use_lbd {
            fail_config("Invalid config: SAT_LBD_UPDATE_REASONS=on requires SAT_USE_LBD=on");
        }
        if self.update_propagation_reason_lbd && !self.update_reason_lbd {
            fail_config(
                "Invalid config: SAT_LBD_UPDATE_PROP_REASONS=on requires SAT_LBD_UPDATE_REASONS=on",
            );
        }
        if self.restart_policy == RestartPolicy::KissatEma && !self.use_lbd {
            fail_config("Invalid config: SAT_RESTART=kissat-ema requires SAT_USE_LBD=on");
        }
        if self.restart_policy == RestartPolicy::KissatEma
            && self.search_mode_policy == SearchModePolicy::Single
        {
            fail_config(
                "Invalid config: SAT_RESTART=kissat-ema requires SAT_SEARCH_MODE=focused-stable",
            );
        }
        if self.restart_slow_window == 0 {
            fail_config("Invalid config: SAT_EMA_SLOW_WINDOW must be at least 1");
        }
        if self.search_mode_policy == SearchModePolicy::Single
            && matches!(
                self.phase_policy,
                PhasePolicy::TargetThenSaved | PhasePolicy::BestThenTargetThenSaved
            )
        {
            fail_config(&format!(
                "Invalid config: SAT_PHASE={} requires SAT_SEARCH_MODE=focused-stable",
                self.phase_policy.as_str()
            ));
        }
        if self.vmtf == VmtfMode::FocusedOnly && self.search_mode_policy == SearchModePolicy::Single
        {
            fail_config(
                "Invalid config: SAT_VMTF=focused-only requires SAT_SEARCH_MODE=focused-stable",
            );
        }
        if self.vmtf == VmtfMode::Single && self.search_mode_policy != SearchModePolicy::Single {
            fail_config("Invalid config: SAT_VMTF=single requires SAT_SEARCH_MODE=single");
        }
        if self.rephase && self.search_mode_policy == SearchModePolicy::Single {
            fail_config("Invalid config: SAT_REPHASE=on requires SAT_SEARCH_MODE=focused-stable");
        }
        if self.reorder && self.reorder_interval_conflicts == 0 {
            fail_config("Invalid config: SAT_REORDER_INTERVAL_CONFLICTS must be at least 1");
        }
        if self.focused_activity_decay <= 0.0 || self.focused_activity_decay >= 1.0 {
            fail_config("Invalid config: SAT_VAR_DECAY_FOCUSED must be finite and in (0, 1)");
        }
        if self.stable_activity_decay <= 0.0 || self.stable_activity_decay >= 1.0 {
            fail_config("Invalid config: SAT_VAR_DECAY_STABLE must be finite and in (0, 1)");
        }
        if self.mode_use_ticks && self.search_mode_policy == SearchModePolicy::Single {
            fail_config(
                "Invalid config: SAT_MODE_USE_TICKS=on requires SAT_SEARCH_MODE=focused-stable",
            );
        }
        if self.otfs && self.clause_min_mode == ClauseMinMode::Off {
            fail_config("Invalid config: SAT_OTFS=on requires SAT_CLAUSE_MIN=basic|recursive-limited|inblock");
        }
        if self.otss && self.clause_min_mode == ClauseMinMode::Off {
            fail_config("Invalid config: SAT_OTSS=on requires SAT_CLAUSE_MIN=basic|recursive-limited|inblock");
        }
        if self.hbr && !self.probe {
            fail_config("Invalid config: SAT_HBR=on requires SAT_PROBE=on");
        }
        if self.gate_bve && !self.gate_extract {
            fail_config("Invalid config: SAT_GATE_BVE=on requires SAT_GATE_EXTRACT=on");
        }
        let unsupported = [
            (self.inprocess, "SAT_INPROCESS"),
            (self.vivify, "SAT_VIVIFY"),
            (self.probe, "SAT_PROBE"),
            (self.hbr, "SAT_HBR"),
            (self.transitive, "SAT_TRANSITIVE"),
            (self.forward_subsume, "SAT_FORWARD_SUBSUME"),
            (self.gate_extract, "SAT_GATE_EXTRACT"),
            (self.gate_bve, "SAT_GATE_BVE"),
            (self.rcheck, "SAT_RCHECK"),
        ];
        for (enabled, name) in unsupported {
            if enabled {
                fail_config(&format!(
                    "{name}=on is represented in SolverConfig but its implementation bead has not landed"
                ));
            }
        }
        if self.reduce_policy == ReducePolicy::Activity {
            fail_config("SAT_REDUCE=activity is not implemented yet; use legacy or lbd-tiered");
        }
        if self.search_mode_policy == SearchModePolicy::FocusedStable && !self.use_lbd {
            fail_config("Invalid config: SAT_SEARCH_MODE=focused-stable requires SAT_USE_LBD=on");
        }
        if let Some(interval) = self.reduce_min_interval {
            if interval < 50 {
                fail_config("Invalid config: SAT_REDUCE_MIN_INTERVAL must be at least 50");
            }
        }
    }

    pub(crate) fn emit_requested_outputs(&self) {
        if self.config_dump {
            for line in self.config_replay_text().lines() {
                println!("c config {line}");
            }
        }
        if let Some(path) = &self.config_out {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent).unwrap_or_else(|err| {
                        eprintln!(
                            "Error creating config output directory {}: {err}",
                            parent.display()
                        );
                        std::process::exit(2);
                    });
                }
            }
            fs::write(path, self.config_replay_text()).unwrap_or_else(|err| {
                eprintln!("Error writing SAT_CONFIG_OUT {}: {err}", path.display());
                std::process::exit(2);
            });
        }
    }

    pub(crate) fn config_hash(&self) -> String {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in self.stable_config_body(false).bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{hash:016x}")
    }

    pub(crate) fn profile_name(&self) -> &'static str {
        self.profile.as_str()
    }

    pub(crate) fn proof_policy_name(&self) -> &'static str {
        self.proof_policy.as_str()
    }

    pub(crate) fn config_replay_text(&self) -> String {
        let mut body = self.stable_config_body(true);
        body.push_str("config_hash=");
        body.push_str(&self.config_hash());
        body.push('\n');
        body
    }

    fn stable_config_body(&self, include_output_fields: bool) -> String {
        let mut lines = Vec::new();
        push_kv(
            &mut lines,
            "schema_version",
            self.schema_version.to_string(),
        );
        push_kv(&mut lines, "profile", self.profile.as_str());
        push_kv(&mut lines, "search_axis", self.axes.search.as_str());
        push_kv(&mut lines, "preprocess_axis", self.axes.preprocess.as_str());
        push_kv(&mut lines, "proof_policy", self.proof_policy.as_str());
        push_kv_bool(&mut lines, "config_dump", self.config_dump);
        if include_output_fields {
            push_kv_path(&mut lines, "config_out", self.config_out.as_ref());
            push_kv_path(&mut lines, "config_replay", self.config_replay.as_ref());
        }
        push_kv_bool(
            &mut lines,
            "config_replay_allow_overrides",
            self.config_replay_allow_overrides,
        );
        push_kv_bool(&mut lines, "strict_config", self.strict_config);
        push_kv_option_string(&mut lines, "run_label", self.run_label.as_deref());
        push_kv_bool(&mut lines, "stats_json", self.stats_json);
        push_kv_bool(&mut lines, "hot_stats", self.hot_stats);
        push_kv_bool(&mut lines, "trace_full", self.trace_full);
        push_kv_bool(&mut lines, "trace_proof", self.trace_proof);
        push_kv_bool(&mut lines, "trace_preprocess", self.trace_preprocess);
        push_kv_bool(
            &mut lines,
            "trace_preprocess_details",
            self.trace_preprocess_details,
        );
        push_kv(
            &mut lines,
            "trace_search_interval",
            self.trace_search_interval.to_string(),
        );
        push_kv_bool(&mut lines, "check_invariants", self.check_invariants);
        push_kv(
            &mut lines,
            "deterministic_seed",
            self.deterministic_seed.to_string(),
        );
        push_kv_option_u64(&mut lines, "conflict_limit", self.conflict_limit);
        push_kv_option_u64(&mut lines, "propagation_limit", self.propagation_limit);
        push_kv_option_u64(&mut lines, "tick_limit", self.tick_limit);
        push_kv_option_f64(&mut lines, "wall_limit_sec", self.wall_limit_sec);
        push_kv_option_u64(&mut lines, "rss_limit_mb", self.rss_limit_mb);
        push_kv_option_u64(&mut lines, "learned_lit_limit", self.learned_lit_limit);
        push_kv_option_u64(&mut lines, "binary_clause_limit", self.binary_clause_limit);
        push_kv_option_u64(
            &mut lines,
            "extension_bytes_limit",
            self.extension_bytes_limit,
        );
        push_kv_option_u64(&mut lines, "proof_bytes_limit", self.proof_bytes_limit);
        push_kv_bool(&mut lines, "use_lbd", self.use_lbd);
        push_kv_bool(&mut lines, "update_reason_lbd", self.update_reason_lbd);
        push_kv_bool(
            &mut lines,
            "update_propagation_reason_lbd",
            self.update_propagation_reason_lbd,
        );
        push_kv(&mut lines, "restart_policy", self.restart_policy.as_str());
        push_kv(
            &mut lines,
            "restart_block_margin",
            format_f64(self.restart_block_margin),
        );
        push_kv(
            &mut lines,
            "restart_slow_window",
            self.restart_slow_window.to_string(),
        );
        push_kv_bool(&mut lines, "restart_reuse_trail", self.restart_reuse_trail);
        push_kv_bool(
            &mut lines,
            "restart_reuse_trail_focused",
            self.restart_reuse_trail_focused,
        );
        push_kv_bool(
            &mut lines,
            "restart_reuse_trail_stable",
            self.restart_reuse_trail_stable,
        );
        push_kv(&mut lines, "reduce_policy", self.reduce_policy.as_str());
        push_kv(&mut lines, "phase_policy", self.phase_policy.as_str());
        push_kv_option_phase_policy(
            &mut lines,
            "focused_phase_policy",
            self.focused_phase_policy,
        );
        push_kv_option_phase_policy(&mut lines, "stable_phase_policy", self.stable_phase_policy);
        push_kv(
            &mut lines,
            "search_mode_policy",
            self.search_mode_policy.as_str(),
        );
        push_kv_bool(&mut lines, "mode_use_ticks", self.mode_use_ticks);
        push_kv_bool(&mut lines, "lucky", self.lucky);
        push_kv_bool(&mut lines, "warmup", self.warmup);
        push_kv_bool(&mut lines, "bump_reasons", self.bump_reasons);
        push_kv(
            &mut lines,
            "bump_reasons_limit_multiplier",
            self.bump_reasons_limit_multiplier.to_string(),
        );
        push_kv_bool(&mut lines, "chrono_backtrack", self.chrono_backtrack);
        push_kv_bool(&mut lines, "binary_fast_path", self.binary_fast_path);
        push_kv(&mut lines, "clause_min_mode", self.clause_min_mode.as_str());
        push_kv_bool(&mut lines, "otfs", self.otfs);
        push_kv_bool(&mut lines, "otss", self.otss);
        push_kv(&mut lines, "vmtf", self.vmtf.as_str());
        push_kv_bool(&mut lines, "rephase", self.rephase);
        push_kv_bool(&mut lines, "reorder", self.reorder);
        push_kv(
            &mut lines,
            "minimize_depth_limit",
            self.minimize_depth_limit.to_string(),
        );
        push_kv(
            &mut lines,
            "chrono_max_delta",
            self.chrono_max_delta.to_string(),
        );
        push_kv(
            &mut lines,
            "mode_init_conflicts",
            self.mode_init_conflicts.to_string(),
        );
        push_kv(
            &mut lines,
            "mode_interval_scale",
            format_f64(self.mode_interval_scale),
        );
        push_kv(
            &mut lines,
            "focused_activity_decay",
            format_f64(self.focused_activity_decay),
        );
        push_kv(
            &mut lines,
            "stable_activity_decay",
            format_f64(self.stable_activity_decay),
        );
        push_kv(
            &mut lines,
            "rephase_init_conflicts",
            self.rephase_init_conflicts.to_string(),
        );
        push_kv(
            &mut lines,
            "reorder_interval_conflicts",
            self.reorder_interval_conflicts.to_string(),
        );
        push_kv_bool(&mut lines, "simplification", self.simplification);
        push_kv_bool(&mut lines, "bve", self.bve);
        push_kv_bool(&mut lines, "full_bsr", self.full_bsr);
        push_kv_bool(&mut lines, "bsr_formula_gate", self.bsr_formula_gate);
        push_kv_bool(
            &mut lines,
            "use_resolved_conflict_analysis",
            self.use_resolved_conflict_analysis,
        );
        push_kv(
            &mut lines,
            "initial_clause_mode",
            self.initial_clause_mode.as_str(),
        );
        push_kv(&mut lines, "branch_mode", self.branch_mode.as_str());
        push_kv_option_usize(&mut lines, "reduce_db_init", self.reduce_db_init);
        push_kv_option_usize(&mut lines, "reduce_db_interval", self.reduce_db_interval);
        push_kv_option_usize(&mut lines, "reduce_min_interval", self.reduce_min_interval);
        push_kv_option_bool(
            &mut lines,
            "post_preprocess_reduce_db_reset",
            self.post_preprocess_reduce_db_reset,
        );
        push_kv_option_isize(&mut lines, "subsumption_limit", self.subsumption_limit);
        push_kv_bool(&mut lines, "inprocess", self.inprocess);
        push_kv_bool(&mut lines, "vivify", self.vivify);
        push_kv_bool(&mut lines, "probe", self.probe);
        push_kv_bool(&mut lines, "hbr", self.hbr);
        push_kv_bool(&mut lines, "transitive", self.transitive);
        push_kv_bool(&mut lines, "forward_subsume", self.forward_subsume);
        push_kv_bool(&mut lines, "gate_extract", self.gate_extract);
        push_kv_bool(&mut lines, "gate_bve", self.gate_bve);
        push_kv_bool(&mut lines, "rcheck", self.rcheck);
        push_kv(
            &mut lines,
            "inprocess_interval_conflicts",
            self.inprocess_interval_conflicts.to_string(),
        );
        push_kv(
            &mut lines,
            "inprocess_max_rounds",
            self.inprocess_max_rounds.to_string(),
        );
        push_kv(
            &mut lines,
            "vivify_ticks_budget",
            self.vivify_ticks_budget.to_string(),
        );
        push_kv(
            &mut lines,
            "vivify_max_clause_len",
            self.vivify_max_clause_len.to_string(),
        );
        push_kv(
            &mut lines,
            "probe_ticks_budget",
            self.probe_ticks_budget.to_string(),
        );
        push_kv(
            &mut lines,
            "eliminate_ticks_budget",
            self.eliminate_ticks_budget.to_string(),
        );
        push_kv(
            &mut lines,
            "eliminate_resolution_budget",
            self.eliminate_resolution_budget.to_string(),
        );
        push_kv(
            &mut lines,
            "transitive_max_depth",
            self.transitive_max_depth.to_string(),
        );
        push_kv(
            &mut lines,
            "transitive_ticks_per_source",
            self.transitive_ticks_per_source.to_string(),
        );
        push_kv(
            &mut lines,
            "transitive_max_removed_per_round",
            self.transitive_max_removed_per_round.to_string(),
        );
        push_kv(
            &mut lines,
            "rcheck_ticks_budget",
            self.rcheck_ticks_budget.to_string(),
        );
        if include_output_fields {
            push_kv_bool(&mut lines, "replay_overridden", self.replay_overridden);
            push_kv_list(&mut lines, "replay_override_env", &self.replay_override_env);
        }
        push_kv_list(&mut lines, "legacy_aliases_used", &self.legacy_aliases_used);
        for feature in &self.feature_statuses {
            let prefix = format!("feature.{}", feature.name);
            push_kv_bool(&mut lines, &format!("{prefix}.enabled"), feature.enabled);
            push_kv(
                &mut lines,
                &format!("{prefix}.maturity"),
                feature.maturity.as_str(),
            );
            push_kv_bool(
                &mut lines,
                &format!("{prefix}.proof_validated"),
                feature.proof_validated,
            );
            push_kv_bool(
                &mut lines,
                &format!("{prefix}.model_validated"),
                feature.model_validated,
            );
            push_kv_bool(
                &mut lines,
                &format!("{prefix}.full_set_validated"),
                feature.full_set_validated,
            );
            push_kv_path(
                &mut lines,
                &format!("{prefix}.validation_artifact"),
                feature.validation_artifact.as_ref(),
            );
        }
        lines.sort();
        lines.join("\n") + "\n"
    }

    fn from_replay_file(path: &Path) -> Self {
        let text = fs::read_to_string(path).unwrap_or_else(|err| {
            eprintln!("Error reading SAT_CONFIG_REPLAY {}: {err}", path.display());
            std::process::exit(2);
        });
        Self::from_replay_text(&text, path)
    }

    fn from_replay_text(text: &str, path: &Path) -> Self {
        let replay = parse_replay_kv(text, path);
        let schema_version = replay
            .get("schema_version")
            .unwrap_or_else(|| fail_config("SAT_CONFIG_REPLAY missing schema_version"))
            .parse::<u32>()
            .unwrap_or_else(|err| fail_config(&format!("Invalid replay schema_version: {err}")));
        if schema_version != CONFIG_SCHEMA_VERSION {
            fail_config(&format!(
                "Unsupported config replay schema_version={schema_version}; expected {CONFIG_SCHEMA_VERSION}"
            ));
        }
        let replay_legacy_aliases = replay
            .get("legacy_aliases_used")
            .map(|value| parse_replay_list(value));
        let mut config = Self::default();
        let mut env_map = BTreeMap::new();
        for (field, value) in replay {
            if field == "config_hash" || field.starts_with("feature.") {
                continue;
            }
            if let Some(env_name) = replay_field_to_env(field.as_str()) {
                env_map.insert(env_name.to_string(), value);
            }
        }
        config.apply_env_overrides(&env_map, &all_sat_env_keys(&env_map));
        if let Some(legacy_aliases) = replay_legacy_aliases {
            config.legacy_aliases_used = legacy_aliases;
        }
        config.config_replay = Some(path.to_path_buf());
        config.refresh_feature_statuses();
        config
    }

    #[cfg(test)]
    pub(crate) fn json_stats_line(&self, status: &str) -> String {
        let mut features = String::new();
        features.push('[');
        for (idx, feature) in self.feature_statuses.iter().enumerate() {
            if idx > 0 {
                features.push(',');
            }
            features.push('{');
            push_json_field(&mut features, "name", feature.name, true);
            push_json_field(&mut features, "maturity", feature.maturity.as_str(), true);
            push_json_bool_field(&mut features, "enabled", feature.enabled, true);
            push_json_bool_field(
                &mut features,
                "proof_validated",
                feature.proof_validated,
                true,
            );
            push_json_bool_field(
                &mut features,
                "model_validated",
                feature.model_validated,
                true,
            );
            push_json_bool_field(
                &mut features,
                "full_set_validated",
                feature.full_set_validated,
                false,
            );
            features.push('}');
        }
        features.push(']');

        let mut json = String::new();
        json.push('{');
        push_json_number_field(
            &mut json,
            "schema_version",
            self.schema_version as u64,
            true,
        );
        push_json_field(&mut json, "status", status, true);
        push_json_field(&mut json, "config_hash", &self.config_hash(), true);
        push_json_field(&mut json, "profile", self.profile.as_str(), true);
        push_json_field(&mut json, "search_axis", self.axes.search.as_str(), true);
        push_json_field(
            &mut json,
            "preprocess_axis",
            self.axes.preprocess.as_str(),
            true,
        );
        push_json_field(&mut json, "proof_policy", self.proof_policy.as_str(), true);
        push_json_number_field(&mut json, "seed", self.deterministic_seed, true);
        push_json_bool_field(&mut json, "replay_overridden", self.replay_overridden, true);
        push_json_string_array_field(
            &mut json,
            "replay_override_env",
            &self.replay_override_env,
            true,
        );
        json.push_str("\"features\":");
        json.push_str(&features);
        json.push('}');
        format!("c JSON_STATS {json}")
    }
}

fn normalize_profile(requested: SolverProfile, axes: ProfileAxes) -> SolverProfile {
    match (requested, axes.search, axes.preprocess) {
        (SolverProfile::Baseline, SearchAxis::Safe, PreprocessAxis::Off) => SolverProfile::Baseline,
        (SolverProfile::Default, SearchAxis::Validated, PreprocessAxis::Conservative) => {
            SolverProfile::Default
        }
        (SolverProfile::Fast, SearchAxis::Strong, PreprocessAxis::GateAware) => SolverProfile::Fast,
        (SolverProfile::Experimental, _, _) => SolverProfile::Experimental,
        _ => SolverProfile::Experimental,
    }
}

fn feature_metadata(config: &SolverConfig) -> Vec<FeatureStatus> {
    vec![
        feature(
            "SAT_USE_LBD",
            config.use_lbd,
            FeatureMaturity::SmokeSafe,
            true,
            true,
            false,
            "log/phase1/5b2.2.52-s11-single-lbd-clean",
        ),
        feature(
            "SAT_RESTART_REUSE_TRAIL",
            config.restart_reuse_trail
                || config.restart_reuse_trail_focused
                || config.restart_reuse_trail_stable,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "log/bench-11-kissat-port-2026-05-25-18-43-57/results.csv",
        ),
        feature(
            "SAT_LBD_UPDATE_REASONS",
            config.update_reason_lbd,
            FeatureMaturity::SmokeSafe,
            true,
            true,
            false,
            "log/1.14h/summary.md",
        ),
        feature(
            "SAT_LBD_UPDATE_PROP_REASONS",
            config.update_propagation_reason_lbd,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "log/1.14h/summary.md",
        ),
        feature(
            "SAT_LUCKY",
            config.lucky,
            FeatureMaturity::SmokeSafe,
            true,
            true,
            false,
            "log/phase1/3fs-lucky-off-default-profile/results.csv",
        ),
        feature(
            "SAT_WARMUP",
            config.warmup,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "bd:SAT-playground-5b2.2.36",
        ),
        feature(
            "SAT_BUMP_REASONS",
            config.bump_reasons,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "bd:SAT-playground-5b2.2.37",
        ),
        feature(
            "SAT_CHRONO",
            config.chrono_backtrack,
            FeatureMaturity::SmokeSafe,
            true,
            true,
            false,
            "log/1.13/summary.md",
        ),
        feature(
            "SAT_BINARY_FAST",
            config.binary_fast_path,
            FeatureMaturity::SmokeSafe,
            true,
            true,
            false,
            "log/1.6/summary.md",
        ),
        feature(
            "SAT_VMTF",
            config.vmtf.enabled(),
            FeatureMaturity::SmokeSafe,
            true,
            true,
            false,
            "log/bench-11-kissat-port-2026-05-25-20-01-30/results.csv",
        ),
        feature(
            "SAT_REORDER",
            config.reorder,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "log/phase1/1.14n-summary.md",
        ),
        feature(
            "SAT_REPHASE",
            config.rephase,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "log/bench-11-kissat-port-2026-05-25-18-29-53/results.csv",
        ),
        feature(
            "SAT_MODE_USE_TICKS",
            config.mode_use_ticks,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "log/bench-11-kissat-port-2026-05-25-20-01-30/results.csv",
        ),
        feature(
            "SAT_OTFS",
            config.otfs,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "log/phase1/1.14g-otfs-summary.md",
        ),
        feature(
            "SAT_OTSS",
            config.otss,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "bead/SAT-playground-5b2.2.39",
        ),
        feature(
            "SAT_SIMPLIFICATION",
            config.simplification,
            FeatureMaturity::SmokeSafe,
            true,
            true,
            false,
            "solver/11-kissat-port/BASELINE_LOCK.raw.txt",
        ),
        feature(
            "SAT_BVE",
            config.bve,
            FeatureMaturity::SmokeSafe,
            true,
            true,
            false,
            "solver/11-kissat-port/BASELINE_LOCK.raw.txt",
        ),
        feature(
            "SAT_FULL_BSR",
            config.full_bsr,
            FeatureMaturity::SmokeSafe,
            true,
            true,
            false,
            "solver/11-kissat-port/BASELINE_LOCK.raw.txt",
        ),
        feature(
            "SAT_BSR_FORMULA_GATE",
            config.bsr_formula_gate,
            FeatureMaturity::Experimental,
            false,
            false,
            false,
            "",
        ),
        feature(
            "SAT_INPROCESS",
            config.inprocess,
            FeatureMaturity::ParkingLot,
            false,
            false,
            false,
            "",
        ),
        feature(
            "SAT_VIVIFY",
            config.vivify,
            FeatureMaturity::ParkingLot,
            false,
            false,
            false,
            "",
        ),
        feature(
            "SAT_PROBE",
            config.probe,
            FeatureMaturity::ParkingLot,
            false,
            false,
            false,
            "",
        ),
        feature(
            "SAT_HBR",
            config.hbr,
            FeatureMaturity::ParkingLot,
            false,
            false,
            false,
            "",
        ),
        feature(
            "SAT_TRANSITIVE",
            config.transitive,
            FeatureMaturity::ParkingLot,
            false,
            false,
            false,
            "",
        ),
        feature(
            "SAT_FORWARD_SUBSUME",
            config.forward_subsume,
            FeatureMaturity::ParkingLot,
            false,
            false,
            false,
            "",
        ),
        feature(
            "SAT_GATE_EXTRACT",
            config.gate_extract,
            FeatureMaturity::ParkingLot,
            false,
            false,
            false,
            "",
        ),
        feature(
            "SAT_GATE_BVE",
            config.gate_bve,
            FeatureMaturity::ParkingLot,
            false,
            false,
            false,
            "",
        ),
        feature(
            "SAT_RCHECK",
            config.rcheck,
            FeatureMaturity::ParkingLot,
            false,
            false,
            false,
            "",
        ),
    ]
}

fn feature(
    name: &'static str,
    enabled: bool,
    maturity: FeatureMaturity,
    proof_validated: bool,
    model_validated: bool,
    full_set_validated: bool,
    artifact: &str,
) -> FeatureStatus {
    FeatureStatus {
        name,
        enabled,
        maturity,
        proof_validated,
        model_validated,
        full_set_validated,
        validation_artifact: if artifact.is_empty() {
            None
        } else {
            Some(PathBuf::from(artifact))
        },
    }
}

fn parse_replay_kv(text: &str, path: &Path) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for (line_idx, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            fail_config(&format!(
                "{}:{}: expected key=value in config replay",
                path.display(),
                line_idx + 1
            ));
        };
        values.insert(key.trim().to_string(), decode_replay_value(value.trim()));
    }
    values
}

fn decode_replay_value(value: &str) -> String {
    if value == "none" {
        String::new()
    } else {
        value
            .replace("\\n", "\n")
            .replace("\\=", "=")
            .replace("\\\\", "\\")
    }
}

fn parse_replay_list(value: &str) -> Vec<String> {
    if value.is_empty() {
        Vec::new()
    } else {
        value
            .split(',')
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect()
    }
}

fn encode_replay_value(value: &str) -> String {
    if value.is_empty() {
        "none".to_string()
    } else {
        value
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace('=', "\\=")
    }
}

fn replay_field_to_env(field: &str) -> Option<&'static str> {
    match field {
        "profile" => Some("SAT_PROFILE"),
        "search_axis" => Some("SAT_SEARCH_AXIS"),
        "preprocess_axis" => Some("SAT_PREPROCESS_AXIS"),
        "proof_policy" => Some("SAT_PROOF"),
        "config_dump" => Some("SAT_CONFIG_DUMP"),
        "config_out" => Some("SAT_CONFIG_OUT"),
        "config_replay_allow_overrides" => Some("SAT_CONFIG_REPLAY_ALLOW_OVERRIDES"),
        "strict_config" => Some("SAT_STRICT_CONFIG"),
        "run_label" => Some("SAT_RUN_LABEL"),
        "stats_json" => Some("SAT_STATS_JSON"),
        "hot_stats" => Some("SAT_STATS_HOT"),
        "trace_full" => Some("SAT_TRACE_FULL"),
        "trace_proof" => Some("SAT_TRACE_PROOF"),
        "trace_preprocess" => Some("SAT_TRACE_PREPROCESS"),
        "trace_preprocess_details" => Some("SAT_TRACE_PREPROCESS_DETAILS"),
        "trace_search_interval" => Some("SAT_TRACE_SEARCH_INTERVAL"),
        "check_invariants" => Some("SAT_CHECK_INVARIANTS"),
        "deterministic_seed" => Some("SAT_SEED"),
        "conflict_limit" => Some("SAT_LIMIT_CONFLICTS"),
        "propagation_limit" => Some("SAT_LIMIT_PROPAGATIONS"),
        "tick_limit" => Some("SAT_LIMIT_TICKS"),
        "wall_limit_sec" => Some("SAT_LIMIT_WALL_SEC"),
        "rss_limit_mb" => Some("SAT_LIMIT_RSS_MB"),
        "learned_lit_limit" => Some("SAT_LIMIT_LEARNED_LITS"),
        "binary_clause_limit" => Some("SAT_LIMIT_BINARY_CLAUSES"),
        "extension_bytes_limit" => Some("SAT_LIMIT_EXTENSION_BYTES"),
        "proof_bytes_limit" => Some("SAT_LIMIT_PROOF_BYTES"),
        "use_lbd" => Some("SAT_USE_LBD"),
        "update_reason_lbd" => Some("SAT_LBD_UPDATE_REASONS"),
        "update_propagation_reason_lbd" => Some("SAT_LBD_UPDATE_PROP_REASONS"),
        "restart_policy" => Some("SAT_RESTART"),
        "restart_block_margin" => Some("SAT_RESTART_BLOCK_MARGIN"),
        "restart_slow_window" => Some("SAT_EMA_SLOW_WINDOW"),
        "restart_reuse_trail" => Some("SAT_RESTART_REUSE_TRAIL"),
        "restart_reuse_trail_focused" => Some("SAT_RESTART_REUSE_TRAIL_FOCUSED"),
        "restart_reuse_trail_stable" => Some("SAT_RESTART_REUSE_TRAIL_STABLE"),
        "reduce_policy" => Some("SAT_REDUCE"),
        "phase_policy" => Some("SAT_PHASE"),
        "focused_phase_policy" => Some("SAT_FOCUSED_PHASE"),
        "stable_phase_policy" => Some("SAT_STABLE_PHASE"),
        "search_mode_policy" => Some("SAT_SEARCH_MODE"),
        "mode_use_ticks" => Some("SAT_MODE_USE_TICKS"),
        "lucky" => Some("SAT_LUCKY"),
        "warmup" => Some("SAT_WARMUP"),
        "bump_reasons" => Some("SAT_BUMP_REASONS"),
        "bump_reasons_limit_multiplier" => Some("SAT_BUMP_REASONS_LIMIT"),
        "chrono_backtrack" => Some("SAT_CHRONO"),
        "binary_fast_path" => Some("SAT_BINARY_FAST"),
        "clause_min_mode" => Some("SAT_CLAUSE_MIN"),
        "otfs" => Some("SAT_OTFS"),
        "otss" => Some("SAT_OTSS"),
        "vmtf" => Some("SAT_VMTF"),
        "rephase" => Some("SAT_REPHASE"),
        "reorder" => Some("SAT_REORDER"),
        "minimize_depth_limit" => Some("SAT_MINIMIZE_DEPTH_LIMIT"),
        "chrono_max_delta" => Some("SAT_CHRONO_MAX_DELTA"),
        "mode_init_conflicts" => Some("SAT_MODE_INIT_CONFLICTS"),
        "mode_interval_scale" => Some("SAT_MODE_INTERVAL_SCALE"),
        "focused_activity_decay" => Some("SAT_VAR_DECAY_FOCUSED"),
        "stable_activity_decay" => Some("SAT_VAR_DECAY_STABLE"),
        "rephase_init_conflicts" => Some("SAT_REPHASE_INIT_CONFLICTS"),
        "reorder_interval_conflicts" => Some("SAT_REORDER_INTERVAL_CONFLICTS"),
        "simplification" => Some("SAT_SIMPLIFICATION"),
        "bve" => Some("SAT_BVE"),
        "full_bsr" => Some("SAT_FULL_BSR"),
        "bsr_formula_gate" => Some("SAT_BSR_FORMULA_GATE"),
        "use_resolved_conflict_analysis" => Some("SAT_CONFLICT_ANALYSIS_MODE"),
        "initial_clause_mode" => Some("SAT_INITIAL_CLAUSE_MODE"),
        "branch_mode" => Some("SAT_BRANCH_MODE"),
        "reduce_db_init" => Some("SAT_REDUCE_DB_INIT"),
        "reduce_db_interval" => Some("SAT_REDUCE_DB_INTERVAL"),
        "reduce_min_interval" => Some("SAT_REDUCE_MIN_INTERVAL"),
        "post_preprocess_reduce_db_reset" => Some("SAT_POST_PREPROCESS_REDUCE_DB_RESET"),
        "subsumption_limit" => Some("SAT_SUBSUMPTION_LIMIT"),
        "inprocess" => Some("SAT_INPROCESS"),
        "vivify" => Some("SAT_VIVIFY"),
        "probe" => Some("SAT_PROBE"),
        "hbr" => Some("SAT_HBR"),
        "transitive" => Some("SAT_TRANSITIVE"),
        "forward_subsume" => Some("SAT_FORWARD_SUBSUME"),
        "gate_extract" => Some("SAT_GATE_EXTRACT"),
        "gate_bve" => Some("SAT_GATE_BVE"),
        "rcheck" => Some("SAT_RCHECK"),
        "inprocess_interval_conflicts" => Some("SAT_INPROCESS_INTERVAL_CONFLICTS"),
        "inprocess_max_rounds" => Some("SAT_INPROCESS_MAX_ROUNDS"),
        "vivify_ticks_budget" => Some("SAT_VIVIFY_TICKS"),
        "vivify_max_clause_len" => Some("SAT_VIVIFY_MAX_CLAUSE_LEN"),
        "probe_ticks_budget" => Some("SAT_PROBE_TICKS"),
        "eliminate_ticks_budget" => Some("SAT_ELIMINATE_TICKS"),
        "eliminate_resolution_budget" => Some("SAT_ELIMINATE_RESOLUTIONS"),
        "transitive_max_depth" => Some("SAT_TRANSITIVE_MAX_DEPTH"),
        "transitive_ticks_per_source" => Some("SAT_TRANSITIVE_TICKS_PER_SOURCE"),
        "transitive_max_removed_per_round" => Some("SAT_TRANSITIVE_MAX_REMOVED_PER_ROUND"),
        "rcheck_ticks_budget" => Some("SAT_RCHECK_TICKS"),
        _ => None,
    }
}

fn validate_removed_and_parked_vars(env_map: &BTreeMap<String, String>) {
    for name in REMOVED_ALIASES {
        if env_map.contains_key(*name) {
            fail_config(&format!(
                "{name} is not accepted; use SAT_INPROCESS plus explicit BVE flags"
            ));
        }
    }
    for name in PARKING_LOT_DENYLIST {
        if env_map.contains_key(*name) {
            fail_config(&format!(
                "{name} is parked and may not be used until explicitly unparked"
            ));
        }
    }
}

fn validate_unknown_sat_vars(env_map: &BTreeMap<String, String>) {
    let allowed: BTreeSet<&str> = allowed_env_vars().into_iter().collect();
    let unknown: Vec<&str> = env_map
        .keys()
        .map(String::as_str)
        .filter(|name| !allowed.contains(name))
        .collect();
    if !unknown.is_empty() {
        fail_config(&format!(
            "Unknown SAT_* environment variables with SAT_STRICT_CONFIG=on: {}",
            unknown.join(",")
        ));
    }
}

fn validate_legacy_conflicts(env_map: &BTreeMap<String, String>, strict: bool) {
    if !strict {
        return;
    }
    let conflicts = [
        ("SAT_CCMIN_MODE", "SAT_CLAUSE_MIN"),
        ("SAT_BVE", "SAT_GATE_BVE"),
        ("SAT_SIMPLIFICATION", "SAT_INPROCESS"),
        ("SAT_SIMPLIFICATION", "SAT_VIVIFY"),
        ("SAT_SIMPLIFICATION", "SAT_PROBE"),
        ("SAT_SIMPLIFICATION", "SAT_HBR"),
        ("SAT_SIMPLIFICATION", "SAT_TRANSITIVE"),
        ("SAT_SIMPLIFICATION", "SAT_FORWARD_SUBSUME"),
        ("SAT_SIMPLIFICATION", "SAT_GATE_EXTRACT"),
        ("SAT_SIMPLIFICATION", "SAT_GATE_BVE"),
        ("SAT_SIMPLIFICATION", "SAT_RCHECK"),
    ];
    for (legacy, explicit) in conflicts {
        if env_map.contains_key(legacy) && env_map.contains_key(explicit) {
            fail_config(&format!(
                "Conflicting legacy and explicit config variables with SAT_STRICT_CONFIG=on: {legacy} and {explicit}"
            ));
        }
    }
}

fn replay_override_env(env_map: &BTreeMap<String, String>, allow_overrides: bool) -> Vec<String> {
    let allowed: BTreeSet<&str> = if allow_overrides {
        allowed_env_vars().into_iter().collect()
    } else {
        REPLAY_DEFAULT_ALLOWED_OVERRIDES.iter().copied().collect()
    };
    let always_allowed: BTreeSet<&str> = REPLAY_ALWAYS_ALLOWED.iter().copied().collect();
    let mut overrides: Vec<String> = env_map
        .keys()
        .map(String::as_str)
        .filter(|name| !always_allowed.contains(name))
        .filter(|name| allowed.contains(name))
        .map(str::to_string)
        .collect();
    let rejected: Vec<&str> = env_map
        .keys()
        .map(String::as_str)
        .filter(|name| !always_allowed.contains(name))
        .filter(|name| !allowed.contains(name))
        .collect();
    if !rejected.is_empty() {
        fail_config(&format!(
            "SAT_CONFIG_REPLAY rejects env overrides by default; disallowed or unknown variables: {}",
            rejected.join(",")
        ));
    }
    overrides.sort();
    overrides
}

fn allowed_env_vars() -> Vec<&'static str> {
    vec![
        "SAT_STATS_JSON",
        "SAT_STATS_HOT",
        "SAT_TRACE_FULL",
        "SAT_TRACE_PROOF",
        "SAT_TRACE_PREPROCESS",
        "SAT_TRACE_PREPROCESS_DETAILS",
        "SAT_TRACE_SEARCH_INTERVAL",
        "SAT_CHECK_INVARIANTS",
        "SAT_SEED",
        "SAT_PROFILE",
        "SAT_SEARCH_AXIS",
        "SAT_PREPROCESS_AXIS",
        "SAT_PROOF",
        "SAT_CONFIG_DUMP",
        "SAT_CONFIG_OUT",
        "SAT_CONFIG_REPLAY",
        "SAT_CONFIG_REPLAY_ALLOW_OVERRIDES",
        "SAT_STRICT_CONFIG",
        "SAT_RUN_LABEL",
        "SAT_LIMIT_CONFLICTS",
        "SAT_LIMIT_PROPAGATIONS",
        "SAT_LIMIT_TICKS",
        "SAT_LIMIT_WALL_SEC",
        "SAT_LIMIT_RSS_MB",
        "SAT_LIMIT_LEARNED_LITS",
        "SAT_LIMIT_BINARY_CLAUSES",
        "SAT_LIMIT_EXTENSION_BYTES",
        "SAT_LIMIT_PROOF_BYTES",
        "SAT_USE_LBD",
        "SAT_LBD_UPDATE_REASONS",
        "SAT_LBD_UPDATE_PROP_REASONS",
        "SAT_RESTART",
        "SAT_RESTART_BLOCK_MARGIN",
        "SAT_EMA_SLOW_WINDOW",
        "SAT_RESTART_REUSE_TRAIL",
        "SAT_RESTART_REUSE_TRAIL_FOCUSED",
        "SAT_RESTART_REUSE_TRAIL_STABLE",
        "SAT_REDUCE",
        "SAT_PHASE",
        "SAT_FOCUSED_PHASE",
        "SAT_STABLE_PHASE",
        "SAT_SEARCH_MODE",
        "SAT_MODE_USE_TICKS",
        "SAT_LUCKY",
        "SAT_WARMUP",
        "SAT_BUMP_REASONS",
        "SAT_BUMP_REASONS_LIMIT",
        "SAT_CHRONO",
        "SAT_BINARY_FAST",
        "SAT_CLAUSE_MIN",
        "SAT_OTFS",
        "SAT_OTSS",
        "SAT_VMTF",
        "SAT_REPHASE",
        "SAT_REORDER",
        "SAT_MINIMIZE_DEPTH_LIMIT",
        "SAT_CHRONO_MAX_DELTA",
        "SAT_MODE_INIT_CONFLICTS",
        "SAT_MODE_INTERVAL_SCALE",
        "SAT_VAR_DECAY_FOCUSED",
        "SAT_VAR_DECAY_STABLE",
        "SAT_REPHASE_INIT_CONFLICTS",
        "SAT_REORDER_INTERVAL_CONFLICTS",
        "SAT_SIMPLIFICATION",
        "SAT_BVE",
        "SAT_FULL_BSR",
        "SAT_BSR_FORMULA_GATE",
        "SAT_INPROCESS",
        "SAT_VIVIFY",
        "SAT_PROBE",
        "SAT_HBR",
        "SAT_TRANSITIVE",
        "SAT_FORWARD_SUBSUME",
        "SAT_GATE_EXTRACT",
        "SAT_GATE_BVE",
        "SAT_RCHECK",
        "SAT_INPROCESS_INTERVAL_CONFLICTS",
        "SAT_INPROCESS_MAX_ROUNDS",
        "SAT_VIVIFY_TICKS",
        "SAT_VIVIFY_MAX_CLAUSE_LEN",
        "SAT_PROBE_TICKS",
        "SAT_ELIMINATE_TICKS",
        "SAT_ELIMINATE_RESOLUTIONS",
        "SAT_TRANSITIVE_MAX_DEPTH",
        "SAT_TRANSITIVE_TICKS_PER_SOURCE",
        "SAT_TRANSITIVE_MAX_REMOVED_PER_ROUND",
        "SAT_RCHECK_TICKS",
        "SAT_INITIAL_CLAUSE_MODE",
        "SAT_BRANCH_MODE",
        "SAT_CONFLICT_ANALYSIS_MODE",
        "SAT_CCMIN_MODE",
        "SAT_REDUCE_DB_INIT",
        "SAT_REDUCE_DB_INTERVAL",
        "SAT_REDUCE_MIN_INTERVAL",
        "SAT_POST_PREPROCESS_REDUCE_DB_RESET",
        "SAT_SUBSUMPTION_LIMIT",
    ]
}

fn all_sat_env_keys(env_map: &BTreeMap<String, String>) -> Vec<String> {
    env_map.keys().cloned().collect()
}

fn get_selected<'a>(
    env_map: &'a BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
) -> Option<&'a str> {
    if key_set.contains(name) {
        env_map.get(name).map(String::as_str)
    } else {
        None
    }
}

fn parse_bool_map(env_map: &BTreeMap<String, String>, name: &str, default: bool) -> bool {
    env_map
        .get(name)
        .map(|value| parse_bool_value(name, value))
        .unwrap_or(default)
}

fn parse_bool_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: bool,
) -> bool {
    get_selected(env_map, key_set, name)
        .map(|value| parse_bool_value(name, value))
        .unwrap_or(default)
}

fn parse_bool_value(name: &str, value: &str) -> bool {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "enabled" => true,
        "0" | "false" | "no" | "off" | "disabled" => false,
        other => fail_config(&format!("Invalid {name}={other}; expected boolean")),
    }
}

fn parse_conflict_analysis_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    default: bool,
) -> bool {
    get_selected(env_map, key_set, "SAT_CONFLICT_ANALYSIS_MODE")
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "minisat" | "mini" | "seen" | "false" | "off" | "0" => false,
            "resolved" | "solver10" | "legacy" | "true" | "on" | "1" => {
                fail_config("SAT_CONFLICT_ANALYSIS_MODE=resolved is retired; use minisat")
            }
            other => fail_config(&format!(
                "Invalid SAT_CONFLICT_ANALYSIS_MODE={other}; expected minisat"
            )),
        })
        .unwrap_or(default)
}

fn parse_enum_selected<T>(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: T,
    parse: fn(&str, &str) -> T,
) -> T {
    get_selected(env_map, key_set, name)
        .map(|value| parse(value, name))
        .unwrap_or(default)
}

fn parse_string_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: Option<String>,
) -> Option<String> {
    get_selected(env_map, key_set, name)
        .map(|value| value.to_string())
        .or(default)
}

fn parse_path_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: Option<PathBuf>,
) -> Option<PathBuf> {
    get_selected(env_map, key_set, name)
        .map(PathBuf::from)
        .or(default)
}

fn parse_u32_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: u32,
) -> u32 {
    get_selected(env_map, key_set, name)
        .map(|value| parse_u32_value(name, value))
        .unwrap_or(default)
}

fn parse_u64_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: u64,
) -> u64 {
    get_selected(env_map, key_set, name)
        .map(|value| parse_u64_value(name, value))
        .unwrap_or(default)
}

fn parse_usize_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: usize,
) -> usize {
    get_selected(env_map, key_set, name)
        .map(|value| parse_usize_value(name, value))
        .unwrap_or(default)
}

fn parse_f64_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: f64,
) -> f64 {
    get_selected(env_map, key_set, name)
        .map(|value| parse_f64_value(name, value))
        .unwrap_or(default)
}

fn parse_option_bool_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: Option<bool>,
) -> Option<bool> {
    get_selected(env_map, key_set, name)
        .map(|value| {
            if value.is_empty() {
                None
            } else {
                Some(parse_bool_value(name, value))
            }
        })
        .unwrap_or(default)
}

fn parse_option_phase_policy_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: Option<PhasePolicy>,
) -> Option<PhasePolicy> {
    get_selected(env_map, key_set, name)
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" | "default" | "none" => None,
            _ => Some(PhasePolicy::parse(value, name)),
        })
        .unwrap_or(default)
}

fn parse_option_u64_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: Option<u64>,
) -> Option<u64> {
    get_selected(env_map, key_set, name)
        .map(|value| {
            if value.is_empty() {
                None
            } else {
                Some(parse_u64_value(name, value))
            }
        })
        .unwrap_or(default)
}

fn parse_option_usize_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: Option<usize>,
) -> Option<usize> {
    get_selected(env_map, key_set, name)
        .map(|value| {
            if value.is_empty() {
                None
            } else {
                Some(parse_usize_value(name, value))
            }
        })
        .unwrap_or(default)
}

fn parse_option_isize_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: Option<isize>,
) -> Option<isize> {
    get_selected(env_map, key_set, name)
        .map(|value| {
            if value.is_empty() {
                None
            } else {
                Some(parse_isize_value(name, value))
            }
        })
        .unwrap_or(default)
}

fn parse_option_f64_selected(
    env_map: &BTreeMap<String, String>,
    key_set: &BTreeSet<&str>,
    name: &str,
    default: Option<f64>,
) -> Option<f64> {
    get_selected(env_map, key_set, name)
        .map(|value| {
            if value.is_empty() {
                None
            } else {
                Some(parse_f64_value(name, value))
            }
        })
        .unwrap_or(default)
}

fn parse_u32_value(name: &str, value: &str) -> u32 {
    value
        .trim()
        .parse::<u32>()
        .unwrap_or_else(|err| fail_config(&format!("Invalid {name}={value:?}: {err}")))
}

fn parse_u64_value(name: &str, value: &str) -> u64 {
    value
        .trim()
        .parse::<u64>()
        .unwrap_or_else(|err| fail_config(&format!("Invalid {name}={value:?}: {err}")))
}

fn parse_usize_value(name: &str, value: &str) -> usize {
    value
        .trim()
        .parse::<usize>()
        .unwrap_or_else(|err| fail_config(&format!("Invalid {name}={value:?}: {err}")))
}

fn parse_isize_value(name: &str, value: &str) -> isize {
    value
        .trim()
        .parse::<isize>()
        .unwrap_or_else(|err| fail_config(&format!("Invalid {name}={value:?}: {err}")))
}

fn parse_f64_value(name: &str, value: &str) -> f64 {
    let parsed = value
        .trim()
        .parse::<f64>()
        .unwrap_or_else(|err| fail_config(&format!("Invalid {name}={value:?}: {err}")));
    if !parsed.is_finite() || parsed < 0.0 {
        fail_config(&format!(
            "Invalid {name}={value:?}: expected finite non-negative float"
        ));
    }
    parsed
}

fn push_kv(lines: &mut Vec<String>, key: &str, value: impl AsRef<str>) {
    lines.push(format!("{key}={}", encode_replay_value(value.as_ref())));
}

fn push_kv_bool(lines: &mut Vec<String>, key: &str, value: bool) {
    push_kv(lines, key, if value { "true" } else { "false" });
}

fn push_kv_path(lines: &mut Vec<String>, key: &str, value: Option<&PathBuf>) {
    match value {
        Some(path) => push_kv(lines, key, path.display().to_string()),
        None => push_kv(lines, key, ""),
    }
}

fn push_kv_option_string(lines: &mut Vec<String>, key: &str, value: Option<&str>) {
    push_kv(lines, key, value.unwrap_or(""));
}

fn push_kv_option_bool(lines: &mut Vec<String>, key: &str, value: Option<bool>) {
    match value {
        Some(value) => push_kv_bool(lines, key, value),
        None => push_kv(lines, key, ""),
    }
}

fn push_kv_option_phase_policy(lines: &mut Vec<String>, key: &str, value: Option<PhasePolicy>) {
    match value {
        Some(value) => push_kv(lines, key, value.as_str()),
        None => push_kv(lines, key, ""),
    }
}

fn push_kv_option_u64(lines: &mut Vec<String>, key: &str, value: Option<u64>) {
    match value {
        Some(value) => push_kv(lines, key, value.to_string()),
        None => push_kv(lines, key, ""),
    }
}

fn push_kv_option_usize(lines: &mut Vec<String>, key: &str, value: Option<usize>) {
    match value {
        Some(value) => push_kv(lines, key, value.to_string()),
        None => push_kv(lines, key, ""),
    }
}

fn push_kv_option_isize(lines: &mut Vec<String>, key: &str, value: Option<isize>) {
    match value {
        Some(value) => push_kv(lines, key, value.to_string()),
        None => push_kv(lines, key, ""),
    }
}

fn push_kv_option_f64(lines: &mut Vec<String>, key: &str, value: Option<f64>) {
    match value {
        Some(value) => push_kv(lines, key, format_f64(value)),
        None => push_kv(lines, key, ""),
    }
}

fn push_kv_list(lines: &mut Vec<String>, key: &str, values: &[String]) {
    push_kv(lines, key, values.join(","));
}

fn format_f64(value: f64) -> String {
    let mut text = format!("{value:.12}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.push('0');
    }
    text
}

#[cfg(test)]
fn push_json_field(out: &mut String, key: &str, value: &str, comma: bool) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":\"");
    out.push_str(&json_escape(value));
    out.push('"');
    if comma {
        out.push(',');
    }
}

#[cfg(test)]
fn push_json_bool_field(out: &mut String, key: &str, value: bool, comma: bool) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":");
    out.push_str(if value { "true" } else { "false" });
    if comma {
        out.push(',');
    }
}

#[cfg(test)]
fn push_json_number_field(out: &mut String, key: &str, value: u64, comma: bool) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":");
    out.push_str(&value.to_string());
    if comma {
        out.push(',');
    }
}

#[cfg(test)]
fn push_json_string_array_field(out: &mut String, key: &str, values: &[String], comma: bool) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":[");
    for (idx, value) in values.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(value));
        out.push('"');
    }
    out.push(']');
    if comma {
        out.push(',');
    }
}

#[cfg(test)]
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

fn fail_config(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn env_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn temp_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        env::temp_dir().join(format!(
            "sat-playground-config-{label}-{}-{nanos}.cfg",
            std::process::id()
        ))
    }

    #[test]
    fn test_default_config_uses_solver10_compatible_search_defaults() {
        let config = SolverConfig::from_env_map(&env_map(&[]));

        assert_eq!(config.profile, SolverProfile::Default);
        assert_eq!(config.axes.search, SearchAxis::Validated);
        assert_eq!(config.axes.preprocess, PreprocessAxis::Conservative);
        assert!(config.simplification);
        assert!(config.bve);
        assert!(config.full_bsr);
        assert!(!config.use_lbd);
        assert_eq!(config.search_mode_policy, SearchModePolicy::Single);
        assert!(!config.mode_use_ticks);
        assert_eq!(config.proof_policy, ProofPolicy::Drat);
        assert_eq!(
            config.initial_clause_mode,
            InitialClauseMode::CanonicalSorted
        );
    }

    #[test]
    fn test_baseline_profile_disables_existing_preprocess_controls() {
        let config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));

        assert_eq!(config.profile, SolverProfile::Baseline);
        assert_eq!(config.axes.preprocess, PreprocessAxis::Off);
        assert!(!config.simplification);
        assert!(!config.bve);
        assert!(!config.full_bsr);
        assert!(!config.use_lbd);
        assert_eq!(config.search_mode_policy, SearchModePolicy::Single);
        assert!(!config.mode_use_ticks);
        assert_eq!(
            config.initial_clause_mode,
            InitialClauseMode::CanonicalSorted
        );
    }

    #[test]
    fn test_initial_clause_mode_profiles_and_overrides() {
        let default_config = SolverConfig::from_env_map(&env_map(&[]));
        let fast = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "fast")]));
        let explicit_raw =
            SolverConfig::from_env_map(&env_map(&[("SAT_INITIAL_CLAUSE_MODE", "raw")]));
        let explicit_auto =
            SolverConfig::from_env_map(&env_map(&[("SAT_INITIAL_CLAUSE_MODE", "auto")]));
        let explicit_kissat_watch =
            SolverConfig::from_env_map(&env_map(&[("SAT_INITIAL_CLAUSE_MODE", "kissat-watch")]));

        assert_eq!(
            default_config.initial_clause_mode,
            InitialClauseMode::CanonicalSorted
        );
        assert_eq!(fast.initial_clause_mode, InitialClauseMode::CanonicalSorted);
        assert_eq!(explicit_raw.initial_clause_mode, InitialClauseMode::Raw);
        assert_eq!(explicit_auto.initial_clause_mode, InitialClauseMode::Auto);
        assert_eq!(
            explicit_kissat_watch.initial_clause_mode,
            InitialClauseMode::KissatWatch
        );
        assert!(explicit_kissat_watch
            .config_replay_text()
            .contains("initial_clause_mode=kissat-watch"));
        assert!(default_config
            .config_replay_text()
            .contains("initial_clause_mode=canonical-sorted"));
    }

    #[test]
    fn test_axis_override_reports_experimental_profile() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_PROFILE", "fast"),
            ("SAT_PREPROCESS_AXIS", "conservative"),
        ]));

        assert_eq!(config.profile, SolverProfile::Experimental);
        assert_eq!(config.axes.search, SearchAxis::Strong);
        assert_eq!(config.axes.preprocess, PreprocessAxis::Conservative);
    }

    #[test]
    fn test_default_profiles_do_not_bundle_target_phase_with_ema_restart() {
        for profile in ["baseline", "default", "fast", "experimental"] {
            let config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", profile)]));

            if config.restart_policy == RestartPolicy::KissatEma {
                assert_ne!(config.phase_policy, PhasePolicy::TargetThenSaved);
                assert_ne!(config.phase_policy, PhasePolicy::BestThenTargetThenSaved);
            }
        }
    }

    #[test]
    fn test_target_phase_remains_explicit_opt_in_when_ema_restart_is_active() {
        let ema_default_phase = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_RESTART", "kissat-ema"),
        ]));
        assert_eq!(ema_default_phase.restart_policy, RestartPolicy::KissatEma);
        assert_eq!(ema_default_phase.phase_policy, PhasePolicy::Legacy);

        let explicit_target = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_RESTART", "kissat-ema"),
            ("SAT_PHASE", "target-then-saved"),
        ]));
        assert_eq!(explicit_target.restart_policy, RestartPolicy::KissatEma);
        assert_eq!(explicit_target.phase_policy, PhasePolicy::TargetThenSaved);
    }

    #[test]
    fn test_config_hash_changes_with_effective_feature_flag() {
        let off = SolverConfig::from_env_map(&env_map(&[
            ("SAT_SEARCH_MODE", "single"),
            ("SAT_MODE_USE_TICKS", "off"),
            ("SAT_USE_LBD", "off"),
        ]));
        let on = SolverConfig::from_env_map(&env_map(&[("SAT_USE_LBD", "on")]));

        assert_ne!(off.config_hash(), on.config_hash());
    }

    #[test]
    fn test_lbd_reason_update_and_tiered_reduce_are_runtime_supported() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_LBD_UPDATE_REASONS", "on"),
            ("SAT_REDUCE", "lbd-tiered"),
        ]));

        assert!(config.use_lbd);
        assert!(config.update_reason_lbd);
        assert!(!config.update_propagation_reason_lbd);
        assert_eq!(config.reduce_policy, ReducePolicy::LbdTiered);
    }

    #[test]
    fn test_propagation_lbd_reason_update_is_runtime_supported_with_reason_update() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_LBD_UPDATE_REASONS", "on"),
            ("SAT_LBD_UPDATE_PROP_REASONS", "on"),
            ("SAT_REDUCE", "lbd-tiered"),
        ]));

        assert!(config.use_lbd);
        assert!(config.update_reason_lbd);
        assert!(config.update_propagation_reason_lbd);
        assert_eq!(config.reduce_policy, ReducePolicy::LbdTiered);
    }

    #[test]
    fn test_kissat_ema_restart_is_runtime_supported_with_lbd() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_RESTART", "kissat-ema"),
            ("SAT_RESTART_BLOCK_MARGIN", "1.25"),
        ]));

        assert!(config.use_lbd);
        assert_eq!(config.restart_policy, RestartPolicy::KissatEma);
        assert_eq!(config.restart_block_margin, 1.25);
    }

    #[test]
    fn test_ema_slow_window_is_runtime_supported_and_replayable() {
        let default_config = SolverConfig::from_env_map(&env_map(&[]));
        assert_eq!(default_config.restart_slow_window, 4096);

        let schema_row = CONFIG_SCHEMA_CSV
            .lines()
            .find(|line| line.starts_with("SAT_EMA_SLOW_WINDOW,"))
            .expect("SAT_EMA_SLOW_WINDOW schema row");
        let columns: Vec<&str> = schema_row.split(',').collect();
        assert_eq!(columns[4], "4096");
        assert_eq!(columns[5], "4096");
        assert_eq!(columns[6], "4096");

        let config = SolverConfig::from_env_map(&env_map(&[("SAT_EMA_SLOW_WINDOW", "100000")]));

        assert_eq!(config.restart_slow_window, 100000);
        let replay = config.config_replay_text();
        assert!(replay.contains("restart_slow_window=100000"));

        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<ema-slow-window-test>"));
        assert_eq!(replayed.restart_slow_window, config.restart_slow_window);
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_lbd_tiered_reduce_min_interval_is_runtime_supported() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_REDUCE", "lbd-tiered"),
            ("SAT_REDUCE_MIN_INTERVAL", "200"),
        ]));

        assert_eq!(config.reduce_policy, ReducePolicy::LbdTiered);
        assert_eq!(config.reduce_min_interval, Some(200));
    }

    #[test]
    fn test_reluctant_restart_is_runtime_supported() {
        let config = SolverConfig::from_env_map(&env_map(&[("SAT_RESTART", "reluctant")]));

        assert_eq!(config.restart_policy, RestartPolicy::Reluctant);
    }

    #[test]
    fn test_focused_stable_search_mode_is_runtime_supported_with_lbd() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
        ]));

        assert!(config.use_lbd);
        assert_eq!(config.search_mode_policy, SearchModePolicy::FocusedStable);
        assert_eq!(config.vmtf, VmtfMode::FocusedOnly);
    }

    #[test]
    fn test_focused_stable_search_mode_honors_explicit_vmtf_off() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_VMTF", "off"),
        ]));

        assert_eq!(config.search_mode_policy, SearchModePolicy::FocusedStable);
        assert_eq!(config.vmtf, VmtfMode::Off);
    }

    #[test]
    fn test_mode_use_ticks_is_runtime_supported_with_focused_stable_mode() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_MODE_USE_TICKS", "on"),
        ]));

        assert!(config.mode_use_ticks);
        assert_eq!(config.search_mode_policy, SearchModePolicy::FocusedStable);
    }

    #[test]
    fn test_vmtf_focused_only_is_runtime_supported_with_focused_stable_mode() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_VMTF", "on"),
        ]));

        assert_eq!(config.vmtf, VmtfMode::FocusedOnly);
        assert_eq!(config.search_mode_policy, SearchModePolicy::FocusedStable);
    }

    #[test]
    fn test_vmtf_single_mode_is_runtime_supported_without_focused_stable() {
        let config = SolverConfig::from_env_map(&env_map(&[("SAT_VMTF", "single")]));

        assert_eq!(config.vmtf, VmtfMode::Single);
        assert_eq!(config.search_mode_policy, SearchModePolicy::Single);

        let replay = config.config_replay_text();
        assert!(replay.contains("vmtf=single"));

        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<vmtf-single-test>"));
        assert_eq!(replayed.vmtf, VmtfMode::Single);
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_rephase_is_runtime_supported_with_focused_stable_mode() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_REPHASE", "on"),
            ("SAT_REPHASE_INIT_CONFLICTS", "17"),
        ]));

        assert!(config.rephase);
        assert_eq!(config.search_mode_policy, SearchModePolicy::FocusedStable);
        assert_eq!(config.rephase_init_conflicts, 17);
    }

    #[test]
    fn test_var_decay_per_mode_controls_are_replayable() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_VAR_DECAY_FOCUSED", "0.91"),
            ("SAT_VAR_DECAY_STABLE", "0.997"),
        ]));

        assert_eq!(config.focused_activity_decay, 0.91);
        assert_eq!(config.stable_activity_decay, 0.997);

        let replay = config.config_replay_text();
        assert!(replay.contains("focused_activity_decay=0.91"));
        assert!(replay.contains("stable_activity_decay=0.997"));

        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<var-decay-test>"));
        assert_eq!(
            replayed.focused_activity_decay,
            config.focused_activity_decay
        );
        assert_eq!(replayed.stable_activity_decay, config.stable_activity_decay);
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_reorder_is_runtime_supported_and_replayable() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_REORDER", "on"),
            ("SAT_REORDER_INTERVAL_CONFLICTS", "37"),
        ]));

        assert!(config.reorder);
        assert_eq!(config.reorder_interval_conflicts, 37);
        let replay = config.config_replay_text();
        assert!(replay.contains("reorder=true"));
        assert!(replay.contains("reorder_interval_conflicts=37"));

        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<reorder-test>"));
        assert!(replayed.reorder);
        assert_eq!(
            replayed.reorder_interval_conflicts,
            config.reorder_interval_conflicts
        );
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_chrono_backtrack_is_runtime_supported() {
        let default_config = SolverConfig::from_env_map(&env_map(&[]));
        assert_eq!(DEFAULT_CHRONO_MAX_DELTA, 5_000);
        assert_eq!(default_config.chrono_max_delta, DEFAULT_CHRONO_MAX_DELTA);

        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_CHRONO", "on"),
            ("SAT_CHRONO_MAX_DELTA", "7"),
        ]));

        assert!(config.chrono_backtrack);
        assert_eq!(config.chrono_max_delta, 7);
    }

    #[test]
    fn test_lucky_defaults_off_and_can_be_enabled() {
        let config = SolverConfig::from_env_map(&env_map(&[]));
        assert!(!config.lucky);

        let fast = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "fast")]));
        assert!(!fast.lucky);

        let baseline = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
        assert!(!baseline.lucky);

        let enabled = SolverConfig::from_env_map(&env_map(&[("SAT_LUCKY", "on")]));
        assert!(enabled.lucky);
    }

    #[test]
    fn test_lucky_is_replayable() {
        let config = SolverConfig::from_env_map(&env_map(&[("SAT_LUCKY", "off")]));
        let replay = config.config_replay_text();
        assert!(replay.contains("lucky=false"));

        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<lucky-test>"));
        assert!(!replayed.lucky);
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_warmup_defaults_off_and_can_be_enabled() {
        let config = SolverConfig::from_env_map(&env_map(&[]));
        assert!(!config.warmup);

        let fast = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "fast")]));
        assert!(!fast.warmup);

        let baseline = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
        assert!(!baseline.warmup);

        let enabled = SolverConfig::from_env_map(&env_map(&[("SAT_WARMUP", "on")]));
        assert!(enabled.warmup);
    }

    #[test]
    fn test_warmup_is_replayable() {
        let config = SolverConfig::from_env_map(&env_map(&[("SAT_WARMUP", "on")]));
        let replay = config.config_replay_text();
        assert!(replay.contains("warmup=true"));

        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<warmup-test>"));
        assert!(replayed.warmup);
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_bump_reasons_defaults_off_and_can_be_enabled() {
        let config = SolverConfig::from_env_map(&env_map(&[]));
        assert!(!config.bump_reasons);
        assert_eq!(config.bump_reasons_limit_multiplier, 10);

        let fast = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "fast")]));
        assert!(!fast.bump_reasons);

        let baseline = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
        assert!(!baseline.bump_reasons);

        let enabled = SolverConfig::from_env_map(&env_map(&[("SAT_BUMP_REASONS", "on")]));
        assert!(enabled.bump_reasons);

        let custom_limit = SolverConfig::from_env_map(&env_map(&[
            ("SAT_BUMP_REASONS", "on"),
            ("SAT_BUMP_REASONS_LIMIT", "25"),
        ]));
        assert!(custom_limit.bump_reasons);
        assert_eq!(custom_limit.bump_reasons_limit_multiplier, 25);
    }

    #[test]
    fn test_bump_reasons_is_replayable() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_BUMP_REASONS", "on"),
            ("SAT_BUMP_REASONS_LIMIT", "5"),
        ]));
        let replay = config.config_replay_text();
        assert!(replay.contains("bump_reasons=true"));
        assert!(replay.contains("bump_reasons_limit_multiplier=5"));

        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<bump-reasons-test>"));
        assert!(replayed.bump_reasons);
        assert_eq!(replayed.bump_reasons_limit_multiplier, 5);
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_minimize_depth_limit_default_matches_kissat() {
        let config = SolverConfig::from_env_map(&env_map(&[]));
        assert_eq!(config.minimize_depth_limit, 1_000);

        let schema_row = CONFIG_SCHEMA_CSV
            .lines()
            .find(|line| line.starts_with("SAT_MINIMIZE_DEPTH_LIMIT,"))
            .expect("SAT_MINIMIZE_DEPTH_LIMIT schema row");
        let columns: Vec<&str> = schema_row.split(',').collect();
        assert_eq!(columns[4], "1000");
        assert_eq!(columns[5], "1000");
        assert_eq!(columns[6], "1000");
    }

    #[test]
    fn test_minimize_depth_limit_can_be_overridden_and_replayed() {
        let config = SolverConfig::from_env_map(&env_map(&[("SAT_MINIMIZE_DEPTH_LIMIT", "4096")]));
        assert_eq!(config.minimize_depth_limit, 4096);

        let replay = config.config_replay_text();
        assert!(replay.contains("minimize_depth_limit=4096"));

        let replayed =
            SolverConfig::from_replay_text(&replay, Path::new("<minimize-depth-limit-test>"));
        assert_eq!(replayed.minimize_depth_limit, config.minimize_depth_limit);
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_binary_fast_path_preserves_default_clause_minimization() {
        let config = SolverConfig::from_env_map(&env_map(&[("SAT_BINARY_FAST", "on")]));

        assert!(config.binary_fast_path);
        assert_eq!(config.clause_min_mode, ClauseMinMode::RecursiveLimited);
    }

    #[test]
    fn test_binary_fast_path_honors_explicit_clause_minimization() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_BINARY_FAST", "on"),
            ("SAT_CLAUSE_MIN", "recursive-limited"),
        ]));

        assert!(config.binary_fast_path);
        assert_eq!(config.clause_min_mode, ClauseMinMode::RecursiveLimited);
    }

    #[test]
    fn test_binary_fast_path_honors_explicit_clause_minimization_off() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_BINARY_FAST", "on"),
            ("SAT_CLAUSE_MIN", "off"),
        ]));

        assert!(config.binary_fast_path);
        assert_eq!(config.clause_min_mode, ClauseMinMode::Off);
    }

    #[test]
    fn test_inblock_clause_minimization_is_runtime_supported() {
        let config = SolverConfig::from_env_map(&env_map(&[("SAT_CLAUSE_MIN", "inblock")]));

        assert_eq!(config.clause_min_mode, ClauseMinMode::InBlockShrink);
    }

    #[test]
    fn test_otfs_is_runtime_supported_with_clause_minimization() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_CLAUSE_MIN", "recursive-limited"),
            ("SAT_OTFS", "on"),
        ]));

        assert!(config.otfs);
        assert_eq!(config.clause_min_mode, ClauseMinMode::RecursiveLimited);
    }

    #[test]
    fn test_phase_policies_are_runtime_supported() {
        let saved = SolverConfig::from_env_map(&env_map(&[("SAT_PHASE", "saved")]));
        let target = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_PHASE", "target-then-saved"),
        ]));
        let best = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_PHASE", "best-then-target-then-saved"),
        ]));

        assert_eq!(saved.phase_policy, PhasePolicy::Saved);
        assert_eq!(target.phase_policy, PhasePolicy::TargetThenSaved);
        assert_eq!(best.phase_policy, PhasePolicy::BestThenTargetThenSaved);
    }

    #[test]
    fn test_focused_stable_phase_map_controls_are_replayable() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_FOCUSED_PHASE", "target-then-saved"),
            ("SAT_STABLE_PHASE", "saved"),
        ]));

        assert_eq!(config.search_mode_policy, SearchModePolicy::FocusedStable);
        assert_eq!(
            config.focused_phase_policy,
            Some(PhasePolicy::TargetThenSaved)
        );
        assert_eq!(config.stable_phase_policy, Some(PhasePolicy::Saved));

        let replay = config.config_replay_text();
        assert!(replay.contains("focused_phase_policy=target-then-saved"));
        assert!(replay.contains("stable_phase_policy=saved"));

        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<phase-map-test>"));
        assert_eq!(replayed.focused_phase_policy, config.focused_phase_policy);
        assert_eq!(replayed.stable_phase_policy, config.stable_phase_policy);
        assert_eq!(replayed.config_hash(), config.config_hash());

        let auto = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_FOCUSED_PHASE", "auto"),
            ("SAT_STABLE_PHASE", ""),
        ]));
        assert_eq!(auto.focused_phase_policy, None);
        assert_eq!(auto.stable_phase_policy, None);
    }

    #[test]
    fn test_restart_reuse_trail_is_runtime_supported_and_replayable() {
        let config = SolverConfig::from_env_map(&env_map(&[("SAT_RESTART_REUSE_TRAIL", "on")]));

        assert!(config.restart_reuse_trail);
        assert!(config.restart_reuse_trail_focused);
        assert!(config.restart_reuse_trail_stable);

        let replay = config.config_replay_text();
        assert!(replay.contains("restart_reuse_trail=true"));
        assert!(replay.contains("restart_reuse_trail_focused=true"));
        assert!(replay.contains("restart_reuse_trail_stable=true"));

        let replayed =
            SolverConfig::from_replay_text(&replay, Path::new("<restart-reuse-trail-test>"));
        assert!(replayed.restart_reuse_trail);
        assert!(replayed.restart_reuse_trail_focused);
        assert!(replayed.restart_reuse_trail_stable);
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_restart_reuse_trail_per_mode_controls_are_replayable() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_RESTART_REUSE_TRAIL", "on"),
            ("SAT_RESTART_REUSE_TRAIL_FOCUSED", "off"),
            ("SAT_RESTART_REUSE_TRAIL_STABLE", "on"),
        ]));

        assert!(config.restart_reuse_trail);
        assert!(!config.restart_reuse_trail_focused);
        assert!(config.restart_reuse_trail_stable);

        let replay = config.config_replay_text();
        assert!(replay.contains("restart_reuse_trail=true"));
        assert!(replay.contains("restart_reuse_trail_focused=false"));
        assert!(replay.contains("restart_reuse_trail_stable=true"));

        let replayed =
            SolverConfig::from_replay_text(&replay, Path::new("<restart-reuse-mode-test>"));
        assert_eq!(replayed.restart_reuse_trail, config.restart_reuse_trail);
        assert_eq!(
            replayed.restart_reuse_trail_focused,
            config.restart_reuse_trail_focused
        );
        assert_eq!(
            replayed.restart_reuse_trail_stable,
            config.restart_reuse_trail_stable
        );
        assert_eq!(replayed.config_hash(), config.config_hash());

        let focused_only =
            SolverConfig::from_env_map(&env_map(&[("SAT_RESTART_REUSE_TRAIL_FOCUSED", "on")]));
        assert!(!focused_only.restart_reuse_trail);
        assert!(focused_only.restart_reuse_trail_focused);
        assert!(!focused_only.restart_reuse_trail_stable);
    }

    #[test]
    fn test_config_replay_round_trip_preserves_effective_config() {
        let path = temp_path("round-trip");
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_RUN_LABEL", "round-trip"),
            ("SAT_CONFIG_OUT", path.to_str().expect("utf8 temp path")),
        ]));
        config.emit_requested_outputs();

        let replayed = SolverConfig::from_env_map(&env_map(&[(
            "SAT_CONFIG_REPLAY",
            path.to_str().expect("utf8 temp path"),
        )]));

        assert_eq!(replayed.use_lbd, config.use_lbd);
        assert_eq!(replayed.run_label, config.run_label);
        assert_eq!(replayed.config_hash(), config.config_hash());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_default_replay_allows_only_documented_runtime_overrides() {
        let path = temp_path("override");
        let config = SolverConfig::default();
        fs::write(&path, config.config_replay_text()).expect("write replay");

        let replayed = SolverConfig::from_env_map(&env_map(&[
            ("SAT_CONFIG_REPLAY", path.to_str().expect("utf8 temp path")),
            ("SAT_RUN_LABEL", "rerun"),
            ("SAT_STATS_JSON", "on"),
            ("SAT_STATS_HOT", "on"),
            ("SAT_TRACE_PROOF", "on"),
            ("SAT_TRACE_SEARCH_INTERVAL", "1000"),
        ]));

        assert_eq!(replayed.run_label.as_deref(), Some("rerun"));
        assert!(replayed.stats_json);
        assert!(replayed.hot_stats);
        assert!(replayed.trace_proof);
        assert_eq!(replayed.trace_search_interval, 1000);
        assert!(replayed.replay_overridden);
        assert_eq!(
            replayed.replay_override_env,
            vec![
                "SAT_RUN_LABEL",
                "SAT_STATS_HOT",
                "SAT_STATS_JSON",
                "SAT_TRACE_PROOF",
                "SAT_TRACE_SEARCH_INTERVAL"
            ]
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_hot_stats_are_explicit_and_replayable() {
        let default_config = SolverConfig::default();
        assert!(!default_config.hot_stats);

        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_STATS_JSON", "on"),
            ("SAT_STATS_HOT", "on"),
        ]));
        assert!(config.stats_json);
        assert!(config.hot_stats);

        let replay = config.config_replay_text();
        assert!(replay.contains("hot_stats=true"));

        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<hot-stats-test>"));
        assert!(replayed.hot_stats);
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_individual_trace_flags_are_parsed_and_replayable() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_TRACE_PROOF", "on"),
            ("SAT_TRACE_PREPROCESS", "on"),
            ("SAT_TRACE_PREPROCESS_DETAILS", "on"),
            ("SAT_TRACE_SEARCH_INTERVAL", "12345"),
        ]));

        assert!(!config.trace_full);
        assert!(config.trace_proof);
        assert!(config.trace_preprocess);
        assert!(config.trace_preprocess_details);
        assert_eq!(config.trace_search_interval, 12345);

        let replay = config.config_replay_text();
        assert!(replay.contains("trace_proof=true"));
        assert!(replay.contains("trace_preprocess=true"));
        assert!(replay.contains("trace_preprocess_details=true"));
        assert!(replay.contains("trace_search_interval=12345"));

        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<trace-test>"));
        assert_eq!(replayed.trace_proof, config.trace_proof);
        assert_eq!(replayed.trace_preprocess, config.trace_preprocess);
        assert_eq!(
            replayed.trace_preprocess_details,
            config.trace_preprocess_details
        );
        assert_eq!(replayed.trace_search_interval, config.trace_search_interval);
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_trace_full_still_implies_preprocess_detail_tracing() {
        let config = SolverConfig::from_env_map(&env_map(&[("SAT_TRACE_FULL", "on")]));

        assert!(config.trace_full);
        assert!(config.trace_preprocess);
        assert!(config.trace_preprocess_details);
        assert!(!config.trace_proof);
        assert_eq!(config.trace_search_interval, 0);
    }

    #[test]
    fn test_eliminate_resolution_budget_is_parsed_and_replayable() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_ELIMINATE_RESOLUTIONS", "1234"),
            ("SAT_ELIMINATE_TICKS", "5678"),
        ]));

        assert_eq!(config.eliminate_resolution_budget, 1234);
        assert_eq!(config.eliminate_ticks_budget, 5678);

        let replay = config.config_replay_text();
        assert!(replay.contains("eliminate_resolution_budget=1234"));
        let replayed =
            SolverConfig::from_replay_text(&replay, Path::new("<eliminate-budget-test>"));
        assert_eq!(
            replayed.eliminate_resolution_budget,
            config.eliminate_resolution_budget
        );
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_schema_and_feature_csv_are_loaded_into_binary() {
        assert!(CONFIG_SCHEMA_CSV.contains("SAT_USE_LBD"));
        assert!(CONFIG_SCHEMA_CSV.contains("SAT_CONFIG_REPLAY"));
        assert!(CONFIG_SCHEMA_CSV.contains("SAT_ELIMINATE_RESOLUTIONS"));
        assert!(FEATURES_CSV.contains("SAT_USE_LBD"));
        assert!(FEATURES_CSV.contains("SAT_FULL_BSR"));
    }

    #[test]
    fn test_json_stats_line_contains_config_identity() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_STATS_JSON", "on"),
            ("SAT_USE_LBD", "on"),
        ]));
        let line = config.json_stats_line("SAT");

        assert!(line.starts_with("c JSON_STATS "));
        assert!(line.contains("\"config_hash\":\""));
        assert!(line.contains("\"profile\":\"default\""));
        assert!(line.contains("\"name\":\"SAT_USE_LBD\""));
    }

    #[test]
    fn test_legacy_aliases_are_recorded_in_dump_and_hash_surface() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_CCMIN_MODE", "1"),
            ("SAT_BRANCH_MODE", "occurrence"),
        ]));
        let dump = config.config_replay_text();

        assert!(dump.contains("legacy_aliases_used=SAT_BRANCH_MODE,SAT_CCMIN_MODE"));
        assert_eq!(config.clause_min_mode, ClauseMinMode::Basic);
        assert_eq!(config.branch_mode, BranchMode::Occurrence);
    }

    #[test]
    fn test_allowed_env_list_covers_schema_csv_rows() {
        let allowed: BTreeSet<&str> = allowed_env_vars().into_iter().collect();
        for line in CONFIG_SCHEMA_CSV.lines().skip(1) {
            let env_var = line.split(',').next().expect("env_var column");
            if !env_var.is_empty() {
                assert!(
                    allowed.contains(env_var),
                    "schema env missing from allowlist: {env_var}"
                );
            }
        }
    }
}
