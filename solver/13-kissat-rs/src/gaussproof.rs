//! Not in kissat. The VeriPB proof of the parity engine (bead
//! SAT-playground-1v2.7.2): what the engine derives, as cutting-planes
//! steps in the structure pass's proof stream. A stub until X.2: the
//! hooks are in place and do nothing.

use crate::internal::Solver;

/// Row `r` was added into row `s` (s ^= r).
#[inline]
pub fn added(_solver: &mut Solver, _s: usize, _r: usize) {}
