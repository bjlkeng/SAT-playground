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
    let mut intern = |v: i32,
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

    SweepEnv {
        kitten,
        to_outer,
        to_kitten,
        outer_clauses,
        truncated,
    }
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
}
