use super::*;
use crate::config::{
    PreprocessAxis, ProofPolicy, ReducePolicy, SearchAxis, SearchModePolicy, SolverProfile,
};
use crate::limits::BudgetClass;
use crate::output::{
    write_model_file, write_result_contract, OutputContract, OutputContractState,
    ProofCompleteness, ResultContractFields, SolveStatus, RESULT_JSON, STATUS_TXT,
};
use crate::stats::ProofStats;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

struct ContractRun {
    status: SolveStatus,
    proof_stats: ProofStats,
    out_dir: PathBuf,
    result_json: String,
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/golden")
        .join(name)
}

fn make_contract_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sat-playground-s11-contract-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create contract output dir");
    path
}

fn env_map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

fn proof_completeness_for(status: SolveStatus, proof_policy: ProofPolicy) -> ProofCompleteness {
    match (status, proof_policy) {
        (SolveStatus::Unsat, ProofPolicy::Drat) => ProofCompleteness::Complete,
        (SolveStatus::Unsat, ProofPolicy::Off) => ProofCompleteness::NotRequested,
        (SolveStatus::Sat, ProofPolicy::Drat) => ProofCompleteness::Incomplete,
        (SolveStatus::Sat, ProofPolicy::Off) => ProofCompleteness::NotRequested,
        (SolveStatus::Unknown, ProofPolicy::Drat) => ProofCompleteness::Incomplete,
        (SolveStatus::Unknown, ProofPolicy::Off) => ProofCompleteness::NotRequested,
        _ => ProofCompleteness::None,
    }
}

fn run_contract(path: &Path, mut config: SolverConfig, label: &str) -> ContractRun {
    let out_dir = make_contract_dir(label);
    let parsed = parse_cnf(path.to_str().expect("golden path utf8"));
    let (status, proof_stats, model_check_result, proof_completeness, unknown_reason) = match parsed
    {
        Ok((num_vars, clauses)) => {
            let mut solver = Solver::new_with_config(num_vars, clauses.clone(), &config);
            let (outcome, proof_stats) =
                solver.solve_to_output(out_dir.to_str().expect("output path utf8"), &config);
            let model_path = if outcome.status == SolveStatus::Sat {
                let model = solver.sat_model.as_ref().expect("SAT model snapshot");
                Some(write_model_file(&out_dir, model))
            } else {
                None
            };
            let model_check_result = if outcome.status == SolveStatus::Sat {
                let model = solver.sat_model.as_ref().expect("SAT model snapshot");
                if !config.check_invariants {
                    "not_checked"
                } else if verify_model_against_clauses(&clauses, model) {
                    "pass"
                } else {
                    "fail"
                }
            } else {
                "not_applicable"
            };
            let proof_completeness = proof_completeness_for(outcome.status, config.proof_policy);
            let fields = ResultContractFields::new(
                outcome.unknown_reason,
                None,
                model_check_result,
                "not_checked",
                "complete",
            )
            .with_termination_reason(outcome.termination_reason());
            write_result_contract(
                &out_dir,
                outcome.status,
                &config,
                &fields,
                model_path.as_deref(),
                proof_completeness,
            );
            (
                outcome.status,
                proof_stats,
                model_check_result,
                proof_completeness,
                outcome.unknown_reason,
            )
        }
        Err(message) => {
            config.stats_json = true;
            let proof_stats = ProofStats {
                state: "not-created",
                ..ProofStats::default()
            };
            let fields = ResultContractFields::new(
                Some(&message),
                None,
                "not_applicable",
                "not_applicable",
                "complete",
            );
            write_result_contract(
                &out_dir,
                SolveStatus::ParseError,
                &config,
                &fields,
                None,
                ProofCompleteness::None,
            );
            (
                SolveStatus::ParseError,
                proof_stats,
                "not_applicable",
                ProofCompleteness::None,
                Some("parse-error"),
            )
        }
    };

    let result_json =
        fs::read_to_string(out_dir.join(RESULT_JSON)).expect("result.json should be written");
    let contract = OutputContract {
        status,
        proof_completeness,
        model_written: out_dir.join("model.txt").exists(),
        proof_written: out_dir.join("proof.out").exists(),
        stats_written: config.stats_json,
        result_json_written: true,
        output_contract_state: OutputContractState::Complete,
    };
    contract.validate().unwrap_or_else(|err| {
        panic!(
            "{label}: output contract rejected status={status:?} model_check={model_check_result} unknown_reason={unknown_reason:?}: {err}"
        )
    });

    ContractRun {
        status,
        proof_stats,
        out_dir,
        result_json,
    }
}

fn assert_manifest_expectation(case_name: &str, run: &ContractRun) {
    let manifest = fs::read_to_string(golden_path("manifest.tsv")).expect("read manifest");
    let line = manifest
        .lines()
        .find(|line| line.starts_with(case_name))
        .unwrap_or_else(|| panic!("manifest missing {case_name}"));
    let fields: Vec<_> = line.split('\t').collect();
    assert_eq!(fields.len(), 7, "manifest row shape changed: {line}");
    assert_eq!(fields[1], run.status.as_str(), "{case_name}: status drift");
    assert_eq!(
        fields[2],
        run.status.exit_code().to_string(),
        "{case_name}: exit-code drift"
    );
    assert_eq!(
        fs::read_to_string(run.out_dir.join(STATUS_TXT))
            .expect("read status file")
            .trim(),
        fields[3],
        "{case_name}: status-file drift"
    );
    assert!(
        run.result_json
            .contains(&format!("\"model_check_result\": \"{}\"", fields[4])),
        "{case_name}: model-check field drift"
    );
    match fields[5] {
        "present" => assert!(
            run.out_dir.join("proof.out").exists(),
            "{case_name}: expected final proof"
        ),
        "absent" => assert!(
            !run.out_dir.join("proof.out").exists(),
            "{case_name}: final proof must be absent"
        ),
        other => panic!("{case_name}: unsupported proof expectation {other}"),
    }
    for required in fields[6].split(',') {
        assert!(
            run.result_json.contains(&format!("\"{required}\"")),
            "{case_name}: missing required result.json field {required}"
        );
    }
}

#[test]
fn test_golden_sat_tiny_output_contract() {
    let run = run_contract(&golden_path("sat_tiny.cnf"), SolverConfig::default(), "sat");
    assert_manifest_expectation("sat_tiny.cnf", &run);
    assert!(run.out_dir.join("model.txt").exists());
    assert!(!run.out_dir.join("proof.out.tmp").exists());
    assert!(run
        .result_json
        .contains("\"proof_completeness\": \"incomplete\""));
}

#[test]
fn test_check_invariants_runs_internal_sat_model_check() {
    let config = SolverConfig {
        check_invariants: true,
        ..SolverConfig::default()
    };
    let run = run_contract(&golden_path("sat_tiny.cnf"), config, "sat-invariants");
    assert_eq!(run.status, SolveStatus::Sat);
    assert!(run.result_json.contains("\"model_check_result\": \"pass\""));
}

#[test]
fn test_golden_unsat_proof_contract() {
    let run = run_contract(
        &golden_path("unsat_empty_clause.cnf"),
        SolverConfig::default(),
        "unsat",
    );
    assert_manifest_expectation("unsat_empty_clause.cnf", &run);
    let proof = fs::read_to_string(run.out_dir.join("proof.out")).expect("read proof");
    assert_eq!(proof.lines().last(), Some("0"));
    assert!(run.proof_stats.finalized);
}

#[test]
fn test_golden_unknown_limit_contract() {
    let mut config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
    config.tick_limit = Some(0);
    config.stats_json = true;
    let run = run_contract(&golden_path("split_clause.cnf"), config, "unknown");
    assert_eq!(run.status, SolveStatus::Unknown);
    assert!(run
        .result_json
        .contains("\"unknown_reason\": \"tick-limit\""));
    assert!(run
        .result_json
        .contains("\"termination_reason\": \"tick-limit\""));
    assert!(!run.out_dir.join("proof.out").exists());
    assert!(!run.out_dir.join("proof.out.tmp").exists());
}

#[test]
fn test_golden_parse_error_contract() {
    let config = SolverConfig {
        stats_json: true,
        ..SolverConfig::default()
    };
    let run = run_contract(
        &golden_path("malformed_missing_zero.cnf"),
        config,
        "parse-error",
    );
    assert_manifest_expectation("malformed_missing_zero.cnf", &run);
    assert!(run.result_json.contains("\"status\": \"PARSE_ERROR\""));
}

#[test]
fn test_empty_formula_sat() {
    let run = run_contract(
        &golden_path("empty_formula.cnf"),
        SolverConfig::default(),
        "empty-formula",
    );
    assert_eq!(run.status, SolveStatus::Sat);
}

#[test]
fn test_empty_clause_unsat() {
    let run = run_contract(
        &golden_path("unsat_empty_clause.cnf"),
        SolverConfig::default(),
        "empty-clause",
    );
    assert_eq!(run.status, SolveStatus::Unsat);
}

#[test]
fn test_tautological_clause_parse() {
    let (vars, clauses) =
        parse_cnf(golden_path("tautology.cnf").to_str().expect("path utf8")).unwrap();
    assert_eq!(vars, 2);
    assert_eq!(clauses, vec![vec![1, -1], vec![2]]);
}

#[test]
fn test_split_clause_parse() {
    let (vars, clauses) =
        parse_cnf(golden_path("split_clause.cnf").to_str().expect("path utf8")).unwrap();
    assert_eq!(vars, 3);
    assert_eq!(clauses, vec![vec![1, -2, 3]]);
}

#[test]
fn test_malformed_dimacs_rejected() {
    for name in [
        "malformed_missing_zero.cnf",
        "malformed_var_out_of_bounds.cnf",
    ] {
        assert!(
            parse_cnf(golden_path(name).to_str().expect("path utf8")).is_err(),
            "{name} should be rejected before solver construction"
        );
    }
}

#[test]
fn test_unknown_limit_flushes_stats() {
    let mut config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
    config.tick_limit = Some(0);
    config.stats_json = true;
    let run = run_contract(&golden_path("split_clause.cnf"), config, "unknown-stats");
    assert_eq!(run.status, SolveStatus::Unknown);
    assert!(run.result_json.contains("\"stats_json_seen\": true"));
    assert!(run
        .result_json
        .contains("\"proof_completeness\": \"incomplete\""));
}

#[test]
fn test_sat_deletes_temp_proof() {
    let run = run_contract(
        &golden_path("sat_tiny.cnf"),
        SolverConfig::default(),
        "sat-temp",
    );
    assert_eq!(run.status, SolveStatus::Sat);
    assert!(run.proof_stats.temp_deleted);
    assert!(!run.out_dir.join("proof.out.tmp").exists());
}

#[test]
fn test_unsat_renames_completed_proof() {
    let run = run_contract(
        &golden_path("unsat_empty_clause.cnf"),
        SolverConfig::default(),
        "unsat-rename",
    );
    assert_eq!(run.status, SolveStatus::Unsat);
    assert!(run.proof_stats.finalized);
    assert!(run.out_dir.join("proof.out").exists());
    assert!(!run.out_dir.join("proof.out.tmp").exists());
}

#[test]
fn test_output_contract_rejects_unsat_with_incomplete_proof() {
    let contract = OutputContract {
        status: SolveStatus::Unsat,
        proof_completeness: ProofCompleteness::Incomplete,
        model_written: false,
        proof_written: true,
        stats_written: false,
        result_json_written: true,
        output_contract_state: OutputContractState::Complete,
    };
    assert_eq!(contract.validate(), Err("unsat_proof_incomplete"));
}

#[test]
fn test_output_contract_rejects_sat_without_model_check() {
    let contract = OutputContract {
        status: SolveStatus::Sat,
        proof_completeness: ProofCompleteness::Incomplete,
        model_written: false,
        proof_written: false,
        stats_written: false,
        result_json_written: true,
        output_contract_state: OutputContractState::Complete,
    };
    assert_eq!(contract.validate(), Err("sat_model_missing"));
}

#[test]
fn test_output_contract_unknown_never_finalizes_proof() {
    let mut config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
    config.tick_limit = Some(0);
    let run = run_contract(&golden_path("split_clause.cnf"), config, "unknown-proof");
    assert_eq!(run.status, SolveStatus::Unknown);
    assert!(run.proof_stats.incomplete);
    assert!(run.proof_stats.temp_deleted);
    assert!(!run.out_dir.join("proof.out").exists());
    assert!(!run.out_dir.join("proof.out.tmp").exists());
}

#[test]
fn test_budget_class_names_cover_future_abort_scopes() {
    assert_eq!(BudgetClass::SolveLimit.as_str(), "solve-limit");
    assert_eq!(BudgetClass::PassBudget.as_str(), "pass-budget");
    assert_eq!(BudgetClass::EditBudget.as_str(), "edit-budget");
    assert_eq!(
        BudgetClass::EmergencyMemoryLimit.as_str(),
        "emergency-memory-limit"
    );
}

#[test]
fn test_output_contract_rejects_incomplete_contract_state() {
    let contract = OutputContract {
        status: SolveStatus::Sat,
        proof_completeness: ProofCompleteness::Incomplete,
        model_written: true,
        proof_written: false,
        stats_written: false,
        result_json_written: true,
        output_contract_state: OutputContractState::Rejected,
    };
    assert_eq!(OutputContractState::Rejected.as_str(), "rejected");
    assert_eq!(contract.validate(), Err("output_contract_incomplete"));
}

#[test]
fn test_profile_baseline_matches_solver10_feature_defaults() {
    let config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "baseline")]));
    assert_eq!(config.profile, SolverProfile::Baseline);
    assert_eq!(config.axes.search, SearchAxis::Safe);
    assert_eq!(config.axes.preprocess, PreprocessAxis::Off);
    assert!(!config.simplification);
    assert!(!config.bve);
    assert!(!config.full_bsr);
    assert!(!config.use_lbd);
}

#[test]
fn test_profile_search_conservative_enables_only_documented_features() {
    let config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "search-conservative")]));
    assert_eq!(config.profile, SolverProfile::Default);
    assert_eq!(config.axes.search, SearchAxis::Validated);
    assert_eq!(config.axes.preprocess, PreprocessAxis::Conservative);
    assert!(config.simplification);
    assert!(config.bve);
    assert!(config.full_bsr);
    // Default search config is now the fstab_lbdtier promotion (profile20 Stage-1, 2026-05-30/31):
    // focused-stable + LBD + ticks + LBD-tiered reduce. Preprocessing axis is unchanged.
    assert!(config.use_lbd);
    assert_eq!(config.search_mode_policy, SearchModePolicy::FocusedStable);
    assert!(config.mode_use_ticks);
    assert_eq!(config.reduce_policy, ReducePolicy::LbdTiered);
    assert!(!config.inprocess);
}

#[test]
fn test_profile_inprocess_conservative_enables_only_documented_features() {
    let config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", "inprocess-conservative")]));
    assert_eq!(config.profile, SolverProfile::Default);
    assert_eq!(config.axes.preprocess, PreprocessAxis::Conservative);
    assert!(config.simplification);
    assert!(config.bve);
    assert!(config.full_bsr);
    assert!(!config.inprocess);
    assert!(!config.vivify);
    assert!(!config.probe);
    assert!(!config.hbr);
}

#[test]
fn test_readme_profile_examples_have_matching_config_hashes() {
    let readme = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("read solver README");
    for profile in ["baseline", "default", "fast", "experimental"] {
        assert!(
            readme.contains(profile),
            "README config examples should mention profile {profile}"
        );
        let config = SolverConfig::from_env_map(&env_map(&[("SAT_PROFILE", profile)]));
        let replay = config.config_replay_text();
        assert!(
            replay.contains(&format!("config_hash={}", config.config_hash())),
            "replay for {profile} should include the effective config hash"
        );
    }
}
