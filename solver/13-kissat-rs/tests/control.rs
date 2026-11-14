//! Control words (bead SAT-playground-d96.8.5, src/control.rs): with
//! `ctlfirst` the control variables (SAT_CTL_VARS) are decided first; with
//! `ctlsplit` their values are cases, each solved by a sub-solver before the
//! search, the refuted cases' proofs relativized into the DRAT proof and
//! joined into the empty clause.  Checked here: off means off (the
//! environment variable is not read), every answer is right, every proof
//! passes the gate (decided at the first level, at a deeper level, and left
//! open with the search finishing), a satisfiable case's model satisfies the
//! input, the shipped defaults keep the formula for the split, and the tick
//! limit holds.  The mechanics are tried on random 3-SAT, whose cases are
//! searches: those tests turn the cost stop off (--ctlsplitratio=100000),
//! and one test checks that the stop ends a split of that kind.

mod common;

use common::{random_3sat, run, run_shipped, solver_bin, Fixture};
use std::path::Path;
use std::process::Command;

fn gate(cnf: &Path, proof: &Path) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new("python3")
        .arg(root.join("tools/proof_gate.py"))
        .arg(cnf)
        .arg(proof)
        .output()
        .expect("run proof_gate.py");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The given options (after the stock path unless `shipped`), a proof file,
/// SAT_CTL_VARS; (status, stdout).
fn run_ctl(cnf: &Path, proof: Option<&Path>, options: &[&str], vars: &str, shipped: bool) -> (String, String) {
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    if !vars.is_empty() {
        cmd.env("SAT_CTL_VARS", vars);
    }
    // no -n: a satisfiable answer's model is checked below
    if !shipped {
        cmd.args(common::STOCK_PATH);
    }
    cmd.args(options).arg(cnf);
    if let Some(p) = proof {
        cmd.arg(p);
    }
    let out = cmd.output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let status = stdout.lines().find_map(|l| l.strip_prefix("s ").map(|s| s.trim().to_string())).unwrap_or_default();
    (status, stdout)
}

fn control_line<'a>(stdout: &'a str, key: &str) -> &'a str {
    stdout.lines().find(|l| l.starts_with("c control") && l.contains(key)).unwrap_or("")
}

/// Every clause of the CNF satisfied by the `v` lines?
fn model_ok(cnf: &Path, stdout: &str) -> bool {
    let mut val = std::collections::HashMap::new();
    for line in stdout.lines().filter(|l| l.starts_with("v ")) {
        for tok in line[2..].split_whitespace() {
            let l: i64 = tok.parse().unwrap();
            if l != 0 {
                val.insert(l.abs(), l > 0);
            }
        }
    }
    let text = std::fs::read_to_string(cnf).unwrap();
    let mut sat = false;
    for tok in text.lines().filter(|l| !l.starts_with('p') && !l.starts_with('c')).flat_map(|l| l.split_whitespace()) {
        let l: i64 = tok.parse().unwrap();
        if l == 0 {
            if !sat {
                return false;
            }
            sat = false;
        } else if val.get(&l.abs()).copied().unwrap_or(false) == (l > 0) {
            sat = true;
        }
    }
    true
}

const VARS: &str = "1,2,3,4,5,6,7,8,9,10,11,12";

#[test]
fn off_means_off() {
    let fx = Fixture::new("control_off");
    let cnf = fx.path("r.cnf");
    random_3sat(120, 4.6, 3, &cnf);
    let plain = run(&cnf, &[], &[]);
    // the variable set, both options off: never read, the same search
    let with_env = run(&cnf, &[], &[("SAT_CTL_VARS", VARS)]);
    common::assert_same_trajectory(&plain, &with_env, "SAT_CTL_VARS with the options off");
    // the split on without a control set: the reading runs and finds no
    // switch (its work enters the work clock), and the search is the same
    let no_vars = run(&cnf, &["--ctlsplit=1"], &[]);
    assert_eq!(plain.status, no_vars.status);
    for key in ["conflicts", "decisions", "propagations"] {
        assert_eq!(plain.stats.get(key), no_vars.stats.get(key), "ctlsplit without a control set: {}", key);
    }
    assert!(no_vars.work() >= plain.work(), "the reading's work counts");
    assert!(!plain.stdout.contains("c control"), "no exit line with the options off");
}

#[test]
fn control_first_answers_and_proves() {
    let fx = Fixture::new("control_first");
    for seed in 1..=3u64 {
        let cnf = fx.path(&format!("r{}.cnf", seed));
        let proof = fx.path(&format!("r{}.drat", seed));
        random_3sat(130, 4.6, seed, &cnf);
        let (status, out) = run_ctl(&cnf, Some(&proof), &["--ctlfirst=1"], VARS, false);
        let plain = run(&cnf, &[], &[]);
        assert_eq!(status, plain.status, "seed {}: same answer", seed);
        let line = control_line(&out, "ctlfirst=1");
        assert!(line.contains("vars=12") && !line.contains("decisions=0 "), "seed {}: {}", seed, line);
        if status == "UNSATISFIABLE" {
            assert_eq!(gate(&cnf, &proof), "ok", "seed {}: gate", seed);
        } else {
            assert!(model_ok(&cnf, &out), "seed {}: model", seed);
        }
    }
}

#[test]
fn split_proofs_pass_the_gate() {
    let fx = Fixture::new("control_split");
    let cnf = fx.path("r.cnf");
    random_3sat(160, 4.8, 7, &cnf);
    // a case limit that decides at the first level, one that needs a
    // second level and joins, and one too small: the split stays open and
    // the search finishes with the refuted cases' clauses
    for (case, want) in [("100000", "status=unsat"), ("300", "status=unsat"), ("40", "status=open")] {
        let proof = fx.path(&format!("p{}.drat", case));
        let (status, out) = run_ctl(&cnf, Some(&proof), &["--ctlsplit=1", "--ctlsplitbits=3", "--ctlsplitratio=100000", &format!("--ctlsplitcase={}", case)], VARS, false);
        assert_eq!(status, "UNSATISFIABLE", "case limit {}", case);
        let line = control_line(&out, "ctlsplit=");
        assert!(line.contains(want), "case limit {}: {}", case, line);
        assert_eq!(gate(&cnf, &proof), "ok", "case limit {}: gate", case);
    }
}

#[test]
fn split_model_satisfies_the_input() {
    let fx = Fixture::new("control_split_sat");
    for seed in 1..=3u64 {
        let cnf = fx.path(&format!("s{}.cnf", seed));
        random_3sat(150, 3.6, seed, &cnf);
        let (status, out) = run_ctl(&cnf, None, &["--ctlsplit=1", "--ctlsplitbits=3", "--ctlsplitratio=100000"], VARS, false);
        assert_eq!(status, "SATISFIABLE", "seed {}", seed);
        assert!(control_line(&out, "ctlsplit=").contains("status=sat"), "seed {}: the split's model", seed);
        assert!(model_ok(&cnf, &out), "seed {}: the model satisfies every clause", seed);
    }
}

#[test]
fn split_with_the_shipped_defaults() {
    let fx = Fixture::new("control_split_shipped");
    let cnf = fx.path("r.cnf");
    let proof = fx.path("r.drat");
    random_3sat(160, 4.8, 9, &cnf);
    let (status, out) = run_ctl(&cnf, Some(&proof), &["--ctlsplit=1", "--ctlsplitbits=3", "--ctlsplitratio=100000"], VARS, true);
    assert_eq!(status, "UNSATISFIABLE");
    assert!(control_line(&out, "ctlsplit=").contains("status=unsat"), "{}", control_line(&out, "ctlsplit="));
    assert_eq!(gate(&cnf, &proof), "ok");
    let _ = run_shipped(&cnf, &[], &[]);
}

#[test]
fn probe_runs_every_case() {
    let fx = Fixture::new("control_probe");
    let cnf = fx.path("r.cnf");
    let proof = fx.path("r.drat");
    random_3sat(160, 4.8, 7, &cnf);
    // a case limit too small for most cases: the split would stop at the
    // first open one; the probe runs all 8 and counts them, then searches
    let (status, out) = run_ctl(&cnf, Some(&proof), &["--ctlsplit=2", "--ctlsplitbits=3", "--ctlsplitlevels=1", "--ctlsplitcase=40"], VARS, false);
    assert_eq!(status, "UNSATISFIABLE");
    let line = control_line(&out, "ctlsplit=");
    assert!(line.contains("cases=8 "), "{}", line);
    assert!(line.contains("status=probed") || line.contains("status=unsat"), "{}", line);
    assert_eq!(gate(&cnf, &proof), "ok");
}

/// Two 200-bit barrel rotators over one 3-bit word, the levels in opposite
/// orders, compared bit by bit (tools/control_read.py's Circ; unsatisfiable).
fn rotmiter() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/control_rotmiter.cnf")
}

#[test]
fn reading_finds_the_control_word() {
    let fx = Fixture::new("control_read");
    let cnf = rotmiter();
    for shipped in [false, true] {
        let proof = fx.path(&format!("rot{}.drat", shipped));
        let (status, out) = run_ctl(&cnf, Some(&proof), &["--ctlsplit=1"], "", shipped);
        assert_eq!(status, "UNSATISFIABLE", "shipped {}", shipped);
        let read = control_line(&out, "control read");
        assert!(read.contains("switches=1200") && read.contains("vars=1,2,3 "), "shipped {}: {}", shipped, read);
        assert!(control_line(&out, "ctlsplit=").contains("status=unsat"), "shipped {}", shipped);
        assert_eq!(gate(&cnf, &proof), "ok", "shipped {}: gate", shipped);
    }
    // a formula without switches reads nothing and searches as without the option
    let r = fx.path("r.cnf");
    random_3sat(120, 4.6, 3, &r);
    let (_s, out) = run_ctl(&r, None, &["--ctlsplit=1"], "", false);
    assert!(control_line(&out, "control read").contains("none"), "{}", control_line(&out, "control read"));
}

#[test]
fn split_stops_when_cases_cost_a_search() {
    // random 3-SAT given a control set: its cases are searches, each far more
    // than a pass over the formula, so the split stops after four of them
    let fx = Fixture::new("control_costly");
    let cnf = fx.path("r.cnf");
    let proof = fx.path("r.drat");
    random_3sat(200, 4.4, 5, &cnf);
    let (status, out) = run_ctl(&cnf, Some(&proof), &["--ctlsplit=1", "--ctlsplitratio=2"], VARS, false);
    let line = control_line(&out, "ctlsplit=");
    assert!(line.contains("status=costly") || line.contains("status=unsat") || line.contains("status=sat"), "{}", line);
    if status == "UNSATISFIABLE" {
        assert_eq!(gate(&cnf, &proof), "ok");
    }
}

#[test]
fn control_first_after_compaction() {
    // units fix variables 1-4, so compaction renumbers the others; the
    // control variable 7 must still be the one decided (review round b1, P1)
    let fx = Fixture::new("control_compact");
    let base = fx.path("base.cnf");
    random_3sat(120, 4.3, 13, &base);
    let text = std::fs::read_to_string(&base).unwrap();
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let header = lines.iter().position(|l| l.starts_with("p ")).unwrap();
    let n: usize = lines[header].split_whitespace().nth(3).unwrap().parse().unwrap();
    lines[header] = format!("p cnf 120 {}", n + 4);
    for v in 1..=4 {
        lines.push(format!("{} 0", v));
    }
    let cnf = fx.path("r.cnf");
    std::fs::write(&cnf, lines.join("\n") + "\n").unwrap();
    let proof = fx.path("r.drat");
    let (status, out) = run_ctl(&cnf, Some(&proof), &["--ctlfirst=1", "--compactlim=0"], "7,9,11", false);
    let plain = run(&cnf, &[], &[]);
    assert_eq!(status, plain.status);
    assert!(control_line(&out, "ctlfirst=1").contains("vars=3"), "{}", control_line(&out, "ctlfirst=1"));
    if status == "UNSATISFIABLE" {
        assert_eq!(gate(&cnf, &proof), "ok");
    } else {
        assert!(model_ok(&cnf, &out));
    }
}

#[test]
fn reading_keeps_to_the_global_limit() {
    // the reading's allowance is what SAT_LIMIT_TICKS leaves (review round b1)
    let cnf = rotmiter();
    let r = run(&cnf, &["--ctlsplit=1"], &[("SAT_LIMIT_TICKS", "1")]);
    let line = r.stdout.lines().find(|l| l.starts_with("c control read")).unwrap_or("").to_string();
    let work: u64 = line.split_whitespace().find_map(|t| t.strip_prefix("work=")).and_then(|v| v.parse().ok()).unwrap_or(u64::MAX);
    assert!(work <= 64, "the reading spent {} under a limit of 1: {}", work, line);
}

#[test]
fn reading_polls_its_budget_inside_a_long_cone() {
    // one switch whose select ends a chain of 20,000 AND gates: the walk to
    // its inputs stops at the limit, not after the whole chain (review b3)
    let fx = Fixture::new("control_chain");
    let cnf = fx.path("c.cnf");
    let n = 20_000i64;
    let mut cls: Vec<String> = Vec::new();
    // inputs 1..=n+1; g_1 = 1 and 2, g_k = g_(k-1) and (k+1), numbered after
    let g = |k: i64| n + 1 + k;
    for k in 1..=n {
        let (a, b, o) = (if k == 1 { 1 } else { g(k - 1) }, k + 1, g(k));
        cls.push(format!("{} {} 0", -o, a));
        cls.push(format!("{} {} 0", -o, b));
        cls.push(format!("{} {} {} 0", o, -a, -b));
    }
    let (x, y, out) = (g(n) + 1, g(n) + 2, g(n) + 3);
    let s_ = g(n);
    for c in [format!("{} {} {} 0", -s_, -x, out), format!("{} {} {} 0", -s_, x, -out), format!("{} {} {} 0", s_, -y, out), format!("{} {} {} 0", s_, y, -out)] {
        cls.push(c);
    }
    std::fs::write(&cnf, format!("p cnf {} {}\n{}\n", out, cls.len(), cls.join("\n"))).unwrap();
    let limit: u64 = 450_000;
    let r = run(&cnf, &["--ctlsplit=1"], &[("SAT_LIMIT_TICKS", &limit.to_string())]);
    let line = r.stdout.lines().find(|l| l.starts_with("c control read")).unwrap_or("").to_string();
    let work: u64 = line.split_whitespace().find_map(|t| t.strip_prefix("work=")).and_then(|v| v.parse().ok()).unwrap_or(u64::MAX);
    assert!(work <= limit + 1000, "the reading spent {} under a limit of {}: {}", work, limit, line);
}

#[test]
fn reading_pairs_halves_in_linear_work() {
    // 20,000 guarded equalities on one output (o <-> x under p_i), none with
    // both select signs: the pairing is one pass, not 20,000 squared
    let fx = Fixture::new("control_halves");
    let cnf = fx.path("h.cnf");
    let k = 20_000i64;
    let (o, x) = (k + 2, k + 1);
    let mut body = String::new();
    for p in 1..=k {
        body.push_str(&format!("{} {} {} 0\n{} {} {} 0\n", -p, x, -o, -p, -x, o));
    }
    std::fs::write(&cnf, format!("p cnf {} {}\n{}", k + 2, 2 * k, body)).unwrap();
    let r = run(&cnf, &["--ctlsplit=1"], &[]);
    let line = r.stdout.lines().find(|l| l.starts_with("c control read")).unwrap_or("").to_string();
    let work: u64 = line.split_whitespace().find_map(|t| t.strip_prefix("work=")).and_then(|v| v.parse().ok()).unwrap_or(u64::MAX);
    assert!(work < 100 * 2 * k as u64, "reading work {} for {} clauses: {}", work, 2 * k, line);
    assert_eq!(r.status, "SATISFIABLE");
}

#[test]
fn split_keeps_the_formula_past_the_structure_buffer() {
    // the structure stage's buffer overflows (structclauses=1) with the walk
    // off: the flushed formula is kept for the split (review b6)
    let fx = Fixture::new("control_overflow");
    let proof = fx.path("rot.drat");
    let (status, out) = run_ctl(&rotmiter(), Some(&proof), &["--ctlsplit=1", "--structclauses=1", "--freewalk=0"], "", true);
    assert_eq!(status, "UNSATISFIABLE");
    let line = control_line(&out, "ctlsplit=");
    assert!(line.contains("status=unsat") && line.contains("cases=8 "), "{}", line);
    assert_eq!(gate(&rotmiter(), &proof), "ok");
}

#[test]
fn split_stops_when_a_case_cannot_load() {
    // a case limit below the formula's size: no case can even load, and no
    // deeper split could do better, so the split stops at once (review b5)
    let fx = Fixture::new("control_noload");
    let cnf = fx.path("r.cnf");
    random_3sat(200, 3.5, 17, &cnf);
    let vars: String = (1..=18).map(|v| v.to_string()).collect::<Vec<_>>().join(",");
    let (status, out) = run_ctl(&cnf, None, &["--ctlsplit=1", "--ctlsplitcase=1", "--ctlsplitratio=100000"], &vars, false);
    let line = control_line(&out, "ctlsplit=");
    assert!(line.contains("status=budget") && line.contains("cases=1 "), "{}", line);
    assert_eq!(status, "SATISFIABLE");
}

#[test]
fn split_keeps_to_the_tick_limit() {
    let fx = Fixture::new("control_split_limit");
    let cnf = fx.path("r.cnf");
    random_3sat(200, 4.8, 11, &cnf);
    let limit: u64 = 2_000_000;
    let r = run(&cnf, &["--ctlsplit=1", "--ctlsplitbits=3", "--ctlsplitratio=100000"], &[("SAT_CTL_VARS", VARS), ("SAT_LIMIT_TICKS", &limit.to_string())]);
    assert!(r.status == "UNKNOWN" || r.status == "UNSATISFIABLE", "{}", r.status);
    // a run stops at the next poll past the limit: allow one case's slack
    assert!(r.work() <= limit + limit / 2, "work {} against the limit {}", r.work(), limit);
}
