//! The divider and square-root pass (bead SAT-playground-skz.23): a miter of
//! two dividers, or of two square roots, on one output bit is decided with a
//! proof the gate accepts, a tampered proof is rejected, a broken circuit is
//! not refuted, a formula without the structure sees the stock search, and
//! the pass keeps to the tick limit. The toy dividers are
//! `tools/divsqrt_proof.py toy`'s, gate for gate: quotient bit k of an n by n
//! divider, a full-width restoring array with a borrow chain against a
//! narrow array of adders over the low divisor bits (with `shortcut` its
//! rows ignore the high divisor bits and only its output checks them, as in
//! the real cells). Every clause starts with its gate's literal, and the
//! inputs a[k..n-1], b[0..n-1] are numbered in the text order of their names.
//!
//! The pass proves its row pairs in one of two ways (bead SAT-playground-lhf,
//! 2026-11-18): as questions to one solver that is loaded once and keeps what
//! it learned (`--structdivassume=1`, the default), or with a fresh solver
//! for every pair (`--structdivassume=0`). The tests run both (`CHAINS`).

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

/// The two ways the pass proves its pairs: a fresh solver for every pair, and
/// questions to one solver.
const FRESH: &str = "--structdivassume=0";
const ONE: &str = "--structdivassume=1";
const CHAINS: [&str; 2] = [FRESH, ONE];

/// A short name of a chain for file names.
fn tag(chain: &str) -> &'static str {
    if chain == ONE {
        "one"
    } else {
        "fresh"
    }
}

/// The stock path plus the pass with the given chain, with a proof file;
/// (stdout, stderr).
fn run_div_with_proof(cnf: &Path, proof: &Path, chain: &str, env: &[(&str, &str)]) -> (String, String) {
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.arg("-n").args(common::STOCK_PATH).arg("--structdiv=1").arg(chain).arg(cnf).arg(proof).output().expect("run sat-solver");
    (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// "N of M" from the pass's report line.
fn pairs_proven(stdout: &str) -> (usize, usize) {
    let line = stdout.lines().find(|l| l.contains("row pairs proven equal")).unwrap_or_else(|| panic!("no report line of the pass: {}", stdout));
    let words: Vec<&str> = line.split_whitespace().collect();
    let at = words.iter().position(|w| *w == "row").expect("the word 'row'");
    (words[at - 3].parse().expect("pairs proven"), words[at - 1].parse().expect("pairs"))
}

// ---------------------------------------------------------------------------
// the toy generator
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum S {
    C(bool),
    L(i64),
}

fn neg(x: S) -> S {
    match x {
        S::C(b) => S::C(!b),
        S::L(l) => S::L(-l),
    }
}

/// Gates with constant folding; each gate's clauses start with its literal.
struct B {
    n: i64,
    clauses: Vec<Vec<i64>>,
}

impl B {
    fn new() -> B {
        B { n: 0, clauses: Vec::new() }
    }
    fn var(&mut self) -> i64 {
        self.n += 1;
        self.n
    }
    fn and(&mut self, a: S, b: S) -> S {
        match (a, b) {
            (S::C(false), _) | (_, S::C(false)) => S::C(false),
            (S::C(true), x) | (x, S::C(true)) => x,
            (S::L(x), S::L(y)) => {
                if x == y {
                    return S::L(x);
                }
                if x == -y {
                    return S::C(false);
                }
                let g = self.var();
                self.clauses.extend([vec![-g, x], vec![-g, y], vec![g, -x, -y]]);
                S::L(g)
            }
        }
    }
    fn or(&mut self, a: S, b: S) -> S {
        neg(self.and(neg(a), neg(b)))
    }
    fn xor(&mut self, a: S, b: S) -> S {
        match (a, b) {
            (S::C(c), x) | (x, S::C(c)) => {
                if c {
                    neg(x)
                } else {
                    x
                }
            }
            (S::L(x), S::L(y)) => {
                if x == y {
                    return S::C(false);
                }
                if x == -y {
                    return S::C(true);
                }
                let g = self.var();
                self.clauses.extend([vec![-g, x, y], vec![-g, -x, -y], vec![g, -x, y], vec![g, x, -y]]);
                S::L(g)
            }
        }
    }
    fn maj(&mut self, a: S, b: S, c: S) -> S {
        let ab = self.and(a, b);
        let o = self.or(a, b);
        let co = self.and(c, o);
        self.or(ab, co)
    }
    /// s ? t : e
    fn mux(&mut self, s: S, t: S, e: S) -> S {
        let S::L(sl) = s else {
            return if s == S::C(true) { t } else { e };
        };
        if t == e {
            return t;
        }
        match (t, e) {
            (S::L(tl), S::L(el)) => {
                let g = self.var();
                self.clauses.extend([vec![-g, -sl, tl], vec![-g, sl, el], vec![g, -sl, -tl], vec![g, sl, -el]]);
                S::L(g)
            }
            _ => {
                let x = self.and(s, t);
                let y = self.and(neg(s), e);
                self.or(x, y)
            }
        }
    }
    /// The miter: the xor of the two sides asserted.
    fn finish(mut self, p: S, q: S) -> (i64, Vec<Vec<i64>>) {
        let S::L(root) = self.xor(p, q) else { panic!("the sides folded to constants") };
        self.clauses.push(vec![root]);
        (self.n, self.clauses)
    }
}

/// a - b with a borrow chain (d bits, then no-borrow = a >= b).
fn sub_borrow(g: &mut B, a: &[S], b: &[S]) -> (Vec<S>, S) {
    let mut borrow = S::C(false);
    let mut d = Vec::with_capacity(a.len());
    for i in 0..a.len() {
        let bi = if i < b.len() { b[i] } else { S::C(false) };
        let x = g.xor(a[i], bi);
        d.push(g.xor(x, borrow));
        borrow = g.maj(neg(a[i]), bi, borrow);
    }
    (d, neg(borrow))
}

/// a + ~b + 1 with a carry chain (sum bits, then carry out = a >= b).
fn sub_carry(g: &mut B, a: &[S], b: &[S]) -> (Vec<S>, S) {
    let mut carry = S::C(true);
    let mut s = Vec::with_capacity(a.len());
    for i in 0..a.len() {
        let nb = if i < b.len() { neg(b[i]) } else { S::C(true) };
        let x = g.xor(a[i], nb);
        s.push(g.xor(x, carry));
        carry = g.maj(a[i], nb, carry);
    }
    (s, carry)
}

/// Quotient bit k of an n by n divider, two ways. `bug`: the narrow side
/// brings down a 0 instead of the dividend's top bit.
fn toy_divider(n: usize, k: usize, shortcut: bool, bug: bool) -> (i64, Vec<Vec<i64>>) {
    let mut g = B::new();
    let mut names: Vec<(String, bool, usize)> = (k..n).map(|i| (format!("a[{}]", i), true, i)).chain((0..n).map(|i| (format!("b[{}]", i), false, i))).collect();
    names.sort_by(|x, y| x.0.cmp(&y.0));
    let mut a = vec![0i64; n];
    let mut b = vec![0i64; n];
    for (_, is_a, i) in &names {
        let v = g.var();
        if *is_a {
            a[*i] = v;
        } else {
            b[*i] = v;
        }
    }
    let bs: Vec<S> = b.iter().map(|&v| S::L(v)).collect();
    // side 1: full-width rows, T - b with a borrow chain
    let mut r: Vec<S> = vec![S::C(false); n];
    let mut q1 = S::C(false);
    for j in (k..n).rev() {
        let mut t = vec![S::L(a[j])];
        t.extend(r.iter().copied());
        let (d, ge) = sub_borrow(&mut g, &t, &bs);
        q1 = ge;
        r = (0..n).map(|i| g.mux(q1, d[i], t[i])).collect();
    }
    // side 2: rows of width n-k+1 over the low divisor bits, T + ~b + 1
    let w = n - k;
    let mut big = S::C(false);
    for &bv in &bs[w..n] {
        big = g.or(big, bv);
    }
    let mut r: Vec<S> = vec![S::C(false); w];
    let mut q2 = S::C(false);
    for j in (k..n).rev() {
        let top = if bug && j == n - 1 { S::C(false) } else { S::L(a[j]) };
        let mut t = vec![top];
        t.extend(r.iter().copied());
        let (s, carry) = sub_carry(&mut g, &t, &bs[..w]);
        q2 = if shortcut { carry } else { g.and(carry, neg(big)) };
        r = (0..w).map(|i| g.mux(q2, s[i], t[i])).collect();
    }
    if shortcut {
        q2 = g.and(q2, neg(big));
    }
    g.finish(q1, q2)
}

/// The low bit of isqrt(x) for an n-bit x (n even, the inputs bit 0 first),
/// digit by digit, two ways: a borrow chain over steps+3 bits against a carry
/// chain over steps+2. `bug`: the second side's first trial is 4Q + 3.
fn toy_sqrt(n: usize, bug: bool) -> (i64, Vec<Vec<i64>>) {
    let mut g = B::new();
    let x: Vec<S> = (0..n).map(|_| S::L(g.var())).collect();
    let steps = n / 2;
    let mut outs = Vec::new();
    for side in 0..2 {
        let width = steps + 3 - side;
        let mut r: Vec<S> = vec![S::C(false); width];
        let mut q: Vec<S> = Vec::new(); // the root so far, least significant first
        let mut ge = S::C(false);
        for i in (0..steps).rev() {
            // R = 4R + the next two bits; T = 4Q + 1
            let mut rr = vec![x[2 * i], x[2 * i + 1]];
            rr.extend(r[..width - 2].iter().copied());
            let one = if side == 1 && bug && i == steps - 1 { S::C(true) } else { S::C(false) };
            let mut t = vec![S::C(true), one];
            t.extend(q.iter().copied());
            t.truncate(width);
            let (d, ok) = if side == 0 { sub_borrow(&mut g, &rr, &t) } else { sub_carry(&mut g, &rr, &t) };
            ge = ok;
            r = (0..width).map(|j| g.mux(ge, d[j], rr[j])).collect();
            q.insert(0, ge);
        }
        outs.push(ge);
    }
    g.finish(outs[0], outs[1])
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

fn div_work(cnf: &Path, proof: Option<&Path>, limit: Option<u64>, chain: &str) -> (String, u64) {
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    if let Some(l) = limit {
        cmd.env("SAT_LIMIT_TICKS", l.to_string());
    }
    cmd.arg("-n").args(common::STOCK_PATH).arg("--structdiv=1").arg(chain).arg(cnf);
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

// ---------------------------------------------------------------------------
// the tests
// ---------------------------------------------------------------------------

// (the last two have pairs that are not equal: a model drops them, and with
// one solver the chain goes on in the same solver)
const DIVIDERS: &[(usize, usize, bool)] = &[(6, 2, false), (8, 3, false), (8, 3, true), (12, 4, false), (12, 4, true), (12, 3, false), (16, 5, false)];

#[test]
fn div_pass_refutes_toy_dividers_with_checked_proofs() {
    let fx = Fixture::new("structdiv_toys");
    for &(n, k, shortcut) in DIVIDERS {
        let name = format!("div_n{}_k{}_{}", n, k, shortcut);
        let cnf = fx.path(&format!("{}.cnf", name));
        let (vars, clauses) = toy_divider(n, k, shortcut, false);
        write_cnf(&cnf, vars, &clauses);
        let mut proven: Vec<(usize, usize)> = Vec::new();
        for chain in CHAINS {
            let proof = fx.path(&format!("{}.{}.drat", name, tag(chain)));
            let (stdout, stderr) = run_div_with_proof(&cnf, &proof, chain, &[("SAT_STRUCT_DEBUG", "1")]);
            assert!(stdout.contains("s UNSATISFIABLE"), "{} {}: {}\n{}", name, chain, stdout, stderr);
            let fired = format!("div/sqrt: quotient bit {} of a {}/{} divider", k, n, n);
            assert!(stdout.contains(&fired), "{} {}: the pass did not fire: {}\n{}", name, chain, stdout, stderr);
            assert_eq!(stdout.contains("asked of one solver"), chain == ONE, "{} {}: {}", name, chain, stdout);
            assert_eq!(gate(&cnf, &proof), "ok", "{} {}: gate", name, chain);
            proven.push(pairs_proven(&stdout));
        }
        assert_eq!(proven[0], proven[1], "{}: the two chains prove the same pairs", name);
    }
}

#[test]
fn sqrt_pass_refutes_toy_square_roots_with_checked_proofs() {
    let fx = Fixture::new("structdiv_sqrt");
    for n in [8usize, 12, 16] {
        let name = format!("sqrt_n{}", n);
        let cnf = fx.path(&format!("{}.cnf", name));
        let (vars, clauses) = toy_sqrt(n, false);
        write_cnf(&cnf, vars, &clauses);
        let mut proven: Vec<(usize, usize)> = Vec::new();
        for chain in CHAINS {
            let proof = fx.path(&format!("{}.{}.drat", name, tag(chain)));
            let (stdout, stderr) = run_div_with_proof(&cnf, &proof, chain, &[("SAT_STRUCT_DEBUG", "1")]);
            assert!(stdout.contains("s UNSATISFIABLE"), "{} {}: {}\n{}", name, chain, stdout, stderr);
            let fired = format!("div/sqrt: the low root bit of a {}-bit square root", n);
            assert!(stdout.contains(&fired), "{} {}: the pass did not fire: {}\n{}", name, chain, stdout, stderr);
            assert_eq!(gate(&cnf, &proof), "ok", "{} {}: gate", name, chain);
            proven.push(pairs_proven(&stdout));
        }
        assert_eq!(proven[0], proven[1], "{}: the two chains prove the same pairs", name);
    }
}

#[test]
fn div_pass_works_without_a_proof_file() {
    let fx = Fixture::new("structdiv_noproof");
    let cnf = fx.path("m.cnf");
    let (vars, clauses) = toy_divider(12, 4, true, false);
    write_cnf(&cnf, vars, &clauses);
    let mut proven: Vec<(usize, usize)> = Vec::new();
    for chain in CHAINS {
        let r = run(&cnf, &["--structdiv=1", chain], &[]);
        assert_eq!(r.status, "UNSATISFIABLE", "{}: {}", chain, r.stdout);
        assert!(r.stdout.contains("div/sqrt: quotient bit 4 of a 12/12 divider"), "{}: {}", chain, r.stdout);
        proven.push(pairs_proven(&r.stdout));
    }
    assert_eq!(proven[0], proven[1], "the two chains prove the same pairs");
}

#[test]
fn the_one_solver_proof_deletes_what_the_solver_deleted() {
    // One solver keeps its learned clauses from question to question, so its
    // lines stay in the proof; when it deletes one, the proof deletes that
    // step (`del id`), and when the chain ends nothing of the solver is left:
    // every step it added is deleted again. The fresh chain's steps live in
    // blocks and need no deletion.
    let fx = Fixture::new("structdiv_del");
    let cnf = fx.path("m.cnf");
    let (vars, clauses) = toy_divider(16, 5, false, false);
    write_cnf(&cnf, vars, &clauses);
    let proof = fx.path("m.one.drat");
    let (stdout, _) = run_div_with_proof(&cnf, &proof, ONE, &[]);
    assert!(stdout.contains("asked of one solver"), "{}", stdout);
    let (proven, pairs) = pairs_proven(&stdout);
    assert!(proven > 0 && proven < pairs, "some pairs proven and some not: {}", stdout);
    let text = std::fs::read_to_string(fx.path("m.one.drat.pbp")).unwrap();
    assert!(!text.contains("subproof"), "one solver writes no blocks");
    let rups = text.lines().filter(|l| l.starts_with("rup ")).count();
    let deleted: usize = text.lines().filter(|l| l.starts_with("del id ")).map(|l| l.split_whitespace().count() - 3).sum();
    let reds = text.lines().filter(|l| l.starts_with("red ")).count();
    assert_eq!(reds, 4 * pairs, "every pair's d is defined, before the first question");
    // what stays: per proven pair its definition (4), ~d and its two clauses
    assert_eq!(rups + reds - deleted, 7 * proven, "{} rup, {} red, {} deleted, {} proven", rups, reds, deleted, proven);
    // the solver deletes clauses while the chain runs, and the proof follows
    // then, not only when the chain ends
    let first_del = text.lines().position(|l| l.starts_with("del id ")).expect("a deletion");
    let last_rup = text.lines().collect::<Vec<_>>().iter().rposition(|l| l.starts_with("rup ")).unwrap();
    assert!(first_del < last_rup, "a deletion at line {} before the last step at line {}", first_del, last_rup);
    assert_eq!(gate(&cnf, &proof), "ok");
    let fresh = fx.path("m.fresh.drat");
    let (stdout, _) = run_div_with_proof(&cnf, &fresh, FRESH, &[]);
    assert!(!stdout.contains("asked of one solver"), "{}", stdout);
    let text = std::fs::read_to_string(fx.path("m.fresh.drat.pbp")).unwrap();
    assert!(text.contains("subproof") && !text.contains("del id"), "the fresh chain writes blocks and no deletion");
}

#[test]
fn a_circuit_with_no_model_without_its_root_is_refuted_by_the_one_solver() {
    // Clauses that contradict each other without the miter's root: on two
    // constants (found while the circuit is loaded), and on two of the
    // circuit's inputs (found by the first question's search). The fresh
    // chain proves every pair, vacuously. The one solver ends up with the
    // empty clause, which holds for the formula too: the pass refutes it,
    // and the proof ends in a refutation the gate accepts.
    let fx = Fixture::new("structdiv_nomodel");
    let (vars, clauses) = toy_divider(8, 3, false, false);
    let (u, v) = (vars + 1, vars + 2);
    let mut constants = clauses.clone();
    constants.extend([vec![u], vec![v], vec![-u, -v]]);
    let mut inputs = clauses.clone();
    inputs.extend([vec![1, 2], vec![1, -2], vec![-1, 2], vec![-1, -2]]);
    for (name, nv, cls) in [("constants", vars + 2, constants), ("inputs", vars, inputs)] {
        let cnf = fx.path(&format!("{}.cnf", name));
        write_cnf(&cnf, nv, &cls);
        for chain in CHAINS {
            let proof = fx.path(&format!("{}.{}.drat", name, tag(chain)));
            let (stdout, stderr) = run_div_with_proof(&cnf, &proof, chain, &[("SAT_STRUCT_DEBUG", "1")]);
            assert!(stdout.contains("s UNSATISFIABLE"), "{} {}: {}\n{}", name, chain, stdout, stderr);
            assert!(stdout.contains("div/sqrt: quotient bit 3 of a 8/8 divider"), "{} {}: {}\n{}", name, chain, stdout, stderr);
            assert_eq!(stdout.contains("the circuit without its root is unsatisfiable"), chain == ONE, "{} {}: {}", name, chain, stdout);
            assert_eq!(gate(&cnf, &proof), "ok", "{} {}", name, chain);
            let r = run(&cnf, &["--structdiv=1", chain], &[]);
            assert_eq!(r.status, "UNSATISFIABLE", "{} {} without a proof: {}", name, chain, r.stdout);
        }
    }
}

#[test]
fn a_tampered_div_proof_fails_the_gate() {
    let fx = Fixture::new("structdiv_tamper");
    let cnf = fx.path("m.cnf");
    let (vars, clauses) = toy_divider(8, 3, false, false);
    write_cnf(&cnf, vars, &clauses);
    let proof = fx.path("m.drat");
    let (stdout, _) = run_div_with_proof(&cnf, &proof, FRESH, &[]);
    assert!(stdout.contains("div/sqrt: quotient bit 3"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    // claim one pair's ~d one stronger than it is: its pbc goal is then
    // infeasible and the sub-call's steps do not refute its negation
    let pbp = fx.path("m.drat.pbp");
    let text = std::fs::read_to_string(&pbp).unwrap();
    let tampered = text.replacen(">= 1 : subproof", ">= 2 : subproof", 1);
    assert_ne!(tampered, text, "the proof has a pbc block to tamper with");
    std::fs::write(&pbp, tampered).unwrap();
    let verdict = gate(&cnf, &proof);
    assert!(verdict.starts_with("FAIL"), "{}", verdict);

    // one solver: three broken proofs, each rejected
    let proof = fx.path("one.drat");
    let (stdout, _) = run_div_with_proof(&cnf, &proof, ONE, &[]);
    assert!(stdout.contains("asked of one solver"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    let pbp = fx.path("one.drat.pbp");
    let text = std::fs::read_to_string(&pbp).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    // a d is a variable past the formula's: "rup 1 ~xD >= 1 ;" says the pair
    // of D is equal
    let unit_of_d = |l: &str| -> Option<i64> {
        let rest = l.strip_prefix("rup 1 ~x")?.strip_suffix(" >= 1 ;")?;
        rest.parse::<i64>().ok().filter(|d| *d > vars)
    };
    let at = lines.iter().position(|l| unit_of_d(l).is_some()).expect("a pair's ~d in the proof");
    let d = unit_of_d(lines[at]).unwrap();
    // 1. the pair is claimed to differ: "d" where the proof has "~d"
    let mut wrong = lines.clone();
    let flipped = format!("rup 1 x{} >= 1 ;", d);
    for l in wrong.iter_mut() {
        if unit_of_d(l) == Some(d) {
            *l = &flipped;
        }
    }
    std::fs::write(&pbp, wrong.join("\n") + "\n").unwrap();
    let verdict = gate(&cnf, &proof);
    assert!(verdict.starts_with("FAIL"), "a pair claimed to differ: {}", verdict);
    // 2. the solver's own steps left out: only the definitions, the ~d lines
    // and the pairs' clauses stay, and they do not follow by themselves
    let first_red = lines.iter().position(|l| l.starts_with("red ")).unwrap();
    let last_red = lines.iter().rposition(|l| l.starts_with("red ")).unwrap();
    let is_pair_line = |i: usize| unit_of_d(lines[i]).is_some() || (i > 0 && unit_of_d(lines[i - 1]).is_some()) || (i > 1 && unit_of_d(lines[i - 2]).is_some());
    let mut bare: Vec<&str> = lines[..=last_red].to_vec();
    assert!(first_red <= last_red);
    for i in last_red + 1..lines.len() {
        let l = lines[i];
        if (l.starts_with("rup ") && !is_pair_line(i)) || l.starts_with("del id ") || l.starts_with("core id ") {
            continue;
        }
        bare.push(l);
    }
    std::fs::write(&pbp, bare.join("\n") + "\n").unwrap();
    let verdict = gate(&cnf, &proof);
    assert!(verdict.starts_with("FAIL"), "the solver's steps left out: {}", verdict);
    // 3. a definition changed: d = x or y is not d = x xor y
    let reds: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].starts_with("red ")).collect();
    let mut weak = lines.clone();
    weak.remove(reds[1]);
    std::fs::write(&pbp, weak.join("\n") + "\n").unwrap();
    let verdict = gate(&cnf, &proof);
    assert!(verdict.starts_with("FAIL"), "a definition clause left out: {}", verdict);
    // and the untouched proof is still accepted
    std::fs::write(&pbp, &text).unwrap();
    assert_eq!(gate(&cnf, &proof), "ok");
}

#[test]
fn a_broken_divider_or_square_root_is_not_refuted() {
    // the narrow side's top row brings down a 0, or the second square root's
    // first trial is off: the miter is satisfiable and the answer must be a
    // model, with or without a proof
    let fx = Fixture::new("structdiv_bug");
    let cases: Vec<(&str, (i64, Vec<Vec<i64>>))> = vec![
        ("div", toy_divider(8, 3, false, true)),
        ("div_shortcut", toy_divider(12, 4, true, true)),
        ("sqrt", toy_sqrt(12, true)),
    ];
    for (name, (vars, clauses)) in cases {
        let cnf = fx.path(&format!("{}.cnf", name));
        write_cnf(&cnf, vars, &clauses);
        for chain in CHAINS {
            let r = run(&cnf, &["--structdiv=1", chain], &[("SAT_STRUCT_DEBUG", "1")]);
            assert_eq!(r.status, "SATISFIABLE", "{} {}: {}\n{}", name, chain, r.stdout, r.stderr);
            let proof = fx.path(&format!("{}.{}.drat", name, tag(chain)));
            let (stdout, _) = run_div_with_proof(&cnf, &proof, chain, &[]);
            assert!(stdout.contains("s SATISFIABLE"), "{} {}: {}", name, chain, stdout);
        }
    }
}

#[test]
fn div_pass_without_a_divider_leaves_the_search_unchanged() {
    // a pigeonhole formula has no circuit to read: nothing is derived, the
    // search is the stock one, and the work clock grows by the pass's work
    let fx = Fixture::new("structdiv_decline");
    let cnf = fx.path("php.cnf");
    php(6, &cnf);
    let stock = run(&cnf, &[], &[]);
    let with = run(&cnf, &["--structdiv=1"], &[]);
    assert_eq!(stock.status, with.status);
    assert!(with.stdout.contains("div/sqrt: no divider or square-root miter, work"), "{}", with.stdout);
    for key in ["conflicts", "decisions", "propagations"] {
        assert_eq!(stock.stats.get(key), with.stats.get(key), "{} differs", key);
    }
    assert!(with.work() >= stock.work(), "the pass's work is in the work clock");
}

#[test]
fn every_step_of_the_div_pass_keeps_to_the_tick_limit() {
    // a divider and a square root, with and without a proof file, each
    // stopped at fractions of its own total: no limit is passed by more
    // than 10%
    let fx = Fixture::new("structdiv_limits");
    let cases: Vec<(&str, (i64, Vec<Vec<i64>>))> = vec![("div", toy_divider(12, 4, false, false)), ("sqrt", toy_sqrt(16, false))];
    for (name, (vars, clauses)) in cases {
        let cnf = fx.path(&format!("{}.cnf", name));
        write_cnf(&cnf, vars, &clauses);
        for chain in CHAINS {
            for with_proof in [false, true] {
                let proof = fx.path(&format!("{}.{}.drat", name, tag(chain)));
                let p = if with_proof { Some(proof.as_path()) } else { None };
                let (status, total) = div_work(&cnf, p, None, chain);
                assert_eq!(status, "UNSATISFIABLE", "{} {} proof {}", name, chain, with_proof);
                for frac in [5u64, 10, 20, 30, 40, 50, 60, 70, 80, 90, 95] {
                    let limit = total * frac / 100;
                    let (st, work) = div_work(&cnf, p, Some(limit), chain);
                    assert!(work <= limit + limit / 10, "{} {} proof {}: limit {} of {}: work {} ({})", name, chain, with_proof, limit, total, work, st);
                    // a proof cut short by the limit is still a proof of what
                    // was added: the gate checks its first stage
                    if with_proof && st == "UNSATISFIABLE" {
                        assert_eq!(gate(&cnf, &proof), "ok", "{} {}: limit {}", name, chain, limit);
                    }
                }
            }
        }
    }
}

/// The pass's own work, from its report line ("... work 517K").
fn pass_work_k(stdout: &str) -> u64 {
    let line = stdout.lines().find(|l| l.contains("div/sqrt: ")).unwrap_or_else(|| panic!("no report line of the pass: {}", stdout));
    let word = line.split_whitespace().last().expect("a last word");
    word.strip_suffix('K').and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("no work on the line: {}", line))
}

#[test]
fn a_chain_cut_short_by_its_own_limit_leaves_a_checked_proof() {
    // The pass's own limit (structdivticks) stops the chain part of the way:
    // the pairs proven so far join the formula, the search does the rest,
    // and the proof passes the gate. With one solver this is the path where
    // the solver's steps and the unused definitions are deleted with pairs
    // still pending.
    let fx = Fixture::new("structdiv_cut");
    let cases: Vec<(&str, (i64, Vec<Vec<i64>>))> = vec![("div", toy_divider(12, 4, false, false)), ("sqrt", toy_sqrt(16, false))];
    for (name, (vars, clauses)) in cases {
        let cnf = fx.path(&format!("{}.cnf", name));
        write_cnf(&cnf, vars, &clauses);
        for chain in CHAINS {
            let proof = fx.path(&format!("{}.{}.drat", name, tag(chain)));
            let (stdout, _) = run_div_with_proof(&cnf, &proof, chain, &[]);
            let (all, _) = pairs_proven(&stdout);
            let total = pass_work_k(&stdout);
            assert!(all >= 4 && total >= 20, "{} {}: {}", name, chain, stdout);
            let mut partial = 0;
            for frac in [15u64, 30, 45, 60, 75, 90] {
                let limit = format!("--structdivticks={}", total * frac / 100);
                let mut cmd = Command::new(solver_bin());
                for key in common::SOLVER_ENV_VARS {
                    cmd.env_remove(key);
                }
                let out = cmd.arg("-n").args(common::STOCK_PATH).arg("--structdiv=1").arg(chain).arg(&limit).arg(&cnf).arg(&proof).output().expect("run sat-solver");
                let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                assert!(stdout.contains("s UNSATISFIABLE"), "{} {} {}: {}", name, chain, limit, stdout);
                if stdout.contains("row pairs proven equal") {
                    let (proven, _) = pairs_proven(&stdout);
                    assert!(proven <= all, "{} {} {}: {}", name, chain, limit, stdout);
                    if proven > 0 && proven < all {
                        partial += 1;
                    }
                }
                assert_eq!(gate(&cnf, &proof), "ok", "{} {} {}", name, chain, limit);
            }
            assert!(partial > 0, "{} {}: no limit stopped the chain part of the way", name, chain);
        }
    }
}

#[test]
fn the_shipped_defaults_run_the_div_pass() {
    // on by default since 2026-10-28 (the owner's call after the paired
    // run): with no option at all a toy divider is decided through the pass,
    // with a proof the gate accepts, and --structdiv=0 turns it off
    let fx = Fixture::new("structdiv_default");
    let cnf = fx.path("m.cnf");
    let (vars, clauses) = toy_divider(12, 4, true, false);
    write_cnf(&cnf, vars, &clauses);
    let proof = fx.path("m.drat");
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    let out = cmd.arg("-n").arg(&cnf).arg(&proof).output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("\ns UNSATISFIABLE"), "{}", stdout);
    assert!(stdout.contains("div/sqrt: quotient bit 4 of a 12/12 divider"), "the pass is on by default: {}", stdout);
    // since 2026-11-18 its pairs are questions to one solver by default
    // (bead SAT-playground-lhf); --structdivassume=0 is the chain as it was
    assert!(stdout.contains("asked of one solver"), "one solver by default: {}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    // (two runs without a proof file: a proof's lines are work too)
    let plain = common::run_shipped(&cnf, &[], &[]);
    let given = common::run_shipped(&cnf, &["--structdivassume=1"], &[]);
    let line = |text: &str| text.lines().find(|l| l.contains("div/sqrt: ")).map(str::to_string).unwrap_or_default();
    assert!(line(&plain.stdout).contains("asked of one solver"), "{}", plain.stdout);
    assert_eq!(line(&plain.stdout), line(&given.stdout), "the default v the option given");
    assert_eq!(plain.work(), given.work(), "the default v the option given: work");
    let fresh = common::run_shipped(&cnf, &["--structdivassume=0"], &[]);
    assert_eq!(fresh.status, "UNSATISFIABLE");
    assert!(fresh.stdout.contains("div/sqrt: quotient bit 4 of a 12/12 divider") && !fresh.stdout.contains("asked of one solver"), "{}", fresh.stdout);
    let off = common::run_shipped(&cnf, &["--structdiv=0"], &[]);
    assert_eq!(off.status, "UNSATISFIABLE");
    assert!(!off.stdout.contains("div/sqrt:"), "{}", off.stdout);
}

#[test]
fn the_reading_keeps_to_the_tick_limit() {
    // Codex review round 1: a six-input and written as 10,000 copies of its
    // seven clauses went to work 9,740,088 under a limit of 5,100,000 (the
    // truth table's work was reserved, then charged again)
    let fx = Fixture::new("structdiv_read_limit");
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for _ in 0..10_000 {
        for i in 1..=6 {
            clauses.push(vec![-7, i]);
        }
        clauses.push(vec![7, -1, -2, -3, -4, -5, -6]);
    }
    clauses.push(vec![7]);
    let cnf = fx.path("and6.cnf");
    write_cnf(&cnf, 7, &clauses);
    for limit in [5_100_000u64, 2_000_000] {
        let value = limit.to_string();
        let r = run(&cnf, &["--structdiv=1"], &[("SAT_LIMIT_TICKS", &value)]);
        assert!(r.work() <= limit + limit / 10, "limit {}: work {} ({})", limit, r.work(), r.status);
    }
}

#[test]
fn repeated_root_units_keep_the_sub_calls_linear() {
    // Codex review round 1: 300,000 copies of the miter's unit made every
    // sub-call's import quadratic (14.4 s, UNKNOWN under --time=1, against
    // 0.04 s with the pass off); the copies are now found by clause id and
    // the import polls the termination flag
    let fx = Fixture::new("structdiv_root_copies");
    let (vars, mut clauses) = toy_divider(6, 2, false, false);
    let root = clauses.last().unwrap().clone();
    assert_eq!(root.len(), 1);
    for _ in 0..300_000 {
        clauses.push(root.clone());
    }
    let cnf = fx.path("m.cnf");
    write_cnf(&cnf, vars, &clauses);
    // (0.06 s now; the old import overran even --time=3 and answered UNKNOWN)
    let r = run(&cnf, &["--structdiv=1", "--time=3"], &[("SAT_STRUCT_DEBUG", "1")]);
    assert_eq!(r.status, "UNSATISFIABLE", "{}\n{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("div/sqrt: quotient bit 2 of a 6/6 divider"), "{}", r.stdout);
}

#[test]
fn the_sub_calls_see_the_circuit_numbered_densely() {
    // one unused unit on variable 200,000: the sub-calls see the circuit
    // renamed 1..k, so the toy refutes, with a checked proof, in the same
    // work as without the stray variable plus the reading's tables (a dozen
    // visits per variable), not a 200,000-entry table per sub-call
    let fx = Fixture::new("structdiv_dense");
    let (vars, clauses) = toy_divider(12, 4, false, false);
    let plain = fx.path("plain.cnf");
    write_cnf(&plain, vars, &clauses);
    let mut sparse_clauses = clauses.clone();
    sparse_clauses.insert(0, vec![-200_000]);
    let sparse = fx.path("sparse.cnf");
    write_cnf(&sparse, 200_000, &sparse_clauses);
    for chain in CHAINS {
        let (sa, wa) = div_work(&plain, None, None, chain);
        let (sb, wb) = div_work(&sparse, None, None, chain);
        assert_eq!(sa, "UNSATISFIABLE", "{}", chain);
        assert_eq!(sb, "UNSATISFIABLE", "{}", chain);
        assert!(wb < wa + 16 * 200_000, "{}: the sparse formula's work {} against {}", chain, wb, wa);
        let proof = fx.path(&format!("sparse.{}.drat", tag(chain)));
        let (sc, _) = div_work(&sparse, Some(&proof), None, chain);
        assert_eq!(sc, "UNSATISFIABLE", "{}", chain);
        assert_eq!(gate(&sparse, &proof), "ok", "{}", chain);
    }
}

// ---------------------------------------------------------------------------
// clauses added to the circuit (bead SAT-playground-skz.27)
// ---------------------------------------------------------------------------

/// Clauses g | u for the first `count` gates g in number order, u the
/// smallest input outside g's support: each joins g's group (its first
/// literal) and has to be set aside for g to read as a gate.
fn extra_on_gates(clauses: &[Vec<i64>], inputs: i64, count: usize) -> Vec<Vec<i64>> {
    let mut sup: std::collections::BTreeMap<i64, std::collections::BTreeSet<i64>> = std::collections::BTreeMap::new();
    for c in clauses.iter().filter(|c| c.len() > 1) {
        let e = sup.entry(c[0].abs()).or_default();
        e.extend(c[1..].iter().map(|l| l.abs()));
    }
    let out: Vec<Vec<i64>> = sup
        .iter()
        .filter(|(g, _)| **g > inputs)
        .take(count)
        .map(|(g, s)| vec![*g, (1..=inputs).find(|u| !s.contains(u)).expect("an input outside the support")])
        .collect();
    assert_eq!(out.len(), count, "the toy has {} gates", count);
    out
}

#[test]
fn div_pass_reads_a_divider_with_clauses_added() {
    // a clause on two inputs, first literal an input (as Satsuma's lex mode
    // adds 49653 -49655 to div_miter_lec__2), and clauses g | u on three
    // gates: each is set aside, and the pass fires with a checked proof
    let fx = Fixture::new("structdiv_extra");
    let (vars, clauses) = toy_divider(8, 3, false, false);
    let inputs = (8 - 3) + 8;
    let mut more = clauses.clone();
    more.push(vec![1, -2]);
    more.extend(extra_on_gates(&clauses, inputs, 3));
    let cnf = fx.path("m.cnf");
    write_cnf(&cnf, vars, &more);
    for chain in CHAINS {
        let proof = fx.path(&format!("m.{}.drat", tag(chain)));
        let (stdout, stderr) = run_div_with_proof(&cnf, &proof, chain, &[("SAT_STRUCT_DEBUG", "1")]);
        assert!(stdout.contains("s UNSATISFIABLE"), "{}: {}\n{}", chain, stdout, stderr);
        assert!(stdout.contains("div/sqrt: quotient bit 3 of a 8/8 divider"), "{}: the pass did not fire: {}\n{}", chain, stdout, stderr);
        assert_eq!(gate(&cnf, &proof), "ok", "{}", chain);
    }
}

#[test]
fn div_pass_reads_a_root_propagated_into_two_clauses() {
    // the root's unit propagated into its xor, as Satsuma's fix mode leaves
    // div_miter_lec__2: x | y and ~x | ~y where the xor's clauses were (both
    // start with x, so they join the group of side one's output), the unit
    // kept on a variable no clause defines any more
    let fx = Fixture::new("structdiv_root");
    let (vars, clauses) = toy_divider(8, 3, false, false);
    let g = clauses.last().unwrap()[0];
    assert!(clauses.last().unwrap().len() == 1 && g > 0, "the toy ends with the root's unit");
    let xor: Vec<&Vec<i64>> = clauses.iter().filter(|c| c.len() == 3 && c[0].abs() == g).collect();
    assert_eq!(xor.len(), 4, "the root is an xor of four clauses");
    let mut out: Vec<Vec<i64>> = Vec::new();
    for c in &clauses {
        if c.len() == 3 && c[0].abs() == g {
            if c[0] == -g {
                out.push(c[1..].to_vec());
            }
            continue;
        }
        out.push(c.clone());
    }
    let cnf = fx.path("m.cnf");
    write_cnf(&cnf, vars, &out);
    for chain in CHAINS {
        let proof = fx.path(&format!("m.{}.drat", tag(chain)));
        let (stdout, stderr) = run_div_with_proof(&cnf, &proof, chain, &[("SAT_STRUCT_DEBUG", "1")]);
        assert!(stdout.contains("s UNSATISFIABLE"), "{}: {}\n{}", chain, stdout, stderr);
        assert!(stdout.contains("div/sqrt: quotient bit 3 of a 8/8 divider"), "{}: the pass did not fire: {}\n{}", chain, stdout, stderr);
        assert_eq!(gate(&cnf, &proof), "ok", "{}", chain);
    }
}

#[test]
fn the_reader_sets_aside_at_most_64_clauses() {
    // 64 clauses g | u are a circuit with extras and the pass fires; 65 are
    // past the limit, the pass declines and the search refutes the toy alone,
    // both with a checked proof
    let fx = Fixture::new("structdiv_aside");
    let (vars, clauses) = toy_divider(8, 3, false, false);
    let inputs = (8 - 3) + 8;
    for (count, fires) in [(64usize, true), (65, false)] {
        let mut more = clauses.clone();
        more.extend(extra_on_gates(&clauses, inputs, count));
        let cnf = fx.path(&format!("m{}.cnf", count));
        write_cnf(&cnf, vars, &more);
        for chain in CHAINS {
            let proof = fx.path(&format!("m{}.{}.drat", count, tag(chain)));
            let (stdout, stderr) = run_div_with_proof(&cnf, &proof, chain, &[("SAT_STRUCT_DEBUG", "1")]);
            assert!(stdout.contains("s UNSATISFIABLE"), "{} {}: {}\n{}", count, chain, stdout, stderr);
            assert_eq!(stdout.contains("div/sqrt: quotient bit 3 of a 8/8 divider"), fires, "{} {}: {}\n{}", count, chain, stdout, stderr);
            if !fires {
                assert!(stdout.contains("div/sqrt: no divider or square-root miter"), "{} {}: {}", count, chain, stdout);
                assert!(stderr.contains("structdiv: more than 64 clauses define no gate"), "{} {}: {}", count, chain, stderr);
            }
            assert_eq!(gate(&cnf, &proof), "ok", "{} {}", count, chain);
        }
    }
}
