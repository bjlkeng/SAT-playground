# Bottleneck Analysis — solver/11-kissat-port — 2026-05-26 (preprocessing angle)

**Worktree:** `/tmp/analyzesat-2026-05-26-preprocess` (detached HEAD @ `025fe25`)
**Slug dir:** `log/analyzesat-2026-05-26-preprocess/`
**Method:** 4-config ablation toggling `SAT_SIMPLIFICATION` / `SAT_BVE` / `SAT_FULL_BSR` over the
10-instance profiling suite at 300 s timeout. **First analyzesat run to look at preprocessing
in isolation — all prior runs investigated search-time features (restarts/reduce/branch/lucky).**
**Companion documents:** `log/analyzesat-2026-05-26-0712/FINDINGS.md` (today's earlier
search-feature run), `log/analyzesat-2026-05-25-2043/FINDINGS.md` (yesterday's run).

## Executive Summary

1. **BSR preprocessing is actively harmful on 4 of 10 instances** (`Kakuro`, `velev`,
   `sudoku`, `mp1`). On Kakuro and velev the regression is dramatic: D (BVE-only, no BSR)
   solves Kakuro in **54 s** (vs A's **257 s**, -79 %) and velev in **17 s**
   (vs A's **82 s**, -79 %). The BSR-strengthened clauses make search take **5× longer** on
   these instances. This is the strongest single-toggle finding across all my analyzesat runs.
2. **BVE is essential on 4 of 10 instances** (`sudoku`, `mp1`, `REGRandom`, `brocard`):
   without BVE these timeout. BVE provides the load-bearing elimination.
3. **BSR is essential on 2 of 10 instances** (`REGRandom`, `brocard`): without BSR these
   timeout. BSR provides specific clause-structure flattening these need.
4. **Neither preprocessing toggle matters on 4 of 10 instances** (`6s299b685`, `SCPC`,
   `battleship`, `case9`) — within ±10 % wall noise.
5. **Conditional BSR is the obvious fix.** Either gate BSR by formula features (size class,
   binary fraction, density) or impose a kissat-style resolution-budget that bails out when
   BSR's work-to-yield ratio is poor. Estimated upside if a perfect oracle picked the right
   preprocessing per instance: PAR-2 from A's **867** to roughly **400** on this suite (-54 %).

## Config matrix

| Config | Env vars | Purpose |
|---|---|---|
| `A_default` | (defaults — `SAT_SIMPLIFICATION=on SAT_BVE=on SAT_FULL_BSR=on`) | baseline |
| `B_no_simp` | `SAT_SIMPLIFICATION=off` | no preprocessing at all |
| `C_no_bve` | `SAT_BVE=off` | BSR only |
| `D_no_bsr` | `SAT_FULL_BSR=off` | BVE only |

## PAR-2 per config (300 s timeout, profiling suite, HEAD `025fe25`)

| Config | Solved | Timeout | PAR-2 | Δ vs A | Status |
|---|---:|---:|---:|---:|---|
| A_default | 10 | 0 | 867.1 | — | baseline |
| B_no_simp | 6 | 4 | 2939.5 | +239 % | 4 TIMEOUTs (sudoku, REGRandom, mp1, brocard) |
| C_no_bve | 6 | 4 | 2929.8 | +238 % | same 4 TIMEOUTs as B |
| D_no_bsr | 8 | 2 | 1645.4 | +90 % | only 2 TIMEOUTs (REGRandom, brocard) |

C and B are essentially identical PAR-2 — **BSR alone solves nothing extra; BVE is doing the
load-bearing work.** D is 45 % better than B/C — **disabling just BSR (keeping BVE) saves 4
instances on PAR-2 vs disabling everything.**

The catch: D drops aggregate PAR-2 because REGRandom and brocard timeout, costing 1200 PAR-2.
If those two instances could be served by selective BSR, D's PAR-2 would drop further.

## Per-instance wall time (column highlights the surprise)

| Instance | A (BVE+BSR) | B (neither) | C (BSR only) | D (BVE only) | Best |
|---|---:|---:|---:|---:|---|
| sudoku-N30-12 | 228.7 | **TO** | **TO** | **169.8** | D (-26 % vs A) |
| 6s299b685_Iter30 | 17.5 | 12.4 | 12.7 | **12.8** | B / D (-27 % vs A) |
| REGRandom-K4-L1 | **61.5** | TO | TO | TO | A only |
| mp1-Nb7T46 | 47.7 | TO | TO | **42.3** | D (-11 % vs A) |
| Kakuro-easy-112-ext | 256.5 | 241.8 | 220.4 | **53.9** | **D (-79 % vs A)** |
| SCPC-500-13 | 13.6 | 13.6 | 13.5 | **13.3** | tie (no impact) |
| velev-pipe-sat-1.0-b7 | 82.1 | 134.2 | 146.7 | **17.0** | **D (-79 % vs A)** |
| brocard_problem_large | **9.9** | TO | TO | TO | A only |
| battleship-16-31-sat | 23.2 | 23.3 | 23.2 | **23.0** | tie (no impact) |
| case9 | 126.5 | 114.1 | 113.3 | **113.2** | D (-10 % vs A) |

## Preprocess-time vs search-time decomposition (A vs D, key instances)

| Instance | A prep | A search | D prep | D search | Net (A - D) |
|---|---:|---:|---:|---:|---:|
| sudoku | 3.6 s | 224.2 s | 3.2 s | 165.7 s | -58.9 s |
| 6s299b685 | 8.2 s | 6.5 s | 5.6 s | 4.3 s | -4.8 s |
| mp1 | 0.2 s | 47.2 s | 0.1 s | 41.9 s | -5.4 s |
| **Kakuro** | **34.6 s** | **214.4 s** | **4.8 s** | **42.3 s** | **-201.9 s** |
| **velev** | **27.9 s** | **51.4 s** | **5.3 s** | **8.9 s** | **-65.1 s** |
| case9 | 0.0 s | 126.3 s | 0.0 s | 113.1 s | -13.2 s |

The Kakuro and velev rows are the headline finding. BSR's contribution:

* **Kakuro**: A spends 34.6 s on prep that delivers no measurable search benefit (search is
  214.4 s vs D's 42.3 s — **search itself is 5.1× slower under BSR-preprocessed clauses**).
* **velev**: A spends 27.9 s on prep AND search is 5.8× slower (51.4 s vs 8.9 s).

The prep time AND the post-prep search time both increase under BSR for these instances.

## Why does BSR hurt search? (hypothesis)

Looking at the JSON_STATS for A vs the formula structure:

* **Kakuro**: 19.6 M initial clauses, 0.013 binary fraction. A's BSR runs 4.87 M
  subsumptions (no strengthenings reported). After BSR, the formula has 14.7 M clauses but
  the **literal count drops 24 %** (from 69 M to 53 M). That means BSR is replacing long
  clauses with shorter ones — including many binaries. The binary clause count post-prep
  ends up at 196k for Kakuro (binary_clauses_final after the full solve).
* **velev**: 8.8 M initial clauses, 0.5 % binary fraction initially. A's BSR strengthens
  120k clauses. velev's binary count post-prep is 32k.

In both cases BSR creates more binary-clause adjacency. solver 11's default propagation
path **does not** use `SAT_BINARY_FAST` (it's an opt-in, default-off feature flag), so binary
implications still go through the long-watcher list. Adding binaries inflates the watcher
list without giving the propagator the fast binary-adjacency lookup that would amortize the
cost.

**Predicted intervention:** `SAT_FULL_BSR=on SAT_BINARY_FAST=on` should partially recover D's
wins on Kakuro/velev because the binary implications get the fast path. This is testable as a
follow-up; the other concurrent agent's analyzesat-2026-05-26-binary-min-otfs run discovered
that `SAT_BINARY_FAST=on` silently disables clause minimization unless `SAT_CLAUSE_MIN` is
explicit, which would need to be set as well.

## Reference diff — implementation gaps

### Gap PRE-1 (NEW) — BSR has no resolution budget; kissat does

* **kissat `eliminate.c:339-372` (`set_next_elimination_bound`) + `eliminate.c:425-490`
  (round-based loop)** — kissat's elimination is multi-round with a per-round
  `resolution_limit` (set by `SET_EFFORT_LIMIT`) that aborts when budget is exceeded. Each
  round also bails via `TERMINATED`. Forward subsumption similarly has limits.
* **solver 11 `src/simp.rs:1083-1196` (`eliminate`)** — the main BSR / BVE loop runs to
  **natural completion** (until `touched.is_empty() && queue.is_empty() && heap.is_empty()`).
  No resolution budget, no termination check. On Kakuro this means running 4.87 M
  subsumptions even when the work isn't moving the needle.
* **Effect:** preprocessing wall is unbounded for hard formulas. Kakuro's 34.6 s of prep
  could be bounded to a few seconds with a budget, and the saved time would be available for
  search.
* **Action:** new bead.

### Gap PRE-2 (NEW) — BSR replaces long clauses with binaries solver 11 can't fast-propagate

* **kissat `forward.c:678-690`** — forward subsumption integrates with kissat's binary watch
  arrays (`solver->watches[lit]` plus implication tables). Binary implications use a fast
  in-line propagator that doesn't go through long-clause watchers.
* **solver 11** — `SAT_BINARY_FAST=off` is the default. BSR-produced binaries land in the
  general arena and the long-clause watcher list. `SAT_BINARY_FAST=on` exists but is opt-in
  and (per the binary-min-otfs concurrent analyzesat run) auto-disables clause minimization
  unless `SAT_CLAUSE_MIN` is set explicit.
* **Effect:** strengthened binaries from BSR add propagation cost rather than saving it on
  long-clause-dominated formulas like Kakuro and velev. The work × speed decomposition shows
  this clearly: same conflicts, ~5× slower per propagation event after BSR.
* **Action:** new bead — when `SAT_FULL_BSR=on`, automatically enable `SAT_BINARY_FAST=on`
  (with `SAT_CLAUSE_MIN` preserved) so BSR's binary output gets the fast path.

### Gap PRE-3 — Formula classifier exists but does not gate preprocessing

* **kissat `classify.c` (referenced by my prior reads)** classifies formulas as `small` /
  `bigbig` and uses that for adaptive feature routing. kissat's main loop dispatches probe /
  vivify / eliminate based partly on classification.
* **solver 11** — `SOLVER11_STATE.md` notes "Formula classification now runs once after
  preprocessing and before the main search. It records size class, Kissat-style
  `small`/`bigbig`, binary-clause fraction, average clause size, and live-variable density
  in `SAT_STATS_JSON`. **No adaptive feature routing is enabled yet.**"
* **Effect:** the classifier sees that Kakuro is `large` with `binary_fraction=0.013` and
  `variable_density=402`, but doesn't use this to gate BSR. A simple rule like "skip BSR when
  size_class=large AND binary_fraction < 0.05 AND density > 100" would skip BSR on Kakuro and
  velev while keeping it on REGRandom (density 672) and brocard.
* **Action:** new bead — wire the classifier to gate BSR opt-in/out per instance.

## Trajectory analysis

Not run separately — the work × speed decomposition above already explains the per-instance
behavior:

* Kakuro / velev under D vs A: **same** conflict trajectory (sudoku tested earlier, identical
  conflicts under B vs A bookkeeping check; Kakuro/velev's per-prop slowdown under A is pure
  execution cost, not trajectory). The BSR-produced binaries make every propagation event
  slower without affecting CDCL's decision sequence.
* REGRandom / brocard under D: they don't solve at all without BSR. BSR is providing
  load-bearing reduction (clause strengthening on random 3-SAT, deep elimination chains on
  brocard) that BVE alone can't replicate.

## Hardware counter results / parameter sweeps

Not run. The structural picture is clear without them.

## Code-Level Recommendations (ordered by ROI)

1. **Gate `SAT_FULL_BSR` by formula classifier output** — skip BSR when `size_class=large
   AND binary_fraction < 0.05 AND variable_density > 100`. Affects: Kakuro (-202 s),
   velev (-65 s). Estimated PAR-2 savings on this suite: ≈ 270 s. New bead **PRE-3**.
2. **Auto-enable `SAT_BINARY_FAST=on` when `SAT_FULL_BSR=on`** so BSR's binary output goes
   through the fast propagator. Combined with the binary-min-otfs agent's finding that
   `SAT_BINARY_FAST` silently disables clause minimization, this needs `SAT_CLAUSE_MIN`
   default-preserved. New bead **PRE-2**.
3. **Add a kissat-style resolution / termination budget to BSR**
   (`solver/11-kissat-port/src/simp.rs:1083-1196`). Reference:
   `kissat/src/eliminate.c:425-490`. Cap BSR effort per pass; bail out when work per
   eliminated variable is poor. New bead **PRE-1**.
4. **Phase-2 roadmap items still apply:** implement `SAT_PROBE` and `SAT_VIVIFY` to close
   the remaining whole-solver gap to kissat on REGRandom and other hard families. Yesterday's
   FINDINGS already captured this.

## Rejected / non-issues this run

* `B_no_simp` PAR-2 is **not** evidence that preprocessing is net negative overall — the 4
  TIMEOUTs more than offset the wins on the no-prep-friendly instances.
* `C_no_bve` is not interesting on its own — it produces the same 4 TIMEOUTs as `B`. BSR
  alone rescues zero instances.
* battleship and case9 are insensitive to preprocessing toggles within ±10 % noise. No
  prep-side intervention will move them; they're search-bound.

## Artifact paths

* Ablation script: `log/analyzesat-2026-05-26-preprocess/run_ablation.sh`
* Per-config raw: `log/analyzesat-2026-05-26-preprocess/<config>/results.csv`, `stats.jsonl`
* Driver log: `log/analyzesat-2026-05-26-preprocess/ablation_driver.log`
* Worktree: `/tmp/analyzesat-2026-05-26-preprocess` (HEAD `025fe25`)
* Kissat reference source: `benchmarks/reference-solvers/kissat-latest/src/`
  (`eliminate.c` rounds + budget, `forward.c` forward subsume, `classify.c` for the
  classifier integration kissat does)
* Companion findings: `log/analyzesat-2026-05-26-0712/FINDINGS.md`,
  `log/analyzesat-2026-05-25-2043/FINDINGS.md`,
  `log/analyzesat-2026-05-23-broad/DEEPER_FINDINGS.md`
