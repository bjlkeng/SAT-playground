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

Status (2026-09-04): all engines ported; counter parity exact at
`--conflicts=100000` on the 20 discriminating cells + 14 medium cells and on
full brocard runs; wall ratio v kissat 1.00-1.06x (memory-bound giants
1.00-1.02x, cache-resident search-bound cells 1.02-1.06x). **Phase-8 acceptance run PASSED
2026-09-04** (paired 400x2 @ 3600 s / 16 GB / 16+16 pinned physical cores,
no proofs, `tools/run_kissat_full.sh`; logs
`log/kissat-full-accept-20260904-072748` v
`log/solver13-full-accept-20260904-072750`, report via
`tools/compare_full_runs.py`): solved **312 v kissat 313** (floor 306.7),
PAR-2 **792,489 v 786,872 = 1.0071x** (ceiling 1.02x), **zero SAT/UNSAT
contradictions**; 284 both-solved cells wall geomean **1.013x** (158,884 v
156,921 s). The one lost cell, lockchart-group3-L15-K29-p4, is a 53 s
wall-coin (kissat UNSAT at 3546.6 s); the other two wall-band cells
(frb80-14-1 3396 s, bp4_LPI_FPBEQ_ZR 3071 s) held. Both arms abort on
memory (exit 134) on pj2002_k500 and 17.normalised. Residual by family:
Kakuro 1.15-1.22x (4 cells; 490 MB CNFs, parse/giant-clause bound),
REGRandom 1.15x, crusti 1.11x; the `N.normalised` family runs 0.81-0.94x
(faster than the C). Measured results
(all tier-1 probes, NOT acceptance evidence):

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
  6. Unchecked `*propagate++` trail read and `WATCHES (not_lit)` lookups in
     the propagation path and the assign prefetch: 100.00 → 99.97 s (no
     measurable change; kept — it is the C's shape), kissat 95.25 s.
  Net: 109.5 → 99.2 s on the same deal, gap **+13.7% → +3.8%**.
- 2026-09-03 **brocard was the memory-bound best case.** Paired step-5 v
  kissat at `--conflicts=100000` on other discriminating cells (identical
  conflict counts, statuses match): circuit 4.35 v 3.24 s (**1.34x**),
  Timetable_C_392 18.9 v 14.7 s (1.29x), Kakuro 92.7 v 71.9 s (1.29x),
  REGRandom 6.5 v 5.4 s (1.19x), battleship 0.38 v 0.29 s. perf on those
  cells: every engine 15-90% slower with the same trajectory — the crate's
  checked indexing (+27% branches over the C) exposed once misses stop
  hiding it. Two universal fixes, each paired on brocard + circuit +
  Timetable with parity exact:
  7. `ClauseRef`/`ClauseMut`/`arena.clause()` unchecked in release
     (debug-asserted), like `kissat_dereference_clause` under NDEBUG:
     circuit 4.31 → 4.19 s, Timetable 20.82 → 19.48 s, brocard 100.41 →
     98.39 s (C 3.26 / 15.63 / 96.35).
  8. `src/uvec.rs`: `UVec<T>`, a Vec newtype whose `[]` is unchecked in
     release (range slicing stays checked); values, marks, assigned, flags,
     links, watches, trail, frames, the three phase arrays and the shared
     watch stack switched to it — ~800 index sites at once with no call-site
     edits. circuit 4.20 → 3.92 s (**1.20x** C), Timetable 20.18 → 17.07 s
     (**1.18x**), brocard 99.42 → 96.80 s (**1.000x**, C 96.83).
  9. Loop-local base pointers (arena / values / watch stack) inside
     `propagate_literal` only — the C's `ward *const arena`, `value *const
     values` locals — with `fast_assign` still taking `&mut Solver` (the
     earlier rejected variant also threaded the pointers through assign).
     4-way paired with step 8 and kissat: circuit 1.191x → 1.164x, brocard
     1.015x → 1.009x, Timetable unchanged. Kept.
  10. kitten's per-var/per-lit arrays (`vars`, `links`, `marks`, `values`,
     `failed`, `phases`, `import`, `watches`) on `UVec`: Timetable 1.187x →
     1.138x, circuit 1.164x → 1.152x, brocard 1.009x → 1.007x.
  11. `Heap` arrays (`stack`, `score`, `pos`) on `UVec` and the heap
     operations `#[inline(always)]` (C: `inlineheap.h` static inline, folded
     into `kissat_next_decision_variable`; ours were three separate symbols
     carrying 1.9k branch samples v the C's 458 on circuit). Paired 3-cell
     run: Timetable 1.230x → 1.118x (**−9%**), circuit 1.172x → 1.160x,
     brocard 1.014x → 1.001x.
  12. kitten: `klauses` on `UVec` and `#[inline(always)]` on the helpers
     kitten.c has as static inline (watch_klause, assign,
     propagate_literal, propagate, move_to_front, unassign, the klause
     accessors). Timetable 1.140x → 1.126x, brocard 1.016x → 1.007x,
     circuit flat.
  13. The remaining Solver stacks (analyzed, levels, minimize, poisoned,
     promote, removable, shrinkable, clause, shadow, delayed, etrail, units,
     sorter) on `UVec`: circuit 1.160x → 1.142x, SCPC-500-14 1.203x →
     1.180x, Timetable/brocard flat (paired 4-cell run, parity exact).
  `parity.py --conflicts 100000` (20 discriminating cells, full default
  config): 20/20 exact on the step-5, step-10, step-11 and step-13 (HEAD
  4b0ba3f) binaries; every
  step verified 80-counter exact on brocard + circuit + Timetable (+ SCPC
  from step 13 on).
- 2026-09-04 wider paired check, step-11 v kissat, 10 `sat-comp-2025-medium`
  cells at `--conflicts=100000` (all UNKNOWN at the limit, identical conflict
  counts): ratios 1.137-1.201, i.e. **~1.17x on search-bound cells**;
  brocard-class memory-bound cells sit at 1.00-1.01x. On SCPC-500-14
  `perf stat`: instructions +6.6%, branches +14%, L1-icache misses 2.9x the
  C's (30.6M v 10.7M), dcache misses equal; per-function instruction counts
  put `search_propagate` EQUAL to the C (84557 v 84305 samples) — the
  residual is the analyze cluster (+8%), kitten (+25%), sparse collect, and
  front-end pressure.
- 2026-09-04 REJECTED: `#[inline(never)]` on deduce_first_uip_clause /
  bump_analyzed / shrink_clause / minimize_clause / learn_clause to mirror
  kissat's no-LTO translation-unit boundaries (the icache profile put 30% of
  RS misses in the fully-inlined `analyze`). Same-core `perf stat` on SCPC:
  icache misses 27.5M → 28.8M (no reduction), cycles −1.3%; paired 4-cell
  run circuit −0.8%, SCPC −1.5%, Timetable +1%, brocard flat — a wash, so
  not kept. The icache excess is not from analyze's inlining.
  Whole-program `perf stat` at that point: cycles +3.4%, instructions
  +15.6% (253.8G v 219.6G), branches +27% (49.1G v 38.7G), L1/LLC misses
  equal — the residual is instruction overhead hiding under memory latency,
  not extra misses.
- 2026-09-04 **the icache/fmt residual found and fixed: eager verbose-message
  formatting.** `print::extremely_verbose/very_verbose/verbose/phase` take
  `impl Display`, and ~160 call sites built the message with `format!(...)`
  BEFORE the verbosity check — so every restart (`restarting`), every
  `kimits::delaying` and every inprocessing phase formatted floats and
  malloc'd a String at verbosity 0 (the C's `kissat_extremely_verbose` is a
  macro that tests verbosity first). perf: `float_to_decimal_common_shortest`
  0.75% of circuit cycles, `format_inner`+`malloc` 5.5% of the L1-icache
  misses. Fix: `format!` → `format_args!` inside those calls (arguments are
  still evaluated, formatting is not; `fmt::Arguments` is `Display`), and
  `very_verbose_if_not_bumpreasons` takes `impl Display`. Paired 3-rep runs
  (s15/s16 v kissat v step-13, pinned, idle siblings): SCPC-500-14 5.03 →
  4.51 s (C 4.32: **1.16x → 1.044x**), circuit 3.58 → 3.26 s (C 3.17:
  **1.13x → 1.03x**), Timetable 16.9 → 16.4 s (C 15.3: 1.11x → 1.07x),
  brocard 93.4 v 91.9 s (1.017x); counters exact on all four. SCPC `perf
  stat`: icache misses 27.9M → 20.8M (C 10.0M), instructions +3.0%, branches
  +6.4%, cycles +6.6% (C reference).
- 2026-09-04 kitten `import_literal` `#[inline(always)]` (C static, inlined
  into `kitten_clause_with_id_and_exception`; ours was an out-of-line call
  with the +0x0/+0x7 prologue visible in the profile) and `enlarge_external/
  enlarge_internal` `#[cold] #[inline(never)]`. 4-way paired: Timetable
  −1.8%, circuit +1.2%, SCPC +0.9% — a wash, kept for the C's shape. Also
  inlining new_reference/new_original_klause/export_literal was no better.
- 2026-09-04 Timetable phase split (`--profile=2`, both binaries; note
  `--profile=4` doubles the runtime of BOTH arms and hides the gap): search
  0.983x, simplify 1.104x (eliminate **1.161x**, +0.57 s of the +0.88 s
  total), probe 1.068x (sweep 1.12x, factor 1.11x). The remaining excess is
  in elimination (kitten definition extraction, `inlined_connect_clause`
  2x — its cost sits on the `*end != INVALID` watch-slot read in
  `push_vectors`, same instruction the C pays for), not in search.
- 2026-09-04 wider paired check (step-17b = HEAD 7d46c4c, 14
  `sat-comp-2025-medium` cells, `--conflicts=100000`, 7 pairs at a time on
  physical cores with idle siblings; scratchpad `wideout/`): see the plan
  handoff for the table — ratios 0.99-1.06 on the 11 search-bound cells,
  counters exact everywhere.
- 2026-09-03 REJECTED: kissat's FAST_ASSIGN shape — hoisting raw base
  pointers of arena/assigned/values/watch-stack into `propagate_literal`
  locals and threading `values`/`assigned` through `fast_assign` exactly as
  `fastassign.h` does. Static solver-field reloads in the loop did drop
  (111 → 91) but the paired run was 104.62 s v 102.60 s inline-only (+2%)
  and `perf stat` showed +1.06% instructions and +2.9% LLC misses: LLVM's
  codegen with the separate raw pointers is worse (more spills), so the C
  idiom does not transfer. Do not re-add without new evidence.
