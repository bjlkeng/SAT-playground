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
use std::collections::{HashMap, HashSet};

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
    /// The tick count the pass must not exceed; `more` once a matrix has
    /// been verified (a cell with structure is worth more work), which
    /// carries the detection in progress and its fixings to the end, while
    /// a further detection starts only within `base`.
    budget: u64,
    base: u64,
    more: u64,
    extended: bool,
    /// The solver's termination flag (a time limit or a signal).
    stop: &'a std::sync::atomic::AtomicBool,
    /// The solver's tick counter, updated at every budget check.
    ticks: *mut u64,
    /// The literals fixed so far (true literals).
    trail: Vec<i32>,
    /// Propagation marks, kept across calls and reset per literal queued.
    queued: Vec<bool>,
    /// The clauses this pass derived that are not units: a symmetry used
    /// from then on must map them onto the formula too.
    extra: Vec<Vec<i32>>,
    extra_occ: Vec<Vec<u32>>,
}

const MAX_CLASSES: usize = 6;
const MAX_ROWS: usize = 256;
/// A cell of x's class up to this size is sorted by a second anchor.
const SUBCELL_MAX: usize = 64;

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
    fn new(f: &'a Formula, budget: u64, more: u64, ticks: *mut u64, stop: &'a std::sync::atomic::AtomicBool) -> Work<'a> {
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
            base: budget,
            more,
            extended: false,
            stop,
            ticks,
            trail: Vec::new(),
            queued: vec![false; nl],
            extra: Vec::new(),
            extra_occ: vec![Vec::new(); nl],
        };
        for i in 0..f.len() {
            if f.taut[i] {
                w.satisfied[i] = true;
            }
        }
        w
    }

    /// Record a derived clause that is not a unit, for every later
    /// symmetry check.
    fn add_extra(&mut self, clause: &[i32]) {
        let k = self.extra.len() as u32;
        for &l in clause {
            self.extra_occ[idx(l)].push(k);
        }
        self.extra.push(clause.to_vec());
    }

    /// The active literals of derived clause k, or None when it is satisfied.
    fn reduced_extra(&self, k: usize) -> Option<Vec<i32>> {
        let mut out: Vec<i32> = Vec::with_capacity(self.extra[k].len());
        for &l in &self.extra[k] {
            match self.lit_value(l) {
                0 => out.push(l),
                v if v > 0 => return None,
                _ => {}
            }
        }
        Some(out)
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
        let m = self.f.len() as u32;
        for k in 0..self.extra.len() {
            if let Some(mut c) = self.reduced_extra(k) {
                self.work += c.len() as u64 + 1;
                canon(&mut c);
                self.clause_set.insert(c, m + k as u32);
            }
        }
        !self.over()
    }

    /// The base colouring: colour refinement to a fixed point from one
    /// class of all active literals and one of all active clauses.
    /// Canonical: the same formula gives the same labels, and literals
    /// that a symmetry exchanges get the same label.
    fn refine_base(&mut self) -> Colouring {
        let nl = 2 * self.value.len();
        let m = self.f.len();
        let clauses: Vec<u32> = (0..m).filter(|&i| !self.satisfied[i]).map(|i| i as u32).collect();
        let lits: Vec<u32> = (2..nl).filter(|&li| self.lit_value(lit_of(li)) == 0).map(|li| li as u32).collect();
        let mut col = Colouring {
            cl: vec![0; nl],
            cc: vec![1; m],
            size: vec![lits.len() as u32, clauses.len() as u32],
            next: 2,
        };
        // every active clause and literal is hashed in the first round:
        // clauses of one length are one class until their literals differ,
        // and literals differ by what they occur in
        self.refine_rounds(&mut col, clauses, lits, Vec::new());
        col
    }

    /// `base` with the given literals each in a fresh class of its own,
    /// refined to a fixed point. Only what the split reaches is re-hashed:
    /// a clause when a literal of it changed class, a literal when a clause
    /// of it or its negation did. The fixed point is the one a refinement
    /// from scratch reaches, at the cost of the part that changes.
    fn refine_from(&mut self, base: &Colouring, lits: &[i32]) -> Vec<u32> {
        let mut col = base.clone();
        let mut changed: Vec<u32> = Vec::with_capacity(lits.len());
        for &l in lits {
            let li = idx(l);
            let old = col.cl[li] as usize;
            if col.size[old] > 0 {
                col.size[old] -= 1;
            }
            col.cl[li] = col.next;
            col.size.push(1);
            col.next += 1;
            changed.push(li as u32);
        }
        self.refine_rounds(&mut col, Vec::new(), Vec::new(), changed);
        col.cl
    }

    /// Rounds of refinement until nothing changes: re-hash the touched
    /// clauses, split their classes, re-hash the touched literals, split
    /// theirs. `clauses` and `lits` are re-hashed in the first round
    /// whatever their neighbours did; `changed` are the literals whose
    /// label changed before. Returns early at the budget (the colouring is
    /// then partial, and the caller checks `over`).
    fn refine_rounds(&mut self, col: &mut Colouring, clauses: Vec<u32>, lits: Vec<u32>, changed: Vec<u32>) {
        let m = self.f.len();
        let nl = col.cl.len();
        let mut cmark: Vec<bool> = vec![false; m];
        let mut lmark: Vec<bool> = vec![false; nl];
        let mut touched_c: Vec<u32> = clauses;
        for &i in &touched_c {
            cmark[i as usize] = true;
        }
        let mut first_lits: Vec<u32> = lits;
        let mut changed_l: Vec<u32> = changed;
        let (w0, mut rounds, mut hashed) = (self.work, 0u32, 0u64);
        loop {
            rounds += 1;
            // the clauses a changed literal is in
            for (n, &li) in changed_l.iter().enumerate() {
                if n % 1024 == 0 && self.over() {
                    return;
                }
                for &i in &self.occ[li as usize] {
                    if !self.satisfied[i as usize] && !cmark[i as usize] {
                        cmark[i as usize] = true;
                        touched_c.push(i);
                    }
                }
                self.work += self.occ[li as usize].len() as u64 + 1;
            }
            // their new hash: the old label and the multiset of literal labels
            let mut keys: Vec<(u32, u64, u32)> = Vec::with_capacity(touched_c.len());
            for (n, &i) in touched_c.iter().enumerate() {
                if n % 4096 == 0 && self.over() {
                    return;
                }
                let i = i as usize;
                let mut h: u64 = mix(col.cc[i] as u64 + 1);
                let mut len = 0u64;
                for &l in self.f.clause(i) {
                    if self.lit_value(l) != 0 {
                        continue;
                    }
                    len += 1;
                    h = h.wrapping_add(mix(col.cl[idx(l)] as u64 + 7));
                }
                self.work += len + 1;
                keys.push((col.cc[i], mix(h ^ mix(len + 3)), i as u32));
            }
            hashed += touched_c.len() as u64;
            for &i in &touched_c {
                cmark[i as usize] = false;
            }
            touched_c.clear();
            self.work += keys.len() as u64;
            let changed_c = split(&mut keys, &mut col.cc, &mut col.size, &mut col.next);
            // the literals of a changed clause, the negations of changed literals
            let mut touched_l: Vec<u32> = std::mem::take(&mut first_lits);
            for &li in &touched_l {
                lmark[li as usize] = true;
            }
            for (n, &i) in changed_c.iter().enumerate() {
                if n % 1024 == 0 && self.over() {
                    return;
                }
                for &l in self.f.clause(i as usize) {
                    if self.lit_value(l) != 0 {
                        continue;
                    }
                    let li = idx(l);
                    if !lmark[li] {
                        lmark[li] = true;
                        touched_l.push(li as u32);
                    }
                }
                self.work += self.f.clause(i as usize).len() as u64 + 1;
            }
            for &li in &changed_l {
                let n = (li ^ 1) as usize;
                if self.lit_value(lit_of(n)) == 0 && !lmark[n] {
                    lmark[n] = true;
                    touched_l.push(n as u32);
                }
            }
            changed_l.clear();
            if touched_l.is_empty() {
                break;
            }
            // their new hash: own label, the negation's, the multiset of clause labels
            let mut keys: Vec<(u32, u64, u32)> = Vec::with_capacity(touched_l.len());
            for (n, &li) in touched_l.iter().enumerate() {
                if n % 4096 == 0 && self.over() {
                    return;
                }
                let li = li as usize;
                let mut h: u64 = mix(col.cl[li] as u64 + 11).wrapping_add(mix(col.cl[li ^ 1] as u64 + 101) << 1);
                for &i in &self.occ[li] {
                    if self.satisfied[i as usize] {
                        continue;
                    }
                    h = h.wrapping_add(mix(col.cc[i as usize] as u64 + 5));
                }
                self.work += self.occ[li].len() as u64 + 1;
                keys.push((col.cl[li], mix(h), li as u32));
            }
            hashed += touched_l.len() as u64;
            for &li in &touched_l {
                lmark[li as usize] = false;
            }
            self.work += keys.len() as u64;
            changed_l = split(&mut keys, &mut col.cl, &mut col.size, &mut col.next);
            if changed_l.is_empty() || self.over() {
                break;
            }
        }
        if debug() {
            eprintln!("structsym: refinement: {} rounds, {} elements hashed, {}K work, {} classes", rounds, hashed, (self.work - w0) / 1000, col.size.iter().filter(|&&s| s > 0).count());
        }
    }

    /// Is swapping rows a and b (position-wise, as literals) a symmetry of
    /// the reduced formula? Only clauses touching a moved variable are
    /// checked; the rest map to themselves.
    fn verify_swap(&mut self, a: &[i32], b: &[i32]) -> bool {
        let Some(map) = swap_map(a, b, &self.value) else {
            if debug() && std::env::var_os("SAT_STRUCT_DEBUG_SWAP").is_some() {
                eprintln!("structsym:     swap fails: no consistent map for rows {:?} and {:?}", &a[..a.len().min(6)], &b[..b.len().min(6)]);
            }
            return false;
        };
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
                        if debug() && std::env::var_os("SAT_STRUCT_DEBUG_SWAP").is_some() {
                            eprintln!("structsym:     swap fails: clause {:?} (reduced) maps to {:?}, not in the set", self.reduced(i as usize), img);
                        }
                        return false;
                    }
                }
                for oi in 0..self.extra_occ[li].len() {
                    let k = self.extra_occ[li][oi] as usize;
                    let key = self.f.len() as u32 + k as u32;
                    if seen_clause.contains_key(&key) {
                        continue;
                    }
                    seen_clause.insert(key, ());
                    let Some(c) = self.reduced_extra(k) else { continue };
                    let mut img: Vec<i32> = c.iter().map(|x| *map.get(x).unwrap_or(x)).collect();
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

/// The members per row, v, of a cell of (R-1)v members under the second
/// anchor y (index `yi`), from the cell sizes within the rest: v-1
/// members in cells of at most v-1, the other (R-2)v in cells of a
/// multiple of R-2, with R >= 4 and v >= 2; the fit with the most rows,
/// or None when no factorization fits.
fn row_mates_per_row(sizes: &[usize], yi: usize) -> Option<usize> {
    let n = sizes.len(); // (R-1)v members, y among them
    let mut best: Option<(usize, usize)> = None; // (R, v)
    for v in 2..=n / 2 {
        if n % v != 0 {
            continue;
        }
        let rows = n / v + 1; // R
        if rows < 4 {
            continue;
        }
        let mut mates = 0usize;
        let mut ok = true;
        for (zi, &sz) in sizes.iter().enumerate() {
            if zi == yi {
                continue;
            }
            if sz <= v - 1 {
                mates += 1;
            } else if sz % (rows - 2) != 0 {
                ok = false;
                break;
            }
        }
        if ok && mates == v - 1 && best.map_or(true, |(r, _)| rows > r) {
            best = Some((rows, v));
        }
    }
    best.map(|(_, v)| v)
}

/// A stable colouring of the reduced formula: a label per literal and per
/// clause (one label space; literals start at 0, clauses at 1), the number
/// of active members of every label, and the next fresh label. Labels
/// only ever split; an element keeps its label until its class splits
/// under it.
#[derive(Clone)]
struct Colouring {
    cl: Vec<u32>,
    cc: Vec<u32>,
    size: Vec<u32>,
    next: u32,
}

/// Split the classes of the re-hashed elements, `keys` = (old label, new
/// hash, element): elements with the same old label and hash stay
/// together. A class whose active members were all re-hashed keeps its
/// label for the fragment with the smallest hash; every other fragment
/// gets a fresh label, in hash order, so the labels are canonical. A
/// re-hashed element never rejoins members that were not re-hashed: it
/// has a neighbour with a label that did not exist when they were
/// labelled, and they do not. Returns the elements whose label changed.
fn split(keys: &mut [(u32, u64, u32)], label: &mut [u32], size: &mut Vec<u32>, next: &mut u32) -> Vec<u32> {
    keys.sort_unstable();
    let mut changed: Vec<u32> = Vec::new();
    let mut s = 0;
    while s < keys.len() {
        let old = keys[s].0;
        let mut e = s;
        while e < keys.len() && keys[e].0 == old {
            e += 1;
        }
        let whole = (e - s) as u32 == size[old as usize];
        let mut fs = s;
        let mut first = true;
        while fs < e {
            let h = keys[fs].1;
            let mut fe = fs;
            while fe < e && keys[fe].1 == h {
                fe += 1;
            }
            if !(whole && first) {
                let fresh = *next;
                *next += 1;
                size.push((fe - fs) as u32);
                size[old as usize] -= (fe - fs) as u32;
                for k in &keys[fs..fe] {
                    label[k.2 as usize] = fresh;
                    changed.push(k.2);
                }
            }
            first = false;
            fs = fe;
        }
        s = e;
    }
    changed
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
    let base_col = w.refine_base();
    let base = base_col.cl.clone();
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
        for (_, v) in classes.iter().take(6) {
            eprintln!("structsym:   class of {}: first members {:?}", v.len(), v.iter().take(8).collect::<Vec<_>>());
        }
    }
    // candidate anchor sets from every class, smallest first; the negation
    // of a class is a class, and its candidate sets are the negations of
    // this one's (deduplicated below), so one of the two is enough. The
    // plain candidates (x and one cell) go first; the cells are kept, and
    // the cells whose plain candidate gave no matrix are then sorted into
    // rows by a second anchor (below), which costs a refinement per row
    let mut candidates: Vec<Vec<i32>> = Vec::new();
    let mut kept: Vec<(i32, Vec<i32>, Vec<i32>)> = Vec::new(); // (x, class, cell)
    let mut done: HashSet<u32> = HashSet::new();
    for (c, class) in classes.iter().take(MAX_CLASSES) {
        if w.over() {
            break;
        }
        if done.contains(&base[idx(-class[0])]) {
            continue;
        }
        done.insert(*c);
        // individualize the first member: the cells its class splits into
        // are the candidate anchor sets (one anchor per row)
        let x = class[0];
        let alpha = w.refine_from(&base_col, &[x]);
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
            if rest.len() >= 2 && rest.len() <= SUBCELL_MAX {
                kept.push((x, class.clone(), rest));
            }
        }
    }
    let mut tried: Vec<Vec<i32>> = Vec::new();
    let worked = try_candidates(w, candidates, &mut tried, &base_col, &base_size, &mut found);
    if !w.over() {
        let mut candidates: Vec<Vec<i32>> = Vec::new();
        for (x, class, rest) in kept {
            if w.over() {
                break;
            }
            // the cell's plain candidate, as tried: a matrix from it needs
            // no sorting
            let mut plain = vec![x];
            plain.extend(rest.iter().copied());
            plain.sort_unstable_by_key(|l| (l.abs(), *l < 0));
            if worked.contains(&plain) {
                continue;
            }
            for members in sort_cell_into_rows(w, &base_col, x, &class, &rest) {
                let mut anchors = vec![x];
                anchors.extend(members.iter().copied());
                anchors.sort_unstable_by_key(|l| (l.abs(), *l < 0));
                candidates.push(anchors);
            }
        }
        let _ = try_candidates(w, candidates, &mut tried, &base_col, &base_size, &mut found);
    }
    found
}

/// Try candidate anchor sets, smallest first, each once (a set and its
/// negation are the same matrix). Returns the sets that gave a matrix
/// (a set tried earlier counts when it did).
fn try_candidates(w: &mut Work, mut candidates: Vec<Vec<i32>>, tried: &mut Vec<Vec<i32>>, base_col: &Colouring, base_size: &[usize], found: &mut Vec<Matrix>) -> Vec<Vec<i32>> {
    candidates.sort_by_key(|c| c.len());
    let mut worked: Vec<Vec<i32>> = Vec::new();
    for anchors in candidates {
        if w.over() {
            break;
        }
        let neg: Vec<i32> = {
            let mut n: Vec<i32> = anchors.iter().map(|l| -l).collect();
            n.sort_unstable_by_key(|l| (l.abs(), *l < 0));
            n
        };
        if tried.contains(&anchors) || tried.contains(&neg) {
            continue;
        }
        tried.push(anchors.clone());
        let (mut matrix, guessed) = rows_from_anchors(w, &anchors, base_col, base_size, false);
        if matrix.is_none() && guessed && !w.over() {
            // a guessed pairing failed: once more, refinement before guesses
            matrix = rows_from_anchors(w, &anchors, base_col, base_size, true).0;
        }
        if let Some(m) = matrix {
            found.push(m);
            worked.push(neg);
            worked.push(anchors);
            if !w.extended {
                // structure: the larger limit, once, for this detection and its fixings
                w.extended = true;
                w.budget = w.budget.max(w.more);
            }
        }
    }
    worked
}

/// Sort a cell of x's images into rows by a second anchor; the groups of
/// members, one anchor per row to be chosen from each, or nothing when
/// the cell stays one group.
fn sort_cell_into_rows(w: &mut Work, base_col: &Colouring, x: i32, class: &[i32], rest: &[i32]) -> Vec<Vec<i32>> {
    // a second symmetry can put several literals of one row into the cell
    // of x's images (a round robin's home/away flip puts both sides of the
    // same game on another day there), and then one anchor per row is one
    // per group of row-mates. The cell holds (R-1)v members, v per row.
    // With x and a member y individualized, y's v-1 row-mates lie in cells
    // of at most v-1 members (nothing moves them to another row), while
    // the other R-2 rows' members lie in cells of a multiple of R-2 (those
    // rows still exchange); the factorization that fits gives y's row.
    // Rows are taken in turn: the first member not yet placed is refined
    // with x, its row is read off, and the next unplaced member follows,
    // so a cell of R-1 rows costs R-1 refinements. Members whose
    // refinement gives x's class another shape are sorted apart as well.
    let mut row_of: Vec<Option<usize>> = vec![None; rest.len()];
    let mut groups: Vec<(Vec<usize>, Vec<i32>)> = Vec::new(); // (shape, representatives)
    let mut rows = 0usize;
    for yi in 0..rest.len() {
        if row_of[yi].is_some() {
            continue;
        }
        let y = rest[yi];
        let gamma = w.refine_from(base_col, &[x, y]);
        if w.over() {
            return Vec::new();
        }
        let mut count: HashMap<u32, usize> = HashMap::new();
        for &l in class.iter() {
            *count.entry(gamma[idx(l)]).or_default() += 1;
        }
        // the size of each member's cell within the rest
        let mut within: HashMap<u32, usize> = HashMap::new();
        for (zi, &z) in rest.iter().enumerate() {
            if zi != yi {
                *within.entry(gamma[idx(z)]).or_default() += 1;
            }
        }
        let sizes: Vec<usize> = rest.iter().map(|&z| within.get(&gamma[idx(z)]).copied().unwrap_or(0)).collect();
        row_of[yi] = Some(rows);
        if let Some(v) = row_mates_per_row(&sizes, yi) {
            for (zi, &sz) in sizes.iter().enumerate() {
                if zi != yi && sz <= v - 1 && row_of[zi].is_none() {
                    row_of[zi] = Some(rows);
                }
            }
        }
        rows += 1;
        let mut shape: Vec<usize> = count.into_values().collect();
        shape.sort_unstable();
        match groups.iter_mut().find(|(g, _)| *g == shape) {
            Some((_, members)) => members.push(y),
            None => groups.push((shape, vec![y])),
        }
    }
    if rows == rest.len() && groups.len() == 1 {
        return Vec::new(); // every member its own row: the plain candidate again
    }
    if debug() {
        eprintln!("structsym:   the cell of {} sorts into {} rows, groups {:?}, under a second anchor", rest.len(), rows, groups.iter().map(|(_, m)| m.len()).collect::<Vec<_>>());
    }
    groups.into_iter().map(|(_, m)| m).collect()
}

/// Build the rows anchored at `anchors` (one literal per row), pair the
/// entries column by column, verify every row swap. None when anything
/// fails. Row i is the set of literals whose colour after individualizing
/// anchor i differs from their colour after individualizing two other
/// anchors (a literal outside rows i, j, k keeps its colour across the
/// three, a literal in row i does not); with two anchors the old
/// equal-parts rule splits their union.
fn rows_from_anchors(w: &mut Work, anchors: &[i32], base_col: &Colouring, base_size: &[usize], careful: bool) -> (Option<Matrix>, bool) {
    let mut guessed = false;
    let base = &base_col.cl;
    let nl = 2 * w.value.len();
    let r = anchors.len();
    let mut is_anchor = vec![false; nl];
    for &a in anchors {
        is_anchor[idx(a)] = true;
        is_anchor[idx(-a)] = true;
    }
    let mut anchors_in_class: HashMap<u32, usize> = HashMap::new();
    for &a in anchors {
        *anchors_in_class.entry(base[idx(a)]).or_default() += 1;
        *anchors_in_class.entry(base[idx(-a)]).or_default() += 1;
    }
    // one refinement per anchor, the first six first: rows 0 and 1 compare
    // with anchors 1-5 only, and when their shapes disagree the rest is
    // not computed
    let mut alphas: Vec<Vec<u32>> = Vec::with_capacity(r);
    let mut rows: Vec<Vec<(u32, Vec<i32>)>> = Vec::with_capacity(r);
    let mut shape: Vec<(u32, usize)> = Vec::new();
    let first = r.min(6);
    for &a in &anchors[..first] {
        alphas.push(w.refine_from(base_col, &[a]));
        if w.over() {
            return (None, guessed);
        }
    }
    // row i as cells keyed by the colour under its own individualization
    for i in 0..r {
        if i == 2 && r > first {
            for &a in &anchors[first..] {
                alphas.push(w.refine_from(base_col, &[a]));
                if w.over() {
                    return (None, guessed);
                }
            }
        }
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
                    return (None, guessed);
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
        let s: Vec<(u32, usize)> = row.iter().map(|(c, vs)| (*c, vs.len())).collect();
        if i == 0 {
            shape = s;
            if debug() {
                eprintln!("structsym:   {} anchors, row 0 cells {:?}", r, shape.iter().map(|s| s.1).collect::<Vec<_>>());
            }
            if shape.is_empty() {
                return (None, guessed);
            }
        } else if s != shape {
            if debug() {
                eprintln!("structsym:   row {} has another shape {:?}", i, s.iter().map(|s| s.1).collect::<Vec<_>>());
            }
            return (None, guessed);
        }
        rows.push(row);
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
        // refinements that singled out none of the link matches: after a
        // few in one cell the ambiguity is a symmetry of its own, and the
        // refinement is not tried again there
        let mut futile = 0usize;
        let (cell_work, mut refinements) = (w.work, 0usize);
        for &u in &first {
            // binary-clause pairing in every row; else the entries sharing
            // any clause with u, told apart by their links to the anchors;
            // several left over go to the refinement below, and only when
            // that singles out none is the smallest not yet paired taken
            let mut partners: Vec<Option<i32>> = (1..r).map(|ri| pair_by_binary(w, u, &rows[ri][ci].1)).collect();
            let mut leftover: Vec<Vec<i32>> = vec![Vec::new(); r];
            for ri in 1..r {
                if partners[ri - 1].is_none() && !rows[ri][ci].1.contains(&u) {
                    match pair_by_clause_and_links(w, u, &rows[ri][ci].1, anchors, ri) {
                        Linked::One(v) => partners[ri - 1] = Some(v),
                        Linked::Several(vs) => leftover[ri] = vs,
                        Linked::None => {}
                    }
                }
                if w.over() {
                    return (None, guessed);
                }
            }
            if partners.iter().any(|p| p.is_none()) {
                // the guess first unless `careful` (a retry after a guessed
                // matrix failed): then the refinement, until it has been
                // futile a few times in this cell
                let only_leftovers = (1..r).all(|ri| partners[ri - 1].is_some() || !leftover[ri].is_empty() || rows[ri][ci].1.contains(&u));
                let refine = !(only_leftovers && (!careful || futile >= 3));
                let gamma = if refine { refinements += 1; w.refine_from(base_col, &[anchors[0], u]) } else { Vec::new() };
                if w.over() {
                    return (None, guessed);
                }
                let mut singled = false;
                for ri in 1..r {
                    if partners[ri - 1].is_some() {
                        continue;
                    }
                    let cell = &rows[ri][ci].1;
                    if cell.contains(&u) {
                        partners[ri - 1] = Some(u);
                        continue;
                    }
                    let mut single: Option<i32> = None;
                    if refine {
                        let mut before: HashMap<u32, usize> = HashMap::new();
                        let mut now: HashMap<u32, usize> = HashMap::new();
                        for &v in cell {
                            *before.entry(beta[idx(v)]).or_default() += 1;
                            *now.entry(gamma[idx(v)]).or_default() += 1;
                        }
                        for &v in cell {
                            if now[&gamma[idx(v)]] == 1 && before[&beta[idx(v)]] > 1 {
                                if single.is_some() {
                                    single = None;
                                    break;
                                }
                                single = Some(v);
                            }
                        }
                        singled |= single.is_some();
                    }
                    if single.is_none() && !leftover[ri].is_empty() {
                        guessed = true;
                        // entries refinement and links cannot tell apart are
                        // usually exchanged by a symmetry of their own, so
                        // any one-to-one choice pairs them, and the swap
                        // verification rejects the matrix when it does not
                        single = leftover[ri].iter().copied().filter(|v| !paired[ri].contains(v)).min_by_key(|&v| (v.abs(), v < 0));
                    }
                    partners[ri - 1] = single;
                }
                if refine && !singled && only_leftovers {
                    futile += 1;
                }
            }
            for ri in 1..r {
                match partners[ri - 1] {
                    Some(v) if !paired[ri].contains(&v) => paired[ri].push(v),
                    other => {
                        if debug() {
                            eprintln!("structsym:   pairing failed for {} in row {} ({:?}); anchors {:?}; the cell {:?}; row 0's {:?}", u, ri, other, &anchors[..anchors.len().min(10)], &rows[ri][ci].1[..rows[ri][ci].1.len().min(8)], &first[..first.len().min(8)]);
                        }
                        return (None, guessed);
                    }
                }
            }
            if w.over() {
                return (None, guessed);
            }
        }
        for ri in 1..r {
            matrix[ri].extend(paired[ri].iter().copied());
        }
        if debug() {
            eprintln!("structsym:   cell {} of {} entries paired: {} refinements, {} futile, {}K work", ci, size, refinements, futile, (w.work - cell_work) / 1000);
        }
    }
    // verify every swap with row 0 (these generate all row permutations)
    for ri in 1..r {
        let (a, b) = (matrix[0].clone(), matrix[ri].clone());
        if !w.verify_swap(&a, &b) {
            if debug() {
                eprintln!("structsym:   swap 0 <-> {} is not a symmetry", ri);
            }
            return (None, guessed);
        }
    }
    if debug() {
        eprintln!("structsym:   matrix {} x {} verified", r, matrix[0].len());
    }
    (Some(Matrix { rows: matrix }), guessed)
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

/// The clauses a literal shares with the anchors, as a sorted list of
/// (length, the literal's polarity, the anchors in the clause as (row,
/// polarity)) with rows `a` and `b` exchanged: what the swap of rows a
/// and b must map a's entry's links onto.
fn anchor_links(w: &mut Work, u: i32, anchors: &[i32], a: usize, b: usize) -> Vec<(usize, bool, Vec<(usize, bool)>)> {
    // an anchor's row, and whether the anchor is the positive literal: a
    // clause's polarity is recorded relative to the anchor's own sign,
    // since the exchange carries the anchor onto the other row's anchor
    // whatever their signs
    let mut row_of: HashMap<i32, (usize, bool)> = HashMap::new();
    for (k, &x) in anchors.iter().enumerate() {
        let k = if k == a { b } else if k == b { a } else { k };
        row_of.insert(x.abs(), (k, x > 0));
    }
    let mut out: Vec<(usize, bool, Vec<(usize, bool)>)> = Vec::new();
    for li in [idx(u), idx(-u)] {
        for &i in &w.occ[li] {
            if w.satisfied[i as usize] {
                continue;
            }
            let c = w.f.clause(i as usize);
            w.work += c.len() as u64;
            let mut hits: Vec<(usize, bool)> = Vec::new();
            let mut len = 0;
            for &l in c {
                if w.lit_value(l) != 0 {
                    continue;
                }
                len += 1;
                if let Some(&(k, positive)) = row_of.get(&l.abs()) {
                    hits.push((k, (l > 0) == positive));
                }
            }
            if !hits.is_empty() {
                hits.sort_unstable();
                out.push((len, li == idx(u), hits));
            }
        }
    }
    out.sort_unstable();
    out
}

enum Linked {
    One(i32),
    Several(Vec<i32>),
    None,
}

/// The entries of `cell` (row `ri`) that share a clause of any length with
/// `u` at the same polarity (a game's "plays once" clause spans the days),
/// and among several the ones whose links to the anchors are what the
/// swap of rows 0 and ri carries `u`'s links onto: one, or several for
/// the caller to tell apart.
fn pair_by_clause_and_links(w: &mut Work, u: i32, cell: &[i32], anchors: &[i32], ri: usize) -> Linked {
    let mut in_cell: HashSet<i32> = cell.iter().copied().collect();
    let mut hits: Vec<i32> = Vec::new();
    for li in [idx(u), idx(-u)] {
        let sign = if li == idx(u) { 1 } else { -1 };
        for &i in &w.occ[li] {
            if w.satisfied[i as usize] {
                continue;
            }
            let c = w.f.clause(i as usize);
            w.work += c.len() as u64;
            for &l in c {
                let v = l * sign;
                if w.lit_value(l) == 0 && l.abs() != u.abs() && in_cell.remove(&v) {
                    hits.push(v);
                }
            }
        }
        if w.over() {
            return Linked::None;
        }
    }
    match hits.len() {
        0 => Linked::None,
        1 => Linked::One(hits[0]),
        _ => {
            let want = anchor_links(w, u, anchors, 0, 0);
            let mut matching: Vec<i32> = Vec::new();
            if want.is_empty() {
                matching = hits;
            } else {
                for &v in &hits {
                    if anchor_links(w, v, anchors, 0, ri) == want {
                        matching.push(v);
                    }
                    if w.over() {
                        return Linked::None;
                    }
                }
            }
            match matching.len() {
                0 => Linked::None,
                1 => Linked::One(matching[0]),
                _ => Linked::Several(matching),
            }
        }
    }
}

/// Positions of the matrix entries: literal index -> (row, column); an
/// entry shared by two rows has no single position, (SHARED, SHARED).
const SHARED: u32 = u32::MAX - 1;

fn positions(m: &Matrix, nl: usize) -> Vec<(u32, u32)> {
    let mut pos: Vec<(u32, u32)> = vec![(u32::MAX, u32::MAX); nl];
    let mut count: Vec<u8> = vec![0; nl];
    for (ri, row) in m.rows.iter().enumerate() {
        for (ci, &l) in row.iter().enumerate() {
            pos[idx(l)] = (ri as u32, ci as u32);
            count[idx(l)] = count[idx(l)].saturating_add(1);
        }
    }
    for li in 0..nl {
        if count[li] > 1 {
            pos[li] = (SHARED, SHARED);
        }
    }
    pos
}

/// What a clause becomes when the matrix's row swaps carry every entry
/// of it onto the lowest row it touches: that row's entries in the
/// columns the clause uses, plus the literals outside the matrix. None
/// when the clause has an entry shared by two rows (no single swap
/// carries it), touches no row but one, or does not get shorter. A
/// clause in one column, one entry per row, becomes a unit.
fn transported(m: &Matrix, pos: &[(u32, u32)], c: &[i32]) -> Option<Vec<i32>> {
    let mut target = u32::MAX;
    for &l in c {
        let (r, _) = pos[idx(l)];
        if r == SHARED {
            return None;
        }
        if r != u32::MAX && r < target {
            target = r;
        }
    }
    if target == u32::MAX {
        return None;
    }
    let mut out: Vec<i32> = Vec::with_capacity(c.len());
    let mut rows = 0usize;
    let mut moved = false;
    for &l in c {
        let (r, k) = pos[idx(l)];
        if r == u32::MAX {
            out.push(l);
        } else {
            if r != target {
                moved = true;
            }
            rows += 1;
            out.push(m.rows[target as usize][k as usize]);
        }
    }
    if rows < 2 || !moved {
        return None;
    }
    out.sort_unstable_by_key(|l| (l.abs(), *l < 0));
    out.dedup();
    if out.len() >= c.len() || out.windows(2).any(|p| p[0] == -p[1]) {
        return None; // not shorter, or both signs of a variable (always true)
    }
    Some(out)
}

/// The clause to fix through the matrix: the one whose transported form
/// is shortest (a unit before a binary), the longest clause among those.
/// Returns (clause index, its active literals, the transported form).
fn best_transport_clause(w: &mut Work, m: &Matrix, pos: &[(u32, u32)]) -> Option<(usize, Vec<i32>, Vec<i32>)> {
    let mut best: Option<(usize, Vec<i32>, Vec<i32>)> = None;
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
        if c.len() >= 4096 && w.over() {
            break;
        }
        if let Some(t) = transported(m, pos, &c) {
            w.work += t.len() as u64;
            // a form an earlier derived clause subsumes adds nothing
            // (t is sorted, so each check is a merge)
            if w.extra.iter().any(|e| e.iter().all(|l| t.binary_search_by_key(&(l.abs(), *l < 0), |x| (x.abs(), *x < 0)).is_ok())) {
                continue;
            }
            let better = match &best {
                None => true,
                Some((_, bc, bt)) => (t.len(), std::cmp::Reverse(c.len())) < (bt.len(), std::cmp::Reverse(bc.len())),
            };
            if better {
                best = Some((i, c, t));
            }
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

/// Derive the transported form `t` of `clause` through the matrix: for
/// every entry of the clause in another row, a redundance step with the
/// row exchange as witness gives (t or not the entry); the clause then
/// gives t by propagation, and the steps are deleted. A unit is assigned
/// and propagated; a longer clause joins the symmetry checks. Returns
/// false on a conflict, or when a witness could not be built.
fn fix_transport(w: &mut Work, m: &Matrix, pos: &[(u32, u32)], clause: &[i32], t: &[i32], derived: &mut Derived) -> bool {
    let trow = clause.iter().map(|&l| pos[idx(l)].0).filter(|&r| r != u32::MAX).min().unwrap_or(u32::MAX) as usize;
    let mut steps: Vec<u64> = Vec::new();
    // the exchange with the target row, built once per source row
    let mut exchanges: HashMap<u32, (HashMap<i32, i32>, Vec<(i32, i32)>)> = HashMap::new();
    for &l in clause {
        let lrow = pos[idx(l)].0;
        if lrow == u32::MAX || lrow as usize == trow {
            continue;
        }
        if !exchanges.contains_key(&lrow) {
            let Some(map) = row_exchange(m, trow, lrow as usize, &w.value) else { return false };
            let mut witness: Vec<(i32, i32)> = Vec::with_capacity(map.len());
            for (&from, &to) in &map {
                if from > 0 {
                    // the map holds both polarities; one entry per variable
                    witness.push((from, to));
                }
            }
            witness.sort_unstable();
            w.work += 2 * map.len() as u64 + 1;
            exchanges.insert(lrow, (map, witness));
        }
        let (map, witness) = &exchanges[&lrow];
        let carried = *map.get(&l).unwrap_or(&l);
        if !t.contains(&carried) {
            return false; // the exchange does not carry l into t
        }
        if t.contains(&-l) {
            continue; // (t or not l) would be a tautology
        }
        let mut step: Vec<i32> = t.to_vec();
        step.push(-l);
        w.work += step.len() as u64 + witness.len() as u64;
        if let Some(pw) = derived.proof.as_mut() {
            if let Ok(id) = pw.red(&step, witness) {
                steps.push(id);
            }
        }
        if w.over() {
            return false;
        }
    }
    let id = derived.proof.as_mut().map(|pw| pw.rup(t).unwrap_or(0)).unwrap_or(0);
    if let Some(pw) = derived.proof.as_mut() {
        let _ = pw.delete(&steps);
    }
    derived.add(t, id);
    if t.len() == 1 {
        return w.assign_and_propagate(t[0], derived);
    }
    w.add_extra(t);
    true
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
    let live_rows: Vec<usize> = (0..m.rows.len()).filter(|&ri| live_cols.iter().any(|&k| unassigned(m.rows[ri][k], w))).collect();
    if live_rows.len() < 2 {
        return None;
    }
    // the columns are aligned to row 0: an entry two rows share sits in
    // the column of row 0's entry it is the image of, so for another
    // reference the rows are re-aligned as images of the reference under
    // the exchange composed through row 0 (a literal map, whatever the
    // assignment; the verification then checks it on the reduced formula)
    let free = vec![0i8; w.value.len()];
    for &reference in live_rows.iter().take(2) {
        let base: Vec<i32> = live_cols.iter().map(|&k| m.rows[reference][k]).collect();
        let mut kept: Vec<Vec<i32>> = vec![base.clone()];
        for &ri in &live_rows {
            if ri == reference {
                continue;
            }
            let row: Vec<i32> = if reference == 0 {
                live_cols.iter().map(|&k| m.rows[ri][k]).collect()
            } else {
                let Some(map) = row_exchange(m, reference, ri, &free) else { continue };
                base.iter().map(|l| *map.get(l).unwrap_or(l)).collect()
            };
            if w.verify_swap(&base, &row) {
                kept.push(row);
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
/// it happens, `budget` is the tick count it may reach, and `more` the
/// one it may reach once a matrix has been verified.
pub fn run(f: &Formula, derived: &mut Derived, budget: u64, more: u64, ticks: &mut u64, stop: &std::sync::atomic::AtomicBool) {
    let mut w = Work::new(f, budget, more, ticks as *mut u64, stop);
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
            // the extension finishes a detection and its fixings, it does
            // not start another: the base limit again from here on
            w.budget = w.base;
            if w.over() {
                break;
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
        // over every matrix, the clause whose transported form is shortest
        let mut choice: Option<(usize, Vec<i32>, Vec<i32>)> = None;
        for (mi, m) in pool.iter().enumerate() {
            let pos = positions(m, nl);
            if let Some((_, c, t)) = best_transport_clause(&mut w, m, &pos) {
                let better = match &choice {
                    None => true,
                    Some((_, bc, bt)) => (t.len(), std::cmp::Reverse(c.len())) < (bt.len(), std::cmp::Reverse(bc.len())),
                };
                if better {
                    choice = Some((mi, c, t));
                }
            }
            if w.over() {
                break;
            }
        }
        let Some((mi, clause, t)) = choice else {
            pool.clear();
            continue; // nothing to fix through these matrices: detect again, once
        };
        let pos = positions(&pool[mi], nl);
        let before = w.work;
        let ok = fix_transport(&mut w, &pool[mi], &pos, &clause, &t, derived);
        fixed += 1;
        if debug() {
            eprintln!("structsym: fix {} through matrix {} ({} rows): a {}-literal clause becomes {:?}, {}K work, trail {}", fixed, mi, pool[mi].rows.len(), clause.len(), t, (w.work - before) / 1000, w.trail.len());
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
            let before = (m.rows.len(), m.rows[0].len());
            match shrink(&m, &mut w) {
                Some(s) => {
                    if debug() {
                        eprintln!("structsym:   matrix {}x{} shrinks to {}x{}", before.0, before.1, s.rows.len(), s.rows[0].len());
                    }
                    pool.push(s);
                }
                None => {
                    if debug() {
                        eprintln!("structsym:   matrix {}x{} dropped", before.0, before.1);
                    }
                }
            }
            if w.over() {
                break;
            }
        }
    }
    *ticks = w.work;
    if fixed > 0 || matrices > 0 {
        derived.notes.push(format!(
            "symmetry: {} matri{} with interchangeable rows, {} fixing{}, {} propagated{}",
            matrices,
            if matrices == 1 { "x" } else { "ces" },
            fixed,
            if fixed == 1 { "" } else { "s" },
            w.trail.len().saturating_sub(fixed),
            if derived.refuted { ", refuted" } else { "" }
        ));
    }
}
