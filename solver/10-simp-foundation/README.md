# 10-simp-foundation

This iteration starts from `09-root-simp-opts` and adds the first profiled propagation cleanup for
the next simplification/preprocessing line of work.

## Current State

`10` inherits:

- parse-time duplicate-clause filtering using a sorted literal key while preserving the first
  occurrence's original literal order
- watched-literal BCP with blocker fast paths
- a binary-clause propagation fast path that avoids the general long-clause scan while preserving
  the reason-clause invariant that the implied literal is stored at position 0
- EVSIDS-style variable activity and saved-phase branching
- conflict-clause minimization modes: `none`, `basic`, and `deep`
- deep minimization through learned-clause reasons
- MiniSat-style learned-clause activity bumps and learned-clause reduction thresholds
- a MiniSat-style packed clause arena with stable clause refs and relocating GC
- streamed proof logging through a fixed 16 MiB byte buffer into `proof.out.tmp`
- root-level `simplify()` that deletes satisfied clauses and trims root-false literals from
  surviving original clauses
- the profiled `09` hot-path cleanup: lazy branch-heap cleanup, bottom-up heap rebuilds, in-place
  watcher compaction, in-place learned-clause reduction, scratch-buffer conflict analysis, and the
  learned-unit shortcut

## What Changed

- copied `09-root-simp-opts` into a new self-contained iteration directory
- renamed the package / iteration metadata for `10-simp-foundation`
- added a binary-clause branch in propagation so two-literal clauses directly test/enqueue the
  other watched literal instead of falling through the long-clause replacement loop
- added MiniSat-simp-inspired duplicate-clause filtering before the arena is built

## Intended Focus

The next useful work is still to close selected gaps between `09` and MiniSat `simp`, starting with
small, measurable simplification passes before attempting full bounded variable elimination.

Candidate directions:

- maintain occurrence lists for original clauses
- add backward subsumption and simple self-subsuming resolution
- add bounded variable elimination for low-cost variables
- normalize clauses during parsing before the arena is built
- add benchmark instrumentation for simplification impact on active variables, clauses, literals,
  and propagation rate

## Validation

- `cargo test` — `41/41`
- `bash tools/smoke_test.sh solver/10-simp-foundation` — `9/9`

## Targeted Optimization Log

Machine: AMD Ryzen 5 5600, 62 GiB RAM.

Target instance:
`5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7.cnf.xz` from
`benchmarks/sat-comp-2025-medium`.

Baseline command:

```bash
bash tools/bench.sh -t 500 -m 16384 -d /tmp/sat-opt-kakuro-one solver/10-simp-foundation
```

Baseline result before changes:

- `246.104s`, SAT verified, PAR-2 `246.104`
- log: `log/bench-10-simp-foundation-2026-05-02-19-42-43`

Profiler evidence:

- baseline `perf record -F 99 -g -e cycles:u` for 120 seconds showed
  `sat_solver::Solver::propagate` at `92.44%` self time
- after the kept change, the same 120 second sample still showed propagation as the main hotspot
  at `91.10%`, so future work should keep focusing on watcher/propagation costs
- post-change profile data: `log/profile-10-kakuro-binary-bcp/perf.data`

Kept improvement 1:

- binary-clause propagation fast path
- result: `185.972s`, SAT verified, PAR-2 `185.972`
- improvement: `24.4%` faster than the `246.104s` baseline
- log: `log/bench-10-simp-foundation-2026-05-02-22-32-16`

MiniSat simplification comparison:

- `minisat -no-pre`: `152.331s` CPU, `19619849` clauses, `69507454` literals at search
- `minisat -no-elim`: `123.699s` CPU, `14751209` clauses, `52814974` literals at search
- `minisat` with full `simp`: `85.263s` CPU, `142307` active vars, `14742137` clauses,
  `52871496` literals at search
- occurrence analysis of the input found `4,868,640` permutation-equivalent duplicate clauses,
  which explains nearly all of the `-no-elim` clause reduction

Kept improvement 2:

- parse-time duplicate-clause filtering using sorted literal keys
- result: `115.040s`, SAT verified, PAR-2 `115.040`
- improvement: `38.1%` faster than binary propagation alone, and `53.3%` faster than the original
  `10` baseline before optimization
- log: `log/bench-10-simp-foundation-2026-05-02-23-28-19`
- post-change profile data: `log/profile-10-kakuro-dedup/perf.data`; propagation remains the main
  hotspot at `86.99%`, while duplicate filtering itself accounts for `2.37%`

Rejected attempts:

- parse-time clause normalization: unit-clean, but exceeded the `238.7s` keep threshold before
  completing the target run, so it was reverted
- first binary shortcut implementation: produced an invalid UNSAT proof because it trusted stale
  watcher blockers and skipped the reason-head invariant; fixed before keeping the final version
- encoded binary marker in `Watcher`: unit-clean, but exceeded the incremental `180.4s` keep
  threshold before completing, so it was reverted
- pure-literal cleanup was skipped after analysis because only `1,134` pure literals affected about
  `2,106` clauses on this instance, far below the observed duplicate-clause opportunity
