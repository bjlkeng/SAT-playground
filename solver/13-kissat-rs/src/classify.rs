// Port of src/classify.c (kissat 4.0.4).

use crate::internal::Solver;

/// Port of `kissat_classify`.
pub fn classify(solver: &mut Solver) {
    let clauses = solver.statistics.clauses_binary + solver.statistics.clauses_irredundant;
    let small_clauses_limit = solver.options.smallclauses as u32; // unsigned
    if clauses <= small_clauses_limit as u64 {
        solver.classification.small = true;
        solver.classification.bigbig = false;
    } else {
        solver.classification.small = false;
        let bigbigfraction = solver.options.bigbigfraction as u32; // unsigned
        let percent = bigbigfraction as f64 / 1000.0;
        let actual = crate::utilities::percent(
            solver.statistics.clauses_binary as f64,
            clauses as f64,
        );
        if actual >= percent {
            solver.classification.bigbig = true;
        } else {
            solver.classification.bigbig = false;
        }
    }
    crate::print::very_verbose(
        solver,
        format_args!(
            "formula classified as having a {} total number of clauses",
            if solver.classification.small {
                "small"
            } else {
                "large"
            }
        ),
    );
    crate::print::very_verbose(
        solver,
        format_args!(
            "formula classified to have a {} binary clauses fraction",
            if solver.classification.bigbig {
                "large"
            } else {
                "small"
            }
        ),
    );
}
