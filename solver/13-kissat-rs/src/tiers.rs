// Port of src/tiers.c (kissat 4.0.4).
//
// PORT NOTES:
//  - TIER1RELATIVE / TIER2RELATIVE (GET_OPTION (tierXrelative) / 1e3) are the
//    Options::tier1_relative()/tier2_relative() helpers.  The C
//    `uint64_t limit = total_used * TIERXRELATIVE;` multiplies u64 by double
//    and truncates back to u64 — ported exactly.
//  - retiered is a COUNTER (GET (retiered) prints the real count).
//  - kissat_print_tier_usage_statistics reproduces the C stdout bytes exactly
//    (dynamic field widths via decimal_digits, %5.2f percent columns).

use crate::internal::Solver;
use crate::statistics::MAX_GLUE_USED;

// static compute_tier_limits (out-parameters become a tuple).
fn compute_tier_limits(solver: &Solver, stable: bool) -> (u32, u32) {
    let statistics = &solver.statistics;
    let used_stats = &statistics.used[stable as usize].glue;
    let mut total_used: u64 = 0;
    for glue in 0..=MAX_GLUE_USED {
        total_used += used_stats[glue];
    }
    let mut tier1: i32 = -1;
    let mut tier2: i32 = -1;
    if total_used != 0 {
        let accumulated_tier1_limit =
            (total_used as f64 * solver.options.tier1_relative()) as u64;
        let accumulated_tier2_limit =
            (total_used as f64 * solver.options.tier2_relative()) as u64;
        let mut accumulated_used: u64 = 0;
        let mut glue = 0usize;
        while glue <= MAX_GLUE_USED {
            let glue_used = used_stats[glue];
            accumulated_used += glue_used;
            if accumulated_used >= accumulated_tier1_limit {
                tier1 = glue as i32;
                break;
            }
            glue += 1;
        }
        if accumulated_used < accumulated_tier2_limit {
            let mut glue = (tier1 + 1) as usize;
            while glue <= MAX_GLUE_USED {
                let glue_used = used_stats[glue];
                accumulated_used += glue_used;
                if accumulated_used >= accumulated_tier2_limit {
                    tier2 = glue as i32;
                    break;
                }
                glue += 1;
            }
        }
    }
    if tier1 < 0 {
        tier1 = solver.options.tier1;
        tier2 = solver.options.tier2.max(tier1); // MAX (GET_OPTION (tier2), tier1)
    } else if tier2 < 0 {
        tier2 = tier1;
    }
    debug_assert!(tier1 >= 0);
    debug_assert!(tier2 >= 0);
    (tier1 as u32, tier2 as u32)
}

/// Port of `kissat_compute_and_set_tier_limits`.
pub fn compute_and_set_tier_limits(solver: &mut Solver) {
    let stable = solver.stable;
    let (tier1, tier2) = compute_tier_limits(solver, stable);
    solver.tier1[stable as usize] = tier1;
    solver.tier2[stable as usize] = tier2;
    crate::print::phase(
        solver,
        "retiered",
        solver.statistics.retiered, // GET (retiered)
        format!(
            "recomputed {} tier1 limit {} and tier2 limit {} after {} conflicts",
            if stable { "stable" } else { "focused" },
            tier1,
            tier2,
            solver.statistics.conflicts
        ),
    );
}

// static decimal_digits
fn decimal_digits(i: u64) -> u32 {
    let mut res: u32 = 1;
    let mut limit: u64 = 10;
    loop {
        if i < limit {
            return res;
        }
        limit *= 10;
        res += 1;
    }
}

/// Port of `kissat_print_tier_usage_statistics` (exact fn name/signature
/// required by statistics.rs `print_glue_usage`).
pub fn print_tier_usage_statistics(solver: &mut Solver, stable: bool) {
    let (tier1, tier2) = compute_tier_limits(solver, stable);
    let statistics = &solver.statistics;
    let used_stats = &statistics.used[stable as usize].glue;
    let mut total_used: u64 = 0;
    for glue in 0..=MAX_GLUE_USED {
        total_used += used_stats[glue];
    }
    let mode = if stable { "stable" } else { "focused" };
    debug_assert!(tier1 <= tier2);
    let span = tier2 - tier1 + 1;
    let max_printed: u32 = 5;
    let (prefix, suffix): (u32, u32) = if span > max_printed {
        (tier1 + max_printed / 2 - 1, tier2 - max_printed / 2 + 1)
    } else {
        (u32::MAX, 0)
    };
    let mut accumulated_middle: u64 = 0;
    let mut glue_digits: u32 = 1;
    let mut clauses_digits: u32 = 1;
    for glue in 0..=MAX_GLUE_USED as u32 {
        if glue < tier1 {
            continue;
        }
        let used = used_stats[glue as usize];
        let mut tmp_glue: u32 = 0;
        let mut tmp_clauses: u32 = 0;
        if glue <= prefix || suffix <= glue {
            tmp_glue = decimal_digits(glue as u64);
            tmp_clauses = decimal_digits(used);
        } else {
            accumulated_middle += used;
            if glue + 1 == suffix {
                tmp_glue =
                    decimal_digits((prefix + 1) as u64) + decimal_digits(glue as u64) + 1;
                tmp_clauses = decimal_digits(accumulated_middle);
            }
        }
        if tmp_glue > glue_digits {
            glue_digits = tmp_glue;
        }
        if tmp_clauses > clauses_digits {
            clauses_digits = tmp_clauses;
        }
        if glue == tier2 {
            break;
        }
    }
    // sprintf (fmt, "%%%d" PRIu64, clauses_digits): right-justified width.
    let clauses_width = clauses_digits as usize;
    let glue_width = glue_digits as usize;
    let solver_prefix = solver.prefix.clone();
    let mut accumulated_middle: u64 = 0;
    let mut accumulated: u64 = 0;
    for glue in 0..=MAX_GLUE_USED as u32 {
        let used = used_stats[glue as usize];
        accumulated += used;
        if glue < tier1 {
            continue;
        }
        if glue <= prefix || suffix <= glue + 1 {
            print!("{}{} glue ", solver_prefix, mode);
        }
        if glue <= prefix || suffix <= glue {
            let s = format!("{}", glue);
            print!("{}", s);
            let mut len = s.len();
            while len > 0 && len < glue_width {
                print!(" ");
                len += 1;
            }
            print!(" used ");
            print!("{:>width$}", used, width = clauses_width);
            print!(
                " clauses {:5.2}% accumulated {:5.2}%",
                crate::format::percent(used as f64, total_used as f64),
                crate::format::percent(accumulated as f64, total_used as f64)
            );
            if glue == tier1 {
                print!(" tier1");
            }
            if glue == tier2 {
                print!(" tier2");
            }
            println!();
        } else {
            accumulated_middle += used;
            if glue + 1 == suffix {
                let s = format!("{}-{}", prefix + 1, suffix - 1);
                print!("{}", s);
                let mut len = s.len();
                while len > 0 && len < glue_width {
                    print!(" ");
                    len += 1;
                }
                print!(" used ");
                print!("{:>width$}", accumulated_middle, width = clauses_width);
                println!(
                    " clauses {:5.2}% accumulated {:5.2}%",
                    crate::format::percent(accumulated_middle as f64, total_used as f64),
                    crate::format::percent(accumulated as f64, total_used as f64)
                );
            }
        }
        if glue == tier2 {
            break;
        }
    }
}
