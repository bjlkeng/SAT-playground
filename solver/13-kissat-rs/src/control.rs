//! Not in kissat (bead SAT-playground-d96.8.5): control words.
//!
//! A control word is a small set of variables whose values decide most of
//! a formula's switches (solver README, "Control words"): on the oisc cells
//! one input word as wide as the registers steers three quarters of the
//! muxes, and with its bits fixed the rest falls to propagation and the
//! usual clean-up.  The outside split (`tools/control_census.py split`)
//! decides all three oisc cells the search never decides.
//!
//! This module is the first use inside the solver, the decision order:
//! with `ctlfirst` on, `next_decision_variable` takes the first free,
//! active control variable before kissat's own choice, so the search
//! enumerates the control word's values at the top of its trail.  Answers
//! and proofs are untouched (a decision needs no proof).
//!
//! The second use is the split (`ctlsplit`, below): the control word's
//! values as cases, each solved by a sub-solver before the search.
//!
//! For the measurement step the control set comes from the environment:
//! `SAT_CTL_VARS` is a comma-separated list of DIMACS variables, decided (or
//! split on) in that order.  With both options off nothing here runs beyond
//! the branch's one test, and the environment is never read.

use crate::internal::Solver;
use crate::literal::lit;

pub const INVALID_IDX: u32 = u32::MAX;

#[derive(Default)]
pub struct Control {
    /// Control decisions are on (`ctlfirst` and a non-empty set).
    pub on: bool,
    /// DIMACS variables, in decision order (mapped through `import_` at each
    /// decision).
    pub vars: Vec<u32>,
    /// The DIMACS variables asked for, and how many were found active.
    pub asked: usize,
    pub decisions: u64,
    /// The split (`ctlsplit`): the formula kept for its sub-solvers, a model
    /// one of them found, and its counts for the exit line.
    pub formula: Vec<i32>,
    /// The parse copies the formula here itself (the structure stage is off).
    pub keeping: bool,
    pub model: Option<Vec<bool>>,
    pub split_status: String,
    pub split_cases: u64,
    pub split_refuted: u64,
    pub split_unknown: u64,
    pub split_sat: u64,
    pub split_work: u64,
    pub split_lemmas: u64,
    /// The reading (SAT_CTL_VARS unset): switches, cover and its work.
    pub read_muxes: u64,
    pub read_cover: f64,
    pub read_work: u64,
    /// The work clock when the split began (structure passes, the walk).
    pub split_pre: u64,
}

/// Read the control set after the formula is in the solver.
pub fn install(solver: &mut Solver) {
    if solver.options.ctlfirst == 0 {
        return;
    }
    let Ok(text) = std::env::var("SAT_CTL_VARS") else {
        crate::print::message(solver, "control: ctlfirst on but SAT_CTL_VARS unset, nothing to decide");
        return;
    };
    // DIMACS variables, mapped to internal ones at each decision: compaction
    // renumbers the internal variables (and keeps `import_` current)
    let mut vars: Vec<u32> = Vec::new();
    let mut asked = 0usize;
    for tok in text.split(',') {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        asked += 1;
        let Ok(e) = tok.parse::<i64>() else { continue };
        if e <= 0 || e as usize >= solver.import_.len() {
            continue;
        }
        let imp = &solver.import_[e as usize];
        if !imp.imported || imp.eliminated {
            continue;
        }
        if !vars.contains(&(e as u32)) {
            vars.push(e as u32);
        }
    }
    solver.control.asked = asked;
    solver.control.on = !vars.is_empty();
    solver.control.vars = vars;
    let n = solver.control.vars.len();
    crate::print::message(solver, format!("control: deciding {} of {} control variables first", n, asked));
}

/// The first control variable that is free and active, or INVALID_IDX.
pub fn next(solver: &mut Solver) -> u32 {
    for k in 0..solver.control.vars.len() {
        let e = solver.control.vars[k] as usize;
        if e >= solver.import_.len() {
            continue;
        }
        let imp = &solver.import_[e];
        if !imp.imported || imp.eliminated {
            continue;
        }
        let idx = imp.lit >> 1;
        if idx >= solver.vars {
            continue;
        }
        if solver.values[lit(idx) as usize] == 0 && solver.flags[idx as usize].active() {
            solver.control.decisions += 1;
            return idx;
        }
    }
    INVALID_IDX
}

/// The exit line; nothing when both options are off.
pub fn report(solver: &Solver) {
    let c = &solver.control;
    if solver.options.ctlfirst != 0 {
        println!("c control ctlfirst=1 asked={} vars={} decisions={}", c.asked, c.vars.len(), c.decisions);
    }
    if solver.options.ctlsplit != 0 {
        println!(
            "c control ctlsplit={} status={} cases={} refuted={} unknown={} sat={} work={} lemmas={} pre={} read={}",
            solver.options.ctlsplit,
            if c.split_status.is_empty() { "off" } else { &c.split_status },
            c.split_cases, c.split_refuted, c.split_unknown, c.split_sat, c.split_work, c.split_lemmas,
            c.split_pre, c.read_work
        );
    }
}

// ---------------------------------------------------------------------------
// The split: the control word's values as cases, each in a sub-solver
// ---------------------------------------------------------------------------
//
// With `ctlsplit` on (1), before the search: the first `ctlsplitbits` control
// variables, every value, are cases; each case is the formula plus one unit
// per bit, solved by a sub-solver within `ctlsplitcase` work.  A case that
// ends without an answer is split again on the next bits, down to
// `ctlsplitlevels` levels.  The whole split stops within `ctlsplitticks`.
// With `ctlsplit=2` (the effect test's probe) a case left without an answer
// does not end the split: every case runs, and the exit line counts them.
//
// Proof.  The sub-solver's derived clauses L are RUP from the formula, the
// case's units u and the clauses before them; so (L or not-u) is RUP from
// the formula and the clauses before it (under the negation of not-u the
// units hold, and every earlier (L' or not-u) is L' again).  Each is
// written to the DRAT proof with not-u appended, then the clause not-u
// itself (the case's empty clause), and the case's other lines are deleted.
// When every case of a node is refuted, its clauses are resolved pairwise
// over the node's bits, each resolvent RUP from its two parents, down to
// the node's own not-u, and at the root to the empty clause.  The
// sub-solver runs without `factor` (the one kissat technique that adds
// variables: a fresh variable of one case could clash with another's) and
// without the structure passes and engines, whose proofs are VeriPB.
//
// A satisfiable case's model is a model of the formula: it is checked
// against every clause of the kept formula and printed as it is.  When the
// split ends undecided, the refuted cases' not-u clauses go to the solver
// (they are in the proof already) and the search starts as usual.

struct Node {
    units: Vec<i32>,
    level: usize,
    /// Position in the control list after this node's own bits.
    pos: usize,
    /// The bits its children were split on (empty: not split).
    bits: Vec<i32>,
    children: Vec<usize>,
    /// 0 open, 1 refuted, 2 split, 3 no answer.
    status: u8,
}

/// Keep the formula the search will see (derived clauses, then the input),
/// for the split's sub-solvers.  Nothing with `ctlsplit` off.
pub fn keep(solver: &mut Solver, derived: &[Vec<i32>], lits: &[i32]) {
    if solver.options.ctlsplit == 0 {
        return;
    }
    let mut f: Vec<i32> = Vec::with_capacity(lits.len() + derived.iter().map(|c| c.len() + 1).sum::<usize>());
    for c in derived {
        f.extend_from_slice(c);
        f.push(0);
    }
    f.extend_from_slice(lits);
    solver.control.formula = f;
}

fn read_vars(max_var: i32) -> Vec<i32> {
    let mut out: Vec<i32> = Vec::new();
    if let Ok(text) = std::env::var("SAT_CTL_VARS") {
        for tok in text.split(',') {
            if let Ok(v) = tok.trim().parse::<i32>() {
                if v > 0 && v <= max_var && !out.contains(&v) {
                    out.push(v);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The reading: the control word from the formula (SAT_CTL_VARS unset)
// ---------------------------------------------------------------------------
//
// After tools/control_census.py's first reader, the shapes that paid on the
// 2025 cells (solver README, "Control words"): AND, XOR and if-then-else
// gates written as two-way Tseitin clauses, a gate's output numbered after
// its inputs.  A switch is an if-then-else gate, or the AND form
// -AND(-AND(s, a), -AND(-s, b)).  Each switch's select is followed back to
// the inputs (a memo shared by all selects, a support of more than 200
// inputs counting as wide); an input's weight is the number of switches it
// steers, and the control set is the fewest heaviest inputs that steer 90%
// of what the 16 heaviest steer, decided low index first.  The reading
// charges a tick a clause and a step and stops at `budget`.

pub struct Reading {
    pub muxes: u64,
    /// AND, XOR and if-then-else gates read in all.
    pub gates: u64,
    /// The share of switches the control set steers.
    pub cover: f64,
    pub vars: Vec<i32>,
}

const CAP: usize = 200;

/// The control set read from the flat formula, or None (over budget, or no
/// switch at all).  `work` is the counter the reading adds to (the solver's
/// tick counter) and `budget` the absolute value it may reach.
/// Pay for a step of work before doing it: false (and nothing charged) when
/// the budget does not cover it or the solver was told to stop (`--time`,
/// a signal).
fn pay(work: &mut u64, cost: u64, budget: u64, stop: &std::sync::atomic::AtomicBool) -> bool {
    if work.saturating_add(cost) > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    *work += cost;
    true
}

fn read_control(formula: &[i32], max_var: i32, budget: u64, work: &mut u64, stop: &std::sync::atomic::AtomicBool) -> Option<Reading> {
    let n = max_var as usize + 1;
    // the tables per variable and the scans over them are paid for first
    // (a sparse numbering makes them far bigger than the formula)
    if !pay(work, 4 * n as u64, budget, stop) {
        return None; // a read that cannot pay ends with nothing
    }
    // binary clauses as sorted encoded pairs; ternary clauses with their sorted
    // variable triple
    let enc = |a: i32, b: i32| -> u64 { (((a as i64 + (1 << 31)) as u64) << 32) | ((b as i64 + (1 << 31)) as u64) };
    let mut pairs: Vec<u64> = Vec::new();
    let mut tri: Vec<([u32; 3], [i32; 3])> = Vec::new();
    let mut start = 0usize;
    for (i, &l) in formula.iter().enumerate() {
        if l != 0 {
            continue;
        }
        let c = &formula[start..i];
        start = i + 1;
        if !pay(work, c.len() as u64 + 1, budget, stop) {
            return None;
        }
        if c.len() == 2 {
            pairs.push(enc(c[0], c[1]));
            pairs.push(enc(c[1], c[0]));
        } else if c.len() == 3 {
            let mut vs = [c[0].unsigned_abs(), c[1].unsigned_abs(), c[2].unsigned_abs()];
            vs.sort_unstable();
            if vs[0] != vs[1] && vs[1] != vs[2] {
                tri.push((vs, [c[0], c[1], c[2]]));
            }
        }
    }
    // the sorts and the gate loop below, paid before they run
    if !pay(work, 2 * (pairs.len() + tri.len()) as u64, budget, stop) {
        return None;
    }
    pairs.sort_unstable();
    pairs.dedup();
    tri.sort_unstable();
    tri.dedup();
    let has = |a: i32, b: i32| pairs.binary_search(&enc(a, b)).is_ok();
    // gates: kind 1 and, 2 xor, 3 ite; inputs as literals
    let mut kind = vec![0u8; n];
    let mut ins: Vec<[i32; 3]> = vec![[0; 3]; n];
    for &(_vs, c) in &tri {
        for k in 0..3 {
            let o = c[k];
            if o <= 0 || kind[o as usize] != 0 {
                continue;
            }
            let others: Vec<i32> = (0..3).filter(|&j| j != k).map(|j| -c[j]).collect();
            let (x, y) = (others[0], others[1]);
            if o > x.abs() && o > y.abs() && has(-o, x) && has(-o, y) {
                kind[o as usize] = 1;
                ins[o as usize] = [x, y, 0];
                break;
            }
        }
    }
    // the groups of ternary clauses over one variable triple
    let mut g = 0usize;
    let mut halves: Vec<(u32, i32, u32)> = Vec::new(); // (output var, select lit, data var)
    while g < tri.len() {
        let mut h = g + 1;
        while h < tri.len() && tri[h].0 == tri[g].0 {
            h += 1;
        }
        let group = &tri[g..h];
        if !pay(work, ((h - g) * (h - g)) as u64, budget, stop) {
            return None;
        }
        let vs = group[0].0;
        if group.len() == 4 {
            let odd: Vec<usize> = group.iter().map(|(_v, c)| c.iter().filter(|l| **l < 0).count() % 2).collect();
            let mut sorted: Vec<[i32; 3]> = group.iter().map(|(_v, c)| { let mut x = *c; x.sort_unstable(); x }).collect();
            sorted.sort_unstable();
            sorted.dedup();
            if odd.iter().all(|&p| p == odd[0]) && sorted.len() == 4 {
                let o = vs[2] as usize;
                if kind[o] == 0 {
                    kind[o] = 2;
                    ins[o] = [vs[0] as i32, vs[1] as i32, 0];
                }
            }
        }
        for i in 0..group.len() {
            for j in (i + 1)..group.len() {
                let (c1, c2) = (group[i].1, group[j].1);
                let shared: Vec<i32> = c1.iter().copied().filter(|l| c2.contains(l)).collect();
                if shared.len() != 1 {
                    continue;
                }
                let p = shared[0];
                let r1: Vec<i32> = c1.iter().copied().filter(|&l| l != p).collect();
                let r2: Vec<i32> = c2.iter().copied().filter(|&l| l != p).collect();
                if !(r2.contains(&-r1[0]) && r2.contains(&-r1[1])) {
                    continue;
                }
                let (x, y) = (r1[0].unsigned_abs(), r1[1].unsigned_abs());
                halves.push((x, -p, y));
                halves.push((y, -p, x));
            }
        }
        g = h;
    }
    // by output, then select variable, then sign: a select with both signs
    // among one output's halves is an if-then-else, found in one pass
    if !pay(work, 2 * halves.len() as u64, budget, stop) {
        return None;
    }
    halves.sort_unstable_by_key(|&(o, s, d)| (o, s.unsigned_abs(), s > 0, d));
    let mut i = 0usize;
    while i < halves.len() {
        let (o, s0, _d0) = halves[i];
        let sv = s0.unsigned_abs();
        let mut j = i;
        let (mut neg_d, mut pos_d) = (None, None);
        while j < halves.len() && halves[j].0 == o && halves[j].1.unsigned_abs() == sv {
            if halves[j].1 > 0 {
                pos_d.get_or_insert(halves[j].2);
            } else {
                neg_d.get_or_insert(halves[j].2);
            }
            j += 1;
        }
        if kind[o as usize] == 0 {
            if let (Some(a), Some(b)) = (pos_d, neg_d) {
                if sv < o && a < o && b < o {
                    kind[o as usize] = 3;
                    ins[o as usize] = [sv as i32, a as i32, b as i32];
                }
            }
        }
        i = j;
    }
    // the switches' selects (a scan over the variables, paid first)
    if !pay(work, 2 * n as u64, budget, stop) {
        return None;
    }
    let mut sel_count: Vec<u32> = vec![0; n];
    let mut muxes = 0u64;
    let mut sels: Vec<u32> = Vec::new();
    for o in 1..n {
        match kind[o] {
            3 => {
                let s = ins[o][0].unsigned_abs();
                sel_count[s as usize] += 1;
                muxes += 1;
            }
            1 => {
                let (x, y) = (ins[o][0], ins[o][1]);
                if x < 0 && y < 0 && kind[(-x) as usize] == 1 && kind[(-y) as usize] == 1 {
                    let p = ins[(-x) as usize];
                    let q = ins[(-y) as usize];
                    for &s in &p[..2] {
                        if q[..2].contains(&-s) {
                            sel_count[s.unsigned_abs() as usize] += 1;
                            muxes += 1;
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    for v in 1..n {
        if sel_count[v] > 0 {
            sels.push(v as u32);
        }
    }
    if muxes == 0 {
        return None;
    }
    // supports, memoised: 0 unknown, 1 small (in `small`), 2 wide
    let mut state = vec![0u8; n];
    let mut small: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    let mut sel_sup: Vec<(u32, Option<Vec<u32>>)> = Vec::new();
    for &s in &sels {
        let mut stack: Vec<(u32, bool)> = vec![(s, false)];
        while let Some((u, done)) = stack.pop() {
            if !pay(work, 1, budget, stop) {
                return None;
            }
            if state[u as usize] != 0 {
                continue;
            }
            if kind[u as usize] == 0 {
                state[u as usize] = 1;
                small.insert(u, vec![u]);
                continue;
            }
            let k = if kind[u as usize] == 3 { 3 } else { 2 };
            if !done {
                stack.push((u, true));
                for &l in &ins[u as usize][..k] {
                    let x = l.unsigned_abs();
                    if state[x as usize] == 0 {
                        stack.push((x, false));
                    }
                }
                continue;
            }
            let mut acc: Vec<u32> = Vec::new();
            let mut wide = false;
            for &l in &ins[u as usize][..k] {
                let x = l.unsigned_abs();
                match state[x as usize] {
                    2 => {
                        wide = true;
                        break;
                    }
                    1 => {
                        acc.extend_from_slice(&small[&x]);
                    }
                    _ => acc.push(x), // a cycle edge reads as a leaf
                }
                if acc.len() > 4 * CAP {
                    acc.sort_unstable();
                    acc.dedup();
                }
            }
            if !pay(work, acc.len() as u64, budget, stop) {
                return None;
            }
            acc.sort_unstable();
            acc.dedup();
            if wide || acc.len() > CAP {
                state[u as usize] = 2;
            } else {
                state[u as usize] = 1;
                small.insert(u, acc);
            }
        }
        if *work > budget {
            return None;
        }
        sel_sup.push((s, if state[s as usize] == 1 { Some(small[&s].clone()) } else { None }));
    }
    // weights: an input steers the switches whose select support holds it
    let mut weight: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
    for (s, sup) in &sel_sup {
        if let Some(sup) = sup {
            let c = sel_count[*s as usize] as u64;
            for &u in sup {
                *weight.entry(u).or_insert(0) += c;
            }
        }
    }
    let mut ranked: Vec<(u64, u32)> = weight.iter().map(|(&u, &w)| (w, u)).collect();
    ranked.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked.truncate(16);
    if ranked.is_empty() {
        return None;
    }
    let rank: std::collections::HashMap<u32, usize> = ranked.iter().enumerate().map(|(i, &(_w, u))| (u, i)).collect();
    // cover[k]: switches whose select support lies within the k heaviest
    let mut cover = vec![0u64; 17];
    for (s, sup) in &sel_sup {
        if let Some(sup) = sup {
            let worst = sup.iter().map(|u| rank.get(u).copied().unwrap_or(usize::MAX)).max().unwrap_or(usize::MAX);
            if worst < 16 {
                cover[worst + 1] += sel_count[*s as usize] as u64;
            }
        }
    }
    for k in 1..17 {
        cover[k] += cover[k - 1];
    }
    let top = cover[ranked.len().min(16)];
    if top == 0 {
        return None;
    }
    let mut k = 1;
    while k < ranked.len() && (cover[k] as f64) < 0.9 * top as f64 {
        k += 1;
    }
    let mut vars: Vec<i32> = ranked[..k].iter().map(|&(_w, u)| u as i32).collect();
    vars.sort_unstable();
    let gates = (1..n).filter(|&v| kind[v] != 0).count() as u64;
    Some(Reading { muxes, gates, cover: cover[k] as f64 / muxes as f64, vars })
}

fn negated(units: &[i32]) -> Vec<i32> {
    units.iter().map(|&u| -u).collect()
}

/// Every clause of the flat formula satisfied by the model (index = DIMACS
/// variable)?
fn satisfies(formula: &[i32], model: &[bool]) -> bool {
    let mut sat = false;
    let mut any = false;
    for &l in formula {
        if l == 0 {
            if !sat && any {
                return false;
            }
            if !any {
                return false; // an empty clause
            }
            sat = false;
            any = false;
            continue;
        }
        any = true;
        let v = l.unsigned_abs() as usize;
        let val = v < model.len() && model[v];
        if (l > 0) == val {
            sat = true;
        }
    }
    true
}

enum Case {
    Refuted,
    Sat(Vec<bool>),
    Unknown,
    /// The case's limit does not even cover loading the formula: no deeper
    /// case could do better, so the split stops.
    NoBudget,
}

/// Solve one case in a sub-solver; on UNSAT write its relativized proof.
fn solve_case(solver: &mut Solver, formula: &[i32], max_var: i32, units: &[i32], limit: u64) -> Case {
    // loading the formula into the sub-solver is work too (a literal a
    // tick): paid before it starts, and out of the case's limit
    let load = formula.len() as u64 + 2 * units.len() as u64;
    if load >= limit {
        return Case::NoBudget;
    }
    solver.statistics.ticks = solver.statistics.ticks.saturating_add(load);
    solver.control.split_work = solver.control.split_work.saturating_add(load);
    let limit = limit - load;
    let mut sub = Box::new(crate::internal::init());
    sub.options = solver.options;
    for (name, value) in [
        ("quiet", 1),
        ("factor", 0),
        ("preprocessfactor", 0),
        ("structsym", 0),
        ("structsymext", 0),
        ("structpoints", 0),
        ("structparity", 0),
        ("structcount", 0),
        ("structcsp", 0),
        ("structdd", 0),
        ("structarith", 0),
        ("structsort", 0),
        ("structdiv", 0),
        ("gauss", 0),
        ("card", 0),
        ("freewalk", 0),
        ("freeread", 0),
        ("ctlfirst", 0),
        ("ctlsplit", 0),
    ] {
        crate::internal::set_option(&mut sub, name, value);
    }
    crate::internal::set_ticks_limit(&mut sub, limit);
    let proving = solver.proof.is_some();
    if proving {
        crate::proof::init_captured_proof(&mut sub);
    }
    // the parent's alarm stops it, and its work reaches the parent's tick
    // counter as it goes (as the sorter and divider passes' sub-solvers do)
    sub.parent_stop = &solver.termination.flagged as *const std::sync::atomic::AtomicBool as usize;
    sub.parent_ticks_base = solver.statistics.ticks;
    sub.parent_ticks = &mut solver.statistics.ticks as *mut u64 as usize;
    for (i, &l) in formula.iter().enumerate() {
        if i % 65536 == 0 && solver.termination.flagged.load(std::sync::atomic::Ordering::Relaxed) {
            crate::internal::release(*sub);
            return Case::Unknown;
        }
        crate::internal::add(&mut sub, l);
    }
    for &u in units {
        crate::internal::add(&mut sub, u);
        crate::internal::add(&mut sub, 0);
    }
    let res = crate::internal::solve(&mut sub);
    let used = sub.statistics.work_clock();
    solver.statistics.ticks = sub.parent_ticks_base.saturating_add(used);
    solver.control.split_work = solver.control.split_work.saturating_add(used);
    let out = match res {
        20 => {
            if proving {
                let lines = crate::proof::take_captured_proof(&mut sub);
                let not_u = negated(units);
                let mut written: Vec<Vec<i32>> = Vec::new();
                for c in lines {
                    if c.is_empty() {
                        continue;
                    }
                    // a lemma holding a case unit is satisfied under the case:
                    // relativized it is a tautology, and no later step needs it
                    if c.iter().any(|l| units.contains(l)) {
                        continue;
                    }
                    let mut line = c;
                    for &n in &not_u {
                        if !line.contains(&n) {
                            line.push(n);
                        }
                    }
                    crate::proof::add_external_lits_to_proof(solver, &line);
                    written.push(line);
                }
                crate::proof::add_external_lits_to_proof(solver, &not_u);
                solver.control.split_lemmas += written.len() as u64 + 1;
                for line in written.iter().rev() {
                    if *line != not_u {
                        crate::proof::delete_external_from_proof(solver, line);
                    }
                }
            }
            Case::Refuted
        }
        10 => {
            let mut model = vec![false; max_var as usize + 1];
            for v in 1..=max_var {
                model[v as usize] = crate::internal::value(&mut sub, v) > 0;
            }
            Case::Sat(model)
        }
        _ => Case::Unknown,
    };
    crate::internal::release(*sub);
    out
}

/// Write the resolvents that join a node's refuted children into not-u of
/// the node (the empty clause at the root).
fn join(solver: &mut Solver, node_units: &[i32], bits: &[i32]) {
    if solver.proof.is_none() {
        return;
    }
    let base = negated(node_units);
    let k = bits.len();
    // clause of a prefix of j bits with value p: base plus, per bit i < j,
    // the bit's literal that the prefix's value falsifies
    let clause_of = |j: usize, p: usize| -> Vec<i32> {
        let mut c = base.clone();
        for (i, &b) in bits.iter().enumerate().take(j) {
            c.push(if (p >> i) & 1 == 1 { -b } else { b });
        }
        c
    };
    for j in (0..k).rev() {
        for p in 0..(1usize << j) {
            let c = clause_of(j, p);
            crate::proof::add_external_lits_to_proof(solver, &c);
        }
        // the parents of this layer are no longer needed
        for p in 0..(1usize << (j + 1)) {
            let c = clause_of(j + 1, p);
            crate::proof::delete_external_from_proof(solver, &c);
        }
    }
}

/// The split before the search: 0 to go on searching, 10 or 20 when it
/// decided the formula (a model kept for the witness, or the proof ended
/// with the empty clause).
pub fn split(solver: &mut Solver) -> i32 {
    if solver.options.ctlsplit == 0 {
        return 0;
    }
    let formula = std::mem::take(&mut solver.control.formula);
    if formula.is_empty() || solver.inconsistent {
        solver.control.split_status = "no-formula".to_string();
        crate::print::message(solver, "control split: no formula kept (not buffered, or already refuted)");
        return 0;
    }
    // the free-choice walk may have found a checked model already: the
    // search returns it at once, and a split would only spend work
    if solver.freewalk.model.is_some() {
        solver.control.split_status = "walk-model".to_string();
        return 0;
    }
    solver.control.split_pre = solver.statistics.work_clock();
    let max_var = formula.iter().map(|l| l.abs()).max().unwrap_or(0);
    let mut vars = read_vars(max_var);
    if vars.is_empty() && std::env::var("SAT_CTL_VARS").map(|v| v.trim().is_empty()).unwrap_or(true) {
        // no set given: read it from the formula, within its own budget; the
        // reading's work counts in the work clock
        let mut allowance = (solver.options.ctlreadticks as u64).saturating_mul(1000);
        if solver.limited.ticks {
            allowance = allowance.min(solver.limits.ticks.saturating_sub(solver.statistics.work_clock()));
        }
        // the reading counts straight into the tick counter, against an
        // absolute limit: a signal during the reading finds its work in the
        // exit clock (the harness prices a timed-out cell with that clock)
        let t0 = solver.statistics.ticks;
        let limit = t0.saturating_add(allowance);
        let read = read_control(&formula, max_var, limit, &mut solver.statistics.ticks, &solver.termination.flagged);
        let work = solver.statistics.ticks.saturating_sub(t0);
        solver.control.read_work = work;
        match read {
            Some(r) => {
                solver.control.read_muxes = r.muxes;
                solver.control.read_cover = r.cover;
                println!(
                    "c control read switches={} gates={} cover={:.4} vars={} work={}",
                    r.muxes,
                    r.gates,
                    r.cover,
                    r.vars.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","),
                    work
                );
                // the shape that paid: many switches, most of them steered by
                // the set, and switches that are a large part of a circuit with
                // a datapath (solver README, "Control words"): a quarter of the
                // gates or more (fixing the selects of a few switches leaves
                // the rest of the circuit as it was: grs, summle), and not
                // nearly all of them (in a decision diagram, the gm
                // multipliers, every gate is a switch on an input, and fixing
                // the top variables only expands it); oisc's are half
                let share = r.muxes as f64 / r.gates.max(1) as f64;
                if r.muxes >= 1000 && r.cover >= 0.5 && r.vars.len() >= 2 && (0.25..=0.9).contains(&share) {
                    vars = r.vars;
                }
            }
            None => println!("c control read none work={}", work),
        }
    }
    if vars.is_empty() {
        solver.control.split_status = "no-vars".to_string();
        crate::print::message(solver, "control split: no control set (none given, none read)");
        return 0;
    }
    // a case of the control word's kind falls to propagation and the usual
    // clean-up, about a pass over the formula; one that costs ctlsplitratio
    // ticks a literal or more is a search, and the split stops there (the
    // first two cases within four times that, the mean of the cases after)
    let literals = formula.len() as u64;
    let ratio = solver.options.ctlsplitratio.max(1) as u64;
    // ctlsplit=2 is the effect test's probe (bead d96.8.3): a case without an
    // answer does not end the split, every case runs and is counted
    let probe = solver.options.ctlsplit == 2;
    let bits_per = solver.options.ctlsplitbits.max(1) as usize;
    let max_levels = solver.options.ctlsplitlevels.max(1) as usize;
    let case_limit = (solver.options.ctlsplitcase as u64).saturating_mul(1000);
    let budget = (solver.options.ctlsplitticks as u64).saturating_mul(1000);
    let w0 = solver.statistics.work_clock();
    let mut nodes: Vec<Node> = vec![Node { units: Vec::new(), level: 0, pos: 0, bits: Vec::new(), children: Vec::new(), status: 2 }];
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    // split node `n` on the next bits; false when no bits are left
    let expand = |nodes: &mut Vec<Node>, queue: &mut std::collections::VecDeque<usize>, n: usize| -> bool {
        let pos = nodes[n].pos;
        if pos >= vars.len() {
            return false;
        }
        let end = (pos + bits_per).min(vars.len());
        let bits: Vec<i32> = vars[pos..end].to_vec();
        let level = if n == 0 { 0 } else { nodes[n].level + 1 };
        for w in 0..(1usize << bits.len()) {
            let mut units = nodes[n].units.clone();
            for (i, &b) in bits.iter().enumerate() {
                units.push(if (w >> i) & 1 == 1 { b } else { -b });
            }
            let id = nodes.len();
            nodes.push(Node { units, level, pos: end, bits: Vec::new(), children: Vec::new(), status: 0 });
            nodes[n].children.push(id);
            queue.push_back(id);
        }
        nodes[n].bits = bits;
        nodes[n].status = 2;
        true
    };
    expand(&mut nodes, &mut queue, 0);
    let mut verdict = 0;
    let mut stopped = "";
    while let Some(n) = queue.pop_front() {
        let spent = solver.statistics.work_clock().saturating_sub(w0);
        if spent >= budget {
            stopped = "budget";
            break;
        }
        if solver.termination.flagged.load(std::sync::atomic::Ordering::SeqCst) {
            stopped = "stopped";
            break;
        }
        let mut limit = case_limit.min(budget - spent);
        if solver.limited.ticks {
            let left = solver.limits.ticks.saturating_sub(solver.statistics.work_clock());
            limit = limit.min(left);
        }
        // the first two cases test the shape itself: a case that is a search
        // (four times the cost bound a literal) ends the split at once, before
        // it spends a full case limit (gm16spctrc spent four of 3G each)
        let mut early = false;
        if !probe && solver.control.split_cases < 2 {
            let cap = ratio.saturating_mul(4).saturating_mul(literals).max(1);
            if cap < limit {
                // the cap binds: a case that reaches it is a search
                limit = cap;
                early = true;
            }
        }
        if limit == 0 {
            stopped = "budget";
            break;
        }
        if !probe && solver.control.split_cases >= 4 {
            let mean = solver.control.split_work / solver.control.split_cases;
            if mean > ratio.saturating_mul(literals) {
                stopped = "costly";
                break;
            }
        }
        solver.control.split_cases += 1;
        let units = nodes[n].units.clone();
        match solve_case(solver, &formula, max_var, &units, limit) {
            Case::Refuted => {
                nodes[n].status = 1;
                solver.control.split_refuted += 1;
            }
            Case::Sat(model) => {
                if satisfies(&formula, &model) {
                    solver.control.split_sat += 1;
                    solver.control.model = Some(model);
                    verdict = 10;
                    break;
                }
                // never trust an unchecked model: treat the case as open
                crate::print::message(solver, "control split: a case's model failed the check; split abandoned");
                nodes[n].status = 3;
                stopped = "bad-model";
                break;
            }
            Case::NoBudget => {
                nodes[n].status = 3;
                stopped = "budget";
                break;
            }
            Case::Unknown => {
                solver.control.split_unknown += 1;
                if early {
                    nodes[n].status = 3;
                    stopped = "costly";
                    break;
                }
                if nodes[n].level + 1 < max_levels && expand(&mut nodes, &mut queue, n) {
                    continue;
                }
                nodes[n].status = 3;
                if probe {
                    continue; // the probe runs every case and reports them all
                }
                stopped = "open";
                break;
            }
        }
    }
    if verdict == 0 && stopped.is_empty() {
        // every case answered: join the refuted ones bottom-up
        fn refuted(nodes: &[Node], n: usize) -> bool {
            match nodes[n].status {
                1 => true,
                2 => nodes[n].children.iter().all(|&c| refuted(nodes, c)),
                _ => false,
            }
        }
        if refuted(&nodes, 0) {
            // post-order: a split node's children first
            let mut order: Vec<usize> = Vec::new();
            let mut stack = vec![(0usize, false)];
            while let Some((n, done)) = stack.pop() {
                if nodes[n].status != 2 {
                    continue;
                }
                if done {
                    order.push(n);
                } else {
                    stack.push((n, true));
                    for &c in &nodes[n].children {
                        stack.push((c, false));
                    }
                }
            }
            for n in order {
                let units = nodes[n].units.clone();
                let bits = nodes[n].bits.clone();
                join(solver, &units, &bits);
            }
            verdict = 20;
        }
    }
    solver.control.split_status = match verdict {
        10 => "sat".to_string(),
        20 => "unsat".to_string(),
        _ => if !stopped.is_empty() { stopped.to_string() } else if probe { "probed".to_string() } else { "open".to_string() },
    };
    if verdict == 20 {
        solver.inconsistent = true;
    } else if verdict == 0 {
        // the refuted cases' clauses help the search, and they are in the
        // proof already; a split node whose children are all refuted gives its
        // own clause (joined in the proof) instead
        fn collect(solver: &mut Solver, nodes: &[Node], n: usize, out: &mut Vec<Vec<i32>>) -> bool {
            match nodes[n].status {
                1 => {
                    out.push(negated(&nodes[n].units));
                    true
                }
                2 => {
                    let mut all = true;
                    let mut mine: Vec<Vec<i32>> = Vec::new();
                    for &c in &nodes[n].children {
                        all &= collect(solver, nodes, c, &mut mine);
                    }
                    if all && n != 0 {
                        join(solver, &nodes[n].units, &nodes[n].bits);
                        out.push(negated(&nodes[n].units));
                    } else {
                        out.extend(mine);
                    }
                    all
                }
                _ => false,
            }
        }
        let mut learned: Vec<Vec<i32>> = Vec::new();
        collect(solver, &nodes, 0, &mut learned);
        for c in &learned {
            for &l in c {
                crate::internal::add(solver, l);
            }
            crate::internal::add(solver, 0);
        }
    }
    let msg = format!(
        "control split: {} ({} cases, {} refuted, {} without an answer, work {})",
        solver.control.split_status, solver.control.split_cases, solver.control.split_refuted,
        solver.control.split_unknown, solver.control.split_work
    );
    crate::print::message(solver, msg);
    verdict
}
