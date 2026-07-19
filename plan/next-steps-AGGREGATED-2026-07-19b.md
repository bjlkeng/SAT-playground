# AGGREGATED next-steps plan — 2026-07-19b (supersedes next-steps-AGGREGATED-2026-07-19.md)

One-file plan for the next session. Folds in the 2026-07-19 elim-scratch
session on top of everything the morning aggregate already covered. Where this
file contradicts an older `plan/next-steps-*.md`, THIS file wins; older notes
are provenance and negative-result ledgers only.

## Current state (verified 2026-07-19, end of day)

- HEAD: **56a0bb5** (`SAT_ELIM_SCRATCH` BVE/insertion wall diet, default ON)
  + 6f3c27e (notes). Newest gate `log/abtest-cand-vs-base-2026-07-19-09-23-57`:
  **PASS, WIN 68 vs 67** (+rbsat-v1375 1765.2s vs base TIMEOUT), both-solved
  conflicts EXACT tie (67 cells, 0 mismatches), PAR-2 140,952.1 vs 142,926.7
  (−1,974.6). Zero contradictions/correctness failures; checker-timeout on
  sqrt-mitern170 + vex arm-symmetric (benign drat-trim verify window).
- **The 68-vs-69 wobble is oski20, not a regression**: this gate's cand
  solved-set = the previous 69-set minus oski15a01b20s (prev gate 1742.8s —
  the documented in-gate-contention-marginal cell; it needs ~60s more margin
  or a quieter gate). Same-gate A/B is the valid comparison; treat the
  medium baseline as the 69-lineage with rbsat/oski20 as coin-flip cells.
- **Wall-lottery margins banked by the diet** (in-gate, this gate vs 07-18
  gate): TT492 1527.2s (was 1638) −111s; vex 1611.9s (was 1651) −39s; sted2
  1568.6s; rbsat 1765.2s. Every future gate inherits these.
- Kissat 4.0.4 reference: **74/100** (`log/kissat-medium-20260705-203444`).
  Net gap ~5-6 depending on the oski20/rbsat coin flips.
- Promotion ledger (newest first): 56a0bb5 SAT_ELIM_SCRATCH (68v67 WIN,
  conflicts exact tie, PAR-2 −1,975) | 70493e3 SAT_CONGRUENCE_FASTIDX (69v68,
  PAR-2 −1,864) | 4bf2de4 flywheel groundwork default-OFF | 6199fb2
  SAT_WATCH_POOL | 2ed8e27 SAT_WATCH_INLINE_BIN | d23e454 SAT_HOTLOOP_PTR |
  6633bc7 binary-edge tag | 075b7e8 SAT_DECISION_ARM=24 | 038f9c1 binary DRAT
  | 2f92794 vivify-yield arming | 3683ab5 vivify ALE | e5bd1f9 armed collapse
  bundle | 906e7cc giant-arena parse | 15911aa preflight | a402efd factor |
  c579bfe congruence inprocess | 689f080 chrono.
- **Wall-diet arc is now 5-for-5** (bintag, hotloop, watchpool, fastidx,
  elim-scratch): trajectory-identical diets remain the most reliable gate
  winners and keep flipping wall-lottery cells.

## The kissat-only cells, with honest flippability verdicts

| cell | kissat | class | verdict |
|---|---|---|---|
| oski20 | 617s | root-armed BMC wire | now a COIN FLIP at gate contention (in @1742.8s on 07-18, out @1800+ on 07-19). 98.6% of wall is pure search — further wall diets help it most of any cell; ~60s more margin likely flips it stably. |
| TT406 | 41s | decision-armed walk lottery | cheapest +1 in principle (solved at 236s in 075b7e8 lineage; traded for TT492). BLOCKED on a TT-class stabilizer; decision-armed rerolls are −EV while TT492 is in (TT492 margin now +111s better, making rerolls even riskier). |
| Bubble | 354s | density | single mechanisms ALL measured dead. Remaining play: multi-mechanism ensemble/economics (kissat probe.c pass ORDER with per-pass budgets). |
| fixedbandwidth-eq-37 | 576s | density | same class as Bubble. |
| bp4_TCO_CSO_IXA_LP_ZR | 1287s | structured SAT (2.1 dec/conf) | never analyzed — cheap measurement session first. |
| pj2008 | 1165s | giant (8.6M vars), <200k conflicts | wall is formula size at parse; measure root-collapse vs memory-locality (SAT_TRACE_TIMING run + kissat -s compare) before code. |
| goldcrest | 1234s | BMC, <1M conflicts | flywheel inert; needs earlier collapse or rate work. |
| booth_wallace / booth_dadda | 1371/1389s | density | same as Bubble class. |
| lockchart-group1 | 1687s | walk economics | kissat needs 94% of budget — NOT realistic this generation. |
| g2 | 1758.9s | unarmed BMC | kissat needs 97.7% of budget — NOT realistic. |

## Load-bearing discoveries (cumulative; newest first)

1. **BVE apply-path decomposition (2026-07-19, SAT_TRACE_ELIM)**: vex root
   eliminate 24.7s = BVE 22.5s (833k pivot attempts) with apply 17.7s —
   resolvent INSERTION 12.0s spread ~evenly over normalize/proof/arena/attach/
   occurrence-index (no single structural chunk); BSR only 0.7s. **Kissat
   spends ~24.7s in eliminate on vex too** (200k-conflict `-s` run) →
   eliminate cost is at PARITY; overhead diets, not architecture ports, are
   the honest play there. (Kissat's dense-mode/no-watcher elimination would
   reroll trajectories — only consider as a deliberate reroll gate.)
2. **Props/s PARITY with kissat** at equal conflict counts (2026-07-18): the
   kissat rate gap on g2-class is clause-DB size (continuous inprocessing
   collapses g2 888k→37k irredundant; we freeze at ~500k → 2x props/conflict).
   Propagation-throughput ports for rate are DEAD; CSR demoted to cache-diet.
3. **Wall-decompose before optimizing** (SAT_TRACE_TIMING / SAT_TRACE_ELIM):
   found root congruence 31% of ibm (fastidx win) and BVE-apply dominance
   (elim-scratch win). Remaining measured non-search chunks: eliminate
   `other` ≈1.2s/round + per-round `vec![false; vars]` flag allocs in
   `eliminate()` (paid per mid-search armed round), congruence
   merge-application ~1.4s/round (second try_els + gates dealloc churn),
   dry-run closure plan recomputed by round 0 (reusable only if
   extract_binaries+els provably made no edits).
4. **Trajectory-identical wall diets are 5-for-5** and flip wall-lottery
   cells. The identity recipe: byte-compare stats AND per-mechanism counters
   (incl. mid-search armed rounds) on 3-4 armed cells + a canary under
   SAT_LIMIT_CONFLICTS; verbatim legacy off-switch arm for the simultaneous
   A/B (SAT_ELIM_SCRATCH=off, SAT_CONGRUENCE_FASTIDX=off precedents).
5. **Hash-order insensitivity is provable from cross-process reproducibility**
   (fastidx): fixed-seed FxHash (`src/fxhash.rs`, no crate dep) is inside the
   tested envelope; reusable for other hash-hot paths.
6. **Incremental gate-extraction caching is BLOCKED on lit-order sensitivity**;
   canonicalized (sorted-lit) extraction unblocks it but is a full-suite
   reroll — only do it WITH the cache in the same gate.
7. **Flywheel groundwork exists** (SAT_ELIM_UNARMED_FLYWHEEL, default OFF,
   4bf2de4): g2 −12% wall at 2M conflicts, +27% window rate, but no flip and
   a +3.16M-conflict reroll on 59-129706 — not gate-worthy alone.
8. **The gate-EV method (3 sessions, 3 correct predictions)**: enumerate the
   reroll surface from the last gate TSV, screen plausible flips standalone at
   full budget, predict the lexicographic outcome BEFORE running the gate.
   Trajectory-identical changes have empty reroll surface → predicted
   tie/tie/PAR-2-win + lottery upside; that's exactly what happened again.

## RANKED PLAN for next session

### 1. Continue the wall-diet arc (still highest EV-per-hour, 5-for-5)
   Measured next chunks (all identity-safe, bundle into ONE gate):
   a. `eliminate()` per-round allocations: `touched_flags`/`bsr_touched_flags`
      `vec![false; n]`, `heap_versions` vec, and the BinaryHeap rebuild — paid
      on EVERY mid-search armed round (vex/oski/ibm class). Flags can persist
      if provably all-false at round end (verify by counter first); heap is
      algorithmic (kissat keeps a persistent schedule — bigger change, reroll
      risk, separate).
   b. Congruence merge-application ~1.4s/round: second try_els + gates
      dealloc churn (fastidx note); plus dry-run plan reuse for round 0 IF
      extract_binaries+els edit-free is cheaply detectable (add edit counters
      first — measure, don't assume).
   c. eliminate `other` ≈1.2s/round attribution (trace shows it; find it).
   Expected: tie/tie/PAR-2 win + more lottery margin. **oski20 is the flip
   target**: it needs ~60s; TT492/vex margins also deepen.
2. **Canonicalization + incremental extraction** (the big congruence win, one
   deliberate-reroll gate) — unchanged from 07-19 aggregate: sort clause lits
   so gate extraction is lit-order-insensitive, THEN per-clause touched-var
   gate cache (invalidation rule proven sound). Full-suite reroll; only after
   #1 has banked its margin, and watch the TT/sted2/rbsat/oski20 lottery.
3. **Density-class ensemble** (Bubble/fixedbandwidth/booth — 4 cells, kissat's
   biggest margins): kissat probe.c ORDER with per-pass effort budgets
   (congruence → substitute → backbone → vivify → sweep → substitute →
   transitive → backbone → factor, re-run while active vars drop). First step
   stays: instrument clause-count-per-round on Bubble under the armed bundle;
   kissat target curve is 888k→45k-style collapse. A Bubble flip alone = +1.
4. **Flywheel ensemble variant** (g2-class) — only AFTER #3 teaches
   clause-mass cleanup; 59-129706 must not regress beyond the class's wins.
5. **TT406 stabilizer** — unchanged; do NOT reroll decision-armed class blind
   (TT492 is in with +111s margin now; rerolls trade TT cells). Attack only
   with a concrete mechanism hypothesis + paired screens on TT406/TT492/C_395.
6. **pj2008 / bp4_TCO measurement** (pure measurement sessions; no code).

## Measured-dead ledger (do NOT re-run blind)

- Propagation-throughput ports for rate: props/s parity measured (07-18).
- Lit-indexed values array: wall LOSER (lockchart +5.7%).
- Bound escalation on armed cells: conflicts LOSER; decision-armed variant
  trades TT406↔TT492 net 0.
- SAT_INPROCESS_ROUNDS=2: oski20 −19% but oski40 +42% — no honest scoping.
- Unarmed eliminate at fast cadence WITHOUT escalation+gates: pure tax.
- Congruence-learned extraction: byte-identical on vex.
- elim-def (kitten definitions): densification kills oski40; defcores DEAD.
- Backbone, transitive reduction (vex/density), rephase/walk global or
  yield-armed, restart floors/margins, vivify-deduce, vivify-sort, trail
  reuse, ELIM_PRODUCTIVE_MIN_PCT, walk warmup: all dead in noted scopes.
- lockchart-g1 and g2 as flip targets: kissat needs 94-98% of budget itself.

## Standing traps (consolidated)

- check_promotion_gate `running_solver_processes` FAIL from monitor/watcher
  shells — yours OR a previous session's (hit AGAIN 2026-07-19: five zombie
  watcher shells from three prior sessions killed by PID before the gate
  check). Kill by PID; `pkill -f pattern` self-matches.
- **SAT_TRACE_ELIM heisenberg**: with tracing ON, the finest-grain sub-timers
  inflate the measured hot path ~2x (Instant overhead). Use ratios at the
  finest level; absolutes only from the coarse tiers. When OFF the tokens are
  branch-only (Option<Instant> = None) — verified no default-path tax.
- **Don't diff logs of still-running screens** (hit 2026-07-19: premature
  reads showed false DIFFERS). Wait for process exit, then diff.
- `timeout N env sat-solver …` kills before stats JSON — use
  SAT_LIMIT_CONFLICTS for end-state stats.
- Ablation TSV TIMEOUT rows carry zero conflicts — class analysis of unsolved
  cells needs standalone screens.
- kissat progress lines: conflicts is $10; kissat `-s -q` mutually exclusive;
  drat-trim prints \r (don't anchor greps). kissat --conflicts=1000 exits
  BEFORE its first eliminate — use ≥100k-conflict runs for inprocess
  profiling comparisons.
- Trajectory-identity for watcher/arena-order changes needs list-order
  evolution + tick parity + bump-order parity (inlinebin recipe); any change
  to resting clause-lit order rerolls armed cells.
- 2-arm gates only (3-arm changes the contention profile). Wall-lottery cells
  now: oski20 (1742-1800s), rbsat (1765s), sted2 (1569s), TT492 (1527s), vex
  (1612s).
- feature_ablation keeps only results.tsv per arm — extract per-cell stats
  DURING the run or re-screen.
- oski-class standalone walls are load/thermal-sensitive; pair everything.
- Gate tail: drat-trim verify of vex/sqrt-miter proofs adds ~30-40 min after
  the last solver exits — the run is NOT hung.

## Instrumentation now in-tree (use it)

- `SAT_TRACE_TIMING=1`: wall checkpoints (parse / frontend / Solver::new /
  root_propagate / pair_abs_gauss_els / congruence_root / search_start /
  model steps).
- `SAT_TRACE_ELIM=1` (NEW): eliminate decomposition — occ_build/bsr/bve/gather
  totals + BVE sub-steps (setup/partition/gate/resolve/apply) + apply
  sub-steps (pushelim/proofsnap/remove/add/proofdel) + insertion sub-steps
  (norm/proof/arena/attach/index/enq). See heisenberg trap above.
- `SAT_DEBUG_CONGRUENCE=1`: dry-run + per-round + per-step closure timings and
  merge counts.
- `SAT_TRACE_PREPROCESS_DETAILS=1`: elim_round counters (cumulative — diff
  consecutive lines), vivify_yield_probe, unarmed_flywheel lines.
- Off-switch A/B knobs: `SAT_ELIM_SCRATCH=off` (pre-diet BVE/insertion paths
  verbatim, NEW), `SAT_CONGRUENCE_FASTIDX=off`, `SAT_ELIM_UNARMED_FLYWHEEL=on`,
  plus historical knobs in each promotion note.

## Where the evidence lives

- Newest session: `plan/next-steps-elimscratch-2026-07-19.md` (this session's
  full detail), gate `log/abtest-cand-vs-base-2026-07-19-09-23-57` + launch
  log `log/abtest-elimscratch-launch.log`.
- Prior arc: `plan/next-steps-fastidx-promotion-2026-07-19.md`,
  `next-steps-flywheel-decomposition-2026-07-18.md`,
  `next-steps-elimbounds-negatives-2026-07-18.md`,
  `next-steps-walkwarmup-watchpool-2026-07-17.md`,
  `next-steps-inlinebin-2026-07-17.md`, and the superseded
  `next-steps-AGGREGATED-2026-07-19.md` (still valid as provenance).
- Current gate baseline TSVs for the NEXT A/B:
  `log/abtest-cand-vs-base-2026-07-19-09-23-57` (cand arm = the promoted
  lineage, 68/100 in that contention profile with oski20 out).
- Bead: `SAT-playground-2a7`.
