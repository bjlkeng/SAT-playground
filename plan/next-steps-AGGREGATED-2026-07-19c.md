# AGGREGATED next-steps plan — 2026-07-19c (supersedes next-steps-AGGREGATED-2026-07-19b.md)

One-file plan for the next session. Folds the 2026-07-19 evening round-diet
session on top of everything the 19b aggregate covered. Where this file
contradicts an older `plan/next-steps-*.md`, THIS file wins; older notes are
provenance and negative-result ledgers only.

## Current state (verified 2026-07-19, end of evening session)

- HEAD: **3ff775e** = 74eeaf0 (`SAT_ROUND_DIET` per-round inprocessing
  overhead diet, default ON) + session notes. Newest gate
  `log/abtest-cand-vs-base-2026-07-19-16-01-57`: **PASS, WIN — solved 69 vs
  69 (tie), both-solved conflicts EXACT tie (70,963,533, all 100 cells),
  PAR-2 138,593.7 vs 138,999.4 (−405.6)**. Zero contradictions/correctness
  failures. `check_promotion_gate` formal PASS.
- **The medium baseline is 69/100 and this gate confirmed it cleanly**: the
  two documented coin-flip cells were IN for BOTH arms — oski20 (cand
  1693.8s / base 1712.5s) and rbsat (cand 1690.6s / base 1791.1s, base 9s
  from timeout). The 68-vs-69 wobble of the previous gate was contention,
  as diagnosed.
- **Wall-lottery margins banked by the round diet** (in-gate, cand vs base):
  sted2 −187.1s (1590.8s), rbsat −100.5s (1690.6s), aaai10-planning
  −108.8s, SCPC-500-14 −64.9s, TT492 −40.9s (1483.8s), oski40 −22.5s
  (1237.8s), oski20 −18.7s (1693.8s). vex 1456.1s. Every future gate
  inherits these. A few SAT cells ran slower under identical conflicts
  (bp4_CSO_IXA +183.6s) — SAT-lottery timing noise, priced into the PAR-2
  win.
- Kissat 4.0.4 reference: **74/100** (`log/kissat-medium-20260705-203444`).
  Net gap 5.
- Promotion ledger (newest first): 74eeaf0 SAT_ROUND_DIET (69v69 tie,
  conflicts exact tie, PAR-2 −405.6) | 56a0bb5 SAT_ELIM_SCRATCH (68v67 WIN,
  PAR-2 −1,975) | 70493e3 SAT_CONGRUENCE_FASTIDX (69v68, PAR-2 −1,864) |
  4bf2de4 flywheel groundwork default-OFF | 6199fb2 SAT_WATCH_POOL |
  2ed8e27 SAT_WATCH_INLINE_BIN | d23e454 SAT_HOTLOOP_PTR | 6633bc7
  binary-edge tag | 075b7e8 SAT_DECISION_ARM=24 | 038f9c1 binary DRAT |
  2f92794 vivify-yield arming | 3683ab5 vivify ALE | e5bd1f9 armed collapse
  bundle | 906e7cc giant-arena parse | 15911aa preflight | a402efd factor |
  c579bfe congruence inprocess | 689f080 chrono.
- **Wall-diet arc is now 6-for-6** (bintag, hotloop, watchpool, fastidx,
  elim-scratch, round-diet): trajectory-identical diets remain the most
  reliable gate winners. Note the marginal PAR-2 sizes are shrinking
  (−1975 → −406): the easy overhead is mostly gone; remaining chunks below
  are smaller and the arc is approaching diminishing returns.

## The kissat-only cells, with honest flippability verdicts

(Ours-not-kissat's cells like TT492 offset some of these; net gap ≈ 5.)

| cell | kissat | class | verdict |
|---|---|---|---|
| TT406 | 41s | decision-armed walk lottery | cheapest +1 in principle (solved at 236s in 075b7e8 lineage; traded for TT492). BLOCKED on a TT-class stabilizer; decision-armed rerolls are −EV while TT492 is in (TT492 margin now 1483.8s, deepest yet — rerolls riskier than ever). |
| Bubble | 354s | density | single mechanisms ALL measured dead. Remaining play: multi-mechanism ensemble/economics (kissat probe.c pass ORDER with per-pass budgets). |
| fixedbandwidth-eq-37 | 576s | density | same class as Bubble. |
| bp4_TCO_CSO_IXA_LP_ZR | 1287s | structured SAT (2.1 dec/conf) | never analyzed — cheap measurement session first. |
| pj2008 | 1165s | giant (8.6M vars), <200k conflicts | wall is formula size at parse; measure root-collapse vs memory-locality (SAT_TRACE_TIMING run + kissat -s compare) before code. |
| goldcrest | 1234s | BMC, <1M conflicts | flywheel inert; needs earlier collapse or rate work. |
| booth_wallace / booth_dadda | 1371/1389s | density | same as Bubble class. |
| lockchart-group1 | 1687s | walk economics | kissat needs 94% of budget — NOT realistic this generation. |
| g2 | 1758.9s | unarmed BMC | kissat needs 97.7% of budget — NOT realistic. |

oski20 is OFF this list: solved in both arms this gate with ~106s margin.
It remains contention-sensitive — watch it in every gate, don't build on it.

## Load-bearing discoveries (cumulative; newest first)

1. **Round-diet session (2026-07-19 evening)**: (a) congruence round-0
   dry-run plan reuse is self-guarding but NEAR-INERT on ibm — round 0
   keeps finding 500-900 new hidden binaries per armed invocation, so the
   edit-free precondition rarely holds; measure on vex/oski before
   extending it. (b) try_els was cloning original_clause_ids (multi-MB)
   per call with a provably read-only consumer — grep for other defensive
   clones on hot paths. (c) The CSR-for-Vec<Vec> swap
   (compute_representatives_csr) is identity-safe when both build passes
   scan the input stream in order — reusable pattern for other adjacency
   builds.
2. **BVE apply-path decomposition (SAT_TRACE_ELIM)**: vex root eliminate
   24.7s = BVE 22.5s with apply 17.7s — resolvent INSERTION 12.0s spread
   evenly over normalize/proof/arena/attach/occurrence-index. **Kissat
   spends ~24.7s in eliminate on vex too** → eliminate cost is at PARITY;
   overhead diets, not architecture ports, are the honest play. (Kissat's
   dense-mode/no-watcher elimination would reroll trajectories — only as a
   deliberate reroll gate.)
3. **Props/s PARITY with kissat** at equal conflict counts: the kissat rate
   gap on g2-class is clause-DB size (continuous inprocessing collapses g2
   888k→37k irredundant; we freeze at ~500k → 2x props/conflict).
   Propagation-throughput ports for rate are DEAD; CSR demoted to
   cache-diet.
4. **Wall-decompose before optimizing** (SAT_TRACE_TIMING / SAT_TRACE_ELIM /
   SAT_DEBUG_CONGRUENCE): every diet win started from a measured chunk.
   Remaining measured non-search chunks: congruence gates-Vec dealloc churn
   (871k inputs Vecs/round through find_merges_closure), eliminate `other`
   ≈1.2s/round (now attributable via the new heap_build sub-timer —
   unmeasured), compute_representatives_csr's 5 flat per-call arrays,
   congruence merge-application second try_els (partially dieted by
   round-diet).
5. **Trajectory-identical wall diets are 6-for-6**. The identity recipe:
   byte-compare stats AND per-mechanism counters (incl. mid-search armed
   rounds) on 3-4 armed cells + full SAT_STATS_JSON with volatile fields
   stripped (see cmp_json.py approach: drop *_sec, seconds_*, max_rss_mb,
   shas, config_hash, feature_maturity), across cand / off-arm /
   pre-change binaries; verbatim legacy off-switch arm for the simultaneous
   A/B (SAT_ROUND_DIET=off, SAT_ELIM_SCRATCH=off precedents).
6. **Hash-order insensitivity is provable from cross-process
   reproducibility** (fastidx): fixed-seed FxHash (`src/fxhash.rs`) is
   inside the tested envelope; reusable for other hash-hot paths.
7. **Incremental gate-extraction caching is BLOCKED on lit-order
   sensitivity**; canonicalized (sorted-lit) extraction unblocks it but is
   a full-suite reroll — only do it WITH the cache in the same gate.
8. **Flywheel groundwork exists** (SAT_ELIM_UNARMED_FLYWHEEL, default OFF,
   4bf2de4): g2 −12% wall at 2M conflicts, no flip — not gate-worthy alone.
9. **The gate-EV method (4 sessions, 4 correct predictions)**: enumerate the
   reroll surface from the last gate TSV, screen plausible flips standalone,
   predict the lexicographic outcome BEFORE running the gate.
   Trajectory-identical changes have empty reroll surface → predicted
   tie/tie/PAR-2-win + lottery upside; happened again (69v69/tie/−406).

## RANKED PLAN for next session

1. **Wall-diet arc, next (and likely near-final) bundle** — the chunks are
   getting smaller; bundle ALL of these into ONE gate:
   a. Congruence **flat gate arena**: Gate.inputs as (start,len) into one
      literal arena through extract_gates_for_congruence +
      find_merges_closure (kills 871k per-gate Vec allocs+deallocs per
      round). Medium refactor; identity-safe (values unchanged); legacy fns
      verbatim under the knob. Touches MergeKind::Xor inputs too.
   b. **compute_representatives_csr workspace persistence**: the 5 flat
      arrays (disc/low/on_stack/comp_min/repr) + CSR arrays per call →
      persistent workspace param (2 calls/congruence round + root ELS).
   c. **Measure eliminate `other` first** via the new heap_build sub-timer
      (SAT_TRACE_ELIM on vex): if the per-round heap rebuild is chunky, the
      kissat persistent-schedule change is algorithmic (reroll risk —
      separate deliberate gate, NOT part of the diet bundle).
   d. Measure round-0 dry-run reuse hit-rate on vex/oski40
      (SAT_DEBUG_CONGRUENCE, grep "reusing dry-run") before extending.
   Expected: tie/tie/PAR-2 win, smaller than −406. If the measured idle
   deltas come in under ~2s/armed cell, SKIP the gate (3.5h) and move to #2
   — the arc is allowed to end.
2. **Canonicalization + incremental extraction** (the big congruence win,
   one deliberate-reroll gate): sort clause lits so gate extraction is
   lit-order-insensitive, THEN per-clause touched-var gate cache
   (invalidation rule proven sound). Full-suite reroll; watch the
   TT/sted2/rbsat/oski20 lottery — the margins banked by the diets are the
   insurance for exactly this kind of reroll.
3. **Density-class ensemble** (Bubble/fixedbandwidth/booth — 4 cells,
   kissat's biggest margins): kissat probe.c ORDER with per-pass effort
   budgets (congruence → substitute → backbone → vivify → sweep →
   substitute → transitive → backbone → factor, re-run while active vars
   drop). First step stays: instrument clause-count-per-round on Bubble
   under the armed bundle; kissat target curve is 888k→45k-style collapse.
   A Bubble flip alone = +1.
4. **Flywheel ensemble variant** (g2-class) — only AFTER #3 teaches
   clause-mass cleanup; 59-129706 must not regress beyond the class's wins.
5. **TT406 stabilizer** — do NOT reroll decision-armed class blind (TT492
   in with 1483.8s, deepest margin yet; rerolls trade TT cells). Attack
   only with a concrete mechanism hypothesis + paired screens on
   TT406/TT492/C_395.
6. **pj2008 / bp4_TCO measurement** (pure measurement sessions; no code).

## Measured-dead ledger (do NOT re-run blind)

- Propagation-throughput ports for rate: props/s parity measured.
- Lit-indexed values array: wall LOSER (lockchart +5.7%).
- Bound escalation on armed cells: conflicts LOSER; decision-armed variant
  trades TT406↔TT492 net 0.
- SAT_INPROCESS_ROUNDS=2: oski20 −19% but oski40 +42% — no honest scoping.
- Unarmed eliminate at fast cadence WITHOUT escalation+gates: pure tax.
- Congruence-learned extraction: byte-identical on vex.
- elim-def (kitten definitions): densification kills oski40; defcores DEAD.
- Congruence round-0 dry-run reuse on ibm-class: precondition (edit-free
  round 0) almost never holds — component is in-tree and free, but don't
  invest in extending it without a measured hit-rate elsewhere.
- Backbone, transitive reduction (vex/density), rephase/walk global or
  yield-armed, restart floors/margins, vivify-deduce, vivify-sort, trail
  reuse, ELIM_PRODUCTIVE_MIN_PCT, walk warmup: all dead in noted scopes.
- lockchart-g1 and g2 as flip targets: kissat needs 94-98% of budget itself.

## Standing traps (consolidated)

- check_promotion_gate `running_solver_processes` FAIL from monitor/watcher
  shells — yours OR a previous session's. Kill by PID; `pkill -f` self-matches.
  (Clean this session, but stop any pgrep-loop monitors before the check.)
- feature_ablation setup runs ~2 min single-threaded BEFORE the [abtest]
  line appears — not hung. Gate tail: drat-trim verify of vex/sqrt-miter
  proofs adds ~35 min after the last solver exits — not hung.
- **SAT_TRACE_ELIM heisenberg**: finest-grain sub-timers inflate the hot
  path ~2x when tracing is ON. Ratios at the finest level; absolutes only
  from coarse tiers. OFF = branch-only, no default-path tax.
- Don't diff logs of still-running screens; wait for process exit.
- `timeout N env sat-solver …` kills before stats JSON — use
  SAT_LIMIT_CONFLICTS for end-state stats. Stats JSON lands on stderr as a
  `c JSON_STATS {...}` line; strip volatile fields before byte-comparing
  (*_sec, seconds_*, max_rss_mb, solver_git_sha, binary_sha256,
  config_hash, feature_maturity_summary).
- Ablation TSV TIMEOUT rows carry zero conflicts — class analysis of
  unsolved cells needs standalone screens.
- kissat progress lines: conflicts is $10; `-s -q` mutually exclusive;
  drat-trim prints \r; kissat --conflicts=1000 exits BEFORE its first
  eliminate — use ≥100k-conflict runs for inprocess profiling.
- Trajectory-identity for watcher/arena-order changes needs list-order
  evolution + tick parity + bump-order parity (inlinebin recipe); any
  change to resting clause-lit order rerolls armed cells.
- 2-arm gates only (3-arm changes the contention profile). Wall-lottery
  cells now (this gate's cand walls): oski20 1693.8s, rbsat 1690.6s, sted2
  1590.8s, TT492 1483.8s, vex 1456.1s, oski40 1237.8s.
- feature_ablation keeps only results.tsv per arm — extract per-cell stats
  DURING the run or re-screen.
- oski-class standalone walls are load/thermal-sensitive; pair everything.
- Giants (>20M vars): any new persistent workspace must be freed in the
  eliminate turn_off path before GC (see 74eeaf0 / cd8f1b5 precedent).

## Instrumentation now in-tree (use it)

- `SAT_TRACE_TIMING=1`: wall checkpoints (parse / frontend / Solver::new /
  root_propagate / pair_abs_gauss_els / congruence_root / search_start /
  model steps).
- `SAT_TRACE_ELIM=1`: eliminate decomposition — occ_build/bsr/bve/gather
  totals + NEW heap_build sub-timer + BVE sub-steps + apply sub-steps +
  insertion sub-steps. See heisenberg trap.
- `SAT_DEBUG_CONGRUENCE=1`: dry-run + per-round + per-step closure timings,
  merge counts, and the NEW "reusing dry-run plan" line (round-0 reuse
  hit-rate).
- `SAT_TRACE_PREPROCESS_DETAILS=1`: elim_round counters (cumulative — diff
  consecutive lines), vivify_yield_probe, unarmed_flywheel lines.
- `SAT_STATS_JSON=1`: full end-state stats on stderr (`c JSON_STATS`).
- Off-switch A/B knobs: `SAT_ROUND_DIET=off` (NEW — pre-diet eliminate
  round allocs, try_els_legacy, Vec-of-Vec ELS graph, no round-0 reuse),
  `SAT_ELIM_SCRATCH=off`, `SAT_CONGRUENCE_FASTIDX=off`,
  `SAT_ELIM_UNARMED_FLYWHEEL=on`, plus historical knobs in each promotion
  note.

## Where the evidence lives

- Newest session: `plan/next-steps-rounddiet-2026-07-19.md`, gate
  `log/abtest-cand-vs-base-2026-07-19-16-01-57` + launch log
  `log/abtest-rounddiet-launch.log`, commit 74eeaf0.
- Prior arc: `next-steps-elimscratch-2026-07-19.md`,
  `next-steps-fastidx-promotion-2026-07-19.md`,
  `next-steps-flywheel-decomposition-2026-07-18.md`,
  `next-steps-elimbounds-negatives-2026-07-18.md`,
  `next-steps-walkwarmup-watchpool-2026-07-17.md`,
  `next-steps-inlinebin-2026-07-17.md`, and the superseded
  `next-steps-AGGREGATED-2026-07-19b.md` (valid as provenance).
- Current gate baseline TSVs for the NEXT A/B:
  `log/abtest-cand-vs-base-2026-07-19-16-01-57` (cand arm = promoted
  lineage, 69/100 with oski20 AND rbsat in).
- Bead: `SAT-playground-2a7`.
