//! Mid-search backbone probing over the binary implication graph — a port of
//! kissat `backbone.c` (SESSION 15; plan/next-plan.md ranked item 2).
//!
//! The pass probes candidate literals by assuming them on a PRIVATE trail and
//! propagating BINARY clauses only, so a probe costs a walk of the binary
//! implication graph (BIG) instead of a full-formula unit propagation. Probe
//! decisions STACK: a successful probe stays assigned and the next candidate is
//! assumed on top, so one round shares propagation work across the whole
//! candidate schedule (kissat's core trick — this is why it affords ~100 rounds
//! per computation where full-propagation probing affords one partial sweep).
//!
//! On a binary conflict the pass walks the private trail's reason edges to the
//! conflict's single dominator (the "UIP" of the BIG — every in-pass assignment
//! has exactly one parent literal, so the two conflict chains merge at or below
//! the newest probe decision), and `¬uip` is a failed-literal unit. Soundness:
//! the caller keeps the REAL root state fully propagated between rounds, so
//! every binary clause with a root-falsified literal already has its partner
//! root-satisfied; a conflict therefore only involves literals assigned in the
//! newest stacked level, whose reason chains all terminate at the newest probe.
//! `uip → conflict` holds via binary clauses alone, so `¬uip` is RUP and may be
//! emitted to the DRAT proof as a plain unit addition.
//!
//! The round structure mirrors kissat exactly: candidates satisfied when
//! reached (implied by an earlier stacked probe) are dropped — if `A → B` in
//! the BIG and probing `A` did not conflict, probing `B` alone cannot conflict
//! either, so re-probing it is pure waste. Candidates falsified by a stacked
//! probe are KEPT for the next round (they were shadowed, not resolved).
//! Successful probes are flushed from the schedule at the end of the round
//! (still satisfied under the final stack), so every round strictly shrinks the
//! candidate set and the round loop terminates without a progress heuristic.
//! Per-literal candidate flags persist across computations (kissat
//! `flags.backbone0/1`): leftovers are re-flagged and scheduled first next
//! time, so a tick-bounded pass resumes where it stopped instead of rescanning
//! from variable 1.

/// Reason sentinel: the literal was a stacked probe decision.
const DECISION_REASON: u32 = u32::MAX;
/// Reason sentinel: the literal was forced as an in-pass failed-literal unit.
const UNIT_REASON: u32 = u32::MAX - 1;
/// Reason sentinel: the literal was seeded from the real root assignment.
const ROOT_REASON: u32 = u32::MAX - 2;

#[inline(always)]
fn negate_index(lit_idx: usize) -> usize {
    lit_idx ^ 1
}

#[inline(always)]
fn var_of_index(lit_idx: usize) -> usize {
    lit_idx >> 1
}

#[inline(always)]
pub(crate) fn index_to_lit(lit_idx: usize) -> i32 {
    let var = (lit_idx >> 1) as i32 + 1;
    if lit_idx & 1 == 0 {
        var
    } else {
        -var
    }
}

/// CSR adjacency over literal indexes: for every live binary clause `(a ∨ b)`,
/// `partners(index(¬a))` contains `index(b)` and `partners(index(¬b))`
/// contains `index(a)` — i.e. the list keyed by a literal holds the literals
/// its TRUTH forces (assigning `¬a` true forces `b`).
pub(crate) struct BinaryGraph {
    start: Vec<u32>,
    target: Vec<u32>,
}

impl BinaryGraph {
    pub(crate) fn build(num_vars: usize, bins: &[(i32, i32)]) -> Self {
        let n = num_vars * 2;
        let mut start = vec![0u32; n + 1];
        for &(a, b) in bins {
            start[crate::lit::lit_to_index(-a) + 1] += 1;
            start[crate::lit::lit_to_index(-b) + 1] += 1;
        }
        for i in 0..n {
            start[i + 1] += start[i];
        }
        let mut target = vec![0u32; start[n] as usize];
        let mut cursor = start.clone();
        for &(a, b) in bins {
            let ia = crate::lit::lit_to_index(-a);
            let ib = crate::lit::lit_to_index(-b);
            target[cursor[ia] as usize] = crate::lit::lit_to_index(b) as u32;
            cursor[ia] += 1;
            target[cursor[ib] as usize] = crate::lit::lit_to_index(a) as u32;
            cursor[ib] += 1;
        }
        Self { start, target }
    }

    #[inline(always)]
    fn partners(&self, lit_idx: usize) -> &[u32] {
        &self.target[self.start[lit_idx] as usize..self.start[lit_idx + 1] as usize]
    }

    pub(crate) fn num_edges(&self) -> usize {
        self.target.len()
    }
}

/// Private propagation state for one computation. Sized once, reseeded per
/// round; never touches the real solver.
pub(crate) struct BackboneScratch {
    /// Per literal index: 1 = true, -1 = false, 0 = unassigned.
    values: Vec<i8>,
    /// Per var: parent literal index or a sentinel.
    reason: Vec<u32>,
    /// Per var: stack depth at assignment (0 = root seed).
    level: Vec<u32>,
    /// In-pass assignments only (literal indexes), in assignment order.
    trail: Vec<u32>,
    /// Per var: analysis mark.
    analyzed: Vec<bool>,
    analyzed_list: Vec<u32>,
}

impl BackboneScratch {
    pub(crate) fn new(num_vars: usize) -> Self {
        Self {
            values: vec![0i8; num_vars * 2],
            reason: vec![ROOT_REASON; num_vars],
            level: vec![0u32; num_vars],
            trail: Vec::new(),
            analyzed: vec![false; num_vars],
            analyzed_list: Vec::new(),
        }
    }

    /// Reseed from the real root assignment: `root_lit_true` yields every
    /// literal index currently TRUE at the real root.
    pub(crate) fn reseed<I: Iterator<Item = usize>>(&mut self, root_lit_true: I) {
        for v in self.values.iter_mut() {
            *v = 0;
        }
        self.trail.clear();
        for lit_idx in root_lit_true {
            self.values[lit_idx] = 1;
            self.values[negate_index(lit_idx)] = -1;
            self.reason[var_of_index(lit_idx)] = ROOT_REASON;
            self.level[var_of_index(lit_idx)] = 0;
        }
    }

    #[inline(always)]
    fn assign(&mut self, lit_idx: usize, reason: u32, level: u32) {
        debug_assert_eq!(self.values[lit_idx], 0);
        self.values[lit_idx] = 1;
        self.values[negate_index(lit_idx)] = -1;
        self.reason[var_of_index(lit_idx)] = reason;
        self.level[var_of_index(lit_idx)] = level;
        self.trail.push(lit_idx as u32);
    }

    /// Binary-only propagation from `propagate_pos` to the trail end. Returns
    /// the conflicting pair `(falsified_watch, falsified_partner)` on conflict.
    fn propagate(
        &mut self,
        graph: &BinaryGraph,
        propagate_pos: &mut usize,
        level: u32,
        ticks: &mut u64,
    ) -> Option<(usize, usize)> {
        while *propagate_pos < self.trail.len() {
            let lit_idx = self.trail[*propagate_pos] as usize;
            *propagate_pos += 1;
            let partners = graph.partners(lit_idx);
            *ticks += 1 + (partners.len() as u64 >> 3);
            for &w in partners {
                let w = w as usize;
                match self.values[w] {
                    1 => continue,
                    // Conflict clause is `(¬lit ∨ w)`; both literals false.
                    -1 => return Some((negate_index(lit_idx), w)),
                    _ => self.assign(w, lit_idx as u32, level),
                }
            }
        }
        None
    }

    fn backtrack(&mut self, saved: usize) {
        while self.trail.len() > saved {
            let lit_idx = self.trail.pop().unwrap() as usize;
            self.values[lit_idx] = 0;
            self.values[negate_index(lit_idx)] = 0;
        }
    }

    /// Walk the private trail's reason edges from the two conflict literals to
    /// their first common ancestor (the BIG "UIP"). Both chains live entirely
    /// in the newest stacked level (see module docs), so the walk never reaches
    /// a root seed or an older decision. Returns the UIP literal index, or
    /// `None` on an anomaly (defensive: the caller then skips the probe).
    fn analyze(&mut self, conflict: (usize, usize)) -> Option<usize> {
        self.analyzed_list.clear();
        let (a, b) = conflict;
        // The conflict clause's literals are both FALSE: their negations are
        // the trail-assigned literals whose chains we walk.
        for lit_idx in [negate_index(a), negate_index(b)] {
            let var = var_of_index(lit_idx);
            debug_assert_eq!(self.values[lit_idx], 1);
            if !self.analyzed[var] {
                self.analyzed[var] = true;
                self.analyzed_list.push(var as u32);
            }
        }
        let mut result = None;
        'walk: for pos in (0..self.trail.len()).rev() {
            let lit_idx = self.trail[pos] as usize;
            let var = var_of_index(lit_idx);
            if !self.analyzed[var] {
                continue;
            }
            let reason = self.reason[var];
            match reason {
                DECISION_REASON | UNIT_REASON | ROOT_REASON => {
                    // Chains failed to merge before a decision/unit — should be
                    // unreachable (see soundness argument); bail defensively.
                    debug_assert!(false, "backbone analysis reached a non-implied literal");
                    break 'walk;
                }
                parent => {
                    let parent = parent as usize;
                    let parent_var = var_of_index(parent);
                    if self.analyzed[parent_var] {
                        result = Some(parent);
                        break 'walk;
                    }
                    self.analyzed[parent_var] = true;
                    self.analyzed_list.push(parent_var as u32);
                }
            }
        }
        for &var in &self.analyzed_list {
            self.analyzed[var as usize] = false;
        }
        self.analyzed_list.clear();
        result
    }
}

pub(crate) struct RoundResult {
    /// Failed-literal units proven this round, as external DIMACS literals, in
    /// discovery order. Each is RUP against the pre-round formula.
    pub(crate) units: Vec<i32>,
    /// A forced unit's own binary propagation conflicted: the formula is UNSAT
    /// once the round's units are applied and propagated for real.
    pub(crate) inconsistent: bool,
    pub(crate) probes: u64,
    pub(crate) ticks: u64,
}

/// Run one backbone round over `candidates` (literal indexes, order
/// preserved). Mutates `candidates` in place kissat-style: drops satisfied,
/// root-falsified, and failed entries (clearing their `cand_flags` bit), keeps
/// stack-falsified and budget-skipped entries. `ticks_limit` is a cumulative
/// cross-round limit: the round aborts (keeping untried candidates) once
/// `*ticks_total` exceeds it.
pub(crate) fn backbone_round(
    graph: &BinaryGraph,
    scratch: &mut BackboneScratch,
    candidates: &mut Vec<u32>,
    cand_flags: &mut [bool],
    ticks_total: &mut u64,
    ticks_limit: u64,
) -> RoundResult {
    let mut result = RoundResult {
        units: Vec::new(),
        inconsistent: false,
        probes: 0,
        ticks: 0,
    };
    let mut level: u32 = 0;
    let mut propagate_pos = scratch.trail.len();
    debug_assert_eq!(propagate_pos, 0, "round must start on an empty private trail");
    let mut write = 0usize;
    let mut read = 0usize;
    while read < candidates.len() {
        if *ticks_total + result.ticks > ticks_limit {
            break;
        }
        let probe = candidates[read] as usize;
        candidates[write] = candidates[read];
        read += 1;
        write += 1;
        match scratch.values[probe] {
            1 => {
                // Satisfied: implied by an earlier stacked probe (or root).
                write -= 1;
                cand_flags[probe] = false;
                continue;
            }
            -1 => {
                if scratch.level[var_of_index(probe)] == 0 {
                    // Root-falsified: permanently resolved, drop.
                    write -= 1;
                    cand_flags[probe] = false;
                } // else: shadowed by a stacked probe — keep for next round.
                continue;
            }
            _ => {}
        }
        result.probes += 1;
        let saved_level = level;
        let saved_trail = scratch.trail.len();
        level += 1;
        scratch.assign(probe, DECISION_REASON, level);
        let conflict = scratch.propagate(graph, &mut propagate_pos, level, &mut result.ticks);
        let Some(conflict) = conflict else {
            continue;
        };
        // Failed literal.
        let uip = scratch.analyze(conflict);
        scratch.backtrack(saved_trail);
        propagate_pos = scratch.trail.len();
        level = saved_level;
        let Some(uip) = uip else {
            // Analysis anomaly: skip the probe, learn nothing (sound: we only
            // ever emit units we can attribute to a dominator).
            write -= 1;
            cand_flags[probe] = false;
            continue;
        };
        let unit_idx = negate_index(uip);
        result.units.push(index_to_lit(unit_idx));
        write -= 1;
        cand_flags[probe] = false;
        scratch.assign(unit_idx, UNIT_REASON, level);
        if scratch
            .propagate(graph, &mut propagate_pos, level, &mut result.ticks)
            .is_some()
        {
            result.inconsistent = true;
            break;
        }
    }
    // Keep everything not yet reached (budget abort / inconsistency break).
    while read < candidates.len() {
        candidates[write] = candidates[read];
        read += 1;
        write += 1;
    }
    // Flush candidates satisfied under the final stack: successful probes and
    // literals they imply cannot fail in the BIG (see module docs). Runs on a
    // budget abort too (kissat parity): `values` still reflects the stack, and
    // an untried candidate already satisfied by a stacked probe is resolved.
    if !result.inconsistent {
        let mut w = 0usize;
        for r in 0..write {
            let probe = candidates[r] as usize;
            candidates[w] = candidates[r];
            w += 1;
            if scratch.values[probe] == 1 {
                w -= 1;
                cand_flags[probe] = false;
            }
        }
        write = w;
    }
    candidates.truncate(write);
    scratch.backtrack(0);
    *ticks_total += result.ticks;
    result
}

/// Build the candidate schedule: flagged (leftover) literals first, then every
/// other live literal, var-ascending, positive before negative — kissat
/// `schedule_backbone_candidates`. `live` reports whether a literal's variable
/// may be probed (unassigned, not eliminated, a decision variable).
pub(crate) fn schedule_candidates<F: Fn(usize) -> bool>(
    num_vars: usize,
    cand_flags: &[bool],
    live: F,
) -> Vec<u32> {
    let mut candidates = Vec::new();
    let mut not_flagged = 0usize;
    for var in 0..num_vars {
        if !live(var) {
            continue;
        }
        for lit_idx in [var * 2, var * 2 + 1] {
            if cand_flags[lit_idx] {
                candidates.push(lit_idx as u32);
            } else {
                not_flagged += 1;
            }
        }
    }
    if not_flagged > 0 {
        for var in 0..num_vars {
            if !live(var) {
                continue;
            }
            for lit_idx in [var * 2, var * 2 + 1] {
                if !cand_flags[lit_idx] {
                    candidates.push(lit_idx as u32);
                }
            }
        }
    }
    candidates
}

/// Re-flag every remaining candidate when none is flagged (kissat
/// `keep_backbone_candidates`): the next computation then prioritizes exactly
/// the unresolved leftovers.
pub(crate) fn keep_candidates(candidates: &[u32], cand_flags: &mut [bool]) {
    let any_flagged = candidates.iter().any(|&c| cand_flags[c as usize]);
    if !any_flagged {
        for &c in candidates {
            cand_flags[c as usize] = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lit::lit_to_index;

    fn run_rounds(
        num_vars: usize,
        bins: &[(i32, i32)],
        root_true: &[i32],
    ) -> (Vec<i32>, bool) {
        let graph = BinaryGraph::build(num_vars, bins);
        let mut scratch = BackboneScratch::new(num_vars);
        let mut flags = vec![false; num_vars * 2];
        let mut candidates = schedule_candidates(num_vars, &flags, |var| {
            !root_true
                .iter()
                .any(|&l| l.unsigned_abs() as usize == var + 1)
        });
        let mut all_units = Vec::new();
        let mut ticks = 0u64;
        for _ in 0..16 {
            scratch.reseed(root_true.iter().map(|&l| lit_to_index(l)));
            let before = candidates.len();
            let r = backbone_round(
                &graph,
                &mut scratch,
                &mut candidates,
                &mut flags,
                &mut ticks,
                u64::MAX,
            );
            let progressed = r.probes > 0 || candidates.len() < before;
            all_units.extend_from_slice(&r.units);
            if r.inconsistent {
                return (all_units, true);
            }
            if candidates.is_empty() || !progressed {
                break;
            }
        }
        (all_units, false)
    }

    #[test]
    fn chain_conflict_yields_uip_unit() {
        // a→b, b→c, b→¬c: probing a (or b) conflicts at c; UIP is b, unit ¬b.
        let bins = [(-1, 2), (-2, 3), (-2, -3)];
        let (units, inconsistent) = run_rounds(3, &bins, &[]);
        assert!(!inconsistent);
        assert!(units.contains(&-2), "expected unit -2, got {units:?}");
        // After ¬b, probing ¬a etc. finds nothing else conflicting.
        assert!(!units.contains(&1));
    }

    #[test]
    fn no_conflict_no_units() {
        let bins = [(-1, 2), (-2, 3)];
        let (units, inconsistent) = run_rounds(3, &bins, &[]);
        assert!(!inconsistent);
        assert!(units.is_empty(), "got {units:?}");
    }

    #[test]
    fn inconsistent_binaries_detected() {
        // a→b, a→¬b, ¬a→b, ¬a→¬b: both a and ¬a fail — UNSAT.
        let bins = [(-1, 2), (-1, -2), (1, 2), (1, -2)];
        let (units, inconsistent) = run_rounds(2, &bins, &[]);
        // Probing 1 fails (unit ¬1 learned); propagating ¬1 then conflicts.
        assert!(inconsistent);
        assert!(!units.is_empty());
    }

    #[test]
    fn root_seed_respected() {
        // With b true at root, the clause (¬a ∨ b) can never fire; nothing fails.
        let bins = [(-1, 2), (-2, 3), (-2, -3)];
        let (units, inconsistent) = run_rounds(3, &bins, &[2, 3]);
        // b=true, c=true at root: (¬b∨¬c) would be root-conflicting in the real
        // solver, but the graph only sees edges from FALSIFIED literals; probing
        // a or ¬a assigns nothing new and no unit is derivable.
        assert!(!inconsistent);
        assert!(units.is_empty(), "got {units:?}");
    }

    #[test]
    fn satisfied_candidates_flushed_and_leftovers_kept() {
        // a→b: probing a implies b, so b's positive candidate is flushed after
        // the round; probing everything else is clean.
        let bins = [(-1, 2)];
        let graph = BinaryGraph::build(2, &bins);
        let mut scratch = BackboneScratch::new(2);
        let mut flags = vec![false; 4];
        let mut candidates = schedule_candidates(2, &flags, |_| true);
        assert_eq!(candidates.len(), 4);
        let mut ticks = 0u64;
        scratch.reseed(std::iter::empty());
        let r = backbone_round(
            &graph,
            &mut scratch,
            &mut candidates,
            &mut flags,
            &mut ticks,
            u64::MAX,
        );
        assert!(r.units.is_empty());
        assert!(!r.inconsistent);
        // Probe order: 1, ¬1(shadowed? no—1 succeeded and stays stacked, so ¬1
        // is stack-falsified and KEPT), 2 (satisfied → dropped), ¬2 (falsified
        // at stack level → kept). Flush drops the satisfied probes 1 and 2.
        assert!(candidates.contains(&(lit_to_index(-1) as u32)));
        assert!(candidates.contains(&(lit_to_index(-2) as u32)));
        assert!(!candidates.contains(&(lit_to_index(1) as u32)));
        assert!(!candidates.contains(&(lit_to_index(2) as u32)));
    }

    #[test]
    fn tick_budget_aborts_and_keeps_untried() {
        let bins = [(-1, 2), (-2, 3), (-3, 4), (-4, 5)];
        let graph = BinaryGraph::build(5, &bins);
        let mut scratch = BackboneScratch::new(5);
        let mut flags = vec![false; 10];
        let mut candidates = schedule_candidates(5, &flags, |_| true);
        let total = candidates.len();
        let mut ticks = 0u64;
        scratch.reseed(std::iter::empty());
        let r = backbone_round(
            &graph,
            &mut scratch,
            &mut candidates,
            &mut flags,
            &mut ticks,
            0,
        );
        // Budget of 0: the limit is checked BETWEEN probes (kissat parity), so
        // exactly one probe runs, then the round aborts. Probing 1 stacks the
        // whole implication chain 1→2→3→4→5; the flush drops the satisfied
        // positives and every negation survives for the next computation.
        assert_eq!(r.probes, 1);
        let _ = total;
        assert_eq!(candidates.len(), 5);
        for v in 1..=5 {
            assert!(candidates.contains(&(lit_to_index(-v) as u32)));
        }
    }

    #[test]
    fn keep_candidates_reflags_leftovers() {
        let mut flags = vec![false; 6];
        let candidates = vec![1u32, 4u32];
        keep_candidates(&candidates, &mut flags);
        assert!(flags[1] && flags[4]);
        assert!(!flags[0] && !flags[2]);
        // Already-flagged: unchanged.
        let candidates2 = vec![1u32];
        keep_candidates(&candidates2, &mut flags);
        assert!(flags[1] && flags[4]);
    }

    #[test]
    fn diamond_conflict_finds_dominator() {
        // a→b, a→c, b→d, c→¬d: probing a conflicts at d; the dominator is a
        // itself (chains b→d and c→¬d merge only at a), unit ¬a.
        let bins = [(-1, 2), (-1, 3), (-2, 4), (-3, -4)];
        let (units, inconsistent) = run_rounds(4, &bins, &[]);
        assert!(!inconsistent);
        assert!(units.contains(&-1), "expected unit -1, got {units:?}");
    }
}
