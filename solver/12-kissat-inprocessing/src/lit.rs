//! Literal and variable encoding helpers.
//!
//! Solver 12 still uses solver-10 integer literals at the public boundary:
//! DIMACS variables are 1-based and a negative sign represents negation.
//! Clause arena words preserve the raw two's-complement `i32` literal bits in a
//! `u32` slot. This matches solver 10 and is intentionally not a packed sign-bit
//! encoding.

#[inline(always)]
pub(crate) fn lit_to_word(lit: i32) -> u32 {
    lit as u32
}

#[inline(always)]
pub(crate) fn word_to_lit(word: u32) -> i32 {
    word as i32
}

#[inline(always)]
pub(crate) fn lit_to_index(lit: i32) -> usize {
    let var = lit.unsigned_abs() as usize;
    let base = (var - 1) * 2;
    if lit > 0 {
        base
    } else {
        base + 1
    }
}
