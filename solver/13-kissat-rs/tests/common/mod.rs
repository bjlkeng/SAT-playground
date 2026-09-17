// Shared helpers for the solver-13 integration tests (RL plan step A).
//
// The tests drive the release-interface binary (`CARGO_BIN_EXE_sat-solver`)
// on small generated formulas that still take thousands of conflicts, so
// that probe, eliminate, reduce, rephase and mode-switch fire several times.
// Instances are written to a per-process scratch directory under the system
// temp dir and removed when the `Fixture` is dropped.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn solver_bin() -> &'static str {
    env!("CARGO_BIN_EXE_sat-solver")
}

/// A scratch directory that is deleted on drop.
pub struct Fixture {
    pub dir: PathBuf,
}

impl Fixture {
    pub fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "sat13-tests-{}-{}-{}",
            std::process::id(),
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).expect("create fixture dir");
        Fixture { dir }
    }

    pub fn path(&self, file: &str) -> PathBuf {
        self.dir.join(file)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Pigeonhole principle with `n + 1` pigeons and `n` holes (UNSAT). php8 is
/// about 10 k conflicts and 4 M work units for the stock solver; php9 about
/// 27 k conflicts and 15 M.
pub fn php(n: usize, path: &Path) {
    let pigeons = n + 1;
    let holes = n;
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
    write_cnf(path, pigeons * holes, &clauses);
}

/// Uniform random 3-SAT at the given clause/variable ratio, from a private
/// LCG so the formula depends only on `seed`. 250 vars at 4.26 is a
/// satisfiable cell of about 56 k conflicts with two eliminate and five
/// probe rounds under the stock solver.
pub fn random_3sat(vars: usize, ratio: f64, seed: u64, path: &Path) {
    let mut rng = seed.wrapping_mul(2862933555777941757).wrapping_add(3037000493);
    let mut next = || {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (rng >> 33) as usize
    };
    let clauses_n = (vars as f64 * ratio) as usize;
    let mut clauses = Vec::with_capacity(clauses_n);
    for _ in 0..clauses_n {
        let mut lits: Vec<i64> = Vec::with_capacity(3);
        while lits.len() < 3 {
            let v = (next() % vars + 1) as i64;
            if lits.iter().any(|l| l.abs() == v) {
                continue;
            }
            let sign = if next() % 2 == 0 { 1 } else { -1 };
            lits.push(sign * v);
        }
        clauses.push(lits);
    }
    write_cnf(path, vars, &clauses);
}

/// `n` independent pairs (x_i ∨ y_i) ∧ (¬x_i ∨ ¬y_i): satisfiable, and the
/// stock solver's lucky pass solves it before search starts with work
/// proportional to `n`.
pub fn lucky_pairs(n: usize, path: &Path) {
    let mut clauses = Vec::with_capacity(2 * n);
    for i in 0..n {
        let x = (2 * i + 1) as i64;
        let y = (2 * i + 2) as i64;
        clauses.push(vec![x, y]);
        clauses.push(vec![-x, -y]);
    }
    write_cnf(path, 2 * n, &clauses);
}

fn write_cnf(path: &Path, vars: usize, clauses: &[Vec<i64>]) {
    let mut f = fs::File::create(path).expect("create cnf");
    writeln!(f, "p cnf {} {}", vars, clauses.len()).unwrap();
    for c in clauses {
        for l in c {
            write!(f, "{} ", l).unwrap();
        }
        writeln!(f, "0").unwrap();
    }
}

/// One solver run: the `s` status, the `-s` counters (first column of every
/// `c name: value` row inside the statistics block, exactly what
/// tools/parity.py compares), the `c workclock` key=value pairs, and the raw
/// streams.
pub struct Run {
    pub status: String,
    pub stats: BTreeMap<String, u64>,
    pub workclock: BTreeMap<String, u64>,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl Run {
    pub fn work(&self) -> u64 {
        *self.workclock.get("work").expect("work= on the workclock line")
    }
}

/// Environment variables the solver reads (RL plan §7 item 6). The test
/// runner clears every one of them before applying the test's own settings,
/// so a value in the developer's shell can never leak into a test.
pub const SOLVER_ENV_VARS: &[&str] = &[
    "SAT_LIMIT_TICKS",
    "SAT_POLICY",
    "SAT_POLICY_EPOCH_TICKS",
    "SAT_POLICY_SEED",
    "SAT_POLICY_TEMP",
    "SAT_POLICY_SEGMENT",
    "SAT_POLICY_LOG",
    "SAT_POLICY_MARGIN",
    "SAT_POLICY_HORIZON",
    "SAT_WALL_LIMIT",
    "SAT_POLICY_BRANCH",
    "SAT_POLICY_BRANCH_ACTIONS",
    "SAT_POLICY_BRANCH_JOBS",
    "SAT_POLICY_LOG_RESERVED",
];

/// Run the binary with `-n -s`, the given extra kissat options and the given
/// environment additions (a value of "" is passed through as an empty
/// variable, which the solver must treat as unset).
pub fn run(cnf: &Path, options: &[&str], env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(solver_bin());
    cmd.arg("-n").arg("-s");
    for o in options {
        cmd.arg(o);
    }
    cmd.arg(cnf);
    for key in SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let mut status = String::new();
    let mut stats = BTreeMap::new();
    let mut workclock = BTreeMap::new();
    let mut in_stats = false;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("s ") {
            status = rest.trim().to_string();
        } else if line.contains("[ statistics ]") {
            in_stats = true;
        } else if in_stats && line.contains("[ ") && line.contains(" ]") {
            in_stats = false;
        } else if let Some(rest) = line.strip_prefix("c workclock ") {
            for tok in rest.split_whitespace() {
                if let Some((k, v)) = tok.split_once('=') {
                    if let Ok(v) = v.parse::<u64>() {
                        workclock.insert(k.to_string(), v);
                    }
                }
            }
        } else if in_stats {
            if let Some(rest) = line.strip_prefix("c ") {
                if let Some((name, tail)) = rest.split_once(':') {
                    if let Some(first) = tail.split_whitespace().next() {
                        if let Ok(v) = first.parse::<u64>() {
                            stats.insert(name.to_string(), v);
                        }
                    }
                }
            }
        }
    }
    Run {
        status,
        stats,
        workclock,
        stdout,
        stderr,
        exit_code: out.status.code().unwrap_or(-1),
    }
}

/// Like `run`, with a DRAT proof file as the second positional argument.
pub fn run_with_proof(cnf: &Path, proof: &Path, env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(solver_bin());
    cmd.arg("-n").arg("-s").arg(cnf).arg(proof);
    for key in SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let status = stdout
        .lines()
        .find_map(|l| l.strip_prefix("s ").map(|s| s.trim().to_string()))
        .unwrap_or_default();
    Run {
        status,
        stats: BTreeMap::new(),
        workclock: BTreeMap::new(),
        stdout,
        stderr,
        exit_code: out.status.code().unwrap_or(-1),
    }
}

/// Assert two runs took the same trajectory: same status, same `-s`
/// counters and same work-clock line.
pub fn assert_same_trajectory(a: &Run, b: &Run, what: &str) {
    assert_eq!(a.status, b.status, "{}: status differs", what);
    assert_eq!(a.stats, b.stats, "{}: -s counters differ", what);
    assert_eq!(a.workclock, b.workclock, "{}: workclock line differs", what);
}
