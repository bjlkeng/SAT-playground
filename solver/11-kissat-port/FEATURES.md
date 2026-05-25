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
| `SAT_RESTART_REUSE_TRAIL` | `restart_reuse_trail` | Experimental | none | `log/phase1/5b2.2.55-reuse-stable-after` | Optional Kissat-style partial restart. Keeps the decision-level prefix whose VSIDS score or VMTF stamp beats the next decision candidate, then backtracks only to that level. `SAT_RESTART_REUSE_TRAIL=on` enables both mode criteria; `SAT_RESTART_REUSE_TRAIL_FOCUSED` and `SAT_RESTART_REUSE_TRAIL_STABLE` can override them independently for focused/stable matrix tests. Stable-only reuse is rejected for the current single-mode default because it turns mp1 into `UNKNOWN`. |
| `SAT_CHRONO` | `chrono_backtrack` | SmokeSafe | none | `log/1.13/summary.md` | Optional guarded chronological backtracking. It only keeps one level (`current - 1`) when the learned clause is still asserting there and falls back to the normal assertion level otherwise. |
| `SAT_BINARY_FAST` | `binary_fast_path` | SmokeSafe | none | `log/1.6/summary.md`, `log/1.11/summary.md` | Opt-in stable binary-clause IDs and implication edges; default remains off until benchmark promotion. Phase 1.11 made clause minimization binary-reason aware; binary-fast env runs keep minimization off unless `SAT_CLAUSE_MIN` is explicit. |
| `SAT_VMTF` | `vmtf` | SmokeSafe | none | `log/phase1/egy-vmtf-single-formulaguard-profile` | Optional Variable-Move-To-Front branch queue. The Kissat-faithful mode is `SAT_VMTF=focused-only` with focused/stable search: focused mode uses queue decisions and move-to-front bumps without VSIDS score bumps, while stable mode uses the VSIDS heap. `SAT_VMTF=single` is a guarded experimental fallback, not a promoted policy. |
| `SAT_REPHASE` | `rephase` | SmokeSafe | none | `log/1.12/summary.md` | Opt-in stable-mode rephase hook for focused/stable search experiments. Requires `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable`; it cycles saved phase data through best, inverted, and original phases on scheduled stable-mode restarts and starts a new target-phase block. |
| `SAT_SEARCH_MODE` | `search_mode_policy` | SmokeSafe | none | `log/phase1/5b2.2.52-s11-default-clean` | `focused-stable` overlays `SAT_PHASE`: focused mode maps `legacy`/`saved` to `saved` and target/best-target inputs to `target-then-saved`; stable mode uses `best-then-target-then-saved`. `SAT_FOCUSED_PHASE` and `SAT_STABLE_PHASE` override those effective policies for matrix tests. Target/best phase snapshots are captured only in stable mode. It remains opt-in until it beats the solver 10 baseline. |
| `SAT_MODE_USE_TICKS` | `mode_use_ticks` | SmokeSafe | none | `log/phase1/5b2.2.52-s11-focused-noticks-clean` | Optional Kissat-style focused/stable mode scheduling. Focused mode still gates on conflicts with Kissat `nlogpown(count, 4)` mode-interval growth; stable mode gates on propagation search ticks. Requires `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable`. |
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
