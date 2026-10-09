//! Not in kissat. The counting pass of the structure pass (plan section 11,
//! 2026-10-07; bead SAT-playground-1v2.4): pigeonhole-shaped structure
//! refuted by counting.
//!
//! The shape, in plain terms. Some clauses demand at least one of their
//! literals ("every pigeon sits somewhere"); some groups of literals allow
//! at most one of them ("a hole takes one pigeon"), written as the binary
//! clauses that forbid every pair. When every literal of a demand clause
//! belongs to one such group, each demand clause needs a group of its own,
//! and if some set of demand clauses reaches fewer groups than it has
//! members, no assignment satisfies them all: Hall's condition fails. The
//! pass finds such a set by matching (the demand clauses an alternating
//! search reaches from one that cannot be matched) and proves the
//! contradiction by adding things up in VeriPB: the demand clauses as
//! written, and for every group the at-most-one constraint derived from
//! its pair clauses by cutting planes. In the sum every literal that is in
//! a demand clause and in its group cancels against its negation, and
//! what is left cannot reach the right-hand side.
//!
//! Proof: `pol` steps only, over the input clause ids (clause i is
//! constraint i), ending in `conclusion UNSAT`.

use crate::structure::{Derived, Formula};
use std::collections::HashMap;

const MAX_GROUP: usize = 64;

#[inline]
fn idx(l: i32) -> usize {
    (l.unsigned_abs() as usize) * 2 + (l < 0) as usize
}

/// An at-most-one group: its literals and, for every pair, the id of the
/// clause (~a v ~b) (1-based, the proof's constraint id).
struct Group {
    lits: Vec<i32>,
    pair_id: HashMap<(i32, i32), u64>,
}

fn pair_key(a: i32, b: i32) -> (i32, i32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

pub fn run(f: &Formula, derived: &mut Derived, budget: u64, ticks: &mut u64, stop: &std::sync::atomic::AtomicBool) {
    let mut work = *ticks;
    let nl = 2 * (f.vars as usize + 1);
    let stopped = |work: u64| work > budget || stop.load(std::sync::atomic::Ordering::Relaxed);
    // the pair clauses (~a v ~b) as an adjacency over literals a, b (the
    // ones the clause forbids together), and the demand clauses
    let mut adj: Vec<Vec<i32>> = vec![Vec::new(); nl];
    let mut pair_id: HashMap<(i32, i32), u64> = HashMap::new();
    let mut demands: Vec<usize> = Vec::new();
    for i in 0..f.len() {
        if i % 4096 == 0 {
            *ticks = work;
            if stopped(work) {
                return;
            }
        }
        if f.taut[i] || f.dup[i] {
            continue; // a clause with a repeated literal has another coefficient in the proof
        }
        let c = f.clause(i);
        work += c.len() as u64 + 1;
        if c.len() == 2 {
            let (a, b) = (-c[0], -c[1]);
            adj[idx(a)].push(b);
            adj[idx(b)].push(a);
            pair_id.entry(pair_key(a, b)).or_insert(i as u64 + 1);
        }
        if c.len() >= 2 {
            demands.push(i);
        }
    }
    if demands.is_empty() || pair_id.is_empty() {
        *ticks = work;
        return;
    }
    for li in 0..nl {
        if !adj[li].is_empty() {
            adj[li].sort_unstable();
            adj[li].dedup();
            work += adj[li].len() as u64;
        }
        if li % 4096 == 0 {
            *ticks = work;
            if stopped(work) {
                return;
            }
        }
    }
    // groups: for every literal of a demand clause, the largest clique of
    // the pair graph through it that holds no other literal of that clause
    // (a hole for a pigeon, the other square of a domino), found greedily;
    // groups are shared by literal set
    let mut group_index: HashMap<Vec<i32>, u32> = HashMap::new();
    let mut groups: Vec<Group> = Vec::new();
    // for each demand clause, the group of each literal (u32::MAX: none)
    let mut lit_group: HashMap<(usize, i32), u32> = HashMap::new();
    let mut own_mark: Vec<bool> = vec![false; nl];
    for &i in &demands {
        let own: Vec<i32> = f.clause(i).to_vec();
        for &l in &own {
            own_mark[idx(l)] = true;
        }
        for &l in &own {
            if adj[idx(l)].is_empty() {
                continue;
            }
            let mut clique: Vec<i32> = vec![l];
            let mut cand: Vec<i32> = adj[idx(l)].iter().copied().filter(|&m| m != -l && !own_mark[idx(m)]).collect();
            cand.sort_unstable();
            cand.dedup();
            work += adj[idx(l)].len() as u64 + 1;
            while !cand.is_empty() && clique.len() < MAX_GROUP {
                let mut best: Option<(usize, i32)> = None;
                for &m in &cand {
                    let deg = adj[idx(m)].iter().filter(|&&x| cand.binary_search(&x).is_ok()).count();
                    work += adj[idx(m)].len() as u64;
                    if best.map_or(true, |(d, _)| deg > d) {
                        best = Some((deg, m));
                    }
                }
                let (_, m) = best.unwrap();
                clique.push(m);
                let neighbours: &Vec<i32> = &adj[idx(m)];
                cand.retain(|&x| x != m && neighbours.binary_search(&x).is_ok());
                if stopped(work) {
                    *ticks = work;
                    return;
                }
            }
            if clique.len() < 2 {
                continue;
            }
            let mut key = clique.clone();
            key.sort_unstable();
            let gi = match group_index.get(&key) {
                Some(&gi) => gi,
                None => {
                    let mut ids: HashMap<(i32, i32), u64> = HashMap::new();
                    for a in 0..clique.len() {
                        for b in a + 1..clique.len() {
                            ids.insert(pair_key(clique[a], clique[b]), pair_id[&pair_key(clique[a], clique[b])]);
                        }
                    }
                    let gi = groups.len() as u32;
                    group_index.insert(key, gi);
                    groups.push(Group { lits: clique, pair_id: ids });
                    gi
                }
            };
            lit_group.insert((i, l), gi);
        }
        for &l in &own {
            own_mark[idx(l)] = false;
        }
        *ticks = work;
        if stopped(work) {
            return;
        }
    }
    // the demand clauses every literal of which has a group
    let usable: Vec<usize> = demands.iter().copied().filter(|&i| f.clause(i).iter().all(|&l| lit_group.contains_key(&(i, l)))).collect();
    if std::env::var_os("SAT_STRUCT_DEBUG").is_some() {
        eprintln!("structcount: {} demand clauses, {} pair clauses, {} groups (sizes up to {}), {} demand clauses fully grouped",
            demands.len(), pair_id.len(), groups.len(), groups.iter().map(|g| g.lits.len()).max().unwrap_or(0), usable.len());
    }
    if usable.is_empty() || groups.is_empty() {
        *ticks = work;
        return;
    }
    // two demand clauses sharing a literal may not both be in the set (the
    // literal would count twice against one cancellation): two-colour the
    // demand clauses by shared literals and work one colour at a time
    let mut holders: Vec<Vec<usize>> = vec![Vec::new(); nl];
    for &i in &usable {
        for &l in f.clause(i) {
            holders[idx(l)].push(i);
        }
    }
    let mut colour: HashMap<usize, u8> = HashMap::new();
    let mut bipartite = true;
    for &i in &usable {
        if colour.contains_key(&i) {
            continue;
        }
        colour.insert(i, 0);
        let mut queue = std::collections::VecDeque::from(vec![i]);
        while let Some(d) = queue.pop_front() {
            let c = colour[&d];
            for &l in f.clause(d) {
                for &e in &holders[idx(l)] {
                    if e == d {
                        continue;
                    }
                    match colour.get(&e) {
                        None => {
                            colour.insert(e, c ^ 1);
                            queue.push_back(e);
                        }
                        Some(&ce) if ce == c => bipartite = false,
                        _ => {}
                    }
                    work += 1;
                    if work & 4095 == 0 {
                        *ticks = work;
                        if stopped(work) {
                            return;
                        }
                    }
                    if !bipartite {
                        break;
                    }
                }
                if !bipartite {
                    break;
                }
            }
            if !bipartite || stopped(work) {
                break;
            }
        }
        if !bipartite || stopped(work) {
            break;
        }
    }
    *ticks = work;
    if !bipartite || stopped(work) {
        return; // a partial colouring is not used
    }
    let neighbours = |i: usize| -> Vec<usize> {
        let mut v: Vec<usize> = f.clause(i).iter().map(|&l| lit_group[&(i, l)] as usize).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let mut violating: Option<Vec<usize>> = None;
    for class in 0..2u8 {
        let members: Vec<usize> = usable.iter().copied().filter(|d| colour[d] == class).collect();
    // maximum matching by augmenting paths; when a demand clause cannot be
    // matched, the demand clauses reached by the alternating search from
    // it reach fewer groups than their number
    let mut match_group: Vec<Option<usize>> = vec![None; groups.len()];
    let mut seen_group: Vec<bool> = vec![false; groups.len()];
    let mut seen_list: Vec<usize> = Vec::new();
    for &d in &members {
        for &g in &seen_list {
            seen_group[g] = false; // only what the last search touched is reset
        }
        seen_list.clear();
        let mut reached: Vec<usize> = vec![d];
        let mut found = false;
        // iterative alternating search with an explicit stack of (demand, next neighbour index)
        let mut stack: Vec<(usize, Vec<usize>, usize)> = vec![(d, neighbours(d), 0)];
        let mut path: Vec<(usize, usize)> = Vec::new(); // (demand, group) choices
        while let Some(top) = stack.last_mut() {
            if top.2 >= top.1.len() {
                stack.pop();
                path.pop();
                continue;
            }
            let g = top.1[top.2];
            top.2 += 1;
            if seen_group[g] {
                continue;
            }
            seen_group[g] = true;
            seen_list.push(g);
            work += 1;
            if work & 1023 == 0 {
                *ticks = work; // visible to a signal during the search
                if stopped(work) {
                    return;
                }
            }
            path.push((top.0, g));
            match match_group[g] {
                None => {
                    found = true;
                    break;
                }
                Some(d2) => {
                    reached.push(d2);
                    stack.push((d2, neighbours(d2), 0));
                }
            }
        }
        *ticks = work;
        if stopped(work) {
            return;
        }
        if found {
            for (dd, g) in path {
                match_group[g] = Some(dd);
            }
        } else {
            violating = Some(reached);
            break;
        }
    }
    if violating.is_some() {
        break;
    }
    }
    *ticks = work;
    if std::env::var_os("SAT_STRUCT_DEBUG").is_some() {
        eprintln!("structcount: violating set {:?}", violating.as_ref().map(|v| v.len()));
    }
    let Some(set) = violating else { return };
    // the groups the set reaches, and the sum's bookkeeping: the proof is
    // only written when the sum is contradictory
    let mut touched: Vec<usize> = set.iter().flat_map(|&d| neighbours(d)).collect();
    let _ = &colour;
    touched.sort_unstable();
    touched.dedup();
    let mut coeff: HashMap<i32, i64> = HashMap::new(); // literal -> coefficient in the sum
    let mut rhs: i64 = 0;
    for &d in &set {
        for &l in f.clause(d) {
            *coeff.entry(l).or_default() += 1;
        }
        rhs += 1;
    }
    for &g in &touched {
        for &l in &groups[g].lits {
            *coeff.entry(-l).or_default() += 1;
        }
        rhs += groups[g].lits.len() as i64 - 1;
    }
    // l and ~l together give a constant 1 on the left
    let mut slack: i64 = 0;
    let lits: Vec<i32> = coeff.keys().copied().collect();
    for &l in &lits {
        if l > 0 {
            let (a, b) = (coeff.get(&l).copied().unwrap_or(0), coeff.get(&-l).copied().unwrap_or(0));
            let common = a.min(b);
            slack += common;
            if let Some(x) = coeff.get_mut(&l) {
                *x -= common;
            }
            if let Some(x) = coeff.get_mut(&-l) {
                *x -= common;
            }
        }
    }
    let max_lhs: i64 = coeff.values().sum();
    if max_lhs >= rhs - slack {
        return; // no contradiction in this sum
    }
    // the proof: per group the at-most-one constraint from its pairs
    // (sum the pairs of every (k-1)-subset, divide by k-2, and so on up),
    // then the sum of the demand clauses and the group constraints
    let Some(w) = derived.proof.as_mut() else {
        // without a proof file the answer would be unchecked: refuse
        derived.notes.push("counting: a Hall violation found, but no proof file to write it to".to_string());
        return;
    };
    let mut group_ids: Vec<u64> = Vec::new();
    for &g in &touched {
        let lits = &groups[g].lits;
        let k = lits.len();
        // C_2 is the pair clause; C_{m+1} = (pair(1,m+1) + ... + pair(m,m+1)
        // + (m-1) C_m) / m, which is sum(~l) >= m over the first m+1 literals
        let mut current: u64 = groups[g].pair_id[&pair_key(lits[0], lits[1])];
        for m in 2..k {
            let mut expr = String::new();
            for i in 0..m {
                let id = groups[g].pair_id[&pair_key(lits[i], lits[m])];
                if expr.is_empty() {
                    expr = id.to_string();
                } else {
                    expr.push_str(&format!(" {} +", id));
                }
            }
            if m > 1 {
                expr.push_str(&format!(" {} {} * +", current, m - 1));
            }
            expr.push_str(&format!(" {} d", m));
            current = match w.pol(&expr) {
                Ok(id) => id,
                Err(_) => return,
            };
            work += m as u64;
        }
        group_ids.push(current);
        if stopped(work) {
            *ticks = work;
            return;
        }
    }
    let mut expr = String::new();
    for &d in &set {
        let id = d as u64 + 1;
        if expr.is_empty() {
            expr = id.to_string();
        } else {
            expr.push_str(&format!(" {} +", id));
        }
    }
    for &gid in &group_ids {
        expr.push_str(&format!(" {} +", gid));
    }
    if w.pol(&expr).is_err() {
        return;
    }
    // the sum is contradictory: the empty clause follows by propagation of nothing
    let id = w.rup(&[]).unwrap_or(0);
    derived.add(&[], id);
    *ticks = work;
    derived.notes.push(format!(
        "counting: {} demand clauses, {} at-most-one groups, a set of {} demand clauses reaching {} groups, refuted",
        usable.len(),
        groups.len(),
        set.len(),
        touched.len()
    ));
}
