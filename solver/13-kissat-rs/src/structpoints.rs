//! Not in kissat. The point step of the structure pass (bead
//! SAT-playground-1v2.3.4, option structpoints): a formula whose variables
//! are the lines between n points, with every renumbering of the points a
//! symmetry, gets one lex-leader chain per swap of neighbouring points. The
//! chains and their proof follow Satsuma 1.4's Johnson step and its dense
//! lex order (MIT, Markus Anders); the detection is our own.
//!
//! In plain terms. Colour every line between 18 points red or blue with no
//! red triangle and no blue group of six (ramsey_3_6_18): renumbering the
//! points gives the same puzzle, so every colouring has up to 18! renamed
//! copies and the search refutes each dead end once per copy. Read the
//! lines in a fixed order, (0,1), (0,2), ..., (1,2), ..., as a string of
//! bits; of all the copies of a colouring, one comes first. For each swap of
//! points p and p+1 a chain of clauses says: the string must not come after
//! the string of the copy with p and p+1 swapped. The first copy passes
//! every chain, so a solution survives whenever there is one, and the
//! search has 18! fewer places to look (a toy R(3,5) on 14 points: no answer
//! in 300 s without the chains, 13 conflicts with them).
//!
//! What the step does, in order:
//! 1. runs first of the structure passes (src/structure.rs): the parity and
//!    symmetry passes log redundance steps with symmetries as witnesses,
//!    which would have to map its chains too, so once it has added chains
//!    they stay out, and otherwise they run as they would without it. It
//!    needs a formula no pass has touched yet (the dominance steps map the
//!    input onto itself), and it stands down if one did;
//! 2. applies the formula's own units (VeriPB applies them itself, so they
//!    need no step) and colours the literals by the symmetry pass's
//!    refinement, for at most MAX_ROUNDS rounds (the Ramsey cells settle in
//!    one or two; past that the step gives up);
//! 3. takes a class of n(n-1)/2 literals, one per variable, 5 <= n <=
//!    MAX_POINTS, or both literals of n(n-1)/2 variables (red and blue then
//!    play the same part, and flipping every line is a symmetry too), and
//!    counts how often two members share a clause: lines with a point in
//!    common and lines without one occur together a different number of
//!    times, so the count splits the other members of a line into the
//!    2(n-2) that meet it and the rest. Each point is then a star of n-1
//!    lines that meet pairwise: two lines that meet and the lines that meet
//!    both, but for the one that closes a triangle with them, are the star of
//!    their common point, and each of its lines leads to the star of its
//!    other end. Every line must lie in exactly two stars;
//! 4. checks every swap of neighbouring points against every clause (line
//!    (p,c) goes to line (p+1,c), complements alike, everything else stays):
//!    the n-1 swaps generate every renumbering. A swap that fails, or a line
//!    shape that is no complete graph, ends the step;
//! 5. adds the chains. The order is on variables, lines (i,j) with i < j by
//!    i then j. A chain compares, along the swap's lines in that order, each
//!    variable with the literal the swap sends it to: x_1 <= y_1, and while
//!    they agree x_2 <= y_2, and so on, through helper variables a_i ("equal
//!    so far", forced true when the pair agrees): (~x_1 | y_1), then per
//!    step (y_i | ~a_(i-1) | a_i), (~x_i | ~a_(i-1) | a_i), (~a_i | ~x_(i+1)
//!    | y_(i+1)). It stops after the last line whose swapped partner has not
//!    come yet; a partner right after its line is a step of its own, which
//!    the proof needs. With the flip, one more chain: the first line is blue;
//! 6. proves them (VeriPB 3): the order as one linear constraint with powers
//!    of two (def_order, load_order); per chain a `dom` step derives
//!    "assignment <= its swapped copy" as a linear constraint with the swap
//!    as witness; each chain clause follows by rup; each helper is defined by
//!    red steps on itself, two kept and two (a_i -> a_(i-1), a_i -> y_i ->
//!    x_i) only for the proof; a pol step adds K times the last of these to
//!    the linear constraint, which then speaks only of the lines still to
//!    come. The proof-only steps are deleted after each chain, the order is
//!    unloaded, and the kept clauses join F' (src/structproof.rs).
//!
//! The helper variables are numbers after every one the formula and the
//! earlier passes used (`Derived::last_var`), and they are variables of F':
//! the search sees them, the model printed leaves them out (it covers the
//! input's variables only), and the derived file's header counts them.
//!
//! Not yet: points that extra clauses hold apart (a fixed line, a constraint
//! on a few lines): Satsuma moves per-point blocks of other variables along
//! with a swap; here every swap moves the class's lines alone.

use crate::structsym::{canon, cells, idx, Work};
use crate::structure::{Derived, Formula};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

const MIN_POINTS: usize = 5; // the star recovery needs n >= 5
const MAX_POINTS: usize = 64; // 2016 lines; the order's coefficients reach 2^2015
/// Refinement rounds before the step gives up: the 2025 Ramsey cells settle
/// in one or two, and a formula that needs many more has classes too fine
/// for the shape (measured 2026-11-05: up to 394 rounds, and the whole 1G
/// limit, on cells without points)
const MAX_ROUNDS: u32 = 8;
/// The units, and the refinement, may cost this many passes over the
/// formula (literals, clauses and variables) before the step gives up; the
/// Ramsey cells have no units and refine in about four passes' work.
const UNIT_PASSES: u64 = 4;
const REFINE_PASSES: u64 = 8;

fn debug() -> bool {
    std::env::var_os("SAT_STRUCT_DEBUG").is_some()
}

/// n when m = n(n-1)/2 for some n in MIN_POINTS..=MAX_POINTS.
fn points_for(m: usize) -> Option<usize> {
    (MIN_POINTS..=MAX_POINTS).find(|&n| n * (n - 1) / 2 == m)
}

/// The point structure found: `lit[k]` the literal of class member k (one
/// per variable), `line[i * n + j]` the member that is line (i,j).
struct Points {
    n: usize,
    lit: Vec<i32>,
    line: Vec<u32>,
    flip: bool,
}

impl Points {
    fn at(&self, i: usize, j: usize) -> i32 {
        self.lit[self.line[i * self.n + j] as usize]
    }
}

/// Decimal strings of 2^0 .. 2^(n-1).
fn pow2_table(n: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(n);
    let mut limbs: Vec<u32> = vec![1]; // base 10^9, least significant first
    for _ in 0..n {
        let mut s = limbs.last().unwrap().to_string();
        for l in limbs.iter().rev().skip(1) {
            s.push_str(&format!("{:09}", l));
        }
        out.push(s);
        let mut carry = 0u64;
        for l in limbs.iter_mut() {
            let v = *l as u64 * 2 + carry;
            *l = (v % 1_000_000_000) as u32;
            carry = v / 1_000_000_000;
        }
        if carry > 0 {
            limbs.push(carry as u32);
        }
    }
    out
}

/// Member k's counts with every member: how many active clauses hold both
/// (k's own clauses only); None at the budget.
fn row_of(w: &mut Work, lits: &[i32], at: &[u32], k: usize) -> Option<Vec<u32>> {
    if !w.fits(lits.len() as u64) {
        return None;
    }
    let mut row = vec![0u32; lits.len()];
    let mut members: Vec<u32> = Vec::new();
    let li = idx(lits[k]);
    w.work += lits.len() as u64;
    for oi in 0..w.occ[li].len() {
        let i = w.occ[li][oi] as usize;
        if w.satisfied[i] {
            continue;
        }
        let len = w.f.clause(i).len() as u64 + 1;
        if !w.fits(len) {
            return None;
        }
        members.clear();
        for &l in w.f.clause(i) {
            if w.lit_value(l) == 0 && at[idx(l)] != u32::MAX {
                members.push(at[idx(l)]);
            }
        }
        members.sort_unstable();
        members.dedup();
        w.work += len;
        if w.over() {
            return None; // over() also publishes the work (Codex review round 4)
        }
        for &j in &members {
            row[j as usize] += 1;
        }
    }
    row[k] = 0;
    Some(row)
}

/// How often two class members share an active clause, as an m by m table,
/// over the members' clauses; None at the budget.
fn co_occurrence(w: &mut Work, lits: &[i32], at: &[u32]) -> Option<Vec<u32>> {
    let m = lits.len();
    let tables = (m * m) as u64 + w.f.len() as u64; // the table and the clause marks
    if !w.fits(tables) {
        return None;
    }
    let mut co = vec![0u32; m * m];
    let mut done = vec![false; w.f.len()];
    w.work += tables;
    let mut members: Vec<u32> = Vec::new();
    for &l in lits {
        let li = idx(l);
        for oi in 0..w.occ[li].len() {
            let i = w.occ[li][oi] as usize;
            if w.satisfied[i] || done[i] {
                continue;
            }
            done[i] = true;
            let len = w.f.clause(i).len() as u64 + 1;
            if !w.fits(len) {
                return None;
            }
            members.clear();
            for &x in w.f.clause(i) {
                if w.lit_value(x) == 0 && at[idx(x)] != u32::MAX {
                    members.push(at[idx(x)]);
                }
            }
            members.sort_unstable();
            members.dedup();
            w.work += len;
            let pairs = (members.len() * members.len()) as u64;
            if !w.fits(pairs) {
                return None;
            }
            w.work += pairs;
            if w.over() {
                return None; // over() also publishes the work (Codex review round 4)
            }
            for a in 0..members.len() {
                for b in a + 1..members.len() {
                    let (x, y) = (members[a] as usize, members[b] as usize);
                    co[x * m + y] += 1;
                    co[y * m + x] += 1;
                }
            }
        }
    }
    Some(co)
}

/// The points from the "meet" graph (sorted adjacency lists): the line
/// graph of the complete graph on n points, or None.
fn stars(adj: &[Vec<u32>], n: usize) -> Option<Vec<u32>> {
    let m = adj.len();
    let meets = |a: u32, b: u32| adj[a as usize].binary_search(&b).is_ok();
    // line 0 and its first neighbour share a point; the lines meeting both
    // are that point's other n-3 lines and the one closing the triangle,
    // which meets none of the others
    let (a0, b0) = (0u32, adj[0][0]);
    let common: Vec<u32> = adj[0].iter().copied().filter(|&c| meets(b0, c)).collect();
    if common.len() != n - 2 {
        return None;
    }
    let lone: Vec<u32> = common.iter().copied().filter(|&c| !common.iter().any(|&d| d != c && meets(c, d))).collect();
    if lone.len() != 1 {
        return None;
    }
    let mut star0: Vec<u32> = common.iter().copied().filter(|&c| c != lone[0]).collect();
    star0.push(a0);
    star0.push(b0);
    star0.sort_unstable();
    // point 0 is star0's; point j is the other star through star0's line j-1
    let mut all: Vec<Vec<u32>> = vec![star0.clone()];
    for &l in &star0 {
        let mut s: Vec<u32> = adj[l as usize].iter().copied().filter(|x| star0.binary_search(x).is_err()).collect();
        s.push(l);
        s.sort_unstable();
        if s.len() != n - 1 {
            return None;
        }
        all.push(s);
    }
    // every line in exactly two stars, every two stars sharing one line
    let mut owner: Vec<(u32, u32)> = vec![(u32::MAX, u32::MAX); m];
    for (si, s) in all.iter().enumerate() {
        for &l in s {
            let o = &mut owner[l as usize];
            if o.0 == u32::MAX {
                o.0 = si as u32;
            } else if o.1 == u32::MAX {
                o.1 = si as u32;
            } else {
                return None;
            }
        }
    }
    let mut line = vec![u32::MAX; n * n];
    for (l, &(p, q)) in owner.iter().enumerate() {
        if q == u32::MAX || p == q {
            return None;
        }
        let (p, q) = (p as usize, q as usize);
        if line[p * n + q] != u32::MAX {
            return None;
        }
        line[p * n + q] = l as u32;
        line[q * n + p] = l as u32;
    }
    Some(line)
}

/// The point structure on a class of literals (one per variable), checked
/// against every clause, or None.
fn detect(w: &mut Work, lits: Vec<i32>, n: usize, flip: bool) -> Option<Points> {
    let m = lits.len();
    if !w.fits(2 * w.value.len() as u64) {
        return None;
    }
    let mut at = vec![u32::MAX; 2 * w.value.len()];
    for (k, &l) in lits.iter().enumerate() {
        at[idx(l)] = k as u32;
    }
    w.work += 2 * w.value.len() as u64;
    // member 0's counts with the others, from its own clauses first: two
    // values, the meeting one on 2(n-2) members (n = 7 has 10 and 10: both
    // are tried); a class that fails this costs no full count
    let row0 = row_of(w, &lits, &at, 0)?;
    let mut vals: Vec<(u32, usize)> = Vec::new();
    for j in 1..m {
        let v = row0[j];
        match vals.iter_mut().find(|(x, _)| *x == v) {
            Some(e) => e.1 += 1,
            None => vals.push((v, 1)),
        }
    }
    let (meet_size, far_size) = (2 * (n - 2), (n - 2) * (n - 3) / 2);
    if vals.len() != 2 || !vals.iter().any(|&(_, c)| c == far_size) {
        return None;
    }
    let co = co_occurrence(w, &lits, &at)?;
    let mut tried = 0;
    for &(meet, size) in &vals {
        if size != meet_size {
            continue;
        }
        tried += 1;
        if !w.fits((m * m) as u64) {
            return None;
        }
        w.work += (m * m) as u64;
        let adj: Vec<Vec<u32>> = (0..m).map(|a| (0..m).filter(|&b| b != a && co[a * m + b] == meet).map(|b| b as u32).collect()).collect();
        if adj.iter().any(|v| v.len() != meet_size) {
            continue;
        }
        let Some(line) = stars(&adj, n) else { continue };
        let pts = Points { n, lit: lits.clone(), line, flip };
        // every swap of neighbouring points, checked against every clause
        let mut ok = true;
        for p in 0..n - 1 {
            let q = p + 1;
            let a: Vec<i32> = (0..n).filter(|&c| c != p && c != q).map(|c| pts.at(p, c)).collect();
            let b: Vec<i32> = (0..n).filter(|&c| c != p && c != q).map(|c| pts.at(q, c)).collect();
            if !w.verify_swap(&a, &b) {
                ok = false;
                break;
            }
        }
        if ok && flip && !verify_flip(w, &pts.lit) {
            ok = false;
        }
        if ok {
            return Some(pts);
        }
        if debug() {
            eprintln!("structpoints: {} points, a swap fails", n);
        }
        if w.over() {
            return None;
        }
    }
    if debug() && tried == 0 {
        eprintln!("structpoints: a class of {} literals, no meeting count on {} members", m, meet_size);
    }
    None
}

/// Every line flipped (x to ~x) maps every active clause onto one.
fn verify_flip(w: &mut Work, lits: &[i32]) -> bool {
    if !w.fits((w.f.len() + 2 * lits.len()) as u64) {
        return false;
    }
    let mut map: HashMap<i32, i32> = HashMap::with_capacity(2 * lits.len());
    for &l in lits {
        map.insert(l, -l);
        map.insert(-l, l);
    }
    let mut seen = vec![false; w.f.len()];
    w.work += (w.f.len() + 2 * lits.len()) as u64;
    for &l in lits {
        for li in [idx(l), idx(-l)] {
            for oi in 0..w.occ[li].len() {
                let i = w.occ[li][oi] as usize;
                if w.satisfied[i] || seen[i] {
                    continue;
                }
                if !w.fits(w.f.clause(i).len() as u64 + 1) {
                    return false;
                }
                seen[i] = true;
                let mut img: Vec<i32> = Vec::with_capacity(w.f.clause(i).len());
                for &x in w.f.clause(i) {
                    if w.lit_value(x) != 0 {
                        continue;
                    }
                    img.push(*map.get(&x).unwrap_or(&x));
                }
                w.work += img.len() as u64 + 1;
                canon(&mut img);
                if !w.clause_set.contains_key(&img) || w.over() {
                    return false;
                }
            }
        }
    }
    true
}

/// One chain: the steps (variable x, literal y = sigma(x)) and the swap as
/// a map from the variables of its lines to literals, in the order.
struct Chain {
    steps: Vec<(i32, i32)>,
    sigma: Vec<(i32, i32)>,
}

fn chains(p: &Points) -> (Vec<i32>, Vec<Chain>) {
    let n = p.n;
    // the order: lines (i,j), i < j, by i then j
    let mut order: Vec<i32> = Vec::with_capacity(n * (n - 1) / 2);
    let mut pos = vec![usize::MAX; n * n];
    for i in 0..n {
        for j in i + 1..n {
            pos[i * n + j] = order.len();
            pos[j * n + i] = order.len();
            order.push(p.at(i, j).abs());
        }
    }
    let mut out: Vec<Chain> = Vec::new();
    for a in 0..n - 1 {
        let b = a + 1;
        // the swap's lines in the order, each with its partner
        let mut lines: Vec<(usize, usize, usize)> = Vec::new(); // (position, i, j)
        for c in 0..n {
            if c != a && c != b {
                lines.push((pos[a * n + c], a, c));
                lines.push((pos[b * n + c], b, c));
            }
        }
        lines.sort_unstable();
        let partner = |i: usize, c: usize| if i == a { (b, c) } else { (a, c) };
        // sigma on variables: the variable of line e goes to the literal of
        // its partner, signed as e's literal is
        let mut sigma: Vec<(i32, i32)> = Vec::with_capacity(lines.len());
        for &(_, i, c) in &lines {
            let (x, y) = (p.at(i, c), { let (pi, pc) = partner(i, c); p.at(pi, pc) });
            sigma.push((x.abs(), if x > 0 { y } else { -y }));
        }
        // the steps: up to the last line whose partner has not come before it
        let mut last_new = 0;
        let mut seen = vec![false; 2 * n]; // (side, other end): side 0 is a, 1 is b
        for (k, &(_, i, c)) in lines.iter().enumerate() {
            let (pi, pc) = partner(i, c);
            if !seen[usize::from(pi == b) * n + pc] {
                last_new = k;
            }
            seen[usize::from(i == b) * n + c] = true;
        }
        let steps: Vec<(i32, i32)> = sigma[..=last_new].to_vec();
        out.push(Chain { steps, sigma });
    }
    if p.flip {
        let sigma: Vec<(i32, i32)> = order.iter().map(|&v| (v, -v)).collect();
        out.push(Chain { steps: vec![sigma[0]], sigma });
    }
    (order, out)
}

/// The work of writing the chains, as `add_chains` does it: computed before
/// it runs, so the block is written only when all of it fits the limit.
fn chain_cost(chs: &[Chain], nord: usize, proof: bool) -> u64 {
    let digits = (nord / 9 + 2) as u64; // about the words of one coefficient
    let mut cost = 0u64;
    if proof {
        cost += nord as u64 * (digits + 4); // the order's definition and its load
    }
    for ch in chs {
        if proof {
            cost += 2 * ch.sigma.len() as u64 * (digits + 2); // the dom line
        }
        for (i, &(x, y)) in ch.steps.iter().enumerate() {
            cost += 4; // the chain clause
            if i + 1 == ch.steps.len() || y == -x {
                break;
            }
            cost += 8; // the two kept helper clauses
            if proof {
                cost += digits + 16; // two proof-only steps, the pol, the delete
            }
        }
    }
    cost
}

/// Add the chains to F', each clause with its proof step when there is a
/// proof. The caller charges `chain_cost`. Returns (chains, clauses,
/// helper variables).
fn add_chains(order: &[i32], chs: &[Chain], pow2: &[String], derived: &mut Derived) -> (usize, usize, usize) {
    let nord = order.len();
    let k_of: HashMap<i32, &str> = order.iter().enumerate().map(|(i, &v)| (v, pow2[nord - 1 - i].as_str())).collect();
    let neg_k: HashMap<i32, String> = order.iter().enumerate().map(|(i, &v)| (v, format!("-{}", pow2[nord - 1 - i]))).collect();
    let first_fresh = derived.last_var;
    let mut next_var = derived.last_var;
    let mut kept = 0usize;
    if let Some(w) = derived.proof.as_mut() {
        if let Ok(name) = w.def_lex_order(nord, pow2) {
            let _ = w.load_order(&name, Some(order));
        }
    }
    for ch in chs {
        let mut p_id: u64 = 0;
        if let Some(w) = derived.proof.as_mut() {
            // sum K(v) * (sigma(v) - x_v) >= 0 over the swap's variables
            let mut terms: Vec<(&str, i32)> = Vec::with_capacity(2 * ch.sigma.len());
            for &(v, y) in &ch.sigma {
                terms.push((k_of[&v], y));
                terms.push((neg_k[&v].as_str(), v));
            }
            p_id = w.dom(&terms, &ch.sigma).unwrap_or(0);
        }
        let mut prev: Option<i32> = None;
        let mut drop: Vec<u64> = Vec::new();
        for (i, &(x, y)) in ch.steps.iter().enumerate() {
            let mut body: Vec<i32> = prev.map(|a| -a).into_iter().collect();
            body.push(-x);
            if y != -x {
                body.push(y);
            }
            let id = derived.proof.as_mut().map(|w| w.rup(&body).unwrap_or(0)).unwrap_or(0);
            derived.add(&body, id);
            kept += 1;
            if i + 1 == ch.steps.len() || y == -x {
                break;
            }
            next_var += 1;
            let a = next_var;
            let mut c1: Vec<i32> = vec![y];
            let mut c2: Vec<i32> = vec![-x];
            if let Some(pa) = prev {
                c1.push(-pa);
                c2.push(-pa);
            }
            c1.push(a);
            c2.push(a);
            for c in [&c1, &c2] {
                let id = derived.proof.as_mut().map(|w| w.red_const(c, a, true).unwrap_or(0)).unwrap_or(0);
                derived.add(c, id);
                kept += 1;
            }
            if let Some(w) = derived.proof.as_mut() {
                if let Some(pa) = prev {
                    drop.push(w.red_const(&[-a, pa], a, false).unwrap_or(0));
                }
                let r = w.red_const(&[-a, -y, x], a, false).unwrap_or(0);
                drop.push(r);
                let p_new = w.pol(&format!("{} {} {} * +", p_id, r, k_of[&x])).unwrap_or(0);
                let _ = w.delete(&[p_id]);
                p_id = p_new;
            }
            prev = Some(a);
        }
        if let Some(w) = derived.proof.as_mut() {
            drop.push(p_id);
            let _ = w.delete(&drop);
        }
    }
    if let Some(w) = derived.proof.as_mut() {
        let _ = w.load_order("", None);
    }
    derived.last_var = next_var;
    (chs.len(), kept, (next_var - first_fresh) as usize)
}

pub fn run(f: &Formula, derived: &mut Derived, budget: u64, ticks: &mut u64, stop: &AtomicBool) {
    if derived.refuted {
        return;
    }
    if !derived.clauses.is_empty() {
        derived.notes.push(format!("points: skipped, the earlier passes derived {} clauses", derived.clauses.len()));
        return;
    }
    let start = *ticks;
    // the occurrence lists and tables Work::new builds, reserved in full
    // before they are built; then a budget check at every item
    let tables = f.lits.len() as u64 + 4 * (f.vars as u64 + 1) + f.len() as u64;
    if start.saturating_add(tables) > budget || stop.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let mut w = Work::new(f, budget, budget, ticks as *mut u64, stop);
    w.fine = true;
    w.work += tables;
    // each phase within a few passes over the formula: a formula whose units
    // cascade, or whose classes take many rounds to settle, is no point shape
    // worth the step's whole limit (measured 2026-11-05: the whole 1G on cells
    // without points, in the units or the refinement)
    let size = f.lits.len() as u64 + f.len() as u64 + f.vars as u64 + 1;
    // the formula's own units, applied without a proof step (VeriPB applies
    // them itself); a conflict is left to the search
    let mut scratch = Derived { clauses: Vec::new(), proof: None, refuted: false, notes: Vec::new(), last_var: derived.last_var };
    w.limit_to(budget.min(w.work.saturating_add(UNIT_PASSES * size)));
    for i in 0..f.len() {
        if w.satisfied[i] {
            continue;
        }
        let len = f.clause(i).len() as u64 + 1;
        if !w.fits(len) {
            *ticks = w.work;
            return;
        }
        let c = w.reduced(i);
        w.work += len;
        if w.over() || c.is_empty() {
            *ticks = w.work;
            return;
        }
        if c.len() == 1 && (!w.assign_and_propagate(c[0], &mut scratch) || w.over() || w.refused) {
            *ticks = w.work;
            return;
        }
    }
    if debug() {
        eprintln!("structpoints: units applied, work {}K", (w.work - start) / 1000);
    }
    w.round_cap = MAX_ROUNDS;
    w.limit_to(budget.min(w.work.saturating_add(REFINE_PASSES * size)));
    // the refinement's own tables (colours and marks per literal and
    // clause, the first round's lists), reserved before they are built; its
    // rounds then check the limit at every item and before every split
    let colour_tables = 6 * (f.vars as u64 + 1) + 3 * f.len() as u64;
    if !w.fits(colour_tables) {
        *ticks = w.work;
        return;
    }
    w.work += colour_tables;
    let base = w.refine_base();
    if debug() {
        eprintln!("structpoints: refined, work {}K{}", (w.work - start) / 1000, if w.capped { " (capped)" } else { "" });
    }
    if w.over() || w.capped || w.refused {
        if debug() && w.capped {
            eprintln!("structpoints: the refinement needs more than {} rounds", MAX_ROUNDS);
        }
        *ticks = w.work;
        return;
    }
    w.limit_to(budget);
    // the candidate classes, largest first: n(n-1)/2 literals one per
    // variable, or both literals of n(n-1)/2 variables. Building them visits
    // every literal, then sorts each class: reserved before, charged after
    // (Codex review round 4)
    let classes = 4 * (2 * (f.vars as u64 + 1));
    if !w.fits(classes) {
        *ticks = w.work;
        return;
    }
    w.work += classes;
    if w.over() {
        *ticks = w.work;
        return;
    }
    let mut cands: Vec<(usize, Vec<i32>, bool, u32)> = Vec::new();
    for (colour, mut lits) in cells(&base.cl, &w.value) {
        lits.sort_unstable_by_key(|&l| idx(l));
        let mut vars: Vec<u32> = lits.iter().map(|l| l.unsigned_abs()).collect();
        vars.dedup();
        if vars.len() == lits.len() {
            if let Some(n) = points_for(lits.len()) {
                cands.push((n, lits, false, colour));
            }
        } else if 2 * vars.len() == lits.len() {
            if let Some(n) = points_for(vars.len()) {
                cands.push((n, vars.iter().map(|&v| v as i32).collect(), true, colour));
            }
        }
    }
    // largest first, then by the first literal with its sign (a positive
    // class before its negative twin), then by colour: never the hash
    // map's order (Codex review round 1)
    cands.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| idx(a.1[0]).cmp(&idx(b.1[0]))).then_with(|| a.3.cmp(&b.3)));
    // the clause set the swap checks look up, built once there is a class
    // to check
    if cands.is_empty() || !w.rebuild_clause_set() || w.refused {
        *ticks = w.work;
        return;
    }
    let mut found: Option<Points> = None;
    for (n, lits, flip, _) in cands {
        if debug() {
            eprintln!("structpoints: a class of {} {}literals, {} points?", lits.len(), if flip { "flippable " } else { "" }, n);
        }
        if let Some(p) = detect(&mut w, lits, n, flip) {
            found = Some(p);
            break;
        }
        if w.over() || w.refused {
            break;
        }
    }
    let Some(p) = found else {
        *ticks = w.work;
        return;
    };
    // the chains and their proof are written in one piece (a proof cut half
    // way would leave the order loaded), so the whole block is reserved
    // first: building the chains and the powers of two, then writing them
    // (Codex review round 1)
    let n = p.n;
    let nord = n * (n - 1) / 2;
    let build = 8 * (n * n) as u64 + nord as u64 * (nord as u64 / 9 + 3);
    if !w.fits(build) {
        derived.notes.push(format!("points: {} points, no limit left for the chains, work {}K", n, (w.work - start) / 1000));
        *ticks = w.work;
        return;
    }
    let (order, chs) = chains(&p);
    let pow2 = pow2_table(nord);
    w.work += build;
    let cost = chain_cost(&chs, nord, derived.proof.is_some());
    if !w.fits(cost) {
        derived.notes.push(format!("points: {} points, no limit left for the chains, work {}K", n, (w.work - start) / 1000));
        *ticks = w.work;
        return;
    }
    let (nch, nkept, nhelp) = add_chains(&order, &chs, &pow2, derived);
    w.work += cost;
    *ticks = w.work;
    derived.notes.push(format!(
        "points: {} points, {} lex-leader chains{}, {} clauses ({} helper variables), work {}K",
        p.n,
        nch,
        if p.flip { " (one for the colour flip)" } else { "" },
        nkept,
        nhelp,
        (w.work - start) / 1000
    ));
}
