use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;

/// A literal is a non-zero i32. Positive = true, negative = negated.
/// Variables are 1-based.

#[derive(Clone, Copy, PartialEq, Eq)]
enum LitState {
    True,
    False,
    Unassigned,
}

struct Solver {
    num_vars: usize,
    clauses: Vec<Vec<i32>>,
    /// assignment[v] for variable v (1-based, index 0 unused)
    assignment: Vec<Option<bool>>,
    /// DRAT proof clauses (each clause is a vec of literals, empty vec = empty clause)
    proof: Vec<Vec<i32>>,
}

impl Solver {
    fn new(num_vars: usize, clauses: Vec<Vec<i32>>) -> Self {
        Solver {
            num_vars,
            clauses,
            assignment: vec![None; num_vars + 1],
            proof: Vec::new(),
        }
    }

    fn lit_state(&self, lit: i32) -> LitState {
        let var = lit.unsigned_abs() as usize;
        match self.assignment[var] {
            None => LitState::Unassigned,
            Some(val) => {
                let polarity = lit > 0;
                if val == polarity {
                    LitState::True
                } else {
                    LitState::False
                }
            }
        }
    }

    fn assign(&mut self, lit: i32) {
        let var = lit.unsigned_abs() as usize;
        self.assignment[var] = Some(lit > 0);
    }

    fn unassign(&mut self, var: usize) {
        self.assignment[var] = None;
    }

    /// Returns the state of a clause: true if satisfied, false if all lits false (conflict),
    /// or the list of unassigned literals if undetermined.
    fn clause_state(&self, clause: &[i32]) -> ClauseState {
        let mut unassigned = Vec::new();
        for &lit in clause {
            match self.lit_state(lit) {
                LitState::True => return ClauseState::Satisfied,
                LitState::False => {}
                LitState::Unassigned => unassigned.push(lit),
            }
        }
        if unassigned.is_empty() {
            ClauseState::Conflict
        } else if unassigned.len() == 1 {
            ClauseState::Unit(unassigned[0])
        } else {
            ClauseState::Undetermined
        }
    }

    /// Run unit propagation. Returns true if no conflict, false if conflict found.
    /// Pushes assigned variables onto `trail` so they can be undone on backtrack.
    fn unit_propagate(&mut self, trail: &mut Vec<usize>) -> bool {
        let mut changed = true;
        while changed {
            changed = false;
            for i in 0..self.clauses.len() {
                match self.clause_state(&self.clauses[i]) {
                    ClauseState::Conflict => return false,
                    ClauseState::Unit(lit) => {
                        self.assign(lit);
                        trail.push(lit.unsigned_abs() as usize);
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
        true
    }

    /// Pick the first unassigned variable (simple heuristic).
    fn pick_variable(&self) -> Option<usize> {
        for v in 1..=self.num_vars {
            if self.assignment[v].is_none() {
                return Some(v);
            }
        }
        None
    }

    /// Check if all clauses are satisfied.
    fn all_satisfied(&self) -> bool {
        self.clauses.iter().all(|c| {
            c.iter().any(|&lit| self.lit_state(lit) == LitState::True)
        })
    }

    /// Main DPLL search. Returns true if SAT, false if UNSAT.
    fn solve(&mut self) -> bool {
        // Check for empty clauses upfront
        if self.clauses.iter().any(|c| c.is_empty()) {
            self.proof.push(vec![]);
            return false;
        }
        self.dpll()
    }

    fn dpll(&mut self) -> bool {
        // Unit propagation
        let mut trail = Vec::new();
        if !self.unit_propagate(&mut trail) {
            // Undo propagated assignments
            for &var in &trail {
                self.unassign(var);
            }
            return false;
        }

        // Check if all clauses satisfied
        if self.all_satisfied() {
            return true;
        }

        // Pick a branching variable
        let var = match self.pick_variable() {
            Some(v) => v,
            None => {
                // All assigned but not all satisfied → conflict
                for &v in &trail {
                    self.unassign(v);
                }
                return false;
            }
        };

        // Try positive then negative
        for &polarity in &[true, false] {
            let lit = if polarity { var as i32 } else { -(var as i32) };
            self.assign(lit);

            if self.dpll() {
                return true;
            }

            self.unassign(var);
        }

        // Both branches failed — undo unit propagation trail and backtrack
        for &v in &trail {
            self.unassign(v);
        }
        false
    }
}

enum ClauseState {
    Satisfied,
    Conflict,
    Unit(i32),
    Undetermined,
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

        for token in line.split_whitespace() {
            let lit: i32 = match token.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            if lit == 0 {
                clauses.push(current_clause.clone());
                current_clause.clear();
            } else {
                current_clause.push(lit);
            }
        }
    }

    if !current_clause.is_empty() {
        clauses.push(current_clause);
    }

    (num_vars, clauses)
}

/// Write DRAT proof to file.
fn write_proof(output_dir: &str, proof: &[Vec<i32>]) {
    let proof_path = Path::new(output_dir).join("proof.out");
    let mut file = fs::File::create(&proof_path).unwrap_or_else(|e| {
        eprintln!("Error creating {}: {}", proof_path.display(), e);
        std::process::exit(1);
    });

    for clause in proof {
        let line: String = clause
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(file, "{} 0", line).expect("Failed to write proof");
    }
}

/// Print a satisfying assignment, respecting 4096-char line limit.
fn print_assignment(assignment: &[Option<bool>]) {
    let mut line = String::from("v");
    // Skip index 0 (unused)
    for var in 1..assignment.len() {
        let lit = match assignment[var] {
            Some(true) => var as i32,
            Some(false) => -(var as i32),
            None => var as i32, // unassigned → default to positive (don't-care)
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

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: sat-solver <cnf_path> <output_dir>");
        std::process::exit(1);
    }

    let cnf_path = &args[1];
    let output_dir = &args[2];

    let (num_vars, clauses) = parse_cnf(cnf_path);
    let mut solver = Solver::new(num_vars, clauses);

    if solver.solve() {
        println!("s SATISFIABLE");
        print_assignment(&solver.assignment);
    } else {
        println!("s UNSATISFIABLE");
        // For basic DPLL, write minimal DRAT proof: just the empty clause
        if solver.proof.is_empty() {
            solver.proof.push(vec![]);
        }
        write_proof(output_dir, &solver.proof);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_solver(num_vars: usize, clauses: Vec<Vec<i32>>) -> Solver {
        Solver::new(num_vars, clauses)
    }

    #[test]
    fn test_unit_clause_sat() {
        // x1 = true
        let mut s = make_solver(1, vec![vec![1]]);
        assert!(s.solve());
        assert_eq!(s.assignment[1], Some(true));
    }

    #[test]
    fn test_contradiction_unsat() {
        // x1 AND NOT x1
        let mut s = make_solver(1, vec![vec![1], vec![-1]]);
        assert!(!s.solve());
    }

    #[test]
    fn test_empty_clause_unsat() {
        // Contains empty clause
        let mut s = make_solver(2, vec![vec![1, 2], vec![]]);
        assert!(!s.solve());
    }

    #[test]
    fn test_two_clause_sat() {
        // (x1) AND (x2)
        let mut s = make_solver(2, vec![vec![1], vec![2]]);
        assert!(s.solve());
        assert_eq!(s.assignment[1], Some(true));
        assert_eq!(s.assignment[2], Some(true));
    }

    #[test]
    fn test_chain_unsat() {
        // x1, -1 2, -2 3, -3  (chain forces contradiction)
        let mut s = make_solver(3, vec![vec![1], vec![-1, 2], vec![-2, 3], vec![-3]]);
        assert!(!s.solve());
    }

    #[test]
    fn test_three_sat_instance() {
        let clauses = vec![
            vec![1, 2, 3],
            vec![-1, 2, 4],
            vec![1, -3, 5],
            vec![-2, 4, 5],
            vec![-1, -4, -5],
            vec![3, 4, -2],
        ];
        let mut s = make_solver(5, clauses.clone());
        assert!(s.solve());
        // Verify all clauses satisfied
        for clause in &clauses {
            let sat = clause.iter().any(|&lit| {
                let var = lit.unsigned_abs() as usize;
                match s.assignment[var] {
                    Some(val) => val == (lit > 0),
                    None => false,
                }
            });
            assert!(sat, "Clause {:?} not satisfied", clause);
        }
    }

    #[test]
    fn test_pigeonhole_3_2_unsat() {
        let clauses = vec![
            vec![1, 2],
            vec![3, 4],
            vec![5, 6],
            vec![-1, -3],
            vec![-1, -5],
            vec![-3, -5],
            vec![-2, -4],
            vec![-2, -6],
            vec![-4, -6],
        ];
        let mut s = make_solver(6, clauses);
        assert!(!s.solve());
    }

    #[test]
    fn test_no_clauses_sat() {
        let s_result = make_solver(3, vec![]);
        // No clauses → trivially SAT
        let mut s = s_result;
        assert!(s.solve());
    }
}
