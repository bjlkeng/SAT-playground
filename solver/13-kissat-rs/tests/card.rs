// The count and alldifferent engine (src/card.rs, epic SAT-playground-iin): lemmas
// from sums of rows and caps, option card, on by default since 2026-11-02.
mod common;

use common::{assert_same_trajectory, run, Fixture};
use std::path::Path;
use std::process::Command;

fn gate(cnf: &Path, proof: &Path) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new("python3").arg(root.join("tools/proof_gate.py")).arg(cnf).arg(proof).output().expect("run proof_gate.py");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// VeriPB alone: the lemmas hold whatever the solver answered.
fn stream_check(cnf: &Path, proof: &Path) -> String {
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools");
    let code = format!("import sys; sys.path.insert(0, {:?}); import proof_gate; print(proof_gate.check_stream({:?}, {:?}))", tools, cnf, proof);
    let out = Command::new("python3").arg("-c").arg(code).output().expect("run proof_gate");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Cnf {
    vars: i64,
    clauses: Vec<Vec<i64>>,
}

impl Cnf {
    fn new(vars: i64) -> Cnf {
        Cnf { vars, clauses: Vec::new() }
    }
    fn fresh(&mut self) -> i64 {
        self.vars += 1;
        self.vars
    }
    fn add(&mut self, c: &[i64]) {
        self.clauses.push(c.to_vec());
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
    /// At most `k` of `lits`, a Sinz counter.
    fn at_most(&mut self, lits: &[i64], k: usize) {
        let n = lits.len();
        let mut s = vec![vec![0i64; k]; n];
        for row in s.iter_mut().take(n - 1) {
            for r in row.iter_mut() {
                *r = self.vars + 1;
                self.vars += 1;
            }
        }
        for i in 0..n - 1 {
            self.add(&[-lits[i], s[i][0]]);
            if i > 0 {
                for j in 0..k {
                    self.add(&[-s[i - 1][j], s[i][j]]);
                }
                for j in 1..k {
                    self.add(&[-lits[i], -s[i - 1][j - 1], s[i][j]]);
                }
                self.add(&[-lits[i], -s[i - 1][k - 1]]);
            }
        }
        self.add(&[-lits[n - 1], -s[n - 2][k - 1]]);
    }
    /// At most one of `lits`, a ladder: x_i -> s_i, s_i -> s_(i+1), x_(i+1) -> -s_i.
    fn ladder(&mut self, lits: &[i64]) {
        let s: Vec<i64> = lits[..lits.len() - 1].iter().map(|_| self.fresh()).collect();
        for (i, &x) in lits[..lits.len() - 1].iter().enumerate() {
            self.add(&[-x, s[i]]);
        }
        for i in 0..s.len().saturating_sub(1) {
            self.add(&[-s[i], s[i + 1]]);
        }
        for i in 1..lits.len() {
            self.add(&[-lits[i], -s[i - 1]]);
        }
    }
    fn pairs(&mut self, lits: &[i64]) {
        for i in 0..lits.len() {
            for j in i + 1..lits.len() {
                self.add(&[-lits[i], -lits[j]]);
            }
        }
    }
}

/// A grid of rows x cols variables, a clause per row. The row's variable j of row
/// i is i * cols + j + 1.
fn grid(rows: i64, cols: i64) -> (Cnf, Vec<Vec<i64>>) {
    let mut f = Cnf::new(rows * cols);
    let table: Vec<Vec<i64>> = (0..rows).map(|i| (0..cols).map(|j| i * cols + j + 1).collect()).collect();
    for r in &table {
        f.add(r);
    }
    (f, table)
}

fn column(table: &[Vec<i64>], j: usize) -> Vec<i64> {
    table.iter().map(|r| r[j]).collect()
}

/// The numbers on the `c card` line.
fn card_line(stdout: &str) -> std::collections::BTreeMap<String, String> {
    let line = stdout.lines().find(|l| l.starts_with("c card ")).unwrap_or_else(|| panic!("no card line:\n{}", stdout));
    line.split_whitespace().filter_map(|t| t.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn number(line: &std::collections::BTreeMap<String, String>, key: &str) -> u64 {
    line[key].parse().unwrap()
}

fn lemmas(line: &std::collections::BTreeMap<String, String>) -> u64 {
    number(line, "violations") + number(line, "forced") + number(line, "root_clauses")
}

/// Solve with a proof and send the answer through the two-stage gate.
fn refuted_through_the_gate(fx: &Fixture, cnf: &Path, options: &[&str], name: &str) -> std::collections::BTreeMap<String, String> {
    let proof = fx.path(&format!("{}.drat", name));
    let mut cmd = Command::new(common::solver_bin());
    cmd.args(common::STOCK_PATH).args(options).arg(cnf).arg(&proof);
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    assert_eq!(gate(cnf, &proof), "ok", "{}: the gate refused the proof\n{}", name, stdout);
    card_line(&stdout)
}

fn model_is_good(stdout: &str, clauses: &[Vec<i64>]) -> bool {
    let model: std::collections::HashSet<i64> = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("v "))
        .flat_map(|l| l.split_whitespace().map(|t| t.parse::<i64>().unwrap()))
        .filter(|&l| l != 0)
        .collect();
    clauses.iter().all(|c| c.iter().any(|l| model.contains(l)))
}

/// Without a proof file, with the witness printed and checked.
fn satisfied(cnf: &Path, f: &Cnf, options: &[&str]) -> std::collections::BTreeMap<String, String> {
    let mut cmd = Command::new(common::solver_bin());
    cmd.args(common::STOCK_PATH).args(options).arg(cnf);
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("s SATISFIABLE"), "{}", stdout);
    assert!(model_is_good(&stdout, &f.clauses), "bad model:\n{}", stdout);
    card_line(&stdout)
}

/// k + 1 true cells wanted of a board that has room for only k, in pairs per
/// row and per column: "at most k" over the negations, as a Sinz counter.
fn over_full_board(n: i64) -> Cnf {
    let (mut f, table) = grid(n, n);
    f.clauses.clear();
    for r in &table {
        f.pairs(r);
    }
    for j in 0..n as usize {
        f.pairs(&column(&table, j));
    }
    let negations: Vec<i64> = table.iter().flatten().map(|&x| -x).collect();
    let at_most = (n * n - (n + 1)) as usize;
    f.at_most(&negations, at_most);
    f
}

/// n of the board's cells wanted and three rows left with two columns: a search
/// has to find that out, because no one clause or count says it.
fn long_board(n: i64) -> Cnf {
    let (mut f, table) = grid(n, n);
    f.clauses.clear();
    for r in &table {
        f.pairs(r);
    }
    for j in 0..n as usize {
        f.pairs(&column(&table, j));
    }
    for r in &table[..3] {
        for &x in &r[2..] {
            f.add(&[-x]);
        }
    }
    let negations: Vec<i64> = table.iter().flatten().rev().map(|&x| -x).collect();
    f.at_most(&negations, (n * n - n) as usize);
    f
}

#[test]
fn on_by_default_and_off_prints_nothing() {
    let fx = Fixture::new("card-off");
    let cnf = fx.path("f.cnf");
    let (mut f, table) = grid(5, 4);
    for j in 0..4 {
        f.ladder(&column(&table, j));
    }
    f.write(&cnf);
    // the stock path less its --card=0, so the engine is at its default
    let rest: Vec<&str> = common::STOCK_PATH.iter().copied().filter(|o| *o != "--card=0").collect();
    assert_eq!(rest.len() + 1, common::STOCK_PATH.len());
    let default = common::run_shipped(&cnf, &rest, &[]);
    let on = run(&cnf, &["--card=1"], &[]);
    assert_same_trajectory(&default, &on, "card=1 is the default");
    assert!(number(&card_line(&default.stdout), "violations") >= 1, "{}", default.stdout);
    let off = run(&cnf, &[], &[]);
    assert_eq!(off.status, "UNSATISFIABLE");
    assert!(!off.stdout.contains("c card "), "{}", off.stdout);
}

#[test]
fn the_shipped_defaults_run_the_engine() {
    // on by default since 2026-11-02, the owner's call after the paired run
    // and the fresh-seed rerun: with no option at all, every other structure
    // pass on, a count over its rows is refuted by the engine with a proof
    // the gate accepts, and --card=0 turns it off
    let fx = Fixture::new("card-default");
    let cnf = fx.path("f.cnf");
    over_full_board(6).write(&cnf);
    let proof = fx.path("f.drat");
    let mut cmd = Command::new(common::solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    let out = cmd.arg(&cnf).arg(&proof).output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("\ns UNSATISFIABLE"), "{}", stdout);
    assert!(number(&card_line(&stdout), "violations") >= 1, "the engine is on by default: {}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok", "the gate refused the proof\n{}", stdout);
    let off = common::run_shipped(&cnf, &["--card=0"], &[]);
    assert_eq!(off.status, "UNSATISFIABLE");
    assert!(!off.stdout.contains("c card "), "{}", off.stdout);
}

#[test]
fn nothing_to_read_means_the_same_search() {
    // no rows and caps that meet, so the engine never looks at anything
    let fx = Fixture::new("card-nothing");
    let cnf = fx.path("f.cnf");
    common::random_3sat(60, 4.2, 7, &cnf);
    let off = run(&cnf, &[], &[]);
    let on = run(&cnf, &["--card=1"], &[]);
    assert_eq!(off.status, on.status);
    assert_eq!(off.stats["conflicts"], on.stats["conflicts"]);
    assert_eq!(off.stats["decisions"], on.stats["decisions"]);
    assert_eq!(number(&card_line(&on.stdout), "nets"), 0, "{}", on.stdout);
}

#[test]
fn a_count_over_its_rows_is_refuted_with_a_proof() {
    let fx = Fixture::new("card-over");
    let cnf = fx.path("f.cnf");
    let f = over_full_board(6);
    f.write(&cnf);
    let line = refuted_through_the_gate(&fx, &cnf, &["--card=1"], "over");
    assert!(number(&line, "counts") >= 1, "{:?}", line);
    assert!(lemmas(&line) >= 1, "{:?}", line);
}

#[test]
fn a_grid_with_too_many_rows_is_refuted_for_every_column_form() {
    let fx = Fixture::new("card-php");
    for (name, ladder) in [("pairs", false), ("ladder", true)] {
        let cnf = fx.path(&format!("{}.cnf", name));
        let (mut f, table) = grid(8, 7);
        for j in 0..7 {
            let col = column(&table, j);
            if ladder {
                f.ladder(&col);
            } else {
                f.pairs(&col);
            }
        }
        f.write(&cnf);
        let line = refuted_through_the_gate(&fx, &cnf, &["--card=1"], name);
        assert!(number(&line, "violations") >= 1, "{}: {:?}", name, line);
    }
}

#[test]
fn a_tight_set_of_rows_forces_the_others_off_its_columns() {
    // three rows hold only three columns, so the other rows cannot use them,
    // which clause propagation alone does not see
    let fx = Fixture::new("card-forced");
    let cnf = fx.path("f.cnf");
    let (mut f, table) = grid(6, 6);
    for j in 0..6 {
        f.pairs(&column(&table, j));
    }
    for r in &table[..3] {
        for &x in &r[3..] {
            f.add(&[-x]);
        }
    }
    f.write(&cnf);
    let line = satisfied(&cnf, &f, &["--card=1", "--cardint=1", "--lucky=0"]);
    assert!(number(&line, "forced") >= 1, "{:?}", line);
    // the formula is satisfiable, and the lemmas must still hold
    let proof = fx.path("p.drat");
    let mut cmd = Command::new(common::solver_bin());
    cmd.args(common::STOCK_PATH).args(["--card=1", "--cardint=1", "--lucky=0"]).arg(&cnf).arg(&proof);
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    assert!(String::from_utf8_lossy(&cmd.output().unwrap().stdout).contains("s SATISFIABLE"));
    assert_eq!(stream_check(&cnf, &proof), "ok");
}

#[test]
fn a_run_with_a_proof_makes_the_same_decisions_as_one_without() {
    let fx = Fixture::new("card-same");
    let cnf = fx.path("f.cnf");
    let (mut f, table) = grid(10, 9);
    for j in 0..9 {
        f.ladder(&column(&table, j));
    }
    f.write(&cnf);
    let plain = run(&cnf, &["--card=1", "--cardint=1"], &[]);
    let proof = fx.path("p.drat");
    let mut cmd = Command::new(common::solver_bin());
    cmd.arg("-n").arg("-s").args(common::STOCK_PATH).args(["--card=1", "--cardint=1"]).arg(&cnf).arg(&proof);
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let conflicts = |text: &str| text.lines().find(|l| l.starts_with("c conflicts:")).map(|l| l.split_whitespace().nth(2).unwrap().to_string());
    assert_eq!(conflicts(&plain.stdout), conflicts(&stdout));
    let work = |text: &str| text.lines().find(|l| l.starts_with("c workclock ")).map(|l| l.split_whitespace().find(|t| t.starts_with("work=")).unwrap().to_string());
    assert_eq!(work(&plain.stdout), work(&stdout), "the work clock must not depend on the proof file");
    assert_eq!(card_line(&plain.stdout), card_line(&stdout));
}

#[test]
fn a_tight_grid_says_every_column_is_used() {
    // n rows and n columns: every column holds a true. The stock search has to
    // find that case by case; with the root clauses it needs no search for it,
    // and each clause goes through the gate
    let fx = Fixture::new("card-root");
    let cnf = fx.path("f.cnf");
    let (mut f, table) = grid(6, 6);
    for j in 0..6 {
        f.pairs(&column(&table, j));
    }
    // now no way to use column 0: the root clause for it fails at once
    for r in &table {
        f.add(&[-r[0], r[1]]);
        f.add(&[-r[0], -r[1]]);
    }
    f.write(&cnf);
    let line = refuted_through_the_gate(&fx, &cnf, &["--card=1", "--cardroot=1", "--eliminate=0", "--probe=0", "--lucky=0"], "root");
    assert!(lemmas(&line) >= 1, "{:?}", line);

    // a satisfiable one: the clauses are added and the model holds
    let cnf2 = fx.path("g.cnf");
    let (mut f, table) = grid(6, 6);
    for j in 0..6 {
        f.pairs(&column(&table, j));
    }
    f.write(&cnf2);
    let line = satisfied(&cnf2, &f, &["--card=1", "--cardroot=1", "--eliminate=0", "--probe=0", "--lucky=0"]);
    assert_eq!(number(&line, "root_clauses"), 6, "{:?}", line);
}

#[test]
fn an_input_clause_is_not_added_again() {
    let fx = Fixture::new("card-root2");
    let cnf = fx.path("f.cnf");
    let (mut f, table) = grid(5, 5);
    for j in 0..5 {
        let col = column(&table, j);
        f.pairs(&col);
        f.add(&col);
    }
    f.write(&cnf);
    let line = satisfied(&cnf, &f, &["--card=1", "--cardroot=1", "--eliminate=0", "--probe=0", "--lucky=0"]);
    assert_eq!(number(&line, "root_clauses"), 0, "{:?}", line);
}

#[test]
fn the_guards_keep_lemmas_out_and_the_answer_right() {
    let fx = Fixture::new("card-guards");
    let cnf = fx.path("f.cnf");
    let (mut f, table) = grid(8, 7);
    for j in 0..7 {
        f.ladder(&column(&table, j));
    }
    f.write(&cnf);
    let stock = run(&cnf, &[], &[]);
    assert_eq!(stock.status, "UNSATISFIABLE");
    // a board where the lemmas are long, because they come from deep states
    let long = fx.path("long.cnf");
    long_board(7).write(&long);
    // no lemma fits this length
    let short = run(&long, &["--card=1", "--cardint=1", "--cardmaxlen=1"], &[]);
    assert_eq!(short.status, "UNSATISFIABLE");
    let line = card_line(&short.stdout);
    assert!(number(&line, "too_long") >= 1, "{:?}", line);
    // no proof lines to spend: the ladders' columns need them
    let poor = run(&cnf, &["--card=1", "--cardprooftotal=0"], &[]);
    assert_eq!(poor.status, "UNSATISFIABLE");
    let line = card_line(&poor.stdout);
    assert!(number(&line, "no_budget") >= 1, "{:?}", line);
    // no forced literals when told so
    let unforced = run(&cnf, &["--card=1", "--cardforced=0", "--cardmaxlen=1"], &[]);
    assert_eq!(number(&card_line(&unforced.stdout), "forced"), 0);
    // reading stopped before it starts: nothing to look at, the answer stands
    let unread = run(&cnf, &["--card=1", "--cardreadticks=0"], &[]);
    assert_eq!(unread.status, "UNSATISFIABLE");
    assert_eq!(number(&card_line(&unread.stdout), "nets"), 0, "{}", unread.stdout);
}

#[test]
fn a_stop_in_the_middle_leaves_a_proof_veripb_accepts() {
    // a formula the engine writes lemmas for, stopped by a conflict limit: the
    // stream is finished at exit, and every line up to there holds
    let fx = Fixture::new("card-stop");
    let cnf = fx.path("f.cnf");
    long_board(9).write(&cnf);
    let proof = fx.path("p.drat");
    let mut cmd = Command::new(common::solver_bin());
    cmd.args(common::STOCK_PATH).args(["--card=1", "--cardint=1", "--conflicts=20", "--lucky=0"]).arg(&cnf).arg(&proof);
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let code = out.status.code().unwrap_or(-1);
    assert!(code == 0 || code == 10 || code == 20, "{}", stdout);
    assert!(lemmas(&card_line(&stdout)) > 0, "the test needs a lemma before the stop:\n{}", stdout);
    assert_eq!(stream_check(&cnf, &proof), "ok");
}

#[test]
fn a_stream_with_no_checked_output_is_refused() {
    // an unproved clause in the derived file, and a stream that never says the
    // derived file is its output: VeriPB has nothing to compare it with
    let fx = Fixture::new("card-noout");
    let cnf = fx.path("f.cnf");
    std::fs::write(&cnf, "p cnf 1 1\n1 0\n").unwrap();
    let proof = fx.path("p.drat");
    std::fs::write(&proof, "").unwrap();
    std::fs::write(fx.path("p.drat.derived.cnf"), "p cnf 1 1\n-1 0\n").unwrap();
    std::fs::write(fx.path("p.drat.pbp"), "pseudo-Boolean proof version 2.0\nf 1 ;\noutput NONE ;\nconclusion NONE ;\nend pseudo-Boolean proof ;\n").unwrap();
    let verdict = stream_check(&cnf, &proof);
    assert!(verdict.starts_with("FAIL"), "{}", verdict);
}

#[test]
fn a_proof_that_cannot_start_turns_the_engine_off() {
    // with too few file handles the proof and its stream open but the derived file
    // does not. The engine must stay off: its lemmas would have no proof.
    let fx = Fixture::new("card-handles");
    let cnf = fx.path("f.cnf");
    over_full_board(5).write(&cnf);
    let proof = fx.path("p.drat");
    let mut script = String::from("ulimit -n 5; exec");
    script.push_str(&format!(" '{}'", common::solver_bin()));
    for o in common::STOCK_PATH.iter().chain(["--card=1"].iter()) {
        script.push_str(&format!(" '{}'", o));
    }
    script.push_str(&format!(" '{}' '{}'", cnf.display(), proof.display()));
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(script);
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("cannot write"), "the test needs the failure:\n{}", stdout);
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    let line = card_line(&stdout);
    assert_eq!(number(&line, "nets"), 0, "{}", stdout);
    assert_eq!(lemmas(&line), 0, "{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok", "{}", stdout);
}

#[test]
fn the_readers_test_hook_leaves_the_real_proof_alone() {
    // the hook proves every count into a file of its own; the search then writes
    // those proofs again into its stream, or the stream names lines it never wrote
    let fx = Fixture::new("card-hook");
    let cnf = fx.path("f.cnf");
    over_full_board(6).write(&cnf);
    let proof = fx.path("p.drat");
    let hook = fx.path("hook.pbp");
    let mut cmd = Command::new(common::solver_bin());
    cmd.args(common::STOCK_PATH).args(["--card=1"]).arg(&cnf).arg(&proof);
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    cmd.env("SAT_CARD_READ_CHECK", &hook);
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("s UNSATISFIABLE"), "{}", stdout);
    assert!(lemmas(&card_line(&stdout)) > 0, "the test needs a lemma:\n{}", stdout);
    assert_eq!(gate(&cnf, &proof), "ok", "{}", stdout);
}

#[test]
fn the_readers_test_hook_never_writes_over_the_input() {
    let fx = Fixture::new("card-hook-alias");
    let cnf = fx.path("f.cnf");
    std::fs::write(&cnf, "p cnf 2 1\n1 2 0\n").unwrap();
    let before = std::fs::read(&cnf).unwrap();
    let link = fx.path("link.cnf");
    std::os::unix::fs::symlink(&cnf, &link).unwrap();
    for target in [&cnf, &link] {
        let on = run(&cnf, &["--card=1"], &[("SAT_CARD_READ_CHECK", target.to_str().unwrap())]);
        assert_eq!(on.status, "SATISFIABLE", "{}", on.stdout);
        assert!(on.stdout.contains("cannot write the check file"), "{}", on.stdout);
        assert_eq!(std::fs::read(&cnf).unwrap(), before);
    }
}

/// Four rows that fill four columns, and `extra` in the first of those columns: the
/// tight rows force those literals false with no other literal in the reason.
fn tight_block(f: &mut Cnf, extra: &[i64]) {
    let table: Vec<Vec<i64>> = (0..4).map(|_| (0..4).map(|_| f.fresh()).collect()).collect();
    for r in &table {
        f.add(r);
    }
    for j in 0..4 {
        let mut col = column(&table, j);
        if j < extra.len() {
            col.push(extra[j]);
        }
        f.pairs(&col);
    }
}

#[test]
fn a_unit_stays_whatever_the_shortest_lemma_allowed() {
    // a fifth row with two literals in the tight columns and two ways out
    let fx = Fixture::new("card-minlen");
    let cnf = fx.path("f.cnf");
    let mut f = Cnf::new(0);
    let (a, b, y, z) = (f.fresh(), f.fresh(), f.fresh(), f.fresh());
    f.add(&[a, b, y, z]);
    tight_block(&mut f, &[a, b]);
    f.write(&cnf);
    let on = run(&cnf, &["--card=1", "--cardminlen=3", "--lucky=0"], &[]);
    let line = card_line(&on.stdout);
    assert_eq!(on.status, "SATISFIABLE", "{}", on.stdout);
    assert!(number(&line, "units") > 0, "{}", on.stdout);
    assert_eq!(number(&line, "too_short"), 0, "{}", on.stdout);
}

#[test]
fn a_conflict_from_the_engine_counts_as_a_conflict() {
    // a board the engine refutes in its first look, before any decision
    let fx = Fixture::new("card-conflicts");
    let cnf = fx.path("f.cnf");
    over_full_board(7).write(&cnf);
    let on = run(&cnf, &["--card=1"], &[]);
    assert_eq!(on.status, "UNSATISFIABLE", "{}", on.stdout);
    let line = card_line(&on.stdout);
    assert!(number(&line, "violations") >= 1, "{}", on.stdout);
    assert!(on.stats.get("conflicts").copied().unwrap_or(0) >= number(&line, "violations"), "{}", on.stdout);
}

#[test]
fn a_root_unit_marks_the_networks_it_tightens() {
    // a larger grid is read first and finds nothing: six rows over five columns,
    // one of them with x as a way out. The tight block then forces x false. The
    // unit leaves the trail at once, and the larger grid has to be read again
    let fx = Fixture::new("card-flush");
    let cnf = fx.path("f.cnf");
    let mut f = Cnf::new(0);
    let x = f.fresh();
    let table: Vec<Vec<i64>> = (0..6).map(|_| (0..5).map(|_| f.fresh()).collect()).collect();
    for (i, r) in table.iter().enumerate() {
        let mut row = r.clone();
        if i == 5 {
            row.push(x);
        }
        f.add(&row);
    }
    for j in 0..5 {
        f.pairs(&column(&table, j));
    }
    tight_block(&mut f, &[x]);
    f.write(&cnf);
    let options = ["--card=1", "--lucky=0", "--probe=0", "--eliminate=0"];
    let on = run(&cnf, &options, &[]);
    assert_eq!(on.status, "UNSATISFIABLE", "{}", on.stdout);
    assert_eq!(on.stats.get("decisions").copied().unwrap_or(0), 0, "{}", on.stdout);
    let proved = refuted_through_the_gate(&fx, &cnf, &options, "flush");
    assert!(number(&proved, "units") >= 1, "{:?}", proved);
    assert!(number(&proved, "violations") >= 1, "{:?}", proved);
}

#[test]
fn a_budget_that_covers_nothing_costs_nothing() {
    // the tables are not built, so they are not charged: the same work as with the
    // engine off, and the answer the stock path gets inside the tick limit
    let fx = Fixture::new("card-nobudget");
    let cnf = fx.path("f.cnf");
    std::fs::write(&cnf, "p cnf 2000000 1\n1 2000000 0\n").unwrap();
    let env = [("SAT_LIMIT_TICKS", "10000")];
    let on = run(&cnf, &["--card=1", "--cardreadticks=0"], &env);
    let off = run(&cnf, &["--card=0"], &env);
    assert_eq!(off.status, "SATISFIABLE", "{}", off.stdout);
    assert_eq!(on.status, "SATISFIABLE", "{}", on.stdout);
    assert_eq!(on.workclock.get("work"), off.workclock.get("work"), "{}\n{}", on.stdout, off.stdout);
}

#[test]
fn the_tick_limit_holds_through_the_read_and_the_build() {
    // overlapping counts over one board: at every limit the structure stage stops at
    // the limit, wherever in the reading or the building that falls
    let fx = Fixture::new("card-limit");
    let cnf = fx.path("f.cnf");
    let (mut f, table) = grid(8, 8);
    f.clauses.clear();
    for r in &table {
        f.pairs(r);
    }
    for j in 0..8 {
        f.pairs(&column(&table, j));
    }
    let negations: Vec<i64> = table.iter().flatten().map(|&x| -x).collect();
    for m in (30..64).step_by(6) {
        f.at_most(&negations[..m], m - 3);
    }
    f.write(&cnf);
    let whole = run(&cnf, &["--card=1", "--conflicts=0"], &[]);
    let top = whole.work();
    for k in 1..60 {
        let limit = top * k / 60;
        let limit_text = limit.to_string();
        let on = run(&cnf, &["--card=1", "--conflicts=0"], &[("SAT_LIMIT_TICKS", limit_text.as_str())]);
        assert!(on.work() <= limit + 200 + limit / 100, "limit {}: {}", limit, on.stdout);
    }
}

#[test]
fn the_tick_limit_holds_for_tied_counts() {
    // two counts tied by many copies of one clause: one network of 20,000 rows
    let fx = Fixture::new("card-tied");
    let cnf = fx.path("f.cnf");
    let mut f = Cnf::new(8);
    f.at_most(&[1, 2, 3, 4], 2);
    f.at_most(&[5, 6, 7, 8], 2);
    for _ in 0..20_000 {
        f.add(&[1, 5]);
    }
    f.write(&cnf);
    let whole = run(&cnf, &["--card=1", "--cardmulti=1", "--conflicts=0"], &[]);
    let top = whole.work();
    for k in 1..40 {
        let limit = top * k / 40;
        let limit_text = limit.to_string();
        let on = run(&cnf, &["--card=1", "--cardmulti=1", "--conflicts=0"], &[("SAT_LIMIT_TICKS", limit_text.as_str())]);
        assert!(on.work() <= limit + 200 + limit / 100, "limit {}: {}", limit, on.stdout);
    }
}

#[test]
fn reading_leaves_the_parity_engine_its_budget() {
    // XOR chains with a small structure budget: the parity engine's read comes first,
    // so turning the engine on takes nothing from it
    let fx = Fixture::new("card-parity");
    let cnf = fx.path("f.cnf");
    let mut f = Cnf::new(0);
    let vars: Vec<i64> = (0..600).map(|_| f.fresh()).collect();
    // x0 + x1 + x2 = 1, x1 + x2 + x3 = 1, ... and the sum that contradicts them
    let mut xor3 = |a: i64, b: i64, c: i64, odd: bool| {
        for sa in [1, -1] {
            for sb in [1, -1] {
                for sc in [1, -1] {
                    let negatives = (sa < 0) as i32 + (sb < 0) as i32 + (sc < 0) as i32;
                    // the clause forbids the assignment it makes all false
                    if (negatives % 2 == 1) == odd {
                        continue;
                    }
                    f.add(&[sa * a, sb * b, sc * c]);
                }
            }
        }
    };
    for w in vars.windows(3).step_by(2) {
        xor3(w[0], w[1], w[2], true);
    }
    f.write(&cnf);
    let args = ["--gauss=1", "--structticks=100", "--conflicts=1000"];
    let off: Vec<&str> = args.iter().copied().chain(["--card=0"]).collect();
    let on: Vec<&str> = args.iter().copied().chain(["--card=1"]).collect();
    let a = common::run(&cnf, &off, &[]);
    let b = common::run(&cnf, &on, &[]);
    assert_eq!(a.workclock.get("gauss_ticks").is_some(), true, "{}", a.stdout);
    assert_eq!(a.workclock.get("gauss_ticks"), b.workclock.get("gauss_ticks"), "{}\n{}", a.stdout, b.stdout);
}

#[test]
fn a_build_budget_covers_a_long_clause_whole() {
    // one clause longer than the budget: it is not read, and its cost is not charged
    let fx = Fixture::new("card-longclause");
    let cnf = fx.path("f.cnf");
    let lits: Vec<String> = (1..=100_000).map(|l| l.to_string()).collect();
    std::fs::write(&cnf, format!("p cnf 100000 1\n{} 0\n", lits.join(" "))).unwrap();
    let on = run(&cnf, &["--card=1", "--cardreadticks=250"], &[]);
    let line = card_line(&on.stdout);
    let read: u64 = line["read_ticks"].split(',').map(|x| x.parse::<u64>().unwrap()).sum();
    assert!(read <= 3 * 250_000, "{}", on.stdout);
    assert_eq!(on.status, "SATISFIABLE", "{}", on.stdout);
}

#[test]
fn a_columns_proof_is_planned_with_its_pairs() {
    // 100 rows over three columns: each column's proof names 4,950 pairs, which a
    // plan of lines alone would miss, so a small proof budget refuses it
    let fx = Fixture::new("card-colplan");
    let cnf = fx.path("f.cnf");
    let (mut f, table) = grid(100, 3);
    f.clauses.clear();
    for r in &table {
        f.add(r);
    }
    for j in 0..3 {
        f.pairs(&column(&table, j));
    }
    f.write(&cnf);
    let on = run(&cnf, &["--card=1", "--lucky=0", "--cardproofmax=1"], &[]);
    let line = card_line(&on.stdout);
    assert_eq!(number(&line, "nets"), 0, "{}", on.stdout);
    // what `pol` writes for each column, counted by its terms: 10,290 of them
    let wide = run(&cnf, &["--card=1", "--lucky=0"], &[]);
    let line = card_line(&wide.stdout);
    assert!(number(&line, "planned_terms") >= 3 * 10_290, "{}", wide.stdout);
}

#[test]
fn one_look_stays_inside_its_limit() {
    // a grid whose read costs far more than one look may spend
    let fx = Fixture::new("card-look");
    let cnf = fx.path("f.cnf");
    let (mut f, table) = grid(70, 70);
    f.clauses.clear();
    for r in &table {
        f.add(r);
    }
    for j in 0..70 {
        f.pairs(&column(&table, j));
    }
    f.write(&cnf);
    let base = ["--card=1", "--cardint=1", "--cardgrow=0", "--cardyield=0", "--lucky=0"];
    // larger than the limit from the start: never read, and it costs next to nothing
    let small: Vec<&str> = base.iter().copied().chain(["--cardlook=1"]).collect();
    let on = run(&cnf, &small, &[]);
    let line = card_line(&on.stdout);
    assert!(number(&line, "checks") > 0, "{}", on.stdout);
    assert_eq!(number(&line, "nets_read"), 0, "{}", on.stdout);
    assert!(number(&line, "no_room") > 0, "{}", on.stdout);
    assert!(number(&line, "look_max") <= 1000 + 200, "{}", on.stdout);
    // room for its size but not for the whole read: it ends at the limit, once
    let middle: Vec<&str> = base.iter().copied().chain(["--cardlook=20"]).collect();
    let on = run(&cnf, &middle, &[]);
    let line = card_line(&on.stdout);
    assert_eq!(number(&line, "cut_short"), 1, "{}", on.stdout);
    assert_eq!(number(&line, "nets_read"), 1, "{}", on.stdout);
    assert!(number(&line, "look_max") <= 20_000 + 200, "{}", on.stdout);
    // a count whose proof costs more than a look may spend, and networks that share
    // their literals: neither the proof nor the trail goes past the limit
    let board = fx.path("board.cnf");
    over_full_board(8).write(&board);
    let on = run(&board, &["--card=1", "--cardlook=1", "--cardint=1", "--lucky=0"], &[]);
    let line = card_line(&on.stdout);
    assert!(number(&line, "no_room") > 0, "{}", on.stdout);
    assert!(number(&line, "look_max") <= 1000 + 1000, "{}", on.stdout);
    // a matching that takes most of the limit: the sum after it is not started
    let php = fx.path("php.cnf");
    let (mut f, table) = grid(31, 30);
    f.clauses.clear();
    for r in &table {
        f.add(r);
    }
    for j in 0..30 {
        f.pairs(&column(&table, j));
    }
    f.write(&php);
    let on = run(&php, &["--card=1", "--cardlook=6", "--lucky=0", "--probe=0", "--eliminate=0", "--conflicts=1"], &[]);
    let line = card_line(&on.stdout);
    assert!(number(&line, "look_max") <= 6000 + 200, "{}", on.stdout);
    // nested scans, a capacity scan over rows with many edges each
    let wide = fx.path("wide.cnf");
    let (mut f, table) = grid(70, 71);
    f.clauses.clear();
    for r in &table {
        f.add(r);
    }
    for j in 0..71 {
        f.pairs(&column(&table, j));
    }
    f.write(&wide);
    for look in ["--cardlook=10", "--cardlook=20", "--cardlook=40"] {
        let on = run(&wide, &["--card=1", look, "--lucky=0", "--probe=0", "--eliminate=0", "--conflicts=1"], &[]);
        let line = card_line(&on.stdout);
        let most: u64 = look.rsplit('=').next().unwrap().parse().unwrap();
        assert!(number(&line, "look_max") <= most * 1000 + 200, "{}: {}", look, on.stdout);
    }
    // no limit on the look, the effort limit used up: whole reads, later
    let free = run(&cnf, &base, &[]);
    let line = card_line(&free.stdout);
    assert!(number(&line, "nets_read") > 100, "{}", free.stdout);
    assert!(number(&line, "cut_short") * 10 <= number(&line, "nets_read"), "{}", free.stdout);
}

#[test]
fn the_file_of_constraints_is_gone() {
    // the prototype's input had no proofs; only the readers feed the engine now
    let fx = Fixture::new("card-file");
    let cnf = fx.path("f.cnf");
    let (f, _) = grid(3, 3);
    f.write(&cnf);
    let list = fx.path("list.card");
    std::fs::write(&list, "K 1 1 2 3\nK 2 -1 -2 -3\nN 0 | 1\n").unwrap();
    let on = run(&cnf, &["--card=1"], &[("SAT_CARD_FILE", list.to_str().unwrap())]);
    assert_eq!(number(&card_line(&on.stdout), "nets"), 0, "{}", on.stdout);
}
