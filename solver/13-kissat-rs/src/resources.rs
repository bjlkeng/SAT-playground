// Port of src/resources.h + src/resources.c (kissat 4.0.4).
//
// PORT NOTE: wall_clock_time / process_time / maximum_resident_set_size use
// libc gettimeofday / getrusage exactly as the C does.  process_time in
// particular is on the profile START/STOP path (hundreds of thousands of
// calls per run at --profile>=3): an earlier libc-free version parsed
// /proc/self/stat per call, which swamped small profiles (decide showed 10x
// the reference cost on brocard, 2026-09-03) — do not regress this to file
// I/O.  current_resident_set_size keeps the /proc/self/statm read (the C
// reads the same file; only the page-size sysconf is hardcoded to 4096, the
// x86-64/aarch64 Linux default).

use crate::format::{self, Format};
use crate::internal::Solver;
use crate::profile;
use std::io::Write as _;

// kissat_wall_clock_time
pub fn wall_clock_time() -> f64 {
    let mut tv = libc::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    if unsafe { libc::gettimeofday(&mut tv, std::ptr::null_mut()) } != 0 {
        return 0.0;
    }
    1e-6 * tv.tv_usec as f64 + tv.tv_sec as f64
}

// kissat_process_time
pub fn process_time() -> f64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) } != 0 {
        return 0.0;
    }
    let mut res = u.ru_utime.tv_sec as f64 + 1e-6 * u.ru_utime.tv_usec as f64;
    res += u.ru_stime.tv_sec as f64 + 1e-6 * u.ru_stime.tv_usec as f64;
    res
}

// kissat_maximum_resident_set_size
pub fn maximum_resident_set_size() -> u64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) } != 0 {
        return 0;
    }
    (u.ru_maxrss as u64) << 10
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
