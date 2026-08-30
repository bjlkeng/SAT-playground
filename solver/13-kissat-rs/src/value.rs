// Port of src/value.h (kissat 4.0.4).
// PORT NOTE: the C `mark` typedef also lives in value.h (there is no
// mark.h in 4.0.4), so `Mark` is defined here.
// The VALUE(LIT)/MARK(LIT) macros are direct array indexing
// (`solver.values[lit]` / `solver.marks[lit]`) at call sites.

pub type Value = i8;
pub type Mark = i8;

/// C `BOOL_TO_VALUE(B)`: true -> -1, false -> 1.
#[inline]
pub fn bool_to_value(b: bool) -> Value {
    if b {
        -1
    } else {
        1
    }
}
