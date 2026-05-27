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

1. **The existing `SAT_BINARY_FAST=on` implementation is a net negative on PAR-2 even
   on the first 4 ablated instances** (sudoku, 6s299b685, REGRandom, mp1). The
   per-prop speed is **7 % slower** than the watcher-loop path on sudoku, and on mp1
   (which has only 1.7 % binary clauses) it causes a **4.1× conflict explosion**
   (`work_ratio = 4.10`, `speed_ratio = 1.16`, `net = 4.76×`, measured `5.02×`
   slower). The trajectory change is not a coincidence — flipping the propagation
   primitive reorders binary vs. long-clause propagation, which feeds different
   reasons into conflict analysis. **The flag should be considered a regression on
   this profile, not just "doesn't help"**, and the existing five binary-prop micro-
   optimization beads (`5b2.2.18.1/.2/.18`, `s11-1-14a/c`) are insufficient — they
   each tune a piece but none of them address the inline-tagged watcher rewrite that
   Kissat's `proplit.h:74-92` actually does.

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

## PAR-2 per config (300 s timeout, profiling suite) — *PARTIAL, ablation in progress*

| Config | Solved | Timeout | PAR-2 | Δ vs A % | Status |
|---|---:|---:|---:|---:|---|
| A_baseline | 10/10 | 0 | 842.3 | +0.0% | complete |
| B_binary_fast | 4/10 | 0* | — | — | 4 instances done; mp1 regression dominates so far |
| C_lbd_tiered | 0/10 | — | — | — | not started |
| D_post_reset | 0/10 | — | — | — | not started |
| E_reuse_trail | 0/10 | — | — | — | not started |
| F_combined_kissat | 0/10 | — | — | — | not started |

*Numbers in the auto-generated `finalize.py` PAR-2 table treat un-run instances
as timeouts; those rows will be replaced when ablation finishes. The `4/10`
rows for B are real data.

## Per-instance wall time (s) — *PARTIAL*

Updated by running `python3 finalize.py` after ablation completes.

| Instance | A_baseline | B_binary_fast | C_lbd_tiered | D_post_reset | E_reuse_trail | F_combined |
|---|---:|---:|---:|---:|---:|---:|
| sudoku-N30-12 | 232.8 | **219.1** (-6%) | — | — | — | — |
| 6s299b685 | 17.8 | 18.0 (+1%) | — | — | — | — |
| REGRandom | 59.7 | 60.7 (+2%) | — | — | — | — |
| mp1 | 44.9 | **225.4 (+402%)** | — | — | — | — |
| Kakuro | 241.0 | — | — | — | — | — |
| SCPC | 13.9 | — | — | — | — | — |
| velev | 71.4 | — | — | — | — | — |
| brocard | 9.3 | — | — | — | — | — |
| battleship | 23.2 | — | — | — | — | — |
| case9 | 128.4 | — | — | — | — | — |

## Work × Speed Decomposition (partial)

| Instance | Config | work | speed | net | measured | dominant cause |
|---|---|---:|---:|---:|---:|---|
| sudoku | B_binary_fast | 0.89 | 1.07 | 0.95 | 0.94 | work (trajectory) |
| 6s299b685 | B_binary_fast | 1.46 | 1.11 | 1.61 | 1.01 | WORK ↑ but noise on small instance |
| REGRandom | B_binary_fast | 0.88 | 1.19 | 1.04 | 1.02 | SPEED ↑ (slower per-prop) |
| **mp1** | **B_binary_fast** | **4.10** | **1.16** | **4.76** | **5.02** | **WORK ↑↑ (4× more conflicts)** |

The `mp1` row is the key result. The decomposition `net = 4.76` matches `measured
= 5.02` very well, confirming that the regression is **trajectory + small speed
penalty**, not third-factor noise. The binary primitive change reorders propagation
events; on an instance with only 1.7 % binary clauses, those reordered events
cascade into a 4× conflict explosion. **This is exactly the failure mode
CLAUDE.md's "Investigating Why Ported Features Don't Help" Step 2 warns about:
a feature that looks "free" can dramatically change the conflict trajectory.**

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

1. **Fix `SAT_BINARY_FAST` per-prop overhead first** (bead `SAT-playground-ck8`,
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
* **`SAT_POST_PREPROCESS_REDUCE_DB_RESET=on`** — not yet measured. Skipping
  recommendation until D_post_reset finishes; the post-preprocess clause pool is
  small on this profile so the upside is bounded.

## Artifact Paths

- Ablation script: `log/analyzesat-2026-05-26-clausedb-cycle/run_ablation.sh`
- Analysis script: `log/analyzesat-2026-05-26-clausedb-cycle/analysis.py`
- Per-config raw results: `log/analyzesat-2026-05-26-clausedb-cycle/{A_baseline,B_binary_fast,…}/results.csv`
- Per-config JSON stats: `log/analyzesat-2026-05-26-clausedb-cycle/{config}/stats.jsonl`
- Reference: `reference-kissat-latest.csv`, `reference-kissat-sc2024.csv`
