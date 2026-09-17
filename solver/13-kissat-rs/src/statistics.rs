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

/// Declares `Statistics` from one list, so the RL logger (policy.rs, plan
/// step A.5) can dump every counter by name (`Statistics::NAMES`,
/// `Statistics::values`) without a second hand-written list that would
/// drift. Fields are in statistics.h table order. Tiers: unmarked =
/// COUNTER (printed by `-s`); `STATISTIC` = kept as a real field, never
/// printed (see the PORT NOTE above); `METRIC (re-enabled)` = a kissat
/// METRICS-only counter re-enabled 2026-09-16 for the RL observation (plan
/// §5.1, step A.6). Re-enabled counters are pure increments at exactly the C
/// `INC`/`ADD` sites, are never printed, and are never read by a heuristic,
/// so they cannot change the trajectory; the `GET (...)` message sites of
/// METRIC counters keep printing `u64::MAX` (no count) exactly as the
/// reference build does, which tools/parity.py --phases relies on. Not
/// re-enabled: `allocated_*` (malloc accounting the port has no equivalent
/// of), `extensions` and `walk_previous` (no `INC` site in kissat 4.0.4).
macro_rules! statistics_fields {
    ($($name:ident,)*) => {
        #[derive(Default)]
        pub struct Statistics {
            $(pub $name: u64,)*
            pub used: [Used; 2],
        }

        impl Statistics {
            /// Every counter's name, in field order.
            pub const NAMES: &'static [&'static str] = &[$(stringify!($name),)*];

            /// Every counter's value, in `NAMES` order.
            pub fn values(&self) -> Vec<u64> {
                vec![$(self.$name,)*]
            }
        }
    };
}

statistics_fields! {
    ands_eliminated, // STATISTIC
    ands_extracted, // METRIC (re-enabled)
    arena_enlarged, // METRIC (re-enabled)
    arena_garbage, // METRIC (re-enabled)
    arena_resized, // METRIC (re-enabled)
    arena_shrunken, // METRIC (re-enabled)
    backbone_computations,
    backbone_implied, // METRIC (re-enabled)
    backbone_probes, // METRIC (re-enabled)
    backbone_propagations, // METRIC (re-enabled)
    backbone_rounds, // METRIC (re-enabled)
    backbone_ticks,
    backbone_units, // STATISTIC
    best_saved, // METRIC (re-enabled)
    chronological,
    clauses_added,
    clauses_binary,
    clauses_deleted, // STATISTIC
    clauses_factored, // STATISTIC
    clauses_improved, // STATISTIC
    clauses_irredundant,
    clauses_kept1, // STATISTIC
    clauses_kept2, // STATISTIC
    clauses_kept3, // STATISTIC
    clauses_learned,
    clauses_original,
    clauses_promoted1, // STATISTIC
    clauses_promoted2, // STATISTIC
    clauses_reduced, // STATISTIC
    clauses_reduced_tier1, // STATISTIC
    clauses_reduced_tier2, // STATISTIC
    clauses_reduced_tier3, // STATISTIC
    clauses_redundant,
    clauses_unfactored, // STATISTIC
    clauses_used,
    clauses_used_focused,
    clauses_used_stable,
    closures,
    compacted, // METRIC (re-enabled)
    conflicts,
    congruent,
    congruent_ands, // STATISTIC
    congruent_arity, // STATISTIC
    congruent_arity_ands, // STATISTIC
    congruent_arity_xors, // STATISTIC
    congruent_binaries, // STATISTIC
    congruent_ites, // STATISTIC
    congruent_collisions, // STATISTIC
    congruent_collisions_find, // STATISTIC
    congruent_collisions_index, // STATISTIC
    congruent_collisions_removed, // STATISTIC
    congruent_equivalences, // STATISTIC
    congruent_gates,
    congruent_gates_ands,
    congruent_gates_ites,
    congruent_gates_xors,
    congruent_indexed, // STATISTIC
    congruent_lookups, // STATISTIC
    congruent_lookups_find, // STATISTIC
    congruent_lookups_removed, // STATISTIC
    congruent_matched,
    congruent_matched_ands,
    congruent_matched_ites,
    congruent_matched_xors,
    congruent_rewritten, // STATISTIC
    congruent_rewritten_ands, // STATISTIC
    congruent_rewritten_ites, // STATISTIC
    congruent_rewritten_xors, // STATISTIC
    congruent_simplified, // STATISTIC
    congruent_simplified_ands, // STATISTIC
    congruent_simplified_ites, // STATISTIC
    congruent_simplified_xors, // STATISTIC
    congruent_subsumed, // STATISTIC
    congruent_trivial_ite, // STATISTIC
    congruent_unary, // STATISTIC
    congruent_unary_ands, // STATISTIC
    congruent_unary_ites, // STATISTIC
    congruent_unary_xors, // STATISTIC
    congruent_units, // STATISTIC
    congruent_xors, // STATISTIC
    decisions,
    definitions_checked, // METRIC (re-enabled)
    definitions_eliminated, // STATISTIC
    definitions_extracted, // METRIC (re-enabled)
    definition_units, // STATISTIC
    defragmentations, // METRIC (re-enabled)
    dense_garbage_collections, // METRIC (re-enabled)
    dense_propagations, // METRIC (re-enabled)
    dense_ticks, // METRIC (re-enabled)
    duplicated, // METRIC (re-enabled)
    eagerly_subsumed, // STATISTIC
    eliminate_attempted, // STATISTIC
    eliminated,
    eliminate_resolutions,
    eliminate_units, // STATISTIC
    eliminations,
    equivalences_eliminated, // STATISTIC
    equivalences_extracted, // METRIC (re-enabled)
    factored,
    factorizations,
    factor_ticks,
    fast_eliminated,
    fast_strengthened,
    fast_subsumed,
    fresh, // STATISTIC
    flipped, // STATISTIC
    flushed, // METRIC (re-enabled)
    focused_decisions, // METRIC (re-enabled)
    focused_modes, // METRIC (re-enabled)
    focused_propagations, // METRIC (re-enabled)
    focused_restarts, // METRIC (re-enabled)
    focused_ticks, // METRIC (re-enabled)
    forward_checks,
    forward_steps,
    forward_strengthened, // STATISTIC
    forward_subsumed, // STATISTIC
    forward_subsumptions, // METRIC (re-enabled)
    garbage_collections, // METRIC (re-enabled)
    gates_checked, // METRIC (re-enabled)
    gates_eliminated, // STATISTIC
    gates_extracted, // METRIC (re-enabled)
    if_then_else_eliminated, // STATISTIC
    if_then_else_extracted, // METRIC (re-enabled)
    initial_decisions, // METRIC (re-enabled)
    iterations,
    jumped_reasons, // STATISTIC
    kitten_conflicts, // STATISTIC
    kitten_decisions, // STATISTIC
    kitten_flip, // STATISTIC
    kitten_flipped, // STATISTIC
    kitten_propagations,
    kitten_sat, // STATISTIC
    kitten_solved,
    kitten_ticks,
    kitten_unknown, // STATISTIC
    kitten_unsat, // STATISTIC
    literals_bumped, // METRIC (re-enabled)
    literals_deduced, // METRIC (re-enabled)
    literals_factor,
    literals_factored, // STATISTIC
    literals_learned, // METRIC (re-enabled)
    literals_minimized, // METRIC (re-enabled)
    literals_minshrunken, // METRIC (re-enabled)
    literals_shrunken, // METRIC (re-enabled)
    literals_unfactored, // STATISTIC
    moved, // METRIC (re-enabled)
    on_the_fly_strengthened, // STATISTIC
    on_the_fly_subsumed, // STATISTIC
    probing_propagations, // METRIC (re-enabled)
    probings,
    probing_ticks,
    propagations,
    queue_decisions, // STATISTIC
    random_decisions, // STATISTIC
    random_sequences,
    reductions,
    reordered,
    reordered_focused, // STATISTIC
    reordered_stable, // STATISTIC
    rephased,
    rephased_best, // METRIC (re-enabled)
    rephased_inverted, // METRIC (re-enabled)
    rephased_original, // METRIC (re-enabled)
    rephased_walking, // METRIC (re-enabled)
    rescaled, // METRIC (re-enabled)
    restarts,
    restarts_levels, // STATISTIC
    restarts_reused_levels, // STATISTIC
    restarts_reused_trails, // STATISTIC
    retiered,
    saved_decisions, // METRIC (re-enabled)
    score_decisions, // METRIC (re-enabled)
    searches,
    search_propagations, // METRIC (re-enabled)
    search_ticks,
    sparse_gcs, // METRIC (re-enabled)
    stable_decisions, // METRIC (re-enabled)
    stable_modes, // METRIC (re-enabled)
    stable_propagations, // METRIC (re-enabled)
    stable_restarts, // METRIC (re-enabled)
    stable_ticks, // METRIC (re-enabled)
    strengthened,
    substituted,
    substitute_ticks,
    substitute_units, // STATISTIC
    substitutions, // STATISTIC
    subsumed,
    subsumption_checks,
    sweep,
    sweep_clauses, // STATISTIC
    sweep_completed,
    sweep_depth, // STATISTIC
    sweep_environment, // STATISTIC
    sweep_equivalences,
    sweep_fixed_backbone, // STATISTIC
    sweep_flip_backbone, // STATISTIC
    sweep_flipped_backbone, // STATISTIC
    sweep_flip_equivalences, // STATISTIC
    sweep_flipped_equivalences, // STATISTIC
    sweep_sat, // STATISTIC
    sweep_sat_backbone, // STATISTIC
    sweep_sat_equivalences, // STATISTIC
    sweep_solved,
    sweep_solved_backbone, // STATISTIC
    sweep_solved_equivalences, // STATISTIC
    sweep_unknown_backbone, // STATISTIC
    sweep_unknown_equivalences, // STATISTIC
    sweep_units,
    sweep_unsat, // STATISTIC
    sweep_unsat_backbone, // STATISTIC
    sweep_unsat_equivalences, // STATISTIC
    sweep_variables, // STATISTIC
    switched,
    target_decisions, // METRIC (re-enabled)
    target_saved, // METRIC (re-enabled)
    ticks, // STATISTIC
    transitive_probes, // METRIC (re-enabled)
    transitive_propagations, // METRIC (re-enabled)
    transitive_reduced, // METRIC (re-enabled)
    transitive_reductions, // METRIC (re-enabled)
    transitive_ticks,
    transitive_units, // METRIC (re-enabled)
    units,
    variables_activated,
    variables_eliminate,
    variables_extension,
    variables_factor,
    variables_original,
    variables_subsume,
    vectors_defrags_needed, // METRIC (re-enabled)
    vectors_enlarged, // METRIC (re-enabled)
    vivifications,
    vivified,
    vivified_asym, // STATISTIC
    vivified_implied, // STATISTIC
    vivified_instantiated, // STATISTIC
    vivified_instirr, // STATISTIC
    vivified_instred, // STATISTIC
    vivified_irredundant, // STATISTIC
    vivified_promoted, // STATISTIC
    vivified_shrunken, // STATISTIC
    vivified_shrunkirr, // STATISTIC
    vivified_shrunkred, // STATISTIC
    vivified_subirr, // STATISTIC
    vivified_subred, // STATISTIC
    vivified_subsumed, // STATISTIC
    vivified_tier1, // STATISTIC
    vivified_tier2, // STATISTIC
    vivified_tier3, // STATISTIC
    vivified_unlearn, // STATISTIC
    vivify_checks,
    vivify_probes,
    vivify_propagations, // STATISTIC
    vivify_reused,
    vivify_ticks, // STATISTIC
    vivify_units, // STATISTIC
    walk_decisions, // METRIC (re-enabled)
    walk_improved, // STATISTIC
    walks,
    walk_steps,
    warming_conflicts, // STATISTIC
    warming_decisions,
    warming_propagations,
    warmups,
    weakened, // METRIC (re-enabled)
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
    use super::*;

    #[test]
    fn field_list_is_complete_unique_and_matches_values() {
        let names = Statistics::NAMES;
        let st = Statistics::default();
        assert_eq!(names.len(), st.values().len());
        let mut sorted: Vec<&str> = names.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate counter name");
        for must in ["conflicts", "ticks", "eliminate_resolutions", "literals_learned",
                     "search_propagations", "vivify_ticks", "arena_garbage", "weakened"] {
            assert!(names.contains(&must), "{} missing", must);
        }
        // Values follow the field order.
        let mut st = Statistics::default();
        st.conflicts = 7;
        let i = names.iter().position(|n| *n == "conflicts").unwrap();
        assert_eq!(st.values()[i], 7);
    }

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

// ---------------------------------------------------------------------------
// Work clock — NOT in kissat. RL scheduler plan §3.4 / §10 step A′.
//
// W = ticks + K_RES × eliminate_resolutions is the deterministic work unit the
// harness prices cells in (tick PAR-2, CLAUDE.md "Evaluation") and the unit
// SAT_LIMIT_TICKS will limit (plan step A). `ticks` is kissat's never-printed
// all-propagation counter (search, probing, backbone, beyond, initially,
// transitive and the dense propagation inside eliminate all add to it);
// eliminate's own effort has no tick equivalent, so its resolutions are
// weighted in with K_RES.
//
// K_RES is PROVISIONAL. It was set 2026-09-15 from a `--profile=2` run of
// the 20 discriminating cells (README, "Work clock"): eliminate wall time
// per resolution, converted to search-tick units with each cell's own
// search ticks per second (median 11.5, geomean 10.8 over 13 cells). Plan
// step B refits it from the stock traces. Keep it here, in one place; the
// harness reads it back from the `c workclock` line and never hard-codes it.
pub const K_RES: u64 = 11;

impl Statistics {
    /// W = ticks + K_RES × eliminate_resolutions (plan §3.4).
    pub fn work_clock(&self) -> u64 {
        self.ticks
            .saturating_add(K_RES.saturating_mul(self.eliminate_resolutions))
    }
}

/// The `c workclock ...` exit line: every work kind the RL reward weights
/// (plan §4), the three headline search counters, K_RES and W itself, as
/// `key=value` pairs. No colon after the first word, so `tools/parity.py`'s
/// counter regex (`^c name:`) can never mistake it for an `-s` row.
pub fn work_clock_line(st: &Statistics, prefix: &str) -> String {
    format!(
        "{}workclock ticks={} search_ticks={} probing_ticks={} backbone_ticks={} \
         transitive_ticks={} factor_ticks={} substitute_ticks={} kitten_ticks={} \
         eliminate_resolutions={} forward_steps={} walk_steps={} flipped={} \
         conflicts={} decisions={} propagations={} k_res={} work={}",
        prefix,
        st.ticks,
        st.search_ticks,
        st.probing_ticks,
        st.backbone_ticks,
        st.transitive_ticks,
        st.factor_ticks,
        st.substitute_ticks,
        st.kitten_ticks,
        st.eliminate_resolutions,
        st.forward_steps,
        st.walk_steps,
        st.flipped,
        st.conflicts,
        st.decisions,
        st.propagations,
        K_RES,
        st.work_clock(),
    )
}

/// Print the work-clock line. Called at the end of
/// `internal::print_statistics`, i.e. on every exit path kissat prints its
/// statistics on (normal exit and the signal handler, so a run killed by the
/// harness `timeout` still reports the work it consumed), after the
/// `[ resources ]` section and therefore OUTSIDE the `-s` statistics block
/// that `tools/parity.py` diffs against the C binary.
pub fn print_work_clock(solver: &Solver) {
    let line = work_clock_line(&solver.statistics, &solver.prefix);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = out.write_all(line.as_bytes());
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

#[cfg(test)]
mod work_clock_tests {
    use super::*;

    #[test]
    fn work_clock_weights_resolutions() {
        let mut st = Statistics::default();
        st.ticks = 1000;
        st.eliminate_resolutions = 7;
        assert_eq!(st.work_clock(), 1000 + K_RES * 7);
        st.ticks = u64::MAX;
        assert_eq!(st.work_clock(), u64::MAX, "saturates instead of wrapping");
    }

    #[test]
    fn work_clock_line_is_key_value_and_not_a_stat_row() {
        let mut st = Statistics::default();
        st.ticks = 1000;
        st.search_ticks = 600;
        st.eliminate_resolutions = 7;
        st.conflicts = 3;
        let line = work_clock_line(&st, "c ");
        assert!(line.starts_with("c workclock ticks=1000 search_ticks=600 "));
        assert!(line.ends_with(&format!(" k_res={} work={}", K_RES, 1000 + K_RES * 7)));
        assert!(line.contains(" eliminate_resolutions=7 "));
        assert!(line.contains(" conflicts=3 "));
        // parity.py's STAT_RE is `^c ([a-z_0-9]+):\s+(\d+)`; the line has no
        // `name:` token, so it can never be read as an `-s` counter.
        assert!(!line.split_whitespace().any(|tok| tok.ends_with(':')));
        for tok in line.split_whitespace().skip(2) {
            let (_, v) = tok.split_once('=').expect("key=value");
            assert!(v.parse::<u64>().is_ok(), "{tok}");
        }
    }
}
