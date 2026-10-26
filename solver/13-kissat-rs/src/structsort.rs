//! Not in kissat. The sorter pass of the structure pass (bead
//! SAT-playground-skz.22, 2026-10-26): a miter of two sorting programs over
//! the same N words of W bits is refuted by counting.
//!
//! In plain terms. Both sides of the 2025 sorter miters are built from swap
//! units: W pairs of multiplexers under one select s over two words X and
//! Y, with C = s ? Y : X and D = s ? X : Y. Whatever s is, {C, D} = {X, Y},
//! so each side's output is a rearrangement of its input by construction.
//! Two sorted rearrangements of the same words are the same sequence, so
//! what is left to prove is that each side's output is sorted, and that is
//! easy for CDCL (at most 1.5 s per adjacent pair on the 2025 cells, where
//! the miter takes 148 s to a timeout). "The same words" is a count, which
//! resolution cannot do compactly and cutting planes can, by thresholds.
//!
//! What the pass does, in order (`tools/sorter_proof.py` is the prototype,
//! checked on toys and on the seven 2025 cells first; solver README
//! "Sorter miters: census and certificate"):
//! 1. reads the formula as a circuit in file order: the clauses whose
//!    largest variable is v are v's gate, each checked to define v (a truth
//!    table over at most six inputs, or a wide and/or); a unit on any other
//!    variable is a constant, and the one unit on a gate is the miter's
//!    root. Under the root, N gates each read W xors, and with every xor
//!    "equal" the root is false. The N W primary inputs are the input
//!    words, least significant bit first;
//! 2. finds the swap units (multiplexer pairs by select and data pair,
//!    grouped into words), the two sides (the data cone of each compared
//!    word), and checks per side that each word feeds one unit and that the
//!    units telescope from the N input words to the N output words; the
//!    output positions come from simulation (ascending), and each xor group
//!    must compare the same position on both sides;
//! 3. per side and adjacent pair (k, k+1): fresh carry variables compute
//!    out_k + ~out_k+1 (its carry out says out_k > out_k+1), and the full
//!    solver with variable addition off refutes the side's clauses plus
//!    that carry. Its added clauses are the rup steps of one pbc, which
//!    gives "no carry out"; the carry cells' inequalities summed
//!    with weights 2^i turn that into value(out_k+1) - value(out_k) >= 0;
//! 4. the count: for every word w and t in 1..2^W-1 a fresh g(w,t) with
//!    g -> value(w) >= t and ~g -> value(w) <= t-1; per swap unit and t,
//!    g(C,t) + g(D,t) = g(X,t) + g(Y,t) from the multiplexer clauses; summed
//!    per side, every inner word cancels. With the outputs sorted, the two
//!    sides' g(out_k,t) are equal by one rup step each, the bits follow by
//!    rup over the thresholds, the xors are false and so is the root: the
//!    empty clause.
//! Without a proof file the checks of steps 1-3 are the whole check.

use crate::structproof::Writer;
use crate::structure::{Derived, Formula};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_SUPPORT: usize = 6; // a table gate's truth table fits in a u64
const MAX_VARS: i32 = 1 << 21; // a sorter miter is small: beyond this the pass does not read
const MAX_WIDTH: usize = 10; // words of at most this many bits (2^W - 1 thresholds each)
const MAX_WORDS: usize = 64;
const SOLVER_CONFLICTS: u32 = 2_000_000; // the full solver's conflict limit per lemma

fn debug() -> bool {
    std::env::var_os("SAT_STRUCT_DEBUG").is_some()
}

// ---------------------------------------------------------------------------
// reading the circuit
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Gate {
    /// The function `tt` of `sup` (sorted variables; minterm m: bit i of m
    /// is the value of sup[i]).
    Table { sup: Vec<i32>, tt: u64 },
    /// An or of the literals (an and when `and`), any width.
    Wide { and: bool, lits: Vec<i32> },
}

impl Gate {
    fn vars(&self) -> Vec<i32> {
        match self {
            Gate::Table { sup, .. } => sup.clone(),
            Gate::Wide { lits, .. } => lits.iter().map(|l| l.abs()).collect(),
        }
    }
    fn arity(&self) -> usize {
        match self {
            Gate::Table { sup, .. } => sup.len(),
            Gate::Wide { lits, .. } => lits.len(),
        }
    }
    /// The output under 64 patterns at once, from the variables' words.
    fn eval(&self, val: &[u64]) -> u64 {
        match self {
            Gate::Table { sup, tt } => {
                let mut out = 0u64;
                for m in 0..(1u32 << sup.len()) {
                    if (tt >> m) & 1 == 1 {
                        let mut term = u64::MAX;
                        for (i, &u) in sup.iter().enumerate() {
                            let x = val[u as usize];
                            term &= if (m >> i) & 1 == 1 { x } else { !x };
                        }
                        out |= term;
                    }
                }
                out
            }
            Gate::Wide { and, lits } => {
                let lv = |l: i32| if l > 0 { val[l as usize] } else { !val[(-l) as usize] };
                if *and {
                    lits.iter().fold(u64::MAX, |a, &l| a & lv(l))
                } else {
                    lits.iter().fold(0u64, |a, &l| a | lv(l))
                }
            }
        }
    }
    fn is_buf(&self) -> Option<i32> {
        match self {
            Gate::Table { sup, tt } if sup.len() == 1 && *tt == 0b10 => Some(sup[0]),
            _ => None,
        }
    }
    /// An xor (Some(false)) or xnor (Some(true)) of two variables.
    fn xor_kind(&self) -> Option<bool> {
        match self {
            Gate::Table { sup, tt } if sup.len() == 2 && *tt == 0b0110 => Some(false),
            Gate::Table { sup, tt } if sup.len() == 2 && *tt == 0b1001 => Some(true),
            _ => None,
        }
    }
    /// (select, then, else) with out = s ? t : e over three variables.
    fn ite(&self) -> Option<(i32, i32, i32)> {
        let Gate::Table { sup, tt } = self else { return None };
        if sup.len() != 3 {
            return None;
        }
        for p in 0..3 {
            let others: Vec<usize> = (0..3).filter(|&i| i != p).collect();
            for (t, e) in [(others[0], others[1]), (others[1], others[0])] {
                let ok = (0..8u32).all(|m| {
                    let want = if (m >> p) & 1 == 1 { (m >> t) & 1 } else { (m >> e) & 1 };
                    ((tt >> m) & 1) as u32 == want
                });
                if ok {
                    return Some((sup[p], sup[t], sup[e]));
                }
            }
        }
        None
    }
}

/// The clauses of length two or more by their largest variable.
struct Groups {
    first: Vec<u32>,
    ids: Vec<u32>,
}

impl Groups {
    fn of(&self, v: i32) -> &[u32] {
        let v = v as usize;
        if v + 1 >= self.first.len() {
            return &[];
        }
        &self.ids[self.first[v] as usize..self.first[v + 1] as usize]
    }
}

fn group_clauses(f: &Formula, work: &mut u64) -> Groups {
    let nv = f.vars as usize + 1;
    *work += 3 * nv as u64 + f.len() as u64; // the tables: counts, starts, cursors, and one slot per clause
    let mut count = vec![0u32; nv + 1];
    let mut top = vec![0u32; f.len()];
    for i in 0..f.len() {
        if f.taut[i] {
            continue;
        }
        let c = f.clause(i);
        *work += c.len() as u64 + 1;
        if c.len() < 2 {
            continue; // units are constants or the root, never part of a gate
        }
        let m = c.iter().map(|l| l.unsigned_abs()).max().unwrap();
        top[i] = m;
        count[m as usize] += 1;
    }
    let mut first = vec![0u32; nv + 1];
    for v in 0..nv {
        first[v + 1] = first[v] + count[v];
    }
    let mut pos = first.clone();
    let mut ids = vec![0u32; first[nv] as usize];
    for (i, &m) in top.iter().enumerate() {
        if m != 0 {
            ids[pos[m as usize] as usize] = i as u32;
            pos[m as usize] += 1;
        }
    }
    Groups { first, ids }
}

/// Truth table of v over `support` if the clauses determine v for every
/// assignment of the support, else None (as the arithmetic pass reads one).
fn definition_tt(v: i32, clause_ids: &[u32], f: &Formula, support: &[i32], work: &mut u64) -> Option<u64> {
    let n = support.len();
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
                let i = support.iter().position(|&u| u == l.abs())? as u8;
                others.push((i, l > 0));
            }
        }
        match sign {
            1 => pos.push(others),
            -1 => neg.push(others),
            _ => return None,
        }
        *work += c.len() as u64;
    }
    let mut tt = 0u64;
    for m in 0..(1u32 << n) {
        *work += (pos.len() + neg.len()) as u64;
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

/// A wide or (v = l_1 | ... | l_n: the clause ~v l_1 .. l_n and v ~l_i) or
/// a wide and (v = m_1 & ... & m_n: v ~m_1 .. ~m_n and ~v m_i), from a
/// group with exactly those clauses (repeated clauses allowed).
fn wide_gate(v: i32, clause_ids: &[u32], f: &Formula, work: &mut u64) -> Option<Gate> {
    let mut long: Option<&[i32]> = None;
    let mut binaries: Vec<(i32, i32)> = Vec::new(); // (sign of v, other literal)
    for &ci in clause_ids {
        let c = f.clause(ci as usize);
        *work += c.len() as u64;
        let vl = *c.iter().find(|l| l.abs() == v)?;
        if c.len() == 2 {
            let other = *c.iter().find(|l| l.abs() != v)?;
            binaries.push((vl.signum(), other));
        } else {
            match long {
                None => long = Some(c),
                Some(prev) => {
                    let mut a = prev.to_vec();
                    let mut b = c.to_vec();
                    a.sort_unstable();
                    b.sort_unstable();
                    if a != b {
                        return None;
                    }
                }
            }
        }
    }
    let long = long?;
    let vl = *long.iter().find(|l| l.abs() == v)?;
    let mut lits: Vec<i32> = long.iter().copied().filter(|l| l.abs() != v).collect();
    lits.sort_unstable();
    let want: HashSet<(i32, i32)> = lits.iter().map(|&l| (-vl.signum(), -l)).collect();
    let have: HashSet<(i32, i32)> = binaries.iter().copied().collect();
    if want != have {
        return None;
    }
    if vl < 0 {
        Some(Gate::Wide { and: false, lits }) // ~v | l_1 | ..: v -> or; l_i -> v
    } else {
        Some(Gate::Wide { and: true, lits: lits.iter().map(|l| -l).collect() }) // v | ~m_1 | ..
    }
}

/// v's gate from its group, or None (not a definition, or the work it
/// would take is past the limit: a group is read only when the limit
/// covers it, and so is its truth table).
fn read_gate(v: i32, groups: &Groups, f: &Formula, work: &mut u64, stopped: &dyn Fn(u64) -> bool) -> Option<Gate> {
    let ids = groups.of(v);
    if ids.is_empty() {
        return None;
    }
    let size: u64 = ids.iter().map(|&ci| f.clause(ci as usize).len() as u64).sum::<u64>() + ids.len() as u64;
    if stopped(work.saturating_add(size)) {
        return None;
    }
    let mut sup: Vec<i32> = Vec::new();
    for &ci in ids {
        for &l in f.clause(ci as usize) {
            if l.abs() != v {
                sup.push(l.abs());
            }
        }
    }
    *work += sup.len() as u64;
    sup.sort_unstable();
    sup.dedup();
    if sup.is_empty() {
        return None;
    }
    if sup.len() <= MAX_SUPPORT {
        let scan = size.saturating_add((1u64 << sup.len()).saturating_mul(ids.len() as u64));
        if stopped(work.saturating_add(scan)) {
            return None;
        }
        return definition_tt(v, ids, f, &sup, work).map(|tt| Gate::Table { sup, tt });
    }
    if stopped(work.saturating_add(2 * size)) {
        return None;
    }
    wide_gate(v, ids, f, work)
}

/// The gate's output from its inputs' values (None if one is missing).
fn eval_with(g: &Gate, value: &dyn Fn(i32) -> Option<bool>) -> Option<bool> {
    match g {
        Gate::Table { sup, tt } => {
            let mut m = 0u32;
            for (i, &u) in sup.iter().enumerate() {
                if value(u)? {
                    m |= 1 << i;
                }
            }
            Some((tt >> m) & 1 == 1)
        }
        Gate::Wide { and, lits } => {
            let mut acc = *and;
            for &l in lits {
                let x = value(l.abs())? != (l < 0);
                if *and {
                    acc &= x;
                } else {
                    acc |= x;
                }
            }
            Some(acc)
        }
    }
}

/// The miter's top: the root literal (asserted by a unit), and per compared
/// word the xor gates under it (the xor variable and whether it is an xnor).
struct Top {
    root: i32,
    groups: Vec<Vec<(i32, bool)>>,
}

fn read_top(f: &Formula, groups: &Groups, work: &mut u64, stopped: &dyn Fn(u64) -> bool) -> Option<Top> {
    // the one unit on a variable with a group (a second one ends the look)
    let mut root: Option<i32> = None;
    *work += f.len() as u64;
    for i in 0..f.len() {
        if !f.taut[i] && f.clause(i).len() == 1 {
            let l = f.clause(i)[0];
            if !groups.of(l.abs()).is_empty() {
                match root {
                    None => root = Some(l),
                    Some(r) if r == l => {}
                    Some(_) => return None,
                }
            }
        }
    }
    let root = root?;
    let mut memo: HashMap<i32, Option<Gate>> = HashMap::new();
    let mut gate_at = |v: i32, work: &mut u64| -> Option<Gate> {
        if let Some(g) = memo.get(&v) {
            return g.clone();
        }
        let g = read_gate(v, groups, f, work, stopped);
        memo.insert(v, g.clone());
        g
    };
    let rv = root.abs();
    let rg = gate_at(rv, work)?;
    let children = rg.vars();
    if children.len() < 2 || children.len() > MAX_WORDS {
        return None;
    }
    let mut out: Vec<Vec<(i32, bool)>> = Vec::new();
    // with every xor "equal" (an xor 0, an xnor 1): each child's value,
    // then the root's, which must contradict the unit
    let mut values: HashMap<i32, bool> = HashMap::new();
    for &c in &children {
        let mut cv = c;
        for _ in 0..64 {
            match gate_at(cv, work).and_then(|g| g.is_buf()) {
                Some(u) => cv = u,
                None => break,
            }
        }
        let cg = gate_at(cv, work)?;
        if cg.arity() > MAX_WIDTH {
            return None; // (before its inputs are read)
        }
        let mut xors: Vec<(i32, bool)> = Vec::new();
        let mut leaf_value: HashMap<i32, bool> = HashMap::new();
        for x in cg.vars() {
            let mut xv = x;
            for _ in 0..64 {
                match gate_at(xv, work).and_then(|g| g.is_buf()) {
                    Some(u) => xv = u,
                    None => break,
                }
            }
            let xnor = gate_at(xv, work)?.xor_kind()?;
            xors.push((xv, xnor));
            leaf_value.insert(x, xnor);
        }
        if xors.is_empty() || xors.len() > MAX_WIDTH {
            return None;
        }
        values.insert(c, eval_with(&cg, &|u| leaf_value.get(&u).copied())?);
        out.push(xors);
    }
    if eval_with(&rg, &|u| values.get(&u).copied())? == (root > 0) {
        return None;
    }
    let w = out[0].len();
    if out.iter().any(|g| g.len() != w) {
        return None;
    }
    Some(Top { root, groups: out })
}

/// Work in one piece: charged when the limit covers it, else Err.
fn reserve(work: &mut u64, cost: u64, stopped: &dyn Fn(u64) -> bool) -> Result<(), String> {
    if stopped(work.saturating_add(cost)) {
        return Err("budget".into());
    }
    *work += cost;
    Ok(())
}

struct Unit {
    sel: i32,
    x: usize,
    y: usize,
    c: usize,
    d: usize,
}

/// The circuit as words and swap units.
struct Sorter {
    n: usize,
    w: usize,
    gates: Vec<Option<Gate>>,
    alias: HashMap<i32, i32>,
    constant: HashMap<i32, bool>,
    words: Vec<Vec<i32>>,
    lane: Vec<usize>,
    made_by: Vec<Option<usize>>,
    word_of: HashMap<i32, (usize, usize)>,
    units: Vec<Unit>,
    /// per side: its units and its output words by position (ascending)
    sides: Vec<(Vec<usize>, Vec<usize>)>,
    /// per compared word pair: (position, side 0's word, side 1's word, the xors by bit)
    pairs: Vec<(usize, usize, usize, Vec<(i32, bool)>)>,
}

impl Sorter {
    /// The variable a buffer chain copies (the table holds the chain's end).
    fn res(&self, v: i32) -> i32 {
        self.alias.get(&v).copied().unwrap_or(v)
    }
}

fn read_sorter(f: &Formula, groups: &Groups, top: &Top, work: &mut u64, stopped: &dyn Fn(u64) -> bool) -> Result<Sorter, String> {
    let n = top.groups.len();
    let w = top.groups[0].len();
    // with room for N different words, simulation names the positions;
    // otherwise the lanes do (input word k starts in lane k, C keeps X's
    // lane, D Y's), checked by simulation; the sortedness lemmas verify
    // either way
    let distinct = n <= (1usize << w);
    // every group a definition, in circuit order
    let nv = f.vars as usize + 1;
    if stopped(work.saturating_add(nv as u64)) {
        return Err("no budget for the gate table".into());
    }
    *work += nv as u64;
    let mut gates: Vec<Option<Gate>> = vec![None; nv];
    for v in 1..nv as i32 {
        if groups.of(v).is_empty() {
            continue;
        }
        let Some(g) = read_gate(v, groups, f, work, stopped) else {
            return Err(if stopped(*work) { "budget".into() } else { format!("the clauses of {} do not define it", v) });
        };
        if g.vars().iter().any(|&u| u >= v) {
            return Err(format!("gate {} reads a later variable", v));
        }
        gates[v as usize] = Some(g);
        if stopped(*work) {
            return Err("budget".into());
        }
    }
    // constants and the root: a visit per literal and clause, and the
    // table of variables, reserved in full first (Codex review round 3)
    let scan = f.lits.len() as u64 + f.len() as u64 + 2 * nv as u64;
    if stopped(work.saturating_add(scan)) {
        return Err("budget".into());
    }
    *work += f.len() as u64 + 2 * nv as u64;
    let mut constant: HashMap<i32, bool> = HashMap::new();
    let mut present = vec![false; nv];
    for i in 0..f.len() {
        if f.taut[i] {
            continue;
        }
        let c = f.clause(i);
        for &l in c {
            present[l.unsigned_abs() as usize] = true;
        }
        if c.len() == 1 {
            let l = c[0];
            if gates[l.unsigned_abs() as usize].is_some() {
                if l != top.root {
                    return Err(format!("a second unit on gate {}", l.abs()));
                }
            } else if let Some(&b) = constant.get(&l.abs()) {
                if b != (l > 0) {
                    return Err("two opposite units".into());
                }
            } else {
                constant.insert(l.abs(), l > 0);
            }
        }
    }
    *work += f.lits.len() as u64;
    let primary: Vec<i32> = (1..nv as i32).filter(|&v| present[v as usize] && gates[v as usize].is_none() && !constant.contains_key(&v)).collect();
    if primary.len() != n * w {
        return Err(format!("{} primary inputs, the compares read {} words of {} bits", primary.len(), n, w));
    }
    let mut s = Sorter {
        n,
        w,
        gates,
        alias: HashMap::new(),
        constant,
        words: Vec::new(),
        lane: Vec::new(),
        made_by: Vec::new(),
        word_of: HashMap::new(),
        units: Vec::new(),
        sides: Vec::new(),
        pairs: Vec::new(),
    };
    for k in 0..n {
        let bits = primary[k * w..(k + 1) * w].to_vec();
        for (i, &v) in bits.iter().enumerate() {
            s.word_of.insert(v, (k, i));
        }
        s.words.push(bits);
        s.lane.push(k);
        s.made_by.push(None);
    }
    // a buffer copies its input's representative: the inputs come first,
    // so one lookup resolves any chain. (This scan and the multiplexer
    // scan below are one piece of work: a visit per variable each and four
    // per multiplexer at most, reserved here; Codex review round 4.)
    reserve(work, 6 * nv as u64, stopped)?;
    for v in 1..nv as i32 {
        if let Some(u) = s.gates[v as usize].as_ref().and_then(|g| g.is_buf()) {
            let r = s.alias.get(&u).copied().unwrap_or(u);
            s.alias.insert(v, r);
        }
    }
    // multiplexers by select and data pair: then-variable -> gate
    let mut slices: HashMap<(i32, i32, i32), HashMap<i32, i32>> = HashMap::new();
    let mut order: Vec<(i32, i32, i32)> = Vec::new();
    for v in 1..nv as i32 {
        let Some((sel, t, e)) = s.gates[v as usize].as_ref().and_then(|g| g.ite()) else { continue };
        let (sel, t, e) = (s.res(sel), s.res(t), s.res(e));
        if t == e {
            continue;
        }
        let key = (sel, t.min(e), t.max(e));
        let entry = slices.entry(key).or_default();
        if entry.is_empty() {
            order.push(key);
        }
        entry.insert(t, v);
    }
    let mut pending: HashMap<(i32, usize, usize), HashMap<usize, (i32, i32)>> = HashMap::new();
    let mut todo: Vec<(i32, i32, i32)> = order.into_iter().filter(|k| slices[k].len() == 2).collect();
    loop {
        let mut progressed = false;
        let mut rest = Vec::new();
        // (every slice looked at is charged, a deferred one too: a round
        // that settles one unit still visits all that are left)
        *work += todo.len() as u64;
        if stopped(*work) {
            return Err("budget".into());
        }
        for key in todo {
            let (sel, a, b) = key;
            let (Some(&(wa, ia)), Some(&(wb, ib))) = (s.word_of.get(&a), s.word_of.get(&b)) else {
                rest.push(key);
                continue;
            };
            if ia != ib || wa == wb {
                rest.push(key);
                continue;
            }
            let (a, b, wa, wb) = if wa < wb { (a, b, wa, wb) } else { (b, a, wb, wa) };
            let entry = &slices[&key];
            // C = s ? Y : X (its then-input is b, in Y); D = s ? X : Y
            let (cg, dg) = (entry[&b], entry[&a]);
            let p = pending.entry((sel, wa, wb)).or_default();
            p.insert(ia, (cg, dg));
            if p.len() == w {
                let bits = pending.remove(&(sel, wa, wb)).unwrap();
                let uid = s.units.len();
                let cw: Vec<i32> = (0..w).map(|i| bits[&i].0).collect();
                let dw: Vec<i32> = (0..w).map(|i| bits[&i].1).collect();
                let c = s.words.len();
                for (i, &v) in cw.iter().enumerate() {
                    s.word_of.insert(v, (c, i));
                }
                s.words.push(cw);
                s.lane.push(s.lane[wa]);
                s.made_by.push(Some(uid));
                let d = s.words.len();
                for (i, &v) in dw.iter().enumerate() {
                    s.word_of.insert(v, (d, i));
                }
                s.words.push(dw);
                s.lane.push(s.lane[wb]);
                s.made_by.push(Some(uid));
                s.units.push(Unit { sel, x: wa, y: wb, c, d });
            }
            progressed = true;
            *work += 4;
        }
        todo = rest;
        if !progressed || todo.is_empty() || stopped(*work) {
            break;
        }
    }
    if stopped(*work) {
        return Err("budget".into());
    }
    // the compared words: each xor group pairs one word of each side, bit by bit
    let mut compared: Vec<(usize, usize, Vec<(i32, bool)>)> = Vec::new();
    for g in &top.groups {
        let mut pair: Option<(usize, usize)> = None;
        let mut by_bit: Vec<Option<(i32, bool)>> = vec![None; w];
        for &(x, xnor) in g {
            let sup = s.gates[x as usize].as_ref().unwrap().vars();
            let (a, b) = (s.res(sup[0]), s.res(sup[1]));
            let (Some(&(wa, ia)), Some(&(wb, ib))) = (s.word_of.get(&a), s.word_of.get(&b)) else {
                return Err(format!("xor {} does not read two word bits", x));
            };
            if ia != ib || wa == wb {
                return Err(format!("xor {} reads bits of different weight", x));
            }
            let pr = (wa.min(wb), wa.max(wb));
            if pair.map_or(false, |p| p != pr) || by_bit[ia].is_some() {
                return Err("an xor group does not compare one word pair bit by bit".into());
            }
            pair = Some(pr);
            by_bit[ia] = Some((x, xnor));
        }
        let (a, b) = pair.unwrap();
        compared.push((a, b, by_bit.into_iter().map(|x| x.unwrap()).collect()));
    }
    // the sides: the data cone of each compared word
    // (each walk charges a visit per word and polls: Codex review round 5)
    fn data_cone(s: &Sorter, wid: usize, work: &mut u64, stopped: &dyn Fn(u64) -> bool) -> Option<HashSet<usize>> {
        let mut cone = HashSet::new();
        let mut stack = vec![wid];
        while let Some(x) = stack.pop() {
            *work += 1;
            if stopped(*work) {
                return None;
            }
            if let Some(u) = s.made_by[x] {
                if cone.insert(u) {
                    stack.push(s.units[u].x);
                    stack.push(s.units[u].y);
                }
            }
        }
        Some(cone)
    }
    let mut side_units: Vec<HashSet<usize>> = Vec::new();
    let mut side_of: HashMap<usize, usize> = HashMap::new();
    for &(a, b, _) in &compared {
        for wid in [a, b] {
            let Some(cone) = data_cone(&s, wid, work, stopped) else { return Err("budget".into()) };
            if cone.is_empty() {
                return Err("a compared word is an input".into());
            }
            let found = side_units.iter().position(|us| !us.is_disjoint(&cone));
            match found {
                Some(si) => {
                    side_units[si].extend(cone);
                    side_of.insert(wid, si);
                }
                None => {
                    side_units.push(cone);
                    side_of.insert(wid, side_units.len() - 1);
                }
            }
        }
    }
    if side_units.len() != 2 || compared.iter().any(|(a, b, _)| side_of[a] == side_of[b]) || side_units[0].intersection(&side_units[1]).next().is_some() {
        return Err("not two separate sides".into());
    }
    // per side: one reader per word, and the units telescope from the inputs to the outputs
    let mut outs: Vec<Vec<usize>> = vec![Vec::new(), Vec::new()];
    for &(a, b, _) in &compared {
        outs[side_of[&a]].push(a);
        outs[side_of[&b]].push(b);
    }
    for si in 0..2 {
        let mut balance: HashMap<usize, i64> = HashMap::new();
        let mut readers: HashMap<usize, u32> = HashMap::new();
        reserve(work, 4 * side_units[si].len() as u64 + 2 * n as u64, stopped)?;
        for &u in &side_units[si] {
            let un = &s.units[u];
            *balance.entry(un.c).or_default() += 1;
            *balance.entry(un.d).or_default() += 1;
            *balance.entry(un.x).or_default() -= 1;
            *balance.entry(un.y).or_default() -= 1;
            *readers.entry(un.x).or_default() += 1;
            *readers.entry(un.y).or_default() += 1;
        }
        if readers.values().any(|&r| r > 1) {
            return Err(format!("side {}: a word feeds two units", si));
        }
        let mut want: HashMap<usize, i64> = HashMap::new();
        for &o in &outs[si] {
            *want.entry(o).or_default() += 1;
        }
        for k in 0..n {
            *want.entry(k).or_default() -= 1;
        }
        balance.retain(|_, v| *v != 0);
        want.retain(|_, v| *v != 0);
        if balance != want {
            return Err(format!("side {}: the units do not telescope from the inputs to the outputs", si));
        }
    }
    // the output positions by simulation: 64 patterns of distinct words;
    // the table and every gate's evaluation reserved up front
    let eval_cost: u64 = s.gates.iter().flatten().map(|g| 1u64 << g.arity().min(MAX_SUPPORT)).sum();
    reserve(work, nv as u64 + s.constant.len() as u64 + eval_cost, stopped)?;
    let mut val = vec![0u64; nv];
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || -> u64 {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    // (the 64 samples: drawing N words, packing their bits, decoding and
    // sorting each side's outputs)
    reserve(work, 64 * n as u64 * (n as u64 + 4 * w as u64 + 8), stopped)?;
    let mut inputs: Vec<Vec<u64>> = vec![vec![0; 64]; n];
    for j in 0..64 {
        let mut chosen: Vec<u64> = Vec::with_capacity(n);
        while chosen.len() < n {
            let x = next() % (1u64 << w);
            if !distinct || !chosen.contains(&x) {
                chosen.push(x);
            }
        }
        for k in 0..n {
            inputs[k][j] = chosen[k];
        }
    }
    for k in 0..n {
        for (i, &v) in s.words[k].iter().enumerate() {
            let mut word = 0u64;
            for j in 0..64 {
                word |= ((inputs[k][j] >> i) & 1) << j;
            }
            val[v as usize] = word;
        }
    }
    for (&v, &b) in &s.constant {
        val[v as usize] = if b { u64::MAX } else { 0 };
    }
    for v in 1..nv {
        if let Some(g) = &s.gates[v] {
            val[v] = g.eval(&val);
        }
    }
    if stopped(*work) {
        return Err("budget".into());
    }
    let value_of = |wid: usize, j: usize| -> u64 { s.words[wid].iter().enumerate().map(|(i, &v)| ((val[v as usize] >> j) & 1) << i).sum() };
    let mut sides: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();
    for si in 0..2 {
        let mut order: Option<Vec<usize>> = None;
        let mut by_lane: Vec<usize> = outs[si].clone();
        by_lane.sort_by_key(|&wid| s.lane[wid]);
        if !distinct && by_lane.iter().enumerate().any(|(k, &wid)| s.lane[wid] != k) {
            return Err(format!("side {}: the outputs do not fill the lanes once each", si));
        }
        for j in 0..64 {
            let mut o = if distinct { outs[si].clone() } else { by_lane.clone() };
            if distinct {
                o.sort_by_key(|&wid| value_of(wid, j));
            }
            let got: Vec<u64> = o.iter().map(|&wid| value_of(wid, j)).collect();
            if got.windows(2).any(|p| p[0] > p[1]) {
                return Err(format!("side {}: the outputs are not ascending by lane", si));
            }
            let mut want: Vec<u64> = (0..n).map(|k| inputs[k][j]).collect();
            want.sort_unstable();
            if got != want {
                return Err(format!("side {}: the outputs are not the inputs rearranged", si));
            }
            match &order {
                None => order = Some(o),
                Some(prev) if *prev != o => return Err(format!("side {}: the outputs are not sorted in one order", si)),
                _ => {}
            }
        }
        let mut units: Vec<usize> = side_units[si].iter().copied().collect();
        units.sort_unstable();
        sides.push((units, order.unwrap()));
    }
    let pos: Vec<HashMap<usize, usize>> = sides.iter().map(|(_, o)| o.iter().enumerate().map(|(k, &wid)| (wid, k)).collect()).collect();
    for (a, b, xors) in compared {
        let (a, b) = if side_of[&a] == 0 { (a, b) } else { (b, a) };
        if pos[0][&a] != pos[1][&b] {
            return Err("the miter compares words at different positions".into());
        }
        s.pairs.push((pos[0][&a], a, b, xors));
    }
    s.pairs.sort_by_key(|p| p.0);
    s.sides = sides;
    Ok(s)
}

/// The clauses of every gate in the cone of a side's outputs, and the
/// units of the constants it reads.
fn side_clauses(f: &Formula, groups: &Groups, s: &Sorter, si: usize, work: &mut u64, stopped: &dyn Fn(u64) -> bool) -> Option<Vec<Vec<i32>>> {
    let mut seen: HashSet<i32> = HashSet::new();
    let mut stack: Vec<i32> = s.sides[si].1.iter().flat_map(|&wid| s.words[wid].iter().copied()).collect();
    let mut out: Vec<Vec<i32>> = Vec::new();
    while let Some(v) = stack.pop() {
        *work += 1;
        if stopped(*work) {
            return None;
        }
        if !seen.insert(v) {
            continue;
        }
        if let Some(&b) = s.constant.get(&v) {
            out.push(vec![if b { v } else { -v }]);
            continue;
        }
        if let Some(g) = &s.gates[v as usize] {
            // (a group may hold many copies of a clause: polled per clause,
            // Codex review round 6)
            for &ci in groups.of(v) {
                *work += f.clause(ci as usize).len() as u64 + 1;
                if stopped(*work) {
                    return None;
                }
                out.push(f.clause(ci as usize).to_vec());
            }
            stack.extend(g.vars());
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// the sortedness lemmas
// ---------------------------------------------------------------------------

enum Verdict {
    /// The clauses that refute the lemma, each implied by unit propagation
    /// from the earlier ones, the lemma's clauses and the assumed literal;
    /// the last one is the empty clause.
    Refuted(Vec<Vec<i32>>),
    Sat,
    Unknown,
}

/// The full solver on the clauses plus the unit `assume`, with variable
/// addition and every structure pass off: its added clauses (each implied
/// by unit propagation under the unit), then the empty clause. The
/// sub-solver sees the lemma's variables numbered 1..k in order of first
/// use, so its tables are the lemma's size whatever the formula's numbers
/// are (the carries come after every variable of the formula; Codex
/// review round 7), and its clauses are mapped back.
fn solver_refute(clauses: &[Vec<i32>], assume: i32, conflicts: u32, ticks: u64, work: &mut u64, stop: &AtomicBool) -> Verdict {
    if ticks == 0 || stop.load(Ordering::Relaxed) {
        return Verdict::Unknown;
    }
    let mut dense: HashMap<i32, i32> = HashMap::new();
    let mut back: Vec<i32> = vec![0];
    let rename = |l: i32, dense: &mut HashMap<i32, i32>, back: &mut Vec<i32>| -> i32 {
        let v = *dense.entry(l.abs()).or_insert_with(|| {
            back.push(l.abs());
            back.len() as i32 - 1
        });
        if l < 0 {
            -v
        } else {
            v
        }
    };
    let renamed: Vec<Vec<i32>> = clauses.iter().map(|c| c.iter().map(|&l| rename(l, &mut dense, &mut back)).collect()).collect();
    let unit = rename(assume, &mut dense, &mut back);
    let mut sub = Box::new(crate::internal::init());
    for (name, value) in [
        ("quiet", 1),
        ("factor", 0),
        ("preprocessfactor", 0),
        ("structsym", 0),
        ("structsymext", 0),
        ("structparity", 0),
        ("structcount", 0),
        ("structcsp", 0),
        ("structarith", 0),
        ("structsort", 0),
        ("gauss", 0),
    ] {
        crate::internal::set_option(&mut sub, name, value);
    }
    crate::internal::set_conflict_limit(&mut sub, conflicts);
    crate::internal::set_ticks_limit(&mut sub, ticks);
    crate::proof::init_captured_proof(&mut sub);
    // (the parent's alarm stops it; its work reaches the parent's tick
    // counter as it goes)
    sub.parent_stop = stop as *const AtomicBool as usize;
    sub.parent_ticks_base = *work;
    sub.parent_ticks = work as *mut u64 as usize;
    for c in &renamed {
        for &l in c {
            crate::internal::add(&mut sub, l);
        }
        crate::internal::add(&mut sub, 0);
    }
    crate::internal::add(&mut sub, unit);
    crate::internal::add(&mut sub, 0);
    let res = crate::internal::solve(&mut sub);
    *work = sub.parent_ticks_base.saturating_add(sub.statistics.work_clock());
    let lemmas = crate::proof::take_captured_proof(&mut sub);
    crate::internal::release(*sub);
    match res {
        20 => {
            // a clause the unit satisfies is never needed under it; the
            // others in the formula's numbers
            let mut out: Vec<Vec<i32>> = Vec::with_capacity(lemmas.len() + 1);
            for c in lemmas {
                if c.is_empty() || c.contains(&unit) {
                    continue;
                }
                *work += c.len() as u64 + 1;
                out.push(c.iter().map(|&l| if l < 0 { -back[(-l) as usize] } else { back[l as usize] }).collect());
            }
            out.push(Vec::new());
            Verdict::Refuted(out)
        }
        10 => Verdict::Sat,
        _ => Verdict::Unknown,
    }
}

/// The carry chain of a + ~b (LSB first): cs[i] is the carry into bit i+1,
/// so cs[w-1] says a > b. Its defining clauses in the order they are added
/// by `red`: each fresh carry's "-> c" clauses with the witness c = 0
/// first, then its "c ->" clauses with c = 1.
fn carry_chain(a: &[i32], b: &[i32], next_var: &mut i32) -> (Vec<i32>, Vec<(Vec<i32>, i32, bool)>) {
    let w = a.len();
    let cs: Vec<i32> = (0..w)
        .map(|_| {
            *next_var += 1;
            *next_var
        })
        .collect();
    let mut defs: Vec<(Vec<i32>, i32, bool)> = Vec::new();
    let c1 = cs[0];
    defs.push((vec![-c1, a[0]], c1, false));
    defs.push((vec![-c1, -b[0]], c1, false));
    defs.push((vec![c1, -a[0], b[0]], c1, true));
    for i in 1..w {
        let (c, ci) = (cs[i], cs[i - 1]);
        defs.push((vec![-c, a[i], -b[i]], c, false));
        defs.push((vec![-c, a[i], ci], c, false));
        defs.push((vec![-c, -b[i], ci], c, false));
        defs.push((vec![c, -a[i], b[i]], c, true));
        defs.push((vec![c, -a[i], -ci], c, true));
        defs.push((vec![c, b[i], -ci], c, true));
    }
    (cs, defs)
}

// ---------------------------------------------------------------------------
// the proof of the count
// ---------------------------------------------------------------------------

fn sum_expr(ids: &[(u64, i64)]) -> String {
    let mut s = String::new();
    for (n, &(id, w)) in ids.iter().enumerate() {
        if n > 0 {
            s.push(' ');
        }
        if w == 1 {
            s.push_str(&id.to_string());
        } else {
            s.push_str(&format!("{} {} *", id, w));
        }
        if n > 0 {
            s.push_str(" +");
        }
    }
    s
}

/// Steps 4 of the module notes, after the sortedness constraints `sorted`
/// (per side and adjacent pair: value(out_k+1) - value(out_k) >= 0).
/// Returns the id of the empty clause.
#[allow(clippy::too_many_arguments)]
fn write_count(
    w: &mut Writer,
    f: &Formula,
    groups: &Groups,
    s: &Sorter,
    sorted: &[Vec<u64>],
    next_var: &mut i32,
    work: &mut u64,
    stopped: &dyn Fn(u64) -> bool,
) -> std::io::Result<Option<u64>> {
    let wd = s.w;
    let m = (1i64 << wd) - 1;
    let t_max = m as usize;
    // every loop below looks at the limit after each step it charges
    // (a threshold, a unit's count at one threshold, a node of a bit's
    // tree: Codex review round 5)
    macro_rules! poll {
        () => {
            if stopped(*work) {
                return Ok(None);
            }
        };
    }
    // the input clauses a sum may add by id: the multiplexers', with their
    // literals as written (a clause with a repeated literal is derived)
    let mut clause_id: HashMap<Vec<i32>, u64> = HashMap::new();
    for un in &s.units {
        for wid in [un.c, un.d] {
            for &v in &s.words[wid] {
                for &ci in groups.of(v) {
                    *work += f.clause(ci as usize).len() as u64 + 1;
                    poll!();
                    if !f.dup[ci as usize] {
                        let mut c = f.clause(ci as usize).to_vec();
                        c.sort_unstable();
                        clause_id.entry(c).or_insert(ci as u64 + 1);
                    }
                }
            }
        }
        if stopped(*work) {
            return Ok(None);
        }
    }
    // the thresholds of every word of either side
    let mut used: Vec<usize> = (0..s.n).collect();
    for (units, _) in &s.sides {
        for &u in units {
            let un = &s.units[u];
            used.extend([un.c, un.d]);
        }
    }
    used.sort_unstable();
    used.dedup();
    let slot: HashMap<usize, usize> = used.iter().enumerate().map(|(i, &wid)| (wid, i)).collect();
    let tables = 3 * (used.len() * t_max) as u64;
    if stopped(work.saturating_add(tables)) {
        return Ok(None);
    }
    *work += tables;
    let mut gv = vec![0i32; used.len() * t_max];
    let mut g1 = vec![0u64; used.len() * t_max];
    let mut g2 = vec![0u64; used.len() * t_max];
    let at = |wid: usize, t: usize| slot[&wid] * t_max + (t - 1);
    for &wid in &used {
        let bits = &s.words[wid];
        for t in 1..=t_max {
            *next_var += 1;
            let g = *next_var;
            let i = at(wid, t);
            gv[i] = g;
            let mut terms: Vec<(i64, i32)> = bits.iter().enumerate().map(|(b, &v)| (1i64 << b, v)).collect();
            terms.push((t as i64, -g));
            g1[i] = w.red_terms(&terms, t as i64, g, false)?;
            let mut terms: Vec<(i64, i32)> = bits.iter().enumerate().map(|(b, &v)| (1i64 << b, -v)).collect();
            terms.push((m - t as i64 + 1, g));
            g2[i] = w.red_terms(&terms, m - t as i64 + 1, g, true)?;
            *work += 2 * (wd as u64 + 1);
            poll!();
        }
    }
    // per unit and t: the count is kept
    let mut ge: HashMap<(usize, usize), u64> = HashMap::new();
    let mut le: HashMap<(usize, usize), u64> = HashMap::new();
    for (units, _) in &s.sides {
        for &u in units {
            let un = &s.units[u];
            let sel = un.sel;
            let (xw, yw, cw, dw) = (&s.words[un.x], &s.words[un.y], &s.words[un.c], &s.words[un.d]);
            let mut per_j: Vec<Vec<(u64, i64)>> = vec![Vec::new(); 8];
            for i in 0..wd {
                let (x, y, c, d) = (s.res(xw[i]), s.res(yw[i]), cw[i], dw[i]);
                let cls: [Vec<i32>; 8] = [
                    vec![-sel, -y, c],
                    vec![-sel, y, -c],
                    vec![sel, -x, c],
                    vec![sel, x, -c],
                    vec![-sel, -x, d],
                    vec![-sel, x, -d],
                    vec![sel, -y, d],
                    vec![sel, y, -d],
                ];
                for (j, cl) in cls.iter().enumerate() {
                    let mut key = cl.clone();
                    key.sort_unstable();
                    let id = match clause_id.get(&key) {
                        Some(&id) => id,
                        None => w.rup(cl)?,
                    };
                    per_j[j].push((id, 1i64 << i));
                }
                *work += 8 * 4;
                poll!();
            }
            let mut l = [0u64; 8];
            for j in 0..8 {
                l[j] = w.pol(&sum_expr(&per_j[j]))?;
            }
            *work += 8 * wd as u64;
            poll!();
            for t in 1..=t_max {
                let (ix, iy, ic, id) = (at(un.x, t), at(un.y, t), at(un.c, t), at(un.d, t));
                // "case and g(src) -> g(dst)": G1(src) + L_j + G2(dst), divided by M
                for &(src, j, dst) in &[(iy, 0usize, ic), (ic, 1, iy), (ix, 2, ic), (ic, 3, ix), (ix, 4, id), (id, 5, ix), (iy, 6, id), (id, 7, iy)] {
                    w.pol(&format!("{} {} + {} + {} d", g1[src], l[j], g2[dst], m))?;
                }
                let (gx, gy, gc, gd) = (gv[ix], gv[iy], gv[ic], gv[id]);
                ge.insert(
                    (u, t),
                    w.pbc(&[(1, gc), (1, gd), (1, -gx), (1, -gy)], 2, &[vec![sel, -gx], vec![sel], vec![-gx], vec![]])?,
                );
                le.insert(
                    (u, t),
                    w.pbc(&[(1, -gc), (1, -gd), (1, gx), (1, gy)], 2, &[vec![sel, gx], vec![sel], vec![gx], vec![]])?,
                );
                *work += 40;
                poll!();
            }
        }
    }
    // per t: the sums per side, the two sides against each other, the
    // sorted thresholds, and the two sides' thresholds equal by position
    let pos_word: Vec<&Vec<usize>> = s.sides.iter().map(|(_, o)| o).collect();
    for t in 1..=t_max {
        let mut cnt: Vec<(u64, u64)> = Vec::new();
        for (units, _) in &s.sides {
            let a: Vec<(u64, i64)> = units.iter().map(|&u| (ge[&(u, t)], 1)).collect();
            let b: Vec<(u64, i64)> = units.iter().map(|&u| (le[&(u, t)], 1)).collect();
            cnt.push((w.pol(&sum_expr(&a))?, w.pol(&sum_expr(&b))?));
            *work += 2 * units.len() as u64;
        }
        w.pol(&format!("{} {} +", cnt[0].0, cnt[1].1))?;
        w.pol(&format!("{} {} +", cnt[1].0, cnt[0].1))?;
        poll!();
        for si in 0..2 {
            let o = pos_word[si];
            for k in 0..s.n - 1 {
                w.pol(&format!("{} {} + {} + {} d", sorted[si][k], g1[at(o[k], t)], g2[at(o[k + 1], t)], m))?;
            }
        }
        for &(_, a, b, _) in &s.pairs {
            w.rup(&[-gv[at(a, t)], gv[at(b, t)]])?;
            w.rup(&[-gv[at(b, t)], gv[at(a, t)]])?;
        }
        *work += 4 * s.n as u64;
        poll!();
    }
    // the bits from the thresholds, top bit first, over the values of the higher bits
    /// Ok(false) when the limit stopped it.
    fn tree(w: &mut Writer, a: &[i32], b: &[i32], i: usize, fixed: &mut Vec<(usize, bool)>, work: &mut u64, stopped: &dyn Fn(u64) -> bool) -> std::io::Result<bool> {
        let wd = a.len();
        if fixed.len() < wd - 1 - i {
            let j = wd - 1 - fixed.len();
            for v in [false, true] {
                fixed.push((j, v));
                let done = tree(w, a, b, i, fixed, work, stopped)?;
                fixed.pop();
                if !done {
                    return Ok(false);
                }
            }
        }
        let mut base: Vec<i32> = fixed.iter().map(|&(j, v)| if v { -a[j] } else { a[j] }).collect();
        base.extend([-a[i], b[i]]);
        w.rup(&base)?;
        let k = base.len();
        base[k - 2] = a[i];
        base[k - 1] = -b[i];
        w.rup(&base)?;
        *work += 2 * k as u64;
        Ok(!stopped(*work))
    }
    for &(_, a, b, _) in &s.pairs {
        let (aw, bw) = (&s.words[a], &s.words[b]);
        for i in (0..wd).rev() {
            if !tree(w, aw, bw, i, &mut Vec::new(), work, stopped)? {
                return Ok(None);
            }
        }
    }
    // the compares are false, and with them the root
    for (_, a, _, xors) in &s.pairs {
        for (i, &(x, xnor)) in xors.iter().enumerate() {
            let abit = s.words[*a][i];
            if xnor {
                w.rup(&[x, abit])?;
                w.rup(&[x])?;
            } else {
                w.rup(&[-x, abit])?;
                w.rup(&[-x])?;
            }
            *work += 4;
        }
        poll!();
    }
    Ok(Some(w.rup(&[])?))
}

// ---------------------------------------------------------------------------
// the pass
// ---------------------------------------------------------------------------

/// Steps 3 and 4 of the module notes on the reading `s`: the sortedness
/// lemmas, then (with a proof) the count. Some(id of the empty clause, or
/// 0 without a proof) when every step went through.
#[allow(clippy::too_many_arguments)]
fn refute(f: &Formula, groups: &Groups, s: &Sorter, mut proof: Option<&mut Writer>, next_var: &mut i32, budget: u64, work: &mut u64, stop: &AtomicBool) -> Option<u64> {
    let stopped = |w: u64| w >= budget || stop.load(Ordering::Relaxed);
    let dbg = debug();
    let start = *work;
    let mut sorted: Vec<Vec<u64>> = vec![Vec::new(), Vec::new()];
    let mut lemma_clauses = 0usize;
    for si in 0..2 {
        let base = side_clauses(f, groups, s, si, work, &stopped)?;
        let order = s.sides[si].1.clone();
        for k in 0..s.n - 1 {
            if stopped(*work) {
                return None;
            }
            let a: Vec<i32> = s.words[order[k]].clone();
            let b: Vec<i32> = s.words[order[k + 1]].clone();
            // (the copy here and the sub-solver's import of it)
            let size: u64 = 2 * base.iter().map(|c| c.len() as u64 + 1).sum::<u64>() + 64 * s.w as u64;
            if stopped(work.saturating_add(size)) {
                return None;
            }
            *work += size;
            let (cs, defs) = carry_chain(&a, &b, next_var);
            let carry_out = cs[s.w - 1];
            let mut clauses = base.clone();
            clauses.extend(defs.iter().map(|(c, _, _)| c.clone()));
            // (the full solver, not kitten: on the 2025 cells it is five
            // times faster here and its clauses are an eighth of the bytes,
            // BubbleVsPancakeSort_8_4 1.2 s and 4 MB against 6.4 s and 33 MB)
            let before = *work;
            let left = budget.saturating_sub(*work);
            let verdict = solver_refute(&clauses, carry_out, SOLVER_CONFLICTS, left, work, stop);
            if dbg {
                let n = if let Verdict::Refuted(st) = &verdict { st.len() } else { 0 };
                eprintln!("structsort: the full solver on side {} pair {}: {} clauses, work {}K", si, k, n, (*work - before) / 1000);
            }
            let steps = match verdict {
                Verdict::Refuted(steps) => steps,
                Verdict::Sat => {
                    if dbg {
                        eprintln!("structsort: side {} pair {} is not in order: the pass declines", si, k);
                    }
                    return None;
                }
                Verdict::Unknown => {
                    if dbg {
                        eprintln!("structsort: side {} pair {}: no verdict within the budget", si, k);
                    }
                    return None;
                }
            };
            lemma_clauses += steps.len();
            if let Some(w) = proof.as_deref_mut() {
                // the carry chain's definitions, then "no carry out" by
                // contradiction with the solver's clauses, then the linear
                // sum; written whole or not at all, so its cost is reserved
                // before the first line (a pbc block cannot stop halfway)
                let block: u64 = steps.iter().map(|c| c.len() as u64 + 1).sum::<u64>() + 4 * defs.len() as u64 + 4 * s.w as u64;
                if stopped(work.saturating_add(block)) {
                    return None;
                }
                *work += block;
                let mut ids: Vec<u64> = Vec::with_capacity(defs.len());
                for (clause, var, value) in &defs {
                    ids.push(w.red_const(clause, *var, *value).ok()?);
                }
                let no_carry = w.pbc(&[(1, -carry_out)], 1, &steps).ok()?;
                let mut cells: Vec<u64> = Vec::with_capacity(s.w);
                cells.push(w.pol(&format!("{} x{} +", ids[2], cs[0])).ok()?);
                for i in 1..s.w {
                    let j = 3 + 6 * (i - 1);
                    cells.push(w.pol(&format!("{} {} + {} + 2 d", ids[j + 3], ids[j + 4], ids[j + 5])).ok()?);
                }
                let mut weights: Vec<(u64, i64)> = cells.iter().enumerate().map(|(i, &id)| (id, 1i64 << i)).collect();
                weights.push((no_carry, 1i64 << s.w));
                sorted[si].push(w.pol(&sum_expr(&weights)).ok()?);
            }
        }
    }
    if dbg {
        eprintln!("structsort: every pair in order ({} clauses from the solver); work {}K", lemma_clauses, (*work - start) / 1000);
    }
    match proof {
        Some(w) => match write_count(w, f, groups, s, &sorted, next_var, work, &stopped) {
            Ok(Some(id)) => Some(id),
            _ => {
                if dbg {
                    eprintln!("structsort: the count's proof stopped (budget or a write)");
                }
                None
            }
        },
        None => Some(0),
    }
}

pub fn run(f: &Formula, derived: &mut Derived, budget: u64, _hard: u64, work: &mut u64, stop: &AtomicBool) {
    let stopped = |w: u64| w >= budget || stop.load(Ordering::Relaxed);
    if stopped(*work) || f.vars > MAX_VARS || derived.refuted {
        return;
    }
    let start = *work;
    let dbg = debug();
    // the grouping and the root's scan in one piece: their tables (three
    // per variable, one slot per clause), a visit per literal and clause,
    // and the scan for units
    let cost = 3 * (f.vars as u64 + 1) + 3 * f.len() as u64 + f.lits.len() as u64;
    if stopped(work.saturating_add(cost)) {
        return;
    }
    let groups = group_clauses(f, work);
    let Some(top) = read_top(f, &groups, work, &stopped) else {
        if dbg {
            eprintln!("structsort: no miter of xor compares at the root");
        }
        return;
    };
    let s = match read_sorter(f, &groups, &top, work, &stopped) {
        Ok(s) => s,
        Err(why) => {
            if dbg {
                eprintln!("structsort: {}", why);
            }
            return;
        }
    };
    if dbg {
        eprintln!(
            "structsort: {} words of {} bits; side 0: {} units, side 1: {} units",
            s.n,
            s.w,
            s.sides[0].0.len(),
            s.sides[1].0.len()
        );
    }
    // the fresh variables (carries, then thresholds) come after every
    // number used so far, and the counter goes back to `derived` on every
    // path: a later pass must not reuse a number this pass's proof holds
    let mut next_var: i32 = derived.last_var.max(f.vars);
    let outcome = refute(f, &groups, &s, derived.proof.as_mut(), &mut next_var, budget, work, stop);
    derived.last_var = next_var;
    let Some(id) = outcome else { return };
    if stopped(*work) {
        return;
    }
    derived.add(&[], id);
    derived.notes.push(format!(
        "sorter: two swap networks of {} and {} units over {} words of {} bits, both sorted: refuted, work {}K",
        s.sides[0].0.len(),
        s.sides[1].0.len(),
        s.n,
        s.w,
        (*work - start) / 1000
    ));
}
