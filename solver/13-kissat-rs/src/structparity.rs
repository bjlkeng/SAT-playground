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
//! General elimination (sums of constraints that mix with other clauses)
//! is step two of the bead.

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
                out.push(Xor { vars: vars.clone(), clauses: by_parity[parity].clone() });
                break;
            }
        }
    }
    out
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
