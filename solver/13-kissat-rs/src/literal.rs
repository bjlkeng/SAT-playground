// Port of src/literal.h (kissat 4.0.4).
// PORT NOTE: VALID_INTERNAL_INDEX / VALID_INTERNAL_LITERAL depend on the
// solver's VARS/LITS and in C appear only inside asserts (NDEBUG build drops
// them); they are provided here as helpers taking the bound explicitly.
// The C macros' embedded asserts are omitted per the NDEBUG convention.

pub const LD_MAX_VAR: u32 = 30;
pub const LD_MAX_LIT: u32 = 1 + LD_MAX_VAR;

/// External variables are `int` in C, so this is `i32`.
pub const EXTERNAL_MAX_VAR: i32 = (1 << LD_MAX_VAR) - 1;
pub const INTERNAL_MAX_VAR: u32 = (1u32 << LD_MAX_VAR) - 2;
pub const INTERNAL_MAX_LIT: u32 = 2 * INTERNAL_MAX_VAR + 1;

pub const ILLEGAL_LIT: u32 = (1u32 << LD_MAX_LIT) - 1;

pub const INVALID_IDX: u32 = u32::MAX;
pub const INVALID_LIT: u32 = u32::MAX;

#[inline]
pub fn valid_internal_index(idx: u32, vars: u32) -> bool {
    idx < vars
}

#[inline]
pub fn valid_internal_literal(lit: u32, lits: u32) -> bool {
    lit < lits
}

#[inline]
pub fn valid_external_literal(elit: i32) -> bool {
    elit != 0 && elit != i32::MIN && elit.unsigned_abs() as i32 <= EXTERNAL_MAX_VAR
}

/// C `IDX(LIT)`.
#[inline]
pub fn idx(lit: u32) -> u32 {
    lit >> 1
}

/// C `LIT(IDX)`.
#[inline]
pub fn lit(idx: u32) -> u32 {
    idx << 1
}

/// C `NOT(LIT)`.
#[inline]
pub fn not(lit: u32) -> u32 {
    lit ^ 1
}

/// C `NEGATED(LIT)` (a.k.a. the sign bit, SGN).
#[inline]
pub fn negated(lit: u32) -> u32 {
    lit & 1
}

/// C `STRIP(LIT)`.
#[inline]
pub fn strip(lit: u32) -> u32 {
    lit & !1u32
}
