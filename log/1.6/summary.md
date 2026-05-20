# Solver 11 Phase 1.6 - Binary implication fast path

Date: 2026-05-20

Bead: `SAT-playground-5b2.2.9` (`[1.6] Binary implication fast path`)

## Scope

This slice adds an opt-in binary implication propagation path behind
`SAT_BINARY_FAST=on`. The default solver path remains the legacy watched-clause
path for solver-10 parity.

The implementation keeps binary clauses in the arena so proof logging, model
checking, clause deletion, garbage collection, and debugging can still refer to
the canonical clause representation. The fast path adds stable `BinaryClauseId`
metadata and adjacency edges keyed by the assigned-true antecedent literal.

## Implementation Notes

- Added stable binary metadata (`BinaryClause`, `BinaryClauseId`, `BinaryOrigin`)
  and binary implication edges (`BinaryEdge`).
- Added a `BinaryImplications` abstraction with the current nested adjacency and
  a flat/offset representation scaffold for later compression work.
- Registered original and learned binary clauses when `SAT_BINARY_FAST=on`; long
  watcher propagation remains used when the flag is off.
- Added binary reason support in propagation, conflict analysis, reason pinning,
  clause locking, clause deletion, and GC remapping.
- Kept original binary clauses in the arena for traceability rather than moving
  them into a separate proof/model-only store.
- Added binary propagation/stale-edge stats (`binary_props`,
  `binary_stale_skips`) for opt-in hot-stat diagnostics.
- Updated feature registry/docs to mark `SAT_BINARY_FAST` as SmokeSafe but not
  promoted to any default profile.

## Fresh-Eyes Review Findings

- Fixed `BinaryImplications::Flat::add_edge`; the first draft could silently
  lose appended edges in flat mode. It now inserts at the correct slice boundary,
  updates following offsets, and marks the flat view dirty.
- Fixed binary conflict analysis to skip the resolved variable by identity
  rather than assuming the implied literal is stored at a fixed position.
- Added binary-conflict level normalization so a conflict discovered from stale
  propagation state is analyzed at the actual maximum decision level involved.
- Found that current clause minimization is still not proof-safe with arbitrary
  binary reason literal order on the search-core `case9` instance. The Phase 1.6
  fix disables clause minimization while `SAT_BINARY_FAST=on`; Phase 1.11 owns
  the fully binary-reason-aware minimization work.

## Validation

Commands run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test -- --nocapture
bash tools/smoke_test.sh solver/11-kissat-port
SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port
SAT_BINARY_FAST=on bash tools/smoke_test.sh solver/11-kissat-port
SAT_BINARY_FAST=on SAT_USE_LBD=on SAT_REDUCE=lbd-tiered SAT_RESTART=kissat-ema SAT_SEARCH_MODE=focused-stable SAT_PHASE=saved bash tools/smoke_test.sh solver/11-kissat-port
SAT_BINARY_FAST=on bash solver/11-kissat-port/run.sh /tmp/s11-case9.cnf /tmp/s11-case9-proof-20260520-1443
SAT_BINARY_FAST=on SAT_STATS_JSON=on SAT_STATS_HOT=on bash tools/bench.sh -t 120 -m 16384 -d benchmarks/iteration/search-core --log-dir log/1.6/search-core-binary-fast-ccmin-off solver/11-kissat-port
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling --log-dir log/1.6/profile-after-final solver/11-kissat-port
python3 tools/compare_bench.py --before log/1.9/profile-after/results.csv --after log/1.6/profile-after-final/results.csv --timeout 120
git diff --check
```

Results:

- Unit tests: 215 passed.
- Default smoke: 9/9 passed.
- Invariant smoke: 9/9 passed.
- Binary fast smoke: 9/9 passed.
- Combined Phase 1 smoke with binary fast: 9/9 passed.
- Focused `case9` binary-fast rerun returned SAT with a model, avoiding the
  prior invalid UNSAT/proof path.
- Search-core binary-fast gate:
  - log: `log/1.6/search-core-binary-fast-ccmin-off/results.csv`
  - 9 instances, 3 solved (3 SAT), 6 unsolved (5 timeout + 1 unknown), no
    verification failures, PAR-2 1556.657.
- Default profile bench:
  - log: `log/1.6/profile-after-final/results.csv`
  - 11 instances, 9 solved (6 SAT + 3 UNSAT), 2 timeouts, PAR-2 711.619.
  - Compare against `log/1.9/profile-after/results.csv`: verdict PASS, no
    status regressions, same solved count, PAR-2 delta +3.619 seconds
    (~0.51% of PAR-2), median paired speedup 0.9864. This is treated as
    no regression for a default-off feature on a single-run profile gate.

## Remaining Work

`SAT_BINARY_FAST=on` is correctness-safe for the tested gates but not promoted
as a performance default. The next related work should restore clause
minimization under binary reasons in Phase 1.11 and then reevaluate binary-fast
performance with the same search-core/profile gates.
