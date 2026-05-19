# Solver 11 Discriminating Benchmark Set

This directory is the fixed Section 0 benchmark set for solver 11. It is intentionally small and stable: use it for go/no-go checks while individual tasks are still using narrower generated iteration sets.

The authoritative metadata is `MANIFEST.csv`. Each row records the logical instance name, committed symlink path, decompressed CNF SHA-256, compressed-file SHA-256, expected status, expected-status source, root-cause tag, external Kissat table time, local Kissat time placeholder, solver 10 reference time, and notes about exact or closest available filenames.

Rules:

- Keep `selection_version` stable unless this set is deliberately revised.
- Keep symlinks in sync with manifest rows; every `.cnf*` symlink here must have exactly one manifest row.
- Treat `kissat_reference_external_time` as informational. Fill `kissat_reference_local_time` only from `tools/run_reference_baseline.sh` results on the current benchmark host.
- If a closest available filename is used, explain the difference in `notes`.
- Validate after changes with `python3 tools/select_iter_bench.py --check-manifest benchmarks/discriminating/MANIFEST.csv`.

Recommended iteration command:

```bash
bash tools/bench.sh -t 300 -d benchmarks/discriminating solver/11-kissat-port
```

Use `-t 600` for end-of-phase confirmation when a feature is a promotion candidate.
