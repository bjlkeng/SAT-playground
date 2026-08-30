// Port of src/format.h + src/format.c (kissat 4.0.4).
//
// PORT NOTE: The C functions return `const char *` pointers into a rotating
// ring of NUM_FORMAT_STRINGS static buffers inside `struct format`, so that up
// to 16 formatted values can be alive inside one printf call. Rust cannot
// return several live `&str` borrows of the same `&mut Format`, so every
// formatter writes into the ring slot (rotation preserved) and returns an
// owned `String` clone of it. Byte content is identical; only the lifetime
// mechanics differ.
//
// PORT NOTE: `kissat_average` / `kissat_percent` live in utilities.h in C
// (static inline). They are hosted here so statistics.rs / profile.rs /
// resources.rs can share them, per the cluster layout.
//
// PORT NOTE: C `word` (uintptr_t) -> u64 for `format_signs`.

pub const NUM_FORMAT_STRINGS: usize = 16;
pub const FORMAT_STRING_SIZE: usize = 128; // C buffer capacity; informational only in Rust.

#[derive(Default)]
pub struct Format {
    pub pos: usize,
    pub str_: [String; NUM_FORMAT_STRINGS],
}

// utilities.h: kissat_average
pub fn average(a: f64, b: f64) -> f64 {
    if b != 0.0 {
        a / b
    } else {
        0.0
    }
}

// utilities.h: kissat_percent
pub fn percent(a: f64, b: f64) -> f64 {
    average(100.0 * a, b)
}

// utilities.h: kissat_is_power_of_two (private helper here; format.c uses it)
fn is_power_of_two(w: u64) -> bool {
    w != 0 && (w & (w - 1)) == 0
}

// kissat_next_format_string: returns the ring slot index to write into.
pub fn next_format_string(format: &mut Format) -> usize {
    let res = format.pos;
    format.pos += 1;
    if format.pos == NUM_FORMAT_STRINGS {
        format.pos = 0;
    }
    res
}

// static format_count (char *res, uint64_t w)
fn format_count_str(w: u64) -> String {
    if w >= 128 && is_power_of_two(w) {
        let mut l: u32 = 0;
        while (1u64 << l) != w {
            l += 1;
        }
        format!("2^{}", l)
    } else if w >= 1000 && w % 1000 == 0 {
        let mut w = w;
        let mut l: u32 = 0;
        while w % 10 == 0 {
            l += 1;
            w /= 10;
        }
        format!("{}e{}", w, l)
    } else {
        format!("{}", w)
    }
}

// kissat_format_count
pub fn format_count(format: &mut Format, w: u64) -> String {
    let i = next_format_string(format);
    format.str_[i] = format_count_str(w);
    format.str_[i].clone()
}

// kissat_format_value
pub fn format_value(format: &mut Format, boolean: bool, value: i32) -> String {
    if boolean && value != 0 {
        return "true".to_string();
    }
    if boolean && value == 0 {
        return "false".to_string();
    }
    if value == i32::MAX {
        return "INT_MAX".to_string();
    }
    if value == i32::MIN {
        return "INT_MIN".to_string();
    }
    let i = next_format_string(format);
    if value < 0 {
        format.str_[i] = format!("-{}", format_count_str(value.unsigned_abs() as u64));
    } else {
        format.str_[i] = format_count_str(value as u64);
    }
    format.str_[i].clone()
}

// kissat_format_bytes
pub fn format_bytes(format: &mut Format, bytes: u64) -> String {
    let i = next_format_string(format);
    format.str_[i] = if bytes < (1u64 << 10) {
        format!("{} bytes", bytes)
    } else if bytes < (1u64 << 20) {
        format!("{} bytes ({} KB)", bytes, (bytes + (1u64 << 9)) >> 10)
    } else if bytes < (1u64 << 30) {
        format!("{} bytes ({} MB)", bytes, (bytes + (1u64 << 19)) >> 20)
    } else {
        format!("{} bytes ({} GB)", bytes, (bytes + (1u64 << 29)) >> 30)
    };
    format.str_[i].clone()
}

// kissat_format_time
pub fn format_time(format: &mut Format, seconds: f64) -> String {
    if seconds == 0.0 {
        return "0s".to_string();
    }
    let i = next_format_string(format);
    let mut rounded = seconds.round() as u64;
    let mut minutes = rounded / 60;
    rounded %= 60;
    let mut hours = minutes / 60;
    minutes %= 60;
    let days = hours / 24;
    hours %= 24;
    let mut res = String::new();
    if days != 0 {
        res.push_str(&format!("{}d", days));
    }
    if hours != 0 {
        if !res.is_empty() {
            res.push(' ');
        }
        res.push_str(&format!("{}h", hours));
    }
    if minutes != 0 {
        if !res.is_empty() {
            res.push(' ');
        }
        res.push_str(&format!("{}m", minutes));
    }
    if rounded != 0 {
        if !res.is_empty() {
            res.push(' ');
        }
        res.push_str(&format!("{}s", rounded));
    }
    format.str_[i] = res;
    format.str_[i].clone()
}

// kissat_format_signs
pub fn format_signs(format: &mut Format, size: u32, signs: u64) -> String {
    let i = next_format_string(format);
    debug_assert!((size as usize + 1) < FORMAT_STRING_SIZE);
    let mut res = String::new();
    let mut bit: u64 = 1;
    for _ in 0..size {
        res.push(if (bit & signs) != 0 { '1' } else { '0' });
        bit <<= 1;
    }
    format.str_[i] = res;
    format.str_[i].clone()
}

// kissat_format_ordinal
pub fn format_ordinal(format: &mut Format, ordinal: u64) -> String {
    let mod100 = ordinal % 100;
    let suffix = if (10..=19).contains(&mod100) {
        "th"
    } else {
        match mod100 % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    let i = next_format_string(format);
    format.str_[i] = format!("{}{}", ordinal, suffix);
    format.str_[i].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_matches_c() {
        // plain
        assert_eq!(format_count_str(0), "0");
        assert_eq!(format_count_str(127), "127");
        assert_eq!(format_count_str(999), "999");
        assert_eq!(format_count_str(1001), "1001");
        // powers of two from 128 up
        assert_eq!(format_count_str(128), "2^7");
        assert_eq!(format_count_str(64), "64"); // below 128 stays plain
        assert_eq!(format_count_str(1 << 20), "2^20");
        // multiples of 1000 strip all trailing zeros
        assert_eq!(format_count_str(1000), "1e3");
        assert_eq!(format_count_str(1500000), "15e5");
        assert_eq!(format_count_str(2000), "2e3");
    }

    #[test]
    fn time_matches_c() {
        let mut f = Format::default();
        assert_eq!(format_time(&mut f, 0.0), "0s");
        assert_eq!(format_time(&mut f, 1.4), "1s");
        assert_eq!(format_time(&mut f, 61.0), "1m 1s");
        assert_eq!(format_time(&mut f, 3600.0), "1h");
        assert_eq!(format_time(&mut f, 90061.0), "1d 1h 1m 1s");
    }

    #[test]
    fn bytes_matches_c() {
        let mut f = Format::default();
        assert_eq!(format_bytes(&mut f, 1023), "1023 bytes");
        assert_eq!(format_bytes(&mut f, 1024), "1024 bytes (1 KB)");
        assert_eq!(format_bytes(&mut f, 1536), "1536 bytes (2 KB)");
        assert_eq!(format_bytes(&mut f, 1 << 20), "1048576 bytes (1 MB)");
        assert_eq!(format_bytes(&mut f, 1 << 30), "1073741824 bytes (1 GB)");
    }

    #[test]
    fn ordinal_matches_c() {
        let mut f = Format::default();
        assert_eq!(format_ordinal(&mut f, 1), "1st");
        assert_eq!(format_ordinal(&mut f, 2), "2nd");
        assert_eq!(format_ordinal(&mut f, 3), "3rd");
        assert_eq!(format_ordinal(&mut f, 4), "4th");
        assert_eq!(format_ordinal(&mut f, 11), "11th");
        assert_eq!(format_ordinal(&mut f, 12), "12th");
        assert_eq!(format_ordinal(&mut f, 21), "21st");
        assert_eq!(format_ordinal(&mut f, 111), "111th");
    }

    #[test]
    fn value_matches_c() {
        let mut f = Format::default();
        assert_eq!(format_value(&mut f, true, 1), "true");
        assert_eq!(format_value(&mut f, true, 0), "false");
        assert_eq!(format_value(&mut f, false, i32::MAX), "INT_MAX");
        assert_eq!(format_value(&mut f, false, i32::MIN), "INT_MIN");
        assert_eq!(format_value(&mut f, false, -2000), "-2e3");
        assert_eq!(format_value(&mut f, false, 42), "42");
    }
}
