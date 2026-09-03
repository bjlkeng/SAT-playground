// Port of src/sweep.c (kissat 4.0.4).
//
// PORT NOTES:
//  - The C `struct sweeper` holds a `kissat *solver` back pointer; the Rust
//    Sweeper drops it and every function takes (solver, sweeper) explicitly.
//  - The kitten lives in `solver.kitten: Option<Box<Kitten>>` exactly as in C
//    (init_sweeper creates it, release_sweeper destroys it).  kitten calls
//    that need `&mut Solver` (statistics/termination) temporarily take the
//    box out of the option and restore it afterwards; statistics effects
//    happen at the same points as in C.  Read-only kitten calls borrow via
//    `solver.kitten.as_ref()/as_mut()`.
//  - CHECK_AND_ADD_* / REMOVE_CHECKER_* are checker macros, compiled out in
//    the NDEBUG reference build.  The proof macros (ADD_*_TO_PROOF /
//    DELETE_*_FROM_PROOF) are compiled in and guard on `solver->proof`
//    internally; ported as explicit `if solver.proof.is_some()` call sites.
//    CHECKING_OR_PROVING is defined (NPROOFS off), so clear_core's deletion
//    loop and substitute_connected_clauses' added/removed clearing are in.
//  - `kissat_propagated` (inline.h) is inlined here as a private helper.
//  - ADD (arena_garbage, ...) is METRIC — compiled out.
//  - sweep_depth/sweep_clauses/sweep_environment/sweep_variables and the
//    sat/unsat/flip breakdown counters are STATISTIC tier: real (unprinted)
//    fields, incremented 1:1.  sweep, sweep_completed, sweep_equivalences,
//    sweep_solved, sweep_units are COUNTERs (parity oracle).
//  - The C gotos (STOP_SWEEP_BACKBONE / STOP_SWEEP_EQUIVALENCES / DONE) are
//    restructured with a `done` flag; profile STOP placement and the skipped
//    verbose prints match the C control flow exactly.

use crate::internal::{Solver, INVALID};
use crate::kimits::DelayId;
use crate::kitten;
use crate::kitten::Kitten;
use crate::literal::{idx as IDX, lit as LIT, not as NOT};
use crate::profile::Prof;
use crate::reference::{Reference, INVALID_REF};
use crate::watch::{binary_watch, watch_is_binary, watch_lit, watch_ref};

const INVALID_IDX: u32 = INVALID;
const INVALID_LIT: u32 = INVALID;

struct SweeperLimit {
    ticks: u64,
    clauses: u32,
    depth: u32,
    vars: u32,
}

/// Port of `struct sweeper` (without the `kissat *solver` back pointer).
pub struct Sweeper {
    depths: Vec<u32>, // per var
    reprs: Vec<u32>,  // per lit
    next: Vec<u32>,   // per var
    prev: Vec<u32>,   // per var
    first: u32,
    last: u32,
    encoded: u32,
    save: u32,
    vars: Vec<u32>,
    refs: Vec<Reference>,
    clause: Vec<u32>,
    backbone: Vec<u32>,
    partition: Vec<u32>,
    core: [Vec<u32>; 2],
    limit: SweeperLimit,
}

/// kissat_propagated (inline.h): propagate cursor caught up with the trail.
#[inline]
fn propagated(solver: &Solver) -> bool {
    solver.propagate == solver.trail.len()
}

fn sweep_solve(solver: &mut Solver) -> i32 {
    let mut k = solver.kitten.take().expect("kitten");
    kitten::kitten_randomize_phases(&mut k);
    solver.statistics.sweep_solved += 1; // INC (sweep_solved) — COUNTER
    let res = kitten::kitten_solve(&mut k, solver);
    solver.kitten = Some(k);
    if res == 10 {
        solver.statistics.sweep_sat += 1; // STATISTIC
    }
    if res == 20 {
        solver.statistics.sweep_unsat += 1; // STATISTIC
    }
    res
}

fn set_kitten_ticks_limit(solver: &mut Solver, sweeper: &Sweeper) {
    let mut remaining = 0u64;
    if solver.statistics.kitten_ticks < sweeper.limit.ticks {
        remaining = sweeper.limit.ticks - solver.statistics.kitten_ticks;
    }
    let mut k = solver.kitten.take().expect("kitten");
    kitten::kitten_set_ticks_limit(&mut k, solver, remaining);
    solver.kitten = Some(k);
}

fn kitten_ticks_limit_hit(solver: &Solver, sweeper: &Sweeper, _when: &str) -> bool {
    solver.statistics.kitten_ticks >= sweeper.limit.ticks
}

fn init_sweeper(solver: &mut Solver) -> Sweeper {
    let vars = solver.vars as usize;
    let lits = solver.lits() as usize;
    let mut sweeper = Sweeper {
        depths: vec![0; vars],                 // CALLOC (depths, VARS)
        reprs: (0..lits as u32).collect(),     // reprs[lit] = lit
        prev: vec![INVALID_IDX; vars],         // memset 0xff
        next: vec![INVALID_IDX; vars],         // memset 0xff
        first: INVALID_IDX,
        last: INVALID_IDX,
        encoded: 0,
        save: 0,
        vars: Vec::new(),
        refs: Vec::new(),
        clause: Vec::new(),
        backbone: Vec::new(),
        partition: Vec::new(),
        core: [Vec::new(), Vec::new()],
        limit: SweeperLimit {
            ticks: 0,
            clauses: 0,
            depth: 0,
            vars: 0,
        },
    };
    debug_assert!(solver.kitten.is_none());
    solver.kitten = Some(kitten::kitten_embedded());
    kitten::kitten_track_antecedents(solver.kitten.as_mut().unwrap());
    crate::dense::enter_dense_mode(solver, None);
    crate::watch::connect_irredundant_large_clauses(solver);

    // C: `unsigned completed = solver->statistics.sweep_completed;`
    // (truncating u64 → unsigned assignment kept).
    let mut completed = solver.statistics.sweep_completed as u32;
    const MAX_COMPLETED: u32 = 32;
    if completed > MAX_COMPLETED {
        completed = MAX_COMPLETED;
    }

    let mut vars_limit: u64 = solver.options.sweepvars as u64;
    vars_limit <<= completed;
    let max_vars_limit = solver.options.sweepmaxvars as u64;
    if vars_limit > max_vars_limit {
        vars_limit = max_vars_limit;
    }
    sweeper.limit.vars = vars_limit as u32;
    crate::print::extremely_verbose(
        solver,
        format!("sweeper variable limit {}", sweeper.limit.vars),
    );

    let mut depth_limit: u64 = solver.statistics.sweep_completed;
    depth_limit += solver.options.sweepdepth as u64;
    let max_depth = solver.options.sweepmaxdepth as u64;
    if depth_limit > max_depth {
        depth_limit = max_depth;
    }
    sweeper.limit.depth = depth_limit as u32;
    crate::print::extremely_verbose(
        solver,
        format!("sweeper depth limit {}", sweeper.limit.depth),
    );

    let mut clause_limit: u64 = solver.options.sweepclauses as u64;
    clause_limit <<= completed;
    let max_clause_limit = solver.options.sweepmaxclauses as u64;
    if clause_limit > max_clause_limit {
        clause_limit = max_clause_limit;
    }
    sweeper.limit.clauses = clause_limit as u32;
    crate::print::extremely_verbose(
        solver,
        format!("sweeper clause limit {}", sweeper.limit.clauses),
    );

    if solver.options.sweepcomplete != 0 {
        sweeper.limit.ticks = u64::MAX;
        crate::print::extremely_verbose(solver, "unlimited sweeper ticks limit");
    } else {
        let ticks_limit = crate::set_effort_limit!(solver, sweep, sweepeffort, kitten_ticks);
        sweeper.limit.ticks = ticks_limit;
    }
    set_kitten_ticks_limit(solver, &sweeper);
    sweeper
}

fn release_sweeper(solver: &mut Solver, sweeper: &mut Sweeper) -> u32 {
    let mut merged = 0u32;
    for idx in 0..solver.vars {
        if !solver.flags[idx as usize].active {
            continue;
        }
        let lit = LIT(idx);
        if sweeper.reprs[lit as usize] != lit {
            merged += 1;
        }
    }
    // DEALLOC / RELEASE_STACK: dropped with the Sweeper.
    let k = solver.kitten.take().expect("kitten");
    kitten::kitten_release(k); // solver->kitten = 0
    crate::dense::resume_sparse_mode(solver, false, None);
    merged
}

fn clear_sweeper(solver: &mut Solver, sweeper: &mut Sweeper) {
    {
        let k = solver.kitten.as_mut().unwrap();
        kitten::kitten_clear(k);
        kitten::kitten_track_antecedents(k);
    }
    for i in 0..sweeper.vars.len() {
        let idx = sweeper.vars[i];
        debug_assert!(sweeper.depths[idx as usize] != 0);
        sweeper.depths[idx as usize] = 0;
    }
    sweeper.vars.clear();
    for i in 0..sweeper.refs.len() {
        let ref_ = sweeper.refs[i];
        let mut c = solver.arena.clause_mut(ref_);
        debug_assert!(c.as_ref().swept());
        c.set_swept(false);
    }
    sweeper.refs.clear();
    sweeper.backbone.clear();
    sweeper.partition.clear();
    sweeper.encoded = 0;
    set_kitten_ticks_limit(solver, sweeper);
}

fn sweep_repr(sweeper: &mut Sweeper, lit: u32) -> u32 {
    let mut res;
    {
        let mut prev = lit;
        loop {
            res = sweeper.reprs[prev as usize];
            if res == prev {
                break;
            }
            prev = res;
        }
    }
    if res == lit {
        return res;
    }
    {
        let not_res = NOT(res);
        let mut prev = lit;
        loop {
            let next = sweeper.reprs[prev as usize];
            if next == res {
                break;
            }
            let not_prev = NOT(prev);
            sweeper.reprs[not_prev as usize] = not_res;
            sweeper.reprs[prev as usize] = res;
            prev = next;
        }
        debug_assert!(sweeper.reprs[NOT(prev) as usize] == not_res);
    }
    res
}

fn add_literal_to_environment(sweeper: &mut Sweeper, depth: u32, lit: u32) {
    let repr = sweep_repr(sweeper, lit);
    if repr != lit {
        return;
    }
    let idx = IDX(lit);
    if sweeper.depths[idx as usize] != 0 {
        return;
    }
    debug_assert!(depth < u32::MAX);
    sweeper.depths[idx as usize] = depth + 1;
    sweeper.vars.push(idx);
}

fn sweep_clause(solver: &mut Solver, sweeper: &mut Sweeper, depth: u32) {
    debug_assert!(sweeper.clause.len() > 1);
    for i in 0..sweeper.clause.len() {
        let lit = sweeper.clause[i];
        add_literal_to_environment(sweeper, depth, lit);
    }
    {
        let mut k = solver.kitten.take().expect("kitten");
        kitten::kitten_clause(&mut k, solver, &sweeper.clause);
        solver.kitten = Some(k);
    }
    sweeper.clause.clear();
    sweeper.encoded += 1;
}

fn sweep_binary(solver: &mut Solver, sweeper: &mut Sweeper, depth: u32, lit: u32, other: u32) {
    if sweep_repr(sweeper, lit) != lit {
        return;
    }
    if sweep_repr(sweeper, other) != other {
        return;
    }
    debug_assert!(solver.values[lit as usize] == 0);
    let other_value = solver.values[other as usize];
    if other_value > 0 {
        return; // skipping satisfied
    }
    let other_idx = IDX(other);
    let other_depth = sweeper.depths[other_idx as usize];
    let lit_idx = IDX(lit);
    let lit_depth = sweeper.depths[lit_idx as usize];
    if other_depth != 0 && other_depth < lit_depth {
        return; // skipping depth copied
    }
    debug_assert!(other_value == 0);
    debug_assert!(sweeper.clause.is_empty());
    sweeper.clause.push(lit);
    sweeper.clause.push(other);
    sweep_clause(solver, sweeper, depth);
}

fn sweep_reference(solver: &mut Solver, sweeper: &mut Sweeper, depth: u32, ref_: Reference) {
    debug_assert!(sweeper.clause.is_empty());
    let (swept, garbage, size) = {
        let c = solver.arena.clause(ref_);
        (c.swept(), c.garbage(), c.size())
    };
    if swept {
        return;
    }
    if garbage {
        return;
    }
    for i in 0..size {
        let lit = solver.arena.clause(ref_).lit(i);
        let value = solver.values[lit as usize];
        if value > 0 {
            crate::clause::mark_clause_as_garbage(solver, ref_);
            sweeper.clause.clear();
            return;
        }
        if value < 0 {
            continue;
        }
        sweeper.clause.push(lit);
    }
    sweeper.refs.push(ref_);
    solver.arena.clause_mut(ref_).set_swept(true);
    sweep_clause(solver, sweeper, depth);
}

/// save_core_clause: the kitten core traversal callback.
fn save_core_clause(solver: &Solver, sweeper: &mut Sweeper, learned: bool, lits: &[u32]) {
    if solver.inconsistent {
        return;
    }
    let core = &mut sweeper.core[sweeper.save as usize];
    let saved = core.len();
    let mut non_false = 0u32;
    for &lit in lits {
        let value = solver.values[lit as usize];
        if value > 0 {
            core.truncate(saved); // RESIZE_STACK (*core, saved)
            return;
        }
        core.push(lit);
        if value < 0 {
            continue;
        }
        if !learned {
            non_false += 1;
            if non_false > 1 {
                core.truncate(saved);
                return;
            }
        }
    }
    core.push(INVALID_LIT);
}

fn add_core(solver: &mut Solver, sweeper: &mut Sweeper, core_idx: u32) {
    if solver.inconsistent {
        return;
    }
    debug_assert!(core_idx == 0 || core_idx == 1);
    let mut core = std::mem::take(&mut sweeper.core[core_idx as usize]);

    let end_core = core.len();
    let mut q: usize = 0;
    let mut p: usize = 0;

    while p != end_core {
        let c = p;
        while core[p] != INVALID_LIT {
            p += 1;
        }
        let mut satisfied = false;
        let mut unit = INVALID_LIT;

        let d = q;

        let mut l = c;
        while !satisfied && l != p {
            let lit = core[l];
            let value = solver.values[lit as usize];
            if value > 0 {
                satisfied = true;
                break;
            }
            if value == 0 {
                unit = lit;
                core[q] = lit;
                q += 1;
            }
            l += 1;
        }

        let new_size = q - d;
        p += 1;

        if satisfied {
            q = d;
            continue; // not adding satisfied clause
        }

        if new_size == 0 {
            // CHECK_AND_ADD_EMPTY (): compiled out.
            if solver.proof.is_some() {
                crate::proof::add_empty_to_proof(solver);
            }
            solver.inconsistent = true;
            core.clear();
            sweeper.core[core_idx as usize] = core;
            return;
        }

        if new_size == 1 {
            q = d;
            debug_assert!(unit != INVALID_LIT);
            // CHECK_AND_ADD_UNIT (unit): compiled out.
            if solver.proof.is_some() {
                crate::proof::add_unit_to_proof(solver, unit);
            }
            crate::assign::assign_unit(solver, unit, "sweeping backbone reason");
            solver.statistics.sweep_units += 1; // INC — COUNTER
            continue;
        }

        core[q] = INVALID_LIT;
        q += 1;

        debug_assert!(new_size > 1);
        // CHECK_AND_ADD_LITS: compiled out.
        if solver.proof.is_some() {
            // ADD_LITS_TO_PROOF (new_size, d): `core` is detached from the
            // sweeper here, so the slice can be passed directly.
            crate::proof::add_lits_to_proof(solver, &core[d..d + new_size]);
        }
    }
    core.truncate(q); // SET_END_OF_STACK (*core, q)
    sweeper.core[core_idx as usize] = core;
}

fn save_core(solver: &mut Solver, sweeper: &mut Sweeper, core: u32) {
    debug_assert!(core == 0 || core == 1);
    debug_assert!(sweeper.core[core as usize].is_empty());
    sweeper.save = core;
    let mut k = solver.kitten.take().expect("kitten");
    let _ = kitten::kitten_compute_clausal_core(&mut k); // learned ptr NULL
    kitten::kitten_traverse_core_clauses(&mut k, |learned, lits| {
        save_core_clause(solver, sweeper, learned, lits)
    });
    solver.kitten = Some(k);
}

fn clear_core(solver: &mut Solver, sweeper: &mut Sweeper, core_idx: u32) {
    if solver.inconsistent {
        return;
    }
    debug_assert!(core_idx == 0 || core_idx == 1);
    let mut core = std::mem::take(&mut sweeper.core[core_idx as usize]);
    // #ifdef CHECKING_OR_PROVING (defined: NPROOFS off) — delete the
    // sub-solver core clauses from the proof.
    {
        let end = core.len();
        let mut c: usize = 0;
        let mut p: usize = 0;
        while c != end {
            while core[p] != INVALID_LIT {
                p += 1;
            }
            let size = p - c;
            debug_assert!(size > 1);
            // REMOVE_CHECKER_LITS: compiled out.
            if solver.proof.is_some() {
                crate::proof::delete_internal_from_proof(solver, &core[c..p]);
            }
            p += 1;
            c = p;
        }
    }
    core.clear();
    sweeper.core[core_idx as usize] = core;
}

fn save_add_clear_core(solver: &mut Solver, sweeper: &mut Sweeper) {
    save_core(solver, sweeper, 0);
    add_core(solver, sweeper, 0);
    clear_core(solver, sweeper, 0);
}

fn init_backbone_and_partition(solver: &mut Solver, sweeper: &mut Sweeper) {
    let kitten_ref: &Kitten = solver.kitten.as_ref().unwrap();
    for i in 0..sweeper.vars.len() {
        let idx = sweeper.vars[i];
        if !solver.flags[idx as usize].active {
            continue;
        }
        let lit = LIT(idx);
        let not_lit = NOT(lit);
        let tmp = kitten::kitten_value(kitten_ref, lit);
        let candidate = if tmp < 0 { not_lit } else { lit };
        sweeper.backbone.push(candidate);
        sweeper.partition.push(candidate);
    }
    sweeper.partition.push(INVALID_LIT);
}

fn sweep_empty_clause(solver: &mut Solver, sweeper: &mut Sweeper) {
    debug_assert!(!solver.inconsistent);
    save_add_clear_core(solver, sweeper);
    debug_assert!(solver.inconsistent);
}

fn sweep_refine_partition(solver: &mut Solver, sweeper: &mut Sweeper) {
    let old_partition = std::mem::take(&mut sweeper.partition);
    let mut new_partition: Vec<u32> = Vec::new();
    // The kitten is only read (kitten_value); take it out so sweep_repr can
    // borrow sweeper mutably alongside.
    let kitten_box = solver.kitten.take().expect("kitten");
    let kitten_ref: &Kitten = &kitten_box;
    let old_end = old_partition.len();
    let mut p: usize = 0;
    while p != old_end {
        let mut q = p;
        let mut assigned_true = 0u32;
        loop {
            let other = old_partition[q];
            if other == INVALID_LIT {
                break;
            }
            if sweep_repr(sweeper, other) == other
                && solver.values[other as usize] == 0
            {
                let value = kitten::kitten_value(kitten_ref, other);
                if value > 0 {
                    new_partition.push(other);
                    assigned_true += 1;
                }
            }
            q += 1;
        }
        if assigned_true == 0 {
            // no positive literal in class
        } else if assigned_true == 1 {
            new_partition.pop();
        } else {
            new_partition.push(INVALID_LIT);
        }

        let mut assigned_false = 0u32;
        let mut r = p;
        loop {
            let other = old_partition[r];
            if other == INVALID_LIT {
                break;
            }
            if sweep_repr(sweeper, other) == other
                && solver.values[other as usize] == 0
            {
                let value = kitten::kitten_value(kitten_ref, other);
                if value < 0 {
                    new_partition.push(other);
                    assigned_false += 1;
                }
            }
            r += 1;
        }

        if assigned_false == 0 {
            // no negative literal in class
        } else if assigned_false == 1 {
            new_partition.pop();
        } else {
            new_partition.push(INVALID_LIT);
        }

        p = q + 1;
    }
    solver.kitten = Some(kitten_box);
    // RELEASE_STACK (old_partition); sweeper->partition = new_partition;
    drop(old_partition);
    sweeper.partition = new_partition;
}

fn sweep_refine_backbone(solver: &mut Solver, sweeper: &mut Sweeper) {
    let mut backbone = std::mem::take(&mut sweeper.backbone);
    let kitten_ref: &Kitten = solver.kitten.as_ref().unwrap();
    let end = backbone.len();
    let mut q: usize = 0;
    for p in 0..end {
        let lit = backbone[p];
        if solver.values[lit as usize] != 0 {
            continue;
        }
        let value = kitten::kitten_value(kitten_ref, lit);
        if value == 0 {
            // dropping sub-solver unassigned
        } else if value >= 0 {
            backbone[q] = lit;
            q += 1;
        }
    }
    backbone.truncate(q); // SET_END_OF_STACK
    sweeper.backbone = backbone;
}

fn sweep_refine(solver: &mut Solver, sweeper: &mut Sweeper) {
    if !sweeper.backbone.is_empty() {
        sweep_refine_backbone(solver, sweeper);
    }
    if !sweeper.partition.is_empty() {
        sweep_refine_partition(solver, sweeper);
    }
}

fn flip_backbone_literals(solver: &mut Solver, sweeper: &mut Sweeper) {
    let max_rounds = solver.options.sweepfliprounds as u32;
    if max_rounds == 0 {
        return;
    }
    debug_assert!(!sweeper.backbone.is_empty());
    if kitten::kitten_status(solver.kitten.as_ref().unwrap()) != 10 {
        return;
    }
    let mut kitten = solver.kitten.take().expect("kitten");
    let mut round = 0u32;
    loop {
        round += 1;
        let mut flipped = 0u32;
        let end = sweeper.backbone.len();
        let mut q: usize = 0;
        let mut p: usize = 0;
        while p != end {
            let lit = sweeper.backbone[p];
            p += 1;
            solver.statistics.sweep_flip_backbone += 1; // INC — STATISTIC
            if kitten::kitten_flip_literal(&mut kitten, solver, lit) {
                solver.statistics.sweep_flipped_backbone += 1; // STATISTIC
                flipped += 1;
            } else {
                sweeper.backbone[q] = lit;
                q += 1;
            }
        }
        sweeper.backbone.truncate(q); // SET_END_OF_STACK

        if crate::terminated!(solver, sweep_terminated_1) {
            break;
        }
        if solver.statistics.kitten_ticks > sweeper.limit.ticks {
            break;
        }
        if !(flipped != 0 && round < max_rounds) {
            break;
        }
    }
    solver.kitten = Some(kitten);
}

fn sweep_backbone_candidate(solver: &mut Solver, sweeper: &mut Sweeper, lit: u32) -> bool {
    let value = kitten::kitten_fixed(solver.kitten.as_ref().unwrap(), lit);
    if value != 0 {
        solver.statistics.sweep_fixed_backbone += 1; // STATISTIC
        debug_assert!(value > 0);
        return false;
    }

    solver.statistics.sweep_flip_backbone += 1; // STATISTIC
    let flipped = {
        let mut k = solver.kitten.take().expect("kitten");
        let res =
            kitten::kitten_status(&k) == 10 && kitten::kitten_flip_literal(&mut k, solver, lit);
        solver.kitten = Some(k);
        res
    };
    if flipped {
        solver.statistics.sweep_flipped_backbone += 1; // STATISTIC
        return false;
    }

    let not_lit = NOT(lit);
    solver.statistics.sweep_solved_backbone += 1; // STATISTIC
    kitten::kitten_assume(solver.kitten.as_mut().unwrap(), not_lit);
    let res = sweep_solve(solver);
    if res == 10 {
        sweep_refine(solver, sweeper);
        solver.statistics.sweep_sat_backbone += 1; // STATISTIC
        return false;
    }

    if res == 20 {
        save_add_clear_core(solver, sweeper);
        solver.statistics.sweep_unsat_backbone += 1; // STATISTIC
        return true;
    }

    solver.statistics.sweep_unknown_backbone += 1; // STATISTIC
    false
}

fn add_binary(solver: &mut Solver, lit: u32, other: u32) {
    crate::clause::new_binary_clause(solver, lit, other);
}

fn scheduled_variable(sweeper: &Sweeper, idx: u32) -> bool {
    sweeper.prev[idx as usize] != INVALID_IDX || sweeper.first == idx
}

fn schedule_inner(solver: &Solver, sweeper: &mut Sweeper, idx: u32) {
    if !solver.flags[idx as usize].active {
        return;
    }
    let next = sweeper.next[idx as usize];
    if next != INVALID_IDX {
        // rescheduling inner as last
        let prev = sweeper.prev[idx as usize];
        debug_assert!(sweeper.prev[next as usize] == idx);
        sweeper.prev[next as usize] = prev;
        if prev == INVALID_IDX {
            debug_assert!(sweeper.first == idx);
            sweeper.first = next;
        } else {
            debug_assert!(sweeper.next[prev as usize] == idx);
            sweeper.next[prev as usize] = next;
        }
        let last = sweeper.last;
        if last == INVALID_IDX {
            debug_assert!(sweeper.first == INVALID_IDX);
            sweeper.first = idx;
        } else {
            debug_assert!(sweeper.next[last as usize] == INVALID_IDX);
            sweeper.next[last as usize] = idx;
        }
        sweeper.prev[idx as usize] = last;
        sweeper.next[idx as usize] = INVALID_IDX;
        sweeper.last = idx;
    } else if sweeper.last != idx {
        // scheduling inner as last
        let last = sweeper.last;
        if last == INVALID_IDX {
            debug_assert!(sweeper.first == INVALID_IDX);
            sweeper.first = idx;
        } else {
            debug_assert!(sweeper.next[last as usize] == INVALID_IDX);
            sweeper.next[last as usize] = idx;
        }
        debug_assert!(sweeper.next[idx as usize] == INVALID_IDX);
        sweeper.prev[idx as usize] = last;
        sweeper.last = idx;
    }
    // else: keeping inner scheduled as last
}

fn schedule_outer(sweeper: &mut Sweeper, idx: u32) {
    debug_assert!(!scheduled_variable(sweeper, idx));
    let first = sweeper.first;
    if first == INVALID_IDX {
        debug_assert!(sweeper.last == INVALID_IDX);
        sweeper.last = idx;
    } else {
        debug_assert!(sweeper.prev[first as usize] == INVALID_IDX);
        sweeper.prev[first as usize] = idx;
    }
    debug_assert!(sweeper.prev[idx as usize] == INVALID_IDX);
    sweeper.next[idx as usize] = first;
    sweeper.first = idx;
}

fn next_scheduled(sweeper: &mut Sweeper) -> u32 {
    let res = sweeper.last;
    if res == INVALID_IDX {
        return INVALID_IDX;
    }
    let prev = sweeper.prev[res as usize];
    debug_assert!(sweeper.next[res as usize] == INVALID_IDX);
    sweeper.prev[res as usize] = INVALID_IDX;
    if prev == INVALID_IDX {
        debug_assert!(sweeper.first == res);
        sweeper.first = INVALID_IDX;
    } else {
        debug_assert!(sweeper.next[prev as usize] == res);
        sweeper.next[prev as usize] = INVALID_IDX;
    }
    sweeper.last = prev;
    res
}

fn substitute_connected_clauses(
    solver: &mut Solver,
    sweeper: &mut Sweeper,
    lit: u32,
    repr: u32,
) {
    if solver.inconsistent {
        return;
    }
    if solver.values[lit as usize] != 0 {
        return;
    }
    if solver.values[repr as usize] != 0 {
        return;
    }

    debug_assert!(lit != repr);
    debug_assert!(lit != NOT(repr));

    let checking_or_proving = solver.proof.is_some(); // kissat_checking_or_proving
    debug_assert!(solver.added.is_empty());
    debug_assert!(solver.removed.is_empty());

    debug_assert!(solver.delayed.is_empty());

    {
        let v = solver.watches[lit as usize];
        let begin_watches = v.begin;
        let end_watches = v.end;

        let mut q = begin_watches;
        let mut p = q;

        while p != end_watches {
            let head = solver.vectors.stack[p];
            solver.vectors.stack[q] = head;
            q += 1;
            p += 1;
            if watch_is_binary(head) {
                let other = watch_lit(head);
                let other_value = solver.values[other as usize];
                if other == NOT(repr) {
                    continue;
                }
                if other_value < 0 {
                    break;
                }
                if other_value > 0 {
                    continue;
                }
                if other == repr {
                    // CHECK_AND_ADD_UNIT (lit): compiled out.
                    if solver.proof.is_some() {
                        crate::proof::add_unit_to_proof(solver, lit);
                    }
                    crate::assign::assign_unit(solver, lit, "substituted binary clause");
                    solver.statistics.sweep_units += 1; // INC — COUNTER
                    break;
                }
                // CHECK_AND_ADD_BINARY / REMOVE_CHECKER_BINARY: compiled out.
                if solver.proof.is_some() {
                    crate::proof::add_binary_to_proof(solver, repr, other);
                    crate::proof::delete_binary_from_proof(solver, lit, other);
                }
                solver.delayed.push(head); // PUSH_STACK (*delayed, head.raw)
                let src = binary_watch(lit); // src.binary.lit = lit
                let dst = binary_watch(repr); // dst.binary.lit = repr
                crate::watch::substitute_large_watch(solver, other, src, dst);
                q -= 1;
            } else {
                let ref_ = watch_ref(head);
                debug_assert!(sweeper.clause.is_empty());
                if solver.arena.clause(ref_).garbage() {
                    continue;
                }

                let mut satisfied = false;
                let mut repr_already_watched = false;
                let not_repr = NOT(repr);
                let size = solver.arena.clause(ref_).size();
                for i in 0..size {
                    let other = solver.arena.clause(ref_).lit(i);
                    if other == lit {
                        solver.clause.push(repr);
                        continue;
                    }
                    debug_assert!(other != NOT(lit));
                    if other == repr {
                        debug_assert!(!repr_already_watched);
                        repr_already_watched = true;
                        continue;
                    }
                    if other == not_repr {
                        satisfied = true;
                        break;
                    }
                    let tmp = solver.values[other as usize];
                    if tmp < 0 {
                        continue;
                    }
                    if tmp > 0 {
                        satisfied = true;
                        break;
                    }
                    solver.clause.push(other);
                }

                if satisfied {
                    solver.clause.clear();
                    crate::clause::mark_clause_as_garbage(solver, ref_);
                    continue;
                }

                let new_size = solver.clause.len() as u32;

                if new_size == 0 {
                    debug_assert!(!solver.inconsistent);
                    solver.inconsistent = true;
                    // CHECK_AND_ADD_EMPTY: compiled out.
                    if solver.proof.is_some() {
                        crate::proof::add_empty_to_proof(solver);
                    }
                    break;
                }

                if new_size == 1 {
                    let unit = solver.clause.pop().unwrap();
                    // CHECK_AND_ADD_UNIT: compiled out.
                    if solver.proof.is_some() {
                        crate::proof::add_unit_to_proof(solver, unit);
                    }
                    crate::assign::assign_unit(solver, unit, "substituted large clause");
                    solver.statistics.sweep_units += 1; // INC — COUNTER
                    break;
                }

                // CHECK_AND_ADD_STACK / REMOVE_CHECKER_CLAUSE: compiled out.
                if solver.proof.is_some() {
                    // ADD_STACK_TO_PROOF (solver->clause):
                    let ilits = std::mem::take(&mut solver.clause);
                    crate::proof::add_lits_to_proof(solver, &ilits);
                    solver.clause = ilits;
                    // DELETE_CLAUSE_FROM_PROOF (c):
                    crate::proof::delete_clause_from_proof(solver, ref_);
                }

                if !solver.arena.clause(ref_).redundant() {
                    let ilits = std::mem::take(&mut solver.clause);
                    crate::flags::mark_added_literals(solver, new_size, &ilits);
                    solver.clause = ilits;
                }

                if new_size == 2 {
                    let second = solver.clause.pop().unwrap();
                    let first = solver.clause.pop().unwrap();
                    debug_assert!(first == repr || second == repr);
                    let other = first ^ second ^ repr;
                    let src = head; // src.raw = head.raw
                    let dst = binary_watch(repr); // kissat_binary_watch (repr)
                    crate::watch::substitute_large_watch(solver, other, src, dst);
                    debug_assert!(solver.statistics.clauses_irredundant != 0);
                    solver.statistics.clauses_irredundant -= 1;
                    debug_assert!(solver.statistics.clauses_binary < u64::MAX);
                    solver.statistics.clauses_binary += 1;
                    let dst = binary_watch(other); // dst.binary.lit = other
                    solver.delayed.push(dst);
                    // ADD (arena_garbage, bytes): METRIC, compiled out.
                    solver.arena.clause_mut(ref_).set_garbage(true);
                    q -= 1;
                    continue;
                }

                debug_assert!(2 < new_size);
                let old_size = solver.arena.clause(ref_).size();
                debug_assert!(new_size <= old_size);

                for i in 0..new_size {
                    let other = solver.clause[i as usize];
                    solver.arena.clause_mut(ref_).set_lit(i, other);
                }

                if new_size < old_size {
                    {
                        let mut c = solver.arena.clause_mut(ref_);
                        c.set_size(new_size);
                        c.set_searched(2);
                    }
                    let (redundant, glue) = {
                        let c = solver.arena.clause(ref_);
                        (c.redundant(), c.glue())
                    };
                    if redundant && glue >= new_size {
                        crate::promote::promote_clause(solver, ref_, new_size - 1);
                    }
                    if !solver.arena.clause(ref_).shrunken() {
                        let mut c = solver.arena.clause_mut(ref_);
                        c.set_shrunken(true);
                        c.set_lit(old_size - 1, INVALID_LIT);
                    }
                }

                if !repr_already_watched {
                    solver.delayed.push(head);
                }
                solver.clause.clear();
                q -= 1;
            }
        }
        while p != end_watches {
            let head = solver.vectors.stack[p];
            solver.vectors.stack[q] = head;
            q += 1;
            p += 1;
        }
        solver.watches[lit as usize].end = q; // SET_END_OF_WATCHES
    }
    {
        let delayed = std::mem::take(&mut solver.delayed);
        for &head in delayed.iter() {
            crate::vector::push_vectors(solver, repr, head); // PUSH_WATCHES
        }
        solver.delayed = delayed;
        solver.delayed.clear();
    }

    if checking_or_proving {
        solver.added.clear();
        solver.removed.clear();
    }
}

fn sweep_remove(sweeper: &mut Sweeper, lit: u32) {
    debug_assert!(sweeper.reprs[lit as usize] != lit);
    let partition = &mut sweeper.partition;
    let end_partition = partition.len();
    let mut p: usize = 0;
    while partition[p] != lit {
        debug_assert!(p + 1 != end_partition);
        p += 1;
    }
    let mut begin_class = p;
    while begin_class != 0 && partition[begin_class - 1] != INVALID_LIT {
        begin_class -= 1;
    }
    let mut end_class = p;
    while partition[end_class] != INVALID_LIT {
        end_class += 1;
    }
    let size = end_class - begin_class;
    debug_assert!(size > 1);
    let mut q = begin_class;
    if size == 2 {
        // completely squashing equivalence class
        let mut r = end_class + 1;
        while r != end_partition {
            partition[q] = partition[r];
            q += 1;
            r += 1;
        }
    } else {
        let mut r = begin_class;
        while r != end_partition {
            if r != p {
                partition[q] = partition[r];
                q += 1;
            }
            r += 1;
        }
    }
    partition.truncate(q); // SET_END_OF_STACK
}

fn flip_partition_literals(solver: &mut Solver, sweeper: &mut Sweeper) {
    let max_rounds = solver.options.sweepfliprounds as u32;
    if max_rounds == 0 {
        return;
    }
    debug_assert!(!sweeper.partition.is_empty());
    if kitten::kitten_status(solver.kitten.as_ref().unwrap()) != 10 {
        return;
    }
    let mut kitten = solver.kitten.take().expect("kitten");
    let mut round = 0u32;
    loop {
        round += 1;
        let mut flipped = 0u32;
        let end = sweeper.partition.len();
        let mut dst: usize = 0;
        let mut src: usize = 0;
        while src != end {
            let mut end_src = src;
            loop {
                debug_assert!(end_src != end);
                if sweeper.partition[end_src] == INVALID_LIT {
                    break;
                }
                end_src += 1;
            }
            let mut size = (end_src - src) as u32;
            debug_assert!(size > 1);
            let mut q = dst;
            let mut p = src;
            while p != end_src {
                let lit = sweeper.partition[p];
                p += 1;
                if kitten::kitten_flip_literal(&mut kitten, solver, lit) {
                    flipped += 1;
                    size -= 1;
                    if size < 2 {
                        break;
                    }
                } else {
                    sweeper.partition[q] = lit;
                    q += 1;
                }
            }
            if size > 1 {
                sweeper.partition[q] = INVALID_LIT;
                q += 1;
                dst = q;
            }
            src = end_src + 1;
        }
        sweeper.partition.truncate(dst); // SET_END_OF_STACK

        if crate::terminated!(solver, sweep_terminated_2) {
            break;
        }
        if solver.statistics.kitten_ticks > sweeper.limit.ticks {
            break;
        }
        if !(flipped != 0 && round < max_rounds) {
            break;
        }
    }
    solver.kitten = Some(kitten);
}

fn sweep_equivalence_candidates(
    solver: &mut Solver,
    sweeper: &mut Sweeper,
    lit: u32,
    other: u32,
) -> bool {
    let not_other = NOT(other);
    let not_lit = NOT(lit);
    let n = sweeper.partition.len();
    debug_assert!(n >= 3);
    debug_assert!(sweeper.partition[n - 3] == lit);
    debug_assert!(sweeper.partition[n - 2] == other);
    let third = if n == 3 {
        INVALID_LIT
    } else {
        sweeper.partition[n - 4]
    };
    let status = kitten::kitten_status(solver.kitten.as_ref().unwrap());
    if status == 10 {
        let flipped_lit = {
            let mut k = solver.kitten.take().expect("kitten");
            let res = kitten::kitten_flip_literal(&mut k, solver, lit);
            solver.kitten = Some(k);
            res
        };
        if flipped_lit {
            solver.statistics.sweep_flip_equivalences += 1; // STATISTIC
            solver.statistics.sweep_flipped_equivalences += 1; // STATISTIC
            if third == INVALID_LIT {
                sweeper.partition.truncate(n - 3);
            } else {
                sweeper.partition[n - 3] = other;
                sweeper.partition[n - 2] = INVALID_LIT;
                sweeper.partition.truncate(n - 1);
            }
            return false;
        }
        let flipped_other = {
            let mut k = solver.kitten.take().expect("kitten");
            let res = kitten::kitten_flip_literal(&mut k, solver, other);
            solver.kitten = Some(k);
            res
        };
        if flipped_other {
            solver.statistics.sweep_flip_equivalences += 2; // ADD (.., 2)
            solver.statistics.sweep_flipped_equivalences += 1; // STATISTIC
            if third == INVALID_LIT {
                sweeper.partition.truncate(n - 3);
            } else {
                sweeper.partition[n - 2] = INVALID_LIT;
                sweeper.partition.truncate(n - 1);
            }
            return false;
        }
        solver.statistics.sweep_flip_equivalences += 2; // ADD (.., 2)
    }
    {
        let k = solver.kitten.as_mut().unwrap();
        kitten::kitten_assume(k, not_lit);
        kitten::kitten_assume(k, other);
    }
    solver.statistics.sweep_solved_equivalences += 1; // STATISTIC
    let res = sweep_solve(solver);
    if res == 10 {
        solver.statistics.sweep_sat_equivalences += 1; // STATISTIC
        sweep_refine(solver, sweeper);
    } else if res == 0 {
        solver.statistics.sweep_unknown_equivalences += 1; // STATISTIC
    }

    if res != 20 {
        return false;
    }

    solver.statistics.sweep_unsat_equivalences += 1; // STATISTIC

    save_core(solver, sweeper, 0);

    {
        let k = solver.kitten.as_mut().unwrap();
        kitten::kitten_assume(k, lit);
        kitten::kitten_assume(k, not_other);
    }
    let res = sweep_solve(solver);
    solver.statistics.sweep_solved_equivalences += 1; // STATISTIC
    if res == 10 {
        solver.statistics.sweep_sat_equivalences += 1; // STATISTIC
        sweep_refine(solver, sweeper);
    } else if res == 0 {
        solver.statistics.sweep_unknown_equivalences += 1; // STATISTIC
    }

    if res != 20 {
        sweeper.core[0].clear();
        return false;
    }

    solver.statistics.sweep_unsat_equivalences += 1; // STATISTIC

    save_core(solver, sweeper, 1);

    solver.statistics.sweep_equivalences += 1; // INC — COUNTER

    add_core(solver, sweeper, 0);
    add_binary(solver, lit, not_other);
    clear_core(solver, sweeper, 0);

    add_core(solver, sweeper, 1);
    add_binary(solver, not_lit, other);
    clear_core(solver, sweeper, 1);

    let repr;
    if lit < other {
        repr = lit;
        sweeper.reprs[other as usize] = lit;
        sweeper.reprs[not_other as usize] = not_lit;
        substitute_connected_clauses(solver, sweeper, other, lit);
        substitute_connected_clauses(solver, sweeper, not_other, not_lit);
        sweep_remove(sweeper, other);
    } else {
        repr = other;
        sweeper.reprs[lit as usize] = other;
        sweeper.reprs[not_lit as usize] = not_other;
        substitute_connected_clauses(solver, sweeper, lit, other);
        substitute_connected_clauses(solver, sweeper, not_lit, not_other);
        sweep_remove(sweeper, lit);
    }

    let repr_idx = IDX(repr);
    schedule_inner(solver, sweeper, repr_idx);

    true
}

fn sweep_variable(solver: &mut Solver, sweeper: &mut Sweeper, idx: u32) -> &'static str {
    debug_assert!(!solver.inconsistent);
    if !solver.flags[idx as usize].active {
        return "inactive variable";
    }
    let start = LIT(idx);
    if sweeper.reprs[start as usize] != start {
        return "non-representative variable";
    }
    debug_assert!(sweeper.vars.is_empty());
    debug_assert!(sweeper.refs.is_empty());
    debug_assert!(sweeper.backbone.is_empty());
    debug_assert!(sweeper.partition.is_empty());
    debug_assert!(sweeper.encoded == 0);

    solver.statistics.sweep_variables += 1; // INC — STATISTIC

    debug_assert!(solver.values[start as usize] == 0);
    add_literal_to_environment(sweeper, 0, start);

    let mut limit_reached = false;
    let mut expand: usize = 0;
    let mut next_expand: usize = 1;
    let mut success = false;
    let mut depth: u32 = 1;

    while !limit_reached {
        if sweeper.encoded >= sweeper.limit.clauses {
            limit_reached = true;
            break;
        }
        if expand == next_expand {
            if depth >= sweeper.limit.depth {
                break; // environment depth limit reached
            }
            next_expand = sweeper.vars.len();
            if expand == next_expand {
                break; // completely copied all clauses
            }
            depth += 1;
        }
        let choices = (next_expand - expand) as u32;
        if solver.options.sweeprand != 0 && choices > 1 {
            let swap = crate::random::pick_random(&mut solver.random, 0, choices);
            if swap != 0 {
                sweeper.vars.swap(expand, expand + swap as usize);
            }
        }
        let expand_idx = sweeper.vars[expand];
        for sign in 0..2u32 {
            let lit = LIT(expand_idx) + sign;
            // all_binary_large_watches: single-word entries (dense mode).
            let v = solver.watches[lit as usize];
            let mut wp = v.begin;
            let end_w = v.end;
            while wp != end_w {
                let watch = solver.vectors.stack[wp];
                wp += 1;
                if watch_is_binary(watch) {
                    let other = watch_lit(watch);
                    sweep_binary(solver, sweeper, depth, lit, other);
                } else {
                    let ref_ = watch_ref(watch);
                    sweep_reference(solver, sweeper, depth, ref_);
                }
                if sweeper.vars.len() >= sweeper.limit.vars as usize {
                    limit_reached = true;
                    break;
                }
            }
            if limit_reached {
                break;
            }
        }
        expand += 1;
    }
    solver.statistics.sweep_depth += depth as u64; // ADD — STATISTIC
    solver.statistics.sweep_clauses += sweeper.encoded as u64; // ADD — STATISTIC
    solver.statistics.sweep_environment += sweeper.vars.len() as u64; // ADD — STATISTIC
    crate::print::extremely_verbose(
        solver,
        format!(
            "sweeping variable {} environment of {} variables {} clauses depth {}",
            crate::inline::export_literal(solver, LIT(idx)),
            sweeper.vars.len(),
            sweeper.encoded,
            depth
        ),
    );
    let res = sweep_solve(solver);
    let mut done = false;
    if res == 10 {
        init_backbone_and_partition(solver, sweeper);
        // #ifndef QUIET snapshots:
        let units0 = solver.statistics.sweep_units;
        let solved0 = solver.statistics.sweep_solved;
        crate::profile::start_checked(solver, Prof::sweepbackbone); // START
        loop {
            if sweeper.backbone.is_empty() {
                break;
            }
            if solver.inconsistent
                || crate::terminated!(solver, sweep_terminated_3)
                || kitten_ticks_limit_hit(solver, sweeper, "backbone refinement")
            {
                limit_reached = true;
                done = true; // goto DONE (via STOP_SWEEP_BACKBONE)
                break;
            }
            flip_backbone_literals(solver, sweeper);
            if crate::terminated!(solver, sweep_terminated_4)
                || kitten_ticks_limit_hit(solver, sweeper, "backbone refinement")
            {
                limit_reached = true;
                done = true; // goto STOP_SWEEP_BACKBONE
                break;
            }
            if sweeper.backbone.is_empty() {
                break;
            }
            let lit = sweeper.backbone.pop().unwrap();
            if !solver.flags[IDX(lit) as usize].active {
                continue;
            }
            if sweep_backbone_candidate(solver, sweeper, lit) {
                success = true;
            }
        }
        crate::profile::stop_checked(solver, Prof::sweepbackbone); // STOP
        if !done {
            let units = solver.statistics.sweep_units - units0;
            let solved = solver.statistics.sweep_solved - solved0;
            crate::print::extremely_verbose(
                solver,
                format!(
                    "complete swept variable {} backbone with {} units in {} solver calls",
                    crate::inline::export_literal(solver, LIT(idx)),
                    units,
                    solved
                ),
            );
            debug_assert!(sweeper.backbone.is_empty());
            let equivalences0 = solver.statistics.sweep_equivalences;
            let solved0 = solver.statistics.sweep_solved;
            crate::profile::start_checked(solver, Prof::sweepequivalences); // START
            loop {
                if sweeper.partition.is_empty() {
                    break;
                }
                if solver.inconsistent
                    || crate::terminated!(solver, sweep_terminated_5)
                    || kitten_ticks_limit_hit(solver, sweeper, "partition refinement")
                {
                    limit_reached = true;
                    done = true; // goto DONE (via STOP_SWEEP_EQUIVALENCES)
                    break;
                }
                flip_partition_literals(solver, sweeper);
                if crate::terminated!(solver, sweep_terminated_6)
                    || kitten_ticks_limit_hit(solver, sweeper, "backbone refinement")
                {
                    limit_reached = true;
                    done = true; // goto STOP_SWEEP_EQUIVALENCES
                    break;
                }
                if sweeper.partition.is_empty() {
                    break;
                }
                if sweeper.partition.len() > 2 {
                    let end = sweeper.partition.len();
                    debug_assert!(sweeper.partition[end - 1] == INVALID_LIT);
                    let lit = sweeper.partition[end - 3];
                    let other = sweeper.partition[end - 2];
                    if sweep_equivalence_candidates(solver, sweeper, lit, other) {
                        success = true;
                    }
                } else {
                    sweeper.partition.clear();
                }
            }
            crate::profile::stop_checked(solver, Prof::sweepequivalences); // STOP
            if !done {
                let equivalences = solver.statistics.sweep_equivalences - equivalences0;
                let solved = solver.statistics.sweep_solved - solved0;
                if equivalences != 0 {
                    crate::print::extremely_verbose(
                        solver,
                        format!(
                            "complete swept variable {} partition with {} \
                             equivalences in {} solver calls",
                            crate::inline::export_literal(solver, LIT(idx)),
                            equivalences,
                            solved
                        ),
                    );
                }
            }
        }
    } else if res == 20 {
        sweep_empty_clause(solver, sweeper);
    }

    // DONE:
    clear_sweeper(solver, sweeper);

    if !solver.inconsistent && !propagated(solver) {
        let _ = crate::propdense::dense_propagate(solver);
    }

    if success && limit_reached {
        return "successfully despite reaching limit";
    }
    if !success && !limit_reached {
        return "unsuccessfully without reaching limit";
    } else if success && !limit_reached {
        return "successfully without reaching limit";
    }
    debug_assert!(!success && limit_reached);
    "unsuccessfully and reached limit"
}

/// Port of `struct sweep_candidate`.
#[derive(Clone, Copy)]
struct SweepCandidate {
    rank: u32,
    idx: u32,
}

/// C name kept (sic): `scheduable_variable`.  The `size_t *occ_ptr`
/// out-parameter becomes an Option return.
fn scheduable_variable(solver: &Solver, sweeper: &Sweeper, idx: u32) -> Option<usize> {
    let lit = LIT(idx);
    let pos = solver.watches[lit as usize].size();
    if pos == 0 {
        return None;
    }
    let max_occurrences = sweeper.limit.clauses as usize;
    if pos > max_occurrences {
        return None;
    }
    let not_lit = NOT(lit);
    let neg = solver.watches[not_lit as usize].size();
    if neg == 0 {
        return None;
    }
    if neg > max_occurrences {
        return None;
    }
    Some(pos + neg)
}

fn schedule_all_other_not_scheduled_yet(solver: &mut Solver, sweeper: &mut Sweeper) -> u32 {
    let mut fresh: Vec<SweepCandidate> = Vec::new();
    let incomplete = solver.sweep_incomplete;
    for idx in 0..solver.vars {
        let f = &solver.flags[idx as usize];
        if !f.active {
            continue;
        }
        if incomplete && !f.sweep {
            continue;
        }
        if scheduled_variable(sweeper, idx) {
            continue;
        }
        match scheduable_variable(solver, sweeper, idx) {
            None => {
                solver.flags[idx as usize].sweep = false;
                continue;
            }
            Some(occ) => {
                fresh.push(SweepCandidate {
                    rank: occ as u32, // C: unsigned rank = (size_t) occ
                    idx,
                });
            }
        }
    }
    let size = fresh.len();
    debug_assert!(size <= u32::MAX as usize);
    crate::sort::radix_stack(&mut fresh, |cand: &SweepCandidate| cand.rank);
    for i in 0..fresh.len() {
        let cand = fresh[i];
        schedule_outer(sweeper, cand.idx);
    }
    size as u32
}

fn reschedule_previously_remaining(solver: &mut Solver, sweeper: &mut Sweeper) -> u32 {
    let mut rescheduled = 0u32;
    let remaining = std::mem::take(&mut solver.sweep_schedule);
    for &idx in remaining.iter() {
        let f = &solver.flags[idx as usize];
        if !f.active {
            continue;
        }
        if scheduled_variable(sweeper, idx) {
            continue;
        }
        match scheduable_variable(solver, sweeper, idx) {
            None => {
                solver.flags[idx as usize].sweep = false;
                continue;
            }
            Some(_) => {
                schedule_inner(solver, sweeper, idx);
                rescheduled += 1;
            }
        }
    }
    drop(remaining); // RELEASE_STACK (*remaining)
    rescheduled
}

fn incomplete_variables(solver: &Solver) -> u32 {
    let mut res = 0u32;
    for idx in 0..solver.vars {
        let f = &solver.flags[idx as usize];
        if !f.active {
            continue;
        }
        if f.sweep {
            res += 1;
        }
    }
    res
}

fn mark_incomplete(solver: &mut Solver, sweeper: &mut Sweeper) {
    let mut marked = 0u32;
    // all_scheduled (idx)
    let mut idx = sweeper.first;
    while idx != INVALID_IDX {
        let next = sweeper.next[idx as usize];
        if !solver.flags[idx as usize].sweep {
            solver.flags[idx as usize].sweep = true;
            marked += 1;
        }
        idx = next;
    }
    solver.sweep_incomplete = true;
    crate::print::extremely_verbose(
        solver,
        format!("marked {} scheduled sweeping variables as incomplete", marked),
    );
}

fn schedule_sweeping(solver: &mut Solver, sweeper: &mut Sweeper) -> u32 {
    let rescheduled = reschedule_previously_remaining(solver, sweeper);
    let fresh = schedule_all_other_not_scheduled_yet(solver, sweeper);
    let scheduled = fresh + rescheduled;
    let incomplete = incomplete_variables(solver);
    crate::print::phase(
        solver,
        "sweep",
        solver.statistics.sweep, // GET (sweep)
        format_args!(
            "scheduled {} variables {:.0}% ({} rescheduled {:.0}%, {} incomplete {:.0}%)",
            scheduled,
            crate::utilities::percent(scheduled as f64, solver.active as f64),
            rescheduled,
            crate::utilities::percent(rescheduled as f64, scheduled as f64),
            incomplete,
            crate::utilities::percent(incomplete as f64, scheduled as f64)
        ),
    );
    if incomplete != 0 {
        debug_assert!(solver.sweep_incomplete);
    } else {
        if solver.sweep_incomplete {
            solver.statistics.sweep_completed += 1; // INC — COUNTER
        }
        mark_incomplete(solver, sweeper);
    }
    scheduled
}

fn unschedule_sweeping(solver: &mut Solver, sweeper: &mut Sweeper, swept: u32, scheduled: u32) {
    debug_assert!(solver.sweep_schedule.is_empty());
    debug_assert!(solver.sweep_incomplete);
    // all_scheduled (idx)
    let mut idx = sweeper.first;
    while idx != INVALID_IDX {
        let next = sweeper.next[idx as usize];
        if solver.flags[idx as usize].active {
            solver.sweep_schedule.push(idx);
        }
        idx = next;
    }
    let retained = solver.sweep_schedule.len();
    crate::print::extremely_verbose(
        solver,
        format!(
            "retained {} variables {:.0}% to be swept next time",
            retained,
            crate::utilities::percent(retained as f64, solver.active as f64)
        ),
    );
    let incomplete = incomplete_variables(solver);
    if incomplete != 0 {
        crate::print::extremely_verbose(
            solver,
            format!(
                "need to sweep {} more variables {:.0}% for completion",
                incomplete,
                crate::utilities::percent(incomplete as f64, solver.active as f64)
            ),
        );
    } else {
        crate::print::extremely_verbose(solver, "no more variables needed to complete sweep");
        solver.sweep_incomplete = false;
        solver.statistics.sweep_completed += 1; // INC — COUNTER
    }
    crate::print::phase(
        solver,
        "sweep",
        solver.statistics.sweep, // GET (sweep)
        format_args!(
            "swept {} variables ({} remain {:.0}%)",
            swept,
            incomplete,
            crate::utilities::percent(incomplete as f64, scheduled as f64)
        ),
    );
}

/// Port of `kissat_sweep`.  Returns `eliminated != 0` (the C `bool` return of
/// the `uint64_t eliminated` value).
pub fn sweep(solver: &mut Solver) -> bool {
    if solver.options.sweep == 0 {
        return false;
    }
    if solver.inconsistent {
        return false;
    }
    if crate::terminated!(solver, sweep_terminated_7) {
        return false;
    }
    if crate::kimits::delaying(solver, DelayId::Sweep) {
        return false;
    }
    debug_assert!(solver.level == 0);
    debug_assert!(solver.unflushed == 0);
    crate::profile::start_checked(solver, Prof::sweep); // START (sweep)
    solver.statistics.sweep += 1; // INC (sweep) — COUNTER
    let equivalences0 = solver.statistics.sweep_equivalences;
    let units0 = solver.statistics.sweep_units;
    let mut sweeper = init_sweeper(solver);
    let scheduled = schedule_sweeping(solver, &mut sweeper);
    let mut swept: u64 = 0;
    let mut limit: u64 = 10;
    loop {
        if solver.inconsistent {
            break;
        }
        if crate::terminated!(solver, sweep_terminated_8) {
            break;
        }
        if solver.statistics.kitten_ticks > sweeper.limit.ticks {
            break;
        }
        let idx = next_scheduled(&mut sweeper);
        if idx == INVALID_IDX {
            break;
        }
        solver.flags[idx as usize].sweep = false;
        let res = sweep_variable(solver, &mut sweeper, idx);
        crate::print::extremely_verbose(
            solver,
            format!(
                "swept[{}] external variable {} {}",
                swept,
                crate::inline::export_literal(solver, LIT(idx)),
                res
            ),
        );
        swept += 1;
        if swept == limit {
            crate::print::very_verbose(
                solver,
                format!(
                    "found {} equivalences and {} units after sweeping {} variables ",
                    solver.statistics.sweep_equivalences - equivalences0,
                    solver.statistics.sweep_units - units0,
                    swept
                ),
            );
            limit *= 10;
        }
    }
    crate::print::very_verbose(solver, format!("swept {} variables", swept));
    let equivalences = solver.statistics.sweep_equivalences - equivalences0;
    let units = solver.statistics.sweep_units - units0;
    crate::print::phase(
        solver,
        "sweep",
        solver.statistics.sweep, // GET (sweep)
        format_args!("found {} equivalences and {} units", equivalences, units),
    );
    unschedule_sweeping(solver, &mut sweeper, swept as u32, scheduled);
    let inactive = release_sweeper(solver, &mut sweeper);

    if !solver.inconsistent {
        solver.propagate = 0; // solver->propagate = solver->trail.begin
        let _ = crate::proprobe::probing_propagate(solver, INVALID_REF, true);
    }

    let eliminated = equivalences + units;
    // #ifndef QUIET (kept):
    debug_assert!(solver.active >= inactive);
    solver.active -= inactive;
    crate::report::report(solver, eliminated == 0, '='); // REPORT (!eliminated, '=')
    solver.active += inactive;

    if crate::utilities::average(eliminated as f64, swept as f64) < 0.001 {
        crate::kimits::bump_delay(solver, DelayId::Sweep); // BUMP_DELAY (sweep)
    } else {
        crate::kimits::reduce_delay(solver, DelayId::Sweep); // REDUCE_DELAY (sweep)
    }
    crate::profile::stop_checked(solver, Prof::sweep); // STOP (sweep)
    eliminated != 0
}
