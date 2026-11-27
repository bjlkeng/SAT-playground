// Memory room (src/room.rs; epic SAT-playground-qu3): a pass whose tables
// are optional asks whether there is room before it takes memory, and steps
// aside when there is not, instead of dying on a failed allocation.  The
// first such pass is the congruence pass.
//
// The formulas of these tests give the pass tables to build:
//   - a circuit of AND gates: 2,000 inputs, each gate over two earlier
//     variables, every tenth gate written twice (the pass merges the pair);
//   - the same with XOR gates;
//   - independent if-then-else gates;
//   - 4,095 AND gates of 256 inputs each: few gates, long lists.
// All are satisfiable, so a run that answers prints a model, and the model
// is checked against the clauses.
//
// What is checked:
//   - with room to spare the run is the run without the option, step for
//     step;
//   - told of a limit below what the process already holds, the pass does
//     not start (`gates=0`), and the answer holds;
//   - by default the pass asks for the room of its whole run before it
//     starts: it starts only at a higher limit than when it asks for its
//     first tables alone, and a quarter above that limit it finishes;
//   - with `--memwhole=0` it starts on the room of its first tables, and at
//     the smallest limit where it makes a gate it stops part way (`gates`
//     above 0 and below the number of gates), keeps going without the pass,
//     and the answer holds.  For the circuit, where the memory is in the
//     list of gates, and for the wide gates, where it is in the gates' own
//     small buffers.  Under a real address-space limit of that size (what
//     `ulimit -v` sets) the same happens and the process lives;
//   - the pass never costs a run its life: under every real limit from an
//     eighth above the least the run needs without the pass, megabyte by
//     megabyte up to where the pass has all the room it wants, the run with
//     the pass ends as it should too.  For the AND circuit, the same with
//     only AND gates looked for, the XOR circuit and the if-then-else gates;
//   - gates can be many more than clauses: one condition with 3,000 "then"
//     and 3,000 "else" values makes 9 million if-then-else gates out of
//     12,000 clauses.  There the question before the pass cannot know, the
//     gates' own questions stop it part way (by default too), and it stops
//     at once: no further pair is looked up.  And the steps after the
//     extraction ask too: with a second name for the condition, every one
//     of the gates is rewritten and indexed again, the hash table wants to
//     double, and under a real limit the run lives through that;
//   - the limit is the process's: the helper solvers of the control split
//     (eight case solvers on the committed fixture) keep under the run's
//     --memlimit too, though their own options do not carry it.  And the
//     limit is known before anything solves: the incremental mode
//     (`--inccnf=1`), which leaves the application early, keeps under it.

mod common;

use std::collections::BTreeMap;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use common::{assert_same_trajectory, run, solver_bin, Fixture};

const INPUTS: usize = 2_000;
const GATES: usize = 120_000;

/// A deterministic stream of numbers below a bound.
struct Draw(u64);

impl Draw {
    fn below(&mut self, bound: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % bound
    }
}

fn write_cnf(path: &Path, vars: usize, clauses: &[Vec<i64>]) {
    let mut text = format!("p cnf {} {}\n", vars, clauses.len());
    for c in clauses {
        for l in c {
            text.push_str(&l.to_string());
            text.push(' ');
        }
        text.push_str("0\n");
    }
    std::fs::write(path, text).unwrap();
}

/// A circuit of `gates` gates over two earlier variables each, a twin for
/// every tenth; AND gates, or XOR gates.  Returns its clauses.
fn circuit(path: &Path, gates: usize, xor: bool) -> Vec<Vec<i64>> {
    let mut draw = Draw(0x9E37_79B9_7F4A_7C15);
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    let mut vars = INPUTS;
    let gate = |clauses: &mut Vec<Vec<i64>>, vars: &mut usize, a: i64, b: i64| {
        *vars += 1;
        let g = *vars as i64;
        if xor {
            clauses.push(vec![-g, a, b]);
            clauses.push(vec![-g, -a, -b]);
            clauses.push(vec![g, -a, b]);
            clauses.push(vec![g, a, -b]);
        } else {
            clauses.push(vec![-g, a]);
            clauses.push(vec![-g, b]);
            clauses.push(vec![g, -a, -b]);
        }
    };
    for k in 0..gates {
        let a = 1 + draw.below(vars);
        let mut b = 1 + draw.below(vars);
        while b == a {
            b = 1 + draw.below(vars);
        }
        gate(&mut clauses, &mut vars, a as i64, b as i64);
        if k % 10 == 0 {
            gate(&mut clauses, &mut vars, a as i64, b as i64);
        }
    }
    write_cnf(path, vars, &clauses);
    clauses
}

fn and_circuit(path: &Path, gates: usize) -> Vec<Vec<i64>> {
    circuit(path, gates, false)
}

/// `gates` if-then-else gates, each over four variables of its own.
fn ite_gates(path: &Path, gates: usize) -> Vec<Vec<i64>> {
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for k in 0..gates as i64 {
        let (x, c, t, e) = (4 * k + 1, 4 * k + 2, 4 * k + 3, 4 * k + 4);
        clauses.push(vec![-c, -t, x]);
        clauses.push(vec![-c, t, -x]);
        clauses.push(vec![c, -e, x]);
        clauses.push(vec![c, e, -x]);
    }
    write_cnf(path, 4 * gates, &clauses);
    clauses
}

/// 4,095 AND gates of 256 inputs each, the inputs drawn from 4,096
/// variables, so the gate lists of the inputs are long.  Every variable true
/// is a model.
fn wide_gates(path: &Path) -> Vec<Vec<i64>> {
    let mut draw = Draw(0x2545_F491_4F6C_DD1D);
    let pool = 4096usize;
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for k in 0..4095usize {
        let g = (pool + 1 + k) as i64;
        let mut inputs: Vec<i64> = Vec::new();
        while inputs.len() < 256 {
            let x = (1 + draw.below(pool)) as i64;
            if !inputs.contains(&x) {
                inputs.push(x);
            }
        }
        let mut long = vec![g];
        for &x in &inputs {
            clauses.push(vec![-g, x]);
            long.push(-x);
        }
        clauses.push(long);
    }
    write_cnf(path, pool + 4095, &clauses);
    clauses
}

struct Outcome {
    stdout: String,
    stderr: String,
    /// The exit code; -1 when a signal ended the process (an abort).
    code: i32,
}

/// Run with a model printed (`-n` left out), the lucky check off (it would
/// answer before the pass runs), and an optional real address-space limit.
fn solve(cnf: &Path, options: &[&str], address_space: Option<u64>) -> Outcome {
    let mut cmd = Command::new(solver_bin());
    cmd.args(common::STOCK_PATH).arg("--lucky=0").args(options).arg(cnf);
    for key in common::SOLVER_ENV_VARS {
        cmd.env_remove(key);
    }
    if let Some(bytes) = address_space {
        unsafe {
            cmd.pre_exec(move || {
                let r = libc::rlimit {
                    rlim_cur: bytes as libc::rlim_t,
                    rlim_max: bytes as libc::rlim_t,
                };
                if libc::setrlimit(libc::RLIMIT_AS, &r) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let out = cmd.output().expect("run sat-solver");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code().unwrap_or(-1),
    }
}

/// The run answered "satisfiable" and its model satisfies every clause.
fn assert_model(o: &Outcome, clauses: &[Vec<i64>], what: &str) {
    assert_eq!(o.code, 10, "{}: exit code; stderr:\n{}", what, o.stderr);
    assert!(o.stdout.lines().any(|l| l == "s SATISFIABLE"), "{}: no SATISFIABLE line", what);
    let mut truth: BTreeMap<i64, bool> = BTreeMap::new();
    for l in o.stdout.lines().filter_map(|l| l.strip_prefix("v ")) {
        for t in l.split_whitespace() {
            let x: i64 = t.parse().unwrap();
            if x != 0 {
                truth.insert(x.abs(), x > 0);
            }
        }
    }
    for c in clauses {
        assert!(
            c.iter().any(|&y| truth.get(&y.abs()).copied() == Some(y > 0)),
            "{}: the model leaves the clause {:?} false",
            what,
            c
        );
    }
}

/// The fields of the `c room ...` exit line, or None when there is none.
/// The line has a field `late` only when a gate was asked for after the
/// pass had stopped, that is when a loop did not end at once: never, in any
/// run of these tests.
fn room_line(stdout: &str) -> Option<BTreeMap<String, String>> {
    let line = stdout.lines().find_map(|l| l.strip_prefix("c room "))?;
    let fields: BTreeMap<String, String> = line
        .split_whitespace()
        .filter_map(|t| t.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    assert!(!fields.contains_key("late"), "a loop went on after the room was gone: c room {}", line);
    Some(fields)
}

fn gates_of(line: &BTreeMap<String, String>) -> u64 {
    line["gates"].parse().unwrap()
}

/// The `-s` counter of that name.
fn counter(stdout: &str, name: &str) -> u64 {
    let head = format!("c {}:", name);
    stdout
        .lines()
        .find(|l| l.starts_with(&head))
        .and_then(|l| l.split_whitespace().nth(2))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("no counter {}", name))
}

/// The process has no address-space limit of its own, so the option is the
/// only limit a run sees.  Under one (a developer's `ulimit -v`) the sizes
/// these tests look for would not be theirs to set.
fn no_limit_of_our_own() -> bool {
    let mut r = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut r) == 0 && r.rlim_cur == libc::RLIM_INFINITY }
}

/// The smallest limit told (megabytes) at which the pass makes a gate, by
/// bisection, and that run.
fn smallest_limit_with_a_gate(cnf: &Path, options: &[&str], what: &str) -> (u64, Outcome) {
    let makes_a_gate = |mb: u64| -> (bool, Outcome) {
        let limit = format!("--memlimit={}", mb);
        let mut all: Vec<&str> = options.to_vec();
        all.extend_from_slice(&["-s", &limit]);
        let o = solve(cnf, &all, None);
        let on = match room_line(&o.stdout) {
            None => true,
            Some(line) => gates_of(&line) > 0,
        };
        (on, o)
    };
    let (mut low, mut high) = (8u64, 4096u64);
    assert!(!makes_a_gate(low).0 && makes_a_gate(high).0, "{}: the two ends of the search", what);
    while high - low > 1 {
        let mid = (low + high) / 2;
        if makes_a_gate(mid).0 {
            high = mid;
        } else {
            low = mid;
        }
    }
    let (_, o) = makes_a_gate(high);
    (high, o)
}

#[test]
fn with_room_to_spare_the_run_is_the_old_one() {
    let fx = Fixture::new("room-spare");
    let cnf = fx.path("and.cnf");
    and_circuit(&cnf, GATES);
    let plain = run(&cnf, &["--lucky=0"], &[]);
    let roomy = run(&cnf, &["--lucky=0", "--memlimit=1000000"], &[]);
    let frugal = run(&cnf, &["--lucky=0", "--memlimit=1000000", "--memwhole=0"], &[]);
    assert_eq!(plain.status, "SATISFIABLE");
    assert!(plain.stats.get("congruent_gates").copied().unwrap_or(0) > 100_000, "the pass builds its gates: {:?}", plain.stats.get("congruent_gates"));
    assert!(plain.stats.get("congruent").copied().unwrap_or(0) >= 12_000, "and merges the doubled ones: {:?}", plain.stats.get("congruent"));
    assert_same_trajectory(&plain, &roomy, "a limit far away");
    assert_same_trajectory(&plain, &frugal, "a limit far away, the room asked for as the tables grow");
    assert!(room_line(&plain.stdout).is_none() && room_line(&roomy.stdout).is_none(), "no exit line when no pass stepped aside");
}

#[test]
fn without_room_the_pass_does_not_start() {
    let fx = Fixture::new("room-none");
    let cnf = fx.path("and.cnf");
    let clauses = and_circuit(&cnf, GATES);
    let o = solve(&cnf, &["-s", "--memlimit=8"], None);
    assert_model(&o, &clauses, "a limit of 8 MB");
    let line = room_line(&o.stdout).expect("the exit line");
    assert_eq!(line["congruence"], "off");
    assert_eq!(gates_of(&line), 0, "the pass never started: {:?}", line);
    assert_eq!(line["limit_mb"], "8");
    assert_eq!(counter(&o.stdout, "closures"), 0, "no closure was counted");
}

#[test]
fn by_default_the_pass_asks_for_its_whole_run() {
    if !no_limit_of_our_own() {
        eprintln!("skipped: this process runs under an address-space limit");
        return;
    }
    // The question before the pass is for its whole run, not for its first
    // tables: it starts only at a higher limit than with `--memwhole=0`.
    // The gates come on top of what was promised then (they may not use
    // it), so at that limit the pass stops among its gates; half as much
    // again and it has all its room: every gate is made and nothing is
    // reported.
    let fx = Fixture::new("room-whole");
    let cnf = fx.path("and.cnf");
    let clauses = and_circuit(&cnf, GATES);
    let (whole, o) = smallest_limit_with_a_gate(&cnf, &[], "the circuit");
    assert_model(&o, &clauses, "the smallest limit at which the pass starts");
    let (part, _) = smallest_limit_with_a_gate(&cnf, &["--memwhole=0"], "the circuit, asked as it grows");
    assert!(whole > part, "asked for its whole run the pass starts at {} MB, for its first tables at {} MB", whole, part);
    let line = room_line(&o.stdout).expect("at the smallest limit the gates do not all fit");
    assert!(gates_of(&line) < GATES as u64, "{:?}", line);
    let roomy = solve(&cnf, &["-s", &format!("--memlimit={}", whole + whole / 2)], None);
    assert_model(&roomy, &clauses, "half as much again");
    assert!(room_line(&roomy.stdout).is_none(), "half above {} MB the pass ran to its end: {:?}", whole, room_line(&roomy.stdout));
    assert!(counter(&roomy.stdout, "congruent_gates") > 100_000);
    // one megabyte less than `whole` and it does not start at all
    let below = solve(&cnf, &["-s", &format!("--memlimit={}", whole - 1)], None);
    assert_model(&below, &clauses, "one megabyte less");
    assert_eq!(gates_of(&room_line(&below.stdout).expect("the exit line")), 0);
}

/// With `--memwhole=0`: at the smallest limit at which the pass makes a
/// gate its tables cannot all fit, so it must stop part way, and the answer
/// must hold.  Returns that limit.
fn stops_part_way(cnf: &Path, clauses: &[Vec<i64>], total: u64, options: &[&str], what: &str) -> u64 {
    let mut all: Vec<&str> = options.to_vec();
    all.push("--memwhole=0");
    let (limit, o) = smallest_limit_with_a_gate(cnf, &all, what);
    assert_model(&o, clauses, &format!("{}: the smallest limit at which the pass makes a gate", what));
    let line = room_line(&o.stdout).unwrap_or_else(|| panic!("{}: at {} MB the tables cannot all fit", what, limit));
    let gates = gates_of(&line);
    assert!(gates > 0 && gates < total, "{}: it stopped part way: {:?}", what, line);
    assert_eq!(line["limit_mb"], limit.to_string());
    // the pass ran once and is off afterwards
    assert_eq!(counter(&o.stdout, "closures"), 1, "{}", what);
    limit
}

#[test]
fn asked_as_it_grows_the_pass_stops_part_way() {
    if !no_limit_of_our_own() {
        eprintln!("skipped: this process runs under an address-space limit");
        return;
    }
    let fx = Fixture::new("room-part");
    let cnf = fx.path("and.cnf");
    let clauses = and_circuit(&cnf, GATES);
    let limit = stops_part_way(&cnf, &clauses, GATES as u64, &[], "the circuit");

    // The same under a real address-space limit, which is what the harness
    // sets: the process lives, and the answer holds.  The pass frees its
    // tables when it stops, so the rest of the run has their room.
    let real = solve(&cnf, &["-s", "--memwhole=0"], Some(limit << 20));
    assert_model(&real, &clauses, "a real address-space limit");
    let line = room_line(&real.stdout).expect("the exit line under a real limit");
    assert!(gates_of(&line) < GATES as u64, "{:?}", line);
    assert_eq!(line["limit_mb"], limit.to_string());
}

#[test]
fn few_wide_gates_stop_part_way_too() {
    if !no_limit_of_our_own() {
        eprintln!("skipped: this process runs under an address-space limit");
        return;
    }
    // The list of 4,095 gates is 200 KB: what does not fit here is the
    // gates' own buffers, a kilobyte of inputs a gate and as much again in
    // the gate lists of its inputs.  (`--factor=0`: that pass comes after
    // this one and takes seven seconds on a million binary clauses.)
    let fx = Fixture::new("room-wide");
    let cnf = fx.path("wide.cnf");
    let clauses = wide_gates(&cnf);
    stops_part_way(&cnf, &clauses, 4095, &["--factor=0"], "wide gates");
}

/// Under every real address-space limit from an eighth above the least the
/// run needs without the pass, megabyte by megabyte up to the first limit
/// where the pass has all the room it wants and two more: the run with the
/// pass ends as the run without it does (an answer, or the conflict limit),
/// and a model holds.  Returns how many of them stepped aside.
fn sweep(cnf: &Path, clauses: &[Vec<i64>], options: &[&str], what: &str) -> usize {
    sweep_by(cnf, clauses, options, 1, what)
}

/// The same, `step` megabytes at a time.
fn sweep_by(cnf: &Path, clauses: &[Vec<i64>], options: &[&str], step: u64, what: &str) -> usize {
    let ended = |o: &Outcome| o.code == 10 || o.code == 0;
    let mut without: Vec<&str> = options.to_vec();
    without.push("--congruence=0");
    let lives = |mb: u64| ended(&solve(cnf, &without, Some(mb << 20)));
    let (mut low, mut high) = (4u64, 1024u64);
    assert!(!lives(low) && lives(high), "{}: the two ends of the search", what);
    while high - low > 1 {
        let mid = (low + high) / 2;
        if lives(mid) {
            high = mid;
        } else {
            low = mid;
        }
    }
    let least = high;
    let mut with: Vec<&str> = options.to_vec();
    with.push("-s");
    let mut aside = 0usize;
    let mut roomy = 0usize;
    let mut mb = least + least / 8 + 1;
    while roomy < 3 {
        let o = solve(cnf, &with, Some(mb << 20));
        let here = format!("{}: a real limit of {} MB (the run without the pass needs {})", what, mb, least);
        assert!(ended(&o), "{}: exit code {}; stderr:\n{}", here, o.code, o.stderr);
        if o.code == 10 {
            assert_model(&o, clauses, &here);
        }
        match room_line(&o.stdout) {
            Some(_) => {
                aside += 1;
                roomy = 0;
            }
            None => roomy += 1,
        }
        mb += step;
        assert!(mb < least * 64, "{}: the pass never got its room", what);
    }
    aside
}

#[test]
fn the_pass_never_costs_a_run_its_life() {
    if !no_limit_of_our_own() {
        eprintln!("skipped: this process runs under an address-space limit");
        return;
    }
    let fx = Fixture::new("room-sweep");
    let cnf = fx.path("and.cnf");
    let clauses = and_circuit(&cnf, 30_000);
    let aside = sweep(&cnf, &clauses, &[], "the circuit");
    assert!(aside > 0, "the circuit: no limit of the sweep made the pass step aside");
    let aside = sweep(&cnf, &clauses, &["--congruencexors=0", "--congruenceites=0"], "the circuit, AND gates only");
    assert!(aside > 0, "AND gates only: no limit of the sweep made the pass step aside");
    // The XOR circuit is hard for the search, so its runs end at a conflict
    // limit: what counts is that they end by themselves.
    let xor = fx.path("xor.cnf");
    let clauses = circuit(&xor, 30_000, true);
    let aside = sweep(&xor, &clauses, &["--conflicts=1000"], "the XOR circuit");
    assert!(aside > 0, "the XOR circuit: no limit of the sweep made the pass step aside");
    let ite = fx.path("ite.cnf");
    let clauses = ite_gates(&ite, 30_000);
    let aside = sweep(&ite, &clauses, &[], "if-then-else gates");
    assert!(aside > 0, "if-then-else gates: no limit of the sweep made the pass step aside");
}

#[test]
fn helper_solvers_keep_under_the_limit_too() {
    // The control split answers this fixture with eight case solvers, each a
    // solver of its own with default options.  Told of 1 MB, every one of
    // their congruence passes must find no room; the answer stays.
    let cnf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/control_rotmiter.cnf");
    let plain = common::run_shipped(&cnf, &[], &[]);
    let told = common::run_shipped(&cnf, &["--memlimit=1"], &[]);
    assert_eq!(plain.status, "UNSATISFIABLE");
    assert_eq!(told.status, "UNSATISFIABLE");
    assert!(plain.stdout.contains("c control split: unsat (8 cases"), "the split runs its eight cases");
    assert!(room_line(&plain.stdout).is_none(), "no line without a refusal");
    let line = room_line(&told.stdout).expect("the exit line");
    let refused: u64 = line["refused"].parse().unwrap();
    assert!(refused >= 8, "each case solver's pass was refused: {:?}", line);
    assert_eq!(line["congruence"], "on", "the run's own pass never asked: {:?}", line);
}

#[test]
fn the_incremental_mode_keeps_under_the_limit() {
    // Two gates with the same inputs and one question with no assumption:
    // the pass runs in that solve.  Told of 1 MB it must not.
    let fx = Fixture::new("room-icnf");
    let icnf = fx.path("twins.icnf");
    std::fs::write(&icnf, "p inccnf\n-3 1 0\n-3 2 0\n3 -1 -2 0\n-4 1 0\n-4 2 0\n4 -1 -2 0\na 0\n").unwrap();
    let ask = |options: &[&str]| -> String {
        let mut cmd = Command::new(solver_bin());
        cmd.args(["--inccnf=1", "--lucky=0", "-s"]).args(options).arg(&icnf);
        for key in common::SOLVER_ENV_VARS {
            cmd.env_remove(key);
        }
        let out = cmd.output().expect("run sat-solver");
        assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let plain = ask(&[]);
    assert_eq!(counter(&plain, "closures"), 1, "the pass runs in the question's solve");
    assert_eq!(counter(&plain, "congruent_gates"), 1);
    assert!(room_line(&plain).is_none());
    let told = ask(&["--memlimit=1"]);
    assert_eq!(counter(&told, "closures"), 0, "told of 1 MB the pass does not start");
    let line = room_line(&told).expect("the exit line");
    assert_eq!(line["congruence"], "off");
    assert_eq!(gates_of(&line), 0);
    assert!(told.contains("c assume solves=1 sat=1"), "the question is answered all the same");
}

/// Two variables x and y and one condition c: when c holds, x and y each
/// equal every one of `n` variables; when it does not, every one of `n`
/// others.  Every pair of a "then" and an "else" is an if-then-else gate
/// for x, n times n gates from 8n clauses, and the same pairs for y match
/// them (so x and y are merged).
fn one_condition(path: &Path, n: usize) -> Vec<Vec<i64>> {
    let (x, c, y) = (1i64, 2i64, 3i64);
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    for i in 0..n as i64 {
        let t = 4 + i;
        let e = 4 + n as i64 + i;
        for lhs in [x, y] {
            clauses.push(vec![-c, -lhs, t]);
            clauses.push(vec![-c, lhs, -t]);
            clauses.push(vec![c, -lhs, e]);
            clauses.push(vec![c, lhs, -e]);
        }
    }
    write_cnf(path, 3 + 2 * n, &clauses);
    clauses
}

#[test]
fn many_gates_from_few_clauses_stop_the_pass_at_once() {
    if !no_limit_of_our_own() {
        eprintln!("skipped: this process runs under an address-space limit");
        return;
    }
    let fx = Fixture::new("room-pairs");
    let cnf = fx.path("pairs.cnf");
    let n = 1_000usize;
    let clauses = one_condition(&cnf, n);
    let pairs = (n * n) as u64;
    // with all the room, every pair becomes a gate for x, and the pairs of
    // y match them
    let all = solve(&cnf, &["-s"], None);
    assert_model(&all, &clauses, "no limit");
    assert_eq!(counter(&all.stdout, "congruent_gates_ites"), pairs);
    assert!(counter(&all.stdout, "congruent_matched_ites") > 0, "the pairs of y match the gates of x");
    // At the smallest limit where the pass makes a gate: the formula is
    // small, so the pass starts (by default too), and its million gates do
    // not fit.  It stops among the pairs of x.
    let (limit, o) = smallest_limit_with_a_gate(&cnf, &[], "one condition");
    assert_model(&o, &clauses, "the smallest limit at which the pass makes a gate");
    let line = room_line(&o.stdout).unwrap_or_else(|| panic!("at {} MB a million gates cannot fit", limit));
    let gates = gates_of(&line);
    assert!(gates > 0 && gates < pairs, "it stopped part way: {:?}", line);
    assert_eq!(counter(&o.stdout, "congruent_gates_ites"), gates);
    // And at once.  Had the loop gone on over the remaining pairs, those of
    // y would have found the gates of x and matched.
    assert_eq!(counter(&o.stdout, "congruent_matched_ites"), 0, "the pairs went on after the room was gone");
}

/// Exactly one of `n` variables: one clause of all of them and a binary
/// clause for every pair.  The long clause is one base clause with n
/// candidate gates (x_i is the AND of the others negated), each of n - 1
/// inputs.
fn exactly_one(path: &Path, n: usize) -> Vec<Vec<i64>> {
    let mut clauses: Vec<Vec<i64>> = vec![(1..=n as i64).collect()];
    for i in 1..=n as i64 {
        for j in i + 1..=n as i64 {
            clauses.push(vec![-i, -j]);
        }
    }
    write_cnf(path, n, &clauses);
    clauses
}

#[test]
fn the_candidates_of_one_wide_clause_stop_at_once_too() {
    if !no_limit_of_our_own() {
        eprintln!("skipped: this process runs under an address-space limit");
        return;
    }
    // (`--factor=0`: that pass comes after this one and is slow on 700,000
    // binary clauses.)
    let stock = ["--factor=0"];
    let fx = Fixture::new("room-one");
    let cnf = fx.path("one.cnf");
    let n = 1_200usize;
    let clauses = exactly_one(&cnf, n);
    let plain = solve(&cnf, &["--factor=0", "-s"], None);
    assert_model(&plain, &clauses, "no limit");
    assert_eq!(counter(&plain.stdout, "congruent_gates_ands"), n as u64, "every candidate of the clause is a gate");
    // Asked as it grows (`--memwhole=0`), at the smallest limit where the
    // pass makes a gate: the n gates of n - 1 inputs do not fit (their
    // inputs and the gate lists of the inputs are 11 MB), so it stops among
    // the candidates of the one clause.  `room_line` fails the test if
    // another candidate was tried after that.
    stops_part_way(&cnf, &clauses, n as u64, &stock, "exactly one");
}

/// Two conditions that have nothing to do with each other.  Each has one
/// variable that equals every one of n others when the condition holds and
/// every one of n others when it does not: `small` values for the first
/// condition, `large` for the second.
fn two_conditions(path: &Path, small: usize, large: usize) -> Vec<Vec<i64>> {
    let mut clauses: Vec<Vec<i64>> = Vec::new();
    let mut next = 1i64;
    for n in [small, large] {
        let (x, c) = (next, next + 1);
        next += 2;
        for _ in 0..n {
            let (t, e) = (next, next + 1);
            next += 2;
            clauses.push(vec![-c, -x, t]);
            clauses.push(vec![-c, x, -t]);
            clauses.push(vec![c, -x, e]);
            clauses.push(vec![c, x, -e]);
        }
    }
    write_cnf(path, (next - 1) as usize, &clauses);
    clauses
}

#[test]
fn the_gates_leave_the_room_of_the_later_steps() {
    if !no_limit_of_our_own() {
        eprintln!("skipped: this process runs under an address-space limit");
        return;
    }
    // The first condition makes 1,024 times 1,024 gates.  The second has
    // 150,000 values: megabytes of pairs to copy and sort before its first
    // gate, and 22 billion gates that fit under no limit of this test, so
    // the pass always steps aside.  The room for the second condition's
    // lists and for the watches' return was promised before the pass
    // started; the gates of the first must leave it alone, and the watches
    // must not come back behind the holes of the steps before.  Before
    // that, the run died at limits of 242 and 243 MB (in the second
    // condition's lists), and then at 169 and 170 MB (the watch store
    // doubling when the watches came back).
    let fx = Fixture::new("room-two");
    let cnf = fx.path("two.cnf");
    let clauses = two_conditions(&cnf, 1024, 150_000);
    let ended = |o: &Outcome| o.code == 10;
    let lives = |mb: u64| ended(&solve(&cnf, &["--congruence=0"], Some(mb << 20)));
    let (mut low, mut high) = (4u64, 1024u64);
    assert!(!lives(low) && lives(high), "the two ends of the search");
    while high - low > 1 {
        let mid = (low + high) / 2;
        if lives(mid) {
            high = mid;
        } else {
            low = mid;
        }
    }
    let least = high;
    let start = least + least / 8 + 1;
    let mut aside = 0usize;
    for mb in start..start + 140 {
        let o = solve(&cnf, &["-s"], Some(mb << 20));
        let here = format!("a real limit of {} MB (the run without the pass needs {})", mb, least);
        assert!(ended(&o), "{}: exit code {}; stderr:\n{}", here, o.code, o.stderr);
        assert_model(&o, &clauses, &here);
        if room_line(&o.stdout).is_some() {
            aside += 1;
        }
    }
    assert_eq!(aside, 140, "the second condition's gates fit nowhere, so the pass steps aside at every limit");
}

/// One variable x and a condition c with a second, smaller name a (a = c):
/// when c holds x equals each of `n` variables, when it does not each of
/// `n` others.  n times n gates; then the pass merges c into a and rewrites
/// every one of them.
fn one_condition_with_two_names(path: &Path, n: usize) -> Vec<Vec<i64>> {
    let (a, x, c) = (1i64, 2i64, 3i64);
    let mut clauses: Vec<Vec<i64>> = vec![vec![-a, c], vec![a, -c]];
    for i in 0..n as i64 {
        let t = 4 + i;
        let e = 4 + n as i64 + i;
        clauses.push(vec![-c, -x, t]);
        clauses.push(vec![-c, x, -t]);
        clauses.push(vec![c, -x, e]);
        clauses.push(vec![c, x, -e]);
    }
    write_cnf(path, 3 + 2 * n, &clauses);
    clauses
}

#[test]
fn the_steps_after_the_extraction_ask_too() {
    if !no_limit_of_our_own() {
        eprintln!("skipped: this process runs under an address-space limit");
        return;
    }
    // 1,024 times 1,024 gates fit at some limits and their rewriting does
    // not: there the hash table, full of the places of the gates taken out
    // to be indexed again, asks for twice its size.
    let fx = Fixture::new("room-later");
    let cnf = fx.path("names.cnf");
    let clauses = one_condition_with_two_names(&cnf, 1024);
    let aside = sweep_by(&cnf, &clauses, &[], 4, "one condition with two names");
    assert!(aside > 0, "no limit of the sweep made the pass step aside");
}
