//! Equivalent-literal substitution (ELS): literal equivalences from the binary
//! implication graph.
//!
//! Phase A of the inprocessing-pipeline effort (bead SAT-playground-otd). Circuit
//! miters (BubbleVsPancakeSort, VexRiscv) are binary-heavy: their binary implication
//! graph has large non-trivial strongly-connected components (SCCs), and every literal
//! in one SCC is logically equivalent. Collapsing each class to a single representative
//! removes variables and unlocks further bounded-variable elimination (BVE).
//!
//! This module is the *pure* core: from the set of binary clauses over the currently
//! active variables it computes, for each variable, a representative literal (or detects
//! that the formula is UNSAT because some literal is equivalent to its own negation). The
//! solver-side substitution (rewriting clauses, emitting the DRAT proof, maintaining watch
//! lists and model-reconstruction witnesses) lives in `main.rs`.
//!
//! ## Graph
//! A binary clause `(a ∨ b)` is the pair of implications `¬a → b` and `¬b → a`. Over the
//! `2·num_vars` literal nodes these edges form the binary implication graph. Its SCCs are
//! the equivalence classes: if `x` and `y` are in one SCC there are implication paths
//! `x → … → y` and `y → … → x`, i.e. `x ≡ y`.
//!
//! ## Representative choice (negation symmetry)
//! Nodes are indexed by [`lit_to_index`]: variable `v` (1-based) maps its positive literal
//! to `(v-1)*2` and its negative literal to `(v-1)*2 + 1`, so `index(¬l) == index(l) ^ 1`.
//! The representative of an SCC is the member with the smallest node index. Because the
//! implication graph is skew-symmetric (`a → b` implies `¬b → ¬a`), SCCs come in dual
//! pairs `S` and `¬S`, and for this index encoding `min(¬S) == ¬(min(S))` whenever no
//! variable has both polarities in `S`. That last condition is exactly "not UNSAT", so on
//! a satisfiable-so-far formula the representative map respects negation:
//! `repr(¬l) == ¬repr(l)`. The caller relies on this to substitute negative occurrences.

use crate::lit::lit_to_index;

/// Convert a node index back to a signed 1-based literal (inverse of [`lit_to_index`]).
#[inline]
fn node_to_lit(node: u32) -> i32 {
    let var = (node / 2) as i32 + 1;
    if node & 1 == 0 {
        var
    } else {
        -var
    }
}

/// Outcome of the SCC / representative computation.
pub(crate) enum ElsResult {
    /// The formula is UNSAT: literal `lit` and `-lit` share an SCC (`lit ≡ ¬lit`).
    Unsat(i32),
    /// `repr[v]` is the signed representative literal of the *positive* literal of
    /// variable `v` (1-based; index 0 unused and set to 0). `repr[v] == v` means `v` is
    /// its own representative (not substituted). The representative of the negative
    /// literal `¬v` is `-repr[v]`.
    Reprs(Vec<i32>),
}

/// Compute equivalence-class representatives from the binary implication graph.
///
/// `num_vars` is the (1-based) variable count. `active[v]` must be `true` only for
/// variables that participate — the caller passes the unassigned, non-eliminated
/// variables. `binaries` lists the live binary clauses `(a, b)` over active variables
/// (the caller filters out satisfied/assigned ones); each contributes edges `¬a → b` and
/// `¬b → a`.
///
/// Returns [`ElsResult::Unsat`] if any literal is equivalent to its negation, otherwise
/// the per-variable representative map.
pub(crate) fn compute_representatives(
    num_vars: usize,
    active: &[bool],
    binaries: &[(i32, i32)],
) -> ElsResult {
    let n = num_vars.saturating_mul(2);
    if n == 0 {
        return ElsResult::Reprs(vec![0]);
    }

    // Adjacency over literal nodes. Edge x -> y means the implication x → y.
    let mut adjacency: Vec<Vec<u32>> = vec![Vec::new(); n];
    for &(a, b) in binaries {
        let va = a.unsigned_abs() as usize;
        let vb = b.unsigned_abs() as usize;
        if va == 0 || vb == 0 || va > num_vars || vb > num_vars {
            continue;
        }
        if !active[va] || !active[vb] {
            continue;
        }
        // (a ∨ b): ¬a → b and ¬b → a.
        adjacency[lit_to_index(-a)].push(lit_to_index(b) as u32);
        adjacency[lit_to_index(-b)].push(lit_to_index(a) as u32);
    }

    // Iterative Tarjan. `comp_min[node]` becomes the smallest node index in `node`'s SCC.
    const UNVISITED: u32 = u32::MAX;
    let mut disc = vec![UNVISITED; n];
    let mut low = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut comp_min = vec![UNVISITED; n];
    let mut scc_stack: Vec<u32> = Vec::new();
    let mut counter: u32 = 0;
    // Explicit DFS work stack of (node, next-child-index).
    let mut work: Vec<(u32, usize)> = Vec::new();
    let mut members: Vec<u32> = Vec::new();

    for start in 0..n {
        let var = start / 2 + 1;
        if !active[var] || disc[start] != UNVISITED {
            continue;
        }
        work.push((start as u32, 0));
        while let Some(&(v, ci)) = work.last() {
            let vu = v as usize;
            if ci == 0 {
                disc[vu] = counter;
                low[vu] = counter;
                counter += 1;
                scc_stack.push(v);
                on_stack[vu] = true;
            }
            if ci < adjacency[vu].len() {
                let w = adjacency[vu][ci];
                work.last_mut().unwrap().1 += 1;
                let wu = w as usize;
                if disc[wu] == UNVISITED {
                    work.push((w, 0));
                } else if on_stack[wu] {
                    low[vu] = low[vu].min(disc[wu]);
                }
            } else {
                if low[vu] == disc[vu] {
                    members.clear();
                    loop {
                        let w = scc_stack.pop().unwrap();
                        on_stack[w as usize] = false;
                        members.push(w);
                        if w == v {
                            break;
                        }
                    }
                    let min_node = *members.iter().min().unwrap();
                    for &w in &members {
                        comp_min[w as usize] = min_node;
                    }
                }
                work.pop();
                if let Some(&(parent, _)) = work.last() {
                    low[parent as usize] = low[parent as usize].min(low[vu]);
                }
            }
        }
    }

    // UNSAT iff a variable's two polarity nodes landed in one SCC.
    let mut repr = vec![0i32; num_vars + 1];
    for v in 1..=num_vars {
        if !active[v] {
            repr[v] = v as i32;
            continue;
        }
        let pos = lit_to_index(v as i32);
        let neg = lit_to_index(-(v as i32));
        if comp_min[pos] != UNVISITED && comp_min[pos] == comp_min[neg] {
            return ElsResult::Unsat(v as i32);
        }
        repr[v] = if comp_min[pos] == UNVISITED {
            v as i32
        } else {
            node_to_lit(comp_min[pos])
        };
    }

    ElsResult::Reprs(repr)
}

/// Flat-CSR variant of [`compute_representatives`] (SAT_ROUND_DIET): identical
/// results, but the implication graph is three flat arrays instead of a
/// `Vec<Vec<u32>>` with `2·num_vars` headers and per-node heap blocks. Per-node
/// edge order equals the legacy push order (both passes scan `binaries` in
/// order), so the Tarjan traversal — and therefore every SCC representative —
/// is byte-identical.
pub(crate) fn compute_representatives_csr(
    num_vars: usize,
    active: &[bool],
    binaries: &[(i32, i32)],
) -> ElsResult {
    let n = num_vars.saturating_mul(2);
    if n == 0 {
        return ElsResult::Reprs(vec![0]);
    }

    // CSR adjacency over literal nodes. Edge x -> y means the implication x → y.
    let edge_endpoints = |a: i32, b: i32| -> Option<()> {
        let va = a.unsigned_abs() as usize;
        let vb = b.unsigned_abs() as usize;
        if va == 0 || vb == 0 || va > num_vars || vb > num_vars {
            return None;
        }
        if !active[va] || !active[vb] {
            return None;
        }
        Some(())
    };
    let mut start = vec![0u32; n + 1];
    for &(a, b) in binaries {
        if edge_endpoints(a, b).is_none() {
            continue;
        }
        start[lit_to_index(-a) + 1] += 1;
        start[lit_to_index(-b) + 1] += 1;
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    let mut edges = vec![0u32; start[n] as usize];
    let mut cursor = start.clone();
    for &(a, b) in binaries {
        if edge_endpoints(a, b).is_none() {
            continue;
        }
        // (a ∨ b): ¬a → b and ¬b → a.
        let ia = lit_to_index(-a);
        let ib = lit_to_index(-b);
        edges[cursor[ia] as usize] = lit_to_index(b) as u32;
        cursor[ia] += 1;
        edges[cursor[ib] as usize] = lit_to_index(a) as u32;
        cursor[ib] += 1;
    }

    // Iterative Tarjan. `comp_min[node]` becomes the smallest node index in `node`'s SCC.
    const UNVISITED: u32 = u32::MAX;
    let mut disc = vec![UNVISITED; n];
    let mut low = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut comp_min = vec![UNVISITED; n];
    let mut scc_stack: Vec<u32> = Vec::new();
    let mut counter: u32 = 0;
    // Explicit DFS work stack of (node, next-child-index).
    let mut work: Vec<(u32, usize)> = Vec::new();
    let mut members: Vec<u32> = Vec::new();

    for start_node in 0..n {
        let var = start_node / 2 + 1;
        if !active[var] || disc[start_node] != UNVISITED {
            continue;
        }
        work.push((start_node as u32, 0));
        while let Some(&(v, ci)) = work.last() {
            let vu = v as usize;
            if ci == 0 {
                disc[vu] = counter;
                low[vu] = counter;
                counter += 1;
                scc_stack.push(v);
                on_stack[vu] = true;
            }
            let deg = (start[vu + 1] - start[vu]) as usize;
            if ci < deg {
                let w = edges[start[vu] as usize + ci];
                work.last_mut().unwrap().1 += 1;
                let wu = w as usize;
                if disc[wu] == UNVISITED {
                    work.push((w, 0));
                } else if on_stack[wu] {
                    low[vu] = low[vu].min(disc[wu]);
                }
            } else {
                if low[vu] == disc[vu] {
                    members.clear();
                    loop {
                        let w = scc_stack.pop().unwrap();
                        on_stack[w as usize] = false;
                        members.push(w);
                        if w == v {
                            break;
                        }
                    }
                    let min_node = *members.iter().min().unwrap();
                    for &w in &members {
                        comp_min[w as usize] = min_node;
                    }
                }
                work.pop();
                if let Some(&(parent, _)) = work.last() {
                    low[parent as usize] = low[parent as usize].min(low[vu]);
                }
            }
        }
    }

    // UNSAT iff a variable's two polarity nodes landed in one SCC.
    let mut repr = vec![0i32; num_vars + 1];
    for v in 1..=num_vars {
        if !active[v] {
            repr[v] = v as i32;
            continue;
        }
        let pos = lit_to_index(v as i32);
        let neg = lit_to_index(-(v as i32));
        if comp_min[pos] != UNVISITED && comp_min[pos] == comp_min[neg] {
            return ElsResult::Unsat(v as i32);
        }
        repr[v] = if comp_min[pos] == UNVISITED {
            v as i32
        } else {
            node_to_lit(comp_min[pos])
        };
    }

    ElsResult::Reprs(repr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_active(num_vars: usize) -> Vec<bool> {
        vec![true; num_vars + 1]
    }

    /// Signed representative of an arbitrary literal, given the positive-literal map.
    fn repr_of(repr: &[i32], lit: i32) -> i32 {
        let v = lit.unsigned_abs() as usize;
        if lit > 0 {
            repr[v]
        } else {
            -repr[v]
        }
    }

    #[test]
    fn no_binaries_all_self_representative() {
        let r = compute_representatives(3, &all_active(3), &[]);
        match r {
            ElsResult::Reprs(repr) => {
                assert_eq!(repr[1], 1);
                assert_eq!(repr[2], 2);
                assert_eq!(repr[3], 3);
            }
            ElsResult::Unsat(_) => panic!("unexpected UNSAT"),
        }
    }

    #[test]
    fn simple_equivalence_pair() {
        // 1 ≡ 2 via (¬1 ∨ 2) and (1 ∨ ¬2): edges 1→2 and 2→1.
        let bins = vec![(-1, 2), (1, -2)];
        match compute_representatives(2, &all_active(2), &bins) {
            ElsResult::Reprs(repr) => {
                // Representative is the smaller-index literal: positive 1 (index 0).
                assert_eq!(repr[1], 1);
                assert_eq!(repr[2], 1, "2 must collapse to 1");
                // Negation symmetry: repr(¬2) == ¬repr(2) == -1.
                assert_eq!(repr_of(&repr, -2), -1);
            }
            ElsResult::Unsat(_) => panic!("unexpected UNSAT"),
        }
    }

    #[test]
    fn equivalence_chain_collapses_to_one_representative() {
        // Chain 1 ≡ 2 ≡ 3 ≡ 4 as a directed cycle 1→2→3→4→1 (and the dual for negatives).
        // Edge x→y comes from binary (¬x ∨ y).
        let bins = vec![(-1, 2), (-2, 3), (-3, 4), (-4, 1)];
        match compute_representatives(4, &all_active(4), &bins) {
            ElsResult::Reprs(repr) => {
                assert_eq!(repr[1], 1);
                assert_eq!(repr[2], 1);
                assert_eq!(repr[3], 1);
                assert_eq!(repr[4], 1);
                // Idempotence: repr of a representative is itself.
                assert_eq!(repr_of(&repr, repr[3]), repr[3]);
            }
            ElsResult::Unsat(_) => panic!("unexpected UNSAT"),
        }
    }

    #[test]
    fn literal_equivalent_to_its_negation_is_unsat() {
        // A genuine SCC over {+1,+2,-1,-2} containing both +1 and -1 ⇒ 1 ≡ ¬1 ⇒ UNSAT.
        // Cycle 1→2→-1→-2→1: edge x→y from binary (¬x ∨ y).
        //   1→2  : (¬1 ∨ 2)  = (-1, 2)
        //   2→-1 : (¬2 ∨ ¬1) = (-2, -1)
        //   -1→-2: (1 ∨ ¬2)  = (1, -2)
        //   -2→1 : (2 ∨ 1)   = (2, 1)
        let bins = vec![(-1, 2), (-2, -1), (1, -2), (2, 1)];
        match compute_representatives(2, &all_active(2), &bins) {
            ElsResult::Unsat(lit) => {
                assert!(lit.unsigned_abs() == 1 || lit.unsigned_abs() == 2);
            }
            ElsResult::Reprs(_) => panic!("expected UNSAT for x ≡ ¬x"),
        }
    }

    /// The CSR variant must agree with the legacy Vec-of-Vec implementation on
    /// every scenario (including UNSAT detection and inactive filtering).
    #[test]
    fn csr_variant_matches_legacy() {
        let cases: Vec<(usize, Vec<bool>, Vec<(i32, i32)>)> = vec![
            (3, all_active(3), vec![]),
            (2, all_active(2), vec![(-1, 2), (1, -2)]),
            (4, all_active(4), vec![(-1, 2), (-2, 3), (-3, 4), (-4, 1)]),
            (2, all_active(2), vec![(-1, 2), (-2, -1), (1, -2), (2, 1)]),
            (3, {
                let mut a = all_active(3);
                a[2] = false;
                a
            }, vec![(-1, 3), (1, -3)]),
            // Duplicate edges and self-referential binaries.
            (5, all_active(5), vec![(-1, 2), (-1, 2), (1, -2), (3, 4), (-4, 5), (0, 1), (6, 1)]),
        ];
        for (num_vars, active, bins) in cases {
            let legacy = compute_representatives(num_vars, &active, &bins);
            let csr = compute_representatives_csr(num_vars, &active, &bins);
            match (legacy, csr) {
                (ElsResult::Reprs(a), ElsResult::Reprs(b)) => assert_eq!(a, b),
                (ElsResult::Unsat(a), ElsResult::Unsat(b)) => assert_eq!(a, b),
                _ => panic!("legacy and CSR disagree on outcome kind"),
            }
        }
    }

    #[test]
    fn inactive_variable_is_self_representative() {
        // Variable 2 inactive: even with an equivalence edge it is not collapsed.
        let mut active = all_active(3);
        active[2] = false;
        let bins = vec![(-1, 3), (1, -3)]; // 1 ≡ 3
        match compute_representatives(3, &active, &bins) {
            ElsResult::Reprs(repr) => {
                assert_eq!(repr[2], 2);
                assert_eq!(repr[3], 1);
            }
            ElsResult::Unsat(_) => panic!("unexpected UNSAT"),
        }
    }
}
