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
    // Deterministic output order: HashMap iteration order varies per process
    // (RandomState), which would make proof construction order — and thus proof
    // size/time — nondeterministic. Sort groups by their smallest member clause
    // index, which recovers the generator's clause order (e.g. row-major vertex
    // order on grid Tseitin instances, exactly the locality the summation proof
    // wants).
    let mut ordered: Vec<(Vec<u32>, Vec<usize>)> = groups.into_iter().collect();
    ordered.sort_by_key(|(_, members)| members.iter().copied().min().unwrap_or(usize::MAX));
    for (vars, members) in ordered {
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

/// Hard cap on the width (variable count) of any XOR row we are willing to
/// materialize. `xor_row_clauses` enumerates `2^(k-1)` clauses for a width-`k`
/// row, so width alone bounds both time and memory. Combined rows grow in width
/// across elimination steps (symmetric difference of the two operand supports),
/// so an unbounded chain can reach widths that (1) OOM well before k=32
/// (k=25 => 2^24 clause Vecs) and (2) with release `overflow-checks = false`
/// wrap `1u32 << k` for k>=32, silently enumerating the WRONG clause set and
/// emitting an invalid DRAT proof under `s UNSATISFIABLE`. Exceeding this cap
/// aborts the refutation safely (`None` -> caller reports UNKNOWN, never UNSAT),
/// leaving CDCL to solve the instance. 24 keeps the bounded-treewidth parity
/// systems this engine targets (e.g. the 12x12 Tseitin grid) while making the
/// materialization cost bounded and the `1u32 << k` shift always well-defined.
const MAX_ROW_WIDTH: usize = 24;

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
    // Invariant: all call sites gate width at MAX_ROW_WIDTH (< 32) before
    // reaching here, so `1u32 << k` below never wraps. This asserts the invariant
    // in debug builds; release safety is provided by the combine_rows guards.
    debug_assert!(
        k < 32,
        "xor_row_clauses width {k} would overflow 1u32 << k (see MAX_ROW_WIDTH guard)"
    );
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

    // Width guard (mpo (a)): `xor_row_clauses(a_vars)` / `(b_vars)` below
    // materialize 2^(k-1) clauses each, so abort BEFORE materializing if either
    // operand row is wider than the cap. Combined rows fed back through the
    // elimination loop can reach these widths; aborting here (None) is always
    // safe — the caller reports UNKNOWN, never UNSAT.
    if a_vars.len() > MAX_ROW_WIDTH || b_vars.len() > MAX_ROW_WIDTH {
        return None;
    }

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
    // Result width guard (mpo (a)): a combined row wider than the cap must never
    // be stored back into the active set, or a later combine would try to
    // materialize 2^(k-1) clauses for it. Abort now rather than defer the OOM.
    if c_vars.len() > MAX_ROW_WIDTH {
        return None;
    }
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

// ===========================================================================
// Scalable Tseitin-component refutation with extension variables (bead
// SAT-playground-kk8).
//
// The resolution-only engine above materializes 2^(k-1) clauses per XOR row, so
// it is capped at MAX_ROW_WIDTH = 24 and cannot refute parity systems whose
// elimination fill-in exceeds that width (a 3-regular expander like
// tseitin_n188_d3, cutwidth ~30, or a 400x400 grid, cutwidth ~400). Those
// systems have polynomial *extended-resolution* refutations: DRAT permits RAT
// additions over fresh variables, so we can define `z <-> a ^ b` (4 clauses,
// pivot literal first) and use `z` to keep every materialized row narrow.
//
// This engine targets the closed Tseitin shape: every variable occurs in
// exactly two XOR constraints (edges of a graph whose vertices are the
// constraints). Summing all constraints of a connected component cancels every
// variable, leaving `0 = charge`; if the component's charge (XOR of rhs) is
// odd the formula is UNSAT. Detection is linear (union-find + parity).
//
// The proof sums the component's constraints one at a time, maintaining the
// partial-sum row P. Raw cut variables in P are compressed in pairs into fresh
// definition variables whenever |P| exceeds a small target width, pairing the
// two variables whose *next use* (second occurrence) is farthest in the future
// (Belady). When an upcoming constraint needs a variable hidden inside
// definitions, the definition chain is reopened top-down with one small row
// combine per level. Every row combine stays within MAX_ROW_WIDTH, so the
// existing `combine_rows` machinery (pure RUP resolvents) is reused unchanged.
// Every emitted line is independently valid DRAT (RAT definitions + RUP
// resolvents), so a caller may stream lines directly into the live proof and
// safely abandon on an internal guard: leftover additions never invalidate a
// later proof.
// ===========================================================================

/// Raw (uncompressed) partial-sum width target. Smaller keeps each combine's
/// materialization tiny (2^(w-1) clauses; chain pointers add ~2 per live chain
/// on top); larger reduces compression churn. On the 400x400 grid Tseitin cell
/// the steady-state row is ~4 chain pointers + raws, so 2 keeps combines at
/// ~2^6 clauses. Overridable via `SAT_TSEITIN_COMPRESS` for measurement.
const TSEITIN_COMPRESS_TARGET: usize = 2;

/// Abort proof construction after this many emitted clauses. This is a
/// VERIFIABILITY bound, not just a runtime backstop: the harness re-checks
/// every UNSAT proof with backward drat-trim under a 1800 s cap, and measured
/// throughput there is ~8-25 k lemmas/s (tseitin_n188_d3: 4.71 M lemmas =
/// 187 s idle). Proofs past ~6 M lemmas risk `checker-timeout`, which the
/// promotion gate counts as a correctness failure — worse than not answering.
/// (The 400x400 grid Tseitin cell generates a valid 14.6 M-lemma proof in
/// 22 s but cannot be verified in time; it is deliberately left unsolved. See
/// TSEITIN_MAX_COMPONENT.)
const TSEITIN_MAX_EMIT: u64 = 6_000_000;

/// Skip the Tseitin engine on components larger than this. Large components
/// (e.g. the 160 k-equation 400x400 grid) produce proofs beyond the
/// verifiability bound above; declining early keeps the solve trajectory of
/// those cells byte-identical to the pre-engine baseline.
const TSEITIN_MAX_COMPONENT: usize = 20_000;

/// Find a connected component of the XOR system that is *closed Tseitin*
/// (every variable of the component occurs in exactly two constraints) with
/// odd charge (XOR of member rhs values) — a sound UNSAT witness. Returns the
/// member indices (ascending) of the smallest such component, or None.
pub(crate) fn find_odd_closed_tseitin_component(xors: &[XorConstraint]) -> Option<Vec<usize>> {
    use std::collections::HashMap;
    if xors.is_empty() {
        return None;
    }
    // var -> constraint indices (up to 3 recorded; more than 2 disqualifies).
    let mut occ: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, x) in xors.iter().enumerate() {
        for &v in &x.vars {
            let e = occ.entry(v).or_default();
            if e.len() < 3 {
                e.push(i);
            }
        }
    }
    // Union-find over constraints.
    let mut parent: Vec<usize> = (0..xors.len()).collect();
    fn find(parent: &mut Vec<usize>, mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for eqs in occ.values() {
        for w in eqs.windows(2) {
            let (a, b) = (find(&mut parent, w[0]), find(&mut parent, w[1]));
            if a != b {
                parent[a] = b;
            }
        }
    }
    let n = xors.len();
    let mut charge: Vec<bool> = vec![false; n];
    let mut eligible: Vec<bool> = vec![true; n];
    let mut size: Vec<usize> = vec![0; n];
    for i in 0..n {
        let r = find(&mut parent, i);
        charge[r] ^= xors[i].rhs;
        size[r] += 1;
    }
    for (_, eqs) in occ.iter() {
        // A variable occurring once (dangling edge) makes its component's
        // parity satisfiable; occurring 3+ times breaks the Tseitin shape.
        if eqs.len() != 2 {
            let r = find(&mut parent, eqs[0]);
            eligible[r] = false;
        }
    }
    // Smallest odd, closed, eligible component (deterministic tie-break by root).
    let mut best: Option<usize> = None;
    for i in 0..n {
        let r = find(&mut parent, i);
        if r == i && charge[r] && eligible[r] {
            if best.map(|b| size[r] < size[b] || (size[r] == size[b] && r < b)).unwrap_or(true) {
                best = Some(r);
            }
        }
    }
    let root = best?;
    let members: Vec<usize> = (0..n).filter(|&i| find(&mut parent, i) == root).collect();
    Some(members)
}

/// Emit a DRAT refutation of a closed odd-charge Tseitin component (as found
/// by `find_odd_closed_tseitin_component`) using extension variables. Fresh
/// definition variables are numbered from `num_vars + 1`. Returns `true` iff
/// the empty clause was emitted. Every emitted line is independently valid
/// DRAT, so streaming `emit` into a live proof is safe even on `false`.
pub(crate) fn tseitin_refute_with_proof(
    xors: &[XorConstraint],
    component: &[usize],
    num_vars: usize,
    emit: &mut dyn FnMut(&[i32]),
    emit_del: &mut dyn FnMut(&[i32]),
) -> bool {
    use std::collections::HashMap;
    // Measurement overrides for the verifiability caps (see the const docs):
    // SAT_TSEITIN_MAX_COMPONENT / SAT_TSEITIN_MAX_EMIT. Defaults unchanged.
    let max_component: usize = std::env::var("SAT_TSEITIN_MAX_COMPONENT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(TSEITIN_MAX_COMPONENT);
    let max_emit: u64 = std::env::var("SAT_TSEITIN_MAX_EMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(TSEITIN_MAX_EMIT);
    if component.is_empty() || component.len() > max_component {
        return false;
    }
    let compress_target: usize = std::env::var("SAT_TSEITIN_COMPRESS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&c: &usize| (2..=12).contains(&c))
        .unwrap_or(TSEITIN_COMPRESS_TARGET);

    // Processing order: a connected, deterministic walk of the component.
    // Connectivity is required for soundness: `combine_rows` only derives the
    // sum row's clauses when the operands share a variable, so every summand
    // should cancel at least one variable of the partial sum. Among the
    // frontier (constraints adjacent to the processed set) we greedily take
    // the one minimizing cut growth (#introduced - #cancelled variables),
    // tie-broken by index — this bounds the live cut near the graph's
    // bisection width (the whole scheme's width driver) and keeps next-use
    // distances local so the compression chains stay coherent. On a row-major
    // grid this reproduces row-major order.
    let order: Vec<usize> = {
        // var -> the (up to two) component constraints containing it.
        let mut var_eqs: HashMap<u32, (usize, usize)> = HashMap::new();
        for &ei in component {
            for &v in &xors[ei].vars {
                match var_eqs.entry(v) {
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert((ei, usize::MAX));
                    }
                    std::collections::hash_map::Entry::Occupied(mut e) => {
                        if e.get().1 != usize::MAX {
                            return false; // >2 occurrences: not closed Tseitin
                        }
                        e.get_mut().1 = ei;
                    }
                }
            }
        }
        if var_eqs.values().any(|&(_, b)| b == usize::MAX) {
            return false; // some variable occurs once: component not closed
        }
        let in_comp: std::collections::HashSet<usize> = component.iter().copied().collect();
        let start = *component.iter().min().unwrap();
        let mut processed: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut frontier: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        let mut order = Vec::with_capacity(component.len());
        let mut push_neighbors =
            |ei: usize,
             processed: &std::collections::HashSet<usize>,
             frontier: &mut std::collections::BTreeSet<usize>| {
                for &v in &xors[ei].vars {
                    let (a, b) = var_eqs[&v];
                    let other = if a == ei { b } else { a };
                    if in_comp.contains(&other) && !processed.contains(&other) {
                        frontier.insert(other);
                    }
                }
            };
        order.push(start);
        processed.insert(start);
        push_neighbors(start, &processed, &mut frontier);
        loop {
            // Minimize (introduced - cancelled); ties by smallest index.
            let pick = frontier
                .iter()
                .min_by_key(|&&ei| {
                    let mut delta: i64 = 0;
                    for &v in &xors[ei].vars {
                        let (a, b) = var_eqs[&v];
                        let other = if a == ei { b } else { a };
                        if processed.contains(&other) {
                            delta -= 1; // cancels a live cut variable
                        } else {
                            delta += 1; // introduces a new cut variable
                        }
                    }
                    (delta, ei)
                })
                .copied();
            let Some(pick) = pick else {
                break;
            };
            frontier.remove(&pick);
            order.push(pick);
            processed.insert(pick);
            push_neighbors(pick, &processed, &mut frontier);
        }
        if order.len() != component.len() {
            return false; // component not connected (should not happen)
        }
        order
    };

    // Next use of each variable: step index (into `order`) of its second
    // occurrence. Every component variable occurs exactly twice.
    let mut first_seen: HashMap<u32, usize> = HashMap::new();
    let mut next_use: HashMap<u32, usize> = HashMap::new();
    for (step, &ei) in order.iter().enumerate() {
        for &v in &xors[ei].vars {
            match first_seen.entry(v) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(step);
                }
                std::collections::hash_map::Entry::Occupied(_) => {
                    next_use.insert(v, step);
                }
            }
        }
    }

    let emitted = std::cell::Cell::new(0u64);
    // Per-category emission counters (SAT_DEBUG_GAUSS breakdown).
    let cat_main = std::cell::Cell::new(0u64);
    let cat_shift = std::cell::Cell::new(0u64);
    let cat_unpark = std::cell::Cell::new(0u64);
    let cat_append = std::cell::Cell::new(0u64);
    let cat_create = std::cell::Cell::new(0u64);
    let cat_park = std::cell::Cell::new(0u64);
    let cat_defs = std::cell::Cell::new(0u64);

    // One row combine, with proof-hygiene deletions. Handles both the shared
    // case (resolution cascade via `combine_rows`; `have` is fresh per call —
    // a duplicate emission across combines is a legal DRAT addition and
    // per-call scoping keeps memory bounded) and the disjoint case (every
    // clause of the sum `C = A ^ B` of variable-disjoint rows is directly RUP
    // given both operands' full clause sets: falsifying a C-clause assigns
    // every variable of A and B, violating the parity of at least one side).
    //
    // Deletions keep drat-trim's live clause database small (the derivation is
    // linear, so spent clauses are never revisited): after each combine we
    // delete every intermediate resolvent (emitted clauses not in the result
    // row), the old P row's clauses, and — when `delete_q` says this was the
    // Q row's last use — the Q row's clauses. drat-trim matches deletions on
    // sorted literals and keeps duplicate additions as separate copies, so
    // deleting our copy never kills a live original.
    #[allow(clippy::too_many_arguments)]
    fn do_combine(
        p_vars: &[u32],
        p_rhs: bool,
        q_vars: &[u32],
        q_rhs: bool,
        delete_q: bool,
        emitted: &std::cell::Cell<u64>,
        cat: &std::cell::Cell<u64>,
        add: &mut dyn FnMut(&[i32]),
        del: &mut dyn FnMut(&[i32]),
    ) -> Option<(Vec<u32>, bool)> {
        let disjoint = !p_vars.iter().any(|v| q_vars.contains(v));
        let mut local: Vec<Vec<i32>> = Vec::new();
        let res = {
            let mut emitf = |c: &[i32]| local.push(c.to_vec());
            if disjoint {
                let mut c_vars: Vec<u32> = p_vars.to_vec();
                c_vars.extend_from_slice(q_vars);
                c_vars.sort_unstable();
                c_vars.dedup();
                if c_vars.len() != p_vars.len() + q_vars.len()
                    || c_vars.len() > MAX_ROW_WIDTH
                {
                    return None;
                }
                let c_rhs = p_rhs ^ q_rhs;
                for cl in xor_row_clauses(&c_vars, c_rhs) {
                    emitf(&cl);
                }
                (c_vars, c_rhs)
            } else {
                let mut have = std::collections::HashSet::new();
                combine_rows(p_vars, p_rhs, q_vars, q_rhs, &mut have, &mut emitf)?
            }
        };
        emitted.set(emitted.get() + local.len() as u64);
        cat.set(cat.get() + local.len() as u64);
        for c in &local {
            add(c);
        }
        let result_set: std::collections::HashSet<Vec<i32>> =
            xor_row_clauses(&res.0, res.1).into_iter().collect();
        for c in &local {
            if !result_set.contains(c) {
                del(c);
            }
        }
        for c in xor_row_clauses(p_vars, p_rhs) {
            del(&c);
        }
        if delete_q {
            for c in xor_row_clauses(q_vars, q_rhs) {
                del(&c);
            }
        }
        Some(res)
    }

    // A prefix-accumulator chain compresses cut variables ordered by next use.
    // `zs[j]` (fresh) is defined as the parity of `vars[0..=j+1]`; `z(1)` is
    // `vars[0]` itself. While alive the chain occupies P as `{z(end)}` before
    // first consumption, then as the pointer pair `{z(end), z(base)}` meaning
    // "parity of vars[base..end]". Consumption shifts the base pointer forward
    // with one 3-var definition-row combine per variable — no unwinding.
    struct Chain {
        vars: Vec<u32>,
        zs: Vec<u32>,
        base: usize, // number of consumed vars; 0 = untouched
        /// Parked pointer pair: `(z', z_end, z_base)` with `z' = z_end ^
        /// z_base` defined. While parked, P holds only `z'` for this chain
        /// (instead of the pair), halving the standing width cost of a
        /// dormant partially-consumed chain. Unparking replays the same
        /// definition row.
        parked: Option<(u32, u32, u32)>,
    }
    impl Chain {
        fn z(&self, j: usize) -> u32 {
            // 1-based prefix index; z(1) = vars[0].
            if j == 1 {
                self.vars[0]
            } else {
                self.zs[j - 2]
            }
        }
        fn len(&self) -> usize {
            self.vars.len()
        }
    }

    // Raw-var width target for P (excluding chain pointers) and the hard cap on
    // total P width. Exceeding the cap aborts safely (caller falls back).
    let raw_target = compress_target;
    const P_WIDTH_CAP: usize = 14;

    let mut fresh: u32 = num_vars as u32;
    // Definition-variable recycling. A definition variable is returned to this
    // free list the moment no live (undeleted) emitted clause can mention it:
    // a park variable at unpark (its definition row is consumed there,
    // delete_q=true), a chain's variables when the chain dies (every
    // definition row has been consumed by a shift by then, and only z(end)/
    // z(base) ever sat in P — both cancel in the dying combine, whose old-P
    // clauses are deleted inside `do_combine`). Redefining a recycled
    // variable is sound DRAT: the RAT check on the new definition's pivot
    // sees no live clause over the variable, exactly as for a never-used one.
    // This keeps the proof's variable space at `num_vars + O(live chains)`
    // instead of one variable per definition — on the 400x400 grid Tseitin
    // proof ~320 k total vars instead of ~1.1 M. Backward drat-trim's array
    // footprint and its per-RAT-lemma watch scans are proportional to the
    // variable space, which is what made that proof unverifiable in time.
    let mut free_zs: Vec<u32> = Vec::new();
    let mut chains: Vec<Chain> = Vec::new();
    // var -> chain index (only while the var is unconsumed inside a chain).
    let mut in_chain: HashMap<u32, usize> = HashMap::new();

    // Partial-sum row.
    let mut p_vars: Vec<u32> = xors[order[0]].vars.clone();
    let mut p_rhs: bool = xors[order[0]].rhs;

    // Emit the four RAT definition clauses of `z <-> a ^ b`, pivot first.
    fn emit_def(
        z: u32,
        a: u32,
        b: u32,
        emitted: &std::cell::Cell<u64>,
        add: &mut dyn FnMut(&[i32]),
    ) {
        let (zi, ai, bi) = (z as i32, a as i32, b as i32);
        emitted.set(emitted.get() + 4);
        add(&[-zi, ai, bi]);
        add(&[-zi, -ai, -bi]);
        add(&[zi, -ai, bi]);
        add(&[zi, ai, -bi]);
    }

    let debug = std::env::var("SAT_DEBUG_GAUSS").is_ok();
    let mut max_width = 0usize;
    for (step, &ei) in order.iter().enumerate().skip(1) {
        max_width = max_width.max(p_vars.len());
        if emitted.get() > max_emit || p_vars.len() > P_WIDTH_CAP {
            if debug {
                eprintln!(
                    "c tseitin abort step={} emitted={} p_width={} max_width={} chains={}",
                    step,
                    emitted.get(),
                    p_vars.len(),
                    max_width,
                    chains.len()
                );
            }
            return false;
        }
        let eq = &xors[ei];
        // Expose every to-be-cancelled variable hidden inside a chain by
        // shifting that chain's base pointer forward to it.
        for &v in &eq.vars {
            if next_use.get(&v) != Some(&step) {
                continue; // first occurrence: nothing hidden
            }
            let Some(&ci) = in_chain.get(&v) else {
                continue; // raw in P already
            };
            // Unpark first: consumption needs the pointer pair in P. The park
            // definition is spent after this (last use), so `zp` recycles.
            if let Some((zp, ze, zb)) = chains[ci].parked.take() {
                let mut row = vec![zp, ze, zb];
                row.sort_unstable();
                let Some((nv, nr)) =
                    do_combine(&p_vars, p_rhs, &row, false, true, &emitted, &cat_unpark, emit, emit_del)
                else {
                    return false;
                };
                p_vars = nv;
                p_rhs = nr;
                free_zs.push(zp);
            }
            loop {
                if emitted.get() > max_emit {
                    return false;
                }
                let (die, shift_row): (bool, Vec<u32>) = {
                    let c = &chains[ci];
                    let b = c.base;
                    debug_assert!(b < c.len());
                    if b == 0 {
                        if c.len() == 2 {
                            // {z2, x1, x2}: shares z2 (= z(end)) with P.
                            (true, vec![c.z(2), c.vars[0], c.vars[1]])
                        } else {
                            // {z2, x1, x2} shares nothing with P (z(end) is
                            // deeper): disjoint sum, exposes x1 and x2.
                            (false, vec![c.z(2), c.vars[0], c.vars[1]])
                        }
                    } else if b + 1 == c.len() {
                        // Final shift {z(end), z(b), x_last}: both pointers
                        // cancel; the chain dies.
                        (true, vec![c.z(b + 1), c.z(b), c.vars[b]])
                    } else {
                        // {z(b+1), z(b), x_{b+1}}: pointer moves forward.
                        (false, vec![c.z(b + 1), c.z(b), c.vars[b]])
                    }
                };
                let mut row = shift_row;
                row.sort_unstable();
                row.dedup();
                // The shift consumes this definition row (its last use).
                let Some((nv, nr)) =
                    do_combine(&p_vars, p_rhs, &row, false, true, &emitted, &cat_shift, emit, emit_del)
                else {
                    return false;
                };
                p_vars = nv;
                p_rhs = nr;
                let c = &mut chains[ci];
                if die {
                    for &x in &c.vars[c.base..] {
                        in_chain.remove(&x);
                    }
                    // The dying combine cancelled both pointers out of P and
                    // consumed the last definition row; every interior z was
                    // already recycled by the shift that moved base past it.
                    if c.base >= 2 {
                        free_zs.push(c.z(c.base));
                    }
                    free_zs.push(c.z(c.len()));
                    c.base = c.len();
                } else if c.base == 0 {
                    // First opening of a len>=3 chain exposed vars[0] and
                    // vars[1] raw.
                    in_chain.remove(&c.vars[0]);
                    in_chain.remove(&c.vars[1]);
                    c.base = 2;
                } else {
                    in_chain.remove(&c.vars[c.base]);
                    // The old base pointer z(base) is now fully dead: its
                    // definition row was consumed by the previous shift, the
                    // row consumed just now (the definition of z(base+1)) was
                    // its last remaining reference, and it left P in this
                    // combine (old-P clauses deleted inside `do_combine`).
                    // base is never 1, so z(base) here is always a fresh var.
                    free_zs.push(c.z(c.base));
                    c.base += 1;
                }
                if !in_chain.contains_key(&v) {
                    break;
                }
                if p_vars.len() > P_WIDTH_CAP {
                    return false;
                }
            }
        }
        // Sum the constraint into P (the exposed shared variables cancel); the
        // constraint's axiom clauses are spent afterwards.
        let Some((nv, nr)) =
            do_combine(&p_vars, p_rhs, &eq.vars, eq.rhs, true, &emitted, &cat_main, emit, emit_del)
        else {
            return false;
        };
        p_vars = nv;
        p_rhs = nr;
        // Compress: move far-next-use raw vars into prefix chains until the raw
        // width is back at the target. The excess is swept in ascending
        // next-use order, each var appended to a chain whose tail stays below
        // it (keeping every chain's internal order ascending — what makes
        // forward consumption line up) or paired with the next excess var into
        // a new chain. Ascending sweep makes consecutive far vars coalesce
        // into few large chains instead of fragmenting into many pointer pairs.
        let mut raw: Vec<u32> = p_vars
            .iter()
            .copied()
            .filter(|v| next_use.contains_key(v) && !in_chain.contains_key(v))
            .collect();
        if raw.len() > raw_target {
            raw.sort_by_key(|&v| (next_use[&v], v));
            let excess = raw.split_off(raw_target);
            let mut pending: Option<u32> = None;
            for &x in &excess {
                if emitted.get() > max_emit {
                    return false;
                }
                let x_nu = next_use[&x];
                // Best append target: alive chain whose tail next-use is
                // maximal but still <= x's.
                let mut target: Option<usize> = None;
                for (i, c) in chains.iter().enumerate() {
                    if c.base >= c.len() {
                        continue; // dead
                    }
                    let tail_nu = next_use[c.vars.last().unwrap()];
                    if tail_nu <= x_nu
                        && target
                            .map(|t| tail_nu > next_use[chains[t].vars.last().unwrap()])
                            .unwrap_or(true)
                    {
                        target = Some(i);
                    }
                }
                if let Some(ci) = target {
                    // Append x: fresh z(end+1) = z(end) ^ x; P swaps
                    // {z(end), x} for {z(end+1)}. A parked chain must be
                    // unparked first (the pair returns to P).
                    if let Some((zp, ze, zb)) = chains[ci].parked.take() {
                        let mut row = vec![zp, ze, zb];
                        row.sort_unstable();
                        let Some((nv, nr)) = do_combine(
                            &p_vars, p_rhs, &row, false, true, &emitted, &cat_unpark, emit, emit_del,
                        ) else {
                            return false;
                        };
                        p_vars = nv;
                        p_rhs = nr;
                        free_zs.push(zp);
                    }
                    let zend = chains[ci].z(chains[ci].len());
                    let z = free_zs.pop().unwrap_or_else(|| {
                        fresh += 1;
                        fresh
                    });
                    cat_defs.set(cat_defs.get() + 4);
                    emit_def(z, zend, x, &emitted, emit);
                    let mut row = vec![z, zend, x];
                    row.sort_unstable();
                    // The append definition is reused by the future shift
                    // through this position: keep it live (delete_q=false).
                    let Some((nv, nr)) = do_combine(
                        &p_vars, p_rhs, &row, false, false, &emitted, &cat_append, emit, emit_del,
                    ) else {
                        return false;
                    };
                    p_vars = nv;
                    p_rhs = nr;
                    let c = &mut chains[ci];
                    c.vars.push(x);
                    c.zs.push(z);
                    in_chain.insert(x, ci);
                } else if let Some(x1) = pending.take() {
                    // Pair the two unplaced excess vars (ascending order) into
                    // a fresh chain. The creation definition is reused when the
                    // chain is first opened: keep it live (delete_q=false).
                    let z = free_zs.pop().unwrap_or_else(|| {
                        fresh += 1;
                        fresh
                    });
                    cat_defs.set(cat_defs.get() + 4);
                    emit_def(z, x1, x, &emitted, emit);
                    let mut row = vec![z, x1, x];
                    row.sort_unstable();
                    let Some((nv, nr)) = do_combine(
                        &p_vars, p_rhs, &row, false, false, &emitted, &cat_create, emit, emit_del,
                    ) else {
                        return false;
                    };
                    p_vars = nv;
                    p_rhs = nr;
                    let ci = chains.len();
                    chains.push(Chain { vars: vec![x1, x], zs: vec![z], base: 0, parked: None });
                    in_chain.insert(x1, ci);
                    in_chain.insert(x, ci);
                } else {
                    pending = Some(x);
                }
            }
            // A leftover unpaired var simply stays raw.
        }
        // Park the pointer pair of every partially-consumed chain that is not
        // needed on the very next step: {z_end, z_base} becomes one fresh var.
        for ci in 0..chains.len() {
            let c = &chains[ci];
            if c.base == 0 || c.base >= c.len() || c.parked.is_some() {
                continue;
            }
            let next_needed = next_use[&c.vars[c.base]];
            if next_needed <= step + 1 {
                continue; // consumed imminently: parking would just churn
            }
            let ze = c.z(c.len());
            let zb = c.z(c.base);
            let zp = free_zs.pop().unwrap_or_else(|| {
                fresh += 1;
                fresh
            });
            cat_defs.set(cat_defs.get() + 4);
            emit_def(zp, ze, zb, &emitted, emit);
            let mut row = vec![zp, ze, zb];
            row.sort_unstable();
            // The park definition is reused at unpark: keep it live.
            let Some((nv, nr)) =
                do_combine(&p_vars, p_rhs, &row, false, false, &emitted, &cat_park, emit, emit_del)
            else {
                return false;
            };
            p_vars = nv;
            p_rhs = nr;
            chains[ci].parked = Some((zp, ze, zb));
        }
    }

    if debug {
        eprintln!(
            "c tseitin emit breakdown: main={} shift={} unpark={} append={} create={} park={} defs={} total={}",
            cat_main.get(),
            cat_shift.get(),
            cat_unpark.get(),
            cat_append.get(),
            cat_create.get(),
            cat_park.get(),
            cat_defs.get(),
            emitted.get()
        );
    }
    if p_vars.is_empty() && p_rhs {
        emitted.set(emitted.get() + 1);
        emit(&[]);
        true
    } else {
        false
    }
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
    fn combine_rows_aborts_on_overwide_input_without_materializing() {
        // An operand row wider than MAX_ROW_WIDTH would make xor_row_clauses
        // enumerate 2^(k-1) clauses (2^24 for width 25 -> OOM / 1u32<<25 hazard).
        // The input guard must return None BEFORE any materialization.
        let a_vars: Vec<u32> = (1..=(MAX_ROW_WIDTH as u32 + 1)).collect();
        let b_vars: Vec<u32> = vec![MAX_ROW_WIDTH as u32 + 1, MAX_ROW_WIDTH as u32 + 2];
        let mut have = std::collections::HashSet::new();
        let mut emit = |_: &[i32]| panic!("over-wide input must abort before emitting");
        let r = combine_rows(&a_vars, false, &b_vars, false, &mut have, &mut emit);
        assert!(r.is_none(), "over-wide input row must abort, not materialize");
    }

    #[test]
    fn combine_rows_aborts_when_result_exceeds_width_cap() {
        // Two disjoint rows (no shared variable, so no resolution blow-up) whose
        // union width 13 + 12 = 25 exceeds MAX_ROW_WIDTH; the result-width guard
        // must abort so the wide row is never stored for a later materialization.
        let a_vars: Vec<u32> = (1..=13).collect();
        let b_vars: Vec<u32> = (14..=25).collect(); // disjoint from a_vars
        let mut have = std::collections::HashSet::new();
        let mut emit = |_: &[i32]| {};
        let r = combine_rows(&a_vars, false, &b_vars, false, &mut have, &mut emit);
        assert!(
            r.is_none(),
            "combined row wider than MAX_ROW_WIDTH must abort (safe UNKNOWN), not be stored"
        );
    }

    #[test]
    fn combine_rows_succeeds_within_width_cap() {
        // A normal small combine (within the cap) must still work and return the
        // symmetric-difference row, proving the guards do not over-abort.
        let a_vars: Vec<u32> = vec![1, 2, 3];
        let b_vars: Vec<u32> = vec![3, 4]; // shares var 3
        let mut have = std::collections::HashSet::new();
        let mut emit = |_: &[i32]| {};
        let r = combine_rows(&a_vars, false, &b_vars, true, &mut have, &mut emit);
        let (cvars, crhs) = r.expect("in-cap combine must succeed");
        assert_eq!(cvars, vec![1, 2, 4]);
        assert!(crhs, "rhs is a_rhs ^ b_rhs = false ^ true = true");
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

    // ---- Closed-Tseitin extension-variable refutation (bead SAT-playground-kk8) ----

    /// drat-trim verification for the Tseitin engine: CNF = the XOR rows'
    /// clausal encodings, proof = the extension-variable summation refutation.
    fn drat_verify_tseitin(xors: &[XorConstraint], num_vars: usize) -> Option<bool> {
        use std::io::Write;
        let drat = drat_trim_path();
        if !drat.exists() {
            eprintln!("drat-trim not found at {drat:?}; skipping proof check");
            return None;
        }
        let component =
            find_odd_closed_tseitin_component(xors).expect("expected an odd closed component");
        let mut cnf_clauses: Vec<Vec<i32>> = Vec::new();
        for x in xors {
            cnf_clauses.extend(xor_row_clauses(&x.vars, x.rhs));
        }
        // (is_add, clause) so deletions land interleaved in emission order.
        let proof_lines: std::cell::RefCell<Vec<(bool, Vec<i32>)>> =
            std::cell::RefCell::new(Vec::new());
        let ok = {
            let mut emit = |c: &[i32]| proof_lines.borrow_mut().push((true, c.to_vec()));
            let mut emit_del = |c: &[i32]| proof_lines.borrow_mut().push((false, c.to_vec()));
            tseitin_refute_with_proof(xors, &component, num_vars, &mut emit, &mut emit_del)
        };
        let proof_lines = proof_lines.into_inner();
        assert!(ok, "expected the Tseitin engine to produce a proof");
        assert_eq!(
            proof_lines.last().map(|(add, c)| *add && c.is_empty()),
            Some(true)
        );
        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let tag = format!("{}_{}_{}", pid, num_vars, xors.len());
        let cnf_path = dir.join(format!("tseitin_proof_{tag}.cnf"));
        let proof_path = dir.join(format!("tseitin_proof_{tag}.drat"));
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
            for (add, c) in &proof_lines {
                if !add {
                    write!(f, "d ").unwrap();
                }
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
    fn tseitin_detects_odd_cycle() {
        let xors = vec![
            XorConstraint { vars: vec![1, 2], rhs: true },
            XorConstraint { vars: vec![2, 3], rhs: true },
            XorConstraint { vars: vec![1, 3], rhs: true },
        ];
        let comp = find_odd_closed_tseitin_component(&xors).expect("odd cycle is closed+odd");
        assert_eq!(comp, vec![0, 1, 2]);
    }

    #[test]
    fn tseitin_rejects_even_cycle_and_open_chain() {
        let even = vec![
            XorConstraint { vars: vec![1, 2], rhs: true },
            XorConstraint { vars: vec![2, 3], rhs: true },
            XorConstraint { vars: vec![1, 3], rhs: false },
        ];
        assert!(find_odd_closed_tseitin_component(&even).is_none());
        // Open chain: var 3 occurs once, so the odd charge is satisfiable.
        let open = vec![
            XorConstraint { vars: vec![1, 2], rhs: true },
            XorConstraint { vars: vec![2, 3], rhs: false },
        ];
        assert!(find_odd_closed_tseitin_component(&open).is_none());
    }

    #[test]
    fn tseitin_rejects_var_in_three_constraints() {
        let xors = vec![
            XorConstraint { vars: vec![1, 2], rhs: true },
            XorConstraint { vars: vec![1, 3], rhs: true },
            XorConstraint { vars: vec![1, 2, 3], rhs: true },
        ];
        assert!(find_odd_closed_tseitin_component(&xors).is_none());
    }

    #[test]
    fn tseitin_picks_odd_component_among_several() {
        // Component A (vars 1-3): even cycle (consistent). Component B (vars
        // 4-6): odd cycle (UNSAT).
        let xors = vec![
            XorConstraint { vars: vec![1, 2], rhs: true },
            XorConstraint { vars: vec![2, 3], rhs: true },
            XorConstraint { vars: vec![1, 3], rhs: false },
            XorConstraint { vars: vec![4, 5], rhs: true },
            XorConstraint { vars: vec![5, 6], rhs: true },
            XorConstraint { vars: vec![4, 6], rhs: true },
        ];
        let comp = find_odd_closed_tseitin_component(&xors).expect("odd component exists");
        assert_eq!(comp, vec![3, 4, 5]);
    }

    #[test]
    fn tseitin_proof_odd_cycle_verified() {
        let xors = vec![
            XorConstraint { vars: vec![1, 2], rhs: true },
            XorConstraint { vars: vec![2, 3], rhs: true },
            XorConstraint { vars: vec![1, 3], rhs: true },
        ];
        if let Some(v) = drat_verify_tseitin(&xors, 3) {
            assert!(v, "drat-trim must verify the odd-cycle ER proof");
        }
    }

    #[test]
    fn tseitin_proof_grid_verified() {
        // 6x6 grid: cutwidth ~6 exceeds the compression target, so extension
        // variables and reopening are exercised.
        let (nv, xors) = grid_tseitin(6, 6);
        if let Some(v) = drat_verify_tseitin(&xors, nv) {
            assert!(v, "drat-trim must verify the 6x6 grid ER proof");
        }
    }

    #[test]
    fn tseitin_proof_wide_grid_verified() {
        // 4x20 grid: long rows force compression churn and deep reopen chains.
        let (nv, xors) = grid_tseitin(4, 20);
        if let Some(v) = drat_verify_tseitin(&xors, nv) {
            assert!(v, "drat-trim must verify the 4x20 grid ER proof");
        }
    }

    #[test]
    fn tseitin_proof_expander_verified() {
        // Deterministic random 3-regular-ish multigraph on 30 nodes with odd
        // charge: cutwidth well above the compression target, non-grid shape.
        let n = 30usize;
        let mut state: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };
        // Build edges: a cycle (ensures connectivity + every node degree >= 2)
        // plus n/2 random chords.
        let mut edges: Vec<(usize, usize)> = (0..n).map(|i| (i, (i + 1) % n)).collect();
        for _ in 0..n / 2 {
            let a = next() % n;
            let b = next() % n;
            if a != b {
                edges.push((a.min(b), a.max(b)));
            }
        }
        let mut inc: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (ei, &(a, b)) in edges.iter().enumerate() {
            inc[a].push(ei as u32 + 1);
            inc[b].push(ei as u32 + 1);
        }
        let mut xors = Vec::new();
        for (i, mut vars) in inc.into_iter().enumerate() {
            vars.sort_unstable();
            vars.dedup();
            xors.push(XorConstraint { vars, rhs: i == 0 });
        }
        // Charge is odd (exactly one rhs=true) and every edge var occurs in
        // exactly its two endpoint constraints (dedup killed none: a==b skipped).
        assert!(gaussian_unsat(&xors, edges.len()));
        if let Some(v) = drat_verify_tseitin(&xors, edges.len()) {
            assert!(v, "drat-trim must verify the expander ER proof");
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
