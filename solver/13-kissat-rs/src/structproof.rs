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
//! Satsuma logs orbitopal fixing), `pol` (cutting planes, for counting). The
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

    /// A cutting-planes derivation in reverse Polish notation, e.g.
    /// "12 13 + 2 d". Returns the id of the derived constraint.
    pub fn pol(&mut self, expr: &str) -> std::io::Result<u64> {
        if let Err(e) = writeln!(self.out, "pol {} ;", expr) {
            self.failed = true;
            return Err(e);
        }
        self.constraints += 1;
        self.steps += 1;
        Ok(self.constraints)
    }

    /// Mark a derived constraint as part of F' (it is added to the solver's
    /// formula by the caller).
    pub fn keep(&mut self, id: u64) {
        self.core.push(id);
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
