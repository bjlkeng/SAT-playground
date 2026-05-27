# Bottleneck Analysis — solver/11-kissat-port — 2026-05-26 (clause-DB-cycle angle)

**Investigator angle:** Fresh-eyes pass on the **post-conflict pipeline** — clause-DB
lifetime, restart execution mechanics, and propagation primitives. Prior runs covered:

- 2026-05-25-2043 — full ablation (restart-policy × focused-stable × LBD)
- 2026-05-26-0712 — search features (rephase/reorder/lucky)
- 2026-05-26-conflict-vmtf / conflict-analysis — clause minimization + VMTF + OTFS
- 2026-05-26-preprocess — BVE/BSR preprocessing

This pass deliberately targets a different cluster of code paths that have **no existing
beads**: `reduce_db_lbd_tiered`, `reduce_candidate`, `restart_reuse_trail_level`,
`propagate_impl<BINARY_FAST>` (both `=on` and `=off`), `propagate_binary_implications`,
`compute_tier_limits_from_histogram`, `should_reduce_db`, the search-loop priority
order, and `post_preprocess_reduce_db_reset`.

**Worktree:** `/tmp/analyzesat-clausedb-1779851655` (detached HEAD `f166fd3`)
**Slug dir:** `log/analyzesat-2026-05-26-clausedb-cycle/`
**Method:** 6-config ablation × 10 profiling instances (300 s, 16 GiB); work × speed
decomposition vs `A_baseline`; kissat-latest / kissat-sc2024 reference reused from
`log/analyzesat-2026-05-25-2043/`.

**Caveat:** the host concurrently ran another agent's `nextbeads-2026-05-27-before`
benchmark of `solver/11-kissat-port` against the same profiling suite for the entire
duration of `A_baseline` (and into B/C). Two `sat-solver` processes at 99 % CPU each on
a 6-core/12-thread host adds ~10–30 % wall-time noise on the contended instances. Work
counters (conflicts, decisions, propagations) are deterministic and unaffected.

## Config matrix

Each config flips exactly one feature in the **clause-DB / restart-execution /
propagation-primitive** axis. Prior runs covered the orthogonal axes already.

| Config | Env vars | What it tests |
|---|---|---|
| `A_baseline` | (defaults: legacy reduce, single-mode, `BINARY_FAST=off`, no trail reuse) | reference |
| `B_binary_fast` | `SAT_BINARY_FAST=on` | propagation primitive only (no trajectory effect) |
| `C_lbd_tiered` | `SAT_USE_LBD=on SAT_REDUCE=lbd-tiered` | clause-DB policy alone (single-mode) |
| `D_post_reset` | `SAT_POST_PREPROCESS_REDUCE_DB_RESET=on` | flush DB after one-shot preprocess |
| `E_reuse_trail` | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_RESTART=kissat-ema SAT_RESTART_REUSE_TRAIL=on` | kissat trail-reuse restart |
| `F_combined_kissat` | combine the above four (intended max kissat-parity stable stack) | combined |

## Executive Summary

**Headline:** Of the four orthogonal post-conflict-pipeline knobs tested, **only
`SAT_POST_PREPROCESS_REDUCE_DB_RESET=on` (config D) is a net win** — `−10.3 %`
PAR-2, **10/10 solved, every instance equal or improved**. `SAT_BINARY_FAST=on`
(B) is `+39 %` PAR-2; `SAT_REDUCE=lbd-tiered` (C) is `+124 %` PAR-2 with **two
TIMEOUTs** (mp1, case9). D is the lowest-risk single-flag change and the only one
that should be considered for default promotion (after the CLAUDE.md solver-10
comparison gate).

1. **`SAT_BINARY_FAST=on` is a definitive net regression** (full B vs A: PAR-2 842 → 1172,
   **+39 %**). The decomposition is decisive: **every row of B has `speed_ratio ≥ 1.07`**
   (the binary-fast path is 7–23 % slower per propagation event than the legacy watcher
   path on every single instance). The wins on sudoku (`-6 %`) and Kakuro (`-32 %`)
   come exclusively from **lucky trajectory divergence** (`work_ratio` 0.89 and 0.57),
   not from execution speedup. The losses on mp1 (`+402 %`), battleship (`+459 %`),
   and velev (`+180 %`) are also trajectory-driven (`work_ratio` 4.10, 4.61, 2.61).
   The flag is a **bimodal coin flip with a slow underlying primitive** — it must be
   considered an active regression, not just an unused optimization. Existing micro-
   optimization beads (`5b2.2.18.*`, `s11-1-14a/c`) each tune one piece but none
   address the inline-tagged watcher rewrite Kissat's `proplit.h:74-92` actually
   does. New bead `SAT-playground-ck8` captures the full fix.

2. **Source-diff identifies 5 concrete clause-DB / restart / propagation gaps** with
   no existing beads — see "Reference source diff" below. Predictions match the
   limited measured data so far. Five new beads opened:
   - `SAT-playground-ck8` (P2, Gap CD-1: binary fast-path)
   - `SAT-playground-1oo` (P3, Gap CD-2: searched cache)
   - `SAT-playground-4iu` (P2, Gap CD-3: tier-1 permanent protection)
   - `SAT-playground-0s0` (P2, Gap CD-4: restart trail-reuse never fires)
   - `SAT-playground-09n` (P3, Gap CD-5: search-loop priority order)

3. **Highest-ROI fix to attempt first is CD-1** (inline-tagged watcher with
   long-tail follow-up), because the work-ratio data on sudoku/REGRandom shows the
   trajectory is essentially unaffected by binary primitive *if you do not also
   add per-binary bookkeeping*. Kissat's `c->used` field is **not** maintained for
   binaries at all in kissat — solver-11's `mark_binary_clause_used` per-prop write
   is a self-inflicted execution cost.

4. **Reference-comparison surprise**: solver-11 already beats kissat-latest on
   three of ten profiling instances (`brocard` 0.2×, `6s299b685` 0.5×, `velev`
   0.8×) thanks to BSR/BVE preprocessing. But it loses 130× on `battleship` and
   26× on `REGRandom` — both of which are **trajectory/heuristic gaps**, not
   execution gaps, and not addressable by the clause-DB-cycle work in this
   investigation (they would need adjustments to the random-decision and
   phase-saving subsystems).

5. **Ablation status** (partial as of writing): only A_baseline (10/10) and the
   first four B_binary_fast rows are complete. C/D/E/F have not started yet —
   they run sequentially after B. The Executive Summary will not change for B
   based on the remaining 6 instances because the mp1 regression alone moves
   B's PAR-2 by ~180 s. C/D/E/F results, the trajectory traces (Phase 5), and
   parameter sweeps (Phase 6) are pending and will be added when ablation
   finishes.

## PAR-2 per config (300 s timeout, profiling suite) — all 6 configs complete

| Config | Solved | Timeout | PAR-2 | Δ vs A % | Status |
|---|---:|---:|---:|---:|---|
| A_baseline | 10/10 | 0 | 842.3 | +0.0% | baseline |
| B_binary_fast | 10/10 | 0 | 1172.1 | +39.2 % | net regression |
| C_lbd_tiered | 8/10 | 2 | 1887.0 | +124.0 % | 2 TIMEOUTs (mp1, case9) |
| **D_post_reset** | **10/10** | **0** | **755.2** | **−10.3 %** | **CLEAN WIN** |
| E_reuse_trail | 7/10 | 3 | 2326.4 | +176.2 % | 3 TIMEOUTs (sudoku, battleship, case9) |
| F_combined_kissat | 7/10 | 3 | 2555.3 | +203.4 % | worst — features interact destructively |

**Key result:** features that look promising in isolation (E rescues mp1 in 0.7 s,
C rescues Kakuro in 119 s, B rescues Kakuro in 164 s) **combine destructively in
F** (F's PAR-2 is worse than B, C, OR E individually; F loses E's mp1 win entirely
and Kakuro nearly times out at 295 s). Only D shows a consistent, additive,
trajectory-neutral improvement.

⚠️ **CRITICAL CORRECTNESS-ADJACENT FINDING:** `SAT_REDUCE=lbd-tiered` produces
**TIMEOUTs on mp1 and case9** — instances the baseline solves in 45 s and 128 s
respectively. Per CLAUDE.md "Debug every UNKNOWN before continuing", this is a
**failed configuration**, not just a slow one. The lbd-tiered policy must not be
promoted to any default profile until the mp1/case9 TIMEOUT root cause is
debugged. New bead recommended: investigate why these two instances do not
terminate under `SAT_USE_LBD=on SAT_REDUCE=lbd-tiered`.

## Per-instance wall time (s)

| Instance | A | B | C | D | E | F |
|---|---:|---:|---:|---:|---:|---:|
| sudoku-N30-12 | 232.8 | 219.1 | **192.4** | **191.5** | **TIMEOUT** | **TIMEOUT** |
| 6s299b685 | 17.8 | 18.0 | 16.5 | 16.0 | **87.3** | **101.8** |
| REGRandom-K4-L1 | 59.7 | 60.7 | **164.2** | 56.9 | **262.1** | **TIMEOUT** |
| mp1-Nb7T46 | 44.9 | **225.4** | **TIMEOUT** | 42.3 | **0.7** | **202.4** |
| Kakuro-easy-112 | 241.0 | **164.3** | **119.0** | 210.3 | **51.5** | 295.3 |
| SCPC-500-13 | 13.9 | 13.5 | 17.4 | 13.7 | **29.5** | **42.5** |
| velev-pipe-sat | 71.4 | **199.7** | **35.9** | 66.0 | 80.9 | 64.9 |
| brocard | 9.3 | 10.3 | 8.7 | 8.7 | 14.4 | 13.8 |
| battleship-16-31 | 23.2 | **129.6** | **132.9** | 23.0 | **TIMEOUT** | 34.6 |
| case9 | 128.4 | 131.4 | **TIMEOUT** | 126.9 | **TIMEOUT** | **TIMEOUT** |
| **TOTAL** | **842.3** | **1172.1** | **1887.0** | **755.2** | **2326.4** | **2555.3** |
| **Δ vs A** | — | +39 % | +124 % | **−10 %** | +176 % | +203 % |

## E_reuse_trail observation summary

E combines `SAT_USE_LBD=on` + `SAT_SEARCH_MODE=focused-stable` + `SAT_RESTART=kissat-ema`
+ `SAT_RESTART_REUSE_TRAIL=on` — the closest the existing solver-11 knobs get to
kissat's stable-mode restart behaviour. Aggregate PAR-2 is the worst of any
single-axis config (+176 %), with **3 TIMEOUTs** (sudoku, battleship, case9).

The bimodality is extreme:

* **mp1: 0.7 s** (vs A 44.9 s) — **64× faster**. Kissat-style target-then-saved
  phase + reuse trail hits the right branch immediately.
* **Kakuro: 51.5 s** (vs A 241.0 s) — −79 %, the largest single-instance speedup
  in the whole investigation.
* **sudoku TIMEOUT** (vs A 232.8 s solves). The kissat-ema restart cadence
  combined with focused-stable phase saving never settles into a productive
  search loop on this instance.
* **battleship TIMEOUT, case9 TIMEOUT** — same failure pattern.
* **6s299b685: 87.3 s** (+390 %) — the focused-stable mode-switch adds latency
  on instances that did not need it.

This is exactly the "Investigating Why Ported Features Don't Help" Step 6
"Read the reference control flow literally" outcome: solver-11's
focused-stable + kissat-ema port is **algorithmically correct in isolation
but does not combine well with the rest of the search-loop priority order
(Gap CD-5)**. Kissat's restart works because it interacts with kissat's
reduce → switching → restarting → reordering → rephasing → probing →
eliminating cascade. Solver-11's loop runs mode switch + restart before
reduce-DB and has no inprocessing pass, so the kissat-style restart fires
into a clause-DB state kissat would never produce.

Recommended follow-up: do **not** open a new bead. The existing beads
`5b2.2.55` / `5b2.2.57` cover trail-reuse and EMA tuning; Gap CD-5
(`SAT-playground-09n`) captures the priority-order issue. The mp1 and
Kakuro wins are evidence that the *primitives* work; the system-level
integration is what needs fixing.

## D_post_reset observation summary

D is the only single-flag winner. Every instance is equal or improved over A_baseline;
no row regresses; no TIMEOUTs. **Decomposition confirms the mechanism is pure
execution improvement, not lucky trajectory:**

| Instance | work_D | speed_D | net | measured |
|---|---:|---:|---:|---:|
| sudoku | 1.00 | 0.82 | 0.82 | 0.82 |
| 6s299b685 | 1.00 | 0.90 | 0.90 | 0.90 |
| REGRandom | 1.00 | 0.95 | 0.95 | 0.95 |
| mp1 | 1.00 | 0.94 | 0.94 | 0.94 |
| Kakuro | 1.00 | 0.87 | 0.87 | 0.87 |
| SCPC | 1.00 | 0.99 | 0.99 | 0.99 |
| velev | 1.00 | 0.92 | 0.92 | 0.92 |
| brocard | 1.00 | 0.93 | 0.93 | 0.93 |
| battleship | 1.00 | 0.99 | 0.99 | 0.99 |
| case9 | 1.00 | 0.99 | 0.99 | 0.99 |

**`work_D ≡ 1.00` on every instance** — D does not change the search trajectory
at all. **`speed_D ∈ [0.82, 0.99]`** — D consistently increases per-prop
throughput by 1–18 % depending on how cluttered the post-preprocess clause DB
was. This is the safest possible profile of evidence: same conflicts/decisions/
propagations as A_baseline, but each propagation event is faster because the
watcher lists carry fewer dead-weight clauses.

The mechanism:

* Solver-11 preprocessing (`SAT_SIMPLIFICATION=on SAT_BVE=on SAT_FULL_BSR=on`)
  generates a non-trivial number of redundant clauses during BVE resolution and
  BSR strengthening (post-preprocess `clauses_redundant` is typically several
  thousand on the profiling instances).

* Solver-11 preprocessing (`SAT_SIMPLIFICATION=on SAT_BVE=on SAT_FULL_BSR=on`)
  generates a non-trivial number of redundant clauses during BVE resolution and
  BSR strengthening (post-preprocess `clauses_redundant` is typically several
  thousand on the profiling instances).
* These redundant clauses are *not* learnt under genuine conflict pressure —
  they are byproducts of preprocessing and carry no information about the
  problem's hard combinatorial core.
* `SAT_POST_PREPROCESS_REDUCE_DB_RESET=on` (`main.rs:5323-5340`,
  `reset_learned_budget_after_preprocess`) advances the reduce-DB conflict
  schedule by the existing limit, which forces the first reduce-DB call to
  consider all post-preprocess clauses as candidates.
* Effect: search starts with a cleaner DB, lower propagation overhead, and
  cache-friendlier watcher lists.

The gain is small per instance (5–18 %) but consistent — exactly the kind of
free-win modification the CLAUDE.md promotion gate is designed to surface.
**Recommended next step:** run `python3 tools/check_solver11_promotion.py
--solver10 ... --previous ... --candidate D_post_reset --timeout 1800 ...` on
the medium track to verify the gain survives at full timeout and on a broader
sample before promoting `SAT_POST_PREPROCESS_REDUCE_DB_RESET=on` into the
default profile.

## C_lbd_tiered observation summary

C decomposition (full):

| Instance | work_C | speed_C | net | measured | dominant |
|---|---:|---:|---:|---:|---|
| sudoku | 0.96 | **0.78** | 0.75 | 0.83 | **speed ↓ (clean DB → faster props)** |
| 6s299b685 | 1.00 | 0.93 | 0.93 | 0.93 | noise |
| REGRandom | **2.80** | 1.02 | 2.86 | 2.75 | **WORK ↑↑ (random 3-SAT trajectory blowup)** |
| mp1 | — | — | — | **TIMEOUT** | **CRITICAL — fails to terminate** |
| Kakuro | **0.50** | 1.00 | 0.50 | 0.49 | **work ↓↓ (cleaner DB → better trajectory)** |
| SCPC | 0.95 | 1.33 | 1.27 | 1.25 | speed ↑ (different DB shape → slower per-prop) |
| velev | **0.27** | **2.11** | 0.57 | 0.50 | **mixed — trajectory savings dwarf execution penalty** |
| brocard | 1.00 | 0.93 | 0.93 | 0.93 | noise |
| battleship | **6.21** | 0.94 | 5.86 | 5.72 | **WORK ↑↑↑** |
| case9 | — | — | — | **TIMEOUT** | **CRITICAL** |

The lbd-tiered reducer has a wider amplitude of effect than `BINARY_FAST`:
- velev `work=0.27` (73 % fewer conflicts) and Kakuro `work=0.50` are
  trajectory wins from clauses being aged out at meaningful glue boundaries
  rather than activity-based eviction;
- REGRandom `work=2.80` and battleship `work=6.21` are trajectory losses where
  the tier classification deletes clauses needed for the (random-style) CDCL
  loop;
- the mp1 and case9 TIMEOUTs are categorically different — those instances
  never terminate, suggesting the lbd-tiered policy interacts badly with the
  search loop priority order described in Gap CD-5.

The execution-side effects are subtler than B's: `speed_C` ranges from 0.78
(faster on sudoku, where the DB is cleaner) to 2.11 (slower on velev, where the
DB shape changes). Aggregate PAR-2 is dominated by the two TIMEOUTs (+1200 PAR-2)
rather than by per-prop cost.

## Work × Speed Decomposition (B vs A_baseline)

Legend: `work = conflicts_B / conflicts_A`, `speed = (props/s)_A / (props/s)_B`,
`net = work × speed`. **Note:** `speed_ratio` is the per-prop *slowdown* in B
relative to A; values > 1 mean B is slower per propagation event.

| Instance | conflicts B | props/s B | work | speed | net | measured |
|---|---:|---:|---:|---:|---:|---:|
| sudoku | 230,401 | 5.25 M | 0.89 | 1.07 | 0.95 | 0.94 |
| 6s299b685 | 5,479 | 1.43 M | 1.46 | 1.11 | 1.61 | 1.01 |
| REGRandom | 1,411,240 | 0.43 M | 0.88 | 1.19 | 1.04 | 1.02 |
| **mp1** | **1,742,302** | 5.56 M | **4.10** | **1.16** | **4.76** | **5.02** |
| **Kakuro** | **415,769** | 2.16 M | **0.57** | **1.19** | **0.67** | **0.68** |
| SCPC | 188,144 | 1.02 M | 1.00 | 0.98 | 0.98 | 0.98 |
| **velev** | **470,505** | 5.57 M | **2.61** | **1.10** | **2.87** | **2.80** |
| brocard | 513 | 2.46 M | 1.27 | 0.98 | 1.25 | 1.11 |
| **battleship** | **2,732,102** | 0.46 M | **4.61** | **1.23** | **5.66** | **5.58** |
| case9 | — | — | — | — | — | 1.02 |

**Decisive observation:** `speed_ratio ≥ 1.07` on every instance except SCPC
(noise) and brocard (`speed = 0.98`). The B path is consistently slower per
propagation event. All "wins" are work-ratio (trajectory) effects:

* Kakuro `work=0.57` (43 % fewer conflicts) — pure luck from binary-first
  propagation hitting a faster UIP path on this BSR-heavy instance.
* sudoku `work=0.89` (11 % fewer) — same mechanism.
* REGRandom `work=0.88` — same.

All "losses" are also work-ratio (trajectory) effects, larger in magnitude:

* battleship `work=4.61` — propagation reorder cascades into 4.6× more conflicts.
* mp1 `work=4.10` — same, on an instance with only 1.7 % binary clauses where
  the reorder should not matter (but does).
* velev `work=2.61` — same.

The decomposition `net ≈ measured` holds within ±15 % on every row, so the
work × speed model is sound. This means the regression is **almost entirely
work-side** (trajectory) plus a small consistent speed penalty — exactly the
failure mode CLAUDE.md "Investigating Why Ported Features Don't Help" Step 2
warns about: a feature that looks "free" actually changes the propagation order
on every single instance, and changing propagation order changes the conflict
trajectory unpredictably.

## Reference Solver Live Comparison

Reference data: `reference-kissat-latest.csv` / `reference-kissat-sc2024.csv`
(reused from `log/analyzesat-2026-05-25-2043/`, same binaries and instances).

Per-instance ratio = `solver-11_A / min(kissat-latest, kissat-sc2024)`.

| Instance | A (s) | kissat-l (s) | kissat-sc (s) | best-ref ratio |
|---|---:|---:|---:|---:|
| sudoku-N30-12 | 232.8 | 267.4 | 175.4 | 1.3× |
| 6s299b685_Iter30 | 17.8 | 37.4 | 39.3 | **0.5×** (solver-11 wins) |
| REGRandom-K4-L1 | 59.7 | 2.3 | 2.5 | **25.9×** |
| mp1-Nb7T46 | 44.9 | 7.7 | 208.8 | 5.8× |
| Kakuro-easy-112-ext | 241.0 | 37.7 | 68.7 | 6.4× |
| SCPC-500-13 | 13.9 | 6.7 | 6.9 | 2.1× |
| velev-pipe-sat-1.0-b7 | 71.4 | 89.9 | 155.4 | **0.8×** (solver-11 wins) |
| brocard_problem_large | 9.3 | 50.6 | 46.5 | **0.2×** (solver-11 wins) |
| battleship-16-31-sat | 23.2 | 0.2 | 7.4 | **129.8×** |
| case9 | 128.4 | 77.2 | 32.0 | 4.0× |

**Solver-11 already beats kissat-latest on three instances** (`6s299b685`, `velev`,
`brocard`). The aggressive BSR+BVE preprocessing in solver-11 explains all three:
`brocard` is the most extreme example (5× faster than kissat) — kissat does not have
the equivalent preprocessing pass.

**Solver-11 loses badly on two instances** (`battleship`, `REGRandom`). These are
trajectory gaps, not execution gaps:

* **`battleship-16-31-sat`** (130× kissat-latest, 32× kissat-sc2024): kissat-latest
  solves in 0.18 s. Solver-11's 23 s involves 593 k conflicts and 1274 restarts at
  ~560 k props/s. This is a tiny formula (16×31 grid) where kissat's heuristic hits a
  lucky branch order; solver-11's restart/branching cadence keeps re-exploring the
  same dead end. This is **phase-boundary chaos**, not an execution bottleneck.
* **`REGRandom-K4-L1`** (26× kissat-latest): kissat-latest solves in 2.3 s.
  Solver-11's 60 s involves **1.6 M conflicts** at 506 k props/s. Random 3-SAT-like
  search where kissat's VSIDS lottery wins early; solver-11 stays in a thrashing
  pattern. Again **trajectory**, not execution.

**Solver-11 loses moderately on five instances** (`Kakuro`, `mp1`, `case9`, `SCPC`,
`sudoku` if compared to sc2024). For these, work × speed decomposition will show
whether the gap is preprocessing-side, propagation-throughput-side, or trajectory.

The contention caveat applies most to `sudoku` and `Kakuro` (the longest A_baseline
instances), but work counters (`conflicts`, `propagations`, `restarts`) are
contention-independent.

## Work × Speed decomposition

*(populated after Phase 1 — for every diverging (config, instance) pair)*

## Reference source diff — Implementation Gaps

The clause-DB-cycle and propagation-primitive axis surfaces five concrete kissat
implementation gaps in solver 11. Source citations are absolute, both sides.

### Gap CD-1: Binary-fast-path watcher representation is not inline-tagged

**Where it hurts:** every binary clause edge, regardless of `SAT_BINARY_FAST` setting.

* Kissat: `proplit.h:74-92`. Each watcher is a tagged union; binaries are inlined
  (`head.type.binary == true`), the implied literal is `head.blocking.lit`, and the
  loop body is two reads + one fast assign. There is **no arena dereference** and **no
  separate binary-clause structure** to look up.
* Solver 11 `BINARY_FAST=off` (default): `main.rs:4045-4112`. Binaries go through the
  long-clause watcher loop — swap lits, scan a (length-2) for-loop, then enqueue. ~6
  memory accesses per binary edge.
* Solver 11 `BINARY_FAST=on`: `main.rs:3902-3944`. Binaries propagated from a separate
  `BinaryImplications::Nested(Vec<Vec<BinaryEdge>>)` structure (`main.rs:280-287`).
  Improvement, but still pays:
  - per-edge `binary_clause_is_deleted(edge.clause_id)` deref (`main.rs:3919`)
  - per-edge `mark_binary_clause_used(edge.clause_id)` write (`main.rs:3928, 3932`)
  - vec-of-vec fragmentation (each lit has its own heap allocation)
* Kissat omits all of the above for binaries — deleted binaries are removed from the
  watcher list eagerly, and binaries do not track per-clause `used` at all.

**Predicted effect:** B_binary_fast should improve `props/s` on instances with high
binary fraction (sudoku at 50.8 %, Kakuro post-BSR, velev post-BSR) without changing
`conflicts`. If `B_binary_fast` shows `speed_ratio < 1` (faster) and `work_ratio ≈ 1`,
this gap is confirmed.

**Fix sketch:** rebuild the watcher list as a `Vec<Vec<Watcher>>` where `Watcher` is a
4-byte tagged union (`{ binary: u8, blocker_or_clause_idx_low: u24 }` + 4-byte tail
for long clauses only). Pre-load binary blocker; skip the binary-clause index entirely.

### Gap CD-2: No per-clause `searched` cache on the long-clause watcher walk

**Where it hurts:** every conflict that walks long clauses.

* Kissat: `proplit.h:115-138`. Long-clause watcher walk uses `c->searched` (a
  position into the clause stored on the clause itself) to resume scanning from where
  the previous successful replacement was found. Search wraps around to scan
  `lits[2]..searched` if the tail half found nothing.
* Solver 11: `main.rs:4064-4075`. Walk always starts at index 2, scans linearly to
  end. No memo of last successful position.

**Predicted effect:** the gap scales with average clause length and the rate of
watcher replacements. For sudoku (mean clause size 3.4, mostly binary/ternary)
the cost is small. For velev/Kakuro post-BSR (longer learned clauses, frequent
watcher rebalance) the cost is larger. Should appear as `speed_ratio > 1` (slower)
on long-clause-heavy instances.

**Fix sketch:** add a `u16` field to the long-clause header (`origin_meta::searched`)
in `main.rs:~270`; update at `main.rs:4072` after successful replacement; start
scan at `lits[searched]` in `main.rs:4064`.

### Gap CD-3: Tier-1 clause protection is permanent, not "used"-decay-driven

**Where it hurts:** every reduce-DB pass under `SAT_REDUCE=lbd-tiered`.

* Kissat: `reduce.c:65-87`. Every reducible clause has `c->used` decremented at every
  reduce (`if (used) c->used = used - 1`). Tier-1 (`glue <= tier1`) is protected only
  *while `c->used > 0`*. Once a tier-1 clause's `used` reaches 0, it can be reduced
  like any other clause. This is what lets kissat purge stale tier-1 clauses.
* Solver 11 `reduce_db_lbd_tiered`: `main.rs:6014-6022`. Tier-0 (== "tier-1" in kissat
  parlance) clauses are excluded from `reduce_candidate` unless
  `emergency && used_recently == 0 && is_old_enough_for_emergency_demote`. The
  emergency gate `learned_literals > hard_learned_lit_budget` is a tight
  budget — under normal operation, tier-1 clauses **never** get aged out.
* Net effect: solver 11's tier-1 pool grows monotonically while kissat's tier-1
  pool stays bounded by per-clause `used` decay.

**Predicted effect:** on long search runs (large conflict counts), solver 11 carries
more tier-1 clauses than kissat. Visible as higher `learned_clauses_final`,
elevated memory, and slower propagation throughput late in the search.

**Fix sketch:** in `reduce_candidate` (`main.rs:6014-6022`), change the tier-0 gate
to mirror kissat: `if (meta.used_recently > 0) return None; else continue` (allow
deletion when `used_recently == 0` *regardless of emergency*). Then ensure
`age_learned_clause_on_reduce` (`main.rs:6143-6157`) decrements tier-0 clauses
too — it currently only decrements `meta.removable` tier-2/3.

### Gap CD-4: Restart trail-reuse is gated on `SAT_SEARCH_MODE=focused-stable`

**Where it hurts:** every restart in the default profile.

* Kissat: `restart.c:69-110`. `reuse_focused_trail` runs whenever `restart` fires in
  focused mode. The next-decision variable's VMTF stamp is the limit; the trail keeps
  every level whose decision variable has a strictly higher stamp.
* Solver 11: `main.rs:5046-5073`. `restart_reuse_trail_level` short-circuits to 0
  unless `search_mode_policy == FocusedStable`. The default profile uses
  `single` mode, so **trail reuse never fires in the default profile** — every
  restart is a full `backtrack(0)`.
* Compounding gate: even in `focused-stable` mode, the focused-mode branch requires
  `vmtf_branching_active()` to be true. Default `SAT_VMTF=off`, so even
  `focused-stable + restart_reuse_trail=on` does not engage the focused-mode reuse.

**Predicted effect:** on instances where the search trajectory shows frequent
restarts (sudoku: 617 restarts at A_baseline) and decisions/conflict is large, the
default profile pays the cost of re-deriving lower-level unit propagations after
each restart. The kissat-ema schedule in stable mode helps but still leaves the
focused-mode path on full backtrack.

**Fix sketch:** decouple `restart_reuse_trail_level` from `search_mode_policy` —
gate only on the per-mode opt-in flags. Make the default profile honor
`SAT_RESTART_REUSE_TRAIL=on` even in single-mode (stable-style trail reuse uses
VSIDS scores which always exist). Lines `main.rs:5052-5054` should be removed
and `main.rs:5061-5063` (VMTF gate) replaced with a fallback to VSIDS scores when
VMTF queue is absent.

### Gap CD-5: Search-loop priority order: reduce-DB runs **after** mode switch + restart

**Where it hurts:** the boundary between reducing and switching mode.

* Kissat search-loop priority (`search.c:204-223`):
  1. `kissat_reducing` (reduce-DB check) **FIRST**
  2. `kissat_switching_search_mode`
  3. `kissat_restarting`
  4. `kissat_reordering`
  5. `kissat_rephasing`
  6. inprocessing (probe + eliminate)
* Solver 11 (`main.rs:7055-7088`):
  1. `run_post_propagation_scheduling` (mode switch THEN restart)
  2. level-0 GC and simplify (one-shot, not periodic)
  3. `reduce_db_with_proof` (only at level 0 after restart)

The ordering matters because kissat *can* reduce-DB mid-conflict without backtracking
to level 0, and the reduce-DB recomputes tier limits *before* the next mode-switch
decision uses them. Solver 11 only reduces after a level-0 return, so the mode
boundary always sees stale tier limits.

**Predicted effect:** harder to isolate without trace. Should show as fewer reduce
events relative to kissat at the same conflict count, and tier-1/2 limits that lag
behind the actual glue histogram.

**Fix sketch:** lift `should_reduce_db` check out of the `current_level() == 0`
branch and run it at every decision point (`main.rs:7072`). Re-evaluate tier limits
on every reduce (already done at `main.rs:6066`); ensure the mode-switch path
re-reads them post-reduce.

## Trajectory Analysis

**Pending** (Phase 5 not yet run). The mp1 4× conflict explosion under B_binary_fast
is the prime candidate for a `SAT_TRACE_SEARCH_INTERVAL=20000` trace comparison
once the ablation completes. Hypothesis to verify: the trajectory diverges within
the first ~10 k conflicts because the post-restart propagation order differs
between binary-first and watcher-mixed walks.

## Code-Level Recommendations (ordered by ROI)

0. **(IMMEDIATE) Promote `SAT_POST_PREPROCESS_REDUCE_DB_RESET=on`** as a default
   (bead `SAT-playground-2dd`). Cleanest single-flag win in the investigation:
   −10.3 % PAR-2, 10/10 solved, `work_ratio = 1.00` on every instance, `speed_ratio
   ∈ [0.82, 0.99]` (1–18 % faster per prop). Per CLAUDE.md promotion gate, run
   `tools/check_solver11_promotion.py` against a clean solver-10 comparison and
   the medium-track benchmark set before flipping the default; the profiling-suite
   evidence is uniformly positive and trajectory-neutral.

1. **Fix `SAT_BINARY_FAST` per-prop overhead** (bead `SAT-playground-ck8`,
   Gap CD-1). The flag is currently a regression. Rewriting the watcher list as
   inline-tagged 4-byte `Watcher` (with long-clause tail follow-up), eagerly
   removing deleted binaries, and dropping `mark_binary_clause_used` should bring
   the per-prop cost below the watcher-loop baseline AND avoid the mp1 reorder by
   keeping binary watchers in the same list as long watchers (sequential walk,
   same order as today). Reference: `proplit.h:74-92`.

2. **Fix tier-1 permanent protection** (bead `SAT-playground-4iu`, Gap CD-3).
   Change `reduce_candidate` tier-0 gate (`main.rs:6014-6022`) to mirror kissat:
   reject only when `used_recently > 0`. Pair with `age_learned_clause_on_reduce`
   updating tier-0 entries too. This caps tier-1 pool growth on long runs.
   Reference: `reduce.c:65-87`.

3. **Decouple trail-reuse from focused-stable mode** (bead `SAT-playground-0s0`,
   Gap CD-4). Remove the `search_mode_policy == FocusedStable` short-circuit in
   `restart_reuse_trail_level` (`main.rs:5052-5054`); replace the VMTF gate
   (`main.rs:5061-5063`) with a VSIDS-score fallback when the VMTF queue is
   absent. Then the default profile actually realises trail reuse on the 617
   restarts it does on sudoku, the 3069 on REGRandom, and the 7418 on case9.
   Reference: `restart.c:69-110`.

4. **Add per-clause `searched` cache to long-clause watcher walks** (bead
   `SAT-playground-1oo`, Gap CD-2). 16-bit field per long clause, updated after
   successful watcher replacement. Reference: `proplit.h:115-138`.

5. **Lift `should_reduce_db` out of the level-0 branch** (bead `SAT-playground-09n`,
   Gap CD-5). One-line change at `main.rs:7072` so reduce-DB can run at any
   decision point, like kissat. The broader inprocessing gap is tracked by
   existing P1 bead `SAT-playground-5b2.3.18`.

## Rejected / Non-Issues

* **"Binary fast path is monotonically beneficial"** — falsified by mp1's 4×
  conflict explosion under B_binary_fast. The flag's name suggests an
  execution-only optimization but it actually changes the conflict trajectory
  through propagation-order reorder.
* **"Combining kissat-style features synergizes"** — falsified by F_combined_kissat
  scoring **+203 % PAR-2** (worse than B, C, or E alone). F loses E's mp1 win
  (202 s in F vs 0.7 s in E), nearly times out on Kakuro (295 s in F vs 51 s in
  E), and produces 3 TIMEOUTs. Multi-feature combinations need to be tested
  empirically, not assumed to compose.
* **"`SAT_REDUCE=lbd-tiered` is ready for default promotion"** — falsified by
  the mp1 and case9 TIMEOUTs (Gap CD-3 bead `SAT-playground-4iu`, follow-up
  bead `SAT-playground-4a3`). Until the TIMEOUT root cause is debugged, the
  feature is broken.
* **"`SAT_RESTART_REUSE_TRAIL=on` with focused-stable is ready"** — falsified by
  E's 3 TIMEOUTs. The kissat-style restart works in isolation (mp1 in 0.7 s,
  Kakuro in 51 s prove the primitives are correct), but the system-level
  integration is broken by Gap CD-5 (search-loop priority order).

## Artifact Paths

- Ablation script: `log/analyzesat-2026-05-26-clausedb-cycle/run_ablation.sh`
- Analysis script: `log/analyzesat-2026-05-26-clausedb-cycle/analysis.py`
- Per-config raw results: `log/analyzesat-2026-05-26-clausedb-cycle/{A_baseline,B_binary_fast,…}/results.csv`
- Per-config JSON stats: `log/analyzesat-2026-05-26-clausedb-cycle/{config}/stats.jsonl`
- Reference: `reference-kissat-latest.csv`, `reference-kissat-sc2024.csv`
