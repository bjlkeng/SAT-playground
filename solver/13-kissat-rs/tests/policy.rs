// RL scheduler plumbing: the stock action is trajectory-neutral, random
// mode replays from its seed, and bad settings are usage errors (plan
// step A, beads SAT-playground-p9m.6.2 / .6.3 / .6.4).

mod common;

use common::{assert_same_trajectory, php, random_3sat, run, Fixture};

/// Small epochs so a one-second cell takes hundreds of observation epochs
/// and dozens of decisions.
const EPOCHS: &str = "50000,200000";

#[test]
fn stock_action_is_trajectory_neutral_on_random_3sat() {
    let fx = Fixture::new("policy-stock-r3");
    let cnf = fx.path("r3.cnf");
    random_3sat(250, 4.26, 1, &cnf);
    let plain = run(&cnf, &[], &[]);
    assert_eq!(plain.status, "SATISFIABLE");
    let on = run(&cnf, &[], &[("SAT_POLICY", "stock"), ("SAT_POLICY_EPOCH_TICKS", EPOCHS)]);
    assert_same_trajectory(&plain, &on, "policy-on-STOCK v policy-off");
    // Default epochs too (D0 plus a handful of observation epochs).
    let on = run(&cnf, &[], &[("SAT_POLICY", "stock")]);
    assert_same_trajectory(&plain, &on, "policy-on-STOCK v policy-off (default epochs)");
    assert!(on.stdout.contains("policy mode: stock action every epoch"), "{}", on.stdout);
}

#[test]
fn stock_action_is_trajectory_neutral_on_php_with_a_conflict_limit() {
    let fx = Fixture::new("policy-stock-php");
    let cnf = fx.path("php9.cnf");
    php(9, &cnf);
    let plain = run(&cnf, &["--conflicts=20000"], &[]);
    assert_eq!(plain.status, "UNKNOWN");
    let on = run(
        &cnf,
        &["--conflicts=20000"],
        &[("SAT_POLICY", "stock"), ("SAT_POLICY_EPOCH_TICKS", "20000,80000")],
    );
    assert_same_trajectory(&plain, &on, "policy-on-STOCK v policy-off under --conflicts");
}

#[test]
fn seeded_random_mode_replays_to_identical_counters_and_moves_the_trajectory() {
    let fx = Fixture::new("policy-random");
    let cnf = fx.path("r3.cnf");
    random_3sat(250, 4.26, 1, &cnf);
    let plain = run(&cnf, &["--conflicts=40000"], &[]);
    let env = [
        ("SAT_POLICY", "random"),
        ("SAT_POLICY_SEED", "7"),
        ("SAT_POLICY_EPOCH_TICKS", EPOCHS),
    ];
    let a = run(&cnf, &["--conflicts=40000"], &env);
    let b = run(&cnf, &["--conflicts=40000"], &env);
    assert_same_trajectory(&a, &b, "two seeded random runs");
    assert!(
        a.stats != plain.stats,
        "random mode with dozens of decisions should change some counter"
    );
    // Another seed is another trajectory.
    let c = run(
        &cnf,
        &["--conflicts=40000"],
        &[
            ("SAT_POLICY", "random"),
            ("SAT_POLICY_SEED", "8"),
            ("SAT_POLICY_EPOCH_TICKS", EPOCHS),
        ],
    );
    assert!(c.stats != a.stats, "seed 7 and seed 8 should differ");
}

#[test]
fn seeded_jitter_mode_replays_to_identical_counters() {
    let fx = Fixture::new("policy-jitter");
    let cnf = fx.path("r3.cnf");
    random_3sat(250, 4.26, 1, &cnf);
    let env = [
        ("SAT_POLICY", "jitter"),
        ("SAT_POLICY_SEED", "3"),
        ("SAT_POLICY_TEMP", "0.7"),
        ("SAT_POLICY_EPOCH_TICKS", EPOCHS),
    ];
    let a = run(&cnf, &["--conflicts=40000"], &env);
    let b = run(&cnf, &["--conflicts=40000"], &env);
    assert_same_trajectory(&a, &b, "two seeded jitter runs");
    assert!(a.stdout.contains("policy mode: per-decision jitter"), "{}", a.stdout);
}

#[test]
fn random_mode_keeps_the_answer_right() {
    // Scheduling is answer-neutral: a perturbed run of an UNSAT cell must
    // still say UNSAT, and of a SAT cell SAT.
    let fx = Fixture::new("policy-answers");
    let unsat = fx.path("php8.cnf");
    php(8, &unsat);
    let sat = fx.path("r3.cnf");
    random_3sat(250, 4.26, 1, &sat);
    for seed in ["1", "2", "3"] {
        let env = [
            ("SAT_POLICY", "random"),
            ("SAT_POLICY_SEED", seed),
            ("SAT_POLICY_TEMP", "100"),
            ("SAT_POLICY_EPOCH_TICKS", EPOCHS),
        ];
        assert_eq!(run(&unsat, &[], &env).status, "UNSATISFIABLE", "seed {}", seed);
        assert_eq!(run(&sat, &[], &env).status, "SATISFIABLE", "seed {}", seed);
    }
}

#[test]
fn bad_policy_settings_are_usage_errors() {
    let fx = Fixture::new("policy-errors");
    let cnf = fx.path("php8.cnf");
    php(8, &cnf);
    for (k, v) in [
        ("SAT_POLICY", "net.bin"),
        ("SAT_POLICY_EPOCH_TICKS", "0"),
        ("SAT_POLICY_EPOCH_TICKS", "100,50"),
        ("SAT_POLICY_EPOCH_TICKS", "100,150"),
        ("SAT_POLICY_EPOCH_TICKS", "1,2,3"),
        ("SAT_POLICY_SEED", "-1"),
        ("SAT_POLICY_TEMP", "0"),
        ("SAT_POLICY_SEGMENT", "0.5"),
    ] {
        let mut env = vec![("SAT_POLICY", "stock")];
        env.push((k, v));
        let r = run(&cnf, &[], &env);
        assert_eq!(r.exit_code, 1, "{}={} stdout:\n{}", k, v, r.stdout);
        assert!(r.stderr.contains(k), "{}={} stderr:\n{}", k, v, r.stderr);
    }
    // Empty SAT_POLICY is off, and the other variables are then ignored.
    let r = run(&cnf, &[], &[("SAT_POLICY", ""), ("SAT_POLICY_SEED", "junk")]);
    assert_eq!(r.status, "UNSATISFIABLE");
    assert!(!r.stdout.contains("[ policy ]"));
}
