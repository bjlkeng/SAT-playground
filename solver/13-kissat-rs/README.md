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

- 2026-09-03 parity, `benchmarks/discriminating` (20 xz instances), **full
  default config `--conflicts=100000`**: **20/20 at exact 80-counter
  parity** — 10x the previous horizon; multiple full solves inside the
  limit (battleship SAT, Kakuro SAT, REGRandom UNSAT, brocard UNSAT).

Performance notes (tier-1, brocard full default runs, quiet-ish host):

- 2026-09-03 wall gap vs reference: ~8.5% slower overall (87.4 v 94.8 s
  totals; search +6%, probe/simplify/vivify/sweep +20-25%, parse 1.19x).
  Earlier `--profile=4` phase ratios (decide 10x, lucky 7x, parse 4x) were
  measurement artifacts of the old `process_time()` reading and parsing
  /proc/self/stat per profile START/STOP; resources.rs now uses libc
  getrusage/gettimeofday exactly like the C. With the honest clock the
  `--profile=4` totals differ by ~3.5% and counters remain exact.
- 2026-09-03 REJECTED: software-prefetch of the next watched clause in
  `propagate_literal` (solver12 bead 5b2.8.1 pattern). Paired simultaneous
  brocard A/B: 99.50 s with prefetch v 96.25 s without (+3.4% regression),
  counters identical. solver13's 2-word interleaved watch layout does not
  benefit; do not re-add without new evidence.
- 2026-09-03 **profiler unlocked** (perf_event_paranoid=1, perf + valgrind
  present) and the propagation gap closed structurally, all measured with
  the same protocol: simultaneous pinned-core brocard full default runs,
  candidate v previous step v reference kissat, 80-counter parity checked on
  every arm (exact throughout). Session start on this protocol: 109.5 s v
  kissat 96.4 s (**+13.7%**).
  1. `#[inline(always)]` on the fast-assign chain (`assign`,
     `fast_binary_assign`, `fast_assign_reference`, `assignment_level`,
     `push_vectors`, `push_blocking_watch`, `delay_watching_large`,
     `watch_large_delayed`). perf showed C's `kissat_search_propagate` as one
     73.7% frame while ours split into `search_propagate` 52.8% +
     out-of-line `assign` 12.8% + `push_vectors` 10.6%; the C are all header
     `static inline`. 109.53 → 102.60 s (**−6.3%**), kissat 96.36 s alongside.
  2. `struct assigned` repacked to 16 bytes (`internal.rs`: the five bools as
     bits of one `flags` word, `repr(C)`, compile-time size guard). Five
     plain bools made it 20 bytes — a 25% larger var-indexed array on the
     hottest random-access path. 102.14 → 100.39 s (**−1.7%**), kissat 95.17 s.
  3. `sort_literals_inline`/`move_smallest_literal_to_front`
     `#[inline(always)]` (C static inline; ours was a separate 0.6% symbol),
     `watch_large_clauses` walked by word offset with unchecked reads, and
     `backtrack_without_updating_phases` loops with unchecked trail/assigned
     indexing. 102.00 → 99.76 s (**−2.2%**), kissat 95.80 s.
  4. PUSH_ARRAY ported unchecked (`resize.rs` keeps `trail` capacity at
     `size`, `assign` writes without the Vec grow check) plus unchecked
     indexing in `move_smallest_literal_to_front`: 100.83 → 100.39 s
     (−0.4%, within run noise; kept for structural fidelity), kissat 96.35 s.
  5. `substitute_clauses` literal loop read unchecked (it carried +73% of
     the C's branches): 100.38 → 99.17 s (**−1.2%**), kissat 95.52 s.
  Net: 109.5 → 99.2 s on the same deal, gap **+13.7% → +3.8%**.
  Whole-program `perf stat` at that point: cycles +3.4%, instructions
  +15.6% (253.8G v 219.6G), branches +27% (49.1G v 38.7G), L1/LLC misses
  equal — the residual is instruction overhead hiding under memory latency,
  not extra misses.
- 2026-09-03 REJECTED: kissat's FAST_ASSIGN shape — hoisting raw base
  pointers of arena/assigned/values/watch-stack into `propagate_literal`
  locals and threading `values`/`assigned` through `fast_assign` exactly as
  `fastassign.h` does. Static solver-field reloads in the loop did drop
  (111 → 91) but the paired run was 104.62 s v 102.60 s inline-only (+2%)
  and `perf stat` showed +1.06% instructions and +2.9% LLC misses: LLVM's
  codegen with the separate raw pointers is worse (more spills), so the C
  idiom does not transfer. Do not re-add without new evidence.
