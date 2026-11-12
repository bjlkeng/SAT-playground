//! Not in kissat: the free-choice walk (`freewalk`, src/freewalk.rs). Off and
//! on a cell the file test turns down, the search is the stock one; on, the
//! walk changes nothing the search reads, so the search after it, between its
//! turns or around its rephase slots is the stock search, only later; its
//! work is in the work clock; a model it finds is answered and satisfies
//! every input clause.

mod common;

use common::{assert_same_trajectory, run, Fixture, Run};
use std::collections::BTreeMap;
use std::path::Path;

/// The walk's toys (tools/free_walk.py fixtures), as in the unit tests.
const TOYS: &str = include_str!("fixtures/freewalk_toys.txt");

/// The DIMACS text of one toy.
fn toy(name: &str) -> String {
    let mut out = String::new();
    let mut on = false;
    for line in TOYS.lines() {
        if let Some(n) = line.strip_prefix("toy ") {
            on = n == name;
        } else if line == "end" {
            if on {
                return out;
            }
        } else if on && !line.starts_with("sat ") {
            out.push_str(line);
            out.push('\n');
        }
    }
    panic!("no toy {}", name);
}

/// The broad test: pigeonhole in groups has no definition, so only the broad
/// test admits it (the default is the strict test since 2026-11-12).
const BROAD: &str = "--freewalktest=3";

/// Pigeonhole with n + 1 pigeons and n holes, each pigeon in exactly one hole
/// (a group per pigeon) and at most one pigeon per hole: unsatisfiable, and
/// the walk can never satisfy the holes' nogoods.
fn php_groups(n: usize, path: &Path) -> Vec<Vec<i64>> {
    let var = |p: usize, h: usize| (p * n + h + 1) as i64;
    let mut cl: Vec<Vec<i64>> = Vec::new();
    for p in 0..=n {
        cl.push((0..n).map(|h| var(p, h)).collect());
        for a in 0..n {
            for b in a + 1..n {
                cl.push(vec![-var(p, a), -var(p, b)]);
            }
        }
    }
    for h in 0..n {
        for p in 0..=n {
            for q in p + 1..=n {
                cl.push(vec![-var(p, h), -var(q, h)]);
            }
        }
    }
    write(path, (n + 1) * n, &cl);
    cl
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

/// The `c freewalk` line's fields.
fn walk_line(r: &Run) -> BTreeMap<String, String> {
    let line = r.stdout.lines().find(|l| l.starts_with("c freewalk ")).unwrap_or_else(|| panic!("no freewalk line in:\n{}", r.stdout));
    line.split_whitespace().filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn num(m: &BTreeMap<String, String>, k: &str) -> u64 {
    m.get(k).unwrap_or_else(|| panic!("no {} in {:?}", k, m)).parse().unwrap()
}

/// The same search: the status and every `-s` counter, and the work clock
/// line apart from the total ticks and W (the reading and the walk add to
/// those).
fn assert_same_search(a: &Run, b: &Run, what: &str) {
    assert_eq!(a.status, b.status, "{}: status differs", what);
    assert_eq!(a.stats, b.stats, "{}: -s counters differ", what);
    let strip = |w: &BTreeMap<String, u64>| -> BTreeMap<String, u64> { w.iter().filter(|(k, _)| k.as_str() != "ticks" && k.as_str() != "work").map(|(k, v)| (k.clone(), *v)).collect() };
    assert_eq!(strip(&a.workclock), strip(&b.workclock), "{}: work clock line differs", what);
    assert!(b.workclock["ticks"] >= a.workclock["ticks"], "{}: the reading's and the walk's work are added", what);
}

/// The model of a run that printed one (no `-n`), checked against the clauses.
fn check_model(stdout: &str, clauses: &[Vec<i64>], vars: usize) {
    assert!(stdout.contains("s SATISFIABLE"), "{}", stdout);
    let lits: Vec<i64> = stdout.lines().filter_map(|l| l.strip_prefix("v ")).flat_map(|l| l.split_whitespace().map(|t| t.parse::<i64>().unwrap())).collect();
    assert_eq!(lits.last(), Some(&0), "the model ends with 0");
    let mut val = vec![None; vars + 1];
    for &l in &lits[..lits.len() - 1] {
        let v = l.unsigned_abs() as usize;
        assert!(v >= 1 && v <= vars, "a variable out of range: {}", l);
        assert!(val[v].is_none(), "variable {} printed twice", v);
        val[v] = Some(l > 0);
    }
    assert!(val[1..].iter().all(|x| x.is_some()), "every declared variable has a value");
    for c in clauses {
        assert!(c.iter().any(|&l| val[l.unsigned_abs() as usize] == Some(l > 0)), "clause {:?} is false", c);
    }
}

fn run_with_model(cnf: &Path, options: &[&str]) -> String {
    let mut cmd = std::process::Command::new(common::solver_bin());
    cmd.args(common::STOCK_PATH).args(options).arg(cnf);
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    String::from_utf8_lossy(&cmd.output().expect("run sat-solver").stdout).into_owned()
}

#[test]
fn off_and_on_a_cell_the_test_turns_down_the_search_is_stock() {
    let fx = Fixture::new("freewalk-declined");
    // random 3-SAT: no group and no definition, so every test turns it down
    let cnf = fx.path("random.cnf");
    common::random_3sat(200, 4.26, 7, &cnf);
    let stock = run(&cnf, &[], &[]);
    assert_same_trajectory(&stock, &run(&cnf, &["--freewalk=0"], &[]), "freewalk=0 v no option");
    for test in ["--freewalktest=1", "--freewalktest=2", "--freewalktest=3"] {
        let on = run(&cnf, &["--freewalk=2", test], &[]);
        assert_same_search(&stock, &on, test);
        assert_eq!(walk_line(&on)["status"], "declined", "{}", test);
    }
    // pigeons in groups, no definition: the strict and loose tests turn it down
    let cnf = fx.path("php.cnf");
    php_groups(6, &cnf);
    let stock = run(&cnf, &[], &[]);
    for test in ["--freewalktest=1", "--freewalktest=2"] {
        let on = run(&cnf, &["--freewalk=2", test], &[]);
        assert_same_search(&stock, &on, test);
        let line = walk_line(&on);
        assert_eq!(line["status"], "declined");
        assert_eq!(line["read_groups"], "7");
    }
}

#[test]
fn the_walk_first_stops_at_its_budget_and_the_search_after_it_is_stock() {
    let fx = Fixture::new("freewalk-first");
    let cnf = fx.path("php.cnf");
    php_groups(8, &cnf);
    let stock = run(&cnf, &[], &[]);
    assert_eq!(stock.status, "UNSATISFIABLE");
    assert!(stock.stats["conflicts"] > 1000, "the search has work to do");
    let first = run(&cnf, &["--freewalk=1", BROAD, "--freewalkticks=2000"], &[]);
    assert_same_search(&stock, &first, "walk first, then search");
    let line = walk_line(&first);
    assert_eq!(line["status"], "walked");
    assert_eq!(line["placement"], "first");
    let work = num(&line, "work");
    assert!(work >= 2_000_000 && work < 2_100_000, "the walk stops within a step of its budget ({})", work);
    assert!(first.workclock["ticks"] >= stock.workclock["ticks"] + work, "the walk's work is in the work clock");
    // the same run twice: the same walk
    let again = run(&cnf, &["--freewalk=1", BROAD, "--freewalkticks=2000"], &[]);
    let (a, b) = (walk_line(&first), walk_line(&again));
    for k in ["steps", "trials", "flips", "units", "work", "best", "hash"] {
        assert_eq!(a[k], b[k], "{} differs between two runs", k);
    }
    assert_eq!(first.workclock, again.workclock);
}

#[test]
fn turns_with_the_search_leave_the_search_as_it_was_and_take_their_share() {
    let fx = Fixture::new("freewalk-turns");
    let cnf = fx.path("php.cnf");
    php_groups(8, &cnf);
    let stock = run(&cnf, &[], &[]);
    let turns = run(&cnf, &["--freewalk=2", BROAD, "--freewalkshare=250", "--freewalkturn=100"], &[]);
    assert_same_search(&stock, &turns, "turns");
    let line = walk_line(&turns);
    assert_eq!(line["status"], "walked");
    let runs = num(&line, "runs");
    assert!(runs > 3, "the walk took several turns ({})", runs);
    let share = num(&line, "work") as f64 / turns.workclock["work"] as f64;
    assert!(share > 0.2 && share < 0.26, "the walk's share of the work clock ({:.3})", share);
    // a walk that runs out of steps ends its turns, and the search goes on as before
    let short = run(&cnf, &["--freewalk=2", BROAD, "--freewalkshare=250", "--freewalkturn=100", "--freewalksteps=5"], &[]);
    assert_same_search(&stock, &short, "turns that run out of steps");
    assert_eq!(num(&walk_line(&short), "steps"), 5);
}

#[test]
fn rephase_slots_leave_the_search_as_it_was() {
    let fx = Fixture::new("freewalk-rephase");
    let cnf = fx.path("php.cnf");
    php_groups(8, &cnf);
    // early and frequent rephasing, so walking slots come up on a small formula
    let early: &[&str] = &["--rephaseinit=10", "--rephaseint=10", "--stable=2"];
    let stock = run(&cnf, early, &[]);
    let mut on: Vec<&str> = early.to_vec();
    on.extend(["--freewalk=3", BROAD, "--freewalkshare=250"]);
    let slots = run(&cnf, &on, &[]);
    assert_same_search(&stock, &slots, "rephase slots");
    let line = walk_line(&slots);
    assert_eq!(line["placement"], "rephase");
    assert!(num(&line, "runs") > 0, "the walk ran in a slot");
    assert!(num(&line, "work") > 0);
}

#[test]
fn the_shipped_defaults_walk_first_with_the_strict_test() {
    // on by default since 2026-11-12 (the owner's call after the 400-cell run,
    // bead d96.2.5): first, the strict test, 7,200,000 K. The other passes off,
    // the walk at its defaults
    let others: Vec<&str> = common::STOCK_PATH.iter().copied().filter(|o| !o.starts_with("--freewalk")).collect();
    assert_eq!(others.len() + 1, common::STOCK_PATH.len(), "the stock path turns the walk off");
    let fx = Fixture::new("freewalk-defaults");
    let cnf = fx.path("lockchart.cnf");
    std::fs::write(&cnf, toy("lockchart-big")).unwrap();
    let on = common::run_shipped(&cnf, &others, &[]);
    let line = walk_line(&on);
    assert_eq!((line["placement"].as_str(), line["test"].as_str(), line["status"].as_str()), ("first", "strict", "found"), "{}", on.stdout);
    assert_eq!(on.status, "SATISFIABLE");
    // random 3-SAT: the strict test turns it down, and the search is the stock one
    let cnf = fx.path("random.cnf");
    common::random_3sat(200, 4.26, 7, &cnf);
    let shipped = common::run_shipped(&cnf, &others, &[]);
    let line = walk_line(&shipped);
    assert_eq!((line["test"].as_str(), line["status"].as_str()), ("strict", "declined"));
    assert_same_search(&run(&cnf, &[], &[]), &shipped, "the shipped walk v the stock path");
}

#[test]
fn a_model_the_walk_finds_is_answered_and_satisfies_every_input_clause() {
    let fx = Fixture::new("freewalk-model");
    let text = toy("lockchart-big");
    let cnf = fx.path("lockchart.cnf");
    std::fs::write(&cnf, &text).unwrap();
    let clauses: Vec<Vec<i64>> = text.lines().filter(|l| !l.starts_with('p') && !l.starts_with('c')).map(|l| l.split_whitespace().map(|t| t.parse::<i64>().unwrap()).filter(|&x| x != 0).collect()).collect();
    let vars: usize = text.lines().find(|l| l.starts_with("p cnf")).unwrap().split_whitespace().nth(2).unwrap().parse().unwrap();
    // first: the walk answers before the search starts
    let out = run_with_model(&cnf, &["--freewalk=1"]);
    check_model(&out, &clauses, vars);
    assert!(out.contains("status=found"), "{}", out);
    let r = run(&cnf, &["--freewalk=1"], &[]);
    assert_eq!(r.status, "SATISFIABLE");
    assert_eq!(r.stats.get("decisions").copied().unwrap_or(0), 0, "no search ran");
    // a header that declares more variables than the clauses use: every one printed
    let wide = fx.path("wide.cnf");
    std::fs::write(&wide, text.replacen(&format!("p cnf {} ", vars), &format!("p cnf {} ", vars + 5), 1)).unwrap();
    let out = run_with_model(&wide, &["--freewalk=1"]);
    check_model(&out, &clauses, vars + 5);
    // turns: the walk's first turn, owed nearly all of the work so far, finds it
    let out = run_with_model(&cnf, &["--freewalk=2", "--freewalkshare=990", "--freewalkturn=1", "--luckyearly=0", "--luckylate=0"]);
    check_model(&out, &clauses, vars);
}

#[test]
fn the_tick_limit_ends_the_walk_and_the_run() {
    let fx = Fixture::new("freewalk-limit");
    let cnf = fx.path("php.cnf");
    php_groups(8, &cnf);
    let limited = run(&cnf, &["--freewalk=1", BROAD, "--freewalkticks=2147483647"], &[("SAT_LIMIT_TICKS", "3000000")]);
    assert_eq!(limited.status, "UNKNOWN");
    let w = limited.work();
    assert!(w >= 3_000_000 && w < 3_100_000, "the run stops at the limit, within a step ({})", w);
    assert_eq!(limited.stats.get("conflicts").copied().unwrap_or(0), 0, "no search after the walk used the budget");
}
