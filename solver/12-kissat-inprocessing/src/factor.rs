//! Bounded variable addition (BVA), a port of kissat 4.0.4 `factor.c`
//! (bead SAT-playground-5b2.3.46).
//!
//! Factoring finds a set of "factor" literals {f_1..f_m} and a set of clause
//! "rests" {D_1..D_n} such that every product clause (f_i ∨ D_j) is present in
//! the formula, then introduces a fresh variable z and replaces the m*n product
//! clauses with m + n clauses:
//!
//! ```text
//!   (z ∨ f_i)   for every factor literal f_i     ("dividers")
//!   (¬z ∨ D_j)  for every rest D_j               ("quotients")
//! ```
//!
//! The transform is equisatisfiable and model-restricting: any model of the
//! factored formula restricted to the original variables satisfies the original
//! formula, so no reconstruction stack is needed. In DRAT the divider clauses
//! are RAT on the fresh literal z (no clause contains ¬z yet), and each
//! quotient clause is RAT on ¬z because every resolvent (D_j ∨ f_i) is an
//! existing product clause; deletions follow the additions. The fresh literal
//! is emitted first in each added clause because drat-trim checks RAT on the
//! first literal.
//!
//! This is a preprocessing-only pass over the parsed formula, run before the
//! solver is constructed. Like kissat's `kissat_factoring` delay rule
//! (`log10(vars) <= eliminations + factordelay` with zero eliminations at
//! preprocessing time), the caller only invokes it on formulas with at most
//! `FACTOR_MAX_VARS` variables; the tick budget bounds the worst case.

/// Maximum clause size considered for factoring (kissat `factorsize`).
pub(crate) const FACTOR_MAX_CLAUSE_SIZE: usize = 5;
/// Candidate-clause refinement rounds (kissat `factorcandrounds`).
pub(crate) const FACTOR_CAND_ROUNDS: usize = 2;
/// Initial tick budget (kissat `factoriniticks`, 700 million ticks). This is one
/// budget *slice*: the pass keeps granting itself further slices while the last
/// slice applied at least one factoring (kissat reschedules factor with a fresh
/// effort budget each probe as long as candidate literals stay marked), up to
/// `FACTOR_SLICE_MAX` slices in total.
pub(crate) const FACTOR_TICKS_LIMIT: u64 = 700_000_000;
/// Hard cap on productivity-extended budget slices.
const FACTOR_SLICE_MAX: u64 = 16;
/// Frontend delay guard: only factor formulas with at most this many variables
/// (kissat: `log10(active) <= eliminations(0) + factordelay(4)` at preprocess
/// time, i.e. 10^4 variables).
pub(crate) const FACTOR_MAX_VARS: usize = 10_000;
/// Required clause reduction must exceed this bound. kissat uses the eliminate
/// additional-clauses bound, which escalates 0 -> 16 (`eliminatebound`) across
/// elimination rounds; this one-shot frontend pass uses the mature bound so only
/// factorings with a solid clause reduction perturb the formula.
pub(crate) const FACTOR_BOUND: usize = 16;

const MARK_FACTOR: u8 = 1;
const MARK_QUOTIENT: u8 = 2;
const MARK_NOUNTED: u8 = 4;

/// One DRAT step of the factoring transform, in emission order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FactorProofStep {
    /// Clause addition; the fresh (RAT pivot) literal is first.
    Add(Vec<i32>),
    /// Clause deletion (an original product clause).
    Delete(Vec<i32>),
}

#[derive(Debug)]
pub(crate) struct FactorOutcome {
    /// Total variable count after factoring (original + fresh).
    pub(crate) num_vars: usize,
    /// The transformed formula: surviving original clauses in input order,
    /// followed by added clauses in creation order.
    pub(crate) clauses: Vec<Vec<i32>>,
    /// Ordered DRAT steps to replay into the proof log before any solver step.
    pub(crate) steps: Vec<FactorProofStep>,
    pub(crate) fresh_vars: usize,
    pub(crate) clauses_removed: usize,
    pub(crate) clauses_added: usize,
    pub(crate) ticks: u64,
    /// False when the tick budget stopped the pass early.
    pub(crate) completed: bool,
}

struct FClause {
    lits: Vec<i32>,
    alive: bool,
    /// Per-scan "already matched" flag (kissat's `c->quotient` bit).
    matched: bool,
    /// Participates in factoring (size/cleanliness/refinement filter).
    candidate: bool,
}

/// A quotient in the factoring chain: the factor literal plus the clause ids
/// containing it, and for chain position > 0 the index of the matching clause
/// in the previous quotient's list (kissat `struct quotient`).
struct Quotient {
    factor: u32,
    clause_ids: Vec<u32>,
    matches: Vec<u32>,
}

struct Factoring {
    initial_lits: usize,
    next_var: usize,
    clauses: Vec<FClause>,
    /// Per-literal candidate clause ids; dead ids are skipped lazily.
    occ: Vec<Vec<u32>>,
    /// Per-literal live candidate count (occ minus dead entries).
    live: Vec<u32>,
    marks: Vec<u8>,
    count: Vec<u32>,
    processed: Vec<bool>,
    ticks: u64,
    limit: u64,
    bound: usize,
    steps: Vec<FactorProofStep>,
    fresh_vars: usize,
    clauses_removed: usize,
    clauses_added: usize,
}

/// Internal literal index: variable `v` (1-based) positive -> 2*(v-1),
/// negative -> 2*(v-1)+1.
fn lit_index(lit: i32) -> usize {
    let var = lit.unsigned_abs() as usize;
    2 * (var - 1) + usize::from(lit < 0)
}

fn index_lit(index: usize) -> i32 {
    let var = (index / 2 + 1) as i32;
    if index % 2 == 0 {
        var
    } else {
        -var
    }
}

fn clause_is_clean(lits: &[i32], num_vars: usize) -> bool {
    // No duplicate variables (covers tautologies) and all variables in range.
    // Candidate clauses must be plain sets so content matching is sound.
    for (i, &lit) in lits.iter().enumerate() {
        let var = lit.unsigned_abs() as usize;
        if var == 0 || var > num_vars {
            return false;
        }
        for &other in &lits[..i] {
            if other.unsigned_abs() == lit.unsigned_abs() {
                return false;
            }
        }
    }
    true
}

impl Factoring {
    fn new(num_vars: usize, input: &[Vec<i32>], limit: u64, bound: usize) -> Self {
        let num_lits = 2 * num_vars;
        let mut clauses = Vec::with_capacity(input.len());
        for lits in input {
            let candidate = lits.len() >= 2
                && lits.len() <= FACTOR_MAX_CLAUSE_SIZE
                && clause_is_clean(lits, num_vars);
            clauses.push(FClause {
                lits: lits.clone(),
                alive: true,
                matched: false,
                candidate,
            });
        }

        // Candidate refinement (kissat connect_clauses_to_factor): binaries are
        // always connected; larger clauses stay candidates only while every one
        // of their literals occurs at least twice among candidate clauses.
        let mut occ_count = vec![0u32; num_lits];
        for clause in clauses.iter().filter(|c| c.candidate) {
            for &lit in &clause.lits {
                occ_count[lit_index(lit)] += 1;
            }
        }
        for _ in 0..FACTOR_CAND_ROUNDS {
            let mut next_count = vec![0u32; num_lits];
            let mut changed = false;
            for clause in clauses.iter_mut() {
                if !clause.candidate {
                    continue;
                }
                if clause.lits.len() > 2
                    && clause
                        .lits
                        .iter()
                        .any(|&lit| occ_count[lit_index(lit)] < 2)
                {
                    clause.candidate = false;
                    changed = true;
                    continue;
                }
                for &lit in &clause.lits {
                    next_count[lit_index(lit)] += 1;
                }
            }
            occ_count = next_count;
            if !changed {
                break;
            }
        }

        let mut occ: Vec<Vec<u32>> = vec![Vec::new(); num_lits];
        let mut live = vec![0u32; num_lits];
        for (id, clause) in clauses.iter().enumerate() {
            if !clause.candidate {
                continue;
            }
            for &lit in &clause.lits {
                let index = lit_index(lit);
                occ[index].push(id as u32);
                live[index] += 1;
            }
        }

        Factoring {
            initial_lits: num_lits,
            next_var: num_vars + 1,
            clauses,
            occ,
            live,
            marks: vec![0; num_lits],
            count: vec![0; num_lits],
            processed: vec![false; num_lits],
            ticks: 0,
            limit,
            bound,
            steps: Vec::new(),
            fresh_vars: 0,
            clauses_removed: 0,
            clauses_added: 0,
        }
    }

    fn out_of_ticks(&self) -> bool {
        self.ticks > self.limit
    }

    /// For clause `id` (which must contain exactly one FACTOR-marked literal to
    /// be usable), QUOTIENT-mark the rest literals and return the rest literal
    /// with the fewest live occurrences. Returns None when the clause contains
    /// more than one factor literal (kissat `factors > 1`).
    fn mark_rest_and_min_lit(&mut self, id: u32) -> Option<usize> {
        let clause = &self.clauses[id as usize];
        let mut factors = 0usize;
        let mut min_index = usize::MAX;
        let mut min_size = u32::MAX;
        for &lit in &clause.lits {
            let index = lit_index(lit);
            if self.marks[index] & MARK_FACTOR != 0 {
                factors += 1;
                if factors > 1 {
                    break;
                }
            } else {
                self.marks[index] |= MARK_QUOTIENT;
                let size = self.live[index];
                if size < min_size {
                    min_size = size;
                    min_index = index;
                }
            }
        }
        if factors != 1 {
            self.unmark_rest(id);
            return None;
        }
        debug_assert!(min_index != usize::MAX);
        Some(min_index)
    }

    fn unmark_rest(&mut self, id: u32) {
        for &lit in &self.clauses[id as usize].lits {
            self.marks[lit_index(lit)] &= !MARK_QUOTIENT;
        }
    }

    /// Count-phase scan (kissat `next_factor`): find the literal that matches
    /// the most clauses of the last quotient, counting each source clause at
    /// most once per candidate literal.
    fn next_factor(&mut self, last: &Quotient) -> Option<(u32, u32)> {
        let mut counted: Vec<usize> = Vec::new();
        let mut qlauses: Vec<u32> = Vec::new();
        'clauses: for &c_id in &last.clause_ids {
            self.ticks += 1;
            let Some(min_index) = self.mark_rest_and_min_lit(c_id) else {
                continue;
            };
            let c_len = self.clauses[c_id as usize].lits.len();
            let mut nounted: Vec<usize> = Vec::new();
            self.ticks += 1 + self.occ[min_index].len() as u64 / 8;
            for scan in 0..self.occ[min_index].len() {
                let d_id = self.occ[min_index][scan];
                if d_id == c_id {
                    continue;
                }
                let d = &self.clauses[d_id as usize];
                self.ticks += 1;
                if !d.alive || d.matched || d.lits.len() != c_len {
                    continue;
                }
                let mut next_index = usize::MAX;
                let mut ok = true;
                for &lit in &d.lits {
                    let index = lit_index(lit);
                    let mark = self.marks[index];
                    if mark & MARK_QUOTIENT != 0 {
                        continue;
                    }
                    if mark & (MARK_FACTOR | MARK_NOUNTED) != 0 || next_index != usize::MAX {
                        ok = false;
                        break;
                    }
                    next_index = index;
                }
                if !ok || next_index == usize::MAX {
                    continue;
                }
                if next_index >= self.initial_lits {
                    continue;
                }
                self.marks[next_index] |= MARK_NOUNTED;
                nounted.push(next_index);
                self.clauses[d_id as usize].matched = true;
                qlauses.push(d_id);
                if self.count[next_index] == 0 {
                    counted.push(next_index);
                }
                self.count[next_index] += 1;
            }
            for index in nounted {
                self.marks[index] &= !MARK_NOUNTED;
            }
            self.unmark_rest(c_id);
            if self.out_of_ticks() {
                break 'clauses;
            }
        }
        for id in qlauses {
            self.clauses[id as usize].matched = false;
        }

        let mut next_index = usize::MAX;
        let mut next_count = 0u32;
        let mut next_score = 0u32;
        if !self.out_of_ticks() {
            // Max count wins; ties by live-occurrence score, first-discovered
            // on equal score (kissat watches_score tie-break).
            for &index in &counted {
                let count = self.count[index];
                if count < next_count {
                    continue;
                }
                let score = self.live[index];
                if count > next_count || (next_index != usize::MAX && score > next_score) {
                    next_count = count;
                    next_score = score;
                    next_index = index;
                }
            }
        }
        for index in counted {
            self.count[index] = 0;
        }
        if next_index == usize::MAX || next_count < 2 {
            return None;
        }
        Some((next_index as u32, next_count))
    }

    /// Build-phase scan (kissat `factorize_next`): collect, for each clause of
    /// the last quotient, the clause that matches it on `next`.
    fn factorize_next(&mut self, last: &Quotient, next_index: usize) -> Quotient {
        let mut clause_ids: Vec<u32> = Vec::new();
        let mut matches: Vec<u32> = Vec::new();
        let mut qlauses: Vec<u32> = Vec::new();
        for (i, &c_id) in last.clause_ids.iter().enumerate() {
            self.ticks += 1;
            let Some(min_index) = self.mark_rest_and_min_lit(c_id) else {
                continue;
            };
            let c_len = self.clauses[c_id as usize].lits.len();
            self.ticks += 1 + self.occ[min_index].len() as u64 / 8;
            for scan in 0..self.occ[min_index].len() {
                let d_id = self.occ[min_index][scan];
                if d_id == c_id {
                    continue;
                }
                let d = &self.clauses[d_id as usize];
                self.ticks += 1;
                if !d.alive || d.matched || d.lits.len() != c_len {
                    continue;
                }
                let mut ok = true;
                for &lit in &d.lits {
                    let index = lit_index(lit);
                    if self.marks[index] & MARK_QUOTIENT != 0 {
                        continue;
                    }
                    if index != next_index {
                        ok = false;
                        break;
                    }
                }
                if !ok {
                    continue;
                }
                self.clauses[d_id as usize].matched = true;
                qlauses.push(d_id);
                clause_ids.push(d_id);
                matches.push(i as u32);
                break;
            }
            self.unmark_rest(c_id);
        }
        for id in qlauses {
            self.clauses[id as usize].matched = false;
        }
        Quotient {
            factor: next_index as u32,
            clause_ids,
            matches,
        }
    }

    /// Pick the chain prefix with the largest clause reduction
    /// (kissat `best_quotient`); returns (chain index, reduction).
    fn best_quotient(chain: &[Quotient]) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize)> = None;
        for (depth, quotient) in chain.iter().enumerate() {
            let factors = depth + 1;
            let quotients = quotient.clause_ids.len();
            let before = quotients * factors;
            let after = quotients + factors;
            if before > after {
                let reduction = before - after;
                if best.map_or(true, |(_, best_reduction)| best_reduction < reduction) {
                    best = Some((depth, reduction));
                }
            }
        }
        best
    }

    /// Align the chain clause lists so position i in every quotient refers to
    /// the same rest (kissat `flush_unmatched_clauses`).
    fn flush_unmatched(chain: &mut [Quotient], chosen: usize) {
        for p in (1..=chosen).rev() {
            let (head, tail) = chain.split_at_mut(p);
            let cur = &tail[0];
            let prev = &mut head[p - 1];
            let n = cur.clause_ids.len();
            debug_assert_eq!(n, cur.matches.len());
            for i in 0..n {
                let j = cur.matches[i] as usize;
                debug_assert!(i <= j);
                if p > 1 {
                    prev.matches[i] = prev.matches[j];
                }
                prev.clause_ids[i] = prev.clause_ids[j];
            }
            prev.clause_ids.truncate(n);
            if p > 1 {
                prev.matches.truncate(n);
            }
        }
    }

    fn add_clause(&mut self, lits: Vec<i32>) {
        // Added clauses (dividers and quotients) are candidate-sized by
        // construction and participate in later factorings like kissat's
        // freshly connected clauses.
        let id = self.clauses.len() as u32;
        for &lit in &lits {
            let index = lit_index(lit);
            self.occ[index].push(id);
            self.live[index] += 1;
        }
        self.steps.push(FactorProofStep::Add(lits.clone()));
        self.clauses.push(FClause {
            lits,
            alive: true,
            matched: false,
            candidate: true,
        });
        self.clauses_added += 1;
    }

    fn delete_clause(&mut self, id: u32) {
        let clause = &mut self.clauses[id as usize];
        debug_assert!(clause.alive);
        clause.alive = false;
        let lits = clause.lits.clone();
        for &lit in &lits {
            let index = lit_index(lit);
            self.live[index] -= 1;
            // Eagerly remove the id so occurrence scans never walk dead
            // clauses (kissat `eagerly_remove_watch`).
            let occ = &mut self.occ[index];
            if let Some(pos) = occ.iter().position(|&entry| entry == id) {
                occ.swap_remove(pos);
            }
        }
        self.steps.push(FactorProofStep::Delete(lits));
        self.clauses_removed += 1;
    }

    /// Apply the chosen factoring (kissat `apply_factoring`): add dividers and
    /// quotient clauses for a fresh variable, then delete the product clauses.
    /// Returns the literals whose candidate scores changed.
    fn apply_factoring(&mut self, chain: &mut Vec<Quotient>, chosen: usize) -> Vec<usize> {
        Self::flush_unmatched(chain, chosen);
        let fresh_var = self.next_var;
        self.next_var += 1;
        self.fresh_vars += 1;
        let fresh = fresh_var as i32;
        // Grow per-literal arrays for the fresh variable. The fresh literals
        // are marked processed so they are never scheduled as `first`, and the
        // `initial_lits` guard keeps them out of `next` selection.
        for _ in 0..2 {
            self.occ.push(Vec::new());
            self.live.push(0);
            self.marks.push(0);
            self.count.push(0);
            self.processed.push(true);
        }

        let mut touched: Vec<usize> = Vec::new();
        // Dividers (z ∨ f_p), fresh literal first (RAT pivot).
        for quotient in chain[..=chosen].iter() {
            let factor_lit = index_lit(quotient.factor as usize);
            touched.push(quotient.factor as usize);
            self.add_clause(vec![fresh, factor_lit]);
        }
        // Quotient clauses (¬z ∨ rest), fresh literal first (RAT pivot). The
        // rests are read from the chosen quotient's clause list.
        let rests: Vec<Vec<i32>> = chain[chosen]
            .clause_ids
            .iter()
            .map(|&id| {
                let factor_lit = index_lit(chain[chosen].factor as usize);
                self.clauses[id as usize]
                    .lits
                    .iter()
                    .copied()
                    .filter(|&lit| lit != factor_lit)
                    .collect()
            })
            .collect();
        for rest in rests {
            let mut lits = Vec::with_capacity(rest.len() + 1);
            lits.push(-fresh);
            for &lit in &rest {
                touched.push(lit_index(lit));
                lits.push(lit);
            }
            self.add_clause(lits);
        }
        // Delete the factored product clauses.
        for quotient in chain[..=chosen].iter() {
            for &id in &quotient.clause_ids {
                debug_assert!(self.clauses[id as usize].alive);
            }
        }
        let mut to_delete: Vec<u32> = Vec::new();
        for quotient in chain[..=chosen].iter() {
            to_delete.extend_from_slice(&quotient.clause_ids);
        }
        for id in to_delete {
            self.delete_clause(id);
        }
        touched.sort_unstable();
        touched.dedup();
        touched
    }

    fn run(&mut self) -> bool {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        let mut heap: BinaryHeap<(u32, Reverse<usize>)> = BinaryHeap::new();
        for index in 0..self.initial_lits {
            if self.live[index] > 1 {
                heap.push((self.live[index], Reverse(index)));
            }
        }

        let slice = self.limit;
        let absolute_cap = slice.saturating_mul(FACTOR_SLICE_MAX);
        let mut fresh_at_checkpoint = self.fresh_vars;
        while let Some((score, Reverse(first_index))) = heap.pop() {
            if self.ticks >= absolute_cap {
                return false;
            }
            if self.out_of_ticks() {
                // Grant another budget slice only while the pass keeps paying
                // for itself; an unproductive slice ends the run like kissat's
                // single exhausted effort budget.
                if self.fresh_vars == fresh_at_checkpoint {
                    return false;
                }
                fresh_at_checkpoint = self.fresh_vars;
                self.limit = self.limit.saturating_add(slice);
            }
            if self.processed[first_index] || self.live[first_index] < 2 {
                continue;
            }
            if score != self.live[first_index] {
                heap.push((self.live[first_index], Reverse(first_index)));
                continue;
            }
            self.processed[first_index] = true;

            // Build quotient[0]: the live candidate clauses containing `first`.
            debug_assert_eq!(self.marks[first_index], 0);
            self.marks[first_index] = MARK_FACTOR;
            let clause_ids: Vec<u32> = self.occ[first_index]
                .iter()
                .copied()
                .filter(|&id| self.clauses[id as usize].alive)
                .collect();
            self.ticks += 1 + clause_ids.len() as u64 / 8;
            let mut chain = vec![Quotient {
                factor: first_index as u32,
                clause_ids,
                matches: Vec::new(),
            }];

            if chain[0].clause_ids.len() > 1 {
                loop {
                    let Some((next, _count)) = self.next_factor(chain.last().unwrap()) else {
                        break;
                    };
                    let quotient = self.factorize_next(chain.last().unwrap(), next as usize);
                    debug_assert_eq!(self.marks[next as usize], 0);
                    self.marks[next as usize] = MARK_FACTOR;
                    chain.push(quotient);
                }
                if let Some((chosen, reduction)) = Self::best_quotient(&chain) {
                    if reduction > self.bound {
                        let touched = self.apply_factoring(&mut chain, chosen);
                        for index in touched {
                            if !self.processed[index] && self.live[index] > 1 {
                                heap.push((self.live[index], Reverse(index)));
                            }
                        }
                    }
                }
            }

            for quotient in &chain {
                debug_assert!(self.marks[quotient.factor as usize] & MARK_FACTOR != 0);
                self.marks[quotient.factor as usize] &= !MARK_FACTOR;
            }
        }
        true
    }
}

/// Run bounded variable addition over a parsed formula. Returns None when the
/// pass is a no-op (nothing factored), so the caller can skip re-plumbing.
pub(crate) fn factor_formula(
    num_vars: usize,
    clauses: &[Vec<i32>],
    tick_limit: u64,
    bound: usize,
) -> Option<FactorOutcome> {
    if num_vars == 0 || clauses.is_empty() {
        return None;
    }
    if num_vars > (i32::MAX as usize) / 2 {
        return None;
    }
    let mut factoring = Factoring::new(num_vars, clauses, tick_limit, bound);
    let completed = factoring.run();
    if factoring.fresh_vars == 0 {
        return None;
    }
    let transformed: Vec<Vec<i32>> = factoring
        .clauses
        .iter()
        .filter(|clause| clause.alive)
        .map(|clause| clause.lits.clone())
        .collect();
    Some(FactorOutcome {
        num_vars: factoring.next_var - 1,
        clauses: transformed,
        steps: std::mem::take(&mut factoring.steps),
        fresh_vars: factoring.fresh_vars,
        clauses_removed: factoring.clauses_removed,
        clauses_added: factoring.clauses_added,
        ticks: factoring.ticks,
        completed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assignments_satisfying(num_vars: usize, clauses: &[Vec<i32>]) -> Vec<u32> {
        let mut result = Vec::new();
        for bits in 0u32..(1 << num_vars) {
            let ok = clauses.iter().all(|clause| {
                clause.iter().any(|&lit| {
                    let var = lit.unsigned_abs() as usize;
                    let value = bits >> (var - 1) & 1 == 1;
                    (lit > 0) == value
                })
            });
            if ok {
                result.push(bits);
            }
        }
        result
    }

    #[test]
    fn factors_two_by_three_grid() {
        // 2 factor literals x 3 rests: 6 clauses -> 5 (reduction 1).
        let clauses = vec![
            vec![1, 3, 4],
            vec![1, 5, 6],
            vec![1, 7, 8],
            vec![2, 3, 4],
            vec![2, 5, 6],
            vec![2, 7, 8],
        ];
        let outcome = factor_formula(8, &clauses, u64::MAX, 0).expect("factoring applies");
        assert_eq!(outcome.num_vars, 9);
        assert_eq!(outcome.fresh_vars, 1);
        assert_eq!(outcome.clauses_removed, 6);
        assert_eq!(outcome.clauses_added, 5);
        assert!(outcome.completed);
        assert_eq!(outcome.clauses.len(), 5);

        // Two dividers (9 ∨ f) and three quotients (-9 ∨ D).
        let dividers: Vec<_> = outcome
            .clauses
            .iter()
            .filter(|clause| clause.contains(&9))
            .collect();
        let quotients: Vec<_> = outcome
            .clauses
            .iter()
            .filter(|clause| clause.contains(&-9))
            .collect();
        assert_eq!(dividers.len(), 2);
        assert_eq!(quotients.len(), 3);

        // Proof: additions first (pivot literal first in each), then deletions.
        let adds: Vec<_> = outcome
            .steps
            .iter()
            .take_while(|step| matches!(step, FactorProofStep::Add(_)))
            .collect();
        assert_eq!(adds.len(), 5);
        for step in &adds {
            let FactorProofStep::Add(lits) = step else {
                unreachable!()
            };
            assert_eq!(lits[0].unsigned_abs(), 9);
        }
        let deletes = outcome.steps.len() - adds.len();
        assert_eq!(deletes, 6);
    }

    #[test]
    fn no_factoring_on_two_by_two_grid() {
        // 2x2: 4 clauses -> 4, reduction zero, must not factor.
        let clauses = vec![
            vec![1, 3, 4],
            vec![1, 5, 6],
            vec![2, 3, 4],
            vec![2, 5, 6],
        ];
        assert!(factor_formula(6, &clauses, u64::MAX, 0).is_none());
    }

    #[test]
    fn factors_binary_only_grid() {
        // Binary clauses: 3 factor literals x 2 rests over single literals.
        // 6 binaries -> 3 dividers + 2 quotient binaries (reduction 1).
        let clauses = vec![
            vec![1, 4],
            vec![1, 5],
            vec![2, 4],
            vec![2, 5],
            vec![3, 4],
            vec![3, 5],
        ];
        let outcome = factor_formula(5, &clauses, u64::MAX, 0).expect("factoring applies");
        assert_eq!(outcome.fresh_vars, 1);
        assert_eq!(outcome.clauses.len(), 5);
    }

    #[test]
    fn tick_limit_stops_factoring() {
        let clauses = vec![
            vec![1, 3, 4],
            vec![1, 5, 6],
            vec![1, 7, 8],
            vec![2, 3, 4],
            vec![2, 5, 6],
            vec![2, 7, 8],
        ];
        // Zero budget: the pass starts over budget and must be a no-op.
        assert!(factor_formula(8, &clauses, 0, 0).is_none());
    }

    #[test]
    fn skips_dirty_clauses() {
        // Tautologies and duplicate literals never participate.
        let clauses = vec![
            vec![1, -1, 3],
            vec![2, 2, 4],
            vec![1, 3, 4],
            vec![2, 3, 4],
        ];
        assert!(factor_formula(4, &clauses, u64::MAX, 0).is_none());
    }

    #[test]
    fn model_restriction_preserves_original_models() {
        // Every model of the factored formula, restricted to the original
        // variables, must satisfy the original formula, and the original and
        // factored formulas must agree on satisfiability of every projection.
        let clauses = vec![
            vec![1, 3, 4],
            vec![1, 5, 6],
            vec![1, 7, 8],
            vec![2, 3, 4],
            vec![2, 5, 6],
            vec![2, 7, 8],
            vec![-1, -2],
            vec![-3, 5],
        ];
        let num_vars = 8;
        let outcome = factor_formula(num_vars, &clauses, u64::MAX, 0).expect("factoring applies");
        let original_models = assignments_satisfying(num_vars, &clauses);
        let factored_models = assignments_satisfying(outcome.num_vars, &outcome.clauses);

        // Restriction of factored models to original vars ⊆ original models.
        let mask = (1u32 << num_vars) - 1;
        let mut restricted: Vec<u32> = factored_models.iter().map(|m| m & mask).collect();
        restricted.sort_unstable();
        restricted.dedup();
        for model in &restricted {
            assert!(original_models.contains(model));
        }
        // Every original model extends to a factored model.
        assert_eq!(restricted, original_models);
    }

    #[test]
    fn chains_three_factors() {
        // 3 factor literals x 3 rests: 9 clauses -> 6 (reduction 3).
        let mut clauses = Vec::new();
        for factor in 1..=3 {
            for rest in 0..3 {
                clauses.push(vec![factor, 4 + rest * 2, 5 + rest * 2]);
            }
        }
        let outcome = factor_formula(9, &clauses, u64::MAX, 0).expect("factoring applies");
        assert_eq!(outcome.fresh_vars, 1);
        assert_eq!(outcome.clauses_removed, 9);
        assert_eq!(outcome.clauses_added, 6);
    }
}
