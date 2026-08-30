// TEMPORARY stub modules for core-wave engines referenced by the foundation.
// Each `pub mod` here is re-exported as a crate module from main.rs and is
// replaced wholesale by its real port (src/<name>.rs) when its wave lands.

pub mod restart {
    use crate::internal::Solver;
    pub fn update_focused_restart_limit(solver: &mut Solver) {
        let _ = solver;
        unimplemented!("restart wave pending")
    }
}

pub mod import {
    use crate::internal::Solver;
    pub fn import_literal(solver: &mut Solver, _elit: i32) -> u32 {
        let _ = solver;
        unimplemented!("import wave pending")
    }
}

pub mod propsearch {
    use crate::internal::Solver;
    use crate::reference::Reference;
    pub fn search_propagate(solver: &mut Solver) -> Reference {
        let _ = solver;
        unimplemented!("propsearch wave pending")
    }
}

pub mod backtrack {
    use crate::internal::Solver;
    pub fn backtrack_without_updating_phases(solver: &mut Solver, _level: u32) {
        let _ = solver;
        unimplemented!("backtrack wave pending")
    }
}

pub mod search {
    use crate::internal::Solver;
    pub fn search(solver: &mut Solver) -> i32 {
        let _ = solver;
        unimplemented!("search wave pending")
    }
}

pub mod resize {
    use crate::internal::Solver;
    pub fn increase_size(solver: &mut Solver, _new_size: u32) {
        let _ = solver;
        unimplemented!("resize wave pending")
    }
}

pub mod bump {
    use crate::internal::Solver;
    pub fn update_scores(solver: &mut Solver) {
        let _ = solver;
        unimplemented!("bump wave pending")
    }
}

pub mod decide {
    use crate::internal::Solver;
    pub fn start_random_sequence(solver: &mut Solver) {
        let _ = solver;
        unimplemented!("decide wave pending")
    }
}
