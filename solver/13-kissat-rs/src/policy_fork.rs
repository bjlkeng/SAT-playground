// Not in kissat. Fork mode: branch children at decision points (plan
// §5.3 flavour 1, §7 item 6c, §15 items 3 and 4; step A.8, bead
// SAT-playground-p9m.6.8).
//
// The collector asks for counterfactuals with
//
//   SAT_POLICY_BRANCH=<D>:<knob>[,<D>:<knob>...]
//
// where D is a decision index (0 = D0) and knob one of probe, eliminate,
// reduce, rephase, reorder, mode, margin, sweep. When the parent takes
// decision D it forks one child per alternative entry of that knob's
// menu (every entry but the parent's own, or the entries listed for the
// knob in SAT_POLICY_BRANCH_ACTIONS=<knob>=<e>|<e>[;<knob>=...]). A child
// holds its entry for that one decision epoch and then returns control
// to the parent's policy (stock, random or the net; the RNG state is
// inherited on purpose so a random-mode child continues the parent's
// segment), stops on the tick limit it inherited (`SAT_LIMIT_TICKS`,
// B_cell or twice it on the timeout band), and writes to its own files
// named after the parent's log, the branch point and the child index:
//
//   <log>.b<D>.<k>       the child's policy log (its header carries a
//                        `branch` block: parent pid, parent log, decision,
//                        epoch, knob, entry, index, parent rows)
//   <log>.b<D>.<k>.out   its stdout (the `s` line, model, statistics)
//   <log>.b<D>.<k>.err   its stderr
//
// so the harness, which reads the parent's stdout, never sees a second
// `s` line. At most SAT_POLICY_BRANCH_JOBS children (default 4) are alive
// per parent: the parent blocks at a branch point until one exits, which
// costs wall and no ticks. At exit the parent reaps every child and
// prints one summary line; its log footer lists the children.
//
// Hygiene (plan §7 item 6c). Before fork: stdout, stderr and the log
// writer are flushed, so no buffered output is duplicated. In the child:
// stdout and stderr are redirected first (dup2), the parent's log handle
// is forgotten (never written or closed from the child; its file offset
// is shared with the parent), a new log is opened and its header written,
// the child's decision is put in force and one row records it. A proof
// file or a `-o` output would be written by every child too, so fork mode
// refuses to start with either. A parent killed by a signal sends SIGTERM
// to its live children from the handler (kill(2) is async-signal-safe).
//
// Solver 13 is single-threaded and libc is a dependency already, so
// fork(2) is a plain call; the children of file.rs (decompressors) are
// reaped before search, so waitpid(-1) only ever sees branch children.

use crate::internal::Solver;
use crate::policy::{
    Action, Effort, Timer, INTERVAL_MENU, MARGIN_MENU, MODE_MENU, SWEEP_MENU,
};

pub const MAX_JOBS: usize = 64;
pub const DEFAULT_JOBS: usize = 4;

/// The knobs a branch point can vary, in policy_net::HEAD_NAMES order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Knob {
    Probe = 0,
    Eliminate = 1,
    Reduce = 2,
    Rephase = 3,
    Reorder = 4,
    Mode = 5,
    Margin = 6,
    Sweep = 7,
}

pub const N_KNOBS: usize = 8;

impl Knob {
    pub const ALL: [Knob; N_KNOBS] = [
        Knob::Probe,
        Knob::Eliminate,
        Knob::Reduce,
        Knob::Rephase,
        Knob::Reorder,
        Knob::Mode,
        Knob::Margin,
        Knob::Sweep,
    ];

    pub fn name(self) -> &'static str {
        crate::policy_net::HEAD_NAMES[self as usize]
    }

    pub fn from_name(s: &str) -> Option<Knob> {
        Knob::ALL.iter().copied().find(|k| k.name() == s)
    }

    pub fn menu(self) -> &'static [f32] {
        match self {
            Knob::Probe | Knob::Eliminate | Knob::Reduce | Knob::Rephase | Knob::Reorder => &INTERVAL_MENU,
            Knob::Mode => &MODE_MENU,
            Knob::Margin => &MARGIN_MENU,
            Knob::Sweep => &SWEEP_MENU,
        }
    }

    pub fn get(self, act: &Action) -> f32 {
        match self {
            Knob::Probe => act.interval_mult[Timer::Probe as usize],
            Knob::Eliminate => act.interval_mult[Timer::Eliminate as usize],
            Knob::Reduce => act.interval_mult[Timer::Reduce as usize],
            Knob::Rephase => act.interval_mult[Timer::Rephase as usize],
            Knob::Reorder => act.interval_mult[Timer::Reorder as usize],
            Knob::Mode => act.interval_mult[Timer::Mode as usize],
            Knob::Margin => act.restart_margin,
            Knob::Sweep => act.effort_mult[Effort::Sweep as usize],
        }
    }

    pub fn set(self, act: &mut Action, v: f32) {
        match self {
            Knob::Probe => act.interval_mult[Timer::Probe as usize] = v,
            Knob::Eliminate => act.interval_mult[Timer::Eliminate as usize] = v,
            Knob::Reduce => act.interval_mult[Timer::Reduce as usize] = v,
            Knob::Rephase => act.interval_mult[Timer::Rephase as usize] = v,
            Knob::Reorder => act.interval_mult[Timer::Reorder as usize] = v,
            Knob::Mode => act.interval_mult[Timer::Mode as usize] = v,
            Knob::Margin => act.restart_margin = v,
            Knob::Sweep => act.effort_mult[Effort::Sweep as usize] = v,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BranchPoint {
    pub decision: u64,
    pub knob: Knob,
}

/// What a child knows about itself (also the header's `branch` block).
#[derive(Clone, Debug)]
pub struct ChildInfo {
    pub parent_pid: u32,
    pub parent_log: String,
    pub decision: u64,
    pub epoch: u64,
    pub knob: Knob,
    pub entry: f32,
    pub index: usize,
    pub parent_rows: u64,
    /// True when masking turned the entry back into the parent's action.
    pub masked_to_parent: bool,
}

#[derive(Clone, Debug)]
pub struct Branching {
    /// Points still to come, in the order given.
    pub schedule: Vec<BranchPoint>,
    /// Entries to fork per knob; `None` = every entry but the parent's.
    pub actions: [Option<Vec<f32>>; N_KNOBS],
    pub jobs: usize,
    /// Live children (pids) of this parent.
    pub live: [libc::pid_t; MAX_JOBS],
    pub n_live: usize,
    pub points: u64,
    pub forked: u64,
    pub reaped: u64,
    /// Children that ended other than with exit 0, 10 or 20.
    pub abnormal: u64,
    /// Preformatted entries of the footer's `branches` array.
    pub json: String,
    /// Set in a child; `None` in the parent.
    pub child: Option<ChildInfo>,
    /// Preformatted `branch` block of a child (header and footer).
    pub child_json: String,
    /// The files a child's log may not alias (the parent's own list).
    pub reserved: Vec<(String, String)>,
    /// Every file a child of the schedule could create, checked at start
    /// and re-checked against a child's own files at each fork.
    pub scheduled_files: Vec<String>,
}

impl Default for Branching {
    fn default() -> Self {
        Branching {
            schedule: Vec::new(),
            actions: Default::default(),
            jobs: DEFAULT_JOBS,
            live: [0; MAX_JOBS],
            n_live: 0,
            points: 0,
            forked: 0,
            reaped: 0,
            abnormal: 0,
            json: String::new(),
            child: None,
            child_json: String::from("null"),
            reserved: Vec::new(),
            scheduled_files: Vec::new(),
        }
    }
}

impl Branching {
    pub fn enabled(&self) -> bool {
        !self.schedule.is_empty() || self.points > 0
    }
}

/// `SAT_POLICY_BRANCH`: `D:knob[,D:knob...]`.
pub fn parse_schedule(v: &str) -> Result<Vec<BranchPoint>, String> {
    let mut out: Vec<BranchPoint> = Vec::new();
    for item in v.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let (d, k) = item
            .split_once(':')
            .ok_or_else(|| format!("SAT_POLICY_BRANCH='{}': expected <decision>:<knob> items", v))?;
        let decision = d
            .trim()
            .parse::<u64>()
            .map_err(|_| format!("SAT_POLICY_BRANCH='{}': '{}' is not a decision index", v, d.trim()))?;
        let knob = Knob::from_name(k.trim()).ok_or_else(|| {
            format!(
                "SAT_POLICY_BRANCH='{}': '{}' is not a knob (probe, eliminate, reduce, rephase, reorder, mode, margin, sweep)",
                v,
                k.trim()
            )
        })?;
        if out.iter().any(|bp| bp.decision == decision) {
            // One knob per point: the children's file names carry the
            // decision and the child index, so a second point at the same
            // decision would truncate the first point's files.
            return Err(format!("SAT_POLICY_BRANCH='{}': decision {} is listed twice", v, decision));
        }
        out.push(BranchPoint { decision, knob });
    }
    if out.is_empty() {
        return Err(format!("SAT_POLICY_BRANCH='{}': no branch points", v));
    }
    Ok(out)
}

/// The stem of a child's files: `<log>.b<D>.<k>`.
fn child_stem(parent_log: &str, decision: u64, index: usize) -> String {
    format!("{}.b{}.{}", parent_log, decision, index)
}

/// A child's three files: its log, its stdout and its stderr.
fn child_files(stem: &str) -> [String; 3] {
    [stem.to_string(), format!("{}.out", stem), format!("{}.err", stem)]
}

/// A path resolved once for alias checks: its string, its final absolute
/// form (symlinks followed by hand, so a dangling link to a not-yet-created
/// file counts) and its device and inode when it exists.
struct Resolved {
    s: String,
    canon: Option<std::path::PathBuf>,
    ino: Option<(u64, u64)>,
}

fn resolve_file(p: &str) -> Resolved {
    use std::os::unix::fs::MetadataExt as _;
    Resolved {
        s: p.to_string(),
        canon: crate::policy_log::resolve(p),
        ino: std::fs::metadata(p).ok().map(|m| (m.dev(), m.ino())),
    }
}

fn aliases(a: &Resolved, b: &Resolved) -> bool {
    a.s == b.s || (a.canon.is_some() && a.canon == b.canon) || (a.ino.is_some() && a.ino == b.ino)
}

/// Refuse when one of `files` (a child's log, stdout and stderr, or every
/// file of the schedule) is, by string, resolved path or inode, a file
/// this run owns or reads, the file behind a standard stream, one of
/// `others` (the other children's files), or another of `files`. Nothing
/// is created here.
fn check_files(files: &[String], reserved: &[(String, String)], others: &[String]) -> Result<(), String> {
    let res: Vec<Resolved> = files.iter().map(|f| resolve_file(f)).collect();
    let res_reserved: Vec<(&String, Resolved)> = reserved.iter().map(|(w, p)| (w, resolve_file(p))).collect();
    let res_others: Vec<Resolved> = others.iter().map(|f| resolve_file(f)).collect();
    for (i, f) in res.iter().enumerate() {
        for (what, other) in &res_reserved {
            if aliases(f, other) {
                return Err(format!("branch child file '{}' is the {} file '{}'", f.s, what, other.s));
            }
        }
        if let Some(stream) = crate::policy_log::standard_stream_behind(&f.s) {
            return Err(format!("branch child file '{}' is the file behind this process's {}", f.s, stream));
        }
        for g in &res_others {
            if aliases(f, g) {
                return Err(format!("branch child files '{}' and '{}' are the same file", f.s, g.s));
            }
        }
        for g in &res[..i] {
            if aliases(f, g) {
                return Err(format!("branch child files '{}' and '{}' are the same file", f.s, g.s));
            }
        }
    }
    Ok(())
}

/// `SAT_POLICY_BRANCH_ACTIONS`: `knob=e|e[;knob=e|e...]`, entries as the
/// menu values (`0`, `0.5`, `1`, `2`, `4`).
pub fn parse_actions(v: &str) -> Result<[Option<Vec<f32>>; N_KNOBS], String> {
    let mut out: [Option<Vec<f32>>; N_KNOBS] = Default::default();
    for item in v.split(';') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let (k, es) = item
            .split_once('=')
            .ok_or_else(|| format!("SAT_POLICY_BRANCH_ACTIONS='{}': expected <knob>=<entry>|<entry> items", v))?;
        let knob = Knob::from_name(k.trim())
            .ok_or_else(|| format!("SAT_POLICY_BRANCH_ACTIONS='{}': '{}' is not a knob", v, k.trim()))?;
        let mut entries = Vec::new();
        for e in es.split('|') {
            let x = e
                .trim()
                .parse::<f32>()
                .ok()
                .filter(|x| knob.menu().contains(x))
                .ok_or_else(|| {
                    format!(
                        "SAT_POLICY_BRANCH_ACTIONS='{}': '{}' is not in the {} menu {:?}",
                        v,
                        e.trim(),
                        knob.name(),
                        knob.menu()
                    )
                })?;
            if !entries.contains(&x) {
                entries.push(x);
            }
        }
        if entries.is_empty() {
            return Err(format!("SAT_POLICY_BRANCH_ACTIONS='{}': no entries for {}", v, knob.name()));
        }
        out[knob as usize] = Some(entries);
    }
    Ok(out)
}

/// Configure fork mode from the environment; called from
/// `policy::init_from_env` once the log is known. `taken` are the files
/// the run owns; a proof or an output file refuses fork mode.
pub fn init_from_env(p: &mut crate::policy::Policy, taken: &[(&str, &str)], ticks_limited: bool) -> Result<(), String> {
    let env = |name: &str| match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    };
    let Some(schedule) = env("SAT_POLICY_BRANCH") else {
        return Ok(());
    };
    let b = &mut p.branching;
    b.schedule = parse_schedule(&schedule)?;
    if p.log.is_none() {
        return Err("SAT_POLICY_BRANCH needs SAT_POLICY_LOG: children log next to the parent's log".to_string());
    }
    if !ticks_limited {
        // Children inherit no alarm and no wall clock: a child without a
        // tick budget would run until it solved, and a parent under
        // `--time` alone would wait for it (plan §5.4: every perturbed
        // run stops on a tick budget).
        return Err("SAT_POLICY_BRANCH needs SAT_LIMIT_TICKS: children stop on the inherited tick budget, never on a wall clock".to_string());
    }
    for (what, path) in taken {
        if *what == "proof" || *what == "output" {
            return Err(format!(
                "SAT_POLICY_BRANCH refuses to fork with a {} file open ('{}'): every child would write it too",
                what, path
            ));
        }
    }
    if let Some(v) = env("SAT_POLICY_BRANCH_ACTIONS") {
        b.actions = parse_actions(&v)?;
    }
    if let Some(v) = env("SAT_POLICY_BRANCH_JOBS") {
        b.jobs = v
            .parse::<usize>()
            .ok()
            .filter(|&j| (1..=MAX_JOBS).contains(&j))
            .ok_or_else(|| format!("SAT_POLICY_BRANCH_JOBS='{}': expected 1..{}", v, MAX_JOBS))?;
    }
    b.reserved = taken.iter().map(|(w, p)| (w.to_string(), p.to_string())).collect();
    // Every file a child of this schedule could create is checked now,
    // before any work is done: the whole menu per point, since the entries
    // actually forked depend on the parent's action at the time; against
    // the run's files, the standard streams and each other.
    let parent_log = p.log.as_ref().map(|l| l.path().to_string()).unwrap_or_default();
    let mut all: Vec<String> = Vec::new();
    for bp in &b.schedule {
        for k in 0..bp.knob.menu().len() {
            all.extend(child_files(&child_stem(&parent_log, bp.decision, k)));
        }
    }
    check_files(&all, &b.reserved, &[]).map_err(|e| format!("SAT_POLICY_BRANCH: {}", e))?;
    b.scheduled_files = all;
    Ok(())
}

fn menu_value(v: f32) -> String {
    if v == v.trunc() {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

/// The `branch` block of a child's header and footer.
fn child_json(c: &ChildInfo) -> String {
    let mut s = String::with_capacity(256);
    s.push_str(&format!("{{\"parent_pid\":{},\"parent_log\":", c.parent_pid));
    crate::policy_log::json_escape_into(&c.parent_log, &mut s);
    s.push_str(&format!(
        ",\"decision\":{},\"epoch\":{},\"knob\":\"{}\",\"entry\":{},\"index\":{},\"parent_rows\":{},\"masked_to_parent\":{}}}",
        c.decision,
        c.epoch,
        c.knob.name(),
        menu_value(c.entry),
        c.index,
        c.parent_rows,
        c.masked_to_parent
    ));
    s
}

/// Called at the end of every `policy::decide` in the parent: fork the
/// children of the branch points at this decision, if any.
pub fn maybe_branch(solver: &mut Solver) {
    let p = &solver.policy;
    if p.branching.child.is_some() || p.branching.schedule.is_empty() {
        return;
    }
    let decision = p.decisions.saturating_sub(1);
    if !p.branching.schedule.iter().any(|bp| bp.decision == decision) {
        return;
    }
    let due: Vec<BranchPoint> = p.branching.schedule.iter().copied().filter(|bp| bp.decision == decision).collect();
    solver.policy.branching.schedule.retain(|bp| bp.decision != decision);
    let parent_act = solver.policy.act;
    for bp in due {
        let entries: Vec<f32> = match &solver.policy.branching.actions[bp.knob as usize] {
            Some(list) => list.clone(),
            None => {
                let own = bp.knob.get(&parent_act);
                bp.knob.menu().iter().copied().filter(|&e| e != own).collect()
            }
        };
        solver.policy.branching.points += 1;
        for (k, entry) in entries.into_iter().enumerate() {
            fork_one(solver, decision, bp.knob, entry, k, parent_act);
            if solver.policy.branching.child.is_some() {
                // This is the child: it takes no part in the rest of the
                // parent's branching.
                return;
            }
        }
    }
}

fn flush_before_fork(solver: &mut Solver) {
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    if let Some(log) = solver.policy.log.as_mut() {
        log.flush();
    }
}

fn fork_one(solver: &mut Solver, decision: u64, knob: Knob, entry: f32, index: usize, parent_act: Action) {
    if solver.proof.is_some() {
        crate::print::warning(solver, "not forking: a proof file is open");
        return;
    }
    // The child's files, checked again now (the start-up check covered the
    // schedule, but a file can have appeared since): an alias skips the
    // child rather than truncating anything.
    let parent_log = solver.policy.log.as_ref().map(|l| l.path().to_string()).unwrap_or_default();
    let stem = child_stem(&parent_log, decision, index);
    let files = child_files(&stem);
    let others: Vec<String> = solver
        .policy
        .branching
        .scheduled_files
        .iter()
        .filter(|f| !files.contains(f))
        .cloned()
        .collect();
    if let Err(text) = check_files(&files, &solver.policy.branching.reserved, &others) {
        crate::print::warning(solver, format_args!("not forking: {}", text));
        return;
    }
    let parent_pid = std::process::id();
    wait_for_slot(solver);
    // Handled signals stay blocked from before the pre-fork flush until
    // the parent has registered the child and the child has detached from
    // the parent's files and forgotten its siblings; the mask is inherited
    // across fork, so the child starts blocked too. Otherwise a SIGTERM
    // during the flush would re-enter the log writer from the handler
    // (Codex review round 4), and one in the fork window would run the
    // child's inherited handler against the parent's log and live list,
    // or leave the parent with an untracked child.
    let guard = crate::policy_log::SignalGuard::block();
    flush_before_fork(solver);
    // SAFETY: the process is single-threaded (solver 13 spawns no threads;
    // file.rs's decompressor children are reaped before search), stdout,
    // stderr and the log are flushed, and the child touches only its own
    // copies of every handle.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        drop(guard);
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        crate::print::warning(
            solver,
            format_args!("fork failed at decision {} (os error {}); skipping this child", decision, code),
        );
        return;
    }
    if pid == 0 {
        become_child(solver, parent_pid, &stem, decision, knob, entry, index, parent_act);
        drop(guard);
        return;
    }
    let b = &mut solver.policy.branching;
    b.live[b.n_live] = pid;
    b.n_live += 1;
    b.forked += 1;
    // The footer's branch record is completed while signals are still
    // blocked: the handler writes `json` as it stands, so it must never see
    // a comma without its record or a string mid-reallocation.
    let record = format!(
        "{}{{\"decision\":{},\"knob\":\"{}\",\"entry\":{},\"index\":{},\"pid\":{}}}",
        if b.json.is_empty() { "" } else { "," },
        decision,
        knob.name(),
        menu_value(entry),
        index,
        pid
    );
    b.json.push_str(&record);
    drop(guard);
    crate::print::verbose(
        solver,
        format_args!(
            "policy branch: decision {} knob {} entry {} child {} pid {}",
            decision,
            knob.name(),
            menu_value(entry),
            index,
            pid
        ),
    );
}

/// Redirect a standard descriptor to a fresh file; false on failure.
fn redirect(fd: libc::c_int, path: &str) -> bool {
    let Ok(cpath) = std::ffi::CString::new(path) else {
        return false;
    };
    // SAFETY: plain libc calls with a valid C string and descriptors.
    unsafe {
        let new = libc::open(
            cpath.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC,
            0o644,
        );
        if new < 0 {
            return false;
        }
        let ok = libc::dup2(new, fd) == fd;
        libc::close(new);
        ok
    }
}

#[allow(clippy::too_many_arguments)]
fn become_child(
    solver: &mut Solver,
    parent_pid: u32,
    stem: &str,
    decision: u64,
    knob: Knob,
    entry: f32,
    index: usize,
    parent_act: Action,
) {
    let (parent_log, parent_rows, wall0) = {
        let log = solver.policy.log.as_ref().expect("fork mode needs a log");
        (log.path().to_string(), log.rows(), solver.policy.wall0)
    };
    // Streams first, so anything printed from here on lands in the
    // child's files, never in the parent's stdout.
    let out_ok = redirect(1, &format!("{}.out", stem));
    let err_ok = redirect(2, &format!("{}.err", stem));
    if !out_ok || !err_ok {
        // SAFETY: _exit skips every destructor and buffer, which is what a
        // child that could not detach from its parent's streams needs.
        unsafe { libc::_exit(1) };
    }
    // The parent's log handle: never written or closed from here (the
    // file offset is shared with the parent), so it is forgotten.
    if let Some(old) = solver.policy.log.take() {
        std::mem::forget(old);
    }
    {
        let b = &mut solver.policy.branching;
        b.schedule.clear();
        b.n_live = 0;
        b.forked = 0;
        b.points = 0;
        b.json.clear();
    }
    let reserved: Vec<(String, String)> = solver.policy.branching.reserved.clone();
    let taken: Vec<(&str, &str)> = reserved.iter().map(|(w, p)| (w.as_str(), p.as_str())).collect();
    match crate::policy_log::Logger::open(stem, &taken, wall0) {
        Ok(log) => solver.policy.log = Some(Box::new(log)),
        Err(text) => {
            eprintln!("sat-solver: branch child: {}", text);
            // SAFETY: as above; a child without a log has no purpose.
            unsafe { libc::_exit(1) };
        }
    }
    // The child's decision: the parent's action with one knob changed,
    // masked like any other; one-shot entries re-arm as usual.
    let mut act = parent_act;
    knob.set(&mut act, entry);
    let act = crate::policy::mask(solver, act);
    let info = ChildInfo {
        parent_pid,
        parent_log,
        decision,
        epoch: solver.policy.obs_epochs.saturating_sub(1),
        knob,
        entry,
        index,
        parent_rows,
        masked_to_parent: act == parent_act,
    };
    solver.policy.branching.child_json = child_json(&info);
    solver.policy.branching.child = Some(info);
    solver.policy.act = act;
    solver.policy.decided = act;
    // Header now (allocation is fine: this is the normal path), then one
    // row for the branch state under the child's decision (not a boundary:
    // the parent already pushed this boundary's snapshot).
    crate::policy_log::prepare(solver);
    crate::policy::log_row(solver);
    crate::print::message(
        solver,
        format!(
            "policy branch child: decision {} knob {} entry {} index {} of parent {} (masked to parent: {})",
            decision,
            knob.name(),
            menu_value(entry),
            index,
            parent_pid,
            act == parent_act
        ),
    );
}

/// Reap one child; `blocking` waits for one to exit. Returns false when
/// there is none to reap.
fn reap_one(solver: &mut Solver, blocking: bool) -> bool {
    let mut status: libc::c_int = 0;
    loop {
        let flags = if blocking { 0 } else { libc::WNOHANG };
        // SAFETY: waitpid on this process's children.
        let pid = unsafe { libc::waitpid(-1, &mut status, flags) };
        if pid > 0 {
            let b = &mut solver.policy.branching;
            if let Some(i) = b.live[..b.n_live].iter().position(|&p| p == pid) {
                b.live[i] = b.live[b.n_live - 1];
                b.n_live -= 1;
            }
            b.reaped += 1;
            let code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else if libc::WIFSIGNALED(status) {
                128 + libc::WTERMSIG(status)
            } else {
                -1
            };
            if !matches!(code, 0 | 10 | 20) {
                b.abnormal += 1;
            }
            crate::print::verbose(solver, format_args!("policy branch: child pid {} exited {}", pid, code));
            return true;
        }
        if pid == 0 {
            return false;
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EINTR) => continue,
            _ => {
                // ECHILD: nothing left to wait for.
                solver.policy.branching.n_live = 0;
                return false;
            }
        }
    }
}

/// A parent that has been told to stop (kissat's `--time` alarm or an
/// external `terminate`) does not wait for children to finish their
/// budgets: they are told to stop too, seal their logs and exit.
fn stop_children_if_terminating(solver: &Solver) {
    if solver.termination.flagged.load(std::sync::atomic::Ordering::SeqCst) {
        kill_children(solver);
    }
}

/// Block until fewer than `jobs` children are alive.
fn wait_for_slot(solver: &mut Solver) {
    while solver.policy.branching.n_live > 0 {
        // Reap finished children first without blocking.
        if !reap_one(solver, false) {
            break;
        }
    }
    while solver.policy.branching.n_live >= solver.policy.branching.jobs {
        stop_children_if_terminating(solver);
        if !reap_one(solver, true) {
            break;
        }
    }
}

/// At the parent's exit: reap every child and print the summary line.
pub fn wait_children(solver: &mut Solver) {
    if !solver.policy.on || solver.policy.branching.child.is_some() {
        return;
    }
    if !solver.policy.branching.enabled() && solver.policy.branching.forked == 0 {
        return;
    }
    while solver.policy.branching.n_live > 0 {
        stop_children_if_terminating(solver);
        if !reap_one(solver, true) {
            break;
        }
    }
    let b = &solver.policy.branching;
    crate::print::message(
        solver,
        format!(
            "policy branch: {} points, {} children forked, {} reaped, {} abnormal, {} points not reached",
            b.points,
            b.forked,
            b.reaped,
            b.abnormal,
            b.schedule.len()
        ),
    );
}

/// From the signal handler: SIGTERM every live child (kill(2) is
/// async-signal-safe; nothing here allocates or prints).
pub fn kill_children(solver: &Solver) {
    let b = &solver.policy.branching;
    if b.child.is_some() {
        return;
    }
    for &pid in &b.live[..b.n_live] {
        if pid > 0 {
            // SAFETY: signalling this process's own child.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_and_actions_parse() {
        let s = parse_schedule("0:probe, 12:mode ,3:sweep").unwrap();
        assert_eq!(s.len(), 3);
        assert_eq!(s[1], BranchPoint { decision: 12, knob: Knob::Mode });
        assert!(parse_schedule("").is_err());
        assert!(parse_schedule("x:probe").unwrap_err().contains("not a decision index"));
        assert!(parse_schedule("1:vivify").unwrap_err().contains("not a knob"));
        assert!(parse_schedule("1").unwrap_err().contains("expected"));
        assert!(parse_schedule("0:probe,0:reduce").unwrap_err().contains("listed twice"));
        let a = parse_actions("probe=0|4;mode=0.5").unwrap();
        assert_eq!(a[Knob::Probe as usize].as_deref(), Some(&[0.0f32, 4.0][..]));
        assert_eq!(a[Knob::Mode as usize].as_deref(), Some(&[0.5f32][..]));
        assert!(a[Knob::Sweep as usize].is_none());
        assert!(parse_actions("probe=3").unwrap_err().contains("not in the probe menu"));
        assert!(parse_actions("mode=0").unwrap_err().contains("not in the mode menu"));
        assert!(parse_actions("probe").unwrap_err().contains("expected"));
    }

    #[test]
    fn child_files_may_not_alias_reserved_files_or_each_other() {
        let dir = std::env::temp_dir().join(format!("sat13-fork-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("run.log").to_str().unwrap().to_string();
        let cnf = dir.join("x.cnf").to_str().unwrap().to_string();
        std::fs::write(&cnf, b"p cnf 1 1\n1 0\n").unwrap();
        let reserved = vec![("input".to_string(), cnf.clone()), ("log".to_string(), log.clone())];
        let files = child_files(&child_stem(&log, 3, 1));
        assert_eq!(files[0], format!("{}.b3.1", log));
        assert!(check_files(&files, &reserved, &[]).is_ok());
        // The child's stdout file is the input, through a symlink.
        std::os::unix::fs::symlink(&cnf, format!("{}.b3.1.out", log)).unwrap();
        let err = check_files(&files, &reserved, &[]).unwrap_err();
        assert!(err.contains("input"), "{}", err);
        std::fs::remove_file(format!("{}.b3.1.out", log)).unwrap();
        // The child's log is the parent's log (a dangling link is enough).
        std::os::unix::fs::symlink("run.log", format!("{}.b3.1", log)).unwrap();
        let err = check_files(&files, &reserved, &[]).unwrap_err();
        assert!(err.contains("log file"), "{}", err);
        std::fs::remove_file(format!("{}.b3.1", log)).unwrap();
        // Two of the three are one file.
        std::fs::write(format!("{}.b3.1.out", log), b"").unwrap();
        std::fs::hard_link(format!("{}.b3.1.out", log), format!("{}.b3.1.err", log)).unwrap();
        let err = check_files(&files, &reserved, &[]).unwrap_err();
        assert!(err.contains("same file"), "{}", err);
        std::fs::remove_file(format!("{}.b3.1.err", log)).unwrap();
        // Another point's file, through a dangling link to this child's log.
        let other = child_files(&child_stem(&log, 5, 0));
        std::os::unix::fs::symlink("run.log.b3.1", format!("{}.b5.0", log)).unwrap();
        let mut all: Vec<String> = files.to_vec();
        all.extend(other.iter().cloned());
        let err = check_files(&all, &reserved, &[]).unwrap_err();
        assert!(err.contains("same file"), "{}", err);
        assert!(check_files(&files, &reserved, &other).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn knobs_read_and_write_the_action() {
        let mut act = Action::default();
        for k in Knob::ALL {
            assert_eq!(k.get(&act), 1.0);
            assert_eq!(Knob::from_name(k.name()), Some(k));
            let e = k.menu()[0];
            k.set(&mut act, e);
            assert_eq!(k.get(&act), e);
        }
        assert_eq!(act.effort_mult[Effort::Sweep as usize], 0.0);
        assert_eq!(act.restart_margin, 0.5);
        assert_eq!(menu_value(0.5), "0.5");
        assert_eq!(menu_value(4.0), "4");
    }
}
