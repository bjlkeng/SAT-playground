// Port of src/extend.h types (kissat 4.0.4); extend.c lands with the
// proof/witness wave.

use crate::internal::Solver;

/// C `struct extension` — `signed int lit : 31; bool blocking : 1;`
/// packed into one u32 word for layout fidelity.
#[derive(Clone, Copy)]
pub struct Extension(pub u32);

impl Extension {
    #[inline]
    pub fn new(blocking: bool, lit: i32) -> Extension {
        debug_assert!(lit.unsigned_abs() < (1 << 30));
        Extension((((lit as u32) << 1) >> 1) | ((blocking as u32) << 31))
    }

    #[inline]
    pub fn lit(self) -> i32 {
        // Sign-extend the low 31 bits.
        ((self.0 << 1) as i32) >> 1
    }

    #[inline]
    pub fn blocking(self) -> bool {
        (self.0 >> 31) != 0
    }
}

pub type Extensions = Vec<Extension>;

pub fn extend(solver: &mut Solver) {
    let _ = solver;
    unimplemented!("extend wave pending")
}
