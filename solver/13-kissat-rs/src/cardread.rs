//! Not in kissat. Reading counts from the input clauses, with their proofs
//! (epic SAT-playground-iin, bead iin.7; plan/count-and-hall-lemmas-plan.md).
//!
//! The reader finds every count it can in the formula and says what each one
//! means as a Card: at least `need` of `lits`. A count's clauses are checked
//! exactly when it is read, so that its proof cannot fail: detection may guess,
//! the check that follows may not. The proof is written only when a lemma first
//! needs the count. `tools/card_toy_proof.py` is the checked reference for the
//! proof lines and `tools/card_census.py` for what to read.
//!
//! What is read, in the order the budget pays for it:
//!
//! - Merge nodes, which are the columns of a Sinz counter and the nodes of a
//!   totalizer: children a and b, outputs r, clauses (-a_i v -b_j v r_(i+j)).
//!   Their outputs link them into trees. A unit -r_m gives "at most m - 1 of the
//!   leaves", and overflow clauses at the root give "at most c of the leaves".
//!   A backward encoding is the same clauses over negated literals, so nothing
//!   here looks at signs.
//! - Direct nodes, a tree's leaves written out: every m-subset of t inputs
//!   implies output m.
//! - Decision diagrams, from an asserted node down: n means "if y then A else B",
//!   written (-n v -y v A) and (-n v y v B), with or without (-n v A v B), one way
//!   or both. A diagram whose variable order holds says at least val(root) of the
//!   level literals, and one that says "exactly" gives the other bound too.
//! - Comparator networks, from a unit on a wire back to the inputs: (a, b) gives
//!   max h and min l by (-a v h), (-b v h), (-a v -b v l), or one of the two where
//!   the other output was left out. The sum over the network gives at most (free
//!   outputs + left-out outputs) of the inputs. A single unit on output k + 1 of a
//!   sorting network gives at most k only because the network sorts: that is read
//!   from sorted vectors merged two at a time, or for few inputs one subset at a time.
//! - Adder networks: cells a + b + c <= s + 2m and a + b <= s + 2m, written as the
//!   clauses of an XOR and a majority or an AND, each wire read once at the weight
//!   it was made at, and units or compare clauses on the bits nothing reads. A
//!   network the other way round is the same cells over negated literals.
//! - One-way counters, from the asserted register down: s means "at least j of
//!   the first i", written (-s v lo v x) and (-s v hi) or (-s v lo v hi).
//! - Subset families: every m-subset of t literals as a clause.
//!
//! A bound of one is a clause: of these only a long one-way chain reads as a count.
//!
//! Detection is free to be wrong. What stands between a misreading and a wrong
//! answer are the checks at the end of each reading (`check_slot` with
//! `check_count`, `check_walk`, `check_family`, `d_check`, `check_network`,
//! `net_sum`, `v_check`): they look up every clause a line of the proof relies on
//! and check that no variable appears twice. They do not
//! reuse anything detection worked out about the clauses. If they pass, the lines
//! go through VeriPB, for these reasons:
//!
//! - A node's `pbc` adds its children's constraints to the negation. That is
//!   arithmetic, and it leaves (a_(i+1) v b_(t-i)) for every split. With those and
//!   the node's clauses, which the check found, propagation runs from one end of
//!   the splits to the other (`tools/card_toy_proof.py` has the derivation). More
//!   constraints in the checker can only make propagation stop sooner.
//! - A one-way register, the same, with its lo and hi.
//! - A subset family: the division rounds every sum to exactly the next size up.
//! - A diagram node, as a one-way register: its value is at most its false child's
//!   and at most one more than its true child's, which is all the arithmetic needs
//!   for propagation to close (`tools/card_toy_proof.py`). No case split is needed,
//!   and neither is the third clause.
//! - An adder cell: its lines need its clauses and nothing else. The network's sum
//!   is arithmetic, which comes out right when every wire is read once at its
//!   weight, and the check on the bits runs the propagation VeriPB will run.
//! - A comparator: the division rounds the sum of its three clauses to exactly
//!   a + b <= h + l. A merge: the check runs the comparators forward for every case
//!   the `rup` lines take, which is the propagation those lines rely on.
//!
//! Budgets: every table and every scan is paid for before it is made, every loop
//! pays per item, and no hash map is ever walked, so the same formula gives the
//! same `ticks`. When the money runs out the reader stops and keeps the counts
//! it has already checked. Networks and adders may spend half of what is left each,
//! and a new diagram starts only while half is untouched, so that none starves what
//! comes after. `prove` charges only its `work` argument.
//!
//! Proofs: a count's constraints are written once and shared with the other counts
//! that need them. When no count still to be proved needs a helper constraint it
//! is deleted from the proof, because a checker's work on each line grows with
//! what it holds (ITC2021_Late_10 took VeriPB over 30 minutes without the deletion).

use crate::structproof::Writer;
use crate::structure::Formula;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

/// The longest clause in the lookup table (the census's limit for subsets).
const MAX_LEN: usize = 9;
/// The largest direct node or subset family: their proofs go through a table
/// with one entry per subset of the inputs.
const MAX_T: usize = 14;
/// The longest list of literals a walk looks through: a literal implied by more
/// than this is a hub, not a register.
const MAX_DEG: usize = 1024;
/// The most literals that may imply a first output: its pairs are all tried.
const MAX_PRE: usize = 16;
/// The most short clauses a one-way register may hold negated: two for itself,
/// two more when its counter is written both ways.
const MAX_REG: usize = 8;
/// The shortest chain read as "at least one of": shorter ones are just resolvents
/// of the gates around an asserted literal.
const MIN_CHAIN: usize = 5;
/// The most outputs a direct node's reading looks for (the census's limit).
const MAX_DIRECT_OUT: usize = 7;
const NONE: u32 = u32::MAX;
/// The most clauses of one length a literal may have and still be looked at as an
/// adder's carry or sum, or a network's wire: a wire is in a few gates.
const MAX_WIRE: usize = 16;
/// The most literals that may imply a carry written as an OR: its two inputs, and
/// what reads it, in a network written both ways.
const MAX_CARRY_PRE: usize = 4;
/// The most bits an adder network's number may have.
const MAX_BITS: usize = 30;
/// How many half comparators in a row a wire may hang on before a full comparator.
const MAX_HALF: u32 = 16;
/// The most inputs a sorted vector's reading looks at.
const MAX_NET: usize = 4096;
/// A diagram node that can never be true.
const DEAD: u32 = u32::MAX;
/// While a diagram is read: a child that is no literal.
const T_KID: i32 = i32::MAX;
const F_KID: i32 = i32::MIN;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Form {
    Sinz,
    Totalizer,
    OneWay,
    Subset,
    Diagram,
    Network,
    Adder,
}

/// At least `need` of `lits`: external literals, each variable once.
#[derive(Clone)]
pub struct Found {
    pub lits: Vec<i32>,
    pub need: i32,
    pub form: Form,
}

#[derive(Clone, Copy)]
enum Plan {
    /// A merge tree's constraint: a cap as it stands, or an output's constraint
    /// added to its unit clause (the clause id and the multiplier). `del` says the
    /// proof has helper constraints to delete at its end.
    Merge { slot: u32, unit: Option<(u64, i64)>, del: bool },
    OneWay { walk: u32 },
    Subset { family: u32 },
    Diagram { count: u32 },
    Adder { net: u32 },
    /// A comparator network's sum.
    NetSum { net: u32 },
    /// A sorted vector's constraint with the unit on its wire.
    NetTree { slot: u32, unit: u64, mult: i64, order_start: u32, order_len: u32 },
    /// Every t-subset of the inputs, then the sums of the subsets.
    NetSubsets { net: u32 },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kid {
    Leaf(i32),
    Node(u32),
    /// A vector no node defines.
    Open,
}

const NEW: u8 = 0;
const ON_PATH: u8 = 1;
const DONE: u8 = 2;
const BAD: u8 = 3;

/// A merge node, or a direct node when `xs` is not empty.
#[derive(Clone)]
struct Node {
    a: Kid,
    b: Kid,
    /// What the clauses showed of the children, until they are linked.
    va: Vec<i32>,
    vb: Vec<i32>,
    /// A direct node's inputs.
    xs: Vec<i32>,
    /// Outputs r_1, r_2, ...
    r: Vec<i32>,
    /// The count its cap speaks of: at most this many of the leaves.
    cap_c: u32,
    leaf_start: u32,
    leaf_len: u32,
    /// The first proof slot: outputs 1..=r.len(), then the cap.
    slot: u32,
    state: u8,
    /// Every merge below has a leaf for a child: a sequential counter.
    chain: bool,
    parent: bool,
    /// The clauses show an overflow clause of this node.
    overflow: bool,
}

fn new_node() -> Node {
    Node { a: Kid::Open, b: Kid::Open, va: Vec::new(), vb: Vec::new(), xs: Vec::new(), r: Vec::new(), cap_c: 0, leaf_start: 0, leaf_len: 0, slot: NONE, state: NEW, chain: false, parent: false, overflow: false }
}

/// One constraint a merge or direct node can prove: for output t, E(t), which is
/// M r_t + sum of ~l >= M with M = leaves - t + 1; for the cap, the same with no
/// output and t = c + 1.
#[derive(Clone)]
struct Slot {
    node: u32,
    t: u32,
    cap: bool,
    /// 0 not looked at, 1 good, 2 bad.
    state: u8,
    stamp: u32,
    /// A direct node's clause ids: where they start in `ids`.
    aux: u32,
    /// The proof id once written.
    id: u64,
    /// How many counts not yet proved need it. At 0 it is deleted from the proof.
    uses: u32,
    /// It is a count's own constraint and is never deleted.
    keep: bool,
}

/// A register of a one-way counter: T(s) is j ~s + sum of the first i inputs >= j.
#[derive(Clone)]
struct Reg {
    s: i32,
    j: u32,
    /// Its inputs are the walk's from this position to the end.
    d: u32,
    lo: u32,
    hi: u32,
    tslot: u32,
}

#[derive(Clone)]
struct Walk {
    /// The inputs, last one first.
    in_start: u32,
    in_len: u32,
    reg_start: u32,
    reg_len: u32,
    k: u32,
    unit_id: u64,
}

#[derive(Clone)]
struct Family {
    /// The family's literals: the clause literals, so at least t - m + 1 of them.
    lits: Vec<i32>,
    m: u32,
    /// Clause ids of the m-subsets, by ascending bit mask over `lits`.
    ids: Vec<u32>,
    id: u64,
}

/// What a diagram node proves. T(n): c ~n + the inputs from its level on >= c, or
/// ~n >= 1 when nothing below it can hold (`c` is `DEAD`).
#[derive(Clone)]
struct DRec {
    lit: i32,
    c: u32,
    /// Its inputs, in `d_inputs`.
    start: u32,
    len: u32,
    /// The constraints its proof adds: the false child's, then the true child's.
    kids: [u32; 2],
    /// y with (-n v y) proved by a line of its own first: an exact grid's edge, where
    /// the node's own clause on that side says less than the other clauses do.
    lemma: i32,
    lemma_id: u64,
    /// Where it sits in the order of writing: a `del` closes each level.
    level: u32,
    id: u64,
    /// The parents not yet written and the counts not yet proved that need it.
    pending: u32,
    /// A trial run of the writing, for the line count.
    sim: u32,
    stamp: u32,
}

/// A count of a diagram.
#[derive(Clone)]
struct DCount {
    order_start: u32,
    order_len: u32,
    root: u32,
    unit_id: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CellKind {
    /// a + b + c <= s + 2m
    Full,
    /// a + b <= s + 2m
    Half,
    /// a + b + 1 <= s + 2m: a half adder the other way round
    HalfOne,
}

/// One adder cell. For a half adder `c` is 0.
#[derive(Clone, Copy)]
struct ACell {
    kind: CellKind,
    a: i32,
    b: i32,
    c: i32,
    s: i32,
    m: i32,
}

/// An adder network and the bound on its bits.
#[derive(Clone)]
struct ANet {
    /// The cells, each with the weight of its inputs.
    cells: Vec<(ACell, u32)>,
    /// The bits nothing reads, with their weights.
    bits: Vec<(i32, u32)>,
    /// sum of 2^w ~bit >= k
    k: u64,
}

/// A comparator, or half of one: `h` or `l` is 0 when that output was left out.
#[derive(Clone, Copy)]
struct Gate {
    a: i32,
    b: i32,
    h: i32,
    l: i32,
}

/// The sum over a comparator network.
#[derive(Clone)]
struct NetSum {
    /// Per gate: its clause ids, (-a v h), (-b v h), (-a v -b v l); a half comparator
    /// has its one clause first, and `other` the input it adds as an axiom.
    gates: Vec<([u64; 3], i32)>,
    /// The units on false outputs.
    units: Vec<u64>,
    /// The outputs that are free, weakened away.
    free: Vec<i32>,
}

/// Every t-subset of the inputs as a clause, by propagation.
#[derive(Clone)]
struct NetSubsets {
    inputs: Vec<i32>,
    t: u32,
}

/// A sorted vector: wires that say "at least rank of the leaves".
#[derive(Clone)]
struct VNode {
    leaf_start: u32,
    leaf_len: u32,
    a: u32,
    b: u32,
    /// (rank, wire, slot), by rank. A leaf has one wire, its input, and no slot.
    wires: Vec<(u32, i32, u32)>,
}

/// E(t): M r_t + sum of ~leaves >= M, with M = leaves - t + 1.
#[derive(Clone)]
struct VSlot {
    node: u32,
    t: u32,
    lit: i32,
    id: u64,
}

#[derive(Clone, Default)]
pub struct Reading {
    pub found: Vec<Found>,
    plans: Vec<Plan>,
    line_counts: Vec<u64>,
    proved: Vec<u64>,
    nodes: Vec<Node>,
    leaves: Vec<i32>,
    slots: Vec<Slot>,
    ids: Vec<u32>,
    epoch: u32,
    regs: Vec<Reg>,
    walks: Vec<Walk>,
    walk_inputs: Vec<i32>,
    /// One proof id per distinct T statement, shared by the walks that hold it.
    tslots: Vec<u64>,
    /// How many counts not yet proved need each T statement.
    tslot_uses: Vec<u32>,
    /// Where each T statement's inputs are, to compare two of them.
    tslot_inputs: Vec<(u32, u32)>,
    families: Vec<Family>,
    d_recs: Vec<DRec>,
    d_counts: Vec<DCount>,
    /// The counted literals of each reading, the first level first.
    d_inputs: Vec<i32>,
    /// The records of each count in the order of writing: children first.
    d_order: Vec<u32>,
    a_nets: Vec<ANet>,
    net_sums: Vec<NetSum>,
    net_subsets: Vec<NetSubsets>,
    v_nodes: Vec<VNode>,
    v_slots: Vec<VSlot>,
    v_leaves: Vec<i32>,
    v_order: Vec<u32>,
    /// The test hook is on (`SAT_CARD_READ_CHECK`): say what was proved.
    hook: bool,
    /// The number of input clauses: an id above it is one of the proof's own.
    input: u64,
}

enum Term {
    /// Nothing needed: a leaf's constraint is an axiom, and past the leaves
    /// nothing can be true.
    None,
    Slot(u32),
    /// The child stops counting short of what is needed.
    Bad,
}

#[inline]
fn li(l: i32) -> usize {
    ((l.unsigned_abs() as usize) << 1) | (l < 0) as usize
}

#[inline]
fn var(l: i32) -> usize {
    l.unsigned_abs() as usize
}

#[inline]
fn mix(l: i32) -> u64 {
    let x = (l as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let y = (x ^ (x >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    y ^ (y >> 32)
}

/// A hash of a set of literals that does not depend on their order.
fn set_hash(lits: &[i32]) -> u64 {
    lits.iter().fold(0u64, |h, &l| h.wrapping_add(mix(l)))
}

fn binom(n: u64, k: u64) -> u64 {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut r = 1u64;
    for i in 0..k {
        r = r.saturating_mul(n - i) / (i + 1);
    }
    r
}

/// Of the literals that could be a node's next output, the one numbered closest
/// to the last output. Two trees over the same children have an output each for
/// every count, and encoders number a node's outputs together. This is a guess,
/// and only a guess: a wrong one loses a count, because the check comes later.
fn closest(cands: impl Iterator<Item = i32>, last: i32) -> Option<i32> {
    cands.min_by_key(|&o| ((o.unsigned_abs() as i64 - last.unsigned_abs() as i64).abs(), o))
}

/// The next larger number with the same count of bits.
#[inline]
fn next_mask(x: u32) -> u32 {
    let c = x & x.wrapping_neg();
    let r = x + c;
    (((r ^ x) >> 2) / c) | r
}

/// The `pol` lines that sum the m-subsets of t literals up to all t.
fn lattice_pols(t: usize, m: usize) -> u64 {
    if m >= t {
        0
    } else if m == 1 {
        1
    } else if m == 2 {
        t as u64 - 2
    } else {
        ((m + 1)..=t).map(|s| binom(t as u64, s as u64)).sum()
    }
}

/// Those lines, and one `del` when there is more than one of them.
fn lattice_lines(t: usize, m: usize) -> u64 {
    let pols = lattice_pols(t, m);
    pols + (pols >= 2) as u64
}

/// Add up the clauses of a family. `ids` are the clause ids of the m-subsets by
/// ascending bit mask. What comes out, for the whole set T of t literals, is
/// sum over T of y >= t - m + 1, and with a guard in every clause the guard has
/// that same coefficient. One size at a time: for T of size s + 1, sum its
/// s-subsets' constraints and divide by s. A guard rounds up to the right
/// coefficient too. For m = 1 one sum is enough, for m = 2 the prefixes are (as
/// in `structcount`), and for m = t the clause is already the answer. The steps on
/// the way are deleted at the end.
fn write_lattice(w: &mut Writer, t: usize, m: usize, ids: &[u32], work: &mut u64) -> Option<u64> {
    write_lattice_del(w, t, m, ids, &[], work)
}

/// The same, with more ids for its `del` line.
fn write_lattice_del(w: &mut Writer, t: usize, m: usize, ids: &[u32], also: &[u64], work: &mut u64) -> Option<u64> {
    *work += also.len() as u64;
    if m >= t {
        return Some(ids[0] as u64);
    }
    if m == 1 {
        let mut expr = ids[0].to_string();
        for id in &ids[1..] {
            expr.push_str(&format!(" {} +", id));
        }
        *work += ids.len() as u64;
        let id = w.pol(&expr).ok()?;
        w.delete(also).ok()?;
        return Some(id);
    }
    *work += 1u64 << t;
    let mut by: Vec<u64> = vec![0; 1usize << t];
    let mut mask = (1u32 << m) - 1;
    for &id in ids {
        by[mask as usize] = id as u64;
        mask = next_mask(mask);
    }
    let mut made: Vec<u64> = Vec::new();
    if m == 2 {
        let mut cur = by[0b11];
        for s in 2..t {
            let mut expr = by[1 | (1usize << s)].to_string();
            for i in 1..s {
                expr.push_str(&format!(" {} +", by[(1usize << i) | (1usize << s)]));
            }
            expr.push_str(&format!(" {} {} * + {} d", cur, s - 1, s));
            *work += s as u64 + 2;
            cur = w.pol(&expr).ok()?;
            made.push(cur);
        }
        made.pop();
        made.extend_from_slice(also);
        if !made.is_empty() {
            *work += made.len() as u64;
            w.delete(&made).ok()?;
        }
        return Some(cur);
    }
    for size in (m + 1)..=t {
        let mut mask = (1u32 << size) - 1;
        while mask < (1u32 << t) {
            let mut expr = String::new();
            let mut rest = mask;
            while rest != 0 {
                let bit = rest & rest.wrapping_neg();
                rest ^= bit;
                let id = by[(mask ^ bit) as usize];
                if expr.is_empty() {
                    expr = id.to_string();
                } else {
                    expr.push_str(&format!(" {} +", id));
                }
            }
            expr.push_str(&format!(" {} d", size - 1));
            *work += size as u64 + 1;
            let id = w.pol(&expr).ok()?;
            by[mask as usize] = id;
            made.push(id);
            mask = next_mask(mask);
        }
    }
    made.pop();
    made.extend_from_slice(also);
    if !made.is_empty() {
        *work += made.len() as u64;
        w.delete(&made).ok()?;
    }
    Some(by[(1usize << t) - 1])
}

/// A literal as an axiom in a `pol` line.
fn lit_axiom(l: i32) -> String {
    if l > 0 {
        format!("x{}", l)
    } else {
        format!("~x{}", -l)
    }
}

/// `pbc`, its `rup` lines, `qed`.
fn a_cell_lines(kind: CellKind) -> u64 {
    match kind {
        CellKind::Full => 9,
        CellKind::Half => 5,
        CellKind::HalfOne => 4,
    }
}

fn rup_line(lits: &[i32]) -> String {
    if lits.is_empty() {
        return "rup >= 1 ;".to_string();
    }
    let mut l = String::from("rup");
    for &q in lits {
        l.push_str(if q > 0 { " 1 x" } else { " 1 ~x" });
        l.push_str(&q.unsigned_abs().to_string());
    }
    l.push_str(" >= 1 ;");
    l
}

/// inputs (+ 1) <= s + 2m, as sum of ~inputs + s + 2m >= the number of them (+ 1).
/// `tools/card_toy_proof.py` has why these lines close.
fn a_write_cell(c: ACell, w: &mut Writer, work: &mut u64) -> Option<u64> {
    let (terms, rhs, lines): (Vec<(i128, i32)>, i128, Vec<String>) = match c.kind {
        CellKind::Full => (
            vec![(1, -c.a), (1, -c.b), (1, -c.c), (1, c.s), (2, c.m)],
            3,
            vec![rup_line(&[-c.m]), rup_line(&[c.s, -c.a]), rup_line(&[c.s, -c.b]), rup_line(&[c.s, -c.c]), rup_line(&[c.s]), rup_line(&[c.a]), rup_line(&[])],
        ),
        CellKind::Half => (vec![(1, -c.a), (1, -c.b), (1, c.s), (2, c.m)], 2, vec![rup_line(&[-c.a]), rup_line(&[-c.b]), rup_line(&[])]),
        CellKind::HalfOne => (vec![(1, -c.a), (1, -c.b), (1, c.s), (2, c.m)], 3, vec![rup_line(&[-c.m]), rup_line(&[])]),
    };
    *work += terms.len() as u64 + lines.len() as u64;
    w.pbc_raw(&terms, rhs, &lines).ok()
}

impl Reading {
    /// The count a child shows: its outputs, but no more than it has leaves.
    fn kid_shows(&self, k: Kid) -> i64 {
        match k {
            Kid::Leaf(_) => 1,
            Kid::Node(c) => {
                let nd = &self.nodes[c as usize];
                (nd.r.len() as i64).min(nd.leaf_len as i64)
            }
            Kid::Open => 0,
        }
    }

    fn kid_out(&self, k: Kid, i: i64) -> i32 {
        match k {
            Kid::Leaf(l) => l,
            Kid::Node(c) => self.nodes[c as usize].r[i as usize - 1],
            Kid::Open => 0,
        }
    }

    /// The splits of t between the children that have a clause: i from a and
    /// t - i from b, for i from the first number to the second.
    fn splits(&self, nd: &Node, t: i64) -> (i64, i64) {
        let imax = t.min(self.kid_shows(nd.a));
        let imin = (t - self.kid_shows(nd.b)).max(0);
        (imin, imax)
    }

    /// What the proof needs of a child's count x.
    fn term(&self, k: Kid, x: i64) -> Term {
        match k {
            Kid::Leaf(_) => Term::None,
            Kid::Open => Term::Bad,
            Kid::Node(c) => {
                let nd = &self.nodes[c as usize];
                let w = nd.r.len() as i64;
                if x > nd.leaf_len as i64 {
                    Term::None
                } else if x <= w {
                    Term::Slot(nd.slot + x as u32 - 1)
                } else if x == w + 1 {
                    Term::Slot(nd.slot + w as u32)
                } else {
                    Term::Bad
                }
            }
        }
    }

    /// The slots slot `s` is summed from. False when a child hides counts.
    fn deps(&self, s: u32, out: &mut Vec<u32>) -> bool {
        out.clear();
        let sl = &self.slots[s as usize];
        let nd = &self.nodes[sl.node as usize];
        if !nd.xs.is_empty() {
            return true;
        }
        let t = sl.t as i64;
        let (imin, imax) = self.splits(nd, t);
        let mut i = imax;
        while i >= imin - 1 {
            let (x, y) = (i + 1, t - i);
            i -= 1;
            if x < 1 || y < 1 {
                continue;
            }
            for (k, v) in [(nd.a, x), (nd.b, y)] {
                match self.term(k, v) {
                    Term::None => {}
                    Term::Slot(d) => out.push(d),
                    Term::Bad => return false,
                }
            }
        }
        true
    }

    /// The `pol` lines of a merge slot, one per split.
    fn slot_pols(&self, nd: &Node, t: i64) -> u64 {
        let (imin, imax) = self.splits(nd, t);
        let mut lines = 0u64;
        let mut i = imax;
        while i >= imin - 1 {
            if i + 1 >= 1 && t - i >= 1 {
                lines += 1;
            }
            i -= 1;
        }
        lines
    }

    /// The proof lines of one slot: `pbc`, its `pol` lines, `rup`, `qed`.
    fn slot_lines(&self, s: u32) -> u64 {
        let sl = &self.slots[s as usize];
        let nd = &self.nodes[sl.node as usize];
        if nd.xs.is_empty() {
            3 + self.slot_pols(nd, sl.t as i64)
        } else {
            lattice_lines(nd.xs.len(), sl.t as usize)
        }
    }

    fn write_slot(&mut self, s: u32, w: &mut Writer, work: &mut u64) -> Option<()> {
        let (node, t, cap) = {
            let sl = &self.slots[s as usize];
            (sl.node as usize, sl.t as i64, sl.cap)
        };
        if !self.nodes[node].xs.is_empty() {
            let aux = self.slots[s as usize].aux as usize;
            let n = self.nodes[node].xs.len();
            let count = binom(n as u64, t as u64) as usize;
            let id = write_lattice(w, n, t as usize, &self.ids[aux..aux + count], work)?;
            self.slots[s as usize].id = id;
            return Some(());
        }
        let neg = w.next_id();
        let nd = &self.nodes[node];
        let (imin, imax) = self.splits(nd, t);
        let mut lines: Vec<String> = Vec::with_capacity((imax - imin + 3).max(1) as usize);
        let mut i = imax;
        while i >= imin - 1 {
            let (x, y) = (i + 1, t - i);
            i -= 1;
            if x < 1 || y < 1 {
                continue;
            }
            let mut line = format!("pol {}", neg);
            for (k, v) in [(nd.a, x), (nd.b, y)] {
                if let Term::Slot(d) = self.term(k, v) {
                    line.push_str(&format!(" {} +", self.slots[d as usize].id));
                }
            }
            line.push_str(" ;");
            lines.push(line);
        }
        lines.push("rup >= 1 ;".to_string());
        let n = nd.leaf_len as i64;
        let m = n - t + 1;
        let mut terms: Vec<(i128, i32)> = Vec::with_capacity(n as usize + 1);
        if !cap {
            terms.push((m as i128, nd.r[t as usize - 1]));
        }
        for &l in &self.leaves[nd.leaf_start as usize..(nd.leaf_start + nd.leaf_len) as usize] {
            terms.push((1, -l));
        }
        *work += terms.len() as u64 + lines.len() as u64;
        let id = w.pbc_raw(&terms, m as i128, &lines).ok()?;
        self.slots[s as usize].id = id;
        Some(())
    }

    /// The lines a slot's proof still has to write, given what is written.
    fn lines_left(&mut self, root: u32, work: &mut u64) -> u64 {
        self.epoch = self.epoch.wrapping_add(1);
        let e = self.epoch;
        let mut total = 0u64;
        let mut stack = vec![root];
        let mut buf: Vec<u32> = Vec::new();
        self.slots[root as usize].stamp = e;
        while let Some(s) = stack.pop() {
            *work += 1;
            if self.slots[s as usize].id != 0 {
                continue;
            }
            total += self.slot_lines(s);
            let _ = self.deps(s, &mut buf);
            *work += buf.len() as u64;
            for &d in &buf {
                if self.slots[d as usize].stamp != e {
                    self.slots[d as usize].stamp = e;
                    stack.push(d);
                }
            }
        }
        total
    }

    /// Write a slot's proof: what it is summed from first.
    fn write_tree(&mut self, root: u32, w: &mut Writer, work: &mut u64) -> Option<()> {
        let mut stack: Vec<(u32, bool)> = vec![(root, false)];
        let mut buf: Vec<u32> = Vec::new();
        while let Some((s, expanded)) = stack.pop() {
            if self.slots[s as usize].id != 0 {
                continue;
            }
            if expanded {
                self.write_slot(s, w, work)?;
                continue;
            }
            stack.push((s, true));
            let _ = self.deps(s, &mut buf);
            *work += buf.len() as u64 + 1;
            for &d in buf.iter().rev() {
                if self.slots[d as usize].id == 0 {
                    stack.push((d, false));
                }
            }
        }
        Some(())
    }

    /// A count is proved: what only it needed leaves the proof, so that the
    /// checker's database, and its work on every later line, stays small.
    fn drop_helpers(&mut self, root: u32, w: &mut Writer, work: &mut u64) -> Option<()> {
        self.epoch = self.epoch.wrapping_add(1);
        let e = self.epoch;
        let mut stack = vec![root];
        let mut buf: Vec<u32> = Vec::new();
        let mut ids: Vec<u64> = Vec::new();
        self.slots[root as usize].stamp = e;
        while let Some(s) = stack.pop() {
            *work += 1;
            let sl = &mut self.slots[s as usize];
            sl.uses = sl.uses.saturating_sub(1);
            if sl.uses == 0 && !sl.keep && sl.id > self.input {
                ids.push(sl.id);
            }
            let _ = self.deps(s, &mut buf);
            *work += buf.len() as u64;
            for &d in &buf {
                if self.slots[d as usize].stamp != e {
                    self.slots[d as usize].stamp = e;
                    stack.push(d);
                }
            }
        }
        if !ids.is_empty() {
            *work += ids.len() as u64;
            w.delete(&ids).ok()?;
        }
        Some(())
    }

    fn write_reg(&mut self, g: usize, walk: usize, w: &mut Writer, work: &mut u64) -> Option<()> {
        let r = &self.regs[g];
        let wk = &self.walks[walk];
        let neg = w.next_id();
        let mut lines: Vec<String> = Vec::with_capacity(3);
        for q in [r.lo, r.hi] {
            if q != NONE {
                lines.push(format!("pol {} {} + ;", neg, self.tslots[self.regs[q as usize].tslot as usize]));
            }
        }
        lines.push("rup >= 1 ;".to_string());
        let inputs = &self.walk_inputs[(wk.in_start + r.d) as usize..(wk.in_start + wk.in_len) as usize];
        let mut terms: Vec<(i128, i32)> = Vec::with_capacity(inputs.len() + 1);
        terms.push((r.j as i128, -r.s));
        for &x in inputs.iter().rev() {
            terms.push((1, x));
        }
        *work += terms.len() as u64 + lines.len() as u64;
        let id = w.pbc_raw(&terms, r.j as i128, &lines).ok()?;
        let slot = self.regs[g].tslot as usize;
        self.tslots[slot] = id;
        Some(())
    }

    fn reg_lines(&self, g: usize) -> u64 {
        let r = &self.regs[g];
        3 + (r.lo != NONE) as u64 + (r.hi != NONE) as u64
    }

    fn net_sum_lines(&self, net: u32) -> u64 {
        let n = &self.net_sums[net as usize];
        let helpers = n.gates.iter().filter(|g| g.0[1] != 0 || g.1 != 0).count() as u64;
        helpers + 1 + (helpers > 0) as u64
    }

    fn write_net_sum(&mut self, net: u32, w: &mut Writer, work: &mut u64) -> Option<u64> {
        let n = &self.net_sums[net as usize];
        let mut made: Vec<u64> = Vec::new();
        let mut expr = String::new();
        for (i, &(ids, other)) in n.gates.iter().enumerate() {
            let id = if ids[1] != 0 {
                let id = w.pol(&format!("{} {} + {} + 2 d", ids[0], ids[1], ids[2])).ok()?;
                made.push(id);
                id
            } else if other != 0 {
                let id = w.pol(&format!("{} {} +", ids[0], lit_axiom(-other))).ok()?;
                made.push(id);
                id
            } else {
                ids[0]
            };
            expr.push_str(&id.to_string());
            expr.push_str(if i > 0 { " + " } else { " " });
        }
        for &u in &n.units {
            expr.push_str(&format!("{} + ", u));
        }
        for &o in &n.free {
            expr.push_str(&format!("x{} w ", o.unsigned_abs()));
        }
        *work += n.gates.len() as u64 * 3 + n.units.len() as u64 + n.free.len() as u64 + 2;
        let id = w.pol(expr.trim_end()).ok()?;
        w.delete(&made).ok()?;
        Some(id)
    }

    fn net_subset_lines(&self, net: u32) -> u64 {
        let n = &self.net_subsets[net as usize];
        let (size, t) = (n.inputs.len(), n.t as usize);
        let rups = binom(size as u64, t as u64);
        // the sums, and one `del` for the steps and the `rup` lines; one clause is the
        // count as it stands
        rups + lattice_pols(size, t) + (rups > 1) as u64
    }

    fn write_net_subsets(&mut self, net: u32, w: &mut Writer, work: &mut u64) -> Option<u64> {
        let n = &self.net_subsets[net as usize];
        let (size, t) = (n.inputs.len(), n.t as usize);
        let mut ids: Vec<u32> = Vec::with_capacity(binom(size as u64, t as u64) as usize);
        let mut clause: Vec<i32> = Vec::with_capacity(t);
        let mut mask = (1u32 << t) - 1;
        while mask < (1u32 << size) {
            clause.clear();
            let mut rest = mask;
            while rest != 0 {
                let b = rest.trailing_zeros() as usize;
                rest &= rest - 1;
                clause.push(-n.inputs[b]);
            }
            *work += t as u64 + 1;
            ids.push(u32::try_from(w.rup(&clause).ok()?).ok()?);
            mask = next_mask(mask);
        }
        let done: Vec<u64> = if ids.len() > 1 { ids.iter().map(|&i| i as u64).collect() } else { Vec::new() };
        write_lattice_del(w, size, t, &ids, &done, work)
    }

    /// What a merge's `pol` line adds for a child at rank x: its slot, or nothing for
    /// a leaf and past the leaves.
    fn v_term(&self, child: u32, x: u32) -> Option<u32> {
        let c = &self.v_nodes[child as usize];
        if c.leaf_len == 1 || x > c.leaf_len {
            return None;
        }
        c.wires.iter().find(|w| w.0 == x).map(|w| w.2)
    }

    /// The pairs of a merge's slot: every rank of a wire a child shows, and one past
    /// its leaves, that sum to at most t + 1.
    fn v_pairs(&self, s: u32, out: &mut Vec<(u32, u32)>) {
        out.clear();
        let sl = &self.v_slots[s as usize];
        let nd = &self.v_nodes[sl.node as usize];
        let (a, b) = (&self.v_nodes[nd.a as usize], &self.v_nodes[nd.b as usize]);
        let t = sl.t;
        // past a leaf is the same sum as the leaf itself: one line says both
        let past = |c: &VNode| if c.leaf_len == 1 { None } else { Some(c.leaf_len + 1) };
        let xs = a.wires.iter().map(|w| w.0).chain(past(a));
        for x in xs {
            if x > t {
                continue;
            }
            for y in b.wires.iter().map(|w| w.0).chain(past(b)) {
                if x + y <= t + 1 && (self.v_term(nd.a, x).is_some() || self.v_term(nd.b, y).is_some()) {
                    out.push((x, y));
                }
            }
        }
    }

    /// `pbc`, one `pol` per pair, one `rup` per count of a that is not forced, the
    /// last `rup`, `qed`.
    fn v_slot_lines(&self, s: u32) -> u64 {
        let mut pairs = Vec::new();
        self.v_pairs(s, &mut pairs);
        let sl = &self.v_slots[s as usize];
        let nd = &self.v_nodes[sl.node as usize];
        let (a, b) = (&self.v_nodes[nd.a as usize], &self.v_nodes[nd.b as usize]);
        let cases = a.wires.iter().filter(|w| w.0 <= sl.t && (w.0 as i64) > sl.t as i64 - b.leaf_len as i64).count() as u64;
        3 + pairs.len() as u64 + cases
    }

    fn write_v_slot(&mut self, s: u32, w: &mut Writer, work: &mut u64) -> Option<()> {
        let mut pairs = Vec::new();
        self.v_pairs(s, &mut pairs);
        let neg = w.next_id();
        let sl = &self.v_slots[s as usize];
        let nd = &self.v_nodes[sl.node as usize];
        let (a, b) = (&self.v_nodes[nd.a as usize], &self.v_nodes[nd.b as usize]);
        let t = sl.t;
        let mut lines: Vec<String> = Vec::with_capacity(pairs.len() + 2);
        for &(x, y) in &pairs {
            let mut line = format!("pol {}", neg);
            if let Some(q) = self.v_term(nd.a, x) {
                line.push_str(&format!(" {} +", self.v_slots[q as usize].id));
            }
            if let Some(q) = self.v_term(nd.b, y) {
                line.push_str(&format!(" {} +", self.v_slots[q as usize].id));
            }
            line.push_str(" ;");
            lines.push(line);
        }
        let mut clause: Vec<i32> = Vec::new();
        for i in (0..a.wires.len()).rev() {
            let q = a.wires[i].0;
            if q <= t && (q as i64) > t as i64 - b.leaf_len as i64 {
                clause.clear();
                clause.extend(a.wires[..=i].iter().map(|u| -u.1));
                lines.push(rup_line(&clause));
            }
        }
        lines.push("rup >= 1 ;".to_string());
        let n = nd.leaf_len as i64;
        let m = n - t as i64 + 1;
        let mut terms: Vec<(i128, i32)> = Vec::with_capacity(n as usize + 1);
        terms.push((m as i128, sl.lit));
        for &l in &self.v_leaves[nd.leaf_start as usize..(nd.leaf_start + nd.leaf_len) as usize] {
            terms.push((1, -l));
        }
        *work += terms.len() as u64 + lines.len() as u64;
        let id = w.pbc_raw(&terms, m as i128, &lines).ok()?;
        self.v_slots[s as usize].id = id;
        Some(())
    }

    /// A cell's lines and each network's: `pbc`, its `rup` lines, `qed`; then the sum,
    /// the bound on the bits, their sum, and the `del`.
    fn a_lines(&self, net: u32) -> u64 {
        4 + self.a_nets[net as usize].cells.iter().map(|&(c, _)| a_cell_lines(c.kind)).sum::<u64>()
    }

    fn write_adder(&mut self, net: u32, w: &mut Writer, work: &mut u64) -> Option<u64> {
        let n = &self.a_nets[net as usize];
        let mut ids: Vec<u64> = Vec::with_capacity(n.cells.len() + 2);
        let mut expr = String::new();
        for (i, &(c, wt)) in n.cells.iter().enumerate() {
            let id = a_write_cell(c, w, work)?;
            ids.push(id);
            if wt == 0 {
                expr.push_str(&format!("{} ", id));
            } else {
                expr.push_str(&format!("{} {} * ", id, 1u64 << wt));
            }
            if i > 0 {
                expr.push_str("+ ");
            }
        }
        *work += n.cells.len() as u64 + n.bits.len() as u64 + 3;
        let total = w.pol(expr.trim_end()).ok()?;
        ids.push(total);
        let terms: Vec<(i64, i32)> = n.bits.iter().map(|&(b, wt)| (1i64 << wt, -b)).collect();
        let bound = w.rup_terms(&terms, n.k as i64).ok()?;
        ids.push(bound);
        let id = w.pol(&format!("{} {} +", total, bound)).ok()?;
        w.delete(&ids).ok()?;
        Some(id)
    }

    /// The lines of one diagram record: `pbc`, a sum per child, `rup`, `qed`; a dead
    /// node is one `rup`.
    fn d_rec_lines(&self, s: u32) -> u64 {
        let r = &self.d_recs[s as usize];
        if r.c == DEAD {
            return 1;
        }
        3 + (r.lemma != 0) as u64 + r.kids.iter().filter(|&&k| k != NONE && self.d_recs[k as usize].c != DEAD).count() as u64
    }

    /// The lines a diagram's proof still has to write: a trial run of
    /// `write_diagram`, so a `del` is counted where one will be written.
    fn d_lines_left(&mut self, count: u32, work: &mut u64) -> u64 {
        self.epoch = self.epoch.wrapping_add(1);
        let e = self.epoch;
        let (start, len, root) = {
            let c = &self.d_counts[count as usize];
            (c.order_start as usize, c.order_len as usize, c.root)
        };
        *work += len as u64;
        let mut lines = 1u64; // the sum with the unit
        let mut level = NONE;
        let mut del = false;
        for k in start..start + len {
            let s = self.d_order[k];
            if self.d_recs[s as usize].id != 0 {
                continue;
            }
            let here = self.d_recs[s as usize].level;
            if here != level {
                lines += del as u64;
                del = false;
                level = here;
            }
            lines += self.d_rec_lines(s);
            del |= self.d_recs[s as usize].lemma != 0;
            let kids = self.d_recs[s as usize].kids;
            for q in kids {
                if q != NONE {
                    let r = &mut self.d_recs[q as usize];
                    if r.stamp != e {
                        r.stamp = e;
                        r.sim = r.pending;
                    }
                    r.sim = r.sim.saturating_sub(1);
                    del |= r.sim == 0;
                }
            }
        }
        lines += del as u64;
        // the root is a helper once the sum with the unit is written
        let r = &self.d_recs[root as usize];
        let left = if r.stamp == e { r.sim } else { r.pending };
        lines + (left <= 1) as u64
    }

    fn write_d_rec(&mut self, s: u32, w: &mut Writer, work: &mut u64) -> Option<()> {
        let r = &self.d_recs[s as usize];
        if r.c == DEAD {
            let id = w.rup(&[-r.lit]).ok()?;
            *work += 1;
            self.d_recs[s as usize].id = id;
            return Some(());
        }
        let lemma_id = if r.lemma != 0 { w.rup(&[-r.lit, r.lemma]).ok()? } else { 0 };
        let neg = w.next_id();
        let mut lines: Vec<String> = Vec::with_capacity(3);
        for k in r.kids {
            if k != NONE && self.d_recs[k as usize].c != DEAD {
                lines.push(format!("pol {} {} + ;", neg, self.d_recs[k as usize].id));
            }
        }
        lines.push("rup >= 1 ;".to_string());
        let inputs = &self.d_inputs[r.start as usize..(r.start + r.len) as usize];
        let mut terms: Vec<(i128, i32)> = Vec::with_capacity(inputs.len() + 1);
        terms.push((r.c as i128, -r.lit));
        terms.extend(inputs.iter().map(|&y| (1i128, y)));
        *work += terms.len() as u64 + lines.len() as u64;
        let id = w.pbc_raw(&terms, r.c as i128, &lines).ok()?;
        self.d_recs[s as usize].id = id;
        self.d_recs[s as usize].lemma_id = lemma_id;
        Some(())
    }

    /// The records in their order, each level closed by the `del` of what no record
    /// still to come and no count still to be proved needs: a large diagram would
    /// otherwise hold every sum in the checker at once.
    fn write_diagram(&mut self, count: u32, w: &mut Writer, work: &mut u64) -> Option<u64> {
        let (start, len, root, unit_id) = {
            let c = &self.d_counts[count as usize];
            (c.order_start as usize, c.order_len as usize, c.root, c.unit_id)
        };
        let mut level = NONE;
        let mut done: Vec<u64> = Vec::new();
        for k in start..start + len {
            let s = self.d_order[k];
            if self.d_recs[s as usize].id != 0 {
                continue;
            }
            let here = self.d_recs[s as usize].level;
            if here != level {
                if !done.is_empty() {
                    *work += done.len() as u64;
                    w.delete(&done).ok()?;
                    done.clear();
                }
                level = here;
            }
            self.write_d_rec(s, w, work)?;
            if self.d_recs[s as usize].lemma != 0 {
                done.push(self.d_recs[s as usize].lemma_id);
            }
            let kids = self.d_recs[s as usize].kids;
            for q in kids {
                if q != NONE {
                    let r = &mut self.d_recs[q as usize];
                    r.pending = r.pending.saturating_sub(1);
                    if r.pending == 0 {
                        done.push(r.id);
                    }
                }
            }
        }
        if !done.is_empty() {
            *work += done.len() as u64;
            w.delete(&done).ok()?;
        }
        let top = self.d_recs[root as usize].id;
        let c = self.d_recs[root as usize].c;
        *work += 3;
        let id = w.pol(&format!("{} {} {} * +", top, unit_id, c)).ok()?;
        let r = &mut self.d_recs[root as usize];
        r.pending = r.pending.saturating_sub(1);
        if r.pending == 0 {
            w.delete(&[top]).ok()?;
        }
        Some(id)
    }

    /// The number of proof lines count `i`'s proof writes when nothing it needs
    /// has been written yet. It comes from the plan alone and is the same with a
    /// proof file and without one.
    pub fn lines(&self, i: usize) -> u64 {
        self.line_counts.get(i).copied().unwrap_or(0)
    }

    /// The proof terms count `i` plans for: per line, twice the count's length, and
    /// two more. That covers what every form writes on the toys of
    /// `tools/card_read_check.py` and `tests/card_read.rs`, which check it.
    pub fn terms(&self, i: usize) -> u64 {
        self.lines(i).saturating_mul(2 * self.found.get(i).map_or(0, |c| c.lits.len() as u64) + 2)
    }

    /// The proof lines of count `i` from the input clauses. Returns the id of
    /// its constraint "sum of lits >= need", or None, with nothing written,
    /// when `max_lines` does not cover the whole proof. `next_var` is the last
    /// variable number used by the proof; a form that needs fresh variables
    /// takes the next ones (none of the forms read so far does).
    pub fn prove(&mut self, i: usize, w: &mut Writer, next_var: &mut i32, max_lines: u64, work: &mut u64) -> Option<u64> {
        let _ = next_var;
        if i >= self.found.len() {
            return None;
        }
        if self.proved[i] != 0 {
            return Some(self.proved[i]);
        }
        let before = w.written;
        let id = match self.plans[i] {
            Plan::Merge { slot, unit, del } => {
                let left = self.lines_left(slot, work) + unit.is_some() as u64 + del as u64;
                if left > max_lines {
                    return None;
                }
                self.write_tree(slot, w, work)?;
                let base = self.slots[slot as usize].id;
                let id = match unit {
                    Some((clause, mult)) => {
                        *work += 3;
                        w.pol(&format!("{} {} {} * +", base, clause, mult)).ok()?
                    }
                    None => base,
                };
                self.drop_helpers(slot, w, work)?;
                id
            }
            Plan::OneWay { walk } => {
                let (start, len, k, unit_id) = {
                    let wk = &self.walks[walk as usize];
                    (wk.reg_start as usize, wk.reg_len as usize, wk.k, wk.unit_id)
                };
                // the sum with the unit, and the `del` line
                let mut left = 2u64;
                *work += len as u64;
                for g in start..start + len {
                    if self.tslots[self.regs[g].tslot as usize] == 0 {
                        left += self.reg_lines(g);
                    }
                }
                if left > max_lines {
                    return None;
                }
                // the registers were made from the top down
                for g in (start..start + len).rev() {
                    if self.tslots[self.regs[g].tslot as usize] == 0 {
                        self.write_reg(g, walk as usize, w, work)?;
                    }
                }
                let top = self.tslots[self.regs[start].tslot as usize];
                *work += 3;
                let id = w.pol(&format!("{} {} {} * +", top, unit_id, k)).ok()?;
                // what no count still to be proved needs goes
                let mut ids: Vec<u64> = Vec::new();
                *work += len as u64;
                for g in start..start + len {
                    let t = self.regs[g].tslot as usize;
                    self.tslot_uses[t] = self.tslot_uses[t].saturating_sub(1);
                    if self.tslot_uses[t] == 0 {
                        ids.push(self.tslots[t]);
                    }
                }
                if !ids.is_empty() {
                    w.delete(&ids).ok()?;
                }
                id
            }
            Plan::Diagram { count } => {
                if self.d_lines_left(count, work) > max_lines {
                    return None;
                }
                self.write_diagram(count, w, work)?
            }
            Plan::Adder { net } => {
                if self.a_lines(net) > max_lines {
                    return None;
                }
                self.write_adder(net, w, work)?
            }
            Plan::NetSum { net } => {
                if self.net_sum_lines(net) > max_lines {
                    return None;
                }
                self.write_net_sum(net, w, work)?
            }
            Plan::NetSubsets { net } => {
                if self.net_subset_lines(net) > max_lines {
                    return None;
                }
                self.write_net_subsets(net, w, work)?
            }
            Plan::NetTree { slot, unit, mult, order_start, order_len } => {
                let (a, n) = (order_start as usize, order_len as usize);
                let mut left = 2u64;
                for k in a..a + n {
                    left += self.v_slot_lines(self.v_order[k]);
                }
                *work += n as u64 + left;
                if left > max_lines {
                    return None;
                }
                let mut made: Vec<u64> = Vec::with_capacity(n);
                for k in a..a + n {
                    let q = self.v_order[k];
                    self.write_v_slot(q, w, work)?;
                    made.push(self.v_slots[q as usize].id);
                }
                *work += 3 + n as u64;
                let top = self.v_slots[slot as usize].id;
                let id = w.pol(&format!("{} {} {} * +", top, unit, mult)).ok()?;
                w.delete(&made).ok()?;
                id
            }
            Plan::Subset { family } => {
                let f = &self.families[family as usize];
                if f.id != 0 {
                    f.id
                } else {
                    let (t, m) = (f.lits.len(), f.m as usize);
                    if lattice_lines(t, m) > max_lines {
                        return None;
                    }
                    let ids = std::mem::take(&mut self.families[family as usize].ids);
                    let id = write_lattice(w, t, m, &ids, work);
                    self.families[family as usize].ids = ids;
                    let id = id?;
                    self.families[family as usize].id = id;
                    id
                }
            }
        };
        self.proved[i] = id;
        if self.hook {
            println!("c cardread proved {} {} {} written={} planned={}", i, id, self.lines(i), w.written - before, self.terms(i));
        }
        Some(id)
    }
}

/// The money: work is paid for before it is done.
struct Clock<'a> {
    ticks: &'a mut u64,
    budget: u64,
    stop: &'a AtomicBool,
    last: u64,
}

impl Clock<'_> {
    /// Pay for `n` units of work. None when the limit does not cover them or the
    /// run was told to stop; nothing is charged for work that is not done.
    #[inline]
    fn pay(&mut self, n: u64) -> Option<()> {
        let t = self.ticks.saturating_add(n);
        if t > self.budget {
            return None;
        }
        *self.ticks = t;
        if t - self.last >= 1024 {
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

/// The clause lists and the lookup table.
struct Ix<'f> {
    f: &'f Formula,
    /// Per literal: the other literal of each binary clause that holds it.
    off2: Vec<u32>,
    bin: Vec<i32>,
    /// Per literal: the other two of each ternary clause that holds it.
    off3: Vec<u32>,
    tri: Vec<[i32; 2]>,
    /// Per literal: the clauses of length 4 to MAX_LEN that hold it.
    offl: Vec<u32>,
    long: Vec<u32>,
    /// Per literal: 0 no unit, 1 a unit with a repeated literal only, 2 a clean one.
    unit: Vec<u8>,
    /// (the hash's low bits, clause index + 1), open addressing.
    table: Vec<(u32, u32)>,
    mask: usize,
    /// The clause is the first copy of its literals, the clean one if any is.
    uniq: Vec<bool>,
    /// The clauses longer than the table takes.
    longer: Vec<u32>,
}

impl<'f> Ix<'f> {
    fn build(f: &'f Formula, ck: &mut Clock) -> Option<Ix<'f>> {
        let nl = 2 * (f.vars.max(0) as usize + 1);
        let m = f.len();
        let scan = m as u64 + f.lits.len() as u64;
        ck.pay(m as u64)?;
        let mut short = 0usize;
        let mut longer: Vec<u32> = Vec::new();
        for i in 0..m {
            ck.poll(i)?;
            let len = (f.start[i + 1] - f.start[i]) as usize;
            if (1..=MAX_LEN).contains(&len) {
                short += 1;
            } else if len > MAX_LEN && !f.taut[i] {
                longer.push(i as u32);
            }
        }
        let cap = (2 * short + 16).next_power_of_two();
        // the whole index or none of it: a formula too large for the limit costs
        // one look at its clause sizes
        let index = cap as u64 + 10 * nl as u64 + m as u64 + 4 * scan + f.lits.len() as u64;
        if ck.ticks.saturating_add(index) > ck.budget {
            return None;
        }
        // the tables, whole, before they are made
        ck.pay(cap as u64 + 4 * nl as u64 + m as u64)?;
        let mut ix = Ix {
            f,
            off2: vec![0; nl + 1],
            bin: Vec::new(),
            off3: vec![0; nl + 1],
            tri: Vec::new(),
            offl: vec![0; nl + 1],
            long: Vec::new(),
            unit: vec![0; nl],
            table: vec![(0, 0); cap],
            mask: cap - 1,
            uniq: vec![false; m],
            longer,
        };
        // the clean clauses go in first, so a lookup finds the clean copy if
        // there is one (a clause with a repeated literal has another coefficient)
        ck.pay(2 * scan)?;
        for pass in 0..2 {
            for i in 0..m {
                ck.poll(i)?;
                let c = f.clause(i);
                if f.taut[i] || c.is_empty() || c.len() > MAX_LEN || f.dup[i] != (pass == 1) {
                    continue;
                }
                if ix.insert(i) {
                    ix.uniq[i] = true;
                    if c.len() == 1 {
                        ix.unit[li(c[0])] = if f.dup[i] { 1 } else { 2 };
                    }
                }
            }
        }
        // the lists: count, offsets, fill, sort
        ck.pay(scan + 3 * nl as u64)?;
        let (mut n2, mut n3, mut nlong) = (0usize, 0usize, 0usize);
        for i in 0..m {
            ck.poll(i)?;
            if !ix.uniq[i] {
                continue;
            }
            let c = f.clause(i);
            match c.len() {
                2 => {
                    n2 += 2;
                    for &l in c {
                        ix.off2[li(l) + 1] += 1;
                    }
                }
                3 => {
                    n3 += 3;
                    for &l in c {
                        ix.off3[li(l) + 1] += 1;
                    }
                }
                4..=MAX_LEN => {
                    nlong += c.len();
                    for &l in c {
                        ix.offl[li(l) + 1] += 1;
                    }
                }
                _ => {}
            }
        }
        for i in 0..nl {
            ck.poll(i)?;
            ix.off2[i + 1] += ix.off2[i];
            ix.off3[i + 1] += ix.off3[i];
            ix.offl[i + 1] += ix.offl[i];
        }
        ck.pay(scan + 3 * nl as u64 + (n2 + n3 + nlong) as u64)?;
        ix.bin = vec![0; n2];
        ix.tri = vec![[0, 0]; n3];
        ix.long = vec![0; nlong];
        let mut fill2 = ix.off2.clone();
        let mut fill3 = ix.off3.clone();
        let mut filll = ix.offl.clone();
        for i in 0..m {
            ck.poll(i)?;
            if !ix.uniq[i] {
                continue;
            }
            let c = f.clause(i);
            match c.len() {
                2 => {
                    for k in 0..2 {
                        let p = &mut fill2[li(c[k])];
                        ix.bin[*p as usize] = c[1 - k];
                        *p += 1;
                    }
                }
                3 => {
                    for k in 0..3 {
                        let p = &mut fill3[li(c[k])];
                        ix.tri[*p as usize] = [c[(k + 1) % 3], c[(k + 2) % 3]];
                        *p += 1;
                    }
                }
                4..=MAX_LEN => {
                    for &l in c {
                        let p = &mut filll[li(l)];
                        ix.long[*p as usize] = i as u32;
                        *p += 1;
                    }
                }
                _ => {}
            }
        }
        // each list is sorted once its cost is paid
        for i in 0..nl {
            let (lo, hi) = (ix.off2[i] as usize, ix.off2[i + 1] as usize);
            if hi - lo > 1 {
                let d = (hi - lo) as u64;
                ck.pay(d * (64 - d.leading_zeros() as u64))?;
                ix.bin[lo..hi].sort_unstable();
            }
            ck.poll(i)?;
        }
        Some(ix)
    }

    fn insert(&mut self, i: usize) -> bool {
        let c = self.f.clause(i);
        let h = set_hash(c);
        let tag = (h >> 32) as u32;
        let mut p = h as usize & self.mask;
        loop {
            let (t, idx) = self.table[p];
            if idx == 0 {
                self.table[p] = (tag, i as u32 + 1);
                return true;
            }
            if t == tag && same_set(c, self.f.clause(idx as usize - 1)) {
                return false;
            }
            p = (p + 1) & self.mask;
        }
    }

    #[inline]
    fn bins(&self, l: i32) -> &[i32] {
        let i = li(l);
        &self.bin[self.off2[i] as usize..self.off2[i + 1] as usize]
    }

    #[inline]
    fn tris(&self, l: i32) -> &[[i32; 2]] {
        let i = li(l);
        &self.tri[self.off3[i] as usize..self.off3[i + 1] as usize]
    }

    #[inline]
    fn longs(&self, l: i32) -> &[u32] {
        let i = li(l);
        &self.long[self.offl[i] as usize..self.offl[i + 1] as usize]
    }

    /// How many short clauses hold a literal.
    #[inline]
    fn degree(&self, l: i32) -> usize {
        self.bins(l).len() + self.tris(l).len() + self.longs(l).len()
    }
}

/// A stored clause (no repeated literals) holds exactly the literals asked for.
/// Both directions, so that a request with a literal twice never matches.
fn same_set(a: &[i32], b: &[i32]) -> bool {
    a.len() == b.len() && a.iter().all(|l| b.contains(l)) && b.iter().all(|l| a.contains(l))
}

struct Reader<'a> {
    ix: Ix<'a>,
    ck: Clock<'a>,
    rd: Reading,
    /// One mark per variable.
    mark: Vec<u32>,
    mark_id: u32,
    singles: HashMap<i32, u32>,
    seen_found: HashSet<(u64, i32, u32)>,
    tslot_of: HashMap<(i32, u32, u32, u64), u32>,
    buf: Vec<u32>,
    /// Per literal, a list of the nodes with it as their first output.
    first_head: Vec<u32>,
    first_next: Vec<u32>,
    /// Per literal, a list of (node, place, next) with it as a later output.
    later_head: Vec<u32>,
    later: Vec<(u32, u32, u32)>,
    /// Per literal: a register of a chain that counts only to one.
    clause_read: Vec<bool>,
    /// The slots of the last count checked.
    visited: Vec<u32>,
    /// What the diagram reading has made of each literal and variable: the tables
    /// are paid for once, and a stamp says which reading an entry is from.
    d_at: Vec<u32>,
    d_at_stamp: Vec<u32>,
    d_level: Vec<u32>,
    d_level_stamp: Vec<u32>,
    d_known: Vec<u32>,
    d_epoch: u32,
    /// A literal asserted as a diagram's root: no walk starts there.
    d_root: Vec<bool>,
    /// A node of a diagram that gave a count.
    d_said: Vec<bool>,
    /// One proof statement per node, count and inputs: two counts share them.
    d_rec_of: HashMap<(i32, u32, u32, u64), u32>,
    dn: Vec<DNode>,
    d_came: Vec<(i32, u32)>,
    d_big_head: Vec<u32>,
    d_big: Vec<(u32, u32)>,
    /// Per literal: the gate that makes that wire, and whether it was looked for.
    gate_of: Vec<u32>,
    gate_seen: Vec<bool>,
    gates: Vec<Gate>,
    /// A half comparator may read two wires no gate makes: on the second reading.
    half_on_inputs: bool,
    half_turned_away: bool,
    /// The units a merge tree has said what they mean: not read again as networks.
    tree_units: Vec<i32>,
}

impl<'a> Reader<'a> {
    fn new(ix: Ix<'a>, ck: Clock<'a>, rd: Reading, nv: usize) -> Reader<'a> {
        Reader {
            ix,
            ck,
            rd,
            mark: vec![0; nv],
            mark_id: 0,
            singles: HashMap::new(),
            seen_found: HashSet::new(),
            tslot_of: HashMap::new(),
            buf: Vec::new(),
            first_head: Vec::new(),
            first_next: Vec::new(),
            later_head: Vec::new(),
            later: Vec::new(),
            clause_read: Vec::new(),
            visited: Vec::new(),
            d_at: Vec::new(),
            d_at_stamp: Vec::new(),
            d_level: Vec::new(),
            d_level_stamp: Vec::new(),
            d_known: Vec::new(),
            d_epoch: 0,
            d_root: Vec::new(),
            d_said: Vec::new(),
            d_rec_of: HashMap::new(),
            dn: Vec::new(),
            d_came: Vec::new(),
            d_big_head: Vec::new(),
            d_big: Vec::new(),
            gate_of: Vec::new(),
            gate_seen: Vec::new(),
            gates: Vec::new(),
            half_on_inputs: false,
            half_turned_away: false,
            tree_units: Vec::new(),
        }
    }

    #[inline]
    fn next_mark(&mut self) {
        self.mark_id = self.mark_id.wrapping_add(1);
    }

    #[inline]
    fn set_mark(&mut self, l: i32) {
        self.mark[var(l)] = self.mark_id;
    }

    #[inline]
    fn marked(&self, l: i32) -> bool {
        self.mark[var(l)] == self.mark_id
    }

    /// The clause with exactly these literals, if there is one.
    fn find(&mut self, lits: &[i32]) -> Option<Option<u32>> {
        self.ck.pay(2 + lits.len() as u64)?;
        let h = set_hash(lits);
        let tag = (h >> 32) as u32;
        let mut p = h as usize & self.ix.mask;
        loop {
            let (t, idx) = self.ix.table[p];
            if idx == 0 {
                return Some(None);
            }
            if t == tag && same_set(lits, self.ix.f.clause(idx as usize - 1)) {
                return Some(Some(idx - 1));
            }
            self.ck.pay(1)?;
            p = (p + 1) & self.ix.mask;
        }
    }

    /// A binary clause (p v q).
    fn has_bin(&mut self, p: i32, q: i32) -> Option<bool> {
        let (a, b) = if self.ix.bins(p).len() <= self.ix.bins(q).len() { (p, q) } else { (q, p) };
        let list = self.ix.bins(a);
        self.ck.pay(1 + (usize::BITS - list.len().leading_zeros()) as u64)?;
        Some(self.ix.bins(a).binary_search(&b).is_ok())
    }

    /// The literals r with a clause (-u v -v v r).
    fn out(&mut self, u: i32, v: i32, outs: &mut Vec<i32>) -> Option<()> {
        outs.clear();
        let (x, y) = if self.ix.tris(-u).len() <= self.ix.tris(-v).len() { (-u, -v) } else { (-v, -u) };
        self.ck.pay(1 + self.ix.tris(x).len() as u64)?;
        for &[p, q] in self.ix.tris(x) {
            if p == y {
                outs.push(q);
            } else if q == y {
                outs.push(p);
            }
        }
        Some(())
    }

    /// The candidate that pairs with most of what `a1` pairs with: a shared leaf
    /// can drag in partners of another constraint.
    fn best_partner(&mut self, cands: &[i32], a1: i32) -> Option<i32> {
        let list = self.ix.tris(-a1);
        self.ck.pay(1 + 2 * list.len() as u64)?;
        let mut mine: Vec<i32> = list.iter().flat_map(|&[p, q]| [-p, -q]).collect();
        mine.sort_unstable();
        let mut best = (0usize, cands[0]);
        let mut first = true;
        for &u in cands {
            let list = self.ix.tris(-u);
            self.ck.pay(1 + 2 * list.len() as u64)?;
            let score = list.iter().flat_map(|&[p, q]| [-p, -q]).filter(|x| mine.binary_search(x).is_ok()).count();
            if first || score > best.0 || (score == best.0 && u < best.1) {
                best = (score, u);
                first = false;
            }
        }
        Some(best.1)
    }

    /// One child read along the grid: a_1, then a literal implying the last
    /// output that has a clause with b_1, whose output is the next one. Returns
    /// the child's literals and the outputs along the way.
    fn chain(&mut self, a1: i32, b1: i32, r1: i32, r2: i32, a: &mut Vec<i32>, r: &mut Vec<i32>) -> Option<()> {
        a.clear();
        r.clear();
        a.push(a1);
        r.push(r1);
        r.push(r2);
        self.next_mark();
        for l in [a1, b1, r1, r2] {
            self.set_mark(l);
        }
        let mut cands: Vec<i32> = Vec::new();
        let mut caps: Vec<i32> = Vec::new();
        let mut outs: Vec<i32> = Vec::new();
        loop {
            let rn = *r.last().unwrap();
            let plen = self.ix.bins(rn).len();
            self.ck.pay(1 + plen as u64)?;
            if plen > MAX_DEG {
                break;
            }
            cands.clear();
            caps.clear();
            for p in 0..plen {
                let u = -self.ix.bins(rn)[p];
                if self.marked(u) {
                    continue;
                }
                self.out(u, b1, &mut outs)?;
                if !outs.is_empty() {
                    cands.push(u);
                } else if self.has_bin(-u, -b1)? {
                    caps.push(u); // the overflow (-a_i v -b_1): a_i is the last one
                }
            }
            if cands.is_empty() {
                if caps.len() == 1 {
                    a.push(caps[0]);
                }
                break;
            }
            let u = if cands.len() == 1 { cands[0] } else { self.best_partner(&cands, a1)? };
            self.out(u, b1, &mut outs)?;
            let Some(nxt) = closest(outs.iter().copied().filter(|&o| !self.marked(o)), rn) else { break };
            a.push(u);
            r.push(nxt);
            self.set_mark(u);
            self.set_mark(nxt);
        }
        Some(())
    }

    /// A merge node from its first inputs and first two outputs. The children's
    /// lists are hints for the linking; what the proof relies on is checked later.
    fn read_merge(&mut self, a1: i32, b1: i32, r1: i32, r2: i32) -> Option<()> {
        let (mut ca, mut ra, mut cb, mut rb) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        self.chain(a1, b1, r1, r2, &mut ca, &mut ra)?;
        self.chain(b1, a1, r1, r2, &mut cb, &mut rb)?;
        let m = ra.len().min(rb.len());
        self.ck.pay(1 + m as u64 + 2 * (ca.len() + cb.len() + ra.len().max(rb.len())) as u64)?;
        if ra[..m] != rb[..m] {
            return Some(());
        }
        let mut r = if ra.len() >= rb.len() { ra } else { rb };
        self.next_mark();
        for &l in ca.iter().chain(cb.iter()).chain(r.iter()) {
            if self.marked(l) {
                return Some(());
            }
            self.set_mark(l);
        }
        // the grid: every sum up to the largest that is complete; the outputs
        // past the walks come from the first pair that reaches them
        let (p, q) = (ca.len(), cb.len());
        let mut outs: Vec<i32> = Vec::new();
        let mut c = r.len();
        'sums: for s in 2..=p + q {
            let first = s.saturating_sub(q).max(1);
            let last = p.min(s - 1);
            for i in first..=last {
                let j = s - i;
                if s <= r.len() {
                    if self.find(&[-ca[i - 1], -cb[j - 1], r[s - 1]])?.is_none() {
                        c = s - 1;
                        break 'sums;
                    }
                } else {
                    self.out(ca[i - 1], cb[j - 1], &mut outs)?;
                    let Some(new) = closest(outs.iter().copied().filter(|&o| !self.marked(o)), r[s - 2]) else {
                        c = s - 1;
                        break 'sums;
                    };
                    r.push(new);
                    self.set_mark(new);
                    c = r.len();
                }
            }
        }
        r.truncate(c);
        ca.truncate(c.max(1));
        cb.truncate(c.max(1));
        // an overflow clause among the pairs at c + 1
        let mut overflow = false;
        for i in 1..=ca.len() {
            if c + 1 > i && c + 1 - i <= cb.len() && self.has_bin(-ca[i - 1], -cb[c - i])? {
                overflow = true;
                break;
            }
        }
        let mut nd = new_node();
        nd.overflow = overflow;
        nd.cap_c = r.len() as u32;
        nd.va = ca;
        nd.vb = cb;
        nd.r = r;
        self.rd.nodes.push(nd);
        Some(())
    }

    /// Merge nodes from a literal r_1: two literals u and v that imply it, with a
    /// clause (-u v -v v r_2). With only u and v implying r_1 and the overflow
    /// (-u v -v) instead, a node of one output, a rung of an at-most-one ladder;
    /// the condition keeps out the triangles of a pairwise group.
    fn seeds_from_first(&mut self, r1: i32) -> Option<()> {
        let np = self.ix.bins(r1).len();
        self.ck.pay(1)?;
        if !(2..=MAX_PRE).contains(&np) {
            return Some(());
        }
        let mut outs: Vec<i32> = Vec::new();
        for x in 0..np {
            for y in x + 1..np {
                let (u, v) = (-self.ix.bins(r1)[x], -self.ix.bins(r1)[y]);
                if var(u) == var(v) || var(u) == var(r1) || var(v) == var(r1) {
                    continue;
                }
                self.out(u, v, &mut outs)?;
                if outs.is_empty() {
                    if np == 2 && self.has_bin(-u, -v)? {
                        self.ck.pay(8)?;
                        let mut nd = new_node();
                        nd.overflow = true;
                        nd.cap_c = 1;
                        nd.va = vec![u];
                        nd.vb = vec![v];
                        nd.r = vec![r1];
                        self.rd.nodes.push(nd);
                    }
                    continue;
                }
                outs.sort_unstable();
                for k in 0..outs.len() {
                    let r2 = outs[k];
                    if var(r2) != var(u) && var(r2) != var(v) && var(r2) != var(r1) {
                        self.read_merge(u, v, r1, r2)?;
                    }
                }
            }
        }
        Some(())
    }

    /// A direct node from an output o_2 that every pair of its inputs implies.
    fn direct_from_second(&mut self, o2: i32) -> Option<()> {
        let np = self.ix.tris(o2).len();
        if !(3..=MAX_T * (MAX_T - 1) / 2).contains(&np) {
            return Some(());
        }
        self.ck.pay(4 * np as u64)?;
        let mut xs: Vec<i32> = self.ix.tris(o2).iter().flat_map(|&[p, q]| [-p, -q]).collect();
        xs.sort_unstable();
        xs.dedup();
        let t = xs.len();
        if !(3..=MAX_T).contains(&t) || np != t * (t - 1) / 2 {
            return Some(());
        }
        // the literals all of them imply
        let first = self.ix.bins(-xs[0]);
        if first.is_empty() || first.len() > MAX_DEG {
            return Some(());
        }
        self.ck.pay(first.len() as u64 * (t as u64 + 1))?;
        let mut common: Vec<i32> = first.to_vec();
        for &x in &xs[1..] {
            let list = self.ix.bins(-x);
            common.retain(|o| list.binary_search(o).is_ok());
        }
        self.next_mark();
        for &x in &xs {
            if self.marked(x) {
                return Some(());
            }
            self.set_mark(x);
        }
        if self.marked(o2) {
            return Some(());
        }
        common.retain(|&o| !self.marked(o) && var(o) != var(o2));
        // several trees can read one group of inputs: the nearest numbers first
        common.sort_unstable_by_key(|&o| ((o.unsigned_abs() as i64 - o2.unsigned_abs() as i64).abs(), o));
        for &o1 in common.iter().take(4) {
            let r = self.direct_outputs(&xs, o1, o2)?;
            let mut nd = new_node();
            nd.xs = xs.clone();
            nd.cap_c = r.len() as u32;
            nd.r = r;
            self.rd.nodes.push(nd);
        }
        Some(())
    }

    /// o_3, o_4, ...: a literal with a clause (-S v o_m) for every m-subset S.
    fn direct_outputs(&mut self, xs: &[i32], o1: i32, o2: i32) -> Option<Vec<i32>> {
        let t = xs.len();
        let mut r = vec![o1, o2];
        let mut query: Vec<i32> = Vec::with_capacity(MAX_LEN);
        for m in 3..=t.min(MAX_DIRECT_OUT) {
            let rarest = xs[..m].iter().copied().min_by_key(|&x| (self.ix.longs(-x).len(), x)).unwrap();
            let nlong = self.ix.longs(-rarest).len();
            self.ck.pay(1 + nlong as u64)?;
            let mut cands: Vec<i32> = Vec::new();
            for k in 0..nlong {
                let c = self.ix.f.clause(self.ix.longs(-rarest)[k] as usize);
                if c.len() != m + 1 {
                    continue;
                }
                self.ck.pay((m * m) as u64)?;
                if !xs[..m].iter().all(|&x| c.contains(&-x)) {
                    continue;
                }
                if let Some(&o) = c.iter().find(|&&l| !xs[..m].contains(&-l)) {
                    if !self.marked(o) && !r.contains(&o) {
                        cands.push(o);
                    }
                }
            }
            let prev = *r.last().unwrap();
            cands.sort_unstable_by_key(|&o| ((o.unsigned_abs() as i64 - prev.unsigned_abs() as i64).abs(), o));
            let mut got = 0;
            'cands: for &o in &cands {
                let mut mask = (1u32 << m) - 1;
                while mask < (1u32 << t) {
                    query.clear();
                    let mut rest = mask;
                    while rest != 0 {
                        let b = rest.trailing_zeros() as usize;
                        rest &= rest - 1;
                        query.push(-xs[b]);
                    }
                    query.push(o);
                    if self.find(&query)?.is_none() {
                        continue 'cands;
                    }
                    mask = next_mask(mask);
                }
                got = o;
                break;
            }
            if got == 0 {
                break;
            }
            r.push(got);
        }
        Some(r)
    }

    /// What a child's list of literals is: a node whose outputs start with it, a
    /// first column (one literal implies it, the rest cannot count), or a leaf.
    fn kid_of(&mut self, parent: usize, v: &[i32]) -> Option<Kid> {
        let head = v[0];
        let mut best = NONE;
        let mut best_score = (0usize, 0usize, 0usize, false);
        let mut at = self.first_head[li(head)];
        while at != NONE {
            let ni = at;
            at = self.first_next[ni as usize];
            if ni as usize == parent {
                continue;
            }
            let n = self.rd.nodes[ni as usize].r.len().min(v.len());
            self.ck.pay(1 + n as u64)?;
            if self.rd.nodes[ni as usize].r[..n] != v[..n] {
                continue;
            }
            // how far its outputs past the list go on into the parent's outputs:
            // that tells two trees over the same children apart
            let mut goes_on = 0;
            let width = self.rd.nodes[ni as usize].r.len();
            for k in n..width {
                let Some(&above) = self.rd.nodes[parent].r.get(k) else { break };
                let own = self.rd.nodes[ni as usize].r[k];
                if !self.has_bin(-own, above)? {
                    break;
                }
                goes_on += 1;
            }
            let score = (n, goes_on, width, !self.rd.nodes[ni as usize].xs.is_empty());
            if best == NONE || score > best_score {
                best = ni;
                best_score = score;
            }
        }
        if best != NONE {
            // a child read as one literal is that node only when its second
            // output goes on into the parent's second output
            if v.len() >= 2 || self.rd.nodes[best as usize].r.len() == 1 || best_score.1 >= 1 {
                return Some(Kid::Node(best));
            }
        }
        // a first column: one literal implies its first register. For a child of
        // one literal that needs care: in a two-way encoding every leaf is implied
        // by the parent's top register.
        let pre = self.ix.bins(head);
        if pre.len() == 1 && (v.len() >= 2 || (self.ix.degree(head) + self.ix.degree(-head) <= 4 && !self.rd.nodes[parent].r.contains(&-pre[0]))) {
            let x = -pre[0];
            if let Some(&ni) = self.singles.get(&head) {
                return Some(Kid::Node(ni));
            }
            self.ck.pay(8)?;
            let ni = self.rd.nodes.len() as u32;
            let mut nd = new_node();
            nd.xs = vec![x];
            nd.r = vec![head];
            nd.cap_c = 1;
            self.rd.nodes.push(nd);
            self.singles.insert(head, ni);
            return Some(Kid::Node(ni));
        }
        Some(if v.len() == 1 { Kid::Leaf(head) } else { Kid::Open })
    }

    /// The lists a node is looked up by: per literal, the nodes with it as their
    /// first output, and the nodes with it as a later one.
    fn make_lookups(&mut self) -> Option<()> {
        let count = self.rd.nodes.len();
        let nl = self.ix.off2.len() - 1;
        let total: usize = self.rd.nodes.iter().map(|n| n.r.len()).sum();
        self.ck.pay(2 * nl as u64 + 2 * count as u64 + total as u64)?;
        self.first_head = vec![NONE; nl];
        self.later_head = vec![NONE; nl];
        self.first_next = vec![NONE; count];
        self.later = Vec::with_capacity(total);
        // from the last to the first, so a list runs in node order
        for ni in (0..count).rev() {
            let nd = &self.rd.nodes[ni];
            let h = li(nd.r[0]);
            self.first_next[ni] = self.first_head[h];
            self.first_head[h] = ni as u32;
            if nd.xs.is_empty() {
                for s in 1..nd.r.len() {
                    let h = li(nd.r[s]);
                    self.later.push((ni as u32, s as u32, self.later_head[h]));
                    self.later_head[h] = self.later.len() as u32 - 1;
                }
            }
        }
        Some(())
    }

    /// A node's clauses also read as a smaller node on every i-th count: a_i and
    /// b_i give r_i and r_(2i), a_(2i) and b_i give r_(3i), and so on. Such a
    /// reading is true but says nothing new, and its leaves would be registers:
    /// its first inputs are the i-th of another node's, with r_i as first output.
    fn every_other_count(&mut self, ni: usize) -> Option<bool> {
        let (a1, b1) = (self.rd.nodes[ni].va[0], self.rd.nodes[ni].vb[0]);
        let mut at = self.later_head[li(self.rd.nodes[ni].r[0])];
        while at != NONE {
            let (other, s, next) = self.later[at as usize];
            at = next;
            self.ck.pay(1)?;
            let (x, y) = (self.rd.nodes[other as usize].va.get(s as usize).copied(), self.rd.nodes[other as usize].vb.get(s as usize).copied());
            if (x == Some(a1) && y == Some(b1)) || (x == Some(b1) && y == Some(a1)) {
                return Some(true);
            }
        }
        Some(false)
    }

    /// Find a node's children. False when it is no node of its own.
    fn link(&mut self, ni: usize) -> Option<bool> {
        if self.every_other_count(ni)? {
            return Some(false);
        }
        let va = std::mem::take(&mut self.rd.nodes[ni].va);
        let vb = std::mem::take(&mut self.rd.nodes[ni].vb);
        let a = self.kid_of(ni, &va)?;
        let b = self.kid_of(ni, &vb)?;
        let nd = &mut self.rd.nodes[ni];
        nd.va = va;
        nd.vb = vb;
        nd.a = a;
        nd.b = b;
        Some(true)
    }

    /// A node's leaves in one range of the arena, sharing the children's ranges
    /// whenever they lie next to each other.
    fn merge_ranges(&mut self, ni: usize) -> Option<()> {
        let range = |rd: &Reading, k: Kid| -> (u32, u32) {
            match k {
                Kid::Node(c) => (rd.nodes[c as usize].leaf_start, rd.nodes[c as usize].leaf_len),
                _ => (0, 0),
            }
        };
        let (a, b) = (self.rd.nodes[ni].a, self.rd.nodes[ni].b);
        let (sa, la) = range(&self.rd, a);
        let (sb, lb) = range(&self.rd, b);
        let end = self.rd.leaves.len() as u32;
        let mut cost = 2u64;
        let (start, len) = match (a, b) {
            (Kid::Node(_), Kid::Node(_)) => {
                if sa + la == sb {
                    (sa, la + lb)
                } else if sb + lb == sa {
                    (sb, la + lb)
                } else if sa + la == end {
                    cost += lb as u64;
                    self.ck.pay(cost)?;
                    self.rd.leaves.extend_from_within(sb as usize..(sb + lb) as usize);
                    (sa, la + lb)
                } else if sb + lb == end {
                    cost += la as u64;
                    self.ck.pay(cost)?;
                    self.rd.leaves.extend_from_within(sa as usize..(sa + la) as usize);
                    (sb, la + lb)
                } else {
                    cost += (la + lb) as u64;
                    self.ck.pay(cost)?;
                    self.rd.leaves.extend_from_within(sa as usize..(sa + la) as usize);
                    self.rd.leaves.extend_from_within(sb as usize..(sb + lb) as usize);
                    (end, la + lb)
                }
            }
            (Kid::Node(_), Kid::Leaf(l)) | (Kid::Leaf(l), Kid::Node(_)) => {
                let (s, n) = if matches!(a, Kid::Node(_)) { (sa, la) } else { (sb, lb) };
                if s + n == end {
                    self.rd.leaves.push(l);
                    (s, n + 1)
                } else {
                    cost += n as u64;
                    self.ck.pay(cost)?;
                    self.rd.leaves.extend_from_within(s as usize..(s + n) as usize);
                    self.rd.leaves.push(l);
                    (end, n + 1)
                }
            }
            (Kid::Leaf(x), Kid::Leaf(y)) => {
                self.rd.leaves.push(x);
                self.rd.leaves.push(y);
                (end, 2)
            }
            _ => return Some(()),
        };
        self.ck.pay(2)?;
        let nd = &mut self.rd.nodes[ni];
        nd.leaf_start = start;
        nd.leaf_len = len;
        Some(())
    }

    fn make_slots(&mut self, ni: usize) -> Option<()> {
        let w = self.rd.nodes[ni].r.len() as u32;
        self.ck.pay(w as u64 + 1)?;
        let base = self.rd.slots.len() as u32;
        self.rd.nodes[ni].slot = base;
        for t in 1..=w {
            self.rd.slots.push(Slot { node: ni as u32, t, cap: false, state: 0, stamp: 0, aux: 0, id: 0, uses: 0, keep: false });
        }
        let c = self.rd.nodes[ni].cap_c;
        self.rd.slots.push(Slot { node: ni as u32, t: c + 1, cap: true, state: 0, stamp: 0, aux: 0, id: 0, uses: 0, keep: false });
        Some(())
    }

    /// Everything a node needs from below: its leaves and its proof slots, the
    /// children first, with no recursion. A cycle or a vector no node defines
    /// makes the node bad.
    fn finish(&mut self, root: usize) -> Option<()> {
        let mut stack = vec![root as u32];
        while let Some(&x) = stack.last() {
            self.ck.pay(2)?;
            let x = x as usize;
            match self.rd.nodes[x].state {
                DONE | BAD => {
                    stack.pop();
                }
                NEW => {
                    if !self.rd.nodes[x].xs.is_empty() {
                        let xs = std::mem::take(&mut self.rd.nodes[x].xs);
                        self.ck.pay(xs.len() as u64)?;
                        let start = self.rd.leaves.len() as u32;
                        self.rd.leaves.extend_from_slice(&xs);
                        let nd = &mut self.rd.nodes[x];
                        nd.leaf_start = start;
                        nd.leaf_len = xs.len() as u32;
                        nd.chain = xs.len() == 1;
                        nd.xs = xs;
                        self.make_slots(x)?;
                        self.rd.nodes[x].state = DONE;
                        stack.pop();
                        continue;
                    }
                    self.rd.nodes[x].state = ON_PATH;
                    if !self.link(x)? {
                        self.rd.nodes[x].state = BAD;
                        stack.pop();
                        continue;
                    }
                    let (a, b) = (self.rd.nodes[x].a, self.rd.nodes[x].b);
                    if a == Kid::Open || b == Kid::Open {
                        self.rd.nodes[x].state = BAD;
                        stack.pop();
                        continue;
                    }
                    let mut cyclic = false;
                    for k in [b, a] {
                        if let Kid::Node(c) = k {
                            match self.rd.nodes[c as usize].state {
                                NEW => stack.push(c),
                                ON_PATH => cyclic = true,
                                _ => {}
                            }
                        }
                    }
                    if cyclic {
                        self.rd.nodes[x].state = BAD;
                        stack.truncate(stack.iter().rposition(|&q| q as usize == x).unwrap());
                    }
                }
                _ => {
                    // the children are done
                    stack.pop();
                    let (a, b) = (self.rd.nodes[x].a, self.rd.nodes[x].b);
                    let bad = [a, b].iter().any(|k| matches!(k, Kid::Node(c) if self.rd.nodes[*c as usize].state != DONE));
                    if bad {
                        self.rd.nodes[x].state = BAD;
                        continue;
                    }
                    self.merge_ranges(x)?;
                    let simple = |rd: &Reading, k: Kid| -> bool {
                        match k {
                            Kid::Leaf(_) => true,
                            Kid::Node(c) => rd.nodes[c as usize].xs.len() == 1,
                            Kid::Open => false,
                        }
                    };
                    let chained = |rd: &Reading, k: Kid| -> bool {
                        match k {
                            Kid::Node(c) => rd.nodes[c as usize].chain,
                            _ => true,
                        }
                    };
                    let chain = (simple(&self.rd, a) && chained(&self.rd, b)) || (simple(&self.rd, b) && chained(&self.rd, a));
                    self.rd.nodes[x].chain = chain;
                    self.make_slots(x)?;
                    self.rd.nodes[x].state = DONE;
                }
            }
        }
        Some(())
    }

    /// The check of one constraint of a merge or direct node: is every clause its
    /// lines rely on in the formula? For a direct node the clause ids go into
    /// sums, so a copy with a repeated literal does not count.
    fn check_slot(&mut self, s: u32) -> Option<bool> {
        let (node, t, cap) = {
            let sl = &self.rd.slots[s as usize];
            (sl.node as usize, sl.t as i64, sl.cap)
        };
        let is_direct = !self.rd.nodes[node].xs.is_empty();
        if is_direct {
            let xs = self.rd.nodes[node].xs.clone();
            let n = xs.len();
            let m = t as usize;
            if m < 1 || m > n || n > MAX_T {
                return Some(false);
            }
            let out = if cap { 0 } else { self.rd.nodes[node].r[m - 1] };
            let start = self.rd.ids.len() as u32;
            let mut mask = (1u32 << m) - 1;
            let mut query: Vec<i32> = Vec::with_capacity(m + 1);
            while mask < (1u32 << n) {
                query.clear();
                let mut rest = mask;
                while rest != 0 {
                    let b = rest.trailing_zeros() as usize;
                    rest &= rest - 1;
                    query.push(-xs[b]);
                }
                if !cap {
                    query.push(out);
                }
                match self.find(&query)? {
                    Some(i) if !self.ix.f.dup[i as usize] => self.rd.ids.push(i + 1),
                    _ => {
                        self.rd.ids.truncate(start as usize);
                        return Some(false);
                    }
                }
                mask = next_mask(mask);
            }
            self.rd.slots[s as usize].aux = start;
            return Some(true);
        }
        let (a, b) = (self.rd.nodes[node].a, self.rd.nodes[node].b);
        if matches!(a, Kid::Open) || matches!(b, Kid::Open) {
            return Some(false);
        }
        let (imin, imax) = self.rd.splits(&self.rd.nodes[node], t);
        // with more than one count between the children's limits no line would
        // say so
        if imin - 1 > imax {
            return Some(false);
        }
        let out = if cap { 0 } else { self.rd.nodes[node].r[t as usize - 1] };
        let mut clause = [0i32; 3];
        for i in imin..=imax {
            let j = t - i;
            let mut n = 0;
            if i > 0 {
                clause[n] = -self.rd.kid_out(a, i);
                n += 1;
            }
            if j > 0 {
                clause[n] = -self.rd.kid_out(b, j);
                n += 1;
            }
            if !cap {
                clause[n] = out;
                n += 1;
            }
            if n == 0 || self.find(&clause[..n])?.is_none() {
                return Some(false);
            }
        }
        let mut buf = std::mem::take(&mut self.buf);
        let ok = self.rd.deps(s, &mut buf);
        self.buf = buf;
        self.ck.pay(1 + self.buf.len() as u64)?;
        Some(ok)
    }

    /// Mark a node's leaves. False when a variable appears twice.
    fn mark_leaves(&mut self, ni: usize) -> Option<bool> {
        let nd = &self.rd.nodes[ni];
        let (start, len) = (nd.leaf_start as usize, nd.leaf_len as usize);
        self.ck.pay(1 + len as u64)?;
        self.next_mark();
        for k in start..start + len {
            let l = self.rd.leaves[k];
            if self.marked(l) {
                return Some(false);
            }
            self.set_mark(l);
        }
        Some(true)
    }

    /// Check a count's whole proof from one slot, with the count's leaves marked:
    /// every clause is there and no output is one of the leaves. Returns its line
    /// count and how many of its slots write lines, or None inside when the check
    /// fails.
    fn check_count(&mut self, root: u32) -> Option<Option<(u64, u64)>> {
        self.rd.epoch = self.rd.epoch.wrapping_add(1);
        let e = self.rd.epoch;
        let mut stack = vec![root];
        self.rd.slots[root as usize].stamp = e;
        let mut lines = 0u64;
        let mut written = 0u64;
        let mut buf: Vec<u32> = Vec::new();
        self.visited.clear();
        while let Some(s) = stack.pop() {
            self.ck.pay(3)?;
            self.visited.push(s);
            let state = self.rd.slots[s as usize].state;
            if state == 2 {
                return Some(None);
            }
            if state == 0 {
                let ok = self.check_slot(s)?;
                self.rd.slots[s as usize].state = if ok { 1 } else { 2 };
                if !ok {
                    return Some(None);
                }
            }
            let (node, t, cap) = {
                let sl = &self.rd.slots[s as usize];
                (sl.node as usize, sl.t as usize, sl.cap)
            };
            if !cap && self.marked(self.rd.nodes[node].r[t - 1]) {
                return Some(None);
            }
            let own = self.rd.slot_lines(s);
            lines += own;
            written += (own > 0) as u64;
            if !self.rd.deps(s, &mut buf) {
                return Some(None);
            }
            self.ck.pay(1 + buf.len() as u64)?;
            for &d in &buf {
                if self.rd.slots[d as usize].stamp != e {
                    self.rd.slots[d as usize].stamp = e;
                    stack.push(d);
                }
            }
        }
        Some(Some((lines, written)))
    }

    /// Returns whether it is a new count: the same count read twice stays one.
    fn add_found(&mut self, lits: Vec<i32>, need: i32, form: Form, plan: Plan, lines: u64) -> Option<bool> {
        self.ck.pay(4 + lits.len() as u64)?;
        let key = (set_hash(&lits), need, lits.len() as u32);
        if !self.seen_found.insert(key) {
            return Some(false);
        }
        self.rd.found.push(Found { lits, need, form });
        self.rd.plans.push(plan);
        self.rd.line_counts.push(lines);
        self.rd.proved.push(0);
        Some(true)
    }

    /// A count of a tree, right after its check: its slots learn that one more count
    /// needs them, and a cap that is the count's own constraint stays in the proof.
    /// The count and those marks go in together, or a later `del` would take a slot
    /// another count still needs.
    fn add_tree_count(&mut self, ni: usize, need: i32, form: Form, slot: u32, unit: Option<(u64, i64)>, lines: u64, written: u64) -> Option<()> {
        let lits = self.negated_leaves(ni)?;
        // the root is a helper when a sum with a unit comes after it
        let helpers = written - (unit.is_none() && self.rd.slot_lines(slot) > 0) as u64;
        let del = helpers > 0;
        let total = lines + unit.is_some() as u64 + del as u64;
        if !self.add_found(lits, need, form, Plan::Merge { slot, unit, del }, total)? {
            return Some(());
        }
        if unit.is_none() {
            self.rd.slots[slot as usize].keep = true;
        }
        // paid for by the check's walk over them
        for k in 0..self.visited.len() {
            let s = self.visited[k] as usize;
            self.rd.slots[s].uses += 1;
        }
        Some(())
    }

    fn negated_leaves(&mut self, ni: usize) -> Option<Vec<i32>> {
        let nd = &self.rd.nodes[ni];
        let (s, n) = (nd.leaf_start as usize, nd.leaf_len as usize);
        self.ck.pay(1 + n as u64)?;
        Some(self.rd.leaves[s..s + n].iter().map(|&l| -l).collect())
    }

    /// The counts a finished node gives: a unit on an output, and at the root the
    /// overflow clauses, perhaps with a last input that has nothing else.
    fn emit_node(&mut self, ni: usize) -> Option<()> {
        if self.rd.nodes[ni].state != DONE {
            return Some(());
        }
        let w = self.rd.nodes[ni].r.len();
        self.ck.pay(1 + w as u64)?;
        let has_unit = self.rd.nodes[ni].r.iter().any(|&o| self.ix.unit[li(-o)] != 0);
        let parent = self.rd.nodes[ni].parent;
        if !has_unit && parent {
            return Some(());
        }
        let n = self.rd.nodes[ni].leaf_len as usize;
        if !self.mark_leaves(ni)? {
            return Some(());
        }
        let form = if self.rd.nodes[ni].chain { Form::Sinz } else { Form::Totalizer };
        let base = self.rd.nodes[ni].slot;
        let mut unit_read = false;
        if n >= 3 && has_unit {
            for m in 1..=w.min(n) {
                let o = self.rd.nodes[ni].r[m - 1];
                let u = self.ix.unit[li(-o)];
                if u == 0 {
                    continue;
                }
                if m == 1 {
                    break; // at most none: units, which propagation already has
                }
                if u != 2 {
                    continue;
                }
                let slot = base + m as u32 - 1;
                if let Some((lines, written)) = self.check_count(slot)? {
                    let need = (n - m + 1) as i32;
                    let Some(cid) = self.find(&[-o])? else { continue };
                    self.add_tree_count(ni, need, form, slot, Some((cid as u64 + 1, need as i64)), lines, written)?;
                    unit_read = true;
                    self.tree_units.push(-o);
                    break;
                }
            }
        }
        if parent || w == 0 {
            return Some(());
        }
        // the last input of a Sinz counter has only the overflow clause with the
        // top register: one literal from outside the tree that it excludes. A
        // node that counts all its leaves and shows no overflow clause is a
        // clause in disguise, not the end of a counter.
        let top = self.rd.nodes[ni].r[w - 1];
        let nb = self.ix.bins(-top).len();
        self.ck.pay(1 + nb as u64)?;
        let mut last = 0;
        let mut seen = 0;
        if nb <= MAX_DEG && (n > w || self.rd.nodes[ni].overflow) {
            for &q in self.ix.bins(-top) {
                let u = -q;
                if !self.marked(u) && var(u) != var(top) {
                    last = u;
                    seen += 1;
                }
            }
        }
        if seen == 1 && n + 1 >= 3 && n + 1 > w {
            if let Some((lines, written)) = self.check_last_input(ni, last, w)? {
                let top_node = self.rd.nodes.len() - 1;
                let slot = self.rd.nodes[top_node].slot;
                self.add_tree_count(top_node, (n + 1 - w) as i32, form, slot, None, lines, written)?;
                return Some(());
            }
        }
        // the cap says less than a unit on the same leaves
        if n >= 3 && n > w && !unit_read {
            if !self.mark_leaves(ni)? {
                return Some(());
            }
            let slot = base + w as u32;
            if let Some((lines, written)) = self.check_count(slot)? {
                self.add_tree_count(ni, (n - w) as i32, form, slot, None, lines, written)?;
            }
        }
        Some(())
    }

    /// A node with no outputs, only the overflow clause with one more input. It
    /// stays in the list when it passes. The node's leaves are marked.
    fn check_last_input(&mut self, ni: usize, last: i32, w: usize) -> Option<Option<(u64, u64)>> {
        self.set_mark(last);
        self.ck.pay(8)?;
        let (nodes_len, leaves_len, slots_len) = (self.rd.nodes.len(), self.rd.leaves.len(), self.rd.slots.len());
        let mut nd = new_node();
        nd.a = Kid::Node(ni as u32);
        nd.b = Kid::Leaf(last);
        nd.cap_c = w as u32;
        nd.chain = self.rd.nodes[ni].chain;
        nd.parent = true;
        nd.state = DONE;
        self.rd.nodes.push(nd);
        self.merge_ranges(nodes_len)?;
        self.make_slots(nodes_len)?;
        let slot = self.rd.nodes[nodes_len].slot;
        let result = self.check_count(slot)?;
        if result.is_none() {
            self.rd.nodes.truncate(nodes_len);
            self.rd.slots.truncate(slots_len);
            self.rd.leaves.truncate(leaves_len);
        }
        Some(result)
    }

    /// Merge and direct nodes: read, link, finish, then the counts they give.
    fn read_trees(&mut self) -> Option<()> {
        let nl = self.ix.off2.len() - 1;
        for i in 2..nl {
            let l = if i & 1 == 1 { -((i >> 1) as i32) } else { (i >> 1) as i32 };
            self.ck.pay(1)?;
            self.direct_from_second(l)?;
        }
        for i in 2..nl {
            let l = if i & 1 == 1 { -((i >> 1) as i32) } else { (i >> 1) as i32 };
            self.seeds_from_first(l)?;
        }
        self.make_lookups()?;
        // only a node that can say something is finished, and with it what is below
        // it: a unit on an output, or an overflow clause. Most nodes are pairs of
        // gate inputs that nothing else reads.
        let count = self.rd.nodes.len();
        for ni in 0..count {
            let w = self.rd.nodes[ni].r.len();
            self.ck.pay(1 + w as u64)?;
            if self.rd.nodes[ni].overflow || self.rd.nodes[ni].r.iter().any(|&o| self.ix.unit[li(-o)] != 0) {
                self.finish(ni)?;
            }
        }
        // a parent counts only when it is whole itself; the first columns the
        // linking made are in the list by now
        let count = self.rd.nodes.len();
        self.ck.pay(count as u64)?;
        for ni in 0..count {
            if self.rd.nodes[ni].state != DONE {
                continue;
            }
            let (a, b) = (self.rd.nodes[ni].a, self.rd.nodes[ni].b);
            for k in [a, b] {
                if let Kid::Node(c) = k {
                    self.rd.nodes[c as usize].parent = true;
                }
            }
        }
        for ni in 0..count {
            self.ck.pay(1)?;
            self.emit_node(ni)?;
        }
        Some(())
    }

    /// The nodes no proof can reach go, with what they hold: on a formula full of
    /// gates that is nearly all of them, and the reading stays for the whole run.
    fn keep_used_nodes(&mut self) -> Option<()> {
        let n = self.rd.nodes.len();
        self.ck.pay(1 + n as u64 + self.rd.slots.len() as u64)?;
        let mut map = vec![NONE; n];
        let mut kept = 0u32;
        for (i, nd) in self.rd.nodes.iter().enumerate() {
            if nd.slot != NONE {
                map[i] = kept;
                kept += 1;
            }
        }
        let old = std::mem::take(&mut self.rd.nodes);
        let mut nodes: Vec<Node> = Vec::with_capacity(kept as usize);
        let renumber = |k: Kid| match k {
            Kid::Node(c) if map[c as usize] != NONE => Kid::Node(map[c as usize]),
            Kid::Node(_) => Kid::Open,
            other => other,
        };
        for (i, mut nd) in old.into_iter().enumerate() {
            if map[i] != NONE {
                nd.va = Vec::new();
                nd.vb = Vec::new();
                nd.a = renumber(nd.a);
                nd.b = renumber(nd.b);
                nodes.push(nd);
            }
        }
        self.rd.nodes = nodes;
        for sl in self.rd.slots.iter_mut() {
            sl.node = map[sl.node as usize];
        }
        self.singles = HashMap::new();
        self.first_head = Vec::new();
        self.first_next = Vec::new();
        self.later_head = Vec::new();
        self.later = Vec::new();
        Some(())
    }

    /// The literals in the clauses of -s, the first register's column input is
    /// one of them.
    fn input_candidates(&mut self, s: i32, out: &mut Vec<i32>) -> Option<()> {
        out.clear();
        let (nb, nt) = (self.ix.bins(-s).len(), self.ix.tris(-s).len());
        if nb + nt > MAX_REG {
            return Some(());
        }
        self.ck.pay(1 + (nb + 2 * nt) as u64 * 2)?;
        out.extend_from_slice(self.ix.bins(-s));
        for &[p, q] in self.ix.tris(-s) {
            out.push(p);
            out.push(q);
        }
        out.sort_unstable();
        out.dedup();
        Some(())
    }

    /// One column with input x: for every register its lo and hi. `pick` chooses
    /// among the hi candidates of the lowest register when there are several.
    /// None inside when x does not fit. Returns how many candidates there were.
    fn try_column(&mut self, regs: &[i32], x: i32, pick: usize) -> Option<(Option<Column>, usize)> {
        let l = regs.len();
        let mut next = vec![0i32; l + 1];
        let mut lo = vec![0i32; l];
        let mut hi = vec![0i32; l];
        let mut explained = 0u32;
        let mut alternatives = 0usize;
        self.ck.pay(2 * l as u64 + 2)?;
        let last_real = regs.iter().rposition(|&s| s != 0 && self.ix.unit[li(-s)] == 0);
        let mut ys: Vec<i32> = Vec::new();
        let mut zs: Vec<i32> = Vec::new();
        for h in 0..l {
            let s = regs[h];
            if s == 0 || self.ix.unit[li(-s)] != 0 {
                continue;
            }
            // the clause that holds x
            ys.clear();
            let nt = self.ix.tris(-s).len();
            self.ck.pay(1 + nt as u64)?;
            if nt + self.ix.bins(-s).len() > MAX_REG {
                return Some((None, alternatives));
            }
            for &[p, q] in self.ix.tris(-s) {
                if p == x {
                    ys.push(q);
                } else if q == x {
                    ys.push(p);
                }
            }
            let bare = self.has_bin(-s, x)?;
            let known = next[h];
            let y = if known != 0 {
                if ys.contains(&known) {
                    known
                } else if bare {
                    0
                } else {
                    return Some((None, alternatives));
                }
            } else {
                self.ck.pay(ys.len() as u64 * (l as u64 + 1))?;
                if let Some(&y) = ys.iter().find(|&&y| !self.marked(y) && var(y) != var(x) && !next.iter().any(|&q| q != 0 && var(q) == var(y))) {
                    y
                } else if bare {
                    0
                } else {
                    return Some((None, alternatives));
                }
            };
            lo[h] = y;
            if y != 0 {
                next[h] = y;
            }
            explained += 1;
            // the clause that holds the register below: binary, or with lo
            zs.clear();
            let nb = self.ix.bins(-s).len();
            self.ck.pay(1 + nb as u64 + nt as u64)?;
            for &z in self.ix.bins(-s) {
                if z != x && z != y {
                    zs.push(z);
                }
            }
            if y != 0 {
                for &[p, q] in self.ix.tris(-s) {
                    let z = if p == y {
                        q
                    } else if q == y {
                        p
                    } else {
                        continue;
                    };
                    if z != x {
                        zs.push(z);
                    }
                }
            }
            self.ck.pay(zs.len() as u64 * (h as u64 + 1))?;
            zs.retain(|&z| !self.marked(z) && var(z) != var(x) && !next[..=h].iter().any(|&q| q != 0 && var(q) == var(z)));
            zs.sort_unstable();
            zs.dedup();
            // the register below must take this one as its lo
            let below = regs.get(h + 1).copied().unwrap_or(0);
            if below != 0 && self.ix.unit[li(-below)] == 0 {
                let mut kept: Vec<i32> = Vec::with_capacity(zs.len());
                for k in 0..zs.len() {
                    if self.find(&[-below, zs[k], x])?.is_some() {
                        kept.push(zs[k]);
                    }
                }
                zs = kept;
            }
            if zs.is_empty() {
                // no register below: j = 1, so it must be the lowest
                if Some(h) != last_real {
                    return Some((None, alternatives));
                }
                continue;
            }
            // the lowest register may also be the last one, j = 1, whatever other
            // clauses hold it: that reading is the last option
            let z = if Some(h) == last_real {
                alternatives = zs.len() + 1;
                match zs.get(pick) {
                    Some(&z) => z,
                    None => continue,
                }
            } else {
                zs[0]
            };
            explained += 1;
            hi[h] = z;
            next[h + 1] = z;
        }
        Some((Some(Column { x, regs: regs.to_vec(), lo, hi, next, explained }), alternatives))
    }

    /// Every way the clauses can be read as one column of these registers, the
    /// ones that explain the most clauses first.
    fn column_options(&mut self, regs: &[i32]) -> Option<Vec<Column>> {
        let mut out: Vec<Column> = Vec::new();
        let first = regs.iter().copied().find(|&s| s != 0 && self.ix.unit[li(-s)] == 0);
        let Some(first) = first else {
            // every register is false by a unit: nothing below
            out.push(Column { x: 0, regs: regs.to_vec(), lo: vec![0; regs.len()], hi: vec![0; regs.len()], next: vec![0; regs.len() + 1], explained: 0 });
            return Some(out);
        };
        let mut cands: Vec<i32> = Vec::new();
        self.input_candidates(first, &mut cands)?;
        for &x in &cands {
            if self.marked(x) {
                continue;
            }
            let (col, alternatives) = self.try_column(regs, x, 0)?;
            if let Some(col) = col {
                out.push(col);
                for pick in 1..alternatives.min(4) {
                    let (col, _) = self.try_column(regs, x, pick)?;
                    if let Some(col) = col {
                        out.push(col);
                    }
                }
            }
        }
        // a stable sort: the candidates stay in their order
        out.sort_by_key(|c| std::cmp::Reverse(c.explained));
        Some(out)
    }

    /// Read a one-way counter from an asserted register `u`, column by column
    /// from the top. A wrong guess dies within a column or two; a guess the walk
    /// cannot finish is undone.
    fn read_walk(&mut self, u: i32) -> Option<()> {
        if (self.ix.bins(-u).is_empty() && self.ix.tris(-u).is_empty()) || self.clause_read[li(u)] || self.d_root.get(li(u)) == Some(&true) {
            return Some(());
        }
        self.next_mark();
        self.set_mark(u);
        // the first reading of the top column that gets to the bottom and counts past
        // one is the count. A chain that counts only to one is a long clause in
        // disguise, and every way through it is another true clause: only the longest
        // is kept, only when there is no real count here, and only when it ends in
        // a real input, because a chain of false registers is a resolvent of other
        // constraints' clauses.
        let tops = self.column_options(&[u])?;
        let mut clause_like: Option<Vec<Column>> = None;
        let mut longest = 0;
        let mut steps = 0usize;
        for top in tops {
            self.next_mark();
            self.set_mark(u);
            let Some(cols) = self.walk_down(top, &mut steps)? else { continue };
            if walk_width(&cols) >= 2 {
                return self.check_walk(u, &cols);
            }
            let n = cols.iter().filter(|c| c.x != 0).count();
            if n > longest {
                longest = n;
                clause_like = Some(cols);
            }
        }
        if let Some(cols) = clause_like {
            let total: usize = cols.iter().map(|c| c.regs.len()).sum();
            self.ck.pay(1 + 2 * total as u64)?;
            if longest >= MIN_CHAIN && cols.iter().all(|c| c.regs.iter().all(|&s| s == 0 || self.ix.unit[li(-s)] == 0)) {
                self.check_walk(u, &cols)?;
                // a chain with a unit at every step gives one clause, not one a step
                for c in &cols {
                    for &s in &c.regs {
                        if s != 0 {
                            self.clause_read[li(s)] = true;
                        }
                    }
                }
            }
        }
        Some(())
    }

    /// From one reading of the top column to the bottom, trying the next option
    /// wherever one dies. Returns the columns, or None inside when no option goes
    /// through or the steps, which all readings of a top column share, run out.
    fn walk_down(&mut self, top: Column, steps: &mut usize) -> Option<Option<Vec<Column>>> {
        let mut levels: Vec<(Vec<Column>, usize)> = vec![(vec![top], 0)];
        let found = loop {
            *steps += 1;
            self.ck.pay(1)?;
            if *steps > 16 + 3 * levels.len() {
                break false;
            }
            let (opts, at) = levels.last_mut().unwrap();
            if *at == opts.len() {
                levels.pop();
                let Some((opts, at)) = levels.last() else { break false };
                // undo the marks of the choice that led here
                let col = &opts[*at - 1];
                self.ck.pay(1 + col.next.len() as u64)?;
                if col.x != 0 {
                    self.mark[var(col.x)] = 0;
                }
                for &q in &col.next {
                    if q != 0 {
                        self.mark[var(q)] = 0;
                    }
                }
                continue;
            }
            let i = *at;
            *at += 1;
            let col = &opts[i];
            let (x, mut regs) = (col.x, col.next.clone());
            self.ck.pay(1 + regs.len() as u64)?;
            while regs.last() == Some(&0) {
                regs.pop();
            }
            // a variable the walk has already used ends this option, and so does
            // a lone register of a chain already read as one clause; what the
            // option marked is taken back
            let mut taken: Vec<i32> = Vec::with_capacity(regs.len() + 1);
            let mut clash = regs.len() == 1 && regs[0] != 0 && self.clause_read[li(regs[0])];
            for &q in std::iter::once(&x).chain(regs.iter()) {
                if q != 0 && !clash {
                    if self.marked(q) {
                        clash = true;
                        break;
                    }
                    self.set_mark(q);
                    taken.push(q);
                }
            }
            if clash {
                for &q in &taken {
                    self.mark[var(q)] = 0;
                }
                continue;
            }
            if regs.is_empty() {
                break true;
            }
            let options = self.column_options(&regs)?;
            levels.push((options, 0));
        };
        if !found {
            return Some(None);
        }
        Some(Some(levels.into_iter().map(|(mut opts, at)| opts.swap_remove(at - 1)).collect()))
    }

    /// The check of a one-way counter: the plan alone, against the formula. Every
    /// variable once; for every register, either a false unit, or the clause with
    /// its input and the clause with the register below, those two registers
    /// being the next column's at the same place and one place lower.
    fn check_walk(&mut self, u: i32, cols: &[Column]) -> Option<()> {
        let inputs: Vec<i32> = cols.iter().map(|c| c.x).filter(|&x| x != 0).collect();
        let n = inputs.len();
        let k = walk_width(cols);
        if n < 3 || k == 0 || k >= n {
            return Some(());
        }
        if self.ix.unit[li(u)] != 2 {
            return Some(());
        }
        let Some(cid) = self.find(&[u])? else { return Some(()) };
        let total: usize = cols.iter().map(|c| c.regs.len()).sum();
        self.ck.pay(4 + (n + total) as u64)?;
        // an input column has its input; the last column may have none
        for (d, c) in cols.iter().enumerate() {
            if c.x == 0 && d + 1 != cols.len() {
                return Some(());
            }
        }
        self.next_mark();
        for &x in &inputs {
            if self.marked(x) {
                return Some(());
            }
            self.set_mark(x);
        }
        for c in cols {
            for &s in &c.regs {
                if s != 0 {
                    if self.marked(s) {
                        return Some(());
                    }
                    self.set_mark(s);
                }
            }
        }
        if cols[0].regs.first() != Some(&u) {
            return Some(());
        }
        for (d, c) in cols.iter().enumerate() {
            let below: &[i32] = cols.get(d + 1).map_or(&[], |b| &b.regs);
            let at = |h: usize| below.get(h).copied().unwrap_or(0);
            for (h, &s) in c.regs.iter().enumerate() {
                if s == 0 {
                    continue;
                }
                if self.ix.unit[li(-s)] != 0 {
                    continue;
                }
                let j = k - h;
                let (lo, hi) = (c.lo[h], c.hi[h]);
                if c.x == 0 || (lo != 0 && lo != at(h)) {
                    return Some(());
                }
                let holds_input = if lo != 0 { self.find(&[-s, lo, c.x])?.is_some() } else { self.find(&[-s, c.x])?.is_some() };
                if !holds_input {
                    return Some(());
                }
                if j >= 2 {
                    if hi == 0 || hi != at(h + 1) {
                        return Some(());
                    }
                    let holds_below = self.find(&[-s, hi])?.is_some() || (lo != 0 && self.find(&[-s, lo, hi])?.is_some());
                    if !holds_below {
                        return Some(());
                    }
                } else if hi != 0 {
                    return Some(());
                }
            }
        }
        // the plan: registers from the top down, so a register's lo and hi come
        // after it
        let in_start = self.rd.walk_inputs.len() as u32;
        self.rd.walk_inputs.extend_from_slice(&inputs);
        let reg_start = self.rd.regs.len() as u32;
        // the input position of each column, and a hash of what is left below it
        let mut position = vec![0u32; cols.len()];
        let mut seen_inputs = 0u32;
        for (d, c) in cols.iter().enumerate() {
            position[d] = seen_inputs;
            if c.x != 0 {
                seen_inputs += 1;
            }
        }
        let mut suffix = vec![0u64; n + 1];
        for p in (0..n).rev() {
            suffix[p] = suffix[p + 1].wrapping_add(mix(inputs[p]));
        }
        // where each register will be in the plan
        let mut place: Vec<Vec<u32>> = Vec::with_capacity(cols.len());
        let mut next_index = reg_start;
        for c in cols {
            let mut row = vec![NONE; c.regs.len()];
            for (h, &s) in c.regs.iter().enumerate() {
                if s != 0 {
                    row[h] = next_index;
                    next_index += 1;
                }
            }
            place.push(row);
        }
        for (d, c) in cols.iter().enumerate() {
            for (h, &s) in c.regs.iter().enumerate() {
                if s == 0 {
                    continue;
                }
                let base = self.ix.unit[li(-s)] != 0;
                let below = place.get(d + 1);
                let index = |row: Option<&Vec<u32>>, h: usize| row.and_then(|r| r.get(h).copied()).unwrap_or(NONE);
                let lo = if base || c.lo[h] == 0 { NONE } else { index(below, h) };
                let hi = if base || c.hi[h] == 0 { NONE } else { index(below, h + 1) };
                let j = (k - h) as u32;
                let i = n as u32 - position[d];
                let tslot = self.tslot_for(s, j, i, suffix[position[d] as usize], in_start + position[d])?;
                self.rd.regs.push(Reg { s, j, d: position[d], lo, hi, tslot });
            }
        }
        let reg_len = self.rd.regs.len() as u32 - reg_start;
        let walk = self.rd.walks.len() as u32;
        self.rd.walks.push(Walk { in_start, in_len: n as u32, reg_start, reg_len, k: k as u32, unit_id: cid as u64 + 1 });
        // pbc, its lines, qed; two more for the sum with the unit and the `del`
        let mut lines = 2u64;
        self.ck.pay(1 + 2 * reg_len as u64)?;
        for g in reg_start as usize..(reg_start + reg_len) as usize {
            lines += self.rd.reg_lines(g);
        }
        let mut lits = inputs;
        lits.reverse();
        if self.add_found(lits, k as i32, Form::OneWay, Plan::OneWay { walk }, lines)? {
            for g in reg_start as usize..(reg_start + reg_len) as usize {
                let t = self.rd.regs[g].tslot as usize;
                self.rd.tslot_uses[t] += 1;
            }
        }
        Some(())
    }

    /// One proof id per distinct T statement: two counts over the same registers
    /// share them.
    fn tslot_for(&mut self, s: i32, j: u32, i: u32, hash: u64, start: u32) -> Option<u32> {
        self.ck.pay(2)?;
        let key = (s, j, i, hash);
        if let Some(&slot) = self.tslot_of.get(&key) {
            self.ck.pay(i as u64)?;
            let (a, n) = self.rd.tslot_inputs[slot as usize];
            let here = &self.rd.walk_inputs[start as usize..(start + i) as usize];
            let there = &self.rd.walk_inputs[a as usize..(a + n) as usize];
            if here == there {
                return Some(slot);
            }
            let slot = self.rd.tslots.len() as u32;
            self.rd.tslots.push(0);
            self.rd.tslot_uses.push(0);
            self.rd.tslot_inputs.push((start, i));
            return Some(slot);
        }
        let slot = self.rd.tslots.len() as u32;
        self.rd.tslots.push(0);
        self.rd.tslot_uses.push(0);
        self.rd.tslot_inputs.push((start, i));
        self.tslot_of.insert(key, slot);
        Some(slot)
    }

    fn read_walks(&mut self) -> Option<()> {
        let f = self.ix.f;
        let nl = self.ix.unit.len();
        self.ck.pay(nl as u64)?;
        self.clause_read = vec![false; nl];
        for i in 0..f.len() {
            self.ck.pay(1)?;
            let c = f.clause(i);
            if self.ix.uniq[i] && c.len() == 1 {
                self.read_walk(c[0])?;
            }
        }
        Some(())
    }

    /// The clauses of length m sharing all of `rest`: the extra literal of each.
    fn subset_candidates(&mut self, rest: &[i32], m: usize, out: &mut Vec<i32>) -> Option<()> {
        out.clear();
        let rare = rest.iter().copied().min_by_key(|&l| (self.ix.tris(l).len() + self.ix.longs(l).len(), l)).unwrap();
        if m == 3 {
            let n = self.ix.tris(rare).len();
            self.ck.pay(1 + n as u64)?;
            let other = if rest[0] == rare { rest[1] } else { rest[0] };
            for &[p, q] in self.ix.tris(rare) {
                if p == other {
                    out.push(q);
                } else if q == other {
                    out.push(p);
                }
            }
        } else {
            let n = self.ix.longs(rare).len();
            self.ck.pay(1 + n as u64)?;
            for k in 0..n {
                let c = self.ix.f.clause(self.ix.longs(rare)[k] as usize);
                if c.len() != m {
                    continue;
                }
                self.ck.pay((m * m) as u64)?;
                if !rest.iter().all(|l| c.contains(l)) {
                    continue;
                }
                if let Some(&e) = c.iter().find(|l| !rest.contains(l)) {
                    out.push(e);
                }
            }
        }
        Some(())
    }

    /// Subset families: every m-subset of t literals as a clause, m >= 3, t > m.
    /// The family grows from one clause, a literal at a time, as long as every
    /// new subset is there.
    fn read_subsets(&mut self) -> Option<()> {
        let f = self.ix.f;
        self.ck.pay(f.len() as u64)?;
        let mut used = vec![false; f.len()];
        let mut cands: Vec<i32> = Vec::new();
        let mut query: Vec<i32> = Vec::with_capacity(MAX_LEN);
        for i in 0..f.len() {
            self.ck.pay(1)?;
            let c = f.clause(i);
            let m = c.len();
            if !self.ix.uniq[i] || !(3..=MAX_LEN).contains(&m) || used[i] {
                continue;
            }
            // each literal of a family with t > m is in at least m of its clauses
            self.ck.pay(m as u64)?;
            if c.iter().any(|&l| (if m == 3 { self.ix.tris(l).len() } else { self.ix.longs(l).len() }) < m) {
                continue;
            }
            self.subset_candidates(&c[1..], m, &mut cands)?;
            if cands.len() < 2 {
                continue; // the clause itself is always one
            }
            self.ck.pay(2 + 2 * cands.len() as u64)?;
            cands.sort_unstable();
            cands.dedup();
            if m - 1 + cands.len() > MAX_T {
                continue; // too large for the proof's table, as in the census
            }
            let mut s: Vec<i32> = c.to_vec();
            s.sort_unstable();
            for k in 0..cands.len() {
                let l = cands[k];
                if s.iter().any(|&q| var(q) == var(l)) {
                    continue;
                }
                // every (m-1)-subset of s with l
                let t = s.len();
                let mut mask = (1u32 << (m - 1)) - 1;
                let mut whole = true;
                while mask < (1u32 << t) {
                    query.clear();
                    let mut rest = mask;
                    while rest != 0 {
                        let b = rest.trailing_zeros() as usize;
                        rest &= rest - 1;
                        query.push(s[b]);
                    }
                    query.push(l);
                    if self.find(&query)?.is_none() {
                        whole = false;
                        break;
                    }
                    mask = next_mask(mask);
                }
                if whole {
                    s.push(l);
                }
            }
            if s.len() > m {
                self.check_family(s, m, &mut used)?;
            }
        }
        Some(())
    }

    /// The check of a subset family: every m-subset is a clean clause and no
    /// variable appears twice.
    fn check_family(&mut self, mut s: Vec<i32>, m: usize, used: &mut [bool]) -> Option<()> {
        let t = s.len();
        s.sort_unstable();
        if t > MAX_T || m < 3 || t <= m {
            return Some(());
        }
        self.next_mark();
        for &l in &s {
            if self.marked(l) {
                return Some(());
            }
            self.set_mark(l);
        }
        let mut ids: Vec<u32> = Vec::with_capacity(binom(t as u64, m as u64) as usize);
        self.ck.pay(ids.capacity() as u64)?;
        let mut query: Vec<i32> = Vec::with_capacity(m);
        let mut mask = (1u32 << m) - 1;
        while mask < (1u32 << t) {
            query.clear();
            let mut rest = mask;
            while rest != 0 {
                let b = rest.trailing_zeros() as usize;
                rest &= rest - 1;
                query.push(s[b]);
            }
            match self.find(&query)? {
                Some(i) if !self.ix.f.dup[i as usize] => ids.push(i + 1),
                _ => return Some(()),
            }
            mask = next_mask(mask);
        }
        for &id in &ids {
            used[id as usize - 1] = true;
        }
        let family = self.rd.families.len() as u32;
        let need = (t - m + 1) as i32;
        let lines = lattice_lines(t, m);
        self.rd.families.push(Family { lits: s.clone(), m: m as u32, ids, id: 0 });
        self.add_found(s, need, Form::Subset, Plan::Subset { family }, lines)?;
        Some(())
    }
}

impl Reader<'_> {
    /// The literal both a and b imply, with `made` the one to come closest to.
    fn common_output(&mut self, a: i32, b: i32, near: i32) -> Option<i32> {
        let (la, lb) = (self.ix.bins(-a).len(), self.ix.bins(-b).len());
        self.ck.pay(1 + (la + lb) as u64)?;
        if la > MAX_DEG || lb > MAX_DEG {
            return Some(0);
        }
        let (mut i, mut j) = (0, 0);
        let mut best = 0;
        let mut far = i64::MAX;
        while i < la && j < lb {
            let (x, y) = (self.ix.bins(-a)[i], self.ix.bins(-b)[j]);
            if x < y {
                i += 1;
            } else if y < x {
                j += 1;
            } else {
                let d = (x.unsigned_abs() as i64 - near.unsigned_abs() as i64).abs();
                if x != a && x != b && var(x) != var(near) && var(x) != var(a) && var(x) != var(b) && d < far {
                    far = d;
                    best = x;
                }
                i += 1;
                j += 1;
            }
        }
        Some(best)
    }

    fn add_gate(&mut self, g: Gate) -> Option<u32> {
        self.ck.pay(2)?;
        let at = self.gates.len() as u32;
        self.gates.push(g);
        for o in [g.h, g.l] {
            if o != 0 {
                self.gate_seen[li(o)] = true;
                self.gate_of[li(o)] = at;
            }
        }
        Some(at)
    }

    /// The gate that makes a wire: a comparator with this wire as its min or its max,
    /// or, on top of comparators, an AND or an OR left over from one. Every wire is
    /// looked at once.
    fn gate_for(&mut self, w: i32, depth: u32) -> Option<u32> {
        let at = li(w);
        if self.gate_seen[at] {
            return Some(self.gate_of[at]);
        }
        self.gate_seen[at] = true;
        self.gate_of[at] = NONE;
        let nt = self.ix.tris(w).len();
        let nb = self.ix.bins(w).len();
        self.ck.pay(1)?;
        if nt > MAX_WIRE || nb > MAX_PRE {
            return Some(NONE);
        }
        // w as the min: (-a -b w) and an output both a and b imply
        for k in 0..nt {
            let [p, q] = self.ix.tris(w)[k];
            let (a, b) = (-p, -q);
            if var(a) == var(b) {
                continue;
            }
            let h = self.common_output(a, b, w)?;
            if h != 0 && !self.gate_seen[li(h)] {
                return self.add_gate(Gate { a, b, h, l: w });
            }
        }
        // w as the max: a and b imply it, and (-a -b l)
        let mut outs: Vec<i32> = Vec::new();
        for x in 0..nb {
            for y in x + 1..nb {
                let (a, b) = (-self.ix.bins(w)[x], -self.ix.bins(w)[y]);
                if var(a) == var(b) || var(a) == var(w) || var(b) == var(w) {
                    continue;
                }
                self.out(a, b, &mut outs)?;
                outs.retain(|&l| var(l) != var(w) && var(l) != var(a) && var(l) != var(b));
                if let Some(l) = closest(outs.iter().copied(), w) {
                    if !self.gate_seen[li(l)] {
                        return self.add_gate(Gate { a, b, h: w, l });
                    }
                }
            }
        }
        // half of a comparator, over wires that come from comparators. In a network
        // written both ways a wire's reader looks like a gate that makes it, from the
        // clauses it read the wire back with: a gate whose input is this wire says so.
        if depth >= MAX_HALF {
            return Some(NONE);
        }
        let mut only: Option<(i32, i32)> = None;
        let mut several = false;
        for k in 0..nt {
            let [p, q] = self.ix.tris(w)[k];
            let (a, b) = (-p, -q);
            if var(a) == var(b) || self.clause_of_reader_or(a, b, w)? || self.clause_of_reader_or(b, a, w)? {
                continue;
            }
            several |= only.is_some();
            only = Some((a, b));
        }
        if let (Some((a, b)), false) = (only, several) {
            if self.half_allowed(a, b, depth)? {
                return self.add_gate(Gate { a, b, h: 0, l: w });
            }
        }
        let mut pre: Vec<i32> = Vec::with_capacity(nb);
        for k in 0..nb {
            let a = -self.ix.bins(w)[k];
            if !self.clause_of_reader_and(a, w)? {
                pre.push(a);
            }
        }
        if pre.len() == 2 && var(pre[0]) != var(pre[1]) && self.half_allowed(pre[0], pre[1], depth)? {
            return self.add_gate(Gate { a: pre[0], b: pre[1], h: w, l: 0 });
        }
        Some(NONE)
    }

    /// A half comparator stands on another gate: any AND or OR would do otherwise, and
    /// the counts over gates' outputs would be lost to their inputs. The second reading
    /// also takes one on two plain wires.
    fn half_allowed(&mut self, a: i32, b: i32, depth: u32) -> Option<bool> {
        let (ga, gb) = (self.gate_for(a, depth + 1)?, self.gate_for(b, depth + 1)?);
        let stands = ga != NONE || gb != NONE;
        self.half_turned_away |= !stands;
        Some(self.half_on_inputs || stands)
    }

    /// (-a -b w) is the way back of a = w or not-b, a gate that reads w: w implies a,
    /// and so does not-b.
    fn clause_of_reader_or(&mut self, a: i32, b: i32, w: i32) -> Option<bool> {
        Some(self.has_bin(-w, a)? && self.has_bin(b, a)?)
    }

    /// (-a w) is the way back of a = w and c, a gate that reads w: a implies some c
    /// with (-w -c a).
    fn clause_of_reader_and(&mut self, a: i32, w: i32) -> Option<bool> {
        let nb = self.ix.bins(-a).len();
        if nb > MAX_WIRE {
            self.ck.pay(1)?;
            return Some(false);
        }
        self.ck.pay(1 + nb as u64)?;
        for k in 0..nb {
            let c = self.ix.bins(-a)[k];
            if c != w && var(c) != var(w) && var(c) != var(a) && self.find(&[-w, -c, a])?.is_some() {
                return Some(true);
            }
        }
        Some(false)
    }

    /// The gates behind the units, once as they stand and once more with half
    /// comparators on plain wires when the first reading gave no count.
    fn read_networks(&mut self) -> Option<()> {
        self.half_on_inputs = false;
        self.tree_units.sort_unstable();
        self.ck.pay(1 + self.tree_units.len() as u64)?;
        let before = self.rd.found.len();
        self.half_turned_away = false;
        self.read_networks_once()?;
        if self.rd.found.len() == before && self.half_turned_away {
            self.half_on_inputs = true;
            self.read_networks_once()?;
        }
        Some(())
    }

    /// From the units on wires, the gates behind them; then what each network says.
    fn read_networks_once(&mut self) -> Option<()> {
        let f = self.ix.f;
        let nl = self.ix.unit.len();
        self.ck.pay(2 * nl as u64 + f.len() as u64)?;
        self.gate_of = vec![NONE; nl];
        self.gate_seen = vec![false; nl];
        self.gates.clear();
        let mut roots: Vec<i32> = Vec::new();
        let mut stack: Vec<u32> = Vec::new();
        let mut opened: Vec<bool> = Vec::new();
        for i in 0..f.len() {
            self.ck.poll(i)?;
            if !self.ix.uniq[i] || f.start[i + 1] - f.start[i] != 1 {
                continue;
            }
            let o = -f.clause(i)[0];
            if self.tree_units.binary_search(&-o).is_ok() {
                continue;
            }
            let g = self.gate_for(o, 0)?;
            if g == NONE {
                continue;
            }
            roots.push(o);
            stack.push(g);
            while let Some(g) = stack.pop() {
                if opened.len() < self.gates.len() {
                    opened.resize(self.gates.len(), false);
                }
                if opened[g as usize] {
                    continue;
                }
                opened[g as usize] = true;
                let gate = self.gates[g as usize];
                for x in [gate.a, gate.b] {
                    let g2 = self.gate_for(x, 0)?;
                    if g2 != NONE {
                        stack.push(g2);
                    }
                }
            }
        }
        if self.gates.is_empty() {
            return Some(());
        }
        // one network per group of gates joined by a wire
        let count = self.gates.len();
        self.ck.pay(8 * count as u64 + 8 * roots.len() as u64)?;
        let mut parent: Vec<u32> = (0..count as u32).collect();
        fn root(parent: &mut [u32], mut x: u32) -> u32 {
            while parent[x as usize] != x {
                let up = parent[parent[x as usize] as usize];
                parent[x as usize] = up;
                x = up;
            }
            x
        }
        for i in 0..count {
            let g = self.gates[i];
            for x in [g.a, g.b] {
                let d = self.gate_of[li(x)];
                if d != NONE {
                    let (p, q) = (root(&mut parent, i as u32), root(&mut parent, d));
                    parent[p as usize] = q;
                }
            }
        }
        let tops: Vec<u32> = (0..count as u32).map(|i| root(&mut parent, i)).collect();
        let mut size = vec![0u32; count + 1];
        for &r in &tops {
            size[r as usize + 1] += 1;
        }
        for i in 0..count {
            size[i + 1] += size[i];
        }
        let mut members = vec![0u32; count];
        let mut fill = size.clone();
        for (i, &r) in tops.iter().enumerate() {
            members[fill[r as usize] as usize] = i as u32;
            fill[r as usize] += 1;
        }
        // the roots of each network, in the order of their units
        let mut by_top: Vec<(u32, i32)> = roots.iter().map(|&o| (tops[self.gate_of[li(o)] as usize], o)).collect();
        by_top.sort_by_key(|&(t, _)| t);
        let mut at = 0;
        while at < by_top.len() {
            let t = by_top[at].0;
            let mut end = at;
            while end < by_top.len() && by_top[end].0 == t {
                end += 1;
            }
            let wires: Vec<i32> = by_top[at..end].iter().map(|x| x.1).collect();
            let (a, b) = (size[t as usize] as usize, size[t as usize + 1] as usize);
            let before = self.rd.found.len();
            self.net_comp(&members[a..b], &wires)?;
            // a network that gave a count: no walk starts from its units
            if self.rd.found.len() > before {
                for &o in &wires {
                    if let Some(d) = self.d_root.get_mut(li(-o)) {
                        *d = true;
                    }
                }
            }
            at = end;
        }
        Some(())
    }

    /// One network: its order and its wires, then the sum and the sorted vectors.
    fn net_comp(&mut self, members: &[u32], roots: &[i32]) -> Option<()> {
        let count = members.len();
        self.ck.pay(1 + 16 * count as u64)?;
        let mut idx: HashMap<i32, u32> = HashMap::with_capacity(3 * count);
        let mut lit: Vec<i32> = Vec::new();
        let wire = |l: i32, idx: &mut HashMap<i32, u32>, lit: &mut Vec<i32>| -> u32 {
            *idx.entry(l).or_insert_with(|| {
                lit.push(l);
                lit.len() as u32 - 1
            })
        };
        // the gates over wire numbers
        let mut raw: Vec<[u32; 4]> = Vec::with_capacity(count);
        for &g in members {
            let gate = self.gates[g as usize];
            let a = wire(gate.a, &mut idx, &mut lit);
            let b = wire(gate.b, &mut idx, &mut lit);
            let h = if gate.h != 0 { wire(gate.h, &mut idx, &mut lit) } else { NONE };
            let l = if gate.l != 0 { wire(gate.l, &mut idx, &mut lit) } else { NONE };
            raw.push([a, b, h, l]);
        }
        let n_wires = lit.len();
        self.ck.pay(1 + 4 * n_wires as u64)?;
        let mut made = vec![NONE; n_wires];
        let mut reads = vec![0u32; n_wires];
        for (i, g) in raw.iter().enumerate() {
            for o in [g[2], g[3]] {
                if o != NONE {
                    if made[o as usize] != NONE {
                        return Some(());
                    }
                    made[o as usize] = i as u32;
                }
            }
            reads[g[0] as usize] += 1;
            reads[g[1] as usize] += 1;
        }
        // an order with every gate after what it reads
        let mut indeg = vec![0u32; count];
        let mut head = vec![NONE; count];
        let mut next: Vec<(u32, u32)> = Vec::with_capacity(2 * count);
        for (i, g) in raw.iter().enumerate() {
            for x in [g[0], g[1]] {
                let d = made[x as usize];
                if d != NONE {
                    indeg[i] += 1;
                    next.push((i as u32, head[d as usize]));
                    head[d as usize] = next.len() as u32 - 1;
                }
            }
        }
        let mut order: Vec<u32> = (0..count as u32).filter(|&i| indeg[i as usize] == 0).collect();
        let mut qi = 0;
        while qi < order.len() {
            let mut at = head[order[qi] as usize];
            qi += 1;
            while at != NONE {
                let (u, nx) = next[at as usize];
                at = nx;
                indeg[u as usize] -= 1;
                if indeg[u as usize] == 0 {
                    order.push(u);
                }
            }
        }
        if order.len() != count {
            return Some(()); // cyclic: no network
        }
        // every wire one variable of its own, and the inputs
        self.next_mark();
        let mut inputs: Vec<u32> = Vec::new();
        for (w, &l) in lit.iter().enumerate() {
            if self.marked(l) {
                return Some(());
            }
            self.set_mark(l);
            if made[w] == NONE {
                inputs.push(w as u32);
            }
        }
        let n = inputs.len();
        if n < 3 {
            return Some(());
        }
        let topo: Vec<[u32; 4]> = order.iter().map(|&i| raw[i as usize]).collect();
        // from here on a gate's number is its place in the order
        let mut place = vec![0u32; count];
        for (p, &g) in order.iter().enumerate() {
            place[g as usize] = p as u32;
        }
        for m in made.iter_mut() {
            if *m != NONE {
                *m = place[*m as usize];
            }
        }
        let net = NetView { lit, idx, topo, made, reads, inputs };
        // the sorted vectors say the most; the sum goes where they do not reach
        let best_tree = self.net_vectors(&net, roots)?;
        self.net_sum(&net, best_tree)
    }

    /// The sum over the network, when it says more than the vectors did.
    fn net_sum(&mut self, net: &NetView, best: i32) -> Option<()> {
        let n = net.inputs.len() as i64;
        self.ck.pay(1 + 2 * net.lit.len() as u64 + 3 * net.topo.len() as u64)?;
        if net.reads.iter().enumerate().any(|(w, &r)| r > 1 || (net.made[w] == NONE && r != 1)) {
            return Some(());
        }
        // the outputs nothing reads, and the wires left out
        let mut units: Vec<u64> = Vec::new();
        let mut free: Vec<i32> = Vec::new();
        for (w, &l) in net.lit.iter().enumerate() {
            if net.made[w] == NONE || net.reads[w] != 0 {
                continue;
            }
            if self.ix.unit[li(-l)] == 2 {
                let Some(c) = self.find(&[-l])? else { return Some(()) };
                units.push(c as u64 + 1);
            } else {
                free.push(l);
            }
        }
        let halves = net.topo.iter().filter(|g| g[2] == NONE || g[3] == NONE).count() as i64;
        let need = n - halves - free.len() as i64;
        if need < 2 || need >= n || need as i32 <= best {
            return Some(());
        }
        let lits: Vec<i32> = net.inputs.iter().map(|&x| -net.lit[x as usize]).collect();
        if self.seen_found.contains(&(set_hash(&lits), need as i32, n as u32)) {
            return Some(());
        }
        // the clauses are summed by id: no copy with a repeated literal
        let mut gates: Vec<([u64; 3], i32)> = Vec::with_capacity(net.topo.len());
        for g in &net.topo {
            let (a, b) = (net.lit[g[0] as usize], net.lit[g[1] as usize]);
            let id = |this: &mut Self, c: &[i32]| -> Option<Option<u64>> {
                Some(match this.find(c)? {
                    Some(i) if !this.ix.f.dup[i as usize] => Some(i as u64 + 1),
                    _ => None,
                })
            };
            let entry = if g[2] != NONE && g[3] != NONE {
                let (h, l) = (net.lit[g[2] as usize], net.lit[g[3] as usize]);
                let (Some(x), Some(y), Some(z)) = (id(self, &[-a, h])?, id(self, &[-b, h])?, id(self, &[-a, -b, l])?) else { return Some(()) };
                ([x, y, z], 0)
            } else if g[3] != NONE {
                let l = net.lit[g[3] as usize];
                let Some(z) = id(self, &[-a, -b, l])? else { return Some(()) };
                ([z, 0, 0], 0)
            } else {
                let h = net.lit[g[2] as usize];
                let Some(x) = id(self, &[-a, h])? else { return Some(()) };
                ([x, 0, 0], b)
            };
            gates.push(entry);
        }
        let at = self.rd.net_sums.len() as u32;
        self.rd.net_sums.push(NetSum { gates, units, free });
        let lines = self.rd.net_sum_lines(at);
        self.add_found(lits, need as i32, Form::Network, Plan::NetSum { net: at }, lines)?;
        Some(())
    }

    /// The units whose wires have one cone, the wire that says the most: the rank by
    /// a run of the network, then its vectors, or the subsets when there are few
    /// inputs. Returns the largest bound read.
    fn net_vectors(&mut self, net: &NetView, roots: &[i32]) -> Option<i32> {
        let n = net.inputs.len();
        if n > MAX_NET {
            return Some(0);
        }
        let words = n.div_ceil(64);
        let wires = net.lit.len();
        self.ck.pay(1 + (wires * words) as u64 + (net.topo.len() * words) as u64)?;
        let mut cones = vec![0u64; wires * words];
        for (i, &x) in net.inputs.iter().enumerate() {
            cones[x as usize * words + i / 64] |= 1u64 << (i % 64);
        }
        for g in &net.topo {
            for w in 0..words {
                let v = cones[g[0] as usize * words + w] | cones[g[1] as usize * words + w];
                for o in [g[2], g[3]] {
                    if o != NONE {
                        cones[o as usize * words + w] = v;
                    }
                }
            }
        }
        // the roots by cone
        let mut list: Vec<(Vec<u64>, u32)> = Vec::with_capacity(roots.len());
        for &o in roots {
            self.ck.pay(1 + words as u64)?;
            let Some(at) = net.index_of(o) else { continue };
            let at = at as usize;
            list.push((cones[at * words..(at + 1) * words].to_vec(), at as u32));
        }
        self.ck.pay(1 + list.len() as u64 * (8 + words as u64))?;
        list.sort();
        let mut best = 0i32;
        let mut i = 0;
        while i < list.len() {
            let mut j = i;
            while j < list.len() && list[j].0 == list[i].0 {
                j += 1;
            }
            let x = list[i].0.clone();
            let group: Vec<u32> = list[i..j].iter().map(|c| c.1).collect();
            let got = self.net_cone(net, &cones, words, &x, &group)?;
            best = best.max(got);
            i = j;
        }
        Some(best)
    }

    /// One group of roots with one cone: the wire with the lowest rank.
    fn net_cone(&mut self, net: &NetView, cones: &[u64], words: usize, x: &[u64], group: &[u32]) -> Option<i32> {
        self.ck.pay(1 + net.inputs.len() as u64 + net.lit.len() as u64)?;
        let inputs: Vec<u32> = net.inputs.iter().enumerate().filter(|(i, _)| x[i / 64] >> (i % 64) & 1 == 1).map(|(_, &w)| w).collect();
        let size = inputs.len();
        if size < 3 {
            return Some(0);
        }
        // a guess at each rank: the first m inputs true, m = 1, 2, ...
        let mut rank: Vec<u32> = vec![0; group.len()];
        let mut vals = vec![0u64; net.lit.len()];
        let mut m = 1usize;
        while m <= size {
            let upto = (m + 63).min(size);
            self.ck.pay(1 + net.topo.len() as u64 + net.inputs.len() as u64 + group.len() as u64)?;
            // an input outside the cone stays false
            for &w in &net.inputs {
                vals[w as usize] = 0;
            }
            // the i-th input is true from count i + 1 on
            for (i, &w) in inputs.iter().enumerate() {
                vals[w as usize] = mask_from(m, upto, i + 1);
            }
            run_gates(&net.topo, &mut vals);
            for (k, &w) in group.iter().enumerate() {
                if rank[k] == 0 && vals[w as usize] != 0 {
                    rank[k] = m as u32 + vals[w as usize].trailing_zeros();
                }
            }
            m = upto + 1;
        }
        let best = (0..group.len()).filter(|&k| rank[k] != 0).min_by_key(|&k| (rank[k], net.lit[group[k] as usize]));
        let Some(k) = best else { return Some(0) };
        let (o, t) = (group[k], rank[k] as usize);
        let need = size as i64 - t as i64 + 1;
        if t < 2 || need < 2 || need >= size as i64 {
            return Some(0);
        }
        let olit = net.lit[o as usize];
        if self.ix.unit[li(-olit)] != 2 {
            return Some(0);
        }
        let Some(cid) = self.find(&[-olit])? else { return Some(0) };
        let lits: Vec<i32> = inputs.iter().map(|&w| -net.lit[w as usize]).collect();
        if self.seen_found.contains(&(set_hash(&lits), need as i32, size as u32)) {
            return Some(need as i32);
        }
        // the vectors
        let (nodes0, slots0, leaves0, order0) = (self.rd.v_nodes.len(), self.rd.v_slots.len(), self.rd.v_leaves.len(), self.rd.v_order.len());
        let mut stages: Vec<Vec<u32>> = Vec::new();
        let tree = self.v_build(net, cones, words, &[o], x, &mut stages, &mut vals)?;
        if let Some(top) = tree {
            let slot = self.rd.v_nodes[top as usize].wires.iter().find(|u| u.0 as usize == t && u.1 == olit).map(|u| u.2);
            if let Some(slot) = slot {
                if let Some(written) = self.v_check(net, slot, &stages, nodes0, &mut vals)? {
                    let order_len = self.rd.v_order.len() as u32 - order0 as u32;
                    self.add_found(lits, need as i32, Form::Network, Plan::NetTree { slot, unit: cid as u64 + 1, mult: need, order_start: order0 as u32, order_len }, 2 + written)?;
                    return Some(need as i32);
                }
            }
        }
        self.rd.v_nodes.truncate(nodes0);
        self.rd.v_slots.truncate(slots0);
        self.rd.v_leaves.truncate(leaves0);
        self.rd.v_order.truncate(order0);
        // few inputs: every t-subset runs forward to the wire
        if size <= MAX_T {
            let total = binom(size as u64, t as u64);
            self.ck.pay(1 + total * net.topo.len() as u64)?;
            let mut mask = (1u32 << t) - 1;
            while mask < (1u32 << size) {
                vals.iter_mut().for_each(|v| *v = 0);
                for (i, &w) in inputs.iter().enumerate() {
                    if mask >> i & 1 == 1 {
                        vals[w as usize] = 1;
                    }
                }
                run_gates(&net.topo, &mut vals);
                if vals[o as usize] == 0 {
                    return Some(0);
                }
                mask = next_mask(mask);
            }
            if !self.net_gates_there(net)? {
                return Some(0);
            }
            let at = self.rd.net_subsets.len() as u32;
            let ins: Vec<i32> = inputs.iter().map(|&w| net.lit[w as usize]).collect();
            self.rd.net_subsets.push(NetSubsets { inputs: ins, t: t as u32 });
            let lines = self.rd.net_subset_lines(at);
            self.add_found(lits, need as i32, Form::Network, Plan::NetSubsets { net: at }, lines)?;
            return Some(need as i32);
        }
        Some(0)
    }

    /// Every clause of every gate of the network, which the propagation runs on.
    fn net_gates_there(&mut self, net: &NetView) -> Option<bool> {
        for g in &net.topo {
            if !self.gate_there(net, g)? {
                return Some(false);
            }
        }
        Some(true)
    }

    fn gate_there(&mut self, net: &NetView, g: &[u32; 4]) -> Option<bool> {
        let (a, b) = (net.lit[g[0] as usize], net.lit[g[1] as usize]);
        if g[2] != NONE {
            let h = net.lit[g[2] as usize];
            if self.find(&[-a, h])?.is_none() || self.find(&[-b, h])?.is_none() {
                return Some(false);
            }
        }
        if g[3] != NONE {
            let l = net.lit[g[3] as usize];
            if self.find(&[-a, -b, l])?.is_none() {
                return Some(false);
            }
        }
        Some(true)
    }

    /// A sorted vector over cone x with the wires `want` among its outputs: the
    /// gates from those wires back to where the cone gets smaller, which must be two
    /// vectors over two parts of x. Detection: the ranks are from one run per count,
    /// and `v_check` looks at every case the proof takes.
    #[allow(clippy::too_many_arguments)]
    fn v_build(&mut self, net: &NetView, cones: &[u64], words: usize, want: &[u32], x: &[u64], stages: &mut Vec<Vec<u32>>, vals: &mut [u64]) -> Option<Option<u32>> {
        let size: u32 = x.iter().map(|v| v.count_ones()).sum();
        self.ck.pay(1 + want.len() as u64 * words as u64)?;
        let cone = |w: u32| &cones[w as usize * words..(w as usize + 1) * words];
        if size == 1 {
            // an input: one wire, its own literal
            if want.len() != 1 || net.made[want[0] as usize] != NONE {
                return Some(None);
            }
            let node = self.rd.v_nodes.len() as u32;
            let l = net.lit[want[0] as usize];
            let start = self.rd.v_leaves.len() as u32;
            self.rd.v_leaves.push(l);
            self.rd.v_nodes.push(VNode { leaf_start: start, leaf_len: 1, a: NONE, b: NONE, wires: vec![(1, l, NONE)] });
            stages.push(Vec::new());
            return Some(Some(node));
        }
        // the gates back to where the cone gets smaller
        let mut stage: Vec<u32> = Vec::new();
        let mut cut: Vec<u32> = Vec::new();
        let mut seen: HashSet<u32> = HashSet::new();
        let mut stack: Vec<u32> = want.to_vec();
        while let Some(w) = stack.pop() {
            self.ck.pay(1 + words as u64)?;
            if cone(w) != x {
                cut.push(w);
                continue;
            }
            let g = net.made[w as usize];
            if g == NONE || !seen.insert(g) {
                continue;
            }
            stage.push(g);
            let t = net.topo[g as usize];
            stack.push(t[0]);
            stack.push(t[1]);
        }
        cut.sort_unstable();
        cut.dedup();
        stage.sort_unstable();
        self.ck.pay(1 + 8 * (cut.len() + stage.len()) as u64 + (cut.len() * words) as u64)?;
        // the cut wires by cone: exactly two, apart and together all of x
        let mut parts: Vec<(Vec<u64>, Vec<u32>)> = Vec::new();
        for &w in &cut {
            match parts.iter_mut().find(|p| p.0 == cone(w)) {
                Some(p) => p.1.push(w),
                None => {
                    if parts.len() == 2 {
                        return Some(None);
                    }
                    parts.push((cone(w).to_vec(), vec![w]));
                }
            }
        }
        if parts.len() != 2 {
            return Some(None);
        }
        if parts[0].0.iter().zip(&parts[1].0).any(|(p, q)| p & q != 0) || parts[0].0.iter().zip(&parts[1].0).zip(x).any(|((p, q), v)| (p | q) != *v) {
            return Some(None);
        }
        let Some(a) = self.v_build(net, cones, words, &parts[0].1, &parts[0].0, stages, vals)? else { return Some(None) };
        let Some(b) = self.v_build(net, cones, words, &parts[1].1, &parts[1].0, stages, vals)? else { return Some(None) };
        // the ranks: one run per count of the inputs, the first child filled first. A
        // child's wire of rank r is true from count r on, the other's from na + r on
        let na = self.rd.v_nodes[a as usize].leaf_len as usize;
        let mut from: Vec<(u32, usize)> = Vec::with_capacity(cut.len());
        for (node, start) in [(a, 0usize), (b, na)] {
            let nd = &self.rd.v_nodes[node as usize];
            self.ck.pay(1 + nd.wires.len() as u64)?;
            for u in &nd.wires {
                let Some(w) = net.index_of(u.1) else { return Some(None) };
                from.push((w, u.0 as usize + start));
            }
        }
        let mut ranks: Vec<(u32, i32, u32)> = Vec::with_capacity(want.len());
        let mut left: Vec<u32> = want.to_vec();
        let mut m = 1u32;
        while m <= size && !left.is_empty() {
            let upto = (m + 63).min(size);
            self.ck.pay(1 + stage.len() as u64 + from.len() as u64 + left.len() as u64)?;
            for &(w, f) in &from {
                vals[w as usize] = mask_from(m as usize, upto as usize, f);
            }
            net.run_stage(&stage, vals);
            left.retain(|&w| {
                if vals[w as usize] != 0 {
                    ranks.push((m + vals[w as usize].trailing_zeros(), net.lit[w as usize], 0));
                    false
                } else {
                    true
                }
            });
            m = upto + 1;
        }
        if !left.is_empty() {
            return Some(None);
        }
        ranks.sort_unstable();
        if ranks.windows(2).any(|p| p[0].0 == p[1].0) {
            return Some(None);
        }
        let node = self.rd.v_nodes.len() as u32;
        for r in ranks.iter_mut() {
            r.2 = self.rd.v_slots.len() as u32;
            self.rd.v_slots.push(VSlot { node, t: r.0, lit: r.1, id: 0 });
        }
        let start = self.rd.v_leaves.len() as u32;
        self.ck.pay(size as u64 + words as u64)?;
        for (q, &word) in x.iter().enumerate() {
            let mut rest = word;
            while rest != 0 {
                let i = q * 64 + rest.trailing_zeros() as usize;
                rest &= rest - 1;
                self.rd.v_leaves.push(net.lit[net.inputs[i] as usize]);
            }
        }
        self.rd.v_nodes.push(VNode { leaf_start: start, leaf_len: size, a, b, wires: ranks });
        stages.push(stage);
        Some(Some(node))
    }

    /// The check of a vector's constraint and what it is summed from: the wires and
    /// the gates against the formula, then the run of the gates forward for each case
    /// of each `rup` line. The slots get their order of writing here, children first.
    fn v_check(&mut self, net: &NetView, root: u32, stages: &[Vec<u32>], base: usize, vals: &mut [u64]) -> Option<Option<u64>> {
        let mut written = 0u64;
        let mut stack: Vec<(u32, bool)> = vec![(root, false)];
        let mut done: HashSet<u32> = HashSet::new();
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        let mut gate_ok: HashSet<u32> = HashSet::new();
        while let Some((s, ready)) = stack.pop() {
            self.ck.pay(2)?;
            if ready {
                self.rd.v_order.push(s);
                continue;
            }
            if !done.insert(s) {
                continue;
            }
            stack.push((s, true));
            let (node, t, lit) = {
                let sl = &self.rd.v_slots[s as usize];
                (sl.node as usize, sl.t, sl.lit)
            };
            let (a, b) = (self.rd.v_nodes[node].a, self.rd.v_nodes[node].b);
            let stage = &stages[node - base];
            // the gates of this merge are there
            for &g in stage {
                if gate_ok.insert(g) {
                    self.ck.pay(1)?;
                    if !self.gate_there(net, &net.topo[g as usize])? {
                        return Some(None);
                    }
                }
            }
            // no leaf is the wire
            let (start, len) = (self.rd.v_nodes[node].leaf_start as usize, self.rd.v_nodes[node].leaf_len as usize);
            self.ck.pay(len as u64)?;
            if self.rd.v_leaves[start..start + len].iter().any(|&l| var(l) == var(lit)) {
                return Some(None);
            }
            // every case: a up to p, and what its next wire being false leaves of b
            let (ra, na): (Vec<u32>, u32) = {
                let nd = &self.rd.v_nodes[a as usize];
                (nd.wires.iter().map(|u| u.0).collect(), nd.leaf_len)
            };
            let nb = self.rd.v_nodes[b as usize].leaf_len;
            let floor = t as i64 - nb as i64;
            let mut cases: Vec<u32> = ra.iter().copied().filter(|&q| q <= t && (q as i64) > floor).collect();
            let low = ra.iter().copied().filter(|&q| (q as i64) <= floor).next_back().unwrap_or(0);
            cases.push(low);
            let Some(target) = net.index_of(lit) else { return Some(None) };
            let rb_len = self.rd.v_nodes[b as usize].wires.len();
            self.ck.pay(1 + (cases.len() * (stage.len() + ra.len() + rb_len + 8)) as u64 + (ra.len() * rb_len) as u64)?;
            for &p in &cases {
                let nx = ra.iter().copied().find(|&q| q > p).unwrap_or(na + 1);
                let y_most = t as i64 + 1 - nx as i64;
                self.v_set(net, a, |r| r <= p, vals);
                self.v_set(net, b, |r| (r as i64) <= y_most, vals);
                net.run_stage(stage, vals);
                if vals[target as usize] & 1 == 0 {
                    return Some(None);
                }
            }
            // what it is summed from
            self.rd.v_pairs(s, &mut pairs);
            written += self.rd.v_slot_lines(s);
            for &(x, y) in &pairs {
                for (child, r) in [(a, x), (b, y)] {
                    let c = &self.rd.v_nodes[child as usize];
                    if c.leaf_len > 1 && r <= c.leaf_len {
                        match c.wires.iter().find(|u| u.0 == r) {
                            Some(u) if u.2 != NONE => {
                                if !done.contains(&u.2) {
                                    stack.push((u.2, false));
                                }
                            }
                            _ => return Some(None),
                        }
                    }
                }
            }
        }
        Some(Some(written))
    }

    /// A child's wires for one case of a merge.
    fn v_set(&self, net: &NetView, node: u32, on: impl Fn(u32) -> bool, vals: &mut [u64]) {
        for u in &self.rd.v_nodes[node as usize].wires {
            if let Some(w) = net.index_of(u.1) {
                vals[w as usize] = on(u.0) as u64;
            }
        }
    }
}

/// A comparator network, over wire numbers, its gates in an order of reading.
struct NetView {
    lit: Vec<i32>,
    idx: HashMap<i32, u32>,
    /// Per gate in order: a, b, h, l as wire numbers (h or l may be `NONE`).
    topo: Vec<[u32; 4]>,
    /// Per wire: the gate that makes it.
    made: Vec<u32>,
    reads: Vec<u32>,
    inputs: Vec<u32>,
}

impl NetView {
    fn index_of(&self, l: i32) -> Option<u32> {
        self.idx.get(&l).copied()
    }

    fn run_stage(&self, stage: &[u32], vals: &mut [u64]) {
        for &g in stage {
            let t = self.topo[g as usize];
            let (a, b) = (vals[t[0] as usize], vals[t[1] as usize]);
            if t[2] != NONE {
                vals[t[2] as usize] = a | b;
            }
            if t[3] != NONE {
                vals[t[3] as usize] = a & b;
            }
        }
    }
}

/// The bits for the counts m..=upto, set from count `from` on.
#[inline]
fn mask_from(m: usize, upto: usize, from: usize) -> u64 {
    if from > upto {
        return 0;
    }
    let width = upto - m + 1;
    let all = if width >= 64 { u64::MAX } else { (1u64 << width) - 1 };
    if from <= m {
        all
    } else {
        all & (u64::MAX << (from - m))
    }
}

/// The gates, in their order, forward.
fn run_gates(topo: &[[u32; 4]], vals: &mut [u64]) {
    for t in topo {
        let (a, b) = (vals[t[0] as usize], vals[t[1] as usize]);
        if t[2] != NONE {
            vals[t[2] as usize] = a | b;
        }
        if t[3] != NONE {
            vals[t[3] as usize] = a & b;
        }
    }
}

/// The propagation VeriPB will run on the bound of an adder network: assuming the
/// sum of w ~bit is below k, a bit whose weight is above what is left must be true,
/// and a clause over the ~bits with all but one false forces the last one. True when
/// that ends in a contradiction. `clauses` hold bit indices, each clause saying that
/// one of the ~bits is true.
fn a_bound_closes(weights: &[u64], clauses: &[Vec<u32>], k: u64) -> bool {
    if k == 0 {
        return false;
    }
    // 0 open, 1 ~bit false, 2 ~bit true
    let mut value = vec![0u8; weights.len()];
    let mut slack = k as i64 - 1;
    loop {
        let mut moved = false;
        for (i, &w) in weights.iter().enumerate() {
            if value[i] == 0 && w as i64 > slack {
                value[i] = 1;
                moved = true;
            }
        }
        for c in clauses {
            let mut open = None;
            let mut opens = 0;
            let mut sat = false;
            for &b in c {
                match value[b as usize] {
                    2 => sat = true,
                    0 => {
                        opens += 1;
                        open = Some(b as usize);
                    }
                    _ => {}
                }
            }
            if sat {
                continue;
            }
            if opens == 0 {
                return true;
            }
            if opens == 1 {
                let b = open.unwrap();
                value[b] = 2;
                slack -= weights[b] as i64;
                moved = true;
                if slack < 0 {
                    return true;
                }
            }
        }
        if !moved {
            return false;
        }
    }
}

impl Reader<'_> {
    /// The full adders whose carry is the literal m: a triangle of pairs among
    /// (-a -b), (-a -c), (-b -c).
    fn full_adders_at(&mut self, m: i32, out: &mut Vec<ACell>) -> Option<()> {
        let nt = self.ix.tris(m).len();
        self.ck.pay(1)?;
        if !(3..=MAX_WIRE).contains(&nt) {
            return Some(());
        }
        self.ck.pay((nt * nt) as u64)?;
        let pairs: Vec<[i32; 2]> = self.ix.tris(m).to_vec();
        for i in 0..nt {
            for j in i + 1..nt {
                // the shared literal of the two pairs
                let (pi, pj) = (pairs[i], pairs[j]);
                let shared = if pi[0] == pj[0] || pi[0] == pj[1] {
                    pi[0]
                } else if pi[1] == pj[0] || pi[1] == pj[1] {
                    pi[1]
                } else {
                    continue;
                };
                let p1 = if pi[0] == shared { pi[1] } else { pi[0] };
                let p2 = if pj[0] == shared { pj[1] } else { pj[0] };
                if p1 == p2 || var(p1) == var(p2) {
                    continue;
                }
                // one triangle once: its third pair comes after the first two
                let Some(k) = pairs.iter().position(|q| (q[0] == p1 && q[1] == p2) || (q[0] == p2 && q[1] == p1)) else { continue };
                if k < j {
                    continue;
                }
                self.full_adder(m, -shared, -p1, -p2, out)?;
            }
        }
        Some(())
    }

    /// The half adders around a wire of a full adder or under a unit: making it as
    /// their carry or sum, or reading it. Most of a formula's AND gates are no adders,
    /// so these are looked for only next to something that is.
    fn half_adders_near(&mut self, w: i32, out: &mut Vec<ACell>) -> Option<()> {
        let mut outs: Vec<i32> = Vec::new();
        // as the carry, a + b <= s + 2w: (-a -b w)
        let nt = self.ix.tris(w).len();
        self.ck.pay(1)?;
        if nt <= MAX_WIRE {
            for k in 0..nt {
                let [p, q] = self.ix.tris(w)[k];
                if var(p) != var(q) {
                    self.half_adder(w, -p, -q, out)?;
                    // as the sum: (-a b w) and (a -b w), with a carry over a and b;
                    // either of the pair may be the negative one
                    for (a, b) in [(-p, q), (-q, p)] {
                        self.ck.pay(1)?;
                        if self.find(&[a, -b, w])?.is_some() {
                            self.out(a, b, &mut outs)?;
                            for t in 0..outs.len() {
                                let m = outs[t];
                                if var(m) != var(w) {
                                    self.half_adder(m, a, b, out)?;
                                }
                            }
                        }
                    }
                    // as the sum of a half adder the other way round: (a b w)
                    if self.find(&[-p, -q, w])?.is_some() {
                        let m = self.common_output(p, q, w)?;
                        if m != 0 {
                            self.half_one(m, p, q, out)?;
                        }
                    }
                }
            }
        }
        // as the carry of a half adder the other way round: a and b imply w
        let nb = self.ix.bins(w).len();
        if (2..=MAX_CARRY_PRE).contains(&nb) {
            self.ck.pay((nb * nb) as u64)?;
            for i in 0..nb {
                for j in i + 1..nb {
                    let (a, b) = (-self.ix.bins(w)[i], -self.ix.bins(w)[j]);
                    if var(a) != var(b) {
                        self.half_one(w, a, b, out)?;
                    }
                }
            }
        }
        // reading it: (-w -b m), or w implies m
        let nr = self.ix.tris(-w).len();
        if nr <= MAX_WIRE {
            self.ck.pay(1 + nr as u64)?;
            for k in 0..nr {
                let [p, q] = self.ix.tris(-w)[k];
                for (x, m) in [(p, q), (q, p)] {
                    if var(x) != var(w) && var(m) != var(w) {
                        self.half_adder(m, w, -x, out)?;
                    }
                }
            }
        }
        let nm = self.ix.bins(-w).len();
        if nm <= MAX_CARRY_PRE {
            self.ck.pay(1 + nm as u64)?;
            for k in 0..nm {
                let m = self.ix.bins(-w)[k];
                let np = self.ix.bins(m).len();
                if !(2..=MAX_CARRY_PRE).contains(&np) {
                    continue;
                }
                self.ck.pay(np as u64)?;
                for t in 0..np {
                    let b = -self.ix.bins(m)[t];
                    if b != w && var(b) != var(w) {
                        self.half_one(m, w, b, out)?;
                    }
                }
            }
        }
        Some(())
    }

    /// A full adder with carry m over a, b, c, if a sum is there: the sum comes from
    /// the clause (-a b c s), and its other clauses settle it.
    fn full_adder(&mut self, m: i32, a: i32, b: i32, c: i32, out: &mut Vec<ACell>) -> Option<()> {
        let nl = self.ix.longs(-a).len();
        if nl > 8 * MAX_WIRE {
            return self.ck.pay(1);
        }
        self.ck.pay(1 + nl as u64)?;
        for k in 0..nl {
            let cl = self.ix.f.clause(self.ix.longs(-a)[k] as usize);
            if cl.len() != 4 || !cl.contains(&b) || !cl.contains(&c) {
                continue;
            }
            let Some(&s) = cl.iter().find(|&&l| l != -a && l != b && l != c) else { continue };
            if self.find(&[a, -b, c, s])?.is_some() && self.find(&[a, b, -c, s])?.is_some() && self.find(&[-a, -b, -c, s])?.is_some() {
                out.push(ACell { kind: CellKind::Full, a, b, c, s, m });
                return Some(());
            }
        }
        Some(())
    }

    /// A half adder with carry m over a and b: the sum from (-a b s), checked by
    /// (a -b s).
    fn half_adder(&mut self, m: i32, a: i32, b: i32, out: &mut Vec<ACell>) -> Option<()> {
        let nt = self.ix.tris(-a).len();
        if nt > MAX_WIRE {
            return self.ck.pay(1);
        }
        self.ck.pay(1 + nt as u64)?;
        for k in 0..nt {
            let [p, q] = self.ix.tris(-a)[k];
            let s = if p == b {
                q
            } else if q == b {
                p
            } else {
                continue;
            };
            if self.find(&[a, -b, s])?.is_some() {
                out.push(ACell { kind: CellKind::Half, a, b, c: 0, s, m });
                return Some(());
            }
        }
        Some(())
    }

    /// a + b + 1 <= s + 2m: m follows from a and from b, and s = xnor from (a b s)
    /// and (-a -b s).
    fn half_one(&mut self, m: i32, a: i32, b: i32, out: &mut Vec<ACell>) -> Option<()> {
        let nt = self.ix.tris(a).len();
        if nt > MAX_WIRE {
            return self.ck.pay(1);
        }
        self.ck.pay(1 + nt as u64)?;
        for k in 0..nt {
            let [p, q] = self.ix.tris(a)[k];
            let s = if p == b {
                q
            } else if q == b {
                p
            } else {
                continue;
            };
            if self.find(&[-a, -b, s])?.is_some() {
                out.push(ACell { kind: CellKind::HalfOne, a, b, c: 0, s, m });
                return Some(());
            }
        }
        Some(())
    }

    /// All cells in the formula, then the networks they make, each checked and read
    /// for a bound on its bits.
    fn read_adders(&mut self) -> Option<()> {
        let f = self.ix.f;
        let nl = self.ix.unit.len();
        let mut cells: Vec<ACell> = Vec::new();
        for i in 2..nl {
            let m = if i & 1 == 1 { -((i >> 1) as i32) } else { (i >> 1) as i32 };
            self.full_adders_at(m, &mut cells)?;
        }
        // the half adders next to those, and next to what a unit fixes; a network of
        // half adders alone is found from a unit on one of its bits
        self.ck.pay(nl as u64 + f.len() as u64)?;
        let mut looked = vec![false; nl];
        let mut stack: Vec<i32> = Vec::new();
        for i in 0..f.len() {
            self.ck.poll(i)?;
            if self.ix.uniq[i] && f.start[i + 1] - f.start[i] == 1 {
                let u = f.clause(i)[0];
                if !self.ix.tris(-u).is_empty() || !self.ix.tris(u).is_empty() {
                    stack.push(-u);
                    stack.push(u);
                }
            }
        }
        let mut done = 0;
        loop {
            self.ck.pay(1 + (cells.len() - done) as u64 * 5)?;
            for c in &cells[done..] {
                for l in [c.a, c.b, c.c, c.s, c.m] {
                    if l != 0 {
                        stack.push(l);
                    }
                }
            }
            done = cells.len();
            let Some(w) = stack.pop() else { break };
            if !looked[li(w)] {
                looked[li(w)] = true;
                self.half_adders_near(w, &mut cells)?;
            }
        }
        if cells.is_empty() {
            return Some(());
        }
        // a wire is made once: the first cell that makes it keeps it, a full adder
        // before a half adder over the same wires
        self.ck.pay(1 + 4 * nl as u64 + 8 * cells.len() as u64)?;
        cells.sort_by_key(|c| c.kind != CellKind::Full);
        let mut made = vec![NONE; nl];
        let mut keep: Vec<ACell> = Vec::with_capacity(cells.len());
        for c in cells {
            let (ms, mm) = (li(c.s), li(c.m));
            let distinct = {
                let v = [var(c.a), var(c.b), if c.c != 0 { var(c.c) } else { 0 }, var(c.s), var(c.m)];
                (0..5).all(|i| (0..i).all(|j| v[i] != v[j] || v[i] == 0))
            };
            if !distinct || made[ms] != NONE || made[mm] != NONE {
                continue;
            }
            let at = keep.len() as u32;
            made[ms] = at;
            made[mm] = at;
            keep.push(c);
        }
        // union by the wires one cell makes and another reads
        let count = keep.len();
        let mut parent: Vec<u32> = (0..count as u32).collect();
        fn root(parent: &mut [u32], mut x: u32) -> u32 {
            while parent[x as usize] != x {
                let up = parent[parent[x as usize] as usize];
                parent[x as usize] = up;
                x = up;
            }
            x
        }
        let mut reads = vec![0u8; nl];
        for (i, c) in keep.iter().enumerate() {
            for l in [c.a, c.b, c.c] {
                if l == 0 {
                    continue;
                }
                reads[li(l)] = reads[li(l)].saturating_add(1);
                let d = made[li(l)];
                if d != NONE {
                    let (x, y) = (root(&mut parent, i as u32), root(&mut parent, d));
                    parent[x as usize] = y;
                }
            }
        }
        let mut roots: Vec<u32> = Vec::with_capacity(count);
        for i in 0..count {
            roots.push(root(&mut parent, i as u32));
        }
        // the cells of each network, in the order of their first cell
        let mut size = vec![0u32; count + 1];
        for &r in &roots {
            size[r as usize + 1] += 1;
        }
        for i in 0..count {
            size[i + 1] += size[i];
        }
        let mut members = vec![0u32; count];
        let mut fill = size.clone();
        for (i, &r) in roots.iter().enumerate() {
            members[fill[r as usize] as usize] = i as u32;
            fill[r as usize] += 1;
        }
        for r in 0..count {
            let (a, b) = (size[r] as usize, size[r + 1] as usize);
            if b - a >= 1 {
                self.check_network(&keep, &members[a..b], &made, &reads)?;
            }
        }
        Some(())
    }

    /// The check of one adder network: the plan alone, against the formula. Every
    /// wire read once, at the weight it was made at; every input once, at weight 0;
    /// every clause of every cell there; and the bound on the bits closes by
    /// propagation.
    fn check_network(&mut self, keep: &[ACell], members: &[u32], made: &[u32], reads: &[u8]) -> Option<()> {
        let count = members.len();
        self.ck.pay(1 + 8 * count as u64)?;
        // an order with every cell after those whose wires it reads, and the weights
        let mut order: Vec<(ACell, u32)> = Vec::with_capacity(count);
        let mut wire_w: HashMap<i32, u32> = HashMap::with_capacity(2 * count);
        let mut indeg: HashMap<u32, u32> = HashMap::with_capacity(count);
        let mut users: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut ready: Vec<u32> = Vec::new();
        for &ci in members {
            let c = &keep[ci as usize];
            let mut d = 0;
            for l in [c.a, c.b, c.c] {
                if l != 0 && made[li(l)] != NONE {
                    d += 1;
                    users.entry(made[li(l)]).or_default().push(ci);
                }
            }
            indeg.insert(ci, d);
            if d == 0 {
                ready.push(ci);
            }
        }
        let mut qi = 0;
        let mut inputs: Vec<i32> = Vec::new();
        while qi < ready.len() {
            let ci = ready[qi];
            qi += 1;
            let c = keep[ci as usize];
            let mut wt = None;
            for l in [c.a, c.b, c.c] {
                if l == 0 {
                    continue;
                }
                let lw = if made[li(l)] != NONE {
                    // a wire read twice counts twice: no count. (Both readers are in
                    // this network, by the way the networks were made.)
                    if reads[li(l)] != 1 {
                        return Some(());
                    }
                    let Some(&x) = wire_w.get(&l) else { return Some(()) };
                    x
                } else {
                    inputs.push(l);
                    0
                };
                match wt {
                    None => wt = Some(lw),
                    Some(x) if x == lw => {}
                    _ => return Some(()),
                }
            }
            let wt = wt.unwrap_or(0);
            if wt as usize >= MAX_BITS {
                return Some(());
            }
            wire_w.insert(c.s, wt);
            wire_w.insert(c.m, wt + 1);
            order.push((c, wt));
            if let Some(list) = users.get(&ci) {
                for &u in list {
                    let e = indeg.get_mut(&u).unwrap();
                    *e -= 1;
                    if *e == 0 {
                        ready.push(u);
                    }
                }
            }
        }
        if order.len() != count {
            return Some(()); // cyclic
        }
        let n = inputs.len();
        if n < 3 {
            return Some(());
        }
        // the bits nothing reads, the highest weight first
        let mut bits: Vec<(i32, u32)> = Vec::new();
        for &(c, wt) in &order {
            for (l, lw) in [(c.s, wt), (c.m, wt + 1)] {
                if reads[li(l)] == 0 {
                    bits.push((l, lw));
                }
            }
        }
        if bits.len() > MAX_BITS {
            return Some(());
        }
        bits.sort_by_key(|&(l, w)| (std::cmp::Reverse(w), l));
        // every variable in one place only
        self.next_mark();
        self.ck.pay(1 + inputs.len() as u64 + 2 * count as u64)?;
        for &x in &inputs {
            if self.marked(x) {
                return Some(());
            }
            self.set_mark(x);
        }
        for &(c, _) in &order {
            for l in [c.s, c.m] {
                if self.marked(l) {
                    return Some(());
                }
                self.set_mark(l);
            }
        }
        // the clauses over the ~bits: a unit on ~bit, or a clause of nothing but ~bits
        let weights: Vec<u64> = bits.iter().map(|&(_, w)| 1u64 << w).collect();
        let mut clauses: Vec<Vec<u32>> = Vec::new();
        self.ck.pay(bits.len() as u64)?;
        for j in 0..bits.len() {
            let o = -bits[j].0;
            if self.ix.unit[li(o)] != 0 {
                clauses.push(vec![j as u32]);
            }
            let (nb, nt, nl) = (self.ix.bins(o).len(), self.ix.tris(o).len(), self.ix.longs(o).len());
            if nb + nt + nl > MAX_DEG {
                continue;
            }
            self.ck.pay(1 + (bits.len() * (nb + 2 * nt)) as u64)?;
            let at = |l: i32| bits.iter().position(|&(b, _)| -b == l).map(|p| p as u32);
            for &q in self.ix.bins(o) {
                if let Some(p) = at(q) {
                    if p as usize > j {
                        clauses.push(vec![j as u32, p]);
                    }
                }
            }
            for &[p, q] in self.ix.tris(o) {
                if let (Some(a), Some(b)) = (at(p), at(q)) {
                    if a as usize > j && b as usize > j {
                        clauses.push(vec![j as u32, a, b]);
                    }
                }
            }
            for k in 0..nl {
                let cl = self.ix.f.clause(self.ix.longs(o)[k] as usize);
                self.ck.pay((cl.len() * bits.len()) as u64)?;
                let idx: Vec<Option<u32>> = cl.iter().map(|&l| at(l)).collect();
                if idx.iter().all(|x| x.is_some()) && idx.iter().flatten().all(|&x| x as usize >= j) {
                    clauses.push(idx.into_iter().flatten().collect());
                }
            }
        }
        if clauses.is_empty() {
            return Some(());
        }
        // the least the number on the ~bits can be: from the top bit down, false if no
        // clause then has nothing left
        let mut value = vec![0u8; bits.len()]; // 0 open, 1 false, 2 true
        let mut k = 0u64;
        self.ck.pay(1 + (bits.len() * clauses.len()) as u64)?;
        for j in 0..bits.len() {
            value[j] = 1;
            let forced = clauses.iter().any(|c| c.contains(&(j as u32)) && c.iter().all(|&b| value[b as usize] == 1));
            if forced {
                value[j] = 2;
                k += weights[j];
            }
        }
        // that is the least when no two bits have one weight; otherwise the largest number
        // the propagation closes on, which goes down with the number asked for
        self.ck.pay(1 + 64 * (bits.len() * clauses.len()) as u64)?;
        if !a_bound_closes(&weights, &clauses, k) {
            let (mut lo, mut hi) = (0u64, k);
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if a_bound_closes(&weights, &clauses, mid) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            k = lo;
        }
        if k == 0 {
            return Some(());
        }
        // what comes out: sum of ~inputs >= n + constants + k - W
        let w_total: u64 = weights.iter().sum();
        let constants: u64 = order.iter().map(|&(c, wt)| if c.kind == CellKind::HalfOne { 1u64 << wt } else { 0 }).sum();
        let need = n as i64 + constants as i64 + k as i64 - w_total as i64;
        if need < 2 || need >= n as i64 {
            return Some(());
        }
        let lits: Vec<i32> = inputs.iter().map(|&x| -x).collect();
        if self.seen_found.contains(&(set_hash(&lits), need as i32, n as u32)) {
            return Some(());
        }
        // every clause of every cell
        self.ck.pay(4 * count as u64)?;
        for &(c, _) in &order {
            let ok = match c.kind {
                CellKind::Full => {
                    let (a, b, d, s, m) = (c.a, c.b, c.c, c.s, c.m);
                    let want: [&[i32]; 7] = [&[-a, -b, m], &[-a, -d, m], &[-b, -d, m], &[-a, b, d, s], &[a, -b, d, s], &[a, b, -d, s], &[-a, -b, -d, s]];
                    let mut all = true;
                    for q in want {
                        if self.find(q)?.is_none() {
                            all = false;
                            break;
                        }
                    }
                    all
                }
                CellKind::Half => {
                    let (a, b, s, m) = (c.a, c.b, c.s, c.m);
                    self.find(&[-a, -b, m])?.is_some() && self.find(&[-a, b, s])?.is_some() && self.find(&[a, -b, s])?.is_some()
                }
                CellKind::HalfOne => {
                    let (a, b, s, m) = (c.a, c.b, c.s, c.m);
                    self.find(&[a, b, s])?.is_some() && self.find(&[-a, -b, s])?.is_some() && self.find(&[-a, m])?.is_some() && self.find(&[-b, m])?.is_some()
                }
            };
            if !ok {
                return Some(());
            }
        }
        // the clauses the bound relies on were read from the index, so they are there
        let net = self.rd.a_nets.len() as u32;
        self.rd.a_nets.push(ANet { cells: order, bits, k });
        let lines = self.rd.a_lines(net);
        self.add_found(lits, need as i32, Form::Adder, Plan::Adder { net }, lines)?;
        Some(())
    }
}

/// One node of a diagram being read.
struct DNode {
    lit: i32,
    /// The decision variable, 0 while unread (a child no node defines is a literal).
    x: i32,
    /// The children when x is true and when it is false: literals, or `T_KID` and
    /// `F_KID`.
    hi: i32,
    lo: i32,
    /// The decision variables of the parents that reached it, in `Reader`'s list.
    came: u32,
    level: u32,
    /// Its clauses are no node's.
    no: bool,
}

/// What the clauses of a literal say when it is read as a node.
enum DRead {
    Node { x: i32, hi: i32, lo: i32, strong: bool },
    Wait,
    No,
}

/// A child, once the reading is finished.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DKid {
    /// Nothing is asked for: no clause is needed.
    True,
    /// Nothing can be: the clause is (-n v y) or (-n v -y).
    False,
    /// The clause holds a literal a unit makes false.
    Dead(i32),
    /// Nothing can be, by a line of its own: (-n v y), y being the literal given.
    Lemma(i32),
    /// A literal no node defines: an input from a later level.
    Lit(i32),
    Node(u32),
}

/// A literal in the node's own sign before one in the other, then the nearest
/// number, then the smaller: encoders number and sign the nodes of a diagram
/// together. Only a guess: a wrong one loses a count.
fn d_pick(opts: impl Iterator<Item = i32>, n: i32, not: i32) -> Option<i32> {
    opts.filter(|&l| l != not).min_by_key(|&l| ((l > 0) != (n > 0), (l.unsigned_abs() as i64 - n.unsigned_abs() as i64).abs(), l))
}

impl Reader<'_> {
    /// The clauses that hold -n, read as a node. `came` are the variables of its
    /// parents: in a diagram written both ways a node's clauses to its parents look
    /// like a node of their own. An if-then-else with two children is read at
    /// once. The other shapes (a false child, a true child) need to know the
    /// decision variable from elsewhere, so they wait, unless `forced`.
    fn d_classify(&mut self, ni: usize, forced: bool) -> Option<DRead> {
        let n = self.dn[ni].lit;
        let (nb, nt) = (self.ix.bins(-n).len(), self.ix.tris(-n).len());
        self.ck.pay(1)?;
        // a variable another node decides on is an input
        if nb + nt == 0 || nb + nt > MAX_DEG || self.d_known[var(n)] == self.d_epoch {
            return Some(DRead::No);
        }
        let mut came: Vec<i32> = Vec::new();
        let mut at = self.dn[ni].came;
        while at != NONE {
            let (x, next) = self.d_came[at as usize];
            came.push(x);
            at = next;
        }
        // the clauses looked at, and their entries sorted
        self.ck.pay(1 + (came.len() as u64 + 2 + 2 * (usize::BITS - nt.leading_zeros()) as u64) * nt as u64)?;
        let fresh = |l: i32, came: &[i32]| !came.contains(&(l.abs()));
        // (variable, its sign in the clause, the other literal)
        let mut e: Vec<(i32, bool, i32)> = Vec::with_capacity(2 * nt);
        let mut rest_t = 0usize;
        let mut first_t = (0, 0);
        for &[p, q] in self.ix.tris(-n) {
            if fresh(p, &came) && fresh(q, &came) {
                e.push((p.abs(), p > 0, q));
                e.push((q.abs(), q > 0, p));
                if rest_t == 0 {
                    first_t = (p, q);
                }
                rest_t += 1;
            }
        }
        e.sort_unstable();
        // the variables with both signs: if-then-elses
        let mut strong: Vec<(i32, i32, i32)> = Vec::new();
        let mut comp: Vec<(i32, i32, i32)> = Vec::new();
        let mut i = 0;
        while i < e.len() {
            let var = e[i].0;
            let mut j = i;
            while j < e.len() && e[j].0 == var {
                j += 1;
            }
            let group = &e[i..j];
            let lows = || group.iter().filter(|g| g.1).map(|g| g.2);
            let highs = || group.iter().filter(|g| !g.1).map(|g| g.2);
            if let Some(h0) = d_pick(highs(), n, 0) {
                if d_pick(lows(), n, 0).is_some() {
                    match d_pick(lows(), n, -h0) {
                        Some(l) => strong.push((var, h0, l)),
                        None => {
                            // the false side has only -h0: another true side?
                            let l0 = -h0;
                            match d_pick(highs(), n, -l0) {
                                Some(h1) => strong.push((var, h1, l0)),
                                None => comp.push((var, h0, l0)),
                            }
                        }
                    }
                }
            }
            i = j;
        }
        let known = |this: &Self, x: i32| this.d_known[x as usize] == this.d_epoch;
        if strong.len() == 1 {
            let (x, hi, lo) = strong[0];
            return Some(DRead::Node { x, hi, lo, strong: true });
        }
        if strong.len() > 1 {
            if !forced {
                return Some(DRead::Wait);
            }
            let pick = strong.iter().find(|s| known(self, s.0)).copied().unwrap_or(strong[0]);
            return Some(DRead::Node { x: pick.0, hi: pick.1, lo: pick.2, strong: true });
        }
        // x true gives one child and x false the other's negation: one of them says
        // what the node says
        if let Some(&(x, hi, lo)) = comp.iter().find(|s| known(self, s.0)) {
            return Some(DRead::Node { x, hi, lo, strong: false });
        }
        self.ck.pay(1 + (came.len() as u64 + 1) * nb as u64)?;
        let rb: Vec<i32> = self.ix.bins(-n).iter().copied().filter(|&l| fresh(l, &came)).collect();
        // a binary clause on x, and x's other sign with a child: a false child
        let mut s2: Option<(i32, i32, i32)> = None;
        let mut s2_count = 0;
        self.ck.pay(1 + (rb.len() * e.len()) as u64)?;
        for &b in &rb {
            let x = b.abs();
            let others = e.iter().filter(|g| g.0 == x && g.1 != (b > 0)).map(|g| g.2);
            if let Some(z) = d_pick(others, n, 0) {
                s2_count += 1;
                let c = if b < 0 { (x, F_KID, z) } else { (x, z, F_KID) };
                if s2.is_none() || known(self, x) {
                    s2 = Some(c);
                }
            }
        }
        if let Some((x, hi, lo)) = s2 {
            if known(self, x) || s2_count == 1 {
                return Some(DRead::Node { x, hi, lo, strong: true });
            }
        }
        // two binary clauses and nothing else: x and a child that holds whatever x is
        if rb.len() == 2 && rest_t == 0 {
            let (a, b) = (rb[0], rb[1]);
            let read = |p: i32, q: i32| if p < 0 { (p.abs(), F_KID, q) } else { (p.abs(), q, F_KID) };
            for (p, q) in [(a, b), (b, a)] {
                if known(self, p.abs()) {
                    let (x, hi, lo) = read(p, q);
                    return Some(DRead::Node { x, hi, lo, strong: false });
                }
            }
            if !forced {
                return Some(DRead::Wait);
            }
            // the input is the variable in more clauses
            let deg = |this: &Self, l: i32| this.ix.degree(l) + this.ix.degree(-l);
            let (p, q) = if (deg(self, a), -(a.abs())) >= (deg(self, b), -(b.abs())) { (a, b) } else { (b, a) };
            let (x, hi, lo) = read(p, q);
            return Some(DRead::Node { x, hi, lo, strong: false });
        }
        // one clause: a true child on one side
        if rest_t == 1 && rb.is_empty() {
            let (p, q) = first_t;
            let read = |p: i32, q: i32| if p < 0 { (p.abs(), q, T_KID) } else { (p.abs(), T_KID, q) };
            for (u, v) in [(p, q), (q, p)] {
                if known(self, u.abs()) {
                    let (x, hi, lo) = read(u, v);
                    return Some(DRead::Node { x, hi, lo, strong: false });
                }
            }
            if !forced {
                return Some(DRead::Wait);
            }
            // encoders number the nodes of a diagram together
            let far = |l: i32| (l.unsigned_abs() as i64 - n.unsigned_abs() as i64).abs();
            let (u, v) = if (far(p), -(p.abs())) >= (far(q), -(q.abs())) { (p, q) } else { (q, p) };
            let (x, hi, lo) = read(u, v);
            return Some(DRead::Node { x, hi, lo, strong: false });
        }
        if rb.len() == 1 && rest_t == 0 {
            let b = rb[0];
            let (hi, lo) = if b < 0 { (F_KID, T_KID) } else { (T_KID, F_KID) };
            return Some(DRead::Node { x: b.abs(), hi, lo, strong: false });
        }
        if !forced {
            return Some(DRead::Wait);
        }
        if let (Some(&(x, hi, lo)), true) = (comp.first(), rb.is_empty()) {
            return Some(DRead::Node { x, hi, lo, strong: false });
        }
        Some(DRead::No)
    }

    /// (-n v y) by propagation, for a node with (-n v y v not z): from n and not y,
    /// not z; the node's level has an at-most-one with n, so its other nodes w are
    /// false; a clause holding z has every other literal q false, by (-q v y v w), or
    /// (-q v y), or a unit. That is a contradiction, and every clause of it is looked
    /// up here.
    fn d_edge(&mut self, n: i32, y: i32, z: i32) -> Option<bool> {
        if self.find(&[-n, y, -z])?.is_none() {
            return Some(false);
        }
        self.d_big_index()?;
        // the clauses that hold z: binary, ternary, long, longer
        let mut lists: Vec<Vec<i32>> = Vec::new();
        self.ck.pay(1 + self.ix.bins(z).len() as u64 + self.ix.tris(z).len() as u64)?;
        if self.ix.bins(z).len() > MAX_DEG {
            return Some(false);
        }
        for &q in self.ix.bins(z) {
            lists.push(vec![q]);
        }
        for &[p, q] in self.ix.tris(z) {
            lists.push(vec![p, q]);
        }
        let nl = self.ix.longs(z).len();
        for k in 0..nl {
            let c = self.ix.f.clause(self.ix.longs(z)[k] as usize);
            self.ck.pay(c.len() as u64)?;
            lists.push(c.iter().copied().filter(|&l| l != z).collect());
        }
        let mut at = self.d_big_head[li(z)];
        while at != NONE {
            let (ci, next) = self.d_big[at as usize];
            at = next;
            let c = self.ix.f.clause(ci as usize);
            self.ck.pay(c.len() as u64)?;
            lists.push(c.iter().copied().filter(|&l| l != z).collect());
        }
        'clauses: for c in &lists {
            for &q in c {
                if self.ix.unit[li(-q)] != 0 || self.has_bin(-q, y)? {
                    continue;
                }
                let nt = self.ix.tris(-q).len();
                self.ck.pay(1 + nt as u64)?;
                if nt > MAX_DEG {
                    continue 'clauses;
                }
                let mut found = false;
                for k in 0..nt {
                    let [p, r] = self.ix.tris(-q)[k];
                    let w = if p == y {
                        r
                    } else if r == y {
                        p
                    } else {
                        continue;
                    };
                    if var(w) != var(n) && (self.ix.unit[li(-w)] != 0 || self.has_bin(-n, -w)?) {
                        found = true;
                        break;
                    }
                }
                if !found {
                    continue 'clauses;
                }
            }
            return Some(true);
        }
        Some(false)
    }

    /// Per literal, the clauses longer than the lookup table holds. Only an exact grid
    /// asks, and it is made once.
    fn d_big_index(&mut self) -> Option<()> {
        if !self.d_big_head.is_empty() {
            return Some(());
        }
        let nl = self.ix.unit.len();
        self.ck.pay(nl as u64 + self.ix.longer.len() as u64)?;
        self.d_big_head = vec![NONE; nl];
        for k in 0..self.ix.longer.len() {
            let i = self.ix.longer[k];
            let c = self.ix.f.clause(i as usize);
            self.ck.pay(c.len() as u64)?;
            for &l in c {
                self.d_big.push((i, self.d_big_head[li(l)]));
                self.d_big_head[li(l)] = self.d_big.len() as u32 - 1;
            }
        }
        Some(())
    }

    /// A literal to read as a node, reached by a parent deciding on `x`.
    fn d_enqueue(&mut self, l: i32, x: i32, queue: &mut std::collections::VecDeque<u32>) -> Option<()> {
        self.ck.pay(1)?;
        let at = li(l);
        let ni = if self.d_at_stamp[at] == self.d_epoch {
            self.d_at[at] as usize
        } else {
            let ni = self.dn.len();
            self.d_at_stamp[at] = self.d_epoch;
            self.d_at[at] = ni as u32;
            self.dn.push(DNode { lit: l, x: 0, hi: T_KID, lo: T_KID, came: NONE, level: 0, no: false });
            queue.push_back(ni as u32);
            ni
        };
        if x != 0 && self.dn[ni].x == 0 {
            self.d_came.push((x, self.dn[ni].came));
            self.dn[ni].came = self.d_came.len() as u32 - 1;
        }
        Some(())
    }

    /// A child as the check will see it: a unit makes a literal a terminal, and a
    /// literal of a variable some node has in either sign asks for nothing.
    fn d_kid(&self, k: i32) -> DKid {
        if k == T_KID {
            return DKid::True;
        }
        if k == F_KID {
            return DKid::False;
        }
        if self.ix.unit[li(k)] != 0 {
            return DKid::True;
        }
        if self.ix.unit[li(-k)] != 0 {
            return DKid::Dead(k);
        }
        if self.d_at_stamp[li(k)] == self.d_epoch {
            let c = self.d_at[li(k)];
            if self.dn[c as usize].x != 0 {
                return DKid::Node(c);
            }
        }
        if self.d_at_stamp[li(-k)] == self.d_epoch && self.dn[self.d_at[li(-k)] as usize].x != 0 {
            return DKid::True;
        }
        DKid::Lit(k)
    }

    /// A diagram from an asserted literal: every node it reaches, then the variable
    /// order, then a count for each sign of the inputs that holds one.
    fn read_diagram(&mut self, root: i32) -> Option<()> {
        if self.ix.tris(-root).is_empty() {
            return Some(());
        }
        self.d_epoch = self.d_epoch.wrapping_add(1);
        self.dn.clear();
        self.d_came.clear();
        let mut queue: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
        self.d_enqueue(root, 0, &mut queue)?;
        let mut stalled = 0usize;
        let mut strong = 0usize;
        while let Some(ni) = queue.pop_front() {
            let ni = ni as usize;
            let forced = stalled > queue.len();
            match self.d_classify(ni, forced)? {
                DRead::Wait => {
                    stalled += 1;
                    queue.push_back(ni as u32);
                }
                DRead::No => {
                    stalled = 0;
                    self.dn[ni].no = true;
                }
                DRead::Node { x, hi, lo, strong: s } => {
                    stalled = 0;
                    strong += s as usize;
                    if self.d_is_node(x, s) {
                        self.dn[ni].no = true;
                        continue;
                    }
                    let nd = &mut self.dn[ni];
                    nd.x = x;
                    nd.hi = hi;
                    nd.lo = lo;
                    self.d_known[x as usize] = self.d_epoch;
                    let lit = nd.lit;
                    // of a child and its negation only the one in the node's sign
                    // can be a node
                    let both = hi != T_KID && hi != F_KID && hi == lo.wrapping_neg();
                    for k in [hi, lo] {
                        if k == T_KID || k == F_KID {
                            continue;
                        }
                        if both && (k > 0) != (lit > 0) {
                            continue;
                        }
                        if self.ix.unit[li(k)] == 0 && self.ix.unit[li(-k)] == 0 && self.d_known[var(k)] != self.d_epoch {
                            // the same nodes the other way round: a diagram read before
                            // has said both of its bounds
                            if self.d_said[li(-k)] {
                                return Some(());
                            }
                            self.d_enqueue(k, x, &mut queue)?;
                        }
                    }
                }
            }
        }
        if strong == 0 {
            return Some(()); // a chain of clauses: a one-way counter's walk reads those
        }
        let before = self.rd.found.len();
        self.finish_diagram(root)?;
        if self.rd.found.len() > before {
            self.ck.pay(self.dn.len() as u64)?;
            for k in 0..self.dn.len() {
                if self.dn[k].x != 0 {
                    self.d_said[li(self.dn[k].lit)] = true;
                }
            }
        }
        Some(())
    }

    /// A variable decided on is no node. A clear if-then-else settles that against a
    /// child that is still only waiting to be read; a weaker shape does not.
    fn d_is_node(&self, x: i32, strong: bool) -> bool {
        let known = self.d_known[x as usize] == self.d_epoch;
        for l in [x, -x] {
            if self.d_at_stamp[li(l)] == self.d_epoch {
                let d = &self.dn[self.d_at[li(l)] as usize];
                if d.x != 0 || (!strong && !known && !d.no) {
                    return true;
                }
            }
        }
        false
    }

    /// The order of the levels, the sign of each input by what gives the root most,
    /// then the two counts.
    fn finish_diagram(&mut self, root: i32) -> Option<()> {
        let count = self.dn.len();
        self.ck.pay(1 + 4 * count as u64)?;
        // a level per decision variable and per literal child
        let mut level_vars: Vec<i32> = Vec::new();
        let mut kids: Vec<[DKid; 2]> = vec![[DKid::True; 2]; count];
        for ni in 0..count {
            if self.dn[ni].x == 0 {
                continue;
            }
            let k = [self.d_kid(self.dn[ni].hi), self.d_kid(self.dn[ni].lo)];
            kids[ni] = k;
            let x = self.dn[ni].x;
            self.d_new_level(x, &mut level_vars);
            for c in k {
                if let DKid::Lit(l) = c {
                    self.d_new_level(l.abs(), &mut level_vars);
                }
            }
        }
        // an exact grid's edge: x true gives z and x false gives not z, and the rest of
        // the formula rules out the second
        self.ck.pay(1 + count as u64)?;
        for ni in 0..count {
            let (x, hi, lo, lit) = (self.dn[ni].x, self.dn[ni].hi, self.dn[ni].lo, self.dn[ni].lit);
            if x == 0 || hi == T_KID || hi == F_KID || lo == T_KID || lo == F_KID || hi != -lo {
                continue;
            }
            if self.d_edge(lit, x, hi)? {
                kids[ni][1] = DKid::Lemma(x);
            } else if self.d_edge(lit, -x, lo)? {
                kids[ni][0] = DKid::Lemma(-x);
            }
        }
        let levels = level_vars.len();
        // no variable is both a node and an input
        for ni in 0..count {
            if self.dn[ni].x != 0 && self.d_level_stamp[var(self.dn[ni].lit)] == self.d_epoch {
                return Some(());
            }
        }
        // the edges: a node's variable before the variables of its children
        self.ck.pay(4 + 4 * count as u64 + 2 * levels as u64)?;
        let mut head = vec![NONE; levels];
        let mut next: Vec<(u32, u32)> = Vec::with_capacity(2 * count);
        let mut indeg = vec![0u32; levels];
        for ni in 0..count {
            if self.dn[ni].x == 0 {
                continue;
            }
            let from = self.d_level[self.dn[ni].x as usize] as usize;
            for c in kids[ni] {
                let to = match c {
                    DKid::Lit(l) => self.d_level[l.unsigned_abs() as usize] as usize,
                    DKid::Node(q) => self.d_level[self.dn[q as usize].x as usize] as usize,
                    _ => continue,
                };
                next.push((to as u32, head[from]));
                head[from] = next.len() as u32 - 1;
                indeg[to] += 1;
            }
        }
        // Kahn: the first level the one nothing comes before
        let mut order: Vec<u32> = Vec::with_capacity(levels);
        for (l, &d) in indeg.iter().enumerate() {
            if d == 0 {
                order.push(l as u32);
            }
        }
        let mut qi = 0;
        while qi < order.len() {
            let mut at = head[order[qi] as usize];
            qi += 1;
            while at != NONE {
                let (to, nx) = next[at as usize];
                at = nx;
                indeg[to as usize] -= 1;
                if indeg[to as usize] == 0 {
                    order.push(to);
                }
            }
        }
        if order.len() != levels {
            return Some(()); // no variable order
        }
        // the tables of the levels and the nodes by level, before they are made
        self.ck.pay(8 * levels as u64 + 6 * count as u64)?;
        let mut rank = vec![0u32; levels];
        for (r, &l) in order.iter().enumerate() {
            rank[l as usize] = r as u32;
        }
        let vars: Vec<i32> = order.iter().map(|&l| level_vars[l as usize]).collect();
        // the nodes by level, the last level first
        let mut by_level: Vec<u32> = Vec::with_capacity(count);
        let mut start = vec![0u32; levels + 1];
        for ni in 0..count {
            if self.dn[ni].x != 0 {
                let r = rank[self.d_level[self.dn[ni].x as usize] as usize];
                self.dn[ni].level = r;
                start[r as usize + 1] += 1;
            }
        }
        for r in 0..levels {
            start[r + 1] += start[r];
        }
        by_level.resize(start[levels] as usize, 0);
        let mut fill = start.clone();
        for ni in 0..count {
            if self.dn[ni].x != 0 {
                let r = self.dn[ni].level as usize;
                by_level[fill[r] as usize] = ni as u32;
                fill[r] += 1;
            }
        }
        // the sign of each level from the last one up: the one that asks the most.
        // A level no node decides takes the sign its literal children ask for.
        let mut vote = vec![0i32; levels];
        for ni in 0..count {
            if self.dn[ni].x == 0 {
                continue;
            }
            for c in kids[ni] {
                if let DKid::Lit(l) = c {
                    let r = rank[self.d_level[l.unsigned_abs() as usize] as usize] as usize;
                    vote[r] += if l > 0 { 1 } else { -1 };
                }
            }
        }
        let mut val = vec![0u32; count];
        let mut sign = vec![true; levels];
        let cap = levels as u32 + 2;
        for r in (0..levels).rev() {
            let (a, b) = (start[r] as usize, start[r + 1] as usize);
            self.ck.pay(1 + 2 * (b - a) as u64)?;
            if a == b {
                sign[r] = vote[r] >= 0;
                continue;
            }
            // what each sign gives, and on equal terms which branch asks for less: the
            // literal that lowers what is asked for is the one counted
            let (mut ps, mut ng, mut lean) = (0u64, 0u64, 0i64);
            for &ni in &by_level[a..b] {
                let ni = ni as usize;
                let (vh, vl) = (self.d_val(kids[ni][0], &val, &sign, &rank), self.d_val(kids[ni][1], &val, &sign, &rank));
                ps += d_min(vh, vl).min(cap) as u64;
                ng += d_min(vl, vh).min(cap) as u64;
                lean += vl.min(cap) as i64 - vh.min(cap) as i64;
            }
            sign[r] = ps > ng || (ps == ng && lean >= 0);
            let s = sign[r];
            for &ni in &by_level[a..b] {
                let ni = ni as usize;
                let (vh, vl) = (self.d_val(kids[ni][0], &val, &sign, &rank), self.d_val(kids[ni][1], &val, &sign, &rank));
                val[ni] = if s { d_min(vh, vl) } else { d_min(vl, vh) };
            }
        }
        self.d_root[li(root)] = true;
        let ri = self.d_at[li(root)] as usize;
        if self.dn[ri].x == 0 {
            return Some(());
        }
        self.d_check(root, ri, &vars, &sign, &rank, &kids, &by_level, &start, &mut val)?;
        // the same diagram with every input the other way round: a diagram that says
        // "exactly" also says the other bound
        let other: Vec<bool> = sign.iter().map(|&s| !s).collect();
        self.d_check(root, ri, &vars, &other, &rank, &kids, &by_level, &start, &mut val)
    }

    fn d_new_level(&mut self, v: i32, levels: &mut Vec<i32>) {
        let v = v as usize;
        if self.d_level_stamp[v] != self.d_epoch {
            self.d_level_stamp[v] = self.d_epoch;
            self.d_level[v] = levels.len() as u32;
            levels.push(v as i32);
        }
    }

    /// What a child asks for under the signs chosen so far.
    #[inline]
    fn d_val(&self, k: DKid, val: &[u32], sign: &[bool], rank: &[u32]) -> u32 {
        match k {
            DKid::True => 0,
            DKid::False | DKid::Dead(_) | DKid::Lemma(_) => DEAD,
            DKid::Lit(l) => {
                let r = rank[self.d_level[l.unsigned_abs() as usize] as usize] as usize;
                ((l > 0) == sign[r]) as u32
            }
            DKid::Node(q) => val[q as usize],
        }
    }

    /// The check of a diagram under one sign per level, and its count if it passes.
    /// It takes from the reading only the plan: which child on which side, and the
    /// levels. It works out the values itself and looks up every clause a line of
    /// the proof relies on.
    #[allow(clippy::too_many_arguments)]
    fn d_check(&mut self, root: i32, ri: usize, vars: &[i32], sign: &[bool], rank: &[u32], kids: &[[DKid; 2]], by_level: &[u32], start: &[u32], val: &mut [u32]) -> Option<()> {
        let levels = vars.len();
        if self.ix.unit[li(root)] != 2 {
            return Some(());
        }
        self.ck.pay(1 + 2 * by_level.len() as u64)?;
        for r in (0..levels).rev() {
            let s = sign[r];
            for &ni in &by_level[start[r] as usize..start[r + 1] as usize] {
                let ni = ni as usize;
                let (vh, vl) = (self.d_val(kids[ni][0], val, sign, rank), self.d_val(kids[ni][1], val, sign, rank));
                val[ni] = if s { d_min(vh, vl) } else { d_min(vl, vh) };
            }
        }
        let need = val[ri];
        if need == 0 || need == DEAD {
            return Some(());
        }
        // what the proof needs, from the root: the false child always, the true
        // child when more than one true is asked for, and for a dead node both.
        let kid_of = |ni: usize, r: usize| -> (DKid, DKid) { if sign[r] { (kids[ni][0], kids[ni][1]) } else { (kids[ni][1], kids[ni][0]) } };
        self.next_mark();
        let mut stack: Vec<u32> = vec![ri as u32];
        let mut members: Vec<u32> = Vec::new();
        let mut used = vec![false; levels];
        let mut seen = vec![false; self.dn.len()];
        seen[ri] = true;
        self.set_mark(self.dn[ri].lit);
        self.ck.pay(levels as u64 + self.dn.len() as u64)?;
        while let Some(ni) = stack.pop() {
            self.ck.pay(2)?;
            let ni = ni as usize;
            members.push(ni as u32);
            let r = self.dn[ni].level as usize;
            used[r] = true;
            let c = val[ni];
            let (a, b) = kid_of(ni, r);
            for (k, wanted) in [(b, true), (a, c >= 2)] {
                if !wanted {
                    continue;
                }
                match k {
                    DKid::Lit(l) => {
                        if self.d_val(k, val, sign, rank) == 1 {
                            used[rank[self.d_level[l.unsigned_abs() as usize] as usize] as usize] = true;
                        }
                    }
                    DKid::Node(q) => {
                        if val[q as usize] != 0 && !seen[q as usize] {
                            seen[q as usize] = true;
                            // a node's variable once: n and its negation are not two nodes
                            let lit = self.dn[q as usize].lit;
                            if self.marked(lit) {
                                return Some(());
                            }
                            self.set_mark(lit);
                            stack.push(q);
                        }
                    }
                    _ => {}
                }
            }
        }
        // the counted literals, every variable once and none of them a node
        let mut inputs: Vec<i32> = Vec::new();
        let mut from = vec![0u32; levels + 1];
        for r in 0..levels {
            from[r] = inputs.len() as u32;
            if used[r] {
                inputs.push(if sign[r] { vars[r] } else { -vars[r] });
            }
        }
        from[levels] = inputs.len() as u32;
        let n = inputs.len();
        // a bound of one is a clause; only a one-way counter's walk reads those
        if n < 3 || need < 2 || need as usize >= n {
            return Some(());
        }
        for &y in &inputs {
            if self.marked(y) {
                return Some(());
            }
            self.set_mark(y);
        }
        let key = (set_hash(&inputs), need as i32, n as u32);
        if self.seen_found.contains(&key) {
            return Some(());
        }
        // every clause
        self.ck.pay(1 + members.len() as u64)?;
        for &ni in &members {
            let ni = ni as usize;
            let lit = self.dn[ni].lit;
            let r = self.dn[ni].level as usize;
            let y = if sign[r] { vars[r] } else { -vars[r] };
            let c = val[ni];
            let (a, b) = kid_of(ni, r);
            if !self.d_side(lit, y, b)? || (c >= 2 && !self.d_side(lit, -y, a)?) {
                return Some(());
            }
            let lv = self.dn[ni].level;
            for k in [a, b] {
                let below = match k {
                    DKid::Node(q) => self.dn[q as usize].level,
                    DKid::Lit(l) => rank[self.d_level[l.unsigned_abs() as usize] as usize],
                    _ => continue,
                };
                if below <= lv {
                    return Some(());
                }
            }
        }
        let Some(cid) = self.find(&[root])? else { return Some(()) };
        // the plan: the records, the last level first, so a child is written before
        // its parents. A statement already planned for another count is shared.
        let begin = self.rd.d_inputs.len() as u32;
        self.rd.d_inputs.extend_from_slice(&inputs);
        let mut hash = vec![0u64; n + 1];
        for p in (0..n).rev() {
            hash[p] = hash[p + 1].wrapping_add(mix(inputs[p]));
        }
        self.ck.pay((3 + (usize::BITS - members.len().leading_zeros()) as u64) * members.len() as u64)?;
        members.sort_unstable_by_key(|&ni| std::cmp::Reverse(self.dn[ni as usize].level));
        let mut rec_of: HashMap<u32, u32> = HashMap::with_capacity(members.len());
        let order_start = self.rd.d_order.len() as u32;
        for &ni in &members {
            let ni_us = ni as usize;
            let r = self.dn[ni_us].level as usize;
            let c = val[ni_us];
            let lit = self.dn[ni_us].lit;
            let p = from[r] as usize;
            let (a, b) = kid_of(ni_us, r);
            let lemma = match (a, b) {
                (DKid::Lemma(y), _) | (_, DKid::Lemma(y)) => y,
                _ => 0,
            };
            let mut pair = [NONE; 2];
            for (slot, k) in [(0usize, b), (1usize, if c >= 2 { a } else { DKid::True })] {
                if let DKid::Node(q) = k {
                    if val[q as usize] != 0 {
                        pair[slot] = rec_of[&q];
                    }
                }
            }
            let len = (n - p) as u32;
            let s = self.d_rec_for(lit, c, len, hash[p], begin + p as u32, pair, r as u32, lemma)?;
            rec_of.insert(ni, s);
            self.rd.d_order.push(s);
        }
        let order_len = self.rd.d_order.len() as u32 - order_start;
        let root_rec = rec_of[&(ri as u32)];
        // a dead record has only its `rup`; every other its `pbc`
        let cnum = self.rd.d_counts.len() as u32;
        self.rd.d_counts.push(DCount { order_start, order_len, root: root_rec, unit_id: cid as u64 + 1 });
        self.rd.d_recs[root_rec as usize].pending += 1;
        let mut work = 0u64;
        let lines = self.rd.d_lines_left(cnum, &mut work);
        self.ck.pay(work)?;
        self.add_found(inputs, need as i32, Form::Diagram, Plan::Diagram { count: cnum }, lines)?;
        Some(())
    }

    /// The clause a branch relies on: y false gives the child k, so (-n v y v k), or
    /// the clause without y, which says more; (-n v y) for a false child.
    fn d_side(&mut self, n: i32, y: i32, k: DKid) -> Option<bool> {
        Some(match k {
            DKid::True | DKid::Lemma(_) => true,
            DKid::False => self.find(&[-n, y])?.is_some(),
            DKid::Dead(l) | DKid::Lit(l) => self.find(&[-n, y, l])?.is_some() || self.find(&[-n, l])?.is_some(),
            DKid::Node(q) => {
                let l = self.dn[q as usize].lit;
                self.find(&[-n, y, l])?.is_some() || self.find(&[-n, l])?.is_some()
            }
        })
    }

    /// One record per distinct statement: the same node, the same value, the same
    /// inputs. Its children are counted as used when it is new.
    #[allow(clippy::too_many_arguments)]
    fn d_rec_for(&mut self, lit: i32, c: u32, len: u32, hash: u64, start: u32, kids: [u32; 2], level: u32, lemma: i32) -> Option<u32> {
        self.ck.pay(2)?;
        let key = (lit, c, len, hash);
        if let Some(&s) = self.d_rec_of.get(&key) {
            self.ck.pay(len as u64)?;
            let r = &self.rd.d_recs[s as usize];
            let here = &self.rd.d_inputs[start as usize..(start + len) as usize];
            let there = &self.rd.d_inputs[r.start as usize..(r.start + r.len) as usize];
            if here == there {
                return Some(s);
            }
        }
        let s = self.rd.d_recs.len() as u32;
        self.rd.d_recs.push(DRec { lit, c, start, len, kids, lemma, lemma_id: 0, level, id: 0, pending: 0, sim: 0, stamp: 0 });
        for k in kids {
            if k != NONE {
                self.rd.d_recs[k as usize].pending += 1;
            }
        }
        self.d_rec_of.insert(key, s);
        Some(s)
    }

    fn read_diagrams(&mut self) -> Option<()> {
        let f = self.ix.f;
        let nl = self.ix.unit.len();
        let nv = nl / 2;
        self.ck.pay(4 * nl as u64 + 3 * nv as u64)?;
        self.d_said = vec![false; nl];
        self.d_at = vec![0; nl];
        self.d_at_stamp = vec![0; nl];
        self.d_level = vec![0; nv];
        self.d_level_stamp = vec![0; nv];
        self.d_known = vec![0; nv];
        self.d_root = vec![false; nl];
        self.ck.pay(f.len() as u64)?;
        // one reading may spend all that is left; a new one starts only while half of
        // it is untouched, so that a formula of many units cannot starve what comes after
        let start = *self.ck.ticks;
        let share = self.ck.budget.saturating_sub(start) / 2;
        for i in 0..f.len() {
            self.ck.poll(i)?;
            if self.ix.uniq[i] && f.start[i + 1] - f.start[i] == 1 {
                if *self.ck.ticks - start > share {
                    return Some(());
                }
                let u = f.clause(i)[0];
                self.read_diagram(u)?;
            }
        }
        Some(())
    }
}

/// What a node asks for: one more than its true child's value, or its false child's
/// value, whichever is less.
#[inline]
fn d_min(on_true: u32, on_false: u32) -> u32 {
    let up = if on_true == DEAD { DEAD } else { on_true + 1 };
    up.min(on_false)
}

/// How far a walk counts: one more than the lowest place a register holds.
fn walk_width(cols: &[Column]) -> usize {
    cols.iter().map(|c| c.regs.iter().rposition(|&s| s != 0).map_or(0, |h| h + 1)).max().unwrap_or(0)
}

/// One column of a one-way counter: its input and, per register, the lo and hi
/// registers in the next column, which is `next`.
struct Column {
    x: i32,
    regs: Vec<i32>,
    lo: Vec<i32>,
    hi: Vec<i32>,
    next: Vec<i32>,
    explained: u32,
}

/// Every count in the input clauses, within `budget` (a tick count to stop at).
/// Work is charged to `ticks` as it is done; `stop` is the termination flag.
pub fn read(f: &Formula, budget: u64, ticks: &mut u64, stop: &AtomicBool) -> Reading {
    let hook = std::env::var_os("SAT_CARD_READ_CHECK").is_some();
    let begin = *ticks;
    let last = *ticks;
    let started = std::time::Instant::now();
    let mut ck = Clock { ticks, budget, stop, last };
    let nv = f.vars.max(0) as usize + 1;
    let ix = Ix::build(f, &mut ck);
    let Some(ix) = ix.filter(|_| ck.pay(nv as u64).is_some()) else {
        if hook {
            println!("c cardread ticks={} index={} trees=0 diagrams=0 adders=0 networks=0 walks=0 subsets=0 finished=false ms={}", *ck.ticks - begin, *ck.ticks - begin, started.elapsed().as_millis());
        }
        return Reading { hook, ..Reading::default() };
    };
    let mut r = Reader::new(ix, ck, Reading { hook, input: f.len() as u64, ..Reading::default() }, nv);
    let index_ticks = *r.ck.ticks - begin;
    let trees = r.read_trees().is_some();
    // after a stop too: what is kept is what the found counts need
    let _ = r.keep_used_nodes();
    let tree_ticks = *r.ck.ticks - begin;
    let diagrams = trees && r.read_diagrams().is_some();
    let diagram_ticks = *r.ck.ticks - begin;
    // each of these may spend half of what is left, so neither starves what comes after
    let limit = r.ck.budget;
    let fence = |r: &mut Reader| r.ck.budget = r.ck.ticks.saturating_add(limit.saturating_sub(*r.ck.ticks) / 2);
    // what only the reading of diagrams needs
    r.d_rec_of = HashMap::new();
    r.d_at = Vec::new();
    r.d_at_stamp = Vec::new();
    r.d_level = Vec::new();
    r.d_level_stamp = Vec::new();
    r.d_known = Vec::new();
    r.dn = Vec::new();
    r.d_came = Vec::new();
    r.d_big_head = Vec::new();
    r.d_big = Vec::new();
    fence(&mut r);
    let networks = trees && r.read_networks().is_some();
    let network_ticks = *r.ck.ticks - begin;
    r.gate_of = Vec::new();
    r.gate_seen = Vec::new();
    r.gates = Vec::new();
    fence(&mut r);
    let adders = trees && r.read_adders().is_some();
    let adder_ticks = *r.ck.ticks - begin;
    r.ck.budget = limit;
    let walks = trees && r.read_walks().is_some();
    let walk_ticks = *r.ck.ticks - begin;
    let subsets = walks && r.read_subsets().is_some();
    let done = diagrams && adders && networks && subsets;
    if hook {
        println!(
            "c cardread ticks={} index={} trees={} diagrams={} adders={} networks={} walks={} subsets={} finished={} ms={}",
            *r.ck.ticks - begin,
            index_ticks,
            tree_ticks - index_ticks,
            diagram_ticks - tree_ticks,
            adder_ticks - network_ticks,
            network_ticks - diagram_ticks,
            walk_ticks - adder_ticks,
            *r.ck.ticks - walk_ticks,
            done,
            started.elapsed().as_millis()
        );
    }
    r.rd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formula(clauses: &[Vec<i32>]) -> Formula {
        let mut f = Formula { lits: Vec::new(), start: vec![0], taut: Vec::new(), dup: Vec::new(), vars: 0 };
        for c in clauses {
            for &l in c {
                f.lits.push(l);
                f.vars = f.vars.max(l.abs());
            }
            f.start.push(f.lits.len() as u32);
            f.taut.push(false);
            f.dup.push(false);
        }
        f
    }

    fn read_all(f: &Formula) -> Reading {
        let stop = AtomicBool::new(false);
        let mut ticks = 0u64;
        read(f, u64::MAX, &mut ticks, &stop)
    }

    fn writer(name: &str, f: &Formula) -> (Writer, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("cardread-unit-{}-{}.pbp", std::process::id(), name));
        (Writer::create(path.to_str().unwrap(), f.len() as u64).unwrap(), path)
    }

    /// A one-way counter over `n` inputs with a positive unit on each of `tops`.
    fn one_way(n: i32, k: i32, tops: &[(i32, i32)]) -> Vec<Vec<i32>> {
        let reg = |i: i32, j: i32| n + (i - 1) * k + j;
        let mut c: Vec<Vec<i32>> = Vec::new();
        for i in 1..=n {
            for j in 1..=k.min(i) {
                let s = reg(i, j);
                if i > j {
                    c.push(vec![-s, reg(i - 1, j), i]);
                } else {
                    c.push(vec![-s, i]);
                }
                if j > 1 {
                    c.push(vec![-s, reg(i - 1, j - 1)]);
                }
            }
        }
        for &(i, j) in tops {
            c.push(vec![reg(i, j)]);
        }
        c
    }

    /// A totalizer node's clauses, the outputs numbered from `first`.
    fn merge(c: &mut Vec<Vec<i32>>, a: &[i32], b: &[i32], first: i32) -> Vec<i32> {
        let width = a.len() + b.len();
        let r: Vec<i32> = (0..width as i32).map(|k| first + k).collect();
        for i in 0..=a.len() {
            for j in 0..=b.len() {
                if i + j > 0 {
                    let mut cl = vec![r[i + j - 1]];
                    if i > 0 {
                        cl.push(-a[i - 1]);
                    }
                    if j > 0 {
                        cl.push(-b[j - 1]);
                    }
                    c.push(cl);
                }
            }
        }
        r
    }

    #[test]
    fn a_proof_is_written_whole_or_not_at_all() {
        let f = formula(&one_way(8, 3, &[(8, 3)]));
        let mut r = read_all(&f);
        assert_eq!(r.found.len(), 1);
        assert_eq!(r.found[0].need, 3);
        assert_eq!(r.found[0].lits.len(), 8);
        let lines = r.lines(0);
        assert!(lines > 10);
        let (mut w, path) = writer("whole", &f);
        let mut next_var = f.vars;
        let mut work = 0u64;
        let before = (w.constraints, w.steps);
        assert!(r.prove(0, &mut w, &mut next_var, lines - 1, &mut work).is_none());
        assert_eq!((w.constraints, w.steps), before, "a refused proof writes nothing");
        assert_eq!(r.lines(0), lines);
        let id = r.prove(0, &mut w, &mut next_var, lines, &mut work).expect("the whole proof fits");
        assert!(work > 0);
        assert_eq!(next_var, f.vars, "these forms need no fresh variables");
        assert_eq!(r.lines(0), lines, "what is written does not change the plan");
        // asking again writes nothing and gives the same id
        let after = (w.constraints, w.steps);
        assert_eq!(r.prove(0, &mut w, &mut next_var, 0, &mut work), Some(id));
        assert_eq!((w.constraints, w.steps), after);
        drop(w);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_node_two_counts_share_is_written_once() {
        // two roots over one child, each with a unit
        let mut c: Vec<Vec<i32>> = Vec::new();
        let n1 = merge(&mut c, &[1], &[2], 10);
        let n2 = merge(&mut c, &[3], &[4], 13);
        let shared = merge(&mut c, &n1, &n2, 20);
        let p1 = merge(&mut c, &shared, &[5], 30);
        let p2 = merge(&mut c, &shared, &[6], 40);
        c.push(vec![-p1[2]]);
        c.push(vec![-p2[3]]);
        let f = formula(&c);
        let mut r = read_all(&f);
        assert_eq!(r.found.len(), 2);
        let (mut w, path) = writer("shared", &f);
        let mut next_var = f.vars;
        let mut work = 0u64;
        let a = w.constraints;
        r.prove(0, &mut w, &mut next_var, u64::MAX, &mut work).unwrap();
        let first = w.constraints - a;
        let b = w.constraints;
        // less than a whole proof is enough for the second count now
        let lines = r.lines(1);
        r.prove(1, &mut w, &mut next_var, u64::MAX, &mut work).unwrap();
        let second = w.constraints - b;
        assert!(second < first, "the shared node is reused: {} after {}", second, first);
        assert_eq!(r.lines(1), lines);
        drop(w);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn registers_two_walks_share_are_written_once() {
        let f = formula(&one_way(10, 2, &[(10, 2), (7, 2)]));
        let mut r = read_all(&f);
        assert_eq!(r.found.len(), 2, "{:?}", r.found.iter().map(|c| c.lits.len()).collect::<Vec<_>>());
        let (mut w, path) = writer("walks", &f);
        let mut next_var = f.vars;
        let mut work = 0u64;
        r.prove(0, &mut w, &mut next_var, u64::MAX, &mut work).unwrap();
        let b = w.constraints;
        r.prove(1, &mut w, &mut next_var, u64::MAX, &mut work).unwrap();
        // the second count is inside the first: only its sum with the unit is new
        assert_eq!(w.constraints - b, 1);
        drop(w);
        let _ = std::fs::remove_file(path);
    }

    /// The columns of the one-way counter `one_way(4, 2, ..)`, read by hand.
    fn four_two_columns() -> Vec<Column> {
        let s = |i: i32, j: i32| 4 + (i - 1) * 2 + j;
        let col = |x: i32, regs: Vec<i32>, lo: Vec<i32>, hi: Vec<i32>| Column { x, regs, lo, hi, next: Vec::new(), explained: 0 };
        vec![
            col(4, vec![s(4, 2)], vec![s(3, 2)], vec![s(3, 1)]),
            col(3, vec![s(3, 2), s(3, 1)], vec![s(2, 2), s(2, 1)], vec![s(2, 1), 0]),
            col(2, vec![s(2, 2), s(2, 1)], vec![0, s(1, 1)], vec![s(1, 1), 0]),
            col(1, vec![0, s(1, 1)], vec![0, 0], vec![0, 0]),
        ]
    }

    /// Run `body` on a reader over `f`, as `read` makes one.
    macro_rules! with_reader {
        ($f:expr, $r:ident, $body:block) => {{
            let stop = AtomicBool::new(false);
            let mut ticks = 0u64;
            let mut ck = Clock { ticks: &mut ticks, budget: u64::MAX, stop: &stop, last: 0 };
            let ix = Ix::build($f, &mut ck).unwrap();
            let nv = $f.vars as usize + 1;
            let mut $r = Reader::new(ix, ck, Reading { input: $f.len() as u64, ..Reading::default() }, nv);
            $r.clause_read = vec![false; 2 * nv];
            $body
        }};
    }

    /// The check of a walk on its own: detection cannot hand it these plans, so
    /// nothing else exercises its refusals.
    #[test]
    fn the_walk_check_refuses_every_plan_the_clauses_do_not_hold() {
        let s = |i: i32, j: i32| 4 + (i - 1) * 2 + j;
        let top = s(4, 2);
        let f = formula(&one_way(4, 2, &[(4, 2)]));
        let reads = |cols: &[Column], f: &Formula, u: i32| -> usize {
            with_reader!(f, r, {
                r.check_walk(u, cols).unwrap();
                r.rd.found.len()
            })
        };
        assert_eq!(reads(&four_two_columns(), &f, top), 1, "the plan the clauses hold");
        // a lo that is not the next column's register
        let mut c = four_two_columns();
        c[1].lo[0] = s(2, 1);
        assert_eq!(reads(&c, &f, top), 0, "a wrong lo");
        // the two registers below crossed
        let mut c = four_two_columns();
        c[0].hi[0] = s(3, 2);
        c[0].lo[0] = s(3, 1);
        assert_eq!(reads(&c, &f, top), 0, "hi and lo crossed");
        // no register below a register that must have one
        let mut c = four_two_columns();
        c[2].hi[0] = 0;
        assert_eq!(reads(&c, &f, top), 0, "no hi with j = 2");
        // a register below one that counts only to one
        let mut c = four_two_columns();
        c[1].hi[1] = s(2, 1);
        assert_eq!(reads(&c, &f, top), 0, "a hi with j = 1");
        // an input the clauses do not name
        let mut c = four_two_columns();
        c[1].x = -3;
        assert_eq!(reads(&c, &f, top), 0, "another input");
        // one input twice
        let mut c = four_two_columns();
        c[0].x = 3;
        assert_eq!(reads(&c, &f, top), 0, "an input twice");
        // a register that is also an input
        let mut c = four_two_columns();
        c[0].x = s(3, 2);
        assert_eq!(reads(&c, &f, top), 0, "a register as an input");
        // a register twice
        let mut c = four_two_columns();
        c[2].regs[0] = s(3, 2);
        assert_eq!(reads(&c, &f, top), 0, "a register twice");
        // the plan starts from another register than the asserted one
        assert_eq!(reads(&four_two_columns(), &f, s(3, 2)), 0, "no unit there");
        // no unit at all
        let g = formula(&one_way(4, 2, &[]));
        assert_eq!(reads(&four_two_columns(), &g, top), 0, "no unit");
        // a clause gone
        let mut clauses = one_way(4, 2, &[(4, 2)]);
        clauses.remove(0);
        let g = formula(&clauses);
        assert_eq!(reads(&four_two_columns(), &g, top), 0, "the first clause dropped");
    }

    /// The check of a family on its own, as above.
    #[test]
    fn the_family_check_refuses_what_is_no_family() {
        let lits = [1, -2, 3, 4, -5];
        let mut clauses: Vec<Vec<i32>> = Vec::new();
        for mask in 0u32..32 {
            if mask.count_ones() == 3 {
                clauses.push((0..5).filter(|&b| mask >> b & 1 == 1).map(|b| lits[b]).collect());
            }
        }
        let reads = |f: &Formula, s: Vec<i32>, m: usize| -> usize {
            with_reader!(f, r, {
                let mut used = vec![false; f.len()];
                r.check_family(s, m, &mut used).unwrap();
                r.rd.found.len()
            })
        };
        let f = formula(&clauses);
        assert_eq!(reads(&f, lits.to_vec(), 3), 1, "the family");
        assert_eq!(reads(&f, vec![1, -2, 3, 4, 5], 3), 0, "a sign wrong");
        let mut more = clauses.clone();
        more.push(vec![6, 1]);
        assert_eq!(reads(&formula(&more), vec![1, -2, 3, 4, -5, 6], 3), 0, "a literal too many");
        assert_eq!(reads(&f, vec![1, -2, 3, 4, -4], 3), 0, "a variable twice");
        assert_eq!(reads(&f, lits.to_vec(), 2), 0, "pairs are not the family's");
        for drop in 0..clauses.len() {
            let mut g = clauses.clone();
            g.remove(drop);
            assert_eq!(reads(&formula(&g), lits.to_vec(), 3), 0, "clause {} gone", drop);
        }
        // a clause with a repeated literal is not summed
        let mut g = formula(&clauses);
        g.dup[4] = true;
        assert_eq!(reads(&g, lits.to_vec(), 3), 0, "only a dirty copy");
    }

    /// A diagram with nodes (i, c) over the literals, the asserted root deciding on `y`
    /// and reaching (1, c - 1) and (1, c). The nodes' number starts at `first`.
    fn diagram(c: &mut Vec<Vec<i32>>, x: &[i32], k: usize, first: i32) -> (Vec<i32>, i32) {
        // the rows from the last input up: row[c] is (i, c)
        let n = x.len();
        let mut at = first;
        let mut below: Vec<i32> = vec![0; k + 2];
        let mut tops = Vec::new();
        for i in (1..n).rev() {
            let mut row = vec![0; k + 2];
            for w in 1..=k.min(n - i) {
                at += 1;
                row[w] = at;
                if w > 1 {
                    c.push(vec![-at, -x[i], below[w - 1]]);
                }
                if below[w] != 0 {
                    c.push(vec![-at, x[i], below[w]]);
                } else {
                    c.push(vec![-at, x[i]]);
                }
            }
            below = row;
            tops = below.clone();
        }
        (tops, at)
    }

    /// Two roots over the same nodes: the second count writes only its root and its sum.
    #[test]
    fn a_diagram_two_counts_share_is_written_once() {
        let x: Vec<i32> = (1..=8).collect();
        let mut c: Vec<Vec<i32>> = Vec::new();
        let (tops, last) = diagram(&mut c, &x, 3, 9);
        let (r1, r2) = (last + 1, last + 2);
        for (r, y) in [(r1, 1), (r2, 9)] {
            c.push(vec![-r, -y, tops[2]]);
            c.push(vec![-r, y, tops[3]]);
            c.push(vec![r]);
        }
        let f = formula(&c);
        let mut r = read_all(&f);
        assert_eq!(r.found.len(), 2, "{:?}", r.found.iter().map(|q| (q.need, q.lits.len())).collect::<Vec<_>>());
        assert!(r.found.iter().all(|q| q.need == 3 && q.lits.len() == 8 && q.form == Form::Diagram));
        let (mut w, path) = writer("diagram-shared", &f);
        let mut next_var = f.vars;
        let mut work = 0u64;
        let a = w.constraints;
        r.prove(0, &mut w, &mut next_var, u64::MAX, &mut work).unwrap();
        let first = w.constraints - a;
        let b = w.constraints;
        r.prove(1, &mut w, &mut next_var, u64::MAX, &mut work).unwrap();
        let second = w.constraints - b;
        // the negation, two sums, the rup, the root's constraint; then the sum with the unit
        assert_eq!(second, 6, "after {}", first);
        assert!(first > 20, "{}", first);
        drop(w);
        let _ = std::fs::remove_file(path);
    }

    /// A proof is whole or not at all for the new forms too, and the plan does not move.
    #[test]
    fn a_proof_of_each_new_form_is_written_whole_or_not_at_all() {
        let x: Vec<i32> = (1..=8).collect();
        let mut clauses: Vec<Vec<Vec<i32>>> = Vec::new();
        let mut c: Vec<Vec<i32>> = Vec::new();
        let (tops, last) = diagram(&mut c, &x[1..], 3, 8);
        let r = last + 1;
        c.push(vec![-r, -1, tops[2]]);
        c.push(vec![-r, 1, tops[3]]);
        c.push(vec![r]);
        clauses.push(c);
        // two comparators and a third over their mins and a unit: at most 2 of 4 reads
        // by subsets of three
        clauses.push(vec![vec![-1, 5], vec![-2, 5], vec![-1, -2, 6], vec![-3, 7], vec![-4, 7], vec![-3, -4, 8], vec![-5, 9], vec![-7, 9], vec![-5, -7, 10], vec![-6, 11], vec![-8, 11], vec![-6, -8, 12], vec![-10, 13], vec![-11, 13], vec![-10, -11, 14], vec![-13]]);
        // an adder over six inputs
        let (ad, _, _) = adder_clauses();
        clauses.push(ad);
        for (i, c) in clauses.iter().enumerate() {
            let f = formula(c);
            let mut r = read_all(&f);
            assert!(!r.found.is_empty(), "toy {} reads a count", i);
            let lines = r.lines(0);
            let (mut w, path) = writer(&format!("whole{}", i), &f);
            let mut next_var = f.vars;
            let mut work = 0u64;
            let before = (w.constraints, w.steps);
            assert!(r.prove(0, &mut w, &mut next_var, lines - 1, &mut work).is_none(), "toy {}", i);
            assert_eq!((w.constraints, w.steps), before, "toy {}: a refused proof writes nothing", i);
            let id = r.prove(0, &mut w, &mut next_var, lines, &mut work).expect("the whole proof fits");
            assert_eq!(next_var, f.vars, "toy {}: no fresh variables", i);
            let after = (w.constraints, w.steps);
            assert_eq!(r.prove(0, &mut w, &mut next_var, 0, &mut work), Some(id));
            assert_eq!((w.constraints, w.steps), after);
            drop(w);
            let _ = std::fs::remove_file(path);
        }
    }

    /// A hand-made plan of a diagram through its check: which child on which side, per
    /// node in level order. Detection cannot hand the check a plan its clauses do not
    /// hold, so nothing else exercises the refusals.
    fn diagram_plan_reads(clauses: &[Vec<i32>], root: i32, vars: &[i32], plan: &[(i32, usize, DKid, DKid)], sign: &[bool]) -> Vec<(usize, i32)> {
        let f = formula(clauses);
        with_reader!(&f, r, {
            let nl = r.ix.unit.len();
            r.d_at = vec![0; nl];
            r.d_at_stamp = vec![0; nl];
            r.d_level = vec![0; nl / 2];
            r.d_level_stamp = vec![0; nl / 2];
            r.d_known = vec![0; nl / 2];
            r.d_epoch = 1;
            let rank: Vec<u32> = (0..vars.len() as u32).collect();
            for (l, &v) in vars.iter().enumerate() {
                r.d_level[v as usize] = l as u32;
            }
            let mut by_level: Vec<u32> = Vec::new();
            let mut start = vec![0u32; vars.len() + 1];
            let mut kids: Vec<[DKid; 2]> = Vec::new();
            for l in 0..vars.len() {
                start[l] = by_level.len() as u32;
                for (i, q) in plan.iter().enumerate() {
                    if q.1 == l {
                        by_level.push(i as u32);
                    }
                }
            }
            start[vars.len()] = by_level.len() as u32;
            for q in plan {
                r.dn.push(DNode { lit: q.0, x: vars[q.1], hi: T_KID, lo: T_KID, came: NONE, level: q.1 as u32, no: false });
                kids.push([q.2, q.3]);
            }
            let mut val = vec![0u32; plan.len()];
            let ri = plan.iter().position(|q| q.0 == root).unwrap();
            r.d_check(root, ri, vars, sign, &rank, &kids, &by_level, &start, &mut val).unwrap();
            r.rd.found.iter().map(|c| (c.lits.len(), c.need)).collect::<Vec<_>>()
        })
    }

    #[test]
    fn the_diagram_check_refuses_every_plan_the_clauses_do_not_hold() {
        // at least 2 of 1, 2, 3: root 10 (x1: 11, 12), 11 needs 1 of 2, 3 (x2: true, 13),
        // 12 needs 2 of 2, 3 (x2: 13, false), 13 needs 3 (x3: true, false)
        let c: Vec<Vec<i32>> = vec![vec![-10, -1, 11], vec![-10, 1, 12], vec![-11, 2, 13], vec![-12, -2, 13], vec![-12, 2], vec![-13, 3], vec![10]];
        use DKid::{False, Node, True};
        let vars = [1, 2, 3];
        let plan = || vec![(10, 0, Node(1), Node(2)), (11, 1, True, Node(3)), (12, 1, Node(3), False), (13, 2, True, False)];
        let sign = [true, true, true];
        let reads = |c: &[Vec<i32>], p: &[(i32, usize, DKid, DKid)], s: &[bool]| diagram_plan_reads(c, 10, &vars, p, s);
        assert_eq!(reads(&c, &plan(), &sign), vec![(3, 2)], "the plan the clauses hold");
        // each clause of the plan gone: the true side where two are asked for, the false
        // side anywhere
        for i in 0..6 {
            let mut g = c.clone();
            g.remove(i);
            // (-11 2 13) is the false side of a node that asks for one; (-13 3) the same
            assert!(reads(&g, &plan(), &sign).is_empty(), "clause {} gone", i);
        }
        // a child the clauses do not name
        let mut p = plan();
        p[0].2 = Node(2);
        assert!(reads(&c, &p, &sign).is_empty(), "the true side wired to the other child");
        let mut p = plan();
        p[2].3 = True;
        assert_eq!(reads(&c, &p, &sign), vec![(3, 1)].into_iter().filter(|_| false).collect::<Vec<_>>(), "a false child read as asking nothing: one true, no count");
        // a false child the clause does not say
        let mut p = plan();
        p[1].3 = False;
        assert!(reads(&c, &p, &sign).is_empty(), "(-11 2) is not there");
        // a sign the clauses do not have: x2 the other way
        assert!(reads(&c, &plan(), &[true, false, true]).is_empty(), "x2 counted negated gives nothing above one");
        // a child above its parent
        let p = vec![(10, 1, Node(1), Node(2)), (11, 0, True, Node(3)), (12, 1, Node(3), False), (13, 2, True, False)];
        assert!(reads(&c, &p, &sign).is_empty(), "a child above its parent");
        // a child on its parent's own level: every clause there and every value worked out,
        // but x2 would be counted twice, and x1 false, x2 true, x3 false is a model
        let c3: Vec<Vec<i32>> = vec![vec![-20, -1, 14], vec![-20, 1, 15], vec![-14, 2, 13], vec![-15, -2, 11], vec![-15, 2, 12], vec![-11, 2, 13], vec![-12, -2, 13], vec![-12, 2], vec![-13, 3], vec![20]];
        let p3 = vec![(20, 0, Node(1), Node(4)), (14, 1, True, Node(5)), (11, 1, True, Node(5)), (12, 1, Node(5), False), (15, 1, Node(2), Node(3)), (13, 2, True, False)];
        assert!(diagram_plan_reads(&c3, 20, &vars, &p3, &sign).is_empty(), "a child on its parent's level");
        // a node's variable as an input: x3 is 13's own
        let c2: Vec<Vec<i32>> = vec![vec![-10, -1, 11], vec![-10, 1, 12], vec![-11, 2, 13], vec![-12, -2, 13], vec![-12, 2], vec![-13, 11], vec![10]];
        assert!(diagram_plan_reads(&c2, 10, &[1, 2, 11], &plan(), &sign).is_empty(), "a node's variable as an input");
        // an input twice: not expressible by level, one variable on two levels
        assert!(diagram_plan_reads(&c, 10, &[1, 2, 2], &plan(), &sign).is_empty(), "an input twice");
        // no unit
        let mut g = c.clone();
        g.pop();
        assert!(reads(&g, &plan(), &sign).is_empty(), "no unit");
        // the unit the other way
        let mut g = c.clone();
        g.pop();
        g.push(vec![-10]);
        assert!(reads(&g, &plan(), &sign).is_empty(), "the unit negated");
        // a dead child by a unit: the clause must still be there
        let mut g = c.clone();
        g.push(vec![-20]);
        let mut p = plan();
        p[2].3 = DKid::Dead(20);
        assert!(reads(&g, &p, &sign).is_empty(), "(-12 2 20) is not there");
        g.push(vec![-12, 2, 20]);
        assert_eq!(reads(&g, &p, &sign), vec![(3, 2)], "with it the plan holds");
    }

    /// Gates that are not in the formula through the network's checks.
    #[test]
    fn the_network_check_refuses_gates_the_clauses_do_not_hold() {
        // a sorting network on three wires, its rank 2 output under a unit: at most 1 of 3
        let good: Vec<Gate> = vec![Gate { a: 2, b: 3, h: 4, l: 5 }, Gate { a: 1, b: 4, h: 6, l: 7 }, Gate { a: 5, b: 7, h: 8, l: 9 }];
        let clauses = |gs: &[Gate]| -> Vec<Vec<i32>> {
            let mut c: Vec<Vec<i32>> = Vec::new();
            for g in gs {
                c.push(vec![-g.a, g.h]);
                c.push(vec![-g.b, g.h]);
                c.push(vec![-g.a, -g.b, g.l]);
            }
            c.push(vec![-8]);
            c
        };
        let reads = |c: &[Vec<i32>], gs: &[Gate], roots: &[i32]| -> Vec<(usize, i32)> {
            // a clause of its own keeps every wire's number in the tables
            let mut with: Vec<Vec<i32>> = c.to_vec();
            with.push(vec![20, 21]);
            let f = formula(&with);
            with_reader!(&f, r, {
                r.gates = gs.to_vec();
                let members: Vec<u32> = (0..gs.len() as u32).collect();
                r.net_comp(&members, roots).unwrap();
                r.rd.found.iter().map(|q| (q.lits.len(), q.need)).collect::<Vec<_>>()
            })
        };
        let c = clauses(&good);
        assert_eq!(reads(&c, &good, &[8]), vec![(3, 2)], "at most 1 of 3");
        // each clause gone
        for i in 0..c.len() - 1 {
            let mut g = c.clone();
            g.remove(i);
            let got = reads(&g, &good, &[8]);
            assert!(!got.contains(&(3, 2)), "clause {} gone: {:?}", i, got);
        }
        // no unit
        let mut g = c.clone();
        g.pop();
        assert!(reads(&g, &good, &[8]).is_empty(), "no unit");
        // the unit on the output of rank 3: only the bound of that
        let mut g = c.clone();
        g.pop();
        g.push(vec![-9]);
        assert!(!reads(&g, &good, &[9]).contains(&(3, 2)), "the unit on rank 3");
        // a gate read off its inputs: the plan names another wire
        let bad: Vec<Gate> = vec![Gate { a: 2, b: 3, h: 4, l: 5 }, Gate { a: 1, b: 5, h: 6, l: 7 }, Gate { a: 4, b: 7, h: 8, l: 9 }];
        assert!(reads(&c, &bad, &[8]).is_empty(), "wires crossed");
        // a cycle
        let cyc: Vec<Gate> = vec![Gate { a: 8, b: 3, h: 4, l: 5 }, Gate { a: 1, b: 4, h: 6, l: 7 }, Gate { a: 5, b: 7, h: 8, l: 9 }];
        assert!(reads(&c, &cyc, &[8]).is_empty(), "a cycle");
        // all outputs false but one: the sum says at most 1 of 3 on its own
        let mut g = clauses(&good);
        g.pop();
        for o in [7, 8, 9] {
            g.push(vec![-o]);
        }
        let sum = reads(&g, &good[..2], &[7]);
        assert!(!sum.contains(&(3, 3)), "two comparators, one false output: {:?}", sum);
    }

    /// The propagation on the bits of an adder network: a bound the clauses give closes,
    /// one more does not.
    #[test]
    fn the_bound_on_the_bits_closes_only_where_the_clauses_say_so() {
        let w = [8u64, 4, 2, 1];
        // at least 6 of 15: (b8 v b4), (b8 v b2)
        let c = vec![vec![0u32, 1], vec![0u32, 2]];
        assert!(a_bound_closes(&w, &c, 6));
        assert!(a_bound_closes(&w, &c, 5));
        assert!(!a_bound_closes(&w, &c, 7));
        assert!(!a_bound_closes(&w, &[], 1));
        assert!(!a_bound_closes(&w, &c, 0));
        // a unit
        assert!(a_bound_closes(&w, &[vec![1u32]], 4));
        assert!(!a_bound_closes(&w, &[vec![1u32]], 5));
    }

    fn adder_clauses() -> (Vec<Vec<i32>>, Vec<ACell>, i32) {
        // two full adders and a half adder over 1..=5, the sum of the first read by the
        // second, the carries by the half adder; a compare clause on the bits
        let (s1, m1, s2, m2, s3, m3) = (6, 7, 8, 9, 10, 11);
        let mut c: Vec<Vec<i32>> = Vec::new();
        let full = |c: &mut Vec<Vec<i32>>, a: i32, b: i32, d: i32, s: i32, m: i32| {
            c.push(vec![-a, -b, m]);
            c.push(vec![-a, -d, m]);
            c.push(vec![-b, -d, m]);
            c.push(vec![-a, b, d, s]);
            c.push(vec![a, -b, d, s]);
            c.push(vec![a, b, -d, s]);
            c.push(vec![-a, -b, -d, s]);
        };
        full(&mut c, 1, 2, 3, s1, m1);
        full(&mut c, s1, 4, 5, s2, m2);
        c.push(vec![-m1, -m2, m3]);
        c.push(vec![-m1, m2, s3]);
        c.push(vec![m1, -m2, s3]);
        // at most 3: not the 4 bit, and not both the 2 bit and... the number is s2 + 2 s3 + 4 m3
        c.push(vec![-m3]);
        let cells = vec![
            ACell { kind: CellKind::Full, a: 1, b: 2, c: 3, s: s1, m: m1 },
            ACell { kind: CellKind::Full, a: s1, b: 4, c: 5, s: s2, m: m2 },
            ACell { kind: CellKind::Half, a: m1, b: m2, c: 0, s: s3, m: m3 },
        ];
        (c, cells, m3)
    }

    /// The check of an adder network on its own: detection cannot hand it these plans.
    #[test]
    fn the_adder_check_refuses_what_is_no_network() {
        let reads = |clauses: &[Vec<i32>], cells: &[ACell]| -> Vec<(usize, i32)> {
            let f = formula(clauses);
            with_reader!(&f, r, {
                let nl = r.ix.unit.len();
                let mut made = vec![NONE; nl];
                let mut reads = vec![0u8; nl];
                for (i, c) in cells.iter().enumerate() {
                    made[li(c.s)] = i as u32;
                    made[li(c.m)] = i as u32;
                    for l in [c.a, c.b, c.c] {
                        if l != 0 {
                            reads[li(l)] += 1;
                        }
                    }
                }
                let members: Vec<u32> = (0..cells.len() as u32).collect();
                r.check_network(cells, &members, &made, &reads).unwrap();
                r.rd.found.iter().map(|c| (c.lits.len(), c.need)).collect::<Vec<_>>()
            })
        };
        let (c, cells, _) = adder_clauses();
        assert_eq!(reads(&c, &cells), vec![(5, 2)], "at most 3 of 5");
        // every clause gone in turn: the unit's bound or nothing, never the same bound
        for i in 0..c.len() - 1 {
            let mut g = c.clone();
            g.remove(i);
            assert!(reads(&g, &cells).is_empty(), "clause {} gone", i);
        }
        // a sum read in the other sign
        let mut bad = cells.clone();
        bad[1].a = -6;
        assert!(reads(&c, &bad).is_empty(), "a sum read negated");
        // a carry read at the weight of its inputs
        let mut bad = cells.clone();
        bad[1].a = 7;
        assert!(reads(&c, &bad).is_empty(), "a carry read at the wrong weight");
        // an input twice
        let mut bad = cells.clone();
        bad[0].b = 1;
        assert!(reads(&c, &bad).is_empty(), "an input twice");
        // a cell in a cycle
        let mut bad = cells.clone();
        bad[0].a = 8;
        assert!(reads(&c, &bad).is_empty(), "a cycle");
        // no bound on the bits
        let mut g = c.clone();
        g.pop();
        assert!(reads(&g, &cells).is_empty(), "no unit");
        // a sum read by two cells: every clause there, and it would count twice
        let mut g = c.clone();
        g.push(vec![-6, -12, 14]);
        g.push(vec![-6, 12, 13]);
        g.push(vec![6, -12, 13]);
        g.push(vec![-14]);
        let mut twice = cells.clone();
        twice.push(ACell { kind: CellKind::Half, a: 6, b: 12, c: 0, s: 13, m: 14 });
        assert!(reads(&g, &twice).is_empty(), "a sum read twice");
    }

    #[test]
    fn nothing_is_read_past_the_budget_and_ticks_stay_under_it() {
        let f = formula(&one_way(30, 3, &[(30, 3)]));
        let stop = AtomicBool::new(false);
        let mut full = 0u64;
        let r = read(&f, u64::MAX, &mut full, &stop);
        assert_eq!(r.found.len(), 1);
        for budget in [0, 1, 10, full / 4, full / 2, full - 1] {
            let mut ticks = 0u64;
            let r = read(&f, budget, &mut ticks, &stop);
            assert!(ticks <= budget, "{} ticks over a budget of {}", ticks, budget);
            assert!(r.found.len() <= 1);
        }
        // the stop flag ends the reading at its next look
        stop.store(true, Ordering::Relaxed);
        let mut ticks = 0u64;
        let r = read(&f, u64::MAX, &mut ticks, &stop);
        assert!(r.found.is_empty());
    }
}
