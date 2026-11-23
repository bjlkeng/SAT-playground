//! Not in kissat (bead SAT-playground-d96.14.1): what the search decides on,
//! by role.
//!
//! A reading made outside the solver gives each variable of the formula a
//! role: a real choice, a bit of a number, a test computed from numbers, a
//! helper of the encoding, and so on.  With `rolecount` on, the solver reads
//! those roles from the file named by `SAT_ROLE_FILE` and counts, without
//! touching the search:
//!
//!  - the decisions by the role of the decided variable, in both search
//!    modes, of them those that try the value true, and of them the first
//!    decision after each restart;
//!  - what stands on the trail at a conflict (one conflict in
//!    SAMPLE_CONFLICTS is a sample, fewer on a very long trail): the
//!    decisions of the first TOP_LEVELS levels by role, and the variables
//!    of each role that are set, by a decision or by propagation;
//!  - at checkpoints (the first decision, then 1,000, 10,000, ... conflicts,
//!    and the end) what became of each role's
//!    variables: still held under its own number, fixed, eliminated, or
//!    replaced by an equal variable.
//!
//! Following the solver's renaming.  When the solver finds two variables
//! equal it keeps one and drops the other (src/substitute.rs), and variable
//! elimination removes a variable that two clauses make equal to another in
//! the same way (src/eliminate.rs, src/fastel.rs).  A decision on
//! the kept variable is then a decision on the dropped one too, so every
//! count is kept twice: `own` goes by the decided variable's own role, and
//! `stands` counts the decision for every role the variable stands for, its
//! own and those of the variables replaced by it.  A decision can stand for
//! several roles, so the `stands` numbers of a line need not add up to the
//! decisions.  A replaced variable can be equal to the kept one or to its
//! opposite, and the count of values goes by that: a decision that sets the
//! kept variable true counts as "true" for a role when it makes one of that
//! role's variables true.
//!
//! The role file: one line a variable, `<variable> <role>` or
//! `<first>-<last> <role>`, DIMACS numbers, a role a short word.  Lines
//! starting with `c` or `#` are comments.  A variable of the formula the
//! file does not list has the role `unlisted`, a variable the solver made
//! itself the role `new`.  The solver gives its own variables the numbers
//! after the last one that a clause uses, so a number the formula declares
//! and never uses can become a variable of the solver's: such a number keeps
//! the file's role only as a variable in no clause.
//!
//! Everything here only reads the solver: no tick, no statistic and no
//! decision depends on it (tests/roles.rs holds the search to the one without
//! the option, counter for counter).  With the option off nothing runs beyond
//! one test of a flag at each call site, and the environment is never read.
//! The output is kept in memory and printed with the statistics at the end.
//!
//! The role file is an input of the run.  It is read before the run opens
//! any file to write (`reserve`), and a role file that is one of the run's
//! own files (the formula, the proof, an output file, the policy's log) is
//! refused, so no run can overwrite it.  The statistics are also printed
//! from the signal handler: the saved lines are only ever changed with the
//! handled signals blocked, and from the handler nothing else is read.

use crate::internal::{Solver, DECISION_REASON};

/// A variable of the formula that the file does not list.
const UNLISTED: usize = 0;
/// A variable the solver made itself (it is not in the formula).
const NEW: usize = 1;
/// Roles in all, the two above among them: a role is a bit of a `u32`.
pub const MAX_ROLES: usize = 32;
/// The longest role name.
const MAX_NAME: usize = 24;
/// The decisions of this many levels, from the first, are the trail's top.
const TOP_LEVELS: u32 = 10;
/// One trail sample in this many conflicts, or fewer on a long trail: then
/// one in a `SAMPLE_TRAIL_SHARE`th of the trail's length.
const SAMPLE_CONFLICTS: u64 = 128;
const SAMPLE_TRAIL_SHARE: u64 = 1024;
/// A checkpoint at the first conflict after each of these conflict counts.
const CHECKPOINTS: [u64; 5] = [1_000, 10_000, 100_000, 1_000_000, 10_000_000];

/// One count by role: by the variable's own role, and for every role it
/// stands for.
#[derive(Default, Clone)]
struct Count {
    own: Vec<u64>,
    stands: Vec<u64>,
}

impl Count {
    fn new(roles: usize) -> Count {
        Count { own: vec![0; roles], stands: vec![0; roles] }
    }

    #[inline]
    fn add(&mut self, own: usize, mask: u32) {
        self.own[own] += 1;
        self.add_stands(mask);
    }

    #[inline]
    fn add_stands(&mut self, mask: u32) {
        let mut bits = mask;
        while bits != 0 {
            self.stands[bits.trailing_zeros() as usize] += 1;
            bits &= bits - 1;
        }
    }
}

/// The counts of one search mode (0 focused, 1 stable).
#[derive(Default, Clone)]
struct Mode {
    /// Decisions.
    decide: Count,
    /// Of them, those that set a variable of the role true.
    truth: Count,
    /// Of them all, the first decision after a restart.
    first: Count,
    /// Trail samples: the decisions of the first TOP_LEVELS levels.
    top: Count,
    /// Trail samples: the variables that are set above the root level.
    set: Count,
    /// Of them, the decisions.
    dec: Count,
    conflicts: u64,
    samples: u64,
    /// Decision levels, summed over the samples.
    levels: u64,
}

impl Mode {
    fn new(roles: usize) -> Mode {
        Mode {
            decide: Count::new(roles),
            truth: Count::new(roles),
            first: Count::new(roles),
            top: Count::new(roles),
            set: Count::new(roles),
            dec: Count::new(roles),
            conflicts: 0,
            samples: 0,
            levels: 0,
        }
    }
}

#[derive(Default)]
pub struct Roles {
    /// The count runs (`rolecount` and a role file read).
    pub on: bool,
    path: String,
    /// The role file's text, read by `reserve` and used up by `install`.
    text: Option<String>,
    /// The exit lines have been printed (or are being printed).
    reported: bool,
    /// Role names; a role is its index here.
    names: Vec<String>,
    /// The formula's variables are the DIMACS numbers 1 to this.
    original: usize,
    /// Variables the file lists.
    listed: usize,
    /// By DIMACS variable: its role.
    role: Vec<u8>,
    /// By DIMACS variable: the roles it stands for, a bit each: its own, and
    /// those of every variable replaced by it that is equal to it (`same`)
    /// or to its opposite (`flipped`).
    same: Vec<u32>,
    flipped: Vec<u32>,
    /// By DIMACS variable: the DIMACS literal that replaced it (0: none did).
    merged: Vec<i32>,
    /// Replacements by substitution, and by an elimination of a variable
    /// that was equal to another.
    merges: u64,
    equal_eliminations: u64,
    /// By role: variables the file lists that no clause uses and whose
    /// numbers the solver may take for variables of its own.
    unused: Vec<u64>,
    /// From the role file's reading to the roles' installation: `used` and
    /// `added` follow what the parser hands on and what the solver is given.
    pub tracking: bool,
    /// The largest variable in a clause of the formula.
    used: usize,
    /// The largest variable in a clause given to the solver before the
    /// search: past `used` these are the structure passes' helper variables.
    added: usize,
    modes: [Mode; 2],
    /// The next decision is the first after a restart.
    after_restart: bool,
    /// The first decision has been seen (the checkpoint `start` is taken).
    started: bool,
    /// Conflicts already given to a mode.
    flushed: u64,
    /// Conflicts counted before the roles were installed (a formula can be
    /// refuted while it is loaded): they are in the run's total and in no
    /// mode's.
    before: u64,
    next_sample: u64,
    next_checkpoint: usize,
    checkpoints: u64,
    lines: Vec<String>,
}

// ---------------------------------------------------------------------------
// the role file
// ---------------------------------------------------------------------------

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// (role names, the role of each variable from 0 to `original`, how many
/// variables are listed).
fn parse_roles(text: &str, original: usize) -> Result<(Vec<String>, Vec<u8>, usize), String> {
    let mut names: Vec<String> = vec!["unlisted".to_string(), "new".to_string()];
    let mut role: Vec<u8> = vec![UNLISTED as u8; original + 1];
    let mut seen: Vec<bool> = vec![false; original + 1];
    let mut listed = 0usize;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line == "c" || line.starts_with("c ") || line.starts_with("c\t") {
            continue;
        }
        let at = |what: String| format!("line {}: {}", n + 1, what);
        let mut toks = line.split_whitespace();
        let (Some(range), Some(name), None) = (toks.next(), toks.next(), toks.next()) else {
            return Err(at("expected '<variable> <role>' or '<first>-<last> <role>'".to_string()));
        };
        let (first, last) = match range.split_once('-') {
            Some((a, b)) => (a, b),
            None => (range, range),
        };
        let number = |t: &str| -> Result<usize, String> {
            // digits only: `parse` alone would take a leading plus sign
            if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
                return Err(at(format!("'{}' is not a variable number", t)));
            }
            t.parse::<usize>().map_err(|_| at(format!("'{}' is not a variable number", t)))
        };
        let (first, last) = (number(first)?, number(last)?);
        if first == 0 || last < first {
            return Err(at(format!("'{}' is not a variable or a range of variables", range)));
        }
        if last > original {
            return Err(at(format!("variable {} is not in the formula (its variables are 1 to {})", last, original)));
        }
        if !valid_name(name) {
            return Err(at(format!(
                "'{}' is not a role name (letters, digits, '_' and '-', at most {} characters)",
                name, MAX_NAME
            )));
        }
        if name == "unlisted" || name == "new" {
            return Err(at(format!("the role name '{}' is taken by the count itself", name)));
        }
        let id = match names.iter().position(|x| x == name) {
            Some(id) => id,
            None => {
                if names.len() == MAX_ROLES {
                    return Err(at(format!("more than {} roles", MAX_ROLES - 2)));
                }
                names.push(name.to_string());
                names.len() - 1
            }
        };
        for v in first..=last {
            if seen[v] {
                return Err(at(format!("variable {} is listed twice", v)));
            }
            seen[v] = true;
            role[v] = id as u8;
        }
        listed += last - first + 1;
    }
    Ok((names, role, listed))
}

/// Read the role file's text, before the run opens any file to write.
/// `taken` names the files the run owns (the formula, the proof, an output
/// file, the structure pass's files beside the proof): a role file that is
/// one of them would be overwritten, or is not a role file, and is refused.
/// Returns the role file's path for the files opened later (the policy's
/// log), which may not be it either; None with the count off.
pub fn reserve(solver: &mut Solver, taken: &[(&str, &str)]) -> Result<Option<String>, String> {
    if solver.options.rolecount == 0 {
        return Ok(None);
    }
    let path = match std::env::var("SAT_ROLE_FILE") {
        Ok(p) if !p.trim().is_empty() => p,
        _ => return Err("rolecount is on but SAT_ROLE_FILE names no role file".to_string()),
    };
    for (what, other) in taken {
        if crate::policy_log::same_path(&path, other) {
            return Err(format!("SAT_ROLE_FILE='{}' is the {} file '{}'", path, what, other));
        }
    }
    if let Some(stream) = crate::policy_log::standard_stream_behind(&path) {
        return Err(format!("SAT_ROLE_FILE='{}' is the file behind this process's {}", path, stream));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("role file '{}': {}", path, e))?;
    solver.roles.text = Some(text);
    solver.roles.path = path.clone();
    solver.roles.tracking = true;
    Ok(Some(path))
}

/// A literal of the formula, as the parser hands it on (`structure::sink`).
/// The largest variable seen is the last one the formula uses.
#[inline]
pub fn used(solver: &mut Solver, elit: i32) {
    let v = elit.unsigned_abs() as usize;
    if v > solver.roles.used {
        solver.roles.used = v;
    }
}

/// A literal of a clause given to the solver before the search
/// (`internal::add`): the formula's, and after them the clauses the
/// structure passes derive.  Those passes number their helper variables
/// from the formula's last used variable on, so the numbers past `used` up
/// to the largest one seen here are helpers: variables of the solver's own,
/// whatever the formula declares.
#[inline]
pub fn added(solver: &mut Solver, elit: i32) {
    let v = elit.unsigned_abs() as usize;
    if v > solver.roles.added {
        solver.roles.added = v;
    }
}

/// Take the roles from the text `reserve` read, after the formula is in the
/// solver.  `max_var` is the formula's variable count.  An error is a role
/// file that cannot be used: the run must not go on as if it counted.
pub fn install(solver: &mut Solver, max_var: i32) -> Result<(), String> {
    if solver.options.rolecount == 0 {
        return Ok(());
    }
    let Some(text) = solver.roles.text.take() else {
        return Err("rolecount is on but no role file was read".to_string());
    };
    let path = std::mem::take(&mut solver.roles.path);
    // the formula's variables: its declared count, or more when the relaxed
    // parser took a variable past it
    let original = (max_var.max(0) as usize).max(solver.roles.used);
    let (names, mut role, listed) = parse_roles(&text, original).map_err(|e| format!("role file '{}': {}", path, e))?;
    drop(text);
    // A variable of the solver's own can have a number that the formula
    // declares and never uses, in two ways.  A structure pass numbers its
    // helper variables from the formula's last used variable on: the numbers
    // past `used` up to `added`.  And the solver's own techniques take the
    // next number after every one the solver has registered: any number
    // from `registered` on, later (or one that carries the import's flag
    // already).  The file's variable of such a number is one in no clause;
    // the number itself is the solver's.  A number the solver has registered
    // and neither of the two took is the formula's: with tumble off the
    // solver registers every declared variable, used or not.
    solver.roles.tracking = false;
    let (used, added) = (solver.roles.used, solver.roles.added);
    let registered = solver.import_.len().max(1);
    let mut unused = vec![0u64; names.len()];
    for e in 1..role.len() {
        let taken = e >= registered || (e > used && e <= added) || solver.import_[e].extension;
        if taken {
            unused[role[e] as usize] += 1;
            role[e] = NEW as u8;
        }
    }
    role.truncate(registered);
    if role.len() < registered {
        role.resize(registered, NEW as u8);
    }
    let r = &mut solver.roles;
    r.unused = unused;
    r.same = role.iter().map(|&x| 1u32 << x).collect();
    r.flipped = vec![0; role.len()];
    r.merged = vec![0; role.len()];
    r.role = role;
    r.modes = [Mode::new(names.len()), Mode::new(names.len())];
    r.names = names;
    r.original = original;
    r.listed = listed;
    r.path = path;
    r.flushed = solver.statistics.conflicts;
    r.before = solver.statistics.conflicts;
    r.on = true;
    let (roles, listed) = (solver.roles.names.len() - 2, solver.roles.listed);
    crate::print::message(
        solver,
        format!("roles: counting decisions by role, {} roles over {} of {} variables", roles, listed, original),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// the counts
// ---------------------------------------------------------------------------

impl Roles {
    /// Room for the DIMACS variables below `n`: those past the formula's are
    /// the solver's own.
    fn grow(&mut self, n: usize) {
        if self.role.len() < n {
            self.role.resize(n, NEW as u8);
            self.same.resize(n, 1 << NEW);
            self.flipped.resize(n, 0);
            self.merged.resize(n, 0);
        }
    }

    /// The role of a DIMACS variable, the roles it stands for as itself and
    /// those it stands for as its opposite.
    #[inline]
    fn of(&self, e: usize) -> (usize, u32, u32) {
        match self.role.get(e) {
            Some(&own) => (own as usize, self.same[e], self.flipped[e]),
            None => (NEW, 1 << NEW, 0),
        }
    }
}

/// The role of an internal variable and the roles it stands for, as itself
/// and as its opposite; the last is true when the internal variable is the
/// opposite of its DIMACS variable.
#[inline]
fn look(solver: &Solver, idx: u32) -> (usize, u32, u32, bool) {
    let x = solver.export_.get(idx as usize).copied().unwrap_or(0);
    let (own, same, flipped) = solver.roles.of(x.unsigned_abs() as usize);
    (own, same, flipped, x < 0)
}

/// Give the conflicts since the last call to the mode the search is in.
/// Called before every change of mode, so each conflict goes to its own mode.
fn flush(solver: &mut Solver) {
    let now = solver.statistics.conflicts;
    let mode = solver.stable as usize;
    let r = &mut solver.roles;
    r.modes[mode].conflicts += now.saturating_sub(r.flushed);
    r.flushed = now;
}

/// A decision on `idx` with the value `value` (1 true, -1 false); not one of
/// the warm-up's.
pub fn decided(solver: &mut Solver, idx: u32, value: i32) {
    if !solver.roles.started {
        solver.roles.started = true;
        checkpoint(solver, "start");
    }
    let (own, same, flipped, negated) = look(solver, idx);
    let mask = same | flipped;
    // the value the decision gives the DIMACS variable
    let truth = (value > 0) != negated;
    let mode = solver.stable as usize;
    let r = &mut solver.roles;
    let m = &mut r.modes[mode];
    m.decide.add(own, mask);
    // true for its own role when it is set true, and for every role one of
    // whose variables it makes true
    if truth {
        m.truth.own[own] += 1;
    }
    m.truth.add_stands(if truth { same } else { flipped });
    if r.after_restart {
        r.after_restart = false;
        m.first.add(own, mask);
    }
}

/// The search mode is about to change.
pub fn switching(solver: &mut Solver) {
    flush(solver);
}

/// A restart: the next decision is the first after it.
pub fn restarting(solver: &mut Solver) {
    solver.roles.after_restart = true;
}

/// A conflict of the search, before it is analysed: the trail is whole.
pub fn conflicting(solver: &mut Solver) {
    let conflicts = solver.statistics.conflicts;
    if conflicts >= solver.roles.next_sample {
        // a long trail is walked less often, so a sample costs a conflict
        // about a thousandth of the trail at most
        let step = SAMPLE_CONFLICTS.max(solver.trail.len() as u64 / SAMPLE_TRAIL_SHARE);
        solver.roles.next_sample = conflicts + step;
        sample(solver);
    }
    let next = solver.roles.next_checkpoint;
    if next < CHECKPOINTS.len() && conflicts >= CHECKPOINTS[next] {
        // one checkpoint, named by the last mark that was passed
        let mut k = next;
        while k + 1 < CHECKPOINTS.len() && conflicts >= CHECKPOINTS[k + 1] {
            k += 1;
        }
        solver.roles.next_checkpoint = k + 1;
        checkpoint(solver, &CHECKPOINTS[k].to_string());
    }
}

/// What stands on the trail now.
fn sample(solver: &mut Solver) {
    let mode = solver.stable as usize;
    let levels = solver.level;
    // taken out for the walk: the lookups borrow the solver
    let mut top = std::mem::take(&mut solver.roles.modes[mode].top);
    let mut set = std::mem::take(&mut solver.roles.modes[mode].set);
    let mut dec = std::mem::take(&mut solver.roles.modes[mode].dec);
    for level in 1..=levels.min(TOP_LEVELS) {
        let Some(frame) = solver.frames.get(level as usize) else { break };
        if frame.decision == u32::MAX {
            continue; // a level without a decision (src/assume.rs)
        }
        let (own, same, flipped, _) = look(solver, frame.decision >> 1);
        top.add(own, same | flipped);
    }
    for &lit in solver.trail.iter() {
        let idx = lit >> 1;
        let Some(a) = solver.assigned.get(idx as usize) else { continue };
        if a.level == 0 {
            continue; // fixed for good: not a part of what the search set
        }
        let (own, same, flipped, _) = look(solver, idx);
        set.add(own, same | flipped);
        if a.reason == DECISION_REASON {
            dec.add(own, same | flipped);
        }
    }
    let m = &mut solver.roles.modes[mode];
    m.top = top;
    m.set = set;
    m.dec = dec;
    m.samples += 1;
    m.levels += levels as u64;
}

/// `idx` is being replaced by the equal literal `other` (src/substitute.rs).
pub fn merged(solver: &mut Solver, idx: u32, other: u32) {
    if note_equal(solver, idx, other) {
        solver.roles.merges += 1;
    }
}

/// `idx` is about to be eliminated, its clauses still in place.  When two
/// of them, (-x | r) and (x | -r), make it equal to a literal r, the
/// elimination is a replacement by r: every other clause of x comes back
/// with r in its place.
pub fn eliminating(solver: &mut Solver, idx: u32) {
    let lit = idx << 1;
    let not_lit = lit | 1;
    let pos = solver.watches[lit as usize];
    let neg = solver.watches[not_lit as usize];
    // x = r needs the two clauses (x | -r) and (-x | r).  The other literals
    // of x's two-literal clauses are sorted once, and each two-literal
    // clause of -x is looked up in them: the work is in the order of the
    // clauses of x, however many there are.
    let mut with_x: Vec<u32> = solver.vectors.stack[pos.begin..pos.end]
        .iter()
        .filter(|&&w| crate::watch::watch_is_binary(w))
        .map(|&w| crate::watch::watch_lit(w))
        .collect();
    with_x.sort_unstable();
    let mut found = None;
    for &w in &solver.vectors.stack[neg.begin..neg.end] {
        if !crate::watch::watch_is_binary(w) {
            continue;
        }
        let r = crate::watch::watch_lit(w);
        // a literal that has a value is no partner: its clauses are on their way out
        if r >> 1 == idx || solver.values.get(r as usize).copied().unwrap_or(1) != 0 {
            continue;
        }
        if with_x.binary_search(&(r ^ 1)).is_ok() {
            found = Some(r);
            break;
        }
    }
    if let Some(r) = found {
        if note_equal(solver, idx, r) {
            solver.roles.equal_eliminations += 1;
        }
    }
}

/// Note that the variable `idx` is equal to the literal `other` and goes;
/// false when there is nothing to note.
fn note_equal(solver: &mut Solver, idx: u32, other: u32) -> bool {
    let (Some(&a), Some(&b)) = (solver.export_.get(idx as usize), solver.export_.get((other >> 1) as usize)) else {
        return false;
    };
    if a == 0 || b == 0 {
        return false;
    }
    // the DIMACS literal of the internal literal `other`
    let b = if other & 1 != 0 { -b } else { b };
    let (ea, eb) = (a.unsigned_abs() as usize, b.unsigned_abs() as usize);
    if ea == eb {
        return false;
    }
    let r = &mut solver.roles;
    r.grow(ea.max(eb) + 1);
    // the DIMACS variable `ea` is equal to this DIMACS literal
    let to = if a > 0 { b } else { -b };
    r.merged[ea] = to;
    // `ea` is equal to the variable `eb` itself when `to` is positive
    let (same, flipped) = (r.same[ea], r.flipped[ea]);
    if to > 0 {
        r.same[eb] |= same;
        r.flipped[eb] |= flipped;
    } else {
        r.same[eb] |= flipped;
        r.flipped[eb] |= same;
    }
    true
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Absent,
    Held,
    Fixed,
    Eliminated,
}

/// What a DIMACS variable is in the solver now (a replaced one reads as
/// eliminated here; `checkpoint` follows it to its replacement first).
fn state(solver: &Solver, e: usize) -> State {
    let Some(imp) = solver.import_.get(e) else { return State::Absent };
    if !imp.imported {
        return State::Absent;
    }
    if imp.eliminated {
        return State::Eliminated;
    }
    match solver.flags.get((imp.lit >> 1) as usize) {
        Some(f) if f.fixed() => State::Fixed,
        Some(f) if f.active() => State::Held,
        _ => State::Absent, // imported and in no clause
    }
}

/// The counts so far and what became of each role's variables, as lines for
/// the report.
fn checkpoint(solver: &mut Solver, label: &str) {
    flush(solver);
    let n = solver.import_.len();
    solver.roles.grow(n);
    let roles = solver.roles.names.len();
    #[derive(Default, Clone)]
    struct Hold {
        vars: u64,
        absent: u64,
        held: u64,
        fixed: u64,
        eliminated: u64,
        merged_held: u64,
        merged_fixed: u64,
        merged_eliminated: u64,
        /// Held variables that stand for the role (its own or replaced ones).
        carriers: u64,
        /// Its replaced variables whose replacement is held, by that one's role.
        into: Vec<u64>,
    }
    let mut hold: Vec<Hold> = vec![Hold { into: vec![0; roles], ..Hold::default() }; roles];
    for (id, &n) in solver.roles.unused.iter().enumerate() {
        hold[id].vars += n;
        hold[id].absent += n;
    }
    // The variable each replaced one stands behind now.  A chain of
    // replacements can be as long as the formula (x1 = x2 = x3 ...), so each
    // chain is walked once and every variable on it gets the chain's end.
    let mut root: Vec<u32> = vec![0; n];
    {
        let r = &solver.roles;
        let mut path: Vec<usize> = Vec::new();
        for e in 1..n {
            if r.merged[e] == 0 || root[e] != 0 {
                continue;
            }
            path.clear();
            let mut cur = e;
            // on to the end of the chain, or to a variable whose end is known
            while r.merged[cur] != 0 && root[cur] == 0 && path.len() < n {
                path.push(cur);
                cur = r.merged[cur].unsigned_abs() as usize;
                if cur >= n {
                    break;
                }
            }
            let end = if cur < n && root[cur] != 0 { root[cur] } else { cur as u32 };
            for &p in &path {
                root[p] = end;
            }
        }
    }
    {
        let r = &solver.roles;
        for e in 1..n {
            let own = r.role[e] as usize;
            let h = &mut hold[own];
            h.vars += 1;
            if r.merged[e] == 0 {
                match state(solver, e) {
                    State::Absent => h.absent += 1,
                    State::Fixed => h.fixed += 1,
                    State::Eliminated => h.eliminated += 1,
                    State::Held => {
                        h.held += 1;
                        let mut bits = r.same[e] | r.flipped[e];
                        while bits != 0 {
                            hold[bits.trailing_zeros() as usize].carriers += 1;
                            bits &= bits - 1;
                        }
                    }
                }
                continue;
            }
            // the variable that stands for it now, at the end of its replacements
            let cur = root[e] as usize;
            match state(solver, cur) {
                State::Held => {
                    h.merged_held += 1;
                    let (to, _, _) = r.of(cur);
                    h.into[to] += 1;
                }
                State::Fixed => h.merged_fixed += 1,
                State::Eliminated | State::Absent => h.merged_eliminated += 1,
            }
        }
    }
    let conflicts = solver.statistics.conflicts;
    let head = format!("at={} conflicts={}", label, conflicts);
    let mut lines: Vec<String> = Vec::new();
    {
        let r = &solver.roles;
        for (id, h) in hold.iter().enumerate() {
            if h.vars == 0 && h.carriers == 0 {
                continue;
            }
            let into: Vec<String> =
                h.into.iter().enumerate().filter(|(_, &c)| c != 0).map(|(to, c)| format!("{}:{}", r.names[to], c)).collect();
            lines.push(format!(
                "c roles hold {} role={} vars={} absent={} held={} fixed={} eliminated={} merged_held={} merged_fixed={} merged_eliminated={} carriers={} into={}",
                head,
                r.names[id],
                h.vars,
                h.absent,
                h.held,
                h.fixed,
                h.eliminated,
                h.merged_held,
                h.merged_fixed,
                h.merged_eliminated,
                h.carriers,
                if into.is_empty() { "-".to_string() } else { into.join(",") }
            ));
        }
        let st = &solver.statistics;
        let search = [
            (st.focused_decisions, st.focused_propagations, st.focused_ticks, st.focused_restarts),
            (st.stable_decisions, st.stable_propagations, st.stable_ticks, st.stable_restarts),
        ];
        for (mode, name) in ["focused", "stable"].iter().enumerate() {
            let m = &r.modes[mode];
            let (decisions, propagations, ticks, restarts) = search[mode];
            lines.push(format!(
                "c roles search {} mode={} mode_conflicts={} decisions={} propagations={} ticks={} restarts={} samples={} levels={}",
                head, name, m.conflicts, decisions, propagations, ticks, restarts, m.samples, m.levels
            ));
            for id in 0..roles {
                let all = [&m.decide, &m.truth, &m.first, &m.top, &m.set, &m.dec];
                if all.iter().all(|c| c.own[id] == 0 && c.stands[id] == 0) {
                    continue;
                }
                lines.push(format!(
                    "c roles count {} mode={} role={} decide_own={} decide_stands={} true_own={} true_stands={} first_own={} first_stands={} top_own={} top_stands={} set_own={} set_stands={} dec_own={} dec_stands={}",
                    head,
                    name,
                    r.names[id],
                    m.decide.own[id],
                    m.decide.stands[id],
                    m.truth.own[id],
                    m.truth.stands[id],
                    m.first.own[id],
                    m.first.stands[id],
                    m.top.own[id],
                    m.top.stands[id],
                    m.set.own[id],
                    m.set.stands[id],
                    m.dec.own[id],
                    m.dec.stands[id]
                ));
            }
        }
    }
    // the signal handler prints these lines: they change with its signals blocked
    let _guard = crate::policy_log::SignalGuard::block();
    let r = &mut solver.roles;
    r.checkpoints += 1;
    r.lines.append(&mut lines);
}

/// The exit lines; nothing when the count is off.  `killed` is a run ended
/// by a signal: the solver may have been stopped in the middle of a change,
/// so nothing of it is read then and the last checkpoint stands.
pub fn report(solver: &mut Solver, killed: bool) {
    if !solver.roles.on || solver.roles.reported {
        return; // off, or a signal during the printing below: once is enough
    }
    if !killed {
        checkpoint(solver, "end");
    }
    solver.roles.reported = true;
    let r = &solver.roles;
    println!(
        "c roles file path={} roles={} listed={} original={} merges={} equal_eliminations={} conflicts_before={} checkpoints={} top_levels={} sample_conflicts={} ended={}",
        r.path,
        r.names.len() - 2,
        r.listed,
        r.original,
        r.merges,
        r.equal_eliminations,
        r.before,
        r.checkpoints,
        TOP_LEVELS,
        SAMPLE_CONFLICTS,
        if killed { "signal" } else { "exit" }
    );
    for line in &r.lines {
        println!("{}", line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_role_file_is_read_with_ranges_and_comments() {
        let text = "c a comment\n# another\n\n1 choice\n2-4 helper\n 7   choice \n";
        let (names, role, listed) = parse_roles(text, 8).unwrap();
        assert_eq!(names, ["unlisted", "new", "choice", "helper"]);
        assert_eq!(role, [0, 2, 3, 3, 3, 0, 0, 2, 0]);
        assert_eq!(listed, 5);
    }

    #[test]
    fn a_bad_role_file_is_refused_with_its_line() {
        for (text, what) in [
            ("1 choice\n1 helper\n", "listed twice"),
            ("2-3 a\n3 b\n", "listed twice"),
            ("9 choice\n", "not in the formula"),
            ("0 choice\n", "not a variable"),
            ("4-2 choice\n", "not a variable"),
            ("-3 choice\n", "not a variable number"),
            ("+3 choice\n", "not a variable number"),
            ("x choice\n", "not a variable number"),
            ("1\n", "expected"),
            ("1 choice extra\n", "expected"),
            ("1 new\n", "taken by the count"),
            ("1 unlisted\n", "taken by the count"),
            ("1 a.b\n", "not a role name"),
            ("1 abcdefghijklmnopqrstuvwxyz\n", "not a role name"),
        ] {
            let err = parse_roles(text, 8).unwrap_err();
            assert!(err.contains(what), "{:?}: {}", text, err);
            assert!(err.starts_with("line "), "{:?}: {}", text, err);
        }
        // thirty roles are taken, the thirty-first is one too many
        let many: String = (1..=31).map(|v| format!("{} r{}\n", v, v)).collect();
        assert!(parse_roles(&many, 40).unwrap_err().contains("more than 30 roles"));
        let thirty: String = (1..=30).map(|v| format!("{} r{}\n", v, v)).collect();
        assert_eq!(parse_roles(&thirty, 40).unwrap().0.len(), MAX_ROLES);
    }

    #[test]
    fn a_count_goes_to_the_own_role_and_to_every_role_stood_for() {
        let mut c = Count::new(4);
        c.add(2, 1 << 2);
        c.add(3, (1 << 3) | (1 << 2));
        assert_eq!(c.own, [0, 0, 1, 1]);
        assert_eq!(c.stands, [0, 0, 2, 1]);
    }

    #[test]
    fn variables_past_the_formula_are_the_solvers_own() {
        let mut r = Roles::default();
        r.role = vec![0, 2, 2];
        r.same = vec![1, 4, 4];
        r.flipped = vec![0, 0, 8];
        r.merged = vec![0, 0, 0];
        assert_eq!(r.of(2), (2, 4, 8));
        assert_eq!(r.of(5), (NEW, 1 << NEW, 0));
        r.grow(6);
        assert_eq!(r.role, [0, 2, 2, 1, 1, 1]);
        assert_eq!(r.same, [1, 4, 4, 2, 2, 2]);
        assert_eq!(r.flipped, [0, 0, 8, 0, 0, 0]);
        assert_eq!(r.of(5), (NEW, 1 << NEW, 0));
    }
}
