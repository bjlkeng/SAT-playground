# CLAUDE.md — SAT-playground Development Guide

## Project Overview

This repo builds Boolean SAT solvers iteratively in Rust, one directory per iteration, all conforming to the SAT Competition 2025 interface. Each iteration adds a technique on top of the previous one.

## Build & Run

```bash
# Build any iteration
cd solver/NN-name && bash build.sh        # runs: cargo build --release

# Run on a CNF instance
bash run.sh path/to/instance.cnf /tmp/proof_dir

# Run benchmarks
bash tools/bench.sh solver/NN-name
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

Write DRAT proof to `<output_dir>/proof.out`. This is required from every iteration.

## Code Conventions

- **Language:** Rust (each iteration is its own Cargo project)
- **Binary name:** `sat-solver` (consistent across iterations for tooling)
- **No external SAT solver dependencies** — the point is to build from scratch
- **Allowed crates:** standard utility crates (clap, anyhow, etc.) are fine; no SAT/SMT libraries
- **Each iteration directory is self-contained** — copy-and-modify from the previous iteration, don't use workspace dependencies between iterations
- **Test with small hand-crafted CNF files first**, then graduate to competition benchmarks

## Development Rules

- **Run smoke tests after every change** to a solver: `bash tools/smoke_test.sh solver/NN-name`
- **Only commit solver changes that pass the smoke test** (all 8 tests green). If a test fails, fix the solver before committing.
- **Never modify `tools/smoke_test.sh`** unless the user explicitly asks for changes to it.
- **Always commit and push** when the user asks — don't skip the push step.

## Iteration Workflow

When creating a new iteration:

1. Copy the previous iteration directory: `cp -r solver/NN-prev/ solver/MM-name/`
2. Update `Cargo.toml` package name
3. Implement the new technique
4. Add unit tests for the new feature
5. Run `bash tools/smoke_test.sh solver/MM-name` — all 8 tests must pass
6. Run against benchmarks and record results in the iteration's `README.md`
7. Ensure `build.sh` and `run.sh` still work

## Testing

```bash
# Unit tests within an iteration
cd solver/NN-name && cargo test

# Smoke test — runs all 8 test instances (4 SAT + 4 UNSAT)
bash tools/smoke_test.sh solver/NN-name
```

### Smoke Test Suite

Located in `tests/cnf/`, these are small hand-crafted instances that run in under a second:

**SAT instances** (`tests/cnf/sat/`):
- `unit.cnf` — single unit clause (trivial)
- `two_clause.cnf` — 2 vars, 2 clauses
- `three_sat.cnf` — 5 vars, 6 clauses (small 3-SAT)
- `all_positive.cnf` — 3 vars, all positive literals

**UNSAT instances** (`tests/cnf/unsat/`):
- `contradiction.cnf` — x AND NOT x
- `empty_clause.cnf` — contains an empty clause
- `pigeonhole_3_2.cnf` — 3 pigeons, 2 holes (classic)
- `chain_unsat.cnf` — implication chain forcing contradiction

The smoke test script (`tools/smoke_test.sh`) builds the solver, runs all instances, checks the `s` line, and verifies SAT assignments satisfy the formula.

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
