# Solver 11 Section 1.10 summary

Bead: `SAT-playground-5b2.2.13` - `[1.10] VMTF focused-mode decision queue`

## Implementation

- Added `src/branch.rs` with `VmtfQueue`, a doubly linked Variable-Move-To-Front queue.
- Enabled `SAT_VMTF=on` only as an opt-in focused/stable feature:
  - requires `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable`
  - focused mode selects branch variables from VMTF
  - stable mode continues to select from the existing VSIDS heap
  - all default profile behavior remains on the existing heap path
- Conflict-analysis activity bumping now stamps and moves analyzed variables to the VMTF front while
  the solver is in focused mode.
- Backtracking updates the VMTF search cursor when a newly unassigned variable has a newer stamp
  than the current cursor.
- VMTF decisions still call `pick_branch_phase`, so saved/target/best phase behavior remains shared.
- VMTF-picked variables are removed from the VSIDS heap so the stable-mode heap remains consistent
  after later backtracking reinserts.
- Updated `SolverConfig`, `FEATURES.csv`, `FEATURES.md`, `README.md`, and `SOLVER11_STATE.md`.

## Fresh-eyes review notes

- Rechecked that default `SAT_SEARCH_MODE=single` cannot enter VMTF selection because
  `vmtf_branching_active` requires focused/stable mode and current focused mode.
- Rechecked that stable mode ignores the VMTF queue and still picks the highest-activity heap
  variable.
- Rechecked assigned, eliminated, and non-decision variables are skipped by the VMTF picker.
- Rechecked backtracking updates the VMTF cursor only for valid unassigned decision candidates.
- Rechecked temporary assumption accounting does not stamp VMTF during temporary conflict-analysis
  bump paths.
- Rechecked that VMTF keeps the existing phase-selection path rather than introducing separate
  polarity logic.
- Rechecked the default profile benchmark delta: same solved/timeout statuses as 1.9; the +8.039s
  PAR-2 movement is spread across normal noisy solved rows and does not correspond to active VMTF
  code in default mode.

## Unit and smoke validation

- `cargo test vmtf -- --nocapture`: pass, 8 focused matches
- `cargo test -- --nocapture`: pass, 222 tests
- `cargo clippy --all-targets -- -D warnings`: pass
- `cargo fmt --check`: pass
- `bash tools/smoke_test.sh solver/11-kissat-port`: pass, 9/9,
  `log/2026-05-20-20-22-26`
- `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_VMTF=on bash tools/smoke_test.sh solver/11-kissat-port`:
  pass, 9/9, `log/2026-05-20-20-22-37`
- `SAT_CHECK_INVARIANTS=on SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_VMTF=on bash tools/smoke_test.sh solver/11-kissat-port`:
  pass, 9/9, `log/2026-05-20-20-22-43`

## Default profile benchmark no-regression gate

Command:

```bash
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling --log-dir log/1.10/profile-after solver/11-kissat-port
```

Result:

- 11 instances
- 9 solved: 6 SAT, 3 UNSAT
- 2 timeouts
- PAR-2: `716.039`
- Results: `log/1.10/profile-after/results.csv`

Comparison against `log/1.9/profile-after/results.csv`:

- Solved count: 9 -> 9
- PAR-2: `708.000` -> `716.039`
- Delta: `+8.039`
- Status regressions: none
- Newly solved: none
- New timeouts: none

## Opt-in VMTF search-core gate

Command:

```bash
SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_VMTF=on \
  bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/search-core \
  --log-dir log/1.10/search-core-vmtf solver/11-kissat-port
```

Result:

- 9 instances
- 4 solved: 4 SAT, 0 UNSAT
- 5 unsolved: 4 timeout, 1 unknown
- PAR-2: `1258.395`
- Results: `log/1.10/search-core-vmtf/results.csv`

Comparison against `log/1.9/search-core-focused-stable/results.csv`:

- Solved count: 3 -> 4
- PAR-2: `1567.314` -> `1258.395`
- Delta: `-308.919`
- Newly solved: `battleship-16-31-sat`, `mp1-Nb7T46`
- New timeout: `SC25_Timetable_C_392`
- Notable speedup: `544707209399nw.shuffled-as.sat03-1671`, `86.827s` -> `32.357s`

Promotion decision:

- Keep `SAT_VMTF=on` as opt-in SmokeSafe functionality.
- Do not promote VMTF into default or fast profiles yet because the focused/stable candidate still
  has search-core status churn and a lost solved row.
