// Not in kissat. Raw-state logger for the RL scheduler (plan §5.1, §7 items
// 1, 6d and 6e; step A.5, bead SAT-playground-p9m.6.6).
//
// `SAT_POLICY_LOG=<path>` writes one row per observation epoch (plus one
// final row at exit) with every `Statistics` counter, both averages
// blocks, every limit, the delay counters, the elimination bound, the tier
// glue limits, the search state, the per-epoch learned-clause histogram,
// the per-timer stock counterfactual, the action in force, the policy RNG
// state and the monotonic wall and CPU clocks. The header carries the
// run's configuration (all options, the policy settings); the footer
// carries the result, peak RSS and, from step A.7, the static features
// (known only after preprocessing, so they cannot be in a header that is
// written before parsing).
//
// Format (self-describing, one file):
//   line 1   "SAT13POLICYLOG 1\n"
//   line 2   one JSON object: format, solver, k_res, cnf, pid, policy,
//            options, columns, kinds (one char per column: 'u' = u64,
//            'f' = f64 stored as its bit pattern), row_bytes
//   rows     row_bytes each, little-endian u64 per column
//   sentinel one row whose first u64 is u64::MAX
//   footer   one JSON object: result, exit, reason, rows, wall_ns, cpu_ns,
//            peak_rss_bytes, work, conflicts, static
// tools/policy_log.py reads it; tests/policy_log.rs checks the last row
// against the `-s` counters and the `c workclock` line.
//
// Purity (plan §7 item 6e): `row()` reads solver state and writes to the
// logger's own buffer. It draws nothing from any generator, touches no
// solver container, and increments no counter a heuristic reads. The
// per-epoch histogram lives in `Policy`, fed by `policy::note_learned`
// under `policy.on`, and is read by nothing else.

use crate::internal::Solver;
use crate::policy::{Timer, N_EFFORTS};
use std::io::Write as _;

pub const MAGIC: &str = "SAT13POLICYLOG 1";
pub const FORMAT: u32 = 1;

/// Glue histogram bins for the per-epoch learned-clause quality feature:
/// glue 0-1, 2, 3, 4, 5-6, 7-10, 11-20, 21+.
pub const GLUE_BINS: usize = 8;

pub fn glue_bin(glue: u32) -> usize {
    match glue {
        0 | 1 => 0,
        2 => 1,
        3 => 2,
        4 => 3,
        5 | 6 => 4,
        7..=10 => 5,
        11..=20 => 6,
        _ => 7,
    }
}

pub const EFFORT_NAMES: [&str; N_EFFORTS] = [
    "sweep",
    "vivify",
    "eliminate",
    "backbone",
    "factor",
    "forward",
    "transitive",
    "walk",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    U,
    F,
}

pub struct Logger {
    path: String,
    out: std::io::BufWriter<std::fs::File>,
    header_written: bool,
    finished: bool,
    rows: u64,
    row_width: usize,
    wall0: u64,
    /// Row values; keeps its capacity after the first row, so later rows
    /// (and the sentinel) do not allocate.
    buf: Vec<u64>,
    /// Preallocated footer text (the footer is written from the signal
    /// handler too).
    footer: String,
    /// The first write or flush error as its raw OS error code (0 for a
    /// non-OS error), kept without allocating because it can be set from
    /// the signal handler; once set nothing more is written, the footer is
    /// withheld (so a reader sees an incomplete file) and one warning is
    /// printed.
    error: Option<i32>,
    warned: bool,
}

/// Block the asynchronous signals kissat handles (SIGINT, SIGTERM, SIGALRM)
/// while the logger is taken out of `Policy` and while a row or the footer
/// is being written, so the signal handler's `finish_log` never runs in the
/// middle of a write or finds the logger missing; the signal is delivered
/// when the guard drops. The synchronous signals (SIGSEGV, SIGBUS, SIGABRT)
/// are left alone.
pub(crate) struct SignalGuard {
    old: libc::sigset_t,
}

impl SignalGuard {
    pub(crate) fn block() -> SignalGuard {
        // SAFETY: plain libc calls on stack-allocated signal sets.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            let mut old: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGINT);
            libc::sigaddset(&mut set, libc::SIGTERM);
            libc::sigaddset(&mut set, libc::SIGALRM);
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut old);
            SignalGuard { old }
        }
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        // SAFETY: restores the mask saved by `block`.
        unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, &self.old, std::ptr::null_mut());
        }
    }
}

/// The final absolute form of a path that may not exist yet: symlinks are
/// followed by hand (so a dangling link still resolves to its target), then
/// the parent is canonicalized and the file name re-attached.
pub(crate) fn resolve(p: &str) -> Option<std::path::PathBuf> {
    let mut path = std::path::PathBuf::from(p);
    for _ in 0..64 {
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_symlink() => {
                let target = std::fs::read_link(&path).ok()?;
                path = if target.is_absolute() {
                    target
                } else {
                    path.parent()
                        .filter(|d| !d.as_os_str().is_empty())
                        .unwrap_or(std::path::Path::new("."))
                        .join(target)
                };
            }
            _ => break,
        }
    }
    if let Ok(c) = std::fs::canonicalize(&path) {
        return Some(c);
    }
    let parent = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let name = path.file_name()?.to_owned();
    std::fs::canonicalize(parent).ok().map(|d| d.join(name))
}

/// True when `a` and `b` name the same file: equal strings, equal resolved
/// paths (symlinks followed, missing files resolved against their parent),
/// or the same device and inode when both exist.
pub fn same_path(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if let (Some(x), Some(y)) = (resolve(a), resolve(b)) {
        if x == y {
            return true;
        }
    }
    if let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b)) {
        use std::os::unix::fs::MetadataExt as _;
        return x.dev() == y.dev() && x.ino() == y.ino();
    }
    false
}

/// The standard stream that `path` is the file behind, if any: a CNF read
/// through `< input.cnf` has no positional path to compare against, and a
/// log that truncated the file behind stdout or stderr would destroy the
/// output the harness reads.
pub fn standard_stream_behind(path: &str) -> Option<&'static str> {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::metadata(path).ok()?; // follows symlinks; None if it does not exist yet
    for (fd, name) in [(0, "stdin"), (1, "stdout"), (2, "stderr")] {
        // SAFETY: fstat on a standard descriptor into a zeroed stat buffer.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut st) } == 0 && st.st_dev == meta.dev() && st.st_ino == meta.ino() {
            return Some(name);
        }
    }
    None
}

fn clock_ns(clock: libc::clockid_t) -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(clock, &mut ts) } != 0 {
        return 0;
    }
    (ts.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(ts.tv_nsec as u64)
}

pub fn wall_ns() -> u64 {
    clock_ns(libc::CLOCK_MONOTONIC)
}

pub fn cpu_ns() -> u64 {
    clock_ns(libc::CLOCK_PROCESS_CPUTIME_ID)
}

impl Logger {
    /// Create the log file. `taken` are the paths the run already owns
    /// (the CNF, the proof file, a `-o` output): a log that aliases one of
    /// them would truncate the input or put two writers on the proof, so
    /// it is refused before anything is created.
    pub fn open(path: &str, taken: &[(&str, &str)], wall0: u64) -> Result<Logger, String> {
        for (what, other) in taken {
            if same_path(path, other) {
                return Err(format!(
                    "SAT_POLICY_LOG='{}' is the {} file '{}'",
                    path, what, other
                ));
            }
        }
        if let Some(stream) = standard_stream_behind(path) {
            return Err(format!(
                "SAT_POLICY_LOG='{}' is the file behind this process's {}",
                path, stream
            ));
        }
        let file = std::fs::File::create(path)
            .map_err(|e| format!("SAT_POLICY_LOG='{}': cannot create: {}", path, e))?;
        Ok(Logger {
            path: path.to_string(),
            out: std::io::BufWriter::with_capacity(1 << 16, file),
            header_written: false,
            finished: false,
            rows: 0,
            row_width: 0,
            wall0,
            buf: Vec::with_capacity(1024),
            footer: String::with_capacity(1024),
            error: None,
            warned: false,
        })
    }

    /// Write, remembering the first failure; nothing is written after one.
    fn put(&mut self, bytes: &[u8]) {
        if self.error.is_some() {
            return;
        }
        if let Err(e) = self.out.write_all(bytes) {
            self.error = Some(e.raw_os_error().unwrap_or(0));
        }
    }

    fn flush_checked(&mut self) {
        if self.error.is_some() {
            return;
        }
        if let Err(e) = self.out.flush() {
            self.error = Some(e.raw_os_error().unwrap_or(0));
        }
    }

    pub fn error(&self) -> Option<i32> {
        self.error
    }

    pub fn header_written(&self) -> bool {
        self.header_written
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn rows(&self) -> u64 {
        self.rows
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Flush the buffered rows (before a fork); the handled signals are
    /// blocked meanwhile so the handler's footer cannot re-enter the
    /// writer mid-flush.
    pub fn flush(&mut self) {
        let _guard = SignalGuard::block();
        self.flush_checked();
    }
}

/// Clocks and the row index for one row, read once in `row()` and passed
/// into `columns()` so the column function stays a pure reader of state.
#[derive(Clone, Copy, Default)]
pub struct RowClock {
    pub row: u64,
    pub wall_ns: u64,
    pub cpu_ns: u64,
    /// True when the row is an observation boundary (its snapshot is
    /// pushed into the ring after the row).
    pub boundary: bool,
}

// ---------------------------------------------------------------------------
// Columns. One function defines names, kinds and values together so they
// cannot drift: the header pass collects names and kinds, the row pass
// collects values.
// ---------------------------------------------------------------------------

fn bits(x: f64) -> u64 {
    x.to_bits()
}

fn flag(x: bool) -> u64 {
    x as u64
}

/// Emit every column as (name, kind, value). With `want_names == false`
/// (every row after the header) nothing here allocates: names are not
/// formatted, the counters are visited in place, and every value is a
/// plain read. That is what makes the row writer safe to run from the
/// signal handler, which may interrupt the solver inside `malloc`.
fn columns(solver: &Solver, clock: &RowClock, want_names: bool, mut emit: impl FnMut(&str, Kind, u64)) {
    let p = &solver.policy;
    let st = &solver.statistics;
    macro_rules! u {
        ($v:expr, $($name:tt)+) => {{
            let v: u64 = $v;
            if want_names {
                emit(&format!($($name)+), Kind::U, v)
            } else {
                emit("", Kind::U, v)
            }
        }};
    }
    macro_rules! fl {
        ($v:expr, $($name:tt)+) => {{
            let v: u64 = bits($v);
            if want_names {
                emit(&format!($($name)+), Kind::F, v)
            } else {
                emit("", Kind::F, v)
            }
        }};
    }

    // Row identity and clocks (`search_ticks` and `decisions` are among the
    // statistics counters below; the policy's own decision count is
    // `policy_decisions`).
    u!(clock.row, "row");
    u!(p.obs_epochs, "obs_epoch");
    u!(p.decisions, "policy_decisions");
    u!(st.work_clock(), "work");
    u!(clock.wall_ns, "wall_ns");
    u!(clock.cpu_ns, "cpu_ns");

    // Every statistics counter, by name, then the two clause-use glue
    // histograms (`statistics.used[focused|stable].glue[0..=127]`, bumped
    // by deduce::mark_clause_as_used), which are counters too.
    st.each(|name, value| emit(name, Kind::U, value));
    for (i, tag) in ["f", "s"].iter().enumerate() {
        for (g, n) in st.used[i].glue.iter().enumerate() {
            u!(*n, "used_{}_glue{}", tag, g);
        }
    }

    // Averages, both blocks (f = focused, s = stable).
    for (i, tag) in ["f", "s"].iter().enumerate() {
        let a = &solver.averages[i];
        u!(flag(a.initialized), "avg_{}_initialized", tag);
        fl!(a.fast_glue.value, "avg_{}_fast_glue", tag);
        fl!(a.slow_glue.value, "avg_{}_slow_glue", tag);
        fl!(a.level.value, "avg_{}_level", tag);
        fl!(a.size.value, "avg_{}_size", tag);
        fl!(a.trail.value, "avg_{}_trail", tag);
        fl!(a.decision_rate.value, "avg_{}_decision_rate", tag);
        u!(a.saved_decisions, "avg_{}_saved_decisions", tag);
    }

    // Limits.
    let l = &solver.limits;
    u!(l.conflicts, "lim_conflicts");
    u!(l.decisions, "lim_decisions");
    u!(l.ticks, "lim_ticks");
    u!(flag(solver.limited.conflicts), "limited_conflicts");
    u!(flag(solver.limited.decisions), "limited_decisions");
    u!(flag(solver.limited.ticks), "limited_ticks");
    u!(l.mode.count, "lim_mode_count");
    u!(l.mode.ticks, "lim_mode_ticks");
    u!(l.mode.conflicts, "lim_mode_conflicts");
    u!(l.eliminate.conflicts, "lim_eliminate_conflicts");
    u!(l.eliminate.variables.eliminate, "lim_eliminate_vars_eliminate");
    u!(l.eliminate.variables.subsume, "lim_eliminate_vars_subsume");
    u!(l.factor.marked, "lim_factor_marked");
    u!(l.probe.conflicts, "lim_probe_conflicts");
    u!(l.randec.conflicts, "lim_randec_conflicts");
    u!(l.reduce.conflicts, "lim_reduce_conflicts");
    u!(l.reorder.conflicts, "lim_reorder_conflicts");
    u!(l.rephase.conflicts, "lim_rephase_conflicts");
    u!(l.restart.conflicts, "lim_restart_conflicts");
    u!(l.glue.conflicts, "lim_glue_conflicts");
    u!(l.glue.interval, "lim_glue_interval");

    // Delays, bounds, tiers, enabled passes.
    let d = &solver.delays;
    u!(d.bumpreasons.count as u64, "delay_bumpreasons_count");
    u!(d.bumpreasons.current as u64, "delay_bumpreasons_current");
    u!(d.congruence.count as u64, "delay_congruence_count");
    u!(d.congruence.current as u64, "delay_congruence_current");
    u!(d.sweep.count as u64, "delay_sweep_count");
    u!(d.sweep.current as u64, "delay_sweep_current");
    u!(d.vivifyirr.count as u64, "delay_vivifyirr_count");
    u!(d.vivifyirr.current as u64, "delay_vivifyirr_current");
    u!(solver.bounds.eliminate.max_bound_completed, "bound_eliminate_max_completed");
    u!(solver.bounds.eliminate.additional_clauses as u64, "bound_eliminate_additional_clauses");
    u!(solver.tier1[0] as u64, "tier1_focused");
    u!(solver.tier1[1] as u64, "tier1_stable");
    u!(solver.tier2[0] as u64, "tier2_focused");
    u!(solver.tier2[1] as u64, "tier2_stable");
    u!(flag(solver.enabled.probe), "enabled_probe");
    u!(flag(solver.enabled.eliminate), "enabled_eliminate");

    // Search state.
    u!(flag(solver.stable), "stable");
    u!(solver.level as u64, "level");
    u!(solver.trail.len() as u64, "trail");
    u!(solver.propagate as u64, "propagate");
    u!(solver.unassigned as u64, "unassigned");
    u!(solver.active as u64, "active");
    u!(solver.vars as u64, "vars");
    u!(solver.unflushed as u64, "unflushed");
    u!(solver.best_assigned as u64, "best_assigned");
    u!(solver.target_assigned as u64, "target_assigned");
    u!(solver.randec as u64, "randec");
    u!(flag(solver.inconsistent), "inconsistent");
    u!(flag(solver.classification.small), "class_small");
    u!(flag(solver.classification.bigbig), "class_bigbig");
    u!(solver.arena.size_wards(), "arena_wards");
    u!(solver.arena.capacity_wards(), "arena_capacity_wards");
    u!(solver.first_reducible as u64, "first_reducible");
    u!(solver.last_irredundant as u64, "last_irredundant");
    u!(solver.vectors.stack.len() as u64, "vectors_stack");
    u!(solver.vectors.usable, "vectors_usable");
    u!(solver.mode.ticks, "mode_ticks");
    u!(solver.mode.conflicts, "mode_conflicts");
    u!(solver.last.ticks.eliminate, "last_ticks_eliminate");
    u!(solver.last.ticks.probe, "last_ticks_probe");
    u!(solver.last.conflicts.reduce, "last_conflicts_reduce");
    let r = &solver.reluctant;
    u!(flag(r.limited), "reluctant_limited");
    u!(flag(r.trigger), "reluctant_trigger");
    u!(r.period, "reluctant_period");
    u!(r.wait, "reluctant_wait");
    u!(r.u, "reluctant_u");
    u!(r.v, "reluctant_v");
    u!(r.limit, "reluctant_limit");
    fl!(solver.scinc, "scinc");
    u!(solver.walked as u64, "walked");

    // Per-epoch learned-clause quality (reset after every row).
    for (i, n) in p.epoch_glue.iter().enumerate() {
        u!(*n, "epoch_glue_bin{}", i);
    }
    u!(p.epoch_learned, "epoch_learned");
    u!(p.epoch_learned_size, "epoch_learned_size");
    u!(p.epoch_learned_glue, "epoch_learned_glue");

    // The observation (policy_obs.rs): whether this row is a boundary
    // whose snapshot enters the ring (0 on the terminal row and on a fork
    // child's first row), the horizon, and the vector itself.
    u!(flag(clock.boundary), "row_boundary");
    fl!(p.obs_state.horizon_value, "horizon");
    u!(flag(p.obs_state.horizon_valid), "horizon_valid");
    if want_names {
        for (i, name) in crate::policy_obs::names().iter().enumerate() {
            fl!(p.obs_state.obs.get(i).copied().unwrap_or(0.0) as f64, "obs_{}", name);
        }
    } else {
        for v in p.obs_state.obs.iter() {
            fl!(*v as f64, "");
        }
    }

    // Per-timer stock counterfactual and policy bookkeeping.
    for t in Timer::ALL {
        let ts = &p.timers[t as usize];
        let name = t.name();
        u!(ts.last_fire, "tm_{}_last_fire", name);
        u!(ts.stock_delta, "tm_{}_stock_delta", name);
        u!(ts.fires, "tm_{}_fires", name);
        u!(crate::policy::effective_limit(solver, t), "tm_{}_eff_limit", name);
        u!(flag(crate::policy::stock_would_fire(solver, t)), "tm_{}_stock_would_fire", name);
    }

    // The action in force now (`act_*`; a consumed one-shot reads 1) and
    // the action as decided at the last decision (`dec_*`, the training
    // label). Rows are written at the boundary before that boundary's
    // decision, so a row describes the epoch that just ended and the
    // decision that governed it.
    for (prefix, a) in [("act", &p.act), ("dec", &p.decided)] {
        for t in Timer::ALL {
            fl!(a.interval_mult[t as usize] as f64, "{}_interval_{}", prefix, t.name());
        }
        fl!(a.restart_margin as f64, "{}_restart_margin", prefix);
        for (i, e) in EFFORT_NAMES.iter().enumerate() {
            fl!(a.effort_mult[i] as f64, "{}_effort_{}", prefix, e);
        }
        u!(a.elim_bound as u64, "{}_elim_bound", prefix);
        fl!(a.reduce_fraction as f64, "{}_reduce_fraction", prefix);
    }
    u!(p.rng, "policy_rng");
    u!(p.segment_left, "policy_segment_left");

    // The net's scores at the last decision (policy_net.rs), head by head;
    // zeros without a net. `net_deviations` counts decisions whose masked
    // action was not stock.
    {
        let menus = crate::policy_net::head_menus();
        let mut pos = 0;
        for (k, (n, _)) in menus.iter().enumerate() {
            for j in 0..*n {
                let v = p.net.as_ref().map_or(0.0, |net| net.scores.get(pos + j).copied().unwrap_or(0.0));
                fl!(v, "net_{}_{}", crate::policy_net::HEAD_NAMES[k], j);
            }
            pos += n;
        }
    }
    u!(p.net_deviations, "net_deviations");
}

// ---------------------------------------------------------------------------
// Header, rows, footer
// ---------------------------------------------------------------------------

pub fn json_escape_into(s: &str, out: &mut String) {
    json_escape(s, out)
}

fn json_escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The header line: configuration plus the column list.
fn header_json(solver: &Solver, columns: &[(String, Kind)]) -> String {
    let p = &solver.policy;
    let mut s = String::with_capacity(16 << 10);
    s.push_str(&format!(
        "{{\"format\":{},\"solver\":\"kissat-rs 4.0.4 port\",\"k_res\":{},\"cnf\":",
        FORMAT,
        crate::statistics::K_RES
    ));
    json_escape(&p.cnf_path, &mut s);
    let horizon_budget = p.horizon.budget();
    s.push_str(&format!(
        ",\"pid\":{},\"policy\":{{\"mode\":\"{}\",\"obs_ticks\":{},\"dec_ticks\":{},\"seed\":{},\"temp\":{},\"segment_mean\":{},\"horizon\":\"{}\",\"horizon_budget\":{},\"k_res\":{}}}",
        std::process::id(),
        p.mode.name(),
        p.obs_ticks,
        p.dec_ticks,
        p.seed,
        p.temp,
        p.segment_mean,
        p.horizon.name(),
        horizon_budget,
        crate::statistics::K_RES
    ));
    s.push_str(",\"options\":{");
    for (i, o) in crate::options::OPTION_TABLE.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "\"{}\":{}",
            o.name,
            crate::options::options_get(&solver.options, o.name)
        ));
    }
    s.push_str("},\"branch\":");
    s.push_str(&p.branching.child_json);
    s.push_str(",\"net\":");
    match &p.net {
        Some(net) => s.push_str(&crate::policy_net::header_json(net, p.margin)),
        None => s.push_str("null"),
    }
    s.push_str(",\"static_schema\":");
    s.push_str(&crate::policy_static::schema_json());
    s.push_str(",\"columns\":[");
    for (i, (name, _)) in columns.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        json_escape(name, &mut s);
    }
    s.push_str("],\"kinds\":\"");
    for (_, kind) in columns {
        s.push(if *kind == Kind::F { 'f' } else { 'u' });
    }
    s.push_str(&format!("\",\"row_bytes\":{}}}", 8 * columns.len()));
    s
}

/// Write the magic line and the header. Called once, right after the
/// options and the input path are known and before parsing (normal path,
/// allocation is fine here), so that from then on a terminal row and the
/// footer can be written without allocating, even from the signal handler
/// during parsing or preprocessing. `row()` falls back to writing the
/// header lazily if this was never called.
pub fn prepare(solver: &mut Solver) {
    let _guard = SignalGuard::block();
    let Some(mut log) = solver.policy.log.take() else {
        return;
    };
    if !log.header_written {
        write_header(solver, &mut log);
    }
    solver.policy.log = Some(log);
}

fn write_header(solver: &Solver, log: &mut Logger) {
    let clock = RowClock::default();
    let mut cols: Vec<(String, Kind)> = Vec::with_capacity(512);
    columns(solver, &clock, true, |name, kind, _| cols.push((name.to_string(), kind)));
    let header = header_json(solver, &cols);
    log.put(MAGIC.as_bytes());
    log.put(b"\n");
    log.put(header.as_bytes());
    log.put(b"\n");
    log.row_width = cols.len();
    log.buf.reserve(cols.len());
    log.header_written = true;
}

/// Print the one warning for a failed log, the first time it is seen.
fn warn_once(solver: &Solver, log: &mut Logger) {
    if log.warned {
        return;
    }
    if let Some(code) = log.error {
        log.warned = true;
        // Only the numeric code: rendering an OS error text allocates, and
        // this can run from the signal handler.
        crate::print::warning(
            solver,
            format_args!(
                "policy log '{}' failed and is incomplete (os error {})",
                log.path, code
            ),
        );
    }
}

/// Write one row. Called by `policy::log_row` at every observation epoch
/// and once more by `finish` for the terminal state. Asynchronous signals
/// are blocked for the duration (see `SignalGuard`). With `safe_only`
/// (the signal-handler path) a log whose header is not written yet is left
/// alone: writing the header allocates, and there are no rows to save.
pub fn row(solver: &mut Solver, safe_only: bool) {
    let _guard = SignalGuard::block();
    let Some(mut log) = solver.policy.log.take() else {
        return;
    };
    if log.finished || log.error.is_some() || (safe_only && !log.header_written) {
        solver.policy.log = Some(log);
        return;
    }
    // A fresh observation (built by `policy::epoch` for this boundary)
    // supplies the wall reading; otherwise (the terminal row, a fork
    // child's first row) the clock is read here and the observation is
    // built from the current state, without touching the ring.
    let boundary = solver.policy.obs_state.obs_fresh;
    let wall = if boundary {
        solver.policy.obs_state.obs_wall_ns
    } else {
        let w = wall_ns().saturating_sub(log.wall0);
        crate::policy_obs::observe(solver, w);
        w
    };
    solver.policy.obs_state.obs_fresh = false;
    let clock = RowClock {
        row: log.rows,
        wall_ns: wall,
        cpu_ns: cpu_ns(),
        boundary,
    };
    if !log.header_written {
        write_header(solver, &mut log);
    }
    let mut buf = std::mem::take(&mut log.buf);
    buf.clear();
    columns(solver, &clock, false, |_, _, v| buf.push(v));
    debug_assert_eq!(buf.len(), log.row_width);
    for v in &buf {
        log.put(&v.to_le_bytes());
    }
    log.buf = buf;
    log.rows += 1;
    if !safe_only {
        warn_once(solver, &mut log);
    }
    solver.policy.log = Some(log);
    // The per-epoch learned-clause accumulators are reset by
    // `policy::epoch` at every boundary, with or without a log, so the
    // observation (and a net's decisions) cannot depend on logging.
}

/// Final row, sentinel and footer. `reason` is "solve" on the normal exit
/// path and "signal" from the signal handler; a second call is a no-op.
/// After a write failure the footer is withheld on purpose: an incomplete
/// file is how a reader learns the trace is not usable. On the signal path
/// nothing here allocates once the header exists: the sentinel reuses the
/// row buffer and the footer is formatted into a preallocated string; a
/// log with no header yet is skipped (nothing to save).
pub fn finish(solver: &mut Solver, res: i32, reason: &str) {
    let _guard = SignalGuard::block();
    let safe_only = reason == "signal";
    if solver.policy.log.as_ref().map_or(true, |l| l.finished || (safe_only && !l.header_written)) {
        return;
    }
    row(solver, safe_only);
    let Some(mut log) = solver.policy.log.take() else {
        return;
    };
    let width = log.row_width.max(1);
    let mut buf = std::mem::take(&mut log.buf);
    buf.clear();
    buf.resize(width, 0);
    buf[0] = u64::MAX;
    for v in &buf {
        log.put(&v.to_le_bytes());
    }
    log.buf = buf;
    let result = match res {
        10 => "SATISFIABLE",
        20 => "UNSATISFIABLE",
        _ => "UNKNOWN",
    };
    let mut footer = std::mem::take(&mut log.footer);
    footer.clear();
    {
        use std::fmt::Write as _;
        let _ = write!(
            footer,
            "{{\"result\":\"{}\",\"exit\":{},\"reason\":\"{}\",\"rows\":{},\"wall_ns\":{},\"cpu_ns\":{},\"peak_rss_bytes\":{},\"work\":{},\"conflicts\":{},\"static\":",
            result,
            res,
            reason,
            log.rows,
            wall_ns().saturating_sub(log.wall0),
            cpu_ns(),
            crate::resources::maximum_resident_set_size(),
            solver.statistics.work_clock(),
            solver.statistics.conflicts
        );
    }
    log.put(footer.as_bytes());
    // The static features (step A.7) are preformatted on the normal path
    // into `policy.static_json`; "{}" until then. So are the fork-mode
    // records: the children this parent forked and, in a child, its own
    // branch block.
    log.put(solver.policy.static_json.as_bytes());
    log.put(b",\"branches\":[");
    log.put(solver.policy.branching.json.as_bytes());
    log.put(b"],\"branch\":");
    log.put(solver.policy.branching.child_json.as_bytes());
    log.put(b"}\n");
    log.footer = footer;
    log.flush_checked();
    log.finished = true;
    if !safe_only {
        // Printing takes the stdout lock, which the code a signal
        // interrupted may hold; a footer that is missing already tells a
        // reader the log failed.
        warn_once(solver, &mut log);
    }
    solver.policy.log = Some(log);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glue_bins_cover_the_range_in_order() {
        let mut last = 0;
        for g in 0..100u32 {
            let b = glue_bin(g);
            assert!(b >= last && b < GLUE_BINS);
            last = b;
        }
        assert_eq!(glue_bin(1), 0);
        assert_eq!(glue_bin(2), 1);
        assert_eq!(glue_bin(6), 4);
        assert_eq!(glue_bin(7), 5);
        assert_eq!(glue_bin(20), 6);
        assert_eq!(glue_bin(21), 7);
    }

    #[test]
    fn column_names_are_unique_and_match_kinds() {
        let mut s = crate::internal::init();
        s.statistics.searches = 1;
        s.policy.on = true;
        crate::kimits::init_limits(&mut s);
        let mut names: Vec<String> = Vec::new();
        let mut kinds: Vec<Kind> = Vec::new();
        let mut values: Vec<u64> = Vec::new();
        columns(&s, &RowClock::default(), true, |n, k, v| {
            names.push(n.to_string());
            kinds.push(k);
            values.push(v);
        });
        assert_eq!(names.len(), kinds.len());
        assert_eq!(names.len(), values.len());
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate column");
        assert!(names.len() > 300, "{} columns", names.len());
        let i = names.iter().position(|n| n == "act_restart_margin").unwrap();
        assert_eq!(kinds[i], Kind::F);
        assert_eq!(f64::from_bits(values[i]), 1.0);
        let i = names.iter().position(|n| n == "lim_reduce_conflicts").unwrap();
        assert_eq!(values[i], s.limits.reduce.conflicts);
        let header = header_json(&s, &names.iter().cloned().zip(kinds.iter().cloned()).collect::<Vec<_>>());
        assert!(header.starts_with("{\"format\":1,"));
        assert!(header.ends_with(&format!("\"row_bytes\":{}}}", 8 * names.len())));
        assert!(header.contains("\"eliminateint\":500"));
    }

    #[test]
    fn same_path_follows_dangling_symlinks_and_missing_files() {
        let dir = std::env::temp_dir().join(format!("sat13-same-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let proof = dir.join("proof.out"); // does not exist yet
        let link = dir.join("run.log"); // dangling link to it
        std::os::unix::fs::symlink("proof.out", &link).unwrap();
        let (p, l) = (proof.to_str().unwrap(), link.to_str().unwrap());
        assert!(same_path(l, p), "dangling symlink to a future file");
        assert!(same_path(p, l));
        let other = dir.join("other.log");
        assert!(!same_path(other.to_str().unwrap(), p));
        // A relative spelling of an existing file through a different parent.
        std::fs::write(&proof, b"x").unwrap();
        let dotted = dir.join(".").join("proof.out");
        assert!(same_path(dotted.to_str().unwrap(), p));
        assert!(standard_stream_behind(p).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_escape_handles_quotes_and_control_characters() {
        let mut s = String::new();
        json_escape("a\"b\\c\n\u{1}", &mut s);
        assert_eq!(s, "\"a\\\"b\\\\c\\n\\u0001\"");
    }
}
