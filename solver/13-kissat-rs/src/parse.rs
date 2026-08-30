// Port of src/parse.c + src/parse.h (kissat 4.0.4).
//
// PORT NOTE: The reference build has EMBEDDED undefined, so the embedded
// option-parsing block in header comments (`c --opt=val`) is compiled out
// and is not ported.
// PORT NOTE: C has a static `parse_dimacs` wrapped by the exported
// `kissat_parse_dimacs`; dropping the `kissat_` prefix would collide, so
// the private worker keeps its name with a trailing underscore
// (`parse_dimacs_`), per the keyword-rename convention.
// PORT NOTE: The NONL macro (adjust lineno if the offending char was a
// newline, publish lineno, return message) is the `nonl` helper; the C
// `goto START` / `goto COMPLETE` comment-skipping control flow is
// restructured with labeled loops without changing the order of effects.
// PORT NOTE: START(parse)/STOP(parse) from profile.h are expanded inline;
// `kissat_start (solver, profile)` takes a profile pointer, which in Rust
// would alias `solver`, so `crate::profile::start`/`stop` are assumed to
// take an accessor `fn (&mut Solver) -> &mut Profile`.
// PORT NOTE: kissat_message's printf varargs become `format_args!` at the
// call site (`crate::print::message`).
// PORT NOTE: The 1 MiB read buffer lives on the C stack; here it is
// heap-allocated (Box) to keep Rust stacks safe. Capacity/refill behaviour
// is identical, and `next` is #[inline(always)] like the C
// ATTRIBUTE_ALWAYS_INLINE `next`, so the hot literal-scanning loop stays a
// buffered byte scanner as in C.

use crate::file::{self, File, EOF};
use crate::internal::Solver;
use crate::literal::EXTERNAL_MAX_VAR;

// enum strictness (parse.h)
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Strictness {
    RELAXED_PARSING = 0,
    NORMAL_PARSING = 1,
    PEDANTIC_PARSING = 2,
}

pub use Strictness::{NORMAL_PARSING, PEDANTIC_PARSING, RELAXED_PARSING};

const SIZE_BUFFER: usize = 1 << 20;

struct ReadBuffer {
    chars: Box<[u8]>,
    pos: usize,
    end: usize,
}

impl ReadBuffer {
    fn new() -> ReadBuffer {
        ReadBuffer {
            chars: vec![0u8; SIZE_BUFFER].into_boxed_slice(),
            pos: 0,
            end: 0,
        }
    }
}

fn fill_buffer(buffer: &mut ReadBuffer, file: &mut File) -> usize {
    buffer.pos = 0;
    buffer.end = file::read(file, &mut buffer.chars);
    buffer.end
}

#[inline(always)]
fn next(buffer: &mut ReadBuffer, file: &mut File, lineno_ptr: &mut u64) -> i32 {
    if buffer.pos == buffer.end && fill_buffer(buffer, file) == 0 {
        return EOF;
    }
    let ch = buffer.chars[buffer.pos] as i32;
    buffer.pos += 1;
    if ch == b'\n' as i32 {
        *lineno_ptr += 1;
    }
    ch
}

// NONL macro from parse.c.
#[inline]
fn nonl(
    msg: &'static str,
    ch: i32,
    lineno: &mut u64,
    lineno_ptr: &mut u64,
) -> Option<&'static str> {
    if ch == b'\n' as i32 {
        debug_assert!(*lineno > 0);
        *lineno -= 1;
    }
    *lineno_ptr = *lineno;
    Some(msg)
}

#[inline(always)]
fn faster_is_digit(ch: i32) -> bool {
    b'0' as i32 <= ch && ch <= b'9' as i32
}

fn parse_dimacs_(
    solver: &mut Solver,
    file: &mut File,
    strict: Strictness,
    lineno_ptr: &mut u64,
    max_var_ptr: &mut i32,
) -> Option<&'static str> {
    let mut buffer = ReadBuffer::new();
    let mut lineno: u64 = 1;
    *lineno_ptr = 1;
    let mut first = true;
    let mut ch: i32;
    macro_rules! next {
        () => {
            next(&mut buffer, file, &mut lineno)
        };
    }
    'header: loop {
        ch = next!();
        if ch == b'p' as i32 {
            break;
        } else if ch == EOF {
            if first {
                return Some("empty file");
            } else {
                return Some("end-of-file before header");
            }
        }
        first = false;
        if ch == b'\r' as i32 {
            ch = next!();
            if ch != b'\n' as i32 {
                return Some("expected new-line after carriage-return");
            }
            if strict == PEDANTIC_PARSING {
                return nonl("unexpected empty line", ch, &mut lineno, lineno_ptr);
            }
        } else if ch == b'\n' as i32 {
            if strict == PEDANTIC_PARSING {
                return nonl("unexpected empty line", ch, &mut lineno, lineno_ptr);
            }
        } else if ch == b'c' as i32 {
            // C label `START:` — leading white space of a header comment.
            loop {
                ch = next!();
                if ch == b'\n' as i32 {
                    continue 'header;
                } else if ch == b'\r' as i32 {
                    ch = next!();
                    if ch != b'\n' as i32 {
                        return Some("expected new-line after carriage-return");
                    }
                    continue 'header;
                } else if ch == EOF {
                    return Some("end-of-file in header comment");
                } else if ch == b' ' as i32 || ch == b'\t' as i32 {
                    continue; // goto START
                } else {
                    // while ((ch = NEXT ()) != '\n') ...
                    loop {
                        ch = next!();
                        if ch == b'\n' as i32 {
                            break;
                        }
                        if ch == EOF {
                            return Some("end-of-file in header comment");
                        } else if ch == b'\r' as i32 {
                            ch = next!();
                            if ch != b'\n' as i32 {
                                return Some("expected new-line after carriage-return");
                            }
                            break;
                        }
                    }
                    continue 'header;
                }
            }
        } else {
            return Some("expected 'c' or 'p' at start of line");
        }
    }
    debug_assert!(ch == b'p' as i32);
    ch = next!();
    if ch != b' ' as i32 {
        return nonl("expected space after 'p'", ch, &mut lineno, lineno_ptr);
    }
    ch = next!();
    if strict != PEDANTIC_PARSING {
        while ch == b' ' as i32 || ch == b'\t' as i32 {
            ch = next!();
        }
    }
    if ch != b'c' as i32 {
        return nonl("expected 'c' after 'p '", ch, &mut lineno, lineno_ptr);
    }
    ch = next!();
    if ch != b'n' as i32 {
        return nonl("expected 'n' after 'p c'", ch, &mut lineno, lineno_ptr);
    }
    ch = next!();
    if ch != b'f' as i32 {
        // PORT NOTE: message quirk ('n' instead of 'f') is verbatim from C.
        return nonl("expected 'n' after 'p cn'", ch, &mut lineno, lineno_ptr);
    }
    ch = next!();
    if ch != b' ' as i32 {
        return nonl("expected space after 'p cnf'", ch, &mut lineno, lineno_ptr);
    }
    ch = next!();
    if strict != PEDANTIC_PARSING {
        while ch == b' ' as i32 || ch == b'\t' as i32 {
            ch = next!();
        }
    }
    if !faster_is_digit(ch) {
        return nonl("expected digit after 'p cnf '", ch, &mut lineno, lineno_ptr);
    }
    let mut variables: i32 = ch - b'0' as i32;
    loop {
        ch = next!();
        if !faster_is_digit(ch) {
            break;
        }
        if EXTERNAL_MAX_VAR / 10 < variables {
            return Some("maximum variable too large");
        }
        variables *= 10;
        let digit: i32 = ch - b'0' as i32;
        if EXTERNAL_MAX_VAR - digit < variables {
            return Some("maximum variable too large");
        }
        variables += digit;
    }
    if ch == EOF {
        return Some("unexpected end-of-file while parsing maximum variable");
    }
    if ch == b'\r' as i32 {
        ch = next!();
        if ch != b'\n' as i32 {
            return Some("expected new-line after carriage-return");
        }
    }
    if ch == b'\n' as i32 {
        return nonl(
            "unexpected new-line after maximum variable",
            ch,
            &mut lineno,
            lineno_ptr,
        );
    }
    if ch != b' ' as i32 {
        return Some("expected space after maximum variable");
    }
    ch = next!();
    if strict != PEDANTIC_PARSING {
        while ch == b' ' as i32 || ch == b'\t' as i32 {
            ch = next!();
        }
    }
    if !faster_is_digit(ch) {
        return Some("expected number of clauses after maximum variable");
    }
    let mut clauses: u64 = (ch - b'0' as i32) as u64;
    loop {
        ch = next!();
        if !faster_is_digit(ch) {
            break;
        }
        if u64::MAX / 10 < clauses {
            return Some("number of clauses too large");
        }
        clauses *= 10;
        let digit: u64 = (ch - b'0' as i32) as u64;
        if u64::MAX - digit < clauses {
            return Some("number of clauses too large");
        }
        clauses += digit;
    }
    if ch == EOF {
        return Some("unexpected end-of-file while parsing number of clauses");
    }
    if strict != PEDANTIC_PARSING {
        while ch == b' ' as i32 || ch == b'\t' as i32 {
            ch = next!();
        }
    }
    if ch == b'\r' as i32 {
        ch = next!();
        if ch != b'\n' as i32 {
            return Some("expected new-line after carriage-return");
        }
    }
    if ch == EOF {
        return Some("unexpected end-of-file after parsing number of clauses");
    }
    if ch != b'\n' as i32 {
        return Some("expected new-line after parsing number of clauses");
    }
    crate::print::message(
        solver,
        format_args!("parsed 'p cnf {} {}' header", variables, clauses),
    );
    *max_var_ptr = variables;
    crate::internal::reserve(solver, variables);
    let mut parsed: u64 = 0;
    let mut lit: i32 = 0;
    loop {
        ch = next!();
        if ch == b' ' as i32 {
            continue;
        }
        if ch == b'\t' as i32 {
            continue;
        }
        if ch == b'\n' as i32 {
            continue;
        }
        if ch == b'\r' as i32 {
            ch = next!();
            if ch != b'\n' as i32 {
                return Some("expected new-line after carriage-return");
            }
            continue;
        }
        if ch == b'c' as i32 {
            loop {
                ch = next!();
                if ch == b'\n' as i32 {
                    break;
                }
                if ch == EOF {
                    if strict != PEDANTIC_PARSING {
                        break;
                    }
                    return Some("unexpected end-of-file in comment after header");
                }
            }
            if ch == EOF {
                break;
            }
            continue;
        }
        if ch == EOF {
            break;
        }
        let sign: i32;
        if ch == b'-' as i32 {
            ch = next!();
            if ch == EOF {
                return Some("unexpected end-of-file after '-'");
            }
            if ch == b'\n' as i32 {
                return nonl("unexpected new-line after '-'", ch, &mut lineno, lineno_ptr);
            }
            if !faster_is_digit(ch) {
                return Some("expected digit after '-'");
            }
            if ch == b'0' as i32 {
                return Some("expected non-zero digit after '-'");
            }
            sign = -1;
        } else if !faster_is_digit(ch) {
            return Some("expected digit or '-'");
        } else {
            sign = 1;
        }
        debug_assert!(faster_is_digit(ch));
        let mut idx: i32 = ch - b'0' as i32;
        loop {
            ch = next!();
            if !faster_is_digit(ch) {
                break;
            }
            if EXTERNAL_MAX_VAR / 10 < idx {
                return Some("variable index too large");
            }
            idx *= 10;
            let digit: i32 = ch - b'0' as i32;
            if EXTERNAL_MAX_VAR - digit < idx {
                return Some("variable index too large");
            }
            idx += digit;
        }
        if ch == EOF {
            if strict == PEDANTIC_PARSING {
                if idx != 0 {
                    return Some("unexpected end-of-file after literal");
                } else {
                    return Some("unexpected end-of-file after trailing zero");
                }
            }
        } else if ch == b'\r' as i32 {
            ch = next!();
            if ch != b'\n' as i32 {
                return Some("expected new-line after carriage-return");
            }
        } else if ch == b'c' as i32 {
            loop {
                ch = next!();
                if ch == b'\n' as i32 {
                    break;
                }
                if ch == EOF {
                    if strict != PEDANTIC_PARSING {
                        break;
                    }
                    return Some("unexpected end-of-file in comment after literal");
                }
            }
        } else if ch != b' ' as i32 && ch != b'\t' as i32 && ch != b'\n' as i32 {
            return Some("expected white space after literal");
        }
        if strict != RELAXED_PARSING && idx > variables {
            return nonl(
                "maximum variable index exceeded (try '--relaxed' parsing)",
                ch,
                &mut lineno,
                lineno_ptr,
            );
        }
        if idx != 0 {
            debug_assert!(sign == 1 || sign == -1);
            lit = sign * idx;
        } else {
            if strict != RELAXED_PARSING && parsed == clauses {
                return Some("too many clauses (try '--relaxed' parsing)");
            }
            parsed += 1;
            lit = 0;
        }
        crate::internal::add(solver, lit);
    }
    if lit != 0 {
        return Some("trailing zero missing");
    }
    if strict != RELAXED_PARSING && parsed < clauses {
        if parsed + 1 == clauses {
            return Some("one clause missing (try '--relaxed' parsing)");
        }
        return Some("more than one clause missing (try '--relaxed' parsing)");
    }

    *lineno_ptr = lineno;

    None
}

// kissat_parse_dimacs
pub fn parse_dimacs(
    solver: &mut Solver,
    strict: Strictness,
    file: &mut File,
    lineno_ptr: &mut u64,
    max_var_ptr: &mut i32,
) -> Option<&'static str> {
    // START (parse);
    crate::profile::start_checked(solver, crate::profile::Prof::parse);
    let res = parse_dimacs_(solver, file, strict, lineno_ptr, max_var_ptr);
    if !solver.inconsistent {
        crate::collect::defrag_watches(solver);
    }
    // STOP (parse);
    crate::profile::stop_checked(solver, crate::profile::Prof::parse);
    res
}
