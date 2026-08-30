// Port of src/error.c + src/error.h (kissat 4.0.4), plus the minimal
// colors.h/colors.c terminal surface these messages need.
//
// PORT NOTE: Message format is exactly error.c's: flush stdout, then to
// stderr `kissat: <type>: <message>\n` with BOLD before "kissat: ", RED
// around "<type>: " and NORMAL reset before the message — colors only when
// stderr is connected to a terminal (isatty), cached like
// `kissat_is_terminal[3] = {0, -1, -1}` and overridable via
// force_colors/force_no_colors (colors.c).
// PORT NOTE: C's `kissat_error` prints and returns; kissat's application
// layer then exits with status 1. The `die!` macro bundles print + exit(1)
// as the Rust CLI equivalent; the bare printing function is also exported
// as `error` for call sites that manage control flow themselves.
// PORT NOTE: `kissat_fatal` prints and calls `kissat_abort` (abort(3), so
// SIGABRT) — `fatal` / `fatal_error!` do the same via std::process::abort.
// The `kissat_call_function_instead_of_abort` test hook is not ported
// (only used by the C test harness).

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicI8, Ordering};

// colors.h codes (the subset used here).
pub const BOLD: &str = "\x1b[1m";
pub const RED: &str = "\x1b[31m";
pub const NORMAL: &str = "\x1b[0m";

// colors.c: int kissat_is_terminal[3] = {0, -1, -1};  (-1 = undetermined)
static IS_TERMINAL: [AtomicI8; 3] = [AtomicI8::new(0), AtomicI8::new(-1), AtomicI8::new(-1)];

// kissat_initialize_terminal
fn initialize_terminal(fd: usize) -> i8 {
    debug_assert!(fd == 1 || fd == 2);
    let res = match fd {
        1 => std::io::stdout().is_terminal(),
        2 => std::io::stderr().is_terminal(),
        _ => false,
    } as i8;
    IS_TERMINAL[fd].store(res, Ordering::Relaxed);
    res
}

// kissat_connected_to_terminal (colors.h inline)
pub fn connected_to_terminal(fd: usize) -> bool {
    debug_assert!(fd == 1 || fd == 2);
    let mut res = IS_TERMINAL[fd].load(Ordering::Relaxed);
    if res < 0 {
        res = initialize_terminal(fd);
    }
    res != 0
}

// kissat_force_colors
pub fn force_colors() {
    IS_TERMINAL[1].store(1, Ordering::Relaxed);
    IS_TERMINAL[2].store(1, Ordering::Relaxed);
}

// kissat_force_no_colors
pub fn force_no_colors() {
    IS_TERMINAL[1].store(0, Ordering::Relaxed);
    IS_TERMINAL[2].store(0, Ordering::Relaxed);
}

// typed_error_message_start (static in error.c)
fn typed_error_message_start(err: &mut impl Write, type_: &str) {
    let _ = std::io::stdout().flush();
    let connected = connected_to_terminal(2);
    if connected {
        let _ = err.write_all(BOLD.as_bytes());
    }
    let _ = err.write_all(b"kissat: ");
    if connected {
        let _ = err.write_all(RED.as_bytes());
    }
    let _ = err.write_all(type_.as_bytes());
    let _ = err.write_all(b": ");
    if connected {
        let _ = err.write_all(NORMAL.as_bytes());
    }
}

// kissat_fatal_message_start
pub fn fatal_message_start() {
    let stderr = std::io::stderr();
    let mut err = stderr.lock();
    typed_error_message_start(&mut err, "fatal error");
    let _ = err.flush();
}

// vprint_error (static in error.c)
fn vprint_error(type_: &str, args: std::fmt::Arguments) {
    let stderr = std::io::stderr();
    let mut err = stderr.lock();
    typed_error_message_start(&mut err, type_);
    let _ = err.write_fmt(args);
    let _ = err.write_all(b"\n");
    let _ = err.flush();
}

// kissat_error (print only; see PORT NOTE above)
pub fn error(args: std::fmt::Arguments) {
    vprint_error("error", args);
}

// kissat_abort
pub fn abort() -> ! {
    std::process::abort();
}

// kissat_fatal
pub fn fatal(args: std::fmt::Arguments) -> ! {
    vprint_error("fatal error", args);
    abort();
}

// print + exit(1): the application-layer ERROR pattern around kissat_error.
pub fn die(args: std::fmt::Arguments) -> ! {
    vprint_error("error", args);
    std::process::exit(1);
}

/// `kissat: fatal error: <formatted message>` to stderr, then abort(3).
#[macro_export]
macro_rules! fatal_error {
    ($($arg:tt)*) => {
        $crate::error::fatal(format_args!($($arg)*))
    };
}

/// `kissat: error: <formatted message>` to stderr, then exit(1).
#[macro_export]
macro_rules! die {
    ($($arg:tt)*) => {
        $crate::error::die(format_args!($($arg)*))
    };
}
