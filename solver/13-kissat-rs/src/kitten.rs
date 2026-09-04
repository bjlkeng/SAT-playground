// Port of src/kitten.c + src/kitten.h (kissat 4.0.4).
//
// Embedded build only: STAND_ALONE_KITTEN is NOT defined (the stand-alone
// main/parser/witness code is omitted), LOGGING/CHECK_KITTEN compiled out.
//
// PORT NOTES:
//  - The C `struct kitten` carries a back pointer `kissat *kissat` used only
//    for statistics (`INC`/`ADD` on solver->statistics.kitten_*), the
//    `TERMINATED (kitten_terminated_1)` check and kissat_fatal.  A Rust
//    Kitten inside `solver.kitten: Option<Box<Kitten>>` cannot hold that
//    borrow, so every function that touches statistics or termination takes
//    `solver: &mut Solver` explicitly; call sites take the kitten out of
//    `solver.kitten` around calls (see sweep.rs `with_kitten!`).
//  - Public API: free functions mirroring kitten.h names verbatim
//    (`kitten_solve`, `kitten_value`, ...), first argument `&mut Kitten` /
//    `&Kitten` replacing the C `kitten *`.  `kitten_init` is stand-alone
//    only; the embedded constructor `kitten_embedded` loses its `kissat *`
//    argument (see back-pointer note above).
//  - `kitten_compute_clausal_core`'s `uint64_t *learned` out-parameter
//    becomes a second tuple element of the return value.
//  - The two core traversal functions take closures instead of C
//    `void *state` + function pointer pairs.
//  - `struct katch` is 8 bytes in C (KITTEN_BLIT defined: blit + ref:31 +
//    binary:1); ported as { blit: u32, meta: u32 } with
//    meta = ref | binary << 31.  The propagation/flip tick formula
//    `((char *) end - (char *) begin) >> 7` (bytes per 128-byte cache line)
//    is therefore computed as `(len * 8) >> 7 == len >> 4`.
//  - The `klauses` arena is a STACK (unsigned); a klause at reference r is
//    the words [aux, size, flags, lits[size], antecedent-refs[aux]] where the
//    antecedent block exists only for learned klauses with antecedent
//    tracking on.  `klause *` parameters become u32 references.
//  - C heap arrays (marks/phases/values/failed/vars/links/watches) are Vecs
//    kept at *capacity* length (kitten->size resp. size/2), zero-filled on
//    growth: RESIZE1/RESIZE2 calloc new capacity and copy only the active
//    prefix, but the dropped tail is provably all-zero in C too, so
//    Vec::resize(zero-fill) is equivalent.
//  - INVALID_API_USAGE / kissat_fatal abort even under NDEBUG → panic!.
//  - Statistics tiers (statistics.h): kitten_propagations, kitten_solved,
//    kitten_ticks are COUNTERs (exact parity oracle); kitten_conflicts,
//    kitten_decisions, kitten_flip, kitten_flipped, kitten_sat, kitten_unsat,
//    kitten_unknown are STATISTIC tier — real (never printed) fields in this
//    build, incremented 1:1 at the C call sites.

use crate::internal::Solver;
use crate::random::{next_random64, pick_random};

pub const INVALID: u32 = u32::MAX;

const CORE_FLAG: u32 = 1;
const LEARNED_FLAG: u32 = 2;

/// Port of `struct kar`.
#[derive(Clone, Copy, Default)]
struct Kar {
    level: u32,
    reason: u32,
}

/// Port of `struct kink`.
#[derive(Clone, Copy, Default)]
struct Kink {
    next: u32,
    prev: u32,
    stamp: u64,
}

/// Port of `struct katch` (KITTEN_BLIT variant, 8 bytes).
#[derive(Clone, Copy)]
pub struct Katch {
    blit: u32,
    meta: u32, // ref : 31 | binary : 1  (bit 31)
}

const KATCH_BINARY_BIT: u32 = 1u32 << 31;

impl Katch {
    #[inline]
    fn new(blit: u32, ref_: u32, binary: bool) -> Self {
        debug_assert!(ref_ < KATCH_BINARY_BIT);
        Katch {
            blit,
            meta: ref_ | if binary { KATCH_BINARY_BIT } else { 0 },
        }
    }
    #[inline]
    fn ref_(&self) -> u32 {
        self.meta & !KATCH_BINARY_BIT
    }
    #[inline]
    fn binary(&self) -> bool {
        self.meta & KATCH_BINARY_BIT != 0
    }
}

/// The anonymous queue struct inside `struct kitten`.
#[derive(Default)]
struct Kueue {
    first: u32,
    last: u32,
    stamp: u64,
    search: u32,
}

/// Port of `struct kimits`.
#[derive(Default)]
struct Kimits {
    ticks: u64,
}

/// Port of `struct kitten` (embedded variant, field order preserved).
#[derive(Default)]
pub struct Kitten {
    // First zero (re)initialized field in 'clear_kitten' is 'status'.
    status: i32,

    antecedents: bool,
    learned: bool,

    level: u32,
    propagated: u32,
    unassigned: u32,
    inconsistent: u32,
    failing: u32,

    generator: u64,

    lits: usize,
    evars: usize,

    end_original_ref: usize,

    queue: Kueue,

    // 'size' is the first field NOT zeroed by 'clear_kitten'.
    size: usize,
    esize: usize,

    vars: crate::uvec::UVec<Kar>,     // capacity size/2
    links: crate::uvec::UVec<Kink>,   // capacity size/2
    marks: crate::uvec::UVec<i8>,     // capacity size/2
    values: crate::uvec::UVec<i8>,    // capacity size (per lit)
    failed: crate::uvec::UVec<bool>,  // capacity size (per lit)
    phases: crate::uvec::UVec<u8>,    // capacity size/2
    import: crate::uvec::UVec<u32>,   // capacity esize (per external var)
    watches: crate::uvec::UVec<Vec<Katch>>, // capacity size (per lit)

    analyzed: Vec<u32>,
    assumptions: Vec<u32>,
    core: Vec<u32>,
    eclause: Vec<u32>,
    export_: Vec<u32>,
    klause: Vec<u32>,
    klauses: Vec<u32>,
    resolved: Vec<u32>,
    trail: Vec<u32>,
    units: Vec<u32>,

    limits: Kimits,
    initialized: u64,
}

/*------------------------------------------------------------------------*/
// klause accessors (klause struct over the klauses arena).

impl Kitten {
    #[inline]
    fn k_aux(&self, r: u32) -> u32 {
        self.klauses[r as usize]
    }
    #[inline]
    fn k_size(&self, r: u32) -> u32 {
        self.klauses[r as usize + 1]
    }
    #[inline]
    fn k_flags(&self, r: u32) -> u32 {
        self.klauses[r as usize + 2]
    }
    #[inline]
    fn k_lit(&self, r: u32, i: u32) -> u32 {
        self.klauses[r as usize + 3 + i as usize]
    }
    #[inline]
    fn k_set_lit(&mut self, r: u32, i: u32, lit: u32) {
        self.klauses[r as usize + 3 + i as usize] = lit;
    }
    #[inline]
    fn is_core_klause(&self, r: u32) -> bool {
        self.k_flags(r) & CORE_FLAG != 0
    }
    #[inline]
    fn is_learned_klause(&self, r: u32) -> bool {
        self.k_flags(r) & LEARNED_FLAG != 0
    }
    #[inline]
    fn set_core_klause(&mut self, r: u32) {
        self.klauses[r as usize + 2] |= CORE_FLAG;
    }
    #[inline]
    fn unset_core_klause(&mut self, r: u32) {
        self.klauses[r as usize + 2] &= !CORE_FLAG;
    }
    /// antecedents (c): refs start right after the lits.
    #[inline]
    fn k_antecedent(&self, r: u32, i: u32) -> u32 {
        let size = self.k_size(r);
        self.klauses[r as usize + 3 + size as usize + i as usize]
    }
    /// next_klause.
    #[inline]
    fn next_klause(&self, r: u32) -> u32 {
        let size = self.k_size(r);
        let mut res = r + 3 + size;
        if self.antecedents && self.is_learned_klause(r) {
            res += self.k_aux(r);
        }
        res
    }
}

/*------------------------------------------------------------------------*/
// queue

fn update_search(kitten: &mut Kitten, idx: u32) {
    if kitten.queue.search == idx {
        return;
    }
    kitten.queue.search = idx;
}

fn enqueue(kitten: &mut Kitten, idx: u32) {
    let last = kitten.queue.last;
    if last == INVALID {
        kitten.queue.first = idx;
    } else {
        kitten.links[last as usize].next = idx;
    }
    let l = &mut kitten.links[idx as usize];
    l.prev = last;
    l.next = INVALID;
    kitten.queue.last = idx;
    kitten.links[idx as usize].stamp = kitten.queue.stamp;
    kitten.queue.stamp += 1;
}

fn dequeue(kitten: &mut Kitten, idx: u32) {
    let l = kitten.links[idx as usize];
    let prev = l.prev;
    let next = l.next;
    if prev == INVALID {
        kitten.queue.first = next;
    } else {
        kitten.links[prev as usize].next = next;
    }
    if next == INVALID {
        kitten.queue.last = prev;
    } else {
        kitten.links[next as usize].prev = prev;
    }
}

fn init_queue(kitten: &mut Kitten, old_vars: usize, new_vars: usize) {
    for idx in old_vars..new_vars {
        debug_assert!(kitten.values[2 * idx] == 0);
        debug_assert!(kitten.unassigned < u32::MAX);
        kitten.unassigned += 1;
        enqueue(kitten, idx as u32);
    }
    let last = kitten.queue.last;
    update_search(kitten, last);
}

fn initialize_kitten(kitten: &mut Kitten) {
    kitten.queue.first = INVALID;
    kitten.queue.last = INVALID;
    kitten.inconsistent = INVALID;
    kitten.failing = INVALID;
    kitten.queue.search = INVALID;
    kitten.limits.ticks = u64::MAX;
    kitten.generator = kitten.initialized;
    kitten.initialized += 1;
}

/// clear_kitten: memset from &status up to (exclusive) &size, then
/// re-initialize.  Ported as explicit zeroing of exactly those fields.
fn clear_kitten(kitten: &mut Kitten) {
    kitten.status = 0;
    kitten.antecedents = false;
    kitten.learned = false;
    kitten.level = 0;
    kitten.propagated = 0;
    kitten.unassigned = 0;
    kitten.inconsistent = 0;
    kitten.failing = 0;
    kitten.generator = 0;
    kitten.lits = 0;
    kitten.evars = 0;
    kitten.end_original_ref = 0;
    kitten.queue = Kueue::default();
    initialize_kitten(kitten);
}

/// enlarge_internal: RESIZE1 (var-indexed) / RESIZE2 (lit-indexed) with
/// power-of-two capacity doubling; see module PORT NOTE on zero-fill.
fn enlarge_internal(kitten: &mut Kitten, new_lits: usize) {
    let old_lits = kitten.lits;
    debug_assert!(old_lits < new_lits);
    let old_size = kitten.size;
    let new_vars = new_lits / 2;
    let old_vars = old_lits / 2;
    if old_size < new_lits {
        let mut new_size = if old_size != 0 { 2 * old_size } else { 2 };
        while new_size <= new_lits {
            new_size *= 2;
        }
        kitten.marks.resize(new_size / 2, 0);
        kitten.phases.resize(new_size / 2, 0);
        kitten.values.resize(new_size, 0);
        kitten.failed.resize(new_size, false);
        kitten.vars.resize(new_size / 2, Kar::default());
        kitten.links.resize(new_size / 2, Kink::default());
        kitten.watches.resize(new_size, Vec::new());
        kitten.size = new_size;
    }
    kitten.lits = new_lits;
    init_queue(kitten, old_vars, new_vars);
}

fn status_to_string(status: i32) -> &'static str {
    match status {
        10 => "formula satisfied",
        20 => "formula inconsistent",
        21 => "formula inconsistent and core computed",
        _ => {
            debug_assert!(status == 0);
            "formula unsolved"
        }
    }
}

/// INVALID_API_USAGE → abort in C (even with NDEBUG) → panic here.
macro_rules! invalid_api_usage {
    ($($arg:tt)*) => {
        panic!("kitten: fatal error: invalid API usage: {}", format!($($arg)*))
    };
}

fn require_status(kitten: &Kitten, expected: i32) {
    if kitten.status != expected {
        invalid_api_usage!(
            "invalid status '{}' (expected '{}')",
            status_to_string(kitten.status),
            status_to_string(expected)
        );
    }
}

/*------------------------------------------------------------------------*/

/// kitten_embedded.  PORT NOTE: the C `kissat *` back pointer is not stored
/// (see module header); callers pass `solver` to the functions that need it.
pub fn kitten_embedded() -> Box<Kitten> {
    let mut kitten = Box::new(Kitten::default());
    initialize_kitten(&mut kitten);
    kitten
}

pub fn kitten_track_antecedents(kitten: &mut Kitten) {
    require_status(kitten, 0);
    if kitten.learned {
        invalid_api_usage!("can not start tracking antecedents after learning");
    }
    kitten.antecedents = true;
}

pub fn kitten_randomize_phases(kitten: &mut Kitten) {
    let phases = &mut kitten.phases;
    let vars = kitten.size / 2;

    let mut random = next_random64(&mut kitten.generator);

    let mut i: usize = 0;
    let rest = vars & !63usize;

    // C writes 8 u64 words (p[0..8]) per 64 phases: byte j of word k is bit
    // (8*j + k) of `random` (little endian).  Reproduced with byte stores.
    while i != rest {
        for k in 0..8usize {
            let word = (random >> k) & 0x0101010101010101u64;
            for j in 0..8usize {
                phases[i + 8 * k + j] = ((word >> (8 * j)) & 0xff) as u8;
            }
        }
        random = next_random64(&mut kitten.generator);
        i += 64;
    }

    let mut shift = 0u32;
    while i != vars {
        phases[i] = ((random >> shift) & 1) as u8;
        shift += 1;
        i += 1;
    }
}

pub fn kitten_flip_phases(kitten: &mut Kitten) {
    // C xors 0x0101..01 over u64 groups of 8 then the remainder byte-wise;
    // identical to flipping bit 0 of every phase byte.
    let vars = kitten.size / 2;
    for i in 0..vars {
        kitten.phases[i] ^= 1;
    }
}

pub fn kitten_no_ticks_limit(kitten: &mut Kitten) {
    kitten.limits.ticks = u64::MAX;
}

/// KITTEN_TICKS is `solver->statistics.kitten_ticks` in the embedded build.
pub fn kitten_set_ticks_limit(kitten: &mut Kitten, solver: &Solver, delta: u64) {
    let current = solver.statistics.kitten_ticks;
    let limit = if u64::MAX - delta <= current {
        u64::MAX
    } else {
        current + delta
    };
    kitten.limits.ticks = limit;
}

fn shuffle_unsigned_array(generator: &mut u64, a: &mut [u32]) {
    let size = a.len();
    for i in 0..size {
        let j = pick_random(generator, 0, i as u32) as usize;
        if j == i {
            continue;
        }
        a.swap(i, j);
    }
}

fn shuffle_katches_array(generator: &mut u64, a: &mut [Katch]) {
    let size = a.len();
    for i in 0..size {
        let j = pick_random(generator, 0, i as u32) as usize;
        if j == i {
            continue;
        }
        a.swap(i, j);
    }
}

fn shuffle_katches(kitten: &mut Kitten) {
    let lits = kitten.lits;
    for lit in 0..lits {
        let mut watches = std::mem::take(&mut kitten.watches[lit]);
        shuffle_katches_array(&mut kitten.generator, &mut watches);
        kitten.watches[lit] = watches;
    }
}

fn shuffle_queue(kitten: &mut Kitten) {
    let vars = (kitten.lits / 2) as u32;
    for _ in 0..vars {
        let idx = pick_random(&mut kitten.generator, 0, vars);
        dequeue(kitten, idx);
        enqueue(kitten, idx);
    }
    let last = kitten.queue.last;
    update_search(kitten, last);
}

fn shuffle_units(kitten: &mut Kitten) {
    let mut units = std::mem::take(&mut kitten.units);
    shuffle_unsigned_array(&mut kitten.generator, &mut units);
    kitten.units = units;
}

pub fn kitten_shuffle_clauses(kitten: &mut Kitten) {
    require_status(kitten, 0);
    shuffle_queue(kitten);
    shuffle_katches(kitten);
    shuffle_units(kitten);
}

/*------------------------------------------------------------------------*/

fn watch_klause(kitten: &mut Kitten, lit: u32, ref_: u32) {
    let size = kitten.k_size(ref_);
    debug_assert!(lit == kitten.k_lit(ref_, 0) || lit == kitten.k_lit(ref_, 1));
    let blit = kitten.k_lit(ref_, 0) ^ kitten.k_lit(ref_, 1) ^ lit;
    let binary = size == 2;
    debug_assert!(size > 1);
    let katch = Katch::new(blit, ref_, binary);
    kitten.watches[lit as usize].push(katch);
}

fn connect_new_klause(kitten: &mut Kitten, ref_: u32) {
    let size = kitten.k_size(ref_);
    if size == 0 {
        if kitten.inconsistent == INVALID {
            kitten.inconsistent = ref_;
        }
    } else if size == 1 {
        kitten.units.push(ref_);
    } else {
        let lit0 = kitten.k_lit(ref_, 0);
        let lit1 = kitten.k_lit(ref_, 1);
        watch_klause(kitten, lit0, ref_);
        watch_klause(kitten, lit1, ref_);
    }
}

fn new_reference(kitten: &mut Kitten, solver: &mut Solver) -> u32 {
    let ref_ = kitten.klauses.len();
    if ref_ >= INVALID as usize {
        crate::error::fatal(format_args!(
            "kitten: maximum number of literals exhausted"
        ));
    }
    let res = ref_ as u32;
    solver.statistics.kitten_ticks += 1; // INC (kitten_ticks)
    res
}

fn new_original_klause(kitten: &mut Kitten, solver: &mut Solver, id: u32) {
    let res = new_reference(kitten, solver);
    let size = kitten.klause.len() as u32;
    kitten.klauses.push(id);
    kitten.klauses.push(size);
    kitten.klauses.push(0);
    for i in 0..kitten.klause.len() {
        let lit = kitten.klause[i];
        kitten.klauses.push(lit);
    }
    connect_new_klause(kitten, res);
    kitten.end_original_ref = kitten.klauses.len();
}

fn enlarge_external(kitten: &mut Kitten, eidx: usize) {
    let old_size = kitten.esize;
    let old_evars = kitten.evars;
    debug_assert!(old_evars <= eidx);
    let new_evars = eidx + 1;
    if old_size <= eidx {
        let mut new_size = if old_size != 0 { 2 * old_size } else { 1 };
        while new_size <= eidx {
            new_size *= 2;
        }
        kitten.import.resize(new_size, 0);
        kitten.esize = new_size;
    }
    kitten.evars = new_evars;
}

fn import_literal(kitten: &mut Kitten, elit: u32) -> u32 {
    let eidx = (elit / 2) as usize;
    if eidx >= kitten.evars {
        enlarge_external(kitten, eidx);
    }
    let mut iidx = kitten.import[eidx];
    if iidx == 0 {
        iidx = kitten.export_.len() as u32;
        kitten.export_.push(eidx as u32);
        kitten.import[eidx] = iidx + 1;
    } else {
        iidx -= 1;
    }
    let ilit = 2 * iidx + (elit & 1);
    let new_lits = ((ilit | 1) + 1) as usize;
    debug_assert!((ilit as usize) < new_lits);
    if new_lits > kitten.lits {
        enlarge_internal(kitten, new_lits);
    }
    ilit
}

fn export_literal(kitten: &Kitten, ilit: u32) -> u32 {
    let iidx = (ilit / 2) as usize;
    debug_assert!(iidx < kitten.export_.len());
    let eidx = kitten.export_[iidx];
    2 * eidx + (ilit & 1)
}

/// C `new_learned_klause` (non-static in kitten.c but internal to it).
fn new_learned_klause(kitten: &mut Kitten, solver: &mut Solver) -> u32 {
    let res = new_reference(kitten, solver);
    let size = kitten.klause.len();
    let aux = if kitten.antecedents {
        kitten.resolved.len()
    } else {
        0
    };
    kitten.klauses.push(aux as u32);
    kitten.klauses.push(size as u32);
    kitten.klauses.push(LEARNED_FLAG);
    for i in 0..kitten.klause.len() {
        let lit = kitten.klause[i];
        kitten.klauses.push(lit);
    }
    if aux != 0 {
        for i in 0..kitten.resolved.len() {
            let ref_ = kitten.resolved[i];
            kitten.klauses.push(ref_);
        }
    }
    connect_new_klause(kitten, res);
    kitten.learned = true;
    res
}

pub fn kitten_clear(kitten: &mut Kitten) {
    debug_assert!(kitten.analyzed.is_empty());
    debug_assert!(kitten.klause.is_empty());
    debug_assert!(kitten.eclause.is_empty());
    debug_assert!(kitten.resolved.is_empty());

    kitten.assumptions.clear();
    kitten.core.clear();
    kitten.klause.clear();
    kitten.klauses.clear();
    kitten.trail.clear();
    kitten.units.clear();

    for kit in 0..kitten.lits {
        kitten.watches[kit].clear();
    }

    while let Some(eidx) = kitten.export_.pop() {
        kitten.import[eidx as usize] = 0;
    }

    let lits = kitten.size;
    let vars = lits / 2;

    if vars != 0 {
        for i in 0..vars {
            kitten.phases[i] = 0;
            kitten.vars[i] = Kar::default();
        }
    }

    if lits != 0 {
        for i in 0..lits {
            kitten.values[i] = 0;
            kitten.failed[i] = false;
        }
    }

    clear_kitten(kitten);
}

/// kitten_release: C frees everything; Rust drops.  Call sites set
/// `solver.kitten = None` (release_sweeper: `kitten_release (solver->kitten);
/// solver->kitten = 0;`).
pub fn kitten_release(kitten: Box<Kitten>) {
    drop(kitten);
}

/*------------------------------------------------------------------------*/

fn move_to_front(kitten: &mut Kitten, idx: u32) {
    if idx == kitten.queue.last {
        return;
    }
    dequeue(kitten, idx);
    enqueue(kitten, idx);
    debug_assert!(kitten.values[2 * idx as usize] != 0);
}

fn assign(kitten: &mut Kitten, solver: &mut Solver, lit: u32, reason: u32) {
    let not_lit = lit ^ 1;
    debug_assert!(kitten.values[lit as usize] == 0);
    debug_assert!(kitten.values[not_lit as usize] == 0);
    kitten.values[lit as usize] = 1;
    kitten.values[not_lit as usize] = -1;
    let idx = lit / 2;
    let sign = lit & 1;
    kitten.phases[idx as usize] = sign as u8;
    kitten.trail.push(lit);
    kitten.vars[idx as usize].level = kitten.level;
    let mut reason = reason;
    if kitten.vars[idx as usize].level == 0 {
        debug_assert!(reason != INVALID);
        if kitten.k_size(reason) > 1 {
            if kitten.antecedents {
                kitten.resolved.push(reason);
                let size = kitten.k_size(reason);
                for i in 0..size {
                    let other = kitten.k_lit(reason, i);
                    if other != lit {
                        let other_idx = other / 2;
                        let other_ref = kitten.vars[other_idx as usize].reason;
                        debug_assert!(other_ref != INVALID);
                        kitten.resolved.push(other_ref);
                    }
                }
            }
            kitten.klause.push(lit);
            reason = new_learned_klause(kitten, solver);
            kitten.resolved.clear();
            kitten.klause.clear();
        }
    }
    kitten.vars[idx as usize].reason = reason;
    debug_assert!(kitten.unassigned != 0);
    kitten.unassigned -= 1;
}

fn propagate_literal(kitten: &mut Kitten, solver: &mut Solver, lit: u32) -> u32 {
    debug_assert!(kitten.values[lit as usize] > 0);
    let not_lit = (lit ^ 1) as usize;
    let mut conflict = INVALID;
    let end_watches = kitten.watches[not_lit].len();
    let mut q: usize = 0;
    let mut p: usize = 0;
    // C: ticks = (((char *) end - (char *) q) >> 7) + 1 with 8-byte katches.
    let mut ticks: u64 = ((end_watches as u64 * 8) >> 7) + 1;
    while p != end_watches {
        let katch = kitten.watches[not_lit][p];
        kitten.watches[not_lit][q] = katch;
        q += 1;
        p += 1;
        let ref_ = katch.ref_();
        let blit = katch.blit;
        debug_assert!(blit != not_lit as u32);
        let blit_value = kitten.values[blit as usize];
        if blit_value > 0 {
            continue;
        }
        if katch.binary() {
            if blit_value < 0 {
                solver.statistics.kitten_conflicts += 1; // INC — STATISTIC tier
                conflict = ref_;
                break;
            } else {
                debug_assert!(blit_value == 0);
                assign(kitten, solver, blit, ref_);
                continue;
            }
        }
        debug_assert!(kitten.k_size(ref_) > 1);
        let other = kitten.k_lit(ref_, 0) ^ kitten.k_lit(ref_, 1) ^ not_lit as u32;
        let other_value = kitten.values[other as usize];
        ticks += 1;
        if other_value > 0 {
            kitten.watches[not_lit][q - 1].blit = other;
            continue;
        }
        let mut replacement_value: i8 = -1;
        let mut replacement = INVALID;
        let size = kitten.k_size(ref_);
        let mut r: u32 = 2;
        while r != size {
            replacement = kitten.k_lit(ref_, r);
            replacement_value = kitten.values[replacement as usize];
            if replacement_value >= 0 {
                break;
            }
            r += 1;
        }
        if replacement_value >= 0 {
            debug_assert!(replacement != INVALID);
            kitten.k_set_lit(ref_, 0, other);
            kitten.k_set_lit(ref_, 1, replacement);
            kitten.k_set_lit(ref_, r, not_lit as u32);
            watch_klause(kitten, replacement, ref_);
            q -= 1;
        } else if other_value < 0 {
            solver.statistics.kitten_conflicts += 1; // INC — STATISTIC tier
            conflict = ref_;
            break;
        } else {
            debug_assert!(other_value == 0);
            assign(kitten, solver, other, ref_);
        }
    }
    while p != end_watches {
        let katch = kitten.watches[not_lit][p];
        kitten.watches[not_lit][q] = katch;
        q += 1;
        p += 1;
    }
    kitten.watches[not_lit].truncate(q);
    solver.statistics.kitten_ticks += ticks; // ADD (kitten_ticks, ticks)
    conflict
}

fn propagate(kitten: &mut Kitten, solver: &mut Solver) -> u32 {
    debug_assert!(kitten.inconsistent == INVALID);
    let mut propagated: u64 = 0;
    let mut conflict = INVALID;
    while conflict == INVALID && (kitten.propagated as usize) < kitten.trail.len() {
        let lit = kitten.trail[kitten.propagated as usize];
        conflict = propagate_literal(kitten, solver, lit);
        kitten.propagated += 1;
        propagated += 1;
    }
    solver.statistics.kitten_propagations += propagated; // ADD — COUNTER
    conflict
}

fn bump(kitten: &mut Kitten) {
    let analyzed = std::mem::take(&mut kitten.analyzed);
    for &idx in analyzed.iter() {
        kitten.marks[idx as usize] = 0;
        move_to_front(kitten, idx);
    }
    kitten.analyzed = analyzed;
}

fn unassign(kitten: &mut Kitten, lit: u32) {
    let not_lit = lit ^ 1;
    debug_assert!(kitten.values[lit as usize] != 0);
    debug_assert!(kitten.values[not_lit as usize] != 0);
    let idx = (lit / 2) as usize;
    kitten.values[lit as usize] = 0;
    kitten.values[not_lit as usize] = 0;
    debug_assert!((kitten.unassigned as usize) < kitten.lits / 2);
    kitten.unassigned += 1;
    let stamp = kitten.links[idx].stamp;
    if stamp > kitten.links[kitten.queue.search as usize].stamp {
        update_search(kitten, idx as u32);
    }
}

fn backtrack(kitten: &mut Kitten, jump: u32) {
    debug_assert!(jump < kitten.level);
    while let Some(&lit) = kitten.trail.last() {
        let idx = (lit / 2) as usize;
        let level = kitten.vars[idx].level;
        if level == jump {
            break;
        }
        kitten.trail.pop();
        unassign(kitten, lit);
    }
    kitten.propagated = kitten.trail.len() as u32;
    kitten.level = jump;
}

/// C `completely_backtrack_to_root_level` (non-static, internal).
fn completely_backtrack_to_root_level(kitten: &mut Kitten) {
    let trail = std::mem::take(&mut kitten.trail);
    for &lit in trail.iter() {
        unassign(kitten, lit);
    }
    // trail was taken (CLEAR_STACK).
    drop(trail);
    let units = std::mem::take(&mut kitten.units);
    for &ref_ in units.iter() {
        debug_assert!(kitten.k_size(ref_) == 1);
        let unit = kitten.k_lit(ref_, 0);
        let value = kitten.values[unit as usize];
        if value <= 0 {
            continue;
        }
        unassign(kitten, unit);
    }
    kitten.units = units;
    kitten.propagated = 0;
    kitten.level = 0;
}

fn analyze(kitten: &mut Kitten, solver: &mut Solver, conflict: u32) {
    debug_assert!(kitten.level != 0);
    debug_assert!(kitten.inconsistent == INVALID);
    debug_assert!(kitten.analyzed.is_empty());
    debug_assert!(kitten.resolved.is_empty());
    debug_assert!(kitten.klause.is_empty());
    kitten.klause.push(INVALID);
    let mut reason = conflict;
    let level = kitten.level;
    let mut p = kitten.trail.len();
    let mut open: u32 = 0;
    let mut jump: u32 = 0;
    let mut size: u32 = 1;
    let uip;
    loop {
        debug_assert!(reason != INVALID);
        kitten.resolved.push(reason);
        let csize = kitten.k_size(reason);
        for i in 0..csize {
            let mut lit = kitten.k_lit(reason, i);
            let idx = (lit / 2) as usize;
            if kitten.marks[idx] != 0 {
                continue;
            }
            debug_assert!(kitten.values[lit as usize] < 0);
            kitten.marks[idx] = 1; // marks[idx] = true
            kitten.analyzed.push(idx as u32);
            let tmp = kitten.vars[idx].level;
            if tmp < level {
                if tmp > jump {
                    jump = tmp;
                    if size > 1 {
                        let other = kitten.klause[1];
                        kitten.klause[1] = lit;
                        lit = other;
                    }
                }
                kitten.klause.push(lit);
                size += 1;
            } else {
                open += 1;
            }
        }
        let idx;
        let mut this_uip;
        loop {
            debug_assert!(p > 0);
            p -= 1;
            this_uip = kitten.trail[p];
            if kitten.marks[(this_uip / 2) as usize] != 0 {
                idx = (this_uip / 2) as usize;
                break;
            }
        }
        debug_assert!(open != 0);
        open -= 1;
        if open == 0 {
            uip = this_uip;
            break;
        }
        reason = kitten.vars[idx].reason;
    }
    let not_uip = uip ^ 1;
    kitten.klause[0] = not_uip;
    bump(kitten);
    kitten.analyzed.clear();
    let learned_ref = new_learned_klause(kitten, solver);
    kitten.resolved.clear();
    kitten.klause.clear();
    backtrack(kitten, jump);
    assign(kitten, solver, not_uip, learned_ref);
}

fn failing(kitten: &mut Kitten, solver: &mut Solver) {
    debug_assert!(kitten.inconsistent == INVALID);
    debug_assert!(!kitten.assumptions.is_empty());
    debug_assert!(kitten.analyzed.is_empty());
    debug_assert!(kitten.resolved.is_empty());
    debug_assert!(kitten.klause.is_empty());
    let mut failed_clashing = INVALID;
    let mut first_failed = INVALID;
    let mut failed_unit = INVALID;
    for i in 0..kitten.assumptions.len() {
        let lit = kitten.assumptions[i];
        if kitten.values[lit as usize] >= 0 {
            continue;
        }
        if first_failed == INVALID {
            first_failed = lit;
        }
        let failed_idx = (lit / 2) as usize;
        if kitten.vars[failed_idx].level == 0 {
            failed_unit = lit;
            break;
        }
        if failed_clashing == INVALID && kitten.vars[failed_idx].reason == INVALID {
            failed_clashing = lit;
        }
    }
    let failed = if failed_unit != INVALID {
        failed_unit
    } else if failed_clashing != INVALID {
        failed_clashing
    } else {
        first_failed
    };
    debug_assert!(failed != INVALID);
    let failed_idx = (failed / 2) as usize;
    let failed_reason = kitten.vars[failed_idx].reason;
    kitten.failed[failed as usize] = true;

    if failed_unit != INVALID {
        debug_assert!(kitten.k_size(failed_reason) == 1);
        kitten.failing = failed_reason;
        return;
    }

    let not_failed = failed ^ 1;
    if failed_clashing != INVALID {
        kitten.failed[not_failed as usize] = true;
        debug_assert!(kitten.failing == INVALID);
        return;
    }

    debug_assert!(kitten.marks[failed_idx] == 0);
    kitten.marks[failed_idx] = 1;
    kitten.analyzed.push(failed_idx as u32);
    kitten.klause.push(not_failed);

    let mut work: Vec<u32> = Vec::new();

    debug_assert!(!kitten.trail.is_empty());
    let mut p = kitten.trail.len();
    let mut open: u32 = 1;
    loop {
        if open == 0 {
            break;
        }
        open -= 1;
        let idx;
        loop {
            debug_assert!(p > 0);
            p -= 1;
            let uip = kitten.trail[p];
            if kitten.marks[(uip / 2) as usize] != 0 {
                idx = (uip / 2) as usize;
                break;
            }
        }

        let reason = kitten.vars[idx].reason;
        if reason == INVALID {
            let mut lit = 2 * idx as u32;
            if kitten.values[lit as usize] < 0 {
                lit ^= 1;
            }
            debug_assert!(!kitten.failed[lit as usize]);
            kitten.failed[lit as usize] = true;
            let not_lit = lit ^ 1;
            kitten.klause.push(not_lit);
        } else {
            kitten.resolved.push(reason);
            let csize = kitten.k_size(reason);
            for i in 0..csize {
                let other = kitten.k_lit(reason, i);
                let other_idx = (other / 2) as usize;
                if kitten.marks[other_idx] != 0 {
                    continue;
                }
                debug_assert!(other_idx != idx);
                kitten.marks[other_idx] = 1;
                debug_assert!(kitten.values[other as usize] != 0);
                if kitten.vars[other_idx].level != 0 {
                    open += 1;
                } else {
                    work.push(other_idx as u32);
                }
                kitten.analyzed.push(other_idx as u32);
            }
        }
    }
    let mut next = 0usize;
    while next < work.len() {
        let idx = work[next] as usize;
        next += 1;
        let reason = kitten.vars[idx].reason;
        if reason == INVALID {
            let mut lit = 2 * idx as u32;
            if kitten.values[lit as usize] < 0 {
                lit ^= 1;
            }
            debug_assert!(!kitten.failed[lit as usize]);
            kitten.failed[lit as usize] = true;
            let not_lit = lit ^ 1;
            kitten.klause.push(not_lit);
        } else {
            kitten.resolved.push(reason);
        }
    }

    for i in 0..kitten.analyzed.len() {
        let idx = kitten.analyzed[i] as usize;
        debug_assert!(kitten.marks[idx] != 0);
        kitten.marks[idx] = 0;
    }
    kitten.analyzed.clear();

    drop(work); // RELEASE_STACK (work)

    let resolved = kitten.resolved.len();
    debug_assert!(resolved != 0);

    if resolved == 1 {
        kitten.failing = kitten.resolved[0];
    } else {
        kitten.failing = new_learned_klause(kitten, solver);
    }

    kitten.resolved.clear();
    kitten.klause.clear();
}

fn flush_trail(kitten: &mut Kitten) {
    debug_assert!(kitten.level == 0);
    kitten.propagated = 0;
    kitten.trail.clear();
}

fn decide(kitten: &mut Kitten, solver: &mut Solver) -> i32 {
    if kitten.level == 0 && !kitten.trail.is_empty() {
        flush_trail(kitten);
    }

    let mut decision = INVALID;
    let assumptions = kitten.assumptions.len();
    while (kitten.level as usize) < assumptions {
        let assumption = kitten.assumptions[kitten.level as usize];
        let value = kitten.values[assumption as usize];
        if value < 0 {
            failing(kitten, solver);
            return 20;
        } else if value > 0 {
            kitten.level += 1;
        } else {
            decision = assumption;
            break;
        }
    }

    if kitten.unassigned == 0 {
        return 10;
    }

    if solver.statistics.kitten_ticks >= kitten.limits.ticks {
        return -1;
    }

    if crate::terminated!(solver, kitten_terminated_1) {
        return -1;
    }

    if decision == INVALID {
        let mut idx = kitten.queue.search;
        loop {
            debug_assert!(idx != INVALID);
            if kitten.values[2 * idx as usize] == 0 {
                break;
            }
            idx = kitten.links[idx as usize].prev;
        }
        update_search(kitten, idx);
        let phase = kitten.phases[idx as usize] as u32;
        decision = 2 * idx + phase;
    }
    solver.statistics.kitten_decisions += 1; // INC — STATISTIC tier
    kitten.level += 1;
    assign(kitten, solver, decision, INVALID);
    0
}

fn inconsistent(kitten: &mut Kitten, solver: &mut Solver, ref_: u32) {
    debug_assert!(ref_ != INVALID);
    debug_assert!(kitten.inconsistent == INVALID);

    if !kitten.antecedents {
        kitten.inconsistent = ref_;
        return;
    }

    debug_assert!(kitten.analyzed.is_empty());
    debug_assert!(kitten.resolved.is_empty());

    let mut next: usize = 0;
    let mut ref_ = ref_;

    loop {
        debug_assert!(ref_ != INVALID);
        kitten.resolved.push(ref_);
        let csize = kitten.k_size(ref_);
        for i in 0..csize {
            let lit = kitten.k_lit(ref_, i);
            let idx = (lit / 2) as usize;
            debug_assert!(kitten.vars[idx].level == 0);
            if kitten.marks[idx] != 0 {
                continue;
            }
            debug_assert!(kitten.values[lit as usize] < 0);
            kitten.marks[idx] = 1;
            kitten.analyzed.push(idx as u32);
        }
        if next == kitten.analyzed.len() {
            break;
        }
        let idx = kitten.analyzed[next] as usize;
        next += 1;
        debug_assert!(kitten.vars[idx].level == 0);
        ref_ = kitten.vars[idx].reason;
    }
    debug_assert!(kitten.klause.is_empty());
    let ref_ = new_learned_klause(kitten, solver);
    kitten.inconsistent = ref_;

    for i in 0..kitten.analyzed.len() {
        let idx = kitten.analyzed[i] as usize;
        kitten.marks[idx] = 0;
    }

    kitten.analyzed.clear();
    kitten.resolved.clear();
}

fn propagate_units(kitten: &mut Kitten, solver: &mut Solver) -> i32 {
    if kitten.inconsistent != INVALID {
        return 20;
    }

    if kitten.units.is_empty() {
        return 0;
    }

    let mut next: usize = 0;
    while next < kitten.units.len() {
        let ref_ = kitten.units[next];
        next += 1;
        debug_assert!(ref_ != INVALID);
        debug_assert!(kitten.k_size(ref_) == 1);
        let unit = kitten.k_lit(ref_, 0);
        let value = kitten.values[unit as usize];
        if value > 0 {
            continue;
        }
        if value < 0 {
            inconsistent(kitten, solver, ref_);
            return 20;
        }
        assign(kitten, solver, unit, ref_);
    }
    let conflict = propagate(kitten, solver);
    if conflict == INVALID {
        return 0;
    }
    inconsistent(kitten, solver, conflict);
    20
}

/*------------------------------------------------------------------------*/

fn reset_core(kitten: &mut Kitten) {
    let mut c = 0u32;
    let end = kitten.klauses.len() as u32;
    while c != end {
        let next = kitten.next_klause(c);
        if kitten.is_core_klause(c) {
            kitten.unset_core_klause(c);
        }
        c = next;
    }
    kitten.core.clear();
}

fn reset_assumptions(kitten: &mut Kitten) {
    while let Some(assumption) = kitten.assumptions.pop() {
        kitten.failed[assumption as usize] = false;
    }
    kitten.assumptions.clear();
    if kitten.failing != INVALID {
        kitten.failing = INVALID;
    }
}

fn reset_incremental(kitten: &mut Kitten) {
    completely_backtrack_to_root_level(kitten);
    if !kitten.assumptions.is_empty() {
        reset_assumptions(kitten);
    } else {
        debug_assert!(kitten.failing == INVALID);
    }
    if kitten.status == 21 {
        reset_core(kitten);
    }
    kitten.status = 0; // UPDATE_STATUS (0)
}

/*------------------------------------------------------------------------*/

fn flip_literal(kitten: &mut Kitten, solver: &mut Solver, lit: u32) -> bool {
    solver.statistics.kitten_flip += 1; // INC — STATISTIC tier
    debug_assert!(kitten.values[lit as usize] != 0);
    if kitten.vars[(lit / 2) as usize].level == 0 {
        return false;
    }
    let mut lit = lit;
    if kitten.values[lit as usize] < 0 {
        lit ^= 1;
    }
    debug_assert!(kitten.values[lit as usize] > 0);
    let wlit = lit as usize;
    let end_watches = kitten.watches[wlit].len();
    let mut q: usize = 0;
    let mut p: usize = 0;
    // 8-byte katches; see propagate_literal.
    let mut ticks: u64 = ((end_watches as u64 * 8) >> 7) + 1;
    let mut res = true;
    while p != end_watches {
        let katch = kitten.watches[wlit][p];
        kitten.watches[wlit][q] = katch;
        q += 1;
        p += 1;
        let blit = katch.blit;
        debug_assert!(blit != lit);
        let blit_value = kitten.values[blit as usize];
        if blit_value > 0 {
            continue;
        }
        let ref_ = katch.ref_();
        let other = kitten.k_lit(ref_, 0) ^ kitten.k_lit(ref_, 1) ^ lit;
        let other_value = kitten.values[other as usize];
        ticks += 1;
        if other_value > 0 {
            continue;
        }
        let mut replacement_value: i8 = -1;
        let mut replacement = INVALID;
        let size = kitten.k_size(ref_);
        let mut r: u32 = 2;
        while r != size {
            replacement = kitten.k_lit(ref_, r);
            debug_assert!(replacement != lit);
            replacement_value = kitten.values[replacement as usize];
            debug_assert!(replacement_value != 0);
            if replacement_value > 0 {
                break;
            }
            r += 1;
        }
        if replacement_value > 0 {
            debug_assert!(replacement != INVALID);
            kitten.k_set_lit(ref_, 0, other);
            kitten.k_set_lit(ref_, 1, replacement);
            kitten.k_set_lit(ref_, r, lit);
            watch_klause(kitten, replacement, ref_);
            q -= 1;
        } else {
            debug_assert!(replacement_value < 0);
            res = false;
            break;
        }
    }
    while p != end_watches {
        let katch = kitten.watches[wlit][p];
        kitten.watches[wlit][q] = katch;
        q += 1;
        p += 1;
    }
    kitten.watches[wlit].truncate(q);
    solver.statistics.kitten_ticks += ticks; // ADD (kitten_ticks, ticks)
    if res {
        kitten.values[lit as usize] = -1;
        let not_lit = lit ^ 1;
        kitten.values[not_lit as usize] = 1;
        solver.statistics.kitten_flipped += 1; // INC — STATISTIC tier
    }
    res
}

/*------------------------------------------------------------------------*/

pub fn kitten_assume(kitten: &mut Kitten, elit: u32) {
    if kitten.status != 0 {
        reset_incremental(kitten);
    }
    let ilit = import_literal(kitten, elit);
    kitten.assumptions.push(ilit);
}

pub fn kitten_clause_with_id_and_exception(
    kitten: &mut Kitten,
    solver: &mut Solver,
    id: u32,
    elits: &[u32],
    except: u32,
) {
    if kitten.status != 0 {
        reset_incremental(kitten);
    }
    debug_assert!(kitten.klause.is_empty());
    for &elit in elits {
        if elit == except {
            continue;
        }
        let ilit = import_literal(kitten, elit);
        debug_assert!((ilit as usize) < kitten.lits);
        let iidx = (ilit / 2) as usize;
        if kitten.marks[iidx] != 0 {
            invalid_api_usage!(
                "variable '{}' of literal '{}' occurs twice",
                elit / 2,
                elit
            );
        }
        kitten.marks[iidx] = 1;
        kitten.klause.push(ilit);
    }
    for i in 0..kitten.klause.len() {
        let ilit = kitten.klause[i];
        kitten.marks[(ilit / 2) as usize] = 0;
    }
    new_original_klause(kitten, solver, id);
    kitten.klause.clear();
}

pub fn kitten_clause(kitten: &mut Kitten, solver: &mut Solver, elits: &[u32]) {
    kitten_clause_with_id_and_exception(kitten, solver, INVALID, elits, INVALID);
}

pub fn kitten_unit(kitten: &mut Kitten, solver: &mut Solver, lit: u32) {
    kitten_clause(kitten, solver, &[lit]);
}

pub fn kitten_binary(kitten: &mut Kitten, solver: &mut Solver, a: u32, b: u32) {
    kitten_clause(kitten, solver, &[a, b]);
}

pub fn kitten_solve(kitten: &mut Kitten, solver: &mut Solver) -> i32 {
    if kitten.status != 0 {
        reset_incremental(kitten);
    } else {
        completely_backtrack_to_root_level(kitten);
    }

    solver.statistics.kitten_solved += 1; // INC — COUNTER

    let mut res = propagate_units(kitten, solver);
    while res == 0 {
        let conflict = propagate(kitten, solver);
        if conflict != INVALID {
            if kitten.level != 0 {
                analyze(kitten, solver, conflict);
            } else {
                inconsistent(kitten, solver, conflict);
                res = 20;
            }
        } else {
            res = decide(kitten, solver);
        }
    }

    if res < 0 {
        res = 0;
    }

    if res == 0 && !kitten.assumptions.is_empty() {
        reset_assumptions(kitten);
    }

    kitten.status = res; // UPDATE_STATUS (res)

    if res == 10 {
        solver.statistics.kitten_sat += 1; // STATISTIC tier
    } else if res == 20 {
        solver.statistics.kitten_unsat += 1; // STATISTIC tier
    } else {
        solver.statistics.kitten_unknown += 1; // STATISTIC tier
    }

    res
}

pub fn kitten_status(kitten: &Kitten) -> i32 {
    kitten.status
}

/// Returns `(original, learned)`; the C `uint64_t *learned_ptr` out-parameter
/// is the second tuple element (callers passing NULL ignore it).
pub fn kitten_compute_clausal_core(kitten: &mut Kitten) -> (u32, u64) {
    require_status(kitten, 20);

    if !kitten.antecedents {
        invalid_api_usage!("antecedents not tracked");
    }

    debug_assert!(kitten.resolved.is_empty());

    let mut original: u32 = 0;
    let mut learned: u64 = 0;

    let mut reason_ref = kitten.inconsistent;

    'done: {
        if reason_ref == INVALID {
            debug_assert!(!kitten.assumptions.is_empty());
            reason_ref = kitten.failing;
            if reason_ref == INVALID {
                break 'done; // assumptions mutually inconsistent
            }
        }

        kitten.resolved.push(reason_ref);
        debug_assert!(kitten.core.is_empty());

        while let Some(c_ref) = kitten.resolved.pop() {
            if c_ref == INVALID {
                let d_ref = kitten.resolved.pop().unwrap();
                kitten.core.push(d_ref);
                debug_assert!(!kitten.is_core_klause(d_ref));
                kitten.set_core_klause(d_ref);
                if kitten.is_learned_klause(d_ref) {
                    learned += 1;
                } else {
                    original += 1;
                }
            } else {
                if kitten.is_core_klause(c_ref) {
                    continue;
                }
                kitten.resolved.push(c_ref);
                kitten.resolved.push(INVALID);
                if !kitten.is_learned_klause(c_ref) {
                    continue;
                }
                let aux = kitten.k_aux(c_ref);
                for i in 0..aux {
                    let d_ref = kitten.k_antecedent(c_ref, i);
                    if !kitten.is_core_klause(d_ref) {
                        kitten.resolved.push(d_ref);
                    }
                }
            }
        }
    }

    kitten.status = 21; // UPDATE_STATUS (21)

    (original, learned)
}

/// C passes `void *state` + function pointer; ported as a closure over the
/// original-clause id (`c->aux`).
pub fn kitten_traverse_core_ids(kitten: &Kitten, mut traverse: impl FnMut(u32)) {
    require_status(kitten, 21);

    let mut c = 0u32;
    let end = kitten.end_original_ref as u32;
    while c != end {
        let next = kitten.next_klause(c);
        debug_assert!(!kitten.is_learned_klause(c));
        if !kitten.is_learned_klause(c) && kitten.is_core_klause(c) {
            traverse(kitten.k_aux(c));
        }
        c = next;
    }

    debug_assert!(kitten.status == 21);
}

/// Closure receives (learned, exported-literal slice) per core clause.
pub fn kitten_traverse_core_clauses(
    kitten: &mut Kitten,
    mut traverse: impl FnMut(bool, &[u32]),
) {
    require_status(kitten, 21);

    let mut eclause = std::mem::take(&mut kitten.eclause);
    for i in 0..kitten.core.len() {
        let c_ref = kitten.core[i];
        debug_assert!(kitten.is_core_klause(c_ref));
        let learned = kitten.is_learned_klause(c_ref);
        debug_assert!(eclause.is_empty());
        let size = kitten.k_size(c_ref);
        for j in 0..size {
            let ilit = kitten.k_lit(c_ref, j);
            let elit = export_literal(kitten, ilit);
            eclause.push(elit);
        }
        traverse(learned, &eclause);
        eclause.clear();
    }
    kitten.eclause = eclause;

    debug_assert!(kitten.status == 21);
}

pub fn kitten_shrink_to_clausal_core(kitten: &mut Kitten) {
    require_status(kitten, 21);

    kitten.trail.clear();

    kitten.unassigned = (kitten.lits / 2) as u32;
    kitten.propagated = 0;
    kitten.level = 0;

    let last = kitten.queue.last;
    update_search(kitten, last);

    for i in 0..kitten.lits {
        kitten.values[i] = 0;
    }

    for lit in 0..kitten.lits {
        kitten.watches[lit].clear();
    }

    debug_assert!(kitten.inconsistent != INVALID);
    if kitten.is_learned_klause(kitten.inconsistent) || kitten.k_size(kitten.inconsistent) != 0 {
        kitten.inconsistent = INVALID;
    }

    kitten.units.clear();

    let mut q: usize = 0; // (unsigned *) q - (unsigned *) begin
    let end = kitten.end_original_ref;
    let mut c: usize = 0;
    while c != end {
        let next = kitten.next_klause(c as u32) as usize;
        debug_assert!(!kitten.is_learned_klause(c as u32));
        if kitten.is_learned_klause(c as u32) || !kitten.is_core_klause(c as u32) {
            c = next;
            continue;
        }
        kitten.unset_core_klause(c as u32);
        let dst = q as u32;
        let size = kitten.k_size(c as u32);
        if size == 0 {
            // PORT NOTE: C quirk kept as-is — `if (!kitten->inconsistent)`
            // tests the *reference value* against zero, not against INVALID.
            if kitten.inconsistent == 0 {
                kitten.inconsistent = dst;
            }
        } else if size == 1 {
            kitten.units.push(dst);
        } else {
            // PORT NOTE (bug fix during eliminate-wave validation): C's
            // watch_klause (kitten, c->lits[0], c, dst) reads the blocking
            // literal and size from the OLD clause location `c` while storing
            // the NEW reference `dst`; the Rust watch_klause re-reads them
            // from `dst`, which still holds stale words before the
            // copy_within below.  Construct the katches from `c` directly.
            let lit0 = kitten.k_lit(c as u32, 0);
            let lit1 = kitten.k_lit(c as u32, 1);
            let binary = size == 2;
            kitten.watches[lit0 as usize].push(Katch::new(lit1, dst, binary));
            kitten.watches[lit1 as usize].push(Katch::new(lit0, dst, binary));
        }
        if c == q {
            q = next;
        } else {
            kitten.klauses.copy_within(c..next, q);
            q += next - c;
        }
        c = next;
    }
    kitten.klauses.truncate(q); // SET_END_OF_STACK
    kitten.end_original_ref = kitten.klauses.len();

    kitten.core.clear();

    kitten.status = 0; // UPDATE_STATUS (0)
}

pub fn kitten_value(kitten: &Kitten, elit: u32) -> i8 {
    require_status(kitten, 10);
    let eidx = (elit / 2) as usize;
    if eidx >= kitten.evars {
        return 0;
    }
    let iidx = kitten.import[eidx];
    if iidx == 0 {
        return 0;
    }
    let ilit = 2 * (iidx - 1) + (elit & 1);
    kitten.values[ilit as usize]
}

pub fn kitten_fixed(kitten: &Kitten, elit: u32) -> i8 {
    let eidx = (elit / 2) as usize;
    if eidx >= kitten.evars {
        return 0;
    }
    let mut iidx = kitten.import[eidx];
    if iidx == 0 {
        return 0;
    }
    iidx -= 1;
    let ilit = 2 * iidx + (elit & 1);
    let res = kitten.values[ilit as usize];
    if res == 0 {
        return 0;
    }
    if kitten.vars[iidx as usize].level != 0 {
        return 0;
    }
    res
}

pub fn kitten_flip_literal(kitten: &mut Kitten, solver: &mut Solver, elit: u32) -> bool {
    require_status(kitten, 10);
    let eidx = (elit / 2) as usize;
    if eidx >= kitten.evars {
        return false;
    }
    let iidx = kitten.import[eidx];
    if iidx == 0 {
        return false;
    }
    let ilit = 2 * (iidx - 1) + (elit & 1);
    if kitten_fixed(kitten, elit) != 0 {
        return false;
    }
    flip_literal(kitten, solver, ilit)
}

pub fn kitten_failed(kitten: &Kitten, elit: u32) -> bool {
    require_status(kitten, 20);
    let eidx = (elit / 2) as usize;
    if eidx >= kitten.evars {
        return false;
    }
    let iidx = kitten.import[eidx];
    if iidx == 0 {
        return false;
    }
    let ilit = 2 * (iidx - 1) + (elit & 1);
    kitten.failed[ilit as usize]
}

/*------------------------------------------------------------------------*/

#[cfg(test)]
mod tests {
    use super::*;

    // External kitten literals in tests: variable v (0-based), sign s →
    // lit = 2*v + s, exactly as sweep passes kissat internal literals.
    fn elit(var: u32, negated: bool) -> u32 {
        2 * var + negated as u32
    }

    fn test_solver() -> Solver {
        Solver::default()
    }

    #[test]
    fn sat_simple() {
        let mut solver = test_solver();
        let mut kitten = kitten_embedded();
        // (v0 | v1) & (~v0 | v1)  =>  v1 true
        kitten_clause(&mut kitten, &mut solver, &[elit(0, false), elit(1, false)]);
        kitten_clause(&mut kitten, &mut solver, &[elit(0, true), elit(1, false)]);
        let res = kitten_solve(&mut kitten, &mut solver);
        assert_eq!(res, 10);
        assert_eq!(kitten_status(&kitten), 10);
        assert_eq!(kitten_value(&kitten, elit(1, false)), 1);
        assert_eq!(kitten_value(&kitten, elit(1, true)), -1);
        // Statistics were exercised.
        assert!(solver.statistics.kitten_solved == 1);
        assert!(solver.statistics.kitten_ticks > 0);
    }

    #[test]
    fn unsat_units_with_core() {
        let mut solver = test_solver();
        let mut kitten = kitten_embedded();
        kitten_track_antecedents(&mut kitten);
        // v0 & ~v0
        kitten_clause_with_id_and_exception(&mut kitten, &mut solver, 7, &[elit(0, false)], INVALID);
        kitten_clause_with_id_and_exception(&mut kitten, &mut solver, 8, &[elit(0, true)], INVALID);
        let res = kitten_solve(&mut kitten, &mut solver);
        assert_eq!(res, 20);
        let (original, _learned) = kitten_compute_clausal_core(&mut kitten);
        assert_eq!(original, 2);
        let mut ids = Vec::new();
        kitten_traverse_core_ids(&kitten, |id| ids.push(id));
        ids.sort_unstable();
        assert_eq!(ids, vec![7, 8]);
    }

    #[test]
    fn unsat_search_with_core() {
        let mut solver = test_solver();
        let mut kitten = kitten_embedded();
        kitten_track_antecedents(&mut kitten);
        // All 8 sign combinations over 3 vars: UNSAT, needs real search.
        let mut id = 0;
        for mask in 0..8u32 {
            let clause = [
                elit(0, mask & 1 != 0),
                elit(1, mask & 2 != 0),
                elit(2, mask & 4 != 0),
            ];
            kitten_clause_with_id_and_exception(&mut kitten, &mut solver, id, &clause, INVALID);
            id += 1;
        }
        let res = kitten_solve(&mut kitten, &mut solver);
        assert_eq!(res, 20);
        let (original, learned) = kitten_compute_clausal_core(&mut kitten);
        assert!(original > 0 && original <= 8);
        assert!(learned > 0); // conflict-driven refutation must have lemmas
        // Core clauses traverse with exported literals; learned lemmas
        // (including the final empty klause) and original core clauses.
        let mut n_learned = 0u32;
        let mut n_original = 0u32;
        kitten_traverse_core_clauses(&mut kitten, |learned, lits| {
            if learned {
                n_learned += 1;
            } else {
                n_original += 1;
                assert_eq!(lits.len(), 3);
            }
        });
        assert_eq!(n_original, original);
        assert_eq!(n_learned as u64, learned);
    }

    #[test]
    fn failed_assumptions() {
        let mut solver = test_solver();
        let mut kitten = kitten_embedded();
        kitten_track_antecedents(&mut kitten);
        // v0 -> v1  (~v0 | v1)
        kitten_clause(&mut kitten, &mut solver, &[elit(0, true), elit(1, false)]);
        kitten_assume(&mut kitten, elit(0, false));
        kitten_assume(&mut kitten, elit(1, true));
        let res = kitten_solve(&mut kitten, &mut solver);
        assert_eq!(res, 20);
        assert!(kitten_failed(&kitten, elit(1, true)));
        // Incremental reuse: without assumptions the formula is SAT.
        let res = kitten_solve(&mut kitten, &mut solver);
        assert_eq!(res, 10);
    }

    #[test]
    fn flip_and_shuffle() {
        let mut solver = test_solver();
        let mut kitten = kitten_embedded();
        // Two independent models: (v0 | v1)
        kitten_clause(&mut kitten, &mut solver, &[elit(0, false), elit(1, false)]);
        kitten_randomize_phases(&mut kitten);
        let res = kitten_solve(&mut kitten, &mut solver);
        assert_eq!(res, 10);
        let v0 = kitten_value(&kitten, elit(0, false));
        let v1 = kitten_value(&kitten, elit(1, false));
        assert!(v0 != 0 && v1 != 0);
        if v0 > 0 && v1 > 0 {
            // Both true: either can flip.
            assert!(kitten_flip_literal(&mut kitten, &mut solver, elit(0, false)));
            assert_eq!(kitten_value(&kitten, elit(0, false)), -1);
            // Now v1 is the only satisfying literal of the clause: cannot flip.
            assert!(!kitten_flip_literal(&mut kitten, &mut solver, elit(1, false)));
        } else {
            // Exactly one true: that literal cannot be flipped.
            let sat = if v0 > 0 { elit(0, false) } else { elit(1, false) };
            assert!(!kitten_flip_literal(&mut kitten, &mut solver, sat));
        }
    }

    #[test]
    fn clear_and_reuse() {
        let mut solver = test_solver();
        let mut kitten = kitten_embedded();
        kitten_clause(&mut kitten, &mut solver, &[elit(0, false)]);
        kitten_clause(&mut kitten, &mut solver, &[elit(0, true)]);
        assert_eq!(kitten_solve(&mut kitten, &mut solver), 20);
        kitten_clear(&mut kitten);
        kitten_track_antecedents(&mut kitten);
        kitten_clause(&mut kitten, &mut solver, &[elit(3, false), elit(4, false)]);
        assert_eq!(kitten_solve(&mut kitten, &mut solver), 10);
    }

    #[test]
    fn ticks_limit_unknown() {
        let mut solver = test_solver();
        let mut kitten = kitten_embedded();
        // Hard-ish pigeonhole-flavored formula so search does not finish in
        // zero decisions; with a 0-tick budget solve must return unknown (0).
        for mask in 0..8u32 {
            let clause = [
                elit(0, mask & 1 != 0),
                elit(1, mask & 2 != 0),
                elit(2, mask & 4 != 0),
            ];
            kitten_clause(&mut kitten, &mut solver, &clause);
        }
        kitten_set_ticks_limit(&mut kitten, &solver, 0);
        let res = kitten_solve(&mut kitten, &mut solver);
        assert_eq!(res, 0);
        assert_eq!(kitten_status(&kitten), 0);
        kitten_no_ticks_limit(&mut kitten);
        assert_eq!(kitten_solve(&mut kitten, &mut solver), 20);
    }
}
