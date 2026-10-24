//! Not in kissat. The linear arithmetic pass of the structure pass (bead
//! SAT-playground-skz.3, 2026-10-19): a miter of two multipliers over the
//! same partial products is refuted by counting.
//!
//! In plain terms. After the partial products a multiplier is linear: every
//! adder cell says `a + b (+ c) = s + 2 carry` over the integers. Two adder
//! trees that sum the same weighted inputs give the same integer, whatever
//! their shape, so their output bits are equal bit by bit. Resolution cannot
//! prove that compactly (the search takes 10-28 minutes at 16 bits);
//! cutting planes proves it in a page, and the structure pass writes
//! cutting planes (VeriPB).
//!
//! What the pass does, in order:
//! 1. reads the formula back as a circuit: a variable whose clauses
//!    determine it as a function of a few others is a gate (parity clause
//!    sets define every variable in them, so the direction is settled
//!    forward: a gate is accepted once its inputs are settled and none of
//!    its clauses is used by an accepted gate; a deadlock means a primary
//!    input); equal functions over equal inputs are one signal;
//! 2. finds half and full adders by truth tables over 2- and 3-input cuts,
//!    and gives every signal a weight (a sum keeps its inputs' weight, a
//!    carry doubles it);
//! 3. finds the two cones the formula compares (from the unit clauses,
//!    through the and/or gates of the compare) and checks, as integers,
//!    that each cone is adder cells over partial products shared by both
//!    cones: the weighted sum of the cells' equations cancels every
//!    internal signal and leaves the partial products and the output bits;
//! 4. derives, bit by bit from the top, the two clauses "bit of A = bit of
//!    B" (and "zero" for a bit one side lacks) and hands them to the solver
//!    as derived clauses; the search then finishes in unit propagation.
//!
//! The final-adder step (bead SAT-playground-skz.12, 2026-10-21) handles
//! what a real netlist leaves beyond adder cells. A gate with a sum's or a
//! carry's function over a cut but no partner (a column's top adder whose
//! carry was dead logic; a ripple chain whose sum bits were) becomes a
//! cell with a fresh partner, a variable of the proof alone. A mapper may
//! write a column's carry over two other products of the same four inputs
//! (a_i b_j . a_k b_l = a_i b_l . a_k b_j) and the sum from that carry:
//! over the four inputs both are functions of the two products, a product
//! cell whose proof splits over the inputs. What is left, a carry network
//! (parallel-prefix, carry-skip, a chain of nors), is read as a black box:
//! its output is the one signal a side reads that no cell produces; a
//! virtual ripple chain of fresh cells adds up everything the side's cells
//! leave over below that carry's column, with the two's complement of the
//! side's carry-in-one constants as fixed ones, and kitten proves the
//! network's carry equal to the chain's bit at that column (either way
//! round: a nor chain hands the carry over complemented, and the top cell,
//! a lone sum of the pass's own, then reads it so). The learned clauses of
//! kitten's core are the proof of the lemma, every gate of the cone
//! restated over its inputs first (a full assignment propagates), and the
//! lemma's two clauses cancel the carry against the chain's bit in the
//! weighted sums. The constants left over are multiples of 2^(K+1), which
//! the top block absorbs.
//!
//! Proof: per cell the two sides of its equation as `pbc` blocks with a
//! short case split in `rup` steps; per cone one weighted `pol` sum; the two
//! cones added so the partial products cancel; per bit a weakening and a
//! division. Checked on toys by `tools/arith_toy_proof.py` first. The pass
//! declines (derives nothing) when the integer check does not close, or
//! when kitten finds the network's carry and the chain's bit apart (Booth
//! partial-product logic is left to search).

use crate::structure::{Derived, Formula};
use std::collections::{HashMap, HashSet};

mod booth;

const MAX_OCC: usize = 2048; // a variable with more occurrences is not tried as a gate output
const MAX_SUPPORT: usize = 6; // the truth table of a cell fits in a u64
const MAX_STARTS: usize = 32; // distinct start clauses tried per variable
const MAX_GROW: usize = 4;
const MAX_CUTS: usize = 16; // cuts kept per node
const MAX_GATES: usize = 300_000; // beyond this the pass declines
const MIN_COMPARE_CONE: usize = 16; // a compare's two inputs each have a cone at least this large
const COMPARE_OVERLAP: (usize, usize) = (3, 4); // two cones sharing less than 3/4 of the smaller are apart
const MAX_BITS: usize = 120; // weights are powers of two in an i128

struct Gate {
    sup: Vec<i32>, // variables, sorted
    tt: u64,       // minterm m: bit i of m is the value of sup[i]
}

#[derive(Clone)]
struct Cell {
    fa: bool,          // three inputs, else a half adder (with `one` the carry-in one variant)
    one: bool,         // a half adder with a constant one folded in: la + lb + 1 = s + 2c
    leaves: Vec<i32>,  // representative variables
    negmask: u8,       // bit i: leaf i enters negated
    sum: i32,          // the sum variable (a representative)
    sum_neg: bool,     // the sum literal is the negation of `sum`
    carry: i32,
    /// The variables the proof's case split runs over, when a full
    /// assignment of the leaves alone does not propagate the outputs (a
    /// product cell: its leaves are partial products the gates do not read
    /// as such, so the split is over the primary inputs under them).
    split: Option<Vec<i32>>,
}

impl Cell {
    fn leaf_lit(&self, i: usize) -> i32 {
        if (self.negmask >> i) & 1 == 1 {
            -self.leaves[i]
        } else {
            self.leaves[i]
        }
    }
    fn sum_lit(&self) -> i32 {
        if self.sum_neg {
            -self.sum
        } else {
            self.sum
        }
    }
    /// The carry literal (negative after a complemented reading).
    fn carry_lit(&self) -> i32 {
        self.carry
    }
    fn carry_var(&self) -> i32 {
        self.carry.abs()
    }
}

/// Union-find with an integer offset: exp(x) - exp(root(x)).
struct WeightedUf {
    parent: Vec<i32>,
    off: Vec<i64>,
    /// (debug) two signals whose first meeting in one component is reported
    watch: Option<(i32, i32)>,
}

impl WeightedUf {
    fn new(n: usize) -> Self {
        WeightedUf { parent: (0..n as i32).collect(), off: vec![0; n], watch: None }
    }
    /// Room for a variable past the formula's (a fresh one of the proof).
    fn reserve(&mut self, x: i32) {
        while self.parent.len() <= x as usize {
            let n = self.parent.len() as i32;
            self.parent.push(n);
            self.off.push(0);
        }
    }
    fn find(&mut self, x: i32) -> (i32, i64) {
        self.reserve(x);
        let mut path = Vec::new();
        let mut cur = x;
        while self.parent[cur as usize] != cur {
            path.push(cur);
            cur = self.parent[cur as usize];
        }
        let root = cur;
        let mut acc = 0i64;
        for &y in path.iter().rev() {
            acc += self.off[y as usize];
            self.off[y as usize] = acc;
            self.parent[y as usize] = root;
        }
        (root, if path.is_empty() { 0 } else { self.off[path[0] as usize] })
    }
    /// Require exp(a) - exp(b) == diff; false on a contradiction.
    fn union(&mut self, a: i32, b: i32, diff: i64) -> bool {
        let (ra, oa) = self.find(a);
        let (rb, ob) = self.find(b);
        if ra == rb {
            return oa - ob == diff;
        }
        if let Some((x, y)) = self.watch {
            let (rx, _) = self.find(x);
            let (ry, _) = self.find(y);
            if rx != ry && ((rx == ra && ry == rb) || (rx == rb && ry == ra)) {
                eprintln!("structarith: (watch) union({}, {}, {}) joins the components of {} and {}", a, b, diff, x, y);
            }
        }
        self.parent[ra as usize] = rb;
        self.off[ra as usize] = diff - oa + ob;
        true
    }
}

fn debug() -> bool {
    std::env::var_os("SAT_STRUCT_DEBUG").is_some()
}

/// Truth table of v over `support` if the clauses determine v for every
/// assignment of the support, else None. The clauses' other literals all
/// lie in the support.
fn definition_tt(v: i32, clause_ids: &[u32], f: &Formula, support: &[i32], work: &mut u64) -> Option<u64> {
    let n = support.len();
    // per clause: the sign of v and the other literals as (index, value)
    let mut pos: Vec<Vec<(u8, bool)>> = Vec::new();
    let mut neg: Vec<Vec<(u8, bool)>> = Vec::new();
    for &ci in clause_ids {
        let c = f.clause(ci as usize);
        let mut others = Vec::with_capacity(c.len());
        let mut sign = 0i8;
        for &l in c {
            if l.abs() == v {
                sign = if l > 0 { 1 } else { -1 };
            } else {
                let i = support.iter().position(|&u| u == l.abs()).unwrap() as u8;
                others.push((i, l > 0));
            }
        }
        if sign > 0 {
            pos.push(others);
        } else {
            neg.push(others);
        }
        *work += c.len() as u64;
    }
    let mut tt = 0u64;
    let per_assignment = (pos.len() + neg.len()) as u64;
    for m in 0..(1u32 << n) {
        *work += per_assignment; // charged as scanned, a rejected candidate too
        let violated = |others: &Vec<(u8, bool)>| others.iter().all(|&(i, val)| (((m >> i) & 1) == 1) != val);
        let pv = pos.iter().any(violated);
        let nv = neg.iter().any(violated);
        if pv == nv {
            return None; // both: a constraint; neither: v is free here
        }
        if pv {
            tt |= 1u64 << m;
        }
    }
    Some(tt)
}

struct Netlist {
    gates: Vec<Option<Gate>>, // by variable
    order: Vec<i32>,          // accepted gates in a topological order
    used: Vec<bool>,          // clause ids used by accepted gates
    units: Vec<bool>,         // variables fixed by a unit clause
    rep: Vec<(i32, bool)>,    // structural hashing: representative, negated
    deadlocks: u64,
    /// the xors of an xor collapsed by structural hashing: (a, g, y) for
    /// v = a xor g where g = a xor y (v is y or its complement)
    collapsed: Vec<(i32, i32, i32)>,
}

/// Gates from the clauses, settled forward (see the module notes).
fn recover_gates(f: &Formula, work: &mut u64, stop: &dyn Fn(u64) -> bool) -> Option<Netlist> {
    let nv = f.vars as usize + 1;
    let tables = 12 * nv as u64; // the per-variable tables of gate recovery and hashing
    if stop(*work + tables) {
        return None; // not affordable: neither allocated nor charged
    }
    *work += tables;
    let mut occ: Vec<Vec<u32>> = vec![Vec::new(); nv];
    let mut units = vec![false; nv];
    for i in 0..f.len() {
        if stop(*work) {
            return None;
        }
        if f.taut[i] {
            continue;
        }
        let c = f.clause(i);
        for &l in c {
            occ[l.unsigned_abs() as usize].push(i as u32);
        }
        if c.len() == 1 {
            units[c[0].unsigned_abs() as usize] = true;
        }
        *work += c.len() as u64;
    }
    if stop(*work) {
        return None;
    }
    // candidates: (v, support, tt, clause ids), every definition found
    let mut cands: Vec<(i32, Vec<i32>, u64, Vec<u32>)> = Vec::new();
    let mut cand_count = vec![0u32; nv];
    for v in 1..nv as i32 {
        *work += 1;
        if stop(*work) {
            return None;
        }
        let ids = &occ[v as usize];
        if ids.is_empty() || ids.len() > MAX_OCC {
            continue;
        }
        if stop(*work) {
            return None;
        }
        let mut short: Vec<u32> = ids.iter().copied().filter(|&ci| (2..=MAX_SUPPORT + 1).contains(&f.clause(ci as usize).len())).collect();
        if short.is_empty() {
            continue;
        }
        short.sort_by_key(|&ci| f.clause(ci as usize).len());
        let mut tried: HashSet<Vec<i32>> = HashSet::new();
        let mut starts = 0;
        let mut found: Vec<Vec<u32>> = Vec::new(); // clause sets found for v, to dedupe
        for &start in &short {
            *work += 1;
            if stop(*work) {
                return None; // a variable in thousands of clauses: poll per start
            }
            let mut support: Vec<i32> = f.clause(start as usize).iter().map(|l| l.abs()).filter(|&u| u != v).collect();
            support.sort_unstable();
            support.dedup();
            if support.is_empty() || support.iter().any(|&u| units[u as usize]) || tried.contains(&support) {
                continue;
            }
            starts += 1;
            if starts > MAX_STARTS {
                break;
            }
            for _grow in 0..MAX_GROW {
                if stop(*work) {
                    return None;
                }
                tried.insert(support.clone());
                let sel: Vec<u32> = short
                    .iter()
                    .copied()
                    .filter(|&ci| f.clause(ci as usize).iter().all(|&l| l.abs() == v || support.binary_search(&l.abs()).is_ok()))
                    .collect();
                *work += short.len() as u64;
                let scan = sel.iter().map(|&ci| f.clause(ci as usize).len() as u64).sum::<u64>() + (1u64 << support.len()) * sel.len() as u64;
                if stop(*work + scan) {
                    return None; // the test would not fit in what is left
                }
                if let Some(tt) = definition_tt(v, &sel, f, &support, work) {
                    let mut key = sel.clone();
                    key.sort_unstable();
                    if !found.contains(&key) {
                        found.push(key.clone());
                        cands.push((v, support.clone(), tt, key));
                        cand_count[v as usize] += 1;
                    }
                    break;
                }
                // grow by the clause that adds the fewest new variables
                let mut best: Option<(usize, Vec<i32>)> = None;
                for &ci in &short {
                    let mut oth: Vec<i32> = f.clause(ci as usize).iter().map(|l| l.abs()).filter(|&u| u != v).collect();
                    oth.sort_unstable();
                    oth.dedup();
                    let new: Vec<i32> = oth.iter().copied().filter(|u| support.binary_search(u).is_err()).collect();
                    let meets = oth.iter().any(|u| support.binary_search(u).is_ok());
                    if new.is_empty() || oth.iter().any(|&u| units[u as usize]) || (!meets && support.len() > 1) {
                        continue;
                    }
                    if support.len() + new.len() > MAX_SUPPORT {
                        continue;
                    }
                    if best.as_ref().map_or(true, |(n, _)| new.len() < *n) {
                        best = Some((new.len(), new));
                    }
                }
                *work += short.len() as u64; // the growth scan
                let Some((_, new)) = best else { break };
                support.extend(new);
                support.sort_unstable();
            }
        }
    }
    if stop(*work) {
        return None;
    }
    // is the file numbered in circuit order? judged on the clause sets that
    // define a single variable (and, majority, ite: no direction ambiguity)
    // whose variable has no other definition (two muxes over one select
    // sharing an input "define" that input, eight times over in a Booth
    // multiplier, and those read as out of order: the primary inputs come
    // first in the file)
    let mut sets_of: HashMap<&Vec<u32>, u32> = HashMap::new();
    for (_, _, _, key) in &cands {
        *sets_of.entry(key).or_default() += 1;
    }
    let mut single = 0usize;
    let mut ordered = 0usize;
    for (v, sup, _, key) in &cands {
        if sets_of[key] == 1 && cand_count[*v as usize] == 1 {
            single += 1;
            if sup.iter().all(|&u| u < *v) {
                ordered += 1;
            }
        }
    }
    let topo_numbered = single >= 20 && ordered * 20 >= single * 19;
    if debug() {
        eprintln!("structarith: {} single-definition gates, {} numbered after their inputs: {}", single, ordered, if topo_numbered { "circuit order" } else { "not circuit order" });
    }
    // forward settling
    let mut settled = vec![false; nv];
    let mut has_cands = vec![false; nv];
    for (v, _, _, _) in &cands {
        has_cands[*v as usize] = true;
    }
    let present: Vec<bool> = occ.iter().map(|o| !o.is_empty()).collect();
    for v in 1..nv {
        if present[v] && !has_cands[v] {
            settled[v] = true; // no definition found: a primary input
        }
    }
    let mut waiting_on: Vec<Vec<u32>> = vec![Vec::new(); nv];
    let mut remaining: Vec<u32> = vec![0; cands.len()];
    for (ci, (_, sup, _, _)) in cands.iter().enumerate() {
        for &u in sup {
            if !settled[u as usize] {
                waiting_on[u as usize].push(ci as u32);
                remaining[ci] += 1;
            }
        }
    }
    let mut gates: Vec<Option<Gate>> = (0..nv).map(|_| None).collect();
    let mut order: Vec<i32> = Vec::new();
    let mut used = vec![false; f.len()];
    let mut ready: Vec<u32> = (0..cands.len() as u32).filter(|&ci| remaining[ci as usize] == 0).collect();
    let mut unsettled: Vec<i32> = (1..nv as i32).filter(|&v| has_cands[v as usize] && !settled[v as usize]).collect();
    let mut deadlocks = 0u64;
    loop {
        let mut progressed = false;
        while let Some(ci) = ready.pop() {
            let (v, sup, tt, key) = &cands[ci as usize];
            if settled[*v as usize] || key.iter().any(|&c| used[c as usize]) {
                continue;
            }
            for &c in key {
                used[c as usize] = true;
            }
            gates[*v as usize] = Some(Gate { sup: sup.clone(), tt: *tt });
            order.push(*v);
            if order.len() > MAX_GATES {
                return None; // too big a circuit for the pass, however it settles
            }
            settled[*v as usize] = true;
            progressed = true;
            for &w in &waiting_on[*v as usize] {
                remaining[w as usize] -= 1;
                if remaining[w as usize] == 0 {
                    ready.push(w);
                }
            }
            *work += sup.len() as u64 + waiting_on[*v as usize].len() as u64;
            if stop(*work) {
                return None;
            }
        }
        unsettled.retain(|&v| !settled[v as usize]);
        if unsettled.is_empty() {
            break;
        }
        if !progressed {
            // deadlock: a primary input
            let best = if topo_numbered {
                *unsettled.iter().min().unwrap()
            } else {
                *unsettled.iter().max_by_key(|&&u| (waiting_on[u as usize].len(), occ[u as usize].len(), -u)).unwrap()
            };
            deadlocks += 1;
            settled[best as usize] = true;
            for &w in &waiting_on[best as usize] {
                remaining[w as usize] -= 1;
                if remaining[w as usize] == 0 {
                    ready.push(w);
                }
            }
            *work += unsettled.len() as u64;
        }
        if order.len() > MAX_GATES || stop(*work) {
            return None;
        }
    }
    if order.is_empty() {
        return None;
    }
    let rep: Vec<(i32, bool)> = (0..nv as i32).map(|v| (v, false)).collect();
    Some(Netlist { gates, order, used, units, rep, deadlocks, collapsed: Vec::new() })
}

/// Truth table over inputs whose polarities in negmask are flipped.
fn fold_negations(tt: u64, n: usize, negmask: u32) -> u64 {
    if negmask == 0 {
        return tt;
    }
    let mut out = 0u64;
    for m in 0..(1u32 << n) {
        if (tt >> (m ^ negmask)) & 1 == 1 {
            out |= 1u64 << m;
        }
    }
    out
}

/// Structural hashing: equal functions over equal inputs are one signal.
fn structural_hash(net: &mut Netlist, work: &mut u64, stop: &dyn Fn(u64) -> bool) -> Option<usize> {
    let mut key_to_rep: HashMap<(Vec<i32>, u64), (i32, bool)> = HashMap::new();
    let mut merged = 0usize;
    // an xor of a signal with an xor of that signal and y is y or its
    // complement (a Booth generator xors a row bit with its sign twice,
    // bead skz.4). Only over the gates' own inputs, the very variables:
    // the proof shows v = y by a case split over v's inputs, where the
    // inner xor's clauses give y by unit propagation; through an alias
    // (two copies of one xor merged) it would need that alias's proof,
    // which the cones may no longer ask for once v is collapsed (review,
    // 2026-10-23)
    let xor_const = |g: &Gate| -> Option<bool> {
        match (g.sup.len(), g.tt) {
            (2, XOR2) => Some(false),
            (2, XNOR2) => Some(true),
            _ => None,
        }
    };
    let order = net.order.clone();
    for v in order {
        if stop(*work) {
            return None;
        }
        let g = net.gates[v as usize].as_ref().unwrap();
        let n = g.sup.len();
        let reps: Vec<(i32, bool)> = g.sup.iter().map(|&u| net.rep[u as usize]).collect();
        let mut negmask = 0u32;
        for (i, &(_, ng)) in reps.iter().enumerate() {
            if ng {
                negmask |= 1 << i;
            }
        }
        let mut tt2 = fold_negations(g.tt, n, negmask);
        let mut perm: Vec<usize> = (0..n).collect();
        perm.sort_by_key(|&i| reps[i].0);
        if perm.iter().enumerate().any(|(i, &p)| i != p) {
            let mut tt3 = 0u64;
            for m in 0..(1u32 << n) {
                let mut m2 = 0u32;
                for (new_i, &old_i) in perm.iter().enumerate() {
                    if (m >> old_i) & 1 == 1 {
                        m2 |= 1 << new_i;
                    }
                }
                if (tt2 >> m) & 1 == 1 {
                    tt3 |= 1u64 << m2;
                }
            }
            tt2 = tt3;
        }
        let rsup: Vec<i32> = perm.iter().map(|&i| reps[i].0).collect();
        let neg = tt2 & 1 == 1;
        if neg {
            let mask = if n == 6 { u64::MAX } else { (1u64 << (1u32 << n)) - 1 };
            tt2 = !tt2 & mask;
        }
        *work += 1 << n;
        if n == 1 && tt2 == 0b10 {
            net.rep[v as usize] = (rsup[0], neg); // buffer or inverter: alias the input
            merged += 1;
            continue;
        }
        if let Some(cv) = xor_const(g) {
            // v = u xor w xor cv over its own inputs; w = u xor y0 xor cw
            // over w's own: v = y0 xor cw xor cv
            let mut found: Option<(i32, i32, i32, bool)> = None;
            for (iu, iw) in [(0usize, 1usize), (1, 0)] {
                let (u, w) = (g.sup[iu], g.sup[iw]);
                // (u the very variable both gates read, and w's own clauses
                // over u and y0: a case split over v's inputs gives y0 by
                // unit propagation, aliases or not; a copy of this pair
                // collapses the same way on its own; review, 2026-10-23)
                let Some(gw) = net.gates[w as usize].as_ref() else { continue };
                let Some(cw) = xor_const(gw) else { continue };
                let Some(k) = gw.sup.iter().position(|&x| x == u) else { continue };
                let y0 = gw.sup[1 - k];
                found = Some((u, w, y0, cw != cv));
                break;
            }
            // (onto a gate only: the proof restates a merged signal's
            // equality when its representative is in a cone, and the cones
            // hold no primary input; y0's own equality to its
            // representative comes before v's, y0 being under v)
            if let Some((u, w, y0, c)) = found.filter(|&(_, _, y0, _)| {
                let ry = net.rep[y0 as usize].0;
                net.gates[ry as usize].is_some() && !net.units[ry as usize]
            }) {
                let (ry, rneg) = net.rep[y0 as usize];
                net.rep[v as usize] = (ry, rneg != c);
                net.collapsed.push((u, w, y0));
                merged += 1;
                continue;
            }
        }
        match key_to_rep.get(&(rsup.clone(), tt2)) {
            Some(&(r, rneg)) => {
                net.rep[v as usize] = (r, rneg != neg);
                merged += 1;
            }
            None => {
                key_to_rep.insert((rsup, tt2), (v, neg));
                net.rep[v as usize] = (v, false);
            }
        }
    }
    Some(merged)
}

/// Bit-parallel value of a gate from its truth table and its inputs' words.
fn eval_gate(tt: u64, inputs: &[u64], mask: u64) -> u64 {
    let n = inputs.len();
    let mut out = 0u64;
    for m in 0..(1u32 << n) {
        if (tt >> m) & 1 == 1 {
            let mut term = mask;
            for (i, &w) in inputs.iter().enumerate() {
                term &= if (m >> i) & 1 == 1 { w } else { !w & mask };
            }
            out |= term;
        }
    }
    out
}

/// Truth table of representative node v over the sorted leaves of `cut`
/// (the cone evaluated down to the leaves, iteratively), or None when a
/// primary input that is not a leaf is reached.
fn cut_truth_table(v: i32, leaves: &[i32], net: &Netlist, memo: &mut HashMap<i32, u64>, work: &mut u64) -> Option<u64> {
    let n = leaves.len();
    let width = 1usize << n;
    let mask: u64 = if width >= 64 { u64::MAX } else { (1u64 << width) - 1 };
    memo.clear();
    for (i, &leaf) in leaves.iter().enumerate() {
        let mut w = 0u64;
        for m in 0..width {
            if (m >> i) & 1 == 1 {
                w |= 1u64 << m;
            }
        }
        memo.insert(leaf, w);
    }
    let mut stack = vec![v];
    while let Some(&u) = stack.last() {
        if memo.contains_key(&u) {
            stack.pop();
            continue;
        }
        let (r, neg) = net.rep[u as usize];
        if r != u {
            if let Some(&val) = memo.get(&r) {
                memo.insert(u, if neg { !val & mask } else { val });
                stack.pop();
            } else {
                stack.push(r);
            }
            continue;
        }
        let Some(g) = net.gates[u as usize].as_ref() else { return None };
        let mut pending = false;
        for &x in &g.sup {
            let rx = net.rep[x as usize].0;
            if !memo.contains_key(&rx) {
                stack.push(rx);
                pending = true;
            }
        }
        if pending {
            continue;
        }
        let ins: Vec<u64> = g
            .sup
            .iter()
            .map(|&x| {
                let (rx, nx) = net.rep[x as usize];
                let w = memo[&rx];
                if nx {
                    !w & mask
                } else {
                    w
                }
            })
            .collect();
        memo.insert(u, eval_gate(g.tt, &ins, mask));
        *work += ins.len() as u64;
        stack.pop();
    }
    memo.get(&v).map(|&t| t & mask)
}

const XOR2: u64 = 0b0110;
const XNOR2: u64 = 0b1001;
const XOR3: u64 = 0x96;
const XNOR3: u64 = 0x69;

/// (neg a, neg b) when tt is an AND of the two inputs with those polarities.
fn and_family(tt: u64) -> Option<u8> {
    match tt {
        0b1000 => Some(0),
        0b0100 => Some(1),
        0b0010 => Some(2),
        0b0001 => Some(3),
        _ => None,
    }
}

fn or_family(tt: u64) -> Option<u8> {
    match tt {
        0b1110 => Some(0),
        0b1101 => Some(1),
        0b1011 => Some(2),
        0b0111 => Some(3),
        _ => None,
    }
}

/// The polarity mask when tt is a majority of the three inputs.
fn maj_family(tt: u64) -> Option<u8> {
    for negmask in 0u32..8 {
        let mut t = 0u64;
        for m in 0..8u32 {
            if (m ^ negmask).count_ones() >= 2 {
                t |= 1 << m;
            }
        }
        if t == tt {
            return Some(negmask as u8);
        }
    }
    None
}

/// Half and full adders over the merged netlist, non-overlapping.
/// The half and full adders, and the cuts (up to three leaves) of every
/// representative node, which the final-adder step reuses.
fn find_adders(net: &Netlist, readers: &[u32], work: &mut u64, stop: &dyn Fn(u64) -> bool, fresh: &mut Vec<Fresh>, next_fresh: &mut i32) -> Option<(Vec<Cell>, Vec<Vec<Vec<i32>>>)> {
    // cuts of up to three leaves per representative node
    let nv = net.rep.len();
    if stop(*work + nv as u64) {
        return None; // not affordable: neither allocated nor charged
    }
    *work += nv as u64;
    let mut cuts: Vec<Vec<Vec<i32>>> = vec![Vec::new(); nv];
    for &v in &net.order {
        if net.rep[v as usize].0 != v {
            continue;
        }
        let g = net.gates[v as usize].as_ref().unwrap();
        let mut combos: Vec<Vec<i32>> = vec![Vec::new()];
        for &u in &g.sup {
            let ru = net.rep[u as usize].0;
            let lists: Vec<Vec<i32>> = if cuts[ru as usize].is_empty() { vec![vec![ru]] } else { cuts[ru as usize].clone() };
            let mut next: Vec<Vec<i32>> = Vec::new();
            for c in &combos {
                for d in &lists {
                    let mut u2 = c.clone();
                    for &x in d {
                        if !u2.contains(&x) {
                            u2.push(x);
                        }
                    }
                    if u2.len() <= 3 {
                        u2.sort_unstable();
                        next.push(u2);
                    }
                }
            }
            next.sort_by_key(|c| c.len());
            next.dedup();
            next.truncate(MAX_CUTS * 4);
            *work += next.len() as u64;
            combos = next;
        }
        let mut acc: Vec<Vec<i32>> = vec![vec![v]];
        for c in combos {
            if !c.is_empty() && c.len() <= 3 && !acc.contains(&c) {
                acc.push(c);
            }
        }
        acc.sort_by_key(|c| c.len());
        acc.truncate(MAX_CUTS);
        cuts[v as usize] = acc;
        if stop(*work) {
            return None;
        }
    }
    // truth tables over every 2- and 3-leaf cut
    let mut by_cut: HashMap<Vec<i32>, Vec<(i32, u64)>> = HashMap::new();
    let mut memo: HashMap<i32, u64> = HashMap::new();
    for &v in &net.order {
        if net.rep[v as usize].0 != v {
            continue;
        }
        for c in &cuts[v as usize] {
            if (c.len() == 2 || c.len() == 3) && !c.contains(&v) {
                if let Some(tt) = cut_truth_table(v, c, net, &mut memo, work) {
                    by_cut.entry(c.clone()).or_default().push((v, tt));
                }
            }
        }
        if stop(*work) {
            return None;
        }
    }
    let mut cells: Vec<Cell> = Vec::new();
    let mut used: HashSet<i32> = HashSet::new();
    // full adders first
    let mut keys: Vec<&Vec<i32>> = by_cut.keys().collect();
    keys.sort();
    *work += keys.len() as u64;
    for c in keys.iter() {
        if stop(*work) {
            return None;
        }
        *work += by_cut[*c].len() as u64;
        if c.len() != 3 {
            continue;
        }
        let nodes = &by_cut[*c];
        let sums: Vec<(i32, u64)> = nodes.iter().copied().filter(|&(v, tt)| (tt == XOR3 || tt == XNOR3) && !used.contains(&v)).collect();
        let mut carries: Vec<(i32, u8)> = nodes.iter().filter_map(|&(v, tt)| maj_family(tt).filter(|_| !used.contains(&v)).map(|m| (v, m))).collect();
        for &(s, stt) in &sums {
            // the nearest carry by variable number: two circuits over
            // shared partial products each have their own, and a
            // circuit's variables sit together; a gate inside the sum
            // that only the sum reads is part of the sum, not the carry
            let inside = between(s, c, net);
            let Some(k) = nearest(&carries, s, readers, &inside) else { break };
            let (cv, negmask) = carries.remove(k);
            let flips = (negmask.count_ones() & 1) == 1;
            let sum_neg = (stt == XNOR3) != flips;
            cells.push(Cell { fa: true, one: false, leaves: (*c).clone(), negmask, sum: s, sum_neg, carry: cv, split: None });
            used.insert(s);
            used.insert(cv);
        }
    }
    // a full adder built from 2-input gates holds two half-adder shapes
    // inside it: the gates inside an accepted full adder's cone are taken
    for cell in &cells {
        let stop_at: HashSet<i32> = cell.leaves.iter().copied().collect();
        let mut stack = vec![cell.sum, cell.carry];
        while let Some(u) = stack.pop() {
            *work += 1;
            let r = net.rep[u as usize].0;
            if stop_at.contains(&r) || (used.contains(&r) && r != cell.sum && r != cell.carry) {
                continue;
            }
            let Some(g) = net.gates[r as usize].as_ref() else { continue };
            used.insert(r);
            stack.extend(g.sup.iter().copied());
        }
    }
    // half adders in two passes: first those whose carry gate reads the
    // cut directly (its own inputs are the cut), then the rest. A Booth
    // row bit xor(raw, neg) with the half adder and(bit, neg) reading it
    // also makes a half-adder shape over the deeper cut (raw, neg), with
    // the row bit as its sum; taken first, it would leave the real half
    // adder's sum to no cell (bead skz.4)
    let direct = |v: i32, c: &Vec<i32>| -> bool {
        let Some(g) = net.gates[v as usize].as_ref() else { return false };
        let mut sup: Vec<i32> = g.sup.iter().map(|&u| net.rep[u as usize].0).collect();
        sup.sort_unstable();
        sup.dedup();
        &sup == c
    };
    for pass in 0..2 {
    for c in keys.iter() {
        if stop(*work) {
            return None;
        }
        *work += by_cut[*c].len() as u64;
        if c.len() != 2 {
            continue;
        }
        let nodes = &by_cut[*c];
        let mut sums: Vec<(i32, u64)> = nodes.iter().copied().filter(|&(v, tt)| (tt == XOR2 || tt == XNOR2) && !used.contains(&v)).collect();
        let mut carries: Vec<(i32, u8)> = nodes.iter().filter_map(|&(v, tt)| and_family(tt).filter(|_| !used.contains(&v) && (pass == 1 || direct(v, c))).map(|m| (v, m))).collect();
        if debug() {
            let trace: i32 = std::env::var("SAT_STRUCT_DEBUG_VIRT").ok().and_then(|t| t.parse().ok()).unwrap_or(0);
            if nodes.iter().any(|&(v, _)| v == trace) {
                eprintln!("structarith: find_adders trace {}: cut {:?} nodes {:?} sums {:?} carries {:?} used {:?}", trace, c, nodes, sums, carries, nodes.iter().map(|&(v, _)| used.contains(&v)).collect::<Vec<_>>());
            }
        }
        for &(s, stt) in &sums {
            let inside = between(s, c, net);
            let Some(k) = nearest(&carries, s, readers, &inside) else { break };
            let (cv, negmask) = carries.remove(k);
            let flips = (negmask.count_ones() & 1) == 1;
            let sum_neg = (stt == XNOR2) != flips;
            cells.push(Cell { fa: false, one: false, leaves: (*c).clone(), negmask, sum: s, sum_neg, carry: cv, split: None });
            used.insert(s);
            used.insert(cv);
        }
        sums.retain(|&(v, _)| !used.contains(&v));
        let mut carries1: Vec<(i32, u8)> = nodes.iter().filter_map(|&(v, tt)| or_family(tt).filter(|_| !used.contains(&v) && (pass == 1 || direct(v, c))).map(|m| (v, m))).collect();
        for &(s, stt) in &sums {
            let inside = between(s, c, net);
            let Some(k) = nearest(&carries1, s, readers, &inside) else { break };
            let (cv, negmask) = carries1.remove(k);
            let flips = (negmask.count_ones() & 1) == 1;
            let k = (stt == XNOR2) != flips; // s = (la xor lb) xor k
            cells.push(Cell { fa: false, one: true, leaves: (*c).clone(), negmask, sum: s, sum_neg: !k, carry: cv, split: None });
            used.insert(s);
            used.insert(cv);
        }
    }
    }
    // cells with a fused leaf (bead SAT-playground-skz.4): a Booth row bit
    // is neg xor x, and a generator or a mapper folds that xor into the
    // adder reading it, so over the cut the sum and the carry are no
    // adder's; but sum + 2 carry is still a sum of leaves, one of them the
    // xor of two cut signals. Read off the polynomial of sum + 2 carry
    // over the cut: a literal leaf is a linear term, an xor leaf x + y -
    // 2xy. The xor leaf becomes a fresh variable of the proof. Only
    // signals that are functions of a few inputs are fused (a row bit's
    // parts): the xor of two carries deep in a tree is some other adder's
    // sum, and taking it would leave that adder's gates to no cell
    let mut small: HashMap<i32, bool> = HashMap::new();
    let mut small_cone = |v: i32, work: &mut u64| -> bool {
        if let Some(&b) = small.get(&v) {
            return b;
        }
        let mut sup: HashSet<i32> = HashSet::new();
        let mut seen: HashSet<i32> = HashSet::new();
        let mut stack = vec![v];
        let mut fine = true;
        while let Some(u) = stack.pop() {
            let u = net.rep[u as usize].0;
            if !seen.insert(u) {
                continue;
            }
            *work += 1;
            match net.gates[u as usize].as_ref() {
                Some(g) => {
                    if seen.len() > 64 {
                        fine = false;
                        break;
                    }
                    stack.extend(g.sup.iter().copied());
                }
                None => {
                    sup.insert(u);
                    if sup.len() > MAX_SUPPORT {
                        fine = false;
                        break;
                    }
                }
            }
        }
        small.insert(v, fine);
        fine
    };
    for c in keys.iter() {
        if stop(*work) {
            return None;
        }
        if c.len() < 2 || c.len() > 3 {
            continue;
        }
        let nodes = &by_cut[*c];
        let free: Vec<(i32, u64)> = nodes.iter().copied().filter(|&(v, _)| !used.contains(&v)).collect();
        if free.len() < 2 {
            continue;
        }
        let n = c.len();
        let width = 1usize << n;
        'pairs: for (i, &(sv, stt)) in free.iter().enumerate() {
            for (j, &(cv, ctt)) in free.iter().enumerate() {
                if i == j || used.contains(&sv) || used.contains(&cv) {
                    continue;
                }
                // (charged and polled per pair: many gates over one cut
                // make a quadratic scan)
                *work += 4 * width as u64;
                if stop(*work) {
                    return None;
                }
                // the sum is the parity of the leaves: it must depend on
                // more than one cut signal, the carry must not be it
                for (sigma, gamma) in [(false, false), (true, false), (false, true), (true, true)] {
                    let value = |m: usize| -> i128 {
                        let sb = ((stt >> m) & 1 == 1) != sigma;
                        let cb = ((ctt >> m) & 1 == 1) != gamma;
                        sb as i128 + 2 * cb as i128
                    };
                    // the polynomial of sum + 2 carry over the cut
                    let mut f: Vec<i128> = (0..width).map(value).collect();
                    for b in 0..n {
                        for m in 0..width {
                            if (m >> b) & 1 == 1 {
                                f[m] -= f[m ^ (1 << b)];
                            }
                        }
                    }
                    // degree two at most, pair terms even
                    let mut ok = true;
                    let mut pairs: Vec<((usize, usize), i128)> = Vec::new();
                    let mut linear: Vec<i128> = vec![0; n];
                    for m in 1..width {
                        let coef = f[m];
                        if coef == 0 {
                            continue;
                        }
                        match m.count_ones() {
                            1 => linear[m.trailing_zeros() as usize] = coef,
                            2 => {
                                if coef != 2 && coef != -2 {
                                    ok = false;
                                    break;
                                }
                                let a = m.trailing_zeros() as usize;
                                let b = (m ^ (1 << a)).trailing_zeros() as usize;
                                pairs.push(((a, b), -coef / 2)); // one x xor y leaf, negated when the term is +2xy
                            }
                            _ => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if !ok || pairs.is_empty() {
                        continue;
                    }
                    // leaves: per pair |e| xor leaves (negated when e < 0),
                    // per signal the literal count left after the xor
                    // leaves' linear parts
                    let mut leaves: Vec<(Vec<i32>, bool)> = Vec::new(); // (signals, negated)
                    for &((a, b), e) in &pairs {
                        for _ in 0..e.abs() {
                            leaves.push((vec![c[a], c[b]], e < 0));
                        }
                        linear[a] -= e;
                        linear[b] -= e;
                    }
                    for b in 0..n {
                        let d = linear[b];
                        if d.abs() > 1 {
                            ok = false;
                        }
                        for _ in 0..d.abs() {
                            leaves.push((vec![c[b]], d < 0));
                        }
                    }
                    if !ok {
                        continue;
                    }
                    if leaves.len() < 2 || leaves.len() > 3 {
                        continue;
                    }
                    // (the shape this is for: a row bit, the xor of the
                    // digit's sign, a bare input, with a function of a few
                    // inputs; an xor over a tree signal or a carry
                    // network's gate is some other adder's sum, or would
                    // hide the network's carry inside a fresh leaf)
                    if leaves.iter().any(|(sig, _)| sig.len() == 2 && !(sig.iter().any(|&x| net.gates[x as usize].is_none()) && sig.iter().all(|&x| net.gates[x as usize].is_none() || small_cone(x, work)))) {
                        continue;
                    }
                    // the constant: each negated leaf is 1 - its signal, so
                    // the plain sum of leaves is f[0] minus the negations;
                    // what is left is the cell's carry-in (0 or 1)
                    let negs = leaves.iter().filter(|(_, neg)| *neg).count() as i128;
                    let k = f[0] - negs;
                    if k != 0 && k != 1 {
                        continue;
                    }
                    if k == 1 && leaves.len() == 3 {
                        continue; // a full adder has no carry-in-one
                    }
                    // the fresh variables for the xor leaves, the cell over
                    // them; sigma/gamma say the sum and carry are read
                    // negated (the carry negated is no cell: skip)
                    if gamma {
                        continue;
                    }
                    let mut cell_leaves: Vec<i32> = Vec::new();
                    let mut negmask: u8 = 0;
                    let mut made: Vec<Fresh> = Vec::new();
                    for (idx, (sig, neg)) in leaves.iter().enumerate() {
                        let mut neg = *neg;
                        if sig.len() == 2 {
                            // the xor as a signal when the netlist has one
                            // (the other circuit may read it: then both
                            // sides sum one variable), else fresh
                            let key = vec![sig[0].min(sig[1]), sig[0].max(sig[1])];
                            // (the xor may well be another cell's sum: a
                            // leaf that some cell produces is the tree)
                            let existing = by_cut.get(&key).and_then(|nodes| nodes.iter().find(|&&(u, t)| (t == XOR2 || t == XNOR2) && u != sv && u != cv).copied());
                            match existing {
                                Some((u, t)) => {
                                    cell_leaves.push(u);
                                    if t == XNOR2 {
                                        neg = !neg;
                                    }
                                }
                                None => {
                                    let x = *next_fresh + made.len() as i32;
                                    made.push(Fresh { var: x, def: FreshDef::Xor(vec![sig[0], sig[1]]) });
                                    cell_leaves.push(x);
                                }
                            }
                        } else {
                            cell_leaves.push(sig[0]);
                        }
                        if neg {
                            negmask |= 1 << idx;
                        }
                    }
                    if cell_leaves.len() != leaves.len() || {
                        let mut d = cell_leaves.clone();
                        d.sort_unstable();
                        d.dedup();
                        d.len() != cell_leaves.len()
                    } {
                        continue; // a signal twice: no cell
                    }
                    if debug() {
                        eprintln!("structarith: fused cell over {:?}: leaves {:?} (negmask {}, carry-in {}) = {}{} + 2*{}", c, leaves, negmask, k, if sigma { "-" } else { "" }, sv, cv);
                    }
                    *next_fresh += made.len() as i32;
                    fresh.extend(made);
                    // (the proof's case split runs over the cut: a full
                    // assignment of the cut propagates the fresh xor and
                    // the gates; one of the fresh variable alone does not)
                    cells.push(Cell { fa: leaves.len() == 3, one: k == 1, leaves: cell_leaves, negmask, sum: sv, sum_neg: sigma, carry: cv, split: Some((*c).clone()) });
                    used.insert(sv);
                    used.insert(cv);
                    continue 'pairs;
                }
            }
        }
    }
    Some((cells, cuts))
}

/// The index of the carry to pair with the sum `s`: one some other gate
/// reads (an xor is often built from two and-nots over its inputs that
/// only it reads, and those are no carry), the nearest by variable number
/// among those (two circuits over shared partial products each have their
/// own, and a circuit's variables sit together).
fn nearest(carries: &[(i32, u8)], s: i32, readers: &[u32], inside: &HashSet<i32>) -> Option<usize> {
    carries
        .iter()
        .enumerate()
        .filter(|(_, &(v, _))| !(inside.contains(&v) && readers[v as usize] <= 1))
        .min_by_key(|(_, &(v, _))| (readers[v as usize] <= 1, (v - s).abs()))
        .map(|(i, _)| i)
}

/// The gates strictly between a node and its cut.
fn between(v: i32, cut: &[i32], net: &Netlist) -> HashSet<i32> {
    let mut inside = HashSet::new();
    let mut stack: Vec<i32> = net.gates[v as usize].as_ref().map(|g| g.sup.iter().map(|&u| net.rep[u as usize].0).collect()).unwrap_or_default();
    while let Some(u) = stack.pop() {
        if cut.contains(&u) || !inside.insert(u) {
            continue;
        }
        if let Some(g) = net.gates[u as usize].as_ref() {
            stack.extend(g.sup.iter().map(|&x| net.rep[x as usize].0));
        }
    }
    inside
}

/// The cone of a representative node: every representative gate under it.
fn cone(rt: i32, net: &Netlist, work: &mut u64) -> HashSet<i32> {
    let mut seen = HashSet::new();
    let mut stack = vec![rt];
    while let Some(u) = stack.pop() {
        if u as usize >= net.rep.len() {
            continue; // a fresh variable (a fused leaf): no gate under it
        }
        let r = net.rep[u as usize].0;
        if seen.contains(&r) {
            continue;
        }
        let Some(g) = net.gates[r as usize].as_ref() else { continue };
        seen.insert(r);
        stack.extend(g.sup.iter().copied());
        *work += 1;
    }
    seen
}

fn apart(a: &HashSet<i32>, b: &HashSet<i32>) -> bool {
    let shared = a.intersection(b).count();
    let small = a.len().min(b.len()).max(1);
    shared * COMPARE_OVERLAP.1 < COMPARE_OVERLAP.0 * small
}



/// Random simulation of the merged netlist: 64 input patterns, one word per
/// representative node (primary inputs get random words).
fn simulate(net: &Netlist, work: &mut u64, stop: &dyn Fn(u64) -> bool) -> Option<Vec<u64>> {
    let nv = net.rep.len();
    if stop(*work + nv as u64) {
        return None;
    }
    let mut val = vec![0u64; nv];
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for v in 1..nv {
        if net.gates[v].is_none() && net.rep[v].0 == v as i32 {
            val[v] = next();
        }
    }
    *work += nv as u64;
    for &v in net.order.iter() {
        if stop(*work) {
            return None;
        }
        if net.rep[v as usize].0 != v {
            continue;
        }
        let g = net.gates[v as usize].as_ref().unwrap();
        let ins: Vec<u64> = g
            .sup
            .iter()
            .map(|&u| {
                let (r, ng) = net.rep[u as usize];
                if ng {
                    !val[r as usize]
                } else {
                    val[r as usize]
                }
            })
            .collect();
        val[v as usize] = eval_gate(g.tt, &ins, u64::MAX);
        *work += ins.len() as u64;
    }
    Some(val)
}

fn weight_of_dbg(ci: usize, cells: &[Cell], uf: &mut WeightedUf) -> i64 {
    uf.find(cells[ci].sum).1
}



/// Whether a handful of required differences exp(a) - exp(b) = d between
/// union-find roots can hold together (one cell's equations: at most five).
fn consistent(eqs: &[(i32, i32, i64)]) -> bool {
    let mut keys: Vec<i32> = Vec::with_capacity(2 * eqs.len());
    for &(a, b, _) in eqs {
        for k in [a, b] {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
    }
    let mut parent: Vec<usize> = (0..keys.len()).collect();
    let mut off: Vec<i64> = vec![0; keys.len()]; // exp(x) - exp(parent(x))
    fn find(parent: &mut [usize], off: &mut [i64], x: usize) -> (usize, i64) {
        let mut acc = 0;
        let mut cur = x;
        while parent[cur] != cur {
            acc += off[cur];
            cur = parent[cur];
        }
        (cur, acc)
    }
    for &(a, b, d) in eqs {
        let ia = keys.iter().position(|&k| k == a).unwrap();
        let ib = keys.iter().position(|&k| k == b).unwrap();
        let (ra, oa) = find(&mut parent, &mut off, ia);
        let (rb, ob) = find(&mut parent, &mut off, ib);
        if ra == rb {
            if oa - ob != d {
                return false;
            }
        } else {
            parent[ra] = rb;
            off[ra] = d - oa + ob;
        }
    }
    true
}

fn split_len(inputs: &[i32]) -> usize {
    (2usize << inputs.len()) - 1
}

/// The clauses of a case split over `inputs`, children before parents and
/// the empty clause last: a full assignment's clause first (each is implied
/// once both gates of a merge propagate), then each shorter prefix's clause
/// from its two extensions.
fn case_split(inputs: &[i32]) -> Vec<Vec<i32>> {
    fn rec(prefix: &mut Vec<i32>, inputs: &[i32], out: &mut Vec<Vec<i32>>) {
        if prefix.len() < inputs.len() {
            let x = inputs[prefix.len()];
            prefix.push(x);
            rec(prefix, inputs, out);
            prefix.pop();
            prefix.push(-x);
            rec(prefix, inputs, out);
            prefix.pop();
        }
        out.push(prefix.iter().map(|&l| -l).collect());
    }
    let mut out = Vec::new();
    rec(&mut Vec::new(), inputs, &mut out);
    out
}

/// The multilinear polynomial of a truth table over `n` inputs: the
/// coefficient of every monomial (a bitmask over the inputs), by a Moebius
/// transform (c_T = sum over U in T of (-1)^|T - U| f(U)); the zero ones
/// left out.
fn moebius(tt: u64, n: usize) -> Vec<(u32, i128)> {
    let width = 1usize << n;
    let mut f: Vec<i128> = (0..width).map(|m| ((tt >> m) & 1) as i128).collect();
    for i in 0..n {
        for m in 0..width {
            if (m >> i) & 1 == 1 {
                f[m] -= f[m ^ (1 << i)];
            }
        }
    }
    f.into_iter().enumerate().filter(|&(_, c)| c != 0).map(|(m, c)| (m as u32, c)).collect()
}

/// What a truth table over `n` inputs is under an assignment of the inputs
/// in `fixed` (a bitmask; `values` their values): a constant (`Some(0)` /
/// `Some(1)` as the literal 0 with the value in `const_value`)... encoded as
/// `Reduced`.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Reduced {
    Zero,
    One,
    Lit(usize, bool), // the input's index and whether it is read negated
    Other,
}

fn reduce_tt(tt: u64, n: usize, fixed: u32, values: u32) -> Reduced {
    let width = 1usize << n;
    let rows: Vec<usize> = (0..width).filter(|&m| (m as u32) & fixed == values & fixed).collect();
    let bit = |m: usize| (tt >> m) & 1 == 1;
    if rows.iter().all(|&m| !bit(m)) {
        return Reduced::Zero;
    }
    if rows.iter().all(|&m| bit(m)) {
        return Reduced::One;
    }
    for i in 0..n {
        if (fixed >> i) & 1 == 1 {
            continue;
        }
        if rows.iter().all(|&m| bit(m) == ((m >> i) & 1 == 1)) {
            return Reduced::Lit(i, false);
        }
        if rows.iter().all(|&m| bit(m) != ((m >> i) & 1 == 1)) {
            return Reduced::Lit(i, true);
        }
    }
    Reduced::Other
}

/// A polynomial over the formula's inputs: monomials (sorted variables,
/// the empty one the constant) with i128 coefficients.
#[derive(Default)]
struct Poly {
    terms: HashMap<Vec<i32>, i128>,
    overflow: bool,
}

impl Poly {
    fn add(&mut self, mono: Vec<i32>, c: i128) {
        let e = self.terms.entry(mono).or_insert(0);
        match e.checked_add(c) {
            Some(v) => *e = v,
            None => self.overflow = true,
        }
    }
    /// `w` times the literal of a function `tt` over `sup` (negated: 1 - f).
    fn add_tt(&mut self, sup: &[i32], tt: u64, w: i128, negated: bool) {
        let sign = if negated { -1 } else { 1 };
        if negated {
            self.add(Vec::new(), w);
        }
        for (mask, coef) in moebius(tt, sup.len()) {
            let mono: Vec<i32> = (0..sup.len()).filter(|&i| (mask >> i) & 1 == 1).map(|i| sup[i]).collect();
            match coef.checked_mul(w).and_then(|v| v.checked_mul(sign)) {
                Some(v) => self.add(mono, v),
                None => self.overflow = true,
            }
        }
    }
    fn prune(&mut self) {
        self.terms.retain(|_, c| *c != 0);
    }
}

/// An unproduced leaf that is a function of a few of the formula's inputs
/// through gates no cell owns (a partial product, a Booth row bit, a sign
/// bit) is an input leaf: the sums hold it as its polynomial over the
/// inputs (bead SAT-playground-skz.4). Returns the input leaves and, per
/// gate, whether it lies in an input leaf's cone (input logic: a Booth
/// digit's encoder and row muxes), which is no adder of any kind.
#[derive(Clone)]
struct InputLeaf {
    sup: Vec<i32>, // the inputs, sorted
    tt: u64,       // the function over them
}

/// A gate as a function of a few of the formula's inputs through gates
/// no cell owns: its inputs, its truth table and the gates of its cone;
/// `None` when the cone holds a produced signal (a cell's output), a
/// unit, more than `MAX_SUPPORT` inputs or more than 64 gates.
fn leaf_function(l: i32, net: &Netlist, produced_all: &HashSet<i32>, memo: &mut HashMap<i32, u64>, work: &mut u64, trace_var: i32) -> Option<(InputLeaf, Vec<i32>)> {
    let nv = net.rep.len();
    if l as usize >= nv || net.units[l as usize] {
        return None;
    }
    if net.gates[l as usize].is_none() {
        return Some((InputLeaf { sup: vec![l], tt: 0b10 }, Vec::new()));
    }
    let mut sup: Vec<i32> = Vec::new();
    let mut seen: HashSet<i32> = HashSet::new();
    let mut stack: Vec<i32> = vec![l];
    let mut gates: Vec<i32> = Vec::new();
    while let Some(u) = stack.pop() {
        let u = net.rep[u as usize].0;
        if !seen.insert(u) {
            continue;
        }
        *work += 1;
        if u != l && produced_all.contains(&u) {
            if l == trace_var {
                eprintln!("structarith: input leaf trace {}: {} is a cell's output", l, u);
            }
            return None;
        }
        match net.gates[u as usize].as_ref() {
            Some(g) => {
                gates.push(u);
                if gates.len() > 64 {
                    return None;
                }
                stack.extend(g.sup.iter().copied());
            }
            None => {
                if net.units[u as usize] {
                    return None;
                }
                sup.push(u);
                if sup.len() > MAX_SUPPORT {
                    return None;
                }
            }
        }
    }
    sup.sort_unstable();
    sup.dedup();
    let tt = cut_truth_table(l, &sup, net, memo, work)?;
    if l == trace_var {
        eprintln!("structarith: input leaf trace {}: {} gates, support {:?}, tt {:#x}", l, gates.len(), sup, tt);
    }
    Some((InputLeaf { sup, tt }, gates))
}

#[allow(clippy::too_many_arguments)]
fn classify_input_leaves(
    cells: &[Cell],
    side_cells: &[Vec<usize>; 2],
    net: &Netlist,
    work: &mut u64,
    stopped: &dyn Fn(u64) -> bool,
    trace_var: i32,
    transparent: &HashSet<i32>,
) -> (HashMap<i32, InputLeaf>, Vec<bool>) {
    let nv = net.rep.len();
    let mut input_logic = vec![false; nv];
    let produced_all: HashSet<i32> = {
        let mut set = HashSet::new();
        for s in 0..2 {
            for &ci in &side_cells[s] {
                set.insert(cells[ci].sum);
                set.insert(cells[ci].carry_var());
            }
        }
        // (a re-read half adder's sum is a function of the inputs, and the
        // leaf it reads is a function of that sum: walked through)
        for t in transparent {
            set.remove(t);
        }
        set
    };
    let mut input_leaves: HashMap<i32, InputLeaf> = HashMap::new();
    let mut memo: HashMap<i32, u64> = HashMap::new();
    for s in 0..2 {
        for &ci in &side_cells[s] {
            for &l in &cells[ci].leaves {
                if l as usize >= nv || produced_all.contains(&l) || transparent.contains(&l) || input_leaves.contains_key(&l) {
                    continue;
                }
                if let Some((il, gates)) = leaf_function(l, net, &produced_all, &mut memo, work, trace_var) {
                    // (the gates strictly under the leaf: the leaf itself
                    // may be a lone carry, the and of two products whose
                    // sum is dead logic, which the lone-gate rules recover)
                    for &g in &gates {
                        if g != l {
                            input_logic[g as usize] = true;
                        }
                    }
                    input_leaves.insert(l, il);
                }
            }
        }
        if stopped(*work) {
            break;
        }
    }
    (input_leaves, input_logic)
}

/// One pass of the operand-grid rule: a partial product with no weight
/// (no cell reads it) is placed from three weighted products over the
/// same inputs in one network, pp(a, b) at exp(a, b') + exp(a', b) -
/// exp(a', b'). Returns whether any was placed.
#[allow(clippy::too_many_arguments)]
fn place_products(
    grid: &[((i32, i32), i32)],
    by_var: &HashMap<i32, Vec<i32>>,
    pps: &HashMap<(i32, i32), i32>,
    net: &Netlist,
    uf: &mut WeightedUf,
    weighted: &mut HashSet<i32>,
    work: &mut u64,
    stopped: &dyn Fn(u64) -> bool,
    trace_var: i32,
    side_roots: Option<(&HashSet<i32>, &HashSet<i32>)>,
    side_positions: &[(Vec<(i32, i32, i64)>, Option<i32>)],
) -> bool {
    // (a product already weighted in a component of one side's cells is
    // not tied into the other side's: the sides come to one scale through
    // the compared signals alone, since a product may sit at two columns,
    // one per side: a Booth correction row's against the AND side's)
    let cross = |r: i32, r1: i32| -> bool {
        match side_roots {
            Some((ra, rb)) => (ra.contains(&r) && !rb.contains(&r) && rb.contains(&r1) && !ra.contains(&r1)) || (rb.contains(&r) && !ra.contains(&r) && ra.contains(&r1) && !rb.contains(&r1)),
            None => false,
        }
    };
    let other = |v: i32, x: i32| -> i32 {
        let g = net.gates[v as usize].as_ref().unwrap();
        if g.sup[0] == x {
            g.sup[1]
        } else {
            g.sup[0]
        }
    };
    let mut placed_any = false;
    for &((a, b), v) in grid {
        // (a product with a weight in a component of its own, read
        // by one cell no other cell feeds, is tied the same way)
        let own_root = if weighted.contains(&v) { Some(uf.find(v).0) } else { None };
        let mut done = false;
        for &v1 in &by_var[&a] {
            if done || v1 == v || !weighted.contains(&v1) {
                continue;
            }
            let b2 = other(v1, a);
            let (r1, e1) = uf.find(v1);
            if own_root == Some(r1) {
                break; // already in that component
            }
            if let Some(r) = own_root {
                if cross(r, r1) {
                    continue;
                }
            }
            for &v2 in &by_var[&b] {
                if v2 == v || !weighted.contains(&v2) {
                    continue;
                }
                let a2 = other(v2, b);
                let (r2, e2) = uf.find(v2);
                let Some(&v3) = pps.get(&(a2.min(b2), a2.max(b2))) else { continue };
                if !weighted.contains(&v3) {
                    continue;
                }
                let (r3, e3) = uf.find(v3);
                *work += 4;
                if stopped(*work) {
                    return false; // (the caller checks the limit)
                }
                if v == trace_var {
                    eprintln!("    grid {}: neighbours {} at ({}, {}), {} at ({}, {}), {} at ({}, {})", v, v1, r1, e1, v2, r2, e2, v3, r3, e3);
                }
                if r1 != r2 || r2 != r3 {
                    continue;
                }
                let e = e1 + e2 - e3;
                if uf.union(v, v1, e - e1) {
                    weighted.insert(v);
                    placed_any = true;
                    done = true;
                }
                break;
            }
        }
    }
    if placed_any {
        return true;
    }
    // then by the inputs' positions along each side's operands (from the
    // products its cells read, at the cells' exponents): a product of two
    // placed inputs sits at the sum of their positions, whether or not
    // three neighbours are weighted (a pruned column multiplier's bottom
    // half adders read products no cell has reached). Each side places
    // into its own component, anchored on a product its cells read
    for (side_pairs, anchor) in side_positions {
        let Some(anchor_v) = *anchor else { continue };
        if side_pairs.is_empty() {
            continue;
        }
        let Some(pos) = operand_positions(side_pairs, work) else { continue };
        for &((a, b), v) in grid {
            if weighted.contains(&v) {
                continue;
            }
            let (Some(&(pa, ca)), Some(&(pb, cb))) = (pos.get(&a), pos.get(&b)) else { continue };
            if ca == cb {
                continue;
            }
            *work += 2;
            if stopped(*work) {
                return false;
            }
            // (into the side's own component, beside one of its cells)
            let (_, e_anchor_now) = uf.find(anchor_v);
            if uf.union(v, anchor_v, pa + pb - e_anchor_now) {
                weighted.insert(v);
                placed_any = true;
                if v == trace_var {
                    eprintln!("    grid {}: placed by positions at {} beside {}", v, pa + pb, anchor_v);
                }
            }
        }
    }
    placed_any
}

/// Whether a truth table over `n` inputs is the and of all of them
/// (`Some(false)`) or its complement, the nand (`Some(true)`). A table over
/// n inputs has 2^n rows: the and is the top row alone, the nand every row
/// but the top (review, 2026-10-23: masks over n bits instead of 2^n took
/// `x or ~y` for a nand, and the proof wrote a false leaf identity).
fn monomial_reading(tt: u64, n: usize) -> Option<bool> {
    if n == 0 || n > MAX_SUPPORT {
        return None;
    }
    let rows = 1usize << n;
    let top = 1u64 << (rows - 1);
    let full = if rows >= 64 { u64::MAX } else { (1u64 << rows) - 1 };
    if tt == top {
        Some(false)
    } else if tt == full ^ top {
        Some(true)
    } else {
        None
    }
}

/// Every input's position along its operand from weighted products
/// (`(a, b, exponent)`: `exponent = pos(a) + pos(b)`), by propagation;
/// the two operands are the two colours. None when the pairs contradict.
fn operand_positions(pairs: &[(i32, i32, i64)], work: &mut u64) -> Option<HashMap<i32, (i64, u8)>> {
    let mut pos: HashMap<i32, (i64, u8)> = HashMap::new();
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 64 {
        changed = false;
        rounds += 1;
        for &(x, y, e) in pairs {
            *work += 1;
            match (pos.get(&x).copied(), pos.get(&y).copied()) {
                (None, None) => {
                    if pos.is_empty() {
                        pos.insert(x, (0, 0));
                        pos.insert(y, (e, 1));
                        changed = true;
                    }
                }
                (Some((px, cx)), None) => {
                    pos.insert(y, (e - px, 1 - cx));
                    changed = true;
                }
                (None, Some((py, cy))) => {
                    pos.insert(x, (e - py, 1 - cy));
                    changed = true;
                }
                (Some((px, cx)), Some((py, cy))) => {
                    if px + py != e || cx == cy {
                        return None;
                    }
                }
            }
        }
    }
    Some(pos)
}

/// The product pairs a side's cells read, each at its reading cell's
/// exponent (the side's own view: a product may sit at another column
/// on the other side).
fn side_product_pairs(cells: &[Cell], side: &[usize], pps: &HashMap<(i32, i32), i32>, uf: &mut WeightedUf, work: &mut u64) -> (Vec<(i32, i32, i64)>, Option<i32>) {
    let by_var: HashMap<i32, (i32, i32)> = pps.iter().map(|(&(a, b), &v)| (v, (a, b))).collect();
    let mut out: Vec<(i32, i32, i64)> = Vec::new();
    let mut seen: HashMap<i32, i64> = HashMap::new();
    let mut anchor: Option<i32> = None; // a cell's sum of the side: the component to place into
    for &ci in side {
        let (_, e) = uf.find(cells[ci].sum);
        for &l in &cells[ci].leaves {
            if let Some(&(a, b)) = by_var.get(&l) {
                match seen.get(&l) {
                    Some(&e0) if e0 != e => {} // read at two columns: no evidence
                    Some(_) => {}
                    None => {
                        seen.insert(l, e);
                        out.push((a, b, e));
                        if anchor.is_none() {
                            anchor = Some(cells[ci].sum);
                        }
                    }
                }
            }
        }
        *work += cells[ci].leaves.len() as u64;
    }
    out.sort_unstable();
    (out, anchor)
}

/// Union-find with a parity: flip(x) xor flip(root(x)).
struct ParityUf {
    parent: Vec<usize>,
    par: Vec<u8>,
}

impl ParityUf {
    fn new(n: usize) -> Self {
        ParityUf { parent: (0..n).collect(), par: vec![0; n] }
    }
    fn find(&mut self, x: usize) -> (usize, u8) {
        let mut path = Vec::new();
        let mut cur = x;
        while self.parent[cur] != cur {
            path.push(cur);
            cur = self.parent[cur];
        }
        let root = cur;
        let mut acc = 0u8;
        for &y in path.iter().rev() {
            acc ^= self.par[y];
            self.par[y] = acc;
            self.parent[y] = root;
        }
        (root, if path.is_empty() { 0 } else { self.par[path[0]] })
    }
    /// Require flip(a) xor flip(b) == d; false on a contradiction.
    fn union(&mut self, a: usize, b: usize, d: u8) -> bool {
        let (ra, pa) = self.find(a);
        let (rb, pb) = self.find(b);
        if ra == rb {
            return pa ^ pb == d;
        }
        self.parent[ra] = rb;
        self.par[ra] = d ^ pa ^ pb;
        true
    }
}

/// Every cell has two arithmetic readings: `la + lb (+ lc) = s + 2c` and,
/// with every literal complemented, `~la + ~lb (+ ~lc) = ~s + 2 ~c` (a
/// half adder's complement is the carry-in-one form). A circuit may use
/// either for any cell, so the readings are chosen together: a signal
/// produced by one cell and read by another must be the same literal on
/// both ends, so that it cancels in the weighted sum. A leaf (a partial
/// product) may be read either way by its readers: the linear forms carry
/// a negated reading as `1 - x`. Returns false when no choice fits.
fn resolve_polarities(cells: &mut [Cell], chosen: &[usize], doubled: &HashMap<i32, usize>) -> bool {
    let mut uf = ParityUf::new(cells.len());
    let mut producer: HashMap<i32, (usize, u8)> = HashMap::new(); // var -> (cell, produced negated)
    for &ci in chosen {
        let c = &cells[ci];
        producer.insert(c.sum, (ci, c.sum_neg as u8));
        producer.insert(c.carry_var(), (ci, (c.carry < 0) as u8));
    }
    // (a signal read doubled, by a cell one column above it, is read as it
    // is written there and complemented by its other readers: 2x - x is x,
    // and ~x is 1 - x, so the reads add up to x plus a constant)
    let in_chosen: HashSet<usize> = chosen.iter().copied().collect();
    for &ci in chosen {
        for i in 0..cells[ci].leaves.len() {
            let x = cells[ci].leaves[i];
            let q = ((cells[ci].negmask >> i) & 1) as u8;
            if let Some(&(pc, p)) = producer.get(&x) {
                let opposite = match doubled.get(&x) {
                    Some(&a) if in_chosen.contains(&a) => (a != ci) as u8,
                    _ => 0,
                };
                if !uf.union(pc, ci, p ^ q ^ opposite) {
                    if debug() {
                        eprintln!("structarith: polarity conflict on signal {} between producer cell {:?} and reader cell {:?}", x, (pc, &cells[pc].leaves, cells[pc].negmask, cells[pc].sum, cells[pc].sum_neg, cells[pc].carry, cells[pc].fa), (ci, &cells[ci].leaves, cells[ci].negmask, cells[ci].sum, cells[ci].sum_neg, cells[ci].carry, cells[ci].fa));
                        // the cells of that component, with their flips
                        let (root, _) = uf.find(ci);
                        for &cj in chosen {
                            let (r, f) = uf.find(cj);
                            if r == root {
                                let c = &cells[cj];
                                let lits: Vec<i32> = (0..c.leaves.len()).map(|i| c.leaf_lit(i)).collect();
                                eprintln!("    cell {} flip {}: {:?} = {} + 2*{}", cj, f, lits, c.sum_lit(), c.carry_lit());
                            }
                        }
                    }
                    return false;
                }
            }
        }
    }
    for &ci in chosen {
        let (_, flip) = uf.find(ci);
        if flip == 1 {
            let c = &mut cells[ci];
            let all = (1u8 << c.leaves.len()) - 1;
            c.negmask ^= all;
            c.sum_neg = !c.sum_neg;
            c.carry = -c.carry;
            if !c.fa {
                c.one = !c.one;
            }
        }
    }
    true
}

/// A linear form over variables: coefficients and a constant, for the
/// integer check of the telescoping sum (a negated literal is 1 - x).
#[derive(Default)]
struct Form {
    coeff: HashMap<i32, i128>,
    constant: i128,
    /// a sum left the i128 range (many cells reading one signal at a huge
    /// weight): the pass declines rather than panic or wrap
    overflow: bool,
}

impl Form {
    fn add_lit(&mut self, lit: i32, w: i128) {
        let e = self.coeff.entry(lit.abs()).or_default();
        let (c, k) = if lit > 0 { (e.checked_add(w), Some(self.constant)) } else { (e.checked_sub(w), self.constant.checked_add(w)) };
        match (c, k) {
            (Some(c), Some(k)) => {
                *e = c;
                self.constant = k;
            }
            _ => self.overflow = true,
        }
    }
    fn add_constant(&mut self, w: i128) {
        match self.constant.checked_add(w) {
            Some(k) => self.constant = k,
            None => self.overflow = true,
        }
    }
    fn prune(&mut self) {
        self.coeff.retain(|_, c| *c != 0);
    }
}

/// A variable the pass introduces for the proof alone (the solver never
/// sees it): the missing output of a lone sum or carry gate, or a bit of
/// the virtual ripple chain under a parallel-prefix final adder (bead
/// SAT-playground-skz.12). Defined as a function of literals.
#[derive(Clone)]
enum FreshDef {
    Xor(Vec<i32>),
    And(Vec<i32>),
    Maj(Vec<i32>),
}

#[derive(Clone)]
struct Fresh {
    var: i32,
    def: FreshDef,
}

/// The clauses saying `v` is the function `tt` of `leaves`: one per
/// assignment of the leaves.
fn minterm_clauses(v: i32, leaves: &[i32], tt: u64) -> Vec<Vec<i32>> {
    let n = leaves.len();
    let mut out = Vec::with_capacity(1 << n);
    for m in 0..(1u32 << n) {
        let mut c: Vec<i32> = (0..n).map(|i| if (m >> i) & 1 == 1 { -leaves[i] } else { leaves[i] }).collect();
        c.push(if (tt >> m) & 1 == 1 { v } else { -v });
        out.push(c);
    }
    out
}

/// The defining clauses of a fresh variable, over the variables of its
/// literals.
fn fresh_clauses(fr: &Fresh) -> Vec<Vec<i32>> {
    let (lits, kind) = match &fr.def {
        FreshDef::Xor(l) => (l, 0u8),
        FreshDef::And(l) => (l, 1u8),
        FreshDef::Maj(l) => (l, 2u8),
    };
    // (one clause per row, the value from the definition's kind: a table
    // in a u64 holds six inputs, and a mapped Booth side's network carry
    // is an xor of seven, bead skz.13)
    let n = lits.len();
    let mut out = Vec::with_capacity(1 << n);
    for m in 0..(1u64 << n) {
        let vals: Vec<bool> = lits.iter().enumerate().map(|(i, &l)| (((m >> i) & 1) == 1) != (l < 0)).collect();
        let ones = vals.iter().filter(|&&b| b).count();
        let f = match kind {
            0 => ones % 2 == 1,
            1 => ones == n,
            _ => 2 * ones > n,
        };
        let mut c: Vec<i32> = (0..n).map(|i| if (m >> i) & 1 == 1 { -lits[i].abs() } else { lits[i].abs() }).collect();
        c.push(if f { fr.var } else { -fr.var });
        out.push(c);
    }
    out
}

/// The equivalence of two literals, proven by kitten over the clauses
/// given: the learned clauses of its core, in an order where each is
/// implied by unit propagation from the earlier ones and the clauses,
/// then the two clauses of the equivalence themselves (implied the same
/// way). None when the equivalence does not hold or the budget ran out.
/// Kitten's ticks are charged to `work`.
fn kitten_equivalence(clauses: &[Vec<i32>], a: i32, b: i32, budget: u64, work: &mut u64, stop: &std::sync::atomic::AtomicBool) -> Option<Vec<Vec<i32>>> {
    use crate::kitten;
    let mut index: HashMap<i32, u32> = HashMap::new();
    let idx = |v: i32, index: &mut HashMap<i32, u32>| -> u32 {
        let n = index.len() as u32;
        *index.entry(v).or_insert(n)
    };
    for c in clauses {
        for &l in c {
            idx(l.abs(), &mut index);
        }
    }
    idx(a.abs(), &mut index);
    idx(b.abs(), &mut index);
    let back: HashMap<u32, i32> = index.iter().map(|(&v, &i)| (i, v)).collect();
    let elit = |l: i32| -> u32 { 2 * index[&l.abs()] + (l < 0) as u32 };
    let export = |e: u32| -> i32 {
        let v = back[&(e / 2)];
        if e & 1 == 1 {
            -v
        } else {
            v
        }
    };
    // kitten counts its work in a solver's statistics: a scratch one, its
    // ticks moved to the pass's counter after each call
    let mut scratch = crate::internal::Solver::default();
    let mut k = kitten::kitten_embedded();
    kitten::kitten_track_antecedents(&mut k);
    let mut buf: Vec<u32> = Vec::new();
    if debug() {
        eprintln!("structarith: kitten on {} = {}: work {} at entry", a, b, *work);
    }
    for c in clauses {
        buf.clear();
        buf.extend(c.iter().map(|&l| elit(l)));
        kitten::kitten_clause(&mut k, &mut scratch, &buf);
    }
    *work += clauses.len() as u64;
    if debug() {
        eprintln!("structarith: kitten on {} = {}: work {} after the import, scratch ticks {}", a, b, *work, scratch.statistics.kitten_ticks);
    }
    let mut out: Vec<Vec<i32>> = Vec::new();
    let mut written: HashSet<Vec<i32>> = HashSet::new();
    // kitten polls the termination flag of the solver it is handed, the
    // scratch one here: so it runs in pieces of a million ticks, the live
    // flag read and the work published between them
    const PIECE: u64 = 1 << 20;
    for (x, y) in [(a, -b), (-a, b)] {
        let mut res;
        loop {
            if budget <= *work || stop.load(std::sync::atomic::Ordering::Relaxed) {
                if debug() {
                    eprintln!("structarith: kitten stopped on {} = {}: work {} of {}, stop flag {}", a, b, *work, budget, stop.load(std::sync::atomic::Ordering::Relaxed));
                }
                kitten::kitten_release(k);
                return None;
            }
            let before = scratch.statistics.kitten_ticks;
            kitten::kitten_set_ticks_limit(&mut k, &scratch, PIECE.min(budget - *work));
            kitten::kitten_assume(&mut k, elit(x));
            kitten::kitten_assume(&mut k, elit(y));
            res = kitten::kitten_solve(&mut k, &mut scratch);
            *work += scratch.statistics.kitten_ticks - before;
            if res != 0 {
                break;
            }
        }
        if res != 20 {
            if debug() {
                eprintln!("structarith: kitten says {} on {} = {} with {} assumed, {} ticks", res, a, b, x, scratch.statistics.kitten_ticks);
                if res == 10 {
                    if let Ok(path) = std::env::var("SAT_STRUCT_DEBUG_LEMMA") {
                        // the clauses and the model, for a look offline
                        let mut text = String::new();
                        for c in clauses {
                            text.push_str(&c.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(" "));
                            text.push_str(" 0\n");
                        }
                        text.push_str("c model");
                        for (&v, &i) in index.iter() {
                            let val = kitten::kitten_value(&k, 2 * i);
                            text.push_str(&format!(" {}", if val > 0 { v } else { -v }));
                        }
                        text.push_str(&format!("\nc lemma {} {}\n", a, b));
                        let _ = std::fs::write(format!("{}.{}.{}", path, a, b), text);
                    }
                }
            }
            kitten::kitten_release(k);
            return None;
        }
        kitten::kitten_compute_clausal_core(&mut k);
        kitten::kitten_traverse_core_clauses(&mut k, |learned, lits| {
            if learned {
                let mut c: Vec<i32> = lits.iter().map(|&e| export(e)).collect();
                c.sort_unstable();
                if written.insert(c.clone()) {
                    out.push(c);
                }
            }
        });
    }
    kitten::kitten_release(k);
    Some(out)
}

/// Every variable of a clause set under 64 random assignments of the
/// inputs at once, by unit propagation to a fixpoint (the clauses are a
/// circuit's: a full assignment of a gate's inputs propagates its output).
/// Returns each variable's value word and the word of the patterns under
/// which it was assigned; a pattern that ran into a conflict is dropped
/// from every word.
fn simulate_clauses(clauses: &[Vec<i32>], is_input: &dyn Fn(i32) -> bool, seed: u64, work: &mut u64, stopped: &dyn Fn(u64) -> bool) -> Option<HashMap<i32, (u64, u64)>> {
    // (charged as it goes, a clause its length per visit, and stopped
    // between pieces: review, 2026-10-23)
    if stopped(work.saturating_add(clauses.len() as u64)) {
        return None;
    }
    let mut value: HashMap<i32, u64> = HashMap::new();
    let mut known: HashMap<i32, u64> = HashMap::new();
    let mut vars: Vec<i32> = clauses.iter().flatten().map(|l| l.abs()).collect();
    vars.sort_unstable();
    vars.dedup();
    let mut state = seed | 1;
    let mut next = || -> u64 {
        // xorshift64*
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545F4914F6CDD1D)
    };
    for &v in &vars {
        if is_input(v) {
            value.insert(v, next());
            known.insert(v, u64::MAX);
        } else {
            value.insert(v, 0);
            known.insert(v, 0);
        }
    }
    *work += vars.len() as u64;
    if stopped(*work) {
        return None;
    }
    let mut conflict = 0u64;
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 4096 {
        changed = false;
        rounds += 1;
        for (ci, c) in clauses.iter().enumerate() {
            *work += c.len() as u64 + 1;
            if ci % 16 == 15 && stopped(*work) {
                return None;
            }
            // the patterns under which the clause is satisfied, and per
            // literal those under which every other literal is false
            let mut sat = 0u64;
            let mut false_masks: Vec<u64> = Vec::with_capacity(c.len());
            for &l in c {
                let v = l.abs();
                let kn = known[&v];
                let val = if l > 0 { value[&v] } else { !value[&v] };
                sat |= kn & val;
                false_masks.push(kn & !val);
            }
            let mut all_false = u64::MAX;
            for &m in &false_masks {
                all_false &= m;
            }
            conflict |= all_false;
            for (i, &l) in c.iter().enumerate() {
                let v = l.abs();
                let mut others = u64::MAX;
                for (j, &m) in false_masks.iter().enumerate() {
                    if j != i {
                        others &= m;
                    }
                }
                let unit = others & !sat & !known[&v];
                if unit != 0 {
                    *known.get_mut(&v).unwrap() |= unit;
                    let e = value.get_mut(&v).unwrap();
                    if l > 0 {
                        *e |= unit;
                    } else {
                        *e &= !unit;
                    }
                    changed = true;
                }
            }
        }
    }
    if stopped(*work) {
        return None;
    }
    let keep = !conflict;
    *work += vars.len() as u64;
    Some(vars.iter().map(|&v| (v, (value[&v] & keep, known[&v] & keep))).collect())
}

pub fn run(f: &Formula, derived: &mut Derived, budget: u64, hard: u64, work: &mut u64, stop: &std::sync::atomic::AtomicBool) {
    // the work goes straight into the statistics' tick counter, so a signal
    // during the pass reports it; with no allowance left nothing runs
    if *work >= budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    // the lemmas (kitten proving a carry network's output equal to the
    // chain's bit) run only on a miter the pass has otherwise read in full,
    // and get an allowance of their own on top of the pass's: what they
    // use is added to the pass's budget, so the pass goes on after one.
    // Nothing runs past `hard`, the solver's own tick limit
    let hard = hard.max(budget);
    let extra = std::cell::Cell::new(0u64);
    let stopped = |w: u64| w > budget.saturating_add(extra.get()).min(hard) || stop.load(std::sync::atomic::Ordering::Relaxed);
    let lemma_allowance: u64 = (budget.saturating_sub(*work)).saturating_mul(10);
    let dbg = debug();
    let t0 = std::time::Instant::now();
    let phase = |name: &str, work: u64| {
        if dbg {
            eprintln!("structarith: [{:.2} s, {}K] {}", t0.elapsed().as_secs_f64(), work / 1000, name);
        }
    };
    let Some(mut net) = recover_gates(f, work, &stopped) else {
        if dbg {
            eprintln!("structarith: no gates or stopped");
        }
        return;
    };
    let Some(merged) = structural_hash(&mut net, work, &stopped) else { return };
    if dbg {
        eprintln!("structarith: {} gates, {} merged, {} deadlocks", net.order.len(), merged, net.deadlocks);
        if let Ok(v) = std::env::var("SAT_STRUCT_DEBUG_VAR") {
            let v: i32 = v.parse().unwrap_or(0);
            if v > 0 && (v as usize) < net.rep.len() {
                eprintln!("    var {}: rep {:?}, gate {:?}, in order at {:?}", v, net.rep[v as usize], net.gates[v as usize].as_ref().map(|g| (g.sup.clone(), g.tt)), net.order.iter().position(|&u| u == v));
                if let Some(g) = net.gates[v as usize].as_ref() {
                    for &u in &g.sup {
                        eprintln!("      input {}: rep {:?}, gate {:?}, in order at {:?}", u, net.rep[u as usize], net.gates[u as usize].as_ref().map(|g| (g.sup.clone(), g.tt)), net.order.iter().position(|&x| x == u));
                    }
                    let rs: Vec<i32> = g.sup.iter().map(|&u| net.rep[u as usize].0).collect();
                    for &u in &net.order {
                        let h = net.gates[u as usize].as_ref().unwrap();
                        let hs: Vec<i32> = h.sup.iter().map(|&x| net.rep[x as usize].0).collect();
                        if u != v && hs == rs {
                            eprintln!("      same rep support: {} gate {:?} tt {} rep {:?} at {:?}", u, h.sup, h.tt, net.rep[u as usize], net.order.iter().position(|&x| x == u));
                        }
                    }
                }
            }
        }
    }
    if stopped(*work) {
        return;
    }
    phase("gates", *work);
    // how many representative gates read each representative signal
    let readers: Vec<u32> = {
        let mut r = vec![0u32; net.rep.len()];
        for &v in &net.order {
            if net.rep[v as usize].0 != v {
                continue;
            }
            for &u in &net.gates[v as usize].as_ref().unwrap().sup {
                r[net.rep[u as usize].0 as usize] += 1;
            }
        }
        *work += net.order.len() as u64;
        // an inner xor whose only reader was a collapsed xor is dead: its
        // reads leave the counts (a carry built as (t xor b) xor b would
        // give t a second reader that is not there; review, 2026-10-23)
        // (each inner xor once, and only one counted above: a
        // representative; review, 2026-10-23)
        let mut discounted: HashSet<i32> = HashSet::new();
        for &(_, w, _) in &net.collapsed {
            if net.rep[w as usize] != (w, false) || !discounted.insert(w) {
                continue;
            }
            if r[w as usize] == 0 {
                for &u in &net.gates[w as usize].as_ref().unwrap().sup {
                    let ru = net.rep[u as usize].0 as usize;
                    r[ru] = r[ru].saturating_sub(1);
                }
            }
        }
        *work += net.collapsed.len() as u64;
        r
    };

    // the proof's fresh variables (a fused leaf's xor, a lone gate's
    // missing partner, a chain cell): their numbers continue from the last
    // the proof has used (shared with the parity engine's proof, which
    // takes over the stream later)
    let mut fresh: Vec<Fresh> = Vec::new();
    let mut next_fresh: i32 = derived.last_var.max(f.vars) + 1;
    let Some((mut cells, cuts)) = find_adders(&net, &readers, work, &stopped, &mut fresh, &mut next_fresh) else {
        return;
    };
    // a half adder over (a, y) whose sum g only fed the xor a xor g, which
    // structural hashing collapsed to y: the circuit adds g's complement
    // and a, and the sum it reads is y (a Booth row bit and its sign, the
    // sum the row's raw bit again: the generator's half adder for the
    // row's +1). Read the same identity the other way round: a + y = g +
    // 2c is a + ~g = ~y + 2c, the leaves a and g, the sum y; the proof
    // still splits over (a, y), where everything propagates (bead skz.4)
    let mut transparent: HashSet<i32> = HashSet::new(); // the re-read cells' sums: functions of the inputs, walked through when a leaf's function is read
    // (the half adders by sum, once: a lookup per collapse, not a scan)
    let mut ha_by_sum: HashMap<i32, usize> = HashMap::new();
    if !net.collapsed.is_empty() {
        for (ci, c) in cells.iter().enumerate() {
            if !c.fa && !c.one && c.split.is_none() && c.leaves.len() == 2 {
                ha_by_sum.entry(c.sum).or_insert(ci);
            }
        }
        *work += cells.len() as u64;
    }
    for &(a, g, y) in &net.collapsed {
        *work += 2;
        if stopped(*work) {
            return;
        }
        // (the half adder's cut, its sum and the leaves the split runs over
        // all their own representatives: the gates read these very
        // variables, so the split propagates them without an alias's
        // proof; review, 2026-10-23)
        if [a, g, y].iter().any(|&x| net.rep[x as usize] != (x, false)) {
            continue;
        }
        let Some(&ci) = ha_by_sum.get(&g) else { continue };
        let mut w = vec![a, y];
        w.sort_unstable();
        if !({
            let mut l = cells[ci].leaves.clone();
            l.sort_unstable();
            l == w
        }) {
            continue;
        }
        // (the carry a gate over those variables or the sum itself: the
        // split over the two propagates the sum, then the carry, through
        // the gates' own inputs, no alias needed)
        let cv = cells[ci].carry_var();
        let Some(gc) = net.gates[cv as usize].as_ref() else { continue };
        if net.rep[cv as usize] != (cv, false) || gc.sup.iter().any(|&x| x != a && x != y && x != g) {
            continue;
        }
        // (and only when the sum fed nothing but the collapsed xor and the
        // half adder's own carry: a sum another gate reads keeps its half
        // adder; review, 2026-10-23)
        if readers[g as usize] != gc.sup.contains(&g) as u32 {
            continue;
        }
        let c = &cells[ci];
        let ia = c.leaves.iter().position(|&l| l == a).unwrap();
        let iy = 1 - ia;
        let a_neg = (c.negmask >> ia) & 1 == 1;
        let y_neg = (c.negmask >> iy) & 1 == 1;
        let new_cell = Cell {
            fa: false,
            one: false,
            leaves: vec![a, g],
            negmask: (a_neg as u8) | ((!c.sum_neg as u8) << 1),
            sum: y,
            sum_neg: !y_neg,
            carry: c.carry,
            split: Some({
                let mut l = c.leaves.clone();
                l.sort_unstable();
                l
            }),
        };
        if dbg {
            eprintln!("structarith: the half adder over {:?} with the collapsed sum {} read the other way round: leaves [{}, {}], sum {}", c.leaves, g, a, g, y);
        }
        cells[ci] = new_cell;
        transparent.insert(y);
        *work += 8;
    }
    if dbg {
        if let Ok(v) = std::env::var("SAT_STRUCT_DEBUG_VAR") {
            let v: i32 = v.parse().unwrap_or(0);
            for (ci, c) in cells.iter().enumerate() {
                if c.sum == v || c.carry_var() == v || c.leaves.contains(&v) {
                    let lits: Vec<i32> = (0..c.leaves.len()).map(|i| c.leaf_lit(i)).collect();
                    eprintln!("    cell {} touches {}: {:?} (+{}) = {} + 2*{} (fa {}, split {:?})", ci, v, lits, c.one as u8, c.sum_lit(), c.carry_lit(), c.fa, c.split);
                }
            }
        }
    }
    phase("adders", *work);
    if dbg {
        eprintln!("structarith: {} cells", cells.len());
    }
    // a two-input cell whose inputs are the same signal (equal or
    // complementary on 64 random patterns) is a compare of the two
    // circuits' bits (what the miter asserts), not an adder: the compares
    // of the bits the miter does not assert are still in the file
    {
        let Some(sim) = simulate(&net, work, &stopped) else { return };
        let before = cells.len();
        cells.retain(|c| {
            if c.leaves.len() != 2 || c.leaves.iter().any(|&l| l as usize >= sim.len()) {
                return true; // (a fused leaf is a fresh variable: not simulated)
            }
            let (a, b) = (sim[c.leaves[0] as usize], sim[c.leaves[1] as usize]);
            // (a deep carry is rarely one on random inputs: two signals
            // constant over the patterns are not known to be one signal)
            let constant = a == 0 || a == u64::MAX;
            let keep = constant || (a != b && a != !b);
            if !keep && dbg {
                eprintln!("structarith: cell over two copies of one signal: leaves {:?} (sim {:#x} {:#x}) sum {} carry {}", c.leaves, a, b, c.sum, c.carry);
            }
            keep
        });
        if dbg {
            eprintln!("structarith: {} cells over two copies of one signal dropped", before - cells.len());
        }
    }
    if cells.len() < 4 {
        if dbg {
            eprintln!("structarith: too few cells");
        }
        return;
    }
    // the two cones: walk from the unit clauses through and/or gates to the
    // compared signals; a cell met on the way whose inputs have large,
    // nearly disjoint cones is the compare, not an adder
    let nv = net.rep.len();
    let bare_input = |v: i32| (v as usize) < nv && net.gates[v as usize].is_none();
    let trace_var: i32 = std::env::var("SAT_STRUCT_DEBUG_VIRT").ok().and_then(|t| t.parse().ok()).unwrap_or(0);
    let top_vars: Vec<i32> = {
        let mut s: HashSet<i32> = HashSet::new();
        for i in 0..f.len() {
            if stopped(*work) {
                return;
            }
            *work += 1;
            if !net.used[i] && !f.taut[i] {
                for &l in f.clause(i) {
                    s.insert(l.abs());
                }
                *work += f.clause(i).len() as u64;
            }
        }
        s.into_iter().collect()
    };
    let unit_vars: Vec<i32> = (1..nv as i32).filter(|&v| net.units[v as usize]).collect();
    let mut cone_cache: HashMap<i32, HashSet<i32>> = HashMap::new();
    // past the budget every cone comes back empty and `aborted` is set; each
    // phase that looks at cones checks it before using what it found
    let aborted = std::cell::Cell::new(false);
    let mut cone_of = |rt: i32, net: &Netlist, work: &mut u64| -> HashSet<i32> {
        if aborted.get() {
            return HashSet::new();
        }
        if let Some(c) = cone_cache.get(&rt) {
            *work += 1;
            return c.clone();
        }
        let c = cone(rt, net, work);
        if stopped(*work) {
            aborted.set(true);
            return HashSet::new();
        }
        cone_cache.insert(rt, c.clone());
        c
    };
    let mut compared: Vec<i32> = Vec::new();
    let mut compare_pairs: Vec<(i32, i32)> = Vec::new();
    for _round in 0..3 {
        let mut cell_out: HashMap<i32, usize> = HashMap::new();
        for (ci, c) in cells.iter().enumerate() {
            cell_out.insert(net.rep[c.sum as usize].0, ci);
            cell_out.insert(net.rep[c.carry_var() as usize].0, ci);
        }
        let mut cmp: HashSet<i32> = top_vars.iter().copied().filter(|v| !net.units[*v as usize]).collect();
        let mut pairs: Vec<(i32, i32)> = Vec::new();
        let mut reached: HashSet<i32> = HashSet::new();
        let mut stack: Vec<(i32, u32)> = unit_vars.iter().map(|&u| (u, 0)).collect();
        while let Some((u0, depth)) = stack.pop() {
            *work += 1;
            if aborted.get() || stopped(*work) {
                return;
            }
            let u = net.rep[u0 as usize].0;
            let Some(g) = net.gates[u as usize].as_ref() else { continue };
            if reached.contains(&u) || depth > 12 {
                continue;
            }
            reached.insert(u);
            if g.sup.len() == 1 {
                stack.push((g.sup[0], depth));
                continue;
            }
            if cell_out.contains_key(&u) {
                cmp.insert(u);
                continue;
            }
            let two = g.sup.len() == 2;
            if two && (g.tt == XOR2 || g.tt == XNOR2) {
                let (a, b) = (net.rep[g.sup[0] as usize].0, net.rep[g.sup[1] as usize].0);
                let ca = cone_of(a, &net, work);
                let cb = cone_of(b, &net, work);
                if apart(&ca, &cb) {
                    cmp.insert(a);
                    cmp.insert(b);
                    pairs.push((a, b));
                } else {
                    cmp.insert(u);
                }
            } else if two && (and_family(g.tt).is_some() || or_family(g.tt).is_some()) {
                for &x in &g.sup {
                    stack.push((x, depth + 1));
                }
            } else {
                cmp.insert(u);
            }
        }
        let mut drop: Vec<usize> = Vec::new();
        for (ci, c) in cells.iter().enumerate() {
            if c.leaves.len() != 2 {
                continue;
            }
            let (sv, cv) = (net.rep[c.sum as usize].0, net.rep[c.carry_var() as usize].0);
            if !reached.contains(&sv) && !reached.contains(&cv) {
                continue;
            }
            let ca = cone_of(c.leaves[0], &net, work);
            let cb = cone_of(c.leaves[1], &net, work);
            if ca.len().min(cb.len()) >= MIN_COMPARE_CONE && apart(&ca, &cb) {
                if dbg {
                    eprintln!("structarith: dropped cell {} leaves {:?} (cones {} and {}, shared {}) sum {} carry {} fa {}", ci, c.leaves, ca.len(), cb.len(), ca.intersection(&cb).count(), c.sum, c.carry, c.fa);
                }
                drop.push(ci);
            }
        }
        compared = cmp.into_iter().collect();
        compare_pairs = pairs;
        if aborted.get() || stopped(*work) {
            return;
        }
        if drop.is_empty() {
            break;
        }
        let mut i = 0;
        cells.retain(|_| {
            let keep = !drop.contains(&i);
            i += 1;
            keep
        });
        if stopped(*work) {
            return;
        }
    }
    compared.sort_unstable();
    // a cell that reads a compared signal is the compare logic (an xor and
    // an and over two output bits), not an adder
    {
        let cmp: HashSet<i32> = compared.iter().copied().collect();
        cells.retain(|c| !c.leaves.iter().any(|l| cmp.contains(l)));
    }
    // the compares of the bits the miter does not assert are still in the
    // file, unconstrained: an xor and an and over two output bits, read by
    // no cell. Weights over every cell first (a sum keeps the weight, a
    // carry doubles it; cells in circuit order, a cell that contradicts the
    // weights so far is left out); then a two-input cell whose outputs no
    // cell reads, below its network's top weight, is such a compare and
    // goes. A real cell's carry goes on up except at the very top.
    {
        if stopped(*work + 2 * nv as u64) {
            return;
        }
        *work += 2 * nv as u64; // the position table and the union-find below
        let mut pos: Vec<u32> = vec![0; nv];
        for (i, &v) in net.order.iter().enumerate() {
            pos[v as usize] = i as u32;
        }
        let mut order: Vec<usize> = (0..cells.len()).collect();
        order.sort_by_key(|&ci| (pos[net.rep[cells[ci].sum as usize].0 as usize], !cells[ci].fa, ci));
        let mut uf0 = WeightedUf::new(nv);
        let produced_any: HashSet<i32> = cells.iter().flat_map(|c| [c.sum, c.carry_var()]).collect();
        let mut ok = vec![false; cells.len()];
        for ci in order {
            let c = &cells[ci];
            // (the sum anchors the cell's weight; only a leaf some cell
            // produces ties cells together: a leaf that is a function of
            // the inputs may be read at two weights, as a Booth digit's
            // sign bit is, bead SAT-playground-skz.4)
            let base = c.sum;
            let mut trial: Vec<(i32, i32, i64)> = c.leaves.iter().filter(|&&x| produced_any.contains(&x)).map(|&x| (x, base, 0)).collect();
            trial.push((c.carry_var(), base, 1));
            let eqs: Vec<(i32, i32, i64)> = trial
                .iter()
                .map(|&(a, b, d)| {
                    let (ra, oa) = uf0.find(a);
                    let (rb, ob) = uf0.find(b);
                    (ra, rb, d - oa + ob)
                })
                .collect();
            let fine = consistent(&eqs);
            if fine {
                for &(a, b, d) in &trial {
                    uf0.union(a, b, d);
                }
                ok[ci] = true;
            }
            *work += 8;
        }
        *work += 2 * cells.len() as u64; // the read set and the top table
        let read: HashSet<i32> = cells.iter().enumerate().filter(|(ci, _)| ok[*ci]).flat_map(|(_, c)| c.leaves.iter().copied()).collect();
        let mut top: HashMap<i32, i64> = HashMap::new(); // network root -> highest cell exponent
        for (ci, c) in cells.iter().enumerate() {
            if ok[ci] {
                let (r, o) = uf0.find(c.sum);
                let e = top.entry(r).or_insert(i64::MIN);
                *e = (*e).max(o);
            }
        }
        let mut keep: Vec<bool> = vec![true; cells.len()];
        for (ci, c) in cells.iter().enumerate() {
            if !ok[ci] {
                continue;
            }
            // (a cell whose outputs some gate reads, if no cell, is not a
            // dangling compare: the half adders feeding a parallel-prefix
            // final adder are such cells)
            let unread = readers[net.rep[c.sum as usize].0 as usize] == 0 && readers[net.rep[c.carry_var() as usize].0 as usize] == 0;
            // (a Booth digit's encoder is an xor and an and over two bits of
            // the multiplier: a half adder by function that adds bare
            // inputs and feeds no cell; it is no part of the sums, bead
            // SAT-playground-skz.4)
            let bare = c.leaves.iter().all(|&l| (l as usize) < nv && net.gates[l as usize].is_none());
            if c.leaves.len() == 2 && bare && !read.contains(&c.sum) && !read.contains(&c.carry_var()) {
                keep[ci] = false;
                if dbg {
                    eprintln!("structarith: cell {} over the inputs {:?} feeds no cell: an encoder, dropped", ci, c.leaves);
                }
                continue;
            }
            if c.leaves.len() == 2 && unread && !read.contains(&c.sum) && !read.contains(&c.carry_var()) {
                let (r, o) = uf0.find(c.sum);
                if o + 2 <= top[&r] {
                    keep[ci] = false;
                    if dbg {
                        eprintln!("structarith: dead-end cell {} leaves {:?} at weight exponent {} of {} dropped", ci, c.leaves, o, top[&r]);
                        for &l in &c.leaves {
                            for (cj, d) in cells.iter().enumerate() {
                                if d.sum == l || d.carry_var() == l {
                                    let (rj, oj) = uf0.find(d.sum);
                                    eprintln!("    leaf {} is {} of cell {} (ok {}, root {} exponent {}, leaves {:?})", l, if d.sum == l { "the sum" } else { "the carry" }, cj, ok[cj], rj, oj, d.leaves);
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut i = 0;
        cells.retain(|_| {
            let k = keep[i];
            i += 1;
            k
        });
    }
    // the two sides: the widest compare pair whose cones are apart, else
    // the two largest compared cones that are apart
    let mut candidates: Vec<i32> = Vec::new();
    if let Some(&(a, b)) = compare_pairs.iter().max_by_key(|&&(a, b)| cone_of(a, &net, work).len().min(cone_of(b, &net, work).len())) {
        candidates.push(a);
        candidates.push(b);
    }
    let mut rest: Vec<i32> = compared.iter().copied().filter(|v| !candidates.contains(v)).take(64).collect();
    rest.sort_by_key(|&v| std::cmp::Reverse(cone_of(v, &net, work).len()));
    candidates.extend(rest.into_iter().take(8));
    let mut best: Option<(i32, i32, usize)> = None;
    for i in 0..candidates.len() {
        for j in i + 1..candidates.len() {
            let ca = cone_of(candidates[i], &net, work);
            let cb = cone_of(candidates[j], &net, work);
            if ca.is_empty() || cb.is_empty() || !apart(&ca, &cb) {
                continue;
            }
            let size = ca.len().min(cb.len());
            if best.map_or(true, |(_, _, s)| size > s) {
                best = Some((candidates[i], candidates[j], size));
            }
        }
    }
    if aborted.get() || stopped(*work) {
        return;
    }
    let Some((side_a, side_b, _)) = best else {
        if dbg {
            eprintln!("structarith: no two cones ({} compared)", compared.len());
        }
        return;
    };
    let cone_a = cone_of(side_a, &net, work);
    let cone_b = cone_of(side_b, &net, work);
    if aborted.get() || stopped(*work) {
        return;
    }
    if dbg {
        eprintln!("structarith: sides {} ({} gates) and {} ({} gates)", side_a, cone_a.len(), side_b, cone_b.len());
    }
    // a variable's representative; a fresh variable (a cell's output the
    // netlist does not compute) is its own
    let rep_of = |v: i32| -> i32 { if (v as usize) < nv { net.rep[v as usize].0 } else { v } };
    // bead SAT-playground-skz.13: a technology-mapped Booth side's cells by
    // function. They replace the side's structural cells when they are
    // more; a structural cell that shares a leaf or an output with them,
    // or has an output inside one, is dropped
    let mut virtual_net: [Option<i32>; 2] = [None, None];
    let mut booth_rows: [Vec<(i32, i64)>; 2] = [Vec::new(), Vec::new()]; // a side's row bits no cell reads, with their columns
    let mut booth_cols: Vec<(usize, i64)> = Vec::new(); // a cell by function, its column
    let mut booth_side: HashMap<usize, usize> = HashMap::new(); // a cell by function, its side (a top cell's outputs may both be fresh)
    // (both sides first, then the structural cells of the sides taken are
    // dropped in one pass, then the new cells appended: their indices are
    // recorded once nothing moves them any more; review, 2026-10-24)
    let in_cone_of = |c: &Cell, cone_s: &HashSet<i32>| cone_s.contains(&rep_of(c.sum)) || cone_s.contains(&rep_of(c.carry_var()));
    let mut taken: [Option<booth::BoothCells>; 2] = [None, None];
    for s in 0..2 {
        let cone_s = if s == 0 { &cone_a } else { &cone_b };
        let out_s = if s == 0 { side_a } else { side_b };
        let structural = cells.iter().filter(|c| in_cone_of(c, cone_s)).count();
        *work += cells.len() as u64;
        let Some(bc) = booth::booth_cells(&net, &readers, cone_s, out_s, structural, next_fresh, work, &stopped, dbg) else {
            if stopped(*work) {
                return;
            }
            continue;
        };
        if dbg {
            eprintln!("structarith: booth: side {}: {} cells by function ({} row gates) against {} structural ones", s, bc.cells.len(), bc.row_gates, structural);
        }
        if bc.cells.len() <= structural {
            continue;
        }
        // (its fresh numbers are spent: the other side's continue past them)
        if let Some(m) = bc.fresh.iter().map(|fr| fr.var).max() {
            next_fresh = next_fresh.max(m + 1);
        }
        taken[s] = Some(bc);
    }
    if taken.iter().any(|t| t.is_some()) {
        // (all of a taken side's structural cells: what the new ones leave
        // is the carry network's, a structural cell there would read
        // inside it)
        let before = cells.len();
        cells.retain(|c| !((taken[0].is_some() && in_cone_of(c, &cone_a)) || (taken[1].is_some() && in_cone_of(c, &cone_b))));
        *work += before as u64;
        if dbg {
            eprintln!("structarith: booth: {} structural cells dropped", before - cells.len());
        }
        for s in 0..2 {
            let Some(bc) = taken[s].take() else { continue };
            virtual_net[s] = bc.virtual_net;
            booth_rows[s] = bc.rows_left;
            fresh.extend(bc.fresh);
            booth_cols.extend((cells.len()..).zip(bc.cols.iter().copied()));
            booth_side.extend((cells.len()..cells.len() + bc.cells.len()).map(|ci| (ci, s)));
            cells.extend(bc.cells);
        }
    }
    // the cells of each side: those with an output in the side's cone, and
    // then every cell that consumes an output of a cell the side alone has
    // (a miter on one bit has cells above that bit; without them two
    // carries would dangle at one weight)
    let mut in_side: [Vec<bool>; 2] = [vec![false; cells.len()], vec![false; cells.len()]];
    for (ci, c) in cells.iter().enumerate() {
        let (sv, cv) = (rep_of(c.sum), rep_of(c.carry_var()));
        in_side[0][ci] = cone_a.contains(&sv) || cone_a.contains(&cv);
        in_side[1][ci] = cone_b.contains(&sv) || cone_b.contains(&cv);
        if let Some(&s) = booth_side.get(&ci) {
            in_side[s][ci] = true;
            in_side[1 - s][ci] = false;
        }
    }
    // no closure beyond the cones: the cells above the compared bit may be
    // a different adder, even a faulty one (the miter's fault may be there),
    // and the carries that leave the cone are handled as one top block
    // the weights: a sum keeps the weight, a carry doubles it; the cells
    // are accepted in a consistent set (full adders first)
    if stopped(*work + 3 * nv as u64) {
        return; // the union-find and the position table below are not affordable
    }
    *work += 3 * nv as u64;
    let mut uf = WeightedUf::new(nv);
    // (the cells by function come with their columns: one network, even
    // where no signal ties two of them, a cell over row bits whose outputs
    // only the carry network reads)
    if let Some(&(c0, e0)) = booth_cols.first() {
        for &(ci, e) in &booth_cols {
            uf.union(cells[ci].sum, cells[c0].sum, e - e0);
        }
        *work += booth_cols.len() as u64;
    }
    if let Ok(w) = std::env::var("SAT_STRUCT_DEBUG_WATCH") {
        // (debug: "a,b" watches two signals, anything else the compared ones)
        let parts: Vec<i32> = w.split(',').filter_map(|t| t.trim().parse().ok()).collect();
        uf.watch = Some(if parts.len() == 2 { (parts[0], parts[1]) } else { (side_a, side_b) });
    }
    let mut side_cells: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
    // cells in circuit order, the ones nearest the inputs first: a false
    // cell shape at the top (the compare logic over two output bits)
    // then meets a weight system the real cells have fixed, and is the
    // one rejected
    let mut pos: Vec<u32> = vec![0; nv];
    for (i, &v) in net.order.iter().enumerate() {
        pos[v as usize] = i as u32;
    }
    let mut cell_order: Vec<usize> = (0..cells.len()).collect();
    // (by the earlier of the sum and the carry: both outputs come after
    // the cell's leaves, so a cell producing a signal is committed before
    // every cell reading it; the sum alone can come after a reader of the
    // carry, and the reader would then set the carry's weight)
    let out_pos = |ci: usize| -> u32 {
        let c = &cells[ci];
        let p = |v: i32| -> u32 { if (v as usize) < nv { pos[net.rep[v as usize].0 as usize] } else { u32::MAX } };
        p(c.sum).min(p(c.carry_var()))
    };
    cell_order.sort_by_key(|&ci| (out_pos(ci), !cells[ci].fa, ci));
    let mut rejected: Vec<usize> = Vec::new();
    let produced_any: HashSet<i32> = cells.iter().flat_map(|c| [c.sum, c.carry_var()]).collect();
    let mut by_leaves: [HashMap<Vec<i32>, (usize, u8)>; 2] = [HashMap::new(), HashMap::new()];
    // a produced signal read by two cells, plain by one a column above it
    // and complemented by the other at its own column (2x - x = x: a Booth
    // Wallace tree moves a carry up this way, bead skz.4): the cell above
    // reads it doubled. Keyed by the signal, the cell reading it doubled
    let mut doubled: HashMap<i32, usize> = HashMap::new();
    let cell_reads: HashMap<i32, usize> = {
        let mut m: HashMap<i32, usize> = HashMap::new();
        for c in &cells {
            for &l in &c.leaves {
                *m.entry(l).or_default() += 1;
            }
        }
        m
    };
    *work += cells.len() as u64;
    for ci in cell_order {
        let c = &cells[ci];
        let (sv, cv) = (rep_of(c.sum), rep_of(c.carry_var()));
        let in_a = in_side[0][ci];
        let in_b = in_side[1][ci];
        if !in_a && !in_b {
            continue;
        }
        // a cell both circuits share (hashing merged two identical ones) is
        // part of both sums and cancels in their difference
        // check the cell's equations together, then commit
        let base = sv;
        let mut trial: Vec<(i32, i32, i64)> = c.leaves.iter().filter(|&&x| produced_any.contains(&x)).map(|&x| (x, base, 0)).collect();
        trial.push((cv, base, 1));
        let eqs: Vec<(i32, i32, i64)> = trial
            .iter()
            .map(|&(a, b, d)| {
                let (ra, oa) = uf.find(a);
                let (rb, ob) = uf.find(b);
                (ra, rb, d - oa + ob)
            })
            .collect();
        let mut ok = consistent(&eqs);
        let mut doubled_here: Option<i32> = None;
        if !ok {
            // one produced leaf, read by another cell too, one column below
            // the cell: read doubled
            for (ti, &(x, _, _)) in trial.iter().enumerate() {
                if x == cv || doubled.contains_key(&x) || cell_reads.get(&x).copied().unwrap_or(0) < 2 {
                    continue;
                }
                let mut t2 = trial.clone();
                t2[ti].2 = -1;
                let eqs2: Vec<(i32, i32, i64)> = t2
                    .iter()
                    .map(|&(a, b, d)| {
                        let (ra, oa) = uf.find(a);
                        let (rb, ob) = uf.find(b);
                        (ra, rb, d - oa + ob)
                    })
                    .collect();
                *work += t2.len() as u64;
                if consistent(&eqs2) {
                    trial = t2;
                    ok = true;
                    doubled_here = Some(x);
                    break;
                }
            }
        }
        if !ok {
            if dbg {
                eprintln!("structarith: cell {} leaves {:?} sum {} carry {} fa {} rejected: its weights contradict the cells before it (sides {}{})", ci, c.leaves, c.sum, c.carry, c.fa, if in_a { "A" } else { "" }, if in_b { "B" } else { "" });
                for &x in c.leaves.iter().chain([sv, cv].iter()) {
                    let (r, o) = uf.find(x);
                    eprintln!("    {} at root {} exponent {}{}", x, r, o, if r == x { " (its own)" } else { "" });
                }
            }
            rejected.push(ci);
            continue; // an xor and an and over the same two control signals look like a half adder
        }
        for &(a, b, d) in &trial {
            uf.union(a, b, d);
        }
        if let Some(x) = doubled_here {
            doubled.insert(x, ci);
            if dbg {
                eprintln!("structarith: cell {} reads {} doubled, a column above where it is made", ci, x);
            }
        }
        // one cell per leaf set on a side: two circuits with a merged sum
        // gate each pair it with their own carry, and the cell with the
        // other circuit's carry would read the leaves a second time; the
        // cell with more outputs in the side's cone is the side's
        let mut key = c.leaves.clone();
        key.sort_unstable();
        for (s, here, cone_s) in [(0usize, in_a, &cone_a), (1usize, in_b, &cone_b)] {
            if !here {
                continue;
            }
            let score = cone_s.contains(&sv) as u8 + cone_s.contains(&cv) as u8;
            match by_leaves[s].get(&key) {
                Some(&(_, other_score)) if other_score >= score => continue,
                Some(&(other, _)) => side_cells[s].retain(|&x| x != other),
                None => {}
            }
            by_leaves[s].insert(key.clone(), (ci, score));
            side_cells[s].push(ci);
        }
        *work += 8;
    }
    if dbg {
        eprintln!("structarith: side cells {} and {} (weights consistent), work {}K", side_cells[0].len(), side_cells[1].len(), *work / 1000);
        if let Ok(list) = std::env::var("SAT_STRUCT_DEBUG_CELLS") {
            let vars: Vec<i32> = list.split(',').filter_map(|t| t.parse().ok()).collect();
            let all = list == "all";
            for s in 0..2 {
                for &ci in &side_cells[s] {
                    let c = &cells[ci];
                    if all || vars.iter().any(|v| c.leaves.contains(v) || c.sum == *v || c.carry_var() == *v) {
                        eprintln!("    side {} cell {}: leaves {:?} negmask {} sum {} sum_neg {} carry {} fa {} one {}", s, ci, c.leaves, c.negmask, c.sum, c.sum_neg, c.carry, c.fa, c.one);
                    }
                }
            }
        }
    }
    if side_cells[0].is_empty() || side_cells[1].is_empty() {
        return;
    }
    // the real cells' readings first: the virtual cells below read every
    // signal as its producer finally writes it
    {
        let real: Vec<usize> = {
            let mut v: Vec<usize> = side_cells[0].iter().chain(side_cells[1].iter()).copied().collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        if !resolve_polarities(&mut cells, &real, &doubled) {
            if dbg {
                eprintln!("structarith: no consistent reading of the cells' polarities");
            }
            return;
        }
        // (each side's reading is settled up to a flip of the whole side,
        // both readings of every cell flipping together; which one is
        // decided once the compared signals have their cells, below)
    }
    // a leaf no cell produces takes the weight of the first cell reading
    // it (the grid and the lone-gate rules below want products weighted);
    // it never ties two networks together or moves a weight: a leaf may
    // sit at two weights (a Booth digit's sign bit at the row's lowest
    // column and above its top; the product a_0 b_15 at column 15 of an
    // AND multiplier and at 16 in a Booth one's correction row), and the
    // sums read each cell's own weight, so it is counted right either way.
    // The two sides come to one scale through the compared signals below
    // Within a side, a leaf read once ties the cells of that side (an AND
    // multiplier's lowest columns reach the rest only through products)
    let mut leaf_weighted: HashSet<i32> = HashSet::new(); // (only produced signals have weights so far)
    for s in 0..2 {
        let mut reads: HashMap<i32, u32> = HashMap::new();
        for &ci in &side_cells[s] {
            for &l in &cells[ci].leaves {
                *reads.entry(l).or_default() += 1;
            }
        }
        let mut side_roots: HashSet<i32> = side_cells[s].iter().map(|&ci| uf.find(cells[ci].sum).0).collect();
        // (a leaf is unweighted until a union here or in the grid touched
        // it; being its own root is not the test, a product the grid made
        // a root of its component is one too, and a union with it would
        // tie that component to this cell's)
        for &ci in &side_cells[s] {
            let c = &cells[ci];
            for &l in &c.leaves {
                if produced_any.contains(&l) || bare_input(l) {
                    continue;
                }
                let (rl, ol) = uf.find(l);
                let (rb, ob) = uf.find(c.sum);
                let unweighted = rl == l && !leaf_weighted.contains(&l);
                if unweighted || (rl == rb && ol == ob) {
                    uf.union(l, c.sum, 0);
                    leaf_weighted.insert(l);
                } else if rl != rb && reads[&l] == 1 && side_roots.contains(&rl) {
                    // two of this side's networks, tied at the leaf's weight
                    uf.union(l, c.sum, 0);
                    side_roots = side_cells[s].iter().map(|&ci| uf.find(cells[ci].sum).0).collect();
                    *work += side_cells[s].len() as u64;
                }
            }
        }
        *work += side_cells[s].len() as u64 * 3;
    }
    if trace_var != 0 {
        let (r, e) = uf.find(trace_var);
        eprintln!("structarith: trace {}: after the sides and weights at root {} exponent {}", trace_var, r, e);
    }
    phase("sides and weights", *work);
    // --- the final adder (bead SAT-playground-skz.12): what the cones hold
    // beyond adder cells, made linear ---
    // every variable a committed cell touches has a weight
    let mut weighted: HashSet<i32> = HashSet::new();
    for &ci in side_cells[0].iter().chain(side_cells[1].iter()) {
        let c = &cells[ci];
        weighted.extend(c.leaves.iter().copied());
        weighted.insert(c.sum);
        weighted.insert(c.carry_var());
    }
    let is_input = |v: i32| (v as usize) < nv && net.gates[v as usize].is_none() && !net.units[v as usize];
    // 1. a partial product no cell reads (a column whose sum bit is dead
    //    logic, the top column with only an xor tree) gets its weight from
    //    the operand grid: pp(a, b) sits at exp(a, b') + exp(a', b) -
    //    exp(a', b') for three weighted products over the same inputs
    let mut pps: HashMap<(i32, i32), i32> = HashMap::new(); // (input, input) -> partial product node
    let grid_data: Option<(Vec<((i32, i32), i32)>, HashMap<i32, Vec<i32>>)>;
    {
        let mut by_var: HashMap<i32, Vec<i32>> = HashMap::new();
        for &v in &net.order {
            if net.rep[v as usize].0 != v || !(cone_a.contains(&v) || cone_b.contains(&v)) {
                continue;
            }
            let g = net.gates[v as usize].as_ref().unwrap();
            if g.tt == 0b1000 && g.sup.len() == 2 && is_input(g.sup[0]) && is_input(g.sup[1]) {
                pps.insert((g.sup[0], g.sup[1]), v);
                by_var.entry(g.sup[0]).or_default().push(v);
                by_var.entry(g.sup[1]).or_default().push(v);
            }
        }
        *work += net.order.len() as u64;
        let grid: Vec<((i32, i32), i32)> = {
            let mut g: Vec<((i32, i32), i32)> = pps.iter().map(|(&k, &v)| (k, v)).collect();
            g.sort_unstable(); // (a fixed order: the unions and the work charged must not vary)
            g
        };
        let roots_of = |s: usize, uf: &mut WeightedUf| -> HashSet<i32> { side_cells[s].iter().map(|&ci| uf.find(cells[ci].sum).0).collect() };
        loop {
            let (ra, rb) = (roots_of(0, &mut uf), roots_of(1, &mut uf));
            let side_pos: Vec<(Vec<(i32, i32, i64)>, Option<i32>)> = (0..2).map(|s| side_product_pairs(&cells, &side_cells[s], &pps, &mut uf, work)).collect();
            if !place_products(&grid, &by_var, &pps, &net, &mut uf, &mut weighted, work, &stopped, trace_var, Some((&ra, &rb)), &side_pos) {
                break;
            }
            if stopped(*work) {
                return;
            }
        }
        grid_data = Some((grid, by_var));
    }
    let grid_data = grid_data;
    // 2a. the gates in the cones of input leaves (a Booth digit's encoder
    //     and row muxes) are no adders: the lone-gate and product rules
    //     below leave them alone
    let (input_leaves0, input_logic) = classify_input_leaves(&cells, &side_cells, &net, work, &stopped, trace_var, &transparent);
    if stopped(*work) {
        return;
    }
    // the reading of a side is settled up to a flip of the whole side; how
    // it reads its products decides it: a side reading them mostly negated
    // (the complement reading) is flipped whole before the virtual cells
    // are made, which then read what the flipped cells write. Only product
    // leaves vote: a Booth row bit is an xnor read negated whose sum is
    // the plain product sum, an xor leaf has a negative product term in
    // any reading (bead SAT-playground-skz.4)
    {
        let shared: HashSet<usize> = side_cells[0].iter().copied().filter(|ci| side_cells[1].contains(ci)).collect();
        // each side's leaves as a signed, weighted form over variables
        // (the sign of a variable's net coefficient is its reading, a
        // product read negated at one weight and plain at a larger one
        // reads plain)
        let mut forms: [HashMap<i32, i128>; 2] = [HashMap::new(), HashMap::new()];
        let min_e: i64 = side_cells.iter().flatten().map(|&ci| uf.find(cells[ci].sum).1).min().unwrap_or(0);
        for s in 0..2 {
            let produced: HashSet<i32> = side_cells[s].iter().flat_map(|&ci| [cells[ci].sum, cells[ci].carry_var()]).collect();
            for &ci in &side_cells[s] {
                let c = &cells[ci];
                let e = uf.find(c.sum).1 - min_e;
                if !(0..MAX_BITS as i64).contains(&e) {
                    continue;
                }
                let w = 1i128 << e;
                for i in 0..c.leaves.len() {
                    if !produced.contains(&c.leaves[i]) {
                        let l = c.leaf_lit(i);
                        let e = forms[s].entry(l.abs()).or_default();
                        // (checked: many reads at the top weight could pass
                        // the i128; such a form is of no use for the vote)
                        *e = e.saturating_add(if l < 0 { -w } else { w });
                    }
                }
            }
            *work += side_cells[s].len() as u64 * 2;
        }
        // the leaves both sides read decide between the four readings: the
        // one under which most of them agree in sign (no flip first, one
        // flip before two); a Booth side shares no leaf with an AND side,
        // and then each side's own products decide (an xnor row bit's
        // polynomial is the plain product sum; an xor leaf has a negative
        // product term in any reading, so only product gates vote, and
        // with none the literals do)
        // per side: how many products it reads plain and negated (a
        // product in any polarity: an and of two input literals, or the
        // or of two, which is the and of their complements negated); with
        // no product, every leaf by its literal
        let mut votes: [(usize, usize); 2] = [(0, 0); 2];
        for s in 0..2 {
            let (mut positive, mut negative) = (0usize, 0usize);
            for (&v, &coef) in &forms[s] {
                let Some(il) = input_leaves0.get(&v) else { continue };
                if il.sup.len() != 2 {
                    continue;
                }
                // (an and of two input literals; a nand, the product
                // negated, read negated is the product read plain; an or
                // of two inputs is an addend of its own, no vote)
                let flipped = if and_family(il.tt).is_some() {
                    false
                } else if il.tt == 0b0111 {
                    true
                } else {
                    continue;
                };
                if (coef > 0) != flipped {
                    positive += 1;
                } else {
                    negative += 1;
                }
            }
            if positive + negative == 0 {
                for &coef in forms[s].values() {
                    if coef < 0 {
                        negative += 1;
                    } else {
                        positive += 1;
                    }
                }
            }
            votes[s] = (positive, negative);
            *work += forms[s].len() as u64;
        }
        let common: Vec<i32> = forms[0].keys().filter(|v| forms[1].contains_key(*v)).copied().collect();
        // the reading: of the four, the one under which most leaves both
        // sides read agree in sign; then the fewest flips (a circuit
        // summing or gates reads them as they are); then the most products
        // read plain (one side reading its products negated is the
        // complement reading of that side)
        // the reading, in two steps. Side A is the reference: side B is
        // flipped when the leaves both sides read then agree in sign on
        // more of them (a side reading the shared products the other way
        // round is in the complement reading of the pair; a product
        // negated on both sides is an addend of its own, as a mixed-sign
        // sum has). Then both are flipped together when they read nearly
        // every product negated (the complement reading of the whole,
        // which the lone-gate rules do not expect; a sum genuinely mixed
        // in sign is left alone)
        let agree_b = |fb: bool| -> usize {
            common
                .iter()
                .filter(|&&v| {
                    let a = forms[0][&v].signum();
                    let b = forms[1][&v].signum() * if fb { -1 } else { 1 };
                    a != 0 && a == b
                })
                .count()
        };
        let (keep, turned) = (agree_b(false), agree_b(true));
        let mut flip = [false, turned > keep];
        let (mut plain, mut negated) = (votes[0].0 + if flip[1] { votes[1].1 } else { votes[1].0 }, votes[0].1 + if flip[1] { votes[1].0 } else { votes[1].1 });
        if plain + negated == 0 {
            // no product to vote: every leaf by its literal
            for s in 0..2 {
                for &coef in forms[s].values() {
                    if (coef < 0) != flip[s] {
                        negated += 1;
                    } else {
                        plain += 1;
                    }
                }
            }
        }
        // (every product negated: the complement reading of the whole;
        // most of them negated is a sum with genuinely negated addends,
        // left as it is. A retry of the other reading when the pass later
        // fails would be the robust rule: not built yet. A side found by
        // function, a mapped Booth tree, reads its rows in whichever of its
        // readings the cells' carries chose: there all but a few negated is
        // the complement reading too, bead skz.13)
        let by_function = !booth_cols.is_empty();
        if negated > 0 && (plain == 0 || (by_function && negated > 4 * plain)) {
            flip = [!flip[0], !flip[1]];
            std::mem::swap(&mut plain, &mut negated);
        }
        if dbg {
            eprintln!("structarith: {} leaves read by both sides ({} agree as read, {} with side B turned), products plain/negated {:?}: reading {:?} ({} plain, {} negated under it)", common.len(), keep, turned, votes, flip, plain, negated);
        }
        *work += common.len() as u64 * 4;
        for s in 0..2 {
            if !flip[s] {
                continue;
            }
            // (a cell both sides hold keeps its reading: it is part of
            // both sums and cancels in their difference)
            for &ci in &side_cells[s] {
                if shared.contains(&ci) {
                    continue;
                }
                let c = &mut cells[ci];
                let all = (1u8 << c.leaves.len()) - 1;
                c.negmask ^= all;
                c.sum_neg = !c.sum_neg;
                c.carry = -c.carry;
                if !c.fa {
                    c.one = !c.one;
                }
            }
        }
    }
    // 2. a lone sum (an xor with no carry beside it: a top-column adder
    //    whose carry was dead logic) or a lone carry (a majority or an and
    //    with no sum: the carry chain of a ripple final adder whose sum bits
    //    are dead) becomes a cell with a fresh partner, defined in the
    //    proof alone. A lone sum may read a signal with no weight (the
    //    carry a prefix network computes), which then takes the sum's
    //    weight; a lone carry needs every leaf weighted (the network's own
    //    and gates must not be taken for cells)
    let mut covered: Vec<bool> = vec![false; nv];
    {
        let mark = |c: &Cell, covered: &mut Vec<bool>, work: &mut u64| {
            let stop_at: HashSet<i32> = c.leaves.iter().copied().collect();
            let mut stack = vec![c.sum, c.carry_var()];
            while let Some(u) = stack.pop() {
                *work += 1;
                if u as usize >= net.rep.len() {
                    continue; // a fresh variable
                }
                let r = net.rep[u as usize].0;
                if stop_at.contains(&r) || covered[r as usize] {
                    continue;
                }
                let Some(g) = net.gates[r as usize].as_ref() else { continue };
                covered[r as usize] = true;
                stack.extend(g.sup.iter().copied());
            }
        };
        for &ci in side_cells[0].iter().chain(side_cells[1].iter()) {
            mark(&cells[ci], &mut covered, work);
        }
        let in_cones: Vec<i32> = net.order.iter().copied().filter(|&v| net.rep[v as usize].0 == v && (cone_a.contains(&v) || cone_b.contains(&v))).collect();
        let mut memo: HashMap<i32, u64> = HashMap::new();
        // a gate strictly inside another gate's possible carry cell
        // (between it and a cut with a carry function, read by that path
        // alone) is no cell of its own: a mapped majority holds an and and
        // a nor over the same two inputs, and only the majority adds up.
        // (Sums are not treated so: an xor inside an xor3 is a half adder
        // of its own, and either cut of an xor tree adds up.)
        let mut internal: Vec<bool> = vec![false; nv];
        for &v in &in_cones {
            if covered[v as usize] {
                continue;
            }
            if stopped(*work) {
                return;
            }
            for cut in &cuts[v as usize] {
                if cut.len() < 2 || cut.len() > 3 || cut.contains(&v) {
                    continue;
                }
                let Some(tt) = cut_truth_table(v, cut, &net, &mut memo, work) else { continue };
                let carry = match cut.len() {
                    2 => and_family(tt).is_some() || or_family(tt).is_some(),
                    _ => maj_family(tt).is_some(),
                };
                if !carry {
                    continue;
                }
                let mut inside: Vec<i32> = Vec::new();
                let mut fine = true;
                let mut stack: Vec<i32> = net.gates[v as usize].as_ref().unwrap().sup.iter().map(|&u| net.rep[u as usize].0).collect();
                while let Some(u) = stack.pop() {
                    *work += 1;
                    if cut.contains(&u) || inside.contains(&u) {
                        continue;
                    }
                    let Some(g) = net.gates[u as usize].as_ref() else { fine = false; break };
                    if covered[u as usize] || readers[u as usize] != 1 {
                        fine = false;
                        break;
                    }
                    inside.push(u);
                    stack.extend(g.sup.iter().map(|&x| net.rep[x as usize].0));
                }
                if fine {
                    for u in inside {
                        internal[u as usize] = true;
                    }
                }
            }
        }
        // the literal a cell writes a signal as (a lone sum reads it so,
        // and keeps that under any later flip of both)
        let mut written: HashMap<i32, i32> = HashMap::new();
        for &ci in side_cells[0].iter().chain(side_cells[1].iter()) {
            let c = &cells[ci];
            written.insert(c.sum, c.sum_lit());
            written.insert(c.carry_var(), c.carry_lit());
        }
        // every signal some cell produces, kept up to date as the rounds
        // make cells (produced_any is the snapshot from before them; a leaf
        // that gets a producer in a round has a weight of its own, no
        // input leaf's weak one: review, 2026-10-23)
        let mut produced_live: HashSet<i32> = cells.iter().flat_map(|c| [c.sum, c.carry_var()]).collect();
        // product cells: a mapper may write a column's half-adder carry
        // over two partial products as the and of two other products of
        // the same four inputs (a_i b_j . a_k b_l = a_i b_l . a_k b_j), and
        // the sum as nor(carry, nor(pp, pp)); neither gate then has the two
        // products as a cut. Over the four primary inputs both are
        // functions of the two products the column holds: a sum and a
        // carry over one pair make a cell, a lone one gets a fresh partner
        // below. The proof of such a cell splits over the four inputs.
        let mut func_cuts: HashMap<i32, (Vec<i32>, u64, Vec<i32>)> = HashMap::new(); // gate -> (the pair, tt over it, the inputs)
        // the product pairings (what the product cells below are made
        // of): known before the rounds, so a lone-gate round before the
        // sides are at one scale leaves those gates alone
            let mut by_pair: HashMap<Vec<i32>, Vec<(i32, u64)>> = HashMap::new();
            let mut pairings: HashMap<i32, Vec<(Vec<i32>, u64, Vec<i32>)>> = HashMap::new(); // every pairing a gate reads through
            let trace: i32 = std::env::var("SAT_STRUCT_DEBUG_VIRT").ok().and_then(|t| t.parse().ok()).unwrap_or(0);
            for &v in &in_cones {
                if v == trace {
                    eprintln!("structarith: product trace {}: covered {} internal {}", v, covered[v as usize], internal[v as usize]);
                }
                if covered[v as usize] || internal[v as usize] {
                    continue; // (a regrouped carry is an input leaf of the tree: fine, it is what a product cell is made of)
                }
                if stopped(*work) {
                    return;
                }
                // the primary inputs under v: four of them, through few gates
                let mut sup: Vec<i32> = Vec::new();
                let mut seen: HashSet<i32> = HashSet::new();
                let mut stack: Vec<i32> = vec![v];
                let mut fine = true;
                let mut depth = 0;
                while let Some(u) = stack.pop() {
                    if !seen.insert(u) {
                        continue;
                    }
                    *work += 1;
                    match net.gates[u as usize].as_ref() {
                        Some(g) => {
                            depth += 1;
                            if depth > 24 {
                                fine = false;
                                break;
                            }
                            stack.extend(g.sup.iter().map(|&x| net.rep[x as usize].0));
                        }
                        None => {
                            if net.units[u as usize] {
                                fine = false;
                                break;
                            }
                            sup.push(u);
                            if sup.len() > 4 {
                                fine = false;
                                break;
                            }
                        }
                    }
                }
                if v == trace {
                    eprintln!("    support {:?} fine {}", sup, fine);
                }
                if !fine || sup.len() != 4 {
                    continue;
                }
                sup.sort_unstable();
                let Some(tt) = cut_truth_table(v, &sup, &net, &mut memo, work) else { continue };
                if v == trace {
                    eprintln!("    tt {:#x}", tt);
                }
                let key = |a: i32, b: i32| (a.min(b), a.max(b));
                for (i, j, k, l) in [(0usize, 1usize, 2usize, 3usize), (0, 2, 1, 3), (0, 3, 1, 2)] {
                    let (Some(&p1), Some(&p2)) = (pps.get(&key(sup[i], sup[j])), pps.get(&key(sup[k], sup[l]))) else { continue };
                    if p1 == p2 {
                        continue;
                    }
                    // the function through the two products, if it is one
                    let mut tt2: [Option<bool>; 4] = [None; 4];
                    let mut through = true;
                    for m in 0..16u32 {
                        let bit = |x: usize| (m >> x) & 1 == 1;
                        let idx = ((bit(i) && bit(j)) as usize) | (((bit(k) && bit(l)) as usize) << 1);
                        let val = (tt >> m) & 1 == 1;
                        match tt2[idx] {
                            None => tt2[idx] = Some(val),
                            Some(x) => through &= x == val,
                        }
                    }
                    if !through {
                        continue;
                    }
                    let t2: u64 = (0..4).map(|idx| (tt2[idx].unwrap_or(false) as u64) << idx).sum();
                    // the pair sorted; the table's input order follows
                    let (lo, hi, t2) = if p1 < p2 { (p1, p2, t2) } else { (p2, p1, (t2 & 0b1001) | ((t2 >> 1) & 1) << 2 | ((t2 >> 2) & 1) << 1) };
                    by_pair.entry(vec![lo, hi]).or_default().push((v, t2));
                    pairings.entry(v).or_default().push((vec![lo, hi], t2, sup.clone()));
                }
            }
        let mut rounds = 0;
        let mut changed = true;
        let mut tied = false;
        let mut products_done = false;
        let mut relaxed = false;
        let mut weak_ok = false;
        let pps_set: HashSet<i32> = pps.values().copied().collect();
        while (changed || !relaxed || !weak_ok) && rounds < 60 {
            // a round that made nothing allows the leaves waiting for a
            // producer once; a relaxed round that made nothing allows an
            // input leaf's weight as the only evidence (sign extension
            // reads a bit at two columns; the weights from the top come
            // first); a round that made something goes strict again
            weak_ok = relaxed && !changed;
            relaxed = !changed;
            changed = false;
            rounds += 1;
            let mut deferred = false;
            let side_roots_now: [HashSet<i32>; 2] = [
                side_cells[0].iter().map(|&ci| uf.find(cells[ci].sum).0).collect(),
                side_cells[1].iter().map(|&ci| uf.find(cells[ci].sum).0).collect(),
            ];
            *work += (side_cells[0].len() + side_cells[1].len()) as u64;
            // the half adders by carry, in cell order (the or of two
            // exclusive carries below looks them up; a cell made in this
            // round joins as it is made)
            let mut half_of: HashMap<i32, Vec<usize>> = HashMap::new();
            for (ci, c) in cells.iter().enumerate() {
                if !c.fa && !c.one && c.leaves.len() == 2 {
                    half_of.entry(c.carry_var()).or_default().push(ci);
                }
            }
            *work += cells.len() as u64;
            // the two sides come to one scale as soon as both compared
            // signals have a weight (the top cell of a side may be a lone
            // sum made in a round): the compared signals are one bit. Then
            // the operand grid places more products (a product the one
            // side's tree reads and the other's final adder reads at the
            // same weight was in two networks until now)
            if !tied && weighted.contains(&side_a) && weighted.contains(&side_b) {
                let (ra, ea) = uf.find(side_a);
                let (rb, eb) = uf.find(side_b);
                if ra != rb {
                    uf.union(side_a, side_b, 0);
                    changed = true;
                } else if ea != eb {
                    if dbg {
                        eprintln!("structarith: the compared signals sit at exponents {} and {} of one network", ea, eb);
                        // the signals of both cones with a weight, and the cells over them
                        let mut both: Vec<i32> = weighted.iter().copied().filter(|&v| cone_a.contains(&v) && cone_b.contains(&v)).collect();
                        both.sort_unstable();
                        for &v in both.iter().take(12) {
                            let (r, e) = uf.find(v);
                            let in_cells: Vec<String> = cells.iter().enumerate().filter(|(_, c)| c.sum == v || c.carry_var() == v || c.leaves.contains(&v)).map(|(ci, c)| format!("cell {} {:?}={}+2*{}", ci, c.leaves, c.sum_lit(), c.carry_lit())).collect();
                            eprintln!("    shared {}: root {} exponent {}; {}", v, r, e, in_cells.join("; "));
                        }
                        let both_cells: Vec<usize> = (0..cells.len()).filter(|&ci| side_cells[0].contains(&ci) && side_cells[1].contains(&ci)).collect();
                        eprintln!("    cells on both sides: {:?}; roots of side A's cells {:?}, side B's {:?}", both_cells, side_cells[0].iter().map(|&ci| uf.find(cells[ci].sum).0).collect::<HashSet<_>>(), side_cells[1].iter().map(|&ci| uf.find(cells[ci].sum).0).collect::<HashSet<_>>());
                    }
                    return;
                }
                tied = true;
            }
            if let Some((grid, by_var)) = grid_data.as_ref() {
                // (a product weighted in one side's component is never tied
                // into the other side's: before the tie the sides are
                // separate networks, after it one, and a product may sit at
                // a column of its own on each side)
                while {
                    let side_pos: Vec<(Vec<(i32, i32, i64)>, Option<i32>)> = (0..2).map(|s| side_product_pairs(&cells, &side_cells[s], &pps, &mut uf, work)).collect();
                    let ra: HashSet<i32> = side_cells[0].iter().map(|&ci| uf.find(cells[ci].sum).0).collect();
                    let rb: HashSet<i32> = side_cells[1].iter().map(|&ci| uf.find(cells[ci].sum).0).collect();
                    *work += (side_cells[0].len() + side_cells[1].len()) as u64;
                    place_products(grid, by_var, &pps, &net, &mut uf, &mut weighted, work, &stopped, trace_var, Some((&ra, &rb)), &side_pos)
                } {
                    changed = true;
                    if stopped(*work) {
                        return;
                    }
                }
            }
            // the product cells, once the sides are at one scale (their
            // pairing goes by the products' weights); the first round makes
            // the compared signals' own cells alone, so nothing else is
            // taken for a lone gate before that
            if !products_done && (tied || rounds >= 2) {
                products_done = true;
                changed = true;
                {
                    // a lone gate reads through one pairing: the one whose two
                    // products share a weight (a column's), else the first (a
                    // product of four inputs reads through every pairing)
                    for (&v, list) in &pairings {
                        let same = list.iter().find(|(pair, _, _)| {
                            weighted.contains(&pair[0]) && weighted.contains(&pair[1]) && uf.find(pair[0]) == uf.find(pair[1])
                        });
                        func_cuts.insert(v, same.unwrap_or(&list[0]).clone());
                    }
                    let mut pairs: Vec<Vec<i32>> = by_pair.keys().cloned().collect();
                    pairs.sort_unstable();
                    for pair in pairs {
                        let members = &by_pair[&pair];
                        if dbg && std::env::var_os("SAT_STRUCT_DEBUG_LEMMA").is_some() {
                            eprintln!("    product pair {:?}: {:?}", pair, members.iter().map(|&(v, t)| (v, format!("{:#x}", t), covered[v as usize])).collect::<Vec<_>>());
                        }
                        let sums: Vec<(i32, u64)> = members.iter().copied().filter(|&(_, t)| t == XOR2 || t == XNOR2).collect();
                        let mut carries: Vec<(i32, u8, bool)> = members.iter().filter_map(|&(v, t)| and_family(t).map(|m| (v, m, false)).or_else(|| or_family(t).map(|m| (v, m, true)))).collect();
                        for &(sv, stt) in &sums {
                            // a carry some other gate reads too (the sum is built
                            // from the carry and a nor over the pair, which only
                            // the sum reads: that nor is no carry of the column),
                            // the nearest by variable number among those (the
                            // other circuit may hold one over the same products)
                            let Some(k) = carries
                                .iter()
                                .enumerate()
                                .filter(|(_, &(cv, _, _))| !covered[cv as usize])
                                .min_by_key(|(_, &(cv, _, _))| (readers[cv as usize] <= 1, (cv - sv).abs()))
                                .map(|(i, _)| i)
                            else {
                                break;
                            };
                            let (cv, m, one) = carries.remove(k);
                            if dbg && std::env::var_os("SAT_STRUCT_DEBUG_LEMMA").is_some() {
                                eprintln!("    product cell try: sum {} carry {} (negmask {}, one {}, readers {}) covered {} {}", sv, cv, m, one, readers[cv as usize], covered[sv as usize], covered[cv as usize]);
                            }
                            if covered[sv as usize] || covered[cv as usize] {
                                continue;
                            }
                            let flips = (m.count_ones() & 1) == 1;
                            let k = (stt == XNOR2) != flips;
                            let split = pairings[&sv].iter().find(|(p, _, _)| *p == pair).unwrap().2.clone();
                            let c = Cell { fa: false, one, leaves: pair.clone(), negmask: m, sum: sv, sum_neg: if one { !k } else { k }, carry: cv, split: Some(split) };
                            // the weights, as for any cell: leaves and sum alike, the carry one up
                            let base = pair[0];
                            let trial: Vec<(i32, i32, i64)> = vec![(pair[1], base, 0), (sv, base, 0), (cv, base, 1)];
                            let eqs: Vec<(i32, i32, i64)> = trial
                                .iter()
                                .map(|&(a, b, d)| {
                                    let (ra, oa) = uf.find(a);
                                    let (rb, ob) = uf.find(b);
                                    (ra, rb, d - oa + ob)
                                })
                                .collect();
                            if !consistent(&eqs) {
                                if dbg {
                                    eprintln!("structarith: product cell {:?} = {} + 2*{} contradicts the weights", pair, sv, cv);
                                }
                                continue;
                            }
                            for &(a, b, d) in &trial {
                                uf.union(a, b, d);
                            }
                            weighted.extend([pair[0], pair[1], sv, cv]);
                            mark(&c, &mut covered, work); // the outputs and the gates between them and the pair
                            let ci = cells.len();
                            if cone_a.contains(&sv) || cone_a.contains(&cv) {
                                side_cells[0].push(ci);
                            }
                            if cone_b.contains(&sv) || cone_b.contains(&cv) {
                                side_cells[1].push(ci);
                            }
                            produced_live.insert(c.sum);
                            produced_live.insert(c.carry_var());
                            cells.push(c);
                            func_cuts.remove(&sv);
                            func_cuts.remove(&cv);
                            *work += 8;
                        }
                    }
                    if dbg {
                        eprintln!("structarith: {} product cells, {} lone product gates", pairings.len() - func_cuts.len(), func_cuts.len());
                        if std::env::var_os("SAT_STRUCT_DEBUG_LEMMA").is_some() {
                            let mut lone: Vec<(&i32, &(Vec<i32>, u64, Vec<i32>))> = func_cuts.iter().collect();
                            lone.sort();
                            for (v, (pair, t2, sup)) in lone {
                                let all: Vec<(Vec<i32>, u64)> = pairings[v].iter().map(|(p, t, _)| (p.clone(), *t)).collect();
                                eprintln!("    lone product gate {} over {:?} tt {:#x} inputs {:?}; every pairing {:?}; covered {}", v, pair, t2, sup, all, covered[*v as usize]);
                            }
                        }
                    }
                }
            }
            let targets: Vec<i32> = in_cones.clone();
            let trace: i32 = std::env::var("SAT_STRUCT_DEBUG_VIRT").ok().and_then(|t| t.parse().ok()).unwrap_or(0);
            for &v in &targets {
                if v == trace {
                    eprintln!("structarith: trace {}: covered {} internal {} input logic {} weighted {} cuts {:?}", v, covered[v as usize], internal[v as usize], input_logic[v as usize], weighted.contains(&v), cuts[v as usize]);
                }
                if covered[v as usize] || internal[v as usize] || input_logic[v as usize] {
                    continue;
                }
                if !products_done && pairings.contains_key(&v) {
                    continue; // a product cell's gate, maybe: those are made once the sides are at one scale
                }
                if stopped(*work) {
                    return;
                }
                let mut made: Option<(Cell, i32)> = None; // the cell and the weighted signal it hangs on
                // cuts whose leaves all have weights first, smallest first;
                // then the rest (a lone sum may hang one unweighted leaf
                // on the others' weight)
                // (a cut with a bare input is no adder's: a Booth row bit
                // is an xor of the digit's sign bit and a mux, and a lone
                // sum over it would hang the sign bit at the row's weight)
                let mut tries: Vec<Vec<i32>> = cuts[v as usize].iter().filter(|c| c.len() >= 2 && c.len() <= 3 && !c.contains(&v) && !c.iter().any(|&l| bare_input(l))).cloned().collect();
                tries.sort_by_key(|c| (c.iter().any(|l| !weighted.contains(l)), c.len()));
                if let Some((pair, _, _)) = func_cuts.get(&v) {
                    tries.insert(0, pair.clone());
                }
                for cut in &tries {
                    let func = func_cuts.get(&v).filter(|(pair, _, _)| pair == cut);
                    let tt = match func {
                        Some(&(_, t2, _)) => t2,
                        None => match cut_truth_table(v, cut, &net, &mut memo, work) {
                            Some(t) => t,
                            None => continue,
                        },
                    };
                    let split: Option<Vec<i32>> = func.map(|(_, _, sup)| sup.clone());
                    if v == trace {
                        let ws: Vec<String> = cut.iter().map(|&l| if weighted.contains(&l) { format!("{}@{:?}", l, uf.find(l)) } else { format!("{}@?", l) }).collect();
                        eprintln!("    cut {:?} tt {:#x} leaves {:?} self {:?}", cut, tt, ws, if weighted.contains(&v) { Some(uf.find(v)) } else { None });
                    }
                    let sum_tt = (cut.len() == 2 && (tt == XOR2 || tt == XNOR2)) || (cut.len() == 3 && (tt == XOR3 || tt == XNOR3));
                    // an or of two half adders' carries where the second
                    // half adder reads the first's sum: the two carries are
                    // never both one (the first's carry makes its sum zero,
                    // the second's needs it one), so the or is their sum at
                    // their own weight, with a fresh and of the two (zero
                    // in fact) as the carry above. That is the carry out of
                    // a full adder built as two half adders and an or, and
                    // the top column reads it at its weight (bead skz.4)
                    let or_sum: Option<bool> = if !sum_tt && cut.len() == 2 {
                        let lits: Vec<i32> = cut.iter().map(|&l| written.get(&l).copied().unwrap_or(l)).collect();
                        let mask: u8 = lits.iter().enumerate().map(|(i, &l)| ((l < 0) as u8) << i).sum();
                        let is_or = or_family(tt) == Some(mask);
                        let is_nor = and_family(tt) == Some(mask ^ 0b11);
                        if is_or || is_nor {
                            let half = |x: i32| -> Option<usize> { half_of.get(&x).and_then(|v| v.iter().copied().find(|&ci| cells[ci].carry_lit() == written.get(&x).copied().unwrap_or(x))) };
                            match (half(cut[0]), half(cut[1])) {
                                (Some(a), Some(b)) => {
                                    let reads = |b: usize, a: usize| cells[b].leaves.iter().position(|&l| l == cells[a].sum).map_or(false, |i| cells[b].leaf_lit(i) == cells[a].sum_lit());
                                    (reads(b, a) || reads(a, b)).then_some(is_nor)
                                }
                                _ => None,
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    // the weight the leaves agree on, from those that have
                    // one: two leaves of one component must sit at one
                    // exponent; leaves of different components are tied by
                    // the unions below
                    // (an input leaf's weight is weak evidence: a sign bit is
                    // read at its own column and again at the columns above
                    // (sign extension), so a lone sum over it takes its
                    // weight from a produced leaf or from its own reader
                    // when it can, and waits for one when it cannot)
                    let mut w: Option<(i32, i64, i32)> = None;
                    let mut weak: Option<(i32, i64, i32)> = None;
                    let mut agree = true;
                    let mut unweighted = 0;
                    let mut per_root: HashMap<i32, i64> = HashMap::new();
                    for &l in cut {
                        if weighted.contains(&l) {
                            let (r, e) = uf.find(l);
                            // (for an xor sum only: a carry's leaves must
                            // all sit at one weight, input leaves or not)
                            if sum_tt && input_leaves0.contains_key(&l) && !produced_any.contains(&l) && !produced_live.contains(&l) {
                                // (and only in a component of this gate's own
                                // side: a product the other side reads sits at
                                // that side's column, a Booth correction row's)
                                let own_side = (cone_a.contains(&v) && side_roots_now[0].contains(&r)) || (cone_b.contains(&v) && side_roots_now[1].contains(&r));
                                if weak.is_none() && own_side {
                                    weak = Some((r, e, l));
                                }
                                continue;
                            }
                            match per_root.get(&r) {
                                Some(&e0) => agree &= e0 == e,
                                None => {
                                    per_root.insert(r, e);
                                }
                            }
                            if w.is_none() {
                                w = Some((r, e, l));
                            }
                        } else {
                            unweighted += 1;
                        }
                    }
                    if !agree {
                        continue;
                    }
                    let weak_only = w.is_none() && weak.is_some();
                    if w.is_none() {
                        if weak.is_some() && !weighted.contains(&v) && !weak_ok {
                            deferred = true;
                            continue;
                        }
                        w = weak;
                    }
                    // a gate leaf no cell has written yet may get a producer
                    // in a later round, which would write it the other way
                    // round (a lone carry's chain below a lone sum): such a
                    // leaf waits for the rounds that make no other progress
                    // (then it is the network's carry, which no cell writes)
                    if !relaxed && cut.iter().any(|&l| !written.contains_key(&l) && !bare_input(l) && (l as usize) < nv && net.gates[l as usize].is_some() && !input_logic[l as usize] && !pps_set.contains(&l)) {
                        deferred = true;
                        continue;
                    }
                    if sum_tt || or_sum.is_some() {
                        // the output's weight, from the leaves or from a
                        // cell that reads the output
                        let w = match (w, weighted.contains(&v)) {
                            (Some((r, e, l)), true) => {
                                // (in another component the union below ties the two)
                                let (rv, ev) = uf.find(v);
                                if rv == r && ev != e {
                                    if weak_only {
                                        (rv, ev, v) // the reader's weight over an input leaf's
                                    } else {
                                        continue;
                                    }
                                } else {
                                    (r, e, l)
                                }
                            }
                            (Some(t), false) => t,
                            (None, true) => {
                                let (rv, ev) = uf.find(v);
                                (rv, ev, v)
                            }
                            (None, false) => continue,
                        };
                        let lits: Vec<i32> = cut.iter().map(|&l| written.get(&l).copied().unwrap_or(l)).collect();
                        let negmask: u8 = lits.iter().enumerate().map(|(i, &l)| ((l < 0) as u8) << i).sum();
                        let flips = (negmask.count_ones() & 1) == 1;
                        let carry = next_fresh;
                        next_fresh += 1;
                        fresh.push(Fresh { var: carry, def: if cut.len() == 2 { FreshDef::And(lits) } else { FreshDef::Maj(lits) } });
                        // v = xor of the plain leaves (+ 1 for an xnor) = xor of the literals + the negations' parity
                        let sum_neg = match or_sum {
                            Some(is_nor) => is_nor, // (the or of the literals is the sum; a nor its complement)
                            None => (tt == XNOR2 || tt == XNOR3) != flips,
                        };
                        if dbg && or_sum.is_some() {
                            eprintln!("structarith: the or {} of two exclusive half-adder carries {:?} is their sum", v, cut);
                        }
                        made = Some((Cell { fa: cut.len() == 3, one: false, leaves: cut.clone(), negmask, sum: v, sum_neg, carry, split }, w.2));
                        break;
                    }
                    // a carry needs its leaves weighted, except a lone
                    // majority (never a network's gate) whose output has a
                    // weight: its leaves then sit one below (a column of
                    // an array multiplier whose sums are dead logic)
                    let w = if unweighted > 0 {
                        if cut.len() == 3 && weighted.contains(&v) && maj_family(tt).is_some() {
                            let (rv, ev) = uf.find(v);
                            match w {
                                Some((r, e, _)) if (r, e) != (rv, ev - 1) => continue,
                                _ => {}
                            }
                            // hang the leaves on the output: the base is the output itself, one up
                            Some((rv, ev - 1, v))
                        } else {
                            continue;
                        }
                    } else {
                        w
                    };
                    let Some((r, e, l)) = w else { continue };
                    let (fam, one) = if cut.len() == 2 {
                        match (and_family(tt), or_family(tt)) {
                            (Some(m), _) => (Some(m), false),
                            (None, Some(m)) => (Some(m), true),
                            _ => (None, false),
                        }
                    } else {
                        (maj_family(tt), false)
                    };
                    let Some(m) = fam else { continue };
                    if weighted.contains(&v) {
                        let (rv, ev) = uf.find(v);
                        if rv == r && ev != e + 1 {
                            continue;
                        }
                    }
                    // the gate reads its leaves with the polarities m; a
                    // leaf another cell writes must be read as written, so
                    // either every written leaf matches (the plain reading)
                    // or every one is the complement (the flipped reading,
                    // with the carry and the sum complemented and a carry-in
                    // one toggled); a mix has no reading
                    // (a leaf no cell writes, a partial product, is read
                    // plain: the chains below sum the products as they are)
                    let mut matches = 0;
                    let mut mismatches = 0;
                    for (i, &l) in cut.iter().enumerate() {
                        let expected_neg = written.get(&l).map_or(false, |&wl| wl < 0);
                        if (expected_neg as u8) == ((m >> i) & 1) {
                            matches += 1;
                        } else {
                            mismatches += 1;
                        }
                    }
                    if v == trace {
                        let ws: Vec<String> = cut.iter().map(|&l| format!("{}:{:?}", l, written.get(&l))).collect();
                        eprintln!("    lone carry {} over {:?} mask {:#b}: written {:?}, {} match {} mismatch", v, cut, m, ws, matches, mismatches);
                    }
                    if matches > 0 && mismatches > 0 {
                        continue;
                    }
                    let flips = (m.count_ones() & 1) == 1;
                    let sum = next_fresh;
                    next_fresh += 1;
                    fresh.push(Fresh { var: sum, def: FreshDef::Xor(cut.clone()) });
                    let mut c = Cell { fa: cut.len() == 3, one, leaves: cut.clone(), negmask: m, sum, sum_neg: if one { !flips } else { flips }, carry: v, split };
                    if mismatches > 0 {
                        let all = (1u8 << c.leaves.len()) - 1;
                        c.negmask ^= all;
                        c.sum_neg = !c.sum_neg;
                        c.carry = -c.carry;
                        if !c.fa {
                            c.one = !c.one;
                        }
                    }
                    made = Some((c, l));
                    break;
                }
                let Some((c, base)) = made else { continue };
                // a signal a side's cell produces is read by one cell of
                // that side: a second reader would count it twice
                let mut twice = false;
                for s in 0..2 {
                    let in_cone = if s == 0 { cone_a.contains(&v) } else { cone_b.contains(&v) };
                    if !in_cone {
                        continue;
                    }
                    let produced: HashSet<i32> = side_cells[s].iter().flat_map(|&ci| [cells[ci].sum, cells[ci].carry_var()]).collect();
                    let consumed: HashSet<i32> = side_cells[s].iter().flat_map(|&ci| cells[ci].leaves.iter().copied()).collect();
                    twice |= c.leaves.iter().any(|l| produced.contains(l) && consumed.contains(l));
                    *work += side_cells[s].len() as u64;
                }
                if twice {
                    fresh.pop();
                    next_fresh -= 1;
                    continue;
                }
                // the weights: every leaf and the sum at the base's, the carry
                // one up (the base one down from the carry when the carry is
                // the only weighted signal)
                let mut fine = true;
                let (base, shift) = if base == c.carry_var() { (c.carry_var(), 1i64) } else { (base, 0) };
                for &l in &c.leaves {
                    if produced_any.contains(&l) {
                        fine &= uf.union(l, base, -shift);
                    } else if !bare_input(l) {
                        // (an unweighted leaf takes the cell's weight; a
                        // weighted one in the cell's component must agree;
                        // one in another component is left there: a leaf
                        // never ties two networks)
                        let (rl, ol) = uf.find(l);
                        let (rb, ob) = uf.find(base);
                        if !weighted.contains(&l) || (rl == rb && ol == ob - shift) {
                            uf.union(l, base, -shift);
                        }
                    }
                }
                fine &= uf.union(c.sum, base, -shift) && uf.union(c.carry_var(), base, 1 - shift);
                if !fine {
                    fresh.pop();
                    next_fresh -= 1;
                    continue;
                }
                weighted.extend(c.leaves.iter().copied());
                weighted.insert(c.sum);
                weighted.insert(c.carry_var());
                written.insert(c.sum, c.sum_lit());
                written.insert(c.carry_var(), c.carry_lit());
                produced_live.insert(c.sum);
                produced_live.insert(c.carry_var());
                if !c.fa && !c.one && c.leaves.len() == 2 {
                    half_of.entry(c.carry_var()).or_default().push(cells.len());
                }
                mark(&c, &mut covered, work); // the gate and those between it and the cut
                covered[v as usize] = true;
                let ci = cells.len();
                if cone_a.contains(&v) {
                    side_cells[0].push(ci);
                }
                if cone_b.contains(&v) {
                    side_cells[1].push(ci);
                }
                cells.push(c);
                changed = true;
                *work += 16;
            }
            if !changed && deferred && (!relaxed || !weak_ok) {
                // nothing but deferred work: the next round is a relaxed one
                changed = false;
            }
        }
    }
    if dbg {
        if std::env::var_os("SAT_STRUCT_DEBUG_GATES").is_some() {
            // the gates of each cone that no cell of the side covers
            for (s, cone_s) in [(0usize, &cone_a), (1usize, &cone_b)] {
                let mut covered: HashSet<i32> = HashSet::new();
                for &ci in &side_cells[s] {
                    let c = &cells[ci];
                    let stop_at: HashSet<i32> = c.leaves.iter().copied().collect();
                    let mut stack = vec![c.sum, c.carry_var()];
                    while let Some(u) = stack.pop() {
                        if u as usize >= net.rep.len() {
                            continue; // a fresh variable
                        }
                        let r = net.rep[u as usize].0;
                        if stop_at.contains(&r) || covered.contains(&r) {
                            continue;
                        }
                        let Some(g) = net.gates[r as usize].as_ref() else { continue };
                        covered.insert(r);
                        stack.extend(g.sup.iter().copied());
                    }
                }
                let mut rest: Vec<i32> = cone_s.iter().copied().filter(|r| !covered.contains(r)).collect();
                rest.sort_unstable();
                eprintln!("structarith: side {} has {} gates no cell covers:", s, rest.len());
                let (main_root, _) = uf.find(cells[side_cells[s][0]].sum);
                let mut wt = |x: i32| -> String {
                    if !weighted.contains(&x) {
                        return String::from("@?");
                    }
                    let (rx, ox) = uf.find(x);
                    if rx == main_root { format!("@{}", ox) } else { String::from("@?") }
                };
                for &r in &rest {
                    let g = net.gates[r as usize].as_ref().unwrap();
                    let ins: Vec<String> = g.sup.iter().map(|&u| {
                        let (ru, ng) = net.rep[u as usize];
                        format!("{}{}{}", if ng { "~" } else { "" }, ru, wt(ru))
                    }).collect();
                    eprintln!("    {}{} = tt {:#x} ({}) readers {}", r, wt(r), g.tt, ins.join(", "), readers[r as usize]);
                }
            }
        }
    }
    phase("virtual cells", *work);
    // the networks come to one scale. The compared signals are one bit:
    // they tie the two sides first. Then a leaf both sides read (a product
    // of an AND multiplier) ties what is still apart at the weights it is
    // read at (a pruned side's cells reach its compared bit only through
    // the carry network, so the bit alone does not tie them); a leaf read
    // at two weights (the product a_0 b_15 sits at column 15 of an AND
    // multiplier and at 16 in a Booth one's correction row) then only
    // disagrees with a tie already made, and is left alone
    // (bead SAT-playground-skz.4)
    {
        let (ra, ea) = uf.find(side_a);
        let (rb, eb) = uf.find(side_b);
        if ra != rb {
            uf.union(side_a, side_b, 0);
        } else if ea != eb {
            if dbg {
                eprintln!("structarith: the compared signals sit at exponents {} and {} of one network", ea, eb);
            }
            return;
        }
        let mut read_at: [HashMap<i32, i32>; 2] = [HashMap::new(), HashMap::new()]; // leaf -> a cell's sum reading it
        for s in 0..2 {
            for &ci in &side_cells[s] {
                for &l in &cells[ci].leaves {
                    read_at[s].entry(l).or_insert(cells[ci].sum);
                }
            }
        }
        let mut shared: Vec<i32> = read_at[0].keys().filter(|l| read_at[1].contains_key(l)).copied().collect();
        shared.sort_unstable();
        let mut tied = 0usize;
        for l in shared {
            let (sa, sb) = (read_at[0][&l], read_at[1][&l]);
            // (both cells read l at their own weight: they sit at one exponent)
            if uf.find(sa).0 != uf.find(sb).0 {
                uf.union(sa, sb, 0);
                tied += 1;
            }
        }
        *work += (read_at[0].len() + read_at[1].len()) as u64;
        if dbg && tied > 0 {
            eprintln!("structarith: {} ties between the sides' networks through shared leaves", tied);
        }
    }
    // 2c. the input leaves again, now with the virtual cells in place; a
    //     fused leaf (the xor of two signals, a fresh variable) is the xor
    //     of its signals' functions
    let (mut input_leaves, _) = classify_input_leaves(&cells, &side_cells, &net, work, &stopped, trace_var, &transparent);
    if stopped(*work) {
        return;
    }
    {
        let produced_all: HashSet<i32> = (0..2).flat_map(|t| side_cells[t].iter().map(|&ci| (cells[ci].sum, cells[ci].carry_var()))).flat_map(|(a, b)| [a, b]).collect();
        let mut memo: HashMap<i32, u64> = HashMap::new();
        for fr in &fresh {
            // (a mapped Booth side's last row reads its products as fresh
            // ands when no gate computes them, bead skz.13)
            let (sig, is_and) = match &fr.def {
                FreshDef::Xor(sig) => (sig, false),
                FreshDef::And(sig) => (sig, true),
                _ => continue,
            };
            if sig.len() != 2 || (fr.var as usize) < nv || input_leaves.contains_key(&fr.var) {
                continue;
            }
            if !side_cells.iter().any(|sc| sc.iter().any(|&ci| cells[ci].leaves.contains(&fr.var))) {
                continue; // (a chain cell's sum: produced, not a leaf)
            }
            let parts: Vec<Option<InputLeaf>> = sig
                .iter()
                .map(|&l| match input_leaves.get(&l.abs()) {
                    Some(il) => Some(il.clone()),
                    None => leaf_function(l.abs(), &net, &produced_all, &mut memo, work, trace_var).map(|(il, _)| il),
                })
                .collect();
            let (Some(a), Some(b)) = (&parts[0], &parts[1]) else { continue };
            let mut sup: Vec<i32> = a.sup.iter().chain(b.sup.iter()).copied().collect();
            sup.sort_unstable();
            sup.dedup();
            if sup.len() > MAX_SUPPORT {
                continue;
            }
            // each part's table lifted to the union of the inputs
            let lift = |il: &InputLeaf, negated: bool| -> u64 {
                let mut out = 0u64;
                for m in 0..(1usize << sup.len()) {
                    let mut sm = 0usize;
                    for (i, x) in il.sup.iter().enumerate() {
                        let pos = sup.iter().position(|y| y == x).unwrap();
                        if (m >> pos) & 1 == 1 {
                            sm |= 1 << i;
                        }
                    }
                    let bit = ((il.tt >> sm) & 1 == 1) != negated;
                    if bit {
                        out |= 1u64 << m;
                    }
                }
                out
            };
            let ta = lift(a, sig[0] < 0);
            let tb = lift(b, sig[1] < 0);
            *work += 2 << sup.len();
            input_leaves.insert(fr.var, InputLeaf { sup, tt: if is_and { ta & tb } else { ta ^ tb } });
        }
    }
    // each side's leaf polynomial as it stands (the cells' readings, the
    // constants of carry-in-one cells; what a chain adds later is tracked
    // beside it), at the scale of the lowest cell: the difference between
    // the sides is what a network sums that no cell reads (below)
    // the grid: every input's position along its operand, from the weighted
    // product gates (an and or a nand over two inputs): the exponent of a
    // product is the sum of its inputs' positions, and the two operands are
    // the two colours of the pair graph. The grid's own polynomial, every
    // pair at its weight whether or not a gate computes it (a mapped tree
    // folds products into its cells), is the sum a multiplier's side adds
    // up to, modulo what lies above the carry's column: a side whose
    // network reads what no row holds measures what it misses against it,
    // not against the other side, so neither side waits for the other
    // (bead skz.4)
    // (the positions from the side reading the most products: an AND
    // multiplier reads every pair at its place; a Booth side reads two
    // rows of products, one of them its correction row a * b_15 * 2^16,
    // which puts b_15 one column up and would make the grid a * (b + b_15
    // 2^15). The same grid serves both sides; two Booth sides have none)
    let mut grid_ops: [Option<(Vec<(i32, i64)>, Vec<(i32, i64)>)>; 2] = [None, None];
    {
        let mut candidates: Vec<(usize, Vec<(i32, i32, i64)>)> = (0..2).map(|s| (s, side_product_pairs(&cells, &side_cells[s], &pps, &mut uf, work).0)).collect();
        candidates.sort_by_key(|(s, pairs)| (std::cmp::Reverse(pairs.len()), *s));
        let (full, _) = (candidates[0].0, ());
        let other_len = candidates[1].1.len();
        if candidates[0].1.len() > 2 * other_len.max(1) {
            if let Some(pos) = operand_positions(&candidates[0].1, work) {
                let mut xs: Vec<(i32, i64)> = pos.iter().filter(|(_, &(_, c))| c == 0).map(|(&v, &(p, _))| (v, p)).collect();
                let mut ys: Vec<(i32, i64)> = pos.iter().filter(|(_, &(_, c))| c == 1).map(|(&v, &(p, _))| (v, p)).collect();
                xs.sort_unstable();
                ys.sort_unstable();
                if dbg {
                    eprintln!("structarith: the grid has {} x {} inputs from side {}'s {} product reads (the other side reads {})", xs.len(), ys.len(), full, candidates[0].1.len(), other_len);
                }
                grid_ops = [Some((xs.clone(), ys.clone())), Some((xs, ys))];
            }
        } else if dbg {
            eprintln!("structarith: no side reads a full grid of products ({} and {} reads): the networks measure against each other", candidates[0].1.len(), other_len);
        }
    }
    // (the scale: the lowest cell or the grid's lowest pair, whichever is
    // lower, so a network folding the bottom columns is measured too)
    let min_exp0: i64 = {
        let cells_min = side_cells.iter().flatten().map(|&ci| uf.find(cells[ci].sum).1).min().unwrap_or(0);
        let grid_min = grid_ops.iter().flatten().filter_map(|(xs, ys)| Some(xs.iter().map(|&(_, p)| p).min()? + ys.iter().map(|&(_, p)| p).min()?)).min();
        grid_min.map_or(cells_min, |g| g.min(cells_min))
    };
    let mut pre_poly: [Poly; 2] = [Poly::default(), Poly::default()];
    if dbg && std::env::var_os("SAT_STRUCT_DEBUG_DIFF").is_some() {
        for s in 0..2 {
            let produced: HashSet<i32> = side_cells[s].iter().flat_map(|&ci| [cells[ci].sum, cells[ci].carry_var()]).collect();
            let mut with = 0;
            let mut without: Vec<i32> = Vec::new();
            for &ci in &side_cells[s] {
                for &l in &cells[ci].leaves {
                    if produced.contains(&l) {
                        continue;
                    }
                    if input_leaves.contains_key(&l) {
                        with += 1;
                    } else {
                        without.push(l);
                    }
                }
            }
            eprintln!("structarith: side {}: {} unproduced leaves with a function, {} without {:?}", s, with, without.len(), &without[..without.len().min(12)]);
            let mut ex: Vec<(i32, i32, i64)> = side_cells[s].iter().take(400).map(|&ci| { let (r, e) = uf.find(cells[ci].sum); (cells[ci].sum, r, e) }).collect();
            ex.sort_by_key(|x| x.2);
            eprintln!("    side {} cells (sum, root, exponent) from {:?} to {:?}; min_exp0 {}", s, &ex[..ex.len().min(3)], &ex[ex.len().saturating_sub(3)..], min_exp0);
        }
    }
    for s in 0..2 {
        let produced: HashSet<i32> = side_cells[s].iter().flat_map(|&ci| [cells[ci].sum, cells[ci].carry_var()]).collect();
        for &ci in &side_cells[s] {
            let c = &cells[ci];
            let e = uf.find(c.sum).1 - min_exp0;
            if !(0..MAX_BITS as i64).contains(&e) {
                continue;
            }
            let w = 1i128 << e;
            for i in 0..c.leaves.len() {
                if produced.contains(&c.leaves[i]) {
                    continue;
                }
                if let Some(il) = input_leaves.get(&c.leaves[i]) {
                    pre_poly[s].add_tt(&il.sup, il.tt, w, c.leaf_lit(i) < 0);
                }
            }
            if c.one {
                pre_poly[s].add(Vec::new(), w);
            }
        }
        pre_poly[s].prune();
        *work += side_cells[s].len() as u64 * 4;
    }
    let mut grid_poly: [Poly; 2] = [Poly::default(), Poly::default()];
    let mut grid_pairs: [usize; 2] = [0, 0];
    for s in 0..2 {
        let Some((xs, ys)) = grid_ops[s].as_ref() else { continue };
        for &(x, px) in xs {
            for &(y, py) in ys {
                let e = px + py - min_exp0;
                if !(0..MAX_BITS as i64).contains(&e) {
                    continue;
                }
                let mut m = vec![x, y];
                m.sort_unstable();
                grid_poly[s].add(m, 1i128 << e);
                grid_pairs[s] += 1;
            }
        }
        *work += (xs.len() * ys.len()) as u64;
    }
    // 2d. the selector set of an input leaf that is more than a product
    //     or an input: up to three of its inputs under whose every
    //     assignment the leaf is a constant or a literal of one other
    //     input (a Booth digit's three bits for its row's bits). The
    //     leaves of a side with one selector set form a group: a Booth
    //     row, whose weighted sum is linear over products of a selector
    //     and another input (bead SAT-playground-skz.4)
    let selector_sets = |il: &InputLeaf, work: &mut u64| -> Vec<Vec<i32>> {
        let n = il.sup.len();
        let mut out: Vec<Vec<i32>> = Vec::new();
        if n < 2 || (n == 2 && il.tt == 0b1000) {
            return out; // an input or a product: no group
        }
        for mask in 1u32..(1u32 << n) {
            let k = mask.count_ones() as usize;
            if k > 3 || k >= n {
                continue;
            }
            *work += 1 << n;
            let mut fine = true;
            for values in 0u32..(1u32 << n) {
                if values & !mask != 0 {
                    continue;
                }
                if reduce_tt(il.tt, n, mask, values) == Reduced::Other {
                    fine = false;
                    break;
                }
            }
            if fine {
                out.push((0..n).filter(|&i| (mask >> i) & 1 == 1).map(|i| il.sup[i]).collect());
            }
        }
        out
    };
    // per side: leaf -> its selector set; the choice is the candidate most
    // leaves of the side share (a row's bits all name the digit's bits)
    let mut selector_of: [HashMap<i32, Vec<i32>>; 2] = [HashMap::new(), HashMap::new()];
    for s in 0..2 {
        let mut cands: HashMap<i32, Vec<Vec<i32>>> = HashMap::new();
        let mut votes: HashMap<Vec<i32>, usize> = HashMap::new();
        let mut leaves: Vec<i32> = side_cells[s].iter().flat_map(|&ci| cells[ci].leaves.iter().copied()).filter(|l| input_leaves.contains_key(l)).collect();
        leaves.sort_unstable();
        leaves.dedup();
        for &l in &leaves {
            let sets = selector_sets(&input_leaves[&l], work);
            for set in &sets {
                *votes.entry(set.clone()).or_default() += 1;
            }
            if !sets.is_empty() {
                cands.insert(l, sets);
            }
            if stopped(*work) {
                return;
            }
        }
        for (l, sets) in cands {
            let best = sets.iter().max_by_key(|set| (votes[*set], std::cmp::Reverse(set.len()), std::cmp::Reverse((*set).clone()))).unwrap().clone();
            selector_of[s].insert(l, best);
        }
    }
    if dbg {
        for s in 0..2 {
            let mut by_set: HashMap<&Vec<i32>, usize> = HashMap::new();
            for set in selector_of[s].values() {
                *by_set.entry(set).or_default() += 1;
            }
            if !by_set.is_empty() {
                let mut v: Vec<(&Vec<i32>, usize)> = by_set.into_iter().collect();
                v.sort();
                eprintln!("structarith: side {}: {} leaves in {} selector groups: {:?}", s, selector_of[s].len(), v.len(), &v[..v.len().min(12)]);
            }
        }
    }
    // 3. a leaf no cell of the side produces, a gate over gates that the
    //    other side does not read and no input leaf, is the carry a prefix
    //    network computes: one per side at most; the lemma below ties it
    //    to the virtual ripple chain
    let mut foreign: [Option<i32>; 2] = [None, None];
    for s in 0..2 {
        let produced: HashSet<i32> = side_cells[s].iter().flat_map(|&ci| [cells[ci].sum, cells[ci].carry_var()]).collect();
        let other_reads: HashSet<i32> = side_cells[1 - s].iter().flat_map(|&ci| cells[ci].leaves.iter().copied()).collect();
        let mut found: Vec<i32> = Vec::new();
        let mut reads: HashMap<i32, usize> = HashMap::new();
        for &ci in &side_cells[s] {
            for &l in &cells[ci].leaves {
                if produced.contains(&l) || other_reads.contains(&l) || input_leaves.contains_key(&l) {
                    continue;
                }
                if l as usize >= net.rep.len() {
                    continue; // a fused leaf: the proof defines it
                }
                let Some(g) = net.gates[l as usize].as_ref() else { continue };
                if g.sup.iter().any(|&u| net.gates[net.rep[u as usize].0 as usize].is_some()) {
                    *reads.entry(l).or_default() += 1;
                    if !found.contains(&l) {
                        found.push(l);
                    }
                }
            }
        }
        *work += side_cells[s].len() as u64;
        // (a mapped Booth side's top read as part of its network: the carry
        // into the compared column is a fresh variable its top cell reads,
        // bead skz.13)
        if let Some(vn) = virtual_net[s] {
            if found.is_empty() && side_cells[s].iter().any(|&ci| cells[ci].leaves.contains(&vn)) {
                found.push(vn);
                reads.insert(vn, side_cells[s].iter().filter(|&&ci| cells[ci].leaves.contains(&vn)).count());
            }
        }
        if found.len() > 1 {
            if dbg {
                eprintln!("structarith: side {} reads {} signals no cell produces: {:?}", s, found.len(), &found[..found.len().min(6)]);
            }
            return;
        }
        // read by one cell: the lemma cancels one read of the carry, and
        // the sums would keep every other (an xor tree reading the carry
        // twice adds it twice; the two reads cancel in the xor, not here)
        if let Some(&l) = found.first() {
            if reads[&l] != 1 {
                if dbg {
                    eprintln!("structarith: side {} reads the carry {} in {} cells", s, l, reads[&l]);
                }
                return;
            }
        }
        foreign[s] = found.first().copied();
    }
    if dbg {
        eprintln!("structarith: {} fresh variables for lone sums and carries; foreign leaves {:?}; side cells {} and {}", fresh.len(), foreign, side_cells[0].len(), side_cells[1].len());
    }
    let chosen: Vec<usize> = {
        let mut v: Vec<usize> = side_cells[0].iter().chain(side_cells[1].iter()).copied().collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    if !resolve_polarities(&mut cells, &chosen, &doubled) {
        if dbg {
            eprintln!("structarith: no consistent reading of the cells' polarities");
            if let Ok(list) = std::env::var("SAT_STRUCT_DEBUG_CELLS") {
                let vars: Vec<i32> = list.split(',').filter_map(|t| t.parse().ok()).collect();
                for &ci in &chosen {
                    let c = &cells[ci];
                    if vars.iter().any(|v| c.leaves.contains(v) || c.sum == *v || c.carry_var() == *v) {
                        let lits: Vec<i32> = (0..c.leaves.len()).map(|i| c.leaf_lit(i)).collect();
                        eprintln!("    cell {}: {} + ... = {} + 2*{}   leaves {:?} fa {} one {} weight {}", ci, lits.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(" + "), c.sum_lit(), c.carry_lit(), c.leaves, c.fa, c.one, weight_of_dbg(ci, &cells, &mut uf));
                    }
                }
            }
        }
        return;
    }
    *work += chosen.len() as u64 * 4;
    // 4. under a carry network the bits below the compared one are not in
    //    the file: a virtual ripple chain (fresh cells) adds up everything
    //    the side's cells leave over below and at the compared weight, and
    //    kitten proves the network's carry equal to the chain's bit at that
    //    weight. A carry-in-one reading of a cell puts a constant into the
    //    side's sum; the chain also adds the two's complement of the side's
    //    constant (fresh variables fixed to one), so what is left is a
    //    multiple of 2^(K+1), which the top block absorbs.
    struct Lemma {
        n_lit: i32,                   // the network's carry as the top cell reads it
        sk_lit: i32,                  // the chain's bit at the compared weight
        k: i64,                       // that weight's exponent
        gate_clauses: Vec<Vec<i32>>,  // the network and the cells it reads, each gate over its cut
        steps: Vec<(i32, i32, Vec<Vec<i32>>)>, // the equivalences proven on the way (a, b, kitten's learned clauses), the last one n_lit = sk_lit
    }
    let mut lemmas: [Option<Lemma>; 2] = [None, None];
    let mut trues: HashSet<i32> = HashSet::new(); // fresh variables fixed to one
    // every cell of a side in one weight component (the chain's exponents
    // are offsets from that component's root)
    for s in 0..2 {
        let mut comps: HashMap<i32, usize> = HashMap::new();
        for &ci in &side_cells[s] {
            let (r, _) = uf.find(cells[ci].sum);
            *comps.entry(r).or_default() += 1;
        }
        if comps.len() > 1 {
            if dbg {
                let mut v: Vec<(usize, i32)> = comps.iter().map(|(&r, &n)| (n, r)).collect();
                v.sort_unstable_by(|a, b| b.cmp(a));
                eprintln!("structarith: side {}'s cells are not one weighted network: {} components, sizes {:?}", s, comps.len(), &v[..v.len().min(5)]);
                for &(_, r) in v.iter().skip(1).take(2) {
                    for &ci in &side_cells[s] {
                        let c = &cells[ci];
                        if uf.find(c.sum).0 == r {
                            let lits: Vec<i32> = (0..c.leaves.len()).map(|i| c.leaf_lit(i)).collect();
                            eprintln!("    component {}: cell {:?} = {} + 2*{} fa {} {}", r, lits, c.sum_lit(), c.carry_lit(), c.fa, if c.sum > f.vars || c.carry_var() > f.vars { "virtual" } else { "" });
                        }
                    }
                }
            }
            return;
        }
    }
    let diagnose = std::env::var_os("SAT_STRUCT_DEBUG_NOCHAIN").is_some(); // (debug: skip the lemmas, print the polynomial difference)
    // (a side whose network reads what no row holds waits for the other
    // side's chain, which settles the polynomial difference it reads: that
    // side is taken up again after the other, once)
    let mut side_queue: std::collections::VecDeque<(usize, bool)> = (0..2).map(|s| (s, false)).collect();
    while let Some((s, retried)) = side_queue.pop_front() {
        let Some(n) = foreign[s] else { continue };
        if diagnose {
            continue;
        }
        let pre_poly_at_entry = pre_poly[s].terms.clone(); // (restored when the side is put off: the network-read leaves below add to it)
        let o = if s == 0 { side_a } else { side_b };
        // the top cell: the one producing the side's output, which reads n
        let Some(&top) = side_cells[s].iter().find(|&&ci| cells[ci].sum == o || cells[ci].carry_var() == o) else {
            if dbg {
                eprintln!("structarith: side {}: no cell produces the compared signal {}", s, o);
            }
            return;
        };
        let (_, e_top) = uf.find(cells[top].sum);
        let k = if cells[top].sum == o { e_top } else { e_top + 1 };
        let Some(reader) = side_cells[s].iter().copied().find(|&ci| cells[ci].leaves.contains(&n)) else {
            if dbg {
                eprintln!("structarith: side {}: no cell reads the foreign leaf {}", s, n);
            }
            return;
        };
        let (_, e_n) = uf.find(cells[reader].sum);
        if e_n > k {
            if dbg {
                let c = &cells[reader];
                let lits: Vec<i32> = (0..c.leaves.len()).map(|i| c.leaf_lit(i)).collect();
                eprintln!("structarith: side {}: the network's carry {} sits at exponent {}, above the output at {} (read by {:?} = {} + 2*{}, {})", s, n, e_n, k, lits, c.sum_lit(), c.carry_lit(), if c.sum > f.vars || c.carry_var() > f.vars { "virtual" } else { "real" });
            }
            return;
        }
        // the chain runs up to the carry's own column: a cell there (or
        // above) may read the carry and add on from it
        let k = e_n;
        let n_lit = (0..cells[reader].leaves.len()).find(|&i| cells[reader].leaves[i] == n).map(|i| cells[reader].leaf_lit(i)).unwrap();
        // what the side's cells leave over at or below k, the output and
        // everything the carry feeds (the cell reading it, the cells
        // reading those) aside: the chain adds up what the network adds up
        let consumed: HashSet<i32> = side_cells[s].iter().flat_map(|&ci| cells[ci].leaves.iter().copied()).collect();
        let mut above: HashSet<i32> = HashSet::new(); // outputs of the cells over the carry
        {
            // (the side's cells by leaf, so a long chain costs its length,
            // not its square; the budget checked on the way)
            let mut cells_by_leaf: HashMap<i32, Vec<usize>> = HashMap::new();
            for &ci in &side_cells[s] {
                for &l in &cells[ci].leaves {
                    cells_by_leaf.entry(l).or_default().push(ci);
                }
                *work += cells[ci].leaves.len() as u64;
            }
            let mut frontier: Vec<i32> = vec![n];
            while let Some(x) = frontier.pop() {
                if stopped(*work) {
                    return;
                }
                for &ci in cells_by_leaf.get(&x).map_or(&[][..], |v| &v[..]) {
                    let c = &cells[ci];
                    for out in [c.sum, c.carry_var()] {
                        if above.insert(out) {
                            frontier.push(out);
                        }
                    }
                    *work += 1;
                }
            }
        }
        let mut pool: std::collections::BTreeMap<i64, Vec<i32>> = std::collections::BTreeMap::new();
        let mut pool_vars: Vec<i32> = Vec::new(); // what the chain reads: their cells join the lemma
        let mut constant: i128 = 0; // the carry-in-one constants at or below k, relative to e_lo below
        let mut synthesized_constant: i128 = 0; // what the addend synthesis below adds, at the scale of min_exp0
        let mut e_lo = k;
        for &ci in &side_cells[s] {
            let c = &cells[ci];
            let (_, e) = uf.find(c.sum);
            for (lit, ee) in [(c.sum_lit(), e), (c.carry_lit(), e + 1)] {
                if lit.abs() != o && !above.contains(&lit.abs()) && !consumed.contains(&lit.abs()) && ee <= k {
                    pool.entry(ee).or_default().push(lit);
                    pool_vars.push(lit.abs());
                    e_lo = e_lo.min(ee);
                }
            }
            if c.one && e <= k {
                e_lo = e_lo.min(e);
            }
        }
        // what the network reads that no cell produces or reads: a product
        // or a Booth row bit the tree never touched (a column with one
        // addend goes straight to the final adder). A product's weight is
        // the grid's; a row bit's is its row's: the leaf with the same
        // selector set and function that some cell reads, shifted along
        // the operand by the grid's positions (bead SAT-playground-skz.4)
        {
            let mut memo: HashMap<i32, u64> = HashMap::new();
            let reads_here: HashSet<i32> = consumed.clone();
            let produced_all: HashSet<i32> = (0..2).flat_map(|t| side_cells[t].iter().map(|&ci| (cells[ci].sum, cells[ci].carry_var()))).flat_map(|(a, b)| [a, b]).filter(|v| !transparent.contains(v)).collect();
            let mut found: Vec<(i32, InputLeaf)> = Vec::new();
            let mut seen: HashSet<i32> = HashSet::new();
            // (a virtual carry has no gate: the network is under the
            // compared signal, bead skz.13)
            let mut stack: Vec<i32> = if (n as usize) < nv { vec![n] } else { net.gates[o as usize].as_ref().map(|g| g.sup.clone()).unwrap_or_default() };
            while let Some(u) = stack.pop() {
                let u = if (u as usize) < nv { net.rep[u as usize].0 } else { u };
                if !seen.insert(u) {
                    continue;
                }
                *work += 1;
                if u != n && (produced_all.contains(&u) || reads_here.contains(&u)) {
                    continue; // a cell's output: in the pool already; or a cell's leaf
                }
                let Some(g) = ((u as usize) < nv).then(|| net.gates[u as usize].as_ref()).flatten() else { continue };
                if u != n {
                    if let Some((il, _)) = leaf_function(u, &net, &produced_all, &mut memo, work, trace_var) {
                        found.push((u, il));
                        continue;
                    }
                }
                stack.extend(g.sup.iter().copied());
                if stopped(*work) {
                    return;
                }
            }
            found.sort_by_key(|(u, _)| *u);
            // (a side found by function: its row bits no cell reads go to
            // the pool at their columns, the walk's leaves are theirs or
            // their parts, and the polynomial difference below covers the
            // rest, bead skz.13)
            let by_function = booth_side.values().any(|&t| t == s);
            if by_function {
                found.clear();
                let anchor = booth_cols.iter().find(|&&(ci, _)| booth_side.get(&ci) == Some(&s)).copied();
                if let Some((c0, col0)) = anchor {
                    let (_, e0) = uf.find(cells[c0].sum);
                    let rows = booth_rows[s].clone();
                    for (lit, col) in rows {
                        let e = e0 + (col - col0);
                        if e > k || e < min_exp0 {
                            continue;
                        }
                        let v = lit.abs();
                        let il = if (v as usize) < nv {
                            leaf_function(v, &net, &HashSet::new(), &mut memo, work, trace_var).map(|(il, _)| il)
                        } else {
                            // (a fused row bit: the xor of its select gate and the sign)
                            match fresh.iter().find(|fr| fr.var == v).map(|fr| fr.def.clone()) {
                                Some(FreshDef::Xor(sig)) if sig.len() == 2 => {
                                    let parts: Vec<Option<InputLeaf>> = sig.iter().map(|&l| leaf_function(l.abs(), &net, &HashSet::new(), &mut memo, work, trace_var).map(|(il, _)| il)).collect();
                                    match (&parts[0], &parts[1]) {
                                        (Some(a), Some(b)) => {
                                            let mut sup: Vec<i32> = a.sup.iter().chain(b.sup.iter()).copied().collect();
                                            sup.sort_unstable();
                                            sup.dedup();
                                            let lift = |il: &InputLeaf, negated: bool| -> u64 {
                                                let mut out = 0u64;
                                                for m in 0..(1usize << sup.len()) {
                                                    let mut sm = 0usize;
                                                    for (i, x) in il.sup.iter().enumerate() {
                                                        if (m >> sup.iter().position(|y| y == x).unwrap()) & 1 == 1 {
                                                            sm |= 1 << i;
                                                        }
                                                    }
                                                    if ((il.tt >> sm) & 1 == 1) != negated {
                                                        out |= 1u64 << m;
                                                    }
                                                }
                                                out
                                            };
                                            (sup.len() <= MAX_SUPPORT).then(|| InputLeaf { tt: lift(a, sig[0] < 0) ^ lift(b, sig[1] < 0), sup: sup.clone() })
                                        }
                                        _ => None,
                                    }
                                }
                                _ => None,
                            }
                        };
                        let Some(il) = il else {
                            if dbg && std::env::var_os("SAT_STRUCT_DEBUG_DIFF").is_some() {
                                eprintln!("    row bit {} at column {}: no function", lit, col);
                            }
                            continue;
                        };
                        *work += 8;
                        uf.reserve(v);
                        let (_, ea) = uf.find(cells[c0].sum);
                        let placed = if !weighted.contains(&v) && uf.union(v, cells[c0].sum, e - ea) {
                            weighted.insert(v);
                            true
                        } else {
                            uf.find(v).1 == e
                        };
                        // (a signal at another weight already, a product the
                        // other side reads one column lower than the
                        // correction row: a fresh copy of it at this one)
                        // (the copy is of the variable, x = v, so the leaf's
                        // function stays v's; the literal keeps its sign:
                        // review, 2026-10-24)
                        let (v, lit) = if placed {
                            (v, lit)
                        } else {
                            let x = next_fresh;
                            next_fresh += 1;
                            fresh.push(Fresh { var: x, def: FreshDef::Xor(vec![v]) });
                            uf.reserve(x);
                            if !uf.union(x, cells[c0].sum, e - ea) {
                                continue;
                            }
                            weighted.insert(x);
                            if dbg && std::env::var_os("SAT_STRUCT_DEBUG_DIFF").is_some() {
                                eprintln!("    row bit {} at column {}: weighted elsewhere, a copy {}", lit, col, x);
                            }
                            (x, if lit < 0 { -x } else { x })
                        };
                        if dbg && std::env::var_os("SAT_STRUCT_DEBUG_DIFF").is_some() {
                            eprintln!("    row bit {} at column {} (exponent {}) to the pool", lit, col, e);
                        }
                        pool.entry(e).or_default().push(lit);
                        pool_vars.push(v);
                        e_lo = e_lo.min(e);
                        if (0..MAX_BITS as i64).contains(&(e - min_exp0)) {
                            pre_poly[s].add_tt(&il.sup, il.tt, 1i128 << (e - min_exp0), lit < 0);
                        }
                        input_leaves.entry(v).or_insert(il);
                    }
                }
            }
            // the row templates: per selector set, the cell-read leaves
            // with their weights
            let mut templates: HashMap<Vec<i32>, Vec<(i32, i64)>> = HashMap::new(); // selector set -> (leaf, exponent)
            for &ci in &side_cells[s] {
                let c = &cells[ci];
                let (_, e) = uf.find(c.sum);
                for &l in &c.leaves {
                    if let Some(set) = selector_of[s].get(&l) {
                        templates.entry(set.clone()).or_default().push((l, e));
                    }
                }
            }
            // an input's position along its operand, from the grid
            let shift = |x: i32, y: i32, uf: &mut WeightedUf| -> Option<i64> {
                if x == y {
                    return Some(0);
                }
                let mut best: Option<i64> = None;
                for (&(a, b), &v) in &pps {
                    let partner = if a == x { b } else if b == x { a } else { continue };
                    let (lo, hi) = (partner.min(y), partner.max(y));
                    let Some(&v2) = pps.get(&(lo, hi)) else { continue };
                    if !weighted.contains(&v) || !weighted.contains(&v2) {
                        continue;
                    }
                    let (r1, e1) = uf.find(v);
                    let (r2, e2) = uf.find(v2);
                    if r1 == r2 {
                        best = Some(e1 - e2);
                        break;
                    }
                }
                best
            };
            let mut unplaced: Vec<(i32, InputLeaf)> = Vec::new();
            for (u, il) in found {
                *work += 8;
                // a product: the grid's weight
                if il.sup.len() == 2 && il.tt == 0b1000 && weighted.contains(&u) {
                    let (_, e) = uf.find(u);
                    if e <= k {
                        pool.entry(e).or_default().push(u);
                        pool_vars.push(u);
                        e_lo = e_lo.min(e);
                        if (0..MAX_BITS as i64).contains(&(e - min_exp0)) {
                            pre_poly[s].add_tt(&il.sup, il.tt, 1i128 << (e - min_exp0), false);
                        }
                        input_leaves.entry(u).or_insert(il);
                    }
                    continue;
                }
                // a row bit: the template with its selector set, matched by
                // the function over the shifted inputs
                let sets = selector_sets(&il, work);
                let mut placed = false;
                for set in &sets {
                    let Some(refs) = templates.get(set) else { continue };
                    let a_u: Vec<i32> = il.sup.iter().copied().filter(|x| !set.contains(x)).collect();
                    for &(rl, re) in refs {
                        let ril = input_leaves[&rl].clone();
                        let ril = &ril;
                        let a_r: Vec<i32> = ril.sup.iter().copied().filter(|x| !set.contains(x)).collect();
                        if a_r.len() != a_u.len() || ril.sup.len() != il.sup.len() {
                            continue;
                        }
                        // the inputs paired by position: every pair shifted alike
                        let mut pairs: Vec<(i32, i32, i64)> = Vec::new();
                        let mut fine = true;
                        for &x in &a_u {
                            let mut best: Option<(i32, i64)> = None;
                            for &y in &a_r {
                                if let Some(d) = shift(x, y, &mut uf) {
                                    if best.map_or(true, |(_, bd)| d.abs() < bd.abs()) {
                                        best = Some((y, d));
                                    }
                                }
                            }
                            match best {
                                Some((y, d)) => pairs.push((x, y, d)),
                                None => {
                                    fine = false;
                                    break;
                                }
                            }
                        }
                        if !fine || pairs.iter().any(|&(_, _, d)| d != pairs[0].2) {
                            continue;
                        }
                        let d = pairs[0].2;
                        let mut ys: Vec<i32> = pairs.iter().map(|&(_, y, _)| y).collect();
                        ys.sort_unstable();
                        ys.dedup();
                        if ys.len() != a_r.len() {
                            continue;
                        }
                        // u's table with its inputs renamed to the reference's must equal the reference's
                        let perm: Vec<usize> = il
                            .sup
                            .iter()
                            .map(|&x| {
                                let y = if set.contains(&x) { x } else { pairs.iter().find(|&&(px, _, _)| px == x).unwrap().1 };
                                ril.sup.iter().position(|&z| z == y).unwrap()
                            })
                            .collect();
                        let nn = il.sup.len();
                        let mut same = true;
                        for m in 0..(1usize << nn) {
                            // m indexes u's inputs; the reference's index puts bit i at perm[i]
                            let mut rm = 0usize;
                            for i in 0..nn {
                                if (m >> i) & 1 == 1 {
                                    rm |= 1 << perm[i];
                                }
                            }
                            if ((il.tt >> m) & 1) != ((ril.tt >> rm) & 1) {
                                same = false;
                                break;
                            }
                        }
                        *work += 1 << nn;
                        if !same {
                            continue;
                        }
                        let e = re + d;
                        if e <= k {
                            pool.entry(e).or_default().push(u);
                            pool_vars.push(u);
                            e_lo = e_lo.min(e);
                            if (0..MAX_BITS as i64).contains(&(e - min_exp0)) {
                                pre_poly[s].add_tt(&il.sup, il.tt, 1i128 << (e - min_exp0), false);
                            }
                            input_leaves.entry(u).or_insert(il.clone());
                            if dbg {
                                eprintln!("structarith: side {}: the network reads the row bit {} (selector {:?}), at weight {} by the template {} at {} shifted {}", s, u, set, e, rl, re, d);
                            }
                        }
                        placed = true;
                        break;
                    }
                    if placed {
                        break;
                    }
                }
                if !placed {
                    unplaced.push((u, il));
                }
            }
            // what the network sums that is no signal at all: a generator
            // folds a row's lowest bits and the sign's +1 into the final
            // adder's gates (the and-not chain of a Booth multiplier's
            // first columns). The two sides' polynomials say what is
            // missing: the difference, at the scale of its lowest
            // coefficient, is a function of a few inputs; split into unit
            // addends (fresh variables of the proof, each a truth table over
            // those inputs) and a constant, it joins the chain, and the
            // lemma below checks the network against it. Only the side
            // whose other side is settled (no network, or its chain made)
            // reads the difference as its own
            if !unplaced.is_empty() || by_function {
                let other = 1 - s;
                let settled = grid_pairs[s] > 0 || foreign[other].is_none() || lemmas[other].is_some();
                if !settled {
                    if !retried && side_queue.iter().any(|&(t, _)| t == other) {
                        if dbg {
                            eprintln!("structarith: side {}: the network reads {} gates no row holds (for example {:?}); the other side first", s, unplaced.len(), unplaced.first().map(|u| u.0));
                            for (u, il) in &unplaced {
                                eprintln!("    gate {} over {:?} table {:#x}", u, il.sup, il.tt);
                            }
                        }
                        pre_poly[s].terms = pre_poly_at_entry;
                        side_queue.push_back((s, true));
                        continue;
                    }
                    if dbg {
                        eprintln!("structarith: side {}: the network reads {} gates no row holds (for example {:?}), and the other side is not settled", s, unplaced.len(), unplaced.first().map(|u| u.0));
                    }
                    return;
                }
                if !(0..MAX_BITS as i64).contains(&(k - min_exp0)) {
                    if dbg {
                        eprintln!("structarith: side {}: the sides' difference would need {} weights", s, k - min_exp0 + 1);
                    }
                    return;
                }
                let modulus_k = 1i128 << (k - min_exp0 + 1); // what is above the carry's column does not reach it
                let mut diff: Vec<(Vec<i32>, i128)> = Vec::new();
                let mut seen_m: HashSet<&Vec<i32>> = HashSet::new();
                let mut too_wide = false;
                // (the grid's polynomial when there is one, else the other
                // side's; the constant is left out either way, the chain's
                // fixed ones settle it)
                let reference: &HashMap<Vec<i32>, i128> = if grid_pairs[s] > 0 { &grid_poly[s].terms } else { &pre_poly[other].terms };
                for m in reference.keys().chain(pre_poly[s].terms.keys()) {
                    *work += 2;
                    if !seen_m.insert(m) {
                        continue;
                    }
                    let a = reference.get(m).copied().unwrap_or(0).rem_euclid(modulus_k);
                    let b = pre_poly[s].terms.get(m).copied().unwrap_or(0).rem_euclid(modulus_k);
                    let mut d = (a - b).rem_euclid(modulus_k);
                    if d > modulus_k / 2 {
                        d -= modulus_k;
                    }
                    if d != 0 && !m.is_empty() {
                        if m.len() > MAX_SUPPORT {
                            too_wide = true;
                            break; // (a monomial over more inputs than a table holds)
                        }
                        diff.push((m.clone(), d));
                    }
                    if stopped(*work) {
                        return;
                    }
                }
                diff.sort(); // (a fixed order: the fresh numbers and the pool must not vary from run to run)
                if dbg && std::env::var_os("SAT_STRUCT_DEBUG_DIFF").is_some() {
                    eprintln!("structarith: side {}: min_exp0 {} k {} difference:", s, min_exp0, k);
                    for (m, d) in &diff {
                        eprintln!("    diff {:?} {}", m, d);
                    }
                    for (u, il) in &unplaced {
                        eprintln!("    unplaced {} over {:?} table {:#x}", u, il.sup, il.tt);
                    }
                }
                if diff.is_empty() && !too_wide && unplaced.is_empty() {
                    // (a side found by function whose rows the pool holds in full)
                } else if too_wide || diff.is_empty() {
                    if dbg {
                        eprintln!("structarith: side {}: the network reads {} gates no row holds, and the sides' difference has {} monomials{}", s, unplaced.len(), diff.len(), if too_wide { ", one too wide" } else { "" });
                    }
                    return;
                }
                // each monomial of the difference, at each power of two of its
                // coefficient, is one addend of the chain: a fresh and of the
                // monomial's inputs, read plain for a positive coefficient and
                // complemented for a negative one (-m = (1 - m) - 1: the
                // constant goes with the fixed ones). The lemma below checks
                // the network against them
                let mut n_added = 0usize;
                let mut n_neg = 0usize;
                for (mono, d) in &diff {
                    let neg = *d < 0;
                    let mag = d.unsigned_abs();
                    for c in 0..128u32 {
                        if (mag >> c) & 1 == 0 {
                            continue;
                        }
                        let col = min_exp0 + c as i64;
                        if col > k {
                            continue;
                        }
                        n_added += 1;
                        if n_added > 512 {
                            if dbg {
                                eprintln!("structarith: side {}: the sides' difference needs more than 512 addends", s);
                            }
                            return;
                        }
                        let g = next_fresh;
                        next_fresh += 1;
                        fresh.push(Fresh { var: g, def: FreshDef::And(mono.clone()) });
                        uf.reserve(g);
                        let (_, e_reader) = uf.find(cells[reader].sum);
                        if !uf.union(g, cells[reader].sum, col - e_reader) {
                            return;
                        }
                        pool.entry(col).or_default().push(if neg { -g } else { g });
                        pool_vars.push(g);
                        e_lo = e_lo.min(col);
                        let tt_and = 1u64 << ((1usize << mono.len()) - 1);
                        pre_poly[s].add_tt(mono, tt_and, 1i128 << c, neg);
                        input_leaves.insert(g, InputLeaf { sup: mono.clone(), tt: tt_and });
                        if neg {
                            synthesized_constant -= 1i128 << c;
                            n_neg += 1;
                        }
                    }
                    *work += 8;
                }
                if dbg {
                    eprintln!("structarith: side {}: the network sums {} fresh addends ({} complemented) for {} gates no row holds (difference of {} monomials, for example {:?})", s, n_added, n_neg, unplaced.len(), diff.len(), &diff[..diff.len().min(4)]);
                    for (mono, _) in diff.iter().take(4) {
                        let readers: Vec<String> = input_leaves.iter().filter(|(_, il)| { let mut m = il.sup.clone(); m.sort_unstable(); &m == mono }).map(|(&v, il)| format!("{} tt {:#x} weighted {} exp {:?}", v, il.tt, weighted.contains(&v), if weighted.contains(&v) { Some(uf.find(v)) } else { None })).collect();
                        eprintln!("    {:?}: grid {:?}, side {:?}; gates {:?}", mono, grid_poly[s].terms.get(mono), pre_poly[s].terms.get(mono), readers);
                    }
                }
            }
        }
        if k - e_lo + 1 > MAX_BITS as i64 {
            if dbg {
                eprintln!("structarith: side {}: the chain would span {} weights", s, k - e_lo + 1);
            }
            return;
        }
        // (summed modulo 2^(K+1) as it goes: only the remainder matters,
        // and a sum of many top-weight ones would overflow)
        // Which ones the chain complements: those of the pass's own lone
        // cells (an or gate read as the carry of x + y + 1 with a fresh
        // sum: the network adds x and y, the one is the reading's). A real
        // cell read with a one (a half adder over two complemented bits
        // whose outputs are consumed complemented: 1 + x + y) adds its one
        // to what the network sums (a Booth multiplier's sign-extension
        // constants are built exactly so), and the chain keeps it
        let modulus = 1i128 << (k - e_lo + 1);
        for &ci in &side_cells[s] {
            let c = &cells[ci];
            let (_, e) = uf.find(c.sum);
            let lone = c.sum > f.vars || c.carry_var() > f.vars;
            if c.one && e <= k && lone {
                constant = (constant + (1i128 << (e - e_lo))) % modulus;
            }
        }
        *work += side_cells[s].len() as u64 * 2;
        let mut complement = (modulus - constant.rem_euclid(modulus)) % modulus;
        if synthesized_constant != 0 {
            // the synthesized addends' constant, as fixed ones too (at the
            // scale of min_exp0, moved to e_lo's: e_lo is at or below the
            // addends' column)
            let scaled = if min_exp0 >= e_lo { synthesized_constant << (min_exp0 - e_lo) } else { synthesized_constant >> (e_lo - min_exp0) };
            complement = (complement + scaled).rem_euclid(modulus);
        }
        // (a side found by function, a mapped Booth tree, may leave some of
        // the multiplier's constants to the network, folded into its top:
        // its fixed ones are what makes the side's polynomial the grid's,
        // the complement of its constant term, which counts every reading
        // with a one, every negated leaf and every complemented addend,
        // bead skz.13)
        // (and a signal read doubled leaves 2^e of its own in the sums: x is
        // read as ~x at its column and as x a column above, 2x + (1 - x) is
        // x + 1, bead skz.4)
        if booth_side.values().any(|&t| t == s) && grid_pairs[s] > 0 && min_exp0 >= e_lo {
            let pc = pre_poly[s].terms.get(&Vec::new()).copied().unwrap_or(0);
            let scaled = pc.checked_shl((min_exp0 - e_lo) as u32).unwrap_or(0);
            let mut extra: i128 = 0;
            let mut twice: Vec<i32> = doubled.iter().filter(|(_, &ci)| side_cells[s].contains(&ci)).map(|(&x, _)| x).collect();
            twice.sort_unstable();
            for x in twice {
                let (_, ex) = uf.find(x);
                if ex >= e_lo && ex <= k {
                    extra = (extra + (1i128 << (ex - e_lo))) % modulus;
                }
            }
            complement = (-scaled - extra).rem_euclid(modulus);
        }
        if dbg && std::env::var_os("SAT_STRUCT_DEBUG_POOL").is_some() {
            let pc = pre_poly[s].terms.get(&Vec::new()).copied().unwrap_or(0);
            eprintln!("structarith: side {}: e_lo {} min_exp0 {} k {}: lone ones {} complement {} synthesized {} polynomial constant {} (mod 2^{} that is {})", s, e_lo, min_exp0, k, constant, complement, synthesized_constant, pc, k - min_exp0 + 1, pc.rem_euclid(1i128 << (k - min_exp0 + 1)));
        }
        // (the chain in the complemented domain when that needs fewer fixed
        // ones: a side whose leftovers its cells write complemented, as a
        // mapped Booth side's are once its products read plain, needs a
        // fixed one at nearly every column in the plain domain, and its
        // carries are then none of the network's. Summing the complements
        // instead, ~S = -S - 1 is the sum of the ~items plus K' = -W - K - 1,
        // W the items' weights: few fixed ones, and the chain's carries are
        // the network's own, complemented. Each chain cell is the
        // complement reading of its cell over the complements, so it reads
        // the items as written, bead skz.13)
        let mut negated_domain = false;
        if booth_side.values().any(|&t| t == s) {
            let mut wsum: i128 = 0;
            for (&e, items) in &pool {
                if e >= e_lo && e <= k {
                    wsum = (wsum + (items.len() as i128 % modulus) * ((1i128 << (e - e_lo)) % modulus)) % modulus;
                }
            }
            let k_alt = (-wsum - complement - 1).rem_euclid(modulus);
            if k_alt.count_ones() < complement.count_ones() {
                if dbg {
                    eprintln!("structarith: side {}: the chain sums the complements ({} fixed ones instead of {})", s, k_alt.count_ones(), complement.count_ones());
                }
                negated_domain = true;
                complement = k_alt;
            }
        }
        let (root, _) = uf.find(cells[reader].sum); // the side's one component
        for j in 0..(k - e_lo + 1) {
            if (complement >> j) & 1 == 1 {
                let t = next_fresh;
                next_fresh += 1;
                trues.insert(t);
                uf.reserve(t);
                // weighted like the column it joins (so a chain cell over
                // fixed ones alone sits at the right weight too)
                if !uf.union(t, root, e_lo + j) {
                    return;
                }
                pool.entry(e_lo + j).or_default().push(if negated_domain { -t } else { t });
            }
        }
        if dbg && std::env::var_os("SAT_STRUCT_DEBUG_LEMMA").is_some() {
            for &ci in &side_cells[s] {
                let c = &cells[ci];
                let (_, e) = uf.find(c.sum);
                let lits: Vec<i32> = (0..c.leaves.len()).map(|i| c.leaf_lit(i)).collect();
                eprintln!("    side {} {} cell at {}: {:?} (+{}) = {} + 2*{}", s, if c.sum > f.vars || c.carry_var() > f.vars { "virtual" } else { "real" }, e, lits, c.one as u8, c.sum_lit(), c.carry_lit());
            }
        }
        if let Ok(path) = std::env::var("SAT_STRUCT_DEBUG_POOL") {
            // (debug: the chain's pool, the fixed ones and every fresh
            // definition, to check the arithmetic outside)
            use std::io::Write;
            if let Ok(mut fh) = std::fs::File::create(format!("{}.{}", path, s)) {
                let _ = writeln!(fh, "k {} e_lo {} min_exp0 {} n {} o {}", k, e_lo, min_exp0, n_lit, o);
                for (e, items) in &pool {
                    for &l in items {
                        let _ = writeln!(fh, "pool {} {}", e, l);
                    }
                }
                for &t in &trues {
                    let _ = writeln!(fh, "true {}", t);
                }
                for fr in &fresh {
                    let (kind, l) = match &fr.def {
                        FreshDef::Xor(l) => ("xor", l),
                        FreshDef::And(l) => ("and", l),
                        FreshDef::Maj(l) => ("maj", l),
                    };
                    let _ = writeln!(fh, "fresh {} {} {}", fr.var, kind, l.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" "));
                }
                for &ci in &side_cells[s] {
                    let c = &cells[ci];
                    let (_, e) = uf.find(c.sum);
                    let lits: Vec<String> = (0..c.leaves.len()).map(|i| c.leaf_lit(i).to_string()).collect();
                    let _ = writeln!(fh, "cell {} {} {} {} {} {}", e, c.one as u8, c.sum_lit(), c.carry_lit(), lits.len(), lits.join(" "));
                }
            }
        }
        // the chain, column by column
        let mut carries: Vec<i32> = Vec::new();
        let mut sk_lit: Option<i32> = None;
        let mut chain_cells = 0usize;
        let mut chain_col: Vec<i64> = Vec::new(); // per chain cell, its column
        for e in e_lo..=k {
            let mut items: Vec<i32> = pool.remove(&e).unwrap_or_default();
            if dbg && std::env::var_os("SAT_STRUCT_DEBUG_LEMMA").is_some() {
                eprintln!("    side {} column {}: {:?} + carries {:?}", s, e, items, carries);
            }
            items.append(&mut carries);
            while items.len() >= 2 {
                let take = if items.len() >= 3 { 3 } else { 2 };
                let lits: Vec<i32> = items.drain(..take).collect();
                let sigma = next_fresh;
                let kappa = next_fresh + 1;
                next_fresh += 2;
                // (over the complements in the complemented domain)
                let def_lits: Vec<i32> = if negated_domain { lits.iter().map(|&l| -l).collect() } else { lits.clone() };
                fresh.push(Fresh { var: sigma, def: FreshDef::Xor(def_lits.clone()) });
                fresh.push(Fresh { var: kappa, def: if take == 3 { FreshDef::Maj(def_lits.clone()) } else { FreshDef::And(def_lits.clone()) } });
                let leaves: Vec<i32> = lits.iter().map(|l| l.abs()).collect();
                let negmask: u8 = lits.iter().enumerate().map(|(i, &l)| ((l < 0) as u8) << i).sum();
                let base = leaves[0];
                let mut fine = true;
                for &l in &leaves {
                    fine &= uf.union(l, base, 0);
                }
                fine &= uf.union(sigma, base, 0) && uf.union(kappa, base, 1);
                if !fine {
                    if dbg {
                        eprintln!("structarith: side {}: the chain's weights contradict the cells'", s);
                    }
                    return;
                }
                let (_, eb) = uf.find(base);
                if eb != e {
                    if dbg {
                        eprintln!("structarith: side {}: a chain cell at exponent {} reads a signal at {}", s, e, eb);
                    }
                    return;
                }
                // (the complement reading: ~a + ~b (+ ~c) = sigma + 2 kappa is
                // a + b (+ c) (+ 1) = ~sigma + 2 ~kappa)
                cells.push(Cell { fa: take == 3, one: negated_domain && take == 2, leaves, negmask, sum: sigma, sum_neg: negated_domain, carry: if negated_domain { -kappa } else { kappa }, split: None });
                side_cells[s].push(cells.len() - 1);
                chain_cells += 1;
                chain_col.push(e);
                items.push(if negated_domain { -sigma } else { sigma });
                carries.push(if negated_domain { -kappa } else { kappa });
                *work += 8;
            }
            if e == k {
                sk_lit = items.first().copied();
            }
            if !items.is_empty() && e < k {
                // the column's bit stays as it is (a leftover, or a chain sum)
            }
        }
        if let (Ok(path), Some(skl)) = (std::env::var("SAT_STRUCT_DEBUG_POOL"), sk_lit) {
            // (debug: the chain as built, its bit at the compared weight)
            use std::io::Write;
            if let Ok(mut fh) = std::fs::OpenOptions::new().append(true).open(format!("{}.{}", path, s)) {
                let _ = writeln!(fh, "sk {} negated_domain {}", skl, negated_domain);
                for fr in fresh.iter().rev().take(2 * chain_cells) {
                    let (kind, l) = match &fr.def {
                        FreshDef::Xor(l) => ("xor", l),
                        FreshDef::And(l) => ("and", l),
                        FreshDef::Maj(l) => ("maj", l),
                    };
                    let _ = writeln!(fh, "fresh {} {} {}", fr.var, kind, l.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" "));
                }
            }
        }
        // (the carries out of column k join the top block)
        let Some(sk_lit) = sk_lit else {
            if dbg {
                eprintln!("structarith: side {}: nothing adds into the compared weight below the network", s);
            }
            return;
        };
        // the chain's bit must be a signal a cell produces: the lemma trades
        // the carry for it in the sums, which drop it from the outputs; a
        // fixed one alone at that column is a constant of the leaf side,
        // and the carry's value would be lost (review, 2026-10-21)
        if trues.contains(&sk_lit.abs()) {
            if dbg {
                eprintln!("structarith: side {}: the chain's bit at the compared weight is a fixed one", s);
            }
            return;
        }
        // the lemma's clauses: every gate of the side's cone over its
        // inputs (a full assignment of a gate's inputs propagates its
        // output, so each clause is a rup step), and the definitions of
        // the side's fresh variables. The network alone, or the cells it
        // reads, is not enough: a resynthesised network leans on facts
        // about the tree (signals that are never both one) that only the
        // tree's own gates give
        let fresh_of: HashMap<i32, usize> = fresh.iter().enumerate().map(|(i, f)| (f.var, i)).collect();
        let cone_s = if s == 0 { &cone_a } else { &cone_b };
        let mut gate_clauses: Vec<Vec<i32>> = Vec::new();
        let mut memo: HashMap<i32, u64> = HashMap::new();
        // in circuit order, the inputs' gates first: kitten decides the
        // variables it met last first, and the lemma is a fact about the
        // top of the tree (deciding primary inputs first just simulates
        // the tree, one assignment at a time)
        let mut in_cone: Vec<i32> = cone_s.iter().copied().collect();
        in_cone.sort_by_key(|&v| pos[v as usize]);
        if dbg {
            let trace: i32 = std::env::var("SAT_STRUCT_DEBUG_VIRT").ok().and_then(|t| t.parse().ok()).unwrap_or(0);
            if trace > 0 {
                eprintln!("structarith: side {} cone has {}: {}; in a or b: {} {}", s, trace, cone_s.contains(&trace), cone_a.contains(&trace), cone_b.contains(&trace));
            }
        }
        for &u in &in_cone {
            if stopped(*work) {
                return;
            }
            let Some(g) = net.gates[u as usize].as_ref() else { continue };
            let mut cut: Vec<i32> = g.sup.iter().map(|&x| net.rep[x as usize].0).collect();
            cut.sort_unstable();
            cut.dedup();
            let Some(tt) = cut_truth_table(u, &cut, &net, &mut memo, work) else {
                if dbg {
                    eprintln!("structarith: side {}: gate {} has no table over its inputs", s, u);
                }
                return;
            };
            gate_clauses.extend(minterm_clauses(u, &cut, tt));
        }
        // every cell of the side: its real outputs over its cut (a cell
        // may pair a gate from outside the cone, over shared partial
        // products; a product cell's outputs over the inputs under it)
        for &ci in &side_cells[s] {
            let c = &cells[ci];
            let over: &[i32] = c.split.as_deref().unwrap_or(&c.leaves);
            for out in [c.sum, c.carry_var()] {
                if fresh_of.contains_key(&out) || cone_s.contains(&out) {
                    continue;
                }
                let Some(tt) = cut_truth_table(out, over, &net, &mut memo, work) else {
                    if dbg {
                        eprintln!("structarith: side {}: cell output {} has no table over {:?}", s, out, over);
                    }
                    return;
                };
                gate_clauses.extend(minterm_clauses(out, over, tt));
            }
            if c.split.is_some() {
                for &leaf in &c.leaves {
                    if !cone_s.contains(&leaf) && (leaf as usize) < nv {
                        if let Some(g) = net.gates[leaf as usize].as_ref() {
                            let mut cut: Vec<i32> = g.sup.iter().map(|&x| net.rep[x as usize].0).collect();
                            cut.sort_unstable();
                            cut.dedup();
                            let Some(tt) = cut_truth_table(leaf, &cut, &net, &mut memo, work) else {
                                if dbg {
                                    eprintln!("structarith: side {}: product {} has no table over its inputs", s, leaf);
                                }
                                return;
                            };
                            gate_clauses.extend(minterm_clauses(leaf, &cut, tt));
                        }
                    }
                }
            }
        }
        // (in a fixed order: kitten numbers its variables as it meets them,
        // and its decisions follow, so the order must not change from run
        // to run for the work clock to be the same)
        let mut ones: Vec<i32> = trues.iter().copied().collect();
        ones.sort_unstable();
        // every fresh variable a cell of the side touches gets its
        // definition: the partners of lone cells, the chain cells, and the
        // fresh leaves too (a fused xor, a synthesized addend), or kitten
        // would be free to value them (review, 2026-10-22)
        let side_vars: HashSet<i32> = side_cells[s].iter().flat_map(|&ci| cells[ci].leaves.iter().copied().chain([cells[ci].sum, cells[ci].carry_var()])).collect();
        let lemma_clauses = |_cells: &[Cell], fresh: &[Fresh]| -> Vec<Vec<i32>> {
            let mut clauses = gate_clauses.clone();
            let mut defs: Vec<&Fresh> = fresh.iter().filter(|fr| side_vars.contains(&fr.var)).collect();
            defs.sort_by_key(|fr| fr.var);
            for fr in defs {
                clauses.extend(fresh_clauses(fr));
            }
            for &t in &ones {
                clauses.push(vec![t]);
            }
            clauses
        };
        let mut clauses = lemma_clauses(&cells, &fresh);
        *work += clauses.len() as u64;
        let mut n_lit = n_lit;
        if dbg {
            eprintln!("structarith: side {}: work {} before the lemma ({} clauses)", s, *work, clauses.len());
        }
        let lemma_budget = (*work).saturating_add(lemma_allowance).min(hard);
        let before_all = *work;
        // the clause set simulated on random inputs: the signatures say
        // whether the carry arrives plain or complemented (no failing
        // kitten run to find out), and which signals of the network equal
        // which bits of the chain on the way up
        let is_input_var = |v: i32| (v as usize) < nv && net.gates[v as usize].is_none() && !net.units[v as usize];
        let Some(sig) = simulate_clauses(&clauses, &is_input_var, 0x9E37_79B9_7F4A_7C15 ^ (s as u64 + 1), work, &stopped) else {
            if dbg {
                eprintln!("structarith: side {}: the limit ran out in the simulation", s);
            }
            return;
        };
        let sig_of = |l: i32, sig: &HashMap<i32, (u64, u64)>| -> Option<(u64, u64)> {
            let &(v, kn) = sig.get(&l.abs())?;
            Some((if l < 0 { !v & kn } else { v & kn }, kn))
        };
        let (Some((sn, kn_n)), Some((ssk, kn_sk))) = (sig_of(n_lit, &sig), sig_of(sk_lit, &sig)) else {
            if dbg {
                eprintln!("structarith: side {}: the simulation did not reach the carry or the chain's bit", s);
            }
            return;
        };
        let common = kn_n & kn_sk;
        let plain = (sn ^ ssk) & common == 0;
        let complemented = (sn ^ !ssk) & common == 0;
        let (plain, complemented) = if std::env::var_os("SAT_STRUCT_DEBUG_NOSIM").is_some() { (true, false) } else { (plain, complemented) }; // (debug: let kitten find the counterexample and dump it)
        if common == 0 || (!plain && !complemented) {
            if dbg {
                eprintln!("structarith: side {}: the simulation separates the network's carry {} from the chain's bit {} ({} patterns)", s, n_lit, sk_lit, common.count_ones());
                if std::env::var_os("SAT_STRUCT_DEBUG_LEMMA").is_some() {
                    let mut v: Vec<(&Vec<i32>, &i128)> = pre_poly[s].terms.iter().collect();
                    v.sort();
                    eprintln!("    side {} leaf polynomial at scale {}: {:?}", s, min_exp0, v);
                }
            }
            return;
        }
        if complemented && !plain {
            // the network hands over the carry complemented (a nor chain of
            // alternating polarity): then the output literal is the bit's
            // complement too, and the sums would be off by a half step. The
            // top cell is a lone sum of the pass's own, so it may read the
            // carry either way: read it complemented, and the sum is the
            // bit. The cell is changed first and the lemma proven over the
            // changed clauses, so the learned clauses hold under the
            // definitions the proof writes
            if !(reader == top && fresh_of.contains_key(&cells[top].carry_var())) {
                if dbg {
                    eprintln!("structarith: side {}: the carry {} arrives complemented, and the top cell is not the pass's own", s, n);
                }
                return;
            }
            let i = cells[top].leaves.iter().position(|&l| l == n).unwrap();
            let fi = fresh_of[&cells[top].carry_var()];
            let lits: Vec<i32> = {
                let c = &cells[top];
                (0..c.leaves.len()).map(|j| if j == i { -c.leaf_lit(j) } else { c.leaf_lit(j) }).collect()
            };
            // (the fresh carry's definition as the cell's carry literal
            // needs it: the majority of the leaves, the and of a half
            // adder's, the or of one read with a one; a carry literal read
            // negated is the complement, the majority of the complements
            // or the and of the complements. A top cell made before the
            // reading of its side was settled may be read so, bead skz.13)
            let (fa, one, carry_neg) = (cells[top].fa, cells[top].one, cells[top].carry < 0);
            let neg_lits: Vec<i32> = lits.iter().map(|&l| -l).collect();
            let new_def = match (fa, one, carry_neg) {
                (true, _, false) => FreshDef::Maj(lits),
                (true, _, true) => FreshDef::Maj(neg_lits),
                (false, false, false) => FreshDef::And(lits),
                (false, true, true) => FreshDef::And(neg_lits),
                _ => {
                    if dbg {
                        eprintln!("structarith: side {}: the top cell's carry cannot be read so", s);
                    }
                    return;
                }
            };
            {
                let c = &mut cells[top];
                c.negmask ^= 1 << i;
                c.sum_neg = !c.sum_neg;
                fresh[fi].def = new_def;
            }
            clauses = lemma_clauses(&cells, &fresh);
            *work += clauses.len() as u64;
            n_lit = -n_lit;
            if dbg {
                eprintln!("structarith: side {}: the carry {} arrives complemented; the top cell reads it so", s, n);
            }
        }
        // the ladder: the chain's carries from the bottom up, each proven
        // equal to a network signal with the same signature (or the
        // complement's) when there is one; the binary clauses of a proven
        // step join the clause set, so the next step is local. The top
        // step is the carry against the chain's bit
        let mut steps: Vec<(i32, i32, Vec<Vec<i32>>)> = Vec::new();
        {
            let chain_vars: HashSet<i32> = cells[cells.len() - chain_cells..].iter().flat_map(|c| [c.sum, c.carry_var()]).collect();
            let mut by_sig: HashMap<u64, Vec<i32>> = HashMap::new();
            *work += sig.len() as u64;
            for (&v, &(val, kn)) in &sig {
                if kn == u64::MAX && !chain_vars.contains(&v) && v != n_lit.abs() && v != sk_lit.abs() && !trues.contains(&v) {
                    by_sig.entry(val).or_default().push(v);
                }
            }
            for v in by_sig.values_mut() {
                v.sort_unstable();
            }
            // (the chain's carries in the order the chain made them, each
            // column's after the one before; a column whose two carries
            // match no network signal one by one may match by their xor:
            // under a network with one carry per column the chain's two
            // are never both one when the bottom gives it two, and their
            // xor is the network's carry, bead skz.13)
            let carries: Vec<i32> = cells[cells.len() - chain_cells..].iter().map(|c| c.carry_var()).collect();
            let mut proven = 0usize;
            let mut tried = 0usize;
            let mut col_carries: Vec<(i64, Vec<i32>)> = Vec::new();
            for (i, &kv) in carries.iter().enumerate() {
                match col_carries.last_mut() {
                    Some((e, v)) if *e == chain_col[i] => v.push(kv),
                    _ => col_carries.push((chain_col[i], vec![kv])),
                }
            }
            let pair_rungs = booth_side.values().any(|&t| t == s);
            for (_, col_kvs) in col_carries {
            let mut matched_here = false;
            for &kv in &col_kvs {
                let Some(&(val, kn)) = sig.get(&kv) else { continue };
                if kn != u64::MAX || val == 0 || val == u64::MAX {
                    continue; // (a constant signature matches everything and nothing)
                }
                let (m, lit_m) = match (by_sig.get(&val).and_then(|v| v.first()), by_sig.get(&!val).and_then(|v| v.first())) {
                    (Some(&m), _) => (m, m),
                    (None, Some(&m)) => (m, -m),
                    (None, None) => continue,
                };
                tried += 1;
                let before = *work;
                let core = kitten_equivalence(&clauses, kv, lit_m, lemma_budget, work, stop);
                extra.set(extra.get() + (*work - before));
                let Some(core) = core else {
                    if dbg {
                        eprintln!("structarith: side {}: the chain's carry {} is not the network's {} (or the budget ran out at {})", s, kv, lit_m, *work);
                    }
                    if *work >= lemma_budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    continue;
                };
                clauses.push(vec![-kv, lit_m]);
                clauses.push(vec![kv, -lit_m]);
                steps.push((kv, lit_m, core));
                proven += 1;
                matched_here = true;
                let _ = m;
            }
            if !pair_rungs || matched_here || col_kvs.len() != 2 {
                continue;
            }
            let (Some(&(v1, k1)), Some(&(v2, k2))) = (sig.get(&col_kvs[0]), sig.get(&col_kvs[1])) else { continue };
            if k1 != u64::MAX || k2 != u64::MAX {
                continue;
            }
            let val = v1 ^ v2;
            if val == 0 || val == u64::MAX {
                continue;
            }
            let lit_m = match (by_sig.get(&val).and_then(|v| v.first()), by_sig.get(&!val).and_then(|v| v.first())) {
                (Some(&m), _) => m,
                (None, Some(&m)) => -m,
                (None, None) => continue,
            };
            // the xor as a fresh variable of the proof, its definition in
            // the clause set
            let d = next_fresh;
            next_fresh += 1;
            let fr = Fresh { var: d, def: FreshDef::Xor(vec![col_kvs[0], col_kvs[1]]) };
            clauses.extend(fresh_clauses(&fr));
            fresh.push(fr);
            tried += 1;
            let before = *work;
            let core = kitten_equivalence(&clauses, d, lit_m, lemma_budget, work, stop);
            extra.set(extra.get() + (*work - before));
            let Some(core) = core else {
                if dbg {
                    eprintln!("structarith: side {}: the xor {} of the chain's carries {:?} is not the network's {} (or the budget ran out at {})", s, d, col_kvs, lit_m, *work);
                }
                if *work >= lemma_budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                continue;
            };
            clauses.push(vec![-d, lit_m]);
            clauses.push(vec![d, -lit_m]);
            steps.push((d, lit_m, core));
            proven += 1;
            }
            if dbg && tried > 0 {
                eprintln!("structarith: side {}: {} of {} chain carries matched a network signal and were proven equal, work {}", s, proven, tried, *work);
            }
        }
        let before = *work;
        let core = kitten_equivalence(&clauses, n_lit, sk_lit, lemma_budget, work, stop);
        extra.set(extra.get() + (*work - before));
        if dbg {
            eprintln!("structarith: side {}: work {} after the lemma ({} in all)", s, *work, *work - before_all);
        }
        let Some(core) = core else {
            if dbg {
                eprintln!("structarith: side {}: the network's carry {} is not the chain's bit {} ({} gate clauses, {} in all)", s, n_lit, sk_lit, gate_clauses.len(), clauses.len());
            }
            return;
        };
        if dbg {
            eprintln!("structarith: side {}: carry {} = chain bit {} by {} learned clauses after {} steps ({} gate clauses, {} chain cells, constant {} of modulus {})", s, n_lit, sk_lit, core.len(), steps.len(), gate_clauses.len(), chain_cells, constant, modulus);
        }
        steps.push((n_lit, sk_lit, core));
        lemmas[s] = Some(Lemma { n_lit, sk_lit, k, gate_clauses, steps });
    }
    phase("chains and lemmas", *work);
    derived.last_var = next_fresh - 1; // (also when the pass declines below: the numbers are spent)
    // the integer check: each side's cells summed with their weights; every
    // internal signal cancels; what is left must be the side's leaves (read
    // the same way, with the same weight, by the other side: shared partial
    // products) and the side's output bits, one literal per weight
    let mut exps: [Vec<i64>; 2] = [Vec::new(), Vec::new()];
    let mut roots: [i32; 2] = [0, 0];
    for s in 0..2 {
        let base = cells[side_cells[s][0]].sum;
        let (r, _) = uf.find(base);
        roots[s] = r;
        for &ci in &side_cells[s] {
            let (rc, o) = uf.find(cells[ci].sum);
            if rc != r {
                if dbg {
                    eprintln!("structarith: a side's cells are not one weighted network");
                }
                return;
            }
            exps[s].push(o);
        }
    }
    // the sides are one union-find: a shared leaf ties them to one root, so
    // their offsets are comparable when the roots agree
    if roots[0] != roots[1] {
        if dbg {
            eprintln!("structarith: the two sides share no weighted signal");
        }
        return;
    }
    let min_exp = exps.iter().flatten().copied().min().unwrap();
    let max_exp = exps.iter().flatten().copied().max().unwrap();
    if max_exp - min_exp + 2 > MAX_BITS as i64 {
        if dbg {
            eprintln!("structarith: {} weights, too many", max_exp - min_exp + 2);
        }
        return;
    }
    let weight_of = |ci: usize, cells: &[Cell], uf: &mut WeightedUf| -> i128 {
        let (_, o) = uf.find(cells[ci].sum);
        1i128 << (o - min_exp)
    };
    let mut forms: [Form; 2] = [Form::default(), Form::default()];
    let mut expected: [Form; 2] = [Form::default(), Form::default()];
    let mut bits: [HashMap<u32, Vec<i32>>; 2] = [HashMap::new(), HashMap::new()]; // weight -> output literals
    let mut leaf_form: [Form; 2] = [Form::default(), Form::default()]; // each side's leaves and constants
    // each side's leaves as a polynomial over the formula's inputs: a
    // product is a monomial of two inputs, a Booth row bit a polynomial of
    // five; the two sides must agree as polynomials (bead skz.4)
    let mut poly: [Poly; 2] = [Poly::default(), Poly::default()];
    for s in 0..2 {
        let produced: HashSet<i32> = side_cells[s].iter().flat_map(|&ci| [cells[ci].sum, cells[ci].carry_var()]).collect();
        let consumed: HashSet<i32> = side_cells[s].iter().flat_map(|&ci| cells[ci].leaves.iter().copied()).collect();
        for &ci in &side_cells[s] {
            let c = &cells[ci];
            let w = weight_of(ci, &cells, &mut uf);
            for i in 0..c.leaves.len() {
                let l = c.leaf_lit(i);
                forms[s].add_lit(l, w);
                if !produced.contains(&c.leaves[i]) {
                    expected[s].add_lit(l, w);
                    if trues.contains(&c.leaves[i]) {
                        // a fresh variable fixed to one (read negated, a
                        // complemented chain's, it is zero)
                        if l > 0 {
                            leaf_form[s].add_constant(w);
                            poly[s].add(Vec::new(), w);
                        }
                    } else if foreign[s] != Some(c.leaves[i]) {
                        leaf_form[s].add_lit(l, w); // (the network's carry is the chain's bit, below)
                        match input_leaves.get(&c.leaves[i]) {
                            Some(il) => poly[s].add_tt(&il.sup, il.tt, w, l < 0),
                            // (a leaf over more inputs than a table holds
                            // stays a variable of its own: both sides must
                            // then read it, which the leaf comparison asks)
                            None => poly[s].add_tt(&[c.leaves[i]], 0b10, w, l < 0),
                        }
                    }
                }
            }
            if c.one {
                forms[s].add_constant(w);
                expected[s].add_constant(w);
                leaf_form[s].add_constant(w);
                poly[s].add(Vec::new(), w);
            }
            // a signal this cell reads doubled: its reads add up to its
            // production plus the constant of the complemented read, at the
            // signal's own weight (half this cell's)
            for &l in &c.leaves {
                if doubled.get(&l) == Some(&ci) {
                    let hw = w / 2;
                    if hw == 0 {
                        return; // (the lowest weight: no half of it)
                    }
                    expected[s].add_constant(hw);
                    leaf_form[s].add_constant(hw);
                    poly[s].add(Vec::new(), hw);
                }
            }
            forms[s].add_lit(c.sum_lit(), -w);
            forms[s].add_lit(c.carry_lit(), -2 * w);
            for (lit, wt) in [(c.sum_lit(), w), (c.carry_lit(), 2 * w)] {
                if !consumed.contains(&lit.abs()) {
                    expected[s].add_lit(lit, -wt);
                    if wt & (wt - 1) != 0 {
                        if dbg {
                            eprintln!("structarith: an output weight that is not a power of two");
                        }
                        return;
                    }
                    let bit = wt.trailing_zeros();
                    bits[s].entry(bit).or_default().push(lit);
                }
            }
        }
        forms[s].prune();
        expected[s].prune();
        if forms[s].overflow || expected[s].overflow || leaf_form[s].overflow || poly[s].overflow {
            if dbg {
                eprintln!("structarith: a sum left the i128 range");
            }
            return;
        }
        if forms[s].coeff != expected[s].coeff || forms[s].constant != expected[s].constant {
            if dbg {
                let mut diff: Vec<(i32, i128, i128)> = forms[s]
                    .coeff
                    .iter()
                    .map(|(&v, &c)| (v, c, expected[s].coeff.get(&v).copied().unwrap_or(0)))
                    .filter(|&(_, c, e)| c != e)
                    .collect();
                diff.extend(expected[s].coeff.iter().filter(|(v, _)| !forms[s].coeff.contains_key(v)).map(|(&v, &e)| (v, 0, e)));
                diff.sort_unstable();
                eprintln!(
                    "structarith: side {}'s internal signals do not cancel: {} signals, constants {} v {}, for example {:?}",
                    s,
                    diff.len(),
                    forms[s].constant,
                    expected[s].constant,
                    &diff[..diff.len().min(6)]
                );
                for &(v, _, _) in diff.iter().take(3) {
                    for &ci in &side_cells[s] {
                        let c = &cells[ci];
                        if c.sum == v || c.carry == v || c.leaves.contains(&v) {
                            eprintln!("    cell {:?} leaves {:?} negmask {} sum {} sum_neg {} carry {} one {} fa {} weight {}", ci, c.leaves, c.negmask, c.sum, c.sum_neg, c.carry, c.one, c.fa, weight_of(ci, &cells, &mut uf));
                        }
                    }
                }
            }
            return;
        }
        *work += side_cells[s].len() as u64 * 4;
    }
    // the two sides' outputs are equal only when what they sum is equal:
    // the same leaves with the same total weight (a leaf read twice counts
    // twice) and the same constant (a constant-one cell on one side only
    // makes the miter satisfiable). Compared as linear forms over
    // variables, so a leaf read negated on one side and plain on the other
    // is handled too; without a proof file nothing else catches a mismatch.
    poly[0].prune();
    poly[1].prune();
    *work += (poly[0].terms.len() + poly[1].terms.len()) as u64;
    // the comparison as polynomials comes after the output bits below,
    // which set the modulus the two may differ by. Until the proof can
    // derive one side's leaves from the other's (the group derivations of
    // bead SAT-playground-skz.4), the leaves must also be the same
    // variables with the same coefficients: the proof's sums cancel
    // variables, not functions (two nand gates over one pair of products
    // are one function but two variables, and the sums would keep both)
    leaf_form[0].prune();
    leaf_form[1].prune();
    *work += (leaf_form[0].coeff.len() + leaf_form[1].coeff.len()) as u64;
    let diagnose_poly = std::env::var_os("SAT_STRUCT_DEBUG_POLY").is_some(); // (debug: print the polynomial difference instead of the leaf check)
    // the leaves the two sides read differently: a Booth row bit against
    // products, two gates for one product. The proof expands each such
    // leaf into its polynomial over the inputs (bead SAT-playground-skz.4,
    // "the leaf identities" below), so each must be an input leaf; the
    // leaves both sides read alike cancel as variables, as before
    let mut differing: Vec<i32> = leaf_form[0]
        .coeff
        .iter()
        .filter(|(v, &c)| leaf_form[1].coeff.get(*v).copied().unwrap_or(0) != c)
        .map(|(&v, _)| v)
        .chain(leaf_form[1].coeff.keys().filter(|v| !leaf_form[0].coeff.contains_key(*v)).copied())
        .collect();
    differing.sort_unstable();
    differing.dedup();
    *work += differing.len() as u64;
    if let Some(&v) = differing.iter().find(|&&v| !input_leaves.contains_key(&v) || (!(v as usize) < nv && false)) {
        if dbg && !diagnose && !diagnose_poly {
            let diff: Vec<(i32, i128, i128)> = differing.iter().map(|&v| (v, leaf_form[0].coeff.get(&v).copied().unwrap_or(0), leaf_form[1].coeff.get(&v).copied().unwrap_or(0))).collect();
            eprintln!("structarith: the two sides sum different leaves, and {} is no input leaf: {} variables differ, for example {:?}", v, diff.len(), &diff[..diff.len().min(4)]);
        }
        if !diagnose && !diagnose_poly {
            return;
        }
    }
    let shared_pp = poly[0].terms.keys().filter(|m| m.len() == 2).count();
    // the chain's bit at the compared weight is the network's carry, which
    // the top cell reads: the two cancel, and the output stays alone there
    for s in 0..2 {
        if let Some(lm) = lemmas[s].as_ref() {
            // (one occurrence, at the lemma's weight: that is what the
            // lemma trades the carry for)
            let w = (lm.k - min_exp) as u32; // (the bits are keyed from the lowest weight)
            let Some(pos) = bits[s].get(&w).and_then(|v| v.iter().position(|&l| l == lm.sk_lit)) else {
                if dbg {
                    eprintln!("structarith: side {}: the chain's bit is not an output at weight {}", s, lm.k);
                }
                return;
            };
            let v = bits[s].get_mut(&w).unwrap();
            v.remove(pos);
            if v.is_empty() {
                bits[s].remove(&w);
            }
        }
    }
    // the bits from the top: above K the carries that left the cone (any
    // number of them, at any weights) form one block that is proven equal
    // on both sides and cancelled; at and below K every weight has at most
    // one output literal per side and the bit argument runs
    let mut multi: Vec<u32> = bits.iter().flat_map(|b| b.iter().filter(|(_, v)| v.len() > 1).map(|(&w, _)| w)).collect();
    // (the weights are powers of two of an i128: at most MAX_BITS of them)
    multi.sort_unstable();
    let top_k: i64 = multi.first().map_or(i64::MAX, |&w| w as i64 - 1);
    let single: Vec<u32> = {
        let mut v: Vec<u32> = bits.iter().flat_map(|b| b.keys().copied()).filter(|&w| (w as i64) <= top_k).collect();
        v.sort_unstable();
        v.dedup();
        v.reverse();
        v
    };
    let block: Vec<(u8, i32, u32)> = bits
        .iter()
        .enumerate()
        .flat_map(|(s, b)| b.iter().filter(|(&w, _)| (w as i64) > top_k).flat_map(move |(&w, v)| v.iter().map(move |&l| (s as u8, l, w))))
        .collect();
    if single.is_empty() {
        if dbg {
            eprintln!("structarith: no weight with one output per side");
        }
        return;
    }
    // the two sides' outputs are equal only when what they sum is equal:
    // the same polynomial over the inputs (a leaf read twice counts twice,
    // a leaf read negated is 1 - its function), up to multiples of
    // 2^(K+1) in any coefficient, which the top block absorbs (the
    // constants of carry-in-one cells and the chains' fixed ones; the
    // products a pruned cone lacks above the compared bit; a Booth
    // correction row's products at twice their weight, which the top
    // digit's negative term makes up for). Without a proof file nothing
    // else catches a mismatch
    let mut mono_lit: HashMap<Vec<i32>, i32> = HashMap::new(); // monomial (sorted inputs, 2 or more) -> literal
    {
        // (in a fixed order: two gates for one product must not be picked
        // by hash order, or the proof would differ from run to run)
        let mut prods: Vec<((i32, i32), i32)> = pps.iter().map(|(&k, &v)| (k, v)).collect();
        prods.sort_unstable();
        for ((x, y), v) in prods {
            let mut m = vec![x, y];
            m.sort_unstable();
            mono_lit.entry(m).or_insert(v);
        }
    }
    // a leaf that is one monomial, plain or complemented, may stand for it
    {
        let mut by_var: Vec<(&i32, &InputLeaf)> = input_leaves.iter().collect();
        by_var.sort_by_key(|(v, _)| **v);
        for (&v, il) in by_var {
            if il.sup.len() < 2 {
                continue;
            }
            let lit = match monomial_reading(il.tt, il.sup.len()) {
                Some(false) => v,
                Some(true) => -v,
                None => continue,
            };
            let mut m = il.sup.clone();
            m.sort_unstable();
            mono_lit.entry(m).or_insert(lit);
        }
    }
    // the literal standing for a monomial of two or more inputs: the grid's
    // product, a leaf that is that monomial, else a fresh and of the inputs
    fn designate(mono_lit: &mut HashMap<Vec<i32>, i32>, fresh: &mut Vec<Fresh>, next_fresh: &mut i32, mono: &[i32]) -> i32 {
        if let Some(&l) = mono_lit.get(mono) {
            return l;
        }
        let g = *next_fresh;
        *next_fresh += 1;
        fresh.push(Fresh { var: g, def: FreshDef::And(mono.to_vec()) });
        mono_lit.insert(mono.to_vec(), g);
        g
    }
    let modulus: i128 = if top_k >= MAX_BITS as i64 { 0 } else { 1i128 << (top_k + 1) };
    // the constant the two sides differ by, modulo the modulus: zero, or
    // exactly the top single weight, which says the two output bits there
    // are complements as read. A lone top cell (an xor with a fresh carry)
    // reads either way: x + y = s + 2 (x and y), or x + y + 1 = ~s + 2 (x
    // or y); the second reading adds the top weight to that side's sum and
    // compares the complement, and the bits then agree (a Booth side's
    // sign-extension constants against an AND side: bead skz.4)
    let mut reread: Option<usize> = None;
    let mut residuals: Vec<(usize, u32, Vec<i32>)> = Vec::new(); // (the side without the output bit, its weight, the monomial that is that bit)
    {
        let mut monos: Vec<&Vec<i32>> = poly[0].terms.keys().chain(poly[1].terms.keys()).collect();
        monos.sort();
        monos.dedup();
        let mut off: Vec<(Vec<i32>, i128, i128)> = Vec::new();
        for m in monos {
            let a = poly[0].terms.get(m).copied().unwrap_or(0);
            let b = poly[1].terms.get(m).copied().unwrap_or(0);
            let Some(diff) = a.checked_sub(b) else {
                if dbg {
                    eprintln!("structarith: a polynomial difference left the i128 range");
                }
                return;
            };
            // a coefficient differing by a multiple of 2^(K+1) is carried
            // by the proof's block step (the products a pruned cone lacks
            // above the compared bit; a Booth row's bits above it)
            if diff != 0 && (modulus == 0 || diff % modulus != 0) {
                // a monomial one side's cells never read, at the weight of
                // a bit the other side's cells produce and this side's do
                // not: that bit of the circuit is the monomial itself (an
                // array multiplier's bit 0 is a_0 b_0, no cell's), and the
                // monomial's literal stands as this side's output there
                if !m.is_empty() && diff.unsigned_abs().is_power_of_two() {
                    let w = diff.unsigned_abs().trailing_zeros();
                    let lacking = if diff < 0 { 0 } else { 1 };
                    let have = bits[1 - lacking].get(&w).map_or(0, |v| v.len());
                    let lack = bits[lacking].get(&w).map_or(0, |v| v.len());
                    if (w as i64) <= top_k && have == 1 && lack == 0 && !residuals.iter().any(|&(s2, w2, _)| (s2, w2) == (lacking, w)) {
                        residuals.push((lacking, w, m.clone()));
                        continue;
                    }
                }
                if m.is_empty() && modulus != 0 && diff.rem_euclid(modulus) == modulus / 2 {
                    // the top bits are complements as read: side B's top
                    // cell re-read with a one (side A's when B's is not
                    // the pass's own)
                    let top_w = top_k as u32;
                    let lone_top = |s: usize| -> Option<usize> {
                        let o = if s == 0 { side_a } else { side_b };
                        let ci = side_cells[s].iter().copied().find(|&ci| cells[ci].sum == o && cells[ci].carry_var() > f.vars && cells[ci].leaves.len() == 2 && !cells[ci].one)?;
                        (bits[s].get(&top_w).map_or(false, |v| v.len() == 1 && v[0].abs() == o)).then_some(ci)
                    };
                    reread = lone_top(1).or_else(|| lone_top(0));
                    if reread.is_none() {
                        off.push((m.clone(), a, b));
                    }
                } else {
                    off.push((m.clone(), a, b));
                }
            }
        }
        *work += poly[0].terms.len() as u64 + poly[1].terms.len() as u64;
        if !off.is_empty() || diagnose || diagnose_poly {
            if diagnose || diagnose_poly {
                let mut all: Vec<(Vec<i32>, i128)> = Vec::new();
                for m in poly[0].terms.keys().chain(poly[1].terms.keys()) {
                    let a = poly[0].terms.get(m).copied().unwrap_or(0);
                    let b = poly[1].terms.get(m).copied().unwrap_or(0);
                    let d = a - b;
                    if d != 0 && !all.iter().any(|(mm, _)| mm == m) {
                        all.push((m.clone(), d));
                    }
                }
                all.sort();
                for s in 0..2 {
                    let mut v: Vec<(&Vec<i32>, &i128)> = poly[s].terms.iter().collect();
                    v.sort();
                    let degs: Vec<usize> = (0..6).map(|d| v.iter().filter(|(m, _)| m.len() == d).count()).collect();
                    eprintln!("structarith: diagnosis: side {} polynomial: {} monomials, by degree {:?}; some: {:?}", s, v.len(), degs, &v[..v.len().min(24)]);
                }
                eprintln!("structarith: diagnosis: compared signals at {:?} and {:?}; tops at {:?}", uf.find(side_a), uf.find(side_b), bits.iter().map(|b| b.keys().max().copied()).collect::<Vec<_>>());
                for s in 0..2 {
                    let mut low: Vec<(i64, String)> = side_cells[s]
                        .iter()
                        .map(|&ci| {
                            let c = &cells[ci];
                            let (_, e) = uf.find(c.sum);
                            let lits: Vec<String> = (0..c.leaves.len()).map(|i| {
                                let l = c.leaf_lit(i);
                                match input_leaves.get(&c.leaves[i]) { Some(il) => format!("{}{{{:?}}}", l, il.sup), None => l.to_string() }
                            }).collect();
                            (e, format!("[{}] (+{}) = {} + 2*{}", lits.join(", "), c.one as u8, c.sum_lit(), c.carry_lit()))
                        })
                        .collect();
                    low.sort();
                    let range: Option<(i64, i64)> = std::env::var("SAT_STRUCT_DEBUG_RANGE").ok().and_then(|r| { let v: Vec<i64> = r.split(',').filter_map(|t| t.parse().ok()).collect(); (v.len() == 2).then(|| (v[0], v[1])) });
                    for (e, t) in low.iter().filter(|(e, _)| range.map_or(true, |(lo, hi)| *e >= lo && *e <= hi)).take(if range.is_some() { 400 } else { 6 }) {
                        eprintln!("    side {} cell at {}: {}", s, e, t);
                    }
                }
                eprintln!("structarith: diagnosis: A - B (modulus {}) has {} monomials; exponent of the lowest cell {}:", modulus, all.len(), min_exp);
                for (m, d) in &all {
                    let (sign, mag) = if *d < 0 { ("-", -*d) } else { ("+", *d) };
                    let pow = if mag.count_ones() == 1 { format!("2^{}", mag.trailing_zeros()) } else { format!("{}", mag) };
                    eprintln!("    {:?}: {}{}", m, sign, pow);
                }
                return;
            }
            if dbg {
                eprintln!(
                    "structarith: the two sides sum different polynomials: {} monomials differ (modulus {}), for example {:?}; constants {} and {}",
                    off.len(),
                    modulus,
                    &off[..off.len().min(4)],
                    poly[0].terms.get(&Vec::new()).copied().unwrap_or(0),
                    poly[1].terms.get(&Vec::new()).copied().unwrap_or(0)
                );
                for s in 0..2 {
                    let degs: Vec<(usize, usize)> = {
                        let mut d: HashMap<usize, usize> = HashMap::new();
                        for m in poly[s].terms.keys() {
                            *d.entry(m.len()).or_default() += 1;
                        }
                        let mut v: Vec<(usize, usize)> = d.into_iter().collect();
                        v.sort();
                        v
                    };
                    eprintln!("    side {}: {} monomials by degree {:?}", s, poly[s].terms.len(), degs);
                }
            }
            return;
        }
    }
    if dbg {
        eprintln!(
            "structarith: side A {} cells, side B {} cells ({} with a constant one), {} shared partial products, bits A {} B {}, {} single weights, {} signals in the top block",
            side_cells[0].len(),
            side_cells[1].len(),
            side_cells[0].iter().chain(side_cells[1].iter()).filter(|&&ci| cells[ci].one).count(),
            shared_pp,
            bits[0].len(),
            bits[1].len(),
            single.len(),
            block.len()
        );
    }
    if let Some(ci) = reread {
        // x + y + 1 = ~s + 2 (x or y), the or as the complement of a fresh
        // and of the complements; the old carry keeps its definition (the
        // lemma's learned clauses may lean on it), the new one takes its
        // place in the top block
        let s = if side_cells[1].contains(&ci) { 1 } else { 0 };
        let x = cells[ci].leaf_lit(0);
        let y = cells[ci].leaf_lit(1);
        let g = next_fresh;
        next_fresh += 1;
        derived.last_var = next_fresh - 1;
        fresh.push(Fresh { var: g, def: FreshDef::And(vec![-x, -y]) });
        uf.reserve(g);
        let old_carry = cells[ci].carry_lit();
        let (_, e_ci) = uf.find(cells[ci].sum);
        if !uf.union(g, cells[ci].sum, 1) {
            return;
        }
        let top_w = (e_ci - min_exp) as u32;
        {
            let c = &mut cells[ci];
            c.one = true;
            c.sum_neg = !c.sum_neg;
            c.carry = -g;
        }
        let new_sum = cells[ci].sum_lit();
        if let Some(v) = bits[s].get_mut(&top_w) {
            for l in v.iter_mut() {
                if l.abs() == new_sum.abs() {
                    *l = new_sum;
                }
            }
        }
        if let Some(v) = bits[s].get_mut(&(top_w + 1)) {
            for l in v.iter_mut() {
                if *l == old_carry {
                    *l = -g;
                }
            }
        }
        leaf_form[s].add_constant(1i128 << top_w);
        poly[s].add(Vec::new(), 1i128 << top_w);
        *work += 8;
        if dbg {
            eprintln!("structarith: side {}: the top bits are complements as read; the top cell re-read with a one, its output {}", s, new_sum);
        }
    }
    for (s, w, m) in &residuals {
        let lit = designate(&mut mono_lit, &mut fresh, &mut next_fresh, m);
        bits[*s].entry(*w).or_default().push(lit);
        *work += 4;
        if dbg {
            eprintln!("structarith: side {}: its bit at weight {} is the monomial {:?} itself (literal {})", s, w, m, lit);
        }
    }
    derived.last_var = next_fresh - 1;
    // the leaf identities: a leaf the two sides read differently is
    // expanded into its polynomial over the inputs, each monomial of two or
    // more inputs standing for one literal (the grid's product when there
    // is one, else a fresh and of the inputs), so the two sides' sums
    // cancel monomial by monomial in the proof as they do as polynomials
    // (a Booth row bit against the products it stands for; two gates for
    // one product). The leaves both sides read alike stay variables
    struct LeafIdentity {
        var: i32,
        sup: Vec<i32>,
        terms: Vec<(i128, i32)>, // the monomials' literals with their coefficients
        constant: i128,
    }
    let mut identities: Vec<LeafIdentity> = Vec::new();
    for &v in &differing {
        let il = &input_leaves[&v];
        let mut m_sorted = il.sup.clone();
        m_sorted.sort_unstable();
        if mono_lit.get(&m_sorted).map_or(false, |&l| l.abs() == v) {
            continue; // the literal every side's sums use for its monomial
        }
        if il.sup.len() == 1 && il.tt == 0b10 && il.sup[0] == v {
            continue; // a bare input: a variable of the sums as it is
        }
        let mut terms: Vec<(i128, i32)> = Vec::new();
        let mut constant = 0i128;
        for (mask, coef) in moebius(il.tt, il.sup.len()) {
            let mut mono: Vec<i32> = (0..il.sup.len()).filter(|&i| (mask >> i) & 1 == 1).map(|i| il.sup[i]).collect();
            if mono.is_empty() {
                constant += coef;
                continue;
            }
            mono.sort_unstable();
            let lit = if mono.len() == 1 { mono[0] } else { designate(&mut mono_lit, &mut fresh, &mut next_fresh, &mono) };
            terms.push((coef, lit));
        }
        *work += (1u64 << il.sup.len()) + terms.len() as u64;
        identities.push(LeafIdentity { var: v, sup: il.sup.clone(), terms, constant });
    }
    derived.last_var = next_fresh - 1;
    if dbg && !identities.is_empty() {
        eprintln!("structarith: {} leaf identities over {} monomial literals ({} fresh ands)", identities.len(), mono_lit.len(), fresh.iter().filter(|fr| matches!(fr.def, FreshDef::And(_)) && mono_lit.values().any(|&l| l.abs() == fr.var)).count());
    }
    // the sums as the proof adds them up, variable by variable: every leaf
    // its identity's expansion or itself; the two sides must agree on
    // every variable modulo 2^(K+1) (the polynomial check above agrees on
    // functions; this is the check on the literals the proof cancels)
    {
        let mut lit_forms: [HashMap<i32, i128>; 2] = [HashMap::new(), HashMap::new()];
        let by_var: HashMap<i32, &LeafIdentity> = identities.iter().map(|li| (li.var, li)).collect();
        for s in 0..2 {
            for (&v, &kappa) in &leaf_form[s].coeff {
                match by_var.get(&v) {
                    Some(li) => {
                        for &(c, l) in &li.terms {
                            let e = lit_forms[s].entry(l.abs()).or_default();
                            *e = e.saturating_add(if l < 0 { -c } else { c }.saturating_mul(kappa));
                        }
                    }
                    None => {
                        let e = lit_forms[s].entry(v).or_default();
                        *e = e.saturating_add(kappa);
                    }
                }
            }
            *work += leaf_form[s].coeff.len() as u64;
        }
        // (a bit that is a monomial itself: the other side's leaves hold
        // the monomial, this side's sums hold it as the bit; balanced here)
        for (s, w, m) in &residuals {
            let lit = mono_lit[m];
            let e = lit_forms[*s].entry(lit.abs()).or_default();
            *e = e.saturating_add(if lit < 0 { -(1i128 << *w) } else { 1i128 << *w });
        }
        let mut off: Vec<(i32, i128, i128)> = Vec::new();
        let mut vars: Vec<i32> = lit_forms[0].keys().chain(lit_forms[1].keys()).copied().collect();
        vars.sort_unstable();
        vars.dedup();
        for v in vars {
            let a = lit_forms[0].get(&v).copied().unwrap_or(0);
            let b = lit_forms[1].get(&v).copied().unwrap_or(0);
            if a != b && (modulus == 0 || (a - b) % modulus != 0) {
                off.push((v, a, b));
            }
        }
        if !off.is_empty() {
            if dbg {
                eprintln!("structarith: the two sides' sums differ on {} literals after the leaf identities, for example {:?}", off.len(), &off[..off.len().min(6)]);
            }
            return;
        }
    }
    let bit_of = |s: usize, w: u32| -> Option<i32> { bits[s].get(&w).and_then(|v| v.first().copied()) };
    // the proof (nothing is derived without one when a proof file is asked
    // for; without a proof file the integer check above is the whole check)
    let mut equalities: Vec<(Vec<i32>, u64)> = Vec::new();
    let weights_all: Vec<u32> = single.clone();
    if let Some(w) = derived.proof.as_mut() {
        // 0. the merged signals the cones read: their equivalence to the
        //    representative, each direction by contradiction with a case
        //    split over the merged gate's inputs (under a full assignment of
        //    the inputs both definitions propagate and the functions agree;
        //    unit propagation alone cannot do it for an xor)
        for &v in &net.order {
            let (r, neg) = net.rep[v as usize];
            if r == v || !(cone_a.contains(&r) || cone_b.contains(&r)) {
                continue;
            }
            let rl = if neg { -r } else { r };
            let inputs = &net.gates[v as usize].as_ref().unwrap().sup;
            let tree = case_split(inputs);
            if w.pbc(&[(1, -v), (1, rl)], 1, &tree).is_err() || w.pbc(&[(1, v), (1, -rl)], 1, &tree).is_err() {
                return;
            }
            *work += 2 * tree.len() as u64;
            if stopped(*work) {
                return;
            }
        }
        // 0b. the fresh variables: the ones fixed to one by a unit first
        //     (redundant under the witness that sets them, while nothing
        //     else mentions them), then each of the others defined by its
        //     clauses the same way
        let mut true_ids: HashMap<i32, u64> = HashMap::new();
        let mut ordered: Vec<i32> = trues.iter().copied().collect();
        ordered.sort_unstable();
        for t in ordered {
            match w.red_const(&[t], t, true) {
                Ok(id) => {
                    true_ids.insert(t, id);
                }
                Err(_) => return,
            }
        }
        for fr in &fresh {
            for clause in fresh_clauses(fr) {
                let value = clause.contains(&fr.var);
                if w.red_const(&clause, fr.var, value).is_err() {
                    return;
                }
            }
            *work += 1 << 3;
            if stopped(*work) {
                return;
            }
        }
        // 1. per cell the two inequalities, by contradiction with a case split
        let mut ge_ids: [Vec<(u64, i128)>; 2] = [Vec::new(), Vec::new()];
        let mut le_ids: [Vec<(u64, i128)>; 2] = [Vec::new(), Vec::new()];
        for s in 0..2 {
            for &ci in &side_cells[s] {
                let c = &cells[ci];
                let wt = weight_of(ci, &cells, &mut uf);
                let leaves: Vec<i32> = (0..c.leaves.len()).map(|i| c.leaf_lit(i)).collect();
                let sl = c.sum_lit();
                let cv = c.carry_lit();
                let k = leaves.len() as i64;
                // ge: leaves (+ 1) - s - 2c >= 0, in literal form (-s is
                // ~s - 1, -2c is 2 ~c - 2): leaves + ~s + 2 ~c >= 3 (- 1)
                let mut ge_terms: Vec<(i64, i32)> = leaves.iter().map(|&l| (1, l)).collect();
                ge_terms.push((1, -sl));
                ge_terms.push((2, -cv));
                let ge_rhs = 3 - if c.one { 1 } else { 0 };
                // le: s + 2c - leaves (- 1) >= 0 (each -l is ~l - 1):
                // ~leaves + s + 2 c >= k (+ 1)
                let mut le_terms: Vec<(i64, i32)> = leaves.iter().map(|&l| (1, -l)).collect();
                le_terms.push((1, sl));
                le_terms.push((2, cv));
                let le_rhs = k + if c.one { 1 } else { 0 };
                // both sides by contradiction with a full case split over
                // the cell's inputs: under a full assignment every gate of
                // the cell's cone propagates its output (the definitions do
                // that whatever the encoding), and the negated inequality is
                // then false; a shorter split relying on backward propagation
                // fails on some encodings with the right truth table
                let over: &[i32] = c.split.as_deref().unwrap_or(&c.leaves);
                let split = case_split(over);
                let (ge_sub, le_sub) = (split.clone(), split);
                let ge = match w.pbc(&ge_terms, ge_rhs, &ge_sub) {
                    Ok(id) => id,
                    Err(_) => return,
                };
                let le = match w.pbc(&le_terms, le_rhs, &le_sub) {
                    Ok(id) => id,
                    Err(_) => return,
                };
                ge_ids[s].push((ge, wt));
                le_ids[s].push((le, wt));
                *work += 2 * split_len(over) as u64 + 8;
                if stopped(*work) {
                    return; // the lines written so far derive nothing the formula keeps
                }
            }
        }
        // 1a. the leaf identities, each direction by contradiction with a
        //     full case split over the leaf's inputs (every monomial's
        //     literal and the leaf itself propagate from a full assignment)
        let mut id_ge: HashMap<i32, u64> = HashMap::new(); // expansion - leaf >= -constant
        let mut id_le: HashMap<i32, u64> = HashMap::new(); // leaf - expansion >= constant
        for li in &identities {
            // in literal form: a negative coefficient c on l is |c| on ~l
            // with |c| more on the right-hand side
            let literal_form = |terms: &[(i128, i32)], rhs: i128| -> (Vec<(i64, i32)>, i64) {
                let mut out: Vec<(i64, i32)> = Vec::new();
                let mut r = rhs;
                for &(c, l) in terms {
                    if c > 0 {
                        out.push((c as i64, l));
                    } else if c < 0 {
                        out.push(((-c) as i64, -l));
                        r += -c;
                    }
                }
                (out, r as i64)
            };
            let mut ge_terms: Vec<(i128, i32)> = li.terms.clone();
            ge_terms.push((-1, li.var));
            let mut le_terms: Vec<(i128, i32)> = li.terms.iter().map(|&(c, l)| (-c, l)).collect();
            le_terms.push((1, li.var));
            let (ge_t, ge_r) = literal_form(&ge_terms, -li.constant);
            let (le_t, le_r) = literal_form(&le_terms, li.constant);
            let split = case_split(&li.sup);
            let Ok(ge) = w.pbc(&ge_t, ge_r, &split) else { return };
            let Ok(le) = w.pbc(&le_t, le_r, &split) else { return };
            id_ge.insert(li.var, ge);
            id_le.insert(li.var, le);
            *work += 2 * split_len(&li.sup) as u64 + 8;
            if stopped(*work) {
                return;
            }
        }
        // 1b. the lemmas: the network's gates and the cells it reads, each
        //     restated over its cut (a full assignment of the cut propagates
        //     through the definitions), then kitten's learned clauses in
        //     order, then the equivalence's two clauses; these cancel the
        //     carry against the chain's bit in the sums
        for s in 0..2 {
            let Some(lm) = lemmas[s].as_ref() else { continue };
            for clause in &lm.gate_clauses {
                if w.rup(clause).is_err() {
                    return;
                }
            }
            *work += lm.gate_clauses.len() as u64 * 2;
            // each step: its learned clauses, then the equivalence's two
            // clauses (the steps before the last are the ladder's rungs)
            for (i, (a, b, core)) in lm.steps.iter().enumerate() {
                for clause in core {
                    if w.rup(clause).is_err() {
                        return;
                    }
                }
                *work += core.len() as u64 * 2;
                if i + 1 < lm.steps.len() {
                    if w.rup(&[-a, *b]).is_err() || w.rup(&[*a, -b]).is_err() {
                        return;
                    }
                }
                if stopped(*work) {
                    return;
                }
            }
            let Ok(ge) = w.rup(&[-lm.n_lit, lm.sk_lit]) else { return }; // bit - carry >= 0
            let Ok(le) = w.rup(&[lm.n_lit, -lm.sk_lit]) else { return }; // carry - bit >= 0
            let (_, k) = uf.find(lm.sk_lit.abs());
            let wt = 1i128 << (k - min_exp);
            ge_ids[s].push((ge, wt));
            le_ids[s].push((le, wt));
        }
        // 1c. the fixed ones: a leaf t of weight w in the sums is the
        //     constant w, by t <= 1 (the literal axiom ~t >= 0) one way and
        //     the unit t >= 1 the other; a leaf ~t (a complemented chain's)
        //     is zero, the unit one way and the axiom the other
        let mut true_terms: [String; 2] = [String::new(), String::new()]; // (axioms) for the ge sum
        let mut true_units: [Vec<(u64, i128)>; 2] = [Vec::new(), Vec::new()]; // (units) for the le sum
        let mut false_terms: [String; 2] = [String::new(), String::new()]; // (axioms) for the le sum
        let mut false_units: [Vec<(u64, i128)>; 2] = [Vec::new(), Vec::new()]; // (units) for the ge sum
        for s in 0..2 {
            let mut by_true: HashMap<i32, (i128, i128)> = HashMap::new();
            for &ci in &side_cells[s] {
                let c = &cells[ci];
                let wt = weight_of(ci, &cells, &mut uf);
                for (i, &l) in c.leaves.iter().enumerate() {
                    if trues.contains(&l) {
                        let e = by_true.entry(l).or_default();
                        if c.leaf_lit(i) > 0 {
                            e.0 += wt;
                        } else {
                            e.1 += wt;
                        }
                    }
                }
            }
            let mut by_true: Vec<(i32, (i128, i128))> = by_true.into_iter().collect();
            by_true.sort_unstable();
            for (t, (wp, wn)) in by_true {
                if wp > 0 {
                    true_terms[s].push_str(&format!(" ~x{} {} * +", t, wp));
                    true_units[s].push((true_ids[&t], wp));
                }
                if wn > 0 {
                    false_terms[s].push_str(&format!(" ~x{} {} * +", t, wn));
                    false_units[s].push((true_ids[&t], wn));
                }
            }
        }
        // 2. the telescoping sums, one per side and direction
        let weighted = |w: &mut crate::structproof::Writer, ids: &[(u64, i128)]| -> Result<u64, ()> {
            let mut expr = String::new();
            for (i, &(id, wt)) in ids.iter().enumerate() {
                if i > 0 {
                    expr.push(' ');
                }
                expr.push_str(&format!("{} {} *", id, wt));
                if i > 0 {
                    expr.push_str(" +");
                }
            }
            w.pol(&expr).map_err(|_| ())
        };
        // the leaf identities join the sums with each leaf's net weight
        // on the side: a leaf read plain at weight w adds w (expansion -
        // leaf) to the ge sum and w (leaf - expansion) to the le sum, a
        // leaf read negated the other way round
        for s in 0..2 {
            let mut vars: Vec<i32> = identities.iter().map(|li| li.var).filter(|v| leaf_form[s].coeff.contains_key(v)).collect();
            vars.sort_unstable();
            for v in vars {
                let kappa = leaf_form[s].coeff[&v];
                if kappa == 0 {
                    continue;
                }
                let (for_ge, for_le) = if kappa > 0 { (id_ge[&v], id_le[&v]) } else { (id_le[&v], id_ge[&v]) };
                ge_ids[s].push((for_ge, kappa.abs()));
                le_ids[s].push((for_le, kappa.abs()));
            }
        }
        let Ok(mut ge_a) = weighted(w, &ge_ids[0]) else { return };
        let Ok(mut le_a) = weighted(w, &le_ids[0]) else { return };
        let Ok(mut ge_b) = weighted(w, &ge_ids[1]) else { return };
        let Ok(mut le_b) = weighted(w, &le_ids[1]) else { return };
        for s in 0..2 {
            if true_units[s].is_empty() && false_units[s].is_empty() {
                continue;
            }
            let (ge, le) = if s == 0 { (&mut ge_a, &mut le_a) } else { (&mut ge_b, &mut le_b) };
            let mut gexpr = format!("{}{}", ge, true_terms[s]);
            for &(id, wt) in &false_units[s] {
                gexpr.push_str(&format!(" {} {} * +", id, wt));
            }
            let Ok(g2) = w.pol(&gexpr) else { return };
            let mut expr = format!("{}{}", le, false_terms[s]);
            for &(id, wt) in &true_units[s] {
                expr.push_str(&format!(" {} {} * +", id, wt));
            }
            let Ok(l2) = w.pol(&expr) else { return };
            *ge = g2;
            *le = l2;
            *work += 4;
        }
        // 3. the two sides added: the shared partial products cancel
        let Ok(mut cur1) = w.pol(&format!("{} {} +", ge_a, le_b)) else { return }; // B - A >= 0
        let Ok(mut cur2) = w.pol(&format!("{} {} +", le_a, ge_b)) else { return }; // A - B >= 0
        // 4. the top block: weaken every single-weight literal and divide
        //    by 2^(K+1), which says block B >= block A (and the reverse), then
        //    cancel the block from both running constraints
        if !block.is_empty() {
            let mut wk = String::new();
            for &bit in &weights_all {
                for s in 0..2 {
                    if let Some(l) = bit_of(s, bit) {
                        wk.push_str(&format!(" x{} w", l.abs()));
                    }
                }
            }
            let div = 1i128 << (top_k + 1);
            let Ok(t1) = w.pol(&format!("{}{} {} d", cur1, wk, div)) else { return };
            let Ok(t2) = w.pol(&format!("{}{} {} d", cur2, wk, div)) else { return };
            let Ok(n1) = w.pol(&format!("{} {} {} * +", cur1, t2, div)) else { return };
            let Ok(n2) = w.pol(&format!("{} {} {} * +", cur2, t1, div)) else { return };
            cur1 = n1;
            cur2 = n2;
            *work += wk.len() as u64 / 4 + 4;
        }
        // 5. bit by bit from the top
        for &bit in &weights_all {
            let lower: Vec<i32> = weights_all
                .iter()
                .filter(|&&u| u < bit)
                .flat_map(|&u| [bit_of(0, u), bit_of(1, u)])
                .flatten()
                .map(|l| l.abs())
                .collect();
            let mut wk = String::new();
            for v in &lower {
                wk.push_str(&format!(" x{} w", v));
            }
            let two = 1i128 << bit;
            match (bit_of(0, bit), bit_of(1, bit)) {
                (Some(a), Some(b)) => {
                    let Ok(c1) = w.pol(&format!("{}{} {} d", cur1, wk, two)) else { return }; // b + ~a >= 1
                    let Ok(c2) = w.pol(&format!("{}{} {} d", cur2, wk, two)) else { return }; // a + ~b >= 1
                    let Ok(n1) = w.pol(&format!("{} {} {} * +", cur1, c2, two)) else { return };
                    let Ok(n2) = w.pol(&format!("{} {} {} * +", cur2, c1, two)) else { return };
                    cur1 = n1;
                    cur2 = n2;
                    equalities.push((vec![b, -a], c1));
                    equalities.push((vec![a, -b], c2));
                }
                (None, Some(b)) => {
                    let Ok(c2) = w.pol(&format!("{}{} {} d", cur2, wk, two)) else { return }; // ~b >= 1
                    let Ok(n1) = w.pol(&format!("{} {} {} * +", cur1, c2, two)) else { return };
                    let Ok(n2) = w.pol(&format!("{} x{} w", cur2, b.abs())) else { return };
                    cur1 = n1;
                    cur2 = n2;
                    equalities.push((vec![-b], c2));
                }
                (Some(a), None) => {
                    let Ok(c1) = w.pol(&format!("{}{} {} d", cur1, wk, two)) else { return }; // ~a >= 1
                    let Ok(n2) = w.pol(&format!("{} {} {} * +", cur2, c1, two)) else { return };
                    let Ok(n1) = w.pol(&format!("{} x{} w", cur1, a.abs())) else { return };
                    cur1 = n1;
                    cur2 = n2;
                    equalities.push((vec![-a], c1));
                }
                (None, None) => {}
            }
            *work += lower.len() as u64 + 4;
        }
    } else {
        for &bit in &weights_all {
            match (bit_of(0, bit), bit_of(1, bit)) {
                (Some(a), Some(b)) => {
                    equalities.push((vec![b, -a], 0));
                    equalities.push((vec![a, -b], 0));
                }
                (None, Some(b)) => equalities.push((vec![-b], 0)),
                (Some(a), None) => equalities.push((vec![-a], 0)),
                (None, None) => {}
            }
        }
    }
    phase("proof", *work);
    // the equalities over the formula's variables alone reach the solver;
    // those over the proof's fresh bits stay in the proof
    equalities.retain(|(clause, _)| clause.iter().all(|l| l.abs() <= f.vars));
    let n = equalities.len();
    if stopped(*work) {
        return; // past the limit: nothing derived (the proof lines so far are harmless)
    }
    for (clause, id) in equalities {
        derived.add(&clause, id);
    }
    derived.notes.push(format!(
        "arithmetic: two adder networks of {} and {} cells over {} shared partial products, outputs equal bit by bit: {} clauses",
        side_cells[0].len(),
        side_cells[1].len(),
        shared_pp,
        n
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sum_out_of_range_is_flagged_not_wrapped() {
        let mut f = Form::default();
        let big = i128::MAX / 2 + 1;
        f.add_lit(3, big);
        assert!(!f.overflow);
        f.add_lit(3, big);
        assert!(f.overflow, "two halves of the range past i128::MAX");
        let mut g = Form::default();
        g.add_lit(-5, big);
        g.add_lit(-5, big); // the constant overflows through negated literals
        assert!(g.overflow);
        let mut h = Form::default();
        h.add_constant(i128::MAX);
        h.add_constant(1);
        assert!(h.overflow);
    }

    fn xor_net(gates_def: &[(i32, [i32; 2], u64)], nv: usize) -> Netlist {
        let mut gates: Vec<Option<Gate>> = (0..nv).map(|_| None).collect();
        let mut order = Vec::new();
        for &(v, sup, tt) in gates_def {
            gates[v as usize] = Some(Gate { sup: sup.to_vec(), tt });
            order.push(v);
        }
        Netlist { gates, order, used: Vec::new(), units: vec![false; nv], rep: (0..nv as i32).map(|v| (v, false)).collect(), deadlocks: 0, collapsed: Vec::new() }
    }

    #[test]
    fn an_xor_of_an_xor_collapses_over_its_own_inputs_only() {
        // inputs 1 (x), 2 (z), 3 (y). 4 = p = x xor z, 5 = q = x xor z (a
        // copy), 6 = g = q xor y, 7 = v = p xor g: v is y, but only through
        // the merge of q into p, whose proof the cones may no longer ask
        // for once v is collapsed: no collapse (review, 2026-10-23)
        let mut net = xor_net(&[(4, [1, 2], XOR2), (5, [1, 2], XOR2), (6, [3, 5], XOR2), (7, [4, 6], XOR2)], 8);
        let mut work = 0u64;
        structural_hash(&mut net, &mut work, &|_| false).unwrap();
        assert_eq!(net.rep[5], (4, false), "the copy is merged");
        assert_eq!(net.rep[7], (7, false), "no collapse through the merged copy");
        assert!(net.collapsed.is_empty());
        // the same over the gates' own inputs: 4 = y = and(2, 3), 5 = h =
        // xnor(x, y) over x itself, 6 = v = x xor h: v is y complemented,
        // by unit propagation over v's inputs and h's clauses
        let mut net = xor_net(&[(4, [2, 3], 0b1000), (5, [1, 4], XNOR2), (6, [1, 5], XOR2)], 7);
        structural_hash(&mut net, &mut work, &|_| false).unwrap();
        assert_eq!(net.rep[6], (4, true), "x xor xnor(x, y) is not y");
        assert_eq!(net.collapsed, vec![(1, 5, 4)]);
        // two copies of the pair: the second inner xnor merges into the
        // first, and the second outer xor still collapses on its own
        // (its inner gate's clauses are its own; review, 2026-10-23)
        let mut net = xor_net(&[(4, [2, 3], 0b1000), (5, [1, 4], XNOR2), (6, [1, 5], XOR2), (7, [1, 4], XNOR2), (8, [1, 7], XOR2)], 9);
        structural_hash(&mut net, &mut work, &|_| false).unwrap();
        assert_eq!(net.rep[7], (5, false), "the copy of the inner xnor is merged");
        assert_eq!(net.rep[6], (4, true));
        assert_eq!(net.rep[8], (4, true), "the copy collapses too");
        // onto a primary input: declined (the proof's cones hold no input,
        // and the equality would go unproven)
        let mut net = xor_net(&[(5, [1, 3], XNOR2), (6, [1, 5], XOR2)], 7);
        structural_hash(&mut net, &mut work, &|_| false).unwrap();
        assert_eq!(net.rep[6], (6, false), "no collapse onto the input 3");
        assert!(net.collapsed.is_empty());
    }

    #[test]
    fn a_monomial_is_read_only_from_an_and_or_a_nand_table() {
        assert_eq!(monomial_reading(0b1000, 2), Some(false)); // x and y
        assert_eq!(monomial_reading(0b0111, 2), Some(true)); // nand
        assert_eq!(monomial_reading(0b1011, 2), None); // x or ~y, no monomial
        assert_eq!(monomial_reading(0b1110, 2), None); // x or y
        assert_eq!(monomial_reading(0x80, 3), Some(false));
        assert_eq!(monomial_reading(0x7f, 3), Some(true));
        assert_eq!(monomial_reading(0x7b, 3), None);
        assert_eq!(monomial_reading(1u64 << 63, 6), Some(false));
        assert_eq!(monomial_reading(!(1u64 << 63), 6), Some(true));
        assert_eq!(monomial_reading(0b10, 1), Some(false)); // the input itself
        assert_eq!(monomial_reading(0b01, 1), Some(true));
    }

    #[test]
    fn case_split_ends_in_the_empty_clause_children_first() {
        let t = case_split(&[4, 7]);
        assert_eq!(t.len(), split_len(&[4, 7]));
        assert_eq!(t.last().unwrap(), &Vec::<i32>::new());
        assert_eq!(t[0], vec![-4, -7]); // a full assignment first
        assert_eq!(t[2], vec![-4]); // then its parent
    }
}
