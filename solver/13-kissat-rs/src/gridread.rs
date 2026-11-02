//! Not in kissat. Reading alldifferent grids from the input clauses, with their
//! proofs (epic SAT-playground-iin, bead iin.8; plan/count-and-hall-lemmas-plan.md).
//!
//! A row wants one of its literals. A column is a set of literals that exclude
//! each other, so at most one of them holds. The reader goes by what the clauses
//! do under unit propagation, so any encoding of the exclusions reads alike.
//! `tools/alldiff_census.py` is the reference for what to read and
//! `tools/hall_toy_proof.py` for the proof lines.
//!
//! What is read, as in the census. A row is a clause of three or more literals,
//! or a resolvent: a literal g of a clause whose negation sits in exactly one
//! clause C is replaced by C's other literals, down to depth three. A clause that
//! root units shortened, or that repeats a literal, reads as a row over what is left.
//! Those rows follow from the input by propagation and have no clause of their own.
//! A row needs two literals that propagate something alone.
//!
//! Propagating a literal says which literals of the rows it excludes. What a row's
//! own literals do to each other says what it is: exact-one (all exclude), plain
//! (next to none) or guarded (all but one). Those are the census's three. A gate's
//! clause never reads as a row. It is a clause where the exclusions run through one
//! literal, or one literal excludes all the others.
//!
//! A grid is a set of rows grown around one row: each differs from that row (two
//! or more literals on each side exclude, at most three exclusions a literal on
//! average) and shares an exclusion with at least four in five of the others. The
//! columns are then a clique cover of the rows' literals, with the row with the
//! fewest choices next and, on a tie, the literal that keeps the most choices open.
//! A column of one literal is a value only one row has: it is not a column, the
//! literal just stays in its row. A literal and its negation in two rows exclude each
//! other like any other pair, and such a column is what makes some grids tight.
//!
//! What is read beyond the census:
//! - a partial row: some exclusions inside, in no gate's pattern. These rows are read
//!   last, with what is left of the budget, so they cost the others nothing;
//! - a whole split clause, however long the chain: what is left when every variable
//!   that sits in one clause each way is resolved away (`add_split_rows`);
//! - tighter columns where side constraints compete with them: at the first step of
//!   a clique the literal that gives the largest clique; a literal with two largest
//!   cliques waits, so that a diagonal as long as the columns does not break them up;
//!   and a clique goes when all of its literals fit other cliques (`absorb`).
//!
//! Detection may guess, the check that follows may not. Reading samples rows by a
//! fixed seed and works from what it has read so far. Then `check_grid` looks at
//! each grid it is about to return, against the input alone: each row is an input
//! clause as written or follows by propagation, and each pair of a column is an input
//! clause or the raw list of what one literal's propagation derived holds the other's
//! negation. So the lines `prove_row` and `prove_col` write cannot be rejected: a
//! `rup` line is what the check saw propagation do, and the `pol` line is arithmetic
//! on such pairs. A literal that fails alone is false at the root: it leaves its row
//! and its column.
//!
//! Budgets. A tick is about one cache line, as kissat's are. A whole piece of work
//! (a table, a scan, a clause read to its end) is done only when the limit covers it.
//! Work is charged to `ticks` as it is done. Nothing here iterates a hash table and
//! the only random numbers come from a fixed seed, so two runs charge the same ticks
//! and read the same grids.
//!
//! Where the census's budget went on a formula full of gates was propagation: a
//! literal of a gate can force thousands. So reading starts with a slice of short
//! reads, which stop a literal's propagation after SHORT_READ literals and keep the
//! near exclusions, and goes on in full from the largest grids found (`run`).
//! Next came the scan of a row's neighbours: rows not read yet count as neighbours,
//! because a row is read when a grid comes to it, and on a formula of long clauses
//! they are most of what the scan walks. A scan that cannot end within the row's share
//! is not begun. It looks at the rows read so far, and the row is tried again later.

use crate::structproof::Writer;
use crate::structure::Formula;
use std::cmp::Reverse;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A two-literal row is one not-equal, not a domain.
const MIN_ROW: usize = 3;
/// A longer resolved row is not built.
const MAX_ROW: usize = 5_000;
/// Depth of the resolved rows.
const MAX_DEPTH: u8 = 3;
/// Literals visited building resolved rows, per formula.
const ROW_STEPS: u64 = 5_000_000;
/// A literal's read stops past this many literals and keeps what it has.
const HUB: usize = 50_000;
/// The same, in a short slice of the reading.
const SHORT_READ: usize = 2_000;
/// A slice of short reads is this part of the detection budget.
const SHORT_SLICES: u64 = 4;
/// No new turn of classifying and linking in the last 1/16 of a pass's budget.
const LAST_TURN: u64 = 16;
/// Cliques of rows grown per row read.
const MAX_CLIQUES: usize = 6;
/// At least this part of the detection budget is one row's to spend: 1/50.
const ROW_SHARE: u64 = 50;
/// Rows in a row that may not fit before a clique stops trying, strongest first.
const MAX_FAILS: u32 = 64;
/// Rounds of making a clique cover smaller.
const MAX_ABSORB: u32 = 8;
/// Exclusion pairs cached, each counted both ways; past this the cache starts over,
/// which the default budget does not reach by much. About 400 MB at the most.
const MAX_ENTRIES: u64 = 100_000_000;
/// Larger grids a grid is compared with to see whether it is a part of one.
const MAX_PART_TESTS: usize = 200;
/// Grids returned, biggest first. The census's core grids stay under this on 391
/// of the 400 cells of sat-comp-2025. What a cell past it loses is its smallest
/// grids: the two of those with big grids at all are spg_300_300 and SCPC-500-1,
/// where thousands of grids hold three to seven rows.
const MAX_GRIDS: usize = 5_000;
/// The part of the budget left after setup that stays for the check, in percent,
/// when detection does not finish earlier.
const CHECK_SHARE: u64 = 10;

const NONE: u32 = u32::MAX;

/// Work is counted in sixteenths of a tick.
const TICK: u64 = 16;

/// A table made or filled in order: 16 entries of four bytes are one cache line.
#[inline]
fn items(n: usize) -> u64 {
    n as u64
}

/// A list read in order, with a look at a table for each item: the table may be out
/// of the cache line in hand, the next item of the list is not.
#[inline]
fn looks(n: usize) -> u64 {
    2 * n as u64
}

/// A sort: a comparison stays in the cache.
#[inline]
fn sorting(n: usize) -> u64 {
    let n = n as u64;
    n * (64 - n.leading_zeros() as u64)
}

// What a row's literals do to each other.
const SHAPE_UNKNOWN: u8 = 0;
const SHAPE_EXACT: u8 = 1;
const SHAPE_PLAIN: u8 = 2;
const SHAPE_GUARDED: u8 = 3;
const SHAPE_PARTIAL: u8 = 4;
const SHAPE_GATE: u8 = 5;

// What the check said of a row.
const ROW_UNKNOWN: u8 = 0;
const ROW_AS_READ: u8 = 1;
const ROW_WITHOUT_FAILED: u8 = 2;
const ROW_FAILED_FIRST: u8 = 3;
const ROW_BAD: u8 = 4;

pub struct Row {
    /// The clause's literals: the row wants at least one.
    pub lits: Vec<i32>,
    /// The input clause (0-based index in the Formula), or None for a resolvent
    /// of input clauses, which `prove_row` derives. With an input clause `lits`
    /// holds exactly that clause's literals.
    pub clause: Option<u32>,
    /// Literals the row's proof must first show false: each fails alone, and
    /// propagation does not reach it from the row's literals being false.
    failed: Vec<i32>,
}

pub struct Col {
    /// Literals that exclude each other pairwise, sorted.
    pub lits: Vec<i32>,
    /// The pairs that are not an input clause: each takes a `rup` line.
    rups: u64,
    /// The input pair clauses among the columns of one `read`.
    pairs: Arc<PairTable>,
}

pub struct Grid {
    pub rows: Vec<Row>,
    pub cols: Vec<Col>,
}

/// The input pair clauses (~a v ~b) whose two literals sit in columns, for the
/// `pol` lines. All columns share it: one pair sits in many columns of many grids,
/// and a table per column would cost the square of its size.
struct PairTable {
    /// The literals, sorted; a literal's position is its slot.
    lits: Vec<i32>,
    /// Per slot, its range in `ent`.
    start: Vec<u32>,
    /// (the other literal's slot, always the larger; the constraint id), sorted.
    ent: Vec<(u32, u32)>,
}

impl PairTable {
    fn id(&self, a: i32, b: i32) -> Option<u32> {
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        let sa = self.lits.binary_search(&a).ok()?;
        let sb = self.lits.binary_search(&b).ok()? as u32;
        let e = &self.ent[self.start[sa] as usize..self.start[sa + 1] as usize];
        e.binary_search_by_key(&sb, |p| p.0).ok().map(|i| e[i].1)
    }
}

/// The number of proof lines `prove_row` writes.
pub fn row_lines(row: &Row) -> u64 {
    if row.clause.is_some() {
        return 0;
    }
    1 + row.failed.len() as u64
}

/// The number of proof lines `prove_col` writes.
pub fn col_lines(col: &Col) -> u64 {
    if col.lits.len() < 2 {
        return 0;
    }
    col.rups + (col.lits.len() >= 3) as u64
}

/// The proof lines of a row that is not an input clause. Returns its id, or None
/// with nothing written when `max_lines` does not cover the proof. An input clause
/// takes no line, and its id is returned.
pub fn prove_row(row: &Row, w: &mut Writer, max_lines: u64, work: &mut u64) -> Option<u64> {
    *work += 1;
    if let Some(i) = row.clause {
        return Some(i as u64 + 1);
    }
    if row_lines(row) > max_lines {
        return None;
    }
    *work += (row.lits.len() + row.failed.len()) as u64;
    for &l in &row.failed {
        w.rup(&[-l]).ok()?;
    }
    // with all its literals false, propagation finds a clause it came from false
    w.rup(&row.lits).ok()
}

/// The proof lines of a column's at-most-one: `rup` lines for the pairs that are
/// not input clauses, then one `pol` line. Returns the id of the constraint
/// "sum of ~l >= n - 1", or None with nothing written when `max_lines` does not
/// cover the proof.
///
/// The `pol` line, for literals l1 ... ln: C2 is the pair clause of l1 and l2, and
/// C(m+1) = (pair(1,m+1) + ... + pair(m,m+1) + (m-1) Cm) / m is the sum of the ~l
/// over the first m+1 literals >= m (`tools/hall_toy_proof.py`). Two literals need
/// no `pol`: their pair clause says it.
pub fn prove_col(col: &Col, w: &mut Writer, max_lines: u64, work: &mut u64) -> Option<u64> {
    let n = col.lits.len();
    *work += 1;
    if n < 2 || col_lines(col) > max_lines {
        return None;
    }
    *work += (n as u64) * (n as u64);
    let mut ids: Vec<u64> = Vec::with_capacity(n * (n - 1) / 2);
    for m in 1..n {
        for i in 0..m {
            let (a, b) = (col.lits[i], col.lits[m]);
            let id = match col.pairs.id(a, b) {
                Some(id) => id as u64,
                None => w.rup(&[-a, -b]).ok()?,
            };
            ids.push(id);
        }
    }
    if n == 2 {
        return Some(ids[0]);
    }
    let mut expr = ids[0].to_string();
    let mut k = 1;
    for m in 2..n {
        if m >= 3 {
            expr.push_str(&format!(" {} *", m - 1));
        }
        for _ in 0..m {
            expr.push_str(&format!(" {} +", ids[k]));
            k += 1;
        }
        expr.push_str(&format!(" {} d", m));
    }
    w.pol(&expr).ok()
}

/// The grids of the input clauses, within `budget` (a tick count to stop at).
/// Work is charged to `ticks` as it is done; `stop` is the termination flag. A read
/// that stops at its budget returns what it has checked so far.
pub fn read(f: &Formula, budget: u64, ticks: &mut u64, stop: &AtomicBool) -> Vec<Grid> {
    let debug = std::env::var_os("SAT_STRUCT_DEBUG");
    let (more, debug) = (debug.as_deref() == Some(std::ffi::OsStr::new("2")), debug.is_some());
    let t0 = debug.then(std::time::Instant::now);
    let begin = *ticks;
    let mut clock = Clock { flagged: stop.load(Ordering::Relaxed), units: 0, ticks, stop, last_poll: begin };
    let mut notes = String::new();
    let mut grids = Vec::new();
    if let Some(live) = Live::new(f, &mut clock, budget) {
        if let Some(mut r) = Reader::new(live, &mut clock, budget) {
            let detect = clock.now();
            let wall_setup = t0.map_or(0.0, |t| t.elapsed().as_secs_f64());
            r.run(&mut clock);
            let check = clock.now();
            let wall_detect = t0.map_or(0.0, |t| t.elapsed().as_secs_f64());
            grids = r.finish(&mut clock);
            if debug {
                notes = format!(
                    "setup {} detect {} check {} wall_setup {:.3} wall_detect {:.3} rows {} literals {} classified {} partial {} found {} kept {} rejected {} stopped {} partial_read {}",
                    detect - begin,
                    check - detect,
                    clock.now() - check,
                    wall_setup,
                    wall_detect - wall_setup,
                    r.nrows(),
                    r.ulit.len(),
                    r.classified.len(),
                    r.partial_rows,
                    r.grids.len(),
                    r.kept,
                    r.rejected,
                    r.stopped,
                    r.part2,
                );
            }
        }
    }
    if more {
        // for tools/grid_read_check.py: each grid's rows' clauses and its proof lines
        for g in &grids {
            let clauses: Vec<i64> = g.rows.iter().map(|r| r.clause.map_or(-1, |i| i as i64)).collect();
            let rows: Vec<u64> = g.rows.iter().map(row_lines).collect();
            let cols: Vec<u64> = g.cols.iter().map(col_lines).collect();
            eprintln!("gridinfo clauses={:?} rowlines={:?} collines={:?}", clauses, rows, cols);
        }
    }
    if let Some(t0) = t0 {
        eprintln!("gridread: begin {} budget {} end {} returned {} {} wall {:.3}", begin, budget, clock.now(), grids.len(), notes, t0.elapsed().as_secs_f64());
    }
    grids
}

// ------------------------------------------------------------------ the clock

struct Clock<'a> {
    ticks: &'a mut u64,
    /// The part of a tick not charged yet.
    units: u64,
    stop: &'a AtomicBool,
    last_poll: u64,
    flagged: bool,
}

impl Clock<'_> {
    #[inline]
    fn now(&self) -> u64 {
        *self.ticks
    }

    /// Charge work already done.
    #[inline]
    fn add(&mut self, units: u64) {
        let u = self.units + units;
        *self.ticks = self.ticks.saturating_add(u / TICK);
        self.units = u % TICK;
    }

    /// The termination flag, looked at by work done.
    #[inline]
    fn flagged(&mut self) -> bool {
        if self.ticks.wrapping_sub(self.last_poll) >= 1024 {
            self.last_poll = *self.ticks;
            self.flagged = self.flagged || self.stop.load(Ordering::Relaxed);
        }
        self.flagged
    }

    /// Charge `units` for work about to be done, when `limit` covers it.
    #[inline]
    fn take(&mut self, units: u64, limit: u64) -> bool {
        if self.flagged() || self.ticks.saturating_add((self.units + units) / TICK) > limit {
            return false;
        }
        self.add(units);
        true
    }

    /// Stopped by the limit or the flag.
    #[inline]
    fn over(&mut self, limit: u64) -> bool {
        self.flagged() || *self.ticks >= limit
    }
}

#[inline]
fn lx(l: i32) -> usize {
    (l.unsigned_abs() as usize) * 2 + (l < 0) as usize
}

/// SplitMix64, from a fixed seed.
fn next_random(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn shuffle(v: &mut [u32], state: &mut u64) {
    for i in (1..v.len()).rev() {
        let j = (next_random(state) % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
}

#[inline]
fn length_bucket(n: usize) -> u32 {
    match n {
        0..=4 => n as u32,
        5..=8 => 8,
        9..=16 => 16,
        17..=64 => 64,
        _ => 65,
    }
}

// --------------------------------------------------------------- the formula

#[derive(PartialEq, Eq, Clone, Copy)]
enum Up {
    Done,
    Conflict,
    Hub,
    Stopped,
}

/// The input clauses as propagation needs them, with the root units propagated for
/// good. Clause numbers are the Formula's. A literal fixed at the root stays in the
/// lists and is seen through `val`, so the propagation here is the propagation over
/// the input clauses, which is what VeriPB's `rup` does.
struct Live<'a> {
    f: &'a Formula,
    /// Per literal: 1 true, -1 false, 0 open.
    val: Vec<i8>,
    /// Per literal: what it implies through a two-literal clause (~l v m).
    bin_start: Vec<u32>,
    bin_lit: Vec<i32>,
    /// That clause's constraint id, or 0 when it repeats a literal: its constraint
    /// then has another coefficient, and a `pol` line may not add it.
    bin_id: Vec<u32>,
    /// Per literal: the clauses of three or more literals holding it.
    occ_start: Vec<u32>,
    occ: Vec<u32>,
    /// Per clause of three or more literals: how many are not false.
    left: Vec<u32>,
    /// What a look at `left` costs: little while that table stays in the caches.
    visit: u64,
    /// Per literal: it propagates something alone (it has a clause (~l v m), m open).
    propagates: Vec<bool>,
    /// Root units were found: the three tables below are in use.
    fixed: bool,
    /// Per clause: satisfied at the root, or down to two literals there.
    dead: Vec<bool>,
    /// Per literal: the clauses of three or more literals not dead.
    live_count: Vec<u32>,
    /// Per literal with one such clause: that clause.
    single: Vec<u32>,
    trail: Vec<i32>,
    /// The start of the trail whose clauses have been counted in `left`.
    done: usize,
    /// Clauses with one literal left that is not false, to look at.
    fire: Vec<u32>,
}

impl<'a> Live<'a> {
    /// None when the budget does not cover it or the root units conflict: then no
    /// grid is read, and the search refutes the formula at once.
    fn new(f: &'a Formula, clock: &mut Clock, budget: u64) -> Option<Live<'a>> {
        let nv = f.vars.max(0) as usize;
        let nl = 2 * (nv + 1);
        let m = f.len();
        if !clock.take(items(3 * nl + m), budget) {
            return None;
        }
        let val = vec![0i8; nl];
        let mut bin_start = vec![0u32; nl + 1];
        let mut occ_start = vec![0u32; nl + 1];
        let mut left = vec![0u32; m];
        let mut units: Vec<i32> = Vec::new();
        // the first pass: the sizes of the lists, the units
        for (i, slot) in left.iter_mut().enumerate() {
            let c = f.clause(i);
            if !clock.take(1 + looks(c.len()), budget) {
                return None;
            }
            if f.taut[i] {
                continue;
            }
            match c.len() {
                0 => return None, // an empty clause in the input
                1 => units.push(c[0]),
                2 => {
                    bin_start[lx(-c[0])] += 1;
                    bin_start[lx(-c[1])] += 1;
                }
                n => {
                    *slot = n as u32;
                    for &l in c {
                        occ_start[lx(l)] += 1;
                    }
                }
            }
        }
        if !clock.take(items(2 * nl), budget) {
            return None;
        }
        let mut sum = 0u32;
        for s in bin_start.iter_mut() {
            sum += *s;
            *s = sum;
        }
        let nbin = sum as usize;
        sum = 0;
        for s in occ_start.iter_mut() {
            sum += *s;
            *s = sum;
        }
        let nocc = sum as usize;
        // with no two-literal clause and no unit to make one, nothing propagates alone
        if nbin == 0 && units.is_empty() {
            return None;
        }
        if !clock.take(items(2 * nbin + nocc), budget) {
            return None;
        }
        let mut bin_lit = vec![0i32; nbin];
        let mut bin_id = vec![0u32; nbin];
        let mut occ = vec![0u32; nocc];
        // the second pass, last clause first: each list ends up in file order and
        // its start is where the filling stopped
        for i in (0..m).rev() {
            let c = f.clause(i);
            if !clock.take(1 + looks(c.len()), budget) {
                return None;
            }
            if f.taut[i] {
                continue;
            }
            if c.len() == 2 {
                let id = if f.dup[i] { 0 } else { i as u32 + 1 };
                for (a, b) in [(c[0], c[1]), (c[1], c[0])] {
                    let x = lx(-a);
                    bin_start[x] -= 1;
                    bin_lit[bin_start[x] as usize] = b;
                    bin_id[bin_start[x] as usize] = id;
                }
            } else if c.len() >= 3 {
                for &l in c {
                    let x = lx(l);
                    occ_start[x] -= 1;
                    occ[occ_start[x] as usize] = i as u32;
                }
            }
        }
        let mut live = Live {
            f,
            val,
            bin_start,
            bin_lit,
            bin_id,
            occ_start,
            occ,
            left,
            visit: if m <= 1 << 18 {
                3
            } else if m <= 1 << 21 {
                8
            } else {
                TICK
            },
            propagates: Vec::new(),
            fixed: false,
            dead: Vec::new(),
            live_count: Vec::new(),
            single: Vec::new(),
            trail: Vec::new(),
            done: 0,
            fire: Vec::new(),
        };
        if !live.root(&units, clock, budget) {
            return None;
        }
        if !live.find_propagating(clock, budget) {
            return None;
        }
        Some(live)
    }

    /// Propagates the root units and says which clauses of three or more literals
    /// are out of use, as the census's reduced formula has them.
    fn root(&mut self, units: &[i32], clock: &mut Clock, budget: u64) -> bool {
        if units.is_empty() {
            return true;
        }
        for &u in units {
            if !self.assume(u) {
                return false;
            }
        }
        if self.propagate(clock, budget, usize::MAX) != Up::Done {
            return false;
        }
        self.fixed = true;
        let f = self.f;
        let nl = self.val.len();
        if !clock.take(items(3 * nl + f.len()), budget) {
            return false;
        }
        self.dead = vec![false; f.len()];
        self.live_count = (0..nl).map(|x| self.occ_start[x + 1] - self.occ_start[x]).collect();
        self.propagates = vec![false; nl];
        // first the satisfied clauses
        for t in 0..self.trail.len() {
            let x = lx(self.trail[t]);
            let (a, b) = (self.occ_start[x] as usize, self.occ_start[x + 1] as usize);
            if !clock.take(TICK + looks(b - a), budget) {
                return false;
            }
            for k in a..b {
                if !self.kill(self.occ[k] as usize, clock, budget) {
                    return false;
                }
            }
        }
        // then a clause not satisfied with two left has two open literals: it says
        // what a two-literal clause says
        for t in 0..self.trail.len() {
            let x = lx(-self.trail[t]);
            let (a, b) = (self.occ_start[x] as usize, self.occ_start[x + 1] as usize);
            if !clock.take(TICK + looks(b - a), budget) {
                return false;
            }
            for k in a..b {
                let ci = self.occ[k] as usize;
                if !self.dead[ci] && self.left[ci] <= 2 {
                    if !clock.take(looks(f.clause(ci).len()), budget) {
                        return false;
                    }
                    for &l in f.clause(ci) {
                        if self.val[lx(l)] == 0 {
                            self.propagates[lx(-l)] = true;
                        }
                    }
                    if !self.kill(ci, clock, budget) {
                        return false;
                    }
                }
            }
        }
        // the one clause of a literal that has one left
        self.single = vec![NONE; nl];
        for x in 0..nl {
            if self.live_count[x] == 1 {
                let (a, b) = (self.occ_start[x] as usize, self.occ_start[x + 1] as usize);
                if !clock.take(TICK + looks(b - a), budget) {
                    return false;
                }
                self.single[x] = self.occ[a..b].iter().copied().find(|&ci| !self.dead[ci as usize]).unwrap_or(NONE);
            }
        }
        self.trail.clear();
        self.done = 0;
        true
    }

    fn kill(&mut self, ci: usize, clock: &mut Clock, budget: u64) -> bool {
        if self.dead[ci] {
            return true;
        }
        let f = self.f;
        if !clock.take(TICK + looks(f.clause(ci).len()), budget) {
            return false;
        }
        self.dead[ci] = true;
        for &l in f.clause(ci) {
            self.live_count[lx(l)] -= 1;
        }
        true
    }

    /// Which literals propagate something alone.
    fn find_propagating(&mut self, clock: &mut Clock, budget: u64) -> bool {
        let nl = self.val.len();
        if !clock.take(items(2 * nl), budget) {
            return false;
        }
        if !self.fixed {
            self.propagates = (0..nl).map(|x| self.bin_start[x + 1] > self.bin_start[x]).collect();
            return true;
        }
        for x in 0..nl {
            let (a, b) = (self.bin_start[x] as usize, self.bin_start[x + 1] as usize);
            if a == b || self.val[x] != 0 {
                continue;
            }
            if !clock.take(looks(b - a), budget) {
                return false;
            }
            if self.bin_lit[a..b].iter().any(|&t| self.val[lx(t)] == 0) {
                self.propagates[x] = true;
            }
        }
        true
    }

    #[inline]
    fn is_dead(&self, ci: usize) -> bool {
        self.fixed && self.dead[ci]
    }

    /// The one clause of three or more literals in use holding `l`, when there is
    /// exactly one.
    #[inline]
    fn single_of(&self, l: i32) -> Option<u32> {
        let x = lx(l);
        if self.fixed {
            return (self.single[x] != NONE).then_some(self.single[x]);
        }
        (self.occ_start[x + 1] - self.occ_start[x] == 1).then(|| self.occ[self.occ_start[x] as usize])
    }

    /// How many clauses of two or more literals hold `l`. A clause a root unit
    /// satisfied may still count here, which only makes a split look like none.
    #[inline]
    fn holders(&self, l: i32) -> u32 {
        let long = if self.fixed { self.live_count[lx(l)] } else { self.occ_start[lx(l) + 1] - self.occ_start[lx(l)] };
        long + self.bin_start[lx(-l) + 1] - self.bin_start[lx(-l)]
    }

    /// A variable that sits in one clause each way, the one with ~l of three or more
    /// literals, is what splitting a long clause leaves: walking from `l` goes on in
    /// that clause. A literal at the end of a ladder also sits in one clause each way,
    /// the other a two-literal one, and is not this.
    #[inline]
    fn is_split(&self, l: i32) -> bool {
        self.val[lx(l)] == 0 && self.holders(l) == 1 && self.holders(-l) == 1 && self.single_of(-l).is_some()
    }

    /// Put `l` on the trail. False when it is false already.
    #[inline]
    fn assume(&mut self, l: i32) -> bool {
        match self.val[lx(l)] {
            0 => {
                self.val[lx(l)] = 1;
                self.val[lx(-l)] = -1;
                self.trail.push(l);
                true
            }
            1 => true,
            _ => false,
        }
    }

    /// Unit propagation from the trail. `hub` limits the trail's length. A visit to
    /// a long clause touches its counter, a two-literal clause only its place in a
    /// list. All of a literal's clauses are counted before any of them is used, so
    /// `undo` knows what to take back.
    fn propagate(&mut self, clock: &mut Clock, limit: u64, hub: usize) -> Up {
        self.propagate_how(clock, limit, hub, true)
    }

    /// With `pairs` false a two-literal clause can end it in a conflict but assigns
    /// nothing. A conflict found this way is one the whole propagation finds too.
    fn propagate_how(&mut self, clock: &mut Clock, limit: u64, hub: usize, pairs: bool) -> Up {
        let mut i = self.done;
        while i < self.trail.len() {
            let y = self.trail[i];
            let (by, ny) = (lx(y), lx(-y));
            let (b0, b1) = (self.bin_start[by] as usize, self.bin_start[by + 1] as usize);
            let (o0, o1) = (self.occ_start[ny] as usize, self.occ_start[ny + 1] as usize);
            if !clock.take(self.visit * (1 + (o1 - o0) as u64) + looks(b1 - b0), limit) {
                return Up::Stopped;
            }
            for k in o0..o1 {
                let ci = self.occ[k] as usize;
                let left = self.left[ci] - 1;
                self.left[ci] = left;
                if left <= 1 {
                    self.fire.push(ci as u32);
                }
            }
            i += 1;
            self.done = i;
            for k in b0..b1 {
                let t = self.bin_lit[k];
                let v = self.val[lx(t)];
                if v == 0 && pairs {
                    self.val[lx(t)] = 1;
                    self.val[lx(-t)] = -1;
                    self.trail.push(t);
                } else if v < 0 {
                    self.fire.clear();
                    return Up::Conflict;
                }
            }
            while let Some(ci) = self.fire.pop() {
                let c = self.f.clause(ci as usize);
                if !clock.take(looks(c.len()), limit) {
                    self.fire.clear();
                    return Up::Stopped;
                }
                let mut unit = 0;
                let mut sat = false;
                for &m in c {
                    let v = self.val[lx(m)];
                    if v > 0 {
                        sat = true;
                        break;
                    }
                    if v == 0 {
                        unit = m;
                    }
                }
                if sat {
                    continue;
                }
                if unit == 0 {
                    self.fire.clear();
                    return Up::Conflict;
                }
                self.val[lx(unit)] = 1;
                self.val[lx(-unit)] = -1;
                self.trail.push(unit);
            }
            if self.trail.len() > hub {
                return Up::Hub;
            }
        }
        Up::Done
    }

    /// Back to the root: this trail's counts and values go.
    fn undo(&mut self) {
        for t in 0..self.done {
            let ny = lx(-self.trail[t]);
            for k in self.occ_start[ny] as usize..self.occ_start[ny + 1] as usize {
                self.left[self.occ[k] as usize] += 1;
            }
        }
        for t in 0..self.trail.len() {
            let y = self.trail[t];
            self.val[lx(y)] = 0;
            self.val[lx(-y)] = 0;
        }
        self.trail.clear();
        self.done = 0;
    }
}

// ------------------------------------------------------------------ the rows

/// The rows while they are built: sorted literals by row, each literal set once.
struct RowList {
    off: Vec<u32>,
    lits: Vec<i32>,
    clause: Vec<u32>,
    made: Vec<bool>,
    seen: HashMap<u64, u32>,
}

impl RowList {
    /// `lits` is sorted. A row with no clause of its own takes one when a clause
    /// comes along that says just this.
    fn add(&mut self, lits: &[i32], clause: u32, made: bool) {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &l in lits {
            h = (h ^ (l as u32 as u64)).wrapping_mul(0x0000_0100_0000_01B3);
        }
        match self.seen.get(&h) {
            Some(&r) => {
                let (a, b) = (self.off[r as usize] as usize, self.off[r as usize + 1] as usize);
                if &self.lits[a..b] == lits {
                    if self.clause[r as usize] == NONE {
                        self.clause[r as usize] = clause;
                    }
                    return;
                }
                // another literal set with this hash: a row of its own, not looked up
            }
            None => {
                self.seen.insert(h, self.clause.len() as u32);
            }
        }
        self.lits.extend_from_slice(lits);
        self.off.push(self.lits.len() as u32);
        self.clause.push(clause);
        self.made.push(made);
    }
}

/// Every clause of MIN_ROW or more literals once per literal set, and the
/// resolvents that take literals g away when ~g sits in exactly one such clause,
/// for clauses where that holds of at least half the literals (within ROW_STEPS).
/// Then the whole of each split clause: what is left when every variable that sits
/// in one clause each way is resolved away, however long the chain. A row needs two
/// literals that propagate something. None: out of budget.
fn build_rows(live: &Live, clock: &mut Clock, budget: u64) -> Option<RowList> {
    let f = live.f;
    let mut rows = RowList { off: vec![0], lits: Vec::new(), clause: Vec::new(), made: Vec::new(), seen: HashMap::new() };
    let mut steps = 0u64;
    let mut cur: Vec<i32> = Vec::new();
    let mut out: Vec<i32> = Vec::new();
    let mut stack: Vec<(i32, u8)> = Vec::new();
    for i in 0..f.len() {
        let c = f.clause(i);
        if !clock.take(1, budget) {
            return None;
        }
        if f.taut[i] || c.len() < MIN_ROW || live.is_dead(i) {
            continue;
        }
        // the literals, the test for a row, the test for a resolvent
        if !clock.take(3 * looks(c.len()), budget) {
            return None;
        }
        cur.clear();
        cur.extend(c.iter().copied().filter(|&l| live.val[lx(l)] == 0));
        if cur.iter().filter(|&&l| live.propagates[lx(l)]).count() >= 2 {
            if !clock.take(sorting(cur.len()) + TICK + looks(cur.len()), budget) {
                return None;
            }
            let exact = cur.len() == c.len() && !f.dup[i];
            let mut sorted = cur.clone();
            sorted.sort_unstable();
            rows.add(&sorted, if exact { i as u32 } else { NONE }, false);
        }
        if steps < ROW_STEPS && 2 * cur.iter().filter(|&&l| live.single_of(-l).is_some()).count() >= cur.len() {
            // literals come off the stack until none can be resolved away
            out.clear();
            stack.clear();
            stack.extend(cur.iter().map(|&l| (l, 0u8)));
            let mut changed = false;
            let mut too_long = false;
            while let Some((l, depth)) = stack.pop() {
                steps += 1;
                if !clock.take(looks(1), budget) {
                    return None;
                }
                let d = if depth < MAX_DEPTH { live.single_of(-l) } else { None };
                match d {
                    Some(ci) if ci as usize != i => {
                        changed = true;
                        let dc = f.clause(ci as usize);
                        if !clock.take(looks(dc.len()), budget) {
                            return None;
                        }
                        stack.extend(dc.iter().copied().filter(|&m| m != -l && live.val[lx(m)] == 0).map(|m| (m, depth + 1)));
                    }
                    _ => out.push(l),
                }
                if out.len() + stack.len() > MAX_ROW {
                    too_long = true;
                    break;
                }
            }
            if changed && !too_long {
                if !clock.take(2 * sorting(out.len()) + TICK + looks(out.len()), budget) {
                    return None;
                }
                // the row is a set, and not a tautology
                out.sort_unstable_by_key(|&l| (l.abs(), l));
                out.dedup();
                let taut = out.windows(2).any(|w| w[0] == -w[1]);
                if !taut && out.len() >= MIN_ROW && out.iter().filter(|&&l| live.propagates[lx(l)]).count() >= 2 {
                    out.sort_unstable();
                    rows.add(&out, NONE, true);
                }
            }
        }
    }
    add_split_rows(live, &mut rows, &mut steps, clock, budget)?;
    Some(rows)
}

/// The rows of clauses split through variables that sit in one clause each way. From
/// any clause of such a chain or tree the walk reaches all of it, so a literal it has
/// been through is not started from again. A two-literal clause starts one only when
/// both its literals go on: (g1 v g2) with g1 and g2 the two parts of a split in two.
/// With all the leaves false, propagation runs up the tree to a false clause, so the
/// row follows by propagation.
fn add_split_rows(live: &Live, rows: &mut RowList, steps: &mut u64, clock: &mut Clock, budget: u64) -> Option<()> {
    let f = live.f;
    let nl = live.val.len();
    if !clock.take(items(2 * nl), budget) {
        return None;
    }
    let mut been: Vec<bool> = vec![false; nl];
    let mut out: Vec<i32> = Vec::new();
    let mut stack: Vec<i32> = Vec::new();
    for v in 1..(nl / 2) as i32 {
        for l in [v, -v] {
            if been[lx(l)] || !live.is_split(l) {
                continue;
            }
            if !clock.take(TICK, budget) {
                return None;
            }
            if *steps >= ROW_STEPS {
                return Some(());
            }
            stack.clear();
            match live.single_of(l) {
                Some(ci) => stack.extend(f.clause(ci as usize).iter().copied().filter(|&m| live.val[lx(m)] == 0)),
                None => {
                    // (l v m): both must go on
                    let (b0, b1) = (live.bin_start[lx(-l)] as usize, live.bin_start[lx(-l) + 1] as usize);
                    if b1 != b0 + 1 || !live.is_split(live.bin_lit[b0]) {
                        continue;
                    }
                    stack.extend_from_slice(&[l, live.bin_lit[b0]]);
                }
            }
            out.clear();
            let mut changed = false;
            let mut walk = 0usize;
            while let Some(x) = stack.pop() {
                *steps += 1;
                walk += 1;
                if !clock.take(looks(1), budget) {
                    return None;
                }
                if walk + stack.len() + out.len() > 2 * MAX_ROW {
                    changed = false; // too long, or a walk in a circle
                    break;
                }
                if !live.is_split(x) {
                    out.push(x);
                    continue;
                }
                changed = true;
                been[lx(x)] = true;
                been[lx(-x)] = true;
                let dc = f.clause(live.single_of(-x).unwrap() as usize);
                if !clock.take(looks(dc.len()), budget) {
                    return None;
                }
                stack.extend(dc.iter().copied().filter(|&m| m != -x && live.val[lx(m)] == 0));
            }
            if !changed {
                continue;
            }
            if !clock.take(2 * sorting(out.len()) + TICK + looks(out.len()), budget) {
                return None;
            }
            out.sort_unstable_by_key(|&m| (m.abs(), m));
            out.dedup();
            let taut = out.windows(2).any(|w| w[0] == -w[1]);
            if !taut && out.len() >= MIN_ROW && out.len() <= MAX_ROW && out.iter().filter(|&&m| live.propagates[lx(m)]).count() >= 2 {
                out.sort_unstable();
                rows.add(&out, NONE, true);
            }
        }
    }
    Some(())
}

// ---------------------------------------------------------------- the reader

/// A grid while it waits for its check: row numbers (sorted), columns of two or
/// more literals (sorted), and the number of values only one row has.
#[derive(Default)]
struct GridRec {
    rows: Vec<u32>,
    cols: Vec<Vec<u32>>,
    private: u32,
}

enum Neighbours {
    Stopped,
    Cut,
    List(Vec<(u32, u32)>),
    /// A list from the rows read so far only: the row is tried again later.
    Part(Vec<(u32, u32)>),
}

/// What the check says of a grid.
enum Checked {
    Good(Grid),
    /// Nothing wrong, but nothing left: its columns or rows went with failed literals.
    Dropped,
    /// It claims what the input does not say. Detection has a fault; it is not used.
    Wrong,
    Stopped,
}

/// A clique cover of a grid's literals in the making, by position in the grid. A
/// position's row and, as lists, the positions in other rows it excludes.
struct Cover {
    at: Vec<u32>,
    start: Vec<u32>,
    end: Vec<u32>,
    list: Vec<u32>,
    col: Vec<u32>,
    // the options of the clique in hand: positions that exclude all of it
    flat: Vec<u32>,
    in_flat: Vec<bool>,
    marked: Vec<u32>,
    mark: u32,
    row_n: Vec<u32>,
    rows_here: Vec<u32>,
    limit: u64,
}

impl Cover {
    #[inline]
    fn excl(&self, p: u32) -> (usize, usize) {
        (self.start[p as usize] as usize, self.end[p as usize] as usize)
    }

    /// The options of a clique holding `p` alone. False: out of budget.
    fn options_of(&mut self, p: u32, clock: &mut Clock) -> bool {
        let (a, b) = self.excl(p);
        if !clock.take(TICK + looks(b - a), self.limit) {
            return false;
        }
        self.flat.clear();
        for k in a..b {
            let q = self.list[k];
            if self.col[q as usize] == NONE {
                self.flat.push(q);
                self.in_flat[q as usize] = true;
            }
        }
        true
    }

    /// The row with the fewest options, the first such row on a tie.
    fn fewest(&mut self) -> u32 {
        self.rows_here.clear();
        for &q in &self.flat {
            let r = self.at[q as usize] as usize;
            if self.row_n[r] == 0 {
                self.rows_here.push(r as u32);
            }
            self.row_n[r] += 1;
        }
        let mut best = self.rows_here[0];
        for &r in &self.rows_here {
            if (self.row_n[r as usize], r) < (self.row_n[best as usize], best) {
                best = r;
            }
        }
        for &r in &self.rows_here {
            self.row_n[r as usize] = 0;
        }
        best
    }

    /// The literals of the row a clique from `p` takes next, sorted.
    fn first_options(&mut self, p: u32, out: &mut Vec<u32>, clock: &mut Clock) -> bool {
        out.clear();
        if !self.options_of(p, clock) {
            return false;
        }
        if !self.flat.is_empty() {
            if !clock.take(TICK + looks(self.flat.len()), self.limit) {
                return false;
            }
            let best = self.fewest();
            out.extend(self.flat.iter().copied().filter(|&q| self.at[q as usize] == best));
            out.sort_unstable();
        }
        for &q in &self.flat {
            self.in_flat[q as usize] = false;
        }
        self.flat.clear();
        true
    }

    /// The clique from `p` with `first` next: then the row with the fewest options,
    /// and in it the literal that leaves the most options, the first on a tie.
    /// False: out of budget.
    fn clique(&mut self, p: u32, first: Option<u32>, out: &mut Vec<u32>, clock: &mut Clock) -> bool {
        out.clear();
        out.push(p);
        if !self.options_of(p, clock) {
            return false;
        }
        let mut next = first;
        let mut pick: Vec<u32> = Vec::new();
        while !self.flat.is_empty() {
            if !clock.take(TICK + 2 * looks(self.flat.len()), self.limit) {
                return self.give_up();
            }
            let row = match next {
                Some(m) => self.at[m as usize],
                None => self.fewest(),
            };
            pick.clear();
            for k in 0..self.flat.len() {
                let q = self.flat[k];
                if self.at[q as usize] == row {
                    pick.push(q);
                    self.in_flat[q as usize] = false;
                }
            }
            let at = &self.at;
            self.flat.retain(|&q| at[q as usize] != row);
            let mut m = pick[0];
            if let Some(f) = next.take() {
                m = f;
            } else if pick.len() > 1 {
                pick.sort_unstable();
                let mut top = 0;
                for (i, &x) in pick.iter().enumerate() {
                    let (c, d) = self.excl(x);
                    if !clock.take(TICK + looks(d - c), self.limit) {
                        return self.give_up();
                    }
                    let score = self.list[c..d].iter().filter(|&&y| self.in_flat[y as usize]).count();
                    if i == 0 || score > top {
                        top = score;
                        m = x;
                    }
                }
            }
            out.push(m);
            // what stays must exclude that literal too
            self.mark += 1;
            let (c, d) = self.excl(m);
            if !clock.take(TICK + looks(d - c + self.flat.len()), self.limit) {
                return self.give_up();
            }
            for k in c..d {
                self.marked[self.list[k] as usize] = self.mark;
            }
            let (marked, in_flat, mark) = (&self.marked, &mut self.in_flat, self.mark);
            self.flat.retain(|&q| {
                let keep = marked[q as usize] == mark;
                if !keep {
                    in_flat[q as usize] = false;
                }
                keep
            });
        }
        true
    }

    fn give_up(&mut self) -> bool {
        for &q in &self.flat {
            self.in_flat[q as usize] = false;
        }
        self.flat.clear();
        false
    }
}

/// The reading. Literals of rows are numbered in sorted order, so a sorted list of
/// the numbers is sorted by literal, as the census's lists are.
struct Reader<'a> {
    live: Live<'a>,
    /// A literal's read stops past this many literals, now.
    hub: usize,
    budget: u64,
    /// Detection stops at this tick count.
    det_limit: u64,
    /// One row's minimum share of detection.
    row_share: u64,
    /// One row's share now.
    row_cap: u64,
    rng: u64,
    // the rows: their literals by row
    row_off: Vec<u32>,
    row_lits: Vec<u32>,
    row_clause: Vec<u32>,
    row_made: Vec<bool>,
    ulit: Vec<i32>,
    uid: Vec<u32>,
    // what a literal excludes among the rows' literals, read from its side
    ex_off: Vec<u32>,
    ex_len: Vec<u32>,
    ex_arena: Vec<u32>,
    /// The literals read so far that exclude a literal: it may show from one side.
    incoming: Vec<Vec<u32>>,
    entries: u64,
    want_reset: bool,
    /// Propagating the literal alone fails: it is false at the root.
    failed: Vec<bool>,
    /// The same, found only when the check read the literal again.
    late_failed: Vec<bool>,
    checking: bool,
    /// The literal's read stopped at the short limit.
    short: Vec<bool>,
    /// Such a read is done again when it is needed.
    read_again: bool,
    any_short: bool,
    /// Detection has run out of stamps.
    stamps_out: bool,
    /// Rows are linked in the order they were read, not by kind in random order.
    in_order: bool,
    // what a literal excludes among the domains' literals, this turn
    turn: u32,
    dom_turn: Vec<u32>,
    dom_off: Vec<u32>,
    dom_len: Vec<u32>,
    dom_arena: Vec<u32>,
    /// By literal, the rows in this turn's index: the ones that are or may be domains.
    dr_start: Vec<u32>,
    dr: Vec<u32>,
    /// The same index over the rows read so far, for a row whose scan over all rows
    /// would not fit. `kr_turn` is the turn it was made in.
    kr_start: Vec<u32>,
    kr: Vec<u32>,
    kr_turn: u32,
    index_dirty: bool,
    /// Partial rows have been let in.
    part2: bool,
    partial_rows: usize,
    // what a row is
    shape: Vec<u8>,
    classified: Vec<u32>,
    linked: Vec<bool>,
    queues: Vec<VecDeque<u32>>,
    cut: Vec<u32>,
    // `neighbours`' counters, per row
    pc: Vec<u32>,
    ls: Vec<u32>,
    ms: Vec<u32>,
    last_l: Vec<u32>,
    // the rows sharing a whole grid with a row: the groups, and each row's groups
    groups: Vec<Vec<u32>>,
    row_groups: Vec<Vec<u32>>,
    cover_tag: Vec<u32>,
    // tags: a value counts when it equals the stamp of the use in hand
    stamp: u32,
    own_tag: Vec<u32>,
    m_tag: Vec<u32>,
    e_tag: Vec<u32>,
    x_tag: Vec<u32>,
    g_tag: Vec<u32>,
    g_pos: Vec<u32>,
    // the grid being grown: its literals' rows, and which of its literals exclude a
    // literal, as lists in an arena
    grow_stamp: u32,
    lit_tag: Vec<u32>,
    lit_row: Vec<u32>,
    rev_tag: Vec<u32>,
    rev_head: Vec<u32>,
    rev_next: Vec<u32>,
    rev_val: Vec<u32>,
    // `fits`' counters, per row of the grid
    edges: Vec<u32>,
    ours: Vec<u32>,
    theirs: Vec<u32>,
    last_m: Vec<u32>,
    // the grids
    grids: Vec<GridRec>,
    grid_seen: HashMap<u64, u32>,
    kept: usize,
    rejected: u64,
    stopped: bool,
    row_state: Vec<u8>,
}

impl<'a> Reader<'a> {
    fn new(live: Live<'a>, clock: &mut Clock, budget: u64) -> Option<Reader<'a>> {
        let rows = build_rows(&live, clock, budget)?;
        let nl = live.val.len();
        let nv = nl / 2 - 1;
        let nrows = rows.clause.len();
        let total = rows.lits.len();
        // the numbers of the literals, in sorted order
        if !clock.take(items(2 * nl) + 2 * looks(total), budget) {
            return None;
        }
        let mut uid = vec![NONE; nl];
        for &l in &rows.lits {
            uid[lx(l)] = 0;
        }
        let mut ulit: Vec<i32> = Vec::new();
        for v in (1..=nv as i32).rev() {
            if uid[lx(-v)] != NONE {
                uid[lx(-v)] = ulit.len() as u32;
                ulit.push(-v);
            }
        }
        for v in 1..=nv as i32 {
            if uid[lx(v)] != NONE {
                uid[lx(v)] = ulit.len() as u32;
                ulit.push(v);
            }
        }
        let nu = ulit.len();
        let row_lits: Vec<u32> = rows.lits.iter().map(|&l| uid[lx(l)]).collect();
        // the tables per literal of a row and per row: most stay zero until used,
        // and the use is charged where it happens
        if !clock.take(items(8 * nu + 6 * nrows), budget) {
            return None;
        }
        let det_start = clock.now();
        let room = budget.saturating_sub(det_start);
        let det_limit = det_start + room - room / 100 * CHECK_SHARE;
        let row_share = ((det_limit - det_start) / ROW_SHARE).max(1);
        let mut r = Reader {
            live,
            hub: SHORT_READ,
            budget,
            det_limit,
            row_share,
            row_cap: row_share,
            rng: 0x5eed,
            row_off: rows.off,
            row_lits,
            row_clause: rows.clause,
            row_made: rows.made,
            ulit,
            uid,
            ex_off: vec![NONE; nu],
            ex_len: vec![0; nu],
            ex_arena: Vec::new(),
            incoming: vec![Vec::new(); nu],
            entries: 0,
            want_reset: false,
            failed: vec![false; nu],
            late_failed: Vec::new(),
            checking: false,
            short: vec![false; nu],
            read_again: false,
            any_short: false,
            stamps_out: false,
            in_order: false,
            turn: 0,
            dom_turn: vec![0; nu],
            dom_off: vec![0; nu],
            dom_len: vec![0; nu],
            dom_arena: Vec::new(),
            dr_start: Vec::new(),
            dr: Vec::new(),
            kr_start: Vec::new(),
            kr: Vec::new(),
            kr_turn: 0,
            index_dirty: true,
            part2: false,
            partial_rows: 0,
            shape: vec![SHAPE_UNKNOWN; nrows],
            classified: Vec::new(),
            linked: vec![false; nrows],
            queues: Vec::new(),
            cut: Vec::new(),
            pc: vec![0; nrows],
            ls: vec![0; nrows],
            ms: vec![0; nrows],
            last_l: vec![NONE; nrows],
            groups: Vec::new(),
            row_groups: vec![Vec::new(); nrows],
            cover_tag: vec![0; nrows],
            stamp: 0,
            own_tag: vec![0; nu],
            m_tag: vec![0; nu],
            e_tag: vec![0; nu],
            x_tag: vec![0; nu],
            g_tag: vec![0; nu],
            g_pos: vec![0; nu],
            grow_stamp: 0,
            lit_tag: vec![0; nu],
            lit_row: vec![0; nu],
            rev_tag: vec![0; nu],
            rev_head: vec![0; nu],
            rev_next: Vec::new(),
            rev_val: Vec::new(),
            edges: Vec::new(),
            ours: Vec::new(),
            theirs: Vec::new(),
            last_m: Vec::new(),
            grids: Vec::new(),
            grid_seen: HashMap::new(),
            kept: 0,
            rejected: 0,
            stopped: false,
            row_state: vec![ROW_UNKNOWN; nrows],
        };
        r.make_queues();
        Some(r)
    }

    #[inline]
    fn nrows(&self) -> usize {
        self.row_clause.len()
    }

    #[inline]
    fn row_range(&self, ri: u32) -> (usize, usize) {
        (self.row_off[ri as usize] as usize, self.row_off[ri as usize + 1] as usize)
    }

    /// A new value for the tags. Detection stops well before the numbers run out,
    /// which takes billions of ticks; the check needs far fewer than what stays.
    fn new_stamp(&mut self) -> u32 {
        if self.stamp >= u32::MAX - (1 << 26) {
            self.stamps_out = true;
            self.det_limit = 0;
        }
        self.stamp = self.stamp.wrapping_add(1);
        self.stamp
    }

    /// (one sign or not, length bucket, a resolvent or not): the rows' classes.
    fn row_class(&self, ri: u32) -> (bool, u32, bool) {
        let (a, b) = self.row_range(ri);
        let first = self.ulit[self.row_lits[a] as usize];
        let last = self.ulit[self.row_lits[b - 1] as usize];
        ((first < 0) == (last < 0), length_bucket(b - a), self.row_made[ri as usize])
    }

    /// Rows by class in random order, to be read round robin, so that a small class
    /// is read in full and a big one sampled. Reserved by the caller.
    fn make_queues(&mut self) {
        let n = self.nrows();
        // the classes: two ways for the signs, six length buckets, two for resolvents
        let bucket = |b: u32| -> usize {
            match b {
                0..=3 => 0,
                4 => 1,
                8 => 2,
                16 => 3,
                64 => 4,
                _ => 5,
            }
        };
        let mut by_class: Vec<Vec<u32>> = vec![Vec::new(); 2 * 6 * 2];
        for ri in 0..n as u32 {
            let (sign, b, made) = self.row_class(ri);
            by_class[(sign as usize) * 12 + bucket(b) * 2 + made as usize].push(ri);
        }
        for mut q in by_class {
            if !q.is_empty() {
                shuffle(&mut q, &mut self.rng);
                self.queues.push(q.into_iter().collect());
            }
        }
    }

    // ------------------------------------------------------------ the exclusions

    /// Starts the cache over past MAX_ENTRIES. Called only where no one is in the
    /// middle of reading a row's literals from it.
    fn maybe_reset(&mut self, clock: &mut Clock) {
        if !self.want_reset {
            return;
        }
        self.want_reset = false;
        clock.add(items(2 * self.ulit.len()));
        self.ex_off.fill(NONE);
        self.ex_arena.clear();
        for v in self.incoming.iter_mut() {
            v.clear();
        }
        self.entries = 0;
    }

    /// Reads what literal `u` excludes. False when the budget stopped it. A read
    /// past `hub` keeps what it has: those literals are excluded all the same. Once
    /// the short reads are over, a read that was cut short is done again.
    fn read_lit(&mut self, u: u32, limit: u64, clock: &mut Clock) -> bool {
        let ui = u as usize;
        if self.ex_off[ui] != NONE && !(self.short[ui] && self.read_again) {
            return true;
        }
        if clock.over(limit) {
            return false;
        }
        let assumed = self.live.assume(self.ulit[ui]);
        debug_assert!(assumed);
        let r = self.live.propagate(clock, limit, self.hub);
        let off = self.ex_arena.len();
        match r {
            Up::Stopped => {
                self.live.undo();
                return false;
            }
            Up::Conflict => {
                // in the check it counts for the pair in hand: what a row holds was
                // settled when the row was checked
                if self.checking {
                    self.late_failed[ui] = true;
                } else {
                    self.failed[ui] = true;
                }
                self.short[ui] = false;
            }
            Up::Hub | Up::Done => {
                if !clock.take(looks(self.live.trail.len()), limit) {
                    self.live.undo();
                    return false;
                }
                for k in 0..self.live.trail.len() {
                    let m = self.uid[lx(-self.live.trail[k])];
                    if m != NONE {
                        self.ex_arena.push(m);
                    }
                }
                let g = self.ex_arena.len() - off;
                if !clock.take(sorting(g) + looks(g), limit) {
                    self.ex_arena.truncate(off);
                    self.live.undo();
                    return false;
                }
                self.ex_arena[off..].sort_unstable();
                for k in off..off + g {
                    let m = self.ex_arena[k] as usize;
                    self.incoming[m].push(u);
                }
                self.short[ui] = r == Up::Hub && self.hub < HUB;
                self.any_short |= self.short[ui];
            }
        }
        self.live.undo();
        let g = self.ex_arena.len() - off;
        self.ex_off[ui] = off as u32;
        self.ex_len[ui] = g as u32;
        self.entries += 2 * g as u64 + 1;
        if self.entries > MAX_ENTRIES {
            self.want_reset = true;
        }
        true
    }

    #[inline]
    fn ex_range(&self, u: u32) -> (usize, usize) {
        let a = self.ex_off[u as usize] as usize;
        (a, a + self.ex_len[u as usize] as usize)
    }

    /// What `u` excludes among the domains' literals this turn, from either side
    /// as far as has been read. False when the budget stopped it.
    fn link_lit(&mut self, u: u32, clock: &mut Clock) -> bool {
        let ui = u as usize;
        if self.dom_turn[ui] == self.turn && !(self.short[ui] && self.read_again) {
            return true;
        }
        if !self.read_lit(u, self.det_limit, clock) {
            return false;
        }
        let (a, b) = self.ex_range(u);
        let back = self.incoming[ui].len();
        if !clock.take(TICK + looks(b - a + back) + if back > 0 { sorting(b - a + back) } else { 0 }, self.det_limit) {
            return false;
        }
        let start = self.dom_arena.len();
        for k in a..b {
            let m = self.ex_arena[k] as usize;
            if self.dr_start[m + 1] > self.dr_start[m] {
                self.dom_arena.push(m as u32);
            }
        }
        if back > 0 {
            for k in 0..back {
                let m = self.incoming[ui][k] as usize;
                if self.dr_start[m + 1] > self.dr_start[m] {
                    self.dom_arena.push(m as u32);
                }
            }
            self.dom_arena[start..].sort_unstable();
            let mut w = start;
            for k in start..self.dom_arena.len() {
                if k == start || self.dom_arena[k] != self.dom_arena[k - 1] {
                    self.dom_arena[w] = self.dom_arena[k];
                    w += 1;
                }
            }
            self.dom_arena.truncate(w);
        }
        self.dom_off[ui] = start as u32;
        self.dom_len[ui] = (self.dom_arena.len() - start) as u32;
        self.dom_turn[ui] = self.turn;
        true
    }

    #[inline]
    fn dom_range(&self, u: u32) -> (usize, usize) {
        let a = self.dom_off[u as usize] as usize;
        (a, a + self.dom_len[u as usize] as usize)
    }

    /// The row's literals not known to be false at the root.
    fn domain(&self, ri: u32) -> Vec<u32> {
        let (a, b) = self.row_range(ri);
        self.row_lits[a..b].iter().copied().filter(|&u| !self.failed[u as usize]).collect()
    }

    // ------------------------------------------------------------ what a row is

    /// What the row's own literals do to each other; each of them has been read.
    /// A pair counts from either side. None when the budget stopped it.
    fn row_shape(&mut self, dom: &[u32], clock: &mut Clock) -> Option<u8> {
        let n = dom.len();
        if n < 2 {
            return Some(SHAPE_GATE);
        }
        if !clock.take(TICK + looks(n), self.det_limit) {
            return None;
        }
        let stamp = self.new_stamp();
        for (p, &u) in dom.iter().enumerate() {
            self.g_tag[u as usize] = stamp;
            self.g_pos[u as usize] = p as u32;
        }
        let mut deg = vec![0u32; n];
        for (p, &l) in dom.iter().enumerate() {
            let (a, b) = self.ex_range(l);
            if !clock.take(TICK + looks(b - a), self.det_limit) {
                return None;
            }
            for k in a..b {
                let m = self.ex_arena[k];
                if self.g_tag[m as usize] != stamp {
                    continue;
                }
                deg[p] += 1;
                // a pair seen from this side only counts for the other literal too
                let (c, d) = self.ex_range(m);
                clock.add(TICK / 2);
                if self.ex_arena[c..d].binary_search(&l).is_err() {
                    deg[self.g_pos[m as usize] as usize] += 1;
                }
            }
        }
        let top = *deg.iter().max().unwrap() as usize;
        if top == 0 || (n >= 5 && top <= (n - 1) / 4) {
            return Some(SHAPE_PLAIN);
        }
        if deg.iter().all(|&d| d as usize == n - 1) {
            return Some(SHAPE_EXACT);
        }
        let zero = deg.iter().filter(|&&d| d == 0).count();
        if n >= 4 && zero == 1 && deg.iter().all(|&d| d == 0 || d as usize == n - 2) {
            return Some(SHAPE_GUARDED);
        }
        // none of the census's three. A gate's clause has a literal that excludes
        // all the others, or all its exclusions run through one literal
        let pairs = deg.iter().map(|&d| d as usize).sum::<usize>() / 2;
        if top == n - 1 || pairs == top {
            return Some(SHAPE_GATE);
        }
        Some(SHAPE_PARTIAL)
    }

    /// Reads what the row's literals do to each other. False when the budget or
    /// `limit` stopped it.
    fn classify(&mut self, ri: u32, limit: u64, clock: &mut Clock) -> bool {
        self.maybe_reset(clock);
        let (a, b) = self.row_range(ri);
        let start = clock.now();
        for k in a..b {
            if clock.now() >= limit || !self.read_lit(self.row_lits[k], self.det_limit, clock) {
                return false;
            }
            if clock.now() - start > self.row_cap {
                return true; // stays unclassified: a row this dear is read as it comes up
            }
        }
        if self.shape[ri as usize] == SHAPE_UNKNOWN {
            let dom = self.domain(ri);
            let Some(s) = self.row_shape(&dom, clock) else { return false };
            self.set_shape(ri, s);
        }
        true
    }

    fn set_shape(&mut self, ri: u32, s: u8) {
        self.shape[ri as usize] = s;
        self.classified.push(ri);
        if s == SHAPE_PARTIAL {
            self.partial_rows += 1;
        }
        if s == SHAPE_PARTIAL || s == SHAPE_GATE {
            self.index_dirty = true;
        }
    }

    /// Whether a row of this shape counts as a domain now: one not read yet may be.
    #[inline]
    fn may_be_domain(&self, s: u8) -> bool {
        match s {
            SHAPE_GATE => false,
            SHAPE_PARTIAL => self.part2,
            _ => true,
        }
    }

    fn classify_some(&mut self, limit: u64, clock: &mut Clock) {
        while !self.queues.is_empty() {
            let qs = std::mem::take(&mut self.queues);
            let mut nxt: Vec<VecDeque<u32>> = Vec::with_capacity(qs.len());
            let mut it = qs.into_iter();
            clock.add(1);
            while let Some(mut q) = it.next() {
                let ri = *q.front().unwrap();
                if !self.classify(ri, limit, clock) {
                    // the row stays at the head of its queue for the next turn
                    nxt.push(q);
                    nxt.extend(it);
                    self.queues = nxt;
                    return;
                }
                q.pop_front();
                if !q.is_empty() {
                    nxt.push(q);
                }
            }
            self.queues = nxt;
        }
    }

    /// After a turn of classifying: the index of the rows that are or may be
    /// domains, which is all that linking looks at. What each literal excludes
    /// among them is read again from the fuller exclusions. False: out of budget.
    fn index_domains(&mut self, clock: &mut Clock) -> bool {
        self.turn += 1;
        self.dom_arena.clear();
        if !self.index_dirty {
            return true;
        }
        let n = self.nrows();
        let nu = self.ulit.len();
        if !clock.take(items(2 * nu + n) + 2 * looks(self.row_lits.len()), self.det_limit) {
            return false;
        }
        let mut count = vec![0u32; nu + 1];
        let mut total = 0usize;
        for ri in 0..n as u32 {
            if self.may_be_domain(self.shape[ri as usize]) {
                let (a, b) = self.row_range(ri);
                total += b - a;
                for k in a..b {
                    count[self.row_lits[k] as usize] += 1;
                }
            }
        }
        let mut sum = 0u32;
        for c in count.iter_mut() {
            sum += *c;
            *c = sum;
        }
        let mut dr = vec![0u32; total];
        for ri in (0..n as u32).rev() {
            if self.may_be_domain(self.shape[ri as usize]) {
                let (a, b) = self.row_range(ri);
                for k in a..b {
                    let u = self.row_lits[k] as usize;
                    count[u] -= 1;
                    dr[count[u] as usize] = ri;
                }
            }
        }
        self.dr_start = count;
        self.dr = dr;
        self.index_dirty = false;
        true
    }

    // --------------------------------------------------------- linking the rows

    /// Round robin over the domains not yet linked, by shape and size, exact-one
    /// rows first; then the second pass over what the share cut short. False when
    /// the budget stopped it.
    fn link_new(&mut self, clock: &mut Clock) -> bool {
        if !clock.take(TICK + looks(self.classified.len()), self.det_limit) {
            return false;
        }
        let mut by_kind: Vec<((bool, u32), u32)> = Vec::new();
        for t in 0..self.classified.len() {
            let ri = self.classified[t];
            let s = self.shape[ri as usize];
            if self.may_be_domain(s) && !self.linked[ri as usize] {
                let (a, b) = self.row_range(ri);
                let kind = if self.in_order { (false, 0) } else { (s != SHAPE_EXACT, length_bucket(b - a)) };
                by_kind.push((kind, ri));
            }
        }
        if !clock.take(sorting(by_kind.len()) + looks(by_kind.len()), self.det_limit) {
            return false;
        }
        by_kind.sort_by_key(|e| e.0);
        let mut queues: Vec<VecDeque<u32>> = Vec::new();
        let mut i = 0;
        while i < by_kind.len() {
            let mut j = i;
            let mut q: Vec<u32> = Vec::new();
            while j < by_kind.len() && by_kind[j].0 == by_kind[i].0 {
                q.push(by_kind[j].1);
                j += 1;
            }
            if !self.in_order {
                shuffle(&mut q, &mut self.rng);
            }
            queues.push(q.into_iter().collect());
            i = j;
        }
        while !queues.is_empty() {
            let mut nxt: Vec<VecDeque<u32>> = Vec::with_capacity(queues.len());
            for mut q in queues {
                let ri = q.pop_front().unwrap();
                // a row read again on the way may stand here twice
                if !self.linked[ri as usize] {
                    self.linked[ri as usize] = true;
                    if !self.link(ri, clock) {
                        self.linked[ri as usize] = false; // for the next pass, if there is one
                        return false;
                    }
                }
                if !q.is_empty() {
                    nxt.push(q);
                }
            }
            queues = nxt;
        }
        let cut = std::mem::take(&mut self.cut);
        for (i, &ri) in cut.iter().enumerate() {
            if clock.over(self.det_limit) || !self.link(ri, clock) {
                self.cut.extend_from_slice(&cut[i..]);
                return false;
            }
        }
        true
    }

    /// The rows this domain differs from and the grids through it; a row that
    /// passes its share of the budget goes on `cut`. False when the budget stopped it.
    fn link(&mut self, ri: u32, clock: &mut Clock) -> bool {
        self.maybe_reset(clock);
        let (a, b) = self.row_range(ri);
        if !clock.take(TICK + looks(b - a), self.det_limit) {
            return false;
        }
        for k in a..b {
            let u = self.row_lits[k];
            if !self.failed[u as usize] && !self.link_lit(u, clock) {
                return false;
            }
        }
        let dom = self.domain(ri);
        let room = self.det_limit.saturating_sub(clock.now());
        self.row_cap = self.row_share.max(room / 4);
        let start = clock.now();
        match self.neighbours(ri, &dom, start, clock) {
            Neighbours::Stopped => false,
            Neighbours::Cut => {
                self.cut.push(ri);
                true
            }
            Neighbours::List(mut nbrs) => {
                if !nbrs.is_empty() && !self.grow(ri, &mut nbrs, start, clock) {
                    self.cut.push(ri);
                }
                true
            }
            Neighbours::Part(mut nbrs) => {
                if !nbrs.is_empty() {
                    self.grow(ri, &mut nbrs, start, clock);
                }
                self.cut.push(ri);
                true
            }
        }
    }

    /// The rows this one differs from, by what its own literals exclude, with the
    /// number of pairs: at least two literals on each side, at most three exclusions
    /// a literal on average, no literal in common.
    ///
    /// A scan that cannot end within the row's share is not begun: it would be cut
    /// and leave nothing. The rows read so far may still give a grid, so the scan goes
    /// over them instead, when that fits.
    fn neighbours(&mut self, ri: u32, dom: &[u32], start: u64, clock: &mut Clock) -> Neighbours {
        if !clock.take(TICK + looks(dom.len()), self.det_limit) {
            return Neighbours::Stopped;
        }
        let excluded: usize = dom.iter().map(|&l| self.dom_len[l as usize] as usize).sum();
        if !clock.take(looks(excluded), self.det_limit) {
            return Neighbours::Stopped;
        }
        // each row on the way costs a tick
        let mut walk = 0u64;
        for &l in dom {
            let (a, b) = self.dom_range(l);
            for k in a..b {
                let m = self.dom_arena[k] as usize;
                walk += (self.dr_start[m + 1] - self.dr_start[m]) as u64;
            }
        }
        let mut known = false;
        if walk > self.row_cap {
            if self.classified.len() >= self.nrows() {
                return Neighbours::Cut; // every row has been read: nothing shorter
            }
            if !self.index_known(clock) {
                return Neighbours::Stopped;
            }
            if !clock.take(looks(excluded), self.det_limit) {
                return Neighbours::Stopped;
            }
            let mut walk_known = 0u64;
            for &l in dom {
                let (a, b) = self.dom_range(l);
                for k in a..b {
                    let m = self.dom_arena[k] as usize;
                    walk_known += (self.kr_start[m + 1] - self.kr_start[m]) as u64;
                }
            }
            if walk_known > self.row_cap {
                return Neighbours::Cut;
            }
            known = true;
        }
        let (starts, rows_of) = if known {
            (std::mem::take(&mut self.kr_start), std::mem::take(&mut self.kr))
        } else {
            (std::mem::take(&mut self.dr_start), std::mem::take(&mut self.dr))
        };
        let stamp = self.new_stamp();
        let (ra, rb) = self.row_range(ri);
        for k in ra..rb {
            let u = self.row_lits[k] as usize;
            self.own_tag[u] = stamp;
        }
        let mut touched: Vec<u32> = Vec::new();
        let mut result = None;
        'scan: for &l in dom {
            if clock.over(self.det_limit) {
                result = Some(Neighbours::Stopped);
                break;
            }
            if clock.now() - start > self.row_cap {
                result = Some(Neighbours::Cut);
                break;
            }
            let (a, b) = self.dom_range(l);
            for k in a..b {
                let m = self.dom_arena[k] as usize;
                let first = self.m_tag[m] != stamp;
                self.m_tag[m] = stamp;
                let (h0, h1) = (starts[m] as usize, starts[m + 1] as usize);
                if !clock.take(2 + TICK * (h1 - h0) as u64, self.det_limit) {
                    result = Some(Neighbours::Stopped);
                    break 'scan;
                }
                for h in h0..h1 {
                    let v = rows_of[h] as usize;
                    if v == ri as usize {
                        continue;
                    }
                    if self.pc[v] == 0 {
                        touched.push(v as u32);
                        self.ls[v] = 0;
                        self.ms[v] = 0;
                        self.last_l[v] = NONE;
                    }
                    self.pc[v] += 1;
                    if self.last_l[v] != l {
                        self.last_l[v] = l;
                        self.ls[v] += 1;
                    }
                    if first {
                        self.ms[v] += 1;
                    }
                }
            }
        }
        if known {
            self.kr_start = starts;
            self.kr = rows_of;
        } else {
            self.dr_start = starts;
            self.dr = rows_of;
        }
        // the counters go back to zero whatever happened above
        clock.add(looks(touched.len()));
        let mut out: Vec<(u32, u32)> = Vec::new();
        for &t in &touched {
            let v = t as usize;
            let (pc, ls, ms) = (self.pc[v], self.ls[v], self.ms[v]);
            self.pc[v] = 0;
            if result.is_some() || pc < 2 || ls < 2 || ms < 2 || pc > 3 * ls.min(ms) {
                continue;
            }
            let (a, b) = self.row_range(t);
            clock.add(looks(b - a));
            if self.row_lits[a..b].iter().any(|&u| self.own_tag[u as usize] == stamp) {
                continue;
            }
            out.push((t, pc));
        }
        match result {
            Some(r) => r,
            None if known => Neighbours::Part(out),
            None => Neighbours::List(out),
        }
    }

    /// The index of the rows read so far that are domains, made once a turn. False:
    /// out of budget.
    fn index_known(&mut self, clock: &mut Clock) -> bool {
        if self.kr_turn == self.turn {
            return true;
        }
        let nu = self.ulit.len();
        if !clock.take(items(2 * nu) + looks(self.classified.len()), self.det_limit) {
            return false;
        }
        let mut count = vec![0u32; nu + 1];
        let mut total = 0usize;
        for t in 0..self.classified.len() {
            let ri = self.classified[t];
            if self.may_be_domain(self.shape[ri as usize]) {
                let (a, b) = self.row_range(ri);
                if !clock.take(2 * looks(b - a), self.det_limit) {
                    return false;
                }
                total += b - a;
                for k in a..b {
                    count[self.row_lits[k] as usize] += 1;
                }
            }
        }
        let mut sum = 0u32;
        for c in count.iter_mut() {
            sum += *c;
            *c = sum;
        }
        let mut kr = vec![0u32; total];
        for t in 0..self.classified.len() {
            let ri = self.classified[t];
            if self.may_be_domain(self.shape[ri as usize]) {
                let (a, b) = self.row_range(ri);
                for k in a..b {
                    let u = self.row_lits[k] as usize;
                    count[u] -= 1;
                    kr[count[u] as usize] = ri;
                }
            }
        }
        self.kr_start = count;
        self.kr = kr;
        self.kr_turn = self.turn;
        true
    }

    /// Whether row `v` joins the `n` rows of the grid in hand: no literal in common,
    /// at least two literals on each side against the first row, an exclusion with
    /// at least four in five of the others (all of them below five), and sparse
    /// exclusions throughout.
    fn fits(&mut self, v: u32, n: usize, clock: &mut Clock) -> bool {
        self.maybe_reset(clock);
        let (ra, rb) = self.row_range(v);
        if !clock.take(TICK + looks(rb - ra) + items(n), self.det_limit) {
            return false;
        }
        let mut lits = self.domain(v);
        if lits.len() < 2 {
            return false;
        }
        // what a row is may change when a read cut short is done again
        if self.read_again && lits.iter().any(|&m| self.short[m as usize]) {
            if self.shape[v as usize] == SHAPE_PARTIAL {
                self.partial_rows -= 1;
            }
            self.shape[v as usize] = SHAPE_UNKNOWN;
        }
        if self.shape[v as usize] == SHAPE_UNKNOWN {
            for &m in &lits {
                if !self.read_lit(m, self.det_limit, clock) {
                    return false;
                }
            }
            lits = self.domain(v);
            let Some(s) = self.row_shape(&lits, clock) else { return false };
            self.set_shape(v, s);
        }
        if !self.may_be_domain(self.shape[v as usize]) {
            return false;
        }
        for &m in &lits {
            if !self.link_lit(m, clock) {
                return false;
            }
        }
        lits.retain(|&m| !self.failed[m as usize]);
        let g = self.grow_stamp;
        if lits.iter().any(|&m| self.lit_tag[m as usize] == g) {
            return false;
        }
        if self.edges.len() < n {
            self.edges.resize(n, 0);
            self.ours.resize(n, 0);
            self.theirs.resize(n, 0);
            self.last_m.resize(n, NONE);
        }
        for r in 0..n {
            self.edges[r] = 0;
            self.ours[r] = 0;
            self.theirs[r] = 0;
            self.last_m[r] = NONE;
        }
        let call = self.new_stamp();
        for &m in &lits {
            let (a, b) = self.dom_range(m);
            if !clock.take(TICK + looks(b - a), self.det_limit) {
                return false;
            }
            let token = self.new_stamp();
            for k in a..b {
                let x = self.dom_arena[k];
                self.e_tag[x as usize] = token;
                if self.lit_tag[x as usize] == g {
                    self.edge(x, m, call);
                }
            }
            if self.rev_tag[m as usize] == g {
                let mut p = self.rev_head[m as usize];
                while p != NONE {
                    let x = self.rev_val[p as usize];
                    clock.add(looks(1));
                    if self.e_tag[x as usize] != token {
                        self.edge(x, m, call);
                    }
                    p = self.rev_next[p as usize];
                }
            }
        }
        let mut met = 0;
        for r in 0..n {
            if self.edges[r] > 0 {
                if self.edges[r] > 3 * self.ours[r].min(self.theirs[r]) {
                    return false;
                }
                met += 1;
            }
        }
        let need = if n < 5 { n } else { (4 * n + 4) / 5 };
        met >= need && self.ours[0] >= 2 && self.theirs[0] >= 2
    }

    /// An exclusion between literal `m` of the new row and literal `x` of the grid.
    #[inline]
    fn edge(&mut self, x: u32, m: u32, call: u32) {
        let r = self.lit_row[x as usize] as usize;
        self.edges[r] += 1;
        if self.last_m[r] != m {
            self.last_m[r] = m;
            self.ours[r] += 1;
        }
        if self.x_tag[x as usize] != call {
            self.x_tag[x as usize] = call;
            self.theirs[r] += 1;
        }
    }

    /// Row `ri` joins the grid in hand at position `pos`.
    fn add_member(&mut self, ri: u32, pos: u32, clock: &mut Clock) {
        let g = self.grow_stamp;
        let (ra, rb) = self.row_range(ri);
        clock.add(looks(rb - ra));
        for k in ra..rb {
            let m = self.row_lits[k];
            if self.failed[m as usize] {
                continue;
            }
            self.lit_tag[m as usize] = g;
            self.lit_row[m as usize] = pos;
            let (a, b) = self.dom_range(m);
            clock.add(looks(b - a));
            for j in a..b {
                let y = self.dom_arena[j] as usize;
                let head = if self.rev_tag[y] == g { self.rev_head[y] } else { NONE };
                self.rev_tag[y] = g;
                self.rev_head[y] = self.rev_val.len() as u32;
                self.rev_val.push(m);
                self.rev_next.push(head);
            }
        }
    }

    /// Maximal sets of rows through this row, each seeded by a neighbour no set
    /// holds yet and no whole grid shares with it, every neighbour tried in each.
    /// False when the row's share of the budget cut it short.
    fn grow(&mut self, ri: u32, nbrs: &mut [(u32, u32)], start: u64, clock: &mut Clock) -> bool {
        clock.add(TICK + sorting(nbrs.len()));
        nbrs.sort_unstable_by_key(|&(v, count)| (Reverse(count), v));
        let w = self.new_stamp();
        for gi in 0..self.row_groups[ri as usize].len() {
            let g = self.row_groups[ri as usize][gi] as usize;
            clock.add(looks(self.groups[g].len()));
            for t in 0..self.groups[g].len() {
                let a = self.groups[g][t] as usize;
                self.cover_tag[a] = w;
            }
        }
        let mut next = 0;
        for _ in 0..MAX_CLIQUES {
            while next < nbrs.len() && self.cover_tag[nbrs[next].0 as usize] == w {
                next += 1;
                clock.add(1);
            }
            if next == nbrs.len() {
                break;
            }
            if clock.over(self.det_limit) {
                return false;
            }
            let seed = nbrs[next].0;
            let (ra, rb) = self.row_range(ri);
            for k in ra..rb {
                let u = self.row_lits[k];
                if !self.failed[u as usize] && !self.link_lit(u, clock) {
                    return false;
                }
            }
            self.grow_stamp = self.new_stamp();
            self.rev_val.clear();
            self.rev_next.clear();
            self.add_member(ri, 0, clock);
            let mut members: Vec<u32> = vec![ri];
            let mut whole = true;
            let mut fails = 0;
            for t in std::iter::once(seed).chain(nbrs.iter().map(|e| e.0).filter(|&u| u != seed)) {
                if clock.over(self.det_limit) || clock.now() - start > self.row_cap {
                    whole = false;
                    break;
                }
                if !self.fits(t, members.len(), clock) {
                    // a row just read and found to be no domain is no failed try:
                    // many of them may come before the rows of the grid
                    if !self.may_be_domain(self.shape[t as usize]) {
                        continue;
                    }
                    fails += 1;
                    if fails >= MAX_FAILS {
                        break;
                    }
                    continue;
                }
                fails = 0;
                self.add_member(t, members.len() as u32, clock);
                members.push(t);
            }
            self.cover_tag[seed as usize] = w;
            clock.add(looks(members.len()));
            for &m in &members {
                self.cover_tag[m as usize] = w;
            }
            if members.len() >= 3 && !self.store_grid(&members, clock) {
                return false;
            }
            if !whole {
                return false;
            }
            let g = self.groups.len() as u32;
            for &m in &members {
                self.row_groups[m as usize].push(g);
            }
            self.groups.push(members);
        }
        true
    }

    /// Reads a grid's columns unless these rows have them. False: out of budget.
    fn store_grid(&mut self, members: &[u32], clock: &mut Clock) -> bool {
        let mut key = members.to_vec();
        if !clock.take(TICK + sorting(key.len()) + looks(key.len()), self.det_limit) {
            return false;
        }
        key.sort_unstable();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &r in &key {
            h = (h ^ r as u64).wrapping_mul(0x0000_0100_0000_01B3);
        }
        let known = self.grid_seen.get(&h).copied();
        let same = known.filter(|&i| self.grids[i as usize].rows == key);
        // after the short reads a grid with the same rows may have longer columns
        if same.is_some() && self.hub < HUB {
            return true;
        }
        let Some((cols, private)) = self.read_columns(members, clock) else { return false };
        if let Some(i) = same {
            self.grids[i as usize] = GridRec { rows: key, cols, private };
            return true;
        }
        if known.is_none() {
            self.grid_seen.insert(h, self.grids.len() as u32);
        }
        self.grids.push(GridRec { rows: key, cols, private });
        true
    }

    /// The columns of a grid whose rows are fixed: a clique cover of the literals,
    /// one literal per row in each clique. From a literal the next row is the one
    /// with the fewest literals left that exclude the clique, and on a tie the
    /// literal that leaves the others the most. None when the budget stopped it.
    fn read_columns(&mut self, members: &[u32], clock: &mut Clock) -> Option<(Vec<Vec<u32>>, u32)> {
        let stamp = self.new_stamp();
        // position in the grid of each literal; a row's literals stay together
        let mut lits: Vec<u32> = Vec::new();
        let mut at: Vec<u32> = Vec::new();
        for (i, &ri) in members.iter().enumerate() {
            let (a, b) = self.row_range(ri);
            if !clock.take(looks(b - a), self.det_limit) {
                return None;
            }
            for u in self.domain(ri) {
                self.g_tag[u as usize] = stamp;
                self.g_pos[u as usize] = lits.len() as u32;
                lits.push(u);
                at.push(i as u32);
            }
        }
        let nlits = lits.len();
        // the pairs, each way, as lists by position
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        if !clock.take(items(6 * nlits), self.det_limit) {
            return None;
        }
        let mut start = vec![0u32; nlits + 1];
        for p in 0..nlits {
            let u = lits[p];
            if !self.link_lit(u, clock) {
                return None;
            }
            let (a, b) = self.dom_range(u);
            if !clock.take(TICK + 2 * looks(b - a), self.det_limit) {
                return None;
            }
            for k in a..b {
                let m = self.dom_arena[k];
                if self.g_tag[m as usize] == stamp {
                    let q = self.g_pos[m as usize];
                    if at[q as usize] != at[p] {
                        pairs.push((p as u32, q));
                        start[p + 1] += 1;
                        start[q as usize + 1] += 1;
                    }
                }
            }
        }
        for p in 0..nlits {
            start[p + 1] += start[p];
        }
        let total = start[nlits] as usize;
        let mut list = vec![0u32; total];
        let mut fill = start.clone();
        for &(p, q) in &pairs {
            list[fill[p as usize] as usize] = q;
            fill[p as usize] += 1;
            list[fill[q as usize] as usize] = p;
            fill[q as usize] += 1;
        }
        // a pair read from both sides stays once
        let mut seen = vec![NONE; nlits];
        let mut end = vec![0u32; nlits];
        for p in 0..nlits {
            let (a, b) = (start[p] as usize, start[p + 1] as usize);
            let mut w = a;
            for k in a..b {
                let q = list[k];
                if seen[q as usize] != p as u32 {
                    seen[q as usize] = p as u32;
                    list[w] = q;
                    w += 1;
                }
            }
            end[p] = w as u32;
        }
        drop(pairs);
        drop(fill);
        let mut cover = Cover {
            at,
            start,
            end,
            list,
            col: vec![NONE; nlits],
            in_flat: vec![false; nlits],
            marked: vec![NONE; nlits],
            mark: 0,
            row_n: vec![0; members.len()],
            flat: Vec::new(),
            rows_here: Vec::new(),
            limit: self.det_limit,
        };
        let mut fresh = 0u32;
        let mut first: Vec<u32> = Vec::new();
        let mut best: Vec<u32> = Vec::new();
        let mut cur: Vec<u32> = Vec::new();
        // twice over the literals: a literal with two largest cliques waits the first
        // time, so that a column is taken from a literal that has no other, and a
        // diagonal as long as the columns does not break them up
        for round in 0..2 {
            for p in 0..nlits as u32 {
                if cover.col[p as usize] != NONE {
                    continue;
                }
                // the row a clique from here takes next: of its literals the one
                // that gives the largest clique, the first on a tie
                if !cover.first_options(p, &mut first, clock) {
                    return None;
                }
                best.clear();
                let mut tie = false;
                if first.len() <= 1 {
                    if !cover.clique(p, first.first().copied(), &mut best, clock) {
                        return None;
                    }
                } else {
                    for &x in &first {
                        if !cover.clique(p, Some(x), &mut cur, clock) {
                            return None;
                        }
                        if cur.len() > best.len() {
                            std::mem::swap(&mut cur, &mut best);
                            tie = false;
                        } else if cur.len() == best.len() {
                            tie = true;
                        }
                    }
                }
                if tie && round == 0 {
                    continue;
                }
                for &q in &best {
                    cover.col[q as usize] = fresh;
                }
                fresh += 1;
            }
        }
        // a row of k literals needs k columns; with more than the longest row's, a
        // clique may have taken literals of the true columns (a diagonal next to them)
        let mut per_row = vec![0u32; members.len()];
        for &r in &cover.at {
            per_row[r as usize] += 1;
        }
        let longest = per_row.iter().copied().max().unwrap_or(0);
        if !clock.take(items(fresh as usize + members.len()) + looks(nlits), self.det_limit) {
            return None;
        }
        let Cover { start, end, list, mut col, .. } = cover;
        let fresh = if fresh > longest {
            self.absorb(&mut col, fresh, &start, &end, &list, clock)?
        } else {
            fresh
        };
        // the columns of two or more literals; one literal is a value of its row's own
        if !clock.take(items(fresh as usize) + looks(nlits) + sorting(nlits), self.det_limit) {
            return None;
        }
        let mut by_label: Vec<Vec<u32>> = vec![Vec::new(); fresh as usize];
        for p in 0..nlits {
            by_label[col[p] as usize].push(lits[p]);
        }
        let mut cols: Vec<Vec<u32>> = Vec::new();
        let mut private = 0u32;
        for mut c in by_label {
            if c.len() >= 2 {
                c.sort_unstable();
                cols.push(c);
            } else {
                private += 1;
            }
        }
        Some((cols, private))
    }

    /// A clique cover with fewer cliques: a clique goes when each of its literals
    /// excludes all of some other clique, and then joins it, smallest cliques first,
    /// until a round changes nothing, for at most MAX_ABSORB rounds. Returns the
    /// number of cliques, renumbered. None when the budget stopped it.
    fn absorb(&mut self, col: &mut [u32], cliques: u32, start: &[u32], end: &[u32], list: &[u32], clock: &mut Clock) -> Option<u32> {
        let n = cliques as usize;
        let mut size = vec![0u32; n];
        for &c in col.iter() {
            size[c as usize] += 1;
        }
        let mut members: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (p, &c) in col.iter().enumerate() {
            members[c as usize].push(p as u32);
        }
        // how many of a clique the literal in hand excludes: `mark` says the count is
        // this literal's, this time
        let mut count = vec![0u32; n];
        let mut mark = vec![0u32; n];
        let mut token = 0u32;
        let mut order: Vec<u32> = (0..n as u32).collect();
        let mut target: Vec<u32> = Vec::new();
        for _ in 0..MAX_ABSORB {
            if !clock.take(TICK + sorting(n), self.det_limit) {
                return None;
            }
            order.sort_by_key(|&c| (size[c as usize], c));
            let mut changed = false;
            for &c in &order {
                if size[c as usize] == 0 {
                    continue;
                }
                target.clear();
                for &p in &members[c as usize] {
                    let (a, b) = (start[p as usize] as usize, end[p as usize] as usize);
                    if !clock.take(TICK + 2 * looks(b - a), self.det_limit) {
                        return None;
                    }
                    let mut to = NONE;
                    token += 1;
                    for &q in &list[a..b] {
                        let k = col[q as usize] as usize;
                        if k as u32 == c {
                            continue;
                        }
                        if mark[k] != token {
                            mark[k] = token;
                            count[k] = 0;
                        }
                        count[k] += 1;
                        // the first clique it fills; the larger clique goes first on a tie
                        if count[k] == size[k] && (to == NONE || (size[k], Reverse(k as u32)) > (size[to as usize], Reverse(to))) {
                            to = k as u32;
                        }
                    }
                    if to == NONE {
                        target.clear();
                        break;
                    }
                    target.push(to);
                }
                if target.is_empty() {
                    continue;
                }
                let moved = std::mem::take(&mut members[c as usize]);
                for (i, &p) in moved.iter().enumerate() {
                    let t = target[i];
                    col[p as usize] = t;
                    size[t as usize] += 1;
                    members[t as usize].push(p);
                }
                size[c as usize] = 0;
                changed = true;
            }
            if !changed {
                break;
            }
        }
        // the cliques that stay, numbered in order
        let mut new_label = vec![NONE; n];
        let mut next = 0u32;
        for p in 0..col.len() {
            let c = col[p] as usize;
            if new_label[c] == NONE {
                new_label[c] = next;
                next += 1;
            }
            col[p] = new_label[c];
        }
        Some(next)
    }

    // ------------------------------------------------------------ the whole read

    /// A short slice of the detection budget reads rows with a small limit on a
    /// literal's propagation, which is what keeps a formula full of gates affordable.
    /// A read cut short keeps the near exclusions, so a long column comes out in parts.
    /// So then the rows of the grids found so far are read in full, biggest first, and
    /// a grid grows to what it is: that has up to three quarters of what is left, and
    /// what it did not get to goes back with the rows not read yet. Then the next
    /// short slice, and so on. With every row read this way, what is left goes to all
    /// rows with a read that was cut short.
    fn run(&mut self, clock: &mut Clock) {
        if self.nrows() == 0 {
            return;
        }
        let end = self.det_limit;
        let slice = (end.saturating_sub(clock.now()) / SHORT_SLICES).max(1);
        let mut unread = std::mem::take(&mut self.queues);
        loop {
            let start = clock.now();
            self.hub = SHORT_READ;
            self.read_again = false;
            self.in_order = false;
            self.queues = unread;
            self.det_limit = end.min(clock.now() + slice);
            self.read_pass(true, clock);
            unread = std::mem::take(&mut self.queues);
            let all_read = !self.stopped;
            self.stopped = false;
            self.det_limit = end;
            if all_read && !self.any_short {
                return;
            }
            // a step that does not fit in what is left charges nothing: that is the end
            if clock.over(end) || self.stamps_out || clock.now() == start {
                self.stopped = true;
                return;
            }
            if self.any_short {
                self.hub = HUB;
                self.read_again = true;
                // the short slices that are left keep a part of the budget
                self.det_limit = clock.now() + end.saturating_sub(clock.now()) / 4 * 3;
                if self.read_short_again(true, clock) {
                    self.read_pass(false, clock);
                    let rest = std::mem::take(&mut self.queues);
                    unread.extend(rest);
                }
                self.stopped = false;
                self.det_limit = end;
            }
            if clock.over(end) || self.stamps_out {
                self.stopped = true;
                return;
            }
            if all_read {
                break;
            }
        }
        self.hub = HUB;
        self.read_again = true;
        if !self.read_short_again(false, clock) {
            self.stopped = true;
            return;
        }
        // what the full reads did not get to may have no short read left by now
        self.queues.extend(unread);
        self.read_pass(true, clock);
    }

    /// The rows holding a literal whose read was cut short go back to unread and
    /// unlinked, in order: those of the grids so far, biggest first, or all of them.
    /// Their literals are read again as they come up. The grids so far stay; which
    /// rows a whole grid shares starts over. False: out of budget.
    fn read_short_again(&mut self, grids_only: bool, clock: &mut Clock) -> bool {
        let n = self.nrows();
        if !clock.take(items(3 * n) + looks(self.classified.len()) + sorting(self.grids.len()), self.det_limit) {
            return false;
        }
        let mut queued = vec![false; n];
        let mut again: Vec<u32> = Vec::new();
        let mut order: Vec<u32> = (0..self.grids.len() as u32).collect();
        if grids_only {
            order.sort_by_key(|&g| (Reverse(self.grids[g as usize].rows.len()), g));
        } else {
            order.clear();
        }
        let every: Vec<u32> = if grids_only { Vec::new() } else { (0..n as u32).collect() };
        let mut lists: Vec<&[u32]> = order.iter().map(|&g| self.grids[g as usize].rows.as_slice()).collect();
        lists.push(&every);
        for list in lists {
            if !clock.take(TICK + items(list.len()), self.det_limit) {
                return false;
            }
            for &ri in list {
                if queued[ri as usize] {
                    continue;
                }
                queued[ri as usize] = true;
                let (a, b) = (self.row_off[ri as usize] as usize, self.row_off[ri as usize + 1] as usize);
                if !clock.take(looks(b - a), self.det_limit) {
                    return false;
                }
                if self.row_lits[a..b].iter().any(|&u| self.short[u as usize]) {
                    again.push(ri);
                }
            }
        }
        for &ri in &again {
            if self.shape[ri as usize] == SHAPE_PARTIAL {
                self.partial_rows -= 1;
            }
            self.shape[ri as usize] = SHAPE_UNKNOWN;
            self.linked[ri as usize] = false;
        }
        // a row called a gate's or a partial one on short reads may be neither: it
        // counts as not read yet, and is read when a grid comes to it
        if !clock.take(looks(self.row_lits.len()), self.det_limit) {
            return false;
        }
        for ri in 0..n {
            let s = self.shape[ri];
            if s == SHAPE_GATE || s == SHAPE_PARTIAL {
                let (a, b) = (self.row_off[ri] as usize, self.row_off[ri + 1] as usize);
                if self.row_lits[a..b].iter().any(|&u| self.short[u as usize]) {
                    if s == SHAPE_PARTIAL {
                        self.partial_rows -= 1;
                    }
                    self.shape[ri] = SHAPE_UNKNOWN;
                }
            }
        }
        for v in self.row_groups.iter_mut() {
            v.clear();
        }
        self.groups.clear();
        self.cut.clear();
        let shape = &self.shape;
        self.classified.retain(|&ri| shape[ri as usize] != SHAPE_UNKNOWN);
        self.index_dirty = true;
        self.part2 = false;
        self.in_order = true;
        self.queues = if again.is_empty() { Vec::new() } else { vec![again.into_iter().collect()] };
        true
    }

    /// Turns of classifying and linking within the budget, then the partial rows
    /// with what is left, if `partial` says so. While the rows of the grids read in
    /// full they wait: rows not read at all come before them.
    fn read_pass(&mut self, partial: bool, clock: &mut Clock) {
        let begin = clock.now();
        let mut limit = begin + self.det_limit.saturating_sub(begin) / 5 * 2;
        loop {
            let before = clock.now();
            self.classify_some(limit, clock);
            let stuck = !self.queues.is_empty() && clock.now() == before;
            if stuck || !self.index_domains(clock) || !self.link_new(clock) {
                self.stopped = true;
                return;
            }
            if self.queues.is_empty() {
                break;
            }
            // a turn costs an index of the rows: the last bit of the budget is not worth it
            let left = self.det_limit.saturating_sub(clock.now());
            if clock.over(self.det_limit) || left < self.det_limit.saturating_sub(begin) / LAST_TURN {
                self.stopped = true;
                return;
            }
            // what is left goes half to classifying and half to linking
            limit = clock.now() + (left / 2).max(1);
        }
        if partial && self.partial_rows > 0 {
            self.part2 = true;
            self.index_dirty = true;
            if !self.index_domains(clock) || !self.link_new(clock) {
                self.stopped = true;
            }
        }
    }

    /// A grid whose rows another, larger grid holds is a part of it, from a read the
    /// share cut short or a neighbour tried in another order. Looks at the larger
    /// grids holding the grid's rarest row, up to MAX_PART_TESTS of them. Returns the
    /// grids that stay, the biggest first, or what it could not look at in any order.
    fn drop_parts(&mut self, clock: &mut Clock) -> Vec<u32> {
        let n = self.grids.len();
        let mut order: Vec<u32> = (0..n as u32).filter(|&g| !self.grids[g as usize].cols.is_empty()).collect();
        if !clock.take(TICK + looks(n) + sorting(n), self.budget) {
            return order;
        }
        order.sort_by_key(|&g| Reverse(self.grids[g as usize].rows.len()));
        let mut holders: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut kept: Vec<u32> = Vec::with_capacity(order.len());
        let mut room = true;
        for &g in &order {
            let k = &self.grids[g as usize];
            if room && !clock.take(2 * TICK * (1 + k.rows.len() as u64), self.budget) {
                room = false; // it stays: the check decides what is returned
            }
            if !room {
                kept.push(g);
                continue;
            }
            let rarest = k.rows.iter().copied().min_by_key(|r| holders.get(r).map_or(0, |v| v.len())).unwrap();
            let mut part = false;
            if let Some(list) = holders.get(&rarest) {
                for &o in list.iter().take(MAX_PART_TESTS) {
                    let big = &self.grids[o as usize];
                    if big.rows.len() <= k.rows.len() {
                        continue;
                    }
                    clock.add(TICK + looks(big.rows.len()));
                    let mut it = big.rows.iter();
                    if k.rows.iter().all(|r| it.any(|x| x == r)) {
                        part = true;
                        break;
                    }
                }
            }
            if !part {
                kept.push(g);
                for &r in &k.rows {
                    holders.entry(r).or_default().push(g);
                }
            }
        }
        kept
    }

    /// The grids, checked, biggest first, within the budget and MAX_GRIDS.
    fn finish(&mut self, clock: &mut Clock) -> Vec<Grid> {
        if self.grids.is_empty() || clock.flagged() {
            return Vec::new();
        }
        // what the check reads again, it reads in full, and a read only cut short stays
        self.hub = HUB;
        self.read_again = false;
        self.checking = true;
        if !clock.take(items(self.ulit.len()), self.budget) {
            return Vec::new();
        }
        self.late_failed = vec![false; self.ulit.len()];
        let mut chosen = self.drop_parts(clock);
        self.kept = chosen.len();
        if !clock.take(TICK + 2 * sorting(chosen.len()), self.budget) {
            return Vec::new();
        }
        // past the cap the grids that are mostly private values go first; the rest
        // are then in order of size
        let core = |k: &GridRec| k.rows.len() >= 3 && k.cols.len() >= 3 && k.private as usize <= k.cols.len();
        if chosen.len() > MAX_GRIDS {
            chosen.sort_by_key(|&g| {
                let k = &self.grids[g as usize];
                (!core(k), Reverse(k.rows.len()), Reverse(k.cols.len()), k.private, g)
            });
            chosen.truncate(MAX_GRIDS);
        }
        chosen.sort_by_key(|&g| {
            let k = &self.grids[g as usize];
            (Reverse(k.rows.len()), Reverse(k.cols.len()), k.private, g)
        });
        let Some((table, chosen, slot)) = self.pair_table(chosen, clock) else { return Vec::new() };
        let table = Arc::new(table);
        let mut out = Vec::with_capacity(chosen.len());
        for &g in &chosen {
            match self.check_grid(g as usize, &slot, &table, clock) {
                Checked::Good(grid) => out.push(grid),
                Checked::Dropped => {}
                Checked::Wrong => self.rejected += 1,
                Checked::Stopped => {
                    self.stopped = true;
                    break;
                }
            }
        }
        out
    }

    /// The input pair clauses among the chosen grids' column literals. When they
    /// would take more than half of what is left, the smaller grids go. Returns the
    /// table, the grids it covers and each literal's slot.
    fn pair_table(&mut self, chosen: Vec<u32>, clock: &mut Clock) -> Option<(PairTable, Vec<u32>, Vec<u32>)> {
        let nu = self.ulit.len();
        if !clock.take(items(6 * nu), self.budget) {
            return None;
        }
        let mut slot = vec![NONE; nu];
        let room = self.budget.saturating_sub(clock.now()).saturating_mul(TICK);
        let mut cost_of_table = 0u64;
        let mut covered = 0;
        for &g in &chosen {
            let k = &self.grids[g as usize];
            let seen: usize = k.cols.iter().map(|c| c.len()).sum();
            if !clock.take(TICK + items(k.cols.len()) + 2 * looks(seen), self.budget) {
                break;
            }
            for c in &k.cols {
                for &u in c {
                    if slot[u as usize] == NONE {
                        slot[u as usize] = 0;
                        let x = lx(self.ulit[u as usize]);
                        let d = (self.live.bin_start[x + 1] - self.live.bin_start[x]) as usize;
                        cost_of_table += 2 * TICK + 2 * looks(d) + sorting(d);
                    }
                }
            }
            if cost_of_table > room / 2 {
                break;
            }
            covered += 1;
        }
        let chosen: Vec<u32> = chosen[..covered].to_vec();
        if chosen.is_empty() {
            return None;
        }
        // the slots of what the chosen grids hold; a grid that did not fit is out
        slot.fill(NONE);
        for &g in &chosen {
            for c in &self.grids[g as usize].cols {
                for &u in c {
                    slot[u as usize] = 0;
                }
            }
        }
        let mut lits: Vec<i32> = Vec::new();
        for u in 0..nu {
            if slot[u] != NONE {
                slot[u] = lits.len() as u32;
                lits.push(self.ulit[u]);
            }
        }
        let n = lits.len();
        let mut start = vec![0u32; n + 1];
        // twice over the literals' pair clauses: a pair sits under its smaller literal
        let mut cost = 0u64;
        for p in 0..n {
            let a = lits[p];
            let (b0, b1) = (self.live.bin_start[lx(a)] as usize, self.live.bin_start[lx(a) + 1] as usize);
            cost += TICK + looks(b1 - b0);
            for k in b0..b1 {
                let b = -self.live.bin_lit[k];
                if self.live.bin_id[k] != 0 && a < b {
                    let ub = self.uid[lx(b)];
                    if ub != NONE && slot[ub as usize] != NONE {
                        start[p + 1] += 1;
                    }
                }
            }
        }
        clock.add(2 * cost);
        for p in 0..n {
            start[p + 1] += start[p];
        }
        let mut ent = vec![(0u32, 0u32); start[n] as usize];
        let mut fill = start.clone();
        for p in 0..n {
            let a = lits[p];
            let (b0, b1) = (self.live.bin_start[lx(a)] as usize, self.live.bin_start[lx(a) + 1] as usize);
            for k in b0..b1 {
                let b = -self.live.bin_lit[k];
                if self.live.bin_id[k] != 0 && a < b {
                    let ub = self.uid[lx(b)];
                    if ub != NONE && slot[ub as usize] != NONE {
                        ent[fill[p] as usize] = (slot[ub as usize], self.live.bin_id[k]);
                        fill[p] += 1;
                    }
                }
            }
        }
        // each list sorted; of the same pair twice the first clause stays
        let mut w = 0usize;
        let mut new_start = vec![0u32; n + 1];
        for p in 0..n {
            let (a, b) = (start[p] as usize, start[p + 1] as usize);
            clock.add(sorting(b - a));
            ent[a..b].sort_unstable();
            let w0 = w;
            for i in a..b {
                if w == w0 || ent[w - 1].0 != ent[i].0 {
                    ent[w] = ent[i];
                    w += 1;
                }
            }
            new_start[p + 1] = w as u32;
        }
        ent.truncate(w);
        ent.shrink_to_fit();
        Some((PairTable { lits, start: new_start, ent }, chosen, slot))
    }

    /// The exact check, against the input alone.
    fn check_grid(&mut self, g: usize, slot: &[u32], table: &Arc<PairTable>, clock: &mut Clock) -> Checked {
        let budget = self.budget;
        let row_ids = std::mem::take(&mut self.grids[g].rows);
        let cols_in = std::mem::take(&mut self.grids[g].cols);
        // each row, once per read
        for &ri in &row_ids {
            match self.check_row(ri, clock) {
                Err(()) => return Checked::Stopped,
                Ok(ROW_BAD) => return Checked::Wrong,
                Ok(_) => {}
            }
            // a row left with one literal says that literal is true: another matter
            if self.domain(ri).len() < 2 {
                return Checked::Dropped;
            }
        }
        // each literal's row in this grid; a literal two rows share may be in no column
        let rows_stamp = self.new_stamp();
        for (p, &ri) in row_ids.iter().enumerate() {
            let (a, b) = self.row_range(ri);
            if !clock.take(looks(b - a), budget) {
                return Checked::Stopped;
            }
            for k in a..b {
                let u = self.row_lits[k] as usize;
                self.g_pos[u] = if self.g_tag[u] == rows_stamp { NONE } else { p as u32 };
                self.g_tag[u] = rows_stamp;
            }
        }
        let in_col = self.new_stamp();
        if !clock.take(items(row_ids.len()), budget) {
            return Checked::Stopped;
        }
        let mut row_mark = vec![0u32; row_ids.len()];
        let mut cols: Vec<Col> = Vec::with_capacity(cols_in.len());
        let mut col_no = 0u32;
        let mut found: Vec<bool> = Vec::new();
        for c in &cols_in {
            col_no += 1;
            // a literal found to fail after its column was read is no value any more
            let c: Vec<u32> = c.iter().copied().filter(|&u| !self.failed[u as usize]).collect();
            let k = c.len();
            if k < 2 {
                continue;
            }
            if !clock.take(2 * looks(k) + items(k), budget) {
                return Checked::Stopped;
            }
            let pos_stamp = self.new_stamp();
            for (p, &u) in c.iter().enumerate() {
                let ui = u as usize;
                // of a row of this grid, one literal per row, one column per literal
                if self.g_tag[ui] != rows_stamp || self.g_pos[ui] == NONE || self.x_tag[ui] == in_col || slot[ui] == NONE {
                    return Checked::Wrong;
                }
                self.x_tag[ui] = in_col;
                let r = self.g_pos[ui] as usize;
                if row_mark[r] == col_no {
                    return Checked::Wrong;
                }
                row_mark[r] = col_no;
                self.e_tag[ui] = pos_stamp;
                self.lit_row[ui] = p as u32;
            }
            // each pair: an input clause under its smaller literal, or by propagation
            let mut rups = 0u64;
            found.clear();
            found.resize(k, false);
            for p in 0..k {
                let u = c[p];
                let s = slot[u as usize] as usize;
                let (a, b) = (table.start[s] as usize, table.start[s + 1] as usize);
                if !clock.take(TICK + looks(b - a) + items(k - p), budget) {
                    return Checked::Stopped;
                }
                found[p + 1..k].fill(false);
                let mut have = 0;
                for e in &table.ent[a..b] {
                    let v = self.uid[lx(table.lits[e.0 as usize])] as usize;
                    if self.e_tag[v] == pos_stamp {
                        found[self.lit_row[v] as usize] = true;
                        have += 1;
                    }
                }
                if have < k - 1 - p {
                    for q in p + 1..k {
                        if found[q] {
                            continue;
                        }
                        match self.pair_by_propagation(u, c[q], clock) {
                            Err(()) => return Checked::Stopped,
                            Ok(false) => return Checked::Wrong,
                            Ok(true) => {}
                        }
                        rups += 1;
                    }
                }
            }
            if !clock.take(TICK + looks(k), budget) {
                return Checked::Stopped;
            }
            let lits: Vec<i32> = c.iter().map(|&u| self.ulit[u as usize]).collect();
            cols.push(Col { lits, rups, pairs: Arc::clone(table) });
        }
        if cols.is_empty() {
            return Checked::Dropped;
        }
        let mut rows: Vec<Row> = Vec::with_capacity(row_ids.len());
        for &ri in &row_ids {
            let (a, b) = self.row_range(ri);
            if !clock.take(TICK + looks(b - a), budget) {
                return Checked::Stopped;
            }
            let lits: Vec<i32> = self.row_lits[a..b].iter().filter(|&&u| !self.failed[u as usize]).map(|&u| self.ulit[u as usize]).collect();
            let failed: Vec<i32> = if self.row_state[ri as usize] == ROW_FAILED_FIRST {
                self.row_lits[a..b].iter().filter(|&&u| self.failed[u as usize]).map(|&u| self.ulit[u as usize]).collect()
            } else {
                Vec::new()
            };
            let i = self.row_clause[ri as usize];
            let whole = lits.len() == b - a;
            rows.push(Row { lits, clause: (i != NONE && whole).then_some(i), failed });
        }
        Checked::Good(Grid { rows, cols })
    }

    /// Whether the row is what it says, by its clause or by propagation. A literal
    /// that fails alone is false at the root and stays out of the row. Its proof line
    /// comes first, unless it is the one such literal of an input clause: then the
    /// clause forces it, with the row's literals false. Err: out of budget.
    fn check_row(&mut self, ri: u32, clock: &mut Clock) -> Result<u8, ()> {
        if self.row_state[ri as usize] == ROW_UNKNOWN {
            self.row_state[ri as usize] = self.check_row_now(ri, clock)?;
        }
        Ok(self.row_state[ri as usize])
    }

    fn check_row_now(&mut self, ri: u32, clock: &mut Clock) -> Result<u8, ()> {
        let budget = self.budget;
        let (a, b) = self.row_range(ri);
        if !clock.take(TICK + 2 * looks(b - a), budget) {
            return Err(());
        }
        let failing = self.row_lits[a..b].iter().filter(|&&u| self.failed[u as usize]).count();
        let i = self.row_clause[ri as usize];
        if i != NONE {
            let f = self.live.f;
            let i = i as usize;
            if f.taut[i] || f.dup[i] || f.clause(i).len() != b - a {
                return Ok(ROW_BAD);
            }
            // the clause holds no literal twice and as many as the row, so all of
            // them in the row is the same set
            let row = &self.row_lits[a..b];
            for &l in f.clause(i) {
                let u = self.uid[lx(l)];
                if u == NONE || row.binary_search(&u).is_err() {
                    return Ok(ROW_BAD);
                }
            }
            // the others false, one failing literal is forced by the clause and fails
            return Ok(match failing {
                0 => ROW_AS_READ,
                1 => ROW_WITHOUT_FAILED,
                _ => ROW_FAILED_FIRST,
            });
        }
        if !self.false_row_fails(ri, clock)? {
            return Ok(ROW_BAD);
        }
        Ok(if failing > 0 { ROW_FAILED_FIRST } else { ROW_AS_READ })
    }

    /// With the row's literals false, does propagation over the input fail? First
    /// over the long clauses, which is how the row was made: a literal of a gate can
    /// imply thousands through two-literal clauses that have no part in it.
    fn false_row_fails(&mut self, ri: u32, clock: &mut Clock) -> Result<bool, ()> {
        let (a, b) = self.row_range(ri);
        for pairs in [false, true] {
            for k in a..b {
                let u = self.row_lits[k] as usize;
                if !self.live.assume(-self.ulit[u]) {
                    self.live.undo();
                    return Ok(false); // true at the root, or the row holds both signs
                }
            }
            let r = self.live.propagate_how(clock, self.budget, usize::MAX, pairs);
            self.live.undo();
            match r {
                Up::Conflict => return Ok(true),
                Up::Stopped => return Err(()),
                _ => {}
            }
        }
        Ok(false)
    }

    /// Whether propagating one of the two derives the other's negation. It looks
    /// at the raw list a literal's own propagation gave, not at what the reading
    /// made of it.
    fn pair_by_propagation(&mut self, a: u32, b: u32, clock: &mut Clock) -> Result<bool, ()> {
        for (x, y) in [(a, b), (b, a)] {
            if !self.read_lit(x, self.budget, clock) {
                return Err(());
            }
            if self.failed[x as usize] || self.late_failed[x as usize] {
                return Ok(true); // x alone fails: VeriPB finds the same conflict
            }
            let (c, d) = self.ex_range(x);
            clock.add(TICK / 2);
            if self.ex_arena[c..d].binary_search(&y).is_ok() {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formula(clauses: &[Vec<i32>]) -> Formula {
        let mut f = Formula { lits: Vec::new(), start: vec![0], taut: Vec::new(), dup: Vec::new(), vars: 0 };
        for c in clauses {
            let mut out: Vec<i32> = Vec::new();
            for &l in c {
                f.vars = f.vars.max(l.abs());
                if !out.contains(&l) {
                    out.push(l);
                }
            }
            let taut = out.iter().any(|&l| out.contains(&-l));
            if !taut {
                f.lits.extend_from_slice(&out);
            }
            f.taut.push(taut);
            f.dup.push(out.len() < c.len());
            f.start.push(f.lits.len() as u32);
        }
        f
    }

    fn php(pigeons: i32, holes: i32) -> Vec<Vec<i32>> {
        let var = |p: i32, h: i32| p * holes + h + 1;
        let mut cl: Vec<Vec<i32>> = (0..pigeons).map(|p| (0..holes).map(|h| var(p, h)).collect()).collect();
        for h in 0..holes {
            for a in 0..pigeons {
                for b in a + 1..pigeons {
                    cl.push(vec![-var(a, h), -var(b, h)]);
                }
            }
        }
        cl
    }

    /// The reader run to the end of detection on `clauses`, then `tamper` on what it
    /// holds, then the check. Returns (grids returned, grids the check called wrong).
    fn checked(clauses: &[Vec<i32>], tamper: impl Fn(&mut Reader)) -> (usize, u64) {
        let f = formula(clauses);
        let mut ticks = 0u64;
        let stop = AtomicBool::new(false);
        let mut clock = Clock { flagged: false, units: 0, ticks: &mut ticks, stop: &stop, last_poll: 0 };
        let live = Live::new(&f, &mut clock, u64::MAX).unwrap();
        let mut r = Reader::new(live, &mut clock, u64::MAX).unwrap();
        r.run(&mut clock);
        tamper(&mut r);
        let out = r.finish(&mut clock);
        (out.len(), r.rejected)
    }

    #[test]
    fn the_check_lets_an_honest_grid_through() {
        assert_eq!(checked(&php(5, 4), |_| {}), (1, 0));
    }

    #[test]
    fn the_check_refuses_a_column_that_holds_a_pair_that_does_not_exclude() {
        // two literals of two rows, in different holes
        let (n, wrong) = checked(&php(5, 4), |r| {
            let g = &mut r.grids[0];
            let (a, b) = (g.cols[0][0], g.cols[1][1]);
            g.cols[0][0] = b;
            g.cols[1][1] = a;
        });
        assert_eq!((n, wrong), (0, 1));
    }

    #[test]
    fn the_check_refuses_two_literals_of_one_row_in_a_column() {
        let (n, wrong) = checked(&php(5, 4), |r| {
            let g = &mut r.grids[0];
            // literals 1 and 2 are the first row's
            g.cols[0] = vec![r.uid[lx(1)], r.uid[lx(2)]];
            g.cols[1].retain(|&u| u != r.uid[lx(2)]);
        });
        assert_eq!((n, wrong), (0, 1));
    }

    #[test]
    fn the_check_refuses_a_row_that_says_the_wrong_clause() {
        let (n, wrong) = checked(&php(5, 4), |r| r.row_clause[0] = 1);
        assert_eq!((n, wrong), (0, 1));
        // a repeated literal: its constraint has another coefficient
        let mut cl = php(5, 4);
        let first = cl[0][0];
        cl[0].insert(0, first);
        let (n, wrong) = checked(&cl, |r| r.row_clause[0] = 0);
        assert_eq!((n, wrong), (0, 1));
    }

    #[test]
    fn the_check_refuses_a_row_propagation_does_not_prove() {
        // the second clause of the row set with a literal of another row in it
        let (n, wrong) = checked(&php(5, 4), |r| {
            r.row_clause[1] = NONE;
            let (a, _) = r.row_range(1);
            let other = r.uid[lx(1)];
            r.row_lits[a] = other;
            let (a, b) = r.row_range(1);
            r.row_lits[a..b].sort_unstable();
        });
        assert_eq!((n, wrong), (0, 1));
    }

    #[test]
    fn the_line_counts_are_the_lines_and_a_short_budget_writes_nothing() {
        // one column of pair clauses, one by a ladder: 5 pigeons, 2 holes each way
        let mut cl: Vec<Vec<i32>> = (0..5).map(|p| vec![p * 2 + 1, p * 2 + 2]).collect();
        for (a, b) in [(1, 3), (1, 5), (1, 7), (1, 9), (3, 5), (3, 7), (3, 9), (5, 7), (5, 9), (7, 9)] {
            cl.push(vec![-a, -b]);
        }
        // a third literal in each row, ordered by a ladder over fresh variables
        let third: Vec<i32> = (0..5).map(|p| 11 + p).collect();
        for p in 0..5 {
            cl[p as usize].push(third[p as usize]);
        }
        for i in 0..4 {
            cl.push(vec![-third[i], 16 + i as i32]);
            cl.push(vec![-third[i + 1], -(16 + i as i32)]);
            if i > 0 {
                cl.push(vec![-(16 + i as i32 - 1), 16 + i as i32]);
            }
        }
        for (a, b) in [(2, 4), (2, 6), (2, 8), (2, 10), (4, 6), (4, 8), (4, 10), (6, 8), (6, 10), (8, 10)] {
            cl.push(vec![-a, -b]);
        }
        let f = formula(&cl);
        let mut ticks = 0u64;
        let stop = AtomicBool::new(false);
        let grids = read(&f, u64::MAX, &mut ticks, &stop);
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].cols.len(), 3);
        let path = std::env::temp_dir().join(format!("gridread-test-{}.pbp", std::process::id()));
        let mut w = Writer::create(path.to_str().unwrap(), cl.len() as u64).unwrap();
        let mut work = 0u64;
        let mut ladder = false;
        for c in &grids[0].cols {
            let lines = col_lines(c);
            ladder |= lines > 1;
            if lines > 0 {
                let before = w.steps;
                assert!(prove_col(c, &mut w, lines - 1, &mut work).is_none());
                assert_eq!(w.steps, before, "a budget one line short writes nothing");
            }
            let before = w.steps;
            assert!(prove_col(c, &mut w, lines, &mut work).is_some());
            assert_eq!(w.steps - before, lines, "the column's line count is its lines");
        }
        assert!(ladder, "the ladder's pairs take `rup` lines");
        for r in &grids[0].rows {
            assert_eq!(row_lines(r), 0);
            assert_eq!(prove_row(r, &mut w, 0, &mut work), Some(r.clause.unwrap() as u64 + 1));
        }
        let _ = w.finish_no_conclusion();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_read_stops_at_its_budget_and_on_the_flag() {
        let f = formula(&php(30, 29));
        let stop = AtomicBool::new(false);
        let mut whole = 0u64;
        let all = read(&f, u64::MAX, &mut whole, &stop);
        assert_eq!(all.len(), 1);
        for part in [1, 2, 5, 10, 25, 50, 75, 90] {
            let budget = whole * part / 100;
            let mut ticks = 0u64;
            let _ = read(&f, budget, &mut ticks, &stop);
            assert!(ticks <= budget + budget / 10 + 16, "{} ticks over a limit of {}", ticks, budget);
        }
        let stopped = AtomicBool::new(true);
        let mut ticks = 0u64;
        assert!(read(&f, u64::MAX, &mut ticks, &stopped).is_empty());
        assert_eq!(ticks, 0);
    }
}
