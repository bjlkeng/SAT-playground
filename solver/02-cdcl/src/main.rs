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
    original_clause_count: usize,
    /// assignment[v] for variable v (1-based, index 0 unused)
    /// 0 = unassigned, 1 = true, 2 = false
    assignment: Vec<u8>,
    /// decision level of each variable assignment
    decision_level: Vec<usize>,
    /// reason clause index for each implied assignment; None for decisions/root-unassigned vars
    reason: Vec<Option<usize>>,
    /// assigned literals in chronological order
    trail: Vec<i32>,
    /// trail index where each decision level starts
    trail_limits: Vec<usize>,
    /// DRAT proof clauses
    proof: Vec<Vec<i32>>,
}

impl Solver {
    fn new(num_vars: usize, clauses: Vec<Vec<i32>>) -> Self {
        let clauses: Vec<Box<[i32]>> = clauses.into_iter().map(|c| c.into_boxed_slice()).collect();
        let original_clause_count = clauses.len();
        Solver {
            num_vars,
            clauses,
            original_clause_count,
            assignment: vec![UNASSIGNED; num_vars + 1],
            decision_level: vec![0; num_vars + 1],
            reason: vec![None; num_vars + 1],
            trail: Vec::with_capacity(num_vars),
            trail_limits: Vec::new(),
            proof: Vec::new(),
        }
    }

    #[inline(always)]
    fn current_level(&self) -> usize {
        self.trail_limits.len()
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
    fn clause_state(&self, clause: &[i32]) -> ClauseState {
        let mut unassigned_count = 0u32;
        let mut unassigned_lit = 0i32;
        for &lit in clause {
            match self.lit_value(lit) {
                TRUE => return ClauseState::Satisfied,
                UNASSIGNED => {
                    unassigned_count += 1;
                    if unassigned_count == 1 {
                        unassigned_lit = lit;
                    } else {
                        return ClauseState::Undetermined;
                    }
                }
                FALSE => {}
                _ => unreachable!(),
            }
        }

        if unassigned_count == 0 {
            ClauseState::Conflict
        } else {
            ClauseState::Unit(unassigned_lit)
        }
    }

    #[inline(always)]
    fn enqueue(&mut self, lit: i32, reason: Option<usize>) -> bool {
        match self.lit_value(lit) {
            TRUE => true,
            FALSE => false,
            UNASSIGNED => {
                let var = lit.unsigned_abs() as usize;
                self.assignment[var] = if lit > 0 { TRUE } else { FALSE };
                self.decision_level[var] = self.current_level();
                self.reason[var] = reason;
                self.trail.push(lit);
                true
            }
            _ => unreachable!(),
        }
    }

    fn propagate(&mut self) -> Option<usize> {
        loop {
            let trail_len_before = self.trail.len();
            for clause_idx in 0..self.clauses.len() {
                match self.clause_state(&self.clauses[clause_idx]) {
                    ClauseState::Satisfied | ClauseState::Undetermined => {}
                    ClauseState::Conflict => return Some(clause_idx),
                    ClauseState::Unit(lit) => {
                        if !self.enqueue(lit, Some(clause_idx)) {
                            return Some(clause_idx);
                        }
                    }
                }
            }

            if self.trail.len() == trail_len_before {
                return None;
            }
        }
    }

    fn decide(&mut self, lit: i32) {
        self.trail_limits.push(self.trail.len());
        let inserted = self.enqueue(lit, None);
        debug_assert!(inserted, "decision literal must be unassigned");
    }

    fn pick_branch_lit(&self) -> Option<i32> {
        for var in 1..=self.num_vars {
            if self.assignment[var] == UNASSIGNED {
                return Some(var as i32);
            }
        }
        None
    }

    fn backtrack(&mut self, target_level: usize) {
        let new_trail_len = if target_level == 0 {
            0
        } else {
            self.trail_limits[target_level - 1]
        };

        while self.trail.len() > new_trail_len {
            let lit = self.trail.pop().expect("trail underflow");
            let var = lit.unsigned_abs() as usize;
            self.assignment[var] = UNASSIGNED;
            self.decision_level[var] = 0;
            self.reason[var] = None;
        }

        self.trail_limits.truncate(target_level);
    }

    fn add_clause(&mut self, clause: Vec<i32>) -> usize {
        self.clauses.push(clause.into_boxed_slice());
        self.clauses.len() - 1
    }

    fn mark_clause_literals(
        &self,
        clause: &[i32],
        current_level: usize,
        seen: &mut [bool],
        resolved: &[bool],
        learned: &mut Vec<i32>,
        current_level_count: &mut usize,
    ) {
        for &lit in clause {
            let var = lit.unsigned_abs() as usize;
            if seen[var] || resolved[var] {
                continue;
            }

            let level = self.decision_level[var];
            if level == 0 {
                continue;
            }

            seen[var] = true;
            if level == current_level {
                *current_level_count += 1;
            } else {
                learned.push(lit);
            }
        }
    }

    fn analyze_conflict(&self, conflict_clause_idx: usize) -> (Vec<i32>, usize) {
        let current_level = self.current_level();
        let mut seen = vec![false; self.num_vars + 1];
        let mut resolved = vec![false; self.num_vars + 1];
        let mut learned = Vec::new();
        let mut current_level_count = 0usize;

        self.mark_clause_literals(
            &self.clauses[conflict_clause_idx],
            current_level,
            &mut seen,
            &resolved,
            &mut learned,
            &mut current_level_count,
        );

        debug_assert!(current_level_count > 0);

        let mut trail_index = self.trail.len();
        let uip_lit = loop {
            trail_index -= 1;
            let lit = self.trail[trail_index];
            let var = lit.unsigned_abs() as usize;
            if !seen[var] {
                continue;
            }

            seen[var] = false;
            resolved[var] = true;
            current_level_count -= 1;
            if current_level_count == 0 {
                break lit;
            }

            if let Some(reason_idx) = self.reason[var] {
                self.mark_clause_literals(
                    &self.clauses[reason_idx],
                    current_level,
                    &mut seen,
                    &resolved,
                    &mut learned,
                    &mut current_level_count,
                );
            }
        };

        learned.insert(0, -uip_lit);

        let mut backtrack_level = 0usize;
        for &lit in learned.iter().skip(1) {
            let var = lit.unsigned_abs() as usize;
            backtrack_level = backtrack_level.max(self.decision_level[var]);
        }

        (learned, backtrack_level)
    }

    fn learned_clause_count(&self) -> usize {
        self.clauses.len() - self.original_clause_count
    }

    fn solve(&mut self) -> bool {
        let mut conflict = self.propagate();

        loop {
            match conflict {
                Some(conflict_clause_idx) => {
                    if self.current_level() == 0 {
                        self.proof.push(vec![]);
                        return false;
                    }

                    let (learned_clause, backtrack_level) =
                        self.analyze_conflict(conflict_clause_idx);
                    let asserting_lit = learned_clause[0];
                    let learned_clause_idx = self.add_clause(learned_clause);

                    self.backtrack(backtrack_level);
                    let inserted = self.enqueue(asserting_lit, Some(learned_clause_idx));
                    debug_assert!(inserted, "learned clause must be asserting after backtrack");

                    conflict = self.propagate();
                }
                None => match self.pick_branch_lit() {
                    Some(lit) => {
                        self.decide(lit);
                        conflict = self.propagate();
                    }
                    None => return true,
                },
            }
        }
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
            .map(|lit| lit.to_string())
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

    #[test]
    fn test_cdcl_learns_clause_on_unsat_instance() {
        let clauses = vec![
            vec![1, 2],
            vec![-1, 2],
            vec![1, -2],
            vec![-1, -2],
        ];
        let mut s = make_solver(2, clauses);
        assert!(!s.solve());
        assert!(s.learned_clause_count() > 0);
    }
}
