//! Bead SAT-playground-skz.13: the cells of a technology-mapped Booth
//! tree, found by function.
//!
//! A mapper folds a Booth row bit (the select part xor the digit's sign)
//! into the adder that reads it, and builds a sum from its carry, so over
//! a small cut the gates are no adder's. Over the formula's inputs they
//! still are: each row bit is a known function of five inputs, and a
//! carry gate is the majority of three row signals of one column (row
//! bits, the digit's sign at the row's lowest column, outputs of cells
//! found before) on every input. The digits and the order of the other
//! operand are read off the truth tables of the gates that compute row
//! bits or their select parts. A row signal is such a gate, or a select
//! gate xor the digit's sign (a fresh variable, as in the fused cells).
//! The cells are matched on random simulation, gate by gate in circuit
//! order, and each match is then checked exactly over the smallest set of
//! signals that determines its outputs and its leaves by propagation (a
//! minimum vertex cut), which is the set the proof's case split runs
//! over. A sum or a carry the netlist does not compute (a dead product bit
//! at the bottom, a sum the mapper folded into the next level, a dead
//! carry at the top) is a fresh variable defined over the cell's leaves.

use super::*;

const WORDS: usize = 2; // 128 random input patterns
type Sig = [u64; WORDS];
/// The largest set of signals a cell's case split runs over (2^11 - 1
/// `rup` lines per inequality).
const MAX_SPLIT: usize = 10;
/// The gates walked for one cell's cut.
const MAX_REGION: usize = 4096;
/// The largest number of inputs under one side (supports are bit sets).
const MAX_INPUTS: usize = 128;
/// The fewest gates computing a row bit or its select part for a side to
/// be read as a Booth tree.
const MIN_ROW_GATES: usize = 16;

fn sig_not(a: Sig) -> Sig {
    [!a[0], !a[1]]
}
fn sig_and(a: Sig, b: Sig) -> Sig {
    [a[0] & b[0], a[1] & b[1]]
}
fn sig_or(a: Sig, b: Sig) -> Sig {
    [a[0] | b[0], a[1] | b[1]]
}
fn sig_xor(a: Sig, b: Sig) -> Sig {
    [a[0] ^ b[0], a[1] ^ b[1]]
}
fn sig_maj(a: Sig, b: Sig, c: Sig) -> Sig {
    sig_or(sig_or(sig_and(a, b), sig_and(a, c)), sig_and(b, c))
}
fn sig_lit(a: Sig, neg: bool) -> Sig {
    if neg {
        sig_not(a)
    } else {
        a
    }
}

/// Random simulation, `WORDS` words a variable (representatives and
/// inputs; a merged variable reads its representative's word).
fn simulate_words(net: &Netlist, work: &mut u64, stop: &dyn Fn(u64) -> bool) -> Option<Vec<Sig>> {
    let nv = net.rep.len();
    if stop(*work + (WORDS * nv) as u64) {
        return None;
    }
    let mut val = vec![[0u64; WORDS]; nv];
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for v in 1..nv {
        if net.gates[v].is_none() && net.rep[v].0 == v as i32 {
            for k in 0..WORDS {
                val[v][k] = next();
            }
        }
    }
    *work += (WORDS * nv) as u64;
    let mut ins: Vec<u64> = Vec::new();
    for &v in &net.order {
        if stop(*work) {
            return None;
        }
        if net.rep[v as usize].0 != v {
            continue;
        }
        let g = net.gates[v as usize].as_ref().unwrap();
        for k in 0..WORDS {
            ins.clear();
            for &u in &g.sup {
                let (r, ng) = net.rep[u as usize];
                let w = val[r as usize][k];
                ins.push(if ng { !w } else { w });
            }
            val[v as usize][k] = eval_gate(g.tt, &ins, u64::MAX);
        }
        *work += (WORDS * g.sup.len()) as u64;
    }
    Some(val)
}

/// The bit of the product of the operands `a` and `b` (bit lists, lowest
/// first) that `out` is on the stage's patterns, either polarity; None when
/// it is none of them or the operands are too wide.
fn product_column(net: &Netlist, val: &[Sig], a: &[i32], b: &[i32], out: i32, work: &mut u64) -> Option<usize> {
    if a.is_empty() || b.is_empty() || a.len() + b.len() > 126 {
        return None;
    }
    let word = |v: i32| -> Sig {
        let (r, ng) = net.rep[v as usize];
        sig_lit(val[r as usize], ng)
    };
    let (wa, wb): (Vec<Sig>, Vec<Sig>) = (a.iter().map(|&x| word(x)).collect(), b.iter().map(|&x| word(x)).collect());
    let wo = word(out);
    let mut product: Vec<u128> = Vec::with_capacity(64 * WORDS);
    for w in 0..WORDS {
        for t in 0..64 {
            let x: u128 = wa.iter().enumerate().map(|(i, s)| (((s[w] >> t) & 1) as u128) << i).sum();
            let y: u128 = wb.iter().enumerate().map(|(k, s)| (((s[w] >> t) & 1) as u128) << k).sum();
            product.push(x.wrapping_mul(y));
        }
    }
    *work += (64 * WORDS * (a.len() + b.len())) as u64;
    (0..a.len() + b.len()).find(|&c| {
        let mut sig: Sig = [0; WORDS];
        for (n, p) in product.iter().enumerate() {
            sig[n / 64] |= (((p >> c) & 1) as u64) << (n % 64);
        }
        sig == wo || sig == sig_not(wo)
    })
}

/// The radix-4 Booth roles of a function's inputs: the digit's two low
/// bits (unordered: one = lo xor mid and two = hi ~mid ~lo + ~hi mid lo
/// are symmetric in them; the chain of digits orders them), its sign `hi`,
/// the operand bit selected for one (`dhi`) and for two (`dlo`); absent
/// roles are zero. The function is the row bit (select xor hi) or the
/// select part (one dhi + two dlo), either polarity.
#[derive(Clone, Copy, Debug)]
struct Roles {
    pair: [Option<usize>; 2],
    hi: Option<usize>,
    dhi: Option<usize>,
    dlo: Option<usize>,
}

/// The numbers of ones a row bit's table, or its select part's, can have
/// over `n` inputs (either polarity): a cheap test before the roles.
fn row_popcounts(n: usize) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    let width = 1usize << n;
    let mut assign = [0u8; 6];
    for code in 0..4usize.pow(n as u32) {
        let mut c = code;
        let mut counts = [0u8; 4];
        for a in assign.iter_mut().take(n) {
            *a = (c % 4) as u8;
            c /= 4;
            counts[*a as usize] += 1;
        }
        if counts[0] == 0 || counts[0] > 2 || counts[1] > 1 || counts[2] > 1 || counts[3] > 1 || counts[2] + counts[3] == 0 {
            continue;
        }
        let (mut sel_ones, mut pp_ones) = (0u32, 0u32);
        for m in 0..width {
            let mut p = [false; 2];
            let mut np = 0;
            let (mut h, mut d1, mut d0) = (false, false, false);
            for (i, &a) in assign.iter().enumerate().take(n) {
                let b = (m >> i) & 1 == 1;
                match a {
                    0 => {
                        p[np] = b;
                        np += 1;
                    }
                    1 => h = b,
                    2 => d1 = b,
                    _ => d0 = b,
                }
            }
            let one = p[0] ^ p[1];
            let two = (h && !p[0] && !p[1]) || (!h && p[0] && p[1]);
            let sel = (one && d1) || (two && d0);
            sel_ones += sel as u32;
            pp_ones += (sel ^ h) as u32;
        }
        for k in [sel_ones, width as u32 - sel_ones, pp_ones, width as u32 - pp_ones] {
            if !out.contains(&k) {
                out.push(k);
            }
        }
    }
    out
}

fn booth_roles(tt: u64, n: usize, work: &mut u64) -> Option<Roles> {
    let width = 1usize << n;
    let mask: u64 = if width >= 64 { u64::MAX } else { (1u64 << width) - 1 };
    let mut assign = [0u8; 6];
    // role codes: 0 one of the pair, 1 the sign, 2 data for one, 3 data for two
    for code in 0..4usize.pow(n as u32) {
        let mut c = code;
        let mut counts = [0u8; 4];
        for a in assign.iter_mut().take(n) {
            *a = (c % 4) as u8;
            c /= 4;
            counts[*a as usize] += 1;
        }
        if counts[0] == 0 || counts[0] > 2 || counts[1] > 1 || counts[2] > 1 || counts[3] > 1 || counts[2] + counts[3] == 0 {
            continue;
        }
        *work += width as u64;
        let mut sel_tt = 0u64;
        let mut pp_tt = 0u64;
        for m in 0..width {
            let mut p = [false; 2];
            let mut np = 0;
            let (mut h, mut d1, mut d0) = (false, false, false);
            for (i, &a) in assign.iter().enumerate().take(n) {
                let b = (m >> i) & 1 == 1;
                match a {
                    0 => {
                        p[np] = b;
                        np += 1;
                    }
                    1 => h = b,
                    2 => d1 = b,
                    _ => d0 = b,
                }
            }
            let one = p[0] ^ p[1];
            let two = (h && !p[0] && !p[1]) || (!h && p[0] && p[1]);
            let sel = (one && d1) || (two && d0);
            if sel {
                sel_tt |= 1 << m;
            }
            if sel ^ h {
                pp_tt |= 1 << m;
            }
        }
        for (t, row_bit) in [(sel_tt, false), (pp_tt, true)] {
            if row_bit && counts[1] == 0 {
                continue; // (without a sign the row bit is the select part)
            }
            if t == tt || (!t & mask) == tt {
                let mut roles = Roles { pair: [None, None], hi: None, dhi: None, dlo: None };
                let mut np = 0;
                for (i, &a) in assign.iter().enumerate().take(n) {
                    match a {
                        0 => {
                            roles.pair[np] = Some(i);
                            np += 1;
                        }
                        1 => roles.hi = Some(i),
                        2 => roles.dhi = Some(i),
                        _ => roles.dlo = Some(i),
                    }
                }
                return Some(roles);
            }
        }
    }
    None
}

/// A row signal or an output of a cell found so far, at one column: a
/// variable (of the netlist, or a fresh one), or a fused row bit whose
/// fresh variable is made when a cell first reads it.
#[derive(Clone)]
struct Item {
    var: i32,
    make: Option<FreshDef>,
    word: Sig,
    supp: u128,
    alive: bool,
    /// a cell's output (a row signal is not), and whether it is the second
    /// read of one, a column above where it is made
    produced: bool,
    second: bool,
    col: usize,
    /// a cell's carry (only a carry is read a second time)
    is_carry: bool,
    /// a row bit's gate that computes its complement
    complement: bool,
}

/// A match for one gate: the leaves (items, read negated or not), whether
/// a constant one is folded in, the sum (a gate, read negated or not, or
/// a fresh variable) and the carry (a gate read plain, or fresh).
struct Cand {
    col: usize,
    leaves: Vec<(usize, bool)>,
    one: bool,
    sum: Option<(i32, bool)>,
    carry: Option<i32>,
}

/// What the stage found on one side.
pub(super) struct BoothCells {
    pub cells: Vec<Cell>,
    /// the fresh variables the cells read or write, in definition order
    pub fresh: Vec<Fresh>,
    /// the row-bit and select gates the digits were read from
    pub row_gates: usize,
    /// the carry into the compared column when the side's top is part of
    /// the carry network: a fresh variable, the compared signal xor what
    /// the cells leave over at that column, read by the top cell
    pub virtual_net: Option<i32>,
    /// per cell, its column (its sum's: the carry sits one above)
    pub cols: Vec<i64>,
    /// the row bits no kept cell reads, as the literal that is the row
    /// bit, with their columns: what the carry network sums of the rows
    pub rows_left: Vec<(i32, i64)>,
}

/// The gates and inputs under a cell's outputs and leaves.
struct Region {
    nodes: Vec<i32>,
    index: HashMap<i32, usize>,
}

fn region(net: &Netlist, roots: &[i32], work: &mut u64) -> Option<Region> {
    let mut index: HashMap<i32, usize> = HashMap::new();
    let mut nodes: Vec<i32> = Vec::new();
    let mut stack: Vec<i32> = roots.iter().map(|&v| net.rep[v as usize].0).collect();
    while let Some(u) = stack.pop() {
        if index.contains_key(&u) {
            continue;
        }
        index.insert(u, nodes.len());
        nodes.push(u);
        *work += 1;
        if nodes.len() > MAX_REGION {
            return None;
        }
        if let Some(g) = net.gates[u as usize].as_ref() {
            stack.extend(g.sup.iter().map(|&x| net.rep[x as usize].0));
        }
    }
    Some(Region { nodes, index })
}

/// The smallest set of signals of the region that determines every target
/// and every basis signal by forward propagation through the gates (a
/// minimum vertex cut between the inputs and them; targets and forbidden
/// signals never in it), or None past `MAX_SPLIT` signals.
fn min_cut(net: &Netlist, reg: &Region, targets: &HashSet<i32>, basis: &HashSet<i32>, forbidden: &HashSet<i32>, work: &mut u64) -> Option<Vec<i32>> {
    let nodes = &reg.nodes;
    // node i is 2i (in) and 2i + 1 (out), the source and the sink last; an
    // edge's capacity 1 or "infinite"
    let n = 2 * nodes.len() + 2;
    let (src, snk) = (n - 2, n - 1);
    const INF: i32 = 1 << 20;
    let mut to: Vec<usize> = Vec::new();
    let mut cap: Vec<i32> = Vec::new();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let add = |a: usize, b: usize, c: i32, to: &mut Vec<usize>, cap: &mut Vec<i32>, adj: &mut Vec<Vec<usize>>| {
        adj[a].push(to.len());
        to.push(b);
        cap.push(c);
        adj[b].push(to.len());
        to.push(a);
        cap.push(0);
    };
    for (i, &u) in nodes.iter().enumerate() {
        add(2 * i, 2 * i + 1, if targets.contains(&u) || forbidden.contains(&u) { INF } else { 1 }, &mut to, &mut cap, &mut adj);
        match net.gates[u as usize].as_ref() {
            Some(g) => {
                let mut ins: Vec<usize> = g.sup.iter().map(|&x| reg.index[&net.rep[x as usize].0]).collect();
                ins.sort_unstable();
                ins.dedup();
                for j in ins {
                    add(2 * j + 1, 2 * i, INF, &mut to, &mut cap, &mut adj);
                }
            }
            None => add(src, 2 * i, INF, &mut to, &mut cap, &mut adj),
        }
        if targets.contains(&u) || basis.contains(&u) {
            add(2 * i + 1, snk, INF, &mut to, &mut cap, &mut adj);
        }
    }
    *work += to.len() as u64;
    // augmenting paths, breadth first
    let mut flow = 0usize;
    let mut prev: Vec<usize> = vec![usize::MAX; n];
    loop {
        for p in prev.iter_mut() {
            *p = usize::MAX;
        }
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        queue.push_back(src);
        prev[src] = usize::MAX - 1;
        while let Some(a) = queue.pop_front() {
            if a == snk {
                break;
            }
            for &e in &adj[a] {
                let b = to[e];
                if cap[e] > 0 && prev[b] == usize::MAX {
                    prev[b] = e;
                    queue.push_back(b);
                }
            }
            *work += adj[a].len() as u64;
        }
        if prev[snk] == usize::MAX {
            break;
        }
        let mut b = snk;
        while b != src {
            let e = prev[b];
            cap[e] -= 1;
            cap[e ^ 1] += 1;
            b = to[e ^ 1];
        }
        flow += 1;
        if flow > MAX_SPLIT {
            return None;
        }
    }
    // the cut: the nodes whose in side the source still reaches and whose
    // out side it does not
    let mut seen = vec![false; n];
    let mut queue = vec![src];
    seen[src] = true;
    while let Some(a) = queue.pop() {
        for &e in &adj[a] {
            let b = to[e];
            if cap[e] > 0 && !seen[b] {
                seen[b] = true;
                queue.push(b);
            }
        }
    }
    let mut cut: Vec<i32> = (0..nodes.len()).filter(|&i| seen[2 * i] && !seen[2 * i + 1]).map(|i| nodes[i]).collect();
    cut.sort_unstable();
    if cut.len() != flow {
        return None;
    }
    Some(cut)
}

/// A variable's table read as a literal.
fn table_of(vals: &HashMap<i32, Vec<u64>>, net: &Netlist, lit: i32) -> Option<Vec<u64>> {
    let v = lit.abs();
    let (r, ng) = if (v as usize) < net.rep.len() { net.rep[v as usize] } else { (v, false) };
    let t = vals.get(&r)?;
    Some(if ng != (lit < 0) { t.iter().map(|w| !w).collect() } else { t.clone() })
}

/// The tables of the variables in `want` (gates and fresh variables) from
/// those given in `init`, through the gates and the fresh definitions;
/// None when one depends on an input `init` does not give.
fn eval_tables(net: &Netlist, init: HashMap<i32, Vec<u64>>, want: &[i32], defs: &HashMap<i32, FreshDef>, work: &mut u64) -> Option<HashMap<i32, Vec<u64>>> {
    let nw = init.values().next().map_or(1, |t| t.len());
    let mut vals = init;
    let nv = net.rep.len();
    let repr = |v: i32| -> i32 { if (v as usize) < nv { net.rep[v as usize].0 } else { v } };
    let mut stack: Vec<i32> = want.iter().map(|&l| repr(l.abs())).collect();
    let mut steps = 0usize;
    while let Some(&u) = stack.last() {
        steps += 1;
        if steps > 16 * MAX_REGION {
            return None;
        }
        if vals.contains_key(&u) {
            stack.pop();
            continue;
        }
        // its inputs, as literals, and its table (a gate's) or its kind (a
        // fresh variable's: an xor may have more inputs than a table holds)
        let (ins, tt, kind): (Vec<i32>, u64, u8) = if (u as usize) < nv {
            let g = net.gates[u as usize].as_ref()?; // (an input `init` does not give)
            (g.sup.clone(), g.tt, 3)
        } else {
            match defs.get(&u)? {
                FreshDef::Xor(l) => (l.clone(), 0, 0u8),
                FreshDef::And(l) => (l.clone(), 0, 1u8),
                FreshDef::Maj(l) => (l.clone(), 0, 2u8),
            }
        };
        let mut pending = false;
        for &l in &ins {
            let r = repr(l.abs());
            if !vals.contains_key(&r) {
                stack.push(r);
                pending = true;
            }
        }
        if pending {
            if stack.len() > 16 * MAX_REGION {
                return None;
            }
            continue;
        }
        let tabs: Vec<Vec<u64>> = ins.iter().map(|&l| table_of(&vals, net, l).unwrap()).collect();
        let mut out: Vec<u64> = vec![0; nw];
        let mut words: Vec<u64> = vec![0; ins.len()];
        for (w, o) in out.iter_mut().enumerate() {
            for (k, t) in tabs.iter().enumerate() {
                words[k] = t[w];
            }
            *o = match kind {
                3 => eval_gate(tt, &words, u64::MAX),
                0 => words.iter().fold(0u64, |a, &x| a ^ x),
                1 => words.iter().fold(u64::MAX, |a, &x| a & x),
                _ => {
                    // (the majority, bit by bit)
                    let mut r = 0u64;
                    for b in 0..64 {
                        let ones = words.iter().filter(|&&x| (x >> b) & 1 == 1).count();
                        if 2 * ones > words.len() {
                            r |= 1u64 << b;
                        }
                    }
                    r
                }
            };
        }
        *work += (nw * ins.len()) as u64;
        vals.insert(u, out);
        stack.pop();
    }
    Some(vals)
}

/// The standard tables of `n` signals over all their assignments (bit m:
/// signal i is bit i of m), at least one word each.
fn assignment_tables(n: usize) -> Vec<Vec<u64>> {
    let nw = ((1usize << n) + 63) / 64;
    (0..n)
        .map(|i| {
            (0..nw)
                .map(|w| {
                    let mut word = 0u64;
                    for b in 0..64 {
                        if ((w * 64 + b) >> i) & 1 == 1 {
                            word |= 1u64 << b;
                        }
                    }
                    word
                })
                .collect()
        })
        .collect()
}

/// For a cut that fails the cell's identity on assignment `bad`: the gates
/// of a smallest set of the cut's signals whose values there no input
/// assignment gives together (the mapper used the impossible combination
/// as a don't-care, so the cut must go under them). Inputs exhaustively
/// when there are at most 16 under the cut, else the random patterns.
fn unreachable_set(net: &Netlist, cut: &[i32], bad: usize, val: &[Sig], work: &mut u64) -> Vec<i32> {
    let nv = net.rep.len();
    let mut under: Vec<i32> = Vec::new();
    let mut seen: HashSet<i32> = HashSet::new();
    let mut stack: Vec<i32> = cut.to_vec();
    while let Some(u) = stack.pop() {
        let u = net.rep[u as usize].0;
        if !seen.insert(u) {
            continue;
        }
        *work += 1;
        match net.gates[u as usize].as_ref() {
            Some(g) => stack.extend(g.sup.iter().copied()),
            None => under.push(u),
        }
        if seen.len() > MAX_REGION {
            break;
        }
    }
    under.sort_unstable();
    let tables: Option<Vec<Vec<u64>>> = if under.len() <= 16 && seen.len() <= MAX_REGION {
        let init: HashMap<i32, Vec<u64>> = under.iter().copied().zip(assignment_tables(under.len())).collect();
        eval_tables(net, init, cut, &HashMap::new(), work).and_then(|vals| cut.iter().map(|&k| table_of(&vals, net, k)).collect())
    } else {
        Some(cut.iter().map(|&k| {
            let (r, ng) = net.rep[k as usize];
            sig_lit(val[r as usize], ng).to_vec()
        }).collect())
    };
    let gates_of = |q: &[usize]| -> Vec<i32> { q.iter().map(|&i| cut[i]).filter(|&k| (k as usize) < nv && net.gates[k as usize].is_some()).collect() };
    let Some(tables) = tables else { return gates_of(&(0..cut.len()).collect::<Vec<_>>()) };
    let nw = tables[0].len();
    let valid_bits = if under.len() <= 16 { (1usize << under.len()).min(64 * nw) } else { 64 * nw };
    for size in 2..=cut.len() {
        // (the subsets of this size, in order)
        let mut q: Vec<usize> = (0..size).collect();
        loop {
            if !gates_of(&q).is_empty() {
                let mut any = false;
                for w in 0..nw {
                    let mut m = u64::MAX;
                    for &i in &q {
                        let t = tables[i][w];
                        m &= if (bad >> i) & 1 == 1 { t } else { !t };
                    }
                    let lim = valid_bits.saturating_sub(64 * w).min(64);
                    if lim < 64 {
                        m &= (1u64 << lim) - 1;
                    }
                    if m != 0 {
                        any = true;
                        break;
                    }
                }
                *work += (nw * size) as u64;
                if !any {
                    return gates_of(&q);
                }
            }
            // the next subset
            let mut i = size;
            while i > 0 && q[i - 1] == cut.len() - size + i - 1 {
                i -= 1;
            }
            if i == 0 {
                break;
            }
            q[i - 1] += 1;
            for j in i..size {
                q[j] = q[j - 1] + 1;
            }
        }
    }
    gates_of(&(0..cut.len()).collect::<Vec<_>>())
}

/// The cut a cell's case split runs over: the smallest set of signals
/// that determines its outputs and its leaves' signals, under every
/// assignment of which the cell's identity holds (`check` gives the first
/// failing assignment). Nothing strictly above the leaves is cut (the
/// outputs would be free of them); a failing cut is pushed down under
/// the signals it read in an impossible combination.
fn find_cut(net: &Netlist, targets: &[i32], basis: &[i32], val: &[Sig], check: &mut dyn FnMut(&[i32], &mut u64) -> Option<Option<usize>>, work: &mut u64) -> Option<Vec<i32>> {
    let roots: Vec<i32> = targets.iter().chain(basis.iter()).copied().collect();
    let reg = region(net, &roots, work)?;
    let tset: HashSet<i32> = targets.iter().map(|&v| net.rep[v as usize].0).collect();
    let bset: HashSet<i32> = basis.iter().map(|&v| net.rep[v as usize].0).collect();
    let mut below: HashSet<i32> = HashSet::new();
    let mut stack: Vec<i32> = bset.iter().copied().collect();
    stack.sort_unstable();
    while let Some(u) = stack.pop() {
        if !below.insert(u) {
            continue;
        }
        if let Some(g) = net.gates[u as usize].as_ref() {
            stack.extend(g.sup.iter().map(|&x| net.rep[x as usize].0));
        }
    }
    *work += below.len() as u64;
    let mut forbidden: HashSet<i32> = reg.nodes.iter().copied().filter(|u| !below.contains(u) && net.gates[*u as usize].is_some()).collect();
    for _ in 0..32 {
        let cut = min_cut(net, &reg, &tset, &bset, &forbidden, work)?;
        match check(&cut, work)? {
            None => return Some(cut),
            Some(bad) => {
                let q = unreachable_set(net, &cut, bad, val, work);
                if q.is_empty() || q.iter().all(|u| forbidden.contains(u)) {
                    return None;
                }
                forbidden.extend(q);
            }
        }
    }
    None
}

/// The cells of a mapped Booth side, by function (see the module notes);
/// None when the side is no Booth tree this reads, or past the budget.
#[allow(clippy::too_many_arguments)]
pub(super) fn booth_cells(net: &Netlist, readers: &[u32], cone: &HashSet<i32>, out: i32, structural: usize, first_fresh: i32, and_rows: Option<(&[i32], &[i32])>, operands_out: &mut Option<(Vec<i32>, Vec<i32>)>, work: &mut u64, stopped: &dyn Fn(u64) -> bool, dbg: bool) -> Option<BoothCells> {
    let nv = net.rep.len();
    if stopped(*work + net.order.len() as u64) {
        return None; // (not affordable: the scan below is not started)
    }
    let order: Vec<i32> = net.order.iter().copied().filter(|v| net.rep[*v as usize].0 == *v && cone.contains(v)).collect();
    *work += net.order.len() as u64;
    // the inputs under every gate of the side, as bit sets
    let mut input_index: HashMap<i32, usize> = HashMap::new();
    let mut inputs: Vec<i32> = Vec::new();
    let mut supp: HashMap<i32, u128> = HashMap::new();
    for &v in &order {
        if stopped(*work) {
            return None;
        }
        let g = net.gates[v as usize].as_ref().unwrap();
        let mut s = 0u128;
        for &u in &g.sup {
            let r = net.rep[u as usize].0;
            if net.gates[r as usize].is_some() {
                s |= supp.get(&r).copied().unwrap_or(0);
            } else {
                let i = match input_index.get(&r) {
                    Some(&i) => i,
                    None => {
                        if inputs.len() >= MAX_INPUTS {
                            return None;
                        }
                        input_index.insert(r, inputs.len());
                        inputs.push(r);
                        inputs.len() - 1
                    }
                };
                s |= 1u128 << i;
            }
        }
        supp.insert(v, s);
        *work += g.sup.len() as u64;
    }
    if stopped(*work) {
        return None;
    }
    let bits = |s: u128| -> Vec<i32> {
        let mut v: Vec<i32> = (0..inputs.len()).filter(|&i| (s >> i) & 1 == 1).map(|i| inputs[i]).collect();
        v.sort_unstable();
        v
    };
    // (an AND side: its rows are the products of the operands another
    // side's digits and data gave, each at the sum of its places, bead
    // skz.14; else the Booth rows, read from the side's gates)
    let (row_gates, digits, data) = if and_rows.is_some() {
        (0usize, Vec::new(), Vec::new())
    } else {
    // the gates that compute a row bit or its select part
    let mut memo: HashMap<i32, u64> = HashMap::new();
    let mut digit_keys: HashMap<([Option<i32>; 2], Option<i32>), usize> = HashMap::new();
    let mut data_pairs: HashMap<(Option<i32>, Option<i32>), usize> = HashMap::new();
    let mut row_gates = 0usize;
    // (the tables of ones' counts: done when the limit covers them)
    if stopped(*work + 2 * 1024 * 32) {
        return None;
    }
    let popcounts: [Vec<u32>; 2] = [row_popcounts(4), row_popcounts(5)];
    *work += 2 * 1024 * 32;
    for &v in &order {
        // (the limit looked at before any early continue below)
        if stopped(*work) {
            return None;
        }
        *work += 1;
        let s = supp[&v];
        let n = s.count_ones() as usize;
        if !(4..=5).contains(&n) {
            continue;
        }
        let sup = bits(s);
        let Some(tt) = cut_truth_table(v, &sup, net, &mut memo, work) else { continue };
        if !popcounts[n - 4].contains(&tt.count_ones()) {
            continue;
        }
        let Some(r) = booth_roles(tt, n, work) else { continue };
        let var = |i: Option<usize>| i.map(|i| sup[i]);
        let mut pair = [var(r.pair[0]), var(r.pair[1])];
        pair.sort_unstable();
        *digit_keys.entry((pair, var(r.hi))).or_default() += 1;
        *data_pairs.entry((var(r.dhi), var(r.dlo))).or_default() += 1;
        row_gates += 1;
    }
    if row_gates < MIN_ROW_GATES {
        return None;
    }
    // the digits: the first has one low bit (lo is zero), each next one's
    // lo is the previous one's sign
    let best = |keys: Vec<(&([Option<i32>; 2], Option<i32>), &usize)>| -> Option<([Option<i32>; 2], Option<i32>)> { keys.into_iter().max_by_key(|(k, &c)| (c, std::cmp::Reverse(**k))).map(|(k, _)| *k) };
    let first = best(digit_keys.iter().filter(|((p, h), _)| p[0].is_none() && p[1].is_some() && h.is_some()).collect())?;
    let mut digits: Vec<(Option<i32>, Option<i32>, Option<i32>)> = vec![(None, first.0[1], first.1)];
    let mut used_inputs: HashSet<i32> = [first.0[1].unwrap(), first.1.unwrap()].into_iter().collect();
    loop {
        let hi = digits.last().unwrap().2.unwrap();
        let Some(next) = best(digit_keys.iter().filter(|((p, h), _)| p[0].is_some() && p[1].is_some() && h.is_some() && p.contains(&Some(hi))).collect()) else { break };
        let mid = if next.0[0] == Some(hi) { next.0[1] } else { next.0[0] };
        let nh = next.1.unwrap();
        if !used_inputs.insert(nh) || !used_inputs.insert(mid.unwrap()) {
            return None; // (a cycle)
        }
        digits.push((Some(hi), mid, Some(nh)));
        if digits.len() > MAX_INPUTS {
            return None;
        }
    }
    // the unsigned multiplier's last digit: the top bit alone (its row is
    // the top bit times the operand, plain products)
    let top = digits.last().unwrap().2;
    digits.push((top, None, None));
    // the other operand, in order: the bit selected for one sits one
    // place above the bit selected for two
    let mut succ: HashMap<i32, i32> = HashMap::new();
    let mut data_vars: HashSet<i32> = HashSet::new();
    for (&(dhi, dlo), _) in &data_pairs {
        if let Some(x) = dhi {
            data_vars.insert(x);
        }
        if let Some(x) = dlo {
            data_vars.insert(x);
        }
        if let (Some(h), Some(l)) = (dhi, dlo) {
            match succ.get(&l) {
                Some(&h0) if h0 != h => return None,
                _ => {
                    succ.insert(l, h);
                }
            }
        }
    }
    let succs: HashSet<i32> = succ.values().copied().collect();
    let mut starts: Vec<i32> = data_vars.iter().copied().filter(|x| !succs.contains(x)).collect();
    starts.sort_unstable();
    if starts.len() != 1 {
        return None;
    }
    let mut data: Vec<i32> = vec![starts[0]];
    while let Some(&x) = succ.get(data.last().unwrap()) {
        data.push(x);
        if data.len() > MAX_INPUTS {
            return None;
        }
    }
    if data.len() != data_vars.len() || data.len() < 4 || digits.len() < 3 {
        return None;
    }
    // (the operands, for an AND side: the data bits in order, and the
    // multiplier's bits as the digits read them, two per digit)
    let b_bits: Vec<i32> = digits[..digits.len() - 1].iter().flat_map(|&(_, mid, hi)| [mid, hi]).flatten().collect();
    *operands_out = Some((data.clone(), b_bits));
    (row_gates, digits, data)
    };
    let val = simulate_words(net, work, stopped)?;
    let word_of = |v: i32| -> Sig {
        let (r, ng) = net.rep[v as usize];
        sig_lit(val[r as usize], ng)
    };
    let zero: Sig = [0; WORDS];
    // the gates by their words, both polarities (the first in circuit
    // order); the row items below
    let mut by_word: HashMap<Sig, (i32, bool)> = HashMap::new();
    for &v in &order {
        let w = val[v as usize];
        by_word.entry(w).or_insert((v, false));
        by_word.entry(sig_not(w)).or_insert((v, true));
    }
    *work += 2 * order.len() as u64;
    let n = match and_rows {
        Some((a, _)) => a.len(),
        None => data.len(),
    };
    let ncols = match and_rows {
        Some((a, b)) => a.len() + b.len() + 2,
        None => n + 2 * digits.len() + 4,
    };
    // the signals some gate outside this side's cone reads (a product of
    // the correction row, the multiplier's top bit times a data bit, may
    // be the other side's own product gate, hashed into one: the other
    // side adds it a column lower, bead skz.14)
    let mut read_outside: HashSet<i32> = HashSet::new();
    if stopped(*work + nv as u64) {
        return None; // (not affordable: the scan is not started, review round 24)
    }
    for v in 1..nv {
        if net.rep[v].0 != v as i32 || cone.contains(&(v as i32)) {
            continue;
        }
        if let Some(g) = net.gates[v].as_ref() {
            read_outside.extend(g.sup.iter().map(|&u| net.rep[u as usize].0));
        }
    }
    *work += nv as u64;
    let mut items: Vec<Item> = Vec::new();
    let mut by_col: Vec<Vec<usize>> = vec![Vec::new(); ncols];
    let mut item_vars: HashSet<i32> = HashSet::new();
    let mut row_roots: Vec<i32> = Vec::new(); // the gates computing row bits or their select parts
    let input_bit = |x: Option<i32>| -> u128 { x.map_or(0, |x| input_index.get(&x).map_or(0, |&i| 1u128 << i)) };
    let w_opt = |x: Option<i32>| -> Sig { x.map_or(zero, word_of) };
    if let Some((a, b)) = and_rows {
        // the products: the side's gate when it has one, else a fresh and
        // (a mapped tree folds a product into the cell that reads it)
        for (i, &ai) in a.iter().enumerate() {
            for (k, &bk) in b.iter().enumerate() {
                if !input_index.contains_key(&ai) || !input_index.contains_key(&bk) {
                    continue; // (no gate of the side reads it)
                }
                let pp = sig_and(word_of(ai), word_of(bk));
                let col = i + k;
                let item = match by_word.get(&pp) {
                    Some(&(g, gneg)) => Item { var: g, make: None, word: val[g as usize], supp: supp[&g], alive: true, produced: false, second: false, col, is_carry: false, complement: gneg },
                    None => Item { var: 0, make: Some(FreshDef::And(vec![ai, bk])), word: pp, supp: input_bit(Some(ai)) | input_bit(Some(bk)), alive: true, produced: false, second: false, col, is_carry: false, complement: false },
                };
                if item.var != 0 {
                    item_vars.insert(item.var);
                    row_roots.push(item.var);
                }
                by_col[col].push(items.len());
                items.push(item);
            }
        }
    }
    for (j, &(lo, mid, hi)) in digits.iter().enumerate() {
        let (wl, wm, wh) = (w_opt(lo), w_opt(mid), w_opt(hi));
        let one = sig_xor(wl, wm);
        let two = sig_or(sig_and(wh, sig_and(sig_not(wm), sig_not(wl))), sig_and(sig_not(wh), sig_and(wm, wl)));
        for i in 0..=n {
            let dhi = data.get(i).copied();
            let dlo = if i > 0 { data.get(i - 1).copied() } else { None };
            let sel = sig_or(sig_and(one, w_opt(dhi)), sig_and(two, w_opt(dlo)));
            let pp = sig_xor(sel, wh);
            if pp == zero || pp == sig_not(zero) {
                continue;
            }
            let s = input_bit(lo) | input_bit(mid) | input_bit(hi) | input_bit(dhi) | input_bit(dlo);
            let col = i + 2 * j;
            // (a gate's support is its own: a row bit may not read every
            // input its formula names, and the gates over it read what it
            // reads)
            let item = if let Some(&(g, gneg)) = by_word.get(&pp) {
                if mid.is_none() && hi.is_none() && read_outside.contains(&g) {
                    // (a correction product the other side reads too: a
                    // copy of its own, so that each side weights it)
                    Item { var: 0, make: Some(FreshDef::Xor(vec![if gneg { -g } else { g }])), word: pp, supp: supp[&g], alive: true, produced: false, second: false, col, is_carry: false, complement: false }
                } else {
                    Item { var: g, make: None, word: val[g as usize], supp: supp[&g], alive: true, produced: false, second: false, col, is_carry: false, complement: gneg }
                }
            } else if let (Some(&(g, neg)), Some(h)) = (by_word.get(&sel), hi) {
                Item { var: 0, make: Some(FreshDef::Xor(vec![if neg { -g } else { g }, h])), word: pp, supp: supp[&g] | input_bit(hi), alive: true, produced: false, second: false, col, is_carry: false, complement: false }
            } else if let (Some(l), None, None, Some(d)) = (lo, mid, hi, dhi) {
                Item { var: 0, make: Some(FreshDef::And(vec![l, d])), word: pp, supp: s, alive: true, produced: false, second: false, col, is_carry: false, complement: false }
            } else {
                continue;
            };
            if item.var != 0 {
                item_vars.insert(item.var);
                row_roots.push(item.var);
            } else if let Some(FreshDef::Xor(l)) = &item.make {
                row_roots.push(l[0].abs());
            }
            by_col[col].push(items.len());
            items.push(item);
        }
        // the digit's sign: the row's +1 at its lowest column, and above
        // its top (the sign extension; the first row's at three columns)
        if let Some(h) = hi {
            let it = Item { var: h, make: None, word: word_of(h), supp: input_bit(hi), alive: true, produced: false, second: false, col: 0, is_carry: false, complement: false };
            let mut cols = vec![2 * j, n + 1 + 2 * j];
            if j == 0 {
                cols.extend([n + 2, n + 3]);
            }
            for c in cols {
                if c < ncols {
                    by_col[c].push(items.len());
                    items.push(Item { col: c, ..it.clone() });
                }
            }
        }
    }
    *work += items.len() as u64 * 4;
    if dbg {
        eprintln!("structarith: booth: {} row gates, {} digits, {} operand bits, {} row items over {} columns", row_gates, digits.len(), n, items.len(), ncols);
    }
    // (a side whose cells the structural matcher found, a Booth tree in
    // the generator's own form, has nearly as many as row items, 0.89 of
    // them on the 2025 cells: nothing to do here; a mapped one has 0.42)
    if and_rows.is_none() && 4 * structural >= 3 * items.len() {
        if dbg {
            eprintln!("structarith: booth: {} structural cells for {} row items: left as they are", structural, items.len());
        }
        return None;
    }
    // the matching
    let mut fresh: Vec<Fresh> = Vec::new();
    let mut defs: HashMap<i32, FreshDef> = HashMap::new();
    let mut basis_of: HashMap<i32, Vec<i32>> = HashMap::new();
    let mut next_fresh = first_fresh;
    let mut claimed: HashSet<i32> = HashSet::new();
    let mut cells: Vec<Cell> = Vec::new();
    let mut cell_inside: Vec<Vec<i32>> = Vec::new(); // per cell, the gates computed from its cut on the way to its outputs
    let mut inside_all: HashSet<i32> = HashSet::new(); // every cell's, so far: no output of another cell
    let mut cell_cols: Vec<usize> = Vec::new();
    let mut second_reads: Vec<u8> = Vec::new(); // per cell, its leaves read a second time (a column above where they are made)
    let mut fresh_producer: HashMap<i32, usize> = HashMap::new(); // a fresh output, its cell
    // (a partner gate: in the side, not taken, not a row item, not g)
    let partner = |w: Sig, g: i32, positive: bool, claimed: &HashSet<i32>, item_vars: &HashSet<i32>, inside_all: &HashSet<i32>| -> Option<(i32, bool)> {
        let &(h, neg) = by_word.get(&w)?;
        if h == g || claimed.contains(&h) || item_vars.contains(&h) || inside_all.contains(&h) || (positive && neg) {
            return None;
        }
        Some((h, neg))
    };
    // (whether g lies inside gate h's cone, read by nothing else: the
    // and-not inside a mapped xor is no carry)
    let inner = |g: i32, h: i32, work: &mut u64| -> bool {
        if readers[g as usize] > 1 {
            return false;
        }
        let mut seen: HashSet<i32> = HashSet::new();
        let mut stack = vec![h];
        while let Some(u) = stack.pop() {
            let u = net.rep[u as usize].0;
            if !seen.insert(u) || seen.len() > 64 {
                continue;
            }
            *work += 1;
            if u == g && u != h {
                return true;
            }
            if let Some(gt) = net.gates[u as usize].as_ref() {
                stack.extend(gt.sup.iter().copied());
            }
        }
        false
    };
    // level by level: a round reads only the items there were when it
    // began (a full adder's third leaf may come from a cell of the same
    // level, later in circuit order, and a half adder over the other two
    // would take them first); in each round full adders with both outputs
    // gates first, then half adders, then full adders with a fresh sum (a
    // dead or folded sum), then with a fresh carry (a dead carry)
    if stopped(*work + order.len() as u64) {
        return None;
    }
    let mut gate_readers: HashMap<i32, Vec<i32>> = HashMap::new();
    for &v in &order {
        for &u in &net.gates[v as usize].as_ref().unwrap().sup {
            gate_readers.entry(net.rep[u as usize].0).or_default().push(v);
        }
    }
    *work += order.len() as u64;
    let mut rejected: HashSet<(i32, u8)> = HashSet::new();
    let mut born: Vec<usize> = vec![0; items.len()];
    let mut round = 0usize;
    loop {
        round += 1;
        if round > 64 {
            break;
        }
        let mut made = 0usize;
        for phase in 0..6u8 {
        // (the half adders with a fresh partner only in a round where the
        // others made nothing: each phase scans every gate over every
        // column, and two more in every round cost an 18-bit mapped Booth
        // miter its budget; review round 23)
        if phase >= 4 && made > 0 {
            break;
        }
        for col in by_col.iter_mut() {
            col.retain(|&i| items[i].alive);
            *work += col.len() as u64;
        }
        for &g in &order {
            if stopped(*work) {
                return None;
            }
            *work += 1;
            if claimed.contains(&g) || item_vars.contains(&g) || inside_all.contains(&g) || rejected.contains(&(g, phase)) {
                continue;
            }
            let wg = val[g as usize];
            let ng = sig_not(wg);
            let sg = supp[&g];
            if sg.count_ones() < 2 {
                continue;
            }
            let mut found: Option<Cand> = None;
            'cols: for col in 0..ncols {
                // (and only where more than two signals are left)
                if phase >= 4 && by_col[col].len() <= 2 {
                    *work += 1;
                    continue;
                }
                *work += by_col[col].len() as u64;
                // (a second read only by a cell with both outputs gates:
                // the generator's doubled carries are read by full adders,
                // and a lone cell over two copies took a piece of the
                // network's own sum for an adder, bead skz.14)
                let cands: Vec<usize> = by_col[col].iter().copied().filter(|&i| items[i].alive && born[i] < round && items[i].supp & !sg == 0 && (phase <= 1 || !items[i].second)).collect();
                if cands.len() < 2 || cands.iter().fold(0u128, |a, &i| a | items[i].supp) != sg {
                    continue;
                }
                *work += cands.len() as u64;
                if phase == 0 || phase == 2 || phase == 3 {
                    // full adders
                    for a in 0..cands.len() {
                        for b in a + 1..cands.len() {
                            for c in b + 1..cands.len() {
                                let (ia, ib, ic) = (cands[a], cands[b], cands[c]);
                                if items[ia].supp | items[ib].supp | items[ic].supp != sg {
                                    continue;
                                }
                                *work += 8;
                                for pol in 0..8u8 {
                                    let x = sig_lit(items[ia].word, pol & 1 != 0);
                                    let y = sig_lit(items[ib].word, pol & 2 != 0);
                                    let z = sig_lit(items[ic].word, pol & 4 != 0);
                                    let m = sig_maj(x, y, z);
                                    let s = sig_xor(sig_xor(x, y), z);
                                    let leaves = vec![(ia, pol & 1 != 0), (ib, pol & 2 != 0), (ic, pol & 4 != 0)];
                                    if m == wg && phase != 3 {
                                        match partner(s, g, false, &claimed, &item_vars, &inside_all) {
                                            Some(sum) if phase == 0 => {
                                                found = Some(Cand { col, leaves, one: false, sum: Some(sum), carry: Some(g) });
                                                break 'cols;
                                            }
                                            None if phase == 2 => {
                                                found = Some(Cand { col, leaves, one: false, sum: None, carry: Some(g) });
                                                break 'cols;
                                            }
                                            _ => {}
                                        }
                                    } else if (s == wg || s == ng) && phase != 2 {
                                        match partner(m, g, true, &claimed, &item_vars, &inside_all) {
                                            Some((h, _)) if phase == 0 => {
                                                found = Some(Cand { col, leaves, one: false, sum: Some((g, s == ng)), carry: Some(h) });
                                                break 'cols;
                                            }
                                            None if phase == 3 => {
                                                found = Some(Cand { col, leaves, one: false, sum: Some((g, s == ng)), carry: None });
                                                break 'cols;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }
                // half adders (both outputs gates), with or without a one;
                // one with a fresh partner only in a column the tree has
                // not yet brought down to the final adder's two rows
                if (phase == 4 || phase == 5) && by_col[col].iter().filter(|&&i| items[i].alive).count() <= 2 {
                    continue;
                }
                for a in 0..cands.len() {
                    for b in a + 1..cands.len() {
                        let (ia, ib) = (cands[a], cands[b]);
                        if items[ia].supp | items[ib].supp != sg {
                            continue;
                        }
                        *work += 4;
                        for pol in 0..4u8 {
                            let x = sig_lit(items[ia].word, pol & 1 != 0);
                            let y = sig_lit(items[ib].word, pol & 2 != 0);
                            let s = sig_xor(x, y);
                            let leaves = vec![(ia, pol & 1 != 0), (ib, pol & 2 != 0)];
                            let mut cand: Option<Cand> = None;
                            // (phase 1 both outputs gates; phase 4 an xor
                            // whose carry no gate computes, a final adder's
                            // propagate whose generate it folds away; phase
                            // 5 a carry whose sum no gate computes, a dead
                            // low bit: each a fresh partner, bead skz.14)
                            let and_p = partner(sig_and(x, y), g, true, &claimed, &item_vars, &inside_all);
                            let or_p = partner(sig_or(x, y), g, true, &claimed, &item_vars, &inside_all);
                            if sig_and(x, y) == wg {
                                match partner(s, g, false, &claimed, &item_vars, &inside_all) {
                                    Some(sum) if phase == 1 => {
                                        if !inner(g, sum.0, work) {
                                            cand = Some(Cand { col, leaves, one: false, sum: Some(sum), carry: Some(g) });
                                        }
                                    }
                                    None if phase == 5 => cand = Some(Cand { col, leaves, one: false, sum: None, carry: Some(g) }),
                                    _ => {}
                                }
                            } else if sig_or(x, y) == wg {
                                match partner(sig_not(s), g, false, &claimed, &item_vars, &inside_all) {
                                    Some(sum) if phase == 1 => {
                                        if !inner(g, sum.0, work) {
                                            cand = Some(Cand { col, leaves, one: true, sum: Some(sum), carry: Some(g) });
                                        }
                                    }
                                    None if phase == 5 => cand = Some(Cand { col, leaves, one: true, sum: None, carry: Some(g) }),
                                    _ => {}
                                }
                            } else if s == wg || s == ng {
                                if phase == 1 {
                                    if let Some((h, _)) = and_p {
                                        if !inner(h, g, work) {
                                            cand = Some(Cand { col, leaves, one: false, sum: Some((g, s == ng)), carry: Some(h) });
                                        }
                                    } else if let Some((h, _)) = or_p {
                                        if !inner(h, g, work) {
                                            cand = Some(Cand { col, leaves, one: true, sum: Some((g, s == wg)), carry: Some(h) });
                                        }
                                    }
                                } else if phase == 4 && and_p.is_none() && or_p.is_none() {
                                    cand = Some(Cand { col, leaves, one: false, sum: Some((g, s == ng)), carry: None });
                                }
                            }
                            let Some(cand) = cand else { continue };
                            // (two leaves of a full adder whose carry is a
                            // gate: the full adder is the cell, next round)
                            let third = by_col[col].iter().any(|&iz| {
                                iz != ia && iz != ib && items[iz].alive && (0..8u8).any(|p| {
                                    let m = sig_maj(sig_lit(items[ia].word, p & 1 != 0), sig_lit(items[ib].word, p & 2 != 0), sig_lit(items[iz].word, p & 4 != 0));
                                    partner(m, 0, true, &claimed, &item_vars, &inside_all).is_some()
                                })
                            });
                            *work += by_col[col].len() as u64 * 8;
                            if third {
                                continue;
                            }
                            found = Some(cand);
                            break 'cols;
                        }
                    }
                }
            }
            let Some(cand) = found else { continue };
            if let Some(maxcol) = std::env::var("SAT_STRUCT_DEBUG_BOOTH_MAXCOL").ok().and_then(|t| t.parse::<usize>().ok()) {
                if cand.col > maxcol {
                    continue; // (debug: the columns above are left to the carry network)
                }
            }
            // the leaves as variables (a fused row bit made now) with their
            // bases: the netlist signals that determine them
            let mut leaf_vars: Vec<(i32, bool)> = Vec::new();
            let mut new_fresh: Vec<Fresh> = Vec::new();
            let mut nf = next_fresh;
            for &(i, neg) in &cand.leaves {
                let v = if items[i].var != 0 {
                    items[i].var
                } else {
                    let x = nf;
                    nf += 1;
                    new_fresh.push(Fresh { var: x, def: items[i].make.clone().unwrap() });
                    x
                };
                leaf_vars.push((v, neg));
            }
            let leaf_lits: Vec<i32> = leaf_vars.iter().map(|&(v, neg)| if neg { -v } else { v }).collect();
            let basis_vars = |v: i32, basis_of: &HashMap<i32, Vec<i32>>, new_fresh: &[Fresh]| -> Vec<i32> {
                if (v as usize) < nv {
                    return vec![v];
                }
                if let Some(b) = basis_of.get(&v) {
                    return b.clone();
                }
                let fr = new_fresh.iter().find(|f| f.var == v).unwrap();
                match &fr.def {
                    FreshDef::Xor(l) | FreshDef::And(l) | FreshDef::Maj(l) => l.iter().map(|x| x.abs()).collect(),
                }
            };
            let mut basis: Vec<i32> = Vec::new();
            for &(v, _) in &leaf_vars {
                basis.extend(basis_vars(v, &basis_of, &new_fresh));
            }
            basis.sort_unstable();
            basis.dedup();
            // the outputs: a fresh one defined over the leaves
            let fa = cand.leaves.len() == 3;
            let (sum_var, sum_neg) = match cand.sum {
                Some((h, neg)) => (h, neg),
                None => {
                    let x = nf;
                    nf += 1;
                    new_fresh.push(Fresh { var: x, def: FreshDef::Xor(leaf_lits.clone()) });
                    (x, cand.one) // (with a one folded in the sum is the xnor)
                }
            };
            let carry_lit = match cand.carry {
                Some(h) => h,
                None => {
                    let x = nf;
                    nf += 1;
                    if fa {
                        new_fresh.push(Fresh { var: x, def: FreshDef::Maj(leaf_lits.clone()) });
                        x
                    } else if cand.one {
                        new_fresh.push(Fresh { var: x, def: FreshDef::And(leaf_lits.iter().map(|&l| -l).collect()) });
                        -x // (x or y is the complement of ~x and ~y)
                    } else {
                        new_fresh.push(Fresh { var: x, def: FreshDef::And(leaf_lits.clone()) });
                        x
                    }
                }
            };
            let targets: Vec<i32> = [cand.sum.map(|(h, _)| h), cand.carry].into_iter().flatten().collect();
            // the cut and the exact check over it: under every assignment of
            // its signals the gates and the fresh definitions give the
            // leaves and the outputs of the cell's identity
            let mut all_defs = defs.clone();
            for fr in &new_fresh {
                all_defs.insert(fr.var, fr.def.clone());
            }
            let mut want: Vec<i32> = leaf_lits.clone();
            want.push(sum_var);
            want.push(carry_lit.abs());
            let mut vals_kept: Option<HashMap<i32, Vec<u64>>> = None;
            let mut check = |cut: &[i32], work: &mut u64| -> Option<Option<usize>> {
                let init: HashMap<i32, Vec<u64>> = cut.iter().copied().zip(assignment_tables(cut.len())).collect();
                let vals = eval_tables(net, init, &want, &all_defs, work)?;
                let get = |l: i32| table_of(&vals, net, l);
                let ls: Vec<Vec<u64>> = leaf_lits.iter().map(|&l| get(l)).collect::<Option<Vec<Vec<u64>>>>()?;
                let s = get(if sum_neg { -sum_var } else { sum_var })?;
                let c = get(carry_lit)?;
                let nw = s.len();
                // (the leaves take every combination of values over the
                // cut: a leaf that is a function of the others, the nor of
                // the two others say, makes an identity but no adder; a
                // carry network's generate and kill over one pair do)
                let valid = (1usize << cut.len()).min(64 * nw);
                for m in 0..(1usize << ls.len()) {
                    let mut any = false;
                    for w in 0..nw {
                        let mut x = u64::MAX;
                        for (k, l) in ls.iter().enumerate() {
                            x &= if (m >> k) & 1 == 1 { l[w] } else { !l[w] };
                        }
                        let lim = valid.saturating_sub(64 * w).min(64);
                        if lim < 64 {
                            x &= (1u64 << lim) - 1;
                        }
                        if x != 0 {
                            any = true;
                            break;
                        }
                    }
                    if !any {
                        return None;
                    }
                }
                for w in 0..nw {
                    let (es, ec) = if fa {
                        (ls[0][w] ^ ls[1][w] ^ ls[2][w], (ls[0][w] & ls[1][w]) | (ls[0][w] & ls[2][w]) | (ls[1][w] & ls[2][w]))
                    } else if cand.one {
                        (!(ls[0][w] ^ ls[1][w]), ls[0][w] | ls[1][w])
                    } else {
                        (ls[0][w] ^ ls[1][w], ls[0][w] & ls[1][w])
                    };
                    let diff = (s[w] ^ es) | (c[w] ^ ec);
                    if diff != 0 {
                        let bad = w * 64 + diff.trailing_zeros() as usize;
                        return Some(Some(bad & ((1usize << cut.len()) - 1)));
                    }
                }
                vals_kept = Some(vals);
                Some(None)
            };
            let Some(cut) = find_cut(net, &targets, &basis, &val, &mut check, work) else {
                rejected.insert((g, phase));
                if dbg {
                    eprintln!("structarith: booth: gate {} matched at column {} (leaves {:?}), but no cut of at most {} signals passes", g, cand.col, leaf_lits, MAX_SPLIT);
                }
                continue;
            };
            let vals = vals_kept;
            // commit: the leaves' items are read, the fused ones made; the
            // outputs are items at their columns; the gates computed on the
            // way from the cut to the outputs are the cell's own
            next_fresh = nf;
            let ws: Vec<Sig> = cand.leaves.iter().map(|&(i, neg)| sig_lit(items[i].word, neg)).collect();
            let out_supp = cand.leaves.iter().fold(0u128, |a, &(i, _)| a | items[i].supp);
            for (k, &(i, _)) in cand.leaves.iter().enumerate() {
                items[i].alive = false;
                if items[i].var == 0 {
                    items[i].var = leaf_vars[k].0;
                }
            }
            for fr in &new_fresh {
                let lits = match &fr.def {
                    FreshDef::Xor(l) | FreshDef::And(l) | FreshDef::Maj(l) => l.clone(),
                };
                let mut bb: Vec<i32> = Vec::new();
                for l in &lits {
                    bb.extend(basis_vars(l.abs(), &basis_of, &new_fresh));
                }
                bb.sort_unstable();
                bb.dedup();
                basis_of.insert(fr.var, bb);
                defs.insert(fr.var, fr.def.clone());
            }
            let sum_word = if fa { sig_xor(sig_xor(ws[0], ws[1]), ws[2]) } else if cand.one { sig_not(sig_xor(ws[0], ws[1])) } else { sig_xor(ws[0], ws[1]) };
            let carry_word = if fa { sig_maj(ws[0], ws[1], ws[2]) } else if cand.one { sig_or(ws[0], ws[1]) } else { sig_and(ws[0], ws[1]) };
            // (an item carries its plain variable's word)
            for (v, w, c, is_carry) in [(sum_var, sig_lit(sum_word, sum_neg), cand.col, false), (carry_lit.abs(), sig_lit(carry_word, carry_lit < 0), cand.col + 1, true)] {
                let mut sv = out_supp;
                if (v as usize) < nv {
                    claimed.insert(v);
                    sv = supp[&v];
                }
                if c < ncols {
                    by_col[c].push(items.len());
                    items.push(Item { var: v, make: None, word: w, supp: sv, alive: true, produced: true, second: false, col: c, is_carry, complement: false });
                    born.push(round);
                }
            }
            let mut own: Vec<i32> = Vec::new();
            if let Some(vals) = vals.as_ref() {
                for &u in vals.keys() {
                    if (u as usize) < nv && !cut.contains(&u) && !targets.contains(&u) && net.gates[u as usize].is_some() {
                        own.push(u);
                    }
                }
                own.sort_unstable();
                inside_all.extend(own.iter().copied());
                // a produced leaf some gate outside this cell reads too: a
                // Booth Wallace tree reads a signal at its own column and
                // again a column above (2x - x is x, bead skz.4), so it can
                // be read once more there
                for &(i, _) in &cand.leaves {
                    let x = items[i].var;
                    if !items[i].produced || !items[i].is_carry || items[i].second || (x as usize) >= nv || cand.col + 1 >= ncols {
                        continue;
                    }
                    let outside = gate_readers.get(&x).map_or(false, |rs| rs.iter().any(|r| !vals.contains_key(r) && !targets.contains(r)));
                    *work += 1;
                    if outside {
                        let mut ghost = items[i].clone();
                        ghost.alive = true;
                        ghost.second = true;
                        ghost.col = cand.col + 1;
                        by_col[cand.col + 1].push(items.len());
                        items.push(ghost);
                        born.push(round);
                    }
                }
            }
            let mut negmask = 0u8;
            for (k, &(_, neg)) in leaf_vars.iter().enumerate() {
                if neg {
                    negmask |= 1 << k;
                }
            }
            if dbg && std::env::var_os("SAT_STRUCT_DEBUG_BOOTH").is_some() {
                eprintln!("structarith: booth: cell at column {}: leaves {:?} (+{}) = {}{} + 2*{} over the cut {:?}{}", cand.col, leaf_lits, cand.one as u8, if sum_neg { "-" } else { "" }, sum_var, carry_lit, cut, if cand.sum.is_none() || cand.carry.is_none() { " (a fresh partner)" } else { "" });
            }
            for (v, real) in [(sum_var, cand.sum.is_some()), (carry_lit.abs(), cand.carry.is_some())] {
                if !real {
                    fresh_producer.insert(v, cells.len());
                }
            }
            let mut sec = 0u8;
            for (k, &(i, _)) in cand.leaves.iter().enumerate() {
                if items[i].second {
                    sec |= 1 << k;
                }
            }
            fresh.extend(new_fresh);
            cell_inside.push(own);
            cell_cols.push(cand.col);
            second_reads.push(sec);
            cells.push(Cell { fa, one: cand.one, leaves: leaf_vars.iter().map(|&(v, _)| v).collect(), negmask, sum: sum_var, sum_neg, carry: carry_lit, split: Some(cut) });
            made += 1;
        }
        }
        if made == 0 {
            break;
        }
    }
    if dbg {
        let left: Vec<(usize, Vec<i32>)> = (0..ncols).map(|c| (c, by_col[c].iter().filter(|&&i| items[i].alive && items[i].var != 0).map(|&i| items[i].var).collect::<Vec<i32>>())).filter(|(_, v)| !v.is_empty()).collect();
        eprintln!("structarith: booth: {} cells ({} with a fresh partner) in {} rounds; left per column {:?}", cells.len(), cells.iter().filter(|c| c.sum as usize >= nv || c.carry_var() as usize >= nv).count(), round, left);
    }
    // the cells must read every produced signal as it is written, up to a
    // flip of a whole cell (the two readings of one identity; a signal read
    // a second time, a column above, is read the other way round by its
    // first reader, bead skz.4). An odd cycle of such constraints means
    // one of its cells is no cell of the tree (at the top a mapper merges
    // the tree's last cells into the final adder): the one highest in the
    // tree is dropped, a cell with a fresh partner first, and with it every
    // cell reading what it made, until no cycle is left. (The network may
    // read a cell's leaves too: it recomputes a fresh output from them, or
    // a half adder over its two inputs is its own propagate and generate;
    // the sums agree either way.)
    let mut alive_cell = vec![true; cells.len()];
    let mut final_color: Vec<u8> = vec![0; cells.len()]; // each kept cell's flip in a consistent reading
    let lone_cell = |ci: usize, cells: &[Cell]| -> bool { (cells[ci].sum as usize) >= nv || (cells[ci].carry_var() as usize) >= nv };
    loop {
        if stopped(*work) {
            return None;
        }
        // what a dropped cell made is read by no cell: its readers go too
        loop {
            let mut produced: HashSet<i32> = HashSet::new();
            let mut first_reads: HashMap<i32, usize> = HashMap::new();
            for (ci, c) in cells.iter().enumerate() {
                if !alive_cell[ci] {
                    continue;
                }
                produced.insert(c.sum);
                produced.insert(c.carry_var());
                for (k, &l) in c.leaves.iter().enumerate() {
                    if (second_reads[ci] >> k) & 1 == 0 {
                        *first_reads.entry(l).or_default() += 1;
                    }
                }
            }
            *work += cells.len() as u64;
            let mut changed = false;
            for ci in 0..cells.len() {
                if !alive_cell[ci] {
                    continue;
                }
                let c = &cells[ci];
                let bad = c.leaves.iter().enumerate().any(|(k, &l)| {
                    if (second_reads[ci] >> k) & 1 == 1 && first_reads.get(&l).copied().unwrap_or(0) == 0 {
                        return true; // (a second read whose first is gone)
                    }
                    if (l as usize) < nv {
                        net.gates[l as usize].is_some() && !item_vars.contains(&l) && !produced.contains(&l)
                    } else {
                        fresh_producer.get(&l).map_or(false, |&pc| !alive_cell[pc])
                    }
                });
                if bad {
                    alive_cell[ci] = false;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // the constraints, two-coloured breadth first
        let mut producer: HashMap<i32, (usize, u8)> = HashMap::new();
        let mut ghost_reader: HashMap<i32, usize> = HashMap::new();
        for (ci, c) in cells.iter().enumerate() {
            if !alive_cell[ci] {
                continue;
            }
            producer.insert(c.sum, (ci, c.sum_neg as u8));
            producer.insert(c.carry_var(), (ci, (c.carry < 0) as u8));
            for (k, &l) in c.leaves.iter().enumerate() {
                if (second_reads[ci] >> k) & 1 == 1 {
                    ghost_reader.insert(l, ci);
                }
            }
        }
        let mut adj: Vec<Vec<(usize, u8)>> = vec![Vec::new(); cells.len()];
        for (ci, c) in cells.iter().enumerate() {
            if !alive_cell[ci] {
                continue;
            }
            for (k, &l) in c.leaves.iter().enumerate() {
                let Some(&(pc, p)) = producer.get(&l) else { continue };
                if pc == ci {
                    continue;
                }
                let q = (c.negmask >> k) & 1;
                let opposite = ghost_reader.get(&l).map_or(0, |&g| (g != ci) as u8);
                adj[pc].push((ci, p ^ q ^ opposite));
                adj[ci].push((pc, p ^ q ^ opposite));
            }
        }
        *work += cells.len() as u64 * 4;
        let mut color: Vec<Option<u8>> = vec![None; cells.len()];
        let mut parent: Vec<usize> = vec![usize::MAX; cells.len()];
        let mut conflict: Option<(usize, usize)> = None;
        'bfs: for start in 0..cells.len() {
            if !alive_cell[start] || color[start].is_some() {
                continue;
            }
            color[start] = Some(0);
            let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
            queue.push_back(start);
            while let Some(a) = queue.pop_front() {
                let ca = color[a].unwrap();
                for &(b, d) in &adj[a] {
                    match color[b] {
                        None => {
                            color[b] = Some(ca ^ d);
                            parent[b] = a;
                            queue.push_back(b);
                        }
                        Some(cb) if cb != ca ^ d => {
                            conflict = Some((a, b));
                            break 'bfs;
                        }
                        _ => {}
                    }
                }
                *work += adj[a].len() as u64;
            }
        }
        let Some((a, b)) = conflict else {
            final_color = color.iter().map(|c| c.unwrap_or(0)).collect();
            break;
        };
        // the cycle: both tree paths up to where they meet
        let mut up_a: Vec<usize> = vec![a];
        while parent[*up_a.last().unwrap()] != usize::MAX {
            up_a.push(parent[*up_a.last().unwrap()]);
        }
        let on_a: HashSet<usize> = up_a.iter().copied().collect();
        let mut cycle: Vec<usize> = Vec::new();
        let mut x = b;
        while !on_a.contains(&x) {
            cycle.push(x);
            x = parent[x];
        }
        for &y in &up_a {
            cycle.push(y);
            if y == x {
                break;
            }
        }
        let pick = *cycle.iter().max_by_key(|&&ci| (cell_cols[ci], lone_cell(ci, &cells), ci)).unwrap();
        alive_cell[pick] = false;
        if dbg && std::env::var_os("SAT_STRUCT_DEBUG_BOOTH").is_some() {
            let c = &cells[pick];
            let lits: Vec<i32> = (0..c.leaves.len()).map(|i| c.leaf_lit(i)).collect();
            eprintln!("structarith: booth: an odd cycle of {} cells; the cell at column {} {:?} = {} + 2*{} dropped", cycle.len(), cell_cols[pick], lits, c.sum_lit(), c.carry_lit());
            for &cj in &cycle {
                let d = &cells[cj];
                let l: Vec<i32> = (0..d.leaves.len()).map(|i| d.leaf_lit(i)).collect();
                eprintln!("    cycle cell {} at column {}: {:?} (+{}) = {} + 2*{} second reads {:#b}", cj, cell_cols[cj], l, d.one as u8, d.sum_lit(), d.carry_lit(), second_reads[cj]);
            }
        }
    }
    let dropped = alive_cell.iter().filter(|&&a| !a).count();
    // the side's top: when no cell computes the compared signal, the
    // carry into its column is the network's output. The column is the
    // highest a row bit is computed at; what the cells leave over there
    // (and the row bits there no cell reads) is summed with a fresh
    // variable n = out xor all of it by lone sums of the pass's own, the
    // last one's sum the compared signal; the lemma then proves n equal to
    // the chain's bit there
    let mut top_cells: Vec<Cell> = Vec::new();
    let mut virtual_net: Option<i32> = None;
    let out_rep = net.rep[out as usize].0;
    let made_out = cells.iter().enumerate().any(|(ci, c)| alive_cell[ci] && (c.sum == out_rep || c.carry_var() == out_rep));
    // (a row bit some gate computes, or whose select part one does: the
    // product items of the last row exist at every column)
    let gate_backed = |it: &Item| -> bool {
        !it.produced && match &it.make {
            Some(FreshDef::Xor(_)) => true,
            Some(_) => false,
            None => (it.var as usize) < nv && net.gates[it.var as usize].is_some(),
        }
    };
    // (the compared signal's column by function, the product bit it is,
    // when the operands are known: the highest column a row bit has a gate
    // at is not the compared bit's at the product's top, where every row
    // bit is folded or an input, bead skz.15)
    let ops: Option<(Vec<i32>, Vec<i32>)> = match and_rows {
        Some((a, b)) => Some((a.to_vec(), b.to_vec())),
        None => operands_out.clone(),
    };
    let by_function = ops.as_ref().and_then(|(a, b)| product_column(net, &val, a, b, out, work)).filter(|&c| c < ncols);
    let top_col = by_function.or_else(|| items.iter().filter(|it| gate_backed(it)).map(|it| it.col).max());
    if let (false, Some(kc)) = (made_out, top_col) {
        let consumed: HashSet<i32> = cells.iter().enumerate().filter(|&(ci, _)| alive_cell[ci]).flat_map(|(_, c)| c.leaves.iter().copied()).collect();
        let produced: HashSet<i32> = cells.iter().enumerate().filter(|&(ci, _)| alive_cell[ci]).flat_map(|(_, c)| [c.sum, c.carry_var()]).collect();
        let mut left: Vec<i32> = Vec::new();
        for i in 0..items.len() {
            let it = &items[i];
            if it.col != kc || it.second {
                continue;
            }
            if it.produced {
                if produced.contains(&it.var) && !consumed.contains(&it.var) && !left.contains(&it.var) {
                    left.push(it.var);
                }
                continue;
            }
            if (it.var as usize) < nv && it.var != 0 && net.gates[it.var as usize].is_none() {
                continue; // (a digit's sign: not read at the top)
            }
            if it.var != 0 && consumed.contains(&it.var) {
                continue;
            }
            let v = if it.var != 0 && (it.var as usize) < nv {
                it.var
            } else if it.var != 0 && defs.contains_key(&it.var) {
                it.var
            } else {
                let x = next_fresh;
                next_fresh += 1;
                let def = it.make.clone().unwrap();
                fresh.push(Fresh { var: x, def: def.clone() });
                defs.insert(x, def);
                items[i].var = x; // (made once: the row bits left below must not make it again)
                x
            };
            if !left.contains(&v) {
                left.push(v);
            }
        }
        left.sort_unstable();
        if !left.is_empty() && left.len() <= 6 {
            // (each leaf read as its producer writes it under the cells'
            // consistent reading, so the top cells take part in it: a
            // complemented read only complements n, which the lemma takes
            // either way)
            let mut writer: HashMap<i32, u8> = HashMap::new();
            for (ci, c) in cells.iter().enumerate() {
                if alive_cell[ci] {
                    writer.insert(c.sum, c.sum_neg as u8 ^ final_color[ci]);
                    writer.insert(c.carry_var(), (c.carry < 0) as u8 ^ final_color[ci]);
                }
            }
            let left_lits: Vec<i32> = left.iter().map(|&x| if writer.get(&x).copied().unwrap_or(0) == 1 { -x } else { x }).collect();
            let n = next_fresh;
            next_fresh += 1;
            let mut lits = vec![out_rep];
            lits.extend(left_lits.iter().copied());
            fresh.push(Fresh { var: n, def: FreshDef::Xor(lits.clone()) });
            defs.insert(n, FreshDef::Xor(lits));
            let cell_of = |lits: &[i32]| -> (Vec<i32>, u8) {
                let mut negmask = 0u8;
                for (k, &l) in lits.iter().enumerate() {
                    if l < 0 {
                        negmask |= 1 << k;
                    }
                }
                (lits.iter().map(|l| l.abs()).collect(), negmask)
            };
            // the lone sums: three leaves at a time, the last one with n
            let mut queue: Vec<i32> = left_lits.clone();
            let mut split_base: Vec<i32> = Vec::new(); // (the netlist signals the leaves are over)
            for &l in &left {
                if (l as usize) < nv {
                    split_base.push(l);
                } else if let Some(d) = defs.get(&l) {
                    let (FreshDef::Xor(dl) | FreshDef::And(dl) | FreshDef::Maj(dl)) = d;
                    split_base.extend(dl.iter().map(|x| x.abs()));
                }
            }
            while queue.len() + 1 > 3 {
                let take: Vec<i32> = queue.drain(..3).collect();
                let (sx, cx) = (next_fresh, next_fresh + 1);
                next_fresh += 2;
                fresh.push(Fresh { var: sx, def: FreshDef::Xor(take.clone()) });
                fresh.push(Fresh { var: cx, def: FreshDef::Maj(take.clone()) });
                defs.insert(sx, FreshDef::Xor(take.clone()));
                defs.insert(cx, FreshDef::Maj(take.clone()));
                let mut sp = split_base.clone();
                sp.sort_unstable();
                sp.dedup();
                let (leaves, negmask) = cell_of(&take);
                top_cells.push(Cell { fa: true, one: false, leaves, negmask, sum: sx, sum_neg: false, carry: cx, split: Some(sp) });
                queue.insert(0, sx);
            }
            queue.push(n);
            let cx = next_fresh;
            next_fresh += 1;
            let def = if queue.len() == 3 { FreshDef::Maj(queue.clone()) } else { FreshDef::And(queue.clone()) };
            fresh.push(Fresh { var: cx, def: def.clone() });
            defs.insert(cx, def);
            let mut sp = split_base.clone();
            sp.push(out_rep);
            sp.sort_unstable();
            sp.dedup();
            if sp.len() <= MAX_SPLIT {
                let (leaves, negmask) = cell_of(&queue);
                top_cells.push(Cell { fa: queue.len() == 3, one: false, leaves, negmask, sum: out_rep, sum_neg: false, carry: cx, split: Some(sp) });
                virtual_net = Some(n);
            } else {
                top_cells.clear();
            }
        }
        if dbg {
            eprintln!("structarith: booth: the top at column {}: {} signals left there {:?}, the network's carry {:?}", kc, left.len(), left, virtual_net);
        }
    }
    // the row bits no kept cell reads (top cells included), with their
    // columns; a fused one gets its fresh variable now
    let mut rows_left: Vec<(i32, i64)> = Vec::new();
    {
        let mut read: HashSet<i32> = cells.iter().enumerate().filter(|&(ci, _)| alive_cell[ci]).flat_map(|(_, c)| c.leaves.iter().copied()).collect();
        if virtual_net.is_some() {
            read.extend(top_cells.iter().flat_map(|c| c.leaves.iter().copied()));
        }
        for i in 0..items.len() {
            let it = &items[i];
            if it.produced || it.second || !gate_backed(it) || (it.var != 0 && read.contains(&it.var)) {
                continue;
            }
            let lit = if it.var != 0 {
                if it.complement { -it.var } else { it.var }
            } else {
                let x = next_fresh;
                next_fresh += 1;
                let def = it.make.clone().unwrap();
                fresh.push(Fresh { var: x, def: def.clone() });
                defs.insert(x, def);
                x
            };
            rows_left.push((lit, it.col as i64));
        }
        rows_left.sort_unstable_by_key(|&(l, c)| (c, l));
    }
    let mut kept: Vec<Cell> = cells.into_iter().enumerate().filter(|&(ci, _)| alive_cell[ci]).map(|(_, c)| c).collect();
    let mut cols: Vec<i64> = cell_cols.iter().enumerate().filter(|&(ci, _)| alive_cell[ci]).map(|(_, &c)| c as i64).collect();
    if virtual_net.is_some() {
        let kc = top_col.unwrap_or(0) as i64;
        cols.extend(top_cells.iter().map(|_| kc));
        kept.extend(top_cells);
    }
    // the fresh variables the kept cells read or write, with what their
    // definitions read, in definition order
    let mut used_vars: HashSet<i32> = kept.iter().flat_map(|c| c.leaves.iter().copied().chain([c.sum, c.carry_var()])).filter(|&v| (v as usize) >= nv).collect();
    used_vars.extend(rows_left.iter().map(|&(l, _)| l.abs()).filter(|&v| (v as usize) >= nv));
    for fr in fresh.iter().rev() {
        if used_vars.contains(&fr.var) {
            let (FreshDef::Xor(l) | FreshDef::And(l) | FreshDef::Maj(l)) = &fr.def;
            used_vars.extend(l.iter().map(|x| x.abs()).filter(|&v| (v as usize) >= nv));
        }
    }
    fresh.retain(|fr| used_vars.contains(&fr.var));
    if dbg {
        eprintln!("structarith: booth: {} cells kept, {} dropped (odd cycles of readings, and what read them)", kept.len(), dropped);
    }
    Some(BoothCells { cells: kept, fresh, row_gates, virtual_net, cols, rows_left })
}
