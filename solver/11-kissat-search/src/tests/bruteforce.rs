use super::*;

#[test]
fn brute_force_oracle_matches_solver_across_section0_variants() {
    let formulas = small_oracle_formulas();
    let variants = oracle_config_variants();
    let seed = seed_from_env(0x5eed_0006);

    for (formula_idx, (num_vars, clauses)) in formulas.iter().enumerate() {
        let expected = brute_force_status(*num_vars, clauses);
        for variant in &variants {
            if std::env::var_os("SAT_ORACLE_TRACE").is_some() {
                eprintln!("oracle formula#{formula_idx} variant={}", variant.name);
            }
            assert_solver_matches_oracle(
                &format!("small_formula#{formula_idx} {}", variant.name),
                *num_vars,
                clauses,
                &variant.config,
                expected,
                seed,
            );
        }
    }
}

#[test]
fn randomized_differentials_use_tiny_dpll_oracle_not_solver10_agreement() {
    let seed = seed_from_env(0x51_10_06);
    let mut rng = Lcg::new(seed);
    let mut formulas = Vec::new();

    for _ in 0..18 {
        let num_vars = 1 + rng.range(20);
        let clause_count = 1 + rng.range(80);
        let mut hidden = vec![false; num_vars + 1];
        for slot in hidden.iter_mut().take(num_vars + 1).skip(1) {
            *slot = rng.bool();
        }

        let mut clauses = Vec::with_capacity(clause_count);
        for clause_idx in 0..clause_count {
            let len = 1 + rng.range(5);
            let mut clause = Vec::with_capacity(len);
            let mut hidden_sat_pos = rng.range(len);
            if clause_idx % 7 == 0 {
                hidden_sat_pos = 0;
            }
            for lit_idx in 0..len {
                let var = 1 + rng.range(num_vars);
                let positive = if lit_idx == hidden_sat_pos {
                    hidden[var]
                } else {
                    rng.bool()
                };
                clause.push(if positive { var as i32 } else { -(var as i32) });
            }
            clauses.push(clause);
        }
        formulas.push((num_vars, clauses));
    }

    formulas.push((20, vec![vec![1], vec![-1]]));
    formulas.push((16, vec![vec![1, 2, 3], vec![-1], vec![-2], vec![-3]]));

    let mut configs = Vec::new();
    configs.push(NamedConfig {
        name: "default",
        config: SolverConfig::default(),
    });
    let no_simplification = SolverConfig {
        simplification: false,
        bve: false,
        full_bsr: false,
        ..SolverConfig::default()
    };
    configs.push(NamedConfig {
        name: "SAT_SIMPLIFICATION=off",
        config: no_simplification,
    });
    let lbd = SolverConfig {
        use_lbd: true,
        ..SolverConfig::default()
    };
    configs.push(NamedConfig {
        name: "SAT_USE_LBD=on",
        config: lbd,
    });

    for (formula_idx, (num_vars, clauses)) in formulas.iter().enumerate() {
        let expected = dpll_status(*num_vars, clauses, 250_000).unwrap_or_else(|| {
            panic!(
                "DPLL oracle budget exhausted for seed={seed} formula#{formula_idx}\n{}",
                dimacs_string(*num_vars, clauses)
            )
        });
        for variant in &configs {
            assert_solver_matches_oracle(
                &format!("random_formula#{formula_idx} {}", variant.name),
                *num_vars,
                clauses,
                &variant.config,
                expected,
                seed,
            );
        }
    }
}

#[test]
fn deterministic_shrinker_minimizes_formula_and_enabled_features() {
    let clauses = vec![vec![1, 2, 3], vec![1, 2], vec![1], vec![-4, 5], vec![6]];
    let shrunk = shrink_failure_case(6, &clauses, |_, candidate| {
        candidate.iter().any(|clause| clause == &[1])
    });

    assert_eq!(shrunk.num_vars, 1);
    assert_eq!(shrunk.clauses, vec![vec![1]]);

    let minimized = shrink_feature_set(
        &[
            "SAT_SIMPLIFICATION",
            "SAT_BVE",
            "SAT_FULL_BSR",
            "SAT_USE_LBD",
            "SAT_BINARY_FAST",
        ],
        |features| features.contains(&"SAT_FULL_BSR") && features.contains(&"SAT_USE_LBD"),
    );
    assert_eq!(minimized, vec!["SAT_FULL_BSR", "SAT_USE_LBD"]);
    assert_eq!(
        summarize_feature_set(&minimized),
        "SAT_FULL_BSR,SAT_USE_LBD"
    );
}

#[test]
fn tiny_dpll_oracle_backtracks_unit_propagation_between_branches() {
    let clauses = vec![vec![1, 2], vec![-1, 3], vec![-3, 4], vec![-4]];

    assert_eq!(brute_force_status(4, &clauses), OracleStatus::Sat);
    assert_eq!(dpll_status(4, &clauses, 1000), Some(OracleStatus::Sat));
}
