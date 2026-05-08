# 10-bve-preprocess

MiniSat `simp`-style bounded variable elimination on top of `09-root-simp-opts`.

This iteration keeps the `09` CDCL core and adds a one-shot preprocessing phase before search. The
preprocessor is intentionally isolated in `src/simp.rs` so it can keep evolving toward the full
MiniSat `SimpSolver` design described in
[MINISAT_SIMP_PORT.md](/home/bojji/code/SAT-playground/solver/10-bve-preprocess/MINISAT_SIMP_PORT.md).

## Current State

What is present:

- original-clause occurrence lists and literal occurrence counts during preprocessing
- a separate decision-variable flag so eliminated variables do not re-enter the branch heap
- bounded variable elimination with MiniSat-style `grow = 0` and `clause_lim = 20`
- resolvent insertion through a preprocessing original-clause path
- DRAT logging for preprocessing-generated resolvents/units
- MiniSat-style elimination stack entries and SAT model extension
- SAT output from a complete model snapshot instead of the mutable live assignment vector
- one-shot cleanup after preprocessing: drop occurrence metadata, rebuild branch heap, and force GC

Still not present:

- MiniSat's backward subsumption / backward-subsumption-resolution queue
- asymmetric branching clause strengthening
- `use_rcheck` implied-clause checks
- parse-time canonical original-clause insertion for every input clause
- a fully faithful `SimpSolver::eliminate()` work loop with touched-clause scheduling

So this is now a working BVE preprocessing iteration, but it is not yet a complete MiniSat `simp`
port.

## Validation

Run on 2026-05-08:

```bash
cargo test
bash tools/smoke_test.sh solver/10-bve-preprocess
```

Results:

- `cargo test` in `solver/10-bve-preprocess`: 42 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-08-13-26-40`

## MiniSat-Simp Five-Instance Benchmark

Command:

```bash
bash tools/bench.sh -t 600 -m 16384 -d benchmarks/profiling/minisat-simp-five solver/10-bve-preprocess
```

Result log:

- `log/bench-10-bve-preprocess-2026-05-08-13-08-41/results.csv`

Summary:

- 5 instances
- 4 solved: 3 SAT, 1 UNSAT
- 1 timeout
- PAR-2: `1532.975`

Comparison against matching harness runs:

| Solver | Solved | SAT | UNSAT | Timeouts | PAR-2 | Results |
|---|---:|---:|---:|---:|---:|---|
| `09-root-simp-opts` | 3/5 | 2 | 1 | 2 | `3195.921` | `log/bench-09-root-simp-opts-2026-05-08-09-58-03/results.csv` |
| `10-bve-preprocess` | 4/5 | 3 | 1 | 1 | `1532.975` | `log/bench-10-bve-preprocess-2026-05-08-13-08-41/results.csv` |
| `minisat` | 5/5 | 3 | 2 | 0 | `453.343` | `log/bench-minisat-2026-05-08-09-58-03/results.csv` |

Per-instance notes versus `09`:

- `sudoku-N30-12`: improved from `507.092s` to `181.810s`
- `SC25_Timetable...`: regressed from `80.349s` to `89.000s`
- `REGRandom-K4...`: still timed out
- `mp1-Nb7T46`: improved from timeout to `43.275s`
- `Kakuro...`: improved from `208.480s` to `18.890s`

The remaining gap to MiniSat is concentrated in the random K4 UNSAT instance and the absence of the
full backward-subsumption/asymmetric-strengthening pipeline.
