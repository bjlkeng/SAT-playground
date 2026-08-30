// Port of src/reference.h (kissat 4.0.4).

pub type Reference = u32;

pub const LD_MAX_REF: u32 = 31;
pub const MAX_REF: u32 = (1u32 << LD_MAX_REF) - 1;

pub const INVALID_REF: Reference = u32::MAX;

// C: `typedef STACK (reference) references;`
pub type References = Vec<Reference>;
