// Not in kissat: memory room under the process's limit (epic
// SAT-playground-qu3, bead qu3.2; solver README, "Memory").
//
// kissat grows every table by doubling and the process dies when an
// allocation fails.  For the tables the search cannot do without (clauses,
// watches, the per-variable arrays) there is nothing else to do.  A pass
// whose tables are optional can do better: ask first whether there is room,
// and step aside when there is not, so the run goes on without that pass.
// The first pass that asks is the congruence pass (src/congruence.rs): on
// `pj2002_k500` (44.9M variables, 39M AND gates) its gate table doubling
// from 1.5 to 3 GiB was the allocation that failed under the 16 GB limit.
//
// The limit is the smaller of
//   - the address-space limit of the process (RLIMIT_AS: what `ulimit -v`
//     and the harness's --mem-mb set), and
//   - the option --memlimit, megabytes (0 = none).  It tells the solver
//     about a limit it cannot see (a container's, a watcher's); nothing
//     enforces it.
// What is in use is the address space the process holds (VmSize, the first
// field of /proc/self/statm): the quantity RLIMIT_AS counts, capacity that
// was reserved and not yet touched included.  A sixteenth of the limit is
// kept free for everything that does not ask first.
//
// With no limit the answer is always "yes", and with room to spare it is
// "yes" too: a run that was not near its limit does exactly what it did
// before, step for step (the questions cost a read of /proc at each
// doubling of a table, no work on the work clock and no counter).
//
// The limit is the process's, not a solver's.  The structure passes and the
// control split start helper solvers with options of their own; they share
// the address space of the run, so their passes ask the same question
// against the same limit.  The run's --memlimit is therefore kept here, for
// the process (`tell`), and no question looks at a solver's options.  A
// helper's pass that steps aside shows in the count of refused questions of
// the exit line; the line's other fields are about the run's own pass.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::internal::Solver;

/// The limit the run was told of (--memlimit), in bytes; 0 = none.
static TOLD: AtomicU64 = AtomicU64::new(0);
/// The questions that were answered "no", in this process.
static REFUSED: AtomicU64 = AtomicU64::new(0);
/// The last of them: the bytes asked for and the address space in use.
static LAST_WANTED: AtomicU64 = AtomicU64::new(0);
static LAST_USED: AtomicU64 = AtomicU64::new(0);

/// The run's --memlimit in megabytes (0 = none), for the whole process.
pub fn tell(megabytes: i32) {
    TOLD.store((megabytes.max(0) as u64) << 20, Ordering::Relaxed);
}

/// What the run remembers about the passes that stepped aside.
#[derive(Default)]
pub struct State {
    /// The congruence pass found no room and is off for the rest of the run.
    pub congruence_off: bool,
    /// The gates it held when it stopped (0: it never started).
    pub gates: u64,
    /// The bytes it asked for, the address space in use and the limit then.
    pub wanted: u64,
    pub used: u64,
    pub limit: u64,
    /// The gates it was asked to make after it had stopped.  Every loop that
    /// makes gates ends when the room is gone, so this stays 0; the exit
    /// line shows it when it does not (tests/room.rs holds it at none).
    pub late: u64,
}

/// The soft RLIMIT_AS of the process in bytes; None when it has none.
fn address_space_limit() -> Option<u64> {
    static LIMIT: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        let mut r = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut r) } != 0 {
            return None;
        }
        if r.rlim_cur == libc::RLIM_INFINITY {
            None
        } else {
            Some(r.rlim_cur as u64)
        }
    })
}

/// The smaller of the process's own limit and the one it was told of.
fn smaller(own: Option<u64>, told: u64) -> Option<u64> {
    let told = if told == 0 { None } else { Some(told) };
    match (own, told) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// The limit the passes budget against, in bytes; None when there is none.
pub fn limit() -> Option<u64> {
    smaller(address_space_limit(), TOLD.load(Ordering::Relaxed))
}

/// The address space the process holds now, in bytes.
pub fn in_use() -> Option<u64> {
    // "size resident shared text lib data dt", in pages
    let text = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = text.split_ascii_whitespace().next()?.parse().ok()?;
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page <= 0 {
        return None;
    }
    pages.checked_mul(page as u64)
}

/// The part of the limit kept free for everything that does not ask first.
fn reserve(limit: u64) -> u64 {
    limit / 16
}

/// Whether `bytes` more fit under `limit` with the reserve left free, when
/// `used` counts as in use.
fn fits(used: u64, bytes: u64, limit: u64) -> bool {
    used.saturating_add(bytes).saturating_add(reserve(limit)) <= limit
}

/// `Ok` when `bytes` more of address space still leave the reserve free;
/// otherwise what is in use and the limit, for the report.  Always `Ok`
/// without a limit, and when the use cannot be read.
pub fn ask(bytes: u64) -> Result<(), (u64, u64)> {
    ask_above(0, bytes)
}

/// `ask` for a pass that was promised room before it started and takes it
/// piece by piece.  `floor` is the address space that was in use when the
/// promise was made, plus the promise, plus what the pass was given on top
/// of it since.  What counts as in use is at least that: the part of the
/// promise that is not taken yet stays free for the tables it was made for.
/// The use in an `Err` is what counted.
pub fn ask_above(floor: u64, bytes: u64) -> Result<(), (u64, u64)> {
    let Some(limit) = limit() else {
        return Ok(());
    };
    let Some(used) = in_use() else {
        return Ok(());
    };
    let used = used.max(floor);
    if fits(used, bytes, limit) {
        Ok(())
    } else {
        REFUSED.fetch_add(1, Ordering::Relaxed);
        LAST_WANTED.store(bytes, Ordering::Relaxed);
        LAST_USED.store(used, Ordering::Relaxed);
        Err((used, limit))
    }
}

/// The exit line; nothing when no question was refused.  `refused` counts
/// the questions answered "no" in the whole process, the helper solvers'
/// included; the rest is about the run's own congruence pass.
pub fn report(solver: &Solver) {
    let refused = REFUSED.load(Ordering::Relaxed);
    if refused == 0 {
        return;
    }
    let s = &solver.room;
    if !s.congruence_off {
        // only helper solvers were refused: the last such question
        println!(
            "c room refused={} congruence=on last_wanted_mb={} last_used_mb={} limit_mb={}",
            refused,
            (LAST_WANTED.load(Ordering::Relaxed) + (1 << 20) - 1) >> 20,
            LAST_USED.load(Ordering::Relaxed) >> 20,
            limit().unwrap_or(0) >> 20
        );
        return;
    }
    let late = if s.late == 0 {
        String::new()
    } else {
        format!(" late={}", s.late)
    };
    println!(
        "c room refused={} congruence=off gates={} wanted_mb={} used_mb={} limit_mb={}{}",
        refused,
        s.gates,
        (s.wanted + (1 << 20) - 1) >> 20, // rounded up: a small table is not "0"
        s.used >> 20,
        s.limit >> 20,
        late
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_use_of_address_space_can_be_read() {
        let used = in_use().expect("/proc/self/statm");
        assert!(used > 1 << 20, "a process holds more than a megabyte: {}", used);
        assert!(used < 1 << 44, "and less than 16 TB: {}", used);
    }

    #[test]
    fn the_smaller_limit_counts() {
        assert_eq!(smaller(None, 0), None);
        assert_eq!(smaller(Some(100), 0), Some(100));
        assert_eq!(smaller(None, 50), Some(50));
        assert_eq!(smaller(Some(100), 50), Some(50));
        assert_eq!(smaller(Some(40), 50), Some(40));
    }

    #[test]
    fn the_reserve_is_a_sixteenth() {
        assert_eq!(reserve(16 << 30), 1 << 30);
        assert_eq!(reserve(0), 0);
    }

    #[test]
    fn the_bytes_fit_while_the_reserve_stays_free() {
        let limit = 16u64 << 30; // the reserve is 1 GiB
        assert!(fits(10 << 30, 5 << 30, limit));
        assert!(!fits(10 << 30, (5 << 30) + 1, limit));
        assert!(!fits(u64::MAX, 1, limit), "no overflow");
    }
}
