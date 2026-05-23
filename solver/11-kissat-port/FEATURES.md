# Solver 11 Feature Maturity Ledger

This ledger is the human-facing companion to `FEATURES.csv`.  `SolverConfig`
prints the same maturity records in `SAT_CONFIG_DUMP`, writes them into
`SAT_CONFIG_OUT` replay files, includes them in `config_hash`, and emits them in
`c JSON_STATS ...` when `SAT_STATS_JSON=on`.

| Feature flag | Config field | Current maturity | Enabled profiles | Validation artifact | Notes |
|---|---|---|---|---|---|
| `SAT_USE_LBD` | `use_lbd` | SmokeSafe | none | `log/0.0b/findings.md` | Stable learned-clause metadata records true LBD/glue while policy defaults remain unchanged. |
| `SAT_LBD_UPDATE_REASONS` | `update_reason_lbd` | SmokeSafe | none | `log/1.14h/summary.md` | Optional learned reason-side LBD improvement; requires `SAT_USE_LBD=on`, runs after successful propagation-reason enqueue so the implied literal's level is included, and remains default-off. |
| `SAT_CHRONO` | `chrono_backtrack` | SmokeSafe | none | `log/1.13/summary.md` | Optional guarded chronological backtracking. It only keeps one level (`current - 1`) when the learned clause is still asserting there and falls back to the normal assertion level otherwise. |
| `SAT_BINARY_FAST` | `binary_fast_path` | SmokeSafe | none | `log/1.6/summary.md`, `log/1.11/summary.md` | Opt-in stable binary-clause IDs and implication edges; default remains off until benchmark promotion. Phase 1.11 made clause minimization binary-reason aware; binary-fast env runs keep minimization off unless `SAT_CLAUSE_MIN` is explicit. |
| `SAT_VMTF` | `vmtf` | SmokeSafe | none | `log/1.10/summary.md` | Optional focused-mode Variable-Move-To-Front branch queue. Requires `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable`; stable mode continues to use the VSIDS heap. |
| `SAT_REPHASE` | `rephase` | SmokeSafe | none | `log/1.12/summary.md` | Opt-in stable-mode rephase hook for focused/stable search experiments. Requires `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable`; it cycles saved phase data through best, inverted, and original phases on scheduled stable-mode restarts. |
| `SAT_MODE_USE_TICKS` | `mode_use_ticks` | SmokeSafe | none | `log/1.14/mode-use-ticks.md` | Opt-in Kissat-style focused/stable mode scheduling. Focused mode still gates on conflicts, but its interval uses Kissat `nlogpown(count, 4)` growth; stable mode gates on propagation search ticks rather than conflicts. Requires `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable`. |
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
