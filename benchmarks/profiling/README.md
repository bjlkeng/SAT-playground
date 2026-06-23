# benchmarks/profiling

Small, fast-iteration suite for solver development. Each instance is sized so
that a working solver finishes in well under 5 minutes; the full suite runs in
roughly 15–25 minutes on this repo's reference hardware (AMD Ryzen 5 5600,
solver 10 wall-time baseline ≈ 12 minutes).

`tools/bench.sh` automatically uses a 300-second per-instance timeout when the
benchmark directory resolves to this directory.

## Provenance (2026-05-23 refresh)

Ten instances chosen from the SAT Competition 2025 main-track medium subset
(`benchmarks/sat-comp-2025-medium/`). Selection criteria:

* Each instance was solved by both `kissat-latest` and `solver/10-bve-subsume`
  in under 300 s wall-clock on the reference hardware (records:
  `log/bench-kissat-latest-2026-04-11-22-21-01/results.csv` and
  `log/bench-10-bve-preprocess-2026-05-18-16-04-01/results.csv`).
* Mix of SAT (6) and UNSAT (4).
* Coverage across problem families: scheduling, number theory, set cover,
  hardware verification, combinatorial puzzles, planning, pipeline verification,
  classical sudoku / Kakuro, and random 3-SAT.
* Runtime range spans roughly 8 s to 195 s on solver 10 so a feature that
  shifts both fast and slow ends is easy to spot.

| File | Result | Family | Solver-10 baseline |
|---|---|---|---|
| `9af7646f…-brocard_problem_large` | UNSAT | number theory | 7.9 s |
| `663bb565…-SCPC-500-13` | UNSAT | set cover | 13.2 s |
| `3746303c…-6s299b685_Iter30` | SAT | HWMCC hardware | 15.3 s |
| `ed6d842f…-battleship-16-31-sat` | SAT | combinatorial puzzle | 22.2 s |
| `557d7d4d…-mp1-Nb7T46` | SAT | mp1 family | 40.4 s |
| `46355da7…-REGRandom-K4-L1-Seed40.sanitized` | UNSAT | random 3-SAT | 54.2 s |
| `6832fe90…-velev-pipe-sat-1.0-b7` | SAT | Velev pipeline verification | 61.2 s |
| `fab2022d…-case9` | SAT | planning | 121.4 s |
| `0aa22564…-sudoku-N30-12` | UNSAT | sudoku | 171.0 s |
| `5e933a62…-Kakuro-easy-112-ext.xml.hg_7` | SAT | Kakuro | 194.5 s |

File names preserve the medium-suite hash prefix so the originals can be
re-located in `benchmarks/sat-comp-2025-medium/` if needed.

## Subdirectories

* `legacy/` — the previous 6-instance suite (3 feistel, 3 random) preserved
  here so `tools/ci_solver11_overhead.py` and
  `tools/run_solver11_thin_slice.sh` keep working. The originals also live in
  `benchmarks/crypto/` and `benchmarks/random-3sat/`.
* `minisat-simp-five/` — independent 5-instance MiniSat-simp parity fixture
  (used by solver 10 preprocessing work). Untouched by the 2026-05-23 refresh.

## Running

```bash
bash tools/bench.sh -d benchmarks/profiling solver/NN-name
```

`tools/bench.sh` decompresses `.cnf.xz` to a temp directory automatically.
