# Solver 11 Section 1.9 summary

Bead: `SAT-playground-5b2.2.12` - `[1.9] Focused/stable mode scaffold and reluctant restarts`

## Implementation

- Added opt-in focused/stable search-mode scaffolding behind:
  - `SAT_USE_LBD=on`
  - `SAT_SEARCH_MODE=focused-stable`
- Kept `SAT_SEARCH_MODE=single` as the default, preserving default search behavior.
- Added `SearchMode::{Focused, Stable}` and mode state:
  - `search_mode`
  - `mode_start_conflicts`
  - `mode_start_decisions`
  - `mode_switches`
  - `mode_switch_at_conflicts`
  - `mode_init_conflicts`
  - `mode_interval`
  - `mode_interval_scale`
- Focused/stable policy:
  - focused mode uses the existing decision heap, saved/target phase policy, and EMA restart behavior
  - stable mode uses the existing decision heap, best/target/saved phase behavior, and reluctant restart behavior
- Added `Reluctant { u, v }` with a Luby-prefix schedule matching `1, 1, 2, 1, 1, 2, 4, ...`.
- Added standalone `SAT_RESTART=reluctant` support.
- Mode switching:
  - starts in focused mode when `SAT_SEARCH_MODE=focused-stable`
  - first switch happens after `SAT_MODE_INIT_CONFLICTS`, now defaulting to `2000`
  - later intervals use `sqrt(mode_switches + 1) * mode_init * SAT_MODE_INTERVAL_SCALE`
  - mode switches clear any pending restart and reset restart-window counters so the new mode starts clean
- Added stats and trace output:
  - `reluctant_restarts`
  - `mode_switches`
  - `decisions_focused`
  - `decisions_stable`
- Updated `CONFIG_SCHEMA.csv`, `README.md`, and `SOLVER11_STATE.md`.

## Fresh-eyes review notes

- Rechecked that default `single` mode keeps default behavior: it starts internally as stable only for stats accounting, never switches, and uses the configured restart/phase policy.
- Rechecked that focused/stable mode allocates phase buffers even when `SAT_PHASE=legacy`, because stable mode may need best/target/saved phase fallback.
- Rechecked that mode switching does not rebuild or reorder the branch heap.
- Rechecked that switching modes drains `restart_pending` and resets restart-window counters so a restart scheduled by the previous mode does not leak into the next mode.
- Fresh-eyes adjustment: focused/stable mode now captures target/best phase prefixes even while currently focused, so stable mode has useful best-phase memory when it starts.
- Rechecked temporary-assumption paths: mode switching is disabled while temporary accounting is active.
- Rechecked the initial smoke failures from the first smoke invocation and confirmed they were caused by running three smoke suites in parallel into the same timestamped log directory; rerunning sequentially produced clean results.

## Unit and smoke validation

- `cargo test mode -- --nocapture`: pass, 12 focused matches
- `cargo test restart -- --nocapture`: pass, 13 focused matches
- `cargo fmt --check`: pass
- `cargo test -- --nocapture`: pass, 197 tests
- `cargo clippy --all-targets -- -D warnings`: pass
- `bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9, `log/2026-05-20-11-30-43`
- `SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9, `log/2026-05-20-11-30-51`
- `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9, `log/2026-05-20-11-30-58`
- `SAT_RESTART=reluctant bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9, `log/2026-05-20-11-53-41`
- `git diff --check`: pass

## Default profile benchmark no-regression gate

Command:

```bash
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling --log-dir log/1.9/profile-after solver/11-kissat-port
```

Result:

- 11 instances
- 9 solved: 6 SAT, 3 UNSAT
- 2 timeouts
- PAR-2: `708.000`
- Results: `log/1.9/profile-after/results.csv`

Comparison against `log/1.5/profile-after/results.csv`:

- Verdict: pass
- Solved count: 9 -> 9
- PAR-2: `707.821` -> `708.000`
- Delta: `+0.179`
- Status regressions: none
- Newly solved: none
- New timeouts: none

## Opt-in focused/stable search-core gate

Command:

```bash
SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable \
  bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/search-core \
  --log-dir log/1.9/search-core-focused-stable solver/11-kissat-port
```

Result:

- 9 instances
- 3 solved: 3 SAT, 0 UNSAT
- 6 unsolved: 5 timeout, 1 unknown
- PAR-2: `1567.314`
- Results: `log/1.9/search-core-focused-stable/results.csv`

Comparison against `log/bench-s11-1.3a-search-core-2026-05-19-1709/results.csv`:

- Verdict: fail
- Solved count: 5 -> 3
- PAR-2: `1195.842` -> `1567.314`
- Delta: `+371.472`
- Newly solved: `DLTM_twitter845_79_19`
- New timeouts: `SC25_Timetable_C_406`, `battleship-16-31-sat`, `mp1-Nb7T46`
- Promotion verdict: significant regression

Promotion decision:

- Keep `SAT_SEARCH_MODE=single` as the default.
- Keep focused/stable and reluctant restart behavior as opt-in scaffolding.
- Do not promote focused/stable mode until VMTF/rephase experiments or a later tuning bead removes the solved-instance regressions.

## Manual mode counter capture

Instance: `benchmarks/iteration/search-core/544707209399nw.shuffled-as.sat03-1671.cnf.xz`

Command shape:

```bash
SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable \
SAT_MODE_INIT_CONFLICTS=100 SAT_MODE_INTERVAL_SCALE=1.0 \
SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=5 \
bash solver/11-kissat-port/run.sh <decompressed-input> log/1.9/manual-stats-focused-stable/proof
```

The temporary decompressed input was removed after capture.

Artifacts:

- `log/1.9/manual-stats-focused-stable/stderr.txt`
- `log/1.9/manual-stats-focused-stable/stdout.txt`
- `log/1.9/manual-stats-focused-stable/proof/result.json`

Captured counters:

| Field | Value |
| --- | ---: |
| result | SAT |
| elapsed_sec | 2.333766 |
| conflicts | 19265 |
| decisions | 57762 |
| propagations | 16732595 |
| restarts | 1806 |
| luby_restarts | 0 |
| glucose_restarts | 140 |
| reluctant_restarts | 1674 |
| mode_switches | 43 |
| decisions_focused | 16602 |
| decisions_stable | 41160 |
| phase_saved_used | 17070 |
| phase_target_used | 190 |
| phase_best_used | 40455 |
| phase_initial_used | 47 |

The counter capture confirms that focused/stable switching, focused decisions, stable decisions, EMA restarts, reluctant restarts, and stable best-phase selection are exercised in real search.
