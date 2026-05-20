# Solver 11 Section 1.5 summary

Bead: `SAT-playground-5b2.2.8` - `[1.5] Saved, target, and best phase selection`

## Implementation

- Added opt-in phase selection policies behind `SAT_PHASE`:
  - `legacy` remains the default and preserves solver-10-compatible saved-phase branching.
  - `saved` starts saved phases as unassigned and falls back to the configured initial phase.
  - `target-then-saved` uses a per-restart target phase captured from the deepest unconflicted trail prefix, then saved, then initial.
  - `best-then-target-then-saved` uses the best full-solve phase prefix, then target, saved, and initial.
- Added `target_phase`, `best_phase`, `original_phase`, `target_assigned`, `best_assigned`, `phase_ticks`, and `phase_policy` to the solver.
- Added phase-use counters to normal JSON stats and full trace output:
  - `phase_saved_used`
  - `phase_target_used`
  - `phase_best_used`
  - `phase_initial_used`
- Kept default-profile allocation and behavior conservative: extra target/best/original buffers are only allocated for non-legacy `SAT_PHASE` modes.
- Updated `README.md` and `SOLVER11_STATE.md` to describe the opt-in phase modes and to state that they are not default-profile behavior yet.

## Fresh-eyes review notes

- Reviewed the phase-policy implementation after the first pass for stale state, restart interaction, temporary assumptions, stats accounting, and default-profile overhead.
- Fixed one restart edge case: `perform_restart_if_pending()` now clears the per-restart target phase even when conflict analysis has already backtracked to root before the pending restart is consumed.
- Added a regression test for that root-level pending-restart path.
- Confirmed temporary-assumption accounting does not update saved, target, or best phase buffers.
- Confirmed legacy mode does not allocate the new opt-in phase buffers.

## Unit and smoke validation

- `cargo fmt --check`: pass
- `cargo test phase -- --nocapture`: pass, 14 focused tests
- `cargo test`: pass, 188 tests
- `cargo clippy --all-targets -- -D warnings`: pass
- `bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9, `log/2026-05-20-07-47-06`
- `SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9, `log/2026-05-20-07-47-18`
- `SAT_PHASE=saved bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9, `log/2026-05-20-07-47-22`
- `SAT_PHASE=target-then-saved bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9, `log/2026-05-20-07-47-26`
- `SAT_PHASE=best-then-target-then-saved bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9, `log/2026-05-20-07-47-30`

## Profile benchmark no-regression gate

Command:

```bash
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling --log-dir log/1.5/profile-after solver/11-kissat-port
```

Result:

- 11 instances
- 9 solved: 6 SAT, 3 UNSAT
- 2 timeouts
- PAR-2: `707.821`
- Results: `log/1.5/profile-after/results.csv`

Comparison against `log/1.4/profile-after/results.csv`:

- Verdict: pass
- Solved count: 9 -> 9
- PAR-2: `708.182` -> `707.821`
- Delta: `-0.361`
- Status regressions: none

## Search-core opt-in phase gates

Baseline: `log/bench-s11-1.3a-search-core-2026-05-19-1709/results.csv`

`SAT_PHASE=saved`:

- Results: `log/1.5/search-core-saved/results.csv`
- 9 instances, 5 solved, 3 timeouts, 1 unknown
- PAR-2: `1188.569`
- Comparison verdict: pass
- Solved count: 5 -> 5
- PAR-2 delta: `-7.273`
- Status regressions: none

`SAT_PHASE=target-then-saved`:

- Results: `log/1.5/search-core-target/results.csv`
- 9 instances, 4 solved, 4 timeouts, 1 unknown
- PAR-2: `1430.212`
- Comparison verdict: fail
- Solved count: 5 -> 4
- New timeout: `mp1-Nb7T46`
- Notable wins: `544707209399nw.shuffled-as.sat03-1671` improved by about 79.2s; `SC25_Timetable_C_406` improved by about 69.1s.
- Notable regressions: `SC25_Timetable_C_392`, `battleship-16-31-sat`, and `mp1-Nb7T46`.

`SAT_PHASE=best-then-target-then-saved`:

- Results: `log/1.5/search-core-best/results.csv`
- 9 instances, 3 solved, 5 timeouts, 1 unknown
- PAR-2: `1491.733`
- Comparison verdict: fail
- Solved count: 5 -> 3
- Newly solved: `case9`
- New timeouts: `SC25_Timetable_C_406`, `battleship-16-31-sat`, `mp1-Nb7T46`

Promotion decision:

- Keep `SAT_PHASE=legacy` as the default.
- Keep `saved`, `target-then-saved`, and `best-then-target-then-saved` as opt-in scaffolding for later focused/stable and rephase work.
- `saved` passed the single-run search-core gate, but the gain is small enough that it should not be promoted alone without a repeated/noise-controlled benchmark.
- `target` and `best` modes are strongly search-path sensitive and should be tuned by later beads before promotion.

## Manual phase counter capture

Instance: `benchmarks/iteration/search-core/544707209399nw.shuffled-as.sat03-1671.cnf.xz`

Artifacts:

- `log/1.5/manual-stats-544/saved/stderr.txt`
- `log/1.5/manual-stats-544/target/stderr.txt`
- `log/1.5/manual-stats-544/best/stderr.txt`

Counters:

| Mode | Result | Elapsed | Conflicts | Decisions | Propagations | Saved | Target | Best | Initial | Restarts |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `saved` | UNKNOWN | 5.010506s | 45740 | 62277 | 35191520 | 62230 | 0 | 0 | 47 | 133 |
| `target-then-saved` | SAT | 0.253079s | 2662 | 4761 | 1861408 | 1041 | 3673 | 0 | 47 | 14 |
| `best-then-target-then-saved` | UNKNOWN | 5.009570s | 52197 | 72365 | 34538045 | 201 | 157 | 71960 | 47 | 157 |

These counters confirm that the opt-in policy precedence is exercised in real search, while the benchmark gates show why the non-legacy modes should not be promoted as defaults yet.
