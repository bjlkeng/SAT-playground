//! Not in kissat. The divider and square-root pass of the structure pass
//! (bead SAT-playground-skz.23, 2026-10-27): a one-output miter of two
//! dividers, or of two square roots, is decided by proving the rows'
//! signals equal one by one.
//!
//! In plain terms. Each of the five 2025 div and sqrt miters compares one
//! output bit of two versions of an EPFL circuit (the original against its
//! best 6-input-LUT version). Both versions compute that bit the textbook
//! way, row by row: in long division row j brings down dividend bit j,
//! compares the partial remainder with the divisor and subtracts when it
//! fits; the digit-by-digit square root brings down two bits a step. So
//! each row's quotient (root) bit and remainder bits exist as signals on
//! both sides. Proving them equal row after row is a chain of small checks,
//! each easy once the rows above it are proven; the last one is the
//! compared bit, and the miter then needs no search at all (solver README
//! "Divider and square-root miters: census and certificate";
//! `tools/divsqrt_census.py` is the prototype).
//!
//! What the pass does, in order:
//! 1. reads the formula as a circuit by first literal: every clause belongs
//!    to the gate its first literal names (the shape of ABC's write_cnf),
//!    and each group must define its variable by a truth table over at most
//!    six inputs; a unit on any other variable is a constant, and the one
//!    unit on a gate is the miter's root, an xor of one signal per side;
//! 2. finds the function by probes: a divider (the first side's output is
//!    bit k of floor(a / b) for n-bit a and b, b = 0 giving all ones as a
//!    restoring array does; the inputs are a[k..n-1] and b[0..n-1], and one
//!    input set at a time tells a's bits from b's) or a square root (the low
//!    bit of isqrt of the inputs read as one number). The inputs' bit
//!    positions are their order, or the text order of their names (b[0],
//!    b[10], ..., b[19], b[1], ...), whichever simulation on random inputs
//!    confirms; the pass declines otherwise;
//! 3. runs the restoring recurrence next to the circuit on random inputs
//!    (divisors of random bit length, so every row's quotient bit varies)
//!    and takes, for each row value, the shallowest signal of each side that
//!    computes it or its complement: a pair. A value a gate of both sides
//!    computes needs no pair;
//! 4. proves the pairs in row order (rows top down; in a row the quotient
//!    or root bit, then the remainder bits from the lowest), each by a
//!    sub-call of the full solver (variable addition, pre- and inprocessing
//!    off) on the circuit, the pairs proven so far and "the two signals
//!    differ". A satisfiable sub-call's model is an input on which the
//!    circuit is simulated: every pending pair that differs there is
//!    dropped. With a proof file a fresh d = x xor y is defined by four red
//!    steps, the sub-call's clauses are the rup steps of a pbc that gives
//!    ~d, and (~x | y) and (x | ~y) follow by rup;
//! 5. adds the proven pairs' clauses to F'. The search then sees the miter
//!    with them (on the 2025 cells it needs no conflicts).

use crate::structproof::Writer;
use crate::structure::{Derived, Formula};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_SUPPORT: usize = 6; // a gate's truth table fits in a u64
const MAX_VARS: i32 = 1 << 21; // beyond this the pass does not read
const MAX_INPUTS: usize = 512;
const MAX_WIDTH: usize = 64; // divider operands of at most 64 bits (the rows fit in u128)
const MAX_SQRT_INPUTS: usize = 124; // radicand bits (the rows fit in u128)
const SAMPLES: usize = 2048; // random inputs for the candidates
const WORDS: usize = SAMPLES / 64;
const CHECK_SAMPLES: usize = 1024; // random inputs for the function check
// The sub-call's conflict limit per pair. On div_miter_lec__2 every row pair
// but one takes at most 95K conflicts (all but five under 20K); the compared
// bit's own pair is the miter itself, and the search (every technique on,
// with the proven pairs) finishes it in 630K conflicts where a plain sub-call
// does not in 4M. On the four smaller cells the compared bit takes 44K-67K.
const PAIR_CONFLICTS: u32 = 250_000;

fn debug() -> bool {
    std::env::var_os("SAT_STRUCT_DEBUG").is_some()
}

/// Work in one piece: charged when the limit covers it, else Err.
fn reserve(work: &mut u64, cost: u64, stopped: &dyn Fn(u64) -> bool) -> Result<(), String> {
    if stopped(work.saturating_add(cost)) {
        return Err("budget".into());
    }
    *work += cost;
    Ok(())
}

// ---------------------------------------------------------------------------
// reading the circuit
// ---------------------------------------------------------------------------

/// The function `tt` of `sup` (sorted variables; minterm m: bit i of m is
/// the value of sup[i]).
#[derive(Clone)]
struct Gate {
    sup: Vec<i32>,
    tt: u64,
}

struct Net {
    nv: usize,
    gates: Vec<Option<Gate>>,
    /// The gates, every one after its inputs.
    order: Vec<i32>,
    /// 1 true, -1 false, 0 no constant.
    constant: Vec<i8>,
    inputs: Vec<i32>,
    /// The clause ids of the root's unit.
    root_units: Vec<usize>,
    /// The two side signals, and whether the root asserts a == b (so a
    /// refutation shows a is b's complement).
    a: i32,
    b: i32,
    invert: bool,
    level: Vec<u32>,
    /// 1 in a's cone, 2 in b's, 3 in both.
    side: Vec<u8>,
}

impl Net {
    fn shared(&self, v: i32) -> bool {
        self.side[v as usize] == 3
    }
}

/// Truth table of v over `support` if the clauses determine v for every
/// assignment of the support, else None.
fn definition_tt(v: i32, ids: &[u32], f: &Formula, support: &[i32], work: &mut u64) -> Option<u64> {
    let n = support.len();
    let mut pos: Vec<Vec<(u8, bool)>> = Vec::new();
    let mut neg: Vec<Vec<(u8, bool)>> = Vec::new();
    for &ci in ids {
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

fn read_net(f: &Formula, work: &mut u64, stopped: &dyn Fn(u64) -> bool) -> Result<Net, String> {
    let nv = f.vars as usize + 1;
    // the grouping by first literal: three tables per variable, one slot per
    // clause, a visit per literal and clause
    reserve(work, 3 * nv as u64 + 2 * f.len() as u64 + f.lits.len() as u64, stopped)?;
    let mut count = vec![0u32; nv + 1];
    let mut present = vec![false; nv];
    let mut units: Vec<(usize, i32)> = Vec::new();
    for i in 0..f.len() {
        if f.taut[i] {
            continue;
        }
        let c = f.clause(i);
        if c.is_empty() {
            return Err("the empty clause".into());
        }
        for &l in c {
            present[l.unsigned_abs() as usize] = true;
        }
        if c.len() == 1 {
            units.push((i, c[0]));
        } else {
            count[c[0].unsigned_abs() as usize] += 1;
        }
    }
    let mut first = vec![0u32; nv + 1];
    for v in 0..nv {
        first[v + 1] = first[v] + count[v];
    }
    let mut pos = first.clone();
    let mut ids = vec![0u32; first[nv] as usize];
    for i in 0..f.len() {
        if f.taut[i] || f.clause(i).len() < 2 {
            continue;
        }
        let v = f.clause(i)[0].unsigned_abs() as usize;
        ids[pos[v] as usize] = i as u32;
        pos[v] += 1;
    }
    // every group a definition
    let mut gates: Vec<Option<Gate>> = vec![None; nv];
    for v in 1..nv {
        let group = &ids[first[v] as usize..first[v + 1] as usize];
        if group.is_empty() {
            continue;
        }
        let size: u64 = group.iter().map(|&ci| f.clause(ci as usize).len() as u64 + 1).sum();
        reserve(work, size, stopped)?;
        let mut sup: Vec<i32> = Vec::new();
        for &ci in group {
            for &l in f.clause(ci as usize) {
                if l.unsigned_abs() as usize != v {
                    sup.push(l.abs());
                }
            }
        }
        sup.sort_unstable();
        sup.dedup();
        if sup.is_empty() || sup.len() > MAX_SUPPORT {
            return Err(format!("the clauses of {} read {} inputs", v, sup.len()));
        }
        reserve(work, (1u64 << sup.len()) * (group.len() as u64 + 1), stopped)?;
        let tt = definition_tt(v as i32, group, f, &sup, work).ok_or_else(|| format!("the clauses of {} do not define it", v))?;
        gates[v] = Some(Gate { sup, tt });
    }
    // units: the root on a gate, constants elsewhere
    let mut constant = vec![0i8; nv];
    let mut root: Option<i32> = None;
    let mut root_units: Vec<usize> = Vec::new();
    for &(i, l) in &units {
        let v = l.unsigned_abs() as usize;
        if gates[v].is_some() {
            match root {
                None => root = Some(l),
                Some(r) if r == l => {}
                Some(_) => return Err("a second unit on a gate".into()),
            }
            root_units.push(i);
        } else {
            let b: i8 = if l > 0 { 1 } else { -1 };
            if constant[v] == -b {
                return Err("two opposite units".into());
            }
            constant[v] = b;
        }
    }
    let root = root.ok_or("no unit on a gate")?;
    // the order, every gate after its inputs; a cycle ends the reading
    reserve(work, 4 * nv as u64, stopped)?;
    let mut order: Vec<i32> = Vec::new();
    let mut state = vec![0u8; nv]; // 1 open, 2 done
    for r in 1..nv {
        if gates[r].is_none() || state[r] != 0 {
            continue;
        }
        let mut stack: Vec<(i32, bool)> = vec![(r as i32, false)];
        while let Some((v, done)) = stack.pop() {
            let vi = v as usize;
            if done {
                state[vi] = 2;
                order.push(v);
                continue;
            }
            if state[vi] == 2 {
                continue;
            }
            if state[vi] == 1 {
                return Err(format!("a cycle through {}", v));
            }
            state[vi] = 1;
            stack.push((v, true));
            for &u in &gates[vi].as_ref().unwrap().sup {
                let ui = u as usize;
                if gates[ui].is_some() {
                    if state[ui] == 1 {
                        return Err(format!("a cycle through {}", u));
                    }
                    if state[ui] == 0 {
                        stack.push((u, false));
                    }
                }
            }
        }
        *work += 1;
        if stopped(*work) {
            return Err("budget".into());
        }
    }
    let inputs: Vec<i32> = (1..nv as i32).filter(|&v| present[v as usize] && gates[v as usize].is_none() && constant[v as usize] == 0).collect();
    if inputs.is_empty() || inputs.len() > MAX_INPUTS {
        return Err(format!("{} inputs", inputs.len()));
    }
    let rg = gates[root.unsigned_abs() as usize].as_ref().unwrap();
    if rg.sup.len() != 2 || (rg.tt != 0b0110 && rg.tt != 0b1001) {
        return Err("the root is no xor of two signals".into());
    }
    let (a, b) = (rg.sup[0], rg.sup[1]);
    let invert = (rg.tt == 0b1001) == (root > 0); // an xnor asserted, or an xor denied: a == b asserted
    let mut level = vec![0u32; nv];
    for &v in &order {
        let g = gates[v as usize].as_ref().unwrap();
        level[v as usize] = 1 + g.sup.iter().map(|&u| level[u as usize]).max().unwrap_or(0);
    }
    let mut side = vec![0u8; nv];
    for (bit, start) in [(1u8, a), (2u8, b)] {
        let mut stack = vec![start];
        while let Some(v) = stack.pop() {
            let vi = v as usize;
            if side[vi] & bit != 0 {
                continue;
            }
            side[vi] |= bit;
            if let Some(g) = &gates[vi] {
                stack.extend(g.sup.iter().copied());
            }
        }
    }
    *work += 2 * nv as u64;
    Ok(Net { nv, gates, order, constant, inputs, root_units, a, b, invert, level, side })
}

// ---------------------------------------------------------------------------
// simulation
// ---------------------------------------------------------------------------

/// Every signal's value on `words * 64` input patterns; `set(v, out)`
/// writes input v's words.
fn simulate(net: &Net, words: usize, set: &dyn Fn(i32, &mut [u64]), work: &mut u64) -> Vec<u64> {
    let mut val = vec![0u64; net.nv * words];
    for &v in &net.inputs {
        set(v, &mut val[v as usize * words..(v as usize + 1) * words]);
    }
    for v in 1..net.nv {
        if net.constant[v] == 1 {
            for w in 0..words {
                val[v * words + w] = u64::MAX;
            }
        }
    }
    for &v in &net.order {
        let g = net.gates[v as usize].as_ref().unwrap();
        *work += (words as u64) << g.sup.len();
        for w in 0..words {
            let mut out = 0u64;
            for m in 0..(1u32 << g.sup.len()) {
                if (g.tt >> m) & 1 == 1 {
                    let mut term = u64::MAX;
                    for (i, &u) in g.sup.iter().enumerate() {
                        let x = val[u as usize * words + w];
                        term &= if (m >> i) & 1 == 1 { x } else { !x };
                    }
                    out |= term;
                }
            }
            val[v as usize * words + w] = out;
        }
    }
    val
}

fn sim_cost(net: &Net, words: usize) -> u64 {
    net.order.iter().map(|&v| (words as u64) << net.gates[v as usize].as_ref().unwrap().sup.len()).sum::<u64>() + (net.nv * words) as u64
}

/// splitmix64: the pass's own generator, seeded the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// A random number of `bits` bits (at most 128).
    fn bits(&mut self, bits: usize) -> u128 {
        let x = ((self.next() as u128) << 64) | self.next() as u128;
        if bits >= 128 {
            x
        } else {
            x & ((1u128 << bits) - 1)
        }
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn isqrt(x: u128) -> u128 {
    // the bit-by-bit method: exact for every u128
    let mut rem = x;
    let mut root: u128 = 0;
    let mut bit: u128 = 1u128 << 126;
    while bit > x {
        bit >>= 2;
    }
    while bit != 0 {
        if rem >= root + bit {
            rem -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    root
}

/// Bit indices lo..hi in the text order of their names (`b[0]`, `b[10]`,
/// ..., `b[19]`, `b[1]`, `b[20]`, ...).
fn text_order(lo: usize, hi: usize) -> Vec<usize> {
    let mut v: Vec<usize> = (lo..hi).collect();
    v.sort_by_key(|&i| format!("{}]", i));
    v
}

// ---------------------------------------------------------------------------
// the function
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Func {
    /// bit k of floor(a / b) for width-bit a and b; (input, bit) per operand
    Div { width: usize, k: usize, a: Vec<(i32, usize)>, b: Vec<(i32, usize)> },
    /// the low bit of isqrt(x) for the inputs read as one n-bit number
    Sqrt { n: usize, x: Vec<(i32, usize)> },
}

/// Samples as (a, b) for a divider, (x, 0) for a square root.
fn samples(func: &Func, count: usize, rng: &mut Rng) -> Vec<(u128, u128)> {
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        match func {
            Func::Div { width, k, .. } => {
                // mostly divisors of a random bit length up to width-k, so that
                // every row's quotient bit is 1 on some samples, 0 on others
                let small = width - k;
                let r = rng.below(100);
                let b = if r < 5 {
                    0
                } else if r < 15 || small == 0 {
                    rng.bits(*width)
                } else {
                    let len = 1 + rng.below(small as u64) as usize;
                    (1u128 << (len - 1)) | rng.bits(len - 1)
                };
                let a = rng.bits(*width) & !((1u128 << k) - 1);
                out.push((a, b));
            }
            Func::Sqrt { n, .. } => out.push((rng.bits(*n), 0)),
        }
    }
    out
}

/// The inputs' words for the samples.
fn input_words(func: &Func, samples: &[(u128, u128)]) -> HashMap<i32, Vec<u64>> {
    let words = samples.len().div_ceil(64);
    let mut m: HashMap<i32, Vec<u64>> = HashMap::new();
    let mut put = |v: i32, s: usize, on: bool| {
        let e = m.entry(v).or_insert_with(|| vec![0u64; words]);
        if on {
            e[s / 64] |= 1u64 << (s % 64);
        }
    };
    for (s, &(x, y)) in samples.iter().enumerate() {
        match func {
            Func::Div { a, b, .. } => {
                for &(v, i) in a {
                    put(v, s, (x >> i) & 1 == 1);
                }
                for &(v, i) in b {
                    put(v, s, (y >> i) & 1 == 1);
                }
            }
            Func::Sqrt { x: bits, .. } => {
                for &(v, i) in bits {
                    put(v, s, (x >> i) & 1 == 1);
                }
            }
        }
    }
    m
}

/// The compared bit on a sample.
fn output(func: &Func, (x, y): (u128, u128)) -> bool {
    match func {
        Func::Div { width, k, .. } => {
            let q = if y == 0 { (1u128 << width) - 1 } else { x / y };
            (q >> k) & 1 == 1
        }
        Func::Sqrt { .. } => isqrt(x) & 1 == 1,
    }
}

/// Simulate the samples and compare signal `sig` with the function.
/// Some(negated) when it equals the function or its complement.
fn check(net: &Net, func: &Func, sig: i32, rng: &mut Rng, work: &mut u64, stopped: &dyn Fn(u64) -> bool) -> Result<Option<bool>, String> {
    let words = CHECK_SAMPLES / 64;
    reserve(work, sim_cost(net, words) + (CHECK_SAMPLES as u64) * 4, stopped)?;
    let s = samples(func, CHECK_SAMPLES, rng);
    let iw = input_words(func, &s);
    let val = simulate(net, words, &|v, out| {
        if let Some(w) = iw.get(&v) {
            out.copy_from_slice(w);
        }
    }, &mut 0);
    let mut want = vec![0u64; words];
    for (i, &smp) in s.iter().enumerate() {
        if output(func, smp) {
            want[i / 64] |= 1u64 << (i % 64);
        }
    }
    let got = &val[sig as usize * words..(sig as usize + 1) * words];
    if got == &want[..] {
        return Ok(Some(false));
    }
    if got.iter().zip(&want).all(|(g, w)| *g == !w) {
        return Ok(Some(true));
    }
    Ok(None)
}

fn recognize(net: &Net, rng: &mut Rng, work: &mut u64, stopped: &dyn Fn(u64) -> bool) -> Result<(Func, bool), String> {
    let n = net.inputs.len();
    // one input set at a time (pattern 0: none)
    let words = (n + 1).div_ceil(64);
    reserve(work, sim_cost(net, words), stopped)?;
    let idx: HashMap<i32, usize> = net.inputs.iter().enumerate().map(|(i, &v)| (v, i)).collect();
    let val = simulate(net, words, &|v, out| {
        let p = 1 + idx[&v];
        out[p / 64] |= 1u64 << (p % 64);
    }, &mut 0);
    let bit = |p: usize| (val[net.a as usize * words + p / 64] >> (p % 64)) & 1 == 1;
    let zero = bit(0);
    let a_like: Vec<i32> = net.inputs.iter().enumerate().filter(|&(i, _)| bit(1 + i) == zero).map(|(_, &v)| v).collect();
    let b_like: Vec<i32> = net.inputs.iter().enumerate().filter(|&(i, _)| bit(1 + i) != zero).map(|(_, &v)| v).collect();
    let width = b_like.len();
    if (2..=MAX_WIDTH).contains(&width) && !a_like.is_empty() && a_like.len() <= width {
        let k = width - a_like.len();
        let orders: [(Vec<usize>, Vec<usize>); 2] = [
            ((0..width).collect(), (k..width).collect()),
            (text_order(0, width), text_order(k, width)),
        ];
        for (bo, ao) in orders {
            let func = Func::Div {
                width,
                k,
                a: a_like.iter().copied().zip(ao.iter().copied()).collect(),
                b: b_like.iter().copied().zip(bo.iter().copied()).collect(),
            };
            if let Some(neg) = check(net, &func, net.a, rng, work, stopped)? {
                return Ok((func, neg));
            }
        }
    }
    if n % 2 == 0 && (2..=MAX_SQRT_INPUTS).contains(&n) {
        let mut orders: Vec<Vec<usize>> = vec![(0..n).collect()];
        if n <= 128 {
            // the text order of a[lo..127], as for the 128-bit EPFL square root
            let lo = 128 - n;
            orders.push(text_order(lo, 128).into_iter().map(|i| i - lo).collect());
        }
        for o in orders {
            let func = Func::Sqrt { n, x: net.inputs.iter().copied().zip(o.iter().copied()).collect() };
            if let Some(neg) = check(net, &func, net.a, rng, work, stopped)? {
                return Ok((func, neg));
            }
        }
    }
    Err("neither a divider nor a square root".into())
}

// ---------------------------------------------------------------------------
// the candidates
// ---------------------------------------------------------------------------

struct Pair {
    name: String,
    x: i32,
    y: i32,
}

/// The row values in chain order, as (name, per-sample bits).
fn row_values(func: &Func, s: &[(u128, u128)]) -> Vec<(String, Vec<u64>)> {
    let words = s.len().div_ceil(64);
    let mut out: Vec<(String, Vec<u64>)> = Vec::new();
    for (smp, &(x, y)) in s.iter().enumerate() {
        // every sample yields the same values in the same order: the first
        // one names them
        let mut t = 0usize;
        let mut set = |name: &dyn Fn() -> String, on: bool| {
            if smp == 0 {
                out.push((name(), vec![0u64; words]));
            }
            if on {
                out[t].1[smp / 64] |= 1u64 << (smp % 64);
            }
            t += 1;
        };
        match func {
            Func::Div { width, k, .. } => {
                let mut r: u128 = 0;
                for j in (*k..*width).rev() {
                    let t = 2 * r + ((x >> j) & 1);
                    let q = t >= y;
                    r = if q { t - y } else { t };
                    set(&|| format!("q{}", j), q);
                    for i in 0..=(width - j) {
                        set(&|| format!("R{}.{}", j, i), (r >> i) & 1 == 1);
                    }
                }
            }
            Func::Sqrt { n, .. } => {
                let steps = n / 2;
                for i in (0..steps).rev() {
                    let part = x >> (2 * i);
                    let q = isqrt(part);
                    let r = part - q * q;
                    set(&|| format!("r{}", i), q & 1 == 1);
                    for b in 0..(steps - i + 2) {
                        set(&|| format!("R{}.{}", i, b), (r >> b) & 1 == 1);
                    }
                }
            }
        }
    }
    out
}

/// Per side, the canonical signature (the one with the first sample 0) to
/// the gates computing it, as (shared, level, literal): the literal is
/// negative when the gate computes the complement.
fn candidates(net: &Net, func: &Func, rng: &mut Rng, work: &mut u64, stopped: &dyn Fn(u64) -> bool) -> Result<Vec<Pair>, String> {
    reserve(work, sim_cost(net, WORDS) + 3 * (net.nv * WORDS) as u64 + (SAMPLES as u64) * 256, stopped)?;
    let s = samples(func, SAMPLES, rng);
    let iw = input_words(func, &s);
    let val = simulate(net, WORDS, &|v, out| {
        if let Some(w) = iw.get(&v) {
            out.copy_from_slice(w);
        }
    }, &mut 0);
    let sig = |v: i32| &val[v as usize * WORDS..(v as usize + 1) * WORDS];
    let canonical = |w: &[u64]| -> (Vec<u64>, bool) {
        if w[0] & 1 == 1 {
            (w.iter().map(|x| !x).collect(), true)
        } else {
            (w.to_vec(), false)
        }
    };
    // the sides agree on every sample, or the miter is no theorem the pass
    // could prove
    let (sa, sb) = (sig(net.a), sig(net.b));
    if !sa.iter().zip(sb).all(|(x, y)| if net.invert { *x == !y } else { x == y }) {
        return Err("the sides differ on a sample".into());
    }
    let mut index: [HashMap<Vec<u64>, (bool, u32, i32)>; 2] = [HashMap::new(), HashMap::new()];
    let mut by_shared: std::collections::HashSet<Vec<u64>> = std::collections::HashSet::new();
    for v in 1..net.nv as i32 {
        if net.gates[v as usize].is_none() {
            continue;
        }
        let side = net.side[v as usize];
        let (key, neg) = canonical(sig(v));
        if net.shared(v) {
            by_shared.insert(key.clone());
        }
        let entry = (net.shared(v), net.level[v as usize], if neg { -v } else { v });
        for (k, bit) in [(0usize, 1u8), (1usize, 2u8)] {
            if side & bit != 0 {
                let e = index[k].entry(key.clone()).or_insert(entry);
                // a side's own gates first, then the shallowest, then the lowest number
                if (entry.0, entry.1, entry.2.abs()) < (e.0, e.1, e.2.abs()) {
                    *e = entry;
                }
            }
        }
    }
    let mut pairs: Vec<Pair> = Vec::new();
    for (name, w) in row_values(func, &s) {
        if w.iter().all(|&x| x == 0) || w.iter().all(|&x| x == u64::MAX) {
            continue; // constant on the samples: no candidate
        }
        let (key, neg) = canonical(&w);
        if by_shared.contains(&key) {
            continue; // a gate both sides share computes it: nothing to prove
        }
        let (Some(ea), Some(eb)) = (index[0].get(&key), index[1].get(&key)) else { continue };
        let (x, y) = if neg { (-ea.2, -eb.2) } else { (ea.2, eb.2) };
        pairs.push(Pair { name, x, y });
    }
    Ok(pairs)
}

// ---------------------------------------------------------------------------
// the chain
// ---------------------------------------------------------------------------

enum Verdict {
    /// The sub-call's clauses, each implied by unit propagation from the
    /// circuit, the proven pairs, "x and y differ" and the ones before it;
    /// the last one is the empty clause (only with `capture`).
    Refuted(Vec<Vec<i32>>),
    /// The inputs a model sets true.
    Sat(Vec<i32>),
    Unknown,
}

#[allow(clippy::too_many_arguments)]
fn sub_refute(f: &Formula, net: &Net, eqs: &[[i32; 2]], x: i32, y: i32, capture: bool, ticks: u64, work: &mut u64, stop: &AtomicBool) -> Verdict {
    if ticks == 0 || stop.load(Ordering::Relaxed) {
        return Verdict::Unknown;
    }
    let mut sub = Box::new(crate::internal::init());
    crate::config::set_configuration(&mut sub.options, "plain");
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
        ("structdiv", 0),
        ("gauss", 0),
    ] {
        crate::internal::set_option(&mut sub, name, value);
    }
    crate::internal::set_conflict_limit(&mut sub, PAIR_CONFLICTS);
    crate::internal::set_ticks_limit(&mut sub, ticks);
    if capture {
        crate::proof::init_captured_proof(&mut sub);
    }
    // (the parent's alarm stops it; its work reaches the parent's tick
    // counter as it goes)
    sub.parent_stop = stop as *const AtomicBool as usize;
    sub.parent_ticks_base = *work;
    sub.parent_ticks = work as *mut u64 as usize;
    for i in 0..f.len() {
        if f.taut[i] || net.root_units.contains(&i) {
            continue;
        }
        for &l in f.clause(i) {
            crate::internal::add(&mut sub, l);
        }
        crate::internal::add(&mut sub, 0);
    }
    for e in eqs {
        for &l in e {
            crate::internal::add(&mut sub, l);
        }
        crate::internal::add(&mut sub, 0);
    }
    for c in [[x, y], [-x, -y]] {
        for &l in &c {
            crate::internal::add(&mut sub, l);
        }
        crate::internal::add(&mut sub, 0);
    }
    let res = crate::internal::solve(&mut sub);
    *work = sub.parent_ticks_base.saturating_add(sub.statistics.work_clock());
    if debug() && sub.statistics.conflicts >= 20_000 {
        eprintln!("structdiv: a sub-call of {} conflicts, work {}K, result {}", sub.statistics.conflicts, sub.statistics.work_clock() / 1000, res);
    }
    let verdict = match res {
        20 => {
            let mut steps: Vec<Vec<i32>> = Vec::new();
            if capture {
                for c in crate::proof::take_captured_proof(&mut sub) {
                    if !c.is_empty() {
                        *work += c.len() as u64 + 1;
                        steps.push(c);
                    }
                }
                steps.push(Vec::new());
            }
            Verdict::Refuted(steps)
        }
        10 => {
            let mut on: Vec<i32> = Vec::new();
            for &v in &net.inputs {
                if crate::internal::value(&mut sub, v) > 0 {
                    on.push(v);
                }
            }
            Verdict::Sat(on)
        }
        _ => Verdict::Unknown,
    };
    crate::internal::release(*sub);
    verdict
}

struct Outcome {
    /// (clause, proof id) per proven pair half, in order.
    clauses: Vec<([i32; 2], u64)>,
    proven: usize,
    dropped: usize,
    by_model: usize,
}

#[allow(clippy::too_many_arguments)]
fn chain(f: &Formula, net: &Net, pairs: &[Pair], mut proof: Option<&mut Writer>, next_var: &mut i32, budget: u64, work: &mut u64, stop: &AtomicBool) -> Result<Outcome, String> {
    let stopped = |w: u64| w >= budget || stop.load(Ordering::Relaxed);
    let dbg = debug();
    let mut alive = vec![true; pairs.len()];
    let mut done = vec![false; pairs.len()];
    let mut eqs: Vec<[i32; 2]> = Vec::new();
    let mut out = Outcome { clauses: Vec::new(), proven: 0, dropped: 0, by_model: 0 };
    let size: u64 = f.lits.len() as u64 + f.len() as u64;
    for i in 0..pairs.len() {
        if !alive[i] {
            continue;
        }
        // the sub-call's import of the circuit and the pairs (the budget's end
        // keeps what was proven)
        if reserve(work, size + 3 * eqs.len() as u64 + 6, &stopped).is_err() {
            break;
        }
        let (x, y) = (pairs[i].x, pairs[i].y);
        let before = *work;
        let left = budget.saturating_sub(*work);
        let verdict = sub_refute(f, net, &eqs, x, y, proof.is_some(), left, work, stop);
        match verdict {
            Verdict::Refuted(steps) => {
                if let Some(w) = proof.as_deref_mut() {
                    // the definition of d = x xor y, ~d by contradiction with
                    // the sub-call's clauses, then the pair's two clauses;
                    // written whole or not at all (a pbc block cannot stop
                    // halfway), so its cost is reserved first
                    let block: u64 = steps.iter().map(|c| c.len() as u64 + 1).sum::<u64>() + 32;
                    if reserve(work, block, &stopped).is_err() {
                        break;
                    }
                    *next_var += 1;
                    let d = *next_var;
                    let write = |w: &mut Writer| -> std::io::Result<(u64, u64)> {
                        w.red_const(&[-d, x, y], d, false)?;
                        w.red_const(&[-d, -x, -y], d, false)?;
                        w.red_const(&[d, -x, y], d, true)?;
                        w.red_const(&[d, x, -y], d, true)?;
                        w.pbc(&[(1, -d)], 1, &steps)?;
                        Ok((w.rup(&[-x, y])?, w.rup(&[x, -y])?))
                    };
                    let (id1, id2) = write(w).map_err(|e| e.to_string())?;
                    out.clauses.push(([-x, y], id1));
                    out.clauses.push(([x, -y], id2));
                } else {
                    out.clauses.push(([-x, y], 0));
                    out.clauses.push(([x, -y], 0));
                }
                eqs.push([-x, y]);
                eqs.push([x, -y]);
                done[i] = true;
                out.proven += 1;
                if dbg && (out.proven % 100 == 0 || i + 1 == pairs.len()) {
                    eprintln!("structdiv: {} pairs proven ({}), work {}K", out.proven, pairs[i].name, *work / 1000);
                }
            }
            Verdict::Sat(on) => {
                alive[i] = false;
                out.dropped += 1;
                // the model is an input on which pair i differs: simulate it,
                // and every pending pair that differs there goes too
                if reserve(work, sim_cost(net, 1) + pairs.len() as u64, &stopped).is_err() {
                    break;
                }
                let val = simulate(net, 1, &|v, w| w[0] = if on.binary_search(&v).is_ok() { 1 } else { 0 }, &mut 0);
                let lit = |l: i32| (val[l.unsigned_abs() as usize] & 1 == 1) != (l < 0);
                for j in 0..pairs.len() {
                    if alive[j] && lit(pairs[j].x) != lit(pairs[j].y) {
                        if done[j] {
                            return Err(format!("proven pair {} differs on a model", pairs[j].name));
                        }
                        alive[j] = false;
                        if j != i {
                            out.by_model += 1;
                        }
                    }
                }
                if dbg {
                    eprintln!("structdiv: {} is no equivalence (work {}K)", pairs[i].name, (*work - before) / 1000);
                }
            }
            Verdict::Unknown => {
                alive[i] = false;
                out.dropped += 1;
                if dbg {
                    eprintln!("structdiv: {}: no verdict within the limits (work {}K)", pairs[i].name, (*work - before) / 1000);
                }
                if stopped(*work) {
                    break;
                }
            }
        }
    }
    Ok(out)
}

pub fn run(f: &Formula, derived: &mut Derived, budget: u64, _hard: u64, work: &mut u64, stop: &AtomicBool) {
    let stopped = |w: u64| w >= budget || stop.load(Ordering::Relaxed);
    if stopped(*work) || f.vars > MAX_VARS || derived.refuted {
        return;
    }
    let start = *work;
    let dbg = debug();
    let net = match read_net(f, work, &stopped) {
        Ok(n) => n,
        Err(why) => {
            if dbg {
                eprintln!("structdiv: {}", why);
            }
            return;
        }
    };
    let mut rng = Rng(0x5eed_d1f5);
    let (func, negated) = match recognize(&net, &mut rng, work, &stopped) {
        Ok(r) => r,
        Err(why) => {
            if dbg {
                eprintln!("structdiv: {}", why);
            }
            return;
        }
    };
    let what = match &func {
        Func::Div { width, k, .. } => format!("quotient bit {} of a {}/{} divider", k, width, width),
        Func::Sqrt { n, .. } => format!("the low root bit of a {}-bit square root", n),
    };
    let pairs = match candidates(&net, &func, &mut rng, work, &stopped) {
        Ok(p) => p,
        Err(why) => {
            if dbg {
                eprintln!("structdiv: {}: {}", what, why);
            }
            return;
        }
    };
    if dbg {
        eprintln!("structdiv: {}{}, {} inputs, {} gates, {} pairs", what, if negated { " (negated)" } else { "" }, net.inputs.len(), net.order.len(), pairs.len());
    }
    if pairs.is_empty() {
        return;
    }
    // the fresh variables (one d per pair) come after every number used so
    // far, and the counter goes back to `derived` on every path: a later
    // pass must not reuse a number this pass's proof holds
    let mut next_var: i32 = derived.last_var.max(f.vars);
    let outcome = chain(f, &net, &pairs, derived.proof.as_mut(), &mut next_var, budget, work, stop);
    derived.last_var = next_var;
    let o = match outcome {
        Ok(o) => o,
        Err(why) => {
            if dbg {
                eprintln!("structdiv: the chain stopped: {}", why);
            }
            return;
        }
    };
    for (c, id) in &o.clauses {
        derived.add(c, *id);
    }
    derived.notes.push(format!(
        "div/sqrt: {}, {} of {} row pairs proven equal ({} dropped by a check, {} by a model), work {}K",
        what,
        o.proven,
        pairs.len(),
        o.dropped,
        o.by_model,
        (*work - start) / 1000
    ));
}
