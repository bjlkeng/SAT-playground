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
- gated backward subsumption / BSR for small, dense formulas where the extra sweep pays off
- 64-bit clause abstraction prefiltering for preprocessing subsumption checks
- dynamic elimination heap on the gated full-BSR path
- resolvent insertion through a preprocessing original-clause path
- DRAT logging for preprocessing-generated resolvents/units
- MiniSat-style elimination stack entries and SAT model extension
- SAT output from a complete model snapshot instead of the mutable live assignment vector
- one-shot cleanup after preprocessing: drop occurrence metadata, rebuild branch heap, and force GC

Still incomplete:

- full MiniSat-style BSR as an unconditional default; it regresses broad SAT-heavy cases here
- asymmetric branching clause strengthening
- `use_rcheck` implied-clause checks
- parse-time canonical original-clause insertion for every input clause
- a fully faithful `SimpSolver::eliminate()` work loop for all formula shapes

So this is now a working BVE preprocessing iteration, but it is not yet a complete MiniSat `simp`
port.

## Validation

Run on 2026-05-08:

```bash
cargo test
bash tools/smoke_test.sh solver/10-bve-preprocess
```

Results:

- `cargo test` in `solver/10-bve-preprocess`: 45 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-08-16-08-17`

## MiniSat-Simp Five-Instance Benchmark

Command:

```bash
bash tools/bench.sh -t 600 -m 16384 -d benchmarks/profiling/minisat-simp-five solver/10-bve-preprocess
```

Result log:

- `log/bench-10-bve-preprocess-2026-05-08-15-56-37/results.csv`

Summary:

- 5 instances
- 5 solved: 3 SAT, 2 UNSAT
- 0 timeouts
- PAR-2: `540.550`

Comparison against matching harness runs:

| Solver | Solved | SAT | UNSAT | Timeouts | PAR-2 | Results |
|---|---:|---:|---:|---:|---:|---|
| `09-root-simp-opts` | 3/5 | 2 | 1 | 2 | `3195.921` | `log/bench-09-root-simp-opts-2026-05-08-09-58-03/results.csv` |
| `10-bve-preprocess` before gated BSR | 4/5 | 3 | 1 | 1 | `1532.975` | `log/bench-10-bve-preprocess-2026-05-08-13-08-41/results.csv` |
| `10-bve-preprocess` gated BSR | 5/5 | 3 | 2 | 0 | `540.550` | `log/bench-10-bve-preprocess-2026-05-08-15-56-37/results.csv` |
| `minisat` | 5/5 | 3 | 2 | 0 | `453.343` | `log/bench-minisat-2026-05-08-09-58-03/results.csv` |

Per-instance notes for the gated-BSR run:

- `sudoku-N30-12`: `184.240s`, roughly equal to previous solver `10` and much faster than `09`
- `SC25_Timetable...`: `89.200s`, still far slower than MiniSat's `18.545s`
- `REGRandom-K4...`: now solves UNSAT in `205.600s`; previous solver `10` timed out at `600s`
- `mp1-Nb7T46`: `43.110s`, still faster than MiniSat's `75.054s`
- `Kakuro...`: `18.400s`, still faster than MiniSat's `80.111s`

The remaining gap to MiniSat is now mostly preprocessing speed on K4 and CDCL/search behavior on the
Timetable SAT instance. A direct MiniSat K4 run reported `39.65s` simplification and `61.26s` total
CPU time; gated solver `10` reaches the same K4 residual formula but spent about `117.05s` in
preprocessing during trace runs.
