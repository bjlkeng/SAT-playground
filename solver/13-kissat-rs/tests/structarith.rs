//! The linear arithmetic pass (bead SAT-playground-skz.3): a miter of two
//! multipliers over the same partial products is refuted with a proof the
//! two-stage gate accepts, a tampered proof is rejected, and on a formula
//! without the structure the pass derives nothing and the search is the
//! stock one (only the work clock moves, by the pass's own work).

mod common;

use common::{php, run, solver_bin, Fixture};
use std::collections::{HashMap, HashSet};
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
    /// every gate: its output, its inputs and its clauses, for pruning
    gates: Vec<(i64, Vec<i64>, usize, usize)>,
    /// generate as a nor over the negations, propagate as an xnor read
    /// negated: the polarities a mapped prefix adder shows
    inverted: bool,
}

impl Circuit {
    fn new() -> Self {
        Circuit { vars: 0, clauses: Vec::new(), exotic: false, gates: Vec::new(), inverted: false }
    }
    fn var(&mut self) -> i64 {
        self.vars += 1;
        self.vars
    }
    fn gate(&mut self, inputs: &[i64], f: impl FnOnce(&mut Self, i64)) -> i64 {
        let o = self.var();
        let start = self.clauses.len();
        f(self, o);
        self.gates.push((o, inputs.to_vec(), start, self.clauses.len()));
        o
    }
    fn and2(&mut self, a: i64, b: i64) -> i64 {
        self.gate(&[a, b], |c, o| c.clauses.extend([vec![-o, a], vec![-o, b], vec![o, -a, -b]]))
    }
    fn or2(&mut self, a: i64, b: i64) -> i64 {
        self.gate(&[a, b], |c, o| c.clauses.extend([vec![o, -a], vec![o, -b], vec![-o, a, b]]))
    }
    fn xor2(&mut self, a: i64, b: i64) -> i64 {
        self.gate(&[a, b], |c, o| c.clauses.extend([vec![-o, a, b], vec![-o, -a, -b], vec![o, -a, b], vec![o, a, -b]]))
    }
    fn xor3(&mut self, a: i64, b: i64, c: i64) -> i64 {
        self.gate(&[a, b, c], |cc, o| {
            for m in 0..8 {
                let (ma, mb, mc) = (m & 1, (m >> 1) & 1, (m >> 2) & 1);
                let par = ma ^ mb ^ mc;
                let lit = |x: i64, v: i32| if v == 0 { x } else { -x };
                cc.clauses.push(vec![lit(a, ma), lit(b, mb), lit(c, mc), if par == 1 { o } else { -o }]);
            }
        })
    }
    fn maj3(&mut self, a: i64, b: i64, c: i64) -> i64 {
        self.gate(&[a, b, c], |cc, o| cc.clauses.extend([vec![-o, a, b], vec![-o, a, c], vec![-o, b, c], vec![o, -a, -b], vec![o, -a, -c], vec![o, -b, -c]]))
    }
    /// Keep only the gates under `roots` (dead logic removed, as a
    /// synthesis tool does for one compared bit) and the clauses of no gate.
    fn prune(&mut self, roots: &[i64]) {
        let mut keep: HashSet<i64> = HashSet::new();
        let mut stack: Vec<i64> = roots.iter().map(|r| r.abs()).collect();
        let by_out: HashMap<i64, usize> = self.gates.iter().enumerate().map(|(i, g)| (g.0, i)).collect();
        while let Some(v) = stack.pop() {
            if !keep.insert(v) {
                continue;
            }
            if let Some(&i) = by_out.get(&v) {
                stack.extend(self.gates[i].1.iter().map(|l| l.abs()));
            }
        }
        let mut drop: HashSet<usize> = HashSet::new();
        for (o, _, start, end) in &self.gates {
            if !keep.contains(o) {
                drop.extend(*start..*end);
            }
        }
        let old = std::mem::take(&mut self.clauses);
        self.clauses = old.into_iter().enumerate().filter(|(i, _)| !drop.contains(i)).map(|(_, c)| c).collect();
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

/// Column by column down to two rows, then a carry-propagate adder of the
/// given kind over the rows: `ripple` (full adders), or generate and
/// propagate per column and a parallel-prefix network (`kogge-stone`,
/// `brent-kung`, `sklansky`) whose carries give the sum bits.
fn rows_multiplier(c: &mut Circuit, pp: &[Vec<i64>], two_input: bool, kind: &str, reverse: bool) -> Vec<i64> {
    rows_multiplier_at(c, pp, two_input, kind, reverse, None)
}

/// As `rows_multiplier`; `double_read`: that bit's sum written as
/// `xor(xor(a, carry), xor(b, carry))`, which reads the carry twice (and
/// so cancels it: a wrong bit).
fn rows_multiplier_at(c: &mut Circuit, pp: &[Vec<i64>], two_input: bool, kind: &str, reverse: bool, double_read: Option<usize>) -> Vec<i64> {
    let n = pp.len();
    let mut cols: Vec<Vec<i64>> = vec![Vec::new(); 2 * n];
    for (i, row) in pp.iter().enumerate() {
        for (j, &p) in row.iter().enumerate() {
            cols[i + j].push(p);
        }
    }
    for k in 0..2 * n {
        while cols[k].len() > 2 {
            // (the two sides of a miter reduce in different orders, so that
            // their trees differ, as two architectures' do)
            let mut take = |cols: &mut Vec<Vec<i64>>| if reverse { cols[k].remove(0) } else { cols[k].pop().unwrap() };
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
    }
    let width = 2 * n;
    if kind == "ripple" {
        let mut out = Vec::new();
        let mut carry: Option<i64> = None;
        for col in cols.iter().take(width) {
            let mut ins = col.clone();
            if let Some(cy) = carry {
                ins.push(cy);
            }
            let (s, cy) = match ins.len() {
                0 => (None, None),
                1 => (Some(ins[0]), None),
                2 => {
                    let (s, cy) = c.half_adder(ins[0], ins[1]);
                    (Some(s), Some(cy))
                }
                _ => {
                    let (s, cy) = c.full_adder(ins[0], ins[1], ins[2], two_input);
                    (Some(s), Some(cy))
                }
            };
            out.push(s.unwrap_or(0));
            carry = cy;
        }
        return out;
    }
    // generate and propagate; None is a constant (generate 0, propagate 1)
    let mut g: Vec<Option<i64>> = Vec::new();
    let mut p: Vec<Option<i64>> = Vec::new();
    for col in cols.iter().take(width) {
        match col.len() {
            0 => {
                g.push(None);
                p.push(None);
            }
            1 => {
                g.push(None);
                p.push(Some(col[0]));
            }
            _ => {
                let (a, b) = (col[0], col[1]);
                if c.inverted {
                    let gn = c.or2(-a, -b); // nand: the generate, read negated
                    let pn = c.xor2(-a, b); // xnor: the propagate, read negated
                    g.push(Some(-gn));
                    p.push(Some(-pn));
                } else {
                    p.push(Some(c.xor2(a, b)));
                    g.push(Some(c.and2(a, b)));
                }
            }
        }
    }
    let gor = |c: &mut Circuit, x: Option<i64>, y: Option<i64>| match (x, y) {
        (None, y) => y,
        (x, None) => x,
        (Some(x), Some(y)) => Some(c.or2(x, y)),
    };
    let pand = |c: &mut Circuit, x: Option<i64>, y: Option<i64>| match (x, y) {
        (None, y) => y,
        (x, None) => x,
        (Some(x), Some(y)) => Some(c.and2(x, y)),
    };
    let op = |c: &mut Circuit, hi: (Option<i64>, Option<i64>), lo: (Option<i64>, Option<i64>)| -> (Option<i64>, Option<i64>) {
        let (gh, ph) = hi;
        let (gl, pl) = lo;
        let t = match (ph, gl) {
            (_, None) => None,
            (None, Some(gl)) => Some(gl),
            (Some(ph), Some(gl)) => Some(c.and2(ph, gl)),
        };
        (gor(c, gh, t), pand(c, ph, pl))
    };
    let mut nodes: Vec<(Option<i64>, Option<i64>)> = (0..width).map(|i| (g[i], p[i])).collect();
    match kind {
        "kogge-stone" => {
            let mut span = 1;
            while span < width {
                let prev = nodes.clone();
                for i in 0..width {
                    if i >= span {
                        nodes[i] = op(c, prev[i], prev[i - span]);
                    }
                }
                span *= 2;
            }
        }
        "brent-kung" => {
            let mut d = 1;
            while d < width {
                let mut i = 2 * d - 1;
                while i < width {
                    nodes[i] = op(c, nodes[i], nodes[i - d]);
                    i += 2 * d;
                }
                d *= 2;
            }
            d /= 2;
            while d >= 1 {
                let mut i = 3 * d - 1;
                while i < width {
                    nodes[i] = op(c, nodes[i], nodes[i - d]);
                    i += 2 * d;
                }
                d /= 2;
            }
        }
        "sklansky" => {
            let mut d = 1;
            while d < width {
                let prev = nodes.clone();
                for i in 0..width {
                    if (i / d) % 2 == 1 {
                        nodes[i] = op(c, prev[i], prev[(i / d) * d - 1]);
                    }
                }
                d *= 2;
            }
        }
        _ => panic!("unknown adder {}", kind),
    }
    let mut out = Vec::new();
    for i in 0..width {
        let carry_in = if i == 0 { None } else { nodes[i - 1].0 };
        if double_read == Some(i) {
            let (a, b, cy) = (cols[i][0], cols[i][1], carry_in.expect("a carry into the bit"));
            let (x, y) = (c.xor2(a, cy), c.xor2(b, cy));
            out.push(c.xor2(x, y));
            continue;
        }
        out.push(match (p[i], carry_in) {
            (None, None) => 0,
            (None, Some(x)) => x,
            (Some(x), None) => x,
            (Some(x), Some(y)) => c.xor2(x, y),
        });
    }
    out
}

enum Miter {
    /// an array multiplier against a column-wise one
    ArrayColumn,
    /// two rows and a final adder of each kind, side A against side B
    Rows(&'static str, &'static str),
    /// as Rows, with side B's generates and propagates in inverted polarities
    RowsInverted(&'static str, &'static str),
    /// a Kogge-Stone final adder on side A whose top prefix node takes the
    /// or of its propagates (a wrong carry): satisfiable
    FaultyPrefix,
    /// side A's compared bit reads the carry twice, as
    /// `xor(xor(a, c), xor(b, c))`: the carry cancels, a wrong bit, so
    /// satisfiable (the pass must not take the two reads for one)
    DoubleCarryRead,
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
    miter_pruned(kind, n, bit, two_input, false, path)
}

/// As `miter`; `pruned` keeps only the gates under the two compared bits
/// (the lower sum bits gone, only the carry into the compared column).
fn miter_pruned(kind: Miter, n: usize, bit: usize, two_input: bool, pruned: bool, path: &Path) {
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
        Miter::Rows(ka, kb) => (rows_multiplier(&mut c, &pp, two_input, ka, false), rows_multiplier(&mut c, &pp, two_input, kb, true)),
        Miter::RowsInverted(ka, kb) => {
            let oa = rows_multiplier(&mut c, &pp, two_input, ka, false);
            c.inverted = true;
            let ob = rows_multiplier(&mut c, &pp, two_input, kb, true);
            c.inverted = false;
            (oa, ob)
        }
        Miter::DoubleCarryRead => (rows_multiplier_at(&mut c, &pp, two_input, "kogge-stone", false, Some(bit)), column_multiplier(&mut c, &pp, two_input, None, 2 * n)),
        Miter::FaultyPrefix => {
            // the fault: the compared bit's sum xor(p, carry) reads
            // or(p, carry) in place of the carry, so the bit is wrong for
            // some inputs
            let oa = rows_multiplier(&mut c, &pp, two_input, "kogge-stone", false);
            let ob = column_multiplier(&mut c, &pp, two_input, None, 2 * n);
            let gi = c.gates.iter().position(|g| g.0 == oa[bit]).expect("the compared bit is a gate");
            let (_, ins, start, end) = c.gates[gi].clone();
            assert_eq!(ins.len(), 2, "the sum is an xor of propagate and carry");
            let (p_bit, carry) = (ins[0], ins[1]);
            let wrong = c.or2(p_bit, carry);
            for i in start..end {
                for l in c.clauses[i].iter_mut() {
                    if l.abs() == carry.abs() {
                        *l = if *l > 0 { wrong } else { -wrong };
                    }
                }
            }
            c.gates[gi].1 = vec![p_bit, wrong];
            (oa, ob)
        }
    };
    assert_ne!(oa[bit], ob[bit], "the compared bit is one signal on both sides");
    if pruned {
        c.prune(&[oa[bit], ob[bit]]);
    }
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
    // (the constant one side alone adds is caught by the constants' check,
    // which allows a difference of a multiple of 2^(K+1) only)
    assert!(without.stderr.contains("the two sides' constants differ"), "{}", without.stderr);
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
fn arithmetic_pass_refutes_prefix_adder_miters_with_checked_proofs() {
    // the final-adder step (bead skz.12): two rows added by a parallel-
    // prefix network, in full or pruned to the compared bit's cone (the
    // lower sum bits gone, the network computing one carry), the
    // generates and propagates in plain or inverted polarities
    let fx = Fixture::new("structarith_prefix");
    let cases: Vec<(&str, Miter, usize, usize, bool, bool)> = vec![
        ("kogge_stone_v_brent_kung", Miter::Rows("kogge-stone", "brent-kung"), 6, 8, false, false),
        ("sklansky_v_ripple", Miter::Rows("sklansky", "ripple"), 6, 9, true, false),
        ("kogge_stone_v_column_pruned", Miter::Rows("kogge-stone", "ripple"), 6, 8, false, true),
        ("brent_kung_v_sklansky_pruned", Miter::Rows("brent-kung", "sklansky"), 7, 10, false, true),
        ("inverted_pruned", Miter::RowsInverted("sklansky", "kogge-stone"), 6, 8, true, true),
        ("ripple_dead_sums_v_brent_kung_pruned", Miter::Rows("ripple", "brent-kung"), 6, 9, false, true),
    ];
    for (name, kind, n, bit, two_input, pruned) in cases {
        let cnf = fx.path(&format!("{}.cnf", name));
        miter_pruned(kind, n, bit, two_input, pruned, &cnf);
        if let Ok(dir) = std::env::var("SAT_TEST_KEEP") {
            let _ = std::fs::copy(&cnf, Path::new(&dir).join(format!("{}.cnf", name)));
        }
        let proof = fx.path(&format!("{}.drat", name));
        let stdout = run_arith_with_proof(&cnf, &proof);
        assert!(stdout.contains("s UNSATISFIABLE"), "{}: {}", name, stdout);
        assert!(stdout.contains("arithmetic: two adder networks"), "{}: the pass did not fire: {}", name, stdout);
        assert_eq!(gate(&cnf, &proof), "ok", "{}: two-stage gate", name);
    }
    // with every default on, the parity engine takes the proof stream over
    // after the pass and defines variables of its own: they must not reuse
    // the pass's fresh ones (the proof-only numbers are handed on)
    let cnf = fx.path("defaults.cnf");
    miter_pruned(Miter::Rows("sklansky", "ripple"), 6, 9, true, false, &cnf);
    let proof = fx.path("defaults.drat");
    let out = Command::new(solver_bin()).arg("-n").arg(&cnf).arg(&proof).output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    assert!(stdout.contains("arithmetic: two adder networks"), "{}", stdout);
    assert!(stdout.contains("parity engine on"), "the parity engine ran after the pass: {}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok", "defaults: the parity engine continued the pass's proof");
    // and the work clock is the same from run to run
    let clocks: Vec<String> = (0..3)
        .map(|_| {
            let out = Command::new(solver_bin()).arg("-n").arg(&cnf).output().expect("run sat-solver");
            let text = String::from_utf8_lossy(&out.stdout);
            text.lines().find(|l| l.starts_with("c workclock")).unwrap_or("").to_string()
        })
        .collect();
    assert!(clocks.iter().all(|c| c == &clocks[0]), "{:?}", clocks);
}

#[test]
fn a_tampered_chain_lemma_fails_the_gate() {
    // the lemma that ties a network's carry to the virtual chain's bit is
    // two rup clauses after kitten's learned ones; with one of them claiming
    // the complement, the gate must reject the proof
    let fx = Fixture::new("structarith_lemma_tamper");
    let cnf = fx.path("m.cnf");
    miter_pruned(Miter::Rows("kogge-stone", "ripple"), 6, 8, false, true, &cnf);
    let proof = fx.path("m.drat");
    let stdout = run(&cnf, &["--structarith=1"], &[("SAT_STRUCT_DEBUG", "1")]);
    assert!(stdout.stderr.contains("= chain bit"), "the lemma ran: {}", stdout.stderr);
    let stdout = run_arith_with_proof(&cnf, &proof);
    assert!(stdout.contains("arithmetic: two adder networks"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok");
    let pbp = fx.path("m.drat.pbp");
    let text = std::fs::read_to_string(&pbp).unwrap();
    // the equivalence's clauses: the last two `rup` lines of two literals
    // before the first weighted sum (`pol ... *`)
    let first_pol = text.find("\npol ").expect("a weighted sum");
    let head = &text[..first_pol];
    let line_start = head.rfind("\nrup 1 ").expect("a lemma clause") + 1;
    let line_end = line_start + text[line_start..].find('\n').unwrap();
    let line = &text[line_start..line_end];
    let lits: Vec<&str> = line.split_whitespace().filter(|t| t.starts_with('x') || t.starts_with("~x")).collect();
    assert_eq!(lits.len(), 2, "a binary lemma clause: {}", line);
    let flipped = if lits[1].starts_with('~') { lits[1][1..].to_string() } else { format!("~{}", lits[1]) };
    let tampered = format!("{}{}{}", &text[..line_start], line.replacen(lits[1], &flipped, 1), &text[line_end..]);
    std::fs::write(&pbp, tampered).unwrap();
    let verdict = gate(&cnf, &proof);
    assert!(verdict.starts_with("FAIL"), "{}", verdict);
}

#[test]
fn a_faulty_prefix_network_stays_satisfiable() {
    // a prefix adder with a wrong carry: the network's carry is not the
    // chain's bit, kitten finds the difference, the pass declines, and the
    // formula stays satisfiable with or without a proof file
    let fx = Fixture::new("structarith_faulty_prefix");
    let cnf = fx.path("m.cnf");
    miter_pruned(Miter::FaultyPrefix, 6, 9, false, true, &cnf);
    let without = run(&cnf, &["--structarith=1"], &[("SAT_STRUCT_DEBUG", "1")]);
    assert_eq!(without.status, "SATISFIABLE", "{}", without.stdout);
    assert!(!without.stdout.contains("arithmetic: two adder networks"), "{}", without.stdout);
    let proof = fx.path("m.drat");
    let stdout = run_arith_with_proof(&cnf, &proof);
    assert!(stdout.contains("s SATISFIABLE"), "{}", stdout);
}

#[test]
fn a_carry_read_twice_stays_satisfiable() {
    // the lemma cancels one read of the network's carry against the chain's
    // bit; a side reading it twice must be declined (review of 2026-10-21:
    // both reads were dropped from the leaf forms, a false UNSAT without a
    // proof file)
    let fx = Fixture::new("structarith_double_read");
    for (name, n, bit, pruned) in [("full", 5, 6, false), ("pruned", 6, 8, true)] {
        let cnf = fx.path(&format!("{}.cnf", name));
        miter_pruned(Miter::DoubleCarryRead, n, bit, false, pruned, &cnf);
        let without = run(&cnf, &["--structarith=1"], &[("SAT_STRUCT_DEBUG", "1")]);
        assert_eq!(without.status, "SATISFIABLE", "{}: {}", name, without.stdout);
        assert!(!without.stdout.contains("arithmetic: two adder networks"), "{}: {}", name, without.stdout);
        let proof = fx.path(&format!("{}.drat", name));
        let stdout = run_arith_with_proof(&cnf, &proof);
        assert!(stdout.contains("s SATISFIABLE"), "{}: {}", name, stdout);
    }
}

#[test]
fn a_fixed_one_alone_at_the_chains_top_column_is_declined() {
    // two sums of 12 shared products, one adding 2 and the other 1, bit 2
    // compared: satisfiable. The chain's column at the compared weight held
    // one fixed-one variable and nothing else, and the lemma then traded
    // the network's carry for a constant no output cancelled (review of
    // 2026-10-21: a false UNSAT under the defaults)
    let cnf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fixed_one_chain_bit.cnf");
    let without = run(&cnf, &[], &[("SAT_STRUCT_DEBUG", "1")]);
    assert_eq!(without.status, "SATISFIABLE", "{}", without.stdout);
    assert!(!without.stdout.contains("arithmetic: two adder networks"), "{}", without.stdout);
    let fx = Fixture::new("structarith_fixed_one");
    let proof = fx.path("m.drat");
    let stdout = run_arith_with_proof(&cnf, &proof);
    assert!(stdout.contains("s SATISFIABLE"), "{}", stdout);
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
