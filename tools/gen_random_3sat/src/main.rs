use clap::Parser;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "gen-random-3sat", about = "Generate random 3-SAT instances at the phase transition")]
struct Cli {
    /// Number of variables
    #[arg(long)]
    vars: usize,

    /// Clause-to-variable ratio (default: 4.267, the phase transition)
    #[arg(long, default_value = "4.267")]
    ratio: f64,

    /// Random seed
    #[arg(long, default_value = "1")]
    seed: u64,

    /// Output file (default: stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,
}

fn generate_3sat(num_vars: usize, num_clauses: usize, rng: &mut StdRng) -> Vec<[i32; 3]> {
    let mut clauses = Vec::with_capacity(num_clauses);

    while clauses.len() < num_clauses {
        // Pick 3 distinct variables
        let mut vars = [0u32; 3];
        vars[0] = rng.gen_range(1..=num_vars as u32);
        loop {
            vars[1] = rng.gen_range(1..=num_vars as u32);
            if vars[1] != vars[0] {
                break;
            }
        }
        loop {
            vars[2] = rng.gen_range(1..=num_vars as u32);
            if vars[2] != vars[0] && vars[2] != vars[1] {
                break;
            }
        }

        // Negate each with 50% probability
        let lits: [i32; 3] = [
            if rng.gen_bool(0.5) { vars[0] as i32 } else { -(vars[0] as i32) },
            if rng.gen_bool(0.5) { vars[1] as i32 } else { -(vars[1] as i32) },
            if rng.gen_bool(0.5) { vars[2] as i32 } else { -(vars[2] as i32) },
        ];

        clauses.push(lits);
    }

    clauses
}

fn main() {
    let cli = Cli::parse();

    assert!(cli.vars >= 3, "need at least 3 variables for 3-SAT");

    let num_clauses = (cli.ratio * cli.vars as f64).round() as usize;
    let mut rng = StdRng::seed_from_u64(cli.seed);

    let clauses = generate_3sat(cli.vars, num_clauses, &mut rng);

    let out: Box<dyn Write> = if let Some(ref path) = cli.output {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).expect("Failed to create output directory");
            }
        }
        Box::new(fs::File::create(path).expect("Failed to create output file"))
    } else {
        Box::new(io::stdout().lock())
    };
    let mut out = io::BufWriter::new(out);

    writeln!(out, "c Random 3-SAT instance at phase transition").unwrap();
    writeln!(out, "c vars={} clauses={} ratio={:.3} seed={}",
             cli.vars, num_clauses, cli.ratio, cli.seed).unwrap();
    writeln!(out, "p cnf {} {}", cli.vars, num_clauses).unwrap();

    for clause in &clauses {
        writeln!(out, "{} {} {} 0", clause[0], clause[1], clause[2]).unwrap();
    }

    out.flush().unwrap();

    eprintln!(
        "Generated: {} vars, {} clauses (ratio={:.3}, seed={})",
        cli.vars, num_clauses, cli.ratio, cli.seed,
    );
}
