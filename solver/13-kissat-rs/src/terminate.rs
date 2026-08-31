// Port of src/terminate.h + src/terminate.c (kissat 4.0.4).
//
// PORT NOTE: `TERMINATED (BIT)` expands `#BIT`, `__FILE__`, `__LINE__` and
// `__func__`.  Rust has no `__func__`; the `terminated!` macro below passes
// `file!()`/`line!()` and the stringified bit name, and the report line
// drops the function name (very-verbose diagnostics only; never part of the
// `-s` statistics parity oracle).
// PORT NOTE: COVERAGE is not defined in the reference build, so the
// per-bit mask logic is compiled out: any set flag terminates everything.

use crate::internal::Solver;
use std::sync::atomic::Ordering;

// Termination bits (terminate.h), C names verbatim.
#[allow(non_upper_case_globals)]
pub mod bits {
    pub const backbone_terminated_1: i32 = 1;
    pub const backbone_terminated_2: i32 = 2;
    pub const backbone_terminated_3: i32 = 3;
    pub const congruence_terminated_1: i32 = 4;
    pub const congruence_terminated_2: i32 = 5;
    pub const congruence_terminated_3: i32 = 6;
    pub const congruence_terminated_4: i32 = 7;
    pub const congruence_terminated_5: i32 = 8;
    pub const congruence_terminated_6: i32 = 9;
    pub const congruence_terminated_7: i32 = 10;
    pub const congruence_terminated_8: i32 = 11;
    pub const congruence_terminated_9: i32 = 12;
    pub const congruence_terminated_10: i32 = 13;
    pub const congruence_terminated_11: i32 = 14;
    pub const congruence_terminated_12: i32 = 15;
    pub const eliminate_terminated_1: i32 = 16;
    pub const eliminate_terminated_2: i32 = 17;
    pub const factor_terminated_1: i32 = 18;
    pub const fastel_terminated_1: i32 = 19;
    pub const forward_terminated_1: i32 = 20;
    pub const kitten_terminated_1: i32 = 21;
    pub const kitten_terminated_2: i32 = 22;
    pub const preprocess_terminated_1: i32 = 23;
    pub const search_terminated_1: i32 = 24;
    pub const substitute_terminated_1: i32 = 25;
    pub const sweep_terminated_1: i32 = 26;
    pub const sweep_terminated_2: i32 = 27;
    pub const sweep_terminated_3: i32 = 28;
    pub const sweep_terminated_4: i32 = 29;
    pub const sweep_terminated_5: i32 = 30;
    pub const sweep_terminated_6: i32 = 31;
    pub const sweep_terminated_7: i32 = 32;
    pub const sweep_terminated_8: i32 = 33;
    pub const transitive_terminated_1: i32 = 34;
    pub const transitive_terminated_2: i32 = 35;
    pub const transitive_terminated_3: i32 = 36;
    pub const vivify_terminated_1: i32 = 37;
    pub const vivify_terminated_2: i32 = 38;
    pub const vivify_terminated_3: i32 = 39;
    pub const vivify_terminated_4: i32 = 40;
    pub const vivify_terminated_5: i32 = 41;
    pub const walk_terminated_1: i32 = 42;
    pub const warmup_terminated_1: i32 = 43;
}

/// Port of `kissat_report_termination` (terminate.c, QUIET not defined).
pub fn report_termination(solver: &Solver, name: &str, file: &str, lineno: u32) {
    crate::print::very_verbose(
        solver,
        format!("{}:{}: 'TERMINATED ({})' triggered", file, lineno, name),
    );
}

/// Port of the inline `kissat_terminated` (terminate.h).
pub fn terminated(solver: &mut Solver, bit: i32, name: &str, file: &str, lineno: u32) -> bool {
    debug_assert!((0..64).contains(&bit));
    if !solver.termination.flagged.load(Ordering::SeqCst) {
        return false;
    }
    report_termination(solver, name, file, lineno);
    let _ = bit; // (void) bit — non-COVERAGE build.
    true
}

/// Port of the `TERMINATED (BIT)` macro.
#[macro_export]
macro_rules! terminated {
    ($solver:expr, $bit:ident) => {
        $crate::terminate::terminated(
            $solver,
            $crate::terminate::bits::$bit,
            stringify!($bit),
            file!(),
            line!(),
        )
    };
}
