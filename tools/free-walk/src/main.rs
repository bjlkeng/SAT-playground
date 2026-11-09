//! free-walk: a local search over the free choices of a formula
//! (bead SAT-playground-d96.2.2, G.8.2; a standalone tool, no solver code).
//!
//! Input: the role reading that `tools/free_walk.py export` writes from
//! `tools/free_census.py`: the units, the groups (one value each: the member
//! that is true, or none in an at-most-one group), the free variables, every
//! other variable in the order it is computed (an OR of literals, a small
//! gate, or a one-way helper set to its best value), and the constraint
//! clauses that no role makes true.
//!
//! The search state is one value per group and one bit per free variable.
//! Every other variable is recomputed after each move, incrementally (counts
//! of true inputs, nodes taken level by level in computing order), so the
//! cost, the constraint clauses left false, is exact at every step. A step
//! takes a false constraint clause at random and walks back from one of its
//! literals through the definitions to a group or free bit that could make it
//! true; `--cands` such walks (16) give the candidates. It tries every other
//! value of each candidate and takes the one that lowers the weighted cost
//! most, ties at random, with a random move instead in `--noise` of the steps
//! (5%) and an optional tabu on going back to a value just left (`--tenure`,
//! off). Clause weighting (`--weights all`): when the move taken does not
//! lower the weighted cost, every false clause gets one heavier, and every
//! `--smooth` (3) such steps every heavy clause gets one lighter. With
//! `--phase S` (30 s) the walk alternates: S seconds weighted, S seconds plain
//! (weights back to 1). Plain search did best on the small lockchart cells and
//! weighting on the big ones (sweeps of 2026-11-08, solver README "Free
//! choices: the prototype search").
//!
//! Output: on success `s SATISFIABLE` and a model over every variable (the
//! units, the choices and every computed variable) for tools/verify_sat.py to
//! check against the original formula; `s UNKNOWN` at the time or step limit.

use std::io::{BufWriter, Read, Write};
use std::time::Instant;

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

const WALK_DEPTH: usize = 100_000;

fn fail(msg: &str) -> ! {
    eprintln!("free-walk: {msg}");
    std::process::exit(2);
}

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

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Rng {
        let mut r = Rng(seed ^ 0x5DEE_CE66_D1CE_4E5B);
        r.next();
        r
    }
    fn next(&mut self) -> u64 {
        // splitmix64
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

// ---------------------------------------------------------------------------
// the reading
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

struct Ints {
    d: Vec<u8>,
    p: usize,
}
impl Ints {
    fn next(&mut self) -> i32 {
        if self.p + 4 > self.d.len() {
            fail("the reading ends early");
        }
        let b = &self.d[self.p..self.p + 4];
        self.p += 4;
        i32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }
    fn count(&mut self) -> usize {
        let x = self.next();
        if x < 0 {
            fail("a negative count in the reading");
        }
        x as usize
    }
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

    fn load(path: &str) -> Model {
        let mut d = Vec::new();
        std::fs::File::open(path)
            .and_then(|mut f| f.read_to_end(&mut d))
            .unwrap_or_else(|e| fail(&format!("cannot read {path}: {e}")));
        if d.len() < 4 || &d[..4] != b"FWR1" {
            fail("not a free-walk reading (magic FWR1)");
        }
        let mut r = Ints { d, p: 4 };
        let nv = r.count();
        let var = |r: &mut Ints, what: &str| -> usize {
            let v = r.count();
            if v == 0 || v > nv {
                fail(&format!("{what}: variable {v} out of range"));
            }
            v
        };
        let lit = |r: &mut Ints, what: &str| -> u32 {
            let l = r.next();
            if l == 0 || l.unsigned_abs() as usize > nv {
                fail(&format!("{what}: literal {l} out of range"));
            }
            code(l)
        };
        let mut kind = vec![K_ABSENT; nv + 1];
        let mut fixed = vec![0u8; nv + 1];
        let nfixed = r.count();
        for _ in 0..nfixed {
            let v = var(&mut r, "unit");
            let b = r.next();
            if kind[v] != K_ABSENT || !(b == 0 || b == 1) {
                fail("a bad unit");
            }
            kind[v] = K_FIXED;
            fixed[v] = b as u8;
        }
        let ng = r.count();
        let mut g_start = vec![0u32];
        let mut g_lit = Vec::new();
        let mut g_none = Vec::with_capacity(ng);
        let mut var_group = vec![u32::MAX; nv + 1];
        for g in 0..ng {
            let none = r.next();
            let k = r.count();
            if k == 0 || !(none == 0 || none == 1) {
                fail("a bad group");
            }
            for _ in 0..k {
                let c = lit(&mut r, "group");
                let v = lvar(c);
                if kind[v] != K_ABSENT {
                    fail(&format!("variable {v} is in a group and has another role"));
                }
                kind[v] = K_CHOICE;
                var_group[v] = g as u32;
                g_lit.push(c);
            }
            g_start.push(g_lit.len() as u32);
            g_none.push(none as u8);
        }
        let nf = r.count();
        let mut free = Vec::with_capacity(nf);
        let mut var_free = vec![u32::MAX; nv + 1];
        for i in 0..nf {
            let v = var(&mut r, "free");
            if kind[v] != K_ABSENT {
                fail(&format!("free variable {v} has another role"));
            }
            kind[v] = K_FREE;
            var_free[v] = i as u32;
            free.push(v as u32);
        }
        let nn = r.count();
        let mut n_out = Vec::with_capacity(nn);
        let mut n_kind = Vec::with_capacity(nn);
        let mut n_neg = Vec::with_capacity(nn);
        let mut n_tt = Vec::with_capacity(nn);
        let mut n_start = vec![0u32];
        let mut n_in = Vec::new();
        let mut var_node = vec![u32::MAX; nv + 1];
        let mut hc_start = vec![0u32];
        let mut hc_lit = Vec::new();
        let mut hc_node = Vec::new();
        // an input must be settled before the node that reads it
        let settled = |kind: &Vec<u8>, v: usize| kind[v] != K_ABSENT;
        for n in 0..nn {
            let kd = r.next();
            let (out, neg) = match kd {
                4 => {
                    let o = lit(&mut r, "OR output");
                    let k = r.count();
                    for _ in 0..k {
                        let c = lit(&mut r, "OR input");
                        if !settled(&kind, lvar(c)) {
                            fail(&format!("an OR reads variable {} before it is settled", lvar(c)));
                        }
                        n_in.push(c);
                    }
                    (lvar(o), lneg(o))
                }
                5 => {
                    let v = var(&mut r, "gate output");
                    let k = r.count();
                    if k == 0 || k > 4 {
                        fail("a gate with a bad support");
                    }
                    for _ in 0..k {
                        let u = var(&mut r, "gate input");
                        if !settled(&kind, u) {
                            fail(&format!("a gate reads variable {u} before it is settled"));
                        }
                        n_in.push(u as u32);
                    }
                    let tt = r.next();
                    if tt < 0 || (tt as i64) >= (1i64 << (1u32 << k)) {
                        fail("a bad truth table");
                    }
                    n_tt.push(tt as u32);
                    (v, 0)
                }
                6 => {
                    let y = lit(&mut r, "helper");
                    let ncl = r.count();
                    if ncl > 255 {
                        fail("a helper in more than 255 clauses (the counters are bytes)");
                    }
                    for _ in 0..ncl {
                        let m = r.count();
                        if m > 255 {
                            fail("a helper clause with more than 255 other literals (the counters are bytes)");
                        }
                        for _ in 0..m {
                            let c = lit(&mut r, "helper clause");
                            if !settled(&kind, lvar(c)) {
                                fail(&format!("a helper reads variable {} before it is settled", lvar(c)));
                            }
                            hc_lit.push(c);
                        }
                        n_in.push(hc_node.len() as u32);
                        hc_node.push(n as u32);
                        hc_start.push(hc_lit.len() as u32);
                    }
                    (lvar(y), lneg(y))
                }
                _ => fail(&format!("unknown node kind {kd}")),
            };
            if kd != 5 {
                n_tt.push(0);
            }
            if kind[out] != K_ABSENT {
                fail(&format!("variable {out} is computed twice"));
            }
            kind[out] = match kd {
                4 => K_OR,
                5 => K_GATE,
                _ => K_HELPER,
            };
            var_node[out] = n as u32;
            n_out.push(out as u32);
            n_kind.push(kind[out]);
            n_neg.push(neg);
            n_start.push(n_in.len() as u32);
        }
        let nc = r.count();
        let mut c_start = vec![0u32];
        let mut c_lit = Vec::new();
        for _ in 0..nc {
            let k = r.count();
            if k == 0 {
                fail("an empty constraint clause");
            }
            for _ in 0..k {
                let c = lit(&mut r, "constraint");
                if !settled(&kind, lvar(c)) {
                    fail(&format!("a constraint reads variable {} that has no role", lvar(c)));
                }
                c_lit.push(c);
            }
            c_start.push(c_lit.len() as u32);
        }
        if r.p != r.d.len() {
            fail("trailing data after the reading");
        }
        let mut m = Model {
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
        m.build_readers();
        m.build_levels();
        m
    }

    fn build_readers(&mut self) {
        let nv = self.nv;
        let mut cnt = vec![0u32; nv + 2];
        let each = |f: &mut dyn FnMut(usize, u32, u32, u8)| {
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
        };
        each(&mut |v, _, _, _| cnt[v + 1] += 1);
        for v in 1..nv + 2 {
            cnt[v] += cnt[v - 1];
        }
        let total = cnt[nv + 1] as usize;
        let mut tgt = vec![0u32; total];
        let mut kn = vec![0u8; total];
        let mut aux = vec![0u32; total];
        let mut fill = cnt.clone();
        each(&mut |v, t, x, k| {
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
    }

    /// Levels: a node's inputs all sit on lower levels, so the queue can
    /// take the nodes level by level (a bucket per level, no heap).
    fn build_levels(&mut self) {
        let nn = self.n_out.len();
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
    }
}

// ---------------------------------------------------------------------------
// the search state
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
    // the constraint clauses a move touches, each with whether it was true
    // before the move: its break is the weight of those true before and
    // false after (a clause false only midway through the move is no break)
    con_mark: Vec<u32>,
    epoch: u32,
    touched: Vec<(u32, bool)>,
}

impl State {
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
            let k = m.c_lit[m.c_start[ci] as usize..m.c_start[ci + 1] as usize]
                .iter()
                .filter(|&&c| self.lit_true(c))
                .count() as u32;
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
                        let k = m.hc_lit[m.hc_start[h] as usize..m.hc_start[h + 1] as usize]
                            .iter()
                            .filter(|&&c| self.lit_true(c))
                            .count() as u32;
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
    fn last_break(&self) -> i64 {
        self.touched
            .iter()
            .filter(|&&(c, was)| was && self.con_cnt[c as usize] == 0)
            .map(|&(c, _)| self.w[c as usize] as i64)
            .sum()
    }

    /// Which constraint clauses are true now (for --check).
    fn true_now(&self) -> Vec<bool> {
        self.con_cnt.iter().map(|&c| c > 0).collect()
    }

    /// --check: the move's break against the clauses true before and false after.
    fn check_break(&self, before: &[bool], br: i64) {
        let want: i64 = (0..before.len())
            .filter(|&c| before[c] && self.con_cnt[c] == 0)
            .map(|c| self.w[c] as i64)
            .sum();
        if want != br {
            fail(&format!("a move's break is {br}, but {want} by the clauses true before and false after"));
        }
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
    /// step of the walk.
    /// The value asked for comes with a group: Some(member) when the walk
    /// needs that member true, None when it needs the true member false.
    fn pick_source(&self, m: &Model, ci: usize, rng: &mut Rng) -> Option<(Src, Option<u32>)> {
        let (a, b) = (m.c_start[ci] as usize, m.c_start[ci + 1] as usize);
        let mut t = m.c_lit[a + rng.below(b - a)];
        for _ in 0..WALK_DEPTH {
            let v = lvar(t);
            match m.kind[v] {
                K_CHOICE => {
                    let g = m.var_group[v] as usize;
                    let a = m.g_start[g] as usize;
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

struct Params {
    seed: u64,
    seconds: f64,
    max_steps: u64,
    noise: f64,
    cands: usize,
    target_only: bool,
    score_break: bool,
    tenure: u32,
    tenure_rand: u32,
    wmode: u8,
    smooth: u64,
    bump_best: bool,
    phase: f64,
    check: u64,
    progress: f64,
    dump: bool,
}

fn usage() -> ! {
    eprintln!(
        "usage: free-walk READING [--seed N] [--seconds S] [--steps N] [--noise P] [--cands N]\n\
         \x20                [--tenure N] [--tenure-rand N] [--weights none|clause|all] [--smooth N]\n\
         \x20                [--values all|target] [--score net|break] [--bump-at chosen|best] [--phase S]\n\
         \x20                [--check N] [--progress S] [--dump]\n\
         defaults: seed 0, 60 s, noise 0.05, 16 candidates, no tabu, weights all, smooth 3,\n\
         values all, score net, bump at chosen, phase 30 s; --check N recomputes the state every N\n\
         steps and stops on a difference; --dump prints the assignment where the search stopped"
    );
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
    }
    let mut path = None;
    let mut p = Params {
        seed: 0,
        seconds: 60.0,
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
        phase: 30.0,
        check: 0,
        progress: 10.0,
        dump: false,
    };
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        let val = |i: usize| -> &str {
            if i + 1 >= args.len() {
                usage();
            }
            args[i + 1].as_str()
        };
        let num = |s: &str| -> f64 { s.parse::<f64>().unwrap_or_else(|_| usage()) };
        match a {
            "--seed" => p.seed = num(val(i)) as u64,
            "--seconds" => p.seconds = num(val(i)),
            "--steps" => p.max_steps = num(val(i)) as u64,
            "--noise" => p.noise = num(val(i)),
            "--cands" => p.cands = (num(val(i)) as usize).max(1),
            "--values" => {
                p.target_only = match val(i) {
                    "target" => true,
                    "all" => false,
                    _ => usage(),
                }
            }
            "--score" => {
                p.score_break = match val(i) {
                    "break" => true,
                    "net" => false,
                    _ => usage(),
                }
            }
            "--tenure" => p.tenure = num(val(i)) as u32,
            "--tenure-rand" => p.tenure_rand = num(val(i)) as u32,
            "--weights" => {
                p.wmode = match val(i) {
                    "none" => 0,
                    "clause" => 1,
                    "all" => 2,
                    _ => usage(),
                }
            }
            "--smooth" => p.smooth = num(val(i)) as u64,
            "--bump-at" => {
                p.bump_best = match val(i) {
                    "best" => true,
                    "chosen" => false,
                    _ => usage(),
                }
            }
            "--phase" => p.phase = num(val(i)),
            "--check" => p.check = num(val(i)) as u64,
            "--progress" => p.progress = num(val(i)),
            "--dump" => {
                p.dump = true;
                i += 1;
                continue;
            }
            _ if a.starts_with("--") => usage(),
            _ => {
                if path.is_some() {
                    usage();
                }
                path = Some(a.to_string());
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    let path = path.unwrap_or_else(|| usage());
    let t0 = Instant::now();
    let m = Model::load(&path);
    let load_s = t0.elapsed().as_secs_f64();
    let mut rng = Rng::new(p.seed);
    let mut s = State::new(&m, &mut rng);
    let init_false = s.fl.len();
    println!(
        "c free-walk: {} variables, {} groups ({} members), {} free, {} computed, {} constraint clauses; \
         loaded in {:.2} s; {} clauses false at the start (seed {})",
        m.nv,
        m.ngroups(),
        m.g_lit.len(),
        m.free.len(),
        m.nnodes(),
        m.ncons(),
        load_s,
        init_false,
        p.seed
    );
    let r = search(&m, &mut s, &p, &mut rng, t0);
    let secs = t0.elapsed().as_secs_f64() - load_s;
    let out = std::io::stdout();
    let mut out = BufWriter::new(out.lock());
    let _ = writeln!(
        out,
        "c free-walk: result={} search_seconds={:.3} steps={} trials={} flips={} updates={} pushes={} best={} best_step={}",
        if r.found { "SAT" } else { "UNKNOWN" },
        secs,
        r.steps,
        r.trials,
        s.flips,
        s.upd,
        s.pushes,
        r.best,
        r.best_step
    );
    if r.found {
        if let Err(e) = s.check(&m) {
            fail(&format!("the state is out of step at the end: {e}"));
        }
        if !s.fl.is_empty() {
            fail("a model with false constraint clauses");
        }
        let _ = writeln!(out, "s SATISFIABLE");
    } else {
        let _ = writeln!(out, "s UNKNOWN");
    }
    if r.found || p.dump {
        // the model, or with --dump the assignment where the search stopped
        let mut line = String::from("v");
        for v in 1..=m.nv {
            let b = match m.kind[v] {
                K_FIXED => m.fixed[v],
                K_ABSENT => 0,
                _ => s.val[v],
            };
            let tok = if b == 1 { format!(" {v}") } else { format!(" -{v}") };
            if line.len() + tok.len() > 4000 {
                let _ = writeln!(out, "{line}");
                line = String::from("v");
            }
            line.push_str(&tok);
        }
        line.push_str(" 0");
        let _ = writeln!(out, "{line}");
    }
    let _ = out.flush();
    std::process::exit(if r.found { 10 } else { 0 });
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

struct Outcome {
    found: bool,
    steps: u64,
    trials: u64,
    best: usize,
    best_step: u64,
}

fn search(m: &Model, s: &mut State, p: &Params, rng: &mut Rng, t0: Instant) -> Outcome {
    let ng = m.ngroups();
    // tabu until step: per group value (members, then "none") and per free bit
    let mut g_tabu_start = Vec::with_capacity(ng + 1);
    let mut tot = 0usize;
    for g in 0..ng {
        g_tabu_start.push(tot);
        tot += m.gvalues(g);
    }
    g_tabu_start.push(tot);
    let mut g_tabu = vec![0u64; tot];
    let mut f_tabu = vec![0u64; m.free.len()];
    let mut best = s.fl.len();
    let mut best_step = 0u64;
    let mut steps = 0u64;
    let mut trials = 0u64;
    let mut bumps = 0u64;
    let mut cands: Vec<(Src, Option<u32>)> = Vec::with_capacity(p.cands);
    let mut heavy: Vec<u32> = Vec::new(); // clauses of weight above 1
    let mut weights_on = p.wmode != 0;
    let mut heavy_pos = vec![u32::MAX; m.ncons()];
    let mut moves: Vec<Cand> = Vec::new();
    let mut last_progress = 0.0;
    loop {
        if s.fl.is_empty() {
            return Outcome { found: true, steps, trials, best: 0, best_step: steps };
        }
        if steps >= p.max_steps {
            break;
        }
        if steps & 255 == 0 {
            let t = t0.elapsed().as_secs_f64();
            if p.phase > 0.0 && p.wmode != 0 {
                // alternate: weighted for one phase, plain for the next (weights back to 1)
                let on = ((t / p.phase) as u64) % 2 == 0;
                if on != weights_on {
                    weights_on = on;
                    if !on {
                        for &c in &heavy {
                            s.w[c as usize] = 1;
                            heavy_pos[c as usize] = u32::MAX;
                        }
                        heavy.clear();
                        s.cost = s.fl.len() as u64;
                    }
                }
            }
            if t >= p.seconds {
                break;
            }
            if p.progress > 0.0 && t - last_progress >= p.progress {
                last_progress = t;
                eprintln!(
                    "c t={:.1} steps={} trials={} flips={} false={} best={} cost={}",
                    t,
                    steps,
                    trials,
                    s.flips,
                    s.fl.len(),
                    best,
                    s.cost
                );
            }
        }
        steps += 1;
        let ci = s.fl[rng.below(s.fl.len())] as usize;
        cands.clear();
        for _ in 0..p.cands {
            if let Some(src) = s.pick_source(m, ci, rng) {
                if !cands.contains(&src) {
                    cands.push(src);
                }
            }
        }
        if cands.is_empty() {
            continue;
        }
        moves.clear();
        for &(src, want) in &cands {
            match src {
                Src::Group(g) => {
                    let cur = s.g_cur[g as usize];
                    for b in 0..m.gvalues(g as usize) as u32 {
                        if b == cur || (p.target_only && want.is_some_and(|j| j != b)) {
                            continue;
                        }
                        if moves.iter().any(|c: &Cand| c.src == src && c.value == b) {
                            continue;
                        }
                        let (c0, n0) = (s.cost as i64, s.fl.len() as i64);
                        let before = if p.check > 0 { Some(s.true_now()) } else { None };
                        let old = s.apply(m, src, b);
                        let (dw, dn, br) = (s.cost as i64 - c0, s.fl.len() as i64 - n0, s.last_break());
                        if let Some(bf) = &before {
                            s.check_break(bf, br);
                        }
                        s.undo(m, src, old);
                        trials += 1;
                        moves.push(Cand { src, value: b, dw, dn, br });
                    }
                }
                Src::Free(_) => {
                    if moves.iter().any(|c: &Cand| c.src == src) {
                        continue;
                    }
                    let (c0, n0) = (s.cost as i64, s.fl.len() as i64);
                    let before = if p.check > 0 { Some(s.true_now()) } else { None };
                    let old = s.apply(m, src, 0);
                    let (dw, dn, br) = (s.cost as i64 - c0, s.fl.len() as i64 - n0, s.last_break());
                    if let Some(bf) = &before {
                        s.check_break(bf, br);
                    }
                    s.undo(m, src, old);
                    trials += 1;
                    moves.push(Cand { src, value: 0, dw, dn, br });
                }
            }
        }
        if moves.is_empty() {
            continue;
        }
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
            for c in &moves {
                let allowed = !tabu(c) || ((nf + c.dn) as usize) < best;
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
        if no_gain && weights_on {
            // no gain: the clause the step started from (or every false
            // clause) gets heavier; every `smooth` such steps each heavy
            // clause gets lighter by one
            bumps += 1;
            if p.wmode == 1 {
                heavier(s, &mut heavy, &mut heavy_pos, ci);
            } else {
                for i in 0..s.fl.len() {
                    let c = s.fl[i] as usize;
                    heavier(s, &mut heavy, &mut heavy_pos, c);
                }
            }
            if p.smooth > 0 && bumps % p.smooth == 0 {
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
        if s.fl.len() < best {
            best = s.fl.len();
            best_step = steps;
        }
        if p.check > 0 && steps % p.check == 0 {
            if let Err(e) = s.check(m) {
                fail(&format!("step {steps}: {e}"));
            }
        }
    }
    Outcome { found: false, steps, trials, best, best_step }
}
