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

// ---------------------------------------------------------------------------
// The split in two stages (`--ctlsplitassume=1`, epic SAT-playground-bp3
// step 5): a case is first a question to one case solver, under assumptions;
// only a case it leaves open gets a sub-solver of its own.
// ---------------------------------------------------------------------------

fn stage1_value(line: &str, key: &str) -> u64 {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(&format!("{}=", key)).map(|v| v.parse::<u64>().unwrap()))
        .unwrap_or_else(|| panic!("no {}= on: {}", key, line))
}

#[test]
fn two_stage_proofs_pass_the_gate() {
    let fx = Fixture::new("control_two_stage");
    let cnf = fx.path("r.cnf");
    random_3sat(160, 4.8, 7, &cnf);
    // the three regimes of split_proofs_pass_the_gate (decided at the first
    // level, at a second level with joins, left open), each with a stage 1
    // that answers nearly everything (a limit of 1000 loads), the default
    // (one load) and one that answers nothing (a hundredth of a load)
    for (case, want) in [("100000", "status=unsat"), ("300", "status=unsat"), ("40", "status=open")] {
        for load in ["100000", "100", "1"] {
            let proof = fx.path(&format!("p{}-{}.drat", case, load));
            let (status, out) = run_ctl(
                &cnf,
                Some(&proof),
                &[
                    "--ctlsplit=1", "--ctlsplitassume=1", "--ctlsplitbits=3", "--ctlsplitratio=100000",
                    &format!("--ctlsplitcase={}", case), &format!("--ctlsplitassumeload={}", load),
                ],
                VARS,
                false,
            );
            let what = format!("case limit {}, stage-1 load {}%", case, load);
            assert_eq!(status, "UNSATISFIABLE", "{}", what);
            let line = control_line(&out, "ctlsplit=");
            assert!(line.contains(want), "{}: {}", what, line);
            assert!(stage1_value(line, "stage1") > 0, "{}: {}", what, line);
            assert_eq!(gate(&cnf, &proof), "ok", "{}: gate ({})", what, line);
        }
    }
}

#[test]
fn two_stage_answers_from_the_case_solver() {
    // with room for stage 1, the cases are answered by the case solver: one
    // load of the formula for all of them, none for a sub-solver
    let fx = Fixture::new("control_two_stage_all");
    let cnf = fx.path("r.cnf");
    let proof = fx.path("r.drat");
    random_3sat(160, 4.8, 7, &cnf);
    let options = ["--ctlsplit=1", "--ctlsplitassume=1", "--ctlsplitbits=3", "--ctlsplitratio=100000", "--ctlsplitassumeload=100000"];
    let (status, out) = run_ctl(&cnf, Some(&proof), &options, VARS, false);
    assert_eq!(status, "UNSATISFIABLE");
    let line = control_line(&out, "ctlsplit=");
    // every case is asked of the case solver and none is left open.  The
    // case solver keeps what it learned, so before the last case it can
    // find that the formula itself is unsatisfiable: then that case is not
    // counted as refuted, and the answer is the same
    assert!(line.contains("status=unsat") && line.contains("unknown=0 "), "{}", line);
    assert_eq!(stage1_value(line, "s1loads"), 1, "{}", line);
    assert_eq!(stage1_value(line, "s1open"), 0, "{}", line);
    let cases = stage1_value(line, "cases");
    let answered = stage1_value(line, "s1refuted") + stage1_value(line, "s1skipped");
    assert!(cases >= 7 && (answered == cases || answered + 1 == cases), "{}", line);
    assert_eq!(gate(&cnf, &proof), "ok", "{}", line);
    // the one-stage split on the same formula pays eight loads
    let (_s, one) = run_ctl(&cnf, None, &["--ctlsplit=1", "--ctlsplitbits=3", "--ctlsplitratio=100000"], VARS, false);
    let one_line = control_line(&one, "ctlsplit=");
    assert!(one_line.contains("status=unsat") && !one_line.contains("stage1="), "the one-stage line is what it was: {}", one_line);
}

#[test]
fn two_stage_model_satisfies_the_input() {
    let fx = Fixture::new("control_two_stage_sat");
    for seed in 1..=3u64 {
        let cnf = fx.path(&format!("s{}.cnf", seed));
        random_3sat(150, 3.6, seed, &cnf);
        for load in ["100000", "1"] {
            let (status, out) = run_ctl(
                &cnf,
                None,
                &["--ctlsplit=1", "--ctlsplitassume=1", "--ctlsplitbits=3", "--ctlsplitratio=100000", &format!("--ctlsplitassumeload={}", load)],
                VARS,
                false,
            );
            assert_eq!(status, "SATISFIABLE", "seed {} load {}", seed, load);
            assert!(control_line(&out, "ctlsplit=").contains("status=sat"), "seed {} load {}: the split's model", seed, load);
            assert!(model_ok(&cnf, &out), "seed {} load {}: the model satisfies every clause", seed, load);
        }
    }
}

#[test]
fn two_stage_a_small_core_skips_cases() {
    // variable 1 cannot be true: (not 1 or 200) and (not 1 or not 200).  So
    // the first case with bit 1 on fails with the core "1" alone, and the
    // other three such cases are refuted without being asked
    let fx = Fixture::new("control_two_stage_core");
    let cnf = fx.path("r.cnf");
    let proof = fx.path("r.drat");
    random_3sat(160, 4.8, 7, &cnf);
    let text = std::fs::read_to_string(&cnf).unwrap();
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let header: Vec<&str> = lines[0].split_whitespace().collect();
    let clauses: usize = header[3].parse().unwrap();
    lines[0] = format!("p cnf 200 {}", clauses + 2);
    lines.push("-1 200 0".to_string());
    lines.push("-1 -200 0".to_string());
    std::fs::write(&cnf, lines.join("\n") + "\n").unwrap();
    let (status, out) = run_ctl(
        &cnf,
        Some(&proof),
        &["--ctlsplit=1", "--ctlsplitassume=1", "--ctlsplitbits=3", "--ctlsplitratio=100000", "--ctlsplitassumeload=100000"],
        "1,2,3",
        false,
    );
    assert_eq!(status, "UNSATISFIABLE");
    let line = control_line(&out, "ctlsplit=");
    // (the case solver may refute the whole formula before the last case,
    // so not all three need to be reached)
    assert!(line.contains("status=unsat") && line.contains("unknown=0 "), "{}", line);
    let skipped = stage1_value(line, "s1skipped");
    assert!((1..=3).contains(&skipped), "{}", line);
    assert_eq!(stage1_value(line, "stage1") + skipped, stage1_value(line, "cases"), "a skipped case is not asked: {}", line);
    assert_eq!(gate(&cnf, &proof), "ok", "{}", line);
}

#[test]
fn two_stage_leaves_the_search_alone() {
    // random 3-SAT given a control set: its first case is a search, so the
    // split gives up at once.  The case solver is a solver of its own, so
    // the search afterwards is the plain one, counter for counter.
    let fx = Fixture::new("control_two_stage_costly");
    let cnf = fx.path("r.cnf");
    random_3sat(200, 4.4, 5, &cnf);
    let plain = run(&cnf, &[], &[]);
    let split = run(&cnf, &["--ctlsplit=1", "--ctlsplitassume=1", "--ctlsplitratio=1"], &[("SAT_CTL_VARS", VARS)]);
    let line = control_line(&split.stdout, "ctlsplit=");
    assert!(line.contains("status=costly") && line.contains("refuted=0 "), "the split must give up with nothing refuted: {}", line);
    assert!(stage1_value(line, "stage1") >= 1, "{}", line);
    assert_eq!(plain.status, split.status);
    for key in ["conflicts", "decisions", "propagations", "restarts", "reductions"] {
        assert_eq!(plain.stats.get(key), split.stats.get(key), "the search after a split that gave up: {}", key);
    }
    assert!(split.work() > plain.work(), "the split's work counts");
}

#[test]
fn two_stage_keeps_to_its_limits() {
    // Review round 1: loading the case solver is work that comes out of the
    // limit, and a case that waited for its sub-solver has only what its
    // try in the case solver left of its limit.
    let fx = Fixture::new("control_two_stage_limit");
    let cnf = fx.path("r.cnf");
    random_3sat(200, 4.8, 11, &cnf);
    let load: u64 = 4 * 960; // 960 clauses of three literals and a zero
    // the run's work limit, from far above the load down to just above it
    for limit in [2_000_000u64, 100_000, 3 * load, load + load / 20] {
        let r = run(
            &cnf,
            &["--ctlsplit=1", "--ctlsplitassume=1", "--ctlsplitbits=3", "--ctlsplitratio=100000"],
            &[("SAT_CTL_VARS", VARS), ("SAT_LIMIT_TICKS", &limit.to_string())],
        );
        assert!(r.status == "UNKNOWN" || r.status == "UNSATISFIABLE", "{}", r.status);
        // a run stops at the next poll past the limit: the same slack as
        // the one-stage split has
        assert!(r.work() <= limit + limit / 2, "work {} against the limit {}", r.work(), limit);
    }
    // the case limit: every case of the probe gets both stages, and in all
    // they stay within one case limit a case (plus the loads)
    let case_k: u64 = 60;
    let r = run(
        &cnf,
        &[
            "--ctlsplit=2", "--ctlsplitassume=1", "--ctlsplitbits=3", "--ctlsplitlevels=1", "--ctlsplitassumeload=1000",
            &format!("--ctlsplitcase={}", case_k),
        ],
        &[("SAT_CTL_VARS", VARS)],
    );
    let line = control_line(&r.stdout, "ctlsplit=");
    assert!(line.contains("cases=8 "), "{}", line);
    assert!(stage1_value(line, "s1open") > 0, "the test needs cases that go on to a sub-solver: {}", line);
    let work = stage1_value(line, "work");
    assert!(work <= 8 * case_k * 1000 * 11 / 10, "split work {} against eight case limits of {}K: {}", work, case_k, line);
}

#[test]
fn two_stage_a_waiting_case_pays_for_the_load_it_caused() {
    // Review round 3.  Stage 1 answers nothing here (a hundredth of a load),
    // so every case goes on to a sub-solver.  The first two go at once, and
    // each time the case solver is released; the third case therefore loads
    // it again, and then waits.  That load came out of its limit, and must
    // not be given back when its sub-solver starts.
    let fx = Fixture::new("control_two_stage_reload");
    let cnf = fx.path("r.cnf");
    random_3sat(200, 4.8, 11, &cnf);
    let load: u64 = 4 * 960;
    let case_limit: u64 = 10_000;
    let r = run(
        &cnf,
        &[
            "--verbose=2", "--ctlsplit=2", "--ctlsplitassume=1", "--ctlsplitbits=3", "--ctlsplitlevels=1",
            "--ctlsplitassumeload=1", "--ctlsplitcase=10",
        ],
        &[("SAT_CTL_VARS", VARS)],
    );
    let line = control_line(&r.stdout, "ctlsplit=");
    assert_eq!(stage1_value(line, "s1loads"), 3, "{}", line);
    assert_eq!(stage1_value(line, "s1open"), 8, "{}", line);
    // "c control split: case <order> gets <limit> work in a sub-solver"
    let granted: std::collections::HashMap<u64, u64> = r
        .stdout
        .lines()
        .filter(|l| l.contains("control split: case ") && l.contains(" work in a sub-solver"))
        .map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            let at = w.iter().position(|x| *x == "case").unwrap();
            (w[at + 1].parse().unwrap(), w[at + 3].parse().unwrap())
        })
        .collect();
    assert_eq!(granted.len(), 8, "{:?}", granted);
    for order in 0..3u64 {
        // these three each paid a load of the case solver
        assert!(granted[&order] <= case_limit - load, "case {} got {} after paying a load of {}", order, granted[&order], load);
    }
    for order in 3..8u64 {
        assert!(granted[&order] <= case_limit && granted[&order] > case_limit - load, "case {}: {}", order, granted[&order]);
    }
}

#[test]
fn two_stage_counts_the_propagation_of_the_load() {
    // Review round 2.  One unit clause at the head of a chain of 20,000
    // implications: adding the formula to the case solver propagates the
    // whole chain.  That work must show on the clock as it does for a
    // sub-solver of the one-stage split.
    let fx = Fixture::new("control_two_stage_chain");
    let cnf = fx.path("chain.cnf");
    let n = 20_000;
    let mut text = format!("p cnf {} {}\n", n + 30, n + 40);
    text.push_str("1 0\n");
    for v in 1..n {
        text.push_str(&format!("-{} {} 0\n", v, v + 1));
    }
    // a small satisfiable part on the control variables, so the split has cases
    for k in 0..40 {
        let a = n + 1 + (k % 10);
        let b = n + 11 + (k * 7 % 20);
        text.push_str(&format!("{} {} 0\n", a, if k % 3 == 0 { -(b as i64) } else { b as i64 }));
    }
    std::fs::write(&cnf, text).unwrap();
    let vars = format!("{},{},{}", n + 1, n + 2, n + 3);
    let options = ["--ctlsplit=2", "--ctlsplitbits=3", "--ctlsplitlevels=1", "--ctlsplitratio=100000"];
    let one = run(&cnf, &options, &[("SAT_CTL_VARS", &vars)]);
    let mut two_options = options.to_vec();
    two_options.push("--ctlsplitassume=1");
    let two = run(&cnf, &two_options, &[("SAT_CTL_VARS", &vars)]);
    let (one_line, two_line) = (control_line(&one.stdout, "ctlsplit="), control_line(&two.stdout, "ctlsplit="));
    let literals = (2 * (n as u64 - 1) + 1 + 80) + (n as u64 + 40); // literals and zeros
    // one stage: every case loads and propagates the chain; two stages: once
    let chain = n as u64; // at least a tick a propagated literal
    assert!(stage1_value(one_line, "work") >= 8 * literals, "{}", one_line);
    assert!(
        stage1_value(two_line, "work") >= literals + chain,
        "the chain's propagation is missing from the split's work: {}",
        two_line
    );
    assert!(stage1_value(two_line, "work") < stage1_value(one_line, "work"), "{} v {}", two_line, one_line);
}

#[test]
fn two_stage_on_the_rotator() {
    let fx = Fixture::new("control_two_stage_rot");
    let cnf = rotmiter();
    let proof = fx.path("rot.drat");
    let (status, out) = run_ctl(&cnf, Some(&proof), &["--ctlsplit=1", "--ctlsplitassume=1"], "", false);
    assert_eq!(status, "UNSATISFIABLE");
    let line = control_line(&out, "ctlsplit=");
    assert!(line.contains("status=unsat") && line.contains("cases=8 "), "{}", line);
    assert_eq!(gate(&cnf, &proof), "ok", "{}", line);
    // and with the shipped defaults (the structure stage runs first)
    let proof2 = fx.path("rot2.drat");
    let (status, out) = run_ctl(&cnf, Some(&proof2), &["--ctlsplit=1", "--ctlsplitassume=1"], "", true);
    assert_eq!(status, "UNSATISFIABLE");
    assert!(control_line(&out, "ctlsplit=").contains("status=unsat"));
    assert_eq!(gate(&cnf, &proof2), "ok");
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
fn split_skips_a_formula_past_its_clause_limit() {
    // more input clauses than ctlsplitclauses: the formula is neither kept nor
    // read, and the search is the one without the option, step for step, on
    // each path the formula takes to the split (on 2025's 9.normalised, 158M
    // clauses, the copy and the reading passed 16 GB and the run aborted,
    // where the shipped solver solves it in 95-149 s)
    let cnf = rotmiter(); // 5,601 clauses
    let paths: [(&str, &[&str], bool); 3] = [
        ("structure stage off", &[], false),
        ("structure stage on", &[], true),
        ("structure buffer overflow", &["--structclauses=1", "--freewalk=0"], true),
    ];
    for (what, extra, shipped) in paths {
        let mut off: Vec<&str> = extra.to_vec();
        off.push("--ctlsplit=0");
        let mut on: Vec<&str> = extra.to_vec();
        on.extend(["--ctlsplit=1", "--ctlsplitclauses=5000"]);
        let (a, b) = if shipped {
            (run_shipped(&cnf, &off, &[]), run_shipped(&cnf, &on, &[]))
        } else {
            (run(&cnf, &off, &[]), run(&cnf, &on, &[]))
        };
        common::assert_same_trajectory(&a, &b, what);
        let line = control_line(&b.stdout, "ctlsplit=");
        assert!(line.contains("status=too-big") && line.contains(" read=0"), "{}: {}", what, line);
        assert!(!b.stdout.contains("c control read"), "{}: the reading ran", what);
    }
    // at the limit itself the formula is kept, and the split decides it
    let (status, out) = run_ctl(&cnf, None, &["--ctlsplit=1", "--ctlsplitclauses=5601"], "", false);
    assert_eq!(status, "UNSATISFIABLE");
    assert!(control_line(&out, "ctlsplit=").contains("status=unsat"), "{}", control_line(&out, "ctlsplit="));
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
