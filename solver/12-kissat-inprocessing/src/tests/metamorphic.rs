use super::*;

struct Variant {
    name: &'static str,
    num_vars: usize,
    clauses: Cnf,
    map_model_to_original: fn(&[u8], usize) -> Vec<u8>,
}

fn identity_model(model: &[u8], num_vars: usize) -> Vec<u8> {
    model[..=num_vars].to_vec()
}

#[test]
fn equivalent_formula_transformations_preserve_status_and_original_model_validity() {
    let base_num_vars = 4;
    let base = vec![vec![1], vec![1, 2, 3], vec![-1, 2, 4], vec![1, -3, -4]];
    let expected = brute_force_status(base_num_vars, &base);

    let permutation = [0usize, 3, 1, 4, 2];
    let mut permuted = Vec::new();
    for clause in &base {
        permuted.push(
            clause
                .iter()
                .map(|&lit| {
                    let mapped = permutation[lit.unsigned_abs() as usize] as i32;
                    if lit > 0 {
                        mapped
                    } else {
                        -mapped
                    }
                })
                .collect(),
        );
    }

    fn map_permutation(model: &[u8], num_vars: usize) -> Vec<u8> {
        let permutation = [0usize, 3, 1, 4, 2];
        map_model_by_permutation(model, &permutation, num_vars)
    }

    let mut reversed_clauses = base.clone();
    reversed_clauses.reverse();

    let mut reversed_literals = base.clone();
    for clause in &mut reversed_literals {
        clause.reverse();
    }

    let mut duplicated_clause = base.clone();
    duplicated_clause.push(base[1].clone());

    let mut duplicated_literals = base.clone();
    for clause in &mut duplicated_literals {
        if let Some(&lit) = clause.first() {
            clause.push(lit);
        }
    }

    let mut tautology_added = base.clone();
    tautology_added.push(vec![2, -2, 3]);

    let mut subsumed_added = base.clone();
    subsumed_added.push(vec![1, -2, 3]);

    let mut blocked_looking_implied = base.clone();
    blocked_looking_implied.push(vec![1, -2, 3, -4]);

    let mut flipped = Vec::new();
    let flip = [false, true, false, true, false];
    for clause in &base {
        flipped.push(
            clause
                .iter()
                .map(|&lit| {
                    let var = lit.unsigned_abs() as usize;
                    if flip[var] {
                        -lit
                    } else {
                        lit
                    }
                })
                .collect(),
        );
    }

    fn map_polarity(model: &[u8], num_vars: usize) -> Vec<u8> {
        let flip = [false, true, false, true, false];
        map_model_by_polarity_flip(model, &flip, num_vars)
    }

    let variants = vec![
        Variant {
            name: "permute variable ids",
            num_vars: base_num_vars,
            clauses: permuted,
            map_model_to_original: map_permutation,
        },
        Variant {
            name: "permute clause order",
            num_vars: base_num_vars,
            clauses: reversed_clauses,
            map_model_to_original: identity_model,
        },
        Variant {
            name: "permute literal order",
            num_vars: base_num_vars,
            clauses: reversed_literals,
            map_model_to_original: identity_model,
        },
        Variant {
            name: "duplicate random clause",
            num_vars: base_num_vars,
            clauses: duplicated_clause,
            map_model_to_original: identity_model,
        },
        Variant {
            name: "duplicate random literals",
            num_vars: base_num_vars,
            clauses: duplicated_literals,
            map_model_to_original: identity_model,
        },
        Variant {
            name: "add tautological clause",
            num_vars: base_num_vars,
            clauses: tautology_added,
            map_model_to_original: identity_model,
        },
        Variant {
            name: "add subsumed clause",
            num_vars: base_num_vars,
            clauses: subsumed_added,
            map_model_to_original: identity_model,
        },
        Variant {
            name: "add blocked-looking implied clause",
            num_vars: base_num_vars,
            clauses: blocked_looking_implied,
            map_model_to_original: identity_model,
        },
        Variant {
            name: "consistent variable polarity flips",
            num_vars: base_num_vars,
            clauses: flipped,
            map_model_to_original: map_polarity,
        },
    ];

    let config = SolverConfig {
        use_lbd: true,
        ..SolverConfig::default()
    };
    for variant in variants {
        let outcome = solve_with_config(variant.num_vars, &variant.clauses, &config);
        assert_eq!(
            outcome.status,
            expected,
            "{} changed status from {} to {}",
            variant.name,
            status_name(expected),
            status_name(outcome.status)
        );
        if outcome.status == OracleStatus::Sat {
            let transformed_model = outcome.model.as_ref().expect("SAT variant model");
            let original_model = (variant.map_model_to_original)(transformed_model, base_num_vars);
            assert!(
                verify_model_against_clauses(&base, &original_model),
                "{} produced a model that does not map back to the original CNF",
                variant.name
            );
        }
    }
}

#[test]
fn metamorphic_unsat_transformations_preserve_unsat_status() {
    let unsat = vec![vec![1, 2], vec![-1, 2], vec![1, -2], vec![-1, -2]];
    let mut shuffled = unsat.clone();
    shuffled.reverse();
    shuffled.push(vec![3, -3]);
    shuffled.push(vec![1, 2, 2]);

    let config = SolverConfig::default();
    assert_eq!(brute_force_status(3, &shuffled), OracleStatus::Unsat);
    assert_eq!(
        solve_with_config(3, &shuffled, &config).status,
        OracleStatus::Unsat
    );
}
