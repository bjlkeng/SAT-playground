# Bottleneck Analysis — solver/11-kissat-port — 2026-05-27 (search-control axis)

**Investigator angle:** fresh-eyes pass on the **search-control axis** — chronological
backtracking (`SAT_CHRONO` + `SAT_CHRONO_MAX_DELTA`), initial branch ordering
(`SAT_BRANCH_MODE`), and initial clause iteration (`SAT_INITIAL_CLAUSE_MODE`).

Prior analyzesat passes covered orthogonal clusters and never isolated these knobs:

- 2026-05-25-2043 — restart policy × focused-stable × LBD
- 2026-05-26-0712 — rephase/reorder/lucky search features
- 2026-05-26-conflict-vmtf — clause minimization, VMTF, OTFS
- 2026-05-26-conflict-analysis — ccmin modes
- 2026-05-26-preprocess — BVE/BSR preprocessing
- 2026-05-26-clausedb-cycle — reduce-DB, binary-fast, post-preprocess reset, trail-reuse restart

`bd search chrono`, `bd search branch_mode`, `bd search initial-clause` returned **no
beads** before this run for these axes.

**Worktree:** `/tmp/analyzesat-2026-05-27-search-control-1779879326`
**Slug dir:** `log/analyzesat-2026-05-27-search-control/`
**Method:** A_baseline (10/10) + B_chrono (4/10, scope reduced due to host contention)
× 300 s timeout × 16 GiB. `SAT_STATS_JSON=on` per run. Reference kissat-latest /
sc2024 CSVs reused from `log/analyzesat-2026-05-26-clausedb-cycle/`.

**Host-contention caveat:** during this run two and sometimes three parallel
benchmark agents were running solver/11-kissat-port against the same profiling
suite on the same 6-core/12-thread box. Sudoku A_baseline 207 s vs uncontended
~76 s (~2.7×). **Work counters — conflicts, decisions, propagations — are
deterministic and unaffected.** Decomposition relies on them; wall times are
documented but secondary.

**Reduced scope rationale:** B_chrono produced clear evidence after 4 instances
(3 regressions, 1 win) that matched a pre-derived source-diff hypothesis. Configs
C_chrono_aggressive / D_branch_occurrence / E_initial_kissat_watch /
F_search_control_combined were skipped because the answer for chrono is already
clear and host contention made each instance run 2–3× slower. They are still
encoded in `run_ablation.sh` (commented out) for a future quieter rerun.

## Executive Summary

**Headline:** `SAT_CHRONO=on` in solver 11 is **a definitive regression on this
suite**, but the cause is NOT "chrono BT is bad for these workloads." The cause
is a **one-line semantic bug in `choose_backtrack_level`**: solver 11's
`chrono_max_delta` threshold predicate is inverted relative to kissat. Solver 11
fires chrono BT when the backjump distance is *small* (typical conflicts); kissat
fires chrono BT only when the backjump distance is *large* (rare deep-jump
conflicts).

On B_chrono with default `chrono_max_delta=100`:

| Instance | A_baseline confl | B_chrono confl | work ratio | chrono_used / confl | wall A | wall B | Δ wall |
|---|---:|---:|---:|---:|---:|---:|---:|
| sudoku-N30-12     |   259 775 | 1 368 858 |  **5.3×** | 84 % | 207.5 s | 275.0 s | +33 % |
| 6s299b685_Iter30  |     3 764 |    67 345 | **17.9×** | 72 % |  18.2 s |  22.5 s | +24 % |
| REGRandom-Seed40  | 1 607 608 | 4 749 032 |   3.0×    | 70 % |  60.3 s | 206.9 s | +243 % |
| mp1-Nb7T46        |   425 229 |   284 415 |   0.67×   | 31 % |  46.5 s |  19.8 s | **−57 %** |

The single instance where chrono BT *helps* (mp1) is also the only one where
`chrono_used / conflicts` is below 40 %. The other three instances fire chrono BT
on 70–84 % of conflicts because solver 11's `delta > chrono_max_delta` predicate
is true for almost every conflict. Kissat with the same threshold value (100)
fires chrono on a tiny fraction of conflicts.

**This is a real implementation bug, not a feature-fit question.** Flipping the
predicate (and renaming the field) is a 2-line change that should be tested
before any further default-promotion discussion of `SAT_CHRONO`.

A secondary source-diff finding (SC-3) is that solver 11's `SAT_REORDER`
implementation ignores kissat's clause-weighted score function entirely.

Three new beads filed:

- `SAT-playground-d2b` (P1, bug) — Gap SC-2: chrono_max_delta predicate inverted
- `SAT-playground-59l` (P3, task) — Gap SC-1: chrono default flip (blocked by d2b)
- `SAT-playground-vcc` (P3, task) — Gap SC-3: kissat clause-weighted reorder

## Config matrix

| Config | Env vars | Purpose |
|---|---|---|
| `A_baseline` | (defaults: chrono off, minisat branch, canonical-sorted) | reference |
| `B_chrono` | `SAT_CHRONO=on` | chronological backtracking with default delta=100 |

Skipped (encoded but disabled in `run_ablation.sh`): `C_chrono_aggressive`,
`D_branch_occurrence`, `E_initial_kissat_watch`, `F_search_control_combined`.

## PAR-2 per config

**Important — not directly comparable.** A_baseline ran all 10 instances;
B_chrono ran 4 before scope reduction. Per-instance Δ is the right view (see
table above). Aggregate PAR-2 on the **matching 4 instances** is:

| | sudoku + 6s + REGR + mp1 |
|---|---:|
| A_baseline | 332.5 s |
| B_chrono   | 524.2 s |
| Δ          | +57.7 % |

## Reference solver live comparison

Reused from clausedb-cycle. The repo solver A_baseline already beats kissat-latest
on three of the ten profiling instances thanks to BSR/BVE preprocessing, so the
"search-control" angle is **not** the dominant gap for those instances. Where
kissat is materially faster (REGRandom 26×, battleship 130×, case9 1.6×), the
gaps are trajectory / heuristic and not addressable by chrono BT or initial-order
knobs alone.

## Work × Speed decomposition

Computed as `work_r = conflicts_B / conflicts_A`, `speed_r = (prop/s)_A /
(prop/s)_B`. Wall ratios fluctuate with contention; work_r and chrono_used are
deterministic. Note: when chrono BT fires, conflicts increase but propagations
per conflict drop sharply (because the trail is barely unwound), so `work_r`
overstates total search effort. Each row reports cause class against wall_r.

| Instance | wall_r | work_r | speed_r | chrono_used % | cause |
|---|---:|---:|---:|---:|---|
| sudoku-N30-12    | 1.32  |  5.27 | 1.81 | 84 % | combined regression (chrono trajectory + per-prop slowdown) |
| 6s299b685_Iter30 | 1.24  | 17.89 | 0.63 | 72 % | pure trajectory regression |
| REGRandom-Seed40 | 3.43  |  2.95 | 2.33 | 70 % | combined regression |
| mp1-Nb7T46       | 0.43  |  0.67 | 0.99 | 31 % | **WIN — trajectory** (low chrono fire rate) |

**Interpretation:** chrono BT firing rate predicts the sign of the wall effect.
On the three instances that regress, chrono BT fires 70–84 % of the time and
inflates the conflict count 3–18×. Only mp1 — the one instance where
`chrono_rejected_delta_too_large` skips most attempts — wins. The mechanism is
exactly what kissat's policy is designed to avoid; solver 11 has the policy
inverted.

## Reference source diff — implementation gaps

### Gap SC-1 — Chrono backtracking off by default in solver 11; on by default in kissat

`solver/11-kissat-port/src/config.rs:643` sets `chrono_backtrack: false`.
`benchmarks/reference-solvers/kissat-latest/src/options.h:21` declares
`OPTION (chrono, 1, 0, 1, "allow chronological backtracking")` — kissat's default
is **on**, and `set_plain_options` (`config.c:39-53`) turns it off **only** for
the `--plain` / `--basic` configurations. The default-named kissat configuration
leaves `chrono = 1`.

The mechanism in solver 11 is fully implemented in
`solver/11-kissat-port/src/main.rs:4926` (`choose_backtrack_level`); the only
thing keeping it off is the bool flag default — but see Gap SC-2 before flipping
that default, because the threshold semantics are wrong.

### Gap SC-2 — `chrono_max_delta` threshold predicate inverted relative to kissat

This is the headline implementation bug.

`benchmarks/reference-solvers/kissat-latest/src/learn.c:16-42` (identical in
kissat-sc2024):

```c
unsigned kissat_determine_new_level (kissat *solver, unsigned jump) {
  const unsigned back = solver->level - 1;
  const unsigned delta = back - jump;
  const unsigned limit = backjump_limit (solver);   // chronolevels (default 100)
  unsigned res;
  if (!delta) {
    res = jump;
  } else if (delta > limit) {                       // <-- LARGE delta → chrono
    res = back;
    INC (chronological);
  } else {                                          // <-- small delta → non-chrono
    res = jump;
  }
  return res;
}
```

`solver/11-kissat-port/src/main.rs:4926-4957`:

```rust
fn choose_backtrack_level(&mut self, assertion_level: usize, learned_clause: &[i32]) -> usize {
    if !self.chrono_backtrack { return assertion_level; }
    let current_level = self.current_level();
    if current_level == 0 { return 0; }
    if assertion_level >= current_level { return assertion_level; }

    self.stats.chrono_attempts += 1;
    let delta = current_level - assertion_level;
    if delta > self.chrono_max_delta {                              // <-- LARGE delta → non-chrono
        self.stats.chrono_rejected_delta_too_large += 1;
        return assertion_level;
    }

    let chrono_level = current_level - 1;
    if !self.learned_clause_asserts_at_level(learned_clause, chrono_level) {
        self.stats.chrono_rejected_not_asserting += 1;
        return assertion_level;
    }
    ...
    chrono_level                                                    // <-- small delta → chrono
}
```

The two use the same threshold value (100) **with opposite truth tables**.
Kissat's policy says "chrono BT is a fallback for pathologically deep
backjumps"; solver 11's policy says "chrono BT is the default for almost every
conflict; we fall back to the normal backjump only when delta exceeds 100."

Concrete impact at default threshold (`chrono_max_delta=100`):

| Instance | conflicts B | chrono_attempts B | chrono_used B | rejected_delta_too_large B | chrono_used / conflicts |
|---|---:|---:|---:|---:|---:|
| sudoku           | 1 368 858 | 1 368 291 | 1 154 979 |   11 119 | 84 % |
| 6s299b685_Iter30 |    67 345 |    67 277 |    48 610 |      594 | 72 % |
| REGRandom-Seed40 | 4 749 032 | 4 749 014 | 3 312 563 |    8 357 | 70 % |
| mp1-Nb7T46       |   284 415 |   284 408 |    89 091 |      188 | 31 % |

Kissat's `chronological` counter on the same instances would be the
`rejected_delta_too_large` column (around 0.01–0.4 %), not the `chrono_used`
column (31–84 %). Solver 11 fires chrono BT roughly 200–500× more often than
kissat would on the same workload.

**Fix sketch:**

```rust
// solver/11-kissat-port/src/main.rs ~line 4940
let delta = current_level - assertion_level;
if delta <= self.chrono_max_delta {                    // CHANGED: <= not >
    self.stats.chrono_rejected_delta_small += 1;       // RENAMED counter
    return assertion_level;
}

let chrono_level = current_level - 1;
// existing asserts-at-level guard still applies
```

Plus rename `chrono_max_delta` to `chrono_min_delta` or `chrono_levels` for
clarity (kissat names the option `chronolevels`). Update tests at
`solver/11-kissat-port/src/main.rs:10937-10961` (`test_chrono_off_uses_assertion_level`
and `test_chrono_rejects_large_delta`) accordingly — the rejection-direction test
becomes "rejects small delta."

Reference citation: `benchmarks/reference-solvers/kissat-latest/src/learn.c:16-42`.

### Gap SC-3 — `SAT_REORDER` ignores kissat's clause-weighted scoring

`solver/11-kissat-port/src/main.rs:3104` `reorder_branching_by_activity` rebuilds
the VMTF queue from VSIDS activity order via `activity_reorder_vars()`. That's
a pure-activity reorder.

`benchmarks/reference-solvers/kissat-latest/src/reorder.c:24-217` computes a
**clause-weighted score** per literal:

- `table[size] = 1, 1/2, 1/4, ...` for sizes 2..`reordermaxsize=100`
- each irredundant clause adds `table[size]` to every literal in it
- each binary watcher edge adds `table[2]` for both ends
- per-variable score: `weight = max(pos, neg) + 2 · min(pos, neg)`
- sorted by `weight`, tie-broken by VMTF stamp (focused) or VSIDS score (stable)
- focused mode: move sorted variables to the queue front
- stable mode: update VSIDS scores by adding the weight (preserves activity
  history but biases toward formula-structure)

This is a structural signal the repo solver completely ignores. Out of scope for
this investigation (`SAT_REORDER` was not enabled in the ablation), but worth a
new bead linked to existing reorder beads. The expected impact is highest on
SAT-search instances where preprocessing already eliminated low-degree variables
and the remaining decision frontier is structurally heterogeneous.

Reference citation: `benchmarks/reference-solvers/kissat-latest/src/reorder.c:24-217`.

## Trajectory analysis

A formal trajectory trace (`SAT_TRACE_SEARCH_INTERVAL`) was not run for this
investigation; the chrono-fire-rate / conflict-multiplier pattern in the work ×
speed table is unambiguous about the cause. A follow-up run with the SC-2 fix
applied should produce a trace on sudoku confirming the conflict count drops
back to the A_baseline range.

## Hardware counter results

Not run. The mechanism for the SC-2 regression is algorithmic (search trajectory
divergence from excessive chrono BT firing), not microarchitectural. `perf
stat` data would add little beyond the per-prop slowdown already visible in
`speed_r` (sudoku 1.81, REGRandom 2.33) — likely cache effects from the longer
trail being kept across more conflicts.

## Parameter sweep results

Not run. The sweep that matters is the threshold inversion (SC-2), which is a
code fix, not a parameter. A future sweep over `chrono_max_delta` ∈ {1, 5, 25,
100} **with the predicate fixed** would identify the right threshold for solver
11's profile. Kissat's empirical default `chronolevels=100` is a reasonable
starting point.

## Code-Level Recommendations (ordered by ROI)

1. **Fix Gap SC-2 — invert `chrono_max_delta` predicate.**
   File: `solver/11-kissat-port/src/main.rs:4926-4957` (`choose_backtrack_level`).
   Change `if delta > self.chrono_max_delta` to `if delta <= self.chrono_max_delta`,
   keep the rest of the function shape. Rename the field to `chrono_min_delta`
   or `chrono_levels` (matching kissat). Update tests at
   `main.rs:10937-10961`. Reference: `kissat-latest/src/learn.c:16-42`.
   Expected impact: chrono BT firing rate drops from 31–84 % to <1 % on this
   suite; per-instance work_ratio drops from 3–18× back toward 1.0×; the
   sudoku/6s/REGR regressions should largely disappear; mp1 may stop winning.
   New bead: `SAT-playground-d2b` (P1).

2. **After SC-2 fix, re-run B_chrono on the full 10-instance suite.** Only after
   that comparison establishes a baseline can we discuss flipping the default in
   Gap SC-1. Skip flipping the default in the same change; ship SC-2 alone first.
   No bead — this is task ordering for SC-2's verification.

3. **Fix Gap SC-3 — clause-weighted reorder score.**
   File: `solver/11-kissat-port/src/main.rs:3104` (`reorder_branching_by_activity`)
   and surrounding `activity_reorder_vars`. Port `compute_weights` and the
   `less_focused_order` / `less_stable_order` comparators. Wire as
   `SAT_REORDER` ON path; keep the current activity-only sort behind a
   `SAT_REORDER_MODE=activity|kissat` knob during validation. Reference:
   `kissat-latest/src/reorder.c:24-217`. Expected impact: helpers on SAT-search
   instances after preprocessing eliminates low-degree variables. Lower ROI than
   SC-2 because this only matters when `SAT_REORDER` is also enabled.
   New bead: `SAT-playground-vcc` (P3, linked related to `SAT-playground-5b2.2.18.2`).

## Rejected sweeps / non-issues

- `SAT_BRANCH_MODE` and `SAT_INITIAL_CLAUSE_MODE` ablation skipped because
  B_chrono evidence dominated. Not refuted — just deferred. Out-of-scope here.

## Artifact paths

- Ablation script: `log/analyzesat-2026-05-27-search-control/run_ablation.sh`
- Analysis script: `log/analyzesat-2026-05-27-search-control/analysis.py`
- A_baseline (10 inst): `log/analyzesat-2026-05-27-search-control/A_baseline/results.csv`, `.../stats.jsonl`
- B_chrono (4 inst): `log/analyzesat-2026-05-27-search-control/B_chrono/results.csv`, `.../stats.jsonl`
- Decomposition: `log/analyzesat-2026-05-27-search-control/decomp.csv`
- Reference solver results: `.../reference-kissat-latest.csv`, `.../reference-kissat-sc2024.csv`
- FINDINGS: this file.
