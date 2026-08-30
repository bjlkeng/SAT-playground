// Port of src/internal.h + src/internal.c (kissat 4.0.4).
//
// Build configuration ported: NDEBUG defined; QUIET/NOPTIONS/NPROOFS/METRICS/
// LOGGING NOT defined.  Fields under `#ifndef NDEBUG`-only or METRICS/LOGGING
// guards are omitted; profiles/options/proof/added/removed/original are kept.
//
// PORT NOTE: sibling module type names are the CamelCase of the C typedefs and
// are referenced as if the sibling modules exist (integration pass wires them):
//   crate::arena::Arena            (arena.h: STACK(ward))
//   crate::reference::Reference    (reference.h: typedef unsigned reference)
//   crate::vector::Vectors         (vector.h: struct vectors)
//   crate::watch::{Watch, Watches} (watch.h: union watch = one u32 word;
//                                   typedef vector watches)
//   crate::heap::Heap
//   crate::queue::{Queue, Links}
//   crate::flags::Flags
//   crate::phases::Phases
//   crate::averages::Averages
//   crate::reluctant::Reluctant
//   crate::options::Options
//   crate::statistics::Statistics
//   crate::profile::Profiles
//   crate::format::Format
//   crate::frames::Frame           (frames.h struct frame; NDEBUG: no `saved`)
//   crate::extend::Extension       (extend.h struct extension)
//   crate::proof::Proof
//   crate::mode::Mode
//   crate::random (generator = u64)
// PORT NOTE: all sibling types are assumed to implement Default with all-zero
// contents (C calloc semantics); kissat_init then overwrites the non-zero
// fields exactly as internal.c does.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::arena::Arena;
use crate::averages::Averages;
use crate::extend::Extension;
use crate::flags::Flags;
use crate::format::Format;
use crate::frames::Frame;
use crate::heap::Heap;
use crate::kimits::{Bounds, Delays, Enabled, Limited, Limits, Remember};
use crate::mode::Mode;
use crate::options::Options;
use crate::phases::Phases;
use crate::profile::Profiles;
use crate::proof::Proof;
use crate::queue::{Links, Queue};
use crate::reference::Reference;
use crate::reluctant::Reluctant;
use crate::statistics::Statistics;
use crate::vector::Vectors;
use crate::watch::{Watch, Watches};

/// INVALID_LIT / INVALID_IDX / INVALID_REF / UINT_MAX sentinel.
pub const INVALID: u32 = u32::MAX;

// ---------------------------------------------------------------------------
// assign.h — the `assigned` struct ONLY (assign.c logic lives in assign.rs).
// ---------------------------------------------------------------------------

pub const DECISION_REASON: u32 = u32::MAX;
pub const UNIT_REASON: u32 = DECISION_REASON - 1;

pub const INVALID_LEVEL: u32 = u32::MAX;
pub const INVALID_TRAIL: u32 = u32::MAX;

/// Port of `struct assigned` (assign.h).  C packs the five bools as one-bit
/// bitfields; plain bools here (layout does not affect trajectory).
#[derive(Clone, Copy, Default)]
pub struct Assigned {
    pub level: u32,
    pub trail: u32,

    pub analyzed: bool,
    pub binary: bool,
    pub poisoned: bool,
    pub removable: bool,
    pub shrinkable: bool,

    pub reason: u32,
}

// ---------------------------------------------------------------------------
// classify.h — struct only (classify.c logic lives in classify.rs).
// ---------------------------------------------------------------------------

/// Port of `struct classification` (classify.h).
#[derive(Clone, Copy, Default)]
pub struct Classification {
    pub small: bool,
    pub bigbig: bool,
}

// ---------------------------------------------------------------------------
// internal.h auxiliary structs
// ---------------------------------------------------------------------------

/// Port of `struct datarank` (internal.h).
#[derive(Clone, Copy, Default)]
pub struct Datarank {
    pub data: u32,
    pub rank: u32,
}

/// Port of `struct import` (internal.h).
#[derive(Clone, Copy, Default)]
pub struct Import {
    pub lit: u32,
    pub extension: bool,
    pub imported: bool,
    pub eliminated: bool,
}

/// Port of `struct termination` (internal.h).
/// PORT NOTE: the C struct also carries a `state` pointer and a `terminate`
/// callback for kissat_set_terminate; per port instructions only the volatile
/// `flagged` flag is kept (as an AtomicBool).  kissat_set_terminate is not
/// ported.
#[derive(Default)]
pub struct Termination {
    pub flagged: AtomicBool,
}

/// PORT NOTE: placeholder for `struct kitten` (kitten.c belongs to another
/// agent; replace with the real type at integration).
#[derive(Default)]
pub struct KittenStub;

// ---------------------------------------------------------------------------
// struct kissat  →  pub struct Solver
// ---------------------------------------------------------------------------

/// Port of `struct kissat` (internal.h), fields in C declaration order.
///
/// C heap arrays (`assigned *`, `flags *`, `mark *marks`, `value *values`,
/// `links *`, `watches *`) become Vec<T>; variable-indexed arrays hold
/// `solver.size` entries, literal-indexed arrays `2 * solver.size` entries,
/// exactly mirroring the C allocation in resize.c.
#[derive(Default)]
pub struct Solver {
    pub extended: bool,
    pub inconsistent: bool,
    pub iterating: bool,
    pub preprocessing: bool,
    pub probing: bool,
    pub sectioned: bool, // #ifndef QUIET — kept
    pub stable: bool,
    pub warming: bool,
    pub watching: bool,

    pub large_clauses_watched_after_binary_clauses: bool,

    pub termination: Termination,

    pub vars: u32,
    pub size: u32,
    pub active: u32,
    pub randec: u32,

    pub export_: Vec<i32>,      // ints export;
    pub units: Vec<i32>,        // ints units;
    pub import_: Vec<Import>,   // imports import;
    pub extend: Vec<Extension>, // extensions extend;
    pub witness: Vec<u32>,      // unsigneds witness;

    pub assigned: Vec<Assigned>, // assigned *assigned;  (var-indexed)
    pub flags: Vec<Flags>,       // flags *flags;        (var-indexed)

    pub marks: Vec<i8>, // mark *marks;  (lit-indexed)

    pub values: Vec<i8>, // value *values;  (lit-indexed)
    pub phases: Phases,

    pub eliminated: Vec<i8>, // eliminated = STACK (value)
    pub etrail: Vec<u32>,    // unsigneds etrail;

    pub links: Vec<Links>, // links *links;  (var-indexed)
    pub queue: Queue,

    pub scores: Heap,
    pub scinc: f64,

    pub schedule: Heap,
    pub scoreshift: f64,

    pub level: u32,
    pub frames: Vec<Frame>, // frames frames;

    // PORT NOTE: C `unsigned_array trail` is a preallocated begin/end array of
    // `size` words; `unsigned *propagate` is a cursor pointer into it.  Ported
    // as a Vec plus an index (kissat_reset_propagate → solver.propagate = 0;
    // PUSH_ARRAY → push).  Capacity policy never affects semantics.
    pub trail: Vec<u32>,
    pub propagate: usize,

    pub best_assigned: u32,
    pub target_assigned: u32,
    pub unflushed: u32,
    pub unassigned: u32,

    pub delayed: Vec<u32>, // unsigneds delayed;

    // resolvent stack is (LOGGING || !NDEBUG)-only — omitted.
    pub resolvent_size: u32,
    pub antecedent_size: u32,

    pub ranks: Vec<Datarank>, // dataranks ranks;

    pub analyzed: Vec<u32>,   // unsigneds analyzed;
    pub levels: Vec<u32>,     // unsigneds levels;
    pub minimize: Vec<u32>,   // unsigneds minimize;
    pub poisoned: Vec<u32>,   // unsigneds poisoned;
    pub promote: Vec<u32>,    // unsigneds promote;
    pub removable: Vec<u32>,  // unsigneds removable;
    pub shrinkable: Vec<u32>, // unsigneds shrinkable;

    /// PORT NOTE: C embeds a `clause` header here (`clause conflict;`) used as
    /// the fake binary-conflict clause; kissat_init sets conflict.size = 2.
    /// Uses crate::clause::Clause with its inline lits[3].
    pub conflict: crate::clause::Clause,

    pub clause_satisfied: bool,
    pub clause_shrink: bool,
    pub clause_trivial: bool,

    pub clause: Vec<u32>, // unsigneds clause;
    pub shadow: Vec<u32>, // unsigneds shadow;

    pub arena: Arena,
    pub vectors: Vectors,
    pub first_reducible: Reference,
    pub last_irredundant: Reference,
    pub watches: Vec<Watches>, // watches *watches;  (lit-indexed)

    pub last_learned: [Reference; 4],

    pub sorter: Vec<usize>, // sizes sorter;  (STACK (size_t))

    pub random: u64, // generator random;
    pub averages: [Averages; 2],
    pub tier1: [u32; 2],
    pub tier2: [u32; 2],
    pub reluctant: Reluctant,

    pub bounds: Bounds,
    pub classification: Classification,
    pub delays: Delays,
    pub enabled: Enabled,
    pub limited: Limited,
    pub limits: Limits,
    pub last: Remember, // remember last;
    pub walked: u32,

    pub mode: Mode,

    pub ticks: u64,

    pub format: Format,
    pub prefix: String, // char *prefix;

    pub antecedents: [Vec<Watch>; 2], // statches antecedents[2];
    pub gates: [Vec<Watch>; 2],       // statches gates[2];
    // PORT NOTE: C `patches xorted[2]` is STACK (watch *) — stacks of pointers
    // to watch words inside literal watch lists.  Ported as word indices into
    // the owning vector (usize); the definition/gate-extraction module owner
    // fixes the exact representation at integration.
    pub xorted: [Vec<usize>; 2],
    pub resolvents: Vec<u32>, // unsigneds resolvents;
    pub resolve_gate: bool,

    pub kitten: Option<Box<KittenStub>>, // struct kitten *kitten;
    pub gate_eliminated: bool,           // non-METRICS variant
    pub sweep_incomplete: bool,
    pub sweep_schedule: Vec<u32>, // unsigneds sweep_schedule;

    // !NPROOFS — kept:
    pub added: Vec<u32>,   // unsigneds added;
    pub removed: Vec<u32>, // unsigneds removed;

    // !NPROOFS — kept:
    pub original: Vec<i32>, // ints original;
    pub offset_of_last_original_clause: usize,

    // #ifndef QUIET — kept:
    pub profiles: Profiles,

    // #ifndef NOPTIONS — kept:
    pub options: Options,

    // checker: #ifndef NDEBUG only — omitted.

    // #ifndef NPROOFS — kept:
    pub proof: Option<Box<Proof>>,

    pub statistics: Statistics,
}

// ---------------------------------------------------------------------------
// internal.h macros / inline helpers
// ---------------------------------------------------------------------------

impl Solver {
    /// `VARS` macro.
    #[inline]
    pub fn vars(&self) -> u32 {
        self.vars
    }

    /// `LITS` macro (2 * vars).
    #[inline]
    pub fn lits(&self) -> u32 {
        2 * self.vars
    }

    /// `TIER1` macro.  PORT NOTE: kissat 4.0.4's live `#else` branch reads
    /// tier1[0] and tier2[1] unconditionally (NOT indexed by stable) — quirk
    /// ported as-is.
    #[inline]
    pub fn tier1(&self) -> u32 {
        self.tier1[0]
    }

    /// `TIER2` macro — see tier1 note: index 1, unconditionally.
    #[inline]
    pub fn tier2(&self) -> u32 {
        self.tier2[1]
    }
}

/// Port of `kissat_assigned` (internal.h): number of assigned variables.
#[inline]
pub fn assigned(solver: &Solver) -> u32 {
    debug_assert!(solver.vars >= solver.unassigned);
    solver.vars - solver.unassigned
}

/// Port of `kissat_fixed` (inline.h).
/// PORT NOTE: defined here because inline.h has no module of its own; it is
/// needed by kissat_add below.  If another module claims it, keep one copy.
#[inline]
pub fn fixed(solver: &Solver, lit: u32) -> i8 {
    debug_assert!(lit < solver.lits());
    let res = solver.values[lit as usize];
    if res == 0 {
        return 0;
    }
    if solver.assigned[(lit >> 1) as usize].level != 0 {
        return 0;
    }
    res
}

// ---------------------------------------------------------------------------
// internal.c
// ---------------------------------------------------------------------------

/// Port of `kissat_reset_last_learned`.
pub fn reset_last_learned(solver: &mut Solver) {
    for p in solver.last_learned.iter_mut() {
        *p = INVALID; // INVALID_REF
    }
}

/// Port of `kissat_init`.  C callocs the struct (all zeros) and then sets the
/// non-zero fields; Default provides the zeroing.
pub fn init() -> Solver {
    let mut solver = Solver::default();
    crate::options::init_options(&mut solver.options);
    crate::profile::init_profiles(&mut solver.profiles);
    // START (total)
    // PORT NOTE: sibling profile API guessed as start(solver, ProfileId); the
    // START macro's `GET_OPTION (profile) >= profile->level` check is assumed
    // to live inside crate::profile::start.
    crate::profile::start(&mut solver, crate::profile::Prof::total);
    crate::queue::init_queue(&mut solver);
    // kissat_push_frame (solver, UINT_MAX)
    // PORT NOTE: kissat_push_frame lives in inlineframes.h; assumed ported to
    // crate::frames.
    crate::frames::push_frame(&mut solver, u32::MAX);
    solver.watching = true;
    solver.conflict.size = 2;
    solver.scinc = 1.0;
    solver.first_reducible = INVALID; // INVALID_REF
    solver.last_irredundant = INVALID; // INVALID_REF
    reset_last_learned(&mut solver);
    solver.prefix = "c ".to_string();
    solver
}

/// Port of `kissat_set_prefix`.
pub fn set_prefix(solver: &mut Solver, prefix: &str) {
    solver.prefix = prefix.to_string();
}

/// Port of `kissat_release`.
/// PORT NOTE: in C this frees every stack/array; in Rust Drop does all of it.
/// Kept as an explicit function so C call sites read the same.
pub fn release(solver: Solver) {
    drop(solver);
}

/// Port of `kissat_reserve`.
pub fn reserve(solver: &mut Solver, max_var: i32) {
    debug_assert!(max_var >= 0);
    crate::resize::increase_size(solver, max_var as u32);
    if solver.options.tumble == 0 {
        for idx in 1..=max_var {
            let _ = crate::import::import_literal(solver, idx);
        }
        for idx in 0..max_var as u32 {
            crate::flags::activate_literal(solver, 2 * idx); // LIT (idx)
        }
    }
}

/// Port of `kissat_get_option`.
pub fn get_option(solver: &mut Solver, name: &str) -> i32 {
    crate::options::options_get(&solver.options, name)
}

/// Port of `kissat_set_option`.
pub fn set_option(solver: &mut Solver, name: &str, new_value: i32) -> i32 {
    crate::options::options_set(&mut solver.options, name, new_value)
}

/// Port of `kissat_set_decision_limit`.
pub fn set_decision_limit(solver: &mut Solver, limit: u32) {
    solver.limited.decisions = true;
    debug_assert!(u64::MAX - limit as u64 >= solver.statistics.decisions);
    solver.limits.decisions = solver.statistics.decisions + limit as u64;
}

/// Port of `kissat_set_conflict_limit`.
pub fn set_conflict_limit(solver: &mut Solver, limit: u32) {
    solver.limited.conflicts = true;
    debug_assert!(u64::MAX - limit as u64 >= solver.statistics.conflicts);
    solver.limits.conflicts = solver.statistics.conflicts + limit as u64;
}

/// Port of `kissat_print_statistics` (QUIET off, NDEBUG on: checker part
/// omitted).
pub fn print_statistics(solver: &mut Solver) {
    let verbosity = crate::print::verbosity(solver);
    if verbosity < 0 {
        return;
    }
    if solver.options.profile != 0 {
        crate::print::section(solver, "profiling");
        crate::profile::profiles_print(solver);
    }
    let complete = solver.options.statistics != 0;
    crate::print::section(solver, "statistics");
    let verbose = complete || verbosity > 0;
    crate::statistics::statistics_print(solver, verbose);
    if solver.proof.is_some() {
        crate::print::section(solver, "proof");
        crate::proof::print_proof_statistics(solver, verbose);
    }
    crate::print::section(solver, "glue usage");
    crate::statistics::print_glue_usage(solver);
    crate::print::section(solver, "resources");
    crate::resources::print_resources(solver);
}

/// Port of `kissat_add` — external clause addition.
///
/// With NDEBUG on and NPROOFS off: `checking` is 0, `logging` is false and
/// `proving` is `solver.proof.is_some()`; the original-literal bookkeeping is
/// compiled in and used for proof deletion of shrunken/satisfied originals.
pub fn add(solver: &mut Solver, elit: i32) {
    debug_assert!(solver.statistics.searches == 0, "incremental solving not supported");
    let proving = solver.proof.is_some();
    if elit != 0 {
        debug_assert!(
            elit != i32::MIN && elit.unsigned_abs() <= crate::literal::EXTERNAL_MAX_VAR as u32
        );
        if proving {
            solver.original.push(elit);
        }
        let ilit = crate::import::import_literal(solver, elit);

        let mark = solver.marks[ilit as usize];
        if mark == 0 {
            let value = fixed(solver, ilit);
            if value > 0 {
                if !solver.clause_satisfied {
                    solver.clause_satisfied = true;
                }
            } else if value < 0 {
                if !solver.clause_shrink {
                    solver.clause_shrink = true;
                }
            } else {
                solver.marks[ilit as usize] = 1;
                solver.marks[(ilit ^ 1) as usize] = -1; // NOT (ilit)
                debug_assert!(solver.clause.len() < u32::MAX as usize);
                solver.clause.push(ilit);
            }
        } else if mark < 0 {
            if !solver.clause_trivial {
                solver.clause_trivial = true;
            }
        } else {
            debug_assert!(mark > 0);
            if !solver.clause_shrink {
                solver.clause_shrink = true;
            }
        }
    } else {
        // PORT NOTE: `esize`/`elits` alias solver.original in C; the stack is
        // temporarily taken to satisfy the borrow checker, with identical
        // order of effects (original is not touched until the tail).
        let original = std::mem::take(&mut solver.original);
        let offset = solver.offset_of_last_original_clause;
        let elits = &original[offset..];
        let esize = elits.len();

        // ADD_UNCHECKED_EXTERNAL — no-op under NDEBUG.

        let isize_ = solver.clause.len();
        debug_assert!(isize_ < i32::MAX as usize);

        if solver.inconsistent {
            // skipping original clause
        } else if solver.clause_satisfied {
            // skipping satisfied original clause
        } else if solver.clause_trivial {
            // skipping trivial original clause
        } else {
            // kissat_activate_literals (solver, isize, ilits)
            // PORT NOTE: take/restore of solver.clause for the aliasing call.
            let ilits = std::mem::take(&mut solver.clause);
            crate::flags::activate_literals(solver, ilits.len() as u32, &ilits);
            solver.clause = ilits;

            if isize_ == 0 {
                if !solver.inconsistent {
                    solver.inconsistent = true;
                    // CHECK_AND_ADD_EMPTY — no-op under NDEBUG.
                    // ADD_EMPTY_TO_PROOF
                    if solver.proof.is_some() {
                        crate::proof::add_empty_to_proof(solver);
                    }
                }
            } else if isize_ == 1 {
                let unit = *solver.clause.last().unwrap(); // TOP_STACK

                crate::assign::original_unit(solver, unit);

                if solver.level == 0 {
                    let _ = crate::propsearch::search_propagate(solver);
                }
            } else {
                let res = crate::clause::new_original_clause(solver);

                let a = solver.clause[0];
                let b = solver.clause[1];

                let u = solver.values[a as usize];
                let v = solver.values[b as usize];

                let k = if u != 0 { solver.assigned[(a >> 1) as usize].level } else { u32::MAX };
                let l = if v != 0 { solver.assigned[(b >> 1) as usize].level } else { u32::MAX };

                let mut assign = false;

                if u == 0 && v < 0 {
                    // original clause immediately forcing
                    assign = true;
                } else if u < 0 && k == l {
                    // both watches falsified at level k
                    debug_assert!(v < 0);
                    debug_assert!(k > 0);
                    crate::backtrack::backtrack_without_updating_phases(solver, k - 1);
                } else if u < 0 {
                    // watches falsified at levels k and l, k > l > 0
                    debug_assert!(v < 0);
                    assign = true;
                } else if u > 0 && v < 0 {
                    // first watch satisfied, second falsified
                } else if u == 0 && v > 0 {
                    // PORT NOTE: quirky C comment says "second falsified" but
                    // the condition is second-satisfied; behavior ported
                    // exactly (assign = true).
                    assign = true;
                } else {
                    debug_assert!(u == 0 && v == 0);
                }

                if assign {
                    debug_assert!(solver.level > 0);
                    if isize_ == 2 {
                        debug_assert!(res == INVALID);
                        crate::assign::assign_binary(solver, a, b);
                    } else {
                        debug_assert!(res != INVALID);
                        // PORT NOTE: C dereferences the clause pointer and
                        // passes it alongside the reference; the Rust
                        // assign_reference re-derives the clause from `res`.
                        crate::assign::assign_reference(solver, a, res);
                    }
                }
            }
        }

        // !NDEBUG || !NPROOFS block — NPROOFS off, so compiled:
        if solver.clause_satisfied || solver.clause_trivial {
            if proving {
                if esize == 1 {
                    // skipping deleting unit from proof
                } else {
                    crate::proof::delete_external_from_proof(solver, elits);
                }
            }
        } else if !solver.inconsistent && solver.clause_shrink {
            if proving {
                // PORT NOTE: take/restore of solver.clause around the
                // aliasing proof call, C effect order preserved.
                let ilits = std::mem::take(&mut solver.clause);
                crate::proof::add_lits_to_proof(solver, &ilits);
                solver.clause = ilits;
                crate::proof::delete_external_from_proof(solver, elits);
            }
        }

        solver.original = original;
        // `checking` is 0 under NDEBUG, so only the logging/proving arm:
        if proving {
            solver.original.clear();
            solver.offset_of_last_original_clause = 0;
        }

        for i in 0..solver.clause.len() {
            let lit = solver.clause[i];
            solver.marks[lit as usize] = 0;
            solver.marks[(lit ^ 1) as usize] = 0;
        }

        solver.clause.clear();

        solver.clause_satisfied = false;
        solver.clause_trivial = false;
        solver.clause_shrink = false;
    }
}

/// Port of `kissat_solve`.
pub fn solve(solver: &mut Solver) -> i32 {
    debug_assert!(
        solver.clause.is_empty(),
        "incomplete clause (terminating zero not added)"
    );
    debug_assert!(solver.statistics.searches == 0, "incremental solving not supported");
    crate::search::search(solver)
}

/// Port of `kissat_terminate`.
pub fn terminate(solver: &mut Solver) {
    solver.termination.flagged.store(true, Ordering::SeqCst);
}

// PORT NOTE: kissat_set_terminate (callback registration through C function
// pointers) is not ported; the AtomicBool `termination.flagged` plus
// crate-level signal handling in main.rs covers the same behavior.

/// Port of `kissat_value` — external value lookup.
pub fn value(solver: &mut Solver, elit: i32) -> i32 {
    debug_assert!(elit != 0 && elit != i32::MIN);
    let eidx = elit.unsigned_abs() as usize; // ABS (elit)
    if eidx >= solver.import_.len() {
        return 0;
    }
    let import = solver.import_[eidx];
    if !import.imported {
        return 0;
    }
    let mut tmp: i8;
    if import.eliminated {
        if !solver.extended && !solver.extend.is_empty() {
            crate::extend::extend(solver);
        }
        let eliminated = import.lit;
        tmp = solver.eliminated[eliminated as usize];
    } else {
        let ilit = import.lit;
        tmp = solver.values[ilit as usize];
    }
    if tmp == 0 {
        return 0;
    }
    if elit < 0 {
        tmp = -tmp;
    }
    if tmp < 0 {
        -elit
    } else {
        elit
    }
}
