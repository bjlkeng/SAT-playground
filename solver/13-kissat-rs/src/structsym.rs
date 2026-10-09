//! Not in kissat. The symmetry pass of the structure pass (plan section 11,
//! 2026-10-07; bead SAT-playground-1v2.3, step one): interchangeable rows
//! and fixing.
//!
//! The idea, in plain terms. Many crafted formulas have a matrix of
//! literals whose rows can be swapped without changing the formula
//! (pigeons, teams, cliques, colours). When a clause has all its literals in
//! one column of such a matrix, one per row, any model satisfies one of
//! them, and swapping that row to the top gives a model satisfying the top
//! one: so the top literal can be fixed true without losing
//! satisfiability. The fixed literal propagates, the leftover rows are
//! still interchangeable, and the step repeats. On a pigeonhole formula the
//! steps run out of holes and the empty clause follows.
//!
//! Detection works on literals (a competition cell often has flipped
//! polarities, so a symmetry may map x to not-y): colour refinement on the
//! clause-literal graph gives classes of literals that look alike;
//! individualizing one literal and refining again shows which literals
//! depend on it, and those are its row. A candidate matrix is accepted
//! only after every row swap was applied to every clause and found to map
//! the formula onto itself, so the refinement only guesses and can never
//! produce a wrong symmetry.
//!
//! Proof: each fixing is logged as Satsuma logs it, one redundance step
//! `red (l* v ~l_i) : swap(row l*, row l_i)` per other literal of the
//! clause, then `rup l*`, then the units that propagate as `rup` steps.
//! VeriPB checks every proof goal by unit propagation.

use crate::structure::{Derived, Formula};
use std::collections::HashMap;

/// One interchangeable-row matrix: `rows[i]` lists the literals of row i
/// in column order, every row the same length.
struct Matrix {
    rows: Vec<Vec<i32>>,
}

/// Literal index: 2v for v, 2v+1 for not v.
#[inline]
fn idx(l: i32) -> usize {
    (l.unsigned_abs() as usize) * 2 + (l < 0) as usize
}

#[inline]
fn lit_of(i: usize) -> i32 {
    let v = (i / 2) as i32;
    if i & 1 == 1 {
        -v
    } else {
        v
    }
}

struct Work<'a> {
    f: &'a Formula,
    /// Clauses containing each literal.
    occ: Vec<Vec<u32>>,
    /// Current assignment per variable: 0 unassigned, else the value.
    value: Vec<i8>,
    /// A clause satisfied by the assignment is ignored.
    satisfied: Vec<bool>,
    /// Canonical forms of the active clauses (for symmetry verification).
    clause_set: HashMap<Vec<i32>, u32>,
    /// Work so far, counted live into the solver's tick counter.
    work: u64,
    /// The tick count the pass must not exceed.
    budget: u64,
    /// The solver's termination flag (a time limit or a signal).
    stop: &'a std::sync::atomic::AtomicBool,
    /// The solver's tick counter, updated at every budget check.
    ticks: *mut u64,
    /// The literals fixed so far (true literals).
    trail: Vec<i32>,
    /// Propagation marks, kept across calls and reset per literal queued.
    queued: Vec<bool>,
}

const MAX_CLASSES: usize = 6;
const MAX_ROWS: usize = 256;

fn debug() -> bool {
    std::env::var_os("SAT_STRUCT_DEBUG").is_some()
}

fn mix(mut h: u64) -> u64 {
    h ^= h >> 32;
    h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^ (h >> 32)
}

impl<'a> Work<'a> {
    fn new(f: &'a Formula, budget: u64, ticks: *mut u64, stop: &'a std::sync::atomic::AtomicBool) -> Work<'a> {
        let work = unsafe { *ticks };
        let nl = 2 * (f.vars as usize + 1);
        let mut occ: Vec<Vec<u32>> = vec![Vec::new(); nl];
        for i in 0..f.len() {
            for &l in f.clause(i) {
                occ[idx(l)].push(i as u32);
            }
        }
        let mut w = Work {
            f,
            occ,
            value: vec![0; f.vars as usize + 1],
            satisfied: vec![false; f.len()],
            clause_set: HashMap::new(),
            work,
            budget,
            stop,
            ticks,
            trail: Vec::new(),
            queued: vec![false; nl],
        };
        for i in 0..f.len() {
            if f.taut[i] {
                w.satisfied[i] = true;
            }
        }
        w
    }

    fn over(&mut self) -> bool {
        // publish the work so far (a signal handler reads the tick counter)
        // SAFETY: `ticks` points at the solver's counter, which outlives the pass
        unsafe { *self.ticks = self.work };
        self.work > self.budget || self.stop.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[inline]
    fn lit_value(&self, l: i32) -> i8 {
        let v = self.value[l.unsigned_abs() as usize];
        if l > 0 {
            v
        } else {
            -v
        }
    }

    /// The active (reduced) form of clause i: false literals dropped.
    fn reduced(&self, i: usize) -> Vec<i32> {
        self.f.clause(i).iter().copied().filter(|&l| self.lit_value(l) == 0).collect()
    }

    /// Returns false when the budget ran out part way (the set is then
    /// incomplete and no verification may use it).
    fn rebuild_clause_set(&mut self) -> bool {
        self.clause_set.clear();
        for i in 0..self.f.len() {
            if i % 1024 == 0 && self.over() {
                self.clause_set.clear();
                return false;
            }
            if self.satisfied[i] {
                continue;
            }
            let mut c = self.reduced(i);
            self.work += c.len() as u64 + 1;
            canon(&mut c);
            self.clause_set.insert(c, i as u32);
        }
        !self.over()
    }

    /// Colour refinement to a fixed point from the given start colours per
    /// literal. Canonical: the same formula and start give the same
    /// colours, and literals that a symmetry exchanges get the same colour.
    fn refine(&mut self, start: &[u32]) -> Vec<u32> {
        let nl = start.len();
        let m = self.f.len();
        let mut cl: Vec<u32> = start.to_vec();
        let mut cc: Vec<u32> = vec![0; m];
        let mut classes = count_classes(&cl);
        loop {
            // clauses: old colour + multiset of literal colours
            let mut hc: Vec<u64> = vec![0; m];
            for i in 0..m {
                if i % 4096 == 0 && self.over() {
                    return cl;
                }
                if self.satisfied[i] {
                    continue;
                }
                let mut h: u64 = mix(cc[i] as u64 + 1);
                let mut len = 0u64;
                for &l in self.f.clause(i) {
                    if self.lit_value(l) != 0 {
                        continue;
                    }
                    len += 1;
                    h = h.wrapping_add(mix(cl[idx(l)] as u64 + 7));
                }
                hc[i] = mix(h ^ mix(len + 3));
                self.work += len + 1;
            }
            cc = relabel(&hc);
            // literals: own colour, the negation's colour, multiset of clause colours
            let mut hl: Vec<u64> = vec![0; nl];
            for li in 2..nl {
                if li % 4096 == 0 {
                    self.work += 4096; // the scan itself, sparse numbering included
                    if self.over() {
                        return cl;
                    }
                }
                let l = lit_of(li);
                if self.lit_value(l) != 0 {
                    continue;
                }
                let mut h: u64 = mix(cl[li] as u64 + 11).wrapping_add(mix(cl[li ^ 1] as u64 + 101) << 1);
                for &i in &self.occ[li] {
                    if self.satisfied[i as usize] {
                        continue;
                    }
                    h = h.wrapping_add(mix(cc[i as usize] as u64 + 5));
                }
                hl[li] = mix(h);
                self.work += self.occ[li].len() as u64 + 1;
            }
            let next = relabel(&hl);
            let k = count_classes(&next);
            cl = next;
            if k == classes || self.over() {
                return cl;
            }
            classes = k;
        }
    }

    /// Is swapping rows a and b (position-wise, as literals) a symmetry of
    /// the reduced formula? Only clauses touching a moved variable are
    /// checked; the rest map to themselves.
    fn verify_swap(&mut self, a: &[i32], b: &[i32]) -> bool {
        let Some(map) = swap_map(a, b, &self.value) else { return false };
        let mut seen_clause: HashMap<u32, ()> = HashMap::new();
        for &l in a.iter().chain(b.iter()) {
            for li in [idx(l), idx(-l)] {
                for oi in 0..self.occ[li].len() {
                    let i = self.occ[li][oi];
                    if self.satisfied[i as usize] || seen_clause.contains_key(&i) {
                        continue;
                    }
                    seen_clause.insert(i, ());
                    let mut img: Vec<i32> = Vec::with_capacity(self.f.clause(i as usize).len());
                    for &x in self.f.clause(i as usize) {
                        if self.lit_value(x) != 0 {
                            continue;
                        }
                        img.push(*map.get(&x).unwrap_or(&x));
                    }
                    self.work += img.len() as u64 + 1;
                    canon(&mut img);
                    if !self.clause_set.contains_key(&img) || self.over() {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Assign a literal true and propagate units over the reduced formula,
    /// logging every propagated unit. Returns false on a conflict (the
    /// empty clause was logged).
    fn assign_and_propagate(&mut self, lit: i32, derived: &mut Derived) -> bool {
        let mut queue = vec![lit];
        self.queued[idx(lit)] = true;
        let mut qi = 0;
        let ok = self.propagate_queue(&mut queue, &mut qi, derived);
        for &l in &queue {
            self.queued[idx(l)] = false; // the marks are clean for the next call
        }
        ok
    }

    fn propagate_queue(&mut self, queue: &mut Vec<i32>, qi: &mut usize, derived: &mut Derived) -> bool {
        let qi: &mut usize = qi;
        while *qi < queue.len() {
            let l = queue[*qi];
            *qi += 1;
            let v = l.unsigned_abs() as usize;
            if self.value[v] != 0 {
                if self.lit_value(l) < 0 {
                    let id = derived.proof.as_mut().map(|w| w.rup(&[]).unwrap_or(0)).unwrap_or(0);
                    derived.add(&[], id);
                    return false;
                }
                continue;
            }
            self.value[v] = if l > 0 { 1 } else { -1 };
            self.trail.push(l);
            for &i in &self.occ[idx(l)] {
                self.satisfied[i as usize] = true;
            }
            for oi in 0..self.occ[idx(-l)].len() {
                let i = self.occ[idx(-l)][oi] as usize;
                if self.satisfied[i] {
                    continue;
                }
                self.work += self.f.clause(i).len() as u64 + 1;
                if self.over() {
                    return true; // out of budget: what was derived stays sound
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
                    return false;
                }
                if count == 1 {
                    let u = unassigned.unwrap();
                    if !self.queued[idx(u)] {
                        self.queued[idx(u)] = true;
                        let id = derived.proof.as_mut().map(|w| w.rup(&[u]).unwrap_or(0)).unwrap_or(0);
                        derived.add(&[u], id);
                        queue.push(u);
                    }
                }
            }
        }
        true
    }
}

/// `base` with the given literals given fresh colours (each its own), so a
/// refinement from it converges in a few rounds instead of recomputing the
/// base partition.
fn individualize(base: &[u32], lits: &[i32]) -> Vec<u32> {
    let mut start = base.to_vec();
    let top = base.iter().copied().max().unwrap_or(0);
    for (k, &l) in lits.iter().enumerate() {
        start[idx(l)] = top + 1 + k as u32;
    }
    start
}

/// The literal map of swapping rows a and b, both polarities, or None when
/// the rows are inconsistent (a literal mapped two ways) or an assigned
/// entry would move to a different value.
fn swap_map(a: &[i32], b: &[i32], value: &[i8]) -> Option<HashMap<i32, i32>> {
    let mut map: HashMap<i32, i32> = HashMap::with_capacity(4 * a.len());
    for k in 0..a.len() {
        let (x, y) = (a[k], b[k]);
        if x == y {
            continue; // an entry shared by both rows stays in place
        }
        if x.abs() == y.abs() {
            return None;
        }
        let (vx, vy) = (value[x.unsigned_abs() as usize], value[y.unsigned_abs() as usize]);
        let (sx, sy) = (if x > 0 { vx } else { -vx }, if y > 0 { vy } else { -vy });
        if sx != sy {
            return None;
        }
        for (from, to) in [(x, y), (y, x), (-x, -y), (-y, -x)] {
            match map.insert(from, to) {
                Some(prev) if prev != to => return None,
                _ => {}
            }
        }
    }
    Some(map)
}

fn canon(c: &mut Vec<i32>) {
    c.sort_unstable_by_key(|&l| (l.abs(), l < 0));
}

fn count_classes(c: &[u32]) -> usize {
    let mut v: Vec<u32> = c.to_vec();
    v.sort_unstable();
    v.dedup();
    v.len()
}

/// Canonical colours from hashes: equal hashes get equal colours, colours
/// are ranks of the sorted distinct hashes.
fn relabel(h: &[u64]) -> Vec<u32> {
    let mut distinct: Vec<u64> = h.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    h.iter().map(|x| distinct.binary_search(x).unwrap() as u32).collect()
}

/// Cells of a colouring restricted to unassigned literals: colour -> literals.
fn cells(colours: &[u32], value: &[i8]) -> HashMap<u32, Vec<i32>> {
    let mut m: HashMap<u32, Vec<i32>> = HashMap::new();
    for li in 2..colours.len() {
        let l = lit_of(li);
        if value[l.unsigned_abs() as usize] == 0 {
            m.entry(colours[li]).or_default().push(l);
        }
    }
    m
}

/// Find interchangeable-row matrices in the reduced formula: for the
/// largest literal classes, every way of choosing the anchor cell.
fn find_matrices(w: &mut Work) -> Vec<Matrix> {
    let nl = 2 * w.value.len();
    let mut found: Vec<Matrix> = Vec::new();
    let base = w.refine(&vec![0; nl]);
    let base_cells = cells(&base, &w.value);
    let mut classes: Vec<(u32, Vec<i32>)> = base_cells.into_iter().filter(|(_, vs)| vs.len() >= 2).collect();
    classes.sort_by_key(|(c, vs)| (std::cmp::Reverse(vs.len()), *c));
    let base_size: Vec<usize> = {
        let mut s = vec![0usize; nl];
        for (_, vs) in &classes {
            for &l in vs {
                s[idx(l)] = vs.len();
            }
        }
        s
    };
    if debug() {
        eprintln!("structsym: {} literal classes, sizes {:?}", classes.len(), classes.iter().take(12).map(|(_, v)| v.len()).collect::<Vec<_>>());
    }
    // candidate anchor sets from every class, smallest first
    let mut candidates: Vec<Vec<i32>> = Vec::new();
    for (_, class) in classes.iter().take(MAX_CLASSES) {
        if w.over() {
            break;
        }
        // individualize the first member: the cells its class splits into
        // are the candidate anchor sets (one anchor per row)
        let x = class[0];
        let alpha = w.refine(&individualize(&base, &[x]));
        let mut by_colour: HashMap<u32, Vec<i32>> = HashMap::new();
        for &l in class.iter() {
            if l != x && l != -x {
                by_colour.entry(alpha[idx(l)]).or_default().push(l);
            }
        }
        let mut cells_here: Vec<Vec<i32>> = by_colour.into_values().collect();
        cells_here.sort_by_key(|c| (c.len(), c[0].abs(), c[0] < 0)); // cells are disjoint: a total order
        if debug() {
            eprintln!("structsym: class of {} literals, anchor {} splits the rest into cells {:?}", class.len(), x, cells_here.iter().map(|c| c.len()).collect::<Vec<_>>());
        }
        for rest in cells_here.into_iter().take(3) {
            if rest.len() + 1 > MAX_ROWS || rest.is_empty() {
                continue;
            }
            let mut anchors = vec![x];
            anchors.extend(rest.iter().copied());
            anchors.sort_unstable_by_key(|l| (l.abs(), *l < 0));
            candidates.push(anchors);
        }
    }
    candidates.sort_by_key(|c| c.len());
    let mut tried: Vec<Vec<i32>> = Vec::new();
    for anchors in candidates {
        if w.over() {
            break;
        }
        // the same anchor set by negation is the same matrix
        let neg: Vec<i32> = {
            let mut n: Vec<i32> = anchors.iter().map(|l| -l).collect();
            n.sort_unstable_by_key(|l| (l.abs(), *l < 0));
            n
        };
        if tried.contains(&anchors) || tried.contains(&neg) {
            continue;
        }
        tried.push(anchors.clone());
        if let Some(m) = rows_from_anchors(w, &anchors, &base, &base_size) {
            found.push(m);
        }
    }
    found
}

/// Build the rows anchored at `anchors` (one literal per row), pair the
/// entries column by column, verify every row swap. None when anything
/// fails. Row i is the set of literals whose colour after individualizing
/// anchor i differs from their colour after individualizing two other
/// anchors (a literal outside rows i, j, k keeps its colour across the
/// three, a literal in row i does not); with two anchors the old
/// equal-parts rule splits their union.
fn rows_from_anchors(w: &mut Work, anchors: &[i32], base: &[u32], base_size: &[usize]) -> Option<Matrix> {
    let nl = 2 * w.value.len();
    let r = anchors.len();
    let mut is_anchor = vec![false; nl];
    for &a in anchors {
        is_anchor[idx(a)] = true;
        is_anchor[idx(-a)] = true;
    }
    let mut alphas: Vec<Vec<u32>> = Vec::with_capacity(r);
    for &a in anchors {
        alphas.push(w.refine(&individualize(base, &[a])));
        if w.over() {
            return None;
        }
    }
    let mut anchors_in_class: HashMap<u32, usize> = HashMap::new();
    for &a in anchors {
        *anchors_in_class.entry(base[idx(a)]).or_default() += 1;
        *anchors_in_class.entry(base[idx(-a)]).or_default() += 1;
    }
    // row i as cells keyed by the colour under its own individualization
    let mut rows: Vec<Vec<(u32, Vec<i32>)>> = Vec::with_capacity(r);
    for i in 0..r {
        let mut by: HashMap<u32, Vec<i32>> = HashMap::new();
        // compare with up to four other anchors; a literal of row i differs
        // from all of them, or from all but one when it is shared with that
        // one's row (an edge variable belongs to both endpoints' rows); a
        // literal of two other rows differs from at most two
        let others: Vec<usize> = (1..r.min(5)).map(|d| (i + d) % r).collect();
        let need = if others.len() >= 4 { others.len() - 1 } else { others.len() };
        for li in 2..nl {
            if li % 4096 == 0 {
                w.work += 4096;
                if w.over() {
                    return None;
                }
            }
            let l = lit_of(li);
            if w.value[l.unsigned_abs() as usize] != 0 || is_anchor[li] || base_size[li] == 0 {
                continue;
            }
            let c = alphas[i][li];
            let differs = others.iter().filter(|&&o| alphas[o][li] != c).count();
            if differs >= need {
                by.entry(c).or_default().push(l);
            }
        }
        let mut row: Vec<(u32, Vec<i32>)> = Vec::new();
        for (c, vs) in by {
            if r == 2 {
                // the union of both rows: keep the cells that are half of their class
                let in_class = base_size[idx(vs[0])];
                let others = anchors_in_class.get(&base[idx(vs[0])]).copied().unwrap_or(0);
                if !(in_class >= others && (in_class - others) == vs.len() * 2) {
                    continue;
                }
            }
            row.push((c, vs));
        }
        row.sort_by_key(|(c, _)| *c);
        rows.push(row);
    }
    let shape: Vec<(u32, usize)> = rows[0].iter().map(|(c, vs)| (*c, vs.len())).collect();
    if debug() {
        eprintln!("structsym:   {} anchors, row 0 cells {:?}", r, shape.iter().map(|s| s.1).collect::<Vec<_>>());
    }
    if shape.is_empty() {
        return None;
    }
    for row in &rows {
        let s: Vec<(u32, usize)> = row.iter().map(|(c, vs)| (*c, vs.len())).collect();
        if s != shape {
            if debug() {
                eprintln!("structsym:   a row has another shape {:?}", s.iter().map(|s| s.1).collect::<Vec<_>>());
            }
            return None;
        }
    }
    // pair the entries of each cell across rows: a singleton pairs itself,
    // a bigger cell by binary clauses between the rows when that is unique,
    // else by one refinement per entry u of row 0 with anchor 0 and u
    // individualized: in every other row the partner is the entry that this
    // singles out (a singleton in its cell now, not before); an entry shared
    // with row 0 pairs with itself
    let mut matrix: Vec<Vec<i32>> = anchors.iter().map(|&a| vec![a]).collect();
    let beta = alphas[0].clone();
    for ci in 0..shape.len() {
        let size = shape[ci].1;
        if size == 1 {
            for (ri, row) in rows.iter().enumerate() {
                matrix[ri].push(row[ci].1[0]);
            }
            continue;
        }
        let first: Vec<i32> = rows[0][ci].1.clone();
        matrix[0].extend(first.iter().copied());
        let mut paired: Vec<Vec<i32>> = vec![Vec::with_capacity(size); r];
        for &u in &first {
            // binary-clause pairing in every row
            let mut partners: Vec<Option<i32>> = (1..r).map(|ri| pair_by_binary(w, u, &rows[ri][ci].1)).collect();
            if partners.iter().any(|p| p.is_none()) {
                let gamma = w.refine(&individualize(base, &[anchors[0], u]));
                if w.over() {
                    return None;
                }
                for ri in 1..r {
                    if partners[ri - 1].is_some() {
                        continue;
                    }
                    let cell = &rows[ri][ci].1;
                    if cell.contains(&u) {
                        partners[ri - 1] = Some(u);
                        continue;
                    }
                    let mut before: HashMap<u32, usize> = HashMap::new();
                    let mut now: HashMap<u32, usize> = HashMap::new();
                    for &v in cell {
                        *before.entry(beta[idx(v)]).or_default() += 1;
                        *now.entry(gamma[idx(v)]).or_default() += 1;
                    }
                    let mut single: Option<i32> = None;
                    for &v in cell {
                        if now[&gamma[idx(v)]] == 1 && before[&beta[idx(v)]] > 1 {
                            if single.is_some() {
                                single = None;
                                break;
                            }
                            single = Some(v);
                        }
                    }
                    partners[ri - 1] = single;
                }
            }
            for ri in 1..r {
                match partners[ri - 1] {
                    Some(v) if !paired[ri].contains(&v) => paired[ri].push(v),
                    other => {
                        if debug() {
                            eprintln!("structsym:   pairing failed for {} in row {} ({:?})", u, ri, other);
                        }
                        return None;
                    }
                }
            }
            if w.over() {
                return None;
            }
        }
        for ri in 1..r {
            matrix[ri].extend(paired[ri].iter().copied());
        }
    }
    // verify every swap with row 0 (these generate all row permutations)
    for ri in 1..r {
        let (a, b) = (matrix[0].clone(), matrix[ri].clone());
        if !w.verify_swap(&a, &b) {
            if debug() {
                eprintln!("structsym:   swap 0 <-> {} is not a symmetry", ri);
            }
            return None;
        }
    }
    if debug() {
        eprintln!("structsym:   matrix {} x {} verified", r, matrix[0].len());
    }
    Some(Matrix { rows: matrix })
}

/// The entry of `cell` (in another row) sharing a binary clause with
/// literal `u` at the same polarity, when that entry is unique.
fn pair_by_binary(w: &mut Work, u: i32, cell: &[i32]) -> Option<i32> {
    let mut hit: Option<i32> = None;
    let mut hits = 0;
    for li in [idx(u), idx(-u)] {
        let sign = if li == idx(u) { 1 } else { -1 };
        for &i in &w.occ[li] {
            if w.satisfied[i as usize] {
                continue;
            }
            let c = w.f.clause(i as usize);
            w.work += c.len() as u64;
            let active: Vec<i32> = c.iter().copied().filter(|&l| w.lit_value(l) == 0).collect();
            if active.len() != 2 {
                continue;
            }
            let other = if active[0].abs() == u.abs() { active[1] } else { active[0] };
            let v = other * sign;
            if cell.contains(&v) && hit != Some(v) {
                hit = Some(v);
                hits += 1;
            }
        }
    }
    if hits == 1 {
        hit
    } else {
        None
    }
}

/// Positions of the matrix entries: literal index -> (row, column).
fn positions(m: &Matrix, nl: usize) -> Vec<(u32, u32)> {
    let mut pos: Vec<(u32, u32)> = vec![(u32::MAX, u32::MAX); nl];
    let mut count: Vec<u8> = vec![0; nl];
    for (ri, row) in m.rows.iter().enumerate() {
        for (ci, &l) in row.iter().enumerate() {
            pos[idx(l)] = (ri as u32, ci as u32);
            count[idx(l)] = count[idx(l)].saturating_add(1);
        }
    }
    // an entry shared by two rows has no single position
    for li in 0..nl {
        if count[li] > 1 {
            pos[li] = (u32::MAX, u32::MAX);
        }
    }
    pos
}

/// The longest active clause whose literals all lie in one column of the
/// matrix, one per row: (clause index, its active literals).
fn best_column_clause(w: &mut Work, pos: &[(u32, u32)]) -> Option<(usize, Vec<i32>)> {
    let mut best: Option<(usize, Vec<i32>)> = None;
    for i in 0..w.f.len() {
        if i % 1024 == 0 && w.over() {
            break;
        }
        if w.satisfied[i] {
            continue;
        }
        let c = w.reduced(i);
        w.work += c.len() as u64 + 1;
        if c.len() < 2 {
            continue;
        }
        let col = pos[idx(c[0])].1;
        if col == u32::MAX {
            continue;
        }
        let mut rows_seen: Vec<u32> = Vec::with_capacity(c.len());
        let mut ok = true;
        for &l in &c {
            let (r, k) = pos[idx(l)];
            if k != col || rows_seen.contains(&r) {
                ok = false;
                break;
            }
            rows_seen.push(r);
        }
        if ok && best.as_ref().map_or(true, |(_, b)| c.len() > b.len()) {
            best = Some((i, c));
        }
        if w.over() {
            break;
        }
    }
    best
}

/// The literal map of the symmetry exchanging rows a and b of the matrix:
/// the verified swap when one of them is row 0, else the composition
/// swap(0,a) swap(0,b) swap(0,a) of verified swaps, which is a symmetry
/// whatever entries the rows share.
fn row_exchange(m: &Matrix, a: usize, b: usize, value: &[i8]) -> Option<HashMap<i32, i32>> {
    if a == 0 || b == 0 {
        return swap_map(&m.rows[a], &m.rows[b], value);
    }
    let sa = swap_map(&m.rows[0], &m.rows[a], value)?;
    let sb = swap_map(&m.rows[0], &m.rows[b], value)?;
    let apply = |map: &HashMap<i32, i32>, l: i32| *map.get(&l).unwrap_or(&l);
    let mut out: HashMap<i32, i32> = HashMap::new();
    for &l in sa.keys().chain(sb.keys()) {
        let img = apply(&sa, apply(&sb, apply(&sa, l)));
        if img != l {
            out.insert(l, img);
        }
    }
    Some(out)
}

/// Fix the lowest-row literal of a column clause through the matrix:
/// redundance steps for the other literals, the unit by propagation.
/// Returns false on a conflict, or when a witness could not be built.
fn fix_clause(w: &mut Work, m: &Matrix, pos: &[(u32, u32)], clause: &[i32], derived: &mut Derived) -> bool {
    let mut lits = clause.to_vec();
    lits.sort_by_key(|&l| pos[idx(l)].0);
    let target = lits[0];
    let trow = pos[idx(target)].0 as usize;
    for &l in &lits[1..] {
        let lrow = pos[idx(l)].0 as usize;
        let Some(map) = row_exchange(m, trow, lrow, &w.value) else { return false };
        if *map.get(&l).unwrap_or(&l) != target {
            return false; // the exchange does not carry l to the target
        }
        let mut witness: Vec<(i32, i32)> = Vec::with_capacity(map.len());
        for (&from, &to) in &map {
            if from > 0 {
                // the map holds both polarities; one entry per variable
                witness.push((from, to));
            }
        }
        witness.sort_unstable();
        if let Some(pw) = derived.proof.as_mut() {
            let _ = pw.red(&[target, -l], &witness);
        }
    }
    let id = derived.proof.as_mut().map(|pw| pw.rup(&[target]).unwrap_or(0)).unwrap_or(0);
    derived.add(&[target], id);
    w.assign_and_propagate(target, derived)
}

/// Matrices with the same variables in their first row are the same symmetry.
fn dedupe(ms: Vec<Matrix>) -> Vec<Matrix> {
    let mut out: Vec<Matrix> = Vec::new();
    let mut keys: Vec<Vec<i32>> = Vec::new();
    for m in ms {
        let mut key: Vec<i32> = m.rows.iter().flat_map(|r| r.iter().map(|l| l.abs())).collect();
        key.sort_unstable();
        key.dedup();
        if keys.contains(&key) {
            continue;
        }
        keys.push(key);
        out.push(m);
    }
    out
}

/// Drop the columns the assignment has used up and the rows that no longer
/// swap with the others; the kept rows are verified pairwise against one
/// reference row (a second reference is tried when the first is the odd
/// one out).
fn shrink(m: &Matrix, w: &mut Work) -> Option<Matrix> {
    let cols = m.rows[0].len();
    let unassigned = |l: i32, w: &Work| w.value[l.unsigned_abs() as usize] == 0;
    let live_cols: Vec<usize> = (0..cols).filter(|&k| m.rows.iter().any(|r| unassigned(r[k], w))).collect();
    if live_cols.is_empty() {
        return None;
    }
    let rows: Vec<Vec<i32>> = m
        .rows
        .iter()
        .map(|r| live_cols.iter().map(|&k| r[k]).collect::<Vec<i32>>())
        .filter(|row| row.iter().any(|&l| unassigned(l, w)))
        .collect();
    if rows.len() < 2 {
        return None;
    }
    for reference in 0..rows.len().min(2) {
        let mut kept: Vec<Vec<i32>> = vec![rows[reference].clone()];
        for (ri, row) in rows.iter().enumerate() {
            if ri == reference {
                continue;
            }
            if w.verify_swap(&rows[reference], row) {
                kept.push(row.clone());
            }
            if w.over() {
                return None;
            }
        }
        if kept.len() >= 2 {
            return Some(Matrix { rows: kept });
        }
    }
    None
}

/// `ticks` is the solver's tick counter: the pass's work is added to it as
/// it happens, and `budget` is the tick count it may reach.
pub fn run(f: &Formula, derived: &mut Derived, budget: u64, ticks: &mut u64, stop: &std::sync::atomic::AtomicBool) {
    let mut w = Work::new(f, budget, ticks as *mut u64, stop);
    // the units an earlier pass derived come first (they are logged already;
    // what they propagate is logged here), so every symmetry this pass
    // uses is a symmetry of the formula with those units, as the proof
    // checker will require
    let earlier: Vec<i32> = derived.clauses.iter().filter(|c| c.len() == 1).map(|c| c[0]).collect();
    for u in earlier {
        if derived.refuted || w.over() {
            break;
        }
        if !w.assign_and_propagate(u, derived) {
            break;
        }
    }
    // the formula's own units next, so the rows are detected on the reduced
    // formula; an empty clause in the input ends the pass at once
    for i in 0..f.len() {
        if w.satisfied[i] || derived.refuted || w.over() {
            continue;
        }
        let c = w.reduced(i);
        w.work += c.len() as u64 + 1;
        if c.is_empty() {
            let id = derived.proof.as_mut().map(|pw| pw.rup(&[]).unwrap_or(0)).unwrap_or(0);
            derived.add(&[], id);
            break;
        }
        if c.len() == 1 && !w.assign_and_propagate(c[0], derived) {
            break;
        }
    }
    if derived.refuted || w.over() || !w.rebuild_clause_set() {
        *ticks = w.work;
        return;
    }
    let mut fixed = 0usize;
    let mut matrices = 0usize;
    let nl = 2 * w.value.len();
    let mut pool: Vec<Matrix> = Vec::new();
    let mut detections = 0usize;
    let mut fixed_at_detection = usize::MAX; // fixes when the last detection ran
    loop {
        if w.over() || derived.refuted {
            break;
        }
        if pool.is_empty() {
            if fixed_at_detection == fixed {
                break; // nothing was fixed since the last detection: it would find the same
            }
            fixed_at_detection = fixed;
            detections += 1;
            let before = w.work;
            pool = dedupe(find_matrices(&mut w));
            matrices += pool.len();
            if debug() {
                eprintln!("structsym: detection {} found {} matrices ({:?} rows) for {}K work, {} fixed so far", detections, pool.len(), pool.iter().map(|m| m.rows.len()).collect::<Vec<_>>(), (w.work - before) / 1000, fixed);
            }
            if pool.is_empty() {
                break;
            }
        }
        // the longest column clause over every matrix
        let mut choice: Option<(usize, Vec<i32>)> = None;
        for (mi, m) in pool.iter().enumerate() {
            let pos = positions(m, nl);
            if let Some((_, c)) = best_column_clause(&mut w, &pos) {
                if choice.as_ref().map_or(true, |(_, b)| c.len() > b.len()) {
                    choice = Some((mi, c));
                }
            }
            if w.over() {
                break;
            }
        }
        let Some((mi, clause)) = choice else {
            pool.clear();
            continue; // nothing to fix through these matrices: detect again, once
        };
        let pos = positions(&pool[mi], nl);
        let before = w.work;
        let ok = fix_clause(&mut w, &pool[mi], &pos, &clause, derived);
        fixed += 1;
        if debug() {
            eprintln!("structsym: fix {} through matrix {} ({} rows) with a {}-literal clause, {}K work, trail {}", fixed, mi, pool[mi].rows.len(), clause.len(), (w.work - before) / 1000, w.trail.len());
        }
        if !ok {
            break;
        }
        if !w.rebuild_clause_set() {
            break;
        }
        // keep the rows of every matrix that stay symmetric
        let old = std::mem::take(&mut pool);
        for m in old {
            if let Some(s) = shrink(&m, &mut w) {
                pool.push(s);
            }
            if w.over() {
                break;
            }
        }
    }
    *ticks = w.work;
    if fixed > 0 || matrices > 0 {
        derived.notes.push(format!(
            "symmetry: {} matri{} with interchangeable rows, {} literal{} fixed, {} propagated{}",
            matrices,
            if matrices == 1 { "x" } else { "ces" },
            fixed,
            if fixed == 1 { "" } else { "s" },
            w.trail.len().saturating_sub(fixed),
            if derived.refuted { ", refuted" } else { "" }
        ));
    }
}
