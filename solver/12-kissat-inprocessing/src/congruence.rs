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

use crate::fxhash::FxHashMap as HashMap;
use std::collections::VecDeque;

/// SAT_ELIM_SCRATCH=off replays the pre-diet unconditional gate-inputs clone in
/// `find_merges_closure` (the fair A/B baseline arm). Read once per process.
fn legacy_clone_mode() -> bool {
    static MODE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| matches!(std::env::var("SAT_ELIM_SCRATCH").ok().as_deref(), Some("off") | Some("0") | Some("false")))
}

/// The shape of a detected gate. Determines the DRAT proof chain the driver emits when
/// two such gates are merged.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GateKind {
    /// `out = AND(inputs)` (OR gates are folded into this on the negated output).
    And,
    /// `out = ITE(inputs[0], inputs[1], inputs[2])`, normalized by [`normalize_ite`].
    Ite,
    /// `out = XOR(inputs)` where every input is a positive variable-literal and the parity
    /// target is folded into `out`'s sign (see `main.rs` XOR extraction). Two XOR gates with
    /// the same input set have equivalent outputs (`v = a⊕b⊕… and w = a⊕b⊕… ⇒ v ≡ w`).
    Xor,
}

/// A gate extracted from the clause database in normalized form.
#[derive(Clone, Debug)]
pub(crate) struct Gate {
    /// Output literal (already polarity-adjusted for the canonical form).
    pub(crate) out: i32,
    pub(crate) kind: GateKind,
    /// AND: sorted input literals. ITE: `[cond, then, else]` in canonical order.
    /// XOR: sorted positive input variable-literals.
    pub(crate) inputs: Vec<i32>,
}

/// The proof-chain shape needed to certify a merge.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum MergeKind {
    And,
    Ite { cond: i32 },
    /// The shared, polarity-normalized XOR input literals — needed to emit the parity
    /// resolution ladder that makes the equivalence binaries RUP.
    Xor { inputs: Vec<i32> },
}

/// Two gate outputs that are provably equivalent: `p ≡ q`.
#[derive(Clone, PartialEq, Eq, Debug)]
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
    let mut table: HashMap<(u8, Vec<i32>), i32> = HashMap::default();
    let mut merges: Vec<Merge> = Vec::new();
    for g in gates {
        let tag = match g.kind {
            GateKind::And => 0u8,
            GateKind::Ite => 1u8,
            GateKind::Xor => 2u8,
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
                    GateKind::Xor => MergeKind::Xor {
                        inputs: g.inputs.clone(),
                    },
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

/// Union-find representative of `lit` (two-pass with path compression).
///
/// `parent[v]` is the literal that the positive literal `+v` currently maps to;
/// `find(-v) = -find(+v)`. Roots satisfy `parent[v] == ±v` only in the identity form
/// `parent[v] == v` — a merge never installs `parent[v] == -v` (that is the UNSAT case,
/// detected before the union).
fn find_lit(parent: &mut [i32], lit: i32) -> i32 {
    let mut l = lit;
    let root = loop {
        let v = l.unsigned_abs() as usize;
        let p = if l > 0 { parent[v] } else { -parent[v] };
        if p == l {
            break l;
        }
        l = p;
    };
    // Compress the walked path so long chains stay cheap.
    let mut l = lit;
    while l != root {
        let v = l.unsigned_abs() as usize;
        let next = if l > 0 { parent[v] } else { -parent[v] };
        parent[v] = if l > 0 { root } else { -root };
        l = next;
    }
    root
}

/// Outcome of renormalizing one gate through the current representative map.
enum Renormalized {
    /// Gate dropped (degenerate after substitution). Always sound — only forgoes work.
    Dead,
    /// Gate collapsed to a direct equivalence `input ≡ out` (e.g. `x = AND(a, a)`).
    /// For XOR the `cancelled` vars removed by `a ⊕ a` cancellation THIS pass are
    /// returned alongside so the driver can extend the merge's proof-chain
    /// enumeration (see the XOR cancellation soundness note in `find_merges_closure`).
    Collapsed(Merge, Vec<i32>),
    /// Live gate with its canonical hash key `(tag, inputs)` and canonical output
    /// literal. `cancelled` lists the positive vars removed by XOR cancellation THIS
    /// pass (always empty for AND/ITE).
    Keyed {
        tag: u8,
        inputs: Vec<i32>,
        out: i32,
        cancelled: Vec<i32>,
    },
}

/// Map a gate through `parent` and renormalize it to canonical form.
///
/// Substituting representatives can create duplicate inputs, XOR cancellations, or
/// self-references; each is either collapsed to a direct merge (when the equivalence is
/// RUP from the gate clauses plus the already-recorded equivalence binaries) or the gate
/// is conservatively dropped.
fn renormalize_gate(parent: &mut [i32], g: &Gate) -> Renormalized {
    match g.kind {
        GateKind::And => {
            let out = find_lit(parent, g.out);
            let mut inputs: Vec<i32> = g.inputs.iter().map(|&l| find_lit(parent, l)).collect();
            inputs.sort_unstable();
            inputs.dedup();
            let ov = out.unsigned_abs();
            if inputs.iter().any(|&l| l.unsigned_abs() == ov) {
                return Renormalized::Dead;
            }
            if inputs.len() == 1 {
                // x = AND(a): output equivalent to the sole input. RUP from the original
                // gate clauses plus the input-equivalence binaries recorded earlier.
                return Renormalized::Collapsed(
                    Merge {
                        p: inputs[0],
                        q: out,
                        kind: MergeKind::And,
                    },
                    Vec::new(),
                );
            }
            if inputs.is_empty() {
                return Renormalized::Dead;
            }
            Renormalized::Keyed {
                tag: 0,
                inputs,
                out,
                cancelled: Vec::new(),
            }
        }
        GateKind::Ite => {
            let mut rhs = [
                find_lit(parent, g.inputs[0]),
                find_lit(parent, g.inputs[1]),
                find_lit(parent, g.inputs[2]),
            ];
            let negate = normalize_ite(&mut rhs);
            let mut out = find_lit(parent, g.out);
            if negate {
                out = -out;
            }
            let [c, t, e] = rhs;
            let (cv, tv, ev, ov) = (
                c.unsigned_abs(),
                t.unsigned_abs(),
                e.unsigned_abs(),
                out.unsigned_abs(),
            );
            if t == e {
                // x = ITE(c, t, t) ≡ t. The ITE chain over `c` stays RUP through the
                // recorded input-equivalence binaries.
                if cv == tv || cv == ov || tv == ov {
                    return Renormalized::Dead;
                }
                return Renormalized::Collapsed(
                    Merge {
                        p: t,
                        q: out,
                        kind: MergeKind::Ite { cond: c },
                    },
                    Vec::new(),
                );
            }
            // Any shared variable after substitution: drop (extraction guaranteed four
            // distinct variables; the proof chains rely on that shape).
            if cv == tv || cv == ev || tv == ev || ov == cv || ov == tv || ov == ev {
                return Renormalized::Dead;
            }
            Renormalized::Keyed {
                tag: 1,
                inputs: vec![c, t, e],
                out,
                cancelled: Vec::new(),
            }
        }
        GateKind::Xor => {
            // Inputs are positive variable-literals with the parity folded into the output
            // sign. A representative with negative polarity flips the output; equal pairs
            // cancel (a ⊕ a = 0).
            let mut out = find_lit(parent, g.out);
            let mut inputs: Vec<i32> = Vec::with_capacity(g.inputs.len());
            for &l in &g.inputs {
                let mut r = find_lit(parent, l);
                if r < 0 {
                    out = -out;
                    r = -r;
                }
                inputs.push(r);
            }
            inputs.sort_unstable();
            // Split into surviving inputs and cancelled vars (`a ⊕ a` pairs). The
            // cancelled vars are REMOVED from the hash key (parity is unaffected) but
            // must stay in the merge proof chain: the gate's original CNF still ranges
            // over them, so the equivalence binaries are only unit-derivable after a
            // case split on each cancelled var. Dropping them from the emitted ladder
            // produced non-RUP lemmas (oski15a01b40s drat-trim rejection, 2026-07-14).
            let mut survived: Vec<i32> = Vec::with_capacity(inputs.len());
            let mut cancelled: Vec<i32> = Vec::new();
            let mut i = 0;
            while i < inputs.len() {
                if i + 1 < inputs.len() && inputs[i] == inputs[i + 1] {
                    cancelled.push(inputs[i]);
                    i += 2; // a ⊕ a cancels
                } else {
                    survived.push(inputs[i]);
                    i += 1;
                }
            }
            let inputs = survived;
            let ov = out.unsigned_abs() as i32;
            if inputs.iter().any(|&l| l == ov) {
                return Renormalized::Dead;
            }
            if inputs.is_empty() {
                return Renormalized::Dead;
            }
            if inputs.len() == 1 {
                // out ≡ sole surviving input; the chain enumerates the cancelled vars
                // (the caller extends it with this gate's previously-accumulated ones).
                return Renormalized::Collapsed(
                    Merge {
                        p: inputs[0],
                        q: out,
                        kind: MergeKind::Xor {
                            inputs: cancelled.clone(),
                        },
                    },
                    cancelled,
                );
            }
            Renormalized::Keyed {
                tag: 2,
                inputs,
                out,
                cancelled,
            }
        }
    }
}

/// Worklist congruence closure (`SAT_CONGRUENCE_WORKLIST`) — the kissat `congruence.c`
/// cascade ported onto [`find_merges`]'s Plan interface.
///
/// Where [`find_merges`] performs a single syntactic hashing pass (relying on the caller
/// to substitute the formula and re-extract gates to observe cascaded merges), this
/// variant keeps the gates in memory and cascades internally: every merge `p ≡ q` unions
/// the literal classes, and every gate mentioning `q`'s variable is renormalized through
/// the representative map, rehashed, and re-matched — which can schedule further merges
/// (kissat congruence.c: `rewrite_gates` over per-literal occurrence lists + a FIFO
/// worklist). One call reaches the full congruence fixpoint over the given gate set
/// without touching the formula.
///
/// Proof-order contract: `Plan::merges` is in discovery order, and each merge's
/// equivalence binaries are RUP from the gates' defining clauses **plus the equivalence
/// binaries of the earlier merges** (unit propagation routes through them). The driver
/// must emit the merges in order, and — unlike the [`find_merges`] driver path — must
/// emit them **before** the [`Plan::unsat`] refutation, which may be a cascaded
/// self-negation that depends on them.
pub(crate) fn find_merges_closure(num_vars: usize, gates_in: Vec<Gate>) -> Plan {
    let mut parent: Vec<i32> = (0..=num_vars as i32).collect();
    let mut merges: Vec<Merge> = Vec::new();
    let mut pending: VecDeque<Merge> = VecDeque::new();
    // var -> indices of live gates with that variable among their (current) inputs.
    let mut occ: Vec<Vec<u32>> = vec![Vec::new(); num_vars + 1];
    // key -> (representative output, rep gate's accumulated cancelled vars at insert).
    // Reserved to the gate count: the table takes ~1 entry per gate (1.3M on ibm)
    // and is never iterated, so pre-sizing only removes growth rehashes.
    let mut table: HashMap<(u8, Vec<i32>), (i32, Vec<i32>)> = HashMap::default();
    table.reserve(gates_in.len());
    let mut gates: Vec<Option<Gate>> = gates_in.into_iter().map(Some).collect();
    // Per-gate XOR-cancelled vars accumulated across renormalization passes: the stored
    // canonical `g.inputs` forgets cancelled vars, but the gate's ORIGINAL clauses (the
    // proof-chain support) still range over them, so every merge involving this gate
    // must keep enumerating them in its parity ladder.
    let mut acc_cancelled: Vec<Vec<i32>> = vec![Vec::new(); gates.len()];

    // Sorted, deduped chain-variable union for an XOR merge proof ladder.
    fn xor_chain(key_inputs: &[i32], a: &[i32], b: &[i32]) -> Vec<i32> {
        let mut chain: Vec<i32> = Vec::with_capacity(key_inputs.len() + a.len() + b.len());
        chain.extend_from_slice(key_inputs);
        chain.extend_from_slice(a);
        chain.extend_from_slice(b);
        chain.sort_unstable();
        chain.dedup();
        chain
    }

    // Renormalize gate `idx` and register it. `register_inputs`: on the initial pass every
    // input variable is registered; on reprocessing only the variable that just changed
    // representative maps to a new one (`new_var`), the others are already registered.
    #[allow(clippy::too_many_arguments)]
    fn process_gate(
        idx: u32,
        register_all: bool,
        parent: &mut Vec<i32>,
        gates: &mut Vec<Option<Gate>>,
        occ: &mut Vec<Vec<u32>>,
        table: &mut HashMap<(u8, Vec<i32>), (i32, Vec<i32>)>,
        pending: &mut VecDeque<Merge>,
        acc_cancelled: &mut [Vec<i32>],
    ) {
        let Some(g) = gates[idx as usize].as_ref() else {
            return;
        };
        match renormalize_gate(parent, g) {
            Renormalized::Dead => {
                gates[idx as usize] = None;
            }
            Renormalized::Collapsed(mut m, cancelled) => {
                gates[idx as usize] = None;
                // XOR collapse: extend the ladder with this gate's full cancellation
                // history (this pass + earlier passes).
                if let MergeKind::Xor { inputs } = &mut m.kind {
                    let _ = cancelled; // this pass's vars are already in `inputs`
                    *inputs = xor_chain(inputs, &acc_cancelled[idx as usize], &[]);
                }
                pending.push_back(m);
            }
            Renormalized::Keyed {
                tag,
                inputs,
                out,
                cancelled,
            } => {
                // Keep the canonical form so later reprocessing starts from it (for ITE
                // the output flip is already folded into `out`), and accumulate the
                // cancellation history.
                {
                    let g = gates[idx as usize].as_mut().unwrap();
                    g.out = out;
                    // Skip the write-back clone when renormalization changed nothing —
                    // on fixpoint rounds (~all gates) this kills ~1 Vec clone per gate
                    // per round. Equality compare is cheap and the stored value is
                    // identical either way (behavior-preserving). SAT_ELIM_SCRATCH=off
                    // replays the unconditional legacy clone (A/B baseline arm).
                    if legacy_clone_mode() || g.inputs != inputs {
                        g.inputs = inputs.clone();
                    }
                }
                if !cancelled.is_empty() {
                    let acc = &mut acc_cancelled[idx as usize];
                    acc.extend_from_slice(&cancelled);
                    acc.sort_unstable();
                    acc.dedup();
                }
                if register_all {
                    for &l in &inputs {
                        let v = l.unsigned_abs() as usize;
                        occ[v].push(idx);
                    }
                }
                let key = (tag, inputs);
                match table.get(&key) {
                    None => {
                        let snapshot = acc_cancelled[idx as usize].clone();
                        table.insert(key, (out, snapshot));
                    }
                    Some((rep, rep_cancelled)) => {
                        let rep = *rep;
                        if rep != out {
                            let kind = match tag {
                                0 => MergeKind::And,
                                1 => MergeKind::Ite { cond: key.1[0] },
                                _ => MergeKind::Xor {
                                    inputs: xor_chain(
                                        &key.1,
                                        rep_cancelled,
                                        &acc_cancelled[idx as usize],
                                    ),
                                },
                            };
                            pending.push_back(Merge { p: rep, q: out, kind });
                        }
                    }
                }
            }
        }
    }

    for idx in 0..gates.len() as u32 {
        process_gate(
            idx,
            true,
            &mut parent,
            &mut gates,
            &mut occ,
            &mut table,
            &mut pending,
            &mut acc_cancelled,
        );
    }

    while let Some(m) = pending.pop_front() {
        let p = find_lit(&mut parent, m.p);
        let q = find_lit(&mut parent, m.q);
        if p == q {
            continue;
        }
        // Degenerate ITE merges (condition variable shared with an output) are dropped,
        // matching find_merges: their proof chain over `cond` collapses.
        if let MergeKind::Ite { cond } = &m.kind {
            let cv = cond.unsigned_abs();
            if cv == p.unsigned_abs() || cv == q.unsigned_abs() {
                continue;
            }
        }
        let mut kind = m.kind.clone();
        if let MergeKind::Xor { inputs } = &mut kind {
            // Chain vars equal to var(p)/var(q) are pinned by the RUP assumption
            // itself — enumerating them would only produce tautologies (which the
            // emitter skips, leaving holes in the ladder). Filter them out.
            let (pv, qv) = (p.unsigned_abs() as i32, q.unsigned_abs() as i32);
            inputs.retain(|&v| v != pv && v != qv);
            // Ladder size is 2^|chain| clauses per level; XOR extraction is arity <= 4
            // and cancellations are rare, so this cap only guards pathological unions.
            if inputs.len() > 12 {
                continue;
            }
        }
        let merge = Merge { p, q, kind };
        if p == -q {
            return Plan {
                merges,
                unsat: Some(merge),
            };
        }
        merges.push(merge);
        // Union: q's class now points at p. p keeps its root, so gates registered under
        // p's variable stay current; only gates mentioning q's variable must reprocess.
        let qv = q.unsigned_abs() as usize;
        parent[qv] = if q > 0 { p } else { -p };
        let touched = std::mem::take(&mut occ[qv]);
        let pv = p.unsigned_abs() as usize;
        for idx in touched {
            if gates[idx as usize].is_none() {
                continue;
            }
            // The reprocessed gate's inputs now mention p's variable; register it there so
            // a future merge of p reprocesses it again. Other input registrations are
            // still current.
            occ[pv].push(idx);
            process_gate(
                idx,
                false,
                &mut parent,
                &mut gates,
                &mut occ,
                &mut table,
                &mut pending,
                &mut acc_cancelled,
            );
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
        let m = &plan.merges[0];
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
        let m = &plan.merges[0];
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

    fn xor_gate(out: i32, mut inputs: Vec<i32>) -> Gate {
        inputs.sort_unstable();
        Gate {
            out,
            kind: GateKind::Xor,
            inputs,
        }
    }

    #[test]
    fn xor_gates_same_inputs_merge_outputs() {
        // p = a⊕b and q = a⊕b ⇒ p ≡ q. Inputs are positive vars {1,2}.
        let gates = vec![xor_gate(3, vec![1, 2]), xor_gate(4, vec![2, 1])];
        let plan = find_merges(&gates);
        assert!(plan.unsat.is_none());
        assert_eq!(plan.merges.len(), 1);
        let m = &plan.merges[0];
        assert_eq!(m.p, 3);
        assert_eq!(m.q, 4);
        assert_eq!(m.kind, MergeKind::Xor { inputs: vec![1, 2] });
    }

    #[test]
    fn xor_gates_different_inputs_do_not_merge() {
        let gates = vec![xor_gate(4, vec![1, 2]), xor_gate(5, vec![1, 3])];
        let plan = find_merges(&gates);
        assert!(plan.merges.is_empty());
        assert!(plan.unsat.is_none());
    }

    #[test]
    fn xor_output_and_negation_is_unsat() {
        // p = a⊕b and ¬p = a⊕b ⇒ p ≡ ¬p ⇒ UNSAT.
        let gates = vec![xor_gate(3, vec![1, 2]), xor_gate(-3, vec![1, 2])];
        let plan = find_merges(&gates);
        let u = plan.unsat.expect("must be UNSAT");
        assert_eq!(u.p, 3);
        assert_eq!(u.q, -3);
    }

    #[test]
    fn xor_does_not_cross_match_and_or_ite() {
        // Same input tuple, different kinds must not merge with XOR.
        let x = xor_gate(4, vec![1, 2]);
        let a = and_gate(5, vec![1, 2]);
        let plan = find_merges(&[x, a]);
        assert!(plan.merges.is_empty());
        assert!(plan.unsat.is_none());
    }

    // ---- worklist closure (find_merges_closure) ----

    #[test]
    fn closure_matches_single_pass_on_non_cascading_input() {
        let gates = vec![and_gate(3, vec![1, 2]), and_gate(4, vec![2, 1])];
        let plan = find_merges_closure(4, gates);
        assert!(plan.unsat.is_none());
        assert_eq!(plan.merges.len(), 1);
        assert_eq!((plan.merges[0].p, plan.merges[0].q), (3, 4));
        assert_eq!(plan.merges[0].kind, MergeKind::And);
    }

    #[test]
    fn closure_cascades_and_merges_one_call() {
        // Layer 1: x1 = AND(1,2), x2 = AND(1,2) ⇒ x1 ≡ x2.
        // Layer 2: y1 = AND(x1,5), y2 = AND(x2,5) — congruent only AFTER x1 ≡ x2.
        // The single-pass matcher needs a substitute→re-extract round to see y1 ≡ y2;
        // the closure must find both merges in one call, layer 1 first.
        let gates = vec![
            and_gate(3, vec![1, 2]),
            and_gate(4, vec![1, 2]),
            and_gate(6, vec![3, 5]),
            and_gate(7, vec![4, 5]),
        ];
        let single = find_merges(&gates);
        assert_eq!(single.merges.len(), 1, "single pass sees only layer 1");
        let plan = find_merges_closure(7, gates);
        assert!(plan.unsat.is_none());
        assert_eq!(plan.merges.len(), 2, "closure cascades to layer 2");
        assert_eq!((plan.merges[0].p, plan.merges[0].q), (3, 4));
        assert_eq!((plan.merges[1].p, plan.merges[1].q), (6, 7));
    }

    #[test]
    fn closure_cascades_through_negated_inputs() {
        // x1 = AND(1,2) ⇒ out 3; x2 = AND(1,2) ⇒ out 4 ⇒ 3 ≡ 4.
        // y1 = AND(-3,5) out 6; y2 = AND(-4,5) out 7: after 3 ≡ 4 the mapped input of
        // y2 is -3, so y1 ≡ y2 must cascade with correct polarity handling.
        let gates = vec![
            and_gate(3, vec![1, 2]),
            and_gate(4, vec![1, 2]),
            and_gate(6, vec![-3, 5]),
            and_gate(7, vec![-4, 5]),
        ];
        let plan = find_merges_closure(7, gates);
        assert!(plan.unsat.is_none());
        assert_eq!(plan.merges.len(), 2);
        assert_eq!((plan.merges[1].p, plan.merges[1].q), (6, 7));
    }

    #[test]
    fn closure_xor_cancellation_collapses_to_input_merge() {
        // a ≡ b via matched AND gates (outputs 3 and 4), then z = XOR(3,4,5): after the
        // merge the XOR inputs cancel pairwise, so z ≡ 5 directly.
        let gates = vec![
            and_gate(3, vec![1, 2]),
            and_gate(4, vec![1, 2]),
            xor_gate(6, vec![3, 4, 5]),
        ];
        let plan = find_merges_closure(6, gates);
        assert!(plan.unsat.is_none());
        assert_eq!(plan.merges.len(), 2);
        assert_eq!((plan.merges[0].p, plan.merges[0].q), (3, 4));
        let m = &plan.merges[1];
        assert_eq!((m.p, m.q), (5, 6));
        // The proof-chain enumeration must cover the CANCELLED var (3): the gate's
        // original clauses still range over it, so `z ≡ 5` is only unit-derivable
        // after a case split on it. (The old contract carried the surviving input 5
        // here — i.e. var(p) itself — which emitted only tautologies and produced
        // non-RUP equivalence binaries; oski15a01b40s drat-trim rejection, 2026-07-14.)
        assert_eq!(m.kind, MergeKind::Xor { inputs: vec![3] });
    }

    #[test]
    fn closure_xor_cancellation_extends_keyed_merge_chain() {
        // a ≡ b via matched ANDs (3, 4); w = XOR(3,4,5,6) then cancels to XOR(5,6),
        // colliding with v = XOR(5,6). The merge chain must enumerate the cancelled
        // var 3 alongside the shared key inputs 5,6 — w's original clauses still
        // range over 3 and 4, so the equivalence is only RUP after splitting on 3.
        let gates = vec![
            and_gate(3, vec![1, 2]),
            and_gate(4, vec![1, 2]),
            xor_gate(7, vec![3, 4, 5, 6]),
            xor_gate(8, vec![5, 6]),
        ];
        let plan = find_merges_closure(8, gates);
        assert!(plan.unsat.is_none());
        assert_eq!(plan.merges.len(), 2);
        assert_eq!((plan.merges[0].p, plan.merges[0].q), (3, 4));
        let m = &plan.merges[1];
        assert_eq!((m.p, m.q), (8, 7));
        assert_eq!(
            m.kind,
            MergeKind::Xor {
                inputs: vec![3, 5, 6]
            }
        );
    }

    #[test]
    fn closure_unary_and_collapse_merges_output_with_input() {
        // x = AND(3,4) with 3 ≡ 4 (via layer-1 AND congruence) ⇒ x ≡ 3.
        let gates = vec![
            and_gate(3, vec![1, 2]),
            and_gate(4, vec![1, 2]),
            and_gate(5, vec![3, 4]),
        ];
        let plan = find_merges_closure(5, gates);
        assert!(plan.unsat.is_none());
        assert_eq!(plan.merges.len(), 2);
        assert_eq!((plan.merges[1].p, plan.merges[1].q), (3, 5));
        assert_eq!(plan.merges[1].kind, MergeKind::And);
    }

    #[test]
    fn closure_cascaded_self_negation_reports_unsat_after_merges() {
        // Layer 1 merges 3 ≡ 4; layer 2 gates give 6 ≡ -6 only through that merge.
        // The prior merges MUST be present in Plan::merges (the refutation chain is
        // RUP only after their binaries are emitted).
        let gates = vec![
            and_gate(3, vec![1, 2]),
            and_gate(4, vec![1, 2]),
            and_gate(6, vec![3, 5]),
            and_gate(-6, vec![4, 5]),
        ];
        let plan = find_merges_closure(6, gates);
        let u = plan.unsat.expect("cascaded self-negation must be UNSAT");
        assert_eq!(plan.merges.len(), 1, "layer-1 merge precedes the refutation");
        assert_eq!((plan.merges[0].p, plan.merges[0].q), (3, 4));
        assert_eq!(u.p.unsigned_abs(), 6);
        assert_eq!(u.q, -u.p);
    }

    #[test]
    fn closure_ite_then_else_collapse_merges_with_branch() {
        // x = ITE(c, t, e) with t ≡ e via layer-1 AND congruence ⇒ x ≡ t.
        let gates = vec![
            and_gate(4, vec![1, 2]),
            and_gate(5, vec![1, 2]),
            ite_gate(7, 6, 4, 5),
        ];
        let plan = find_merges_closure(7, gates);
        assert!(plan.unsat.is_none());
        assert_eq!(plan.merges.len(), 2);
        let m = &plan.merges[1];
        assert_eq!((m.p, m.q), (4, 7));
        assert_eq!(m.kind, MergeKind::Ite { cond: 6 });
    }

    #[test]
    fn closure_ite_cascade_via_condition_rewrite() {
        // Two ITE gates whose conditions become equal only after an AND merge.
        let gates = vec![
            and_gate(4, vec![1, 2]),
            and_gate(5, vec![1, 2]),
            ite_gate(8, 4, 6, 7),
            ite_gate(9, 5, 6, 7),
        ];
        let single = find_merges(&gates);
        assert_eq!(single.merges.len(), 1);
        let plan = find_merges_closure(9, gates);
        assert!(plan.unsat.is_none());
        assert_eq!(plan.merges.len(), 2);
        assert_eq!((plan.merges[1].p, plan.merges[1].q), (8, 9));
        assert_eq!(plan.merges[1].kind, MergeKind::Ite { cond: 4 });
    }

    #[test]
    fn closure_xor_sign_flip_through_negative_representative() {
        // -3 ≡ 4 (outputs of AND gates on the same inputs with opposite polarity
        // conventions): p = XOR over var 3 must match q = XOR over var 4 with the
        // output sign flipped by the negative representative.
        let gates = vec![
            and_gate(-3, vec![1, 2]),
            and_gate(4, vec![1, 2]),
            xor_gate(6, vec![3, 5]),
            xor_gate(7, vec![4, 5]),
        ];
        let plan = find_merges_closure(7, gates);
        assert!(plan.unsat.is_none());
        // merge 1: -3 ≡ 4. merge 2: XOR(3,5) vs XOR(4,5): with 4 ↦ -3 the second gate
        // is XOR(3,5) on the flipped output, so 6 ≡ -7.
        assert_eq!(plan.merges.len(), 2);
        let m = &plan.merges[1];
        assert_eq!(m.p.unsigned_abs().max(m.q.unsigned_abs()), 7);
        assert_eq!(
            (m.p, m.q),
            (6, -7),
            "negative representative must flip the XOR output"
        );
    }
}
