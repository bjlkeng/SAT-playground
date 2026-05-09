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
- MiniSat-style backward subsumption / BSR enabled by default, with `SAT_FULL_BSR=off` retained as
  a diagnostic override
- 64-bit clause abstraction prefiltering for preprocessing subsumption checks
- in-place original-clause strengthening during BSR
- a persistent preprocessing loop over touched variables, root assignments, queued subsumption
  clauses, and a dynamic elimination heap
- resolvent insertion through a preprocessing original-clause path, with generated clauses queued
  for immediate subsumption work
- DRAT logging for preprocessing-generated resolvents/units
- MiniSat-style elimination stack entries and SAT model extension
- SAT output from a complete model snapshot instead of the mutable live assignment vector
- one-shot cleanup after preprocessing: drop occurrence metadata, rebuild branch heap, and force GC

Still incomplete:

- asymmetric branching clause strengthening
- `use_rcheck` implied-clause checks
- parse-time canonical original-clause insertion for every input clause
- MiniSat's CDCL implementation details; this solver still keeps the repo's `09` search core

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

Rerun after the large-formula BSR gate on 2026-05-08:

- `cargo test` in `solver/10-bve-preprocess`: 45 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-08-20-35-04`

Rerun after the MiniSat-style persistent preprocessing loop on 2026-05-08:

- `cargo test` in `solver/10-bve-preprocess`: 45 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-08-23-35-41`

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

## Fresh MiniSat-Gap Debugging Notes

The 2026-05-08 fresh five-instance rerun showed solver `10` solving 3/5 while MiniSat solved all 5.
The accepted change from the follow-up debugging loop is a larger-formula full-BSR gate:

- `9af7...brocard_problem_large`: baseline solver `10` solved UNSAT in `163.160s`; with full BSR
  enabled by the new large-formula gate it solved in about `42.3s` (`34.9s` preprocessing +
  `7.4s` search).
- MiniSat's dumped residual for brocard had `4,086,123` clauses and `13,124,041` literals. The new
  large-formula BSR path produces essentially the same residual before search.
- Running solver `10` directly on MiniSat's brocard residual solved in `9.7s`, confirming the
  brocard gap was mostly preprocessing residual quality rather than CDCL search.

Rejected or incomplete hypotheses from that loop:

- Initial negative branching phase did not solve `bp4`, brocard, or Timetable within the tested
  bounds.
- MiniSat-style variable-order activity tie-breaking plus negative phase was worse on brocard and
  Timetable than the existing occurrence tie.
- MiniSat-style backtrack-only phase saving plus negative phase did not fix the SAT-side gaps.
- Forced full BSR matches MiniSat-like residuals on `bp4` and Timetable, but it does not make solver
  `10` solve those SAT targets quickly; running solver `10` on MiniSat's own residual formulas still
  timed out under the tested `90s` bound. Those remain CDCL/search-core gaps.

## MiniSat-Loop Refactor Follow-up

The next refactor implemented the remaining MiniSat `simp` work-loop differences:

- full BSR now runs by default instead of using the earlier formula-size gate
- preprocessing now loops over touched variables, root assignments, queued subsumption clauses, and
  elimination-heap variables until all work is drained
- BSR strengthens original clauses in place
- variable occurrence-cost updates feed a dynamic elimination heap broadly after clause
  add/delete/strengthen events
- generated resolvents are queued immediately for subsumption, and touched variables continuously
  enqueue their occurrence clauses

Direct `600s` checks on the fresh MiniSat-gap instances after this refactor:

| Instance | Result | Notes |
|---|---:|---|
| `849950...circuit_48in64out...` | SAT `208.1s` | Slower than the previous gated path (`49.4s`). |
| `98e8...bp4_TCO_CSO_IXA_LP_ZR` | TIMEOUT | Preprocessing `7.0s`; search still did not find SAT. |
| `9af7...brocard_problem_large` | UNSAT `~15.3s` | Improved from `42.3s` after the large-formula gate and `163.2s` before it. |
| `f17d...SC25_Timetable...` | TIMEOUT | Preprocessing `5.3s`; search still did not find SAT. |
| `f25a...1-TC-256-K-63` | TIMEOUT | With `SAT_FULL_BSR=off`, the same code still solves in `375.4s`; full MiniSat-like preprocessing changes the search trajectory. |

MiniSat enters search on `1-TC` with the same residual size (`422669` clauses / `930421` literals)
and solves in `162.8s`, so the remaining `1-TC`, `bp4`, and Timetable gap is no longer explained
by these `simp` work-loop differences alone.

Matching harness rerun:

- `10-bve-preprocess`: `log/bench-10-bve-preprocess-2026-05-08-22-51-56/results.csv`
- solved `2/5` (`1 SAT`, `1 UNSAT`, `3` timeouts)
- PAR-2: `3823.879`

Compared with the previous gated-BSR run on this same set, the refactor improves Brocard
dramatically (`163.160s -> 16.277s`) but regresses the overall benchmark (`3/5`, PAR-2
`2986.963` -> `2/5`, PAR-2 `3823.879`) because circuit slows down and `1-TC` becomes a timeout.
