use crate::{
    parse_cnf, verify_model_against_clauses, ProofLog, Solver, SolverConfig, FALSE, TRUE,
    UNASSIGNED,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

mod bruteforce;
mod formula_edit_replay;
mod metamorphic;
mod parser_normalization;

pub(super) type Cnf = Vec<Vec<i32>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OracleStatus {
    Sat,
    Unsat,
}

#[derive(Clone, Debug)]
pub(super) struct SolverOutcome {
    pub(super) status: OracleStatus,
    pub(super) model: Option<Vec<u8>>,
    pub(super) config_hash: String,
}

#[derive(Clone)]
pub(super) struct NamedConfig {
    pub(super) name: &'static str,
    pub(super) config: SolverConfig,
}

#[derive(Clone, Debug)]
pub(super) struct NormalizedCnf {
    pub(super) num_vars: usize,
    pub(super) clauses: Cnf,
    pub(super) dense_to_original: Vec<usize>,
}

#[derive(Clone, Debug)]
pub(super) struct ShrunkFailure {
    pub(super) num_vars: usize,
    pub(super) clauses: Cnf,
}

#[derive(Clone, Debug)]
pub(super) struct Lcg {
    state: u64,
}

impl Lcg {
    pub(super) fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9e37_79b9_7f4a_7c15,
        }
    }

    pub(super) fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 32) as u32
    }

    pub(super) fn range(&mut self, upper: usize) -> usize {
        assert!(upper > 0);
        (self.next_u32() as usize) % upper
    }

    pub(super) fn bool(&mut self) -> bool {
        self.next_u32() & 1 == 0
    }
}

pub(super) fn seed_from_env(default: u64) -> u64 {
    std::env::var("SAT_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

pub(super) fn solve_with_config(
    num_vars: usize,
    clauses: &[Vec<i32>],
    config: &SolverConfig,
) -> SolverOutcome {
    assert_formula_within_declared_vars(num_vars, clauses);
    let mut solver = Solver::new_with_config(num_vars, clauses.to_vec(), config);
    let mut proof = ProofLog::disabled();
    let sat = solver.solve_with_proof(&mut proof, config);
    let model = if sat {
        Some(
            solver
                .sat_model
                .clone()
                .unwrap_or_else(|| solver.assignment.clone()),
        )
    } else {
        None
    };
    SolverOutcome {
        status: if sat {
            OracleStatus::Sat
        } else {
            OracleStatus::Unsat
        },
        model,
        config_hash: config.config_hash(),
    }
}

pub(super) fn assert_solver_matches_oracle(
    label: &str,
    num_vars: usize,
    clauses: &[Vec<i32>],
    config: &SolverConfig,
    expected: OracleStatus,
    seed: u64,
) {
    let outcome = solve_with_config(num_vars, clauses, config);
    if outcome.status != expected {
        let shrunk = shrink_failure_case(num_vars, clauses, |candidate_vars, candidate| {
            solve_with_config(candidate_vars, candidate, config).status != expected
        });
        panic!(
            "{label}: solver/oracle status mismatch seed={seed} config_hash={} expected={expected:?} got={:?}\nminimized_dimacs:\n{}",
            outcome.config_hash,
            outcome.status,
            dimacs_string(shrunk.num_vars, &shrunk.clauses)
        );
    }

    if expected == OracleStatus::Sat {
        let model = outcome
            .model
            .as_ref()
            .expect("SAT result must carry a model snapshot");
        if !verify_model_against_clauses(clauses, model) {
            let shrunk = shrink_failure_case(num_vars, clauses, |candidate_vars, candidate| {
                let candidate_outcome = solve_with_config(candidate_vars, candidate, config);
                candidate_outcome.status == OracleStatus::Sat
                    && !verify_model_against_clauses(
                        candidate,
                        candidate_outcome
                            .model
                            .as_ref()
                            .expect("SAT candidate must have a model"),
                    )
            });
            panic!(
                "{label}: SAT model failed original-CNF check seed={seed} config_hash={}\nminimized_dimacs:\n{}",
                outcome.config_hash,
                dimacs_string(shrunk.num_vars, &shrunk.clauses)
            );
        }
    }
}

pub(super) fn brute_force_model(num_vars: usize, clauses: &[Vec<i32>]) -> Option<Vec<u8>> {
    assert!(num_vars <= 24, "brute-force oracle is intentionally small");
    let total = 1u64 << num_vars;
    for mask in 0..total {
        let mut model = vec![UNASSIGNED; num_vars + 1];
        for (var, slot) in model.iter_mut().enumerate().take(num_vars + 1).skip(1) {
            *slot = if (mask & (1u64 << (var - 1))) != 0 {
                TRUE
            } else {
                FALSE
            };
        }
        if verify_model_against_clauses(clauses, &model) {
            return Some(model);
        }
    }
    None
}

pub(super) fn brute_force_status(num_vars: usize, clauses: &[Vec<i32>]) -> OracleStatus {
    if brute_force_model(num_vars, clauses).is_some() {
        OracleStatus::Sat
    } else {
        OracleStatus::Unsat
    }
}

pub(super) fn dpll_status(
    num_vars: usize,
    clauses: &[Vec<i32>],
    mut budget: usize,
) -> Option<OracleStatus> {
    let mut assignment = vec![0i8; num_vars + 1];
    dpll_recurse(num_vars, clauses, &mut assignment, &mut budget).map(|sat| {
        if sat {
            OracleStatus::Sat
        } else {
            OracleStatus::Unsat
        }
    })
}

fn dpll_recurse(
    num_vars: usize,
    clauses: &[Vec<i32>],
    assignment: &mut [i8],
    budget: &mut usize,
) -> Option<bool> {
    if *budget == 0 {
        return None;
    }
    *budget -= 1;

    loop {
        let mut changed = false;
        let mut all_satisfied = true;
        for clause in clauses {
            let mut satisfied = false;
            let mut unassigned = 0usize;
            let mut unit_lit = 0i32;
            for &lit in clause {
                match assigned_lit_value(assignment, lit) {
                    Some(true) => {
                        satisfied = true;
                        break;
                    }
                    Some(false) => {}
                    None => {
                        unassigned += 1;
                        unit_lit = lit;
                    }
                }
            }
            if satisfied {
                continue;
            }
            all_satisfied = false;
            if unassigned == 0 {
                return Some(false);
            }
            if unassigned == 1 {
                let var = unit_lit.unsigned_abs() as usize;
                let value = if unit_lit > 0 { 1 } else { -1 };
                if assignment[var] != 0 && assignment[var] != value {
                    return Some(false);
                }
                if assignment[var] == 0 {
                    assignment[var] = value;
                    changed = true;
                }
            }
        }
        if all_satisfied {
            return Some(true);
        }
        if !changed {
            break;
        }
    }

    let var = choose_branch_var(num_vars, clauses, assignment)?;
    let snapshot = assignment.to_vec();
    for value in [1, -1] {
        assignment.clone_from_slice(&snapshot);
        assignment[var] = value;
        if dpll_recurse(num_vars, clauses, assignment, budget)? {
            assignment.clone_from_slice(&snapshot);
            return Some(true);
        }
    }
    assignment.clone_from_slice(&snapshot);
    Some(false)
}

fn assigned_lit_value(assignment: &[i8], lit: i32) -> Option<bool> {
    let var = lit.unsigned_abs() as usize;
    match assignment[var] {
        0 => None,
        1 => Some(lit > 0),
        -1 => Some(lit < 0),
        _ => unreachable!(),
    }
}

fn choose_branch_var(num_vars: usize, clauses: &[Vec<i32>], assignment: &[i8]) -> Option<usize> {
    let mut scores = vec![0usize; num_vars + 1];
    for clause in clauses {
        if clause
            .iter()
            .any(|&lit| assigned_lit_value(assignment, lit) == Some(true))
        {
            continue;
        }
        for &lit in clause {
            let var = lit.unsigned_abs() as usize;
            if assignment[var] == 0 {
                scores[var] += 1;
            }
        }
    }
    (1..=num_vars)
        .filter(|&var| assignment[var] == 0)
        .max_by_key(|&var| (scores[var], std::cmp::Reverse(var)))
}

pub(super) fn small_oracle_formulas() -> Vec<(usize, Cnf)> {
    let mut formulas = vec![
        (0, vec![]),
        (0, vec![vec![]]),
        (1, vec![vec![1]]),
        (1, vec![vec![1], vec![-1]]),
        (2, vec![vec![1, 2], vec![-1, 2], vec![1, -2], vec![-1, -2]]),
        (3, vec![vec![1], vec![-1, 2], vec![-2, 3], vec![-3]]),
        (4, vec![vec![1, 2, 3], vec![-1, 2, 4], vec![1, -3, -4]]),
    ];

    let mut rng = Lcg::new(0x51_10_0a);
    for num_vars in 1..=4 {
        for _ in 0..5 {
            let clause_count = rng.range(8);
            let mut clauses = Vec::with_capacity(clause_count);
            for _ in 0..clause_count {
                let len = rng.range(4);
                let mut clause = Vec::with_capacity(len);
                for _ in 0..len {
                    let var = rng.range(num_vars) + 1;
                    let sign = if rng.bool() { 1 } else { -1 };
                    clause.push(sign * var as i32);
                }
                clauses.push(clause);
            }
            formulas.push((num_vars, clauses));
        }
    }
    formulas
}

pub(super) fn oracle_config_variants() -> Vec<NamedConfig> {
    let mut variants = Vec::new();
    let default = SolverConfig::default();
    variants.push(NamedConfig {
        name: "default",
        config: default.clone(),
    });

    let mut no_simplification = default.clone();
    no_simplification.simplification = false;
    no_simplification.bve = false;
    no_simplification.full_bsr = false;
    variants.push(NamedConfig {
        name: "SAT_SIMPLIFICATION=off",
        config: no_simplification,
    });

    let mut no_bve = default.clone();
    no_bve.bve = false;
    variants.push(NamedConfig {
        name: "SAT_BVE=off",
        config: no_bve,
    });

    let mut no_full_bsr = default.clone();
    no_full_bsr.full_bsr = false;
    variants.push(NamedConfig {
        name: "SAT_FULL_BSR=off",
        config: no_full_bsr,
    });

    let mut full_bsr = default.clone();
    full_bsr.full_bsr = true;
    variants.push(NamedConfig {
        name: "SAT_FULL_BSR=on",
        config: full_bsr,
    });

    let mut lbd_on = default.clone();
    lbd_on.use_lbd = true;
    variants.push(NamedConfig {
        name: "SAT_USE_LBD=on",
        config: lbd_on,
    });

    let mut lbd_off = default.clone();
    lbd_off.use_lbd = false;
    variants.push(NamedConfig {
        name: "SAT_USE_LBD=off",
        config: lbd_off,
    });

    let mut binary_off = default.clone();
    binary_off.binary_fast_path = false;
    variants.push(NamedConfig {
        name: "SAT_BINARY_FAST=off",
        config: binary_off,
    });

    let mut binary_on = default.clone();
    binary_on.binary_fast_path = true;
    variants.push(NamedConfig {
        name: "SAT_BINARY_FAST=on",
        config: binary_on,
    });

    variants
}

pub(super) fn dimacs_string(num_vars: usize, clauses: &[Vec<i32>]) -> String {
    let mut out = format!("p cnf {num_vars} {}\n", clauses.len());
    for clause in clauses {
        for &lit in clause {
            out.push_str(&lit.to_string());
            out.push(' ');
        }
        out.push_str("0\n");
    }
    out
}

pub(super) fn write_temp_cnf(label: &str, body: &str) -> PathBuf {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "sat-playground-s11-{label}-{}-{id}.cnf",
        std::process::id()
    ));
    fs::write(&path, body).expect("write temp CNF");
    path
}

pub(super) fn parse_temp_cnf(label: &str, body: &str) -> (usize, Cnf, PathBuf) {
    let path = write_temp_cnf(label, body);
    let parsed = parse_cnf(path.to_str().expect("temp CNF path is not UTF-8"))
        .expect("temp CNF should parse");
    (parsed.0, parsed.1, path)
}

pub(super) fn remove_temp(path: &Path) {
    let _ = fs::remove_file(path);
}

pub(super) fn normalize_formula(
    num_vars: usize,
    clauses: &[Vec<i32>],
    dense: bool,
) -> NormalizedCnf {
    let mut old_to_new = vec![0usize; num_vars + 1];
    let mut dense_to_original = vec![0usize];
    if dense {
        let mut used = BTreeSet::new();
        for clause in clauses {
            for &lit in clause {
                used.insert(lit.unsigned_abs() as usize);
            }
        }
        for original in used {
            old_to_new[original] = dense_to_original.len();
            dense_to_original.push(original);
        }
    } else {
        for (var, slot) in old_to_new.iter_mut().enumerate().take(num_vars + 1).skip(1) {
            *slot = var;
        }
        dense_to_original = (0..=num_vars).collect();
    }

    let mut normalized = Vec::new();
    for clause in clauses {
        let mapped: Vec<i32> = clause
            .iter()
            .map(|&lit| {
                let old = lit.unsigned_abs() as usize;
                let new = old_to_new[old];
                if lit > 0 {
                    new as i32
                } else {
                    -(new as i32)
                }
            })
            .collect();
        if let Some(canonical) = canonicalize_clause(&mapped) {
            normalized.push(canonical);
        }
    }
    normalized.sort();
    normalized.dedup();
    NormalizedCnf {
        num_vars: dense_to_original.len().saturating_sub(1),
        clauses: normalized,
        dense_to_original,
    }
}

pub(super) fn canonicalize_clause(clause: &[i32]) -> Option<Vec<i32>> {
    let mut lits = clause.to_vec();
    lits.sort_unstable_by(lit_cmp);
    lits.dedup();
    for idx in 1..lits.len() {
        if lits[idx - 1] == -lits[idx] {
            return None;
        }
    }
    Some(lits)
}

fn lit_cmp(lhs: &i32, rhs: &i32) -> std::cmp::Ordering {
    lhs.unsigned_abs()
        .cmp(&rhs.unsigned_abs())
        .then_with(|| lhs.cmp(rhs))
}

pub(super) fn lift_dense_model(
    dense_model: &[u8],
    dense_to_original: &[usize],
    original_num_vars: usize,
) -> Vec<u8> {
    let mut model = vec![TRUE; original_num_vars + 1];
    for (dense_var, &original_var) in dense_to_original.iter().enumerate().skip(1) {
        model[original_var] = dense_model[dense_var];
    }
    model
}

pub(super) fn formula_within_declared_vars(num_vars: usize, clauses: &[Vec<i32>]) -> bool {
    clauses.iter().flatten().all(|&lit| {
        let var = lit.unsigned_abs() as usize;
        var > 0 && var <= num_vars
    })
}

pub(super) fn assert_formula_within_declared_vars(num_vars: usize, clauses: &[Vec<i32>]) {
    assert!(
        formula_within_declared_vars(num_vars, clauses),
        "test harness refused to solve a formula with literals outside 1..={num_vars}: {clauses:?}"
    );
}

pub(super) fn shrink_failure_case<F>(
    num_vars: usize,
    clauses: &[Vec<i32>],
    mut preserves_failure: F,
) -> ShrunkFailure
where
    F: FnMut(usize, &[Vec<i32>]) -> bool,
{
    let mut current = clauses.to_vec();
    let mut idx = 0usize;
    while idx < current.len() {
        let mut candidate = current.clone();
        candidate.remove(idx);
        if preserves_failure(num_vars, &candidate) {
            current = candidate;
        } else {
            idx += 1;
        }
    }

    let mut clause_idx = 0usize;
    while clause_idx < current.len() {
        let mut lit_idx = 0usize;
        while lit_idx < current[clause_idx].len() {
            let mut candidate = current.clone();
            candidate[clause_idx].remove(lit_idx);
            if preserves_failure(num_vars, &candidate) {
                current = candidate;
            } else {
                lit_idx += 1;
            }
        }
        clause_idx += 1;
    }

    let dense = normalize_formula(num_vars, &current, true);
    if preserves_failure(dense.num_vars, &dense.clauses) {
        ShrunkFailure {
            num_vars: dense.num_vars,
            clauses: dense.clauses,
        }
    } else {
        ShrunkFailure {
            num_vars,
            clauses: current,
        }
    }
}

pub(super) fn shrink_feature_set<F>(
    enabled: &[&'static str],
    mut preserves_failure: F,
) -> Vec<&'static str>
where
    F: FnMut(&[&'static str]) -> bool,
{
    let mut current = enabled.to_vec();
    let mut idx = 0usize;
    while idx < current.len() {
        let mut candidate = current.clone();
        candidate.remove(idx);
        if preserves_failure(&candidate) {
            current = candidate;
        } else {
            idx += 1;
        }
    }
    current
}

pub(super) fn map_model_by_permutation(
    transformed_model: &[u8],
    old_to_new_var: &[usize],
    num_vars: usize,
) -> Vec<u8> {
    let mut model = vec![UNASSIGNED; num_vars + 1];
    for old_var in 1..=num_vars {
        model[old_var] = transformed_model[old_to_new_var[old_var]];
    }
    model
}

pub(super) fn map_model_by_polarity_flip(
    transformed_model: &[u8],
    flipped: &[bool],
    num_vars: usize,
) -> Vec<u8> {
    let mut model = vec![UNASSIGNED; num_vars + 1];
    for var in 1..=num_vars {
        model[var] = if flipped[var] {
            match transformed_model[var] {
                TRUE => FALSE,
                FALSE => TRUE,
                other => other,
            }
        } else {
            transformed_model[var]
        };
    }
    model
}

pub(super) fn status_name(status: OracleStatus) -> &'static str {
    match status {
        OracleStatus::Sat => "SAT",
        OracleStatus::Unsat => "UNSAT",
    }
}

pub(super) fn summarize_feature_set(features: &[&str]) -> String {
    if features.is_empty() {
        "<none>".to_string()
    } else {
        features.join(",")
    }
}

pub(super) fn parsed_clause_count(path: &Path) -> usize {
    parse_cnf(path.to_str().expect("path utf8"))
        .expect("parse temp CNF")
        .1
        .len()
}

pub(super) fn collect_var_occurrences(clauses: &[Vec<i32>]) -> BTreeMap<usize, usize> {
    let mut occurrences = BTreeMap::new();
    for clause in clauses {
        for &lit in clause {
            *occurrences.entry(lit.unsigned_abs() as usize).or_insert(0) += 1;
        }
    }
    occurrences
}
