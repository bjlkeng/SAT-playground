// Port of src/proof.c + src/proof.h (kissat 4.0.4).
//
// PORT NOTE: proof.c does `#undef NDEBUG` before its own code, so the
// reference binary compiles the `units`/`imported`/`empty` self-check
// machinery (resize_proof_units, check_repeated_proof_lines) even in the
// NDEBUG build.  That state feeds asserts only — it never changes the
// emitted proof bytes or the printed statistics — so per CONVENTIONS.md it
// is omitted here.
// PORT NOTE: C `struct proof` carries a `kissat *solver` back-pointer used
// for GET_OPTION (flushproof) and allocation; the Rust port passes
// `&Solver` into the helpers instead (identical read points).
// PORT NOTE: C's `proof->file` is a borrowed `file *` owned by
// application.c (opened before kissat_init_proof, closed after
// kissat_release_proof).  Rust `Proof` owns its `crate::file::File`;
// `release_proof` flushes exactly like C and returns the `File` so the
// application can close it (same close point as application.c).
// PORT NOTE: the C write_buffer is a fixed 1 MiB char array plus `pos`;
// here a Vec<u8> with that capacity, flushed when it reaches SIZE_BUFFER —
// identical byte stream and flush boundaries.

use crate::file::File;
use crate::internal::Solver;
use crate::reference::Reference;

// #define size_buffer (1u << 20)
const SIZE_BUFFER: usize = 1 << 20;

/// Port of `struct proof`.
pub struct Proof {
    buffer: Vec<u8>, // struct write_buffer { chars[size_buffer]; pos; }
    binary: bool,
    file: File,
    line: Vec<i32>, // ints line;
    added: u64,
    deleted: u64,
    lines: u64,
    literals: u64,
}

/// Port of `kissat_init_proof`.
pub fn init_proof(solver: &mut Solver, file: File, binary: bool) {
    debug_assert!(solver.proof.is_none());
    let proof = Box::new(Proof {
        buffer: Vec::with_capacity(SIZE_BUFFER),
        binary,
        file,
        line: Vec::new(),
        added: 0,
        deleted: 0,
        lines: 0,
        literals: 0,
    });
    solver.proof = Some(proof);
}

// flush_buffer (static)
fn flush_buffer(proof: &mut Proof) {
    let bytes = proof.buffer.len();
    if bytes == 0 {
        return;
    }
    let written = crate::file::write(&mut proof.file, &proof.buffer);
    if bytes != written {
        crate::error::fatal(format_args!(
            "flushing {} bytes in proof write-buffer failed",
            bytes
        ));
    }
    proof.buffer.clear();
}

/// Port of `kissat_release_proof`.  Returns the flushed `File` for the
/// application to close (see PORT NOTE above on file ownership).
pub fn release_proof(solver: &mut Solver) -> File {
    let mut proof = solver.proof.take().expect("proof");
    flush_buffer(&mut proof);
    crate::file::flush(&mut proof.file);
    // RELEASE_STACK (proof->line) / kissat_free — dropped with the Box.
    proof.file
}

// PRINT_STAT (statistics.h) — local copy of the private helper in
// statistics.rs, byte-identical to the C macro expansion
// (SFW1=30, SFW2=12, SFW34=16, SFW34EXTENDED=19).
fn print_stat(
    prefix: &str,
    name: &str,
    primary: u64,
    secondary: f64,
    units: Option<&str>,
    type_: Option<&str>,
) {
    print!("{}{:<30} {:>12} ", prefix, format!("{}:", name), primary);
    if let Some(type_) = type_ {
        if secondary >= 0.0 {
            if let Some(units) = units {
                print!("{:>16.0} {:<2}", secondary, units);
            } else {
                print!("{:>19.2}", secondary);
            }
            print!(" {}", type_);
        }
    }
    println!();
}

/// Port of `kissat_print_proof_statistics` (QUIET not defined).
pub fn print_proof_statistics(solver: &Solver, verbose: bool) {
    let proof = solver.proof.as_ref().expect("proof");
    let prefix: &str = &solver.prefix;
    // PERCENT_LINES (NAME) = kissat_percent (proof->NAME, proof->lines)
    print_stat(
        prefix,
        "proof_added",
        proof.added,
        crate::utilities::percent(proof.added as f64, proof.lines as f64),
        Some("%"),
        Some("per line"),
    );
    print_stat(
        prefix,
        "proof_bytes",
        proof.file.bytes,
        proof.file.bytes as f64 / (1u64 << 20) as f64,
        Some("MB"),
        Some(""),
    );
    print_stat(
        prefix,
        "proof_deleted",
        proof.deleted,
        crate::utilities::percent(proof.deleted as f64, proof.lines as f64),
        Some("%"),
        Some("per line"),
    );
    if verbose {
        print_stat(prefix, "proof_lines", proof.lines, 100.0, Some("%"), Some(""));
    }
    if verbose {
        print_stat(
            prefix,
            "proof_literals",
            proof.literals,
            crate::utilities::average(proof.literals as f64, proof.lines as f64),
            Some(""),
            Some("per line"),
        );
    }
}

// write_char (static inline)
#[inline]
fn write_char(proof: &mut Proof, ch: u8) {
    if proof.buffer.len() == SIZE_BUFFER {
        flush_buffer(proof);
    }
    proof.buffer.push(ch);
}

// import_internal_proof_literal (static inline).  The proof line stores
// EXTERNAL literals: every internal literal is exported here.
#[inline]
fn import_internal_proof_literal(solver: &Solver, proof: &mut Proof, ilit: u32) {
    let elit = crate::inline::export_literal(solver, ilit);
    debug_assert!(elit != 0);
    proof.line.push(elit);
    proof.literals += 1;
}

// import_external_proof_literal (static inline)
#[inline]
fn import_external_proof_literal(proof: &mut Proof, elit: i32) {
    debug_assert!(elit != 0);
    proof.line.push(elit);
    proof.literals += 1;
}

// import_internal_proof_binary (static)
fn import_internal_proof_binary(solver: &Solver, proof: &mut Proof, a: u32, b: u32) {
    debug_assert!(proof.line.is_empty());
    import_internal_proof_literal(solver, proof, a);
    import_internal_proof_literal(solver, proof, b);
}

// import_internal_proof_literals (static)
fn import_internal_proof_literals(solver: &Solver, proof: &mut Proof, ilits: &[u32]) {
    debug_assert!(proof.line.is_empty());
    debug_assert!(ilits.len() <= u32::MAX as usize);
    for &ilit in ilits {
        import_internal_proof_literal(solver, proof, ilit);
    }
}

// import_external_proof_literals (static)
fn import_external_proof_literals(proof: &mut Proof, elits: &[i32]) {
    debug_assert!(proof.line.is_empty());
    debug_assert!(elits.len() <= u32::MAX as usize);
    for &elit in elits {
        import_external_proof_literal(proof, elit);
    }
}

// import_proof_clause (static).  C takes `const clause *c`; the crate
// convention passes the arena Reference.
fn import_proof_clause(solver: &Solver, proof: &mut Proof, ref_: Reference) {
    let lits = solver.arena.clause(ref_).lits();
    import_internal_proof_literals(solver, proof, lits);
}

// print_binary_proof_line (static): DRAT binary variable-byte encoding.
// Each external literal maps to x = 2*|elit| + (elit < 0), emitted LSB
// first, 7 bits per byte, high bit set on all but the last byte; the line
// is terminated by a single 0 byte.
fn print_binary_proof_line(proof: &mut Proof) {
    debug_assert!(proof.binary);
    for i in 0..proof.line.len() {
        let elit = proof.line[i];
        let mut x: u32 = 2u32 * elit.unsigned_abs() + (elit < 0) as u32;
        while x & !0x7f != 0 {
            let ch = ((x & 0x7f) | 0x80) as u8;
            write_char(proof, ch);
            x >>= 7;
        }
        write_char(proof, x as u8);
    }
    write_char(proof, 0);
}

// print_non_binary_proof_line (static): ASCII DRAT — literals as signed
// decimals separated by single spaces, each line "lit ... 0\n" (note the
// space after every literal, before the terminating '0').
fn print_non_binary_proof_line(proof: &mut Proof) {
    debug_assert!(!proof.binary);
    let mut buffer = [0u8; 16];
    for i in 0..proof.line.len() {
        let elit = proof.line[i];
        debug_assert!(elit != 0);
        debug_assert!(elit != i32::MIN);
        let eidx: u32;
        if elit < 0 {
            write_char(proof, b'-');
            eidx = elit.unsigned_abs();
        } else {
            eidx = elit as u32;
        }
        // Digits generated backwards into the buffer exactly as in C.
        let mut p = buffer.len();
        let mut tmp = eidx;
        while tmp != 0 {
            p -= 1;
            buffer[p] = b'0' + (tmp % 10) as u8;
            tmp /= 10;
        }
        while p != buffer.len() {
            write_char(proof, buffer[p]);
            p += 1;
        }
        write_char(proof, b' ');
    }
    write_char(proof, b'0');
    write_char(proof, b'\n');
}

// print_proof_line (static)
fn print_proof_line(solver: &Solver, proof: &mut Proof) {
    proof.lines += 1;
    if proof.binary {
        print_binary_proof_line(proof);
    } else {
        print_non_binary_proof_line(proof);
    }
    proof.line.clear();
    if solver.options.flushproof != 0 {
        flush_buffer(proof);
        crate::file::flush(&mut proof.file);
    }
}

// print_added_proof_line (static)
fn print_added_proof_line(solver: &Solver, proof: &mut Proof) {
    proof.added += 1;
    // check_repeated_proof_lines: assert-only (see header PORT NOTE).
    if proof.binary {
        write_char(proof, b'a');
    }
    print_proof_line(solver, proof);
}

// print_delete_proof_line (static)
fn print_delete_proof_line(solver: &Solver, proof: &mut Proof) {
    proof.deleted += 1;
    write_char(proof, b'd');
    if !proof.binary {
        write_char(proof, b' ');
    }
    print_proof_line(solver, proof);
}

// PORT NOTE (all emission functions): the C functions read
// `proof = solver->proof` and then use both; the Rust port takes the boxed
// Proof out of the solver for the call and puts it back, so the shared
// `&Solver` reads (export table, values, arena, options) borrow-check.
// Order of effects is identical.

/// Port of `kissat_add_binary_to_proof`.
pub fn add_binary_to_proof(solver: &mut Solver, a: u32, b: u32) {
    let mut proof = solver.proof.take().expect("proof");
    import_internal_proof_binary(solver, &mut proof, a, b);
    print_added_proof_line(solver, &mut proof);
    solver.proof = Some(proof);
}

/// Port of `kissat_add_clause_to_proof` (C passes `const clause *`).
pub fn add_clause_to_proof(solver: &mut Solver, ref_: Reference) {
    let mut proof = solver.proof.take().expect("proof");
    import_proof_clause(solver, &mut proof, ref_);
    print_added_proof_line(solver, &mut proof);
    solver.proof = Some(proof);
}

/// Port of `kissat_add_empty_to_proof`.
pub fn add_empty_to_proof(solver: &mut Solver) {
    let mut proof = solver.proof.take().expect("proof");
    debug_assert!(proof.line.is_empty());
    print_added_proof_line(solver, &mut proof);
    solver.proof = Some(proof);
}

/// Port of `kissat_add_lits_to_proof` (size + pointer folded into a slice).
pub fn add_lits_to_proof(solver: &mut Solver, ilits: &[u32]) {
    let mut proof = solver.proof.take().expect("proof");
    import_internal_proof_literals(solver, &mut proof, ilits);
    print_added_proof_line(solver, &mut proof);
    solver.proof = Some(proof);
}

/// Port of `kissat_add_unit_to_proof`.
pub fn add_unit_to_proof(solver: &mut Solver, ilit: u32) {
    let mut proof = solver.proof.take().expect("proof");
    debug_assert!(proof.line.is_empty());
    import_internal_proof_literal(solver, &mut proof, ilit);
    print_added_proof_line(solver, &mut proof);
    solver.proof = Some(proof);
}

/// Port of `kissat_shrink_clause_in_proof`: emit the shrunken clause
/// (skipping `remove` and root-level falsified literals other than `keep`),
/// then delete the original.
pub fn shrink_clause_in_proof(solver: &mut Solver, ref_: Reference, remove: u32, keep: u32) {
    let mut proof = solver.proof.take().expect("proof");
    debug_assert!(proof.line.is_empty());
    for &ilit in solver.arena.clause(ref_).lits() {
        if ilit == remove {
            continue;
        }
        // C: ilit != keep && values[ilit] < 0 && !LEVEL (ilit)
        if ilit != keep
            && solver.values[ilit as usize] < 0
            && solver.assigned[crate::literal::idx(ilit) as usize].level == 0
        {
            continue;
        }
        import_internal_proof_literal(solver, &mut proof, ilit);
    }
    print_added_proof_line(solver, &mut proof);
    import_proof_clause(solver, &mut proof, ref_);
    print_delete_proof_line(solver, &mut proof);
    solver.proof = Some(proof);
}

/// Port of `kissat_delete_binary_from_proof`.
pub fn delete_binary_from_proof(solver: &mut Solver, a: u32, b: u32) {
    let mut proof = solver.proof.take().expect("proof");
    import_internal_proof_binary(solver, &mut proof, a, b);
    print_delete_proof_line(solver, &mut proof);
    solver.proof = Some(proof);
}

/// Port of `kissat_delete_clause_from_proof` (C passes `const clause *`).
pub fn delete_clause_from_proof(solver: &mut Solver, ref_: Reference) {
    let mut proof = solver.proof.take().expect("proof");
    import_proof_clause(solver, &mut proof, ref_);
    print_delete_proof_line(solver, &mut proof);
    solver.proof = Some(proof);
}

/// Port of `kissat_delete_external_from_proof` — the literals are already
/// external and go on the line unmapped.
pub fn delete_external_from_proof(solver: &mut Solver, elits: &[i32]) {
    let mut proof = solver.proof.take().expect("proof");
    import_external_proof_literals(&mut proof, elits);
    print_delete_proof_line(solver, &mut proof);
    solver.proof = Some(proof);
}

/// Port of `kissat_delete_internal_from_proof`.
pub fn delete_internal_from_proof(solver: &mut Solver, ilits: &[u32]) {
    let mut proof = solver.proof.take().expect("proof");
    import_internal_proof_literals(solver, &mut proof, ilits);
    print_delete_proof_line(solver, &mut proof);
    solver.proof = Some(proof);
}
