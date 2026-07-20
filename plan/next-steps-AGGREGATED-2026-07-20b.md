# AGGREGATED next-steps plan — 2026-07-20b (supersedes next-steps-AGGREGATED-2026-07-20.md)

One-file plan for the next session. Folds the 2026-07-20 extract-cache
promotion on top of everything the 2026-07-20 morning aggregate covered.
Where this file contradicts an older `plan/next-steps-*.md`, THIS file wins;
older notes are provenance and negative-result ledgers only.

## Current state (verified 2026-07-20, end of session)

- HEAD: **c8228aa** = `SAT_EXTRACT_CACHE` congruence gate-extraction cache
  (default ON). Newest gate `log/abtest-cand-vs-base-2026-07-20-12-03-06`:
  **PASS, WIN — solved 69 vs 68 (oski20 FLIP back IN: cand UNSAT 1692.3s vs
  base TIMEOUT), both-solved conflicts EXACT tie (delta 0, 68 cells), PAR-2
  138,391.4 vs 140,258.6 (−1,867.2)**. Zero contradictions/correctness
  failures. **Lineage 69 is REALIZED in this gate's cand arm** (first time
  since 74eeaf0 that 69 is in-hand, not just implied).
- Kissat 4.0.4 reference: **74/100** (`log/kissat-medium-20260705-203444`).
  Net gap = 5.
- Wall-lottery cells, newest in-gate cand walls: **rbsat 1794.6s (margin
  5.4s — THINNEST EVER, pure coin-flip now, never build on it)**, oski20
  1692.3s (margin 107.7s — reclaimed), sted2 1636.4s, vex 1494.2s, oski40
  1237.8s. sted2/rbsat wobbled negative this gate with identical conflicts
  (load noise, not mechanism).
- Promotion ledger (newest first): c8228aa SAT_EXTRACT_CACHE (69v68 WIN,
  oski20 flip, conflicts exact tie, PAR-2 −1,867) | cc072b2 SAT_CLOSURE_DIET
  (68v67, rbsat flip, PAR-2 −2,758) | 74eeaf0 SAT_ROUND_DIET (69v69, PAR-2
  −406) | 56a0bb5 SAT_ELIM_SCRATCH (68v67, PAR-2 −1,975) | 70493e3
  SAT_CONGRUENCE_FASTIDX (69v68, PAR-2 −1,864) | 6199f b2 SAT_WATCH_POOL |
  2ed8e27 SAT_WATCH_INLINE_BIN | d23e454 SAT_HOTLOOP_PTR | 6633bc7
  binary-edge tag | 075b7e8 SAT_DECISION_ARM=24 | 038f9c1 binary DRAT |
  2f92794 vivify-yield arming | 3683ab5 vivify ALE | e5bd1f9 armed collapse
  bundle | 906e7cc giant-arena parse | 15911aa preflight | a402efd factor |
  c579bfe congruence inprocess | 689f080 chrono.
- **Wall-diet arc is 8-for-8** and has now converted TWO flips in a row
  (rbsat via closure-diet, oski20 via extract-cache). Remaining measured
  chunks: closure `occ` Vec-of-Vec (~0.36s/round closure step on ibm),
  sweep snapshot clones — <2s/cell; the arc remains at its documented end
  unless bundled with other work.

## The kissat-only cells, with honest flippability verdicts

(Ours-not-kissat's cells like TT492 offset some of these; net gap = 5.)

| cell | kissat | class | verdict |
|---|---|---|---|
| TT406 | 41s | decision-armed walk lottery | cheapest +1 in principle, BLOCKED on a TT-class stabilizer; rerolls are −EV while TT492 is in. |
| Bubble | 354s | density | inprocessing route CLOSED (2026-07-19). Conflict-rate/quality-bound: 8.6k/s vs kissat 22k/s AT EQUAL COLLAPSE. Only rate/quality work can flip. |
| fixedbandwidth-eq-37 | 576s | density | same class; BVE-blocked structurally (35/149 vars eliminable at grow=16). |
| booth_wallace / booth_dadda | 1371/1389s | density | same class; conflict QUALITY not volume (booth passed kissat's refutation point at 14.1M without converting). |
| bp4_TCO_CSO_IXA_LP_ZR | 1287s | structured SAT (2.1 dec/conf) | never analyzed — cheap measurement session first. |
| pj2008 | 1165s | giant (8.6M vars), <200k conflicts | measure root-collapse vs memory-locality before code. |
| goldcrest | 1234s | BMC, <1M conflicts | flywheel inert; propagation-bound (7.8k props/conf). |
| lockchart-group1 | 1687s | walk economics | kissat needs 94% of budget — NOT realistic this generation. |
| g2 | 1758.9s | unarmed BMC | kissat needs 97.7% of budget — NOT realistic. |

## Load-bearing discoveries (cumulative; newest first)

1. **Extract-cache session (2026-07-20)**: (a) cross-clause gate
   dependencies are EXACTLY len<=3 clauses (AND↔binaries, ITE↔ternaries;
   XOR recomputed fresh) — invalidation scoped to len<=3 edits took reuse
   from 31% to 98-99.9%; the general lesson: scope invalidation to the
   actual dependency footprint, not "all edits". (b) Within-invocation
   caching needs NO canonicalization — lit-order sensitivity only bites
   across invocations (watch swaps only reorder clauses that die for
   extraction). (c) A full-formula self-verify mode
   (SAT_EXTRACT_CACHE_VERIFY asserting cached==fresh per round on real
   cells) is cheap to build and is stronger evidence than unit fuzzing.
   (d) Tail-appended chains preserve per-key insertion order in flat hash
   structures (head-insertion reverses order and rerolls).
2. **Closure-diet session (2026-07-19)**: per-key bucket Vecs undo arena
   diets; flat-arena + content-keyed chained table is identity-safe and
   fuzz-provable; a wall diet CAN buy a flip.
3. **Density class is conflict-rate/quality-bound, NOT collapse-bound**:
   at equal collapse ours 8.6k conf/s vs kissat 22k. Remaining deltas worth
   measuring: props/conflict on the COLLAPSED DB, learned-clause quality
   (kissat vivifies 54% of checks), reduce/retention policy.
4. **Armed-cell rerolls remain a casino**; no reliable conflicts-tier win
   in the yield-armed bundle space.
5. **Mid-search factor densification hurts yield-armed cells** — keep
   factor decision-armed-only.
6. **Props/s PARITY with kissat** at equal conflict counts on g2-class;
   rate gap there = clause-DB size.
7. **Trajectory-identical wall diets are 8-for-8.** Identity recipe:
   byte-compare stripped SAT_STATS_JSON (drop *_sec, seconds_*, elapsed*,
   max_rss_mb, shas, config_hash, feature_maturity) across cand / off-arm /
   pre-change binaries on 3-4 armed cells (SAT_LIMIT_CONFLICTS for bounded
   cells); verbatim legacy off-switch arm for the simultaneous A/B.
8. **The gate-EV method (6 sessions, 6 correct predictions)**: enumerate
   the reroll surface from the last gate TSV, screen plausible flips
   standalone, predict the lexicographic outcome BEFORE the gate.
   Trajectory-identical → empty reroll surface → tie-or-flip/tie/PAR-2-win.
   The lottery has now paid twice (rbsat, then oski20).
9. Congruence round-0 dry-run reuse fires on vex only; incremental
   gate-extraction ACROSS invocations still blocked on lit-order
   sensitivity (canonicalization = deliberate reroll, see plan #3).

## RANKED PLAN for next session

1. **Density-class rate measurement** (no code until measured; now the top
   play — the wall-diet arc is done and the congruence extraction cost is
   harvested): on Bubble with the armed bundle at ~5M conflicts, measure
   props/conflict and ticks/prop vs kissat on the collapsed formula
   (SAT_STATS_JSON + kissat -s at matched conflict counts). Decide between:
   watcher-layout work (CSR/merged long-clause — also serves
   oski20/goldcrest/pj2008), reduce/retention policy, or learned-clause
   quality (kissat vivifies 54% of checks — our vivify attempts/success on
   the same cells is measurable from stats). The inprocessing-ensemble
   route is CLOSED (do not reopen).
2. **bp4_TCO / pj2008 measurement sessions** (cheap, no code): bp4 never
   analyzed (2.1 dec/conf structured SAT); pj2008 = parse/root-collapse vs
   memory locality (SAT_TRACE_TIMING + kissat -s compare).
3. **Canonicalization + cross-invocation gate cache** (the remaining
   congruence play, ONE deliberate-reroll gate): sort clause lits so
   extraction is lit-order-insensitive, then persist the extract-cache
   ACROSS invocations (the within-invocation infrastructure from c8228aa is
   the foundation — hooks would need to run outside the window too, or the
   cache re-seeded per invocation from a canonical fingerprint). Payoff
   after c8228aa is the dry-run + round-0/1 extractions (~6-7s on ibm, less
   elsewhere) — SMALLER than it was; weigh against the reroll risk
   (rbsat at 5.4s margin WILL flip a coin; price ±1 into the prediction).
4. **TT406 stabilizer** — unchanged: do NOT reroll decision-armed class
   blind (TT492 in); attack only with a concrete mechanism hypothesis +
   paired screens on TT406/TT492/C_395.
5. **Wall-diet arc**: DONE (8-for-8, two flips). Remaining chunks (closure
   `occ` Vec-of-Vec ~0.36s/round, sweep snapshot clones) — bundle only into
   a gate that is happening anyway.

## Measured-dead ledger (do NOT re-run blind)

- Density inprocessing ensemble (ALL variants). 2026-07-19.
- Kissat persistent-elimination-schedule port (heap_build 4ms/round).
- Bound escalation on armed cells; SAT_INPROCESS_ROUNDS=2 global;
  lit-indexed values; congruence-learned extraction; unarmed eliminate at
  fast cadence; elim-def/defcores; backbone; transitive reduction
  (vex/density); rephase/walk global or yield-armed; restart floors/margins;
  vivify-deduce; vivify-sort; trail reuse; ELIM_PRODUCTIVE_MIN_PCT; walk
  warmup: all dead in noted scopes (see 19c and older notes).
- Propagation-throughput ports for g2-class rate: props/s parity measured.
- lockchart-g1 and g2 as flip targets: kissat needs 94-98% of budget.

## Standing traps (consolidated)

- check_promotion_gate `running_solver_processes` FAIL from monitor/watcher
  shells — kill watchers by task id BEFORE the check; `pgrep -f`
  self-matches.
- Agent-harness backgrounded subshells can lose their cwd (a decompress
  landed in the solver dir this session — outputs land in the wrong
  directory and look like silent failures). Run redirections with absolute
  paths or as separate foreground commands.
- Per-key bucket containers in hash tables undo arena diets; tail-append
  chains (not head-insert) to preserve per-key order.
- feature_ablation setup runs ~2 min single-threaded before the [abtest]
  line; drat-trim verify adds ~35 min after the last solver exits; launch
  log flushes result lines late — poll counts, don't infer hangs.
- `timeout N env sat-solver …` kills before stats JSON — use
  SAT_LIMIT_CONFLICTS for end-state stats (stats land on stderr as
  `c JSON_STATS`; strip volatile fields before byte-comparing).
- SAT_TRACE_ELIM heisenberg: finest sub-timers inflate the hot path ~2x.
- Ablation TSV TIMEOUT rows carry zero conflicts — class analysis of
  unsolved cells needs standalone screens.
- kissat: `-s -q` mutually exclusive; --conflicts=1000 exits before its
  first eliminate (use >=100k); drat-trim prints \r.
- Any change to resting clause-lit order rerolls armed cells (plan #3!).
- 2-arm gates only; oski-class walls are load/thermal-sensitive; pair
  everything. rbsat margin 5.4s: coin-flip, never build on it.
- Giants (>20M vars): every persistent workspace must be freed in the
  eliminate turn-off path before GC (ExtractCache joined ElsCsrWs in
  c8228aa).

## Instrumentation in-tree (use it)

- `SAT_TRACE_TIMING=1`, `SAT_TRACE_ELIM=1`, `SAT_DEBUG_CONGRUENCE=1`,
  `SAT_DEBUG_ELS=1`, `SAT_TRACE_PREPROCESS_DETAILS=1`, `SAT_STATS_JSON=1`
  (see 2026-07-20 morning aggregate for details).
- NEW: `SAT_TRACE_EXTRACT=1` — per-extraction scan/and/ite/xor split +
  reuse counts (both cached and shipped paths). `SAT_EXTRACT_CACHE_VERIFY=1`
  — assert cached==fresh extraction every round (use on 1-2 real cells
  after ANY change touching congruence extraction or the hooks).
- Off-switch A/B knobs: `SAT_EXTRACT_CACHE=off` (NEW — shipped flat
  extraction verbatim), `SAT_CLOSURE_DIET=off`, `SAT_ROUND_DIET=off`,
  `SAT_ELIM_SCRATCH=off`, `SAT_CONGRUENCE_FASTIDX=off`, plus historical
  knobs in each promotion note.

## Where the evidence lives

- Newest session: `plan/next-steps-extractcache-2026-07-20.md`, gate
  `log/abtest-cand-vs-base-2026-07-20-12-03-06` + launch log
  `log/abtest-extractcache-launch.log`, commit c8228aa.
- Prior arc: `next-steps-AGGREGATED-2026-07-20.md` (superseded, valid as
  provenance) and the per-session notes it lists.
- **Baseline TSV for the NEXT A/B**:
  `log/abtest-cand-vs-base-2026-07-20-12-03-06/cand/results.tsv` (69/100:
  oski20 IN at 107.7s margin, rbsat IN at 5.4s margin).
- Beads: SAT-playground-5b2.3.39 (congruence, in progress — extract-cache
  work recorded there); SAT-playground-5b2.3.50 (open: global-effort
  cadence redesign scope remains).
