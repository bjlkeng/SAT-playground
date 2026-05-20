# Solver 11 Phase 1.8 - Core search default candidate milestone

Date: 2026-05-20

Bead: `SAT-playground-5b2.2.11` (`[1.8] Core search default candidate milestone`)

## Scope

This milestone evaluated the already-implemented Phase 1.1 through 1.7 search
features as replayable candidate configurations. No solver code or default
profile behavior was changed.

The tested candidates were:

- Conservative config hash `024e0e9587682af1`:
  `SAT_USE_LBD=on`, `SAT_LBD_UPDATE_REASONS=off`,
  `SAT_RESTART=kissat-ema`, `SAT_REDUCE=lbd-tiered`,
  `SAT_PHASE=saved`, `SAT_BINARY_FAST=off`, `SAT_CLAUSE_MIN=off`.
- Strong config hash `c32d0e3dbd78a31b`:
  `SAT_USE_LBD=on`, `SAT_LBD_UPDATE_REASONS=on`,
  `SAT_RESTART=kissat-ema`, `SAT_REDUCE=lbd-tiered`,
  `SAT_PHASE=target-then-saved`, `SAT_BINARY_FAST=on`,
  `SAT_CLAUSE_MIN=off`.
- Exploratory config hash `c32d0e3dbd78a31b`.
  The plan currently makes exploratory identical to strong, so the strong
  benchmark result is the exploratory result too. A separate exploratory replay
  config was still materialized for traceability.

## Candidate Artifacts

Replay/config artifacts:

- `log/1.8/candidates/candidate-conservative.config`
- `log/1.8/candidates/candidate-conservative.smoke-plus.config`
- `log/1.8/candidates/candidate-conservative.search-core.config`
- `log/1.8/candidates/candidate-strong.config`
- `log/1.8/candidates/candidate-strong.smoke-plus.config`
- `log/1.8/candidates/candidate-strong.search-core.config`
- `log/1.8/candidates/candidate-exploratory.config`

Validation and benchmark logs:

- Conservative smoke: `log/2026-05-20-18-26-49`
- Strong smoke: `log/2026-05-20-18-26-56`
- Conservative smoke-plus: `log/1.8/smoke-plus-conservative/results.csv`
- Strong smoke-plus: `log/1.8/smoke-plus-strong/results.csv`
- Conservative search-core: `log/1.8/search-core-conservative/results.csv`
- Strong/exploratory search-core: `log/1.8/search-core-strong/results.csv`
- Final default profile: `log/1.8/profile-default-final/results.csv`
- Comparisons:
  - `log/1.8/compare/search-core-conservative-vs-saved.txt`
  - `log/1.8/compare/search-core-strong-vs-saved.txt`
  - `log/1.8/compare/profile-default-vs-1.6.txt`

## Validation Results

Commands run:

```bash
SAT_PROFILE=experimental ...candidate conservative... bash tools/smoke_test.sh solver/11-kissat-port
SAT_PROFILE=experimental ...candidate strong... bash tools/smoke_test.sh solver/11-kissat-port
cargo test bruteforce -- --nocapture
SAT_PROFILE=experimental ...candidate conservative... bash tools/bench.sh -t 120 -m 16384 -d benchmarks/iteration/smoke-plus --log-dir log/1.8/smoke-plus-conservative solver/11-kissat-port
SAT_PROFILE=experimental ...candidate strong... bash tools/bench.sh -t 120 -m 16384 -d benchmarks/iteration/smoke-plus --log-dir log/1.8/smoke-plus-strong solver/11-kissat-port
SAT_PROFILE=experimental ...candidate conservative... bash tools/bench.sh -t 120 -m 16384 -d benchmarks/iteration/search-core --log-dir log/1.8/search-core-conservative solver/11-kissat-port
SAT_PROFILE=experimental ...candidate strong... bash tools/bench.sh -t 120 -m 16384 -d benchmarks/iteration/search-core --log-dir log/1.8/search-core-strong solver/11-kissat-port
bash tools/bench.sh -t 120 -m 16384 -d benchmarks/profiling --log-dir log/1.8/profile-default-final solver/11-kissat-port
python3 tools/compare_bench.py --before log/1.6/profile-after-final/results.csv --after log/1.8/profile-default-final/results.csv --timeout 120
cargo test -- --nocapture
```

Results:

- Conservative smoke: 9/9 passed, UNSAT proofs verified.
- Strong smoke: 9/9 passed, UNSAT proofs verified.
- Brute-force oracle tests: 4 passed.
- Full unit test suite: 215 passed.
- Conservative smoke-plus: 9 solved / 9, PAR-2 `0.061`.
- Strong smoke-plus: 9 solved / 9, PAR-2 `0.059`.

Search-core candidate results:

- Prior saved-phase reference `log/1.5/search-core-saved/results.csv`:
  5 solved / 9, PAR-2 `1188.569`.
- Conservative:
  1 solved / 9, PAR-2 `1960.600`, compare verdict `FAIL`.
  It solved `544707209399nw.shuffled-as.sat03-1671` faster than the saved
  reference, but regressed four previously solved instances to TIMEOUT:
  `SC25_Timetable_C_392`, `SC25_Timetable_C_406`,
  `battleship-16-31-sat`, and `mp1-Nb7T46`.
- Strong/exploratory:
  1 solved / 9, PAR-2 `2023.940`, compare verdict `FAIL`.
  It also regressed the same four previously solved instances to TIMEOUT and
  was slower than conservative on `544707209399nw.shuffled-as.sat03-1671`.

Default profile no-regression result:

- Current default profile: 9 solved / 11, PAR-2 `711.492`.
- Prior accepted profile from 1.6: 9 solved / 11, PAR-2 `711.619`.
- Compare verdict: `PASS`, no status regressions, PAR-2 delta `-0.127`.

## Decision

No Phase 1.8 candidate is promoted.

The conservative and strong/exploratory compositions pass proof/model smoke,
oracle, and smoke-plus gates, but both fail the search-core promotion rule by
losing four previously solved search-core instances. Keeping the current
default profile is the correct result for this milestone.

The useful finding is narrow: `544707209399nw.shuffled-as.sat03-1671` benefits
from the conservative composition, while timetable, battleship, and mp1
instances regress badly. Later Phase 1 work should treat these as search-path
sensitivity signals rather than assuming the composed LBD/EMA/reduce/phase
stack is globally better.
