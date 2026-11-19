//! Not in kissat. The number pass of the structure pass (epic
//! SAT-playground-d96.9, bead d96.9.4): numbers stored as sorted bits, and
//! the clauses their encoding lacks.
//!
//! The problem, in plain terms. Z3 writes an integer x in 0..n as n sorted
//! bits (bit k is on when x is above k, with the clauses "bit k+1 -> bit
//! k"), and a test on x as a count over the bits. A count over sorted bits
//! only gives the bits back: "at least two of them are on" is the second
//! bit. But the count writes a fresh variable for it and never says the two
//! are equal, so unit propagation runs from the bits to the count and not
//! back. This pass says it: two two-literal clauses for each such variable.
//!
//! A small example. x in 0..3 as bits g1, g2, g3 with g2 -> g1 and g3 ->
//! g2, and a count with the helpers h1 = "at least one bit", h2 = "at least
//! two", h3 = "all three":
//!
//!   set x to 0, 1, 2, 3     h1 comes out off on on on, h2 off off on on,
//!   and propagate           h3 off off off on
//!   so                      h1 is g1, h2 is g2, h3 is g3
//!   ask propagation         "h1 -> g1", "h2 -> g2", "g2 -> h2" and
//!                           "not h3 -> not g3" are not there: add them
//!
//! How the bits are found, with no name and no numbering: gates are read by
//! shape (AND, OR of any width, XOR of two) and settled forward from the
//! variables no shape could define; a chain is a path of two-literal
//! clauses between the variables left free; and three tests by propagation
//! decide what is a bit: the bit test (set a chain's lower members on and
//! its higher ones off around one member: a real bit stays open, a helper
//! is set), the end test (an end member goes when the two patterns that
//! differ in it alone set the same variables: nothing reads it), and who
//! reads whom (a chain that every pattern of another chain sets is a
//! function of that number). `tools/number_read.py` is the same reading in
//! Python, with the tests on Z3's own toys; `tools/number_pass_check.py`
//! holds this file to it, clause for clause.
//!
//! What is added: units and two-literal clauses only, each true in every
//! model of the input. `equal` (a variable that is on from a value upwards
//! and its bit, both ways), `bound` (a variable to the lowest or highest
//! value it allows), `dead` (a value no model takes), `unit` (a variable
//! every value sets the same way), and the steps of a proof where a clause
//! is not checkable as it stands.
//!
//! Proof: every added clause is a `rup` line, in the order it was added.
//! A clause that propagation does not refute by itself is proved in steps
//! (the same clause for the values below, or above, one at a time; each
//! step is a `rup` line too); when even that fails the clause is left out.
//!
//! Not read: a chain of fewer than three members (a formula without gates
//! has thousands: two variables and one clause), a chain past 62 bits, and
//! the partial readers one by one (a placement of the pyramid reads three
//! numbers: it reaches the bits through the helpers once those are equal).

use crate::structure::{Derived, Formula};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

/// A candidate chain has this many members at least.
const MIN_MEMBERS: usize = 3;
/// A chain past this many bits is not read value by value (the patterns are
/// kept as bits of a u64).
const MAX_BITS: usize = 62;
/// Maximal paths taken from one start, and in all; past these, one longest
/// path a start.
const PER_SOURCE: usize = 64;
const TOTAL_PATHS: usize = 20_000;

#[inline]
fn li(l: i32) -> usize {
    ((l.unsigned_abs() as usize) << 1) | (l < 0) as usize
}

#[inline]
fn var(l: i32) -> u32 {
    l.unsigned_abs()
}

// ---------------------------------------------------------------------------
// unit propagation over the input clauses
// ---------------------------------------------------------------------------

struct Prop {
    val: Vec<i8>,
    trail: Vec<i32>,
    /// literal index -> the literals a two-literal clause makes it imply
    bins: Vec<Vec<i32>>,
    /// literal index -> the longer clauses watching it
    watch: Vec<Vec<u32>>,
    lits: Vec<i32>,
    start: Vec<u32>,
    root: usize,
    conflict: bool,
    /// Work: one per literal taken off the trail and per clause looked at.
    steps: u64,
}

impl Prop {
    fn new(f: &Formula) -> Prop {
        let nl = 2 * (f.vars as usize + 1);
        let mut p = Prop {
            val: vec![0; f.vars as usize + 1],
            trail: Vec::new(),
            bins: vec![Vec::new(); nl],
            watch: vec![Vec::new(); nl],
            lits: Vec::new(),
            start: vec![0],
            root: 0,
            conflict: false,
            steps: 0,
        };
        let mut units: Vec<i32> = Vec::new();
        for i in 0..f.len() {
            if f.taut[i] {
                continue;
            }
            let c = f.clause(i);
            p.steps += c.len() as u64 + 1;
            match c.len() {
                0 => p.conflict = true,
                1 => units.push(c[0]),
                2 => {
                    p.bins[li(-c[0])].push(c[1]);
                    p.bins[li(-c[1])].push(c[0]);
                }
                _ => p.push_long(c),
            }
        }
        if !p.conflict && !p.assume(&units) {
            p.conflict = true;
        }
        p.root = p.trail.len();
        p
    }

    fn push_long(&mut self, c: &[i32]) {
        let ci = (self.start.len() - 1) as u32;
        self.lits.extend_from_slice(c);
        self.start.push(self.lits.len() as u32);
        self.watch[li(c[0])].push(ci);
        self.watch[li(c[1])].push(ci);
    }

    #[inline]
    fn value(&self, l: i32) -> i8 {
        let v = self.val[l.unsigned_abs() as usize];
        if l > 0 {
            v
        } else {
            -v
        }
    }

    #[inline]
    fn set(&mut self, l: i32) {
        self.val[l.unsigned_abs() as usize] = if l > 0 { 1 } else { -1 };
        self.trail.push(l);
    }

    /// Set the literals and propagate. False on a conflict; `undo` clears
    /// what was set either way.
    fn assume(&mut self, lits: &[i32]) -> bool {
        if self.conflict {
            return false;
        }
        let head = self.trail.len();
        for &l in lits {
            match self.value(l) {
                0 => self.set(l),
                1 => {}
                _ => return false,
            }
        }
        self.propagate(head)
    }

    fn propagate(&mut self, mut head: usize) -> bool {
        while head < self.trail.len() {
            let l = self.trail[head];
            head += 1;
            let idx = li(l);
            self.steps += 1 + self.bins[idx].len() as u64;
            for k in 0..self.bins[idx].len() {
                let m = self.bins[idx][k];
                match self.value(m) {
                    0 => self.set(m),
                    1 => {}
                    _ => return false,
                }
            }
            let fl = -l;
            let fi = li(fl);
            if self.watch[fi].is_empty() {
                continue;
            }
            let ws = std::mem::take(&mut self.watch[fi]);
            self.steps += ws.len() as u64;
            let mut keep: Vec<u32> = Vec::with_capacity(ws.len());
            let mut ok = true;
            let mut j = 0;
            while j < ws.len() {
                let ci = ws[j] as usize;
                j += 1;
                let (s, e) = (self.start[ci] as usize, self.start[ci + 1] as usize);
                if self.lits[s] == fl {
                    self.lits.swap(s, s + 1);
                }
                let o = self.lits[s];
                let ov = self.value(o);
                if ov == 1 {
                    keep.push(ci as u32);
                    continue;
                }
                let mut moved = false;
                for k in s + 2..e {
                    let m = self.lits[k];
                    if self.value(m) != -1 {
                        self.lits.swap(s + 1, k);
                        self.watch[li(m)].push(ci as u32);
                        moved = true;
                        break;
                    }
                }
                if moved {
                    continue;
                }
                keep.push(ci as u32);
                if ov == -1 {
                    keep.extend_from_slice(&ws[j..]);
                    ok = false;
                    break;
                }
                self.set(o);
            }
            self.watch[fi] = keep;
            if !ok {
                return false;
            }
        }
        true
    }

    fn undo(&mut self, mark: usize) {
        while self.trail.len() > mark {
            let l = self.trail.pop().unwrap();
            self.val[l.unsigned_abs() as usize] = 0;
        }
    }

    /// Setting the literals runs into a conflict by propagation alone: the
    /// clause of their opposites is a `rup` step.
    fn refutes(&mut self, lits: &[i32]) -> bool {
        if self.conflict {
            return true;
        }
        let mark = self.trail.len();
        let ok = self.assume(lits);
        self.undo(mark);
        !ok
    }

    /// Add a clause while nothing is assumed.
    fn add(&mut self, clause: &[i32]) {
        debug_assert_eq!(self.trail.len(), self.root);
        if self.conflict || clause.iter().any(|&l| self.value(l) == 1) {
            return;
        }
        let c: Vec<i32> = clause.iter().copied().filter(|&l| self.value(l) == 0).collect();
        match c.len() {
            0 => self.conflict = true,
            1 => {
                if !self.assume(&c) {
                    self.conflict = true;
                }
                self.root = self.trail.len();
            }
            2 => {
                self.bins[li(-c[0])].push(c[1]);
                self.bins[li(-c[1])].push(c[0]);
            }
            _ => self.push_long(&c),
        }
    }
}

// ---------------------------------------------------------------------------
// a number and what reads it
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Number {
    /// lits[k] is on when the value is above k; lits[k + 1] -> lits[k].
    lits: Vec<i32>,
    /// Values whose pattern propagation refutes, as bits.
    dead: u64,
    /// variable -> (values where a pattern sets it true, where false).
    readers: HashMap<u32, (u64, u64)>,
    /// Past MAX_BITS: not read.
    unread: bool,
}

impl Number {
    fn n(&self) -> usize {
        self.lits.len()
    }
    /// The values no pattern refuted, in order. A chain that was not read
    /// (past MAX_BITS) has none: its values do not fit the masks.
    fn live(&self) -> Vec<usize> {
        if self.unread {
            return Vec::new();
        }
        (0..=self.n()).filter(|&v| self.dead >> v & 1 == 0).collect()
    }
    fn pattern(&self, v: usize) -> Vec<i32> {
        self.lits.iter().enumerate().map(|(k, &l)| if k < v { l } else { -l }).collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Partial,
    Constant,
    Ge,
    Le,
    Eq,
    Ne,
    Set,
}

fn mask_of(values: &[usize]) -> u64 {
    values.iter().fold(0, |m, &v| m | 1u64 << v)
}

/// What a reader is, from where the patterns set it. `live` are the live
/// values in order.
fn kind_of(t: u64, f: u64, live: &[usize]) -> Kind {
    let all = mask_of(live);
    if (t | f) & all != all {
        return Kind::Partial;
    }
    let ts = t & all;
    let k = ts.count_ones() as usize;
    if k == 0 || k == live.len() {
        return Kind::Constant;
    }
    if ts == mask_of(&live[live.len() - k..]) {
        return Kind::Ge;
    }
    if ts == mask_of(&live[..k]) {
        return Kind::Le;
    }
    if k == 1 {
        return Kind::Eq;
    }
    if k == live.len() - 1 {
        return Kind::Ne;
    }
    Kind::Set
}

/// (values, cut points that some variable equals, values with a test of
/// their own): the census's profile of a number.
fn profile_of(num: &Number) -> (usize, usize, usize) {
    let live = num.live();
    let mut cuts: HashSet<u64> = HashSet::new();
    let mut tests: HashSet<u64> = HashSet::new();
    let all = mask_of(&live);
    for &(t, f) in num.readers.values() {
        match kind_of(t, f, &live) {
            Kind::Ge => {
                cuts.insert(t & all);
            }
            Kind::Le => {
                cuts.insert(f & all);
            }
            Kind::Eq => {
                tests.insert(t & all);
            }
            Kind::Ne => {
                tests.insert(f & all);
            }
            _ => {}
        }
    }
    (live.len(), cuts.len(), tests.len())
}

/// The mark of an integer under a count: four values or more, and every
/// cut point but one at most has a variable equal to it.
fn is_counted(num: &Number) -> bool {
    let (values, cuts, _tests) = profile_of(num);
    values >= 4 && cuts + 2 >= values
}

// ---------------------------------------------------------------------------
// the reading
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Counts {
    numbers: usize,
    counted: usize,
    equal: u64,
    bound: u64,
    dead: u64,
    unit: u64,
    steps: u64,
    have: u64,
    noproof: u64,
    holes: u64,
    partial: u64,
    candidates: usize,
    free: usize,
    gates: usize,
    ring: usize,
    not_a_bit: u64,
    end_dropped: u64,
    too_long: u64,
}

struct Reader<'a> {
    p: Prop,
    derived: &'a mut Derived,
    budget: u64,
    base: u64,
    /// The solver's work clock: written at every poll, so a run that a
    /// signal ends in the middle of the pass reports the pass's work.
    ticks: &'a mut u64,
    stop: &'a AtomicBool,
    halt: bool,
    counts: Counts,
    numbers: Vec<Number>,
}

#[derive(Clone, Copy, PartialEq)]
enum LinkKind {
    Equal,
    Bound,
    Dead,
    Unit,
}

impl<'a> Reader<'a> {
    /// Out of budget, stopped from outside, or the proof file failed.
    fn spent(&mut self) -> bool {
        *self.ticks = self.base + self.p.steps;
        if !self.halt && (*self.ticks > self.budget || self.stop.load(Ordering::Relaxed)) {
            self.halt = true;
        }
        if let Some(w) = self.derived.proof.as_ref() {
            if w.failed {
                self.halt = true;
            }
        }
        self.halt
    }

    /// Is the clause there already (propagation runs it every way), a `rup`
    /// step as it stands, or neither.
    fn how(&mut self, clause: &[i32]) -> u8 {
        let mut every = true;
        let mut assumed: Vec<i32> = Vec::with_capacity(clause.len());
        for skip in 0..clause.len() {
            assumed.clear();
            for (k, &l) in clause.iter().enumerate() {
                if k != skip {
                    assumed.push(-l);
                }
            }
            let mark = self.p.trail.len();
            let ok = self.p.assume(&assumed);
            let got = !ok || self.p.value(clause[skip]) == 1;
            self.p.undo(mark);
            if !got {
                every = false;
                break;
            }
        }
        if every {
            return 0;
        }
        assumed.clear();
        assumed.extend(clause.iter().map(|&l| -l));
        if self.p.refutes(&assumed) {
            1
        } else {
            2
        }
    }

    /// Write the clause as a `rup` step, add it to F' and give it to
    /// propagation.
    fn put(&mut self, clause: &[i32]) {
        let mut id = 0;
        if let Some(w) = self.derived.proof.as_mut() {
            match w.rup(clause) {
                Ok(i) => id = i,
                Err(_) => {
                    self.halt = true;
                    return;
                }
            }
        }
        self.derived.add(clause, id);
        self.p.add(clause);
    }

    fn add(&mut self, kind: LinkKind, clause: &[i32], num: Option<&Number>) {
        if self.spent() {
            return;
        }
        let mut how = self.how(clause);
        if how == 0 {
            self.counts.have += 1;
            return;
        }
        if how == 2 {
            match num {
                Some(n) if self.steps(clause, n) => how = 1,
                _ => {
                    self.counts.noproof += 1;
                    return;
                }
            }
        }
        if how == 1 && !self.halt {
            self.put(clause);
            match kind {
                LinkKind::Equal => self.counts.equal += 1,
                LinkKind::Bound => self.counts.bound += 1,
                LinkKind::Dead => self.counts.dead += 1,
                LinkKind::Unit => self.counts.unit += 1,
            }
        }
    }

    /// For (z or B), B a bound literal of the number: the clauses (z or B')
    /// for the weaker bounds, weakest first, each a `rup` step once the one
    /// before it is there; then the clause itself is one. A clause with no
    /// bit at all walks all the way up. False when that does not work (the
    /// steps made so far stay: each is true and was checked).
    fn steps(&mut self, clause: &[i32], num: &Number) -> bool {
        let pos: HashMap<i32, usize> = num.lits.iter().enumerate().map(|(k, &l)| (l, k)).collect();
        let own: HashSet<u32> = num.lits.iter().map(|&l| var(l)).collect();
        let bits: Vec<i32> = clause.iter().copied().filter(|&l| own.contains(&var(l))).collect();
        let rest: Vec<i32> = clause.iter().copied().filter(|&l| !own.contains(&var(l))).collect();
        let seq: Vec<i32> = if bits.is_empty() {
            num.lits.clone()
        } else if bits.len() != 1 {
            return false;
        } else if let Some(&k) = pos.get(&bits[0]) {
            num.lits[..k].to_vec()
        } else if let Some(&k) = pos.get(&-bits[0]) {
            (k + 1..num.n()).rev().map(|j| -num.lits[j]).collect()
        } else {
            return false;
        };
        let mut c: Vec<i32> = Vec::with_capacity(rest.len() + 1);
        let mut neg: Vec<i32> = Vec::with_capacity(rest.len() + 1);
        for lit in seq {
            c.clear();
            c.extend_from_slice(&rest);
            c.push(lit);
            neg.clear();
            neg.extend(c.iter().map(|&l| -l));
            if self.spent() || !self.p.refutes(&neg) {
                return false;
            }
            self.put(&c);
            self.counts.steps += 1;
            if self.halt {
                return false;
            }
        }
        neg.clear();
        neg.extend(clause.iter().map(|&l| -l));
        self.p.refutes(&neg)
    }

    // -------------------------------------------------------------------
    /// The bit test. Drop every member that propagation sets once the
    /// members below it are on and the members above it are off; what it
    /// was set to goes into the clauses (a value between two bits is dead).
    fn prune(&mut self, lits: &[i32]) -> Option<Vec<i32>> {
        let mut lits = lits.to_vec();
        let mut assumed: Vec<i32> = Vec::new();
        while lits.len() >= 2 {
            let mut fail: Vec<(usize, i8)> = Vec::new();
            for k in 0..lits.len() {
                assumed.clear();
                assumed.extend_from_slice(&lits[..k]);
                assumed.extend(lits[k + 1..].iter().map(|&l| -l));
                let mark = self.p.trail.len();
                let ok = self.p.assume(&assumed);
                if !ok {
                    fail.push((k, 0));
                } else {
                    match self.p.value(lits[k]) {
                        1 => fail.push((k, 1)),
                        -1 => fail.push((k, -1)),
                        _ => {}
                    }
                }
                self.p.undo(mark);
                if self.spent() {
                    return None;
                }
            }
            if fail.is_empty() {
                break;
            }
            // two members can each be set by the other: one of them goes, the
            // same one whichever way the chain is written
            let &(k, how) = fail.iter().max_by_key(|&&(k, _)| var(lits[k])).unwrap();
            let below: Vec<i32> = if k > 0 { vec![-lits[k - 1]] } else { Vec::new() };
            let above: Vec<i32> = if k + 1 < lits.len() { vec![lits[k + 1]] } else { Vec::new() };
            let clause: Vec<i32> = match how {
                0 => below.iter().chain(above.iter()).copied().collect(),
                1 => std::iter::once(lits[k]).chain(if below.is_empty() { above } else { below }).collect(),
                _ => std::iter::once(-lits[k]).chain(if above.is_empty() { below } else { above }).collect(),
            };
            if !clause.is_empty() {
                self.add(LinkKind::Dead, &clause, None);
            }
            lits.remove(k);
            self.counts.not_a_bit += 1;
            if self.halt {
                return None;
            }
        }
        if lits.len() >= 2 {
            Some(lits)
        } else {
            None
        }
    }

    /// Set the chain to each value in turn and note what propagation sets.
    fn read(&mut self, lits: &[i32]) -> Number {
        let mut num = Number { lits: lits.to_vec(), dead: 0, readers: HashMap::new(), unread: false };
        if num.n() > MAX_BITS {
            self.counts.too_long += 1;
            num.unread = true;
            return num;
        }
        let own: HashSet<u32> = lits.iter().map(|&l| var(l)).collect();
        for v in 0..=num.n() {
            let pat = num.pattern(v);
            let mark = self.p.trail.len();
            if !self.p.assume(&pat) {
                num.dead |= 1 << v;
            } else {
                for &l in &self.p.trail[mark..] {
                    if own.contains(&var(l)) {
                        continue;
                    }
                    let e = num.readers.entry(var(l)).or_insert((0, 0));
                    if l > 0 {
                        e.0 |= 1 << v;
                    } else {
                        e.1 |= 1 << v;
                    }
                }
            }
            self.p.undo(mark);
            if self.spent() {
                break;
            }
        }
        num
    }

    /// The end test: an end member that nothing reads is no bit. The two
    /// patterns that differ in it alone show it: when propagation sets the
    /// same variables to the same values in both, the member goes.
    fn trim(&mut self, mut num: Number) -> Number {
        while num.n() > 2 && !num.unread && !self.halt {
            let n = num.n();
            let mut cut: Option<bool> = None; // Some(true): the lowest member, Some(false): the highest
            for (low, a, b) in [(true, 0usize, 1usize), (false, n - 1, n)] {
                if num.dead >> a & 1 == 1 || num.dead >> b & 1 == 1 {
                    continue;
                }
                let same = num.readers.values().all(|&(t, f)| (t >> a & 1, f >> a & 1) == (t >> b & 1, f >> b & 1));
                if same {
                    cut = Some(low);
                    break;
                }
            }
            match cut {
                Some(low) => {
                    let shorter: Vec<i32> = if low { num.lits[1..].to_vec() } else { num.lits[..n - 1].to_vec() };
                    num = self.read(&shorter);
                    self.counts.end_dropped += 1;
                }
                None => break,
            }
        }
        num
    }

    // -------------------------------------------------------------------
    fn link_numbers(&mut self) {
        // (rank, number index, variable, on at, off at, kind)
        let mut todo: Vec<(u8, usize, u32, u64, u64, Kind)> = Vec::new();
        let numbers = std::mem::take(&mut self.numbers);
        for (i, num) in numbers.iter().enumerate() {
            let live = num.live();
            if live.len() < 2 {
                continue;
            }
            let n = num.n();
            for v in 0..=n {
                if num.dead >> v & 1 == 0 {
                    continue;
                }
                let clause: Vec<i32> = if v > 0 && v < n {
                    vec![-num.lits[v - 1], num.lits[v]]
                } else if v == 0 {
                    vec![num.lits[0]]
                } else {
                    vec![-num.lits[n - 1]]
                };
                self.add(LinkKind::Dead, &clause, Some(num));
            }
            let mut vars: Vec<u32> = num.readers.keys().copied().collect();
            vars.sort_unstable();
            for w in vars {
                let (t, f) = num.readers[&w];
                let k = kind_of(t, f, &live);
                let rank = match k {
                    Kind::Ge | Kind::Le => 0,
                    Kind::Partial => 2,
                    _ => 1,
                };
                todo.push((rank, i, w, t, f, k));
            }
        }
        for rank in 0..2u8 {
            for &(r, i, w, t, f, k) in &todo {
                if r != rank {
                    continue;
                }
                if self.spent() {
                    break;
                }
                let num = &numbers[i];
                let live = num.live();
                let w = w as i32;
                if rank == 0 {
                    // w equals a bit: on from the first value where a pattern sets it true
                    let ts: Vec<usize> = live.iter().copied().filter(|&v| t >> v & 1 == 1).collect();
                    let bit = if k == Kind::Ge { num.lits[ts[0] - 1] } else { -num.lits[*ts.last().unwrap()] };
                    self.add(LinkKind::Equal, &[-w, bit], Some(num));
                    self.add(LinkKind::Equal, &[w, -bit], Some(num));
                    continue;
                }
                for (sign, forbidden) in [(w, f), (-w, t)] {
                    let allowed: Vec<usize> = live.iter().copied().filter(|&v| forbidden >> v & 1 == 0).collect();
                    if allowed.len() == live.len() {
                        continue;
                    }
                    if allowed.is_empty() {
                        self.add(LinkKind::Unit, &[-sign], Some(num));
                        continue;
                    }
                    let (lo, hi) = (allowed[0], *allowed.last().unwrap());
                    if lo > live[0] {
                        self.add(LinkKind::Bound, &[-sign, num.lits[lo - 1]], Some(num));
                    }
                    if hi < *live.last().unwrap() {
                        self.add(LinkKind::Bound, &[-sign, -num.lits[hi]], Some(num));
                    }
                    // a value ruled out in the middle is a clause of three literals: counted, not added
                    self.counts.holes += live.iter().filter(|&&v| lo < v && v < hi && forbidden >> v & 1 == 1).count() as u64;
                }
            }
        }
        self.counts.partial = todo.iter().filter(|x| x.0 == 2).count() as u64;
        self.numbers = numbers;
    }
}

// ---------------------------------------------------------------------------
// gates by shape, and who is free
// ---------------------------------------------------------------------------

struct Scan {
    /// literal index -> the literals it implies by one two-literal clause
    /// (after the root units), sorted
    imp: Vec<Vec<i32>>,
    /// per variable: the literal of its class of equal variables that names
    /// the class (the variable itself when it has no equal)
    rep: Vec<i32>,
    defined: Vec<bool>,
    gates: usize,
    ring: usize,
    work: u64,
}

/// The clause after the root units: None when one of its literals is true.
fn reduce(c: &[i32], val: &[i8], out: &mut Vec<i32>) -> bool {
    out.clear();
    for &l in c {
        let v = val[l.unsigned_abs() as usize];
        let v = if l > 0 { v } else { -v };
        if v == 1 {
            return false;
        }
        if v == 0 {
            out.push(l);
        }
    }
    true
}

/// `None` when the work limit or the stop flag ends it: `clock` then holds
/// the work spent. The limit is polled inside every loop, and the clock is
/// written at every poll (a signal in the middle reports the work so far).
fn scan(f: &Formula, val: &[i8], start: u64, budget: u64, stop: &AtomicBool, clock: &mut u64) -> Option<Scan> {
    let nv = f.vars as usize;
    let nl = 2 * (nv + 1);
    let mut work = 0u64;
    macro_rules! poll {
        () => {
            *clock = start + work;
            if *clock > budget || stop.load(Ordering::Relaxed) {
                return None;
            }
        };
    }
    let mut imp: Vec<Vec<i32>> = vec![Vec::new(); nl];
    let mut keep: Vec<i32> = Vec::new();
    for i in 0..f.len() {
        if i % 4096 == 0 {
            poll!();
        }
        if f.taut[i] {
            continue;
        }
        let c = f.clause(i);
        work += c.len() as u64 + 1;
        if reduce(c, val, &mut keep) && keep.len() == 2 {
            imp[li(-keep[0])].push(keep[1]);
            imp[li(-keep[1])].push(keep[0]);
        }
    }
    for (k, list) in imp.iter_mut().enumerate() {
        if k % 4096 == 0 {
            poll!();
        }
        work += list.len() as u64;
        list.sort_unstable();
        list.dedup();
    }
    // equal variables, with sign
    let mut parent: Vec<u32> = (0..=nv as u32).collect();
    let mut psign: Vec<i8> = vec![1; nv + 1];
    fn find(parent: &mut [u32], psign: &mut [i8], v: u32) -> (u32, i8) {
        let mut cur = v;
        let mut sign = 1i8;
        let mut path: Vec<(u32, i8)> = Vec::new();
        while parent[cur as usize] != cur {
            path.push((cur, sign));
            sign *= psign[cur as usize];
            cur = parent[cur as usize];
        }
        for (u, before) in path {
            parent[u as usize] = cur;
            psign[u as usize] = sign * before;
        }
        (cur, sign)
    }
    for a_idx in 0..nl {
        if a_idx % 4096 == 0 {
            poll!();
        }
        let a: i32 = if a_idx & 1 == 0 { (a_idx >> 1) as i32 } else { -((a_idx >> 1) as i32) };
        if a == 0 {
            continue;
        }
        for k in 0..imp[a_idx].len() {
            let b = imp[a_idx][k];
            work += 1;
            if var(a) < var(b) && imp[li(b)].binary_search(&a).is_ok() {
                let (ra, sa) = find(&mut parent, &mut psign, var(a));
                let (rb, sb) = find(&mut parent, &mut psign, var(b));
                if ra == rb {
                    continue;
                }
                let rel = (if a > 0 { 1i8 } else { -1 }) * (if b > 0 { 1 } else { -1 }) * sa * sb;
                let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
                parent[hi as usize] = lo;
                psign[hi as usize] = rel;
            }
        }
    }
    let mut rep: Vec<i32> = vec![0; nv + 1];
    for v in 1..=nv as u32 {
        let (r, s) = find(&mut parent, &mut psign, v);
        rep[v as usize] = r as i32 * s as i32;
    }
    let cls = |l: i32| -> u32 { rep[l.unsigned_abs() as usize].unsigned_abs() };
    // every way a variable could be a gate's output: (its class, the classes it reads)
    let mut could: Vec<(u32, Vec<u32>)> = Vec::new();
    let mut three: HashMap<[u32; 3], u8> = HashMap::new();
    for i in 0..f.len() {
        if i % 1024 == 0 {
            poll!();
        }
        if f.taut[i] {
            continue;
        }
        let c = f.clause(i);
        if c.len() < 3 {
            continue;
        }
        work += c.len() as u64;
        if !reduce(c, val, &mut keep) || keep.len() < 3 {
            continue;
        }
        if keep.len() == 3 {
            let mut srt = [keep[0], keep[1], keep[2]];
            srt.sort_unstable_by_key(|l| l.unsigned_abs());
            let key = [var(srt[0]), var(srt[1]), var(srt[2])];
            let pat = (srt[0] > 0) as u8 | ((srt[1] > 0) as u8) << 1 | ((srt[2] > 0) as u8) << 2;
            *three.entry(key).or_insert(0) |= 1 << pat;
        }
        let need = keep.len() - 1;
        for &t in &keep {
            let back = &imp[li(t)];
            if back.len() < need {
                continue;
            }
            // a wide clause whose every literal has many two-literal clauses
            // costs its width squared here: counted and polled a literal
            work += need as u64;
            poll!();
            if keep.iter().all(|&l| l == t || back.binary_search(&-l).is_ok()) {
                let out = cls(t);
                let mut ins: Vec<u32> = keep.iter().filter(|&&l| l != t).map(|&l| cls(l)).filter(|&w| w != out).collect();
                ins.sort_unstable();
                ins.dedup();
                could.push((out, ins));
            }
        }
    }
    const ODD: u8 = 1 << 7 | 1 << 1 | 1 << 2 | 1 << 4;
    const EVEN: u8 = 1 << 0 | 1 << 6 | 1 << 5 | 1 << 3;
    let mut xors: Vec<[u32; 3]> = three.iter().filter(|(_k, &m)| m & ODD == ODD || m & EVEN == EVEN).map(|(k, _m)| *k).collect();
    xors.sort_unstable();
    work += 4 * xors.len() as u64 + three.len() as u64;
    poll!();
    for vs in xors {
        let cs = [cls(vs[0] as i32), cls(vs[1] as i32), cls(vs[2] as i32)];
        for k in 0..3 {
            let mut ins: Vec<u32> = (0..3).filter(|&j| j != k).map(|j| cs[j]).filter(|&w| w != cs[k]).collect();
            ins.sort_unstable();
            ins.dedup();
            could.push((cs[k], ins));
        }
    }
    // settle forward from the variables no shape could define
    let mut has: Vec<bool> = vec![false; nv + 1];
    for (out, _ins) in &could {
        has[*out as usize] = true;
    }
    let mut defined: Vec<bool> = vec![false; nv + 1];
    let mut left: Vec<u32> = vec![0; could.len()];
    let mut waiting: Vec<Vec<u32>> = vec![Vec::new(); nv + 1];
    let mut ready: Vec<u32> = Vec::new();
    for (d, (out, ins)) in could.iter().enumerate() {
        if d % 4096 == 0 {
            poll!();
        }
        let open: Vec<u32> = ins.iter().copied().filter(|&w| has[w as usize]).collect();
        work += ins.len() as u64 + 1;
        if open.is_empty() {
            ready.push(*out);
        } else {
            left[d] = open.len() as u32;
            for w in open {
                waiting[w as usize].push(d as u32);
            }
        }
    }
    let mut popped = 0u64;
    while let Some(v) = ready.pop() {
        popped += 1;
        if popped % 4096 == 0 {
            poll!();
        }
        if defined[v as usize] {
            continue;
        }
        defined[v as usize] = true;
        work += waiting[v as usize].len() as u64 + 1;
        for d in std::mem::take(&mut waiting[v as usize]) {
            left[d as usize] -= 1;
            if left[d as usize] == 0 && !defined[could[d as usize].0 as usize] {
                ready.push(could[d as usize].0);
            }
        }
    }
    let gates = defined.iter().filter(|&&d| d).count();
    let ring = has.iter().filter(|&&h| h).count() - gates;
    *clock = start + work;
    Some(Scan { imp, rep, defined, gates, ring, work })
}

// ---------------------------------------------------------------------------
// candidate chains
// ---------------------------------------------------------------------------

/// Maximal paths of direct links, each lowest bit first and in one
/// orientation (a chain and its mirror are one number).
/// `work` counts the literals walked; past `allowance` the search for paths
/// ends with what it has (the flag in the result says so).
fn candidates(direct: &HashMap<i32, Vec<i32>>, pred: &HashMap<i32, Vec<i32>>, many: &mut u64, allowance: u64, work: &mut u64) -> (Vec<Vec<i32>>, bool) {
    let empty: Vec<i32> = Vec::new();
    let mut height: HashMap<i32, u32> = HashMap::new();
    let mut state: HashMap<i32, u8> = HashMap::new();
    let mut roots: Vec<i32> = direct.keys().copied().collect();
    roots.sort_unstable_by_key(|&u| (var(u), u));
    for &root in &roots {
        if height.contains_key(&root) {
            continue;
        }
        let mut stack: Vec<(i32, usize)> = vec![(root, 0)];
        state.insert(root, 1);
        while let Some(&mut (x, ref mut at)) = stack.last_mut() {
            *work += 1;
            if *work > allowance {
                return (Vec::new(), true);
            }
            let kids = direct.get(&x).unwrap_or(&empty);
            let mut next: Option<i32> = None;
            while *at < kids.len() {
                let w = kids[*at];
                *at += 1;
                if height.contains_key(&w) || state.get(&w) == Some(&1) {
                    continue;
                }
                next = Some(w);
                break;
            }
            match next {
                Some(w) => {
                    state.insert(w, 1);
                    stack.push((w, 0));
                }
                None => {
                    let h = 1 + kids.iter().filter_map(|w| height.get(w)).copied().max().unwrap_or(0);
                    height.insert(x, h);
                    state.insert(x, 2);
                    stack.pop();
                }
            }
        }
    }
    let h_of = |u: i32| -> u32 { height.get(&u).copied().unwrap_or(1) };
    let mut out: Vec<Vec<i32>> = Vec::new();
    let mut have: HashSet<Vec<i32>> = HashSet::new();
    let mut emit = |path: &[i32], out: &mut Vec<Vec<i32>>| {
        // an end that two chains share is a guard on both and a bit of neither
        let mut path = path.to_vec();
        while path.len() >= 2 && direct.get(&path[0]).unwrap_or(&empty).iter().filter(|w| direct.get(w).map_or(false, |d| !d.is_empty())).count() >= 2 {
            path.remove(0);
        }
        while path.len() >= 2 && pred.get(path.last().unwrap()).unwrap_or(&empty).iter().filter(|u| pred.get(u).map_or(false, |d| !d.is_empty())).count() >= 2 {
            path.pop();
        }
        if path.len() < MIN_MEMBERS {
            return;
        }
        let mut lits: Vec<i32> = path.iter().rev().copied().collect();
        let neg = lits.iter().filter(|&&l| l < 0).count();
        if 2 * neg > lits.len() || (2 * neg == lits.len() && lits[0] < 0) {
            lits = lits.iter().rev().map(|&l| -l).collect();
        }
        if have.insert(lits.clone()) {
            out.push(lits);
        }
    };
    let mut sources: Vec<i32> = direct.keys().copied().filter(|u| pred.get(u).map_or(true, |p| p.is_empty())).collect();
    sources.sort_unstable_by_key(|&u| (std::cmp::Reverse(h_of(u)), var(u), u));
    for src in sources {
        if *work > allowance {
            return (out, true);
        }
        let mut count = 0usize;
        if out.len() < TOTAL_PATHS {
            let mut stack: Vec<Vec<i32>> = vec![vec![src]];
            while let Some(path) = stack.pop() {
                *work += path.len() as u64 + 1;
                if count > PER_SOURCE || *work > allowance {
                    break;
                }
                let mut nxt: Vec<i32> = direct.get(path.last().unwrap()).unwrap_or(&empty).iter().copied().filter(|w| !path.contains(w)).collect();
                if nxt.is_empty() {
                    emit(&path, &mut out);
                    count += 1;
                    continue;
                }
                nxt.sort_unstable_by_key(|&w| (std::cmp::Reverse(h_of(w)), var(w), w));
                for &w in nxt.iter().rev() {
                    let mut p2 = path.clone();
                    p2.push(w);
                    stack.push(p2);
                }
            }
            if count <= PER_SOURCE {
                continue;
            }
            *many += 1;
        }
        // past a limit: the longest path from here
        let mut path = vec![src];
        loop {
            *work += path.len() as u64 + 1;
            if *work > allowance {
                return (out, true);
            }
            let nxt: Vec<i32> = direct.get(path.last().unwrap()).unwrap_or(&empty).iter().copied().filter(|w| !path.contains(w)).collect();
            match nxt.iter().copied().max_by_key(|&w| (h_of(w), std::cmp::Reverse(var(w)), w)) {
                Some(w) => path.push(w),
                None => break,
            }
        }
        emit(&path, &mut out);
    }
    (out, false)
}

fn rank_key(t: &[i32]) -> (std::cmp::Reverse<usize>, u32, Vec<i32>) {
    (std::cmp::Reverse(t.len()), t.iter().map(|&l| var(l)).min().unwrap_or(0), t.to_vec())
}

// ---------------------------------------------------------------------------
// the pass
// ---------------------------------------------------------------------------

pub fn run(f: &Formula, derived: &mut Derived, budget: u64, ticks: &mut u64, stop: &AtomicBool) {
    let base = *ticks;
    let size: u64 = f.lits.len() as u64 + f.len() as u64;
    // building the propagator walks the formula once, in one go: without
    // room for that (and for a first look at the two-literal clauses)
    // nothing is started. Everything after it is polled as it runs
    if base.saturating_add(size.saturating_mul(2)) > budget || stop.load(Ordering::Relaxed) {
        println!("c numbers status=no-budget");
        derived.notes.push("numbers: not read, no work left for it".to_string());
        return;
    }
    let p = Prop::new(f);
    *ticks = base + p.steps;
    let mut rd = Reader { p, derived, budget, base, ticks, stop, halt: false, counts: Counts::default(), numbers: Vec::new() };
    if rd.p.conflict {
        println!("c numbers status=root-conflict work={}", rd.p.steps);
        rd.derived.notes.push(format!("numbers: not read, the units refute the formula, work {}K", rd.p.steps / 1000));
        return;
    }
    let sc = match scan(f, &rd.p.val, base + rd.p.steps, budget, stop, rd.ticks) {
        Some(sc) => sc,
        None => {
            let work = *rd.ticks - base;
            println!("c numbers status=stopped numbers=0 added=0 work={}", work);
            rd.derived.notes.push(format!("numbers: not read, stopped at its limit while looking for gates, work {}K", work / 1000));
            return;
        }
    };
    rd.p.steps += sc.work;
    rd.counts.gates = sc.gates;
    rd.counts.ring = sc.ring;
    let free = |v: u32, rd: &Reader| -> bool { rd.p.val[v as usize] == 0 && sc.rep[v as usize] == v as i32 && !sc.defined[v as usize] };
    rd.counts.free = (1..=f.vars as u32).filter(|&v| free(v, &rd)).count();
    let lit_cls = |l: i32| -> i32 {
        let r = sc.rep[l.unsigned_abs() as usize];
        if l > 0 {
            r
        } else {
            -r
        }
    };
    // links between free variables
    let mut edges: HashMap<i32, Vec<i32>> = HashMap::new();
    if !rd.spent() {
        for a_idx in 2..sc.imp.len() {
            if a_idx % 4096 == 0 && rd.spent() {
                break;
            }
            if sc.imp[a_idx].is_empty() {
                continue;
            }
            let a: i32 = if a_idx & 1 == 0 { (a_idx >> 1) as i32 } else { -((a_idx >> 1) as i32) };
            let ra = lit_cls(a);
            if !free(var(ra), &rd) {
                continue;
            }
            rd.p.steps += sc.imp[a_idx].len() as u64;
            for &b in &sc.imp[a_idx] {
                let rb = lit_cls(b);
                if var(ra) != var(rb) && free(var(rb), &rd) {
                    edges.entry(ra).or_default().push(rb);
                }
            }
        }
        for list in edges.values_mut() {
            list.sort_unstable();
            list.dedup();
        }
    }
    // direct links only: drop u -> w when u -> x -> w
    let mut direct: HashMap<i32, Vec<i32>> = HashMap::new();
    let mut pred: HashMap<i32, Vec<i32>> = HashMap::new();
    let mut keys: Vec<i32> = edges.keys().copied().collect();
    keys.sort_unstable();
    for u in keys {
        if rd.spent() {
            break;
        }
        let outs = &edges[&u];
        let mut far: HashSet<i32> = HashSet::new();
        for x in outs {
            if let Some(sx) = edges.get(x) {
                rd.p.steps += sx.len() as u64;
                far.extend(sx.iter().copied());
            }
        }
        let d: Vec<i32> = outs.iter().copied().filter(|w| !far.contains(w)).collect();
        if !d.is_empty() {
            for &w in &d {
                pred.entry(w).or_default().push(u);
            }
            direct.insert(u, d);
        }
    }
    for list in pred.values_mut() {
        list.sort_unstable();
    }
    let mut many = 0u64;
    let cands = if rd.spent() {
        Vec::new()
    } else {
        let allowance = budget.saturating_sub(base + rd.p.steps);
        let mut walked = 0u64;
        let (cands, cut) = candidates(&direct, &pred, &mut many, allowance, &mut walked);
        rd.p.steps += walked;
        if cut {
            rd.halt = true;
        }
        cands
    };
    rd.spent();
    // the bit test, the end test, one reading a candidate
    let mut seen: Vec<Number> = Vec::new();
    let mut index: HashMap<Vec<i32>, usize> = HashMap::new();
    for lits in &cands {
        if rd.spent() {
            break;
        }
        let lits = match rd.prune(lits) {
            Some(l) => l,
            None => continue,
        };
        if index.contains_key(&lits) {
            continue;
        }
        let first = rd.read(&lits);
        let num = rd.trim(first);
        if rd.halt {
            break;
        }
        if !index.contains_key(&num.lits) {
            index.insert(num.lits.clone(), seen.len());
            seen.push(num);
        }
    }
    rd.counts.candidates = seen.len();
    if !rd.halt {
        // who reads whom
        let full: Vec<HashSet<u32>> = seen
            .iter()
            .map(|num| {
                let live = num.live();
                num.readers.iter().filter(|(_w, &(t, f))| !matches!(kind_of(t, f, &live), Kind::Partial | Kind::Constant)).map(|(&w, _)| w).collect()
            })
            .collect();
        let members: Vec<HashSet<u32>> = seen.iter().map(|num| num.lits.iter().map(|&l| var(l)).collect()).collect();
        let mut by_var: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, m) in members.iter().enumerate() {
            for &v in m {
                by_var.entry(v).or_default().push(i);
            }
        }
        let mut order: Vec<usize> = (0..seen.len()).collect();
        order.sort_by_key(|&i| rank_key(&seen[i].lits));
        let covers = |a: usize, t: usize| -> bool { members[t].iter().all(|v| full[a].contains(v) || members[a].contains(v)) };
        let mut chosen: Vec<usize> = Vec::new();
        for &t in &order {
            // every candidate is looked at for every other: counted, and polled a candidate
            rd.p.steps += seen.len() as u64;
            if rd.spent() {
                break;
            }
            if seen[t].unread || seen[t].live().len() < 2 {
                continue;
            }
            let first = var(seen[t].lits[0]);
            let mut beaten = false;
            for a in 0..seen.len() {
                if a == t || !(members[a].contains(&first) || full[a].contains(&first)) {
                    continue;
                }
                if !covers(a, t) {
                    continue;
                }
                if !covers(t, a) || rank_key(&seen[a].lits) < rank_key(&seen[t].lits) {
                    beaten = true;
                    break;
                }
            }
            if !beaten {
                chosen.push(t);
            }
        }
        // two chains that still share a bit are two readings of one number
        chosen.sort_by_key(|&t| (std::cmp::Reverse(full[t].len()), std::cmp::Reverse(seen[t].lits.len()), seen[t].lits.iter().map(|&l| var(l)).min().unwrap(), seen[t].lits.clone()));
        let mut taken: HashSet<u32> = HashSet::new();
        let mut last: Vec<usize> = Vec::new();
        for t in chosen {
            if members[t].iter().any(|v| taken.contains(v)) {
                continue;
            }
            taken.extend(members[t].iter().copied());
            last.push(t);
        }
        last.sort_by_key(|&t| (seen[t].lits.iter().map(|&l| var(l)).min().unwrap(), seen[t].lits.clone()));
        if !rd.halt {
            rd.numbers = last.into_iter().map(|t| seen[t].clone()).collect();
            rd.counts.numbers = rd.numbers.len();
            rd.counts.counted = rd.numbers.iter().filter(|n| is_counted(n)).count();
            rd.link_numbers();
        }
    }
    let work = rd.p.steps;
    *rd.ticks = base + work;
    let c = &rd.counts;
    let mut values: Vec<(usize, usize)> = Vec::new();
    {
        let mut hist: HashMap<usize, usize> = HashMap::new();
        for num in &rd.numbers {
            *hist.entry(num.n() + 1).or_insert(0) += 1;
        }
        values.extend(hist);
        values.sort_unstable();
    }
    let values_text = if values.is_empty() { "-".to_string() } else { values.iter().map(|(v, k)| format!("{}x{}", v, k)).collect::<Vec<_>>().join(",") };
    let added = c.equal + c.bound + c.dead + c.unit + c.steps;
    let status = if rd.halt { "stopped" } else { "read" };
    println!(
        "c numbers status={} numbers={} values={} counted={} added={} equal={} bound={} dead={} unit={} steps={} there={} noproof={} holes={} partial={} candidates={} free={} gates={} ring={} notbit={} ends={} long={} manypaths={} work={}",
        status, c.numbers, values_text, c.counted, added, c.equal, c.bound, c.dead, c.unit, c.steps, c.have, c.noproof, c.holes, c.partial, c.candidates, c.free, c.gates, c.ring, c.not_a_bit, c.end_dropped, c.too_long, many, work
    );
    if added > 0 || c.numbers > 0 {
        rd.derived.notes.push(format!(
            "numbers: {} chains of sorted bits ({} values x count), {} under a count; {} clauses added ({} equal, {} bound, {} dead, {} unit, {} steps), {} there already, {} without a short proof{}, work {}K",
            c.numbers,
            values_text,
            c.counted,
            added,
            c.equal,
            c.bound,
            c.dead,
            c.unit,
            c.steps,
            c.have,
            c.noproof,
            if rd.halt { ", stopped at its limit" } else { "" },
            work / 1000
        ));
    } else {
        rd.derived.notes.push(format!("numbers: no chain of sorted bits{}, work {}K", if rd.halt { " (stopped at its limit)" } else { "" }, work / 1000));
    }
}
