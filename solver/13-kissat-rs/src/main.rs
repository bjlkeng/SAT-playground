// Faithful Rust reimplementation of kissat 4.0.4 (solver 13).
// main.rs mirrors main.c + (eventually) application.c.

pub mod arena;
pub mod assign;
pub mod averages;
pub mod clause;
pub mod collect;
pub mod colors;
pub mod config;
pub mod error;
pub mod extend;
pub mod file;
pub mod flags;
pub mod format;
pub mod frames;
pub mod heap;
pub mod inline;
pub mod inlinequeue;
pub mod internal;
pub mod kimits;
pub mod literal;
pub mod mode;
pub mod options;
pub mod parse;
pub mod phases;
pub mod print;
pub mod profile;
pub mod proof;
pub mod queue;
pub mod random;
pub mod reference;
pub mod reluctant;
pub mod report;
pub mod resources;
pub mod smooth;
pub mod sort;
pub mod statistics;
pub mod stubs;
pub use stubs::{backtrack, bump, decide, import, propsearch, resize, restart, search};
pub mod tiers;
pub mod utilities;
pub mod value;
pub mod vector;
pub mod watch;
pub mod witness;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut cnf: Option<&str> = None;
    for arg in &args[1..] {
        if arg.starts_with('-') {
            continue; // options handled once application.rs lands
        }
        if cnf.is_none() {
            cnf = Some(arg);
        }
    }
    // Scaffold state: interface-conformant placeholder until the core lands.
    println!("c kissat-rs (solver 13) scaffold");
    println!("s UNKNOWN");
    std::process::exit(0);
}
