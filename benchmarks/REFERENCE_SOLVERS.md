# Reference Solvers

Solver 11 performance comparisons use local, SHA-pinned reference binaries when available. External published Kissat times in `benchmarks/discriminating/MANIFEST.csv` are useful context, but local reference runs are the authority for "Kissat-class" claims on this machine.

| solver_name | version_string | git_sha_or_release | build_command | run_command_template | proof_policy | timeout | memory_limit | date_utc | machine_id_or_environment_block |
|---|---|---|---|---|---|---:|---:|---|---|
| kissat-latest | `benchmarks/reference-solvers/kissat-latest/VERSION` | recorded by `tools/build_reference_solvers.sh` | `tools/build_reference_solvers.sh kissat-latest` | `benchmarks/reference-solvers/kissat-latest/build/kissat <cnf>` | status-only reference; no proof required for solver-11 certification | 1800s default | 16384 MB default | generated at build/run time | recorded under `log/reference-baselines/kissat-latest/` |
| kissat-sc2024 | `benchmarks/reference-solvers/kissat-sc2024/VERSION` | recorded by `tools/build_reference_solvers.sh` | `tools/build_reference_solvers.sh kissat-sc2024` | `benchmarks/reference-solvers/kissat-sc2024/build/kissat <cnf>` | status-only reference; no proof required for solver-11 certification | 1800s default | 16384 MB default | generated at build/run time | recorded under `log/reference-baselines/kissat-sc2024/` |
| minisat | vendored MiniSat README/revision | recorded by `tools/build_reference_solvers.sh` | `tools/build_reference_solvers.sh minisat` | `benchmarks/reference-solvers/minisat/build/release/bin/minisat <cnf>` | status-only reference; no proof required for solver-11 certification | 1800s default | 16384 MB default | generated at build/run time | recorded under `log/reference-baselines/minisat/` |
| cadical | 2.1.3 (`benchmarks/reference-solvers/cadical/VERSION`) | `rel-2.1.3`, commit `f13d74439a5b5c963ac5b02d05ce93a8098018b8`; the script fetches it and checks the commit | `tools/build_reference_solvers.sh cadical` | `benchmarks/reference-solvers/cadical/build/cadical <cnf>` | answers only: a second, independent solver for the assumptions fuzz harness (`tools/assume_fuzz.py`), never a speed reference | none (small fuzz formulas) | none | 2026-11-15 | recorded under `log/reference-baselines/cadical/`; the pin is the commit (the build stamps its date into the binary, so the binary's sha256 changes with every build) |

Reference baseline rules:

- Build records live under `log/reference-baselines/<solver>/` and include `commit.txt`, `binary.sha256`, `build-command.txt`, and `environment.txt`.
- Run records live under `log/reference-baselines/<solver>/<suite>/` and include copied `results.csv` and `summary.log` from `tools/bench_reference.sh`; the default suite list includes a small `calibration` run over `tests/cnf/sat` before profiling/discriminating/full benchmark runs.
- Refresh local baselines only when the benchmark machine, toolchain, vendored reference source, or benchmark selection changes.
- If a reference build cannot be produced, keep using the external table only as low-confidence evidence and say so in any promotion note.
- Solver 11 proof/model correctness is never certified by reference solver agreement alone; use `tools/validate_solver_result.py` and proof/model checks for that.
