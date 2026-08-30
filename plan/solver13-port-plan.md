# Solver 13 — faithful kissat 4.0.4 reimplementation in Rust

Goal (set 2026-08-30): `solver/13-kissat-rs` is a faithful, feature-complete
reimplementation of kissat 4.0.4 in Rust. Acceptance: on the 2025 full track
(`benchmarks/sat-comp-2025`, 400 instances) at 3600 s / 16 GB / 32 parallel
cores, solved count and PAR-2 both within **2% of kissat 4.0.4** measured in
the same fresh paired run. Zero correctness failures (valid models, valid DRAT
proofs, no status contradictions).

Reference source: `benchmarks/reference-solvers/kissat-latest/` (kissat
4.0.4, ~32.5k LOC across ~100 `.c` files; binary at `build/kissat`).

Recent kissat 2025-track calibration (sequential quiet host, 3600s/16GB/32j):

| run | solved | PAR-2 |
|---|---|---|
| `log/kissat-full-20260810-073149` | 294/400 | 930,904 |
| `log/kissat-full-20260828-162018` | 292/400 | 939,548 |

So ±2 solved is run noise; the 2% window (~5.8 solved, ~19k PAR-2) is
comfortably wider than noise **if** the port is truly faithful.

## Why fresh port, not solver12

solver12 (46k LOC Rust) is kissat-*derived* but architecturally divergent
(own simp/branch/sweep structure, guarded chrono, different scheduling). The
2026 holdout showed its general search lags kissat OOD (160 v 197). Converging
it to exact kissat behavior is harder than a disciplined transliteration.
solver12 remains a reference for Rust idioms (fast parsing, unchecked
indexing, xz handling) — not for algorithm structure.

## Port conventions (binding for all modules)

- One Rust module per kissat C file, same name: `analyze.c` → `src/analyze.rs`.
  Keep function names (`kissat_analyze` → `analyze::analyze`), field names,
  control flow, and comments structurally recognizable against the C.
- `struct kissat` → `struct Solver` in `internal.rs`, same fields, same roles.
  Skip fields/blocks under `#if !defined(NDEBUG)`, `LOGGING`, `METRICS`,
  `COVERAGE`, `CHECKING`/checker, and `EMBEDDED`/incremental-only paths — the
  reference binary is the default competition build; match THAT build.
- Types: C `unsigned` → `u32`, `uint64_t` → `u64`, `word`/vector words → u32.
  Literal encoding identical: `lit = 2*idx + sign`; INVALID_LIT = u32::MAX.
- Data-structure faithfulness where it affects trajectory or speed:
  - arena.c: single u32 arena holding clauses (header + lits), refs are u32
    offsets; garbage collection preserves kissat's ordering exactly.
  - vector.c: watch lists as segments in one shared u32 arena with kissat's
    enlarge/defrag semantics (watch ORDER affects propagation order and hence
    the whole trajectory — do not substitute Vec<Vec<>> semantics).
  - Watches: same tagged union (binary+blocking-lit inline, large watch =
    blocking lit + ref pair) — see watch.h.
- random.h PRNG ported bit-exactly; kissat_next_random64 etc.
- sort.c radix/quick sorts ported with identical tie-breaking (sort order
  feeds candidate schedules everywhere).
- Options: full options.c table, exact names/defaults/min/max, `--opt=val`
  CLI, plus `--conflicts`/`--decisions` limits and `-s`/`-n`/`-q`. Configs
  (`--sat`, `--unsat`, `--default`) from config.c.
- Statistics: full statistics.h counter set, printed in kissat's `-s` format
  (this is the parity oracle surface).
- ticks counted exactly as kissat does (cache-line approximation in
  utilities.h) — mode switching and probe/eliminate effort limits depend on
  ticks, so tick parity is required for trajectory parity.
- No external SAT deps; std + minimal utility crates only. Release profile:
  fat LTO, 1 codegen unit, target-cpu=native, strip=true (copy solver12).
- Unsafe unchecked indexing allowed in hot paths once parity is established;
  first make it right, then make it fast.

## Parity methodology (the faithfulness oracle)

kissat with fixed options is deterministic. A faithful port must reproduce
its statistics *exactly*: conflicts, decisions, propagations, ticks,
restarts, reductions, eliminated vars, subsumed clauses, sweep counts, etc.

Harness: `solver/13-kissat-rs/tools/parity.py` runs
`kissat -n -s --conflicts=N inst.cnf` and `sat-solver` equivalent, diffs the
stats block. Corpus: `tests/cnf/*`, `benchmarks/discriminating`, plus ~15
medium cells; limits N in {1e3, 1e4, 1e5}. Escalation stages:

1. Search-only parity (`--probe=0 --eliminate=0 ... `, or `--plain`).
2. + each inprocessing engine enabled one at a time.
3. Full default config parity.
4. Full-run parity (no conflict limit) on fast instances: identical status,
   identical final stats, model/proof valid.

Divergence debugging: bisect by comparing per-interval stats (`--verbose`
report lines at matching conflict counts), then instrument both sides.

## Phases / task map (session task list mirrors this)

1. **Scaffold + plan** (this doc; crate skeleton, build.sh, run.sh). ✔ when
   crate builds and run.sh emits `s UNKNOWN` on a trivial CNF.
2. **Foundation**: literal/options/statistics/Solver struct/utilities/random/
   flags/values/assigned/averages/format/profiles.
3. **Core CDCL**: parse, arena/clause/vector/watch, assign+prop{search,beyond,
   initially}, analyze/deduce/minimize/shrink/learn/bump, backtrack, decide/
   queue/heap/phases, restart/reluctant, reduce/tiers/promote, collect, trail,
   mode/kimits/averages, search loop, internal.c API, import.c. Milestone:
   smoke tests pass; stage-1 parity exact.
4. **Proof + witness**: proof.c (DRAT), extend/witness/weaken. drat-trim
   validates UNSAT proofs; models validated by tools/smoke_test.sh.
5. **Inprocessing** (bulk of the work): preprocess, classify, fastel,
   eliminate (BVE + definition/gates/ands/equivalences/ifthenelse + resolve +
   forward/strengthen/substitute + dense/propdense + weaken), backbone, probe
   chain (proprobe, transitive, vivify), sweep + kitten (~4k LOC embedded
   solver), congruence, factor, lucky, walk, warmup, rephase, reorder,
   compact. Milestone: stage-2/3 parity exact per engine.
6. **Parity closure**: full corpus stage-3/4 parity; document any residual
   divergence with mechanism (there should be none — kissat default build has
   no address- or time-dependent behavior in search decisions).
7. **Performance**: perf-record vs kissat on matched-trajectory instances;
   optimize until wall ratio ≈ 1.0x. (PAR-2 within 2% needs near-1.0 speed on
   the ~40 cells inside the last 10% of the timeout.)
8. **Acceptance**: fresh paired 2025 full-track run, both solvers, 3600s/16GB/
   32 pinned cores (methodology of `log/kissat-full-20260828-162018` +
   `tools/run_kissat_full.sh`). Gate: solved ≥ 0.98×kissat, PAR-2 ≤
   1.02×kissat, zero correctness failures.

## Standing rules

- Correctness failures are absolute blockers (CLAUDE.md).
- Commit per completed module group; smoke test before every commit.
- Never quote subset/parity numbers as acceptance evidence; the only
  acceptance evidence is the phase-8 paired run.
- kissat reference binary and source are read-only; never modify them.
