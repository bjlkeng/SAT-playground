# Solver 11 Iteration Benchmark Sets

These sets are generated from `benchmarks/discriminating/MANIFEST.csv` and smoke fixtures by:

```bash
python3 tools/select_iter_bench.py --write
```

Set intent:

- `smoke-plus`: all hand-written smoke tests, including model and proof-output cases.
- `search-core`: SAT-heavy search, phase, restart, decision, and learned-clause quality gaps.
- `preprocess-core`: preprocessing throughput, occurrence-list, BVE/gate, and residual-formula pressure.
- `regression-guards`: small and medium cases that should run alongside most behavior changes.
- `stress`: solver10 timeout or proof-heavy cases, useful before milestone promotion.
- `holdout`: promotion-only cases; do not tune per-task changes on this set.
- `killer-tests`: one historical bug-class slot per common solver failure mode.

`baseline.csv` is the machine-readable selection contract consumed by comparison tooling. `FLAKY.csv` is intentionally present even when empty so quarantines are explicit and can be excluded from promotion evidence without disappearing from stress reports.

Validate with:

```bash
python3 tools/select_iter_bench.py --dry-run
```
