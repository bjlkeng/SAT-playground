// Port of src/smooth.h + src/smooth.c (kissat 4.0.4).
// PORT NOTE: the C functions take `kissat *solver` and a `name` string that
// are used only under LOGGING (not defined in the reference build); both
// arguments are dropped here, which also sidesteps borrow conflicts at
// UPDATE_AVERAGE call sites. The EMA math is ported bit-for-bit, including
// the biased/corrected split and the `new_exp == old_exp` fixed-point test.

#[derive(Clone, Copy, Default)]
pub struct Smooth {
    pub value: f64,
    pub biased: f64,
    pub alpha: f64,
    pub beta: f64,
    pub exp: f64,
}

/// C `kissat_init_smooth` (solver and name arguments dropped, LOGGING only).
pub fn init_smooth(smooth: &mut Smooth, window: i32) {
    debug_assert!(window > 0);
    let alpha = 1.0 / window as f64;
    smooth.value = 0.0;
    smooth.biased = 0.0;
    smooth.alpha = alpha;
    smooth.beta = 1.0 - alpha;
    debug_assert!(smooth.beta > 0.0);
    smooth.exp = 1.0;
}

/// C `kissat_update_smooth` (solver argument dropped, LOGGING only).
pub fn update_smooth(smooth: &mut Smooth, y: f64) {
    let old_biased = smooth.biased;
    let alpha = smooth.alpha;
    let beta = smooth.beta;
    let delta = y - old_biased;
    let scaled_delta = alpha * delta;
    let new_biased = old_biased + scaled_delta;
    smooth.biased = new_biased;
    let old_exp = smooth.exp;
    let new_value;
    if old_exp != 0.0 {
        let mut new_exp = old_exp * beta;
        debug_assert!(new_exp < 1.0);
        if new_exp == old_exp {
            new_exp = 0.0;
            new_value = new_biased;
        } else {
            let div = 1.0 - new_exp;
            debug_assert!(div > 0.0);
            new_value = new_biased / div;
        }
        smooth.exp = new_exp;
    } else {
        new_value = new_biased;
    }
    smooth.value = new_value;
}
