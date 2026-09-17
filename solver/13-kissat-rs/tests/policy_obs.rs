// observe(): the policy's input vector is pure, logged with every row, and
// recomputable offline from the log to the float (plan §3, step A.9, bead
// SAT-playground-p9m.6.9). The horizon variables pick the budget source.

mod common;

use common::{assert_same_trajectory, random_3sat, run, Fixture};

fn check_with_python(log: &std::path::Path) -> (i32, String) {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/policy_obs.py");
    let out = std::process::Command::new("python3")
        .arg(script)
        .arg("--check")
        .arg(log)
        .output()
        .expect("python3 tools/policy_obs.py");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn header_of(log: &std::path::Path) -> String {
    let bytes = std::fs::read(log).expect("log");
    let nl1 = bytes.iter().position(|&b| b == b'\n').unwrap();
    let nl2 = nl1 + 1 + bytes[nl1 + 1..].iter().position(|&b| b == b'\n').unwrap();
    String::from_utf8_lossy(&bytes[nl1 + 1..nl2]).into_owned()
}

#[test]
fn observation_is_recomputable_offline_under_a_tick_limit() {
    let fx = Fixture::new("policy-obs-limit");
    let cnf = fx.path("r3.cnf");
    random_3sat(250, 4.26, 1, &cnf);
    let log = fx.path("run.log");
    // Tiny epochs: hundreds of boundary rows, all three delta windows
    // valid, several passes running inside epochs; the tick limit is the
    // horizon's budget.
    let r = run(
        &cnf,
        &[],
        &[
            ("SAT_POLICY", "stock"),
            ("SAT_POLICY_EPOCH_TICKS", "50000,200000"),
            ("SAT_POLICY_LOG", log.to_str().unwrap()),
            ("SAT_LIMIT_TICKS", "40000000"),
        ],
    );
    assert!(r.status == "SATISFIABLE" || r.status == "UNKNOWN", "{}", r.status);
    let header = header_of(&log);
    assert!(header.contains("\"horizon\":\"limit\""), "{}", header);
    assert!(header.contains("\"obs_g_horizon\""), "the vector is a set of columns");
    assert!(header.contains("\"row_boundary\""));
    let (code, text) = check_with_python(&log);
    assert_eq!(code, 0, "{}", text);
    assert!(text.contains("0 mismatches"), "{}", text);
    assert!(r.stdout.contains("policy horizon: work over the tick limit"), "{}", r.stdout);
}

#[test]
fn observation_is_recomputable_offline_in_random_mode_with_a_wall_horizon() {
    let fx = Fixture::new("policy-obs-wall");
    let cnf = fx.path("r3.cnf");
    random_3sat(250, 4.26, 2, &cnf);
    let log = fx.path("run.log");
    let r = run(
        &cnf,
        &["--conflicts=30000"],
        &[
            ("SAT_POLICY", "random"),
            ("SAT_POLICY_SEED", "5"),
            ("SAT_POLICY_EPOCH_TICKS", "50000,200000"),
            ("SAT_POLICY_LOG", log.to_str().unwrap()),
            ("SAT_WALL_LIMIT", "60"),
        ],
    );
    assert_eq!(r.exit_code & !0x1f, 0, "{}", r.stderr);
    let header = header_of(&log);
    assert!(header.contains("\"horizon\":\"wall\",\"horizon_budget\":60"), "{}", header);
    let (code, text) = check_with_python(&log);
    assert_eq!(code, 0, "{}", text);
}

#[test]
fn explicit_tick_horizon_wins_and_observe_is_trajectory_neutral() {
    let fx = Fixture::new("policy-obs-explicit");
    let cnf = fx.path("r3.cnf");
    random_3sat(250, 4.26, 1, &cnf);
    let plain = run(&cnf, &["--conflicts=20000"], &[]);
    let log = fx.path("run.log");
    let on = run(
        &cnf,
        &["--conflicts=20000"],
        &[
            ("SAT_POLICY", "stock"),
            ("SAT_POLICY_EPOCH_TICKS", "50000,200000"),
            ("SAT_POLICY_LOG", log.to_str().unwrap()),
            ("SAT_POLICY_HORIZON", "ticks:12345678"),
            ("SAT_WALL_LIMIT", "60"),
            ("SAT_LIMIT_TICKS", "999999999"),
        ],
    );
    assert_same_trajectory(&plain, &on, "observe every epoch v policy off");
    let header = header_of(&log);
    assert!(header.contains("\"horizon\":\"ticks\",\"horizon_budget\":12345678"), "{}", header);
    let (code, text) = check_with_python(&log);
    assert_eq!(code, 0, "{}", text);
}

#[test]
fn bad_horizon_settings_are_usage_errors() {
    let fx = Fixture::new("policy-obs-bad");
    let cnf = fx.path("r3.cnf");
    random_3sat(60, 4.0, 1, &cnf);
    for (k, v) in [
        ("SAT_POLICY_HORIZON", "ticks:0"),
        ("SAT_POLICY_HORIZON", "wall:10"),
        ("SAT_POLICY_HORIZON", "12"),
        ("SAT_WALL_LIMIT", "0"),
        ("SAT_WALL_LIMIT", "soon"),
    ] {
        let r = run(&cnf, &[], &[("SAT_POLICY", "stock"), (k, v)]);
        assert_eq!(r.exit_code, 1, "{}={} : {}", k, v, r.stdout);
        assert!(r.stderr.contains(k), "{}={} : {}", k, v, r.stderr);
    }
    // Without any budget the horizon is absent, not an error.
    let r = run(&cnf, &[], &[("SAT_POLICY", "stock")]);
    assert!(r.stdout.contains("policy horizon: none"), "{}", r.stdout);
}
