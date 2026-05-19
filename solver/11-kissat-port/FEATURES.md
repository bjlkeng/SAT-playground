# Solver 11 Feature Maturity Ledger

This ledger is the human-facing companion to `FEATURES.csv`.  `SolverConfig`
prints the same maturity records in `SAT_CONFIG_DUMP`, writes them into
`SAT_CONFIG_OUT` replay files, includes them in `config_hash`, and emits them in
`c JSON_STATS ...` when `SAT_STATS_JSON=on`.

| Feature flag | Config field | Current maturity | Enabled profiles | Validation artifact | Notes |
|---|---|---|---|---|---|
| `SAT_USE_LBD` | `use_lbd` | SmokeSafe | none | `log/0.0b/findings.md` | Thin-slice LBD side-table and counters only; full LBD policy remains future work. |
| `SAT_LBD_UPDATE_REASONS` | `update_reason_lbd` | ParkingLot | none |  | Config-reserved until reason-LBD updates land. |
| `SAT_CHRONO` | `chrono_backtrack` | ParkingLot | none |  | Config-reserved until chronological backtracking lands. |
| `SAT_BINARY_FAST` | `binary_fast_path` | ParkingLot | none |  | Config-reserved until binary implication fast path lands. |
| `SAT_VMTF` | `vmtf` | ParkingLot | none |  | Config-reserved until the decision-heap cleanup and VMTF work land. |
| `SAT_REPHASE` | `rephase` | ParkingLot | none |  | Config-reserved until rephase policy lands. |
| `SAT_SIMPLIFICATION` | `simplification` | SmokeSafe | default, fast | `solver/11-kissat-port/BASELINE_LOCK.raw.txt` | Legacy solver-10 preprocessing umbrella retained for compatibility. |
| `SAT_BVE` | `bve` | SmokeSafe | default, fast | `solver/11-kissat-port/BASELINE_LOCK.raw.txt` | Existing bounded variable elimination switch. |
| `SAT_FULL_BSR` | `full_bsr` | SmokeSafe | default, fast | `solver/11-kissat-port/BASELINE_LOCK.raw.txt` | Existing full backward-subsumption switch. |
| `SAT_INPROCESS` | `inprocess` | ParkingLot | none |  | Config-reserved until inprocessing scheduling lands. |
| `SAT_VIVIFY` | `vivify` | ParkingLot | none |  | Config-reserved until vivification lands. |
| `SAT_PROBE` | `probe` | ParkingLot | none |  | Config-reserved until probing lands. |
| `SAT_HBR` | `hbr` | ParkingLot | none |  | Config-reserved until HBR lands; requires probing. |
| `SAT_TRANSITIVE` | `transitive` | ParkingLot | none |  | Config-reserved until transitive reduction lands. |
| `SAT_FORWARD_SUBSUME` | `forward_subsume` | ParkingLot | none |  | Config-reserved until forward subsumption lands. |
| `SAT_GATE_EXTRACT` | `gate_extract` | ParkingLot | none |  | Config-reserved until gate extraction lands. |
| `SAT_GATE_BVE` | `gate_bve` | ParkingLot | none |  | Config-reserved until gate-aware BVE lands; requires gate extraction. |
| `SAT_RCHECK` | `rcheck` | ParkingLot | none |  | Config-reserved until implied-clause checking lands. |

ParkingLot entries are deliberately accepted as schema fields but rejected when
enabled by the 0.3 runtime validator until the owning implementation bead lands.
That prevents future benchmark artifacts from recording no-op feature flags as
though the solver actually used them.
