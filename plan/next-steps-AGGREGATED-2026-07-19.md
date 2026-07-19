# AGGREGATED next-steps plan — 2026-07-19 (supersedes next-steps-AGGREGATED-2026-07-16.md)

One-file plan for the next session. Everything below already accounts for the
2026-07-18/19 sessions (decomposition + flywheel groundwork + fastidx
promotion). Where this file contradicts an older `plan/next-steps-*.md`, THIS
file wins; the older notes are provenance and negative-result ledgers only.

## Current state (verified 2026-07-19)

- Medium baseline: **69/100 @ 70493e3** (gate
  `log/abtest-cand-vs-base-2026-07-18-22-33-59`: 69 vs 68, both-solved
  conflicts EXACT tie, PAR-2 139,085.0). Kissat 4.0.4 reference: **74/100**
  (`log/kissat-medium-20260705-203444`). Net gap 5 (kissat-only cells: 11;
  we-only cells: 6).
- Solved-set notes: **rbsat-v1375 is now IN** (wall-margin flip from the
  fastidx diet; documented lottery cell — its trajectory is identical to the
  68-lineage, base missed the 1800s wire by 0.05s). TT492 IN, TT406 OUT (the
  standing Timetable trade). sted2/rbsat remain the thinnest wall-lottery
  cells; treat any future flip of either as wall noise unless conflicts change.
- Promotion ledger (newest first): 70493e3 SAT_CONGRUENCE_FASTIDX (69/100,
  conflicts exact tie, PAR-2 −1,864) | 4bf2de4 flywheel groundwork default-OFF
  + decomposition findings | 6199fb2 SAT_WATCH_POOL | 2ed8e27
  SAT_WATCH_INLINE_BIN (TT492 first solve) | d23e454 SAT_HOTLOOP_PTR | 6633bc7
  binary-edge tag | 075b7e8 SAT_DECISION_ARM=24 | 038f9c1 binary DRAT |
  2f92794 vivify-yield arming | 3683ab5 vivify ALE (+vex +oski40) | e5bd1f9
  armed collapse bundle | 906e7cc giant-arena parse | 15911aa preflight |
  a402efd factor | c579bfe congruence inprocess | 689f080 chrono.

## The 11 kissat-only cells, with honest flippability verdicts (2026-07-18/19 evidence)

| cell | kissat | class | verdict |
|---|---|---|---|
| TT406 | 41s | decision-armed walk lottery | **cheapest +1 in principle** — we solved it at 236s in the 075b7e8 lineage; lost to the TT492 trade. BLOCKED on a TT-class stabilizer; every decision-armed reroll is −EV while TT492 is in. |
| Bubble | 354s | density | biggest kissat margin; single mechanisms ALL measured dead (bound knobs, rephase, restarts, deduce, backbone, clslim, def-nocap). Sweep is NOT broken (finds 85 eq vs kissat 100; the historical "zero" was congruence merges — kissat ~0 too). Remaining play: multi-mechanism ensemble/economics. |
| fixedbandwidth-eq-37 | 576s | density (pure throughput per kissat profile) | same class as Bubble. |
| oski20 | 617s | root-armed BMC wire | solves idle at **1271s** (2026-07-18, improved from 1430-1581s); **98.6% of wall is pure search** — no overhead diet left; needs in-gate contention margin (memory-bandwidth diet / CSR) or ~200s more suite speedup. |
| bp4_TCO_CSO_IXA_LP_ZR | 1287s | structured SAT (dense 2.1 dec/conf) | never analyzed deeply; sibling bp4s solve. |
| pj2008 | 1165s | giant, <200k conflicts in 1800s | props/s parity measured globally, so its wall is formula size at parse (8.6M vars) — measure root-collapse vs memory-locality before writing code. |
| goldcrest | 1234s | BMC, <1M conflicts in budget | flywheel inert (never reaches 1M conflicts); needs earlier collapse or rate work. |
| booth_wallace / booth_dadda | 1371/1389s | density | same as Bubble class. |
| lockchart-group1 | 1687s | walk economics | kissat itself needs 94% of the budget — NOT a realistic flip this generation. Walk-effort=25 solves idle @2613s (lottery, −EV). |
| g2 | 1758.9s | unarmed BMC | kissat needs 97.7% of the budget — NOT a realistic flip. Flywheel gives +27% window rate; insufficient by ~2x. |

## Load-bearing discoveries from 2026-07-18/19 (these re-rank everything)

1. **Props/s PARITY with kissat** at equal conflict counts — g2 3.7M vs 3.6M,
   lockchart-g1 4.4M vs 4.4M props/s. The old #1 ranking ("propagation
   throughput on conflict-rate-bound cells") is DEAD. The kissat rate gap on
   g2-class is **clause-DB size** (kissat's continuous inprocessing collapses
   g2 888k→37k irredundant clauses; we freeze at ~500k → 2x props/conflict).
   CSR/merged watchers are demoted to a PAR-2 cache-diet play, NOT a rate play.
2. **Wall-decompose before optimizing** (`SAT_TRACE_TIMING=1` checkpoints, new):
   root congruence closure was a timer-invisible 31% of ibm's wall
   (preprocess_sec starts AFTER it). The fastidx diet promoted from this.
   Remaining measured non-search chunks: **vex root eliminate 28.3s** (the
   next biggest single chunk), oski40 root congruence ~8s (post-diet),
   closure merge-application ~1.4s/round.
3. **Trajectory-identical wall diets are the most reliable gate-winners**
   (fastidx, watchpool, hotloop, bintag — 4 for 4) and can flip wall-lottery
   cells (rbsat this gate). The identity recipe: byte-compare conflicts AND
   per-mechanism counters (merge counts per round) on 2-4 armed cells + a full
   canary, under SAT_LIMIT_CONFLICTS where wall isn't the question.
4. **Hash-order insensitivity is provable from cross-process reproducibility**:
   outcomes were byte-stable across std's randomly-seeded SipHash for months →
   fixed-seed FxHash is inside the tested envelope. `src/fxhash.rs` is inline
   (no crate dep) and reusable for other hash-hot paths.
5. **Incremental gate-extraction caching is BLOCKED on lit-order sensitivity**:
   extraction output order depends on incidental clause-literal order, which
   propagation swaps constantly (incl. PTR_FAST unsafe writes). Exact-identity
   caching would need hot-loop bookkeeping. A canonicalized (sorted-lit)
   extraction unblocks it but is a one-time full-suite reroll — only do it WITH
   the cache in the same gate so the reroll buys the wall win.
6. **Flywheel groundwork exists** (`SAT_ELIM_UNARMED_FLYWHEEL`, default OFF,
   4bf2de4): unarmed cells past 1M conflicts get 100k-cadence eliminate rounds
   with complete-round bound escalation + gate detectors. g2: −12% wall at 2M
   conflicts, +27% window rate, elimination depth 90.7% (beyond kissat's 86% —
   depth is NOT the residual; clause-MASS cleanup is). Measured NOT gate-worthy
   alone: reroll surface = 59-129706 (+3.16M conflicts on this seed) +
   lockchart-g2 (+22k), and no flip anywhere (see verdict table). Escalation
   without the gate detectors is pure tax (+6.3k vars only).
7. **The gate-EV method that worked** (use it before every future gate):
   enumerate the reroll surface from the last gate TSV (which both-solved
   cells can the change touch?), screen those + all plausible flips at full
   budget standalone, and predict the lexicographic outcome. Two sessions, two
   correct predictions (flywheel = would-lose → not run; fastidx = tie/tie/win
   → run, won +1).

## RANKED PLAN for next session

### 1. Continue the wall-diet arc (highest EV-per-hour, repeatable)
   a. **vex root eliminate (28.3s)**: decompose with SAT_TRACE_TIMING +
      targeted counters (occurrence build vs BSR vs BVE resolution scan), then
      apply the fastidx playbook (FxHash where SipHash-hot, flat pools, reserve,
      legacy off-switch knob). vex solves at 1651s in-gate — 28s here is real
      margin on a thin cell, plus root-eliminate cost recurs on every cell.
   b. **Closure trims left on the table**: merge-application ~1.4s/round
      (second try_els + gates dealloc churn), `(u8, Vec<i32>)` key clones in
      find_merges_closure (prehash-bucket layout, ~1.3M clones/round).
      Identity-safe, bounded (~20-30s suite-wide).
   c. Bundle a+b into ONE gate (the bintag pattern: prove conflicts-identical
      on 4-5 cells, gate once). Expected: tie/tie/PAR-2 win + more lottery
      margin (rbsat/sted2/TT492/vex all bank seconds).

### 2. Canonicalization + incremental extraction (the big congruence win, one gate)
   Sort clause literals (or extract in sorted order) to make gate extraction
   lit-order-insensitive, THEN add the per-clause touched-var gate cache
   (invalidation rule proven sound: a clause's gates change only if
   vars(clause) ∩ touched ≠ ∅). Rounds 2-7 extraction → ~free; mid-search
   armed closures too. This is a full-suite REROLL (canonicalization changes
   trajectories) — gate it as reroll+wall in one candidate, watch the TT/sted2/
   rbsat lottery, and only after step 1 has banked its margin.

### 3. Density-class ensemble (Bubble/fixedbandwidth/booth — 4 cells, kissat's biggest margins)
   Single mechanisms are exhausted (see ledger below). The remaining honest
   play is kissat's probe.c ORDER with per-pass effort budgets on armed cells:
   congruence → substitute → backbone → vivify → sweep → substitute →
   transitive → backbone → factor per round, re-run while active vars drop.
   Note the 2026-07-18 correction: our sweep DOES find equivalences (g2 66k,
   Bubble 85) — the gap is what happens BETWEEN passes (clause-mass cleanup,
   substitution feeding elimination). Instrument clause-count-per-round on
   Bubble under the armed bundle first; the kissat target curve is 888k→45k
   style collapse. If the ensemble shrinks Bubble's DB where bound knobs
   didn't, gate it (Bubble flip alone = +1 and trumps conflicts-tier noise).

### 4. Flywheel ensemble variant (g2-class, only AFTER #3 teaches clause-mass cleanup)
   The flywheel (default-off) + fast-cadence vivify/subsume/sweep for the same
   guarded class. Success criterion BEFORE gating: 59-129706 must not regress
   beyond the class's wins (its +3.16M reroll is the measured blocker).
   g2 itself stays unflippable; the target is conflicts-tier/PAR-2 on
   >1M-conflict UNSAT cells plus goldcrest-class knock-ons.

### 5. TT406 stabilizer (the cheapest +1, still blocked)
   Do NOT reroll the decision-armed class blind (TT492 is in; every reroll
   trades TT cells). The open mechanism question: kissat rephases from ~1k
   conflicts and solves TT406 in 41s; our decision-arm fires at 200k+.
   A stabilizer = something that makes the collapse+walk deterministic across
   arena orders (walk-count sensitivity per the walk-effort=25 lockchart
   lottery). Only attack with a concrete mechanism hypothesis + paired screens
   on TT406/TT492/C_395 together.

### 6. pj2008 / bp4_TCO measurement (unclassified cells, measure-first)
   pj2008: is it root-collapse-starved (giant parse leaves 8.6M vars) or
   memory-bound? One SAT_TRACE_TIMING run + kissat -s comparison decides.
   bp4_TCO: never analyzed — same cheap treatment. Both are pure measurement
   sessions before any code.

## Measured-dead ledger (do NOT re-run blind — consolidated from all sessions)

- Propagation-throughput ports for rate: props/s parity measured (2026-07-18).
- Lit-indexed values array: wall LOSER (lockchart +5.7%).
- Bound escalation on armed cells: conflicts LOSER (QG7 +1.4%, Pancake +96%);
  decision-armed variant trades TT406↔TT492 net 0.
- SAT_INPROCESS_ROUNDS=2: oski20 −19% but oski40 +42% — no honest scoping.
- Unarmed eliminate at fast cadence WITHOUT escalation+gates: pure tax.
- Congruence-learned extraction: byte-identical on vex (merge freeze is not
  input starvation).
- elim-def (kitten definitions): densification kills oski40; defcores
  refinement DEAD (cores already minimal). Salvage angles in the 07-16 note.
- Backbone, transitive reduction (for vex/density), rephase/walk global or
  yield-armed, restart floors/margins, vivify-deduce, vivify-sort, trail
  reuse, ELIM_PRODUCTIVE_MIN_PCT, walk warmup: all measured dead in their
  noted scopes (see 07-14/15/16/17 notes for numbers).
- lockchart-g1 and g2 as flip targets: kissat needs 94-98% of budget itself.

## Standing traps (verbatim-relevant, consolidated)

- check_promotion_gate `running_solver_processes` FAIL from monitor/watcher
  shells (yours OR a previous session's) — kill them, re-run; hit in 3
  sessions running.
- `timeout N env sat-solver …` kills before the stats JSON is emitted — use
  SAT_LIMIT_CONFLICTS for end-state stats.
- Ablation TSV TIMEOUT rows carry zero conflicts/decisions — class analysis of
  unsolved cells needs standalone screens.
- kissat progress lines: conflicts is $10; $(NF-3) is IRREDUNDANT CLAUSES.
- kissat `-s -q` mutually exclusive; drat-trim prints \r (don't anchor greps).
- Trajectory-identity for watcher/arena-order changes needs list-order
  evolution + tick parity + bump-order parity (inlinebin recipe); any change
  to resting clause-lit order rerolls armed cells (formula-editing passes read
  arena order).
- 2-arm gates only (3-arm changes the contention profile; lottery cells won't
  reproduce). Wall-lottery cells: sted2, rbsat, TT492 (1638s), vex (1651s).
- perf is blocked on this host (perf_event_paranoid=4) — use SAT_TRACE_TIMING
  checkpoints (now in-tree) + paired /usr/bin/time + ablation decomposition.
- feature_ablation keeps only results.tsv per arm; per-cell JSONs are cleaned
  with the tmp dir — extract per-cell stats DURING the run or re-screen.
- oski-class standalone walls are load/thermal-sensitive; pair everything.
- `pkill -f pattern` self-matches; kill by PID.

## Instrumentation now in-tree (use it)

- `SAT_TRACE_TIMING=1`: wall checkpoints (parse / frontend / Solver::new /
  root_propagate / pair_abs_gauss_els / congruence_root / search_start /
  model_write / model_check / stats / output).
- `SAT_DEBUG_CONGRUENCE=1`: dry-run + per-round + per-step closure timings
  (extract_binaries / els / extract_gates / closure) and merge counts.
- `SAT_TRACE_PREPROCESS_DETAILS=1`: elim_round counters (cumulative — diff
  consecutive lines), vivify_yield_probe lines (dec/conf + best_permille —
  useful for classifying cells), unarmed_flywheel lines.
- Knob off-switches for A/B arms: SAT_CONGRUENCE_FASTIDX=off (pre-diet
  congruence), SAT_ELIM_UNARMED_FLYWHEEL=on (flywheel), plus the ledgered
  historical off-switches in each promotion note.

## Where the evidence lives

- Newest sessions (full detail): `plan/next-steps-fastidx-promotion-2026-07-19.md`,
  `plan/next-steps-flywheel-decomposition-2026-07-18.md`,
  `plan/next-steps-elimbounds-negatives-2026-07-18.md`.
- Prior arc: `plan/next-steps-walkwarmup-watchpool-2026-07-17.md`,
  `next-steps-inlinebin-2026-07-17.md`, `next-steps-hotloop-defcores-2026-07-16.md`,
  and `plan/next-steps-AGGREGATED-2026-07-16.md` (the older ledger, still valid
  as provenance where not superseded above).
- Current gate baseline for the NEXT A/B:
  `log/abtest-cand-vs-base-2026-07-18-22-33-59` (cand arm = the 69/100 lineage).
- Bead: `SAT-playground-2a7`.
