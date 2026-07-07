//! Embedded "kitten" sub-solver for SAT-sweeping (bead SAT-playground-5b2.3.38).
//!
//! A small, dependency-free CDCL solver used by the sweeping inprocessor to reason
//! about a *bounded* sub-formula (kissat caps environments at <=256 vars / <=1024
//! clauses). It must:
//!   - solve the sub-formula completely (never return UNKNOWN within budget),
//!   - solve incrementally under a set of assumption literals (used to prove backbone
//!     units and literal equivalences), and
//!   - on UNSAT, expose a *clausal core*: the subset of the input clauses that
//!     participate in the refutation, which the outer solver turns into RUP lemmas so
//!     the proven fact (unit / equivalence binary) is DRAT-sound.
//!
//! This is a from-scratch Rust CDCL — correctness first, not kissat parity. The core
//! extractor walks the reason graph from the final conflict down to input clauses
//! (kissat's kitten does clausal-core extraction; the 4.0.4 trail-ordered walk is the
//! correct reference — the sc2024 variant produced wrong cores).
//!
//! Phase 1 of the sweeping port: this module is standalone and unit-tested in
//! isolation; it is not yet wired into the search loop (that is Phases 2-5), so its
//! public surface is intentionally unused for now.
#![allow(dead_code)]

/// Literal encoding inside kitten: internal variables are 0-based; a literal is
/// `2*var + sign` where sign 0 = positive, 1 = negative. Callers use signed DIMACS
/// literals at the boundary (`add_clause`, `solve`) and kitten remaps them.
type Lit = u32;

#[inline]
fn lit_from_dimacs(d: i32) -> Lit {
    debug_assert!(d != 0);
    let v = (d.unsigned_abs() - 1) as u32;
    (v << 1) | ((d < 0) as u32)
}

#[inline]
fn lit_var(l: Lit) -> usize {
    (l >> 1) as usize
}

#[inline]
fn lit_neg(l: Lit) -> Lit {
    l ^ 1
}

#[inline]
fn lit_sign_positive(l: Lit) -> bool {
    (l & 1) == 0
}

/// Result of a `solve` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KittenResult {
    Sat,
    Unsat,
}

/// Reason a variable was assigned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reason {
    /// Decision (no antecedent clause).
    Decision,
    /// An assumption literal fixed for this `solve` call.
    Assumption,
    /// Implied by unit propagation on clause `idx` (index into `self.clauses`).
    Clause(usize),
}

const UNASSIGNED: i8 = 0;
const TRUE: i8 = 1;
const FALSE: i8 = -1;

pub(crate) struct Kitten {
    num_vars: usize,
    /// All clauses: the first `num_input` are the input clauses (whose indices form
    /// the clausal core); the rest are learned. Each is a vector of internal `Lit`s.
    clauses: Vec<Vec<Lit>>,
    num_input: usize,

    /// value[var] in {TRUE, FALSE, UNASSIGNED}.
    value: Vec<i8>,
    /// Decision level at which each var was assigned (valid only when assigned).
    level: Vec<u32>,
    /// Reason for each var's assignment (valid only when assigned).
    reason: Vec<Reason>,
    /// Assignment trail (internal literals set true), in assignment order.
    trail: Vec<Lit>,
    /// Index into `trail` of the first not-yet-propagated literal.
    propagated: usize,
    /// trail_lim[d] = trail length when decision level d+1 began.
    trail_lim: Vec<usize>,

    /// Watch lists: watches[lit] = clause indices watching `lit`.
    watches: Vec<Vec<usize>>,

    /// Scratch "seen" marks for conflict analysis / core walk, indexed by var.
    seen: Vec<bool>,

    /// Set to true once the empty clause / a root conflict is derived; the instance is
    /// then permanently UNSAT regardless of assumptions.
    inconsistent: bool,

    /// Input-clause indices in the most recent UNSAT core (filled by `solve` on UNSAT).
    core: Vec<usize>,
}

impl Kitten {
    pub(crate) fn new() -> Self {
        Kitten {
            num_vars: 0,
            clauses: Vec::new(),
            num_input: 0,
            value: Vec::new(),
            level: Vec::new(),
            reason: Vec::new(),
            trail: Vec::new(),
            propagated: 0,
            trail_lim: Vec::new(),
            watches: Vec::new(),
            seen: Vec::new(),
            inconsistent: false,
            core: Vec::new(),
        }
    }

    fn ensure_var(&mut self, var: usize) {
        while self.num_vars <= var {
            self.value.push(UNASSIGNED);
            self.level.push(0);
            self.reason.push(Reason::Decision);
            self.seen.push(false);
            self.watches.push(Vec::new()); // positive literal
            self.watches.push(Vec::new()); // negative literal
            self.num_vars += 1;
        }
    }

    /// Reserve variables 1..=n (DIMACS) so that solving under assumptions that mention
    /// an isolated variable (one in no clause) is well-defined rather than out-of-bounds.
    pub(crate) fn ensure_num_vars(&mut self, n: usize) {
        if n > 0 {
            self.ensure_var(n - 1);
        }
    }

    /// Add an input clause given as signed DIMACS literals. Tautologies and repeated
    /// literals are normalized away; the empty clause makes the instance inconsistent.
    pub(crate) fn add_clause(&mut self, dimacs: &[i32]) {
        let mut lits: Vec<Lit> = Vec::with_capacity(dimacs.len());
        for &d in dimacs {
            let v = (d.unsigned_abs() - 1) as usize;
            self.ensure_var(v);
            let l = lit_from_dimacs(d);
            if lits.contains(&l) {
                continue; // repeated literal
            }
            if lits.contains(&lit_neg(l)) {
                return; // tautology: drop the clause
            }
            lits.push(l);
        }
        let idx = self.clauses.len();
        if lits.is_empty() {
            self.inconsistent = true;
            self.clauses.push(lits);
            self.num_input = self.clauses.len();
            return;
        }
        if lits.len() >= 2 {
            self.watches[lits[0] as usize].push(idx);
            self.watches[lits[1] as usize].push(idx);
        }
        self.clauses.push(lits);
        self.num_input = self.clauses.len();
    }

    #[inline]
    fn lit_value(&self, l: Lit) -> i8 {
        let v = self.value[lit_var(l)];
        if v == UNASSIGNED {
            UNASSIGNED
        } else if lit_sign_positive(l) {
            v
        } else {
            -v
        }
    }

    #[inline]
    fn assign(&mut self, l: Lit, reason: Reason) {
        let var = lit_var(l);
        self.value[var] = if lit_sign_positive(l) { TRUE } else { FALSE };
        self.level[var] = self.trail_lim.len() as u32;
        self.reason[var] = reason;
        self.trail.push(l);
    }

    /// Backtrack to decision `level`, keeping all assignments made at levels 0..=level.
    /// `trail_lim[d]` is the trail length when level d+1 began, so the assignments of
    /// level `level` end at `trail_lim[level]`; popping to there preserves the root
    /// (level-0) assignments on `backtrack(0)` (they are permanent, e.g. enqueued units).
    fn backtrack(&mut self, level: u32) {
        let level = level as usize;
        if level >= self.trail_lim.len() {
            return; // already at or below this level
        }
        let target_len = self.trail_lim[level];
        while self.trail.len() > target_len {
            let l = self.trail.pop().unwrap();
            self.value[lit_var(l)] = UNASSIGNED;
        }
        self.propagated = target_len;
        self.trail_lim.truncate(level);
    }

    /// Unit-propagate to a fixpoint. Returns `Some(conflict_clause_idx)` on conflict.
    fn propagate(&mut self) -> Option<usize> {
        while self.propagated < self.trail.len() {
            let p = self.trail[self.propagated];
            self.propagated += 1;
            // Clauses watching ¬p may now be unit or conflicting.
            let watch_lit = lit_neg(p) as usize;
            let mut i = 0;
            'next_clause: while i < self.watches[watch_lit].len() {
                let cidx = self.watches[watch_lit][i];
                // Ensure the watched literal is at position 1 so position 0 is the
                // "other" watch (standard two-watched-literal bookkeeping).
                let false_lit = lit_neg(p);
                if self.clauses[cidx][0] == false_lit {
                    self.clauses[cidx].swap(0, 1);
                }
                let other = self.clauses[cidx][0];
                if self.lit_value(other) == TRUE {
                    i += 1;
                    continue; // clause already satisfied
                }
                // Look for a new, non-false literal to watch.
                let len = self.clauses[cidx].len();
                for k in 2..len {
                    let cand = self.clauses[cidx][k];
                    if self.lit_value(cand) != FALSE {
                        self.clauses[cidx].swap(1, k);
                        self.watches[cand as usize].push(cidx);
                        self.watches[watch_lit].swap_remove(i);
                        continue 'next_clause; // do not advance i (swap_remove moved a new one in)
                    }
                }
                // No replacement: `other` is unit or the clause is conflicting.
                if self.lit_value(other) == FALSE {
                    return Some(cidx); // conflict
                }
                self.assign(other, Reason::Clause(cidx));
                i += 1;
            }
        }
        None
    }

    /// 1-UIP conflict analysis. Returns the learned clause (internal lits, asserting
    /// literal first) and the backjump level. Assumes `conflict` is a falsified clause
    /// at the current (>0) decision level.
    fn analyze(&mut self, conflict: usize) -> (Vec<Lit>, u32) {
        let current_level = self.trail_lim.len() as u32;
        let mut learned: Vec<Lit> = vec![0]; // reserve slot 0 for the UIP
        let mut path_count = 0usize;
        let mut trail_idx = self.trail.len();
        let mut confl = conflict;
        let mut resolve_lit: Option<Lit> = None;

        loop {
            // Add the reason clause's literals (all but the resolved pivot) to the
            // learned clause, marking their vars; count how many are at current level.
            let clause_len = self.clauses[confl].len();
            for k in 0..clause_len {
                let q = self.clauses[confl][k];
                if Some(q) == resolve_lit {
                    continue; // pivot resolved away
                }
                let v = lit_var(q);
                if self.seen[v] || self.level[v] == 0 {
                    continue;
                }
                self.seen[v] = true;
                if self.level[v] == current_level {
                    path_count += 1;
                } else {
                    learned.push(q);
                }
            }
            // Find the next literal to resolve: walk the trail top-down for a seen,
            // current-level var.
            loop {
                trail_idx -= 1;
                let l = self.trail[trail_idx];
                if self.seen[lit_var(l)] {
                    break;
                }
            }
            let pivot = self.trail[trail_idx];
            let pivot_var = lit_var(pivot);
            self.seen[pivot_var] = false;
            path_count -= 1;
            if path_count == 0 {
                // `pivot` is the UIP; its negation is the asserting literal.
                learned[0] = lit_neg(pivot);
                break;
            }
            // Resolve on the pivot's reason.
            match self.reason[pivot_var] {
                Reason::Clause(c) => {
                    confl = c;
                    resolve_lit = Some(pivot);
                }
                Reason::Decision | Reason::Assumption => {
                    // Should not happen before path_count hits 0 for a proper 1-UIP.
                    learned[0] = lit_neg(pivot);
                    break;
                }
            }
        }

        // Clear the seen marks for the non-UIP learned literals.
        for &q in &learned[1..] {
            self.seen[lit_var(q)] = false;
        }

        // Backjump level = second-highest decision level in the learned clause.
        let mut backjump = 0u32;
        let mut second_pos = 0usize;
        for (pos, &q) in learned.iter().enumerate().skip(1) {
            let lv = self.level[lit_var(q)];
            if lv > backjump {
                backjump = lv;
                second_pos = pos;
            }
        }
        if learned.len() > 2 && second_pos != 1 {
            learned.swap(1, second_pos);
        }
        (learned, backjump)
    }

    fn add_learned(&mut self, lits: Vec<Lit>) -> usize {
        let idx = self.clauses.len();
        if lits.len() >= 2 {
            self.watches[lits[0] as usize].push(idx);
            self.watches[lits[1] as usize].push(idx);
        }
        self.clauses.push(lits);
        idx
    }

    fn pick_decision(&self) -> Option<Lit> {
        // Simple static order over variables; sufficient for bounded environments.
        for v in 0..self.num_vars {
            if self.value[v] == UNASSIGNED {
                return Some((v as u32) << 1); // decide positive
            }
        }
        None
    }

    /// Reset all search state (trail/levels) but keep clauses. Learned clauses are kept
    /// across `solve` calls (incremental); ALL assignments (including level 0) are
    /// cleared — root units are re-enqueued at the start of the next `solve`.
    fn reset_search(&mut self) {
        while let Some(l) = self.trail.pop() {
            self.value[lit_var(l)] = UNASSIGNED;
        }
        self.propagated = 0;
        self.trail_lim.clear();
        self.core.clear();
    }

    /// Solve the current formula under `assumptions` (signed DIMACS lits). Returns
    /// `Sat` (model in `value`) or `Unsat` (core in `self.core`). Assumptions are
    /// installed as pseudo-decisions at level 0-adjacent so a conflict among them (or
    /// with the formula) yields an UNSAT with a valid clausal core.
    pub(crate) fn solve(&mut self, assumptions: &[i32]) -> KittenResult {
        self.reset_search();
        if self.inconsistent {
            self.compute_core_empty();
            return KittenResult::Unsat;
        }

        // Enqueue all unit clauses (input + learned) at the root; they carry no watches
        // (a 1-literal clause has nothing to watch) so propagation cannot discover them.
        for i in 0..self.clauses.len() {
            if self.clauses[i].len() == 1 {
                let u = self.clauses[i][0];
                match self.lit_value(u) {
                    TRUE => {}
                    FALSE => {
                        self.compute_core(i);
                        return KittenResult::Unsat;
                    }
                    _ => self.assign(u, Reason::Clause(i)),
                }
            }
        }

        // Root propagation of any existing units.
        if let Some(confl) = self.propagate() {
            self.compute_core(confl);
            return KittenResult::Unsat;
        }

        // Install assumptions as forced literals at increasing decision levels so a
        // conflict can be analyzed back to them.
        for &a in assumptions {
            let l = lit_from_dimacs(a);
            match self.lit_value(l) {
                TRUE => continue,
                FALSE => {
                    // Assumption contradicts an implied literal: UNSAT under assumptions.
                    // Reconstruct the conflict by treating the failing literal's reason.
                    let confl = self.conflict_for_falsified_assumption(l);
                    self.compute_core(confl);
                    return KittenResult::Unsat;
                }
                _ => {
                    self.trail_lim.push(self.trail.len());
                    self.assign(l, Reason::Assumption);
                    if let Some(confl) = self.propagate() {
                        self.compute_core(confl);
                        return KittenResult::Unsat;
                    }
                }
            }
        }

        // CDCL search.
        loop {
            match self.pick_decision() {
                None => return KittenResult::Sat,
                Some(dec) => {
                    self.trail_lim.push(self.trail.len());
                    self.assign(dec, Reason::Decision);
                    loop {
                        match self.propagate() {
                            None => break,
                            Some(confl) => {
                                let level = self.trail_lim.len() as u32;
                                if level == 0 {
                                    self.compute_core(confl);
                                    return KittenResult::Unsat;
                                }
                                let (learned, backjump) = self.analyze(confl);
                                if learned.len() == 1 {
                                    // Learned a unit: backjump to root and assign it.
                                    self.backtrack(0);
                                    let uip = learned[0];
                                    if self.lit_value(uip) == FALSE {
                                        // Root conflict: UNSAT.
                                        let cidx = self.add_learned(learned);
                                        self.compute_core(cidx);
                                        return KittenResult::Unsat;
                                    }
                                    let cidx = self.add_learned(learned);
                                    self.assign(uip, Reason::Clause(cidx));
                                } else {
                                    let uip = learned[0];
                                    let cidx = self.add_learned(learned);
                                    self.backtrack(backjump);
                                    self.assign(uip, Reason::Clause(cidx));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Value of a DIMACS variable in the current model (only meaningful after `Sat`).
    pub(crate) fn value(&self, dimacs_var: usize) -> Option<bool> {
        let v = dimacs_var - 1;
        if v >= self.num_vars {
            return None;
        }
        match self.value[v] {
            TRUE => Some(true),
            FALSE => Some(false),
            _ => None,
        }
    }

    /// The input-clause indices participating in the most recent refutation.
    pub(crate) fn core(&self) -> &[usize] {
        &self.core
    }

    fn compute_core_empty(&mut self) {
        // The formula contains the empty clause; the core is that clause.
        self.core.clear();
        for (i, c) in self.clauses.iter().enumerate().take(self.num_input) {
            if c.is_empty() {
                self.core.push(i);
                break;
            }
        }
    }

    /// When an assumption `l` is already falsified, find a clause witnessing it: the
    /// reason clause of ¬l (which is currently TRUE / assigned).
    fn conflict_for_falsified_assumption(&self, l: Lit) -> usize {
        let var = lit_var(l);
        match self.reason[var] {
            Reason::Clause(c) => c,
            // Falsified by a root unit with no clause reason: fall back to any input
            // clause containing ¬l that is currently unit-false (best-effort core seed).
            _ => {
                let want = lit_neg(l);
                for (i, c) in self.clauses.iter().enumerate().take(self.num_input) {
                    if c.contains(&want) {
                        return i;
                    }
                }
                0
            }
        }
    }

    /// Walk the reason graph from `conflict` down to input clauses, collecting the
    /// input-clause indices into `self.core`. Marks are cleared before returning.
    fn compute_core(&mut self, conflict: usize) {
        self.core.clear();
        let mut stack: Vec<usize> = Vec::new();
        let mut clause_seen: Vec<bool> = vec![false; self.clauses.len()];
        stack.push(conflict);
        clause_seen[conflict] = true;
        let mut var_stack: Vec<usize> = Vec::new();

        while let Some(cidx) = stack.pop() {
            if cidx < self.num_input {
                self.core.push(cidx);
            }
            // Follow the reasons of the clause's assigned literals.
            for k in 0..self.clauses[cidx].len() {
                let v = lit_var(self.clauses[cidx][k]);
                if self.seen[v] {
                    continue;
                }
                self.seen[v] = true;
                var_stack.push(v);
                if let Reason::Clause(rc) = self.reason[v] {
                    if !clause_seen[rc] {
                        clause_seen[rc] = true;
                        stack.push(rc);
                    }
                }
            }
        }
        for v in var_stack {
            self.seen[v] = false;
        }
        self.core.sort_unstable();
        self.core.dedup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_solver_is_sat() {
        let mut k = Kitten::new();
        assert_eq!(k.solve(&[]), KittenResult::Sat);
    }

    #[test]
    fn single_unit_is_sat_with_model() {
        let mut k = Kitten::new();
        k.add_clause(&[1]);
        assert_eq!(k.solve(&[]), KittenResult::Sat);
        assert_eq!(k.value(1), Some(true));
    }

    #[test]
    fn contradictory_units_are_unsat() {
        let mut k = Kitten::new();
        k.add_clause(&[1]);
        k.add_clause(&[-1]);
        assert_eq!(k.solve(&[]), KittenResult::Unsat);
        // Core must include both conflicting unit clauses.
        let core = k.core().to_vec();
        assert!(core.contains(&0) && core.contains(&1), "core={core:?}");
    }

    #[test]
    fn empty_clause_is_unsat() {
        let mut k = Kitten::new();
        k.add_clause(&[]);
        assert_eq!(k.solve(&[]), KittenResult::Unsat);
        assert_eq!(k.core(), &[0]);
    }

    #[test]
    fn simple_sat_3clause() {
        let mut k = Kitten::new();
        k.add_clause(&[1, 2]);
        k.add_clause(&[-1, 3]);
        k.add_clause(&[-2, -3]);
        assert_eq!(k.solve(&[]), KittenResult::Sat);
        // Verify the returned model satisfies every clause.
        let sat = |lits: &[i32]| lits.iter().any(|&d| k.value(d.unsigned_abs() as usize) == Some(d > 0));
        assert!(sat(&[1, 2]) && sat(&[-1, 3]) && sat(&[-2, -3]));
    }

    #[test]
    fn pigeonhole_3_into_2_is_unsat() {
        // 3 pigeons, 2 holes: x_{p,h} = pigeon p in hole h. UNSAT.
        let mut k = Kitten::new();
        let var = |p: i32, h: i32| (p - 1) * 2 + h; // vars 1..6
        // each pigeon in some hole
        for p in 1..=3 {
            k.add_clause(&[var(p, 1), var(p, 2)]);
        }
        // no two pigeons share a hole
        for h in 1..=2 {
            for p1 in 1..=3 {
                for p2 in (p1 + 1)..=3 {
                    k.add_clause(&[-var(p1, h), -var(p2, h)]);
                }
            }
        }
        assert_eq!(k.solve(&[]), KittenResult::Unsat);
        assert!(!k.core().is_empty());
    }

    #[test]
    fn solve_under_assumption_unsat_then_sat() {
        // (x1 OR x2) AND (-x1 OR x2)  =>  x2 forced; assuming -x2 is UNSAT.
        let mut k = Kitten::new();
        k.add_clause(&[1, 2]);
        k.add_clause(&[-1, 2]);
        assert_eq!(k.solve(&[-2]), KittenResult::Unsat);
        assert_eq!(k.solve(&[2]), KittenResult::Sat);
        // Backbone: x2 is entailed, so assuming x2 is SAT and the model has x2 true.
        assert_eq!(k.value(2), Some(true));
    }

    /// Exhaustive truth-table check: is `clauses` satisfiable over `nv` variables?
    fn brute_force_sat(nv: usize, clauses: &[Vec<i32>]) -> bool {
        for mask in 0u32..(1u32 << nv) {
            let val = |d: i32| {
                let v = (d.unsigned_abs() - 1) as u32;
                let bit = (mask >> v) & 1 == 1;
                if d > 0 {
                    bit
                } else {
                    !bit
                }
            };
            if clauses.iter().all(|c| c.iter().any(|&d| val(d))) {
                return true;
            }
        }
        false
    }

    #[test]
    fn fuzz_against_brute_force() {
        // Deterministic LCG (no external RNG; Date/rand unavailable in this harness).
        let mut state: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };
        let mut checked = 0;
        for _ in 0..2000 {
            let nv = 2 + (next() % 5) as usize; // 2..=6 vars
            let nc = 1 + (next() % 12) as usize; // 1..=12 clauses
            let mut dimacs: Vec<Vec<i32>> = Vec::new();
            for _ in 0..nc {
                let clen = 1 + (next() % 3) as usize; // 1..=3 lits
                let mut c: Vec<i32> = Vec::new();
                for _ in 0..clen {
                    let v = 1 + (next() % nv as u32) as i32;
                    let lit = if next() & 1 == 0 { v } else { -v };
                    if !c.contains(&lit) && !c.contains(&-lit) {
                        c.push(lit);
                    }
                }
                if !c.is_empty() {
                    dimacs.push(c);
                }
            }
            let mut k = Kitten::new();
            // Ensure all nv vars exist even if unused, so models are well-defined.
            for v in 1..=nv as i32 {
                k.add_clause(&[v, -v]); // tautology (dropped), just forces var creation
            }
            for c in &dimacs {
                k.add_clause(c);
            }
            let got = k.solve(&[]);
            let expect_sat = brute_force_sat(nv, &dimacs);
            match got {
                KittenResult::Sat => {
                    assert!(expect_sat, "kitten SAT but brute-force UNSAT: {dimacs:?}");
                    // Verify the returned model actually satisfies every clause.
                    for c in &dimacs {
                        let ok = c.iter().any(|&d| k.value(d.unsigned_abs() as usize) == Some(d > 0));
                        assert!(ok, "kitten model violates clause {c:?} in {dimacs:?}");
                    }
                }
                KittenResult::Unsat => {
                    assert!(!expect_sat, "kitten UNSAT but brute-force SAT: {dimacs:?}");
                    assert!(!k.core().is_empty(), "UNSAT with empty core: {dimacs:?}");
                }
            }
            checked += 1;
        }
        assert!(checked >= 2000);
    }

    #[test]
    fn incremental_learned_clauses_persist() {
        let mut k = Kitten::new();
        k.add_clause(&[1, 2, 3]);
        k.add_clause(&[-1, 2]);
        k.add_clause(&[-2, 3]);
        // Under x3=false: [-2,3]=>x2=false, [-1,2]=>x1=false, then [1,2,3] is all-false
        // => UNSAT. Then with no assumption the formula is SAT (x1=F,x2=T,x3=T), and any
        // clauses learned during the first (incremental) call must not corrupt it.
        assert_eq!(k.solve(&[-3]), KittenResult::Unsat);
        assert_eq!(k.solve(&[]), KittenResult::Sat);
    }
}
