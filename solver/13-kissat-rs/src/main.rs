// Faithful Rust reimplementation of kissat 4.0.4 (solver 13).
// main.rs mirrors main.c + (eventually) application.c.

pub mod analyze;
pub mod application;
pub mod arena;
pub mod assign;
pub mod averages;
pub mod backtrack;
pub mod bump;
pub mod classify;
pub mod clause;
pub mod decide;
pub mod deduce;
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
pub mod import;
pub mod inline;
pub mod inlinequeue;
pub mod internal;
pub mod kimits;
pub mod learn;
pub mod literal;
pub mod lucky;
pub mod minimize;
pub mod mode;
pub mod options;
pub mod parse;
pub mod phases;
pub mod print;
pub mod profile;
pub mod promote;
pub mod proof;
pub mod propbeyond;
pub mod propinitially;
pub mod proprobe;
pub mod propsearch;
pub mod queue;
pub mod random;
pub mod reduce;
pub mod reference;
pub mod reluctant;
pub mod rephase;
pub mod report;
pub mod resize;
pub mod resources;
pub mod restart;
pub mod search;
pub mod shrink;
pub mod smooth;
pub mod sort;
pub mod statistics;
pub mod strengthen;
pub mod stubs;
pub use stubs::{compact, eliminate, krite, preprocess, probe, reorder, walk};
pub mod terminate;
pub mod tiers;
pub mod trail;
pub mod utilities;
pub mod value;
pub mod vector;
pub mod warmup;
pub mod watch;
pub mod weaken;
pub mod witness;

fn main() {
    // main.c's body (solver setup, signal handling, run, teardown) lives in
    // application::application; exit with its code (0/1/10/20).
    let args: Vec<String> = std::env::args().collect();
    let res = application::application(args);
    std::process::exit(res);
}
