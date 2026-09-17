// SAT_LIMIT_TICKS: a deterministic work-clock limit (RL plan §3.4, step A.1,
// bead SAT-playground-p9m.6.1).

mod common;

use common::{assert_same_trajectory, lucky_pairs, php, random_3sat, run, Fixture};

#[test]
fn limit_binds_inside_the_lucky_passes() {
    // Lucky solves this cell outright before search; kissat's own limits
    // never bound lucky, the work budget must.
    let fx = Fixture::new("limit-lucky");
    let cnf = fx.path("pairs.cnf");
    lucky_pairs(100_000, &cnf);
    let plain = run(&cnf, &[], &[]);
    assert_eq!(plain.status, "SATISFIABLE");
    assert_eq!(plain.stats["conflicts"], 0, "solved by lucky, not search");
    let total = plain.work();
    assert!(total > 100_000, "fixture too small: work {}", total);
    for limit in ["0", "100", "50000"] {
        let r = run(&cnf, &[], &[("SAT_LIMIT_TICKS", limit)]);
        assert_eq!(r.status, "UNKNOWN", "limit {} stdout:\n{}", limit, r.stdout);
        let limit: u64 = limit.parse().unwrap();
        assert!(r.work() >= limit);
        assert!(
            r.work() < limit + total / 10,
            "limit {}: overran by {} of a {} run",
            limit,
            r.work() - limit,
            total
        );
    }
    // A limit above lucky's own work still lets lucky solve it.
    let value = (total * 2).to_string();
    assert_eq!(run(&cnf, &[], &[("SAT_LIMIT_TICKS", &value)]).status, "SATISFIABLE");
}

#[test]
fn limit_stops_twice_at_identical_counters() {
    let fx = Fixture::new("limit-deterministic");
    let cnf = fx.path("php8.cnf");
    php(8, &cnf);
    let limit: u64 = 1_500_000;
    let value = limit.to_string();
    let a = run(&cnf, &[], &[("SAT_LIMIT_TICKS", &value)]);
    let b = run(&cnf, &[], &[("SAT_LIMIT_TICKS", &value)]);
    assert_eq!(a.status, "UNKNOWN", "stdout:\n{}", a.stdout);
    assert_eq!(a.exit_code, 0);
    assert_same_trajectory(&a, &b, "two runs with the same work-clock limit");
    assert!(a.work() >= limit, "stopped before the limit: {} < {}", a.work(), limit);
    assert!(
        a.work() < 2 * limit,
        "overran the limit by more than itself: {} v {}",
        a.work(),
        limit
    );
    assert!(a.stats["conflicts"] > 0, "the limit should stop a run that has started searching");
}

#[test]
fn limit_is_honoured_within_one_pass_on_a_cell_with_inprocessing() {
    // Random 3-SAT: several probe rounds and two eliminate rounds before it
    // is solved, so limits land inside inprocessing as well as in search.
    let fx = Fixture::new("limit-overrun");
    let cnf = fx.path("r3.cnf");
    random_3sat(250, 4.26, 1, &cnf);
    let full = run(&cnf, &[], &[]);
    assert_ne!(full.status, "UNKNOWN");
    let total = full.work();
    assert!(total > 10_000_000, "fixture too small: work {}", total);
    for frac in [10u64, 25, 50, 75] {
        let limit = total * frac / 100;
        let value = limit.to_string();
        let r = run(&cnf, &[], &[("SAT_LIMIT_TICKS", &value)]);
        assert_eq!(r.status, "UNKNOWN", "limit {} of {}", limit, total);
        assert!(r.work() >= limit, "limit {}: work {}", limit, r.work());
        // A pass polls termination every few resolutions or kitten ticks,
        // so the overrun is a small fraction of the limit.
        let overrun = r.work() - limit;
        assert!(
            overrun * 10 <= limit,
            "limit {} overrun by {} (more than 10%)",
            limit,
            overrun
        );
    }
}

#[test]
fn non_binding_limit_leaves_the_run_unchanged() {
    let fx = Fixture::new("limit-nonbinding");
    let cnf = fx.path("php8.cnf");
    php(8, &cnf);
    let plain = run(&cnf, &[], &[]);
    assert_eq!(plain.status, "UNSATISFIABLE");
    let huge = run(&cnf, &[], &[("SAT_LIMIT_TICKS", "1000000000000")]);
    assert_same_trajectory(&plain, &huge, "unset v non-binding limit");
    let empty = run(&cnf, &[], &[("SAT_LIMIT_TICKS", "")]);
    assert_same_trajectory(&plain, &empty, "unset v empty value");
}

#[test]
fn zero_limit_stops_before_search_with_unknown() {
    let fx = Fixture::new("limit-zero");
    let cnf = fx.path("php8.cnf");
    php(8, &cnf);
    let r = run(&cnf, &[], &[("SAT_LIMIT_TICKS", "0")]);
    assert_eq!(r.status, "UNKNOWN", "stdout:\n{}", r.stdout);
    assert_eq!(r.exit_code, 0);
}

#[test]
fn invalid_value_is_a_usage_error() {
    let fx = Fixture::new("limit-invalid");
    let cnf = fx.path("php8.cnf");
    php(8, &cnf);
    let r = run(&cnf, &[], &[("SAT_LIMIT_TICKS", "lots")]);
    assert_eq!(r.exit_code, 1);
    assert!(r.stderr.contains("SAT_LIMIT_TICKS"), "stderr:\n{}", r.stderr);
    assert!(r.status.is_empty(), "no s line expected, got {}", r.status);
}

#[test]
fn limit_is_reported_in_the_limits_section() {
    let fx = Fixture::new("limit-report");
    let cnf = fx.path("php8.cnf");
    php(8, &cnf);
    let r = run(&cnf, &[], &[("SAT_LIMIT_TICKS", "123456")]);
    assert!(
        r.stdout.contains("work clock limit set to 123456"),
        "stdout:\n{}",
        r.stdout
    );
}
