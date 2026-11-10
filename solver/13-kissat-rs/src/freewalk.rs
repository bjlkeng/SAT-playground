//! Not in kissat. The free-choice walk (bead SAT-playground-d96.2.4.2,
//! G.8.4b; epic SAT-playground-d96.2; solver README "Free choices: the walk
//! in the solver"): a local search over a formula's real choices, the
//! prototype tools/free-walk/src/main.rs ported into the solver.
//!
//! The reading (src/freeread.rs) gives every variable a role: a choice in a
//! group, a free variable, or a variable computed from others (a two-way OR,
//! a small gate, or a one-way helper set to its best value). The clauses no
//! role makes true are the constraints. The walk's state is one value per
//! group (its true member, or none in an at-most-one group) and one bit per
//! free variable. Every computed variable follows from them and is updated
//! level by level after each move, so the count of false constraints is exact
//! at every step. A step takes a false constraint at random, walks back from
//! it to up to 16 groups or free bits whose change could make it true, tries
//! every other value of each (make the move, read the change, undo it) and
//! takes the best by weighted change, or a random one in 5% of the steps.
//! Clause weights alternate with plain phases, a phase's length counted in the
//! walk's own work.
//!
//! How it fits in. It runs where `freewalk` says: once before the search (1),
//! in turns with the search at a share of the work clock (2), or in kissat's
//! walking rephase slots at that share (3). It never touches the search's
//! assignment, phases or clauses, and it counts its work in the work clock
//! (`statistics.ticks`), so when it finds nothing the search goes on as it
//! would have, only later. Its trajectory depends on the reading, the seed
//! and its settings alone: where it runs decides only how much of it runs. A
//! model it finds is answered only after it satisfies every clause of the
//! input as parsed.
//!
//! Work. Building the walk is paid for before it is done, within the
//! reading's limit. A step's work is counted as it is done, like the
//! search's ticks, in units (a flip and each place that reads the flipped
//! variable, a node computed, a hop of a walk back, a clause weighed), and a
//! walk stops between steps once its budget is reached, so it runs past it
//! by at most one step. `UNITS_PER_TICK` units make one tick.

use crate::freeread::{Reading, CHOICE, C_NONE, DEF, FREE, GATE, HELPER};
use crate::internal::Solver;
use std::sync::atomic::{AtomicBool, Ordering};

const K_ABSENT: u8 = 0;
const K_FIXED: u8 = 1;
const K_CHOICE: u8 = 2;
const K_FREE: u8 = 3;
const K_OR: u8 = 4;
const K_GATE: u8 = 5;
const K_HELPER: u8 = 6;

// who reads a variable: an OR input, a helper clause, a gate support, a constraint clause
const R_OR: u8 = 0;
const R_HC: u8 = 1;
const R_GATE: u8 = 2;
const R_CON: u8 = 3;

/// Hops of a walk back from a false literal to a group or free bit.
const WALK_DEPTH: usize = 100_000;

/// Walk work units per tick of the work clock, measured so that a tick of
/// the walk takes about as long as a tick of the search on the lockchart
/// cells (solver README "Free choices: the walk in the solver"): the walk
/// does 153M units a second on group 2 and 65M on group 1, the shipped
/// search 23.5M and 7.3M ticks a second (6.5 and 8.9 units a tick). With 8,
/// a walk tick takes 1.2 times a search tick on group 2 and 0.9 times on
/// group 1.
pub const UNITS_PER_TICK: u64 = 8;

/// Literal code: 2v for v, 2v + 1 for -v.
#[inline]
fn code(l: i32) -> u32 {
    if l > 0 {
        2 * l as u32
    } else {
        2 * l.unsigned_abs() + 1
    }
}
#[inline]
fn lvar(c: u32) -> usize {
    (c >> 1) as usize
}
#[inline]
fn lneg(c: u32) -> u8 {
    (c & 1) as u8
}

/// The prototype's random numbers (splitmix64), seeded from the solver's seed;
/// the search's own generator is never touched.
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut r = Rng(seed ^ 0x5DEE_CE66_D1CE_4E5B);
        r.next();
        r
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in 0..n (n below 2^32).
    fn below(&mut self, n: usize) -> usize {
        (((self.next() >> 32) * n as u64) >> 32) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// The work clock while the walk is built: work is paid for before it is
/// done, in the reading's units (`freeread::UNITS_PER_TICK` to a tick: the
/// build runs at about the reading's speed, 225-546M units a second on the
/// cells of bead SAT-playground-d96.2.5's pilot).
const BUILD_UNITS_PER_TICK: u64 = crate::freeread::UNITS_PER_TICK;

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
        let t = self.ticks.saturating_add(u / BUILD_UNITS_PER_TICK);
        if t > self.budget {
            return None;
        }
        *self.ticks = t;
        self.units = u % BUILD_UNITS_PER_TICK;
        // the stop flag once every 1,024 units
        if t - self.last >= 1024 / BUILD_UNITS_PER_TICK {
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

// ---------------------------------------------------------------------------
// the reading, as the walk reads it
// ---------------------------------------------------------------------------

struct Model {
    nv: usize,
    kind: Vec<u8>,
    fixed: Vec<u8>,
    g_start: Vec<u32>, // group g's members: g_lit[g_start[g]..g_start[g + 1]]
    g_lit: Vec<u32>,
    g_none: Vec<u8>, // an at-most-one group: "none" is a value too (the last one)
    var_group: Vec<u32>,
    free: Vec<u32>,
    var_free: Vec<u32>,
    n_out: Vec<u32>,
    n_kind: Vec<u8>,
    n_neg: Vec<u8>, // OR: the output literal is negative; helper: the helper literal is
    n_tt: Vec<u32>,
    n_start: Vec<u32>, // node n's inputs: n_in[n_start[n]..n_start[n + 1]]
    n_in: Vec<u32>,    // OR: input literal codes; gate: support variables; helper: its helper clauses
    var_node: Vec<u32>,
    hc_start: Vec<u32>, // a helper clause's other literals
    hc_lit: Vec<u32>,
    hc_node: Vec<u32>,
    c_start: Vec<u32>,
    c_lit: Vec<u32>,
    r_start: Vec<u32>, // readers of variable v: r_tgt/r_kn[r_start[v]..r_start[v + 1]]
    r_tgt: Vec<u32>,
    r_kn: Vec<u8>,   // kind | neg << 2
    r_aux: Vec<u32>, // a helper clause's node
    n_level: Vec<u32>, // 1 + the highest level of a node's inputs (the sources are level 0)
    max_level: usize,
}

/// Why there is no walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoWalk {
    /// The work limit ran out, or the run was told to stop.
    Budget,
    /// The reading is not one the walk can take (a variable with two roles, a
    /// node that reads a later variable, an empty constraint, too many
    /// literals for its indices): a bug in the reading, never a guess.
    Malformed,
}

impl Model {
    fn ngroups(&self) -> usize {
        self.g_none.len()
    }
    fn gsize(&self, g: usize) -> usize {
        (self.g_start[g + 1] - self.g_start[g]) as usize
    }
    /// Values of group g: its members, and "none" (the last) in an at-most-one group.
    fn gvalues(&self, g: usize) -> usize {
        self.gsize(g) + self.g_none[g] as usize
    }
    fn nnodes(&self) -> usize {
        self.n_out.len()
    }
    fn ncons(&self) -> usize {
        self.c_start.len() - 1
    }

    /// The walk's tables from the reading, in the prototype's order (its
    /// export, tools/free_walk.py `reading_array`): the units, the groups,
    /// the free variables and the computed variables in the order they were
    /// settled, then the constraints in clause order.
    fn build(r: &Reading, ck: &mut Clock) -> Result<Model, NoWalk> {
        use NoWalk::*;
        let cnf = &r.cnf;
        let nv = cnf.nv;
        let m = cnf.m();
        let fits = |n: usize| if n < u32::MAX as usize { Ok(()) } else { Err(Malformed) };
        // the tables per variable, paid for before they are made
        ck.pay(7 * (nv as u64 + 1)).ok_or(Budget)?;
        let mut kind = vec![K_ABSENT; nv + 1];
        let mut fixed = vec![0u8; nv + 1];
        for v in 1..=nv {
            ck.poll(v).ok_or(Budget)?;
            match r.root.get(v) {
                Some(1) => {
                    kind[v] = K_FIXED;
                    fixed[v] = 1;
                }
                Some(2) => kind[v] = K_FIXED,
                _ => {}
            }
        }
        ck.pay(r.groups.len() as u64 + 1).ok_or(Budget)?;
        let members: usize = r.groups.iter().map(|g| g.len()).sum();
        fits(members)?;
        ck.pay(2 * members as u64 + r.groups.len() as u64).ok_or(Budget)?;
        let mut g_start = Vec::with_capacity(r.groups.len() + 1);
        g_start.push(0u32);
        let mut g_lit = Vec::with_capacity(members);
        let mut g_none = Vec::with_capacity(r.groups.len());
        let mut var_group = vec![u32::MAX; nv + 1];
        for (g, mem) in r.groups.iter().enumerate() {
            for &x in mem {
                let c = code(x);
                let v = lvar(c);
                if v > nv || kind[v] != K_ABSENT {
                    return Err(Malformed);
                }
                kind[v] = K_CHOICE;
                var_group[v] = g as u32;
                g_lit.push(c);
            }
            g_start.push(g_lit.len() as u32);
            g_none.push(r.none_ok[g] as u8);
        }
        // the free variables, then the computed ones, in the order they were settled
        ck.pay(2 * r.order.len() as u64).ok_or(Budget)?;
        let mut free = Vec::new();
        let mut var_free = vec![u32::MAX; nv + 1];
        for &v in &r.order {
            let v = v as usize;
            if r.role[v] == FREE {
                if kind[v] != K_ABSENT {
                    return Err(Malformed);
                }
                kind[v] = K_FREE;
                var_free[v] = free.len() as u32;
                free.push(v as u32);
            } else if r.role[v] == CHOICE && kind[v] != K_CHOICE {
                return Err(Malformed);
            }
        }
        let mut n_out = Vec::new();
        let mut n_kind = Vec::new();
        let mut n_neg = Vec::new();
        let mut n_tt = Vec::new();
        let mut n_start = vec![0u32];
        let mut n_in: Vec<u32> = Vec::new();
        let mut var_node = vec![u32::MAX; nv + 1];
        let mut hc_start = vec![0u32];
        let mut hc_lit: Vec<u32> = Vec::new();
        let mut hc_node: Vec<u32> = Vec::new();
        // an input must be settled before the node that reads it
        let settled = |kind: &Vec<u8>, v: usize| v <= nv && kind[v] != K_ABSENT;
        for (i_order, &v) in r.order.iter().enumerate() {
            ck.poll(i_order).ok_or(Budget)?;
            let v = v as usize;
            let role = r.role[v];
            if !matches!(role, DEF | GATE | HELPER) {
                continue;
            }
            let n = n_out.len();
            let i = r.acc[v];
            if i < 0 {
                return Err(Malformed);
            }
            let i = i as usize;
            let (out, neg, k) = match role {
                DEF => {
                    // -x is the OR of the clause's other literals
                    let (c, x) = (r.c_a[i] as usize, r.c_b[i]);
                    let cl = cnf.clause(c);
                    ck.pay(cl.len() as u64 + 1).ok_or(Budget)?;
                    for &y in cl {
                        if y != x {
                            let cy = code(y);
                            if !settled(&kind, lvar(cy)) {
                                return Err(Malformed);
                            }
                            n_in.push(cy);
                        }
                    }
                    let o = code(-x);
                    n_tt.push(0);
                    (lvar(o), lneg(o), K_OR)
                }
                GATE => {
                    let g = &r.gates[r.c_a[i] as usize];
                    ck.pay(g.support.len() as u64 + 1).ok_or(Budget)?;
                    if g.support.is_empty() || g.support.len() > 4 {
                        return Err(Malformed);
                    }
                    for &u in &g.support {
                        if !settled(&kind, u as usize) {
                            return Err(Malformed);
                        }
                        n_in.push(u);
                    }
                    n_tt.push(g.tt);
                    (v, 0, K_GATE)
                }
                _ => {
                    // the helper literal y, true exactly when one of its clauses
                    // has every other literal false
                    let y = r.c_a[i] as i32;
                    let occs = cnf.occs(y);
                    if occs.len() > 255 {
                        return Err(Malformed); // the counters are bytes
                    }
                    for &c in occs {
                        let cl = cnf.clause(c as usize);
                        ck.pay(cl.len() as u64 + 1).ok_or(Budget)?;
                        if cl.len() > 256 {
                            return Err(Malformed);
                        }
                        for &z in cl {
                            if z != y {
                                let cz = code(z);
                                if !settled(&kind, lvar(cz)) {
                                    return Err(Malformed);
                                }
                                hc_lit.push(cz);
                            }
                        }
                        n_in.push(hc_node.len() as u32);
                        hc_node.push(n as u32);
                        hc_start.push(hc_lit.len() as u32);
                    }
                    n_tt.push(0);
                    let cy = code(y);
                    (lvar(cy), lneg(cy), K_HELPER)
                }
            };
            fits(n_in.len())?;
            fits(hc_lit.len())?;
            if out != v || kind[out] != K_ABSENT {
                return Err(Malformed);
            }
            kind[out] = k;
            var_node[out] = n as u32;
            n_out.push(out as u32);
            n_kind.push(k);
            n_neg.push(neg);
            n_start.push(n_in.len() as u32);
        }
        ck.pay(m as u64).ok_or(Budget)?;
        let mut c_start = vec![0u32];
        let mut c_lit = Vec::new();
        for c in 0..m {
            ck.poll(c).ok_or(Budget)?;
            if r.cover[c] != C_NONE {
                continue;
            }
            let cl = cnf.clause(c);
            ck.pay(cl.len() as u64).ok_or(Budget)?;
            if cl.is_empty() {
                return Err(Malformed);
            }
            for &z in cl {
                let cz = code(z);
                if !settled(&kind, lvar(cz)) {
                    return Err(Malformed);
                }
                c_lit.push(cz);
            }
            fits(c_lit.len())?;
            c_start.push(c_lit.len() as u32);
        }
        let mut model = Model {
            nv,
            kind,
            fixed,
            g_start,
            g_lit,
            g_none,
            var_group,
            free,
            var_free,
            n_out,
            n_kind,
            n_neg,
            n_tt,
            n_start,
            n_in,
            var_node,
            hc_start,
            hc_lit,
            hc_node,
            c_start,
            c_lit,
            r_start: Vec::new(),
            r_tgt: Vec::new(),
            r_kn: Vec::new(),
            r_aux: Vec::new(),
            n_level: Vec::new(),
            max_level: 0,
        };
        model.build_readers(ck)?;
        model.build_levels(ck)?;
        Ok(model)
    }

    /// Every input of every node, helper clause and constraint, with its kind.
    fn each_reader(&self, f: &mut dyn FnMut(usize, u32, u32, u8)) {
        for n in 0..self.n_out.len() {
            let (a, b) = (self.n_start[n] as usize, self.n_start[n + 1] as usize);
            match self.n_kind[n] {
                K_OR => {
                    for &c in &self.n_in[a..b] {
                        f(lvar(c), n as u32, 0, R_OR | lneg(c) << 2);
                    }
                }
                K_GATE => {
                    for &u in &self.n_in[a..b] {
                        f(u as usize, n as u32, 0, R_GATE);
                    }
                }
                _ => {
                    for &h in &self.n_in[a..b] {
                        let h = h as usize;
                        for &c in &self.hc_lit[self.hc_start[h] as usize..self.hc_start[h + 1] as usize] {
                            f(lvar(c), h as u32, n as u32, R_HC | lneg(c) << 2);
                        }
                    }
                }
            }
        }
        for ci in 0..self.c_start.len() - 1 {
            for &c in &self.c_lit[self.c_start[ci] as usize..self.c_start[ci + 1] as usize] {
                f(lvar(c), ci as u32, 0, R_CON | lneg(c) << 2);
            }
        }
    }

    fn build_readers(&mut self, ck: &mut Clock) -> Result<(), NoWalk> {
        let nv = self.nv;
        let entries = self.n_in.len() as u64 + self.hc_lit.len() as u64 + self.c_lit.len() as u64;
        if entries >= u32::MAX as u64 {
            return Err(NoWalk::Malformed);
        }
        // two passes over every input, the count table, then the three tables
        ck.pay(2 * entries + 2 * (nv as u64 + 2) + 9 * entries / 4 + 1).ok_or(NoWalk::Budget)?;
        let mut cnt = vec![0u32; nv + 2];
        self.each_reader(&mut |v, _, _, _| cnt[v + 1] += 1);
        for v in 1..nv + 2 {
            cnt[v] += cnt[v - 1];
        }
        let total = cnt[nv + 1] as usize;
        let mut tgt = vec![0u32; total];
        let mut kn = vec![0u8; total];
        let mut aux = vec![0u32; total];
        let mut fill = cnt.clone();
        self.each_reader(&mut |v, t, x, k| {
            let p = fill[v] as usize;
            tgt[p] = t;
            aux[p] = x;
            kn[p] = k;
            fill[v] += 1;
        });
        self.r_start = cnt;
        self.r_tgt = tgt;
        self.r_kn = kn;
        self.r_aux = aux;
        Ok(())
    }

    /// Levels: a node's inputs all sit on lower levels, so the queue can
    /// take the nodes level by level (a bucket per level, no heap).
    fn build_levels(&mut self, ck: &mut Clock) -> Result<(), NoWalk> {
        let nn = self.n_out.len();
        ck.pay(nn as u64 + self.n_in.len() as u64 + self.hc_lit.len() as u64).ok_or(NoWalk::Budget)?;
        let mut lv = vec![0u32; nn];
        let mut maxl = 0u32;
        for n in 0..nn {
            let (a, b) = (self.n_start[n] as usize, self.n_start[n + 1] as usize);
            let mut l = 0u32;
            let mut see = |u: usize| {
                let w = self.var_node[u];
                if w != u32::MAX && (w as usize) < n {
                    l = l.max(lv[w as usize]);
                }
            };
            match self.n_kind[n] {
                K_OR => self.n_in[a..b].iter().for_each(|&c| see(lvar(c))),
                K_GATE => self.n_in[a..b].iter().for_each(|&u| see(u as usize)),
                _ => {
                    for &h in &self.n_in[a..b] {
                        let h = h as usize;
                        for &c in &self.hc_lit[self.hc_start[h] as usize..self.hc_start[h + 1] as usize] {
                            see(lvar(c));
                        }
                    }
                }
            }
            lv[n] = l + 1;
            maxl = maxl.max(l + 1);
        }
        self.n_level = lv;
        self.max_level = maxl as usize;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// the walk's state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Src {
    Group(u32),
    Free(u32),
}

#[derive(Clone, Copy)]
struct Cand {
    src: Src,
    value: u32, // the group's new value (unused for a free bit)
    dw: i64,    // weighted cost change
    dn: i64,    // false clause count change
    br: i64,    // weight of the clauses it makes false (its break)
}

struct State {
    val: Vec<u8>,
    g_cur: Vec<u32>,
    or_cnt: Vec<u32>, // per node: true inputs of an OR
    hz: Vec<u8>,      // per node: a helper's clauses with no other literal true
    hc_cnt: Vec<u8>,  // per helper clause: other literals true
    con_cnt: Vec<u32>,
    w: Vec<u64>,
    cost: u64,
    fl: Vec<u32>, // false constraint clauses
    fpos: Vec<u32>,
    buckets: Vec<Vec<u32>>, // queued nodes by level
    lo: usize,
    hi: usize,
    in_heap: Vec<bool>,
    trail: Vec<u32>,
    flips: u64,
    upd: u64,
    pushes: u64,
    /// The walk's work so far, in units.
    units: u64,
    // the constraint clauses a move touches, each with whether it was true
    // before the move: its break is the weight of those true before and
    // false after (a clause false only midway through the move is no break)
    con_mark: Vec<u32>,
    epoch: u32,
    touched: Vec<(u32, bool)>,
}

impl State {
    /// The tables, a random value per group (members, then "none") and per
    /// free bit, and everything else computed from them.
    fn new(m: &Model, rng: &mut Rng) -> State {
        let nv = m.nv;
        let nn = m.nnodes();
        let nc = m.ncons();
        let mut s = State {
            val: vec![0u8; nv + 1],
            g_cur: vec![0u32; m.ngroups()],
            or_cnt: vec![0u32; nn],
            hz: vec![0u8; nn],
            hc_cnt: vec![0u8; m.hc_node.len()],
            con_cnt: vec![0u32; nc],
            w: vec![1u64; nc],
            cost: 0,
            fl: Vec::new(),
            fpos: vec![u32::MAX; nc],
            buckets: vec![Vec::new(); m.max_level + 1],
            lo: usize::MAX,
            hi: 0,
            in_heap: vec![false; nn],
            trail: Vec::new(),
            flips: 0,
            upd: 0,
            pushes: 0,
            units: 0,
            con_mark: vec![0u32; nc],
            epoch: 0,
            touched: Vec::new(),
        };
        for v in 1..=nv {
            if m.kind[v] == K_FIXED {
                s.val[v] = m.fixed[v];
            }
        }
        for g in 0..m.ngroups() {
            s.g_cur[g] = rng.below(m.gvalues(g)) as u32;
        }
        for &v in &m.free {
            s.val[v as usize] = (rng.next() >> 63) as u8;
        }
        s.recompute(m);
        s
    }

    #[inline]
    fn lit_true(&self, c: u32) -> bool {
        (self.val[lvar(c)] ^ lneg(c)) == 1
    }

    /// Every computed variable and every count from the sources, from scratch.
    fn recompute(&mut self, m: &Model) {
        for g in 0..m.ngroups() {
            let a = m.g_start[g] as usize;
            let cur = self.g_cur[g] as usize;
            for j in 0..m.gsize(g) {
                let c = m.g_lit[a + j];
                self.val[lvar(c)] = (j == cur) as u8 ^ lneg(c);
            }
        }
        for n in 0..m.nnodes() {
            let v = self.node_value(m, n, true);
            self.val[m.n_out[n] as usize] = v;
        }
        self.cost = 0;
        self.fl.clear();
        for ci in 0..m.ncons() {
            let k = m.c_lit[m.c_start[ci] as usize..m.c_start[ci + 1] as usize].iter().filter(|&&c| self.lit_true(c)).count() as u32;
            self.con_cnt[ci] = k;
            self.fpos[ci] = u32::MAX;
            if k == 0 {
                self.fpos[ci] = self.fl.len() as u32;
                self.fl.push(ci as u32);
                self.cost += self.w[ci];
            }
        }
    }

    /// The value node n computes; with `count` the node's counts are set from
    /// its inputs first (else they are trusted).
    fn node_value(&mut self, m: &Model, n: usize, count: bool) -> u8 {
        let (a, b) = (m.n_start[n] as usize, m.n_start[n + 1] as usize);
        match m.n_kind[n] {
            K_OR => {
                if count {
                    self.or_cnt[n] = m.n_in[a..b].iter().filter(|&&c| self.lit_true(c)).count() as u32;
                }
                (self.or_cnt[n] > 0) as u8 ^ m.n_neg[n]
            }
            K_GATE => {
                let mut idx = 0usize;
                for (k, &u) in m.n_in[a..b].iter().enumerate() {
                    idx |= (self.val[u as usize] as usize) << k;
                }
                ((m.n_tt[n] >> idx) & 1) as u8
            }
            _ => {
                if count {
                    let mut z = 0u8;
                    for &h in &m.n_in[a..b] {
                        let h = h as usize;
                        let k = m.hc_lit[m.hc_start[h] as usize..m.hc_start[h + 1] as usize].iter().filter(|&&c| self.lit_true(c)).count() as u32;
                        self.hc_cnt[h] = k as u8;
                        z += (k == 0) as u8;
                    }
                    self.hz[n] = z;
                }
                (self.hz[n] > 0) as u8 ^ m.n_neg[n]
            }
        }
    }

    /// The incremental state against a recomputation from the sources.
    fn check(&self, m: &Model) -> Result<(), String> {
        let mut t = State {
            val: self.val.clone(),
            g_cur: self.g_cur.clone(),
            or_cnt: vec![0; self.or_cnt.len()],
            hz: vec![0; self.hz.len()],
            hc_cnt: vec![0; self.hc_cnt.len()],
            con_cnt: vec![0; self.con_cnt.len()],
            w: self.w.clone(),
            cost: 0,
            fl: Vec::new(),
            fpos: vec![u32::MAX; self.fpos.len()],
            buckets: Vec::new(),
            lo: usize::MAX,
            hi: 0,
            in_heap: Vec::new(),
            trail: Vec::new(),
            flips: 0,
            upd: 0,
            pushes: 0,
            units: 0,
            con_mark: Vec::new(),
            epoch: 0,
            touched: Vec::new(),
        };
        t.recompute(m);
        for v in 1..=m.nv {
            if t.val[v] != self.val[v] {
                return Err(format!("variable {v}: {} incrementally, {} recomputed", self.val[v], t.val[v]));
            }
        }
        for n in 0..m.nnodes() {
            if m.n_kind[n] == K_OR && t.or_cnt[n] != self.or_cnt[n] {
                return Err(format!("OR node {n}: count {} v {}", self.or_cnt[n], t.or_cnt[n]));
            }
            if m.n_kind[n] == K_HELPER && t.hz[n] != self.hz[n] {
                return Err(format!("helper node {n}: zero count {} v {}", self.hz[n], t.hz[n]));
            }
        }
        if t.hc_cnt != self.hc_cnt {
            return Err("helper clause counts differ".into());
        }
        if t.con_cnt != self.con_cnt {
            return Err("constraint counts differ".into());
        }
        if t.cost != self.cost || t.fl.len() != self.fl.len() {
            return Err(format!("cost {} / {} false v {} / {}", self.cost, self.fl.len(), t.cost, t.fl.len()));
        }
        for (i, &c) in self.fl.iter().enumerate() {
            if self.fpos[c as usize] != i as u32 || self.con_cnt[c as usize] != 0 {
                return Err("the false list is out of step".into());
            }
        }
        Ok(())
    }

    #[inline]
    fn push(&mut self, n: usize, l: usize) {
        if !self.in_heap[n] {
            self.pushes += 1;
            self.units += 1;
            self.in_heap[n] = true;
            self.buckets[l].push(n as u32);
            self.lo = self.lo.min(l);
            self.hi = self.hi.max(l);
        }
    }

    /// Flip variable v and update everything that reads it; with `sched`
    /// the computed variables whose value may change are queued.
    fn flip(&mut self, m: &Model, v: usize, sched: bool) {
        self.val[v] ^= 1;
        self.flips += 1;
        let b = self.val[v];
        let (a, e) = (m.r_start[v] as usize, m.r_start[v + 1] as usize);
        self.upd += (e - a) as u64;
        self.units += 1 + (e - a) as u64;
        for p in a..e {
            let t = m.r_tgt[p] as usize;
            let kn = m.r_kn[p];
            let now = (b ^ (kn >> 2)) == 1;
            match kn & 3 {
                R_OR => {
                    if now {
                        self.or_cnt[t] += 1;
                        if self.or_cnt[t] == 1 && sched {
                            self.push(t, m.n_level[t] as usize);
                        }
                    } else {
                        self.or_cnt[t] -= 1;
                        if self.or_cnt[t] == 0 && sched {
                            self.push(t, m.n_level[t] as usize);
                        }
                    }
                }
                R_HC => {
                    let n = m.r_aux[p] as usize;
                    if now {
                        self.hc_cnt[t] += 1;
                        if self.hc_cnt[t] == 1 {
                            self.hz[n] -= 1;
                            if self.hz[n] == 0 && sched {
                                self.push(n, m.n_level[n] as usize);
                            }
                        }
                    } else {
                        self.hc_cnt[t] -= 1;
                        if self.hc_cnt[t] == 0 {
                            self.hz[n] += 1;
                            if self.hz[n] == 1 && sched {
                                self.push(n, m.n_level[n] as usize);
                            }
                        }
                    }
                }
                R_GATE => {
                    if sched {
                        self.push(t, m.n_level[t] as usize);
                    }
                }
                _ => {
                    if sched && self.con_mark[t] != self.epoch {
                        self.con_mark[t] = self.epoch;
                        self.touched.push((t as u32, self.con_cnt[t] > 0));
                    }
                    if now {
                        self.con_cnt[t] += 1;
                        if self.con_cnt[t] == 1 {
                            // satisfied: out of the false list
                            let i = self.fpos[t] as usize;
                            let last = self.fl.pop().unwrap();
                            if last as usize != t {
                                self.fl[i] = last;
                                self.fpos[last as usize] = i as u32;
                            }
                            self.fpos[t] = u32::MAX;
                            self.cost -= self.w[t];
                        }
                    } else {
                        self.con_cnt[t] -= 1;
                        if self.con_cnt[t] == 0 {
                            self.fpos[t] = self.fl.len() as u32;
                            self.fl.push(t as u32);
                            self.cost += self.w[t];
                        }
                    }
                }
            }
        }
    }

    /// Take the queued nodes level by level: a node's inputs sit on lower
    /// levels, so each is computed once, from final inputs.
    fn propagate(&mut self, m: &Model) {
        while self.lo <= self.hi {
            let l = self.lo;
            let Some(n) = self.buckets[l].pop() else {
                self.lo += 1;
                continue;
            };
            let n = n as usize;
            self.in_heap[n] = false;
            self.units += 1 + if m.n_kind[n] == K_GATE { (m.n_start[n + 1] - m.n_start[n]) as u64 } else { 0 };
            let v = m.n_out[n] as usize;
            let want = self.node_value(m, n, false);
            if want != self.val[v] {
                self.trail.push(v as u32);
                self.flip(m, v, true);
            }
        }
        self.lo = usize::MAX;
        self.hi = 0;
    }

    /// Make the move and everything it changes; returns the old group value
    /// (0 for a free bit). The flipped variables are on the trail.
    fn apply(&mut self, m: &Model, src: Src, value: u32) -> u32 {
        self.trail.clear();
        self.touched.clear();
        if self.epoch == u32::MAX {
            self.units += self.con_mark.len() as u64;
            self.con_mark.iter_mut().for_each(|x| *x = 0);
            self.epoch = 0;
        }
        self.epoch += 1;
        let old;
        match src {
            Src::Group(g) => {
                let g = g as usize;
                old = self.g_cur[g];
                let a = m.g_start[g] as usize;
                let k = m.gsize(g) as u32;
                if old < k {
                    let v = lvar(m.g_lit[a + old as usize]);
                    self.trail.push(v as u32);
                    self.flip(m, v, true);
                }
                if value < k {
                    let v = lvar(m.g_lit[a + value as usize]);
                    self.trail.push(v as u32);
                    self.flip(m, v, true);
                }
                self.g_cur[g] = value;
            }
            Src::Free(f) => {
                old = 0;
                let v = m.free[f as usize] as usize;
                self.trail.push(v as u32);
                self.flip(m, v, true);
            }
        }
        self.propagate(m);
        old
    }

    /// The weight of the constraint clauses the last move made false that
    /// were true before it.
    fn last_break(&mut self) -> i64 {
        self.units += self.touched.len() as u64;
        self.touched.iter().filter(|&&(c, was)| was && self.con_cnt[c as usize] == 0).map(|&(c, _)| self.w[c as usize] as i64).sum()
    }

    /// Which constraint clauses are true now (for the check mode).
    fn true_now(&self) -> Vec<bool> {
        self.con_cnt.iter().map(|&c| c > 0).collect()
    }

    /// The check mode: the move's break against the clauses true before and false after.
    fn check_break(&self, before: &[bool], br: i64) -> Result<(), String> {
        let want: i64 = (0..before.len()).filter(|&c| before[c] && self.con_cnt[c] == 0).map(|c| self.w[c] as i64).sum();
        if want != br {
            return Err(format!("a move's break is {br}, but {want} by the clauses true before and false after"));
        }
        Ok(())
    }

    fn undo(&mut self, m: &Model, src: Src, old: u32) {
        while let Some(v) = self.trail.pop() {
            self.flip(m, v as usize, false);
        }
        if let Src::Group(g) = src {
            self.g_cur[g as usize] = old;
        }
    }

    /// Walk back from a literal of false clause ci to a group or free bit
    /// whose change could make it true. The target literal is false at every
    /// step of the walk. The value asked for comes with a group: Some(member)
    /// when the walk needs that member true, None when it needs the true
    /// member false. The hops and the literals looked at go to `units`.
    fn pick_source(&self, m: &Model, ci: usize, rng: &mut Rng, units: &mut u64) -> Option<(Src, Option<u32>)> {
        let (a, b) = (m.c_start[ci] as usize, m.c_start[ci + 1] as usize);
        let mut t = m.c_lit[a + rng.below(b - a)];
        for _ in 0..WALK_DEPTH {
            *units += 1;
            let v = lvar(t);
            match m.kind[v] {
                K_CHOICE => {
                    let g = m.var_group[v] as usize;
                    let a = m.g_start[g] as usize;
                    *units += m.gsize(g) as u64;
                    let j = (0..m.gsize(g)).find(|&j| lvar(m.g_lit[a + j]) == v).unwrap();
                    let want = if m.g_lit[a + j] == t { Some(j as u32) } else { None };
                    return Some((Src::Group(g as u32), want));
                }
                K_FREE => return Some((Src::Free(m.var_free[v]), None)),
                K_OR => {
                    let n = m.var_node[v] as usize;
                    let (a, b) = (m.n_start[n] as usize, m.n_start[n + 1] as usize);
                    let want_or = (1 ^ lneg(t)) ^ m.n_neg[n];
                    if b == a {
                        return None;
                    }
                    if want_or == 1 {
                        t = m.n_in[a + rng.below(b - a)]; // all inputs are false: make one true
                    } else {
                        // make a true input false
                        *units += (b - a) as u64;
                        let mut pick = None;
                        let mut seen = 0;
                        for &c in &m.n_in[a..b] {
                            if self.lit_true(c) {
                                seen += 1;
                                if rng.below(seen) == 0 {
                                    pick = Some(c);
                                }
                            }
                        }
                        t = pick? ^ 1;
                    }
                }
                K_HELPER => {
                    let n = m.var_node[v] as usize;
                    let (a, b) = (m.n_start[n] as usize, m.n_start[n + 1] as usize);
                    if b == a {
                        return None; // no clause needs it: it stays at its best value
                    }
                    let want_y = (1 ^ lneg(t)) ^ m.n_neg[n];
                    if want_y == 1 {
                        // every helper clause has a true literal: falsify one in a random clause
                        let h = m.n_in[a + rng.below(b - a)] as usize;
                        let mut pick = None;
                        let mut seen = 0;
                        *units += (m.hc_start[h + 1] - m.hc_start[h]) as u64;
                        for &c in &m.hc_lit[m.hc_start[h] as usize..m.hc_start[h + 1] as usize] {
                            if self.lit_true(c) {
                                seen += 1;
                                if rng.below(seen) == 0 {
                                    pick = Some(c);
                                }
                            }
                        }
                        t = pick? ^ 1;
                    } else {
                        // some helper clauses have every literal false: make one of them true
                        let mut pick = None;
                        let mut seen = 0;
                        *units += (b - a) as u64;
                        for &h in &m.n_in[a..b] {
                            if self.hc_cnt[h as usize] == 0 {
                                seen += 1;
                                if rng.below(seen) == 0 {
                                    pick = Some(h as usize);
                                }
                            }
                        }
                        let h = pick?;
                        let (ha, hb) = (m.hc_start[h] as usize, m.hc_start[h + 1] as usize);
                        if hb == ha {
                            return None;
                        }
                        t = m.hc_lit[ha + rng.below(hb - ha)];
                    }
                }
                K_GATE => {
                    let n = m.var_node[v] as usize;
                    let (a, b) = (m.n_start[n] as usize, m.n_start[n + 1] as usize);
                    *units += (b - a) as u64;
                    let mut idx = 0usize;
                    for (k, &u) in m.n_in[a..b].iter().enumerate() {
                        idx |= (self.val[u as usize] as usize) << k;
                    }
                    let out = (m.n_tt[n] >> idx) & 1;
                    let mut pick = None;
                    let mut seen = 0;
                    for k in 0..b - a {
                        if (m.n_tt[n] >> (idx ^ (1 << k))) & 1 != out {
                            seen += 1;
                            if rng.below(seen) == 0 {
                                pick = Some(k);
                            }
                        }
                    }
                    let k = match pick {
                        Some(k) => k,
                        None => rng.below(b - a),
                    };
                    let u = m.n_in[a + k];
                    // the literal over u that is false now: making it true flips u
                    t = 2 * u + self.val[u as usize] as u32;
                }
                _ => return None,
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// the search
// ---------------------------------------------------------------------------

/// The walk's settings (the prototype's options).
#[derive(Clone, Debug)]
pub struct Params {
    pub seed: u64,
    /// Steps at most (u64::MAX for no limit).
    pub max_steps: u64,
    /// Share of the steps that take a random move.
    pub noise: f64,
    /// Walks back from the false clause a step, each giving a candidate.
    pub cands: usize,
    /// Try only the value a walk back asked for (else every other value).
    pub target_only: bool,
    /// Score a move by its break alone (else by its weighted change).
    pub score_break: bool,
    /// Tabu on going back to a value just left, for this many steps (and up
    /// to `tenure_rand` more at random).
    pub tenure: u32,
    pub tenure_rand: u32,
    /// Clause weights: 0 none, 1 the step's clause, 2 every false clause.
    pub wmode: u8,
    /// Every `smooth` weight bumps each heavy clause gets one lighter (0 never).
    pub smooth: u64,
    /// Bump when no move lowers the weighted cost (else when the move taken does not).
    pub bump_best: bool,
    /// Weighted and plain phases of this many units each (0: weighted only).
    pub phase: u64,
    /// The check mode: recompute and compare the state every `check` steps
    /// and every trial's break (0 off; tests only).
    pub check: u64,
}

impl Params {
    /// The prototype's settings of the run of record: 16 candidates, 5%
    /// random moves, every false clause weighted, smoothing every third bump,
    /// phases of `phase` units.
    pub fn prototype(seed: u64, phase: u64) -> Params {
        Params {
            seed,
            max_steps: u64::MAX,
            noise: 0.05,
            cands: 16,
            target_only: false,
            score_break: false,
            tenure: 0,
            tenure_rand: 0,
            wmode: 2,
            smooth: 3,
            bump_best: false,
            phase,
            check: 0,
        }
    }

    pub fn from_options(o: &crate::options::Options) -> Params {
        let mut p = Params::prototype(o.seed as u64, (o.freewalkphase as u64).saturating_mul(1000).saturating_mul(UNITS_PER_TICK));
        if o.freewalksteps > 0 {
            p.max_steps = o.freewalksteps as u64;
        }
        p.noise = o.freewalknoise as f64 / 1000.0;
        p.cands = (o.freewalkcands as usize).max(1);
        p.wmode = o.freewalkweights as u8;
        p.smooth = o.freewalksmooth as u64;
        p
    }
}

/// Why a walk stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// Every constraint clause is true.
    Found,
    /// The work budget is reached.
    Budget,
    /// The step limit is reached.
    Steps,
    /// The run was told to stop (a signal, a time limit).
    Signal,
}

/// A walk: the reading, the state and the search's own memory, kept between
/// turns so that it resumes where it stopped.
pub struct Walk {
    m: Model,
    s: State,
    rng: Rng,
    p: Params,
    g_tabu_start: Vec<usize>,
    g_tabu: Vec<u64>,
    f_tabu: Vec<u64>,
    /// Per group value (from `g_tabu_start`) and per free bit (after the
    /// group values): the step that last tried it, so a step tries each once.
    tried: Vec<u64>,
    best: usize,
    best_step: u64,
    steps: u64,
    trials: u64,
    bumps: u64,
    cands: Vec<(Src, Option<u32>)>,
    heavy: Vec<u32>, // clauses of weight above 1
    heavy_pos: Vec<u32>,
    weights_on: bool,
    moves: Vec<Cand>,
    start_false: usize,
    /// Ticks charged to the work clock so far: `s.units / UNITS_PER_TICK`.
    charged: u64,
}

/// What the reading looks like to the file test.
#[derive(Debug, Clone, Copy, Default)]
pub struct Shape {
    pub groups: u64,
    pub overlap_groups: u64,
    pub free: u64,
    pub defs: u64,
    pub gates: u64,
    pub helpers: u64,
}

impl Shape {
    pub fn of(r: &Reading) -> Shape {
        let mut s = Shape { groups: r.groups.len() as u64, overlap_groups: r.stats.overlap_groups, ..Shape::default() };
        for &v in &r.order {
            match r.role[v as usize] {
                FREE => s.free += 1,
                DEF => s.defs += 1,
                GATE => s.gates += 1,
                HELPER => s.helpers += 1,
                _ => {}
            }
        }
        s
    }

    /// The file tests of G.8.3 (solver README "Free choices: the prototype
    /// search", the decision): 1 strict (groups, none left out for
    /// overlapping, no free variable, a definition or gate), 2 loose (groups
    /// and a definition or gate), 3 broad (groups or a definition or gate).
    pub fn admits(&self, test: i32) -> bool {
        let groups = self.groups > 0;
        let defined = self.defs > 0 || self.gates > 0;
        match test {
            1 => groups && self.overlap_groups == 0 && self.free == 0 && defined,
            2 => groups && defined,
            _ => groups || defined,
        }
    }
}

impl Walk {
    /// Build the walk from the reading within `budget` (the tick count to
    /// stop at), every table paid for before it is made: the walk's tables,
    /// its state and the first assignment from the seed.
    pub fn build(r: &Reading, p: Params, budget: u64, ticks: &mut u64, stop: &AtomicBool) -> Result<Walk, NoWalk> {
        let mut ck = Clock::new(ticks, budget, stop);
        let m = Model::build(r, &mut ck)?;
        let (nv, nn, nc, nh) = (m.nv as u64, m.nnodes() as u64, m.ncons() as u64, m.hc_node.len() as u64);
        let mut tot = 0usize;
        for g in 0..m.ngroups() {
            tot += m.gvalues(g);
        }
        let nf = m.free.len();
        // the state's tables, the search's tables per group value, free bit
        // and constraint, then the first assignment computed from the sources
        ck.pay(2 * (nv + 1) + 7 * nn + nh + 7 * nc + m.max_level as u64 + 1).ok_or(NoWalk::Budget)?;
        ck.pay(3 * (tot as u64 + nf as u64) + m.ngroups() as u64 + 2).ok_or(NoWalk::Budget)?;
        ck.pay(m.g_lit.len() as u64 + m.n_in.len() as u64 + m.hc_lit.len() as u64 + m.c_lit.len() as u64 + nn + nc).ok_or(NoWalk::Budget)?;
        let mut rng = Rng::new(p.seed);
        let s = State::new(&m, &mut rng);
        let mut g_tabu_start = Vec::with_capacity(m.ngroups() + 1);
        let mut t = 0usize;
        for g in 0..m.ngroups() {
            g_tabu_start.push(t);
            t += m.gvalues(g);
        }
        g_tabu_start.push(t);
        let start_false = s.fl.len();
        Ok(Walk {
            g_tabu: vec![0u64; tot],
            f_tabu: vec![0u64; nf],
            tried: vec![0u64; tot + nf],
            g_tabu_start,
            best: start_false,
            best_step: 0,
            steps: 0,
            trials: 0,
            bumps: 0,
            cands: Vec::with_capacity(p.cands),
            heavy: Vec::new(),
            heavy_pos: vec![u32::MAX; m.ncons()],
            weights_on: p.wmode != 0,
            moves: Vec::new(),
            start_false,
            charged: 0,
            m,
            s,
            rng,
            p,
        })
    }

    /// Steps until every constraint clause is true, the step limit, the
    /// budget (the tick count to stop at) or the stop flag. A step's work is
    /// counted into `ticks` as it is done.
    pub fn run(&mut self, budget: u64, ticks: &mut u64, stop: &AtomicBool) -> Stop {
        loop {
            if self.s.fl.is_empty() {
                return Stop::Found;
            }
            if self.steps >= self.p.max_steps {
                return Stop::Steps;
            }
            if *ticks >= budget {
                return Stop::Budget;
            }
            if stop.load(Ordering::Relaxed) {
                return Stop::Signal;
            }
            if self.steps & 255 == 0 && self.p.phase > 0 && self.p.wmode != 0 {
                // alternate: weighted for one phase, plain for the next (weights back to 1)
                let on = (self.s.units / self.p.phase) % 2 == 0;
                if on != self.weights_on {
                    self.weights_on = on;
                    if !on {
                        self.s.units += self.heavy.len() as u64;
                        for &c in &self.heavy {
                            self.s.w[c as usize] = 1;
                            self.heavy_pos[c as usize] = u32::MAX;
                        }
                        self.heavy.clear();
                        self.s.cost = self.s.fl.len() as u64;
                    }
                }
            }
            self.steps += 1;
            self.step();
            let due = self.s.units / UNITS_PER_TICK;
            if due > self.charged {
                *ticks = ticks.saturating_add(due - self.charged);
                self.charged = due;
            }
        }
    }

    /// One step of the prototype's search.
    fn step(&mut self) {
        let Walk { m, s, rng, p, g_tabu_start, g_tabu, f_tabu, tried, best, best_step, steps, trials, bumps, cands, heavy, heavy_pos, weights_on, moves, .. } = self;
        let steps = *steps;
        s.units += 1;
        let ci = s.fl[rng.below(s.fl.len())] as usize;
        cands.clear();
        let mut hops = 0u64;
        for _ in 0..p.cands {
            if let Some(src) = s.pick_source(m, ci, rng, &mut hops) {
                if !cands.contains(&src) {
                    cands.push(src);
                }
            }
        }
        s.units += hops + p.cands as u64;
        if cands.is_empty() {
            return;
        }
        moves.clear();
        let free_base = g_tabu_start[m.ngroups()];
        for &(src, want) in cands.iter() {
            match src {
                Src::Group(g) => {
                    let cur = s.g_cur[g as usize];
                    let base = g_tabu_start[g as usize];
                    for b in 0..m.gvalues(g as usize) as u32 {
                        s.units += 1;
                        if b == cur || (p.target_only && want.is_some_and(|j| j != b)) {
                            continue;
                        }
                        if tried[base + b as usize] == steps {
                            continue;
                        }
                        tried[base + b as usize] = steps;
                        let (c0, n0) = (s.cost as i64, s.fl.len() as i64);
                        let before = if p.check > 0 { Some(s.true_now()) } else { None };
                        let old = s.apply(m, src, b);
                        let (dw, dn, br) = (s.cost as i64 - c0, s.fl.len() as i64 - n0, s.last_break());
                        if let Some(bf) = &before {
                            if let Err(e) = s.check_break(bf, br) {
                                panic!("free-choice walk, check mode, step {steps}: {e}");
                            }
                        }
                        s.undo(m, src, old);
                        *trials += 1;
                        moves.push(Cand { src, value: b, dw, dn, br });
                    }
                }
                Src::Free(f) => {
                    s.units += 1;
                    if tried[free_base + f as usize] == steps {
                        continue;
                    }
                    tried[free_base + f as usize] = steps;
                    let (c0, n0) = (s.cost as i64, s.fl.len() as i64);
                    let before = if p.check > 0 { Some(s.true_now()) } else { None };
                    let old = s.apply(m, src, 0);
                    let (dw, dn, br) = (s.cost as i64 - c0, s.fl.len() as i64 - n0, s.last_break());
                    if let Some(bf) = &before {
                        if let Err(e) = s.check_break(bf, br) {
                            panic!("free-choice walk, check mode, step {steps}: {e}");
                        }
                    }
                    s.undo(m, src, old);
                    *trials += 1;
                    moves.push(Cand { src, value: 0, dw, dn, br });
                }
            }
        }
        if moves.is_empty() {
            return;
        }
        s.units += moves.len() as u64;
        let tabu = |c: &Cand| -> bool {
            match c.src {
                Src::Group(g) => g_tabu[g_tabu_start[g as usize] + c.value as usize] > steps,
                Src::Free(f) => f_tabu[f as usize] > steps,
            }
        };
        let chosen = if rng.unit() < p.noise {
            moves[rng.below(moves.len())]
        } else {
            let nf = s.fl.len() as i64;
            let mut pick: Option<Cand> = None;
            let mut ties = 0;
            let key = |c: &Cand| if p.score_break { c.br } else { c.dw };
            for c in moves.iter() {
                let allowed = !tabu(c) || ((nf + c.dn) as usize) < *best;
                if !allowed {
                    continue;
                }
                match pick {
                    None => {
                        pick = Some(*c);
                        ties = 1;
                    }
                    Some(b) if key(c) < key(&b) => {
                        pick = Some(*c);
                        ties = 1;
                    }
                    Some(b) if key(c) == key(&b) => {
                        ties += 1;
                        if rng.below(ties) == 0 {
                            pick = Some(*c);
                        }
                    }
                    _ => {}
                }
            }
            match pick {
                Some(c) => c,
                None => moves[rng.below(moves.len())],
            }
        };
        let old = s.apply(m, chosen.src, chosen.value);
        s.trail.clear();
        let ten = p.tenure as u64 + if p.tenure_rand > 0 { rng.below(p.tenure_rand as usize + 1) as u64 } else { 0 };
        match chosen.src {
            Src::Group(g) => g_tabu[g_tabu_start[g as usize] + old as usize] = steps + ten,
            Src::Free(f) => f_tabu[f as usize] = steps + ten,
        }
        let no_gain = if p.bump_best { moves.iter().all(|c| c.dw >= 0) } else { chosen.dw >= 0 };
        if no_gain && *weights_on {
            // no gain: the clause the step started from (or every false
            // clause) gets heavier; every `smooth` such steps each heavy
            // clause gets lighter by one
            *bumps += 1;
            if p.wmode == 1 {
                heavier(s, heavy, heavy_pos, ci);
            } else {
                s.units += s.fl.len() as u64;
                for i in 0..s.fl.len() {
                    let c = s.fl[i] as usize;
                    heavier(s, heavy, heavy_pos, c);
                }
            }
            if p.smooth > 0 && *bumps % p.smooth == 0 {
                s.units += heavy.len() as u64;
                let mut i = heavy.len();
                while i > 0 {
                    i -= 1;
                    let c = heavy[i] as usize;
                    s.w[c] -= 1;
                    if s.con_cnt[c] == 0 {
                        s.cost -= 1;
                    }
                    if s.w[c] == 1 {
                        let last = heavy.pop().unwrap();
                        if i < heavy.len() {
                            heavy[i] = last;
                            heavy_pos[last as usize] = i as u32;
                        }
                        heavy_pos[c] = u32::MAX;
                    }
                }
            }
        }
        if s.fl.len() < *best {
            *best = s.fl.len();
            *best_step = steps;
        }
        if p.check > 0 && steps % p.check == 0 {
            if let Err(e) = s.check(m) {
                panic!("free-choice walk, check mode, step {steps}: {e}");
            }
        }
    }

    /// The assignment of every variable of the reading (index 0 unused): the
    /// units, the choices and every computed variable; a variable absent from
    /// the formula after unit propagation is false.
    pub fn assignment(&self) -> Vec<bool> {
        (0..=self.m.nv)
            .map(|v| match self.m.kind[v] {
                K_FIXED => self.m.fixed[v] == 1,
                K_ABSENT => false,
                _ => self.s.val[v] == 1,
            })
            .collect()
    }

    /// The true member of each group (its size for none) and the value of each
    /// free variable, as the reading's own evaluation takes them.
    pub fn choices(&self) -> (Vec<usize>, Vec<(usize, bool)>) {
        let g = self.s.g_cur.iter().map(|&x| x as usize).collect();
        let f = self.m.free.iter().map(|&v| (v as usize, self.s.val[v as usize] == 1)).collect();
        (g, f)
    }

    /// The state against a recomputation from the sources.
    pub fn check_state(&self) -> Result<(), String> {
        self.s.check(&self.m)
    }

    /// Count `units` of work outside the steps (a model check) and charge
    /// what they make up to `ticks`.
    fn charge_extra(&mut self, units: u64, ticks: &mut u64) {
        self.s.units = self.s.units.saturating_add(units);
        let due = self.s.units / UNITS_PER_TICK;
        if due > self.charged {
            *ticks = ticks.saturating_add(due - self.charged);
            self.charged = due;
        }
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }
    pub fn false_now(&self) -> usize {
        self.s.fl.len()
    }
    pub fn work(&self) -> u64 {
        self.charged
    }

    /// FNV-1a over the assignment's bits, variables 1 to nv: two walks in the
    /// same place have the same hash (tools/free_walk_check.py). It allocates
    /// nothing, since the exit report also runs in the signal handler.
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for v in 1..=self.m.nv {
            let b = match self.m.kind[v] {
                K_FIXED => self.m.fixed[v] == 1,
                K_ABSENT => false,
                _ => self.s.val[v] == 1,
            };
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// The counters of the report line.
    fn counters(&self) -> String {
        format!(
            "vars={} steps={} trials={} flips={} updates={} pushes={} best={} best_step={} start_false={} false={} units={} work={} groups={} free={} nodes={} constraints={}",
            self.m.nv,
            self.steps,
            self.trials,
            self.s.flips,
            self.s.upd,
            self.s.pushes,
            self.best,
            self.best_step,
            self.start_false,
            self.s.fl.len(),
            self.s.units,
            self.charged,
            self.m.ngroups(),
            self.m.free.len(),
            self.m.nnodes(),
            self.m.ncons()
        )
    }
}

fn heavier(s: &mut State, heavy: &mut Vec<u32>, heavy_pos: &mut [u32], c: usize) {
    if s.w[c] == 1 {
        heavy_pos[c] = heavy.len() as u32;
        heavy.push(c as u32);
    }
    s.w[c] += 1;
    if s.con_cnt[c] == 0 {
        s.cost += 1;
    }
}

/// Does the assignment (indexed by variable) satisfy every clause of `lits`,
/// the input as parsed, each clause ended by 0? A variable past the end of
/// the assignment counts as false.
pub fn satisfies(model: &[bool], lits: &[i32]) -> bool {
    let mut sat = false;
    for &l in lits {
        if l == 0 {
            if !sat {
                return false;
            }
            sat = false;
        } else if !sat {
            let v = l.unsigned_abs() as usize;
            if (v < model.len() && model[v]) == (l > 0) {
                sat = true;
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// in the solver
// ---------------------------------------------------------------------------

/// The walk's place in the solver (`solver.freewalk`).
#[derive(Default)]
pub struct FreeWalk {
    /// The walk, once built.
    pub walk: Option<Box<Walk>>,
    /// The input clauses as parsed, each ended by 0: a model is answered
    /// only once it satisfies every one.
    pub original: Vec<i32>,
    /// A model the walk found that satisfies every input clause.
    pub model: Option<Vec<bool>>,
    /// Turns with the search are on (placement 2, a walk built).
    pub turns: bool,
    /// The work clock at which the next turn is due.
    pub next_turn: u64,
    /// Why there is no walk (`unread`, `declined`, `budget`, `malformed`),
    /// or `built`.
    pub status: &'static str,
    /// The reading's status when it did not read (`overflow`, `budget`, ...).
    pub reading: &'static str,
    pub shape: Shape,
    pub build_work: u64,
    pub build_ms: u128,
    pub runs: u64,
    pub walk_ms: u128,
    /// Models that failed the check against the input: a bug, never answered;
    /// the walk is off for the rest of the run.
    pub refused: u64,
    /// The walk has stopped for good: its step limit, or a refused model.
    /// Turns end, so the search loop decides again.
    pub done: bool,
}

fn placement_name(p: i32) -> &'static str {
    match p {
        1 => "first",
        2 => "turns",
        3 => "rephase",
        _ => "off",
    }
}

fn test_name(t: i32) -> &'static str {
    match t {
        1 => "strict",
        2 => "loose",
        _ => "broad",
    }
}

/// No reading to walk on: why (the reading's status).
pub fn unread(solver: &mut Solver, why: &'static str) {
    solver.freewalk.status = "unread";
    solver.freewalk.reading = why;
}

/// In the structure pass, after the reading: the file test, then the walk
/// built within `budget` (what is left of the reading's own limit).
pub fn build(solver: &mut Solver, r: &Reading, budget: u64, stop: &AtomicBool) {
    // the file test looks at each settled variable once, paid for first (in
    // the build's units)
    let look = (r.order.len() as u64 + 1).div_ceil(BUILD_UNITS_PER_TICK);
    if solver.statistics.ticks.saturating_add(look) > budget {
        solver.freewalk.status = "budget";
        return;
    }
    solver.statistics.ticks += look;
    let shape = Shape::of(r);
    solver.freewalk.shape = shape;
    if !shape.admits(solver.options.freewalktest) {
        solver.freewalk.status = "declined";
        return;
    }
    let started = std::time::Instant::now();
    let before = solver.statistics.ticks;
    let p = Params::from_options(&solver.options);
    let built = Walk::build(r, p, budget, &mut solver.statistics.ticks, stop);
    solver.freewalk.build_work = solver.statistics.ticks - before;
    solver.freewalk.build_ms = started.elapsed().as_millis();
    match built {
        Ok(w) => {
            solver.freewalk.walk = Some(Box::new(w));
            solver.freewalk.status = "built";
            if solver.options.freewalk == 2 {
                solver.freewalk.turns = true;
                schedule(solver);
            }
        }
        Err(NoWalk::Budget) => solver.freewalk.status = "budget",
        Err(NoWalk::Malformed) => solver.freewalk.status = "malformed",
    }
}

/// Walk until the budget (a tick count to stop at); true when the walk found
/// a model that satisfies every clause of `original`, which is then kept.
fn walk_until(solver: &mut Solver, budget: u64, original: &[i32]) -> bool {
    if solver.freewalk.done || solver.freewalk.model.is_some() {
        return solver.freewalk.model.is_some();
    }
    let Some(w) = solver.freewalk.walk.as_mut() else { return false };
    let started = std::time::Instant::now();
    let stop = w.run(budget, &mut solver.statistics.ticks, &solver.termination.flagged);
    solver.freewalk.runs += 1;
    let mut found = false;
    if stop == Stop::Found {
        let model = w.assignment();
        // the check against every input clause, counted as the walk's work
        w.charge_extra(original.len() as u64, &mut solver.statistics.ticks);
        if satisfies(&model, original) {
            solver.freewalk.model = Some(model);
            found = true;
        } else {
            solver.freewalk.refused += 1;
            solver.freewalk.done = true;
            solver.freewalk.turns = false;
            crate::print::message(solver, "free-choice walk: a model failed the check against the input and is not answered; the walk is off".to_string());
        }
    } else if stop == Stop::Steps {
        // no step is left: a turn would come due at every decision
        solver.freewalk.done = true;
        solver.freewalk.turns = false;
    }
    solver.freewalk.walk_ms += started.elapsed().as_millis();
    found
}

/// Placement 1: the one pass before the search, within `budget`; `original`
/// is the input as parsed (still in the structure pass's buffer).
pub fn first(solver: &mut Solver, budget: u64, original: &[i32]) -> bool {
    walk_until(solver, budget, original)
}

/// The walk's work so far, in ticks.
fn walk_work(solver: &Solver) -> u64 {
    solver.freewalk.walk.as_ref().map_or(0, |w| w.charged)
}

/// The share of the work clock the walk is owed now: the ticks d with
/// w + d = share x (W + d), W the work clock and w the walk's work.
fn owed(solver: &Solver) -> u64 {
    let share = solver.options.freewalkshare.clamp(1, 990) as u128;
    let total = solver.statistics.work_clock() as u128;
    let w = walk_work(solver) as u128;
    let num = (share * total).saturating_sub(1000 * w);
    (num / (1000 - share)).min(u64::MAX as u128) as u64
}

/// The next turn: when the walk is owed one turn's length q, that is when
/// share x W - 1000 w reaches q x (1000 - share).
fn schedule(solver: &mut Solver) {
    let share = solver.options.freewalkshare.clamp(1, 990) as u128;
    let q = (solver.options.freewalkturn.max(1) as u128) * 1000;
    let w = walk_work(solver) as u128;
    let need = 1000 * w + q * (1000 - share);
    solver.freewalk.next_turn = need.div_ceil(share).min(u64::MAX as u128) as u64;
}

/// The budget for `d` more ticks, within the tick limit.
fn budget_for(solver: &Solver, d: u64) -> u64 {
    let mut d = d;
    if solver.limited.ticks {
        d = d.min(solver.limits.ticks.saturating_sub(solver.statistics.work_clock()));
    }
    solver.statistics.ticks.saturating_add(d)
}

/// Placement 2: is a turn due? (One bool when off.)
#[inline]
pub fn turn_due(solver: &Solver) -> bool {
    solver.freewalk.turns && solver.statistics.work_clock() >= solver.freewalk.next_turn
}

/// Placement 2: the walk's turn, until it has its share again; true when it
/// found a model, checked against the input.
pub fn turn(solver: &mut Solver) -> bool {
    let budget = budget_for(solver, owed(solver));
    let original = std::mem::take(&mut solver.freewalk.original);
    let found = walk_until(solver, budget, &original);
    solver.freewalk.original = original;
    schedule(solver);
    found
}

/// Placement 3: in a walking rephase slot, the walk until it has its share;
/// true when it found a model, checked against the input.
pub fn rephase_slot(solver: &mut Solver) -> bool {
    if solver.options.freewalk != 3 || solver.freewalk.walk.is_none() || solver.freewalk.done {
        return false;
    }
    let d = owed(solver);
    if d == 0 {
        return false;
    }
    let budget = budget_for(solver, d);
    let original = std::mem::take(&mut solver.freewalk.original);
    let found = walk_until(solver, budget, &original);
    solver.freewalk.original = original;
    found
}

/// Has the walk found a model (checked against the input)?
#[inline]
pub fn found(solver: &Solver) -> bool {
    solver.freewalk.model.is_some()
}

/// The `c freewalk` exit line (nothing with the walk off).
pub fn report(solver: &Solver) {
    if solver.options.freewalk == 0 {
        return;
    }
    let fw = &solver.freewalk;
    let status = if fw.model.is_some() {
        "found"
    } else if fw.refused > 0 {
        "refused"
    } else if fw.walk.is_some() {
        "walked"
    } else if fw.status.is_empty() {
        "unread"
    } else {
        fw.status
    };
    let mut line = format!(
        "{}freewalk placement={} test={} status={}",
        solver.prefix,
        placement_name(solver.options.freewalk),
        test_name(solver.options.freewalktest),
        status
    );
    if status == "unread" {
        line.push_str(&format!(" reading={}", if fw.reading.is_empty() { "off" } else { fw.reading }));
    } else {
        let s = &fw.shape;
        line.push_str(&format!(
            " read_groups={} overlap_groups={} read_free={} defs={} gates={} helpers={}",
            s.groups, s.overlap_groups, s.free, s.defs, s.gates, s.helpers
        ));
    }
    if let Some(w) = &fw.walk {
        line.push_str(&format!(
            " {} runs={} walk_ms={} build_work={} build_ms={} refused={} hash={:016x}",
            w.counters(),
            fw.runs,
            fw.walk_ms,
            fw.build_work,
            fw.build_ms,
            fw.refused,
            w.hash()
        ));
    }
    println!("{}", line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::freeread::NotRead;
    use crate::structure::Formula;

    /// The parser's literals from DIMACS text (header and comments skipped),
    /// each clause ended by 0, and the highest variable.
    fn literals(text: &str) -> (Vec<i32>, i32) {
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
        (lits, max_var)
    }

    fn formula(lits: &[i32], max_var: i32) -> Formula {
        crate::structure::build_formula(lits, max_var)
    }

    fn read_all(f: &Formula) -> Result<Reading, NotRead> {
        let stop = AtomicBool::new(false);
        let mut ticks = 0u64;
        crate::freeread::read(f, u64::MAX, &mut ticks, &stop)
    }

    fn build_all(r: &Reading, p: Params) -> Walk {
        let stop = AtomicBool::new(false);
        let mut ticks = 0u64;
        Walk::build(r, p, u64::MAX, &mut ticks, &stop).expect("the walk builds")
    }

    /// The toys of tools/free_walk.py selftest (`free_walk.py fixtures`).
    const TOYS: &str = include_str!("../tests/fixtures/freewalk_toys.txt");

    struct Toy {
        name: String,
        sat: bool,
        cnf: String,
    }

    fn toys() -> Vec<Toy> {
        let mut out = Vec::new();
        let mut cur: Option<Toy> = None;
        for line in TOYS.lines() {
            if let Some(name) = line.strip_prefix("toy ") {
                cur = Some(Toy { name: name.to_string(), sat: true, cnf: String::new() });
            } else if let Some(rest) = line.strip_prefix("sat ") {
                cur.as_mut().expect("a toy").sat = rest.trim() == "1";
            } else if line == "end" {
                out.push(cur.take().expect("a toy"));
            } else if let Some(t) = cur.as_mut() {
                t.cnf.push_str(line);
                t.cnf.push('\n');
            }
        }
        out
    }

    fn toy(name: &str) -> Toy {
        toys().into_iter().find(|t| t.name == name).expect("the toy")
    }

    /// The walk on a toy: (its literals, the reading, the walk).
    fn walk_on(t: &Toy, p: Params) -> (Vec<i32>, Reading, Walk) {
        let (lits, mv) = literals(&t.cnf);
        let f = formula(&lits, mv);
        let r = read_all(&f).unwrap_or_else(|e| panic!("{}: the toy is read ({e:?})", t.name));
        let w = build_all(&r, p);
        (lits, r, w)
    }

    fn run_all(w: &mut Walk, max_steps: u64) -> Stop {
        w.p.max_steps = max_steps;
        let stop = AtomicBool::new(false);
        let mut ticks = 0u64;
        w.run(u64::MAX, &mut ticks, &stop)
    }

    /// Does the walk's assignment equal the reading's own evaluation from the
    /// same choices (Reading::assignment, the census's full_assignment)?
    fn agrees(r: &Reading, w: &Walk) -> Result<(), String> {
        let (g, f) = w.choices();
        let free: std::collections::HashMap<usize, bool> = f.into_iter().collect();
        let want = r.assignment(&g, &|v| free.get(&v).copied().unwrap_or(false));
        let got = w.assignment();
        for v in 1..want.len() {
            if want[v] != got[v] {
                return Err(format!("variable {v} (role {}): walk {}, reading {}", r.role[v], got[v], want[v]));
            }
        }
        Ok(())
    }

    const SELFTEST: &[&str] = &["tseitin", "tseitin-one-way", "csp-pairwise", "csp-ladder", "lockchart", "lockchart-big", "guarded", "cycle", "random3-easy"];

    #[test]
    fn the_walk_computes_every_variable_as_the_reading_does() {
        for name in SELFTEST {
            let t = toy(name);
            for seed in 0..5 {
                let (_lits, r, w) = walk_on(&t, Params::prototype(seed, 0));
                w.check_state().unwrap_or_else(|e| panic!("{name} seed {seed}: {e}"));
                agrees(&r, &w).unwrap_or_else(|e| panic!("{name} seed {seed}: {e}"));
            }
        }
    }

    #[test]
    fn a_model_is_found_on_every_satisfiable_toy_with_the_state_checked_every_step() {
        for name in SELFTEST {
            let t = toy(name);
            assert!(t.sat);
            let mut p = Params::prototype(1, 0);
            p.check = 1;
            let (lits, r, mut w) = walk_on(&t, p);
            let stop = run_all(&mut w, 2_000_000);
            assert_eq!(stop, Stop::Found, "{name}: a model within 2M steps");
            w.check_state().unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(satisfies(&w.assignment(), &lits), "{name}: the model satisfies every input clause");
            agrees(&r, &w).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn weighted_walks_keep_the_weighted_cost_exact_and_find_a_model() {
        let t = toy("lockchart-big");
        for (wmode, smooth) in [(1u8, 5u64), (2, 3)] {
            let mut p = Params::prototype(2, 0);
            p.check = 1;
            p.noise = 0.2;
            p.wmode = wmode;
            p.smooth = smooth;
            let (lits, _r, mut w) = walk_on(&t, p);
            assert_eq!(run_all(&mut w, 2_000_000), Stop::Found, "weights {wmode} smooth {smooth}");
            assert!(satisfies(&w.assignment(), &lits));
        }
    }

    #[test]
    fn a_moves_break_counts_only_clauses_true_before_and_false_after() {
        let t = toy("break");
        let (lits, mv) = literals(&t.cnf);
        let r = read_all(&formula(&lits, mv)).expect("read");
        // the toy has constraint clauses over two members of one group
        let mut same = 0;
        for c in 0..r.cnf.m() {
            let cl = r.cnf.clause(c);
            if r.cover[c] == C_NONE && cl.len() == 2 && cl.iter().all(|&z| z > 0) && r.gid[cl[0] as usize] >= 0 && r.gid[cl[0] as usize] == r.gid[cl[1] as usize] {
                same += 1;
            }
        }
        assert_eq!(same, 6);
        let mut p = Params::prototype(3, 0);
        p.check = 1;
        p.score_break = true;
        p.wmode = 0;
        let mut w = build_all(&r, p);
        assert_eq!(run_all(&mut w, 2_000_000), Stop::Found);
        assert!(satisfies(&w.assignment(), &lits));
    }

    #[test]
    fn an_unsatisfiable_toy_ends_at_the_step_limit_with_no_model() {
        let t = toy("unsat");
        assert!(!t.sat);
        let (_lits, _r, mut w) = walk_on(&t, Params::prototype(1, 0));
        assert_eq!(run_all(&mut w, 20_000), Stop::Steps);
        assert!(w.false_now() > 0);
    }

    #[test]
    fn a_model_that_fails_the_input_is_refused() {
        let t = toy("lockchart");
        let (lits, _r, mut w) = walk_on(&t, Params::prototype(1, 0));
        assert_eq!(run_all(&mut w, 2_000_000), Stop::Found);
        let model = w.assignment();
        assert!(satisfies(&model, &lits));
        // the input with one more clause, the negation of a choice the model makes
        let v = (1..model.len()).find(|&v| model[v]).expect("a true variable");
        let mut planted = lits.clone();
        planted.extend_from_slice(&[-(v as i32), 0]);
        assert!(!satisfies(&model, &planted), "the planted clause is false under the model");
        // and in the solver: the model is refused, not answered, and the walk is off
        // (the search has done a billion ticks, so a turn has room to find it)
        let mut solver = crate::internal::init();
        solver.options.freewalk = 2;
        solver.statistics.ticks = 1_000_000_000;
        solver.freewalk.walk = Some(Box::new(walk_on(&t, Params::prototype(1, 0)).2));
        solver.freewalk.turns = true;
        solver.freewalk.original = planted;
        assert!(!turn(&mut solver), "a model that fails the check is not answered");
        assert!(!found(&solver));
        assert_eq!(solver.freewalk.refused, 1);
        assert!(!solver.freewalk.turns, "the walk is off once a model was refused");
        assert!(!turn(&mut solver) && !found(&solver), "and stays off");
        // with the input as it is, the same walk is answered
        let mut solver = crate::internal::init();
        solver.options.freewalk = 2;
        solver.statistics.ticks = 1_000_000_000;
        solver.freewalk.walk = Some(Box::new(walk_on(&t, Params::prototype(1, 0)).2));
        solver.freewalk.turns = true;
        solver.freewalk.original = lits;
        assert!(turn(&mut solver));
        assert!(found(&solver));
    }

    #[test]
    fn a_budget_stops_the_walk_and_the_walk_resumes_where_it_stopped() {
        let t = toy("lockchart-big");
        let p = Params::prototype(4, 5_000);
        let (_lits, _r, mut one) = walk_on(&t, p.clone());
        let (_lits, _r, mut many) = walk_on(&t, p);
        let stop = AtomicBool::new(false);
        // one walk of 3,000 steps against walks of a few ticks each
        let (mut t1, mut t2) = (0u64, 0u64);
        one.p.max_steps = 3_000;
        let s1 = one.run(u64::MAX, &mut t1, &stop);
        many.p.max_steps = 3_000;
        let mut turns = 0;
        let s2 = loop {
            let budget = t2 + 50;
            let s = many.run(budget, &mut t2, &stop);
            turns += 1;
            if s != Stop::Budget {
                break s;
            }
            // at most one step past the budget
            assert!(t2 < budget + 50_000, "a walk stops within a step of its budget");
        };
        assert_eq!(s1, s2);
        assert!(turns > 10, "the budget cut the walk into turns ({turns})");
        assert_eq!(one.steps(), many.steps());
        assert_eq!(one.hash(), many.hash(), "the same assignment");
        assert_eq!(t1, t2, "the same work");
        assert_eq!(one.work(), t1);
        // a budget already reached: no step
        let before = many.steps();
        let mut t3 = 10u64;
        assert_eq!(many.run(10, &mut t3, &stop), if many.false_now() == 0 { Stop::Found } else { Stop::Budget });
        assert_eq!(many.steps(), before);
    }

    #[test]
    fn the_stop_flag_ends_a_walk_before_its_next_step() {
        let t = toy("unsat");
        let (_lits, _r, mut w) = walk_on(&t, Params::prototype(1, 0));
        let stop = AtomicBool::new(true);
        let mut ticks = 0u64;
        assert_eq!(w.run(u64::MAX, &mut ticks, &stop), Stop::Signal);
        assert_eq!(w.steps(), 0);
    }

    #[test]
    fn building_stops_at_its_budget_and_charges_nothing_past_it() {
        let t = toy("lockchart-big");
        let (lits, mv) = literals(&t.cnf);
        let r = read_all(&formula(&lits, mv)).expect("read");
        let stop = AtomicBool::new(false);
        let mut full = 0u64;
        Walk::build(&r, Params::prototype(0, 0), u64::MAX, &mut full, &stop).expect("builds");
        assert!(full > 1000);
        for budget in [0, 10, full / 2, full - 1] {
            let mut ticks = 0u64;
            assert_eq!(Walk::build(&r, Params::prototype(0, 0), budget, &mut ticks, &stop).err(), Some(NoWalk::Budget));
            assert!(ticks <= budget);
        }
        let mut ticks = 0u64;
        assert!(Walk::build(&r, Params::prototype(0, 0), full, &mut ticks, &stop).is_ok());
        assert_eq!(ticks, full);
    }

    #[test]
    fn the_file_tests_admit_what_the_decision_named() {
        let lock = Shape { groups: 4, defs: 6, helpers: 6, ..Shape::default() };
        assert!(lock.admits(1) && lock.admits(2) && lock.admits(3));
        let free = Shape { groups: 4, defs: 6, free: 1, ..Shape::default() };
        assert!(!free.admits(1) && free.admits(2) && free.admits(3));
        let overlap = Shape { groups: 4, overlap_groups: 1, gates: 2, ..Shape::default() };
        assert!(!overlap.admits(1) && overlap.admits(2) && overlap.admits(3));
        let groups_only = Shape { groups: 4, free: 3, ..Shape::default() };
        assert!(!groups_only.admits(1) && !groups_only.admits(2) && groups_only.admits(3));
        let defs_only = Shape { defs: 4, free: 3, ..Shape::default() };
        assert!(!defs_only.admits(1) && !defs_only.admits(2) && defs_only.admits(3));
        let nothing = Shape { free: 30, helpers: 4, ..Shape::default() };
        assert!(!nothing.admits(1) && !nothing.admits(2) && !nothing.admits(3));
    }

    #[test]
    fn the_share_gives_the_walk_its_part_of_the_work() {
        let mut solver = crate::internal::init();
        solver.options.freewalk = 2;
        solver.options.freewalkshare = 250;
        solver.options.freewalkturn = 1; // 1,000 ticks
        let t = toy("unsat");
        solver.freewalk.walk = Some(Box::new(walk_on(&t, Params::prototype(1, 0)).2));
        solver.freewalk.turns = true;
        schedule(&mut solver);
        // the walk is owed nothing until the search has done 3,000 ticks
        assert_eq!(solver.freewalk.next_turn, 3_000);
        solver.statistics.ticks = 2_999;
        assert!(!turn_due(&solver));
        solver.statistics.ticks = 3_000;
        assert!(turn_due(&solver));
        assert_eq!(owed(&solver), 1_000);
        // the search goes on: at W = 30,000 the walk is owed 10,000
        solver.statistics.ticks = 30_000;
        assert_eq!(owed(&solver), 10_000);
        assert!(!turn(&mut solver));
        let w = walk_work(&solver);
        assert!(w >= 10_000 && w < 10_000 + 1_000, "the walk took its share ({w})");
        assert!((w as f64) / (solver.statistics.ticks as f64) >= 0.25);
        assert!(!turn_due(&solver));
        // a tick limit bounds a turn
        solver.limited.ticks = true;
        solver.limits.ticks = solver.statistics.work_clock() + 100;
        solver.statistics.ticks += 1_000_000;
        let before = solver.statistics.ticks;
        assert!(owed(&solver) > 100);
        let budget = budget_for(&solver, owed(&solver));
        assert_eq!(budget, before, "no room past the tick limit");
    }
}
