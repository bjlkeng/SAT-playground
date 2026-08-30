// Port of src/reluctant.h + src/reluctant.c (kissat 4.0.4).
// Reluctant doubling (Knuth's formulation of the Luby sequence), exact.
// The C functions take `reluctant *` directly (no solver) except
// kissat_init_reluctant which reads options from the solver.

use crate::internal::Solver;

#[derive(Clone, Copy, Default)]
pub struct Reluctant {
    pub limited: bool,
    pub trigger: bool,
    pub period: u64,
    pub wait: u64,
    pub u: u64,
    pub v: u64,
    pub limit: u64,
}

pub fn enable_reluctant(reluctant: &mut Reluctant, mut period: u64, limit: u64) {
    if limit != 0 && period > limit {
        period = limit;
    }
    reluctant.limited = limit > 0;
    reluctant.trigger = false;
    reluctant.period = period;
    reluctant.wait = period;
    reluctant.u = 1;
    reluctant.v = 1;
    reluctant.limit = limit;
}

pub fn disable_reluctant(reluctant: &mut Reluctant) {
    reluctant.period = 0;
}

pub fn tick_reluctant(reluctant: &mut Reluctant) {
    if reluctant.period == 0 {
        return;
    }

    if reluctant.trigger {
        return;
    }

    debug_assert!(reluctant.wait > 0);
    reluctant.wait -= 1;
    if reluctant.wait != 0 {
        return;
    }

    let mut u = reluctant.u;
    let mut v = reluctant.v;

    if (u & u.wrapping_neg()) == v {
        u += 1;
        v = 1;
    } else {
        debug_assert!(u64::MAX / 2 >= v);
        v *= 2;
    }

    let mut wait = v * reluctant.period;

    if reluctant.limited && wait > reluctant.limit {
        u = 1;
        v = 1;
        wait = reluctant.period;
    }

    reluctant.trigger = true;
    reluctant.wait = wait;
    reluctant.u = u;
    reluctant.v = v;
}

/// C `kissat_reluctant_triggered` (static inline in reluctant.h).
#[inline]
pub fn reluctant_triggered(reluctant: &mut Reluctant) -> bool {
    if !reluctant.trigger {
        return false;
    }
    reluctant.trigger = false;
    true
}

pub fn init_reluctant(solver: &mut Solver) {
    if solver.options.reluctant != 0 {
        enable_reluctant(
            &mut solver.reluctant,
            solver.options.reluctantint as u64,
            solver.options.reluctantlim as u64,
        );
    } else {
        disable_reluctant(&mut solver.reluctant);
    }
}
