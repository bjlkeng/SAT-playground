//! The grid reader (src/gridread.rs; epic SAT-playground-iin, bead iin.8): a small
//! version of `tools/grid_read_check.py`. Each encoding of a grid reads the same,
//! gates stay out, a broken copy does not claim what it lost, the proof goes through
//! VeriPB with each returned id checked, each claim is confirmed by the stock solver
//! path, and the read stops at its budget and is the same twice.

mod common;

use common::{solver_bin, Fixture, SOLVER_ENV_VARS, STOCK_PATH};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn veripb() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/checkers/VeriPB/target/release/veripb");
    p.exists().then_some(p)
}

/// A formula being written: variables and clauses.
#[derive(Default, Clone)]
struct Enc {
    n: i32,
    cl: Vec<Vec<i32>>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum How {
    Pairwise,
    Ladder,
    Commander,
}

impl Enc {
    fn var(&mut self) -> i32 {
        self.n += 1;
        self.n
    }

    fn pairwise(&mut self, lits: &[i32]) {
        for (i, &a) in lits.iter().enumerate() {
            for &b in &lits[i + 1..] {
                self.cl.push(vec![-a, -b]);
            }
        }
    }

    fn ladder(&mut self, lits: &[i32]) {
        let s: Vec<i32> = (1..lits.len()).map(|_| self.var()).collect();
        for (i, &x) in lits.iter().enumerate() {
            if i < s.len() {
                self.cl.push(vec![-x, s[i]]);
            }
            if i > 0 {
                self.cl.push(vec![-x, -s[i - 1]]);
                if i < s.len() {
                    self.cl.push(vec![-s[i - 1], s[i]]);
                }
            }
        }
    }

    fn commander(&mut self, lits: &[i32]) {
        let mut cmds = Vec::new();
        for grp in lits.chunks(2) {
            let c = self.var();
            cmds.push(c);
            self.pairwise(grp);
            for &x in grp {
                self.cl.push(vec![-x, c]);
            }
        }
        self.pairwise(&cmds);
    }

    fn amo(&mut self, lits: &[i32], how: How) {
        match how {
            How::Pairwise => self.pairwise(lits),
            How::Ladder => self.ladder(lits),
            How::Commander => self.commander(lits),
        }
    }
}

fn latin(n: usize, how: How) -> Enc {
    let mut e = Enc::default();
    let x: Vec<Vec<Vec<i32>>> = (0..n).map(|_| (0..n).map(|_| (0..n).map(|_| e.var()).collect()).collect()).collect();
    for r in 0..n {
        for c in 0..n {
            e.cl.push(x[r][c].clone());
            e.amo(&x[r][c].clone(), how);
        }
    }
    for r in 0..n {
        for v in 0..n {
            let l: Vec<i32> = (0..n).map(|c| x[r][c][v]).collect();
            e.amo(&l, how);
        }
    }
    for c in 0..n {
        for v in 0..n {
            let l: Vec<i32> = (0..n).map(|r| x[r][c][v]).collect();
            e.amo(&l, how);
        }
    }
    e
}

fn php(pigeons: usize, holes: usize, how: How) -> Enc {
    let mut e = Enc::default();
    let x: Vec<Vec<i32>> = (0..pigeons).map(|_| (0..holes).map(|_| e.var()).collect()).collect();
    for row in &x {
        e.cl.push(row.clone());
    }
    for h in 0..holes {
        let l: Vec<i32> = (0..pigeons).map(|p| x[p][h]).collect();
        e.amo(&l, how);
    }
    e
}

fn queens(n: usize) -> Enc {
    let mut e = Enc::default();
    let q: Vec<Vec<i32>> = (0..n).map(|_| (0..n).map(|_| e.var()).collect()).collect();
    for row in &q {
        e.cl.push(row.clone());
        e.pairwise(row);
    }
    for j in 0..n {
        let l: Vec<i32> = (0..n).map(|i| q[i][j]).collect();
        e.pairwise(&l);
    }
    for k in 0..2 * n - 1 {
        let d: Vec<i32> = (0..n).filter(|&i| k >= i && k - i < n).map(|i| q[i][k - i]).collect();
        e.pairwise(&d);
        let d: Vec<i32> = (0..n).filter(|&i| i + n - 1 >= k && i + n - 1 - k < n).map(|i| q[i][i + n - 1 - k]).collect();
        e.pairwise(&d);
    }
    e
}

fn colouring(edges: &[(usize, usize)], n: usize, colours: usize) -> Enc {
    let mut e = Enc::default();
    let x: Vec<Vec<i32>> = (0..n).map(|_| (0..colours).map(|_| e.var()).collect()).collect();
    for row in &x {
        e.cl.push(row.clone());
    }
    for &(a, b) in edges {
        for c in 0..colours {
            e.cl.push(vec![-x[a][c], -x[b][c]]);
        }
    }
    e
}

fn clique_edges(n: usize) -> Vec<(usize, usize)> {
    (0..n).flat_map(|a| (a + 1..n).map(move |b| (a, b))).collect()
}

fn split_rows(n: usize, parts: usize, per: usize) -> Enc {
    let mut e = Enc::default();
    let d = parts * per;
    let x: Vec<Vec<i32>> = (0..n).map(|_| (0..d).map(|_| e.var()).collect()).collect();
    for row in &x {
        let guards: Vec<i32> = (0..parts).map(|_| e.var()).collect();
        e.cl.push(guards.clone());
        for (k, &g) in guards.iter().enumerate() {
            let mut c = vec![-g];
            c.extend_from_slice(&row[k * per..(k + 1) * per]);
            e.cl.push(c);
        }
    }
    for j in 0..d {
        let l: Vec<i32> = (0..n).map(|i| x[i][j]).collect();
        e.pairwise(&l);
    }
    e
}

fn guarded(n: usize, d: usize) -> Enc {
    let mut e = Enc::default();
    let x: Vec<Vec<i32>> = (0..n).map(|_| (0..d).map(|_| e.var()).collect()).collect();
    for row in &x {
        let g = e.var();
        let mut c = vec![g];
        c.extend_from_slice(row);
        e.cl.push(c);
        e.pairwise(row);
    }
    for j in 0..d {
        let l: Vec<i32> = (0..n).map(|i| x[i][j]).collect();
        e.pairwise(&l);
    }
    e
}

/// Pigeonhole with exclusions that show from one side only.
fn one_way(pigeons: usize, holes: usize) -> Enc {
    let mut e = Enc::default();
    let x: Vec<Vec<i32>> = (0..pigeons).map(|_| (0..holes).map(|_| e.var()).collect()).collect();
    for row in &x {
        e.cl.push(row.clone());
        for &v in row {
            let a = e.var();
            e.cl.push(vec![-v, a]);
        }
    }
    for h in 0..holes {
        for p in 0..pigeons {
            for q in p + 1..pigeons {
                let (a, b) = (e.var(), e.var());
                e.cl.push(vec![-x[p][h], a]);
                e.cl.push(vec![-x[p][h], b]);
                e.cl.push(vec![-a, -b, -x[q][h]]);
            }
        }
    }
    e
}

fn and_chain(n: usize) -> Enc {
    let mut e = Enc::default();
    let ins: Vec<i32> = (0..n).map(|_| e.var()).collect();
    let mut cur = ins[0];
    for &a in &ins[1..] {
        let o = e.var();
        e.cl.push(vec![-o, cur]);
        e.cl.push(vec![-o, a]);
        e.cl.push(vec![o, -cur, -a]);
        cur = o;
    }
    e
}

fn or_gates(n: usize) -> Enc {
    let mut e = Enc::default();
    let ins: Vec<i32> = (0..n).map(|_| e.var()).collect();
    for i in 0..n - 2 {
        let o = e.var();
        e.cl.push(vec![o, -ins[i]]);
        e.cl.push(vec![o, -ins[i + 1]]);
        e.cl.push(vec![-o, ins[i], ins[i + 1]]);
    }
    e
}

fn write_cnf(path: &Path, clauses: &[Vec<i32>], extra_units: &[i32]) {
    let nv = clauses.iter().flatten().chain(extra_units).map(|l| l.abs()).max().unwrap_or(0);
    let mut f = std::io::BufWriter::new(fs::File::create(path).unwrap());
    writeln!(f, "p cnf {} {}", nv, clauses.len() + extra_units.len()).unwrap();
    for c in clauses {
        for l in c {
            write!(f, "{} ", l).unwrap();
        }
        writeln!(f, "0").unwrap();
    }
    for u in extra_units {
        writeln!(f, "{} 0", u).unwrap();
    }
}

struct Grid {
    rows: Vec<Vec<i32>>,
    cols: Vec<Vec<i32>>,
    clauses: Vec<i64>,
    rowlines: Vec<u64>,
    collines: Vec<u64>,
}

impl Grid {
    /// n rows, d columns, values only one row has.
    fn shape(&self) -> (usize, usize, usize) {
        let inside: usize = self.rows.iter().map(|r| r.len()).sum();
        let columns: usize = self.cols.iter().map(|c| c.len()).sum();
        (self.rows.len(), self.cols.len(), inside - columns)
    }
}

struct Read {
    grids: Vec<Grid>,
    unproved: u64,
    begin: u64,
    end: u64,
    rejected: u64,
    code: i32,
    stdout: String,
}

impl Read {
    fn ticks(&self) -> u64 {
        self.end - self.begin
    }
    fn count(&self, shape: (usize, usize, usize)) -> usize {
        self.grids.iter().filter(|g| g.shape() == shape).count()
    }
    fn shapes(&self) -> Vec<(usize, usize, usize)> {
        self.grids.iter().map(|g| g.shape()).collect()
    }
}

/// A list of lists of numbers, as Rust's `{:?}` prints it.
fn lists(text: &str) -> Vec<Vec<i64>> {
    let mut out = Vec::new();
    for part in text.trim().trim_start_matches('[').trim_end_matches(']').split("], [") {
        let part = part.trim_matches(|c| c == '[' || c == ']');
        out.push(part.split(", ").filter(|t| !t.is_empty()).map(|t| t.parse().unwrap()).collect());
    }
    out
}

fn numbers(text: &str) -> Vec<i64> {
    text.trim().trim_matches(|c| c == '[' || c == ']').split(", ").filter(|t| !t.is_empty()).map(|t| t.parse().unwrap()).collect()
}

fn read(cnf: &Path, pbp: &Path, options: &[&str]) -> Read {
    let mut cmd = Command::new(solver_bin());
    cmd.arg("-q").args(STOCK_PATH).arg("--card=1").args(options).arg("--conflicts=0").arg(cnf);
    for key in SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    cmd.env("SAT_CARD_READ_CHECK", pbp).env("SAT_STRUCT_DEBUG", "2");
    let out = cmd.output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let mut r = Read { grids: Vec::new(), unproved: 0, begin: 0, end: 0, rejected: 0, code: out.status.code().unwrap_or(-1), stdout: String::new() };
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("c gridread ") {
            let (_, rest) = rest.split_once("rowlits=").unwrap();
            let (rows, cols) = rest.split_once(" collits=").unwrap();
            let to32 = |v: Vec<Vec<i64>>| -> Vec<Vec<i32>> { v.into_iter().map(|c| c.into_iter().map(|l| l as i32).collect()).collect() };
            r.grids.push(Grid { rows: to32(lists(rows)), cols: to32(lists(cols)), clauses: Vec::new(), rowlines: Vec::new(), collines: Vec::new() });
        } else if let Some(v) = line.strip_prefix("c cardread unproved=") {
            r.unproved = v.trim().parse().unwrap();
        }
    }
    let mut k = 0;
    for line in stderr.lines() {
        if let Some(rest) = line.strip_prefix("gridinfo clauses=") {
            let (clauses, rest) = rest.split_once(" rowlines=").unwrap();
            let (rows, cols) = rest.split_once(" collines=").unwrap();
            let g = &mut r.grids[k];
            g.clauses = numbers(clauses);
            g.rowlines = numbers(rows).into_iter().map(|x| x as u64).collect();
            g.collines = numbers(cols).into_iter().map(|x| x as u64).collect();
            k += 1;
        } else if let Some(rest) = line.strip_prefix("gridread: ") {
            let t: Vec<&str> = rest.split_whitespace().collect();
            for p in t.chunks(2) {
                match p[0] {
                    "begin" => r.begin = p[1].parse().unwrap(),
                    "end" => r.end = p[1].parse().unwrap(),
                    "rejected" => r.rejected = p[1].parse().unwrap(),
                    _ => {}
                }
            }
        }
    }
    assert!(!stderr.contains("panicked"), "{}", stderr);
    r.stdout = stdout;
    r
}

fn lit_text(l: i32) -> String {
    if l > 0 {
        format!("x{}", l)
    } else {
        format!("~x{}", -l)
    }
}

/// The input pair clauses as VeriPB reads them: written with two different literals.
fn pair_ids(clauses: &[Vec<i32>]) -> HashMap<(i32, i32), u64> {
    let mut ids = HashMap::new();
    for (i, c) in clauses.iter().enumerate() {
        if c.len() == 2 && c[0] != c[1] && c[0] != -c[1] {
            let (a, b) = (-c[0], -c[1]);
            ids.entry((a.min(b), a.max(b))).or_insert(i as u64 + 1);
        }
    }
    ids
}

/// The reader's numbers against the CNF: clauses, one literal per row in a column,
/// and the line counts.
fn check_by_hand(name: &str, clauses: &[Vec<i32>], r: &Read) {
    assert!(matches!(r.code, 0 | 10 | 20), "{}: exit code {}", name, r.code);
    assert_eq!(r.unproved, 0, "{}", name);
    assert_eq!(r.rejected, 0, "{}: detection and the check disagree", name);
    let pairs = pair_ids(clauses);
    for g in &r.grids {
        assert!(g.rows.len() >= 3, "{}", name);
        let mut owner: HashMap<i32, usize> = HashMap::new();
        for (ri, row) in g.rows.iter().enumerate() {
            for &l in row {
                owner.insert(l, ri);
            }
            if g.clauses[ri] >= 0 {
                let mut a = clauses[g.clauses[ri] as usize].clone();
                let mut b = row.clone();
                a.sort_unstable();
                b.sort_unstable();
                assert_eq!(a, b, "{}: a row's clause", name);
            }
        }
        for (ci, col) in g.cols.iter().enumerate() {
            assert!(col.len() >= 2, "{}", name);
            let mut rows_here = BTreeSet::new();
            for &l in col {
                assert!(rows_here.insert(owner[&l]), "{}: two literals of one row in a column", name);
            }
            let mut rups = 0;
            for (i, &a) in col.iter().enumerate() {
                for &b in &col[i + 1..] {
                    if !pairs.contains_key(&(a.min(b), a.max(b))) {
                        rups += 1;
                    }
                }
            }
            assert_eq!(g.collines[ci], rups + (col.len() >= 3) as u64, "{}: the column's line count", name);
        }
    }
}

/// VeriPB on the file, with an `e` line per row and column: the id the reader
/// returned is that constraint. The ids are counted from the file.
fn veripb_accepts(name: &str, cnf: &Path, pbp: &Path, clauses: &[Vec<i32>], r: &Read) {
    let Some(bin) = veripb() else {
        eprintln!("VeriPB is not built: the proof lines are not checked");
        return;
    };
    let text = fs::read_to_string(pbp).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let body = &lines[..lines.len() - 3];
    let total: u64 = r.grids.iter().map(|g| g.rowlines.iter().sum::<u64>() + g.collines.iter().sum::<u64>()).sum();
    let start = body.len() - total as usize;
    let mut id = clauses.len() as u64;
    for line in &body[1..start] {
        let s = line.trim();
        if s.starts_with("pbc") || s == "qed;" || s.starts_with("pol") || s.starts_with("rup") || s.starts_with("red") {
            id += 1;
        }
    }
    let pairs = pair_ids(clauses);
    let mut at = start;
    let mut checks: Vec<String> = Vec::new();
    for g in &r.grids {
        for (ri, row) in g.rows.iter().enumerate() {
            let terms: Vec<String> = row.iter().map(|&l| format!("1 {}", lit_text(l))).collect();
            let got = if g.clauses[ri] >= 0 {
                g.clauses[ri] as u64 + 1
            } else {
                at += g.rowlines[ri] as usize;
                id += g.rowlines[ri];
                id
            };
            checks.push(format!("e {} >= 1 : {} ;", terms.join(" "), got));
        }
        for (ci, col) in g.cols.iter().enumerate() {
            let terms: Vec<String> = col.iter().map(|&l| format!("1 {}", lit_text(-l))).collect();
            at += g.collines[ci] as usize;
            id += g.collines[ci];
            let got = if g.collines[ci] == 0 {
                pairs[&(col[0].min(col[1]), col[0].max(col[1]))]
            } else {
                id
            };
            checks.push(format!("e {} >= {} : {} ;", terms.join(" "), col.len() - 1, got));
        }
    }
    assert_eq!(at, body.len(), "{}: the file's lines are what the reader said", name);
    let cut = pbp.with_extension("e");
    let mut out = String::new();
    for l in body {
        out.push_str(l);
        out.push('\n');
    }
    for l in &checks {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("output NONE ;\nconclusion NONE ;\nend pseudo-Boolean proof ;\n");
    fs::write(&cut, out).unwrap();
    let run = Command::new(bin).arg(cnf).arg(&cut).output().expect("run veripb");
    let said = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(said.contains("s VERIFIED NO CONCLUSION"), "{}: VeriPB: {}{}", name, said, String::from_utf8_lossy(&run.stderr));
}

/// The stock solver path on the clauses plus units: unsatisfiable?
fn stock_refutes(clauses: &[Vec<i32>], units: &[i32], path: &Path) -> bool {
    write_cnf(path, clauses, units);
    let mut cmd = Command::new(solver_bin());
    cmd.arg("-q").arg("--card=0").args(STOCK_PATH).arg(path);
    for key in SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    cmd.output().expect("run sat-solver").status.code() == Some(20)
}

/// Each claim decided by the stock path: a pair with both its literals as units, a
/// row with all its literals false.
fn claims_hold(name: &str, clauses: &[Vec<i32>], r: &Read, fx: &Fixture) {
    let path = fx.path("claim.cnf");
    let mut seen: BTreeSet<Vec<i32>> = BTreeSet::new();
    for g in &r.grids {
        for row in &g.rows {
            let units: Vec<i32> = row.iter().map(|&l| -l).collect();
            if seen.insert(units.clone()) {
                assert!(stock_refutes(clauses, &units, &path), "{}: row {:?} does not follow", name, row);
            }
        }
        for col in &g.cols {
            for (i, &a) in col.iter().enumerate() {
                for &b in &col[i + 1..] {
                    if seen.insert(vec![a, b]) {
                        assert!(stock_refutes(clauses, &[a, b], &path), "{}: {} and {} do not exclude each other", name, a, b);
                    }
                }
            }
        }
    }
}

/// Reads a toy and checks all of it. The caller looks at the shapes.
fn read_toy(name: &str, clauses: &[Vec<i32>], fx: &Fixture) -> Read {
    let cnf = fx.path("toy.cnf");
    let pbp = fx.path("toy.pbp");
    write_cnf(&cnf, clauses, &[]);
    let r = read(&cnf, &pbp, &[]);
    check_by_hand(name, clauses, &r);
    veripb_accepts(name, &cnf, &pbp, clauses, &r);
    claims_hold(name, clauses, &r, fx);
    r
}

#[test]
fn a_latin_square_reads_alike_in_three_encodings() {
    let fx = Fixture::new("grid_latin");
    for how in [How::Pairwise, How::Ladder, How::Commander] {
        let r = read_toy(&format!("latin {:?}", how), &latin(3, how).cl, &fx);
        assert_eq!(r.shapes(), vec![(3, 3, 0); 6], "{:?}", how);
    }
}

#[test]
fn flipped_signs_and_new_names_read_the_same() {
    let fx = Fixture::new("grid_flip");
    let mut e = latin(3, How::Commander);
    for c in e.cl.iter_mut() {
        for l in c.iter_mut() {
            // every third variable negated, every number doubled and moved up
            let v = l.abs();
            let sign = if v % 3 == 0 { -l.signum() } else { l.signum() };
            *l = sign * (2 * v + 100);
        }
    }
    let r = read_toy("latin renamed", &e.cl, &fx);
    assert_eq!(r.shapes(), vec![(3, 3, 0); 6]);
}

#[test]
fn every_shape_of_grid_reads_as_it_should() {
    let fx = Fixture::new("grid_shapes");
    for how in [How::Pairwise, How::Ladder, How::Commander] {
        let r = read_toy("pigeonhole", &php(5, 4, how).cl, &fx);
        assert_eq!(r.shapes(), vec![(5, 4, 0)], "{:?}", how);
    }
    assert_eq!(read_toy("queens", &queens(6).cl, &fx).shapes(), vec![(6, 6, 0)]);
    assert_eq!(read_toy("colouring", &colouring(&clique_edges(5), 5, 4).cl, &fx).shapes(), vec![(5, 4, 0)]);
    assert_eq!(read_toy("guarded", &guarded(3, 3).cl, &fx).shapes(), vec![(3, 3, 3)]);
    assert_eq!(read_toy("one way", &one_way(4, 3).cl, &fx).shapes(), vec![(4, 3, 0)]);
    let r = read_toy("split", &split_rows(4, 3, 2).cl, &fx);
    let g = r.grids.iter().find(|g| g.shape() == (4, 6, 0)).unwrap();
    assert!(g.rows.iter().all(|row| row.len() == 6), "each row over its leaves");
    assert!(g.clauses.iter().all(|&c| c == -1), "a resolvent has no clause of its own");
    // a split in two parts, and a chain of three clauses
    let r = read_toy("split in two", &split_rows(4, 2, 2).cl, &fx);
    assert!(r.count((4, 4, 0)) >= 1, "{:?}", r.shapes());
    let g = r.grids.iter().find(|g| g.shape() == (4, 4, 0)).unwrap();
    assert!(g.rows.iter().all(|row| row.len() == 4), "each row over its leaves");
    assert!(g.clauses.iter().all(|&c| c == -1), "a resolvent has no clause of its own");
    // a five-cycle's edges differ but three rows do not make a grid
    let cycle: Vec<(usize, usize)> = (0..5).map(|i| (i, (i + 1) % 5)).collect();
    assert!(read_toy("a five-cycle", &colouring(&cycle, 5, 3).cl, &fx).grids.is_empty());
}

#[test]
fn a_row_the_input_says_only_by_propagation_has_no_clause_of_its_own() {
    let fx = Fixture::new("grid_rows");
    // a unit shortens a row
    let mut e = php(4, 3, How::Pairwise);
    let z = e.var();
    e.cl[0].push(z);
    e.cl.push(vec![-z]);
    let r = read_toy("a unit", &e.cl, &fx);
    assert_eq!(r.shapes(), vec![(4, 3, 0)]);
    assert_eq!(r.grids[0].clauses.iter().filter(|&&c| c == -1).count(), 1);
    // a repeated literal: its constraint has another coefficient
    let mut e = php(4, 3, How::Pairwise);
    let first = e.cl[0][0];
    e.cl[0].insert(0, first);
    let r = read_toy("a repeated literal", &e.cl, &fx);
    assert_eq!(r.shapes(), vec![(4, 3, 0)]);
    assert_eq!(r.grids[0].clauses.iter().filter(|&&c| c == -1).count(), 1);
    // a pair clause with a repeated literal is no pair clause for the `pol` line
    let mut e = php(4, 3, How::Pairwise);
    let i = e.cl.iter().position(|c| c.len() == 2).unwrap();
    let (a, b) = (e.cl[i][0], e.cl[i][1]);
    e.cl[i] = vec![a, a, b];
    let r = read_toy("a repeated literal in a pair", &e.cl, &fx);
    assert_eq!(r.shapes(), vec![(4, 3, 0)]);
    assert_eq!(r.grids[0].collines.iter().sum::<u64>(), 3 + 1, "one `rup` line, three `pol` lines");
    // two literals of one clause fail alone: their proofs come first
    let mut e = php(4, 3, How::Pairwise);
    let (f1, f2, g) = (e.var(), e.var(), e.var());
    e.cl[0].extend_from_slice(&[f1, f2]);
    e.cl.extend_from_slice(&[vec![-f1, g], vec![-f1, -g], vec![-f2, g], vec![-f2, -g]]);
    let r = read_toy("two failing literals", &e.cl, &fx);
    assert_eq!(r.shapes(), vec![(4, 3, 0)]);
    assert_eq!(r.grids[0].rowlines.iter().sum::<u64>(), 3);
}

#[test]
fn gates_stay_out() {
    let fx = Fixture::new("grid_gates");
    assert!(read_toy("and chain", &and_chain(6).cl, &fx).grids.is_empty());
    assert!(read_toy("or gates", &or_gates(8).cl, &fx).grids.is_empty());
    // gates over a grid's literals leave the grid as it is
    let mut e = php(5, 4, How::Pairwise);
    for i in 1..20 {
        let o = e.var();
        e.cl.push(vec![o, -i]);
        e.cl.push(vec![o, -(i + 1)]);
        e.cl.push(vec![-o, i, i + 1]);
    }
    assert!(read_toy("gates over a grid", &e.cl, &fx).count((5, 4, 0)) == 1);
}

#[test]
fn a_broken_copy_does_not_claim_what_it_lost() {
    let fx = Fixture::new("grid_broken");
    // a pair clause dropped: the pair shares no column
    let mut e = php(5, 4, How::Pairwise);
    let i = e.cl.iter().position(|c| c.len() == 2).unwrap();
    let c = e.cl.remove(i);
    let r = read_toy("pair dropped", &e.cl, &fx);
    assert!(!r.grids.iter().any(|g| g.cols.iter().any(|col| col.contains(&-c[0]) && col.contains(&-c[1]))));
    assert!(!stock_refutes(&e.cl, &[-c[0], -c[1]], &fx.path("sem.cnf")), "the pair is free now");
    // any clause of a ladder dropped: no column of the whole is left
    let whole = php(5, 4, How::Ladder);
    let columns: BTreeSet<Vec<i32>> = read_toy("whole", &whole.cl, &fx).grids.iter().flat_map(|g| g.cols.iter().cloned()).collect();
    assert_eq!(columns.len(), 4);
    for i in 5..whole.cl.len() {
        let mut cl = whole.cl.clone();
        cl.remove(i);
        let r = read_toy("ladder clause dropped", &cl, &fx);
        let now: BTreeSet<Vec<i32>> = r.grids.iter().flat_map(|g| g.cols.iter().cloned()).collect();
        assert!(!columns.is_subset(&now), "clause {:?} dropped, all columns still read", whole.cl[i]);
    }
}

/// A private generator: the formulas depend only on the seed.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.next(100) < percent
    }
}

fn mutated(seed: u64) -> Vec<Vec<i32>> {
    let mut g = Lcg(seed.wrapping_mul(2862933555777941757).wrapping_add(3037000493));
    let hows = [How::Pairwise, How::Ladder, How::Commander];
    let how = hows[g.next(3)];
    let e = match g.next(6) {
        0 => latin(3, how),
        1 => php(4 + g.next(3), 3 + g.next(3), how),
        2 => queens(4 + g.next(2)),
        3 => colouring(&clique_edges(5), 5, 3 + g.next(2)),
        4 => guarded(3 + g.next(2), 3 + g.next(2)),
        _ => one_way(3 + g.next(2), 2 + g.next(2)),
    };
    let nv = e.n;
    let mut cl = e.cl;
    let random_lit = |g: &mut Lcg| (g.next(nv as usize) as i32 + 1) * if g.chance(50) { 1 } else { -1 };
    // dropped clauses
    if g.chance(40) {
        cl.retain(|_| !g.chance(5));
    }
    // noise
    for _ in 0..g.next(4) {
        let c = vec![random_lit(&mut g), random_lit(&mut g)];
        cl.push(c);
    }
    for _ in 0..g.next(3) {
        let c = vec![random_lit(&mut g), random_lit(&mut g), random_lit(&mut g)];
        cl.push(c);
    }
    // repeated literals, tautologies, duplicates, units
    if g.chance(30) && !cl.is_empty() {
        let i = g.next(cl.len());
        let l = cl[i][0];
        cl[i].push(l);
    }
    if g.chance(30) {
        let l = random_lit(&mut g);
        cl.push(vec![l, -l, random_lit(&mut g)]);
    }
    if g.chance(30) && !cl.is_empty() {
        let c = cl[g.next(cl.len())].clone();
        cl.push(c);
    }
    if g.chance(25) {
        let l = random_lit(&mut g);
        cl.push(vec![l]);
    }
    // signs and names, with gaps
    let gap = 1 + g.next(3) as i32;
    let flips: Vec<bool> = (0..=nv).map(|_| g.chance(50)).collect();
    for c in cl.iter_mut() {
        for l in c.iter_mut() {
            let v = l.abs();
            let sign = if flips[v as usize] { -l.signum() } else { l.signum() };
            *l = sign * (v * gap);
        }
    }
    // shuffled
    for i in (1..cl.len()).rev() {
        let j = g.next(i + 1);
        cl.swap(i, j);
    }
    cl
}

#[test]
fn random_mutated_encodings_hold_up() {
    let fx = Fixture::new("grid_fuzz");
    let mut with_grid = 0;
    for seed in 0..80 {
        let cl = mutated(seed);
        let r = read_toy(&format!("fuzz {}", seed), &cl, &fx);
        with_grid += !r.grids.is_empty() as u32;
    }
    assert!(with_grid >= 20, "only {} of the mutated formulas held a grid", with_grid);
}

#[test]
fn the_read_stops_at_its_budget_and_two_runs_are_alike() {
    let fx = Fixture::new("grid_budget");
    let cnf = fx.path("b.cnf");
    let pbp = fx.path("b.pbp");
    let mut padded = php(14, 13, How::Pairwise).cl;
    // a stray variable far out, and thousands of copies of clauses
    for _ in 0..3000 {
        padded.push(padded[0].clone());
        padded.push(padded[20].clone());
    }
    padded.push(vec![-2_000_000]);
    for (name, cl) in [("latin 7 ladder", latin(7, How::Ladder).cl), ("padded pigeonhole", padded)] {
        write_cnf(&cnf, &cl, &[]);
        let a = read(&cnf, &pbp, &[]);
        let b = read(&cnf, &pbp, &[]);
        assert!(!a.grids.is_empty(), "{}", name);
        assert_eq!(a.ticks(), b.ticks(), "{}: two runs", name);
        let lines = |r: &Read| -> Vec<String> { r.stdout.lines().filter(|l| l.starts_with("c gridread")).map(String::from).collect() };
        assert_eq!(lines(&a), lines(&b), "{}: two runs", name);
        let whole = a.ticks();
        assert!(whole > 20_000, "{}: {} ticks", name, whole);
        for percent in [5, 10, 25, 50, 75, 95] {
            let k = (whole * percent / 100 / 1000).max(1);
            let opt = format!("--cardreadticks={}", k);
            let r = read(&cnf, &pbp, &[&opt]);
            let limit = k * 1000;
            assert!(r.ticks() <= limit + limit / 10, "{} at {}%: {} ticks over a limit of {}", name, percent, r.ticks(), limit);
            check_by_hand(name, &cl, &r);
            veripb_accepts(name, &cnf, &pbp, &cl, &r);
        }
    }
}

/// A grid with many rows around it that are no domains: long clauses over groups of
/// literals that exclude each other inside a group, each literal also excluding a
/// whole column of the grid. Such a row has more exclusions towards a grid row than
/// two grid rows have. With a budget too small to read all rows before linking, those
/// rows come first among a grid row's neighbours, and they must not stop the read.
#[test]
fn a_grid_behind_many_rows_that_are_no_domains() {
    let fx = Fixture::new("grid_behind");
    let (n, d, groups, per) = (6, 10, 40, 10);
    let mut e = Enc::default();
    let rows: Vec<Vec<i32>> = (0..n).map(|_| (0..d).map(|_| e.var()).collect()).collect();
    for r in &rows {
        e.cl.push(r.clone());
        e.pairwise(r);
    }
    for j in 0..d {
        let col: Vec<i32> = rows.iter().map(|r| r[j]).collect();
        e.pairwise(&col);
    }
    let pool: Vec<Vec<i32>> = (0..groups).map(|_| (0..per).map(|_| e.var()).collect()).collect();
    for g in &pool {
        e.pairwise(g);
        for (i, &p) in g.iter().enumerate() {
            for r in &rows {
                e.cl.push(vec![-p, -r[i % d]]);
            }
        }
    }
    for a in 0..groups {
        for b in 0..a {
            let mut c = pool[a].clone();
            c.extend_from_slice(&pool[b]);
            e.cl.push(c);
        }
    }
    let cnf = fx.path("b.cnf");
    let pbp = fx.path("b.pbp");
    write_cnf(&cnf, &e.cl, &[]);
    let r = read(&cnf, &pbp, &["--cardreadticks=400"]);
    assert!(r.grids.iter().any(|g| g.rows.len() >= n), "read {:?}", r.shapes());
    check_by_hand("behind", &e.cl, &r);
    veripb_accepts("behind", &cnf, &pbp, &e.cl, &r);
}

#[test]
fn off_means_off() {
    let fx = Fixture::new("grid_off");
    let cnf = fx.path("off.cnf");
    let pbp = fx.path("off.pbp");
    write_cnf(&cnf, &php(4, 3, How::Pairwise).cl, &[]);
    let mut cmd = Command::new(solver_bin());
    cmd.arg("-q").arg("--card=0").args(STOCK_PATH).arg(&cnf);
    for key in SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    cmd.env("SAT_CARD_READ_CHECK", &pbp);
    let out = cmd.output().expect("run sat-solver");
    assert_eq!(out.status.code(), Some(20));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("c gridread"));
    assert!(!pbp.exists());
}
