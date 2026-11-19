//! The number pass (bead SAT-playground-d96.9.4, src/structnum.rs): numbers
//! stored as sorted bits, and the clauses their encoding lacks.
//!
//! The toy of these tests is the README's: x in 0..3 as three sorted bits
//! g1, g2, g3 (variables 1, 2, 3), a test T = 4 for "x = 2" written as a
//! count, and the count's helpers 7 = "at least one bit", 5 = "at least
//! two", 6 = "all three". The pass must add the four two-literal clauses
//! that say the helpers are the bits, in the order `tools/number_read.py`
//! lists them, with a proof the gate accepts. It must be silent when it is
//! off, add nothing where no number is under a count, keep helpers off the
//! ends of a number (many ends on one bit; a helper on each end), stop at
//! its work limit wherever that falls (in the units, inside a long clause,
//! in the look for gates), end early on a formula with too many paths or
//! with candidates that are no numbers under a count, and leave a chain it
//! cannot read alone. That the pass and the Python reader agree on every
//! toy of Z3's catalog is `tools/number_pass_check.py catalog --proof`.

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

/// The toy with 60 ends a side: 3,679 clauses. With the fan rule the pass
/// costs 99,899 units of work (about the formula's size); with the rule
/// switched off it cost 14,624,631 (measured 2026-11-19).
const WORK_BOUND_FAN: u64 = 400_000;

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
    assert!(work < 200 * n as u64, "{} units of work for a chain of {}", work, n);
    // and it stops at a limit set in the middle of the walk (the pass starts
    // only with room for its tables and two walks of the formula: 2.4M here)
    let cut = run(&cnf, &["--structnum=1", "--structnumticks=5000"], &[]);
    let r = report(&cut.stdout);
    assert_eq!(r.get("status").map(String::as_str), Some("stopped"), "{:?}", r);
    let spent: u64 = r["work"].parse().unwrap();
    assert!(spent <= 5_000_000 + 20_000, "the pass ran to {} with a limit of 5,000,000", spent);
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

/// "x = 2" and "x >= 3" at once, each forced by a case split over two fresh
/// variables that propagation does not see: the pass reads the number as it
/// is, and the search then refutes the formula, so the gate has a proof to
/// check. Returns the number of variables.
fn refute(cl: &mut Vec<Vec<i64>>, vars: i64) -> i64 {
    cl.extend(force(4, vars + 1, vars + 2));
    cl.extend(force(3, vars + 3, vars + 4));
    vars + 4
}

/// The toy with `tops` variables that each need x at its top value (p -> g3)
/// and `lows` that each need it at 0 (q -> not g1), no two of a kind on at
/// once. They are true inputs like the bits, so each pair of them makes a
/// chain of its own over the same three bits.
fn toy_with_ends(tops: i64, lows: i64) -> (i64, Vec<Vec<i64>>) {
    let mut cl = toy();
    let p = |i: i64| 7 + i; // 8 ..
    let q = |j: i64| 7 + tops + j;
    for i in 1..=tops {
        cl.push(vec![-p(i), 3]);
        for k in i + 1..=tops {
            cl.push(vec![-p(i), -p(k)]);
        }
    }
    for j in 1..=lows {
        cl.push(vec![-q(j), -1]);
        for k in j + 1..=lows {
            cl.push(vec![-q(j), -q(k)]);
        }
    }
    (7 + tops + lows, cl)
}

#[test]
fn many_ends_on_one_bit_are_no_bits_of_the_number() {
    // six on top and six below: more than four ends on one member, so none of
    // them is followed, the chain is the three bits, and the toy's four
    // clauses come out
    let fx = Fixture::new("num-fan");
    let (cnf, proof) = (fx.path("fan.cnf"), fx.path("fan.proof"));
    let (vars, mut cl) = toy_with_ends(6, 6);
    let vars = refute(&mut cl, vars);
    write_cnf(&cnf, vars, &cl);
    let (out, _err) = run_num(&cnf, &proof, &[]);
    let r = report(&out);
    assert_eq!(r.get("status").map(String::as_str), Some("read"), "{:?}", r);
    assert_eq!(derived(&proof), vec![vec![-5, 2], vec![5, -2], vec![6, -3], vec![-7, 1]], "{:?}", r);
    assert!(r["fanlinks"].parse::<u64>().unwrap() >= 12, "{:?}", r);
    assert_eq!(r.get("countedvalues").map(String::as_str), Some("4x1"), "{:?}", r);
    assert_eq!(status(&out), "UNSATISFIABLE");
    assert_eq!(gate(&cnf, &proof), "ok");
    // and the cost does not grow with the pairs: 60 on top and 60 below are
    // 3,600 pairs, each a chain to test if they were followed
    let (big, big_proof) = (fx.path("big.cnf"), fx.path("big.proof"));
    let (vars, cl) = toy_with_ends(60, 60);
    write_cnf(&big, vars, &cl);
    let (out, _err) = run_num(&big, &big_proof, &[]);
    let r = report(&out);
    assert_eq!(derived(&big_proof), vec![vec![-5, 2], vec![5, -2], vec![6, -3], vec![-7, 1]], "{:?}", r);
    let work: u64 = r["work"].parse().unwrap();
    assert!(work < WORK_BOUND_FAN, "the pass spent {} on the toy with 60 ends a side", work);
}

#[test]
fn a_helper_on_each_end_is_cut_off_the_number() {
    // a (8) needs x at its top value and sets c (10); b (9) needs x at 0 and
    // sets d (11). Each is read by something, so the end test keeps both, and
    // the chain "not b, g1, g2, g3, a" has six values with three steps that
    // the count reads: not under a count. The number is the stretch between
    // the lowest and the highest step some variable equals: the three bits
    let fx = Fixture::new("num-core");
    let (cnf, proof) = (fx.path("core.cnf"), fx.path("core.proof"));
    let mut cl = toy();
    cl.extend([vec![-8, 3], vec![-8, 10], vec![-9, -1], vec![-9, 11]]);
    let vars = refute(&mut cl, 11);
    write_cnf(&cnf, vars, &cl);
    let (out, _err) = run_num(&cnf, &proof, &[]);
    let r = report(&out);
    assert_eq!(r.get("status").map(String::as_str), Some("read"), "{:?}", r);
    assert_eq!(derived(&proof), vec![vec![-5, 2], vec![5, -2], vec![6, -3], vec![-7, 1]], "{:?}", r);
    assert_eq!(r.get("corecuts").map(String::as_str), Some("1"), "{:?}", r);
    assert_eq!(r.get("countedvalues").map(String::as_str), Some("4x1"), "{:?}", r);
    assert_eq!(status(&out), "UNSATISFIABLE");
    assert_eq!(gate(&cnf, &proof), "ok");
}

#[test]
fn the_look_for_gates_stops_at_the_limit_by_work() {
    // 1,000 clauses of 1,000 literals and no two-literal clause: every scan of
    // the formula is a million looks. A limit that ends in the middle of a
    // scan must hold there, whichever scan it is (the look for gates used to
    // check every 1,024 clauses, which here was never). One unit on a
    // variable of its own makes the pass walk the long clauses twice after
    // loading them: with nothing set at the root it skips them once
    let fx = Fixture::new("num-wide");
    let cnf = fx.path("wide.cnf");
    let n = 1000i64;
    let mut cl: Vec<Vec<i64>> = (1..=n).map(|i| (1..=n).map(|v| if v == i { -v } else { v }).collect()).collect();
    // (without the unit: the load and one walk, about two million)
    let plain = fx.path("plain.cnf");
    write_cnf(&plain, n, &cl);
    let two: u64 = report(&run(&plain, &["--structnum=1"], &[]).stdout)["work"].parse().unwrap();
    assert!(two < 2_100_000, "the long clauses were walked more than twice: {}", two);
    cl.push(vec![n + 1]);
    write_cnf(&cnf, n + 1, &cl);
    let free = run(&cnf, &["--structnum=1"], &[]);
    let full: u64 = report(&free.stdout)["work"].parse().unwrap();
    assert!(full > 3_000_000, "the example no longer costs what this test is about: {}", full);
    for limit_k in [2002u64, 2400, 2900, 3300] {
        if limit_k * 1000 >= full {
            continue;
        }
        let cut = run(&cnf, &["--structnum=1", &format!("--structnumticks={}", limit_k)], &[]);
        let r = report(&cut.stdout);
        let spent: u64 = r.get("work").map(|w| w.parse().unwrap()).unwrap_or(0);
        assert!(matches!(r.get("status").map(String::as_str), Some("stopped") | Some("no-budget")), "limit {}K: {:?}", limit_k, r);
        assert!(spent <= limit_k * 1000 + 10_000, "the pass ran to {} with a limit of {}K", spent, limit_k);
        assert_eq!(cut.status, free.status);
    }
}

#[test]
fn nothing_is_added_where_no_number_is_under_a_count() {
    // g3 -> g2 -> m -> g1, and "g1 and not g2 -> m": a chain of four with a
    // helper in it. The bit test finds that m is on whenever g1 is (a clause
    // propagation does not have), but no count reads the chain, so the
    // clause is held back and the solver gets nothing
    let fx = Fixture::new("num-held");
    let (cnf, proof) = (fx.path("held.cnf"), fx.path("held.proof"));
    let mut cl: Vec<Vec<i64>> = vec![vec![-3, 2], vec![-2, 1], vec![-4, 3], vec![2, -1, 3]];
    // refuted by a case split that propagation does not see, so there is a proof to check
    cl.extend(force(4, 5, 6));
    cl.extend(force(-1, 7, 8));
    write_cnf(&cnf, 8, &cl);
    let (out, _err) = run_num(&cnf, &proof, &[]);
    let r = report(&out);
    assert_eq!(r.get("status").map(String::as_str), Some("read"), "{:?}", r);
    assert_eq!(r.get("held").map(String::as_str), Some("1"), "{:?}", r);
    assert_eq!(r.get("added").map(String::as_str), Some("0"), "{:?}", r);
    assert_eq!(r.get("dead").map(String::as_str), Some("0"), "the report counts what was written: {:?}", r);
    assert!(derived(&proof).is_empty(), "{:?}", derived(&proof));
    assert_eq!(status(&out), "UNSATISFIABLE");
    assert_eq!(gate(&cnf, &proof), "ok");
}

#[test]
fn too_many_paths_end_the_pass_early() {
    // a comb: c1 -> c2 -> ... -> c3000, and a tooth s_i -> c_i on every
    // member. Each tooth starts a path of its own down the rest of the
    // chain: 3,000 paths of 1,500 members on average, over links that are
    // made with a few thousand looks. The search for paths may cost three
    // times the work of making the links (plus 1M); past that the pass says
    // "dense" and reads nothing
    let fx = Fixture::new("num-dense");
    let cnf = fx.path("dense.cnf");
    let n = 3000i64;
    let mut cl: Vec<Vec<i64>> = Vec::new();
    for i in 1..=n {
        if i < n {
            cl.push(vec![-i, i + 1]);
        }
        cl.push(vec![-(n + i), i]);
    }
    write_cnf(&cnf, 2 * n, &cl);
    let on = run(&cnf, &["--structnum=1"], &[]);
    let r = report(&on.stdout);
    assert_eq!(r.get("status").map(String::as_str), Some("dense"), "{:?}", r);
    assert_eq!(r.get("added").map(String::as_str), Some("0"), "{:?}", r);
    let (links, paths): (u64, u64) = (r["wlinks"].parse().unwrap(), r["wpaths"].parse().unwrap());
    assert!(paths <= 3 * links + 1_000_000 + 10_000, "the search for paths spent {} with {} spent on the links", paths, links);
    let off = run(&cnf, &[], &[]);
    assert_eq!(on.status, off.status);
}

#[test]
fn tests_that_find_no_number_under_a_count_end_early() {
    // 30,000 chains of 20 variables each, nothing reading them: every chain
    // is a candidate and none is under a count. Testing them all costs far
    // more than the 50M the tests may spend before the first number under a
    // count; the pass ends its tests there and says "partial"
    let fx = Fixture::new("num-partial");
    let cnf = fx.path("partial.cnf");
    let (chains, len) = (30_000i64, 20i64);
    let mut cl: Vec<Vec<i64>> = Vec::new();
    for c in 0..chains {
        for k in 1..len {
            cl.push(vec![c * len + k, -(c * len + k + 1)]);
        }
    }
    write_cnf(&cnf, chains * len, &cl);
    let on = run(&cnf, &["--structnum=1"], &[]);
    let r = report(&on.stdout);
    assert_eq!(r.get("status").map(String::as_str), Some("partial"), "{:?}", r);
    assert_eq!(r.get("added").map(String::as_str), Some("0"), "{:?}", r);
    let read: u64 = r["wread"].parse().unwrap();
    assert!(read > 50_000_000 && read < 52_000_000, "the tests spent {}", read);
    let off = run(&cnf, &[], &[]);
    assert_eq!(on.status, off.status);
}

#[test]
fn no_table_is_built_without_work_for_it() {
    // the toy with its variables numbered from 1,000,001: seven variables
    // in use and tables of a million entries. With a limit that does not
    // cover the tables the pass does not start (it used to build them
    // first); with one that does, it reads the toy
    let fx = Fixture::new("num-sparse");
    let (cnf, proof) = (fx.path("sparse.cnf"), fx.path("sparse.proof"));
    let shift = 1_000_000i64;
    let cl: Vec<Vec<i64>> = toy().iter().map(|c| c.iter().map(|&l| if l > 0 { l + shift } else { l - shift }).collect()).collect();
    write_cnf(&cnf, 7 + shift, &cl);
    let (out, _err) = run_num(&cnf, &proof, &["--structnumticks=1"]);
    assert_eq!(report(&out).get("status").map(String::as_str), Some("no-budget"), "{:?}", report(&out));
    let (out, _err) = run_num(&cnf, &proof, &["--structnumticks=100"]);
    assert_eq!(report(&out).get("status").map(String::as_str), Some("no-budget"), "{:?}", report(&out));
    let (out, _err) = run_num(&cnf, &proof, &[]);
    let r = report(&out);
    assert_eq!(r.get("status").map(String::as_str), Some("read"), "{:?}", r);
    assert_eq!(derived(&proof), vec![vec![-5 - shift, 2 + shift], vec![5 + shift, -2 - shift], vec![6 + shift, -3 - shift], vec![-7 - shift, 1 + shift]]);
    assert!(r["work"].parse::<u64>().unwrap() > 6 * shift as u64, "the tables are paid: {:?}", r);
}

#[test]
fn a_limit_never_gives_a_reading_that_looks_whole_and_is_not() {
    // 50 copies of the toy on variables of their own: 50 numbers, 200
    // clauses. Whatever the limit, a run that says "read" has read them
    // all (a step skipped for lack of room used to leave an empty reading
    // that was reported as whole)
    let fx = Fixture::new("num-copies");
    let cnf = fx.path("copies.cnf");
    let copies = 50i64;
    let mut cl: Vec<Vec<i64>> = Vec::new();
    for k in 0..copies {
        cl.extend(toy().iter().map(|c| c.iter().map(|&l| if l > 0 { l + 7 * k } else { l - 7 * k }).collect::<Vec<i64>>()));
    }
    write_cnf(&cnf, 7 * copies, &cl);
    let free = run(&cnf, &["--structnum=1"], &[]);
    let whole = report(&free.stdout);
    assert_eq!(whole.get("status").map(String::as_str), Some("read"), "{:?}", whole);
    assert_eq!(whole.get("counted").map(String::as_str), Some("50"), "{:?}", whole);
    assert_eq!(whole.get("added").map(String::as_str), Some("200"), "{:?}", whole);
    let full: u64 = whole["work"].parse().unwrap();
    let mut cut = 0;
    for limit_k in 1..=(full / 1000 + 2) {
        let on = run(&cnf, &["--structnum=1", &format!("--structnumticks={}", limit_k)], &[]);
        let r = report(&on.stdout);
        let status = r.get("status").cloned().unwrap_or_default();
        if status == "read" {
            assert_eq!(r.get("counted").map(String::as_str), Some("50"), "limit {}K: {:?}", limit_k, r);
            assert_eq!(r.get("added").map(String::as_str), Some("200"), "limit {}K: {:?}", limit_k, r);
        } else {
            cut += 1;
            assert!(status == "stopped" || status == "no-budget", "limit {}K: {:?}", limit_k, r);
            let spent: u64 = r.get("work").map(|w| w.parse().unwrap()).unwrap_or(0);
            assert!(spent <= limit_k * 1000 + 5_000, "limit {}K: the pass ran to {}", limit_k, spent);
        }
        assert_eq!(on.status, free.status);
    }
    assert!(cut >= 5, "the limits of this test never cut the pass: {} of them did", cut);
}

#[test]
fn a_long_list_of_two_literal_clauses_is_paid_as_it_is_walked() {
    // the toy and 100,000 more clauses "h1 or not g1" (the same clause again
    // and again): setting g1 walks a list of 100,000. A limit that ends
    // inside the list holds there (the whole list used to be charged at
    // once, before any of it was walked)
    let fx = Fixture::new("num-star");
    let cnf = fx.path("star.cnf");
    let mut cl = toy();
    for _ in 0..100_000 {
        cl.push(vec![7, -1]);
    }
    write_cnf(&cnf, 7, &cl);
    let free = run(&cnf, &["--structnum=1"], &[]);
    let full: u64 = report(&free.stdout)["work"].parse().unwrap();
    for limit_k in [650u64, 800, 950, 1100, 1300] {
        if limit_k * 1000 >= full {
            continue;
        }
        let on = run(&cnf, &["--structnum=1", &format!("--structnumticks={}", limit_k)], &[]);
        let r = report(&on.stdout);
        let spent: u64 = r.get("work").map(|w| w.parse().unwrap()).unwrap_or(0);
        assert_ne!(r.get("status").map(String::as_str), Some("read"), "limit {}K of {}: {:?}", limit_k, full, r);
        assert!(spent <= limit_k * 1000 + 5_000, "limit {}K: the pass ran to {}", limit_k, spent);
    }
    assert!(full > 1_300_000, "the example no longer costs what this test is about: {}", full);
}
