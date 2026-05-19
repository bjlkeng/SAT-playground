//! Configuration parsing boundary for solver 11.
//!
//! Task 0.1 keeps this deliberately small: it centralizes the existing
//! environment helpers without changing defaults. Task 0.3 grows this into the
//! full `SolverConfig` object, schema, dump, and replay contract.

use std::env;

pub(crate) fn parse_bool_env(name: &str, default: bool) -> bool {
    match env::var(name) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" | "enabled" => true,
            "0" | "false" | "no" | "off" | "disabled" => false,
            other => {
                eprintln!("Invalid {name}={other}; expected boolean");
                std::process::exit(2);
            }
        },
        Err(_) => default,
    }
}

pub(crate) fn parse_use_resolved_conflict_analysis() -> bool {
    match env::var("SAT_CONFLICT_ANALYSIS_MODE") {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "minisat" | "mini" | "seen" => false,
            "resolved" | "solver10" | "legacy" => true,
            other => {
                eprintln!("Invalid SAT_CONFLICT_ANALYSIS_MODE={other}; expected minisat/resolved");
                std::process::exit(2);
            }
        },
        Err(_) => false,
    }
}

pub(crate) fn parse_usize_env(name: &str, default: usize) -> usize {
    match env::var(name) {
        Ok(value) => match value.trim().parse::<usize>() {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("Invalid {name}={value:?}: {err}");
                std::process::exit(2);
            }
        },
        Err(_) => default,
    }
}

pub(crate) fn parse_optional_usize_env(name: &str) -> usize {
    match env::var(name) {
        Ok(value) => match value.trim().parse::<usize>() {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("Invalid {name}={value:?}: {err}");
                std::process::exit(2);
            }
        },
        Err(_) => 0,
    }
}
