//! The linear arithmetic pass (bead SAT-playground-skz.3): a miter of two
//! multipliers over the same partial products is refuted with a proof the
//! two-stage gate accepts, a tampered proof is rejected, and on a formula
//! without the structure the pass derives nothing and the search is the
//! stock one (only the work clock moves, by the pass's own work).

mod common;

use common::{php, run, solver_bin, Fixture};
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

/// The stock path plus the arithmetic pass, with a proof file.
fn run_arith_with_proof(cnf: &Path, proof: &Path) -> String {
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    let out = cmd
        .arg("-n")
        .args(common::STOCK_PATH)
        .arg("--structarith=1")
        .arg(cnf)
        .arg(proof)
        .output()
        .expect("run sat-solver");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A circuit written as CNF, gate by gate (Tseitin clauses).
struct Circuit {
    vars: i64,
    clauses: Vec<Vec<i64>>,
    /// full adders with an unusual carry encoding
    exotic: bool,
}

impl Circuit {
    fn new() -> Self {
        Circuit { vars: 0, clauses: Vec::new(), exotic: false }
    }
    fn var(&mut self) -> i64 {
        self.vars += 1;
        self.vars
    }
    fn and2(&mut self, a: i64, b: i64) -> i64 {
        let o = self.var();
        self.clauses.extend([vec![-o, a], vec![-o, b], vec![o, -a, -b]]);
        o
    }
    fn or2(&mut self, a: i64, b: i64) -> i64 {
        let o = self.var();
        self.clauses.extend([vec![o, -a], vec![o, -b], vec![-o, a, b]]);
        o
    }
    fn xor2(&mut self, a: i64, b: i64) -> i64 {
        let o = self.var();
        self.clauses.extend([vec![-o, a, b], vec![-o, -a, -b], vec![o, -a, b], vec![o, a, -b]]);
        o
    }
    fn xor3(&mut self, a: i64, b: i64, c: i64) -> i64 {
        let o = self.var();
        for m in 0..8 {
            let (ma, mb, mc) = (m & 1, (m >> 1) & 1, (m >> 2) & 1);
            let par = ma ^ mb ^ mc;
            let lit = |x: i64, v: i32| if v == 0 { x } else { -x };
            self.clauses.push(vec![lit(a, ma), lit(b, mb), lit(c, mc), if par == 1 { o } else { -o }]);
        }
        o
    }
    fn maj3(&mut self, a: i64, b: i64, c: i64) -> i64 {
        let o = self.var();
        self.clauses.extend([vec![-o, a, b], vec![-o, a, c], vec![-o, b, c], vec![o, -a, -b], vec![o, -a, -c], vec![o, -b, -c]]);
        o
    }
    fn full_adder(&mut self, a: i64, b: i64, c: i64, two_input: bool) -> (i64, i64) {
        if self.exotic {
            // the carry through a cone that unit propagation cannot run
            // backwards: (a & b) | ((((a | b) & c) ^ b) ^ b)
            let s = self.xor3(a, b, c);
            let ab = self.and2(a, b);
            let a_or_b = self.or2(a, b);
            let t = self.and2(a_or_b, c);
            let u = self.xor2(t, b);
            let v = self.xor2(u, b);
            return (s, self.or2(ab, v));
        }
        if two_input {
            let t = self.xor2(a, b);
            let s = self.xor2(t, c);
            let (x, y) = (self.and2(a, b), self.and2(t, c));
            (s, self.or2(x, y))
        } else {
            (self.xor3(a, b, c), self.maj3(a, b, c))
        }
    }
    fn half_adder(&mut self, a: i64, b: i64) -> (i64, i64) {
        (self.xor2(a, b), self.and2(a, b))
    }
    fn write(&self, path: &Path) {
        let mut text = format!("p cnf {} {}\n", self.vars, self.clauses.len());
        for c in &self.clauses {
            for l in c {
                text.push_str(&format!("{} ", l));
            }
            text.push_str("0\n");
        }
        std::fs::write(path, text).unwrap();
    }
}

/// Row by row with ripple carries; the output bits by weight.
fn array_multiplier(c: &mut Circuit, pp: &[Vec<i64>], two_input: bool) -> Vec<i64> {
    let n = pp.len();
    let mut acc: Vec<i64> = pp[0].clone();
    let mut out = vec![acc[0]];
    acc.remove(0);
    for row in pp.iter().skip(1) {
        let mut new = Vec::new();
        let mut carry: Option<i64> = None;
        for j in 0..n {
            let mut ins = vec![row[j]];
            if j < acc.len() {
                ins.push(acc[j]);
            }
            if let Some(cy) = carry {
                ins.push(cy);
            }
            let (s, cy) = match ins.len() {
                1 => (ins[0], None),
                2 => {
                    let (s, cy) = c.half_adder(ins[0], ins[1]);
                    (s, Some(cy))
                }
                _ => {
                    let (s, cy) = c.full_adder(ins[0], ins[1], ins[2], two_input);
                    (s, Some(cy))
                }
            };
            new.push(s);
            carry = cy;
        }
        if let Some(cy) = carry {
            new.push(cy);
        }
        out.push(new[0]);
        acc = new[1..].to_vec();
    }
    out.extend(acc);
    out
}

/// Column by column. `plus_one`: both sides of a miter add 2^J, folded into
/// a pair of column J as `x + y + 1 = xnor(x, y) + 2 or(x, y)` (from the
/// front or the back of the column); `reverse_from`: columns from there on
/// are reduced in the other order.
fn column_multiplier(c: &mut Circuit, pp: &[Vec<i64>], two_input: bool, plus_one: Option<(usize, bool)>, reverse_from: usize) -> Vec<i64> {
    column_multiplier_dup(c, pp, two_input, plus_one, reverse_from, None)
}

/// As `column_multiplier`, with `duplicate`: pp[i][j] read twice (the
/// product is then off by 2^(i+j)).
fn column_multiplier_dup(c: &mut Circuit, pp: &[Vec<i64>], two_input: bool, plus_one: Option<(usize, bool)>, reverse_from: usize, duplicate: Option<(usize, usize)>) -> Vec<i64> {
    let n = pp.len();
    let mut cols: Vec<Vec<i64>> = vec![Vec::new(); 2 * n];
    for (i, row) in pp.iter().enumerate() {
        for (j, &p) in row.iter().enumerate() {
            cols[i + j].push(p);
        }
    }
    if let Some((i, j)) = duplicate {
        // at the other end of the column from the original: the two copies
        // are read by different cells (in one cell they would not be an adder)
        cols[i + j].insert(0, pp[i][j]);
    }
    let mut out = Vec::new();
    for k in 0..2 * n {
        if let Some((j, from_back)) = plus_one {
            if j == k && cols[k].len() >= 2 {
                let (x, y) = if from_back { (cols[k].pop().unwrap(), cols[k].pop().unwrap()) } else { (cols[k].remove(0), cols[k].remove(0)) };
                let s = c.xor2(x, y); // xnor(x, y) is the negation of the xor
                let cy = c.or2(x, y);
                cols[k].push(-s);
                cols[k + 1].push(cy);
            }
        }
        while cols[k].len() > 1 {
            let rev = k >= reverse_from;
            let mut take = |cols: &mut Vec<Vec<i64>>| if rev { cols[k].remove(0) } else { cols[k].pop().unwrap() };
            let (s, cy) = if cols[k].len() >= 3 {
                let (x, y, z) = (take(&mut cols), take(&mut cols), take(&mut cols));
                c.full_adder(x, y, z, two_input)
            } else {
                let (x, y) = (take(&mut cols), take(&mut cols));
                c.half_adder(x, y)
            };
            cols[k].push(s);
            if k + 1 < 2 * n {
                cols[k + 1].push(cy);
            }
        }
        out.push(cols[k][0]);
    }
    out
}

enum Miter {
    /// an array multiplier against a column-wise one
    ArrayColumn,
    /// column-wise against column-wise, only side A adding 2^J: the two
    /// products differ, so the miter is satisfiable
    PlusOneOneSide(usize),
    /// column-wise against column-wise, side A reading pp[i][j] twice:
    /// satisfiable
    DuplicateLeaf(usize, usize),
    /// an array multiplier against a column-wise one, both with full adders
    /// whose carry goes through an unusual cone
    Exotic,
    /// both column-wise, both adding 2^J folded into different pairs
    PlusOne(usize),
    /// both column-wise, the same reduction below column K (written twice,
    /// so hashing merges gates, xors included, inside both cones)
    Shared(usize),
}

/// A miter asserting that output bit `bit` of two n x n multipliers differs.
fn miter(kind: Miter, n: usize, bit: usize, two_input: bool, path: &Path) {
    let mut c = Circuit::new();
    let a: Vec<i64> = (0..n).map(|_| c.var()).collect();
    let b: Vec<i64> = (0..n).map(|_| c.var()).collect();
    let pp: Vec<Vec<i64>> = (0..n).map(|i| (0..n).map(|j| c.and2(a[i], b[j])).collect()).collect();
    let (oa, ob) = match kind {
        Miter::ArrayColumn => (array_multiplier(&mut c, &pp, two_input), column_multiplier(&mut c, &pp, two_input, None, 2 * n)),
        Miter::DuplicateLeaf(i, j) => (
            column_multiplier_dup(&mut c, &pp, two_input, None, 2 * n, Some((i, j))),
            column_multiplier(&mut c, &pp, two_input, None, 2 * n),
        ),
        Miter::Exotic => {
            c.exotic = true;
            (array_multiplier(&mut c, &pp, two_input), column_multiplier(&mut c, &pp, two_input, None, 2 * n))
        }
        Miter::PlusOneOneSide(j) => (
            column_multiplier(&mut c, &pp, two_input, Some((j, false)), 2 * n),
            column_multiplier(&mut c, &pp, two_input, None, 2 * n),
        ),
        Miter::PlusOne(j) => (
            column_multiplier(&mut c, &pp, two_input, Some((j, false)), 2 * n),
            column_multiplier(&mut c, &pp, two_input, Some((j, true)), 2 * n),
        ),
        Miter::Shared(k) => (column_multiplier(&mut c, &pp, two_input, None, 2 * n), column_multiplier(&mut c, &pp, two_input, None, k)),
    };
    assert_ne!(oa[bit], ob[bit], "the compared bit is one signal on both sides");
    let x = c.xor2(oa[bit], ob[bit]);
    c.clauses.push(vec![x]);
    c.write(path);
}

#[test]
fn arithmetic_pass_refutes_multiplier_miters_with_checked_proofs() {
    let fx = Fixture::new("structarith_miters");
    let cases: Vec<(&str, Miter, usize, usize, bool)> = vec![
        ("array_column_xor3", Miter::ArrayColumn, 6, 8, false),
        ("array_column_and2", Miter::ArrayColumn, 6, 9, true),
        // a constant one folded into a half adder on both sides
        ("plus_one", Miter::PlusOne(3), 6, 8, false),
        // merged xor gates inside both cones
        ("shared_columns", Miter::Shared(4), 6, 8, false),
        ("shared_columns_and2", Miter::Shared(5), 8, 11, true),
        // the cell proofs must not rely on backward propagation
        ("exotic_carry", Miter::Exotic, 4, 5, false),
        ("exotic_carry_6", Miter::Exotic, 6, 8, false),
    ];
    for (name, kind, n, bit, two_input) in cases {
        let cnf = fx.path(&format!("{}.cnf", name));
        miter(kind, n, bit, two_input, &cnf);
        let proof = fx.path(&format!("{}.drat", name));
        let stdout = run_arith_with_proof(&cnf, &proof);
        assert!(stdout.contains("s UNSATISFIABLE"), "{}: {}", name, stdout);
        assert!(stdout.contains("arithmetic: two adder networks"), "{}: the pass did not fire: {}", name, stdout);
        assert_eq!(gate(&cnf, &proof), "ok", "{}: two-stage gate", name);
    }
}

#[test]
fn a_miter_whose_sides_add_different_constants_stays_satisfiable() {
    // side A computes x*y + 2^3, side B x*y: bit 4 differs for some inputs,
    // so the formula is satisfiable and the pass must derive nothing, with
    // or without a proof file
    let fx = Fixture::new("structarith_constant");
    let cnf = fx.path("m.cnf");
    miter(Miter::PlusOneOneSide(3), 6, 4, false, &cnf);
    let without = run(&cnf, &["--structarith=1"], &[("SAT_STRUCT_DEBUG", "1")]);
    assert_eq!(without.status, "SATISFIABLE", "{}", without.stdout);
    assert!(!without.stdout.contains("arithmetic: two adder networks"), "{}", without.stdout);
    assert!(without.stderr.contains("the two sides sum different leaves"), "{}", without.stderr);
    let proof = fx.path("m.drat");
    let stdout = run_arith_with_proof(&cnf, &proof);
    assert!(stdout.contains("s SATISFIABLE"), "{}", stdout);
}

#[test]
fn a_miter_reading_a_leaf_twice_on_one_side_stays_satisfiable() {
    // side A reads pp[1][1] twice (its product is off by 4): bit 4 differs
    // for some inputs; the pass must see that the two sides sum different
    // leaves, with or without a proof file
    let fx = Fixture::new("structarith_duplicate");
    let cnf = fx.path("m.cnf");
    miter(Miter::DuplicateLeaf(1, 1), 4, 4, false, &cnf);
    let without = run(&cnf, &["--structarith=1"], &[("SAT_STRUCT_DEBUG", "1")]);
    assert_eq!(without.status, "SATISFIABLE", "{}", without.stdout);
    assert!(!without.stdout.contains("arithmetic: two adder networks"), "{}", without.stdout);
    // and it is the leaf comparison that stops it (before the fix, an
    // earlier build answered UNSAT here)
    assert!(without.stderr.contains("the two sides sum different leaves"), "{}", without.stderr);
    let proof = fx.path("m.drat");
    let stdout = run_arith_with_proof(&cnf, &proof);
    assert!(stdout.contains("s SATISFIABLE"), "{}", stdout);
}

#[test]
fn a_tampered_arithmetic_proof_fails_the_gate() {
    let fx = Fixture::new("structarith_tamper");
    let cnf = fx.path("m.cnf");
    miter(Miter::ArrayColumn, 6, 8, false, &cnf);
    let proof = fx.path("m.drat");
    let stdout = run_arith_with_proof(&cnf, &proof);
    assert!(stdout.contains("arithmetic: two adder networks"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    // claim one adder cell's inequality one stronger than it is
    let pbp = fx.path("m.drat.pbp");
    let text = std::fs::read_to_string(&pbp).unwrap();
    let tampered = text.replacen(">= 3 : subproof", ">= 4 : subproof", 1);
    assert_ne!(tampered, text, "the proof has a cell inequality to tamper with");
    std::fs::write(&pbp, tampered).unwrap();
    let verdict = gate(&cnf, &proof);
    assert!(verdict.starts_with("FAIL"), "{}", verdict);
}

#[test]
fn arithmetic_pass_without_a_miter_leaves_the_search_unchanged() {
    // a pigeonhole formula has no adder cells: nothing is derived, the
    // search is the stock one, and the work clock grows by the pass's work
    let fx = Fixture::new("structarith_decline");
    let cnf = fx.path("php.cnf");
    php(6, &cnf);
    let stock = run(&cnf, &[], &[]);
    let with = run(&cnf, &["--structarith=1"], &[]);
    assert_eq!(stock.status, with.status);
    assert!(!with.stdout.contains("arithmetic: two adder networks"), "{}", with.stdout);
    assert!(with.stdout.contains("arithmetic: no adder miter, work"), "{}", with.stdout);
    for key in ["conflicts", "decisions", "propagations"] {
        assert_eq!(stock.stats.get(key), with.stats.get(key), "{} differs", key);
    }
    assert!(with.work() >= stock.work(), "the pass's work is in the work clock");
    // a miter: the pass's work is in the clock and the search is short
    let cnf = fx.path("m.cnf");
    miter(Miter::ArrayColumn, 8, 11, true, &cnf);
    let with = run(&cnf, &["--structarith=1"], &[]);
    assert_eq!(with.status, "UNSATISFIABLE");
    assert!(with.stdout.contains("arithmetic: two adder networks"), "{}", with.stdout);
    assert!(with.work() > 0);
}
