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
- 2026-09-04 **Kakuro family (post-acceptance item 1).** Paired
  `--profile=2` on Kakuro-easy-112 (490 MB CNF, 18.8M irredundant clauses):
  total 1.152x — probe 1.20x (congruence 1.24x, vivify 1.24x, sweep 1.23x,
  walking 1.20x), preprocess 1.24x, search 1.07x. `perf stat`: instructions
  +10.6%, LLC misses +9%, memory-stall cycles +14%; per function
  `watch_large_clauses` 74.4 v 53.5 G cycles at only +7% instructions (its
  cost is the DRAM miss on the watch-stack slot read in `push_vectors`),
  `extract_gates` +52% instructions (`init_xor_gate_extraction` alone 26 G:
  per-literal `arena.clause(ref).lit(i)` + checked `Vec` counter indexing),
  `walk` +56% (the same push in `connect_large_counters`).
  Fixes, each paired (RS new v kissat v RS previous, pinned, counters exact
  on Kakuro + Timetable + circuit + SCPC):
  14. `vector::PushCursor` — the watch-stack base pointer, length, capacity
     and `usable` decrement hoisted across a clause loop with the generic
     `push_vectors` as the slow path (enlarge relocates the stack, so the
     cursor re-syncs after it). Through `&mut Solver` LLVM reloaded all four
     around every store. Used by `watch_large_clauses`,
     `connect_irredundant_large_clauses`/`inlined_connect_clause` and walk's
     `connect_large_counters`. Kakuro 84.8 → 82.6 s (`watch_large_clauses`
     74.4 → 64.1 G cycles), circuit −1.8%.
  15. Congruence XOR/ITE counting passes walk `clause.lits()` slices with
     `UVec` counters (C pointer walks): −6 G instructions, −0.9% cycles;
     `extract_gates` cycles now equal to the C's congruence cluster.
  16. `move_smallest_literal_to_front` best-update in select form (the C's
     cmov chain): a wash — LLVM still branches; kept for shape.
  17. Verbose-format laziness (above) had already taken restart/delay cost.
  State: Kakuro **1.152x → 1.072x** (78.2 v 73.0 s, `--profile=2` split:
  congruence 1.13x +2.1 s, preprocess 1.14x +1.4 s, vivify 1.09x +1.1 s,
  walking 1.15x +0.8 s, search 1.05x); Timetable **1.026x** (eliminate 1.04x,
  sweep 1.07x, factor 1.09x, backbone 1.17x). The residual in the push
  loops is memory-stall time on identical accesses (layout verified: the
  `[vectors] enlarged`/defrag phase lines match the C's line for line) that
  no code-shape change so far recovers; the no-sort experiment showed the
  literal sort itself is ~2.5 s of Kakuro's 80 s in both arms.
- 2026-09-04 **THE LOAD-SENSITIVITY LAW (REGRandom, factor).** The
  acceptance run's REGRandom 1.15x is not reproducible on a quiet host: paired
  quiet runs put step-21 at **0.98x** kissat (5.10 v 5.19 s), but with 28
  background kissat Kakuro runs on other cores (the acceptance run is a
  32-way load) it is **1.10-1.14x**, core-swap and 4-way layouts agreeing.
  `perf stat` under load: identical offcore requests, L2 misses, LLC loads
  and GHz; front-end fine (RS has fewer undelivered uops and 3x fewer branch
  misses); but `cycle_activity.stalls_l3_miss` 8.98 v 7.27 G and instructions
  +13% (38.4 v 34.1 G). Same misses, less overlap: the extra instructions
  between misses fill the OOO window, so each miss costs more once latency
  rises under load. **Screen perf candidates under load** (28 background
  `kissat -n -q Kakuro.cnf` on cores 8-35, the pair on 2/4) — quiet paired
  timing understates the metric-relevant gap. Instruction count in
  miss-heavy loops is the lever, not miss count.
  18. factor `next_factor`/`factorize_next`: the `for j in 0..size {
     clause(ref).lit(j) }` and `for wi in begin..end { stack[wi] }` loops
     rewritten as slice walks (addr2line attribution had Range::next + lt +
     unchecked_add at ~21% of factor's retired instructions). Instructions
     38.4 → **33.9 G (C 34.1)**; REGRandom **0.92x quiet, 0.96x under load**
     (was 1.12x loaded). Counters exact.
  19. The same two rewrites applied crate-wide by a compile-checked
     transform (scratch `slicify.py`/`slicify2.py`: rewrite every syntactic
     match, build, revert the sites the borrow checker rejects — bodies that
     mutate the arena or call `&mut Solver` methods — and repeat): 55
     literal loops in 19 files + 17 watch-range loops in 7 files. circuit
     −1.8%, SCPC −2.2%, Timetable −0.8%, Kakuro/REGRandom flat; counters
     exact on all five cells; `parity.py --conflicts 100000` on the 20
     discriminating cells: **20/20** for both the literal-loop (09fb200) and
     the watch-range (3843da7) transform binaries.
- 2026-09-04 **loaded screen of the current tree (3843da7)**: 14 wide
  medium cells + circuit/SCPC/Timetable/REGRandom, RS v kissat paired on
  cores 2/4 under 28 background kissat Kakuro runs (scratch
  `loadscreen.sh`, `loadout24/`): case7 1.016, clqcl_50 1.017, crusti 1.046,
  DLTM 0.982, oddball_24 1.006, QG7 1.007, ramsey 1.000, reconf10 0.982,
  RoundRobin 1.036, sudoku 1.029, tseitin_grid 0.981, VanDerWaerden 1.037,
  velev 1.041, xor_op 1.039, circuit 1.006, SCPC 1.043, Timetable 1.032,
  REGRandom 0.987 — geomean ≈ **1.02x under load**, no cell above 1.05x.
  Kakuro quiet 1.077x (instructions 327.5 v 306.4 G, cycles 292 v 271 G).
- 2026-09-04 REJECTED: unchecked `counts[lit]` in vivify's `count_literal`
  and unchecked `dst[pos]` in `radix_scatter` (addr2line attribution had
  them at 19% / 8% of their functions' instructions). Instructions −2 G on
  Kakuro but wall +0.5% Kakuro / +1.2% Timetable, each variant alone also
  slower (vivify-only +3.4%, radix-only +2.3% on a 4-way Timetable run) —
  a code-layout effect; not kept.
- 2026-09-04 **crusti (1.046x loaded) → factor's `schedule_factorization`
  scan, and the `Flags` struct.** crusti's gap is factor again (4.18 v
  3.27 s, 1.28x; instructions 14.5 v 11.5 G) and the inlined-function
  attribution put 44% of factor's cycles in `schedule_factorization`'s
  `for idx in vars { if flags[idx].active ... }` scan (factor.rs:170-171).
  Our `Flags` was 10 bytes (ten bools + a u8) v the C's 2-byte bitfield
  struct, so every var-indexed scan (factor rounds, backbone, sweep,
  eliminate scheduling) streamed 5x the cache lines.
  20. `Flags` packed into a `#[repr(transparent)] u16` with `active()` /
     `set_active(v)` / `factor()` / `factor_or(bits)` / `factor_and(bits)`
     accessors (bit order = the C declaration order; compile-time size
     guard); 124 access sites rewritten by regex over the receiver shapes
     `flags[..]`, `f`, `flags`, `pivot_flags`, zero compile errors. Quiet
     3-rep: crusti 15.92 → 15.58 s (C 14.97, **1.063 → 1.040x**), REGRandom
     5.67 → 5.58 s (C 5.75), circuit 3.22 → 3.27 s (+1.6%, layout), Timetable
     flat, Kakuro +1%; counters exact on all five; 20-cell parity below.
- 2026-09-04 **LAYOUT DIVERGENCE FOUND AND FIXED (invisible to the counters):
  two `SET_END_OF_WATCHES` ports set `watches[lit].end` directly.** The C
  macro is `kissat_resize_vector`: it also memsets the freed tail to
  INVALID and adds it to `vectors.usable`. Without the poison the next push
  into that list saw an occupied slot and relocated the whole vector
  (doubling it, leaving holes), and with `usable` undercounted the defrag
  never triggered. Found via crusti's page faults (119k v the C's 53k) →
  `/usr/bin/time` maxrss **320 MB v 74 MB** → massif (portable build; the
  native one SIGILLs valgrind) putting 512 MB in `enlarge_stack` under
  factor's `new_binary_clause` → `-v` logs: RS `[vectors] enlarged` 2^23 →
  2^27 during factorization-1 where the C stays at 2^23, then `[defrag]
  freed 71M usable 98%` v `5.6M 82%`. Sites: `factor::eagerly_remove_watch`
  (factor.rs:735) and `sweep::substitute_connected_clauses` (sweep.rs:1027);
  every other SET_END_OF_WATCHES port already used `vector::resize_vector`
  (the `watch.c` `end -= 1/2` decrements are faithful as written). After
  the fix (a118aa9): crusti maxrss 72 MB, faults 55k, and the
  `[vectors]`/`[defrag]`/`[arena]` sequences identical to the C on crusti,
  REGRandom, Timetable, circuit, SCPC, velev, sudoku and Kakuro; counters
  exact on all. All 80 `-s` counters had been exact throughout — the bug
  only changed memory layout, RSS and wall.
  **New oracle: `parity.py --phases`** runs both binaries with `-v` and
  diffs the bracketed phase lines (numeric tokens at 1e-5 relative
  tolerance, report rows/options skipped); the pre-fix binary fails crusti
  at phase line 73 (`[vectors] enlarged to 2^24` where the C prints the
  factorization summary). Run it alongside the counter check for any
  change touching vectors, watches, arena growth or clause allocation.
  Remaining cosmetic `-v` differences: `format_count` prints `1000` where
  the C prints `1e3`, and `{}` floats print full expansions where the C
  uses `%g` (values identical).
- 2026-09-03 REJECTED: kissat's FAST_ASSIGN shape — hoisting raw base
  pointers of arena/assigned/values/watch-stack into `propagate_literal`
  locals and threading `values`/`assigned` through `fast_assign` exactly as
  `fastassign.h` does. Static solver-field reloads in the loop did drop
  (111 → 91) but the paired run was 104.62 s v 102.60 s inline-only (+2%)
  and `perf stat` showed +1.06% instructions and +2.9% LLC misses: LLVM's
  codegen with the separate raw pointers is worse (more spills), so the C
  idiom does not transfer. Do not re-add without new evidence.
