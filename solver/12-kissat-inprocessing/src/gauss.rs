//! XOR/parity reasoning: extraction of XOR constraints from CNF and Gaussian
//! elimination over GF(2).
//!
//! Targets parity-heavy instances (e.g. Tseitin grid formulas) where CDCL needs
//! exponential resolution but Gaussian elimination detects unsatisfiability in
//! polynomial time.
//!
//! This module is Phases 1+2 of the parity engine (bead SAT-playground-qld):
//! extraction (`extract_xors`) and UNSAT *detection* (`gaussian_unsat`). Detection
//! does NOT entitle the solver to emit `s UNSATISFIABLE`: a DRAT proof
//! (fresh-variable chaining, Phase 3) must be produced and drat-trim-verified
//! first. Until that lands, `SAT_GAUSS` runs detect-only (logs, no UNSAT claim).
//!
//! Soundness note for UNSAT detection: the extracted XOR constraints are exactly a
//! subset of the original clauses (each XOR is its 2^(k-1) clausal encoding). If
//! the XOR subsystem alone is inconsistent then the whole formula is UNSAT,
//! regardless of any additional non-XOR clauses. So detecting a `0 = 1` row is a
//! sound witness of UNSAT (the proof obligation is only about *certifying* it).

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
}
