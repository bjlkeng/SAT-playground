//! The number pass (bead SAT-playground-d96.9.4, src/structnum.rs): numbers
//! stored as sorted bits, and the clauses their encoding lacks.
//!
//! The toy of these tests is the README's: x in 0..3 as three sorted bits
//! g1, g2, g3 (variables 1, 2, 3), a test T = 4 for "x = 2" written as a
//! count, and the count's helpers 7 = "at least one bit", 5 = "at least
//! two", 6 = "all three". The pass must add the four two-literal clauses
//! that say the helpers are the bits, in the order `tools/number_read.py`
//! lists them, with a proof the gate accepts; it must be silent when it is
//! off, change nothing on a formula without such numbers, stop at its work
//! limit, and leave a chain it cannot read alone. That the pass and the
//! Python reader agree on every toy of Z3's catalog is
//! `tools/number_pass_check.py catalog --proof`.

mod common;

use common::{php, run, run_shipped, solver_bin, Fixture};
use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;
use std::process::Command;

fn gate(cnf: &Path, proof: &Path) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new("python3").arg(root.join("tools/proof_gate.py")).arg(cnf).arg(proof).output().expect("run proof_gate.py");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write_cnf(path: &Path, vars: i64, clauses: &[Vec<i64>]) {
    let mut f = std::fs::File::create(path).expect("create cnf");
    writeln!(f, "p cnf {} {}", vars, clauses.len()).unwrap();
    for c in clauses {
        let line: Vec<String> = c.iter().map(|l| l.to_string()).collect();
        writeln!(f, "{} 0", line.join(" ")).unwrap();
    }
}

/// The toy: 7 variables, 19 clauses.
fn toy() -> Vec<Vec<i64>> {
    vec![
        vec![1, -2],
        vec![2, -3],
        vec![-7, 1, 2, 3],
        vec![7, -1],
        vec![7, -2],
        vec![7, -3],
        vec![-5, 1, 2],
        vec![-5, 1, 3],
        vec![-5, 2, 3],
        vec![5, -1, -2],
        vec![5, -1, -3],
        vec![5, -2, -3],
        vec![-6, 1],
        vec![-6, 2],
        vec![-6, 3],
        vec![6, -1, -2, -3],
        vec![-4, 5],
        vec![-4, -6],
        vec![4, -5, 6],
    ]
}

/// Four clauses that force `lit` by a case split over two fresh variables
/// `a` and `b`. Propagation does not see it, not even with every bit set:
/// with `lit` off they are four two-literal clauses with no unit among them.
fn force(lit: i64, a: i64, b: i64) -> Vec<Vec<i64>> {
    vec![vec![lit, a, b], vec![lit, a, -b], vec![lit, -a, b], vec![lit, -a, -b]]
}

/// The stock path plus the number pass, with a proof file. (stdout, stderr)
fn run_num(cnf: &Path, proof: &Path, extra: &[&str]) -> (String, String) {
    let mut cmd = Command::new(solver_bin());
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    let out = cmd.arg("-n").args(common::STOCK_PATH).arg("--structnum=1").args(extra).arg(cnf).arg(proof).output().expect("run sat-solver");
    assert!(out.status.code().is_some(), "the solver was ended by a signal: {}", String::from_utf8_lossy(&out.stderr));
    (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// The fields of the pass's report line.
fn report(stdout: &str) -> BTreeMap<String, String> {
    let lines: Vec<&str> = stdout.lines().filter(|l| l.starts_with("c numbers ")).collect();
    assert_eq!(lines.len(), 1, "one 'c numbers' line is expected:\n{}", stdout);
    lines[0].split_whitespace().skip(2).filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn derived(proof: &Path) -> Vec<Vec<i64>> {
    let text = std::fs::read_to_string(format!("{}.derived.cnf", proof.display())).unwrap_or_default();
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('p') && !l.starts_with('c'))
        .map(|l| l.split_whitespace().map(|t| t.parse::<i64>().unwrap()).filter(|&v| v != 0).collect())
        .collect()
}

fn status(stdout: &str) -> String {
    stdout.lines().find_map(|l| l.strip_prefix("s ").map(|s| s.trim().to_string())).unwrap_or_default()
}

#[test]
fn the_toy_gets_its_four_clauses_in_the_readers_order() {
    let fx = Fixture::new("num-toy");
    let (cnf, proof) = (fx.path("toy.cnf"), fx.path("toy.proof"));
    write_cnf(&cnf, 7, &toy());
    let (out, _err) = run_num(&cnf, &proof, &[]);
    assert_eq!(status(&out), "SATISFIABLE");
    // h2 -> g2, g2 -> h2, not h3 -> not g3, h1 -> g1; "g1 -> h1" and "h3 -> g3"
    // are clauses of the toy, and with the four in the test's own links follow
    assert_eq!(derived(&proof), vec![vec![-5, 2], vec![5, -2], vec![6, -3], vec![-7, 1]]);
    let r = report(&out);
    for (k, v) in [("status", "read"), ("numbers", "1"), ("values", "4x1"), ("counted", "1"), ("added", "4"), ("equal", "4"), ("noproof", "0")] {
        assert_eq!(r.get(k).map(String::as_str), Some(v), "{} in {:?}", k, r);
    }
    assert!(out.contains("c structure pass: numbers: 1 chains of sorted bits"), "{}", out);
}

#[test]
fn a_refuted_toy_passes_the_gate_and_a_changed_proof_does_not() {
    // "x = 2" and "x >= 3" at once, each forced by a case split that
    // propagation does not see, so the pass reads the number as it is and
    // the search then refutes the formula
    let fx = Fixture::new("num-unsat");
    let (cnf, proof) = (fx.path("u.cnf"), fx.path("u.proof"));
    let mut cl = toy();
    cl.extend(force(4, 8, 9));
    cl.extend(force(3, 10, 11));
    write_cnf(&cnf, 11, &cl);
    let (out, _err) = run_num(&cnf, &proof, &[]);
    assert_eq!(status(&out), "UNSATISFIABLE");
    assert_eq!(report(&out).get("added").map(String::as_str), Some("4"));
    assert_eq!(gate(&cnf, &proof), "ok");
    // a step that does not follow
    let pbp = format!("{}.pbp", proof.display());
    let text = std::fs::read_to_string(&pbp).unwrap();
    assert!(text.contains("rup 1 ~x5 1 x2 >= 1 ;"), "{}", text);
    std::fs::write(&pbp, text.replace("rup 1 ~x5 1 x2 >= 1 ;", "rup 1 x5 1 x2 >= 1 ;")).unwrap();
    assert!(gate(&cnf, &proof).starts_with("FAIL"), "{}", gate(&cnf, &proof));
    std::fs::write(&pbp, &text).unwrap();
    assert_eq!(gate(&cnf, &proof), "ok");
    // a derived clause that the proof never showed
    let der = format!("{}.derived.cnf", proof.display());
    let dtext = std::fs::read_to_string(&der).unwrap();
    assert!(dtext.contains("-5 2 0"), "{}", dtext);
    std::fs::write(&der, dtext.replace("-5 2 0", "5 2 0")).unwrap();
    assert!(gate(&cnf, &proof).starts_with("FAIL"), "{}", gate(&cnf, &proof));
}

#[test]
fn a_value_no_model_takes_becomes_a_clause() {
    // x = 2 is ruled out behind a helper: "g2 -> g3 or 8" and "g2 -> g3 or
    // not 8". Propagation sees it only when x is set to 2, which the reading
    // does: the value is dead, and "g2 -> g3" is added. With "x = 2" forced
    // by a case split the formula has no model, and the gate checks it all
    let fx = Fixture::new("num-dead");
    let (cnf, proof) = (fx.path("d.cnf"), fx.path("d.proof"));
    let mut cl = toy();
    cl.extend([vec![-2, 3, 8], vec![-2, 3, -8]]);
    cl.extend(force(4, 9, 10));
    write_cnf(&cnf, 10, &cl);
    let (out, _err) = run_num(&cnf, &proof, &[]);
    assert_eq!(status(&out), "UNSATISFIABLE");
    assert!(derived(&proof).contains(&vec![-2, 3]), "{:?}", derived(&proof));
    assert_ne!(report(&out).get("dead").map(String::as_str), Some("0"), "{:?}", report(&out));
    assert_eq!(gate(&cnf, &proof), "ok");
}

#[test]
fn off_by_default_and_nothing_changes_without_such_numbers() {
    let fx = Fixture::new("num-off");
    let cnf = fx.path("toy.cnf");
    write_cnf(&cnf, 7, &toy());
    // the shipped defaults and the stock path: no trace of the pass
    for r in [run_shipped(&cnf, &[], &[]), run(&cnf, &[], &[])] {
        assert_eq!(r.status, "SATISFIABLE");
        assert!(!r.stdout.contains("c numbers "), "{}", r.stdout);
        assert!(!r.stdout.contains("numbers:"), "{}", r.stdout);
    }
    // pigeons: no chain of three, nothing derived, and the search is the one
    // without the pass, counter for counter (the pass's own work aside)
    let ph = fx.path("php7.cnf");
    php(7, &ph);
    let off = run(&ph, &[], &[]);
    let on = run(&ph, &["--structnum=1"], &[]);
    assert_eq!(on.status, "UNSATISFIABLE");
    let r = report(&on.stdout);
    assert_eq!(r.get("status").map(String::as_str), Some("read"));
    assert_eq!(r.get("added").map(String::as_str), Some("0"));
    assert_eq!(r.get("numbers").map(String::as_str), Some("0"));
    for key in ["conflicts", "decisions", "propagations", "restarts", "reductions"] {
        assert_eq!(off.stats.get(key), on.stats.get(key), "{} differs with the pass on", key);
    }
    let own: u64 = r["work"].parse().unwrap();
    assert_eq!(on.work(), off.work() + own, "the pass's work is in the work clock, and nothing else moved");
}

#[test]
fn the_pass_stops_at_its_work_limit() {
    // 100 variables, at most one of them (every pair forbidden) and the
    // clause "at least one" a hundred times over: each copy looks like a
    // wide gate for each of its 100 literals, 100 x 100 x 100 lookups in all.
    // The limit must hold inside that loop, not after it
    let fx = Fixture::new("num-limit");
    let cnf = fx.path("amo.cnf");
    let mut cl: Vec<Vec<i64>> = Vec::new();
    for a in 1..=100i64 {
        for b in a + 1..=100 {
            cl.push(vec![-a, -b]);
        }
    }
    for _ in 0..100 {
        cl.push((1..=100).collect());
    }
    write_cnf(&cnf, 100, &cl);
    let free = run(&cnf, &["--structnum=1"], &[]);
    let full: u64 = report(&free.stdout)["work"].parse().unwrap();
    assert!(full > 1_000_000, "the example no longer costs what this test is about: {}", full);
    let off = run(&cnf, &[], &[]);
    let cut = run(&cnf, &["--structnum=1", "--structnumticks=100"], &[]);
    let r = report(&cut.stdout);
    assert_eq!(r.get("status").map(String::as_str), Some("stopped"), "{:?}", r);
    let work: u64 = r["work"].parse().unwrap();
    assert!(work <= 100_000 + 20_000, "the pass ran to {} with a limit of 100,000", work);
    assert_eq!(cut.status, off.status);
    assert_eq!(cut.work(), off.work() + work, "what the pass spent before it stopped is in the work clock");
    // with no room at all the pass does not start
    let none = run(&cnf, &["--structnum=1", "--structnumticks=1"], &[]);
    assert!(none.stdout.contains("c numbers status=no-budget"), "{}", none.stdout);
    assert_eq!(none.status, off.status);
    assert_eq!(none.work(), off.work());
}

#[test]
fn a_chain_past_the_mask_is_left_alone() {
    // 64 sorted bits, each with a helper equal to it by an OR with the bit
    // above (so the chain is under a count): the values do not fit the masks
    // the patterns are kept in. The pass must leave the chain unread and not
    // trip over it
    let chain = |bits: i64| -> (i64, Vec<Vec<i64>>) {
        let mut cl: Vec<Vec<i64>> = (1..bits).map(|k| vec![k, -(k + 1)]).collect();
        for k in 1..bits {
            // h = bits + k is "bit k or bit k + 1", which is bit k
            let h = bits + k;
            cl.extend([vec![-h, k, k + 1], vec![h, -k], vec![h, -(k + 1)]]);
        }
        (2 * bits - 1, cl)
    };
    let fx = Fixture::new("num-long");
    let (cnf, proof) = (fx.path("long.cnf"), fx.path("long.proof"));
    let (vars, cl) = chain(64);
    write_cnf(&cnf, vars, &cl);
    let (out, err) = run_num(&cnf, &proof, &[]);
    assert_eq!(status(&out), "SATISFIABLE", "{}\n{}", out, err);
    let r = report(&out);
    assert_eq!(r.get("status").map(String::as_str), Some("read"), "{:?}", r);
    assert_eq!(r.get("numbers").map(String::as_str), Some("0"), "{:?}", r);
    assert_eq!(r.get("added").map(String::as_str), Some("0"), "{:?}", r);
    assert_ne!(r.get("long").map(String::as_str), Some("0"), "{:?}", r);
    // 62 bits are read: "h -> bit k" is what the encoding lacks, for each helper
    let (vars, cl) = chain(62);
    write_cnf(&cnf, vars, &cl);
    let (out, _err) = run_num(&cnf, &proof, &[]);
    let r = report(&out);
    assert_eq!(r.get("long").map(String::as_str), Some("0"), "{:?}", r);
    assert_eq!(r.get("numbers").map(String::as_str), Some("1"), "{:?}", r);
    let d = derived(&proof);
    assert_eq!(d.len(), 61, "{:?}", d);
    assert!(d.contains(&vec![-63, 1]) && d.contains(&vec![-123, 61]), "{:?}", d);
}

#[test]
fn a_very_long_chain_costs_its_length_and_no_more() {
    // 200,000 variables, each implying the one before. No number is read off
    // it (it is far past the masks), and finding that out must cost about
    // its length: walking it with one path copied a step, or testing every
    // member against all the others, would be its length squared
    let fx = Fixture::new("num-huge");
    let cnf = fx.path("huge.cnf");
    let n = 200_000i64;
    let cl: Vec<Vec<i64>> = (1..n).map(|k| vec![k, -(k + 1)]).collect();
    write_cnf(&cnf, n, &cl);
    let on = run(&cnf, &["--structnum=1"], &[]);
    let r = report(&on.stdout);
    assert_eq!(r.get("status").map(String::as_str), Some("read"), "{:?}", r);
    assert_eq!(r.get("added").map(String::as_str), Some("0"), "{:?}", r);
    assert_ne!(r.get("long").map(String::as_str), Some("0"), "{:?}", r);
    let work: u64 = r["work"].parse().unwrap();
    assert!(work < 40 * n as u64, "{} units of work for a chain of {}", work, n);
    // and it stops at a limit set in the middle of the walk
    let cut = run(&cnf, &["--structnum=1", "--structnumticks=1500"], &[]);
    let r = report(&cut.stdout);
    assert_eq!(r.get("status").map(String::as_str), Some("stopped"), "{:?}", r);
    let spent: u64 = r["work"].parse().unwrap();
    assert!(spent <= 1_500_000 + 20_000, "the pass ran to {} with a limit of 1,500,000", spent);
}

#[test]
fn work_inside_a_long_clause_is_counted_and_limited() {
    // four sorted bits under a count, and 2,000 gates in a row that go off one
    // after the other once the lowest bit is off (gate j is "gate j - 1 and
    // t"), with one clause over all of them. Each time a gate goes off, the
    // clause is searched for another literal to watch, past all the gates
    // that went off before: 1 + 2 + ... + 2,000 looks, two million a
    // propagation. They are work, and the limit holds in the middle of one
    let fx = Fixture::new("num-wide");
    let cnf = fx.path("wide.cnf");
    let mut cl: Vec<Vec<i64>> = vec![vec![1, -2], vec![2, -3], vec![3, -4]];
    for k in 1..4i64 {
        let h = 4 + k; // 5, 6, 7: "bit k or bit k + 1", which is bit k
        cl.extend([vec![-h, k, k + 1], vec![h, -k], vec![h, -(k + 1)]]);
    }
    let t = 8i64;
    let gate = |j: i64| 8 + j; // gates 9 ..= 2008
    let count = 2000i64;
    for j in 1..=count {
        let below = if j == 1 { 1 } else { gate(j - 1) };
        cl.extend([vec![-gate(j), below], vec![-gate(j), t], vec![gate(j), -below, -t]]);
    }
    let mut long: Vec<i64> = (1..=count).map(gate).collect();
    long.push(gate(count) + 1);
    cl.push(long);
    write_cnf(&cnf, gate(count) + 1, &cl);
    let on = run(&cnf, &["--structnum=1"], &[]);
    let r = report(&on.stdout);
    assert_eq!(r.get("status").map(String::as_str), Some("read"), "{:?}", r);
    assert_eq!(r.get("numbers").map(String::as_str), Some("1"), "{:?}", r);
    let work: u64 = r["work"].parse().unwrap();
    assert!(work > 2_000_000, "the looks inside the long clause are not in the work: {}", work);
    let cut = run(&cnf, &["--structnum=1", "--structnumticks=300"], &[]);
    let r = report(&cut.stdout);
    assert_eq!(r.get("status").map(String::as_str), Some("stopped"), "{:?}", r);
    let spent: u64 = r["work"].parse().unwrap();
    assert!(spent <= 300_000 + 10_000, "the pass ran to {} with a limit of 300,000", spent);
    assert_eq!(r.get("added").map(String::as_str), Some("0"), "nothing may be concluded from a propagation cut short: {:?}", r);
    assert_eq!(cut.status, on.status);
}

#[test]
fn the_limit_holds_while_the_units_are_set() {
    // the same row of 2,000 gates and the clause over them, but now a unit
    // switches the first gate off: the two million looks happen while the
    // formula's own units are set, before any reading. The limit must be
    // there from the first step
    let fx = Fixture::new("num-root");
    let cnf = fx.path("root.cnf");
    let gate = |j: i64| 1 + j; // gates 2 ..= 2001; variable 1 is "t"
    let count = 2000i64;
    let mut cl: Vec<Vec<i64>> = Vec::new();
    for j in 2..=count {
        cl.extend([vec![-gate(j), gate(j - 1)], vec![-gate(j), 1], vec![gate(j), -gate(j - 1), -1]]);
    }
    let mut long: Vec<i64> = (1..=count).map(gate).collect();
    long.push(gate(count) + 1);
    cl.push(long);
    cl.push(vec![-gate(1)]);
    write_cnf(&cnf, gate(count) + 1, &cl);
    let free = run(&cnf, &["--structnum=1"], &[]);
    let full: u64 = report(&free.stdout)["work"].parse().unwrap();
    assert!(full > 1_500_000, "the example no longer costs what this test is about: {}", full);
    let cut = run(&cnf, &["--structnum=1", "--structnumticks=100"], &[]);
    let r = report(&cut.stdout);
    assert_eq!(r.get("status").map(String::as_str), Some("stopped"), "{:?}", r);
    let spent: u64 = r["work"].parse().unwrap();
    assert!(spent <= 100_000 + 10_000, "the pass ran to {} with a limit of 100,000", spent);
    assert_eq!(cut.status, free.status);
}
