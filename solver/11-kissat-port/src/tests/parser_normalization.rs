use super::*;

#[test]
fn parser_normalization_differentials_preserve_status_and_mapped_models() {
    let cases = vec![
        (
            "split-duplicate-tautology",
            "c before p-line\np cnf 4 4\n1 2\n-3 0\nc after clause\n2 -2 4 0\n1 2 2 0\n-4 0\n",
        ),
        (
            "duplicate-zero-empty-clause",
            "p cnf 3 3\n1 0\n0\n2 3 0 0\n",
        ),
    ];
    let config = SolverConfig::default();

    for (label, body) in cases {
        let (num_vars, clauses, path) = parse_temp_cnf(label, body);
        assert!(
            formula_within_declared_vars(num_vars, &clauses),
            "{label}: parser produced out-of-range literals in a valid differential case"
        );

        let normalized = normalize_formula(num_vars, &clauses, true);
        let normalized_body = dimacs_string(normalized.num_vars, &normalized.clauses);
        let (norm_num_vars, norm_clauses, norm_path) =
            parse_temp_cnf(&format!("{label}-normalized"), &normalized_body);

        let original = solve_with_config(num_vars, &clauses, &config);
        let normalized_outcome = solve_with_config(norm_num_vars, &norm_clauses, &config);
        assert_eq!(
            original.status,
            normalized_outcome.status,
            "{label}: normalization changed status; original={} normalized={}",
            status_name(original.status),
            status_name(normalized_outcome.status)
        );

        if original.status == OracleStatus::Sat {
            let original_model = original.model.as_ref().expect("original SAT model");
            let normalized_model = normalized_outcome
                .model
                .as_ref()
                .expect("normalized SAT model");
            assert!(verify_model_against_clauses(&clauses, original_model));
            assert!(verify_model_against_clauses(
                &norm_clauses,
                normalized_model
            ));
            let lifted =
                lift_dense_model(normalized_model, &normalized.dense_to_original, num_vars);
            assert!(
                verify_model_against_clauses(&clauses, &lifted),
                "{label}: dense normalized model did not map back to original variables"
            );
        }

        remove_temp(&path);
        remove_temp(&norm_path);
    }
}

#[test]
fn parser_fuzz_variants_are_classified_before_solver_execution() {
    let valid_cases = vec![
        (
            "comments-spaces-tabs",
            "c before\np cnf 2 2\n\t1\t0\nc between\n -1  2   0\nc after\n",
            OracleStatus::Sat,
        ),
        ("split-clause", "p cnf 3 1\n1\n-2\n3 0\n", OracleStatus::Sat),
        (
            "duplicate-literals",
            "p cnf 2 2\n1 1 2 0\n-1 -1 0\n",
            OracleStatus::Sat,
        ),
        ("tautology", "p cnf 2 2\n1 -1 0\n2 0\n", OracleStatus::Sat),
        (
            "huge-in-range-var-id",
            "p cnf 100000 1\n100000 0\n",
            OracleStatus::Sat,
        ),
        (
            "trailing-duplicate-zero-is-empty-clause",
            "p cnf 1 1\n1 0 0\n",
            OracleStatus::Unsat,
        ),
    ];

    let config = SolverConfig::default();
    for (label, body, expected) in valid_cases {
        let (num_vars, clauses, path) = parse_temp_cnf(label, body);
        assert!(
            formula_within_declared_vars(num_vars, &clauses),
            "{label}: expected all literals to be in range"
        );
        assert_eq!(
            solve_with_config(num_vars, &clauses, &config).status,
            expected,
            "{label}: unexpected parser/solver status"
        );
        remove_temp(&path);
    }

    for (label, body, needle) in [
        (
            "out-of-range-var-id",
            "p cnf 2 1\n3 0\n",
            "beyond declared bound",
        ),
        (
            "missing-terminal-zero",
            "p cnf 3 1\n1\t -2   3\n",
            "missing terminal 0",
        ),
        ("empty-file", "", "missing problem line"),
    ] {
        let path = write_temp_cnf(label, body);
        let err = parse_cnf(path.to_str().expect("path utf8")).expect_err("expected parse error");
        assert!(
            err.contains(needle),
            "{label}: expected parse error containing {needle:?}, got {err:?}"
        );
        remove_temp(&path);
    }
}

#[test]
fn parser_fuzz_records_current_compressed_input_boundary() {
    let path = write_temp_cnf("plain-data-with-xz-suffix", "p cnf 1 1\n1 0\n");
    let compressed_path = path.with_extension("cnf.xz");
    fs::rename(&path, &compressed_path).expect("rename temp CNF to .cnf.xz");

    assert_eq!(
        parsed_clause_count(&compressed_path),
        1,
        "parse_cnf ignores extensions; benchmark harness owns real decompression"
    );
    remove_temp(&compressed_path);
}

#[test]
fn normalization_removes_duplicate_literals_tautologies_and_dense_maps_variables() {
    let clauses = vec![
        vec![10, 10, -2],
        vec![2, -2, 10],
        vec![-10, 7],
        vec![-10, 7],
    ];
    let normalized = normalize_formula(10, &clauses, true);

    assert_eq!(normalized.num_vars, 3);
    assert_eq!(normalized.dense_to_original, vec![0, 2, 7, 10]);
    assert_eq!(normalized.clauses, vec![vec![-1, 3], vec![2, -3]]);
    assert_eq!(collect_var_occurrences(&normalized.clauses).len(), 3);
}
