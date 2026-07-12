# Next steps after the worklist-congruence session (2026-07-12 evening)

Context for a fresh session. State as of this writing:

- Medium baseline: **62-63/100 @ 689f080 lineage** (this session's fresh base arm:
  62/100, conflicts 53,454,576, PAR-2 155,612; rbsat-v1375 timed out in BOTH arms —
  it remains the ±1 coin-flip cell). Kissat 4.0.4 reference: 74/100. Gap ≈ 11-12.
- NOT promoted this session. Two new env knobs landed **default-off (inert)**:
  `SAT_CONGRUENCE_WORKLIST` and `SAT_ELIM_ARMED_BOUNDS`. Default behavior is
  byte-identical to the promoted baseline.

## What was built (and validated)

**Worklist congruence closure** (`congruence.rs::find_merges_closure` + driver in
`try_congruence`, env `SAT_CONGRUENCE_WORKLIST`): kissat congruence.c parity —
union-find repr over literals + per-var gate occurrence lists + FIFO worklist;
merges cascade in memory instead of via the 64×O(formula) substitute→re-extract
rounds. Proof contract: merges are emitted in discovery order (each is RUP from the
gate clauses + earlier merge binaries); a cascaded self-negation is refuted AFTER
its supporting merges (the old path refutes before — do not swap the order back).
Validated: 602 tests (9 closure + 3 solver-level), smoke 9/9, crafted 8-layer miter
cascade proof drat-trim VERIFIED, and 61/61 in-gate candidate proofs verified.

- VexRiscv root closure: 63 rounds / 36.5s → **3 rounds / 2.4s (15x)**.
- Standalone conflicts@420s idle: VexRiscv 501k vs 400k (+25%), oski 316k vs 95k
  (3.3x), ibm-2004 SAT 191s vs 346s wall.

## Why it did NOT gate (A/B log/abtest-cand-vs-base-2026-07-12-15-19-53)

62==62 identical solved sets; both-solved conflicts **53,697,796 vs 53,454,576
(+243k) → LOSE**; PAR-2 better (155,355 vs 155,612). Only 4 of 100 cells diverged
(the ≥3000-dry-merge threshold protects the rest byte-identically): bp4_CSO_AM_IXA_LP
−103k, 931621d9 −42k, 374630 −1.7k, **ibm-2004 +390k — the entire loss**.

### The load-bearing insight: single-cell trajectory variance is the wall
ibm-2004 (SAT, 622k vars, 59% binary, congruence-armed) conflict history across
promotions: 1.49M → 390k (c579bfe) → 292k (689f080) → 681k (this worklist roll).
Its ±400k swings are ~3x larger than the −150k mechanism-level wins available per
A/B on the other edited cells. Any future candidate that touches armed formulas
rolls this die. Implication: **capability candidates should aim for a solved-count
flip (tier 1), not a conflicts-tier win (tier 2)** — or must leave SAT-armed cells'
trajectories bit-identical.

## Negative results this session (measured — do not re-run blind)

1. **SAT_ELIM_ARMED_BOUNDS v1 (grow 0→16 + clslim=100 on armed mid-search
   eliminate)**: VexRiscv ticks/prop 26 → 205 — clslim=100 resolvents poison the
   watch lists; conflicts 126k vs 400k @420s. clslim=100 is the poison, not the
   grow bound. Reworked to v2: clslim stays 20, kissat-proportional effort budget
   (10% of search ticks since last armed round, floor 50M). v2 screen (idle,
   worklist+v2 vs worklist-only idle rates): VexRiscv 2.59M conflicts/1200s
   (2157/s, 2.3x the base config's ~950/s; ticks/prop 13 — poison gone),
   oski 1.44M/1200s (+59%), ibm byte-identical to worklist-only (681,366
   conflicts — v2 adds no perturbation on it). Still NO standalone solves —
   throughput alone does not convert; the collapse bundle (next step 1) is the
   missing piece, which is why no second A/B was run.
2. **Binary DRAT proof output**: measured pointless. div-mitern172 writes a 369MB
   text proof for 2.5s of 146s wall (1.7%), conflicts identical with SAT_PROOF=off.
   Proof IO is not a lever on this suite. (Config stubs were written and reverted.)
3. **Worklist alone does not flip any gap cell standalone**: VexRiscv 2.43M
   conflicts/1750s no-solve, oski 1.44M no-solve. Raw conflict volume does not
   convert without mid-search formula COLLAPSE — kissat solves these by eliminating
   49-74% of vars mid-search (eliminate↔substitute↔congruence interplay), not by
   out-conflicting us.

## Ranked next steps

### 1. The full probe-round parity bundle (the real +1 play, multi-session)
Kissat's oski recipe: 20 mid-search rounds of congruence → substitute → vivify →
eliminate (with forward subsumption DURING elimination) → factor, collapsing 74% of
vars. We now have the cheap re-closure substrate (worklist) and the armed-bounds
skeleton. Missing pieces, in dependency order:
  a. **Substitution after mid-search closure** already exists (ELS) ✓
  b. **Forward subsumption during armed elimination** (kissat
     eliminate.c:kissat_forward_subsume_during_elimination) — keeps resolvents from
     bloating, which is what made even clslim=20 elimination yield ~nothing.
  c. **Mid-search factor** (fresh-var growth mid-search — resize var-indexed
     arrays; the known BVA follow-up from a402efd).
  d. Then elim-yield arming (`SAT_ELIM_PRODUCTIVE_MIN_PCT`, knob already in)
     becomes honest for the Timetable class too.
Gate strategy per the variance insight: promote only on a solved-count flip
(oski/VexRiscv/goldcrest/TT406 in-gate), not on conflicts.

### 2. Tagged-binary watchers (bead ck8 endgame) — parked with analysis
Kissat parity: tag bit + inline other-literal in the watcher, zero arena access for
binary edges. Real throughput on binary-heavy cells (ibm 59%, sudoku 51%), BUT
conflict-order parity is NOT free: eager binary detach (swap_remove) reorders watch
lists vs lazy compaction; skipping swap_clause_lits changes arena lit order that
conflict analysis iterates (bump order → heap tie-breaks). Without exact parity it
is another ibm-style coin flip; with parity discipline it loses half its savings.
If attempted: keep analysis-visible order (implied, false_lit), audit
clause_set_deleted choke point (3 sites: simp.rs:501, main.rs:3837, 7570/7595),
mask tag bits at all ~15 watcher.clause_idx sites, and validate conflicts-identical
on ≥5 cells before any A/B.

### 3. Housekeeping / traps (additions to the standing list)
- The 3000-merge dry-run threshold now has TWO counters: the closure counts
  distinct class-joins (deduped), the single-pass counts colliding gate pairs
  (with duplicates) — closure counts are LOWER on the same formula (vex dry:
  5.8k vs 7.7k). Borderline cells (DLTM ≈3.0k) can flip across the threshold
  between the two paths.
- SAT_LIMIT_WALL_SEC + SAT_STATS_JSON is the clean way to get end-of-run stats
  from screens (external `timeout` kills before JSON_STATS is emitted; stats go
  to STDERR).
- Background screens: launch via setsid + a status file; the harness Bash tool
  kills its process group at ~600s.

## Where the evidence lives
- A/B: `log/abtest-cand-vs-base-2026-07-12-15-19-53` (+ launch log
  `log/abtest-congrworklist-launch.log`), gate FAIL output in session log.
- Bead: `SAT-playground-2a7` comment dated this session (full numbers).
- Screens: scratchpad JSONs are gone after reboot; all numbers are in the bead
  comment and this note.
