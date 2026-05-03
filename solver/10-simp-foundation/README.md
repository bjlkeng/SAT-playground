# 10-simp-foundation

This iteration starts from `09-root-simp-opts` and adds the first profiled propagation cleanup for
the next simplification/preprocessing line of work.

## Current State

`10` inherits:

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

- `cargo test` — `40/40`
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

Kept improvement:

- binary-clause propagation fast path
- result: `185.972s`, SAT verified, PAR-2 `185.972`
- improvement: `24.4%` faster than the `246.104s` baseline
- log: `log/bench-10-simp-foundation-2026-05-02-22-32-16`

Rejected attempts:

- parse-time clause normalization: unit-clean, but exceeded the `238.7s` keep threshold before
  completing the target run, so it was reverted
- first binary shortcut implementation: produced an invalid UNSAT proof because it trusted stale
  watcher blockers and skipped the reason-head invariant; fixed before keeping the final version
- encoded binary marker in `Watcher`: unit-clean, but exceeded the incremental `180.4s` keep
  threshold before completing, so it was reverted
