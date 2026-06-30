# Solver 12 Feature Maturity Ledger

This ledger is the human-facing companion to `FEATURES.csv`.  `SolverConfig`
prints the same maturity records in `SAT_CONFIG_DUMP`, writes them into
`SAT_CONFIG_OUT` replay files, includes them in `config_hash`, and emits them in
`c JSON_STATS ...` when `SAT_STATS_JSON=on`.

> ⚠️ **Search-feature efficacy is under fresh re-evaluation (2026-05-29, bead `SAT-playground-gbc`).**
> The `Current maturity` / `Validation artifact` / `Notes` efficacy claims for search features below
> are STALE — the corresponding rows in `FEATURES.csv` are reset to `ReevalPending`. The prior ledger
> and README validation tables were archived to `archive/efficacy-reeval-2026-05-29/`; **do not consult
> the archived material unless explicitly asked.**

| Feature flag | Config field | Current maturity | Enabled profiles | Validation artifact | Notes |
|---|---|---|---|---|---|
| `SAT_USE_LBD` | `use_lbd` | SmokeSafe | none | `log/phase1/5b2.2.52-s11-single-lbd-clean` | Stable learned-clause metadata records true LBD/glue but remains opt-in after the default focused-stable promotion regressed the solver 10 profiling baseline. |
| `SAT_LBD_UPDATE_REASONS` | `update_reason_lbd` | SmokeSafe | none | `log/1.14h/summary.md` | Optional conflict-analysis reason-side LBD improvement; requires `SAT_USE_LBD=on` and remains default-off. |
| `SAT_LBD_UPDATE_PROP_REASONS` | `update_propagation_reason_lbd` | Experimental | none | `log/1.14h/summary.md` | Optional propagation-time learned reason LBD refresh and lbd-tiered recent-use marking after the implied literal is enqueued. Requires `SAT_LBD_UPDATE_REASONS=on`; isolated after feature-mode profile regression. |
| `SAT_RESTART_REUSE_TRAIL` | `restart_reuse_trail` | Experimental | none | `log/bench-11-kissat-port-2026-05-25-18-43-57/results.csv` | Kissat-style partial restart experiment. Env-facing `SAT_RESTART_REUSE_TRAIL*` requests remain enabled and default-off; stable/focused trail reuse is effective only in focused/stable mode so the single-mode Luby path does not retain the mp1 prefix that previously led to `UNKNOWN`. |
| `SAT_CHRONO` | `chrono_backtrack` | SmokeSafe | none | `log/1.13/summary.md` | Optional guarded chronological backtracking. It only keeps one level (`current - 1`) when the learned clause is still asserting there and falls back to the normal assertion level otherwise. |
| `SAT_BINARY_FAST` | `binary_fast_path` | SmokeSafe | none | `log/analyzesat-2026-05-26-binary-min-otfs/FINDINGS.md` | Opt-in stable binary-clause IDs and implication edges; default remains off until benchmark promotion. Phase 1.11 made clause minimization binary-reason aware; binary-fast env runs now preserve the default recursive minimization unless `SAT_CLAUSE_MIN=off` is explicit, because implicit min-off produced `UNKNOWN` on a baseline-solved Sudoku row. |
| `SAT_CLAUSE_MIN` | `clause_min_mode` | SmokeSafe | default, fast | `log/2026-05-27-08-59-50` | Learned-clause minimization. Default remains recursive-limited; opt-in `inblock` now includes Kissat-style level-block shrink after same-level minimization, replacing a decision-level literal block with one UIP literal when the reason closure proves it. |
| `SAT_VMTF` | `vmtf` | SmokeSafe | none | `log/bench-11-kissat-port-2026-05-25-20-01-30/results.csv` | Variable-Move-To-Front branch queue. `SAT_SEARCH_MODE=focused-stable` defaults to focused-only VMTF unless `SAT_VMTF=off` is explicit; focused conflict bumps preserve Kissat's existing queue-stamp order before moving variables to the front. Focused-mode phase cycling, random decision sequences, and scaled stable reluctant restarts keep the actual path status-safe on the profiling suite. `SAT_VMTF=single` remains the guarded single-mode experimental fallback. |
| `SAT_REORDER` | `reorder` | Experimental | none | `log/phase1/1.14n-summary.md` | Opt-in periodic decision-order rebuild. Stable mode rebuilds the VSIDS heap from current activities; VMTF mode rebuilds the queue in the same activity order. Default remains off until profiling shows a promotion-safe interval/configuration. |
| `SAT_REPHASE` | `rephase` | Experimental | none | `log/bench-11-kissat-port-2026-05-25-18-29-53/results.csv` | Stable-mode rephase hook for focused/stable search experiments. Env-facing requests remain enabled when `SAT_SEARCH_MODE=focused-stable`. |
| `SAT_SEARCH_MODE` | `search_mode_policy` | Experimental | none | `log/bench-11-kissat-port-2026-05-25-20-01-30/results.csv` | Focused/stable mode overlay. Env-facing `focused-stable` requests execute the actual focused/stable path with LBD enabled and focused-only VMTF by default. `SAT_VAR_DECAY_FOCUSED` and `SAT_VAR_DECAY_STABLE` tune only this path; defaults preserve the legacy 0.95 variable decay until a slower stable-mode decay passes the profiling gate. |
| `SAT_MODE_USE_TICKS` | `mode_use_ticks` | Experimental | none | `log/bench-11-kissat-port-2026-05-25-20-01-30/results.csv` | Kissat-style focused/stable mode scheduling. Env-facing requests remain enabled with focused/stable search; conflict-triggered switches now run at the post-propagation scheduling boundary. |
| `SAT_LUCKY` | `lucky` | SmokeSafe | none | `log/phase1/3fs-lucky-off-default-profile/results.csv` | Pre-search lucky assignment pass. Default and fast profiles leave it off after the lucky-on rerun solved only the battleship row while adding time elsewhere; `SAT_LUCKY=on` remains the explicit opt-in for all-true/all-false, forward/backward false/true temporary propagation probes, and bounded small-formula local repair. |
| `SAT_OTFS` | `otfs` | Experimental | none | `log/phase1/1.14g-otfs-summary.md` | Optional bounded learned-clause-only subsumption after learning. It now checks a Kissat-style four-clause recent learned window instead of scanning watcher lists globally, and deletes only when LBD metadata shows the new clause is better; default remains off after enabled profile regressions. |
| `SAT_SIMPLIFICATION` | `simplification` | SmokeSafe | default, fast | `solver/12-kissat-inprocessing/BASELINE_LOCK.raw.txt` | Legacy solver-10 preprocessing umbrella retained for compatibility. |
| `SAT_BVE` | `bve` | SmokeSafe | default, fast | `solver/12-kissat-inprocessing/BASELINE_LOCK.raw.txt` | Existing bounded variable elimination switch. |
| `SAT_FULL_BSR` | `full_bsr` | SmokeSafe | default, fast | `solver/12-kissat-inprocessing/BASELINE_LOCK.raw.txt` | Existing full backward-subsumption switch. |
| `SAT_INPROCESS` | `inprocess` | ParkingLot | none |  | Config-reserved until inprocessing scheduling lands. |
| `SAT_VIVIFY` | `vivify` | ParkingLot | none |  | Config-reserved until vivification lands. |
| `SAT_PROBE` | `probe` | ParkingLot | none |  | Config-reserved until probing lands. |
| `SAT_HBR` | `hbr` | ParkingLot | none |  | Config-reserved until HBR lands; requires probing. |
| `SAT_TRANSITIVE` | `transitive` | ParkingLot | none |  | Config-reserved until transitive reduction lands. |
| `SAT_FORWARD_SUBSUME` | `forward_subsume` | ParkingLot | none |  | Config-reserved until forward subsumption lands. |
| `SAT_GATE_EXTRACT` | `gate_extract` | Experimental | none | `log/seedgate-s12_gate_bve-2026-06-29-09-22-12` | AND/OR gate detection over a pivot's clauses (`x <-> OR(o1..ok)`: base clause + binaries). Enables `SAT_GATE_BVE`. Default-off. |
| `SAT_GATE_BVE` | `gate_bve` | Experimental | none | `log/seedgate-s12_gate_bve-2026-06-29-09-22-12` | Gate-aware bounded variable elimination: when the pivot is gate-defined, resolution is restricted to gate-vs-nongate pairs (Plaisted-Greenbaum), so gate vars eliminate where naive all-pairs BVE blows past the resolvent bound. Sound (DRAT-verified on a large UNSAT, 0 RAT lemmas) and a reusable building block, but **rejected for the default**: as a one-shot root pass on profile20 it nets −1 (sudoku derailed; the 0.6–2% clause reduction was too weak to crack the timing-out targets VexRiscv/Bubble/tseitin). Requires `SAT_GATE_EXTRACT=on`. See bead `SAT-playground-2ro`. |
| `SAT_RCHECK` | `rcheck` | ParkingLot | none |  | Config-reserved until implied-clause checking lands. |
| `SAT_GAUSS` | `gauss` | SmokeSafe | default, fast | `log/seedgate-s12_gauss-2026-06-30-08-18-10` | XOR/parity Gaussian elimination over GF(2). Extracts XOR constraints from the CNF (a degree-k XOR is its 2^(k-1) clause group) and, when they cover ≥90% of the formula (parity-structured, e.g. Tseitin), refutes at the root via Gaussian elimination, emitting a **pure-resolution DRAT proof** (drat-trim VERIFIED, 0 RAT lemmas). Promoted to default/fast: profile20 5×5/900s **+5 solved (76→81)** — `tseitin_grid_n12` 0/5→5/5, which CDCL cannot refute in polynomial resolution — with **byte-identical conflicts** on all 76 prior-solved cells (coverage-gated, trajectory-neutral). Sound: no false UNSAT (case9/div fall through). Bead `SAT-playground-qld`. |

ParkingLot entries are deliberately accepted as schema fields but rejected when
enabled by the 0.3 runtime validator until the owning implementation bead lands.
That prevents future benchmark artifacts from recording no-op feature flags as
though the solver actually used them.
