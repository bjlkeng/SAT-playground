// Port of src/application.c + src/main.c + src/handle.c (+ the build.c
// banner essentials) (kissat 4.0.4).
//
// Build configuration: NDEBUG defined; QUIET/NOPTIONS/NPROOFS/LOGGING/
// EMBEDDED undefined; KISSAT_HAS_COMPRESSION defined (file.rs has the
// compression pipes).  The `kissat_check_satisfying_assignment` call is
// `#ifndef NDEBUG` only and is omitted.
//
// PORT NOTE: the banner identifies this binary as the Rust port (SOLVER_NAME
// and the build lines); VERSION stays "4.0.4" and all line/color formats
// match build.c exactly.  ID (a git SHA in the reference) is the port
// identifier string.
// PORT NOTE (quirk kept): application.c declares `time_option` but never
// assigns it, so the "multiple '--time'" error is dead code — repeated
// `--time=<n>` options are accepted and the last one wins.
// PORT NOTE (quirk kept): the `ERROR (...)` use inside C `run_application`
// ("could not write DIMACS file") expands to `return false`, i.e. the
// function returns 0 — exit code 0 on that failure path.  Ported as-is.
// PORT NOTE: kissat_force_colors/kissat_force_no_colors act on the single C
// global `kissat_is_terminal[]`; the Rust port has that state duplicated in
// colors.rs (stdout) and error.rs (stderr), so both are forced here.
// PORT NOTE: main.c/handle.c signal machinery uses libc signal(2) through a
// minimal extern "C" shim (the colors.rs isatty pattern); the handler reads
// the solver through a raw pointer exactly like the C static `solver`
// variable.  SIGBUS is present (not __MINGW32__).

use crate::file::{self, File};
use crate::internal::Solver;
use crate::parse::{Strictness, NORMAL_PARSING, PEDANTIC_PARSING, RELAXED_PARSING};
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

const SOLVER_NAME: &str = "Kissat SAT Solver (Rust port)";

// ------------------------------------------------------------------------
// build.c essentials (banner / version strings)
// ------------------------------------------------------------------------

const VERSION: &str = "4.0.4";
const ID: &str = "kissat-rs (Rust port of kissat 4.0.4)";
const COMPILER: &str = "rustc (release profile: fat LTO, codegen-units=1)";
const BUILD: &str = "Rust port, solver/13-kissat-rs";

const COPYRIGHT_LINES: [&str; 2] = [
    "Copyright (c) 2021-2024 Armin Biere University of Freiburg",
    "Copyright (c) 2019-2021 Armin Biere Johannes Kepler University Linz",
];

/// C: `kissat_version`.
pub fn version() -> &'static str {
    VERSION
}

/// C: `kissat_id`.
pub fn id() -> &'static str {
    ID
}

/// C: `kissat_compiler`.
pub fn compiler() -> &'static str {
    COMPILER
}

/// C: `kissat_copyright`.
pub fn copyright() -> &'static [&'static str] {
    &COPYRIGHT_LINES
}

// PREFIX (COLORS) / NL () from build.c: colors only with a prefix AND a
// terminal (C sets `connected_to_terminal = false` when `!prefix`).
fn banner_line(prefix: Option<&str>, color: &str, text: &str) {
    let connected = prefix.is_some() && crate::colors::connected_to_terminal(1);
    if let Some(prefix) = prefix {
        print!("{}", prefix);
    }
    if connected {
        print!("{}", color);
    }
    print!("{}", text);
    println!();
    if connected {
        print!("{}", crate::colors::NORMAL);
    }
}

/// C: `kissat_build`.
pub fn build(prefix: Option<&str>) {
    let magenta = crate::colors::MAGENTA;
    if !ID.is_empty() {
        banner_line(prefix, magenta, &format!("Version {} {}", VERSION, ID));
    } else {
        banner_line(prefix, magenta, &format!("Version {}", VERSION));
    }
    banner_line(prefix, magenta, COMPILER);
    banner_line(prefix, magenta, BUILD);
}

/// C: `kissat_banner`.
pub fn banner(prefix: Option<&str>, name: &str) {
    let bold_magenta = concat!("\x1b[1m", "\x1b[35m");
    banner_line(prefix, bold_magenta, name);
    banner_line(prefix, bold_magenta, "");
    for line in COPYRIGHT_LINES.iter() {
        banner_line(prefix, bold_magenta, line);
    }
    if prefix.is_some() {
        banner_line(prefix, "", "");
    }
    build(prefix);
}

// ------------------------------------------------------------------------
// handle.c — signal and alarm plumbing
// ------------------------------------------------------------------------

const SIGABRT: i32 = 6;
const SIGBUS: i32 = 7;
const SIGINT: i32 = 2;
const SIGSEGV: i32 = 11;
const SIGTERM: i32 = 15;
const SIGALRM: i32 = 14;

// SIGNALS list from handle.h (in that order).
const SIGNALS: [i32; 5] = [SIGABRT, SIGBUS, SIGINT, SIGSEGV, SIGTERM];

extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
    fn raise(sig: i32) -> i32;
    fn alarm(seconds: u32) -> u32;
}

// kissat_signal_name (handle.h inline)
fn signal_name(sig: i32) -> &'static str {
    match sig {
        SIGABRT => "SIGABRT",
        SIGBUS => "SIGBUS",
        SIGINT => "SIGINT",
        SIGSEGV => "SIGSEGV",
        SIGTERM => "SIGTERM",
        SIGALRM => "SIGALRM",
        _ => "SIGUNKNOWN",
    }
}

// static kissat *volatile solver; (main.c)
static SOLVER: AtomicUsize = AtomicUsize::new(0);
// static volatile int caught_signal;
static CAUGHT_SIGNAL: AtomicI32 = AtomicI32::new(0);
// static volatile bool handler_set;
static HANDLER_SET: AtomicBool = AtomicBool::new(false);
static SAVED_HANDLERS: [AtomicUsize; 5] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
// static volatile bool caught_alarm, alarm_handler_set; (handle.c)
static CAUGHT_ALARM: AtomicBool = AtomicBool::new(false);
static ALARM_HANDLER_SET: AtomicBool = AtomicBool::new(false);
static SAVED_ALARM_HANDLER: AtomicUsize = AtomicUsize::new(0);
// static volatile bool ignore_alarm = false; (main.c)
static IGNORE_ALARM: AtomicBool = AtomicBool::new(false);

// kissat_signal_handler (main.c): print, dump statistics, re-raise.
fn signal_handler(sig: i32) {
    let ptr = SOLVER.load(Ordering::SeqCst) as *mut Solver;
    if !ptr.is_null() {
        // SAFETY: mirrors the C handler's access to the static solver.
        let solver = unsafe { &mut *ptr };
        crate::print::signal_msg(solver, "caught", sig, signal_name(sig));
        crate::internal::print_statistics(solver);
        crate::print::signal_msg(solver, "raising", sig, signal_name(sig));
    }
}

// static void catch_signal (int sig) (handle.c)
extern "C" fn catch_signal(sig: i32) {
    if CAUGHT_SIGNAL.load(Ordering::SeqCst) != 0 {
        return;
    }
    CAUGHT_SIGNAL.store(sig, Ordering::SeqCst);
    signal_handler(sig);
    reset_signal_handler();
    unsafe {
        raise(sig);
    }
}

// static void catch_alarm (int sig) (handle.c)
extern "C" fn catch_alarm(sig: i32) {
    debug_assert!(sig == SIGALRM);
    if CAUGHT_ALARM.load(Ordering::SeqCst) {
        return;
    }
    CAUGHT_ALARM.store(true, Ordering::SeqCst);
    if !ALARM_HANDLER_SET.load(Ordering::SeqCst) {
        unsafe {
            raise(sig);
        }
    }
    // kissat_alarm_handler (main.c):
    if IGNORE_ALARM.load(Ordering::SeqCst) {
        return;
    }
    let ptr = SOLVER.load(Ordering::SeqCst) as *mut Solver;
    if !ptr.is_null() {
        let solver = unsafe { &mut *ptr };
        crate::internal::terminate(solver);
    }
}

// kissat_init_signal_handler
fn init_signal_handler(solver: &mut Solver) {
    SOLVER.store(solver as *mut Solver as usize, Ordering::SeqCst);
    HANDLER_SET.store(true, Ordering::SeqCst);
    let handler: extern "C" fn(i32) = catch_signal;
    for (i, &sig) in SIGNALS.iter().enumerate() {
        let old = unsafe { signal(sig, handler as usize) };
        SAVED_HANDLERS[i].store(old, Ordering::SeqCst);
    }
}

// kissat_reset_signal_handler
fn reset_signal_handler() {
    if !HANDLER_SET.load(Ordering::SeqCst) {
        return;
    }
    for (i, &sig) in SIGNALS.iter().enumerate() {
        let old = SAVED_HANDLERS[i].load(Ordering::SeqCst);
        unsafe {
            signal(sig, old);
        }
    }
    HANDLER_SET.store(false, Ordering::SeqCst);
}

// kissat_init_alarm
fn init_alarm() {
    debug_assert!(!CAUGHT_ALARM.load(Ordering::SeqCst));
    ALARM_HANDLER_SET.store(true, Ordering::SeqCst);
    let handler: extern "C" fn(i32) = catch_alarm;
    let old = unsafe { signal(SIGALRM, handler as usize) };
    SAVED_ALARM_HANDLER.store(old, Ordering::SeqCst);
}

// kissat_reset_alarm
fn reset_alarm() {
    debug_assert!(ALARM_HANDLER_SET.load(Ordering::SeqCst));
    ALARM_HANDLER_SET.store(false, Ordering::SeqCst);
    let old = SAVED_ALARM_HANDLER.load(Ordering::SeqCst);
    unsafe {
        signal(SIGALRM, old);
    }
}

// ------------------------------------------------------------------------
// struct application
// ------------------------------------------------------------------------

struct App {
    input_path: Option<String>,
    output_path: Option<String>,
    proof_path: Option<String>,
    binary: i32,
    force: bool,
    time: i32,
    conflicts: i32,
    decisions: i32,
    strict: Strictness,
    partial: bool,
    witness: bool,
    max_var: i32,
}

// static void init_app (application *, kissat *)
fn init_app() -> App {
    App {
        input_path: None,
        output_path: None,
        proof_path: None,
        binary: 0,
        force: false,
        time: 0,
        conflicts: -1,
        decisions: -1,
        strict: NORMAL_PARSING,
        partial: false,
        witness: true,
        max_var: 0,
    }
}

// ------------------------------------------------------------------------
// usage
// ------------------------------------------------------------------------

fn print_common_dimacs_and_proof_usage() {
    println!();
    println!("Furthermore '<dimacs>' is the input file in DIMACS format.");
    println!("If '<proof>' is specified then a proof trace is written.");
}

fn print_complete_dimacs_and_proof_usage() {
    println!();
    println!("Furthermore '<dimacs>' is the input file in DIMACS format.");
    println!("The solver reads from '<stdin>' if '<dimacs>' is unspecified.");
    println!("If the path has a '.bz2', '.gz', '.lzma', '7z' or '.xz' suffix");
    println!("then the solver tries to find a corresponding decompression");
    println!("tool ('bzip2', 'gzip', 'lzma', '7z', or 'xz') to decompress");
    println!("the input file on-the-fly after checking that the input file");
    println!("has the correct format (starts with the corresponding");
    println!("signature bytes).");
    println!();
    println!("If '<proof>' is specified then a proof trace is written to the");
    println!("given file.  If the file name is '-' then the proof is written");
    println!("to '<stdout>'. In this case the ASCII version of the DRAT format");
    println!("is used.  For real files the binary proof format is used unless");
    println!("'--no-binary' is specified.");
    println!();
    println!("Writing of compressed proof files follows the same principle");
    println!("as reading compressed files. The compression format is based");
    println!("on the file suffix and it is checked that the corresponding");
    println!("compression utility can be found.");
}

fn print_force_usage() {
    // !NPROOFS && KISSAT_HAS_COMPRESSION branch:
    println!("  -f      force writing proofs (to existing CNF alike file)");
}

fn print_common_usage() {
    print!(
        "usage: kissat [ <option> ... ] [ <dimacs> [ <proof> ] ]\n\
         \n\
         where '<option>' is one of the following common options:\n\
         \n\
         \x20 -h      print this list of common command line options\n\
         \x20 --help  print complete list of command line options\n"
    );
    println!();
    print_force_usage();
    println!("  -n      do not print satisfying assignment");
    println!();
    println!("  -q      suppress all messages");
    println!("  -s      print complete statistics");
    println!("  -v      increase verbose level");
    print_common_dimacs_and_proof_usage();
}

fn print_complete_usage() {
    print!(
        "usage: kissat [ <option> ... ] [ <dimacs> [ <proof> ] ]\n\
         \n\
         where '<option>' is one of the following common options:\n\
         \n\
         \x20 --help  print this list of all command line options\n\
         \x20 -h      print only reduced list of command line options\n"
    );
    println!();
    print_force_usage();
    println!("  -n      do not print satisfying assignment");
    println!();
    println!("  -q      suppress all messages (see also '--quiet')");
    println!("  -s      print all statistics (see also '--statistics')");
    println!("  -v      increase verbose level (see also '--verbose')");
    println!();
    println!("Further '<option>' can be one of the following less frequent options:");
    println!();
    println!("  --banner             print solver information");
    println!("  --build              print build information");
    println!("  --color              use colors (default if connected to terminal)");
    println!("  --no-color           no colors (default if not connected to terminal)");
    println!("  --compiler           print compiler information");
    println!("  --copyright          print copyright information");
    println!("  --force              same as '-f' (force writing proof)");
    println!("  --id                 print 'git' identifier (SHA-1 hash)");
    println!("  --range              print option range list");
    println!("  --relaxed            relaxed parsing (ignore DIMACS header)");
    println!("  --strict             stricter parsing (no empty header lines)");
    println!("  --version            print version");
    println!();
    println!("The following solving limits can be enforced:");
    println!();
    println!("  --conflicts=<limit>");
    println!("  --decisions=<limit>");
    println!("  --time=<seconds>");
    println!();
    println!("Satisfying assignments have by default values for all variables");
    println!("unless '--partial' is specified, then only values are printed");
    println!("for variables which are necessary to satisfy the formula.");
    println!();
    println!("The following predefined 'configurations' (option settings) are supported:");
    println!();
    crate::config::configuration_usage();
    println!();
    println!("Or '<option>' is one of the following long options:\n");
    crate::options::options_usage();
    print_complete_dimacs_and_proof_usage();
}

// static bool parsed_one_option_and_return_zero_exit_code (char *arg)
fn parsed_one_option_and_return_zero_exit_code(arg: &str) -> bool {
    if arg == "-h" {
        print_common_usage();
        return true;
    }
    if arg == "--help" {
        print_complete_usage();
        return true;
    }
    if arg == "--banner" {
        banner(None, SOLVER_NAME);
        return true;
    }
    if arg == "--build" {
        build(None);
        return true;
    }
    if arg == "--copyright" {
        for line in copyright() {
            println!("{}", line);
        }
        return true;
    }
    if arg == "--compiler" {
        println!("{}", compiler());
        return true;
    }
    if arg == "--id" {
        println!("{}", id());
        return true;
    }
    if arg == "--range" {
        crate::options::print_option_range_list();
        return true;
    }
    if arg == "--version" {
        println!("{}", version());
        return true;
    }
    false
}

const SINGLE_FIRST_OPTION_TABLE: [&str; 9] = [
    "-h",
    "--help",
    "--banner",
    "--build",
    "--copyright",
    "--compiler",
    "--id",
    "--range",
    "--version",
];

fn single_first_option(arg: &str) -> bool {
    SINGLE_FIRST_OPTION_TABLE.iter().any(|&o| o == arg)
}

// ERROR (...) — prints `kissat: error: ...` and makes the caller fail.
macro_rules! app_error {
    ($($fmt:tt)*) => {{
        crate::error::error(format_args!($($fmt)*));
        return false;
    }};
}

// static bool most_likely_existing_cnf_file (const char *path)
fn most_likely_existing_cnf_file(path: &str) -> bool {
    if !file::file_readable(path) {
        return false;
    }
    for suffix in [
        ".dimacs",
        ".dimacs.7z",
        ".dimacs.bz2",
        ".dimacs.gz",
        ".dimacs.lzma",
        ".dimacs.xz",
        ".cnf",
        ".cnf.7z",
        ".cnf.bz2",
        ".cnf.gz",
        ".cnf.lzma",
        ".cnf.xz",
    ] {
        if crate::utilities::has_suffix(path, suffix) {
            return true;
        }
    }
    false
}

// LONG_TRUE_OPTION (ARG, NAME)
fn long_true_option(arg: &str, name: &str) -> bool {
    arg == format!("--{}", name)
        || arg == format!("--{}=1", name)
        || arg == format!("--{}=true", name)
}

// LONG_FALSE_OPTION (ARG, NAME)
fn long_false_option(arg: &str, name: &str) -> bool {
    arg == format!("--no-{}", name)
        || arg == format!("--{}=0", name)
        || arg == format!("--{}=false", name)
}

// static bool parse_options (application *, int argc, char **argv)
fn parse_options(solver: &mut Solver, app: &mut App, args: &[String]) -> bool {
    let mut strict_option: Option<&str> = None;
    let mut configuration: Option<&str> = None;
    let mut force_option: Option<&str> = None;
    let mut conflicts_option: Option<&str> = None;
    let mut decisions_option: Option<&str> = None;
    // `time_option` is declared but never set in C (see PORT NOTE above).
    let mut i = 1usize;
    while i < args.len() {
        let arg: &str = &args[i];
        if single_first_option(arg) {
            app_error!(
                "option '{}' only allowed as {} argument",
                arg,
                if i == 1 { "single" } else { "first" }
            );
        } else if arg == "-f" || long_true_option(arg, "force") || long_true_option(arg, "forced")
        {
            if app.force {
                let force_option = force_option.unwrap();
                if force_option == arg {
                    app_error!("multiple '{}' options", force_option);
                } else {
                    app_error!("'{}' and '{}' have the same effect", force_option, arg);
                }
            }
            app.force = true;
            force_option = Some(arg);
        } else if long_true_option(arg, "relax") || long_true_option(arg, "relaxed") {
            if let Some(strict_option) = strict_option {
                if app.strict != RELAXED_PARSING {
                    app_error!(
                        "can not combine contradictory '{}' and '{}'",
                        strict_option,
                        arg
                    );
                } else if strict_option == arg {
                    app_error!("multiple '{}' options", strict_option);
                } else {
                    app_error!("'{}' and '{}' have the same effect", strict_option, arg);
                }
            }
            app.strict = RELAXED_PARSING;
            strict_option = Some(arg);
        } else if long_true_option(arg, "strict")
            || long_true_option(arg, "stricter")
            || long_true_option(arg, "pedantic")
        {
            if let Some(strict_option) = strict_option {
                if app.strict != PEDANTIC_PARSING {
                    app_error!(
                        "can not combine contradictory '{}' and '{}'",
                        strict_option,
                        arg
                    );
                } else if strict_option == arg {
                    app_error!("multiple '{}' options", strict_option);
                } else {
                    app_error!("'{}' and '{}' have the same effect", strict_option, arg);
                }
            }
            app.strict = PEDANTIC_PARSING;
            strict_option = Some(arg);
        } else if arg == "-n" {
            app.witness = false;
        } else if arg == "-q" {
            crate::internal::set_option(solver, "quiet", 1);
        } else if arg == "-s" {
            crate::internal::set_option(solver, "statistics", 1);
        } else if arg == "-v" {
            let mut value = crate::internal::get_option(solver, "verbose");
            if value < i32::MAX {
                value += 1;
            }
            crate::internal::set_option(solver, "verbose", value);
        } else if arg == "--color" || arg == "--colors" || arg == "--colour" || arg == "--colours"
        {
            crate::colors::force_colors();
            crate::error::force_colors();
        } else if arg == "--no-color"
            || arg == "--no-colors"
            || arg == "--no-colour"
            || arg == "--no-colours"
        {
            crate::colors::force_no_colors();
            crate::error::force_no_colors();
        } else if let Some(valstr) = crate::options::parse_option_name(arg, "time") {
            match crate::options::parse_option_value(valstr) {
                Some(val) if val > 0 => {
                    app.time = val;
                    unsafe {
                        alarm(val as u32);
                    }
                }
                _ => app_error!("invalid argument in '{}' (try '-h')", arg),
            }
        } else if let Some(valstr) = crate::options::parse_option_name(arg, "conflicts") {
            match crate::options::parse_option_value(valstr) {
                Some(val) if val >= 0 => {
                    if let Some(conflicts_option) = conflicts_option {
                        app_error!("multiple '{}' and '{}'", conflicts_option, arg);
                    }
                    crate::internal::set_conflict_limit(solver, val as u32);
                    app.conflicts = val;
                    conflicts_option = Some(arg);
                }
                _ => app_error!("invalid argument in '{}' (try '-h')", arg),
            }
        } else if let Some(valstr) = crate::options::parse_option_name(arg, "decisions") {
            match crate::options::parse_option_value(valstr) {
                Some(val) if val >= 0 => {
                    if let Some(decisions_option) = decisions_option {
                        app_error!("multiple '{}' and '{}'", decisions_option, arg);
                    }
                    crate::internal::set_decision_limit(solver, val as u32);
                    app.decisions = val;
                    decisions_option = Some(arg);
                }
                _ => app_error!("invalid argument in '{}' (try '-h')", arg),
            }
        } else if arg == "--partial" {
            app.partial = true;
        } else if long_false_option(arg, "binary") {
            app.binary = -1;
        } else if arg.starts_with("--") && crate::config::has_configuration(&arg[2..]) {
            if let Some(configuration) = configuration {
                app_error!("multiple configurations '{}' and '{}'", configuration, arg);
            }
            crate::config::set_configuration(&mut solver.options, &arg[2..]);
            configuration = Some(arg);
        } else if arg.starts_with("--") {
            match crate::options::options_parse_arg(arg) {
                Some((name, value)) => {
                    let name = name.to_string();
                    crate::internal::set_option(solver, &name, value);
                }
                None => app_error!("invalid long option '{}' (try '-h')", arg),
            }
        } else if arg == "-o" {
            i += 1;
            if i == args.len() {
                app_error!("argument to '-o' missing (try '-h')");
            }
            let arg: &str = &args[i];
            if let Some(output_path) = &app.output_path {
                app_error!(
                    "multiple output options '-o {}' and '-o {}' (try '-h')",
                    output_path,
                    arg
                );
            }
            app.output_path = Some(arg.to_string());
        } else if arg == "-l" {
            // #ifndef LOGGING branch:
            app_error!(
                "invalid short option '{}' (configured without '-l' or '-g')",
                arg
            );
        } else if arg.starts_with('-') && arg.len() > 1 {
            app_error!("invalid short option '{}' (try '-h')", arg);
        } else if app.proof_path.is_some() {
            app_error!(
                "three file arguments '{}', '{}' and '{}' (try '-h')",
                app.input_path.as_deref().unwrap_or(""),
                app.proof_path.as_deref().unwrap(),
                arg
            );
        } else if let Some(input_path) = app.input_path.clone() {
            if input_path == arg {
                app_error!("will not read and write '{}' at the same time", input_path);
            }
            // KISSAT_HAS_COMPRESSION realpath(3) aliasing check:
            match std::fs::canonicalize(&input_path) {
                Ok(real_input_path) => {
                    if let Ok(real_arg_path) = std::fs::canonicalize(arg) {
                        if real_input_path == real_arg_path {
                            let real_input_str = real_input_path.to_string_lossy().into_owned();
                            let real_arg_str = real_arg_path.to_string_lossy().into_owned();
                            if arg != real_arg_str && input_path != real_input_str {
                                app_error!(
                                    "will not read and write '{}' and '{}' \
                                     pointing to the same file '{}'",
                                    input_path,
                                    arg,
                                    real_input_str
                                );
                            } else {
                                app_error!(
                                    "will not read and write '{}' and '{}' \
                                     pointing to the same file",
                                    input_path,
                                    arg
                                );
                            }
                        }
                    }
                }
                Err(_) => app_error!(
                    "can not get absolute path of '{}' (unexpectedly)",
                    input_path
                ),
            }
            if !app.force && most_likely_existing_cnf_file(arg) {
                app_error!("not writing proof to '{}' file (use '-f')", arg);
            }
            if !file::file_writable(arg) {
                app_error!("can not write proof to '{}'", arg);
            }
            app.proof_path = Some(arg.to_string());
        } else {
            if !file::file_readable(arg) {
                app_error!("can not read '{}'", arg);
            }
            app.input_path = Some(arg.to_string());
        }
        i += 1;
    }
    if crate::internal::get_option(solver, "quiet") != 0 {
        if crate::internal::get_option(solver, "statistics") != 0 {
            app_error!("can not use '--quiet' ('-q') with '--statistics' ('-s')");
        }
        if crate::internal::get_option(solver, "verbose") != 0 {
            app_error!("can not use '--quiet' ('-q') with '--verbose' ('-v')");
        }
    }
    true
}

// static bool parse_input (application *)
fn parse_input(solver: &mut Solver, app: &mut App) -> bool {
    let entered = crate::resources::process_time();
    let mut lineno: u64 = 0;
    let mut file = File::new();
    match &app.input_path {
        None => file::read_already_open_file(&mut file, "<stdin>"),
        Some(path) => {
            if !file::open_to_read_file(&mut file, path) {
                app_error!("failed to open '{}' for reading", path);
            }
        }
    }
    crate::print::section(solver, "parsing");
    crate::print::message(
        solver,
        format!(
            "opened and reading {}DIMACS file:",
            if file.compressed { "compressed " } else { "" }
        ),
    );
    crate::print::line(solver);
    crate::print::message(solver, format!("  {}", file.path));
    crate::print::line(solver);
    let error = crate::parse::parse_dimacs(solver, app.strict, &mut file, &mut lineno, &mut app.max_var);
    file::close_file(&mut file);
    if let Some(error) = error {
        app_error!("{}:{}: parse error: {}", file.path, lineno, error);
    }
    // #ifndef QUIET — kept:
    let bytes_str = crate::format::format_bytes(&mut solver.format, file.bytes);
    crate::print::message(
        solver,
        format!("closing input after reading {}", bytes_str),
    );
    if file.compressed {
        debug_assert!(app.input_path.is_some());
        let bytes = file::file_size(app.input_path.as_deref().unwrap());
        let size_str = crate::format::format_bytes(&mut solver.format, bytes);
        crate::print::message(
            solver,
            format!(
                "inflated input file of size {} by {:.2}",
                size_str,
                crate::utilities::average(file.bytes as f64, bytes as f64)
            ),
        );
    }
    crate::print::message(
        solver,
        format!(
            "finished parsing after {:.2} seconds",
            crate::resources::process_time() - entered
        ),
    );
    true
}

// static bool write_proof (application *)
fn write_proof(solver: &mut Solver, app: &mut App) -> bool {
    let path = match &app.proof_path {
        None => return true,
        Some(path) => path.clone(),
    };
    let mut file = File::new();
    let mut binary = true;
    if path == "-" {
        binary = false;
        file::write_already_open_file(&mut file, "<stdout>");
    } else if !file::open_to_write_file(&mut file, &path) {
        app_error!("failed to open and write proof to '{}'", path);
    } else if app.binary < 0 {
        binary = false;
    }
    // Capture the print fields before handing the file to the proof module
    // (C keeps the file in the application struct; see proof.rs PORT NOTE).
    let close = file.close;
    let compressed = file.compressed;
    let file_path = file.path.clone();
    crate::proof::init_proof(solver, file, binary);
    // #ifndef QUIET — kept:
    crate::print::section(solver, "proving");
    crate::print::message(
        solver,
        format!(
            "{}writing proof to {}DRAT file:",
            if close { "opened and " } else { "" },
            if compressed { "compressed " } else { "" }
        ),
    );
    crate::print::line(solver);
    crate::print::message(solver, format!("  {}", file_path));
    true
}

// static void close_proof (application *)
fn close_proof(solver: &mut Solver, app: &mut App) {
    if app.proof_path.is_none() {
        return;
    }
    let mut file = crate::proof::release_proof(solver);
    file::close_file(&mut file);
}

// static void print_option (kissat *, int value, const opt *)
fn print_option(solver: &mut Solver, value: i32, o: &crate::options::OptionEntry) {
    let b = o.low == 0 && o.high == 1;
    let val_str = crate::format::format_value(&mut solver.format, b, value);
    let def_str = crate::format::format_value(&mut solver.format, b, o.value);
    let buffer = format!("{}={}", o.name, val_str);
    crate::print::message(
        solver,
        format!(
            "--{:<30} ({} default '{}')",
            buffer,
            if value == o.value {
                "same as"
            } else {
                "different from"
            },
            def_str
        ),
    );
}

// static void print_options (kissat *)
fn print_options(solver: &mut Solver) {
    let verbosity = crate::print::verbosity(solver);
    if verbosity < 0 {
        return;
    }
    let mut printed: usize = 0;
    for o in crate::options::OPTION_TABLE {
        let value = crate::options::options_get(&solver.options, o.name);
        if o.value != value || verbosity > 0 {
            if printed == 0 {
                crate::print::section(solver, "options");
            }
            printed += 1;
            print_option(solver, value, o);
        }
    }
}

// static void print_limits (application *)
fn print_limits(solver: &mut Solver, app: &App) {
    let verbosity = crate::print::verbosity(solver);
    if verbosity < 1 && app.conflicts < 0 && app.decisions < 0 {
        return;
    }

    crate::print::section(solver, "limits");
    if app.time == 0 && app.conflicts < 0 && app.decisions < 0 {
        crate::print::message(solver, "no time, conflict nor decision limit set");
    } else {
        if app.time != 0 {
            crate::print::message(
                solver,
                format!("time limit set to {} seconds", app.time),
            );
        } else if verbosity > 0 {
            crate::print::message(solver, "no time limit");
        }

        if app.conflicts >= 0 {
            crate::print::message(
                solver,
                format!("conflict limit set to {} conflicts", app.conflicts),
            );
        } else if verbosity > 0 {
            crate::print::message(solver, "no conflict limit");
        }

        if app.decisions >= 0 {
            crate::print::message(
                solver,
                format!("decision limit set to {} decisions", app.decisions),
            );
        } else if verbosity > 0 {
            crate::print::message(solver, "no decision limit");
        }
    }
}

// static int run_application (kissat *, int argc, char **argv, bool *)
fn run_application(solver: &mut Solver, args: &[String], cancel_alarm_ptr: &mut bool) -> i32 {
    *cancel_alarm_ptr = false;
    if args.len() == 2 && parsed_one_option_and_return_zero_exit_code(&args[1]) {
        return 0;
    }
    let mut app = init_app();
    let ok = parse_options(solver, &mut app, args);
    if app.time > 0 {
        *cancel_alarm_ptr = true;
    }
    if !ok {
        return 1;
    }
    // #ifndef QUIET — kept:
    crate::print::section(solver, "banner");
    if solver.options.quiet == 0 {
        banner(Some("c "), SOLVER_NAME);
        let _ = std::io::stdout().flush();
    }
    if !write_proof(solver, &mut app) {
        return 1;
    }
    if !parse_input(solver, &mut app) {
        close_proof(solver, &mut app);
        return 1;
    }
    print_options(solver);
    print_limits(solver, &app);
    crate::print::section(solver, "solving");
    let res = crate::internal::solve(solver);
    close_proof(solver, &mut app);
    crate::print::section(solver, "result");
    if app.output_path.as_deref() == Some("-") {
        // #ifndef QUIET — kept:
        let status = if res == 20 {
            "UNSATISFIABLE"
        } else if res == 10 {
            "SATISFIABLE"
        } else {
            "UNKNOWN"
        };
        crate::print::message(
            solver,
            format!(
                "not printing 's {}' status line when writing DIMACS to '<stdout>'",
                status
            ),
        );
    } else if res == 20 {
        println!("s UNSATISFIABLE");
        let _ = std::io::stdout().flush();
    } else if res == 10 {
        println!("s SATISFIABLE");
        let _ = std::io::stdout().flush();
        if app.witness {
            crate::witness::print_witness(solver, app.max_var, app.partial);
        }
    } else {
        println!("s UNKNOWN");
        let _ = std::io::stdout().flush();
    }
    if let Some(path) = app.output_path.clone() {
        if path == "-" {
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            crate::krite::write_dimacs(solver, &mut out);
        } else {
            match std::fs::File::create(&path) {
                Ok(mut out) => {
                    crate::krite::write_dimacs(solver, &mut out);
                    // fclose (file)
                    drop(out);
                }
                Err(_) => {
                    // ERROR (...) in C expands to `return false` here — the
                    // quirky 0 exit code is ported as-is (see PORT NOTE).
                    crate::error::error(format_args!(
                        "could not write DIMACS file '{}'",
                        path
                    ));
                    return 0;
                }
            }
        }
    }
    crate::internal::print_statistics(solver);
    // #ifndef QUIET — kept:
    crate::print::section(solver, "shutting down");
    crate::print::message(solver, format!("exit {}", res));
    res
}

// int kissat_application (kissat *, int argc, char **argv)
fn kissat_application(solver: &mut Solver, args: &[String]) -> i32 {
    let mut cancel_alarm = false;
    let res = run_application(solver, args, &mut cancel_alarm);
    if cancel_alarm {
        unsafe {
            alarm(0);
        }
    }
    res
}

/// Port of `main` (main.c): solver setup, signal/alarm handler installation,
/// the application run and the teardown.  Returns the process exit code.
pub fn application(args: Vec<String>) -> i32 {
    let mut solver = Box::new(crate::internal::init());
    init_alarm();
    init_signal_handler(solver.as_mut());
    let res = kissat_application(solver.as_mut(), &args);
    reset_signal_handler();
    IGNORE_ALARM.store(true, Ordering::SeqCst);
    reset_alarm();
    SOLVER.store(0, Ordering::SeqCst);
    crate::internal::release(*solver);
    res
}
