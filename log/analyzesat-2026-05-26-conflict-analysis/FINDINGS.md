# Bottleneck Analysis - solver/11-kissat-port - conflict analysis

**Worktree:** `/tmp/analyzesat-20260526-184922` on branch
`side-01-analyzesat-20260526-184922`, HEAD `e7ec1f8`.
**Slug dir:** `log/analyzesat-2026-05-26-conflict-analysis/`.
**Fresh area:** conflict-analysis traversal, specifically
`SAT_CONFLICT_ANALYSIS_MODE=resolved` versus default MiniSat-style analysis.

This is deliberately separate from the previous AnalyzeSAT passes over clause
minimization, binary fast path, OTFS, branching/phase policy, preprocessing,
proof logging, initial clause order, LBD reducer policy, and EMA restarts.

## Method

Ran a 6-config ablation over `benchmarks/profiling/` at 300s and 16 GiB with
`SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295`:

| config | env |
|---|---|
| `A_baseline` | defaults |
| `B_resolved` | `SAT_CONFLICT_ANALYSIS_MODE=resolved` |
| `C_lbd_metadata` | `SAT_USE_LBD=on` |
| `D_lbd_resolved` | `SAT_USE_LBD=on SAT_CONFLICT_ANALYSIS_MODE=resolved` |
| `E_focused_stable` | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable` |
| `F_focused_stable_resolved` | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_CONFLICT_ANALYSIS_MODE=resolved` |

Artifacts:

* `run_conflict_ablation.sh` - benchmark driver.
* `analyze_conflict_ablation.py` - matrix, decomposition, and paired comparison.
* `matrix.csv` - per-row status, wall time, search counters, preprocessing shape.
* `decomp.csv` - work x speed against `A_baseline`.
* `resolved_pairwise.csv` - direct resolved/non-resolved paired comparison.
* `traces/velev/` - conflict-aligned trace for default vs resolved on velev.
* `perf_attempt.txt` and `perf_attempt_escalated.txt` - perf was blocked by
  `perf_event_paranoid=4`.

## Executive summary

1. **`SAT_CONFLICT_ANALYSIS_MODE=resolved` is same-work on the profiling suite.**
   In every paired row (`B` vs `A`, `D` vs `C`, `F` vs `E`), conflicts,
   decisions, propagations, restarts, and final learned-clause counts are
   identical. The mode does not change the CDCL trajectory on these instances.
2. **The mode is not a promotion candidate.** PAR-2 is worse in all three
   paired sums: `B` 911.5 vs `A` 836.7 (+74.8), `D` 875.6 vs `C` 856.5
   (+19.1), and `F` 778.8 vs `E` 758.9 (+19.9). Some individual traced repeats
   move the other way, but with identical counters; that is timing noise, not a
   search improvement.
3. **The code-level reason is clear.** Default analysis already matches the
   MiniSat model: skip literal position 0 in a reason clause, because the
   propagated literal is stored there. Resolved mode instead scans from position
   0 and filters the just-resolved variable through `scratch_resolved`. With the
   reason-clause invariant holding, both produce the same learned clauses and
   the same bumped-variable set, but the resolved mode keeps an extra hot-loop
   branch.
4. **Focused/stable remains the larger search lever, but it is unrelated to this
   switch.** `E_focused_stable` is the best row at PAR-2 758.9, with the known
   trajectory tradeoff: huge wins on mp1/Kakuro/case9 and losses on 6s,
   REGRandom, SCPC, brocard, and velev. Adding `resolved` to that row does not
   alter the trajectory and worsens PAR-2 to 778.8.
5. **The separate Kissat reason-side activity bead is still the real opportunity
   in this area.** Solver 11 already bumps variables reached during normal 1-UIP
   expansion, but it does not implement Kissat's post-clause reason-side
   expansion controlled by `bumpreasons`, `bumpreasonslimit`, and
   `bumpreasonsrate`.

## PAR-2 results

| config | solved | timeout | unknown | error | PAR-2 |
|---|---:|---:|---:|---:|---:|
| `A_baseline` | 10 | 0 | 0 | 0 | 836.7 |
| `B_resolved` | 10 | 0 | 0 | 0 | 911.5 |
| `C_lbd_metadata` | 10 | 0 | 0 | 0 | 856.5 |
| `D_lbd_resolved` | 10 | 0 | 0 | 0 | 875.6 |
| `E_focused_stable` | 10 | 0 | 0 | 0 | 758.9 |
| `F_focused_stable_resolved` | 10 | 0 | 0 | 0 | 778.8 |

No benchmark row produced `UNKNOWN`, `TIMEOUT`, or `ERROR`. SAT rows validated
models with `tools/verify_sat.py`. UNSAT proof files were produced, but DRAT
validation was skipped because `drat-trim` is not installed.

## Same-work evidence

Every pair below had `same_work=yes` for all 10 instances in
`resolved_pairwise.csv`.

| pair | wall sum base | wall sum resolved | delta | geometric wall ratio | same work |
|---|---:|---:|---:|---:|---:|
| `B_resolved` vs `A_baseline` | 836.654 | 911.499 | +74.845 | 1.070 | 10/10 |
| `D_lbd_resolved` vs `C_lbd_metadata` | 856.485 | 875.569 | +19.084 | 1.004 | 10/10 |
| `F_focused_stable_resolved` vs `E_focused_stable` | 758.911 | 778.778 | +19.867 | 0.967 | 10/10 |

The `F` geometric wall ratio is below 1.0 even though PAR-2 worsened because
most rows were slightly faster in that run, while sudoku alone regressed from
239.014s to 285.884s. Since work counters are identical, this is timing noise
or execution-layout noise, not a search-path difference.

## Critical trace

The velev row was traced with `SAT_TRACE_SEARCH_INTERVAL=20000` under default
and resolved mode. Both traces hit the same state at every conflict checkpoint:

* 20k conflicts: decisions 310,694, propagations 40,400,931, restarts 68.
* 100k conflicts: decisions 1,377,622, propagations 242,102,674, restarts 254.
* Done: conflicts 179,968, decisions 2,576,743, propagations 436,279,112,
  restarts 503.

The full-matrix velev row had `B_resolved` slower (84.486s vs 70.004s). The
trace rerun had `B_resolved` faster (search 43.092s vs 48.317s). The useful
finding is not either wall-time sample; it is the identical conflict-aligned
trajectory.

## Source diff

Solver 11's default path:

* `solver/11-kissat-port/src/main.rs:6470` starts 1-UIP analysis.
* `solver/11-kissat-port/src/main.rs:6502` chooses reason traversal start
  position `1` in default mode.
* `solver/11-kissat-port/src/main.rs:6301` marks reason-clause literals and
  pushes variables into `scratch_bumped_vars`.
* `solver/11-kissat-port/src/main.rs:4173` bumps activity for those analyzed
  variables.

Resolved mode:

* `solver/11-kissat-port/src/main.rs:6493` clears `scratch_seen[var]` and sets
  `scratch_resolved[var]`.
* `solver/11-kissat-port/src/main.rs:6504` scans reason clauses from position
  `0`.
* `solver/11-kissat-port/src/main.rs:6316` and `:6354` add the extra
  `scratch_resolved` skip check in the clause and binary marking loops.
* `solver/11-kissat-port/src/main.rs:6550` clears `scratch_resolved` for
  bumped vars.

This is an alternate encoding of the same invariant, not a distinct algorithmic
feature. MiniSat's public `Solver.cc` uses the same basic convention: reason
clauses skip the propagated literal while walking antecedents. Kissat's separate
reason-side activity bump is different: Debian's Kissat source exposes
`analyze_reason_side_literals`, and the Kissat man page documents
`--bumpreasons`, `--bumpreasonslimit`, and `--bumpreasonsrate` with defaults
enabled, limit 10, and decision-rate limit 10.

Sources:

* MiniSat source: https://github.com/niklasso/minisat/blob/master/minisat/core/Solver.cc
* Kissat analyze source: https://sources.debian.org/src/kissat/4.0.3-2/src/analyze.c/
* Kissat options: https://manpages.debian.org/testing/kissat/kissat.1.en.html

## Beads

* Added evidence note to existing `SAT-playground-5b2.2.37`
  ("Bump reason-side variable activities during conflict analysis").
* Created `SAT-playground-5b2.2.60`
  ("[1.14m] Retire resolved conflict-analysis mode") and linked it as related
  to `SAT-playground-5b2.2.37`.

## Recommendation

Do not promote or tune `SAT_CONFLICT_ANALYSIS_MODE=resolved`. It should either
be removed from the public/internal config surface or kept only as a debug
invariant test that asserts equivalence with default MiniSat-style analysis.

For real conflict-analysis follow-up work, implement
`SAT-playground-5b2.2.37`: a guarded Kissat-style post-clause reason-side
variable activity bump. That is a different mechanism than resolved traversal
and is still unimplemented.
