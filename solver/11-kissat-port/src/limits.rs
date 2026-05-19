//! Limit-check boundary.
//!
//! Section 0 keeps limit handling intentionally small and explicit.  Search
//! limits end the current solve as `UNKNOWN`; optional-pass and edit budgets
//! are named here so later inprocessing work can route its local aborts through
//! the same vocabulary without changing the external result contract.

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
