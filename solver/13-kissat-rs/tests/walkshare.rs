//! Not in kissat: the local-search share (`walkshare`, `walkstall`). Off is
//! the stock trajectory; on, a walk may use its share of the work clock,
//! its steps count in the clock, and the work limit still ends the run.

mod common;

use common::{assert_same_trajectory, run, Fixture};
use std::path::Path;

/// Uniform random 3-SAT with a planted solution (every clause has a
/// literal true under it), from a fixed seed.
fn planted_3sat(vars: usize, clauses: usize, seed: u64, path: &Path) -> Vec<Vec<i64>> {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };
    let hidden: Vec<bool> = (0..vars).map(|_| next() % 2 == 1).collect();
    let mut out: Vec<Vec<i64>> = Vec::new();
    while out.len() < clauses {
        let mut c: Vec<i64> = Vec::new();
        while c.len() < 3 {
            let v = next() % vars;
            if c.iter().any(|l| l.unsigned_abs() as usize == v + 1) {
                continue;
            }
            c.push(if next() % 2 == 1 { (v + 1) as i64 } else { -((v + 1) as i64) });
        }
        if c.iter().any(|&l| (l > 0) == hidden[l.unsigned_abs() as usize - 1]) {
            out.push(c);
        }
    }
    let mut text = format!("p cnf {} {}\n", vars, out.len());
    for c in &out {
        for l in c {
            text.push_str(&format!("{} ", l));
        }
        text.push_str("0\n");
    }
    std::fs::write(path, text).unwrap();
    out
}

fn model(stdout: &str) -> Vec<i64> {
    stdout
        .lines()
        .filter_map(|l| l.strip_prefix("v "))
        .flat_map(|l| l.split_whitespace().map(|t| t.parse::<i64>().unwrap()))
        .filter(|&l| l != 0)
        .collect()
}

// early and frequent rephasing, so walks happen on a small formula, and a
// tiny stock walk limit, so the share is what gives a walk its length
const EARLY_WALKS: &[&str] = &["--rephaseinit=10", "--rephaseint=10", "--walkeffort=1"];

#[test]
fn share_off_is_the_stock_trajectory_and_on_it_walks_longer() {
    let fx = Fixture::new("walkshare");
    let cnf = fx.path("planted.cnf");
    let clauses = planted_3sat(400, 1680, 20261012, &cnf);
    let stock = run(&cnf, EARLY_WALKS, &[]);
    let mut off: Vec<&str> = EARLY_WALKS.to_vec();
    off.extend(["--walkshare=0", "--walkstall=0"]);
    assert_same_trajectory(&stock, &run(&cnf, &off, &[]), "walkshare=0 v no option");
    assert_eq!(stock.status, "SATISFIABLE");
    let mut on: Vec<&str> = EARLY_WALKS.to_vec();
    on.push("--walkshare=1000");
    let shared = run(&cnf, &on, &[]);
    assert_eq!(shared.status, "SATISFIABLE");
    // (`run` asks for no witness; the model comes from a run that prints it)
    let mut cmd = std::process::Command::new(common::solver_bin());
    cmd.args(common::STOCK_PATH).args(&on).arg(&cnf);
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    let printed = String::from_utf8_lossy(&cmd.output().expect("run sat-solver").stdout).into_owned();
    assert!(printed.contains("s SATISFIABLE"), "{}", printed);
    let m = model(&printed);
    for c in &clauses {
        assert!(c.iter().any(|l| m.contains(l)), "clause {:?} unsatisfied", c);
    }
    // the walks of the shared run were longer, per walk
    let per_walk = |r: &common::Run| r.workclock["walk_steps"] as f64 / (r.stats.get("walks").copied().unwrap_or(0).max(1) as f64);
    assert!(shared.stats.get("walks").copied().unwrap_or(0) >= 1, "{}", shared.stdout);
    assert!(per_walk(&shared) > 4.0 * per_walk(&stock), "steps per walk {} v {}\n{}", per_walk(&shared), per_walk(&stock), shared.stdout);
    // and their steps are in the work clock: against the stock run stopped
    // at the same conflict, the clock is ahead by the walk's steps over
    // eight (and by the search ticks made after the walk)
    let conflicts = format!("--conflicts={}", shared.stats["conflicts"]);
    let mut cut: Vec<&str> = EARLY_WALKS.to_vec();
    cut.push(&conflicts);
    let stopped = run(&cnf, &cut, &[]);
    let ahead = shared.workclock["ticks"] as i64 - stopped.workclock["ticks"] as i64;
    let searched = shared.workclock["search_ticks"] as i64 - stopped.workclock["search_ticks"] as i64;
    let walked = (shared.workclock["walk_steps"] / 8) as i64;
    assert!(walked > 10_000, "{}", shared.stdout);
    assert!((ahead - searched - walked).abs() <= walked / 50 + 2000, "clock ahead by {}, search by {}, walk steps over eight {}", ahead, searched, walked);
}

#[test]
fn the_work_limit_ends_a_shared_walk() {
    let fx = Fixture::new("walkshare-limit");
    let cnf = fx.path("planted.cnf");
    planted_3sat(400, 1680, 20261012, &cnf);
    // the shared run is solved by its first long walk; the stock run
    // stopped at that conflict gives the clock at which the walk began
    let mut on: Vec<&str> = EARLY_WALKS.to_vec();
    on.push("--walkshare=1000");
    let full = run(&cnf, &on, &[]);
    assert_eq!(full.status, "SATISFIABLE");
    assert_eq!(full.stats["walks"], 1, "{}", full.stdout);
    let conflicts = format!("--conflicts={}", full.stats["conflicts"]);
    let mut cut: Vec<&str> = EARLY_WALKS.to_vec();
    cut.push(&conflicts);
    let before = run(&cnf, &cut, &[]);
    let walked = full.workclock["walk_steps"] / 8;
    assert!(walked > 30_000, "{}", full.stdout);
    // a limit half way into the walk: the run ends there, unsolved, with
    // about half of the walk made, and twice the same
    let limit = before.workclock["ticks"] + walked / 2;
    let value = limit.to_string();
    let r = run(&cnf, &on, &[("SAT_LIMIT_TICKS", &value)]);
    assert_eq!(r.status, "UNKNOWN", "{}", r.stdout);
    assert_eq!(r.stats["walks"], 1, "{}", r.stdout);
    let steps = r.workclock["walk_steps"];
    assert!(steps > full.workclock["walk_steps"] / 5 && steps < full.workclock["walk_steps"] * 4 / 5, "walk steps {} of {}", steps, full.workclock["walk_steps"]);
    assert!(r.work() >= limit && r.work() < limit + 20_000, "limit {}: stopped at {}", limit, r.work());
    assert_same_trajectory(&r, &run(&cnf, &on, &[("SAT_LIMIT_TICKS", &value)]), "two runs, one work limit");
}

#[test]
fn a_stalled_walk_ends_early() {
    let fx = Fixture::new("walkshare-stall");
    // unsatisfiable: no walk reaches zero, each one settles at its minimum
    let cnf = fx.path("php9.cnf");
    common::php(9, &cnf);
    let mut on: Vec<&str> = EARLY_WALKS.to_vec();
    on.extend(["--walkshare=1000", "--conflicts=3000"]);
    let free = run(&cnf, &on, &[]);
    on.push("--walkstall=2");
    let stalled = run(&cnf, &on, &[]);
    assert!(free.stats.get("walks").copied().unwrap_or(0) >= 1, "{}", free.stdout);
    assert!(stalled.workclock["walk_steps"] < free.workclock["walk_steps"], "{} v {}", stalled.workclock["walk_steps"], free.workclock["walk_steps"]);
}
