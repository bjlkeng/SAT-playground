//! Gate congruence closure (SAT_CONGRUENCE): Phase B of the inprocessing-pipeline
//! effort (bead SAT-playground-otd).
//!
//! Two gates that compute the *same function of the same inputs* have equivalent
//! outputs. Circuit miters (BubbleVsPancakeSort, VexRiscv) encode the two circuits
//! being compared with many structurally identical gates; detecting that their
//! outputs are equivalent and substituting one for the other collapses the miter and
//! unlocks bounded-variable elimination (BVE) that the plain solver cannot reach.
//!
//! This module is the *pure* core. The solver-side gate extraction (reading the clause
//! database), the DRAT proof emission, and the substitution (reusing the Phase A ELS
//! machinery) live in `main.rs`. Here we only:
//!   * normalize an ITE triple to a canonical form ([`normalize_ite`]), and
//!   * hash the extracted gates by (type, normalized inputs) and report which output
//!     literals must be merged as equivalences ([`find_merges`]).
//!
//! ## Gate normalization
//! * AND / OR gates are folded into a single AND representation: an OR gate
//!   `L = OR(o1..ok)` is the AND gate `¬L = AND(¬o1..¬ok)`, so every gate is stored as
//!   `out = AND(inputs)` with `inputs` sorted. Two AND gates with identical sorted
//!   inputs have equal outputs.
//! * ITE gates `out = ITE(cond, then, else)` are normalized by [`normalize_ite`] so the
//!   condition and the then-branch are positive literals; the normalization may flip the
//!   output polarity. Two ITE gates with identical normalized `[cond, then, else]` have
//!   equal (canonical) outputs.
//!
//! ## Soundness
//! Every gate is extracted from clauses actually present in the formula, so the
//! biconditional `out ↔ f(inputs)` is entailed. When two gates share a key their outputs
//! `p` and `q` satisfy `p ≡ q`, and the two equivalence binaries `(¬p∨q), (p∨¬q)` are
//! RUP from the gates' defining clauses (directly for AND, via a short resolution chain
//! for ITE — emitted by the driver). If instead `q == ¬p` the formula is UNSAT.

use std::collections::HashMap;

/// The shape of a detected gate. Determines the DRAT proof chain the driver emits when
/// two such gates are merged.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GateKind {
    /// `out = AND(inputs)` (OR gates are folded into this on the negated output).
    And,
    /// `out = ITE(inputs[0], inputs[1], inputs[2])`, normalized by [`normalize_ite`].
    Ite,
}

/// A gate extracted from the clause database in normalized form.
#[derive(Clone, Debug)]
pub(crate) struct Gate {
    /// Output literal (already polarity-adjusted for the canonical form).
    pub(crate) out: i32,
    pub(crate) kind: GateKind,
    /// AND: sorted input literals. ITE: `[cond, then, else]` in canonical order.
    pub(crate) inputs: Vec<i32>,
}

/// The proof-chain shape needed to certify a merge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MergeKind {
    And,
    Ite { cond: i32 },
}

/// Two gate outputs that are provably equivalent: `p ≡ q`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Merge {
    /// The representative output (first gate seen for the key).
    pub(crate) p: i32,
    /// The colliding output; `p ≡ q`.
    pub(crate) q: i32,
    pub(crate) kind: MergeKind,
}

/// Result of congruence matching over a set of gates.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    /// Equivalences `p ≡ q` to install (both distinct variables, `q != ¬p`).
    pub(crate) merges: Vec<Merge>,
    /// A merge that equates a literal with its own negation (`q == ¬p`) — the formula is
    /// UNSAT and the driver emits the refutation instead of substituting.
    pub(crate) unsat: Option<Merge>,
}

/// Normalize an ITE right-hand side `[cond, then, else]` in place; returns whether the
/// left-hand side (output) must be negated to reach the canonical form.
///
/// Uses the identities `ITE(¬c, t, e) = ITE(c, e, t)` and `¬x = ITE(c, ¬t, ¬e)` when
/// `x = ITE(c, t, e)`. After normalization `cond > 0` and `then > 0`.
pub(crate) fn normalize_ite(rhs: &mut [i32; 3]) -> bool {
    if rhs[0] < 0 {
        rhs[0] = -rhs[0];
        rhs.swap(1, 2);
    }
    if rhs[1] > 0 {
        return false;
    }
    rhs[1] = -rhs[1];
    rhs[2] = -rhs[2];
    true
}

/// Hash the gates by `(kind, inputs)` and report the output equivalences that follow.
///
/// The first gate seen for a key becomes the representative; every later gate with the
/// same key yields a merge `repr ≡ out` (skipped when they are already the same literal).
/// A collision `repr == ¬out` means a literal is equivalent to its negation, i.e. the
/// formula is UNSAT — returned via [`Plan::unsat`] (matching stops there).
///
/// Degenerate ITE merges whose condition shares a variable with either output are
/// dropped: their proof chain would not be expressible as plain binaries. Skipping a
/// merge is always sound (it only forgoes an optimization).
pub(crate) fn find_merges(gates: &[Gate]) -> Plan {
    let mut table: HashMap<(u8, Vec<i32>), i32> = HashMap::new();
    let mut merges: Vec<Merge> = Vec::new();
    for g in gates {
        let tag = match g.kind {
            GateKind::And => 0u8,
            GateKind::Ite => 1u8,
        };
        let key = (tag, g.inputs.clone());
        match table.get(&key).copied() {
            None => {
                table.insert(key, g.out);
            }
            Some(rep) => {
                if rep == g.out {
                    continue;
                }
                let kind = match g.kind {
                    GateKind::And => MergeKind::And,
                    GateKind::Ite => MergeKind::Ite { cond: g.inputs[0] },
                };
                // Drop degenerate ITE merges (condition shares a variable with an output)
                // before deciding UNSAT-vs-merge: their proof chain over `cond` collapses.
                // Skipping is sound; it only forgoes an optimization / detection.
                if let MergeKind::Ite { cond } = kind {
                    let cv = cond.unsigned_abs();
                    if cv == rep.unsigned_abs() || cv == g.out.unsigned_abs() {
                        continue;
                    }
                }
                let merge = Merge {
                    p: rep,
                    q: g.out,
                    kind,
                };
                if rep == -g.out {
                    return Plan {
                        merges,
                        unsat: Some(merge),
                    };
                }
                merges.push(merge);
            }
        }
    }
    Plan {
        merges,
        unsat: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn and_gate(out: i32, mut inputs: Vec<i32>) -> Gate {
        inputs.sort_unstable();
        Gate {
            out,
            kind: GateKind::And,
            inputs,
        }
    }

    fn ite_gate(out_lit: i32, cond: i32, then_lit: i32, else_lit: i32) -> Gate {
        let mut rhs = [cond, then_lit, else_lit];
        let negate = normalize_ite(&mut rhs);
        Gate {
            out: if negate { -out_lit } else { out_lit },
            kind: GateKind::Ite,
            inputs: rhs.to_vec(),
        }
    }

    #[test]
    fn no_gates_no_merges() {
        let plan = find_merges(&[]);
        assert!(plan.merges.is_empty());
        assert!(plan.unsat.is_none());
    }

    #[test]
    fn and_gates_same_inputs_merge_outputs() {
        // x = AND(1,2) and y = AND(2,1) ⇒ x ≡ y.
        let gates = vec![and_gate(3, vec![1, 2]), and_gate(4, vec![2, 1])];
        let plan = find_merges(&gates);
        assert!(plan.unsat.is_none());
        assert_eq!(plan.merges.len(), 1);
        let m = plan.merges[0];
        assert_eq!(m.p, 3);
        assert_eq!(m.q, 4);
        assert_eq!(m.kind, MergeKind::And);
    }

    #[test]
    fn and_gates_different_inputs_do_not_merge() {
        let gates = vec![and_gate(3, vec![1, 2]), and_gate(4, vec![1, -2])];
        let plan = find_merges(&gates);
        assert!(plan.merges.is_empty());
        assert!(plan.unsat.is_none());
    }

    #[test]
    fn and_gates_output_and_negation_is_unsat() {
        // x = AND(1,2) and ¬x = AND(1,2) ⇒ x ≡ ¬x ⇒ UNSAT.
        let gates = vec![and_gate(3, vec![1, 2]), and_gate(-3, vec![1, 2])];
        let plan = find_merges(&gates);
        let u = plan.unsat.expect("must be UNSAT");
        assert_eq!(u.p, 3);
        assert_eq!(u.q, -3);
    }

    #[test]
    fn or_and_congruence_via_and_folding() {
        // x = OR(1,2) folds to ¬x = AND(-1,-2); y = AND(-1,-2) ⇒ ¬x ≡ y, i.e. outputs -3, 4.
        let or_x = and_gate(-3, vec![-1, -2]); // ¬x = AND(-1,-2)
        let and_y = and_gate(4, vec![-1, -2]);
        let plan = find_merges(&[or_x, and_y]);
        assert_eq!(plan.merges.len(), 1);
        assert_eq!(plan.merges[0].p, -3);
        assert_eq!(plan.merges[0].q, 4);
    }

    #[test]
    fn ite_gates_same_function_merge() {
        // x = ITE(1,2,3) and y = ITE(1,2,3) ⇒ x ≡ y.
        let gates = vec![ite_gate(4, 1, 2, 3), ite_gate(5, 1, 2, 3)];
        let plan = find_merges(&gates);
        assert!(plan.unsat.is_none());
        assert_eq!(plan.merges.len(), 1);
        let m = plan.merges[0];
        assert_eq!(m.kind, MergeKind::Ite { cond: 1 });
        // Outputs are the canonical (unnegated here) forms.
        assert_eq!(m.p, 4);
        assert_eq!(m.q, 5);
    }

    #[test]
    fn ite_normalization_matches_across_condition_flip() {
        // ITE(-1,2,3) == ITE(1,3,2): flipping the condition swaps then/else.
        let g1 = ite_gate(4, -1, 2, 3);
        let g2 = ite_gate(5, 1, 3, 2);
        let plan = find_merges(&[g1, g2]);
        assert_eq!(plan.merges.len(), 1, "condition-flipped ITEs must match");
    }

    #[test]
    fn ite_normalization_negated_then_flips_output() {
        // x = ITE(1,-2,3) normalizes to ¬x = ITE(1,2,-3).
        let mut rhs = [1, -2, 3];
        let negate = normalize_ite(&mut rhs);
        assert!(negate);
        assert_eq!(rhs, [1, 2, -3]);
    }

    #[test]
    fn ite_and_do_not_cross_match() {
        // Same literal tuple but different gate kinds must not merge.
        let a = and_gate(4, vec![1, 2]);
        let i = Gate {
            out: 5,
            kind: GateKind::Ite,
            inputs: vec![1, 2, 3],
        };
        let plan = find_merges(&[a, i]);
        assert!(plan.merges.is_empty());
    }
}
