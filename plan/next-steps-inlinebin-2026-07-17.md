# Next steps after the watch-inline-binary promotion (2026-07-17)

Context for a fresh session. State as of this writing:

- Medium baseline: **68/100** (68==68 in this session's winning gate; rbsat and
  sted2 both landed IN for both arms). Kissat 4.0.4 reference: 74/100
  (`log/kissat-medium-20260705-203444`). Gap ≈ 6.
- **PROMOTED: SAT_WATCH_INLINE_BIN default on** — watch-inline binary
  propagation (kissat proplit.h parity; the bead ck8 endgame applied to the
  DEFAULT path, where binaries are arena 2-clauses in the watch lists, NOT the
  rejected SAT_BINARY_FAST edge machinery). Length-2 clauses get tagged
  watchers (bit 31 = binary, bit 30 = cached learnt flag) whose blocker is
  exactly the other literal; propagation, conflict detection, and reason
  assignment resolve with ZERO arena dereference. Off-switch
  `SAT_WATCH_INLINE_BIN=off` = byte-for-byte shipped baseline (proved at
  100-cell scale: the base arm reproduced the 68-lineage conflicts
  65,324,524 exactly).
- Gate `log/abtest-cand-vs-base-2026-07-17-14-14-39` (launch
  `log/abtest-inlinebin2-launch.log`): **PASS, WIN** — solved 68==68,
  both-solved conflicts 64,570,274 vs 64,655,713 (−85,439), zero
  contradictions, zero correctness failures. 66/67 both-solved pairs
  byte-identical conflicts; the ONLY divergent both-solved cell is C_395
  (−85k, equal wall). Solved-set swap inside the decision-armed Timetable
  class: **TT492 SAT 1638s verify=ok — FIRST EVER solve** (kissat needs
  1052s; one of the 10 documented gap cells) for TT406 lost to its re-roll
  (base 236s). Both-solved wall 21,731 vs 22,023 (−1.3%).

## The mechanism

Legacy binary visit with blocker not TRUE: random arena load (header +
deleted check + both lits) plus a dirty-line l0/l1 normalization swap, then
enqueue/conflict. Inline visit: value(blocker) decides everything —
TRUE skip / FALSE conflict / UNASSIGNED enqueue(blocker, Clause(idx)) — no
arena touch; analysis loads the clause only when it actually resolves it.

Load-bearing design points (all learned the hard way this session):

1. **Deletion untags IN PLACE** (single choke point `clause_set_deleted` →
   `detach_inline_binary_watchers`): the entry becomes an ordinary stale
   watcher the legacy loop drops at its next visit. Eager swap_remove instead
   reorders lists AND skips the stale-visit ticks legacy pays → mode-switch
   points shift → full-suite trajectory reroll (measured: sudoku diverged from
   conflict #1 via root-simplification deletions).
2. **Positional reason invariant**: legacy propagation normalizes the
   propagated literal to lits[0]; the inline path does not. Every consumer of
   that invariant had to become var-based: conflict-analysis reason marking
   (skip-by-var), `lit_redundant` minimization descent (start pos 0, var
   check), and `clause_locked` (check BOTH watched positions — this one was a
   real soundness catch: a locked binary reported unlocked → deleted while a
   live reason → GC "live reason removed" panic on ibm; release builds would
   have propagated from a deleted clause).
3. **Conflict bump-order parity**: at a binary conflict the legacy arena
   holds [blocker, false_lit]; an `inline_binary_conflict_hint` (take-once)
   hands analysis that exact marking order without the arena write.
4. **Root-arming scope** (the v1→v2 fix): tags activate via a one-time O(W)
   pass ONLY after `maybe_arm_congruence_productive_search` declines to arm.
   Root-armed BMC/miter cells (oski, vex, ibm, bp4, DLTM) run mid-search
   formula-editing rounds that read binary literal order from the arena;
   with tags active from parse their trajectories reroll — the v1 unscoped
   gate `log/abtest-cand-vs-base-2026-07-17-11-15-41` LOST 66 vs 68 exactly
   there (oski40 rolled to TIMEOUT; sted2 lost to wall noise). Scoped, those
   cells are byte-identical to base; mid-search-armed cells (decision-armed
   Timetables, yield-armed density) still reroll — that is where the
   TT492/TT406 swap and the C_395 conflicts win come from.

## Measured this session (do not re-run blind)

1. Identity screens (SAT_LIMIT_CONFLICTS, idle): sudoku 200k, lockchart 100k,
   sted2 2M, ibm 100k (scoped), oski40 300k (scoped) all byte-identical
   knob-on vs knob-off (conflicts/decisions/props/ticks). Wall on identical
   trajectories: lockchart −1.8%..−4.5%, sudoku/sted2 neutral (cache-resident
   formulas gain nothing — the win is the removed cache miss, not ALU).
2. sted2's v1 in-gate loss (TIMEOUT vs base 1647s) was PURE WALL NOISE:
   trajectory byte-identical, idle wall 370.2 vs 371.6s. The thinnest-cell
   lottery, as documented in the decision-arm note. It solved 1674s in-gate
   for BOTH v2 arms.
3. The unscoped v1 gate is a complete negative result for "tags from parse":
   66 vs 68 (oski40 armed-roll TIMEOUT + sted2 noise), even though its
   conflicts tier was FAVORABLE (−583k with TT406 −115k, C_395 −545k, vex
   −51k) and wall −2.7%. Armed-cell rerolls are casino chips; the root-armed
   scope cashes out the deterministic part.
4. In-gate wall on identical-trajectory cells (v2): both-solved wall −1.3%
   with conflicts pinned — smaller than the v1 −2.7% because root-armed cells
   (the biggest wall cells) now run legacy.

## Ranked next steps

### 1. lockchart-group1 (unchanged target, now slightly closer)
Solve point ~270k conflicts (proven, model validated); we reach ~265k idle in
1750s. The inline diet banks a few percent on exactly this cell class
(11k props/conflict, big arena). Remaining big lever: CSR/merged watcher
layout (bead ck8 long-clause part) and walk-effort economics (444.6M walk
steps dominate the solve wall — see hotloop-defcores note).

### 2. TT406 recovery / Timetable-class roll stability
TT406 lost to a trajectory reroll (its 075b7e8 flip was collapse+walk luck at
a specific arena order; kissat itself needs rephase to solve it). If a future
change rerolls the decision-armed class again, watch TT406/TT492/C_395
together: the class holds 2-3 solvable cells and the rolls trade them. A
mechanism-level stabilizer (warmup? walk effort on armed cells?) is the
honest fix — see the decision-arm note's TT492-depth section.

### 3. oski20 margin (unchanged)
Root-armed → byte-identical → still TIMEOUT in-gate both arms. The inline
diet does NOT apply there (scoped off). The CSR watcher endgame and further
in-gate margin remain the levers.

### 4. Density class (unchanged)
Yield-armed rerolls happen under the inline scope but both arms timeout —
no metric effect. The elimination-bound instrumentation play from the
hotloop-defcores note is still open.

## Housekeeping / traps (additions this session)

- Trajectory-identity for a watcher-layout change requires reproducing THREE
  legacy behaviors, not one: list-order evolution (untag-in-place, not
  remove), tick accounting on stale visits, and analysis bump order (hint).
  Wall-limit screens are useless for this; SAT_LIMIT_CONFLICTS + byte-compare
  conflicts/decisions/props/ticks.
- `clause_locked` positional assumption was load-bearing for soundness;
  any future change that stops normalizing the propagated literal to lits[0]
  must re-audit ALL `clause_lit(_, 0)` reads (grep is NOT enough — the
  clause_locked hit was at line ~7766, past the first grep page).
- perf on this host is blocked (perf_event_paranoid=4); use documented
  analyzesat findings + paired /usr/bin/time probes instead.
- The A/B preflight running_solver_processes FAIL from your own monitor
  shells: TaskStop monitors, re-run the gate check (standard, hit again).

## Where the evidence lives

- Winning gate: `log/abtest-cand-vs-base-2026-07-17-14-14-39` + launch log
  `log/abtest-inlinebin2-launch.log`; formal check output in the commit.
- Losing v1 gate (unscoped, documented above):
  `log/abtest-cand-vs-base-2026-07-17-11-15-41` + `log/abtest-inlinebin-launch.log`.
- Identity/wall screens: scratchpad (dies on reboot) — all decision-relevant
  numbers are in this note and the commit message.
- Bead: `SAT-playground-2a7` (running gap-analysis log).
