// Port of src/report.c (kissat 4.0.4).
//
// One-line progress reports ('c <type> ...' rows) plus the three-row column
// header every 20 reports. Byte-identical to the C output (colors only when
// stdout is a terminal, exactly as C's TERMINAL/COLOR macros).
//
// PORT NOTE: C builds `char line[128]` with sprintf and tracks `pos`; the
// Rust String equals the same bytes (pos == line.len() throughout). The C
// REPORTS X-macro is unrolled here into parallel name/value tables in the
// exact table order; the second (header) expansion of REPORTS only uses the
// names, as in C.
//
// PORT NOTE: 'e' / 'b','c','d','=' / '(',')' use C's concatenated color
// literals (BOLD GREEN etc.) — printed here as two consecutive escape
// sequences, which is byte-identical to the concatenated C string literal.
// The C switch's '(' / ')' case has no `break`, but it is the last case, so
// behavior is identical.

use crate::colors;
use crate::format;
use crate::internal::Solver;
use crate::profile;
use crate::resources;
use std::io::Write as _;

const ROWS: usize = 3;

// kissat_report
pub fn report(solver: &mut Solver, verbose: bool, type_: char) {
    let verbosity = crate::print::verbosity(solver);
    if verbosity < 0 {
        return;
    }
    if verbose && verbosity < 2 {
        return;
    }

    // REP ("seconds", ...) evaluates kissat_time first (flushes profiles).
    let seconds = profile::time(solver);
    // #define MB (kissat_current_resident_set_size () / (double) (1 << 20))
    let mb = resources::current_resident_set_size() as f64 / (1u64 << 20) as f64;

    let st = &solver.statistics;
    let idx = solver.stable as usize;
    let a = &solver.averages[idx]; // AVERAGE (NAME) = averages[stable].NAME.value

    // REPORTS table: names and sprintf-formatted values, in table order.
    let names: [&str; 19] = [
        "seconds",
        "MB",
        "level",
        "switched",
        "reductions",
        "restarts",
        "rate",
        "conflicts",
        "redundant",
        "size/glue",
        "size",
        "glue",
        "tier1",
        "tier2",
        "trail",
        "binary",
        "irredundant",
        "variables",
        "remaining",
    ];
    let values: [String; 19] = [
        format!("{:5.2}", seconds),                       // "%5.2f"
        format!("{:2.0}", mb),                            // "%2.0f"
        format!("{:.0}", a.level.value),                  // "%.0f"
        format!("{:1}", st.switched),                     // "%1" PRIu64
        format!("{:1}", st.reductions),                   // "%1" PRIu64
        format!("{:2}", st.restarts),                     // "%2" PRIu64
        format!("{:.0}", a.decision_rate.value),          // "%.0f"
        format!("{:3}", st.conflicts),                    // "%3" PRIu64
        format!("{:3}", st.clauses_redundant),            // "%3" PRIu64
        format!(
            "{:.1}",
            format::average(a.size.value, a.slow_glue.value)
        ), // "%.1f"
        format!("{:.0}", a.size.value),                   // "%.0f"
        format!("{:.0}", a.slow_glue.value),              // "%.0f"
        format!("{:1}", solver.tier1[idx]),               // "%1u"
        format!("{:1}", solver.tier2[idx]),               // "%1u"
        format!("{:.0}%", a.trail.value),                 // "%.0f%%"
        format!("{:3}", st.clauses_binary),               // "%3" PRIu64
        format!("{:2}", st.clauses_irredundant),          // "%2" PRIu64
        format!("{:2}", solver.active),                   // "%2u"
        format!(
            "{:1.0}%",
            format::percent(solver.active as f64, st.variables_original as f64)
        ), // "%1.0f%%" of REMAINING_VARIABLES
    ];

    // char line[128], *p; unsigned pad[32], n = 1, pos = 0; pad[0] = 0;
    let mut line = String::new();
    let mut pad = [0usize; 32];
    let mut n = 1usize;
    for v in &values {
        line.push(' ');
        line.push_str(v);
        pad[n] = line.len();
        n += 1;
    }
    debug_assert!(line.len() < 128);

    // TERMINAL (stdout, 1)
    let connected_to_terminal = colors::connected_to_terminal(1);
    macro_rules! color {
        ($code:expr) => {
            if connected_to_terminal {
                print!("{}", $code);
            }
        };
    }

    // if (!(solver->limits.reports++ % 20)) { ... }
    let reports_before = solver.limits.reports;
    solver.limits.reports += 1;
    if reports_before % 20 == 0 {
        let mut rows: [String; ROWS] = Default::default();
        let mut last = [0usize; ROWS];
        let mut row = 0usize;
        let mut i = 1usize;
        for name in &names {
            if last[row] != 0 {
                rows[row].push(' ');
                last[row] += 1;
            }
            let mut target = pad[i];
            let name_len = name.len();
            let val_len = pad[i] - pad[i - 1] - 1;
            if val_len < name_len {
                target += (name_len - val_len) / 2;
            }
            while last[row] + name_len < target {
                rows[row].push(' ');
                last[row] += 1;
            }
            for c in name.chars() {
                rows[row].push(c);
                last[row] += 1;
            }
            row += 1;
            if row == ROWS {
                row = 0;
            }
            i += 1;
        }
        debug_assert_eq!(i, n);
        if solver.limits.reports > 1 {
            crate::print::line(solver);
        }
        for row in rows.iter() {
            print!("{}", solver.prefix);
            color!(colors::CYAN);
            print!("{}", row);
            color!(colors::NORMAL);
            println!();
        }
        crate::print::line(solver);
    }

    crate::print::prefix(solver);
    match type_ {
        '1' | '0' | '?' | 'i' | '.' => {
            color!(colors::BOLD);
        }
        'e' => {
            color!(colors::BOLD);
            color!(colors::GREEN);
        }
        '2' | 's' => {
            color!(colors::GREEN);
        }
        'f' | 't' | 'u' | 'v' | 'w' | 'x' => {
            color!(colors::BLUE);
        }
        'b' | 'c' | 'd' | '=' => {
            color!(colors::BOLD);
            color!(colors::BLUE);
        }
        '[' | ']' => {
            color!(colors::MAGENTA);
        }
        '(' | ')' => {
            color!(colors::BOLD);
            color!(colors::YELLOW);
        }
        _ => {}
    }
    print!("{}", type_);
    color!(colors::NORMAL);
    if solver.preprocessing {
        color!(colors::YELLOW);
    } else if solver.stable {
        color!(colors::MAGENTA);
    }
    print!("{}", line);
    color!(colors::NORMAL);
    println!();
    std::io::stdout().flush().ok();
}
