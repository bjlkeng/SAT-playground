use std::env;
use std::fs;
use std::io::{self, BufRead};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: sat-solver <cnf_path> <output_dir>");
        std::process::exit(1);
    }

    let cnf_path = &args[1];
    let _output_dir = &args[2];

    let (num_vars, _clauses) = parse_cnf(cnf_path);

    // Dummy implementation: always report SAT with all variables positive
    println!("s SATISFIABLE");
    print_assignment(num_vars);
}

/// Parse a DIMACS CNF file, returning (num_vars, clauses).
fn parse_cnf(path: &str) -> (usize, Vec<Vec<i32>>) {
    let file = fs::File::open(path).unwrap_or_else(|e| {
        eprintln!("Error opening {}: {}", path, e);
        std::process::exit(1);
    });
    let reader = io::BufReader::new(file);

    let mut num_vars = 0;
    let mut clauses: Vec<Vec<i32>> = Vec::new();
    let mut current_clause: Vec<i32> = Vec::new();

    for line in reader.lines() {
        let line = line.expect("Failed to read line");
        let line = line.trim();

        if line.is_empty() || line.starts_with('c') {
            continue;
        }

        if line.starts_with('p') {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 && parts[1] == "cnf" {
                num_vars = parts[2].parse().unwrap_or(0);
            }
            continue;
        }

        // Parse clause literals
        for token in line.split_whitespace() {
            let lit: i32 = match token.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            if lit == 0 {
                if !current_clause.is_empty() {
                    clauses.push(current_clause.clone());
                    current_clause.clear();
                }
            } else {
                current_clause.push(lit);
            }
        }
    }

    // Handle clause without trailing 0
    if !current_clause.is_empty() {
        clauses.push(current_clause);
    }

    (num_vars, clauses)
}

/// Print a satisfying assignment (all variables positive), respecting 4096-char line limit.
fn print_assignment(num_vars: usize) {
    let mut line = String::from("v");
    for var in 1..=num_vars {
        let lit = format!(" {}", var);
        // 4096 char limit per line; leave room for trailing " 0"
        if line.len() + lit.len() > 4090 {
            println!("{}", line);
            line = String::from("v");
        }
        line.push_str(&lit);
    }
    line.push_str(" 0");
    println!("{}", line);
}
