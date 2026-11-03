//! The count reader (src/cardread.rs, bead SAT-playground-iin.7): every form reads
//! with the right n and bound, its proof goes through VeriPB, the formula plus the
//! count's negation is unsatisfiable, a broken copy does not read as the same
//! bound, a fuzz of mutated encodings never panics, and the read budget holds.
//! `tools/card_read_check.py` is the long version of all of this.

mod common;

use common::{run_shipped, solver_bin, Fixture, SOLVER_ENV_VARS, STOCK_PATH};
use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

fn veripb() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/checkers/VeriPB/target/release/veripb");
    p.exists().then_some(p)
}

/// A CNF being written.
#[derive(Clone, Default)]
struct Enc {
    nv: i32,
    clauses: Vec<Vec<i32>>,
}

impl Enc {
    fn new(nv: i32) -> Enc {
        Enc { nv, clauses: Vec::new() }
    }
    fn var(&mut self) -> i32 {
        self.nv += 1;
        self.nv
    }
    fn add(&mut self, c: &[i32]) {
        self.clauses.push(c.to_vec());
    }
    fn write(&self, path: &Path) {
        let mut out = String::new();
        out.push_str(&format!("p cnf {} {}\n", self.nv.max(1), self.clauses.len()));
        for c in &self.clauses {
            for l in c {
                out.push_str(&format!("{} ", l));
            }
            out.push_str("0\n");
        }
        fs::write(path, out).unwrap();
    }
}

/// The Sinz counter: at most k of `x`, a register column per input but the last.
fn sinz(e: &mut Enc, x: &[i32], k: usize, first_units: bool) -> Vec<Vec<i32>> {
    let mut cols: Vec<Vec<i32>> = Vec::new();
    for _ in 0..x.len() - 1 {
        cols.push((0..k).map(|_| e.var()).collect());
    }
    e.add(&[-x[0], cols[0][0]]);
    if first_units {
        for j in 1..k {
            e.add(&[-cols[0][j]]);
        }
    }
    for i in 1..x.len() - 1 {
        e.add(&[-x[i], cols[i][0]]);
        e.add(&[-cols[i - 1][0], cols[i][0]]);
        for j in 1..k {
            e.add(&[-x[i], -cols[i - 1][j - 1], cols[i][j]]);
            e.add(&[-cols[i - 1][j], cols[i][j]]);
        }
        e.add(&[-x[i], -cols[i - 1][k - 1]]);
    }
    e.add(&[-x[x.len() - 1], -cols[x.len() - 2][k - 1]]);
    cols
}

/// A totalizer node's outputs up to `c`; overflow clauses when `capped`.
fn merge(e: &mut Enc, a: &[i32], b: &[i32], c: usize, capped: bool) -> Vec<i32> {
    let c = c.min(a.len() + b.len());
    let r: Vec<i32> = (0..c).map(|_| e.var()).collect();
    for i in 0..=a.len() {
        for j in 0..=b.len() {
            let t = i + j;
            if t == 0 {
                continue;
            }
            let mut cl: Vec<i32> = Vec::new();
            if i > 0 {
                cl.push(-a[i - 1]);
            }
            if j > 0 {
                cl.push(-b[j - 1]);
            }
            if t <= c {
                cl.push(r[t - 1]);
                e.clauses.push(cl);
            } else if t == c + 1 && capped {
                e.clauses.push(cl);
            }
        }
    }
    r
}

fn totalizer(e: &mut Enc, x: &[i32], c: usize, capped: bool) -> Vec<i32> {
    if x.len() == 1 {
        return vec![x[0]];
    }
    let m = x.len() / 2;
    let a = totalizer(e, &x[..m], c, capped);
    let b = totalizer(e, &x[m..], c, capped);
    merge(e, &a, &b, c, capped)
}

/// Totalizer leaves of `t` inputs written out: every m-subset implies output m.
fn direct(e: &mut Enc, x: &[i32], c: usize) -> Vec<i32> {
    let c = c.min(x.len());
    let r: Vec<i32> = (0..c).map(|_| e.var()).collect();
    for mask in 1u32..(1 << x.len()) {
        let m = mask.count_ones() as usize;
        if m > c {
            continue;
        }
        let mut cl: Vec<i32> = (0..x.len()).filter(|&b| mask >> b & 1 == 1).map(|b| -x[b]).collect();
        cl.push(r[m - 1]);
        e.clauses.push(cl);
    }
    r
}

/// A one-way counter: registers mean at least j of the first i, in one direction.
fn one_way(e: &mut Enc, x: &[i32], k: usize, binary_hi: bool) -> i32 {
    let n = x.len();
    let mut s: Vec<Vec<i32>> = vec![vec![0; k + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=k.min(i) {
            s[i][j] = e.var();
        }
    }
    for i in 1..=n {
        for j in 1..=k.min(i) {
            let v = s[i][j];
            let lo = if i > 1 { s[i - 1][j] } else { 0 };
            if lo == 0 {
                e.add(&[-v, x[i - 1]]);
            } else {
                e.add(&[-v, lo, x[i - 1]]);
            }
            if j > 1 {
                let hi = s[i - 1][j - 1];
                if lo == 0 || binary_hi {
                    e.add(&[-v, hi]);
                } else {
                    e.add(&[-v, lo, hi]);
                }
            }
        }
    }
    e.add(&[s[n][k]]);
    s[n][k]
}

fn subset(e: &mut Enc, lits: &[i32], m: usize) {
    for mask in 1u32..(1 << lits.len()) {
        if mask.count_ones() as usize == m {
            let cl: Vec<i32> = (0..lits.len()).filter(|&b| mask >> b & 1 == 1).map(|b| lits[b]).collect();
            e.clauses.push(cl);
        }
    }
}

/// A decision diagram for at least k of `x`: node (i, c) decides x[i] with c still
/// wanted. Returns the root, asserted.
fn diagram(e: &mut Enc, x: &[i32], k: usize, third: bool, both_ways: bool) -> i32 {
    #[derive(Clone, Copy, PartialEq)]
    enum K {
        T,
        F,
        N(i32),
    }
    fn node(e: &mut Enc, x: &[i32], memo: &mut std::collections::HashMap<(usize, usize), K>, i: usize, c: usize, third: bool, both: bool) -> K {
        if c == 0 {
            return K::T;
        }
        if c > x.len() - i {
            return K::F;
        }
        if let Some(&q) = memo.get(&(i, c)) {
            return q;
        }
        let hi = node(e, x, memo, i + 1, c - 1, third, both);
        let lo = node(e, x, memo, i + 1, c, third, both);
        let v = e.var();
        let y = x[i];
        match hi {
            K::T => {}
            K::F => e.add(&[-v, -y]),
            K::N(h) => e.add(&[-v, -y, h]),
        }
        match lo {
            K::T => {}
            K::F => e.add(&[-v, y]),
            K::N(l) => e.add(&[-v, y, l]),
        }
        if third {
            match (hi, lo) {
                (K::N(h), K::N(l)) => e.add(&[-v, h, l]),
                (K::N(h), K::F) => e.add(&[-v, h]),
                (K::F, K::N(l)) => e.add(&[-v, l]),
                _ => {}
            }
        }
        if both {
            match hi {
                K::F => {}
                K::T => e.add(&[v, -y]),
                K::N(h) => e.add(&[v, -y, -h]),
            }
            match lo {
                K::F => {}
                K::T => e.add(&[v, y]),
                K::N(l) => e.add(&[v, y, -l]),
            }
        }
        memo.insert((i, c), K::N(v));
        K::N(v)
    }
    let mut memo = std::collections::HashMap::new();
    match node(e, x, &mut memo, 0, k, third, both_ways) {
        K::N(r) => {
            e.add(&[r]);
            r
        }
        _ => panic!("a diagram with no root"),
    }
}

fn full_adder(e: &mut Enc, a: i32, b: i32, c: i32, both: bool) -> (i32, i32) {
    let (s, m) = (e.var(), e.var());
    for q in [[-a, -b, m], [-a, -c, m], [-b, -c, m]] {
        e.add(&q);
    }
    for q in [[-a, b, c, s], [a, -b, c, s], [a, b, -c, s], [-a, -b, -c, s]] {
        e.add(&q);
    }
    if both {
        for q in [[a, b, -m], [a, c, -m], [b, c, -m]] {
            e.add(&q);
        }
        for q in [[a, b, c, -s], [-a, -b, c, -s], [-a, b, -c, -s], [a, -b, -c, -s]] {
            e.add(&q);
        }
    }
    (s, m)
}

fn half_adder(e: &mut Enc, a: i32, b: i32, both: bool) -> (i32, i32) {
    let (s, m) = (e.var(), e.var());
    for q in [[-a, -b, m], [-a, b, s], [a, -b, s]] {
        e.add(&q);
    }
    if both {
        e.add(&[a, -m]);
        e.add(&[b, -m]);
        e.add(&[a, b, -s]);
        e.add(&[-a, -b, -s]);
    }
    (s, m)
}

/// The bits of the number of true literals of `x`, the low bit first.
fn popcount(e: &mut Enc, x: &[i32], both: bool) -> Vec<i32> {
    let mut cols: Vec<Vec<i32>> = vec![x.to_vec()];
    let mut bits = Vec::new();
    let mut w = 0;
    while w < cols.len() {
        let mut col = std::mem::take(&mut cols[w]);
        let mut up = Vec::new();
        while col.len() > 2 {
            let (a, b, c) = (col.pop().unwrap(), col.pop().unwrap(), col.pop().unwrap());
            let (s, m) = full_adder(e, a, b, c, both);
            col.insert(0, s);
            up.push(m);
        }
        if col.len() == 2 {
            let (a, b) = (col.pop().unwrap(), col.pop().unwrap());
            let (s, m) = half_adder(e, a, b, both);
            col.push(s);
            up.push(m);
        }
        bits.push(col[0]);
        if !up.is_empty() {
            if w + 1 == cols.len() {
                cols.push(Vec::new());
            }
            cols[w + 1].extend(up);
        }
        w += 1;
    }
    bits
}

fn compare_at_most(e: &mut Enc, bits: &[i32], k: usize) {
    for i in 0..bits.len() {
        if (k >> i) & 1 == 0 {
            let mut c = vec![-bits[i]];
            c.extend((i + 1..bits.len()).filter(|&h| (k >> h) & 1 == 1).map(|h| -bits[h]));
            e.add(&c);
        }
    }
}

fn compare_at_least(e: &mut Enc, bits: &[i32], k: usize) {
    for i in 0..bits.len() {
        if (k >> i) & 1 == 1 {
            let mut c = vec![bits[i]];
            c.extend((i + 1..bits.len()).filter(|&h| (k >> h) & 1 == 0).map(|h| bits[h]));
            e.add(&c);
        }
    }
}

/// Batcher's odd-even merge, of two sorted lists, the largest first.
fn merge_lists(e: &mut Enc, a: &[i32], b: &[i32], gates: &mut Vec<[i32; 4]>) -> Vec<i32> {
    if a.is_empty() {
        return b.to_vec();
    }
    if b.is_empty() {
        return a.to_vec();
    }
    if a.len() == 1 && b.len() == 1 {
        let (h, l) = (e.var(), e.var());
        gates.push([a[0], b[0], h, l]);
        return vec![h, l];
    }
    let every = |v: &[i32], from: usize| -> Vec<i32> { v.iter().skip(from).step_by(2).copied().collect() };
    let odd = merge_lists(e, &every(a, 0), &every(b, 0), gates);
    let even = merge_lists(e, &every(a, 1), &every(b, 1), gates);
    let mut out = vec![odd[0]];
    let (mut i, mut j) = (1, 0);
    while i < odd.len() && j < even.len() {
        let (h, l) = (e.var(), e.var());
        gates.push([even[j], odd[i], h, l]);
        out.push(h);
        out.push(l);
        i += 1;
        j += 1;
    }
    out.extend_from_slice(&odd[i..]);
    out.extend_from_slice(&even[j..]);
    out
}

fn sort_net(e: &mut Enc, x: &[i32], gates: &mut Vec<[i32; 4]>) -> Vec<i32> {
    if x.len() == 1 {
        return x.to_vec();
    }
    let m = x.len() / 2;
    let a = sort_net(e, &x[..m], gates);
    let b = sort_net(e, &x[m..], gates);
    merge_lists(e, &a, &b, gates)
}

/// A sorting network's clauses, one way or both, and its outputs, the largest first.
fn network(e: &mut Enc, x: &[i32], both: bool) -> Vec<i32> {
    let mut gates = Vec::new();
    let out = sort_net(e, x, &mut gates);
    for [a, b, h, l] in gates {
        e.add(&[-a, h]);
        e.add(&[-b, h]);
        e.add(&[-a, -b, l]);
        if both {
            e.add(&[-h, a, b]);
            e.add(&[-l, a]);
            e.add(&[-l, b]);
        }
    }
    out
}

/// A small generator, so a failure repeats.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

struct Count {
    form: String,
    lits: Vec<i32>,
    need: i32,
    id: u64,
    lines: u64,
}

struct Read {
    counts: Vec<Count>,
    unproved: u64,
    ticks: u64,
    code: i32,
    stdout: String,
}

fn read(cnf: &Path, pbp: &Path, env: &[(&str, &str)]) -> Read {
    let mut cmd = Command::new(solver_bin());
    cmd.arg("-q").args(STOCK_PATH).arg("--card=1").arg("--conflicts=0").arg(cnf);
    for key in SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    cmd.env("SAT_CARD_READ_CHECK", pbp);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut r = Read { counts: Vec::new(), unproved: 0, ticks: 0, code: out.status.code().unwrap_or(-1), stdout: String::new() };
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("c cardread ") else { continue };
        if let Some(v) = rest.strip_prefix("unproved=") {
            r.unproved = v.trim().parse().unwrap();
        } else if let Some(v) = rest.strip_prefix("ticks=") {
            r.ticks = v.split_whitespace().next().unwrap().parse().unwrap();
        } else if let Some(v) = rest.strip_prefix("proved ") {
            // the count, its proof's id and lines, then what the proof wrote against
            // what the proof budget planned for it
            let words: Vec<&str> = v.split_whitespace().collect();
            let t: Vec<u64> = words[..3].iter().map(|w| w.parse().unwrap()).collect();
            r.counts[t[0] as usize].id = t[1];
            r.counts[t[0] as usize].lines = t[2];
            let written: u64 = words[3].strip_prefix("written=").unwrap().parse().unwrap();
            let planned: u64 = words[4].strip_prefix("planned=").unwrap().parse().unwrap();
            assert!(written <= planned, "count {}: {} terms written, {} planned", t[0], written, planned);
        } else {
            let (form, rest) = rest.split_once(" n=").unwrap();
            let (_, rest) = rest.split_once(" need=").unwrap();
            let (need, rest) = rest.split_once(" lits=[").unwrap();
            let lits: Vec<i32> = rest.trim_end_matches(']').split(", ").filter(|t| !t.is_empty()).map(|t| t.parse().unwrap()).collect();
            r.counts.push(Count { form: form.to_string(), lits, need: need.parse().unwrap(), id: 0, lines: 0 });
        }
    }
    r.stdout = stdout;
    r
}

fn lit_str(l: i32) -> String {
    if l > 0 {
        format!("x{}", l)
    } else {
        format!("~x{}", -l)
    }
}

/// VeriPB on the reader's lines, with an `e` line per count saying the returned id
/// is exactly that count. The grid reader's lines come after them and are cut off.
fn veripb_accepts(cnf: &Path, pbp: &Path, clauses: usize, r: &Read) -> Result<(), String> {
    let Some(bin) = veripb() else {
        eprintln!("VeriPB is not built: the proof lines are not checked");
        return Ok(());
    };
    if r.counts.is_empty() {
        return Ok(());
    }
    let last = r.counts.iter().map(|c| c.id).max().unwrap();
    let mut id = clauses as u64;
    let mut in_block = false;
    let cut = pbp.with_extension("cut");
    let mut out = std::io::BufWriter::new(fs::File::create(&cut).unwrap());
    for line in BufReader::new(fs::File::open(pbp).unwrap()).lines() {
        let line = line.unwrap();
        let s = line.trim();
        if s.starts_with("output NONE") || s.starts_with("conclusion") || s.starts_with("end pseudo") || (!in_block && id >= last) {
            break;
        }
        writeln!(out, "{}", line).unwrap();
        if s.starts_with("pbc") {
            in_block = true;
            id += 1;
        } else if s == "qed;" {
            in_block = false;
            id += 1;
        } else if s.starts_with("pol") || s.starts_with("rup") || s.starts_with("red") {
            id += 1;
        }
    }
    for c in &r.counts {
        let terms: Vec<String> = c.lits.iter().map(|&l| format!("1 {}", lit_str(l))).collect();
        writeln!(out, "e {} >= {} : {} ;", terms.join(" "), c.need, c.id).unwrap();
    }
    writeln!(out, "output NONE ;\nconclusion NONE ;\nend pseudo-Boolean proof ;").unwrap();
    drop(out);
    let run = Command::new(bin).arg(cnf).arg(&cut).output().expect("run veripb");
    let text = String::from_utf8_lossy(&run.stdout).into_owned();
    if text.contains("s VERIFIED NO CONCLUSION") {
        Ok(())
    } else {
        Err(format!("{}{}", text, String::from_utf8_lossy(&run.stderr)))
    }
}

/// Is "at least need of lits" implied? The opposite count by a plain counter,
/// decided by the stock path.
fn implied(e: &Enc, lits: &[i32], need: i32, path: &Path) -> bool {
    let mut f = e.clone();
    let k = (need - 1) as usize;
    for l in lits {
        f.nv = f.nv.max(l.abs());
    }
    if k == 0 {
        for &l in lits {
            f.add(&[-l]);
        }
    } else {
        // at most k of lits, the registers one way
        let mut prev: Vec<i32> = Vec::new();
        for (i, &l) in lits.iter().enumerate() {
            let cur: Vec<i32> = (0..k).map(|_| f.var()).collect();
            f.add(&[-l, cur[0]]);
            if i > 0 {
                for j in 0..k {
                    f.add(&[-prev[j], cur[j]]);
                    if j > 0 {
                        f.add(&[-l, -prev[j - 1], cur[j]]);
                    }
                }
                f.add(&[-l, -prev[k - 1]]);
            }
            prev = cur;
        }
    }
    f.write(path);
    let mut cmd = Command::new(solver_bin());
    cmd.arg("-q").arg("-n").arg("--card=0").args(STOCK_PATH).arg(path);
    for key in SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    cmd.output().expect("run sat-solver").status.code() == Some(20)
}

/// Everything that must hold of any count on any formula.
fn check_counts(e: &Enc, fx: &Fixture, name: &str, r: &Read, cnf: &Path, pbp: &Path) {
    assert!(matches!(r.code, 0 | 10 | 20), "{}: exit code {}\n{}", name, r.code, r.stdout);
    assert_eq!(r.unproved, 0, "{}: unproved counts", name);
    for c in &r.counts {
        let vars: BTreeSet<i32> = c.lits.iter().map(|l| l.abs()).collect();
        assert_eq!(vars.len(), c.lits.len(), "{}: a variable twice in {:?}", name, c.lits);
        assert!(c.need >= 1 && (c.need as usize) < c.lits.len(), "{}: need {} of {}", name, c.need, c.lits.len());
        assert!(c.id != 0, "{}: no proof id", name);
        assert!(implied(e, &c.lits, c.need, &fx.path("sem.cnf")), "{}: not implied: {} need {} of {:?}", name, c.form, c.need, c.lits);
    }
    veripb_accepts(cnf, pbp, e.clauses.len(), r).unwrap_or_else(|why| panic!("{}: VeriPB: {}", name, why));
}

fn run_one(e: &Enc, fx: &Fixture, name: &str) -> Read {
    let cnf = fx.path("f.cnf");
    let pbp = fx.path("f.pbp");
    e.write(&cnf);
    let r = read(&cnf, &pbp, &[]);
    check_counts(e, fx, name, &r, &cnf, &pbp);
    r
}

fn has(r: &Read, lits: &[i32], need: i32) -> bool {
    let want: BTreeSet<i32> = lits.iter().copied().collect();
    r.counts.iter().any(|c| c.need == need && c.lits.iter().copied().collect::<BTreeSet<i32>>() == want)
}

/// The same toy with variables renamed, and with inputs or registers negated.
fn variants(e: &Enc, inputs: &[i32], rng: &mut Rng) -> Vec<(Enc, Vec<i32>, Vec<bool>)> {
    let mut out = Vec::new();
    for (flip_inputs, flip_regs) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut perm: Vec<i32> = (1..=e.nv).collect();
        rng.shuffle(&mut perm);
        let mut flip: Vec<bool> = vec![flip_regs; e.nv as usize + 1];
        for &x in inputs {
            flip[x.unsigned_abs() as usize] = flip_inputs;
        }
        let map = |l: i32| -> i32 {
            let v = l.unsigned_abs() as usize;
            let w = perm[v - 1];
            if (l > 0) != flip[v] {
                w
            } else {
                -w
            }
        };
        let mut f = Enc::new(e.nv);
        for c in &e.clauses {
            let mut cl: Vec<i32> = c.iter().map(|&l| map(l)).collect();
            rng.shuffle(&mut cl);
            f.clauses.push(cl);
        }
        rng.shuffle(&mut f.clauses);
        out.push((f, perm, flip));
    }
    out
}

fn map_lit(perm: &[i32], flip: &[bool], l: i32) -> i32 {
    let v = l.unsigned_abs() as usize;
    let w = perm[v - 1];
    if (l > 0) != flip[v] {
        w
    } else {
        -w
    }
}

fn expect(e: &Enc, inputs: &[i32], wanted: &[(Vec<i32>, i32)], fx: &Fixture, name: &str, rng: &mut Rng) {
    let r = run_one(e, fx, name);
    for (lits, need) in wanted {
        assert!(has(&r, lits, *need), "{}: need {} of {:?} not read: {:?}", name, need, lits, r.counts.iter().map(|c| (c.form.clone(), c.need, c.lits.len())).collect::<Vec<_>>());
    }
    for (f, perm, flip) in variants(e, inputs, rng) {
        let r = run_one(&f, fx, name);
        for (lits, need) in wanted {
            let mapped: Vec<i32> = lits.iter().map(|&l| map_lit(&perm, &flip, l)).collect();
            assert!(has(&r, &mapped, *need), "{} (renamed): need {} of {:?} not read", name, need, mapped);
        }
    }
}

fn at_most(x: &[i32], k: usize) -> (Vec<i32>, i32) {
    (x.iter().map(|&l| -l).collect(), (x.len() - k) as i32)
}

fn at_least(x: &[i32], k: usize) -> (Vec<i32>, i32) {
    (x.to_vec(), k as i32)
}

#[test]
fn every_form_reads_the_right_bound_and_proves_it() {
    let fx = Fixture::new("cardread_forms");
    let mut rng = Rng(11);
    for (n, k) in [(7usize, 3usize), (12, 5), (9, 1), (5, 4)] {
        let x: Vec<i32> = (1..=n as i32).collect();
        for first_units in [true, false] {
            let mut e = Enc::new(n as i32);
            sinz(&mut e, &x, k, first_units);
            expect(&e, &x, &[at_most(&x, k)], &fx, &format!("Sinz at most {} of {}, units {}", k, n, first_units), &mut rng);
        }
        // a totalizer with outputs up to k + 1 and a unit
        let mut e = Enc::new(n as i32);
        let r = totalizer(&mut e, &x, k + 1, false);
        e.add(&[-r[k]]);
        expect(&e, &x, &[at_most(&x, k)], &fx, &format!("totalizer bounded, at most {} of {}", k, n), &mut rng);
        // capped with overflow clauses
        let mut e = Enc::new(n as i32);
        totalizer(&mut e, &x, k, true);
        expect(&e, &x, &[at_most(&x, k)], &fx, &format!("totalizer capped, at most {} of {}", k, n), &mut rng);
        // the full tree
        let mut e = Enc::new(n as i32);
        let r = totalizer(&mut e, &x, n, false);
        e.add(&[-r[k]]);
        expect(&e, &x, &[at_most(&x, k)], &fx, &format!("totalizer full, at most {} of {}", k, n), &mut rng);
        // backward: at least k, the same clauses over negated literals
        let neg: Vec<i32> = x.iter().map(|&l| -l).collect();
        let mut e = Enc::new(n as i32);
        let r = totalizer(&mut e, &neg, n - k + 1, false);
        e.add(&[-r[n - k]]);
        expect(&e, &x, &[at_least(&x, k)], &fx, &format!("totalizer backward, at least {} of {}", k, n), &mut rng);
        for binary_hi in [false, true] {
            let mut e = Enc::new(n as i32);
            one_way(&mut e, &x, k, binary_hi);
            expect(&e, &x, &[at_least(&x, k)], &fx, &format!("one-way, at least {} of {}, binary {}", k, n, binary_hi), &mut rng);
        }
    }
    // direct leaves of three inputs, as the ITC2021 cells write them
    let x: Vec<i32> = (1..=9).collect();
    let mut e = Enc::new(9);
    let a = direct(&mut e, &x[0..3], 3);
    let b = direct(&mut e, &x[3..6], 3);
    let c = direct(&mut e, &x[6..9], 3);
    let ab = merge(&mut e, &a, &b, 3, false);
    let r = merge(&mut e, &ab, &c, 3, false);
    e.add(&[-r[2]]);
    expect(&e, &x, &[at_most(&x, 2)], &fx, "totalizer with direct leaves, at most 2 of 9", &mut rng);
    for (t, m) in [(5usize, 3usize), (7, 3), (6, 4), (9, 8)] {
        let lits: Vec<i32> = (1..=t as i32).map(|v| if v % 3 == 0 { v } else { -v }).collect();
        let mut e = Enc::new(t as i32);
        subset(&mut e, &lits, m);
        let x: Vec<i32> = (1..=t as i32).collect();
        expect(&e, &x, &[(lits.clone(), (t - m + 1) as i32)], &fx, &format!("every {}-subset of {}", m, t), &mut rng);
    }
}

/// The form a count was read as.
fn form_of(r: &Read, lits: &[i32], need: i32) -> String {
    let want: BTreeSet<i32> = lits.iter().copied().collect();
    r.counts.iter().find(|c| c.need == need && c.lits.iter().copied().collect::<BTreeSet<i32>>() == want).map_or(String::new(), |c| c.form.clone())
}

#[test]
fn a_diagram_reads_with_and_without_the_third_clause_one_way_and_both() {
    let fx = Fixture::new("cardread_diagram");
    let mut rng = Rng(21);
    for (n, k) in [(6usize, 3usize), (10, 4), (12, 11), (16, 2)] {
        let x: Vec<i32> = (1..=n as i32).collect();
        for third in [true, false] {
            for both in [false, true] {
                let mut e = Enc::new(n as i32);
                diagram(&mut e, &x, k, third, both);
                let name = format!("diagram at least {} of {}, third {}, both ways {}", k, n, third, both);
                expect(&e, &x, &[at_least(&x, k)], &fx, &name, &mut rng);
                let r = run_one(&e, &fx, &name);
                let (lits, need) = at_least(&x, k);
                assert_eq!(form_of(&r, &lits, need), "Diagram", "{}", name);
            }
        }
    }
    // inputs in another order than their numbers
    let mut order: Vec<i32> = (1..=12).collect();
    rng.shuffle(&mut order);
    for third in [true, false] {
        let mut e = Enc::new(12);
        diagram(&mut e, &order, 5, third, false);
        expect(&e, &order, &[at_least(&order, 5)], &fx, "diagram, shuffled levels", &mut rng);
    }
    // the unit on another node: fewer inputs, or fewer asked for
    let x: Vec<i32> = (1..=8).collect();
    let mut e = Enc::new(8);
    let root = diagram(&mut e, &x, 4, false, false);
    e.clauses.pop();
    let r = run_one(&e, &fx, "no unit");
    assert!(r.counts.is_empty());
    e.add(&[-root]);
    let r = run_one(&e, &fx, "the root false");
    assert!(!has(&r, &x, 4));
}

#[test]
fn an_adder_network_reads_through_units_and_compare_clauses() {
    let fx = Fixture::new("cardread_adder");
    let mut rng = Rng(31);
    for n in [3usize, 5, 7, 8, 15, 16] {
        let x: Vec<i32> = (1..=n as i32).collect();
        for k in [2usize, n / 2, n - 2] {
            if k < 2 || n - k < 2 {
                continue;
            }
            for both in [true, false] {
                let mut e = Enc::new(n as i32);
                let bits = popcount(&mut e, &x, both);
                compare_at_most(&mut e, &bits, k);
                expect(&e, &x, &[at_most(&x, k)], &fx, &format!("adder at most {} of {}, both ways {}", k, n, both), &mut rng);
            }
            let mut e = Enc::new(n as i32);
            let bits = popcount(&mut e, &x, true);
            compare_at_least(&mut e, &bits, k);
            expect(&e, &x, &[at_least(&x, k)], &fx, &format!("adder at least {} of {}", k, n), &mut rng);
            let mut e = Enc::new(n as i32);
            let bits = popcount(&mut e, &x, true);
            for (i, &b) in bits.iter().enumerate() {
                e.add(&[if (k >> i) & 1 == 1 { b } else { -b }]);
            }
            expect(&e, &x, &[at_least(&x, k), at_most(&x, k)], &fx, &format!("adder exactly {} of {}", k, n), &mut rng);
        }
    }
}

#[test]
fn a_comparator_network_reads_by_its_sum_and_by_its_merges() {
    let fx = Fixture::new("cardread_network");
    let mut rng = Rng(41);
    for n in [4usize, 5, 8, 12, 16] {
        let x: Vec<i32> = (1..=n as i32).collect();
        for k in [2usize, n / 2, n - 2] {
            if k < 2 || n - k < 2 {
                continue;
            }
            for both in [false, true] {
                // one unit: only the merges say it
                let mut e = Enc::new(n as i32);
                let out = network(&mut e, &x, both);
                e.add(&[-out[k]]);
                let name = format!("network at most {} of {}, one unit, both ways {}", k, n, both);
                expect(&e, &x, &[at_most(&x, k)], &fx, &name, &mut rng);
                // every output above the bound false: the sum says it
                let mut e = Enc::new(n as i32);
                let out = network(&mut e, &x, both);
                for &o in &out[k..] {
                    e.add(&[-o]);
                }
                expect(&e, &x, &[at_most(&x, k)], &fx, &format!("network at most {} of {}, all above false, both ways {}", k, n, both), &mut rng);
            }
            let mut e = Enc::new(n as i32);
            let out = network(&mut e, &x, true);
            e.add(&[out[k - 1]]);
            expect(&e, &x, &[at_least(&x, k)], &fx, &format!("network at least {} of {}, both ways", k, n), &mut rng);
        }
    }
}

#[test]
fn a_tree_with_no_bound_reads_nothing() {
    let fx = Fixture::new("cardread_free");
    let x: Vec<i32> = (1..=6).collect();
    let mut e = Enc::new(6);
    totalizer(&mut e, &x, 6, false);
    let r = run_one(&e, &fx, "a full tree, no unit");
    assert!(r.counts.is_empty(), "{:?}", r.counts.iter().map(|c| c.need).collect::<Vec<_>>());
}

#[test]
fn a_broken_copy_does_not_read_as_the_same_bound() {
    let fx = Fixture::new("cardread_broken");
    let x: Vec<i32> = (1..=7).collect();
    // Sinz: every clause dropped in turn. Whatever is read is checked; the bound
    // is read only when it still holds
    let mut good = Enc::new(7);
    sinz(&mut good, &x, 3, true);
    let (lits, need) = at_most(&x, 3);
    assert!(has(&run_one(&good, &fx, "good Sinz"), &lits, need));
    for i in 0..good.clauses.len() {
        let mut e = good.clone();
        e.clauses.remove(i);
        let r = run_one(&e, &fx, &format!("Sinz without clause {}", i));
        if has(&r, &lits, need) {
            assert!(implied(&e, &lits, need, &fx.path("sem.cnf")));
        }
    }
    // a one-way counter, the same
    let mut good = Enc::new(7);
    one_way(&mut good, &x, 3, false);
    let (lits, need) = at_least(&x, 3);
    assert!(has(&run_one(&good, &fx, "good one-way"), &lits, need));
    for i in 0..good.clauses.len() {
        let mut e = good.clone();
        e.clauses.remove(i);
        run_one(&e, &fx, &format!("one-way without clause {}", i));
    }
    // a forward totalizer with a positive unit is no lower bound
    let mut e = Enc::new(6);
    let xs: Vec<i32> = (1..=6).collect();
    let r = totalizer(&mut e, &xs, 6, false);
    e.add(&[r[2]]);
    let (lits, need) = at_least(&xs, 3);
    assert!(!has(&run_one(&e, &fx, "unit r_3 on a forward tree"), &lits, need));
    // children that stop counting short, with no overflow clauses, under a root
    // unit: one child's four inputs satisfy every clause
    let mut e = Enc::new(8);
    let xs: Vec<i32> = (1..=8).collect();
    let a0 = merge(&mut e, &xs[0..1], &xs[1..2], 2, false);
    let a1 = merge(&mut e, &xs[2..3], &xs[3..4], 2, false);
    let b0 = merge(&mut e, &xs[4..5], &xs[5..6], 2, false);
    let b1 = merge(&mut e, &xs[6..7], &xs[7..8], 2, false);
    let a = merge(&mut e, &a0, &a1, 2, false);
    let b = merge(&mut e, &b0, &b1, 2, false);
    let r = merge(&mut e, &a, &b, 4, false);
    e.add(&[-r[3]]);
    let (lits, need) = at_most(&xs, 3);
    assert!(!has(&run_one(&e, &fx, "children hide counts"), &lits, need));
    // one subset missing
    let lits: Vec<i32> = (1..=5).map(|v| -v).collect();
    let mut good = Enc::new(5);
    subset(&mut good, &lits, 3);
    for i in 0..good.clauses.len() {
        let mut e = good.clone();
        e.clauses.remove(i);
        let r = run_one(&e, &fx, "a subset missing");
        assert!(!has(&r, &lits, 3));
    }
    // a diagram, an adder network, a comparator network: every clause dropped in turn
    let x: Vec<i32> = (1..=6).collect();
    for third in [true, false] {
        let mut good = Enc::new(6);
        diagram(&mut good, &x, 3, third, false);
        for i in 0..good.clauses.len() {
            let mut e = good.clone();
            e.clauses.remove(i);
            run_one(&e, &fx, &format!("diagram third {} without clause {}", third, i));
        }
    }
    let mut good = Enc::new(6);
    let bits = popcount(&mut good, &x, false);
    compare_at_most(&mut good, &bits, 2);
    for i in 0..good.clauses.len() {
        let mut e = good.clone();
        e.clauses.remove(i);
        run_one(&e, &fx, &format!("adder without clause {}", i));
    }
    let mut good = Enc::new(6);
    let out = network(&mut good, &x, false);
    good.add(&[-out[2]]);
    for i in 0..good.clauses.len() {
        let mut e = good.clone();
        e.clauses.remove(i);
        run_one(&e, &fx, &format!("network without clause {}", i));
    }
    // a unit moved or a bound off by one is no bound of the old value
    let (lits, need) = at_most(&x, 2);
    let mut e = Enc::new(6);
    let out = network(&mut e, &x, false);
    e.add(&[-out[3]]);
    assert!(!has(&run_one(&e, &fx, "the unit one output down"), &lits, need));
    let mut e = Enc::new(6);
    let bits = popcount(&mut e, &x, false);
    compare_at_most(&mut e, &bits, 3);
    assert!(!has(&run_one(&e, &fx, "the compare one higher"), &lits, need));
    let mut e = Enc::new(6);
    let root = diagram(&mut e, &x, 3, false, false);
    e.clauses.pop();
    e.add(&[root - 1]);
    let (lits3, need3) = at_least(&x, 3);
    assert!(!has(&run_one(&e, &fx, "the unit one node down"), &lits3, need3));
    // a subset clause with a repeated literal is not summed by id
    let mut e = good_subset(&lits_five());
    let first = e.clauses[3][0];
    e.clauses[3].push(first);
    assert!(!has(&run_one(&e, &fx, "a repeated literal"), &lits_five(), 3));
    let mut e = good_subset(&lits_five());
    let mut copy = e.clauses[3].clone();
    let first = copy[0];
    copy.push(first);
    e.clauses.push(copy);
    assert!(has(&run_one(&e, &fx, "a clean copy beside it"), &lits_five(), 3));
}

fn lits_five() -> Vec<i32> {
    (1..=5).map(|v| -v).collect()
}

fn good_subset(lits: &[i32]) -> Enc {
    let mut e = Enc::new(5);
    subset(&mut e, lits, 3);
    e
}

fn random_encoding(rng: &mut Rng) -> Enc {
    let n = 3 + rng.below(10);
    let k = 1 + rng.below(n - 1);
    let x: Vec<i32> = (1..=n as i32).collect();
    let mut e = Enc::new(n as i32);
    match rng.below(10) {
        7 => {
            let mut order = x.clone();
            rng.shuffle(&mut order);
            let third = rng.chance(50);
            let both = rng.chance(30);
            diagram(&mut e, &order, k, third, both);
        }
        8 => {
            let both = rng.chance(50);
            let bits = popcount(&mut e, &x, both);
            if rng.chance(50) {
                compare_at_most(&mut e, &bits, k);
            } else {
                compare_at_least(&mut e, &bits, k);
            }
        }
        9 => {
            let both = rng.chance(30);
            let out = network(&mut e, &x, both);
            e.add(&[-out[k]]);
        }
        0 => {
            sinz(&mut e, &x, k, rng.chance(70));
        }
        1 => {
            let r = totalizer(&mut e, &x, k + 1, false);
            e.add(&[-r[k]]);
        }
        2 => {
            totalizer(&mut e, &x, k, true);
        }
        3 => {
            one_way(&mut e, &x, k, rng.chance(50));
        }
        4 => {
            let t = 4 + rng.below(4);
            let m = 3 + rng.below(t - 3);
            let lits: Vec<i32> = (1..=t as i32).map(|v| if v % 2 == 0 { v } else { -v }).collect();
            e = Enc::new(t as i32);
            subset(&mut e, &lits, m);
        }
        5 => {
            let a = direct(&mut e, &x[..n / 2], 3);
            let b = direct(&mut e, &x[n / 2..], 3);
            let r = merge(&mut e, &a, &b, 3, false);
            if let Some(&o) = r.get(2) {
                e.add(&[-o]);
            }
        }
        _ => {
            // forward and backward over one tree
            let r = totalizer(&mut e, &x, n, false);
            let neg: Vec<i32> = x.iter().map(|&l| -l).collect();
            let q = totalizer(&mut e, &neg, n, false);
            e.add(&[-r[k]]);
            e.add(&[-q[n - k]]);
        }
    }
    e
}

fn mutate(e: &mut Enc, rng: &mut Rng) {
    let lit = |rng: &mut Rng, nv: i32| -> i32 {
        let v = 1 + rng.below(nv as usize) as i32;
        if rng.chance(50) {
            v
        } else {
            -v
        }
    };
    let nv = e.nv.max(1);
    for _ in 0..rng.below(3) {
        if !e.clauses.is_empty() {
            let i = rng.below(e.clauses.len());
            e.clauses.remove(i);
        }
    }
    for _ in 0..rng.below(4) {
        let len = 1 + rng.below(4);
        let c: Vec<i32> = (0..len).map(|_| lit(rng, nv)).collect();
        e.clauses.push(c);
    }
    for _ in 0..rng.below(3) {
        if !e.clauses.is_empty() {
            let c = e.clauses[rng.below(e.clauses.len())].clone();
            e.clauses.push(c);
        }
    }
    if rng.chance(30) && !e.clauses.is_empty() {
        let i = rng.below(e.clauses.len());
        let first = e.clauses[i][0];
        e.clauses[i].push(first);
    }
    if rng.chance(20) {
        let l = lit(rng, nv);
        e.clauses.push(vec![l, -l, lit(rng, nv)]);
    }
    if rng.chance(30) && !e.clauses.is_empty() {
        let i = rng.below(e.clauses.len());
        let at = rng.below(e.clauses[i].len());
        e.clauses[i][at] = lit(rng, nv);
    }
    // renamed variables and flipped signs
    let mut perm: Vec<i32> = (1..=nv).collect();
    rng.shuffle(&mut perm);
    let flip: Vec<bool> = (0..=nv).map(|_| rng.chance(50)).collect();
    for c in e.clauses.iter_mut() {
        for l in c.iter_mut() {
            let v = l.unsigned_abs() as usize;
            let w = perm[v - 1];
            *l = if (*l > 0) != flip[v] { w } else { -w };
        }
        rng.shuffle(c);
    }
    rng.shuffle(&mut e.clauses);
}

#[test]
fn a_fuzz_of_mutated_encodings_reads_only_true_counts() {
    let fx = Fixture::new("cardread_fuzz");
    let mut rng = Rng(2026);
    let mut read_some = 0;
    for case in 0..150 {
        let mut e = random_encoding(&mut rng);
        mutate(&mut e, &mut rng);
        let r = run_one(&e, &fx, &format!("fuzz case {}", case));
        read_some += !r.counts.is_empty() as usize;
    }
    assert!(read_some >= 30, "the fuzz reads too few counts to mean much: {}", read_some);
}

#[test]
fn the_proof_lines_are_what_the_plan_says() {
    let fx = Fixture::new("cardread_lines");
    let x: Vec<i32> = (1..=8).collect();
    let mut toys: Vec<(&str, Enc)> = Vec::new();
    let mut e = Enc::new(8);
    sinz(&mut e, &x, 3, true);
    toys.push(("Sinz", e));
    let mut e = Enc::new(8);
    let r = totalizer(&mut e, &x, 4, false);
    e.add(&[-r[3]]);
    toys.push(("totalizer", e));
    let mut e = Enc::new(8);
    one_way(&mut e, &x, 3, false);
    toys.push(("one-way", e));
    let mut e = Enc::new(8);
    let lits: Vec<i32> = x.iter().map(|&l| -l).collect();
    subset(&mut e, &lits, 4);
    toys.push(("subset", e));
    let mut e = Enc::new(8);
    diagram(&mut e, &x, 3, true, false);
    toys.push(("diagram", e));
    let mut e = Enc::new(8);
    diagram(&mut e, &x, 3, false, true);
    toys.push(("diagram both ways", e));
    let mut e = Enc::new(8);
    let bits = popcount(&mut e, &x, false);
    compare_at_most(&mut e, &bits, 3);
    toys.push(("adder", e));
    let mut e = Enc::new(8);
    let out = network(&mut e, &x, false);
    e.add(&[-out[3]]);
    toys.push(("network, merges", e));
    let mut e = Enc::new(8);
    let out = network(&mut e, &x, false);
    for &o in &out[3..] {
        e.add(&[-o]);
    }
    toys.push(("network, sum", e));
    for (name, e) in toys {
        let r = run_one(&e, &fx, name);
        assert_eq!(r.counts.len(), 1, "{}", name);
        // the reader's own lines: from the header to the last one before the id
        // of its count, counted the way the writer writes them
        let text = fs::read_to_string(fx.path("f.pbp")).unwrap();
        let mut id = e.clauses.len() as u64;
        let mut lines = 0u64;
        let mut in_block = false;
        for line in text.lines().skip(1) {
            let s = line.trim();
            if !in_block && id >= r.counts[0].id {
                // the helpers' deletion closes the proof
                lines += s.starts_with("del") as u64;
                break;
            }
            lines += 1;
            if s.starts_with("pbc") {
                in_block = true;
                id += 1;
            } else if s == "qed;" {
                in_block = false;
                id += 1;
            } else if !s.starts_with("del") {
                id += 1;
            }
        }
        assert_eq!(lines, r.counts[0].lines, "{}: the plan's line count", name);
    }
}

#[test]
fn the_read_budget_stops_on_time_and_twice_the_same() {
    let fx = Fixture::new("cardread_budget");
    let x: Vec<i32> = (1..=60).collect();
    let mut toys: Vec<(&str, Enc)> = Vec::new();
    let mut e = Enc::new(60);
    sinz(&mut e, &x, 8, true);
    toys.push(("Sinz", e));
    let mut e = Enc::new(60);
    let r = totalizer(&mut e, &x, 6, false);
    e.add(&[-r[5]]);
    toys.push(("totalizer", e));
    let mut e = Enc::new(60);
    one_way(&mut e, &x, 6, true);
    toys.push(("one-way", e));
    let mut e = Enc::new(60);
    diagram(&mut e, &x, 10, true, false);
    toys.push(("diagram", e));
    let mut e = Enc::new(60);
    let bits = popcount(&mut e, &x, true);
    compare_at_most(&mut e, &bits, 10);
    toys.push(("adder", e));
    let mut e = Enc::new(60);
    let out = network(&mut e, &x, false);
    e.add(&[-out[3]]);
    toys.push(("network", e));
    // a stray large variable makes every table per variable large
    let mut e = Enc::new(100_000);
    sinz(&mut e, &x, 3, true);
    e.nv = e.nv.max(100_000);
    e.add(&[99_999, 100_000]);
    toys.push(("sparse", e));
    let cnf = fx.path("b.cnf");
    let pbp = fx.path("b.pbp");
    for (name, e) in toys {
        e.write(&cnf);
        let full = read(&cnf, &pbp, &[]);
        let again = read(&cnf, &pbp, &[]);
        assert!(full.ticks > 1000, "{}: {}", name, full.ticks);
        assert_eq!(full.ticks, again.ticks, "{}: two runs", name);
        for percent in [1u64, 10, 25, 50, 75, 90, 99] {
            let limit = (full.ticks * percent / 100).max(1);
            let value = limit.to_string();
            let r = read(&cnf, &pbp, &[("SAT_LIMIT_TICKS", value.as_str())]);
            let s = read(&cnf, &pbp, &[("SAT_LIMIT_TICKS", value.as_str())]);
            assert!(r.ticks <= limit + limit / 10, "{} at {}%: {} ticks over a limit of {}", name, percent, r.ticks, limit);
            assert_eq!(r.ticks, s.ticks, "{} at {}%: two runs", name, percent);
            assert!(matches!(r.code, 0 | 10 | 20), "{} at {}%: exit {}", name, percent, r.code);
            // what a stopped reader keeps is still checked
            assert_eq!(r.unproved, 0);
            veripb_accepts(&cnf, &pbp, e.clauses.len(), &r).unwrap_or_else(|why| panic!("{} at {}%: VeriPB: {}", name, percent, why));
        }
    }
}

#[test]
fn the_reader_stays_out_with_the_option_off_and_runs_with_the_shipped_defaults() {
    let fx = Fixture::new("cardread_off");
    let x: Vec<i32> = (1..=8).collect();
    let mut e = Enc::new(8);
    sinz(&mut e, &x, 3, true);
    let cnf = fx.path("s.cnf");
    e.write(&cnf);
    let pbp = fx.path("s.pbp");
    let hook = pbp.to_string_lossy().into_owned();
    let off = run_shipped(&cnf, &["--card=0"], &[("SAT_CARD_READ_CHECK", hook.as_str())]);
    assert_eq!(off.status, "SATISFIABLE");
    assert!(!off.stdout.contains("c cardread"), "{}", off.stdout);
    assert!(!pbp.exists());
    // no option: the engine is on by default since 2026-11-02
    let on = run_shipped(&cnf, &[], &[("SAT_CARD_READ_CHECK", hook.as_str())]);
    assert_eq!(on.status, "SATISFIABLE");
    assert!(on.stdout.contains("c cardread unproved=0"), "{}", on.stdout);
}
