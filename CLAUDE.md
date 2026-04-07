# CLAUDE.md — SAT-playground Development Guide

## Project Overview

This repo builds Boolean SAT solvers iteratively in Rust, one directory per iteration, all conforming to the SAT Competition 2025 interface. Each iteration adds a technique on top of the previous one.

## Build & Run

```bash
# Build any iteration
cd NN-name && bash build.sh        # runs: cargo build --release

# Run on a CNF instance
bash run.sh path/to/instance.cnf /tmp/proof_dir

# Run benchmarks
bash tools/bench.sh ./NN-name/run.sh benchmarks/*.cnf
```

## Solver Interface Contract (SAT Competition 2025)

Every iteration MUST provide `build.sh` and `run.sh` at its top level:

- **`build.sh`**: No arguments. Builds the solver binary.
- **`run.sh <cnf_path> <output_dir>`**: Runs the solver. Prints to stdout. Writes `proof.out` to `<output_dir>` when UNSAT.

### Required stdout format

```
s SATISFIABLE
v 1 -2 3 0
```

or

```
s UNSATISFIABLE
```

or

```
s UNKNOWN
```

Rules:
- Exactly one `s` line per run
- `v` lines only when SAT — space-separated literals, terminated by `0`, max 4096 chars/line
- `c` comment lines are allowed anywhere
- Partial assignments are fine as long as every clause is satisfied

### UNSAT proofs

Write DRAT proof to `<output_dir>/proof.out`. This is required from iteration 07 onward.

## Code Conventions

- **Language:** Rust (each iteration is its own Cargo project)
- **Binary name:** `sat-solver` (consistent across iterations for tooling)
- **No external SAT solver dependencies** — the point is to build from scratch
- **Allowed crates:** standard utility crates (clap, anyhow, etc.) are fine; no SAT/SMT libraries
- **Each iteration directory is self-contained** — copy-and-modify from the previous iteration, don't use workspace dependencies between iterations
- **Test with small hand-crafted CNF files first**, then graduate to competition benchmarks

## Iteration Workflow

When creating a new iteration:

1. Copy the previous iteration directory: `cp -r NN-prev/ MM-name/`
2. Update `Cargo.toml` package name
3. Implement the new technique
4. Add unit tests for the new feature
5. Run against benchmarks and record results in the iteration's `README.md`
6. Ensure `build.sh` and `run.sh` still work

## Testing

```bash
# Unit tests within an iteration
cd NN-name && cargo test

# Quick smoke test with a trivial SAT instance
echo "p cnf 2 2\n1 2 0\n-1 2 0" > /tmp/test.cnf
bash run.sh /tmp/test.cnf /tmp/out
# Should print: s SATISFIABLE and valid v lines

# Quick UNSAT test
echo "p cnf 1 2\n1 0\n-1 0" > /tmp/unsat.cnf
bash run.sh /tmp/unsat.cnf /tmp/out
# Should print: s UNSATISFIABLE
```

## DIMACS CNF Format Reference

```
c optional comment
p cnf <num_vars> <num_clauses>
<lit> <lit> ... 0        ← each line is one clause
```

- Variables: positive integers `1..num_vars`
- Literals: variable or its negation (e.g., `3` or `-3`)
- Clause: list of literals terminated by `0`
- No clause may contain both `x` and `-x`

## Benchmarks

Download from: `https://benchmark-database.de/?track=main_2025&context=cnf`

```bash
cd benchmarks
wget -O track_main_2025.uri "https://benchmark-database.de/?track=main_2025&context=cnf"
wget --content-disposition -i track_main_2025.uri
```

Competition scoring is PAR-2: sum of runtimes for solved instances + 2 × 5000s for each unsolved instance. Lower is better.

## Key SAT Competition 2025 Facts

- **Main Track:** 5000s timeout, 30 GB RAM, 8-core Xeon, PAR-2 scoring
- **Winner:** kissat-sc2024 (PAR-2: 2788, 306/400 solved)
- **Proof formats:** DRAT → LRAT (via drat-trim) → cake_lpr; or DRAT → GRAT (via gratgen) → gratchk; or VeriPB → cake_pb_cnf
- **Benchmark source:** https://benchmark-database.de/?track=main_2025&context=cnf

## Common Pitfalls

- Forgetting the trailing `0` on `v` lines
- Printing `v` lines when the result is UNSAT
- Not handling empty clauses (immediately UNSAT)
- Not handling unit clauses at the top level
- Off-by-one on variable indexing (DIMACS is 1-based)
- Exceeding 4096 characters on a single `v` line
