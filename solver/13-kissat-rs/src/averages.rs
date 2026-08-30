// Port of src/averages.h + src/averages.c (kissat 4.0.4).
// QUIET is NOT defined in the reference build, so level/size/trail EMAs are
// kept.
// PORT NOTE: C `kissat_init_averages (kissat *, averages *)` takes a pointer
// into solver->averages[solver->stable] while reading GET_OPTION from the
// same solver; the Rust version takes the index (0 = focused, 1 = stable)
// and derives the averages entry internally to satisfy the borrow checker.
// C call site `kissat_init_averages (solver, &AVERAGES)` (mode.c) becomes
// `averages::init_averages(solver, solver.stable as usize)`.
// The AVERAGES / EMA / AVERAGE / UPDATE_AVERAGE macros map at call sites to
// `solver.averages[solver.stable as usize].NAME` and
// `crate::smooth::update_smooth(&mut ..., value)`.

use crate::internal::Solver;
use crate::smooth::Smooth;

#[derive(Clone, Copy, Default)]
pub struct Averages {
    pub initialized: bool,
    pub fast_glue: Smooth,
    pub slow_glue: Smooth,
    // #ifndef QUIET (kept — QUIET not defined):
    pub level: Smooth,
    pub size: Smooth,
    pub trail: Smooth,
    pub decision_rate: Smooth,
    pub saved_decisions: u64,
}

pub fn init_averages(solver: &mut Solver, which: usize) {
    let emaslow = solver.options.emaslow;
    let emafast = solver.options.emafast;
    let averages = &mut solver.averages[which];
    if averages.initialized {
        return;
    }
    crate::smooth::init_smooth(&mut averages.level, emaslow);
    crate::smooth::init_smooth(&mut averages.size, emaslow);
    crate::smooth::init_smooth(&mut averages.trail, emaslow);
    crate::smooth::init_smooth(&mut averages.fast_glue, emafast);
    crate::smooth::init_smooth(&mut averages.slow_glue, emaslow);
    crate::smooth::init_smooth(&mut averages.decision_rate, emaslow);
    averages.initialized = true;
}
