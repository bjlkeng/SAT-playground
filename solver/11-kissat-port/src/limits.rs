//! Limit-check boundary.
//!
//! Section 0 keeps limit handling intentionally small and explicit.  Search
//! limits end the current solve as `UNKNOWN`; optional-pass and edit budgets
//! are named here so later inprocessing work can route its local aborts through
//! the same vocabulary without changing the external result contract.

use crate::config::SolverConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum BudgetClass {
    SolveLimit,
    PassBudget,
    EditBudget,
    EmergencyMemoryLimit,
}

impl BudgetClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SolveLimit => "solve-limit",
            Self::PassBudget => "pass-budget",
            Self::EditBudget => "edit-budget",
            Self::EmergencyMemoryLimit => "emergency-memory-limit",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LimitHit {
    pub(crate) class: BudgetClass,
    pub(crate) reason: &'static str,
}

impl LimitHit {
    pub(crate) fn solve(reason: &'static str) -> Self {
        Self {
            class: BudgetClass::SolveLimit,
            reason,
        }
    }

    pub(crate) fn emergency_memory(reason: &'static str) -> Self {
        Self {
            class: BudgetClass::EmergencyMemoryLimit,
            reason,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RuntimeLimits {
    active: bool,
    pub(crate) conflict_limit: Option<u64>,
    pub(crate) propagation_limit: Option<u64>,
    pub(crate) tick_limit: Option<u64>,
    pub(crate) wall_limit_sec: Option<f64>,
    pub(crate) rss_limit_mb: Option<u64>,
    pub(crate) learned_lit_limit: Option<u64>,
    pub(crate) binary_clause_limit: Option<u64>,
    pub(crate) extension_bytes_limit: Option<u64>,
    pub(crate) proof_bytes_limit: Option<u64>,
}

impl RuntimeLimits {
    pub(crate) fn from_config(config: &SolverConfig) -> Self {
        let active = config.conflict_limit.is_some()
            || config.propagation_limit.is_some()
            || config.tick_limit.is_some()
            || config.wall_limit_sec.is_some()
            || config.rss_limit_mb.is_some()
            || config.learned_lit_limit.is_some()
            || config.binary_clause_limit.is_some()
            || config.extension_bytes_limit.is_some()
            || config.proof_bytes_limit.is_some();
        Self {
            active,
            conflict_limit: config.conflict_limit,
            propagation_limit: config.propagation_limit,
            tick_limit: config.tick_limit,
            wall_limit_sec: config.wall_limit_sec,
            rss_limit_mb: config.rss_limit_mb,
            learned_lit_limit: config.learned_lit_limit,
            binary_clause_limit: config.binary_clause_limit,
            extension_bytes_limit: config.extension_bytes_limit,
            proof_bytes_limit: config.proof_bytes_limit,
        }
    }

    #[inline]
    pub(crate) fn is_active(&self) -> bool {
        self.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_limits_default_to_inactive() {
        let limits = RuntimeLimits::from_config(&SolverConfig::default());

        assert!(!limits.is_active());
        assert_eq!(limits.conflict_limit, None);
        assert_eq!(limits.propagation_limit, None);
        assert_eq!(limits.tick_limit, None);
        assert_eq!(limits.wall_limit_sec, None);
        assert_eq!(limits.rss_limit_mb, None);
        assert_eq!(limits.learned_lit_limit, None);
        assert_eq!(limits.binary_clause_limit, None);
        assert_eq!(limits.extension_bytes_limit, None);
        assert_eq!(limits.proof_bytes_limit, None);
    }

    #[test]
    fn runtime_limits_notice_each_supported_limit() {
        macro_rules! assert_active {
            ($field:ident, $value:expr) => {{
                let mut config = SolverConfig::default();
                config.$field = Some($value);
                let limits = RuntimeLimits::from_config(&config);
                assert!(
                    limits.is_active(),
                    "{} should activate limits",
                    stringify!($field)
                );
                assert_eq!(limits.$field, Some($value));
            }};
        }

        assert_active!(conflict_limit, 0);
        assert_active!(propagation_limit, 0);
        assert_active!(tick_limit, 0);
        assert_active!(wall_limit_sec, 0.0);
        assert_active!(rss_limit_mb, 1);
        assert_active!(learned_lit_limit, 0);
        assert_active!(binary_clause_limit, 0);
        assert_active!(extension_bytes_limit, 0);
        assert_active!(proof_bytes_limit, 0);
    }
}
