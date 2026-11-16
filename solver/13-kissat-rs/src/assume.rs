//! Not in kissat (epic SAT-playground-bp3, plan/assumptions-plan.md):
//! solving under assumptions.
//!
//! One solver is asked again and again about the same formula: "is it
//! satisfiable with these literals true, for now?"  It answers, takes the
//! literals back, and keeps what it learned.  An answer is SAT with a model
//! that satisfies the assumptions, UNSAT with a failed core (a subset of the
//! assumptions that cannot hold together with the formula), or unknown at a
//! limit.
//!
//! How it works.  Assumption `i` of the list belongs to decision level `i`.
//! Before every free decision the search loop looks whether its level is
//! still below the number of assumptions; if so the next assumption is
//! decided instead (`decide` below).  One already true gets a level with
//! nothing on it, so the levels and the list stay in step (kitten does the
//! same).  One already false ends the solve, and `fail` walks the trail back
//! from it to the assumptions that forced it: the failed core.
//!
//! With no assumptions `count` is zero and the search does what it always
//! did; the level test is the only thing added to its path.
//!
//! A variable can only be assumed while it is frozen: no pass may eliminate
//! it or replace it by an equivalent one.  The frozen mark is a spare bit of
//! the variable's flags, read where the parity engine's columns are already
//! protected (eliminate.rs, substitute.rs, fastel.rs).
//!
//! The caller's literals are DIMACS numbers.  They are mapped to internal
//! ones each time they are read, because a compaction in the middle of a
//! search renumbers the internal variables.

use std::sync::atomic::Ordering;

use crate::internal::{Solver, DECISION_REASON};
use crate::literal::{idx, not, EXTERNAL_MAX_VAR, INVALID_LIT};

#[derive(Default)]
pub struct Assume {
    /// The number of assumptions of the solve in progress; zero when no
    /// solve is in progress or it has none.  The search loop compares the
    /// level with this before every free decision.
    pub count: u32,
    /// The literals for the next solve, in the caller's order.
    pub list: Vec<i32>,
    /// The list of the solve in progress, or of the last one.
    pub current: Vec<i32>,
    /// Per place of `current`: in the failed core of the last UNSAT answer.
    pub failed: Vec<bool>,
    /// How often each DIMACS variable is frozen.
    pub frozen: Vec<u32>,
    /// Limits for the next solve only (conflicts; work-clock units).
    pub call_conflicts: Option<u64>,
    pub call_ticks: Option<u64>,
    /// The run's own limits (`--conflicts`, `--decisions`,
    /// `SAT_LIMIT_TICKS`) as absolute numbers, remembered at the first solve:
    /// the search clears its limits when it stops.
    run_set: bool,
    run_conflicts: Option<u64>,
    run_decisions: Option<u64>,
    run_ticks: Option<u64>,
    /// A limit of the run was reached: every later solve answers 0 at once.
    pub exhausted: bool,
    /// The mode the last search ended in (`stop_search` forgets it): a
    /// later solve goes on in it.
    pub stable: bool,
    /// For the exit line.
    pub solves: u64,
    pub sat: u64,
    pub unsat: u64,
    pub unknown: u64,
    pub decided: u64,
    pub empty: u64,
    pub core: u64,
}

/// The internal literal of a DIMACS literal that is imported and not
/// eliminated.
#[inline]
fn internal(solver: &Solver, elit: i32) -> u32 {
    let import = &solver.import_[elit.unsigned_abs() as usize];
    debug_assert!(import.imported && !import.eliminated);
    if elit < 0 {
        not(import.lit)
    } else {
        import.lit
    }
}

/// From now on no pass removes or renames this variable.  Counted: two
/// freezes need two melts.  Refused when the variable is already eliminated
/// or substituted.  A variable the formula never used becomes an ordinary
/// free variable first.
pub fn freeze(solver: &mut Solver, var: i32) -> Result<(), String> {
    if var <= 0 || var > EXTERNAL_MAX_VAR {
        return Err(format!("cannot freeze '{}': not a variable", var));
    }
    let v = var as usize;
    if v < solver.import_.len() && solver.import_[v].imported && solver.import_[v].eliminated {
        return Err(format!(
            "variable {} was eliminated or substituted before it was frozen",
            var
        ));
    }
    let ilit = crate::import::import_literal(solver, var);
    debug_assert!(ilit != INVALID_LIT);
    let i = idx(ilit) as usize;
    let flags = solver.flags[i];
    if !flags.active() && !flags.fixed() {
        crate::flags::activate_literal(solver, ilit);
    }
    if !solver.flags[i].fixed() {
        solver.flags[i].set_frozen(true);
    }
    if solver.assume.frozen.len() <= v {
        solver.assume.frozen.resize(v + 1, 0);
    }
    solver.assume.frozen[v] += 1;
    Ok(())
}

/// Take one freeze back.
pub fn melt(solver: &mut Solver, var: i32) {
    let v = var as usize;
    debug_assert!(var > 0 && v < solver.assume.frozen.len() && solver.assume.frozen[v] > 0);
    if var <= 0 || v >= solver.assume.frozen.len() || solver.assume.frozen[v] == 0 {
        return;
    }
    solver.assume.frozen[v] -= 1;
    if solver.assume.frozen[v] != 0 {
        return;
    }
    let import = solver.import_[v];
    if import.imported && !import.eliminated {
        solver.flags[idx(import.lit) as usize].set_frozen(false);
    }
}

/// Add a literal to the next solve's list.  Its variable stays frozen while
/// it is in the list.
pub fn assume(solver: &mut Solver, elit: i32) -> Result<(), String> {
    if elit == 0 || elit == i32::MIN || elit.unsigned_abs() > EXTERNAL_MAX_VAR as u32 {
        return Err(format!("cannot assume '{}': not a literal", elit));
    }
    if solver.policy.on {
        return Err("assumptions cannot be used together with the RL policy (SAT_POLICY)".to_string());
    }
    if let Some(what) = engine_present(solver) {
        return Err(format!("assumptions cannot be used in a solver that has {}", what));
    }
    freeze(solver, elit.abs())?;
    solver.assume.list.push(elit);
    Ok(())
}

/// The parity engine, the count engine or the free-choice walk, when the
/// structure stage gave this solver one.  They do not know about
/// assumptions: an engine's lazy reasons would have to be written out before
/// a failed core is analysed, and the walk answers SAT without looking at
/// them.  Until they do (plan, step 3), the two cannot be in one solver.
/// The test mode turns the structure stage off, and the split's own solvers
/// run without it, so neither meets this.
fn engine_present(solver: &Solver) -> Option<&'static str> {
    if solver.gauss.on || solver.gauss.xors.len() >= 2 {
        Some("the parity engine (--gauss)")
    } else if solver.card.on || !solver.card.nets.is_empty() {
        Some("the count engine (--card)")
    } else if solver.freewalk.walk.is_some() || solver.freewalk.model.is_some() {
        Some("the free-choice walk (--freewalk)")
    } else {
        None
    }
}

/// Limits for the next solve only: conflicts, and work-clock units.
pub fn limit(solver: &mut Solver, conflicts: Option<u64>, ticks: Option<u64>) {
    solver.assume.call_conflicts = conflicts;
    solver.assume.call_ticks = ticks;
}

/// After an UNSAT answer: is the assumption at this place of the list in
/// the failed core?
pub fn failed(solver: &Solver, place: usize) -> bool {
    solver.assume.failed.get(place).copied().unwrap_or(false)
}

/// Has a limit of the run been reached?
fn run_limit_reached(solver: &Solver) -> bool {
    let a = &solver.assume;
    let st = &solver.statistics;
    a.run_conflicts.is_some_and(|c| st.conflicts >= c)
        || a.run_decisions.is_some_and(|d| st.decisions >= d)
        || a.run_ticks.is_some_and(|t| st.work_clock() >= t)
}

/// Set the search's limits for this solve: the smaller of the run's limit
/// and the call's own.
fn arm_limits(solver: &mut Solver) {
    if !solver.assume.run_set {
        solver.assume.run_set = true;
        solver.assume.run_conflicts = solver.limited.conflicts.then_some(solver.limits.conflicts);
        solver.assume.run_decisions = solver.limited.decisions.then_some(solver.limits.decisions);
        solver.assume.run_ticks = solver.limited.ticks.then_some(solver.limits.ticks);
    }
    let call_conflicts = solver.assume.call_conflicts.take().map(|n| solver.statistics.conflicts.saturating_add(n));
    let call_ticks = solver.assume.call_ticks.take().map(|n| solver.statistics.work_clock().saturating_add(n));
    let smaller = |a: Option<u64>, b: Option<u64>| match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    };
    let conflicts = smaller(solver.assume.run_conflicts, call_conflicts);
    let ticks = smaller(solver.assume.run_ticks, call_ticks);
    let decisions = solver.assume.run_decisions;
    solver.limited.conflicts = conflicts.is_some();
    solver.limits.conflicts = conflicts.unwrap_or(0);
    solver.limited.decisions = decisions.is_some();
    solver.limits.decisions = decisions.unwrap_or(0);
    solver.limited.ticks = ticks.is_some();
    solver.limits.ticks = ticks.unwrap_or(0);
}

/// Answer under the list: 10 SAT, 20 UNSAT (the core is in `failed`, empty
/// when the formula itself is unsatisfiable), 0 unknown.  The list is used
/// once.  With an empty list this is the plain solve, asked again.
pub fn solve(solver: &mut Solver) -> i32 {
    debug_assert!(solver.clause.is_empty(), "incomplete clause (terminating zero not added)");
    let list = std::mem::take(&mut solver.assume.list);
    solver.assume.failed.clear();
    solver.assume.failed.resize(list.len(), false);
    solver.assume.current = list;
    solver.assume.solves += 1;

    // the last answer's model goes: back to level zero
    solver.extended = false;
    restore_mode(solver);
    if !solver.inconsistent {
        crate::backtrack::backtrack_propagate_and_flush_trail(solver);
    }

    arm_limits(solver);
    let over = solver.assume.exhausted
        || run_limit_reached(solver)
        || solver.termination.stopped.load(Ordering::SeqCst);
    let res = if solver.inconsistent {
        20
    } else if over {
        0
    } else {
        search_under(solver)
    };
    if run_limit_reached(solver) {
        solver.assume.exhausted = true;
    }
    restore_mode(solver);
    match res {
        10 => solver.assume.sat += 1,
        20 => solver.assume.unsat += 1,
        _ => solver.assume.unknown += 1,
    }
    for i in 0..solver.assume.current.len() {
        let var = solver.assume.current[i].abs();
        melt(solver, var);
    }
    res
}

/// The search clears its mode flag when it stops.  Between two solves the
/// flag is set again to the mode the last search ended in, because a
/// variable that becomes free or active goes into that mode's own structure
/// (the score heap in stable mode, the queue in focused mode).  Found by the
/// fuzz harness: without this a second search in stable mode met an empty
/// heap.
fn restore_mode(solver: &mut Solver) {
    if solver.statistics.searches != 0 {
        solver.stable = solver.assume.stable;
    }
}

/// The search of one solve.  `count` is non-zero only while a search under
/// assumptions runs: the search and the passes read it.
fn search_under(solver: &mut Solver) -> i32 {
    solver.assume.count = solver.assume.current.len() as u32;
    let res = crate::search::search(solver);
    solver.assume.count = 0;
    res
}

/// Called by the search when it starts under assumptions, after its limits
/// are set: elimination removes variables, so it waits for a plain solve.
pub fn started(solver: &mut Solver) {
    debug_assert!(solver.assume.count != 0);
    solver.enabled.eliminate = false;
}

/// Called by the search instead of its first-search set-up when it has
/// searched before: scores, phases, averages, limits and the random stream
/// all go on from where the last search stopped.
pub fn start_again(solver: &mut Solver) {
    let stable = solver.assume.stable;
    solver.stable = stable;
    let searches = solver.statistics.searches;
    let conflicts = solver.statistics.conflicts;
    crate::print::phase(
        solver,
        "search",
        searches,
        format_args!(
            "continuing {} search after {} conflicts",
            if stable { "stable" } else { "focus" },
            conflicts
        ),
    );
    crate::kimits::init_enabled(solver);
    // The target and best phases stay, but not their measure (how many
    // variables were assigned without a conflict when they were saved): it
    // was taken under other assumptions, and after a SAT answer it is "all
    // of them", which nothing could beat.  From zero, the first assignments
    // of this solve replace them.
    solver.target_assigned = 0;
    solver.best_assigned = 0;
    if stable {
        crate::profile::start_checked(solver, crate::profile::Prof::stable);
        crate::report::report(solver, false, '[');
    } else {
        crate::profile::start_checked(solver, crate::profile::Prof::focused);
        crate::report::report(solver, false, '{');
    }
}

/// The next assumption instead of a free decision; called while
/// `level < count`.  Returns 0 to go on and 20 when the assumption is
/// false.
pub fn decide(solver: &mut Solver) -> i32 {
    let place = solver.level as usize;
    debug_assert!(place < solver.assume.current.len());
    let lit = internal(solver, solver.assume.current[place]);
    let value = solver.values[lit as usize];
    if value < 0 {
        fail(solver, place, lit);
        return 20;
    }
    solver.level += 1;
    if value > 0 {
        // true already: a level with nothing on it
        crate::frames::push_empty_frame(solver);
        solver.assume.empty += 1;
    } else {
        crate::frames::push_frame(solver, lit);
        crate::assign::assign_decision(solver, lit);
        solver.assume.decided += 1;
    }
    0
}

/// The assumption at `place` (internal literal `lit`) is false.  Mark the
/// assumptions that force this in `failed`, and add the clause that negates
/// them to the proof.
fn fail(solver: &mut Solver, place: usize, lit: u32) {
    debug_assert!(solver.values[lit as usize] < 0);
    debug_assert!(solver.analyzed.is_empty());
    solver.assume.failed[place] = true;
    let mut clashing = false;
    let root = solver.assigned[idx(lit) as usize].level == 0;
    if !root {
        // every decision on the trail is an assumption here (the search is
        // still inside the assumption levels), and a literal's reason
        // literals come before it on the trail
        crate::inline::push_analyzed(solver, idx(lit));
        let mut open = 1u32;
        let mut t = solver.trail.len();
        while open > 0 {
            debug_assert!(t > 0);
            t -= 1;
            let l = solver.trail[t];
            if !solver.assigned[idx(l) as usize].analyzed() {
                continue;
            }
            open -= 1;
            // a reason the parity engine keeps as a row becomes a clause
            // here, so the proof's checker can follow it
            let a = crate::gauss::assigned_with_reason(solver, l);
            debug_assert!(a.level != 0);
            if a.binary() {
                mark(solver, a.reason, &mut open);
            } else if a.reason == DECISION_REASON {
                let p = (a.level - 1) as usize;
                debug_assert!(p < place);
                solver.assume.failed[p] = true;
                if l == not(lit) {
                    clashing = true;
                }
            } else {
                let size = solver.arena.clause(a.reason).size();
                for i in 0..size {
                    let other = solver.arena.clause(a.reason).lit(i);
                    if other != l {
                        mark(solver, other, &mut open);
                    }
                }
            }
        }
        crate::analyze::reset_only_analyzed_literals(solver);
    }
    let core = solver.assume.failed.iter().filter(|&&f| f).count();
    solver.assume.core += core as u64;
    // The proof line: the clause that negates the core.  Set the core, and
    // propagation alone reaches the conflict.  Not needed when the false
    // assumption is a root unit (its negation is a unit of the proof or of
    // the formula), nor when the core holds a literal and its negation (the
    // clause would be a tautology).
    if solver.proof.is_some() && !root && !clashing {
        let clause: Vec<i32> = (0..solver.assume.current.len())
            .filter(|&p| solver.assume.failed[p])
            .map(|p| -solver.assume.current[p])
            .collect();
        crate::proof::add_external_lits_to_proof(solver, &clause);
    }
}

#[inline]
fn mark(solver: &mut Solver, other: u32, open: &mut u32) {
    let i = idx(other);
    let b = &solver.assigned[i as usize];
    if b.level != 0 && !b.analyzed() {
        crate::inline::push_analyzed(solver, i);
        *open += 1;
    }
}

/// The exit line; nothing when no solve went through this module.
pub fn report(solver: &Solver) {
    let a = &solver.assume;
    if a.solves == 0 {
        return;
    }
    println!(
        "c assume solves={} sat={} unsat={} unknown={} decided={} empty={} core={}",
        a.solves, a.sat, a.unsat, a.unknown, a.decided, a.empty, a.core
    );
}

// ---------------------------------------------------------------------------
// The test mode: an incremental CNF file, one answer block per question
// ---------------------------------------------------------------------------
//
// `--inccnf=1` reads the format CaDiCaL and iLingeling read:
//
//     p inccnf
//     1 -2 0
//     2 3 0
//     a 1 2 0
//     a 0
//
// The header has no counts.  Clauses come first; every `a` line after them
// is one question: solve under these literals.  Each question is answered
// on its own (CaDiCaL's binary treats them as cases of one question and
// stops at the first satisfiable one).  Per question one block is printed:
//
//     q 1 UNSATISFIABLE
//     f 1 2 0            the failed core, in the question's order
//     p 37               bytes of the DRAT file so far (with a proof)
//     q 2 SATISFIABLE
//     v 1 -2 -3 0        the model
//     p 37
//
// No `s` line is printed, so the output cannot be taken for a competition
// answer.

pub struct Inccnf {
    /// The clauses, each ended by a zero.
    pub clauses: Vec<i32>,
    pub questions: Vec<Vec<i32>>,
    pub max_var: i32,
}

/// Parse the bytes of an incremental CNF file.
pub fn parse_inccnf(bytes: &[u8]) -> Result<Inccnf, String> {
    let mut out = Inccnf { clauses: Vec::new(), questions: Vec::new(), max_var: 0 };
    let mut header = false;
    let mut open_clause = false;
    let mut question: Option<Vec<i32>> = None;
    let mut lineno = 1usize;
    let n = bytes.len();
    let mut i = 0usize;
    let at_line_start = |i: usize| i == 0 || bytes[i - 1] == b'\n';
    while i < n {
        let ch = bytes[i];
        if ch == b'\n' {
            lineno += 1;
            i += 1;
            continue;
        }
        if ch == b' ' || ch == b'\t' || ch == b'\r' {
            i += 1;
            continue;
        }
        if ch == b'c' && at_line_start(i) {
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if ch == b'p' {
            if header {
                return Err(format!("line {}: a second header", lineno));
            }
            let end = bytes[i..].iter().position(|&b| b == b'\n').map_or(n, |k| i + k);
            let line = std::str::from_utf8(&bytes[i..end]).map_err(|_| format!("line {}: the header is not text", lineno))?;
            let words: Vec<&str> = line.split_whitespace().collect();
            if words != ["p", "inccnf"] {
                return Err(format!("line {}: expected the header 'p inccnf', got '{}'", lineno, line.trim_end()));
            }
            header = true;
            i = end;
            continue;
        }
        if !header {
            return Err(format!("line {}: expected the header 'p inccnf' first", lineno));
        }
        if ch == b'a' {
            if open_clause {
                return Err(format!("line {}: a question inside a clause (a zero is missing)", lineno));
            }
            if question.is_some() {
                return Err(format!("line {}: a question inside a question (a zero is missing)", lineno));
            }
            question = Some(Vec::new());
            i += 1;
            continue;
        }
        // an integer
        let start = i;
        if ch == b'-' {
            i += 1;
        }
        let digits = i;
        let mut value: i64 = 0;
        while i < n && bytes[i].is_ascii_digit() {
            value = value * 10 + (bytes[i] - b'0') as i64;
            if value > EXTERNAL_MAX_VAR as i64 {
                return Err(format!("line {}: a variable past the largest one ({})", lineno, EXTERNAL_MAX_VAR));
            }
            i += 1;
        }
        if i == digits {
            return Err(format!("line {}: unexpected character '{}'", lineno, bytes[start] as char));
        }
        if i < n && !(bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\r' || bytes[i] == b'\n') {
            return Err(format!("line {}: unexpected character '{}' after a number", lineno, bytes[i] as char));
        }
        let lit = if bytes[start] == b'-' { -(value as i32) } else { value as i32 };
        if lit.abs() > out.max_var {
            out.max_var = lit.abs();
        }
        match question.as_mut() {
            Some(q) => {
                if lit == 0 {
                    out.questions.push(question.take().unwrap());
                } else {
                    q.push(lit);
                }
            }
            None => {
                if !out.questions.is_empty() {
                    return Err(format!(
                        "line {}: a clause after the first question (adding clauses between solves is not supported)",
                        lineno
                    ));
                }
                out.clauses.push(lit);
                open_clause = lit != 0;
            }
        }
    }
    if !header {
        return Err("no 'p inccnf' header".to_string());
    }
    if open_clause {
        return Err("the last clause has no closing zero".to_string());
    }
    if question.is_some() {
        return Err("the last question has no closing zero".to_string());
    }
    Ok(out)
}

/// The options the test mode cannot run with, each turned off (with a
/// message when it was on): the structure stage, the engines, the
/// free-choice walk and the control options.
pub fn inccnf_options(solver: &mut Solver) {
    let mut turned_off: Vec<&str> = Vec::new();
    for name in [
        "structsym", "structsymext", "structpoints", "structparity", "structcount", "structcsp", "structdd",
        "structarith", "structsort", "structdiv", "gauss", "card", "freewalk", "freeread", "ctlfirst", "ctlsplit",
    ] {
        if crate::internal::get_option(solver, name) != 0 {
            crate::internal::set_option(solver, name, 0);
            turned_off.push(name);
        }
    }
    if !turned_off.is_empty() {
        crate::print::message(
            solver,
            format!("incremental mode: turned off {}", turned_off.join(", ")),
        );
    }
}

/// Answer every question of a parsed file; `max_var` is the largest
/// variable for the model lines.  Returns Err with a message when a
/// question is refused.
pub fn answer_questions(solver: &mut Solver, input: &Inccnf, witness: bool) -> Result<(), String> {
    use std::io::Write;
    // every question's variables are frozen before the first solve, so a
    // plain solve between two questions cannot remove one
    for q in &input.questions {
        for &l in q {
            freeze(solver, l.abs())?;
        }
    }
    let conflicts = solver.options.assumeconflicts;
    let ticks = solver.options.assumeticks;
    for (n, q) in input.questions.iter().enumerate() {
        for &l in q {
            assume(solver, l)?;
        }
        limit(
            solver,
            (conflicts >= 0).then_some(conflicts as u64),
            (ticks >= 0).then(|| (ticks as u64).saturating_mul(1000)),
        );
        let before = (solver.statistics.work_clock(), solver.statistics.conflicts, solver.statistics.decisions);
        let res = solve(solver);
        let status = match res {
            10 => "SATISFIABLE",
            20 => "UNSATISFIABLE",
            _ => "UNKNOWN",
        };
        // what this question cost (a comment line; not printed with -q)
        crate::print::message(
            solver,
            format!(
                "question {} work {} conflicts {} decisions {}",
                n + 1,
                solver.statistics.work_clock() - before.0,
                solver.statistics.conflicts - before.1,
                solver.statistics.decisions - before.2
            ),
        );
        println!("q {} {}", n + 1, status);
        if res == 10 && witness {
            crate::witness::print_witness(solver, input.max_var, false);
        } else if res == 20 {
            let mut line = String::from("f");
            for (place, &l) in q.iter().enumerate() {
                if failed(solver, place) {
                    line.push_str(&format!(" {}", l));
                }
            }
            line.push_str(" 0");
            println!("{}", line);
        }
        if solver.proof.is_some() {
            println!("p {}", crate::proof::flushed_bytes(solver));
        }
        let _ = std::io::stdout().flush();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal;

    fn solver_of(clauses: &[&[i32]]) -> Box<Solver> {
        let mut s = Box::new(internal::init());
        internal::set_option(&mut s, "quiet", 1);
        for c in clauses {
            for &l in *c {
                internal::add(&mut s, l);
            }
            internal::add(&mut s, 0);
        }
        s
    }

    fn ask(s: &mut Solver, lits: &[i32]) -> i32 {
        for &l in lits {
            assume(s, l).unwrap();
        }
        solve(s)
    }

    fn core(s: &Solver, lits: &[i32]) -> Vec<i32> {
        (0..lits.len()).filter(|&p| failed(s, p)).map(|p| lits[p]).collect()
    }

    // the plan's worked example: a=1 b=2 c=3 d=4 x=5 y=6
    const EXAMPLE: &[&[i32]] = &[&[-1, 5], &[-2, -5, 6], &[-3, -6]];

    #[test]
    fn worked_example_core_then_model() {
        let mut s = solver_of(EXAMPLE);
        let q = [1, 2, 4, 3];
        assert_eq!(ask(&mut s, &q), 20);
        assert_eq!(core(&s, &q), vec![1, 2, 3]);
        assert!(!s.inconsistent);
        // asked again with less, the same solver answers SAT
        assert_eq!(ask(&mut s, &[1, 2]), 10);
        for l in [1, 2, 5, 6, -3] {
            assert_eq!(internal::value(&mut s, l), l, "literal {}", l);
        }
        // and the first question still fails, with the same core
        assert_eq!(ask(&mut s, &q), 20);
        assert_eq!(core(&s, &q), vec![1, 2, 3]);
        assert_eq!(s.assume.solves, 3);
    }

    #[test]
    fn empty_list_is_the_plain_solve() {
        let mut s = solver_of(EXAMPLE);
        assert_eq!(ask(&mut s, &[]), 10);
        assert_eq!(ask(&mut s, &[3, 2]), 10);
        assert_eq!(internal::value(&mut s, 1), -1);
        assert_eq!(ask(&mut s, &[]), 10);
    }

    #[test]
    fn a_literal_twice_and_with_its_negation() {
        let mut s = solver_of(EXAMPLE);
        // twice: the second is true at its turn, an empty level
        assert_eq!(ask(&mut s, &[1, 1, 2]), 10);
        assert_eq!(internal::value(&mut s, 6), 6);
        assert!(s.assume.empty >= 1);
        // with its negation: the pair is the core
        let q = [4, 1, -4, 2];
        assert_eq!(ask(&mut s, &q), 20);
        assert_eq!(core(&s, &q), vec![4, -4]);
        assert_eq!(ask(&mut s, &[4]), 10);
        assert_eq!(ask(&mut s, &[-4]), 10);
    }

    #[test]
    fn all_assigned_before_the_last_assumption() {
        // under a, propagation sets b false and no variable is left: that
        // is not a model while the assumption b is still to be looked at
        let mut s = solver_of(&[&[-1, -2]]);
        let q = [1, 2];
        assert_eq!(ask(&mut s, &q), 20);
        assert_eq!(core(&s, &q), vec![1, 2]);
        assert_eq!(ask(&mut s, &[1]), 10);
        assert_eq!(internal::value(&mut s, 2), -2);
    }

    #[test]
    fn a_root_unit_fails_alone() {
        let mut s = solver_of(&[&[-1], &[1, 2, 3]]);
        let q = [2, 1, 3];
        assert_eq!(ask(&mut s, &q), 20);
        assert_eq!(core(&s, &q), vec![1]);
        assert_eq!(ask(&mut s, &[2, 3]), 10);
    }

    #[test]
    fn an_unsatisfiable_formula_has_an_empty_core() {
        let mut s = solver_of(&[&[1, 2], &[1, -2], &[-1, 3], &[-1, -3]]);
        let q = [2, 3];
        assert_eq!(ask(&mut s, &q), 20);
        assert!(s.inconsistent);
        assert_eq!(core(&s, &q), Vec::<i32>::new());
        assert_eq!(ask(&mut s, &[]), 20);
    }

    #[test]
    fn a_variable_the_formula_never_used() {
        let mut s = solver_of(EXAMPLE);
        assert_eq!(ask(&mut s, &[9, 1]), 10);
        assert_eq!(internal::value(&mut s, 9), 9);
        assert_eq!(ask(&mut s, &[-9]), 10);
        assert_eq!(internal::value(&mut s, 9), -9);
        let q = [9, -9];
        assert_eq!(ask(&mut s, &q), 20);
        assert_eq!(core(&s, &q), vec![9, -9]);
    }

    #[test]
    fn freeze_counts_and_refuses_an_eliminated_variable() {
        let mut s = solver_of(EXAMPLE);
        freeze(&mut s, 5).unwrap();
        freeze(&mut s, 5).unwrap();
        let i = idx(internal(&s, 5)) as usize;
        assert!(s.flags[i].frozen());
        melt(&mut s, 5);
        assert!(s.flags[i].frozen());
        melt(&mut s, 5);
        assert!(!s.flags[i].frozen());
        // a variable marked eliminated cannot be frozen or assumed
        s.import_[6].eliminated = true;
        assert!(freeze(&mut s, 6).is_err());
        assert!(assume(&mut s, -6).is_err());
        assert!(s.assume.list.is_empty());
        s.import_[6].eliminated = false;
        assert!(freeze(&mut s, 0).is_err());
        assert!(assume(&mut s, 0).is_err());
    }

    /// Random 3-SAT plus a few gates and equivalences whose outputs occur
    /// nowhere else: what elimination and substitution remove first.
    fn removable_formula() -> (Vec<Vec<i32>>, Vec<i32>) {
        let mut state: u64 = 12345;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as i32
        };
        let n = 40;
        let mut clauses: Vec<Vec<i32>> = Vec::new();
        while clauses.len() < 150 {
            let c: Vec<i32> = (0..3).map(|_| (next() % n + 1) * if next() % 2 == 0 { 1 } else { -1 }).collect();
            if c[0].abs() != c[1].abs() && c[0].abs() != c[2].abs() && c[1].abs() != c[2].abs() {
                clauses.push(c);
            }
        }
        let mut outputs = Vec::new();
        for g in (n + 1)..=(n + 6) {
            let (a, b) = (next() % n + 1, next() % n + 1);
            if g % 2 == 0 {
                clauses.extend([vec![-g, a], vec![-g, b], vec![g, -a, -b]]); // g = a and b
            } else {
                clauses.extend([vec![-g, a], vec![g, -a]]); // g = a
            }
            outputs.push(g);
        }
        (clauses, outputs)
    }

    fn often(s: &mut Solver) {
        // no lucky shortcut: it would answer before any pass runs
        for (name, value) in [
            ("eliminateinit", 0), ("eliminateint", 10), ("probeinit", 0), ("probeint", 2), ("luckyearly", 0), ("luckylate", 0),
        ] {
            internal::set_option(s, name, value);
        }
    }

    #[test]
    fn an_eliminated_variable_is_refused_and_a_frozen_one_stays() {
        let (clauses, outputs) = removable_formula();
        let refs: Vec<&[i32]> = clauses.iter().map(|c| c.as_slice()).collect();
        // a plain solve first: elimination and substitution run, and remove
        // some of the gate outputs
        let mut s = solver_of(&refs);
        often(&mut s);
        let first = solve(&mut s);
        assert!(first == 10 || first == 20);
        let gone: Vec<i32> = outputs.iter().copied().filter(|&g| s.import_[g as usize].eliminated).collect();
        assert!(!gone.is_empty(), "the test needs a removed variable, or it checks nothing");
        for &g in &gone {
            assert!(freeze(&mut s, g).is_err(), "variable {} is gone and cannot be frozen", g);
            assert!(assume(&mut s, g).is_err(), "nor assumed");
        }
        assert!(s.assume.list.is_empty());
        // frozen before the same solve, the same variables stay, and can be
        // assumed afterwards
        let mut t = solver_of(&refs);
        often(&mut t);
        for &g in &outputs {
            freeze(&mut t, g).unwrap();
        }
        assert_eq!(solve(&mut t), first);
        for &g in &outputs {
            assert!(!t.import_[g as usize].eliminated, "frozen variable {} was removed", g);
        }
        for &g in &gone {
            let res = ask(&mut t, &[g]);
            assert!(res == 10 || res == 20);
            if res == 10 {
                assert_eq!(internal::value(&mut t, g), g);
            }
        }
    }

    #[test]
    fn a_call_limit_leaves_the_solver_usable() {
        // pigeons: 4 into 3 holes, unsatisfiable, needs conflicts
        let mut clauses: Vec<Vec<i32>> = Vec::new();
        let var = |p: i32, h: i32| p * 3 + h + 1;
        for p in 0..4 {
            clauses.push((0..3).map(|h| var(p, h)).collect());
        }
        for h in 0..3 {
            for p in 0..4 {
                for q in (p + 1)..4 {
                    clauses.push(vec![-var(p, h), -var(q, h)]);
                }
            }
        }
        // a switch variable 13 guards every pigeon clause, so the formula is
        // satisfiable and only fails under the assumption 13
        let guarded: Vec<Vec<i32>> = clauses
            .iter()
            .map(|c| if c.len() == 3 { let mut d = c.clone(); d.push(-13); d } else { c.clone() })
            .collect();
        let refs: Vec<&[i32]> = guarded.iter().map(|c| c.as_slice()).collect();
        let mut s = solver_of(&refs);
        limit(&mut s, Some(0), None);
        assert_eq!(ask(&mut s, &[13]), 0, "no conflict allowed: unknown");
        assert!(!s.assume.exhausted);
        assert_eq!(ask(&mut s, &[13]), 20);
        assert_eq!(core(&s, &[13]), vec![13]);
        assert_eq!(ask(&mut s, &[-13]), 10);
    }

    #[test]
    fn the_runs_limit_holds_across_solves() {
        let mut s = solver_of(EXAMPLE);
        internal::set_decision_limit(&mut s, 0);
        // no decision is allowed in the whole run; an assumption is not one
        assert_eq!(ask(&mut s, &[1, 2]), 0);
        assert!(s.assume.exhausted);
        assert_eq!(ask(&mut s, &[1]), 0);
        assert_eq!(s.assume.unknown, 2);
    }

    #[test]
    fn a_stop_stays_for_the_run() {
        let mut s = solver_of(EXAMPLE);
        assert_eq!(ask(&mut s, &[1]), 10);
        internal::terminate(&mut s);
        assert_eq!(ask(&mut s, &[1]), 0);
        assert_eq!(ask(&mut s, &[]), 0);
    }

    #[test]
    fn the_policy_refuses_assumptions() {
        let mut s = solver_of(EXAMPLE);
        s.policy.on = true;
        assert!(assume(&mut s, 1).is_err());
    }

    #[test]
    fn a_solver_with_an_engine_refuses_assumptions() {
        // the parity engine wanted (two XORs from the structure stage)
        let mut s = solver_of(EXAMPLE);
        s.gauss.xors = vec![(vec![1, 2], 0), (vec![2, 3], 1)];
        assert!(assume(&mut s, 1).unwrap_err().contains("parity engine"));
        s.gauss.xors.clear();
        s.gauss.on = true;
        assert!(assume(&mut s, 1).unwrap_err().contains("parity engine"));
        s.gauss.on = false;
        // the count engine running
        s.card.on = true;
        assert!(assume(&mut s, 1).unwrap_err().contains("count engine"));
        s.card.on = false;
        // a model of the free-choice walk
        s.freewalk.model = Some(vec![false; 7]);
        assert!(assume(&mut s, 1).unwrap_err().contains("free-choice walk"));
        s.freewalk.model = None;
        // freezing alone is always allowed, and so is assuming once they are gone
        assert!(s.assume.list.is_empty());
        freeze(&mut s, 1).unwrap();
        assert_eq!(ask(&mut s, &[1, 2]), 10);
    }

    #[test]
    fn parse_reads_clauses_then_questions() {
        let f = parse_inccnf(b"c hi\np inccnf\n1 -2 0\n2 3\n0\na 1 2 0\na 0\na -3\n 0\n").unwrap();
        assert_eq!(f.clauses, vec![1, -2, 0, 2, 3, 0]);
        assert_eq!(f.questions, vec![vec![1, 2], vec![], vec![-3]]);
        assert_eq!(f.max_var, 3);
        assert!(parse_inccnf(b"p cnf 1 1\n1 0\n").is_err());
        assert!(parse_inccnf(b"1 0\n").is_err());
        assert!(parse_inccnf(b"p inccnf\n1 2\n").is_err());
        assert!(parse_inccnf(b"p inccnf\na 1\n").is_err());
        assert!(parse_inccnf(b"p inccnf\na 1 0\n2 0\n").is_err());
        assert!(parse_inccnf(b"p inccnf\n1 x 0\n").is_err());
        assert!(parse_inccnf(b"p inccnf\n1 a 2 0\n").is_err());
        assert!(parse_inccnf(b"p inccnf\n99999999999 0\n").is_err());
        let g = parse_inccnf(b"p inccnf\n").unwrap();
        assert!(g.clauses.is_empty() && g.questions.is_empty());
    }
}
