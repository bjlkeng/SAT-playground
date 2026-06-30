//! XOR/parity reasoning: extraction of XOR constraints from CNF and Gaussian
//! elimination over GF(2).
//!
//! Targets parity-heavy instances (e.g. Tseitin grid formulas) where CDCL needs
//! exponential resolution but Gaussian elimination detects unsatisfiability in
//! polynomial time.
//!
//! Parity engine (bead SAT-playground-qld), all phases:
//! - `extract_xors`: recover XOR constraints from CNF.
//! - `gaussian_unsat`: detect a `0 = 1` contradiction (cheap, dense bitset).
//! - `gaussian_unsat_with_proof`: refute AND emit a DRAT proof — pure-resolution
//!   Gaussian elimination in min-degree order; each emitted clause is a resolvent of
//!   already-present clauses (RUP), so the proof is drat-trim-verifiable with no RAT
//!   lemmas. Validated on Tseitin grids (tseitin_grid_n12: 1.26M-clause proof,
//!   `s VERIFIED`). The driver (`try_gauss_refute`) buffers the proof and commits it
//!   only on success, so a non-refuting attempt leaves the proof stream untouched.
//!
//! Soundness: the extracted XOR constraints are exactly a subset of the original
//! clauses (each XOR is its 2^(k-1) clausal encoding). If the XOR subsystem alone is
//! inconsistent then the whole formula is UNSAT regardless of any non-XOR clauses, so
//! a `0 = 1` row is a sound UNSAT witness — and the emitted resolution proof certifies
//! it from the original CNF. `SAT_GAUSS` only emits `s UNSATISFIABLE` after a complete
//! proof has been written.

/// One XOR constraint: `x_{vars[0]} ^ x_{vars[1]} ^ ... = rhs`.
///
/// `vars` holds 1-based variable indices, sorted ascending and deduplicated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct XorConstraint {
    pub(crate) vars: Vec<u32>,
    pub(crate) rhs: bool,
}

/// Sign bitmask of a clause relative to a sorted variable set: bit `j` is set iff
/// `vars[j]` occurs negated in `clause`.
fn sign_mask(clause: &[i32], vars: &[u32]) -> u64 {
    let mut mask = 0u64;
    for &lit in clause {
        let v = lit.unsigned_abs();
        if let Ok(idx) = vars.binary_search(&v) {
            if lit < 0 {
                mask |= 1u64 << idx;
            }
        }
    }
    mask
}

/// Extract XOR constraints from a CNF clause list.
///
/// A degree-`k` XOR over variable set `V` is encoded by exactly `2^(k-1)` clauses,
/// all over `V`, all with the same negation-parity, all distinct sign patterns.
/// Even negation-parity encodes `rhs = true` (`x1 ^ ... ^ xk = 1`); odd encodes
/// `rhs = false`. A variable set is only accepted as an XOR when its clause group
/// has exactly `2^(k-1)` members, a single shared parity, and distinct signs.
///
/// Groups of degree above `max_degree` are skipped — their `2^(k-1)` clause count
/// makes a complete group both unlikely and expensive to detect. Clauses with a
/// repeated variable or an embedded `0` literal are ignored.
///
/// Returns the extracted constraints and the (unsorted) clause indices consumed.
pub(crate) fn extract_xors(
    clauses: &[Vec<i32>],
    max_degree: usize,
) -> (Vec<XorConstraint>, Vec<usize>) {
    use std::collections::HashMap;

    // Group clause indices by their sorted variable set.
    let mut groups: HashMap<Vec<u32>, Vec<usize>> = HashMap::new();
    'clauses: for (ci, clause) in clauses.iter().enumerate() {
        if clause.is_empty() || clause.len() > max_degree {
            continue;
        }
        let mut vars: Vec<u32> = Vec::with_capacity(clause.len());
        for &lit in clause {
            if lit == 0 {
                continue 'clauses;
            }
            vars.push(lit.unsigned_abs());
        }
        vars.sort_unstable();
        if vars.windows(2).any(|w| w[0] == w[1]) {
            // Repeated variable: not a clean XOR clause.
            continue;
        }
        groups.entry(vars).or_default().push(ci);
    }

    let mut xors = Vec::new();
    let mut consumed = Vec::new();
    for (vars, members) in groups {
        let k = vars.len();
        if k == 0 || k > max_degree || k > 31 {
            continue;
        }
        let expected = 1usize << (k - 1); // 2^(k-1)
        if members.len() != expected {
            continue;
        }
        // All members share one negation-parity, and their sign patterns are all
        // distinct (=> they are exactly the 2^(k-1) sign combinations of that parity).
        let parity0 = (sign_mask(&clauses[members[0]], &vars).count_ones() & 1) == 1;
        let mut seen: u64 = 0; // bitset over the 2^k possible sign masks (k <= up to 31, but expected <= here small)
        let mut ok = true;
        if k <= 6 {
            // Compact distinctness via a bitset over 2^k mask values.
            for &ci in &members {
                let mask = sign_mask(&clauses[ci], &vars);
                if (mask.count_ones() & 1 == 1) != parity0 {
                    ok = false;
                    break;
                }
                let bit = 1u64 << mask;
                if seen & bit != 0 {
                    ok = false;
                    break;
                }
                seen |= bit;
            }
        } else {
            use std::collections::HashSet;
            let mut set = HashSet::new();
            for &ci in &members {
                let mask = sign_mask(&clauses[ci], &vars);
                if (mask.count_ones() & 1 == 1) != parity0 {
                    ok = false;
                    break;
                }
                if !set.insert(mask) {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        // Even negation-parity (parity0 == false) => rhs = 1 (true).
        let rhs = !parity0;
        for &ci in &members {
            consumed.push(ci);
        }
        xors.push(XorConstraint { vars, rhs });
    }

    (xors, consumed)
}

/// Gaussian elimination over GF(2). Returns `true` iff the XOR system is
/// inconsistent (elimination derives a `0 = 1` row), a sound witness of UNSAT.
pub(crate) fn gaussian_unsat(xors: &[XorConstraint], num_vars: usize) -> bool {
    if xors.is_empty() {
        return false;
    }
    let words = num_vars / 64 + 1;
    // Each row: variable bitset (bit (v-1) for 1-based var v) plus the rhs bit.
    let mut row_bits: Vec<Vec<u64>> = Vec::with_capacity(xors.len());
    let mut row_rhs: Vec<bool> = Vec::with_capacity(xors.len());
    for x in xors {
        let mut bits = vec![0u64; words];
        for &v in &x.vars {
            if v == 0 || (v as usize) > num_vars {
                continue;
            }
            let b = (v as usize) - 1;
            bits[b / 64] |= 1u64 << (b % 64);
        }
        row_bits.push(bits);
        row_rhs.push(x.rhs);
    }

    let n = row_bits.len();
    let mut pivot = 0usize;
    for col in 0..num_vars {
        if pivot >= n {
            break;
        }
        let w = col / 64;
        let bit = 1u64 << (col % 64);
        // Find a pivot row at/below `pivot` with this column set.
        let mut sel = None;
        for r in pivot..n {
            if row_bits[r][w] & bit != 0 {
                sel = Some(r);
                break;
            }
        }
        let Some(sel) = sel else {
            continue;
        };
        row_bits.swap(pivot, sel);
        row_rhs.swap(pivot, sel);

        let pivot_bits = row_bits[pivot].clone();
        let pivot_rhs = row_rhs[pivot];
        for r in 0..n {
            if r != pivot && row_bits[r][w] & bit != 0 {
                for i in 0..words {
                    row_bits[r][i] ^= pivot_bits[i];
                }
                row_rhs[r] ^= pivot_rhs;
            }
        }
        pivot += 1;
    }

    // A row with no variables but rhs = true is the contradiction 0 = 1.
    for r in 0..n {
        if row_rhs[r] && row_bits[r].iter().all(|&w| w == 0) {
            return true;
        }
    }
    false
}

// ===========================================================================
// Phase 3: DRAT proof emission for parity-UNSAT (bead SAT-playground-qld).
//
// Strategy: variable elimination over the XOR rows (Gaussian elimination), where
// every step is certified by *resolution*. Each clause we emit is a resolvent of
// two clauses already in the accumulated formula, hence trivially RUP, so the
// proof needs no extension/fresh variables. drat-trim does unit propagation over
// all accumulated + original clauses, so every emitted line verifies as RUP.
//
// To eliminate a variable `v`: take all rows containing `v`, pick one as pivot,
// and replace each other row `R` with `R xor pivot` (which drops `v` and any
// other shared variable, GF(2)). Combining two XOR rows is done by clause
// resolution: we materialise the clauses of both rows and resolve away their
// shared variables one at a time; the surviving clauses over the symmetric
// difference are exactly the clauses of the combined row, and every resolvent is
// emitted as a proof line. Elimination order is greedy min-degree, which keeps
// intermediate row width bounded (~treewidth) on structured parity instances
// like Tseitin grids, so the proof stays polynomial.
//
// The width guard (`MAX_COMBINE_VARS`) caps the local variable union of a single
// combine at 128 (fits a u128 sign-mask). On the bounded-width instances this
// engine targets the union stays small (<=~26 for the 12x12 Tseitin grid). If a
// combine would exceed the cap we abort and return `false` (no proof emitted),
// which is always safe: the caller must then report UNKNOWN, never UNSAT.
// ===========================================================================

const MAX_COMBINE_VARS: usize = 128;

/// Canonicalise a clause: sort literals by variable then sign, dedup. Matches the
/// representation used for the dedup/`have` set and for drat-trim output.
fn canon_clause(mut c: Vec<i32>) -> Vec<i32> {
    c.sort_by_key(|&l| (l.unsigned_abs(), l));
    c.dedup();
    c
}

/// All clauses of the XOR row `XOR(vars) = rhs` (the `2^(k-1)` full clauses whose
/// negation-parity matches `rhs`). For an empty `vars`, the row is `0 = rhs`:
/// `0 = 1` yields the single empty clause, `0 = 0` yields nothing.
fn xor_row_clauses(vars: &[u32], rhs: bool) -> Vec<Vec<i32>> {
    let k = vars.len();
    let mut out = Vec::new();
    if k == 0 {
        if rhs {
            out.push(Vec::new());
        }
        return out;
    }
    for mask in 0u32..(1u32 << k) {
        let parity_odd = (mask.count_ones() & 1) == 1;
        // Even negation-parity encodes rhs = true.
        if (!parity_odd) != rhs {
            continue;
        }
        let mut cl: Vec<i32> = Vec::with_capacity(k);
        for (j, &v) in vars.iter().enumerate() {
            if (mask >> j) & 1 == 1 {
                cl.push(-(v as i32));
            } else {
                cl.push(v as i32);
            }
        }
        out.push(canon_clause(cl));
    }
    out
}

/// Combine two XOR rows `A = (a_vars, a_rhs)` and `B = (b_vars, b_rhs)` into
/// `C = A xor B`, emitting (via `emit`) every resolvent so that all of `C`'s
/// clauses end up in the accumulated formula. Returns `Some((c_vars, c_rhs))` on
/// success, or `None` if the variable union exceeds `MAX_COMBINE_VARS`.
///
/// `have` tracks every clause already asserted (original axioms + previously
/// emitted) so each clause is emitted at most once. The clauses of `A` and `B`
/// themselves are assumed already present and are not re-emitted.
fn combine_rows(
    a_vars: &[u32],
    a_rhs: bool,
    b_vars: &[u32],
    b_rhs: bool,
    have: &mut std::collections::HashSet<Vec<i32>>,
    emit: &mut dyn FnMut(&[i32]),
) -> Option<(Vec<u32>, bool)> {
    use std::collections::HashSet;

    // Local variable index = sorted union of the two rows' variables.
    let mut union: Vec<u32> = a_vars.to_vec();
    union.extend_from_slice(b_vars);
    union.sort_unstable();
    union.dedup();
    if union.len() > MAX_COMBINE_VARS {
        return None;
    }
    let local_of = |v: u32| -> usize { union.binary_search(&v).unwrap() };

    // A local clause is (pos_mask, neg_mask) over the union index.
    let to_local = |cl: &[i32]| -> (u128, u128) {
        let mut pos = 0u128;
        let mut neg = 0u128;
        for &lit in cl {
            let idx = local_of(lit.unsigned_abs());
            if lit < 0 {
                neg |= 1u128 << idx;
            } else {
                pos |= 1u128 << idx;
            }
        }
        (pos, neg)
    };
    let to_global = |pos: u128, neg: u128| -> Vec<i32> {
        let mut cl = Vec::new();
        for (i, &v) in union.iter().enumerate() {
            if pos & (1u128 << i) != 0 {
                cl.push(v as i32);
            } else if neg & (1u128 << i) != 0 {
                cl.push(-(v as i32));
            }
        }
        canon_clause(cl)
    };

    // Shared variables (local indices), eliminated one at a time.
    let a_set: HashSet<u32> = a_vars.iter().copied().collect();
    let mut shared: Vec<usize> = b_vars
        .iter()
        .copied()
        .filter(|v| a_set.contains(v))
        .map(local_of)
        .collect();
    shared.sort_unstable();

    // Working clause set (deduped) in local masks.
    let mut work: Vec<(u128, u128)> = Vec::new();
    {
        let mut seen: HashSet<(u128, u128)> = HashSet::new();
        for cl in xor_row_clauses(a_vars, a_rhs) {
            let m = to_local(&cl);
            if seen.insert(m) {
                work.push(m);
            }
        }
        for cl in xor_row_clauses(b_vars, b_rhs) {
            let m = to_local(&cl);
            if seen.insert(m) {
                work.push(m);
            }
        }
    }

    for &ti in &shared {
        let bit = 1u128 << ti;
        let mut pos: Vec<(u128, u128)> = Vec::new();
        let mut neg: Vec<(u128, u128)> = Vec::new();
        let mut newwork: Vec<(u128, u128)> = Vec::new();
        let mut seen: HashSet<(u128, u128)> = HashSet::new();
        for &(p, n) in &work {
            if p & bit != 0 {
                pos.push((p, n));
            } else if n & bit != 0 {
                neg.push((p, n));
            } else if seen.insert((p, n)) {
                newwork.push((p, n));
            }
        }
        for &(pp, pn) in &pos {
            let pp_c = pp & !bit;
            let pn_c = pn & !bit;
            for &(np, nn) in &neg {
                let rp = pp_c | (np & !bit);
                let rn = pn_c | (nn & !bit);
                // Tautological resolvent (a variable both pos and neg): skip.
                if rp & rn != 0 {
                    continue;
                }
                if seen.insert((rp, rn)) {
                    let g = to_global(rp, rn);
                    if !have.contains(&g) {
                        emit(&g);
                        have.insert(g);
                    }
                    newwork.push((rp, rn));
                }
            }
        }
        work = newwork;
    }

    let mut c_vars: Vec<u32> = a_vars
        .iter()
        .copied()
        .collect::<HashSet<u32>>()
        .symmetric_difference(&b_vars.iter().copied().collect())
        .copied()
        .collect();
    c_vars.sort_unstable();
    Some((c_vars, a_rhs ^ b_rhs))
}

/// Greedy min-degree elimination order over the XOR variable/row incidence,
/// simulating fill-in via symmetric difference. Returns the variable order.
fn min_degree_order(xors: &[XorConstraint]) -> Vec<u32> {
    use std::collections::{HashMap, HashSet};
    let mut rows: Vec<HashSet<u32>> =
        xors.iter().map(|x| x.vars.iter().copied().collect()).collect();
    let mut active: Vec<usize> = (0..rows.len()).collect();
    let mut all: HashSet<u32> = HashSet::new();
    for r in &rows {
        all.extend(r.iter().copied());
    }
    let mut order = Vec::new();
    while !all.is_empty() {
        let mut deg: HashMap<u32, usize> = HashMap::new();
        for &ri in &active {
            for &v in &rows[ri] {
                *deg.entry(v).or_insert(0) += 1;
            }
        }
        if deg.is_empty() {
            break;
        }
        // Min degree, tie-break by smallest variable id for determinism.
        let v = *deg
            .iter()
            .min_by_key(|(&var, &d)| (d, var))
            .map(|(var, _)| var)
            .unwrap();
        order.push(v);
        let containing: Vec<usize> =
            active.iter().copied().filter(|&ri| rows[ri].contains(&v)).collect();
        if let Some((&piv, others)) = containing.split_first() {
            let piv_set = rows[piv].clone();
            for &ri in others {
                let merged: HashSet<u32> =
                    rows[ri].symmetric_difference(&piv_set).copied().collect();
                rows[ri] = merged;
            }
            active.retain(|&ri| ri != piv);
        }
        all.remove(&v);
        for &ri in &active {
            rows[ri].remove(&v);
        }
    }
    order
}

/// Emit a DRAT proof refuting the XOR system, if it is inconsistent. Returns
/// `true` iff a proof was successfully emitted (the system is UNSAT *and* the
/// construction stayed within its width guard); in that case the empty clause
/// has been emitted last. Returns `false` if the system is satisfiable or if the
/// construction could not be completed — the caller must then report UNKNOWN,
/// never UNSAT.
///
/// `emit(clause)` is called once per proof line (a clause addition); the empty
/// clause is emitted as `&[]`.
pub(crate) fn gaussian_unsat_with_proof(
    xors: &[XorConstraint],
    _num_vars: usize,
    emit: &mut dyn FnMut(&[i32]),
) -> bool {
    use std::collections::HashSet;
    if xors.is_empty() {
        return false;
    }
    // Seed `have` with every original clause of every XOR row so we never
    // re-emit an axiom (they are already in drat-trim's formula).
    let mut have: HashSet<Vec<i32>> = HashSet::new();
    for x in xors {
        for cl in xor_row_clauses(&x.vars, x.rhs) {
            have.insert(cl);
        }
    }

    let order = min_degree_order(xors);

    // Active rows as (sorted var set, rhs).
    let mut remaining: Vec<(Vec<u32>, bool)> =
        xors.iter().map(|x| (x.vars.clone(), x.rhs)).collect();

    for &v in &order {
        let bucket: Vec<usize> = remaining
            .iter()
            .enumerate()
            .filter(|(_, (vars, _))| vars.binary_search(&v).is_ok())
            .map(|(i, _)| i)
            .collect();
        if bucket.is_empty() {
            continue;
        }
        let pivot = remaining[bucket[0]].clone();
        let mut to_add: Vec<(Vec<u32>, bool)> = Vec::new();
        let mut empty_found = false;
        for &bi in &bucket[1..] {
            let row = remaining[bi].clone();
            let combined = combine_rows(&row.0, row.1, &pivot.0, pivot.1, &mut have, emit);
            let Some((cvars, crhs)) = combined else {
                // Width guard tripped: abandon proof (safe; no UNSAT claim).
                return false;
            };
            if cvars.is_empty() {
                if crhs {
                    empty_found = true;
                }
                // 0 = 0 row: drop it.
            } else {
                to_add.push((cvars, crhs));
            }
        }
        // Remove the whole bucket (pivot consumed; others replaced by combos).
        let bucket_set: HashSet<usize> = bucket.iter().copied().collect();
        let mut next: Vec<(Vec<u32>, bool)> = Vec::new();
        for (i, r) in remaining.into_iter().enumerate() {
            if !bucket_set.contains(&i) {
                next.push(r);
            }
        }
        remaining = next;
        remaining.extend(to_add);
        if empty_found {
            emit(&[]);
            return true;
        }
    }
    // Any leftover `0 = 1` row.
    for (vars, rhs) in &remaining {
        if vars.is_empty() && *rhs {
            emit(&[]);
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the clausal encoding of an XOR constraint over `vars` with the given
    /// rhs: the 2^(k-1) full clauses whose negation-parity matches.
    fn xor_clauses(vars: &[i32], rhs: bool) -> Vec<Vec<i32>> {
        let k = vars.len();
        let mut clauses = Vec::new();
        for mask in 0u32..(1u32 << k) {
            // Clause forbids the assignment that falsifies it; include the literal
            // for each var with polarity per `mask`. We want clauses whose
            // negation-parity is even iff rhs == true.
            let neg_count = mask.count_ones();
            let parity_odd = (neg_count & 1) == 1;
            // even negs => rhs true. Keep clauses matching the requested rhs.
            if (!parity_odd) != rhs {
                continue;
            }
            let mut clause = Vec::with_capacity(k);
            for (j, &v) in vars.iter().enumerate() {
                if mask & (1 << j) != 0 {
                    clause.push(-v);
                } else {
                    clause.push(v);
                }
            }
            clauses.push(clause);
        }
        clauses
    }

    #[test]
    fn extracts_single_xor_degree3() {
        let clauses = xor_clauses(&[1, 3, 4], true);
        assert_eq!(clauses.len(), 4);
        let (xors, consumed) = extract_xors(&clauses, 8);
        assert_eq!(xors.len(), 1);
        assert_eq!(xors[0].vars, vec![1, 3, 4]);
        assert!(xors[0].rhs);
        assert_eq!(consumed.len(), 4);
    }

    #[test]
    fn extracts_xor_rhs_false() {
        let clauses = xor_clauses(&[2, 5], false);
        let (xors, _) = extract_xors(&clauses, 8);
        assert_eq!(xors.len(), 1);
        assert_eq!(xors[0].vars, vec![2, 5]);
        assert!(!xors[0].rhs);
    }

    #[test]
    fn incomplete_group_not_extracted() {
        // Only 3 of the 4 clauses of a degree-3 XOR: not a complete group.
        let mut clauses = xor_clauses(&[1, 2, 3], true);
        clauses.pop();
        let (xors, consumed) = extract_xors(&clauses, 8);
        assert!(xors.is_empty());
        assert!(consumed.is_empty());
    }

    #[test]
    fn non_xor_clauses_not_extracted() {
        // A plain Horn-ish set that does not form a parity group.
        let clauses = vec![vec![1, 2, 3], vec![1, 2, 3], vec![-1, 2]];
        let (xors, _) = extract_xors(&clauses, 8);
        assert!(xors.is_empty());
    }

    #[test]
    fn odd_cycle_is_unsat() {
        // x1^x2=1, x2^x3=1, x3^x1=1 sums to 0=1 (odd cycle) => UNSAT.
        let mut clauses = Vec::new();
        clauses.extend(xor_clauses(&[1, 2], true));
        clauses.extend(xor_clauses(&[2, 3], true));
        clauses.extend(xor_clauses(&[3, 1], true));
        let (xors, _) = extract_xors(&clauses, 8);
        assert_eq!(xors.len(), 3);
        assert!(gaussian_unsat(&xors, 3));
    }

    #[test]
    fn even_cycle_is_satisfiable() {
        // x1^x2=1, x2^x3=1, x3^x1=0 is consistent => no contradiction.
        let mut clauses = Vec::new();
        clauses.extend(xor_clauses(&[1, 2], true));
        clauses.extend(xor_clauses(&[2, 3], true));
        clauses.extend(xor_clauses(&[3, 1], false));
        let (xors, _) = extract_xors(&clauses, 8);
        assert_eq!(xors.len(), 3);
        assert!(!gaussian_unsat(&xors, 3));
    }

    #[test]
    fn independent_consistent_xors_not_unsat() {
        let mut clauses = Vec::new();
        clauses.extend(xor_clauses(&[1, 2, 3], true));
        clauses.extend(xor_clauses(&[4, 5], false));
        let (xors, _) = extract_xors(&clauses, 8);
        assert_eq!(xors.len(), 2);
        assert!(!gaussian_unsat(&xors, 5));
    }

    // ---- Phase 3: DRAT proof emission validated by drat-trim ----

    fn drat_trim_path() -> std::path::PathBuf {
        // CARGO_MANIFEST_DIR = .../solver/12-kissat-inprocessing
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest
            .join("../../tools/checkers/drat-trim/drat-trim")
            .canonicalize()
            .unwrap_or_else(|_| manifest.join("../../tools/checkers/drat-trim/drat-trim"))
    }

    /// Build the CNF from the XOR rows, emit a proof, and run drat-trim. Returns
    /// `Some(verified)` or `None` if the drat-trim binary is unavailable (skip).
    fn drat_verify_xor_system(xors: &[XorConstraint], num_vars: usize) -> Option<bool> {
        use std::io::Write;
        let drat = drat_trim_path();
        if !drat.exists() {
            eprintln!("drat-trim not found at {drat:?}; skipping proof check");
            return None;
        }
        // Assemble CNF clauses (the XOR rows' clausal encodings = axioms).
        let mut cnf_clauses: Vec<Vec<i32>> = Vec::new();
        for x in xors {
            cnf_clauses.extend(xor_row_clauses(&x.vars, x.rhs));
        }
        // Emit the proof.
        let mut proof_lines: Vec<Vec<i32>> = Vec::new();
        let ok = {
            let mut emit = |c: &[i32]| proof_lines.push(c.to_vec());
            gaussian_unsat_with_proof(xors, num_vars, &mut emit)
        };
        assert!(ok, "expected UNSAT proof to be produced");
        assert_eq!(
            proof_lines.last().map(|c| c.is_empty()),
            Some(true),
            "proof must end with the empty clause"
        );

        // Write CNF + proof to temp files.
        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let cnf_path = dir.join(format!("gauss_proof_{pid}_{num_vars}.cnf"));
        let proof_path = dir.join(format!("gauss_proof_{pid}_{num_vars}.drat"));
        {
            let mut f = std::fs::File::create(&cnf_path).unwrap();
            writeln!(f, "p cnf {} {}", num_vars, cnf_clauses.len()).unwrap();
            for c in &cnf_clauses {
                for &l in c {
                    write!(f, "{l} ").unwrap();
                }
                writeln!(f, "0").unwrap();
            }
        }
        {
            let mut f = std::fs::File::create(&proof_path).unwrap();
            for c in &proof_lines {
                for &l in c {
                    write!(f, "{l} ").unwrap();
                }
                writeln!(f, "0").unwrap();
            }
        }
        let out = std::process::Command::new(&drat)
            .arg(&cnf_path)
            .arg(&proof_path)
            .output()
            .expect("failed to run drat-trim");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let _ = std::fs::remove_file(&cnf_path);
        let _ = std::fs::remove_file(&proof_path);
        Some(stdout.contains("s VERIFIED"))
    }

    #[test]
    fn proof_odd_cycle_verified() {
        let xors = vec![
            XorConstraint { vars: vec![1, 2], rhs: true },
            XorConstraint { vars: vec![2, 3], rhs: true },
            XorConstraint { vars: vec![1, 3], rhs: true },
        ];
        assert!(gaussian_unsat(&xors, 3));
        if let Some(v) = drat_verify_xor_system(&xors, 3) {
            assert!(v, "drat-trim must verify the odd-cycle proof");
        }
    }

    /// Build a small RxC grid Tseitin parity system (edges = variables, one node
    /// carries odd charge => UNSAT) as XOR constraints.
    fn grid_tseitin(rows: usize, cols: usize) -> (usize, Vec<XorConstraint>) {
        use std::collections::HashMap;
        let mut eid: HashMap<(usize, usize), u32> = HashMap::new();
        let mut nv: u32 = 0;
        let mut edge = |a: (usize, usize), b: (usize, usize)| -> u32 {
            let key = if a <= b { (a, b) } else { (b, a) };
            // node ids encoded as i*cols+j packed into the tuple via flattening
            let ka = key.0 .0 * cols + key.0 .1;
            let kb = key.1 .0 * cols + key.1 .1;
            let flat = (ka, kb);
            *eid.entry(flat).or_insert_with(|| {
                nv += 1;
                nv
            })
        };
        let mut inc: Vec<Vec<u32>> = vec![Vec::new(); rows * cols];
        for i in 0..rows {
            for j in 0..cols {
                if j + 1 < cols {
                    let e = edge((i, j), (i, j + 1));
                    inc[i * cols + j].push(e);
                    inc[i * cols + j + 1].push(e);
                }
                if i + 1 < rows {
                    let e = edge((i, j), (i + 1, j));
                    inc[i * cols + j].push(e);
                    inc[(i + 1) * cols + j].push(e);
                }
            }
        }
        let mut xors = Vec::new();
        for (idx, mut vars) in inc.into_iter().enumerate() {
            vars.sort_unstable();
            xors.push(XorConstraint { vars, rhs: idx == 0 });
        }
        (nv as usize, xors)
    }

    #[test]
    fn proof_small_grid_tseitin_verified() {
        // 3x3 grid Tseitin: parity-UNSAT, exercises multi-variable combines.
        let (nv, xors) = grid_tseitin(3, 3);
        assert!(gaussian_unsat(&xors, nv));
        if let Some(v) = drat_verify_xor_system(&xors, nv) {
            assert!(v, "drat-trim must verify the 3x3 grid Tseitin proof");
        }
    }

    #[test]
    fn proof_grid_tseitin_4x4_verified() {
        let (nv, xors) = grid_tseitin(4, 4);
        assert!(gaussian_unsat(&xors, nv));
        if let Some(v) = drat_verify_xor_system(&xors, nv) {
            assert!(v, "drat-trim must verify the 4x4 grid Tseitin proof");
        }
    }

    #[test]
    fn proof_not_emitted_for_satisfiable_system() {
        // Even cycle is consistent: no proof should be produced.
        let xors = vec![
            XorConstraint { vars: vec![1, 2], rhs: true },
            XorConstraint { vars: vec![2, 3], rhs: true },
            XorConstraint { vars: vec![1, 3], rhs: false },
        ];
        let mut lines = 0usize;
        let mut emit = |_c: &[i32]| lines += 1;
        let ok = gaussian_unsat_with_proof(&xors, 3, &mut emit);
        assert!(!ok, "satisfiable system must not yield a proof");
    }
}
