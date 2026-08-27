//! Configuration parsing boundary for solver 12.
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
const DEFAULT_MODE_INIT_CONFLICTS: u64 = 3000;
const DEFAULT_MODE_INTERVAL_SCALE: f64 = 1.5;
const DEFAULT_REPHASE_INIT_CONFLICTS: u64 = 1000;
const DEFAULT_WALK_EFFORT_PERMILLE: u64 = 50;
const DEFAULT_REORDER_INTERVAL_CONFLICTS: u64 = 10_000;
const DEFAULT_RESTART_BLOCK_MARGIN: f64 = 0.0;
const DEFAULT_RESTART_SLOW_WINDOW: u64 = 4_096;
const DEFAULT_VAR_DECAY_FOCUSED: f64 = 0.95;
const DEFAULT_VAR_DECAY_STABLE: f64 = 0.95;

const PARKING_LOT_DENYLIST: &[&str] = &["SAT_WALK", "SAT_BCE"];
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
    "SAT_BIN_POOL",
    "SAT_TRACE_PREPROCESS_DETAILS",
    "SAT_TRACE_SEARCH_INTERVAL",
    "SAT_LIMIT_WALL_SEC",
    "SAT_LIMIT_RSS_MB",
];

#[cfg(test)]
pub(crate) const CONFIG_SCHEMA_CSV: &str = include_str!("../CONFIG_SCHEMA.csv");
#[cfg(test)]
pub(crate) const FEATURES_CSV: &str = include_str!("../FEATURES.csv");

// Default eliminate-pass effort budgets (bead SAT-playground-5b2.3.24, analyzesat PRE-1).
// Chosen from the measured natural tick usage across profile20 (2026-06-10, counters active
// via huge budgets; tick counts are work counters — deterministic per formula, consumed
// before any seed-dependent choice, contention-immune): 18 of 20 instances finish within
// 2.77B ticks (max REGRandom 2.766B — an instance where BSR is measured load-bearing, so it
// must stay under budget), while the two runaways sit above (velev 3.72B, Kakuro 10.0B,
// both BSR-harmful per log/analyzesat-2026-05-26-preprocess −79% studies). 3B binds the
// runaways and spares the rest. The resolution budget is an independent BVE safety cap at
// ~1.5× the largest observed legitimate usage (VexRiscv 67.0M attempts); it binds nothing
// on profile20. Explicit 0 remains the unlimited opt-out (and skips counting in the hot
// loop entirely). A wall-clock cap was rejected on purpose: it would break the
// per-(config,seed) determinism of conflicts that the lexicographic metric's tiebreak
// relies on. Kept via the 2026-06-10 5x5 profile20 seedgate vs the unlimited prior default:
// solved 65/100 both (identical cell sets), conflicts tiebreak −0.3% (Kakuro −39.8%,
// velev +16.3%, all other rows bit-identical), PAR-2 +0.33% (noise). Known limits: the
// budget exhausts inside Kakuro's initial BSR so its BVE never runs, and clean_occurs /
// resolvent-materialization work is NOT tick-counted (VexRiscv-shaped BVE-heavy walls stay
// unbounded — see bead 5b2.3.23). Evidence: log/seedgate-elimbudget-{on,off}-2026-06-10-*.
pub(crate) const DEFAULT_ELIMINATE_TICKS_BUDGET: u64 = 3_000_000_000;
pub(crate) const DEFAULT_ELIMINATE_RESOLUTION_BUDGET: u64 = 100_000_000;

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
    /// Binary DRAT (drat-trim `-i` wire format, auto-detected by drat-trim):
    /// 'a'/'d' tag byte, 7-bit varint literals (2*var+sign), 0x00 terminator.
    /// Semantically identical proof stream at ~2.5-3x fewer bytes and no ASCII
    /// formatting cost — pure proof-I/O wall savings on the multi-GB UNSAT
    /// cells (oski20 writes 7.3GB of text DRAT inside a ~1500s solve).
    DratBinary,
    Lrat,
}

impl ProofPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Drat => "drat",
            Self::DratBinary => "drat-binary",
            Self::Lrat => "lrat",
        }
    }

    fn parse(value: &str, env_name: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" | "enabled" | "drat" => Self::Drat,
            "0" | "false" | "no" | "off" | "disabled" => Self::Off,
            "binary" | "drat-binary" | "bindrat" => Self::DratBinary,
            "lrat" => Self::Lrat,
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected off/drat/drat-binary/lrat"
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
    InBlockLate,
}

impl ClauseMinMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Basic => "basic",
            Self::RecursiveLimited => "recursive-limited",
            Self::InBlockShrink => "inblock",
            Self::InBlockLate => "inblock-late",
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
            "inblock-late" | "inblock_late" | "late-inblock" | "late_inblock" => {
                Self::InBlockLate
            }
            other => fail_config(&format!(
                "Invalid {env_name}={other}; expected off/basic/recursive-limited/inblock/inblock-late"
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
    pub(crate) stable_target_reset: bool,
    pub(crate) search_mode_policy: SearchModePolicy,
    pub(crate) mode_use_ticks: bool,
    pub(crate) lucky: bool,
    pub(crate) warmup: bool,
    pub(crate) bump_reasons: bool,
    pub(crate) bump_reasons_limit_multiplier: u32,
    pub(crate) chrono_backtrack: bool,
    pub(crate) binary_fast_path: bool,
    // bead 5b2.8.1: software-prefetch the next watched clause during propagation. Pure cache
    // hint — does not change search results. Default off.
    pub(crate) prefetch_watched_clauses: bool,
    pub(crate) clause_min_mode: ClauseMinMode,
    pub(crate) inblock_delay_conflicts: u64,
    pub(crate) inblock_binary_min: f64,
    pub(crate) otfs: bool,
    /// Opt-in OTSS: on-the-fly self-subsuming resolution. After conflict analysis
    /// produces a learned clause, scan participating reason clauses (those whose
    /// literals were marked during analyze) and delete any that are strict
    /// supersets of the learned clause. Off by default. Bead SAT-playground-5b2.2.39.
    pub(crate) otss: bool,
    /// Opt-in: drop the `!over_budget` gate on tier 2 (high-LBD) candidates in
    /// `reduce_candidate`, so high-LBD learnt clauses enter the candidate pool
    /// on every scheduled reduction, not only emergency runs. Tier 1 keeps its
    /// `!over_budget` protection. Bead SAT-playground-5b2.2.44. NOTE: with the
    /// current delete-until-budget loop in `reduce_db_lbd_tiered`, this flag is
    /// a no-op when `!over_budget` (the delete loop breaks immediately) — see
    /// bd note on 5b2.2.44 for the dependency on a paired delete-loop change
    /// (the broader package was rejected via 5b2.2.59). Default off so the
    /// candidate-pool sort sees no extra entries until the delete loop catches
    /// up.
    pub(crate) reduce_tier2_at_budget: bool,
    /// Opt-in: after `reduce_db_lbd_tiered` removes deleted-clause records, sweep
    /// every watch list and drop watchers whose `clause_idx` is past the arena or
    /// whose clause is deleted. Off by default. Bead SAT-playground-s11-1-14b is
    /// paired with bead s11-1-14a's blocker-fast-path which intentionally
    /// short-circuits stale checks when the blocker is TRUE — so stale-with-TRUE
    /// entries accumulate across reduces and this sweep cleans them up.
    pub(crate) watch_compact_enabled: bool,
    pub(crate) vmtf: VmtfMode,
    pub(crate) rephase: bool,
    /// Restrict the rephase/walk cycle to vivify-yield-armed formulas (the
    /// conflict-dense refutation signature: probe yield + decisions/conflict
    /// <= 3 + !deep_phase). The 2026-07-05 global-rephase A/B lost 10 solved
    /// cells, and the 2026-07-14 screen showed the congruence-armed SAT canary
    /// ibm-2004 regressing +46% conflicts under rephasing — the yield-armed
    /// class is where kissat's rephase/walk evidence lives (49-65 rephases,
    /// 16-22 walks per density cell) and has no SAT trajectory to lose.
    pub(crate) rephase_armed_only: bool,
    /// Enable the ProbSAT walk slots of the kissat rephase schedule
    /// (best, walk, inverted, best, walk, original).
    pub(crate) walk: bool,
    /// Walk effort in permille of search ticks since the last walk
    /// (kissat `walkeffort`, default 50).
    pub(crate) walk_effort_permille: u64,
    /// Warm up the walker's starting assignment kissat-style (warmup.c):
    /// before each walk, complete the root assignment by repeated
    /// decide + propagate-beyond-conflicts (assignments eagerly save phases),
    /// then backtrack to root without touching the saved phases, so the walk
    /// starts from a unit-propagation-consistent completion of the decision
    /// phases instead of the raw saved/target snapshot (kissat `warmup`,
    /// default on there).
    pub(crate) walk_warmup: bool,
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
    // Opt-in experiment: defer BSR queue draining until the outer preprocessing loop,
    // instead of immediately after each successful BVE elimination.
    pub(crate) bsr_drain_batched: bool,
    pub(crate) bsr_occurrence_limit: u64,
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
    /// Scoped gate-aware BVE (SAT_GATE_BVE_SCOPED): at the root, dry-run plain BVE
    /// (E0) and gate-aware BVE (E1) on throwaway sub-solvers, then enable gate_bve
    /// for the real run only when the net elimination gain crosses
    /// `gate_bve_min_gain_pct`. Formulas below the threshold (or above
    /// `gate_bve_scoped_max_vars`) stay byte-identical to the plain baseline.
    pub(crate) gate_bve_scoped: bool,
    /// Minimum net elimination gain in percent (E1/E0 - 1 >= pct/100) for the
    /// scoped pass to adopt gate-aware BVE (SAT_GATE_BVE_MIN_GAIN_PCT).
    pub(crate) gate_bve_min_gain_pct: u64,
    /// Variable-count cap for the scoped dry-run (SAT_GATE_BVE_SCOPED_MAX_VARS):
    /// larger formulas skip the dry-run entirely and keep plain BVE.
    pub(crate) gate_bve_scoped_max_vars: usize,
    pub(crate) rcheck: bool,
    pub(crate) gauss: bool,
    pub(crate) factor: bool,
    pub(crate) pair_abs_refute: bool,
    /// Pigeonhole-counting extended-resolution refutation (SAT_PHP_REFUTE):
    /// detects the relativized-PHP (`rphp`) and clique-coloring (`clqcl`)
    /// clause shapes at the root and refutes them with a counting DRAT proof
    /// over fresh definition variables. Strict all-or-nothing detection, so
    /// non-matching formulas stay byte-identical.
    pub(crate) php_refute: bool,
    pub(crate) els: bool,
    pub(crate) congruence: bool,
    pub(crate) congruence_xor: bool,
    pub(crate) congruence_iter: bool,
    pub(crate) inprocess_interval_conflicts: u64,
    pub(crate) inprocess_max_rounds: u64,
    pub(crate) vivify_ticks_budget: u64,
    pub(crate) vivify_permille: u64,
    pub(crate) vivify_max_clause_len: usize,
    pub(crate) probe_ticks_budget: u64,
    pub(crate) eliminate_ticks_budget: u64,
    pub(crate) eliminate_resolution_budget: u64,
    /// Mid-giant materialized-resolvent cap for the root BVE pass (SESSION 14).
    /// 0 = off. Set post-parse in main.rs for 5-20M-var instances (below the
    /// giant-light floor); not an env-parsed config key — the env override is
    /// SAT_GIANT_ELIM_RESOLVENTS, read at the main.rs scope decision.
    pub(crate) giant_elim_resolvent_budget: u64,
    pub(crate) eliminate_occurrence_limit: u64,
    pub(crate) transitive_max_depth: u32,
    pub(crate) transitive_ticks_per_source: u64,
    pub(crate) transitive_max_removed_per_round: u64,
    /// Total tick budget for the root transitive-reduction scan
    /// (SAT_TRANSITIVE_TICKS); 0 = proportional default
    /// (original_literals * 20 clamped to [10M, 100M]).
    pub(crate) transitive_ticks_budget: u64,
    /// Adoption threshold for the root transitive-reduction dry-run
    /// (SAT_TRANSITIVE_MIN_REMOVED_PERMILLE): the collected removals/units are
    /// applied only when removed * 1000 >= live_binaries * permille. Below the
    /// threshold nothing is touched, so the cell's trajectory stays
    /// byte-identical to SAT_TRANSITIVE=off.
    pub(crate) transitive_min_removed_permille: u64,
    /// Apply the failed-literal units found by the ROOT transitive dry-run
    /// even when the removable-binary count stays below
    /// `transitive_min_removed_permille` (SAT_TRANSITIVE_UNITS_ONLY). The
    /// units are RUP-sound independent of adoption; deletions stay withheld
    /// and the formula does NOT become a round adopter (`transitive_adopted`
    /// stays 0, so no inprocessing rounds fire). Root-only: inprocessing-round
    /// scans keep their own threshold semantics.
    pub(crate) transitive_units_only: bool,
    /// Re-run transitive reduction at every inprocessing round
    /// (SAT_TRANSITIVE_INPROCESS), but ONLY on formulas whose root pass
    /// adopted (crossed `transitive_min_removed_permille`). Formulas below the
    /// root threshold never scan mid-search, so their trajectories stay
    /// byte-identical to SAT_TRANSITIVE_INPROCESS=off.
    pub(crate) transitive_inprocess: bool,
    /// Adoption threshold for each inprocessing-round transitive dry-run
    /// (SAT_TRANSITIVE_INPROCESS_MIN_REMOVED_PERMILLE). Default 0 = kissat
    /// parity: apply everything the round finds (the formula's trajectory is
    /// already rerolled by the root adoption).
    pub(crate) transitive_inprocess_min_removed_permille: u64,
    /// Run standalone equivalent-literal substitution every inprocessing round
    /// (SAT_ELS_INPROCESS, kissat probe.c parity — kissat_substitute runs each
    /// probe round), but ONLY on formulas whose root transitive pass adopted
    /// (`transitive_adopted == 1`), the SESSION 7 root-adopter scope: every
    /// non-adopter keeps a byte-identical trajectory.
    pub(crate) els_inprocess: bool,
    /// Run root failed-literal probing every inprocessing round
    /// (SAT_PROBE_INPROCESS, kissat probe.c parity — binary_clauses_backbone
    /// runs each probe round), scoped to root-transitive adopters exactly like
    /// `els_inprocess`. Does not touch the root SAT_PROBE pass.
    pub(crate) probe_inprocess: bool,
    /// Extend the inprocessing-round transitive scan to SCOPED-GATE-BVE
    /// adopters (`gate_bve_scoped_adopted == 1`), the second root-pass adopter
    /// class (SAT_TRANSITIVE_INPROCESS_GBVE). Same safety shape as the
    /// root-transitive scope: reroll risk confined to cells that already
    /// rerolled at the gate-BVE promotion; non-adopters stay byte-identical.
    pub(crate) transitive_inprocess_gbve: bool,
    /// Extend the inprocessing-round failed-literal probe to scoped-gate-BVE
    /// adopters (SAT_PROBE_INPROCESS_GBVE), exactly like
    /// `transitive_inprocess_gbve`.
    pub(crate) probe_inprocess_gbve: bool,
    /// SAT_PROBE_INPROCESS_ARMED (default off, SESSION 14d): run the
    /// failed-literal probe round on inprocess_aggressive-armed formulas.
    pub(crate) probe_inprocess_armed: bool,
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
            // Binary DRAT default since the 2026-07-15 promotion
            // (log/abtest-bindrat-vs-base-2026-07-15-01-07-48, gate PASS/WIN:
            // solved 66==66 with identical both-solved conflicts, PAR-2
            // 146409.6 vs 146492.7; sted2_0x1e3-216 solved 1746s in the binary
            // arm only). Trajectory-neutral: proof bytes never feed back into
            // search; drat-trim auto-detects the format (64 verify=ok in-gate).
            // Off-switch: SAT_PROOF=drat restores ASCII DRAT.
            proof_policy: ProofPolicy::DratBinary,
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
            stable_target_reset: false,
            search_mode_policy: SearchModePolicy::Single,
            mode_use_ticks: false,
            lucky: false,
            warmup: false,
            bump_reasons: false,
            bump_reasons_limit_multiplier: 10,
            chrono_backtrack: false,
            binary_fast_path: false,
            prefetch_watched_clauses: false,
            clause_min_mode: ClauseMinMode::InBlockLate,
            inblock_delay_conflicts: 1_000_000,
            inblock_binary_min: 0.85,
            otfs: false,
            otss: false,
            reduce_tier2_at_budget: false,
            watch_compact_enabled: false,
            vmtf: VmtfMode::Off,
            rephase: false,
            // SESSION 14d: default OFF (rephase/walk on unarmed cells too) — part
            // of the A/B3 full-bench winning arm (280v276, zero losses).
            rephase_armed_only: false,
            walk: true,
            walk_effort_permille: DEFAULT_WALK_EFFORT_PERMILLE,
            walk_warmup: false,
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
            bsr_drain_batched: false,
            bsr_occurrence_limit: 0,
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
            transitive: true,
            forward_subsume: false,
            gate_extract: false,
            gate_bve: false,
            gate_bve_scoped: false,
            gate_bve_min_gain_pct: 2,
            gate_bve_scoped_max_vars: 100_000,
            rcheck: false,
            gauss: false,
            factor: false,
            pair_abs_refute: false,
            php_refute: false,
            // SESSION 14 (2026-07-30): default ON with the percent-scale apply
            // threshold (SAT_ELS_MIN_SUBST_PERMILLE, main.rs) — the root pass
            // declines byte-identically below 5% merge mass, so banked cells are
            // untouched while structural collapses (blockpuzzle/bv_ILA-class)
            // still fire. Unthresholded root ELS measured NET-NEGATIVE on the
            // 2026-07-31 full-bench A/B (rerolled oddball-tto_zp x4 etc.).
            els: true,
            congruence: false,
            congruence_xor: false,
            congruence_iter: false,
            inprocess_interval_conflicts: 0,
            inprocess_max_rounds: 0,
            vivify_ticks_budget: 0,
            vivify_permille: 0,
            vivify_max_clause_len: 0,
            probe_ticks_budget: 0,
            eliminate_ticks_budget: DEFAULT_ELIMINATE_TICKS_BUDGET,
            eliminate_resolution_budget: DEFAULT_ELIMINATE_RESOLUTION_BUDGET,
            giant_elim_resolvent_budget: 0,
            eliminate_occurrence_limit: 2000,
            transitive_max_depth: 0,
            transitive_ticks_per_source: 0,
            transitive_max_removed_per_round: 0,
            transitive_ticks_budget: 0,
            transitive_min_removed_permille: 100,
            transitive_units_only: false,
            transitive_inprocess: true,
            transitive_inprocess_min_removed_permille: 0,
            els_inprocess: false,
            transitive_inprocess_gbve: false,
            probe_inprocess_gbve: false,
            probe_inprocess_armed: false,
            // Default-on since the 2026-07-27 promotion (gate WIN 72v72
            // identical solved sets, both-solved conflicts -121,608:
            // log/abtest-cand-vs-base-2026-07-27-11-58-13). Root-adopter
            // scope: only sted2/ibm trajectories move; everything else is
            // byte-identical (rbsat digit-exact at 100k conflicts).
            probe_inprocess: true,
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
                // profile20 Stage-1 ablation (2026-05-30/31): the "fstab_lbdtier" config —
                // focused-stable search + LBD + tick mode-switching + LBD-tiered reduction
                // (VMTF auto-resolves to FocusedOnly) — wins aggregate PAR-2 by 2x over the
                // prior single-mode default (5653 vs 6808 on profile20, 13/20 vs 10/20 solved;
                // clear of the solver-10 floor 6773). The win is SAT_REDUCE=lbd-tiered
                // (cracks 3 hard headroom instances), amplified by focused-stable+VMTF halving
                // the easy-half overhead. See log/feature-ablation-2026-05-30-12-11-01/FINDINGS.md.
                self.use_lbd = true;
                self.search_mode_policy = SearchModePolicy::FocusedStable;
                self.mode_use_ticks = true;
                self.reduce_policy = ReducePolicy::LbdTiered;
                // SAT-playground-5b2.2.67 (2026-06-17): glue recompute + tier
                // promotion on reason use (kissat deduce.c mark_clause_as_used).
                // Folding this into the lbd-tiered default feeds the reducer a
                // kissat-faithful distribution instead of frozen birth LBDs.
                // profile20 5x5/900s: +4 solved (71->75, oddball 1/5->4/5),
                // -17.0M conflicts, -5844 PAR-2 vs the prior default.
                self.update_reason_lbd = true;
                self.update_propagation_reason_lbd = true;
                // SAT-playground-5b2.2.64 (2026-06-13): profile20 5x5/900s re-eval promoted
                // target-then-saved to default/fast focused phases (+1 solved, -11.1M conflicts
                // vs 156ade2 current baseline). Keep the raw config/baseline profile on legacy
                // phase so single-mode and explicit legacy fixtures remain valid.
                self.phase_policy = PhasePolicy::TargetThenSaved;
                // SAT-playground-8id (2026-07-04): kissat-parity anti-diversification fix
                // for the FocusedStable default. Stable-mode decisions previously consulted
                // best_phase FIRST (BestThenTargetThenSaved), and neither best_assigned nor
                // target_assigned is ever reset on the default path (rephase is off), so an
                // early-captured all-time-best prefix was replayed by every stable decision
                // forever. Demote stable to TargetThenSaved AND reset the target snapshot on
                // each switch into stable so every stable phase recaptures a fresh prefix.
                // The two are only effective together: demote alone is byte-identical to the
                // prior default (best-first masks target); the reset only bites once best is
                // out of the consult path. sat-comp-2025-medium single-seed A/B (32c/16GB/1800s,
                // log/abtest-demote_reset-vs-demote-vs-base-2026-07-04-02-51-41): 53/100 vs 48/100
                // (+5 solved; 7 cracks incl. UNSAT 0f269188, all drat/model verified; 2 losses),
                // demote-alone 48/100 with identical conflicts.
                self.stable_phase_policy = Some(PhasePolicy::TargetThenSaved);
                self.stable_target_reset = true;
                // SAT-playground-5b2.6.6 (2026-06-13): batching BSR queue drains until
                // the outer preprocessing loop improves profile20 5x5/900s from 67/100
                // to 70/100 solved by preserving more BVE opportunity on sqrt170/bp4.
                self.bsr_drain_batched = true;
                // SAT-playground-5b2.3.45 (2026-06-13): kissat-style BSR occurrence
                // cap (`SAT_BSR_OCCLIM=1000`) keeps the same 70/100 solved profile20
                // 5x5/900s count while cutting solved-cell conflicts by 4.36M and
                // PAR-2 by 2284s vs the batched-drain default.
                self.bsr_occurrence_limit = 1000;
                // SAT-playground-5b2.3.28 (2026-07-10): skip full BSR on the
                // existing large/sparse/dense formula gate. sat-comp-2025-medium
                // single-seed A/B (32c/16GB/1800s, log/abtest-gate-vs-base-
                // 2026-07-10-18-54-18): solved ties 61/100; conflicts improve
                // 57,183,554 -> 57,134,586 with no contradictions.
                self.bsr_formula_gate = true;
                // 70h: SAT_LUCKY promoted to the default/fast profiles (2026-05-30 re-eval):
                // n>=5 aggregate -12 PAR-2 vs lucky-off, and lucky robustly solves the
                // order-fragile battleship instance (0.08s vs lucky-off 18-904s/timeouts).
                self.lucky = true;
                self.initial_clause_mode = InitialClauseMode::CanonicalSorted;
                // SAT-playground-5b2.8.1 (2026-06-17): software-prefetch the next watched
                // clause in propagation (perf: propagate is 83% of self-time, bottlenecked on
                // random arena[clause_idx] cache misses). Conflict-preserving (a pure cache
                // hint): seedgate solved 75=75 with byte-identical conflicts on all 75
                // shared-solved cells. Single-instance (quiet cores, the competition scenario)
                // it is faster-or-neutral on all 20 profile20 instances (-0.1%..-20.9%, e.g.
                // case9 -14%, SCPC -21%, Kakuro -10%). The 5x5 seedgate's PAR-2 was only -0.14%
                // because 5 parallel cells saturate memory bandwidth and the extra prefetch
                // traffic contends; that is a measurement artifact of the parallel sweep, not
                // the single-instance objective.
                self.prefetch_watched_clauses = true;
                // SAT-playground-qld (2026-06-30): XOR/parity Gaussian refutation promoted
                // to default/fast. profile20 5x5/900s: +5 solved (76->81) — tseitin_grid_n12
                // 0/5->5/5, which CDCL cannot refute (exponential resolution) but Gaussian
                // elimination over GF(2) cracks at root in ~5s. Trajectory-neutral: the 76
                // prior-solved cells keep byte-identical conflicts (the engine is
                // coverage-gated, firing only on parity-structured formulas; extraction tax
                // <=0.74s on the largest cell, which times out regardless). The DRAT proof is
                // pure resolution and drat-trim VERIFIED (1.26M-clause proof, 0 RAT lemmas).
                self.gauss = true;
                // SAT-playground-5b2.3.46 (2026-07-11): bounded variable addition
                // (kissat factor.c port) promoted to default/fast. Frontend pass on the
                // parsed formula (vars <= 10^4, clause size <= 5, reduction bound 16,
                // 700M-tick productivity-extended budget) that factors shared subclauses
                // into fresh variables with DRAT RAT definitions. sat-comp-2025-medium
                // single-seed A/B (32c/16GB/1800s,
                // log/abtest-cand-vs-base-2026-07-11-17-39-48): 63/100 vs 61/100
                // (+REGRandom-K4 UNSAT 28s ex-timeout, +MVRoundRobin_n16_d10_v2 UNSAT
                // 183s ex-timeout), PAR-2 153448.7 vs 158211.3, zero contradictions or
                // correctness failures; promotion_gate PASS. REGRandom's factored DRAT
                // proof (128,640 RAT lemmas) drat-trim VERIFIED standalone.
                self.factor = true;
                // 2026-07-12: gate congruence closure (with XOR extraction) promoted to
                // default/fast, now running BOTH at the root and inside every guarded
                // inprocessing round (kissat probe.c parity: congruence -> substitute
                // before vivify/sweep). try_congruence first DRY-RUNS gate extraction +
                // merge matching on the untouched formula and applies NOTHING — no hidden
                // binaries, no ELS substitution, no merges — below
                // CONGRUENCE_MIN_APPLY_MERGES (3000) or above 10M live clauses, so
                // non-gate-circuit formulas keep byte-identical trajectories. Formulas
                // whose ROOT closure clears the productivity bar additionally switch the
                // inprocessing scheduler to the early doubling cadence (first round at
                // 10k conflicts) and run mid-search BVE rounds between search phases —
                // the kissat mechanism on miter/BMC circuits (VexRiscv: 183k congruent
                // matches + 13 eliminations; oski: 74% vars eliminated over 20 rounds).
                // sat-comp-2025-medium single-seed A/B (32c/16GB/1800s,
                // log/abtest-cand-vs-base-2026-07-12-00-03-56): solved 62/100 == 62/100
                // with IDENTICAL solved sets, both-solved conflicts 53,552,717 vs
                // 54,651,852 (-2.0%, at-least-two-ibm-2004 389,682 vs 1,488,817), zero
                // contradictions or correctness failures; promotion_gate PASS. The
                // unthresholded variant (log/abtest-cand-vs-base-2026-07-11-21-59-04)
                // lost 4 SAT cells whose formulas were rewritten for sub-threshold merge
                // counts (oddball_80 39, bp4_CSO_IXA 648, bp5_CSO 1892, plus 34k
                // zero-merge hidden binaries on Timetables and pure-binary ELS rewrites
                // on Kakuro) — the dry-run threshold exists to keep exactly those cells
                // byte-identical. Mid-search congruence+BVE DRAT proofs drat-trim
                // VERIFIED standalone (div-mitern172, 292MB proof, s VERIFIED).
                self.congruence = true;
                self.congruence_xor = true;
                // 2026-07-25: scoped gate-aware BVE promoted to default/fast. At the
                // root, plain BVE (E0) and Plaisted-Greenbaum gate-aware BVE (E1) are
                // dry-run on throwaway sub-solvers built from the live root-simplified
                // clauses (tick-budgeted, no proof, fully deterministic); gate-aware
                // BVE is enabled for the real run only when the net elimination gain
                // (E1/E0 - 1) reaches SAT_GATE_BVE_MIN_GAIN_PCT (2%), and only for
                // formulas <= SAT_GATE_BVE_SCOPED_MAX_VARS (100k) — the cap keeps the
                // big gate-3 reroll casualties (TT496 260k, bp5_CSO 380k, VexRiscv
                // 723k vars) byte-identical at zero dry-run cost. The threshold
                // filters the degenerate churn case (bp5_CSO: 56,646 gate
                // eliminations, 0% net gain — measured plan/kissat-gaps.md 2.6c).
                // sat-comp-2025-medium single-seed A/B (32c/16GB/1800s,
                // log/abtest-cand-vs-base-2026-07-25-13-59-16): solved 72/100 vs
                // 71/100 (+RoundRobin_n16_d13 UNSAT 119s FIRST-EVER — kissat cannot
                // even at 3600s; +bp4_TCO_CSO_IXA_LP_ZR SAT 237s kissat-only cell;
                // -bp4_BC012_CSO_FPBEQ, the pre-judged single casualty), both-solved
                // conflicts -1,088,186 on 70 cells (58 trajectory-identical), wall
                // -481s, PAR-2 126512.9 vs 130449.2; promotion_gate PASS, zero
                // contradictions or correctness failures. RoundRobin proof drat-trim
                // VERIFIED standalone (90MB, s VERIFIED).
                self.gate_bve_scoped = true;
                // 2026-07-09: adjacent-pair parity abstraction refuter promoted to
                // default/fast. It detects complete pair-XOR expansions such as the
                // sat-comp-2025-medium xor_op family, introduces fresh parity variables,
                // lifts the compact abstract clauses by resolution from the concrete
                // expansion, then maps a compact abstract UNSAT proof back into the
                // outer DRAT proof. Medium single-seed A/B (32c/16GB/1800s,
                // log/abtest-pairabs-vs-base-2026-07-09-08-20-53): 58/100 vs 55/100,
                // same both-solved conflicts, PAR-2 170286.2 vs 180659.0,
                // promotion_gate PASS. This recovers xor_op_n36/n38/n40.
                self.pair_abs_refute = true;
                // 2026-07-28: pigeonhole-counting extended-resolution refutation
                // promoted to default/fast. Detects the relativized-PHP (rphp) and
                // clique-coloring (clqcl) clause shapes by strict structural matching
                // (shuffle/sign-flip invariant, every required clause verified by
                // exact lookup) and refutes P pigeons -> N places -> H<P holes with a
                // counting DRAT proof over fresh W/G definition variables. Medium
                // single-seed A/B (32c/16GB/1800s,
                // log/abtest-cand-vs-base-2026-07-28-08-08-20): 74/100 vs 71/100
                // (+rphp5_050 +rphp5_085 +clqcl_40_6_5 +clqcl_50_6_5, all FIRST-EVER,
                // all UNSAT <0.3s with drat-trim-verified proofs, kissat cannot solve
                // any at 3600s; -oski15a01b20s, the documented wall-coin flipper at
                // its exact reference conflict count 2,663,684), 70 both-solved cells
                // ALL conflict-identical, PAR-2 115260.5 vs 128218.6;
                // promotion_gate PASS, zero contradictions/correctness failures.
                self.php_refute = true;
                // SAT-playground-5b2.2.76-adjacent (2026-07-05): promote kissat-parity
                // reason-side literal bumping (analyze.c bump_reason mark set) to the
                // default profile. Stock kissat bumps not only the 1UIP-analyzed
                // variables but the literals on the reason side of the learned clause;
                // solver 12 previously bumped only the analyzed set (SAT_BUMP_REASONS
                // off), a real VSIDS-quality gap. sat-comp-2025-medium single-seed A/B
                // (32c/16GB/1800s, log/abtest-bumpreason-vs-chrono-vs-rephase-vs-base-
                // 2026-07-05-12-35-41): 53/100 solved (tie vs base) but conflicts on the
                // both-solved cells 36,570,896 vs 61,594,138 (-41%), PAR-2 185121 vs
                // 192220; check_promotion_gate PASS, 0 contradictions, 0 correctness
                // failures. The +4/-4 solved shuffle nets a tie; the conflict reduction
                // is the promotable win (lexicographic level 2). Multiplier stays at the
                // kissat-default 10x initial-bumped-set cap.
                self.bump_reasons = true;
                // SAT-playground (2026-07-05): promote chronological backtracking
                // (kissat/cadical parity — both default it on) to default/fast, ON TOP
                // of SAT_BUMP_REASONS. Chrono backtracks only to the conflict's second-
                // highest level instead of always to the asserting level, preserving
                // out-of-order propagations. sat-comp-2025-medium single-seed A/B on the
                // bump_reasons baseline (32c/16GB/1800s, log/abtest-chrono-vs-base-
                // 2026-07-05-17-53-24): 55/100 vs 53 (+2 solved), PAR-2 179710 vs 185028;
                // check_promotion_gate PASS, 0 contradictions, 0 correctness failures.
                // The +2 exactly recovers the two SAT cells (mp1-Nb7T46, 59-129706) that
                // bump_reasons' VSIDS shift had shuffled out — zero regressions, a
                // mechanism-plausible interaction, not a boundary flip. chrono_max_delta
                // keeps the kissat reassign-delta cap.
                self.chrono_backtrack = true;
                // SAT-playground (2026-07-09): promote guarded SAT sweeping to the
                // default/fast profiles at a conservative 1M-conflict cadence. The
                // sweep pass is a Kissat-style inprocessing capability gap for miters,
                // but the raw 1M arm lost a long SAT trajectory. The deep-phase guard
                // in main.rs skips sweep when both best and target phase prefixes are
                // already near-complete, preserving the fragile SAT cell while keeping
                // the miter/circuit conflict wins. sat-comp-2025-medium single-seed A/B
                // (32c/16GB/1800s, log/abtest-sweepguard1m-vs-base-2026-07-08-22-35-36):
                // 55/100 solved tie, conflicts on both-solved cells 48,310,770 vs
                // 49,165,593, PAR-2 180415.3 vs 180795.7; promotion_gate PASS with
                // zero correctness failures or SAT/UNSAT contradictions.
                self.inprocess = true;
                self.inprocess_interval_conflicts = 1_000_000;
                // 2026-07-10: enable learned-clause vivification behind the
                // delayed scheduler in main.rs. Sweep keeps the 1M cadence; vivify
                // only starts on very long searches where the focused screen found
                // a Kissat-gap recovery without disturbing sub-6M-conflict SAT cells.
                self.vivify = true;
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
        self.stable_target_reset = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_STABLE_TARGET_RESET",
            self.stable_target_reset,
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
        self.prefetch_watched_clauses = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_PREFETCH",
            self.prefetch_watched_clauses,
        );
        self.clause_min_mode = parse_enum_selected(
            env_map,
            &key_set,
            "SAT_CLAUSE_MIN",
            self.clause_min_mode,
            ClauseMinMode::parse,
        );
        self.inblock_delay_conflicts = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_INBLOCK_DELAY_CONFLICTS",
            self.inblock_delay_conflicts,
        );
        self.inblock_binary_min = parse_f64_selected(
            env_map,
            &key_set,
            "SAT_INBLOCK_BINARY_MIN",
            self.inblock_binary_min,
        );
        self.otfs = parse_bool_selected(env_map, &key_set, "SAT_OTFS", self.otfs);
        self.otss = parse_bool_selected(env_map, &key_set, "SAT_OTSS", self.otss);
        self.reduce_tier2_at_budget = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_REDUCE_TIER2_AT_BUDGET",
            self.reduce_tier2_at_budget,
        );
        self.watch_compact_enabled = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_WATCH_COMPACT",
            self.watch_compact_enabled,
        );
        let vmtf_explicit = get_selected(env_map, &key_set, "SAT_VMTF").is_some();
        self.vmtf = parse_enum_selected(env_map, &key_set, "SAT_VMTF", self.vmtf, VmtfMode::parse);
        self.rephase = parse_bool_selected(env_map, &key_set, "SAT_REPHASE", self.rephase);
        self.rephase_armed_only = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_REPHASE_ARMED_ONLY",
            self.rephase_armed_only,
        );
        self.walk = parse_bool_selected(env_map, &key_set, "SAT_WALK", self.walk);
        self.walk_effort_permille = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_WALK_EFFORT",
            self.walk_effort_permille,
        );
        self.walk_warmup =
            parse_bool_selected(env_map, &key_set, "SAT_WALK_WARMUP", self.walk_warmup);
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
        self.bsr_drain_batched = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_BSR_DRAIN_BATCHED",
            self.bsr_drain_batched,
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
        self.bsr_occurrence_limit = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_BSR_OCCLIM",
            self.bsr_occurrence_limit,
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
        self.gate_bve_scoped = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_GATE_BVE_SCOPED",
            self.gate_bve_scoped,
        );
        self.gate_bve_min_gain_pct = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_GATE_BVE_MIN_GAIN_PCT",
            self.gate_bve_min_gain_pct,
        );
        self.gate_bve_scoped_max_vars = parse_usize_selected(
            env_map,
            &key_set,
            "SAT_GATE_BVE_SCOPED_MAX_VARS",
            self.gate_bve_scoped_max_vars,
        );
        // Explicit global gate-BVE supersedes the scoped per-formula decision:
        // with scoped on by default, SAT_GATE_BVE=on must stay usable as the
        // unconditional variant (A/B arms, reproductions) without a config error.
        if self.gate_bve_scoped && self.gate_bve {
            self.gate_bve_scoped = false;
        }
        self.rcheck = parse_bool_selected(env_map, &key_set, "SAT_RCHECK", self.rcheck);
        self.gauss = parse_bool_selected(env_map, &key_set, "SAT_GAUSS", self.gauss);
        self.factor = parse_bool_selected(env_map, &key_set, "SAT_FACTOR", self.factor);
        self.pair_abs_refute = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_PAIR_ABS_REFUTE",
            self.pair_abs_refute,
        );
        self.php_refute =
            parse_bool_selected(env_map, &key_set, "SAT_PHP_REFUTE", self.php_refute);
        self.els = parse_bool_selected(env_map, &key_set, "SAT_ELS", self.els);
        self.congruence =
            parse_bool_selected(env_map, &key_set, "SAT_CONGRUENCE", self.congruence);
        self.congruence_xor = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_CONGRUENCE_XOR",
            self.congruence_xor,
        );
        self.congruence_iter = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_CONGRUENCE_ITER",
            self.congruence_iter,
        );
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
        self.vivify_permille = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_VIVIFY_PERMILLE",
            self.vivify_permille,
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
        self.eliminate_occurrence_limit = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_ELIMINATE_OCCLIM",
            self.eliminate_occurrence_limit,
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
        self.transitive_ticks_budget = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_TRANSITIVE_TICKS",
            self.transitive_ticks_budget,
        );
        self.transitive_min_removed_permille = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_TRANSITIVE_MIN_REMOVED_PERMILLE",
            self.transitive_min_removed_permille,
        );
        self.transitive_units_only = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_TRANSITIVE_UNITS_ONLY",
            self.transitive_units_only,
        );
        self.transitive_inprocess = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_TRANSITIVE_INPROCESS",
            self.transitive_inprocess,
        );
        self.transitive_inprocess_min_removed_permille = parse_u64_selected(
            env_map,
            &key_set,
            "SAT_TRANSITIVE_INPROCESS_MIN_REMOVED_PERMILLE",
            self.transitive_inprocess_min_removed_permille,
        );
        self.els_inprocess = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_ELS_INPROCESS",
            self.els_inprocess,
        );
        self.probe_inprocess = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_PROBE_INPROCESS",
            self.probe_inprocess,
        );
        self.transitive_inprocess_gbve = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_TRANSITIVE_INPROCESS_GBVE",
            self.transitive_inprocess_gbve,
        );
        self.probe_inprocess_gbve = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_PROBE_INPROCESS_GBVE",
            self.probe_inprocess_gbve,
        );
        self.probe_inprocess_armed = parse_bool_selected(
            env_map,
            &key_set,
            "SAT_PROBE_INPROCESS_ARMED",
            self.probe_inprocess_armed,
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

    pub(crate) fn refresh_feature_statuses(&mut self) {
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
            fail_config("Invalid config: SAT_OTFS=on requires SAT_CLAUSE_MIN=basic|recursive-limited|inblock|inblock-late");
        }
        if self.otss && self.clause_min_mode == ClauseMinMode::Off {
            fail_config("Invalid config: SAT_OTSS=on requires SAT_CLAUSE_MIN=basic|recursive-limited|inblock|inblock-late");
        }
        if !(0.0..=1.0).contains(&self.inblock_binary_min) {
            fail_config("Invalid config: SAT_INBLOCK_BINARY_MIN must be in [0,1]");
        }
        if self.hbr && !self.probe {
            fail_config("Invalid config: SAT_HBR=on requires SAT_PROBE=on");
        }
        if self.vivify && !self.inprocess {
            fail_config("Invalid config: SAT_VIVIFY=on requires SAT_INPROCESS=on (vivify runs in the inprocessing round)");
        }
        if self.gate_bve && !self.gate_extract {
            fail_config("Invalid config: SAT_GATE_BVE=on requires SAT_GATE_EXTRACT=on");
        }
        let unsupported = [
            (self.hbr, "SAT_HBR"),
            (self.forward_subsume, "SAT_FORWARD_SUBSUME"),
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
        push_kv_bool(&mut lines, "stable_target_reset", self.stable_target_reset);
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
        push_kv_bool(
            &mut lines,
            "prefetch_watched_clauses",
            self.prefetch_watched_clauses,
        );
        push_kv(&mut lines, "clause_min_mode", self.clause_min_mode.as_str());
        push_kv(
            &mut lines,
            "inblock_delay_conflicts",
            self.inblock_delay_conflicts.to_string(),
        );
        push_kv(
            &mut lines,
            "inblock_binary_min",
            self.inblock_binary_min.to_string(),
        );
        push_kv_bool(&mut lines, "otfs", self.otfs);
        push_kv_bool(&mut lines, "otss", self.otss);
        push_kv_bool(&mut lines, "reduce_tier2_at_budget", self.reduce_tier2_at_budget);
        push_kv_bool(&mut lines, "watch_compact_enabled", self.watch_compact_enabled);
        push_kv(&mut lines, "vmtf", self.vmtf.as_str());
        push_kv_bool(&mut lines, "rephase", self.rephase);
        push_kv_bool(&mut lines, "rephase_armed_only", self.rephase_armed_only);
        push_kv_bool(&mut lines, "walk", self.walk);
        push_kv(
            &mut lines,
            "walk_effort_permille",
            self.walk_effort_permille.to_string(),
        );
        push_kv_bool(&mut lines, "walk_warmup", self.walk_warmup);
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
        push_kv_bool(&mut lines, "bsr_drain_batched", self.bsr_drain_batched);
        push_kv(
            &mut lines,
            "bsr_occurrence_limit",
            self.bsr_occurrence_limit.to_string(),
        );
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
        push_kv_bool(&mut lines, "gate_bve_scoped", self.gate_bve_scoped);
        push_kv(
            &mut lines,
            "gate_bve_min_gain_pct",
            self.gate_bve_min_gain_pct.to_string(),
        );
        push_kv(
            &mut lines,
            "gate_bve_scoped_max_vars",
            self.gate_bve_scoped_max_vars.to_string(),
        );
        push_kv_bool(&mut lines, "rcheck", self.rcheck);
        push_kv_bool(&mut lines, "gauss", self.gauss);
        push_kv_bool(&mut lines, "factor", self.factor);
        push_kv_bool(&mut lines, "pair_abs_refute", self.pair_abs_refute);
        push_kv_bool(&mut lines, "php_refute", self.php_refute);
        push_kv_bool(&mut lines, "els", self.els);
        push_kv_bool(&mut lines, "congruence", self.congruence);
        push_kv_bool(&mut lines, "congruence_xor", self.congruence_xor);
        push_kv_bool(&mut lines, "congruence_iter", self.congruence_iter);
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
            "vivify_permille",
            self.vivify_permille.to_string(),
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
            "eliminate_occurrence_limit",
            self.eliminate_occurrence_limit.to_string(),
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
            "transitive_ticks_budget",
            self.transitive_ticks_budget.to_string(),
        );
        push_kv(
            &mut lines,
            "transitive_min_removed_permille",
            self.transitive_min_removed_permille.to_string(),
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
            "SAT_PREFETCH",
            config.prefetch_watched_clauses,
            FeatureMaturity::Experimental,
            false,
            false,
            false,
            "",
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
            "SAT_REDUCE_TIER2_AT_BUDGET",
            config.reduce_tier2_at_budget,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "bead/SAT-playground-5b2.2.44",
        ),
        feature(
            "SAT_WATCH_COMPACT",
            config.watch_compact_enabled,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "bead/SAT-playground-s11-1-14b",
        ),
        feature(
            "SAT_SIMPLIFICATION",
            config.simplification,
            FeatureMaturity::SmokeSafe,
            true,
            true,
            false,
            "solver/12-kissat-inprocessing/BASELINE_LOCK.raw.txt",
        ),
        feature(
            "SAT_BVE",
            config.bve,
            FeatureMaturity::SmokeSafe,
            true,
            true,
            false,
            "solver/12-kissat-inprocessing/BASELINE_LOCK.raw.txt",
        ),
        feature(
            "SAT_FULL_BSR",
            config.full_bsr,
            FeatureMaturity::SmokeSafe,
            true,
            true,
            false,
            "solver/12-kissat-inprocessing/BASELINE_LOCK.raw.txt",
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
            "SAT_BSR_DRAIN_BATCHED",
            config.bsr_drain_batched,
            FeatureMaturity::DiscriminatingValidated,
            false,
            false,
            false,
            "",
        ),
        feature(
            "SAT_INPROCESS",
            config.inprocess,
            FeatureMaturity::DiscriminatingValidated,
            true,
            true,
            true,
            "log/abtest-sweepguard1m-vs-base-2026-07-08-22-35-36",
        ),
        feature(
            "SAT_VIVIFY",
            config.vivify,
            FeatureMaturity::FullSetValidated,
            true,
            true,
            true,
            "log/abtest-cand-vs-base-2026-07-11-08-54-35",
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
            FeatureMaturity::Experimental,
            false,
            false,
            false,
            "root binary-implication transitive reduction (kissat transitive.c)",
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
            FeatureMaturity::Experimental,
            false,
            false,
            false,
            "",
        ),
        feature(
            "SAT_GATE_BVE",
            config.gate_bve,
            FeatureMaturity::Experimental,
            false,
            false,
            false,
            "",
        ),
        feature(
            "SAT_GATE_BVE_SCOPED",
            config.gate_bve_scoped,
            FeatureMaturity::FullSetValidated,
            true,
            true,
            true,
            "log/abtest-cand-vs-base-2026-07-25-13-59-16",
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
        feature(
            "SAT_GAUSS",
            config.gauss,
            FeatureMaturity::SmokeSafe,
            true,
            false,
            false,
            "log/seedgate-s12_gauss-2026-06-30-08-18-10",
        ),
        feature(
            "SAT_PAIR_ABS_REFUTE",
            config.pair_abs_refute,
            FeatureMaturity::FullSetValidated,
            true,
            false,
            true,
            "log/abtest-pairabs-vs-base-2026-07-09-08-20-53",
        ),
        feature(
            "SAT_PHP_REFUTE",
            config.php_refute,
            FeatureMaturity::FullSetValidated,
            true,
            false,
            true,
            "log/abtest-cand-vs-base-2026-07-28-08-08-20",
        ),
        feature(
            "SAT_ELS",
            config.els,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "bd:SAT-playground-otd",
        ),
        feature(
            "SAT_CONGRUENCE",
            config.congruence,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "bd:SAT-playground-otd",
        ),
        feature(
            "SAT_CONGRUENCE_XOR",
            config.congruence_xor,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "bd:SAT-playground-otd",
        ),
        feature(
            "SAT_CONGRUENCE_ITER",
            config.congruence_iter,
            FeatureMaturity::Experimental,
            true,
            true,
            false,
            "bd:SAT-playground-otd",
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
        "stable_target_reset" => Some("SAT_STABLE_TARGET_RESET"),
        "search_mode_policy" => Some("SAT_SEARCH_MODE"),
        "mode_use_ticks" => Some("SAT_MODE_USE_TICKS"),
        "lucky" => Some("SAT_LUCKY"),
        "warmup" => Some("SAT_WARMUP"),
        "bump_reasons" => Some("SAT_BUMP_REASONS"),
        "bump_reasons_limit_multiplier" => Some("SAT_BUMP_REASONS_LIMIT"),
        "chrono_backtrack" => Some("SAT_CHRONO"),
        "binary_fast_path" => Some("SAT_BINARY_FAST"),
        "prefetch_watched_clauses" => Some("SAT_PREFETCH"),
        "clause_min_mode" => Some("SAT_CLAUSE_MIN"),
        "inblock_delay_conflicts" => Some("SAT_INBLOCK_DELAY_CONFLICTS"),
        "inblock_binary_min" => Some("SAT_INBLOCK_BINARY_MIN"),
        "otfs" => Some("SAT_OTFS"),
        "otss" => Some("SAT_OTSS"),
        "reduce_tier2_at_budget" => Some("SAT_REDUCE_TIER2_AT_BUDGET"),
        "watch_compact_enabled" => Some("SAT_WATCH_COMPACT"),
        "vmtf" => Some("SAT_VMTF"),
        "rephase" => Some("SAT_REPHASE"),
        "rephase_armed_only" => Some("SAT_REPHASE_ARMED_ONLY"),
        "walk" => Some("SAT_WALK"),
        "walk_effort_permille" => Some("SAT_WALK_EFFORT"),
        "walk_warmup" => Some("SAT_WALK_WARMUP"),
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
        "bsr_drain_batched" => Some("SAT_BSR_DRAIN_BATCHED"),
        "bsr_occurrence_limit" => Some("SAT_BSR_OCCLIM"),
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
        "gate_bve_scoped" => Some("SAT_GATE_BVE_SCOPED"),
        "gate_bve_min_gain_pct" => Some("SAT_GATE_BVE_MIN_GAIN_PCT"),
        "gate_bve_scoped_max_vars" => Some("SAT_GATE_BVE_SCOPED_MAX_VARS"),
        "rcheck" => Some("SAT_RCHECK"),
        "gauss" => Some("SAT_GAUSS"),
        "factor" => Some("SAT_FACTOR"),
        "pair_abs_refute" => Some("SAT_PAIR_ABS_REFUTE"),
        "php_refute" => Some("SAT_PHP_REFUTE"),
        "els" => Some("SAT_ELS"),
        "congruence" => Some("SAT_CONGRUENCE"),
        "inprocess_interval_conflicts" => Some("SAT_INPROCESS_INTERVAL_CONFLICTS"),
        "inprocess_max_rounds" => Some("SAT_INPROCESS_MAX_ROUNDS"),
        "vivify_ticks_budget" => Some("SAT_VIVIFY_TICKS"),
        "vivify_permille" => Some("SAT_VIVIFY_PERMILLE"),
        "vivify_max_clause_len" => Some("SAT_VIVIFY_MAX_CLAUSE_LEN"),
        "probe_ticks_budget" => Some("SAT_PROBE_TICKS"),
        "eliminate_ticks_budget" => Some("SAT_ELIMINATE_TICKS"),
        "eliminate_resolution_budget" => Some("SAT_ELIMINATE_RESOLUTIONS"),
        "eliminate_occurrence_limit" => Some("SAT_ELIMINATE_OCCLIM"),
        "transitive_max_depth" => Some("SAT_TRANSITIVE_MAX_DEPTH"),
        "transitive_ticks_per_source" => Some("SAT_TRANSITIVE_TICKS_PER_SOURCE"),
        "transitive_max_removed_per_round" => Some("SAT_TRANSITIVE_MAX_REMOVED_PER_ROUND"),
        "transitive_ticks_budget" => Some("SAT_TRANSITIVE_TICKS"),
        "transitive_min_removed_permille" => Some("SAT_TRANSITIVE_MIN_REMOVED_PERMILLE"),
        "transitive_units_only" => Some("SAT_TRANSITIVE_UNITS_ONLY"),
        "transitive_inprocess" => Some("SAT_TRANSITIVE_INPROCESS"),
        "transitive_inprocess_min_removed_permille" => {
            Some("SAT_TRANSITIVE_INPROCESS_MIN_REMOVED_PERMILLE")
        }
        "els_inprocess" => Some("SAT_ELS_INPROCESS"),
        "probe_inprocess" => Some("SAT_PROBE_INPROCESS"),
        "transitive_inprocess_gbve" => Some("SAT_TRANSITIVE_INPROCESS_GBVE"),
        "probe_inprocess_gbve" => Some("SAT_PROBE_INPROCESS_GBVE"),
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
        ("SAT_BVE", "SAT_GATE_BVE_SCOPED"),
        ("SAT_SIMPLIFICATION", "SAT_INPROCESS"),
        ("SAT_SIMPLIFICATION", "SAT_VIVIFY"),
        ("SAT_SIMPLIFICATION", "SAT_PROBE"),
        ("SAT_SIMPLIFICATION", "SAT_HBR"),
        ("SAT_SIMPLIFICATION", "SAT_TRANSITIVE"),
        ("SAT_SIMPLIFICATION", "SAT_FORWARD_SUBSUME"),
        ("SAT_SIMPLIFICATION", "SAT_GATE_EXTRACT"),
        ("SAT_SIMPLIFICATION", "SAT_GATE_BVE"),
        ("SAT_SIMPLIFICATION", "SAT_GATE_BVE_SCOPED"),
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
        "SAT_RESTART_DIVE",
        "SAT_DIVE_REUSE_TRAIL",
        "SAT_RESTART_DIVE2",
        "SAT_DEBUG_DIVE",
        "SAT_RESTART_DIVE_COLLAPSE",
        "SAT_RESTART_DIVE_BINFRAC",
        "SAT_RESTART_FLOOR",
        "SAT_RESTART_MARGIN",
        "SAT_EMA_SLOW_WINDOW",
        "SAT_RESTART_REUSE_TRAIL",
        "SAT_RESTART_REUSE_TRAIL_FOCUSED",
        "SAT_RESTART_REUSE_TRAIL_STABLE",
        "SAT_REDUCE",
        "SAT_PHASE",
        "SAT_FOCUSED_PHASE",
        "SAT_STABLE_PHASE",
        "SAT_STABLE_TARGET_RESET",
        "SAT_SEARCH_MODE",
        "SAT_MODE_USE_TICKS",
        "SAT_LUCKY",
        "SAT_WARMUP",
        "SAT_BUMP_REASONS",
        "SAT_BUMP_REASONS_LIMIT",
        "SAT_CHRONO",
        "SAT_BINARY_FAST",
        "SAT_PREFETCH",
        "SAT_CLAUSE_MIN",
        "SAT_OTFS",
        "SAT_OTSS",
        "SAT_REDUCE_TIER2_AT_BUDGET",
        "SAT_WATCH_COMPACT",
        "SAT_VMTF",
        "SAT_REPHASE",
        "SAT_REPHASE_ARMED_ONLY",
        "SAT_WALK",
        "SAT_WALK_EFFORT",
        "SAT_WALK_WARMUP",
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
        "SAT_BSR_DRAIN_BATCHED",
        "SAT_BSR_OCCLIM",
        "SAT_INPROCESS",
        "SAT_VIVIFY",
        "SAT_PROBE",
        "SAT_HBR",
        "SAT_VIVIFY_DEDUCE_ARMED_MIN",
        "SAT_VIVIFY_SORT_ARMED_MIN",
        "SAT_VIVIFY_TIER_SPLIT_ARMED_MIN",
        "SAT_RESTART_REUSE_TRAIL_ARMED_MIN",
        "SAT_REPHASE_UNARMED_MIN",
        "SAT_WALK_WARMUP_UNARMED",
        "SAT_WALK_STALL_GIVEUP",
        "SAT_GAUSS_MIN_COVERAGE",
        "SAT_SWEEPCOUNT",
        "SAT_SWEEP_YIELD_ESCALATE",
        "SAT_SWEEP_YIELD_MIN_EQUIVS",
        "SAT_SWEEP_YIELD_PROBE",
        "SAT_SWEEP_FAITHFUL",
        "SAT_SWEEP_FAITHFUL_EFFORT",
        "SAT_WALK_EFFORT_YIELD_ARMED",
        "SAT_DEBUG_SWEEP",
        "SAT_DEBUG_SWEEPCOUNT",
        "SAT_BACKBONE",
        "SAT_BACKBONE_SCOPE",
        "SAT_BACKBONE_ARMED_MIN",
        "SAT_BACKBONE_EFFORT",
        "SAT_BACKBONE_TICKS",
        "SAT_BACKBONE_ROUNDS",
        "SAT_BACKBONE_MAX_ROUNDS",
        "SAT_TRANSITIVE",
        "SAT_FORWARD_SUBSUME",
        "SAT_GATE_EXTRACT",
        "SAT_GATE_BVE",
        "SAT_GATE_BVE_SCOPED",
        "SAT_GATE_BVE_MIN_GAIN_PCT",
        "SAT_GATE_BVE_SCOPED_MAX_VARS",
        "SAT_RCHECK",
        "SAT_GAUSS",
        "SAT_FACTOR",
        "SAT_PAIR_ABS_REFUTE",
        "SAT_ELS",
        "SAT_CONGRUENCE",
        "SAT_CONGRUENCE_XOR",
        "SAT_CONGRUENCE_ITER",
        "SAT_INPROCESS_INTERVAL_CONFLICTS",
        "SAT_INPROCESS_MAX_ROUNDS",
        "SAT_VIVIFY_TICKS",
        "SAT_VIVIFY_PERMILLE",
        "SAT_VIVIFY_MAX_CLAUSE_LEN",
        "SAT_PROBE_TICKS",
        "SAT_ELIMINATE_TICKS",
        "SAT_ELIMINATE_RESOLUTIONS",
        "SAT_ELIMINATE_OCCLIM",
        "SAT_TRANSITIVE_MAX_DEPTH",
        "SAT_TRANSITIVE_TICKS_PER_SOURCE",
        "SAT_TRANSITIVE_MAX_REMOVED_PER_ROUND",
        "SAT_TRANSITIVE_TICKS",
        "SAT_TRANSITIVE_MIN_REMOVED_PERMILLE",
        "SAT_TRANSITIVE_UNITS_ONLY",
        "SAT_TRANSITIVE_INPROCESS",
        "SAT_TRANSITIVE_INPROCESS_MIN_REMOVED_PERMILLE",
        "SAT_ELS_INPROCESS",
        "SAT_PROBE_INPROCESS",
        "SAT_TRANSITIVE_INPROCESS_GBVE",
        "SAT_PROBE_INPROCESS_GBVE",
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
    fn test_default_config_uses_fstab_lbdtier_search_defaults() {
        // profile20 Stage-1 ablation (2026-05-30/31) promoted the "fstab_lbdtier" config to the
        // default/fast profiles: focused-stable + LBD + tick mode-switching + LBD-tiered reduction
        // (VMTF auto-resolves to FocusedOnly), with the guarded sweep inprocessing cadence
        // promoted later as a conservative preprocessing increment.
        let config = SolverConfig::from_env_map(&env_map(&[]));

        assert_eq!(config.profile, SolverProfile::Default);
        assert_eq!(config.axes.search, SearchAxis::Validated);
        assert_eq!(config.axes.preprocess, PreprocessAxis::Conservative);
        assert!(config.simplification);
        assert!(config.bve);
        assert!(config.full_bsr);
        assert!(config.bsr_formula_gate);
        assert!(config.use_lbd);
        assert_eq!(config.search_mode_policy, SearchModePolicy::FocusedStable);
        assert!(config.mode_use_ticks);
        assert_eq!(config.reduce_policy, ReducePolicy::LbdTiered);
        assert_eq!(config.phase_policy, PhasePolicy::TargetThenSaved);
        assert_eq!(config.bsr_occurrence_limit, 1000);
        assert!(config.inprocess);
        assert_eq!(config.inprocess_interval_conflicts, 1_000_000);
        assert!(config.vivify);
        assert!(config.factor);
        assert_eq!(config.vmtf, VmtfMode::FocusedOnly);
        assert!(config.lucky);
        assert_eq!(config.proof_policy, ProofPolicy::DratBinary);
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
        assert!(!config.bsr_formula_gate);
        assert!(!config.inprocess);
        assert_eq!(config.inprocess_interval_conflicts, 0);
        assert!(!config.vivify);
        assert_eq!(config.bsr_occurrence_limit, 0);
        assert!(!config.use_lbd);
        assert_eq!(config.search_mode_policy, SearchModePolicy::Single);
        assert!(!config.mode_use_ticks);
        assert_eq!(config.phase_policy, PhasePolicy::Legacy);
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
    fn test_default_fast_profiles_promote_target_phase_only() {
        for profile in ["default", "fast"] {
            let config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", profile)]));

            assert_eq!(config.phase_policy, PhasePolicy::TargetThenSaved);
        }

        for profile in ["baseline", "experimental"] {
            let config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", profile)]));

            assert_eq!(config.phase_policy, PhasePolicy::Legacy);
        }
    }

    #[test]
    fn test_target_phase_default_survives_ema_restart_and_can_be_overridden() {
        let ema_default_phase = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_RESTART", "kissat-ema"),
        ]));
        assert_eq!(ema_default_phase.restart_policy, RestartPolicy::KissatEma);
        assert_eq!(ema_default_phase.phase_policy, PhasePolicy::TargetThenSaved);

        let explicit_saved = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_SEARCH_MODE", "focused-stable"),
            ("SAT_RESTART", "kissat-ema"),
            ("SAT_PHASE", "saved"),
        ]));
        assert_eq!(explicit_saved.restart_policy, RestartPolicy::KissatEma);
        assert_eq!(explicit_saved.phase_policy, PhasePolicy::Saved);
    }

    #[test]
    fn test_config_hash_changes_with_effective_feature_flag() {
        let off = SolverConfig::from_env_map(&env_map(&[
            ("SAT_SEARCH_MODE", "single"),
            ("SAT_MODE_USE_TICKS", "off"),
            ("SAT_PHASE", "legacy"),
            ("SAT_USE_LBD", "off"),
            // default reduce policy is now lbd-tiered, which requires use_lbd; reset it so the
            // use_lbd=off fixture is a valid config (lbd-tiered+!lbd hard-fails validation).
            ("SAT_REDUCE", "legacy"),
            // 5b2.2.67: the default profile now also enables LBD reason updates,
            // which likewise require use_lbd; disable them for this use_lbd=off fixture.
            ("SAT_LBD_UPDATE_REASONS", "off"),
            ("SAT_LBD_UPDATE_PROP_REASONS", "off"),
        ]));
        let on = SolverConfig::from_env_map(&env_map(&[("SAT_USE_LBD", "on")]));

        assert_ne!(off.config_hash(), on.config_hash());
    }

    #[test]
    fn test_lbd_reason_update_and_tiered_reduce_are_runtime_supported() {
        // 5b2.2.67: the default profile now enables both LBD-update flags, so
        // pin PROP off explicitly here to verify the flags remain independently
        // controllable (REASONS on, PROP off).
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_USE_LBD", "on"),
            ("SAT_LBD_UPDATE_REASONS", "on"),
            ("SAT_LBD_UPDATE_PROP_REASONS", "off"),
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
        // The default search mode is now focused-stable, so single-mode VMTF must request single
        // mode explicitly (SAT_VMTF=single requires SAT_SEARCH_MODE=single), and the default's
        // focused-stable-only mode_use_ticks must be turned off (it requires focused-stable).
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_SEARCH_MODE", "single"),
            ("SAT_MODE_USE_TICKS", "off"),
            ("SAT_PHASE", "legacy"),
            ("SAT_VMTF", "single"),
        ]));

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
        // Promoted ON in default/fast (2026-07-05, +2 solved on medium over the
        // bump_reasons baseline); OFF in baseline; env-overridable.
        assert!(default_config.chrono_backtrack);
        assert!(SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "fast")])).chrono_backtrack);
        assert!(!SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")])).chrono_backtrack);
        assert!(!SolverConfig::from_env_map(&env_map(&[("SAT_CHRONO", "off")])).chrono_backtrack);

        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_CHRONO", "on"),
            ("SAT_CHRONO_MAX_DELTA", "7"),
        ]));

        assert!(config.chrono_backtrack);
        assert_eq!(config.chrono_max_delta, 7);
    }

    #[test]
    fn test_lucky_promoted_default_on_baseline_off_overridable() {
        // 70h: SAT_LUCKY promoted ON in default/fast (2026-05-30), OFF in baseline, env-overridable.
        let config = SolverConfig::from_env_map(&env_map(&[]));
        assert!(config.lucky);

        let fast = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "fast")]));
        assert!(fast.lucky);

        let baseline = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
        assert!(!baseline.lucky);

        let off = SolverConfig::from_env_map(&env_map(&[("SAT_LUCKY", "off")]));
        assert!(!off.lucky);

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
    fn test_bump_reasons_on_in_default_and_fast_off_in_baseline() {
        // Promoted to the default/fast profiles 2026-07-05 (kissat-parity reason-side
        // bumping; medium single-seed A/B -41% both-solved conflicts, gate PASS).
        let config = SolverConfig::from_env_map(&env_map(&[]));
        assert!(config.bump_reasons);
        assert_eq!(config.bump_reasons_limit_multiplier, 10);

        let fast = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "fast")]));
        assert!(fast.bump_reasons);

        let baseline = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
        assert!(!baseline.bump_reasons);

        let disabled = SolverConfig::from_env_map(&env_map(&[("SAT_BUMP_REASONS", "off")]));
        assert!(!disabled.bump_reasons);

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
        assert_eq!(config.clause_min_mode, ClauseMinMode::InBlockLate);
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
    fn test_prefetch_watched_clauses_is_parsed_and_replayable() {
        // bead 5b2.8.1: conflict-preserving propagation prefetch, promoted to default+fast
        // (off in raw/baseline). Mirrors the bsr_drain_batched promotion pattern.
        assert!(!SolverConfig::default().prefetch_watched_clauses);
        assert!(SolverConfig::from_env_map(&env_map(&[])).prefetch_watched_clauses);
        assert!(
            SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "fast")])).prefetch_watched_clauses
        );
        assert!(
            !SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]))
                .prefetch_watched_clauses
        );
        assert!(
            !SolverConfig::from_env_map(&env_map(&[("SAT_PREFETCH", "off")])).prefetch_watched_clauses
        );

        let config = SolverConfig::from_env_map(&env_map(&[("SAT_PREFETCH", "on")]));
        assert!(config.prefetch_watched_clauses);

        let replay = config.config_replay_text();
        assert!(replay.contains("prefetch_watched_clauses=true"));
        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<prefetch-test>"));
        assert_eq!(
            replayed.prefetch_watched_clauses,
            config.prefetch_watched_clauses
        );
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_inblock_clause_minimization_is_runtime_supported() {
        let config = SolverConfig::from_env_map(&env_map(&[("SAT_CLAUSE_MIN", "inblock")]));

        assert_eq!(config.clause_min_mode, ClauseMinMode::InBlockShrink);
    }

    #[test]
    fn test_late_inblock_clause_minimization_is_replayable() {
        let config = SolverConfig::from_env_map(&env_map(&[
            ("SAT_CLAUSE_MIN", "inblock-late"),
            ("SAT_INBLOCK_DELAY_CONFLICTS", "123456"),
            ("SAT_INBLOCK_BINARY_MIN", "0.9"),
        ]));

        assert_eq!(config.clause_min_mode, ClauseMinMode::InBlockLate);
        assert_eq!(config.inblock_delay_conflicts, 123_456);
        assert!((config.inblock_binary_min - 0.9).abs() < 1e-12);

        let replay = config.config_replay_text();
        assert!(replay.contains("clause_min_mode=inblock-late"));
        assert!(replay.contains("inblock_delay_conflicts=123456"));
        assert!(replay.contains("inblock_binary_min=0.9"));
        let replayed =
            SolverConfig::from_replay_text(&replay, Path::new("<inblock-late-test>"));
        assert_eq!(replayed.clause_min_mode, config.clause_min_mode);
        assert_eq!(
            replayed.inblock_delay_conflicts,
            config.inblock_delay_conflicts
        );
        assert!((replayed.inblock_binary_min - config.inblock_binary_min).abs() < 1e-12);
        assert_eq!(replayed.config_hash(), config.config_hash());
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
    fn test_default_eliminate_budgets_bound_the_preprocess_pass() {
        // bead 5b2.3.24 (analyzesat PRE-1): the eliminate pass must ship with a
        // bounded default effort budget so preprocessing wall is bounded on hard
        // formulas; explicit 0 stays available as the unlimited opt-out.
        let config = SolverConfig::from_env_map(&env_map(&[]));
        assert_eq!(config.eliminate_ticks_budget, DEFAULT_ELIMINATE_TICKS_BUDGET);
        assert_eq!(
            config.eliminate_resolution_budget,
            DEFAULT_ELIMINATE_RESOLUTION_BUDGET
        );
        assert!(DEFAULT_ELIMINATE_TICKS_BUDGET > 0);
        assert!(DEFAULT_ELIMINATE_RESOLUTION_BUDGET > 0);

        for (env_var, expected) in [
            ("SAT_ELIMINATE_TICKS", DEFAULT_ELIMINATE_TICKS_BUDGET),
            ("SAT_ELIMINATE_RESOLUTIONS", DEFAULT_ELIMINATE_RESOLUTION_BUDGET),
        ] {
            let schema_row = CONFIG_SCHEMA_CSV
                .lines()
                .find(|line| line.starts_with(&format!("{env_var},")))
                .unwrap_or_else(|| panic!("{env_var} schema row"));
            let columns: Vec<&str> = schema_row.split(',').collect();
            let expected = expected.to_string();
            assert_eq!(columns[4], expected, "{env_var} baseline_default");
            assert_eq!(columns[5], expected, "{env_var} default_default");
            assert_eq!(columns[6], expected, "{env_var} fast_default");
        }

        let unlimited = SolverConfig::from_env_map(&env_map(&[
            ("SAT_ELIMINATE_TICKS", "0"),
            ("SAT_ELIMINATE_RESOLUTIONS", "0"),
        ]));
        assert_eq!(unlimited.eliminate_ticks_budget, 0);
        assert_eq!(unlimited.eliminate_resolution_budget, 0);
    }

    #[test]
    fn test_eliminate_occurrence_limit_is_parsed_and_replayable() {
        let config = SolverConfig::from_env_map(&env_map(&[("SAT_ELIMINATE_OCCLIM", "2000")]));
        assert_eq!(config.eliminate_occurrence_limit, 2000);

        let replay = config.config_replay_text();
        assert!(replay.contains("eliminate_occurrence_limit=2000"));
        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<occlim-test>"));
        assert_eq!(
            replayed.eliminate_occurrence_limit,
            config.eliminate_occurrence_limit
        );
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_vivify_permille_is_parsed_and_replayable() {
        // Defaults to 0 (=> the solver uses DEFAULT_VIVIFY_PERMILLE).
        let default_config = SolverConfig::from_env_map(&env_map(&[]));
        assert_eq!(default_config.vivify_permille, 0);

        let config = SolverConfig::from_env_map(&env_map(&[("SAT_VIVIFY_PERMILLE", "250")]));
        assert_eq!(config.vivify_permille, 250);

        let replay = config.config_replay_text();
        assert!(replay.contains("vivify_permille=250"));
        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<vivify-permille-test>"));
        assert_eq!(replayed.vivify_permille, config.vivify_permille);
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_bsr_occurrence_limit_is_parsed_and_replayable() {
        let raw_config = SolverConfig::default();
        assert_eq!(raw_config.bsr_occurrence_limit, 0);

        let default_config = SolverConfig::from_env_map(&env_map(&[]));
        assert_eq!(default_config.bsr_occurrence_limit, 1000);

        let fast_config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "fast")]));
        assert_eq!(fast_config.bsr_occurrence_limit, 1000);

        let baseline_config =
            SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
        assert_eq!(baseline_config.bsr_occurrence_limit, 0);

        let config = SolverConfig::from_env_map(&env_map(&[("SAT_BSR_OCCLIM", "1000")]));
        assert_eq!(config.bsr_occurrence_limit, 1000);

        let disabled = SolverConfig::from_env_map(&env_map(&[("SAT_BSR_OCCLIM", "0")]));
        assert_eq!(disabled.bsr_occurrence_limit, 0);

        let replay = config.config_replay_text();
        assert!(replay.contains("bsr_occurrence_limit=1000"));
        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<bsr-occlim-test>"));
        assert_eq!(replayed.bsr_occurrence_limit, config.bsr_occurrence_limit);
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_bsr_formula_gate_is_promoted_and_overridable() {
        let raw_config = SolverConfig::default();
        assert!(!raw_config.bsr_formula_gate);

        let default_config = SolverConfig::from_env_map(&env_map(&[]));
        assert!(default_config.bsr_formula_gate);

        let fast_config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "fast")]));
        assert!(fast_config.bsr_formula_gate);

        let baseline_config =
            SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
        assert!(!baseline_config.bsr_formula_gate);

        let disabled =
            SolverConfig::from_env_map(&env_map(&[("SAT_BSR_FORMULA_GATE", "off")]));
        assert!(!disabled.bsr_formula_gate);

        let enabled = SolverConfig::from_env_map(&env_map(&[("SAT_BSR_FORMULA_GATE", "on")]));
        assert!(enabled.bsr_formula_gate);

        let replay = disabled.config_replay_text();
        assert!(replay.contains("bsr_formula_gate=false"));
        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<bsr-gate-test>"));
        assert_eq!(replayed.bsr_formula_gate, disabled.bsr_formula_gate);
        assert_eq!(replayed.config_hash(), disabled.config_hash());
    }

    #[test]
    fn test_lbd_reason_update_promoted_in_default_profile() {
        // SAT-playground-5b2.2.67: glue recompute + tier promotion on reason use
        // is on by default in default/fast (lbd-tiered), off in raw/baseline.
        assert!(!SolverConfig::default().update_reason_lbd);
        let default_cfg = SolverConfig::from_env_map(&env_map(&[]));
        assert!(default_cfg.update_reason_lbd);
        assert!(default_cfg.update_propagation_reason_lbd);
        let fast = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "fast")]));
        assert!(fast.update_reason_lbd && fast.update_propagation_reason_lbd);
        let baseline = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
        assert!(!baseline.update_reason_lbd && !baseline.update_propagation_reason_lbd);
        let off = SolverConfig::from_env_map(&env_map(&[
            ("SAT_LBD_UPDATE_REASONS", "off"),
            ("SAT_LBD_UPDATE_PROP_REASONS", "off"),
        ]));
        assert!(!off.update_reason_lbd && !off.update_propagation_reason_lbd);
    }

    #[test]
    fn test_bsr_drain_batched_is_parsed_and_replayable() {
        let raw_config = SolverConfig::default();
        assert!(!raw_config.bsr_drain_batched);

        let default_config = SolverConfig::from_env_map(&env_map(&[]));
        assert!(default_config.bsr_drain_batched);

        let fast_config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "fast")]));
        assert!(fast_config.bsr_drain_batched);

        let baseline_config =
            SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
        assert!(!baseline_config.bsr_drain_batched);

        let disabled = SolverConfig::from_env_map(&env_map(&[("SAT_BSR_DRAIN_BATCHED", "off")]));
        assert!(!disabled.bsr_drain_batched);

        let config = SolverConfig::from_env_map(&env_map(&[("SAT_BSR_DRAIN_BATCHED", "on")]));
        assert!(config.bsr_drain_batched);

        let replay = config.config_replay_text();
        assert!(replay.contains("bsr_drain_batched=true"));
        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<bsr-drain-test>"));
        assert_eq!(replayed.bsr_drain_batched, config.bsr_drain_batched);
        assert_eq!(replayed.config_hash(), config.config_hash());
    }

    #[test]
    fn test_pair_abs_refute_is_default_profile_and_replayable() {
        assert!(!SolverConfig::default().pair_abs_refute);

        let default_config = SolverConfig::from_env_map(&env_map(&[]));
        assert!(default_config.pair_abs_refute);

        let fast_config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "fast")]));
        assert!(fast_config.pair_abs_refute);

        let baseline_config =
            SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
        assert!(!baseline_config.pair_abs_refute);

        let disabled = SolverConfig::from_env_map(&env_map(&[("SAT_PAIR_ABS_REFUTE", "off")]));
        assert!(!disabled.pair_abs_refute);

        let replay = default_config.config_replay_text();
        assert!(replay.contains("pair_abs_refute=true"));
        let replayed = SolverConfig::from_replay_text(&replay, Path::new("<pair-abs-test>"));
        assert_eq!(replayed.pair_abs_refute, default_config.pair_abs_refute);
        assert_eq!(replayed.config_hash(), default_config.config_hash());
    }

    #[test]
    fn test_schema_and_feature_csv_are_loaded_into_binary() {
        assert!(CONFIG_SCHEMA_CSV.contains("SAT_USE_LBD"));
        assert!(CONFIG_SCHEMA_CSV.contains("SAT_CONFIG_REPLAY"));
        assert!(CONFIG_SCHEMA_CSV.contains("SAT_BSR_OCCLIM"));
        assert!(CONFIG_SCHEMA_CSV.contains("SAT_ELIMINATE_RESOLUTIONS"));
        assert!(CONFIG_SCHEMA_CSV.contains("SAT_ELIMINATE_OCCLIM"));
        assert!(CONFIG_SCHEMA_CSV.contains("SAT_PAIR_ABS_REFUTE"));
        assert!(FEATURES_CSV.contains("SAT_USE_LBD"));
        assert!(FEATURES_CSV.contains("SAT_FULL_BSR"));
        assert!(FEATURES_CSV.contains("SAT_PAIR_ABS_REFUTE"));
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
