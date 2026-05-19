use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn usage() {
    eprintln!(
        "Usage: sat-bench <subcommand> [args...]\n\
         \n\
         Subcommands:\n\
           status-compare   Compare status columns from two results.csv files\n\
           validate-result  Validate one solver output directory\n\
           select-iter      Create/check solver 11 iteration benchmark sets\n\
           compare          Correctness-first paired benchmark comparison\n\
           extract-hot      Extract hot/regressed instances from benchmark CSVs\n\
           validate-plan    Validate solver 11 source-map and plan scaffolding\n\
           profile          Delegate to tools/profile_solver11.sh when present\n"
    );
}

fn find_repo_root() -> Result<PathBuf, String> {
    let mut dir = env::current_dir().map_err(|err| format!("current_dir failed: {err}"))?;
    loop {
        if dir.join("tools").is_dir() && dir.join("solver").is_dir() {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err("could not find SAT-playground repo root from current directory".to_string());
        }
    }
}

fn run_python(repo: &Path, script: &str, args: &[String]) -> Result<ExitCode, String> {
    let status = Command::new("python3")
        .arg(repo.join("tools").join(script))
        .args(args)
        .status()
        .map_err(|err| format!("failed to run python3 tools/{script}: {err}"))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn run_shell_script(repo: &Path, script: &str, args: &[String]) -> Result<ExitCode, String> {
    let path = repo.join("tools").join(script);
    if !path.exists() {
        return Err(format!(
            "tools/{script} is not present yet; this subcommand is reserved for the 0.5a profiling bead"
        ));
    }
    let status = Command::new("bash")
        .arg(path)
        .args(args)
        .status()
        .map_err(|err| format!("failed to run tools/{script}: {err}"))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn dispatch(repo: &Path, subcommand: &str, args: &[String]) -> Result<ExitCode, String> {
    match subcommand {
        "status-compare" => run_python(repo, "status_compare.py", args),
        "validate-result" => run_python(repo, "validate_solver_result.py", args),
        "select-iter" => run_python(repo, "select_iter_bench.py", args),
        "compare" => run_python(repo, "compare_bench.py", args),
        "extract-hot" => run_python(repo, "extract_hot_instances.py", args),
        "validate-plan" => run_python(repo, "validate_solver11_plan.py", args),
        "profile" => run_shell_script(repo, "profile_solver11.sh", args),
        "-h" | "--help" | "help" => {
            usage();
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown sat-bench subcommand: {other}")),
    }
}

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        usage();
        return ExitCode::from(2);
    }
    let subcommand = args.remove(0);
    let repo = match find_repo_root() {
        Ok(repo) => repo,
        Err(err) => {
            eprintln!("sat-bench: {err}");
            return ExitCode::from(2);
        }
    };
    match dispatch(&repo, &subcommand, &args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("sat-bench: {err}");
            ExitCode::from(2)
        }
    }
}
