// Port of src/resources.h + src/resources.c (kissat 4.0.4).
//
// PORT NOTE (libc-free deviations, values only differ in the unobservable
// time/memory readings, never in solver trajectory or line formats):
// - kissat_wall_clock_time: C uses gettimeofday. Rust uses
//   SystemTime::now() since UNIX_EPOCH — same value semantics (seconds since
//   the epoch), higher precision.
// - kissat_process_time: C uses getrusage(RUSAGE_SELF) user+system CPU time.
//   Rust reads /proc/self/stat fields utime(14) + stime(15) (also
//   user+system CPU of the process) divided by USER_HZ. USER_HZ is fixed at
//   100 on Linux (sysconf(_SC_CLK_TCK) needs libc); granularity is 10 ms
//   versus getrusage's ~1 us. Falls back to 0.0 on failure like C.
// - kissat_maximum_resident_set_size: C uses getrusage ru_maxrss << 10.
//   Rust reads VmHWM from /proc/self/status (same kernel counter that feeds
//   ru_maxrss) and multiplies the kB value by 1024.
// - kissat_current_resident_set_size: C reads /proc/<pid>/statm and
//   multiplies by sysconf(_SC_PAGESIZE). Rust reads /proc/self/statm and
//   uses 4096 as the page size (the default on x86-64/aarch64 Linux).

use crate::format::{self, Format};
use crate::internal::Solver;
use crate::profile;
use std::io::Write as _;

// kissat_wall_clock_time
pub fn wall_clock_time() -> f64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(_) => 0.0,
    }
}

// kissat_process_time
pub fn process_time() -> f64 {
    const USER_HZ: f64 = 100.0;
    let stat = match std::fs::read_to_string("/proc/self/stat") {
        Ok(s) => s,
        Err(_) => return 0.0,
    };
    // comm (field 2) may contain spaces/parens: parse after the last ')'.
    let rest = match stat.rfind(')') {
        Some(i) => &stat[i + 1..],
        None => return 0.0,
    };
    let fields: Vec<&str> = rest.split_ascii_whitespace().collect();
    // fields[0] is stat field 3 (state); utime is field 14, stime field 15.
    let utime: u64 = match fields.get(11).and_then(|s| s.parse().ok()) {
        Some(v) => v,
        None => return 0.0,
    };
    let stime: u64 = match fields.get(12).and_then(|s| s.parse().ok()) {
        Some(v) => v,
        None => return 0.0,
    };
    (utime + stime) as f64 / USER_HZ
}

// kissat_maximum_resident_set_size
pub fn maximum_resident_set_size() -> u64 {
    let status = match std::fs::read_to_string("/proc/self/status") {
        Ok(s) => s,
        Err(_) => return 0,
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            let kb: u64 = match rest.trim().trim_end_matches(" kB").trim().parse() {
                Ok(v) => v,
                Err(_) => return 0,
            };
            return kb << 10;
        }
    }
    0
}

// kissat_current_resident_set_size
pub fn current_resident_set_size() -> u64 {
    const PAGE_SIZE: u64 = 4096;
    let statm = match std::fs::read_to_string("/proc/self/statm") {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let mut it = statm.split_ascii_whitespace();
    let _dummy = it.next();
    match it.next().and_then(|s| s.parse::<u64>().ok()) {
        Some(rss) => rss * PAGE_SIZE,
        None => 0,
    }
}

// kissat_print_resources
//
// C format (SFW1=30, SFW2=12, SFW3=5, SFW4=10):
//   printf ("%s%-30s %12" PRIu64 " %-5s %10.0f MB\n",
//           prefix, "maximum-resident-set-size:", rss, "bytes", rss/2^20);
//   printf ("%sprocess-time: %30s %18.2f seconds\n",
//           prefix, kissat_format_time (&buffer, t), t);
pub fn print_resources(solver: &mut Solver) {
    let rss = maximum_resident_set_size();
    let t = profile::time(solver);
    println!(
        "{}{:<30} {:>12} {:<5} {:>10.0} MB",
        solver.prefix,
        "maximum-resident-set-size:",
        rss,
        "bytes",
        rss as f64 / (1u64 << 20) as f64
    );
    {
        let mut buffer = Format::default();
        let formatted = format::format_time(&mut buffer, t);
        println!(
            "{}process-time: {:>30} {:>18.2} seconds",
            solver.prefix, formatted, t
        );
    }
    std::io::stdout().flush().ok();
}
