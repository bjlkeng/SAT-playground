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
//! shape (AND, OR of any width, XOR of two); a bit is a true input, a
//! variable that no shape could define; a chain is a path of two-literal
//! clauses between such variables; and tests by propagation decide what is
//! a bit: the bit test (set a chain's lower members on and its higher ones
//! off around one member: a real bit stays open, a helper is set), the end
//! test (an end member goes when the two patterns that differ in it alone
//! set the same variables: nothing reads it), and who reads whom (a chain
//! that every pattern of another chain sets is a function of that number).
//! Two more rules keep helpers off the ends of a chain:
//!   - Ends that many share. With more than four ends hanging on one
//!     member (a label of the pyramid has hundreds of placements above its
//!     highest bit and below its lowest), none of them is followed: every
//!     pair of them would be a chain of its own over the same bits.
//!   - The stretch under the count. A chain that is not under a count as
//!     it stands is cut to the stretch between its lowest and its highest
//!     step that some variable equals: a member past it is a helper that
//!     only implies the lowest or the highest value.
//! `tools/number_read.py` is the same reading in Python, with the tests on
//! Z3's own toys; `tools/number_pass_check.py` holds this file to it,
//! clause for clause.
//!
//! What is added: units and two-literal clauses only, each true in every
//! model of the input. `equal` (a variable that is on from a value upwards
//! and its bit, both ways), `bound` (a variable to the lowest or highest
//! value it allows), `dead` (a value no model takes), `unit` (a variable
//! every value sets the same way), and the steps of a proof where a clause
//! is not checkable as it stands. Only a number under a count gets clauses
//! (three bits or more, and every step between its values, but one at most,
//! has a variable equal to it), and without one in the formula the pass
//! adds nothing at all: what the tests find on the way is held back.
//!
//! Proof: every added clause is a `rup` line. The clauses are written at
//! the end, in the order they were found: each was shown by propagation
//! over the input and the clauses found before it. A clause that
//! propagation does not refute by itself is proved in steps (the same
//! clause for the values below, or above, one at a time; each step is a
//! `rup` line too); when even that fails the clause is left out.
//!
//! Cost. All work is paid into one clock (`Clock`), which writes the
//! solver's work clock and looks at the limit and the stop flag every few
//! thousand units, inside propagation too. Three things keep the pass cheap
//! where it finds nothing: the search for paths may cost three times the
//! work of making the links (past that: `status=dense`, nothing read); the
//! tests may cost 50M before the first number under a count and 20M more
//! for each one found (past that: `status=partial`, what was read is
//! linked); and `structnumticks` bounds all of it (`status=stopped`).
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
/// More ends than this on one member of a chain, and none of them is read
/// as the next bit (`candidates`). In the catalog of Z3's encodings a real
/// end bit shares its neighbour with two other ends at most.
const END_FAN: usize = 4;
/// What the pass may spend where it finds nothing, so that it stays cheap
/// on the formulas it is not meant for (measured on the 400 cells of 2025,
/// solver README, "The number pass in the solver").
///
/// The search for paths may cost this many times the work of making the
/// links it walks, plus a floor. On the pyramid cells it costs under 0.7
/// times; where groups of variables that exclude each other give millions
/// of paths it ran to the pass's whole limit.
const PATHS_PER_LINK: u64 = 3;
const PATHS_FLOOR: u64 = 1_000_000;
/// The tests of the candidates may cost this much before the first number
/// under a count is found, and this much more for each one found. The
/// pyramid cells need 0.4M to 4M of tests a number under a count; a
/// timetable with 57,000 candidates and none under a count ran to the limit.
const READ_FIRST: u64 = 50_000_000;
const READ_MORE: u64 = 20_000_000;

#[inline]
fn li(l: i32) -> usize {
    ((l.unsigned_abs() as usize) << 1) | (l < 0) as usize
}

#[inline]
fn var(l: i32) -> u32 {
    l.unsigned_abs()
}

// ---------------------------------------------------------------------------
// the pass's work, counted in one place
// ---------------------------------------------------------------------------

/// Every loop of the pass pays its work here. When enough was paid since
/// the last look, the solver's work clock is written (so a run that a
/// signal ends in the middle reports the pass's work) and the limit and
/// the stop flag are looked at.
struct Clock<'a> {
    /// The solver's work clock.
    ticks: &'a mut u64,
    stop: &'a AtomicBool,
    /// The clock's value when the pass started.
    base: u64,
    /// The clock value the pass may reach.
    budget: u64,
    /// The pass's own work so far.
    work: u64,
    /// `work` at the last look.
    looked: u64,
    /// The limit or the stop flag ended the pass. It stays set.
    over: bool,
}

impl<'a> Clock<'a> {
    /// Work between two looks at the limits.
    const STEP: u64 = 4096;

    /// Count `n` units. True when the pass must end.
    #[inline]
    fn pay(&mut self, n: u64) -> bool {
        self.work += n;
        if self.work - self.looked >= Self::STEP {
            self.look()
        } else {
            self.over
        }
    }

    /// Write the clock and look at the limit and the stop flag.
    fn look(&mut self) -> bool {
        self.looked = self.work;
        *self.ticks = self.base.saturating_add(self.work);
        if *self.ticks > self.budget || self.stop.load(Ordering::Relaxed) {
            self.over = true;
        }
        self.over
    }

    /// Is there room for a piece of work of size `n` that is done in one
    /// go (a sort, a table)? Without room the piece is not started.
    fn room(&mut self, n: u64) -> bool {
        !self.look() && self.base.saturating_add(self.work).saturating_add(n) <= self.budget
    }
}

// ---------------------------------------------------------------------------
// unit propagation over the input clauses
// ---------------------------------------------------------------------------

struct Prop<'a> {
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
    /// The pass's work. Propagation pays one per literal taken off the
    /// trail, per two-literal clause of it, per longer clause looked at and
    /// per literal looked at inside one; the limit holds inside it too.
    clock: Clock<'a>,
    /// The loading of the formula or a propagation was cut short at the
    /// limit. What a cut propagation had set is no conflict and not all
    /// that follows: nothing may be concluded from it, and the reading ends
    /// (`Reader::spent`).
    stopped: bool,
}

impl<'a> Prop<'a> {
    /// The clock bounds the work from the first step on: the formula is
    /// walked and its units are propagated here, and that alone can be
    /// long.
    fn new(f: &Formula, clock: Clock<'a>) -> Prop<'a> {
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
            clock,
            stopped: false,
        };
        let mut units: Vec<i32> = Vec::new();
        for i in 0..f.len() {
            let c = f.clause(i);
            if p.clock.pay(c.len() as u64 + 1) {
                // not all of the formula is in: nothing can be read from it
                p.stopped = true;
                return p;
            }
            if f.taut[i] {
                continue;
            }
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
        if self.stopped {
            return true;
        }
        while head < self.trail.len() {
            let l = self.trail[head];
            let idx = li(l);
            if self.clock.pay(1 + self.bins[idx].len() as u64) {
                // out of work in the middle: not a conflict, and not the whole
                // propagation either. The caller sees `stopped` and gives up
                self.stopped = true;
                return true;
            }
            head += 1;
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
            let mut keep: Vec<u32> = Vec::with_capacity(ws.len());
            let mut ok = true;
            let mut j = 0;
            while j < ws.len() {
                if self.clock.pay(1) {
                    // out of work in the middle of the list: the clauses not
                    // looked at stay watched here
                    keep.extend_from_slice(&ws[j..]);
                    self.watch[fi] = keep;
                    self.stopped = true;
                    return true;
                }
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
                let mut cut = false;
                for k in s + 2..e {
                    let m = self.lits[k];
                    if self.clock.pay(1) {
                        // a clause can be as long as the formula: the limit
                        // holds inside it. Nothing of it was changed yet
                        cut = true;
                        break;
                    }
                    if self.value(m) != -1 {
                        self.lits.swap(s + 1, k);
                        self.watch[li(m)].push(ci as u32);
                        moved = true;
                        break;
                    }
                }
                if cut {
                    keep.push(ci as u32);
                    keep.extend_from_slice(&ws[j..]);
                    self.watch[fi] = keep;
                    self.stopped = true;
                    return true;
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

/// The steps of the chain that some variable equals. Step k is the one
/// from value k - 1 to value k (member k - 1 goes on); a variable equals it
/// when it is on from value k upwards and off below, or the other way
/// round, and both values are live.
fn covered_steps(num: &Number, live: &[usize]) -> Vec<usize> {
    let mut steps: Vec<usize> = Vec::new();
    for &(t, f) in num.readers.values() {
        let up = match kind_of(t, f, live) {
            Kind::Ge => t,
            Kind::Le => f,
            _ => continue,
        };
        if let Some(first) = live.iter().copied().find(|&v| up >> v & 1 == 1) {
            if first >= 1 && num.dead >> (first - 1) & 1 == 0 {
                steps.push(first);
            }
        }
    }
    steps.sort_unstable();
    steps.dedup();
    steps
}

/// The mark of an integer under a count: three bits or more, and every cut
/// point between its values, but one at most, has a variable equal to it.
fn is_counted(num: &Number) -> bool {
    let (values, cuts, _tests) = profile_of(num);
    num.n() >= 3 && values >= 2 && cuts + 2 >= values
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
    fan_links: u64,
    core_cut: u64,
    /// The pass's work when each part ended: loading the formula and
    /// setting its units, the look for gates, the links between free
    /// variables, the search for paths. The rest is the tests and the
    /// clauses.
    at_load: u64,
    at_scan: u64,
    at_links: u64,
    at_paths: u64,
}

struct Reader<'a> {
    /// Propagation, and with it the pass's clock (`p.clock`).
    p: Prop<'a>,
    derived: &'a mut Derived,
    halt: bool,
    counts: Counts,
    numbers: Vec<Number>,
    /// The clauses found so far, in order. Each was a `rup` step when it
    /// was found and propagation has it since. They are written to the
    /// proof and given to the solver at the end, all of them and in this
    /// order, and only when a number under a count was read: without one
    /// the pass adds nothing (the bit test finds such clauses on any chain
    /// of implications, a circuit's too).
    found: Vec<Vec<i32>>,
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
    /// Writes the solver's work clock.
    fn spent(&mut self) -> bool {
        if self.p.clock.look() || self.p.stopped {
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
        if self.p.stopped {
            return 2; // nothing is known: not there, not shown
        }
        if every {
            return 0;
        }
        assumed.clear();
        assumed.extend(clause.iter().map(|&l| -l));
        if self.p.refutes(&assumed) && !self.p.stopped {
            1
        } else {
            2
        }
    }

    /// Keep the clause (a `rup` step as things stand) and give it to
    /// propagation. `flush` writes the kept clauses.
    fn put(&mut self, clause: &[i32]) {
        self.found.push(clause.to_vec());
        self.p.add(clause);
    }

    /// Write the kept clauses as `rup` steps, in the order they were
    /// found, and add them to F'. Each was shown by propagation over the
    /// input and the clauses kept before it, which is what the proof holds
    /// at its line. Returns how many were written.
    fn flush(&mut self) -> u64 {
        let mut written = 0u64;
        for clause in std::mem::take(&mut self.found) {
            let mut id = 0;
            if let Some(w) = self.derived.proof.as_mut() {
                match w.rup(&clause) {
                    Ok(i) => id = i,
                    Err(_) => {
                        self.halt = true;
                        break;
                    }
                }
            }
            self.derived.add(&clause, id);
            written += 1;
        }
        written
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
            if self.spent() || !self.p.refutes(&neg) || self.p.stopped {
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
        self.p.refutes(&neg) && !self.p.stopped
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
                let seen = if !ok { Some(0) } else { Some(self.p.value(lits[k])).filter(|&v| v != 0) };
                self.p.undo(mark);
                if self.spent() {
                    return None; // also when this very propagation was cut short: `seen` is not used
                }
                if let Some(how) = seen {
                    fail.push((k, how));
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
            let ok = self.p.assume(&pat);
            if self.p.stopped {
                // cut short: neither "dead" nor "this is all it sets" can be said
                self.p.undo(mark);
                self.spent();
                num.unread = true;
                return num;
            }
            if !ok {
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
                num.unread = true;
                return num;
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
                if self.p.clock.pay(num.readers.len() as u64) {
                    self.halt = true;
                    return num;
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

    /// The number under a count is the stretch of the chain between its
    /// lowest and its highest step that some variable equals. A member past
    /// that stretch is a helper that only implies the lowest or the highest
    /// value. (The pyramid's increasing-order option hangs one on each end
    /// of a label: six bits came back as a chain of eight, nine values with
    /// six steps read by the count, and so not "under a count" at all.) A
    /// count gives every bit back as a threshold, so the bits of the number
    /// are inside the stretch. A chain that is under a count as it stands
    /// is left alone (one step may lack its variable: cutting there would
    /// take a real bit), and so is one of which fewer than three members
    /// would remain.
    fn core(&mut self, mut num: Number) -> Number {
        while !num.unread && num.n() > 3 && !self.halt {
            // the readers are looked at twice a round: paid
            if self.p.clock.pay(2 * num.readers.len() as u64) {
                self.halt = true;
                break;
            }
            if is_counted(&num) {
                break;
            }
            let live = num.live();
            let steps = covered_steps(&num, &live);
            if steps.len() < 2 {
                break;
            }
            let (lo, hi) = (steps[0], *steps.last().unwrap());
            if hi - lo + 1 < 3 || (lo == 1 && hi == num.n()) {
                break;
            }
            let shorter: Vec<i32> = num.lits[lo - 1..hi].to_vec();
            num = self.read(&shorter);
            self.counts.core_cut += 1;
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
            // only an integer under a count gets clauses: every step between
            // its values, but one at most, has a variable equal to it
            if live.len() < 2 || !is_counted(num) {
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
            // the sort of its readers is one piece of work: done only with room for it
            if !self.p.clock.room(num.readers.len() as u64) {
                self.halt = true;
                break;
            }
            self.p.clock.pay(num.readers.len() as u64);
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
    /// Choices written as a binary code, for the census: (how many, their
    /// value tests in all, the widest in bits). A choice is a set of free
    /// variables with three or more value tests, each the AND of all of
    /// them with their signs.
    codes: (usize, usize, usize),
    /// literal index -> the literals it implies by one two-literal clause
    /// (after the root units), sorted
    imp: Vec<Vec<i32>>,
    /// per variable: the literal of its class of equal variables that names
    /// the class (the variable itself when it has no equal)
    rep: Vec<i32>,
    /// A shape could define the variable (as a gate's output), whether or
    /// not the forward settling got to it. A bit of a number is none of
    /// these: no clause set reads as its definition.
    shaped: Vec<bool>,
    gates: usize,
    ring: usize,
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

/// `None` when the work limit or the stop flag ends it. Every loop pays its
/// work as it goes, before any early `continue`; a sort or a table is paid
/// in one piece and not started without room for it. Nothing is paid inside
/// a loop over a hash table: such a loop runs in another order every run,
/// and where the limit cut it would then differ from run to run.
fn scan(f: &Formula, val: &[i8], rooted: bool, clock: &mut Clock) -> Option<Scan> {
    let nv = f.vars as usize;
    let nl = 2 * (nv + 1);
    macro_rules! pay {
        ($n:expr) => {
            if clock.pay(($n) as u64) {
                return None;
            }
        };
    }
    // the tables a literal and a variable (below): one piece
    if !clock.room(4 * nl as u64) {
        return None;
    }
    pay!(4 * nl);
    let mut imp: Vec<Vec<i32>> = vec![Vec::new(); nl];
    let mut keep: Vec<i32> = Vec::new();
    // with no variable set at the root, only the clauses of two literals
    // are clauses of two literals: the others are not walked
    for i in 0..f.len() {
        let c = f.clause(i);
        if !rooted && c.len() != 2 {
            pay!(1);
            continue;
        }
        pay!(c.len() + 1);
        if f.taut[i] {
            continue;
        }
        if reduce(c, val, &mut keep) && keep.len() == 2 {
            imp[li(-keep[0])].push(keep[1]);
            imp[li(-keep[1])].push(keep[0]);
        }
    }
    for list in imp.iter_mut() {
        let n = list.len() as u64;
        if n >= Clock::STEP && !clock.room(n) {
            return None;
        }
        pay!(n + 1);
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
        pay!(1 + imp[a_idx].len());
        let a: i32 = if a_idx & 1 == 0 { (a_idx >> 1) as i32 } else { -((a_idx >> 1) as i32) };
        if a == 0 {
            continue;
        }
        for k in 0..imp[a_idx].len() {
            let b = imp[a_idx][k];
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
        pay!(1);
        let (r, s) = find(&mut parent, &mut psign, v);
        rep[v as usize] = r as i32 * s as i32;
    }
    let cls = |l: i32| -> u32 { rep[l.unsigned_abs() as usize].unsigned_abs() };
    // every way a variable could be a gate's output: (its class, the classes it reads)
    let mut could: Vec<(u32, Vec<u32>)> = Vec::new();
    // the AND and OR gates with their input literals, for the binary codes:
    // the first one found for an output literal
    let mut shaped: Vec<Vec<i32>> = Vec::new();
    let mut shaped_out: HashSet<i32> = HashSet::new();
    let mut three: HashMap<[u32; 3], u8> = HashMap::new();
    for i in 0..f.len() {
        let c = f.clause(i);
        if c.len() < 3 {
            pay!(1);
            continue;
        }
        pay!(c.len() + 1);
        if f.taut[i] {
            continue;
        }
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
            // costs up to its width squared here: every look is paid
            let mut looks = 1u64;
            let mut all = true;
            for &l in &keep {
                if l == t {
                    continue;
                }
                looks += 1;
                if back.binary_search(&-l).is_err() {
                    all = false;
                    break;
                }
            }
            pay!(looks);
            if all {
                let out = cls(t);
                let mut ins: Vec<u32> = keep.iter().filter(|&&l| l != t).map(|&l| cls(l)).filter(|&w| w != out).collect();
                ins.sort_unstable();
                ins.dedup();
                could.push((out, ins));
                if shaped_out.insert(-t) {
                    let mut lits: Vec<i32> = keep.iter().copied().filter(|&l| l != t).collect();
                    lits.sort_unstable_by_key(|l| l.unsigned_abs());
                    shaped.push(lits);
                }
            }
        }
    }
    const ODD: u8 = 1 << 7 | 1 << 1 | 1 << 2 | 1 << 4;
    const EVEN: u8 = 1 << 0 | 1 << 6 | 1 << 5 | 1 << 3;
    // the XORs out of the table of three-literal clauses, sorted: one piece
    if !clock.room(6 * three.len() as u64) {
        return None;
    }
    pay!(6 * three.len());
    let mut xors: Vec<[u32; 3]> = three.iter().filter(|(_k, &m)| m & ODD == ODD || m & EVEN == EVEN).map(|(k, _m)| *k).collect();
    xors.sort_unstable();
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
    if !clock.room(4 * nv as u64 + 2 * could.len() as u64) {
        return None;
    }
    pay!(4 * nv + 2 * could.len());
    let mut has: Vec<bool> = vec![false; nv + 1];
    for (out, _ins) in &could {
        has[*out as usize] = true;
    }
    let mut defined: Vec<bool> = vec![false; nv + 1];
    let mut left: Vec<u32> = vec![0; could.len()];
    let mut waiting: Vec<Vec<u32>> = vec![Vec::new(); nv + 1];
    let mut ready: Vec<u32> = Vec::new();
    for (d, (out, ins)) in could.iter().enumerate() {
        pay!(ins.len() + 1);
        let open: Vec<u32> = ins.iter().copied().filter(|&w| has[w as usize]).collect();
        if open.is_empty() {
            ready.push(*out);
        } else {
            left[d] = open.len() as u32;
            for w in open {
                waiting[w as usize].push(d as u32);
            }
        }
    }
    while let Some(v) = ready.pop() {
        pay!(1);
        if defined[v as usize] {
            continue;
        }
        defined[v as usize] = true;
        pay!(waiting[v as usize].len());
        for d in std::mem::take(&mut waiting[v as usize]) {
            left[d as usize] -= 1;
            if left[d as usize] == 0 && !defined[could[d as usize].0 as usize] {
                ready.push(could[d as usize].0);
            }
        }
    }
    let gates = defined.iter().filter(|&&d| d).count();
    let ring = has.iter().filter(|&&h| h).count() - gates;
    // binary codes: the gates over one set of free variables, by their sign patterns
    let mut groups: HashMap<Vec<u32>, Vec<Vec<bool>>> = HashMap::new();
    for lits in shaped.iter() {
        pay!(2 * lits.len() + 1);
        let vs: Vec<u32> = lits.iter().map(|&l| var(l)).collect();
        if vs.windows(2).any(|w| w[0] == w[1]) || vs.iter().any(|&v| val[v as usize] != 0 || defined[rep[v as usize].unsigned_abs() as usize]) {
            // (a coded choice may sit on variables in a ring of shapes: only settled gates are ruled out)
            continue;
        }
        groups.entry(vs).or_default().push(lits.iter().map(|&l| l > 0).collect());
    }
    // (what the loop below looks at was paid above, a literal of each gate)
    let mut codes = (0usize, 0usize, 0usize);
    for (vs, pats) in &groups {
        // three value tests or more, and no two of them on the same code
        let distinct: HashSet<&Vec<bool>> = pats.iter().collect();
        if pats.len() >= 3 && distinct.len() == pats.len() {
            codes.0 += 1;
            codes.1 += pats.len();
            codes.2 = codes.2.max(vs.len());
        }
    }
    Some(Scan { codes, imp, rep, shaped: has, gates, ring })
}

// ---------------------------------------------------------------------------
// candidate chains
// ---------------------------------------------------------------------------

/// What the search for candidates found besides them.
#[derive(Default)]
struct Paths {
    /// Starts with more than PER_SOURCE paths: one longest path was taken.
    many: u64,
    /// Paths left out for their length.
    long: u64,
    /// Links to ends that were not followed (see `candidates`).
    fan_links: u64,
    /// The clock ended the search.
    cut: bool,
    /// The search passed its allowance: too many paths for chains of bits.
    dense: bool,
}

/// Maximal paths of direct links, each lowest bit first and in one
/// orientation (a chain and its mirror are one number).
///
/// Ends that many share are cut off first. A member x of a chain can have
/// many ends hanging on it: literals that lead into x and that nothing
/// leads into, or literals x leads into that lead nowhere. Each of them
/// makes a path of its own over the same members, and with a fan at both
/// ends every pair does. (A label of the pyramid has hundreds of placements
/// above its highest bit, "this placement needs the label at its top
/// value", and hundreds below its lowest one: one chain of six bits came
/// back as 8,425 candidates.) Two ends on one member cannot both be its
/// next bit, and among many none stands out, so past END_FAN of them their
/// links are not followed: the chain is read without them, and they come
/// out as its readers.
///
/// `allowance` is the work the search may cost. Past it the formula has too
/// many paths to be chains of bits, and nothing is read.
fn candidates(direct: &HashMap<i32, Vec<i32>>, pred: &HashMap<i32, Vec<i32>>, allowance: u64, clock: &mut Clock) -> (Vec<Vec<i32>>, Paths) {
    let empty: Vec<i32> = Vec::new();
    let mut info = Paths::default();
    let started = clock.work;
    macro_rules! pay {
        ($n:expr) => {
            if clock.pay(($n) as u64) {
                info.cut = true;
                return (Vec::new(), info);
            }
            if clock.work - started > allowance {
                info.dense = true;
                return (Vec::new(), info);
            }
        };
    }
    // the starts of links and their ends, each list in order: two sorts, one piece each
    if !clock.room(2 * (direct.len() + pred.len()) as u64) {
        info.cut = true;
        return (Vec::new(), info);
    }
    pay!(2 * (direct.len() + pred.len()));
    let mut from: Vec<i32> = direct.keys().copied().collect();
    from.sort_unstable();
    let mut into: Vec<i32> = pred.keys().copied().collect();
    into.sort_unstable();
    let is_source = |u: i32| pred.get(&u).map_or(true, |p| p.is_empty());
    let is_sink = |w: i32| direct.get(&w).map_or(true, |d| d.is_empty());
    // the members a fan was cut from: ends as they stand
    let mut top: HashSet<i32> = HashSet::new();
    let mut bottom: HashSet<i32> = HashSet::new();
    for &x in &into {
        let ps = &pred[&x];
        pay!(1 + ps.len());
        if ps.iter().filter(|&&u| is_source(u)).count() > END_FAN {
            top.insert(x);
        }
    }
    for &u in &from {
        let outs = &direct[&u];
        pay!(1 + outs.len());
        if outs.iter().filter(|&&w| is_sink(w)).count() > END_FAN {
            bottom.insert(u);
        }
    }
    let mut kept_direct: HashMap<i32, Vec<i32>> = HashMap::new();
    let mut kept_pred: HashMap<i32, Vec<i32>> = HashMap::new();
    for &u in &from {
        let outs = &direct[&u];
        pay!(1 + outs.len());
        let source = is_source(u);
        let fan_below = bottom.contains(&u);
        let kept: Vec<i32> = outs.iter().copied().filter(|&w| !(fan_below && is_sink(w)) && !(source && top.contains(&w))).collect();
        info.fan_links += (outs.len() - kept.len()) as u64;
        if !kept.is_empty() {
            for &w in &kept {
                kept_pred.entry(w).or_default().push(u); // in the order of `from`: sorted
            }
            kept_direct.insert(u, kept);
        }
    }
    let direct = &kept_direct;
    let pred = &kept_pred;
    let mut height: HashMap<i32, u32> = HashMap::new();
    let mut state: HashMap<i32, u8> = HashMap::new();
    let mut roots: Vec<i32> = direct.keys().copied().collect();
    pay!(2 * roots.len());
    roots.sort_unstable_by_key(|&u| (var(u), u));
    for &root in &roots {
        if height.contains_key(&root) {
            continue;
        }
        let mut stack: Vec<(i32, usize)> = vec![(root, 0)];
        state.insert(root, 1);
        while let Some(&mut (x, ref mut at)) = stack.last_mut() {
            let kids = direct.get(&x).unwrap_or(&empty);
            let before = *at;
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
            pay!(1 + *at - before);
            match next {
                Some(w) => {
                    state.insert(w, 1);
                    stack.push((w, 0));
                }
                None => {
                    pay!(kids.len());
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
    // the links looked at to judge a path's two ends
    let ends_cost = |path: &[i32]| -> u64 {
        path.len() as u64 + path.iter().map(|x| (direct.get(x).map_or(0, |d| d.len()) + pred.get(x).map_or(0, |d| d.len())) as u64).sum::<u64>()
    };
    let mut emit = |path: &[i32], out: &mut Vec<Vec<i32>>, long: &mut u64| {
        // an end that two chains share is a guard on both and a bit of
        // neither. (A member that a fan was cut from stays: it is an end
        // only because its many ends were taken away, and the tests say
        // whether it is a bit.)
        let (mut a, mut b) = (0usize, path.len());
        while b - a >= 2 && !top.contains(&path[a]) && direct.get(&path[a]).unwrap_or(&empty).iter().filter(|w| direct.get(w).map_or(false, |d| !d.is_empty())).count() >= 2 {
            a += 1;
        }
        while b - a >= 2 && !bottom.contains(&path[b - 1]) && pred.get(&path[b - 1]).unwrap_or(&empty).iter().filter(|u| pred.get(u).map_or(false, |d| !d.is_empty())).count() >= 2 {
            b -= 1;
        }
        let path = &path[a..b];
        if path.len() < MIN_MEMBERS {
            return;
        }
        if path.len() > 4 * MAX_BITS {
            // far past what can be read value by value, even with a few
            // helpers taken out: testing every member would cost its length
            // squared
            *long += 1;
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
    // a member's successors that are not on the path, tallest first, and what finding them cost
    let sorted_kids = |x: i32, on: &HashSet<i32>| -> (Vec<i32>, u64) {
        let all = direct.get(&x).unwrap_or(&empty);
        let mut nxt: Vec<i32> = all.iter().copied().filter(|w| !on.contains(w)).collect();
        nxt.sort_unstable_by_key(|&w| (std::cmp::Reverse(h_of(w)), var(w), w));
        (nxt, 1 + all.len() as u64)
    };
    let mut sources: Vec<i32> = direct.keys().copied().filter(|u| pred.get(u).map_or(true, |p| p.is_empty())).collect();
    pay!(2 * sources.len());
    sources.sort_unstable_by_key(|&u| (std::cmp::Reverse(h_of(u)), var(u), u));
    for src in sources {
        pay!(1);
        let mut count = 0usize;
        if out.len() < TOTAL_PATHS {
            // every maximal path from here, in the order of a walk that takes
            // the tallest successor first; one path is kept and grown in place
            let mut path: Vec<i32> = vec![src];
            let mut on: HashSet<i32> = HashSet::new();
            on.insert(src);
            let (first, cost) = sorted_kids(src, &on);
            pay!(cost);
            let mut stack: Vec<(Vec<i32>, usize)> = vec![(first, 0)];
            while let Some((kids, at)) = stack.last_mut() {
                pay!(1);
                if kids.is_empty() && *at == 0 {
                    // a path's end: nothing below that is not on the path already
                    *at = 1;
                    pay!(ends_cost(&path));
                    emit(&path, &mut out, &mut info.long);
                    count += 1;
                    if count > PER_SOURCE {
                        break;
                    }
                }
                if *at < kids.len() {
                    let w = kids[*at];
                    *at += 1;
                    path.push(w);
                    on.insert(w);
                    let (next, cost) = sorted_kids(w, &on);
                    pay!(cost);
                    stack.push((next, 0));
                } else {
                    stack.pop();
                    if let Some(w) = path.pop() {
                        on.remove(&w);
                    }
                }
            }
            if count <= PER_SOURCE {
                continue;
            }
            info.many += 1;
        }
        // past a limit: the longest path from here
        let mut path = vec![src];
        let mut on: HashSet<i32> = HashSet::new();
        on.insert(src);
        loop {
            let all = direct.get(path.last().unwrap()).unwrap_or(&empty);
            pay!(1 + all.len());
            let nxt: Vec<i32> = all.iter().copied().filter(|w| !on.contains(w)).collect();
            match nxt.iter().copied().max_by_key(|&w| (h_of(w), std::cmp::Reverse(var(w)), w)) {
                Some(w) => {
                    path.push(w);
                    on.insert(w);
                }
                None => break,
            }
        }
        pay!(ends_cost(&path));
        emit(&path, &mut out, &mut info.long);
    }
    (out, info)
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
    // building the propagator walks the formula, and so does the first look
    // at its two-literal clauses: without room for both nothing is started.
    // Everything after that pays as it runs
    if base.saturating_add(size.saturating_mul(2)) > budget || stop.load(Ordering::Relaxed) {
        println!("c numbers status=no-budget");
        derived.notes.push("numbers: not read, no work left for it".to_string());
        return;
    }
    let clock = Clock { ticks, stop, base, budget, work: 0, looked: 0, over: false };
    let p = Prop::new(f, clock);
    let mut rd = Reader { p, derived, halt: false, counts: Counts::default(), numbers: Vec::new(), found: Vec::new() };
    rd.p.clock.look();
    rd.counts.at_load = rd.p.clock.work;
    if rd.p.stopped {
        // the formula's own clauses and units alone took the pass to its limit
        println!("c numbers status=stopped numbers=0 added=0 wload={} work={}", rd.p.clock.work, rd.p.clock.work);
        rd.derived.notes.push(format!("numbers: not read, stopped at its limit while setting the formula's units, work {}K", rd.p.clock.work / 1000));
        return;
    }
    if rd.p.conflict {
        println!("c numbers status=root-conflict work={}", rd.p.clock.work);
        rd.derived.notes.push(format!("numbers: not read, the units refute the formula, work {}K", rd.p.clock.work / 1000));
        return;
    }
    let rooted = rd.p.root > 0;
    let sc = match scan(f, &rd.p.val, rooted, &mut rd.p.clock) {
        Some(sc) => sc,
        None => {
            rd.p.clock.look();
            let work = rd.p.clock.work;
            println!("c numbers status=stopped numbers=0 added=0 wload={} wscan={} work={}", rd.counts.at_load, work - rd.counts.at_load, work);
            rd.derived.notes.push(format!("numbers: not read, stopped at its limit while looking for gates, work {}K", work / 1000));
            return;
        }
    };
    rd.counts.gates = sc.gates;
    rd.counts.ring = sc.ring;
    rd.counts.at_scan = rd.p.clock.work;
    let codes = sc.codes;
    // a bit is a true input: open, the one name of its class, and no shape
    // could define it (a variable in a ring of shapes, such as a member of
    // an exactly-one group, is left out: thousands of them imply each other)
    let free = |v: u32, rd: &Reader| -> bool { rd.p.val[v as usize] == 0 && sc.rep[v as usize] == v as i32 && !sc.shaped[v as usize] };
    if !rd.p.clock.pay(f.vars as u64) {
        rd.counts.free = (1..=f.vars as u32).filter(|&v| free(v, &rd)).count();
    }
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
            if rd.p.clock.pay(1 + sc.imp[a_idx].len() as u64) {
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
            for &b in &sc.imp[a_idx] {
                let rb = lit_cls(b);
                if var(ra) != var(rb) && free(var(rb), &rd) {
                    edges.entry(ra).or_default().push(rb);
                }
            }
        }
        // each list sorted: paid in one piece (the table is walked in no fixed order)
        let total: u64 = edges.values().map(|l| l.len() as u64 + 1).sum();
        if !rd.spent() && rd.p.clock.room(total) {
            rd.p.clock.pay(total);
            for list in edges.values_mut() {
                list.sort_unstable();
                list.dedup();
            }
        } else {
            rd.halt = true;
        }
    }
    // direct links only: drop u -> w when u -> x -> w
    let mut direct: HashMap<i32, Vec<i32>> = HashMap::new();
    let mut pred: HashMap<i32, Vec<i32>> = HashMap::new();
    if !rd.spent() && rd.p.clock.room(2 * edges.len() as u64) {
        rd.p.clock.pay(2 * edges.len() as u64);
        let mut keys: Vec<i32> = edges.keys().copied().collect();
        keys.sort_unstable();
        'keys: for u in keys {
            let outs = &edges[&u];
            if rd.p.clock.pay(1 + outs.len() as u64) {
                break;
            }
            let mut far: HashSet<i32> = HashSet::new();
            for x in outs {
                if let Some(sx) = edges.get(x) {
                    if rd.p.clock.pay(sx.len() as u64) {
                        break 'keys;
                    }
                    far.extend(sx.iter().copied());
                }
            }
            let d: Vec<i32> = outs.iter().copied().filter(|w| !far.contains(w)).collect();
            if !d.is_empty() {
                for &w in &d {
                    pred.entry(w).or_default().push(u); // in the order of the keys: sorted
                }
                direct.insert(u, d);
            }
        }
    }
    rd.counts.at_links = rd.p.clock.work;
    let mut many = 0u64;
    // too many paths for chains of bits: nothing is read
    let mut dense = false;
    let cands = if rd.spent() {
        Vec::new()
    } else {
        let allowance = PATHS_PER_LINK.saturating_mul(rd.counts.at_links - rd.counts.at_scan).saturating_add(PATHS_FLOOR);
        let (cands, info) = candidates(&direct, &pred, allowance, &mut rd.p.clock);
        many = info.many;
        rd.counts.too_long += info.long;
        rd.counts.fan_links = info.fan_links;
        if info.cut {
            rd.halt = true;
        }
        dense = info.dense;
        cands
    };
    rd.spent();
    rd.counts.at_paths = rd.p.clock.work;
    // the bit test, the end test, one reading a candidate. The tests may
    // cost READ_FIRST before the first number under a count and READ_MORE
    // for each one found; past that the candidates left are not tested
    let mut seen: Vec<Number> = Vec::new();
    let mut index: HashMap<Vec<i32>, usize> = HashMap::new();
    // the chains read already, under a count or not: one reading each
    let mut tried: HashSet<Vec<i32>> = HashSet::new();
    let mut partial = false;
    let mut counted_seen = 0u64;
    let mut tested = 0usize;
    for lits in &cands {
        if rd.spent() {
            break;
        }
        if rd.p.clock.work - rd.counts.at_paths > READ_FIRST.saturating_add(READ_MORE.saturating_mul(counted_seen)) {
            partial = true;
            break;
        }
        let lits = match rd.prune(lits) {
            Some(l) => l,
            None => continue,
        };
        if !tried.insert(lits.clone()) {
            continue;
        }
        let first = rd.read(&lits);
        let trimmed = rd.trim(first);
        let num = rd.core(trimmed);
        if rd.halt {
            break;
        }
        tested += 1;
        // only a number under a count goes on: no other chain gets clauses,
        // and none takes part in the choice between chains that overlap
        // (a timetable has 57,000 chains and no number under a count)
        if rd.p.clock.pay(num.readers.len() as u64) || !is_counted(&num) {
            continue;
        }
        if !index.contains_key(&num.lits) {
            counted_seen += 1;
            index.insert(num.lits.clone(), seen.len());
            seen.push(num);
        }
    }
    rd.counts.candidates = tested;
    if !rd.spent() {
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
        // The chains that could hold t as a function hold or read its first
        // member. Which chains read a variable is looked up, not searched:
        // with every candidate tried against every other, 57,000 candidates
        // (a timetable) would be three billion looks
        let firsts: HashSet<u32> = seen.iter().map(|num| var(num.lits[0])).collect();
        let mut read_by: HashMap<u32, Vec<usize>> = HashMap::new();
        for (a, readers) in full.iter().enumerate() {
            if rd.p.clock.pay(1 + readers.len() as u64) {
                rd.halt = true;
                break;
            }
            for v in readers {
                if firsts.contains(v) {
                    read_by.entry(*v).or_default().push(a);
                }
            }
        }
        let none: Vec<usize> = Vec::new();
        let mut chosen: Vec<usize> = Vec::new();
        for &t in &order {
            if rd.halt {
                break;
            }
            if seen[t].unread || seen[t].live().len() < 2 {
                continue;
            }
            let first = var(seen[t].lits[0]);
            let holders = by_var.get(&first).unwrap_or(&none);
            let readers = read_by.get(&first).unwrap_or(&none);
            if rd.p.clock.pay(1 + ((holders.len() + readers.len()) * seen[t].lits.len()) as u64) {
                rd.halt = true;
                break;
            }
            let mut beaten = false;
            for &a in holders.iter().chain(readers.iter()) {
                if a == t {
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
        if !rd.spent() {
            rd.numbers = last.into_iter().map(|t| seen[t].clone()).collect();
            rd.counts.numbers = rd.numbers.len();
            rd.counts.counted = rd.numbers.iter().filter(|n| is_counted(n)).count();
            rd.link_numbers();
        }
    }
    // the clauses are written only when a number under a count was read
    let found = rd.found.len() as u64;
    let written = if rd.counts.counted > 0 { rd.flush() } else { 0 };
    let held = found - written;
    if written == 0 {
        let c = &mut rd.counts;
        (c.equal, c.bound, c.dead, c.unit, c.steps) = (0, 0, 0, 0, 0);
    }
    rd.p.clock.look();
    let work = rd.p.clock.work;
    let c = &rd.counts;
    let histogram = |only_counted: bool| -> String {
        let mut hist: HashMap<usize, usize> = HashMap::new();
        for num in rd.numbers.iter().filter(|n| !only_counted || is_counted(n)) {
            *hist.entry(num.n() + 1).or_insert(0) += 1;
        }
        let mut list: Vec<(usize, usize)> = hist.into_iter().collect();
        list.sort_unstable();
        if list.is_empty() {
            "-".to_string()
        } else {
            list.iter().map(|(v, k)| format!("{}x{}", v, k)).collect::<Vec<_>>().join(",")
        }
    };
    let values_text = histogram(false);
    let counted_text = histogram(true);
    let added = written;
    // stopped: at the pass's limit. dense: too many paths, nothing read.
    // partial: the tests ran out of their allowance, what was read is linked
    let status = if rd.halt {
        "stopped"
    } else if dense {
        "dense"
    } else if partial {
        "partial"
    } else {
        "read"
    };
    println!(
        "c numbers status={} numbers={} values={} counted={} countedvalues={} added={} held={} equal={} bound={} dead={} unit={} steps={} there={} noproof={} holes={} partial={} candidates={} free={} gates={} ring={} notbit={} ends={} long={} manypaths={} fanlinks={} corecuts={} codes={} codetests={} codebits={} wload={} wscan={} wlinks={} wpaths={} wread={} work={}",
        status, c.numbers, values_text, c.counted, counted_text, added, held, c.equal, c.bound, c.dead, c.unit, c.steps, c.have, c.noproof, c.holes, c.partial, c.candidates, c.free, c.gates, c.ring, c.not_a_bit, c.end_dropped, c.too_long, many, c.fan_links, c.core_cut, codes.0, codes.1, codes.2, c.at_load, c.at_scan - c.at_load, c.at_links - c.at_scan, c.at_paths - c.at_links, work - c.at_paths, work
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
            match status {
                "stopped" => ", stopped at its limit",
                "partial" => ", not every candidate tested",
                _ => "",
            },
            work / 1000
        ));
    } else {
        rd.derived.notes.push(format!(
            "numbers: no number under a count{}, work {}K",
            match status {
                "stopped" => " (stopped at its limit)",
                "dense" => " (too many paths for chains of bits)",
                "partial" => " (not every candidate tested)",
                _ => "",
            },
            work / 1000
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formula(vars: i32, clauses: &[Vec<i32>]) -> Formula {
        let mut lits = Vec::new();
        let mut start = vec![0u32];
        for c in clauses {
            lits.extend_from_slice(c);
            start.push(lits.len() as u32);
        }
        Formula { lits, start, taut: vec![false; clauses.len()], dup: vec![false; clauses.len()], vars }
    }

    #[test]
    fn the_clock_looks_every_step_of_work_and_keeps_its_verdict() {
        let stop = AtomicBool::new(false);
        let mut ticks = 100u64;
        let mut c = Clock { ticks: &mut ticks, stop: &stop, base: 100, budget: 100 + 3 * Clock::STEP, work: 0, looked: 0, over: false };
        assert!(!c.pay(Clock::STEP - 1));
        assert_eq!(*c.ticks, 100, "no look before a step's worth of work");
        assert!(!c.pay(1));
        assert_eq!(*c.ticks, 100 + Clock::STEP, "the clock is written at the look");
        // room is counted to the limit itself
        assert!(c.room(2 * Clock::STEP));
        assert!(!c.room(2 * Clock::STEP + 1));
        assert!(!c.pay(2 * Clock::STEP));
        assert!(c.pay(Clock::STEP), "past the limit");
        assert!(c.pay(0) && c.over, "and it stays over");
        // the stop flag ends it too, at the next look
        let mut ticks = 0u64;
        let mut c = Clock { ticks: &mut ticks, stop: &stop, base: 0, budget: u64::MAX, work: 0, looked: 0, over: false };
        stop.store(true, Ordering::Relaxed);
        assert!(!c.pay(1));
        assert!(c.pay(Clock::STEP));
    }

    /// One long propagation: x1 -> x2 -> ... -> xn. The clock is written on
    /// the way (a signal in the middle reports the work so far), and the
    /// stop flag ends it within one look.
    #[test]
    fn a_long_propagation_writes_the_clock_and_sees_the_stop_flag() {
        let n = 200_000i32;
        let chain: Vec<Vec<i32>> = (1..n).map(|v| vec![-v, v + 1]).collect();
        let f = formula(n, &chain);
        let stop = AtomicBool::new(false);
        let mut ticks = 7u64;
        let clock = Clock { ticks: &mut ticks, stop: &stop, base: 7, budget: u64::MAX, work: 0, looked: 0, over: false };
        let mut p = Prop::new(&f, clock);
        assert!(!p.stopped && !p.conflict);
        let loaded = p.clock.work;
        assert!(p.assume(&[1]) && !p.stopped);
        assert_eq!(p.trail.len(), n as usize);
        assert!(p.clock.work - loaded >= n as u64);
        assert!(p.clock.base + p.clock.work - *p.clock.ticks <= Clock::STEP, "the clock was last written at most one step of work ago");
        p.undo(0);
        stop.store(true, Ordering::Relaxed);
        let before = p.clock.work;
        assert!(p.assume(&[1]), "a cut propagation claims no conflict");
        assert!(p.stopped);
        assert!(p.clock.work - before <= 2 * Clock::STEP, "ended {} units after the flag went up", p.clock.work - before);
        assert_eq!(*p.clock.ticks, p.clock.base + p.clock.looked);
        assert!(p.trail.len() < n as usize);
    }

    /// The limit holds inside one clause: a clause of 100,000 literals whose
    /// literals are all false but one costs 100,000 looks when its watch
    /// goes. Cut in the middle, the clause is still watched: the same
    /// propagation with room for it sets the last literal.
    #[test]
    fn a_propagation_cut_inside_a_long_clause_leaves_it_watched() {
        let n = 100_000i32;
        let f = formula(n, &[(1..=n).collect::<Vec<i32>>()]);
        let stop = AtomicBool::new(false);
        let mut ticks = 0u64;
        let clock = Clock { ticks: &mut ticks, stop: &stop, base: 0, budget: u64::MAX, work: 0, looked: 0, over: false };
        let mut p = Prop::new(&f, clock);
        let loaded = p.clock.work;
        // x3 .. xn off first (the watches sit on x1 and x2), then x2: one scan of the whole clause
        let mut lits: Vec<i32> = (3..=n).map(|v| -v).collect();
        lits.push(-2);
        p.clock.budget = loaded + n as u64 + n as u64 / 2;
        assert!(p.assume(&lits));
        assert!(p.stopped, "the scan of the clause had to be cut");
        assert!(p.clock.work <= p.clock.budget + Clock::STEP + 2, "work {} past the limit {}", p.clock.work, p.clock.budget);
        assert_eq!(p.value(1), 0, "nothing is concluded from a cut scan");
        p.undo(0);
        p.stopped = false;
        p.clock.over = false;
        p.clock.budget = u64::MAX;
        assert!(p.assume(&lits) && !p.stopped);
        assert_eq!(p.value(1), 1, "with room, the clause sets its last literal");
    }
}
