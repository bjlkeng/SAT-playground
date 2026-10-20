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
//! Proof: per cell the two sides of its equation as `pbc` blocks with a
//! short case split in `rup` steps; per cone one weighted `pol` sum; the two
//! cones added so the partial products cancel; per bit a weakening and a
//! division. Checked on toys by `tools/arith_toy_proof.py` first. The pass
//! declines (derives nothing) when the integer check does not close: a cone
//! with gates outside adder cells (a parallel-prefix final adder, Booth
//! partial-product logic) is left to search.

use crate::structure::{Derived, Formula};
use std::collections::{HashMap, HashSet};

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
}

impl WeightedUf {
    fn new(n: usize) -> Self {
        WeightedUf { parent: (0..n as i32).collect(), off: vec![0; n] }
    }
    fn find(&mut self, x: i32) -> (i32, i64) {
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
    let mut sets_of: HashMap<&Vec<u32>, u32> = HashMap::new();
    for (_, _, _, key) in &cands {
        *sets_of.entry(key).or_default() += 1;
    }
    let mut single = 0usize;
    let mut ordered = 0usize;
    for (v, sup, _, key) in &cands {
        if sets_of[key] == 1 {
            single += 1;
            if sup.iter().all(|&u| u < *v) {
                ordered += 1;
            }
        }
    }
    let topo_numbered = single >= 20 && ordered * 20 >= single * 19;
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
    Some(Netlist { gates, order, used, units, rep, deadlocks })
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
fn find_adders(net: &Netlist, work: &mut u64, stop: &dyn Fn(u64) -> bool) -> Option<Vec<Cell>> {
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
        let carries: Vec<(i32, u8)> = nodes.iter().filter_map(|&(v, tt)| maj_family(tt).filter(|_| !used.contains(&v)).map(|m| (v, m))).collect();
        for (&(s, stt), &(cv, negmask)) in sums.iter().zip(carries.iter()) {
            let flips = (negmask.count_ones() & 1) == 1;
            let sum_neg = (stt == XNOR3) != flips;
            cells.push(Cell { fa: true, one: false, leaves: (*c).clone(), negmask, sum: s, sum_neg, carry: cv });
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
        let carries: Vec<(i32, u8)> = nodes.iter().filter_map(|&(v, tt)| and_family(tt).filter(|_| !used.contains(&v)).map(|m| (v, m))).collect();
        for (&(s, stt), &(cv, negmask)) in sums.iter().zip(carries.iter()) {
            let flips = (negmask.count_ones() & 1) == 1;
            let sum_neg = (stt == XNOR2) != flips;
            cells.push(Cell { fa: false, one: false, leaves: (*c).clone(), negmask, sum: s, sum_neg, carry: cv });
            used.insert(s);
            used.insert(cv);
        }
        sums.retain(|&(v, _)| !used.contains(&v));
        let carries1: Vec<(i32, u8)> = nodes.iter().filter_map(|&(v, tt)| or_family(tt).filter(|_| !used.contains(&v)).map(|m| (v, m))).collect();
        for (&(s, stt), &(cv, negmask)) in sums.iter().zip(carries1.iter()) {
            let flips = (negmask.count_ones() & 1) == 1;
            let k = (stt == XNOR2) != flips; // s = (la xor lb) xor k
            cells.push(Cell { fa: false, one: true, leaves: (*c).clone(), negmask, sum: s, sum_neg: !k, carry: cv });
            used.insert(s);
            used.insert(cv);
        }
    }
    Some(cells)
}

/// The cone of a representative node: every representative gate under it.
fn cone(rt: i32, net: &Netlist, work: &mut u64) -> HashSet<i32> {
    let mut seen = HashSet::new();
    let mut stack = vec![rt];
    while let Some(u) = stack.pop() {
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
    uf.find(cells[ci].leaves[0]).1
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
/// both ends, and a leaf two cells read must be read the same way. Returns
/// false when no choice fits.
fn resolve_polarities(cells: &mut [Cell], chosen: &[usize]) -> bool {
    let mut uf = ParityUf::new(cells.len());
    let mut producer: HashMap<i32, (usize, u8)> = HashMap::new(); // var -> (cell, produced negated)
    for &ci in chosen {
        let c = &cells[ci];
        producer.insert(c.sum, (ci, c.sum_neg as u8));
        producer.insert(c.carry_var(), (ci, (c.carry < 0) as u8));
    }
    let mut first_reader: HashMap<i32, (usize, u8)> = HashMap::new();
    for &ci in chosen {
        for i in 0..cells[ci].leaves.len() {
            let x = cells[ci].leaves[i];
            let q = ((cells[ci].negmask >> i) & 1) as u8;
            if let Some(&(pc, p)) = producer.get(&x) {
                if !uf.union(pc, ci, p ^ q) {
                    if debug() {
                        eprintln!("structarith: polarity conflict on signal {} between producer cell {:?} and reader cell {:?}", x, (pc, &cells[pc].leaves, cells[pc].negmask, cells[pc].sum, cells[pc].sum_neg, cells[pc].carry, cells[pc].fa), (ci, &cells[ci].leaves, cells[ci].negmask, cells[ci].sum, cells[ci].sum_neg, cells[ci].carry, cells[ci].fa));
                    }
                    return false;
                }
            } else if let Some(&(rc, r)) = first_reader.get(&x) {
                if !uf.union(rc, ci, r ^ q) {
                    if debug() {
                        eprintln!("structarith: polarity conflict on leaf {} between reader cells {:?} and {:?}", x, (rc, &cells[rc].leaves, cells[rc].negmask, cells[rc].fa), (ci, &cells[ci].leaves, cells[ci].negmask, cells[ci].fa));
                    }
                    return false;
                }
            } else {
                first_reader.insert(x, (ci, q));
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

pub fn run(f: &Formula, derived: &mut Derived, budget: u64, work: &mut u64, stop: &std::sync::atomic::AtomicBool) {
    // the work goes straight into the statistics' tick counter, so a signal
    // during the pass reports it; with no allowance left nothing runs
    if *work >= budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let stopped = |w: u64| w > budget || stop.load(std::sync::atomic::Ordering::Relaxed);
    let dbg = debug();
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
    let Some(mut cells) = find_adders(&net, work, &stopped) else {
        return;
    };
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
            if c.leaves.len() != 2 {
                return true;
            }
            let (a, b) = (sim[c.leaves[0] as usize], sim[c.leaves[1] as usize]);
            a != b && a != !b
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
        let mut ok = vec![false; cells.len()];
        for ci in order {
            let c = &cells[ci];
            let base = c.leaves[0];
            let mut trial: Vec<(i32, i32, i64)> = c.leaves[1..].iter().map(|&x| (x, base, 0)).collect();
            trial.push((c.sum, base, 0));
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
                let (r, o) = uf0.find(c.leaves[0]);
                let e = top.entry(r).or_insert(i64::MIN);
                *e = (*e).max(o);
            }
        }
        let mut keep: Vec<bool> = vec![true; cells.len()];
        for (ci, c) in cells.iter().enumerate() {
            if !ok[ci] {
                continue;
            }
            if c.leaves.len() == 2 && !read.contains(&c.sum) && !read.contains(&c.carry_var()) {
                let (r, o) = uf0.find(c.leaves[0]);
                if o + 2 <= top[&r] {
                    keep[ci] = false;
                    if dbg {
                        eprintln!("structarith: dead-end cell {} leaves {:?} at weight exponent {} of {} dropped", ci, c.leaves, o, top[&r]);
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
    // the cells of each side: those with an output in the side's cone, and
    // then every cell that consumes an output of a cell the side alone has
    // (a miter on one bit has cells above that bit; without them two
    // carries would dangle at one weight)
    let mut in_side: [Vec<bool>; 2] = [vec![false; cells.len()], vec![false; cells.len()]];
    for (ci, c) in cells.iter().enumerate() {
        let (sv, cv) = (net.rep[c.sum as usize].0, net.rep[c.carry_var() as usize].0);
        in_side[0][ci] = cone_a.contains(&sv) || cone_a.contains(&cv);
        in_side[1][ci] = cone_b.contains(&sv) || cone_b.contains(&cv);
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
    cell_order.sort_by_key(|&ci| (pos[net.rep[cells[ci].sum as usize].0 as usize], !cells[ci].fa, ci));
    let mut rejected: Vec<usize> = Vec::new();
    for ci in cell_order {
        let c = &cells[ci];
        let (sv, cv) = (net.rep[c.sum as usize].0, net.rep[c.carry_var() as usize].0);
        let in_a = in_side[0][ci];
        let in_b = in_side[1][ci];
        if !in_a && !in_b {
            continue;
        }
        // a cell both circuits share (hashing merged two identical ones) is
        // part of both sums and cancels in their difference
        // check the cell's equations together, then commit
        let base = c.leaves[0];
        let mut trial: Vec<(i32, i32, i64)> = c.leaves[1..].iter().map(|&x| (x, base, 0)).collect();
        trial.push((sv, base, 0));
        trial.push((cv, base, 1));
        let eqs: Vec<(i32, i32, i64)> = trial
            .iter()
            .map(|&(a, b, d)| {
                let (ra, oa) = uf.find(a);
                let (rb, ob) = uf.find(b);
                (ra, rb, d - oa + ob)
            })
            .collect();
        let ok = consistent(&eqs);
        if !ok {
            rejected.push(ci);
            continue; // an xor and an and over the same two control signals look like a half adder
        }
        for &(a, b, d) in &trial {
            uf.union(a, b, d);
        }
        if in_a {
            side_cells[0].push(ci);
        }
        if in_b {
            side_cells[1].push(ci);
        }
        *work += 8;
    }
    if dbg {
        eprintln!("structarith: side cells {} and {} (weights consistent), work {}K", side_cells[0].len(), side_cells[1].len(), *work / 1000);
    }
    if side_cells[0].is_empty() || side_cells[1].is_empty() {
        return;
    }
    let chosen: Vec<usize> = {
        let mut v: Vec<usize> = side_cells[0].iter().chain(side_cells[1].iter()).copied().collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    if !resolve_polarities(&mut cells, &chosen) {
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
    // the integer check: each side's cells summed with their weights; every
    // internal signal cancels; what is left must be the side's leaves (read
    // the same way, with the same weight, by the other side: shared partial
    // products) and the side's output bits, one literal per weight
    let mut exps: [Vec<i64>; 2] = [Vec::new(), Vec::new()];
    let mut roots: [i32; 2] = [0, 0];
    for s in 0..2 {
        let base = cells[side_cells[s][0]].leaves[0];
        let (r, _) = uf.find(base);
        roots[s] = r;
        for &ci in &side_cells[s] {
            let (rc, o) = uf.find(cells[ci].leaves[0]);
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
    let weight_of = |ci: usize, uf: &mut WeightedUf| -> i128 {
        let (_, o) = uf.find(cells[ci].leaves[0]);
        1i128 << (o - min_exp)
    };
    let mut forms: [Form; 2] = [Form::default(), Form::default()];
    let mut expected: [Form; 2] = [Form::default(), Form::default()];
    let mut bits: [HashMap<u32, Vec<i32>>; 2] = [HashMap::new(), HashMap::new()]; // weight -> output literals
    let mut leaf_form: [Form; 2] = [Form::default(), Form::default()]; // each side's leaves and constants
    for s in 0..2 {
        let produced: HashSet<i32> = side_cells[s].iter().flat_map(|&ci| [cells[ci].sum, cells[ci].carry_var()]).collect();
        let consumed: HashSet<i32> = side_cells[s].iter().flat_map(|&ci| cells[ci].leaves.iter().copied()).collect();
        for &ci in &side_cells[s] {
            let c = &cells[ci];
            let w = weight_of(ci, &mut uf);
            for i in 0..c.leaves.len() {
                let l = c.leaf_lit(i);
                forms[s].add_lit(l, w);
                if !produced.contains(&c.leaves[i]) {
                    expected[s].add_lit(l, w);
                    leaf_form[s].add_lit(l, w);
                }
            }
            if c.one {
                forms[s].add_constant(w);
                expected[s].add_constant(w);
                leaf_form[s].add_constant(w);
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
        if forms[s].overflow || expected[s].overflow || leaf_form[s].overflow {
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
                            eprintln!("    cell {:?} leaves {:?} negmask {} sum {} sum_neg {} carry {} one {} fa {} weight {}", ci, c.leaves, c.negmask, c.sum, c.sum_neg, c.carry, c.one, c.fa, weight_of(ci, &mut uf));
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
    leaf_form[0].prune();
    leaf_form[1].prune();
    *work += (leaf_form[0].coeff.len() + leaf_form[1].coeff.len()) as u64;
    if leaf_form[0].coeff != leaf_form[1].coeff || leaf_form[0].constant != leaf_form[1].constant {
        if dbg {
            let mut diff: Vec<(i32, i128, i128)> = leaf_form[0]
                .coeff
                .iter()
                .map(|(&v, &c)| (v, c, leaf_form[1].coeff.get(&v).copied().unwrap_or(0)))
                .filter(|&(_, a, b)| a != b)
                .collect();
            diff.extend(leaf_form[1].coeff.iter().filter(|(v, _)| !leaf_form[0].coeff.contains_key(v)).map(|(&v, &b)| (v, 0, b)));
            diff.sort_unstable();
            eprintln!(
                "structarith: the two sides sum different leaves: {} variables differ, constants {} and {}, for example {:?}",
                diff.len(),
                leaf_form[0].constant,
                leaf_form[1].constant,
                &diff[..diff.len().min(4)]
            );
        }
        return;
    }
    let shared_pp = leaf_form[0].coeff.len();
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
        // 1. per cell the two inequalities, by contradiction with a case split
        let mut ge_ids: [Vec<(u64, i128)>; 2] = [Vec::new(), Vec::new()];
        let mut le_ids: [Vec<(u64, i128)>; 2] = [Vec::new(), Vec::new()];
        for s in 0..2 {
            for &ci in &side_cells[s] {
                let c = &cells[ci];
                let wt = weight_of(ci, &mut uf);
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
                let split = case_split(&c.leaves);
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
                *work += 2 * split_len(&c.leaves) as u64 + 8;
                if stopped(*work) {
                    return; // the lines written so far derive nothing the formula keeps
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
        let Ok(ge_a) = weighted(w, &ge_ids[0]) else { return };
        let Ok(le_a) = weighted(w, &le_ids[0]) else { return };
        let Ok(ge_b) = weighted(w, &ge_ids[1]) else { return };
        let Ok(le_b) = weighted(w, &le_ids[1]) else { return };
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

    #[test]
    fn case_split_ends_in_the_empty_clause_children_first() {
        let t = case_split(&[4, 7]);
        assert_eq!(t.len(), split_len(&[4, 7]));
        assert_eq!(t.last().unwrap(), &Vec::<i32>::new());
        assert_eq!(t[0], vec![-4, -7]); // a full assignment first
        assert_eq!(t[2], vec![-4]); // then its parent
    }
}
