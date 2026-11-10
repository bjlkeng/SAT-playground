//! Not in kissat. The free-choice reading (bead SAT-playground-d96.2.4.1,
//! G.8.4a; epic SAT-playground-d96.2; solver README "Free choices: census"):
//! the reading of tools/free_census.py, ported to the structure pass's
//! buffered input, so that the solver can search over a formula's real
//! choices (the walk of bead d96.2.4.2).
//!
//! After unit propagation every variable left gets one role:
//! - choice: a member of a group. An exactly-one group is a clause whose
//!   literals exclude each other, by a binary clause for each pair or
//!   through chains of binary clauses. An at-most-one group comes from a
//!   clause (g | m1 | ... | mk), g the only literal of its sign, whose m
//!   exclude each other: "none" is a value too.
//! - def: a two-way definition. A clause (x | m1 | ... | mk) with every
//!   binary (-x | -mi) makes -x the OR of the mi, of any width; or a small
//!   gate of up to four inputs whose clauses fix it for every input value.
//! - helper: a one-way helper. Every clause holding its literal y has all
//!   its other variables computed already, so y is set true exactly when
//!   one of those clauses needs it.
//! - free: anything else, or a variable freed to break a cycle.
//!
//! Roles are settled forward from the groups: a definition or a helper once
//! all its inputs are settled, two-way definitions first. The constraint
//! clauses are the ones no role makes true: what a search over the choices
//! has to satisfy.
//!
//! Every step follows the census, in the same order, so that the counts
//! match its table (benchmarks/rl/free_2025.tsv) cell by cell;
//! tools/free_read_check.py checks that. Where the census had wall-clock
//! limits, this reading has a work budget counted in the work clock: it pays
//! for work before doing it, and gives up as a whole when the budget runs out.

use crate::structure::Formula;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};

/// Roles of a variable.
pub const ABSENT: u8 = 0;
pub const CHOICE: u8 = 2;
pub const FREE: u8 = 3;
pub const DEF: u8 = 4;
pub const GATE: u8 = 5;
pub const HELPER: u8 = 6;

/// What makes a clause true by construction (its cover).
pub const C_NONE: u8 = 0;
pub const C_GROUP: u8 = 1;
pub const C_DEF: u8 = 2;
pub const C_GATE: u8 = 3;
pub const C_HELPER: u8 = 4;

/// A helper's literal sits in clauses with at most this many other literals.
const MAX_HELPER_IN: usize = 64;
/// Small gates: truth tables over up to this many inputs.
const MAX_GATE_SUPPORT: usize = 4;
/// A variable in more short clauses than this is not tried as a small gate.
const MAX_GATE_SHORT: usize = 32;
/// Start clauses tried per variable for a small gate.
const MAX_GATE_STARTS: usize = 6;
/// Literals reached per exclusion walk, beside 8 per group member.
const CHAIN_BUDGET: usize = 256;
/// Longest clause tried as a group whose exclusions run through others.
const MAX_CHAIN_GROUP: usize = 2_048;
/// Literals per walk in the first-pair screen of a group candidate.
const QUICK_BUDGET: usize = 128;

/// Index of a literal: 2v for v, 2v + 1 for -v.
#[inline]
fn li(lit: i32) -> usize {
    if lit > 0 {
        2 * lit as usize
    } else {
        1 + 2 * lit.unsigned_abs() as usize
    }
}

#[inline]
fn var(lit: i32) -> usize {
    lit.unsigned_abs() as usize
}

/// The reading counts its work in units, 16 to a tick, as the grid reader
/// does, so that a tick of the reading takes about as long as a tick of the
/// search: on the 2025 cells the reading does a median 435M units a second
/// and the search 27M ticks a second (bead SAT-playground-d96.2.5). The
/// walk's build counts the same way (src/freewalk.rs).
pub const UNITS_PER_TICK: u64 = 16;

/// The work clock: work is paid for before it is done.
struct Clock<'a> {
    ticks: &'a mut u64,
    budget: u64,
    stop: &'a AtomicBool,
    /// Units paid and not yet a whole tick.
    units: u64,
    last: u64,
}

impl<'a> Clock<'a> {
    fn new(ticks: &'a mut u64, budget: u64, stop: &'a AtomicBool) -> Clock<'a> {
        let last = *ticks;
        Clock { ticks, budget, stop, units: 0, last }
    }

    /// Pay for `n` units of work. None when the limit does not cover them or
    /// the run was told to stop; nothing is charged for work not done.
    #[inline]
    fn pay(&mut self, n: u64) -> Option<()> {
        let u = self.units.saturating_add(n);
        let t = self.ticks.saturating_add(u / UNITS_PER_TICK);
        if t > self.budget {
            return None;
        }
        *self.ticks = t;
        self.units = u % UNITS_PER_TICK;
        // the stop flag once every 1,024 units
        if t - self.last >= 1024 / UNITS_PER_TICK {
            self.last = t;
            if self.stop.load(Ordering::Relaxed) {
                return None;
            }
        }
        Some(())
    }

    /// A loop whose whole cost is paid already still looks at the stop flag.
    #[inline]
    fn poll(&self, i: usize) -> Option<()> {
        if i & 4095 == 0 && self.stop.load(Ordering::Relaxed) {
            None
        } else {
            Some(())
        }
    }
}

/// Marks over literal indices that are cleared by moving to the next stamp.
struct Marks {
    mark: Vec<u32>,
    stamp: u32,
}

impl Marks {
    fn new(n: usize) -> Marks {
        Marks { mark: vec![0; n], stamp: 0 }
    }
    #[inline]
    fn next(&mut self) {
        if self.stamp == u32::MAX {
            self.mark.iter_mut().for_each(|m| *m = 0);
            self.stamp = 0;
        }
        self.stamp += 1;
    }
    #[inline]
    fn set(&mut self, l: usize) {
        self.mark[l] = self.stamp;
    }
    #[inline]
    fn has(&self, l: usize) -> bool {
        self.mark[l] == self.stamp
    }
    #[inline]
    fn clear(&mut self, l: usize) {
        self.mark[l] = 0;
    }
}

/// A formula in flat arrays: clause c is `lits[starts[c]..starts[c + 1]]`.
pub struct Cnf {
    pub nv: usize,
    pub lits: Vec<i32>,
    pub starts: Vec<u32>,
    /// Empty clauses (kept as clauses of no literal): the formula is refuted.
    pub empty: usize,
    occ_start: Vec<u32>,
    occ: Vec<u32>,
    /// `bin_part[bin_start[l]..bin_start[l + 1]]` are the literals y with
    /// (lit | y) a binary clause, `bin_cid` the clause of each.
    bin_start: Vec<u32>,
    bin_part: Vec<i32>,
    bin_cid: Vec<u32>,
}

impl Cnf {
    fn new(nv: usize, lits: Vec<i32>, starts: Vec<u32>, empty: usize) -> Cnf {
        Cnf { nv, lits, starts, empty, occ_start: Vec::new(), occ: Vec::new(), bin_start: Vec::new(), bin_part: Vec::new(), bin_cid: Vec::new() }
    }

    pub fn m(&self) -> usize {
        self.starts.len() - 1
    }

    #[inline]
    pub fn clause(&self, c: usize) -> &[i32] {
        &self.lits[self.starts[c] as usize..self.starts[c + 1] as usize]
    }

    #[inline]
    fn size(&self, c: usize) -> usize {
        (self.starts[c + 1] - self.starts[c]) as usize
    }

    /// The clauses holding `lit` (after `build_occ`).
    #[inline]
    pub fn occs(&self, lit: i32) -> &[u32] {
        let l = li(lit);
        &self.occ[self.occ_start[l] as usize..self.occ_start[l + 1] as usize]
    }

    #[inline]
    fn bins(&self, l: usize) -> std::ops::Range<usize> {
        self.bin_start[l] as usize..self.bin_start[l + 1] as usize
    }

    /// Does variable v occur at all (after `build_occ`)?
    #[inline]
    fn present(&self, v: usize) -> bool {
        self.occ_start[2 * v] != self.occ_start[2 * v + 2]
    }

    /// The formula of the structure pass, tautologies left out, empty clauses
    /// kept (they refute it).
    fn from_formula(f: &Formula, ck: &mut Clock) -> Option<Cnf> {
        ck.pay(f.lits.len() as u64 + f.len() as u64)?;
        let mut lits = Vec::with_capacity(f.lits.len());
        let mut starts = Vec::with_capacity(f.len() + 1);
        starts.push(0u32);
        let mut empty = 0;
        let mut nv = f.vars.max(0) as usize;
        for i in 0..f.len() {
            ck.poll(i)?;
            if f.taut[i] {
                continue;
            }
            let c = f.clause(i);
            if c.is_empty() {
                empty += 1;
            }
            for &x in c {
                nv = nv.max(var(x));
            }
            lits.extend_from_slice(c);
            starts.push(lits.len() as u32);
        }
        Some(Cnf::new(nv, lits, starts, empty))
    }

    /// Occurrence lists by literal index, in clause order, and the
    /// binary-clause partners.
    fn build_occ(&mut self, ck: &mut Clock) -> Option<()> {
        let n_idx = 2 * self.nv + 2;
        let m = self.m();
        ck.pay(3 * self.lits.len() as u64 + 3 * n_idx as u64 + 2 * m as u64)?;
        let mut os = vec![0u32; n_idx + 1];
        for &l in &self.lits {
            os[li(l) + 1] += 1;
        }
        for i in 1..=n_idx {
            os[i] += os[i - 1];
        }
        let mut occ = vec![0u32; self.lits.len()];
        let mut fill = os.clone();
        let mut bcount = vec![0u32; n_idx + 1];
        for c in 0..m {
            ck.poll(c)?;
            let (s0, s1) = (self.starts[c] as usize, self.starts[c + 1] as usize);
            for &lit in &self.lits[s0..s1] {
                let l = li(lit);
                occ[fill[l] as usize] = c as u32;
                fill[l] += 1;
            }
            if s1 - s0 == 2 {
                bcount[li(self.lits[s0]) + 1] += 1;
                bcount[li(self.lits[s0 + 1]) + 1] += 1;
            }
        }
        for i in 1..=n_idx {
            bcount[i] += bcount[i - 1];
        }
        let nb = bcount[n_idx] as usize;
        ck.pay(2 * nb as u64)?;
        let mut bpart = vec![0i32; nb];
        let mut bcid = vec![0u32; nb];
        let mut bfill = bcount.clone();
        for c in 0..m {
            ck.poll(c)?;
            let s0 = self.starts[c] as usize;
            if self.starts[c + 1] as usize - s0 != 2 {
                continue;
            }
            let (a, b) = (self.lits[s0], self.lits[s0 + 1]);
            let (la, lb) = (li(a), li(b));
            let p = bfill[la] as usize;
            bpart[p] = b;
            bcid[p] = c as u32;
            bfill[la] += 1;
            let p = bfill[lb] as usize;
            bpart[p] = a;
            bcid[p] = c as u32;
            bfill[lb] += 1;
        }
        self.occ_start = os;
        self.occ = occ;
        self.bin_start = bcount;
        self.bin_part = bpart;
        self.bin_cid = bcid;
        Some(())
    }
}

/// Unit propagation at the root.
enum Up {
    Conflict,
    /// The reduced formula, the root values (0 unset, 1 true, 2 false) and
    /// the clauses the units satisfied.
    Kept(Cnf, Vec<u8>, usize),
}

fn unit_propagate(mut cnf: Cnf, ck: &mut Clock) -> Option<Up> {
    if cnf.empty > 0 {
        return Some(Up::Conflict);
    }
    let m = cnf.m();
    ck.pay(m as u64)?;
    let mut queue: Vec<i32> = Vec::new();
    for c in 0..m {
        ck.poll(c)?;
        if cnf.size(c) == 1 {
            queue.push(cnf.lits[cnf.starts[c] as usize]);
        }
    }
    // a table per variable is paid for before it is made
    ck.pay(cnf.nv as u64 + 1)?;
    let val = vec![0u8; cnf.nv + 1];
    if queue.is_empty() {
        return Some(Up::Kept(cnf, val, 0));
    }
    let mut val = val;
    cnf.build_occ(ck)?;
    ck.pay(2 * m as u64)?;
    let mut sat = vec![false; m];
    let mut nfalse = vec![0u32; m];
    let mut qi = 0;
    while qi < queue.len() {
        let lit = queue[qi];
        qi += 1;
        ck.pay(1)?;
        let v = var(lit);
        let want = if lit > 0 { 1 } else { 2 };
        if val[v] != 0 {
            if val[v] != want {
                return Some(Up::Conflict);
            }
            continue;
        }
        val[v] = want;
        let l = li(lit);
        let (a, b) = (cnf.occ_start[l] as usize, cnf.occ_start[l + 1] as usize);
        ck.pay((b - a) as u64 + 1)?;
        for &c in &cnf.occ[a..b] {
            sat[c as usize] = true;
        }
        let nl = l ^ 1;
        let (a, b) = (cnf.occ_start[nl] as usize, cnf.occ_start[nl + 1] as usize);
        ck.pay((b - a) as u64)?;
        for p in a..b {
            let c = cnf.occ[p] as usize;
            if sat[c] {
                continue;
            }
            let nf = nfalse[c] + 1;
            nfalse[c] = nf;
            let k = cnf.size(c) as u32;
            if nf == k {
                return Some(Up::Conflict);
            }
            if nf == k - 1 {
                ck.pay(k as u64)?;
                for &x in cnf.clause(c) {
                    let vx = val[var(x)];
                    if vx == 0 {
                        queue.push(x);
                        break;
                    }
                    if (vx == 1) == (x > 0) {
                        sat[c] = true;
                        break;
                    }
                }
            }
        }
    }
    let nsat = sat.iter().filter(|&&s| s).count();
    ck.pay(cnf.lits.len() as u64 + m as u64)?;
    let mut nl = Vec::new();
    let mut ns = vec![0u32];
    for c in 0..m {
        ck.poll(c)?;
        if sat[c] {
            continue;
        }
        for &x in cnf.clause(c) {
            if val[var(x)] == 0 {
                nl.push(x);
            }
        }
        ns.push(nl.len() as u32);
    }
    Some(Up::Kept(Cnf::new(cnf.nv, nl, ns, 0), val, nsat))
}

/// A small gate: its inputs, its truth table (input k is bit k of the row)
/// and the clauses that define it.
#[derive(Clone)]
pub struct Gate {
    pub support: Vec<u32>,
    pub tt: u32,
    pub clauses: Vec<u32>,
}

#[derive(Default, Clone)]
pub struct Stats {
    pub overlap_groups: u64,
    pub derived_groups: u64,
    pub cycle_breaks: u64,
    pub topo_numbered: bool,
}

/// The roles of one formula.
pub struct Reading {
    /// The formula after unit propagation.
    pub cnf: Cnf,
    /// Root values from unit propagation: 0 unset, 1 true, 2 false.
    pub root: Vec<u8>,
    /// Clauses before unit propagation (tautologies left out).
    pub clauses_read: usize,
    pub satisfied: usize,
    /// Member literals of each group: at most one true, exactly one unless
    /// `none_ok` (an at-most-one group).
    pub groups: Vec<Vec<i32>>,
    pub chain: Vec<bool>,
    pub none_ok: Vec<bool>,
    pub gid: Vec<i32>,
    pub role: Vec<u8>,
    pub pos: Vec<i32>,
    /// The variables in the order they were settled.
    pub order: Vec<u32>,
    /// The accepted candidate of each variable.
    pub acc: Vec<i32>,
    // candidates: output, kind, a (clause / gate index / literal), b (pivot)
    pub c_out: Vec<u32>,
    pub c_kind: Vec<u8>,
    pub c_a: Vec<i64>,
    pub c_b: Vec<i32>,
    pub c_in_start: Vec<u64>,
    pub c_in: Vec<u32>,
    pub gates: Vec<Gate>,
    pub cover: Vec<u8>,
    /// How many variables each clause defines (the circuit-order test).
    npass: Vec<u32>,
    pub stats: Stats,
}

/// Why there is no reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotRead {
    /// Unit propagation (or an empty clause) refutes the formula.
    Refuted,
    /// The work limit ran out, or the run was told to stop.
    Budget,
}

/// Read the roles of the structure pass's formula within `budget` (the tick
/// count to stop at), counting the work into `ticks`.
pub fn read(f: &Formula, budget: u64, ticks: &mut u64, stop: &AtomicBool) -> Result<Reading, NotRead> {
    let mut ck = Clock::new(ticks, budget, stop);
    let cnf = Cnf::from_formula(f, &mut ck).ok_or(NotRead::Budget)?;
    let clauses_read = cnf.m();
    let (red, root, satisfied) = match unit_propagate(cnf, &mut ck).ok_or(NotRead::Budget)? {
        Up::Conflict => return Err(NotRead::Refuted),
        Up::Kept(red, root, nsat) => (red, root, nsat),
    };
    let mut r = Reading::new(red, root, clauses_read, satisfied, &mut ck).ok_or(NotRead::Budget)?;
    r.run(&mut ck).ok_or(NotRead::Budget)?;
    Ok(r)
}

impl Reading {
    /// The tables per variable and per clause, paid for before they are made.
    fn new(cnf: Cnf, root: Vec<u8>, clauses_read: usize, satisfied: usize, ck: &mut Clock) -> Option<Reading> {
        let nv = cnf.nv;
        let m = cnf.m();
        ck.pay(4 * (nv as u64 + 1) + m as u64)?;
        Some(Reading {
            cnf,
            root,
            clauses_read,
            satisfied,
            groups: Vec::new(),
            chain: Vec::new(),
            none_ok: Vec::new(),
            gid: vec![-1; nv + 1],
            role: vec![ABSENT; nv + 1],
            pos: vec![-1; nv + 1],
            order: Vec::new(),
            acc: vec![-1; nv + 1],
            c_out: Vec::new(),
            c_kind: Vec::new(),
            c_a: Vec::new(),
            c_b: Vec::new(),
            c_in_start: vec![0],
            c_in: Vec::new(),
            gates: Vec::new(),
            cover: vec![C_NONE; m],
            npass: Vec::new(),
            stats: Stats::default(),
        })
    }

    #[inline]
    fn inputs(&self, i: usize) -> &[u32] {
        &self.c_in[self.c_in_start[i] as usize..self.c_in_start[i + 1] as usize]
    }

    fn add_cand(&mut self, v: usize, kind: u8, a: i64, b: i32, inputs: impl IntoIterator<Item = u32>) {
        self.c_out.push(v as u32);
        self.c_kind.push(kind);
        self.c_a.push(a);
        self.c_b.push(b);
        self.c_in.extend(inputs);
        self.c_in_start.push(self.c_in.len() as u64);
    }

    fn run(&mut self, ck: &mut Clock) -> Option<()> {
        self.cnf.build_occ(ck)?;
        let n_idx = 2 * self.cnf.nv + 2;
        ck.pay(n_idx as u64)?;
        let mut marks = Marks::new(n_idx);
        let (piv_c, piv_x, npass) = self.pivot_pass(ck, &mut marks)?;
        let claimed = self.two_way_candidates(&piv_c, &piv_x, ck, &mut marks)?;
        self.find_groups(&npass, &piv_c, &piv_x, ck, n_idx)?;
        self.npass = npass;
        drop(piv_c);
        drop(piv_x);
        self.helper_candidates(&claimed, ck)?;
        drop(claimed);
        self.settle(ck, &mut marks)?;
        self.classify_rest(ck)?;
        debug_assert!(self.acyclic());
        Some(())
    }

    // -- step 1: the pivot test ---------------------------------------------

    /// For every clause and literal x of it: is (-x | -m) a binary clause
    /// for every other literal m? Then -x is the OR of the others.
    fn pivot_pass(&self, ck: &mut Clock, part: &mut Marks) -> Option<(Vec<u32>, Vec<i32>, Vec<u32>)> {
        let cnf = &self.cnf;
        let mut piv_c = Vec::new();
        let mut piv_x = Vec::new();
        // the table, and a look at each literal (the loop's early continues)
        ck.pay(cnf.m() as u64 + 2 * cnf.nv as u64)?;
        let mut npass = vec![0u32; cnf.m()];
        for v in 1..=cnf.nv {
            ck.poll(v)?;
            for x in [v as i32, -(v as i32)] {
                let l = li(x);
                let (a, b) = (cnf.occ_start[l] as usize, cnf.occ_start[l + 1] as usize);
                if a == b {
                    continue;
                }
                let nl = l ^ 1;
                let nb = (cnf.bin_start[nl + 1] - cnf.bin_start[nl]) as usize;
                if nb == 0 {
                    continue;
                }
                ck.pay((b - a) as u64)?;
                let mut built = false;
                for p in a..b {
                    let c = cnf.occ[p] as usize;
                    let (s0, s1) = (cnf.starts[c] as usize, cnf.starts[c + 1] as usize);
                    if s1 - s0 - 1 > nb {
                        continue;
                    }
                    if !built {
                        ck.pay(nb as u64)?;
                        part.next();
                        for q in cnf.bins(nl) {
                            part.set(li(cnf.bin_part[q]));
                        }
                        built = true;
                    }
                    ck.pay((s1 - s0) as u64)?;
                    if cnf.lits[s0..s1].iter().all(|&y| y == x || part.has(li(-y))) {
                        piv_c.push(c as u32);
                        piv_x.push(x);
                        npass[c] += 1;
                    }
                }
            }
        }
        Some((piv_c, piv_x, npass))
    }

    // -- step 2: candidates ---------------------------------------------------

    /// Every two-way definition found, for every variable: the OR/AND ones
    /// from the pivot test, and small gates for the variables the pivot test
    /// gives none. Returns claimed: clause -> the variable whose definition
    /// uses it (-2 when two variables' do).
    fn two_way_candidates(&mut self, piv_c: &[u32], piv_x: &[i32], ck: &mut Clock, mem: &mut Marks) -> Option<Vec<i32>> {
        let m = self.cnf.m();
        let nv = self.cnf.nv;
        ck.pay(m as u64 + nv as u64)?;
        let mut claimed = vec![-1i32; m];
        fn claim(claimed: &mut [i32], c: usize, v: i32) {
            let o = claimed[c];
            claimed[c] = if o == -1 || o == v { v } else { -2 };
        }
        let mut has_def = vec![false; nv + 1];
        for (k, (&c, &x)) in piv_c.iter().zip(piv_x).enumerate() {
            ck.poll(k)?;
            let c = c as usize;
            let v = var(x);
            let size = self.cnf.size(c);
            ck.pay(2 * size as u64)?;
            let s0 = self.cnf.starts[c] as usize;
            for q in s0..s0 + size {
                let y = self.cnf.lits[q];
                if y != x {
                    self.c_in.push(var(y) as u32);
                }
            }
            self.c_out.push(v as u32);
            self.c_kind.push(DEF);
            self.c_a.push(c as i64);
            self.c_b.push(x);
            self.c_in_start.push(self.c_in.len() as u64);
            has_def[v] = true;
            claim(&mut claimed, c, v as i32);
            mem.next();
            for &y in self.cnf.clause(c) {
                mem.set(li(y));
            }
            let ln = li(-x);
            let range = self.cnf.bins(ln);
            ck.pay(range.len() as u64)?;
            for p in range {
                let z = self.cnf.bin_part[p];
                if mem.has(li(-z)) && z != -x {
                    claim(&mut claimed, self.cnf.bin_cid[p] as usize, v as i32);
                }
            }
        }
        for v in 1..=nv {
            ck.poll(v)?;
            if has_def[v] || !self.cnf.present(v) {
                continue;
            }
            if let Some(g) = self.gate_candidate(v, ck)? {
                for k in 0..self.gates[g].clauses.len() {
                    let c = self.gates[g].clauses[k] as usize;
                    claim(&mut claimed, c, v as i32);
                }
            }
        }
        Some(claimed)
    }

    /// A small gate for v (index into `gates`), or None; the outer None is
    /// the budget.
    fn gate_candidate(&mut self, v: usize, ck: &mut Clock) -> Option<Option<usize>> {
        let vi = v as i32;
        let (pos_occ, neg_occ) = (self.cnf.occs(vi), self.cnf.occs(-vi));
        ck.pay((pos_occ.len() + neg_occ.len()) as u64)?;
        let mut short: Vec<u32> = Vec::new();
        for &c in pos_occ.iter().chain(neg_occ.iter()) {
            let k = self.cnf.size(c as usize);
            if (2..=MAX_GATE_SUPPORT + 1).contains(&k) {
                short.push(c);
            }
        }
        if short.is_empty() || short.len() > MAX_GATE_SHORT {
            return Some(None);
        }
        // the variables of each short clause but v, sorted
        ck.pay(8 * short.len() as u64)?;
        let vs: Vec<Vec<u32>> = short
            .iter()
            .map(|&c| {
                let mut s: Vec<u32> = self.cnf.clause(c as usize).iter().map(|&x| var(x) as u32).filter(|&u| u as usize != v).collect();
                s.sort_unstable();
                s
            })
            .collect();
        let mut tried: Vec<Vec<u32>> = Vec::new();
        let mut n_starts = 0;
        let mut st: Vec<usize> = (0..short.len()).filter(|&j| self.cnf.size(short[j] as usize) >= 3).collect();
        st.sort_by_key(|&j| self.cnf.size(short[j] as usize)); // stable, as Python's sort
        for &s in &st {
            let mut support = vs[s].clone();
            if tried.contains(&support) {
                continue;
            }
            n_starts += 1;
            if n_starts > MAX_GATE_STARTS {
                return Some(None);
            }
            for _grow in 0..3 {
                ck.pay(64 * short.len() as u64)?;
                tried.push(support.clone());
                let sel: Vec<usize> = (0..short.len()).filter(|&j| subset(&vs[j], &support)).collect();
                if let Some(tt) = self.definition_tt(v, &sel.iter().map(|&j| short[j]).collect::<Vec<_>>(), &support) {
                    let mut clauses: Vec<u32> = sel.iter().map(|&j| short[j]).collect();
                    clauses.sort_unstable();
                    self.gates.push(Gate { support: support.clone(), tt, clauses });
                    let g = self.gates.len() - 1;
                    self.add_cand(v, GATE, g as i64, 0, support.iter().copied());
                    return Some(Some(g));
                }
                let mut best: Option<usize> = None;
                for j in 0..short.len() {
                    let oth = &vs[j];
                    let new = minus_len(oth, &support);
                    if new == 0 || !intersects(oth, &support) || union_len(&support, oth) > MAX_GATE_SUPPORT {
                        continue;
                    }
                    if best.is_none_or(|b| new < minus_len(&vs[b], &support)) {
                        best = Some(j);
                    }
                }
                let Some(b) = best else { break };
                support = union(&support, &vs[b]);
            }
        }
        Some(None)
    }

    /// Truth table of v over `support` if the clauses (each holding v and
    /// otherwise only support variables) fix v for every support value with
    /// no value forbidden, else None.
    fn definition_tt(&self, v: usize, clauses: &[u32], support: &[u32]) -> Option<u32> {
        let mut pos: Vec<Vec<(usize, bool)>> = Vec::new();
        let mut neg: Vec<Vec<(usize, bool)>> = Vec::new();
        for &c in clauses {
            let mut others = Vec::new();
            let mut sign = 0;
            for &lit in self.cnf.clause(c as usize) {
                if var(lit) == v {
                    sign = if lit > 0 { 1 } else { -1 };
                } else {
                    let i = support.iter().position(|&u| u as usize == var(lit)).expect("a gate clause reads its support only");
                    others.push((i, lit > 0));
                }
            }
            if sign > 0 {
                pos.push(others);
            } else {
                neg.push(others);
            }
        }
        let hit = |list: &Vec<Vec<(usize, bool)>>, mt: u32| list.iter().any(|oth| oth.iter().all(|&(i, b)| ((mt >> i) & 1 == 1) != b));
        let mut tt = 0u32;
        for mt in 0..(1u32 << support.len()) {
            let pv = hit(&pos, mt);
            let nv = hit(&neg, mt);
            if pv == nv {
                return None; // forbidden (both) or not determined (neither)
            }
            if pv {
                tt |= 1 << mt;
            }
        }
        Some(tt)
    }

    // -- step 3: groups -----------------------------------------------------

    /// The choices: exactly-one clauses whose literals exclude each other,
    /// and at-most-one clauses (g | m1 | ... | mk) whose m do, taken biggest
    /// first while they share no variable with a taken group. A group with a
    /// member defined by other clauses is not a choice.
    fn find_groups(&mut self, npass: &[u32], piv_c: &[u32], piv_x: &[i32], ck: &mut Clock, n_idx: usize) -> Option<()> {
        let m = self.cnf.m();
        let nv = self.cnf.nv;
        ck.pay(2 * n_idx as u64)?;
        let mut walk = Walk { seen: Marks::new(n_idx), targets: Marks::new(n_idx), queue: VecDeque::new() };
        // (members, clause, chained, none allowed)
        let mut cands: Vec<(Vec<i32>, u32, bool, bool)> = Vec::new();
        let mut tested: HashMap<Vec<i32>, bool> = HashMap::new();
        // a look at each clause (the loop's early continues)
        ck.pay(m as u64)?;
        for c in 0..m {
            ck.poll(c)?;
            let k = self.cnf.size(c);
            if k < 3 {
                continue;
            }
            if npass[c] as usize == k {
                ck.pay(k as u64)?;
                cands.push((self.cnf.clause(c).to_vec(), c as u32, false, false));
                continue;
            }
            ck.pay(k as u64)?;
            let cl = self.cnf.clause(c);
            let npos = cl.iter().filter(|&&x| x > 0).count();
            if k >= 4 && (npos == 1 || npos == k - 1) && k - 1 <= MAX_CHAIN_GROUP {
                let gsign = npos == 1; // the guard is the one positive literal
                let mem: Vec<i32> = cl.iter().copied().filter(|&x| (x > 0) != gsign).collect();
                if self.excl(&mem, &mut tested, &mut walk, ck)? {
                    cands.push((mem, c as u32, true, true));
                }
                continue;
            }
            if npass[c] > 0 || k > MAX_CHAIN_GROUP {
                continue;
            }
            let cl = cl.to_vec();
            if self.excl(&cl, &mut tested, &mut walk, ck)? {
                cands.push((cl, c as u32, true, false));
            }
        }
        drop(tested);
        ck.pay(m as u64 + 2 * nv as u64 + piv_c.len() as u64 + self.c_out.len() as u64)?;
        let mut is_group = vec![false; m];
        let mut members = vec![false; nv + 1];
        for (mem, c, _, _) in &cands {
            is_group[*c as usize] = true;
            for &x in mem {
                members[var(x)] = true;
            }
        }
        // A member is defined elsewhere when a gate defines it, or a pivot
        // definition of two or more inputs from a clause that is not itself a
        // group (a commander: the guard of one group is a member of the next;
        // such a definition does not count). The census asks this per
        // candidate, of the definitions from clauses other than the
        // candidate's own; but the candidate's clause is always a group
        // clause, so the answer is the same for every candidate and is kept
        // per variable (one pass over the definitions, not one per candidate).
        let mut outside = vec![false; nv + 1];
        for (&c, &x) in piv_c.iter().zip(piv_x) {
            if members[var(x)] && self.cnf.size(c as usize) >= 3 && !is_group[c as usize] {
                outside[var(x)] = true;
            }
        }
        for i in 0..self.c_out.len() {
            if self.c_kind[i] == GATE {
                outside[self.c_out[i] as usize] = true;
            }
        }
        drop(is_group);
        // bigger groups first: a sort, paid for before it is done
        let n = cands.len() as u64;
        ck.pay(n * (64 - n.leading_zeros() as u64 + 1))?;
        cands.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.1.cmp(&b.1)));
        for (k, (mem, _clause, chained, none_ok)) in cands.into_iter().enumerate() {
            ck.poll(k)?;
            ck.pay(2 * mem.len() as u64)?;
            if mem.iter().any(|&x| outside[var(x)]) {
                self.stats.derived_groups += 1;
                continue;
            }
            if mem.iter().any(|&x| self.gid[var(x)] >= 0) {
                self.stats.overlap_groups += 1;
                continue;
            }
            let g = self.groups.len() as i32;
            for &x in &mem {
                self.gid[var(x)] = g;
            }
            self.groups.push(mem);
            self.chain.push(chained);
            self.none_ok.push(none_ok);
        }
        Some(())
    }

    /// Does every literal of `mem` exclude every other one? With the cheap
    /// first-pair screen first (not cached), and each literal set tested once.
    fn excl(&self, mem: &[i32], tested: &mut HashMap<Vec<i32>, bool>, walk: &mut Walk, ck: &mut Clock) -> Option<bool> {
        let cnf = &self.cnf;
        ck.pay(mem.len() as u64)?;
        let has_impl = mem.iter().all(|&x| {
            let l = li(x) ^ 1;
            cnf.bin_start[l + 1] != cnf.bin_start[l]
        });
        if !(has_impl && walk.excludes(cnf, mem[0], mem[1], QUICK_BUDGET, ck)?) {
            return Some(false);
        }
        let n = mem.len() as u64;
        ck.pay(n * (64 - n.leading_zeros() as u64 + 2))?;
        let mut key = mem.to_vec();
        key.sort_unstable();
        if let Some(&r) = tested.get(&key) {
            return Some(r);
        }
        let r = walk.excludes_all(cnf, mem, ck)?;
        tested.insert(key, r);
        Some(r)
    }

    // -- step 4: helpers ----------------------------------------------------

    /// One-way helpers for the variables outside the groups: a literal y
    /// whose clauses hold at most MAX_HELPER_IN other literals in all, none
    /// of them a clause another variable's definition uses.
    fn helper_candidates(&mut self, claimed: &[i32], ck: &mut Clock) -> Option<()> {
        let nv = self.cnf.nv;
        let mut ins: Vec<u32> = Vec::new();
        // a look at each variable (the loop's early continues)
        ck.pay(nv as u64)?;
        for v in 1..=nv {
            ck.poll(v)?;
            if self.gid[v] >= 0 || !self.cnf.present(v) {
                continue;
            }
            for y in [v as i32, -(v as i32)] {
                let mut w = 0usize;
                ins.clear();
                let mut ok = true;
                let occs = self.cnf.occs(y);
                for &c in occs {
                    let c = c as usize;
                    ck.pay(1)?;
                    if claimed[c] != -1 && claimed[c] != v as i32 {
                        ok = false;
                        break;
                    }
                    let size = self.cnf.size(c);
                    w += size - 1;
                    if w > MAX_HELPER_IN {
                        ok = false;
                        break;
                    }
                    ck.pay(size as u64)?;
                    for &z in self.cnf.clause(c) {
                        if z != y {
                            ins.push(var(z) as u32);
                        }
                    }
                }
                if ok {
                    ins.sort_unstable();
                    ins.dedup();
                    ins.retain(|&u| u as usize != v);
                    let taken = std::mem::take(&mut ins);
                    self.add_cand(v, HELPER, y as i64, 0, taken.iter().copied());
                    ins = taken;
                }
            }
        }
        Some(())
    }

    // -- step 5: settle forward -----------------------------------------------

    fn put(&mut self, v: usize, r: u8, ci: i32) {
        self.role[v] = r;
        self.acc[v] = ci;
        self.pos[v] = self.order.len() as i32;
        self.order.push(v as u32);
    }

    fn settle(&mut self, ck: &mut Clock, mem: &mut Marks) -> Option<()> {
        let nv = self.cnf.nv;
        let n_cand = self.c_out.len();
        ck.pay(4 * nv as u64 + 2 * n_cand as u64 + 2 * self.c_in.len() as u64)?;
        let present: Vec<bool> = (0..=nv).map(|v| v > 0 && self.cnf.present(v)).collect();
        let mut has_cand = vec![false; nv + 1];
        let mut two_way = vec![false; nv + 1];
        for i in 0..n_cand {
            let v = self.c_out[i] as usize;
            has_cand[v] = true;
            if self.c_kind[i] != HELPER {
                two_way[v] = true;
            }
        }
        for g in 0..self.groups.len() {
            for j in 0..self.groups[g].len() {
                let v = var(self.groups[g][j]);
                self.put(v, CHOICE, -1);
            }
        }
        for v in 1..=nv {
            if present[v] && self.gid[v] < 0 && !has_cand[v] {
                self.put(v, FREE, -1);
            }
        }
        // waiting lists over the unsettled inputs
        let mut rem = vec![0u32; n_cand];
        let mut wcount = vec![0u64; nv + 2];
        for i in 0..n_cand {
            ck.poll(i)?;
            let mut k = 0;
            for &u in self.inputs(i) {
                if self.pos[u as usize] < 0 {
                    wcount[u as usize + 1] += 1;
                    k += 1;
                }
            }
            rem[i] = k;
        }
        for u in 1..nv + 2 {
            wcount[u] += wcount[u - 1];
        }
        let mut wl = vec![0u32; wcount[nv + 1] as usize];
        let mut wfill = wcount.clone();
        for i in 0..n_cand {
            ck.poll(i)?;
            for &u in self.inputs(i) {
                if self.pos[u as usize] < 0 {
                    wl[wfill[u as usize] as usize] = i as u32;
                    wfill[u as usize] += 1;
                }
            }
        }
        drop(wfill);
        let mut ready2: VecDeque<u32> = VecDeque::new();
        let mut ready1: VecDeque<u32> = VecDeque::new();
        for i in 0..n_cand {
            if rem[i] == 0 {
                if self.c_kind[i] != HELPER {
                    ready2.push_back(i as u32);
                } else {
                    ready1.push_back(i as u32);
                }
            }
        }
        let mut unsettled = (1..=nv).filter(|&v| present[v] && self.pos[v] < 0).count();
        let mut low: Option<Vec<u32>> = None;
        let mut breaks = 0u64;
        let mut steps = 0usize;
        while unsettled > 0 {
            steps += 1;
            ck.poll(steps)?;
            let (v, r, ci) = if let Some(i) = ready2.pop_front().or_else(|| ready1.pop_front()) {
                let i = i as usize;
                let v = self.c_out[i] as usize;
                if self.pos[v] >= 0 {
                    continue;
                }
                (v, self.c_kind[i], i as i32)
            } else {
                if low.is_none() {
                    // A dead lock frees one variable. In a file numbered in
                    // circuit order the lowest-numbered unsettled one;
                    // otherwise the one the most waiting candidates read,
                    // among those with no two-way definition first.
                    let gate_clauses: u64 = self.gates.iter().map(|g| g.clauses.len() as u64 + 1).sum();
                    ck.pay(n_cand as u64 + self.c_in.len() as u64 + 2 * gate_clauses)?;
                    let topo = self.topo_numbered();
                    self.stats.topo_numbered = topo;
                    // the list and its sort, paid for before they are done
                    let n = nv as u64;
                    ck.pay(4 * n + n * (64 - n.leading_zeros() as u64 + 1))?;
                    let mut l: Vec<u32> = Vec::new();
                    if topo {
                        for v in (1..=nv).rev() {
                            if present[v] && self.pos[v] < 0 {
                                l.push(v as u32);
                            }
                        }
                    } else {
                        let mut items: Vec<(bool, u64, u64, u32)> = Vec::new();
                        for v in 1..=nv {
                            if present[v] && self.pos[v] < 0 {
                                let w = wcount[v + 1] - wcount[v];
                                let o = (self.cnf.occ_start[2 * v + 2] - self.cnf.occ_start[2 * v]) as u64;
                                items.push((two_way[v], w, o, v as u32));
                            }
                        }
                        l = free_order(items);
                    }
                    low = Some(l);
                }
                let l = low.as_mut().expect("set above");
                let mut v = 0usize;
                while let Some(u) = l.pop() {
                    if self.pos[u as usize] < 0 {
                        v = u as usize;
                        break;
                    }
                }
                debug_assert!(v != 0, "every unsettled variable is in the dead-lock order");
                breaks += 1;
                (v, FREE, -1)
            };
            // settle v
            self.put(v, r, ci);
            if ci >= 0 {
                self.cover_cand(ci as usize, ck, mem)?;
            }
            let (a, b) = (wcount[v] as usize, wcount[v + 1] as usize);
            ck.pay((b - a) as u64 + 1)?;
            for &i in &wl[a..b] {
                let i = i as usize;
                rem[i] -= 1;
                if rem[i] == 0 {
                    if self.c_kind[i] != HELPER {
                        ready2.push_back(i as u32);
                    } else {
                        ready1.push_back(i as u32);
                    }
                }
            }
            unsettled -= 1;
        }
        self.stats.cycle_breaks = breaks;
        Some(())
    }

    /// Is the file numbered in circuit order? Judged on the two-way
    /// definitions whose clauses define one variable only: at least 20, and
    /// 95% of them read only lower-numbered variables. With fewer, on the
    /// helpers: 80% read only lower- or only higher-numbered variables.
    fn topo_numbered(&self) -> bool {
        let mut gate_defines: HashMap<&[u32], u64> = HashMap::new();
        for g in &self.gates {
            *gate_defines.entry(&g.clauses[..]).or_default() += 1;
        }
        let (mut single, mut ordered, mut hel, mut mono) = (0u64, 0u64, 0u64, 0u64);
        for i in 0..self.c_out.len() {
            let v = self.c_out[i];
            let ins = self.inputs(i);
            let (lo, hi) = (ins.iter().min().copied(), ins.iter().max().copied());
            if self.c_kind[i] == HELPER {
                hel += 1;
                if ins.is_empty() || hi.is_some_and(|h| h < v) || lo.is_some_and(|l| l > v) {
                    mono += 1;
                }
            } else {
                let defines = if self.c_kind[i] == DEF { self.npass[self.c_a[i] as usize] as u64 } else { gate_defines[&self.gates[self.c_a[i] as usize].clauses[..]] };
                if defines == 1 {
                    single += 1;
                    if ins.is_empty() || hi.is_some_and(|h| h < v) {
                        ordered += 1;
                    }
                }
            }
        }
        if single >= 20 {
            return ordered as f64 >= 0.95 * single as f64;
        }
        hel >= 20 && mono as f64 >= 0.8 * hel as f64
    }

    /// Mark the clauses the accepted candidate i makes true.
    fn cover_cand(&mut self, i: usize, ck: &mut Clock, mem: &mut Marks) -> Option<()> {
        match self.c_kind[i] {
            DEF => {
                let (c, x) = (self.c_a[i] as usize, self.c_b[i]);
                self.cover[c] = C_DEF;
                mem.next();
                let cl = &self.cnf.lits[self.cnf.starts[c] as usize..self.cnf.starts[c + 1] as usize];
                ck.pay(cl.len() as u64)?;
                for &y in cl {
                    mem.set(li(y));
                }
                let range = self.cnf.bins(li(-x));
                ck.pay(range.len() as u64)?;
                for p in range {
                    let z = self.cnf.bin_part[p];
                    if mem.has(li(-z)) && z != -x {
                        self.cover[self.cnf.bin_cid[p] as usize] = C_DEF;
                    }
                }
            }
            GATE => {
                let g = self.c_a[i] as usize;
                ck.pay(self.gates[g].clauses.len() as u64)?;
                for k in 0..self.gates[g].clauses.len() {
                    let c = self.gates[g].clauses[k] as usize;
                    self.cover[c] = C_GATE;
                }
            }
            _ => {
                let y = self.c_a[i] as i32;
                let n = self.cnf.occs(y).len();
                ck.pay(n as u64 + self.cnf.occs(-y).len() as u64)?;
                for k in 0..n {
                    let c = self.cnf.occs(y)[k] as usize;
                    self.cover[c] = C_HELPER;
                }
                if n == 0 {
                    // a pure literal: -y is always true
                    for k in 0..self.cnf.occs(-y).len() {
                        let c = self.cnf.occs(-y)[k] as usize;
                        self.cover[c] = C_HELPER;
                    }
                }
            }
        }
        Some(())
    }

    // -- step 6: what is left -----------------------------------------------

    /// Clauses no role covered: over one group's members and implied by the
    /// group (two or more negated members, or every member for an
    /// exactly-one), or constraints.
    fn classify_rest(&mut self, ck: &mut Clock) -> Option<()> {
        let nv = self.cnf.nv;
        let m = self.cnf.m();
        ck.pay(nv as u64 + m as u64)?;
        let mut member = vec![0i32; nv + 1];
        for g in &self.groups {
            for &x in g {
                member[var(x)] = x;
            }
        }
        for c in 0..m {
            ck.poll(c)?;
            if self.cover[c] != C_NONE {
                continue;
            }
            let cl = self.cnf.clause(c);
            let g = self.gid[var(cl[0])];
            if g < 0 {
                continue;
            }
            ck.pay(cl.len() as u64)?;
            if cl[1..].iter().any(|&x| self.gid[var(x)] != g) {
                continue;
            }
            let negs = cl.iter().filter(|&&x| member[var(x)] == -x).count();
            let g = g as usize;
            if negs >= 2 || (negs == 0 && !self.none_ok[g] && cl.len() == self.groups[g].len()) {
                self.cover[c] = C_GROUP;
            }
        }
        Some(())
    }

    /// Every accepted definition and helper reads only variables settled
    /// before it (true by construction).
    fn acyclic(&self) -> bool {
        self.order.iter().all(|&v| {
            let i = self.acc[v as usize];
            i < 0 || self.inputs(i as usize).iter().all(|&u| self.pos[u as usize] < self.pos[v as usize])
        })
    }

    // -- computing every variable from the choices ----------------------------

    /// Values (bit-parallel over up to 64 patterns, `mask` the patterns in
    /// use) of every variable: `choice[g][j]` says in which patterns member j
    /// of group g is the true one, `free(v)` in which free v is true.
    pub fn evaluate(&self, choice: &[Vec<u64>], free: &dyn Fn(usize) -> u64, mask: u64) -> Vec<u64> {
        let cnf = &self.cnf;
        let mut val = vec![0u64; cnf.nv + 1];
        let lv = |val: &[u64], z: i32| if z > 0 { val[z as usize] } else { mask ^ val[var(z)] };
        let mut done = vec![false; self.groups.len()];
        for &v in &self.order {
            let v = v as usize;
            match self.role[v] {
                CHOICE => {
                    let g = self.gid[v] as usize;
                    if done[g] {
                        continue;
                    }
                    done[g] = true;
                    for (j, &x) in self.groups[g].iter().enumerate() {
                        let mj = choice[g][j];
                        val[var(x)] = if x > 0 { mj } else { mask ^ mj };
                    }
                }
                FREE => val[v] = free(v) & mask,
                DEF => {
                    let i = self.acc[v] as usize;
                    let (c, x) = (self.c_a[i] as usize, self.c_b[i]);
                    let mut t = 0u64;
                    for &y in cnf.clause(c) {
                        if y != x {
                            t |= lv(&val, y);
                        }
                    }
                    let xv = mask ^ t; // x is true exactly when no other literal is
                    val[v] = if x > 0 { xv } else { mask ^ xv };
                }
                GATE => {
                    let g = &self.gates[self.c_a[self.acc[v] as usize] as usize];
                    let mut out = 0u64;
                    for mt in 0..(1u32 << g.support.len()) {
                        if (g.tt >> mt) & 1 == 0 {
                            continue;
                        }
                        let mut t = mask;
                        for (k, &u) in g.support.iter().enumerate() {
                            t &= if (mt >> k) & 1 == 1 { val[u as usize] } else { mask ^ val[u as usize] };
                        }
                        out |= t;
                    }
                    val[v] = out;
                }
                HELPER => {
                    let y = self.c_a[self.acc[v] as usize] as i32;
                    let mut yv = 0u64;
                    for &c in cnf.occs(y) {
                        let mut t = mask;
                        for &z in cnf.clause(c as usize) {
                            if z != y {
                                t &= mask ^ lv(&val, z);
                            }
                        }
                        yv |= t; // y is needed by clause c
                    }
                    val[v] = if y > 0 { yv } else { mask ^ yv };
                }
                _ => {}
            }
        }
        val
    }

    /// (covered clauses false in some pattern, mean false constraint clauses
    /// per pattern) over 64 random choice assignments: one member per group
    /// (or none in an at-most-one group), random free variables, everything
    /// else computed. A covered clause that comes out false is a bug.
    pub fn simulate(&self, seed: u64) -> (u64, f64) {
        let mut rng = seed;
        let mut next = move || {
            // splitmix64
            rng = rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = rng;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        let mut choice: Vec<Vec<u64>> = Vec::with_capacity(self.groups.len());
        for (g, mem) in self.groups.iter().enumerate() {
            let n = mem.len() + self.none_ok[g] as usize;
            let mut ms = vec![0u64; mem.len() + 1];
            for p in 0..64 {
                ms[(next() % n as u64) as usize] |= 1u64 << p;
            }
            choice.push(ms);
        }
        let mut free = vec![0u64; self.cnf.nv + 1];
        for &v in &self.order {
            if self.role[v as usize] == FREE {
                free[v as usize] = next();
            }
        }
        let val = self.evaluate(&choice, &|v| free[v], u64::MAX);
        let mut bad = 0u64;
        let mut false_total = 0u64;
        for c in 0..self.cnf.m() {
            let mut s = 0u64;
            for &z in self.cnf.clause(c) {
                s |= if z > 0 { val[z as usize] } else { !val[var(z)] };
                if s == u64::MAX {
                    break;
                }
            }
            if s != u64::MAX {
                if self.cover[c] != C_NONE {
                    bad += 1;
                } else {
                    false_total += 64 - s.count_ones() as u64;
                }
            }
        }
        (bad, false_total as f64 / 64.0)
    }

    /// One assignment of every variable of the formula: `choice[g]` is the
    /// true member of group g (its length for none), `free(v)` the value of
    /// free v; root values from unit propagation, absent variables false.
    pub fn assignment(&self, choice: &[usize], free: &dyn Fn(usize) -> bool) -> Vec<bool> {
        let cm: Vec<Vec<u64>> = self.groups.iter().enumerate().map(|(g, mem)| (0..mem.len()).map(|j| (choice[g] == j) as u64).collect()).collect();
        let val = self.evaluate(&cm, &|v| free(v) as u64, 1);
        (0..=self.cnf.nv)
            .map(|v| match self.root.get(v) {
                Some(1) => true,
                Some(2) => false,
                _ => self.pos[v] >= 0 && val[v] & 1 == 1,
            })
            .collect()
    }

    // -- summary --------------------------------------------------------------

    pub fn summary(&self) -> Summary {
        let cnf = &self.cnf;
        let mut rc = [0u64; 7];
        for &v in &self.order {
            rc[self.role[v as usize] as usize] += 1;
        }
        let n_free = rc[FREE as usize];
        let present = self.order.len() as u64;
        let mut bits = 0f64;
        for (g, mem) in self.groups.iter().enumerate() {
            bits += ((mem.len() + self.none_ok[g] as usize) as f64).log2();
        }
        bits += n_free as f64;
        let mut cov = [0u64; 5];
        for &c in &self.cover {
            cov[c as usize] += 1;
        }
        let mut con_lens = Tally::default();
        let mut con_choice = 0u64;
        for c in 0..cnf.m() {
            if self.cover[c] != C_NONE {
                continue;
            }
            let cl = cnf.clause(c);
            con_lens.add(cl.len());
            if cl.iter().all(|&z| matches!(self.role[var(z)], CHOICE | FREE)) {
                con_choice += 1;
            }
        }
        let mut sizes = Tally::default();
        for g in &self.groups {
            sizes.add(g.len());
        }
        let (mut wide, mut dmax) = (0u64, 0u64);
        for &v in &self.order {
            if self.role[v as usize] == DEF {
                let i = self.acc[v as usize] as usize;
                let k = self.c_in_start[i + 1] - self.c_in_start[i];
                dmax = dmax.max(k);
                wide += (k > 6) as u64;
            }
        }
        let fixed = self.root.iter().filter(|&&x| x != 0).count() as u64;
        Summary {
            vars: cnf.nv as u64,
            clauses: self.clauses_read as u64,
            fixed,
            satisfied: self.satisfied as u64,
            present,
            groups: self.groups.len() as u64,
            group_vars: self.groups.iter().map(|g| g.len() as u64).sum(),
            group_sizes: sizes.top(6),
            chain_groups: self.chain.iter().filter(|&&b| b).count() as u64,
            amo_groups: self.none_ok.iter().filter(|&&b| b).count() as u64,
            overlap_groups: self.stats.overlap_groups,
            derived_groups: self.stats.derived_groups,
            defs: rc[DEF as usize],
            wide_defs: wide,
            def_inputs_max: dmax,
            gates: rc[GATE as usize],
            helpers: rc[HELPER as usize],
            free: n_free,
            cycle_breaks: self.stats.cycle_breaks,
            bits,
            choice_share: if present > 0 { (self.groups.len() as u64 + n_free) as f64 / present as f64 } else { 0.0 },
            cov_group: cov[C_GROUP as usize],
            cov_def: cov[C_DEF as usize],
            cov_gate: cov[C_GATE as usize],
            cov_helper: cov[C_HELPER as usize],
            constraints: cov[C_NONE as usize],
            constraint_share: if cnf.m() > 0 { cov[C_NONE as usize] as f64 / cnf.m() as f64 } else { 0.0 },
            con_choice,
            con_lens: con_lens.top(6),
            topo_numbered: self.stats.topo_numbered,
        }
    }
}

/// The census's exclusion walks over the binary implications.
struct Walk {
    seen: Marks,
    targets: Marks,
    queue: VecDeque<i32>,
}

impl Walk {
    /// Does every literal of cl exclude every other one through the binary
    /// implications (x -> ... -> -y)? Breadth first, so the short chains
    /// come first.
    fn excludes_all(&mut self, cnf: &Cnf, cl: &[i32], ck: &mut Clock) -> Option<bool> {
        let budget = CHAIN_BUDGET + 8 * cl.len();
        let edges = 16 * budget;
        for &x in cl {
            self.targets.next();
            ck.pay(cl.len() as u64)?;
            for &y in cl {
                if y != x {
                    self.targets.set(li(-y));
                }
            }
            let mut remaining = cl.len() - 1;
            self.seen.next();
            self.seen.set(li(x));
            let mut nseen = 1usize;
            self.queue.clear();
            self.queue.push_back(x);
            let mut looked = 0usize;
            while remaining > 0 {
                let Some(y) = self.queue.pop_front() else { break };
                let ny = li(-y); // the binaries (-y | z) are y -> z
                let range = cnf.bins(ny);
                looked += range.len();
                if looked > edges {
                    return Some(false);
                }
                ck.pay(range.len() as u64 + 1)?;
                for p in range {
                    let z = cnf.bin_part[p];
                    let lz = li(z);
                    if self.seen.has(lz) {
                        continue;
                    }
                    if self.seen.has(li(-z)) {
                        return Some(false); // x is a failed literal
                    }
                    self.seen.set(lz);
                    nseen += 1;
                    self.queue.push_back(z);
                    if self.targets.has(lz) {
                        self.targets.clear(lz);
                        remaining -= 1;
                    }
                }
                if nseen > budget {
                    return Some(false);
                }
            }
            if remaining > 0 {
                return Some(false);
            }
        }
        Some(true)
    }

    /// Does x exclude y (x -> ... -> -y) within a small walk?
    fn excludes(&mut self, cnf: &Cnf, x: i32, y: i32, budget: usize, ck: &mut Clock) -> Option<bool> {
        self.seen.next();
        self.seen.set(li(x));
        let mut nseen = 1usize;
        self.queue.clear();
        self.queue.push_back(x);
        let mut looked = 0usize;
        let target = -y;
        while let Some(u) = self.queue.pop_front() {
            let range = cnf.bins(li(-u));
            looked += range.len();
            if looked > 16 * budget {
                return Some(false);
            }
            ck.pay(range.len() as u64 + 1)?;
            for p in range {
                let z = cnf.bin_part[p];
                if z == target {
                    return Some(true);
                }
                let lz = li(z);
                if !self.seen.has(lz) {
                    self.seen.set(lz);
                    nseen += 1;
                    self.queue.push_back(z);
                }
            }
            if nseen > budget {
                return Some(false);
            }
        }
        Some(false)
    }
}

/// The variables in an array with the one a dead lock frees first at the
/// end (for pop): no two-way definition first, then the most waiting
/// candidates, the most occurrences, the lowest number.
fn free_order(mut items: Vec<(bool, u64, u64, u32)>) -> Vec<u32> {
    items.sort_unstable_by(|a, b| (b.0, std::cmp::Reverse(b.1), std::cmp::Reverse(b.2), b.3).cmp(&(a.0, std::cmp::Reverse(a.1), std::cmp::Reverse(a.2), a.3)));
    items.into_iter().map(|t| t.3).collect()
}

#[inline]
fn subset(a: &[u32], b: &[u32]) -> bool {
    a.iter().all(|x| b.binary_search(x).is_ok())
}

#[inline]
fn intersects(a: &[u32], b: &[u32]) -> bool {
    a.iter().any(|x| b.binary_search(x).is_ok())
}

/// |a - b|
#[inline]
fn minus_len(a: &[u32], b: &[u32]) -> usize {
    a.iter().filter(|x| b.binary_search(x).is_err()).count()
}

#[inline]
fn union_len(a: &[u32], b: &[u32]) -> usize {
    a.len() + minus_len(b, a)
}

fn union(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut u: Vec<u32> = a.iter().chain(b.iter()).copied().collect();
    u.sort_unstable();
    u.dedup();
    u
}

/// Counts of values in the order each value first came, for "size x count"
/// lists sorted by count (ties in that order), as the census's Counter.
#[derive(Default)]
struct Tally {
    index: HashMap<usize, usize>,
    items: Vec<(usize, u64)>,
}

impl Tally {
    fn add(&mut self, k: usize) {
        match self.index.get(&k) {
            Some(&i) => self.items[i].1 += 1,
            None => {
                self.index.insert(k, self.items.len());
                self.items.push((k, 1));
            }
        }
    }
    fn top(&self, n: usize) -> Vec<(usize, u64)> {
        let mut v = self.items.clone();
        v.sort_by(|a, b| b.1.cmp(&a.1)); // stable
        v.truncate(n);
        v
    }
}

/// The census's columns for one reading.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub vars: u64,
    pub clauses: u64,
    pub fixed: u64,
    pub satisfied: u64,
    pub present: u64,
    pub groups: u64,
    pub group_vars: u64,
    pub group_sizes: Vec<(usize, u64)>,
    pub chain_groups: u64,
    pub amo_groups: u64,
    pub overlap_groups: u64,
    pub derived_groups: u64,
    pub defs: u64,
    pub wide_defs: u64,
    pub def_inputs_max: u64,
    pub gates: u64,
    pub helpers: u64,
    pub free: u64,
    pub cycle_breaks: u64,
    pub bits: f64,
    pub choice_share: f64,
    pub cov_group: u64,
    pub cov_def: u64,
    pub cov_gate: u64,
    pub cov_helper: u64,
    pub constraints: u64,
    pub constraint_share: f64,
    pub con_choice: u64,
    pub con_lens: Vec<(usize, u64)>,
    pub topo_numbered: bool,
}

fn tally_text(t: &[(usize, u64)]) -> String {
    let s: Vec<String> = t.iter().map(|(k, n)| format!("{}x{}", k, n)).collect();
    if s.is_empty() {
        "-".to_string()
    } else {
        s.join(",")
    }
}

impl Summary {
    /// `key=value` pairs, the lists as `size x count` joined by commas.
    pub fn line(&self) -> String {
        format!(
            "vars={} clauses={} fixed={} satisfied={} present={} groups={} group_vars={} group_sizes={} chain_groups={} amo_groups={} overlap_groups={} derived_groups={} defs={} wide_defs={} def_inputs_max={} gates={} helpers={} free={} cycle_breaks={} bits={:.1} choice_share={:.4} cov_group={} cov_def={} cov_gate={} cov_helper={} constraints={} constraint_share={:.4} con_choice={} con_lens={} topo={}",
            self.vars,
            self.clauses,
            self.fixed,
            self.satisfied,
            self.present,
            self.groups,
            self.group_vars,
            tally_text(&self.group_sizes),
            self.chain_groups,
            self.amo_groups,
            self.overlap_groups,
            self.derived_groups,
            self.defs,
            self.wide_defs,
            self.def_inputs_max,
            self.gates,
            self.helpers,
            self.free,
            self.cycle_breaks,
            self.bits,
            self.choice_share,
            self.cov_group,
            self.cov_def,
            self.cov_gate,
            self.cov_helper,
            self.constraints,
            self.constraint_share,
            self.con_choice,
            tally_text(&self.con_lens),
            self.topo_numbered as u8
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Formula from DIMACS text (header and comment lines skipped), as the
    /// structure pass builds it from the parser's literals.
    fn formula(text: &str) -> Formula {
        let mut lits = Vec::new();
        let mut max_var = 0;
        for line in text.lines() {
            let t = line.trim_start();
            if t.starts_with('p') || t.starts_with('c') {
                continue;
            }
            for tok in t.split_whitespace() {
                let x: i32 = tok.parse().expect("a literal");
                lits.push(x);
                max_var = max_var.max(x.abs());
            }
        }
        crate::structure::build_formula(&lits, max_var)
    }

    fn read_all(f: &Formula) -> Result<Reading, NotRead> {
        let stop = AtomicBool::new(false);
        let mut ticks = 0u64;
        read(f, u64::MAX, &mut ticks, &stop)
    }

    /// The toys of free_census.py selftest with the census's own summary of
    /// each (tools/free_read_check.py fixtures).
    const TOYS: &str = include_str!("../tests/fixtures/freeread_toys.txt");

    struct Toy {
        name: String,
        expect: HashMap<String, String>,
        plant: Option<Vec<i32>>,
        cnf: String,
    }

    fn toys() -> Vec<Toy> {
        let mut out = Vec::new();
        let mut cur: Option<Toy> = None;
        for line in TOYS.lines() {
            if let Some(name) = line.strip_prefix("toy ") {
                cur = Some(Toy { name: name.to_string(), expect: HashMap::new(), plant: None, cnf: String::new() });
            } else if let Some(rest) = line.strip_prefix("expect ") {
                let t = cur.as_mut().expect("a toy");
                for kv in rest.split_whitespace() {
                    let (k, v) = kv.split_once('=').expect("key=value");
                    t.expect.insert(k.to_string(), v.to_string());
                }
            } else if let Some(rest) = line.strip_prefix("plant") {
                cur.as_mut().expect("a toy").plant = Some(rest.split_whitespace().map(|x| x.parse().expect("a literal")).collect());
            } else if line == "end" {
                out.push(cur.take().expect("a toy"));
            } else if let Some(t) = cur.as_mut() {
                t.cnf.push_str(line);
                t.cnf.push('\n');
            }
        }
        out
    }

    fn fields(s: &Summary) -> HashMap<String, String> {
        s.line().split_whitespace().map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())).expect("key=value")).collect()
    }

    /// Is the assignment (indexed by variable) a model of every clause?
    fn is_model(f: &Formula, a: &[bool]) -> bool {
        (0..f.len()).all(|i| f.taut[i] || f.clause(i).iter().any(|&z| if z > 0 { a[z as usize] } else { !a[var(z)] }))
    }

    #[test]
    fn the_census_toys_read_as_the_census_reads_them() {
        let toys = toys();
        assert!(toys.len() >= 11, "the toys file holds the census's toys");
        for t in &toys {
            let f = formula(&t.cnf);
            let got = read_all(&f);
            if t.expect.get("status").map(String::as_str) == Some("refuted") {
                assert_eq!(got.err(), Some(NotRead::Refuted), "{}", t.name);
                continue;
            }
            let r = got.unwrap_or_else(|e| panic!("{}: {:?}", t.name, e));
            let s = r.summary();
            let have = fields(&s);
            for (k, want) in &t.expect {
                if k == "status" {
                    continue;
                }
                let h = have.get(k).unwrap_or_else(|| panic!("{}: no field {}", t.name, k));
                if matches!(k.as_str(), "bits" | "choice_share" | "constraint_share") {
                    let (a, b): (f64, f64) = (h.parse().unwrap(), want.parse().unwrap());
                    assert!((a - b).abs() < 1e-3, "{}: {} is {} in the census, {} here", t.name, k, want, h);
                } else {
                    assert_eq!(h, want, "{}: {} is {} in the census, {} here", t.name, k, want, h);
                }
            }
            let (bad, _) = r.simulate(1);
            assert_eq!(bad, 0, "{}: the simulation finds a covered clause false", t.name);
            assert!(r.acyclic(), "{}", t.name);
            if let Some(plant) = &t.plant {
                // the planted model's choices (and free values), computed
                // through the reading, give a model of the toy
                let truth: std::collections::HashSet<i32> = plant.iter().copied().collect();
                let holds = |x: i32| if x > 0 { truth.contains(&x) } else { !truth.contains(&-x) };
                let mut choice = Vec::new();
                for (g, mem) in r.groups.iter().enumerate() {
                    let on: Vec<usize> = (0..mem.len()).filter(|&j| holds(mem[j])).collect();
                    match on.len() {
                        1 => choice.push(on[0]),
                        0 if r.none_ok[g] => choice.push(mem.len()),
                        _ => panic!("{}: the plant has {} members of group {} true", t.name, on.len(), g),
                    }
                }
                let a = r.assignment(&choice, &|v| truth.contains(&(v as i32)));
                assert!(is_model(&f, &a), "{}: the planted choices do not give a model", t.name);
            }
        }
    }

    #[test]
    fn an_empty_clause_or_a_unit_conflict_refutes() {
        assert_eq!(read_all(&formula("1 2 0\n0\n")).err(), Some(NotRead::Refuted));
        assert_eq!(read_all(&formula("1 0\n-1 2 0\n-2 3 0\n-3 0\n")).err(), Some(NotRead::Refuted));
    }

    #[test]
    fn tautologies_and_repeated_literals_are_read_as_the_census_reads_them() {
        // (1 | -1 | 2) is left out, (3 | 3 | 4) is (3 | 4)
        let f = formula("p cnf 4 3\n1 -1 2 0\n3 3 4 0\n-3 -4 0\n");
        let r = read_all(&f).unwrap();
        let s = r.summary();
        assert_eq!(s.clauses, 2);
        assert_eq!(r.cnf.clause(0), &[3, 4]);
    }

    #[test]
    fn the_budget_stops_the_reading_and_nothing_past_it_is_charged() {
        let t = toys().into_iter().find(|t| t.name == "lockchart").expect("the lockchart toy");
        let f = formula(&t.cnf);
        let stop = AtomicBool::new(false);
        let mut full = 0u64;
        assert!(read(&f, u64::MAX, &mut full, &stop).is_ok());
        for frac in [1u64, 10, 50, 90, 99] {
            let limit = full * frac / 100;
            let mut ticks = 0u64;
            assert_eq!(read(&f, limit, &mut ticks, &stop).err(), Some(NotRead::Budget), "{}%", frac);
            assert!(ticks <= limit, "{}%: {} ticks over a limit of {}", frac, ticks, limit);
        }
        let mut again = 0u64;
        assert!(read(&f, full, &mut again, &stop).is_ok());
        assert_eq!(again, full, "the same reading costs the same work");
        let flagged = AtomicBool::new(true);
        let mut ticks = 0u64;
        assert_eq!(read(&f, u64::MAX, &mut ticks, &flagged).err(), Some(NotRead::Budget), "the stop flag stops it");
    }

    #[test]
    fn many_copies_of_one_group_read_in_linear_work() {
        // 40,000 copies of (1 | 2 | 3) with the three exclusions: one group,
        // the other copies overlap it. Asking per candidate whether a member
        // is defined elsewhere would scan every copy for every copy (4.8
        // billion checks, Codex review of d96.2.4.1); per variable it is one
        // pass.
        let mut text = String::from("-1 -2 0\n-1 -3 0\n-2 -3 0\n");
        for _ in 0..40_000 {
            text.push_str("1 2 3 0\n");
        }
        let f = formula(&text);
        let started = std::time::Instant::now();
        let r = read_all(&f).unwrap();
        let s = r.summary();
        assert_eq!((s.groups, s.overlap_groups, s.derived_groups), (1, 39_999, 0));
        assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    }

    #[test]
    fn the_tables_per_variable_are_paid_for_before_they_are_made() {
        // one clause on variable 10,000,000: a small limit stops the reading
        // before its tables are made, and a reading that runs pays for them
        let f = formula("10000000 1 0\n");
        let stop = AtomicBool::new(false);
        let mut ticks = 0u64;
        assert_eq!(read(&f, 1_000, &mut ticks, &stop).err(), Some(NotRead::Budget));
        assert!(ticks <= 1_000, "{}", ticks);
        let mut ticks = 0u64;
        assert!(read(&f, u64::MAX, &mut ticks, &stop).is_ok());
        assert!(ticks >= 4 * 10_000_000 / UNITS_PER_TICK, "{}", ticks);
    }

    #[test]
    fn the_reading_counts_sixteen_units_to_a_tick() {
        // 1,000 units paid one at a time are 62 ticks and 8 units over; a
        // payment the limit does not cover charges nothing
        let stop = AtomicBool::new(false);
        let mut ticks = 5u64;
        let mut ck = Clock::new(&mut ticks, 1_000, &stop);
        for _ in 0..1_000 {
            ck.pay(1).unwrap();
        }
        assert_eq!((*ck.ticks, ck.units), (5 + 62, 8));
        assert!(ck.pay(16 * 10_000).is_none());
        assert_eq!((*ck.ticks, ck.units), (5 + 62, 8));
        ck.pay(16 * (1_000 - 67) - 8).unwrap();
        assert_eq!((*ck.ticks, ck.units), (1_000, 0));
        assert!(ck.pay(16).is_none());
        // the stop flag ends a reading within 1,024 units
        let stop = AtomicBool::new(true);
        let mut ticks = 0u64;
        let mut ck = Clock::new(&mut ticks, u64::MAX, &stop);
        let mut paid = 0;
        while ck.pay(1).is_some() {
            paid += 1;
        }
        assert!(paid < 1_024, "{}", paid);
    }

    #[test]
    fn the_dead_lock_order_is_the_census_order() {
        // no two-way definition first, then the most waiting candidates, the
        // most occurrences, the lowest number
        let a = (1u32 << 26) + 1;
        let b = (1u32 << 30) + 5;
        let c = u32::MAX >> 1;
        let mut fo = free_order(vec![(false, 5, 10, a), (false, 7, 3, b), (true, 100, 100, c), (false, 7, 3, 3)]);
        let mut got = Vec::new();
        while let Some(v) = fo.pop() {
            got.push(v);
        }
        assert_eq!(got, vec![3, b, a, c]);
    }
}
