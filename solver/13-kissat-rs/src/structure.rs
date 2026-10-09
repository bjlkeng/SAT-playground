//! Not in kissat. The structure pass (plan section 11, 2026-10-07; epic
//! SAT-playground-1v2): detect symmetry, parity, counting and CSP structure
//! in the input formula before the solver sees it, and settle what can be
//! settled in seconds.
//!
//! How it fits in. With any `struct*` option on, the parser hands every
//! literal to [`sink`], which buffers the formula instead of adding it to
//! the solver (up to `structclauses` clauses; past that the buffer is
//! flushed into the solver and the pass is off for this run). After the
//! parse, [`run`] gives the buffered formula to each enabled pass. A pass
//! only adds clauses (units, binaries, at most the empty clause) and logs
//! every one in the VeriPB proof (`structproof`). The derived clauses go
//! into the solver first, then the buffered clauses in file order, so the
//! search solves F' = F plus derived and its DRAT proof is a proof of F'.
//! With every option off nothing here runs and the parse path is the stock
//! one, byte for byte.
//!
//! Artifacts next to the DRAT proof path `<proof>`: `<proof>.pbp` (the
//! VeriPB proof) and `<proof>.derived.cnf` (the derived clauses, `p cnf
//! <vars> <k>`); the gate builds F' from the input and this file. Without a
//! proof path the pass runs without artifacts (a SAT answer needs none).

use crate::internal::Solver;
use crate::structproof::Writer;

#[derive(Default)]
pub struct State {
    /// Buffering is on: the parser's literals go to `lits`.
    pub buffering: bool,
    /// The buffer was abandoned (too many clauses); the pass is off.
    pub overflow: bool,
    /// Flat literals, every clause terminated by 0, in file order.
    pub lits: Vec<i32>,
    pub clauses: u64,
    pub max_var: i32,
    /// Work units spent by the passes (one per literal visited, roughly).
    pub work: u64,
    pub work_limit: u64,
}

/// The formula the passes see: clause i is `lits[start[i]..start[i+1]]`,
/// literals deduplicated, tautologies kept as empty slots with `taut[i]`
/// set (so clause i is DIMACS clause i+1 for the proof's constraint ids).
pub struct Formula {
    pub lits: Vec<i32>,
    pub start: Vec<u32>,
    pub taut: Vec<bool>,
    /// The DIMACS clause had a repeated literal: its proof constraint
    /// carries a coefficient above 1, so a proof step that adds it by id
    /// must not assume the deduplicated form.
    pub dup: Vec<bool>,
    pub vars: i32,
}

impl Formula {
    pub fn len(&self) -> usize {
        self.start.len() - 1
    }
    pub fn clause(&self, i: usize) -> &[i32] {
        &self.lits[self.start[i] as usize..self.start[i + 1] as usize]
    }
}

/// What the passes derived, in the order the solver receives it.
pub struct Derived {
    pub clauses: Vec<Vec<i32>>,
    pub proof: Option<Writer>,
    pub refuted: bool,
    /// One line per pass for the `c structure` report.
    pub notes: Vec<String>,
}

impl Derived {
    /// Add a derived clause to F' after it was logged (`id` is its proof
    /// constraint id, 0 without a proof).
    pub fn add(&mut self, clause: &[i32], id: u64) {
        if let Some(w) = self.proof.as_mut() {
            if id != 0 {
                w.keep(id);
            }
        }
        if clause.is_empty() {
            self.refuted = true;
        }
        self.clauses.push(clause.to_vec());
    }
}

pub fn enabled(solver: &Solver) -> bool {
    let o = &solver.options;
    o.structsym != 0 || o.structsymext != 0 || o.structparity != 0 || o.structcount != 0 || o.structcsp != 0
}

/// Called once before the parse: turn buffering on when a pass is enabled.
pub fn prepare(solver: &mut Solver) {
    if enabled(solver) {
        solver.structure.buffering = true;
        solver.structure.work_limit = (solver.options.structticks as u64).saturating_mul(1000);
    }
}

/// The parser's literal sink: buffer, or add to the solver.
#[inline]
pub fn sink(solver: &mut Solver, elit: i32) {
    if !solver.structure.buffering {
        crate::internal::add(solver, elit);
        return;
    }
    solver.structure.lits.push(elit);
    if elit == 0 {
        solver.structure.clauses += 1;
        if solver.structure.clauses > solver.options.structclauses as u64 {
            flush(solver);
            solver.structure.overflow = true;
        }
    } else {
        let v = elit.abs();
        if v > solver.structure.max_var {
            solver.structure.max_var = v;
        }
    }
}

/// Hand the buffer to the solver in file order and turn buffering off.
fn flush(solver: &mut Solver) {
    solver.structure.buffering = false;
    let lits = std::mem::take(&mut solver.structure.lits);
    for &l in &lits {
        crate::internal::add(solver, l);
    }
}

fn build_formula(lits: &[i32], max_var: i32) -> Formula {
    let mut f = Formula { lits: Vec::with_capacity(lits.len()), start: vec![0], taut: Vec::new(), dup: Vec::new(), vars: max_var };
    let mut seen: Vec<u8> = vec![0; max_var as usize + 1]; // 1 = positive seen, 2 = negative, 3 = both
    let mut cur: Vec<i32> = Vec::new();
    for &l in lits {
        if l != 0 {
            cur.push(l);
            continue;
        }
        let mut taut = false;
        let mut out: Vec<i32> = Vec::with_capacity(cur.len());
        for &x in &cur {
            let v = x.unsigned_abs() as usize;
            let bit = if x > 0 { 1 } else { 2 };
            if seen[v] & (3 - bit) != 0 {
                taut = true;
            }
            if seen[v] & bit == 0 {
                seen[v] |= bit;
                out.push(x);
            }
        }
        for &x in &cur {
            seen[x.unsigned_abs() as usize] = 0;
        }
        if !taut {
            f.lits.extend_from_slice(&out);
        }
        f.taut.push(taut);
        f.dup.push(out.len() < cur.len());
        f.start.push(f.lits.len() as u32);
        cur.clear();
    }
    f
}

/// Files the generated artifacts may not alias: the input, the proof, an
/// output file, the policy log and the wrapper's reserved files.
fn artifact_clash(path: &str, taken: &[(&str, &str)]) -> Option<String> {
    for (what, other) in taken {
        if crate::policy_log::same_path(path, other) {
            return Some(format!("'{}' is the {} file", path, what));
        }
    }
    if let Some(stream) = crate::policy_log::standard_stream_behind(path) {
        return Some(format!("'{}' is the file behind this process's {}", path, stream));
    }
    None
}

/// The files this run owns or reads, which the artifacts may not alias.
fn owned_files<'a>(taken: &[(&'a str, &'a str)], log_path: &'a str, reserved: &'a str) -> Vec<(&'a str, &'a str)> {
    let mut all: Vec<(&str, &str)> = taken.to_vec();
    if !log_path.is_empty() {
        all.push(("policy log", log_path));
    }
    for line in reserved.lines() {
        if !line.trim().is_empty() {
            all.push(("wrapper-reserved", line.trim()));
        }
    }
    all
}

/// Remove the sidecars of an earlier run at this proof path, unless one of
/// them is a file this run owns; returns the reason they are still there
/// (a clash, or a file that exists and cannot be removed).
fn retire_sidecars(pbp: &str, der: &str, all: &[(&str, &str)]) -> Option<String> {
    if let Some(why) = artifact_clash(pbp, all).or_else(|| artifact_clash(der, all)) {
        return Some(why);
    }
    for path in [pbp, der] {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Some(format!("cannot remove the earlier '{}': {}", path, e)),
        }
    }
    None
}

/// Run the enabled passes on the buffered formula, then feed the solver.
/// `proof_path` is the DRAT proof path the application opened, if any;
/// `taken` names the files this run owns or reads, which the artifacts
/// may not alias. When the artifacts cannot be written the passes do not
/// run and the formula goes to the solver as parsed.
/// Err(text) when a pass is enabled and the proof path's sidecars cannot
/// be used (one aliases a file of this run, or an earlier run's sidecar
/// cannot be removed): the gate could not check the answer, so the run
/// does not start.
pub fn run(solver: &mut Solver, proof_path: Option<&str>, taken: &[(&str, &str)]) -> Result<(), String> {
    // the sidecar paths and the files they may not alias (the proof on
    // stdout has no sidecar path at all)
    let log_path: String = solver.policy.log.as_ref().map(|l| l.path.clone()).unwrap_or_default();
    let reserved = std::env::var("SAT_POLICY_LOG_RESERVED").unwrap_or_default();
    // the weights file of a learned policy (SAT_POLICY=<path>) is read, not
    // written, but it is a file this run must not delete
    let weights: String = match std::env::var("SAT_POLICY") {
        Ok(v) if !matches!(v.trim(), "" | "stock" | "random" | "jitter") => v.trim().to_string(),
        _ => String::new(),
    };
    let mut all: Vec<(&str, &str)> = owned_files(taken, &log_path, &reserved);
    if !weights.is_empty() {
        all.push(("weights", weights.as_str()));
    }
    let sidecars: Option<(String, String)> = match proof_path {
        Some(p) if p != "-" => Some((format!("{}.pbp", p), format!("{}.derived.cnf", p))),
        _ => None,
    };
    if !solver.structure.buffering {
        // off, or overflowed and flushed during the parse: an earlier run's
        // sidecars at this proof path would mislead the gate, so they go
        // (never a file this run owns); a sidecar that stays is reported,
        // since the gate would then check the wrong proof
        if let Some((pbp, der)) = &sidecars {
            if let Some(why) = retire_sidecars(pbp, der, &all) {
                if enabled(solver) {
                    return Err(format!("structure pass: {}", why));
                }
                crate::print::message(solver, format!("structure sidecars of an earlier run remain: {}", why));
                println!("c structure sidecars stale: {}", why);
            }
        }
        if solver.structure.overflow {
            crate::print::message(solver, format!("structure pass off: more than {} clauses", solver.options.structclauses));
        }
        return Ok(());
    }
    solver.structure.buffering = false;
    let lits = std::mem::take(&mut solver.structure.lits);
    // the pass's work counts in the work clock and against the tick limit
    let mut budget = solver.structure.work_limit;
    if solver.limited.ticks {
        let w = solver.statistics.work_clock();
        budget = budget.min(solver.limits.ticks.saturating_sub(w));
    }
    let mut proof: Option<Writer> = None;
    let mut proof_ok = true;
    if proof_path == Some("-") {
        crate::print::message(solver, "structure pass off: the proof goes to stdout, which has no sidecar files".to_string());
        proof_ok = false;
    } else if let Some((pbp, der)) = &sidecars {
        match retire_sidecars(pbp, der, &all) {
            Some(why) => {
                // the formula is still buffered: nothing was fed to the solver
                return Err(format!("structure pass: {}", why));
            }
            None => match Writer::create(pbp, solver.structure.clauses) {
                Ok(w) => proof = Some(w),
                Err(e) => {
                    crate::print::message(solver, format!("structure pass off: cannot write {}: {}", pbp, e));
                    proof_ok = false;
                }
            },
        }
    }
    if !proof_ok {
        // the formula goes to the solver as parsed
        for &l in &lits {
            crate::internal::add(solver, l);
        }
        return Ok(());
    }
    let mut derived = Derived { clauses: Vec::new(), proof, refuted: false, notes: Vec::new() };
    // the passes count their work straight into statistics.ticks, so a
    // signal during the pass reports it, and they stop on the termination
    // flag (a time limit or a signal)
    let start = solver.statistics.ticks;
    if proof_ok {
        let formula = build_formula(&lits, solver.structure.max_var);
        let stop: *const std::sync::atomic::AtomicBool = &solver.termination.flagged;
        // SAFETY: the flag outlives the pass and is only read through it
        let stop_ref: &std::sync::atomic::AtomicBool = unsafe { &*stop };
        if solver.options.structparity != 0 && !derived.refuted {
            crate::structparity::run(&formula, &mut derived, budget, &mut solver.statistics.ticks, stop_ref);
        }
        if solver.options.structcount != 0 && !derived.refuted {
            crate::structcount::run(&formula, &mut derived, budget, &mut solver.statistics.ticks, stop_ref);
        }
        if solver.options.structsym != 0 && !derived.refuted {
            crate::structsym::run(&formula, &mut derived, budget, &mut solver.statistics.ticks, stop_ref);
        }
    }
    let work = solver.statistics.ticks.saturating_sub(start);
    solver.structure.work = work;
    // the proof must be complete before its clauses are used; a failed
    // proof drops the derived clauses and the solver sees the input alone
    let mut k = derived.clauses.len();
    if let Some((pbp, der)) = &sidecars {
        let (pbp, der) = (pbp.clone(), der.clone());
        match derived.proof.take() {
            Some(w) if k > 0 => {
                let finished = if w.failed { Err("a proof step could not be written".to_string()) } else { w.finish().map_err(|e| e.to_string()) };
                let written = finished.and_then(|_| write_derived(&der, &derived.clauses, solver.structure.max_var).map_err(|e| e.to_string()));
                match written {
                    Ok(()) => println!("c structure proof {} derived {}", pbp, der),
                    Err(e) => {
                        crate::print::message(solver, format!("structure pass dropped: {}", e));
                        derived.clauses.clear();
                        derived.refuted = false;
                        k = 0;
                        let _ = std::fs::remove_file(&pbp);
                        let _ = std::fs::remove_file(&der);
                    }
                }
            }
            Some(w) => {
                drop(w);
                let _ = std::fs::remove_file(&pbp);
                let _ = std::fs::remove_file(&der);
            }
            None => {}
        }
    }
    // the derived clauses first, then the formula in file order
    for c in &derived.clauses {
        for &l in c {
            crate::internal::add(solver, l);
        }
        crate::internal::add(solver, 0);
    }
    for &l in &lits {
        crate::internal::add(solver, l);
    }
    for note in &derived.notes {
        crate::print::message(solver, format!("structure pass: {}", note));
    }
    if k > 0 {
        let units = derived.clauses.iter().filter(|c| c.len() == 1).count();
        crate::print::message(
            solver,
            format!(
                "structure pass: {} derived clause{} ({} unit{}), {}, work {}K",
                k,
                if k == 1 { "" } else { "s" },
                units,
                if units == 1 { "" } else { "s" },
                if derived.refuted { "refuted" } else { "equisatisfiable extension" },
                work / 1000
            ),
        );
    }
    Ok(())
}
fn write_derived(path: &str, clauses: &[Vec<i32>], max_var: i32) -> std::io::Result<()> {
    use std::io::Write;
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    writeln!(out, "p cnf {} {}", max_var, clauses.len())?;
    for c in clauses {
        for &l in c {
            write!(out, "{} ", l)?;
        }
        writeln!(out, "0")?;
    }
    out.flush()
}
