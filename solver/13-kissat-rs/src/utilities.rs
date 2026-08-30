// Port of src/utilities.h + src/utilities.c (kissat 4.0.4).
// PORT NOTE: C `word` is `uintptr_t`; this port targets 64-bit hosts only, so
// `Word = u64`. kissat_cache_lines is ported bit-exactly (ticks parity
// depends on it); the C build asserts `size == 4` — under NDEBUG the `size`
// argument is unused, but it is kept for call-site fidelity.
// PORT NOTE: the SWAP/MIN/MAX/ABS macros are not ported (use std::mem::swap,
// .min()/.max(), unsigned_abs at call sites).

pub type Word = u64;

pub const WORD_ALIGNMENT_MASK: Word = (std::mem::size_of::<Word>() - 1) as Word;
pub const W2RD_ALIGNMENT_MASK: Word = (2 * std::mem::size_of::<Word>() - 1) as Word;

pub const ASSUMED_LD_CACHE_LINE_BYTES: u32 = 7;

#[inline]
pub fn cache_lines(n: Word, _size: usize) -> Word {
    if n == 0 {
        return 0;
    }
    debug_assert_eq!(_size, 4);
    let shift: u32 = ASSUMED_LD_CACHE_LINE_BYTES - 2;
    let mask: Word = ((1 as Word) << shift) - 1;
    let masked: Word = n + mask;
    masked >> shift
}

#[inline]
pub fn average(a: f64, b: f64) -> f64 {
    if b != 0.0 {
        a / b
    } else {
        0.0
    }
}

#[inline]
pub fn percent(a: f64, b: f64) -> f64 {
    average(100.0 * a, b)
}

#[inline]
pub fn aligned_word(word: Word) -> bool {
    word & WORD_ALIGNMENT_MASK == 0
}

#[inline]
pub fn align_word(w: Word) -> Word {
    let mut res = w;
    if res & WORD_ALIGNMENT_MASK != 0 {
        res = 1 + (res | WORD_ALIGNMENT_MASK);
    }
    res
}

#[inline]
pub fn align_w2rd(w: Word) -> Word {
    let mut res = w;
    if res & W2RD_ALIGNMENT_MASK != 0 {
        res = 1 + (res | W2RD_ALIGNMENT_MASK);
    }
    res
}

/// C `kissat_has_suffix` (byte-wise suffix comparison from the end).
pub fn has_suffix(str_: &str, suffix: &str) -> bool {
    str_.as_bytes().ends_with(suffix.as_bytes())
}

#[inline]
pub fn is_power_of_two(w: u64) -> bool {
    w != 0 && (w & (w - 1)) == 0
}

#[inline]
pub fn is_zero_or_power_of_two(w: Word) -> bool {
    w & w.wrapping_sub(1) == 0
}

#[inline]
pub fn leading_zeroes_of_unsigned(x: u32) -> u32 {
    x.leading_zeros() // C returns 32 for x == 0; leading_zeros matches
}

#[inline]
pub fn leading_zeroes_of_word(x: Word) -> u32 {
    x.leading_zeros()
}

#[inline]
pub fn log2_floor_of_word(x: Word) -> u32 {
    if x != 0 {
        (std::mem::size_of::<Word>() as u32) * 8 - 1 - leading_zeroes_of_word(x)
    } else {
        0
    }
}

#[inline]
pub fn log2_ceiling_of_word(x: Word) -> u32 {
    if x == 0 {
        return 0;
    }
    let tmp = log2_floor_of_word(x);
    tmp + ((x ^ ((1 as Word) << tmp) != 0) as u32)
}

#[inline]
pub fn leading_zeroes_of_uint64(x: u64) -> u32 {
    x.leading_zeros()
}

#[inline]
pub fn log2_floor_of_uint64(x: u64) -> u32 {
    if x != 0 {
        63 - leading_zeroes_of_uint64(x)
    } else {
        0
    }
}

#[inline]
pub fn log2_ceiling_of_uint64(x: u64) -> u32 {
    if x == 0 {
        return 0;
    }
    let tmp = log2_floor_of_uint64(x);
    tmp + ((x ^ (1u64 << tmp) != 0) as u32)
}
