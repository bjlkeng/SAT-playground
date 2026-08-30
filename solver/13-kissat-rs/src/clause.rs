// Port of src/clause.h + src/clause.c (kissat 4.0.4).
// (reference.h constants live in reference.rs and are re-exported here.)
//
// Clause memory layout (see also arena.rs): a clause lives in the arena as
// consecutive u32 words:
//
//   word 0            packed header (bit layout below)
//   word 1            searched
//   word 2            size
//   word 3..3+size    literals
//   ..padding..       up to a multiple of 4 words (= one 16-byte ward)
//
// Header word bit layout (mirrors the C bitfield under GCC/x86-64
// little-endian rules: bits allocated LSB-first in declaration order):
//
//   bits  0..=18  glue      (unsigned glue : LD_MAX_GLUE /* 19 */)
//   bit   19      garbage
//   bit   20      quotient
//   bit   21      reason
//   bit   22      redundant
//   bit   23      shrunken
//   bit   24      subsume
//   bit   25      swept
//   bit   26      vivify
//   bits 27..=31  used      (unsigned used : LD_MAX_USED /* 5 */)
//
// C sizeof (struct clause) == 24 and SIZE_OF_CLAUSE_HEADER (offsetof
// searched) == 4 bytes == 1 word.
//
// PORT NOTES:
//  - Arena clauses are accessed through the lightweight views ClauseRef<'_>
//    / ClauseMut<'_> returned by arena.clause(ref) / arena.clause_mut(ref).
//    The plain owned `Clause` struct below mirrors the C struct with its
//    inline lits[3] and exists for the embedded fake binary-conflict header
//    (solver.conflict, cf. internal.rs and kissat_binary_conflict).
//  - C functions taking `clause *` take a `Reference` here (the C pointer is
//    always derived from / convertible to a reference via the arena base).
//  - CHECK_AND_ADD_* / REMOVE_CHECKER_* are compiled out (NDEBUG build).
//  - ADD/SUB (arena_garbage) are METRIC counters: compiled out in the
//    reference build and omitted (comments mark the spots).
//    INC (clauses_deleted) is STATISTIC-tier: also compiled out in C, but
//    kept as a real counter per statistics.rs policy (never printed, never
//    read — cannot diverge).
//  - `solver.proof` is Option<Box<Proof>>, mirroring the C pointer in the
//    `if (solver->proof)` guards of the proof.h macros.
//  - The singular kissat_mark_added_literal / kissat_mark_removed_literal
//    helpers are inline.h territory: called as crate::inline::*.

use crate::internal::Solver;
use crate::literal::INVALID_LIT;

pub use crate::reference::{References, Reference, INVALID_REF, LD_MAX_REF, MAX_REF};

/*------------------------------------------------------------------------*/
// clause.h constants

pub const LD_MAX_GLUE: u32 = 19;
pub const LD_MAX_USED: u32 = 5;

pub const MAX_GLUE: u32 = (1u32 << LD_MAX_GLUE) - 1;
pub const MAX_USED: u32 = (1u32 << LD_MAX_USED) - 1;

// Header word bit masks (layout documented above).
pub const GLUE_MASK: u32 = MAX_GLUE; // bits 0..=18
pub const GARBAGE_BIT: u32 = 1 << 19;
pub const QUOTIENT_BIT: u32 = 1 << 20;
pub const REASON_BIT: u32 = 1 << 21;
pub const REDUNDANT_BIT: u32 = 1 << 22;
pub const SHRUNKEN_BIT: u32 = 1 << 23;
pub const SUBSUME_BIT: u32 = 1 << 24;
pub const SWEPT_BIT: u32 = 1 << 25;
pub const VIVIFY_BIT: u32 = 1 << 26;
pub const USED_SHIFT: u32 = 27;
pub const USED_MASK: u32 = MAX_USED << USED_SHIFT; // bits 27..=31

/// Word offsets within a clause.
pub const HEADER_OFFSET: usize = 0;
pub const SEARCHED_OFFSET: usize = 1;
pub const SIZE_OFFSET: usize = 2;
pub const LITS_OFFSET: usize = 3;

/// C SIZE_OF_CLAUSE_HEADER == offsetof (clause, searched) == 4 bytes.
pub const SIZE_OF_CLAUSE_HEADER: usize = 4;

/// kissat_bytes_of_clause: sizeof (clause) + (size - 3) * 4 == 12 + 4*size,
/// aligned up to one ward (16 bytes).
#[inline]
pub fn bytes_of_clause(size: u32) -> usize {
    debug_assert!(size >= 3);
    crate::arena::align_ward(12 + 4 * size as usize)
}

/// bytes_of_clause in u32 words (always a multiple of 4).
#[inline]
pub fn words_of_clause(size: u32) -> usize {
    bytes_of_clause(size) / 4
}

/*------------------------------------------------------------------------*/
// Owned mirror of C `struct clause` (used for solver.conflict, the fake
// binary-conflict header set up by kissat_binary_conflict; its lits[3] is
// inline exactly as in C).

#[derive(Clone, Copy, Default)]
pub struct Clause {
    /// The packed glue/flags/used word (bit layout above).
    pub header: u32,
    pub searched: u32,
    pub size: u32,
    pub lits: [u32; 3],
}

/*------------------------------------------------------------------------*/
// Clause views over arena words.  The backing slice runs from the clause
// start to the END of the arena, because a shrunken clause's INVALID_LIT
// terminator lives beyond `lits[size]` (kissat_actual_bytes_of_clause).

#[derive(Clone, Copy)]
pub struct ClauseRef<'a> {
    pub(crate) words: &'a [u32],
}

macro_rules! clause_getters {
    () => {
        #[inline]
        pub fn header(&self) -> u32 {
            self.words[HEADER_OFFSET]
        }
        #[inline]
        pub fn glue(&self) -> u32 {
            self.header() & GLUE_MASK
        }
        #[inline]
        pub fn garbage(&self) -> bool {
            self.header() & GARBAGE_BIT != 0
        }
        #[inline]
        pub fn quotient(&self) -> bool {
            self.header() & QUOTIENT_BIT != 0
        }
        #[inline]
        pub fn reason(&self) -> bool {
            self.header() & REASON_BIT != 0
        }
        #[inline]
        pub fn redundant(&self) -> bool {
            self.header() & REDUNDANT_BIT != 0
        }
        #[inline]
        pub fn shrunken(&self) -> bool {
            self.header() & SHRUNKEN_BIT != 0
        }
        #[inline]
        pub fn subsume(&self) -> bool {
            self.header() & SUBSUME_BIT != 0
        }
        #[inline]
        pub fn swept(&self) -> bool {
            self.header() & SWEPT_BIT != 0
        }
        #[inline]
        pub fn vivify(&self) -> bool {
            self.header() & VIVIFY_BIT != 0
        }
        #[inline]
        pub fn used(&self) -> u32 {
            (self.header() & USED_MASK) >> USED_SHIFT
        }
        #[inline]
        pub fn searched(&self) -> u32 {
            self.words[SEARCHED_OFFSET]
        }
        #[inline]
        pub fn size(&self) -> u32 {
            self.words[SIZE_OFFSET]
        }
        #[inline]
        pub fn lit(&self, i: u32) -> u32 {
            debug_assert!(i < self.size());
            self.words[LITS_OFFSET + i as usize]
        }
        /// kissat_actual_bytes_of_clause in u32 words (multiple of 4).
        /// For shrunken clauses this scans past END_LITS up to and including
        /// the INVALID_LIT terminator (post-increment, exactly as in C).
        #[inline]
        pub fn actual_words(&self) -> usize {
            let mut p = LITS_OFFSET + self.size() as usize;
            if self.shrunken() {
                loop {
                    let w = self.words[p];
                    p += 1;
                    if w == INVALID_LIT {
                        break;
                    }
                }
            }
            (p + 3) & !3usize
        }
        /// kissat_actual_bytes_of_clause.
        #[inline]
        pub fn actual_bytes(&self) -> usize {
            self.actual_words() * 4
        }
    };
}

impl<'a> ClauseRef<'a> {
    clause_getters!();

    /// BEGIN_LITS/END_LITS as a slice.
    #[inline]
    pub fn lits(&self) -> &'a [u32] {
        let size = self.words[SIZE_OFFSET] as usize;
        &self.words[LITS_OFFSET..LITS_OFFSET + size]
    }
}

pub struct ClauseMut<'a> {
    pub(crate) words: &'a mut [u32],
}

impl<'a> ClauseMut<'a> {
    clause_getters!();

    #[inline]
    pub fn as_ref(&self) -> ClauseRef<'_> {
        ClauseRef { words: self.words }
    }
    #[inline]
    pub fn lits(&self) -> &[u32] {
        let size = self.words[SIZE_OFFSET] as usize;
        &self.words[LITS_OFFSET..LITS_OFFSET + size]
    }
    #[inline]
    pub fn lits_mut(&mut self) -> &mut [u32] {
        let size = self.words[SIZE_OFFSET] as usize;
        &mut self.words[LITS_OFFSET..LITS_OFFSET + size]
    }
    #[inline]
    pub fn set_header(&mut self, header: u32) {
        self.words[HEADER_OFFSET] = header;
    }
    #[inline]
    pub fn set_glue(&mut self, glue: u32) {
        debug_assert!(glue <= MAX_GLUE);
        self.words[HEADER_OFFSET] = (self.words[HEADER_OFFSET] & !GLUE_MASK) | (glue & GLUE_MASK);
    }
    #[inline]
    fn set_bit(&mut self, bit: u32, value: bool) {
        if value {
            self.words[HEADER_OFFSET] |= bit;
        } else {
            self.words[HEADER_OFFSET] &= !bit;
        }
    }
    #[inline]
    pub fn set_garbage(&mut self, v: bool) {
        self.set_bit(GARBAGE_BIT, v);
    }
    #[inline]
    pub fn set_quotient(&mut self, v: bool) {
        self.set_bit(QUOTIENT_BIT, v);
    }
    #[inline]
    pub fn set_reason(&mut self, v: bool) {
        self.set_bit(REASON_BIT, v);
    }
    #[inline]
    pub fn set_redundant(&mut self, v: bool) {
        self.set_bit(REDUNDANT_BIT, v);
    }
    #[inline]
    pub fn set_shrunken(&mut self, v: bool) {
        self.set_bit(SHRUNKEN_BIT, v);
    }
    #[inline]
    pub fn set_subsume(&mut self, v: bool) {
        self.set_bit(SUBSUME_BIT, v);
    }
    #[inline]
    pub fn set_swept(&mut self, v: bool) {
        self.set_bit(SWEPT_BIT, v);
    }
    #[inline]
    pub fn set_vivify(&mut self, v: bool) {
        self.set_bit(VIVIFY_BIT, v);
    }
    #[inline]
    pub fn set_used(&mut self, used: u32) {
        debug_assert!(used <= MAX_USED);
        self.words[HEADER_OFFSET] =
            (self.words[HEADER_OFFSET] & !USED_MASK) | (used << USED_SHIFT);
    }
    #[inline]
    pub fn set_searched(&mut self, searched: u32) {
        self.words[SEARCHED_OFFSET] = searched;
    }
    #[inline]
    pub fn set_size(&mut self, size: u32) {
        self.words[SIZE_OFFSET] = size;
    }
    #[inline]
    pub fn set_lit(&mut self, i: u32, lit: u32) {
        debug_assert!(i < self.size());
        self.words[LITS_OFFSET + i as usize] = lit;
    }
}

/*------------------------------------------------------------------------*/
// clause.c

fn inc_clause(solver: &mut Solver, original: bool, redundant: bool, binary: bool) {
    if binary {
        solver.statistics.clauses_binary += 1;
    } else if redundant {
        solver.statistics.clauses_redundant += 1;
    } else {
        solver.statistics.clauses_irredundant += 1;
    }
    solver.statistics.clauses_added += 1;
    if original {
        solver.statistics.clauses_original += 1;
    }
}

fn dec_clause(solver: &mut Solver, redundant: bool, binary: bool) {
    if binary {
        debug_assert!(solver.statistics.clauses_binary > 0);
        solver.statistics.clauses_binary -= 1;
    } else if redundant {
        debug_assert!(solver.statistics.clauses_redundant > 0);
        solver.statistics.clauses_redundant -= 1;
    } else {
        debug_assert!(solver.statistics.clauses_irredundant > 0);
        solver.statistics.clauses_irredundant -= 1;
    }
}

fn init_clause(res: &mut ClauseMut, redundant: bool, glue: u32, size: u32) {
    debug_assert!(redundant || glue == 0);
    let glue = MAX_GLUE.min(glue); // glue = MIN (MAX_GLUE, glue)
    // glue set, all flag bits false, used = 0:
    let mut header = glue & GLUE_MASK;
    if redundant {
        header |= REDUNDANT_BIT;
    }
    res.set_header(header);
    res.set_searched(2);
    res.set_size(size);
}

pub fn connect_referenced(solver: &mut Solver, ref_: Reference) {
    crate::watch::inlined_connect_clause(solver, ref_);
}

/// PORT NOTE: C kissat_connect_clause takes `clause *` and derives the
/// reference (kissat_reference_clause); here the reference is passed
/// directly, making it identical to connect_referenced.
pub fn connect_clause(solver: &mut Solver, ref_: Reference) {
    crate::watch::inlined_connect_clause(solver, ref_);
}

// C static `new_binary_clause` (renamed: the public wrapper below takes the
// same name once the kissat_ prefix is dropped).
fn new_binary(
    solver: &mut Solver,
    original: bool,
    watch: bool,
    first: u32,
    second: u32,
) -> Reference {
    debug_assert!(first != second);
    debug_assert!(first != crate::literal::not(second));
    if watch {
        crate::watch::watch_binary(solver, first, second);
    }
    crate::inline::mark_added_literal(solver, first);
    crate::inline::mark_added_literal(solver, second);
    inc_clause(solver, original, false, true);
    if !original {
        // CHECK_AND_ADD_BINARY: compiled out (NDEBUG).
        if solver.proof.is_some() {
            crate::proof::add_binary_to_proof(solver, first, second);
        }
    }
    INVALID_REF
}

fn new_large_clause(
    solver: &mut Solver,
    original: bool,
    redundant: bool,
    glue: u32,
    lits: &[u32],
) -> Reference {
    let size = lits.len() as u32;
    debug_assert!(size > 2);
    let res = crate::arena::allocate_clause(solver, size as usize);
    {
        let mut c = solver.arena.clause_mut(res);
        init_clause(&mut c, redundant, glue, size);
        c.lits_mut().copy_from_slice(lits); // memcpy (c->lits, lits, ...)
    }
    if solver.watching {
        crate::watch::watch_reference(solver, lits[0], lits[1], res);
    } else {
        connect_clause(solver, res);
    }
    if redundant {
        if solver.first_reducible == INVALID_REF {
            solver.first_reducible = res;
        }
    } else {
        crate::flags::mark_added_literals(solver, size, lits);
        solver.last_irredundant = res;
    }
    inc_clause(solver, original, redundant, false);
    if !original {
        // CHECK_AND_ADD_CLAUSE: compiled out (NDEBUG).
        if solver.proof.is_some() {
            crate::proof::add_clause_to_proof(solver, res);
        }
    }
    res
}

fn new_clause(
    solver: &mut Solver,
    original: bool,
    redundant: bool,
    glue: u32,
    lits: &[u32],
) -> Reference {
    let res = if lits.len() == 2 {
        new_binary(solver, original, true, lits[0], lits[1])
    } else {
        new_large_clause(solver, original, redundant, glue, lits)
    };
    crate::collect::defrag_watches_if_needed(solver);
    res
}

pub fn new_binary_clause(solver: &mut Solver, first: u32, second: u32) {
    let _ = new_binary(solver, false, true, first, second);
}

pub fn new_unwatched_binary_clause(solver: &mut Solver, first: u32, second: u32) {
    let _ = new_binary(solver, false, false, first, second);
}

// PORT NOTE (all three constructors): C passes BEGIN_STACK (solver->clause)
// into new_clause while `solver` stays fully accessible.  The stack is taken
// out and restored around the call so the literals can be passed as a slice
// alongside `&mut Solver`; nothing below reads or writes solver->clause, so
// this is unobservable.  new_original_clause sorts solver->clause in place
// first (the restored stack is the sorted one, as in C).

pub fn new_original_clause(solver: &mut Solver) -> Reference {
    let mut lits = std::mem::take(&mut solver.clause);
    let size = lits.len() as u32;
    crate::sort::sort_literals(solver, size, &mut lits);
    let res = new_clause(solver, true, false, 0, &lits);
    solver.clause = lits;
    res
}

pub fn new_irredundant_clause(solver: &mut Solver) -> Reference {
    let lits = std::mem::take(&mut solver.clause);
    let res = new_clause(solver, false, false, 0, &lits);
    solver.clause = lits;
    res
}

pub fn new_redundant_clause(solver: &mut Solver, glue: u32) -> Reference {
    let lits = std::mem::take(&mut solver.clause);
    let res = new_clause(solver, false, true, glue, &lits);
    solver.clause = lits;
    res
}

/// kissat_mark_clause_as_garbage.  The C static helper of the same name is
/// folded in: its only extra effect in the public wrapper was
/// ADD (arena_garbage, bytes), a METRIC counter compiled out in the
/// reference build.
pub fn mark_clause_as_garbage(solver: &mut Solver, ref_: Reference) {
    let (redundant, size) = {
        let c = solver.arena.clause(ref_);
        debug_assert!(!c.garbage());
        debug_assert!(c.size() > 2);
        (c.redundant(), c.size())
    };
    if !redundant {
        // kissat_mark_removed_literals (flags.c) is exactly this loop over
        // kissat_mark_removed_literal; inlined because a lits slice would
        // borrow the arena across the &mut Solver call.
        for i in 0..size {
            let lit = solver.arena.clause(ref_).lit(i);
            crate::inline::mark_removed_literal(solver, lit);
        }
    }
    // REMOVE_CHECKER_CLAUSE: compiled out (NDEBUG).
    if solver.proof.is_some() {
        crate::proof::delete_clause_from_proof(solver, ref_);
    }
    dec_clause(solver, redundant, false);
    solver.arena.clause_mut(ref_).set_garbage(true);
    // ADD (arena_garbage, kissat_actual_bytes_of_clause (c)): METRIC,
    // compiled out in the reference build.
}

/// kissat_delete_clause.  C returns the `clause *` following this one; the
/// port returns the following Reference (same offset math).
pub fn delete_clause(solver: &mut Solver, ref_: Reference) -> Reference {
    let words = {
        let c = solver.arena.clause(ref_);
        debug_assert!(c.size() > 2);
        debug_assert!(c.garbage());
        c.actual_words()
    };
    // SUB (arena_garbage, bytes): METRIC, compiled out.
    solver.statistics.clauses_deleted += 1; // INC (clauses_deleted): STATISTIC
    ref_ + (words / crate::arena::WORDS_PER_WARD) as u32
}

pub fn delete_binary(solver: &mut Solver, a: u32, b: u32) {
    crate::inline::mark_removed_literal(solver, a);
    crate::inline::mark_removed_literal(solver, b);
    // REMOVE_CHECKER_BINARY: compiled out (NDEBUG).
    if solver.proof.is_some() {
        crate::proof::delete_binary_from_proof(solver, a, b);
    }
    dec_clause(solver, false, true);
    solver.statistics.clauses_deleted += 1; // INC (clauses_deleted): STATISTIC
}
