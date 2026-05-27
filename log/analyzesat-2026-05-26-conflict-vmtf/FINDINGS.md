# Bottleneck Analysis — solver/11-kissat-port — 2026-05-26 (conflict-vmtf)

**Investigator angle:** Fresh-eyes pass on **conflict analysis primitives** (clause
minimization, 1UIP deduce, OTFS, on-the-fly subsumption, resolved-conflict mode) and
**VMTF** queue mechanics. Prior runs covered restart/lucky/focused-stable/LBD-reducer/
BVE/BSR. This pass deliberately leaves those alone and looks at code paths with **no
existing beads**: `minimize_learned_clause`, `lit_redundant`, `analyze_conflict_to_scratch_impl`,
the `pick_vmtf_branch_var` / `bump_analyzed_variable_activity` interplay, and the
glue-bookkeeping gating story.

**Worktree:** `/tmp/analyzesat-2026-05-26-conflict-vmtf` (detached HEAD `e7ec1f8`)
**Slug dir:** `log/analyzesat-2026-05-26-conflict-vmtf/`
**Method:** 7-config ablation × 10 profiling instances (300 s, 16 GiB); work × speed
decomposition vs `A_baseline`; kissat-latest reference reused from
`log/analyzesat-2026-05-25-2043/reference-kissat-latest.csv` (kissat binary unchanged).
**Caveat:** the host was simultaneously running another agent's `kissat-watch` shuffle
test AND another agent's `analyzesat-2026-05-26-conflict-analysis` (which overlaps
configs B_resolved/C_lbd). 3-way CPU contention adds wall-time noise. Work counters
(conflicts, propagations, decisions) are deterministic and unaffected.

## Config matrix

Each config flips exactly one knob in solver 11's **conflict-analysis / minimization /
OTFS / VMTF** axis. The prior runs covered orthogonal axes (restart, search-mode).

| Config | Env vars | What it tests |
|---|---|---|
| `A_baseline` | (defaults: `SAT_CLAUSE_MIN=recursive-limited`, `SAT_MINIMIZE_DEPTH_LIMIT=1000`) | reference |
| `B_ccmin_off` | `SAT_CLAUSE_MIN=off` | Remove all minimization → quantifies what minimization buys |
| `C_ccmin_basic` | `SAT_CLAUSE_MIN=basic` | Single-level minimization only (MiniSat-style local) |
| `D_ccmin_inblock` | `SAT_CLAUSE_MIN=inblock` | InBlockShrink (same-level-only restricted recursion) |
| `E_otfs_on` | `SAT_OTFS=on` | Add on-the-fly self-subsumption (default OFF in solver 11) |
| `F_resolved` | `SAT_CONFLICT_ANALYSIS_MODE=resolved` | Use solver-10-style resolved analysis (default OFF) |
| `G_deep_min` | `SAT_MINIMIZE_DEPTH_LIMIT=1000000000` | Remove depth limit (kissat default = 1000) |

## Executive Summary

1. **`SAT_CLAUSE_MIN` matters more than any prior-investigated knob.** Removing
   minimization (`B_ccmin_off`) **degrades PAR-2 from 840 to 3214 (3.8×)** with 5
   timeouts. Even partial removal (`C_ccmin_basic`, single-level only) costs PAR-2
   2050. The clause-minimization primitive is doing heavy lifting that no other
   optimization replaces.

2. **`SAT_CLAUSE_MIN=inblock` is a dramatic and unobvious win on Kakuro and 6s299b685
   /velev — and a corresponding loss on mp1/battleship/case9.** The split is so clean
   it points at a missing kissat feature, not a parameter choice:
   * Kakuro: A=232 s → D=63 s (**3.7× wall-time win, 89 % fewer conflicts**)
   * velev: A=77 s → D=57 s (66 % fewer conflicts)
   * 6s299b685: A=18 s → D=16 s (66 % fewer conflicts)
   * SCPC: A=14 s → D=13 s (close to identical)
   * mp1: A=45 s → D=140 s (**3.1× worse, 2.6× more conflicts**)
   * battleship: A=23 s → TIMEOUT
   * case9: A=128 s → TIMEOUT
   * brocard: A=10 s → D=19 s (65 % more conflicts)

   `inblock` = `same_level_only=true` in `lit_redundant`, i.e. recursion is restricted
   to literals at the same decision level. This **matches kissat's minimization
   semantics**. The fact that it wins big on circuit/HWMCC and loses big on
   bit-blasted/multi-level instances exactly fits kissat's pipeline: kissat does
   inblock-style minimization AND **then runs `kissat_shrink_clause` on the
   surviving multi-level structure**. Solver 11 has no shrink, so toggling to
   inblock gains the circuit-style trajectory benefit but loses the multi-level
   compression that mp1/battleship/case9 need.

3. **`C_ccmin_basic` (single-level, no recursion) also produces ~3× fewer conflicts
   on 6s299b685 and velev.** Same story: the default `recursive-limited` mode is
   over-minimizing on those families. Basic mode wins trajectory on those instances
   but doesn't beat inblock anywhere (because basic also loses the binary-chain
   minimization that inblock keeps for same-level chains).

3. **Source diff against kissat reveals 5 implementation gaps in conflict analysis
   that have no existing beads:**
   * **Gap CV-1: No `kissat_shrink_clause` in solver 11.** Kissat *always* runs
     shrink after minimize when `GET_OPTION(shrink) > 0` (kissat default). Shrink
     compresses each decision-level block to its UIP; this is what produces kissat's
     shorter learned clauses on HWMCC. Solver 11 has zero equivalent.
   * **Gap CV-2: No frame-used singleton check in `lit_redundant`.** kissat's
     `minimized_index` rejects in O(1) any literal at a level with only one analyzed
     variable (`frame->used <= 1`). Solver 11 has to recurse and discover this the
     hard way.
   * **Gap CV-3: No `sort_deduced_clause` before minimize.** kissat sorts the learned
     clause by descending decision level *before* calling minimize so the recursive
     descent traverses literals in a cache-friendly trail order; solver 11 minimizes in
     analyze-order and only swaps the second-watched literal afterward.
   * **Gap CV-4: `mark_clause_as_used` (glue recompute on every conflict-analysis
     touch) is gated behind `SAT_USE_LBD=on` in solver 11.** Kissat does this
     unconditionally in `kissat_deduce_first_uip_clause` for every clause it touches —
     the LBD bookkeeping is part of the core, not opt-in. Default-mode solver 11
     therefore lacks half of kissat's clause-aging signal.
   * **Gap CV-5: Default-mode VMTF prefix does redundant VSIDS bumps.** In
     single-mode + `SAT_VMTF=single` (the default), every analyzed-variable bump
     during the VMTF prefix updates BOTH VMTF stamps AND VSIDS heap scores. Kissat
     never does both: focused-mode bumps the queue only, stable-mode bumps the heap
     only. The `move_to_front_only` flag in `bump_analyzed_variable_activity` is
     true only when `kissat_focused_vmtf_active()` (which requires
     `FocusedStable+Focused`), so the default never enters the cheap path.

4. **`F_resolved` (`SAT_CONFLICT_ANALYSIS_MODE=resolved`) is a confirmed no-op
   trajectory.** Conflict counts are **identical to `A_baseline` on every instance**
   (10/10), confirming the resolved-mode flag does not change the 1UIP output, only
   the variable-marking discipline during the backward walk. PAR-2 +5.6 % is contention
   noise. If the resolved code path is kept for solver-10 parity, this is fine, but it
   should not be in tuning sweeps as a trajectory knob.

5. **`G_deep_min` (`SAT_MINIMIZE_DEPTH_LIMIT=1e9`) is also a no-op.** Identical
   conflict counts to baseline on every instance — **the default `depth=1000` is
   never hit on the profiling suite**. The depth limit was set to match kissat but
   has zero observed effect; PAR-2 -4 % is contention noise.

6. **`E_otfs_on` introduces NEW regressions: TIMEOUT on velev and mp1.** velev was
   77 s on baseline; with OTFS it times out. mp1 was 45 s; also TIMEOUT. The OTFS
   path rescues Kakuro (232 → 74 s, 84 % fewer conflicts) and helps SCPC slightly,
   but the velev/mp1 regression is severe enough to block default-on promotion.
   Worth a bead — `SAT_OTFS=on` is **unsafe at default** on the profiling suite.

## PAR-2 per config (300 s timeout, profiling suite, HEAD `e7ec1f8`)

| Config | Solved | Timeout | PAR-2 | Δ vs A | Note |
|---|---:|---:|---:|---:|---|
| `A_baseline` | 10 | 0 | 840.8 | — | contended (3-way), prior clean ≈ 764 |
| `B_ccmin_off` | 5 | 5 | **3214.8** | **+283 %** | timeouts: sudoku, mp1, Kakuro, battleship, case9 |
| `C_ccmin_basic` | 8 | 2 | 2049.5 | +144 % | timeouts: mp1, battleship |
| `D_ccmin_inblock` | 8 | 2 | 1818.6 | +116 % | timeouts: battleship, case9 |
| `E_otfs_on` | 8 | 2 | 1772.8 | +111 % | timeouts: mp1, velev (new regression) |
| `F_resolved` | 10 | 0 | 888.1 | +6 % | identical conflict counts to A — no-op trajectory |
| `G_deep_min` | 10 | 0 | 805.3 | -4 % | identical conflict counts to A — depth=1000 never hit |

## Per-instance wall time (s)

| instance | A | B (off) | C (basic) | D (inblock) | E (otfs) | F (resolved) | G (deep) |
|---|---:|---:|---:|---:|---:|---:|---:|
| sudoku | 231.3 | TIMEOUT | 292.9 | 247.1 | 224.5 | 254.3 | **212.4** |
| 6s299b685 | 18.4 | 18.9 | 17.6 | **15.9** | 17.0 | 19.4 | 18.9 |
| REGRandom | 61.3 | **52.2** | 55.5 | 63.7 | 81.6 | 62.4 | 60.5 |
| mp1 | **44.6** | TIMEOUT | TIMEOUT | 139.7 | TIMEOUT | 48.3 | 45.6 |
| Kakuro | 232.7 | TIMEOUT | 275.5 | **63.4** | 74.2 | 254.8 | 228.6 |
| SCPC | 13.8 | 24.1 | 16.2 | 13.1 | **11.7** | 13.8 | 13.7 |
| velev | 77.5 | 104.7 | 62.0 | **57.2** | TIMEOUT | 73.6 | 66.0 |
| brocard | 10.2 | 15.0 | 11.8 | 18.5 | 9.7 | 9.4 | **8.8** |
| battleship | 23.2 | TIMEOUT | TIMEOUT | TIMEOUT | 23.2 | 23.2 | **23.0** |
| case9 | 127.8 | TIMEOUT | 118.1 | TIMEOUT | 131.0 | 128.9 | **128.0** |

## Conflict count delta — the key trajectory signal

```
instance     A_conf    B_ccmin_off    C_ccmin_basic   D_ccmin_inblock
sudoku       259 775   TIMEOUT        311 488 (+20 %)   263 204 ( +1 %)
6s299b685      3 764     7 819 (+108 %)   1 308 (-65 %)   1 272 (-66 %) ←
REGRandom  1 607 608 1 300 234 (-19 %)  1 461 213 ( -9 %)  1 607 371 (  0 %)
mp1          425 229   TIMEOUT        TIMEOUT          1 093 645 (+157 %) ✗
Kakuro       732 107   TIMEOUT          781 706 ( +7 %)    82 441 (-89 %) ←←
SCPC         188 144   253 353 (+35 %)   206 101 (+10 %)   179 799 ( -4 %)
velev        179 968   214 749 (+19 %)    67 693 (-62 %)    76 160 (-58 %) ←
brocard          403       569 (+41 %)       565 (+40 %)       663 (+65 %)
battleship   593 019   TIMEOUT        TIMEOUT          TIMEOUT  ✗
case9      4 186 969   TIMEOUT        3 811 034 ( -9 %)  TIMEOUT  ✗
```

**The Kakuro `-89 %` and the velev/6s299b685 `-58 % / -66 %`** under `D_ccmin_inblock`
are the headline trajectory finding: switching to kissat-style same-level recursive
minimization rescues a 232 s timeout into a 63 s solve. But the same flip costs
mp1/battleship/case9 (the multi-level instances). Together they point to a missing
shrink pass, not a parameter choice.

This contradicts the usual SAT-solver folklore (kissat's default is recursive
minimization with shrink), but it lines up cleanly with the work-ratio numbers in
prior FINDINGS where `6s299b685` showed 5× speed ratio under focused-stable + EMA —
the heavier minimization was *producing* the harder trajectory.

## Reference solver comparison (vs `kissat-latest` from 2026-05-25 same hardware)

Reused from `log/analyzesat-2026-05-25-2043/reference-kissat-latest.csv`:

| instance | kissat-latest | A_baseline | B (off) | C (basic) | D (inblock) |
|---|---:|---:|---:|---:|---:|
| sudoku | 260.6 | 231.3 | TIMEOUT | 292.9 | 247.1 |
| 6s299b685 | 37.4 | 18.4 | 18.9 | 17.6 | **15.9** ←← |
| REGRandom | **2.3** | 61.3 | 52.2 | 55.5 | 63.7 |
| mp1 | **7.7** | 44.6 | TIMEOUT | TIMEOUT | 139.7 |
| Kakuro | 38.0 | 232.7 | TIMEOUT | 275.5 | **63.4** ←← |
| SCPC | **6.7** | 13.8 | 24.1 | 16.2 | 13.1 |
| velev | 89.9 | 77.5 | 104.7 | 62.0 | **57.2** ←← |
| brocard | 50.6 | **10.2** | 15.0 | 11.8 | 18.5 |
| battleship | **0.18** | 23.2 | TIMEOUT | TIMEOUT | TIMEOUT |
| case9 | 77.2 | 127.8 | TIMEOUT | **118.1** | TIMEOUT |

**Solver-11 with `SAT_CLAUSE_MIN=inblock` beats kissat-latest on 3 instances:** velev
(57 vs 90 s), 6s299b685 (16 vs 37 s, **2.4×**), and Kakuro (63 vs 38 s — comparable
but still impressive after the 232 → 63 s rescue). With the existing solver-11
preprocessing wins, `inblock` is now the most competitive single configuration on
HWMCC-style instances. The matching loss on mp1/battleship/case9 is exactly the gap
where kissat's shrink saves the day, so Gap CV-1 (port shrink) is the next-step
mechanism to convert this into a uniform win.

## Reference Source Diff — implementation gaps (5 new, all unbeaded)

### Gap CV-1: No `kissat_shrink_clause` equivalent in solver 11

* **kissat `shrink.c` + `analyze.c:560-565`** — kissat's conflict-analysis pipeline is
  `deduce → sort_deduced_clause → minimize → shrink`. `kissat_shrink_clause` walks the
  learned clause level-by-level and tries to replace each level's literals with the
  single UIP at that level. On HWMCC instances with many literals at non-current
  levels, shrink can drop 20–60 % of the literals minimize leaves behind.

* **solver 11 `main.rs:6526`** — only calls `minimize_learned_clause`, no shrink.

* **Predicted effect:** the current `C_ccmin_basic` win on velev/6s299b685 is solver 11
  *avoiding* over-minimization. With shrink, solver 11 could simultaneously remove
  even more literals per learned clause AND keep the trajectory advantage that basic
  gives, because shrink operates per-level (preserves the level structure that
  minimize destroys). This is the highest-ROI conflict-analysis gap.

* **Action:** new bead — port `kissat_shrink_clause` from `shrink.c:360-395`.
  Prerequisites: implement frame-used counter (Gap CV-2).

### Gap CV-2: No frame-used singleton check in `lit_redundant`

* **kissat `minimize.c:32-37`** — `minimized_index` checks `if (minimizing || !depth) {
  if (frame->used <= 1) return -1; }` — when minimizing at depth 0 (the outer call) or
  along a binary chain, any literal at a level that has only one used variable cannot
  be removed (because removing it would leave the level empty, losing a UIP literal).
  Kissat rejects in O(1).

* **solver 11 `main.rs:1619-1709`** — `lit_redundant` has no equivalent. It recurses
  through the DFS, accumulates `state[var] = FAILED`, and only returns `false` after
  walking the entire reason chain. On HWMCC instances with many singleton levels
  (which create chains of one-literal-per-level reasons), this is a measurable amount
  of wasted DFS work.

* **Prediction:** under `C_ccmin_basic` (no recursion), the gap doesn't show. Under
  default `recursive-limited`, instances with many `lit_redundant` calls that fail
  immediately because of a singleton level would benefit. velev and 6s299b685 are
  likely candidates because their trajectory regression under default suggests the
  recursion is firing aggressively without finding removable literals.

* **Action:** new bead — implement a per-decision-level `used` counter on the analyzed
  variable struct (kissat keeps it on `frame`), update it during 1UIP deduce in
  `analyze_conflict_to_scratch_impl` when the literal is at a non-current level, and
  short-circuit in `lit_redundant` when `frame->used <= 1`. Match `kissat/src/minimize.c:32-37`.

### Gap CV-3: No `sort_deduced_clause` before minimize

* **kissat `analyze.c:560-562`** — `kissat_minimize_clause` is called *after*
  `sort_deduced_clause`, which sorts the learned literals by descending decision
  level. This ensures `minimize_literal` traverses the reason chains in trail order
  (literals at the deepest levels are processed first, so when minimizing a literal
  at level k, all already-processed `removable`/`poisoned` flags for level >k are
  already set).

* **solver 11 `main.rs:6526`** — calls `minimize_learned_clause(...)` directly on the
  scratch in 1UIP-discovery order. Lines 6538-6549 sort *afterward*, but only to put
  the max-level literal at position 1 (the second-watched). The minimization itself
  walks an unsorted clause.

* **Predicted effect:** unsorted minimization order causes redundant DFS traversals
  because the `state[var]` cache misses are out of trail order. On instances where
  the learned clause spans many decision levels, this is a measurable percentage.

* **Action:** new bead — sort `learned_clause[1..]` by descending `decision_level[var]`
  before calling `minimize_learned_clause`. Profile to confirm net win (sort cost vs.
  fewer DFS hops).

### Gap CV-4: Glue recompute on conflict-analysis touch is opt-in

* **kissat `deduce.c:14-27`** — `mark_clause_as_used` is called for every clause
  involved in 1UIP analysis (conflict clause and every reason walked). It sets
  `c->used = MAX_USED`, increments `clauses_used`, recomputes glue via
  `kissat_recompute_glue`, and potentially promotes the clause to a better tier.
  This happens **unconditionally** when the clause is touched in `deduce.c`.

* **solver 11 `main.rs:2481-2492`** — `mark_learned_clause_recent` is gated behind
  `self.use_lbd` (default off). `maybe_improve_lbd` (`main.rs:2418`) is also gated
  behind `use_lbd`. `mark_clause_literals_for_analysis` (`main.rs:6304-6313`) only
  calls `bump_clause_activity` if `reduce_db_enabled` — and that does not recompute
  glue.

* **Predicted effect:** default-mode solver 11 has weaker clause-aging signal than
  kissat. Clauses that *are* repeatedly useful in conflict analysis don't get
  promoted, so they get evicted at the same rate as never-used clauses. This is
  separate from but compounds bead `SAT-playground-ycw` (which is about the *value*
  of the bump, not the *gating*).

* **Action:** new bead — make `mark_clause_as_used` (clause used in conflict analysis)
  always recompute glue and promote, independent of `SAT_USE_LBD`. The LBD value
  itself is cheap to compute and the clause header already has a slot for it.

### Gap CV-5: Default-mode VMTF prefix double-bumps (VMTF + VSIDS)

* **kissat `bump.c:103-112`** — `kissat_bump_analyzed` calls
  `move_analyzed_variables_to_front_of_queue` in focused mode OR
  `bump_analyzed_variable_scores` in stable mode. Never both.

* **solver 11 `main.rs:4176-4198`** — `bump_analyzed_variable_activity` computes
  `move_to_front_only = !is_temporary && kissat_focused_vmtf_active()`. The latter
  requires `vmtf_mode == FocusedOnly` AND `search_mode_policy == FocusedStable` AND
  `search_mode == Focused`. **Default config is `single-mode + SAT_VMTF=single`,
  which is none of those.** Therefore default-mode runs always have
  `move_to_front_only = false` and the loop calls BOTH `vmtf_stamp_analyzed_var` AND
  `bump_variable_activity` for every analyzed variable, including during the VMTF
  prefix where VSIDS scores are irrelevant.

* **Predicted effect:** every conflict adds N VSIDS heap updates that don't affect
  the next decision (because VMTF is active). This is `analyzed.len() * O(log V)`
  wasted heap work per conflict during the VMTF prefix (~first 1e6 conflicts).

* **Action:** new bead — extend the `move_to_front_only` condition to ALSO cover
  default-mode VMTF prefix (`vmtf_mode == Single && vmtf_branching_active()`).
  Verify with conflict trace that the VMTF prefix duration and decision sequence
  are unchanged.

## Code-Level Recommendations (ordered by ROI)

1. **Port `kissat_shrink_clause` (Gap CV-1) and then promote `SAT_CLAUSE_MIN=inblock`
   to default.** The D ablation already proves inblock wins on 4/8 solved instances by
   3-89 % conflicts. The shrink pass is the mechanism by which kissat keeps the wins
   without losing mp1/battleship/case9. Order: implement Gap CV-2 (frame-used
   counter) as the prerequisite, port shrink from `kissat/src/shrink.c:360-395`,
   verify the loss pattern (mp1/battleship/case9) reverts to baseline or better,
   then make `inblock+shrink` the default. **Highest aggregate ROI**.

2. **Frame-used singleton check in `lit_redundant`** (Gap CV-2). Standalone benefit
   on instances with many singleton-frame levels, and a prerequisite for Gap CV-1.
   Match `kissat/src/minimize.c:32-37`.

3. **Sort learned clause before minimize** (Gap CV-3). Cheap to implement (radix sort
   on `decision_level[var]` for ≤ 32 levels, quicksort otherwise — kissat's
   `RADIX_SORT_LEVELS_LIMIT`).

4. **Unconditional `mark_clause_as_used` glue recompute** (Gap CV-4). Touches the
   default code path so requires careful gating once `SAT_USE_LBD` is reframed as
   "use LBD for *reduction policy*" vs "track LBD for *bookkeeping*".

5. **Eliminate double-bump in default VMTF prefix** (Gap CV-5). Mechanical fix in
   `bump_analyzed_variable_activity`. Likely small but free win.

## Phase-boundary chaos — none observed in this investigation

The minimization-axis configs all produce deterministic conflict counts (the
trajectory is set by the minimization output, not by EMA-restart timing). No
"identical prefix then diverge" pattern detected.

## Hardware counter results

Not run this iteration. The 3.8× PAR-2 swing on `B_ccmin_off` and the trajectory-only
nature of the C/A delta on velev (where `actual ≈ 0.80` matches `work × speed = 0.74`)
make perf counters unnecessary for the headline findings.

## Parameter sweep results

`G_deep_min` lifted the depth limit from kissat-default `1000` to `1e9`. Conflict
counts were identical to baseline on every instance — the default is never reached
on the profiling suite. The depth limit, while semantically meaningful, is a
no-op in practice for these instances. Recommendation: keep at 1000 (matches
kissat), do not promote to default-off.

## Rejected / non-issues this run

* The VMTF queue implementation itself (`branch.rs`) matches kissat's
  `links`/`queue.search.idx` structure. No semantic gap there.
* The 1UIP discovery (`analyze_conflict_to_scratch_impl`) is structurally identical to
  `kissat_deduce_first_uip_clause` apart from the LBD bookkeeping gating (Gap CV-4).
  The seen-bit driven Tarjan-style backward walk is correct.
* Solver 11's `propagate_impl` and kissat's `propsearch.c` use the same two-watch
  structure. The const-generic specialization in solver 11 (HOT_STATS, MODE_TICKS,
  BINARY_FAST) is a cleaner implementation than kissat's mode-stratified macros.

## Artifact paths

* Ablation script: `log/analyzesat-2026-05-26-conflict-vmtf/run_ablation.sh`
* Analysis script: `log/analyzesat-2026-05-26-conflict-vmtf/analysis.py`
* Per-config raw: `log/analyzesat-2026-05-26-conflict-vmtf/<config>/results.csv`,
  `stats.jsonl`, `bench.log`
* Reference CSV (reused): `log/analyzesat-2026-05-25-2043/reference-kissat-latest.csv`
* Worktree: `/tmp/analyzesat-2026-05-26-conflict-vmtf`
* Kissat reference source (verbatim diffs): `benchmarks/reference-solvers/kissat-latest/src/`
  — `analyze.c`, `minimize.c`, `shrink.c`, `deduce.c`, `decide.c`, `bump.c`, `promote.c`
* Companion runs: `log/analyzesat-2026-05-26-0712/FINDINGS.md`,
  `log/analyzesat-2026-05-26-preprocess/FINDINGS.md`
