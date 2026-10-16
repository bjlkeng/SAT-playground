//! Not in kissat. The parity engine (plan section 11, 2026-10-15; bead
//! SAT-playground-1v2.7): XOR constraints reasoned about inside search.
//!
//! An XOR constraint says that an odd (or even) number of its variables
//! are true. Clauses handle a long XOR badly: a clause propagates only
//! when one of its literals is left, and the sum of two XORs has no short
//! clause form at all. The parity pass settles closed XOR systems before
//! search; this engine handles XORs mixed with other clauses, during
//! search, the way CryptoMiniSat does (Han and Jiang, "When Boolean
//! Satisfiability Meets Gaussian Elimination in a Simplex Way", 2012).
//!
//! The XOR constraints the parity pass extracts from the input are rows
//! of a matrix over GF(2): one bit per variable, plus the parity. The
//! matrix is kept in reduced row echelon form: every row has a basic
//! variable that occurs in no other row. A row watches its basic variable
//! and one other unassigned variable, so while both are unassigned the
//! row can neither propagate nor be false. After clause propagation
//! reaches a fixpoint the engine reads the new part of the trail. A row
//! whose other watch was assigned moves it to another unassigned
//! variable, or, when none is left, propagates its basic variable, or, with
//! everything assigned, is satisfied or a conflict. A row whose basic
//! variable was assigned takes a new basic variable among its unassigned
//! ones and adds itself to every other row holding that variable (the
//! Jordan step). This keeps the propagation complete: whatever the XOR
//! system implies under the assignment, some row then has one unassigned
//! variable. Backtracking undoes nothing in the matrix: the rows stay a
//! basis of the same space, and the engine's per-variable bits are
//! cleared as the solver unassigns. After inprocessing, which assigns at
//! level zero without the engine, it re-reads every row.
//!
//! A propagation or a conflict puts the row's clause (every literal of the
//! row false, but the propagated one) into the clause arena as a
//! redundant clause: the reason of the assignment, or the conflict handed
//! to analyze. Rows are sums of input XORs alone, with no unit substituted
//! and no learned fact used, so each such clause follows from the input
//! and the proof of it is a few cutting-planes lines in the structure
//! pass's VeriPB stream (`gaussproof`, bead X.2). The variables of the
//! matrix are frozen: never eliminated and never substituted, so the
//! columns stay live variables of the solver. The engine's work is
//! charged to the work clock (`gauss_ticks`).
//!
//! With `gauss=0` nothing here runs and the solver is the stock one.

use crate::internal::Solver;
use crate::literal::{idx, not};
use crate::propsearch::Conflict;

const INVALID: u32 = u32::MAX;

/// The engine's work between two checks of its yield (ticks).
const YIELD_WINDOW: u64 = 1 << 26;

/// What inspecting a row finds.
enum Status {
    /// At least two variables unassigned (the row still needs two watches).
    Open,
    /// Exactly one variable unassigned: the column and the value it must take.
    Unit(u32, bool),
    /// Every variable assigned with the right parity.
    Satisfied,
    /// Every variable assigned with the wrong parity.
    Falsified,
}

struct Row {
    /// The XOR of the row's variables must equal this (0 or 1).
    parity: u32,
    /// The row's pivot column: in no other row.
    basic: u32,
    /// The second watched column, or INVALID for a row of one variable.
    other: u32,
    /// False once the row is a settled unit or empty: it is skipped.
    live: bool,
}

#[derive(Default)]
pub struct State {
    /// The engine runs (XORs were found and fit the limits).
    pub on: bool,
    /// The matrix exists (`build` ran); before that only `init`'s
    /// per-variable tables do.
    built: bool,
    /// Set around inprocessing: the engine neither reads the trail nor
    /// propagates; `resync` afterwards re-reads every row.
    pub suspended: bool,
    /// Per internal variable index: the variable is a matrix column and
    /// must stay a live variable (not eliminated, not substituted).
    pub frozen: Vec<bool>,
    /// The XOR constraints the structure pass extracted (external
    /// variables, parity), kept until `build`.
    pub xors: Vec<(Vec<i32>, u32)>,
    /// Per external variable: it occurs in a clause outside the XORs. One
    /// that does not and is in at most two XORs is a helper of an
    /// encoding; its row is dropped once it is a pivot (see `build`).
    pub mixed: Vec<bool>,
    /// Per column: the variable is mixed (see `mixed`).
    col_mixed: Vec<bool>,
    /// How far along the trail the engine has read.
    pub cursor: usize,
    /// Per internal variable index: its column, for the variables the
    /// solver still has at the build (INVALID for the rest).
    col_of: Vec<u32>,
    /// Per column: the external variable, and the internal literal of its
    /// positive side (a fixed variable's may be shared with other fixed
    /// ones after a compaction, which is why `col_of` is not its inverse).
    ext_of: Vec<i32>,
    lit_of: Vec<u32>,
    /// The external literals of the clause being built (the proof's copy).
    proof_clause: Vec<i32>,
    words: usize,
    rows: Vec<Row>,
    bits: Vec<u64>,
    /// Per column: assigned as far as the engine knows; assigned true;
    /// read from the trail already.
    assigned: Vec<u64>,
    truth: Vec<u64>,
    scanned: Vec<u64>,
    /// Per column: the rows watching it.
    watch: Vec<Vec<u32>>,
    /// Rows whose basic variable was assigned, waiting for the Jordan step.
    queue: Vec<u32>,
    /// Work counted since the last charge to the work clock.
    work: u64,
    /// The work of building the matrix, outside the effort cap.
    build_ticks: u64,
    /// The engine's work at which the yield is next checked.
    next_yield_check: u64,
    // statistics for the exit report
    pub propagations: u64,
    pub conflicts: u64,
    pub additions: u64,
    pub units: u64,
    pub pivots: u64,
}

impl State {
    #[inline]
    fn row_bits(&self, r: usize) -> &[u64] {
        &self.bits[r * self.words..(r + 1) * self.words]
    }

    #[inline]
    fn has(&self, r: usize, col: u32) -> bool {
        self.bits[r * self.words + (col as usize >> 6)] >> (col & 63) & 1 != 0
    }

    #[inline]
    fn bit(v: &[u64], col: u32) -> bool {
        v[col as usize >> 6] >> (col & 63) & 1 != 0
    }

    #[inline]
    fn set(v: &mut [u64], col: u32, on: bool) {
        if on {
            v[col as usize >> 6] |= 1u64 << (col & 63);
        } else {
            v[col as usize >> 6] &= !(1u64 << (col & 63));
        }
    }

    /// The row's status under the engine's view of the assignment, and
    /// charges the row's words as work.
    fn status(&mut self, r: usize) -> Status {
        let words = self.words;
        self.work += words as u64;
        let bits = &self.bits[r * words..(r + 1) * words];
        let mut unassigned = 0u32;
        let mut last = INVALID;
        let mut trues = 0u32;
        for (i, &w) in bits.iter().enumerate() {
            let open = w & !self.assigned[i];
            if open != 0 {
                unassigned += open.count_ones();
                if unassigned <= 1 {
                    last = (i as u32) * 64 + open.trailing_zeros();
                }
                if unassigned > 1 {
                    return Status::Open;
                }
            }
            trues += (w & self.truth[i]).count_ones();
        }
        let sum = trues & 1;
        let parity = self.rows[r].parity;
        if unassigned == 1 {
            Status::Unit(last, sum != parity)
        } else if sum == parity {
            Status::Satisfied
        } else {
            Status::Falsified
        }
    }

    /// An unassigned column of the row other than `skip`, if any.
    fn unassigned_col(&mut self, r: usize, skip: u32) -> u32 {
        let words = self.words;
        self.work += words as u64;
        let bits = &self.bits[r * words..(r + 1) * words];
        for (i, &w) in bits.iter().enumerate() {
            let mut open = w & !self.assigned[i];
            while open != 0 {
                let col = (i as u32) * 64 + open.trailing_zeros();
                if col != skip {
                    return col;
                }
                open &= open - 1;
            }
        }
        INVALID
    }

    /// Any column of the row other than `skip`, assigned or not, or
    /// INVALID for a row of one variable: the second watch of a row with
    /// no unassigned column at level zero, where every assignment is for
    /// good (in search `latest_col` picks the backtrack-safe one).
    fn any_col(&mut self, r: usize, skip: u32) -> u32 {
        let words = self.words;
        self.work += words as u64;
        let bits = &self.bits[r * words..(r + 1) * words];
        for (i, &w) in bits.iter().enumerate() {
            let mut w = w;
            while w != 0 {
                let col = (i as u32) * 64 + w.trailing_zeros();
                if col != skip {
                    return col;
                }
                w &= w - 1;
            }
        }
        INVALID
    }

    /// row `r` ^= row `s` (bits and parity).
    fn add_row(&mut self, r: usize, s: usize) {
        let words = self.words;
        self.work += words as u64;
        let (lo, hi) = if r < s { (r, s) } else { (s, r) };
        let (a, b) = self.bits.split_at_mut(hi * words);
        let lo_slice = &mut a[lo * words..(lo + 1) * words];
        let hi_slice = &mut b[..words];
        if r < s {
            for (x, y) in lo_slice.iter_mut().zip(hi_slice.iter()) {
                *x ^= *y;
            }
        } else {
            for (x, y) in hi_slice.iter_mut().zip(lo_slice.iter()) {
                *x ^= *y;
            }
        }
        self.rows[r].parity ^= self.rows[s].parity;
        self.additions += 1;
    }
}

/// Which part of the solver's columns the engine may hold: the matrix
/// takes rows x columns bits, and with a proof the table of row sets
/// rows x rows bits more; both within `gausslimit` (millions of bits).
fn fits(solver: &Solver, rows: usize, cols: usize) -> bool {
    let limit = (solver.options.gausslimit as u64).saturating_mul(1 << 20);
    let mut bits = (rows as u64).saturating_mul(cols as u64);
    if solver.proof.is_some() {
        bits = bits.saturating_add(crate::gaussproof::State::table_bytes(rows).saturating_mul(8));
    }
    bits <= limit
}

/// Called once the formula is in the solver: whether the engine is wanted
/// (option on, at least two XORs). Nothing is frozen and nothing is
/// mapped yet: preprocessing runs as it would without the engine, and the
/// matrix is built after it over the variables that are left (an XOR
/// variable preprocessing removed is pivoted out and its row dropped).
pub fn init(solver: &mut Solver) {
    let g = &mut solver.gauss;
    g.on = false;
    if solver.options.gauss == 0 || g.xors.len() < 2 {
        g.xors = Vec::new();
        g.mixed = Vec::new();
        return;
    }
    crate::print::message(
        solver,
        format_args!("parity engine: {} XOR constraints in the input, the matrix is built after preprocessing", solver.gauss.xors.len()),
    );
}

/// Build the matrix at the start of search (after preprocessing): rows
/// from the XORs, reduced row echelon form, watches, and the units it
/// yields at level zero. Returns false when the matrix refutes the
/// formula (the solver is then inconsistent).
pub fn build(solver: &mut Solver) -> bool {
    if solver.gauss.xors.len() < 2 || solver.gauss.built {
        return true;
    }
    let xors = std::mem::take(&mut solver.gauss.xors);
    let mixed = std::mem::take(&mut solver.gauss.mixed);
    // the columns: every variable of an XOR; one preprocessing removed
    // (eliminated or substituted) is "gone", one fixed at level zero is
    // a column assigned for good, the rest map to their internal index
    let nvars = solver.vars() as usize;
    let mut col_of: Vec<u32> = vec![INVALID; nvars];
    let mut ext_of: Vec<i32> = Vec::new();
    let mut lit_of_col: Vec<u32> = Vec::new();
    let mut gone: Vec<bool> = Vec::new();
    let mut col_mixed: Vec<bool> = Vec::new();
    let mut col_index: std::collections::HashMap<i32, u32> = std::collections::HashMap::new();
    let mut fixed = 0usize;
    for (vars, _) in &xors {
        for &ev in vars {
            if col_index.contains_key(&ev) {
                continue;
            }
            let col = ext_of.len() as u32;
            col_index.insert(ev, col);
            ext_of.push(ev);
            col_mixed.push(mixed.get(ev as usize).copied().unwrap_or(true));
            let entry = solver.import_[ev as usize];
            if !entry.imported || entry.eliminated {
                gone.push(true);
                lit_of_col.push(crate::literal::INVALID_LIT);
                continue;
            }
            let ilit = entry.lit;
            gone.push(false);
            lit_of_col.push(ilit);
            if solver.values[ilit as usize] != 0 {
                fixed += 1; // assigned for good; never notified
            } else {
                col_of[idx(ilit) as usize] = col;
            }
        }
    }
    let cols = ext_of.len();
    if !fits(solver, xors.len(), cols) {
        crate::print::message(
            solver,
            format_args!("parity engine off: {} XORs over {} variables exceed gausslimit={}M bits", xors.len(), cols, solver.options.gausslimit),
        );
        return true;
    }
    let mut g = std::mem::take(&mut solver.gauss);
    let words = (cols + 63) / 64;
    g.words = words;
    g.col_of = col_of;
    g.ext_of = ext_of;
    g.lit_of = lit_of_col;
    g.col_mixed = col_mixed;
    g.assigned = vec![0; words];
    g.truth = vec![0; words];
    g.scanned = vec![0; words];
    g.watch = vec![Vec::new(); cols];
    // rows
    let mut rows: Vec<Row> = Vec::new();
    let mut bits: Vec<u64> = Vec::new();
    for (vars, parity) in &xors {
        let start = bits.len();
        bits.resize(start + words, 0);
        for &ev in vars {
            let col = col_index[&ev];
            bits[start + (col as usize >> 6)] |= 1u64 << (col & 63);
        }
        let p = *parity;
        rows.push(Row { parity: p, basic: INVALID, other: INVALID, live: true });
    }
    g.rows = rows;
    g.bits = bits;
    // the proof: row i starts as input XOR i
    if solver.gaussproof.active() {
        solver.gaussproof.rows(g.rows.len());
    }
    // the engine's view of the assignment: whatever is fixed at level
    // zero; a gone column counts as unassigned until it is pivoted out
    debug_assert!(solver.level == 0);
    for col in 0..cols as u32 {
        if gone[col as usize] {
            continue;
        }
        let val = solver.values[g.lit_of[col as usize] as usize];
        State::set(&mut g.assigned, col, val != 0);
        State::set(&mut g.truth, col, val > 0);
        State::set(&mut g.scanned, col, val != 0);
    }
    // Gauss-Jordan elimination: pivot columns in the order of fewest
    // occurrences (the chain variables of an encoding first), each
    // eliminated from every other row
    let n = g.rows.len();
    let mut occ: Vec<u32> = vec![0; cols];
    for r in 0..n {
        for (i, &w) in g.row_bits(r).iter().enumerate() {
            let mut w = w;
            while w != 0 {
                occ[i * 64 + w.trailing_zeros() as usize] += 1;
                w &= w - 1;
            }
        }
    }
    // a helper variable, in at most two XORs and in no other clause (the
    // chain variables of a long XOR's encoding), is pivoted first and its
    // row is dropped after the reduction: the matrix then holds the long
    // XORs the chains encode, as CryptoMiniSat's "xor together" does.
    // Other pure variables keep their rows: a variable the solver decides
    // on must stay in the matrix for the propagation to be complete
    // A variable preprocessing removed is pivoted first of all and its
    // row dropped too: the rows left are over the solver's variables, and
    // still sums of input XORs. A helper is judged on the solver's
    // clauses as they are now, not the input's: a candidate (pure in the
    // input, in at most two XORs) stays one only if every clause of the
    // solver that holds it lies within one of its XORs' variable sets; a
    // variable that stands in for a substituted one carries that one's
    // clauses (an error bit's at-most-one clauses, say) and is no helper
    let candidate: Vec<bool> = (0..cols).map(|c| !gone[c] && !g.col_mixed[c] && occ[c] <= 2).collect();
    let mut own: Vec<Vec<u32>> = vec![Vec::new(); cols];
    for (i, (vars, _)) in xors.iter().enumerate() {
        for &ev in vars {
            let c = col_index[&ev] as usize;
            if candidate[c] {
                own[c].push(i as u32);
            }
        }
    }
    let xor_cols: Vec<Vec<u32>> = xors.iter().map(|(vars, _)| vars.iter().map(|&ev| col_index[&ev]).collect()).collect();
    let (outside, scanned) = clauses_outside(solver, &g.col_of, &candidate, &own, &xor_cols);
    g.work += scanned;
    {
        // the scan of the formula is charged like the reduction's work,
        // and the limits are polled before the reduction starts
        let work = std::mem::take(&mut g.work);
        g.build_ticks += work;
        solver.statistics.gauss_ticks += work;
        solver.statistics.ticks += work;
        let over = solver.limited.ticks && solver.statistics.work_clock() >= solver.limits.ticks;
        if over || solver.termination.flagged.load(std::sync::atomic::Ordering::Relaxed) {
            crate::print::message(solver, "parity engine off: the work limit or a termination request ended the build".to_string());
            g.rows = Vec::new();
            g.bits = Vec::new();
            solver.gauss = g;
            return true;
        }
    }
    let helper: Vec<bool> = (0..cols).map(|c| gone[c] || (candidate[c] && !outside[c])).collect();
    let mut order: Vec<u32> = (0..cols as u32).collect();
    order.sort_by_key(|&c| (!gone[c as usize], !helper[c as usize], occ[c as usize], c));
    // a column whose turn finds every row holding it a pivot row already
    // is skipped; a later addition can carry it into a row that has no
    // pivot yet, so the pass repeats until no such row is left, and then
    // every row without a pivot is a zero row
    let mut pivot_of_row: Vec<bool> = vec![false; n];
    g.work += (n as u64) * (words as u64) + cols as u64;
    loop {
        let mut progress = false;
        for &col in &order {
            if occ[col as usize] == 0 {
                continue;
            }
            // the two scans of the rows for this column, charged whether
            // or not a row is added
            g.work += (n as u64) / 8 + 1;
            let mut src = INVALID;
            for r in 0..n {
                if !pivot_of_row[r] && g.has(r, col) {
                    src = r as u32;
                    break;
                }
            }
            if src != INVALID {
                let src = src as usize;
                pivot_of_row[src] = true;
                g.rows[src].basic = col;
                g.work += (n as u64) / 8 + 1;
                for r in 0..n {
                    if r != src && g.has(r, col) {
                        g.add_row(r, src);
                        if solver.gaussproof.active() {
                            solver.gaussproof.added(r, src);
                        }
                    }
                }
                g.pivots += 1;
                progress = true;
            }
            // the reduction charges its work as it goes and stops on the
            // work limit or a termination request (a signal, --time); the
            // engine is then off and the search sees the limit itself
            if g.work >= 1 << 20 {
                let work = std::mem::take(&mut g.work);
                g.build_ticks += work;
                solver.statistics.gauss_ticks += work;
                solver.statistics.ticks += work;
                let over = solver.limited.ticks && solver.statistics.work_clock() >= solver.limits.ticks;
                if over || solver.termination.flagged.load(std::sync::atomic::Ordering::Relaxed) {
                    crate::print::message(solver, "parity engine off: the work limit or a termination request ended the reduction".to_string());
                    g.on = false;
                    g.frozen = Vec::new();
                    g.rows = Vec::new();
                    g.bits = Vec::new();
                    solver.gauss = g;
                    return true;
                }
            }
        }
        if !progress {
            break;
        }
    }
    // rows without a pivot are zero rows: a parity of one refutes; a row
    // whose pivot is a helper or a gone variable is dropped (see above)
    let mut refuted = false;
    let mut dropped = 0usize;
    for r in 0..n {
        if !pivot_of_row[r] {
            debug_assert!(g.row_bits(r).iter().all(|&w| w == 0));
            if g.rows[r].parity == 1 {
                if !refuted {
                    // the proof of the empty clause, from this row alone
                    solver.gaussproof.clause(r, &[]);
                }
                refuted = true;
            }
            g.rows[r].live = false;
        } else if helper[g.rows[r].basic as usize] {
            g.rows[r].live = false;
            dropped += 1;
        }
    }
    debug_assert!((0..n).all(|r| !g.rows[r].live || (0..cols as u32).all(|c| !gone[c as usize] || !g.has(r, c))));
    // the live rows' variables stay the solver's from here on: no
    // elimination, no substitution (src/eliminate.rs, fastel.rs,
    // substitute.rs), no compaction (src/collect.rs)
    let mut frozen = vec![false; nvars];
    for col in 0..cols {
        if !gone[col] {
            frozen[idx(g.lit_of[col]) as usize] = true;
        }
    }
    g.frozen = frozen;
    g.on = true;
    g.work += (n as u64) * (words as u64);
    let work = std::mem::take(&mut g.work);
    g.build_ticks += work;
    g.next_yield_check = YIELD_WINDOW;
    solver.statistics.gauss_ticks += work;
    solver.statistics.ticks += work;
    solver.gauss = g;
    if refuted {
        crate::print::message(solver, "parity engine: the XOR system is inconsistent".to_string());
        if !solver.inconsistent {
            solver.inconsistent = true;
            if solver.proof.is_some() {
                crate::proof::add_empty_to_proof(solver);
            }
        }
        return false;
    }
    solver.gauss.built = true;
    // watches; a row whose basic variable is fixed already needs the
    // Jordan step, a row with one unassigned variable (or none) settles
    let mut pending: Vec<usize> = Vec::new();
    for r in 0..n {
        if !solver.gauss.rows[r].live {
            continue;
        }
        let basic = solver.gauss.rows[r].basic;
        let mut other = solver.gauss.unassigned_col(r, basic);
        let open = other != INVALID;
        if !open {
            other = solver.gauss.any_col(r, basic);
        }
        solver.gauss.rows[r].other = other;
        solver.gauss.watch[basic as usize].push(r as u32);
        if other != INVALID {
            solver.gauss.watch[other as usize].push(r as u32);
        }
        if State::bit(&solver.gauss.assigned, basic) {
            solver.gauss.queue.push(r as u32);
        } else if !open {
            pending.push(r);
        }
    }
    solver.gauss.cursor = solver.trail.len();
    let mut conflict: Option<Conflict> = None;
    for r in pending {
        let status = solver.gauss.status(r);
        conflict = settle(solver, r, status);
        if conflict.is_some() {
            break;
        }
    }
    if conflict.is_none() {
        conflict = propagate(solver);
    }
    if conflict.is_some() {
        if !solver.inconsistent {
            solver.inconsistent = true;
            if solver.proof.is_some() {
                crate::proof::add_empty_to_proof(solver);
            }
        }
        crate::print::message(solver, "parity engine: the XOR system refutes the formula at level zero".to_string());
        return false;
    }
    let live = solver.gauss.rows.iter().filter(|r| r.live).count();
    crate::print::message(
        solver,
        format_args!(
            "parity engine on: {} rows over {} variables ({} fixed, {} gone, {} words a row), {} row additions to reduce, {} rows of helper and gone variables dropped",
            live,
            cols,
            fixed,
            gone.iter().filter(|&&x| x).count(),
            words,
            solver.gauss.additions,
            dropped
        ),
    );
    true
}

/// Of the assigned columns of row `r` other than `skip`, the one assigned
/// at the highest level (the latest on the trail among those): a backtrack
/// that unassigns any column of the row unassigns this one, so a row with
/// no unassigned column to watch still sees the next assignment. INVALID
/// for a row of one variable.
fn latest_col(solver: &mut Solver, r: usize, skip: u32) -> u32 {
    let words = solver.gauss.words;
    solver.gauss.work += words as u64;
    let mut best = INVALID;
    let mut best_key = (0u32, 0u32);
    for i in 0..words {
        let mut w = solver.gauss.bits[r * words + i];
        while w != 0 {
            let col = (i as u32) * 64 + w.trailing_zeros();
            w &= w - 1;
            if col == skip {
                continue;
            }
            let l = solver.gauss.lit_of[col as usize];
            let a = solver.assigned[idx(l) as usize];
            let key = (a.level, a.trail);
            if best == INVALID || key > best_key {
                best = col;
                best_key = key;
            }
        }
    }
    best
}

/// Per candidate helper column, whether some irredundant clause of the
/// solver holds its variable and does not lie within one of its XORs'
/// variable sets (`own` names the XORs, `xor_cols` their columns);
/// binary clauses through the watches, large ones through the arena,
/// garbage and redundant clauses left out. Also the work of the scan (one
/// per watch and per literal visited).
fn clauses_outside(solver: &Solver, col_of: &[u32], candidate: &[bool], own: &[Vec<u32>], xor_cols: &[Vec<u32>]) -> (Vec<bool>, u64) {
    let cols = candidate.len();
    let mut outside = vec![false; cols];
    let mut work: u64 = 0;
    let mut mark: Vec<u32> = vec![u32::MAX; cols];
    // the clause's columns are in `buf` (INVALID for a variable that is
    // no column); each candidate among them is checked
    let check = |buf: &[u32], outside: &mut Vec<bool>, mark: &mut Vec<u32>| {
        for &c in buf {
            if c == INVALID || !candidate[c as usize] || outside[c as usize] {
                continue;
            }
            let mut within = false;
            for &x in &own[c as usize] {
                let set = &xor_cols[x as usize];
                for &m in set {
                    mark[m as usize] = x;
                }
                if buf.iter().all(|&b| b != INVALID && mark[b as usize] == x) {
                    within = true;
                    break;
                }
            }
            if !within {
                outside[c as usize] = true;
            }
        }
    };
    let vars = solver.vars() as usize;
    let mut buf: Vec<u32> = Vec::new();
    for i in 0..vars {
        let col = col_of[i];
        if col == INVALID || !candidate[col as usize] {
            continue;
        }
        for lit in [2 * i as u32, 2 * i as u32 + 1] {
            let w = solver.watches[lit as usize];
            let mut p = w.begin;
            while p != w.end {
                let watch = solver.vectors.stack[p];
                let binary = crate::watch::watch_is_binary(watch);
                p += if binary { 1 } else { 2 };
                work += 1;
                if binary {
                    let other = crate::watch::watch_lit(watch);
                    buf.clear();
                    buf.push(col);
                    buf.push(col_of[idx(other) as usize]);
                    check(&buf, &mut outside, &mut mark);
                }
            }
        }
    }
    let last_irredundant = solver.last_irredundant;
    let mut ref_: crate::reference::Reference = 0;
    while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        if last_irredundant != crate::reference::INVALID_REF && ref_ > last_irredundant {
            break;
        }
        let c = solver.arena.clause(ref_);
        work += 1;
        if !c.redundant() && !c.garbage() {
            work += c.size() as u64;
            buf.clear();
            let mut any = false;
            for &lit in c.lits() {
                let col = col_of[idx(lit) as usize];
                any |= col != INVALID && candidate[col as usize];
                buf.push(col);
            }
            if any {
                check(&buf, &mut outside, &mut mark);
            }
        }
        ref_ = next;
    }
    (outside, work)
}

/// The internal literal of column `col` with value `value`.
#[inline]
fn literal(g: &State, col: u32, value: bool) -> u32 {
    let l = g.lit_of[col as usize];
    if value {
        l
    } else {
        not(l)
    }
}

/// The row's clause, false under the assignment except `propagated`
/// (INVALID for a conflict), which takes `value`: every variable of the
/// row with its value negated, the propagated literal first. The proof's
/// copy (external literals) holds every variable of the row; the solver's
/// leaves out those fixed at level zero: kissat drops root-false literals
/// itself, and after a compaction every fixed variable shares one
/// internal literal, which a clause may hold only once (the DRAT addition
/// of the shorter clause is implied by the proved one and the units).
fn clause_of(solver: &mut Solver, r: usize, propagated: u32, value: bool) {
    solver.clause.clear();
    solver.gauss.proof_clause.clear();
    let words = solver.gauss.words;
    if propagated != INVALID {
        let lit = literal(&solver.gauss, propagated, value);
        solver.clause.push(lit);
        let ev = solver.gauss.ext_of[propagated as usize];
        solver.gauss.proof_clause.push(if value { ev } else { -ev });
    }
    for i in 0..words {
        let mut w = solver.gauss.bits[r * words + i];
        while w != 0 {
            let col = (i as u32) * 64 + w.trailing_zeros();
            w &= w - 1;
            if col == propagated {
                continue;
            }
            let l = solver.gauss.lit_of[col as usize];
            let val = solver.values[l as usize];
            debug_assert!(val != 0);
            let ev = solver.gauss.ext_of[col as usize];
            solver.gauss.proof_clause.push(if val > 0 { -ev } else { ev });
            if solver.assigned[idx(l) as usize].level == 0 {
                continue;
            }
            // the literal that is false now
            let lit = if val > 0 { not(l) } else { l };
            solver.clause.push(lit);
        }
    }
}

/// The proof of the clause on `solver.clause` (a clause of row `r`), as
/// external literals, and its place in the extended formula.
fn prove_clause(solver: &mut Solver, r: usize) {
    if !solver.gaussproof.active() {
        return;
    }
    let ext = std::mem::take(&mut solver.gauss.proof_clause);
    solver.gaussproof.clause(r, &ext);
    solver.gauss.proof_clause = ext;
}

/// Put the clause on `solver.clause` into the arena as a redundant clause
/// with the propagated literal first and the highest-level false literal
/// second (the watches, as for a learned clause), or as a binary clause.
/// Returns the reference (INVALID_REF for a binary clause) and the
/// clause's first two literals.
fn materialize(solver: &mut Solver, propagated: bool) -> (crate::reference::Reference, u32, u32) {
    let size = solver.clause.len();
    debug_assert!(size >= 2);
    let start = if propagated { 1 } else { 0 };
    // the false literal with the highest level goes to the first watch
    // slot after the propagated literal, the next highest after it, so the
    // two watches are sound when the solver backtracks
    let end = (start + 2).min(size);
    for slot in start..end {
        let mut best = slot;
        let mut best_level = solver.assigned[idx(solver.clause[slot]) as usize].level;
        for p in slot + 1..size {
            let level = solver.assigned[idx(solver.clause[p]) as usize].level;
            if level > best_level {
                best_level = level;
                best = p;
            }
        }
        solver.clause.swap(slot, best);
    }
    let a = solver.clause[0];
    let b = solver.clause[1];
    if size == 2 {
        let _ = crate::clause::new_redundant_clause(solver, 1);
        return (crate::reference::INVALID_REF, a, b);
    }
    // the glue is the number of distinct levels
    let mut levels: Vec<u32> = solver.clause.iter().map(|&l| solver.assigned[idx(l) as usize].level).collect();
    levels.sort_unstable();
    levels.dedup();
    let glue = levels.len() as u32;
    let reference = crate::clause::new_redundant_clause(solver, glue);
    (reference, a, b)
}

/// Act on a row's status: propagate its unit or return its conflict;
/// `None` when nothing happened or the unit was assigned.
fn settle(solver: &mut Solver, r: usize, status: Status) -> Option<Conflict> {
    match status {
        Status::Open | Status::Satisfied => None,
        Status::Unit(col, value) => {
            let lit = literal(&solver.gauss, col, value);
            let val = solver.values[lit as usize];
            if val != 0 {
                // assigned by the solver before the engine read it
                State::set(&mut solver.gauss.assigned, col, true);
                State::set(&mut solver.gauss.truth, col, (val > 0) == value);
                if val > 0 {
                    return None;
                }
                return settle(solver, r, Status::Falsified);
            }
            clause_of(solver, r, col, value);
            debug_assert!(solver.clause[0] == lit);
            prove_clause(solver, r);
            let size = solver.clause.len();
            solver.gauss.propagations += 1;
            if size == 1 {
                // the row's other variables are all fixed: a unit, assigned
                // at level zero
                solver.gauss.units += 1;
                solver.gauss.rows[r].live = false;
                solver.clause.clear();
                crate::assign::learned_unit(solver, lit);
            } else {
                let (reference, _, other) = materialize(solver, true);
                solver.clause.clear();
                if reference == crate::reference::INVALID_REF {
                    crate::assign::assign_binary(solver, lit, other);
                } else {
                    crate::assign::assign_reference(solver, lit, reference);
                }
            }
            State::set(&mut solver.gauss.assigned, col, true);
            State::set(&mut solver.gauss.truth, col, value);
            None
        }
        Status::Falsified => {
            clause_of(solver, r, INVALID, false);
            prove_clause(solver, r);
            solver.gauss.conflicts += 1;
            let size = solver.clause.len();
            if size == 0 {
                // an empty row of parity one, or a row false under the
                // level-zero assignment alone: the formula is refuted
                solver.gauss.rows[r].live = false;
                if solver.level > 0 && !solver.inconsistent {
                    // at level zero the caller's conflict handling does this
                    solver.inconsistent = true;
                    if solver.proof.is_some() {
                        crate::proof::add_empty_to_proof(solver);
                    }
                }
                let _ = crate::propsearch::binary_conflict(solver, 0, 0);
                return Some(Conflict::Binary);
            }
            if size == 1 {
                // the row is a unit the assignment contradicts: at level
                // zero the formula is refuted; above it the unit holds, so
                // the solver backtracks to level zero and assigns it
                let lit = solver.clause[0];
                solver.clause.clear();
                let level = solver.assigned[idx(lit) as usize].level;
                solver.gauss.units += 1;
                solver.gauss.rows[r].live = false;
                if level == 0 {
                    let _ = crate::propsearch::binary_conflict(solver, lit, lit);
                    return Some(Conflict::Binary);
                }
                crate::backtrack::backtrack_after_conflict(solver, 0);
                crate::assign::learned_unit(solver, lit);
                let col = solver.gauss.col_of[idx(lit) as usize];
                State::set(&mut solver.gauss.assigned, col, true);
                State::set(&mut solver.gauss.truth, col, crate::literal::negated(lit) == 0);
                return None;
            }
            let (reference, a, b) = materialize(solver, false);
            solver.clause.clear();
            if reference == crate::reference::INVALID_REF {
                Some(crate::propsearch::binary_conflict(solver, a, b))
            } else {
                Some(Conflict::Clause(reference))
            }
        }
    }
}

/// A row's basic variable was assigned: give it a new one among its
/// unassigned variables and eliminate that one from every other row
/// (the Jordan step), or settle the row when one or none is left.
fn pivot(solver: &mut Solver, r: usize) -> Option<Conflict> {
    if !solver.gauss.rows[r].live {
        return None;
    }
    let basic = solver.gauss.rows[r].basic;
    if !State::bit(&solver.gauss.assigned, basic) {
        return None; // unassigned again (a backtrack in between)
    }
    // the new basic variable: the other watch when it is still good, else
    // any unassigned variable of the row
    let old_other = solver.gauss.rows[r].other;
    let u = if old_other != INVALID && solver.gauss.has(r, old_other) && !State::bit(&solver.gauss.assigned, old_other) {
        old_other
    } else {
        solver.gauss.unassigned_col(r, INVALID)
    };
    if u == INVALID {
        let status = solver.gauss.status(r);
        return settle(solver, r, status);
    }
    let second = solver.gauss.unassigned_col(r, u);
    if second == INVALID {
        // one unassigned variable: the row propagates it, no pivot needed
        let status = solver.gauss.status(r);
        return settle(solver, r, status);
    }
    // u becomes the basic variable and `second` the other watch; stale
    // entries in the watch lists are dropped when they fire
    solver.gauss.pivots += 1;
    solver.gauss.rows[r].basic = u;
    solver.gauss.rows[r].other = second;
    if u != old_other {
        solver.gauss.watch[u as usize].push(r as u32);
    }
    solver.gauss.watch[second as usize].push(r as u32);
    let n = solver.gauss.rows.len();
    // the scan of every row for the new basic variable, charged whether
    // or not a row holds it
    solver.gauss.work += (n as u64) / 8 + 1;
    for s in 0..n {
        if s == r || !solver.gauss.rows[s].live || !solver.gauss.has(s, u) {
            continue;
        }
        solver.gauss.add_row(s, r);
        crate::gaussproof::added(solver, s, r);
        // the row's watches: its basic variable is untouched (not in row
        // r); its other watch may be gone or assigned
        let other = solver.gauss.rows[s].other;
        let sb = solver.gauss.rows[s].basic;
        let mut open = other != INVALID && solver.gauss.has(s, other) && !State::bit(&solver.gauss.assigned, other);
        if !open {
            // a new second watch: an unassigned column, else any column
            // (a watch on an assigned column fires again after a
            // backtrack; a row with no second watch would miss it)
            let mut o = solver.gauss.unassigned_col(s, sb);
            open = o != INVALID;
            if !open {
                o = latest_col(solver, s, sb);
            }
            if o != other {
                solver.gauss.rows[s].other = o;
                if o != INVALID {
                    solver.gauss.watch[o as usize].push(s as u32);
                }
            }
        }
        if State::bit(&solver.gauss.assigned, sb) {
            // its basic variable is assigned: it needs the Jordan step too
            // (its unassigned variables came from row r)
            solver.gauss.queue.push(s as u32);
        } else if !open {
            // the basic variable is the only unassigned one
            let status = solver.gauss.status(s);
            if let Some(c) = settle(solver, s, status) {
                return Some(c);
            }
        }
    }
    None
}

/// Inspect the rows watching column `col`, which was just assigned.
fn notify(solver: &mut Solver, col: u32) -> Option<Conflict> {
    let list = std::mem::take(&mut solver.gauss.watch[col as usize]);
    let mut keep: Vec<u32> = Vec::with_capacity(list.len());
    let mut result: Option<Conflict> = None;
    let mut i = 0;
    while i < list.len() {
        let r = list[i] as usize;
        i += 1;
        let row = &solver.gauss.rows[r];
        if !row.live || (row.basic != col && row.other != col) {
            continue; // a stale watch
        }
        if result.is_some() {
            keep.push(r as u32);
            continue;
        }
        if row.basic == col {
            keep.push(r as u32);
            solver.gauss.queue.push(r as u32);
            continue;
        }
        // the other watch: move it when another unassigned variable
        // exists; else to the column assigned at the highest level (with
        // chronological backtracking that need not be this one), so a
        // backtrack that frees any column of the row frees the watch
        let basic = row.basic;
        let mut o = solver.gauss.unassigned_col(r, basic);
        if o == INVALID {
            o = latest_col(solver, r, basic);
        }
        if o != col && o != INVALID {
            solver.gauss.rows[r].other = o;
            solver.gauss.watch[o as usize].push(r as u32);
            if !State::bit(&solver.gauss.assigned, o) {
                continue;
            }
        } else {
            keep.push(r as u32);
        }
        let status = solver.gauss.status(r);
        result = settle(solver, r, status);
    }
    // rows added to this column's list meanwhile (pivots) stay
    let mut now = std::mem::take(&mut solver.gauss.watch[col as usize]);
    keep.append(&mut now);
    solver.gauss.watch[col as usize] = keep;
    result
}

/// Publish the engine's work to the work clock; true when the work limit
/// is reached or a termination was requested, with the engine then off
/// for the run (the search sees the limit itself; the clauses the engine
/// added stay valid).
fn over_limit(solver: &mut Solver) -> bool {
    let work = std::mem::take(&mut solver.gauss.work);
    solver.statistics.gauss_ticks += work;
    solver.statistics.ticks += work;
    let over = solver.limited.ticks && solver.statistics.work_clock() >= solver.limits.ticks;
    if over || solver.termination.flagged.load(std::sync::atomic::Ordering::Relaxed) {
        solver.gauss.on = false;
        solver.gauss.frozen = Vec::new();
        crate::print::message(solver, "parity engine off: the work limit or a termination request".to_string());
        return true;
    }
    false
}

/// Read the trail from the engine's cursor, inspect the rows watching the
/// assigned variables, run the pending Jordan steps; propagates through
/// the solver's assign and returns the first conflict. Called after clause
/// propagation reached a fixpoint; the caller propagates again when the
/// engine assigned anything.
pub fn propagate(solver: &mut Solver) -> Option<Conflict> {
    if !solver.gauss.on || !solver.gauss.built || solver.gauss.suspended {
        return None;
    }
    let mut result: Option<Conflict> = None;
    'outer: loop {
        // first every new assignment goes into the engine's bits, then the
        // rows are inspected: a row is judged on the whole trail, never on
        // a part of it (the clauses are at a fixpoint, so what the trail
        // holds is all there is until the engine assigns)
        let start = solver.gauss.cursor;
        let end = solver.trail.len();
        for i in start..end {
            let lit = solver.trail[i];
            let v = idx(lit) as usize;
            if let Some(&col) = solver.gauss.col_of.get(v) {
                if col != INVALID {
                    State::set(&mut solver.gauss.assigned, col, true);
                    State::set(&mut solver.gauss.truth, col, crate::literal::negated(lit) == 0);
                }
            }
        }
        solver.gauss.cursor = end;
        solver.gauss.work += (end - start) as u64;
        for i in start..end {
            let lit = solver.trail[i];
            let v = idx(lit) as usize;
            let col = match solver.gauss.col_of.get(v) {
                Some(&c) if c != INVALID => c,
                _ => continue,
            };
            if State::bit(&solver.gauss.scanned, col) {
                continue;
            }
            State::set(&mut solver.gauss.scanned, col, true);
            result = notify(solver, col);
            if result.is_some() {
                // the rows after the conflicting one were not inspected:
                // this literal is read again after the backtrack
                State::set(&mut solver.gauss.scanned, col, false);
                if solver.gauss.cursor > i {
                    solver.gauss.cursor = i;
                }
                break 'outer;
            }
        }
        // the pending pivots: they may propagate or conflict
        while let Some(r) = solver.gauss.queue.pop() {
            result = pivot(solver, r as usize);
            if result.is_some() {
                break 'outer;
            }
            if solver.gauss.work >= 1 << 20 && over_limit(solver) {
                return result;
            }
        }
        if solver.gauss.work >= 1 << 20 && over_limit(solver) {
            return result;
        }
        if solver.gauss.cursor >= solver.trail.len() {
            break;
        }
    }
    // the pending pivots stay queued across a conflict: a pivot re-checks
    // that its row's basic variable is still assigned
    let work = std::mem::take(&mut solver.gauss.work);
    solver.statistics.gauss_ticks += work;
    solver.statistics.ticks += work;
    // the yield guard: at every window of the engine's work, its
    // propagations and conflicts per million ticks since the build must
    // reach gaussyield, else it stops for the run (the clauses it added
    // stay valid); on a cell it helps the yield is in the hundreds, on
    // one where it only burns work below one
    let min_yield = solver.options.gaussyield as u64;
    if min_yield != 0 && solver.gauss.on {
        let spent = solver.statistics.gauss_ticks.saturating_sub(solver.gauss.build_ticks);
        if spent >= solver.gauss.next_yield_check {
            solver.gauss.next_yield_check = spent.saturating_add(YIELD_WINDOW);
            let events = solver.gauss.propagations + solver.gauss.conflicts;
            let per_million = events.saturating_mul(1_000_000) / spent.max(1);
            if per_million < min_yield {
                solver.gauss.on = false;
                solver.gauss.frozen = Vec::new();
                crate::print::message(
                    solver,
                    format_args!(
                        "parity engine off: {} propagations and conflicts in {} ticks, {} per million, under gaussyield={}",
                        events, spent, per_million, min_yield
                    ),
                );
            }
        }
    }
    // the effort cap: the engine may use gausseffort per mille of the
    // search ticks (plus a start allowance); past that it stops for the
    // run, and the clauses it added stay valid
    let effort = solver.options.gausseffort as u64;
    if effort != 0 {
        let allowed = (solver.statistics.search_ticks / 1000).saturating_mul(effort).saturating_add(10_000_000);
        let spent = solver.statistics.gauss_ticks.saturating_sub(solver.gauss.build_ticks);
        if spent > allowed {
            solver.gauss.on = false;
            solver.gauss.frozen = Vec::new();
            crate::print::message(
                solver,
                format_args!(
                    "parity engine off: {} ticks exceed gausseffort={} per mille of {} search ticks",
                    solver.statistics.gauss_ticks,
                    effort,
                    solver.statistics.search_ticks
                ),
            );
        }
    }
    result
}

/// The solver unassigned this literal (backtracking).
#[inline]
pub fn unassigned(solver: &mut Solver, lit: u32) {
    let v = idx(lit) as usize;
    if let Some(&col) = solver.gauss.col_of.get(v) {
        if col != INVALID {
            State::set(&mut solver.gauss.assigned, col, false);
            State::set(&mut solver.gauss.truth, col, false);
            State::set(&mut solver.gauss.scanned, col, false);
        }
    }
}

/// The trail was cut back: positions from `new_end` on now hold the
/// assignments the backtrack kept (those of a lower level, with
/// chronological backtracking). They are read again, since a row that
/// lost a variable to the backtrack may now be a unit on them.
pub fn backtracked(solver: &mut Solver, new_end: usize) {
    if !solver.gauss.built {
        return;
    }
    for i in new_end..solver.trail.len() {
        let v = idx(solver.trail[i]) as usize;
        if let Some(&col) = solver.gauss.col_of.get(v) {
            if col != INVALID {
                State::set(&mut solver.gauss.scanned, col, false);
            }
        }
    }
    if solver.gauss.cursor > new_end {
        solver.gauss.cursor = new_end;
    }
}

/// After inprocessing: the assignment may have changed at level zero
/// without the engine seeing it. Re-read every column from the solver's
/// values and re-inspect every row; returns a conflict at level zero as
/// the solver's inconsistency.
pub fn resync(solver: &mut Solver) {
    if !solver.gauss.on || !solver.gauss.built || solver.inconsistent {
        return;
    }
    debug_assert!(solver.level == 0);
    let cols = solver.gauss.ext_of.len();
    for col in 0..cols as u32 {
        let l = solver.gauss.lit_of[col as usize];
        if l == crate::literal::INVALID_LIT {
            continue; // a gone column: in no live row
        }
        let val = solver.values[l as usize];
        State::set(&mut solver.gauss.assigned, col, val != 0);
        State::set(&mut solver.gauss.truth, col, val > 0);
        State::set(&mut solver.gauss.scanned, col, val != 0);
    }
    solver.gauss.cursor = solver.trail.len();
    solver.gauss.queue.clear();
    let n = solver.gauss.rows.len();
    let mut conflict: Option<Conflict> = None;
    for r in 0..n {
        if !solver.gauss.rows[r].live {
            continue;
        }
        let basic = solver.gauss.rows[r].basic;
        if State::bit(&solver.gauss.assigned, basic) {
            conflict = pivot(solver, r);
        } else {
            let other = solver.gauss.rows[r].other;
            if other == INVALID || !solver.gauss.has(r, other) || State::bit(&solver.gauss.assigned, other) {
                let mut o = solver.gauss.unassigned_col(r, basic);
                let open = o != INVALID;
                if !open {
                    o = solver.gauss.any_col(r, basic);
                }
                if o != other {
                    solver.gauss.rows[r].other = o;
                    if o != INVALID {
                        solver.gauss.watch[o as usize].push(r as u32);
                    }
                }
                if !open {
                    let status = solver.gauss.status(r);
                    conflict = settle(solver, r, status);
                }
            }
        }
        if conflict.is_some() {
            break;
        }
    }
    let work = std::mem::take(&mut solver.gauss.work);
    solver.statistics.gauss_ticks += work;
    solver.statistics.ticks += work;
    if conflict.is_some() {
        if !solver.inconsistent {
            solver.inconsistent = true;
            if solver.proof.is_some() {
                crate::proof::add_empty_to_proof(solver);
            }
        }
    }
}

/// The exit report line.
pub fn report(solver: &Solver) {
    let g = &solver.gauss;
    if g.rows.is_empty() {
        return;
    }
    println!(
        "c gauss rows={} columns={} propagations={} conflicts={} units={} pivots={} additions={} ticks={}",
        g.rows.iter().filter(|r| r.live).count(),
        g.ext_of.len(),
        g.propagations,
        g.conflicts,
        g.units,
        g.pivots,
        g.additions,
        solver.statistics.gauss_ticks
    );
}
