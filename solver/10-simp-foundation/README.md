# 10-simp-foundation

This iteration starts from `09-root-simp-opts` and adds the first profiled propagation and
MiniSat-style preprocessing line of work.

## Current State

`10` inherits:

- parse-time duplicate-clause filtering using a sorted literal key while preserving the first
  occurrence's original literal order
- parse-time pure-literal cleanup for formulas with at least `100,000` clauses
- limited bounded variable elimination, capped at `5,000` eliminated variables for medium formulas
  and `25,000` for large formulas, with DRAT resolvent additions and SAT model reconstruction for
  eliminated variables
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
- added a conservative bounded variable elimination pass for low-occurrence variables
- tuned BVE to stop before it starts damaging the CDCL search path on the target instance
- lowered the BVE activation threshold for medium formulas and added pure-literal cleanup after
  measuring MiniSat-faster timetable instances

## Intended Focus

The next useful work is still to close selected gaps between `09` and MiniSat `simp`, but the target
instance now shows that more simplification is not automatically better. BVE needs cost controls that
track downstream search impact, not just formula size.

Candidate directions:

- maintain occurrence lists for original clauses
- add backward subsumption and simple self-subsuming resolution
- broaden bounded variable elimination only with a better cost model or per-instance cutoff
- normalize clauses during parsing before the arena is built
- add benchmark instrumentation for simplification impact on active variables, clauses, literals,
  and propagation rate

## Validation

- `cargo test` — `44/44`
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

Kept improvement 3:

- limited bounded variable elimination for low-occurrence variables on large formulas
- generated resolvents are emitted before search as proof additions; eliminated clauses are stored
  so SAT assignments can be extended back to the original variables
- result: `107.276s`, SAT verified, PAR-2 `107.276`
- improvement: `6.8%` faster than duplicate filtering alone, and `56.4%` faster than the original
  `10` baseline before optimization
- log: `log/bench-10-simp-foundation-2026-05-03-00-23-08`
- post-change profile data: `log/profile-10-kakuro-bve/perf.data`; propagation remains the main
  hotspot at `83.43%`, while BVE itself accounts for `0.20%`

Kept improvement 4:

- retuned the BVE cap from `50,000` to `25,000` eliminated variables
- result: `84.070s`, SAT verified, PAR-2 `84.070`
- improvement: `21.6%` faster than the `50,000` cap BVE result, `26.9%` faster than duplicate
  filtering alone, and `65.8%` faster than the original `10` baseline before optimization
- log: `log/bench-10-simp-foundation-2026-05-03-00-57-59`
- profile data with symbols: `log/profile-10-kakuro-bve-cap25-symbols/perf.data`; propagation was
  `72.55%`, while the two duplicate-filtering hash helpers together were about `10.62%`

Kept improvement 5:

- removed the second duplicate filter after BVE, while keeping parse-time duplicate filtering
- this was only a `2.0%` improvement with the earlier `50,000` BVE cap, but became worthwhile after
  the cap was tuned to `25,000`
- result: `78.931s`, SAT verified, PAR-2 `78.931`
- improvement: `6.1%` faster than `25,000`-cap BVE with the post-BVE duplicate filter, `31.4%`
  faster than duplicate filtering alone, and `67.9%` faster than the original `10` baseline before
  optimization
- this is `7.4%` faster than the measured MiniSat `simp` CPU time of `85.263s` on the same target
  instance
- log: `log/bench-10-simp-foundation-2026-05-03-01-08-02`
- profile data with symbols: `log/profile-10-kakuro-bve-cap25-no-post-dedup/perf.data`;
  propagation was `77.75%`, hash helpers were down to about `5.60%`, and the remaining
  parse-time duplicate filter was `0.61%`

MiniSat-faster sample:

- selected three SAT Competition 2025 instances where historical MiniSat `simp` beat `09` and both
  solvers finished within `180s`
- target directory: `/tmp/sat-minisat-faster-three`
- pre-change solver `10` result: `224.272s` total
- fresh MiniSat `simp` result on the same three: `117.956s` total

Kept improvement 6:

- lowered BVE activation from `1,000,000` clauses to `100,000` clauses, while using a smaller
  `5,000` eliminated-variable cap for medium formulas and retaining `25,000` for large formulas
- result on the MiniSat-faster three-instance target: `183.302s`, SAT verified, PAR-2 `183.302`
- improvement: `18.3%` faster than previous solver `10` on that target set
- log: `log/bench-10-simp-foundation-2026-05-03-08-37-17`
- six-instance guard sample result: `272.069s`, improved from `309.151s`
- profile data with symbols on the strongest win:
  `log/profile-10-c392-medium-bve-cap5k/perf.data`; propagation was `47.25%`, with branch
  selection and heap maintenance becoming visible after the simplification win

Kept improvement 7:

- added one parse-time pure-literal cleanup pass before BVE, storing removed clauses so SAT models
  can be extended back to the original variables
- opportunity check on `SC25_Timetable_C_392` found `12,089` pure variables after dedup, removing
  `35,782` clauses and `96,385` literals in one pass
- result on the MiniSat-faster three-instance target: `144.567s`, SAT verified, PAR-2 `144.567`
- improvement: `21.1%` faster than adaptive medium BVE alone and `35.5%` faster than previous
  solver `10` on that target set
- six-instance guard sample result: `232.762s`, improved from `272.069s`
- Kakuro guard result: `32.758s`, improved from `78.931s`
- logs: `log/bench-10-simp-foundation-2026-05-03-08-53-49`,
  `log/bench-10-simp-foundation-2026-05-03-08-56-27`,
  `log/bench-10-simp-foundation-2026-05-03-09-01-40`
- profile data with symbols on `SC25_Timetable_C_393`:
  `log/profile-10-c393-pure-medium-bve/perf.data`; propagation was `56.44%`, branch selection was
  `10.46%`, and parse-time preprocessing was below the main search costs

Rejected attempts:

- parse-time clause normalization: unit-clean, but exceeded the `238.7s` keep threshold before
  completing the target run, so it was reverted
- first binary shortcut implementation: produced an invalid UNSAT proof because it trusted stale
  watcher blockers and skipped the reason-head invariant; fixed before keeping the final version
- encoded binary marker in `Watcher`: unit-clean, but exceeded the incremental `180.4s` keep
  threshold before completing, so it was reverted
- binary-clause subsumption and binary self-subsuming-resolution analysis found zero opportunities
  on the target formula, so no solver change was made
- dynamic BVE candidate requeueing with the same conservative candidate limits reached `110.641s`,
  slower than the accepted `50,000` cap BVE baseline, so it was reverted
- a broader BVE threshold attempt exceeded the cutoff before completing and was reverted
- raising the BVE cap to `75,000` with the same candidate thresholds reached `133.949s`, so it was
  reverted
- lowering the BVE cap to `20,000` reached `132.102s`, so it was reverted
- raising the no-post-dedup BVE cap to `30,000` reached `127.683s`, so it was reverted
- a blocker-only binary propagation fast path exceeded the cutoff before completing, so it was
  reverted
- lowering the medium-formula BVE cap to `1,000` exceeded the three-instance keep cutoff before
  finishing the first instance, so it was reverted
