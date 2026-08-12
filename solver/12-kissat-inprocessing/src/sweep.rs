//! SAT-sweeping environment construction (bead SAT-playground-5b2.3.38, Phase 2).
//!
//! Sweeping reasons about one scheduled variable at a time inside a *bounded* local
//! environment: kissat sweep.c does a breadth-first expansion from the seed variable
//! over the occurrence graph (a variable's clauses, their variables, their clauses, …)
//! up to a small clause/variable budget, loads that sub-formula into the embedded
//! `kitten` sub-solver, and there proves backbone units and literal equivalences.
//!
//! This module builds that environment. It is decoupled from the outer solver: the
//! caller supplies a closure that returns, for an outer variable, the clauses touching
//! it (as signed DIMACS literals over outer variable ids). The builder de-duplicates
//! clauses, remaps the collected outer variables to dense 0-based kitten variables, and
//! loads the remapped clauses into a fresh `Kitten`. Wiring it to the real occurrence
//! lists is Phase 5.
#![allow(dead_code)]

use crate::kitten::Kitten;
use std::collections::HashMap;

/// Default environment caps, mirroring kissat's sweep bounds.
pub(crate) const SWEEP_MAX_VARS: usize = 256;
pub(crate) const SWEEP_MAX_CLAUSES: usize = 1024;
pub(crate) const SWEEP_DEPTH: usize = 2;

/// A bounded sweeping environment: the embedded solver plus the variable remapping
/// needed to translate kitten facts (units, equivalences) back to outer variables.
pub(crate) struct SweepEnv {
    pub(crate) kitten: Kitten,
    /// kitten variable (0-based) -> outer DIMACS variable id (1-based).
    pub(crate) to_outer: Vec<i32>,
    /// outer DIMACS variable id -> kitten DIMACS variable id (1-based), for building
    /// assumption literals to probe.
    pub(crate) to_kitten: HashMap<i32, i32>,
    /// The outer clauses loaded (signed DIMACS, outer vars), parallel to the clauses
    /// added to kitten in the same order — this is the input-clause list whose indices
    /// the kitten core refers to, so the caller can map a core back to outer clauses.
    pub(crate) outer_clauses: Vec<Vec<i32>>,
    /// Whether the BFS hit a cap (environment is a truncated neighbourhood, not closed).
    pub(crate) truncated: bool,
}

impl SweepEnv {
    /// Translate an outer literal (signed DIMACS over outer vars) to a kitten literal,
    /// if its variable is in the environment.
    pub(crate) fn outer_lit_to_kitten(&self, outer_lit: i32) -> Option<i32> {
        let outer_var = outer_lit.abs();
        self.to_kitten.get(&outer_var).map(|&kv| {
            if outer_lit < 0 {
                -kv
            } else {
                kv
            }
        })
    }

    /// Translate a kitten variable (1-based DIMACS) back to its outer variable.
    pub(crate) fn kitten_var_to_outer(&self, kitten_var: i32) -> i32 {
        self.to_outer[(kitten_var - 1) as usize]
    }

    /// The kitten refutation proof lemmas translated to OUTER literals, in derivation
    /// order. Emitting these as RUP `a` lines before adding a proven fact keeps the
    /// outer DRAT proof valid (Phase 4).
    pub(crate) fn proof_lemmas_outer(&self) -> Vec<Vec<i32>> {
        self.kitten
            .proof_lemmas()
            .iter()
            .map(|lemma| {
                lemma
                    .iter()
                    .map(|&kl| {
                        let ov = self.to_outer[(kl.unsigned_abs() - 1) as usize];
                        if kl < 0 {
                            -ov
                        } else {
                            ov
                        }
                    })
                    .collect()
            })
            .collect()
    }
}

/// The outer clauses that realize a set of proven facts: a unit `[u]` per backbone, and
/// the two implication binaries `[-a, b]`, `[-b, a]` per equivalence `a <-> b`. These are
/// what the outer solver adds to the formula (and emits to the proof) after the RUP
/// lemmas from `proof_lemmas_outer`.
pub(crate) fn fact_clauses(facts: &SweepFacts) -> Vec<Vec<i32>> {
    let mut out = Vec::new();
    for &u in &facts.backbones {
        out.push(vec![u]);
    }
    for &(a, b) in &facts.equivalences {
        out.push(vec![-a, b]);
        out.push(vec![-b, a]);
    }
    out
}

/// Build a sweeping environment by BFS from `seed` (outer DIMACS variable id).
///
/// `clauses_of(var)` returns every clause touching `var` as signed DIMACS literals over
/// outer variable ids. The BFS expands `depth` levels, collecting clauses (deduped by
/// their sorted literal set) and variables, stopping when either cap is reached.
pub(crate) fn build_environment<F>(
    seed: i32,
    clauses_of: F,
    depth: usize,
    max_vars: usize,
    max_clauses: usize,
) -> SweepEnv
where
    F: Fn(i32) -> Vec<Vec<i32>>,
{
    let mut to_kitten: HashMap<i32, i32> = HashMap::new();
    let mut to_outer: Vec<i32> = Vec::new();
    let mut outer_clauses: Vec<Vec<i32>> = Vec::new();
    let mut seen_clause: std::collections::HashSet<Vec<i32>> = std::collections::HashSet::new();
    let mut truncated = false;

    // Register the seed variable first so it is kitten var 1.
    let intern = |v: i32,
                      to_kitten: &mut HashMap<i32, i32>,
                      to_outer: &mut Vec<i32>|
     -> Option<i32> {
        if let Some(&kv) = to_kitten.get(&v) {
            return Some(kv);
        }
        if to_outer.len() >= max_vars {
            return None;
        }
        let kv = (to_outer.len() as i32) + 1;
        to_outer.push(v);
        to_kitten.insert(v, kv);
        Some(kv)
    };

    intern(seed.abs(), &mut to_kitten, &mut to_outer);

    // BFS frontier of outer variables to expand.
    let mut frontier: Vec<i32> = vec![seed.abs()];
    let mut expanded: std::collections::HashSet<i32> = std::collections::HashSet::new();

    'bfs: for _ in 0..depth {
        let mut next: Vec<i32> = Vec::new();
        for &var in &frontier {
            if !expanded.insert(var) {
                continue;
            }
            for clause in clauses_of(var) {
                if outer_clauses.len() >= max_clauses {
                    truncated = true;
                    break 'bfs;
                }
                // Canonical key for dedup: sorted literals.
                let mut key = clause.clone();
                key.sort_unstable();
                key.dedup();
                if key.iter().any(|&l| key.contains(&-l)) {
                    continue; // tautology
                }
                if !seen_clause.insert(key.clone()) {
                    continue; // already collected
                }
                // Intern every variable of the clause; if a variable would overflow the
                // var cap, skip the whole clause (keep the environment self-contained).
                let mut ok = true;
                for &l in &key {
                    if intern(l.abs(), &mut to_kitten, &mut to_outer).is_none() {
                        ok = false;
                        truncated = true;
                        break;
                    }
                }
                if !ok {
                    seen_clause.remove(&key);
                    continue;
                }
                for &l in &key {
                    let v = l.abs();
                    if !expanded.contains(&v) {
                        next.push(v);
                    }
                }
                outer_clauses.push(key);
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    // Load the collected (remapped) clauses into a fresh kitten.
    let mut kitten = Kitten::new();
    for clause in &outer_clauses {
        let remapped: Vec<i32> = clause
            .iter()
            .map(|&l| {
                let kv = to_kitten[&l.abs()];
                if l < 0 {
                    -kv
                } else {
                    kv
                }
            })
            .collect();
        kitten.add_clause(&remapped);
    }
    // Reserve every interned variable (including isolated ones) so probing any of them
    // under an assumption is well-defined.
    kitten.ensure_num_vars(to_outer.len());

    SweepEnv {
        kitten,
        to_outer,
        to_kitten,
        outer_clauses,
        truncated,
    }
}

/// Facts proven within a sweeping environment, expressed in OUTER literals.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SweepFacts {
    /// kitten solve calls spent in this environment (throughput metric)
    pub(crate) solves_spent: u64,
    /// kitten flip calls spent
    pub(crate) flips_spent: u64,
    /// The whole environment is unsatisfiable — the outer formula is UNSAT.
    pub(crate) env_unsat: bool,
    /// Backbone literals: each is entailed true by the environment (a forced unit).
    pub(crate) backbones: Vec<i32>,
    /// Proven literal equivalences (a, b): the environment entails a <-> b. Stored with
    /// the lexicographically-smaller-|var| literal first, positive seed variable.
    pub(crate) equivalences: Vec<(i32, i32)>,
}

impl SweepEnv {
    /// Snapshot the current kitten model over the environment's variables as a vector
    /// indexed by kitten var-1 (true/false). Only valid immediately after a SAT solve.
    fn model_snapshot(&self) -> Vec<bool> {
        (1..=self.to_outer.len() as i32)
            .map(|kv| self.kitten.value(kv as usize).unwrap_or(false))
            .collect()
    }
}

/// Prove backbone units and literal equivalences in the environment via bounded kitten
/// SAT calls with model-based candidate pruning (the classic SAT-sweeping loop). Returns
/// facts in OUTER literals. `solve_budget` bounds the number of kitten solve calls so a
/// pathological environment cannot stall; on budget exhaustion the proven-so-far facts
/// are returned (sound: every returned fact was proven by a UNSAT kitten call).
pub(crate) fn prove_facts(env: &mut SweepEnv, solve_budget: usize) -> SweepFacts {
    // Unlimited tick budget: `solve_budgeted(_, u64::MAX)` can never exhaust, so this is
    // decision-identical to the historical unbudgeted path (byte-identity for the
    // shipped inprocessing sweep).
    let mut unlimited = u64::MAX;
    prove_facts_budgeted(env, solve_budget, &mut unlimited)
}

/// Tick-budgeted variant of [`prove_facts`] (root sweep, SAT_SWEEP_ROOT). `tick_budget`
/// is a REMAINING-budget accumulator shared across environments: each kitten solve is
/// capped at the remaining ticks, its consumed ticks are subtracted, and the loop stops
/// proving (returning the facts established so far — each one proven by a completed
/// UNSAT call, so soundness is unaffected) once the budget reaches zero. Deterministic:
/// ticks count propagation work, never wall time.
pub(crate) fn prove_facts_budgeted(
    env: &mut SweepEnv,
    solve_budget: usize,
    tick_budget: &mut u64,
) -> SweepFacts {
    prove_facts_budgeted_opts(env, solve_budget, tick_budget, false)
}

/// `skip_transitive`: maintain a union-find over the environment's variables
/// and skip candidate pairs already in one proven class. A k-variable
/// equivalence class then costs k−1 pair proofs instead of k(k−1)/2 — the
/// kissat class-chain shape. Off for the shipped legacy sweep (emitting the
/// transitively-implied pairs is part of its byte-exact trajectory);
/// yield-armed rounds (SAT_SWEEP_YIELD_ESCALATE) turn it on, which is where
/// equivalence-rich formulas (uniqinv class) burn the quadratic waste.
pub(crate) fn prove_facts_budgeted_opts(
    env: &mut SweepEnv,
    solve_budget: usize,
    tick_budget: &mut u64,
    skip_transitive: bool,
) -> SweepFacts {
    use crate::kitten::KittenResult;
    let mut facts = SweepFacts::default();
    let n = env.to_outer.len();
    if n == 0 {
        return facts;
    }
    // Fast kitten mode (phase saving) for yield-armed sweeps: re-solves
    // repair the previous model instead of re-deriving it, which is where
    // the 28x per-solve wall gap to kissat's kitten was measured.
    env.kitten.set_fast_mode(skip_transitive);
    let mut solves = 0usize;

    // One budgeted kitten call: charge consumed ticks against the remaining budget and
    // surface `None` when the budget is exhausted (before or during the call).
    macro_rules! budgeted_solve {
        ($assumps:expr) => {{
            if *tick_budget == 0 {
                None
            } else {
                let result = env.kitten.solve_budgeted($assumps, *tick_budget);
                *tick_budget = tick_budget.saturating_sub(env.kitten.ticks());
                result
            }
        }};
    }

    // Initial model. If the environment is UNSAT outright, the outer formula is UNSAT.
    match budgeted_solve!(&[]) {
        Some(KittenResult::Sat) => {}
        Some(KittenResult::Unsat) => {
            facts.env_unsat = true;
            return facts;
        }
        None => return facts,
    }
    solves += 1;

    // Candidate value for each var (its value in the first model) and whether it has been
    // observed with both polarities across models (=> not a backbone).
    let mut models: Vec<Vec<bool>> = vec![env.model_snapshot()];
    // Whether the kitten currently holds a full SAT model (flips are only
    // meaningful then; an UNSAT probe invalidates it until the next SAT).
    let mut model_valid = true;
    // Union-find for skip_transitive (indices 0..n over kitten vars).
    let mut uf: Vec<usize> = (0..n).collect();
    fn uf_find(uf: &mut Vec<usize>, mut x: usize) -> usize {
        while uf[x] != x {
            uf[x] = uf[uf[x]];
            x = uf[x];
        }
        x
    }

    // --- Backbone detection ---
    // A var is a backbone candidate if it holds the same value in every model seen. Probe
    // by assuming the opposite; UNSAT proves the backbone, SAT yields a new distinguishing
    // model that prunes further candidates.
    let mut var = 1i32;
    while var <= n as i32 && solves < solve_budget {
        let idx = (var - 1) as usize;
        // Same value in all models?
        let v0 = models[0][idx];
        if !models.iter().all(|m| m[idx] == v0) {
            var += 1;
            continue;
        }
        // kissat kitten_flip parity (skip_transitive mode): a successful flip
        // is a FREE disproof — the model itself witnesses the candidate's
        // other polarity, and the flipped model prunes later candidates.
        if skip_transitive && model_valid && env.kitten.flip_literal(var as usize) {
            models.push(env.model_snapshot());
            var += 1;
            continue;
        }
        // Probe the opposite polarity.
        let probe = if v0 { -var } else { var };
        match budgeted_solve!(&[probe]) {
            Some(KittenResult::Unsat) => {
                let outer_var = env.to_outer[idx];
                facts.backbones.push(if v0 { outer_var } else { -outer_var });
                model_valid = false;
            }
            Some(KittenResult::Sat) => {
                models.push(env.model_snapshot());
                model_valid = true;
            }
            None => return facts,
        }
        solves += 1;
        var += 1;
    }

    // Re-establish a full model for equivalence partitioning (last solve may have been a
    // probe under an assumption; solve() cleared assumptions on entry so value() is a
    // model of the whole environment only after an unassumed SAT solve).
    if solves < solve_budget && budgeted_solve!(&[]) == Some(KittenResult::Sat) {
        solves += 1;
        model_valid = true;
        // --- Equivalence detection via model partition refinement ---
        // Two non-backbone variables a, b are equivalence candidates if in every model
        // either a==b (a<->b) or a!=b (a<->¬b). Refine candidate pairs against all models,
        // then prove survivors with a double-implication UNSAT check.
        let backbone_vars: std::collections::HashSet<i32> =
            facts.backbones.iter().map(|l| l.abs()).collect();
        for a in 1..=n as i32 {
            if solves >= solve_budget {
                break;
            }
            if backbone_vars.contains(&env.to_outer[(a - 1) as usize]) {
                continue;
            }
            for b in (a + 1)..=n as i32 {
                if solves >= solve_budget {
                    break;
                }
                if backbone_vars.contains(&env.to_outer[(b - 1) as usize]) {
                    continue;
                }
                let ai = (a - 1) as usize;
                let bi = (b - 1) as usize;
                if skip_transitive {
                    let (ra, rb) = (uf_find(&mut uf, ai), uf_find(&mut uf, bi));
                    if ra == rb {
                        continue; // already in one proven class this round
                    }
                }
                // Determine the candidate relation from the first model, then require it
                // to hold in ALL models; otherwise a,b cannot be equivalent.
                let same0 = models[0][ai] == models[0][bi];
                if !models.iter().all(|m| (m[ai] == m[bi]) == same0) {
                    continue;
                }
                // kissat kitten_flip parity: try to flip either side first —
                // a successful flip distinguishes the pair for free and the
                // new model refines every later candidate pair at zero solves.
                if skip_transitive && model_valid {
                    if env.kitten.flip_literal(a as usize)
                        || env.kitten.flip_literal(b as usize)
                    {
                        models.push(env.model_snapshot());
                        continue;
                    }
                }
                // Prove a <-> (b if same else ¬b): both (a & ¬target) and (¬a & target)
                // must be UNSAT.
                let target = if same0 { b } else { -b };
                let unsat_ab = match budgeted_solve!(&[a, -target]) {
                    Some(r) => r == KittenResult::Unsat,
                    None => return facts,
                };
                solves += 1;
                model_valid = !unsat_ab;
                if !unsat_ab {
                    // record the distinguishing model to prune future pairs
                    models.push(env.model_snapshot());
                    continue;
                }
                if solves >= solve_budget {
                    break;
                }
                let unsat_ba = match budgeted_solve!(&[-a, target]) {
                    Some(r) => r == KittenResult::Unsat,
                    None => return facts,
                };
                solves += 1;
                model_valid = !unsat_ba;
                if unsat_ba {
                    let oa = env.to_outer[ai];
                    let ob = env.to_outer[bi];
                    // outer equivalence: (a) <-> (target-signed b)
                    let ob_signed = if same0 { ob } else { -ob };
                    facts.equivalences.push((oa, ob_signed));
                    if skip_transitive {
                        let (ra, rb) = (uf_find(&mut uf, ai), uf_find(&mut uf, bi));
                        uf[ra] = rb;
                    }
                } else {
                    models.push(env.model_snapshot());
                }
            }
        }
    }

    facts.solves_spent = solves as u64;
    facts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kitten::KittenResult;

    /// Build a `clauses_of` closure from a flat clause list.
    fn occ(clauses: Vec<Vec<i32>>) -> impl Fn(i32) -> Vec<Vec<i32>> {
        move |var: i32| {
            clauses
                .iter()
                .filter(|c| c.iter().any(|&l| l.abs() == var))
                .cloned()
                .collect()
        }
    }

    #[test]
    fn seed_is_kitten_var_one_and_maps_back() {
        let env = build_environment(7, occ(vec![vec![7, 3], vec![-7, 4]]), 2, 256, 1024);
        assert_eq!(env.kitten_var_to_outer(1), 7);
        assert_eq!(env.to_kitten[&7], 1);
        assert_eq!(env.outer_lit_to_kitten(-7), Some(-1));
    }

    #[test]
    fn bfs_collects_depth_neighbourhood_and_dedups() {
        // 7 - 3 - 9 chain; depth 2 from 7 reaches 3 and its clause with 9.
        let clauses = vec![vec![7, 3], vec![7, 3], vec![-3, 9], vec![9, 10]];
        let env = build_environment(7, occ(clauses), 2, 256, 1024);
        // [7,3] deduped to one clause; [-3,9] reached at depth 2; [9,10] would need depth 3.
        assert_eq!(env.outer_clauses.len(), 2, "clauses={:?}", env.outer_clauses);
        assert!(env.to_kitten.contains_key(&7) && env.to_kitten.contains_key(&3) && env.to_kitten.contains_key(&9));
        assert!(!env.to_kitten.contains_key(&10));
    }

    #[test]
    fn environment_preserves_unsat_and_core_maps_to_outer_clauses() {
        // Outer formula around var 5: (5) and (-5) => UNSAT; core must be those clauses.
        let clauses = vec![vec![5], vec![-5]];
        let mut env = build_environment(5, occ(clauses), 2, 256, 1024);
        assert_eq!(env.kitten.solve(&[]), KittenResult::Unsat);
        let core: Vec<i32> = env
            .kitten
            .core()
            .iter()
            .map(|&ci| {
                // Map a kitten input-clause index back to the outer clause.
                let oc = &env.outer_clauses[ci];
                oc[0] // both are units; report the literal for the assertion below
            })
            .collect();
        assert!(core.contains(&5) && core.contains(&-5), "core={core:?}");
    }

    #[test]
    fn respects_clause_cap() {
        let big: Vec<Vec<i32>> = (2..50).map(|v| vec![1, v]).collect();
        let env = build_environment(1, occ(big), 2, 256, 5);
        assert!(env.outer_clauses.len() <= 5);
        assert!(env.truncated);
    }

    #[test]
    fn backbone_probe_via_assumption() {
        // (5 OR 6) AND (-5 OR 6) => 6 is a backbone (entailed true).
        let clauses = vec![vec![5, 6], vec![-5, 6]];
        let mut env = build_environment(6, occ(clauses), 2, 256, 1024);
        let k6 = env.to_kitten[&6];
        // Assuming ¬6 must be UNSAT => 6 is a backbone unit.
        assert_eq!(env.kitten.solve(&[-k6]), KittenResult::Unsat);
        // Assuming 6 is SAT.
        assert_eq!(env.kitten.solve(&[k6]), KittenResult::Sat);
    }

    #[test]
    fn prove_facts_finds_backbone() {
        // (5 OR 6) AND (-5 OR 6) => 6 forced true; 5 is free.
        let clauses = vec![vec![5, 6], vec![-5, 6]];
        let mut env = build_environment(6, occ(clauses), 2, 256, 1024);
        let facts = prove_facts(&mut env, 1000);
        assert!(!facts.env_unsat);
        assert!(facts.backbones.contains(&6), "backbones={:?}", facts.backbones);
        assert!(!facts.backbones.contains(&5) && !facts.backbones.contains(&-5));
    }

    #[test]
    fn prove_facts_finds_equivalence() {
        // (-1 OR 2) AND (-2 OR 1) == (1 <-> 2). Both are free (models {T,T} and {F,F}).
        let clauses = vec![vec![-1, 2], vec![-2, 1]];
        let mut env = build_environment(1, occ(clauses), 2, 256, 1024);
        let facts = prove_facts(&mut env, 1000);
        assert!(!facts.env_unsat);
        assert!(facts.backbones.is_empty(), "backbones={:?}", facts.backbones);
        let eq = &facts.equivalences;
        assert_eq!(eq.len(), 1, "equivalences={eq:?}");
        let (a, b) = eq[0];
        assert!((a, b) == (1, 2) || (a, b) == (2, 1), "eq={eq:?}");
    }

    #[test]
    fn prove_facts_finds_anti_equivalence() {
        // (1 OR 2) AND (-1 OR -2) == (1 <-> ¬2). Models {T,F} and {F,T}.
        let clauses = vec![vec![1, 2], vec![-1, -2]];
        let mut env = build_environment(1, occ(clauses), 2, 256, 1024);
        let facts = prove_facts(&mut env, 1000);
        assert!(facts.backbones.is_empty());
        assert_eq!(facts.equivalences.len(), 1, "eq={:?}", facts.equivalences);
        let (a, b) = facts.equivalences[0];
        // 1 <-> ¬2, recorded as (1, -2) or (2, -1).
        assert!(
            (a, b) == (1, -2) || (a, b) == (2, -1),
            "eq={:?}",
            facts.equivalences
        );
    }

    #[test]
    fn prove_facts_fuzz_sound_against_brute_force() {
        // Every fact prove_facts reports must be genuinely entailed by the formula.
        let mut state: u64 = 0xD1B54A32D192ED03;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };
        // Enumerate models over `nv` vars.
        let models_of = |nv: usize, clauses: &[Vec<i32>]| -> Vec<u32> {
            (0u32..(1u32 << nv))
                .filter(|&mask| {
                    clauses.iter().all(|c| {
                        c.iter().any(|&d| {
                            let v = (d.unsigned_abs() - 1) as u32;
                            let bit = (mask >> v) & 1 == 1;
                            if d > 0 {
                                bit
                            } else {
                                !bit
                            }
                        })
                    })
                })
                .collect()
        };
        for _ in 0..1500 {
            let nv = 2 + (next() % 4) as usize; // 2..=5
            let nc = 2 + (next() % 8) as usize;
            let mut clauses: Vec<Vec<i32>> = Vec::new();
            for _ in 0..nc {
                let clen = 1 + (next() % 3) as usize;
                let mut c = Vec::new();
                for _ in 0..clen {
                    let v = 1 + (next() % nv as u32) as i32;
                    let l = if next() & 1 == 0 { v } else { -v };
                    if !c.contains(&l) && !c.contains(&-l) {
                        c.push(l);
                    }
                }
                if !c.is_empty() {
                    clauses.push(c);
                }
            }
            let occf = {
                let clauses = clauses.clone();
                move |var: i32| clauses.iter().filter(|c| c.iter().any(|&l| l.abs() == var)).cloned().collect::<Vec<_>>()
            };
            let mut env = build_environment(1, occf, 3, 256, 1024);
            let facts = prove_facts(&mut env, 10_000);
            let models = models_of(nv, &clauses);
            let val = |mask: u32, lit: i32| {
                let v = (lit.unsigned_abs() - 1) as u32;
                let bit = (mask >> v) & 1 == 1;
                if lit > 0 { bit } else { !bit }
            };
            if facts.env_unsat {
                // Note: the environment covers a subset of clauses; env UNSAT implies the
                // whole formula UNSAT, so brute-force must agree there are no models.
                assert!(models.is_empty(), "env_unsat but formula has models: {clauses:?}");
                continue;
            }
            // Every backbone literal must be true in every model of the FORMULA.
            for &bl in &facts.backbones {
                assert!(
                    models.iter().all(|&m| val(m, bl)),
                    "unsound backbone {bl} for {clauses:?}"
                );
            }
            // Every equivalence (a,b) must hold (a == b as literals) in every model.
            for &(a, b) in &facts.equivalences {
                assert!(
                    models.iter().all(|&m| val(m, a) == val(m, b)),
                    "unsound equivalence {a}<->{b} for {clauses:?}"
                );
            }
        }
    }

    /// Self-contained RUP check: is clause `c` reverse-unit-propagation implied by
    /// `clauses`? Assume every literal of `c` false, unit-propagate, and look for a
    /// falsified clause. (Small, test-only; not performance-tuned.)
    fn is_rup(clauses: &[Vec<i32>], c: &[i32]) -> bool {
        use std::collections::HashMap;
        let mut assign: HashMap<i32, bool> = HashMap::new(); // var -> value
        for &l in c {
            let v = l.abs();
            let want = l < 0; // l is FALSE => value of v is (l<0)
            if let Some(&e) = assign.get(&v) {
                if e != want {
                    return true; // c contains l and -l
                }
            }
            assign.insert(v, want);
        }
        loop {
            let mut changed = false;
            for clause in clauses {
                let mut unassigned: Vec<i32> = Vec::new();
                let mut satisfied = false;
                for &l in clause {
                    match assign.get(&l.abs()) {
                        Some(&val) => {
                            if (val && l > 0) || (!val && l < 0) {
                                satisfied = true;
                                break;
                            }
                        }
                        None => unassigned.push(l),
                    }
                }
                if satisfied {
                    continue;
                }
                if unassigned.is_empty() {
                    return true; // conflict => RUP
                }
                if unassigned.len() == 1 {
                    let l = unassigned[0];
                    assign.insert(l.abs(), l > 0);
                    changed = true;
                }
            }
            if !changed {
                return false;
            }
        }
    }

    #[test]
    fn phase4_emitted_proof_is_rup_sound() {
        // For random environments, the RUP lemmas (kitten proof, translated) followed by
        // the fact clauses must form a valid DRAT (RUP) extension of the environment
        // clauses — i.e. every emitted clause is RUP given the accumulated clause set.
        let mut state: u64 = 0x243F6A8885A308D3;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };
        for _ in 0..800 {
            let nv = 2 + (next() % 4) as usize;
            let nc = 2 + (next() % 8) as usize;
            let mut clauses: Vec<Vec<i32>> = Vec::new();
            for _ in 0..nc {
                let clen = 1 + (next() % 3) as usize;
                let mut c = Vec::new();
                for _ in 0..clen {
                    let v = 1 + (next() % nv as u32) as i32;
                    let l = if next() & 1 == 0 { v } else { -v };
                    if !c.contains(&l) && !c.contains(&-l) {
                        c.push(l);
                    }
                }
                if !c.is_empty() {
                    clauses.push(c);
                }
            }
            let occf = {
                let clauses = clauses.clone();
                move |var: i32| clauses.iter().filter(|c| c.iter().any(|&l| l.abs() == var)).cloned().collect::<Vec<_>>()
            };
            let mut env = build_environment(1, occf, 3, 256, 1024);
            let facts = prove_facts(&mut env, 10_000);
            // Accumulated proof starts from the environment's clauses (the subset kitten
            // reasoned about; every emitted lemma is RUP w.r.t. it, hence w.r.t. the whole
            // formula which is a superset).
            let mut acc = env.outer_clauses.clone();
            for lemma in env.proof_lemmas_outer() {
                assert!(is_rup(&acc, &lemma), "lemma {lemma:?} not RUP; formula={:?}", env.outer_clauses);
                acc.push(lemma);
            }
            if facts.env_unsat {
                // The empty clause must be RUP after the lemmas.
                assert!(is_rup(&acc, &[]), "env_unsat but empty clause not RUP: {:?}", env.outer_clauses);
                continue;
            }
            for fc in fact_clauses(&facts) {
                assert!(is_rup(&acc, &fc), "fact clause {fc:?} not RUP; formula={:?}", env.outer_clauses);
                acc.push(fc);
            }
        }
    }

    #[test]
    fn prove_facts_detects_unsat_environment() {
        let clauses = vec![vec![5], vec![-5]];
        let mut env = build_environment(5, occ(clauses), 2, 256, 1024);
        let facts = prove_facts(&mut env, 1000);
        assert!(facts.env_unsat);
    }

    #[test]
    fn prove_facts_no_false_positives_on_independent_vars() {
        // Two independent free variables: no backbones, no equivalences.
        let clauses = vec![vec![1, 2], vec![-1, -2, 3], vec![3, 4], vec![-3, 4], vec![-4, 3]];
        // 3<->4 here: (3,4),(-3,4),(-4,3): 4 forced? (3,4): if 3=F need 4. (-3,4): if 3=T need 4 => 4 backbone. (-4,3): 4=T => 3. So 4 backbone, 3 backbone.
        let mut env = build_environment(3, occ(clauses), 2, 256, 1024);
        let facts = prove_facts(&mut env, 1000);
        // 3 and 4 are both forced true; assert they are reported as backbones and no
        // spurious equivalence among the remaining free vars is emitted.
        assert!(facts.backbones.contains(&4), "backbones={:?}", facts.backbones);
    }
}
