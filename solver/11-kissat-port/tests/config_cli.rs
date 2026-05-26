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

    let output = Command::new(env!("CARGO_BIN_EXE_sat-solver"))
        .env_clear()
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
