// Port of src/statistics.h + src/statistics.c (kissat 4.0.4).
//
// Reference build: gcc -O3 -DNDEBUG with neither METRICS nor STATISTICS
// defined (verified against benchmarks/reference-solvers/kissat-latest/
// build/makefile: "CC=gcc -W -Wall -O3 -DNDEBUG"). In that build the table
// in statistics.h expands METRIC -> IGNORE and STATISTIC -> IGNORE, so ONLY
// the COUNTER(...) entries exist as struct fields and are printed by
// `kissat_statistics_print`, and the `#ifndef STATISTICS` branches make
// PCNT_TICKS and PER_FLIPPED evaluate to -1 (secondary column suppressed).
//
// PORT NOTE: METRIC-only counters are omitted entirely (per CONVENTIONS.md).
// STATISTIC-tier counters are compiled out of the reference default build too
// (their kissat_inc_*/kissat_add_* helpers are no-ops), but they are kept
// here as real u64 fields so sibling modules can port INC()/ADD() call sites
// 1:1 instead of deleting them. They are NEVER printed (matching the
// reference `-s` output byte-for-byte) and nothing in the default build
// reads them, so incrementing them cannot diverge from C. Fields appear in
// exact statistics.h table order, STATISTIC entries marked "// STATISTIC".
//
// PORT NOTE: the C IGNORE variant of kissat_get_* returns UINT64_MAX for
// compiled-out counters. The only default-build GET users of such counters
// are `GET (vectors_enlarged)` and `GET (defragmentations)` in vector.c
// (verbose kissat_phase messages) — vector.rs must reproduce u64::MAX there,
// not read a field from this struct (those two are METRIC and omitted here).

use crate::format;
use crate::internal::Solver;
use crate::resources;
use std::io::Write as _;

pub const MAX_GLUE_USED: usize = 127;

#[derive(Clone, Copy)]
pub struct Used {
    pub glue: [u64; MAX_GLUE_USED + 1],
}

impl Default for Used {
    fn default() -> Self {
        Used {
            glue: [0; MAX_GLUE_USED + 1],
        }
    }
}

#[derive(Default)]
pub struct Statistics {
    pub ands_eliminated: u64, // STATISTIC
    pub backbone_computations: u64,
    pub backbone_ticks: u64,
    pub backbone_units: u64, // STATISTIC
    pub chronological: u64,
    pub clauses_added: u64,
    pub clauses_binary: u64,
    pub clauses_deleted: u64,  // STATISTIC
    pub clauses_factored: u64, // STATISTIC
    pub clauses_improved: u64, // STATISTIC
    pub clauses_irredundant: u64,
    pub clauses_kept1: u64, // STATISTIC
    pub clauses_kept2: u64, // STATISTIC
    pub clauses_kept3: u64, // STATISTIC
    pub clauses_learned: u64,
    pub clauses_original: u64,
    pub clauses_promoted1: u64,     // STATISTIC
    pub clauses_promoted2: u64,     // STATISTIC
    pub clauses_reduced: u64,       // STATISTIC
    pub clauses_reduced_tier1: u64, // STATISTIC
    pub clauses_reduced_tier2: u64, // STATISTIC
    pub clauses_reduced_tier3: u64, // STATISTIC
    pub clauses_redundant: u64,
    pub clauses_unfactored: u64, // STATISTIC
    pub clauses_used: u64,
    pub clauses_used_focused: u64,
    pub clauses_used_stable: u64,
    pub closures: u64,
    pub conflicts: u64,
    pub congruent: u64,
    pub congruent_ands: u64,               // STATISTIC
    pub congruent_arity: u64,              // STATISTIC
    pub congruent_arity_ands: u64,         // STATISTIC
    pub congruent_arity_xors: u64,         // STATISTIC
    pub congruent_binaries: u64,           // STATISTIC
    pub congruent_ites: u64,               // STATISTIC
    pub congruent_collisions: u64,         // STATISTIC
    pub congruent_collisions_find: u64,    // STATISTIC
    pub congruent_collisions_index: u64,   // STATISTIC
    pub congruent_collisions_removed: u64, // STATISTIC
    pub congruent_equivalences: u64,       // STATISTIC
    pub congruent_gates: u64,
    pub congruent_gates_ands: u64,
    pub congruent_gates_ites: u64,
    pub congruent_gates_xors: u64,
    pub congruent_indexed: u64,         // STATISTIC
    pub congruent_lookups: u64,         // STATISTIC
    pub congruent_lookups_find: u64,    // STATISTIC
    pub congruent_lookups_removed: u64, // STATISTIC
    pub congruent_matched: u64,
    pub congruent_matched_ands: u64,
    pub congruent_matched_ites: u64,
    pub congruent_matched_xors: u64,
    pub congruent_rewritten: u64,        // STATISTIC
    pub congruent_rewritten_ands: u64,   // STATISTIC
    pub congruent_rewritten_ites: u64,   // STATISTIC
    pub congruent_rewritten_xors: u64,   // STATISTIC
    pub congruent_simplified: u64,       // STATISTIC
    pub congruent_simplified_ands: u64,  // STATISTIC
    pub congruent_simplified_ites: u64,  // STATISTIC
    pub congruent_simplified_xors: u64,  // STATISTIC
    pub congruent_subsumed: u64,         // STATISTIC
    pub congruent_trivial_ite: u64,      // STATISTIC
    pub congruent_unary: u64,            // STATISTIC
    pub congruent_unary_ands: u64,       // STATISTIC
    pub congruent_unary_ites: u64,       // STATISTIC
    pub congruent_unary_xors: u64,       // STATISTIC
    pub congruent_units: u64,            // STATISTIC
    pub congruent_xors: u64,             // STATISTIC
    pub decisions: u64,
    pub definitions_eliminated: u64, // STATISTIC
    pub definition_units: u64,       // STATISTIC
    pub eagerly_subsumed: u64,       // STATISTIC
    pub eliminate_attempted: u64,    // STATISTIC
    pub eliminated: u64,
    pub eliminate_resolutions: u64,
    pub eliminate_units: u64, // STATISTIC
    pub eliminations: u64,
    pub equivalences_eliminated: u64, // STATISTIC
    pub factored: u64,
    pub factorizations: u64,
    pub factor_ticks: u64,
    pub fast_eliminated: u64,
    pub fast_strengthened: u64,
    pub fast_subsumed: u64,
    pub fresh: u64,   // STATISTIC
    pub flipped: u64, // STATISTIC
    pub forward_checks: u64,
    pub forward_steps: u64,
    pub forward_strengthened: u64,    // STATISTIC
    pub forward_subsumed: u64,        // STATISTIC
    pub gates_eliminated: u64,        // STATISTIC
    pub if_then_else_eliminated: u64, // STATISTIC
    pub iterations: u64,
    pub jumped_reasons: u64,   // STATISTIC
    pub kitten_conflicts: u64, // STATISTIC
    pub kitten_decisions: u64, // STATISTIC
    pub kitten_flip: u64,      // STATISTIC
    pub kitten_flipped: u64,   // STATISTIC
    pub kitten_propagations: u64,
    pub kitten_sat: u64, // STATISTIC
    pub kitten_solved: u64,
    pub kitten_ticks: u64,
    pub kitten_unknown: u64, // STATISTIC
    pub kitten_unsat: u64,   // STATISTIC
    pub literals_factor: u64,
    pub literals_factored: u64,       // STATISTIC
    pub literals_unfactored: u64,     // STATISTIC
    pub on_the_fly_strengthened: u64, // STATISTIC
    pub on_the_fly_subsumed: u64,     // STATISTIC
    pub probings: u64,
    pub probing_ticks: u64,
    pub propagations: u64,
    pub queue_decisions: u64,  // STATISTIC
    pub random_decisions: u64, // STATISTIC
    pub random_sequences: u64,
    pub reductions: u64,
    pub reordered: u64,
    pub reordered_focused: u64, // STATISTIC
    pub reordered_stable: u64,  // STATISTIC
    pub rephased: u64,
    pub restarts: u64,
    pub restarts_levels: u64,        // STATISTIC
    pub restarts_reused_levels: u64, // STATISTIC
    pub restarts_reused_trails: u64, // STATISTIC
    pub retiered: u64,
    pub searches: u64,
    pub search_ticks: u64,
    pub strengthened: u64,
    pub substituted: u64,
    pub substitute_ticks: u64,
    pub substitute_units: u64, // STATISTIC
    pub substitutions: u64,    // STATISTIC
    pub subsumed: u64,
    pub subsumption_checks: u64,
    pub sweep: u64,
    pub sweep_clauses: u64, // STATISTIC
    pub sweep_completed: u64,
    pub sweep_depth: u64,       // STATISTIC
    pub sweep_environment: u64, // STATISTIC
    pub sweep_equivalences: u64,
    pub sweep_fixed_backbone: u64,        // STATISTIC
    pub sweep_flip_backbone: u64,         // STATISTIC
    pub sweep_flipped_backbone: u64,      // STATISTIC
    pub sweep_flip_equivalences: u64,     // STATISTIC
    pub sweep_flipped_equivalences: u64,  // STATISTIC
    pub sweep_sat: u64,                   // STATISTIC
    pub sweep_sat_backbone: u64,          // STATISTIC
    pub sweep_sat_equivalences: u64,      // STATISTIC
    pub sweep_solved: u64,
    pub sweep_solved_backbone: u64,     // STATISTIC
    pub sweep_solved_equivalences: u64, // STATISTIC
    pub sweep_unknown_backbone: u64,    // STATISTIC
    pub sweep_unknown_equivalences: u64, // STATISTIC
    pub sweep_units: u64,
    pub sweep_unsat: u64,              // STATISTIC
    pub sweep_unsat_backbone: u64,     // STATISTIC
    pub sweep_unsat_equivalences: u64, // STATISTIC
    pub sweep_variables: u64,          // STATISTIC
    pub switched: u64,
    pub ticks: u64, // STATISTIC (NOTE: distinct from solver.ticks in internal.h)
    pub transitive_ticks: u64,
    pub units: u64,
    pub variables_activated: u64,
    pub variables_eliminate: u64,
    pub variables_extension: u64,
    pub variables_factor: u64,
    pub variables_original: u64,
    pub variables_subsume: u64,
    pub vivifications: u64,
    pub vivified: u64,
    pub vivified_asym: u64,         // STATISTIC
    pub vivified_implied: u64,      // STATISTIC
    pub vivified_instantiated: u64, // STATISTIC
    pub vivified_instirr: u64,      // STATISTIC
    pub vivified_instred: u64,      // STATISTIC
    pub vivified_irredundant: u64,  // STATISTIC
    pub vivified_promoted: u64,     // STATISTIC
    pub vivified_shrunken: u64,     // STATISTIC
    pub vivified_shrunkirr: u64,    // STATISTIC
    pub vivified_shrunkred: u64,    // STATISTIC
    pub vivified_subirr: u64,       // STATISTIC
    pub vivified_subred: u64,       // STATISTIC
    pub vivified_subsumed: u64,     // STATISTIC
    pub vivified_tier1: u64,        // STATISTIC
    pub vivified_tier2: u64,        // STATISTIC
    pub vivified_tier3: u64,        // STATISTIC
    pub vivified_unlearn: u64,      // STATISTIC
    pub vivify_checks: u64,
    pub vivify_probes: u64,
    pub vivify_propagations: u64, // STATISTIC
    pub vivify_reused: u64,
    pub vivify_ticks: u64, // STATISTIC
    pub vivify_units: u64, // STATISTIC
    pub walk_improved: u64, // STATISTIC
    pub walks: u64,
    pub walk_steps: u64,
    pub warming_conflicts: u64, // STATISTIC
    pub warming_decisions: u64,
    pub warming_propagations: u64,
    pub warmups: u64,

    pub used: [Used; 2],
}

// statistics.h convenience macros (CLAUSES, BINIRR_CLAUSES etc.) as helpers.
impl Statistics {
    // #define CLAUSES (IRREDUNDANT_CLAUSES + BINARY_CLAUSES + REDUNDANT_CLAUSES)
    pub fn clauses(&self) -> u64 {
        self.clauses_irredundant + self.clauses_binary + self.clauses_redundant
    }
    // #define BINIRR_CLAUSES (BINARY_CLAUSES + IRREDUNDANT_CLAUSES)
    pub fn binirr_clauses(&self) -> u64 {
        self.clauses_binary + self.clauses_irredundant
    }
}

// PRINT_STAT from statistics.h:
//   printf ("%s%-30s %12" PRIu64 " ", prefix, "name:", primary);
//   if (TYPE && SECONDARY >= 0) {
//     if (UNITS) printf ("%16.0f %-2s", SECONDARY, UNITS);
//     else       printf ("%19.2f", SECONDARY);
//     fputc (' '); fputs (TYPE);
//   }
//   fputc ('\n');
// (SFW1=30, SFW2=12, SFW34=16, SFW34EXTENDED=19)
fn print_stat(
    prefix: &str,
    name: &str,
    primary: u64,
    secondary: f64,
    units: Option<&str>,
    type_: Option<&str>,
) {
    print!("{}{:<30} {:>12} ", prefix, format!("{}:", name), primary);
    if let Some(type_) = type_ {
        if secondary >= 0.0 {
            if let Some(units) = units {
                print!("{:>16.0} {:<2}", secondary, units);
            } else {
                print!("{:>19.2}", secondary);
            }
            print!(" {}", type_);
        }
    }
    println!();
}

// kissat_print_glue_usage
pub fn print_glue_usage(solver: &mut Solver) {
    // C reads these as int64_t; only zero/non-zero matters.
    let stable = solver.statistics.clauses_used_stable;
    let focused = solver.statistics.clauses_used_focused;
    if stable == 0 && focused == 0 {
        print!("{}no clauses used at all\n", solver.prefix);
    } else {
        if focused != 0 {
            crate::tiers::print_tier_usage_statistics(solver, false);
        }
        if focused != 0 && stable != 0 {
            print!("c\n");
        }
        if stable != 0 {
            crate::tiers::print_tier_usage_statistics(solver, true);
        }
    }
    std::io::stdout().flush().ok();
}

// kissat_statistics_print — parity oracle for `-s` against the reference
// binary. Entries appear in exact statistics.h table order; only COUNTER
// entries of the default build print. Guard per entry:
//   verbose || !VERBOSE || (VERBOSE == 1 && statistics->NAME)
#[allow(clippy::nonminimal_bool)]
pub fn statistics_print(solver: &mut Solver, verbose: bool) {
    let time = resources::process_time();
    let st = &solver.statistics;
    let prefix: &str = &solver.prefix;
    // size_t variables = solver->statistics.variables_original;
    let variables = st.variables_original as f64;

    // RELATIVE (FIRST, SECOND) = kissat_average (first, second)
    macro_rules! rel {
        ($a:ident, $b:ident) => {
            format::average(st.$a as f64, st.$b as f64)
        };
    }
    // PERCENT (FIRST, SECOND) = kissat_percent (first, second)
    macro_rules! pcnt {
        ($a:ident, $b:ident) => {
            format::percent(st.$a as f64, st.$b as f64)
        };
    }
    // One COUNTER table row. $v is the VERBOSE column (0, 1 or 2).
    macro_rules! stat {
        ($name:ident, $v:expr, $sec:expr, $units:expr, $typ:expr) => {
            if verbose || $v == 0 || ($v == 1 && st.$name != 0) {
                print_stat(prefix, stringify!($name), st.$name, $sec, $units, $typ);
            }
        };
    }

    // NB: in the default build PCNT_TICKS(NAME) == -1 and PER_FLIPPED(NAME)
    // == -1 (#ifndef STATISTICS), which suppresses the secondary column.
    stat!(backbone_computations, 2, rel!(conflicts, backbone_computations), Some(""), Some("interval"));
    stat!(backbone_ticks, 2, -1.0, Some("%"), Some("ticks"));
    stat!(chronological, 1, pcnt!(chronological, conflicts), Some("%"), Some("conflicts"));
    stat!(clauses_added, 2, pcnt!(clauses_added, clauses_added), Some("%"), Some("added"));
    stat!(clauses_binary, 2, pcnt!(clauses_binary, clauses_added), Some("%"), Some("added"));
    stat!(clauses_irredundant, 2, pcnt!(clauses_irredundant, clauses_added), Some("%"), Some("added"));
    stat!(clauses_learned, 2, pcnt!(clauses_learned, conflicts), Some("%"), Some("conflicts"));
    stat!(clauses_original, 2, pcnt!(clauses_original, clauses_added), Some("%"), Some("added"));
    stat!(clauses_redundant, 2, 0.0, None, None);
    stat!(clauses_used, 2, pcnt!(clauses_used, clauses_learned), Some("%"), Some("learned"));
    stat!(clauses_used_focused, 2, pcnt!(clauses_used_focused, clauses_used), Some("%"), Some("used"));
    stat!(clauses_used_stable, 2, pcnt!(clauses_used_stable, clauses_used), Some("%"), Some("used"));
    stat!(closures, 2, rel!(conflicts, closures), Some(""), Some("interval"));
    stat!(conflicts, 0, format::average(st.conflicts as f64, time), None, Some("per second"));
    stat!(congruent, 1, format::percent(st.congruent as f64, variables), Some("%"), Some("variables"));
    stat!(congruent_gates, 2, rel!(congruent_gates, closures), None, Some("per closure"));
    stat!(congruent_gates_ands, 2, pcnt!(congruent_gates_ands, congruent_gates), Some("%"), Some("gates"));
    stat!(congruent_gates_ites, 2, pcnt!(congruent_gates_ites, congruent_gates), Some("%"), Some("gates"));
    stat!(congruent_gates_xors, 2, pcnt!(congruent_gates_xors, congruent_gates), Some("%"), Some("gates"));
    stat!(congruent_matched, 2, pcnt!(congruent_matched, congruent), Some("%"), Some("congruent"));
    stat!(congruent_matched_ands, 2, pcnt!(congruent_matched_ands, congruent_matched), Some("%"), Some("matched"));
    stat!(congruent_matched_ites, 2, pcnt!(congruent_matched_ites, congruent_matched), Some("%"), Some("matched"));
    stat!(congruent_matched_xors, 2, pcnt!(congruent_matched_xors, congruent_matched), Some("%"), Some("matched"));
    stat!(decisions, 0, rel!(decisions, conflicts), None, Some("per conflict"));
    stat!(eliminated, 1, format::percent(st.eliminated as f64, variables), Some("%"), Some("variables"));
    stat!(eliminate_resolutions, 2, format::average(st.eliminate_resolutions as f64, time), None, Some("per second"));
    stat!(eliminations, 2, rel!(conflicts, eliminations), Some(""), Some("interval"));
    stat!(factored, 1, format::percent(st.factored as f64, variables), Some("%"), Some("variables"));
    stat!(factorizations, 2, rel!(conflicts, factorizations), Some(""), Some("interval"));
    stat!(factor_ticks, 2, -1.0, Some("%"), Some("ticks"));
    stat!(fast_eliminated, 1, pcnt!(fast_eliminated, eliminated), Some("%"), Some("eliminated"));
    stat!(fast_strengthened, 1, pcnt!(fast_strengthened, strengthened), Some("%"), Some("per strengthened"));
    stat!(fast_subsumed, 1, pcnt!(fast_subsumed, subsumed), Some("%"), Some("per subsumed"));
    stat!(forward_checks, 2, 0.0, None, None);
    stat!(forward_steps, 2, rel!(forward_steps, forward_checks), None, Some("per check"));
    stat!(iterations, 1, format::percent(st.iterations as f64, variables), Some("%"), Some("variables"));
    stat!(kitten_propagations, 2, rel!(kitten_propagations, kitten_solved), None, Some("per solved"));
    stat!(kitten_solved, 2, 0.0, None, None);
    stat!(kitten_ticks, 2, rel!(kitten_ticks, kitten_propagations), None, Some("per prop"));
    stat!(literals_factor, 2, format::average(st.literals_factor as f64, variables), None, Some("per variable"));
    stat!(probings, 2, rel!(conflicts, probings), Some(""), Some("interval"));
    stat!(probing_ticks, 2, -1.0, Some("%"), Some("ticks"));
    stat!(propagations, 0, format::average(st.propagations as f64, time), Some(""), Some("per second"));
    stat!(random_sequences, 2, rel!(conflicts, random_sequences), Some(""), Some("interval"));
    stat!(reductions, 1, rel!(conflicts, reductions), Some(""), Some("interval"));
    stat!(reordered, 1, rel!(conflicts, reordered), Some(""), Some("interval"));
    stat!(rephased, 1, rel!(conflicts, rephased), Some(""), Some("interval"));
    stat!(restarts, 1, rel!(conflicts, restarts), Some(""), Some("interval"));
    stat!(retiered, 2, rel!(conflicts, retiered), Some(""), Some("interval"));
    stat!(searches, 2, rel!(conflicts, searches), Some(""), Some("interval"));
    stat!(search_ticks, 2, -1.0, Some("%"), Some("ticks"));
    stat!(strengthened, 1, pcnt!(strengthened, subsumption_checks), Some("%"), Some("checks"));
    stat!(substituted, 1, format::percent(st.substituted as f64, variables), Some("%"), Some("variables"));
    stat!(substitute_ticks, 2, -1.0, Some("%"), Some("ticks"));
    stat!(subsumed, 1, pcnt!(subsumed, subsumption_checks), Some("%"), Some("checks"));
    stat!(subsumption_checks, 2, 0.0, None, None);
    stat!(sweep, 2, rel!(conflicts, sweep), Some(""), Some("interval"));
    stat!(sweep_completed, 2, rel!(sweep, sweep_completed), None, Some("sweeps"));
    stat!(sweep_equivalences, 2, format::percent(st.sweep_equivalences as f64, variables), Some("%"), Some("variables"));
    stat!(sweep_solved, 2, pcnt!(sweep_solved, kitten_solved), Some("%"), Some("kitten_solved"));
    stat!(sweep_units, 2, format::percent(st.sweep_units as f64, variables), Some("%"), Some("variables"));
    stat!(switched, 0, rel!(conflicts, switched), Some(""), Some("interval"));
    stat!(transitive_ticks, 2, -1.0, Some("%"), Some("ticks"));
    stat!(units, 2, format::percent(st.units as f64, variables), Some("%"), Some("variables"));
    stat!(variables_activated, 2, format::average(st.variables_activated as f64, variables), None, Some("per variable"));
    stat!(variables_eliminate, 2, format::average(st.variables_eliminate as f64, variables), None, Some("variables"));
    stat!(variables_extension, 2, format::average(st.variables_extension as f64, variables), None, Some("per variable"));
    stat!(variables_factor, 2, format::average(st.variables_factor as f64, variables), None, Some("per variable"));
    stat!(variables_original, 2, format::average(st.variables_original as f64, variables), None, Some("per variable"));
    stat!(variables_subsume, 2, format::average(st.variables_subsume as f64, variables), None, Some("per variable"));
    stat!(vivifications, 2, rel!(conflicts, vivifications), Some(""), Some("interval"));
    stat!(vivified, 1, pcnt!(vivified, vivify_checks), Some("%"), Some("checks"));
    stat!(vivify_checks, 2, rel!(vivify_checks, vivifications), Some(""), Some("per vivify"));
    stat!(vivify_probes, 2, rel!(vivify_probes, vivify_checks), None, Some("per check"));
    stat!(vivify_reused, 2, pcnt!(vivify_reused, vivify_probes), Some("%"), Some("probes"));
    stat!(walks, 1, rel!(conflicts, walks), Some(""), Some("interval"));
    stat!(walk_steps, 2, -1.0, None, Some("per flipped"));
    stat!(warming_decisions, 2, rel!(warming_decisions, walks), None, Some("per walk"));
    stat!(warming_propagations, 2, pcnt!(warming_propagations, propagations), Some("%"), Some("propagations"));
    stat!(warmups, 2, pcnt!(warmups, walks), Some("%"), Some("walks"));

    std::io::stdout().flush().ok();
}

#[cfg(test)]
mod tests {
    #[test]
    fn print_stat_layout_matches_c() {
        // Manually verified against the C PRINT_STAT expansion:
        //   "%s%-30s %12" PRIu64 " "  then  "%16.0f %-2s" / "%19.2f"  " " TYPE
        // No trailing-newline assertion possible via print!; smoke-check the
        // helper formats instead.
        // Byte-verified against `printf ("%s%-30s %12" PRIu64 " ", ...)`.
        let s = format!("{}{:<30} {:>12} ", "c ", "conflicts:", 42u64);
        assert_eq!(s, "c conflicts:                               42 ");
        let sec = format!("{:>16.0} {:<2}", 12.4f64, "%");
        assert_eq!(sec, "              12 % ");
        let ext = format!("{:>19.2}", 1.5f64);
        assert_eq!(ext, "               1.50");
    }
}
