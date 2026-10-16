//! Not in kissat. The parity pass of the structure pass (plan section 11,
//! 2026-10-07; bead SAT-playground-1v2.2), step one: pure XOR systems.
//!
//! An XOR constraint over k variables is the 2^(k-1) clauses that forbid
//! every assignment of one parity. When a set of such constraints is
//! closed (every variable of the set occurs nowhere else in the formula)
//! and every variable sits in exactly two constraints, the constraints are
//! the vertices and the variables the edges of a graph, and flipping every
//! variable on a cycle keeps the parity at every vertex: it is a symmetry
//! of the formula, and a symmetry that maps a variable to its own
//! negation lets that variable be fixed to either value (the redundance
//! rule with the flip as witness). Fixing one edge per fundamental cycle
//! of a spanning forest leaves a forest, where unit propagation settles
//! every edge from the leaves up; an odd total parity then falsifies the
//! root's constraint and the formula is refuted. This is Tseitin's formula
//! on any graph, and the symmetry pass cannot see it, since the symmetry
//! exchanges no two literals.
//!
//! Proof: `red x : x -> ~x for every x on the cycle` per fixed edge,
//! then every propagated unit as `rup`, then the empty clause as `rup`.
//!
//! Step two (2026-10-12): flip symmetries of any formula. Negating every
//! variable of a set S maps the formula onto itself when, for every set V
//! of variables that some clauses are written over, the sign patterns of
//! those clauses are closed under flipping S within V. The flips that
//! keep one such group of patterns form a linear space over GF(2), so the
//! sets S are the solutions of a sparse linear system, one unknown per
//! variable: a variable alone in its group of clauses cannot be flipped,
//! the two variables of `a xor b` substituted for a variable must be
//! flipped together, a Tseitin vertex must be met an even number of
//! times. Solving it leaves free variables, and for each free variable f
//! one flip symmetry that holds f and no other free variable; that
//! symmetry fixes f (the same redundance step as above), and since it
//! leaves the other free variables alone, all of them can be fixed. A
//! formula whose variables were each replaced by the XOR of two fresh
//! ones becomes the formula it was built from. The algebra only
//! proposes: every flip is checked against every clause it touches
//! before it is used. Sums of XOR constraints that mix with other
//! clauses (Gaussian elimination with a proof) are not done here.

use crate::structure::{Derived, Formula};
use std::collections::HashMap;

const MAX_K: usize = 8;

/// Literal index: 2v for v, 2v+1 for not v.
#[inline]
fn idx(l: i32) -> usize {
    (l.unsigned_abs() as usize) * 2 + (l < 0) as usize
}

/// One XOR constraint: its variables and the indices of its clauses.
struct Xor {
    vars: Vec<i32>,
    clauses: Vec<u32>,
    /// The number of true variables the constraint allows, mod 2.
    parity: u32,
}

/// The XOR constraints of the formula: groups of clauses over one variable
/// set that are exactly the sign patterns of one parity.
fn extract(f: &Formula, work: &mut u64, budget: u64, ticks: &mut u64, stop: &std::sync::atomic::AtomicBool) -> Vec<Xor> {
    let mut groups: HashMap<Vec<i32>, Vec<u32>> = HashMap::new();
    for i in 0..f.len() {
        if i % 4096 == 0 {
            *ticks = *work; // published for a signal during the pass
            if *work > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
                return Vec::new();
            }
        }
        if f.taut[i] {
            continue;
        }
        let c = f.clause(i);
        *work += c.len() as u64 + 1;
        if c.len() < 2 || c.len() > MAX_K {
            continue;
        }
        let mut key: Vec<i32> = c.iter().map(|l| l.abs()).collect();
        key.sort_unstable();
        key.dedup();
        if key.len() != c.len() {
            continue;
        }
        groups.entry(key).or_default().push(i as u32);
    }
    let mut out = Vec::new();
    // groups in the order of their first clause: the same input charges
    // the same work whatever the hash seed, so a tick limit is deterministic
    let mut ordered: Vec<(Vec<i32>, Vec<u32>)> = groups.into_iter().collect();
    ordered.sort_unstable_by_key(|(_, clauses)| clauses[0]);
    for (gi, (vars, clauses)) in ordered.into_iter().enumerate() {
        if gi % 1024 == 0 {
            *ticks = *work;
            if *work > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
                return Vec::new();
            }
        }
        let k = vars.len();
        let need = 1usize << (k - 1);
        *work += 1;
        if clauses.len() < need {
            continue;
        }
        *work += (clauses.len() % 1024) as u64 * k as u64; // the rest is charged inside the scan
        // the clause with negatives on an odd number of the variables
        // forbids the pattern with an odd number of trues: group by parity
        let mut by_parity: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
        let mut seen: [HashMap<u64, ()>; 2] = [HashMap::new(), HashMap::new()];
        for (ni, &ci) in clauses.iter().enumerate() {
            if ni % 1024 == 1023 {
                // a group may hold any number of duplicate clauses
                *work += 1024 * k as u64;
                *ticks = *work;
                if *work > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
                    return Vec::new();
                }
            }
            let c = f.clause(ci as usize);
            let mut pattern: u64 = 0;
            let mut negatives = 0usize;
            for &l in c {
                let pos = vars.binary_search(&l.abs()).unwrap();
                if l < 0 {
                    negatives += 1;
                    pattern |= 1 << pos;
                }
            }
            let parity = negatives & 1;
            if seen[parity].insert(pattern, ()).is_none() {
                by_parity[parity].push(ci);
            }
        }
        for parity in 0..2 {
            if by_parity[parity].len() == need {
                // these clauses forbid every pattern with `parity` trues
                out.push(Xor { vars: vars.clone(), clauses: by_parity[parity].clone(), parity: 1 - parity as u32 });
                break;
            }
        }
    }
    out
}

/// The XOR constraints of the formula for the parity engine
/// (`src/gauss.rs`): each as its variables and the parity it allows, and
/// per variable whether it occurs in a clause outside those XORs (a
/// variable that does not is the engine's to drop once it has pivoted on
/// it: the XOR clauses themselves still fix it).
pub fn xors(f: &Formula, budget: u64, ticks: &mut u64, stop: &std::sync::atomic::AtomicBool) -> (Vec<(Vec<i32>, u32)>, Vec<bool>) {
    // as closed_systems counts: `work` is the absolute tick count
    let mut work = *ticks;
    let xors = extract(f, &mut work, budget, ticks, stop);
    *ticks = work;
    if xors.len() < 2 || work > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
        // nothing for the engine, or the budget is spent: no second scan
        return (Vec::new(), Vec::new());
    }
    let mut in_xor: Vec<bool> = vec![false; f.len()];
    for x in &xors {
        for &c in &x.clauses {
            in_xor[c as usize] = true;
        }
    }
    let mut mixed: Vec<bool> = vec![false; f.vars as usize + 1];
    for i in 0..f.len() {
        if i % 4096 == 0 {
            *ticks = work;
            if work > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
                return (Vec::new(), Vec::new());
            }
        }
        if in_xor[i] || f.taut[i] {
            continue;
        }
        let c = f.clause(i);
        work += c.len() as u64 + 1;
        for &l in c {
            mixed[l.unsigned_abs() as usize] = true;
        }
    }
    *ticks = work;
    (xors.into_iter().map(|x| (x.vars, x.parity)).collect(), mixed)
}

struct Prop<'a> {
    f: &'a Formula,
    occ: Vec<Vec<u32>>,
    value: Vec<i8>,
    satisfied: Vec<bool>,
    queued: Vec<bool>,
    work: u64,
    budget: u64,
    propagated: usize,
    ticks: *mut u64,
    stop: &'a std::sync::atomic::AtomicBool,
}

impl<'a> Prop<'a> {
    #[inline]
    fn lit_value(&self, l: i32) -> i8 {
        let v = self.value[l.unsigned_abs() as usize];
        if l > 0 {
            v
        } else {
            -v
        }
    }

    /// Assign a literal true and propagate units, logging every unit;
    /// false on a conflict (the empty clause was logged) or when out of
    /// budget before the queue was empty.
    fn assign_and_propagate(&mut self, lit: i32, derived: &mut Derived) -> bool {
        let mut queue = vec![lit];
        self.queued[idx(lit)] = true;
        let mut qi = 0;
        let mut ok = true;
        while qi < queue.len() {
            let l = queue[qi];
            qi += 1;
            let v = l.unsigned_abs() as usize;
            if self.value[v] != 0 {
                if self.lit_value(l) < 0 {
                    let id = derived.proof.as_mut().map(|w| w.rup(&[]).unwrap_or(0)).unwrap_or(0);
                    derived.add(&[], id);
                    ok = false;
                    break;
                }
                continue;
            }
            self.value[v] = if l > 0 { 1 } else { -1 };
            for &i in &self.occ[idx(l)] {
                self.satisfied[i as usize] = true;
            }
            for oi in 0..self.occ[idx(-l)].len() {
                let i = self.occ[idx(-l)][oi] as usize;
                if self.satisfied[i] {
                    continue;
                }
                self.work += self.f.clause(i).len() as u64 + 1;
                // SAFETY: `ticks` is the solver's counter, alive for the pass
                unsafe { *self.ticks = self.work };
                if self.work > self.budget || self.stop.load(std::sync::atomic::Ordering::Relaxed) {
                    ok = false;
                    break;
                }
                let mut unassigned: Option<i32> = None;
                let mut count = 0;
                for &x in self.f.clause(i) {
                    let val = self.lit_value(x);
                    if val > 0 {
                        count = -1;
                        break;
                    }
                    if val == 0 {
                        count += 1;
                        unassigned = Some(x);
                    }
                }
                if count < 0 {
                    self.satisfied[i] = true;
                    continue;
                }
                if count == 0 {
                    let id = derived.proof.as_mut().map(|w| w.rup(&[]).unwrap_or(0)).unwrap_or(0);
                    derived.add(&[], id);
                    ok = false;
                    break;
                }
                if count == 1 {
                    let u = unassigned.unwrap();
                    if !self.queued[idx(u)] {
                        self.queued[idx(u)] = true;
                        let id = derived.proof.as_mut().map(|w| w.rup(&[u]).unwrap_or(0)).unwrap_or(0);
                        derived.add(&[u], id);
                        self.propagated += 1;
                        queue.push(u);
                    }
                }
            }
            if !ok {
                break;
            }
        }
        for &l in &queue {
            self.queued[idx(l)] = false;
        }
        ok
    }
}

pub fn run(f: &Formula, derived: &mut Derived, budget: u64, ticks: &mut u64, stop: &std::sync::atomic::AtomicBool) {
    closed_systems(f, derived, budget, ticks, stop);
    if !derived.refuted {
        flips(f, derived, budget, ticks, stop);
    }
}

/// Variables in a group of clauses over one variable set (one mask word).
const FLIP_MAX_K: usize = 64;
/// Clauses in one such group.
const FLIP_MAX_GROUP: usize = 1 << 16;

#[inline]
fn mix(mut h: u64) -> u64 {
    h = h.wrapping_add(0x9e37_79b9_7f4a_7c15);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

/// The clauses written over one set of variables: the variables in
/// ascending order and the distinct sign patterns (bit j set when
/// `vars[j]` is negated), sorted.
struct Group {
    vars: Vec<i32>,
    masks: Vec<u64>,
}

/// The number of bits of n: what one search in a sorted list of n costs.
#[inline]
fn depth(n: usize) -> u64 {
    64 - (n as u64).leading_zeros() as u64
}

/// Step two: the flip symmetries of the formula, one per free variable of
/// the linear system described in the module text; each fixes its free
/// variable to true.
fn flips(f: &Formula, derived: &mut Derived, budget: u64, ticks: &mut u64, stop: &std::sync::atomic::AtomicBool) {
    let mut work = *ticks;
    let mut polled = work; // the work when it was last published
    // No piece of work starts unless the limit still covers its cost (a
    // clause to scan, a sort, a table per variable); it is charged as it
    // starts, and the count is published every few thousand units for a
    // signal during the step. Nothing is derived before the last loop, so
    // stopping early anywhere above it leaves the formula as it was.
    let covers = |work: u64, cost: u64| work.saturating_add(cost) <= budget && !stop.load(std::sync::atomic::Ordering::Relaxed);
    macro_rules! spend {
        ($cost:expr) => {{
            let cost: u64 = $cost;
            if !covers(work, cost) {
                *ticks = work;
                return;
            }
            work += cost;
            if work - polled >= 4096 {
                polled = work;
                *ticks = work;
            }
        }};
    }
    macro_rules! spend_or_leave {
        ($cost:expr, $label:lifetime) => {{
            let cost: u64 = $cost;
            if !covers(work, cost) {
                break $label;
            }
            work += cost;
            if work - polled >= 4096 {
                polled = work;
                *ticks = work;
            }
        }};
    }
    let nv = f.vars as usize + 1;
    // the tables per variable of the whole step (two counts, three marks)
    // and its last scan over the variables
    spend!(6 * nv as u64);
    // a flip maps the clauses holding x onto the clauses holding not x, so
    // a variable it can move occurs as often in either sign; what an
    // earlier step or pass derived stays as it is
    let mut count: Vec<u32> = vec![0; 2 * nv];
    for i in 0..f.len() {
        spend!(f.clause(i).len() as u64 + 1);
        if f.taut[i] {
            continue;
        }
        for &l in f.clause(i) {
            count[idx(l)] += 1;
        }
    }
    let mut open: Vec<bool> = (0..nv).map(|v| v > 0 && count[2 * v] > 0 && count[2 * v] == count[2 * v + 1]).collect();
    drop(count);
    for c in &derived.clauses {
        spend!(c.len() as u64 + 1);
        for &l in c {
            open[l.unsigned_abs() as usize] = false;
        }
    }
    if !open.iter().any(|&o| o) {
        *ticks = work;
        return;
    }
    // the clauses holding such a variable, keyed by a hash of their
    // variable set; equal sets are then runs of the sorted list (sorted
    // by hash, then clause number: the same input gives the same order)
    let mut keyed: Vec<(u64, u32)> = Vec::new();
    for i in 0..f.len() {
        spend!(f.clause(i).len() as u64 + 1);
        if f.taut[i] {
            continue;
        }
        let c = f.clause(i);
        if !c.iter().any(|l| open[l.unsigned_abs() as usize]) {
            continue;
        }
        let mut h: u64 = 0;
        for &l in c {
            h = h.wrapping_add(mix(l.unsigned_abs() as u64));
        }
        keyed.push((mix(h ^ ((c.len() as u64) << 48)), i as u32));
    }
    spend!(keyed.len() as u64 * depth(keyed.len()));
    keyed.sort_unstable();
    // every run becomes a group, or closes its variables: a clause alone
    // over its variable set has no image under any flip of them
    let mut groups: Vec<Group> = Vec::new();
    let mut equations: Vec<Vec<i32>> = Vec::new();
    let mut reference: Vec<i32> = Vec::new();
    let mut scratch: Vec<i32> = Vec::new();
    let mut a = 0usize;
    while a < keyed.len() {
        let mut b = a + 1;
        while b < keyed.len() && keyed[b].0 == keyed[a].0 {
            b += 1;
        }
        spend!(1);
        // (a clause holds a variable once, so its length is the size of
        // its variable set; a wide one is turned away before anything of
        // it is copied or sorted)
        let k = f.clause(keyed[a].1 as usize).len();
        let mut usable = b - a >= 2 && k <= FLIP_MAX_K && b - a <= 4 * FLIP_MAX_GROUP;
        let mut masks: Vec<u64> = Vec::new();
        reference.clear();
        if usable {
            reference.extend(f.clause(keyed[a].1 as usize).iter().map(|l| l.abs()));
            reference.sort_unstable();
            masks.reserve(b - a);
            for m in a..b {
                spend!(k as u64 + 1);
                let c = f.clause(keyed[m].1 as usize);
                if c.len() != k {
                    usable = false; // two variable sets with one hash: neither is looked at
                    break;
                }
                scratch.clear();
                scratch.extend(c.iter().map(|l| l.abs()));
                scratch.sort_unstable();
                if scratch != reference {
                    usable = false;
                    break;
                }
                let mut mask: u64 = 0;
                for &l in c {
                    if l < 0 {
                        mask |= 1 << reference.binary_search(&l.abs()).unwrap();
                    }
                }
                masks.push(mask);
            }
        }
        if usable {
            spend!(masks.len() as u64 * depth(masks.len()));
            masks.sort_unstable();
            masks.dedup();
            usable = masks.len() >= 2 && masks.len() <= FLIP_MAX_GROUP;
        }
        if !usable {
            for m in a..b {
                let c = f.clause(keyed[m].1 as usize);
                spend!(c.len() as u64 + 1);
                for &l in c {
                    open[l.unsigned_abs() as usize] = false;
                }
            }
            a = b;
            continue;
        }
        a = b;
        let mut open_mask: u64 = 0;
        for (j, &v) in reference.iter().enumerate() {
            if open[v as usize] {
                open_mask |= 1 << j;
            }
        }
        // the flips of open variables that keep the patterns: every one
        // maps the first pattern onto another, so the candidates are the
        // differences from it; the kept ones as a basis, each with a
        // pivot bit that no other basis vector has. A flip keeps a set of
        // patterns exactly when it keeps the patterns outside it, so when
        // more than half of all patterns are present the few absent ones
        // are looked at instead (none absent: every flip keeps the group).
        let mut basis: Vec<(u64, u64)> = Vec::new();
        let absent: Vec<u64>;
        let probe: &[u64] = if k <= 17 && (masks.len() as u64) * 2 > 1u64 << k {
            spend!((1u64 << k) * depth(masks.len()));
            absent = (0..1u64 << k).filter(|x| masks.binary_search(x).is_err()).collect();
            &absent
        } else {
            &masks
        };
        if probe.is_empty() {
            for j in 0..k {
                if open_mask >> j & 1 == 1 {
                    basis.push((1 << j, 1 << j));
                }
            }
        }
        let first = probe.first().copied().unwrap_or(0);
        let deep = depth(probe.len());
        for n in 1..probe.len() {
            spend!(1);
            let mut t = first ^ probe[n];
            if t & !open_mask != 0 {
                continue;
            }
            for &(v, pivot) in &basis {
                if t & pivot != 0 {
                    t ^= v;
                }
            }
            if t == 0 {
                continue;
            }
            // the check of one candidate may look at every pattern, and
            // stops at the first it does not keep: covered in full,
            // charged for what it looked at
            if !covers(work, probe.len() as u64 * deep) {
                *ticks = work;
                return;
            }
            let mut checked = 0u64;
            let keeps = probe.iter().all(|&x| {
                checked += 1;
                probe.binary_search(&(x ^ t)).is_ok()
            });
            work += checked * deep;
            if keeps {
                let pivot = t & t.wrapping_neg();
                for entry in basis.iter_mut() {
                    if entry.0 & pivot != 0 {
                        entry.0 ^= t;
                    }
                }
                basis.push((t, pivot));
            }
        }
        // what the basis says about each open variable of the group: in no
        // basis vector, it cannot be flipped; a pivot, it is free here;
        // otherwise it is flipped exactly when an odd number of the pivots
        // whose vectors hold it are (one equation)
        spend!((k * (basis.len() + 1)) as u64);
        let pivots: u64 = basis.iter().fold(0, |acc, e| acc | e.1);
        for (j, &v) in reference.iter().enumerate() {
            let bit = 1u64 << j;
            if open_mask & bit == 0 || pivots & bit != 0 {
                continue;
            }
            let mut equation: Vec<i32> = vec![v];
            for &(vector, pivot) in &basis {
                if vector & bit != 0 {
                    equation.push(reference[pivot.trailing_zeros() as usize]);
                }
            }
            if equation.len() == 1 {
                open[v as usize] = false;
            } else {
                equation.sort_unstable();
                equations.push(equation);
            }
        }
        groups.push(Group { vars: reference.clone(), masks });
    }
    drop(keyed);
    // the system: closed variables are zero, equal equations are one
    let mut entries: u64 = 0;
    for e in equations.iter_mut() {
        spend!(e.len() as u64 + 1);
        e.retain(|&v| open[v as usize]);
        entries += e.len() as u64 + 1;
    }
    equations.retain(|e| !e.is_empty());
    spend!(entries * depth(equations.len()));
    equations.sort_unstable();
    equations.dedup();
    // elimination, shortest equation first, pivot the variable of it in the
    // fewest equations: the pivot is removed from every other equation, so
    // an equation chosen later holds no earlier pivot, and what is left of
    // a chosen equation says what its pivot is the sum of. An equation of
    // one variable closes it (the sum of nothing).
    let mut uses: HashMap<i32, Vec<u32>> = HashMap::new();
    for (ei, e) in equations.iter().enumerate() {
        spend!(2 * e.len() as u64 + 2); // its entries here, and its place in the queue below
        for &v in e {
            uses.entry(v).or_default().push(ei as u32);
        }
    }
    let mut alive: Vec<bool> = vec![true; equations.len()];
    let mut heap: std::collections::BinaryHeap<std::cmp::Reverse<(usize, u32)>> =
        equations.iter().enumerate().map(|(ei, e)| std::cmp::Reverse((e.len(), ei as u32))).collect();
    let mut order: Vec<(i32, u32)> = Vec::new(); // (pivot variable, its equation)
    let mut is_pivot: Vec<bool> = vec![false; nv];
    while let Some(std::cmp::Reverse((len, ei))) = heap.pop() {
        let e = ei as usize;
        if !alive[e] || equations[e].len() != len {
            continue; // consumed, or changed since it was queued (queued again then)
        }
        alive[e] = false;
        if len == 0 {
            continue;
        }
        spend!(len as u64 + 1);
        let mut pivot = equations[e][0];
        let mut fewest = usize::MAX;
        for &v in &equations[e] {
            let n = uses.get(&v).map_or(0, |u| u.len());
            if n < fewest {
                fewest = n;
                pivot = v;
            }
        }
        spend!(fewest.min(1 << 40) as u64 + 1);
        let users: Vec<u32> = uses.get(&pivot).cloned().unwrap_or_default();
        for &oi in &users {
            let o = oi as usize;
            if o == e || !alive[o] || equations[o].binary_search(&pivot).is_err() {
                continue; // itself, consumed, or a stale entry
            }
            spend!((equations[o].len() + equations[e].len()) as u64 + 1);
            // the symmetric difference of two sorted lists
            let (x, y) = (&equations[o], &equations[e]);
            let mut sum: Vec<i32> = Vec::with_capacity(x.len() + y.len());
            let (mut i, mut j) = (0, 0);
            while i < x.len() && j < y.len() {
                if x[i] == y[j] {
                    i += 1;
                    j += 1;
                } else if x[i] < y[j] {
                    sum.push(x[i]);
                    i += 1;
                } else {
                    sum.push(y[j]);
                    uses.entry(y[j]).or_default().push(oi);
                    j += 1;
                }
            }
            sum.extend_from_slice(&x[i..]);
            for &v in &y[j..] {
                sum.push(v);
                uses.entry(v).or_default().push(oi);
            }
            heap.push(std::cmp::Reverse((sum.len(), oi)));
            equations[o] = sum;
        }
        is_pivot[pivot as usize] = true;
        order.push((pivot, ei));
    }
    drop(uses);
    // the pivots each variable is part of the sum of (a pivot's sum holds
    // free variables and later pivots only)
    let mut part_of: HashMap<i32, Vec<u32>> = HashMap::new();
    for (oi, &(pivot, ei)) in order.iter().enumerate() {
        spend!(equations[ei as usize].len() as u64 + 1);
        for &v in &equations[ei as usize] {
            if v != pivot {
                part_of.entry(v).or_default().push(oi as u32);
            }
        }
    }
    let mut groups_of: HashMap<i32, Vec<u32>> = HashMap::new();
    for (gi, g) in groups.iter().enumerate() {
        spend!(g.vars.len() as u64 + 1);
        for &v in &g.vars {
            if open[v as usize] {
                groups_of.entry(v).or_default().push(gi as u32);
            }
        }
    }
    spend!((2 * order.len() + groups.len()) as u64 + 1);
    // one flip per free variable f: f, and every pivot whose sum holds an
    // odd number of flipped variables, settled from the last pivot back
    let mut in_flip: Vec<u32> = vec![0; nv]; // the stamp of the flip holding the variable
    let mut odd: Vec<bool> = vec![false; order.len()];
    let mut queued: Vec<bool> = vec![false; order.len()];
    let mut seen_group: Vec<u32> = vec![0; groups.len()];
    let (mut found, mut fixed, mut largest, mut rejected) = (0usize, 0usize, 0usize, 0usize);
    let mut stamp = 0u32;
    // (leaving this loop early leaves the flip in hand unfinished and
    // unused; the ones before it stand)
    'fixing: for v in 1..nv {
        if !open[v] || is_pivot[v] {
            continue;
        }
        stamp += 1;
        let free = v as i32;
        let mut flip: Vec<i32> = vec![free];
        in_flip[v] = stamp;
        let mut pending: std::collections::BinaryHeap<u32> = std::collections::BinaryHeap::new();
        let mut touched: Vec<u32> = Vec::new();
        if let Some(list) = part_of.get(&free) {
            spend_or_leave!(list.len() as u64 + 1, 'fixing);
            for &oi in list {
                odd[oi as usize] ^= true;
                if !queued[oi as usize] {
                    queued[oi as usize] = true;
                    touched.push(oi);
                    pending.push(oi);
                }
            }
        }
        while let Some(oi) = pending.pop() {
            spend_or_leave!(2, 'fixing);
            if !odd[oi as usize] {
                continue;
            }
            let pivot = order[oi as usize].0;
            flip.push(pivot);
            in_flip[pivot as usize] = stamp;
            if let Some(list) = part_of.get(&pivot) {
                spend_or_leave!(list.len() as u64, 'fixing);
                for &pi in list {
                    odd[pi as usize] ^= true;
                    if !queued[pi as usize] {
                        queued[pi as usize] = true;
                        touched.push(pi);
                        pending.push(pi);
                    }
                }
            }
        }
        for &oi in &touched {
            odd[oi as usize] = false;
            queued[oi as usize] = false;
        }
        // the check: every group a flipped variable is in keeps its patterns
        let mut keeps = true;
        'check: for &x in &flip {
            let Some(list) = groups_of.get(&x) else {
                keeps = false; // a flipped variable in no group: not open after all
                break;
            };
            for &gi in list {
                if seen_group[gi as usize] == stamp {
                    continue;
                }
                seen_group[gi as usize] = stamp;
                let g = &groups[gi as usize];
                // a search per pattern, charged with its depth
                spend_or_leave!(g.vars.len() as u64 + g.masks.len() as u64 * depth(g.masks.len()), 'fixing);
                let mut t: u64 = 0;
                for (j, &u) in g.vars.iter().enumerate() {
                    if in_flip[u as usize] == stamp {
                        t |= 1 << j;
                    }
                }
                if !g.masks.iter().all(|&m| g.masks.binary_search(&(m ^ t)).is_ok()) {
                    keeps = false;
                    break 'check;
                }
            }
        }
        if !keeps {
            rejected += 1;
            continue;
        }
        spend_or_leave!(flip.len() as u64 * (depth(flip.len()) + 1), 'fixing);
        found += 1;
        largest = largest.max(flip.len());
        flip.sort_unstable();
        let witness: Vec<(i32, i32)> = flip.iter().map(|&x| (x, -x)).collect();
        // (to true: the value search tries first by default, and what the
        // first step fixes its edges to; either value is sound)
        let id = match derived.proof.as_mut() {
            Some(w) => w.red(&[free], &witness).unwrap_or(0),
            None => 0,
        };
        derived.add(&[free], id);
        fixed += 1;
    }
    *ticks = work;
    if found > 0 || rejected > 0 {
        derived.notes.push(format!(
            "parity: {} flip symmetr{} (of up to {} variable{}), {} variable{} fixed{}",
            found,
            if found == 1 { "y" } else { "ies" },
            largest,
            if largest == 1 { "" } else { "s" },
            fixed,
            if fixed == 1 { "" } else { "s" },
            if rejected > 0 { format!(", {} proposed and rejected", rejected) } else { String::new() }
        ));
    }
}

/// Step one: closed XOR systems as graphs (the module text).
fn closed_systems(f: &Formula, derived: &mut Derived, budget: u64, ticks: &mut u64, stop: &std::sync::atomic::AtomicBool) {
    let mut work = *ticks;
    let xors = extract(f, &mut work, budget, ticks, stop);
    *ticks = work;
    if xors.is_empty() || work > budget {
        return;
    }
    let nv = f.vars as usize + 1;
    // occurrences of every variable over the whole formula, and inside the XOR clauses
    let mut total: Vec<u32> = vec![0; nv];
    for i in 0..f.len() {
        if i % 4096 == 0 {
            *ticks = work;
            if work > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
        }
        if f.taut[i] {
            continue;
        }
        work += f.clause(i).len() as u64 + 1;
        for &l in f.clause(i) {
            total[l.unsigned_abs() as usize] += 1;
        }
    }
    *ticks = work;
    let mut in_xor: Vec<u32> = vec![0; nv];
    let mut constraints_of: Vec<Vec<u32>> = vec![Vec::new(); nv];
    for (xi, x) in xors.iter().enumerate() {
        for &v in &x.vars {
            in_xor[v as usize] += x.clauses.len() as u32;
            constraints_of[v as usize].push(xi as u32);
        }
        work += x.vars.len() as u64;
        if xi % 4096 == 0 {
            *ticks = work;
            if work > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
        }
    }
    // a constraint is usable when every variable of it occurs only in the
    // clauses of usable constraints (closure against the whole formula: a
    // dropped constraint's clauses stay in the input) and in exactly two of
    // them; counts per variable are kept and a dropped constraint puts its
    // neighbours on a worklist, so the work is linear in the system's size
    let _ = in_xor;
    let mut live: Vec<u32> = vec![0; nv];       // usable constraints holding the variable
    let mut occ_live: Vec<u32> = vec![0; nv];   // their clauses holding it
    for (xi, x) in xors.iter().enumerate() {
        for &v in &x.vars {
            live[v as usize] += 1;
            occ_live[v as usize] += x.clauses.len() as u32;
        }
        work += x.vars.len() as u64;
        if xi % 4096 == 0 {
            *ticks = work;
            if work > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
        }
    }
    // a variable turns bad when it is not in exactly two usable constraints
    // or occurs outside their clauses; a bad variable drops every usable
    // constraint holding it, which may turn their other variables bad;
    // every constraint is dropped at most once and every variable's list
    // is walked once when it turns bad
    let mut usable: Vec<bool> = vec![true; xors.len()];
    let mut bad: Vec<bool> = vec![false; nv];
    let mut bad_list: Vec<i32> = Vec::new();
    for v in 1..nv {
        if !constraints_of[v].is_empty() && (live[v] != 2 || occ_live[v] != total[v]) {
            bad[v] = true;
            bad_list.push(v as i32);
        }
        if v % 4096 == 0 {
            work += 4096;
            *ticks = work;
            if work > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
        }
    }
    while let Some(v) = bad_list.pop() {
        for ci in 0..constraints_of[v as usize].len() {
            let c = constraints_of[v as usize][ci] as usize;
            if !usable[c] {
                continue;
            }
            usable[c] = false;
            work += xors[c].vars.len() as u64 + 1;
            for &u in &xors[c].vars {
                live[u as usize] -= 1;
                occ_live[u as usize] -= xors[c].clauses.len() as u32;
                if !bad[u as usize] && (live[u as usize] != 2 || occ_live[u as usize] != total[u as usize]) {
                    bad[u as usize] = true;
                    bad_list.push(u);
                }
            }
        }
        *ticks = work;
        if work > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
    }
    *ticks = work;
    let system: Vec<usize> = (0..xors.len()).filter(|&i| usable[i]).collect();
    if system.is_empty() || work > budget {
        *ticks = work;
        return;
    }
    // the graph: vertices are usable constraints, edges their shared variables;
    // a spanning forest by search, and for every non-tree edge the cycle it
    // closes (the edge plus the tree path between its ends)
    let n = xors.len();
    let mut parent: Vec<Option<(usize, i32)>> = vec![None; n]; // (parent constraint, edge variable)
    let mut depth: Vec<u32> = vec![0; n];
    let mut visited: Vec<bool> = vec![false; n];
    let mut tree_edge: Vec<bool> = vec![false; nv];
    for &root in &system {
        if visited[root] {
            continue;
        }
        visited[root] = true;
        // breadth first: the tree is shallow, so the cycles stay short
        let mut queue = std::collections::VecDeque::from(vec![root]);
        while let Some(c) = queue.pop_front() {
            for &v in &xors[c].vars {
                let other = constraints_of[v as usize].iter().copied().find(|&o| o as usize != c && usable[o as usize]);
                let Some(o) = other else { continue };
                let o = o as usize;
                if !visited[o] {
                    visited[o] = true;
                    parent[o] = Some((c, v));
                    depth[o] = depth[c] + 1;
                    tree_edge[v as usize] = true;
                    queue.push_back(o);
                }
                work += 1;
            }
            *ticks = work;
            if work > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
        }
    }
    *ticks = work;
    let mut fixed = 0usize;
    let mut out_of_budget = false;
    'constraints: for &c in &system {
        for &v in &xors[c].vars {
            if tree_edge[v as usize] {
                continue;
            }
            // handle each non-tree edge once, from its lower-numbered end
            let o = constraints_of[v as usize].iter().copied().find(|&o| o as usize != c && usable[o as usize]).map(|o| o as usize);
            let Some(o) = o else { continue };
            if o < c {
                continue;
            }
            // the cycle: v plus the tree path c .. o
            let mut cycle: Vec<i32> = vec![v];
            let (mut a, mut b) = (c, o);
            while depth[a] > depth[b] {
                let (p, e) = parent[a].unwrap();
                cycle.push(e);
                a = p;
            }
            while depth[b] > depth[a] {
                let (p, e) = parent[b].unwrap();
                cycle.push(e);
                b = p;
            }
            while a != b {
                let (pa, ea) = parent[a].unwrap();
                let (pb, eb) = parent[b].unwrap();
                cycle.push(ea);
                cycle.push(eb);
                a = pa;
                b = pb;
            }
            work += cycle.len() as u64;
            *ticks = work;
            if work > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
                out_of_budget = true;
                break 'constraints;
            }
            let witness: Vec<(i32, i32)> = cycle.iter().map(|&x| (x, -x)).collect();
            let id = match derived.proof.as_mut() {
                Some(w) => w.red(&[v], &witness).unwrap_or(0),
                None => 0,
            };
            derived.add(&[v], id);
            fixed += 1;
        }
    }
    if out_of_budget {
        // the fixed edges so far are sound on their own; no propagation
        *ticks = work;
        derived.notes.push(format!("parity: {} XOR constraints, {} in a closed system, {} edges fixed, out of budget", xors.len(), system.len(), fixed));
        return;
    }
    // then unit propagation over the whole formula from the fixed edges
    let nl = 2 * nv;
    let mut occ: Vec<Vec<u32>> = vec![Vec::new(); nl];
    for i in 0..f.len() {
        if i % 4096 == 0 {
            *ticks = work;
            if work > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
                // the fixed edges are sound on their own; no propagation
                derived.notes.push(format!("parity: {} XOR constraints, {} in a closed system, {} edges fixed, out of budget", xors.len(), system.len(), fixed));
                return;
            }
        }
        if f.taut[i] {
            continue;
        }
        work += f.clause(i).len() as u64 + 1;
        for &l in f.clause(i) {
            occ[idx(l)].push(i as u32);
        }
    }
    *ticks = work;
    let mut p = Prop { f, occ, value: vec![0; nv], satisfied: f.taut.clone(), queued: vec![false; nl], work, budget, propagated: 0, ticks: ticks as *mut u64, stop };
    let units: Vec<i32> = derived.clauses.iter().filter(|c| c.len() == 1).map(|c| c[0]).collect();
    for u in units {
        if derived.refuted || p.work > budget {
            break;
        }
        if !p.assign_and_propagate(u, derived) {
            break;
        }
    }
    *ticks = p.work;
    derived.notes.push(format!(
        "parity: {} XOR constraints, {} in a closed system, {} edge{} fixed by cycle flips, {} propagated{}",
        xors.len(),
        system.len(),
        fixed,
        if fixed == 1 { "" } else { "s" },
        p.propagated,
        if derived.refuted { ", refuted" } else { "" }
    ));
}
