// Port of src/random.h (kissat 4.0.4).
// Bit-exact PRNG: linear congruential on u64 with the exact C constants.
// PORT NOTE: `pick_random` deliberately reproduces the C truncation
// `(unsigned) (delta * fraction)` — do not "fix" the rounding.

pub type Generator = u64;

#[inline]
pub fn next_random64(rng: &mut Generator) -> u64 {
    *rng = rng
        .wrapping_mul(6364136223846793005u64)
        .wrapping_add(1442695040888963407u64);
    *rng
}

#[inline]
pub fn next_random32(rng: &mut Generator) -> u32 {
    (next_random64(rng) >> 32) as u32
}

#[inline]
pub fn pick_random(rng: &mut Generator, l: u32, r: u32) -> u32 {
    debug_assert!(l <= r);
    if l == r {
        return l;
    }
    let delta = r - l;
    let tmp = next_random32(rng);
    let fraction = tmp as f64 / 4294967296.0;
    let scaled = (delta as f64 * fraction) as u32;
    debug_assert!(scaled < delta);
    l + scaled
}

#[inline]
pub fn pick_bool(rng: &mut Generator) -> bool {
    pick_random(rng, 0, 2) != 0
}

#[inline]
pub fn pick_double(rng: &mut Generator) -> f64 {
    next_random32(rng) as f64 / 4294967296.0
}
