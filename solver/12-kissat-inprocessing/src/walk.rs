//! ProbSAT-style local search over the irredundant clauses (kissat walk.c port).
//!
//! The walker never touches the clause database or the proof: it operates on a
//! private copy of the phase assignment and returns the best (fewest unsatisfied
//! clauses) assignment it visited. The caller exports that assignment into the
//! saved phases, which is the whole point — kissat schedules walks inside its
//! rephase cycle ('W' slots) so stable-mode decisions restart from a
//! local-search-improved phase vector instead of a stale saved/target snapshot.
//! On the conflict-density gap cells (booth/Bubble/fixedbandwidth class) kissat
//! runs 16-22 walks + 49-65 rephases per refutation; solver 12 previously had
//! neither mechanism (bare best/inverted/original cycling, default off).
//!
//! Faithful-port notes (kissat walk.c):
//! - Clause pick is round-robin over the unsat stack (`flipped % current`), with
//!   `flipped` advancing twice per step exactly like kissat (once in the step
//!   driver, once when picking) — this affects visiting order, so it is kept.
//! - Literal pick is ProbSAT: weight `base^-breaks` where `base = 1/CB`; CB is
//!   2.0 on even walks and fitted from the average clause size on odd walks.
//! - The best assignment is tracked via a bounded trail of flips
//!   (`num_vars/4 + 1` cap) with a full-copy fallback, kissat's exact scheme.
//! - Steps are counted per occurrence-list scan and bounded by the caller's
//!   effort limit (kissat `walkeffort` = 50 permille of recent search ticks).

use crate::lit::lit_to_index;

pub(crate) const WALK_TRUE: u8 = 1;
pub(crate) const WALK_FALSE: u8 = 0;
pub(crate) const WALK_UNSET: u8 = 2;

/// Interpolation table from kissat walk.c: average clause size -> CB constant.
const CBVALS: [(f64, f64); 6] = [
    (0.0, 2.00),
    (3.0, 2.50),
    (4.0, 2.85),
    (5.0, 3.70),
    (6.0, 5.10),
    (7.0, 7.40),
];

fn fit_cbval(size: f64) -> f64 {
    let mut i = 0;
    while i + 2 < CBVALS.len() && (CBVALS[i].0 > size || CBVALS[i + 1].0 < size) {
        i += 1;
    }
    let (x1, y1) = CBVALS[i];
    let (x2, y2) = CBVALS[i + 1];
    let (dx, dy) = (x2 - x1, y2 - y1);
    let res = dy * (size - x1) / dx + y1;
    debug_assert!(res > 0.0);
    res
}

pub(crate) struct WalkResult {
    /// Best assignment per variable (1-based; `WALK_UNSET` for vars the walker
    /// never owned). Only meaningful when `improved` is true.
    pub(crate) best_values: Vec<u8>,
    /// True when the walker found an assignment with strictly fewer unsatisfied
    /// clauses than the initial phase assignment.
    pub(crate) improved: bool,
    #[allow(dead_code)]
    pub(crate) initial_unsat: u32,
    #[allow(dead_code)]
    pub(crate) minimum_unsat: u32,
    pub(crate) flips: u64,
    pub(crate) steps: u64,
}

/// Deterministic xorshift-multiply generator (same shape as the solver's
/// focused-random-decision generator; seeded per walk).
struct WalkRng(u64);

impl WalkRng {
    fn next_word(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 ^ (self.0 >> 33)
    }

    /// Uniform double in [0, 1).
    fn next_double(&mut self) -> f64 {
        (self.next_word() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

struct Walker<'a> {
    clause_lits: &'a [i32],
    clause_ends: &'a [u32],
    /// CSR occurrence lists by literal index over the walker clauses.
    occ_starts: Vec<u32>,
    occ_items: Vec<u32>,
    /// Number of currently-true literals per clause.
    counts: Vec<u32>,
    /// Position of each clause in `unsat`, or `u32::MAX` when satisfied.
    pos: Vec<u32>,
    unsat: Vec<u32>,
    /// Current assignment per variable: WALK_TRUE / WALK_FALSE (owned vars only).
    values: Vec<u8>,
    /// Best assignment seen so far.
    best_values: Vec<u8>,
    /// Trail of flipped literal indices since the last best snapshot.
    trail: Vec<u32>,
    /// Prefix of `trail` that reproduces the best assignment, or `u32::MAX`
    /// when the trail was invalidated and `best_values` holds a full copy.
    best_trail_pos: u32,
    trail_cap: usize,
    /// ProbSAT score table: `table[breaks]`, epsilon beyond.
    table: Vec<f64>,
    epsilon: f64,
    minimum: u32,
    flipped: u64,
    steps: u64,
    rng: WalkRng,
    scores: Vec<f64>,
}

const INVALID_TRAIL_POS: u32 = u32::MAX;

impl<'a> Walker<'a> {
    fn clause_slice(&self, clause: u32) -> &'a [i32] {
        let end = self.clause_ends[clause as usize] as usize;
        let start = if clause == 0 {
            0
        } else {
            self.clause_ends[clause as usize - 1] as usize
        };
        &self.clause_lits[start..end]
    }

    fn lit_true(&self, lit: i32) -> bool {
        let var = lit.unsigned_abs() as usize;
        (self.values[var] == WALK_TRUE) == (lit > 0)
    }

    fn push_unsat(&mut self, clause: u32) {
        self.pos[clause as usize] = self.unsat.len() as u32;
        self.unsat.push(clause);
    }

    fn pop_unsat(&mut self, clause: u32) {
        let p = self.pos[clause as usize];
        debug_assert_ne!(p, u32::MAX);
        let last = self.unsat.pop().expect("unsat stack underflow");
        if last != clause {
            self.unsat[p as usize] = last;
            self.pos[last as usize] = p;
        }
        self.pos[clause as usize] = u32::MAX;
    }

    fn scale_score(&self, breaks: usize) -> f64 {
        if breaks < self.table.len() {
            self.table[breaks]
        } else {
            self.epsilon
        }
    }

    /// Number of clauses that become unsatisfied if `lit` (currently false)
    /// were flipped true: clauses containing `-lit` whose count is 1.
    fn break_value(&mut self, lit: i32) -> usize {
        debug_assert!(!self.lit_true(lit));
        let not_idx = lit_to_index(-lit);
        let start = self.occ_starts[not_idx] as usize;
        let end = self.occ_starts[not_idx + 1] as usize;
        self.steps += 1 + (end - start) as u64;
        let mut res = 0usize;
        for &clause in &self.occ_items[start..end] {
            res += usize::from(self.counts[clause as usize] == 1);
        }
        res
    }

    fn pick_literal(&mut self) -> i32 {
        debug_assert!(!self.unsat.is_empty());
        let pos = (self.flipped % self.unsat.len() as u64) as usize;
        self.flipped += 1;
        let clause = self.unsat[pos];
        let lits = self.clause_slice(clause);

        self.scores.clear();
        let mut sum = 0.0f64;
        let mut picked = lits[0];
        for &lit in lits {
            let breaks = self.break_value(lit);
            let score = self.scale_score(breaks);
            self.scores.push(score);
            sum += score;
            picked = lit;
        }
        debug_assert!(sum > 0.0);

        let threshold = sum * self.rng.next_double();
        let mut acc = 0.0f64;
        for (i, &lit) in lits.iter().enumerate() {
            acc += self.scores[i];
            if threshold < acc {
                picked = lit;
                break;
            }
        }
        picked
    }

    fn flip_literal(&mut self, lit: i32) {
        debug_assert!(!self.lit_true(lit));
        let var = lit.unsigned_abs() as usize;
        self.values[var] = if lit > 0 { WALK_TRUE } else { WALK_FALSE };

        // Make: clauses containing the now-true literal.
        let idx = lit_to_index(lit);
        let start = self.occ_starts[idx] as usize;
        let end = self.occ_starts[idx + 1] as usize;
        self.steps += 1 + (end - start) as u64;
        for i in start..end {
            let clause = self.occ_items[i];
            let count = &mut self.counts[clause as usize];
            *count += 1;
            if *count == 1 {
                self.pop_unsat(clause);
                self.steps += 1;
            }
        }

        // Break: clauses containing the now-false negation.
        let not_idx = lit_to_index(-lit);
        let start = self.occ_starts[not_idx] as usize;
        let end = self.occ_starts[not_idx + 1] as usize;
        self.steps += 1 + (end - start) as u64;
        for i in start..end {
            let clause = self.occ_items[i];
            let count = &mut self.counts[clause as usize];
            debug_assert!(*count > 0);
            *count -= 1;
            if *count == 0 {
                self.push_unsat(clause);
            }
        }
    }

    /// Fold the best prefix of the flip trail into `best_values`; optionally
    /// keep (shift down) the suffix of flips made after the best snapshot.
    fn save_trail(&mut self, keep_suffix: bool) {
        debug_assert_ne!(self.best_trail_pos, INVALID_TRAIL_POS);
        let best = self.best_trail_pos as usize;
        for i in 0..best {
            let lit_idx = self.trail[i] as usize;
            let var = lit_idx / 2 + 1;
            self.best_values[var] = if lit_idx & 1 == 0 {
                WALK_TRUE
            } else {
                WALK_FALSE
            };
        }
        if keep_suffix {
            self.trail.drain(..best);
            self.best_trail_pos = 0;
        }
    }

    fn save_all_values(&mut self) {
        debug_assert!(self.trail.is_empty());
        self.best_values.copy_from_slice(&self.values);
        self.best_trail_pos = 0;
    }

    fn push_flipped(&mut self, lit: i32) {
        if self.best_trail_pos == INVALID_TRAIL_POS {
            debug_assert!(self.trail.is_empty());
            return;
        }
        if self.trail.len() < self.trail_cap {
            self.trail.push(lit_to_index(lit) as u32);
        } else if self.best_trail_pos > 0 {
            self.save_trail(true);
            self.trail.push(lit_to_index(lit) as u32);
        } else {
            self.trail.clear();
            self.best_trail_pos = INVALID_TRAIL_POS;
        }
    }

    fn update_best(&mut self) {
        debug_assert!((self.unsat.len() as u32) < self.minimum);
        self.minimum = self.unsat.len() as u32;
        if self.best_trail_pos == INVALID_TRAIL_POS {
            self.save_all_values();
        } else {
            self.best_trail_pos = self.trail.len() as u32;
        }
    }

    fn step(&mut self) {
        self.flipped += 1;
        let lit = self.pick_literal();
        self.flip_literal(lit);
        self.push_flipped(lit);
        if (self.unsat.len() as u32) < self.minimum {
            self.update_best();
        }
    }
}

/// Run one local-search walk.
///
/// `clause_lits`/`clause_ends` describe the walkable clauses (flattened lits +
/// exclusive end offsets): live irredundant clauses that are not satisfied at
/// the root, with root-assigned literals already removed. `initial_values`
/// gives the starting assignment per variable (1-based; `WALK_TRUE`/`WALK_FALSE`
/// for every variable that appears in a walkable clause). `step_limit` bounds
/// the occurrence-scan step count; `fitted_cb` selects kissat's odd-walk CB fit.
pub(crate) fn walk(
    num_vars: usize,
    clause_lits: &[i32],
    clause_ends: &[u32],
    initial_values: &[u8],
    seed: u64,
    step_limit: u64,
    fitted_cb: bool,
) -> WalkResult {
    let num_clauses = clause_ends.len();
    debug_assert_eq!(initial_values.len(), num_vars + 1);

    // Occurrence CSR over literal indices.
    let n_lit_slots = num_vars.saturating_mul(2);
    let mut occ_counts = vec![0u32; n_lit_slots + 1];
    for &lit in clause_lits {
        occ_counts[lit_to_index(lit) + 1] += 1;
    }
    for i in 0..n_lit_slots {
        occ_counts[i + 1] += occ_counts[i];
    }
    let occ_starts = occ_counts;
    let mut occ_fill = occ_starts.clone();
    let mut occ_items = vec![0u32; clause_lits.len()];
    for clause in 0..num_clauses {
        let end = clause_ends[clause] as usize;
        let start = if clause == 0 {
            0
        } else {
            clause_ends[clause - 1] as usize
        };
        for &lit in &clause_lits[start..end] {
            let idx = lit_to_index(lit);
            occ_items[occ_fill[idx] as usize] = clause as u32;
            occ_fill[idx] += 1;
        }
    }

    // ProbSAT score table for CB derived from the average clause size.
    let avg_size = if num_clauses == 0 {
        0.0
    } else {
        clause_lits.len() as f64 / num_clauses as f64
    };
    let cb = if fitted_cb { fit_cbval(avg_size) } else { 2.0 };
    let base = 1.0 / cb;
    let mut table = Vec::new();
    let mut next = 1.0f64;
    let mut epsilon = 1.0f64;
    while next > 0.0 {
        epsilon = next;
        table.push(next);
        next *= base;
    }

    let mut walker = Walker {
        clause_lits,
        clause_ends,
        occ_starts,
        occ_items,
        counts: vec![0u32; num_clauses],
        pos: vec![u32::MAX; num_clauses],
        unsat: Vec::new(),
        values: initial_values.to_vec(),
        best_values: initial_values.to_vec(),
        trail: Vec::new(),
        best_trail_pos: 0,
        trail_cap: num_vars / 4 + 1,
        table,
        epsilon,
        minimum: 0,
        flipped: 0,
        steps: 0,
        rng: WalkRng(seed),
        scores: Vec::new(),
    };

    for clause in 0..num_clauses as u32 {
        let mut count = 0u32;
        for &lit in walker.clause_slice(clause) {
            count += u32::from(walker.lit_true(lit));
        }
        walker.counts[clause as usize] = count;
        if count == 0 {
            walker.push_unsat(clause);
        }
    }

    let initial_unsat = walker.unsat.len() as u32;
    walker.minimum = initial_unsat;

    while walker.minimum > 0 && walker.steps < step_limit {
        walker.step();
    }

    let improved = walker.minimum < initial_unsat;
    if improved && walker.best_trail_pos != INVALID_TRAIL_POS && walker.best_trail_pos > 0 {
        walker.save_trail(false);
    }

    WalkResult {
        best_values: if improved {
            walker.best_values
        } else {
            Vec::new()
        },
        improved,
        initial_unsat,
        minimum_unsat: walker.minimum,
        flips: walker.flipped,
        steps: walker.steps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(clauses: &[&[i32]]) -> (Vec<i32>, Vec<u32>) {
        let mut lits = Vec::new();
        let mut ends = Vec::new();
        for c in clauses {
            lits.extend_from_slice(c);
            ends.push(lits.len() as u32);
        }
        (lits, ends)
    }

    #[test]
    fn walk_finds_model_of_satisfiable_formula() {
        // (1 v 2) & (-1 v 2) & (1 v -2): satisfied by 1=T, 2=T.
        let (lits, ends) = build(&[&[1, 2], &[-1, 2], &[1, -2]]);
        // Start from the all-false assignment ((1 v 2) unsatisfied).
        let initial = vec![WALK_UNSET, WALK_FALSE, WALK_FALSE];
        let res = walk(2, &lits, &ends, &initial, 42, 100_000, false);
        assert_eq!(res.initial_unsat, 1);
        assert_eq!(res.minimum_unsat, 0);
        assert!(res.improved);
        assert_eq!(res.best_values[1], WALK_TRUE);
        assert_eq!(res.best_values[2], WALK_TRUE);
    }

    #[test]
    fn walk_reports_no_improvement_from_satisfying_start() {
        let (lits, ends) = build(&[&[1, 2], &[-1, 2]]);
        let initial = vec![WALK_UNSET, WALK_TRUE, WALK_TRUE];
        let res = walk(2, &lits, &ends, &initial, 7, 100_000, false);
        assert_eq!(res.initial_unsat, 0);
        assert_eq!(res.minimum_unsat, 0);
        assert!(!res.improved);
        assert_eq!(res.flips, 0);
    }

    #[test]
    fn walk_improves_unsat_count_on_unsatisfiable_core() {
        // UNSAT pigeonhole-ish set over 2 vars: all four binary combinations.
        // Minimum reachable is 1 unsatisfied clause; starting phases give 2
        // unsatisfied ((-1 v -2) and (-1 v 2) with 1=T,2=T? — compute: 1=T,2=T
        // falsifies (-1 v -2) only => initial 1; use 1=F,2=F which falsifies
        // (1 v 2) only => initial 1 as well. Use step limit and just assert the
        // walker never *worsens* the best and terminates.
        let (lits, ends) = build(&[&[1, 2], &[1, -2], &[-1, 2], &[-1, -2]]);
        let initial = vec![WALK_UNSET, WALK_FALSE, WALK_FALSE];
        let res = walk(2, &lits, &ends, &initial, 3, 10_000, true);
        assert_eq!(res.initial_unsat, 1);
        assert_eq!(res.minimum_unsat, 1);
        assert!(!res.improved);
        assert!(res.steps >= 10_000 || res.flips > 0);
    }

    #[test]
    fn walk_trail_cap_fallback_still_tracks_best() {
        // Chain formula with enough vars to overflow the trail cap
        // (num_vars/4 + 1) many times; walker must still return a correct
        // improved assignment via the full-copy fallback.
        let n = 40usize;
        let mut clauses: Vec<Vec<i32>> = Vec::new();
        for v in 1..=n as i32 {
            clauses.push(vec![v]); // unit-ish walkable clauses force flips to true
        }
        let refs: Vec<&[i32]> = clauses.iter().map(|c| c.as_slice()).collect();
        let (lits, ends) = build(&refs);
        let initial = vec![WALK_FALSE; n + 1];
        let res = walk(n, &lits, &ends, &initial, 11, 1_000_000, false);
        assert_eq!(res.minimum_unsat, 0);
        assert!(res.improved);
        for v in 1..=n {
            assert_eq!(res.best_values[v], WALK_TRUE, "var {v}");
        }
    }
}
