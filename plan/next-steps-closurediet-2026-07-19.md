# Closure-diet promotion + density-ensemble negatives (2026-07-19 night session)

## Outcome

**PROMOTED: `SAT_CLOSURE_DIET` (default on) — wall-diet arc win #7.**
Gate `log/abtest-cand-vs-base-2026-07-19-20-53-42` (launch log
`log/abtest-closurediet-launch.log`), arms `cand:` vs
`base:SAT_CLOSURE_DIET=off`: **PASS, WIN — solved 68 vs 67 (rbsat-v1375
FLIP: cand SAT 1726.6s vs base TIMEOUT 1800s), both-solved conflicts EXACT
tie (delta 0 over all 100 cells), PAR-2 140,817.4 vs 143,575.2 (−2,757.9)**.
`check_promotion_gate` formal PASS; zero contradictions, zero correctness
failures. oski20 timed out in BOTH arms this gate (documented
contention-sensitive coin-flip cell; 69-lineage cell count unaffected as an
A/B).

In-gate wall margins banked (cand vs base): sted2 −136.5s (1564.1s),
Timetable-901fa −132.4s (1500.6s), oski40 −17.0s (1205.7s), rbsat-v1375
−73.5s (the flip). Every future gate inherits these.

## The change (solver/12-kissat-inprocessing)

One knob, two identity-safe components; `SAT_CLOSURE_DIET=off` replays the
shipped per-call allocating implementations verbatim (fair A/B arm):

1. **Congruence flat gate arena**: `extract_gates_for_congruence_flat`
   (verbatim transcription of `_fast` with the three gate-push sites emitting
   into one shared literal arena, `congruence::FlatGates`) +
   `find_merges_closure_flat` consuming it. Kills the 0.9–1.3M per-gate
   input-`Vec` allocations per armed congruence round. The closure table is
   a content-keyed **chained flat-entry vector** (`FxHashMap<u64 hash, head
   index>` with `next`-chains inside one `Vec<FlatKeyEntry>`; arena is
   append-only so older key windows stay valid). Identity argument: same
   gates in the same order, content-equality resolves hash collisions,
   representative-is-first-seen preserved; fuzz-verified against the legacy
   closure (800 random gate sets + cascade/UNSAT cases).
2. **ELS CSR workspace persistence**: `els::ElsCsrWs` (10 arrays) reused
   across `compute_representatives_csr_ws` calls (2/congruence round + root
   ELS); released in the giant turn-off path in `simp.rs` before GC
   (74eeaf0/cd8f1b5 precedent).

Measured basis (pre-change binary, SAT_DEBUG_CONGRUENCE): extraction+closure
= ibm 25s of 137s wall (20 rounds), oski40 12s of 872s, vex 4.1s (5 rounds,
round-0 dry-run reuse fires there). Clean paired ibm screen: cand 130.7s vs
off 134.7s (−3%), stats identical.

**TRAP (cost a redesign): the first flat table used
`FxHashMap<u64, Vec<Entry>>` — one bucket-Vec allocation per distinct key
(~1.2M) gave back everything the arena saved and measured as a small wall
REGRESSION (+0.5s/cell, identity screens). Per-key bucket containers defeat
arena diets; chain-in-flat-vector is the pattern.**

Identity evidence: 9-run screen (ibm full-SAT / vex @300k-conflict limit /
bubble @1.5M limit × cand / off / pre-change binary) — stripped stats JSON
IDENTICAL in every comparison. 659 unit tests (3 new fuzz-identity suites),
smoke 9/9 with drat-trim.

## Density-ensemble campaign: NEGATIVE, reverted (do not re-run blind)

Plan-19c item #3 attempted first (SAT_DENSITY_ENSEMBLE, scoped to
yield_search_armed): standalone ELS substitute passes after vivify+eliminate
(kissat probe.c parity), persistent sweep seed cursor (kissat sweep_schedule
parity), mid-search factor, proberounds=2. Full numbers in bead
`SAT-playground-5b2.3.50` notes. Headlines:

- The mechanisms WORK structurally: Bubble mid-search elimination deepened
  56%→69-72% (ELS burst 72 vars; bound escalated to 16 — the kissat shape),
  booth_wallace reached 79% eliminated.
- **They do not convert**: Bubble 15.5M conflicts @1810s without refuting
  (kissat 6.5M @295s); booth passed kissat's 12.1M-conflict refutation point
  at 14.1M without converting; fixedbandwidth BVE structurally blocked
  (35/149 vars eliminable). The density class is **conflict-rate/quality
  bound** (8.6k/s vs kissat 22k/s at equal collapse), not
  formula-rewriting-bound. The inprocessing-ensemble route to a density flip
  is DEAD this generation.
- Yield-armed reroll pairs (Pancake, QG7, oddball_24, sqrt-miters×2,
  div-miter, aaai10): v1 net +646k conflicts / +616s wall (mid-search factor
  densification the driver: QG7 +113k factor-only delta, Pancake +828k with
  220 fresh vars); v2 (ELS+cursor only) Pancake −93k, QG7 byte-identical,
  oddball −34k, but sqrt170 +175k (sign FLIP vs v1's −127k, which needed
  factor+cursor together). Net ≈ +48k: the armed-reroll casino, no reliable
  conflicts-tier win in this bundle space.

## Also measured this session

- **eliminate `heap_build` = 4ms/round, `other` ≈ 30ms/round on vex** (new
  sub-timer): plan-19c item 1c (kissat persistent elimination schedule)
  closes as measured-inert. Do not port.
- Congruence round-0 dry-run reuse (74eeaf0 component): fires on vex
  (saves ~3s/run), never on ibm/oski40 — confirmed nearly-free/nearly-inert
  outside vex.
- Kissat Bubble reference (this host, -v): UNSAT 295s, 6.53M conflicts,
  88% vars deactivated (2506 elim + 314 substitute + 168 congruent + 100
  sweep + 336 factor fresh), 434k vivified, 65k kitten sweep solves.

## Ranked next steps (updates plan-19c)

1. **Canonicalization + incremental gate extraction** (19c item #2, now the
   top play): sort clause lits for lit-order-insensitive extraction + the
   per-clause touched-var gate cache, one deliberate-reroll gate. The
   banked margins (sted2 1564s, rbsat 1726s, TT492, oski40 1206s) are the
   insurance.
2. **Density class**: ONLY via conflict-rate/quality work (propagation-rate
   on the collapsed DB, learned-clause quality). No more rewriting passes.
3. **TT406 stabilizer / pj2008 / bp4_TCO measurement** — unchanged from 19c.
4. Wall-diet arc: remaining chunks (congruence gates-Vec dealloc churn is
   now DONE; occ Vec-of-Vec in the closure + sweep snapshot clones remain,
   but sizes suggest <2s/cell — the arc is near its documented end).

## Standing traps (additions)

- Per-key bucket containers (HashMap<hash, Vec<...>>) silently reintroduce
  the alloc churn an arena removes — chain through one flat Vec instead.
- Agent-harness compound `( a & b & ); wait` loses the second subshell
  silently; run paired screens as separate background tasks.
- pgrep self-match again (watchers matching their own cmdline): bracket
  patterns [n] do NOT help when the monitored string is in ANOTHER monitor's
  cmdline. Kill watchers by task id before check_promotion_gate.

## Where the evidence lives

- Gate: `log/abtest-cand-vs-base-2026-07-19-20-53-42` + launch log
  `log/abtest-closurediet-launch.log`; formal gate PASS output in session
  log.
- Screens/measurements: scratchpad (dies on reboot); all decision-relevant
  numbers preserved above and in bead `SAT-playground-5b2.3.50`.
- Baseline TSVs for the NEXT A/B: this gate's
  `cand/results.tsv` (68/100 with rbsat in, oski20 out).
