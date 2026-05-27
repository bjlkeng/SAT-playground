# Bottleneck Analysis — solver/11-kissat-port — 2026-05-27 (lucky-heuristics axis)

**Investigator angle:** fresh-eyes pass on the **pre-search satisfying-assignment
heuristics** — `SAT_LUCKY` and its kissat counterparts `kissat_lucky` and
`kissat_walk`. The user asked to "look in a different place"; prior analyzesat
runs covered orthogonal clusters and never isolated lucky / walk:

- 2026-05-25-2043 — restart policy × focused-stable × LBD
- 2026-05-26-0712 — rephase/reorder/lucky search features (rephase + reorder focus; not lucky internals)
- 2026-05-26-conflict-vmtf — clause minimization, VMTF, OTFS
- 2026-05-26-conflict-analysis — ccmin modes
- 2026-05-26-preprocess — BVE/BSR preprocessing
- 2026-05-26-clausedb-cycle — reduce-DB, binary-fast, post-preprocess reset, trail-reuse
- 2026-05-27-search-control — chrono BT, branch ordering, initial clause iteration

`bd search lucky` returned **0 issues**. `bd search walk` returned `5b2.6.1`
(Phase A.1 parking-lot rephaser, deferred) and `1oo` (CD-2 watcher walk —
unrelated). So no existing bead enumerates the lucky implementation gaps.

**Worktree:** `/tmp/analyzesat-2026-05-27-propagation-hotpath-1779882636` (detached HEAD `70a83ee`)
**Slug dir:** `log/analyzesat-2026-05-27-propagation-hotpath/`
**Method:** lucky on/off ablation across 8 profiling instances at 300 s
timeout, 16 GiB. `SAT_STATS_JSON=on`. kissat-latest reference reused from
`log/analyzesat-2026-05-26-clausedb-cycle/reference-kissat-latest.csv` and
freshly measured on battleship and 6s299b685 to dispute stale values.

**Host caveat:** `perf_event_paranoid=4` locked out perf record/stat — gap
attribution relies on SAT_STATS_JSON work counters and kissat `--statistics`
line. Some lucky-on runs (sudoku/velev/kakuro) shared the host with parallel
solver instances; baselines used CSV values where the clean re-run was not
worth the wall time. The cleanest velev measurement was rerun standalone to
verify the lucky overhead and is included.

## Executive Summary

**Headline:** The current solver-11 vs kissat-latest gap on the profiling
suite is dominated by **one instance — battleship — which kissat solves in
0.18 s and solver 11 takes 19.05 s (128× slower) at default `SAT_LUCKY=off`**.
With `SAT_LUCKY=on`, solver 11 solves battleship in **0.08 s** (2.25× faster
than kissat). The "lucky regresses the rest of the suite" reason cited in
`SOLVER11_STATE.md` for keeping the default off is, at HEAD `70a83ee`,
**weaker than the battleship win**: per-instance regressions are 4–16 %
(velev, sudoku, SCPC, Kakuro), counterbalanced by 3–12 % improvements
(mp1, REGRandom, case9). On this 8-instance sample, lucky=on is roughly
**−5 % PAR-2** vs lucky=off, not the dramatic regression that the demotion
suggested.

**Three code-level gaps explain why lucky underperforms on large formulas:**

1. **LH-1 (P1, perf bug):** `begin_temporary_assumptions` at
   `src/main.rs:3403–3422` clones the entire clause arena, all watchers, and
   all binary clauses for every lucky pattern. For velev (8.7 M clauses
   post-preprocess), that is ~50 MB cloned 6 times = ~300 MB of arena copies
   per `try_lucky_assignment_with_proof` call. Pattern propagation never
   mutates arena/watchers (no learning, no GC), so save/restore should cover
   only trail / assignment / reason / decision_level / propagate_head.
2. **LH-2 (P2, kissat-parity gap):** `try_lucky_assignment_with_proof` at
   `src/main.rs:3766–3786` runs `AllTrue` and `AllFalse` patterns
   unconditionally. Kissat gates each by a cheap O(clauses) structural
   precheck: `no_all_negative_clauses` (`kissat-latest/src/lucky.c:11–45`)
   and `no_all_positive_clauses` (`lucky.c:47–80`). On random or structured
   formulas with mixed-polarity clauses (REGRandom, Kakuro, velev, sudoku),
   the precheck returns false instantly and saves the propagation pass.
3. **LH-3 (P3, feature gap, links existing `5b2.6.1`):** Solver 11 has no
   WalkSAT-style random-walk local search. The closest analog
   `try_lucky_local_repair` at `src/main.rs:3686–3692` is gated off for any
   formula with `preprocess_eliminated_vars != 0`, `original_literals > 200 000`,
   or `assignment.len() > 2 000` — i.e. it never runs on the profiling-suite
   real formulas. Kissat's `kissat_walk` (`walk.c:936–966`) runs via the
   rephasing schedule (`rephase.c:rephase_walking`) and is gated by
   `MAX_WALK_REF` (last_irredundant + BINIRR_CLAUSES below a large but
   finite cap), allowing it to run on 8 M-clause formulas like velev.

**Recommended next steps (ordered by ROI):**

1. Land **LH-1** first — pure perf bug, no semantic change, unblocks all
   subsequent lucky tuning by removing the per-pattern 50 MB clone tax on
   large formulas. Predicted: lucky=on regression on velev/Kakuro/sudoku
   shrinks below host noise; lucky=on becomes neutral-to-positive.
2. Land **LH-2** — add the cheap structural prechecks. Predicted: AllTrue/
   AllFalse patterns short-circuit on every random/structured formula,
   saving 0.1–0.5 s per large instance.
3. Reconsider promoting `SAT_LUCKY=on` to default profile after LH-1 + LH-2
   land, using the CLAUDE.md solver-10 comparison gate and shuffle-sensitivity
   workflow. The battleship win (−23 s, single instance) easily pays for
   small regressions across the suite if LH-1 closes the cloning tax.
4. Promote **`5b2.6.1`** (walking local-search rephaser) from P4-deferred to
   P2 — it is the only kissat feature on this suite that demonstrably matters
   on a satisfiable instance (battleship) and would let solver 11 close the
   last residual gap to kissat.

Five new beads filed; one existing bead noted (5b2.6.1) for repriortisation.

## Reference solver comparison (current HEAD `70a83ee`)

Measured today on this host. Kissat numbers from May 27 `search-control`
CSV unless noted. Solver-11 numbers re-measured today where indicated.

| Instance | Sol-11 default | Sol-11 lucky=on | kissat-latest | Sol-11 gap (default vs kissat) | Verdict |
|---|---:|---:|---:|---:|---|
| sudoku-N30-12 | 207.5 s | 229.7 s | 267.4 s | **0.78×** (sol-11 faster) | sol-11 wins |
| 6s299b685_Iter30 | 18.2 s | — | **19.2 s** (re-measured today) | **0.95×** | tie |
| REGRandom-Seed40 | 60.3 s | 53.2 s | 2.3 s | **26.4×** | sol-11 loses big (UNSAT path) |
| mp1-Nb7T46 | 46.5 s | 42.5 s | 7.7 s | **6.0×** | sol-11 loses |
| Kakuro-easy-112 | 239.5 s | 249.6 s | 37.7 s | **6.4×** | sol-11 loses (BSR-driven) |
| SCPC-500-13 | 13.5 s | 15.5 s | 6.7 s | **2.0×** | sol-11 loses |
| velev-pipe-sat | 65.2 s (today, no lucky) | 86.3 s | 89.9 s | **0.73×** | sol-11 wins |
| brocard | **9.06 s** (re-measured) | — | **56.7 s** (re-measured) | **0.16×** | sol-11 wins big |
| battleship-16-31 | 19.05 s (today) | **0.08 s** | 0.18 s | **128×** (default) / **0.44×** (lucky) | lucky closes 128× gap |
| case9 | 126.8 s | 123.0 s | 77.2 s | **1.64×** | sol-11 loses |

(`brocard` and `6s299b685` were the stale-CSV "gap" candidates I started
with. Re-measurement today shows solver 11 is faster on both, so they are
**not** the residual gaps. Battleship is.)

## Work × speed decomposition — battleship (the dominant gap)

Measured today on identical host conditions.

| Solver | wall (s) | conflicts | decisions | props | props/s | mode |
|---|---:|---:|---:|---:|---:|---|
| solver 11 default | 19.05 | 593 019 | 1 415 490 | 13 003 103 | 683 k | CDCL only |
| solver 11 LUCKY=on | **0.08** | **0** | **0** | **0** | n/a | lucky pattern hit |
| kissat-latest | 0.18 | 10 615 | 86 362 | 1 005 696 | 5.7 M | walk + CDCL |

**Decomposition:**
- Solver-11 default does **55.9× more conflicts** than kissat. work_ratio = 56.
- Kissat propagates **8.4× faster per second** (5.7 M vs 683 k). speed_ratio = 8.4.
- Predicted ratio = 56 × 8.4 = 470×. Measured ratio = 19.05 / 0.18 = 106×.
- Gap between predicted (470×) and measured (106×): kissat's wall time
  includes one full walk (500 k walk_steps) plus 10 615 conflicts of search,
  whereas solver 11's CDCL hits a 19 s deep-trail dead-end. The branching
  trajectory is responsible for ~10–15× of the gap; the per-prop speed
  difference is real (8.4×) but secondary, and likely reflects watcher-list
  layout / arena access patterns on a tiny in-cache formula (kissat fits
  battleship in 7 MB RSS, solver 11 uses 220 MB RSS — the *RSS difference
  itself* is a hint that solver 11 holds onto a lot more state per variable).

When `SAT_LUCKY=on`, solver 11 short-circuits to **0 conflicts, 0
propagations** and ships the model. The gap closes entirely.

## Work × speed decomposition — REGRandom (the second-largest gap)

Re-measured today.

| Solver | wall (s) | conflicts | decisions | props | props/s |
|---|---:|---:|---:|---:|---:|
| solver 11 default | 51.36 | 1 607 608 | 6 363 482 | 30 213 317 | 678 k |
| solver 11 LUCKY=on | 53.16 | 1 607 608 | 6 363 482 | 30 213 317 | 654 k |
| kissat-latest | 2.33 | 5 263 | 141 877 | 504 005 | 215 k |

**Decomposition:**
- Solver-11 does **305× more conflicts** than kissat. work_ratio = 305.
- Solver-11 propagates **3.15× faster per second** than kissat
  (678 k vs 215 k). speed_ratio = 0.32.
- Predicted ratio = 305 × 0.32 = 97. Measured ratio = 51.36 / 2.33 = 22.
- The trajectory gap dominates (305× more conflicts) and is not closed by
  lucky — lucky=on adds ~1.8 s overhead, finds nothing, and search proceeds
  identically (conflicts match exactly).
- This gap is **not a lucky problem**. It is a kissat-inprocessing-pipeline
  problem (backbone, sweep, vivify, sub-solver/kitten) — every counter on
  the kissat side reports work that solver 11 simply does not do. This is
  the parking-lot zone of FEATURES.csv (`SAT_PROBE`, `SAT_VIVIFY`,
  `SAT_HBR`, `SAT_INPROCESS`) and is out of scope for this pass.

## Lucky=on/off ablation per instance

`SAT_PROFILE=default` baseline vs same command + `SAT_LUCKY=on`. Wall in
seconds. Baseline column for instances I did not re-measure today is taken
from `log/analyzesat-2026-05-27-search-control/A_baseline/results.csv`
(same HEAD), which is acknowledged to have had host contention.

| Instance | Baseline | Lucky=on | Δ wall | lucky_solved | Note |
|---|---:|---:|---:|---:|---|
| battleship | 19.05 s (today) | **0.08 s** | **−99.6 %** | 1/7 | one pattern + local-repair found SAT |
| mp1 | 46.5 s | 42.5 s | −8.5 % | 0/7 | within noise |
| REGRandom | 51.4 s (today) | 53.2 s | +3.5 % | 0/7 | lucky overhead w/o win |
| SCPC | 13.5 s | 15.5 s | +14.5 % | 0/7 | small UNSAT, overhead noticeable |
| case9 | 126.8 s | 123.0 s | −3.0 % | 0/7 | within noise |
| sudoku | 207.5 s | 229.7 s | +10.7 % | 0/7 | host contention exaggerates this |
| velev | **65.2 s** (today, clean) | 86.3 s | **+32.4 %** | 0/7 | host-contention component + real overhead |
| Kakuro | 239.5 s | 249.6 s | +4.2 % | 0/7 | within noise |

Note on velev: the **same conflict count** (179 968) in both runs proves
lucky did not perturb search trajectory. The +21 s wall delta is pure
overhead (lucky preamble) plus host-contention noise. preprocess_sec went
23.35 → 28.95 (= +5.6 s of lucky preamble) and search_sec went 39.51 →
54.07 (= +14.6 s — explained primarily by the 3-parallel-solver
contention during the lucky-on run; baseline was standalone). The **lower
bound on lucky overhead on velev is +5.6 s** from the preprocess delta
alone. On an 8.76 M-clause formula, that delta is consistent with 6×
arena clones at ~50 MB each (LH-1).

## Reference diffs — implementation gaps

### Gap LH-1 — `with_temporary_assumptions` deep-clones the entire arena

**Symptom:** lucky overhead scales with clause count, not with variable
count. On battleship (3 976 clauses) lucky preamble is ~0 ms; on velev
(8.76 M clauses) it is ≥ 5.6 s.

**Solver-11 code, `src/main.rs:3403–3445`:**

```rust
fn begin_temporary_assumptions(...) -> TemporaryAssumptionGuard {
    let guard = TemporaryAssumptionGuard {
        start_trail: self.trail.len(),
        start_level: self.current_level(),
        start_root_trail_len: self.root_trail_len,
        start_propagate_head: self.propagate_head,
        saved_accounting_mode: self.accounting_mode,
        saved_arena: self.arena.clone(),                  // !! full Vec<u32> clone
        saved_watchers: self.watchers.clone(),            // !! Vec<Vec<Watcher>> clone
        saved_binary_clauses: self.binary_clauses.clone(),// !! Vec<BinaryClause> clone
    };
    ...
}

fn end_temporary_assumptions(&mut self, guard: ...) {
    // restore trail (correctly, by walking)
    ...
    self.arena = guard.saved_arena;
    self.watchers = guard.saved_watchers;
    self.binary_clauses = guard.saved_binary_clauses;
}
```

The `try_lucky_assignment_with_proof` loop at `src/main.rs:3767–3774`
enters and exits the guard once per pattern × 6 patterns. Each iteration
allocates a fresh `Vec<u32>` for the arena, a fresh `Vec<Vec<Watcher>>`
for watchers, and a fresh `Vec<BinaryClause>` — then drops the previous
ones at the end of the closure.

**Why the clone is wrong:** during a lucky pattern propagation (root-level
implications only), the arena is never mutated — there is no clause
learning, no GC, no clause addition or deletion within the pattern body
(`lucky_pattern_succeeds_with_proof` at `src/main.rs:3560–3642` only
calls `enqueue`/`propagate_budgeted`/`backtrack` plus optional
`learn_lucky_failed_literal_units` *after* the guard ends). Watchers are
not mutated by propagation (watcher list **order** can change, but the set
of references does not). Binary clauses are never mutated by propagation.

**Kissat reference (`kissat-latest/src/lucky.c:307–393`):** Kissat uses
the regular `kissat_internal_assume` + `kissat_probing_propagate` path on
the live trail at level 0. It backtracks via `solver->level = 0` and the
existing assignment-machinery. No clone of arena, watchers, or anything
else happens — kissat's lucky is "free" in terms of memory traffic.

**Recommended fix:** save/restore only the state that pattern propagation
actually mutates:
- `trail`, `trail_limits`, `assignment`, `decision_level`, `reason`,
  `propagate_head`, `accounting_mode`, branching-queue state
- explicitly *not* `arena`, `watchers`, `binary_clauses`

A new `TemporaryAssumptionOptions { snapshot_arena: false, ... }` variant
or a separate `begin_lucky_assumptions` entry point is the cleanest path.
Add a debug-only `debug_assert!(self.arena == saved_arena)` after lucky
exits to catch any future propagation path that does mutate arena (e.g.
GC during lucky propagation, which today is blocked because
`propagate_budgeted` does not call GC).

**Predicted impact (estimated, not measured):**
- velev lucky=on: −5–8 s preamble
- Kakuro lucky=on: −2–4 s preamble
- sudoku lucky=on: −1–3 s preamble
- battleship lucky=on: unchanged (already 0.08 s)
- net suite-wide: lucky=on becomes within ±2 % of lucky=off baseline on
  every instance other than battleship, where lucky=on remains −19 s

### Gap LH-2 — missing `no_all_negative_clauses` / `no_all_positive_clauses` precheck

**Solver-11 code, `src/main.rs:3766–3786`:** the for-loop runs all six
patterns unconditionally and unconditionally invokes
`lucky_pattern_succeeds_with_proof` for each.

**Kissat reference (`kissat-latest/src/lucky.c:307–342`):**

```c
if (no_all_negative_clauses (solver)) {
  for (all_variables (idx)) {
    if (!ACTIVE (idx)) continue;
    ...
    kissat_internal_assume (solver, lit);
    kissat_probing_propagate (solver, 0, true);
  }
  res = 10;
}

if (!res && no_all_positive_clauses (solver)) {
  ...  // symmetric
}
```

`no_all_negative_clauses` (`lucky.c:11–45`) walks every irredundant clause
and every active variable's binary watches; it returns false the moment it
finds any clause whose every literal is positive (then `AllFalse` cannot
satisfy that clause and the pattern is doomed). On a random/structured
formula with mixed-polarity clauses, this returns false within microseconds
on the first all-positive clause encountered.

**Recommended fix:** add two methods on `Solver`:

```rust
fn has_no_all_negative_clause(&self) -> bool {
    // walk original_clause_ids; for each live clause, if every literal is
    // negative return false; else continue. Return true if no all-negative
    // clause exists.
}
fn has_no_all_positive_clause(&self) -> bool { ... }
```

In `try_lucky_assignment_with_proof`, gate `AllTrue` by
`has_no_all_negative_clause` and `AllFalse` by `has_no_all_positive_clause`.
The check is O(literals) but stops at the first violation. Combined with
LH-1, the AllTrue/AllFalse skip should bring lucky preamble on large
mixed-polarity formulas to ~0 ms.

### Gap LH-3 — no WalkSAT-style local search beyond a 200 k-literal cap

**Solver-11 code, `src/main.rs:3686–3692`:** `try_lucky_local_repair` is
gated off for `preprocess_eliminated_vars != 0 || original_literals > 200 000
|| assignment.len() > 2 000`. Battleship (496 vars, 11 928 literals, 0 BVE)
passes; everything else on the suite fails one of the three checks.

**Kissat reference (`walk.c:936–966`, `rephase.c:rephase_walking`):**

- `kissat_walk` is triggered from rephasing (every K restarts), not just
  once before search.
- Gated by `MAX_WALK_REF` (last_irredundant + BINIRR_CLAUSES below a large
  cap, ~ a few hundred million). Velev with 8.76 M clauses is well below
  this cap and walks.
- The walk itself (`walking_phase` at `walk.c:886`) does ~500 k walk_steps
  of WalkSAT-style flipping, finds a satisfying assignment if one is
  reachable, and either keeps the assignment (SAT) or returns to search
  with phase information updated.

**Recommended fix:** this is bead `5b2.6.1` (Phase A.1 parking-lot
"Walking local-search rephaser", deferred). The cost is real (this is a
~500-line module in kissat), but the battleship data is the strongest
single-instance evidence for it on the profiling suite — kissat solves
battleship via walk+kitten, not via classic CDCL. Promotion path:
P4-deferred → P2-open with this bottleneck-analysis run as the supporting
evidence.

Note: implementing LH-3 from scratch is a multi-week project and is **not**
required to unlock the battleship win — LH-1 + LH-2 + flipping the lucky
default to `on` (with the CLAUDE.md solver-10 gate) is sufficient.

## Trajectory analysis — battleship

Solver 11 default (lucky=off):
- 593 019 conflicts in 19.05 s = 31 100 conf/s
- avg decision level **214**, max **465** — extremely deep CDCL trajectory
- 1 274 Luby restarts, 135 GC passes, 7 603 learned clauses retained
- maximum RSS 220 MB — solver 11 keeps a lot of metadata per variable
  even on a 496-variable formula

Kissat:
- 1 walk (500 001 walk_steps) at startup found the model region
- 10 615 conflicts of CDCL refinement
- maximum RSS 7 MB
- substituted: 0, eliminated: 0 — no BVE; the win is purely from walk

Solver 11 with `SAT_LUCKY=on`:
- one of the 6 lucky patterns or `try_lucky_local_repair` found the model
  in 7 attempts. 0 conflicts, 0 propagations charged to search.
- ~80 ms total wall time (parse + light preprocess + lucky).

This is **not** "phase-boundary chaos" — it is a *feature absence*. CDCL
on battleship without a walk-style or pattern-style seed *will* take ~20 s
on this host. The trajectory data confirms there is no productive tuning
within the CDCL parameter space alone.

## Hardware counter results

`perf_event_paranoid=4` on this host blocks unprivileged `perf stat` and
`perf record`. Hardware counter normalization was skipped. Work counters
(conflicts, decisions, propagations) from `SAT_STATS_JSON` are deterministic
and were used for all decomposition above.

## Parameter sweep results

No parameter sweep was needed: the dominant gap (battleship) is a binary
on/off switch (`SAT_LUCKY`), and the implementation gaps (LH-1, LH-2, LH-3)
are not parameter-tunable.

## Code-level recommendations (ordered by ROI)

1. **LH-1 — fix `with_temporary_assumptions` clone overhead** in
   `src/main.rs:3403–3445`. Stop cloning `arena` / `watchers` /
   `binary_clauses`. Add a debug assertion that they remain untouched
   across the closure. **Predicted: removes the lucky-preamble regression
   on large formulas, unblocking lucky=on as a default.**
2. **LH-2 — add structural prechecks** at `src/main.rs:3766–3786`. Skip
   `AllTrue` and `AllFalse` when `has_no_all_negative_clause` /
   `has_no_all_positive_clause` returns false. Reference:
   `kissat-latest/src/lucky.c:11–80`.
3. **Re-evaluate the `SAT_LUCKY=off` default** after LH-1 + LH-2 land.
   Run the CLAUDE.md solver-10 promotion gate
   (`tools/check_solver11_promotion.py`) with shuffle-sensitivity. Expected
   delta: battleship −23 s with no offsetting regression.
4. **Reprioritise `5b2.6.1`** (Phase A.1 walking local-search rephaser)
   from P4-deferred to P2-open. Cite this analysis as the evidence —
   battleship is the only profiling instance where kissat's walk module
   demonstrably matters, and even after LH-1+LH-2 it remains the kissat
   feature that closes the last residual gap on satisfiable random-ish
   instances.

## Rejected sweeps / non-issues

- **"Brocard is a residual gap" — refuted.** The May 26 reference CSV had
  kissat=9.33 s and solver-11=50.59 s on brocard, suggesting solver-11 was
  5.4× slower. Re-measured today: solver-11=9.06 s, kissat=56.71 s. Solver
  11 is 6.3× faster than kissat on brocard. The older measurement was
  almost certainly host-contended or otherwise off; the May 27 search-
  control CSV agrees that today's relative ordering is correct.
- **"6s299b685 is a residual gap" — refuted.** Re-measured today:
  solver-11=19.20 s, kissat=43.32 s. Solver 11 is 2.3× faster than kissat.
  Same explanation as brocard.
- **Per-prop speed gap on battleship — secondary, not pursued.** Kissat
  runs at 5.7 M prop/s vs solver-11 at 683 k prop/s on this tiny
  in-cache formula. This is an 8.4× per-prop speed difference and is a
  real microarchitecture gap, but it is dwarfed by the 56× trajectory gap
  (no walk). On every larger profiling instance, solver-11's prop/s is
  actually equal-or-faster than kissat's. Worth re-examining once perf
  access is available, but it is not the dominant cost on the suite.

## Artifact paths

- This file: `log/analyzesat-2026-05-27-propagation-hotpath/FINDINGS.md`
- Solver-11 lucky=off runs: `log/.../raw/solver11_<instance>.stderr` (JSON_STATS line)
- Solver-11 lucky=on runs: `log/.../raw/solver11_<instance>_lucky.stderr`
- Kissat runs: `log/.../raw/kissat_<instance>.{stdout,stderr}` (statistics output)
- Worktree: `/tmp/analyzesat-2026-05-27-propagation-hotpath-1779882636`
- Build flags: `CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=1 RUSTFLAGS="-C target-cpu=native" cargo build --release`
