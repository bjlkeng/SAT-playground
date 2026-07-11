use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_output_dir(label: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sat-playground-s11-cli-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create output dir");
    path
}

#[test]
fn single_mode_kissat_ema_is_rejected() {
    let cnf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("golden")
        .join("sat_tiny.cnf");
    let out_dir = temp_output_dir("single-mode-kissat-ema");

    // The default search mode is now focused-stable, under which kissat-ema is valid; this test
    // verifies the SINGLE-mode rejection path, so it must request single mode explicitly (and turn
    // off the default's focused-stable-only mode_use_ticks / lbd-tiered reduce).
    let output = Command::new(env!("CARGO_BIN_EXE_sat-solver"))
        .env_clear()
        .env("SAT_SEARCH_MODE", "single")
        .env("SAT_MODE_USE_TICKS", "off")
        .env("SAT_REDUCE", "legacy")
        .env("SAT_USE_LBD", "on")
        .env("SAT_RESTART", "kissat-ema")
        .arg(cnf)
        .arg(&out_dir)
        .output()
        .expect("run sat-solver");

    let _ = fs::remove_dir_all(&out_dir);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("SAT_RESTART=kissat-ema requires SAT_SEARCH_MODE=focused-stable"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn zero_ema_slow_window_is_rejected() {
    let cnf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("golden")
        .join("sat_tiny.cnf");
    let out_dir = temp_output_dir("zero-ema-slow-window");

    let output = Command::new(env!("CARGO_BIN_EXE_sat-solver"))
        .env_clear()
        .env("SAT_EMA_SLOW_WINDOW", "0")
        .arg(cnf)
        .arg(&out_dir)
        .output()
        .expect("run sat-solver");

    let _ = fs::remove_dir_all(&out_dir);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("SAT_EMA_SLOW_WINDOW must be at least 1"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn resolved_conflict_analysis_mode_is_rejected() {
    let cnf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("golden")
        .join("sat_tiny.cnf");
    let out_dir = temp_output_dir("resolved-conflict-analysis");

    let output = Command::new(env!("CARGO_BIN_EXE_sat-solver"))
        .env_clear()
        .env("SAT_CONFLICT_ANALYSIS_MODE", "resolved")
        .arg(cnf)
        .arg(&out_dir)
        .output()
        .expect("run sat-solver");

    let _ = fs::remove_dir_all(&out_dir);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("SAT_CONFLICT_ANALYSIS_MODE=resolved is retired; use minisat"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn single_mode_target_phase_policies_are_rejected() {
    let cnf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("golden")
        .join("sat_tiny.cnf");

    for phase in ["target-then-saved", "best-then-target-then-saved"] {
        let out_dir = temp_output_dir("single-mode-target-phase");

        // Default search mode is now focused-stable (under which target phases are valid); request
        // single mode explicitly to exercise the single-mode rejection path this test is named for.
        let output = Command::new(env!("CARGO_BIN_EXE_sat-solver"))
            .env_clear()
            .env("SAT_SEARCH_MODE", "single")
            .env("SAT_MODE_USE_TICKS", "off")
            .env("SAT_REDUCE", "legacy")
            .env("SAT_PHASE", phase)
            .arg(&cnf)
            .arg(&out_dir)
            .output()
            .expect("run sat-solver");

        let _ = fs::remove_dir_all(&out_dir);

        assert_eq!(output.status.code(), Some(2), "phase={phase}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!(
                "SAT_PHASE={phase} requires SAT_SEARCH_MODE=focused-stable"
            )),
            "phase={phase}, unexpected stderr: {stderr}"
        );
    }
}

#[test]
fn factor_bva_sat_model_covers_only_original_variables() {
    // A 3x10 factorable grid (30 clauses over 23 vars, clause reduction 17 >
    // FACTOR_BOUND): SAT_FACTOR introduces a fresh variable 24 internally, but
    // the reported model must stop at the original variable count and satisfy
    // every original clause.
    let out_dir = temp_output_dir("factor-bva-sat");
    let cnf_path = out_dir.join("factor_grid.cnf");
    let mut original_clauses: Vec<Vec<i32>> = Vec::new();
    for factor in 1..=3i32 {
        for rest in 0..10i32 {
            original_clauses.push(vec![factor, 4 + 2 * rest, 5 + 2 * rest]);
        }
    }
    let mut dimacs = String::from("p cnf 23 30\n");
    for clause in &original_clauses {
        for lit in clause {
            dimacs.push_str(&format!("{lit} "));
        }
        dimacs.push_str("0\n");
    }
    fs::write(&cnf_path, dimacs).expect("write cnf");

    let output = Command::new(env!("CARGO_BIN_EXE_sat-solver"))
        .env_clear()
        .env("SAT_FACTOR", "on")
        .arg(&cnf_path)
        .arg(&out_dir)
        .output()
        .expect("run sat-solver");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let _ = fs::remove_dir_all(&out_dir);

    assert_eq!(output.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("s SATISFIABLE"), "stdout: {stdout}");
    assert!(
        stderr.contains("c factor_bva fresh_vars=1"),
        "factoring must fire: {stderr}"
    );

    let mut values = std::collections::HashMap::new();
    for line in stdout.lines().filter(|line| line.starts_with('v')) {
        for token in line.split_whitespace().skip(1) {
            let lit: i32 = token.parse().expect("literal token");
            if lit == 0 {
                continue;
            }
            let var = lit.unsigned_abs();
            assert!(
                (1..=23).contains(&var),
                "model must not mention fresh variables: {lit}"
            );
            values.insert(var, lit > 0);
        }
    }
    assert_eq!(values.len(), 23, "model must cover the original variables");
    for clause in &original_clauses {
        assert!(
            clause
                .iter()
                .any(|&lit| values[&lit.unsigned_abs()] == (lit > 0)),
            "original clause {clause:?} unsatisfied"
        );
    }
}
