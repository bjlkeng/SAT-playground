//! Solving under assumptions (epic SAT-playground-bp3, src/assume.rs,
//! plan/assumptions-plan.md): the binary's test mode `--inccnf=1` reads an
//! incremental CNF file and answers every `a` line with the same solver.
//!
//! Checked here, on the binary: every answer is right (against a truth
//! table), every model satisfies the formula and the assumptions, every
//! failed core is a subset of the question and unsatisfiable with the
//! formula, every proof passes the gate with the core as units and a wrong
//! core is rejected through the same gate; a question that stops at its
//! limit leaves the solver usable; the run's own limits hold across
//! questions; a `--time` stop holds for the rest of the run; a signal ends
//! the run with its statistics; a compaction in the middle of a solve under
//! assumptions keeps the answers right; and what the mode refuses.
//!
//! The larger randomized check, with independent solvers as oracles, is
//! tools/assume_fuzz.py.

mod common;

use common::{solver_bin, Fixture};
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

struct Answer {
    status: String,
    model: Vec<i64>,
    core: Option<Vec<i64>>,
    proof_bytes: Option<u64>,
}

fn write_icnf(path: &Path, clauses: &[Vec<i64>], questions: &[Vec<i64>]) {
    let mut f = std::fs::File::create(path).expect("create icnf");
    writeln!(f, "p inccnf").unwrap();
    for c in clauses {
        for l in c {
            write!(f, "{} ", l).unwrap();
        }
        writeln!(f, "0").unwrap();
    }
    for q in questions {
        write!(f, "a ").unwrap();
        for l in q {
            write!(f, "{} ", l).unwrap();
        }
        writeln!(f, "0").unwrap();
    }
}

fn write_cnf(path: &Path, vars: usize, clauses: &[Vec<i64>]) {
    let mut f = std::fs::File::create(path).expect("create cnf");
    writeln!(f, "p cnf {} {}", vars, clauses.len()).unwrap();
    for c in clauses {
        for l in c {
            write!(f, "{} ", l).unwrap();
        }
        writeln!(f, "0").unwrap();
    }
}

fn command(options: &[&str], env: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.arg("--inccnf=1");
    cmd.args(options);
    cmd
}

/// (exit code, stdout) of the test mode on a file.
fn run_icnf(icnf: &Path, proof: Option<&Path>, options: &[&str], env: &[(&str, &str)]) -> (i32, String) {
    let mut cmd = command(options, env);
    cmd.arg(icnf);
    if let Some(p) = proof {
        cmd.arg(p);
    }
    let out = cmd.output().expect("run sat-solver");
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
}

fn answers(stdout: &str) -> Vec<Answer> {
    let mut out: Vec<Answer> = Vec::new();
    for line in stdout.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("q") => {
                let n: usize = words.next().unwrap().parse().unwrap();
                assert_eq!(n, out.len() + 1, "answers are numbered in order");
                out.push(Answer { status: words.next().unwrap().to_string(), model: Vec::new(), core: None, proof_bytes: None });
            }
            Some("v") if !out.is_empty() => {
                let a = out.last_mut().unwrap();
                a.model.extend(words.map(|w| w.parse::<i64>().unwrap()).filter(|&l| l != 0));
            }
            Some("f") if !out.is_empty() => {
                let lits: Vec<i64> = words.map(|w| w.parse::<i64>().unwrap()).collect();
                assert_eq!(lits.last(), Some(&0), "a core line ends with 0");
                out.last_mut().unwrap().core = Some(lits[..lits.len() - 1].to_vec());
            }
            Some("p") if !out.is_empty() => {
                out.last_mut().unwrap().proof_bytes = Some(words.next().unwrap().parse().unwrap());
            }
            _ => {}
        }
    }
    out
}

/// tools/proof_gate.py with the core as units and the proof cut at `bytes`.
fn gate_units(cnf: &Path, proof: &Path, core: &[i64], bytes: u64) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let units = core.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(" ");
    let out = Command::new("python3")
        .arg(root.join("tools/proof_gate.py"))
        .arg(cnf)
        .arg(proof)
        .arg(format!("--units={}", units))
        .arg(format!("--proof-bytes={}", bytes))
        .output()
        .expect("run proof_gate.py");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Truth table: is the formula satisfiable with these literals true?
fn satisfiable(vars: usize, clauses: &[Vec<i64>], lits: &[i64]) -> bool {
    assert!(vars <= 22);
    let holds = |m: u32, l: i64| ((m >> (l.unsigned_abs() - 1)) & 1 == 1) == (l > 0);
    (0..(1u32 << vars)).any(|m| lits.iter().all(|&l| holds(m, l)) && clauses.iter().all(|c| c.iter().any(|&l| holds(m, l))))
}

/// Check every answer of a run against the truth table; with a proof, every
/// UNSAT answer through the gate.  Returns how many answers were SAT and
/// UNSAT.
fn check_all(
    vars: usize,
    clauses: &[Vec<i64>],
    questions: &[Vec<i64>],
    got: &[Answer],
    proof: Option<(&Path, &Path)>,
    limited: bool,
) -> (usize, usize) {
    assert_eq!(got.len(), questions.len(), "one answer per question");
    let (mut sat, mut unsat) = (0, 0);
    for (i, (q, a)) in questions.iter().zip(got).enumerate() {
        let what = format!("question {} {:?}", i + 1, q);
        if a.status == "UNKNOWN" {
            assert!(limited, "{}: UNKNOWN without a limit", what);
            continue;
        }
        let truth = satisfiable(vars, clauses, q);
        if a.status == "SATISFIABLE" {
            assert!(truth, "{}: answered SATISFIABLE, it is not", what);
            let value = |l: i64| a.model.contains(&l);
            assert!(clauses.iter().all(|c| c.iter().any(|&l| value(l))), "{}: the model breaks a clause", what);
            assert!(q.iter().all(|&l| value(l)), "{}: the model breaks an assumption", what);
            assert!(a.model.iter().all(|&l| !value(-l)), "{}: the model gives a variable both values", what);
            sat += 1;
        } else {
            assert_eq!(a.status, "UNSATISFIABLE", "{}", what);
            assert!(!truth, "{}: answered UNSATISFIABLE, it is not", what);
            let core = a.core.as_ref().unwrap_or_else(|| panic!("{}: no core line", what));
            let mut left = q.clone();
            for l in core {
                let at = left.iter().position(|x| x == l).unwrap_or_else(|| panic!("{}: core literal {} is not assumed", what, l));
                left.remove(at);
            }
            assert!(!satisfiable(vars, clauses, core), "{}: the core {:?} is satisfiable with the formula", what, core);
            if let Some((cnf, proof)) = proof {
                let bytes = a.proof_bytes.unwrap_or_else(|| panic!("{}: no p line", what));
                assert_eq!(gate_units(cnf, proof, core, bytes), "ok", "{}: the proof under the core {:?}", what, core);
            }
            unsat += 1;
        }
    }
    (sat, unsat)
}

/// A private generator, so a formula depends only on its seed.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as usize
    }
    fn lit(&mut self, vars: usize) -> i64 {
        let v = (self.next() % vars + 1) as i64;
        if self.next() % 2 == 0 {
            v
        } else {
            -v
        }
    }
}

fn random_3sat(rng: &mut Lcg, vars: usize, clauses_n: usize) -> Vec<Vec<i64>> {
    let mut clauses = Vec::with_capacity(clauses_n);
    while clauses.len() < clauses_n {
        let c = vec![rng.lit(vars), rng.lit(vars), rng.lit(vars)];
        if c[0].abs() != c[1].abs() && c[0].abs() != c[2].abs() && c[1].abs() != c[2].abs() {
            clauses.push(c);
        }
    }
    clauses
}

/// The plan's worked example: a=1 b=2 c=3 d=4 x=5 y=6.
fn example() -> Vec<Vec<i64>> {
    vec![vec![-1, 5], vec![-2, -5, 6], vec![-3, -6]]
}

#[test]
fn the_worked_example_and_its_proof() {
    let fx = Fixture::new("assume-example");
    let clauses = example();
    let questions = vec![vec![1, 2, 4, 3], vec![1, 2], vec![], vec![4, -4], vec![7], vec![1, 2, 4, 3]];
    let (icnf, cnf, proof) = (fx.path("toy.icnf"), fx.path("toy.cnf"), fx.path("toy.proof"));
    write_icnf(&icnf, &clauses, &questions);
    write_cnf(&cnf, 7, &clauses);
    let (code, stdout) = run_icnf(&icnf, Some(&proof), &["-q"], &[]);
    assert_eq!(code, 0, "{}", stdout);
    assert!(!stdout.lines().any(|l| l.starts_with("s ")), "no competition status line in this mode");
    let got = answers(&stdout);
    let (sat, unsat) = check_all(7, &clauses, &questions, &got, Some((&cnf, &proof)), false);
    assert_eq!((sat, unsat), (3, 3));
    assert_eq!(got[0].core, Some(vec![1, 2, 3]), "d is not in the core");
    assert_eq!(got[3].core, Some(vec![4, -4]), "a literal and its negation");
    assert_eq!(got[5].core, Some(vec![1, 2, 3]), "asked again, the same core");
    assert!(got[4].model.contains(&7), "a variable the formula never used takes the assumed value");
    // a wrong core is rejected through the same gate
    let bytes = got[0].proof_bytes.unwrap();
    assert_eq!(gate_units(&cnf, &proof, &[1, 2, 3], bytes), "ok");
    assert!(gate_units(&cnf, &proof, &[1, 3], bytes).starts_with("FAIL"), "the wrong core a, c");
    assert!(gate_units(&cnf, &proof, &[2, 3], bytes).starts_with("FAIL"), "the wrong core b, c");
    assert!(gate_units(&cnf, &proof, &[], bytes).starts_with("FAIL"), "the formula alone is satisfiable");
}

#[test]
fn every_answer_right_on_random_formulas() {
    let fx = Fixture::new("assume-random");
    let mut rng = Lcg(20261115);
    let (mut sat, mut unsat) = (0, 0);
    for round in 0..40 {
        let vars = 8 + rng.next() % 9;
        let count = vars * 4 + rng.next() % vars;
        let clauses = random_3sat(&mut rng, vars, count);
        let mut questions: Vec<Vec<i64>> = Vec::new();
        for _ in 0..8 {
            let k = rng.next() % 5;
            let mut q: Vec<i64> = (0..k).map(|_| rng.lit(vars + 1)).collect();
            if !q.is_empty() && rng.next() % 5 == 0 {
                q.push(q[0]); // the same literal twice
            }
            if !q.is_empty() && rng.next() % 7 == 0 {
                q.push(-q[0]); // a literal with its negation
            }
            questions.push(q);
        }
        let (icnf, cnf, proof) = (fx.path("r.icnf"), fx.path("r.cnf"), fx.path("r.proof"));
        write_icnf(&icnf, &clauses, &questions);
        write_cnf(&cnf, vars + 1, &clauses);
        // every other round with the intervals made tiny, so reduction,
        // probing, elimination (in the plain questions) and mode switches
        // happen inside these small solves
        let often: &[&str] = &[
            "-q", "--eliminateinit=0", "--eliminateint=10", "--probeinit=0", "--probeint=2", "--reduceinit=2",
            "--reduceint=2", "--rephaseinit=10", "--rephaseint=10", "--modeinit=10", "--modeint=10", "--compactlim=0",
        ];
        let options: &[&str] = if round % 2 == 0 { &["-q"] } else { often };
        let (code, stdout) = run_icnf(&icnf, Some(&proof), options, &[]);
        assert_eq!(code, 0, "round {}: {}", round, stdout);
        let (s, u) = check_all(vars + 1, &clauses, &questions, &answers(&stdout), Some((&cnf, &proof)), false);
        sat += s;
        unsat += u;
    }
    assert!(sat > 40 && unsat > 40, "both kinds of answer are exercised: {} SAT, {} UNSAT", sat, unsat);
}

/// Pigeons into holes, every pigeon clause guarded by the switch variable
/// `vars + 1`: satisfiable, and under the switch as hard as the pigeons.
fn guarded_php(holes: usize) -> (usize, Vec<Vec<i64>>) {
    let pigeons = holes + 1;
    let var = |p: usize, h: usize| (p * holes + h + 1) as i64;
    let switch = (pigeons * holes + 1) as i64;
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for p in 0..pigeons {
        let mut c: Vec<i64> = (0..holes).map(|h| var(p, h)).collect();
        c.push(-switch);
        clauses.push(c);
    }
    for h in 0..holes {
        for a in 0..pigeons {
            for b in (a + 1)..pigeons {
                clauses.push(vec![-var(a, h), -var(b, h)]);
            }
        }
    }
    (switch as usize, clauses)
}

#[test]
fn a_question_that_stops_at_its_limit_and_the_next() {
    let fx = Fixture::new("assume-limit");
    let (vars, clauses) = guarded_php(7);
    let s = vars as i64;
    let questions = vec![vec![s], vec![-s], vec![s], vec![]];
    let icnf = fx.path("php.icnf");
    write_icnf(&icnf, &clauses, &questions);
    // five conflicts a question: the pigeons need far more
    let (code, stdout) = run_icnf(&icnf, None, &["-q", "--assumeconflicts=5"], &[]);
    assert_eq!(code, 0, "{}", stdout);
    let got = answers(&stdout);
    let status: Vec<&str> = got.iter().map(|a| a.status.as_str()).collect();
    assert_eq!(status, ["UNKNOWN", "SATISFIABLE", "UNKNOWN", "SATISFIABLE"]);
    assert!(got[1].model.contains(&-s));
    // without the limit the same file is answered in full, by one solver
    let (code, stdout) = run_icnf(&icnf, None, &["-q"], &[]);
    assert_eq!(code, 0, "{}", stdout);
    let got = answers(&stdout);
    let status: Vec<&str> = got.iter().map(|a| a.status.as_str()).collect();
    assert_eq!(status, ["UNSATISFIABLE", "SATISFIABLE", "UNSATISFIABLE", "SATISFIABLE"]);
    assert_eq!(got[0].core, Some(vec![s]));
    assert_eq!(got[2].core, Some(vec![s]));
    // a work limit a question (in thousands) does the same
    let (code, stdout) = run_icnf(&icnf, None, &["-q", "--assumeticks=1"], &[]);
    assert_eq!(code, 0, "{}", stdout);
    let got = answers(&stdout);
    assert_eq!(got.len(), 4);
    assert_eq!(got[0].status, "UNKNOWN");
    assert_eq!(got[2].status, "UNKNOWN");
}

fn exit_value(stdout: &str, line: &str, key: &str) -> u64 {
    let l = stdout.lines().find(|l| l.starts_with(line)).unwrap_or_else(|| panic!("no '{}' line in:\n{}", line, stdout));
    l.split_whitespace()
        .find_map(|w| w.strip_prefix(&format!("{}=", key)).map(|v| v.parse::<u64>().unwrap()))
        .unwrap_or_else(|| panic!("no {}= on: {}", key, l))
}

#[test]
fn the_runs_limits_hold_across_questions() {
    let fx = Fixture::new("assume-run-limit");
    let (vars, clauses) = guarded_php(8);
    let s = vars as i64;
    // the same hard question many times: each needs thousands of conflicts
    let questions: Vec<Vec<i64>> = (0..6).map(|_| vec![s]).collect();
    let icnf = fx.path("php.icnf");
    write_icnf(&icnf, &clauses, &questions);
    // the run's conflict limit: reached inside the first question, and it
    // stays reached; today's single search would forget it after one solve
    let (code, stdout) = run_icnf(&icnf, None, &["--conflicts=300"], &[]);
    assert_eq!(code, 0, "{}", stdout);
    let got = answers(&stdout);
    assert_eq!(got.len(), 6);
    assert!(got.iter().all(|a| a.status == "UNKNOWN"), "{}", stdout);
    let conflicts = exit_value(&stdout, "c workclock", "conflicts");
    assert!((300..=301).contains(&conflicts), "the run stopped at its limit: {} conflicts", conflicts);
    assert_eq!(exit_value(&stdout, "c assume", "solves"), 6);
    assert_eq!(exit_value(&stdout, "c assume", "unknown"), 6);
    // the run's work limit, the same way
    let (code, stdout) = run_icnf(&icnf, None, &[], &[("SAT_LIMIT_TICKS", "200000")]);
    assert_eq!(code, 0, "{}", stdout);
    let got = answers(&stdout);
    assert!(got.iter().all(|a| a.status == "UNKNOWN"), "{}", stdout);
    let work = exit_value(&stdout, "c workclock", "work");
    assert!((200_000..400_000).contains(&work), "the run stopped near its work limit: {}", work);
    // and a limit a question next to a roomy limit of the run: every
    // question gets its own 50 conflicts
    let (code, stdout) = run_icnf(&icnf, None, &["--conflicts=100000", "--assumeconflicts=50"], &[]);
    assert_eq!(code, 0, "{}", stdout);
    assert!(answers(&stdout).iter().all(|a| a.status == "UNKNOWN"));
    let conflicts = exit_value(&stdout, "c workclock", "conflicts");
    assert!((300..=306).contains(&conflicts), "six questions of 50 conflicts each: {}", conflicts);
}

#[test]
fn a_limit_reached_before_the_search_starts() {
    // Review round 1.  A question without assumptions runs the first
    // preprocessing, and a tiny work limit is reached inside it, before the
    // search starts.  The stop flag the limit raised must not reach the next
    // question: it has a limit of its own and must get to use it.
    let fx = Fixture::new("assume-early-limit");
    let mut rng = Lcg(5);
    let clauses = random_3sat(&mut rng, 300, 1260);
    let questions = vec![vec![], vec![], vec![1, -2], vec![]];
    let icnf = fx.path("r.icnf");
    write_icnf(&icnf, &clauses, &questions);
    let (code, stdout) = run_icnf(&icnf, None, &["--lucky=0", "--assumeticks=1"], &[]);
    assert_eq!(code, 0, "{}", stdout);
    let got = answers(&stdout);
    assert_eq!(got.len(), 4);
    let work: Vec<u64> = stdout
        .lines()
        .filter(|l| l.starts_with("c question "))
        .map(|l| l.split_whitespace().nth(4).unwrap().parse().unwrap())
        .collect();
    assert_eq!(work.len(), 4, "{}", stdout);
    for (i, w) in work.iter().enumerate() {
        assert!(*w > 0, "question {} came back with no work done: {:?}\n{}", i + 1, work, stdout);
    }
}

#[test]
fn old_side_files_of_the_proof_path_are_removed() {
    // Review round 1.  The proof path was used before by a run whose
    // structure pass left a .pbp and a .derived.cnf next to it.  The test
    // mode writes neither; left in place they would make the gate check the
    // old structure proof against this run's answer.
    let fx = Fixture::new("assume-sidecars");
    let clauses = vec![vec![-1i64]];
    let questions = vec![vec![1i64]];
    let (icnf, cnf, proof) = (fx.path("u.icnf"), fx.path("u.cnf"), fx.path("u.proof"));
    write_icnf(&icnf, &clauses, &questions);
    write_cnf(&cnf, 1, &clauses);
    let pbp = fx.path("u.proof.pbp");
    let derived = fx.path("u.proof.derived.cnf");
    std::fs::write(&pbp, "pseudo-Boolean proof version 3.0\nf 99 ;\n").unwrap();
    std::fs::write(&derived, "p cnf 1 1\n1 0\n").unwrap();
    let (code, stdout) = run_icnf(&icnf, Some(&proof), &["-q"], &[]);
    assert_eq!(code, 0, "{}", stdout);
    assert!(!pbp.exists() && !derived.exists(), "the earlier run's side files are gone");
    let got = answers(&stdout);
    assert_eq!(got[0].status, "UNSATISFIABLE");
    assert_eq!(got[0].core, Some(vec![1]));
    assert_eq!(gate_units(&cnf, &proof, &[1], got[0].proof_bytes.unwrap()), "ok");
    // and the gate itself refuses a core next to a structure proof: that
    // proof may have cut off the very model the assumptions ask for
    std::fs::write(&pbp, "pseudo-Boolean proof version 3.0\nf 1 ;\n").unwrap();
    std::fs::write(&derived, "p cnf 1 0\n").unwrap();
    assert!(gate_units(&cnf, &proof, &[1], got[0].proof_bytes.unwrap()).starts_with("FAIL:a failed core cannot"));
}

#[test]
fn a_time_limit_stops_the_rest_of_the_run() {
    let fx = Fixture::new("assume-time");
    // eleven holes: far beyond a second of search
    let (vars, clauses) = guarded_php(11);
    let s = vars as i64;
    let questions = vec![vec![-s], vec![s], vec![s], vec![-s]];
    let icnf = fx.path("php.icnf");
    write_icnf(&icnf, &clauses, &questions);
    let start = std::time::Instant::now();
    let (code, stdout) = run_icnf(&icnf, None, &["-q", "--time=1"], &[]);
    assert_eq!(code, 0, "{}", stdout);
    assert!(start.elapsed().as_secs() < 20, "the alarm ended the run");
    let got = answers(&stdout);
    let status: Vec<&str> = got.iter().map(|a| a.status.as_str()).collect();
    // the first is easy; the second is stopped by the alarm; after that the
    // stop holds, even for a question that would be easy
    assert_eq!(status, ["SATISFIABLE", "UNKNOWN", "UNKNOWN", "UNKNOWN"]);
}

#[test]
fn a_signal_ends_the_run_with_its_statistics() {
    let fx = Fixture::new("assume-signal");
    let (vars, clauses) = guarded_php(11);
    let s = vars as i64;
    let questions = vec![vec![-s], vec![s], vec![-s]];
    let icnf = fx.path("php.icnf");
    write_icnf(&icnf, &clauses, &questions);
    let mut cmd = command(&[], &[]);
    cmd.arg(&icnf).stdout(Stdio::piped()).stderr(Stdio::null());
    let child = cmd.spawn().expect("spawn sat-solver");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let killed = Command::new("kill").arg("-TERM").arg(child.id().to_string()).status().expect("kill");
    assert!(killed.success());
    let out = child.wait_with_output().expect("wait");
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(out.status.signal(), Some(15), "the process ends by the signal, as the plain solver does");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("caught signal 15"), "{}", stdout);
    assert_eq!(answers(&stdout).len(), 1, "the first question was answered before the signal");
    assert_eq!(exit_value(&stdout, "c assume", "solves"), 2, "the statistics show the solve in progress");
}

#[test]
fn a_compaction_in_the_middle_of_a_solve() {
    let fx = Fixture::new("assume-compact");
    let mut rng = Lcg(77);
    let mut compactions = 0;
    for _ in 0..12 {
        // 16 variables of random 3-SAT, and variables 17 to 20 fixed by unit
        // clauses: fixed variables are what a compaction squeezes out
        let vars = 16;
        let count = 66 + rng.next() % 6;
        let mut clauses = random_3sat(&mut rng, vars, count);
        for v in 17..=20 {
            clauses.push(vec![if rng.next() % 2 == 0 { v } else { -v }]);
        }
        let mut questions: Vec<Vec<i64>> = Vec::new();
        for _ in 0..8 {
            // a fixed variable first (true: a level with nothing on it;
            // false: a core of one), then free ones
            let fixed = 17 + (rng.next() % 4) as i64;
            let mut q = vec![if rng.next() % 3 == 0 { -fixed } else { fixed }];
            for _ in 0..(1 + rng.next() % 3) {
                q.push(rng.lit(vars));
            }
            questions.push(q);
        }
        let (icnf, cnf, proof) = (fx.path("c.icnf"), fx.path("c.cnf"), fx.path("c.proof"));
        write_icnf(&icnf, &clauses, &questions);
        write_cnf(&cnf, 20, &clauses);
        let (code, stdout) = run_icnf(
            &icnf,
            Some(&proof),
            &["-v", "--reduceinit=2", "--reduceint=2", "--compactlim=0", "--restartint=1", "--chrono=0"],
            &[],
        );
        assert_eq!(code, 0, "{}", stdout);
        compactions += stdout.lines().filter(|l| l.contains("compacting garbage collection")).count();
        check_all(20, &clauses, &questions, &answers(&stdout), Some((&cnf, &proof)), false);
    }
    assert!(compactions > 0, "the test must see a compaction, or it checks nothing");
}

#[test]
fn what_the_mode_refuses() {
    let fx = Fixture::new("assume-refuse");
    let icnf = fx.path("toy.icnf");
    write_icnf(&icnf, &example(), &[vec![1, 2]]);
    // the RL policy and its log
    for (key, value) in [("SAT_POLICY", "stock"), ("SAT_POLICY_LOG", "/dev/null")] {
        let mut cmd = command(&["-q"], &[(key, value)]);
        let out = cmd.arg(&icnf).output().expect("run");
        assert_eq!(out.status.code(), Some(1), "{} must be refused", key);
        assert!(String::from_utf8_lossy(&out.stderr).contains(key) || String::from_utf8_lossy(&out.stdout).contains(key));
        assert!(answers(&String::from_utf8_lossy(&out.stdout)).is_empty());
    }
    // a plain DIMACS file in this mode, and a clause after the first question
    let cnf = fx.path("plain.cnf");
    write_cnf(&cnf, 6, &example());
    let (code, stdout) = run_icnf(&cnf, None, &["-q"], &[]);
    assert_eq!(code, 1, "{}", stdout);
    let late = fx.path("late.icnf");
    std::fs::write(&late, "p inccnf\n1 2 0\na 1 0\n-1 0\n").unwrap();
    let (code, stdout) = run_icnf(&late, None, &["-q"], &[]);
    assert_eq!(code, 1, "{}", stdout);
    assert!(answers(&stdout).is_empty(), "nothing is answered from a file that is refused");
    // an incremental file without the mode is a parse error of the plain reader
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    let out = cmd.arg("-q").arg(&icnf).output().expect("run");
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn off_means_no_trace() {
    // without the mode nothing of it shows: no exit line, the usual answer
    let fx = Fixture::new("assume-off");
    let cnf = fx.path("toy.cnf");
    write_cnf(&cnf, 6, &example());
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    let out = cmd.arg("-s").arg(&cnf).output().expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(10));
    assert!(stdout.contains("s SATISFIABLE"));
    assert!(!stdout.contains("c assume"), "{}", stdout);
    assert!(!stdout.lines().any(|l| l.starts_with("q ")));
}
