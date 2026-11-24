//! Not in kissat (bead SAT-playground-d96.14.9; an experiment, every option
//! off by default): nudges toward the formula's real unknowns.
//!
//! A reading made outside the solver says which variables are the real
//! unknowns of the problem (a choice of a pick-one group, a bit of a stored
//! number, an input) and which are bookkeeping computed from them.  The
//! solver picks what to guess on from the conflicts alone.  Each option here
//! leans that pick, or the value, or the local search, toward the real
//! unknowns, and leaves the conflict heuristics in charge:
//!
//!  - `guidebump=P`: in the stable mode a preferred variable in a conflict
//!    gains P percent of the usual score, not 100.
//!  - `guidecause=N`: after a conflict is analysed, the reasons of the
//!    variables in it are walked back, at most N reasons, and every preferred
//!    variable met is bumped with the conflict's own variables: the real
//!    unknowns the conflict came from get the credit, not only the
//!    bookkeeping next to it.
//!  - `guidefront`: in the focused mode the variables of a conflict go to the
//!    front of the queue as always, the preferred ones last, so they are in
//!    front of the others.
//!  - `guidewindow=W`: in the focused mode a decision takes a preferred
//!    variable when one is among the next W unassigned variables of the
//!    queue.
//!  - `guiderestart=K`: the first K decisions after a restart are preferred
//!    variables, the best ones by the mode's own order.  A restart then goes
//!    back to the root: kissat keeps the part of the trail that its next
//!    decision would only rebuild, and with a head start the next decisions
//!    are others, so the K preferred variables sit at the top of the tree.
//!  - `guiderandec`: the random decisions of kissat's random sequences are
//!    drawn from the preferred variables.
//!  - `guidewarm`: the warm-up before a local search sets the preferred
//!    variables first, so the bookkeeping gets its values from them by
//!    propagation.
//!  - `guidewalk=P`: the local search weighs a flip of a preferred variable
//!    at P percent.
//!  - `guidephase=1|2`: a decision on a variable with a preferred value
//!    tries that variable true (1) or false (2), in `guidephaseshare` percent
//!    of such decisions.
//!  - `guideinit`: at the first decision the preferred variables go to the
//!    front of the queue and get one bump of score.  Not before: the solver
//!    simplifies the formula first and replaces variables by equal ones, and
//!    the start belongs to the variables that are there when the search
//!    begins.
//!  - `guideprotect`: a preferred variable is not eliminated.
//!
//! The variables come from the role file of src/roles.rs (`SAT_ROLE_FILE`):
//! the preferred ones are those of the roles named in `SAT_GUIDE_ROLES`
//! (default `choice,number,free,control`), the ones with a preferred value
//! those of `SAT_GUIDE_VALUE_ROLES` (default `choice`).  A variable the
//! solver replaces by an equal one is followed as src/roles.rs follows it:
//! the kept variable then stands for the replaced one, with the value
//! turned round when the two are opposites.
//!
//! None of this can make an answer wrong: a decision, its value, the order
//! of a queue and a start of the local search are free choices of the
//! solver, and a variable that is kept is one elimination not done.  With
//! `guide` on and every dial off the search is the old one (tests/guide.rs).
//! The lists looked through for a pick are paid on the work clock.

use crate::internal::{Solver, DECISION_REASON, UNIT_REASON};
use crate::literal::{lit, INVALID_IDX};
use crate::queue::disconnected;

/// The variable stands for a preferred variable.
const PREFERRED: u8 = 1;
/// A variable with a preferred value is equal to it, or is its opposite.
const VALUE_SAME: u8 = 2;
const VALUE_FLIPPED: u8 = 4;

/// At most this many entries of the list are looked at for one pick.
const SCAN: usize = 4096;
/// Random draws for one random pick before the usual pick is taken.
const TRIES: u32 = 64;

#[derive(Default)]
pub struct Counts {
    /// Score bumps with the bonus, and conflicts whose queue moves were
    /// ordered.
    pub bumped: u64,
    pub fronted: u64,
    /// Decisions taken by the window, by the head start after a restart,
    /// from the list in a random sequence, and in a warm-up.
    pub window: u64,
    pub restart: u64,
    pub random: u64,
    pub random_missed: u64,
    pub warm: u64,
    /// Decisions on a variable with a preferred value, and those whose
    /// value was changed.
    pub valued: u64,
    pub turned: u64,
    /// Flip weights raised in the local search.
    pub walk: u64,
    /// Variables `guideinit` put in front at the first decision.
    pub started: u64,
    /// Preferred variables bumped as causes of a conflict, and the reasons
    /// walked for them.
    pub credited: u64,
    pub reasons: u64,
    /// List entries and queue links looked at (paid as ticks).
    pub looked: u64,
}

#[derive(Default)]
pub struct Guide {
    /// The nudges run (`guide`, and the roles installed).
    pub on: bool,
    /// By DIMACS variable: PREFERRED, VALUE_SAME, VALUE_FLIPPED.
    bits: Vec<u8>,
    /// The bits of a variable the solver makes itself after the start (the
    /// role `new`, which `SAT_GUIDE_ROLES` may name): such a variable has a
    /// number past `bits`.  The nudges that go by the list do not reach it,
    /// those that ask about a variable do.
    new_bits: u8,
    /// The DIMACS variables that stand for a preferred variable, in the
    /// order found.  One that is gone since (fixed, eliminated, replaced)
    /// stays listed and is skipped wherever the list is used.
    list: Vec<u32>,
    /// Where the next look through the list starts.
    cursor: usize,
    /// The roles of the preferred variables and of those with a preferred
    /// value, a bit a role.
    var_mask: u32,
    value_mask: u32,
    // the dials, from the options
    pub bump: f64,
    pub bump_on: bool,
    pub front: bool,
    pub window: u32,
    pub restart: u32,
    pub phase: i8,
    pub phase_share: u32,
    pub randec: bool,
    pub warm: bool,
    pub walk: f64,
    pub walk_on: bool,
    pub protect: bool,
    pub cause: u32,
    /// For the walk back through the reasons: by internal variable the stamp
    /// of the walk that saw it, and the walk's queue.
    seen: Vec<u32>,
    stamp: u32,
    queue: Vec<u32>,
    /// Decisions left of the head start after the last restart.
    prefix: u32,
    /// `guideinit` has not been done yet (it waits for the first decision).
    init_pending: bool,
    /// A restart is looking at what the next decision would be: nothing is
    /// decided and nothing is counted.
    pub peek: bool,
    /// The first decision of the search: 0 not seen, 1 a preferred
    /// variable, 2 another.
    pub first: u8,
    /// Where the warm-up's walk through the list stands.
    warm_cursor: usize,
    pub n: Counts,
}

fn names_of(var: &str, default: &str) -> Vec<String> {
    let text = std::env::var(var).unwrap_or_else(|_| default.to_string());
    text.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

/// The bits of a DIMACS variable from the roles it stands for now.
fn bits_now(solver: &Solver, e: usize) -> u8 {
    let (same, flipped) = crate::roles::masks(solver, e);
    let g = &solver.guide;
    let mut b = 0u8;
    if (same | flipped) & g.var_mask != 0 {
        b |= PREFERRED;
    }
    if same & g.value_mask != 0 {
        b |= VALUE_SAME;
    }
    if flipped & g.value_mask != 0 {
        b |= VALUE_FLIPPED;
    }
    b
}

#[inline]
fn bits_of(solver: &Solver, idx: u32) -> u8 {
    let x = solver.export_.get(idx as usize).copied().unwrap_or(0);
    solver.guide.bits.get(x.unsigned_abs() as usize).copied().unwrap_or(solver.guide.new_bits)
}

/// The internal variable stands for a preferred variable.
#[inline]
pub fn prefers(solver: &Solver, idx: u32) -> bool {
    bits_of(solver, idx) & PREFERRED != 0
}

/// The internal variable of a listed DIMACS variable when it can be decided
/// now: in the solver, active and without a value.
#[inline]
fn candidate(solver: &Solver, e: u32) -> Option<u32> {
    let imp = solver.import_.get(e as usize)?;
    if !imp.imported || imp.eliminated {
        return None;
    }
    let idx = imp.lit >> 1;
    if idx >= solver.vars {
        return None;
    }
    if !solver.flags[idx as usize].active() || solver.values[lit(idx) as usize] != 0 {
        return None;
    }
    Some(idx)
}

/// Read the dials and mark the preferred variables, after the roles are
/// installed and before the search.  An error is a setting that cannot be
/// used: the run must not go on as if it nudged.
pub fn install(solver: &mut Solver) -> Result<(), String> {
    if solver.options.guide == 0 {
        return Ok(());
    }
    if !solver.roles.follow {
        return Err("guide is on but no roles were read".to_string());
    }
    let mut missing: Vec<String> = Vec::new();
    let mut mask_of = |solver: &Solver, names: &[String]| -> u32 {
        let mut mask = 0u32;
        for name in names {
            match crate::roles::role_id(solver, name) {
                Some(id) => mask |= 1 << id,
                None => missing.push(name.clone()),
            }
        }
        mask
    };
    let var_names = names_of("SAT_GUIDE_ROLES", "choice,number,free,control");
    let value_names = names_of("SAT_GUIDE_VALUE_ROLES", "choice");
    let var_mask = mask_of(solver, &var_names);
    let value_mask = mask_of(solver, &value_names);
    let o = &solver.options;
    let (bump, walk) = (o.guidebump as f64 / 100.0, o.guidewalk as f64 / 100.0);
    let init = o.guideinit != 0;
    let g = &mut solver.guide;
    g.var_mask = var_mask;
    g.value_mask = value_mask;
    g.bump_on = o.guidebump != 0 && o.guidebump != 100;
    g.bump = if g.bump_on { bump } else { 1.0 };
    g.front = o.guidefront != 0;
    g.window = o.guidewindow as u32;
    g.restart = o.guiderestart as u32;
    g.phase = match o.guidephase {
        1 => 1,
        2 => -1,
        _ => 0,
    };
    g.phase_share = o.guidephaseshare as u32;
    g.randec = o.guiderandec != 0;
    g.warm = o.guidewarm != 0;
    g.walk_on = o.guidewalk != 0 && o.guidewalk != 100;
    g.walk = if g.walk_on { walk } else { 1.0 };
    g.protect = o.guideprotect != 0;
    g.cause = o.guidecause as u32;
    let known = crate::roles::known(solver);
    let mut bits = vec![0u8; known];
    let mut list: Vec<u32> = Vec::new();
    let mut valued = 0usize;
    for e in 1..known {
        let b = bits_now(solver, e);
        bits[e] = b;
        if b & PREFERRED != 0 {
            list.push(e as u32);
        }
        if b & (VALUE_SAME | VALUE_FLIPPED) != 0 {
            valued += 1;
        }
    }
    solver.guide.bits = bits;
    solver.guide.list = list;
    // a variable made later has the role `new`: the bits of that role
    let new_role = crate::roles::role_id(solver, "new").map_or(0u32, |id| 1 << id);
    solver.guide.new_bits = (if var_mask & new_role != 0 { PREFERRED } else { 0 }) | (if value_mask & new_role != 0 { VALUE_SAME } else { 0 });
    solver.guide.init_pending = init;
    solver.guide.on = true;
    let (n, roles, value_roles) = (solver.guide.list.len(), var_names.join(","), value_names.join(","));
    crate::print::message(
        solver,
        format!(
            "guide: {} preferred variables (roles {}), {} with a preferred value (roles {}){}",
            n,
            roles,
            valued,
            value_roles,
            if missing.is_empty() { String::new() } else { format!("; not in the role file: {}", missing.join(",")) }
        ),
    );
    Ok(())
}

/// The roles the DIMACS variable `e` stands for have changed (a variable
/// was replaced by it).
pub fn refresh(solver: &mut Solver, e: usize) {
    let b = bits_now(solver, e);
    let g = &mut solver.guide;
    if g.bits.len() <= e {
        let fill = g.new_bits;
        g.bits.resize(e + 1, fill);
    }
    let old = g.bits[e];
    g.bits[e] = b;
    if b & PREFERRED != 0 && old & PREFERRED == 0 {
        g.list.push(e as u32);
    }
}

/// A restart: the head start begins.
#[inline]
pub fn restarted(solver: &mut Solver) {
    solver.guide.prefix = solver.guide.restart;
}

/// The first decision of the search is about to be chosen: `guideinit`
/// puts the preferred variables that are in the solver now, the stand-ins
/// of replaced ones among them, at the front of the queue and gives each one
/// bump of score.  The first conflicts reorder them as they do any variable.
pub fn start(solver: &mut Solver) {
    solver.guide.init_pending = false;
    let mut looked = 0u64;
    for k in 0..solver.guide.list.len() {
        let e = solver.guide.list[k];
        looked += 1;
        if let Some(idx) = candidate(solver, e) {
            crate::inlinequeue::move_to_front(solver, idx);
            crate::bump::bump_variable(solver, idx);
            solver.guide.n.started += 1;
        }
    }
    solver.guide.n.looked += looked;
    solver.statistics.ticks += looked;
}

/// The first decision of the search is `idx` (noted for the exit line).
#[inline]
pub fn first_decision(solver: &mut Solver, idx: u32) {
    solver.guide.first = if prefers(solver, idx) { 1 } else { 2 };
}

/// `guideinit` waits for the first decision.
#[inline]
pub fn init_pending(solver: &Solver) -> bool {
    solver.guide.init_pending
}

/// A warm-up begins: its walk through the list starts at the top.
#[inline]
pub fn warming(solver: &mut Solver) {
    solver.guide.warm_cursor = 0;
}

/// The preferred variable that the mode's own order ranks best among up to
/// SCAN list entries: the largest score in the stable mode, the latest
/// stamp of the queue in the focused mode.
fn best(solver: &mut Solver) -> u32 {
    let n = solver.guide.list.len();
    if n == 0 {
        return INVALID_IDX;
    }
    let look = n.min(SCAN);
    let start = if n > SCAN { solver.guide.cursor % n } else { 0 };
    let stable = solver.stable;
    let mut res = INVALID_IDX;
    let mut best_score = f64::NEG_INFINITY;
    let mut best_stamp = 0u32;
    for k in 0..look {
        let e = solver.guide.list[(start + k) % n];
        let Some(idx) = candidate(solver, e) else { continue };
        if stable {
            let score = crate::heap::get_heap_score(&solver.scores, idx);
            if res == INVALID_IDX || score > best_score {
                res = idx;
                best_score = score;
            }
        } else {
            let stamp = solver.links[idx as usize].stamp;
            if res == INVALID_IDX || stamp > best_stamp {
                res = idx;
                best_stamp = stamp;
            }
        }
    }
    solver.guide.cursor = (start + look) % n;
    solver.guide.n.looked += look as u64;
    solver.statistics.ticks += look as u64;
    res
}

/// A conflict has been analysed: `solver.analyzed` holds the variables to
/// bump and the trail is still whole.  Walk back from them through the
/// reasons of the propagated ones, the nearest first and at most `cause`
/// reasons, and add every preferred variable met to the variables to bump.
/// A preferred variable ends its path: it is the cause looked for.
pub fn credit_causes(solver: &mut Solver) {
    let budget = solver.guide.cause as u64;
    let vars = solver.vars as usize;
    let mut seen = std::mem::take(&mut solver.guide.seen);
    if seen.len() < vars {
        seen.resize(vars, 0);
    }
    let mut stamp = solver.guide.stamp.wrapping_add(1);
    if stamp == 0 {
        seen.iter_mut().for_each(|x| *x = 0);
        stamp = 1;
    }
    solver.guide.stamp = stamp;
    let mut queue = std::mem::take(&mut solver.guide.queue);
    queue.clear();
    for i in 0..solver.analyzed.len() {
        let idx = solver.analyzed[i];
        seen[idx as usize] = stamp;
        if !prefers(solver, idx) {
            queue.push(idx);
        }
    }
    let (mut head, mut reasons, mut work, mut credited) = (0usize, 0u64, 0u64, 0u64);
    // one literal of a reason: a preferred variable is bumped, another is walked on from
    let mut visit = |solver: &mut Solver, queue: &mut Vec<u32>, other: u32, credited: &mut u64| {
        let v = other >> 1;
        if seen[v as usize] == stamp {
            return;
        }
        seen[v as usize] = stamp;
        let a = solver.assigned[v as usize];
        if a.level == 0 || a.analyzed() {
            return;
        }
        if prefers(solver, v) {
            crate::inline::push_analyzed(solver, v);
            *credited += 1;
        } else {
            queue.push(v);
        }
    };
    while head < queue.len() && reasons < budget {
        let idx = queue[head];
        head += 1;
        let a = solver.assigned[idx as usize];
        // a decision has no reason; a row of the parity engine is not a clause yet
        if a.level == 0 || a.reason == DECISION_REASON || a.reason == UNIT_REASON || a.gauss() {
            continue;
        }
        reasons += 1;
        if a.binary() {
            work += 1;
            visit(solver, &mut queue, a.reason, &mut credited);
        } else {
            let size = solver.arena.clause(a.reason).size();
            work += 1 + size as u64 / 8;
            for i in 0..size {
                let other = solver.arena.clause(a.reason).lit(i);
                if other >> 1 != idx {
                    visit(solver, &mut queue, other, &mut credited);
                }
            }
        }
    }
    solver.guide.seen = seen;
    solver.guide.queue = queue;
    solver.guide.n.credited += credited;
    solver.guide.n.reasons += reasons;
    solver.guide.n.looked += work;
    solver.statistics.ticks += work;
}

/// A decision of the head start after a restart, or INVALID_IDX.
pub fn head_start(solver: &mut Solver) -> u32 {
    if solver.guide.prefix == 0 || solver.guide.peek {
        return INVALID_IDX;
    }
    solver.guide.prefix -= 1;
    let idx = best(solver);
    if idx != INVALID_IDX {
        solver.guide.n.restart += 1;
    }
    idx
}

/// A decision of the warm-up: the next listed variable without a value, or
/// INVALID_IDX when the list is through.  Nothing loses its value during a
/// warm-up, so the list is walked once.
pub fn warm(solver: &mut Solver) -> u32 {
    let n = solver.guide.list.len();
    let mut k = solver.guide.warm_cursor;
    let from = k;
    let mut res = INVALID_IDX;
    while k < n {
        let e = solver.guide.list[k];
        k += 1;
        if let Some(idx) = candidate(solver, e) {
            res = idx;
            break;
        }
    }
    solver.guide.warm_cursor = k;
    let looked = (k - from) as u64;
    solver.guide.n.looked += looked;
    solver.statistics.ticks += looked;
    if res != INVALID_IDX {
        solver.guide.n.warm += 1;
    }
    res
}

/// A random decision: a preferred variable drawn from the list, or
/// INVALID_IDX after TRIES draws without one that can be decided.
pub fn random(solver: &mut Solver) -> u32 {
    let n = solver.guide.list.len();
    if n == 0 {
        return INVALID_IDX;
    }
    let mut res = INVALID_IDX;
    let mut tries = 0u32;
    while tries < TRIES {
        tries += 1;
        let k = crate::random::next_random32(&mut solver.random) as usize % n;
        let e = solver.guide.list[k];
        if let Some(idx) = candidate(solver, e) {
            res = idx;
            break;
        }
    }
    solver.guide.n.looked += tries as u64;
    solver.statistics.ticks += tries as u64;
    if solver.guide.peek {
        return res; // a restart's look ahead: no decision yet
    }
    if res != INVALID_IDX {
        solver.guide.n.random += 1;
    } else {
        solver.guide.n.random_missed += 1;
    }
    res
}

/// In the focused mode `first` is the queue's pick.  When it is not a
/// preferred variable and one is among the next `window` unassigned
/// variables behind it, that one is taken.  The queue's own position stays
/// at `first`, which keeps no value.
pub fn window(solver: &mut Solver, first: u32) -> u32 {
    if prefers(solver, first) {
        return first;
    }
    let window = solver.guide.window;
    // long runs of assigned variables are not walked without end
    let most = 8 * window as u64 + 64;
    let mut res = first;
    let mut idx = first;
    let mut unassigned = 0u32;
    let mut steps = 0u64;
    while steps < most {
        idx = solver.links[idx as usize].prev;
        if disconnected(idx) {
            break;
        }
        steps += 1;
        if solver.values[lit(idx) as usize] != 0 {
            continue;
        }
        if prefers(solver, idx) {
            res = idx;
            break;
        }
        unassigned += 1;
        if unassigned >= window {
            break;
        }
    }
    solver.guide.n.looked += steps;
    solver.statistics.ticks += steps;
    if res != first && !solver.guide.peek {
        solver.guide.n.window += 1;
    }
    res
}

/// The value of a decision on `idx` that the solver would set to `value`
/// (1 true, -1 false): the preferred one when `idx` stands for a variable
/// with a preferred value.
pub fn value(solver: &mut Solver, idx: u32, value: i32) -> i32 {
    let x = solver.export_.get(idx as usize).copied().unwrap_or(0);
    let b = solver.guide.bits.get(x.unsigned_abs() as usize).copied().unwrap_or(solver.guide.new_bits);
    let (same, flipped) = (b & VALUE_SAME != 0, b & VALUE_FLIPPED != 0);
    if same == flipped {
        return value; // none, or two that pull apart
    }
    solver.guide.n.valued += 1;
    let share = solver.guide.phase_share;
    if share < 100 && crate::random::next_random32(&mut solver.random) % 100 >= share {
        return value;
    }
    // the value of the DIMACS variable that gives the preferred variable its
    // preferred value, then that of the internal variable
    let dimacs_true = (solver.guide.phase > 0) == same;
    let internal_true = dimacs_true != (x < 0);
    let res = if internal_true { 1 } else { -1 };
    if res != value {
        solver.guide.n.turned += 1;
    }
    res
}

/// The exit line; nothing when the nudges are off.
pub fn report(solver: &Solver) {
    let g = &solver.guide;
    if !g.on {
        return;
    }
    let o = &solver.options;
    let c = &g.n;
    println!(
        "c guide list={} bump={} front={} window={} restart={} phase={} share={} randec={} warm={} walk={} init={} protect={} cause={} \
         bumped={} fronted={} by_window={} by_restart={} by_random={} random_missed={} by_warm={} valued={} turned={} walk_weighted={} \
         started={} first={} credited={} reasons={} looked={}",
        g.list.len(),
        o.guidebump,
        o.guidefront,
        o.guidewindow,
        o.guiderestart,
        o.guidephase,
        o.guidephaseshare,
        o.guiderandec,
        o.guidewarm,
        o.guidewalk,
        o.guideinit,
        o.guideprotect,
        o.guidecause,
        c.bumped,
        c.fronted,
        c.window,
        c.restart,
        c.random,
        c.random_missed,
        c.warm,
        c.valued,
        c.turned,
        c.walk,
        c.started,
        match g.first {
            1 => "preferred",
            2 => "other",
            _ => "none",
        },
        c.credited,
        c.reasons,
        c.looked
    );
}
