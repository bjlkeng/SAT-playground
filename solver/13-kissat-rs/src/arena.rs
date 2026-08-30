// Port of src/arena.h + src/arena.c (kissat 4.0.4).
//
// Build configuration: 64-bit, COMPACT **not** defined.  Hence
//   ward == w2rd == uintptr_t[2] == 16 bytes,
//   LD_MAX_ARENA == LD_MAX_REF == 31, MAX_ARENA == 2^31 wards.
//
// The C arena is a STACK (ward) and a `reference` indexes *wards*.  Here the
// arena is a Vec<u32> of words with the invariant len % 4 == 0; reference `r`
// addresses word offset 4*r (WORDS_PER_WARD).  All clause layout math is in
// u32 words (see clause.rs).
//
// PORT NOTES:
//  - The C stack growth policy is observable (enlarge cadence drives phase
//    messages and the MAX_ARENA fatal), so CAPACITY_STACK is mirrored in
//    `capacity_wards` and replicated exactly: ENLARGE_STACK doubles, with the
//    quirky initial capacity of BYTES_PER_ELEMENT elements == sizeof (ward)
//    == 16 wards; SHRINK_STACK shrinks to the power-of-two ceiling of size.
//  - report_resized's "moved"/"in place" comes from comparing the Vec buffer
//    pointer around the reallocation, mirroring the C begin-pointer compare.
//    Vec/realloc may of course differ in when they move; output-only.
//  - INC/GET on arena_resized/arena_enlarged/arena_shrunken are METRIC
//    counters, compiled out in the reference build (neither METRICS nor
//    STATISTICS defined); GET (arena_resized) therefore yields UINT64_MAX,
//    which kissat_phase renders as "no count" — hardcoded as u64::MAX below.
//  - kissat_clause_in_arena is !NDEBUG/LOGGING only: not ported.
//  - New arena words are zero-filled (C leaves them uninitialized; they are
//    always overwritten by init_clause + memcpy before any read).

use crate::clause::{self, ClauseMut, ClauseRef};
use crate::internal::Solver;
use crate::reference::{Reference, MAX_REF};

/// One ward (16 bytes) in u32 words.
pub const WORDS_PER_WARD: usize = 4;
/// sizeof (ward) == sizeof (w2rd) == 16.
pub const BYTES_PER_WARD: usize = 16;

pub const LD_MAX_ARENA: u32 = 31; // == LD_MAX_REF (64-bit, non-COMPACT)
pub const MAX_ARENA: u64 = 1u64 << LD_MAX_ARENA; // in wards

/// kissat_align_ward == kissat_align_w2rd (non-COMPACT): round up to 16.
#[inline]
pub fn align_ward(bytes: usize) -> usize {
    (bytes + (BYTES_PER_WARD - 1)) & !(BYTES_PER_WARD - 1)
}

// kissat_log2_floor_of_word / kissat_log2_ceiling_of_word (utilities.h).
#[inline]
fn log2_ceiling_u64(x: u64) -> u32 {
    if x == 0 {
        return 0;
    }
    let floor = 63 - x.leading_zeros();
    floor + if x != (1u64 << floor) { 1 } else { 0 }
}

#[derive(Default)]
pub struct Arena {
    words: Vec<u32>,
    /// CAPACITY_STACK mirror, in wards (see PORT NOTES above).
    capacity_wards: u64,
}

impl Arena {
    pub fn new() -> Self {
        Arena {
            words: Vec::new(),
            capacity_wards: 0,
        }
    }

    /// SIZE_STACK (solver->arena), in wards.
    #[inline]
    pub fn size_wards(&self) -> u64 {
        debug_assert!(self.words.len() % WORDS_PER_WARD == 0);
        (self.words.len() / WORDS_PER_WARD) as u64
    }

    /// CAPACITY_STACK (solver->arena), in wards.
    #[inline]
    pub fn capacity_wards(&self) -> u64 {
        self.capacity_wards
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Raw word access (BEGIN_STACK..END_STACK).
    #[inline]
    pub fn words(&self) -> &[u32] {
        &self.words
    }

    #[inline]
    pub fn words_mut(&mut self) -> &mut [u32] {
        &mut self.words
    }

    /// kissat_dereference_clause (== unchecked variant under NDEBUG).
    #[inline]
    pub fn clause(&self, ref_: Reference) -> ClauseRef<'_> {
        debug_assert!((ref_ as u64) < self.size_wards());
        ClauseRef {
            words: &self.words[ref_ as usize * WORDS_PER_WARD..],
        }
    }

    #[inline]
    pub fn clause_mut(&mut self, ref_: Reference) -> ClauseMut<'_> {
        debug_assert!((ref_ as u64) < self.size_wards());
        ClauseMut {
            words: &mut self.words[ref_ as usize * WORDS_PER_WARD..],
        }
    }

    /// kissat_next_clause as reference math: the reference of the clause
    /// following `ref_` (actual size includes shrunken-terminator scan).
    #[inline]
    pub fn next_clause_ref(&self, ref_: Reference) -> Reference {
        ref_ + (self.clause(ref_).actual_words() / WORDS_PER_WARD) as u32
    }

    /// RESIZE_STACK / SET_END_OF_STACK for the arena (used by collect).
    pub fn truncate_wards(&mut self, wards: u64) {
        debug_assert!(wards <= self.size_wards());
        self.words.truncate(wards as usize * WORDS_PER_WARD);
    }

    /// One ENLARGE_STACK step (doubling; initial capacity 16 wards).
    fn enlarge_stack(&mut self) {
        let new_capacity = if self.capacity_wards != 0 {
            2 * self.capacity_wards
        } else {
            BYTES_PER_WARD as u64 // ENLARGE_STACK quirk: BYTES_PER_ELEMENT
        };
        let want_words = new_capacity as usize * WORDS_PER_WARD;
        self.words.reserve_exact(want_words - self.words.len());
        self.capacity_wards = new_capacity;
    }

    /// SHRINK_STACK (stack.h) for the arena.
    fn shrink_stack(&mut self) {
        let size = self.size_wards();
        if size == self.capacity_wards {
            return; // FULL_STACK
        }
        let old_bytes = self.capacity_wards * BYTES_PER_WARD as u64;
        if size == 0 {
            self.words = Vec::new(); // kissat_free + INIT_STACK
            self.capacity_wards = 0;
            return;
        }
        if old_bytes <= 8 {
            return; // OLD_BYTES <= sizeof (void *)
        }
        let new_capacity = 1u64 << log2_ceiling_u64(size);
        let new_bytes = new_capacity * BYTES_PER_WARD as u64;
        if new_bytes == old_bytes {
            return;
        }
        debug_assert!(new_bytes < old_bytes);
        self.words.shrink_to(new_capacity as usize * WORDS_PER_WARD);
        self.capacity_wards = new_capacity;
    }
}

// kissat_percent (utilities.h).
#[inline]
fn percent(a: f64, b: f64) -> f64 {
    if b != 0.0 {
        100.0 * a / b
    } else {
        0.0
    }
}

fn report_resized(solver: &mut Solver, mode: &str, moved: bool) {
    // #ifndef QUIET (kept: QUIET not defined in the reference build).
    let capacity = solver.arena.capacity_wards();
    let bytes = capacity * BYTES_PER_WARD as u64;
    let capacity_str = crate::format::format_count(&mut solver.format, capacity);
    let bytes_str = crate::format::format_bytes(&mut solver.format, bytes);
    // GET (arena_resized): IGNOREd METRIC in reference build -> UINT64_MAX.
    crate::print::phase(
        solver,
        "arena",
        u64::MAX,
        format_args!(
            "{} to {} {}-byte-words {} ({})",
            mode,
            capacity_str,
            BYTES_PER_WARD,
            bytes_str,
            if moved { "moved" } else { "in place" }
        ),
    );
}

/// kissat_allocate_clause: reserves space for a clause of `size` literals and
/// returns its reference (the pre-allocation SIZE_STACK in wards).
pub fn allocate_clause(solver: &mut Solver, size: usize) -> Reference {
    debug_assert!(size <= u32::MAX as usize);
    let res = solver.arena.size_wards();
    debug_assert!(res <= MAX_REF as u64);
    let bytes = clause::bytes_of_clause(size as u32);
    debug_assert!(bytes % BYTES_PER_WARD == 0); // kissat_aligned_word
    let needed = (bytes / BYTES_PER_WARD) as u64;
    debug_assert!(needed <= u32::MAX as u64);
    let mut capacity = solver.arena.capacity_wards();
    debug_assert!(capacity <= MAX_ARENA);
    let mut available = capacity - res;
    if needed > available {
        let old_ptr = solver.arena.words.as_ptr();
        loop {
            if capacity == MAX_ARENA {
                let bytes_str = crate::format::format_bytes(
                    &mut solver.format,
                    MAX_ARENA * BYTES_PER_WARD as u64,
                );
                crate::error::fatal(format_args!(
                    "maximum arena capacity of 2^{} {}-byte-words {} exhausted",
                    LD_MAX_ARENA, BYTES_PER_WARD, bytes_str
                ));
            }
            solver.arena.enlarge_stack();
            capacity = solver.arena.capacity_wards();
            available = capacity - res;
            if needed <= available {
                break;
            }
        }
        // INC (arena_resized); INC (arena_enlarged): METRIC, compiled out.
        let moved = solver.arena.words.as_ptr() != old_ptr;
        report_resized(solver, "enlarged", moved);
        debug_assert!(capacity <= MAX_ARENA);
    }
    // solver->arena.end += needed;
    let new_len = solver.arena.words.len() + needed as usize * WORDS_PER_WARD;
    solver.arena.words.resize(new_len, 0);
    res as Reference
}

/// kissat_shrink_arena.
pub fn shrink_arena(solver: &mut Solver) {
    let capacity = solver.arena.capacity_wards();
    let size = solver.arena.size_wards();
    // #ifndef QUIET phase reports (GET (arena_resized) -> UINT64_MAX):
    let capacity_bytes = capacity * BYTES_PER_WARD as u64;
    let capacity_str = crate::format::format_count(&mut solver.format, capacity);
    let capacity_bytes_str = crate::format::format_bytes(&mut solver.format, capacity_bytes);
    crate::print::phase(
        solver,
        "arena",
        u64::MAX,
        format_args!(
            "capacity of {} {}-byte-words {}",
            capacity_str, BYTES_PER_WARD, capacity_bytes_str
        ),
    );
    let size_bytes = size * BYTES_PER_WARD as u64;
    let size_str = crate::format::format_count(&mut solver.format, size);
    let size_bytes_str = crate::format::format_bytes(&mut solver.format, size_bytes);
    crate::print::phase(
        solver,
        "arena",
        u64::MAX,
        format_args!(
            "filled {:.0}% with {} {}-byte-words {}",
            percent(size as f64, capacity as f64),
            size_str,
            BYTES_PER_WARD,
            size_bytes_str
        ),
    );
    if size > capacity / 4 {
        crate::print::phase(
            solver,
            "arena",
            u64::MAX,
            format_args!("not shrinking since more than 25% filled"),
        );
        return;
    }
    // INC (arena_resized); INC (arena_shrunken): METRIC, compiled out.
    let old_ptr = solver.arena.words.as_ptr();
    solver.arena.shrink_stack();
    let moved = solver.arena.words.as_ptr() != old_ptr;
    report_resized(solver, "shrunken", moved);
}
