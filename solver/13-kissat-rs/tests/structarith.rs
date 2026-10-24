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
    /// A gate given by its function over `ins`, one clause per row.
    fn table(&mut self, ins: &[i64], f: &dyn Fn(&[bool]) -> bool) -> i64 {
        let ins_v = ins.to_vec();
        self.gate(ins, |c, o| {
            for m in 0..(1u32 << ins_v.len()) {
                let bits: Vec<bool> = (0..ins_v.len()).map(|i| (m >> i) & 1 == 1).collect();
                let mut clause: Vec<i64> = ins_v.iter().zip(bits.iter()).map(|(&x, &b)| if b { -x } else { x }).collect();
                clause.push(if f(&bits) { o } else { -o });
                c.clauses.push(clause);
            }
        })
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
    column_multiplier_dup(c, pp, two_input, plus_one, reverse_from, None, &[])
}

/// As `column_multiplier`, with `duplicate`: pp[i][j] read twice (the
/// product is then off by 2^(i+j)); `extra`: more addends (column, literal).
fn column_multiplier_dup(c: &mut Circuit, pp: &[Vec<i64>], two_input: bool, plus_one: Option<(usize, bool)>, reverse_from: usize, duplicate: Option<(usize, usize)>, extra: &[(usize, i64)]) -> Vec<i64> {
    let n = pp.len();
    let mut cols: Vec<Vec<i64>> = vec![Vec::new(); 2 * n];
    for (i, row) in pp.iter().enumerate() {
        for (j, &p) in row.iter().enumerate() {
            cols[i + j].push(p);
        }
    }
    for &(k, l) in extra {
        cols[k].push(l);
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

/// Column by column with half adders, every partial product first xored
/// with the shared input `sign` (as a Booth row bit is with its digit's
/// sign), the xor folded into the first adder of its column: that adder
/// computes its sum and carry straight from (p, sign, the other addend)
/// as one 3-input gate each, which is what a generator does when it folds
/// the row bit into the adder (the fused-leaf cells of structarith).
fn folded_sign_multiplier(c: &mut Circuit, pp: &[Vec<i64>], sign: i64) -> Vec<i64> {
    let n = pp.len();
    let mut cols: Vec<Vec<i64>> = vec![Vec::new(); 2 * n];
    for (i, row) in pp.iter().enumerate() {
        for (j, &p) in row.iter().enumerate() {
            cols[i + j].push(p);
        }
    }
    let mut raw: std::collections::HashSet<i64> = cols.iter().flatten().copied().collect();
    fn table(c: &mut Circuit, ins: &[i64], f: &dyn Fn(&[bool]) -> bool) -> i64 {
        let ins_v = ins.to_vec();
        c.gate(ins, |c, o| {
            for m in 0..(1u32 << ins_v.len()) {
                let bits: Vec<bool> = (0..ins_v.len()).map(|i| (m >> i) & 1 == 1).collect();
                let mut clause: Vec<i64> = ins_v.iter().zip(bits.iter()).map(|(&x, &b)| if b { -x } else { x }).collect();
                clause.push(if f(&bits) { o } else { -o });
                c.clauses.push(clause);
            }
        })
    }
    let mut out = Vec::new();
    for k in 0..2 * n {
        // every raw addend but the first of the column gets its xor as a
        // signal (the first is folded into the adder reading it)
        let mut first_raw = true;
        let many = cols[k].len() > 1;
        for idx in 0..cols[k].len() {
            let x = cols[k][idx];
            if raw.contains(&x) {
                if first_raw && many {
                    first_raw = false;
                } else {
                    let y = c.xor2(x, sign);
                    raw.remove(&x);
                    cols[k][idx] = y;
                }
            }
        }
        while cols[k].len() > 1 {
            let (x, y) = (cols[k].remove(0), cols[k].remove(0));
            let (s, cy) = if raw.remove(&x) {
                let s = table(c, &[x, sign, y], &|b| b[0] ^ b[1] ^ b[2]);
                let cy = table(c, &[x, sign, y], &|b| (b[0] ^ b[1]) && b[2]);
                (s, cy)
            } else {
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

/// A radix-4 Booth multiplier of the unsigned `a` and `b` (n even), as the
/// 2025 generator's `origin` form builds it: digit j is `-2 b_{2j+1} +
/// b_{2j} + b_{2j-1}`; its row's bit t is `(one and a_t) or (two and
/// a_{t-1})` xored with the sign `neg = b_{2j+1}`, the `+neg` bit at the
/// row's lowest column, `~neg` at the column above the row (the sign
/// extension's `s~`), and the constant ones that complete the sign
/// extension (`-2^(2j+n+1)` for each row, as a positive constant modulo
/// `2^2n`) built as the generator builds them: a half adder over two
/// complemented bits of the column whose outputs are read complemented
/// (`1 + x + y`). The correction row `a * b_{n-1} * 2^n` makes the sum
/// `a * b`. The columns reduce with full and half adders to two rows and
/// a final adder of `kind` (`ripple`, or a prefix kind) adds them.
fn booth_multiplier(c: &mut Circuit, a: &[i64], b: &[i64], two_input: bool, kind: &str) -> Vec<i64> {
    booth_multiplier_with(c, a, b, two_input, kind, false, None)
}

/// As `booth_multiplier`, with two shapes a Booth Wallace generator makes:
/// `plus_neg_ha` adds each row's bit 0 and its +neg (rows 1 up) in a half
/// adder whose sum is computed back from the row bit, xor(xnor(raw, neg),
/// neg), the raw bit again, and whose carry is neg and not raw; and
/// `doubled_carry` sends the first carry out of column J to column J+1
/// complemented and to column J+2 plain (x 2^(J+1) = ~x 2^(J+1) + x
/// 2^(J+2) - 2^(J+1), the constant with the sign-extension ones; with
/// `false` in the pair the constant is left out, a wrong multiplier).
fn booth_multiplier_with(c: &mut Circuit, a: &[i64], b: &[i64], two_input: bool, kind: &str, plus_neg_ha: bool, doubled: Option<(usize, bool)>) -> Vec<i64> {
    let doubled_carry = doubled.map(|(k, _)| k);
    let n = a.len();
    assert!(n % 2 == 0 && n >= 4);
    let width = 2 * n;
    let mut cols: Vec<Vec<i64>> = vec![Vec::new(); width + 2];
    let mut constant: i128 = 0;
    if let Some((k, true)) = doubled {
        constant -= 1i128 << (k + 1);
    }
    for j in 0..n / 2 {
        let lo = if j == 0 { None } else { Some(b[2 * j - 1]) };
        let mid = b[2 * j];
        let hi = b[2 * j + 1];
        // one: the digit is odd; two: the digit is +2 or -2
        let one = match lo {
            Some(lo) => c.xor2(mid, lo),
            None => mid,
        };
        let differ = c.xor2(hi, mid);
        let two = c.and2(differ, -one);
        let neg = hi;
        for t in 0..=n {
            let first = if t < n { Some(c.and2(one, a[t])) } else { None };
            let second = if t > 0 { Some(c.and2(two, a[t - 1])) } else { None };
            let raw = match (first, second) {
                (Some(x), Some(y)) => c.or2(x, y),
                (Some(x), None) | (None, Some(x)) => x,
                (None, None) => unreachable!(),
            };
            if plus_neg_ha && j > 0 && t == 0 {
                // bit + neg = raw + 2 (neg and not raw): the half adder's
                // sum xor(xnor(raw, neg), neg) is not raw, read complemented
                let nbit = c.xor2(raw, -neg);
                let s2 = c.xor2(nbit, neg);
                let cy = c.and2(neg, -raw);
                cols[2 * j].push(-s2);
                cols[2 * j + 1].push(cy);
                continue;
            }
            let bit = c.xor2(raw, neg);
            if 2 * j + t < width {
                cols[2 * j + t].push(bit);
            }
        }
        if !(plus_neg_ha && j > 0) {
            cols[2 * j].push(neg); // the +1 of a negative digit
        }
        if 2 * j + n + 1 < width {
            cols[2 * j + n + 1].push(-neg); // s~
        }
        constant -= 1i128 << (2 * j + n + 1);
    }
    // the correction row for an unsigned multiplicand's top digit
    for i in 0..n {
        if i + n < width {
            let p = c.and2(a[i], b[n - 1]);
            cols[i + n].push(p);
        }
    }
    // the constant ones, each as a half adder over two complemented bits
    // of its column read complemented
    let constant = constant.rem_euclid(1i128 << width);
    for k in 0..width {
        if (constant >> k) & 1 == 1 {
            assert!(cols[k].len() >= 2, "a constant one needs two bits in its column");
            let x = cols[k].pop().unwrap();
            let y = cols[k].pop().unwrap();
            let s = c.xor2(-x, -y);
            let cy = c.and2(-x, -y);
            cols[k].push(-s);
            cols[k + 1].push(-cy);
        }
    }
    // the tree: every column down to two bits (the doubled carry joins
    // column J+2 last, after the carries out of J+1, so the adder reading
    // it there reads produced signals beside it, as in the 2025 cells)
    let mut deferred: Option<(usize, i64)> = None;
    for k in 0..width {
        if let Some((kk, cy)) = deferred {
            if kk == k {
                cols[k].push(cy);
                deferred = None;
            }
        }
        let mut first = true;
        while cols[k].len() > 2 {
            let (s, cy) = if cols[k].len() >= 3 {
                let (x, y, z) = (cols[k].remove(0), cols[k].remove(0), cols[k].remove(0));
                c.full_adder(x, y, z, two_input)
            } else {
                unreachable!()
            };
            cols[k].push(s);
            if first && doubled_carry == Some(k) {
                cols[k + 1].push(-cy);
                deferred = Some((k + 2, cy));
            } else {
                cols[k + 1].push(cy);
            }
            first = false;
        }
    }
    // the final adder
    let mut out = Vec::new();
    match kind {
        "ripple" => {
            let mut carry: Option<i64> = None;
            for k in 0..width {
                let bits: Vec<i64> = cols[k].iter().copied().chain(carry).collect();
                let (s, cy) = match bits.len() {
                    0 => (0, None),
                    1 => (bits[0], None),
                    2 => {
                        let (s, cy) = c.half_adder(bits[0], bits[1]);
                        (s, Some(cy))
                    }
                    _ => {
                        let (s, cy) = c.full_adder(bits[0], bits[1], bits[2], two_input);
                        (s, Some(cy))
                    }
                };
                out.push(s);
                carry = cy;
            }
        }
        _ => {
            // two rows (a column with one bit gets a zero: the sum is that
            // bit, the carry none), then the prefix adder of rows_multiplier
            let rows: Vec<Vec<i64>> = cols[..width].to_vec();
            return prefix_sum(c, &rows, kind);
        }
    }
    out
}

/// A radix-4 Booth multiplier as a technology mapper writes one (bead
/// SAT-playground-skz.13): no gate computes a row bit, the select part
/// xor the digit's sign; the first full adder reading a row bit takes its
/// two parts, its sum the xor of four signals and its carry the majority
/// of the row bit and the two others, each one gate over the four. The
/// constant ones are complemented half adders, the final adder a ripple.
fn booth_multiplier_mapped(c: &mut Circuit, a: &[i64], b: &[i64]) -> Vec<i64> {
    booth_multiplier_mapped_with(c, a, b, false)
}

/// As `booth_multiplier_mapped`; `reverse` takes each column's adders from
/// its last signals first (another compressor order).
fn booth_multiplier_mapped_with(c: &mut Circuit, a: &[i64], b: &[i64], reverse: bool) -> Vec<i64> {
    #[derive(Clone, Copy)]
    enum Item {
        Lit(i64),
        Row(i64, i64), // (select, sign): the row bit select xor sign, no gate of its own
    }
    let n = a.len();
    assert!(n % 2 == 0 && n >= 4);
    let width = 2 * n;
    let mut cols: Vec<Vec<Item>> = vec![Vec::new(); width + 2];
    let mut constant: i128 = 0;
    for j in 0..n / 2 {
        let lo = if j == 0 { None } else { Some(b[2 * j - 1]) };
        let mid = b[2 * j];
        let hi = b[2 * j + 1];
        let one = match lo {
            Some(lo) => c.xor2(mid, lo),
            None => mid,
        };
        let differ = c.xor2(hi, mid);
        let two = c.and2(differ, -one);
        for t in 0..=n {
            let first = if t < n { Some(c.and2(one, a[t])) } else { None };
            let second = if t > 0 { Some(c.and2(two, a[t - 1])) } else { None };
            let raw = match (first, second) {
                (Some(x), Some(y)) => c.or2(x, y),
                (Some(x), None) | (None, Some(x)) => x,
                (None, None) => unreachable!(),
            };
            if 2 * j + t < width {
                cols[2 * j + t].push(Item::Row(raw, hi));
            }
        }
        cols[2 * j].push(Item::Lit(hi));
        if 2 * j + n + 1 < width {
            cols[2 * j + n + 1].push(Item::Lit(-hi));
        }
        constant -= 1i128 << (2 * j + n + 1);
    }
    for i in 0..n {
        if i + n < width {
            let p = c.and2(a[i], b[n - 1]);
            cols[i + n].push(Item::Lit(p));
        }
    }
    let lit = |c: &mut Circuit, it: Item| -> i64 {
        match it {
            Item::Lit(x) => x,
            Item::Row(r, s) => c.xor2(r, s),
        }
    };
    let constant = constant.rem_euclid(1i128 << width);
    for k in 0..width {
        if (constant >> k) & 1 == 1 {
            assert!(cols[k].len() >= 2, "a constant one needs two bits in its column");
            let x = cols[k].pop().unwrap();
            let y = cols[k].pop().unwrap();
            let (x, y) = (lit(c, x), lit(c, y));
            let s = c.xor2(-x, -y);
            let cy = c.and2(-x, -y);
            cols[k].push(Item::Lit(-s));
            cols[k + 1].push(Item::Lit(-cy));
        }
    }
    for k in 0..width {
        while cols[k].len() > 2 {
            let (x, y, z) = if reverse {
                (cols[k].pop().unwrap(), cols[k].pop().unwrap(), cols[k].pop().unwrap())
            } else {
                (cols[k].remove(0), cols[k].remove(0), cols[k].remove(0))
            };
            // a row bit folded into the adder: its select and sign read
            // straight, the other two as signals
            let (s, cy) = match (x, y, z) {
                (Item::Row(r, sg), y, z) | (y, Item::Row(r, sg), z) | (y, z, Item::Row(r, sg)) => {
                    let (y, z) = (lit(c, y), lit(c, z));
                    let s = c.table(&[r, sg, y, z], &|v| v[0] ^ v[1] ^ v[2] ^ v[3]);
                    let cy = c.table(&[r, sg, y, z], &|v| {
                        let p = v[0] ^ v[1];
                        (p && v[2]) || (p && v[3]) || (v[2] && v[3])
                    });
                    (s, cy)
                }
                (x, y, z) => {
                    let (x, y, z) = (lit(c, x), lit(c, y), lit(c, z));
                    c.full_adder(x, y, z, false)
                }
            };
            cols[k].push(Item::Lit(s));
            cols[k + 1].push(Item::Lit(cy));
        }
    }
    let mut out = Vec::new();
    let mut carry: Option<i64> = None;
    for k in 0..width {
        let col: Vec<Item> = cols[k].clone();
        let mut bits: Vec<i64> = col.into_iter().map(|it| lit(c, it)).collect();
        bits.extend(carry);
        let (s, cy) = match bits.len() {
            0 => (0, None),
            1 => (bits[0], None),
            2 => {
                let (s, cy) = c.half_adder(bits[0], bits[1]);
                (s, Some(cy))
            }
            _ => {
                let (s, cy) = c.full_adder(bits[0], bits[1], bits[2], false);
                (s, Some(cy))
            }
        };
        out.push(s);
        carry = cy;
    }
    out
}

/// The sum of columns of at most two bits by a prefix adder of `kind`
/// (the final adders of `rows_multiplier`).
fn prefix_sum(c: &mut Circuit, cols: &[Vec<i64>], kind: &str) -> Vec<i64> {
    let width = cols.len();
    let g: Vec<Option<i64>> = cols.iter().map(|col| if col.len() == 2 { Some(c.and2(col[0], col[1])) } else { None }).collect();
    let p: Vec<Option<i64>> = cols.iter().map(|col| match col.len() { 2 => Some(c.xor2(col[0], col[1])), 1 => Some(col[0]), _ => None }).collect();
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
    (0..width)
        .map(|i| {
            let carry_in = if i == 0 { None } else { nodes[i - 1].0 };
            match (p[i], carry_in) {
                (None, None) => 0,
                (None, Some(x)) => x,
                (Some(x), None) => x,
                (Some(x), Some(y)) => c.xor2(x, y),
            }
        })
        .collect()
}

enum Miter {
    /// an array multiplier against a column-wise one
    ArrayColumn,
    /// two rows and a final adder of each kind, side A against side B
    Rows(&'static str, &'static str),
    /// as Rows, with side B's generates and propagates in inverted polarities
    RowsInverted(&'static str, &'static str),
    /// as RowsInverted, with the products of the columns from J up read
    /// negated on both sides (a mixed-sign reading: which side to flip is
    /// then not a matter of counting)
    RowsInvertedMixed(&'static str, &'static str, usize),
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
    /// every partial product xored with a shared sign input: side A with
    /// the xors as signals, side B with each folded into the first adder
    /// reading it (fused-leaf cells)
    FoldedSign,
    /// a radix-4 Booth multiplier with a final adder of the kind against
    /// a column-wise AND multiplier: different leaves on the two sides
    /// (row bits against products), the sign-extension constants built
    /// into complemented half adders, the complementary top bits
    Booth(&'static str),
    /// a Booth multiplier as a mapper writes it (no gate per row bit)
    /// against an AND multiplier
    BoothMapped,
    /// two such Booth multipliers, their compressors in opposite orders
    BoothMappedBoth,
    /// as Booth, with the rows' +neg half adders computed back from the row
    /// bit (`plus_neg_ha`) and a carry read at two columns (`doubled_carry`)
    BoothShapes(&'static str, bool, Option<(usize, bool)>),
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
            column_multiplier_dup(&mut c, &pp, two_input, None, 2 * n, Some((i, j)), &[]),
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
        Miter::RowsInvertedMixed(ka, kb, from) => {
            let mixed: Vec<Vec<i64>> = pp.iter().enumerate().map(|(i, row)| row.iter().enumerate().map(|(j, &p)| if i + j >= from { -p } else { p }).collect()).collect();
            let oa = rows_multiplier(&mut c, &mixed, two_input, ka, false);
            c.inverted = true;
            let ob = rows_multiplier(&mut c, &mixed, two_input, kb, true);
            c.inverted = false;
            (oa, ob)
        }
        Miter::DoubleCarryRead => (rows_multiplier_at(&mut c, &pp, two_input, "kogge-stone", false, Some(bit)), column_multiplier(&mut c, &pp, two_input, None, 2 * n)),
        Miter::Booth(kind) => (booth_multiplier(&mut c, &a, &b, two_input, kind), column_multiplier(&mut c, &pp, two_input, None, 2 * n)),
        Miter::BoothMapped => (booth_multiplier_mapped(&mut c, &a, &b), column_multiplier(&mut c, &pp, two_input, None, 2 * n)),
        Miter::BoothMappedBoth => (booth_multiplier_mapped_with(&mut c, &a, &b, false), booth_multiplier_mapped_with(&mut c, &a, &b, true)),
        Miter::BoothShapes(kind, ha, doubled) => (booth_multiplier_with(&mut c, &a, &b, two_input, kind, ha, doubled), column_multiplier(&mut c, &pp, two_input, None, 2 * n)),
        Miter::FoldedSign => {
            let sign = c.var();
            let signed: Vec<Vec<i64>> = pp.iter().map(|row| row.iter().map(|&p| c.xor2(p, sign)).collect()).collect();
            (column_multiplier(&mut c, &signed, two_input, None, 2 * n), folded_sign_multiplier(&mut c, &pp, sign))
        }
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
    assert!(without.stderr.contains("the two sides sum different polynomials"), "{}", without.stderr);
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
    assert!(without.stderr.contains("the two sides sum different"), "{}", without.stderr);
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
        ("inverted_mixed_pruned", Miter::RowsInvertedMixed("sklansky", "kogge-stone", 5), 6, 8, true, true),
        // (with every product of a side negated, as RowsInvertedMixed(.., 2)
        // has, the pass reads the side as the complement of a multiplier
        // and flips it: a circuit summing the nands of all its products is
        // no multiplier, and a side written in inverted logic is)
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
fn fused_leaf_cells_are_proven_over_their_cut() {
    // side B folds each addend's xor with a shared sign into the adder
    // reading it (no xor signal); the matcher reads sum + 2 carry over the
    // cut as a sum of leaves one of which is the xor, which side A has as
    // a signal: both sides then sum the same variables. The cell's case
    // split must run over the cut, where everything propagates (review of
    // 2026-10-22: split over the leaves, the proof failed)
    let fx = Fixture::new("structarith_fused");
    // (a half-adder-only column reduction of five rows or more makes the
    // sum of two exclusive carries, which the matcher takes for a full
    // adder and leaves the always-zero carry beside it unread: those sizes
    // are not of this test)
    for (n, bit) in [(3usize, 3usize), (4, 5), (4, 4)] {
        let cnf = fx.path(&format!("m{}_{}.cnf", n, bit));
        miter(Miter::FoldedSign, n, bit, false, &cnf);
        if let Ok(dir) = std::env::var("SAT_TEST_KEEP") {
            let _ = std::fs::copy(&cnf, Path::new(&dir).join(format!("folded_sign_{}_{}.cnf", n, bit)));
        }
        let without = run(&cnf, &["--structarith=1"], &[("SAT_STRUCT_DEBUG", "1")]);
        assert_eq!(without.status, "UNSATISFIABLE", "{}", without.stdout);
        assert!(without.stdout.contains("arithmetic: two adder networks"), "{}", without.stdout);
        assert!(without.stderr.contains("fused cell over"), "no fused cell was made: {}", without.stderr);
        let proof = fx.path(&format!("m{}_{}.drat", n, bit));
        let stdout = run_arith_with_proof(&cnf, &proof);
        assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
        assert_eq!(gate(&cnf, &proof), "ok", "n={} bit={}: two-stage gate", n, bit);
    }
}

#[test]
fn booth_miters_are_refuted_with_checked_proofs() {
    // a radix-4 Booth multiplier against an AND multiplier: the sides'
    // leaves are different variables (row bits against products), so the
    // proof expands the leaves into monomial literals; the Booth side
    // carries its sign-extension constants in complemented half adders
    // (the chain keeps those ones); the two compared bits come out as
    // complements as read (a top cell re-read with a one); the AND side's
    // bit 0 is a bare product, which stands as its output there
    let fx = Fixture::new("structarith_booth");
    let cases: Vec<(&str, usize, usize, bool, bool)> = match std::env::var("SAT_TEST_BOOTH_CASES") {
        Ok(spec) => spec.split(';').map(|c| { let f: Vec<&str> = c.split(',').collect(); (match f[0] { "ripple" => "ripple", "kogge-stone" => "kogge-stone", _ => "sklansky" }, f[1].parse().unwrap(), f[2].parse().unwrap(), f[3] == "1", f[4] == "1") }).collect(),
        // (a pruned miter, or a prefix final adder, still declines on
        // these toys: the lone carries of a pruned column multiplier's
        // bottom and the weight of a top column read straight by the
        // final adder's half adder; open, bead SAT-playground-skz.4)
        Err(_) => vec![("ripple", 6usize, 9usize, true, false), ("ripple", 6, 10, false, false), ("ripple", 8, 13, false, false), ("ripple", 8, 12, true, false)],
    };
    for (kind, n, bit, two_input, pruned) in cases {
        let cnf = fx.path(&format!("booth_{}_{}_{}.cnf", kind, n, bit));
        miter_pruned(Miter::Booth(kind), n, bit, two_input, pruned, &cnf);
        if let Ok(dir) = std::env::var("SAT_TEST_KEEP") {
            let _ = std::fs::copy(&cnf, Path::new(&dir).join(format!("booth_{}_{}_{}.cnf", kind, n, bit)));
        }
        let r = run(&cnf, &["--structarith=1"], &[("SAT_STRUCT_DEBUG", "1")]);
        assert_eq!(r.status, "UNSATISFIABLE", "{} {} {}: {}\n{}", kind, n, bit, r.stdout, r.stderr);
        assert!(r.stdout.contains("arithmetic: two adder networks"), "{} {} {}: {}\n{}", kind, n, bit, r.stdout, r.stderr);
        assert!(r.stderr.contains("leaf identities"), "no leaf identity on {} {} {}: {}", kind, n, bit, r.stderr);
        let proof = fx.path(&format!("booth_{}_{}_{}.drat", kind, n, bit));
        let stdout = run_arith_with_proof(&cnf, &proof);
        assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
        assert_eq!(gate(&cnf, &proof), "ok", "{} n={} bit={}: two-stage gate", kind, n, bit);
    }
}

#[test]
fn a_mapped_booth_multiplier_is_read_by_function() {
    // bead SAT-playground-skz.13: no gate computes a row bit; the first
    // full adder reading one takes the select part and the sign straight,
    // its sum and carry one gate each over four signals. The cells come by
    // function over the inputs (row bits from the select gates' truth
    // tables, matched on simulation, checked exactly over a minimum cut)
    let fx = Fixture::new("structarith_booth_mapped");
    for (n, bit) in [(8usize, 12usize), (8, 13)] {
        let cnf = fx.path(&format!("booth_mapped_{}_{}.cnf", n, bit));
        miter_pruned(Miter::BoothMapped, n, bit, false, false, &cnf);
        if let Ok(dir) = std::env::var("SAT_TEST_KEEP") {
            let _ = std::fs::copy(&cnf, Path::new(&dir).join(format!("booth_mapped_{}_{}.cnf", n, bit)));
        }
        let r = run(&cnf, &["--structarith=1"], &[("SAT_STRUCT_DEBUG", "1")]);
        assert_eq!(r.status, "UNSATISFIABLE", "{} {}: {}\n{}", n, bit, r.stdout, r.stderr);
        assert!(r.stderr.contains("cells by function"), "the cells by function did not run on {} {}: {}", n, bit, r.stderr);
        assert!(r.stdout.contains("arithmetic: two adder networks"), "{} {}: {}\n{}", n, bit, r.stdout, r.stderr);
        let proof = fx.path(&format!("booth_mapped_{}_{}.drat", n, bit));
        let stdout = run_arith_with_proof(&cnf, &proof);
        assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
        assert_eq!(gate(&cnf, &proof), "ok", "mapped Booth n={} bit={}: two-stage gate", n, bit);
    }
}

#[test]
fn two_mapped_booth_sides_are_both_read_by_function() {
    // both sides by function: their structural cells are dropped before
    // either side's new cells are placed, so the cells' indices and
    // columns stay theirs (review, 2026-10-24: a second side's removal
    // shifted the first side's cells under their recorded columns). The
    // answer must be right; when the pass fires, its proof must pass
    let fx = Fixture::new("structarith_booth_mapped_both");
    let (n, bit) = (8usize, 12usize);
    let cnf = fx.path("booth_mapped_both.cnf");
    miter_pruned(Miter::BoothMappedBoth, n, bit, false, false, &cnf);
    if let Ok(dir) = std::env::var("SAT_TEST_KEEP") {
        let _ = std::fs::copy(&cnf, Path::new(&dir).join("booth_mapped_both.cnf"));
    }
    let r = run(&cnf, &["--structarith=1"], &[("SAT_STRUCT_DEBUG", "1")]);
    assert_eq!(r.status, "UNSATISFIABLE", "{}\n{}", r.stdout, r.stderr);
    assert_eq!(r.stderr.matches("cells by function (").count(), 2, "both sides by function: {}", r.stderr);
    assert!(!r.stderr.contains("contradict the cells before it"), "a cell by function rejected for its weights: {}", r.stderr);
    if r.stdout.contains("arithmetic: two adder networks") {
        let proof = fx.path("booth_mapped_both.drat");
        let stdout = run_arith_with_proof(&cnf, &proof);
        assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
        assert_eq!(gate(&cnf, &proof), "ok", "two mapped Booth sides: two-stage gate");
    }
}

#[test]
fn booth_wallace_shapes_are_refuted_with_checked_proofs() {
    // two shapes of the 2025 Booth Wallace cells: a row's +neg added in a
    // half adder whose sum is the raw bit again (xor(xnor(raw, neg), neg):
    // structural hashing collapses the double xor, and the half adder is
    // read the other way round, the row bit and neg its leaves), and a
    // carry read complemented at its column and plain one column up (the
    // upper reader reads it doubled)
    let fx = Fixture::new("structarith_booth_shapes");
    for (n, bit, ha, doubled) in [(6usize, 9usize, true, None), (6, 9, false, Some((5usize, true))), (8, 12, true, Some((6, true)))] {
        let tag = format!("{}_{}_{}_{}", n, bit, ha, doubled.map_or("none".to_string(), |(k, _)| k.to_string()));
        let cnf = fx.path(&format!("shapes_{}.cnf", tag));
        miter_pruned(Miter::BoothShapes("ripple", ha, doubled), n, bit, true, false, &cnf);
        if let Ok(dir) = std::env::var("SAT_TEST_KEEP") {
            let _ = std::fs::copy(&cnf, Path::new(&dir).join(format!("booth_shapes_{}.cnf", tag)));
        }
        let r = run(&cnf, &["--structarith=1"], &[("SAT_STRUCT_DEBUG", "1")]);
        assert_eq!(r.status, "UNSATISFIABLE", "n={} bit={} ha={} doubled={:?}: {}\n{}", n, bit, ha, doubled, r.stdout, r.stderr);
        assert!(r.stdout.contains("arithmetic: two adder networks"), "n={} bit={} ha={} doubled={:?}: {}", n, bit, ha, doubled, r.stderr);
        if ha {
            assert!(r.stderr.contains("read the other way round"), "no re-read half adder: {}", r.stderr);
        }
        if doubled.is_some() {
            assert!(r.stderr.contains("doubled, a column above"), "no doubled read: {}", r.stderr);
        }
        let proof = fx.path(&format!("shapes_{}.drat", tag));
        let stdout = run_arith_with_proof(&cnf, &proof);
        assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
        assert_eq!(gate(&cnf, &proof), "ok", "n={} bit={} ha={} doubled={:?}: two-stage gate", n, bit, ha, doubled);
    }
}

#[test]
fn an_xor_of_an_xor_through_a_merged_copy_keeps_a_checked_proof() {
    // p and q two copies of x xor z, g = q xor y, v = p xor g: v is y, but
    // only through the merge of p and q; side A reads v where side B reads
    // the product y, and the proof has to pass the gate (review,
    // 2026-10-23). (Gate recovery leaves v undefined here, so this checks
    // the miter end to end; the collapse rule itself, own inputs only, is
    // the unit test an_xor_of_an_xor_collapses_over_its_own_inputs_only)
    let fx = Fixture::new("structarith_xor_copy");
    let cnf = fx.path("xor_copy.cnf");
    {
        let mut c = Circuit::new();
        let n = 4usize;
        let a: Vec<i64> = (0..n).map(|_| c.var()).collect();
        let b: Vec<i64> = (0..n).map(|_| c.var()).collect();
        let x = c.var();
        let z = c.var();
        let pp: Vec<Vec<i64>> = (0..n).map(|i| (0..n).map(|j| c.and2(a[i], b[j])).collect()).collect();
        let y = pp[1][2];
        let p = c.xor2(x, z);
        let q = c.xor2(x, z);
        let g = c.xor2(q, y);
        let v = c.xor2(p, g);
        let mut ppa = pp.clone();
        ppa[1][2] = v;
        let oa = column_multiplier(&mut c, &ppa, false, None, 2 * n);
        let ob = column_multiplier(&mut c, &pp, false, None, 0);
        let bit = 5;
        let m = c.xor2(oa[bit], ob[bit]);
        c.clauses.push(vec![m]);
        c.write(&cnf);
    }
    if let Ok(dir) = std::env::var("SAT_TEST_KEEP") {
        let _ = std::fs::copy(&cnf, Path::new(&dir).join("xor_copy.cnf"));
    }
    let r = run(&cnf, &["--structarith=1"], &[("SAT_STRUCT_DEBUG", "1")]);
    assert_eq!(r.status, "UNSATISFIABLE", "{}", r.stdout);
    let proof = fx.path("xor_copy.drat");
    let stdout = run_arith_with_proof(&cnf, &proof);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok", "two-stage gate");
}

#[test]
fn a_doubled_carry_without_its_constant_stays_satisfiable() {
    // the carry read at two columns, but the constant the rewriting needs
    // left out: the Booth side is off by 2^(J+1), the miter satisfiable,
    // and the pass must not derive the bits' equality
    let fx = Fixture::new("structarith_doubled_wrong");
    let cnf = fx.path("doubled_wrong.cnf");
    miter_pruned(Miter::BoothShapes("ripple", false, Some((5, false))), 6, 9, true, false, &cnf);
    let r = run(&cnf, &["--structarith=1"], &[("SAT_STRUCT_DEBUG", "1")]);
    assert_eq!(r.status, "SATISFIABLE", "{}\n{}", r.stdout, r.stderr);
    assert!(!r.stdout.contains("arithmetic: two adder networks"), "the pass fired on a wrong multiplier: {}", r.stdout);
}

#[test]
fn a_booth_multiplier_with_a_wrong_sign_constant_stays_satisfiable() {
    // the sign-extension constant short of one bit: the two sides compute
    // different functions, and the pass must not derive their equality
    let fx = Fixture::new("structarith_booth_wrong");
    let cnf = fx.path("booth_wrong.cnf");
    {
        let mut c = Circuit::new();
        let n = 6usize;
        let a: Vec<i64> = (0..n).map(|_| c.var()).collect();
        let b: Vec<i64> = (0..n).map(|_| c.var()).collect();
        let pp: Vec<Vec<i64>> = (0..n).map(|i| (0..n).map(|j| c.and2(a[i], b[j])).collect()).collect();
        let oa = booth_multiplier(&mut c, &a, &b, true, "ripple");
        // the fault: one constant one of the Booth side spoilt (its
        // complemented half adder's and reading one input plain)
        // (the and of the complemented half adder: three clauses over two
        // negative inputs; its first input read plain)
        let gi = c.gates.iter().position(|g| g.1.len() == 2 && g.1[0] < 0 && g.1[1] < 0 && g.3 - g.2 == 3).expect("a complemented half adder's and");
        let (_, ins, start, end) = c.gates[gi].clone();
        let x = ins[0];
        for i in start..end {
            for l in c.clauses[i].iter_mut() {
                if l.abs() == x.abs() {
                    *l = -*l;
                }
            }
        }
        let ob = column_multiplier(&mut c, &pp, true, None, 2 * n);
        let bit = 9;
        let x = c.xor2(oa[bit], ob[bit]);
        c.clauses.push(vec![x]);
        c.write(&cnf);
    }
    let r = run(&cnf, &["--structarith=1"], &[("SAT_STRUCT_DEBUG", "1")]);
    assert_eq!(r.status, "SATISFIABLE", "{}\n{}", r.stdout, r.stderr);
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
