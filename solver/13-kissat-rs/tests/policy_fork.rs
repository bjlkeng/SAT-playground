// Fork mode: children branch off at decision points into their own files,
// a stock child matches the parent, the parent's output is not duplicated,
// a proof file refuses fork mode, the semaphore bounds live children, and
// a killed parent takes its children with it (plan §5.3 flavour 1, §7 item
// 6c; step A.8, bead SAT-playground-p9m.6.8).

mod common;

use common::{random_3sat, run, Fixture, Run};
use std::collections::BTreeMap;
use std::path::Path;

const EPOCHS: &str = "50000,200000";
/// A tick budget no test cell reaches: fork mode requires one.
const TICKS: &str = "4000000000";

/// The `-s` counters and the status of a child's captured stdout.
fn parse_out(path: &Path) -> Run {
    let stdout = std::fs::read_to_string(path).unwrap_or_default();
    let mut status = String::new();
    let mut stats = BTreeMap::new();
    let mut workclock = BTreeMap::new();
    let mut in_stats = false;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("s ") {
            status = rest.trim().to_string();
        } else if line.contains("[ statistics ]") {
            in_stats = true;
        } else if in_stats && line.contains("[ ") && line.contains(" ]") {
            in_stats = false;
        } else if let Some(rest) = line.strip_prefix("c workclock ") {
            for tok in rest.split_whitespace() {
                if let Some((k, v)) = tok.split_once('=') {
                    if let Ok(v) = v.parse::<u64>() {
                        workclock.insert(k.to_string(), v);
                    }
                }
            }
        } else if in_stats {
            if let Some(rest) = line.strip_prefix("c ") {
                if let Some((name, tail)) = rest.split_once(':') {
                    if let Some(first) = tail.split_whitespace().next() {
                        if let Ok(v) = first.parse::<u64>() {
                            stats.insert(name.to_string(), v);
                        }
                    }
                }
            }
        }
    }
    Run { status, stats, workclock, stdout, stderr: String::new(), exit_code: 0 }
}

fn header_and_footer(log: &Path) -> (String, String) {
    let bytes = std::fs::read(log).expect("log exists");
    let nl1 = bytes.iter().position(|&b| b == b'\n').unwrap();
    let nl2 = nl1 + 1 + bytes[nl1 + 1..].iter().position(|&b| b == b'\n').unwrap();
    let header = String::from_utf8_lossy(&bytes[nl1 + 1..nl2]).into_owned();
    // The footer is the text after the last newline-terminated binary
    // block: find the last '{"result"'.
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let footer = match text.rfind("{\"result\"") {
        Some(i) => text[i..].trim().to_string(),
        None => String::new(),
    };
    (header, footer)
}

fn rows_of(log: &Path, col: &str) -> Vec<u64> {
    let py = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/policy_log.py");
    let o = std::process::Command::new("python3")
        .arg(py)
        .arg(log)
        .arg("--tail")
        .arg("100000")
        .arg("--columns")
        .arg(col)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&o.stdout).into_owned();
    text.lines().skip_while(|l| !l.starts_with(col)).skip(1).filter_map(|l| l.trim().parse::<u64>().ok()).collect()
}

#[test]
fn stock_child_matches_the_parent_and_nothing_is_duplicated() {
    let fx = Fixture::new("policy-fork-stock");
    let cnf = fx.path("r3.cnf");
    random_3sat(250, 4.26, 1, &cnf);
    let log = fx.path("run.log");
    let env = [
        ("SAT_POLICY", "stock"),
        ("SAT_POLICY_EPOCH_TICKS", EPOCHS),
        ("SAT_POLICY_LOG", log.to_str().unwrap()),
        ("SAT_LIMIT_TICKS", "30000000"),
        ("SAT_POLICY_BRANCH", "2:probe"),
        ("SAT_POLICY_BRANCH_ACTIONS", "probe=1"),
    ];
    let parent = run(&cnf, &[], &env);
    let plain = run(
        &cnf,
        &[],
        &[("SAT_POLICY", "stock"), ("SAT_POLICY_EPOCH_TICKS", EPOCHS), ("SAT_LIMIT_TICKS", "30000000")],
    );
    common::assert_same_trajectory(&plain, &parent, "parent with a branch point v no branching");
    // Exactly one `s` line and one statistics block in the parent's stdout.
    assert_eq!(parent.stdout.lines().filter(|l| l.starts_with("s ")).count(), 1, "{}", parent.stdout);
    assert_eq!(parent.stdout.matches("[ statistics ]").count(), 1);
    assert!(parent.stdout.contains("policy branch: 1 points, 1 children forked, 1 reaped, 0 abnormal"), "{}", parent.stdout);
    // The child: its own out/err/log, the same trajectory at the same
    // tick limit, a header with the branch block, a complete footer.
    let child_log = fx.path("run.log.b2.0");
    let child_out = fx.path("run.log.b2.0.out");
    assert!(child_log.exists() && child_out.exists() && fx.path("run.log.b2.0.err").exists());
    let child = parse_out(&child_out);
    common::assert_same_trajectory(&parent, &child, "stock child v parent");
    assert_eq!(child.stdout.lines().filter(|l| l.starts_with("s ")).count(), 1);
    assert!(child.stdout.contains("policy branch child: decision 2 knob probe entry 1 index 0"), "{}", child.stdout);
    let (h, f) = header_and_footer(&child_log);
    assert!(h.contains("\"branch\":{\"parent_pid\":"), "{}", &h[..300]);
    // X_d is four X_o here, so decision 2 is boundary 8, after rows 0..8.
    assert!(h.contains("\"decision\":2,\"epoch\":8,\"knob\":\"probe\",\"entry\":1,\"index\":0,\"parent_rows\":9,\"masked_to_parent\":true"), "{}", h);
    assert!(f.contains("\"reason\":\"solve\"") && f.contains("\"branch\":{\"parent_pid\":"), "{}", f);
    // The child's first row is the branch state, not a boundary; from
    // then on its rows are boundaries like the parent's.
    let boundary = rows_of(&child_log, "row_boundary");
    assert!(boundary.len() > 3, "{:?}", boundary);
    assert_eq!(boundary[0], 0);
    assert_eq!(boundary[1], 1);
    let epochs = rows_of(&child_log, "obs_epoch");
    assert_eq!(epochs[0], 9, "the child's first row is written after the parent's ninth boundary");
    assert_eq!(epochs[1], 9, "its next row is that boundary's successor");
    // The parent's log: no duplicated rows (the count in its footer equals
    // the rows present), and its footer lists the child.
    let (ph, pf) = header_and_footer(&log);
    assert!(ph.contains("\"branch\":null"));
    assert!(pf.contains("\"branches\":[{\"decision\":2,\"knob\":\"probe\",\"entry\":1,\"index\":0,\"pid\":"), "{}", pf);
    let rows = rows_of(&log, "row");
    let n: u64 = pf.split("\"rows\":").nth(1).unwrap().split(',').next().unwrap().parse().unwrap();
    assert_eq!(rows.len() as u64, n);
    assert_eq!(rows, (0..n).collect::<Vec<u64>>());
}

#[test]
fn default_children_cover_the_menu_and_agree_on_the_answer() {
    let fx = Fixture::new("policy-fork-menu");
    let cnf = fx.path("r3.cnf");
    random_3sat(250, 4.26, 1, &cnf);
    let log = fx.path("run.log");
    let env = [
        ("SAT_POLICY", "stock"),
        ("SAT_POLICY_EPOCH_TICKS", EPOCHS),
        ("SAT_POLICY_LOG", log.to_str().unwrap()),
        ("SAT_POLICY_BRANCH", "1:reduce,3:mode"),
        ("SAT_POLICY_BRANCH_JOBS", "3"),
        ("SAT_LIMIT_TICKS", TICKS),
    ];
    let parent = run(&cnf, &[], &env);
    assert_eq!(parent.status, "SATISFIABLE");
    assert!(parent.stdout.contains("policy branch: 2 points, 6 children forked, 6 reaped, 0 abnormal"), "{}", parent.stdout);
    let mut differing = 0;
    for (d, k, n) in [(1u64, "reduce", 4usize), (3, "mode", 2)] {
        for i in 0..n {
            let out = fx.path(&format!("run.log.b{}.{}.out", d, i));
            let child = parse_out(&out);
            assert_eq!(child.status, "SATISFIABLE", "{} child {} of {}", k, i, d);
            if child.stats != parent.stats {
                differing += 1;
            }
            let (h, _) = header_and_footer(&fx.path(&format!("run.log.b{}.{}", d, i)));
            assert!(h.contains(&format!("\"decision\":{},", d)) && h.contains(&format!("\"knob\":\"{}\"", k)), "{}", h);
            // Non-stock entries only: the parent's own (1) is never forked.
            assert!(!h.contains("\"entry\":1,"), "{}", h);
        }
    }
    assert!(differing >= 3, "only {} of 6 children left the parent's trajectory", differing);
    // The parent's footer lists all six with distinct pids.
    let (_, pf) = header_and_footer(&log);
    assert_eq!(pf.matches("\"pid\":").count(), 6, "{}", pf);
}

#[test]
fn fork_mode_is_refused_with_a_proof_or_without_a_log_and_bad_settings_are_errors() {
    let fx = Fixture::new("policy-fork-refuse");
    let cnf = fx.path("r3.cnf");
    random_3sat(60, 4.0, 1, &cnf);
    let log = fx.path("run.log");
    let proof = fx.path("proof.out");
    let r = common::run_with_proof(
        &cnf,
        &proof,
        &[("SAT_POLICY_LOG", log.to_str().unwrap()), ("SAT_POLICY_BRANCH", "0:probe"), ("SAT_LIMIT_TICKS", TICKS)],
    );
    assert_eq!(r.exit_code, 1, "{}", r.stdout);
    assert!(r.stderr.contains("refuses to fork with a proof file"), "{}", r.stderr);
    let r = run(&cnf, &[], &[("SAT_POLICY", "stock"), ("SAT_POLICY_BRANCH", "0:probe"), ("SAT_LIMIT_TICKS", TICKS)]);
    assert_eq!(r.exit_code, 1);
    assert!(r.stderr.contains("needs SAT_POLICY_LOG"), "{}", r.stderr);
    // No tick budget: children would have nothing to stop on.
    let r = run(&cnf, &[], &[("SAT_POLICY_LOG", log.to_str().unwrap()), ("SAT_POLICY_BRANCH", "0:probe")]);
    assert_eq!(r.exit_code, 1);
    assert!(r.stderr.contains("needs SAT_LIMIT_TICKS"), "{}", r.stderr);
    for (k, v) in [
        ("SAT_POLICY_BRANCH", "probe"),
        ("SAT_POLICY_BRANCH", "1:vivify"),
        ("SAT_POLICY_BRANCH", "2:probe,2:reduce"),
        ("SAT_POLICY_BRANCH_ACTIONS", "probe=3"),
        ("SAT_POLICY_BRANCH_JOBS", "0"),
        ("SAT_POLICY_BRANCH_JOBS", "1000"),
    ] {
        let mut env = vec![("SAT_POLICY_LOG", log.to_str().unwrap()), ("SAT_POLICY_BRANCH", "0:probe"), ("SAT_LIMIT_TICKS", TICKS)];
        env.retain(|(key, _)| *key != k);
        env.push((k, v));
        let r = run(&cnf, &[], &env);
        assert_eq!(r.exit_code, 1, "{}={}: {}", k, v, r.stdout);
        assert!(r.stderr.contains(k), "{}={}: {}", k, v, r.stderr);
    }
}

#[test]
fn child_files_that_alias_the_input_or_the_parent_log_are_refused_at_start() {
    let fx = Fixture::new("policy-fork-alias");
    // The input is named like the child's stdout file of a branch at D0.
    let log = fx.path("run.log");
    let cnf = fx.path("run.log.b0.0.out");
    random_3sat(60, 4.0, 1, &cnf);
    let before = std::fs::metadata(&cnf).unwrap().len();
    let r = run(&cnf, &[], &[("SAT_POLICY_LOG", log.to_str().unwrap()), ("SAT_POLICY_BRANCH", "0:probe"), ("SAT_LIMIT_TICKS", TICKS)]);
    assert_eq!(r.exit_code, 1, "{}", r.stdout);
    assert!(r.stderr.contains("SAT_POLICY_BRANCH") && r.stderr.contains("input"), "{}", r.stderr);
    assert_eq!(std::fs::metadata(&cnf).unwrap().len(), before, "the input was truncated");
    // A dangling link from a child's log to the parent's log; and the
    // same, for a point the run would only reach later.
    let cnf = fx.path("x.cnf");
    random_3sat(60, 4.0, 1, &cnf);
    std::os::unix::fs::symlink("run.log", fx.path("run.log.b1.3")).unwrap();
    let r = run(&cnf, &[], &[("SAT_POLICY_LOG", log.to_str().unwrap()), ("SAT_POLICY_BRANCH", "1:reduce"), ("SAT_LIMIT_TICKS", TICKS)]);
    assert_eq!(r.exit_code, 1, "{}", r.stdout);
    assert!(r.stderr.contains("log file"), "{}", r.stderr);
    std::fs::remove_file(fx.path("run.log.b1.3")).unwrap();
    // A later point's log linked to an earlier point's log.
    std::os::unix::fs::symlink("run.log.b0.0", fx.path("run.log.b1.0")).unwrap();
    let r = run(&cnf, &[], &[("SAT_POLICY_LOG", log.to_str().unwrap()), ("SAT_POLICY_BRANCH", "0:mode,1:mode"), ("SAT_LIMIT_TICKS", TICKS)]);
    assert_eq!(r.exit_code, 1, "{}", r.stdout);
    assert!(r.stderr.contains("same file"), "{}", r.stderr);
    std::fs::remove_file(fx.path("run.log.b1.0")).unwrap();
    // With the links gone the same run is fine and the child is a real one
    // whose header names the parent's pid, not its own.
    let r = run(&cnf, &[], &[("SAT_POLICY_LOG", log.to_str().unwrap()), ("SAT_POLICY_BRANCH", "0:mode"), ("SAT_POLICY_EPOCH_TICKS", EPOCHS), ("SAT_LIMIT_TICKS", TICKS)]);
    assert!(r.status == "SATISFIABLE" || r.status == "UNSATISFIABLE", "{}", r.stdout);
    let (ph, _) = header_and_footer(&log);
    let parent_pid: u64 = ph.split("\"pid\":").nth(1).unwrap().split(',').next().unwrap().parse().unwrap();
    let (ch, _) = header_and_footer(&fx.path("run.log.b0.0"));
    assert!(ch.contains(&format!("\"branch\":{{\"parent_pid\":{},", parent_pid)), "{}", ch);
    let child_pid: u64 = ch.split("\"pid\":").nth(1).unwrap().split(',').next().unwrap().parse().unwrap();
    assert_ne!(child_pid, parent_pid);
}

#[test]
fn a_parent_whose_time_limit_expires_stops_its_children() {
    // kissat's own `--time=1` alarm: the parent stops searching, tells its
    // live children to stop (they inherit no alarm), reaps them and exits
    // within seconds instead of waiting for their tick budgets.
    let fx = Fixture::new("policy-fork-alarm");
    let cnf = fx.path("r3big.cnf");
    random_3sat(450, 4.26, 3, &cnf); // minutes of search
    let log = fx.path("run.log");
    let t0 = std::time::Instant::now();
    let r = run(
        &cnf,
        &["--time=1"],
        &[
            ("SAT_POLICY_LOG", log.to_str().unwrap()),
            ("SAT_POLICY_EPOCH_TICKS", "20000,20000"),
            ("SAT_POLICY_BRANCH", "0:reduce"),
            ("SAT_POLICY_BRANCH_JOBS", "4"),
            ("SAT_LIMIT_TICKS", TICKS),
        ],
    );
    let elapsed = t0.elapsed().as_secs_f64();
    assert_eq!(r.status, "UNKNOWN", "{}", r.stdout);
    assert!(elapsed < 20.0, "parent took {:.1} s", elapsed);
    assert!(r.stdout.contains("4 children forked, 4 reaped"), "{}", r.stdout);
    let (_, pf) = header_and_footer(&log);
    let pids: Vec<i32> = pf
        .split("\"pid\":")
        .skip(1)
        .filter_map(|s| s.split(|c: char| !c.is_ascii_digit()).next().and_then(|d| d.parse().ok()))
        .collect();
    assert_eq!(pids.len(), 4, "{}", pf);
    for pid in pids {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid)).unwrap_or_default();
        assert!(stat.is_empty() || stat.contains(") Z "), "child {} still alive", pid);
    }
    for i in 0..4 {
        let (_, cf) = header_and_footer(&fx.path(&format!("run.log.b0.{}", i)));
        assert!(cf.contains("\"reason\":\"signal\"") || cf.contains("\"reason\":\"solve\""), "child {}: {}", i, cf);
    }
}

#[test]
fn a_killed_parent_takes_its_children_with_it() {
    let fx = Fixture::new("policy-fork-kill");
    let cnf = fx.path("r3big.cnf");
    random_3sat(450, 4.26, 3, &cnf); // minutes of search
    let log = fx.path("run.log");
    let mut cmd = std::process::Command::new(common::solver_bin());
    cmd.arg("-n").arg("-q").arg(&cnf);
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    cmd.env("SAT_POLICY_LOG", &log)
        .env("SAT_POLICY_EPOCH_TICKS", "20000,20000")
        .env("SAT_POLICY_BRANCH", "0:reduce,1:probe")
        .env("SAT_POLICY_BRANCH_JOBS", "8")
        .env("SAT_LIMIT_TICKS", TICKS)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().expect("spawn");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    let _ = child.wait();
    let (_, pf) = header_and_footer(&log);
    assert!(pf.contains("\"reason\":\"signal\""), "{}", pf);
    let pids: Vec<i32> = pf
        .split("\"pid\":")
        .skip(1)
        .filter_map(|s| s.split(|c: char| !c.is_ascii_digit()).next().and_then(|d| d.parse().ok()))
        .collect();
    assert_eq!(pids.len(), 8, "{}", pf);
    // Every child is gone within a few seconds (SIGTERM from the parent's
    // handler; a child seals its own log and re-raises).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let alive: Vec<i32> = pids
            .iter()
            .copied()
            .filter(|&pid| {
                let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid)).unwrap_or_default();
                !stat.is_empty() && !stat.contains(") Z ")
            })
            .collect();
        if alive.is_empty() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "children still alive: {:?}", alive);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    for (d, n) in [(0u64, 4usize), (1, 4)] {
        for i in 0..n {
            let (_, cf) = header_and_footer(&fx.path(&format!("run.log.b{}.{}", d, i)));
            assert!(cf.contains("\"reason\":\"signal\"") || cf.contains("\"reason\":\"solve\""), "child {}.{}: {}", d, i, cf);
        }
    }
}
