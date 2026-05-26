# 11-kissat-port

Solver 11 starts as a current copy of `10-bve-preprocess`, including the MiniSat `simp`-style bounded variable elimination baseline, so new Kissat-class work can proceed without losing solver 10 fixes.

This iteration keeps the `09` CDCL core and adds a one-shot preprocessing phase before search. The
preprocessor is intentionally isolated in `src/simp.rs` so it can keep evolving toward the full
MiniSat `SimpSolver` design described in
[MINISAT_SIMP_PORT.md](/home/bojji/code/SAT-playground/solver/11-kissat-port/MINISAT_SIMP_PORT.md).

## Current State

What is present:

- original-clause occurrence lists and literal occurrence counts during preprocessing
- a separate decision-variable flag so eliminated variables do not re-enter the branch heap
- bounded variable elimination with MiniSat-style `grow = 0` and `clause_lim = 20`, enabled by
  default and disabled with `SAT_BVE=off`
- MiniSat-style backward subsumption / BSR, enabled by default and disabled with
  `SAT_FULL_BSR=off`
- preprocessing can be bypassed for comparison runs with `SAT_SIMPLIFICATION=off`
- 64-bit clause abstraction prefiltering for preprocessing subsumption checks
- large-formula preprocessing stores original-clause abstractions inline to avoid sparse arena-indexed
  side-table loads in hot BSR scans
- sorted-clause subsumption relation for long clauses on large canonical formulas
- in-place original-clause strengthening during BSR
- one-pass strengthened-clause compaction/proof logging metadata updates
- lazy occurrence-list membership cleanup for large formulas after clause strengthening
- lazy preprocessing watcher detach on small formulas and the large inline-preprocessing path
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
- opt-in LBD metadata and LBD-tiered reduction state. Newly learned clauses initialize their
  `used_recently` counter to the solver's maximum for every LBD tier, matching Kissat's
  maximum initial learned-clause retention semantics before reduce-DB aging decides eviction.
  `SAT_LBD_UPDATE_REASONS=on` keeps reason-side LBD improvement scoped to conflict analysis.
  Learned-reason LBD recomputation now walks arena clauses directly instead of allocating a
  temporary literal vector per reason. LBD-tiered reduce-DB also reuses a persistent delete-marker
  table and compacts `learned_clause_ids` in place instead of allocating a dense delete vector for
  every reduction pass.
  `SAT_LBD_UPDATE_PROP_REASONS=on` is a separate experimental extension that recomputes learned
  propagation-reason LBD after the implied literal is enqueued, lowers the stored LBD/tier if the
  current assignment gives a better glue value, and marks learned propagation reasons recently used
  in lbd-tiered runs. The propagation-time extension remains isolated after profile testing showed
  that enabling it in the lbd-tiered feature mode regressed the current profiling suite.
  The LBD-tiered reducer now computes focused-mode and stable-mode tier thresholds from recent
  glue-use histograms at the start of each LBD reduction pass: tier 1 covers the first 50% of
  recently used learned-clause glue counts, tier 2 covers the first 90%, and both thresholds keep
  the original `2/6` constants as minimum floors. The same pass reclassifies live learned clauses
  under the current mode's thresholds and ages `used_recently` for every scanned kept learned
  clause, so clauses are protected for the current pass but do not stay protected indefinitely.
  `SAT_REDUCE=lbd-tiered` uses a Kissat-style conflict-count reduction schedule: the first
  reduction is scheduled at `SAT_REDUCE_DB_INIT` or 1,000 conflicts, later reductions are scheduled
  at the current conflict count plus `sqrt(reduce_db_calls) * SAT_REDUCE_DB_INTERVAL`, and the hard
  learned-literal budget is retained only as an emergency trigger. A minimum conflict interval
  guard prevents repeated high-LBD emergency reductions from firing more often than
  `SAT_REDUCE_MIN_INTERVAL` conflicts; lbd-tiered mode defaults this guard to `100`.
- an LBD EMA restart policy (`SAT_USE_LBD=on SAT_RESTART=kissat-ema`) for single-mode Phase 1
  search experiments. An optional Glucose-style decision-level blocker can suppress EMA restarts
  when the recent decision-level EMA is high relative to the slow baseline; set
  `SAT_RESTART_BLOCK_MARGIN` above `0` to enable it. The blocker is default-off after profile
  testing showed the `1.4` margin regressed the current profiling suite. Restart trail reuse is
  available for the focused/stable search path; stable-mode reuse is deliberately not applied to
  the solver-10-compatible single-mode Luby path because that hybrid preserved a harmful SAT prefix
  on `mp1`.
  Single-mode search still defaults to legacy Luby restarts for solver-10 parity. Built-in profiles intentionally do not bundle
  `SAT_RESTART=kissat-ema` with target-phase policies; `SAT_PHASE=target-then-saved` remains an
  explicit opt-in when EMA restarts are active because HWMCC-style instances have regressed under
  that combination.
- opt-in saved/target/best phase selection policies via `SAT_PHASE`, with legacy saved-phase
  branching kept as the default for solver-10 parity. In focused/stable search, target/best phase
  snapshots are captured only while stable mode is active, matching Kissat's update boundary; target
  phases persist across mode switches and restart cycles, then reset when a rephase event starts a
  new phase block. Single-mode target policies retain restart-time target reset behavior after the
  all-mode persistence experiment regressed the profiling suite.
- focused/stable search-mode scaffolding with focused EMA restarts and stable reluctant restarts.
  Env-facing `SAT_SEARCH_MODE=focused-stable` now enters the actual focused/stable path and defaults
  to focused-only VMTF unless `SAT_VMTF=off` is explicitly requested for an ablation. This matches
  Kissat's focused-mode branching model instead of the rejected VSIDS-in-focused hybrid.
  The `default` and `fast` profiles use the solver-10-compatible single-mode search path after the
  focused/stable default candidate regressed the clean solver 10 profiling baseline.
  Entering focused mode resets the LBD EMA restart averages so focused-mode restart calibration
  does not inherit stable-mode glue. Entering stable mode refreshes the VSIDS heap from current
  variable activities before stable-mode decisions resume. Focused EMA restarts now use Kissat's
  soft throttle: after each focused restart, the minimum EMA interval becomes
  `50 + kissat_logn(focused_restarts) - 1`.
  Stable reluctant restarts now use Kissat's `reluctantint=1024` scale instead of raw Luby counts,
  which fixed the focused/stable case9 wall-clock UNKNOWN caused by hundreds of thousands of stable
  restarts. Focused mode also has Kissat-style random decision sequences and the focused phase cycle
  that periodically forces initial and inverted-initial phases by mode-switch count.
- Kissat-style mode scheduling is available for focused/stable search. The implementation keeps
  focused-mode switches conflict-gated with
  `nlogpown(count, 4)` interval growth, but gates stable-mode duration on propagation search ticks.
  Tick mode also resets all restart EMAs on every mode switch.
- Variable-Move-To-Front branching. Focused mode uses the VMTF queue and move-to-front conflict
  bumps without updating VSIDS scores, while stable mode uses the VSIDS heap and score bumps.
  Conflict-bumped variables are now moved in existing queue-stamp order, matching Kissat's
  `sort_bump` behavior; the old scratch-order movement could invert the focused queue and produced
  `UNKNOWN` on Sudoku. `SAT_VMTF=single` remains a default-off experimental fallback for the
  solver-10-compatible single-mode path; it is bounded by fixed budgets and should not be treated as
  the promoted VMTF policy.
- focused/stable rephasing is available behind `SAT_REPHASE=on`. It runs only on scheduled stable-mode restarts and cycles saved phases through
  best, inverted, and original polarity sources
- opt-in guarded chronological backtracking (`SAT_CHRONO=on`) that keeps only `current - 1`
  instead of the normal assertion level when the learned clause remains asserting there; it falls
  back to ordinary non-chronological backtracking when the level gap exceeds
  `SAT_CHRONO_MAX_DELTA` or the learned clause would stop being unit
- opt-in binary implication fast path (`SAT_BINARY_FAST=on`) that keeps binary clauses in the arena
  for proof/model/debug traceability while propagating them through stable binary IDs and implication
  edges; default propagation remains the legacy watched-clause path. Clause minimization is
  binary-reason aware and remains controlled separately by `SAT_CLAUSE_MIN`; binary-fast env runs
  keep minimization off unless `SAT_CLAUSE_MIN` is explicit because the search-core gate does not
  justify promoting recursive minimization on that path yet. With `SAT_OTFS=on` and clause
  minimization enabled, newly learned non-unit clauses also run a bounded OTFS pass over their
  watched literals, deleting live learned watched clauses subsumed by the learned clause when they
  are not active reasons. The pass is capped at learned clauses of 20 literals and candidate clauses
  within 4 extra literals, and emits DRAT deletion records before tombstoning clauses. Original
  clauses are intentionally not removed during search OTFS so SAT models remain checked against the
  full input formula even if the subsuming learned clause is later reduced away. The feature remains
  default-off after the enabled profiling run regressed the current Phase 1 profile suite.
- post-preprocess formula classification in `SAT_STATS_JSON` and `SAT_TRACE_PREPROCESS` output:
  solver 11 records size class, Kissat-style `small`/`bigbig` flags, binary-clause fraction,
  average clause size, and live-variable density. This is instrumentation for future adaptive
  policy routing and does not change default behavior yet.

Still incomplete:

- asymmetric branching clause strengthening
- `use_rcheck` implied-clause checks
- MiniSat's CDCL implementation details; this solver still keeps the repo's `09` search core

So this is now a working BVE preprocessing iteration, but it is not yet a complete MiniSat `simp`
port.

## Solver 11 Configuration Contract

Solver 11 now parses all `SAT_*` runtime controls through one `SolverConfig`
object before reading the CNF. Search, propagation, simplification, and proof
code should not call `std::env::var` directly. The checked-in schema and
feature ledger are:

- `CONFIG_SCHEMA.csv`: environment variable, config field, type, defaults,
  feature/limit/legacy classification, conflicts, requirements, and task owner
- `FEATURES.csv`: machine-readable feature maturity, validation state, promoted
  profiles, validation artifact, risk IDs, and last task that changed the row
- `FEATURES.md`: human-readable notes for the same feature maturity records

Important config controls:

```bash
SAT_PROFILE=baseline|default|fast|experimental
SAT_SEARCH_AXIS=safe|validated|strong
SAT_PREPROCESS_AXIS=off|conservative|gate-aware
SAT_PROOF=off|drat|lrat
SAT_CONFIG_DUMP=on
SAT_CONFIG_OUT=log/solver11.config
SAT_CONFIG_REPLAY=log/solver11.config
SAT_STRICT_CONFIG=on
SAT_STATS_JSON=on
SAT_STATS_HOT=on
SAT_USE_LBD=on
SAT_RESTART=legacy-luby|kissat-ema|reluctant
SAT_RESTART_BLOCK_MARGIN=<f64>  # 0 disables the level blocker
SAT_RESTART_REUSE_TRAIL=on|off
SAT_RESTART_REUSE_TRAIL_FOCUSED=on|off
SAT_RESTART_REUSE_TRAIL_STABLE=on|off
SAT_REDUCE=legacy|lbd-tiered
SAT_REDUCE_MIN_INTERVAL=<usize>  # lbd-tiered default is 100, values must be >= 50
SAT_CLAUSE_MIN=off|basic|recursive-limited|inblock
SAT_OTFS=on|off
SAT_MINIMIZE_DEPTH_LIMIT=<u32>
SAT_PHASE=legacy|saved|target-then-saved|best-then-target-then-saved
SAT_FOCUSED_PHASE=auto|legacy|saved|target-then-saved|best-then-target-then-saved
SAT_STABLE_PHASE=auto|legacy|saved|target-then-saved|best-then-target-then-saved
SAT_SEARCH_MODE=single|focused-stable
SAT_MODE_USE_TICKS=on
SAT_CHRONO=on
SAT_CHRONO_MAX_DELTA=<usize>
SAT_VMTF=off|focused-only|single  # focused-stable defaults to focused-only unless explicit
SAT_REPHASE=on
SAT_BINARY_FAST=on
```

The `default` and `fast` profiles currently keep `SAT_USE_LBD=off`,
`SAT_SEARCH_MODE=single`, and `SAT_MODE_USE_TICKS=off` while retaining the solver-10 preprocessing
stack. The focused/stable search stack remains opt-in until it beats the clean solver 10 profiling
baseline. `baseline` keeps both LBD/focused-stable search and preprocessing off.

When `SAT_SEARCH_MODE=focused-stable` is enabled, `SAT_PHASE` acts as an input preference rather
than the literal phase policy used in every mode: focused mode maps `legacy` and `saved` to `saved`,
and maps `target-then-saved` and `best-then-target-then-saved` to `target-then-saved`; stable mode
always uses `best-then-target-then-saved`. Target/best phase snapshots are updated only in stable
mode. `SAT_FOCUSED_PHASE` and `SAT_STABLE_PHASE` override those per-mode effective policies for
focused/stable matrix tests; `auto` or an empty value keeps the default mapping. In single-mode
search, `SAT_PHASE` is used directly and the per-mode overrides are inert.

`SAT_CONFIG_OUT` writes a deterministic replay file with `schema_version`,
effective profile/axes, proof policy, every config field, feature maturity
records, legacy aliases used, and a stable `config_hash`. `SAT_CONFIG_REPLAY`
loads that file before CNF parsing. By default replay allows only the documented
runtime overrides (`SAT_CONFIG_OUT`, `SAT_RUN_LABEL`, `SAT_STATS_JSON`,
`SAT_STATS_HOT`, `SAT_TRACE_FULL`, `SAT_TRACE_PROOF`, `SAT_TRACE_PREPROCESS`,
`SAT_TRACE_PREPROCESS_DETAILS`, `SAT_TRACE_SEARCH_INTERVAL`,
`SAT_LIMIT_WALL_SEC`, `SAT_LIMIT_RSS_MB`); set
`SAT_CONFIG_REPLAY_ALLOW_OVERRIDES=on` only for explicit experiments.

Phase-selection telemetry distinguishes the default legacy saved-phase path
from explicit phase-policy fallbacks: `phase_legacy_used` counts
`SAT_PHASE=legacy`, `phase_saved_used` counts explicit saved-phase policy use,
and `phase_initial_used` counts true initial-phase fallbacks.

Unpromoted Phase 1 and Phase 2 feature flags default off. Flags whose implementation
bead has not landed are represented in the schema but fail fast if enabled, so
benchmark artifacts cannot accidentally record no-op feature claims. `lrat`
still fails fast. `SAT_LIMIT_*` values are parsed into the config contract and
solve-ending limit hits now return structured `UNKNOWN` results while deleting
any temporary proof artifact.

## Solver 11 Result Contract

Every normally exited solver 11 run writes the minimal 0.3a output contract into
the output directory passed as the second `run.sh` argument:

- `result.json`: mandatory machine-readable result contract
- `status.txt`: exact status string, one of `SAT`, `UNSAT`, `UNKNOWN`, or
  `PARSE_ERROR`
- `model.txt`: emitted for `SAT`
- `proof.out`: emitted for `UNSAT` when `SAT_PROOF=drat`

`result.json` includes:

```text
schema_version, status, exit_code, termination_reason, unknown_reason,
status_file, model_file, proof_file, proof_completeness,
model_check_result, proof_check_result, config_hash, input_sha256, profile,
proof_policy, output_contract_state, stats_json_seen
```

The solver keeps SAT Competition stdout compatibility: exactly one `s` line is
printed, and `v` lines are printed only for SAT. `PARSE_ERROR` is represented as
`s UNKNOWN` on stdout with `status=PARSE_ERROR` in `result.json`. `SAT`,
`UNSAT`, and `UNKNOWN` currently exit `0`; `PARSE_ERROR` exits `2`. When
`SAT_STATS_JSON=on`, the final `c JSON_STATS {...}` record is emitted on stderr
and `result.json` records `stats_json_seen=true`. The JSON stats line includes
config identity, input and binary hashes, result/status fields, timing buckets,
formula sizes, clause-database/GC budget counters, CDCL/preprocessing/watch/LBD/proof
counters, focused/stable mode timing, per-mode conflict/LBD/decision-level
diagnostics, focused restart interval diagnostics, output-contract state, and explicit zero/null placeholders for planned
Phase 1/2 counters that are not implemented yet. High-frequency watcher diagnostics (`watch_scans`,
`watch_blocker_hits`, `watch_clause_loads`, `watch_stale_skips`,
`binary_props`, and `long_props`) default to zero to keep the release hot path
solver-10-equivalent; set `SAT_STATS_HOT=on` for profiling runs that need those
counters. SAT model files are written on every SAT result; the expensive
internal reparse-and-check pass is only run with `SAT_CHECK_INVARIANTS=on`, so
normal runs record `model_check_result=not_checked` and rely on the smoke/bench
harness validation of emitted assignments. The JSON line records
`hot_diagnostics_enabled` so artifacts are explicit about this choice. Formula-classification
fields are emitted as `formula_size_class`, `formula_kissat_small`, `formula_kissat_bigbig`,
`formula_binary_fraction`, `formula_avg_clause_size`, and `formula_variable_density`.
`SAT_TRACE_FULL=on` emits an additional
human-readable `c trace_full ...` line with glue, restart, phase, inprocess,
learned-clause, GC, and branch-heap counters.
Lower-level diagnostics can be enabled independently with `SAT_TRACE_PROOF=on`,
`SAT_TRACE_PREPROCESS=on`, `SAT_TRACE_PREPROCESS_DETAILS=on`, and
`SAT_TRACE_SEARCH_INTERVAL=N` for periodic and final `c search ...` counters.

## Validation

Focused/stable default-profile promotion was reverted on 2026-05-24 after a clean solver 10
comparison:

```bash
SAT_STATS_JSON=on bash tools/bench.sh -t 300 -m 16384 -d benchmarks/profiling \
  --log-dir log/phase1/5b2.2.34-after-default solver/11-kissat-port
python3 tools/compare_bench.py \
  --before log/phase1/5b2.2.34-before-rerun/results.csv \
  --after log/phase1/5b2.2.34-after-default/results.csv \
  --timeout 300
```

| Run | Settings | Solved | SAT | UNSAT | Timeouts | PAR-2 | Results |
|---|---|---:|---:|---:|---:|---:|---|
| before focused restart logn | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on SAT_STATS_JSON=on` | 9/10 | 5 | 4 | 1 | `1756.406` | `log/phase1/5b2.2.34-before-rerun/results.csv` |
| default after promotion | `SAT_STATS_JSON=on` | 9/10 | 5 | 4 | 1 | `1220.197` | `log/phase1/5b2.2.34-after-default/results.csv` |

The promoted run improves aggregate PAR-2 by `536.209` under the Phase 1 PAR-2-only promotion
rule. Instance churn is recorded for diagnosis: `case9` changed from timeout to SAT in `1.321s`,
while `mp1` changed from SAT in `255.953s` to timeout. The largest wins were `case9`
(`-298.679s`), K4 (`-109.586s`), and battleship (`-101.356s`).

Follow-up clean 300s / 16 GiB comparison against solver 10 showed the promoted focused/stable
default was not acceptable as a global default:

| Run | Settings | Solved | PAR-2 | Results |
|---|---|---:|---:|---|
| solver 10 default | `solver/10-bve-preprocess` | 10/10 | `699.671` | `log/phase1/solver10-default-300-vs-solver11-clean/results.csv` |
| solver 11 focused/stable default | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 9/10 | `1201.538` | `log/phase1/5b2.2.52-s11-default-clean/results.csv` |
| solver 11 single/no-LBD | `SAT_USE_LBD=off SAT_SEARCH_MODE=single SAT_MODE_USE_TICKS=off SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `798.196` | `log/phase1/5b2.2.52-s11-single-nolbd-clean/results.csv` |

The default and fast profiles therefore use single-mode/no-LBD search until the focused/stable stack
or a replacement default clears the solver 10 baseline.

Restart trail reuse remains default-off after the 2026-05-25 per-mode split. The implementation now
keeps the Kissat stable reuse rule inside focused/stable mode instead of applying it to the
solver-10-compatible single-mode Luby path. That fixes the previous `mp1` `UNKNOWN`: the exact
`SAT_RESTART_REUSE_TRAIL_STABLE=on` single-mode rerun now follows the normal single-mode restart
path and solves `mp1`, while focused/stable runs can still enable stable or focused reuse explicitly.

| Run | Settings | Solved | PAR-2 | Results |
|---|---|---:|---:|---|
| default after per-mode reuse controls | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `748.732` | `log/phase1/5b2.2.55-default-after/results.csv` |
| stable-only reuse, stopped on hard failure | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 SAT_RESTART_REUSE_TRAIL_STABLE=on` | 3/4 before stop | `FAIL` | `log/phase1/5b2.2.55-reuse-stable-after/results.csv` |
| stable-only reuse after focused/stable scoping | `SAT_RESTART_REUSE_TRAIL_STABLE=on SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` on mp1 | 1/1 | `42.419` | `log/bench-11-kissat-port-2026-05-25-18-43-57/results.csv` |

Focused/stable phase-map controls landed on 2026-05-25 as experiment knobs. The first matrix was
rejected because the env-facing focused/stable path used a VSIDS-in-focused hybrid and the VMTF
batch bump order did not match Kissat. After the focused/stable default was changed to focused-only
VMTF and analyzed variables are moved in existing queue-stamp order, the prior current-auto Velev
`UNKNOWN` rerun solves with the actual focused/stable/tick path enabled.

| Run | Settings | Solved before stop | Outcome | Results |
|---|---|---:|---|---|
| default after phase-map controls | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | PAR-2 `755.634`; no status regression vs `5b2.2.55-default-after` | `log/phase1/5b2.2.54-default-after/results.csv` |
| current auto/auto mapping | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on` | 6/7 | rejected: velev SAT to `UNKNOWN` | `log/phase1/5b2.2.54-current-auto-after/results.csv` |
| saved/saved mapping | `SAT_FOCUSED_PHASE=saved SAT_STABLE_PHASE=saved` plus focused/stable/ticks | 3/4 | rejected: mp1 SAT to `UNKNOWN` | `log/phase1/5b2.2.54-saved-saved-after/results.csv` |
| saved/best mapping | `SAT_FOCUSED_PHASE=saved SAT_STABLE_PHASE=best-then-target-then-saved` plus focused/stable/ticks | 6/7 | rejected: velev SAT to `UNKNOWN` | `log/phase1/5b2.2.54-saved-best-after/results.csv` |
| target/best mapping | `SAT_FOCUSED_PHASE=target-then-saved SAT_STABLE_PHASE=best-then-target-then-saved` plus focused/stable/ticks | 1/2 | rejected: Sudoku UNSAT to `UNKNOWN` | `log/phase1/5b2.2.54-target-best-after/results.csv` |
| current auto/auto after focused VMTF default | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on` on velev | 1/1 | SAT, `0` unknown | `log/bench-11-kissat-port-2026-05-25-18-29-53/results.csv` |

A rejected UNKNOWN cleanup attempt on 2026-05-25 normalized rejected knobs to the safe single-mode
path. That quarantine was reverted. The current fix keeps the env-facing code paths enabled and
addresses the underlying issues: focused/stable defaults to focused-only VMTF, focused VMTF
batches preserve Kissat queue-stamp order, conflict-triggered mode switches happen at the
post-propagation scheduling boundary, stable restart reuse no longer applies to the single-mode
Luby path, focused mode includes Kissat random decision sequences and phase cycling, and stable
reluctant restarts use Kissat's `1024` conflict scale.

| Rerun | Settings | Scope | Outcome | Results |
|---|---|---|---|---|
| focused/stable VMTF actual-path fix | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on SAT_VMTF=focused-only` | Sudoku | UNSAT, `0` unknown | `log/bench-11-kissat-port-2026-05-25-18-19-49/results.csv` |
| focused/stable auto actual-path fix | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on` | velev | SAT, `0` unknown | `log/bench-11-kissat-port-2026-05-25-18-29-53/results.csv` |
| restart reuse actual-path fix | `SAT_RESTART_REUSE_TRAIL_STABLE=on` | mp1 | SAT, `0` unknown | `log/bench-11-kissat-port-2026-05-25-18-43-57/results.csv` |
| focused/stable case9 actual-path fix | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on` | case9 | SAT, `0` unknown | `log/bench-11-kissat-port-2026-05-25-19-57-35/results.csv` |
| full focused/stable actual-path profile | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10 profiling instances | 10/10, PAR-2 `938.956`, `0` unknown | `log/bench-11-kissat-port-2026-05-25-20-01-30/results.csv` |
| final default profile check | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10 profiling instances | 10/10, PAR-2 `757.854`, `0` unknown | `log/bench-11-kissat-port-2026-05-25-20-22-29/results.csv` |

Formula classification landed as instrumentation on 2026-05-25. The default path remains
status-safe; adaptive feature routing is still future work because binary-fast and VMTF are
construction-time choices in the current solver.

| Run | Settings | Solved | PAR-2 | Results |
|---|---|---:|---:|---|
| before classifier | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `755.634` | `log/phase1/5b2.2.54-default-after/results.csv` |
| after classifier | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `753.747` | `log/phase1/f06-classify-default-after/results.csv` |

Any future default/fast promotion must pass the solver 10 gate on the same benchmark set:

```bash
python3 tools/check_solver11_promotion.py \
  --solver10 log/phase1/solver10-default-300-vs-solver11-clean/results.csv \
  --previous log/phase1/5b2.2.53-after-default-rollback/results.csv \
  --candidate log/phase1/<candidate>/results.csv \
  --timeout 300 \
  --memory-mb 16384
```

Solver 11 single-mode parity follow-up on 2026-05-24 kept two low-risk runtime fixes:

- SAT model files are still emitted for SAT, but the internal full-CNF reparse/check now runs only
  with `SAT_CHECK_INVARIANTS=on`; normal benchmark runs record `model_check_result=not_checked`
  and rely on the existing harness assignment verification.
- Propagation is specialized on `SAT_BINARY_FAST`, so the default binary-fast-off path compiles out
  the per-propagation binary implication branch.

| Run | Settings | Solved | PAR-2 | Results |
|---|---|---:|---:|---|
| solver 11 rollback baseline | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `785.980` | `log/phase1/5b2.2.53-after-default-rollback/results.csv` |
| after model-check gating | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `783.169` | `log/phase1/5b2.2.56-after-model-check-gate/results.csv` |
| after binary-fast propagation specialization | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `745.558` | `log/phase1/5b2.2.56-after-prop-specialization/results.csv` |
| rejected accounting/LBD helper hoist | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `750.454` | `log/phase1/5b2.2.56-after-prop-accounting-hoist/results.csv` |
| rejected input-hash opt-in | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `746.133` | `log/phase1/5b2.2.56-after-input-hash-optin/results.csv` |

The accepted state improves solver 11 by `40.422` PAR-2 against the rollback baseline, with no
status changes or correctness failures. It still trails the solver 10 clean baseline by `45.887`
PAR-2, mainly on sudoku (`+13.574s`), Kakuro (`+14.878s`), and `case9` (`+6.434s`).

`SAT_VMTF=single` was added as a default-off diagnostic experiment on 2026-05-25. An initial
unbounded version was rejected because `UNKNOWN` means the solver produced neither SAT nor UNSAT
and is therefore a benchmark failure, even if some rows improve. This single-mode route is not the
Kissat-faithful VMTF policy and is not a promotion target; the supported Kissat-like path is
focused-mode VMTF inside focused/stable search. The table below is retained as historical evidence
for the single-mode experiment and for why focused-mode parity became the next implementation
target.

| Run | Settings | Solved | PAR-2 | Results |
|---|---|---:|---:|---|
| before VMTF-single work | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `745.558` | `log/phase1/5b2.2.56-after-prop-specialization/results.csv` |
| after VMTF-single work, default | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `755.000` | `log/phase1/egy-default-no-regression/results.csv` |
| rejected unbounded VMTF single-mode branch queue | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 SAT_VMTF=single` | 5/10 | `3156.865` | `log/phase1/egy-vmtf-single-profile/results.csv` |
| current default after guarded VMTF | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `755.239` | `log/phase1/egy-default-after-formulaguard-profile/results.csv` |
| guarded VMTF single-mode branch queue | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 SAT_VMTF=single` | 10/10 | `647.341` | `log/phase1/egy-vmtf-single-formulaguard-profile/results.csv` |

Default behavior had no status churn and the default-off `755.239` PAR-2 run was consistent with
the earlier `755.000` default-off rerun. The guarded single-mode experiment was profile-positive on
that run and had no `UNKNOWN`/timeout/error rows, but it remains a non-Kissat diagnostic path rather
than a default or fast-profile candidate.

Latest correctness checks for the accepted state:

- `cargo test`: 328 passed
- `cargo clippy --all-targets -- -D warnings`: passed
- `bash tools/smoke_test.sh solver/11-kissat-port`: 9/9 passed
- `SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port`: 9/9 passed
- `SAT_VMTF=single bash tools/smoke_test.sh solver/11-kissat-port`: 9/9 passed
- `bash -n tools/bench.sh`: passed
- `python3 tools/validate_solver_result.py --self-test`: passed
- `python3 tools/compare_bench.py --self-test`: passed
- `python3 tools/validate_solver11_plan.py`: passed
- comparison verdict against rollback: `significant_improvement`, `PASS`

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

Latest rerun after the 2026-05-15 simplification data-layout pass:

- `cargo test` in `solver/10-bve-preprocess`: 48 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-15-21-45-47`

## 2026-05-15 Profile Simplification Optimization Pass

Target: close the solver-10 preprocessing gap to MiniSat `simp` on `benchmarks/profiling` while
keeping the search algorithm unchanged.

Accepted code-level changes:

- changed simplification occurrence refs from `usize` to `u32` and manually compact dirty
  occurrence lists
- moved propagation's blocker-satisfied fast path before deleted-clause/header checks
- for large formulas, store original-clause abstractions inline with original clauses during
  preprocessing, then strip the preprocessing-only words before search GC
- gate inline abstractions to formulas with at least `750000` original clauses so small/medium
  solved instances keep their previous search trajectory
- use a sorted subsumption relation for short clauses (`len >= 2`) only when clauses are known to be
  canonical-sorted and inline abstractions are active
- match MiniSat's removal scheduling more closely by smudging occurrence lists on deleted clauses
  without treating every removed-clause variable as a backward-subsumption touch
- read inline original-clause abstractions from one loaded header in the subsumption hot path
- reuse the preprocessing scratch buffer for strengthened clauses, compute the strengthened
  abstraction during that same scan, and compact the arena clause in one pass
- replace per-strengthen occurrence-list `position()` removal with lazy membership cleaning on the
  large inline path only
- use lazy preprocessing watcher detach on small formulas and the large inline path; keep strict
  detach for mid-sized formulas where lazy detach perturbed the search path

Key focused K4 movement:

| Build | Preprocess | Total | Notes |
|---|---:|---:|---|
| previous accepted | `103.796s` | `159.31s` | `log/diagnostics/current-best-k4-2026-05-15-21-25/k4.stderr` |
| inline abstractions only | `97.181s` | `158.96s` | `log/diagnostics/simp-inline-strip-2026-05-15-19-42-00/k4.stderr` |
| gated inline + sorted relation | `77.708s` | `132.38s` | `log/diagnostics/simp-inline-gated-sorted9-2026-05-15-20-03-00/k4.stderr` |
| one-pass strengthen updates | `67.996s` | `122.68s` | `log/diagnostics/simp-strength-onepass-compact-2026-05-15-20-50-00/k4.stderr` |
| lazy occurrence membership cleanup | `53.524s` | `108.40s` | `log/diagnostics/simp-lazy-occ-membership-2026-05-15-20-56-00/k4.stderr` |

The final profile bench solves K4 in `95.300s` including proof verification. A follow-up MiniSat
parity pass found that solver 10 was incorrectly using clause deletion as a backward-subsumption
touch source; after matching MiniSat's removal scheduling, focused Kakuro preprocessing improved
from `99.419s` to `71.201s` (`SAT_TRACE_PREPROCESS=1`, `/tmp/kakuro-112.cnf`, 2026-05-15).
MiniSat's verbose run on the same decompressed input reports `28.09s` simplification, so the
remaining gap is now about `2.5x` on preprocessing rather than `3.4x`.

Full `benchmarks/profiling` result:

```bash
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/10-bve-preprocess
```

| Solver/run | Solved | SAT | UNSAT | Timeouts | PAR-2 | Results |
|---|---:|---:|---:|---:|---:|---|
| solver 10 previous accepted | 7/11 | 5 | 2 | 4 | `1087.869` | `log/bench-10-bve-preprocess-2026-05-15-18-39-10/results.csv` |
| solver 10 after this pass | 9/11 | 6 | 3 | 2 | `679.222` | `log/bench-10-bve-preprocess-2026-05-15-21-32-49/results.csv` |
| MiniSat `simp` | 10/11 | 6 | 4 | 1 | `559.646` | `log/bench-minisat-2026-05-14-13-31-31/results.csv` |

The accepted pass flips `REGRandom-K4` from timeout to verified UNSAT (`95.300s`) and `random_v355`
from timeout to SAT (`21.270s`) while preserving `mp1`. Remaining non-search gap is still visible on
Kakuro (`71s` focused preprocessing versus MiniSat's `28.09s` simplification).

Rejected experiments from this pass:

- all-formula inline abstractions: K4 preprocessing improved, but Feistel search roughly doubled
  because the migration perturbed small-instance search paths
- direct inline abstraction loads: K4 preprocessing regressed (`79.259s` versus `77.708s`)
- sorted-relation threshold `5`: tied threshold `9` on K4 and was not worth extra risk
- cleaning every driver occurrence list on large formulas: exceeded the prior Kakuro preprocessing
  time before producing a trace line
- ungated lazy occurrence-membership cleanup: solved K4, but regressed Timetable; gating it to the
  large inline path kept K4 and restored Timetable
- globally lazy preprocessing watcher detach: improved K4/Kakuro and made `random_v355` solve, but
  moved `mp1` to timeout; the accepted policy applies it only to small formulas and the large inline
  path
- side-table abstractions on Kakuro: regressed focused preprocessing to `121.529s`; inline
  abstractions remain faster despite the header arithmetic
- specialized split subsumption hot loop: regressed focused Kakuro preprocessing to `112.626s`
- MiniSat-style contiguous `Vec` subsumption queue: regressed focused Kakuro preprocessing to
  `80.179s`
- MiniSat-style gather-time queue marking: regressed focused Kakuro preprocessing to `72.853s`

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

- full BSR became force-runnable instead of using the earlier formula-size gate; current default is
  off, with `SAT_FULL_BSR=on` retained for targeted comparison runs
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

## Full-BSR Code-Level Optimization Pass

On 2026-05-15, a focused code-level pass targeted the full-BSR preprocessing gap on the current
`benchmarks/profiling` set. The main target was
`46355...REGRandom-K4-L1-Seed40`, where profiling showed almost all proof-off preprocessing time in
`backward_subsumption_check`, especially `subsumption_relation`.

Accepted changes:

- mark each driver clause once per BSR driver instead of rebuilding candidate-side marks for every
  candidate relation check
- pass the already-known driver length into `subsumption_relation`
- scan candidate clauses through `clause_slice()` after length and abstraction filters pass

K4 proof-off preprocessing trace, with identical residual stats
(`eliminated=512`, `resolvents=8192`, `strengthened=348160`):

| Step | Time | Log |
|---|---:|---|
| Baseline | `116.316s` | `log/opt-10-simp-baseline-2026-05-14-22-26-45` |
| Prepared driver marks | `104.752s` | `log/opt-10-simp-prepared-driver-2026-05-14-22-48-48` |
| Candidate slice scan | `94.317s` | `log/opt-10-simp-clause-slice-2026-05-14-22-54-51` |
| Delayed slice creation | `91.222s` | `log/opt-10-simp-delayed-slice-2026-05-14-23-02-40` |

The final proof-on K4 trace still needs about `95.060s` preprocessing plus `64.068s` search
(`log/opt-10-simp-k4-proof-on-current-2026-05-15-00-16-06`), so K4 remains outside a `120s`
profile-bench cap even after the preprocessing speedup.

Final profile benchmark:

```bash
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling solver/10-bve-preprocess
```

- `10-bve-preprocess`: 7/11 solved, PAR-2 `1093.858`
- results: `log/bench-10-bve-preprocess-2026-05-15-00-25-35/results.csv`
- comparison MiniSat run: 10/11 solved, PAR-2 `559.646`
  (`log/bench-minisat-2026-05-14-13-31-31/results.csv`)

Rejected code-level attempts from this pass:

- sorted original clauses plus two-pointer relation: K4 regressed to `116.874s`
- candidate metadata hoisting: K4 regressed to `99.654s`
- removing duplicate live-candidate checks: K4 regressed to `95.931s`
- direct abstraction indexing: K4 regressed to `98.516s`
- unchecked candidate scanning: K4 regressed to `100.302s`
- early relation exit: K4 improved only to `90.752s`, below the 3% keep threshold
- occurrence-count best-variable cleanup: Kakuro improved from `166.964s` to `135.785s`, but K4
  regressed to `102.500s`
- manual occurrence-list compaction: K4 regressed to `95.788s`; Kakuro regressed to `170.497s`
- gated/const-generic occurrence cleanup: still regressed or timed out on K4
- direct driver-slice marking: K4 regressed to `93.521s`
- branchless relation-sign encoding: K4 regressed to `98.359s`
- custom literal-variable helper: K4 regressed to `99.772s`
- driver-clause sentinel instead of enum equality: K4 improved only to `90.455s`, below the 3%
  keep threshold

Validation for the accepted changes:

- `cargo test` in `solver/10-bve-preprocess`: 48 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-15-00-25-25`

## Kakuro MiniSat-Simp Parity Pass

On 2026-05-15, a focused MiniSat comparison pass targeted
`5e933a...Kakuro-easy-112-ext.xml.hg_7.cnf.xz`, where MiniSat full simplification finishes in
about `28.09s` and solver 10 was still spending `71.201s` in preprocessing.

Accepted change:

- Store inline original-clause abstractions as one 32-bit word, matching MiniSat's non-learnt
  clause `abst` field, while keeping learned clauses on two activity words. This removes one
  arena word per live original clause and turns the abstraction filter into the same 32-bit
  variable mask shape MiniSat uses.

Kakuro focused trace:

| Step | Preprocess Time | Notes |
|---|---:|---|
| Previous solver 10 baseline | `71.201s` | pushed baseline from `130ed5c` |
| 32-bit inline original abstraction word | `67.529s` | `eliminated=56052`, `subsumed=4868640`, same residual formula stats |
| MiniSat simp reference | `28.09s` | verbose MiniSat simplification time on the same decompressed input |

The accepted change closes about `5.2%` of the Kakuro preprocessing runtime versus the previous
solver 10 baseline, but the remaining gap is still about `2.4x` versus MiniSat on this instance.

Fresh profile after the accepted change:

- `log/diagnostics/kakuro-abs32-symbols-perf/perf.data`
- `backward_subsumption_check`: about `79%` of sampled cycles
- `original_clause_abstraction` under `subsumption_relation`: about `15%`
- `clean_occurs`: about `7%`

Rejected fresh-pass attempts:

- unmarked initial BSR seeding: `71.806s`, worse than the previous baseline
- delayed heap-update parity: `71.905s`, worse despite MiniSat-like heap stats
- direct short-clause sorted relation: `71.224s`, effectively tied and below the keep threshold
- candidate length/abstraction hoisting: `72.303s`, branch/register pressure regressed
- no-clone BSR seed loop: `71.423s`, below the keep threshold
- end-to-end `u32` abstraction plumbing: `68.645s`, worse than the accepted `67.529s`
- preallocating the BSR queue: `66.846s`, only about `1.0%` faster than the accepted baseline,
  below the 3% keep threshold
- direct inline abstraction loads in the candidate loop: `68.004s`, worse than the helper path
- raw-pointer occurrence scanning: did not finish preprocessing within the `110s` cap

Validation after the accepted change:

- `cargo test` in `solver/10-bve-preprocess`: 48 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-15-23-40-34`
