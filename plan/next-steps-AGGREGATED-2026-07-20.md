# AGGREGATED next-steps plan — 2026-07-20 (supersedes next-steps-AGGREGATED-2026-07-19c.md)

One-file plan for the next session. Folds the 2026-07-19 night closure-diet +
density-ensemble session on top of everything the 19c aggregate covered. Where
this file contradicts an older `plan/next-steps-*.md`, THIS file wins; older
notes are provenance and negative-result ledgers only.

## Current state (verified 2026-07-20, start of session)

- HEAD: **94edc94** = cc072b2 (`SAT_CLOSURE_DIET` congruence/ELS closure
  allocation diet, default ON) + beads sync. Newest gate
  `log/abtest-cand-vs-base-2026-07-19-20-53-42`: **PASS, WIN — solved 68 vs
  67 (rbsat-v1375 FLIP: cand SAT 1726.6s vs base TIMEOUT), both-solved
  conflicts EXACT tie (delta 0, all 100 cells), PAR-2 140,817.4 vs
  143,575.2 (−2,757.9)**. Zero contradictions/correctness failures.
  `check_promotion_gate` formal PASS.
- **Lineage cell count remains 69**: this gate's cand arm is 68 because
  oski20 timed out in BOTH arms (documented contention-sensitive coin-flip;
  cand 1800.2s vs its 1693.8s solve last gate). rbsat is now IN with a
  73.5s margin — the thinnest banked margin; treat rbsat exactly as oski20
  was treated: watch it every gate, never build on it.
- Kissat 4.0.4 reference: **74/100** (`log/kissat-medium-20260705-203444`).
  Net gap ≈ 5 (lineage 69 vs 74).
- Wall-lottery cells, newest in-gate cand walls: rbsat 1726.6s (margin
  73.5s), oski20 TIMEOUT (was 1693.8s — reclaims on a quiet gate), sted2
  1564.1s (−136.5 this gate), vex 1618.1s, TT-901fa 1500.6s (−132.4),
  TT492 ~1484s (19c), oski40 1205.7s (−17.0).
- Promotion ledger (newest first): cc072b2 SAT_CLOSURE_DIET (68v67 WIN,
  rbsat flip, conflicts exact tie, PAR-2 −2,758) | 74eeaf0 SAT_ROUND_DIET
  (69v69, PAR-2 −406) | 56a0bb5 SAT_ELIM_SCRATCH (68v67, PAR-2 −1,975) |
  70493e3 SAT_CONGRUENCE_FASTIDX (69v68, PAR-2 −1,864) | 4bf2de4 flywheel
  groundwork default-OFF | 6199fb2 SAT_WATCH_POOL | 2ed8e27
  SAT_WATCH_INLINE_BIN | d23e454 SAT_HOTLOOP_PTR | 6633bc7 binary-edge tag |
  075b7e8 SAT_DECISION_ARM=24 | 038f9c1 binary DRAT | 2f92794 vivify-yield
  arming | 3683ab5 vivify ALE | e5bd1f9 armed collapse bundle | 906e7cc
  giant-arena parse | 15911aa preflight | a402efd factor | c579bfe
  congruence inprocess | 689f080 chrono.
- **Wall-diet arc is 7-for-7** (bintag, hotloop, watchpool, fastidx,
  elim-scratch, round-diet, closure-diet) and this one converted a flip.
  Remaining measured chunks (closure `occ` Vec-of-Vec, sweep snapshot
  clones) look <2s/cell — the arc is at its documented end; only bundle
  opportunistically with other work.

## The kissat-only cells, with honest flippability verdicts

(Ours-not-kissat's cells like TT492 offset some of these; net gap ≈ 5.)

| cell | kissat | class | verdict |
|---|---|---|---|
| TT406 | 41s | decision-armed walk lottery | cheapest +1 in principle, BLOCKED on a TT-class stabilizer; decision-armed rerolls are −EV while TT492 is in. |
| Bubble | 354s | density | **inprocessing route CLOSED 2026-07-19**: full ensemble deepened collapse 56→72% but no refutation at 15.5M conflicts (kissat 6.5M). Class is conflict-rate/quality-bound: 8.6k/s vs kissat 22k/s AT EQUAL COLLAPSE. Only rate/quality work can flip these. |
| fixedbandwidth-eq-37 | 576s | density | same class; additionally BVE-blocked structurally (35/149 vars eliminable at grow=16). |
| booth_wallace / booth_dadda | 1371/1389s | density | same class; booth passed kissat's 12.1M-conflict refutation point at 14.1M without converting — conflict QUALITY, not volume. |
| bp4_TCO_CSO_IXA_LP_ZR | 1287s | structured SAT (2.1 dec/conf) | never analyzed — cheap measurement session first. |
| pj2008 | 1165s | giant (8.6M vars), <200k conflicts | wall is formula size at parse; measure root-collapse vs memory-locality before code. |
| goldcrest | 1234s | BMC, <1M conflicts | flywheel inert; propagation-bound (7.8k props/conf). |
| lockchart-group1 | 1687s | walk economics | kissat needs 94% of budget — NOT realistic this generation. |
| g2 | 1758.9s | unarmed BMC | kissat needs 97.7% of budget — NOT realistic. |

## Load-bearing discoveries (cumulative; newest first)

1. **Closure-diet session (2026-07-19 night)**: (a) per-key bucket
   containers (`HashMap<hash, Vec<entry>>`) silently reintroduce the alloc
   churn an arena removes — ~1.2M bucket Vecs measured as a wall REGRESSION
   before the chain-in-one-flat-Vec redesign fixed it. Audit any future
   arena work for this. (b) The flat-arena + content-keyed chained table is
   identity-safe and fuzz-provable (800 random gate sets vs legacy closure);
   reusable for other Vec-of-Vec hash pipelines. (c) A wall diet CAN buy a
   flip: rbsat was 9s from timeout and flipped in with −73.5s.
2. **Density class is conflict-rate/quality-bound, NOT collapse-bound**
   (kills the 19c #3 ensemble play): scoped ELS-substitute passes + kissat
   sweep-schedule cursor + factor + proberounds=2 reproduce kissat's
   variable-deactivation shape on Bubble/booth but never refute. Kissat
   Bubble reference (this host): 6.53M conflicts @295s, 88% deactivated
   (2506 elim + 314 substitute + 168 congruent + 100 sweep), 434k vivified.
   Ours at equal collapse: 8.6k conf/s vs 22k. The remaining deltas worth
   measuring there: props/conflict on the COLLAPSED DB, learned-clause
   quality (kissat vivifies 54% of checks), reduce/retention policy.
3. **Armed-cell rerolls remain a casino** (7-cell yield-armed pairs, two
   bundle variants): per-cell conflict deltas flip sign between variants
   (sqrt170 −127k → +175k when factor left the bundle); QG7's regression was
   factor-only; Pancake's −93k needed factor OUT. No reliable conflicts-tier
   win exists in that bundle space — don't re-enter without a flip-grade
   mechanism.
4. **Mid-search factor densification hurts yield-armed cells** (QG7 +113k
   with factor as the only delta; Pancake +828k with 220 fresh vars) even
   though kissat factors these cells — kissat's economics differ; keep
   factor decision-armed-only.
5. **eliminate heap_build = 4ms/round, `other` ≈ 30ms/round (vex)**: the
   kissat persistent-elimination-schedule port (19c item 1c) is CLOSED as
   measured-inert.
6. **BVE apply-path decomposition**: vex eliminate cost is at PARITY with
   kissat; overhead diets, not architecture ports, were the honest play —
   now largely harvested.
7. **Props/s PARITY with kissat** at equal conflict counts on g2-class; rate
   gap there = clause-DB size. (Bubble-class post-collapse rate gap is the
   NEW open question — measure props/conflict there before believing any
   rate story.)
8. **Trajectory-identical wall diets are 7-for-7.** Identity recipe:
   byte-compare stripped SAT_STATS_JSON (drop *_sec, seconds_*, elapsed*,
   max_rss_mb, shas, config_hash, feature_maturity) across cand / off-arm /
   pre-change binaries on 3-4 armed cells (use SAT_LIMIT_CONFLICTS for
   bounded cells); verbatim legacy off-switch arm for the simultaneous A/B.
9. **Hash-order insensitivity is provable from cross-process
   reproducibility** (fastidx); fixed-seed FxHash is inside the tested
   envelope.
10. **Incremental gate-extraction caching is BLOCKED on lit-order
    sensitivity**; canonicalized (sorted-lit) extraction unblocks it but is
    a full-suite reroll — only do it WITH the cache in the same gate.
11. **Congruence round-0 dry-run reuse** fires on vex (~3s/run saved),
    never on ibm/oski40 — in-tree, free, don't extend.
12. **The gate-EV method (5 sessions, 5 correct predictions)**: enumerate
    the reroll surface from the last gate TSV, screen plausible flips
    standalone, predict the lexicographic outcome BEFORE the gate.
    Trajectory-identical → empty reroll surface → tie/tie/PAR-2-win +
    lottery upside (this time the lottery paid: rbsat).

## RANKED PLAN for next session

1. **Canonicalization + incremental gate extraction** (the big congruence
   play, ONE deliberate-reroll gate): sort clause lits so gate extraction is
   lit-order-insensitive, THEN the per-clause touched-var gate cache
   (invalidation rule proven sound). Extraction is 2.1-2.7s per armed round
   × 5-20 rounds per cell even after the closure diet — the cache targets
   most of it. Full-suite reroll; the banked margins (sted2 1564s, TT-901fa
   1500s, oski40 1206s, TT492 1484s) are the insurance, but note rbsat's
   thin 73.5s margin WILL wobble — price a ±1 on it into the prediction.
2. **Density-class rate measurement** (no code until measured): on Bubble
   with the armed bundle at ~5M conflicts, measure props/conflict and
   ticks/prop vs kissat on the collapsed formula (SAT_STATS_JSON +
   kissat -s at matched conflict counts). Decide between: watcher-layout
   work (CSR/merged long-clause — also serves oski20/goldcrest/pj2008),
   reduce/retention policy, or learned-clause quality. The inprocessing-
   ensemble route is CLOSED (do not reopen).
3. **bp4_TCO / pj2008 measurement sessions** (cheap, no code): bp4 never
   analyzed; pj2008 = parse/root-collapse vs memory locality
   (SAT_TRACE_TIMING + kissat -s compare).
4. **TT406 stabilizer** — unchanged: do NOT reroll decision-armed class
   blind (TT492 in at ~1484s); attack only with a concrete mechanism
   hypothesis + paired screens on TT406/TT492/C_395.
5. **Wall-diet arc**: at its end. Remaining chunks (closure `occ`
   Vec-of-Vec, sweep snapshot clone) <2s/cell — bundle only if #1's gate is
   happening anyway and the additions are identity-safe.

## Measured-dead ledger (do NOT re-run blind)

- **Density inprocessing ensemble (ALL variants)**: ELS-substitute passes,
  sweep persistent cursor, yield-armed factor, proberounds=2 — structurally
  effective, zero flips, reroll-casino conflicts. 2026-07-19.
- Kissat persistent-elimination-schedule port: heap_build 4ms/round.
- Bound escalation on armed cells; SAT_INPROCESS_ROUNDS=2 global;
  lit-indexed values; congruence-learned extraction; unarmed eliminate at
  fast cadence; elim-def/defcores; backbone; transitive reduction
  (vex/density); rephase/walk global or yield-armed; restart floors/margins;
  vivify-deduce; vivify-sort; trail reuse; ELIM_PRODUCTIVE_MIN_PCT; walk
  warmup: all dead in noted scopes (see 19c and older notes).
- Propagation-throughput ports for g2-class rate: props/s parity measured.
- lockchart-g1 and g2 as flip targets: kissat needs 94-98% of budget itself.

## Standing traps (consolidated)

- check_promotion_gate `running_solver_processes` FAIL from monitor/watcher
  shells — yours OR a previous session's. Kill by task id BEFORE the check;
  `pkill -f` and `pgrep -f` self-match, and bracket patterns ([n]) do NOT
  help when the string lives in ANOTHER monitor's cmdline.
- Agent-harness compound `( a & ) ( b & ); wait` can lose a subshell's cwd
  (outputs land in the wrong directory, look like a silent failure) — run
  paired screens as SEPARATE background tasks.
- Per-key bucket containers in hash tables undo arena diets (see discovery
  #1).
- feature_ablation setup runs ~2 min single-threaded before the [abtest]
  line; per-run result.json files are consumed after collection (poll the
  work-dir high-water mark, not result counts); gate tail: drat-trim verify
  adds ~35 min after the last solver exits.
- `timeout N env sat-solver …` kills before stats JSON — use
  SAT_LIMIT_CONFLICTS for end-state stats. Stats land on stderr as
  `c JSON_STATS {...}`; strip volatile fields before byte-comparing.
- SAT_TRACE_ELIM heisenberg: finest sub-timers inflate the hot path ~2x;
  ratios at the finest level, absolutes only from coarse tiers.
- Ablation TSV TIMEOUT rows carry zero conflicts — class analysis of
  unsolved cells needs standalone screens.
- kissat: `-s -q` mutually exclusive; progress-line conflicts is $10;
  --conflicts=1000 exits before its first eliminate (use ≥100k for
  inprocess profiling); drat-trim prints \r.
- Trajectory-identity for watcher/arena-order changes needs list-order
  evolution + tick parity + bump-order parity (inlinebin recipe); any change
  to resting clause-lit order rerolls armed cells (relevant to plan #1!).
- 2-arm gates only. oski-class standalone walls are load/thermal-sensitive;
  pair everything.
- Giants (>20M vars): every persistent workspace must be freed in the
  eliminate turn-off path before GC (ElsCsrWs joined that list in cc072b2).

## Instrumentation in-tree (use it)

- `SAT_TRACE_TIMING=1`: wall checkpoints (parse / frontend / root steps).
- `SAT_TRACE_ELIM=1`: eliminate decomposition incl. heap_build sub-timer.
- `SAT_DEBUG_CONGRUENCE=1`: dry-run + per-round + per-step closure timings,
  merge counts, round-0 "reusing dry-run plan" line.
- `SAT_DEBUG_ELS=1`: per-call ELS substitution/binaries/timing lines.
- `SAT_TRACE_PREPROCESS_DETAILS=1`: elim_round counters (CUMULATIVE — diff
  lines), vivify_yield_probe, unarmed_flywheel lines.
- `SAT_STATS_JSON=1`: full end-state stats (`c JSON_STATS` on stderr).
- Off-switch A/B knobs: `SAT_CLOSURE_DIET=off` (NEW — per-call allocating
  extraction/closure/ELS-CSR verbatim), `SAT_ROUND_DIET=off`,
  `SAT_ELIM_SCRATCH=off`, `SAT_CONGRUENCE_FASTIDX=off`,
  `SAT_ELIM_UNARMED_FLYWHEEL=on`, plus historical knobs in each promotion
  note.

## Where the evidence lives

- Newest session: `plan/next-steps-closurediet-2026-07-19.md`, gate
  `log/abtest-cand-vs-base-2026-07-19-20-53-42` + launch log
  `log/abtest-closurediet-launch.log`, commit cc072b2. Density-ensemble
  numbers also in bead `SAT-playground-5b2.3.50` notes (claim released).
- Prior arc: `next-steps-AGGREGATED-2026-07-19c.md` (superseded, valid as
  provenance) and the per-session notes it lists.
- **Baseline TSVs for the NEXT A/B**:
  `log/abtest-cand-vs-base-2026-07-19-20-53-42/cand/results.tsv` (68/100:
  rbsat IN at 73.5s margin, oski20 OUT — lineage 69 when oski20 lands).
- Bead: `SAT-playground-5b2.3.50` (open: global-effort cadence redesign
  scope remains; density scope resolved-negative).
