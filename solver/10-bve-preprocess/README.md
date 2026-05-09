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
- parse-time canonical original-clause insertion for input clauses: duplicate literals are removed,
  tautologies / already-satisfied clauses are skipped, root units are enqueued immediately, and
  surviving clauses use the same normalized representation as preprocessing-generated clauses
- diagnostic `SAT_INITIAL_CLAUSE_MODE` switch for initial clause loading experiments:
  `canonical-sorted` (default), `input-order`, or `raw`
- DRAT logging for preprocessing-generated resolvents/units
- MiniSat-style elimination stack entries and SAT model extension
- SAT output from a complete model snapshot instead of the mutable live assignment vector
- one-shot cleanup after preprocessing: drop occurrence metadata, rebuild branch heap, and force GC
- lazy deleted-clause watcher cleanup during propagation, with strict detach retained where
  preprocessing removes an original clause before tombstoning it

Still incomplete:

- asymmetric branching clause strengthening
- `use_rcheck` implied-clause checks
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

Latest rerun after the lazy deleted-clause watcher cleanup on 2026-05-09:

- `cargo test` in `solver/10-bve-preprocess`: 48 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-09-15-38-19`

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

## Parse-Time Canonical Insertion Follow-up

On 2026-05-09, initial parsed clauses were routed through the same MiniSat-style original-clause
normalization path used by preprocessing-generated resolvents. Validation:

- `cargo test` in `solver/10-bve-preprocess`: 48 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-09-07-33-09`

Benchmark rerun:

- `log/bench-10-bve-preprocess-2026-05-09-00-21-53/results.csv`
- 5/5 solved, PAR-2 `946.556`

Diff versus the previous accepted `minisat-simp-five` run
(`log/bench-10-bve-preprocess-2026-05-08-15-56-37/results.csv`):

| Instance | Before | After | Delta |
|---|---:|---:|---:|
| `sudoku-N30-12` | `184.240s` | `357.536s` | `+173.296s` |
| `SC25_Timetable...392...` | `89.198s` | `29.561s` | `-59.637s` |
| `REGRandom-K4...` | `205.602s` | `201.044s` | `-4.558s` |
| `mp1-Nb7T46` | `43.106s` | `45.757s` | `+2.651s` |
| `Kakuro...` | `18.404s` | `312.658s` | `+294.254s` |

Follow-up Kakuro isolation runs:

| Mode | Full BSR | Time | Results |
|---|---:|---:|---|
| `canonical-sorted` | on | `312.658s` | `log/bench-10-bve-preprocess-2026-05-09-00-21-53/results.csv` |
| `raw` | on | `454.667s` | `log/bench-10-bve-preprocess-2026-05-09-07-20-27/results.csv` |
| `canonical-sorted` | off | `95.817s` | `log/bench-10-bve-preprocess-2026-05-09-07-28-51/results.csv` |
| `raw` | off | `19.140s` | `log/bench-10-bve-preprocess-2026-05-09-07-31-04/results.csv` |
| `input-order` | off | `19.508s` | `log/bench-10-bve-preprocess-2026-05-09-07-32-00/results.csv` |

Conclusion: parse-time canonical insertion closes a real MiniSat `addClause_()` semantic gap and
keeps correctness intact, but it should not be considered a default performance win yet. The Kakuro
regression is a compound search-path sensitivity: full BSR/work-loop policy is the largest factor,
and sorted canonical literal order adds another large slowdown. Canonical semantics that preserve
input literal order recover the old fast behavior when full BSR is disabled, so duplicate removal,
tautology skipping, and immediate root units are not the observed Kakuro problem by themselves.

## MiniSat CDCL Compatibility Follow-up

On 2026-05-09, the CDCL core was moved closer to MiniSat in five targeted areas:

- learned-clause budget adjustment now starts at 100 conflicts and is reset after preprocessing
  from the residual original-clause count unless `SAT_REDUCE_DB_INIT`,
  `SAT_REDUCE_DB_INTERVAL`, or `SAT_POST_PREPROCESS_REDUCE_DB_RESET` override it
- conflict analysis defaults to MiniSat's `seen`-only behavior and skips literal position 0 in
  reason clauses; the older solver-10 `scratch_resolved` behavior remains available with
  `SAT_CONFLICT_ANALYSIS_MODE=resolved`
- variable and learned-clause activities now use `f64`; learned-clause activity uses two arena
  words
- proof generation remains enabled by default, with `SAT_PROOF=off` available only as a diagnostic
  mode
- branch defaults are MiniSat-like: variable-order tie-breaking and negative initial polarity;
  the previous occurrence-count ordering is available with `SAT_BRANCH_MODE=occurrence`

Validation:

- `cargo test` in `solver/10-bve-preprocess`: 48 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-09-10-42-44`

Benchmark command:

```bash
bash tools/bench.sh -t 600 -m 16384 -d benchmarks/profiling/minisat-simp-five solver/10-bve-preprocess
```

Before/after logs:

- before: `log/bench-10-bve-preprocess-2026-05-09-10-19-52/results.csv`
- after: `log/bench-10-bve-preprocess-2026-05-09-10-43-00/results.csv`

| Instance | Before | After | Delta |
|---|---:|---:|---:|
| `sudoku-N30-12` | `340.515s` | `353.204s` | `+12.689s` |
| `SC25_Timetable...392...` | `53.842s` | `32.634s` | `-21.208s` |
| `REGRandom-K4...` | `204.339s` | `226.317s` | `+21.978s` |
| `mp1-Nb7T46` | `44.788s` | `46.763s` | `+1.975s` |
| `Kakuro...112...` | `303.744s` | `288.306s` | `-15.438s` |
| **PAR-2** | **`947.228`** | **`947.224`** | **`-0.004`** |

Timetable trace stats, with default proof generation:

| Metric | Before | After |
|---|---:|---:|
| Preprocess time | `4.656s` | `4.665s` |
| Eliminated vars | `106138` | `106138` |
| Resolvents | `334136` | `334136` |
| Subsumed clauses | `57400` | `57400` |
| Strengthened clauses | `126577` | `126577` |
| Search time | `49.085s` | `27.818s` |
| Conflicts | `412742` | `292899` |
| Decisions | `8314512` | `5734322` |
| Propagations | `140891805` | `92303633` |
| Restarts | `1017` | `700` |

The Timetable improvement is therefore search-path driven: preprocessing produced identical counts,
but the MiniSat-like CDCL defaults reduced conflicts by about 29% and decisions by about 31%.
The aggregate five-instance score is effectively flat because the same search-path changes regress
the two UNSAT instances and slightly regress `mp1`.

Proof-off diagnostic on Timetable:

- command added `SAT_PROOF=off` with the same trace settings
- elapsed time changed from `32.731s` to `32.096s`
- search time changed from `27.818s` to `27.243s`
- conflicts/decisions/propagations were unchanged
- no `proof.out` or `proof.out.tmp` was written

Conclusion: proof streaming has measurable but small SAT-side overhead on this target. The larger
effect is the CDCL search trajectory change from MiniSat-compatible analysis/branching defaults.

## Lazy Deleted-Clause Watcher Follow-up

On 2026-05-09, five smaller CDCL/code-level changes from the MiniSat comparison were tested one at
a time against the current solver-10 baseline. The three-instance diagnostic set was Timetable,
K4, and `mp1`, using `SAT_TRACE_PREPROCESS=1`, a very high search trace interval, and a `600s`
per-instance cap.

Diagnostic logs:

- individual-change matrix: `log/diagnostics/individual-2026-05-09/summary.tsv`
- lazy-detach Sudoku/Kakuro validation: `log/diagnostics/individual-2026-05-09/candidate_remaining.tsv`

Three-instance totals:

| Change | Elapsed delta | Search delta | Outcome |
|---|---:|---:|---|
| Trim root-false literals from learned clauses | `-1.326s` | `-0.110s` | Same search counters; noise. |
| Store learned-clause activity as `f32` | `-1.070s` | `+0.132s` | Same search counters; noise. |
| Lazy detach deleted watchers | `-15.821s` | `-15.015s` | Only clear isolated win. |
| MiniSat positive-before-negative literal tie sort | `+8.058s` | `+0.024s` | Worse preprocessing on K4. |
| Attach learned clause after backtrack | `-1.330s` | `+0.304s` | Same search counters; noise. |

The kept change is lazy deleted-clause watcher cleanup:

- ordinary `detach_clause()` is now lazy; deleted or stale watchers are skipped and compacted out
  when the relevant watch list is scanned during propagation
- `detach_clause_strict()` remains available for places that still need eager unlinking
- preprocessing original-clause removal uses strict detach before marking the clause deleted
- propagation tolerates watcher entries whose clause was deleted or whose watched literal moved
  during in-place strengthening

Full five-instance trace validation for the kept change:

| Instance | Baseline elapsed | Lazy detach elapsed | Search delta | Counter movement |
|---|---:|---:|---:|---|
| `sudoku-N30-12` | `359.334s` | `317.959s` | `-41.524s` | conflicts `-16.4%`, decisions `-18.6%`, propagations `-7.7%` |
| `SC25_Timetable...392...` | `32.762s` | `21.599s` | `-11.080s` | conflicts `-28.8%`, decisions `-23.6%`, propagations `-24.5%` |
| `REGRandom-K4...` | `227.351s` | `225.398s` | `-1.244s` | conflicts `+5.9%`, decisions `+5.6%`, propagations `+3.7%` |
| `mp1-Nb7T46` | `46.989s` | `44.284s` | `-2.691s` | same conflicts/decisions/propagations; faster throughput |
| `Kakuro...112...` | `289.194s` | `352.258s` | `+61.406s` | conflicts `+66.0%`, decisions `+49.2%`, propagations `+64.0%` |

Aggregate trace totals:

- elapsed: `955.630s -> 961.498s` (`+5.868s`, `+0.6%`)
- search: `477.397s -> 482.264s` (`+4.867s`, `+1.0%`)

Conclusion: lazy detach is a useful implementation simplification and a real win on Sudoku,
Timetable, and `mp1`, but it is still a search-path tradeoff rather than an aggregate
five-instance performance win. The large `mp1` regression seen in the combined experimental patch
did not reproduce for any single change, so it was an interaction effect and the other four changes
were not kept.
