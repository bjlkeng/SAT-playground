# Solver 11 Feature Maturity Ledger

This ledger is the human-facing companion to `FEATURES.csv`.  `SolverConfig`
prints the same maturity records in `SAT_CONFIG_DUMP`, writes them into
`SAT_CONFIG_OUT` replay files, includes them in `config_hash`, and emits them in
`c JSON_STATS ...` when `SAT_STATS_JSON=on`.

| Feature flag | Config field | Current maturity | Enabled profiles | Validation artifact | Notes |
|---|---|---|---|---|---|
| `SAT_USE_LBD` | `use_lbd` | SmokeSafe | none | `log/phase1/5b2.2.52-s11-single-lbd-clean` | Stable learned-clause metadata records true LBD/glue but remains opt-in after the default focused-stable promotion regressed the solver 10 profiling baseline. |
| `SAT_LBD_UPDATE_REASONS` | `update_reason_lbd` | SmokeSafe | none | `log/1.14h/summary.md` | Optional conflict-analysis reason-side LBD improvement; requires `SAT_USE_LBD=on` and remains default-off. |
| `SAT_LBD_UPDATE_PROP_REASONS` | `update_propagation_reason_lbd` | Experimental | none | `log/1.14h/summary.md` | Optional propagation-time learned reason LBD refresh and lbd-tiered recent-use marking after the implied literal is enqueued. Requires `SAT_LBD_UPDATE_REASONS=on`; isolated after feature-mode profile regression. |
| `SAT_RESTART_REUSE_TRAIL` | `restart_reuse_trail` | Experimental | none | `log/phase1/unknown-cleanup-reuse-stable-after` | Internal Kissat-style partial restart experiment. Env-facing `SAT_RESTART_REUSE_TRAIL*` requests are normalized to off because stable-only reuse turned mp1 into `UNKNOWN`; rerunning that env config now follows the status-safe baseline path. |
| `SAT_CHRONO` | `chrono_backtrack` | SmokeSafe | none | `log/1.13/summary.md` | Optional guarded chronological backtracking. It only keeps one level (`current - 1`) when the learned clause is still asserting there and falls back to the normal assertion level otherwise. |
| `SAT_BINARY_FAST` | `binary_fast_path` | SmokeSafe | none | `log/1.6/summary.md`, `log/1.11/summary.md` | Opt-in stable binary-clause IDs and implication edges; default remains off until benchmark promotion. Phase 1.11 made clause minimization binary-reason aware; binary-fast env runs keep minimization off unless `SAT_CLAUSE_MIN` is explicit. |
| `SAT_VMTF` | `vmtf` | SmokeSafe | none | `log/phase1/unknown-cleanup-focused-vmtf-after` | Variable-Move-To-Front branch queue. Env-facing `focused-only`/`on` requests are normalized to off because the current focused/stable stack produced `UNKNOWN`; `SAT_VMTF=single` remains the guarded single-mode experimental fallback. |
| `SAT_REPHASE` | `rephase` | Experimental | none | `log/phase1/unknown-cleanup-current-auto-after` | Internal stable-mode rephase hook for focused/stable search experiments. Env-facing requests are normalized off while focused/stable search is quarantined. |
| `SAT_SEARCH_MODE` | `search_mode_policy` | Experimental | none | `log/phase1/unknown-cleanup-current-auto-after` | Internal focused/stable mode overlay. Env-facing `focused-stable` requests are normalized to single-mode execution after phase1 runs produced `UNKNOWN` on baseline-solved rows. |
| `SAT_MODE_USE_TICKS` | `mode_use_ticks` | Experimental | none | `log/phase1/unknown-cleanup-current-auto-after` | Internal Kissat-style focused/stable mode scheduling. Env-facing requests are normalized off while focused/stable search is quarantined. |
| `SAT_OTFS` | `otfs` | Experimental | none | `log/phase1/1.14g-otfs-summary.md` | Optional bounded learned-clause-only on-the-fly subsumption after learning. Requires clause minimization and remains default-off after the enabled profile run regressed the profiling suite. |
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
