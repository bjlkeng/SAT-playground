// Port of src/config.c + src/config.h (kissat 4.0.4).
//
// The reference build has NOPTIONS undefined, so the real (option-mutating)
// variant is ported; the NOPTIONS dummy is not.
//
// PORT NOTE: C `kissat_set_configuration` takes `kissat *solver` and goes
// through `kissat_set_option` -> `kissat_options_set` (the CLAMPING setter,
// not the rejecting `options_parse_arg` path). Here the functions take
// `&mut Options` directly, per the port interface; the effect is identical.

use crate::options::{
    self, Options, RESTARTINT_SAT, STABLE_UNSAT, TARGET_SAT,
};

/// C: `kissat_has_configuration` (returns int 0/1 in C).
pub fn has_configuration(name: &str) -> bool {
    if name == "basic" {
        return true;
    }
    if name == "default" {
        return true;
    }
    if name == "plain" {
        return true;
    }
    if name == "sat" {
        return true;
    }
    if name == "unsat" {
        return true;
    }
    false
}

/// C: `kissat_configuration_usage` — `FMT` is `"  --%-8s %s"`.
pub fn configuration_usage() {
    println!(
        "  --{:<8} {}",
        "basic",
        "basic CDCL solving ('--plain' but no restarts, minimize, reduce)"
    );
    println!("  --{:<8} {}", "default", "default configuration");
    println!(
        "  --{:<8} {}",
        "plain", "plain CDCL solving without advanced techniques"
    );
    println!(
        "  --{:<8} {} ('--target={} --restartint={}')",
        "sat", "target satisfiable instances", TARGET_SAT, RESTARTINT_SAT
    );
    println!(
        "  --{:<8} {} ('--stable={}')",
        "unsat", "target unsatisfiable instances", STABLE_UNSAT
    );
}

/// C: `set_plain_options` (static in config.c).
fn set_plain_options(options: &mut Options) {
    options::options_set(options, "bumpreasons", 0);
    options::options_set(options, "chrono", 0);
    options::options_set(options, "compact", 0);
    options::options_set(options, "eagersubsume", 0);
    options::options_set(options, "jumpreasons", 0);
    options::options_set(options, "otfs", 0);
    options::options_set(options, "preprocess", 0);
    options::options_set(options, "reorder", 0);
    options::options_set(options, "rephase", 0);
    options::options_set(options, "restartreusetrail", 0);
    options::options_set(options, "simplify", 0);
    options::options_set(options, "stable", 2);
    options::options_set(options, "tumble", 0);
}

/// C: `kissat_set_configuration` (returns int 0/1 in C).
pub fn set_configuration(options: &mut Options, name: &str) -> bool {
    if name == "basic" {
        set_plain_options(options);
        options::options_set(options, "restart", 0);
        options::options_set(options, "reduce", 0);
        options::options_set(options, "minimize", 0);
        return true;
    }
    if name == "default" {
        return true;
    }
    if name == "plain" {
        set_plain_options(options);
        return true;
    }
    if name == "sat" {
        options::options_set(options, "target", TARGET_SAT);
        options::options_set(options, "restartint", RESTARTINT_SAT);
        return true;
    }
    if name == "unsat" {
        options::options_set(options, "stable", STABLE_UNSAT);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_configurations() {
        for name in ["basic", "default", "plain", "sat", "unsat"] {
            assert!(has_configuration(name));
            let mut options = Options::default();
            assert!(set_configuration(&mut options, name));
        }
        assert!(!has_configuration("simp"));
        let mut options = Options::default();
        assert!(!set_configuration(&mut options, "simp"));
        assert_eq!(options, Options::default()); // unknown name changes nothing
    }

    #[test]
    fn default_configuration_is_identity() {
        let mut options = Options::default();
        assert!(set_configuration(&mut options, "default"));
        assert_eq!(options, Options::default());
    }

    #[test]
    fn plain_sets_the_thirteen_plain_options() {
        let mut options = Options::default();
        assert!(set_configuration(&mut options, "plain"));
        assert_eq!(options.bumpreasons, 0);
        assert_eq!(options.chrono, 0);
        assert_eq!(options.compact, 0);
        assert_eq!(options.eagersubsume, 0);
        assert_eq!(options.jumpreasons, 0);
        assert_eq!(options.otfs, 0);
        assert_eq!(options.preprocess, 0);
        assert_eq!(options.reorder, 0);
        assert_eq!(options.rephase, 0);
        assert_eq!(options.restartreusetrail, 0);
        assert_eq!(options.simplify, 0);
        assert_eq!(options.stable, 2);
        assert_eq!(options.tumble, 0);
        // untouched by plain:
        assert_eq!(options.restart, 1);
        assert_eq!(options.reduce, 1);
        assert_eq!(options.minimize, 1);
    }

    #[test]
    fn basic_is_plain_without_restart_reduce_minimize() {
        let mut plain = Options::default();
        set_configuration(&mut plain, "plain");
        let mut basic = Options::default();
        set_configuration(&mut basic, "basic");
        assert_eq!(basic.restart, 0);
        assert_eq!(basic.reduce, 0);
        assert_eq!(basic.minimize, 0);
        let mut plain_adjusted = plain;
        plain_adjusted.restart = 0;
        plain_adjusted.reduce = 0;
        plain_adjusted.minimize = 0;
        assert_eq!(basic, plain_adjusted);
    }

    #[test]
    fn sat_and_unsat() {
        let mut options = Options::default();
        set_configuration(&mut options, "sat");
        assert_eq!(options.target, 2);
        assert_eq!(options.restartint, 50);
        let mut options = Options::default();
        set_configuration(&mut options, "unsat");
        assert_eq!(options.stable, 0);
    }
}
