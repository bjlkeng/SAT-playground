// Port of src/frames.h (kissat 4.0.4).
// PORT NOTE: the `#ifndef NDEBUG` member `saved` is omitted (NDEBUG build).
// The FRAME(LEVEL) macro is direct indexing at call sites:
// `solver.frames[level as usize]`.
// kissat_push_frame lives in inlineframes.h and is ported with that file's
// module, not here.

/// C `struct frame`.
#[derive(Clone, Copy, Default)]
pub struct Frame {
    pub promote: bool,
    pub decision: u32,
    pub trail: u32,
    pub used: u32,
}

/// C `typedef STACK (frame) frames`.
pub type Frames = Vec<Frame>;

/// Port of inlineframes.h `kissat_push_frame`.
#[inline]
pub fn push_frame(solver: &mut crate::internal::Solver, decision: u32) {
    debug_assert!(solver.level == 0 || decision != u32::MAX);
    let trail = solver.trail.len() as u32;
    solver.frames.push(Frame {
        decision,
        promote: false,
        trail,
        used: 0,
    });
}
