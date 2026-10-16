// The parity engine (src/gauss.rs, bead SAT-playground-1v2.7): XOR
// reasoning inside search, option gauss, off by default.
mod common;

use common::{run, Fixture};
use std::path::Path;

/// A private LCG so a formula depends on its seed alone.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The clauses of x1 xor ... xor xk = parity (every pattern of the wrong
/// parity forbidden), over the given literals' variables.
fn xor_clauses(vars: &[i64], parity: u32) -> Vec<Vec<i64>> {
    let k = vars.len();
    let mut out = Vec::new();
    for pattern in 0..(1u32 << k) {
        if pattern.count_ones() % 2 != parity {
            // forbid the assignment with this pattern of trues
            out.push((0..k).map(|i| if pattern >> i & 1 == 1 { -vars[i] } else { vars[i] }).collect());
        }
    }
    out
}

/// A random system of `m` 3-XORs over `n` variables with a planted
/// solution (so it is consistent), and the planted assignment.
fn planted_xors(n: usize, m: usize, rng: &mut Rng) -> (Vec<Vec<i64>>, Vec<bool>) {
    let planted: Vec<bool> = (0..n).map(|_| rng.below(2) == 1).collect();
    let mut clauses = Vec::new();
    for _ in 0..m {
        let mut vars: Vec<i64> = Vec::new();
        while vars.len() < 3 {
            let v = rng.below(n as u64) as i64 + 1;
            if !vars.contains(&v) {
                vars.push(v);
            }
        }
        let parity = vars.iter().filter(|&&v| planted[v as usize - 1]).count() as u32 % 2;
        clauses.extend(xor_clauses(&vars, parity));
    }
    (clauses, planted)
}

fn write(path: &Path, vars: usize, clauses: &[Vec<i64>]) {
    let mut text = format!("p cnf {} {}\n", vars, clauses.len());
    for c in clauses {
        for l in c {
            text.push_str(&format!("{} ", l));
        }
        text.push_str("0\n");
    }
    std::fs::write(path, text).unwrap();
}

fn model(stdout: &str) -> Vec<i64> {
    stdout
        .lines()
        .filter_map(|l| l.strip_prefix("v "))
        .flat_map(|l| l.split_whitespace().map(|t| t.parse::<i64>().unwrap()))
        .filter(|&l| l != 0)
        .collect()
}

/// Run with a witness and check it against every clause.
fn run_and_check_model(cnf: &Path, clauses: &[Vec<i64>], options: &[&str]) {
    let mut cmd = std::process::Command::new(common::solver_bin());
    cmd.args(common::STOCK_PATH).args(options).arg(cnf);
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.lines().any(|l| l == "s SATISFIABLE"), "expected SATISFIABLE:\n{}", stdout);
    let m = model(&stdout);
    for c in clauses {
        assert!(c.iter().any(|l| m.contains(l)), "the model falsifies {:?}", c);
    }
}

fn assert_same_search(a: &common::Run, b: &common::Run, what: &str) {
    assert_eq!(a.status, b.status, "{}: status", what);
    for key in ["conflicts", "decisions", "propagations"] {
        assert_eq!(a.stats.get(key), b.stats.get(key), "{}: {}", what, key);
    }
    assert_eq!(a.workclock["search_ticks"], b.workclock["search_ticks"], "{}: search ticks", what);
}

fn assert_same_trajectory(a: &common::Run, b: &common::Run, what: &str) {
    assert_same_search(a, b, what);
    assert_eq!(a.work(), b.work(), "{}: work", what);
}

#[test]
fn gauss_off_is_the_stock_trajectory_and_on_without_xors_too() {
    let fx = Fixture::new("gauss-stock");
    // a formula with XORs: the option at 0 is the stock run exactly
    let cnf = fx.path("mixed.cnf");
    let mut rng = Rng(20261015);
    let (mut clauses, planted) = planted_xors(80, 70, &mut rng);
    for _ in 0..40 {
        let a = rng.below(80) as i64 + 1;
        let b = rng.below(80) as i64 + 1;
        let la = if planted[a as usize - 1] { a } else { -a };
        clauses.push(vec![la, if rng.below(2) == 1 { b } else { -b }]);
    }
    write(&cnf, 80, &clauses);
    let stock = run(&cnf, &[], &[]);
    assert_same_trajectory(&stock, &run(&cnf, &["--gauss=0"], &[]), "gauss=0 v no option");
    assert!(!stock.stdout.contains("parity engine"));
    // a formula without XORs: the engine never starts and the search is
    // the stock one (the work clock holds the scan for XORs, nothing else)
    let plain = fx.path("plain.cnf");
    common::random_3sat(150, 4.2, 7, &plain);
    let base = run(&plain, &[], &[]);
    let on = run(&plain, &["--gauss=1"], &[]);
    assert_same_search(&base, &on, "gauss=1 without XORs");
    assert!(on.work() >= base.work() && on.work() < base.work() + 100_000, "{} v {}", on.work(), base.work());
    assert!(!on.stdout.contains("parity engine on"));
}

/// A small parity-learning formula (the par family's shape): n unknown
/// bits, m samples each the parity of a random subset through a chain of
/// helper variables with an error bit, at most one error (pairwise), one
/// error planted. The engine solves it in a few conflicts; the stock
/// solver cannot within the conflict limit.
fn parity_learning(n: usize, m: usize, rng: &mut Rng) -> (usize, Vec<Vec<i64>>) {
    let x: Vec<bool> = (0..n).map(|_| rng.below(2) == 1).collect();
    let bad = rng.below(m as u64) as usize;
    let mut next = n as i64;
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    let mut errors: Vec<i64> = Vec::new();
    for j in 0..m {
        let mut sub: Vec<i64> = (1..=n as i64).filter(|_| rng.below(2) == 1).collect();
        if sub.len() < 2 {
            sub = vec![1, 2];
        }
        next += 1;
        let e = next;
        errors.push(e);
        let mut b = sub.iter().filter(|&&v| x[v as usize - 1]).count() as u32 % 2;
        if j == bad {
            b ^= 1;
        }
        next += 1;
        let mut t = next;
        clauses.extend(xor_clauses(&[sub[0], sub[1], t], 0));
        for &v in &sub[2..] {
            next += 1;
            clauses.extend(xor_clauses(&[t, v, next], 0));
            t = next;
        }
        clauses.extend(xor_clauses(&[t, e], b));
    }
    for i in 0..m {
        for j in i + 1..m {
            clauses.push(vec![-errors[i], -errors[j]]);
        }
    }
    (next as usize, clauses)
}

#[test]
fn the_engine_solves_what_clause_search_cannot() {
    let fx = Fixture::new("gauss-par");
    let cnf = fx.path("par.cnf");
    let mut rng = Rng(20261015 + 1);
    let (vars, clauses) = parity_learning(28, 48, &mut rng);
    write(&cnf, vars, &clauses);
    let on = run(&cnf, &["--gauss=1", "--gausseffort=0"], &[]);
    assert_eq!(on.status, "SATISFIABLE", "{}", on.stdout);
    assert!(on.stdout.contains("parity engine on"), "{}", on.stdout);
    assert!(on.stats["conflicts"] < 2000, "{} conflicts", on.stats["conflicts"]);
    run_and_check_model(&cnf, &clauses, &["--gauss=1", "--gausseffort=0"]);
    let off = run(&cnf, &["--gauss=0", "--conflicts=20000"], &[]);
    assert_eq!(off.status, "UNKNOWN", "the stock solver solved a formula meant to be beyond its conflict limit:\n{}", off.stdout);
}

/// An XOR system that is inconsistent on its own (a Tseitin formula on a
/// random cubic graph with an odd charge), mixed with clauses so the parity
/// pass's closed-system step is not what settles it: the engine refutes it
/// when it builds the matrix.
#[test]
fn an_inconsistent_system_is_refuted_at_once() {
    let fx = Fixture::new("gauss-tseitin");
    let cnf = fx.path("tseitin.cnf");
    let mut rng = Rng(20261015 + 4);
    let n = 60usize; // vertices, cubic: 90 edges
    // a random cubic graph by pairing 3n stubs; multi-edges are fine
    let mut stubs: Vec<usize> = (0..n).flat_map(|v| [v, v, v]).collect();
    for i in (1..stubs.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        stubs.swap(i, j);
    }
    let mut edges_of: Vec<Vec<i64>> = vec![Vec::new(); n];
    for (k, pair) in stubs.chunks(2).enumerate() {
        edges_of[pair[0]].push(k as i64 + 1);
        edges_of[pair[1]].push(k as i64 + 1);
    }
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for v in 0..n {
        // a loop edge is at the vertex twice: it cancels out of the XOR
        let mut vars: Vec<i64> = edges_of[v].iter().copied().filter(|e| edges_of[v].iter().filter(|f| f == &e).count() % 2 == 1).collect();
        vars.sort_unstable();
        vars.dedup();
        if vars.is_empty() {
            assert_ne!(v, 0, "the charged vertex has only loops");
            continue;
        }
        clauses.extend(xor_clauses(&vars, if v == 0 { 1 } else { 0 }));
    }
    // clauses on the edge variables, so the system is not closed
    let m = 3 * n / 2;
    for _ in 0..40 {
        let a = rng.below(m as u64) as i64 + 1;
        let b = rng.below(m as u64) as i64 + 1;
        let c = rng.below(m as u64) as i64 + 1;
        clauses.push(vec![a, -b, c]);
    }
    write(&cnf, m, &clauses);
    let on = run(&cnf, &["--gauss=1", "--gausseffort=0"], &[]);
    assert_eq!(on.status, "UNSATISFIABLE", "{}", on.stdout);
    assert!(
        on.stdout.contains("XOR system is inconsistent") || on.stdout.contains("refutes the formula at level zero"),
        "{}",
        on.stdout
    );
    assert_eq!(on.stats["conflicts"], 0);
    let off = run(&cnf, &["--gauss=0", "--conflicts=20000"], &[]);
    assert_eq!(off.status, "UNKNOWN", "{}", off.stdout);
}

/// Long XORs through chains of helper variables (as encodings write
/// them): the helpers occur in no other clause, so the engine drops their
/// rows, and the model it finds satisfies every clause.
#[test]
fn chains_are_reduced_and_the_model_holds() {
    let fx = Fixture::new("gauss-chains");
    let cnf = fx.path("chains.cnf");
    let n = 40usize;
    let mut rng = Rng(20261015 + 2);
    let planted: Vec<bool> = (0..n).map(|_| rng.below(2) == 1).collect();
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    let mut next = n as i64;
    // 30 long XORs of 8 variables each, as chains of 3-XORs over helpers
    for _ in 0..30 {
        let mut vars: Vec<i64> = Vec::new();
        while vars.len() < 8 {
            let v = rng.below(n as u64) as i64 + 1;
            if !vars.contains(&v) {
                vars.push(v);
            }
        }
        let parity = vars.iter().filter(|&&v| planted[v as usize - 1]).count() as u32 % 2;
        // t1 = v1 xor v2, t2 = t1 xor v3, ..., t6 = t5 xor v7, t6 xor v8 = parity
        next += 1;
        let mut t = next;
        clauses.extend(xor_clauses(&[vars[0], vars[1], t], 0));
        for &v in &vars[2..7] {
            next += 1;
            clauses.extend(xor_clauses(&[t, v, next], 0));
            t = next;
        }
        clauses.extend(xor_clauses(&[t, vars[7]], parity));
    }
    // and some clauses true under the planting, on the main variables
    for _ in 0..60 {
        let mut c: Vec<i64> = Vec::new();
        for _ in 0..3 {
            let v = rng.below(n as u64) as i64 + 1;
            c.push(if rng.below(3) == 0 || planted[v as usize - 1] { v } else { -v });
        }
        if !c.iter().any(|&l| (l > 0) == planted[l.unsigned_abs() as usize - 1]) {
            c[0] = -c[0];
        }
        clauses.push(c);
    }
    write(&cnf, next as usize, &clauses);
    let on = run(&cnf, &["--gauss=1", "--gausseffort=0"], &[]);
    assert_eq!(on.status, "SATISFIABLE");
    assert!(on.stdout.contains("rows of helper variables dropped"), "{}", on.stdout);
    run_and_check_model(&cnf, &clauses, &["--gauss=1", "--gausseffort=0"]);
}

/// The engine's work is in the work clock, so a tick limit ends a run with
/// the engine on at the same place twice.
#[test]
fn the_work_limit_covers_the_engine() {
    let fx = Fixture::new("gauss-limit");
    let cnf = fx.path("mixed.cnf");
    let mut rng = Rng(20261015 + 3);
    let (clauses, _) = planted_xors(200, 190, &mut rng);
    write(&cnf, 200, &clauses);
    let full = run(&cnf, &["--gauss=1", "--gausseffort=0"], &[]);
    assert!(full.stdout.contains("parity engine on"), "{}", full.stdout);
    let gauss = full.workclock["gauss_ticks"];
    assert!(gauss > 0);
    let limit = (full.work() / 2).to_string();
    let a = run(&cnf, &["--gauss=1", "--gausseffort=0"], &[("SAT_LIMIT_TICKS", &limit)]);
    let b = run(&cnf, &["--gauss=1", "--gausseffort=0"], &[("SAT_LIMIT_TICKS", &limit)]);
    assert_same_trajectory(&a, &b, "two runs at the same tick limit");
    assert!(a.work() <= full.work());
}
