//! Decisions counted by role (bead SAT-playground-d96.14.1, src/roles.rs).
//! With `rolecount` on the solver reads a role for each variable from the
//! file named by SAT_ROLE_FILE and counts what the search decides on.
//! Checked here: off means off (the environment variable is not read), the
//! search is the same with the count on (counter for counter and on the
//! work clock), the counts add up to the solver's own, a variable replaced
//! by an equal one is followed, a variable the solver makes itself is
//! `new`, a role file that cannot be used stops the run, and a run ended by
//! a signal still prints what it had.

mod common;

use common::{php, random_3sat, run, run_shipped, solver_bin, Fixture, Run};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

type Line = BTreeMap<String, String>;

/// The `c roles <kind> key=value ...` lines of a run.
fn lines(stdout: &str, kind: &str) -> Vec<Line> {
    let head = format!("c roles {} ", kind);
    stdout
        .lines()
        .filter_map(|l| l.strip_prefix(&head))
        .map(|rest| rest.split_whitespace().filter_map(|t| t.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect())
        .collect()
}

fn num(line: &Line, key: &str) -> u64 {
    line.get(key).unwrap_or_else(|| panic!("no {} in {:?}", key, line)).parse().unwrap_or_else(|_| panic!("{} in {:?}", key, line))
}

/// The lines of a kind at one checkpoint.
fn at<'a>(all: &'a [Line], label: &str) -> Vec<&'a Line> {
    all.iter().filter(|l| l.get("at").map(String::as_str) == Some(label)).collect()
}

fn counted(cnf: &Path, roles: &Path, options: &[&str], shipped: bool) -> Run {
    let mut all: Vec<&str> = vec!["--rolecount=1"];
    all.extend_from_slice(options);
    let env = [("SAT_ROLE_FILE", roles.to_str().unwrap())];
    if shipped {
        run_shipped(cnf, &all, &env)
    } else {
        run(cnf, &all, &env)
    }
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Random 3-SAT over `n` variables in which each of the first `k` variables
/// v has a copy n+v, equal to it by two clauses (to its opposite with
/// `flip`), that takes about half of its places.  Roles: the originals
/// `orig`, the copies `copy`, the rest `rest`.
fn planted_equal(n: u64, k: u64, ratio: f64, seed: u64, cnf: &Path, roles: &Path) {
    planted(n, k, ratio, seed, false, cnf, roles)
}

fn planted(n: u64, k: u64, ratio: f64, seed: u64, flip: bool, cnf: &Path, roles: &Path) {
    let mut rng = Lcg(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut text = String::new();
    let m = (n as f64 * ratio) as u64;
    let mut count = 0u64;
    for _ in 0..m {
        let mut vs: Vec<u64> = Vec::new();
        while vs.len() < 3 {
            let v = 1 + rng.below(n);
            if !vs.contains(&v) {
                vs.push(v);
            }
        }
        for v in vs {
            let copied = v <= k && rng.below(2) == 0;
            // the literal wanted of v; its copy stands in with the same sign, or the other with `flip`
            let negative = rng.below(2) == 0;
            let (v, negative) = if copied { (n + v, negative != flip) } else { (v, negative) };
            text.push_str(&format!("{}{} ", if negative { "-" } else { "" }, v));
        }
        text.push_str("0\n");
        count += 1;
    }
    for v in 1..=k {
        if flip {
            text.push_str(&format!("{} {} 0\n-{} -{} 0\n", v, n + v, v, n + v));
        } else {
            text.push_str(&format!("-{} {} 0\n{} -{} 0\n", v, n + v, v, n + v));
        }
        count += 2;
    }
    std::fs::write(cnf, format!("p cnf {} {}\n{}", n + k, count, text)).unwrap();
    std::fs::write(roles, format!("c the originals, the rest, the copies\n1-{} orig\n{}-{} rest\n{}-{} copy\n", k, k + 1, n, n + 1, n + k)).unwrap();
}

fn one_role(vars: usize, path: &Path) {
    std::fs::write(path, format!("1-{} all\n", vars)).unwrap();
}

#[test]
fn off_means_off() {
    let fx = Fixture::new("roles_off");
    let cnf = fx.path("r.cnf");
    random_3sat(120, 4.6, 3, &cnf);
    let plain = run(&cnf, &[], &[]);
    // the variable set, the option off: never read (the file does not exist)
    let with_env = run(&cnf, &[], &[("SAT_ROLE_FILE", "/nonexistent/roles")]);
    common::assert_same_trajectory(&plain, &with_env, "SAT_ROLE_FILE with the option off");
    assert_eq!(with_env.exit_code, plain.exit_code);
    assert!(!plain.stdout.contains("c roles"), "no exit line with the option off");
    assert!(!with_env.stdout.contains("c roles"), "no exit line with the option off");
}

#[test]
fn the_search_is_the_same_with_the_count_on() {
    let fx = Fixture::new("roles_same");
    // random formulas with and without a model, and one whose equal variables the solver merges
    for seed in 1..=4u64 {
        let cnf = fx.path(&format!("r{}.cnf", seed));
        let roles = fx.path(&format!("r{}.roles", seed));
        random_3sat(150, 4.3, seed, &cnf);
        std::fs::write(&roles, "1-50 a\n51-100 b\n").unwrap();
        for shipped in [false, true] {
            let plain = if shipped { run_shipped(&cnf, &[], &[]) } else { run(&cnf, &[], &[]) };
            let with = counted(&cnf, &roles, &[], shipped);
            common::assert_same_trajectory(&plain, &with, &format!("random seed {} shipped {}", seed, shipped));
            assert!(with.stdout.contains("c roles file "), "seed {}: the count ran", seed);
        }
    }
    for seed in 1..=3u64 {
        let cnf = fx.path(&format!("e{}.cnf", seed));
        let roles = fx.path(&format!("e{}.roles", seed));
        planted_equal(200, 40, 4.25, seed, &cnf, &roles);
        for shipped in [false, true] {
            let plain = if shipped { run_shipped(&cnf, &[], &[]) } else { run(&cnf, &[], &[]) };
            let with = counted(&cnf, &roles, &[], shipped);
            common::assert_same_trajectory(&plain, &with, &format!("planted seed {} shipped {}", seed, shipped));
        }
    }
    // pigeons: the stock search works long on it, so every checkpoint up to 10,000 conflicts is taken
    let cnf = fx.path("php.cnf");
    let roles = fx.path("php.roles");
    php(8, &cnf);
    one_role(8 * 9, &roles);
    let plain = run(&cnf, &[], &[]);
    let with = counted(&cnf, &roles, &[], false);
    common::assert_same_trajectory(&plain, &with, "php 8");
    let hold = lines(&with.stdout, "hold");
    for label in ["start", "1000", "end"] {
        assert!(!at(&hold, label).is_empty(), "php 8: the checkpoint {}", label);
    }
}

#[test]
fn the_counts_add_up() {
    let fx = Fixture::new("roles_sums");
    let cnf = fx.path("e.cnf");
    let roles = fx.path("e.roles");
    planted_equal(220, 40, 4.25, 7, &cnf, &roles);
    let r = counted(&cnf, &roles, &[], false);
    assert!(!r.status.is_empty());
    let search = lines(&r.stdout, "search");
    let count = lines(&r.stdout, "count");
    let end_search = at(&search, "end");
    let end_count = at(&count, "end");
    assert_eq!(end_search.len(), 2, "one line a mode");
    // every conflict goes to one mode, every decision to one role
    let conflicts: u64 = end_search.iter().map(|l| num(l, "mode_conflicts")).sum();
    assert_eq!(Some(&conflicts), r.stats.get("conflicts"));
    let decisions: u64 = end_count.iter().map(|l| num(l, "decide_own")).sum();
    assert_eq!(Some(&decisions), r.stats.get("decisions"));
    assert!(decisions > 1_000, "the formula takes a search: {}", decisions);
    let mut samples = 0u64;
    let mut restarts = 0u64;
    for s in &end_search {
        let mode = &s["mode"];
        let of_mode: Vec<&&Line> = end_count.iter().filter(|l| &l["mode"] == mode).collect();
        // the solver's own count of the mode's decisions
        assert_eq!(of_mode.iter().map(|l| num(l, "decide_own")).sum::<u64>(), num(s, "decisions"), "mode {}", mode);
        // a sample holds a decision for each of its levels, and at most ten of them are its top
        let top: u64 = of_mode.iter().map(|l| num(l, "top_own")).sum();
        let dec: u64 = of_mode.iter().map(|l| num(l, "dec_own")).sum();
        assert_eq!(dec, num(s, "levels"), "mode {}: a decision a level", mode);
        assert!(top <= 10 * num(s, "samples") && top <= dec, "mode {}", mode);
        for l in &of_mode {
            assert!(num(l, "set_own") >= num(l, "dec_own"), "{:?}", l);
            assert!(num(l, "set_stands") >= num(l, "dec_stands"), "{:?}", l);
            assert!(num(l, "decide_stands") >= num(l, "decide_own"), "{:?}", l);
            assert!(num(l, "true_own") <= num(l, "decide_own"), "{:?}", l);
            assert!(num(l, "true_stands") <= num(l, "decide_stands"), "{:?}", l);
            assert!(num(l, "true_stands") >= num(l, "true_own"), "{:?}", l);
        }
        samples += num(s, "samples");
        restarts += num(s, "restarts");
    }
    // one conflict in 128 is a sample
    assert_eq!(samples, (conflicts + 127) / 128, "samples of {} conflicts", conflicts);
    let first: u64 = end_count.iter().map(|l| num(l, "first_own")).sum();
    assert!(first <= restarts && first > 0, "first decisions {} of {} restarts", first, restarts);
    // the checkpoints are cumulative
    for label in ["start", "1000", "10000"] {
        let before: u64 = at(&count, label).iter().map(|l| num(l, "decide_own")).sum();
        assert!(before <= decisions, "{}", label);
    }
    assert_eq!(at(&search, "1000").iter().map(|l| num(l, "mode_conflicts")).sum::<u64>(), num(at(&search, "1000")[0], "conflicts"));
    assert_eq!(num(at(&search, "1000")[0], "conflicts"), 1_000, "the checkpoint is taken at its conflict");
}

#[test]
fn a_variable_replaced_by_an_equal_one_is_followed() {
    let fx = Fixture::new("roles_merge");
    let cnf = fx.path("e.cnf");
    let roles = fx.path("e.roles");
    planted_equal(220, 40, 4.25, 7, &cnf, &roles);
    let r = counted(&cnf, &roles, &[], false);
    let file = &lines(&r.stdout, "file")[0];
    // the solver finds every planted pair before its first decision
    assert_eq!(num(file, "merges"), 40, "{:?}", file);
    assert_eq!(file["ended"], "exit");
    let hold = lines(&r.stdout, "hold");
    let start = at(&hold, "start");
    let of = |role: &str| -> &Line { start.iter().find(|l| l["role"] == role).unwrap_or_else(|| panic!("no {}", role)) };
    let (orig, copy, rest) = (of("orig"), of("copy"), of("rest"));
    // of each pair one is held and the other follows it, under the other's role
    assert_eq!(num(orig, "held") + num(copy, "held"), 40);
    assert_eq!(num(orig, "merged_held") + num(copy, "merged_held"), 40);
    assert_eq!(num(orig, "held"), num(copy, "merged_held"));
    assert_eq!(num(copy, "held"), num(orig, "merged_held"));
    for (line, other, n) in [(orig, "copy", num(orig, "merged_held")), (copy, "orig", num(copy, "merged_held"))] {
        let want = if n == 0 { "-".to_string() } else { format!("{}:{}", other, n) };
        assert_eq!(line["into"], want, "{:?}", line);
    }
    // every held variable of a pair carries both roles
    assert_eq!(num(orig, "carriers"), 40);
    assert_eq!(num(copy, "carriers"), 40);
    assert_eq!(num(rest, "held"), 180);
    assert_eq!(num(rest, "carriers"), 180);
    assert_eq!(rest["into"], "-");
    // a decision on a pair's variable stands for both roles, whichever name was kept
    let count = lines(&r.stdout, "count");
    for mode in ["focused", "stable"] {
        let get = |role: &str, key: &str| -> u64 {
            at(&count, "end").iter().find(|l| l["mode"] == mode && l["role"] == role).map(|l| num(l, key)).unwrap_or(0)
        };
        let both = get("orig", "decide_own") + get("copy", "decide_own");
        assert!(both > 0, "mode {}", mode);
        assert_eq!(get("orig", "decide_stands"), both, "mode {}", mode);
        assert_eq!(get("copy", "decide_stands"), both, "mode {}", mode);
        assert_eq!(get("rest", "decide_stands"), get("rest", "decide_own"), "mode {}", mode);
        // equal variables get the same value: true for one role is true for the other
        let trues = get("orig", "true_own") + get("copy", "true_own");
        assert!(trues > 0 && trues < both, "mode {}: both values are tried", mode);
        assert_eq!(get("orig", "true_stands"), trues, "mode {}", mode);
        assert_eq!(get("copy", "true_stands"), trues, "mode {}", mode);
        assert_eq!(get("rest", "true_stands"), get("rest", "true_own"), "mode {}", mode);
    }
}

#[test]
fn a_variable_replaced_by_its_opposite_counts_the_other_value() {
    let fx = Fixture::new("roles_flip");
    let cnf = fx.path("f.cnf");
    let roles = fx.path("f.roles");
    planted(220, 40, 4.25, 7, true, &cnf, &roles);
    let plain = run(&cnf, &[], &[]);
    let r = counted(&cnf, &roles, &[], false);
    common::assert_same_trajectory(&plain, &r, "planted opposites");
    let file = &lines(&r.stdout, "file")[0];
    assert_eq!(num(file, "merges"), 40, "{:?}", file);
    let count = lines(&r.stdout, "count");
    for mode in ["focused", "stable"] {
        let get = |role: &str, key: &str| -> u64 {
            at(&count, "end").iter().find(|l| l["mode"] == mode && l["role"] == role).map(|l| num(l, key)).unwrap_or(0)
        };
        let both = get("orig", "decide_own") + get("copy", "decide_own");
        assert!(both > 0, "mode {}", mode);
        assert_eq!(get("orig", "decide_stands"), both, "mode {}", mode);
        assert_eq!(get("copy", "decide_stands"), both, "mode {}", mode);
        // a decision on a pair makes exactly one of the two true
        assert_eq!(get("orig", "true_stands") + get("copy", "true_stands"), both, "mode {}", mode);
        // an original is true when it is decided true, or when its copy is decided false
        let want = get("orig", "true_own") + (get("copy", "decide_own") - get("copy", "true_own"));
        assert_eq!(get("orig", "true_stands"), want, "mode {}", mode);
    }
}

#[test]
fn a_role_file_that_cannot_be_used_stops_the_run() {
    let fx = Fixture::new("roles_bad");
    let cnf = fx.path("r.cnf");
    random_3sat(60, 4.0, 1, &cnf);
    // the option on and no file named
    let none = run(&cnf, &["--rolecount=1"], &[]);
    assert_eq!(none.exit_code, 1, "{}", none.stderr);
    assert!(none.status.is_empty(), "no answer: {}", none.status);
    assert!(none.stderr.contains("SAT_ROLE_FILE"), "{}", none.stderr);
    let empty = run(&cnf, &["--rolecount=1"], &[("SAT_ROLE_FILE", "")]);
    assert_eq!(empty.exit_code, 1, "{}", empty.stderr);
    // a file that is not there, and files with a fault
    let missing = run(&cnf, &["--rolecount=1"], &[("SAT_ROLE_FILE", "/nonexistent/roles")]);
    assert_eq!(missing.exit_code, 1, "{}", missing.stderr);
    assert!(missing.status.is_empty());
    for (text, what) in [("61 a\n", "not in the formula"), ("1 a\n1 b\n", "line 2"), ("1 a b\n", "line 1")] {
        let roles = fx.path("bad.roles");
        std::fs::write(&roles, text).unwrap();
        let bad = counted(&cnf, &roles, &[], false);
        assert_eq!(bad.exit_code, 1, "{:?}: {}", text, bad.stderr);
        assert!(bad.status.is_empty(), "{:?}: no answer", text);
        assert!(bad.stderr.contains(what), "{:?}: {}", text, bad.stderr);
    }
    // an empty file is a reading that names nothing: every variable is unlisted
    let roles = fx.path("empty.roles");
    std::fs::write(&roles, "c nothing\n").unwrap();
    let r = counted(&cnf, &roles, &[], false);
    assert!(!r.status.is_empty());
    let hold = lines(&r.stdout, "hold");
    let end = at(&hold, "end");
    assert_eq!(end.len(), 1, "{:?}", end);
    assert_eq!(end[0]["role"], "unlisted");
    assert_eq!(num(end[0], "vars"), 60);
}

#[test]
fn a_variable_the_solver_makes_is_new() {
    // on pigeons the solver's `factor` step rewrites the pairwise clauses of a
    // hole with variables of its own, and then decides on them
    let fx = Fixture::new("roles_new");
    let cnf = fx.path("php.cnf");
    let roles = fx.path("php.roles");
    php(8, &cnf);
    one_role(8 * 9, &roles);
    let plain = run(&cnf, &[], &[]);
    let r = counted(&cnf, &roles, &[], false);
    common::assert_same_trajectory(&plain, &r, "php 8");
    let made = r.stats.get("variables_extension").copied().unwrap_or(0);
    assert!(made > 0, "the formula makes the solver add variables");
    let hold = lines(&r.stdout, "hold");
    let end = at(&hold, "end");
    let new = end.iter().find(|l| l["role"] == "new").expect("a line for the solver's own variables");
    assert_eq!(num(new, "vars"), made);
    let all = end.iter().find(|l| l["role"] == "all").unwrap();
    assert_eq!(num(all, "vars"), 72);
    assert!(end.iter().all(|l| l["role"] != "unlisted"), "every variable of the formula is listed");
    let count = lines(&r.stdout, "count");
    let on_new: u64 = at(&count, "end").iter().filter(|l| l["role"] == "new").map(|l| num(l, "decide_own")).sum();
    let on_all: u64 = at(&count, "end").iter().filter(|l| l["role"] == "all").map(|l| num(l, "decide_own")).sum();
    assert!(on_new > 0, "decisions on the solver's own variables");
    assert_eq!(Some(&(on_new + on_all)), r.stats.get("decisions"));
}

#[test]
fn a_search_that_ends_in_stable_mode_keeps_its_conflicts_there() {
    // the search clears its mode when it stops; the conflicts since the last
    // switch must go to the mode they were made in
    let fx = Fixture::new("roles_stable_end");
    let cnf = fx.path("php.cnf");
    let roles = fx.path("php.roles");
    php(8, &cnf);
    one_role(8 * 9, &roles);
    let plain = run(&cnf, &["--stable=2", "--conflicts=500"], &[]);
    let r = counted(&cnf, &roles, &["--stable=2", "--conflicts=500"], false);
    common::assert_same_trajectory(&plain, &r, "php 8, stable only");
    assert_eq!(r.status, "UNKNOWN");
    let search = lines(&r.stdout, "search");
    let end = at(&search, "end");
    let of = |mode: &str| -> &Line { end.iter().find(|l| l["mode"] == mode).unwrap() };
    assert_eq!(num(of("focused"), "mode_conflicts"), 0, "{:?}", of("focused"));
    assert_eq!(num(of("focused"), "decisions"), 0);
    assert_eq!(Some(&num(of("stable"), "mode_conflicts")), r.stats.get("conflicts"));
    assert_eq!(Some(&num(of("stable"), "decisions")), r.stats.get("decisions"));
    assert!(num(of("stable"), "mode_conflicts") >= 500);
    // and a run that switches modes: each mode's conflicts are its own, whichever it ends in
    for limit in ["--conflicts=1500", "--conflicts=4000", "--conflicts=9000"] {
        let r = counted(&cnf, &roles, &[limit], false);
        let search = lines(&r.stdout, "search");
        let end = at(&search, "end");
        let sum: u64 = end.iter().map(|l| num(l, "mode_conflicts")).sum();
        assert_eq!(Some(&sum), r.stats.get("conflicts"), "{}", limit);
        for l in &end {
            // a mode with conflicts has decisions, and the other way round
            assert_eq!(num(l, "mode_conflicts") == 0, num(l, "decisions") == 0, "{}: {:?}", limit, l);
        }
    }
}

#[test]
fn a_variable_eliminated_while_equal_to_another_is_followed() {
    // with probing off the planted pairs are not substituted: variable
    // elimination removes one of each pair, which is a replacement all the same
    let fx = Fixture::new("roles_elim_equal");
    let cnf = fx.path("e.cnf");
    let roles = fx.path("e.roles");
    planted_equal(200, 40, 4.25, 3, &cnf, &roles);
    let options = ["--probe=0", "--eliminateinit=0"];
    let plain = run(&cnf, &options, &[]);
    let r = counted(&cnf, &roles, &options, false);
    common::assert_same_trajectory(&plain, &r, "planted pairs, probing off");
    let file = &lines(&r.stdout, "file")[0];
    assert_eq!(num(file, "merges"), 0, "nothing substitutes with probing off: {:?}", file);
    assert_eq!(num(file, "equal_eliminations"), 40, "{:?}", file);
    let hold = lines(&r.stdout, "hold");
    let end = at(&hold, "end");
    let of = |role: &str| -> &Line { end.iter().find(|l| l["role"] == role).unwrap() };
    let followed = |l: &Line| num(l, "merged_held") + num(l, "merged_fixed") + num(l, "merged_eliminated");
    assert_eq!(followed(of("orig")) + followed(of("copy")), 40);
    assert_eq!(followed(of("rest")), 0);
    // from the checkpoint at which every pair is one variable, a decision on it counts for both roles
    let labels: Vec<String> = hold.iter().map(|l| l["at"].clone()).collect();
    let first_whole = ["start", "1000", "10000"].iter().copied().find(|label| {
        labels.iter().any(|l| l == label)
            && at(&hold, label).iter().filter(|l| l["role"] == "orig" || l["role"] == "copy").map(|l| followed(l)).sum::<u64>() == 40
    });
    let count = lines(&r.stdout, "count");
    let get = |label: &str, role: &str, key: &str| -> u64 {
        at(&count, label).iter().filter(|l| l["role"] == role).map(|l| num(l, key)).sum()
    };
    let label = first_whole.expect("every pair is one variable at some checkpoint");
    for key in ["decide_stands", "true_stands"] {
        let since = |role: &str| get("end", role, key) - get(label, role, key);
        assert!(since("orig") > 0, "{}: decisions on the pairs after {}", key, label);
        assert_eq!(since("orig"), since("copy"), "{} since {}", key, label);
    }
}

#[test]
fn a_number_the_formula_declares_and_never_uses_can_be_the_solvers() {
    // the solver numbers its own variables after the last one a clause uses:
    // with 1,000 variables declared and 72 used they get numbers the role
    // file lists, and are the solver's all the same
    let fx = Fixture::new("roles_declared");
    let cnf = fx.path("php.cnf");
    let wide = fx.path("wide.cnf");
    let roles = fx.path("wide.roles");
    php(8, &cnf);
    let text = std::fs::read_to_string(&cnf).unwrap();
    let (head, body) = text.split_once('\n').unwrap();
    let clauses = head.split_whitespace().last().unwrap();
    assert!(head.starts_with("p cnf 72 "), "{}", head);
    std::fs::write(&wide, format!("p cnf 1000 {}\n{}", clauses, body)).unwrap();
    one_role(1000, &roles);
    let exact_roles = fx.path("php.roles");
    one_role(72, &exact_roles);
    // with tumble off the solver registers all 1,000 before the formula is read
    for tumble in ["--tumble=1", "--tumble=0"] {
        let plain = run(&wide, &[tumble], &[]);
        let r = counted(&wide, &roles, &[tumble], false);
        common::assert_same_trajectory(&plain, &r, "php 8 declared wide");
        let made = r.stats.get("variables_extension").copied().unwrap_or(0);
        assert!(made > 0, "{}", tumble);
        let hold = lines(&r.stdout, "hold");
        let end = at(&hold, "end");
        let new = end.iter().find(|l| l["role"] == "new").expect("the solver's own variables");
        assert_eq!(num(new, "vars"), made, "{}", tumble);
        let all = end.iter().find(|l| l["role"] == "all").unwrap();
        assert_eq!(num(all, "vars"), 1000, "{}", tumble);
        if tumble == "--tumble=1" {
            assert_eq!(num(all, "absent"), 1000 - 72, "{:?}", all);
        } else {
            // registered up front, the 928 unused ones are variables the solver holds, in no clause
            assert_eq!(num(all, "absent"), 0, "{:?}", all);
            assert!(num(all, "held") >= 1000 - 72, "{:?}", all);
        }
        let count = lines(&r.stdout, "count");
        let on = |role: &str| -> u64 { at(&count, "end").iter().filter(|l| l["role"] == role).map(|l| num(l, "decide_own")).sum() };
        assert!(on("new") > 0, "{}: decisions on the solver's own variables", tumble);
        // the same counts as with the exact header
        let exact = counted(&cnf, &exact_roles, &[tumble], false);
        let exact_count = lines(&exact.stdout, "count");
        let exact_on = |role: &str| -> u64 { at(&exact_count, "end").iter().filter(|l| l["role"] == role).map(|l| num(l, "decide_own")).sum() };
        assert_eq!(Some(&(on("new") + on("all"))), r.stats.get("decisions"), "{}", tumble);
        if tumble == "--tumble=1" {
            // the same search too (with tumble off the solver's inner numbering goes by the
            // header, so the search itself can be another one)
            assert_eq!(on("new"), exact_on("new"));
            assert_eq!(on("all"), exact_on("all"));
        }
    }
}

#[test]
fn a_role_file_that_is_one_of_the_runs_own_files_is_refused_and_kept() {
    // the role file is an input: a run that would write over it stops before it opens any file
    let fx = Fixture::new("roles_alias");
    let cnf = fx.path("r.cnf");
    random_3sat(60, 4.0, 1, &cnf);
    let roles = fx.path("r.roles");
    let text = "1-30 a\n31-60 b\n";
    let link = fx.path("link.drat");
    std::fs::write(&roles, text).unwrap();
    std::os::unix::fs::symlink(&roles, &link).unwrap();
    let r = roles.to_str().unwrap();
    let run_with = |args: &[&str], env: &[(&str, &str)]| -> (i32, String, String) {
        let mut cmd = Command::new(solver_bin());
        for key in common::SOLVER_ENV_VARS {
            cmd.env_remove(key);
        }
        cmd.arg("-n").args(common::STOCK_PATH).arg("--rolecount=1").args(args);
        cmd.env("SAT_ROLE_FILE", r);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run sat-solver");
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let c = cnf.to_str().unwrap();
    let out = fx.path("out.cnf");
    let cases: Vec<(&str, Vec<&str>, Vec<(&str, &str)>, &str)> = vec![
        ("the proof", vec![c, r], vec![], "is the proof file"),
        ("a link to it as the proof", vec![c, link.to_str().unwrap()], vec![], "is the proof file"),
        ("the output", vec!["-o", r, c], vec![], "is the output file"),
        ("the formula", vec![r], vec![], "is the input file"),
        ("the policy's log", vec![c], vec![("SAT_POLICY_LOG", r)], "is the role file"),
    ];
    for (what, args, env, message) in cases {
        let (code, stdout, stderr) = run_with(&args, &env);
        assert_eq!(code, 1, "{}: {}", what, stderr);
        assert!(stderr.contains(message), "{}: {}", what, stderr);
        assert!(!stdout.lines().any(|l| l.starts_with("s ")), "{}: no answer", what);
        assert_eq!(std::fs::read_to_string(&roles).unwrap(), text, "{}: the role file is as it was", what);
    }
    // and with files of its own the same run counts
    let proof = fx.path("p.drat");
    let (code, stdout, stderr) = run_with(&["-o", out.to_str().unwrap(), c, proof.to_str().unwrap()], &[]);
    assert!(code == 10 || code == 20, "{}: {}", code, stderr);
    assert!(stdout.contains("c roles file "), "{}", stdout);
    assert_eq!(std::fs::read_to_string(&roles).unwrap(), text);
}

#[test]
fn the_wrapper_counts_only_without_an_output_directory() {
    // run.sh with an output directory writes files of its own, and cannot tell
    // whether the role file is one of them: it does not hand the role file on
    let fx = Fixture::new("roles_wrapper");
    let cnf = fx.path("r.cnf");
    random_3sat(60, 4.0, 1, &cnf);
    let roles = fx.path("r.roles");
    let text = "1-30 a\n31-60 b\n";
    std::fs::write(&roles, text).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("run.sh");
    let stock = common::STOCK_PATH.join(" ");
    let wrapper = |outdir: Option<&Path>, extra: &str| -> (String, String) {
        let mut cmd = Command::new("bash");
        for key in common::SOLVER_ENV_VARS {
            cmd.env_remove(key);
        }
        cmd.arg(&script).arg(&cnf);
        if let Some(d) = outdir {
            cmd.arg(d);
        }
        cmd.env("SAT_ROLE_FILE", &roles).env("SAT_EXTRA_ARGS", format!("{} {}", stock, extra));
        let out = cmd.output().expect("run run.sh");
        (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    // the binary itself through the wrapper: the count runs
    let (stdout, _) = wrapper(None, "--rolecount=1");
    assert!(stdout.contains("c roles file "), "{}", stdout);
    assert!(stdout.lines().any(|l| l.starts_with("s ")));
    // with an output directory and a role file named: nothing is run and no file is touched,
    // whether the option is on or not, and wherever the role file is
    let outdir = fx.path("out");
    for extra in ["--rolecount=1", ""] {
        let (stdout, stderr) = wrapper(Some(&outdir), extra);
        assert!(stdout.is_empty(), "{:?}: {}", extra, stdout);
        assert!(stderr.contains("SAT_ROLE_FILE is set"), "{:?}: {}", extra, stderr);
        assert!(!outdir.exists(), "{:?}: no output directory was made", extra);
        assert_eq!(std::fs::read_to_string(&roles).unwrap(), text);
    }
    // a role file among the wrapper's own output files stays as it was
    let inside = fx.path("out3");
    std::fs::create_dir_all(&inside).unwrap();
    for name in ["status.txt", "solver_stdout.tmp", "proof.out", "result.json", "model.txt"] {
        let own = inside.join(name);
        std::fs::write(&own, text).unwrap();
        let mut cmd = Command::new("bash");
        for key in common::SOLVER_ENV_VARS {
            cmd.env_remove(key);
        }
        cmd.arg(&script).arg(&cnf).arg(&inside);
        cmd.env("SAT_ROLE_FILE", &own).env("SAT_EXTRA_ARGS", format!("{} --rolecount=1", stock));
        let out = cmd.output().expect("run run.sh");
        assert_eq!(out.status.code(), Some(1), "{}", name);
        assert_eq!(std::fs::read_to_string(&own).unwrap(), text, "{}: the role file is as it was", name);
        std::fs::remove_file(&own).unwrap();
    }
    // without a role file named the wrapper answers as ever
    let outdir2 = fx.path("out2");
    let mut cmd = Command::new("bash");
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    cmd.arg(&script).arg(&cnf).arg(&outdir2).env("SAT_EXTRA_ARGS", &stock);
    let out = cmd.output().expect("run run.sh");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.lines().any(|l| l == "s SATISFIABLE" || l == "s UNSATISFIABLE"), "{}", stdout);
    let status = std::fs::read_to_string(outdir2.join("status.txt")).unwrap();
    assert!(status.trim() == "SAT" || status.trim() == "UNSAT", "{}", status);
}

/// Ramsey: every two-colouring of the edges of the complete graph on `n`
/// points has a red clique of `s` points or a blue one of `t`.
fn ramsey(s: usize, t: usize, n: usize) -> (usize, Vec<Vec<i64>>) {
    let mut lit = std::collections::HashMap::new();
    let mut k = 0i64;
    for i in 0..n {
        for j in i + 1..n {
            k += 1;
            lit.insert((i, j), k);
        }
    }
    fn subsets(n: usize, k: usize) -> Vec<Vec<usize>> {
        let mut out = Vec::new();
        let mut cur: Vec<usize> = (0..k).collect();
        loop {
            out.push(cur.clone());
            let mut i = k;
            while i > 0 && cur[i - 1] == n - k + i - 1 {
                i -= 1;
            }
            if i == 0 {
                return out;
            }
            cur[i - 1] += 1;
            for j in i..k {
                cur[j] = cur[j - 1] + 1;
            }
        }
    }
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for (size, sign) in [(s, -1i64), (t, 1i64)] {
        for q in subsets(n, size) {
            let mut c = Vec::new();
            for a in 0..q.len() {
                for b in a + 1..q.len() {
                    c.push(sign * lit[&(q[a], q[b])]);
                }
            }
            clauses.push(c);
        }
    }
    (k as usize, clauses)
}

#[test]
fn a_helper_of_a_structure_pass_is_the_solvers_too() {
    // the point pass adds helper variables as clauses, numbered after the
    // formula's last used variable: with 1,000 variables declared and 91 used
    // those numbers are in the role file, and are the solver's all the same
    let fx = Fixture::new("roles_structure");
    let (vars, clauses) = ramsey(3, 5, 14);
    assert_eq!(vars, 91);
    let body: String = clauses.iter().map(|c| c.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(" ") + " 0\n").collect();
    let counts = |declared: usize, tumble: &str| -> (u64, u64, u64, u64, u64, u64) {
        let cnf = fx.path(&format!("r{}.cnf", declared));
        let roles = fx.path(&format!("r{}.roles", declared));
        std::fs::write(&cnf, format!("p cnf {} {}\n{}", declared, clauses.len(), body)).unwrap();
        std::fs::write(&roles, format!("1-{} edge\n", declared)).unwrap();
        let plain = run(&cnf, &["--structpoints=1", tumble], &[]);
        let r = counted(&cnf, &roles, &["--structpoints=1", tumble], false);
        common::assert_same_trajectory(&plain, &r, "ramsey with the point pass");
        assert!(r.stdout.contains("points: 14 points, 13 lex-leader chains,"), "the point pass fired: {}", r.stdout);
        let hold = lines(&r.stdout, "hold");
        let end = at(&hold, "end");
        let of = |role: &str, key: &str| -> u64 { end.iter().find(|l| l["role"] == role).map(|l| num(l, key)).unwrap_or(0) };
        let count = lines(&r.stdout, "count");
        let on = |role: &str| -> u64 { at(&count, "end").iter().filter(|l| l["role"] == role).map(|l| num(l, "decide_own")).sum() };
        assert_eq!(of("edge", "vars"), declared as u64);
        // the helpers: the solver's own variables less those its own techniques made
        let helpers = of("new", "vars") - r.stats.get("variables_extension").copied().unwrap_or(0);
        // the declared numbers no clause uses; the helpers take the first of them; with tumble
        // off the solver registers the rest all the same and holds them
        let unused_declared = declared as u64 - 91;
        let taken = helpers.min(unused_declared);
        let registered_unused = if tumble == "--tumble=0" { unused_declared - taken } else { 0 };
        assert_eq!(of("edge", "absent"), unused_declared - registered_unused, "{} {}", declared, tumble);
        let in_use = of("edge", "held") + of("edge", "fixed") + of("edge", "eliminated") + of("edge", "merged_held") + of("edge", "merged_fixed") + of("edge", "merged_eliminated");
        assert_eq!(in_use, 91 + registered_unused, "{} {}", declared, tumble);
        (helpers, on("new"), on("edge"), of("new", "vars"), in_use, of("edge", "absent"))
    };
    // with tumble off the solver registers every declared variable before the formula is read,
    // so the helpers' numbers are ones it already has: they are the solver's all the same
    for tumble in ["--tumble=1", "--tumble=0"] {
        let exact = counts(91, tumble);
        let wide = counts(1000, tumble);
        assert!(exact.0 > 0, "{}: the point pass adds variables", tumble);
        // the helpers are the solver's, whatever the header declares
        assert_eq!(exact.0, wide.0, "{}: the helpers", tumble);
        assert!(wide.1 > 0 && wide.2 > 0, "{}: decisions on both", tumble);
        if tumble == "--tumble=1" {
            // the same search too (with tumble off the solver's inner numbering goes by the
            // header, so the search itself is another one)
            // (all but the last number: the declared numbers nobody uses are the header's)
            assert_eq!((exact.0, exact.1, exact.2, exact.3, exact.4), (wide.0, wide.1, wide.2, wide.3, wide.4), "the same count whatever the header declares");
            // of the file's 1,000 edges 909 are in no clause; 220 of those numbers are helpers now
            assert_eq!((exact.5, wide.5), (0, 1000 - 91));
        } else {
            // tumble off: only the 220 numbers the helpers took are in no clause as edges; the
            // other 689 the solver holds
            assert_eq!((exact.5, wide.5), (0, 220));
            assert_eq!((exact.4, wide.4), (91, 91 + 689));
        }
    }
}

#[test]
fn a_conflict_found_while_the_formula_is_loaded_is_counted_apart() {
    // (1 | 2), (1 | -2), (-1): refuted by propagation as the last clause comes in, before any
    // role is installed.  That conflict is in the run's total and in neither mode's.
    let fx = Fixture::new("roles_early");
    let cnf = fx.path("u.cnf");
    let roles = fx.path("u.roles");
    std::fs::write(&cnf, "p cnf 2 3\n1 2 0\n1 -2 0\n-1 0\n").unwrap();
    std::fs::write(&roles, "1-2 all\n").unwrap();
    for shipped in [false, true] {
        let plain = if shipped { run_shipped(&cnf, &[], &[]) } else { run(&cnf, &[], &[]) };
        let r = counted(&cnf, &roles, &[], shipped);
        common::assert_same_trajectory(&plain, &r, "refuted while loaded");
        assert_eq!(r.status, "UNSATISFIABLE");
        let conflicts = r.stats.get("conflicts").copied().unwrap_or(0);
        let file = &lines(&r.stdout, "file")[0];
        let search = lines(&r.stdout, "search");
        let in_modes: u64 = at(&search, "end").iter().map(|l| num(l, "mode_conflicts")).sum();
        assert_eq!(num(file, "conflicts_before") + in_modes, conflicts, "shipped {}: {:?}", shipped, file);
        assert_eq!(in_modes, 0, "shipped {}: no search, no conflict of a mode", shipped);
        assert_eq!(r.stats.get("decisions").copied().unwrap_or(0), 0);
    }
}

#[test]
fn a_long_chain_of_replacements_is_walked_once() {
    // x1 = x2 = ... = x200000, each equality two clauses, beside a pigeonhole formula that
    // needs a search.  With probing off, elimination removes the chain one variable at a
    // time, each replaced by the next: one chain as long as the formula.  A report that
    // walked it from every variable would take minutes.
    let fx = Fixture::new("roles_chain");
    let cnf = fx.path("chain.cnf");
    let roles = fx.path("chain.roles");
    let n = 200_000u64;
    let (pigeons, holes) = (8u64, 7u64);
    let mut text = String::new();
    let mut count = 0u64;
    for i in 1..n {
        text.push_str(&format!("-{} {} 0\n{} -{} 0\n", i, i + 1, i, i + 1));
        count += 2;
    }
    let var = |p: u64, h: u64| n + p * holes + h + 1;
    for p in 0..pigeons {
        for h in 0..holes {
            text.push_str(&format!("{} ", var(p, h)));
        }
        text.push_str("0\n");
        count += 1;
    }
    for h in 0..holes {
        for a in 0..pigeons {
            for b in a + 1..pigeons {
                text.push_str(&format!("-{} -{} 0\n", var(a, h), var(b, h)));
                count += 1;
            }
        }
    }
    std::fs::write(&cnf, format!("p cnf {} {}\n{}", n + pigeons * holes, count, text)).unwrap();
    std::fs::write(&roles, format!("1-{} link\n{}-{} pigeon\n", n, n + 1, n + pigeons * holes)).unwrap();
    let options = ["--lucky=0", "--probe=0", "--eliminateinit=0"];
    let plain = run(&cnf, &options, &[]);
    let started = std::time::Instant::now();
    let r = counted(&cnf, &roles, &options, false);
    let seconds = started.elapsed().as_secs_f64();
    common::assert_same_trajectory(&plain, &r, "a long chain");
    let file = &lines(&r.stdout, "file")[0];
    assert_eq!(num(file, "equal_eliminations"), n - 1, "{:?}", file);
    // every link but the last stands behind the chain's end, whatever became of that one
    let hold = lines(&r.stdout, "hold");
    let end = at(&hold, "end");
    let link = end.iter().find(|l| l["role"] == "link").unwrap();
    let followed = num(link, "merged_held") + num(link, "merged_fixed") + num(link, "merged_eliminated");
    assert_eq!(followed, n - 1, "{:?}", link);
    assert_eq!(num(link, "vars"), n);
    // (a walk from every variable took 19 s for half this chain on an idle machine, and four
    // times that for this one)
    assert!(seconds < 30.0, "the counted run took {:.1} s", seconds);
}

#[test]
fn a_formula_read_in_the_relaxed_mode_can_use_variables_past_its_header() {
    // `p cnf 1 1` and a clause over variables 2 and 3: the relaxed parser takes it, and the
    // role file may name those variables
    let fx = Fixture::new("roles_relaxed");
    let cnf = fx.path("r.cnf");
    let roles = fx.path("r.roles");
    std::fs::write(&cnf, "p cnf 1 1\n2 3 0\n").unwrap();
    std::fs::write(&roles, "2-3 choice\n").unwrap();
    for shipped in [false, true] {
        let plain = if shipped { run_shipped(&cnf, &["--relaxed"], &[]) } else { run(&cnf, &["--relaxed"], &[]) };
        assert_eq!(plain.status, "SATISFIABLE", "{}", plain.stderr);
        let r = counted(&cnf, &roles, &["--relaxed"], shipped);
        common::assert_same_trajectory(&plain, &r, "relaxed parsing");
        let hold = lines(&r.stdout, "hold");
        let end = at(&hold, "end");
        let choice = end.iter().find(|l| l["role"] == "choice").unwrap_or_else(|| panic!("shipped {}: {}", shipped, r.stdout));
        assert_eq!(num(choice, "vars"), 2, "shipped {}", shipped);
        assert!(end.iter().all(|l| l["role"] != "new"), "shipped {}: no variable is the solver's own: {:?}", shipped, end);
        // a variable past the largest one any clause uses is still refused
        let bad = fx.path("bad.roles");
        std::fs::write(&bad, "4 choice\n").unwrap();
        let refused = counted(&cnf, &bad, &["--relaxed"], shipped);
        assert_eq!(refused.exit_code, 1);
        assert!(refused.stderr.contains("not in the formula"), "{}", refused.stderr);
    }
}

#[test]
fn a_run_ended_by_a_signal_prints_what_it_had() {
    let fx = Fixture::new("roles_signal");
    let cnf = fx.path("php.cnf");
    let roles = fx.path("php.roles");
    php(12, &cnf); // the stock search does not finish this in the test's time
    one_role(12 * 13, &roles);
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    cmd.arg("-n").args(common::STOCK_PATH).arg("--rolecount=1").arg(&cnf);
    cmd.env("SAT_ROLE_FILE", &roles).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd.spawn().expect("run sat-solver");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let killed = Command::new("kill").arg("-TERM").arg(child.id().to_string()).status().expect("kill");
    assert!(killed.success(), "the solver was still running");
    let mut stdout = String::new();
    child.stdout.take().unwrap().read_to_string(&mut stdout).unwrap();
    let status = child.wait().unwrap();
    assert!(status.code().is_none(), "ended by the signal: {:?}", status);
    assert!(!stdout.lines().any(|l| l.starts_with("s ")), "no answer");
    let file = lines(&stdout, "file");
    assert_eq!(file.len(), 1, "{}", stdout);
    assert_eq!(file[0]["ended"], "signal");
    // nothing of the solver is read from the handler: the last regular checkpoint stands
    let hold = lines(&stdout, "hold");
    assert!(at(&hold, "end").is_empty(), "no checkpoint from the signal handler");
    assert!(!at(&hold, "start").is_empty(), "the checkpoint of the first decision");
}
