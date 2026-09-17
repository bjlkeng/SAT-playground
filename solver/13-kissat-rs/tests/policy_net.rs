// The learned policy end to end: a weights file is loaded from
// SAT_POLICY, an infinite margin (or a stock-biased net at a working
// margin) is trajectory-identical to policy-off, a net that prefers
// non-stock entries moves the trajectory deterministically and still
// answers correctly, the log records the scores, and bad files or margins
// are usage errors (plan §6.4, step A.10, bead SAT-playground-p9m.6.10).

mod common;

use common::{assert_same_trajectory, random_3sat, run, Fixture};

const EPOCHS: &str = "50000,200000";

fn make_net(fx: &Fixture, log: &std::path::Path, name: &str, seed: u32, bias: f64) -> std::path::PathBuf {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/rl/policy_net.py");
    let out = fx.path(name);
    let o = std::process::Command::new("python3")
        .arg(script)
        .arg("--fixture")
        .arg(log)
        .arg(&out)
        .arg("--seed")
        .arg(seed.to_string())
        .arg("--stock-bias")
        .arg(bias.to_string())
        .output()
        .expect("python3 tools/rl/policy_net.py --fixture");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    out
}

fn summary_line(stdout: &str) -> &str {
    stdout
        .lines()
        .find(|l| l.starts_with("c policy net: ") && l.contains("decisions"))
        .unwrap_or("")
}

#[test]
fn net_with_infinite_margin_or_stock_bias_is_trajectory_neutral() {
    let fx = Fixture::new("policy-net-neutral");
    let cnf = fx.path("r3.cnf");
    random_3sat(250, 4.26, 1, &cnf);
    // A log from a stock run gives the observation layout the file needs.
    let log = fx.path("layout.log");
    let plain = run(
        &cnf,
        &[],
        &[("SAT_POLICY", "stock"), ("SAT_POLICY_EPOCH_TICKS", EPOCHS), ("SAT_POLICY_LOG", log.to_str().unwrap())],
    );
    assert_eq!(plain.status, "SATISFIABLE");
    let off = run(&cnf, &[], &[]);
    assert_same_trajectory(&off, &plain, "policy-on-STOCK v off");
    // Stock far ahead in every head (the "large positive score" of plan
    // §6.4 (1); a random fixture's heads spread by tens on raw log-scale
    // inputs): at a working margin the net never leaves stock, at an
    // infinite margin it cannot.
    let net = make_net(&fx, &log, "biased.bin", 1, 1000.0);
    for margin in ["inf", "1", "0.25"] {
        let r = run(
            &cnf,
            &[],
            &[
                ("SAT_POLICY", net.to_str().unwrap()),
                ("SAT_POLICY_EPOCH_TICKS", EPOCHS),
                ("SAT_POLICY_MARGIN", margin),
            ],
        );
        assert_same_trajectory(&off, &r, &format!("net (stock biased) at margin {}", margin));
        assert!(r.stdout.contains("policy mode: learned policy"), "{}", r.stdout);
        let line = summary_line(&r.stdout);
        assert!(line.contains(" 0 not stock (0.0%)"), "margin {}: {:?}", margin, line);
    }
}

#[test]
fn net_that_prefers_other_entries_moves_the_trajectory_deterministically() {
    let fx = Fixture::new("policy-net-deviate");
    let cnf = fx.path("r3.cnf");
    random_3sat(250, 4.26, 1, &cnf);
    let log = fx.path("layout.log");
    let plain = run(
        &cnf,
        &["--conflicts=40000"],
        &[("SAT_POLICY", "stock"), ("SAT_POLICY_EPOCH_TICKS", EPOCHS), ("SAT_POLICY_LOG", log.to_str().unwrap())],
    );
    // Stock behind by 3: at margin 0 some other entry wins almost always.
    let net = make_net(&fx, &log, "wild.bin", 2, -3.0);
    let out_log = fx.path("net.log");
    let env = [
        ("SAT_POLICY", net.to_str().unwrap()),
        ("SAT_POLICY_EPOCH_TICKS", EPOCHS),
        ("SAT_POLICY_MARGIN", "0"),
        ("SAT_POLICY_LOG", out_log.to_str().unwrap()),
    ];
    let a = run(&cnf, &["--conflicts=40000"], &env);
    let b = run(&cnf, &["--conflicts=40000"], &env);
    assert_same_trajectory(&a, &b, "two runs of the same net");
    assert!(a.stats != plain.stats, "a net that prefers non-stock entries should change some counter");
    let line = summary_line(&a.stdout);
    assert!(!line.contains(" 0 not stock"), "{:?}", line);
    // The log carries the header's net block, the 35 scores and the
    // deviation count; the decided action is not always stock.
    let bytes = std::fs::read(&out_log).unwrap();
    let nl1 = bytes.iter().position(|&b| b == b'\n').unwrap();
    let nl2 = nl1 + 1 + bytes[nl1 + 1..].iter().position(|&b| b == b'\n').unwrap();
    let header = String::from_utf8_lossy(&bytes[nl1 + 1..nl2]).into_owned();
    assert!(header.contains("\"net\":{\"path\":"), "{}", &header[..200]);
    assert!(header.contains("\"margin\":0"), "{}", header);
    assert!(header.contains("\"net_probe_0\"") && header.contains("\"net_sweep_3\"") && header.contains("\"net_deviations\""));
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/policy_obs.py");
    let o = std::process::Command::new("python3").arg(script).arg("--check").arg(&out_log).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    // The observation vectors of a net run recompute offline like any other.
    let py = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/policy_log.py");
    let o = std::process::Command::new("python3")
        .arg(py)
        .arg(&out_log)
        .arg("--tail")
        .arg("1")
        .arg("--columns")
        .arg("net_deviations,dec_interval_probe,dec_interval_reduce,net_probe_2")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&o.stdout).into_owned();
    let last = text.lines().last().unwrap_or("");
    let fields: Vec<&str> = last.split('\t').collect();
    assert_eq!(fields.len(), 4, "{}", text);
    assert!(fields[0].parse::<u64>().unwrap() > 0, "net_deviations {}", text);
    // Wild random runs at every margin still answer correctly.
    let full = run(&cnf, &[], &env);
    assert_eq!(full.status, "SATISFIABLE");
    // Logging must not change what the net decides: the same net without
    // a log takes the same trajectory (the per-epoch learned-clause
    // features are reset at every boundary, not by the row writer).
    let unlogged = run(
        &cnf,
        &["--conflicts=40000"],
        &[("SAT_POLICY", net.to_str().unwrap()), ("SAT_POLICY_EPOCH_TICKS", EPOCHS), ("SAT_POLICY_MARGIN", "0")],
    );
    assert_same_trajectory(&a, &unlogged, "net with a log v without");
}

#[test]
fn the_log_may_not_be_the_weights_file() {
    let fx = Fixture::new("policy-net-alias");
    let cnf = fx.path("r3.cnf");
    random_3sat(60, 4.0, 1, &cnf);
    let log = fx.path("layout.log");
    let _ = run(&cnf, &[], &[("SAT_POLICY_LOG", log.to_str().unwrap())]);
    let net = make_net(&fx, &log, "net.bin", 1, 3.0);
    let before = std::fs::metadata(&net).unwrap().len();
    assert!(before > 1000);
    for log_path in [net.clone(), {
        let link = fx.path("link.log");
        std::os::unix::fs::symlink(&net, &link).unwrap();
        link
    }] {
        let r = run(
            &cnf,
            &[],
            &[("SAT_POLICY", net.to_str().unwrap()), ("SAT_POLICY_LOG", log_path.to_str().unwrap())],
        );
        assert_eq!(r.exit_code, 1, "{}", r.stdout);
        assert!(r.stderr.contains("weights"), "{}", r.stderr);
        assert_eq!(std::fs::metadata(&net).unwrap().len(), before, "the weights file was truncated");
    }
}

#[test]
fn bad_weights_files_and_margins_are_usage_errors() {
    let fx = Fixture::new("policy-net-bad");
    let cnf = fx.path("r3.cnf");
    random_3sat(60, 4.0, 1, &cnf);
    // Not a file: the mode-name message.
    let r = run(&cnf, &[], &[("SAT_POLICY", "netty")]);
    assert_eq!(r.exit_code, 1);
    assert!(r.stderr.contains("stock, random, jitter or the path"), "{}", r.stderr);
    // A file that is not a weights file.
    let r = run(&cnf, &[], &[("SAT_POLICY", cnf.to_str().unwrap())]);
    assert_eq!(r.exit_code, 1);
    assert!(r.stderr.contains("bad magic"), "{}", r.stderr);
    // A truncated weights file.
    let log = fx.path("layout.log");
    let _ = run(&cnf, &[], &[("SAT_POLICY_LOG", log.to_str().unwrap())]);
    let net = make_net(&fx, &log, "net.bin", 1, 3.0);
    let bytes = std::fs::read(&net).unwrap();
    let cut = fx.path("cut.bin");
    std::fs::write(&cut, &bytes[..bytes.len() - 5]).unwrap();
    let r = run(&cnf, &[], &[("SAT_POLICY", cut.to_str().unwrap())]);
    assert_eq!(r.exit_code, 1);
    assert!(r.stderr.contains("truncated"), "{}", r.stderr);
    // Bad margins.
    for m in ["-1", "nan", "much"] {
        let r = run(&cnf, &[], &[("SAT_POLICY", net.to_str().unwrap()), ("SAT_POLICY_MARGIN", m)]);
        assert_eq!(r.exit_code, 1, "margin {}", m);
        assert!(r.stderr.contains("SAT_POLICY_MARGIN"), "{}", r.stderr);
    }
}
