//! The structure pass (plan section 11, 2026-10-07; epic SAT-playground-1v2):
//! off means off, the symmetry pass settles a pigeonhole formula with a
//! proof the two-stage gate accepts, and a satisfiable formula keeps a
//! checkable model.

mod common;

use common::{php, run, run_with_proof, solver_bin, Fixture};
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

fn run_options_with_proof(cnf: &Path, proof: &Path, options: &[&str]) -> String {
    let out = solver_command()
        .arg("-n")
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
    let stock = run(&cnf, &[], &[]);
    let again = run(&cnf, &["--structsym=0", "--structparity=0"], &[]);
    common::assert_same_trajectory(&stock, &again, "struct options off");
    let proof = fx.path("off.drat");
    let out = run_with_proof(&cnf, &proof, &[]);
    assert_eq!(out.status, "UNSATISFIABLE");
    assert!(!fx.path("off.drat.pbp").exists(), "no VeriPB proof with every pass off");
    assert!(!fx.path("off.drat.derived.cnf").exists());
    assert!(!out.stdout.contains("structure pass"));
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
