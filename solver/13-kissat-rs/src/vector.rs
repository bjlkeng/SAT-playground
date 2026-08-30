// Port of src/vector.h + src/vector.c + src/inlinevector.h (kissat 4.0.4).
//
// Build configuration: 64-bit, COMPACT **not** defined.  The C
// `struct vector` holds raw begin/end pointers into the shared vectors stack
// (solver->vectors.stack); this port stores *word offsets* instead.
// Consequences (all semantics-preserving):
//
//  - fix_vector_pointers_after_moving_stack becomes a no-op (offsets are
//    stable across reallocation); the surrounding INC/phase effects remain.
//  - The C "null vector" test `!vector->begin` maps to `begin == 0`:
//    stack word 0 is a reserved sentinel (kissat_push_vectors pushes a 0
//    entry on first use, and defrag compacts starting at offset 1), so no
//    live vector ever begins at offset 0.
//
// The C stack growth policy (ENLARGE_STACK doubling from an initial capacity
// of BYTES_PER_ELEMENT == sizeof (unsigned) == 4 entries; SHRINK_STACK to
// the power-of-two ceiling) is replicated via an explicit `capacity` mirror,
// because FULL_STACK decides whether kissat_push_vectors appends in place or
// relocates+doubles the vector, which changes hole layout and `usable`
// accounting and hence defrag timing — all trajectory-relevant.
//
// PORT NOTES:
//  - Vectors are identified by the literal owning the watch list
//    (solver.watches[lit]); watches are the only users of `vector` in kissat.
//  - INC (vectors_enlarged) / INC (defragmentations) are METRIC counters,
//    compiled out in the reference build; GET (...) in the phase calls yields
//    UINT64_MAX ("no count" in kissat_phase) — hardcoded u64::MAX.
//  - RADIX_SORT rank in C is the absolute begin *address* (uintptr_t);
//    ranking by the begin offset yields the identical order (single shared
//    base).  Pass structure may differ in skipped high bytes only; the
//    resulting permutation is identical.
//  - START/STOP (defrag) and (radix) map to crate::profile::
//    {start,stop}_checked, which carry the C macros' GET_OPTION (profile)
//    level guard.
//  - CHECK_VECTORS / kissat_check_vectors: not defined in the reference
//    build; no-ops, omitted.

use crate::internal::Solver;

pub const LD_MAX_VECTORS: u32 = 48; // sizeof (word) == 8, non-COMPACT
pub const MAX_VECTORS: u64 = 1u64 << LD_MAX_VECTORS;

pub const INVALID_VECTOR_ELEMENT: u32 = u32::MAX;

// kissat_log2_ceiling_of_word (utilities.h).
#[inline]
fn log2_ceiling_u64(x: u64) -> u32 {
    if x == 0 {
        return 0;
    }
    let floor = 63 - x.leading_zeros();
    floor + if x != (1u64 << floor) { 1 } else { 0 }
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

/// C `struct vector` (non-COMPACT: begin/end pointers) as word offsets into
/// Vectors::stack.  `begin == 0` encodes the C NULL begin ("null vector").
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Vector {
    pub begin: usize,
    pub end: usize,
}

impl Vector {
    /// kissat_size_vector.
    #[inline]
    pub fn size(&self) -> usize {
        self.end - self.begin
    }
    /// kissat_empty_vector.
    #[inline]
    pub fn empty(&self) -> bool {
        self.end == self.begin
    }
    /// C `!vector->begin` (null pointer test).
    #[inline]
    pub fn is_null(&self) -> bool {
        self.begin == 0
    }
    /// kissat_last_vector_pointer, as a word offset.
    #[inline]
    pub fn last_offset(&self) -> usize {
        debug_assert!(!self.empty());
        self.end - 1
    }
}

/// C `struct vectors`: the shared u32 stack backing every watch list plus the
/// count of usable (INVALID-marked) holes.
pub struct Vectors {
    pub stack: Vec<u32>,
    /// CAPACITY_STACK mirror (entries); see module PORT NOTES.
    capacity: usize,
    pub usable: u64, // C: size_t usable
}

impl Default for Vectors {
    fn default() -> Self {
        Self::new()
    }
}

impl Vectors {
    pub fn new() -> Self {
        Vectors {
            stack: Vec::new(),
            capacity: 0,
            usable: 0,
        }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[inline]
    fn full(&self) -> bool {
        self.stack.len() == self.capacity // FULL_STACK
    }

    /// One ENLARGE_STACK step (doubling; initial capacity 4 entries).
    fn enlarge_stack(&mut self) {
        let new_capacity = if self.capacity != 0 {
            2 * self.capacity
        } else {
            4 // ENLARGE_STACK quirk: BYTES_PER_ELEMENT == sizeof (unsigned)
        };
        self.stack.reserve_exact(new_capacity - self.stack.len());
        self.capacity = new_capacity;
    }

    /// PUSH_STACK on the shared stack (sentinel push only).
    fn push_stack(&mut self, e: u32) {
        if self.full() {
            self.enlarge_stack();
        }
        self.stack.push(e);
    }

    /// SHRINK_STACK (stack.h) for the unsigned stack.
    fn shrink_stack(&mut self) {
        if self.full() {
            return;
        }
        let old_bytes = self.capacity * 4;
        let old_size = self.stack.len();
        if old_size == 0 {
            self.stack = Vec::new(); // kissat_free + INIT_STACK
            self.capacity = 0;
            return;
        }
        if old_bytes <= 8 {
            return; // OLD_BYTES <= sizeof (void *)
        }
        let new_capacity = 1usize << log2_ceiling_u64(old_size as u64);
        let new_bytes = new_capacity * 4;
        if new_bytes == old_bytes {
            return;
        }
        debug_assert!(new_bytes < old_bytes);
        self.stack.shrink_to(new_capacity);
        self.capacity = new_capacity;
    }

    /// kissat_inc_usable.
    #[inline]
    pub fn inc_usable(&mut self) {
        self.usable += 1;
    }
    /// kissat_add_usable.
    #[inline]
    pub fn add_usable(&mut self, inc: u64) {
        self.usable += inc;
    }
    /// kissat_dec_usable.
    #[inline]
    pub fn dec_usable(&mut self) {
        debug_assert!(self.usable > 0);
        self.usable -= 1;
    }
}

/// kissat_enlarge_vector: doubles solver.watches[lit] by relocating it to the
/// end of the shared stack; returns the word offset of the first free slot
/// (C: pointer just past the copied old contents), which holds
/// INVALID_VECTOR_ELEMENT.
pub fn enlarge_vector(solver: &mut Solver, lit: u32) -> usize {
    let old_vector_size = solver.watches[lit as usize].size();
    debug_assert!((old_vector_size as u64) < MAX_VECTORS / 2);
    let new_vector_size = if old_vector_size != 0 {
        2 * old_vector_size
    } else {
        1
    };
    let old_stack_size = solver.vectors.stack.len();
    let mut capacity = solver.vectors.capacity;
    debug_assert!((capacity as u64) <= MAX_VECTORS);
    let mut available = capacity - old_stack_size;
    if new_vector_size > available {
        let old_ptr = solver.vectors.stack.as_ptr();
        let mut enlarged = 0u32;
        loop {
            if capacity as u64 == MAX_VECTORS {
                let bytes_str =
                    crate::format::format_bytes(&mut solver.format, MAX_VECTORS * 4);
                crate::error::fatal(format_args!(
                    "maximum vector stack size of 2^{} entries {} exhausted",
                    LD_MAX_VECTORS, bytes_str
                ));
            }
            enlarged += 1;
            solver.vectors.enlarge_stack();
            capacity = solver.vectors.capacity;
            available = capacity - old_stack_size;
            if new_vector_size <= available {
                break;
            }
        }
        if enlarged != 0 {
            // INC (vectors_enlarged): METRIC, compiled out.
            let moved = solver.vectors.stack.as_ptr() != old_ptr;
            let count_str = crate::format::format_count(&mut solver.format, capacity as u64);
            let bytes_str = crate::format::format_bytes(&mut solver.format, capacity as u64 * 4);
            // GET (vectors_enlarged): IGNOREd METRIC -> UINT64_MAX.
            crate::print::phase(
                solver,
                "vectors",
                u64::MAX,
                format_args!(
                    "enlarged to {} entries {} ({})",
                    count_str,
                    bytes_str,
                    if moved { "moved" } else { "in place" }
                ),
            );
            // fix_vector_pointers_after_moving_stack: no-op (offsets).
        }
        debug_assert!((capacity as u64) <= MAX_VECTORS);
        debug_assert!(new_vector_size <= available);
    }
    let begin_old_vector = solver.watches[lit as usize].begin;
    let begin_new_vector = solver.vectors.stack.len();
    let middle_new_vector = begin_new_vector + old_vector_size;
    let end_new_vector = begin_new_vector + new_vector_size;
    if old_vector_size != 0 {
        let stack = &mut solver.vectors.stack;
        // memcpy old contents to the fresh region ...
        stack.extend_from_within(begin_old_vector..begin_old_vector + old_vector_size);
        // ... then memset the old region to 0xff.
        for w in &mut stack[begin_old_vector..begin_old_vector + old_vector_size] {
            *w = INVALID_VECTOR_ELEMENT;
        }
    }
    solver.vectors.usable += old_vector_size as u64; // holes left behind
    solver
        .vectors
        .add_usable((new_vector_size - old_vector_size) as u64);
    // memset (middle..end, 0xff) + stack->end = end_new_vector:
    solver
        .vectors
        .stack
        .resize(end_new_vector, INVALID_VECTOR_ELEMENT);
    solver.watches[lit as usize] = Vector {
        begin: begin_new_vector,
        end: middle_new_vector,
    };
    debug_assert!(solver.watches[lit as usize].size() == old_vector_size);
    middle_new_vector
}

/// kissat_push_vectors (inlinevector.h): append `e` to solver.watches[lit].
pub fn push_vectors(solver: &mut Solver, lit: u32, e: u32) {
    debug_assert!(e != INVALID_VECTOR_ELEMENT);
    if solver.watches[lit as usize].is_null() {
        if solver.vectors.stack.is_empty() {
            solver.vectors.push_stack(0); // reserved sentinel word 0
        }
        if solver.vectors.full() {
            let end = enlarge_vector(solver, lit);
            debug_assert!(solver.vectors.stack[end] == INVALID_VECTOR_ELEMENT);
            solver.vectors.stack[end] = e;
            solver.vectors.dec_usable();
        } else {
            // *(vector->begin = stack->end++) = e;
            let begin = solver.vectors.stack.len();
            solver.watches[lit as usize].begin = begin;
            solver.vectors.stack.push(e);
        }
        // vector->end = vector->begin;
        let begin = solver.watches[lit as usize].begin;
        solver.watches[lit as usize].end = begin;
    } else {
        let end = solver.watches[lit as usize].end;
        if end == solver.vectors.stack.len() {
            if solver.vectors.full() {
                let end = enlarge_vector(solver, lit);
                debug_assert!(solver.vectors.stack[end] == INVALID_VECTOR_ELEMENT);
                solver.vectors.stack[end] = e;
                solver.vectors.dec_usable();
            } else {
                solver.vectors.stack.push(e); // *stack->end++ = e
            }
        } else {
            let end = if solver.vectors.stack[end] != INVALID_VECTOR_ELEMENT {
                enlarge_vector(solver, lit)
            } else {
                end
            };
            debug_assert!(solver.vectors.stack[end] == INVALID_VECTOR_ELEMENT);
            solver.vectors.stack[end] = e;
            solver.vectors.dec_usable();
        }
    }
    solver.watches[lit as usize].end += 1;
}

/// kissat_remove_from_vector: remove the first occurrence of `remove`,
/// shifting the tail left (relative order preserved) and poisoning the freed
/// last slot.
pub fn remove_from_vector(solver: &mut Solver, lit: u32, remove: u32) {
    let v = solver.watches[lit as usize];
    let begin = v.begin;
    let end = v.end;
    debug_assert!(begin != end);
    {
        let stack = &mut solver.vectors.stack;
        let mut p = begin;
        while stack[p] != remove {
            p += 1;
            debug_assert!(p != end);
        }
        p += 1;
        while p != end {
            stack[p - 1] = stack[p];
            p += 1;
        }
        stack[p - 1] = INVALID_VECTOR_ELEMENT;
    }
    debug_assert!(v.begin < v.end);
    solver.watches[lit as usize].end = end - 1;
    solver.vectors.inc_usable();
}

/// kissat_resize_vector (shrink only).
pub fn resize_vector(solver: &mut Solver, lit: u32, new_size: usize) {
    let v = solver.watches[lit as usize];
    let old_size = v.size();
    debug_assert!(new_size <= old_size);
    if new_size == old_size {
        return;
    }
    solver.watches[lit as usize].end = v.begin + new_size;
    let delta = old_size - new_size;
    solver.vectors.add_usable(delta as u64);
    // memset (end, 0xff, delta * sizeof (unsigned)):
    let stack = &mut solver.vectors.stack;
    for w in &mut stack[v.begin + new_size..v.begin + old_size] {
        *w = INVALID_VECTOR_ELEMENT;
    }
}

/// kissat_release_vector (inlinevector.h).
pub fn release_vector(solver: &mut Solver, lit: u32) {
    resize_vector(solver, lit, 0);
}

/// kissat_defrag_vectors.  C signature is (solver, size_unsorted, unsorted);
/// the only caller passes (LITS, solver->watches), which is what this
/// operates on directly.
pub fn defrag_vectors(solver: &mut Solver) {
    let size_vectors = solver.vectors.stack.len();
    if size_vectors < 2 {
        return;
    }
    // START (defrag).
    crate::profile::start_checked(solver, crate::profile::Prof::defrag);
    // INC (defragmentations): METRIC, compiled out.
    let size_unsorted = solver.watches.len(); // C: LITS
    let mut sorted: Vec<u32> = Vec::with_capacity(size_unsorted); // kissat_malloc
    for i in 0..size_unsorted {
        let v = &mut solver.watches[i];
        if v.empty() {
            // C (non-COMPACT): vector->begin = vector->end = 0;
            v.begin = 0;
            v.end = 0;
        } else {
            sorted.push(i as u32);
        }
    }
    // RADIX_SORT (unsigned, uintptr_t, size_sorted, sorted, RANK_OFFSET);
    // rank = begin offset (see module PORT NOTES).  START/STOP (radix) lives
    // inside the C macro; hoisted here (level guard in start/stop_checked).
    crate::profile::start_checked(solver, crate::profile::Prof::radix);
    {
        let watches = &solver.watches;
        crate::sort::radix_sort::<u32, u64, _>(&mut sorted, |&i| watches[i as usize].begin as u64);
    }
    crate::profile::stop_checked(solver, crate::profile::Prof::radix);
    let mut p: usize = 1; // old_begin_stack + 1 (skip the sentinel word)
    {
        let watches = &mut solver.watches;
        let stack = &mut solver.vectors.stack;
        for k in 0..sorted.len() {
            let j = sorted[k] as usize;
            let size = watches[j].size();
            if size == 0 {
                // Dead in practice (sorted holds only non-empty vectors) but
                // present in the C non-COMPACT branch; ported as-is.
                watches[j].begin = 0;
                watches[j].end = 0;
                continue;
            }
            let q = watches[j].begin;
            let new_end_of_vector = p + size;
            watches[j].begin = p;
            watches[j].end = new_end_of_vector;
            stack.copy_within(q..q + size, p); // memmove
            p = new_end_of_vector;
        }
    }
    drop(sorted); // kissat_free
    let freed = solver.vectors.stack.len() - p; // END_STACK - p (pre-truncate)
    let freed_fraction = percent(freed as f64, size_vectors as f64);
    let bytes_str = crate::format::format_bytes(&mut solver.format, freed as u64 * 4);
    // GET (defragmentations): IGNOREd METRIC -> UINT64_MAX.
    crate::print::phase(
        solver,
        "defrag",
        u64::MAX,
        format_args!(
            "freed {} usable entries {:.0}% thus {}",
            freed, freed_fraction, bytes_str
        ),
    );
    solver.vectors.stack.truncate(p); // SET_END_OF_STACK
    solver.vectors.shrink_stack(); // SHRINK_STACK
    // fix_vector_pointers_after_moving_stack: no-op (offsets).
    solver.vectors.usable = 0;
    // STOP (defrag).
    crate::profile::stop_checked(solver, crate::profile::Prof::defrag);
}

/// kissat_release_vectors.
pub fn release_vectors(solver: &mut Solver) {
    solver.vectors.stack = Vec::new(); // RELEASE_STACK
    solver.vectors.capacity = 0;
    solver.vectors.usable = 0;
}
