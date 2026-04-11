use clap::Parser;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "gen-crypto-bench", about = "Generate XOR-heavy crypto CNF benchmarks")]
struct Cli {
    /// Block size in bits (must be even)
    #[arg(long, default_value = "16")]
    block_size: usize,

    /// Key size in bits (per-round key)
    #[arg(long, default_value = "8")]
    key_size: usize,

    /// Number of Feistel rounds
    #[arg(long, default_value = "4")]
    rounds: usize,

    /// Random seed for reproducibility
    #[arg(long, default_value = "42")]
    seed: u64,

    /// Output file (default: stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Circuit builder — Tseitin encoding
// ---------------------------------------------------------------------------

struct Circuit {
    next_var: i32,
    clauses: Vec<Vec<i32>>,
}

impl Circuit {
    fn new() -> Self {
        Circuit {
            next_var: 1,
            clauses: Vec::new(),
        }
    }

    /// Allocate n fresh variables, return them as a Vec.
    fn alloc_vars(&mut self, n: usize) -> Vec<i32> {
        let vars: Vec<i32> = (self.next_var..self.next_var + n as i32).collect();
        self.next_var += n as i32;
        vars
    }

    /// Assert a variable to a constant value (unit clause).
    fn assert_const(&mut self, var: i32, value: bool) {
        if value {
            self.clauses.push(vec![var]);
        } else {
            self.clauses.push(vec![-var]);
        }
    }

    /// z = x AND y (3 clauses)
    fn and_gate(&mut self, x: i32, y: i32) -> i32 {
        let z = self.alloc_vars(1)[0];
        // z -> x: (-z, x)
        // z -> y: (-z, y)
        // x & y -> z: (-x, -y, z)
        self.clauses.push(vec![-z, x]);
        self.clauses.push(vec![-z, y]);
        self.clauses.push(vec![-x, -y, z]);
        z
    }

    /// z = x XOR y (4 clauses)
    fn xor_gate(&mut self, x: i32, y: i32) -> i32 {
        let z = self.alloc_vars(1)[0];
        // Truth table encoding:
        // (-x, -y, -z)  — all false
        // ( x,  y, -z)  — both true → z false
        // ( x, -y,  z)  — x true, y false → z true
        // (-x,  y,  z)  — x false, y true → z true
        self.clauses.push(vec![-x, -y, -z]);
        self.clauses.push(vec![x, y, -z]);
        self.clauses.push(vec![x, -y, z]);
        self.clauses.push(vec![-x, y, z]);
        z
    }

    fn num_vars(&self) -> i32 {
        self.next_var - 1
    }

    fn num_clauses(&self) -> usize {
        self.clauses.len()
    }
}

// ---------------------------------------------------------------------------
// Feistel cipher (plain Rust, for computing expected ciphertext)
// ---------------------------------------------------------------------------

/// Rotate left within `width` bits.
fn rotate_left(val: u64, shift: usize, width: usize) -> u64 {
    let mask = (1u64 << width) - 1;
    ((val << shift) | (val >> (width - shift))) & mask
}

/// Round function F(r, k) = ((r AND k) XOR (r <<< 3)) XOR k
/// Both r and k are `half_size` bits wide, but k may be shorter (key_size).
/// We truncate/extend k to match half_size by repeating.
fn round_fn(r: u64, k: u64, half_size: usize) -> u64 {
    let mask = (1u64 << half_size) - 1;
    let k = k & mask;
    let r_and_k = r & k;
    let r_rot = rotate_left(r, 3, half_size);
    ((r_and_k ^ r_rot) ^ k) & mask
}

/// Encrypt a block using the Feistel cipher.
fn feistel_encrypt(plaintext: u64, key: u64, block_size: usize, key_size: usize, rounds: usize) -> u64 {
    let half = block_size / 2;
    let mask = (1u64 << half) - 1;
    let key_mask = (1u64 << key_size) - 1;

    let mut left = (plaintext >> half) & mask;
    let mut right = plaintext & mask;

    for round in 0..rounds {
        // Derive per-round key by rotating
        let round_key = rotate_left(key & key_mask, round * 3 % key_size, key_size) & key_mask;
        let f = round_fn(right, round_key, half);
        let new_right = left ^ f;
        left = right;
        right = new_right & mask;
    }

    (left << half) | right
}

// ---------------------------------------------------------------------------
// Feistel cipher → Circuit encoding
// ---------------------------------------------------------------------------

/// Encode the Feistel cipher as a circuit, returning (plaintext_vars, key_vars, ciphertext_vars).
fn encode_feistel(
    circuit: &mut Circuit,
    block_size: usize,
    key_size: usize,
    rounds: usize,
) -> (Vec<i32>, Vec<i32>, Vec<i32>) {
    let half = block_size / 2;

    // Allocate input variables
    let plaintext = circuit.alloc_vars(block_size);
    let key = circuit.alloc_vars(key_size);

    // Rust computes: left = upper bits (half..block), right = lower bits (0..half)
    // Variables are indexed by bit position, so plaintext[i] = bit i
    let mut left: Vec<i32> = plaintext[half..].to_vec();   // upper bits
    let mut right: Vec<i32> = plaintext[..half].to_vec();   // lower bits

    for round in 0..rounds {
        // Derive per-round key by bit rotation
        let shift = (round * 3) % key_size;
        // rotate_left: bit i of result = bit (i - shift) mod width of original
        let round_key: Vec<i32> = (0..key_size)
            .map(|i| key[(i + key_size - shift) % key_size])
            .collect();

        // Compute F(right, round_key):
        // Step 1: r_and_k = right[i] AND round_key[i] (for each bit, pad key if needed)
        let mut r_and_k = Vec::with_capacity(half);
        for i in 0..half {
            let k_bit = round_key[i % key_size];
            let a = circuit.and_gate(right[i], k_bit);
            r_and_k.push(a);
        }

        // Step 2: r_rot = right rotated left by 3
        // rotate_left by 3: bit i of result = bit (i - 3) mod half of original
        let r_rot: Vec<i32> = (0..half).map(|i| right[(i + half - 3) % half]).collect();

        // Step 3: tmp = r_and_k XOR r_rot
        let mut tmp = Vec::with_capacity(half);
        for i in 0..half {
            let x = circuit.xor_gate(r_and_k[i], r_rot[i]);
            tmp.push(x);
        }

        // Step 4: f = tmp XOR round_key
        let mut f = Vec::with_capacity(half);
        for i in 0..half {
            let k_bit = round_key[i % key_size];
            let x = circuit.xor_gate(tmp[i], k_bit);
            f.push(x);
        }

        // new_right = left XOR f
        let mut new_right = Vec::with_capacity(half);
        for i in 0..half {
            let x = circuit.xor_gate(left[i], f[i]);
            new_right.push(x);
        }

        // Swap
        left = right;
        right = new_right;
    }

    // Final ciphertext: bit i maps to right[i] for i < half, left[i-half] for i >= half
    // This matches Rust's (left << half) | right
    let mut ciphertext = right;
    ciphertext.extend(left);

    (plaintext, key, ciphertext)
}

// ---------------------------------------------------------------------------
// Bit helpers
// ---------------------------------------------------------------------------

fn get_bit(val: u64, bit: usize) -> bool {
    (val >> bit) & 1 == 1
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    assert!(cli.block_size % 2 == 0, "block_size must be even");
    assert!(cli.block_size >= 4, "block_size must be >= 4");
    assert!(cli.key_size >= 2, "key_size must be >= 2");
    assert!(cli.rounds >= 1, "rounds must be >= 1");

    let mut rng = StdRng::seed_from_u64(cli.seed);
    let block_mask = (1u64 << cli.block_size) - 1;
    let key_mask = (1u64 << cli.key_size) - 1;

    let plaintext: u64 = rng.gen::<u64>() & block_mask;
    let key: u64 = rng.gen::<u64>() & key_mask;
    let ciphertext = feistel_encrypt(plaintext, key, cli.block_size, cli.key_size, cli.rounds);

    // Build circuit
    let mut circuit = Circuit::new();
    let (pt_vars, _key_vars, ct_vars) =
        encode_feistel(&mut circuit, cli.block_size, cli.key_size, cli.rounds);

    // Assert plaintext bits
    for (i, &var) in pt_vars.iter().enumerate() {
        circuit.assert_const(var, get_bit(plaintext, i));
    }

    // Assert ciphertext bits
    for (i, &var) in ct_vars.iter().enumerate() {
        circuit.assert_const(var, get_bit(ciphertext, i));
    }

    // Output DIMACS CNF
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

    writeln!(out, "c Feistel cipher key recovery — XOR-heavy crypto benchmark").unwrap();
    writeln!(out, "c block_size={} key_size={} rounds={} seed={}",
             cli.block_size, cli.key_size, cli.rounds, cli.seed).unwrap();
    writeln!(out, "c plaintext=0x{:x} key=0x{:x} ciphertext=0x{:x}",
             plaintext, key, ciphertext).unwrap();
    writeln!(out, "c vars={} clauses={}", circuit.num_vars(), circuit.num_clauses()).unwrap();
    writeln!(out, "c Expected: SATISFIABLE (key recovery)").unwrap();
    writeln!(out, "p cnf {} {}", circuit.num_vars(), circuit.num_clauses()).unwrap();

    for clause in &circuit.clauses {
        let mut line = String::new();
        for &lit in clause {
            line.push_str(&lit.to_string());
            line.push(' ');
        }
        line.push('0');
        writeln!(out, "{}", line).unwrap();
    }

    out.flush().unwrap();

    // Print summary to stderr
    eprintln!(
        "Generated: {} vars, {} clauses (block={}, key={}, rounds={}, seed={})",
        circuit.num_vars(),
        circuit.num_clauses(),
        cli.block_size,
        cli.key_size,
        cli.rounds,
        cli.seed,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feistel_roundtrip() {
        // Encrypt then verify a known computation
        let pt: u64 = 0xABCD;
        let key: u64 = 0x42;
        let ct = feistel_encrypt(pt, key, 16, 8, 4);
        // Same inputs should produce same output (deterministic)
        assert_eq!(ct, feistel_encrypt(pt, key, 16, 8, 4));
        // Different key should (almost certainly) produce different output
        assert_ne!(ct, feistel_encrypt(pt, 0x43, 16, 8, 4));
    }

    #[test]
    fn test_circuit_xor_gate_clause_count() {
        let mut c = Circuit::new();
        let vars = c.alloc_vars(2);
        c.xor_gate(vars[0], vars[1]);
        assert_eq!(c.num_clauses(), 4);
    }

    #[test]
    fn test_circuit_and_gate_clause_count() {
        let mut c = Circuit::new();
        let vars = c.alloc_vars(2);
        c.and_gate(vars[0], vars[1]);
        assert_eq!(c.num_clauses(), 3);
    }

    #[test]
    fn test_encode_produces_clauses() {
        let mut c = Circuit::new();
        let (pt, key, ct) = encode_feistel(&mut c, 16, 8, 2);
        assert_eq!(pt.len(), 16);
        assert_eq!(key.len(), 8);
        assert_eq!(ct.len(), 16);
        assert!(c.num_clauses() > 0);
        assert!(c.num_vars() > 24); // at least pt + key + internal gates
    }

    #[test]
    fn test_rotate_left() {
        assert_eq!(rotate_left(0b1100, 1, 4), 0b1001);
        assert_eq!(rotate_left(0b0001, 3, 4), 0b1000);
    }
}
