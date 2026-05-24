use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const CONFIG_SCHEMA: &str = "CONFIG_SCHEMA.csv";
const FEATURES: &str = "FEATURES.csv";

fn main() {
    println!("cargo:rerun-if-changed={CONFIG_SCHEMA}");
    println!("cargo:rerun-if-changed={FEATURES}");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    emit_git_rerun_triggers(&manifest_dir);

    let target_generated = manifest_dir.join("target/generated");
    fs::create_dir_all(&target_generated).expect("create target/generated");

    let schema = read(&manifest_dir.join(CONFIG_SCHEMA));
    let features = read(&manifest_dir.join(FEATURES));
    validate_schema(&schema);
    validate_features(&features);

    fs::write(target_generated.join(CONFIG_SCHEMA), &schema).expect("write generated schema");
    fs::write(target_generated.join(FEATURES), &features).expect("write generated features");
    fs::write(
        target_generated.join("README_config_table.md"),
        readme_config_table(&schema),
    )
    .expect("write generated README table");

    println!(
        "cargo:rustc-env=SOLVER_GIT_SHA={}",
        command_output("git", &["rev-parse", "HEAD"])
    );
    println!(
        "cargo:rustc-env=SOLVER_RUSTC_VERSION={}",
        command_output("rustc", &["--version"])
    );
}

fn emit_git_rerun_triggers(manifest_dir: &Path) {
    let git_entry = manifest_dir.join("../../.git");
    if git_entry.is_dir() {
        emit_git_dir_rerun_triggers(&git_entry);
        return;
    }

    println!("cargo:rerun-if-changed={}", git_entry.display());
    let Ok(git_file) = fs::read_to_string(&git_entry) else {
        return;
    };
    let Some(git_dir) = git_file.trim().strip_prefix("gitdir:") else {
        return;
    };
    let git_dir = git_dir.trim();
    let git_dir = if Path::new(git_dir).is_absolute() {
        PathBuf::from(git_dir)
    } else {
        manifest_dir.join("../..").join(git_dir)
    };
    emit_git_dir_rerun_triggers(&git_dir);
}

fn emit_git_dir_rerun_triggers(git_dir: &Path) {
    let head_path = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head_path.display());
    let Ok(head) = fs::read_to_string(&head_path) else {
        return;
    };
    let Some(reference) = head.trim().strip_prefix("ref:") else {
        return;
    };
    let reference = reference.trim();
    println!("cargo:rerun-if-changed={}", git_dir.join(reference).display());
    println!("cargo:rerun-if-changed={}", git_dir.join("packed-refs").display());
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

fn command_output(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn validate_schema(schema: &str) {
    for denied in ["SAT_WALK", "SAT_SWEEP", "SAT_ELS", "SAT_BCE"] {
        assert!(
            !schema.lines().any(|line| line.starts_with(denied)),
            "parked flag {denied} must not appear in CONFIG_SCHEMA.csv"
        );
    }
}

fn validate_features(features: &str) {
    for (line_idx, line) in features.lines().enumerate().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        let columns: Vec<&str> = line.split(',').collect();
        assert!(
            columns.len() >= 8,
            "FEATURES.csv line {} has too few columns",
            line_idx + 1
        );
        let feature = columns[0];
        let promoted_profiles = columns[6];
        let validation_artifact = columns[7];
        if promoted_profiles != "none" {
            assert!(
                !validation_artifact.is_empty(),
                "promoted feature {feature} is missing a validation artifact"
            );
        }
    }
}

fn readme_config_table(schema: &str) -> String {
    let mut out = String::from("| Env var | Field | Type | Default | Notes |\n");
    out.push_str("|---|---|---|---|---|\n");
    for line in schema.lines().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        let columns: Vec<&str> = line.split(',').collect();
        if columns.len() < 14 {
            continue;
        }
        let env_var = columns[0];
        let field = columns[1];
        let type_name = columns[2];
        let default = columns[5];
        let notes = match (columns[7], columns[8], columns[9]) {
            ("true", _, _) => "feature",
            (_, "true", _) => "limit",
            (_, _, "true") => "legacy",
            _ => "",
        };
        out.push_str(&format!(
            "| `{env_var}` | `{field}` | `{type_name}` | `{default}` | {notes} |\n"
        ));
    }
    out
}
