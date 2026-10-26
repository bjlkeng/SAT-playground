//! The sorter pass (bead SAT-playground-skz.22): a miter of two sorting
//! programs over the same words is refuted with a proof the gate accepts, a
//! tampered proof is rejected, a broken sorter is not refuted, and on a
//! formula without the structure the pass derives nothing and the search is
//! the stock one. The toys are written in the 2025 cells' gate shapes, as
//! `tools/sorter_proof.py` writes them (the comparator as an OR chain of
//! "a_i and not b_i and the higher bits equal", exchanges as multiplexer
//! pairs, pancake and selection sort with a binary index register).

mod common;

use common::{php, run, solver_bin, Fixture};
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

/// The stock path plus the sorter pass, with a proof file.
fn run_sort_with_proof(cnf: &Path, proof: &Path, env: &[(&str, &str)]) -> (String, String) {
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .arg("-n")
        .args(common::STOCK_PATH)
        .arg("--structsort=1")
        .arg(cnf)
        .arg(proof)
        .output()
        .expect("run sat-solver");
    (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

// ---------------------------------------------------------------------------
// the toy generator (tools/sorter_proof.py's, gate for gate)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum S {
    Const(bool),
    Lit(i64),
}

struct Gen {
    n: i64,
    clauses: Vec<Vec<i64>>,
    /// The negative control: skip this exchange call (0 is the first).
    drop_exchange: Option<usize>,
    exchanges: usize,
    /// A variable fixed false by a unit that every comparison ORs in with
    /// its low bit (the same function; the real cells' index logic reads
    /// constants the same way).
    zero: Option<i64>,
}

impl Gen {
    fn new() -> Gen {
        Gen { n: 0, clauses: Vec::new(), drop_exchange: None, exchanges: 0, zero: None }
    }
    fn var(&mut self) -> i64 {
        self.n += 1;
        self.n
    }
    fn add(&mut self, c: &[i64]) {
        self.clauses.push(c.to_vec());
    }
    fn neg(s: S) -> S {
        match s {
            S::Const(b) => S::Const(!b),
            S::Lit(l) => S::Lit(-l),
        }
    }
    fn lit(s: S) -> i64 {
        match s {
            S::Lit(l) => l,
            S::Const(_) => panic!("a constant where a literal is needed"),
        }
    }
    fn and(&mut self, lits: &[S]) -> S {
        let mut out: Vec<i64> = Vec::new();
        for &s in lits {
            match s {
                S::Const(false) => return S::Const(false),
                S::Const(true) => {}
                S::Lit(l) => {
                    if !out.contains(&l) {
                        out.push(l)
                    }
                }
            }
        }
        if out.iter().any(|l| out.contains(&-l)) {
            return S::Const(false);
        }
        if out.is_empty() {
            return S::Const(true);
        }
        if out.len() == 1 {
            return S::Lit(out[0]);
        }
        let o = self.var();
        for &l in &out {
            self.add(&[-o, l]);
        }
        let mut c = vec![o];
        c.extend(out.iter().map(|l| -l));
        self.add(&c);
        S::Lit(o)
    }
    fn or(&mut self, lits: &[S]) -> S {
        let mut out: Vec<i64> = Vec::new();
        for &s in lits {
            match s {
                S::Const(true) => return S::Const(true),
                S::Const(false) => {}
                S::Lit(l) => {
                    if !out.contains(&l) {
                        out.push(l)
                    }
                }
            }
        }
        if out.iter().any(|l| out.contains(&-l)) {
            return S::Const(true);
        }
        if out.is_empty() {
            return S::Const(false);
        }
        if out.len() == 1 {
            return S::Lit(out[0]);
        }
        let o = self.var();
        for &l in &out {
            self.add(&[o, -l]);
        }
        let mut c = vec![-o];
        c.extend(out.iter().copied());
        self.add(&c);
        S::Lit(o)
    }
    /// An inverter gate (the real cells write one where a select is negated).
    fn not(&mut self, s: S) -> S {
        match s {
            S::Const(b) => S::Const(!b),
            S::Lit(l) => {
                let o = self.var();
                self.add(&[o, l]);
                self.add(&[-o, -l]);
                S::Lit(o)
            }
        }
    }
    fn xor(&mut self, a: i64, b: i64) -> i64 {
        let o = self.var();
        self.add(&[-o, a, b]);
        self.add(&[-o, -a, -b]);
        self.add(&[o, -a, b]);
        self.add(&[o, a, -b]);
        o
    }
    fn ite(&mut self, s: i64, t: i64, e: i64) -> i64 {
        assert!(s > 0 && t > 0 && e > 0);
        let o = self.var();
        self.add(&[-s, -t, o]);
        self.add(&[-s, t, -o]);
        self.add(&[s, -e, o]);
        self.add(&[s, e, -o]);
        o
    }
    /// a > b for LSB-first words: the top bit a & ~b, each lower bit
    /// a_i & ~b_i & (a_top == b_top) chained through u & (a_j == b_j), all
    /// of it ORed in a chain.
    fn gt(&mut self, a: &[i64], b: &[i64]) -> S {
        let w = a.len();
        let top = w - 1;
        let t_top = self.var();
        self.add(&[-t_top, a[top]]);
        self.add(&[-t_top, -b[top]]);
        self.add(&[t_top, -a[top], b[top]]);
        let mut acc = S::Lit(t_top);
        for i in (0..w.saturating_sub(1)).rev() {
            let mut u = self.var();
            self.add(&[-u, a[i]]);
            self.add(&[-u, -b[i]]);
            self.add(&[-u, -a[top], b[top]]);
            self.add(&[-u, a[top], -b[top]]);
            self.add(&[u, -a[i], b[i], a[top], b[top]]);
            self.add(&[u, -a[i], b[i], -a[top], -b[top]]);
            for j in ((i + 1)..top).rev() {
                let v = self.var();
                self.add(&[-v, u]);
                self.add(&[-v, -a[j], b[j]]);
                self.add(&[-v, a[j], -b[j]]);
                self.add(&[v, -u, a[j], b[j]]);
                self.add(&[v, -u, -a[j], -b[j]]);
                u = v;
            }
            acc = self.or(&[acc, S::Lit(u)]);
        }
        if let Some(z) = self.zero {
            let t = self.and(&[S::Lit(z), S::Lit(a[0])]);
            acc = self.or(&[acc, t]);
        }
        acc
    }
    fn exchange(&mut self, s: S, x: &[i64], y: &[i64]) -> (Vec<i64>, Vec<i64>) {
        self.exchanges += 1;
        if self.drop_exchange == Some(self.exchanges - 1) {
            return (x.to_vec(), y.to_vec());
        }
        let s = Gen::lit(s);
        let c: Vec<i64> = (0..x.len()).map(|i| self.ite(s, y[i], x[i])).collect();
        let d: Vec<i64> = (0..x.len()).map(|i| self.ite(s, x[i], y[i])).collect();
        (c, d)
    }
}

fn index_bits(n: usize) -> usize {
    (usize::BITS - (n - 1).leading_zeros()).max(1) as usize
}

fn eq_const(g: &mut Gen, m: &[S], c: usize) -> S {
    let lits: Vec<S> = (0..m.len()).map(|b| if (c >> b) & 1 == 1 { m[b] } else { Gen::neg(m[b]) }).collect();
    g.and(&lits)
}

/// m := cond ? j : m, bit by bit.
fn set_if(g: &mut Gen, m: &[S], cond: S, j: usize) -> Vec<S> {
    (0..m.len()).map(|b| if (j >> b) & 1 == 1 { g.or(&[cond, m[b]]) } else { g.and(&[Gen::neg(cond), m[b]]) }).collect()
}

fn bubble(g: &mut Gen, lanes: &[Vec<i64>]) -> Vec<Vec<i64>> {
    let mut lanes = lanes.to_vec();
    let n = lanes.len();
    for p in 0..n - 1 {
        for j in 0..n - 1 - p {
            let s = g.gt(&lanes[j], &lanes[j + 1]);
            let (c, d) = g.exchange(s, &lanes[j], &lanes[j + 1]);
            lanes[j] = c;
            lanes[j + 1] = d;
        }
    }
    lanes
}

/// The first maximum's index in a binary register, then flip(0..m) and
/// flip(0..size-1) merged as in the real cells.
fn pancake(g: &mut Gen, lanes: &[Vec<i64>]) -> Vec<Vec<i64>> {
    let mut lanes = lanes.to_vec();
    let n = lanes.len();
    let bits = index_bits(n);
    for size in (2..=n).rev() {
        let mut m = vec![S::Const(false); bits];
        for j in 1..size {
            let mut terms = Vec::new();
            for i in 0..j {
                let e = eq_const(g, &m, i);
                let c = g.gt(&lanes[j].clone(), &lanes[i].clone());
                terms.push(g.and(&[e, c]));
            }
            let cond = g.or(&terms);
            m = set_if(g, &m, cond, j);
        }
        for c in 1..size - 1 {
            let sel = eq_const(g, &m, c);
            for p in 0..(c + 1) / 2 {
                let (x, y) = g.exchange(sel, &lanes[p].clone(), &lanes[c - p].clone());
                lanes[p] = x;
                lanes[c - p] = y;
            }
        }
        let e = eq_const(g, &m, size - 1);
        let sel = g.not(e);
        for p in 0..size / 2 {
            let q = size - 1 - p;
            let (x, y) = g.exchange(sel, &lanes[p].clone(), &lanes[q].clone());
            lanes[p] = x;
            lanes[q] = y;
        }
    }
    lanes
}

/// The first minimum of the rest, then one conditional swap per candidate.
fn selection(g: &mut Gen, lanes: &[Vec<i64>]) -> Vec<Vec<i64>> {
    let mut lanes = lanes.to_vec();
    let n = lanes.len();
    let bits = index_bits(n);
    for i in 0..n - 1 {
        let mut m: Vec<S> = (0..bits).map(|b| S::Const((i >> b) & 1 == 1)).collect();
        for j in i + 1..n {
            let mut terms = Vec::new();
            for l in i..j {
                let e = eq_const(g, &m, l);
                let c = g.gt(&lanes[l].clone(), &lanes[j].clone());
                terms.push(g.and(&[e, c]));
            }
            let cond = g.or(&terms);
            m = set_if(g, &m, cond, j);
        }
        for j in i + 1..n {
            let sel = eq_const(g, &m, j);
            let (x, y) = g.exchange(sel, &lanes[i].clone(), &lanes[j].clone());
            lanes[i] = x;
            lanes[j] = y;
        }
    }
    lanes
}

fn sorter(name: &str) -> fn(&mut Gen, &[Vec<i64>]) -> Vec<Vec<i64>> {
    match name {
        "bubble" => bubble,
        "pancake" => pancake,
        "selection" => selection,
        _ => panic!("no sorter {}", name),
    }
}

/// A sorter miter: side A and side B over the same N words of W bits, every
/// output bit compared. `bug`: side A skips its first exchange, so it does
/// not sort and the miter is satisfiable. Returns the clauses.
fn toy(a: &str, b: &str, n: usize, w: usize, bug: bool) -> (i64, Vec<Vec<i64>>) {
    toy_with(a, b, n, w, bug, false)
}

/// As `toy`; `zero`: the comparisons read a constant (its unit is the last
/// clause).
fn toy_with(a: &str, b: &str, n: usize, w: usize, bug: bool, zero: bool) -> (i64, Vec<Vec<i64>>) {
    let mut g = Gen::new();
    let words: Vec<Vec<i64>> = (0..n).map(|_| (0..w).map(|_| g.var()).collect()).collect();
    if zero {
        g.zero = Some(g.var());
    }
    g.drop_exchange = if bug { Some(0) } else { None };
    let sa = sorter(a)(&mut g, &words);
    g.drop_exchange = None;
    let sb = sorter(b)(&mut g, &words);
    let mut diffs = Vec::new();
    for k in 0..n {
        let xs: Vec<S> = (0..w).map(|i| S::Lit(g.xor(sa[k][i], sb[k][i]))).collect();
        diffs.push(g.or(&xs));
    }
    let root = g.or(&diffs);
    g.add(&[Gen::lit(root)]);
    if let Some(z) = g.zero {
        g.add(&[-z]);
    }
    (g.n, g.clauses)
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

// ---------------------------------------------------------------------------
// the tests
// ---------------------------------------------------------------------------

const SHAPES: &[(&str, &str, usize, usize)] = &[
    ("bubble", "pancake", 4, 3),
    ("pancake", "selection", 4, 3),
    ("bubble", "selection", 5, 3),
    ("selection", "bubble", 4, 4),
    ("pancake", "bubble", 5, 2),
    ("selection", "pancake", 3, 2),
    ("bubble", "bubble", 3, 2),
];

#[test]
fn sorter_pass_refutes_toy_sorter_miters_with_checked_proofs() {
    let fx = Fixture::new("structsort_toys");
    for &(a, b, n, w) in SHAPES {
        let name = format!("{}_{}_{}x{}", a, b, n, w);
        let cnf = fx.path(&format!("{}.cnf", name));
        let (vars, clauses) = toy(a, b, n, w, false);
        write_cnf(&cnf, vars, &clauses);
        let proof = fx.path(&format!("{}.drat", name));
        let (stdout, stderr) = run_sort_with_proof(&cnf, &proof, &[("SAT_STRUCT_DEBUG", "1")]);
        assert!(stdout.contains("s UNSATISFIABLE"), "{}: {}\n{}", name, stdout, stderr);
        assert!(stdout.contains("sorter: two swap networks"), "{}: the pass did not fire: {}\n{}", name, stdout, stderr);
        assert_eq!(gate(&cnf, &proof), "ok", "{}: gate", name);
    }
}

#[test]
fn sorter_pass_refutes_without_a_proof_file() {
    let fx = Fixture::new("structsort_noproof");
    let cnf = fx.path("m.cnf");
    let (vars, clauses) = toy("pancake", "selection", 5, 3, false);
    write_cnf(&cnf, vars, &clauses);
    let r = run(&cnf, &["--structsort=1"], &[]);
    assert_eq!(r.status, "UNSATISFIABLE", "{}", r.stdout);
    assert!(r.stdout.contains("sorter: two swap networks"), "{}", r.stdout);
    // the search does not start: the pass refutes before it
    assert_eq!(r.stats.get("conflicts").copied().unwrap_or(0), 0, "{}", r.stdout);
}

#[test]
fn a_tampered_sorter_proof_fails_the_gate() {
    let fx = Fixture::new("structsort_tamper");
    let cnf = fx.path("m.cnf");
    let (vars, clauses) = toy("bubble", "pancake", 4, 3, false);
    write_cnf(&cnf, vars, &clauses);
    let proof = fx.path("m.drat");
    let (stdout, _) = run_sort_with_proof(&cnf, &proof, &[]);
    assert!(stdout.contains("sorter: two swap networks"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    // claim one swap unit's count inequality one stronger than it is
    let pbp = fx.path("m.drat.pbp");
    let text = std::fs::read_to_string(&pbp).unwrap();
    let tampered = text.replacen(">= 2 : subproof", ">= 3 : subproof", 1);
    assert_ne!(tampered, text, "the proof has a count inequality to tamper with");
    std::fs::write(&pbp, tampered).unwrap();
    let verdict = gate(&cnf, &proof);
    assert!(verdict.starts_with("FAIL"), "{}", verdict);
}

#[test]
fn a_broken_sorter_is_not_refuted() {
    // side A skips its first exchange: it does not sort, the miter is
    // satisfiable, and the pass must derive nothing, with or without a proof
    let fx = Fixture::new("structsort_bug");
    for &(a, b, n, w) in &[("bubble", "pancake", 4usize, 3usize), ("selection", "pancake", 5, 3)] {
        let cnf = fx.path(&format!("{}_{}.cnf", a, b));
        let (vars, clauses) = toy(a, b, n, w, true);
        write_cnf(&cnf, vars, &clauses);
        let r = run(&cnf, &["--structsort=1"], &[("SAT_STRUCT_DEBUG", "1")]);
        assert_eq!(r.status, "SATISFIABLE", "{}: {}\n{}", a, r.stdout, r.stderr);
        assert!(!r.stdout.contains("sorter: two swap networks"), "{}: {}", a, r.stdout);
        let proof = fx.path(&format!("{}_{}.drat", a, b));
        let (stdout, _) = run_sort_with_proof(&cnf, &proof, &[]);
        assert!(stdout.contains("s SATISFIABLE"), "{}: {}", a, stdout);
    }
}

#[test]
fn repeated_literals_keep_a_checked_proof() {
    // the same formula with repeated literals: the first 40 three-literal
    // clauses (the first comparison's and the first exchange's
    // multiplexers) written with every literal twice, and the unit of a
    // constant the comparisons read written twice. The proof must not add a
    // clause with a repeated literal by id where it assumes the
    // deduplicated form, and the constant must reach the sortedness lemmas
    // (an earlier reading dropped a unit it did not count as one)
    let fx = Fixture::new("structsort_dup");
    let (vars, clauses) = toy_with("bubble", "pancake", 4, 3, false, true);
    let mut doubled = 0;
    let mut out: Vec<Vec<i64>> = Vec::new();
    for c in &clauses {
        let mut d = c.clone();
        if (c.len() == 3 && doubled < 40) || c.len() == 1 && c[0] < 0 {
            d.extend_from_slice(c);
            doubled += (c.len() == 3) as usize;
        }
        out.push(d);
    }
    for (name, cls) in [("plain", &clauses), ("doubled", &out)] {
        let cnf = fx.path(&format!("{}.cnf", name));
        write_cnf(&cnf, vars, cls);
        let proof = fx.path(&format!("{}.drat", name));
        let (stdout, stderr) = run_sort_with_proof(&cnf, &proof, &[("SAT_STRUCT_DEBUG", "1")]);
        assert!(stdout.contains("s UNSATISFIABLE"), "{}: {}\n{}", name, stdout, stderr);
        assert!(stdout.contains("sorter: two swap networks"), "{}: {}\n{}", name, stdout, stderr);
        assert_eq!(gate(&cnf, &proof), "ok", "{}", name);
    }
}

#[test]
fn sorter_pass_without_a_sorter_leaves_the_search_unchanged() {
    // a pigeonhole formula has no swap network: nothing is derived, the
    // search is the stock one, and the work clock grows by the pass's work
    let fx = Fixture::new("structsort_decline");
    let cnf = fx.path("php.cnf");
    php(6, &cnf);
    let stock = run(&cnf, &[], &[]);
    let with = run(&cnf, &["--structsort=1"], &[]);
    assert_eq!(stock.status, with.status);
    assert!(!with.stdout.contains("sorter: two swap networks"), "{}", with.stdout);
    assert!(with.stdout.contains("sorter: no sorter miter, work"), "{}", with.stdout);
    for key in ["conflicts", "decisions", "propagations"] {
        assert_eq!(stock.stats.get(key), with.stats.get(key), "{} differs", key);
    }
    assert!(with.work() >= stock.work(), "the pass's work is in the work clock");
}

#[test]
fn the_sorter_pass_stays_within_the_tick_limit() {
    // Codex review round 1: a toy with 30,000 copies of an unused and
    // gate's clauses went to work 1,309,800 under a limit of 450,000; a
    // group is now read only when the limit covers it, and the gate table
    // polls by work done
    let fx = Fixture::new("structsort_limit");
    let (vars, mut clauses) = toy("bubble", "pancake", 4, 3, false);
    let g = vars + 1;
    for _ in 0..30_000 {
        clauses.push(vec![-g, 1]);
        clauses.push(vec![-g, 2]);
        clauses.push(vec![g, -1, -2]);
    }
    let cnf = fx.path("m.cnf");
    write_cnf(&cnf, g, &clauses);
    for limit in [450_000u64, 2_000_000] {
        let value = limit.to_string();
        let r = run(&cnf, &["--structsort=1"], &[("SAT_LIMIT_TICKS", &value)]);
        assert!(r.work() <= limit + limit / 10, "limit {}: work {} ({})", limit, r.work(), r.status);
    }
}

#[test]
fn the_reading_charges_its_tables() {
    // Codex review round 1: the per-variable tables of a formula with a
    // large variable number are work, which the clock must show
    let fx = Fixture::new("structsort_tables");
    let small = fx.path("small.cnf");
    write_cnf(&small, 2, &[vec![1, 2], vec![-1, -2], vec![1]]);
    let big = fx.path("big.cnf");
    write_cnf(&big, 2_000_000, &[vec![1, 2_000_000], vec![-1, -2_000_000], vec![1]]);
    let a = run(&small, &["--structsort=1"], &[]);
    let b = run(&big, &["--structsort=1"], &[]);
    assert_eq!(a.status, "SATISFIABLE");
    assert_eq!(b.status, "SATISFIABLE");
    assert!(b.work() >= a.work() + 2 * 2_000_000, "the tables of 2M variables are not in the clock: {} v {}", b.work(), a.work());
}
