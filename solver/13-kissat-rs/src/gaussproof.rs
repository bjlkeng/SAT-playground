//! Not in kissat. The VeriPB proof of the parity engine (bead
//! SAT-playground-1v2.7.2): what the engine derives, as cutting-planes
//! steps in the structure pass's proof stream, so the gate's first stage
//! proves every clause the engine added from the input alone and its
//! second stage (drat-trim) sees those clauses as part of the extended
//! formula.
//!
//! The shape (checked by hand on a toy with VeriPB 3.0.2 before the code
//! was written; scratchpad xor/pb/toy.py of 2026-10-15):
//!
//! - An input XOR x1 + ... + xk = c (mod 2) becomes two linear
//!   inequalities through a chain of fresh variables: for each link,
//!   p = a xor b and z = a and b (defined by `red`, seven clauses), from
//!   which a + b = p + 2z follows in both directions (a `rup` of two
//!   conditional constraints and one `pol` each way). Summed along the
//!   chain, x1 + ... + xk = p_last + 2 (z1 + ... ); and p_last = c follows
//!   from the XOR's clauses level by level (a `rup` per clause over the
//!   chain's prefix with a split on the next variable). So the XOR is
//!   GE: x1 + ... + xk - 2 (z1 + ...) >= c and LE: the reverse.
//! - A row of the matrix is a sum of input XORs; its GE is the `pol` sum
//!   of theirs and its LE likewise. The row keeps the set of input XORs it
//!   is the sum of (a bit set) and derives its two constraints only when a
//!   clause is needed from it: an addition in the matrix costs nothing in
//!   the proof.
//! - A clause of a row (every variable of the row assigned, parity wrong)
//!   is one `pol` line: the row's LE plus the literal axioms of the
//!   clause's literals, divided by two and doubled, plus the row's GE. The
//!   even part cancels and the odd right-hand side is what the division
//!   cuts: what is left is exactly the clause.
//!
//! Every derived clause is kept in the core and written to the derived
//! file; the stream is finished at exit, whatever the answer.

use crate::internal::Solver;
use crate::structproof::Writer;
use std::io::Write as _;

#[derive(Default)]
pub struct State {
    /// The proof stream, taken over from the structure pass.
    pub writer: Option<Writer>,
    pbp_path: String,
    derived_path: String,
    /// The derived file, header patched at the end.
    derived: Option<std::io::BufWriter<std::fs::File>>,
    derived_count: u64,
    max_var: i32,
    /// The next fresh variable of the proof (never in a derived clause).
    fresh: i32,
    /// Per input XOR: (GE, LE) constraint ids, 0 before they are derived.
    base: Vec<(u64, u64)>,
    /// The input XORs: variables and parity.
    xors: Vec<(Vec<i32>, u32)>,
    /// Per matrix row: the set of input XORs it is the sum of, and its
    /// (GE, LE) ids with whether they match the set.
    words: usize,
    sets: Vec<u64>,
    ids: Vec<(u64, u64)>,
    current: Vec<bool>,
    /// The row's ids are an input XOR's own (shared, never deleted).
    ids_base: Vec<bool>,
    /// A write failed: the run ends with an error at the next chance.
    pub failed: bool,
    pub clauses: u64,
    pub lines: u64,
}

/// The sum of these constraint ids as a balanced tree in reverse Polish
/// notation: VeriPB adds two constraints in time linear in their size,
/// so a chain of n additions is quadratic in n and a tree n log n.
fn sum_rpn(ids: &[u64]) -> String {
    fn rec(ids: &[u64], out: &mut String) {
        if ids.len() == 1 {
            out.push_str(&ids[0].to_string());
            return;
        }
        let mid = ids.len() / 2;
        rec(&ids[..mid], out);
        out.push(' ');
        rec(&ids[mid..], out);
        out.push_str(" +");
    }
    let mut out = String::new();
    rec(ids, &mut out);
    out
}

fn lit_str(l: i32) -> String {
    if l > 0 {
        format!("x{}", l)
    } else {
        format!("~x{}", -l)
    }
}

impl State {
    /// Take the proof stream over from the structure pass, with its
    /// derived clauses (written to the derived file first) and the input
    /// XORs the engine's rows are sums of.
    pub fn take_over(
        &mut self,
        writer: Writer,
        pbp_path: &str,
        derived_path: &str,
        pass_clauses: &[Vec<i32>],
        max_var: i32,
        xors: &[(Vec<i32>, u32)],
    ) -> std::io::Result<()> {
        let file = std::fs::File::create(derived_path)?;
        let mut out = std::io::BufWriter::new(file);
        // the header with room for the final count
        writeln!(out, "p cnf {} {:>12}", max_var, 0)?;
        for c in pass_clauses {
            for &l in c {
                write!(out, "{} ", l)?;
            }
            writeln!(out, "0")?;
        }
        self.derived = Some(out);
        self.derived_count = pass_clauses.len() as u64;
        self.writer = Some(writer);
        self.pbp_path = pbp_path.to_string();
        self.derived_path = derived_path.to_string();
        self.max_var = max_var;
        self.fresh = max_var;
        self.xors = xors.to_vec();
        self.base = vec![(0, 0); xors.len()];
        Ok(())
    }

    pub fn active(&self) -> bool {
        self.writer.is_some() && !self.failed
    }

    /// The bytes the table of row sets takes for `n` rows: n rows of n
    /// bits (the engine's size limit counts them with the matrix).
    pub fn table_bytes(n: usize) -> u64 {
        (n as u64) * (((n as u64) + 63) / 64) * 8
    }

    /// The rows: row i starts as input XOR i.
    pub fn rows(&mut self, n: usize) {
        self.words = (n + 63) / 64;
        self.sets = vec![0; n * self.words];
        for i in 0..n {
            self.sets[i * self.words + (i >> 6)] |= 1u64 << (i & 63);
        }
        self.ids = vec![(0, 0); n];
        self.current = vec![false; n];
        self.ids_base = vec![false; n];
    }

    /// Row `s` ^= row `r`: only the set changes; the row's two constraints
    /// are derived again from the set when a clause next needs them. (Kept
    /// incrementally, as the sum of the old form and row r's, they grow
    /// terms that never cancel and VeriPB slows thirtyfold; measured
    /// 2026-10-16 on par32-2: 72 MB of proof against 836 MB, but the
    /// check ran past eight minutes against 64 s.)
    pub fn added(&mut self, s: usize, r: usize) {
        let w = self.words;
        for i in 0..w {
            let x = self.sets[r * w + i];
            self.sets[s * w + i] ^= x;
        }
        self.current[s] = false;
    }

    fn fresh_var(&mut self) -> i32 {
        self.fresh += 1;
        self.fresh
    }

    /// A step that could not be written: as for the DRAT proof, the run
    /// ends here with an error (an answer without its proof is no answer).
    fn fatal(e: std::io::Error) -> ! {
        crate::error::fatal(format_args!("parity engine: could not write the proof: {}", e))
    }

    fn pol(&mut self, expr: &str) -> u64 {
        self.lines += 1;
        match self.writer.as_mut().unwrap().pol(expr) {
            Ok(id) => id,
            Err(e) => Self::fatal(e),
        }
    }

    fn rup(&mut self, terms: &[(i64, i32)], rhs: i64) -> u64 {
        self.lines += 1;
        match self.writer.as_mut().unwrap().rup_terms(terms, rhs) {
            Ok(id) => id,
            Err(e) => Self::fatal(e),
        }
    }

    fn red(&mut self, clause: &[i32], var: i32, value: bool) -> u64 {
        self.lines += 1;
        match self.writer.as_mut().unwrap().red_const(clause, var, value) {
            Ok(id) => id,
            Err(e) => Self::fatal(e),
        }
    }

    /// Fresh p = a xor b and z = a and b, and the two sides of
    /// a + b = p + 2z. Returns (p, GE, LE) with GE: a + b - p - 2z >= 0
    /// and LE: p + 2z - a - b >= 0.
    fn link(&mut self, a: i32, b: i32) -> (i32, u64, u64) {
        let p = self.fresh_var();
        let z = self.fresh_var();
        let d1 = self.red(&[-p, a, b], p, false);
        let _d2 = self.red(&[-p, -a, -b], p, false);
        let _d3 = self.red(&[p, -a, b], p, true);
        let _d4 = self.red(&[p, a, -b], p, true);
        let a1 = self.red(&[-z, a], z, false);
        let a2 = self.red(&[-z, b], z, false);
        let _a3 = self.red(&[z, -a, -b], z, true);
        // z implies not p; then GE = (A1 + A2 + Q) * 2 + D1, divided by 3
        let q = self.rup(&[(1, -z), (1, -p)], 1);
        let ge = self.pol(&format!("{} {} + {} + 2 * {} + 3 d", a1, a2, q, d1));
        // LE from two conditionals (a true: p + 2z + ~b >= 2; a false:
        // p + 2z + ~b >= 1), 2 * the first + the second, divided by 3
        let ca1 = self.rup(&[(1, p), (2, z), (1, -b), (2, -a)], 2);
        let ca0 = self.rup(&[(1, p), (2, z), (1, -b), (1, a)], 1);
        let le = self.pol(&format!("{} 2 * {} + 3 d", ca1, ca0));
        (p, ge, le)
    }

    /// The two sides of input XOR `i`, derived once.
    fn base_of(&mut self, i: usize) -> (u64, u64) {
        if self.base[i].0 != 0 {
            return self.base[i];
        }
        let (vars, parity) = self.xors[i].clone();
        let k = vars.len();
        debug_assert!(k >= 2);
        let (mut p, ge, le) = self.link(vars[0], vars[1]);
        let mut ges = vec![ge];
        let mut les = vec![le];
        let mut chain = vec![p];
        for &v in &vars[2..] {
            let (p2, ge, le) = self.link(p, v);
            p = p2;
            ges.push(ge);
            les.push(le);
            chain.push(p);
        }
        // the parity of the chain's end from the XOR's clauses, level by
        // level: level j holds the clauses over (chain[j-2], vars[j..]) of
        // the wrong parity, each from the level before by a split on
        // vars[j-1] (level 1 is the input clauses)
        let mut unit = 0u64;
        for j in 2..=k {
            let mut cur: Vec<i32> = vec![chain[j - 2]];
            cur.extend_from_slice(&vars[j..]);
            let split = vars[j - 1];
            let m = cur.len();
            for pattern in 0..(1u32 << m) {
                if pattern.count_ones() % 2 == parity {
                    continue; // an allowed pattern: no clause
                }
                // the clause forbidding this pattern of trues
                let terms: Vec<(i64, i32)> = (0..m).map(|t| (1, if pattern >> t & 1 == 1 { -cur[t] } else { cur[t] })).collect();
                let mut with = terms.clone();
                with.push((1, split));
                self.rup(&with, 1);
                let mut without = terms.clone();
                without.push((1, -split));
                self.rup(&without, 1);
                unit = self.rup(&terms, 1);
            }
        }
        let end = chain[k - 2];
        let sum_ge = sum_rpn(&ges);
        let sum_le = sum_rpn(&les);
        let (ge, le) = if parity == 1 {
            // p_last >= 1 is the unit; LE needs ~p_last >= 0, an axiom
            (self.pol(&format!("{} {} +", sum_ge, unit)), self.pol(&format!("{} ~x{} +", sum_le, end)))
        } else {
            (self.pol(&format!("{} x{} +", sum_ge, end)), self.pol(&format!("{} {} +", sum_le, unit)))
        };
        self.base[i] = (ge, le);
        (ge, le)
    }

    /// The two sides of row `r` for its current set.
    fn row_ids(&mut self, r: usize) -> (u64, u64) {
        if self.current[r] {
            return self.ids[r];
        }
        let w = self.words;
        let mut members: Vec<usize> = Vec::new();
        for i in 0..w {
            let mut x = self.sets[r * w + i];
            while x != 0 {
                members.push(i * 64 + x.trailing_zeros() as usize);
                x &= x - 1;
            }
        }
        debug_assert!(!members.is_empty());
        let old = self.ids[r];
        let mut ges: Vec<u64> = Vec::with_capacity(members.len());
        let mut les: Vec<u64> = Vec::with_capacity(members.len());
        for &i in &members {
            let (g, l) = self.base_of(i);
            ges.push(g);
            les.push(l);
        }
        let (ge, le) = if members.len() == 1 {
            (ges[0], les[0])
        } else {
            let sg = sum_rpn(&ges);
            let sl = sum_rpn(&les);
            (self.pol(&sg), self.pol(&sl))
        };
        if old.0 != 0 && !self.ids_base[r] && (old.0 != ge || old.1 != le) {
            // the row's earlier form is no longer needed (an input XOR's
            // own form is shared and stays)
            if let Some(w) = self.writer.as_mut() {
                if let Err(e) = w.delete(&[old.0, old.1]) {
                    Self::fatal(e);
                }
            }
        }
        self.ids[r] = (ge, le);
        self.ids_base[r] = members.len() == 1;
        self.current[r] = true;
        (ge, le)
    }

    /// Derive a clause of row `r` (external literals, every variable of
    /// the row) and add it to the extended formula.
    pub fn clause(&mut self, r: usize, clause: &[i32]) {
        if !self.active() {
            return;
        }
        let (ge, le) = self.row_ids(r);
        // LE + (the literal axioms, summed first among themselves, so
        // the one big addition is into LE once), divided by two and
        // doubled, plus GE
        let mut expr = String::new();
        expr.push_str(&le.to_string());
        if !clause.is_empty() {
            expr.push(' ');
            expr.push_str(&lit_str(clause[0]));
            for &l in &clause[1..] {
                expr.push(' ');
                expr.push_str(&lit_str(l));
                expr.push_str(" +");
            }
            expr.push_str(" +");
        }
        expr.push_str(&format!(" 2 d 2 * {} +", ge));
        let id = self.pol(&expr);
        if let Some(w) = self.writer.as_mut() {
            w.keep(id);
            if clause.is_empty() {
                w.refuted = true;
            }
        }
        if let Some(out) = self.derived.as_mut() {
            let mut line = String::new();
            for &l in clause {
                line.push_str(&l.to_string());
                line.push(' ');
            }
            line.push_str("0\n");
            if let Err(e) = out.write_all(line.as_bytes()) {
                Self::fatal(e);
            }
        }
        self.derived_count += 1;
        self.clauses += 1;
    }

    /// Finish the proof and the derived file; the solver's answer is
    /// checked against them. An error means the files are not a proof.
    pub fn finish(&mut self) -> std::io::Result<()> {
        let writer = match self.writer.take() {
            Some(w) => w,
            None => return Ok(()),
        };
        if self.failed || writer.failed {
            return Err(std::io::Error::other("a proof step could not be written"));
        }
        writer.finish()?;
        if let Some(mut out) = self.derived.take() {
            out.flush()?;
            let mut file = out.into_inner().map_err(|e| e.into_error())?;
            use std::io::Seek as _;
            file.seek(std::io::SeekFrom::Start(0))?;
            write!(file, "p cnf {} {:>12}", self.max_var, self.derived_count)?;
            file.flush()?;
        }
        Ok(())
    }

    /// Remove the sidecars (a failed proof): the answer stands without them
    /// only if it was never UNSAT by the engine, which the caller decides.
    pub fn discard(&mut self) {
        self.writer = None;
        self.derived = None;
        let _ = std::fs::remove_file(&self.pbp_path);
        let _ = std::fs::remove_file(&self.derived_path);
    }
}

/// Row `r` was added into row `s` (s ^= r): the engine's hook.
#[inline]
pub fn added(solver: &mut Solver, s: usize, r: usize) {
    if solver.gaussproof.active() {
        solver.gaussproof.added(s, r);
    }
}
