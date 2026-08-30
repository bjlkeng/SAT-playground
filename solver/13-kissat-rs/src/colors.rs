// Port of src/colors.h + colors.c (kissat 4.0.4).

use std::sync::atomic::{AtomicI8, Ordering};

pub const BLUE: &str = "\x1b[34m";
pub const BOLD: &str = "\x1b[1m";
pub const CYAN: &str = "\x1b[36m";
pub const GREEN: &str = "\x1b[32m";
pub const MAGENTA: &str = "\x1b[35m";
pub const NORMAL: &str = "\x1b[0m";
pub const RED: &str = "\x1b[31m";
// C defines WHITE as "\037[34m" (a typo in kissat, kept for fidelity; unused).
pub const WHITE: &str = "\x1f[34m";
pub const YELLOW: &str = "\x1b[33m";

pub const LIGHT_GRAY: &str = "\x1b[1;37m";
pub const DARK_GRAY: &str = "\x1b[0;37m";

// kissat_is_terminal[3] = {0, -1, -1}; index by fd (1=stdout, 2=stderr).
static IS_TERMINAL: [AtomicI8; 3] = [AtomicI8::new(0), AtomicI8::new(-1), AtomicI8::new(-1)];

fn isatty(fd: i32) -> bool {
    // SAFETY: isatty is async-signal-safe and takes any fd.
    extern "C" {
        fn isatty(fd: i32) -> i32;
    }
    unsafe { isatty(fd) != 0 }
}

pub fn initialize_terminal(fd: usize) -> i8 {
    debug_assert!(fd == 1 || fd == 2);
    let res = isatty(fd as i32) as i8;
    IS_TERMINAL[fd].store(res, Ordering::Relaxed);
    res
}

pub fn force_colors() {
    IS_TERMINAL[1].store(1, Ordering::Relaxed);
    IS_TERMINAL[2].store(1, Ordering::Relaxed);
}

pub fn force_no_colors() {
    IS_TERMINAL[1].store(0, Ordering::Relaxed);
    IS_TERMINAL[2].store(0, Ordering::Relaxed);
}

#[inline]
pub fn connected_to_terminal(fd: usize) -> bool {
    debug_assert!(fd == 1 || fd == 2);
    let mut res = IS_TERMINAL[fd].load(Ordering::Relaxed);
    if res < 0 {
        res = initialize_terminal(fd);
    }
    res == 1
}

pub fn bold_green_color_code(fd: usize) -> &'static str {
    if connected_to_terminal(fd) {
        "\x1b[1m\x1b[32m"
    } else {
        ""
    }
}

pub fn normal_color_code(fd: usize) -> &'static str {
    if connected_to_terminal(fd) {
        NORMAL
    } else {
        ""
    }
}
