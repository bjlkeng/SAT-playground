//! The structure pass (plan section 11, 2026-10-07; epic SAT-playground-1v2):
//! off means off, the symmetry pass settles a pigeonhole formula with a
//! proof the two-stage gate accepts, and a satisfiable formula keeps a
//! checkable model.

mod common;

use common::{php, run, run_shipped, run_with_proof, solver_bin, Fixture};
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

/// The solver with the shell's own settings cleared, as the common helpers do.
fn solver_command() -> Command {
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    cmd
}

/// The stock path plus `options`, so a test of one pass sees only that pass
/// (the shipped defaults have structsym and structparity on).
fn run_options_with_proof(cnf: &Path, proof: &Path, options: &[&str]) -> String {
    let out = solver_command()
        .arg("-n")
        .args(common::STOCK_PATH)
        .args(options)
        .arg(cnf)
        .arg(proof)
        .output()
        .expect("run sat-solver");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn every_pass_off_leaves_the_stock_trajectory_and_no_artifacts() {
    let fx = Fixture::new("structure_off");
    let cnf = fx.path("php7.cnf");
    php(7, &cnf);
    // `run` puts every pass off (common::STOCK_PATH); the same with the
    // budget option set is the same trajectory
    let stock = run(&cnf, &[], &[]);
    let again = run(&cnf, &["--structticks=1"], &[]);
    common::assert_same_trajectory(&stock, &again, "struct options off");
    let proof = fx.path("off.drat");
    let out = run_with_proof(&cnf, &proof, &[]);
    assert_eq!(out.status, "UNSATISFIABLE");
    assert!(!fx.path("off.drat.pbp").exists(), "no VeriPB proof with every pass off");
    assert!(!fx.path("off.drat.derived.cnf").exists());
    assert!(!out.stdout.contains("structure pass"));
}

#[test]
fn the_shipped_defaults_run_the_promoted_passes() {
    let fx = Fixture::new("structure_default");
    let cnf = fx.path("php7.cnf");
    php(7, &cnf);
    let proof = fx.path("default.drat");
    let out = solver_command().arg("-n").arg(&cnf).arg(&proof).output().expect("run sat-solver");
    let out = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.contains("\ns UNSATISFIABLE"), "{}", out);
    assert!(fx.path("default.drat.pbp").exists(), "the symmetry pass is on by default");
    assert!(out.contains("symmetry:"), "{}", out);
    assert_eq!(gate(&cnf, &proof), "ok");
    // with no budget the pass finds nothing and the stock search decides
    let capped = run_shipped(&cnf, &["--structticks=0"], &[]);
    assert_eq!(capped.status, "UNSATISFIABLE");
    assert!(!capped.stdout.contains("symmetry:"), "no fixing with no budget");
}

#[test]
fn symmetry_pass_refutes_pigeonhole_with_a_checked_proof() {
    let fx = Fixture::new("structure_php");
    let cnf = fx.path("php9.cnf");
    php(9, &cnf);
    let proof = fx.path("sym.drat");
    let stdout = run_options_with_proof(&cnf, &proof, &["--structsym=1"]);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    assert!(stdout.contains("structure pass: symmetry:"), "{}", stdout);
    assert!(fx.path("sym.drat.pbp").exists() && fx.path("sym.drat.derived.cnf").exists());
    let verdict = gate(&cnf, &proof);
    assert_eq!(verdict, "ok", "two-stage gate: {}", verdict);
}

#[test]
fn a_tampered_structure_proof_fails_the_gate() {
    let fx = Fixture::new("structure_tamper");
    let cnf = fx.path("php6.cnf");
    php(6, &cnf);
    let proof = fx.path("t.drat");
    let stdout = run_options_with_proof(&cnf, &proof, &["--structsym=1"]);
    assert!(stdout.contains("s UNSATISFIABLE"));
    assert_eq!(gate(&cnf, &proof), "ok");
    // the pass refuted the formula: the proof's last step derives the empty
    // clause; without it the conclusion is unjustified and must be caught
    let pbp = fx.path("t.drat.pbp");
    let text = std::fs::read_to_string(&pbp).unwrap();
    let tampered: String = text.lines().filter(|l| l.trim() != "rup >= 1 ;").map(|l| format!("{}\n", l)).collect();
    assert_ne!(text, tampered, "the proof ends in the empty clause");
    std::fs::write(&pbp, tampered).unwrap();
    let verdict = gate(&cnf, &proof);
    assert!(verdict.starts_with("FAIL"), "{}", verdict);
}

#[test]
fn satisfiable_formula_keeps_a_model_of_the_input() {
    let fx = Fixture::new("structure_sat");
    let cnf = fx.path("php_sat.cnf");
    // 4 pigeons, 5 holes: the symmetry pass fixes some literals, the model
    // must still satisfy every input clause
    let pigeons = 4;
    let holes = 5;
    let var = |p: usize, h: usize| (p * holes + h + 1) as i64;
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for p in 0..pigeons {
        clauses.push((0..holes).map(|h| var(p, h)).collect());
    }
    for h in 0..holes {
        for a in 0..pigeons {
            for b in (a + 1)..pigeons {
                clauses.push(vec![-var(a, h), -var(b, h)]);
            }
        }
    }
    let mut text = format!("p cnf {} {}\n", pigeons * holes, clauses.len());
    for c in &clauses {
        for l in c {
            text.push_str(&format!("{} ", l));
        }
        text.push_str("0\n");
    }
    std::fs::write(&cnf, text).unwrap();
    let out = solver_command().arg("--structsym=1").arg(&cnf).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("s SATISFIABLE"), "{}", stdout);
    let model: Vec<i64> = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("v "))
        .flat_map(|l| l.split_whitespace().map(|t| t.parse::<i64>().unwrap()))
        .filter(|&l| l != 0)
        .collect();
    for c in &clauses {
        assert!(c.iter().any(|l| model.contains(l)), "clause {:?} unsatisfied", c);
    }
}

#[test]
fn gate_rejects_an_unchecked_extension_and_counts_the_pass_in_the_work_clock() {
    // the pass extends php(4 pigeons, 5 holes) without refuting it; a proof
    // whose output section no longer names the extended formula must fail
    let fx = Fixture::new("structure_output");
    let cnf = fx.path("sat.cnf");
    let holes = 5;
    let var = |p: usize, h: usize| (p * holes + h + 1) as i64;
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for p in 0..4 {
        clauses.push((0..holes).map(|h| var(p, h)).collect());
    }
    for h in 0..holes {
        for a in 0..4 {
            for b in (a + 1)..4 {
                clauses.push(vec![-var(a, h), -var(b, h)]);
            }
        }
    }
    // one more clause makes it unsatisfiable without touching the symmetry
    // the pass uses first: every pigeon is also forbidden from every hole
    // but through a fresh literal that is then falsified
    clauses.push(vec![21]);
    for p in 0..4 {
        for h in 0..holes {
            clauses.push(vec![-21, -var(p, h)]);
        }
    }
    let mut text = format!("p cnf 21 {}\n", clauses.len());
    for c in &clauses {
        for l in c {
            text.push_str(&format!("{} ", l));
        }
        text.push_str("0\n");
    }
    std::fs::write(&cnf, text).unwrap();
    let proof = fx.path("o.drat");
    let stdout = run_options_with_proof(&cnf, &proof, &["--structsym=1"]);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    let work: u64 = stdout
        .lines()
        .find(|l| l.starts_with("c workclock"))
        .and_then(|l| l.split_whitespace().find_map(|kv| kv.strip_prefix("ticks=")).map(|v| v.parse().unwrap()))
        .expect("workclock ticks");
    assert!(work > 0, "the pass's work counts in the work clock");
    let pbp = fx.path("o.drat.pbp");
    if pbp.exists() {
        assert_eq!(gate(&cnf, &proof), "ok");
        let text = std::fs::read_to_string(&pbp).unwrap();
        let tampered = text.replace("output EQUISATISFIABLE FILE ;", "output NONE ;");
        if tampered != text {
            std::fs::write(&pbp, tampered).unwrap();
            let verdict = gate(&cnf, &proof);
            assert!(verdict.starts_with("FAIL"), "{}", verdict);
        }
    }
}

/// Tseitin's formula on a cycle of n vertices with one odd charge: every
/// edge variable sits in exactly two XOR constraints, the sum of all
/// constraints is 0 = 1.
fn tseitin_cycle(n: usize, path: &Path) {
    let edge = |i: usize| (i + 1) as i64; // edge i joins vertex i and vertex (i+1) mod n
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for v in 0..n {
        let (a, b) = (edge((v + n - 1) % n), edge(v));
        let charge = if v == 0 { 1 } else { 0 };
        // a xor b = charge: forbid the two patterns of the other parity
        if charge == 1 {
            clauses.push(vec![a, b]);
            clauses.push(vec![-a, -b]);
        } else {
            clauses.push(vec![a, -b]);
            clauses.push(vec![-a, b]);
        }
    }
    let mut text = format!("p cnf {} {}\n", n, clauses.len());
    for c in &clauses {
        for l in c {
            text.push_str(&format!("{} ", l));
        }
        text.push_str("0\n");
    }
    std::fs::write(path, text).unwrap();
}

#[test]
fn parity_pass_refutes_tseitin_with_a_checked_proof() {
    let fx = Fixture::new("structure_tseitin");
    let cnf = fx.path("tseitin.cnf");
    tseitin_cycle(40, &cnf);
    let proof = fx.path("p.drat");
    let stdout = run_options_with_proof(&cnf, &proof, &["--structparity=1"]);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    assert!(stdout.contains("structure pass: parity:") && stdout.contains("refuted"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    // the symmetry pass alone sees no interchangeable rows here
    let stdout = run_options_with_proof(&cnf, &proof, &["--structsym=1"]);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    assert!(!stdout.contains("refuted"), "{}", stdout);
}

/// Pigeonhole clauses (`pigeons` into `holes`) with every variable replaced
/// by the XOR of `arity` fresh ones: a literal is false on half of the
/// assignments of its block, and a clause forbids every combination of
/// those, so a clause of w literals becomes (2^(arity-1))^w clauses.
/// Returns the clauses as written.
fn xorified_php(pigeons: usize, holes: usize, arity: usize, path: &Path) -> Vec<Vec<i64>> {
    let var = |p: usize, h: usize| p * holes + h; // the substituted variable, from 0
    let mut original: Vec<Vec<(usize, bool)>> = Vec::new(); // (variable, positive)
    for p in 0..pigeons {
        original.push((0..holes).map(|h| (var(p, h), true)).collect());
    }
    for h in 0..holes {
        for a in 0..pigeons {
            for b in a + 1..pigeons {
                original.push(vec![(var(a, h), false), (var(b, h), false)]);
            }
        }
    }
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for c in &original {
        // per literal, the sign patterns of its block that falsify it
        let mut parts: Vec<Vec<Vec<i64>>> = Vec::new();
        for &(x, positive) in c {
            let mut options: Vec<Vec<i64>> = Vec::new();
            for bits in 0..1u32 << arity {
                // the block's assignment `bits` has parity = the value of x;
                // the literal is false when that value is not its sign
                let value = bits.count_ones() % 2 == 1;
                if value == positive {
                    continue;
                }
                options.push((0..arity).map(|j| {
                    let v = (x * arity + j + 1) as i64;
                    if bits >> j & 1 == 1 { -v } else { v } // the clause forbids this assignment
                }).collect());
            }
            parts.push(options);
        }
        let mut product: Vec<Vec<i64>> = vec![Vec::new()];
        for options in &parts {
            let mut next = Vec::new();
            for prefix in &product {
                for o in options {
                    let mut clause = prefix.clone();
                    clause.extend(o);
                    next.push(clause);
                }
            }
            product = next;
        }
        clauses.extend(product);
    }
    let mut text = format!("p cnf {} {}\n", pigeons * holes * arity, clauses.len());
    for c in &clauses {
        for l in c {
            text.push_str(&format!("{} ", l));
        }
        text.push_str("0\n");
    }
    std::fs::write(path, text).unwrap();
    clauses
}

#[test]
fn parity_pass_undoes_an_xor_substitution_by_flip_symmetries() {
    let fx = Fixture::new("structure_xorified");
    // 5 pigeons, 4 holes, every variable the XOR of two: 20 blocks, and
    // flipping both variables of a block maps the formula onto itself
    let cnf = fx.path("xphp2.cnf");
    xorified_php(5, 4, 2, &cnf);
    let proof = fx.path("xphp2.drat");
    let stdout = run_options_with_proof(&cnf, &proof, &["--structparity=1"]);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    assert!(stdout.contains("parity: 20 flip symmetries (of up to 2 variables), 20 variables fixed"), "{}", stdout);
    assert!(stdout.contains("20 derived clauses (20 units)"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    let pbp = std::fs::read_to_string(fx.path("xphp2.drat.pbp")).unwrap();
    assert_eq!(pbp.lines().filter(|l| l.starts_with("red ")).count(), 20, "one redundance step per block");
    // the XOR of three: two free variables in every block, each with a flip
    // of its own (the free variable and the block's third)
    let cnf = fx.path("xphp3.cnf");
    xorified_php(3, 2, 3, &cnf);
    let proof = fx.path("xphp3.drat");
    let stdout = run_options_with_proof(&cnf, &proof, &["--structparity=1"]);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    assert!(stdout.contains("parity: 12 flip symmetries (of up to 2 variables), 12 variables fixed"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    // with the step off nothing is derived
    let stdout = run_options_with_proof(&cnf, &proof, &[]);
    assert!(!stdout.contains("structure pass"), "{}", stdout);
}

#[test]
fn flip_symmetries_keep_a_model_of_the_input_and_stop_at_a_work_limit() {
    let fx = Fixture::new("structure_xorified_sat");
    // 4 pigeons, 5 holes: satisfiable, and so is the substituted formula
    let cnf = fx.path("xphp_sat.cnf");
    let clauses = xorified_php(4, 5, 2, &cnf);
    let out = solver_command().arg("--structparity=1").arg("--structsym=0").arg(&cnf).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("s SATISFIABLE"), "{}", stdout);
    assert!(stdout.contains("parity: 20 flip symmetries"), "{}", stdout);
    let model: Vec<i64> = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("v "))
        .flat_map(|l| l.split_whitespace().map(|t| t.parse::<i64>().unwrap()))
        .filter(|&l| l != 0)
        .collect();
    for c in &clauses {
        assert!(c.iter().any(|l| model.contains(l)), "clause {:?} unsatisfied", c);
    }
    // any work limit: the step stops where it is, and what it derived by
    // then is sound (a status line, no panic)
    for limit in ["1", "50", "200", "500", "1000", "2000", "3000", "5000", "10000"] {
        let r = run_shipped(&cnf, &["--structsym=0"], &[("SAT_LIMIT_TICKS", limit)]);
        assert!(matches!(r.status.as_str(), "UNKNOWN" | "SATISFIABLE"), "limit {}: {}\n{}", limit, r.status, r.stderr);
        assert_eq!(r.exit_code, if r.status == "SATISFIABLE" { 10 } else { 0 }, "limit {}: {}", limit, r.stderr);
    }
}

/// A round robin with sides: n teams, D days, every pair plays at least
/// once on some day on one of two sides, a team plays at most one game a
/// day, and the two sides are told apart by a second binary between the
/// side-0 literals of two games sharing a team (as MVRoundRobin has). With
/// D < n - 1 the days cannot hold every pair.
fn round_robin(n: usize, days: usize, path: &Path) {
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            pairs.push((i, j));
        }
    }
    let var = |p: usize, d: usize, s: usize| (1 + (p * days + d) * 2 + s) as i64;
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for p in 0..pairs.len() {
        clauses.push((0..days).flat_map(|d| [var(p, d, 0), var(p, d, 1)]).collect());
    }
    for d in 0..days {
        for t in 0..n {
            let mine: Vec<usize> = (0..pairs.len()).filter(|&p| pairs[p].0 == t || pairs[p].1 == t).collect();
            for (k, &a) in mine.iter().enumerate() {
                for &b in &mine[k + 1..] {
                    for sa in 0..2 {
                        for sb in 0..2 {
                            clauses.push(vec![-var(a, d, sa), -var(b, d, sb)]);
                        }
                    }
                    clauses.push(vec![-var(a, d, 0), -var(b, d, 0)]);
                }
            }
        }
    }
    let mut text = format!("p cnf {} {}\n", pairs.len() * days * 2, clauses.len());
    for c in &clauses {
        for l in c {
            text.push_str(&format!("{} ", l));
        }
        text.push_str("0\n");
    }
    std::fs::write(path, text).unwrap();
}

/// A k-clique in a free graph on n vertices, c-coloured: positions hold
/// distinct vertices, clique members are adjacent, adjacent vertices
/// differ in colour. The vertices are interchangeable rows that share
/// their edge variables; with c < k the clique cannot be coloured.
fn clique_colouring(n: usize, k: usize, c: usize, path: &Path) {
    let mut nv = 0i64;
    let mut edge = std::collections::HashMap::new();
    for u in 0..n {
        for v in u + 1..n {
            nv += 1;
            edge.insert((u, v), nv);
            edge.insert((v, u), nv);
        }
    }
    let y = |v: usize, p: usize| (nv as usize + v * k + p + 1) as i64;
    let x = |v: usize, q: usize| (nv as usize + n * k + v * c + q + 1) as i64;
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for p in 0..k {
        clauses.push((0..n).map(|v| y(v, p)).collect());
        for u in 0..n {
            for v in u + 1..n {
                clauses.push(vec![-y(u, p), -y(v, p)]);
            }
        }
    }
    for v in 0..n {
        for p in 0..k {
            for q in p + 1..k {
                clauses.push(vec![-y(v, p), -y(v, q)]);
            }
        }
    }
    for p in 0..k {
        for q in p + 1..k {
            for u in 0..n {
                for v in 0..n {
                    if u != v {
                        clauses.push(vec![-y(u, p), -y(v, q), edge[&(u, v)]]);
                    }
                }
            }
        }
    }
    for v in 0..n {
        clauses.push((0..c).map(|q| x(v, q)).collect());
        for a in 0..c {
            for b in a + 1..c {
                clauses.push(vec![-x(v, a), -x(v, b)]);
            }
        }
    }
    for u in 0..n {
        for v in u + 1..n {
            for q in 0..c {
                clauses.push(vec![-edge[&(u, v)], -x(u, q), -x(v, q)]);
            }
        }
    }
    let total = nv as usize + n * k + n * c;
    let mut text = format!("p cnf {} {}\n", total, clauses.len());
    for cl in &clauses {
        for l in cl {
            text.push_str(&format!("{} ", l));
        }
        text.push_str("0\n");
    }
    std::fs::write(path, text).unwrap();
}

#[test]
fn symmetry_pass_refutes_clique_colouring_through_rows_that_share_entries() {
    let fx = Fixture::new("structure_cliquecolouring");
    let cnf = fx.path("cc.cnf");
    clique_colouring(7, 4, 3, &cnf);
    let proof = fx.path("cc.drat");
    let stdout = run_options_with_proof(&cnf, &proof, &["--structsym=1"]);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    // the vertex rows share their edge variables; after the first vertex is
    // fixed the others must still be found interchangeable (the rows are
    // re-aligned to the new reference), and the positions, then the
    // colours, fix down to the empty clause
    assert!(stdout.contains("structure pass: symmetry:") && stdout.contains("refuted"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
}

#[test]
fn symmetry_pass_breaks_day_symmetry_with_clauses_not_units() {
    let fx = Fixture::new("structure_roundrobin");
    let cnf = fx.path("rr.cnf");
    round_robin(6, 4, &cnf);
    let proof = fx.path("rr.drat");
    let stdout = run_options_with_proof(&cnf, &proof, &["--structsym=1"]);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    // the days are interchangeable rows, but no clause lies in one column
    // (a game's clause holds both sides on every day): the fixings are
    // clauses, "game k plays on a day at most k", and the search finishes
    assert!(stdout.contains("structure pass: symmetry: 1 matrix") && stdout.contains("3 fixings"), "{}", stdout);
    assert!(stdout.contains("3 derived clauses (0 units)"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    let pbp = std::fs::read_to_string(fx.path("rr.drat.pbp")).unwrap();
    assert_eq!(pbp.lines().filter(|l| l.starts_with("del id")).count(), 3, "the steps of each fixing are deleted");
    let derived = std::fs::read_to_string(fx.path("rr.drat.derived.cnf")).unwrap();
    assert!(derived.lines().filter(|l| !l.starts_with('p') && !l.starts_with('c')).all(|l| l.split_whitespace().count() >= 3), "{}", derived);
}

/// An n x n board without two opposite corners, covered by dominoes: one
/// variable per domino, every square in at least one and in no two. The
/// removed corners have one colour, so the squares of the other colour
/// outnumber the dominoes that can cover them.
fn mutilated_board(n: usize, path: &Path) {
    let removed = |r: usize, c: usize| (r == 0 && c == 0) || (r == n - 1 && c == n - 1);
    let mut dominoes: Vec<((usize, usize), (usize, usize))> = Vec::new();
    for r in 0..n {
        for c in 0..n {
            if removed(r, c) {
                continue;
            }
            if c + 1 < n && !removed(r, c + 1) {
                dominoes.push(((r, c), (r, c + 1)));
            }
            if r + 1 < n && !removed(r + 1, c) {
                dominoes.push(((r, c), (r + 1, c)));
            }
        }
    }
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for r in 0..n {
        for c in 0..n {
            if removed(r, c) {
                continue;
            }
            let on: Vec<i64> = dominoes.iter().enumerate().filter(|(_, d)| d.0 == (r, c) || d.1 == (r, c)).map(|(i, _)| (i + 1) as i64).collect();
            clauses.push(on.clone());
            for a in 0..on.len() {
                for b in a + 1..on.len() {
                    clauses.push(vec![-on[a], -on[b]]);
                }
            }
        }
    }
    let mut text = format!("p cnf {} {}\n", dominoes.len(), clauses.len());
    for c in &clauses {
        for l in c {
            text.push_str(&format!("{} ", l));
        }
        text.push_str("0\n");
    }
    std::fs::write(path, text).unwrap();
}

#[test]
fn counting_pass_refutes_a_mutilated_board_after_the_other_passes() {
    let fx = Fixture::new("structure_board");
    let cnf = fx.path("board.cnf");
    mutilated_board(8, &cnf);
    let proof = fx.path("board.drat");
    // the shipped defaults: the other passes first (they find no way to
    // settle it), then the counting pass within its own limit
    let out = solver_command().arg("-n").arg(&cnf).arg(&proof).output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    assert!(stdout.contains("structure pass: counting:") && stdout.contains("refuted"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    // the same answer when no proof file is asked for
    let r = run_shipped(&cnf, &[], &[]);
    assert_eq!(r.status, "UNSATISFIABLE");
    assert!(r.stdout.contains("structure pass: counting:") && r.stdout.contains("refuted"), "{}", r.stdout);
    // with no room of its own the pass derives nothing, and nothing is lost
    let out = solver_command().arg("-n").arg("--structcountticks=0").arg(&cnf).arg(&proof).output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("s UNSATISFIABLE") && !stdout.contains("counting:"), "{}", stdout);
}

#[test]
fn counting_pass_refutes_pigeonhole_by_cutting_planes() {
    let fx = Fixture::new("structure_count");
    let cnf = fx.path("php8.cnf");
    php(8, &cnf);
    let proof = fx.path("c.drat");
    let stdout = run_options_with_proof(&cnf, &proof, &["--structcount=1"]);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    assert!(stdout.contains("structure pass: counting:") && stdout.contains("refuted"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    // the proof is cutting planes over the input ids: no redundance step
    let pbp = std::fs::read_to_string(fx.path("c.drat.pbp")).unwrap();
    assert!(pbp.contains("pol ") && !pbp.contains("red "), "{}", pbp);
}

/// An n x n board with pairwise at-most-one clauses on rows and columns
/// and a counter decision diagram asserting exactly k rooks, written as
/// the rook cells write it (one-way ITE nodes, constants simplified, a
/// node's subsumed clauses dropped; `tools/dd_toy_proof.py` is the
/// reference). The level order is the rows from the last, shuffled when
/// `seed` is not zero.
fn rook_board(n: usize, k: usize, seed: u64, path: &Path) {
    type Child = Option<Option<i64>>; // None: false; Some(None): true; Some(Some(l)): a literal
    let v = |r: usize, c: usize| (r * n + c + 1) as i64;
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for r in 0..n {
        for c1 in 0..n {
            for c2 in c1 + 1..n {
                clauses.push(vec![-v(r, c1), -v(r, c2)]);
            }
        }
    }
    for c in 0..n {
        for r1 in 0..n {
            for r2 in r1 + 1..n {
                clauses.push(vec![-v(r1, c), -v(r2, c)]);
            }
        }
    }
    let mut levels: Vec<i64> = (0..n).rev().flat_map(|r| (0..n).map(move |c| v(r, c))).collect();
    if seed != 0 {
        let mut s = seed;
        for i in (1..levels.len()).rev() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = ((s >> 33) as usize) % (i + 1);
            levels.swap(i, j);
        }
    }
    fn node(i: usize, c: usize, levels: &[i64], next: &mut i64, memo: &mut std::collections::HashMap<(usize, usize), Child>, clauses: &mut Vec<Vec<i64>>) -> Child {
        let rem = levels.len() - i;
        if c > rem {
            return None;
        }
        if rem == 0 {
            return Some(None);
        }
        if let Some(&r) = memo.get(&(i, c)) {
            return r;
        }
        let x = levels[i];
        let r = if rem == 1 {
            Some(Some(if c == 1 { x } else { -x }))
        } else {
            let hi = if c == 0 { None } else { node(i + 1, c - 1, levels, next, memo, clauses) };
            let lo = node(i + 1, c, levels, next, memo, clauses);
            *next += 1;
            let nn = *next;
            let mut cls: Vec<Vec<i64>> = Vec::new();
            for (a, b) in [(Some(Some(-x)), hi), (Some(Some(x)), lo), (hi, lo)] {
                let mut cl = vec![-nn];
                let mut satisfied = false;
                for t in [a, b] {
                    match t {
                        Some(None) => satisfied = true,
                        None => {}
                        Some(Some(l)) => cl.push(l),
                    }
                }
                if satisfied || cl.iter().any(|&l| cl.contains(&-l)) {
                    continue;
                }
                cls.push(cl);
            }
            for cl in &cls {
                if !cls.iter().any(|o| o.len() < cl.len() && o.iter().all(|l| cl.contains(l))) {
                    clauses.push(cl.clone());
                }
            }
            Some(Some(nn))
        };
        memo.insert((i, c), r);
        r
    }
    let mut next = (n * n) as i64;
    let mut memo = std::collections::HashMap::new();
    let root = node(0, k, &levels, &mut next, &mut memo, &mut clauses).flatten().expect("a root node");
    assert!(root > (n * n) as i64);
    clauses.insert(0, vec![root]);
    let mut text = format!("p cnf {} {}\n", next, clauses.len());
    for c in &clauses {
        for l in c {
            text.push_str(&format!("{} ", l));
        }
        text.push_str("0\n");
    }
    std::fs::write(path, text).unwrap();
}

#[test]
fn decision_diagram_count_refutes_a_rook_board_with_a_checked_proof() {
    let fx = Fixture::new("structure_dd");
    let cnf = fx.path("rook6.cnf");
    rook_board(6, 7, 3, &cnf);
    let proof = fx.path("rook6.drat");
    // the pass alone on the stock path
    let stdout = run_options_with_proof(&cnf, &proof, &["--structdd=1"]);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    assert!(stdout.contains("structure pass: decision-diagram count:") && stdout.contains("the root needs 7, refuted"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    let pbp = std::fs::read_to_string(fx.path("rook6.drat.pbp")).unwrap();
    assert!(pbp.contains("pbc ") && pbp.contains("red ") && pbp.contains("conclusion UNSAT"), "{}", pbp);
    // the same answer without a proof file
    let r = run(&cnf, &["--structdd=1"], &[]);
    assert_eq!(r.status, "UNSATISFIABLE");
    assert!(r.stdout.contains("decision-diagram count:") && r.stdout.contains("refuted"), "{}", r.stdout);
    // with the shipped passes on as well, which run first
    let proof2 = fx.path("rook6b.drat");
    let out = solver_command().arg("-n").arg("--structdd=1").arg(&cnf).arg(&proof2).output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof2), "ok");
    // a proof that asks one node for one more true than its subtree
    // forces fails the gate
    let mut lines: Vec<String> = pbp.lines().map(|l| l.to_string()).collect();
    let idx = lines.iter().position(|l| l.starts_with("pbc ")).unwrap();
    let mut parts: Vec<String> = lines[idx].split(' ').map(|t| t.to_string()).collect();
    let ge = parts.iter().position(|t| t == ">=").unwrap();
    parts[ge + 1] = (parts[ge + 1].parse::<i64>().unwrap() + 1).to_string();
    lines[idx] = parts.join(" ");
    std::fs::write(fx.path("rook6.drat.pbp"), lines.join("\n") + "\n").unwrap();
    assert_ne!(gate(&cnf, &proof), "ok");
}

#[test]
fn decision_diagram_count_leaves_a_satisfiable_board_and_stops_without_room() {
    let fx = Fixture::new("structure_dd_sat");
    // exactly n rooks on an n x n board is satisfiable: the diagram is read
    // and nothing follows, and the model satisfies every input clause
    let cnf = fx.path("rook5sat.cnf");
    rook_board(5, 5, 1, &cnf);
    let out = solver_command().args(common::STOCK_PATH).arg("--structdd=1").arg(&cnf).output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("s SATISFIABLE"), "{}", stdout);
    assert!(stdout.contains("decision-diagram count: nothing refuted") && !stdout.contains(", refuted"), "{}", stdout);
    let model: Vec<i64> = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("v "))
        .flat_map(|l| l.split_whitespace().map(|t| t.parse::<i64>().unwrap()))
        .filter(|&l| l != 0)
        .collect();
    let text = std::fs::read_to_string(&cnf).unwrap();
    for line in text.lines().skip(1) {
        let c: Vec<i64> = line.split_whitespace().map(|t| t.parse::<i64>().unwrap()).filter(|&l| l != 0).collect();
        assert!(c.iter().any(|l| model.contains(l)), "clause {:?} unsatisfied", c);
    }
    // the unsatisfiable board with no room of its own: the pass derives
    // nothing and the search answers; off, the pass leaves no trace
    let cnf = fx.path("rook5.cnf");
    rook_board(5, 6, 2, &cnf);
    let r = run(&cnf, &["--structdd=1", "--structddticks=0"], &[]);
    assert_eq!(r.status, "UNSATISFIABLE", "{}", r.stdout);
    assert!(!r.stdout.contains(", refuted"), "{}", r.stdout);
    let r = run(&cnf, &[], &[]);
    assert_eq!(r.status, "UNSATISFIABLE", "{}", r.stdout);
    assert!(!r.stdout.contains("decision-diagram"), "{}", r.stdout);
}
