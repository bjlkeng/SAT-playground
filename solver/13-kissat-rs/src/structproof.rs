//! Not in kissat. The VeriPB proof of the structure pass (plan section 11,
//! 2026-10-07; bead SAT-playground-1v2.1).
//!
//! The pass only adds clauses to the input formula F: units, binaries and
//! at most the empty clause. Every added clause is logged here as a VeriPB 3
//! step over the original clause numbering (clause i of the DIMACS file is
//! constraint i, in file order, duplicates and tautologies included), and
//! the extended formula F' = F plus the derived clauses is what the search
//! then solves, so the search's DRAT proof is a proof of F' and a model of
//! F' is a model of F. The gate checks the two stages apart
//! (`tools/proof_gate.py`): VeriPB on this file with F' as the output
//! formula, drat-trim on F' with the DRAT proof.
//!
//! Steps used: `rup` (implied by unit propagation), `red C : witness` (the
//! redundance rule, with a symmetry of the current formula as witness, as
//! Satsuma logs orbitopal fixing), `pol` (cutting planes, for counting), and
//! for the point step's lex-leader chains a lex order (`def_order`,
//! `load_order`) with `dom` (dominance-based strengthening). The
//! writer numbers derived constraints from m + 1, where m is the number of
//! clauses in the file, so later steps can name earlier ones.

use std::io::{BufWriter, Write};

pub struct Writer {
    out: BufWriter<std::fs::File>,
    /// The number of constraints so far: the m input clauses plus every
    /// derived step that produced a constraint.
    pub constraints: u64,
    /// The ids of derived clauses that are part of F' (moved to the core
    /// set at the end).
    pub core: Vec<u64>,
    /// Set once the empty clause was derived.
    pub refuted: bool,
    pub steps: u64,
    /// Terms written by `rup` and `pol` lines and in `pbc` blocks: literals, ids and
    /// operators, which is what the card engine's proof budget plans for.
    pub written: u64,
    /// A write failed; the proof is unusable and the caller drops the
    /// derived clauses.
    pub failed: bool,
}

fn lit_str(lit: i32) -> String {
    if lit > 0 {
        format!("x{}", lit)
    } else {
        format!("~x{}", -lit)
    }
}

fn clause_str(clause: &[i32]) -> String {
    let mut s = String::new();
    for &l in clause {
        s.push_str("1 ");
        s.push_str(&lit_str(l));
        s.push(' ');
    }
    s.push_str(">= 1");
    s
}

impl Writer {
    pub fn create(path: &str, input_clauses: u64) -> std::io::Result<Writer> {
        let file = std::fs::File::create(path)?;
        let mut w = Writer {
            out: BufWriter::new(file),
            constraints: input_clauses,
            core: Vec::new(),
            refuted: false,
            steps: 0,
            written: 0,
            failed: false,
        };
        writeln!(w.out, "pseudo-Boolean proof version 3.0")?;
        Ok(w)
    }

    /// A clause implied by unit propagation over the current constraints.
    /// Returns its constraint id.
    pub fn rup(&mut self, clause: &[i32]) -> std::io::Result<u64> {
        if let Err(e) = writeln!(self.out, "rup {} ;", clause_str(clause)) {
            self.failed = true;
            return Err(e);
        }
        self.constraints += 1;
        self.steps += 1;
        self.written += clause.len() as u64 + 1;
        if clause.is_empty() {
            self.refuted = true;
        }
        Ok(self.constraints)
    }

    /// A clause redundant under the witness, a literal permutation given as
    /// pairs (variable, literal) meaning the variable maps to that literal
    /// (its negation to the negated literal). The witness must be a
    /// symmetry of the current formula; VeriPB checks every proof goal by
    /// unit propagation.
    pub fn red(&mut self, clause: &[i32], witness: &[(i32, i32)]) -> std::io::Result<u64> {
        let mut line = format!("red {} :", clause_str(clause));
        for &(from, to) in witness {
            line.push_str(&format!(" x{} -> {}", from, lit_str(to)));
        }
        if let Err(e) = writeln!(self.out, "{} ;", line) {
            self.failed = true;
            return Err(e);
        }
        self.constraints += 1;
        self.steps += 1;
        Ok(self.constraints)
    }

    /// A linear constraint implied by unit propagation over the current
    /// constraints: `terms` are (coefficient, literal) pairs, the
    /// constraint their sum >= `rhs`. Returns its constraint id.
    pub fn rup_terms(&mut self, terms: &[(i64, i32)], rhs: i64) -> std::io::Result<u64> {
        let mut line = String::from("rup");
        for &(c, l) in terms {
            line.push_str(&format!(" {} {}", c, lit_str(l)));
        }
        line.push_str(&format!(" >= {} ;", rhs));
        if let Err(e) = writeln!(self.out, "{}", line) {
            self.failed = true;
            return Err(e);
        }
        self.constraints += 1;
        self.steps += 1;
        Ok(self.constraints)
    }

    /// A clause redundant under the witness that sets `var` to `value`:
    /// the definition of a fresh variable (the clause holds the variable,
    /// which occurs nowhere else or only in clauses the witness satisfies).
    pub fn red_const(&mut self, clause: &[i32], var: i32, value: bool) -> std::io::Result<u64> {
        if let Err(e) = writeln!(self.out, "red {} : x{} -> {} ;", clause_str(clause), var, if value { 1 } else { 0 }) {
            self.failed = true;
            return Err(e);
        }
        self.constraints += 1;
        self.steps += 1;
        Ok(self.constraints)
    }

    /// A linear constraint redundant under the witness that sets `var` to
    /// `value`: one half of the definition of a fresh variable as a
    /// threshold, `var -> sum >= k` or `~var -> sum <= k - 1` (bead
    /// SAT-playground-skz.22). `terms` are (coefficient, literal) pairs.
    pub fn red_terms(&mut self, terms: &[(i64, i32)], rhs: i64, var: i32, value: bool) -> std::io::Result<u64> {
        let mut line = String::from("red");
        for &(c, l) in terms {
            line.push_str(&format!(" {} {}", c, lit_str(l)));
        }
        line.push_str(&format!(" >= {} : x{} -> {} ;", rhs, var, if value { 1 } else { 0 }));
        if let Err(e) = writeln!(self.out, "{}", line) {
            self.failed = true;
            return Err(e);
        }
        self.constraints += 1;
        self.steps += 1;
        Ok(self.constraints)
    }

    /// A cutting-planes derivation in reverse Polish notation, e.g.
    /// "12 13 + 2 d". Returns the id of the derived constraint.
    pub fn pol(&mut self, expr: &str) -> std::io::Result<u64> {
        if let Err(e) = writeln!(self.out, "pol {} ;", expr) {
            self.failed = true;
            return Err(e);
        }
        self.constraints += 1;
        self.steps += 1;
        self.written += 1 + expr.split_ascii_whitespace().count() as u64;
        Ok(self.constraints)
    }

    /// A linear constraint proven by contradiction (`pbc`): VeriPB adds the
    /// negation and the subproof's `rup` clauses must reach the empty
    /// clause. `terms` are (coefficient, literal) pairs, the constraint
    /// their sum >= `rhs`; `subproof` is a list of clauses (an empty one
    /// is the contradiction). The negation and every subproof line take a
    /// constraint id of their own before the derived constraint gets the
    /// next one; returns that id.
    pub fn pbc(&mut self, terms: &[(i64, i32)], rhs: i64, subproof: &[Vec<i32>]) -> std::io::Result<u64> {
        let mut line = String::from("pbc");
        for &(c, l) in terms {
            line.push_str(&format!(" {} {}", c, lit_str(l)));
        }
        line.push_str(&format!(" >= {} : subproof", rhs));
        let mut body = String::new();
        for clause in subproof {
            body.push_str("    rup ");
            body.push_str(&clause_str(clause));
            body.push_str(" ;\n");
        }
        if let Err(e) = write!(self.out, "{}\n{}qed;\n", line, body) {
            self.failed = true;
            return Err(e);
        }
        self.constraints += 1 + subproof.len() as u64 + 1;
        self.steps += 1;
        self.written += 1 + terms.len() as u64 + subproof.iter().map(|c| c.len() as u64 + 1).sum::<u64>();
        Ok(self.constraints)
    }

    /// The id the next constraint will get (the negation of a `pbc` goal
    /// takes it, then every subproof line the next ones).
    pub fn next_id(&self) -> u64 {
        self.constraints + 1
    }

    /// A linear constraint proven by contradiction with a subproof of
    /// arbitrary lines (`rup ... ;`, `pol ... ;`), each written as given.
    /// The negation and every line take a constraint id of their own
    /// before the derived constraint gets the next one; returns that id.
    pub fn pbc_raw(&mut self, terms: &[(i128, i32)], rhs: i128, lines: &[String]) -> std::io::Result<u64> {
        let mut line = String::from("pbc");
        for &(c, l) in terms {
            line.push_str(&format!(" {} {}", c, lit_str(l)));
        }
        line.push_str(&format!(" >= {} : subproof", rhs));
        let mut body = String::new();
        for l in lines {
            body.push_str("    ");
            body.push_str(l);
            body.push('\n');
        }
        if let Err(e) = write!(self.out, "{}\n{}qed;\n", line, body) {
            self.failed = true;
            return Err(e);
        }
        self.constraints += 1 + lines.len() as u64 + 1;
        self.steps += 1;
        self.written += 1 + terms.len() as u64 + lines.iter().map(|l| l.split_ascii_whitespace().count() as u64).sum::<u64>();
        Ok(self.constraints)
    }

    /// Delete derived constraints the proof no longer needs (the steps of
    /// a fixing once its clause is derived), so no later witness has to
    /// map them.
    pub fn delete(&mut self, ids: &[u64]) -> std::io::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut line = String::from("del id");
        for id in ids {
            line.push(' ');
            line.push_str(&id.to_string());
        }
        if let Err(e) = writeln!(self.out, "{} ;", line) {
            self.failed = true;
            return Err(e);
        }
        self.steps += 1;
        self.written += ids.len() as u64;
        Ok(())
    }

    /// A lex order over `n` variables (bead SAT-playground-1v2.3.4): u <= v
    /// when sum 2^(n-i) (v_i - u_i) >= 0, u_1 the most significant, as one
    /// linear constraint, with the proof that it is transitive (the sum of
    /// the two premises and the negated goal is a contradiction), as
    /// Satsuma 1.4 defines its dense order. The order is named `lex<n>` and
    /// defines no constraint of the database.
    pub fn def_lex_order(&mut self, n: usize, pow2: &[String]) -> std::io::Result<String> {
        let name = format!("lex{}", n);
        let mut s = format!("def_order {}\nvars\nleft", name);
        for i in 1..=n {
            s.push_str(&format!(" u{}", i));
        }
        s.push_str(" ;\nright");
        for i in 1..=n {
            s.push_str(&format!(" v{}", i));
        }
        s.push_str(" ;\naux ;\nend ;\ndef\n");
        for i in 1..=n {
            let k = &pow2[n - i];
            s.push_str(&format!("-{} u{} {} v{} ", k, i, k, i));
        }
        s.push_str(">= 0 ;\nend ;\ntransitivity\nvars\nfresh_right");
        for i in 1..=n {
            s.push_str(&format!(" w{}", i));
        }
        s.push_str(" ;\nend ;\nproof\nproofgoal #1\npol 1 2 + 3 + ;\nqed : -1 ;\nqed ;\nend ;\nend ;\n");
        if let Err(e) = self.out.write_all(s.as_bytes()) {
            self.failed = true;
            return Err(e);
        }
        self.steps += 1;
        self.written += 4 * n as u64;
        Ok(name)
    }

    /// Load a defined order on the given variables (the first is the most
    /// significant), or with `vars` None unload the loaded one.
    pub fn load_order(&mut self, name: &str, vars: Option<&[i32]>) -> std::io::Result<()> {
        let mut s = String::from("load_order");
        if let Some(vars) = vars {
            s.push(' ');
            s.push_str(name);
            for &v in vars {
                s.push_str(&format!(" x{}", v));
            }
        }
        s.push_str(" ;\n");
        if let Err(e) = self.out.write_all(s.as_bytes()) {
            self.failed = true;
            return Err(e);
        }
        self.steps += 1;
        self.written += 1 + vars.map_or(0, |v| v.len() as u64);
        Ok(())
    }

    /// Dominance-based strengthening under the loaded order: the linear
    /// constraint sum coef * lit >= 0 (`terms`, coefficients as decimal
    /// strings, which may be any size), with the witness, a literal
    /// permutation given as (variable, literal) pairs, that maps the input
    /// onto itself and makes every solution that violates the constraint
    /// strictly smaller. The subproof proves the negated-order goal (the
    /// negated constraint and the goal's premise add up to a contradiction);
    /// VeriPB autoproves the others. Its three constraints take ids before
    /// the derived one gets the next; returns that id.
    pub fn dom(&mut self, terms: &[(&str, i32)], witness: &[(i32, i32)]) -> std::io::Result<u64> {
        let mut s = String::from("dom");
        for &(c, l) in terms {
            s.push(' ');
            s.push_str(c);
            s.push(' ');
            s.push_str(&lit_str(l));
        }
        s.push_str(" >= 0 :");
        for &(from, to) in witness {
            s.push_str(&format!(" x{} -> {}", from, lit_str(to)));
        }
        s.push_str(" : subproof\nproofgoal #2\npol -1 -2 + ;\nqed #2 : -1 ;\nqed ;\n");
        if let Err(e) = self.out.write_all(s.as_bytes()) {
            self.failed = true;
            return Err(e);
        }
        self.constraints += 4;
        self.steps += 1;
        self.written += 1 + 2 * terms.len() as u64 + 2 * witness.len() as u64;
        Ok(self.constraints)
    }

    /// Mark a derived constraint as part of F' (it is added to the solver's
    /// formula by the caller).
    pub fn keep(&mut self, id: u64) {
        self.core.push(id);
    }

    /// Finish a proof that adds no clause to the formula (the readers' test hook,
    /// `SAT_CARD_READ_CHECK`): every line is checked, and nothing is concluded.
    pub fn finish_no_conclusion(mut self) -> std::io::Result<()> {
        writeln!(self.out, "output NONE ;")?;
        writeln!(self.out, "conclusion NONE ;")?;
        writeln!(self.out, "end pseudo-Boolean proof ;")?;
        self.out.flush()
    }

    /// Finish the proof. With the empty clause derived the conclusion is
    /// UNSAT; otherwise the output section states that the file F' the pass
    /// wrote (F plus the kept clauses) is equisatisfiable with F.
    pub fn finish(mut self) -> std::io::Result<()> {
        if self.refuted {
            writeln!(self.out, "output NONE ;")?;
            writeln!(self.out, "conclusion UNSAT ;")?;
        } else {
            if !self.core.is_empty() {
                write!(self.out, "core id")?;
                for id in &self.core {
                    write!(self.out, " {}", id)?;
                }
                writeln!(self.out, " ;")?;
            }
            writeln!(self.out, "output EQUISATISFIABLE FILE ;")?;
            writeln!(self.out, "conclusion NONE ;")?;
        }
        writeln!(self.out, "end pseudo-Boolean proof ;")?;
        self.out.flush()
    }
}
