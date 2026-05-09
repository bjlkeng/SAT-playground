# MiniSat `simp` Port Design For `solver/10-bve-preprocess`

## Goal

Implement MiniSat's `SimpSolver` preprocessing pipeline in `solver/10-bve-preprocess` faithfully
on top of the current `09-root-simp-opts` Rust baseline.

## Restart Constraints

The next implementation pass should follow these constraints explicitly:

- start from scratch from the current workspace state; do not reuse or port from previous local
  experimental commits
- keep `solver/10-bve-preprocess/src/main.rs` as close to the plain `09-root-simp-opts` baseline
  as practical until the new simplification code is ready
- put the new implementation primarily in a new `src/simp.rs` file so the MiniSat-`simp` work is
  isolated and reviewable

## Benchmarking Approach

The main success criterion is not "some preprocessing improvement"; it is matching the practical
ability of MiniSat `simp` on selected benchmark instances.

Use this exact workflow:

- keep the current five-instance benchmark set in
  `benchmarks/profiling/minisat-simp-five/`
- compare `solver/09-root-simp-opts`, `solver/10-bve-preprocess`, and reference `minisat` with the
  benchmark harness, not ad hoc `run.sh` timings
- run with a 10 minute timeout and 16 GB memory limit:
  - `bash tools/bench.sh -t 600 -m 16384 -d benchmarks/profiling/minisat-simp-five solver/09-root-simp-opts`
  - `bash tools/bench.sh -t 600 -m 16384 -d benchmarks/profiling/minisat-simp-five solver/10-bve-preprocess`
  - `bash tools/bench_reference.sh -t 600 -m 16384 -d benchmarks/profiling/minisat-simp-five minisat`
- use the harness `results.csv` outputs as the source of truth for timing comparisons
- require SAT verification to pass; a faster SAT result with `verified=FAIL` does not count
- treat MiniSat `simp` parity as the target, with solver `09` serving only as the baseline being
  improved upon

## Current Performance Gap Hypotheses

Status after commit `967a312` (`Implement solver 10 BVE preprocessing`):

| Solver | Solved | SAT | UNSAT | Timeouts | PAR-2 | Results |
|---|---:|---:|---:|---:|---:|---|
| `09-root-simp-opts` | 3/5 | 2 | 1 | 2 | `3195.921` | `log/bench-09-root-simp-opts-2026-05-08-09-58-03/results.csv` |
| `10-bve-preprocess` | 4/5 | 3 | 1 | 1 | `1532.975` | `log/bench-10-bve-preprocess-2026-05-08-13-08-41/results.csv` |
| `minisat` | 5/5 | 3 | 2 | 0 | `453.343` | `log/bench-minisat-2026-05-08-09-58-03/results.csv` |

Per-instance gap:

- `sudoku-N30-12`: `10` improves over `09` (`507.092s -> 181.810s`) and is slightly faster than
  MiniSat in some runs.
- `SC25_Timetable...`: `10` regresses slightly versus `09` (`80.349s -> 89.000s`) and is far
  slower than MiniSat (`18.545s`).
- `REGRandom-K4...`: `10` still times out at `600s`; MiniSat solves UNSAT in about `65s`.
- `mp1-Nb7T46`: `10` improves from timeout to `43.275s`, faster than MiniSat's `75.054s`.
- `Kakuro...`: `10` improves from `208.480s` to `18.890s`, faster than MiniSat's `80.111s`.

The reference harness invokes the `minisat` binary, which uses `SimpSolver` with default
`pre=true`, `asymm=false`, `rcheck=false`, and `elim=true`. Therefore the current measured gap is
more likely from MiniSat's default backward-subsumption / BSR / elimination scheduling pipeline than
from optional asymmetric branching or implied-clause checks.

Ordered hypotheses to test:

1. **Backward subsumption and backward-subsumption resolution are the largest missing default
   MiniSat behavior.** MiniSat runs the subsumption queue before and during elimination, deletes
   subsumed clauses, and strengthens clauses by removing one literal. Solver `10` currently runs BVE
   against a much staler formula.
2. **Root assignments generated during preprocessing need to drive subsumption.** MiniSat feeds new
   root assignments through `bwdsub_tmpunit`; solver `10` propagates units but does not use them to
   sweep occurrence lists before search.
3. **Elimination order is too stale.** MiniSat uses an elimination heap whose costs are updated as
   clauses are inserted/deleted. Solver `10` uses one initial sort plus FIFO requeueing of touched
   variables.
4. **Generated resolvents need immediate cleanup.** MiniSat pushes new clauses into the subsumption
   queue. Solver `10` inserts resolvents and updates occurrences, but does not immediately run
   forward/backward subsumption on them.
5. **Some SAT-case gap is probably CDCL-core behavior, not preprocessing.** The timetable instance
   indicates MiniSat's search core, phase/branching details, allocator locality, and learned-clause
   database policy may dominate after preprocessing.
6. **Proof logging adds repo-specific overhead.** Solver `10` logs preprocessing resolvents for DRAT
   correctness; the MiniSat reference run does not produce/check DRAT.

Work queue:

1. Implement and measure backward subsumption + BSR around the existing BVE pass.
2. Add root-assignment-driven subsumption work and verify it changes the random K4 residual formula.
3. Replace FIFO touched-variable requeueing with a dynamic elimination heap.
4. Add counters for eliminated vars, resolvents, subsumed clauses, strengthened literals, root units,
   preprocessing time, and residual live clauses/literals so regressions can be diagnosed without
   guessing.
5. Only after default MiniSat behavior is closer, test optional `asymm` and `rcheck`.

### 2026-05-08 BSR / Heap Experiment Log

Accepted result:

| Variant | Solved | SAT | UNSAT | Timeouts | PAR-2 | Results |
|---|---:|---:|---:|---:|---:|---|
| baseline `10` after commit `967a312` | 4/5 | 3 | 1 | 1 | `1532.975` | `log/bench-10-bve-preprocess-2026-05-08-13-08-41/results.csv` |
| gated full BSR + FIFO non-BSR path | 5/5 | 3 | 2 | 0 | `540.550` | `log/bench-10-bve-preprocess-2026-05-08-15-56-37/results.csv` |
| `minisat` | 5/5 | 3 | 2 | 0 | `453.343` | `log/bench-minisat-2026-05-08-09-58-03/results.csv` |

Per-instance accepted timings:

- `sudoku-N30-12`: `184.240s` (`10` baseline: `181.810s`, MiniSat: around the same range in the
  five-instance harness)
- `SC25_Timetable...`: `89.200s` (`10` baseline: `89.000s`, MiniSat: `18.545s`)
- `REGRandom-K4...`: `205.600s` (`10` baseline: timeout at `600s`, MiniSat: `65.075s` in the
  harness and `61.263s` in a direct verbose run)
- `mp1-Nb7T46`: `43.110s` (`10` baseline: `43.275s`, MiniSat: `75.054s`)
- `Kakuro...`: `18.400s` (`10` baseline: `18.890s`, MiniSat: `80.111s`)

Key diagnosis:

- Full BSR is required for the K4 residual formula. Without BSR, solver `10` eliminates `512`
  variables but grows K4 to `3,100,416` live original literals. MiniSat's verbose run reports the
  same `512` eliminated variables and `825,216` residual clauses but only `2,752,256` literals.
- Gated full BSR reaches that same K4 residual: `512` eliminated variables, `8,192` resolvents,
  `348,160` strengthened clauses/literals, `825,216` residual original clauses, and `2,752,256`
  residual original literals.
- The K4 gap after residual parity is speed: MiniSat reports `39.65s` simplification and `61.26s`
  total CPU time; the gated solver trace spent about `117.05s` in preprocessing and solves the
  instance in `205.600s` in the latest five-instance run.
- After the lazy abstraction resize fix, a single-instance K4 rerun solved in `184.756s`
  (`log/bench-10-bve-preprocess-2026-05-08-16-08-42/results.csv`); rerun the full five-instance
  set if this targeted number needs to replace the aggregate table.

Rejected or constrained attempts:

- **Unconditional full BSR:** solved K4, but overprocessed other formulas. Five-instance PAR-2 was
  `1708.286`, with major regressions on Sudoku (`585.35s`), Timetable (`320.44s`), and Kakuro
  (`578.79s`). Keep full BSR gated.
- **Dynamic elimination heap everywhere:** with BSR gated but heap enabled globally, five-instance
  PAR-2 improved to `693.341` but Timetable regressed to `254.25s`. Keep the heap only on the
  full-BSR-gated path; use the previous FIFO touched-variable queue elsewhere.
- **Occurrence cleanup with clause-membership scans:** profile showed most BSR time in
  `clean_occurs()` and `clause_contains_var()`. Removing the membership scan is correct for the
  current delete/reinsert strengthening model and moved the hotspot into actual BSR relation checks.
- **Post-BVE/root-only scoped BSR:** correctness passed, but K4 still timed out at `180s`; it did
  not delete enough literals to match MiniSat's residual.
- **In-place strengthening:** intended to mimic MiniSat more closely, but no K4 preprocessing trace
  appeared within `150s`, worse than the delete/reinsert path. The likely issue is increased
  occurrence/queue revisit work in this arena implementation. Rejected for now.
- **Raw-slice / branch-based literal hot path:** changed BSR relation internals but regressed K4
  preprocessing from about `117.05s` to `128.54s`. Rejected.

Next hypotheses:

1. Reduce the remaining `117s -> 40s` K4 preprocessing gap by profiling BSR candidate volume and
   queue churn, not by changing residual semantics.
2. Investigate Timetable separately as a CDCL/search-core gap: preprocessing parity work does not
   explain MiniSat's `18.545s` versus solver `10`'s `89.200s`.
3. Revisit in-place strengthening only with a MiniSat-like occurrence vector removal strategy and
   clause-mark queue deduplication; the naive arena mutation was slower.

### 2026-05-08 Fresh 600s MiniSat-Gap Rerun

After the gated-BSR improvement, five more candidate MiniSat-favorable instances were selected from
the 100-instance medium overlap and then rerun from scratch with a `600s` timeout and `16384 MB`
memory cap. These numbers are fresh harness runs, not reused medium-run timings.

Fresh logs:

- `10-bve-preprocess`: `log/bench-10-bve-preprocess-2026-05-08-17-46-01/results.csv`
- `09-root-simp-opts`: `log/bench-09-root-simp-opts-2026-05-08-18-16-07/results.csv`
- `minisat`: `log/bench-minisat-2026-05-08-19-05-28/results.csv`

Summary:

| Solver | Solved | SAT | UNSAT | Timeouts | PAR-2 |
|---|---:|---:|---:|---:|---:|
| `09-root-simp-opts` | 1/5 | 0 | 1 | 4 | `5281.200` |
| `10-bve-preprocess` current gated BSR | 3/5 | 2 | 1 | 2 | `2986.963` |
| `minisat` | 5/5 | 4 | 1 | 0 | `533.762` |

Per-instance fresh results:

| Instance | `10-bve-preprocess` | `09-root-simp-opts` | `minisat` | Notes |
|---|---:|---:|---:|---|
| `849950...circuit_48in64out_with_800gates_4in4out_dist128_seed3` | SAT `49.372s` | TIMEOUT | SAT `86.095s` | Candidate did not hold; current `10` beats MiniSat. |
| `98e8...bp4_TCO_CSO_IXA_LP_ZR` | TIMEOUT | TIMEOUT | SAT `245.131s` | Real MiniSat-over-`10` gap. |
| `9af7...brocard_problem_large` | UNSAT `163.160s` | UNSAT `481.200s` | UNSAT `6.897s` | Real gap; likely preprocessing/search-core efficiency. |
| `f17d...SC25_Timetable_C_406_E_45_Cl_26_D_7_T_50` | TIMEOUT | TIMEOUT | SAT `33.251s` | Real gap; reinforces timetable/search-core hypothesis. |
| `f25a...1-TC-256-K-63` | SAT `374.431s` | TIMEOUT | SAT `162.388s` | Real gap, but current `10` is already much better than `09`. |

Progress so far:

- Solver `10` now clearly improves on `09` on the targeted MiniSat-simp work: the original
  five-instance set moved from `09` PAR-2 `3195.921` to current `10` PAR-2 `540.550`, close to
  MiniSat's `453.343`.
- Gated full BSR is the major accepted improvement. It converted the K4 target from timeout to
  solved and matched MiniSat's residual literal count on that formula.
- Current `10` also improves substantially over `09` on the fresh gap rerun: `2986.963` PAR-2
  versus `5281.200`, with two extra SAT solves and a much faster brocard UNSAT solve.
- MiniSat still has a large advantage on several SAT-heavy or search-sensitive formulas. The fresh
  rerun gives four concrete next targets where MiniSat is still better than current `10`.

Next-session pickup list:

1. Use `98e8...bp4_TCO_CSO_IXA_LP_ZR`, `f17d...SC25_Timetable...`, and
   `f25a...1-TC-256-K-63` as the next SAT-side gap targets. Start with `SAT_TRACE_PREPROCESS=1`
   on current `10` and MiniSat verbose/preprocessing variants if available.
2. For `9af7...brocard_problem_large`, separate preprocessing time from search time. Current `10`
   solves it but is `~24x` slower than MiniSat (`163.160s` vs `6.897s`), so the next question is
   whether MiniSat gets a smaller residual or simply searches it much faster.
3. Do not spend time optimizing the circuit instance as a MiniSat gap; current `10` already wins
   there (`49.372s` vs `86.095s`).
4. Profile before coding. For simplification ideas, first measure opportunity size: subsumption
   hits, BSR strengthens, eliminated variables, resolvents, residual live clauses/literals, and
   preprocessing wall time.
5. Keep the acceptance rule from the optimization workflow: one change at a time, keep only if it
   improves the selected target by more than `3%`, and rerun smoke/cargo tests after kept solver
   logic changes.

### 2026-05-08 Follow-up Debugging Loop

Accepted change:

- Added a very-large-formula full-BSR gate. This targets formulas like
  `9af7...brocard_problem_large`, where MiniSat's default backward subsumption produces a much
  smaller residual than solver `10`'s previous non-BSR path.
- Brocard improved from `163.160s` in the fresh rerun to about `42.3s` in direct tracing
  (`34.9s` preprocessing + `7.4s` search), with 642 conflicts.
- MiniSat's dumped brocard residual was `4,086,123` clauses / `13,124,041` literals; the new
  solver `10` large-BSR path reaches essentially the same residual.

Rejected or incomplete:

- Negative initial phase alone did not solve `bp4`, brocard, or Timetable in the tested bounds.
- Variable-order tie-breaking plus negative phase was worse than the existing occurrence tie on
  brocard and Timetable traces.
- Backtrack-only MiniSat-style phase saving plus negative phase did not fix the SAT-side targets.
- Full BSR on `bp4` and Timetable reaches MiniSat-like residuals, but solver `10` still does not
  solve those SAT instances quickly. Running solver `10` on MiniSat's own residual DIMACS files also
  timed out under the tested `90s` bound for those two, so the remaining gap is CDCL/search-core
  behavior rather than preprocessing residual quality alone.

In this context, "faithfully" means:

- keep the current CDCL search core as the post-preprocessing engine
- port MiniSat `simp` preprocessing semantics, data flow, and cleanup behavior, not just the idea
  of BVE
- preserve SAT model correctness by reconstructing eliminated variables
- preserve UNSAT behavior and proof logging expectations already present in this repo

### 2026-05-08 MiniSat Work-Loop Refactor

Implemented parity items from the current-differences checklist:

- removed the formula-size gate from full backward subsumption / BSR; `SAT_FULL_BSR=off` remains
  only as a diagnostic override
- replaced the split full-BSR/FIFO preprocessing flow with a persistent MiniSat-style loop over
  touched variables, root assignments, queued subsumption clauses, and dynamic elimination-heap
  variables
- changed BSR strengthening to mutate original clauses in place instead of deleting and reinserting
  them
- updated elimination-heap entries broadly when clauses are added, deleted, strengthened, or
  touched through occurrence lists
- queued generated resolvents immediately for subsumption work and used touched variables to keep
  forward/backward subsumption active continuously

Direct `600s` checks after this refactor:

| Instance | Result | Interpretation |
|---|---:|---|
| `849950...circuit_48in64out_with_800gates_4in4out_dist128_seed3` | SAT `208.1s` | Regresses versus the previous gated path (`49.4s`), but still solves. |
| `98e8...bp4_TCO_CSO_IXA_LP_ZR` | TIMEOUT | Preprocessing now finishes in `7.0s`; search remains the blocker. |
| `9af7...brocard_problem_large` | UNSAT `~15.3s` | Improves strongly; preprocessing `7.2s`, search `8.1s`. |
| `f17d...SC25_Timetable_C_406_E_45_Cl_26_D_7_T_50` | TIMEOUT | Preprocessing now finishes in `5.3s`; search remains the blocker. |
| `f25a...1-TC-256-K-63` | TIMEOUT | With `SAT_FULL_BSR=off`, the refactored code still solves in `375.4s`; MiniSat-like preprocessing exposes a search-core gap. |

MiniSat's own `1-TC` run enters search with the same residual size that solver `10` now reports
after preprocessing (`422669` clauses / `930421` literals) and solves in `162.8s`. That makes the
remaining `1-TC` gap a CDCL/search behavior gap, not a missing work-loop simplification item.

Harness rerun on the same five instances:

| Solver | Solved | SAT | UNSAT | Timeouts | PAR-2 | Results |
|---|---:|---:|---:|---:|---:|---|
| `09-root-simp-opts` | 1/5 | 0 | 1 | 4 | `5281.200` | `log/bench-09-root-simp-opts-2026-05-08-18-16-07/results.csv` |
| `10-bve-preprocess` before this refactor | 3/5 | 2 | 1 | 2 | `2986.963` | `log/bench-10-bve-preprocess-2026-05-08-17-46-01/results.csv` |
| `10-bve-preprocess` MiniSat work-loop refactor | 2/5 | 1 | 1 | 3 | `3823.879` | `log/bench-10-bve-preprocess-2026-05-08-22-51-56/results.csv` |
| `minisat` | 5/5 | 4 | 1 | 0 | `533.762` | `log/bench-minisat-2026-05-08-19-05-28/results.csv` |

Per-instance harness result:

| Instance | Before refactor | After refactor | MiniSat |
|---|---:|---:|---:|
| `849950...circuit_48in64out...` | SAT `49.372s` | SAT `207.602s` | SAT `86.095s` |
| `98e8...bp4_TCO_CSO_IXA_LP_ZR` | TIMEOUT | TIMEOUT | SAT `245.131s` |
| `9af7...brocard_problem_large` | UNSAT `163.160s` | UNSAT `16.277s` | UNSAT `6.897s` |
| `f17d...SC25_Timetable...` | TIMEOUT | TIMEOUT | SAT `33.251s` |
| `f25a...1-TC-256-K-63` | SAT `374.431s` | TIMEOUT | SAT `162.388s` |

Conclusion: the requested preprocessing parity changes are not a good default policy yet. They close
most of the Brocard preprocessing gap but regress SAT-heavy formulas by changing the post-preprocess
search trajectory. Keeping these mechanics requires either a smarter activation policy or a deeper
CDCL/search-core parity pass.

Post-simplification residual-size comparison:

| Instance | `10` vars | MiniSat vars | Delta | `10` clauses | MiniSat clauses | Delta | `10` literals | MiniSat literals | Delta |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `849950...circuit_48in64out...` | `3,095` | `3,101` | `-6` | `58,231` | `58,367` | `-136` | `227,354` | `227,904` | `-550` |
| `98e8...bp4_TCO_CSO_IXA_LP_ZR` | `41,630` | `41,628` | `+2` | `486,653` | `486,584` | `+69` | `1,151,466` | `1,151,259` | `+207` |
| `9af7...brocard_problem_large` | `710,180` | `710,178` | `+2` | `4,086,121` | `4,086,123` | `-2` | `13,124,033` | `13,124,041` | `-8` |
| `f17d...SC25_Timetable...` | `104,728` | `111,547` | `-6,819` | `526,408` | `532,998` | `-6,590` | `1,758,006` | `1,745,331` | `+12,675` |
| `f25a...1-TC-256-K-63` | `12,661` | `12,661` | `0` | `422,669` | `422,669` | `0` | `930,421` | `930,421` | `0` |

Interpretation: four of the five residuals are essentially the same size as MiniSat's. The exact
`1-TC` residual-size match is especially important because solver `10` still times out there while
MiniSat solves it, which isolates that gap to CDCL/search behavior. Timetable is the outlier:
solver `10` removes more variables and clauses but leaves more total literals, creating a denser
residual formula that may be harder for the current search core despite being smaller by clause
count.

Most likely remaining gap driver:

1. **Learned-clause quality and CDCL trajectory.** This is the leading hypothesis. On `1-TC`, the
   residual formula is exactly the same size after preprocessing (`12,661` variables, `422,669`
   clauses, `930,421` literals), but MiniSat solves in `162.8s` at `820,550` conflicts while solver
   `10` timed out at `600s` after passing `1.1M` conflicts in direct tracing. The likely cause is
   different conflict analysis/minimization output, reason/literal order, variable activity, and
   learned-clause retention, not missing simplification.
2. **Watcher and clause/literal ordering.** Even when residual sizes match, the two solvers may not
   have identical clause order, literal order, watch placement, or propagation reason selection.
   Those differences feed directly into different conflicts and learned clauses.
3. **Propagation throughput and allocator/cache behavior.** MiniSat reports about `4.9M`
   propagations/sec on `1-TC`; solver `10` traces are closer to roughly `2.2M` propagations/sec.
   This matters for wall time, but it does not fully explain the timeout because solver `10` also
   follows a worse conflict trajectory.
4. **Branch tie-breaking and phase behavior.** These remain plausible path-dependence sources, but
   negative initial phase, MiniSat-style occurrence tie-breaking, and backtrack-only phase saving
   were already tested as standalone changes and did not close the gap.
5. **Learned database timing.** MiniSat's exact `max_learnts` adjustment and `reduceDB` schedule can
   still interact with learned-clause quality, but it is less likely to be the primary independent
   cause.
6. **Proof logging.** Solver `10` pays SAT-side DRAT logging overhead that MiniSat does not, but the
   identical-residual timeout behavior means this is not the main cause.

Next diagnostic: instrument the first `10k` conflicts on `1-TC` in both solvers and compare learned
clause sizes, backtrack levels, propagations per conflict, decision variables, and top activity-bumped
variables. That should expose whether the divergence starts in conflict analysis, propagation reason
ordering, or learned-clause database retention.

## Current Differences vs MiniSat `simp`

Use this as the current working checklist for future parity/debugging passes.

### Preprocessing Differences

- **Default `simp` work loop is now much closer.** Full BSR is on by default, the loop drains
  touched variables/root assignments/subsumption clauses/heap variables, strengthening is in-place,
  generated resolvents are queued immediately, and the elimination heap is updated from touched
  clause changes.
- **Exact MiniSat data-structure behavior can still differ.** The Rust implementation uses arena
  indexes, `VecDeque`, and lazy occurrence cleanup rather than MiniSat's exact `ClauseAllocator`,
  `Queue`, `Heap`, and `vec<CRef>` mutation patterns. Residual sizes can match while search
  trajectories still diverge.
- **Optional MiniSat simp features are still missing.** Solver `10` does not implement asymmetric
  branching (`asymm`) or implied-clause checks (`rcheck`). These are off by default in the MiniSat
  reference runs, so they are not the primary measured gap yet.
- **Proof logging remains repo-specific.** Solver `10` emits DRAT additions for preprocessing
  transformations; the MiniSat reference timing here does not include SAT-side proof I/O.

### CDCL/Search Differences

- **Solver `10` is not MiniSat's CDCL core.** The remaining `bp4`, Timetable, and `1-TC` gaps
  persist after the requested `simp` work-loop differences are implemented. `bp4` and Timetable
  also timed out when solver `10` was run directly on MiniSat's simplified residual DIMACS files.
- **SAT-side search trajectory differs.** Solver `10` can process hundreds of thousands to millions
  of conflicts on those MiniSat residuals without finding SAT, while MiniSat finds SAT much earlier.
- **Phase and branching experiments did not close the gap.** Negative initial phase, MiniSat-style
  variable-order tie-breaking, and backtrack-only full phase saving were tested and rejected as
  standalone fixes.
- **Likely remaining search-core causes:** exact conflict-analysis/minimization behavior, learned
  clause ordering and deletion interactions, watcher/literal ordering effects during propagation,
  restart/search-budget interactions, and subtle phase-saving path dependence.
- **Proof logging may add overhead.** Solver `10` logs DRAT clauses during SAT runs and deletes the
  proof file if SAT; MiniSat reference runs do not pay that SAT-side proof I/O cost. This can hurt
  throughput but does not explain the full SAT-side gap because the timeout persists even on
  MiniSat's simplified residual formulas.

### 2026-05-09 Parse-Time Canonical Insertion

Implemented the current checklist item to route initial parsed clauses through the same canonical
original-clause insertion semantics used for preprocessing resolvents:

- duplicate literals are removed and clauses are sorted into the normalized internal order
- tautological or already-satisfied clauses are skipped instead of allocated
- root units are enqueued immediately instead of stored as persistent original unit clauses
- contradictory root units poison the persistent solver status during construction
- diagnostic `SAT_INITIAL_CLAUSE_MODE` is available for sensitivity checks:
  `canonical-sorted` (default), `input-order`, or `raw`

Validation:

- `cargo test`: 48 passed
- `bash tools/smoke_test.sh solver/10-bve-preprocess`: 9/9 passed
- smoke log: `log/2026-05-09-07-33-09`

Benchmark rerun on `benchmarks/profiling/minisat-simp-five` with `600s` timeout and `16384 MB`:

| Solver/run | Solved | SAT | UNSAT | Timeouts | PAR-2 | Results |
|---|---:|---:|---:|---:|---:|---|
| previous accepted `10` | 5/5 | 3 | 2 | 0 | `540.550` | `log/bench-10-bve-preprocess-2026-05-08-15-56-37/results.csv` |
| parse-canonical `10` | 5/5 | 3 | 2 | 0 | `946.556` | `log/bench-10-bve-preprocess-2026-05-09-00-21-53/results.csv` |
| `minisat` | 5/5 | 3 | 2 | 0 | `453.343` | `log/bench-minisat-2026-05-08-09-58-03/results.csv` |

Per-instance diff:

| Instance | Previous `10` | Parse-canonical `10` | Delta | MiniSat |
|---|---:|---:|---:|---:|
| `sudoku-N30-12` | `184.240s` | `357.536s` | `+173.296s` | `214.501s` |
| `SC25_Timetable...392...` | `89.198s` | `29.561s` | `-59.637s` | `18.545s` |
| `REGRandom-K4...` | `205.602s` | `201.044s` | `-4.558s` | `65.132s` |
| `mp1-Nb7T46` | `43.106s` | `45.757s` | `+2.651s` | `75.054s` |
| `Kakuro...` | `18.404s` | `312.658s` | `+294.254s` | `80.111s` |

Kakuro isolation runs:

| Initial mode | Full BSR | Time | Results |
|---|---:|---:|---|
| `canonical-sorted` | on | `312.658s` | `log/bench-10-bve-preprocess-2026-05-09-00-21-53/results.csv` |
| `raw` | on | `454.667s` | `log/bench-10-bve-preprocess-2026-05-09-07-20-27/results.csv` |
| `canonical-sorted` | off | `95.817s` | `log/bench-10-bve-preprocess-2026-05-09-07-28-51/results.csv` |
| `raw` | off | `19.140s` | `log/bench-10-bve-preprocess-2026-05-09-07-31-04/results.csv` |
| `input-order` | off | `19.508s` | `log/bench-10-bve-preprocess-2026-05-09-07-32-00/results.csv` |

Updated conclusion: canonical parse insertion is semantically implemented, but the earlier
"literal/order changes" explanation was too broad. On Kakuro, full BSR/work-loop policy is the
largest confirmed regression driver; parse canonicalization contributes mainly through sorted
literal order. Canonical semantics that preserve input literal order match the raw fast path when
full BSR is disabled, so duplicate removal, tautology skipping, and immediate root units are not the
observed issue on that instance.

Primary MiniSat references:

- `benchmarks/reference-solvers/minisat/minisat/simp/SimpSolver.h`
- `benchmarks/reference-solvers/minisat/minisat/simp/SimpSolver.cc`
- `benchmarks/reference-solvers/minisat/minisat/core/Solver.cc`
- `benchmarks/reference-solvers/minisat/minisat/simp/Main.cc`

## What MiniSat `SimpSolver` Actually Adds

MiniSat `simp` is not a single BVE routine. It is a preprocessing subsystem layered on top of the
core solver.

### Entry points and control flow

- `solve_()` freezes assumptions temporarily, runs `eliminate()`, then falls back to the core CDCL
  solver, and finally extends the model if SAT.
- `Main.cc` calls `S.eliminate(true)` once after parsing. That means preprocessing normally runs as
  a separate phase and then frees its own heavy bookkeeping before search.
- `eliminate()` itself starts by calling the core `simplify()` pass, then loops while there is
  touched work, pending root assignments for subsumption, or eliminable variables left in the heap.

### Simplification state that exists only for preprocessing

MiniSat adds these persistent structures beyond the core solver:

- `occurs[var]`: occurrence lists of original clauses containing `var`
- `n_occ[lit]`: literal occurrence counts used by the elimination-cost heap
- `elim_heap`: heap ordered by `n_occ[x] * n_occ[~x]`
- `subsumption_queue`: queue of clauses that need backward-subsumption work
- `touched[var]` and `n_touched`: dirty-variable tracking
- `frozen[var]` and `frozen_vars`: variables that must not be eliminated
- `eliminated[var]`: variables removed from the clause database
- `elimclauses`: packed model-extension log for reconstructing eliminated assignments
- `bwdsub_tmpunit`: dummy clause reused to run backward subsumption against new root assignments
- mode flags such as `use_simplification`, `remove_satisfied`, `use_asymm`, `use_rcheck`,
  `use_elim`

### Transformations MiniSat performs

At decision level `0`, `SimpSolver` can:

- check whether a candidate clause is already implied (`use_rcheck`)
- run backward subsumption
- run backward subsumption resolution / clause strengthening
- run asymmetric branching-based clause strengthening (`use_asymm`)
- run bounded variable elimination (`use_elim`)
- remove satisfied clauses and trim root-false literals through the core `simplify()` path
- reconstruct the SAT model through `extendModel()`

### Key behavioral details worth preserving

- assumptions are frozen before preprocessing
- elimination only applies to unassigned, unfrozen, non-eliminated variables
- elimination is bounded by both clause-growth and resolvent-size checks
- new clauses inserted during preprocessing immediately feed back into occurrence lists, touched
  variables, and the subsumption queue
- preprocessing can be turned off permanently after a one-shot run, at which point MiniSat drops
  occurrence/subsumption state and disables extra clause allocator fields
- model extension is required for SAT because eliminated variables are no longer in the live clause
  database

## Current Solver 10 Baseline

The current Rust solver already has a useful subset of MiniSat core behavior:

- arena-based clause storage with a learned-clause extra word
- watched-literal propagation with blocker fast path
- root-level simplify gating through `simplify_assigns` and `simplify_props_remaining`
- learned-clause activity, EVSIDS, restarts, reduction, and GC
- packed clause references that can survive relocation

Relevant current code points:

- solver state: `src/main.rs:184-271`
- root simplify helpers: `src/main.rs:650-743`
- root `simplify()`: `src/main.rs:1262-1285`
- learned-clause insertion/deletion paths: `src/main.rs:1292-1397`
- main solve loop root simplify hook: `src/main.rs:1741-1824`

This is still a `09`-style root simplifier, not a MiniSat `simp` solver:

- there is no original-clause occurrence index
- there is no touched-clause queue
- there is no elimination-cost heap
- there is no frozen/eliminated-variable tracking
- all `add_clause*` helpers currently mean "learned clause", not "general solver clause insertion"
- there is no SAT model extension for eliminated variables
- there is no one-shot `eliminate(true)` preprocessing phase before search

## Gap Analysis: MiniSat vs Current Rust Solver

### 1. Clause ownership and indexing

MiniSat `SimpSolver` preprocessing works over original clauses. The current Rust solver has:

- `original_clause_ids`
- `learned_clause_ids`
- deletion and trimming logic that already treats originals and learned clauses differently

What is missing is a second indexing layer over original clauses:

- per-variable occurrence lists
- literal counts for elimination cost
- touched-variable tracking
- a queue of original clauses awaiting backward-subsumption work

### 2. General clause insertion API

MiniSat's `addClause_()` is the preprocessing entry point for original clauses. It:

- optionally skips the clause if already implied
- inserts it through the core solver
- pushes the new clause into the subsumption queue
- updates `occurs`, `n_occ`, `touched`, and the elimination heap

Current solver 10 has only learned-clause insertion (`add_clause_from_slice()`) and initial parse
construction in `new()`. A faithful port needs a split between:

- original-clause insertion used during parse and preprocessing
- learned-clause insertion used during CDCL conflict analysis

Important semantic gap:

- MiniSat `addClause_()` does not just append a raw clause. It sorts literals, removes duplicates,
  drops root-false literals, treats tautological / already-satisfied clauses as no-ops, turns
  units into immediate root assignments plus propagation, and reports UNSAT through the solver
  state.
- MiniSat only indexes clauses that survive this normalization as non-unit allocated problem
  clauses. Empty, satisfied, tautological, and unit inputs do not become persistent entries in
  `occurs`, `subsumption_queue`, or the problem-clause vector.
- the current Rust `parse_cnf()` + `Solver::new()` path bulk-loads raw clauses directly into the
  arena and only later enqueues root units, so parse-time and preprocessing-time clause insertion
  do not currently share MiniSat-like semantics

The port should therefore route both initial problem construction and preprocessing-generated
resolvent insertion through one canonical original-clause insertion path rather than keeping
`Solver::new()` as a special raw-loader.

### 3. Clause deletion semantics

MiniSat uses one `removeClause()` path that:

- updates occurrence counts
- smudges occurrence lists
- then delegates to the core clause detach/remove path

Current solver 10 has:

- root-simplify deletion for originals and learneds
- learned-only deletion for database reduction

What is missing is a preprocessing-aware original-clause deletion path that keeps occurrence
metadata consistent.

### 4. Root simplification semantics

MiniSat core `simplify()`:

- propagates
- gates on `simpDB_assigns` and `simpDB_props`
- removes satisfied learned clauses
- conditionally removes/trims original clauses if `remove_satisfied`
- rebuilds the order heap
- refreshes the simplify budget

Current solver 10 is already close here, but it differs in important ways:

- it never has a `remove_satisfied = false` mode
- it does not support released/free variables cleanup
- it trims only originals and intentionally never trims unsatisfied learned clauses
- it has no interaction with preprocessing data structures

For the full `simp` port, the root simplify path should stay intact as the foundation, but its
deletion/trim operations must feed occurrence bookkeeping and variable-release semantics where
relevant.

### 5. Backward subsumption pipeline

MiniSat's backward subsumption loop matters because BVE is not run against a stale database. It:

- drains `subsumption_queue`
- also processes new root assignments via `bwdsub_tmpunit`
- picks the least-populated variable in the candidate clause to limit scans
- can either delete a subsumed clause or strengthen it by removing one literal
- recursively feeds new work back into the queue

Current solver 10 has none of this machinery. Without it, a "BVE port" would not be faithful.

### 6. Asymmetric branching

MiniSat optionally strengthens clauses with asymmetric branching before elimination:

- temporarily assigns negations of all non-`v` literals in a clause
- if propagation conflicts, the remaining `v`-literal is removable
- then it strengthens the clause and reruns backward subsumption

This is separate from BVE. It must be modeled as an optional preprocessing pass over a variable's
occurrence list.

### 7. Variable elimination

MiniSat's elimination loop has three distinct parts:

1. split the occurrence list into positive and negative clause sets
2. estimate whether the cross product is allowed using `grow` and `clause_lim`
3. if allowed:
   - mark the variable eliminated
   - record extension clauses in `elimclauses`
   - delete all old clauses containing the variable
   - add every non-tautological resolvent
   - clear the occurrence list
   - rerun backward subsumption

Current solver 10 has none of this state or flow.

### 8. Model extension

MiniSat stores enough data in `elimclauses` to reconstruct assignments for eliminated variables
after SAT. That is not optional if the competition interface requires a satisfying assignment over
the original variables.

Current solver 10 prints assignments directly from the live search state. If variables are
eliminated without extension, SAT output will be incomplete or wrong.

### 9. One-shot preprocessing cleanup

MiniSat `eliminate(true)` is important operationally:

- frees occurrence lists and queues
- disables simplification-only allocator fields
- restores normal `remove_satisfied`
- records `max_simp_var`
- rebuilds the order heap
- forces a full GC

That cleanup keeps search lean after preprocessing. A faithful port should keep the same split:

- heavy structures during preprocessing
- compact search-only state after preprocessing is done

## Proposed Rust Design

### A. Keep the current CDCL core as the base

Do not rewrite the whole solver around a different architecture. Reuse:

- clause arena
- watcher representation
- conflict analysis
- restarts / reduction
- proof logging

The port should add a preprocessing subsystem around the existing arena and clause IDs.

### B. Add explicit clause kinds and insertion paths

Introduce separate APIs:

- `add_original_clause_from_slice()`
- `add_learned_clause_from_slice()` or keep the current name for learned insertion
- `remove_clause_preprocess()` for original-clause-aware deletion

Reason:

- preprocessing and CDCL currently share too much insertion/deletion surface
- MiniSat updates occurrence metadata only for problem clauses and preprocessing-generated
  resolvents, not for learned clauses

Important API design note:

- original-clause insertion should not just return `usize`, because MiniSat-style insertion has
  multiple semantically distinct outcomes:
  - clause rejected as already satisfied / tautological / implied no-op
  - clause normalized into a root unit and propagated without allocating a persistent clause
  - clause allocated as a live non-unit original clause and indexed for preprocessing
  - clause caused immediate UNSAT
- the Rust API should represent those outcomes explicitly so callers do not accidentally assume
  every successful insertion produced a live clause id

### C. Add preprocessing-only state to `Solver`

Add fields corresponding to MiniSat's preprocessing subsystem:

- `ok: bool` or `solver_consistent: bool` as the persistent equivalent of MiniSat's core `ok`
  state
- `use_simplification: bool`
- `remove_satisfied_originals: bool`
- `frozen: Vec<bool>`
- `frozen_vars: Vec<usize>`
- `eliminated: Vec<bool>`
- `occurs: Vec<Vec<usize>>`
- `occurs_dirty: Vec<bool>` or a smudged/clean bitset
- `n_occ: Vec<usize>` indexed by literal
- `touched: Vec<bool>`
- `n_touched: usize`
- `subsumption_queue: VecDeque<usize>`
- `queued_for_subsumption: Vec<bool>` to avoid gross duplication
- `elim_heap: binary min-heap by n_occ[pos] * n_occ[neg]`
- `elim_heap_pos: Vec<usize>`
- `bwdsub_assigns: usize`
- `bwdsub_tmpunit`: represent as a tiny scratch clause rather than a real arena clause
- `elim_clauses: Vec<u32>` for model extension
- `model: Vec<u8>` or equivalent SAT-output snapshot separate from the live search assignment
- preprocessing options:
  - `use_asymm`
  - `use_rcheck`
  - `use_elim`
  - `grow`
  - `clause_lim`
  - `subsumption_lim`
  - `simp_garbage_frac`

Important design choice:

- keep occurrence lists only for original/preprocessed clauses
- do not index learned clauses in `occurs`

That matches MiniSat and avoids polluting elimination costs with learned clauses.

Important state-model choice:

- do not try to encode all permanent UNSAT/preprocessing failure states indirectly through
  `has_empty_clause`, failed root enqueue calls, or one-off return values
- MiniSat's simplification pipeline relies on a persistent `ok` bit that every clause insertion and
  simplification step can poison
- the Rust port should add the equivalent explicitly so parse-time clause normalization,
  preprocessing-generated units, backward-subsumption strengthening, and elimination-time resolvent
  insertion all have one shared way to record "the solver is already inconsistent"

Important representation choice:

- if `bwdsub_tmpunit` is not stored as a real clause in the arena, the subsumption queue cannot be
  just "clause ids plus one magic usize" hidden convention
- prefer an explicit queue item type such as `enum SubsumptionCandidate { Clause(usize), RootUnit(i32) }`
  so the backward-subsumption pipeline cannot accidentally treat scratch work as a relocatable
  arena clause

Important invariant to keep:

- once a variable is marked eliminated, future original-clause insertion should reject any clause
  that still mentions it
- this should be enforced both for resolvent insertion and with debug assertions on generic
  preprocessing insertion paths, matching MiniSat's `addClause_()` expectation that eliminated vars
  never reappear in new problem clauses

Important identity rule:

- preprocessing helpers must define clearly whether an operation preserves a clause id or replaces
  it with a new semantic object
- this matters most for strengthening, where a clause can shrink, become a unit, or be deleted
- any id-preserving path must keep watcher state, occurrence membership, queue-dedup state, and
  root-reason references coherent for that same id
- any id-replacing path must clear all old references before exposing the replacement object

### D. Separate "deleted in arena" from "present in occurrence list"

Occurrence lists should be lazy-cleaned, as in MiniSat's `OccLists`:

- deletion should mark affected variables dirty instead of eagerly removing the clause from every
  occurrence vector
- consumers should call a `clean_occurs(var)` helper before iterating a variable's occurrence list

Reason:

- eager vector removal inside every clause delete/strengthen path will be expensive
- MiniSat explicitly uses smudged occurrence lists for this reason

### E. Reuse the current simplify budget counters

The existing `simplify_assigns` / `simplify_props_remaining` fields already mirror MiniSat core
`simpDB_assigns` / `simpDB_props` closely enough. Keep them.

What changes:

- `simplify()` must use the preprocessing-aware delete/trim helpers when simplification is still
  enabled
- after `eliminate(true)`, simplify should revert to the cheaper post-preprocessing mode

### F. Add a model-extension stack exactly once

Implement MiniSat-style extension logging:

- `mk_elim_clause(unit)` pushes literal and trailing length `1`
- `mk_elim_clause(var, clause)` pushes the clause with the eliminated-variable literal moved to the
  front, then stores the clause length
- `extend_model()` walks this vector backward after SAT and assigns eliminated variables whose
  defining clause is otherwise falsified

This must integrate with the current SAT output path before any elimination is enabled.

Repo-specific consequence:

- the current solver prints `solver.assignment` directly after `solve_to_output()`
- once eliminated variables exist, SAT output should no longer depend on the live trail state alone
- prefer taking an explicit model snapshot at SAT, extending that snapshot, and printing from it
  rather than mutating search bookkeeping purely for output formatting

Capture-timing requirement:

- the SAT snapshot must be taken at the moment the solver has found a complete satisfying
  assignment, before any future cleanup/backtrack path can erase non-root assignments
- once the snapshot path exists, stdout formatting and SAT-side tests should consume only that
  snapshot, not the mutable live assignment vector
- printing should assert that the post-extension snapshot contains no `UNASSIGNED` values for any
  original variable, because the current fallback of treating non-`FALSE` as positive would silently
  emit bogus SAT assignments if extension/model capture is incomplete

### G. Make preprocessing a distinct top-level phase

Mirror MiniSat's operational flow:

1. parse original clauses
2. enqueue/propagate root units
3. call `eliminate(true)` once before CDCL search
4. if preprocessing finds UNSAT, stop immediately
5. otherwise run the normal CDCL search on the simplified formula
6. if SAT, run `extend_model()` before printing the assignment

This is better than trying to interleave full BVE inside the current solve loop.

Repo-specific transition note:

- today `Solver::new()` bulk-loads clauses, stores unit originals in `root_unit_clauses`, and
  `solve_with_proof()` calls `enqueue_root_units()` once before search
- once original-clause insertion becomes canonical and unit insertion propagates immediately, this
  bootstrap path needs to be revisited so units are not effectively handled in two different ways
- either keep `root_unit_clauses` as a pure parse/bootstrap mechanism with well-defined ownership,
  or retire it in favor of direct root enqueue during original-clause insertion

### H. Keep root reason bookkeeping coherent during preprocessing

MiniSat's core `removeClause()` clears the reason of a locked clause before freeing it. That matters
for this port because preprocessing operates at decision level `0`, where original clauses can be
the active reasons for root assignments.

In the current Rust solver:

- root propagation stores clause ids in `reason[var]`
- `simplify()` already clears a locked reason before deleting a satisfied clause
- strengthening can turn a binary or longer original clause into a unit and thereby change the root
  reason source for an assignment

The port should therefore treat root-reason maintenance as a first-class requirement:

- deleting a locked original clause must clear or replace the affected `reason[var]`
- strengthening that preserves implication must leave the surviving clause/reason relation valid
- turning a clause into a unit must not leave stale reason refs to a deleted clause id

Without this, preprocessing can silently corrupt later conflict analysis even if propagation itself
still appears to work.

## Core Algorithms To Port

### 1. `implied(clause)`

Purpose:

- optional expensive redundancy check before inserting a clause

Rust design:

- require decision level `0`
- enqueue negations of all non-false literals in scratch mode
- if any literal is already true, clause is implied immediately
- run propagation
- backtrack to root without mutating the permanent root assignment

Implementation note:

- the current solver lacks a temporary "push root assumptions and cancel" helper
- add one reusable helper for temporary root-probe propagation

State-restoration note:

- the helper must restore not just `assignment`, but also `trail`, `propagate_head`,
  `root_trail_len`, per-variable `reason`, and any branch-heap effects introduced by temporary
  assignments
- it must not perturb `simplify_assigns` / `simplify_props_remaining`, which describe committed
  root-state progress rather than scratch probing work
- it must also respect decision eligibility when unwinding temporary assignments so cancelled probes
  do not reinsert eliminated or otherwise non-branchable variables into the heap

### 2. `gather_touched_clauses()`

Purpose:

- move clauses from touched variables into the subsumption queue

Rust design:

- for each touched variable, clean its occurrence list
- enqueue each live clause once using a `queued_for_subsumption` bit/vector
- clear the touched bit and reset `n_touched`

### 3. `backward_subsumption_check()`

Purpose:

- delete subsumed clauses
- perform backward subsumption resolution by strengthening one literal away

Rust design:

- while queue non-empty or `bwdsub_assigns < root_trail_len`
- when queue empty but new root assignment exists, synthesize a size-1 scratch clause
- choose the smallest occurrence variable from the driver clause
- scan that variable's occurrence list
- skip candidate clauses whose size is at or above `subsumption_lim` when that limit is enabled,
  matching MiniSat's guard against expensive large-clause scans
- classify candidate relation:
  - `subsumed`
  - `strengthen by removing lit`
  - `no action`
- after strengthening/deleting, keep occurrence metadata and queue state consistent

Key implementation constraint:

- the current solver has no clause abstraction bitsets like MiniSat's `Clause::subsumes()` path
- first faithful version should still add a clause-abstraction cache or temporary bitset check,
  otherwise subsumption scans will be too slow and too unlike MiniSat's behavior

Storage constraint:

- MiniSat gets clause abstractions from its temporary `extra_clause_field` support during
  simplification
- the current Rust arena already uses per-clause extra words for learned-clause activity and gives
  original clauses no extra word at all
- the port therefore needs an explicit abstraction-storage decision for original clauses during
  preprocessing, such as:
  - temporary sidecar storage keyed by clause id
  - optional extra-word support for original clauses while simplification is enabled
  - or recomputation on demand if the measured cost is acceptable

This should be decided before implementing backward subsumption so the clause layout does not churn
mid-port.

### 4. `strengthen_clause()`

Purpose:

- remove one literal from a live original clause at root

Rust design:

- if binary, delete clause and rewrite it as a unit in place of the removed literal
- otherwise:
  - detach watchers
  - remove the literal
  - reattach watchers
  - update occurrence counts
  - update touched bits / queue state
- if clause becomes unit, enqueue it and propagate immediately

Important mismatch to resolve:

- current clause trimming assumes watched literals remain in positions `0` and `1`
- general strengthening can delete any literal, including watched ones
- add a generic "remove literal from clause and restore watched invariant" helper rather than
  reusing `trim_root_false_literals()`

Binary-to-unit design note:

- MiniSat's `strengthenClause()` can conceptually turn a binary clause into a unit while reusing the
  underlying clause storage, but the current Rust arena/update model may make that awkward
- the Rust port should choose one explicit strategy:
  - preserve the same clause id as a live unit representation, or
  - delete the binary clause and create/update a separate unit representation
- whichever strategy is chosen, it must also update:
  - root-unit bookkeeping
  - root reasons
  - occurrence membership / dirty bits
  - queued-for-subsumption state for the old and new representations

### 5. `asymm()` / `asymm_var()`

Purpose:

- strengthen clauses using asymmetric branching before elimination

Rust design:

- iterate the occurrence list of a variable
- for each clause, temporarily assign negations of all other non-false literals
- if propagation conflicts, strengthen away the variable's literal in that clause
- after finishing the variable, rerun backward subsumption

### 6. `merge()`

Purpose:

- compute resolvent size quickly for the elimination bound
- compute the actual resolvent when elimination proceeds

Rust design:

- implement both forms:
  - `merge_size_only(pos_clause, neg_clause, var) -> tautological? + size`
  - `merge_into_scratch(...) -> tautological?`
- keep the same duplicate/tautology semantics as MiniSat:
  - duplicate literals collapse
  - complementary literals make the resolvent tautological and therefore skipped

### 7. `eliminate_var()`

Purpose:

- bounded variable elimination

Rust design:

1. clean the variable's occurrence list
2. split into `pos` and `neg`
3. estimate whether elimination is allowed:
   - count only non-tautological resolvents
   - reject if `count > occurs(v).len() + grow`
   - reject if any resolvent exceeds `clause_lim` when set
4. if allowed:
   - mark variable eliminated
   - disable it as a decision variable / branch candidate
   - push extension clauses into `elim_clauses`
   - delete every clause containing the variable
   - add all non-tautological resolvents as original clauses
   - clear `occurs[v]`
   - optionally clear watcher vectors for `v` if empty
   - run backward subsumption again

### 8. `extend_model()`

Purpose:

- restore assignments for eliminated variables after SAT

Rust design:

- walk `elim_clauses` backward by stored clause lengths
- if every non-head literal in an extension clause is false in the model, assign the head literal
  true

Output integration:

- call before `print_assignment()`
- ensure eliminated variables appear with concrete truth values in stdout

### 9. `eliminate(turn_off_elim)`

Purpose:

- entire preprocessing phase

Rust design:

1. call root `simplify()`
2. if simplification is already disabled, return
3. loop while any of:
   - touched work remains
   - new root assignments remain for backward subsumption
   - elimination heap non-empty
4. inside the loop:
   - gather touched clauses
   - run backward subsumption if needed
   - pop candidate variables from the elimination heap
   - skip eliminated/assigned variables
   - optionally run asymmetric branching
   - optionally run variable elimination
   - run simplification GC threshold checks
5. cleanup:
   - if `turn_off_elim`, drop preprocessing-only structures, disable extra clause metadata if
     possible, rebuild branch heap, and force full GC
   - else keep preprocessing state alive and only do cheap GC

## Integration Details For This Repo

### Proof logging

The current solver writes DRAT proof additions for learned clauses only. MiniSat `simp` itself does
not emit DRAT in this old codebase.

For this repo, decide explicitly before implementation whether preprocessing additions/deletions
must also be logged for proof correctness. A faithful semantic port of MiniSat simplification is
not automatically a faithful DRAT-producing port.

Recommended approach:

- treat proof logging as a separate acceptance gate
- first get the simplification/search semantics correct behind tests
- then add DRAT support for preprocessing transformations if required by the repo's checker flow

This is the single biggest place where "faithful to MiniSat simp" and "faithful to repo proof
requirements" may diverge.

Specific repo risk to remember:

- if preprocessing derives UNSAT before the CDCL loop, the current solver would still need to emit
  a proof acceptable to this repo's checker flow
- that case is operationally different from "CDCL learned the empty clause", so it deserves its
  own acceptance test once preprocessing proof support exists

Queue/dedup bookkeeping risk to remember:

- MiniSat uses clause mark bits while gathering touched clauses to avoid duplicate work in the
  subsumption queue
- the Rust plan currently prefers explicit queue-dedup state keyed by clause id
- if a clause is deleted, relocated, or replaced during strengthening, that dedup state must be
  cleared or migrated in lockstep
- otherwise the solver can end up permanently skipping needed backward-subsumption work or
  resurrecting stale queue entries

### Clause normalization and solver-status handling

MiniSat's preprocessing entry points rely on core `addClause_()` semantics, not on raw clause
storage. For this repo that means the implementation should decide up front how to represent the
equivalent of MiniSat's persistent `ok` state:

- empty clause insertion must poison the solver immediately
- unit clauses created during parse, strengthening, or resolvent insertion must enqueue and
  propagate at decision level `0`
- tautological or already-satisfied clauses must not enter occurrence lists, watcher state, or the
  subsumption queue

If this is not made explicit in the Rust API, later preprocessing phases will inherit subtle
differences between "clauses from the parser" and "clauses created during simplification".

### Watchers and root units

Current solver 10 stores unit clauses in the watcher structure and tracks original root units in
`root_unit_clauses`. MiniSat's `addClause_()` does not allocate persistent clause objects for unit
clauses at all; it only enqueues them at decision level `0`. The Rust port therefore needs one
explicit representation decision early:

- either keep repo-specific persistent unit-clause refs for bootstrap/proof purposes and treat them
  as an intentional divergence from MiniSat's internal storage model
- or move units to pure root-assignment semantics and ensure no preprocessing structure expects a
  clause id for them

If the first option is chosen, strengthening/elimination must keep the unit-clause bookkeeping valid
across:

- clause deletion
- garbage collection
- unit creation by strengthening
- root-reason rewrites when the backing clause for an implied root unit changes or disappears

### Garbage collection and relocation

The current Rust arena already supports relocating clause references during GC. The preprocessing
port needs to extend that relocation surface to every simplification-owned reference container:

- occurrence lists
- subsumption queue
- any queued/dedup markers keyed by clause id
- the scratch unit representation, if it becomes a real arena clause instead of a pure scratch
  object

MiniSat handles this explicitly in `SimpSolver::relocAll()`. The Rust plan should do the same
rather than assuming the existing learned/original clause-id relocation logic is sufficient.

Allocator-layout cleanup note:

- if simplification temporarily enables extra per-clause metadata for original clauses, the
  `turn_off_elim` cleanup must ensure the post-preprocessing arena layout no longer depends on it
- forcing a full GC after disabling simplification should be treated as the mechanism that
  canonicalizes the final clause layout back to the lean search-only form

### Branching heap

MiniSat disables eliminated variables as decision variables. In the current Rust solver that should
mean:

- mark them non-branchable permanently
- ensure they are removed or skipped in the branch heap
- do not reinsert them on backtrack

This likely requires a new `decision_var: Vec<bool>` or equivalent flag rather than relying only on
assignment state.

Current-Rust-specific consequence:

- the current `backtrack(0)` / `push_branch_var()` path reinserts every unassigned variable
- `rebuild_branch_queue()` also currently repopulates from all unassigned variables
- the port must gate both paths on decision eligibility, otherwise eliminated/frozen-out variables
  will silently re-enter the branch heap after backtrack, restart, GC rebuild, or temporary probe
  cancellation

### Assumptions

MiniSat freezes assumptions before elimination. The current CLI solver does not expose assumptions
yet, but the design should still leave room for them:

- add `freeze_var()` / `thaw()` helpers now
- even if assumptions are not used yet, this avoids painting the solver into a corner

### Variable lifecycle and reuse

MiniSat's core solver has `newVar()`, `releaseVar()`, `free_vars`, and decision-variable toggles as
part of the same lifecycle. The current Rust solver allocates all vectors up front from the parsed
header and never reuses variables.

For solver 10, the important point is not to fully port variable recycling on day one, but to keep
the preprocessing design compatible with it:

- decision eligibility should be tracked separately from assignment state
- eliminated variables must be permanently non-branchable
- any future support for assumptions or released variables should not require rewriting the
  preprocessing data-structure layout

## Recommended Implementation Order

The order below is chosen to preserve correctness and keep regressions local.

### Phase 0: metadata and baseline cleanup

1. Re-audit solver-10 metadata and docs so the plan matches the current tree exactly.
2. Keep current solver behavior unchanged.
3. Add tests that pin the current root simplify behavior and current SAT output shape.

Exit criteria:

- `cargo test`
- `bash tools/smoke_test.sh solver/10-bve-preprocess`

### Phase 1: preprocessing state scaffolding

1. Add preprocessing configuration fields and vectors to `Solver`.
2. Add a decision-variable flag separate from assignment state.
3. Split original-clause insertion from learned-clause insertion.
4. Build occurrence lists and literal counts during initial parse.
5. Add lazy-clean helpers for occurrence lists and queue-dedup state.
6. Make initial parse/build use the same original-clause canonicalization path intended for
   preprocessing insertions.
7. Decide where clause abstractions for original clauses live during simplification.

Tests first:

- occurrence lists built correctly from parsed clauses
- literal counts update on insertion
- deleted original clauses disappear after `clean_occurs(var)`
- duplicate literals are removed and tautological original clauses are skipped before indexing
- unit original clauses propagate immediately through the canonical insertion path
- non-unit original clauses are the only clauses that enter occurrence/subsumption indexing when
  following MiniSat semantics
- preprocessing insertion rejects clauses that still contain eliminated variables
- original-clause insertion reports whether it allocated a clause, produced a unit, became a no-op,
  or detected UNSAT
- preprocessing-time UNSAT poisons the persistent solver-consistency flag, not just the immediate
  caller's return path
- SAT output uses a model snapshot path that can tolerate eliminated variables later

### Phase 2: preprocessing-aware deletion and strengthening primitives

1. Implement original-clause-aware `remove_clause_preprocess()`.
2. Implement generic clause literal removal / strengthening with watcher repair.
3. Update root `simplify()` to route through preprocessing-aware deletion when
   `use_simplification` is enabled.

Tests first:

- deleting an original clause updates occurrence counts and touched state
- strengthening a non-binary clause preserves watcher correctness
- strengthening a binary clause yields a unit and propagates it
- deleting or strengthening a locked root reason leaves `reason[var]` pointing at a valid live
  clause or `NO_REASON` as appropriate
- strengthening that replaces a clause representation clears or remaps any queued/dedup state tied
  to the old clause id

### Phase 3: touched-clause and backward-subsumption pipeline

1. Implement `gather_touched_clauses()`.
2. Implement scratch-clause handling for new root assignments.
3. Implement backward subsumption and backward subsumption resolution.
4. Optionally add clause abstraction/cache if needed for performance parity.

Tests first:

- clause A subsumes clause B and B is removed
- clause A backward-subsumes clause B up to one literal and B is strengthened
- new root assignments feed the subsumption queue through the scratch unit path
- `subsumption_lim` prevents scans/strengthening against oversized candidate clauses

### Phase 4: implied-clause checking and optional asymmetric branching

1. Add temporary root-probe helper for implication checks.
2. Implement `implied()`.
3. Implement `asymm()` and `asymm_var()`.

Tests first:

- implied clause is skipped when `use_rcheck` is enabled
- asymmetric branching removes the target literal on a crafted instance
- temporary root probes restore `trail`, `propagate_head`, `root_trail_len`, and root reasons

### Phase 5: elimination heap and bounded variable elimination

1. Implement elimination-cost heap keyed by `n_occ[pos] * n_occ[neg]`.
2. Implement `merge_size_only()` and `merge_into_scratch()`.
3. Implement `eliminate_var()` with:
   - grow cap
   - clause-length cap
   - extension stack writes
   - resolvent insertion
   - occurrence cleanup
4. Re-run backward subsumption after each elimination.

Tests first:

- variable not eliminated when resolvent count exceeds `occurs(v) + grow`
- variable not eliminated when a resolvent exceeds `clause_lim`
- successful elimination deletes old clauses and inserts expected resolvents
- eliminated variable never appears in the branch heap
- eliminated or otherwise non-decision variables are not reinserted by backtrack/restart/heap rebuild

### Phase 6: model extension

1. Implement `mk_elim_clause_*` helpers.
2. Implement `extend_model()`.
3. Call it on SAT after preprocessing.

Tests first:

- SAT model assigns eliminated variables consistently with the extension stack
- printed assignment still satisfies the original CNF
- the final printed SAT snapshot contains no unassigned original variables

### Phase 7: one-shot preprocessing entry point

1. Add `eliminate(turn_off_elim)` around the current search entry point.
2. Run `eliminate(true)` before the CDCL loop, matching MiniSat's operational model.
3. Implement cleanup:
   - drop occurrence/subsumption structures
   - disable preprocessing-only paths
   - rebuild branch heap
   - force full GC
4. Make the root-unit bootstrap consistent with the new original-clause insertion semantics.

Tests first:

- preprocessing-only structures are empty/disabled after `eliminate(true)`
- search still works after preprocessing cleanup
- a formula solved during preprocessing returns UNSAT before entering CDCL
- unit clauses are not double-enqueued across preprocessing startup

### Phase 8: proof and benchmark hardening

1. Decide and implement proof logging policy for preprocessing transforms.
2. Re-run unit tests, smoke tests, and targeted benchmarks.
3. Compare against MiniSat `-pre`/`-no-pre` behavior on a small regression set.
4. Verify that preprocessing-owned metadata survives arena relocation across forced GC.
5. Add an acceptance check for proofs produced when preprocessing alone detects UNSAT.

Tests and checks:

- `cargo test`
- `bash tools/smoke_test.sh solver/10-bve-preprocess`
- proof checker run on UNSAT smoke tests
- benchmark spot-checks against MiniSat on targeted instances

## Minimum Test Matrix To Add

Add targeted unit tests for:

- occurrence-list construction and lazy cleanup
- original-clause canonicalization parity with MiniSat-style `addClause_()`
- clause-abstraction storage / lookup correctness for backward-subsumption candidates
- touched-clause queue dedup
- queue-dedup state stays correct when clauses are deleted, relocated, or replaced by strengthening
- backward subsumption delete case
- backward subsumption strengthen case
- backward subsumption respects `subsumption_lim`
- asymmetric branching strengthen case
- elimination rejection by `grow`
- elimination rejection by `clause_lim`
- successful elimination with expected resolvents
- eliminated variable removed from branching
- backtrack / restart / temporary-probe cancellation do not reinsert eliminated variables into the
  branch heap
- model extension after SAT
- SAT output printing from the extended model snapshot
- SAT output rejects or asserts on any remaining unassigned original variable after extension
- preprocessing cleanup after `eliminate(true)`
- occurrence/subsumption metadata relocation across GC
- preprocessing-detected UNSAT proof path
- persistent solver-consistency flag stays poisoned across later preprocessing/search entry points

Keep the existing smoke suite as the final guardrail.

## Main Risks

### 1. Proof correctness risk

Preprocessing transformations change the formula before CDCL. If DRAT logging is not updated,
UNSAT proofs may stop checking even if search is logically correct.

### 2. Watcher corruption risk

General clause strengthening is more invasive than current root trimming. Any bug here will produce
silent propagation errors.

### 3. Model-extension risk

Elimination without extension will produce incomplete SAT assignments.

### 4. Performance risk

Naive eager occurrence maintenance or naive subsumption scans can erase any benefit from BVE.

### 5. Scope creep risk

Trying to implement all of BVE, subsumption, asymm, proof logging, and metadata cleanup in one
patch is likely to fail. The phases above should be kept separate.

## Recommended First Coding Slice

If implementation starts immediately, the highest-value first patch is:

1. add baseline tests for current simplify / SAT-output behavior
2. add preprocessing state fields
3. split original vs learned clause insertion
4. build occurrence lists and literal counts at parse time
5. add unit tests for occurrence maintenance only

Reason:

- every later `simp` feature depends on having correct original-clause indexing
- it is the lowest-risk slice that materially advances the faithful port
- it does not yet force decisions about proof logging
