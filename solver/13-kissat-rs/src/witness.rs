// Port of src/witness.c + src/witness.h (kissat 4.0.4).
//
// PORT NOTE: C prints through the (FILE-buffered) global stdout with
// fputs/fputc; here a single BufWriter over the locked stdout handle is
// created in `print_witness` and threaded through the private helpers
// (C's globals-based helpers gain the writer parameter). Output bytes are
// identical: each line is "v" + buffered chunk + "\n".
// PORT NOTE: C's `print_int` takes the solver only for its stack
// allocator (PUSH_STACK); the Rust version drops that parameter.
// PORT NOTE: The line-length rule is verbatim from witness.c: flush before
// appending when the pending payload would exceed 77 characters, keeping
// every "v" line at most 78 characters wide — comfortably inside the
// 4096-character limit of the competition output format.
// PORT NOTE: Rust's std::process::exit does not flush stdout buffers (C's
// exit does), so the writer is flushed explicitly before returning.

use std::io::Write;

use crate::internal::Solver;

// flush_buffer (static in witness.c)
fn flush_buffer(out: &mut impl Write, buffer: &mut Vec<u8>) {
    let _ = out.write_all(b"v");
    let _ = out.write_all(buffer);
    let _ = out.write_all(b"\n");
    buffer.clear();
}

// print_int (static in witness.c)
fn print_int(out: &mut impl Write, buffer: &mut Vec<u8>, i: i32) {
    let tmp = format!(" {}", i);
    let buf_len = buffer.len();
    if buf_len + tmp.len() > 77 {
        flush_buffer(out, buffer);
    }
    buffer.extend_from_slice(tmp.as_bytes());
}

// kissat_print_witness
pub fn print_witness(solver: &mut Solver, max_var: i32, partial: bool) {
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let mut buffer: Vec<u8> = Vec::new();
    for eidx in 1..=max_var {
        let mut tmp = crate::internal::value(solver, eidx);
        if tmp == 0 && !partial {
            tmp = eidx;
        }
        if tmp != 0 {
            print_int(&mut out, &mut buffer, tmp);
        }
    }
    print_int(&mut out, &mut buffer, 0);
    debug_assert!(!buffer.is_empty());
    flush_buffer(&mut out, &mut buffer);
    let _ = out.flush();
}
