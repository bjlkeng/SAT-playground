//! Not in kissat. The counting pass, step two (bead SAT-playground-skz.6,
//! 2026-10-31): a count written as a decision diagram, over variables that
//! at-most-one groups cover, refuted when the diagram asks for more trues
//! than there are groups.
//!
//! The shape, in plain terms. The rook cells of sat-comp-2025 put n+1 rooks
//! on an n x n board: pairwise binary clauses allow at most one rook per
//! row and per column, and the count is a decision diagram of one-way ITE
//! nodes, `node -> (x ? hi : lo)`, one board variable per level, children
//! on later levels, the root asserted by a unit (135-176K nodes). Search
//! cannot make the counting argument (n+1 rooks on n rows); this pass does.
//!
//! What the pass does, in order (`tools/dd_toy_proof.py` is the checked
//! prototype):
//! 1. reads the diagram from a positive unit: a variable is a node when
//!    at most three clauses hold it negated and those clauses, with the
//!    node removed, are an ITE on one board variable (the other literals
//!    are nodes read the same way, or board literals); a clause that holds
//!    a node negated next to the variable (the way a board variable holds
//!    its level's nodes) ends the reading of that variable;
//! 2. orders the board variables by level: a node's variable comes before
//!    the variables of its clauses and of its children, and the order is
//!    a topological sort of those constraints (a node with two board
//!    variables of one sign each reads on the earlier one); children must
//!    sit strictly below their parent, or the pass declines;
//! 3. computes val(node), the least number of trues the node forces among
//!    the board variables at its level and below: val(hi) + 1 or val(lo),
//!    whichever is smaller (a false child counts as infinite, a true one
//!    as nothing, a board literal as one or nothing);
//! 4. partitions the board variables into groups, cliques of the pairwise
//!    at-most-one clauses, greedily in level order (a variable in no such
//!    clause is a group of its own); with fewer groups than val(root), the
//!    formula is unsatisfiable: the diagram wants val(root) trues at
//!    distinct levels and the groups allow one each;
//! 5. writes the proof. One fresh variable per level, w = x or w_next,
//!    says "this level's group has a true variable here or below" (three
//!    `red` steps), and one `rup` clause per level says a true x closes its
//!    group below (the pair clauses do it). Then, bottom up, every node
//!    gets the constraint "val(node) * ~node + the open groups' w >= val"
//!    (at most as many terms as there are groups, never the board's size)
//!    by a `pbc` of three lines: the lo child's constraint added to the
//!    negation (the lo child is impossible), the hi child's constraint
//!    added with x's clauses (impossible once x holds), and `rup >= 1`
//!    over the node's own clauses. At the root the sum has one term per
//!    group but asks for more: `pol` gives ~root, `rup` the empty clause.
//!    Without a proof file steps 1-4 are the whole check (as the counting
//!    pass refutes without one).
//!
//! Proof size on `rook-51-0-0`: 143K lines, 87 MB, VeriPB 22 s (the
//! prototype); the board-wide sums the 2026-10-14 notes priced at 175M
//! terms are not written.

use crate::structproof::Writer;
use crate::structure::{Derived, Formula};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};

/// A node is defined by at most this many clauses (ITE: three), each of
/// at most this many literals (the node, the level variable, a child).
const MAX_DEFS: usize = 3;
const MAX_CLAUSE: usize = 3;
const INF: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Child {
    True,
    False,
    /// A board literal (positive: one true; negative: nothing).
    Lit(i32),
    /// An index into the node list.
    Node(usize),
}

struct Node {
    var: i32,
    /// The defining clauses: (constraint id, literals without ~var).
    defs: Vec<(u64, Vec<i32>)>,
    /// The board variables in them (one or two).
    bvars: Vec<i32>,
    /// The child node variables (positive literals that read as nodes).
    children: Vec<i32>,
    x: i32,
    level: u32,
    hi: Child,
    lo: Child,
    /// The clause that gives hi once x holds, and lo once it does not.
    c1: u64,
    c2: u64,
    val: u32,
    parents: u32,
}

fn new_node(var: i32) -> Node {
    Node { var, defs: Vec::new(), bvars: Vec::new(), children: Vec::new(), x: 0, level: 0, hi: Child::True, lo: Child::True, c1: 0, c2: 0, val: 0, parents: 0 }
}

/// What the defining clauses leave when x takes `value`: nothing (true),
/// an empty clause (false), or one literal that is a unit among them and
/// sits in every other one; None when they are not an ITE on x.
fn residual(defs: &[(u64, Vec<i32>)], x: i32, value: bool) -> Option<(Option<i32>, bool, u64)> {
    let sat = if value { x } else { -x };
    let mut left: Vec<(Vec<i32>, u64)> = Vec::new();
    for (id, d) in defs {
        if d.contains(&sat) {
            continue;
        }
        left.push((d.iter().copied().filter(|&l| l.abs() != x).collect(), *id));
    }
    if left.is_empty() {
        return Some((None, true, 0));
    }
    for (rest, id) in &left {
        if rest.is_empty() {
            return Some((None, false, *id));
        }
    }
    for (rest, id) in &left {
        if rest.len() == 1 && left.iter().all(|(r, _)| r.contains(&rest[0])) {
            return Some((Some(rest[0]), true, *id));
        }
    }
    None
}

/// The diagram under one asserted root, read and checked.
struct Diagram {
    root: usize,
    members: Vec<usize>,
    /// Board variables by level.
    order: Vec<i32>,
    /// Group -> its levels ascending.
    groups: Vec<Vec<u32>>,
    group_of_level: Vec<usize>,
}

pub fn run(f: &Formula, derived: &mut Derived, budget: u64, ticks: &mut u64, stop: &AtomicBool) {
    let mut work = *ticks;
    let stopped = |work: u64| work > budget || stop.load(Ordering::Relaxed);
    let nv = f.vars.max(0) as usize;
    // the tables with an entry per variable are charged before they are
    // made, and the pass does not start when the limit does not cover them
    let table = nv as u64 + 1;
    if work.saturating_add(table) > budget || stop.load(Ordering::Relaxed) {
        return;
    }
    work += table;
    *ticks = work;
    // pass 1: how often each variable occurs negated, and the positive units
    let mut count: Vec<u32> = vec![0; nv + 1];
    let mut units: Vec<(i32, u64)> = Vec::new();
    let mut last_poll = work;
    for i in 0..f.len() {
        // the limits by work done since the last look, before any clause
        // is skipped and before a long clause is read
        let c = f.clause(i);
        work += 1;
        if work - last_poll >= 4096 {
            last_poll = work;
            *ticks = work;
            if stopped(work) {
                return;
            }
        }
        // a clause is read whole only when the limit covers it
        if work.saturating_add(c.len() as u64) > budget {
            *ticks = work;
            return;
        }
        if f.taut[i] || f.dup[i] {
            continue;
        }
        work += c.len() as u64;
        if c.len() == 1 && c[0] > 0 {
            units.push((c[0], i as u64 + 1));
        }
        for &l in c {
            if l < 0 {
                count[(-l) as usize] = count[(-l) as usize].saturating_add(1);
            }
        }
    }
    *ticks = work;
    if !units.iter().any(|&(v, _)| (1..=MAX_DEFS as u32).contains(&count[v as usize])) {
        return; // no asserted root of the right shape: the usual exit
    }
    // pass 2: the negative occurrences of the variables that may be nodes
    // (three more tables per variable: the offsets, their fill, the kinds)
    if work.saturating_add(3 * table) > budget || stop.load(Ordering::Relaxed) {
        *ticks = work;
        return;
    }
    work += 3 * table;
    *ticks = work;
    let mut off: Vec<u32> = vec![0; nv + 2];
    for v in 1..=nv {
        let n = if (1..=MAX_DEFS as u32).contains(&count[v]) { count[v] } else { 0 };
        off[v + 1] = off[v] + n;
    }
    let mut fill: Vec<u32> = off.clone();
    let mut occ: Vec<u32> = vec![0; off[nv + 1] as usize];
    last_poll = work;
    for i in 0..f.len() {
        let c = f.clause(i);
        work += 1;
        if work - last_poll >= 4096 {
            last_poll = work;
            *ticks = work;
            if stopped(work) {
                return;
            }
        }
        if work.saturating_add(c.len() as u64) > budget {
            *ticks = work;
            return;
        }
        if f.taut[i] || f.dup[i] {
            continue;
        }
        work += c.len() as u64;
        for &l in c {
            if l < 0 {
                let v = (-l) as usize;
                if off[v + 1] > off[v] {
                    occ[fill[v] as usize] = i as u32;
                    fill[v] += 1;
                }
            }
        }
    }
    let occ_of = |v: i32| -> &[u32] { &occ[off[v as usize] as usize..off[v as usize + 1] as usize] };
    // read nodes from the roots: a variable is read once its positive
    // literals (the children) are; 0 unknown, 1 on the path, 2 node, 3 not
    let mut kind: Vec<u8> = vec![0; nv + 1];
    let mut nodes: Vec<Node> = Vec::new();
    let mut node_of: HashMap<i32, usize> = HashMap::new();
    let mut stack: Vec<(i32, bool)> = Vec::new();
    last_poll = work;
    for &(root, _) in &units {
        if kind[root as usize] != 0 {
            continue;
        }
        stack.push((root, false));
        while let Some((v, built)) = stack.pop() {
            work += 1;
            // the limits by work done since the last look, before any
            // early continue
            if work - last_poll >= 1024 {
                last_poll = work;
                *ticks = work;
                if stopped(work) {
                    return;
                }
            }
            if !built {
                if kind[v as usize] != 0 {
                    continue;
                }
                let o = occ_of(v);
                // an ITE node's clauses have at most three literals: a
                // wider one ends the reading before any child is queued
                work += o.len() as u64;
                if o.is_empty() || o.iter().any(|&ci| f.clause(ci as usize).len() > MAX_CLAUSE) {
                    kind[v as usize] = 3;
                    continue;
                }
                kind[v as usize] = 1;
                stack.push((v, true));
                for &ci in o {
                    let c = f.clause(ci as usize);
                    work += c.len() as u64;
                    for &l in c {
                        if l > 0 && l != v && kind[l as usize] == 0 {
                            stack.push((l, false));
                        }
                    }
                }
                continue;
            }
            let mut nd = new_node(v);
            let mut ok = true;
            for &ci in occ_of(v) {
                let c = f.clause(ci as usize);
                work += c.len() as u64;
                let mut d: Vec<i32> = Vec::with_capacity(c.len());
                for &l in c {
                    if l == -v {
                        continue;
                    }
                    d.push(l);
                    let k = kind[l.unsigned_abs() as usize];
                    if k == 2 {
                        if l < 0 {
                            ok = false;
                        } else if !nd.children.contains(&l) {
                            if nd.children.len() == 2 {
                                ok = false; // a third child: not an ITE
                                break;
                            }
                            nd.children.push(l);
                        }
                    } else if k == 1 {
                        ok = false; // a cycle, or the level variable of a node on the path
                    } else if !nd.bvars.contains(&l.abs()) {
                        if nd.bvars.len() == 2 {
                            ok = false; // a third board variable: not an ITE
                            break;
                        }
                        nd.bvars.push(l.abs());
                    }
                }
                if !ok {
                    break;
                }
                nd.defs.push((ci as u64 + 1, d));
            }
            if ok && (1..=2).contains(&nd.bvars.len()) {
                kind[v as usize] = 2;
                node_of.insert(v, nodes.len());
                nodes.push(nd);
            } else {
                kind[v as usize] = 3;
            }
        }
        *ticks = work;
        if stopped(work) {
            return;
        }
    }
    if std::env::var_os("SAT_STRUCT_DEBUG").is_some() {
        eprintln!("structdd: {} nodes read under {} positive units", nodes.len(), units.len());
    }
    for &(root, _) in &units {
        if kind[root as usize] != 2 {
            continue;
        }
        // the helpers count on the published counter itself, so a signal
        // during them reports their work
        *ticks = work;
        let dd = match analyse(f, &mut nodes, &node_of, &kind, root, budget, ticks, stop) {
            Ok(Some(dd)) => dd,
            Ok(None) => {
                work = *ticks;
                continue;
            }
            Err(()) => return, // out of budget
        };
        work = *ticks;
        let need = nodes[dd.root].val;
        let g = dd.groups.len() as u32;
        if std::env::var_os("SAT_STRUCT_DEBUG").is_some() {
            eprintln!("structdd: root x{}: {} nodes, {} board variables, {} groups, root needs {}", root, dd.members.len(), dd.order.len(), g, need);
        }
        if need != INF && need <= g {
            continue; // the groups can hold what the diagram asks for
        }
        let mut id = 0u64;
        if derived.proof.is_some() {
            let mut next_var = derived.last_var.max(f.vars);
            *ticks = work;
            let written = {
                let w = derived.proof.as_mut().unwrap();
                prove(&dd, &nodes, w, &mut next_var, budget, ticks, stop)
            };
            derived.last_var = next_var;
            work = *ticks;
            match written {
                Some(empty) => id = empty,
                None => return, // the proof is incomplete: nothing is derived
            }
        }
        *ticks = work;
        derived.add(&[], id);
        derived.notes.push(format!(
            "decision-diagram count: {} nodes over {} board variables in {} at-most-one groups, the root needs {}, refuted",
            dd.members.len(),
            dd.order.len(),
            g,
            if need == INF { "the impossible".to_string() } else { need.to_string() }
        ));
        return;
    }
    *ticks = work;
}

/// Read the diagram under `root` and check it: Ok(None) when it is not a
/// diagram the pass handles, Err(()) when the budget ran out.
#[allow(clippy::too_many_arguments)]
fn analyse(f: &Formula, nodes: &mut [Node], node_of: &HashMap<i32, usize>, kind: &[u8], root: i32, budget: u64, work: &mut u64, stop: &AtomicBool) -> Result<Option<Diagram>, ()> {
    let stopped = |work: u64| work > budget || stop.load(Ordering::Relaxed);
    // the nodes under the root
    let ri = node_of[&root];
    let mut members: Vec<usize> = Vec::new();
    let mut seen: HashMap<usize, ()> = HashMap::new();
    let mut stack = vec![ri];
    let mut last_poll = *work;
    while let Some(ni) = stack.pop() {
        *work += 1;
        if *work - last_poll >= 4096 {
            last_poll = *work;
            if stopped(*work) {
                return Err(());
            }
        }
        if seen.insert(ni, ()).is_some() {
            continue;
        }
        members.push(ni);
        for &c in &nodes[ni].children {
            stack.push(node_of[&c]);
        }
    }
    // the board (at most two variables a node) and its tables are
    // reserved before they are made
    let bound = 2 * members.len() as u64;
    if work.saturating_add(2 * bound + 1) > budget || stop.load(Ordering::Relaxed) {
        return Err(());
    }
    *work += 2 * bound + 1;
    let mut board: Vec<i32> = members.iter().flat_map(|&ni| nodes[ni].bvars.iter().copied()).collect();
    board.sort_unstable();
    board.dedup();
    if board.iter().any(|&b| kind[b as usize] == 2) {
        return Ok(None); // a variable read both as a node and as a board variable
    }
    let nb = board.len();
    // the tables with an entry per board variable (index, edges, degrees,
    // levels, groups, adjacency)
    if work.saturating_add(6 * nb as u64 + 1) > budget {
        return Err(());
    }
    *work += 6 * nb as u64 + 1;
    let bix: HashMap<i32, usize> = board.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    // the level order: a node's variable before its clauses' other board
    // variables and its children's; a node with two board variables and
    // no sign that tells them apart adds nothing and reads on the earlier
    let mut after: Vec<Vec<usize>> = vec![Vec::new(); nb];
    let mut indeg: Vec<u32> = vec![0; nb];
    for &ni in &members {
        if *work - last_poll >= 4096 {
            last_poll = *work;
            if stopped(*work) {
                return Err(());
            }
        }
        let nd = &nodes[ni];
        let x = if nd.bvars.len() == 1 {
            Some(nd.bvars[0])
        } else {
            let both: Vec<i32> = nd.bvars.iter().copied().filter(|&b| nd.defs.iter().any(|(_, d)| d.contains(&b)) && nd.defs.iter().any(|(_, d)| d.contains(&-b))).collect();
            if both.len() == 1 {
                Some(both[0])
            } else {
                None
            }
        };
        let Some(x) = x else { continue };
        let xi = bix[&x];
        for &b in &nd.bvars {
            if b != x {
                after[xi].push(bix[&b]);
                indeg[bix[&b]] += 1;
            }
        }
        for &c in &nd.children {
            for &b in &nodes[node_of[&c]].bvars {
                after[xi].push(bix[&b]);
                indeg[bix[&b]] += 1;
            }
        }
        *work += nd.bvars.len() as u64 + nd.children.len() as u64;
    }
    let mut ready: BinaryHeap<Reverse<i32>> = board.iter().enumerate().filter(|&(i, _)| indeg[i] == 0).map(|(_, &b)| Reverse(b)).collect();
    let mut order: Vec<i32> = Vec::with_capacity(nb);
    let mut level: Vec<u32> = vec![0; nb];
    while let Some(Reverse(b)) = ready.pop() {
        level[bix[&b]] = order.len() as u32;
        order.push(b);
        for &t in &after[bix[&b]] {
            indeg[t] -= 1;
            if indeg[t] == 0 {
                ready.push(Reverse(board[t]));
            }
        }
        *work += 1 + after[bix[&b]].len() as u64;
        if *work - last_poll >= 4096 {
            last_poll = *work;
            if stopped(*work) {
                return Err(());
            }
        }
    }
    if order.len() != nb {
        return Ok(None); // no level order
    }
    if stopped(*work) {
        return Err(());
    }
    // every node as an ITE on its earliest board variable, children below
    let lvl = |b: i32| level[bix[&b]];
    for &ni in &members {
        if *work - last_poll >= 4096 {
            last_poll = *work;
            if stopped(*work) {
                return Err(());
            }
        }
        let nd = &nodes[ni];
        let x = *nd.bvars.iter().min_by_key(|&&b| lvl(b)).unwrap();
        let li = lvl(x);
        let mut sides: [(Child, u64); 2] = [(Child::True, 0); 2];
        for (s, value) in [(0usize, true), (1usize, false)] {
            let Some((lit, sat, id)) = residual(&nd.defs, x, value) else { return Ok(None) };
            let child = match (lit, sat) {
                (None, true) => Child::True,
                (None, false) => Child::False,
                (Some(l), _) => {
                    if let Some(&ci) = node_of.get(&l) {
                        if kind[l as usize] == 2 && nd.children.contains(&l) {
                            Child::Node(ci)
                        } else {
                            return Ok(None);
                        }
                    } else {
                        if lvl(l.abs()) <= li {
                            return Ok(None); // a board literal not below the level
                        }
                        Child::Lit(l)
                    }
                }
            };
            sides[s] = (child, id);
        }
        let nd = &mut nodes[ni];
        nd.x = x;
        nd.level = li;
        nd.hi = sides[0].0;
        nd.c1 = sides[0].1;
        nd.lo = sides[1].0;
        nd.c2 = sides[1].1;
        nd.parents = 0;
        *work += nd.defs.len() as u64 * 4;
    }
    for &ni in &members {
        let (li, hi, lo) = (nodes[ni].level, nodes[ni].hi, nodes[ni].lo);
        for child in [hi, lo] {
            if let Child::Node(c) = child {
                if nodes[c].level <= li {
                    return Ok(None);
                }
                nodes[c].parents += 1;
            }
        }
        *work += 1;
        if *work - last_poll >= 4096 {
            last_poll = *work;
            if stopped(*work) {
                return Err(());
            }
        }
    }
    if stopped(*work) {
        return Err(());
    }
    // val, bottom up
    if work.saturating_add(members.len() as u64) > budget {
        return Err(());
    }
    *work += members.len() as u64;
    let mut by_level: Vec<usize> = members.clone();
    by_level.sort_by_key(|&ni| Reverse(nodes[ni].level));
    for &ni in &by_level {
        *work += 1;
        if *work - last_poll >= 4096 {
            last_poll = *work;
            if stopped(*work) {
                return Err(());
            }
        }
        let v = |child: Child, nodes: &[Node]| -> u32 {
            match child {
                Child::True => 0,
                Child::False => INF,
                Child::Lit(l) => (l > 0) as u32,
                Child::Node(c) => nodes[c].val,
            }
        };
        let (vh, vl) = (v(nodes[ni].hi, nodes), v(nodes[ni].lo, nodes));
        let through_hi = if vh == INF { INF } else { vh + 1 };
        nodes[ni].val = through_hi.min(vl);
    }
    // the groups: cliques of the pair clauses among the board variables
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for i in 0..f.len() {
        *work += 1;
        if *work - last_poll >= 4096 {
            last_poll = *work;
            if stopped(*work) {
                return Err(());
            }
        }
        if f.taut[i] || f.dup[i] {
            continue;
        }
        let c = f.clause(i);
        if c.len() == 2 && c[0] < 0 && c[1] < 0 {
            if let (Some(&a), Some(&b)) = (bix.get(&-c[0]), bix.get(&-c[1])) {
                adj[a].push(b);
                adj[b].push(a);
            }
        }
    }
    // each list is sorted only when the limit covers it
    for a in adj.iter_mut() {
        let cost = a.len() as u64 + 1;
        if work.saturating_add(cost) > budget {
            return Err(());
        }
        *work += cost;
        if *work - last_poll >= 4096 {
            last_poll = *work;
            if stopped(*work) {
                return Err(());
            }
        }
        a.sort_unstable();
        a.dedup();
    }
    let mut group_of: Vec<usize> = vec![usize::MAX; nb];
    let mut groups: Vec<Vec<u32>> = Vec::new();
    let mut group_of_level: Vec<usize> = vec![0; nb];
    let intersect = |cand: &[usize], with: &[usize]| -> Vec<usize> { cand.iter().copied().filter(|m| with.binary_search(m).is_ok()).collect() };
    for &b in &order {
        let bi = bix[&b];
        if group_of[bi] != usize::MAX {
            continue;
        }
        let gi = groups.len();
        let mut clique: Vec<usize> = vec![bi];
        group_of[bi] = gi;
        let mut cand: Vec<usize> = adj[bi].iter().copied().filter(|&m| group_of[m] == usize::MAX).collect();
        *work += adj[bi].len() as u64 + 1;
        if !cand.is_empty() {
            // the first pick by common neighbours among the candidates (a
            // row or a column), the rest by intersection; every scan of
            // the candidates is charged and the budget polled
            let mut best = (0usize, cand[0]);
            for &m in &cand {
                let deg = intersect(&cand, &adj[m]).len();
                *work += cand.len() as u64 + 1;
                if deg > best.0 {
                    best = (deg, m);
                }
                if stopped(*work) {
                    return Err(());
                }
            }
            let m = best.1;
            clique.push(m);
            group_of[m] = gi;
            *work += cand.len() as u64;
            cand = intersect(&cand, &adj[m]).into_iter().filter(|&u| u != m).collect();
            while let Some(&m) = cand.first() {
                clique.push(m);
                group_of[m] = gi;
                *work += cand.len() as u64;
                cand = intersect(&cand[1..], &adj[m]);
                if stopped(*work) {
                    return Err(());
                }
            }
        }
        let mut levels: Vec<u32> = clique.iter().map(|&m| level[m]).collect();
        levels.sort_unstable();
        for &l in &levels {
            group_of_level[l as usize] = gi;
        }
        groups.push(levels);
        if stopped(*work) {
            return Err(());
        }
    }
    Ok(Some(Diagram { root: ri, members, order, groups, group_of_level }))
}

/// The groups with a level at or below `i`, each with the position of its
/// first such level: the open groups of level i, the ones that stay open
/// longest first (so the list is a prefix walk, and nothing is tabled per
/// level).
fn open_at(gl: &[Vec<u32>], by_last: &[usize], i: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for &gi in by_last {
        let levels = &gl[gi];
        if (*levels.last().unwrap() as usize) < i {
            break;
        }
        out.push((gi, levels.partition_point(|&l| (l as usize) < i)));
    }
    out
}

/// The VeriPB proof of the refutation; returns the empty clause's id, or
/// None when a step could not be written or the budget ran out.
fn prove(dd: &Diagram, nodes: &[Node], w: &mut Writer, next_var: &mut i32, budget: u64, work: &mut u64, stop: &AtomicBool) -> Option<u64> {
    let stopped = |work: u64| work > budget || stop.load(Ordering::Relaxed);
    let gl = &dd.groups;
    let g = gl.len();
    // the sort of the groups, the level table and the sort of the nodes are
    // reserved before they are made
    let setup = g as u64 + dd.order.len() as u64 + 2 * dd.members.len() as u64 + 3;
    if work.saturating_add(setup) > budget || stop.load(Ordering::Relaxed) {
        return None;
    }
    *work += setup;
    let mut by_last: Vec<usize> = (0..g).collect();
    by_last.sort_by_key(|&gi| Reverse(*gl[gi].last().unwrap()));
    // per group and position: the variable for "a true here or below", the
    // clause (~x v w), the clause (~w_next v w) and the clause (~x v ~w_next)
    let mut vv: Vec<Vec<i32>> = Vec::with_capacity(g);
    let mut h1: Vec<Vec<u64>> = Vec::with_capacity(g);
    let mut cc: Vec<Vec<u64>> = Vec::with_capacity(g);
    let mut h3: Vec<Vec<u64>> = Vec::with_capacity(g);
    let mut last_poll = *work;
    for levels in gl {
        let t = levels.len();
        let mut v = vec![0i32; t];
        let mut a = vec![0u64; t];
        let mut b = vec![0u64; t];
        v[t - 1] = dd.order[levels[t - 1] as usize];
        for k in (0..t - 1).rev() {
            let x = dd.order[levels[k] as usize];
            let q = v[k + 1];
            *next_var += 1;
            let fresh = *next_var;
            v[k] = fresh;
            w.red_const(&[-fresh, x, q], fresh, false).ok()?;
            a[k] = w.red_const(&[-x, fresh], fresh, true).ok()?;
            b[k] = w.red_const(&[-q, fresh], fresh, true).ok()?;
            *work += 9;
            if *work - last_poll >= 4096 {
                last_poll = *work;
                if stopped(*work) {
                    return None;
                }
            }
        }
        vv.push(v);
        h1.push(a);
        cc.push(b);
        h3.push(vec![0u64; t]);
    }
    for (gi, levels) in gl.iter().enumerate() {
        for k in 0..levels.len() - 1 {
            let x = dd.order[levels[k] as usize];
            h3[gi][k] = w.rup(&[-x, -vv[gi][k + 1]]).ok()?;
            *work += 2;
            if *work - last_poll >= 4096 {
                last_poll = *work;
                if stopped(*work) {
                    return None;
                }
            }
        }
    }
    if stopped(*work) {
        return None;
    }
    let mut mono_cache: HashMap<(usize, usize, usize), u64> = HashMap::new();
    let mut h3_cache: HashMap<(usize, usize, usize), u64> = HashMap::new();
    let mut lit_t: HashMap<i32, u64> = HashMap::new();
    let level_of: HashMap<i32, usize> = dd.order.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let mut tids: Vec<u64> = vec![0; nodes.len()];
    let mut pending: Vec<u32> = nodes.iter().map(|n| n.parents).collect();
    let mut by_level: Vec<usize> = dd.members.clone();
    by_level.sort_by_key(|&ni| Reverse(nodes[ni].level));
    for &ni in &by_level {
        let nd = &nodes[ni];
        let i = nd.level as usize;
        let qg = dd.group_of_level[i];
        if nd.val == INF {
            tids[ni] = w.rup(&[-nd.var]).ok()?;
            *work += 1;
        } else if nd.val > 0 {
            let a = nd.val as i128;
            let open_i = open_at(gl, &by_last, i);
            // the node's lines cost about three scans of its open groups:
            // written only when the limit covers them
            let cost = 3 * open_i.len() as u64 + 8;
            if work.saturating_add(cost) > budget {
                return None;
            }
            // a child's (val, constraint id, level): a board literal's
            // constraint is "~y + the open groups at its level >= 1"
            let mut child_info = |child: Child, w: &mut Writer| -> Option<(u32, Option<u64>, Option<usize>)> {
                Some(match child {
                    Child::True => (0, None, None),
                    Child::False => (INF, None, None),
                    Child::Lit(l) if l < 0 => (0, None, None),
                    Child::Lit(y) => {
                        let j = level_of[&y];
                        let id = match lit_t.get(&y) {
                            Some(&id) => id,
                            None => {
                                let mut terms: Vec<(i64, i32)> = vec![(1, -y)];
                                terms.extend(open_at(gl, &by_last, j).into_iter().map(|(gi, k)| (1i64, vv[gi][k])));
                                let id = w.rup_terms(&terms, 1).ok()?;
                                lit_t.insert(y, id);
                                id
                            }
                        };
                        (1, Some(id), Some(j))
                    }
                    // a child that forces nothing has no constraint
                    Child::Node(c) => (nodes[c].val, if nodes[c].val == INF || nodes[c].val == 0 { None } else { Some(tids[c]) }, Some(nodes[c].level as usize)),
                })
            };
            let (bval, blo, jlo) = child_info(nd.lo, w)?;
            let (cval, bhi, jhi) = child_info(nd.hi, w)?;
            // the tokens that cancel the open groups of level i, negated in
            // ~T, against a child's sum at level j (a group open at j has
            // its first level at or after j; a group that ends between
            // takes a literal axiom)
            let mut cancel = |j: Option<usize>, hi_side: bool, w: &mut Writer| -> Option<String> {
                let mut s = String::new();
                for &(gi, k) in &open_i {
                    let there = j.and_then(|j| {
                        let k2 = gl[gi].partition_point(|&l| (l as usize) < j);
                        (k2 < gl[gi].len()).then_some(k2)
                    });
                    if hi_side && gi == qg {
                        if h1[gi][k] != 0 {
                            s.push_str(&format!(" {} +", h1[gi][k]));
                        }
                        if let Some(k2) = there {
                            let id = if k2 == k + 1 {
                                h3[gi][k]
                            } else {
                                match h3_cache.get(&(gi, k, k2)) {
                                    Some(&id) => id,
                                    None => {
                                        let id = w.rup(&[-dd.order[gl[gi][k] as usize], -vv[gi][k2]]).ok()?;
                                        h3_cache.insert((gi, k, k2), id);
                                        id
                                    }
                                }
                            };
                            s.push_str(&format!(" {} +", id));
                        }
                        continue;
                    }
                    match there {
                        None => s.push_str(&format!(" x{} +", vv[gi][k])),
                        Some(k2) if k2 == k => {}
                        Some(k2) => {
                            let id = if k2 == k + 1 {
                                cc[gi][k]
                            } else {
                                match mono_cache.get(&(gi, k, k2)) {
                                    Some(&id) => id,
                                    None => {
                                        let id = w.rup(&[vv[gi][k], -vv[gi][k2]]).ok()?;
                                        mono_cache.insert((gi, k, k2), id);
                                        id
                                    }
                                }
                            };
                            s.push_str(&format!(" {} +", id));
                        }
                    }
                }
                Some(s)
            };
            // the helper clauses are written before the negation's id is taken
            let lo_line = if bval != INF && bval >= 1 { Some(format!("{}{}", blo.unwrap(), cancel(jlo, false, w)?)) } else { None };
            let hi_line = if cval != INF { Some(format!("{}{}", bhi.map(|id| format!("{} +", id)).unwrap_or_default(), cancel(jhi, true, w)?)) } else { None };
            let neg = w.next_id();
            let mut lines: Vec<String> = Vec::with_capacity(3);
            if let Some(l) = lo_line {
                lines.push(format!("pol {} {} + ;", neg, l.trim_start()));
            }
            if let Some(l) = hi_line {
                let l = l.trim();
                lines.push(if l.is_empty() { format!("pol {} ;", neg) } else { format!("pol {} {} ;", neg, l) });
            }
            lines.push("rup >= 1 ;".to_string());
            let mut terms: Vec<(i128, i32)> = vec![(a, -nd.var)];
            terms.extend(open_i.iter().map(|&(gi, k)| (1i128, vv[gi][k])));
            *work += cost;
            tids[ni] = w.pbc_raw(&terms, a, &lines).ok()?;
        }
        // a child's constraint goes once its parents have used it
        let mut done: Vec<u64> = Vec::new();
        for child in [nd.hi, nd.lo] {
            if let Child::Node(c) = child {
                pending[c] -= 1;
                if pending[c] == 0 && tids[c] != 0 && c != dd.root {
                    done.push(tids[c]);
                }
            }
        }
        w.delete(&done).ok()?;
        if stopped(*work) {
            return None;
        }
    }
    let root = &nodes[dd.root];
    if root.val != INF {
        // the root's sum has one term per group and asks for more: ~root
        let mut expr = tids[dd.root].to_string();
        for (gi, k) in open_at(gl, &by_last, 0) {
            expr.push_str(&format!(" ~x{} +", vv[gi][k]));
        }
        expr.push_str(&format!(" {} d", root.val));
        w.pol(&expr).ok()?;
        *work += g as u64;
    }
    // with the input unit that asserts the root, the empty clause
    w.rup(&[]).ok()
}
