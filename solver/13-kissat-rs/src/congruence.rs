// Port of src/congruence.c (kissat 4.0.4).
//
// Build configuration (matching the reference `gcc -O3 -DNDEBUG` build):
//   - INDEX_LARGE_CLAUSES / INDEX_BINARY_CLAUSES NOT defined (the commented
//     out `#define`s at the top of congruence.c), so the AND-gate search uses
//     find_first_and_gate / find_remaining_and_gate over binary watch lists
//     and the XOR side-clause search uses find_large_xor_side_clause.
//   - MERGE_CONDITIONAL_EQUIVALENCES IS defined, so ITE extraction goes
//     through conditional-equivalence merging (extract_ite_gates_of_variable);
//     the alternative extract_ite_gates_with_base_clause path is not compiled.
//   - CHECKING_OR_PROVING is defined (NPROOFS not defined), so all proof
//     chain code (closure->chain, add_*_proof_chain, delete_proof_chain) is
//     compiled in; the CHECK_AND_ADD_* / REMOVE_CHECKER_* checker macros are
//     no-ops (NDEBUG, no CHECKING).
//
// PORT NOTE (gate storage / hash-order fidelity): C heap-allocates `struct
// gate` and stores raw pointers in the open-addressing hash table and the
// occurrence lists.  Here gates live in an index arena `Closure::gates`
// (never freed until the closure is dropped; C only frees gates during
// reset, when no dangling pointer is ever dereferenced, so keeping them is
// unobservable).  Pointer identity becomes the GateId index.  The table
// stores GateIds with NULL == NO_GATE (u32::MAX) and REMOVED ==
// u32::MAX - 1.  The hash function (nonce mix with rotate-left-4), the
// table sizes (doubling from 1, "full" at 2*entries >= size, capped at
// 1 << 32), reduce_hash and the linear probing/wrap-around order are ported
// verbatim so that which gate is *found first* — and hence the whole merge
// trajectory — is identical to C.  No std HashMap anywhere.
//
// PORT NOTE (nonces): C copies `generator random = solver->random` and draws
// 16 nonces from the copy — solver->random itself is NOT advanced.  Ported
// exactly (the copy is a local u64).
//
// PORT NOTE (FIFO): C's unsigned_fifo (fifo.h) is a plain FIFO (enqueue at
// end, dequeue at front); its buffer-moving policy only affects allocation,
// not order.  Ported as std::collections::VecDeque<u32>.
//
// PORT NOTE: `closure->equivalences` is declared in C but never used; it is
// omitted.  `closure->units` (a pointer into the trail) becomes a usize
// index.  Gate `id` and `closure->gates_added` exist only under
// LOGGING/!NDEBUG and are omitted.
//
// PORT NOTE (dense mode): kissat_enter_dense_mode / kissat_resume_sparse_mode
// live in dense.c (another wave); call sites go through crate::dense (stubbed
// in stubs.rs until that wave lands — kissat_congruence itself is only
// reachable from the probe driver, also pending).

use std::collections::VecDeque;

use crate::internal::{Solver, INVALID};
use crate::kimits::DelayId;
use crate::literal::{idx, lit as make_lit, negated, not, strip};
use crate::profile::Prof;
use crate::reference::{Reference, INVALID_REF};
use crate::terminated;
use crate::utilities::percent;
use crate::watch::{watch_is_binary, watch_lit, watch_ref, LitPair};

const AND_GATE: u32 = 0;
const XOR_GATE: u32 = 1;
const ITE_GATE: u32 = 2;

const LD_MAX_ARITY: u32 = 26;
const MAX_ARITY: u32 = (1 << LD_MAX_ARITY) - 1;

const SIZE_NONCES: usize = 16;

type GateId = u32;
/// C NULL entry in the gate hash table.
const NO_GATE: GateId = u32::MAX;
/// C `REMOVED` sentinel (`~(uintptr_t) 0`).
const REMOVED: GateId = u32::MAX - 1;

/// `MAX_HASH_TABLE_SIZE` — ((size_t) 1 << 32).
const MAX_HASH_TABLE_SIZE: usize = 1usize << 32;

/// struct gate.
struct Gate {
    lhs: u32,
    hash: u32,
    tag: u32,
    garbage: bool,
    indexed: bool,
    #[allow(dead_code)]
    marked: bool,
    shrunken: bool,
    arity: u32,
    /// Allocated once with the original arity; shrinking overwrites in place
    /// exactly like C (terminator INVALID at the old last slot).
    rhs: Vec<u32>,
}

/// struct closure.
pub struct Closure {
    scheduled: Vec<bool>,          // bool *scheduled;      (VARS)
    occurrences: Vec<Vec<GateId>>, // gates *occurrences;   (LITS)
    garbage: Vec<GateId>,          // gates garbage;
    lits: Vec<u32>,                // unsigneds lits;
    rhs: Vec<u32>,                 // unsigneds rhs;
    unsimplified: Vec<u32>,        // unsigneds unsimplified;
    binaries: Vec<LitPair>,        // litpairs binaries;
    schedule: VecDeque<u32>,       // unsigned_fifo schedule;
    repr: Vec<u32>,                // unsigned *repr;       (LITS)
    hash_table: Vec<GateId>,       // gate_hash_table hash;
    hash_entries: usize,
    nonces: [u64; SIZE_NONCES],
    units: usize,          // unsigned *units (index into solver.trail)
    negbincount: Vec<u32>, // unsigned *negbincount; (LITS)
    largecount: Vec<u32>,  // unsigned *largecount;  (LITS)
    condbin: [Vec<LitPair>; 2],
    condeq: [Vec<LitPair>; 2],
    chain: Vec<u32>, // unsigneds chain; (CHECKING_OR_PROVING)
    gates: Vec<Gate>,
}

// static void init_closure
fn init_closure(solver: &mut Solver) -> Closure {
    let vars = solver.vars as usize;
    let lits = solver.lits() as usize;
    let mut repr = Vec::with_capacity(lits);
    for l in 0..lits as u32 {
        repr.push(l);
    }
    let mut nonces = [0u64; SIZE_NONCES];
    let mut random = solver.random; // generator random = solver->random;
    for nonce in nonces.iter_mut() {
        *nonce = 1 | crate::random::next_random64(&mut random);
    }
    Closure {
        scheduled: vec![false; vars],
        occurrences: vec![Vec::new(); lits],
        garbage: Vec::new(),
        lits: Vec::new(),
        rhs: Vec::new(),
        unsimplified: Vec::new(),
        binaries: Vec::new(),
        schedule: VecDeque::new(),
        repr,
        hash_table: Vec::new(),
        hash_entries: 0,
        nonces,
        units: 0,
        negbincount: Vec::new(),
        largecount: Vec::new(),
        condbin: [Vec::new(), Vec::new()],
        condeq: [Vec::new(), Vec::new()],
        chain: Vec::new(),
        gates: Vec::new(),
    }
}

// void reset_gate_hash_table (non-static in C, but only used here).
// delete_gate is a free in C — nothing to do in the arena model.
fn reset_gate_hash_table(closure: &mut Closure) {
    closure.hash_table = Vec::new();
    closure.hash_entries = 0;
}

// static void reset_closure
fn reset_closure(solver: &mut Solver, closure: &mut Closure) {
    for occs in closure.occurrences.iter_mut() {
        *occs = Vec::new(); // RELEASE_STACK (occurrences[lit])
    }
    closure.occurrences = Vec::new();

    reset_gate_hash_table(closure);
    closure.garbage = Vec::new();
    closure.binaries = Vec::new();
    closure.scheduled = Vec::new();
    closure.lits = Vec::new();
    closure.rhs = Vec::new();
    closure.unsimplified = Vec::new();
    closure.schedule = VecDeque::new();
    closure.chain = Vec::new();

    if !solver.inconsistent && solver.unflushed != 0 {
        crate::trail::flush_trail(solver);
    }
}

// static unsigned reset_repr
fn reset_repr(solver: &mut Solver, closure: &mut Closure) -> u32 {
    let mut res = 0u32;
    for i in 0..solver.vars {
        let l = make_lit(i);
        if solver.values[l as usize] == 0 && closure.repr[l as usize] != l {
            res += 1;
        }
    }
    closure.repr = Vec::new();
    res
}

// static void sort_lits — SORT (unsigned, arity, lits, LESS_LIT)
fn sort_lits(solver: &mut Solver, lits: &mut [u32]) {
    crate::sort::sort(&mut solver.sorter, lits, |a, b| a < b);
}

// static unsigned hash_lits
fn hash_lits(nonces: &[u64; SIZE_NONCES], tag: u32, lits: &[u32]) -> u32 {
    let mut n = tag as usize;
    debug_assert!(n < SIZE_NONCES);
    let mut hash = 0u64;
    for &l in lits {
        hash = hash.wrapping_add(l as u64);
        hash = hash.wrapping_mul(nonces[n]);
        n += 1;
        hash = (hash << 4) | (hash >> 60);
        if n == SIZE_NONCES {
            n = 0;
        }
    }
    hash ^= hash >> 32;
    hash as u32
}

// static size_t reduce_hash
fn reduce_hash(hash: u32, size: usize, size2: usize) -> usize {
    debug_assert!(size <= size2);
    debug_assert!(size2 <= 2 * size);
    let mut res = (hash as usize) & (size2 - 1);
    if res >= size {
        res -= size;
    }
    debug_assert!(res < size);
    res
}

// static bool closure_hash_table_is_full
fn closure_hash_table_is_full(closure: &Closure) -> bool {
    if closure.hash_table.len() == MAX_HASH_TABLE_SIZE {
        return false;
    }
    if 2 * closure.hash_entries < closure.hash_table.len() {
        return false;
    }
    true
}

// static bool match_lits
fn match_lits(gate: &Gate, tag: u32, hash: u32, size: usize, lits: &[u32]) -> bool {
    debug_assert!(!gate.garbage);
    if gate.tag != tag {
        return false;
    }
    if gate.hash != hash {
        return false;
    }
    if gate.arity as usize != size {
        return false;
    }
    for (i, &l) in gate.rhs[..gate.arity as usize].iter().enumerate() {
        if l != lits[i] {
            return false;
        }
    }
    true
}

// static void resize_gate_hash_table
fn resize_gate_hash_table(solver: &mut Solver, closure: &mut Closure) {
    let old_size = closure.hash_table.len();
    let new_size = if old_size != 0 { 2 * old_size } else { 1 };
    let old_entries = closure.hash_entries;
    crate::print::extremely_verbose(
        solver,
        format_args!(
            "resizing gate table of size {} filled with {} entries {:.0}%",
            old_size,
            old_entries,
            percent(old_entries as f64, old_size as f64)
        ),
    );
    let mut new_table = vec![NO_GATE; new_size];
    let mut flushed = 0usize;
    for old_pos in 0..old_size {
        let g = closure.hash_table[old_pos];
        if g == NO_GATE {
            continue;
        }
        if g == REMOVED {
            flushed += 1;
            continue;
        }
        let mut new_pos = reduce_hash(closure.gates[g as usize].hash, new_size, new_size);
        while new_table[new_pos] != NO_GATE {
            debug_assert!(new_table[new_pos] != REMOVED);
            new_pos += 1;
            if new_pos == new_size {
                new_pos = 0;
            }
        }
        new_table[new_pos] = g;
    }
    crate::print::extremely_verbose(
        solver,
        format_args!(
            "flushed {} entries {:.0}% resizing table of size {}",
            flushed,
            percent(flushed as f64, old_size as f64),
            old_size
        ),
    );
    debug_assert!(flushed <= old_entries);
    let new_entries = old_entries - flushed;
    closure.hash_table = new_table;
    closure.hash_entries = new_entries;
    crate::print::very_verbose(
        solver,
        format_args!(
            "resized gate table to {} with {} entries {:.0}%",
            new_size,
            new_entries,
            percent(new_entries as f64, new_size as f64)
        ),
    );
}

// static bool remove_gate
fn remove_gate(solver: &mut Solver, closure: &mut Closure, g: GateId) -> bool {
    if !closure.gates[g as usize].indexed {
        return false;
    }
    debug_assert!(!solver.inconsistent);
    let hash_size = closure.hash_table.len();
    let mut pos = reduce_hash(closure.gates[g as usize].hash, hash_size, hash_size);
    solver.statistics.congruent_lookups += 1;
    solver.statistics.congruent_lookups_removed += 1;
    let mut collisions = 0u64;
    while closure.hash_table[pos] != g {
        collisions += 1;
        pos += 1;
        if pos == hash_size {
            pos = 0;
        }
    }
    solver.statistics.congruent_collisions_removed += collisions;
    solver.statistics.congruent_collisions += collisions;
    closure.hash_table[pos] = REMOVED;
    closure.gates[g as usize].indexed = false;
    true
}

// static gate *find_gate — `except == NO_GATE` is C's NULL.
fn find_gate(
    solver: &mut Solver,
    closure: &mut Closure,
    tag: u32,
    hash: u32,
    size: usize,
    lits: &[u32],
    except: GateId,
) -> GateId {
    if closure.hash_entries == 0 {
        return NO_GATE;
    }
    debug_assert!(!solver.inconsistent);
    let hash_size = closure.hash_table.len();
    let start_pos = reduce_hash(hash, hash_size, hash_size);
    solver.statistics.congruent_lookups += 1;
    solver.statistics.congruent_lookups_find += 1;
    let mut pos = start_pos;
    let mut collisions = 0u64;
    let mut res = NO_GATE;
    loop {
        let g = closure.hash_table[pos];
        if g == NO_GATE {
            break;
        }
        if g == REMOVED {
            // skip
        } else if closure.gates[g as usize].garbage {
            debug_assert!(closure.gates[g as usize].indexed);
            closure.gates[g as usize].indexed = false;
            closure.hash_table[pos] = REMOVED;
        } else if g != except && match_lits(&closure.gates[g as usize], tag, hash, size, lits) {
            solver.statistics.congruent_matched += 1;
            res = g;
            break;
        }
        collisions += 1;
        pos += 1;
        if pos == hash_size {
            pos = 0;
        }
        if pos == start_pos {
            break;
        }
    }
    solver.statistics.congruent_collisions_find += collisions;
    solver.statistics.congruent_collisions += collisions;
    res
}

// static void index_gate
fn index_gate(solver: &mut Solver, closure: &mut Closure, g: GateId) {
    debug_assert!(!closure.gates[g as usize].indexed);
    debug_assert!(!solver.inconsistent);
    debug_assert!(closure.gates[g as usize].arity > 1);
    if closure_hash_table_is_full(closure) {
        resize_gate_hash_table(solver, closure);
    }
    solver.statistics.congruent_indexed += 1;
    let hash_size = closure.hash_table.len();
    let mut pos = reduce_hash(closure.gates[g as usize].hash, hash_size, hash_size);
    let mut collisions = 0u64;
    loop {
        let h = closure.hash_table[pos];
        if h == NO_GATE || h == REMOVED {
            break;
        }
        collisions += 1;
        pos += 1;
        if pos == hash_size {
            pos = 0;
        }
    }
    solver.statistics.congruent_collisions_index += collisions;
    solver.statistics.congruent_collisions += collisions;
    closure.hash_table[pos] = g;
    closure.hash_entries += 1;
    closure.gates[g as usize].indexed = true;
}

// static unsigned parity_lits
fn parity_lits(lits: &[u32]) -> u32 {
    let mut res = 0u32;
    for &l in lits {
        res ^= negated(l);
    }
    res
}

// static void inc_lits
fn inc_lits(lits: &mut [u32]) {
    let mut carry = true;
    let mut i = 0usize;
    while carry && i != lits.len() {
        let l = lits[i];
        let not_lit = not(l);
        carry = negated(not_lit) == 0;
        lits[i] = not_lit;
        i += 1;
    }
}

// check_* functions are !NDEBUG only — omitted.

// static inline unsigned find_repr
fn find_repr(closure: &Closure, lit: u32) -> u32 {
    let repr = &closure.repr;
    let mut res = lit;
    let mut next = repr[res as usize];
    while res != next {
        res = next;
        next = repr[res as usize];
    }
    res
}

// static bool learn_congruence_unit
fn learn_congruence_unit(solver: &mut Solver, _closure: &mut Closure, unit: u32) -> bool {
    debug_assert!(!solver.inconsistent);
    let value = solver.values[unit as usize];
    if value > 0 {
        return true;
    }
    solver.statistics.congruent_units += 1;
    if value < 0 {
        solver.inconsistent = true;
        // CHECK_AND_ADD_EMPTY (): no-op.  ADD_EMPTY_TO_PROOF ():
        if solver.proof.is_some() {
            crate::proof::add_empty_to_proof(solver);
        }
        return false;
    }
    crate::assign::learned_unit(solver, unit);
    let conflict = crate::proprobe::probing_propagate(solver, INVALID_REF, false);
    if conflict.is_none() {
        return true;
    }
    debug_assert!(solver.inconsistent);
    false
}

// static void add_binary_clause
fn add_binary_clause(solver: &mut Solver, closure: &mut Closure, a: u32, b: u32) {
    if solver.inconsistent {
        return;
    }
    if a == not(b) {
        return;
    }
    let a_value = solver.values[a as usize];
    if a_value > 0 {
        return;
    }
    let b_value = solver.values[b as usize];
    if b_value > 0 {
        return;
    }
    let mut unit = INVALID;
    if a == b {
        unit = a;
    } else if a_value < 0 && b_value == 0 {
        unit = b;
    } else if a_value == 0 && b_value < 0 {
        unit = a;
    }
    if unit != INVALID {
        let _ = learn_congruence_unit(solver, closure, unit);
        return;
    }
    debug_assert!(a_value == 0 && b_value == 0);
    if solver.watching {
        crate::clause::new_binary_clause(solver, a, b);
    } else {
        crate::clause::new_unwatched_binary_clause(solver, a, b);
        let pair = LitPair {
            lits: [a.min(b), a.max(b)],
        };
        closure.binaries.push(pair);
    }
}

// static void schedule_literal
fn schedule_literal(closure: &mut Closure, lit: u32) {
    let i = idx(lit) as usize;
    if closure.scheduled[i] {
        return;
    }
    closure.scheduled[i] = true;
    closure.schedule.push_back(lit); // ENQUEUE_FIFO
}

// static unsigned dequeue_next_scheduled_literal
fn dequeue_next_scheduled_literal(closure: &mut Closure) -> u32 {
    let res = closure.schedule.pop_front().unwrap(); // DEQUEUE_FIFO
    let i = idx(res) as usize;
    debug_assert!(closure.scheduled[i]);
    closure.scheduled[i] = false;
    res
}

// static bool merge_literals
fn merge_literals(solver: &mut Solver, closure: &mut Closure, lit: u32, other: u32) -> bool {
    debug_assert!(!solver.inconsistent);
    let repr_lit = find_repr(closure, lit);
    let repr_other = find_repr(closure, other);
    if repr_lit == repr_other {
        return false;
    }
    let lit_value = solver.values[lit as usize];
    let other_value = solver.values[other as usize];
    debug_assert!(lit_value == solver.values[repr_lit as usize]);
    debug_assert!(other_value == solver.values[repr_other as usize]);
    if lit_value != 0 {
        if lit_value == other_value {
            return false;
        }
        if lit_value == -other_value {
            solver.inconsistent = true;
            if solver.proof.is_some() {
                crate::proof::add_empty_to_proof(solver);
            }
            return false;
        }
        debug_assert!(other_value == 0);
        let unit = if lit_value < 0 { not(other) } else { other };
        let _ = learn_congruence_unit(solver, closure, unit);
        return false;
    }
    if lit_value == 0 && other_value != 0 {
        let unit = if other_value < 0 { not(lit) } else { lit };
        let _ = learn_congruence_unit(solver, closure, unit);
        return false;
    }
    let mut smaller = repr_lit;
    let mut larger = repr_other;
    if smaller > larger {
        std::mem::swap(&mut smaller, &mut larger);
    }
    debug_assert!(closure.repr[smaller as usize] == smaller);
    debug_assert!(closure.repr[larger as usize] > smaller);
    if repr_lit == not(repr_other) {
        crate::assign::learned_unit(solver, smaller);
        solver.inconsistent = true;
        if solver.proof.is_some() {
            crate::proof::add_empty_to_proof(solver);
        }
        return false;
    }
    let not_smaller = not(smaller);
    let not_larger = not(larger);
    closure.repr[larger as usize] = smaller;
    closure.repr[not_larger as usize] = not_smaller;
    add_binary_clause(solver, closure, not_larger, smaller);
    add_binary_clause(solver, closure, larger, not_smaller);
    schedule_literal(closure, larger);
    solver.statistics.congruent += 1;
    true
}

// static void connect_occurrence
fn connect_occurrence(closure: &mut Closure, lit: u32, g: GateId) {
    closure.occurrences[lit as usize].push(g);
}

// static gate *new_gate
fn new_gate(
    solver: &mut Solver,
    closure: &mut Closure,
    tag: u32,
    hash: u32,
    lhs: u32,
    lits: &[u32],
) -> GateId {
    let arity = lits.len() as u32;
    let g = closure.gates.len() as GateId;
    closure.gates.push(Gate {
        lhs,
        hash,
        tag,
        garbage: false,
        indexed: false,
        marked: false,
        shrunken: false,
        arity,
        rhs: lits.to_vec(),
    });
    for &l in lits {
        connect_occurrence(closure, l, g);
    }
    index_gate(solver, closure, g);
    solver.statistics.congruent_arity += arity as u64;
    solver.statistics.congruent_gates += 1;
    g
}

// static gate *find_and_lits — `lits` must already be detached from closure.
fn find_and_lits(
    solver: &mut Solver,
    closure: &mut Closure,
    lits: &mut [u32],
    except: GateId,
) -> (GateId, u32) {
    sort_lits(solver, lits);
    let hash = hash_lits(&closure.nonces, AND_GATE, lits);
    let g = find_gate(solver, closure, AND_GATE, hash, lits.len(), lits, except);
    if g != NO_GATE {
        solver.statistics.congruent_matched_ands += 1;
    }
    (g, hash)
}

// static gate *find_and_gate
fn find_and_gate(solver: &mut Solver, closure: &mut Closure, g: GateId) -> (GateId, u32) {
    let mut rhs = std::mem::take(&mut closure.gates[g as usize].rhs);
    let arity = closure.gates[g as usize].arity as usize;
    let res = find_and_lits(solver, closure, &mut rhs[..arity], g);
    closure.gates[g as usize].rhs = rhs;
    res
}

// static gate *new_and_gate
fn new_and_gate(solver: &mut Solver, closure: &mut Closure, lhs: u32) -> GateId {
    let mut rhs = std::mem::take(&mut closure.rhs);
    rhs.clear();
    for i in 0..closure.lits.len() {
        let l = closure.lits[i];
        if lhs != l {
            debug_assert!(lhs != not(l));
            rhs.push(not(l));
        }
    }
    let arity = rhs.len();
    debug_assert!(arity + 1 == closure.lits.len());
    let (g, hash) = find_and_lits(solver, closure, &mut rhs[..], NO_GATE);
    if g != NO_GATE {
        let g_lhs = closure.gates[g as usize].lhs;
        if merge_literals(solver, closure, g_lhs, lhs) {
            solver.statistics.congruent_ands += 1;
        }
        closure.rhs = rhs;
        return NO_GATE;
    }
    let g = new_gate(solver, closure, AND_GATE, hash, lhs, &rhs[..]);
    closure.rhs = rhs;
    solver.statistics.congruent_arity_ands += arity as u64;
    solver.statistics.congruent_gates_ands += 1;
    g
}

/*------------------------------------------------------------------------*/
// CHECKING_OR_PROVING proof-chain helpers (compiled in: proofs enabled).

// static void copy_literals
fn copy_literals(dst: &mut Vec<u32>, src: &[u32]) {
    dst.extend_from_slice(src);
    dst.push(INVALID); // INVALID_LIT terminator
}

// static void simplify_and_add_to_proof_chain
fn simplify_and_add_to_proof_chain(solver: &mut Solver, unsimplified: &[u32], chain: &mut Vec<u32>) {
    let mut clause = std::mem::take(&mut solver.clause);
    debug_assert!(clause.is_empty());
    let mut trivial = false;
    for &l in unsimplified {
        let lit_mark = solver.marks[l as usize];
        if lit_mark & 4 != 0 {
            continue;
        }
        let not_lit = not(l);
        let not_lit_mark = solver.marks[not_lit as usize];
        if not_lit_mark & 4 != 0 {
            trivial = true;
            break;
        }
        solver.marks[l as usize] = lit_mark | 4;
        clause.push(l);
    }
    for &l in &clause {
        let mark = solver.marks[l as usize];
        debug_assert!(mark & 4 != 0);
        solver.marks[l as usize] = mark & !4;
    }
    if !trivial {
        // CHECK_AND_ADD_STACK: no-op.  ADD_STACK_TO_PROOF:
        if solver.proof.is_some() {
            crate::proof::add_lits_to_proof(solver, &clause);
        }
        copy_literals(chain, &clause);
    }
    clause.clear();
    solver.clause = clause;
}

// static void add_xor_matching_proof_chain
fn add_xor_matching_proof_chain(
    solver: &mut Solver,
    closure: &mut Closure,
    g: GateId,
    lhs1: u32,
    lhs2: u32,
) {
    if lhs1 == lhs2 {
        return;
    }
    if solver.proof.is_none() {
        // kissat_checking_or_proving == proving in this build.
        return;
    }
    let mut unsimplified = std::mem::take(&mut closure.unsimplified);
    let mut chain = std::mem::take(&mut closure.chain);
    debug_assert!(unsimplified.is_empty());
    debug_assert!(chain.is_empty());
    debug_assert!(closure.gates[g as usize].arity > 1);
    let reduced_arity = (closure.gates[g as usize].arity - 1) as usize;
    for i in 0..reduced_arity {
        let l = closure.gates[g as usize].rhs[i];
        unsimplified.push(l);
    }
    let not_lhs1 = not(lhs1);
    let not_lhs2 = not(lhs2);
    loop {
        let size = unsimplified.len();
        debug_assert!(size < 32);
        for _ in 0..1u32 << size {
            unsimplified.push(not_lhs1);
            unsimplified.push(lhs2);
            simplify_and_add_to_proof_chain(solver, &unsimplified, &mut chain);
            unsimplified.truncate(unsimplified.len() - 2);
            unsimplified.push(lhs1);
            unsimplified.push(not_lhs2);
            simplify_and_add_to_proof_chain(solver, &unsimplified, &mut chain);
            unsimplified.truncate(unsimplified.len() - 2);
            inc_lits(&mut unsimplified);
        }
        debug_assert!(!unsimplified.is_empty());
        unsimplified.pop();
        if unsimplified.is_empty() {
            break;
        }
    }
    closure.unsimplified = unsimplified;
    closure.chain = chain;
}

// static void delete_proof_chain
fn delete_proof_chain(solver: &mut Solver, closure: &mut Closure) {
    if solver.proof.is_none() {
        debug_assert!(closure.chain.is_empty());
        return;
    }
    if closure.chain.is_empty() {
        return;
    }
    let chain = std::mem::take(&mut closure.chain);
    let mut clause = std::mem::take(&mut solver.clause);
    debug_assert!(clause.is_empty());
    let mut start = 0usize;
    let mut p = 0usize;
    while p != chain.len() {
        let l = chain[p];
        if l == INVALID {
            while start != p {
                clause.push(chain[start]);
                start += 1;
            }
            // REMOVE_CHECKER_STACK: no-op.  DELETE_STACK_FROM_PROOF:
            if solver.proof.is_some() {
                crate::proof::delete_internal_from_proof(solver, &clause);
            }
            clause.clear();
            start += 1;
        }
        p += 1;
    }
    debug_assert!(clause.is_empty());
    debug_assert!(start == chain.len());
    solver.clause = clause;
    closure.chain = chain;
    closure.chain.clear();
}

/*------------------------------------------------------------------------*/

// static gate *find_xor_lits
fn find_xor_lits(
    solver: &mut Solver,
    closure: &mut Closure,
    lits: &mut [u32],
    except: GateId,
) -> (GateId, u32) {
    sort_lits(solver, lits);
    let hash = hash_lits(&closure.nonces, XOR_GATE, lits);
    let g = find_gate(solver, closure, XOR_GATE, hash, lits.len(), lits, except);
    if g != NO_GATE {
        solver.statistics.congruent_matched_xors += 1;
    }
    (g, hash)
}

// static gate *find_xor_gate
fn find_xor_gate(solver: &mut Solver, closure: &mut Closure, g: GateId) -> (GateId, u32) {
    let mut rhs = std::mem::take(&mut closure.gates[g as usize].rhs);
    let arity = closure.gates[g as usize].arity as usize;
    let res = find_xor_lits(solver, closure, &mut rhs[..arity], g);
    closure.gates[g as usize].rhs = rhs;
    res
}

// static gate *new_xor_gate
fn new_xor_gate(solver: &mut Solver, closure: &mut Closure, lhs: u32) -> GateId {
    let mut rhs = std::mem::take(&mut closure.rhs);
    rhs.clear();
    let not_lhs = not(lhs);
    for i in 0..closure.lits.len() {
        let l = closure.lits[i];
        if l != lhs && l != not_lhs {
            debug_assert!(negated(l) == 0);
            rhs.push(l);
        }
    }
    let arity = rhs.len();
    debug_assert!(arity + 1 == closure.lits.len());
    let (g, hash) = find_xor_lits(solver, closure, &mut rhs[..], NO_GATE);
    if g != NO_GATE {
        closure.rhs = rhs;
        let g_lhs = closure.gates[g as usize].lhs;
        add_xor_matching_proof_chain(solver, closure, g, g_lhs, lhs);
        if merge_literals(solver, closure, g_lhs, lhs) {
            solver.statistics.congruent_xors += 1;
        }
        if !solver.inconsistent {
            delete_proof_chain(solver, closure);
        }
        return NO_GATE;
    }
    let g = new_gate(solver, closure, XOR_GATE, hash, lhs, &rhs[..]);
    closure.rhs = rhs;
    solver.statistics.congruent_arity_xors += arity as u64;
    solver.statistics.congruent_gates_xors += 1;
    g
}

// static void add_ite_matching_proof_chain (CHECKING_OR_PROVING)
fn add_ite_matching_proof_chain(
    solver: &mut Solver,
    closure: &mut Closure,
    g: GateId,
    lhs1: u32,
    lhs2: u32,
) {
    if lhs1 == lhs2 {
        return;
    }
    if solver.proof.is_none() {
        return;
    }
    let mut unsimplified = std::mem::take(&mut closure.unsimplified);
    let mut chain = std::mem::take(&mut closure.chain);
    debug_assert!(chain.is_empty());
    let cond = closure.gates[g as usize].rhs[0];
    let not_cond = not(cond);
    let not_lhs1 = not(lhs1);
    let not_lhs2 = not(lhs2);
    unsimplified.push(lhs1);
    unsimplified.push(not_lhs2);
    unsimplified.push(cond);
    simplify_and_add_to_proof_chain(solver, &unsimplified, &mut chain);
    unsimplified.pop();
    unsimplified.push(not_cond);
    simplify_and_add_to_proof_chain(solver, &unsimplified, &mut chain);
    unsimplified.pop();
    // CHECK_AND_ADD_STACK (*unsimplified): no-op.  ADD_STACK_TO_PROOF:
    if solver.proof.is_some() {
        crate::proof::add_lits_to_proof(solver, &unsimplified);
    }
    copy_literals(&mut chain, &unsimplified);
    unsimplified.clear();
    unsimplified.push(not_lhs1);
    unsimplified.push(lhs2);
    unsimplified.push(cond);
    simplify_and_add_to_proof_chain(solver, &unsimplified, &mut chain);
    unsimplified.pop();
    unsimplified.push(not_cond);
    simplify_and_add_to_proof_chain(solver, &unsimplified, &mut chain);
    unsimplified.pop();
    simplify_and_add_to_proof_chain(solver, &unsimplified, &mut chain);
    unsimplified.clear();
    closure.unsimplified = unsimplified;
    closure.chain = chain;
}

// static void add_ite_turned_and_binary_clauses (CHECKING_OR_PROVING)
fn add_ite_turned_and_binary_clauses(solver: &mut Solver, closure: &mut Closure, g: GateId) {
    if solver.proof.is_none() {
        return;
    }
    let mut unsimplified = std::mem::take(&mut closure.unsimplified);
    let mut chain = std::mem::take(&mut closure.chain);
    debug_assert!(unsimplified.is_empty());
    debug_assert!(chain.is_empty());
    let not_lhs = not(closure.gates[g as usize].lhs);
    let rhs0 = closure.gates[g as usize].rhs[0];
    let rhs1 = closure.gates[g as usize].rhs[1];
    unsimplified.push(not_lhs);
    unsimplified.push(rhs0);
    simplify_and_add_to_proof_chain(solver, &unsimplified, &mut chain);
    unsimplified.pop();
    unsimplified.push(rhs1);
    simplify_and_add_to_proof_chain(solver, &unsimplified, &mut chain);
    unsimplified.clear();
    closure.unsimplified = unsimplified;
    closure.chain = chain;
}

// static bool normalize_ite_lits
fn normalize_ite_lits(lits: &mut [u32]) -> bool {
    if negated(lits[0]) != 0 {
        lits[0] = not(lits[0]);
        lits.swap(1, 2);
    }
    if negated(lits[1]) == 0 {
        return false;
    }
    lits[1] = not(lits[1]);
    lits[2] = not(lits[2]);
    true
}

// static gate *find_ite_lits
fn find_ite_lits(
    solver: &mut Solver,
    closure: &mut Closure,
    lits: &mut [u32],
    except: GateId,
) -> (GateId, u32, bool) {
    debug_assert!(lits.len() == 3);
    let negate_lhs = normalize_ite_lits(lits);
    let hash = hash_lits(&closure.nonces, ITE_GATE, lits);
    let g = find_gate(solver, closure, ITE_GATE, hash, lits.len(), lits, except);
    if g != NO_GATE {
        solver.statistics.congruent_matched_ites += 1;
    }
    (g, hash, negate_lhs)
}

// static gate *find_ite_gate
fn find_ite_gate(solver: &mut Solver, closure: &mut Closure, g: GateId) -> (GateId, u32, bool) {
    let mut rhs = std::mem::take(&mut closure.gates[g as usize].rhs);
    let arity = closure.gates[g as usize].arity as usize;
    let res = find_ite_lits(solver, closure, &mut rhs[..arity], g);
    closure.gates[g as usize].rhs = rhs;
    res
}

// static gate *new_ite_gate
fn new_ite_gate(
    solver: &mut Solver,
    closure: &mut Closure,
    mut lhs: u32,
    cond: u32,
    then_lit: u32,
    else_lit: u32,
) -> GateId {
    let not_then_lit = not(then_lit);
    if else_lit == not_then_lit {
        return NO_GATE; // skipping ternary XOR gate
    }
    if else_lit == then_lit {
        if merge_literals(solver, closure, lhs, then_lit) {
            solver.statistics.congruent_trivial_ite += 1;
        }
        return NO_GATE;
    }
    let mut rhs = std::mem::take(&mut closure.rhs);
    rhs.clear();
    rhs.push(cond);
    rhs.push(then_lit);
    rhs.push(else_lit);
    debug_assert!(rhs.len() == 3);
    let (g, hash, negate_lhs) = find_ite_lits(solver, closure, &mut rhs[..], NO_GATE);
    if g != NO_GATE {
        closure.rhs = rhs;
        if negate_lhs {
            lhs = not(lhs);
        }
        let g_lhs = closure.gates[g as usize].lhs;
        add_ite_matching_proof_chain(solver, closure, g, g_lhs, lhs);
        if merge_literals(solver, closure, g_lhs, lhs) {
            solver.statistics.congruent_ites += 1;
        }
        if !solver.inconsistent {
            delete_proof_chain(solver, closure);
        }
        return NO_GATE;
    }
    if negate_lhs {
        lhs = not(lhs);
    }
    let g = new_gate(solver, closure, ITE_GATE, hash, lhs, &rhs[..]);
    closure.rhs = rhs;
    solver.statistics.congruent_gates_ites += 1;
    g
}

// static void mark_gate_as_garbage
fn mark_gate_as_garbage(closure: &mut Closure, g: GateId) {
    debug_assert!(!closure.gates[g as usize].garbage);
    closure.gates[g as usize].garbage = true;
    closure.garbage.push(g);
}

// static void shrink_gate — `new_arity` is C's (new_end_rhs - rhs).
fn shrink_gate(closure: &mut Closure, g: GateId, new_arity: u32) {
    let gate = &mut closure.gates[g as usize];
    let old_arity = gate.arity;
    debug_assert!(new_arity <= old_arity);
    if new_arity == old_arity {
        return;
    }
    if !gate.shrunken {
        debug_assert!(gate.rhs[old_arity as usize - 1] != INVALID);
        gate.rhs[old_arity as usize - 1] = INVALID;
        gate.shrunken = true;
    }
    gate.arity = new_arity;
}

// static bool skip_and_gate
fn skip_and_gate(solver: &mut Solver, closure: &mut Closure, g: GateId) -> bool {
    debug_assert!(closure.gates[g as usize].tag == AND_GATE);
    if closure.gates[g as usize].garbage {
        return true;
    }
    let lhs = closure.gates[g as usize].lhs;
    let value_lhs = solver.values[lhs as usize];
    if value_lhs > 0 {
        mark_gate_as_garbage(closure, g);
        return true;
    }
    debug_assert!(closure.gates[g as usize].arity > 1);
    false
}

// static bool gate_contains
fn gate_contains(gate: &Gate, lit: u32) -> bool {
    gate.rhs[..gate.arity as usize].contains(&lit)
}

// static bool rewriting_lhs
fn rewriting_lhs(closure: &mut Closure, g: GateId, dst: u32) -> bool {
    let lhs = closure.gates[g as usize].lhs;
    if dst != lhs && dst != not(lhs) {
        return false;
    }
    mark_gate_as_garbage(closure, g);
    true
}

// static void shrink_and_gate
fn shrink_and_gate(closure: &mut Closure, g: GateId, mut q: u32, falsifies: u32, clashing: u32) {
    debug_assert!(closure.gates[g as usize].tag == AND_GATE);
    if falsifies != INVALID {
        closure.gates[g as usize].rhs[0] = falsifies;
        q = 1;
    } else if clashing != INVALID {
        closure.gates[g as usize].rhs[0] = clashing;
        closure.gates[g as usize].rhs[1] = not(clashing);
        q = 2;
    }
    shrink_gate(closure, g, q);
}

// static void update_and_gate
fn update_and_gate(
    solver: &mut Solver,
    closure: &mut Closure,
    g: GateId,
    falsifies: u32,
    clashing: u32,
) {
    debug_assert!(closure.gates[g as usize].tag == AND_GATE);
    let mut garbage = true;
    if falsifies != INVALID || clashing != INVALID {
        let not_lhs = not(closure.gates[g as usize].lhs);
        let _ = learn_congruence_unit(solver, closure, not_lhs);
    } else if closure.gates[g as usize].arity == 1 {
        let lhs = closure.gates[g as usize].lhs;
        let rhs0 = closure.gates[g as usize].rhs[0];
        let value_lhs = solver.values[lhs as usize];
        if value_lhs > 0 {
            let _ = learn_congruence_unit(solver, closure, rhs0);
        } else if value_lhs < 0 {
            let _ = learn_congruence_unit(solver, closure, not(rhs0));
        } else if merge_literals(solver, closure, lhs, rhs0) {
            solver.statistics.congruent_unary_ands += 1;
            solver.statistics.congruent_unary += 1;
        }
    } else {
        let (h, hash) = find_and_gate(solver, closure, g);
        if h != NO_GATE {
            let g_lhs = closure.gates[g as usize].lhs;
            let h_lhs = closure.gates[h as usize].lhs;
            if merge_literals(solver, closure, g_lhs, h_lhs) {
                solver.statistics.congruent_ands += 1;
            }
        } else {
            remove_gate(solver, closure, g);
            closure.gates[g as usize].hash = hash;
            index_gate(solver, closure, g);
            garbage = false;
        }
    }
    if garbage && !solver.inconsistent {
        mark_gate_as_garbage(closure, g);
    }
}

// static void simplify_and_gate
fn simplify_and_gate(solver: &mut Solver, closure: &mut Closure, g: GateId) {
    if skip_and_gate(solver, closure, g) {
        return;
    }
    let mut falsifies = INVALID;
    let q;
    {
        let old_arity = closure.gates[g as usize].arity as usize;
        let mut qi = 0usize;
        for pi in 0..old_arity {
            let l = closure.gates[g as usize].rhs[pi];
            let value = solver.values[l as usize];
            if value > 0 {
                continue;
            }
            if value < 0 {
                falsifies = l;
                continue;
            }
            closure.gates[g as usize].rhs[qi] = l;
            qi += 1;
        }
        q = qi as u32;
    }
    shrink_and_gate(closure, g, q, falsifies, INVALID);
    update_and_gate(solver, closure, g, falsifies, INVALID);
    solver.statistics.congruent_simplified += 1;
    solver.statistics.congruent_simplified_ands += 1;
}

// static void rewrite_and_gate
fn rewrite_and_gate(solver: &mut Solver, closure: &mut Closure, g: GateId, dst: u32, src: u32) {
    if skip_and_gate(solver, closure, g) {
        return;
    }
    if !gate_contains(&closure.gates[g as usize], src) {
        return;
    }
    debug_assert!(src != INVALID);
    debug_assert!(dst != INVALID);
    debug_assert!(solver.values[src as usize] == solver.values[dst as usize]);
    let mut falsifies = INVALID;
    let mut clashing = INVALID;
    let not_dst = not(dst);
    let mut dst_count = 0u32;
    let mut not_dst_count = 0u32;
    let q;
    {
        let not_lhs = not(closure.gates[g as usize].lhs);
        let old_arity = closure.gates[g as usize].arity as usize;
        let mut qi = 0usize;
        for pi in 0..old_arity {
            let mut l = closure.gates[g as usize].rhs[pi];
            if l == src {
                l = dst;
            }
            if l == not_lhs {
                clashing = l;
                break;
            }
            let value = solver.values[l as usize];
            if value > 0 {
                continue;
            }
            if value < 0 {
                falsifies = l;
                break;
            }
            if l == dst {
                if not_dst_count != 0 {
                    clashing = not_dst;
                    break;
                }
                let prev = dst_count;
                dst_count += 1;
                if prev != 0 {
                    continue;
                }
            }
            if l == not_dst {
                if dst_count != 0 {
                    debug_assert!(not_dst_count == 0);
                    clashing = dst;
                    break;
                }
                debug_assert!(not_dst_count == 0);
                not_dst_count += 1;
            }
            closure.gates[g as usize].rhs[qi] = l;
            qi += 1;
        }
        q = qi as u32;
    }
    debug_assert!(dst_count <= 2);
    debug_assert!(not_dst_count <= 1);
    shrink_and_gate(closure, g, q, falsifies, clashing);
    update_and_gate(solver, closure, g, falsifies, clashing);
    solver.statistics.congruent_rewritten += 1;
    solver.statistics.congruent_rewritten_ands += 1;
}

// static bool skip_xor_gate
fn skip_xor_gate(closure: &Closure, g: GateId) -> bool {
    debug_assert!(closure.gates[g as usize].tag == XOR_GATE);
    if closure.gates[g as usize].garbage {
        return true;
    }
    debug_assert!(closure.gates[g as usize].arity > 1);
    false
}

// static void add_xor_shrinking_proof_chain (CHECKING_OR_PROVING)
fn add_xor_shrinking_proof_chain(solver: &mut Solver, closure: &Closure, g: GateId, pivot: u32) {
    if solver.proof.is_none() {
        return;
    }
    let mut clause = std::mem::take(&mut solver.clause);
    debug_assert!(clause.is_empty());
    let arity = closure.gates[g as usize].arity as usize;
    for i in 0..arity {
        clause.push(closure.gates[g as usize].rhs[i]);
    }
    let lhs = closure.gates[g as usize].lhs;
    let not_lhs = not(lhs);
    clause.push(not_lhs);
    let parity = negated(not_lhs);
    debug_assert!(parity == parity_lits(&clause));
    let not_pivot = not(pivot);
    let size = clause.len();
    debug_assert!(size < 32);
    let end = 1u32 << size;
    for i in 0..end {
        while i != 0 && parity != parity_lits(&clause) {
            inc_lits(&mut clause);
        }
        clause.push(pivot);
        if solver.proof.is_some() {
            crate::proof::add_lits_to_proof(solver, &clause);
        }
        clause.pop();
        clause.push(not_pivot);
        if solver.proof.is_some() {
            crate::proof::add_lits_to_proof(solver, &clause);
        }
        clause.pop();
        if solver.proof.is_some() {
            crate::proof::add_lits_to_proof(solver, &clause);
        }
        clause.push(pivot);
        if solver.proof.is_some() {
            crate::proof::delete_internal_from_proof(solver, &clause);
        }
        clause.pop();
        clause.push(not_pivot);
        if solver.proof.is_some() {
            crate::proof::delete_internal_from_proof(solver, &clause);
        }
        clause.pop();
        inc_lits(&mut clause);
    }
    clause.clear();
    solver.clause = clause;
}

// static void shrink_xor_gate
fn shrink_xor_gate(closure: &mut Closure, g: GateId, q: u32) {
    debug_assert!(closure.gates[g as usize].tag == XOR_GATE);
    shrink_gate(closure, g, q);
}

// static void update_xor_gate
fn update_xor_gate(solver: &mut Solver, closure: &mut Closure, g: GateId) {
    debug_assert!(closure.gates[g as usize].tag == XOR_GATE);
    let mut garbage = true;
    let arity = closure.gates[g as usize].arity;
    if arity == 0 {
        let not_lhs = not(closure.gates[g as usize].lhs);
        let _ = learn_congruence_unit(solver, closure, not_lhs);
    } else if arity == 1 {
        let lhs = closure.gates[g as usize].lhs;
        let rhs0 = closure.gates[g as usize].rhs[0];
        let value_lhs = solver.values[lhs as usize];
        if value_lhs > 0 {
            let _ = learn_congruence_unit(solver, closure, rhs0);
        } else if value_lhs < 0 {
            let _ = learn_congruence_unit(solver, closure, not(rhs0));
        } else if merge_literals(solver, closure, lhs, rhs0) {
            solver.statistics.congruent_unary_xors += 1;
            solver.statistics.congruent_unary += 1;
        }
    } else {
        debug_assert!(arity > 1);
        let (h, hash) = find_xor_gate(solver, closure, g);
        if h != NO_GATE {
            let g_lhs = closure.gates[g as usize].lhs;
            let h_lhs = closure.gates[h as usize].lhs;
            add_xor_matching_proof_chain(solver, closure, g, g_lhs, h_lhs);
            if merge_literals(solver, closure, g_lhs, h_lhs) {
                solver.statistics.congruent_xors += 1;
            }
            if !solver.inconsistent {
                delete_proof_chain(solver, closure);
            }
        } else {
            remove_gate(solver, closure, g);
            closure.gates[g as usize].hash = hash;
            index_gate(solver, closure, g);
            garbage = false;
        }
    }
    if garbage && !solver.inconsistent {
        mark_gate_as_garbage(closure, g);
    }
}

// static void simplify_xor_gate
fn simplify_xor_gate(solver: &mut Solver, closure: &mut Closure, g: GateId) {
    if skip_xor_gate(closure, g) {
        return;
    }
    let mut negate = 0u32;
    let q;
    {
        let old_arity = closure.gates[g as usize].arity as usize;
        let mut qi = 0usize;
        for pi in 0..old_arity {
            let l = closure.gates[g as usize].rhs[pi];
            debug_assert!(negated(l) == 0);
            let value = solver.values[l as usize];
            if value > 0 {
                negate ^= 1;
            }
            if value == 0 {
                closure.gates[g as usize].rhs[qi] = l;
                qi += 1;
            }
        }
        q = qi as u32;
    }
    if negate != 0 {
        let lhs = closure.gates[g as usize].lhs;
        closure.gates[g as usize].lhs = not(lhs);
    }
    shrink_xor_gate(closure, g, q);
    update_xor_gate(solver, closure, g);
    solver.statistics.congruent_simplified += 1;
    solver.statistics.congruent_simplified_xors += 1;
}

// static void rewrite_xor_gate
fn rewrite_xor_gate(solver: &mut Solver, closure: &mut Closure, g: GateId, dst: u32, src: u32) {
    if skip_xor_gate(closure, g) {
        return;
    }
    if rewriting_lhs(closure, g, dst) {
        return;
    }
    if !gate_contains(&closure.gates[g as usize], src) {
        return;
    }
    let original_dst_negated = negated(dst);
    let mut negate = original_dst_negated;
    let mut dst_count = 0u32;
    let dst = strip(dst);
    let q;
    {
        let old_arity = closure.gates[g as usize].arity as usize;
        let mut qi = 0usize;
        for pi in 0..old_arity {
            let mut l = closure.gates[g as usize].rhs[pi];
            debug_assert!(negated(l) == 0);
            if l == src {
                l = dst;
            }
            let value = solver.values[l as usize];
            if value > 0 {
                negate ^= 1;
            }
            if value != 0 {
                continue;
            }
            if l == dst {
                dst_count += 1;
            }
            closure.gates[g as usize].rhs[qi] = l;
            qi += 1;
        }
        q = qi as u32;
    }
    if negate != 0 {
        let lhs = closure.gates[g as usize].lhs;
        closure.gates[g as usize].lhs = not(lhs);
    }
    debug_assert!(dst_count <= 2);
    let mut q = q;
    if dst_count == 2 {
        let end_of_rhs = q as usize;
        let mut qi = 0usize;
        for pi in 0..end_of_rhs {
            let l = closure.gates[g as usize].rhs[pi];
            if l != dst {
                closure.gates[g as usize].rhs[qi] = l;
                qi += 1;
            }
        }
        debug_assert!(qi + 2 == end_of_rhs);
        q = qi as u32;
    }
    shrink_xor_gate(closure, g, q);
    if dst_count > 1 {
        add_xor_shrinking_proof_chain(solver, closure, g, src);
    }
    update_xor_gate(solver, closure, g);
    if !closure.gates[g as usize].garbage
        && !solver.inconsistent
        && original_dst_negated != 0
        && dst_count == 1
    {
        debug_assert!(negated(dst) == 0);
        connect_occurrence(closure, dst, g);
    }
    solver.statistics.congruent_rewritten += 1;
    solver.statistics.congruent_rewritten_xors += 1;
}

// static bool skip_ite_gate
fn skip_ite_gate(closure: &Closure, g: GateId) -> bool {
    debug_assert!(closure.gates[g as usize].tag == ITE_GATE);
    closure.gates[g as usize].garbage
}

// static void simplify_ite_gate
fn simplify_ite_gate(solver: &mut Solver, closure: &mut Closure, g: GateId) {
    if skip_ite_gate(closure, g) {
        return;
    }
    debug_assert!(closure.gates[g as usize].arity == 3);
    let mut garbage = true;
    let lhs = closure.gates[g as usize].lhs;
    let cond = closure.gates[g as usize].rhs[0];
    let then_lit = closure.gates[g as usize].rhs[1];
    let else_lit = closure.gates[g as usize].rhs[2];
    let cond_value = solver.values[cond as usize];
    if cond_value > 0 {
        if merge_literals(solver, closure, lhs, then_lit) {
            solver.statistics.congruent_unary_ites += 1;
            solver.statistics.congruent_unary += 1;
        }
    } else if cond_value < 0 {
        if merge_literals(solver, closure, lhs, else_lit) {
            solver.statistics.congruent_unary_ites += 1;
            solver.statistics.congruent_unary += 1;
        }
    } else {
        let then_value = solver.values[then_lit as usize];
        let else_value = solver.values[else_lit as usize];
        let not_lhs = not(lhs);
        debug_assert!(then_value != 0 || else_value != 0);
        if then_value > 0 && else_value > 0 {
            let _ = learn_congruence_unit(solver, closure, lhs);
        } else if then_value < 0 && else_value < 0 {
            let _ = learn_congruence_unit(solver, closure, not_lhs);
        } else if then_value > 0 && else_value < 0 {
            if merge_literals(solver, closure, lhs, cond) {
                solver.statistics.congruent_unary_ites += 1;
                solver.statistics.congruent_unary += 1;
            }
        } else if then_value < 0 && else_value > 0 {
            let not_cond = not(cond);
            if merge_literals(solver, closure, lhs, not_cond) {
                solver.statistics.congruent_unary_ites += 1;
                solver.statistics.congruent_unary += 1;
            }
        } else {
            {
                let gate = &mut closure.gates[g as usize];
                if then_value > 0 {
                    debug_assert!(else_value == 0);
                    gate.lhs = not_lhs;
                    gate.rhs[0] = not(cond);
                    gate.rhs[1] = not(else_lit);
                } else if then_value < 0 {
                    debug_assert!(else_value == 0);
                    gate.rhs[0] = not(cond);
                    gate.rhs[1] = else_lit;
                } else if else_value > 0 {
                    debug_assert!(then_value == 0);
                    gate.lhs = not_lhs;
                    gate.rhs[0] = not(then_lit);
                    gate.rhs[1] = cond;
                } else {
                    debug_assert!(else_value < 0);
                    debug_assert!(then_value == 0);
                    gate.rhs[0] = cond;
                    gate.rhs[1] = then_lit;
                }
                if gate.rhs[0] > gate.rhs[1] {
                    gate.rhs.swap(0, 1);
                }
                debug_assert!(!gate.shrunken);
                gate.shrunken = true;
                gate.rhs[2] = INVALID;
                gate.arity = 2;
                gate.tag = AND_GATE;
                debug_assert!(gate.rhs[0] < gate.rhs[1]);
                debug_assert!(gate.rhs[0] != not(gate.rhs[1]));
            }
            let (h, hash) = find_and_gate(solver, closure, g);
            if h != NO_GATE {
                debug_assert!(garbage);
                let g_lhs = closure.gates[g as usize].lhs;
                let h_lhs = closure.gates[h as usize].lhs;
                if merge_literals(solver, closure, g_lhs, h_lhs) {
                    solver.statistics.congruent_ands += 1;
                }
            } else {
                remove_gate(solver, closure, g);
                closure.gates[g as usize].hash = hash;
                index_gate(solver, closure, g);
                garbage = false;
                for i in 0..closure.gates[g as usize].arity as usize {
                    let l = closure.gates[g as usize].rhs[i];
                    if l != cond && l != then_lit && l != else_lit {
                        connect_occurrence(closure, l, g);
                    }
                }
            }
        }
    }
    if garbage && !solver.inconsistent {
        mark_gate_as_garbage(closure, g);
    }
    solver.statistics.congruent_simplified += 1;
    solver.statistics.congruent_simplified_ites += 1;
}

// static void rewrite_ite_gate
fn rewrite_ite_gate(solver: &mut Solver, closure: &mut Closure, g: GateId, dst: u32, src: u32) {
    if skip_ite_gate(closure, g) {
        return;
    }
    if !gate_contains(&closure.gates[g as usize], src) {
        return;
    }
    debug_assert!(closure.gates[g as usize].arity == 3);
    let lhs = closure.gates[g as usize].lhs;
    let cond = closure.gates[g as usize].rhs[0];
    let then_lit = closure.gates[g as usize].rhs[1];
    let else_lit = closure.gates[g as usize].rhs[2];
    let not_lhs = not(lhs);
    let not_dst = not(dst);
    let not_cond = not(cond);
    let not_then_lit = not(then_lit);
    let not_else_lit = not(else_lit);
    let mut new_tag = AND_GATE;
    let mut garbage = false;
    let mut shrink = true;
    if src == cond {
        if dst == then_lit {
            // then_lit ? then_lit : else_lit == !(!then_lit & !else_lit)
            let gate = &mut closure.gates[g as usize];
            gate.lhs = not_lhs;
            gate.rhs[0] = not_then_lit;
            gate.rhs[1] = not_else_lit;
        } else if not_dst == then_lit {
            // !then_lit ? then_lit : else_lit == then_lit & else_lit
            let gate = &mut closure.gates[g as usize];
            gate.rhs[0] = else_lit;
            debug_assert!(gate.rhs[1] == then_lit);
        } else if dst == else_lit {
            // else_lit ? then_lit : else_lit == else_lit & then_lit
            let gate = &mut closure.gates[g as usize];
            gate.rhs[0] = else_lit;
            debug_assert!(gate.rhs[1] == then_lit);
        } else if not_dst == else_lit {
            // !else_lit ? then_lit : else_lit == !(!then_lit & !else_lit)
            let gate = &mut closure.gates[g as usize];
            gate.lhs = not_lhs;
            gate.rhs[0] = not_then_lit;
            gate.rhs[1] = not_else_lit;
        } else {
            shrink = false;
            closure.gates[g as usize].rhs[0] = dst;
        }
    } else if src == then_lit {
        if dst == cond {
            // cond ? cond : else_lit == !(!cond & !else_lit)
            let gate = &mut closure.gates[g as usize];
            gate.lhs = not_lhs;
            gate.rhs[0] = not_cond;
            gate.rhs[1] = not_else_lit;
        } else if not_dst == cond {
            // cond ? !cond : else_lit == !cond & else_lit
            let gate = &mut closure.gates[g as usize];
            gate.rhs[0] = not_cond;
            gate.rhs[1] = else_lit;
        } else if dst == else_lit {
            // cond ? else_lit : else_lit == else_lit
            if merge_literals(solver, closure, lhs, else_lit) {
                solver.statistics.congruent_unary_ites += 1;
                solver.statistics.congruent_unary += 1;
            }
            garbage = true;
        } else if not_dst == else_lit {
            // cond ? !else_lit : else_lit == cond ^ else_lit
            new_tag = XOR_GATE;
            let gate = &mut closure.gates[g as usize];
            debug_assert!(gate.rhs[0] == cond);
            gate.rhs[1] = else_lit;
        } else {
            shrink = false;
            closure.gates[g as usize].rhs[1] = dst;
        }
    } else {
        debug_assert!(src == else_lit);
        if dst == cond {
            // cond ? then_lit : cond == cond & then_lit
            let gate = &closure.gates[g as usize];
            debug_assert!(gate.rhs[0] == cond);
            debug_assert!(gate.rhs[1] == then_lit);
        } else if not_dst == cond {
            // cond ? then_lit : !cond == !(!then_lit & cond)
            let gate = &mut closure.gates[g as usize];
            gate.lhs = not_lhs;
            debug_assert!(gate.rhs[0] == cond);
            gate.rhs[1] = not_then_lit;
        } else if dst == then_lit {
            // cond ? then_lit : then_lit == then_lit
            if merge_literals(solver, closure, lhs, then_lit) {
                solver.statistics.congruent_unary_ites += 1;
                solver.statistics.congruent_unary += 1;
            }
            garbage = true;
        } else if not_dst == then_lit {
            // cond ? then_lit : !then_lit == !(cond ^ then_lit)
            new_tag = XOR_GATE;
            let gate = &mut closure.gates[g as usize];
            gate.lhs = not_lhs;
            debug_assert!(gate.rhs[0] == cond);
            debug_assert!(gate.rhs[1] == then_lit);
        } else {
            shrink = false;
            closure.gates[g as usize].rhs[2] = dst;
        }
    }
    if !garbage {
        if shrink {
            {
                let gate = &mut closure.gates[g as usize];
                if gate.rhs[0] > gate.rhs[1] {
                    gate.rhs.swap(0, 1);
                }
                if new_tag == XOR_GATE {
                    let mut negate_lhs = false;
                    if negated(gate.rhs[0]) != 0 {
                        gate.rhs[0] = not(gate.rhs[0]);
                        negate_lhs = !negate_lhs;
                    }
                    if negated(gate.rhs[1]) != 0 {
                        gate.rhs[1] = not(gate.rhs[1]);
                        negate_lhs = !negate_lhs;
                    }
                    if negate_lhs {
                        gate.lhs = not(gate.lhs);
                    }
                }
                debug_assert!(!gate.shrunken);
                gate.shrunken = true;
                gate.rhs[2] = INVALID;
                gate.arity = 2;
                gate.tag = new_tag;
                debug_assert!(gate.rhs[0] < gate.rhs[1]);
                debug_assert!(gate.rhs[0] != not(gate.rhs[1]));
            }
            let (h, hash) = if new_tag == AND_GATE {
                find_and_gate(solver, closure, g)
            } else {
                debug_assert!(new_tag == XOR_GATE);
                find_xor_gate(solver, closure, g)
            };
            if h != NO_GATE {
                garbage = true;
                let g_lhs = closure.gates[g as usize].lhs;
                let h_lhs = closure.gates[h as usize].lhs;
                if new_tag == XOR_GATE {
                    add_xor_matching_proof_chain(solver, closure, g, g_lhs, h_lhs);
                } else {
                    add_ite_turned_and_binary_clauses(solver, closure, g);
                }
                if merge_literals(solver, closure, g_lhs, h_lhs) {
                    solver.statistics.congruent_ands += 1;
                }
                if !solver.inconsistent {
                    delete_proof_chain(solver, closure);
                }
            } else {
                garbage = false;
                remove_gate(solver, closure, g);
                closure.gates[g as usize].hash = hash;
                index_gate(solver, closure, g);
                debug_assert!(closure.gates[g as usize].arity == 2);
                for i in 0..closure.gates[g as usize].arity as usize {
                    let l = closure.gates[g as usize].rhs[i];
                    if l != dst && l != cond && l != then_lit && l != else_lit {
                        connect_occurrence(closure, l, g);
                    }
                }
                if closure.gates[g as usize].tag == AND_GATE {
                    let g_not_lhs = not(closure.gates[g as usize].lhs);
                    for i in 0..closure.gates[g as usize].arity as usize {
                        let l = closure.gates[g as usize].rhs[i];
                        add_binary_clause(solver, closure, g_not_lhs, l);
                    }
                }
            }
        } else {
            let (h, hash, negate_lhs) = find_ite_gate(solver, closure, g);
            debug_assert!(lhs == closure.gates[g as usize].lhs);
            debug_assert!(not_lhs == not(closure.gates[g as usize].lhs));
            if h != NO_GATE {
                garbage = true;
                let normalized_lhs = if negate_lhs { not_lhs } else { lhs };
                let h_lhs = closure.gates[h as usize].lhs;
                add_ite_matching_proof_chain(solver, closure, h, h_lhs, normalized_lhs);
                if merge_literals(solver, closure, h_lhs, normalized_lhs) {
                    solver.statistics.congruent_ites += 1;
                }
                if !solver.inconsistent {
                    delete_proof_chain(solver, closure);
                }
            } else {
                garbage = false;
                remove_gate(solver, closure, g);
                if negate_lhs {
                    closure.gates[g as usize].lhs = not_lhs;
                }
                closure.gates[g as usize].hash = hash;
                index_gate(solver, closure, g);
                debug_assert!(closure.gates[g as usize].arity == 3);
                for i in 0..closure.gates[g as usize].arity as usize {
                    let l = closure.gates[g as usize].rhs[i];
                    if l != dst && l != cond && l != then_lit && l != else_lit {
                        connect_occurrence(closure, l, g);
                    }
                }
            }
        }
    }
    if garbage && !solver.inconsistent {
        mark_gate_as_garbage(closure, g);
    }
    solver.statistics.congruent_rewritten += 1;
    solver.statistics.congruent_rewritten_ites += 1;
}

// static bool simplify_gate
fn simplify_gate(solver: &mut Solver, closure: &mut Closure, g: GateId) -> bool {
    let tag = closure.gates[g as usize].tag;
    if tag == AND_GATE {
        simplify_and_gate(solver, closure, g);
    } else if tag == XOR_GATE {
        simplify_xor_gate(solver, closure, g);
    } else {
        simplify_ite_gate(solver, closure, g);
    }
    !solver.inconsistent
}

// static bool rewrite_gate
fn rewrite_gate(solver: &mut Solver, closure: &mut Closure, g: GateId, dst: u32, src: u32) -> bool {
    let tag = closure.gates[g as usize].tag;
    if tag == AND_GATE {
        rewrite_and_gate(solver, closure, g, dst, src);
    } else if tag == XOR_GATE {
        rewrite_xor_gate(solver, closure, g, dst, src);
    } else {
        rewrite_ite_gate(solver, closure, g, dst, src);
    }
    !solver.inconsistent
}

/*------------------------------------------------------------------------*/

// struct offsetsize
#[derive(Clone, Copy, Default)]
struct OffsetSize {
    offset: u32,
    size: u32,
}

// static bool find_binary
fn find_binary(
    binaries: &[LitPair],
    offsetsize: &[OffsetSize],
    mut lit: u32,
    mut other: u32,
) -> bool {
    debug_assert!(lit != other);
    if lit > other {
        std::mem::swap(&mut lit, &mut other);
    }
    let mut l = offsetsize[lit as usize].offset as usize;
    let mut r = l + offsetsize[lit as usize].size as usize;
    while l < r {
        let m = (l + r) / 2;
        let tmp = binaries[m].lits[1];
        if tmp < other {
            l = m + 1;
        } else if tmp > other {
            r = m;
        } else {
            debug_assert!(binaries[m].lits[0] == lit);
            debug_assert!(binaries[m].lits[1] == other);
            return true;
        }
    }
    false
}

// static uint64_t rank_litpair
fn rank_litpair(p: &LitPair) -> u64 {
    let mut res = p.lits[0] as u64;
    res <<= 32;
    res += p.lits[1] as u64;
    res
}

// static void extract_binaries
fn extract_binaries(solver: &mut Solver, closure: &mut Closure) {
    if solver.options.congruencebinaries == 0 {
        return;
    }
    crate::profile::start_checked(solver, Prof::extractbinaries);
    let lits = solver.lits() as usize;
    let mut offsetsize = vec![OffsetSize::default(); lits];
    {
        let n = closure.binaries.len();
        let mut p = 0usize;
        while p != n {
            let l = closure.binaries[p].lits[0];
            let mut q = p + 1;
            while q != n && closure.binaries[q].lits[0] == l {
                q += 1;
            }
            let size = q - p;
            debug_assert!(size > 0);
            let offset = p;
            if size < 32 {
                crate::sort::sort(
                    &mut solver.sorter,
                    &mut closure.binaries[p..q],
                    |a: &LitPair, b: &LitPair| a.lits[1] < b.lits[1],
                );
            } else {
                crate::sort::radix_sort(&mut closure.binaries[p..q], |x: &LitPair| x.lits[1]);
            }
            offsetsize[l as usize] = OffsetSize {
                offset: offset as u32,
                size: size as u32,
            };
            p = q;
        }
    }
    let last_irredundant = solver.last_irredundant;
    let before = closure.binaries.len();
    let mut extracted = 0usize;
    let mut duplicated = 0usize;
    // for (all_clauses (d))
    let mut ref_: Reference = 0;
    'clauses: while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        let d = solver.arena.clause(ref_);
        if d.garbage() {
            ref_ = next;
            continue;
        }
        if last_irredundant != INVALID_REF && ref_ > last_irredundant {
            break;
        }
        if d.redundant() {
            ref_ = next;
            continue;
        }
        if d.size() != 3 {
            ref_ = next;
            continue;
        }
        let a = d.lit(0);
        let b = d.lit(1);
        let c = d.lit(2);
        if solver.values[a as usize] != 0
            || solver.values[b as usize] != 0
            || solver.values[c as usize] != 0
        {
            ref_ = next;
            continue;
        }
        let not_a = not(a);
        let not_b = not(b);
        let not_c = not(c);
        let (l, k);
        if find_binary(&closure.binaries, &offsetsize, not_a, b)
            || find_binary(&closure.binaries, &offsetsize, not_a, c)
        {
            l = b;
            k = c;
        } else if find_binary(&closure.binaries, &offsetsize, not_b, a)
            || find_binary(&closure.binaries, &offsetsize, not_b, c)
        {
            l = a;
            k = c;
        } else if find_binary(&closure.binaries, &offsetsize, not_c, a)
            || find_binary(&closure.binaries, &offsetsize, not_c, b)
        {
            l = a;
            k = b;
        } else {
            ref_ = next;
            continue 'clauses;
        }
        if !find_binary(&closure.binaries, &offsetsize, l, k) {
            add_binary_clause(solver, closure, l, k);
            extracted += 1;
        }
        ref_ = next;
    }
    drop(offsetsize);
    {
        let end = closure.binaries.len();
        debug_assert!(end - before == extracted);
        crate::sort::radix_sort(&mut closure.binaries[before..end], rank_litpair);
        let mut q = before;
        let mut prev_lit = INVALID;
        let mut prev_other = INVALID;
        for p in before..end {
            let pair = closure.binaries[p];
            let l = pair.lits[0];
            let other = pair.lits[1];
            if p == before || l != prev_lit || other != prev_other {
                closure.binaries[q] = pair;
                prev_lit = l;
                prev_other = other;
                q += 1;
            } else {
                duplicated += 1;
                crate::clause::delete_binary(solver, l, other);
            }
        }
        closure.binaries.truncate(q); // SET_END_OF_STACK
    }
    solver.statistics.congruent_binaries += (extracted - duplicated) as u64;
    crate::print::verbose(
        solver,
        format_args!(
            "extracted {} binaries (plus {} duplicated)",
            extracted, duplicated
        ),
    );
    crate::profile::stop_checked(solver, Prof::extractbinaries);
}

/*------------------------------------------------------------------------*/
// AND gate extraction (INDEX_BINARY_CLAUSES not defined).

// static gate *find_first_and_gate
fn find_first_and_gate(solver: &mut Solver, closure: &mut Closure, lhs: u32) -> GateId {
    debug_assert!(!solver.watching);
    let not_lhs = not(lhs);
    debug_assert!(solver.analyzed.is_empty());
    let arity = closure.lits.len() as u32 - 1;
    let mut matched = 0u32;
    debug_assert!(arity > 1);
    let v = solver.watches[not_lhs as usize];
    for i in v.begin..v.end {
        let w = solver.vectors.stack[i];
        debug_assert!(watch_is_binary(w));
        let other = watch_lit(w);
        let tmp = solver.marks[other as usize];
        if tmp != 0 {
            matched += 1;
            solver.marks[other as usize] |= 2;
            solver.analyzed.push(other);
        }
    }
    if matched < arity {
        return NO_GATE;
    }
    new_and_gate(solver, closure, lhs)
}

// static gate *find_remaining_and_gate
fn find_remaining_and_gate(solver: &mut Solver, closure: &mut Closure, lhs: u32) -> GateId {
    debug_assert!(!solver.watching);
    let not_lhs = not(lhs);
    if solver.marks[not_lhs as usize] < 2 {
        return NO_GATE;
    }
    let arity = closure.lits.len() as u32 - 1;
    let mut matched = 0u32;
    debug_assert!(arity > 1);
    {
        let v = solver.watches[not_lhs as usize];
        for i in v.begin..v.end {
            let w = solver.vectors.stack[i];
            debug_assert!(watch_is_binary(w));
            let other = watch_lit(w);
            let mark = solver.marks[other as usize];
            if mark == 0 {
                continue;
            }
            matched += 1;
            if mark & 2 == 0 {
                continue;
            }
            debug_assert!(mark & 4 == 0);
            solver.marks[other as usize] = mark | 4;
        }
    }
    {
        let mut marked = std::mem::take(&mut solver.analyzed);
        debug_assert!(!marked.is_empty());
        let end_marked = marked.len();
        let mut q = 0usize;
        debug_assert!(solver.marks[not_lhs as usize] == 3);
        for p in 0..end_marked {
            let l = marked[p];
            if l == not_lhs {
                solver.marks[not_lhs as usize] = 1;
                continue;
            }
            let mut mark = solver.marks[l as usize];
            debug_assert!(mark & 3 == 3);
            if mark & 4 != 0 {
                mark = 3;
                marked[q] = l;
                q += 1;
            } else {
                mark = 1;
            }
            solver.marks[l as usize] = mark;
        }
        debug_assert!(q != end_marked);
        debug_assert!(solver.marks[not_lhs as usize] == 1);
        marked.truncate(q); // SET_END_OF_STACK
        solver.analyzed = marked;
    }
    if matched < arity {
        return NO_GATE;
    }
    new_and_gate(solver, closure, lhs)
}

// static inline bool smaller_negated_bin_count
fn smaller_negated_bin_count(negbincount: &[u32], a: u32, b: u32) -> bool {
    let c = negbincount[a as usize];
    let d = negbincount[b as usize];
    if c < d {
        return true;
    }
    if c > d {
        return false;
    }
    a < b
}

// static void extract_and_gates_with_base_clause
fn extract_and_gates_with_base_clause(solver: &mut Solver, closure: &mut Closure, c_ref: Reference) {
    debug_assert!(!solver.inconsistent);
    let arity_limit = (solver.options.congruenceandarity as u32).min(MAX_ARITY);
    let size_limit = arity_limit + 1;
    let mut size = 0u32;
    let mut max_negbincount = 0u32;
    closure.lits.clear();
    let c_size = solver.arena.clause(c_ref).size();
    for i in 0..c_size {
        let l = solver.arena.clause(c_ref).lit(i);
        let value = solver.values[l as usize];
        if value < 0 {
            continue;
        }
        if value > 0 {
            debug_assert!(solver.level == 0);
            crate::clause::mark_clause_as_garbage(solver, c_ref);
            return;
        }
        size += 1;
        if size > size_limit {
            return;
        }
        let count = closure.negbincount[l as usize];
        if count == 0 {
            return;
        }
        if count > max_negbincount {
            max_negbincount = count;
        }
        closure.lits.push(l);
    }
    if size < 3 {
        return;
    }
    let arity = size - 1;
    if max_negbincount < arity {
        return;
    }
    let end_lits = closure.lits.len();
    let mut reduced_lits = 0usize;
    for p in 0..end_lits {
        let l = closure.lits[p];
        let count = closure.negbincount[l as usize];
        let not_lit = not(l);
        solver.marks[not_lit as usize] = 1;
        if count < arity {
            if reduced_lits < p {
                closure.lits[p] = closure.lits[reduced_lits];
                closure.lits[reduced_lits] = l;
                reduced_lits += 1;
            } else if reduced_lits == p {
                reduced_lits += 1;
            }
        }
    }
    debug_assert!(reduced_lits < end_lits);
    let reduced_size = end_lits - reduced_lits;
    debug_assert!(reduced_size > 0);
    // sort_lits_by_negbincount
    {
        let cl = &mut *closure;
        let negbincount = &cl.negbincount;
        crate::sort::sort(
            &mut solver.sorter,
            &mut cl.lits[reduced_lits..end_lits],
            |a: &u32, b: &u32| smaller_negated_bin_count(negbincount, *a, *b),
        );
    }
    let mut first = true;
    for p in reduced_lits..end_lits {
        if solver.inconsistent {
            break;
        }
        if solver.arena.clause(c_ref).garbage() {
            break;
        }
        let lhs = closure.lits[p];
        debug_assert!(arity <= closure.negbincount[lhs as usize]);
        if first {
            first = false;
            debug_assert!(solver.analyzed.is_empty());
            let _ = find_first_and_gate(solver, closure, lhs);
        } else if solver.analyzed.is_empty() {
            break; // early abort AND gate search
        } else {
            let _ = find_remaining_and_gate(solver, closure, lhs);
        }
    }
    for p in 0..end_lits {
        let l = closure.lits[p];
        let not_lit = not(l);
        solver.marks[not_lit as usize] = 0;
    }
    solver.analyzed.clear();
}

/*------------------------------------------------------------------------*/
// XOR gate extraction (INDEX_LARGE_CLAUSES not defined).

// static clause *find_large_xor_side_clause
fn find_large_xor_side_clause(solver: &mut Solver, closure: &mut Closure) -> Reference {
    debug_assert!(!solver.watching);
    let mut least_occurring_literal = INVALID;
    let mut count_least_occurring = u32::MAX;
    let size_lits = closure.lits.len();
    for i in 0..size_lits {
        let l = closure.lits[i];
        debug_assert!(solver.values[l as usize] == 0);
        solver.marks[l as usize] = 1;
        let count = closure.largecount[l as usize];
        if count >= count_least_occurring {
            continue;
        }
        count_least_occurring = count;
        least_occurring_literal = l;
    }
    let mut res = INVALID_REF;
    debug_assert!(least_occurring_literal != INVALID);
    let v = solver.watches[least_occurring_literal as usize];
    for i in v.begin..v.end {
        let w = solver.vectors.stack[i];
        if watch_is_binary(w) {
            break;
        }
        let d_ref = watch_ref(w);
        if solver.arena.clause(d_ref).garbage() {
            continue;
        }
        if (solver.arena.clause(d_ref).size() as usize) < size_lits {
            continue;
        }
        let d_size = solver.arena.clause(d_ref).size();
        let mut found = 0usize;
        for j in 0..d_size {
            let other = solver.arena.clause(d_ref).lit(j);
            let value = solver.values[other as usize];
            if value < 0 {
                continue;
            }
            if value > 0 {
                crate::clause::mark_clause_as_garbage(solver, d_ref);
                debug_assert!(solver.arena.clause(d_ref).garbage());
                break;
            }
            if solver.marks[other as usize] != 0 {
                found += 1;
            } else {
                found = u32::MAX as usize;
                break;
            }
        }
        if found < u32::MAX as usize && !solver.arena.clause(d_ref).garbage() {
            res = d_ref;
            break;
        }
    }
    for i in 0..size_lits {
        let l = closure.lits[i];
        solver.marks[l as usize] = 0;
    }
    res
}

// static void extract_xor_gates_with_base_clause
fn extract_xor_gates_with_base_clause(solver: &mut Solver, closure: &mut Closure, c_ref: Reference) {
    debug_assert!(!solver.inconsistent);
    let mut smallest = INVALID;
    let mut largest = INVALID;
    let arity_limit = (solver.options.congruencexorarity as u32).min(MAX_ARITY);
    let size_limit = arity_limit + 1;
    let mut negated_count = 0u32;
    let mut size = 0u32;
    closure.lits.clear();
    let mut first = true;
    let c_size = solver.arena.clause(c_ref).size();
    for i in 0..c_size {
        let l = solver.arena.clause(c_ref).lit(i);
        let value = solver.values[l as usize];
        if value < 0 {
            continue;
        }
        if value > 0 {
            crate::clause::mark_clause_as_garbage(solver, c_ref);
            return;
        }
        if size == size_limit {
            return;
        }
        if first {
            largest = l;
            smallest = l;
            first = false;
        } else {
            debug_assert!(smallest != INVALID);
            debug_assert!(largest != INVALID);
            if l < smallest {
                smallest = l;
            }
            if l > largest {
                if negated(largest) != 0 {
                    return;
                }
                largest = l;
            }
        }
        if negated(l) != 0 && l < largest {
            return;
        }
        if negated(l) != 0 {
            let prev = negated_count;
            negated_count += 1;
            if prev != 0 {
                return;
            }
        }
        closure.lits.push(l);
        size += 1;
    }
    debug_assert!(size as usize == closure.lits.len());
    if size < 3 {
        return;
    }
    let arity = size - 1;
    let needed_clauses = 1u32 << (arity - 1);
    for i in 0..closure.lits.len() {
        let mut l = closure.lits[i];
        for _sign in 0..2 {
            let count = closure.largecount[l as usize];
            if count < needed_clauses {
                return;
            }
            l = not(l);
        }
    }
    debug_assert!(smallest != INVALID);
    debug_assert!(largest != INVALID);
    let end = 1u32 << arity;
    debug_assert!(negated_count == parity_lits(&closure.lits));
    for i in 0..end {
        while i != 0 && parity_lits(&closure.lits) != negated_count {
            inc_lits(&mut closure.lits);
        }
        if i != 0 {
            let d = find_large_xor_side_clause(solver, closure);
            if d == INVALID_REF {
                return;
            }
            debug_assert!(!solver.arena.clause(d).redundant());
        } else {
            debug_assert!(!solver.arena.clause(c_ref).redundant());
        }
        inc_lits(&mut closure.lits);
    }
    while parity_lits(&closure.lits) != negated_count {
        inc_lits(&mut closure.lits);
    }
    if negated_count != 0 {
        let mut p = 0usize;
        loop {
            let l = closure.lits[p];
            if negated(l) != 0 {
                closure.lits[p] = not(l);
                break;
            }
            p += 1;
        }
    }
    let mut extracted = 0u32;
    for i in 0..closure.lits.len() {
        let mut lhs = closure.lits[i];
        if negated_count == 0 {
            lhs = not(lhs);
        }
        let g = new_xor_gate(solver, closure, lhs);
        if g != NO_GATE {
            extracted += 1;
        }
        if solver.inconsistent {
            break;
        }
    }
    let _ = extracted;
}

/*------------------------------------------------------------------------*/

// static void init_and_gate_extraction
fn init_and_gate_extraction(solver: &mut Solver, closure: &mut Closure) {
    debug_assert!(!solver.watching);
    let lits = solver.lits() as usize;
    let mut negbincount = vec![0u32; lits];
    for i in 0..closure.binaries.len() {
        let pair = closure.binaries[i];
        let l = pair.lits[0];
        let other = pair.lits[1];
        let not_lit = not(l);
        let not_other = not(other);
        negbincount[not_lit as usize] += 1;
        negbincount[not_other as usize] += 1;
        crate::watch::watch_binary(solver, l, other);
    }
    let connected = closure.binaries.len();
    crate::print::very_verbose(solver, format_args!("connected {} binary clauses", connected));
    closure.negbincount = negbincount;
}

// static void reset_and_gate_extraction
fn reset_and_gate_extraction(solver: &mut Solver, closure: &mut Closure) {
    closure.negbincount = Vec::new();
    crate::watch::flush_all_connected(solver);
}

// static void init_xor_gate_extraction
fn init_xor_gate_extraction(solver: &mut Solver, closure: &mut Closure, candidates: &mut Vec<Reference>) {
    debug_assert!(candidates.is_empty());
    debug_assert!(!solver.watching);
    let arity_limit = solver.options.congruencexorarity as u32;
    let size_limit = arity_limit + 1;
    let last_irredundant = solver.last_irredundant;
    let lits_count = solver.lits() as usize;
    let mut largecount = vec![0u32; lits_count];
    // for (all_clauses (c))
    let mut ref_: Reference = 0;
    'clauses: while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        if solver.arena.clause(ref_).garbage() {
            ref_ = next;
            continue;
        }
        if last_irredundant != INVALID_REF && ref_ > last_irredundant {
            break;
        }
        if solver.arena.clause(ref_).redundant() {
            ref_ = next;
            continue;
        }
        let c_size = solver.arena.clause(ref_).size();
        let mut size = 0u32;
        for i in 0..c_size {
            let l = solver.arena.clause(ref_).lit(i);
            let value = solver.values[l as usize];
            if value < 0 {
                continue;
            }
            if value > 0 {
                crate::clause::mark_clause_as_garbage(solver, ref_);
                ref_ = next;
                continue 'clauses; // goto CONTINUE_COUNTING_NEXT_CLAUSE
            }
            if size == size_limit {
                ref_ = next;
                continue 'clauses;
            }
            size += 1;
        }
        if size < 3 {
            ref_ = next;
            continue;
        }
        for i in 0..c_size {
            let l = solver.arena.clause(ref_).lit(i);
            if solver.values[l as usize] == 0 {
                largecount[l as usize] += 1;
            }
        }
        candidates.push(ref_);
        ref_ = next;
    }
    let considered_clauses = solver.statistics.clauses_irredundant;
    let original_candidates = candidates.len();
    crate::print::very_verbose(
        solver,
        format_args!(
            "{} original candidate XOR base clauses ({:.0}% of {} irredundant clauses)",
            original_candidates,
            percent(original_candidates as f64, considered_clauses as f64),
            considered_clauses
        ),
    );
    let counting_rounds = solver.options.congruencexorcounts;
    for round in 1..=counting_rounds {
        let mut removed = 0usize;
        let mut new_largecount = vec![0u32; lits_count];
        let end_candidates = candidates.len();
        let mut q = 0usize;
        'candidates: for p in 0..end_candidates {
            let cand = candidates[p];
            let c_size = solver.arena.clause(cand).size();
            let mut size = 0u32;
            for i in 0..c_size {
                let l = solver.arena.clause(cand).lit(i);
                if solver.values[l as usize] == 0 {
                    size += 1;
                }
            }
            debug_assert!(size >= 3);
            debug_assert!(size <= size_limit);
            let arity = size - 1;
            let needed_clauses = 1u32 << (arity - 1);
            for i in 0..c_size {
                let l = solver.arena.clause(cand).lit(i);
                if largecount[l as usize] < needed_clauses {
                    removed += 1;
                    continue 'candidates; // goto CONTINUE_WITH_NEXT_CANDIDATE_CLAUSE
                }
            }
            for i in 0..c_size {
                let l = solver.arena.clause(cand).lit(i);
                if solver.values[l as usize] == 0 {
                    new_largecount[l as usize] += 1;
                }
            }
            candidates[q] = cand;
            q += 1;
        }
        largecount = new_largecount;
        candidates.truncate(q);
        if removed == 0 {
            break;
        }
        let remaining_candidates = candidates.len();
        let how_often = if round == 1 {
            "once".to_string()
        } else if round == 2 {
            "twice".to_string()
        } else {
            format!("{} times", round)
        };
        crate::print::very_verbose(
            solver,
            format_args!(
                "{} XOR base clause candidates remain ({:.0}% original candidates) after counting {}",
                remaining_candidates,
                percent(remaining_candidates as f64, original_candidates as f64),
                how_often
            ),
        );
    }
    closure.largecount = largecount;
    for i in 0..candidates.len() {
        let cand = candidates[i];
        crate::clause::connect_referenced(solver, cand);
    }
    let connected = candidates.len();
    crate::print::very_verbose(
        solver,
        format_args!(
            "connected {} large clauses {:.0}%",
            connected,
            percent(
                connected as f64,
                solver.statistics.clauses_irredundant as f64
            )
        ),
    );
}

// static void reset_xor_gate_extraction
fn reset_xor_gate_extraction(solver: &mut Solver, closure: &mut Closure) {
    closure.largecount = Vec::new();
    crate::watch::flush_all_connected(solver);
}

// static void init_ite_gate_extraction
fn init_ite_gate_extraction(solver: &mut Solver, closure: &mut Closure, candidates: &mut Vec<Reference>) {
    debug_assert!(candidates.is_empty());
    let last_irredundant = solver.last_irredundant;
    let lits_count = solver.lits() as usize;
    let mut largecount = vec![0u32; lits_count];
    let mut ternary: Vec<Reference> = Vec::new();
    let mut ref_: Reference = 0;
    'clauses: while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        if solver.arena.clause(ref_).garbage() {
            ref_ = next;
            continue;
        }
        if last_irredundant != INVALID_REF && ref_ > last_irredundant {
            break;
        }
        if solver.arena.clause(ref_).redundant() {
            ref_ = next;
            continue;
        }
        let c_size = solver.arena.clause(ref_).size();
        let mut size = 0u32;
        for i in 0..c_size {
            let l = solver.arena.clause(ref_).lit(i);
            let value = solver.values[l as usize];
            if value < 0 {
                continue;
            }
            if value > 0 {
                crate::clause::mark_clause_as_garbage(solver, ref_);
                ref_ = next;
                continue 'clauses;
            }
            if size == 3 {
                ref_ = next;
                continue 'clauses;
            }
            size += 1;
        }
        if size < 3 {
            ref_ = next;
            continue;
        }
        debug_assert!(size == 3);
        ternary.push(ref_);
        for i in 0..c_size {
            let l = solver.arena.clause(ref_).lit(i);
            if solver.values[l as usize] == 0 {
                largecount[l as usize] += 1;
            }
        }
        ref_ = next;
    }
    let counted = ternary.len();
    crate::print::very_verbose(
        solver,
        format_args!(
            "counted {} ternary ITE clauses ({:.0}% of {} irredundant clauses)",
            counted,
            percent(counted as f64, solver.statistics.clauses_irredundant as f64),
            solver.statistics.clauses_irredundant
        ),
    );
    let mut connected = 0usize;
    'ternary: for t in 0..ternary.len() {
        let cand = ternary[t];
        debug_assert!(!solver.arena.clause(cand).garbage());
        let mut positive = 0u32;
        let mut negative = 0u32;
        let mut twice = 0u32;
        let c_size = solver.arena.clause(cand).size();
        for i in 0..c_size {
            let l = solver.arena.clause(cand).lit(i);
            if solver.values[l as usize] != 0 {
                continue;
            }
            let not_lit = not(l);
            let count_not_lit = largecount[not_lit as usize];
            if count_not_lit == 0 {
                continue 'ternary; // goto CONTINUE_WITH_NEXT_TERNARY_CLAUSE
            }
            let count_lit = largecount[l as usize];
            debug_assert!(count_lit > 0);
            if count_lit > 1 && count_not_lit > 1 {
                twice += 1;
            }
            if negated(l) != 0 {
                negative += 1;
            } else {
                positive += 1;
            }
        }
        if twice < 2 {
            continue 'ternary;
        }
        connected += 1;
        crate::clause::connect_clause(solver, cand);
        if positive != 0 && negative != 0 {
            candidates.push(cand);
        }
    }
    crate::print::very_verbose(
        solver,
        format_args!(
            "connected {} ITE clauses ({:.0}% of {} counted clauses)",
            connected,
            percent(connected as f64, counted as f64),
            solver.statistics.clauses_irredundant
        ),
    );
    let size_candidates = candidates.len();
    crate::print::very_verbose(
        solver,
        format_args!(
            "{} candidates ITE base clauses ({:.0}% of {} connected)",
            size_candidates,
            percent(size_candidates as f64, connected as f64),
            connected
        ),
    );
    closure.largecount = largecount;
    closure.condbin[0].clear();
    closure.condbin[1].clear();
    closure.condeq[0].clear();
    closure.condeq[1].clear();
}

// static void reset_ite_gate_extraction
fn reset_ite_gate_extraction(solver: &mut Solver, closure: &mut Closure) {
    closure.condbin[0] = Vec::new();
    closure.condbin[1] = Vec::new();
    closure.condeq[0] = Vec::new();
    closure.condeq[1] = Vec::new();
    closure.largecount = Vec::new();
    crate::watch::flush_all_connected(solver);
}

// static void unmark_all
fn unmark_all(solver: &mut Solver) {
    while let Some(l) = solver.analyzed.pop() {
        solver.marks[l as usize] = 0;
    }
}

/*------------------------------------------------------------------------*/
// MERGE_CONDITIONAL_EQUIVALENCES path.

// static void copy_conditional_equivalences
fn copy_conditional_equivalences(solver: &Solver, lit: u32, condbin: &mut Vec<LitPair>) {
    debug_assert!(condbin.is_empty());
    let v = solver.watches[lit as usize];
    for i in v.begin..v.end {
        let w = solver.vectors.stack[i];
        if watch_is_binary(w) {
            break;
        }
        let ref_ = watch_ref(w);
        let c = solver.arena.clause(ref_);
        let mut first = INVALID;
        let mut second = INVALID;
        for &other in c.lits() {
            if solver.values[other as usize] != 0 {
                continue;
            }
            if other == lit {
                continue;
            }
            if first == INVALID {
                first = other;
            } else {
                debug_assert!(second == INVALID);
                second = other;
            }
        }
        debug_assert!(first != INVALID);
        debug_assert!(second != INVALID);
        let pair = if first < second {
            LitPair {
                lits: [first, second],
            }
        } else {
            debug_assert!(second < first);
            LitPair {
                lits: [second, first],
            }
        };
        condbin.push(pair);
    }
}

// static bool less_litpair
fn less_litpair(p: &LitPair, q: &LitPair) -> bool {
    let a = p.lits[0];
    let b = q.lits[0];
    if a < b {
        return true;
    }
    if a > b {
        return false;
    }
    p.lits[1] < q.lits[1]
}

// static void sort_pairs
fn sort_pairs(solver: &mut Solver, pairs: &mut [LitPair]) {
    let size = pairs.len();
    if size < 32 {
        crate::sort::sort_stack(&mut solver.sorter, pairs, less_litpair);
    } else {
        // PORT NOTE: C really radix-sorts twice (`for (int i = 1; i >= 0;
        // i--) RADIX_STACK (...)`) with the same 64-bit key; kept verbatim.
        for _ in 0..2 {
            crate::sort::radix_stack(pairs, rank_litpair);
        }
    }
}

// static bool find_litpair_second_literal
fn find_litpair_second_literal(lit: u32, pairs: &[LitPair]) -> bool {
    let mut l = 0usize;
    let mut r = pairs.len();
    while l != r {
        let m = l + (r - l) / 2;
        let other = pairs[m].lits[1];
        if other < lit {
            l = m + 1;
        } else if other > lit {
            r = m;
        } else {
            return true;
        }
    }
    false
}

// static void search_condeq
fn search_condeq(
    pos_lit: u32,
    pos: &[LitPair],
    neg_lit: u32,
    neg: &[LitPair],
    condeq: &mut Vec<LitPair>,
) {
    debug_assert!(neg_lit == not(pos_lit));
    debug_assert!(!pos.is_empty());
    debug_assert!(!neg.is_empty());
    debug_assert!(pos[0].lits[0] == pos_lit);
    debug_assert!(neg[0].lits[0] == neg_lit);
    for p in pos {
        let other = p.lits[1];
        let not_other = not(other);
        if find_litpair_second_literal(not_other, neg) {
            let (first, second);
            if negated(pos_lit) != 0 {
                first = neg_lit;
                second = other;
            } else {
                first = pos_lit;
                second = not_other;
            }
            debug_assert!(negated(first) == 0);
            debug_assert!(first < second);
            condeq.push(LitPair {
                lits: [first, second],
            });
            if negated(second) != 0 {
                condeq.push(LitPair {
                    lits: [not(second), not(first)],
                });
            } else {
                condeq.push(LitPair {
                    lits: [second, first],
                });
            }
        }
    }
}

// static void extract_condeq_pairs
fn extract_condeq_pairs(_lit: u32, condbin: &[LitPair], condeq: &mut Vec<LitPair>) {
    let end = condbin.len();
    let mut pos_begin = 0usize;
    let mut next_lit;
    loop {
        if pos_begin == end {
            return;
        }
        next_lit = condbin[pos_begin].lits[0];
        if negated(next_lit) == 0 {
            break;
        }
        pos_begin += 1;
    }
    loop {
        debug_assert!(pos_begin != end);
        debug_assert!(next_lit == condbin[pos_begin].lits[0]);
        debug_assert!(negated(next_lit) == 0);
        let pos_lit = next_lit;
        let mut pos_end = pos_begin + 1;
        loop {
            if pos_end == end {
                return;
            }
            next_lit = condbin[pos_end].lits[0];
            if next_lit != pos_lit {
                break;
            }
            pos_end += 1;
        }
        debug_assert!(pos_end != end);
        debug_assert!(next_lit == condbin[pos_end].lits[0]);
        let neg_lit = not(pos_lit);
        if next_lit != neg_lit {
            if negated(next_lit) != 0 {
                pos_begin = pos_end + 1;
                loop {
                    if pos_begin == end {
                        return;
                    }
                    next_lit = condbin[pos_begin].lits[0];
                    if negated(next_lit) == 0 {
                        break;
                    }
                    pos_begin += 1;
                }
            } else {
                pos_begin = pos_end;
            }
            continue;
        }
        let neg_begin = pos_end;
        let mut neg_end = neg_begin + 1;
        while neg_end != end {
            next_lit = condbin[neg_end].lits[0];
            if next_lit != neg_lit {
                break;
            }
            neg_end += 1;
        }
        let pos_size = pos_end - pos_begin;
        let neg_size = neg_end - neg_begin;
        if pos_size <= neg_size {
            search_condeq(
                pos_lit,
                &condbin[pos_begin..pos_end],
                neg_lit,
                &condbin[neg_begin..neg_end],
                condeq,
            );
        } else {
            search_condeq(
                neg_lit,
                &condbin[neg_begin..neg_end],
                pos_lit,
                &condbin[pos_begin..pos_end],
                condeq,
            );
        }
        if neg_end == end {
            return;
        }
        debug_assert!(next_lit == condbin[neg_end].lits[0]);
        if negated(next_lit) != 0 {
            pos_begin = neg_end + 1;
            loop {
                if pos_begin == end {
                    return;
                }
                next_lit = condbin[pos_begin].lits[0];
                if negated(next_lit) == 0 {
                    break;
                }
                pos_begin += 1;
            }
        } else {
            pos_begin = neg_end;
        }
    }
}

// static void find_conditional_equivalences
fn find_conditional_equivalences(
    solver: &mut Solver,
    lit: u32,
    condbin: &mut Vec<LitPair>,
    condeq: &mut Vec<LitPair>,
) {
    debug_assert!(condbin.is_empty());
    debug_assert!(condeq.is_empty());
    debug_assert!(solver.watches[lit as usize].size() > 1);
    copy_conditional_equivalences(solver, lit, condbin);
    sort_pairs(solver, condbin);
    extract_condeq_pairs(lit, condbin, condeq);
    sort_pairs(solver, condeq);
}

// static void merge_condeq
fn merge_condeq(
    solver: &mut Solver,
    closure: &mut Closure,
    cond: u32,
    condeq: &[LitPair],
    not_condeq: &[LitPair],
) {
    debug_assert!(negated(cond) == 0);
    let mut q = 0usize;
    for p in 0..condeq.len() {
        let cond_pair = condeq[p];
        let lhs = cond_pair.lits[0];
        let then_lit = cond_pair.lits[1];
        debug_assert!(negated(lhs) == 0);
        while q != not_condeq.len() && not_condeq[q].lits[0] < lhs {
            q += 1;
        }
        let mut q2 = q;
        while q2 != not_condeq.len() && not_condeq[q2].lits[0] == lhs {
            let not_cond_pair = not_condeq[q2];
            q2 += 1;
            let else_lit = not_cond_pair.lits[1];
            new_ite_gate(solver, closure, lhs, cond, then_lit, else_lit);
            if solver.inconsistent {
                return;
            }
        }
    }
}

// static void extract_ite_gates_of_literal
fn extract_ite_gates_of_literal(solver: &mut Solver, closure: &mut Closure, lit: u32, not_lit: u32) {
    let mut condbin0 = std::mem::take(&mut closure.condbin[0]);
    let mut condbin1 = std::mem::take(&mut closure.condbin[1]);
    let mut condeq0 = std::mem::take(&mut closure.condeq[0]);
    let mut condeq1 = std::mem::take(&mut closure.condeq[1]);
    find_conditional_equivalences(solver, lit, &mut condbin0, &mut condeq0);
    if !condeq0.is_empty() {
        find_conditional_equivalences(solver, not_lit, &mut condbin1, &mut condeq1);
        if !condeq1.is_empty() {
            if negated(lit) != 0 {
                merge_condeq(solver, closure, not_lit, &condeq0, &condeq1);
            } else {
                merge_condeq(solver, closure, lit, &condeq1, &condeq0);
            }
        }
    }
    // CLEAN_UP:
    condbin0.clear();
    condbin1.clear();
    condeq0.clear();
    condeq1.clear();
    closure.condbin[0] = condbin0;
    closure.condbin[1] = condbin1;
    closure.condeq[0] = condeq0;
    closure.condeq[1] = condeq1;
}

// static void extract_ite_gates_of_variable
fn extract_ite_gates_of_variable(solver: &mut Solver, closure: &mut Closure, i: u32) {
    let lit = make_lit(i);
    let not_lit = not(lit);
    let size_lit_watches = solver.watches[lit as usize].size();
    let size_not_lit_watches = solver.watches[not_lit as usize].size();
    if size_lit_watches <= size_not_lit_watches {
        if size_lit_watches > 1 {
            extract_ite_gates_of_literal(solver, closure, lit, not_lit);
        }
    } else if size_not_lit_watches > 1 {
        extract_ite_gates_of_literal(solver, closure, not_lit, lit);
    }
}

/*------------------------------------------------------------------------*/

// static void extract_and_gates
fn extract_and_gates(solver: &mut Solver, closure: &mut Closure) {
    if solver.options.congruenceands == 0 {
        return;
    }
    crate::profile::start_checked(solver, Prof::extractands);
    let matched_before = solver.statistics.congruent_matched_ands;
    let gates_before = solver.statistics.congruent_gates_ands;
    init_and_gate_extraction(solver, closure);
    let last_irredundant = solver.last_irredundant;
    let mut ref_: Reference = 0;
    while (ref_ as u64) < solver.arena.size_wards() {
        if terminated!(solver, congruence_terminated_1) {
            break;
        }
        if solver.inconsistent {
            break;
        }
        let next = solver.arena.next_clause_ref(ref_);
        if last_irredundant != INVALID_REF && ref_ > last_irredundant {
            break;
        }
        if solver.arena.clause(ref_).redundant() {
            ref_ = next;
            continue;
        }
        if solver.arena.clause(ref_).garbage() {
            ref_ = next;
            continue;
        }
        extract_and_gates_with_base_clause(solver, closure, ref_);
        ref_ = next;
    }
    reset_and_gate_extraction(solver, closure);
    let matched = solver.statistics.congruent_matched_ands - matched_before;
    let extracted = solver.statistics.congruent_gates_ands - gates_before;
    let found = matched + extracted;
    let closures_count = solver.statistics.closures;
    crate::print::phase(
        solver,
        "congruence",
        closures_count,
        format_args!(
            "found {} AND gates ({} extracted {:.0}% + {} matched {:.0}%)",
            found,
            extracted,
            percent(extracted as f64, found as f64),
            matched,
            percent(matched as f64, found as f64)
        ),
    );
    crate::profile::stop_checked(solver, Prof::extractands);
}

// static void extract_xor_gates
fn extract_xor_gates(solver: &mut Solver, closure: &mut Closure) {
    if solver.options.congruencexors == 0 {
        return;
    }
    crate::profile::start_checked(solver, Prof::extractxors);
    let mut candidates: Vec<Reference> = Vec::new();
    init_xor_gate_extraction(solver, closure, &mut candidates);
    let matched_before = solver.statistics.congruent_matched_xors;
    let gates_before = solver.statistics.congruent_gates_xors;
    for i in 0..candidates.len() {
        if terminated!(solver, congruence_terminated_2) {
            break;
        }
        if solver.inconsistent {
            break;
        }
        let c_ref = candidates[i];
        if solver.arena.clause(c_ref).garbage() {
            continue;
        }
        extract_xor_gates_with_base_clause(solver, closure, c_ref);
    }
    reset_xor_gate_extraction(solver, closure);
    let matched = solver.statistics.congruent_matched_xors - matched_before;
    let extracted = solver.statistics.congruent_gates_xors - gates_before;
    let found = matched + extracted;
    let closures_count = solver.statistics.closures;
    crate::print::phase(
        solver,
        "congruence",
        closures_count,
        format_args!(
            "found {} XOR gates ({} extracted {:.0}% + {} matched {:.0}%)",
            found,
            extracted,
            percent(extracted as f64, found as f64),
            matched,
            percent(matched as f64, found as f64)
        ),
    );
    crate::profile::stop_checked(solver, Prof::extractxors);
}

// static void extract_ite_gates
fn extract_ite_gates(solver: &mut Solver, closure: &mut Closure) {
    if solver.options.congruenceites == 0 {
        return;
    }
    crate::profile::start_checked(solver, Prof::extractites);
    let mut candidates: Vec<Reference> = Vec::new();
    init_ite_gate_extraction(solver, closure, &mut candidates);
    let matched_before = solver.statistics.congruent_matched_ites;
    let gates_before = solver.statistics.congruent_gates_ites;
    // MERGE_CONDITIONAL_EQUIVALENCES:
    for i in 0..solver.vars {
        if solver.flags[i as usize].active {
            extract_ite_gates_of_variable(solver, closure, i);
            if solver.inconsistent {
                break;
            }
        }
    }
    reset_ite_gate_extraction(solver, closure);
    drop(candidates); // RELEASE_STACK
    let matched = solver.statistics.congruent_matched_ites - matched_before;
    let extracted = solver.statistics.congruent_gates_ites - gates_before;
    let found = matched + extracted;
    let closures_count = solver.statistics.closures;
    crate::print::phase(
        solver,
        "congruence",
        closures_count,
        format_args!(
            "found {} ITE gates ({} extracted {:.0}% + {} matched {:.0}%)",
            found,
            extracted,
            percent(extracted as f64, found as f64),
            matched,
            percent(matched as f64, found as f64)
        ),
    );
    crate::profile::stop_checked(solver, Prof::extractites);
}

// static void init_extraction
fn init_extraction(solver: &mut Solver, closure: &mut Closure) {
    crate::dense::enter_dense_mode(solver, Some(&mut closure.binaries));
}

// static void reset_extraction
fn reset_extraction(solver: &mut Solver, closure: &mut Closure) {
    crate::dense::resume_sparse_mode(solver, false, Some(&mut closure.binaries));
    closure.binaries = Vec::new();
}

// static void extract_gates
fn extract_gates(solver: &mut Solver, closure: &mut Closure) {
    crate::profile::start_checked(solver, Prof::extract);
    debug_assert!(solver.level == 0);
    let before = solver.statistics.congruent_gates + solver.statistics.congruent_matched;
    init_extraction(solver, closure);
    extract_binaries(solver, closure);
    debug_assert!(!solver.inconsistent);
    extract_and_gates(solver, closure);
    if !solver.inconsistent && !terminated!(solver, congruence_terminated_4) {
        extract_xor_gates(solver, closure);
        if !solver.inconsistent && !terminated!(solver, congruence_terminated_5) {
            extract_ite_gates(solver, closure);
        }
    }
    reset_extraction(solver, closure);
    let after = solver.statistics.congruent_gates + solver.statistics.congruent_matched;
    let found = after - before;
    let closures_count = solver.statistics.closures;
    let active = solver.active;
    crate::print::phase(
        solver,
        "congruence",
        closures_count,
        format_args!(
            "found {} gates ({:.2}% variables)",
            found,
            percent(found as f64, active as f64)
        ),
    );
    crate::profile::stop_checked(solver, Prof::extract);
}

/*------------------------------------------------------------------------*/

// static void find_units
fn find_units(solver: &mut Solver, closure: &mut Closure) {
    debug_assert!(solver.watching);
    debug_assert!(!solver.inconsistent);
    closure.units = solver.propagate;
    let mut units = 0usize;
    for i in 0..solver.vars {
        'restart: loop {
            if !solver.flags[i as usize].active {
                break;
            }
            let base = make_lit(i);
            for sign in 0..2u32 {
                let l = base + sign;
                let v = solver.watches[l as usize];
                debug_assert!(solver.analyzed.is_empty());
                for wi in v.begin..v.end {
                    let w = solver.vectors.stack[wi];
                    if !watch_is_binary(w) {
                        break;
                    }
                    let other = watch_lit(w);
                    let not_other = not(other);
                    if solver.marks[not_other as usize] != 0 {
                        units += 1;
                        let failed = !learn_congruence_unit(solver, closure, l);
                        unmark_all(solver);
                        if failed {
                            return;
                        } else {
                            continue 'restart;
                        }
                    }
                    if solver.marks[other as usize] != 0 {
                        continue;
                    }
                    solver.marks[other as usize] = 1;
                    solver.analyzed.push(other);
                }
                unmark_all(solver);
            }
            break;
        }
    }
    debug_assert!(solver.analyzed.is_empty());
    crate::print::very_verbose(solver, format_args!("found {} units", units));
}

// static void find_equivalences
fn find_equivalences(solver: &mut Solver, closure: &mut Closure) {
    debug_assert!(solver.watching);
    debug_assert!(!solver.inconsistent);
    debug_assert!(solver.analyzed.is_empty());
    for i in 0..solver.vars {
        'restart: loop {
            if !solver.flags[i as usize].active {
                break;
            }
            let l = make_lit(i);
            {
                let v = solver.watches[l as usize];
                debug_assert!(solver.analyzed.is_empty());
                for wi in v.begin..v.end {
                    let w = solver.vectors.stack[wi];
                    if !watch_is_binary(w) {
                        break;
                    }
                    let other = watch_lit(w);
                    if l > other {
                        continue;
                    }
                    if solver.marks[other as usize] != 0 {
                        continue;
                    }
                    solver.marks[other as usize] = 1;
                    solver.analyzed.push(other);
                }
            }
            if solver.analyzed.is_empty() {
                break;
            }
            let not_lit = not(l);
            let mut restart = false;
            {
                let v = solver.watches[not_lit as usize];
                for wi in v.begin..v.end {
                    let w = solver.vectors.stack[wi];
                    if !watch_is_binary(w) {
                        break;
                    }
                    let other = watch_lit(w);
                    if not_lit > other {
                        continue;
                    }
                    if l == other {
                        continue;
                    }
                    let not_other = not(other);
                    if solver.marks[not_other as usize] != 0 {
                        let lit_repr = find_repr(closure, l);
                        let other_repr = find_repr(closure, other);
                        if lit_repr != other_repr {
                            if merge_literals(solver, closure, l, other) {
                                solver.statistics.congruent_equivalences += 1;
                            }
                            unmark_all(solver);
                            if solver.inconsistent {
                                return;
                            } else {
                                restart = true;
                                break;
                            }
                        }
                    }
                }
            }
            if restart {
                continue 'restart;
            }
            unmark_all(solver);
            break;
        }
    }
    debug_assert!(solver.analyzed.is_empty());
    let found = closure.schedule.len();
    crate::print::very_verbose(solver, format_args!("found {} equivalences", found));
}

/*------------------------------------------------------------------------*/

// static bool simplify_gates
fn simplify_gates(solver: &mut Solver, closure: &mut Closure, lit: u32) -> bool {
    debug_assert!(solver.values[lit as usize] != 0);
    let occs = std::mem::take(&mut closure.occurrences[lit as usize]);
    for &g in &occs {
        if !simplify_gate(solver, closure, g) {
            // PORT NOTE: C returns without RELEASE_STACK here; the
            // occurrence list is dropped either way when the closure resets.
            closure.occurrences[lit as usize] = occs;
            return false;
        }
    }
    // RELEASE_STACK (*lit_occurrences):
    closure.occurrences[lit as usize] = Vec::new();
    drop(occs);
    true
}

// static bool rewrite_gates
fn rewrite_gates(solver: &mut Solver, closure: &mut Closure, dst: u32, src: u32) -> bool {
    let occs = std::mem::take(&mut closure.occurrences[src as usize]);
    for &g in &occs {
        if !rewrite_gate(solver, closure, g, dst, src) {
            closure.occurrences[src as usize] = occs;
            return false;
        } else if !closure.gates[g as usize].garbage && gate_contains(&closure.gates[g as usize], dst)
        {
            closure.occurrences[dst as usize].push(g);
        }
    }
    // RELEASE_STACK (*src_occurrences):
    closure.occurrences[src as usize] = Vec::new();
    drop(occs);
    true
}

// static bool propagate_unit
fn propagate_unit(solver: &mut Solver, closure: &mut Closure, lit: u32) -> bool {
    debug_assert!(!solver.inconsistent);
    let not_lit = not(lit);
    simplify_gates(solver, closure, lit) && simplify_gates(solver, closure, not_lit)
}

// static bool propagate_equivalence
fn propagate_equivalence(solver: &mut Solver, closure: &mut Closure, lit: u32) -> bool {
    debug_assert!(!solver.inconsistent);
    if solver.values[lit as usize] != 0 {
        return true;
    }
    let lit_repr = find_repr(closure, lit);
    if solver.inconsistent {
        return false;
    }
    let not_lit = not(lit);
    let not_lit_repr = not(lit_repr);
    rewrite_gates(solver, closure, lit_repr, lit)
        && rewrite_gates(solver, closure, not_lit_repr, not_lit)
}

// static bool propagate_units
fn propagate_units(solver: &mut Solver, closure: &mut Closure) -> bool {
    debug_assert!(!solver.inconsistent);
    while closure.units != solver.trail.len() {
        let l = solver.trail[closure.units];
        closure.units += 1;
        if !propagate_unit(solver, closure, l) {
            return false;
        }
    }
    true
}

// static size_t propagate_units_and_equivalences
fn propagate_units_and_equivalences(solver: &mut Solver, closure: &mut Closure) -> usize {
    debug_assert!(!solver.inconsistent);
    crate::profile::start_checked(solver, Prof::merge);
    let mut propagated = 0usize;
    while !terminated!(solver, congruence_terminated_6)
        && propagate_units(solver, closure)
        && !closure.schedule.is_empty()
    {
        propagated += 1;
        let l = dequeue_next_scheduled_literal(closure);
        if !propagate_equivalence(solver, closure, l) {
            break;
        }
    }
    let units = closure.units;
    crate::print::very_verbose(solver, format_args!("propagated {} congruence units", units));
    crate::print::very_verbose(
        solver,
        format_args!("propagated {} congruence equivalences", propagated),
    );
    crate::profile::stop_checked(solver, Prof::merge);
    propagated
}

/*------------------------------------------------------------------------*/

// static bool find_subsuming_clause
fn find_subsuming_clause(solver: &mut Solver, closure: &mut Closure, c_ref: Reference) -> bool {
    debug_assert!(!solver.arena.clause(c_ref).garbage());
    let c_size = solver.arena.clause(c_ref).size();
    let c_redundant = solver.arena.clause(c_ref).redundant();
    for i in 0..c_size {
        let l = solver.arena.clause(c_ref).lit(i);
        debug_assert!(solver.values[l as usize] <= 0);
        let repr_lit = find_repr(closure, l);
        let value_repr_lit = solver.values[repr_lit as usize];
        debug_assert!(value_repr_lit <= 0);
        if value_repr_lit < 0 {
            continue;
        }
        if solver.marks[repr_lit as usize] != 0 {
            continue;
        }
        debug_assert!(solver.marks[not(repr_lit) as usize] == 0);
        solver.marks[repr_lit as usize] = 1;
    }
    let mut least_occurring_literal = INVALID;
    let mut count_least_occurring = u32::MAX;
    let mut subsuming = INVALID_REF;
    'outer: for i in 0..c_size {
        let l = solver.arena.clause(c_ref).lit(i);
        let repr_lit = find_repr(closure, l);
        let v = solver.watches[repr_lit as usize];
        let count = (v.end - v.begin) as u32;
        if count < count_least_occurring {
            count_least_occurring = count;
            least_occurring_literal = repr_lit;
        }
        'watches: for wi in v.begin..v.end {
            let w = solver.vectors.stack[wi];
            debug_assert!(!watch_is_binary(w));
            let d_ref = watch_ref(w);
            debug_assert!(c_ref != d_ref);
            debug_assert!(!solver.arena.clause(d_ref).garbage());
            if !c_redundant && solver.arena.clause(d_ref).redundant() {
                continue;
            }
            let d_size = solver.arena.clause(d_ref).size();
            for j in 0..d_size {
                let other = solver.arena.clause(d_ref).lit(j);
                let value = solver.values[other as usize];
                if value < 0 {
                    continue;
                }
                debug_assert!(value == 0);
                let repr_other = find_repr(closure, other);
                if solver.marks[repr_other as usize] == 0 {
                    continue 'watches; // goto CONTINUE_WITH_NEXT_CLAUSE
                }
            }
            subsuming = d_ref;
            break 'outer; // goto FOUND_SUBSUMING
        }
    }
    // FOUND_SUBSUMING:
    for i in 0..c_size {
        let l = solver.arena.clause(c_ref).lit(i);
        let repr_lit = find_repr(closure, l);
        let value = solver.values[repr_lit as usize];
        if value == 0 {
            solver.marks[repr_lit as usize] = 0;
        }
    }
    if subsuming != INVALID_REF {
        crate::clause::mark_clause_as_garbage(solver, c_ref);
        solver.statistics.congruent_subsumed += 1;
        true
    } else {
        debug_assert!(least_occurring_literal != INVALID);
        debug_assert!(count_least_occurring < u32::MAX);
        crate::watch::connect_literal(solver, least_occurring_literal, c_ref);
        false
    }
}

// struct refsize
#[derive(Clone, Copy)]
struct RefSize {
    ref_: Reference,
    size: u32,
}

// static void sort_references_by_clause_size
fn sort_references_by_clause_size(candidates: &mut [RefSize]) {
    crate::sort::radix_stack(candidates, |r: &RefSize| r.size);
}

// static void forward_subsume_matching_clauses
fn forward_subsume_matching_clauses(solver: &mut Solver, closure: &mut Closure) {
    crate::profile::start_checked(solver, Prof::matching);
    reset_closure(solver, closure);
    let mut binaries: Vec<LitPair> = Vec::new();
    crate::dense::enter_dense_mode(solver, Some(&mut binaries));
    let vars = solver.vars as usize;
    let mut matchable = vec![false; vars];
    let mut count_matchable = 0usize;
    for i in 0..solver.vars {
        if !solver.flags[i as usize].active {
            continue;
        }
        let l = make_lit(i);
        let repr = find_repr(closure, l);
        if l == repr {
            continue;
        }
        let repr_idx = idx(repr);
        if !matchable[i as usize] {
            matchable[i as usize] = true;
            count_matchable += 1;
        }
        if !matchable[repr_idx as usize] {
            matchable[repr_idx as usize] = true;
            count_matchable += 1;
        }
    }
    let closures_count = solver.statistics.closures;
    let active = solver.active;
    crate::print::phase(
        solver,
        "congruence",
        closures_count,
        format_args!(
            "found {} matchable variables {:.0}%",
            count_matchable,
            percent(count_matchable as f64, active as f64)
        ),
    );
    let mut potential = 0usize;
    let mut candidates: Vec<RefSize> = Vec::new();
    let last_irredundant = solver.last_irredundant;
    let mut ref_: Reference = 0;
    while (ref_ as u64) < solver.arena.size_wards() {
        let next = solver.arena.next_clause_ref(ref_);
        if solver.arena.clause(ref_).garbage() {
            ref_ = next;
            continue;
        }
        if last_irredundant != INVALID_REF && ref_ > last_irredundant {
            break;
        }
        potential += 1;
        let mut contains_matchable = false;
        debug_assert!(solver.analyzed.is_empty());
        let c_size = solver.arena.clause(ref_).size();
        for i in 0..c_size {
            let l = solver.arena.clause(ref_).lit(i);
            let value = solver.values[l as usize];
            if value < 0 {
                continue;
            }
            if value > 0 {
                crate::clause::mark_clause_as_garbage(solver, ref_);
                break;
            }
            if !contains_matchable {
                let lit_idx = idx(l);
                if matchable[lit_idx as usize] {
                    contains_matchable = true;
                }
            }
            let repr = find_repr(closure, l);
            debug_assert!(solver.values[repr as usize] == 0);
            if solver.marks[repr as usize] != 0 {
                continue;
            }
            let not_repr = not(repr);
            if solver.marks[not_repr as usize] != 0 {
                crate::clause::mark_clause_as_garbage(solver, ref_);
                break;
            }
            solver.marks[repr as usize] = 1;
            solver.analyzed.push(repr);
        }
        let size = solver.analyzed.len();
        unmark_all(solver);
        if solver.arena.clause(ref_).garbage() {
            ref_ = next;
            continue;
        }
        if !contains_matchable {
            ref_ = next;
            continue;
        }
        candidates.push(RefSize {
            ref_,
            size: size as u32,
        });
        ref_ = next;
    }
    drop(matchable);
    let size_candidates = candidates.len();
    crate::print::very_verbose(
        solver,
        format_args!(
            "considering {} matchable subsumption candidates {:.0}%",
            size_candidates,
            percent(size_candidates as f64, potential as f64)
        ),
    );
    sort_references_by_clause_size(&mut candidates);
    let mut tried = 0usize;
    let mut subsumed = 0usize;
    for i in 0..candidates.len() {
        if terminated!(solver, congruence_terminated_7) {
            break;
        }
        tried += 1;
        let c_ref = candidates[i].ref_;
        if find_subsuming_clause(solver, closure, c_ref) {
            subsumed += 1;
        }
    }
    let closures_count = solver.statistics.closures;
    crate::print::phase(
        solver,
        "congruence",
        closures_count,
        format_args!(
            "subsumed {} clauses out of {} tried {:.0}%",
            subsumed,
            tried,
            percent(subsumed as f64, tried as f64)
        ),
    );
    crate::dense::resume_sparse_mode(solver, false, Some(&mut binaries));
    drop(candidates);
    drop(binaries);
    crate::profile::stop_checked(solver, Prof::matching);
}

/*------------------------------------------------------------------------*/

// bool kissat_congruence
pub fn congruence(solver: &mut Solver) -> bool {
    if solver.inconsistent {
        return false;
    }
    debug_assert!(solver.level == 0);
    debug_assert!(solver.probing);
    debug_assert!(solver.watching);
    if solver.options.congruence == 0 {
        return false;
    }
    if solver.options.congruenceands == 0
        && solver.options.congruenceites == 0
        && solver.options.congruencexors == 0
    {
        return false;
    }
    if solver.options.congruenceonce != 0 && solver.statistics.closures != 0 {
        return false;
    }
    if terminated!(solver, congruence_terminated_8) {
        return false;
    }
    if crate::kimits::delaying(solver, DelayId::Congruence) {
        return false;
    }
    crate::profile::start_checked(solver, Prof::congruence);
    solver.statistics.closures += 1;
    let mut closure = init_closure(solver);
    extract_gates(solver, &mut closure);
    let mut reset = false;
    if !solver.inconsistent && !terminated!(solver, congruence_terminated_9) {
        find_units(solver, &mut closure);
        if !solver.inconsistent && !terminated!(solver, congruence_terminated_10) {
            find_equivalences(solver, &mut closure);
            if !solver.inconsistent && !terminated!(solver, congruence_terminated_11) {
                let propagated = propagate_units_and_equivalences(solver, &mut closure);
                if !solver.inconsistent
                    && propagated != 0
                    && !terminated!(solver, congruence_terminated_12)
                {
                    forward_subsume_matching_clauses(solver, &mut closure);
                    reset = true;
                }
            }
        }
    }
    if !reset {
        reset_closure(solver, &mut closure);
    }
    let equivalent = reset_repr(solver, &mut closure);
    let closures_count = solver.statistics.closures;
    let active = solver.active;
    crate::print::phase(
        solver,
        "congruence",
        closures_count,
        format_args!(
            "merged {} equivalent variables {:.2}%",
            equivalent,
            percent(equivalent as f64, active as f64)
        ),
    );
    debug_assert!(solver.active >= equivalent);
    // #ifndef QUIET (kept: QUIET not defined):
    solver.active -= equivalent;
    crate::report::report(solver, equivalent == 0, 'c');
    if !solver.inconsistent {
        solver.active += equivalent;
    }
    if crate::utilities::average(equivalent as f64, solver.active as f64) < 0.001 {
        crate::kimits::bump_delay(solver, DelayId::Congruence);
    } else {
        crate::kimits::reduce_delay(solver, DelayId::Congruence);
    }
    crate::profile::stop_checked(solver, Prof::congruence);
    equivalent != 0
}
