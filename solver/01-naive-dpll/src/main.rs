use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;

const UNASSIGNED: u8 = 0;
const TRUE: u8 = 1;
const FALSE: u8 = 2;

enum ClauseState {
    Satisfied,
    Conflict,
    Unit(i32),
    Undetermined,
}

struct Solver {
    num_vars: usize,
    clauses: Vec<Box<[i32]>>,
    /// assignment[v] for variable v (1-based, index 0 unused)
    /// 0 = unassigned, 1 = true, 2 = false
    assignment: Vec<u8>,
    /// DRAT proof clauses
    proof: Vec<Vec<i32>>,
}

impl Solver {
    fn new(num_vars: usize, clauses: Vec<Vec<i32>>) -> Self {
        let clauses: Vec<Box<[i32]>> = clauses.into_iter().map(|c| c.into_boxed_slice()).collect();
        Solver {
            num_vars,
            clauses,
            assignment: vec![UNASSIGNED; num_vars + 1],
            proof: Vec::new(),
        }
    }

    #[inline(always)]
    fn lit_value(&self, lit: i32) -> u8 {
        let var = lit.unsigned_abs() as usize;
        let val = self.assignment[var];
        if val == UNASSIGNED {
            return UNASSIGNED;
        }
        if (lit > 0) == (val == TRUE) {
            TRUE
        } else {
            FALSE
        }
    }

    #[inline(always)]
    fn assign(&mut self, lit: i32) {
        let var = lit.unsigned_abs() as usize;
        self.assignment[var] = if lit > 0 { TRUE } else { FALSE };
    }

    #[inline(always)]
    fn unassign(&mut self, var: usize) {
        self.assignment[var] = UNASSIGNED;
    }

    #[inline(always)]
    fn clause_state(&self, clause: &[i32]) -> ClauseState {
        let mut unassigned_count = 0u32;
        let mut unassigned_lit = 0i32;
        for &lit in clause {
            let v = self.lit_value(lit);
            if v == TRUE {
                return ClauseState::Satisfied;
            }
            if v == UNASSIGNED {
                unassigned_count += 1;
                if unassigned_count == 1 {
                    unassigned_lit = lit;
                } else {
                    return ClauseState::Undetermined;
                }
            }
        }
        if unassigned_count == 0 {
            ClauseState::Conflict
        } else {
            ClauseState::Unit(unassigned_lit)
        }
    }

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

    fn pick_variable(&self) -> Option<usize> {
        for v in 1..=self.num_vars {
            if self.assignment[v] == UNASSIGNED {
                return Some(v);
            }
        }
        None
    }

    fn has_conflict(&self) -> bool {
        for clause in &self.clauses {
            let mut satisfied = false;
            for &lit in clause {
                if self.lit_value(lit) == TRUE {
                    satisfied = true;
                    break;
                }
            }
            if !satisfied {
                return true;
            }
        }
        false
    }

    fn solve(&mut self) -> bool {
        if self.clauses.iter().any(|c| c.is_empty()) {
            self.proof.push(vec![]);
            return false;
        }
        self.dpll()
    }

    fn dpll(&mut self) -> bool {
        let mut trail = Vec::with_capacity(self.num_vars);
        if !self.unit_propagate(&mut trail) {
            for &var in &trail {
                self.unassign(var);
            }
            return false;
        }

        let var = match self.pick_variable() {
            Some(v) => v,
            None => {
                if self.has_conflict() {
                    for &v in &trail {
                        self.unassign(v);
                    }
                    return false;
                }
                return true;
            }
        };

        // Try positive polarity
        self.assign(var as i32);
        if self.dpll() {
            return true;
        }
        self.unassign(var);

        // Try negative polarity
        self.assign(-(var as i32));
        if self.dpll() {
            return true;
        }
        self.unassign(var);

        for &v in &trail {
            self.unassign(v);
        }
        false
    }
}

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
                clauses.push(std::mem::take(&mut current_clause));
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

fn print_assignment(assignment: &[u8]) {
    let mut line = String::from("v");
    for var in 1..assignment.len() {
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
        let mut s = make_solver(1, vec![vec![1]]);
        assert!(s.solve());
        assert_eq!(s.assignment[1], TRUE);
    }

    #[test]
    fn test_contradiction_unsat() {
        let mut s = make_solver(1, vec![vec![1], vec![-1]]);
        assert!(!s.solve());
    }

    #[test]
    fn test_empty_clause_unsat() {
        let mut s = make_solver(2, vec![vec![1, 2], vec![]]);
        assert!(!s.solve());
    }

    #[test]
    fn test_two_clause_sat() {
        let mut s = make_solver(2, vec![vec![1], vec![2]]);
        assert!(s.solve());
        assert_eq!(s.assignment[1], TRUE);
        assert_eq!(s.assignment[2], TRUE);
    }

    #[test]
    fn test_chain_unsat() {
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
        for clause in &clauses {
            let sat = clause.iter().any(|&lit| s.lit_value(lit) == TRUE);
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
        let mut s = make_solver(3, vec![]);
        assert!(s.solve());
    }
}
