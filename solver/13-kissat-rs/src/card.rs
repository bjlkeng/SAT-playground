//! Not in kissat. The count and alldifferent engine (plan
//! plan/count-and-hall-lemmas-plan.md; epic SAT-playground-iin).
//!
//! A Card says "at least `need` of `lits`". A clause is need = 1; "at most K
//! of these" is "at least n - K of their negations"; an alldifferent column
//! is K = 1. A network uses some Cards as rows (they want trues) and some as
//! caps (they allow few). When a row literal meets its negation in a cap the
//! two cancel in the sum, and if the rows want more than the caps allow, the
//! sum is a contradiction that unit propagation, reading one clause at a
//! time, does not see.
//!
//! At a propagation fixpoint, the engine reads the networks whose variables
//! moved. A matching between rows and caps only chooses what to sum. What
//! decides is exact arithmetic on that sum S: its literals that are false now
//! form the lemma, and with those at zero either the rest cannot reach the
//! right-hand side (a contradiction, handed to conflict analysis) or an open
//! literal's coefficient exceeds the slack (it is forced, with the lemma as
//! its reason). S follows from the Cards alone, whatever the search learned,
//! so each lemma follows from the input.
//!
//! The engine holds external literals, so compaction does not touch it. A
//! literal the solver eliminated counts as open and never enters a lemma.
//!
//! With `card=0` nothing here runs and the solver is the stock one.

use crate::internal::Solver;
use crate::literal::idx;
use crate::propsearch::Conflict;

/// Where a Card's linear constraint comes from in the proof.
pub enum Source {
    /// An input clause (0-based index in the Formula).
    Clause(u32),
    /// At least size - 1 of these literals: a clique of input clauses (a v b),
    /// the clause ids in `pairs` in the order (0,1), (0,2), (1,2), (0,3), ...
    Clique(Vec<u32>),
    /// A count from the reader.
    Count(u32),
    /// A grid's row that is a resolvent.
    Row(u32, u32),
    /// A grid's column.
    Col(u32, u32),
}

pub struct Card {
    pub lits: Vec<i32>,
    pub need: i32,
    pub source: Source,
    /// The proof constraint's id, 0 until it is written.
    pub id: u64,
    /// The proof terms it takes at most: its lines, each as long as the Card and
    /// one more. Counted the same with a proof file or without one, so that the
    /// two runs make the same decisions. Lines alone would not do: one line of a
    /// diagram's proof can hold thousands of terms.
    pub terms: u64,
    /// Its lines were charged to the proof budget.
    pub charged: bool,
}

pub struct Net {
    pub rows: Vec<u32>,
    pub caps: Vec<u32>,
    /// The proof budget ran out on one of its Cards: not read again.
    pub dead: bool,
    /// Reads in a row that found nothing, and how many turns it still sits out:
    /// a network that keeps finding nothing is read less and less often.
    pub fails: u8,
    pub skip: u32,
    /// One count and its clauses, a grid, or a group of tied counts.
    pub kind: u8,
    /// What a read is expected to cost: its size at first, then what the last whole
    /// read cost. A read starts only when the look has room for this, so a limit
    /// that is nearly used up pays for whole reads later, not for partial ones now.
    pub cost: u64,
}

const KIND_COUNT: u8 = 0;
const KIND_GRID: u8 = 1;
const KIND_MULTI: u8 = 2;

#[derive(Default)]
pub struct State {
    pub on: bool,
    /// What the readers found, before the networks are built.
    pub reading: crate::cardread::Reading,
    pub grids: Vec<crate::gridread::Grid>,
    pub cards: Vec<Card>,
    pub nets: Vec<Net>,
    /// A Card literal to the networks holding it: a network can only get
    /// tighter when one of its Cards' literals becomes false.
    lit_start: Vec<u32>,
    lit_nets: Vec<u32>,
    dirty: Vec<bool>,
    queue: std::collections::VecDeque<u32>,
    cursor: usize,
    next_check: u64,
    /// Looks in a row that found nothing.
    empty_looks: u32,
    /// A forced literal was assigned: look again after propagating it.
    again: bool,
    /// The work this look may reach: a read that gets there ends with nothing.
    room: u64,
    /// The read in progress ended at that limit, or on a stop.
    cut: bool,
    // scratch, per external literal
    coef: Vec<i32>,
    touched: Vec<i32>,
    cap_of: Vec<u32>,
    owner: Vec<u32>,
    pub work: u64,
    /// The part of `work` already on the solver's clocks.
    published: u64,
    /// The network being read.
    current_kind: u8,
    /// Hashes of the clauses the search already has, so that a root lemma is
    /// not added twice. A collision only loses a lemma.
    known: std::collections::HashSet<u64>,
    /// What the report prints.
    pub s: Stats,
}

/// A look takes the state out of the solver, and a signal can end the run during
/// one: these numbers stay behind for the report.
#[derive(Clone, Copy, Default)]
pub struct Stats {
    pub nets: u64,
    pub cards: u64,
    pub checks: u64,
    pub nets_read: u64,
    pub violations: u64,
    pub forced: u64,
    pub units: u64,
    pub too_long: u64,
    pub lemma_lits: u64,
    pub not_sound: u64,
    /// Proof terms planned so far, with a proof file or without one.
    pub planned_terms: u64,
    pub no_budget: u64,
    pub skipped: u64,
    /// Reads the look's limit or a stop ended early.
    pub cut_short: u64,
    /// Reads and proofs left for later because the look had no room for them.
    pub no_room: u64,
    /// The most work one look took.
    pub look_max: u64,
    pub root_clauses: u64,
    /// Lemmas by the kind of network they came from, and by length: 1, 2, 3-4,
    /// 5-8, 9-16, 17-64, more.
    pub by_kind: [u64; 3],
    pub by_len: [u64; 7],
    pub too_short: u64,
    /// What the readers found: the reading itself is freed when no network comes of it.
    pub counts_found: u64,
    pub grids_found: u64,
    /// The counts found by form: Sinz, totalizer, one-way, subset, diagram, network, adder.
    pub forms_found: [u64; 7],
    pub count_read_ticks: u64,
    pub grid_read_ticks: u64,
    pub build_ticks: u64,
}

impl State {
    /// The proof stream could not start: no lemma could be proved, so the engine
    /// holds nothing to read in the search.
    pub fn clear_for_lost_proof(&mut self) {
        self.s.nets = 0;
        self.s.cards = 0;
        self.nets = Vec::new();
        self.cards = Vec::new();
        self.reading = Default::default();
        self.grids = Vec::new();
    }
}

#[inline]
fn lx(l: i32) -> usize {
    (l.unsigned_abs() as usize) * 2 + (l < 0) as usize
}

/// The value of an external literal: 1, -1, 0 open, 2 no longer in the solver.
#[inline]
fn value(solver: &Solver, e: i32) -> i8 {
    let ev = e.unsigned_abs() as usize;
    if ev >= solver.import_.len() {
        return 2;
    }
    let imp = &solver.import_[ev];
    if !imp.imported || imp.eliminated {
        return 2;
    }
    let lit = if e > 0 { imp.lit } else { imp.lit ^ 1 };
    let v = solver.values[lit as usize];
    if v == 0 && !solver.flags[idx(lit) as usize].active() {
        return 2;
    }
    v
}

#[inline]
fn internal(solver: &Solver, e: i32) -> u32 {
    let imp = &solver.import_[e.unsigned_abs() as usize];
    if e > 0 {
        imp.lit
    } else {
        imp.lit ^ 1
    }
}

/// The scratch tables and the index from a Card literal to its networks.
fn index(g: &mut State, max_var: usize) {
    let lits = 2 * (max_var + 1);
    g.coef = vec![0; lits];
    g.cap_of = vec![0; lits];
    g.owner = vec![0; lits];
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    for (n, net) in g.nets.iter().enumerate() {
        for &c in net.rows.iter().chain(net.caps.iter()) {
            for &l in &g.cards[c as usize].lits {
                pairs.push((lx(l) as u32, n as u32));
            }
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    g.lit_start = vec![0; lits + 1];
    g.lit_nets = Vec::with_capacity(pairs.len());
    for &(l, n) in &pairs {
        g.lit_start[l as usize + 1] += 1;
        g.lit_nets.push(n);
    }
    for i in 1..g.lit_start.len() {
        g.lit_start[i] += g.lit_start[i - 1];
    }
    g.dirty = vec![false; g.nets.len()];
}

/// Called once before the search starts.
pub fn start(solver: &mut Solver) {
    if solver.options.card == 0 {
        return;
    }
    let mut g = std::mem::take(&mut solver.card);
    // a lemma with no line in the stream would break the DRAT proof, so a run that
    // writes a proof has the engine only while the stream is alive
    g.on = !g.nets.is_empty() && (solver.proof.is_none() || solver.gaussproof.active());
    if !g.on {
        solver.card = g;
        return;
    }
    // everything starts dirty: the first look reads every network once
    for n in 0..g.nets.len() {
        g.dirty[n] = true;
        g.queue.push_back(n as u32);
    }
    solver.card = g;
}

/// A look is due between conflicts (with `cardrestart=0`), or a forced literal
/// was just propagated and the rest of the networks are waiting.
#[inline]
pub fn due(solver: &Solver) -> bool {
    solver.card.on && (solver.card.again || (solver.options.cardrestart == 0 && solver.statistics.conflicts >= solver.card.next_check))
}

/// With `cardrestart=1` the look comes just before a restart, when the trail is
/// at its deepest. Returns true when the search should go on instead of
/// restarting: `conflict` holds a conflict to analyze, or a literal was assigned
/// and has to be propagated.
#[inline]
pub fn before_restart(solver: &mut Solver, conflict: &mut Option<Conflict>) -> bool {
    if !solver.card.on || solver.options.cardrestart == 0 || solver.statistics.conflicts < solver.card.next_check {
        return false;
    }
    *conflict = check(solver);
    // a unit can replace the only decision, so the trail's length says nothing
    conflict.is_some() || solver.card.again
}

/// Root literals are about to leave the trail: the networks they made tighter are
/// marked now, or they would never be. Each literal leaves once in a run, so this
/// costs at most one pass over the index, which the build paid for.
pub fn flushing(solver: &mut Solver) {
    let mut g = std::mem::take(&mut solver.card);
    solver.card.s = g.s;
    let end = solver.trail.len();
    let mut scanned = 0u64;
    for i in g.cursor.min(end)..end {
        scanned += mark_literal(solver, &mut g, solver.trail[i]);
    }
    // on the clocks at once: the run may end before the next look
    solver.statistics.card_ticks += scanned / 4;
    solver.statistics.ticks += scanned / 4;
    g.cursor = 0;
    solver.card = g;
}

/// This trail literal made false the Cards' literals opposite it: their networks
/// are queued. Returns what it read.
#[inline]
fn mark_literal(solver: &Solver, g: &mut State, lit: u32) -> u64 {
    let e = solver.export_[idx(lit) as usize];
    let e = if crate::literal::negated(lit) != 0 { -e } else { e };
    // the literal -e is now false
    let x = lx(-e);
    if x + 1 >= g.lit_start.len() {
        return 1;
    }
    for k in g.lit_start[x]..g.lit_start[x + 1] {
        let n = g.lit_nets[k as usize] as usize;
        if !g.dirty[n] {
            g.dirty[n] = true;
            g.queue.push_back(n as u32);
        }
    }
    1 + (g.lit_start[x + 1] - g.lit_start[x]) as u64
}

#[inline]
pub fn backtracked(solver: &mut Solver, new_end: usize) {
    if solver.card.cursor > new_end {
        solver.card.cursor = new_end;
    }
}

/// A clique can be this large; a larger set of pair clauses is split.
const MAX_CLIQUE: usize = 64;
/// Proof terms written per tick on the engine's clock when a Card is first used.
const PROOF_TERMS_PER_TICK: u64 = 16;

/// A Card's proof takes at most this many terms.
fn proof_terms(lines: u64, len: usize) -> u64 {
    lines.saturating_mul(len as u64 + 1)
}

/// The `pol` lines that add up an at-most-one's pairs name every pair once with an
/// operator each, and a multiplication and a division for each literal.
fn clique_terms(len: usize) -> u64 {
    let n = len as u64;
    n.saturating_mul(n).saturating_add(5 * n)
}

/// The Cards and networks from what the readers found. Runs in the structure
/// stage, while the formula is at hand, and stops at `budget`.
pub fn build(solver: &mut Solver, f: &crate::structure::Formula, budget: u64, stop: &std::sync::atomic::AtomicBool) {
    let mut g = std::mem::take(&mut solver.card);
    solver.card.s = g.s;
    let mut work = solver.statistics.ticks;
    let stopped = |work: u64| work > budget || stop.load(std::sync::atomic::Ordering::Relaxed);
    let nl = 2 * (f.vars as usize + 1);
    // the tables are one piece: done only when the budget covers them, and not
    // charged when they are not done
    let tables = nl as u64 + f.len() as u64;
    if stopped(work.saturating_add(tables)) {
        g.reading = Default::default();
        g.grids = Vec::new();
        solver.card = g;
        return;
    }
    work += tables;
    // one Card per input clause used
    let mut clause_card: Vec<u32> = vec![u32::MAX; f.len()];
    let reading = std::mem::take(&mut g.reading);
    // the index from a literal to its counts is one piece too: filled twice over
    work += reading.found.len() as u64 / 16;
    let indexed: u64 = reading.found.iter().map(|c| c.lits.len() as u64).sum::<u64>().saturating_mul(2);
    if stopped(work.saturating_add(indexed)) {
        g.grids = Vec::new();
        solver.statistics.ticks = work;
        solver.card = g;
        return;
    }
    work += indexed;
    // the counts holding a literal as something they limit: a clause literal x
    // cancels against a count that holds -x
    let mut item_start = vec![0u32; nl + 1];
    for c in &reading.found {
        for &l in &c.lits {
            item_start[lx(-l) + 1] += 1;
        }
    }
    for i in 1..item_start.len() {
        item_start[i] += item_start[i - 1];
    }
    let mut item_counts = vec![0u32; item_start[nl] as usize];
    let mut fill = item_start.clone();
    for (ci, c) in reading.found.iter().enumerate() {
        for &l in &c.lits {
            let x = lx(-l);
            item_counts[fill[x] as usize] = ci as u32;
            fill[x] += 1;
        }
    }
    drop(fill);
    // each count's candidate rows: clauses with two or more literals it limits
    let mut cands: Vec<Vec<u32>> = vec![Vec::new(); reading.found.len()];
    let mut hit = vec![0u32; reading.found.len()];
    let mut touched: Vec<u32> = Vec::new();
    for i in 0..f.len() {
        let c = f.clause(i);
        solver.statistics.ticks = work;
        if stopped(work.saturating_add(c.len() as u64)) {
            break;
        }
        work += c.len() as u64;
        if c.len() < 2 || f.taut[i] || f.dup[i] {
            continue;
        }
        // the clause's literals and what they hold, whole or not at all
        let held: u64 = c.iter().map(|&l| (item_start[lx(l) + 1] - item_start[lx(l)]) as u64).sum();
        if stopped(work.saturating_add(held)) {
            break;
        }
        work += held;
        for &l in c {
            let x = lx(l);
            for k in item_start[x]..item_start[x + 1] {
                let ci = item_counts[k as usize] as usize;
                if hit[ci] == 0 {
                    touched.push(ci as u32);
                }
                hit[ci] += 1;
            }
        }
        for &ci in &touched {
            if hit[ci as usize] >= 2 {
                cands[ci as usize].push(i as u32);
            }
            hit[ci as usize] = 0;
        }
        touched.clear();
    }
    drop(hit);
    let mut in_count = vec![false; nl];
    let mut count_card: Vec<u32> = vec![u32::MAX; reading.found.len()];
    for (ci, c) in reading.found.iter().enumerate() {
        solver.statistics.ticks = work;
        // its tables and sorts are one piece
        if cands[ci].is_empty() || stopped(work.saturating_add(c.lits.len() as u64 + 8 * cands[ci].len() as u64)) {
            continue;
        }
        for &l in &c.lits {
            in_count[lx(-l)] = true;
        }
        // binary clauses inside the count make cliques; the rest are rows
        let mut pairs: Vec<(i32, i32, u32)> = Vec::new();
        let mut rest: Vec<u32> = Vec::new();
        for &i in &cands[ci] {
            let cl = f.clause(i as usize);
            if cl.len() == 2 && in_count[lx(cl[0])] && in_count[lx(cl[1])] {
                pairs.push((cl[0], cl[1], i));
                pairs.push((cl[1], cl[0], i));
            } else {
                rest.push(i);
            }
        }
        work += cands[ci].len() as u64;
        pairs.sort_unstable();
        pairs.dedup_by_key(|p| (p.0, p.1));
        work += pairs.len() as u64;
        let mut rows: Vec<u32> = Vec::new();
        // literals by how many partners they have, most first
        let mut degree: Vec<(i32, i32)> = Vec::new();
        let mut i = 0;
        while i < pairs.len() {
            let mut j = i;
            while j < pairs.len() && pairs[j].0 == pairs[i].0 {
                j += 1;
            }
            degree.push((-((j - i) as i32), pairs[i].0));
            i = j;
        }
        degree.sort_unstable();
        let mut degree_of: std::collections::HashMap<i32, i32> = std::collections::HashMap::with_capacity(degree.len());
        for &(d, l) in &degree {
            degree_of.insert(l, d);
        }
        work += degree.len() as u64;
        let pair_id = |a: i32, b: i32| -> Option<u32> { pairs.binary_search_by_key(&(a, b), |p| (p.0, p.1)).ok().map(|k| pairs[k].2) };
        let mut used: std::collections::HashSet<i32> = std::collections::HashSet::new();
        for &(_, v) in &degree {
            if stopped(work) {
                break;
            }
            if used.contains(&v) {
                continue;
            }
            let mut q = vec![v];
            let mut ids: Vec<u32> = Vec::new();
            let first = pairs.partition_point(|p| p.0 < v);
            let mut partners: Vec<(i32, i32)> = Vec::new();
            let mut k = first;
            while k < pairs.len() && pairs[k].0 == v {
                let u = pairs[k].1;
                if !used.contains(&u) {
                    partners.push((degree_of.get(&u).copied().unwrap_or(0), u));
                }
                k += 1;
            }
            work += partners.len() as u64 * 4;
            partners.sort_unstable();
            for &(_, u) in &partners {
                if q.len() >= MAX_CLIQUE || stopped(work) {
                    break;
                }
                let links: Option<Vec<u32>> = q.iter().map(|&w| pair_id(w, u)).collect();
                work += q.len() as u64;
                if let Some(links) = links {
                    ids.extend(links);
                    q.push(u);
                }
            }
            if q.len() >= 2 {
                used.extend(q.iter().copied());
                let terms = clique_terms(q.len());
                if q.len() == 2 {
                    rows.push(clause_row(&mut g, f, &mut clause_card, ids[0] as usize));
                } else {
                    g.cards.push(Card { need: q.len() as i32 - 1, lits: q, source: Source::Clique(ids), id: 0, terms, charged: false });
                    rows.push(g.cards.len() as u32 - 1);
                }
            }
        }
        rest.sort_by_key(|&i| (f.clause(i as usize).len(), i));
        work += rest.len() as u64;
        for &i in &rest {
            if stopped(work) {
                break;
            }
            work += f.clause(i as usize).len() as u64;
            rows.push(clause_row(&mut g, f, &mut clause_card, i as usize));
        }
        for &l in &c.lits {
            in_count[lx(-l)] = false;
        }
        if !rows.is_empty() {
            let cap = count_card_of(&mut g, &reading, &mut count_card, ci);
            g.nets.push(Net { rows, caps: vec![cap], dead: false, fails: 0, skip: 0, kind: KIND_COUNT, cost: 0 });
        }
    }
    if solver.options.cardmulti != 0 {
        work = build_multi(&mut g, f, &reading, &item_start, &item_counts, &mut clause_card, &mut count_card, work, &stopped);
    }
    g.reading = reading;
    if solver.options.cardroot != 0 && !stopped(work) {
        for i in 0..f.len() {
            let c = f.clause(i);
            if stopped(work.saturating_add(1 + c.len() as u64)) {
                break;
            }
            work += 1 + c.len() as u64;
            if !f.taut[i] {
                g.known.insert(clause_hash(c));
            }
        }
    }
    let grids = std::mem::take(&mut g.grids);
    for (gi, grid) in grids.iter().enumerate() {
        solver.statistics.ticks = work;
        if stopped(work) {
            break;
        }
        let mut rows = Vec::with_capacity(grid.rows.len());
        let mut whole = true;
        for (ri, row) in grid.rows.iter().enumerate() {
            if stopped(work.saturating_add(row.lits.len() as u64)) {
                whole = false;
                break;
            }
            work += row.lits.len() as u64;
            match row.clause {
                Some(i) => rows.push(clause_row(&mut g, f, &mut clause_card, i as usize)),
                None => {
                    g.cards.push(Card { lits: row.lits.clone(), need: 1, source: Source::Row(gi as u32, ri as u32), id: 0, terms: proof_terms(crate::gridread::row_lines(row), row.lits.len()), charged: false });
                    rows.push(g.cards.len() as u32 - 1);
                }
            }
        }
        let mut caps = Vec::with_capacity(grid.cols.len());
        for (ci, col) in grid.cols.iter().enumerate() {
            if !whole || stopped(work.saturating_add(col.lits.len() as u64)) {
                whole = false;
                break;
            }
            work += col.lits.len() as u64;
            let lits: Vec<i32> = col.lits.iter().map(|&l| -l).collect();
            let need = lits.len() as i32 - 1;
            // a `rup` line per pair that is no input clause, and the pairs' `pol` line
            let terms = proof_terms(crate::gridread::col_lines(col), 2).saturating_add(clique_terms(lits.len()));
            g.cards.push(Card { lits, need, source: Source::Col(gi as u32, ci as u32), id: 0, terms, charged: false });
            caps.push(g.cards.len() as u32 - 1);
        }
        if whole && !rows.is_empty() && !caps.is_empty() && rows.len() >= solver.options.cardminrows as usize {
            g.nets.push(Net { rows, caps, dead: false, fails: 0, skip: 0, kind: KIND_GRID, cost: 0 });
        }
    }
    g.grids = grids;
    // the walks over the networks' Cards below are paid first
    let refs: u64 = g.nets.iter().map(|n| (n.rows.len() + n.caps.len()) as u64).sum();
    work += g.nets.len() as u64 / 16;
    if stopped(work.saturating_add(2 * refs)) {
        g.nets = Vec::new();
    } else {
        work += 2 * refs;
    }
    // cards too costly to prove never enter a sum
    let most = (solver.options.cardproofmax as u64).saturating_mul(1000);
    for net in &mut g.nets {
        for &c in net.rows.iter().chain(net.caps.iter()) {
            if g.cards[c as usize].terms > most {
                net.dead = true;
            }
        }
    }
    let kinds = solver.options.cardkinds;
    g.nets.retain(|n| !n.dead && kinds >> n.kind & 1 != 0);
    if !g.nets.is_empty() {
        // the index is one piece too: its sort and three tables per literal
        let mut held = 0u64;
        for net in &mut g.nets {
            let mut size = 0u64;
            for &c in net.rows.iter().chain(net.caps.iter()) {
                size += g.cards[c as usize].lits.len() as u64;
            }
            net.cost = size;
            held += size;
        }
        let cost = 3 * nl as u64 + 2 * held;
        if stopped(work.saturating_add(cost)) {
            g.nets = Vec::new();
        } else {
            work += cost;
            index(&mut g, f.vars as usize);
        }
    }
    if g.nets.is_empty() {
        g.cards = Vec::new();
        g.reading = Default::default();
        g.grids = Vec::new();
    }
    g.s.nets = g.nets.len() as u64;
    g.s.cards = g.cards.len() as u64;
    solver.statistics.ticks = work;
    solver.card = g;
}

/// Networks of several counts: a clause with limited literals in two or more
/// counts ties those counts together, and a group of tied counts with all their
/// clauses is one network, the caps of a global cardinality constraint. Where a
/// literal sits in two caps of a network it counts for the first; the arithmetic
/// on the sum decides, as everywhere.
#[allow(clippy::too_many_arguments)]
fn build_multi(
    g: &mut State,
    f: &crate::structure::Formula,
    reading: &crate::cardread::Reading,
    item_start: &[u32],
    item_counts: &[u32],
    clause_card: &mut [u32],
    count_card: &mut [u32],
    mut work: u64,
    stopped: &dyn Fn(u64) -> bool,
) -> u64 {
    let n = reading.found.len();
    if n < 2 {
        return work;
    }
    fn root(parent: &mut [u32], mut x: u32) -> u32 {
        while parent[x as usize] != x {
            parent[x as usize] = parent[parent[x as usize] as usize];
            x = parent[x as usize];
        }
        x
    }
    if stopped(work.saturating_add(n as u64)) {
        return work;
    }
    work += n as u64;
    let mut parent: Vec<u32> = (0..n as u32).collect();
    let mut rows: Vec<(u32, u32)> = Vec::new(); // the first count, the clause
    let mut seen: Vec<u32> = Vec::new();
    for i in 0..f.len() {
        let c = f.clause(i);
        if stopped(work.saturating_add(c.len() as u64)) {
            return work;
        }
        work += c.len() as u64;
        if c.len() < 2 || f.taut[i] || f.dup[i] {
            continue;
        }
        // the clause's literals and what they hold, whole or not at all
        let held: u64 = c.iter().map(|&l| (item_start[lx(l) + 1] - item_start[lx(l)]) as u64).sum();
        if stopped(work.saturating_add(2 * held)) {
            return work;
        }
        work += held;
        seen.clear();
        let mut limited = 0;
        for &l in c {
            let x = lx(l);
            let (a, b) = (item_start[x], item_start[x + 1]);
            if a < b {
                limited += 1;
                seen.extend_from_slice(&item_counts[a as usize..b as usize]);
            }
        }
        seen.sort_unstable();
        seen.dedup();
        work += seen.len() as u64;
        if limited < 2 || seen.len() < 2 {
            continue;
        }
        let first = root(&mut parent, seen[0]);
        for k in 1..seen.len() {
            let r = root(&mut parent, seen[k]);
            parent[r as usize] = first;
        }
        rows.push((seen[0], i as u32));
    }
    // the groups in the order of their smallest count, the same on every run
    if stopped(work.saturating_add(4 * n as u64)) {
        return work;
    }
    work += 4 * n as u64;
    let mut members: Vec<(u32, u32)> = (0..n as u32).map(|c| (root(&mut parent, c), c)).collect();
    members.sort_unstable();
    let mut groups: Vec<(u32, Vec<u32>, Vec<u32>)> = Vec::new();
    let mut slot_of: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut k = 0;
    while k < members.len() {
        let mut e = k;
        while e < members.len() && members[e].0 == members[k].0 {
            e += 1;
        }
        if e - k >= 2 {
            slot_of.insert(members[k].0, groups.len() as u32);
            groups.push((members[k].0, members[k..e].iter().map(|m| m.1).collect(), Vec::new()));
        }
        k = e;
    }
    if stopped(work.saturating_add(rows.len() as u64)) {
        return work;
    }
    work += rows.len() as u64;
    for &(c, i) in &rows {
        let r = root(&mut parent, c);
        if let Some(&s) = slot_of.get(&r) {
            groups[s as usize].2.push(i);
        }
    }
    for (_, counts, clauses) in groups {
        if stopped(work) || clauses.is_empty() {
            continue;
        }
        // a group is used whole, or not at all
        let mut row_cards = Vec::with_capacity(clauses.len());
        let mut whole = true;
        for &i in &clauses {
            let len = f.clause(i as usize).len() as u64;
            if stopped(work.saturating_add(len)) {
                whole = false;
                break;
            }
            work += len;
            row_cards.push(clause_row(g, f, clause_card, i as usize));
        }
        let mut caps = Vec::with_capacity(counts.len());
        for &ci in &counts {
            let len = reading.found[ci as usize].lits.len() as u64;
            if !whole || stopped(work.saturating_add(len)) {
                whole = false;
                break;
            }
            work += len;
            caps.push(count_card_of(g, reading, count_card, ci as usize));
        }
        if whole {
            g.nets.push(Net { rows: row_cards, caps, dead: false, fails: 0, skip: 0, kind: KIND_MULTI, cost: 0 });
        }
    }
    work
}

/// The same for any order of the literals.
fn clause_hash(lits: &[i32]) -> u64 {
    let mut h = lits.len() as u64;
    for &l in lits {
        let x = lx(l) as u64 + 1;
        let mut m = x.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        m ^= m >> 29;
        h = h.wrapping_add(m.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    }
    h
}

/// One Card per count, whichever networks hold it.
fn count_card_of(g: &mut State, reading: &crate::cardread::Reading, count_card: &mut [u32], ci: usize) -> u32 {
    if count_card[ci] == u32::MAX {
        let c = &reading.found[ci];
        g.cards.push(Card { lits: c.lits.clone(), need: c.need, source: Source::Count(ci as u32), id: 0, terms: reading.terms(ci), charged: false });
        count_card[ci] = g.cards.len() as u32 - 1;
    }
    count_card[ci]
}

fn clause_row(g: &mut State, f: &crate::structure::Formula, clause_card: &mut [u32], i: usize) -> u32 {
    if clause_card[i] == u32::MAX {
        g.cards.push(Card { lits: f.clause(i).to_vec(), need: 1, source: Source::Clause(i as u32), id: 0, terms: 0, charged: true });
        clause_card[i] = g.cards.len() as u32 - 1;
    }
    clause_card[i]
}

/// The sum of these Cards may be used if their proofs fit the budget. Decided
/// the same with a proof file or without one.
fn budget_allows(solver: &Solver, g: &mut State, cards: &[u32]) -> bool {
    let total = (solver.options.cardprooftotal as u64).saturating_mul(1000);
    let mut planned = g.s.planned_terms;
    for &c in cards {
        let card = &g.cards[c as usize];
        if !card.charged {
            planned = planned.saturating_add(card.terms);
        }
    }
    if planned > total {
        g.s.no_budget += 1;
        return false;
    }
    // what writing them costs goes on the engine's clock here, by the plan, so
    // the clock reads the same with a proof file and without one; a proof the look
    // has no room for is not started, and nothing is charged for it
    let cost = (planned - g.s.planned_terms) / PROOF_TERMS_PER_TICK;
    if cost > g.room.saturating_sub(g.work) {
        g.s.no_room += 1;
        return false;
    }
    g.work += cost;
    g.s.planned_terms = planned;
    for &c in cards {
        g.cards[c as usize].charged = true;
    }
    true
}

/// The Card's proof constraint, written if this is its first use.
fn prove_card(solver: &mut Solver, g: &mut State, c: u32) -> u64 {
    let card = &g.cards[c as usize];
    if card.id != 0 {
        return card.id;
    }
    let mut proof_work = 0u64;
    let id = match &card.source {
        Source::Clause(i) => *i as u64 + 1,
        Source::Clique(pairs) => {
            // size 2 is the pair itself; each further literal: the previous sum
            // times (t - 2), plus the literal's pairs, divided by (t - 1)
            let n = card.lits.len();
            let mut k = 0usize;
            let mut cur = pairs[k] as u64 + 1;
            k += 1;
            for t in 3..=n {
                let mut expr = format!("{} {} *", cur, t - 2);
                for _ in 0..t - 1 {
                    expr.push_str(&format!(" {} +", pairs[k] as u64 + 1));
                    k += 1;
                }
                expr.push_str(&format!(" {} d", t - 1));
                cur = solver.gaussproof.pol_line(&expr);
            }
            proof_work += n as u64 * n as u64;
            cur
        }
        Source::Count(ci) => {
            let ci = *ci as usize;
            match solver.gaussproof.stream() {
                Some((w, fresh)) => g.reading.prove(ci, w, fresh, u64::MAX, &mut proof_work).unwrap_or(0),
                None => 0,
            }
        }
        Source::Row(gi, ri) => {
            let row = &g.grids[*gi as usize].rows[*ri as usize];
            match solver.gaussproof.stream() {
                Some((w, _)) => crate::gridread::prove_row(row, w, u64::MAX, &mut proof_work).unwrap_or(0),
                None => 0,
            }
        }
        Source::Col(gi, ci) => {
            let col = &g.grids[*gi as usize].cols[*ci as usize];
            match solver.gaussproof.stream() {
                Some((w, _)) => crate::gridread::prove_col(col, w, u64::MAX, &mut proof_work).unwrap_or(0),
                None => 0,
            }
        }
    };
    if id == 0 {
        crate::error::fatal(format_args!("card engine: a constraint read as provable has no proof"));
    }
    g.cards[c as usize].id = id;
    let _ = proof_work; // charged by the plan's lines, the same with no proof file
    id
}

/// The lemma's lines in the stream, if there is one.
fn log_lemma(solver: &mut Solver, g: &mut State, cards: &[u32], clause: &[i32]) {
    if !solver.gaussproof.active() {
        return;
    }
    let ids: Vec<u64> = cards.iter().map(|&c| prove_card(solver, g, c)).collect();
    solver.gaussproof.lemma(&ids, clause);
}

enum Out {
    Nothing,
    Conflict(Conflict),
    Assigned,
    /// A root clause was added: nothing to propagate, the look goes on.
    Added,
}

/// The engine's look at a fixpoint: at most the work its effort share allows.
pub fn check(solver: &mut Solver) -> Option<Conflict> {
    let mut g = std::mem::take(&mut solver.card);
    solver.card.s = g.s;
    g.again = false;
    // looks that keep finding nothing come further and further apart
    g.next_check = solver.statistics.conflicts + ((solver.options.cardint as u64).max(1) << g.empty_looks);
    let effort = solver.options.cardeffort as u64;
    let allowed = (solver.statistics.search_ticks / 1000).saturating_mul(effort).saturating_add(10_000_000);
    if effort != 0 && solver.statistics.card_ticks >= allowed {
        solver.card = g;
        return None;
    }
    g.s.checks += 1;
    let mut room = if effort != 0 { allowed - solver.statistics.card_ticks } else { u64::MAX };
    let look = (solver.options.cardlook as u64).saturating_mul(1000);
    let most = if look != 0 { look } else { u64::MAX };
    room = room.min(most);
    if solver.limited.ticks {
        room = room.min(solver.limits.ticks.saturating_sub(solver.statistics.work_clock()));
    }
    g.room = room;
    // the new part of the trail marks the networks it made tighter; what the limit
    // leaves is marked at the next look
    let end = solver.trail.len();
    let start = g.cursor.min(end);
    let mut i = start;
    let mut scanned = 0u64;
    while i < end && g.work + scanned / 4 < room {
        scanned += mark_literal(solver, &mut g, solver.trail[i]);
        i += 1;
    }
    g.work += scanned / 4;
    g.cursor = i;
    let backoff = solver.options.cardyield.clamp(0, 30) as u8;
    let mut result = None;
    let mut found = false;
    while g.work < room {
        let Some(n) = g.queue.pop_front() else { break };
        let net = &mut g.nets[n as usize];
        if net.dead {
            g.dirty[n as usize] = false;
            continue;
        }
        if net.skip > 0 {
            net.skip -= 1;
            g.dirty[n as usize] = false;
            g.s.skipped += 1;
            g.work += 1;
            continue;
        }
        if net.cost > room - g.work {
            g.s.no_room += 1;
            if net.cost > most {
                // no look may read it: it comes back when the trail marks it again
                g.dirty[n as usize] = false;
                g.work += 1;
                continue;
            }
            // it waits at the front, and the limit grows with the search: waiting
            // spends nothing, so the room for a whole read builds up
            g.queue.push_front(n);
            break;
        }
        g.dirty[n as usize] = false;
        g.s.nets_read += 1;
        g.current_kind = net.kind;
        let before = g.work;
        g.cut = false;
        let out = eval_net(solver, &mut g, n as usize);
        publish(solver, &mut g);
        let spent = g.work - before;
        let cut = matches!(out, Out::Nothing) && g.cut;
        let net = &mut g.nets[n as usize];
        // a read cut short says only that the whole read costs more than it got
        net.cost = if cut { net.cost.max(spent.saturating_mul(2)) } else { spent };
        if let Out::Nothing = out {
            // it counts as a read that found nothing, too
            net.fails = (net.fails + 1).min(backoff);
            net.skip = (1u32 << net.fails) - 1;
            if cut {
                g.s.cut_short += 1;
            }
        } else {
            net.fails = 0;
            net.skip = 0;
            found = true;
        }
        match out {
            Out::Nothing | Out::Added => {}
            Out::Conflict(c) => {
                result = Some(c);
                break;
            }
            Out::Assigned => {
                g.again = true;
                break;
            }
        }
    }
    g.empty_looks = if found { 0 } else { (g.empty_looks + 1).min(solver.options.cardgrow.clamp(0, 30) as u32) };
    publish(solver, &mut g);
    g.s.look_max = g.s.look_max.max(g.work);
    g.work = 0;
    g.published = 0;
    // a backtrack inside the delivery ran with the state taken out
    if g.cursor > solver.trail.len() {
        g.cursor = solver.trail.len();
    }
    solver.card = g;
    result
}

/// The work so far goes on the solver's clocks: a signal can end the run at any point.
#[inline]
fn publish(solver: &mut Solver, g: &mut State) {
    let fresh = g.work - g.published;
    solver.statistics.card_ticks += fresh;
    solver.statistics.ticks += fresh;
    g.published = g.work;
}

/// The look's limit is reached or the run is stopping: the read in progress ends.
#[inline]
fn over(solver: &mut Solver, g: &mut State) -> bool {
    publish(solver, g);
    let stop = g.work >= g.room || solver.termination.flagged.load(std::sync::atomic::Ordering::Relaxed);
    g.cut |= stop;
    stop
}

/// A scan of this many items, paid before it starts: false when the look has no room
/// for all of it, and then nothing is charged. Every scan in a read goes through this,
/// so a read ends at the look's limit, whatever the shape of the network.
#[inline]
fn pay(g: &mut State, items: u64) -> bool {
    if items > g.room.saturating_sub(g.work) {
        g.cut = true;
        return false;
    }
    g.work += items;
    true
}

/// Whether the look has room to add up these Cards and read the sum, which costs up
/// to twice their literals. A sum is one piece of work: done whole or not at all.
fn sum_fits(g: &mut State, cards: &[u32]) -> bool {
    let size: u64 = cards.iter().map(|&c| g.cards[c as usize].lits.len() as u64).sum();
    g.work += cards.len() as u64 / 4;
    if size.saturating_mul(2) <= g.room.saturating_sub(g.work) {
        true
    } else {
        g.s.no_room += 1;
        g.cut = true;
        false
    }
}

/// The sum of these Cards with opposite literals cancelled: `coef` holds its
/// coefficients, `touched` its literals; returns the right-hand side.
fn sum(g: &mut State, cards: &[u32]) -> i64 {
    let mut rhs = 0i64;
    for &c in cards {
        let card = &g.cards[c as usize];
        rhs += card.need as i64;
        for &l in &card.lits {
            let x = lx(l);
            if g.coef[x] == 0 {
                g.touched.push(l);
            }
            g.coef[x] += 1;
        }
        g.work += card.lits.len() as u64;
    }
    for i in 0..g.touched.len() {
        let l = g.touched[i];
        let (a, b) = (g.coef[lx(l)], g.coef[lx(-l)]);
        let m = a.min(b);
        if m > 0 {
            g.coef[lx(l)] -= m;
            g.coef[lx(-l)] -= m;
            rhs -= m as i64;
        }
    }
    rhs
}

fn clear_sum(g: &mut State) {
    for i in 0..g.touched.len() {
        let l = g.touched[i];
        g.coef[lx(l)] = 0;
        g.coef[lx(-l)] = 0;
    }
    g.touched.clear();
}

/// What exact arithmetic on the current sum says.
struct Reading {
    /// The sum's false literals, with coefficients.
    falses: Vec<(i32, i32)>,
    /// Open literals with coefficients.
    opens: Vec<(i32, i32)>,
    /// Right-hand side minus the most the rest can give.
    deficit: i64,
}

fn read_sum(solver: &Solver, g: &mut State, rhs: i64) -> Reading {
    let mut falses = Vec::new();
    let mut opens = Vec::new();
    let mut most = 0i64;
    for i in 0..g.touched.len() {
        let l = g.touched[i];
        let c = g.coef[lx(l)];
        if c <= 0 {
            continue;
        }
        match value(solver, l) {
            -1 => falses.push((l, c)),
            0 => {
                most += c as i64;
                opens.push((l, c));
            }
            _ => most += c as i64,
        }
    }
    g.work += g.touched.len() as u64;
    Reading { falses, opens, deficit: rhs - most }
}

/// The lemma's false literals: all of them, less any the deficit can spare,
/// the highest levels first.
fn lemma_of(solver: &Solver, r: &Reading) -> Vec<i32> {
    let mut f = r.falses.clone();
    if r.deficit > 1 {
        f.sort_by_key(|&(l, _)| std::cmp::Reverse(solver.assigned[idx(internal(solver, l)) as usize].level));
        let mut spare = r.deficit - 1;
        f.retain(|&(_, c)| {
            if (c as i64) <= spare {
                spare -= c as i64;
                false
            } else {
                true
            }
        });
    }
    f.into_iter().map(|(l, _)| l).collect()
}

fn count_lemma(g: &mut State, len: usize) {
    g.s.by_kind[g.current_kind as usize] += 1;
    let bucket = match len {
        0 | 1 => 0,
        2 => 1,
        3..=4 => 2,
        5..=8 => 3,
        9..=16 => 4,
        17..=64 => 5,
        _ => 6,
    };
    g.s.by_len[bucket] += 1;
}

/// A false clause to analyze, or a clause that forces `y`.
fn deliver(solver: &mut Solver, g: &mut State, cards: &[u32], falses: &[i32], y: Option<i32>) -> Out {
    let mut buf: Vec<u32> = Vec::with_capacity(falses.len() + 1);
    if let Some(y) = y {
        buf.push(internal(solver, y));
    }
    let start = buf.len();
    for &l in falses {
        let lit = internal(solver, l);
        if solver.assigned[idx(lit) as usize].level != 0 {
            buf.push(lit);
        }
    }
    buf[start..].sort_unstable();
    let mut w = start;
    for r in start..buf.len() {
        if r == start || buf[r] != buf[w - 1] {
            buf[w] = buf[r];
            w += 1;
        }
    }
    buf.truncate(w);
    if buf.len() > solver.options.cardmaxlen as usize {
        g.s.too_long += 1;
        return Out::Nothing;
    }
    // a contradiction and a unit are always worth having; a forced literal with a
    // short reason may be what the clauses nearly say already
    if y.is_some() && buf.len() >= 2 && (buf.len() as i32) < solver.options.cardminlen {
        g.s.too_short += 1;
        return Out::Nothing;
    }
    if !budget_allows(solver, g, cards) {
        return Out::Nothing;
    }
    count_lemma(g, buf.len());
    // the proof's copy holds every literal; the solver's leaves out those false
    // at level zero (its DRAT line follows from the proved one and the units)
    let mut clause: Vec<i32> = Vec::with_capacity(falses.len() + 1);
    if let Some(y) = y {
        clause.push(y);
    }
    clause.extend_from_slice(falses);
    log_lemma(solver, g, cards, &clause);
    g.work += clause.len() as u64 + cards.len() as u64;
    g.s.lemma_lits += buf.len() as u64;
    match y {
        Some(_) => {
            g.s.forced += 1;
            let lit = buf[0];
            if buf.len() == 1 {
                g.s.units += 1;
                // a unit: assigned at level zero with the trail out of order,
                // as the solver does for a learned unit under chronological
                // backtracking
                crate::assign::learned_unit(solver, lit);
                return Out::Assigned;
            }
            let (reference, a, b) = materialize(solver, buf, true);
            debug_assert!(a == lit);
            let level = solver.assigned[idx(b) as usize].level;
            let probing = solver.probing;
            if reference == crate::reference::INVALID_REF {
                crate::assign::assign(solver, probing, level, true, lit, b);
            } else {
                crate::assign::assign(solver, probing, level, false, lit, reference);
            }
            Out::Assigned
        }
        None => {
            g.s.violations += 1;
            let size = buf.len();
            if size == 0 {
                // every literal is false at level zero, wherever the search is
                if solver.level != 0 {
                    crate::backtrack::backtrack_after_conflict(solver, 0);
                    g.cursor = 0;
                }
                // counting this conflict at level zero makes the solver inconsistent
                let _ = crate::propsearch::binary_conflict(solver, 0, 0);
                return Out::Conflict(Conflict::Binary);
            }
            if size == 1 {
                let lit = buf[0];
                g.s.units += 1;
                crate::backtrack::backtrack_after_conflict(solver, 0);
                g.cursor = 0;
                crate::assign::learned_unit(solver, lit);
                return Out::Assigned;
            }
            let (reference, a, b) = materialize(solver, buf, false);
            if reference == crate::reference::INVALID_REF {
                Out::Conflict(crate::propsearch::binary_conflict(solver, a, b))
            } else {
                Out::Conflict(Conflict::Clause(reference))
            }
        }
    }
}

/// The clause into the arena as a redundant clause, the propagated literal
/// first and the highest-level false literals next (the watches, as for a
/// learned clause), or as a binary clause. Same reasoning as the parity
/// engine's `materialize`.
fn materialize(solver: &mut Solver, mut lits: Vec<u32>, propagated: bool) -> (crate::reference::Reference, u32, u32) {
    let size = lits.len();
    let start = if propagated { 1 } else { 0 };
    let end = (start + 2).min(size);
    for slot in start..end {
        let mut best = slot;
        let mut best_level = solver.assigned[idx(lits[slot]) as usize].level;
        for p in slot + 1..size {
            let level = solver.assigned[idx(lits[p]) as usize].level;
            if level > best_level {
                best_level = level;
                best = p;
            }
        }
        lits.swap(slot, best);
    }
    let (a, b) = (lits[0], lits[1]);
    let reference = if size == 2 {
        let _ = crate::clause::new_redundant_clause_from(solver, 1, &lits);
        crate::reference::INVALID_REF
    } else {
        let mut levels: Vec<u32> = lits[start..].iter().map(|&l| solver.assigned[idx(l) as usize].level).collect();
        levels.sort_unstable();
        levels.dedup();
        crate::clause::new_redundant_clause_from(solver, levels.len() as u32, &lits)
    };
    (reference, a, b)
}

/// One network at the current assignment.
fn eval_net(solver: &mut Solver, g: &mut State, n: usize) -> Out {
    let row_cards = std::mem::take(&mut g.nets[n].rows);
    let cap_cards = std::mem::take(&mut g.nets[n].caps);
    let out = eval_inner(solver, g, &row_cards, &cap_cards);
    g.nets[n].rows = row_cards;
    g.nets[n].caps = cap_cards;
    out
}

fn eval_inner(solver: &mut Solver, g: &mut State, row_cards: &[u32], cap_cards: &[u32]) -> Out {
    // the caps: what each still allows, and which open literal uses which
    let mut marked_caps: Vec<i32> = Vec::new();
    let mut free: Vec<i32> = Vec::with_capacity(cap_cards.len());
    let mut cut = false;
    for (ci, &card) in cap_cards.iter().enumerate() {
        let len = g.cards[card as usize].lits.len() as u64;
        if over(solver, g) || !pay(g, len) {
            cut = true;
            break;
        }
        let c = &g.cards[card as usize];
        let mut trues = 0i32; // literals of the cap false now: items true
        for &l in &c.lits {
            match value(solver, l) {
                -1 => trues += 1,
                0 => {
                    let x = lx(-l);
                    if g.cap_of[x] == 0 {
                        g.cap_of[x] = ci as u32 + 1;
                        marked_caps.push(-l);
                    }
                }
                _ => {}
            }
        }
        free.push(c.lits.len() as i32 - c.need - trues);
    }
    // the rows: what each still wants, and its edges
    struct Row {
        card: u32,
        want: i32,
        first: u32,
        last: u32,
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut edge_lit: Vec<i32> = Vec::new();
    let mut edge_cap: Vec<u32> = Vec::new();
    let mut edge_row: Vec<u32> = Vec::new();
    let mut marked_rows: Vec<i32> = Vec::new();
    // the rows that still want something, those with the fewest open literals
    // first: they leave the most room for the others
    let mut wanting: Vec<(u32, u32, i32)> = Vec::new(); // open count, card, want
    for &card in row_cards {
        let len = g.cards[card as usize].lits.len() as u64;
        if cut || over(solver, g) || !pay(g, len) {
            cut = true;
            break;
        }
        let c = &g.cards[card as usize];
        let mut trues = 0i32;
        let mut outside = 0i32;
        let mut opened = 0u32;
        let mut skip = false;
        for &l in &c.lits {
            match value(solver, l) {
                1 => trues += 1,
                0 => {
                    if g.cap_of[lx(l)] != 0 {
                        opened += 1;
                    } else {
                        outside += 1;
                    }
                }
                -1 => {}
                _ => outside += 1,
            }
            if trues + outside >= c.need {
                skip = true;
                break;
            }
        }
        if !skip {
            wanting.push((opened, card, c.need - trues - outside));
        }
    }
    if !cut && !pay(g, wanting.len() as u64) {
        cut = true;
    }
    if !cut {
        wanting.sort_unstable();
    }
    for &(_, card, want) in &wanting {
        let len = g.cards[card as usize].lits.len() as u64;
        if cut || over(solver, g) || !pay(g, len) {
            cut = true;
            break;
        }
        let c = &g.cards[card as usize];
        let mut clash = false;
        for &l in &c.lits {
            if value(solver, l) == 0 && g.cap_of[lx(l)] != 0 && g.owner[lx(l)] != 0 {
                // rows of a network share no open literal: one true literal
                // would serve two rows, and the sum would not be tight
                clash = true;
                break;
            }
        }
        if clash {
            continue;
        }
        let r = rows.len() as u32;
        let first = edge_lit.len() as u32;
        for &l in &c.lits {
            if value(solver, l) == 0 && g.cap_of[lx(l)] != 0 {
                g.owner[lx(l)] = r + 1;
                marked_rows.push(l);
                edge_lit.push(l);
                edge_cap.push(g.cap_of[lx(l)] - 1);
                edge_row.push(r);
            }
        }
        rows.push(Row { card, want, first, last: edge_lit.len() as u32 });
    }
    let out = if cut {
        Out::Nothing
    } else {
        flow_and_read(solver, g, &rows.iter().map(|r| (r.card, r.want, r.first, r.last)).collect::<Vec<_>>(), &edge_lit, &edge_cap, &edge_row, cap_cards, &mut free)
    };
    for &x in &marked_caps {
        g.cap_of[lx(x)] = 0;
    }
    for &x in &marked_rows {
        g.owner[lx(x)] = 0;
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn flow_and_read(
    solver: &mut Solver,
    g: &mut State,
    rows: &[(u32, i32, u32, u32)],
    edge_lit: &[i32],
    edge_cap: &[u32],
    edge_row: &[u32],
    cap_cards: &[u32],
    free: &mut [i32],
) -> Out {
    let ncaps = cap_cards.len();
    let nrows = rows.len();
    if nrows == 0 {
        return Out::Nothing;
    }
    let mut used = vec![false; edge_lit.len()];
    let mut cap_edges: Vec<Vec<u32>> = vec![Vec::new(); ncaps];
    for (e, &c) in edge_cap.iter().enumerate() {
        cap_edges[c as usize].push(e as u32);
    }
    let mut row_seen = vec![u32::MAX; nrows];
    let mut cap_seen = vec![u32::MAX; ncaps];
    let mut row_parent = vec![u32::MAX; nrows];
    let mut cap_parent = vec![u32::MAX; ncaps];
    let mut stack: Vec<u32> = Vec::new();
    let mut stamp = 0u32;
    for (r, &(_, want, first, last)) in rows.iter().enumerate() {
        for _unit in 0..want {
            if over(solver, g) {
                return Out::Nothing;
            }
            // a free cap directly
            if !pay(g, (last - first) as u64) {
                return Out::Nothing;
            }
            let mut done = false;
            for e in first..last {
                let c = edge_cap[e as usize] as usize;
                if !used[e as usize] && free[c] > 0 {
                    used[e as usize] = true;
                    free[c] -= 1;
                    done = true;
                    break;
                }
            }
            if done {
                continue;
            }
            // an alternating search
            stamp += 1;
            row_seen[r] = stamp;
            row_parent[r] = u32::MAX;
            stack.clear();
            stack.push(r as u32);
            let mut end_edge = u32::MAX;
            'search: while let Some(x) = stack.pop() {
                let (_, _, f, l) = rows[x as usize];
                if over(solver, g) || !pay(g, (l - f) as u64) {
                    return Out::Nothing;
                }
                for e in f..l {
                    if used[e as usize] {
                        continue;
                    }
                    let c = edge_cap[e as usize] as usize;
                    if cap_seen[c] == stamp {
                        continue;
                    }
                    cap_seen[c] = stamp;
                    cap_parent[c] = e;
                    if free[c] > 0 {
                        end_edge = e;
                        break 'search;
                    }
                    if !pay(g, cap_edges[c].len() as u64) {
                        return Out::Nothing;
                    }
                    for &fe in &cap_edges[c] {
                        if used[fe as usize] {
                            let s = edge_row[fe as usize] as usize;
                            if row_seen[s] != stamp {
                                row_seen[s] = stamp;
                                row_parent[s] = fe;
                                stack.push(s as u32);
                            }
                        }
                    }
                }
            }
            if end_edge == u32::MAX {
                // too tight: the rows and caps this search reached
                let mut cards: Vec<u32> = Vec::new();
                for (s, &seen) in row_seen.iter().enumerate() {
                    if seen == stamp {
                        cards.push(rows[s].0);
                    }
                }
                for (c, &seen) in cap_seen.iter().enumerate() {
                    if seen == stamp {
                        cards.push(cap_cards[c]);
                    }
                }
                if !sum_fits(g, &cards) {
                    return Out::Nothing;
                }
                let rhs = sum(g, &cards);
                let reading = read_sum(solver, g, rhs);
                clear_sum(g);
                if reading.deficit <= 0 {
                    g.s.not_sound += 1;
                    return Out::Nothing;
                }
                let lemma = lemma_of(solver, &reading);
                return deliver(solver, g, &cards, &lemma, None);
            }
            // augment back along the search
            let last_cap = edge_cap[end_edge as usize] as usize;
            free[last_cap] -= 1;
            let mut e = end_edge;
            loop {
                used[e as usize] = true;
                let s = edge_row[e as usize] as usize;
                let fe = row_parent[s];
                if fe == u32::MAX {
                    break;
                }
                used[fe as usize] = false;
                e = cap_parent[edge_cap[fe as usize] as usize];
            }
        }
    }
    let at_root = solver.level == 0 && solver.options.cardroot != 0;
    let want_forced = solver.options.cardforced != 0;
    if !want_forced && !at_root {
        return Out::Nothing;
    }
    // which caps can still give a unit back: those with room, and those a row
    // could leave for one that can
    let mut can_free = vec![false; ncaps];
    let mut stack2: Vec<usize> = Vec::new();
    for c in 0..ncaps {
        if free[c] > 0 {
            can_free[c] = true;
            stack2.push(c);
        }
    }
    while let Some(d) = stack2.pop() {
        if over(solver, g) || !pay(g, cap_edges[d].len() as u64) {
            return Out::Nothing;
        }
        for &e in &cap_edges[d] {
            if used[e as usize] {
                continue;
            }
            let s = edge_row[e as usize] as usize;
            let (_, _, f, l) = rows[s];
            if !pay(g, (l - f) as u64) {
                return Out::Nothing;
            }
            for fe in f..l {
                if used[fe as usize] {
                    let c = edge_cap[fe as usize] as usize;
                    if !can_free[c] {
                        can_free[c] = true;
                        stack2.push(c);
                    }
                }
            }
        }
    }
    // each tight cap's closure is a sum with no slack: its open literals with
    // a coefficient above the slack are forced
    let maxper = solver.options.cardmaxper as u64;
    let mut delivered = 0u64;
    let mut any = false;
    let mut in_closure = vec![0u32; ncaps];
    let mut row_in = vec![0u32; nrows];
    let mut closure_stamp = 0u32;
    let mut done_caps = vec![false; ncaps];
    let mut pending: Vec<(Vec<i32>, i32, Vec<u32>)> = Vec::new();
    let mut tight: Vec<Vec<u32>> = Vec::new();
    'closures: for c0 in 0..ncaps {
        if can_free[c0] || done_caps[c0] {
            continue;
        }
        // what was found so far is paid for: it is still delivered
        if over(solver, g) {
            break;
        }
        closure_stamp += 1;
        let mut caps_here: Vec<usize> = vec![c0];
        let mut rows_here: Vec<usize> = Vec::new();
        in_closure[c0] = closure_stamp;
        let mut i = 0;
        while i < caps_here.len() {
            let c = caps_here[i];
            if over(solver, g) || !pay(g, cap_edges[c].len() as u64) {
                break 'closures;
            }
            i += 1;
            done_caps[c] = true;
            for &e in &cap_edges[c] {
                if !used[e as usize] {
                    continue;
                }
                let s = edge_row[e as usize] as usize;
                if row_in[s] == closure_stamp {
                    continue;
                }
                row_in[s] = closure_stamp;
                rows_here.push(s);
                let (_, _, f, l) = rows[s];
                if !pay(g, (l - f) as u64) {
                    break 'closures;
                }
                for fe in f..l {
                    if used[fe as usize] {
                        continue;
                    }
                    let d = edge_cap[fe as usize] as usize;
                    if in_closure[d] != closure_stamp {
                        in_closure[d] = closure_stamp;
                        caps_here.push(d);
                    }
                }
            }
        }
        let mut cards: Vec<u32> = rows_here.iter().map(|&s| rows[s].0).collect();
        cards.extend(caps_here.iter().map(|&c| cap_cards[c]));
        if !sum_fits(g, &cards) {
            break;
        }
        let rhs = sum(g, &cards);
        let reading = read_sum(solver, g, rhs);
        clear_sum(g);
        if reading.deficit > 0 {
            continue; // the search above would have found it; never trust it here
        }
        let slack = -reading.deficit;
        if at_root && slack == 0 && reading.opens.is_empty() {
            tight.push(cards.clone());
        }
        let falses: Vec<i32> = reading.falses.iter().map(|&(l, _)| l).collect();
        if want_forced {
            for &(y, c) in &reading.opens {
                if pending.len() as u64 >= maxper {
                    break;
                }
                if (c as i64) > slack {
                    pending.push((falses.clone(), y, cards.clone()));
                    g.work += (falses.len() + cards.len()) as u64;
                }
            }
        }
        if pending.len() as u64 >= maxper {
            break;
        }
    }
    // the shortest first
    pending.sort_by_key(|p| p.0.len());
    for (falses, y, cards) in pending {
        if delivered >= maxper {
            break;
        }
        // an earlier delivery may have assigned it through a shared literal
        if value(solver, y) != 0 || falses.iter().any(|&l| value(solver, l) != -1) {
            continue;
        }
        if let Out::Assigned = deliver(solver, g, &cards, &falses, Some(y)) {
            delivered += 1;
            any = true;
        }
    }
    if any {
        return Out::Assigned;
    }
    root_lemmas(solver, g, &tight)
}

/// A closure at the root with no slack and nothing left over: its Cards add up
/// to 0 >= 0, so each holds with equality, and the sum of the others says what
/// a Card's own sum cannot. When exactly one of a Card's open literals must stay
/// false, that sum is a clause: a column of a tight grid is used, a group of a
/// tight count holds a true. The arithmetic on that sum decides, as everywhere.
fn root_lemmas(solver: &mut Solver, g: &mut State, tight: &[Vec<u32>]) -> Out {
    let most = solver.options.cardrootmax as u64;
    let mut out = Out::Nothing;
    for cards in tight {
        for &x in cards {
            if g.s.root_clauses >= most || over(solver, g) {
                return out;
            }
            if !pay(g, g.cards[x as usize].lits.len() as u64) {
                return out;
            }
            let card = &g.cards[x as usize];
            let (mut open, mut need) = (0i32, card.need);
            for &l in &card.lits {
                match value(solver, l) {
                    1 => need -= 1,
                    0 => open += 1,
                    _ => {}
                }
            }
            if open - need != 1 || cards.len() < 2 {
                continue;
            }
            let others: Vec<u32> = cards.iter().copied().filter(|&c| c != x).collect();
            if !sum_fits(g, &others) {
                return out;
            }
            let rhs = sum(g, &others);
            let mut clause: Vec<i32> = Vec::new();
            let mut opens: Vec<u32> = Vec::new();
            let mut usable = rhs >= 1;
            for i in 0..g.touched.len() {
                let l = g.touched[i];
                if g.coef[lx(l)] <= 0 {
                    continue;
                }
                clause.push(l);
                match value(solver, l) {
                    0 => opens.push(internal(solver, l)),
                    -1 => {}
                    _ => usable = false, // true now, or no longer in the solver
                }
            }
            g.work += g.touched.len() as u64;
            clear_sum(g);
            if !usable || opens.len() < 2 {
                // one open literal would be a forced one: the look above has it
                continue;
            }
            if opens.len() > solver.options.cardmaxlen as usize {
                g.s.too_long += 1;
                continue;
            }
            let hash = clause_hash(&clause);
            if g.known.contains(&hash) {
                continue;
            }
            if !budget_allows(solver, g, &others) {
                continue;
            }
            g.known.insert(hash);
            log_lemma(solver, g, &others, &clause);
            g.work += clause.len() as u64 + others.len() as u64;
            g.s.root_clauses += 1;
            count_lemma(g, opens.len());
            g.s.lemma_lits += opens.len() as u64;
            let _ = crate::clause::new_redundant_clause_from(solver, 1, &opens);
            out = Out::Added;
        }
    }
    out
}

pub fn report(solver: &Solver) {
    let g = &solver.card;
    if solver.options.card == 0 {
        return;
    }
    println!(
        "c card nets={} cards={} counts={} forms={},{},{},{},{},{},{} grids={} read_ticks={},{},{} planned_terms={} checks={} nets_read={} skipped={} cut_short={} no_room={} look_max={} root_clauses={} violations={} forced={} units={} too_long={} too_short={} no_budget={} not_sound={} mean_len={:.1} by_kind={},{},{} by_len={},{},{},{},{},{},{} ticks={}",
        g.s.nets,
        g.s.cards,
        g.s.counts_found,
        g.s.forms_found[0],
        g.s.forms_found[1],
        g.s.forms_found[2],
        g.s.forms_found[3],
        g.s.forms_found[4],
        g.s.forms_found[5],
        g.s.forms_found[6],
        g.s.grids_found,
        g.s.count_read_ticks,
        g.s.grid_read_ticks,
        g.s.build_ticks,
        g.s.planned_terms,
        g.s.checks,
        g.s.nets_read,
        g.s.skipped,
        g.s.cut_short,
        g.s.no_room,
        g.s.look_max,
        g.s.root_clauses,
        g.s.violations,
        g.s.forced,
        g.s.units,
        g.s.too_long,
        g.s.too_short,
        g.s.no_budget,
        g.s.not_sound,
        g.s.lemma_lits as f64 / (g.s.violations + g.s.forced + g.s.root_clauses).max(1) as f64,
        g.s.by_kind[0],
        g.s.by_kind[1],
        g.s.by_kind[2],
        g.s.by_len[0],
        g.s.by_len[1],
        g.s.by_len[2],
        g.s.by_len[3],
        g.s.by_len[4],
        g.s.by_len[5],
        g.s.by_len[6],
        solver.statistics.card_ticks
    );
}
