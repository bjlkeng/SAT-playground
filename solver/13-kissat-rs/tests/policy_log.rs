// SAT_POLICY_LOG: the raw-state logger (plan §5.1, step A.5, bead
// SAT-playground-p9m.6.6). The file must be complete (every counter is
// reconstructible from the rows), self-describing, and writing it must not
// touch the trajectory.

mod common;

use common::{assert_same_trajectory, php, random_3sat, run, run_with_proof, Fixture};
use std::collections::BTreeMap;

#[test]
fn log_may_not_alias_the_input_or_the_proof() {
    let fx = Fixture::new("policy-log-alias");
    let cnf = fx.path("php8.cnf");
    php(8, &cnf);
    let before = std::fs::metadata(&cnf).unwrap().len();
    // The CNF itself, by its own path and through a symlink.
    let r = run(&cnf, &[], &[("SAT_POLICY_LOG", cnf.to_str().unwrap())]);
    assert_eq!(r.exit_code, 1, "{}", r.stdout);
    assert!(r.stderr.contains("SAT_POLICY_LOG") && r.stderr.contains("input"), "{}", r.stderr);
    let link = fx.path("link.cnf");
    std::os::unix::fs::symlink(&cnf, &link).unwrap();
    let r = run(&cnf, &[], &[("SAT_POLICY_LOG", link.to_str().unwrap())]);
    assert_eq!(r.exit_code, 1, "{}", r.stdout);
    assert_eq!(std::fs::metadata(&cnf).unwrap().len(), before, "the CNF was truncated");
    // The proof file.
    let proof = fx.path("proof.out");
    let r = run_with_proof(&cnf, &proof, &[("SAT_POLICY_LOG", proof.to_str().unwrap())]);
    assert_eq!(r.exit_code, 1, "{}", r.stdout);
    assert!(r.stderr.contains("proof"), "{}", r.stderr);
    // A dangling symlink whose target is the proof file that does not
    // exist yet.
    let dangling = fx.path("dangling.log");
    let proof2 = fx.path("proof2.out");
    std::os::unix::fs::symlink("proof2.out", &dangling).unwrap();
    let r = run_with_proof(&cnf, &proof2, &[("SAT_POLICY_LOG", dangling.to_str().unwrap())]);
    assert_eq!(r.exit_code, 1, "{}", r.stdout);
    assert!(r.stderr.contains("proof"), "{}", r.stderr);
    assert!(!proof2.exists() || std::fs::metadata(&proof2).unwrap().len() == 0);
    // A distinct log next to a proof is fine and the proof stays valid.
    let log = fx.path("run.log");
    let r = run_with_proof(&cnf, &proof, &[("SAT_POLICY_LOG", log.to_str().unwrap())]);
    assert_eq!(r.status, "UNSATISFIABLE");
    assert!(std::fs::metadata(&proof).unwrap().len() > 0);
    assert!(std::fs::metadata(&log).unwrap().len() > 0);
    // Paths the wrapper reserves (SAT_POLICY_LOG_RESERVED, one per line)
    // get the same treatment, including the binary's whole-string trim of
    // the log path (a leading newline) and a not-yet-existing file.
    let capture = fx.path("solver_stdout.tmp");
    let reserved = format!("{}\n{}\n", capture.display(), fx.path("result.json").display());
    let with_newline = format!("\n{}", capture.display());
    let r = run(
        &cnf,
        &[],
        &[("SAT_POLICY_LOG", &with_newline), ("SAT_POLICY_LOG_RESERVED", &reserved)],
    );
    assert_eq!(r.exit_code, 1, "{}", r.stdout);
    assert!(r.stderr.contains("wrapper-reserved"), "{}", r.stderr);
    assert!(!capture.exists(), "the reserved capture file was created by the log");
    let r = run(
        &cnf,
        &[],
        &[("SAT_POLICY_LOG", log.to_str().unwrap()), ("SAT_POLICY_LOG_RESERVED", &reserved)],
    );
    assert_eq!(r.status, "UNSATISFIABLE", "an unrelated log path is fine");
}

#[test]
fn log_may_not_be_the_file_behind_a_standard_stream() {
    let fx = Fixture::new("policy-log-streams");
    let cnf = fx.path("php8.cnf");
    php(8, &cnf);
    let before = std::fs::metadata(&cnf).unwrap().len();
    let spawn = |stdin: std::fs::File, stdout: std::fs::File, log: &str| {
        let mut cmd = std::process::Command::new(common::solver_bin());
        cmd.arg("-n").arg("-q");
        for key in common::SOLVER_ENV_VARS {
            cmd.env_remove(key);
        }
        cmd.env("SAT_POLICY_LOG", log).stdin(stdin).stdout(stdout);
        cmd.output().expect("run")
    };
    // The CNF arrives on stdin and the log names the same file.
    let out = fx.path("out1.txt");
    let o = spawn(
        std::fs::File::open(&cnf).unwrap(),
        std::fs::File::create(&out).unwrap(),
        cnf.to_str().unwrap(),
    );
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("stdin"), "{}", err);
    assert_eq!(std::fs::metadata(&cnf).unwrap().len(), before, "stdin CNF truncated");
    // The log names the file stdout is redirected to.
    let out = fx.path("out2.txt");
    let o = spawn(
        std::fs::File::open(&cnf).unwrap(),
        std::fs::File::create(&out).unwrap(),
        out.to_str().unwrap(),
    );
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("stdout"), "{}", err);
    // Distinct files: the CNF on stdin and a proper log both work.
    let out = fx.path("out3.txt");
    let log = fx.path("run3.log");
    let o = spawn(
        std::fs::File::open(&cnf).unwrap(),
        std::fs::File::create(&out).unwrap(),
        log.to_str().unwrap(),
    );
    assert_eq!(o.status.code(), Some(20));
    let parsed = parse_log(&std::fs::read(&log).unwrap());
    assert!(parsed.header.contains("\"cnf\":\"<stdin>\""), "{}", parsed.header);
    assert!(parsed.footer.contains("\"result\":\"UNSATISFIABLE\""));
}

#[test]
fn write_failure_is_reported_and_leaves_no_footer() {
    let fx = Fixture::new("policy-log-full");
    let cnf = fx.path("php8.cnf");
    php(8, &cnf);
    let r = run(&cnf, &[], &[("SAT_POLICY_LOG", "/dev/full")]);
    assert_eq!(r.status, "UNSATISFIABLE", "the answer is still given");
    assert_eq!(r.exit_code, 20);
    assert!(
        r.stdout.contains("warning:") && r.stdout.contains("policy log") && r.stdout.contains("incomplete"),
        "{}",
        r.stdout
    );
}

#[test]
fn sigterm_leaves_a_complete_log_with_a_footer() {
    // A cell that runs for minutes with rows every 1000 search ticks: a
    // SIGTERM lands during or between row writes; either way the file must
    // end with the sentinel and a footer that says "signal".
    let fx = Fixture::new("policy-log-sigterm");
    let cnf = fx.path("r3big.cnf");
    random_3sat(450, 4.26, 3, &cnf);
    for attempt in 0..6 {
        let log = fx.path(&format!("run{}.log", attempt));
        let mut cmd = std::process::Command::new(common::solver_bin());
        cmd.arg("-n").arg("-q").arg(&cnf);
        for key in common::SOLVER_ENV_VARS {
            cmd.env_remove(key);
        }
        cmd.env("SAT_POLICY_LOG", &log)
            .env("SAT_POLICY_EPOCH_TICKS", "1000,16000")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = cmd.spawn().expect("spawn");
        std::thread::sleep(std::time::Duration::from_millis(200 + 37 * attempt));
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        let _ = child.wait();
        let parsed = parse_log(&std::fs::read(&log).expect("log exists"));
        assert!(parsed.rows.len() > 1, "attempt {}: {} rows", attempt, parsed.rows.len());
        assert!(
            parsed.footer.contains("\"reason\":\"signal\"") && parsed.footer.contains("\"rows\":"),
            "attempt {}: footer {:?}",
            attempt,
            parsed.footer
        );
    }
}

const MAGIC: &str = "SAT13POLICYLOG 1";

struct Log {
    header: String,
    columns: Vec<String>,
    kinds: Vec<char>,
    rows: Vec<Vec<u64>>,
    footer: String,
}

/// A small hand parser for the format (no JSON crate in the tests either):
/// the header is one line; `columns` is the JSON string array after
/// `"columns":[`, `kinds` the string after `"kinds":"`.
fn parse_log(bytes: &[u8]) -> Log {
    let nl1 = bytes.iter().position(|&b| b == b'\n').expect("magic line");
    assert_eq!(&bytes[..nl1], MAGIC.as_bytes());
    let nl2 = nl1 + 1 + bytes[nl1 + 1..].iter().position(|&b| b == b'\n').expect("header line");
    let header = String::from_utf8(bytes[nl1 + 1..nl2].to_vec()).unwrap();
    let cols_start = header.find("\"columns\":[").expect("columns") + "\"columns\":[".len();
    let cols_end = cols_start + header[cols_start..].find(']').unwrap();
    let columns: Vec<String> = header[cols_start..cols_end]
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .collect();
    let kinds_start = header.find("\"kinds\":\"").expect("kinds") + "\"kinds\":\"".len();
    let kinds_end = kinds_start + header[kinds_start..].find('"').unwrap();
    let kinds: Vec<char> = header[kinds_start..kinds_end].chars().collect();
    assert_eq!(kinds.len(), columns.len());
    let row_bytes = 8 * columns.len();
    let mut pos = nl2 + 1;
    let mut rows = Vec::new();
    let mut footer = String::new();
    while pos + row_bytes <= bytes.len() {
        let mut row = Vec::with_capacity(columns.len());
        for i in 0..columns.len() {
            let mut w = [0u8; 8];
            w.copy_from_slice(&bytes[pos + 8 * i..pos + 8 * i + 8]);
            row.push(u64::from_le_bytes(w));
        }
        pos += row_bytes;
        if row[0] == u64::MAX {
            footer = String::from_utf8_lossy(&bytes[pos..]).trim().to_string();
            break;
        }
        rows.push(row);
    }
    Log { header, columns, kinds, rows, footer }
}

fn col(log: &Log, name: &str) -> usize {
    log.columns.iter().position(|c| c == name).unwrap_or_else(|| panic!("column {}", name))
}

#[test]
fn log_is_complete_self_describing_and_trajectory_neutral() {
    let fx = Fixture::new("policy-log");
    let cnf = fx.path("r3.cnf");
    random_3sat(250, 4.26, 1, &cnf);
    let log_path = fx.path("run.log");
    let log_str = log_path.to_str().unwrap();
    let plain = run(&cnf, &[], &[("SAT_POLICY", "stock"), ("SAT_POLICY_EPOCH_TICKS", "50000,200000")]);
    let logged = run(
        &cnf,
        &[],
        &[
            ("SAT_POLICY", "stock"),
            ("SAT_POLICY_EPOCH_TICKS", "50000,200000"),
            ("SAT_POLICY_LOG", log_str),
        ],
    );
    assert_eq!(logged.status, "SATISFIABLE");
    assert_same_trajectory(&plain, &logged, "logging on v off");
    let bytes = std::fs::read(&log_path).expect("log written");
    let log = parse_log(&bytes);
    // Header: configuration and the column list.
    assert!(log.header.contains("\"format\":1"));
    assert!(log.header.contains("\"cnf\":"), "{}", log.header);
    assert!(log.header.contains("\"mode\":\"stock\""));
    assert!(log.header.contains("\"obs_ticks\":50000"));
    assert!(log.header.contains("\"eliminateint\":500"), "options block");
    let names: BTreeMap<&str, usize> = log.columns.iter().enumerate().map(|(i, c)| (c.as_str(), i)).collect();
    assert!(names.len() == log.columns.len(), "duplicate column names");
    for must in [
        "obs_epoch", "search_ticks", "work", "wall_ns", "conflicts", "ticks", "eliminate_resolutions",
        "literals_learned", "avg_f_fast_glue", "avg_s_slow_glue", "lim_reduce_conflicts",
        "delay_sweep_current", "bound_eliminate_max_completed", "tier1_focused", "stable", "level",
        "trail", "unassigned", "arena_wards", "epoch_glue_bin0", "tm_probe_stock_delta",
        "tm_reduce_stock_would_fire", "act_interval_probe", "act_restart_margin", "act_effort_sweep",
        "dec_interval_probe", "policy_rng", "used_f_glue0", "used_f_glue127", "used_s_glue2",
    ] {
        assert!(names.contains_key(must), "column {} missing", must);
    }
    // The clause-use histograms sum to the printed usage counters.
    let last = log.rows.last().unwrap();
    for (tag, counter) in [("f", "clauses_used_focused"), ("s", "clauses_used_stable")] {
        let sum: u64 = (0..128).map(|g| last[col(&log, &format!("used_{}_glue{}", tag, g))]).sum();
        assert_eq!(sum, logged.stats[counter], "used_{} histogram v {}", tag, counter);
    }
    assert_eq!(log.kinds[col(&log, "avg_f_fast_glue")], 'f');
    assert_eq!(log.kinds[col(&log, "conflicts")], 'u');
    // Rows: one per observation epoch plus the terminal row; monotone clocks.
    assert!(log.rows.len() >= 50, "rows {}", log.rows.len());
    let (c_obs, c_ticks, c_wall, c_conf, c_work) = (
        col(&log, "obs_epoch"), col(&log, "search_ticks"), col(&log, "wall_ns"),
        col(&log, "conflicts"), col(&log, "work"),
    );
    for w in log.rows.windows(2) {
        assert!(w[1][c_ticks] >= w[0][c_ticks]);
        assert!(w[1][c_wall] >= w[0][c_wall]);
        assert!(w[1][c_conf] >= w[0][c_conf]);
    }
    assert_eq!(log.rows[0][c_obs], 0, "D0 row first");
    // The terminal row equals the run's own statistics and work clock.
    let last = log.rows.last().unwrap();
    assert_eq!(last[c_conf], logged.stats["conflicts"]);
    assert_eq!(last[c_work], logged.work());
    for (name, value) in &logged.stats {
        if let Some(&i) = names.get(name.as_str()) {
            assert_eq!(last[i], *value, "-s counter {} v last row", name);
        }
    }
    // The per-epoch histogram sums to the learned-clause delta over the run.
    let learned: u64 = log.rows.iter().map(|r| r[col(&log, "epoch_learned")]).sum();
    assert_eq!(learned, logged.stats["clauses_learned"], "epoch_learned sums to clauses_learned");
    // Footer.
    assert!(log.footer.contains("\"result\":\"SATISFIABLE\""), "{}", log.footer);
    assert!(log.footer.contains("\"reason\":\"solve\""));
    assert!(log.footer.contains("\"peak_rss_bytes\":"));
    assert!(log.footer.contains(&format!("\"rows\":{}", log.rows.len())));
}

#[test]
fn log_keeps_the_decided_action_when_a_one_shot_is_consumed() {
    // Wild jitter with tiny epochs: many decisions pick m = 0 for some
    // timer, and the timer fires before the next row, after which the
    // action in force reads 1. The decided action must still say 0.
    let fx = Fixture::new("policy-log-decided");
    let cnf = fx.path("r3.cnf");
    random_3sat(250, 4.26, 1, &cnf);
    let log_path = fx.path("run.log");
    let r = run(
        &cnf,
        &["--conflicts=40000"],
        &[
            ("SAT_POLICY", "jitter"),
            ("SAT_POLICY_SEED", "10"),
            ("SAT_POLICY_TEMP", "100"),
            ("SAT_POLICY_EPOCH_TICKS", "50000,200000"),
            ("SAT_POLICY_LOG", log_path.to_str().unwrap()),
        ],
    );
    assert!(r.exit_code == 0 || r.exit_code == 10 || r.exit_code == 20, "{}", r.stdout);
    let log = parse_log(&std::fs::read(&log_path).unwrap());
    let timers = ["probe", "eliminate", "reduce", "rephase", "reorder", "mode"];
    let mut decided_zero = 0;
    let mut consumed = 0;
    for row in &log.rows {
        for t in timers {
            let act = f64::from_bits(row[col(&log, &format!("act_interval_{}", t))]);
            let dec = f64::from_bits(row[col(&log, &format!("dec_interval_{}", t))]);
            if act == 0.0 {
                assert_eq!(dec, 0.0, "in-force zero without a decided zero");
            }
            if dec == 0.0 {
                decided_zero += 1;
                if act == 1.0 {
                    consumed += 1;
                }
            }
            if dec != 0.0 {
                assert_eq!(act, dec, "a non-zero decision never changes within its epoch");
            }
        }
    }
    assert!(decided_zero > 0, "no one-shot decision in {} rows", log.rows.len());
    assert!(consumed > 0, "no consumed one-shot seen in {} rows", log.rows.len());
    assert!(log.header.contains("\"mode\":\"jitter\""));
}

#[test]
fn sigterm_before_the_first_row_still_leaves_a_complete_log() {
    // A million clause pairs: parsing and lucky take well over the delay,
    // and no observation row exists yet when the signal lands. The header
    // was written at start, so the terminal row and footer still get out.
    let fx = Fixture::new("policy-log-early-sigterm");
    let cnf = fx.path("pairs.cnf");
    common::lucky_pairs(1_000_000, &cnf);
    let mut complete = 0;
    for attempt in 0..3 {
        let log = fx.path(&format!("run{}.log", attempt));
        let mut cmd = std::process::Command::new(common::solver_bin());
        cmd.arg("-n").arg("-q").arg(&cnf);
        for key in common::SOLVER_ENV_VARS {
            cmd.env_remove(key);
        }
        cmd.env("SAT_POLICY_LOG", &log)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = cmd.spawn().expect("spawn");
        std::thread::sleep(std::time::Duration::from_millis(150 + 60 * attempt));
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        let status = child.wait().unwrap();
        let parsed = parse_log(&std::fs::read(&log).expect("log exists"));
        if status.code() == Some(10) {
            // Finished before the kill: a normal complete file.
            assert!(parsed.footer.contains("\"reason\":\"solve\""));
            continue;
        }
        assert!(parsed.header.contains("\"format\":1"), "attempt {}: no header", attempt);
        assert!(
            parsed.footer.contains("\"reason\":\"signal\"") && parsed.footer.contains("\"static\":{}"),
            "attempt {}: footer {:?} with {} rows",
            attempt,
            parsed.footer,
            parsed.rows.len()
        );
        assert!(parsed.rows.len() >= 1, "attempt {}: terminal row missing", attempt);
        complete += 1;
    }
    assert!(complete >= 1, "every attempt finished before the kill; enlarge the fixture");
}

#[test]
fn sigterm_during_model_output_does_not_seal_a_successful_footer() {
    // 30 k variables solved by lucky in a few ms; the `v` lines are a few
    // hundred KB, so with stdout a pipe nobody reads the solver blocks in
    // the witness write. A SIGTERM there must end the log as "signal" with
    // an UNKNOWN result, not as a completed SATISFIABLE record.
    let fx = Fixture::new("policy-log-output-sigterm");
    let cnf = fx.path("pairs.cnf");
    common::lucky_pairs(15_000, &cnf);
    // Both with and without -q: without it the signal handler prints, and
    // the interrupted model write holds the stdout lock, so the log must be
    // sealed before that print.
    for (i, quiet) in [true, false].into_iter().enumerate() {
        let log = fx.path(&format!("run{}.log", i));
        let mut cmd = std::process::Command::new(common::solver_bin());
        if quiet {
            cmd.arg("-q");
        }
        cmd.arg(&cnf); // witness printing on
        for key in common::SOLVER_ENV_VARS {
            cmd.env_remove(key);
        }
        cmd.env("SAT_POLICY_LOG", &log)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = cmd.spawn().expect("spawn");
        std::thread::sleep(std::time::Duration::from_millis(700));
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        let status = child.wait().unwrap();
        assert!(status.code() != Some(10), "quiet={}: the run should have been killed: {:?}", quiet, status);
        let parsed = parse_log(&std::fs::read(&log).expect("log exists"));
        assert!(
            parsed.footer.contains("\"reason\":\"signal\"") && parsed.footer.contains("\"result\":\"UNKNOWN\""),
            "quiet={}: footer {:?}",
            quiet,
            parsed.footer
        );
    }
}

#[test]
fn log_alone_turns_the_policy_on_in_stock_mode() {
    let fx = Fixture::new("policy-log-alone");
    let cnf = fx.path("php8.cnf");
    php(8, &cnf);
    let log_path = fx.path("run.log");
    let plain = run(&cnf, &[], &[]);
    let logged = run(&cnf, &[], &[("SAT_POLICY_LOG", log_path.to_str().unwrap())]);
    assert_same_trajectory(&plain, &logged, "log alone v off");
    assert!(logged.stdout.contains("policy mode: stock action every epoch"));
    let log = parse_log(&std::fs::read(&log_path).unwrap());
    assert!(log.footer.contains("\"result\":\"UNSATISFIABLE\""));
    assert_eq!(log.rows.last().unwrap()[col(&log, "conflicts")], logged.stats["conflicts"]);
}

#[test]
fn log_under_a_tick_limit_ends_with_unknown_and_the_limit_state() {
    let fx = Fixture::new("policy-log-limit");
    let cnf = fx.path("php8.cnf");
    php(8, &cnf);
    let log_path = fx.path("run.log");
    let r = run(
        &cnf,
        &[],
        &[("SAT_LIMIT_TICKS", "1500000"), ("SAT_POLICY_LOG", log_path.to_str().unwrap())],
    );
    assert_eq!(r.status, "UNKNOWN");
    let log = parse_log(&std::fs::read(&log_path).unwrap());
    assert!(log.footer.contains("\"result\":\"UNKNOWN\""));
    // During search the limit is in force (D0 row); the terminal row is
    // written after kissat's stop_search cleared the one-search limit flags,
    // exactly as it clears `limited.conflicts`.
    let first = &log.rows[0];
    assert_eq!(first[col(&log, "limited_ticks")], 1);
    assert_eq!(first[col(&log, "lim_ticks")], 1_500_000);
    let last = log.rows.last().unwrap();
    assert_eq!(last[col(&log, "limited_ticks")], 0);
    assert_eq!(last[col(&log, "lim_ticks")], 1_500_000);
    assert_eq!(last[col(&log, "work")], r.work());
    assert!(last[col(&log, "work")] >= 1_500_000);
}

#[test]
fn unwritable_log_path_is_a_usage_error() {
    let fx = Fixture::new("policy-log-bad");
    let cnf = fx.path("php8.cnf");
    php(8, &cnf);
    let bad = fx.path("no-such-dir").join("run.log");
    let r = run(&cnf, &[], &[("SAT_POLICY_LOG", bad.to_str().unwrap())]);
    assert_eq!(r.exit_code, 1);
    assert!(r.stderr.contains("SAT_POLICY_LOG"), "{}", r.stderr);
}
