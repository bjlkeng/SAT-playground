//! The point step (bead SAT-playground-1v2.3.4): a formula whose variables
//! are the lines between interchangeable points (Ramsey R(s,t) on n points)
//! gets one lex-leader chain per swap of neighbouring points, and one for the
//! colour flip when red and blue play the same part. A toy without a solution
//! is refuted with a proof the gate accepts, a toy with one keeps a model of
//! the input, flipped literals are read, a tampered proof is rejected, a
//! formula without points sees the stock search, the step waits for a formula
//! the earlier passes left alone, and it keeps to the tick limit.

mod common;

use common::{run, run_shipped, solver_bin, Fixture};
use std::io::Write as _;
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

/// Ramsey R(s,t) on n points: line {i,j} (i < j, by i then j) is variable
/// k + 1, true = red; no red K_s, no blue K_t. With `flip` the lines whose
/// hash with the seed is odd are negated throughout.
fn ramsey(s: usize, t: usize, n: usize, flip: Option<u64>) -> (i64, Vec<Vec<i64>>) {
    let mut lit = std::collections::HashMap::new();
    let mut k = 0i64;
    for i in 0..n {
        for j in i + 1..n {
            k += 1;
            let odd = flip.is_some_and(|seed| (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(seed).rotate_left(17) & 1 == 1);
            lit.insert((i, j), if odd { -k } else { k });
        }
    }
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for (size, sign) in [(s, -1i64), (t, 1i64)] {
        for q in subsets(n, size) {
            let mut c = Vec::new();
            for a in 0..q.len() {
                for b in a + 1..q.len() {
                    c.push(sign * lit[&(q[a], q[b])]);
                }
            }
            clauses.push(c);
        }
    }
    (k, clauses)
}

fn subsets(n: usize, k: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut cur: Vec<usize> = (0..k).collect();
    loop {
        out.push(cur.clone());
        let mut i = k;
        while i > 0 && cur[i - 1] == n - k + i - 1 {
            i -= 1;
        }
        if i == 0 {
            return out;
        }
        cur[i - 1] += 1;
        for j in i..k {
            cur[j] = cur[j - 1] + 1;
        }
    }
}

fn write_cnf(path: &Path, vars: i64, clauses: &[Vec<i64>]) {
    let mut f = std::fs::File::create(path).expect("create cnf");
    writeln!(f, "p cnf {} {}", vars, clauses.len()).unwrap();
    for c in clauses {
        for l in c {
            write!(f, "{} ", l).unwrap();
        }
        writeln!(f, "0").unwrap();
    }
}

/// The stock path plus the point step, the model printed, with or without a
/// proof file: (stdout, stderr).
fn run_points(cnf: &Path, proof: Option<&Path>, env: &[(&str, &str)]) -> (String, String) {
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.args(common::STOCK_PATH).arg("--structpoints=1").arg(cnf);
    if let Some(p) = proof {
        cmd.arg(p);
    }
    let out = cmd.output().expect("run sat-solver");
    (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// Every clause has a literal the printed model makes true, and the model
/// names no variable past the input's.
fn model_checks(stdout: &str, vars: i64, clauses: &[Vec<i64>]) {
    let mut lits: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for line in stdout.lines().filter(|l| l.starts_with("v ")) {
        for tok in line[2..].split_whitespace() {
            let l: i64 = tok.parse().unwrap();
            if l != 0 {
                assert!(l.abs() <= vars, "the model names variable {}, past the input's {}", l.abs(), vars);
                lits.insert(l);
            }
        }
    }
    for c in clauses {
        assert!(c.iter().any(|l| lits.contains(l)), "clause {:?} is not satisfied by the model", c);
    }
}

fn points_work(cnf: &Path, proof: Option<&Path>, limit: Option<u64>) -> (String, u64) {
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    if let Some(l) = limit {
        cmd.env("SAT_LIMIT_TICKS", l.to_string());
    }
    cmd.arg("-n").args(common::STOCK_PATH).arg("--structpoints=1").arg(cnf);
    if let Some(p) = proof {
        cmd.arg(p);
    }
    let out = cmd.output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let status = stdout.lines().find_map(|l| l.strip_prefix("s ").map(|s| s.trim().to_string())).unwrap_or_default();
    let work = stdout
        .lines()
        .find_map(|l| l.strip_prefix("c workclock ").and_then(|r| r.split_whitespace().find_map(|t| t.strip_prefix("work=")).and_then(|v| v.parse().ok())))
        .expect("work= on the workclock line");
    (status, work)
}

#[test]
fn point_step_refutes_toy_ramsey_with_checked_proofs() {
    let fx = Fixture::new("structpoints_toys");
    for (s, t, n) in [(3usize, 4usize, 9usize), (3, 5, 14)] {
        let name = format!("r{}{}_{}", s, t, n);
        let cnf = fx.path(&format!("{}.cnf", name));
        let (vars, clauses) = ramsey(s, t, n, None);
        write_cnf(&cnf, vars, &clauses);
        let proof = fx.path(&format!("{}.drat", name));
        let (stdout, stderr) = run_points(&cnf, Some(&proof), &[("SAT_STRUCT_DEBUG", "1")]);
        assert!(stdout.contains("s UNSATISFIABLE"), "{}: {}\n{}", name, stdout, stderr);
        let fired = format!("points: {} points, {} lex-leader chains,", n, n - 1);
        assert!(stdout.contains(&fired), "{}: the step did not fire: {}\n{}", name, stdout, stderr);
        assert_eq!(gate(&cnf, &proof), "ok", "{}: gate", name);
    }
}

#[test]
fn point_step_cuts_the_search_on_a_toy() {
    // R(3,5) on 14 points: the stock search needs millions of conflicts (no
    // answer in 300 s, measured 2026-11-05); with the chains a few hundred
    let fx = Fixture::new("structpoints_cut");
    let cnf = fx.path("r35_14.cnf");
    let (vars, clauses) = ramsey(3, 5, 14, None);
    write_cnf(&cnf, vars, &clauses);
    let r = run(&cnf, &["--structpoints=1"], &[]);
    assert_eq!(r.status, "UNSATISFIABLE", "{}", r.stdout);
    let conflicts = *r.stats.get("conflicts").expect("conflicts in the statistics");
    assert!(conflicts < 20_000, "{} conflicts with the chains", conflicts);
}

#[test]
fn point_step_keeps_a_model_of_a_satisfiable_toy() {
    // R(3,4) on 8 points has colourings: the answer stays SAT, the model is
    // one of the input, and it leaves the helper variables out
    let fx = Fixture::new("structpoints_sat");
    let cnf = fx.path("r34_8.cnf");
    let (vars, clauses) = ramsey(3, 4, 8, None);
    write_cnf(&cnf, vars, &clauses);
    for with_proof in [false, true] {
        let proof = fx.path("r34_8.drat");
        let (stdout, stderr) = run_points(&cnf, if with_proof { Some(proof.as_path()) } else { None }, &[]);
        assert!(stdout.contains("s SATISFIABLE"), "proof {}: {}\n{}", with_proof, stdout, stderr);
        assert!(stdout.contains("points: 8 points, 7 lex-leader chains,"), "proof {}: {}", with_proof, stdout);
        model_checks(&stdout, vars, &clauses);
    }
}

#[test]
fn point_step_reads_flipped_lines() {
    // half the lines negated throughout: the class holds literals of both
    // signs, the chains compare variables with the literals the swaps send
    // them to, and the proof still checks
    let fx = Fixture::new("structpoints_flip");
    for seed in [3u64, 11] {
        let cnf = fx.path(&format!("f{}.cnf", seed));
        let (vars, clauses) = ramsey(3, 4, 9, Some(seed));
        assert!(clauses.iter().flatten().any(|&l| l > 0) && clauses.iter().flatten().any(|&l| l < 0));
        write_cnf(&cnf, vars, &clauses);
        let proof = fx.path(&format!("f{}.drat", seed));
        let (stdout, stderr) = run_points(&cnf, Some(&proof), &[]);
        assert!(stdout.contains("s UNSATISFIABLE"), "seed {}: {}\n{}", seed, stdout, stderr);
        assert!(stdout.contains("points: 9 points, 8 lex-leader chains,"), "seed {}: {}", seed, stdout);
        assert_eq!(gate(&cnf, &proof), "ok", "seed {}", seed);
    }
}

#[test]
fn the_colour_flip_is_broken_too() {
    // R(3,3) on 6 points (no solution) and R(4,4) on 9 (solutions): red and
    // blue play the same part, so flipping every line is a symmetry and one
    // more chain makes the first line blue
    let fx = Fixture::new("structpoints_colour");
    let cnf = fx.path("r33_6.cnf");
    let (vars, clauses) = ramsey(3, 3, 6, None);
    write_cnf(&cnf, vars, &clauses);
    let proof = fx.path("r33_6.drat");
    let (stdout, stderr) = run_points(&cnf, Some(&proof), &[]);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}\n{}", stdout, stderr);
    assert!(stdout.contains("points: 6 points, 6 lex-leader chains (one for the colour flip),"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    let cnf = fx.path("r44_9.cnf");
    let (vars, clauses) = ramsey(4, 4, 9, None);
    write_cnf(&cnf, vars, &clauses);
    let (stdout, stderr) = run_points(&cnf, None, &[]);
    assert!(stdout.contains("s SATISFIABLE"), "{}\n{}", stdout, stderr);
    assert!(stdout.contains("points: 9 points, 9 lex-leader chains (one for the colour flip),"), "{}", stdout);
    model_checks(&stdout, vars, &clauses);
}

#[test]
fn a_tampered_point_proof_fails_the_gate() {
    let fx = Fixture::new("structpoints_tamper");
    let cnf = fx.path("r34_9.cnf");
    let (vars, clauses) = ramsey(3, 4, 9, None);
    write_cnf(&cnf, vars, &clauses);
    let proof = fx.path("r34_9.drat");
    let (stdout, _) = run_points(&cnf, Some(&proof), &[]);
    assert!(stdout.contains("points: 9 points"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    // the first chain's witness swaps points 0 and 1, so line (0,2) goes to
    // line (1,2): send it to line (1,3) instead, which is no symmetry
    let pbp = fx.path("r34_9.drat.pbp");
    let text = std::fs::read_to_string(&pbp).unwrap();
    let tampered = text.replacen(" x2 -> x9 ", " x2 -> x10 ", 1);
    assert_ne!(tampered, text, "the proof has the first chain's witness");
    std::fs::write(&pbp, tampered).unwrap();
    let verdict = gate(&cnf, &proof);
    assert!(verdict.starts_with("FAIL"), "{}", verdict);
}

#[test]
fn point_step_without_points_leaves_the_search_unchanged() {
    // a random 3-SAT formula has no points: nothing is derived, the search
    // is the stock one, and the work clock grows by the step's work
    let fx = Fixture::new("structpoints_none");
    let cnf = fx.path("r.cnf");
    common::random_3sat(120, 4.26, 7, &cnf);
    let stock = run(&cnf, &[], &[]);
    let with = run(&cnf, &["--structpoints=1"], &[]);
    assert_eq!(stock.status, with.status);
    assert!(with.stdout.contains("points: no point symmetry, work"), "{}", with.stdout);
    for key in ["conflicts", "decisions", "propagations"] {
        assert_eq!(stock.stats.get(key), with.stats.get(key), "{} differs", key);
    }
    assert!(with.work() >= stock.work(), "the step's work is in the work clock");
}

#[test]
fn the_symmetry_passes_stay_out_once_the_point_step_has_chains() {
    // five pigeons in five holes beside R(3,4) on 9 points: the point step
    // goes first and adds its chains, and the parity and symmetry passes then
    // stay out (their redundance steps would have to map the chains); the
    // search refutes the formula with a proof the gate accepts
    let fx = Fixture::new("structpoints_first");
    let (vars, mut clauses) = ramsey(3, 4, 9, None);
    let (p, h) = (5i64, 5i64);
    let v = |i: i64, j: i64| vars + i * h + j + 1;
    for i in 0..p {
        clauses.push((0..h).map(|j| v(i, j)).collect());
    }
    for j in 0..h {
        for a in 0..p {
            for b in a + 1..p {
                clauses.push(vec![-v(a, j), -v(b, j)]);
            }
        }
    }
    let cnf = fx.path("both.cnf");
    write_cnf(&cnf, vars + p * h, &clauses);
    let r = run_shipped(&cnf, &["--structpoints=1"], &[]);
    assert_eq!(r.status, "UNSATISFIABLE", "{}", r.stdout);
    assert!(r.stdout.contains("points: 9 points, 8 lex-leader chains,"), "{}", r.stdout);
    assert!(r.stdout.contains("parity: skipped after the point step's chains"), "{}", r.stdout);
    assert!(r.stdout.contains("symmetry: skipped after the point step's chains"), "{}", r.stdout);
    let proof = fx.path("both.drat");
    let out = Command::new(solver_bin()).arg("-n").arg("--structpoints=1").arg(&cnf).arg(&proof).output().expect("run sat-solver");
    assert!(String::from_utf8_lossy(&out.stdout).contains("s UNSATISFIABLE"));
    assert_eq!(gate(&cnf, &proof), "ok");
}

#[test]
fn without_points_the_symmetry_passes_run_as_before() {
    // a pigeonhole formula has rows but no points: with the step on, the
    // symmetry pass refutes it exactly as it does without the step, and only
    // the work clock grows by the step's look
    let fx = Fixture::new("structpoints_rows");
    let cnf = fx.path("php.cnf");
    common::php(7, &cnf);
    let off = run_shipped(&cnf, &[], &[]);
    let on = run_shipped(&cnf, &["--structpoints=1"], &[]);
    assert_eq!(off.status, "UNSATISFIABLE");
    assert_eq!(on.status, off.status);
    assert!(on.stdout.contains("points: no point symmetry, work"), "{}", on.stdout);
    let sym = |r: &common::Run| r.stdout.lines().find(|l| l.contains("structure pass: symmetry:")).map(|l| l.to_string());
    assert!(sym(&off).is_some(), "{}", off.stdout);
    assert_eq!(sym(&on), sym(&off));
    for key in ["conflicts", "decisions", "propagations"] {
        assert_eq!(on.stats.get(key), off.stats.get(key), "{} differs", key);
    }
    assert!(on.work() > off.work(), "the step's work is in the work clock");
}

#[test]
fn every_step_of_the_point_step_keeps_to_the_tick_limit() {
    // stopped at fractions of its own total, with and without a proof file:
    // no limit is passed by more than 10%. R(3,5) on 14 points; R(3,3) on 64
    // points, whose tables (4M for the counts alone) are reserved before they
    // are built; R(3,4) on 5 points with a proof, whose chains are written in
    // one reserved block (both from Codex review round 1)
    let fx = Fixture::new("structpoints_limits");
    for (s, t, n, fracs) in [(3usize, 5usize, 14usize, &[5u64, 20, 40, 60, 80][..]), (3, 3, 64, &[5, 10, 20, 40, 60, 80]), (3, 4, 5, &[60, 80, 95])] {
        let name = format!("r{}{}_{}", s, t, n);
        let cnf = fx.path(&format!("{}.cnf", name));
        let (vars, clauses) = ramsey(s, t, n, None);
        write_cnf(&cnf, vars, &clauses);
        for with_proof in [false, true] {
            let proof = fx.path(&format!("{}.drat", name));
            let p = if with_proof { Some(proof.as_path()) } else { None };
            let (status, total) = points_work(&cnf, p, None);
            assert!(status == "UNSATISFIABLE" || status == "SATISFIABLE", "{} proof {}: {}", name, with_proof, status);
            for &frac in fracs {
                let limit = total * frac / 100;
                let (st, work) = points_work(&cnf, p, Some(limit));
                assert!(work <= limit + limit / 10, "{} proof {}: limit {} of {}: work {} ({})", name, with_proof, limit, total, work, st);
            }
        }
    }
}

#[test]
fn the_refinement_reserves_its_tables_and_splits() {
    // 100,000 variables and one clause: the first round hashes and splits
    // every literal at once, and the tables are per variable, so each bulk
    // step is reserved before it runs; the limit is kept within 10% at any
    // fraction (Codex review round 2: 800,016 work under a 650,000 limit)
    let fx = Fixture::new("structpoints_sparse");
    let cnf = fx.path("sparse.cnf");
    write_cnf(&cnf, 100_000, &[vec![1, 100_000]]);
    let (status, total) = points_work(&cnf, None, None);
    assert_eq!(status, "SATISFIABLE");
    for frac in [10u64, 25, 40, 55, 70, 85] {
        let limit = total * frac / 100;
        let (st, work) = points_work(&cnf, None, Some(limit));
        assert!(work <= limit + limit / 10, "limit {} of {}: work {} ({})", limit, total, work, st);
    }
    let (_, work) = points_work(&cnf, None, Some(650_000));
    assert!(work <= 650_000 + 65_000, "work {} under the review's 650,000", work);
    // one clause of 100,000 literals: every clause is reserved before it is
    // scanned (Codex review round 3: 600,006 work under a 510,000 limit)
    let long = fx.path("long.cnf");
    write_cnf(&long, 100_000, &[(1..=100_000).collect()]);
    let (status, total) = points_work(&long, None, None);
    assert_eq!(status, "SATISFIABLE");
    for frac in [10u64, 30, 50, 70, 90] {
        let limit = total * frac / 100;
        let (st, work) = points_work(&long, None, Some(limit));
        assert!(work <= limit + limit / 10, "long clause: limit {} of {}: work {} ({})", limit, total, work, st);
    }
    let (_, work) = points_work(&long, None, Some(510_000));
    assert!(work <= 510_000 + 51_000, "work {} under the review's 510,000", work);
}

#[test]
fn the_point_step_takes_the_same_work_every_run() {
    // the positive and the negative class of the lines tie on size: the
    // candidates are ordered by sign and colour, never by the hash map, so
    // runs agree on the work and on what is added (Codex review round 1)
    let fx = Fixture::new("structpoints_same");
    let cnf = fx.path("r34_9.cnf");
    let (vars, clauses) = ramsey(3, 4, 9, None);
    write_cnf(&cnf, vars, &clauses);
    let runs: Vec<common::Run> = (0..6).map(|_| run(&cnf, &["--structpoints=1"], &[])).collect();
    for r in &runs[1..] {
        common::assert_same_trajectory(&runs[0], r, "two runs of the point step");
    }
    let note = |r: &common::Run| r.stdout.lines().find(|l| l.contains("points:")).map(|l| l.to_string());
    assert!(note(&runs[0]).is_some_and(|l| l.contains("9 points")));
    assert!(runs.iter().all(|r| note(r) == note(&runs[0])));
}
