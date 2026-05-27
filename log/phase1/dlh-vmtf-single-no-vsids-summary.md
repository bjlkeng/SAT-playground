# SAT-playground-dlh: single-mode VMTF no longer double-bumps VSIDS

## Change

`bump_analyzed_variable_activity` now treats active single-mode VMTF like the
kissat focused VMTF path: analyzed variables are moved in the VMTF queue, but
VSIDS scores are not bumped during the VMTF prefix. Stable mode still bumps
VSIDS scores when VMTF is inactive.

The regression test
`test_vmtf_single_mode_conflict_bump_updates_queue` now asserts that single-mode
VMTF moves the queue without changing the variable activity or activity
increment.

## Verification

- Red test before implementation:
  - `cargo test test_vmtf_single_mode_conflict_bump_updates_queue`
  - failed on `assert!(!bumped_scores)`
- Focused test after implementation:
  - `cargo test vmtf`
  - 18 passed
- Full tests:
  - `cargo test`
  - 381 unit tests + 3 `config_cli` tests passed
- Smoke:
  - `bash tools/smoke_test.sh solver/11-kissat-port`
  - 9/9 passed

## Profile

Before:

- `log/phase1/nextbeads-2026-05-27-before/results.csv`
- 10/10 solved, PAR-2 `841.149`

After:

- `log/phase1/dlh-vmtf-single-no-vsids-profile-after/results.csv`
- `log/phase1/dlh-vmtf-single-no-vsids-profile-after/stats.jsonl`
- 10/10 solved, PAR-2 `810.048`

`tools/compare_bench.py --before log/phase1/nextbeads-2026-05-27-before/results.csv --after log/phase1/dlh-vmtf-single-no-vsids-profile-after/results.csv --timeout 300`
reported:

- `status_changes=[]`
- `status_regressions=[]`
- `PAR2_delta=-31.101`
- `verdict=PASS`

The JSON stats had the same config hash (`146977fbb156cfb0`) and identical
`conflicts`, `decisions`, `propagations`, and `restarts` on every row. This is
therefore not a search-path change in the profiling suite. Treat the measured
PAR-2 improvement as either reduced per-conflict execution overhead or ordinary
run-to-run timing noise; do not use it as evidence of a better trajectory.
