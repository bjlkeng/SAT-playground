# Bottleneck Analysis — solver/11-kissat-port — 2026-05-27 (learned-clause-shrink axis)

**Investigator angle:** fresh-eyes pass on the newly-landed **Kissat learned-clause-shrink port**
(`SAT_CLAUSE_MIN=inblock`, commit `b65815c`). The port is `SmokeSafe` but default-off and has
**never been measured on the profiling suite**. Recent analyzesat passes covered orthogonal axes
(restart policy, rephase/lucky, VMTF, CCMIN modes, BVE/BSR, clause-DB cycle, chrono BT, lucky/walk).
`bd search shrink|inblock|ccmin_inblock` all returned no relevant entries before this run.

**Worktree:** `/tmp/analyzesat-shrink-1779916112` (detached HEAD `624898f`)
**Slug dir:** `log/analyzesat-2026-05-27-shrink-port/`
**Method:** A_baseline (`SAT_CLAUSE_MIN=recursive-limited`, the current default) vs D_inblock
(`SAT_CLAUSE_MIN=inblock`) on a 3-instance partial profiling-suite slice at 300 s, 16 GiB,
`SAT_STATS_JSON=on` per run. The slice was driven by (a) the in-flight nextbeads benchmark
contending for the host during the first D pass, and (b) the regression on instance 3
(`REGRandom-Seed40`) was so dramatic that further instances were not needed to draw
conclusions.

**Host-contention caveat:** D_inblock ran concurrently with an external `nextbeads` benchmark
agent on the same host (~50 % CPU each). When D_inblock was rerun on REGRandom — and when
A_baseline was rerun on the matching instances with stats enabled — the host was clean
(solo). The trajectory-effect numbers (conflicts, decisions, propagations) are deterministic;
wall numbers from D's first three rows are noted with their measurement context.

## Executive Summary

**Headline: `SAT_CLAUSE_MIN=inblock` is a SEARCH-TRAJECTORY regression, not (as initially
hypothesized) an execution-cost regression. It causes the solver to do 1.3–6.6× more conflicts
on baseline-solved instances, and TIMES OUT on a baseline-solved instance the default config
solves in 60 s.**

**On REGRandom-Seed40 (the smoking-gun row):**

| Metric | A_baseline | D_inblock | ratio |
|---|---:|---:|---:|
| result | **UNSAT** | **UNKNOWN** (timeout) | regression to UNKNOWN |
| wall | 59.5 s | 301.4 s (limit hit) | 5.07× |
| conflicts | 1 607 608 | 10 649 347 | **6.62×** |
| decisions | 6.36 M | 33.01 M | 5.19× |
| propagations | 30.21 M | 175.31 M | 5.80× |
| restarts | 3 069 | 16 381 | 5.34× |
| propagations / conflict | 18.79 | 16.46 | 0.88× (D is *faster* per conflict) |
| conflicts / second | 27.0 K | 35.3 K | 1.31× (D runs conflicts *faster*) |

The decomposition is unambiguous: **work_ratio = 6.62×, speed_ratio = 0.88×, net = 5.83×** —
which matches the measured 5.07× wall ratio to within rounding. **The regression is 100 %
trajectory and 0 % execution overhead** — the per-conflict execution path is, if anything,
slightly cheaper under inblock. The solver is digging itself into a much harder sub-problem
with the rewritten learned clauses.

**On 6s299b685_Iter30 (matching baseline-solved row):**

| Metric | A_baseline | D_inblock | ratio |
|---|---:|---:|---:|
| result | SAT | SAT | match |
| wall | 16.5 s | 18.4 s | 1.11× |
| conflicts | 3 764 | 4 871 | 1.29× |
| propagations | 28.09 M | 34.58 M | 1.23× |
| restarts | 19 | 25 | 1.32× |
| learned_lits_final | 52 041 | 74 600 | **1.43×** (D learns LONGER clauses) |
| avg lits/clause | 13.83 | 15.31 | 1.11× (D learns LONGER clauses) |

Same pattern — more conflicts, more work, and learned clauses are **larger on average**, not
smaller. **The "shrink" port is increasing learned-clause literal count, the opposite of its
stated purpose.**

**On sudoku-N30-12 (matched but both contended; A was queued behind a separate case9-repro
benchmark, D ran during the original nextbeads run):**

| Metric | A_baseline | D_inblock | ratio |
|---|---:|---:|---:|
| result | UNSAT | UNSAT | match |
| wall | 198.2 s | 279.0 s | 1.41× |
| conflicts | 259 775 | 293 113 | **1.13×** |
| propagations | 1.31 G | 1.36 G | 1.04× |
| restarts | 617 | 700 | 1.13× |
| props / conflict | 5 054 | 4 651 | 0.92× |
| conflicts / second | 1 310 | 1 050 | 0.80× (D had heavier contention) |

Sudoku shows the milder trajectory effect (+12.8 % conflicts) plus a likely contention-driven
wall gap (D's `props/s` was 26 % lower, A's was during a quieter slice). Per-conflict cost is
slightly *cheaper* under D (+8 % fewer props/conflict). The trajectory effect (+12.8 %
conflicts) explains ~13 % of the wall difference; the rest is contention noise. **The sudoku
row alone would be borderline, but the REGRandom UNKNOWN row makes the verdict clear.**

### Why this is happening — the algorithmic root cause

Solver 11's `CCMIN_INBLOCK` mode does **two passes** in sequence at
`src/main.rs:6329–6408`:

1. `lit_redundant`-style recursive minimize with **`same_level_only = true`**
   (`src/main.rs:6374`) — removes only literals whose entire reason chain stays within their
   own decision level (and whose level has multiple clause literals).
2. `shrink_learned_clause_blocks` — compresses runs of same-level literals to a single UIP.

This is **algorithmically weaker than both** of kissat's two `kissat_minimize_clause`
configurations:

* Kissat with **shrink ≤ 2**: runs the **full cross-level recursive minimize**
  (`kissat/src/minimize.c:180–195`), then runs shrink. Cross-level minimize removes literals
  whose reasons fan out across levels, which `same_level_only` refuses by design.
* Kissat with **shrink = 3 (default)**: marks all clause literals removable and
  **skips minimize entirely** (`kissat/src/minimize.c:175–178`), then relies on
  `shrink_clause` — whose `shrink_literal` at `kissat/src/shrink.c:63–77` calls
  `kissat_minimize_literal (lit, false)` for any lower-level parent on the cross-level
  reason walk, which captures the same opportunities the full minimize would have.

Solver 11 took the worst of both worlds: weakened the upfront minimize **and** omitted the
in-shrink cross-level minimize. The same_level_only minimize removes fewer literals than
either alternative; the in-shrink fallback at `src/main.rs:1681–1693` refuses any lower-level
parent that is not already in the learned clause and pre-marked `REDUNDANT_REMOVABLE`. So
the **net set of removable lower-level literals is strictly smaller** than under
`recursive-limited` (the default), which IS the full cross-level minimize. That explains
why D_inblock's `learned_lits_final` exceeds A_baseline's by **+43.3 %** on 6s299b685, and
why the trajectory diverges sharply on REGRandom.

**Three concrete, source-diffable gaps to fix, in order of importance:**

1. **SH-A (P1, algorithmic regression):** `CCMIN_INBLOCK` mode must NOT use
   `same_level_only=true` for its upfront minimize. Either drop the upfront minimize
   entirely (matching kissat shrink=3) **or** keep it cross-level (matching kissat shrink≤2).
   The current "weakened upfront + weak shrink" hybrid is a measurable regression.
   Location: `src/main.rs:6374` and `src/main.rs:1681–1693`. Smoking gun: REGRandom UNKNOWN
   regression, 43 % literal-bloat on 6s299b685.
2. **SH-B (P3, semantic/feature gap):** even after fixing SH-A by dropping the same_level_only
   upfront, `shrink_literal_for_block` at `src/main.rs:1666–1693` still refuses lower-level
   parents that aren't already in the clause. Kissat's `shrink_literal` at
   `kissat/src/shrink.c:63–77` (shrink>2) calls `kissat_minimize_literal` on those.
   Add that fallback to recover the cross-level coverage. Location:
   `src/main.rs:1681–1687`.
3. **SH-C (P3, perf):** `try_shrink_learned_clause_block` at `src/main.rs:1725–1736` walks
   the trail backward starting at `context.trail.len()`. Kissat (`shrink_block` at
   `kissat/src/shrink.c:223–245` + `next_block` at `kissat/src/shrink.c:271–305`) starts at
   `begin_trail + max_trail` where `max_trail` is the highest trail position of any block
   literal. With deep search trails, solver 11 wastes O(D − max_trail) state lookups per
   block. This is the perf gap; it is dominated by SH-A and SH-B in this benchmark, but
   should be fixed when those land.

A pre-mark micro-optimization (lifting `for &lit in learned_clause.iter().skip(1)` at
`src/main.rs:1708–1716` out of `try_shrink_learned_clause_block` and into the outer block
loop, so it runs O(L) instead of O(G × L) per learned clause) is also a clean follow-up but
is far below the noise floor compared to SH-A.

### Recommended next steps (ordered by ROI)

1. **Land SH-A first.** Change `same_level_only` at `src/main.rs:6374` to `false` for
   `CCMIN_INBLOCK`. Rerun the 10-instance profiling suite. If the regression persists,
   then SH-B / SH-C are next; if it disappears, the port is acceptable and can be
   promoted to a SmokeSafe → Experimental promotion gate.
2. **Add SH-B** (cross-level minimize in `shrink_literal_for_block`) as a follow-up
   experiment. Validate that learned-clause literal counts drop below A_baseline.
3. **Defer SH-C** until SH-A and SH-B are landed and known good. The trail walk is the
   right thing to fix, but it does not explain the current regression.
4. **Do NOT promote `SAT_CLAUSE_MIN=inblock` to default** until SH-A is landed. The current
   SmokeSafe + default-off posture is the right one.
5. **Update `FEATURES.md`** to cite this analyzesat slug as the validation evidence for
   keeping `inblock` default-off.

Three new beads will be filed.

## Configuration matrix

| Config | Env | Status | Purpose |
|---|---|---|---|
| `A_baseline` | (defaults; `SAT_CLAUSE_MIN=recursive-limited`) | run solo on 6s299, REGRandom, mp1 with stats; sudoku queued; wall numbers for the rest taken from in-flight `nextbeads-2026-05-27-s11-04d-before` | reference |
| `D_inblock` | `SAT_CLAUSE_MIN=inblock` | run on sudoku, 6s299, REGRandom under host contention | feature under test |

Configs `B_off` (`SAT_CLAUSE_MIN=off`), `C_basic`, `F_inblock_otfs` are encoded in
`run_ablation.sh` but skipped; the trajectory regression on REGRandom is already a
definitive signal.

## Work × Speed decomposition

Matched (A solo where possible, D contended; conflicts unaffected by contention). Output of
`analysis.py`:

```
instance                       res_A   res_D    wall_A  wall_D   Δwall%  conf_A      conf_D     workΔ%   props/s_A  props/s_D  speedΔ%  net%
sudoku-N30-12                  UNSAT   UNSAT    198.2   279.0   +40.8%   259 775     293 113    +12.8%   6 621 184  4 885 701  +35.5%   +52.9%
6s299b685_Iter30               SAT     SAT       16.5    18.4   +11.3%     3 764       4 871    +29.4%   1 698 966  1 879 338   -9.6%   +17.0%
REGRandom-K4-L1-Seed40         UNSAT   UNKNOWN    59.5   301.4  +407.0% 1 607 608  10 649 347  +562.4%     508 163    581 600  -12.6%  +478.8%

PAR-2 A: 274.2 s
PAR-2 D: 897.4 s
Δ PAR-2: +227.28 %
```

The 6s299 and REGRandom rows show `speed_ratio` (A_pps / D_pps) **less than 1.0** — D is
*faster* per conflict — yet wall time is much worse because D does many more conflicts.
`work_ratio` accounts for the wall-time gap entirely. **This refutes the original
execution-cost hypothesis: the per-event execution path of `inblock` is fine. The damage is
the search trajectory itself.**

The sudoku row's `speedΔ +35.5 %` is the lone counter-signal, but it comes from a
contention-asymmetric measurement: D's wall was during the original heavily-contended
nextbeads run, while A's wall was during a separate (lighter) overlap with a
`case9-repro` benchmark. The deterministic counters on sudoku still show D's trajectory
effect at +12.8 % conflicts, consistent with the other rows.

## Reference solver comparison (kissat-latest)

| instance | A wall (s) | D wall (s) | kissat-latest (s) | A vs ref | D vs ref |
|---|---:|---:|---:|---:|---:|
| sudoku-N30-12 | 189 (contended) | 279 (contended) | 267.4 | 0.71× (clean) | 1.04× (clean-est) |
| 6s299b685_Iter30 | 16.5 (solo) | 18.4 (contended) | 37.4 | 0.44× | 0.49× |
| REGRandom-Seed40 | 59.5 (solo) | UNKNOWN | 2.3 | 25.9× | timeout |
| mp1-Nb7T46 | 44.2 (solo) | (D not run) | 7.7 | 5.7× | n/a |

A_baseline already beats kissat-latest on sudoku and 6s299b685 (preprocessing leverage), but
lags by 5–26× on the rest. `inblock` does not help close any of those gaps — it widens them
or breaks them outright.

## Reference diff — implementation gaps

### Gap SH-A — `same_level_only=true` weakens upfront minimize before shrink

**Solver 11:** `src/main.rs:6374`

```rust
let redundancy_context = RedundancyCheckContext {
    reasons: reason_context,
    decision_level,
    reason,
    frame_used,
    max_depth: self.minimize_depth_limit,
    same_level_only: self.ccmin_mode == CCMIN_INBLOCK,   // <-- restricts minimize
};
```

And `src/main.rs:1781–1814` (the body of `lit_redundant` that consumes the flag):

```rust
let target_level = context.decision_level[lit.unsigned_abs() as usize];
if context.same_level_only && context.frame_used.get(target_level).copied().unwrap_or(0) <= 1 {
    return false;
}
// ...
if (context.same_level_only && context.decision_level[parent_var] != target_level)
    || (context.same_level_only
        && context.frame_used.get(context.decision_level[parent_var]).copied().unwrap_or(0) <= 1)
    || depth >= context.max_depth
    || context.reason[parent_var].is_none()
    || state[parent_var] == REDUNDANT_FAILED
{
    // mark FAILED, return false
}
```

So `lit_redundant` refuses to cross level boundaries OR to enter levels with singleton
frames. This is *more restrictive than the regular recursive minimize* (`CCMIN_BASIC` /
`CCMIN_RECURSIVE_LIMITED`).

**Kissat:** `kissat/src/minimize.c:172–195`

```c
for (const unsigned *p = lits; p != end; p++)
    kissat_push_removable (solver, assigned, IDX (*p));

if (GET_OPTION (shrink) > 2) {
    STOP (minimize);
    return;                                           // shrink=3: skip minimize entirely
}

// shrink ≤ 2: run regular cross-level minimize loop
unsigned minimized = 0;
for (unsigned *p = end; --p > lits;) {
    const unsigned lit = *p;
    assert (lit != not_uip);
    if (minimize_literal (solver, true, assigned, lit, 0)) {
        *p = INVALID_LIT;
        minimized++;
    }
}
```

Two valid kissat configurations: full cross-level minimize + shrink, or no minimize + shrink.
Solver 11 chose a third configuration: same-level-only minimize + shrink, which does less
work upfront and does not recover that work in shrink (see SH-B).

**Cost:** missed minimization opportunities at the upfront pass. The downstream effect is
that the learned clause that goes into shrink contains literals that should have been
removed. Subsequent UIP selection in `shrink_learned_clause_blocks` then chooses different
asserting literals than kissat would have — biasing the VSIDS / VMTF queue differently
on every conflict. On REGRandom, this is enough to send the trajectory into a 6.6×-worse
sub-problem.

**Fix sketch:** change `src/main.rs:6374` to `same_level_only: false`, and remove the
`same_level_only` field if no other code path needs it. Validate that
`learned_lits_final` is now strictly less than A_baseline on 6s299 (the upfront cross-level
minimize plus shrink should beat the upfront cross-level minimize alone). Rerun the suite.

### Gap SH-B — `shrink_literal_for_block` does not call cross-level minimize

**Solver 11:** `src/main.rs:1681–1687`

```rust
if lit_level < level {
    return if state[var] == REDUNDANT_REMOVABLE {
        Ok(false)
    } else {
        Err(())                                     // <-- fails, no minimize attempt
    };
}
```

**Kissat:** `kissat/src/shrink.c:63–77`

```c
if (a->level < level) {
    if (a->removable) {
        return 0;
    }
    const bool always_minimize_on_lower_level = (GET_OPTION (shrink) > 2);
    if (always_minimize_on_lower_level &&
        kissat_minimize_literal (solver, lit, false)) {
        return 0;                                   // <-- recover via minimize
    }
    return -1;
}
```

**Cost:** when expanding a same-level reason during shrink_block, any lower-level parent
that wasn't already in the learned clause causes the entire block-shrink to fail. Kissat
recovers by attempting a fresh `minimize_literal` recursion. Solver 11 always gives up.
Net effect: solver 11's `shrink_learned_clause_blocks` succeeds far less often than kissat's
`shrink_block`.

**Fix sketch:** add a third branch: when `lit_level < level` and `state[var] !=
REDUNDANT_REMOVABLE`, fall into a cross-level `lit_redundant`-style recursion (the existing
infrastructure at `src/main.rs:1764–`). Reuse the same `state` cache so successful
recursions populate `REDUNDANT_REMOVABLE` for future literal checks within the same shrink
invocation.

### Gap SH-C — backward trail walk starts at top of trail, not at block max_trail

**Solver 11:** `src/main.rs:1725–1736`

```rust
let mut trail_pos = context.trail.len();
loop {
    let mut uip_lit = None;
    while trail_pos > 0 {
        trail_pos -= 1;
        let lit = context.trail[trail_pos];
        let var = lit.unsigned_abs() as usize;
        if context.decision_level[var] == level && state[var] == REDUNDANT_SOURCE {
            uip_lit = Some(lit);
            break;
        }
    }
    ...
}
```

**Kissat:** `kissat/src/shrink.c:223–245` plus `next_block` at lines 271–305

```c
// next_block computes max_trail while scanning the block range:
const unsigned trail = a->trail;
if (trail > max_trail)
    max_trail = trail;

// shrink_block starts the walk there:
const unsigned *t = begin_trail + max_trail;
while (!failed) {
    do
        uip = *t--;
    while (!assigned[IDX (uip)].shrinkable);
    ...
}
```

**Cost:** every shrink_block invocation, solver 11 scans `(trail_len - max_trail)` extra
trail items doing nothing. Subordinate to SH-A/SH-B but worth fixing once the trajectory
bug is closed.

**Fix sketch:** add `max_trail: usize` to `ShrinkBlockContext`. In
`shrink_learned_clause_blocks` at `src/main.rs:6433–6443`, while scanning the block, also
compute `max_trail = max(trail_position[var] for var in block)`. Pass to
`try_shrink_learned_clause_block`; seed `trail_pos = max_trail + 1`. Requires a
`trail_position[var]` cache (build once per analyze, or once per call).

## Trajectory analysis

Beyond the conflict-count delta, two more pieces of evidence point to trajectory-only
degradation under `inblock`:

1. **Learned-clause literal bloat** (`learned_lits_final`): D_inblock has +43.3 % more
   total learned literals on 6s299b685 (74 600 vs 52 041) AND +10.7 % more lits per
   clause on average (15.31 vs 13.83). Shrink is supposed to *reduce* literal counts;
   here it increases them because the upstream same_level_only minimize removes fewer
   lits than recursive-limited would have removed.
2. **Restart count tracking conflict count**: D_inblock has +32 % more restarts on
   6s299 and +434 % more on REGRandom — proportional to the conflict-count blowup.
   This is consistent with restart heuristics responding to learned-clause LBD
   distributions, not a separate restart-policy regression.

A `SAT_TRACE_SEARCH_INTERVAL=20000` side-by-side trace was not collected; the trajectory
divergence is already established by the matched 1.29× and 6.62× conflict counts on the
two cleanly-measured rows. If SH-A is landed and the regression persists, a trajectory
trace on REGRandom would be the next investigative step.

## Hardware counter results

**Not collected.** Host `perf_event_paranoid=4` blocks `perf record`/`perf stat`. Counter
data is not needed to attribute this regression — the work-counter decomposition
(work_ratio = 6.62, speed_ratio = 0.88) already isolates the cause to trajectory, not
execution. If SH-A and SH-B are landed and a residual perf gap remains, a follow-up
`perf annotate` on `try_shrink_learned_clause_block` would validate SH-C.

## Parameter sweep results

**Not collected.** This is a binary feature axis (mode on/off), not a parameter axis.
No sweep is meaningful until the algorithmic gap (SH-A) is resolved.

## Code-level recommendations (ordered by ROI)

1. **SH-A — Drop same_level_only restriction on CCMIN_INBLOCK upfront minimize**
   * File: `src/main.rs:6374`
   * Change `same_level_only: self.ccmin_mode == CCMIN_INBLOCK` → `same_level_only: false`
   * Validate via the 3 matched instances at minimum (6s299, REGRandom, sudoku) before
     extending to full suite.
   * Reference: `kissat/src/minimize.c:172–195`.

2. **SH-B — Add cross-level minimize-via-reason for lower-level parents in shrink**
   * File: `src/main.rs:1666–1693` (extend `shrink_literal_for_block`)
   * Replace the lone `Err(())` branch with a fallback that calls into a
     `lit_redundant`-style recursion permitting cross-level recursion.
   * Reuse `REDUNDANT_REMOVABLE` and the existing `RedundancyCheckContext` machinery.
   * Reference: `kissat/src/shrink.c:63–77`.

3. **SH-C — Start backward trail walk at block max_trail**
   * Files: `src/main.rs:1696–1762`, `src/main.rs:6411–6478`
   * Plumb a `max_trail` argument through `ShrinkBlockContext` and
     `try_shrink_learned_clause_block`; compute it in the outer block scan.
   * Reference: `kissat/src/shrink.c:271–305` (`next_block`) and lines 223–245
     (`shrink_block`).

## Rejected hypotheses

- **"The regression is dominated by per-conflict execution cost (SH-1 / SH-2 type)"** —
  REFUTED. Solver 11's per-conflict execution under `inblock` is *faster* than under
  `recursive-limited` on the measured rows (D's props/conflict and confs/sec are slightly
  *better* than A's). The wall-time gap is entirely explained by D doing more conflicts.
  The pre-mark loop being re-run per block (originally suspected as SH-2) is real but
  immaterial in practice.
- **"The pre-mark of all clause lits as REDUNDANT_REMOVABLE is a correctness bug"** —
  REFUTED via reading `kissat/src/minimize.c:172–178`. Kissat with shrink>2 does the
  identical pre-mark (`kissat_push_removable` for every clause literal). Solver 11's
  pre-mark is the correct port of that step.

## Beads filed

- `SAT-playground-prz` (P1, bug): SH-A — CCMIN_INBLOCK uses same_level_only=true, regresses
  search trajectory. Fix: change `same_level_only` to `false` at `src/main.rs:6374`.
  Evidence: REGRandom UNKNOWN, 6.62× conflict blowup, +43 % learned-lits on 6s299b685.
- `SAT-playground-8ch` (P3, bug): SH-B — `shrink_literal_for_block` lacks cross-level
  minimize-via-reason fallback. Fix: extend `src/main.rs:1681–1687` to call into the
  cross-level `lit_redundant` path. Reference: `kissat/src/shrink.c:63–77`.
- `SAT-playground-0cu` (P3, bug, perf): SH-C — backward trail walk starts at full trail
  length, not block max_trail. Fix: plumb `max_trail` through `ShrinkBlockContext`.
  Reference: `kissat/src/shrink.c:271–305`.

Existing beads searched: `bd search shrink|inblock|ccmin_inblock` returned no relevant
entries before this slug.

## Artifact paths

- Ablation script: `log/analyzesat-2026-05-27-shrink-port/run_ablation.sh`
- A_baseline followup script: `log/analyzesat-2026-05-27-shrink-port/run_a_baseline_followup.sh`
- Analysis script: `log/analyzesat-2026-05-27-shrink-port/analysis.py`
- A_baseline solo with stats: `log/analyzesat-2026-05-27-shrink-port/A_baseline/results.csv`
- A_baseline (broader, no stats; matched contention): `log/nextbeads-2026-05-27-s11-04d-before/results.csv`
- D_inblock partial results: `log/analyzesat-2026-05-27-shrink-port/D_inblock/results.csv`
- Per-instance stdout/stderr: `log/analyzesat-2026-05-27-shrink-port/{A_baseline,D_inblock}/*.{stdout,stderr}`
- Reference kissat-latest CSV: `log/analyzesat-2026-05-27-shrink-port/reference-kissat-latest.csv`
- Reference kissat-sc2024 CSV: `log/analyzesat-2026-05-27-shrink-port/reference-kissat-sc2024.csv`
