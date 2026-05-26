# AnalyzeSAT 2026-05-26: preprocessing/order/proof fresh-eyes pass

Branch: `side-01-analyzesat-20260526-111233`
Solver: `solver/11-kissat-port`
Base revision: `885462f85aff3dceceb8f0468b1ec54cebd43100`
Suite: `benchmarks/profiling/`, 10 instances, 300s outer timeout, `SAT_LIMIT_WALL_SEC=295`, `SAT_STATS_JSON=on`.

This run deliberately avoided the already-covered lucky/EMA/restart/clause-min/phase areas and looked at initial clause handling, preprocessing toggles, BSR, and proof logging.

## Artifacts

- Matrix runner: `log/analyzesat-2026-05-26-preprocess-proof/run_preprocess_ablation.sh`
- Raw per-row logs: `log/analyzesat-2026-05-26-preprocess-proof/runs/`
- Matrix CSV: `log/analyzesat-2026-05-26-preprocess-proof/matrix_results.csv`
- Aggregated JSON stats: `log/analyzesat-2026-05-26-preprocess-proof/all_stats.jsonl`
- Analysis script/output: `analysis.py`, `analysis.md`
- Kakuro conflict-aligned traces: `traces/default.stderr`, `traces/input_order.stderr`
- Perf attempt: `perf-stat-attempt.stderr` (blocked by `perf_event_paranoid=4`)

## Result summary

| config | rows | solved | PAR-2 on measured rows | decision |
|---|---:|---:|---:|---|
| `default` | 10 | 10 | 869.765 | baseline |
| `SAT_BVE=off` | 1 | 0 | 600.000 | rejected: Sudoku `UNSAT -> UNKNOWN` |
| `SAT_FULL_BSR=off` | 3 | 2 | 794.416 | rejected: K4 `UNSAT -> UNKNOWN` |
| `SAT_SIMPLIFICATION=off` | 1 | 0 | 600.000 | rejected: Sudoku `UNSAT -> UNKNOWN` |
| `SAT_INITIAL_CLAUSE_MODE=input-order` | 10 | 10 | 717.513 | status-safe on this suite, +152.252 PAR-2 win |
| `SAT_INITIAL_CLAUSE_MODE=raw` | 10 | 10 | 687.588 | status-safe on this suite, +182.177 PAR-2 win |
| `SAT_PROOF=off` | 10 | 10 | 729.089 | diagnostic only: violates UNSAT proof contract |

## Main findings

1. `SAT_INITIAL_CLAUSE_MODE=raw` is a 182.177s PAR-2 win on the full profiling suite with no status regressions in this run. `input-order` is also a 152.252s win. Both are driven by Kakuro: default solves in 255.761s, input-order in 50.008s, raw in 50.714s.

2. The Kakuro win is search-path work, not preprocessing throughput. The trace reports identical preprocessing headline counters for default and input-order: `eliminated=56214`, `resolvents=210762`, `subsumed=4868640`, `original_clauses=14742137`, `original_literals=52891600`. The search then diverges: default needs 732107 conflicts / 3188069 decisions / 617655456 propagations, while input-order needs 37074 conflicts / 329361 decisions / 34528910 propagations.

3. Raw/input order is not a universal promotion. Velev regresses: default 77.545s, input-order 109.516s, raw 122.182s. K4 also regresses by about 11-14s. This needs a guarded policy or classifier, not a blind default flip.

4. Disabling BVE or full simplification immediately violates the current baseline-solved rule: Sudoku is default `UNSAT` in 236.575s but becomes `UNKNOWN` at the internal limit under both `SAT_BVE=off` and `SAT_SIMPLIFICATION=off`.

5. Disabling full BSR is interesting but unsafe. It improves Sudoku (236.575s -> 181.211s) and Iter30 (17.477s -> 13.205s), then fails K4 (`UNSAT` 60.990s -> `UNKNOWN` 296.78s). The K4 regression is 5.0x more conflicts and 1.22x slower propagation throughput before timing out.

6. Proof logging has a pure execution-side cost. `SAT_PROOF=off` leaves conflicts, decisions, and propagations identical to default on all 10 rows, but PAR-2 drops by 140.676s. This is diagnostic only because UNSAT rows have no proof, but it points at proof logging/encoding overhead in the hot path, not search trajectory.

## Source-level notes

Solver 11 routes initial clauses through three modes in `src/main.rs:2016`: `CanonicalSorted`, `CanonicalInputOrder`, and `Raw`. The sorted path uses `normalize_original_clause` in `src/simp.rs:469`, which sorts literals by variable and sign before deduping. The input-order path uses `normalize_original_clause_input_order` in `src/simp.rs:511`, which dedups/tautology-checks while preserving literal order. Raw mode skips normalization and directly attaches clauses in `src/main.rs:2034`.

MiniSat's `Solver::addClause_` sorts literals before attach, but that reference behavior is local clause normalization, not evidence that every benchmark benefits from canonical watch order. The official source sorts `ps` at `Solver.cc:2468`, then allocates/attaches the clause at `Solver.cc:2495-2499`: https://github.com/niklasso/minisat/blob/master/minisat/core/Solver.cc#L2458-L2499

In this solver, canonical sorting changes watched literals and later CDCL trajectory enough to dominate preprocessing on Kakuro. The traces show both modes reach the same preprocessing counts, then default continues for 732k conflicts while input-order exits at 37k conflicts.

## Recommendations

1. Add a guarded `SAT_INITIAL_CLAUSE_MODE` policy instead of flipping the default. The first target should classify cases where raw/input-order is likely safe: Kakuro/Sudoku-like large structured formulas benefit, while Velev-like formulas regress.

2. Keep BVE and full simplification enabled for default/fast profiles. Their disabled modes produced baseline-solved UNKNOWN rows and should remain diagnostic only.

3. Reframe the BSR task as selected/conditional BSR rather than global off. Global off is unsafe, but the early Sudoku/Iter30 wins show there is avoidable BSR-induced search-path damage.

4. Extend the proof logging task to target hot-path proof record/write overhead. `SAT_PROOF=off` is not promotable, but identical work counters across every row make the overhead measurable and actionable.
