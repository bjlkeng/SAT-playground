# SAT-playground-bs6 Memory Abort Follow-Up

Date: 2026-05-20
Bead: `SAT-playground-bs6`

## Finding

The two M.2 normalised-instance failures were process aborts under the 16 GB
benchmark gate, not SAT/UNSAT/proof/model failures.

- `ee5fb3e181571740adb8444bd052dbdc-11.normalised` declares
  `p cnf 53940610 145119077`. Its reproduced allocation failure was
  `2589149280` bytes, exactly `2 * num_vars * size_of::<Vec<Watcher>>()`.
  That identifies the immediate failure as the dense watcher header vector
  allocation in `Solver::new_with_config`.
- `83aa254f7d17e1df7bee19322ac4752b-1.normalised` declares
  `p cnf 29302780 78831317`. It reproduced as a later `memory allocation of
  128 bytes failed`; after the first guard caught only the dense watcher-header
  case, this still aborted during the later preprocessing peak. The final guard
  therefore also accounts for predictable occurrence entries and
  inline-abstraction rebuild peak memory.

## Fix

Solver 11 now performs a pre-solve memory admission check after parsing the CNF
and before constructing dense solver state. The guard combines:

- explicit `SAT_LIMIT_RSS_MB`, when set
- the process address-space limit from `/proc/self/limits`, which is how
  `tools/bench.sh -m` applies the standard memory gate
- the current post-parse high-water RSS
- an estimate of mandatory solver construction and one-shot preprocessing peak
  allocations

If the estimated peak reaches 90% of the effective cap, the solver writes a
complete `UNKNOWN` result contract with `unknown_reason=memory-preflight-limit`
instead of entering an allocation path that can abort before `result.json`.

## Reproduction And Validation

Before:

| Instance | Log | Result |
|---|---|---|
| ee5 | `log/bs6/repro-ee5-before/results.csv` | `ERROR`, exit 134, missing `result.json`, allocation of `2589149280` bytes failed |
| 83aa | `log/bs6/repro-83aa-before/results.csv` | `ERROR`, exit 134, missing `result.json`, allocation of `128` bytes failed |

After final code:

| Instance | Log | Result |
|---|---|---|
| ee5 | `log/bs6/repro-ee5-after2/results.csv` | `UNKNOWN`, no harness error, 26.91s |
| 83aa | `log/bs6/repro-83aa-after2/results.csv` | `UNKNOWN`, no harness error, 14.29s |

Profile no-regression gate:

```text
before=log/profile-compare-solver11-2026-05-19-1647/results.csv
after=log/bs6/profile-after/results.csv
timeout_s=120
status_changes=[]
status_regressions=[]
raw_status_counts_match=true
PAR2_before=710.454
PAR2_after=712.548
PAR2_delta=2.094
solved_before=9
solved_after=9
verdict=PASS
```

The profile drift is a single-run timing delta with unchanged status counts:
9 solved, 2 timeouts, 0 unknown, 0 errors.

## Checks

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test` (163 tests)
- `bash tools/smoke_test.sh solver/11-kissat-port` (9/9)
- `SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port` (9/9)
