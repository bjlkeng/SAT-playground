//! Nudges toward the real unknowns (bead SAT-playground-d96.14.9,
//! src/guide.rs; an experiment, every option off by default).
//! Checked here: off means off, on with no dial is the old search, every
//! nudge runs and gives the plain search's answer with a model that
//! satisfies the formula, the preferred value is the value tried (also
//! through a replacement by an opposite variable), a kept variable is not
//! eliminated, and a setting that cannot be used stops the run.
//! And the roles from the solver's own free-choice reading (`roleread`,
//! bead SAT-playground-d96.14.6): alone it changes nothing; with the nudges
//! and no dial the search is the old one; the reading's counts are the
//! roles, its choices and free variables the preferred ones; with nothing to
//! tell apart, or no reading, the nudges stay off and say why; and the count
//! by role takes the reading's roles, with no role file read.

mod common;

use common::{php, random_3sat, run, run_shipped, solver_bin, Fixture, Run};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

type Line = BTreeMap<String, String>;

fn pairs(rest: &str) -> Line {
    rest.split_whitespace().filter_map(|t| t.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// The exit line `c guide list=...` of a run.
fn guide_line(stdout: &str) -> Line {
    let rest = stdout.lines().find_map(|l| l.strip_prefix("c guide list=")).unwrap_or_else(|| panic!("no guide line in:\n{}", stdout));
    pairs(&format!("list={}", rest))
}

fn num(line: &Line, key: &str) -> u64 {
    line.get(key).unwrap_or_else(|| panic!("no {} in {:?}", key, line)).parse().unwrap_or_else(|_| panic!("{} in {:?}", key, line))
}

/// The `c roles <kind> at=end ...` lines of a run.
fn role_lines(stdout: &str, kind: &str) -> Vec<Line> {
    let head = format!("c roles {} at=end ", kind);
    stdout.lines().filter_map(|l| l.strip_prefix(&head)).map(pairs).collect()
}

/// A count of a role, summed over the two search modes.
fn counted(stdout: &str, role: &str, key: &str) -> u64 {
    role_lines(stdout, "count").iter().filter(|l| l.get("role").map(String::as_str) == Some(role)).map(|l| num(l, key)).sum()
}

fn env_of<'a>(roles: &'a Path, vars: &'a str, values: &'a str) -> [(&'a str, &'a str); 3] {
    [("SAT_ROLE_FILE", roles.to_str().unwrap()), ("SAT_GUIDE_ROLES", vars), ("SAT_GUIDE_VALUE_ROLES", values)]
}

/// The first `k` variables are `pick`, the others `rest`.
fn two_roles(vars: usize, k: usize, path: &Path) {
    std::fs::write(path, format!("1-{} pick\n{}-{} rest\n", k, k + 1, vars)).unwrap();
}

/// Run without `-n` and check the printed model against the formula.
fn model_holds(cnf: &Path, options: &[&str], env: &[(&str, &str)]) -> bool {
    let mut cmd = Command::new(solver_bin());
    cmd.args(common::STOCK_PATH).args(options).arg(cnf);
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run sat-solver");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.lines().any(|l| l == "s SATISFIABLE"), "not satisfiable:\n{}", stdout);
    let mut truth: BTreeMap<i64, bool> = BTreeMap::new();
    for l in stdout.lines().filter_map(|l| l.strip_prefix("v ")) {
        for t in l.split_whitespace() {
            let x: i64 = t.parse().unwrap();
            if x != 0 {
                truth.insert(x.abs(), x > 0);
            }
        }
    }
    let text = std::fs::read_to_string(cnf).unwrap();
    let mut clause: Vec<i64> = Vec::new();
    for line in text.lines().filter(|l| !l.starts_with('p') && !l.starts_with('c')) {
        for t in line.split_whitespace() {
            let x: i64 = t.parse().unwrap();
            if x == 0 {
                if !clause.iter().any(|&y| truth.get(&y.abs()).copied() == Some(y > 0)) {
                    return false;
                }
                clause.clear();
            } else {
                clause.push(x);
            }
        }
    }
    true
}

/// Each nudge, with the counter of the exit line that shows it acted
/// ("" for one without a counter).
const NUDGES: &[(&[&str], &str)] = &[
    (&["--guidebump=800"], "bumped"),
    (&["--guidefront=1"], "fronted"),
    (&["--guidecause=50"], "credited"),
    (&["--guidewindow=8"], "by_window"),
    (&["--guiderestart=3"], "by_restart"),
    (&["--guiderandec=1", "--randecinit=20", "--randecint=20"], "by_random"),
    (&["--guidephase=1"], "valued"),
    (&["--guidephase=2"], "valued"),
    (&["--guidephase=1", "--guidephaseshare=50"], "valued"),
    (&["--guidewarm=1"], "by_warm"),
    (&["--guidewalk=400"], "walk_weighted"),
    (&["--guideinit=1"], ""),
    (&["--guideprotect=1"], ""),
    (
        &["--guidebump=6400", "--guidecause=50", "--guidefront=1", "--guidewindow=64", "--guiderestart=3", "--guiderandec=1", "--guidephase=1", "--guidewarm=1", "--guidewalk=400", "--guideinit=1", "--guideprotect=1"],
        "bumped",
    ),
];

#[test]
fn off_means_off() {
    let fx = Fixture::new("guide_off");
    let cnf = fx.path("r.cnf");
    random_3sat(120, 4.6, 3, &cnf);
    let plain = run(&cnf, &[], &[]);
    // every dial set and the module off: nothing is read (the file does not exist)
    let dials = ["--guidebump=800", "--guidecause=50", "--guidefront=1", "--guidewindow=8", "--guiderestart=3", "--guiderandec=1", "--guidephase=1", "--guidewarm=1", "--guidewalk=400", "--guideinit=1", "--guideprotect=1"];
    let off = run(&cnf, &dials, &[("SAT_ROLE_FILE", "/nonexistent/roles"), ("SAT_GUIDE_ROLES", "pick")]);
    common::assert_same_trajectory(&plain, &off, "the dials with guide off");
    assert_eq!(off.exit_code, plain.exit_code);
    assert!(!off.stdout.contains("c guide"), "no guide line with the module off");
}

#[test]
fn on_with_no_dial_is_the_old_search() {
    let fx = Fixture::new("guide_nodial");
    for (seed, ratio) in [(3u64, 4.6), (5, 4.0)] {
        let cnf = fx.path(&format!("r{}.cnf", seed));
        let roles = fx.path(&format!("r{}.roles", seed));
        random_3sat(140, ratio, seed, &cnf);
        two_roles(140, 30, &roles);
        let env = env_of(&roles, "pick", "pick");
        let plain = run(&cnf, &[], &[]);
        let on = run(&cnf, &["--guide=1"], &env);
        common::assert_same_trajectory(&plain, &on, "guide on with every dial off");
        assert_eq!(num(&guide_line(&on.stdout), "list"), 30);
        // and with the shipped defaults
        let plain = run_shipped(&cnf, &[], &[]);
        let on = run_shipped(&cnf, &["--guide=1"], &env);
        common::assert_same_trajectory(&plain, &on, "guide on with every dial off, shipped defaults");
    }
}

#[test]
fn every_nudge_runs_and_answers_as_the_plain_search() {
    let fx = Fixture::new("guide_each");
    // one formula with a model and one without, both needing a search
    for (name, vars, ratio, seed) in [("sat", 250usize, 4.15, 2u64), ("unsat", 160, 4.6, 3)] {
        let cnf = fx.path(&format!("{}.cnf", name));
        let roles = fx.path(&format!("{}.roles", name));
        random_3sat(vars, ratio, seed, &cnf);
        two_roles(vars, 40, &roles);
        let env = env_of(&roles, "pick", "pick");
        let plain = run(&cnf, &[], &[]);
        // long enough to reach the stable mode and a local search
        assert!(plain.stats["conflicts"] > 3000, "{}: the toy needs a search ({} conflicts)", name, plain.stats["conflicts"]);
        assert_eq!(plain.status, if name == "sat" { "SATISFIABLE" } else { "UNSATISFIABLE" }, "{}", name);
        for (options, counter) in NUDGES {
            let mut all: Vec<&str> = vec!["--guide=1"];
            all.extend_from_slice(options);
            let got: Run = run(&cnf, &all, &env);
            assert_eq!(got.status, plain.status, "{} with {:?}", name, options);
            assert_eq!(got.exit_code, plain.exit_code, "{} with {:?}", name, options);
            let line = guide_line(&got.stdout);
            if !counter.is_empty() {
                assert!(num(&line, counter) > 0, "{} with {:?}: {} is 0 in {:?}", name, options, counter, line);
            }
            if name == "sat" {
                assert!(model_holds(&cnf, &all, &env), "{} with {:?}: the model does not satisfy the formula", name, options);
            }
        }
    }
}

#[test]
fn the_preferred_value_is_the_value_tried() {
    let fx = Fixture::new("guide_value");
    let cnf = fx.path("r.cnf");
    let roles = fx.path("r.roles");
    random_3sat(140, 4.6, 3, &cnf);
    two_roles(140, 40, &roles);
    let env = env_of(&roles, "rest", "pick");
    let yes = run(&cnf, &["--rolecount=1", "--guide=1", "--guidephase=1"], &env);
    let (decided, tried_true) = (counted(&yes.stdout, "pick", "decide_stands"), counted(&yes.stdout, "pick", "true_stands"));
    assert!(decided > 0);
    assert_eq!(tried_true, decided, "true first: every decision on a preferred variable tries true");
    let no = run(&cnf, &["--rolecount=1", "--guide=1", "--guidephase=2"], &env);
    assert!(counted(&no.stdout, "pick", "decide_stands") > 0);
    assert_eq!(counted(&no.stdout, "pick", "true_stands"), 0, "false first: none tries true");
    // the other role's value stays the solver's own: some true, some false
    let (d, t) = (counted(&yes.stdout, "rest", "decide_stands"), counted(&yes.stdout, "rest", "true_stands"));
    assert!(t > 0 && t < d, "{} of {}", t, d);
    // a share of 50 turns about half of them
    let half = run(&cnf, &["--guide=1", "--guidephase=1", "--guidephaseshare=50"], &env);
    let line = guide_line(&half.stdout);
    assert!(num(&line, "turned") > 0 && num(&line, "turned") < num(&line, "valued"), "{:?}", line);
}

/// Random 3-SAT over `n` variables in which each of the first `k` variables
/// has a copy that is its opposite and takes about half of its places.
/// Roles: the originals `orig`, the copies `copy`, the rest `rest`.
fn planted_opposite(n: u64, k: u64, ratio: f64, seed: u64, cnf: &Path, roles: &Path) {
    let mut state = seed.wrapping_mul(0x9E3779B97F4A7C15) | 1;
    let mut below = |m: u64| -> u64 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) % m
    };
    let mut text = String::new();
    let m = (n as f64 * ratio) as u64;
    let mut count = 0u64;
    for _ in 0..m {
        let mut vs: Vec<u64> = Vec::new();
        while vs.len() < 3 {
            let v = 1 + below(n);
            if !vs.contains(&v) {
                vs.push(v);
            }
        }
        for v in vs {
            let copied = v <= k && below(2) == 0;
            let negative = below(2) == 0;
            // the copy stands in with the other sign
            let (v, negative) = if copied { (n + v, !negative) } else { (v, negative) };
            text.push_str(&format!("{}{} ", if negative { "-" } else { "" }, v));
        }
        text.push_str("0\n");
        count += 1;
    }
    for v in 1..=k {
        text.push_str(&format!("{} {} 0\n-{} -{} 0\n", v, n + v, v, n + v));
        count += 2;
    }
    std::fs::write(cnf, format!("p cnf {} {}\n{}", n + k, count, text)).unwrap();
    std::fs::write(roles, format!("1-{} orig\n{}-{} rest\n{}-{} copy\n", k, k + 1, n, n + 1, n + k)).unwrap();
}

#[test]
fn a_variable_replaced_by_its_opposite_gets_the_value_turned_round() {
    let fx = Fixture::new("guide_opposite");
    let cnf = fx.path("o.cnf");
    let roles = fx.path("o.roles");
    planted_opposite(220, 40, 4.3, 5, &cnf, &roles);
    let env = env_of(&roles, "orig", "orig");
    let got = run(&cnf, &["--rolecount=1", "--guide=1", "--guidephase=1", "--guiderestart=1"], &env);
    // the solver replaced originals by their copies: the list holds the copies' numbers too
    let line = guide_line(&got.stdout);
    assert!(num(&line, "list") > 40, "no replacement was followed: {:?}", line);
    let into_copy = role_lines(&got.stdout, "hold")
        .iter()
        .filter(|l| l.get("role").map(String::as_str) == Some("orig"))
        .map(|l| num(l, "merged_held") + num(l, "merged_fixed"))
        .sum::<u64>();
    assert!(into_copy > 0, "no original lives on as a copy");
    // every decision that stands for an original makes the original true,
    // whether the decided variable is the original or its opposite
    let (decided, tried_true) = (counted(&got.stdout, "orig", "decide_stands"), counted(&got.stdout, "orig", "true_stands"));
    assert!(decided > 0);
    assert_eq!(tried_true, decided);
    // and the decided variable was a copy for some of them: set false there
    let (copy_decided, copy_true) = (counted(&got.stdout, "copy", "decide_own"), counted(&got.stdout, "copy", "true_own"));
    assert!(copy_decided > 0, "no decision on a copy");
    assert_eq!(copy_true, 0, "a copy that stands for an original is set false");
    assert!(num(&line, "by_restart") > 0);
}

#[test]
fn the_head_start_takes_the_first_decisions_after_a_restart() {
    let fx = Fixture::new("guide_restart");
    let cnf = fx.path("r.cnf");
    let roles = fx.path("r.roles");
    random_3sat(200, 4.5, 2, &cnf);
    two_roles(200, 40, &roles);
    let env = env_of(&roles, "pick", "pick");
    // the first decisions after a restart, on a preferred variable and in all
    let firsts = |r: &Run| -> (u64, u64) { (counted(&r.stdout, "pick", "first_own"), counted(&r.stdout, "pick", "first_own") + counted(&r.stdout, "rest", "first_own")) };
    let plain = run(&cnf, &["--rolecount=1", "--guide=1"], &env);
    let (on_pick, all) = firsts(&plain);
    assert!(all > 100, "{} restarts with a decision after them", all);
    assert!(on_pick * 2 < all, "the toy tests nothing: {} of {} first decisions are on a preferred variable anyway", on_pick, all);
    for k in ["1", "3"] {
        let got = run(&cnf, &["--rolecount=1", "--guide=1", &format!("--guiderestart={}", k)], &env);
        let (on_pick, all) = firsts(&got);
        assert!(all > 100);
        // all of them, but for a restart at which every preferred variable had a value
        assert!(on_pick * 100 >= all * 98, "guiderestart={}: {} of {} first decisions are on a preferred variable", k, on_pick, all);
        let line = guide_line(&got.stdout);
        // each counted pick was a decision: the restart's own look ahead takes none
        let k: u64 = k.parse().unwrap();
        assert!(num(&line, "by_restart") >= on_pick, "{:?}", line);
        assert!(num(&line, "by_restart") <= k * got.stats["restarts"], "{:?} with {} restarts", line, got.stats["restarts"]);
        assert_eq!(got.status, plain.status);
    }
}

/// Like `planted_opposite` with equal copies, and with every copy named in
/// the file before its original: the solver then keeps the copy's name when
/// it finds the two equal.
fn planted_copy_first(n: u64, k: u64, ratio: f64, seed: u64, cnf: &Path, roles: &Path) {
    let mut state = seed.wrapping_mul(0x9E3779B97F4A7C15) | 1;
    let mut below = |m: u64| -> u64 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) % m
    };
    let mut text = String::new();
    let mut count = 0u64;
    for v in 1..=k {
        text.push_str(&format!("{} -{} 0\n-{} {} 0\n", n + v, v, n + v, v));
        count += 2;
    }
    let m = (n as f64 * ratio) as u64;
    for _ in 0..m {
        let mut vs: Vec<u64> = Vec::new();
        while vs.len() < 3 {
            let v = 1 + below(n);
            if !vs.contains(&v) {
                vs.push(v);
            }
        }
        for v in vs {
            let copied = v <= k && below(2) == 0;
            let negative = below(2) == 0;
            let v = if copied { n + v } else { v };
            text.push_str(&format!("{}{} ", if negative { "-" } else { "" }, v));
        }
        text.push_str("0\n");
        count += 1;
    }
    std::fs::write(cnf, format!("p cnf {} {}\n{}", n + k, count, text)).unwrap();
    std::fs::write(roles, format!("1-{} orig\n{}-{} rest\n{}-{} copy\n", k, k + 1, n, n + 1, n + k)).unwrap();
}

#[test]
fn the_first_order_goes_to_the_variables_that_are_there() {
    let fx = Fixture::new("guide_init");
    let cnf = fx.path("c.cnf");
    let roles = fx.path("c.roles");
    planted_copy_first(220, 40, 4.3, 5, &cnf, &roles);
    let env = env_of(&roles, "orig", "orig");
    // the solver replaces the originals by their copies before its first decision
    let plain = run(&cnf, &["--rolecount=1", "--guide=1"], &env);
    let start: Vec<Line> = plain
        .stdout
        .lines()
        .filter_map(|l| l.strip_prefix("c roles hold at=start "))
        .map(pairs)
        .filter(|l| l.get("role").map(String::as_str) == Some("orig"))
        .collect();
    assert_eq!(start.len(), 1);
    assert_eq!(num(&start[0], "held"), 0, "an original is held under its own number: {:?}", start[0]);
    assert!(num(&start[0], "merged_held") >= 30, "{:?}", start[0]);
    let line = guide_line(&plain.stdout);
    assert_eq!(line["first"], "other", "the toy tests nothing: the first decision is a preferred variable anyway");
    // the first order is given to the copies that stand for them
    let got = run(&cnf, &["--guide=1", "--guideinit=1"], &env);
    let line = guide_line(&got.stdout);
    assert_eq!(line["first"], "preferred", "{:?}", line);
    assert!(num(&line, "started") >= 30 && num(&line, "started") <= 40, "{:?}", line);
    assert_eq!(got.status, plain.status);
}

#[test]
fn the_solvers_own_variables_can_be_preferred() {
    // on pigeons the solver's `factor` step makes variables of its own and
    // decides on them; the role `new` names them, though no list can
    let fx = Fixture::new("guide_new");
    let cnf = fx.path("php.cnf");
    let roles = fx.path("php.roles");
    php(8, &cnf);
    std::fs::write(&roles, "1-72 all\n").unwrap();
    let plain = run(&cnf, &["--rolecount=1", "--guide=1"], &env_of(&roles, "none", "none"));
    assert!(plain.stats.get("variables_extension").copied().unwrap_or(0) > 0, "the formula makes the solver add variables");
    assert!(counted(&plain.stdout, "new", "decide_own") > 0, "decisions on the solver's own variables");
    let got = run(&cnf, &["--guide=1", "--guidebump=800", "--guidefront=1"], &env_of(&roles, "new", "none"));
    let line = guide_line(&got.stdout);
    assert_eq!(num(&line, "list"), 0, "none of them is there at the start");
    assert!(num(&line, "bumped") + num(&line, "fronted") > 0, "{:?}", line);
    assert_eq!(got.status, plain.status);
    // with the formula's own variables preferred, none of the solver's is
    let got = run(&cnf, &["--rolecount=1", "--guide=1", "--guidephase=1"], &env_of(&roles, "all", "new"));
    let (decided, tried_true) = (counted(&got.stdout, "new", "decide_own"), counted(&got.stdout, "new", "true_own"));
    assert!(decided > 0);
    assert_eq!(tried_true, decided, "a variable the solver made takes the value preferred for the role new");
}

#[test]
fn a_kept_variable_is_not_eliminated() {
    let fx = Fixture::new("guide_protect");
    let cnf = fx.path("r.cnf");
    let roles = fx.path("r.roles");
    random_3sat(250, 4.15, 4, &cnf);
    two_roles(250, 125, &roles);
    let env = env_of(&roles, "pick", "pick");
    let gone = |r: &Run, role: &str| -> u64 {
        role_lines(&r.stdout, "hold")
            .iter()
            .filter(|l| l.get("role").map(String::as_str) == Some(role))
            .map(|l| num(l, "eliminated") + num(l, "merged_eliminated"))
            .sum()
    };
    let plain = run(&cnf, &["--rolecount=1", "--guide=1"], &env);
    assert!(gone(&plain, "pick") > 0, "the toy eliminates none of the variables: it tests nothing");
    let kept = run(&cnf, &["--rolecount=1", "--guide=1", "--guideprotect=1"], &env);
    assert_eq!(gone(&kept, "pick"), 0, "a kept variable was eliminated");
    assert!(gone(&kept, "rest") > 0, "the others are eliminated as usual");
    assert_eq!(kept.status, plain.status);
}

#[test]
fn a_setting_that_cannot_be_used_stops_the_run() {
    let fx = Fixture::new("guide_bad");
    let cnf = fx.path("r.cnf");
    random_3sat(60, 4.0, 1, &cnf);
    let none = run(&cnf, &["--guide=1"], &[]);
    assert_eq!(none.exit_code, 1, "{}", none.stdout);
    assert!(none.stderr.contains("guide is on but SAT_ROLE_FILE names no role file"), "{}", none.stderr);
    assert!(none.status.is_empty(), "no answer without the roles");
    let missing = run(&cnf, &["--guide=1"], &[("SAT_ROLE_FILE", "/nonexistent/roles")]);
    assert_eq!(missing.exit_code, 1);
    // a role named for the nudges that the file does not have is said, and is no error
    let roles = fx.path("r.roles");
    two_roles(60, 10, &roles);
    let other = run(&cnf, &["--guide=1", "--guiderestart=1"], &env_of(&roles, "pick,nosuch", "pick"));
    assert!(other.exit_code == 10 || other.exit_code == 20, "{}", other.exit_code);
    assert!(other.stdout.contains("not in the role file: nosuch"), "{}", other.stdout);
}

// ---------------------------------------------------------------------------
// the roles from the solver's own reading (`roleread`, bead d96.14.6)
// ---------------------------------------------------------------------------

/// A formula of the shape the free-choice reading reads: `groups` pick-one
/// groups of `size` variables (a clause and a binary clause for each pair);
/// `defs` variables, each the AND of two members of two different groups;
/// `ccons` random clauses over members of three different groups; and
/// `dcons` clauses over three of the AND variables with at least two
/// negative literals.  No member implies a member of another group and no
/// AND variable is ever implied true by binary clauses, so no constraint
/// clause can be read as a group: the reading names the members `choice`
/// and the AND variables `def`, and nothing else.  The clauses are returned.
fn groups_and_definitions(groups: usize, size: usize, defs: usize, ccons: usize, dcons: usize, seed: u64, path: &Path) -> Vec<Vec<i64>> {
    let mut rng = seed.wrapping_mul(2862933555777941757).wrapping_add(3037000493);
    let mut next = || {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (rng >> 33) as usize
    };
    let member = |g: usize, j: usize| (g * size + j + 1) as i64;
    let mut cl: Vec<Vec<i64>> = Vec::new();
    for g in 0..groups {
        cl.push((0..size).map(|j| member(g, j)).collect());
        for a in 0..size {
            for b in a + 1..size {
                cl.push(vec![-member(g, a), -member(g, b)]);
            }
        }
    }
    let base = (groups * size) as i64;
    for i in 0..defs {
        let d = base + i as i64 + 1;
        let ga = next() % groups;
        let mut gb = next() % (groups - 1);
        if gb >= ga {
            gb += 1;
        }
        let a = member(ga, next() % size);
        let b = member(gb, next() % size);
        cl.extend([vec![-d, a], vec![-d, b], vec![d, -a, -b]]);
    }
    for _ in 0..ccons {
        let mut gs: Vec<usize> = Vec::new();
        while gs.len() < 3 {
            let g = next() % groups;
            if !gs.contains(&g) {
                gs.push(g);
            }
        }
        let mut c = Vec::new();
        for g in gs {
            let v = member(g, next() % size);
            c.push(if next() % 2 == 0 { v } else { -v });
        }
        cl.push(c);
    }
    for _ in 0..dcons {
        let mut ds: Vec<i64> = Vec::new();
        while ds.len() < 3 {
            let v = base + (next() % defs) as i64 + 1;
            if !ds.contains(&v) {
                ds.push(v);
            }
        }
        let third = if next() % 2 == 0 { 1 } else { -1 };
        cl.push(vec![-ds[0], -ds[1], third * ds[2]]);
    }
    let mut text = format!("p cnf {} {}\n", groups * size + defs, cl.len());
    for c in &cl {
        for l in c {
            text.push_str(&format!("{} ", l));
        }
        text.push_str("0\n");
    }
    std::fs::write(path, text).unwrap();
    cl
}

/// The same search with the reading's work added: the status and every
/// `-s` counter, and the work clock line apart from the total ticks and W.
/// For the stock path, where the reading runs only for the roles.
fn assert_same_search(a: &Run, b: &Run, what: &str) {
    assert_eq!(a.status, b.status, "{}: status differs", what);
    assert_eq!(a.stats, b.stats, "{}: -s counters differ", what);
    let strip = |w: &BTreeMap<String, u64>| -> BTreeMap<String, u64> { w.iter().filter(|(k, _)| k.as_str() != "ticks" && k.as_str() != "work").map(|(k, v)| (k.clone(), *v)).collect() };
    assert_eq!(strip(&a.workclock), strip(&b.workclock), "{}: work clock line differs", what);
    assert!(b.workclock["ticks"] >= a.workclock["ticks"], "{}: the reading's work is added", what);
}

/// The `c freeread status=read ...` line of a run.
fn reading_line(stdout: &str) -> Line {
    let rest = stdout.lines().find_map(|l| l.strip_prefix("c freeread status=read ")).unwrap_or_else(|| panic!("no reading line in:\n{}", stdout));
    pairs(rest)
}

/// The `c guide off reason=...` line of a run.
fn off_line(stdout: &str) -> Line {
    let rest = stdout.lines().find_map(|l| l.strip_prefix("c guide off ")).unwrap_or_else(|| panic!("no guide off line in:\n{}", stdout));
    pairs(rest)
}

/// The two toys of shape `groups_and_definitions` that need a search: one
/// with a model (about 8,600 conflicts on the stock path) and one without
/// (about 6,100).
const SHAPED: &[(&str, [usize; 5], u64)] = &[("sat", [150, 4, 500, 1050, 300], 7), ("unsat", [120, 5, 500, 1100, 300], 5)];

#[test]
fn the_read_roles_change_nothing_without_the_count_or_the_nudges() {
    let fx = Fixture::new("roleread_alone");
    let cnf = fx.path("r.cnf");
    random_3sat(120, 4.6, 3, &cnf);
    // no role file is read: the one named does not exist
    let env = [("SAT_ROLE_FILE", "/nonexistent/roles")];
    let plain = run(&cnf, &[], &[]);
    let read = run(&cnf, &["--roleread=1"], &env);
    common::assert_same_trajectory(&plain, &read, "roleread alone, stock path");
    assert!(!read.stdout.contains("c guide") && !read.stdout.contains("c roles"), "{}", read.stdout);
    let plain = run_shipped(&cnf, &[], &[]);
    let read = run_shipped(&cnf, &["--roleread=1"], &env);
    common::assert_same_trajectory(&plain, &read, "roleread alone, shipped defaults");
    assert!(!read.stdout.contains("c guide") && !read.stdout.contains("c roles"), "{}", read.stdout);
}

#[test]
fn the_read_lean_with_no_dial_is_the_old_search() {
    let fx = Fixture::new("roleread_nodial");
    let env = [("SAT_ROLE_FILE", "/nonexistent/roles")];
    // on the shipped path the reading runs anyway (for the walk): the search
    // is the old one to the tick; random 3-SAT is turned down by the walk's
    // test, the shaped toys are walked, briefly
    for (seed, ratio) in [(3u64, 4.6), (5, 4.0)] {
        let cnf = fx.path(&format!("r{}.cnf", seed));
        random_3sat(140, ratio, seed, &cnf);
        let plain = run_shipped(&cnf, &[], &[]);
        let on = run_shipped(&cnf, &["--guide=1", "--roleread=1"], &env);
        common::assert_same_trajectory(&plain, &on, "the read lean with every dial off, shipped defaults");
        assert_eq!(guide_line(&on.stdout)["read"], "1");
        // on the stock path the reading runs for the roles alone: its work
        // is added, and the search is the same
        let plain = run(&cnf, &[], &[]);
        let on = run(&cnf, &["--guide=1", "--roleread=1"], &env);
        assert_same_search(&plain, &on, "the read lean with every dial off, stock path");
    }
    for (name, [g, s, d, cc, dc], seed) in SHAPED {
        let cnf = fx.path(&format!("{}.cnf", name));
        groups_and_definitions(*g, *s, *d, *cc, *dc, *seed, &cnf);
        let short = "--freewalkticks=100";
        let plain = run_shipped(&cnf, &[short], &[]);
        let on = run_shipped(&cnf, &[short, "--guide=1", "--roleread=1"], &env);
        common::assert_same_trajectory(&plain, &on, &format!("{}: the read lean with every dial off, shipped defaults", name));
        let plain = run(&cnf, &[], &[]);
        let on = run(&cnf, &["--guide=1", "--roleread=1"], &env);
        assert_same_search(&plain, &on, &format!("{}: the read lean with every dial off, stock path", name));
    }
}

#[test]
fn the_read_lean_prefers_the_readings_choices_and_free_variables() {
    let fx = Fixture::new("roleread_lean");
    let lean = ["--guide=1", "--roleread=1", "--freeread=1", "--guidebump=200", "--guidefront=1"];
    // the shaped toys: choices and definitions, nothing free
    for (name, [g, s, d, cc, dc], seed) in SHAPED {
        let cnf = fx.path(&format!("{}.cnf", name));
        groups_and_definitions(*g, *s, *d, *cc, *dc, *seed, &cnf);
        let plain = run(&cnf, &[], &[]);
        assert!(plain.stats["conflicts"] > 3000, "{}: the toy needs a search ({} conflicts)", name, plain.stats["conflicts"]);
        assert_eq!(plain.status, if *name == "sat" { "SATISFIABLE" } else { "UNSATISFIABLE" }, "{}", name);
        let got = run(&cnf, &lean, &[]);
        let read = reading_line(&got.stdout);
        let line = guide_line(&got.stdout);
        // the reading as built: every member a choice, every AND a definition
        assert_eq!(num(&read, "group_vars"), (g * s) as u64, "{}: {:?}", name, read);
        assert_eq!(num(&read, "defs"), *d as u64, "{}: {:?}", name, read);
        for (role, key) in [("choice", "group_vars"), ("free", "free"), ("def", "defs"), ("gate", "gates"), ("helper", "helpers")] {
            assert_eq!(num(&line, role), num(&read, key), "{}: {} against the reading's {}", name, role, key);
        }
        // preferred: the choices and the free variables (the solver has made
        // none of its own at the start)
        assert_eq!(num(&line, "list"), num(&line, "choice") + num(&line, "free"), "{}: {:?}", name, line);
        let head = format!("c guide: {} preferred variables (roles choice,free,new), {} with a preferred value (roles choice)", num(&line, "list"), num(&line, "choice"));
        assert!(got.stdout.lines().any(|l| l == head), "{}: no line '{}'", name, head);
        assert!(num(&line, "bumped") > 0 && num(&line, "fronted") > 0, "{}: the lean did not act: {:?}", name, line);
        assert_eq!(got.status, plain.status, "{}", name);
        assert_eq!(got.exit_code, plain.exit_code, "{}", name);
        if *name == "sat" {
            assert!(model_holds(&cnf, &lean, &[]), "{}: the model does not satisfy the formula", name);
        }
    }
    // random 3-SAT: the reading names free variables and helpers
    let cnf = fx.path("r.cnf");
    random_3sat(200, 4.0, 1, &cnf);
    let got = run(&cnf, &lean, &[]);
    let (read, line) = (reading_line(&got.stdout), guide_line(&got.stdout));
    assert!(num(&read, "free") > 0 && num(&read, "helpers") > 0, "{:?}", read);
    assert_eq!(num(&line, "free"), num(&read, "free"));
    assert_eq!(num(&line, "helper"), num(&read, "helpers"));
    assert_eq!(num(&line, "list"), num(&line, "free"), "{:?}", line);
    assert_eq!(got.status, run(&cnf, &[], &[]).status);
}

#[test]
fn the_read_lean_stays_off_when_there_is_nothing_to_tell_apart() {
    let fx = Fixture::new("roleread_off");
    let lean = ["--guide=1", "--roleread=1", "--guidebump=200", "--guidefront=1"];
    // pick-one groups and clauses over them: choices only, no bookkeeping
    let cnf = fx.path("groups.cnf");
    groups_and_definitions(40, 4, 0, 300, 0, 1, &cnf);
    let plain = run(&cnf, &[], &[]);
    let got = run(&cnf, &lean, &[]);
    let line = off_line(&got.stdout);
    assert_eq!(line["reason"], "no-bookkeeping", "{:?}", line);
    assert_eq!((num(&line, "choice"), num(&line, "def") + num(&line, "gate") + num(&line, "helper")), (160, 0), "{:?}", line);
    assert!(!got.stdout.contains("c guide list="), "the nudges are off");
    assert_same_search(&plain, &got, "no bookkeeping");
    // every variable fixed by a unit: no real unknown
    let cnf = fx.path("units.cnf");
    std::fs::write(&cnf, "p cnf 3 3\n1 0\n-2 0\n3 0\n").unwrap();
    let got = run(&cnf, &lean, &[]);
    assert_eq!(off_line(&got.stdout)["reason"], "no-unknowns", "{}", got.stdout);
    assert_eq!(got.status, "SATISFIABLE");
}

#[test]
fn the_read_lean_without_a_reading_is_the_plain_search() {
    let fx = Fixture::new("roleread_unread");
    let lean = ["--guide=1", "--roleread=1", "--guidebump=200", "--guidefront=1"];
    let cnf = fx.path("sat.cnf");
    let [g, s, d, cc, dc] = SHAPED[0].1;
    groups_and_definitions(g, s, d, cc, dc, SHAPED[0].2, &cnf);
    // the reading out of its work limit, and the reading turned down for a
    // formula with more clauses than it takes
    let limits: [&[&str]; 2] = [&["--freereadticks=0"], &["--structclauses=100", "--freereadclauses=100"]];
    for limit in limits {
        let plain = run(&cnf, limit, &[]);
        let mut all = limit.to_vec();
        all.extend_from_slice(&lean);
        let got = run(&cnf, &all, &[]);
        assert_eq!(off_line(&got.stdout)["reason"], "unread", "{:?}: {}", limit, got.stdout);
        assert!(got.stdout.contains("c roles: the formula has no free-choice reading, so no roles"), "{:?}: {}", limit, got.stdout);
        assert_same_search(&plain, &got, &format!("{:?}", limit));
    }
}

#[test]
fn the_count_by_role_takes_the_readings_roles() {
    let fx = Fixture::new("roleread_count");
    let cnf = fx.path("sat.cnf");
    let [g, s, d, cc, dc] = SHAPED[0].1;
    groups_and_definitions(g, s, d, cc, dc, SHAPED[0].2, &cnf);
    // no role file is read, even one that is named and does not exist
    let env = [("SAT_ROLE_FILE", "/nonexistent/roles")];
    let plain = run(&cnf, &[], &[]);
    let got = run(&cnf, &["--rolecount=1", "--roleread=1"], &env);
    assert_eq!(got.exit_code, plain.exit_code, "{}", got.stderr);
    let file = got.stdout.lines().find_map(|l| l.strip_prefix("c roles file ")).map(pairs).unwrap_or_else(|| panic!("no roles file line in:\n{}", got.stdout));
    assert_eq!(file["path"], "reading", "{:?}", file);
    assert_eq!(num(&file, "listed"), (g * s + d) as u64, "{:?}", file);
    let held = |role: &str| -> u64 {
        role_lines(&got.stdout, "hold").iter().filter(|l| l.get("role").map(String::as_str) == Some(role)).map(|l| num(l, "vars")).next().unwrap_or(0)
    };
    assert_eq!((held("choice"), held("def")), ((g * s) as u64, d as u64));
    assert!(counted(&got.stdout, "choice", "decide_own") > 0, "decisions on the choices are counted");
    assert_same_search(&plain, &got, "the count with the reading's roles");
}
