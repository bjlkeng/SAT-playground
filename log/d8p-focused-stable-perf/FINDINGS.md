# d8p — Focused/stable throughput regression: localization — 2026-05-28

**Bead:** `SAT-playground-d8p` (P1). **Agent:** slate-heron via /nextbeads.
**Question:** decompose the focused/stable throughput regression (1.16 matrix:
props/s 4,649,873 default → 1,222,290 focused/VMTF, −74%) and prioritize the
1.14 throughput subtasks (`18.7/18.4/18.9/18.13`) by *measured* impact.

## Executive summary

- **The focused/stable regression is a SEARCH-TRAJECTORY / clause-DB-quality
  problem, NOT per-event code overhead.** The 1.14 throughput subtasks
  (`record_search_ticks`/18.7, `compute_lbd`+LearnedMeta/18.4+18.9,
  `bump_analyzed_variable_activity`/18.13) **do not appear in the profile** —
  they are inlined or <1%. Implementing them will **not** recover the regression.
- This **refutes the premise of `5b2.2.46`** (that the 1.16 comparison was unfair
  because the throughput subset hadn't landed) and **confirms the original
  `5b2.2.30` verdict**: "search-trajectory / inprocessing-dependent."
- It also corrects the triage applied on 2026-05-28 (which reprioritized
  `18.7/18.4/18.9/18.13` **up** to P2 and pointed P0 `5b2.2.53` at `5b2.2.46`)
  — that triage was based on `5b2.2.46`'s now-falsified hypothesis. Corrected here.

## Evidence 1 — symbol profile (perf record, Sudoku, ~80s steady-state)

`perf record -F 1999`, `--sort symbol`. Default vs focused/VMTF
(`SAT_SEARCH_MODE=focused-stable SAT_USE_LBD=on SAT_MODE_USE_TICKS=on
SAT_REDUCE_POLICY=lbd-tiered`):

| Symbol | default | focused/VMTF |
|---|---|---|
| `Solver::propagate` | 76.40% | 72.66% |
| `Solver::backtrack` | 9.31% | 8.13% |
| `push_branch_var_if_decision` | 4.17% | **7.38%** |
| `solve_status_with_proof` | 1.33% | 2.74% |
| `minimize_learned_clause` | 0.61% | 0.77% |
| `analyze_conflict_to_scratch_impl` | 0.56% | (0.30% mark_clause_literals) |

The profiles are **nearly identical**, both `propagate`-dominated. **No
`record_search_ticks`, `compute_lbd_for_clause`, `maybe_improve_lbd`,
`vmtf_stamp_analyzed_var`, `rebuild_branch_queue`, or `maybe_switch_search_mode`
symbol appears in either** (all inlined / below the 0.11% cutoff). The only
material delta is `push_branch_var_if_decision` (+3.2pp) — VMTF decision-picking,
not any 18.x target. The cost lives *inside* `propagate` (clause-DB shape), which
no per-event micro-opt changes.

## Evidence 2 — work-controlled comparison (equal 200,000-conflict budget, Sudoku)

`SAT_LIMIT_CONFLICTS=200000`, both stop cleanly at 200,001 conflicts (UNKNOWN):

| metric @200k conflicts | default | focused/VMTF | delta |
|---|---|---|---|
| search_sec | 146.68 | 195.67 | **+33.4%** |
| propagations | 1,115,141,515 | 1,296,896,392 | +16.3% |
| props / conflict | 5,576 | 6,484 | **+16.3%** |
| props / s | 7.60M | 6.63M | −12.8% |
| learned clauses | 191,422 | 136,255 | −29% |
| learned lits | 23,162,339 | 22,437,268 | — |
| avg learned size | 121 lits | **165 lits** | +36% |
| avg LBD | n/a | **42.1** | (very high glue) |
| max decision level | **804** | **307,092** | ~380× |
| restarts (luby/glucose/reluctant) | 509 / 0 / 0 | 0 / 554 / 27 | — |

At *identical* work (200k conflicts), focused/VMTF takes **+33% wall** because it
explores a **pathologically deep** (max level 307,092 vs 804), **high-glue**
(avg LBD 42, +36% bigger clauses) trajectory, doing **+16% more propagations per
conflict** (longer trails → longer BCP). props/s is only −13% — the bulk of the
wall penalty is doing *more work per conflict*, not slower per-event execution.

Note: the 1.16 "guard100" config showed the opposite restart extreme (163,738
reluctant restarts) where this `lbd-tiered` config shows a deep-dive
(under-restarting, 27 reluctant). **Both are search-policy pathologies** (restart
cadence wrong in opposite directions by config) — neither is per-event overhead.
The `max_decision_level=307,092` value is extreme enough to also warrant a stat
sanity-check, but `avg_lbd=42` + the +16% props/conflict independently confirm a
deep/high-level trajectory regardless of the exact max-level figure.

## Conclusion / prioritization (d8p's mandate)

- **`18.7/18.4/18.9/18.13`: NOT the lever for focused/stable.** Measured impact on
  the profile is ~0 (inlined/<1%). Reprioritize back to P3 (undo the 2026-05-28
  bump). They remain minor absolute-throughput cleanups, not focused/stable fixes.
- **`5b2.2.46`: premise refuted.** Re-running the matrix after the throughput
  subset lands will not change the verdict, because the overhead was never the
  cause. Downgrade and recommend re-scope/close.
- **Real lever = search policy/quality in focused mode** (restart cadence, VMTF
  decision diving, clause-DB glue) — Phase-2 / inprocessing territory, as
  `5b2.2.30` concluded. Opened a new investigation bead for the trajectory
  pathology (deep dive + high LBD).
- **P0 `5b2.2.53` (default decision):** the feature default is gated on the
  search-quality fix, not the throughput subset. Near-term: ship the single-mode
  default (~7% slower than solver10, accepted in `5b2.2.56`).

## Artifacts

- This doc; `run_d8p.sh`; `report_default.txt` / `report_fvmtf.txt`
  (perf symbol breakdowns); `json_default_200k.txt` / `json_fvmtf_200k.txt`
  (work-controlled JSON); `rec_*.data` (perf.data).
