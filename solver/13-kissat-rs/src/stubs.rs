// TEMPORARY stub modules for core-wave engines referenced by the foundation.
// Each `pub mod` here is re-exported as a crate module from main.rs and is
// replaced wholesale by its real port (src/<name>.rs) when its wave lands.

// restart / import / resize / decide / reduce stubs removed: real ports
// landed as src/restart.rs, src/import.rs, src/resize.rs, src/decide.rs,
// src/reduce.rs (decision/GC wave).

pub mod compact {
    use crate::internal::Solver;
    // kissat_compact_literals / kissat_finalize_compacting (compact.c,
    // inprocessing wave), needed by collect::sparse_collect.  The C
    // `unsigned *mfixed_ptr` out-parameter becomes the second tuple element.
    pub fn compact_literals(solver: &mut Solver) -> (u32, u32) {
        let _ = solver;
        unimplemented!("compact wave pending")
    }
    pub fn finalize_compacting(solver: &mut Solver, _vars: u32, _mfixed: u32) {
        let _ = solver;
        unimplemented!("compact wave pending")
    }
}

// --- stubs added by the search/application wave ------------------------

// lucky stub removed: real port landed as src/lucky.rs (lucky/proprobe wave).

pub mod preprocess {
    use crate::internal::Solver;
    // kissat_preprocessing / kissat_preprocess (preprocess.c), needed by
    // search::search.
    // Faithful port of kissat_preprocessing (preprocess.c) so runs with
    // the engines disabled reach search; kissat_preprocess remains stubbed.
    pub fn preprocessing(solver: &mut Solver) -> bool {
        debug_assert!(solver.level == 0);
        debug_assert!(!solver.inconsistent);
        if solver.options.preprocess == 0 {
            return false;
        }
        if solver.options.probe == 0 {
            return false;
        }
        solver.options.preprocessprobe != 0
    }
    pub fn preprocess(solver: &mut Solver) -> i32 {
        let _ = solver;
        unimplemented!("preprocess wave pending")
    }
}

pub mod reorder {
    use crate::internal::Solver;
    // kissat_reordering / kissat_reorder (reorder.c), needed by search::search.
    // Faithful port of kissat_reordering (reorder.c).
    pub fn reordering(solver: &mut Solver) -> bool {
        if solver.options.reorder == 0 {
            return false;
        }
        if !solver.stable && solver.options.reorder < 2 {
            return false;
        }
        if solver.level != 0 {
            return false;
        }
        solver.statistics.conflicts >= solver.limits.reorder.conflicts
    }
    pub fn reorder(solver: &mut Solver) {
        let _ = solver;
        unimplemented!("reorder wave pending")
    }
}

pub mod probe {
    use crate::internal::Solver;
    // kissat_probing / kissat_probe (probe.c), needed by search::search.
    // Faithful port of kissat_probing (probe.c).
    pub fn probing(solver: &mut Solver) -> bool {
        if !solver.enabled.probe {
            return false;
        }
        let conflicts = solver.statistics.conflicts;
        if solver.last.conflicts.reduce == conflicts {
            return false;
        }
        solver.limits.probe.conflicts < conflicts
    }
    pub fn probe(solver: &mut Solver) -> i32 {
        let _ = solver;
        unimplemented!("probe wave pending")
    }
}

pub mod eliminate {
    use crate::internal::Solver;
    // kissat_eliminating / kissat_eliminate (eliminate.c), needed by
    // search::search.
    // Faithful port of kissat_eliminating (eliminate.c).
    pub fn eliminating(solver: &mut Solver) -> bool {
        if !solver.enabled.eliminate {
            return false;
        }
        if solver.statistics.clauses_irredundant == 0 {
            return false;
        }
        let conflicts = solver.statistics.conflicts;
        if solver.last.conflicts.reduce == conflicts {
            return false;
        }
        if solver.limits.eliminate.conflicts > conflicts {
            return false;
        }
        if solver.limits.eliminate.variables.eliminate
            < solver.statistics.variables_eliminate
        {
            return true;
        }
        solver.limits.eliminate.variables.subsume < solver.statistics.variables_subsume
    }
    pub fn eliminate(solver: &mut Solver) -> i32 {
        let _ = solver;
        unimplemented!("eliminate wave pending")
    }
}

pub mod walk {
    use crate::internal::Solver;
    // kissat_walking / kissat_walk (walk.c), needed by rephase::rephase.
    // Faithful port of kissat_walking (walk.c): MAX_WALK_REF bounds.
    pub fn walking(solver: &Solver) -> bool {
        const MAX_WALK_REF: u64 = (1u64 << 31) - 1;
        let last_irredundant = if solver.last_irredundant == crate::reference::INVALID_REF {
            solver.arena.size_wards()
        } else {
            solver.last_irredundant as u64
        };
        if last_irredundant > MAX_WALK_REF {
            return false;
        }
        let clauses = solver.statistics.binirr_clauses();
        clauses <= MAX_WALK_REF
    }
    pub fn walk(solver: &mut Solver) {
        let _ = solver;
        unimplemented!("walk wave pending")
    }
}

pub mod krite {
    use crate::internal::Solver;
    // kissat_write_dimacs (krite.c), needed by application::run_application
    // for the '-o <path>' option.
    pub fn write_dimacs(solver: &mut Solver, _file: &mut dyn std::io::Write) {
        let _ = solver;
        unimplemented!("krite wave pending")
    }
}

// --- stubs added by the conflict-analysis wave -------------------------

// proprobe stub removed: real port landed as src/proprobe.rs (lucky/proprobe
// wave).

// promote stub removed: real port landed as src/promote.rs (decision/GC
// wave; same exact bodies).

// strengthen stub removed: real port landed as src/strengthen.rs.
