# Solver 13 — kissat-rs

Faithful Rust reimplementation of kissat 4.0.4 (reference:
`benchmarks/reference-solvers/kissat-latest/`, built `gcc -O3 -DNDEBUG`).

- Plan and acceptance criteria: `plan/solver13-port-plan.md`
- Binding port conventions: `CONVENTIONS.md`
- Faithfulness oracle: `tools/parity.py` — diffs the deterministic `-s`
  statistics counters against the reference binary at fixed `--conflicts`
  limits. A faithful port matches exactly.

Goal: on `benchmarks/sat-comp-2025` (400 instances, 3600 s / 16 GB / 32
cores), solved count and PAR-2 within 2% of kissat 4.0.4 in a fresh paired
run, with all kissat features implemented.

Status: foundation + core CDCL waves complete; inprocessing wave in
progress. Measured results so far (all tier-1 probes, NOT promotion
evidence):

- 2026-08-30 `tools/smoke_test.sh`: 9/9 PASS — valid SAT models,
  drat-trim-verified UNSAT proofs, default options
  (log/2026-08-30-23-04-12).
- 2026-08-30 parity, smoke corpus (9 CNFs): exact 80-counter `-s` match vs
  reference kissat under `--plain --no-lucky` and `--plain`.
- 2026-08-30 parity, `benchmarks/discriminating` (20 xz instances, real
  SAT-comp cells), `--conflicts=10000 --plain --no-lucky`: **18/20 at exact
  80-counter parity** — identical conflicts/decisions/propagations/ticks/
  restarts over 10k conflicts of real search. The other 2 cells hit the
  harness 600 s cap in BOTH binaries (no divergence observed; scratchpad
  disc_parity.log of session 2026-08-30).
- 2026-09-03 sweep-substitute divergence found and fixed: kissat's
  `substitute_connected_clauses` new_size>2 path ends in a `q--` that
  decrements a *shadowed* inner lits cursor, not the outer watch pointer, so
  the reference keeps a stale occurrence of the substituted clause in the old
  literal's list (later garbage-collected via dense propagation). Our port
  had implemented the intended move semantics; now matches the C behavior
  (see PORT NOTE in `src/sweep.rs`). Isolated via SWEEP_DEBUG watch-list
  hash dumps + per-ref tracing on
  `benchmarks/discriminating/*brocard_problem_large.cnf.xz`.
- 2026-09-03 parity, brocard_problem_large **full default-config run to
  completion** (no limits): both `s UNSATISFIABLE`, all 80 `-s` counters
  exact including probing_ticks 100764057 (was +5 drift pre-fix), ~150 s of
  real search with 3 sweeps, full inprocessing.
- 2026-09-03 parity, `benchmarks/discriminating` (20 xz instances),
  **full default config** `--conflicts=10000`: **20/20 at exact 80-counter
  parity** (statuses match; includes 2 SAT and 2 UNSAT full solves within
  the limit). All inprocessing engines active. Command:
  `python3 solver/13-kissat-rs/tools/parity.py --conflicts 10000
  --timeout 900 benchmarks/discriminating/*.xz`.
