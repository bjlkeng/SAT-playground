// Port of src/print.h + print.c (kissat 4.0.4), QUIET undefined build.
//
// C variadic printf APIs become fns taking `std::fmt::Arguments` (call sites
// use `format_args!(...)`, or the phase!/verbose! style macros below).

use crate::colors::{connected_to_terminal, BLUE, BOLD, CYAN, DARK_GRAY, LIGHT_GRAY, NORMAL, RED,
                    YELLOW};
use crate::internal::Solver;
use std::io::Write;

#[inline]
pub fn verbosity(solver: &Solver) -> i32 {
    if solver.options.quiet != 0 {
        return -1;
    }
    solver.options.verbose
}

fn color(out: &mut impl Write, tty: bool, code: &str) {
    if tty {
        let _ = out.write_all(code.as_bytes());
    }
}

pub fn warning(solver: &Solver, args: impl std::fmt::Display) {
    if verbosity(solver) < 0 {
        return;
    }
    let tty = connected_to_terminal(1);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = out.write_all(solver.prefix.as_bytes());
    color(&mut out, tty, BOLD);
    color(&mut out, tty, YELLOW);
    let _ = out.write_all(b"warning:");
    color(&mut out, tty, NORMAL);
    let _ = write!(out, " {}\n", args);
}

pub fn signal_msg(solver: &Solver, type_: &str, sig: i32, name: &str) {
    if verbosity(solver) < 0 {
        return;
    }
    let tty = connected_to_terminal(1);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = out.write_all(solver.prefix.as_bytes());
    color(&mut out, tty, BOLD);
    color(&mut out, tty, RED);
    let _ = write!(out, "{} signal {} ({})", type_, sig, name);
    color(&mut out, tty, NORMAL);
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

fn print_message(solver: &Solver, code: &str, args: impl std::fmt::Display) {
    let tty = connected_to_terminal(1);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = out.write_all(solver.prefix.as_bytes());
    color(&mut out, tty, code);
    let _ = write!(out, "{}\n", args);
    color(&mut out, tty, NORMAL);
    let _ = out.flush();
}

fn print_line(solver: &Solver) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let bytes = solver.prefix.as_bytes();
    for (i, &ch) in bytes.iter().enumerate() {
        // C: skip a trailing single space of the prefix.
        if ch != b' ' || i + 1 < bytes.len() {
            let _ = out.write_all(&[ch]);
        }
    }
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

pub fn message(solver: &Solver, args: impl std::fmt::Display) {
    if verbosity(solver) < 0 {
        return;
    }
    print_message(solver, "", args);
}

pub fn line(solver: &Solver) {
    if verbosity(solver) >= 0 {
        print_line(solver);
    }
}

pub fn prefix(solver: &Solver) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = out.write_all(solver.prefix.as_bytes());
}

pub fn verbose(solver: &Solver, args: impl std::fmt::Display) {
    if verbosity(solver) < 1 {
        return;
    }
    print_message(solver, LIGHT_GRAY, args);
}

pub fn very_verbose(solver: &Solver, args: impl std::fmt::Display) {
    if verbosity(solver) < 2 {
        return;
    }
    print_message(solver, DARK_GRAY, args);
}

pub fn extremely_verbose(solver: &Solver, args: impl std::fmt::Display) {
    if verbosity(solver) < 3 {
        return;
    }
    print_message(solver, DARK_GRAY, args);
}

pub fn section(solver: &mut Solver, name: &str) {
    if verbosity(solver) < 0 {
        return;
    }
    let tty = connected_to_terminal(1);
    if solver.sectioned {
        line(solver);
    } else {
        solver.sectioned = true;
    }
    {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        let _ = out.write_all(solver.prefix.as_bytes());
        color(&mut out, tty, BLUE);
        let _ = out.write_all(b"---- [ ");
        color(&mut out, tty, BOLD);
        color(&mut out, tty, BLUE);
        let _ = out.write_all(name.as_bytes());
        color(&mut out, tty, NORMAL);
        color(&mut out, tty, BLUE);
        let _ = out.write_all(b" ] ");
        for _ in name.len()..66 {
            let _ = out.write_all(b"-");
        }
        color(&mut out, tty, NORMAL);
        let _ = out.write_all(b"\n");
    }
    print_line(solver);
    let _ = std::io::stdout().flush();
}

pub fn phase(solver: &Solver, name: &str, count: u64, args: impl std::fmt::Display) {
    if verbosity(solver) < 1 {
        return;
    }
    let tty = connected_to_terminal(1);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = out.write_all(solver.prefix.as_bytes());
    if solver.stable {
        color(&mut out, tty, CYAN);
    } else {
        color(&mut out, tty, BOLD);
        color(&mut out, tty, CYAN);
    }
    let _ = write!(out, "[{}", name);
    if count != u64::MAX {
        let _ = write!(out, "-{}", count);
    }
    let _ = out.write_all(b"] ");
    let _ = write!(out, "{}", args);
    color(&mut out, tty, NORMAL);
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}
