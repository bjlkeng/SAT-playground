//! SAT Competition output helpers.
//!
//! This module owns stdout formatting details that are independent from the
//! search algorithm. Proof file creation is still in `main.rs` until the proof
//! boundary is extracted by a later task.

use crate::{FALSE, UNASSIGNED};

pub(crate) fn print_assignment(assignment: &[u8]) {
    let mut line = String::from("v");
    for var in 1..assignment.len() {
        assert_ne!(
            assignment[var], UNASSIGNED,
            "SAT model snapshot left variable {var} unassigned"
        );
        let lit = if assignment[var] == FALSE {
            -(var as i32)
        } else {
            var as i32
        };
        let token = format!(" {}", lit);
        if line.len() + token.len() > 4090 {
            println!("{}", line);
            line = String::from("v");
        }
        line.push_str(&token);
    }
    line.push_str(" 0");
    println!("{}", line);
}
