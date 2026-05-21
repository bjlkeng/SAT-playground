# Solver 11 Profile Benchmark: 300s Default

Date: 2026-05-21

Command:

```bash
bash tools/bench.sh -m 16384 -d benchmarks/profiling \
  --log-dir log/profile-default-300-solver11-2026-05-21 \
  solver/11-kissat-port
```

This intentionally omits `-t` to verify that `tools/bench.sh` now applies the
`benchmarks/profiling` default timeout of 300 seconds per instance.

Result:

- Timeout used by harness: 300s
- Instances: 11
- Solved: 11 (7 SAT, 4 UNSAT)
- Unsolved: 0
- PAR-2: 627.579
- Results CSV: `log/profile-default-300-solver11-2026-05-21/results.csv`

Comparison point:

- Previous 120s profile artifact: `log/1.12a/profile-after/results.csv`
- Previous solved count: 9/11
- New solved count: 11/11
- Newly solved under 300s:
  - `0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12`: UNSAT in 183.412s
  - `5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7`: SAT in 209.427s
- `compare_bench.py` verdict: PASS
- Correctness failures: none

Interpretation:

The 5-minute profiling default changes the benchmark from a short smoke-like profile into a more
useful solver-quality signal for the current solver. The two prior 120s timeout rows were not
hopeless; both complete inside 300s. PAR-2 values from 120s and 300s runs should not be compared as
pure speed regressions because the timeout policy changed, but the solved-count improvement is real.
