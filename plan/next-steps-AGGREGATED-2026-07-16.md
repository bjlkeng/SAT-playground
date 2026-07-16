# Aggregated next-steps notes (all sessions, newest first) — 2026-07-16

Single-file aggregation of every `plan/next-steps-*.md` note for a clean-context
session. NEWEST FIRST: later notes supersede earlier ones where they conflict
(baselines, verdicts on mechanisms, knob defaults). Read the first note for
current state; older notes are provenance, negative-result ledgers, and traps.

Current state in one line: medium baseline **68/100** (rbsat coin-flip landed in;
67 lineage + zero losses) at commit `5c21230`; kissat 4.0.4 reference 74/100;
gap cells by class: density x4 (Bubble, booth_wallace, booth_dadda,
fixedbandwidth), BMC x3 (oski20, g2, goldcrest), structured-SAT x3 (TT492,
bp4_TCO, lockchart-group1), giant x1 (pj2008), coin-flip x1 (rbsat).

Promotion ledger (newest first): 6633bc7 binary-edge tag (trajectory-identical
PAR-2 win) | e7d149a elim-def groundwork default-OFF + kitten core soundness fix
| 075b7e8 SAT_DECISION_ARM=24 (67/100) | 038f9c1 binary DRAT | 2f92794
vivify-yield arming | 3683ab5 vivify ALE (+vex +oski40, 67/100) | e5bd1f9 armed
collapse bundle | 906e7cc giant-arena parse | 15911aa simp-aware preflight |
689f080 chrono-productive | c579bfe congruence inprocess.

## Index
1. next-steps-elimdef-bintag-2026-07-16.md
2. next-steps-decision-arm-2026-07-15.md
3. next-steps-bindrat-2026-07-15.md
4. next-steps-vivify-yield-2026-07-14.md
5. next-steps-vivify-ale-2026-07-14.md
6. next-steps-armed-collapse-2026-07-13.md
7. next-steps-giant-arena-2026-07-13.md
8. next-steps-preflight-factor-2026-07-13.md
9. next-steps-worklist-congruence-2026-07-12.md
10. next-steps-chrono-productive-2026-07-12.md
11. next-steps-congruence-gap-2026-07-12.md


==============================================================================
### SOURCE: plan/next-steps-elimdef-bintag-2026-07-16.md
==============================================================================

# Next steps after elim-def groundwork + binary-edge-tag promotion (2026-07-16)

Context for a fresh session. State as of this writing:

- Medium baseline: **68/100** in this session's final gate (rbsat-v1375, the
  documented ±1 coin-flip cell, landed IN for BOTH arms — strict superset of
  the 67/100 lineage, zero losses; treat the 68th as coin-flip until it
  repeats). Kissat 4.0.4 reference: 74/100
  (`log/kissat-medium-20260705-203444`). Gap ≈ 6-7.
- **PROMOTED 6633bc7: SAT_BINARY_EDGE_TAG default on** — binary-edge deleted
  tag. The binary hot loop no longer does a random 48-byte `BinaryClause` load
  per edge (deleted flag = tag bit 1<<31 in `BinaryEdge::clause_id`,
  maintained at BOTH `deleted = true` sites: `mark_binary_clause_deleted_for_
  clause` and the GC NO_RELOC drop) and no longer writes dead usage metadata
  (`used_count`/`last_used_conflict` — no functional readers; analysis-side
  marking kept). Off-switch `SAT_BINARY_EDGE_TAG=off` = byte-for-byte legacy
  path.
  Gate `log/abtest-cand-vs-base-2026-07-16-02-17-11`: PASS, WIN — 68==68
  solved, both-solved conflicts IDENTICAL 65,324,524 on every pair (the
  trajectory-neutrality proof held across all 100 cells), PAR-2 140,997.7 vs
  141,058.4. Idle paired walls: ibm −5%, oski40 −4.6%, vex −0.4%.
- **COMMITTED e7d149a (default-OFF groundwork): SAT_ELIM_DEF** — kitten-based
  semantic definition extraction in armed BVE (kissat definition.c port) +
  budgeted kitten (`solve_budgeted`) + a **kitten clausal-core soundness fix**
  (compute_core now expands learned clauses through recorded derivation
  antecedents; the old current-reasons-only walk produced non-refuting cores).
  Sweep is unaffected (consumes proof_lemmas, not cores), but ANY future
  core consumer needs this fix. Gate `log/abtest-cand-vs-base-2026-07-15-20-
  35-28`: LOSE 66 vs 67 → stays default-off.

## The elim-def story (measured; do not re-run blind)

Definition extraction WORKS mechanically: oski20's 3.5GB proof with 2,218
definition eliminations is drat-trim VERIFIED; oski20 solved standalone in
every def variant (1254-1561s) while its paired base TIMED OUT (>1750s).
The conflicts tier would have won (−164k both-solved; bp4 −158k, DLTM −148k,
sqrt-mitern170 −118k, Pancake −97k; ibm +366k roll). It is NOT promotable
because:

1. **oski40 is the counter-cell**: base solves ~989s idle; every def variant
   is slower (1358s best) and lost it in-gate. Root cause measured, twice:
   - First: re-check wall (947k kitten checks/run — fixed by the per-var
     occurrence memo, checks → 186k, kitten ticks → 2M total = negligible).
   - Then: **densification** — definition resolvents doubled the live arena,
     tripled learned-clause literals, 5.6x search ticks (+700s wall for −14%
     conflicts). A per-resolvent parent-length cap kills the bloat but also
     the yield (1,643 → 231 eliminations), and even 231 eliminations roll
     oski40's trajectory +589k conflicts (ibm-class variance).
2. The armed scope cannot hold oski20 and oski40 simultaneously — same
   family, opposite responses. This is the single-cell-variance wall from the
   worklist note, again.
3. Density class: definitions FOUND at 99% of checks (Bubble 19,245 found)
   but almost none convert under the resolvent bound (307 eliminated); no
   flip. TT492/lockchart: 0-found class (protected by the 20k-check adaptive
   cutoff). vex: 0-found, byte-identical (protected).

Salvage angles if revisited: kissat's `definitioncores` core REFINEMENT
(shuffle + re-solve to shrink cores → smaller gates → resolvents fit the
bound → conversion rate up — the main structural difference left vs kissat),
forward subsumption of resolvents during elimination, and mid-search factor
to recompress after definition collapse. All three attack the densification
directly instead of capping it.

## Session traps (additions)

- Kitten cores were WRONG for 6 weeks (sweep never consumed them) — when a
  new consumer reads sub-solver output, validate the output itself first
  (the wrong-SAT screens cost a full A/B).
- `SAT_STATS_HOT=1` + SAT_STATS_JSON produced truncated .err JSON in screens
  (only a factor line); the plain-JSON screens were fine. Not debugged.
- `pkill -f 'pat[t]ern'` still self-matches if the LITERAL bracket pattern
  appears in your own command line's launch args — kill by PID instead.
- vex ignored SAT_LIMIT_WALL_SEC=240 for 8+ min (wall checked between
  conflicts only; long parse + inprocess rounds) — known, but it also holds
  for SHORT wall probes.
- feature_ablation keeps only results.tsv per arm; per-cell JSONs live in the
  tmp dir and are cleaned — extract per-cell stats DURING the run or re-screen.

## Ranked next steps

### 1. lockchart-group1 (kissat 1336s SAT — profile NOW CAPTURED, first time)
`kissat -s` profile (this session): 396k conflicts, 39 dec/conf, 4.43G props
@ 3.3M props/s, eliminated 16%, factored 1,531, 18 rephases, congruence 0.
It is a raw-propagation cell (11k props/conflict); our 1750s screens reach
only ~265k conflicts. The wire needs either ~2x propagation throughput on
binary/long mixed scans (CSR watcher endgame, bead ck8, parity analysis in
the worklist note) or the rephase schedule finding the model earlier (our
walk/rephase machinery exists; lockchart decision-arms at 36.2 — check
whether rephases actually fire there and what best_permille reaches).

### 2. More trajectory-identical wall diet (the bintag pattern, repeatable)
The gate cannot lose on these (identical trajectories) and the wire cells
bank every second. Next candidates from the chrono-productive note's list:
watch-list blocker-hit locality, `c->searched`-style replacement caching
(irrelevant for BMC but cheap), arena prefetch tuning (exists: bead 5b2.8.1),
and dropping the dead `binary_dedup_seen` allocation everywhere (0.43GB on
giants — byte-identical trajectories, less RSS). Bundle several, prove
conflicts-identical on 5 cells, gate once.

### 3. oski20 margin (solves 1254-1561s standalone w/ def; 1430-1500s w/o)
Any further suite-wide speedup may flip it in-gate even without elim_def.
It sits with vex/rbsat/sted2 in the wire-cell set that motivates play #2.

### 4. Density class: elim-def core refinement (the honest continuation)
See salvage angles above — refinement is what kissat actually does that we
skipped, and the conversion-rate numbers (99% found / 1.6% converted) say
the cores are too big, exactly what refinement fixes.

## Where the evidence lives

- Bintag gate: `log/abtest-cand-vs-base-2026-07-16-02-17-11` + launch log
  `log/abtest-bintag-launch.log`; formal check output in the 6633bc7 commit.
- Elim-def gate (LOSE, documented): `log/abtest-cand-vs-base-2026-07-15-20-
  35-28` + `log/abtest-elimdef-launch.log`.
- Kissat lockchart profile: scratchpad `screens/kissat-lockchart.out` (dies on
  reboot — key numbers preserved above).
- Bead: `SAT-playground-2a7` (running gap-analysis log).


==============================================================================
### SOURCE: plan/next-steps-decision-arm-2026-07-15.md
==============================================================================

# Next steps after the decision-arm promotion (2026-07-15)

Context for a fresh session. State as of this writing:

- Medium baseline: **67/100** (gate: 67 vs 65 — base arm lost rbsat AND sted2
  to wall noise that run; sted2 landed IN for the candidate). Kissat 4.0.4
  reference: 74/100 (`log/kissat-medium-20260705-203444`). Gap ≈ 7.
- **PROMOTED 075b7e8: SAT_DECISION_ARM=24** (default on). At the vivify-yield
  probe points (200k conflicts, 4x spacing), a formula with cumulative
  decisions/conflict >= 24 and !deep_phase arms: the aggressive-inprocess
  bundle + **mid-search factor** (SAT_FACTOR_INPROCESS scope, still default-off
  globally) + the **kissat rephase/walk schedule** (rephase machinery activated
  at the arming point; `decision_search_armed` passes the armed-only rephase
  gate). Off-switch: `SAT_DECISION_ARM=0` (byte-identical shipped baseline).
- Gate: `log/abtest-decarm-vs-base-2026-07-15-08-15-37` + launch log
  `log/abtest-decarm-launch.log` — **PASS, WIN 67 vs 65**, zero contradictions,
  zero correctness failures. Cand-only flips: **TT406 SAT 246s verify=ok**
  (engineered flip, first ever), sted2 1705s (wall-noise class). No base-only
  cells. 64/65 both-solved pairs byte-identical; single divergence
  SC25_Timetable_C_395 (armed, −71,271 conflicts, still solves). sqrt-mitern170
  + VexRiscv checker-timeout symmetric both arms (benign multi-GB-proof class).

## The mechanism (why TT406 flipped)

Proven on BOTH solvers:

- Kissat solves TT406 in 32-41s via 4 mid-search eliminations to 67% of vars +
  15,151 factored vars + 13 rephases / 11.7M walk steps. **Kissat itself TIMES
  OUT (300s) on TT406 with `--rephase=0` OR `--eliminatebound=0`**;
  `--definitions=0` / `--factor=0` are only ~4x slowdowns (32s -> 120s).
- Our attribution (standalone, forced knobs): armed collapse alone
  (pct40+factor: 18.9k fresh vars, 190k product clauses removed, elim
  108k->119.5k of ~216k vars) churns 1.3M conflicts and does NOT solve;
  collapse + rephase/walk solves ~305s (walk_improved=3 — the walker finds the
  model on the compressed formula). pct40 + rephase WITHOUT factor: TIMEOUT
  at 1750s — **factor and rephase/walk are BOTH load-bearing**.
- Implemented default (arming at 200k conflicts, not root): TT406 standalone
  **199s**, 668k conflicts, factor 18,850 fresh vars, 14 rephases / 5 walks.

## Threshold calibration table (dec/conf at the 200k-conflict probe, measured)

Arms: TT406 45.2, TT492 49.7, lockchart-group1 36.2.
Refuses (all solved/fragile cells): sudoku-N30 10.4, goldcrest 10.7, velev 8.0,
reconf10_68 8.1, oddball_80 6.8-7.5, 59-129706 6.7, mp1 5.8, Kakuro 5.4,
lockchart-g2 4.1, case1 2.6, jkkk 2.2, bp4_TCO 2.1, rbsat-v1375 1.9,
544707/case9/VanDerWaerden 1.3, DLTM 1.8. Threshold 24 has ~2x margin both
ways. NOTE: TT406's ratio DECAYS with conflicts (45.2 @200k -> 23.0 @800k) —
first-probe arming is load-bearing; do not move the rule to probe >= 2.

Identity checks: mp1 336,333 and 544707 241,644 conflicts EXACTLY equal
on/off. sudoku-N30 solves UNSAT 771s standalone unarmored.

## Measured this session (do not re-run blind)

1. **SAT_ELIM_PRODUCTIVE_MIN_PCT=40 stays dead** (re-confirmed under the
   current bundle): mp1 derails 27s -> 600s+ TIMEOUT; the signal cannot
   separate TT406 (49.9% root elim) from mp1 (48.4%).
2. **TT492/495/496 do NOT flip** with the decision-arm bundle (1750s
   standalone TIMEOUT, both root-armed combo and implemented probe-armed).
   TT492 arms and runs the full machinery — the collapse+walk is not enough
   there (kissat needs 1052s itself for 492; 495/496 kissat can't solve).
3. **Density class (Bubble/booth/fixedbandwidth) has NO single missing
   mechanism**: kissat ablations on Bubble ALL still solve — definitions=0
   277s, factor=0 290s, eliminatebound=0 360s, vivify=0 424s (baseline 321s).
   The kissat win there is multi-mechanism compounding. Stop looking for a
   single port; the play is pipeline-level (see next steps).
4. **Mid-search factor under yield-arming is nearly inert on Bubble**: 252
   fresh vars vs kissat's ~10% of vars (18.8k on TT406). Whatever gates factor
   candidates on density cells, it is not the decision-armed bound.
5. Kissat TT406 profile cached: 32s, 170k conflicts, 10M decisions (58.8
   dec/conf), eliminated 67% (4 passes), factored 15.2k (7%), 37 backbone
   computations, 13 rephases, 11.7M walk steps, 0 congruence.

## Ranked next steps

### 1. lockchart-group1 (SAT, kissat 1687s; ours TIMEOUT, NOW ARMS at 36.2)
The only remaining gap cell that decision-arms. It didn't flip in this gate —
screen it standalone with trace to see what the armed bundle does there
(does factor fire? does the walker get close — check walk_improved /
best_permille?). A Timetable-style collapse may need different factor
candidates (lockchart is a big SAT scheduling instance). Cheapest possible
next +1.

### 2. oski20 margin (BMC cascade; solves 1481-1659s standalone, TIMEOUT in-gate)
Unchanged from the bindrat note: needs ~150-300s of suite-wide margin or a
targeted speedup. Any decision-arm-adjacent wall saving does not help here
(oski20 dec/conf is low; it never arms). The congruence-merge stall
(18.4k frozen on vex-class) and elim depth on armed cells remain the levers.

### 3. Density class: probe-pipeline interleave (the compounding play)
With single mechanisms exhausted (bindrat note) and kissat's Bubble surviving
every single ablation (this session), the remaining honest approach is the
kissat probe.c ORDER on armed cells: congruence -> substitute -> backbone ->
vivify -> sweep -> substitute -> transitive -> backbone -> factor, per round,
with per-pass effort budgets tied to search ticks — vs our current fixed
congruence -> vivify -> sweep -> BVE -> factor. This is an architecture
session: per-pass SET_EFFORT_LIMIT parity + a round driver that re-runs
passes while the active-var count drops.

### 4. TT492 depth (kissat 1052s — the hardest solvable Timetable)
Kissat needs 25x TT406's time; our armed bundle gets the collapse but not the
model. Options: warmup (kissat warms 100% of walks via
propagate-beyond-conflicts — NOT ported; TT406 didn't need it, TT492 might),
walk effort scaling on armed cells (SAT_WALK_EFFORT), kitten definitions in
armed BVE (elim depth 55% vs kissat 67% on TT-class).

### 5. Watch cells for regressions in future gates
sted2 is now a SOLVED cell at 1705s (12s inside the wire after losing its old
margin) — it remains the thinnest cell in the suite and is deep-phase-guarded
from all arming; treat any sted2 loss as wall noise, not mechanism, unless
its conflict count changes. C_395 now arms (improved this gate): its
trajectory is coupled to the bundle from here on.

## Housekeeping / traps (additions this session)

- `kissat -s` and `-q` are mutually exclusive — use `-s` only.
- The decision-arm rule shares the yield-probe cadence and guards:
  `SAT_VIVIFY_YIELD_ARM=0` (full-replay off-switch) also disables decision-arm
  probes; the >10M-live-clause cap excludes giants from decision arming.
- `pgrep`-based monitor shells trip check_promotion_gate's
  running_solver_processes FAIL (false positive) — TaskStop monitors before
  the gate check, then re-run (PASS this session after doing exactly that).
- The A/B preflight RAM warning (32x16GB > 90% of 512GB) is expected with the
  standard AGENTS.md config on this host; jobs never all peak simultaneously.
- dcg blocks shell pipelines whose sed replacement text contains `->`
  (misparsed as a redirect) — write awk/basename instead.

## Where the evidence lives

- Winning gate: `log/abtest-decarm-vs-base-2026-07-15-08-15-37` (+ launch log
  `log/abtest-decarm-launch.log`); gate PASS output in session log and the
  075b7e8 commit message.
- Kissat ablations, forced-knob attributions, dec/conf calibration, identity
  checks: scratchpad (dies on reboot) — all decision-relevant numbers are in
  this note, the commit message, and bead 2a7's comment log.
- Bead: `SAT-playground-2a7` (running gap-analysis log).


==============================================================================
### SOURCE: plan/next-steps-bindrat-2026-07-15.md
==============================================================================

# Next steps after the binary-DRAT promotion (2026-07-15)

Context for a fresh session. State as of this writing:

- Medium baseline: 66-67/100 (66 in this gate; rbsat-v1375 the known ±1
  coin-flip landed out at 1789s-vs-1800s, sted2 landed IN — see below).
  Kissat 4.0.4 reference: 74/100.
- **PROMOTED: SAT_PROOF default = binary DRAT** (drat-trim wire format).
  Gate: `log/abtest-bindrat-vs-base-2026-07-15-01-07-48` — **PASS, WIN**:
  solved 66==66, both-solved conflicts IDENTICAL on every pair (trajectory
  neutrality held across all 100 cells), PAR-2 146,409.6 vs 146,492.7.
  Solved-set swap, both wall-noise-class: candidate gained sted2_0x1e3-216
  (SAT 1746s; base TIMEOUT — cheaper temp-proof streaming pulled it under the
  wire) and lost rbsat-v1375 (base 1789s, 11s from the wire, byte-identical
  6.26M-conflict trajectory; the documented coin-flip cell). Verification: 64
  verify=ok / 2 benign checker-timeouts / 34 skip — identical in both arms;
  drat-trim auto-detected binary at scale, zero harness changes.
  Off-switch: SAT_PROOF=drat.
- This session implemented three further kissat-parity capabilities and
  screened them standalone before gating; all are default-off groundwork
  (measured non-winners as scoped — details below).

## Shipped to the A/B: SAT_PROOF=binary (binary DRAT)

drat-trim wire format ('a'/'d' tag byte, 7-bit varint literals with
2*var+sign mapping, 0x00 terminator; final empty clause = `a 0x00`).
Trajectory-neutral by construction — proof bytes never feed back into search;
all 5 smoke UNSAT proofs drat-trim VERIFIED (auto-detected binary mode, no
harness changes needed). Cuts proof bytes ~2.5-3x and removes the ASCII
formatting cost.

Measured (paired, idle host): oski20 writes 7.34GB text DRAT; proof-off saves
~50s of 1515s. Binary recovers roughly half to two-thirds of that per cell at
idle; the in-gate hope is bigger (32-way I/O contention) and lands on every
multi-GB UNSAT cell. Near-wire cells that could flip on margin: vex (1720s
in-gate, ~8GB proof), rbsat-v1375 (1738s coin-flip), oski20 (timed out
in-gate at 1751s; solves 1481-1659s standalone).

## Default-off groundwork (measured, do not re-screen blind)

1. **SAT_REPHASE + SAT_WALK** — full kissat rephase.c/walk.c parity port:
   6-slot schedule (best, walk, inverted, best, walk, original), NLOG3N
   growing interval, saved->target copy semantics, best_assigned reset on 'B',
   ProbSAT walker (fitted CB on odd walks, bounded flip trail, effort =
   SAT_WALK_EFFORT permille of search ticks since last walk, floor 10M).
   Scoped to yield-armed (SAT_REPHASE_ARMED_ONLY, default on) after screens:
   - Global rephase was already known toxic (2026-07-05: 43 vs 53 solved).
   - Armed-wide scope: ibm-2004 (congruence-armed SAT canary) +46% conflicts
     (347k -> 505k). Yield-armed scope: ibm byte-identical (346,627).
   - Yield-armed screens: Bubble 17.7M vs 17.4M conflicts @1790s (no flip),
     booth_wallace 15.4M vs 15.9M (no flip), fixedband 36.7M vs 37.8M (no
     flip), QG7 +3.2% conflicts (2.03M vs 1.97M, both solve). Verdict:
     phase-resetting is NOT the density-class bottleneck. Machinery is clean
     and tested (628 unit tests incl. walker model-finding) for future
     composition (e.g. with warmup — kissat warms 100% of walks via
     propagate-beyond-conflicts; not ported).
2. **SAT_VIVIFY_DEDUCE** — kissat vivify.c vivify_deduce parity: on a
   conflicting assumption walk, analyze the reason cone and strengthen to the
   assumptions actually needed (not the full prefix); on an implied-TRUE walk,
   strengthen to {implied} + cone (previously no edit). Armed-scoped like ALE.
   Proof sound end-to-end: QG7 598MB deduce proof `s VERIFIED` (852s,
   21 RAT lemmas are factor's, expected). Measured: it DOUBLES the
   strengthening rate everywhere (QG7 checks 9.5%->21%; oski20 238k->449k
   strengthenings; Bubble 108k->198k) and it still LOSES:
   - ibm canary: 347k -> 808k conflicts (+133%, SAT trajectory derailed).
   - oski20: conflicts identical (2.659M vs 2.664M), wall +146s (edit cost).
   - Bubble: no flip, 18.2M vs 17.7M conflicts.
   - QG7: +3.2% conflicts.
   Verdict: strengthening QUANTITY is not the conflicts-to-refutation gap.
   Kissat's 54% vivify hit rate coexists with its wins for other reasons
   (probably elimination depth 72% vs our 54% on Bubble, and/or reduce/tier
   retention interactions).
3. **SAT_RESTART_ARMED_FLOOR scope extension** — the armed restart knobs now
   also fire on yield-armed formulas (dedicated `restart_floor_armed` flag;
   default 0/off = inert). Measured on fixedband: floor=1 + margin=1.10 gives
   4.4x restarts (288k vs 65k, kissat-parity cadence) and STILL no flip
   (35.1M conflicts @1790s vs kissat's 12.1M refutation). Restart cadence is
   NOT the density-class bottleneck either.

## Killed this session without an A/B (measure-first wins)

- **Binary-clause backbone port (kissat backbone.c)**: profiled kissat -s on
  Bubble and fixedbandwidth — backbone_ticks 810k/53k, no units, 0.01s spent
  across 113-121 "computations". The plan-note ranking overvalued it;
  computations != yield. Do not build.
- **oski20 via deduce/proof-IO alone**: needs ~150-300s; deduce is +146s
  (worse), proof-off is only -50s idle.

## The remaining honest gap analysis (density class)

Bubble numbers, kissat vs us (this session's paired runs): conflicts to
refutation 6.5M vs 17.4M+ (timeout); restarts/conflict 1/40 vs 1/560 (fixed:
still no flip); vivify hit rate 54% vs 18% (fixed via deduce: still no flip);
rephases/walks 49/16 vs 0 (fixed: still no flip); eliminated vars 72% vs
~54%; substituted 9% vs ~0 (our ELS/congruence find nothing there);
factored 10% vs 0 mid-search. The single-mechanism ports are exhausted —
what remains is the elimination/substitution depth delta and true
multi-mechanism compounding (kissat's probe pipeline interleaves
congruence -> substitute -> backbone -> vivify -> sweep -> substitute ->
transitive -> backbone -> factor per round; ours runs a subset).
Next candidates in order: (a) why armed BVE stalls at ~54% on Bubble
(occurrence/bound limits? gate extraction finding nothing post-strengthen?),
(b) mid-search factor on yield-armed cells (SAT_FACTOR_INPROCESS exists,
default off), (c) the full probe-pipeline interleave order.

## Housekeeping / traps (additions)

- drat-trim prints with \r; `grep -c "^s VERIFIED"` fails — match without
  the anchor and check "NOT VERIFIED" first.
- Background shells reset cwd between invocations — absolute paths for
  long-running screens.
- oski20 standalone wall is load/thermal-sensitive: 1659s (07-14 morning),
  1481-1515s (this session, byte-identical 2,663,684-conflict trajectory).
  Pair everything.


==============================================================================
### SOURCE: plan/next-steps-vivify-yield-2026-07-14.md
==============================================================================

# Next steps after the vivify-yield-arming promotion (2026-07-14 evening)

Context for a fresh session. State as of this writing:

- Medium baseline: **66-67/100** (66 in this session's gate — rbsat-v1375, the
  known ±1 coin-flip cell, timed out in BOTH arms; it solved at 1738s in the
  morning gate). Both-solved conflicts **58,469,094**, PAR-2 146,767.7.
  Kissat 4.0.4 reference: 74/100 (`log/kissat-medium-20260705-203444`).
- Promoted (default-on): **SAT_VIVIFY_YIELD_ARM=170** — an EDIT-FREE dry-run
  probe of learned-clause vivification yield that arms `inprocess_aggressive`
  on conflict-dense formulas the congruence/elim signals miss
  (booth/Bubble/fixedbandwidth class: 0 congruence merges, whole armed bundle
  previously inert there). Off-switch: `SAT_VIVIFY_YIELD_ARM=0` (byte-identical
  shipped baseline).
- Gate evidence: `log/abtest-cand-vs-base-2026-07-14-18-24-40` (PASS, WIN,
  launch log `log/abtest-vivifyyield-launch.log`): 66==66 identical solved
  sets, both-solved conflicts 58,469,094 vs 59,450,839 (**−981,745, −1.65%**),
  zero contradictions, zero correctness failures. 83+ pairs byte-identical;
  only 7 cells diverged (all armed UNSAT, all still solve):
  Pancake −625k, QG7 −221k, oddball_24 −185k, sqrt-mitern170 −155k,
  sqrt-mitern171 −52k, div-mitern172 −13k, **aaai10 +270k (the one regression,
  priced in)**. PAR-2 +758 (armed cells pay vivify wall) — conflicts tier
  decides per the metric.

## The mechanism

Probe = the `vivify_round` analysis walk (learned tier1/tier2 candidates only,
ALE counting) inside the temporary-assumption clone, replaying NOTHING — the
restore discards every would-be edit, so sub-threshold formulas keep
byte-identical trajectories (proved in-gate: MVRoundRobin probes at yield 135
and is conflict-identical). Composite arming rule (all four required), first
probe at 200k conflicts, 4x spacing, max 3 probes:

1. **yield ≥ 170‰** of analyzed candidates would be edited. Alone it does NOT
   separate (Pancake 390 > Bubble 370; SAT cells 544707/59-129706 at 384/352).
2. **decisions/conflict ≤ 3** — refutation-churn signature. Density targets sit
   at 1.3-1.6; SAT cells making progress sit higher (mp1 5.8, 59-129706 7.3,
   Timetables 45-50).
3. **!deep_phase** (same guard as sweep) — sted2 excluded at 966‰ best-phase.
4. **2nd+ probe only (≥800k conflicts)** — every measured fragile solved-SAT
   cell (544707 241k, mp1 336k, case9 431k, case1 748k, velev 782k conflicts)
   finishes before the second probe fires; protected by construction.

Arming = the existing `inprocess_aggressive` bundle: 10k-doubling cadence,
per-round learned vivify + ALE + 300M tick budget, armed BVE with bound
escalation. Chrono-delta and restart knobs stay congruence-scoped (untouched).

## The load-bearing discovery (redirects the density campaign)

The density class is NOT conflict-rate-limited. Baseline screens (idle,
scratchpad, numbers preserved here): Bubble 15.9M conflicts in 1750s @9.1k/s
(kissat refutes at 6.5M), booth_wallace 16.4M (kissat 12.1M), booth_dadda
16.7M, fixedbandwidth 40.5M (kissat 12.1M) — all at 1.3-1.4 decisions/conflict,
same regime as kissat. The gap is **conflicts-to-refutation** (learned-clause
quality / proof progress). Kissat's edge on exactly these cells: continuous
learned vivification (39-55% of checks strengthened, 434-875k vivified/cell),
mid-search elimination (72-77% of vars, 22-26 eliminations), 113-155 backbone
computations, ~50-65 rephases + 16-22 walks. Armed vivify recovered part of it
(targets: Bubble 15.2M, booth_wallace 13.7M, booth_dadda 14.2M, fixedbandwidth
37.2M in the same wall — fewer conflicts, no flip). The remaining mechanisms to
try for the actual flips, in kissat-evidence order: **binary-clause backbone**
(kissat runs it 113-185x on these cells), **rephasing/walk** (49-65 rephases;
ours is off), reduce-policy retention, transitive reduction.

## Also learned this session (do not re-measure blind)

1. **oski20 solves STANDALONE for the first time** (UNSAT 1659s idle, 2.66M
   conflicts, 65,338 merges) with current defaults. Still over the ~1000s
   in-gate line, and TIMEOUT in this gate — but any suite-wide speedup may
   flip it. Kissat: 575s idle.
2. TT406/TT492 probe at decisions/conflict 45-50 — vivify-yield arming
   correctly refuses them. Their mechanism remains real mid-search elimination
   + factor (kissat TT406: 32s, 170k conflicts, 4 eliminations to 67% vars,
   15k factored, 13 rephases). SAT_ELIM_PRODUCTIVE_MIN_PCT stays dead (see
   2026-07-14 morning note).
3. Kissat gap-cell profiles cached (this session, idle host): TT406
   32s/170k conf; Bubble 321s/6.5M @20.3k/s, 1.75 dec/conf, 101 props/conf;
   fixedbandwidth 494s/12.1M @24.6k/s, 13.6 props/conf (pure throughput, near-
   zero inprocessing value); booth_wallace 1170s/12.1M @10.4k/s; oski20
   575s/4.75M, 74% eliminated, 139k congruent matched, 92 vivifications.
4. Remaining gap cells by class: BMC cascade (oski20 near-line, g2 0 merges —
   gate extraction finds nothing, worth investigating WHY; goldcrest 7.8k
   props/conf = propagation-bound), structured SAT (TT406/TT492/lockchart-g1,
   bp4_TCO dec/conf 1.6), giants (pj2008 22.7k props/conf, 9k conflicts
   total — pure propagation, likely needs the CSR watcher rewrite).
5. Probe-yield calibration table (19 cells, scratchpad calib/calib2, gone
   after reboot — key rows preserved in "The mechanism" above and the commit
   message).
6. The A/B preflight's `running_solver_processes_detected` FAIL from your own
   monitor shells: stop the monitors (TaskStop), re-run the gate — standard.

## Where the evidence lives

- Gate: `log/abtest-cand-vs-base-2026-07-14-18-24-40` + launch log
  `log/abtest-vivifyyield-launch.log`; gate PASS output in session log.
- Baseline 12-cell screens, kissat profiles, calibration probes, armed
  screens: scratchpad (dies on reboot); all decision-relevant numbers are in
  this note and the commit message.


==============================================================================
### SOURCE: plan/next-steps-vivify-ale-2026-07-14.md
==============================================================================

# Next steps after the vivify-ALE promotion (2026-07-14, this commit)

Context for a fresh session. State as of this writing:

- Medium baseline: **67/100** (was 64 @ e5bd1f9), both-solved conflicts
  53,963,337, PAR-2 144,705.7. Kissat 4.0.4 reference: 74/100
  (`log/kissat-medium-20260705-203444`). Gap ≈ 7.
- Promoted (default-on):
  - **SAT_VIVIFY_ALE** — asymmetric literal elimination in `vivify_round`:
    strengthen a candidate even when the negated-prefix assumption walk ends
    WITHOUT a conflict, dropping the literals implied FALSE along the way
    (kissat vivify.c parity). Scoped in code to ARMED (`inprocess_aggressive`)
    formulas — unscoped ALE measurably rolled two non-armed solved SAT cells
    (sted2_0x1e3-216, 59-129706) into timeouts via the originals-schedule
    vivify rounds (first A/B, LOSE 65 vs 66,
    `log/abtest-cand-vs-base-2026-07-14-11-02-34`).
  - **SAT_VIVIFY_ARMED_TICKS=300000000** — armed-only per-round vivify budget
    replacing the permille clamp (cap was 100M). With ALE raising per-attempt
    yield, the cap was binding on the BMC cascade cells.
  - **Congruence worklist XOR-cancellation proof fix** (congruence.rs) — see
    "The correctness bug" below. Trajectory-neutral; REQUIRED for any promotion
    that makes oski-class cells solve.
- Gate evidence: `log/abtest-cand-vs-base-2026-07-14-14-02-51` (PASS, WIN
  **67 vs 64**, zero correctness failures; launch log
  `log/abtest-vivifyale2-launch.log`). Cand-only flips, no base-only cells:
  - **VexRiscv UNSAT 1720s** (first ever in-gate; kissat 232s) — flipped in
    BOTH A/Bs run today (1661s in the first).
  - **oski15a01b40s UNSAT 1380s, verify=ok** (first ever; kissat 543s).
  - rbsat-v1375 SAT 1739s (the known ±1 coin-flip cell; byte-identical 6.26M
    conflicts in every run, pure wall noise — do not attribute).
  - Divergent both-solved cells only 3: ibm −23k, bp4_CSO +22k, DLTM +558k
    (the DLTM armed roll is priced by the +3 tier-1 win).

## The mechanism (why ALE works here)

Vivify walk: assume ¬l for each candidate literal under propagation. Old code
only strengthened when the walk CONFLICTED (prefix-shrink). Literals implied
FALSE mid-walk (`FALSE => continue`) were tracked but never removed —
`vivify_removed_literals` was literally always 0. Kissat removes them (ALE).
The replayed `keep` is RUP: propagating ¬keep re-derives every dropped literal
false and the original (still present) clause supplies the conflict. No clause
is ever deleted (the redundant-delete path remains excluded — historically
unsound, div-mitern UNSAT→SAT).

Measured effect (standalone, idle):
- vex: 1696s/3.44M conf (base) → 1230s/2.83M (ALE); strengthenings 14k → 102k.
- oski40: TIMEOUT (never solved) → 1343s (ALE) → **1026s/2.51M @ 300M ticks**;
  strengthenings 237k; props/conflict 626 ≈ kissat's 580 (was 1860 pre-campaign).
- ibm canary: 370k → 368k (ALE) → 347k (ALE+300M). bp4_CSO proof VERIFIED.
- vex saturates at ~1230s regardless of budget (merges frozen 18.4k) — the
  remaining vex gap (1720s in-gate vs kissat 232s) is NOT vivify budget.

## The correctness bug (latent in e5bd1f9 and earlier)

oski40 solving in-gate exposed it: BOTH arms' UNSAT proofs were REJECTED by
drat-trim (`verify=FAIL`, RAT check failed on a merge equivalence binary,
`468056 340750 0`). Root cause: the worklist closure's XOR **cancellation**
(`a ⊕ a` removed during gate renormalization) dropped the cancelled variables
from the DRAT parity ladder. The gate's ORIGINAL clauses still range over the
cancelled vars, so the equivalence binaries need a case split on each — not
unit-derivable → non-RUP. Shipped with ce42829/e5bd1f9; oski never solved
before, so the proof stream was never checked end-to-end.

Fix (congruence.rs): accumulate cancelled vars per gate across renormalization
passes (`acc_cancelled`), carry them into every XOR merge chain (union of both
gates' histories + shared key inputs; table stores the rep's snapshot), emit
the ladder over ALL chain vars (was `l-1`), filter chain vars equal to
var(p)/var(q) (pinned by the RUP assumption; enumerating them creates
tautology holes), cap pathological unions (>12 → drop merge). Validation:
- 2 new unit tests (collapsed + keyed-union paths); 614 total pass.
- ibm byte-identical (369,887), oski40 byte-identical (3,556,063) — proof-only.
- oski40 full 7.5GB proof: **s VERIFIED in 528s** standalone
  (`oski-fixed-dratlong.log`, scratchpad), and verify=ok IN-GATE.

## Negative results this session (measured — do not re-run blind)

1. **SAT_ELIM_PRODUCTIVE_MIN_PCT=40 is dead under the bundle**: mp1 derail
   persists (27s → 586s, 336k → 5.4M conf, and the round machinery ran with
   armed vivify), AND TT406 no longer solves standalone (1751s TIMEOUT — the
   pre-bundle 728s "solve" was the lucky-shuffle trajectory, gone under the
   armed-collapse bundle). TT492 also TIMEOUT at pct40.
2. **Armed restart knobs are non-winners** (new default-off knobs kept as
   groundwork): SAT_RESTART_ARMED_FLOOR=1 → vex −27% conflicts (2.50M) but
   only 1530s; +REUSE_TRAIL_ARMED → TIMEOUT (worse); +MARGIN=1.10 → 1518s.
   Global SAT_RESTART_REUSE_TRAIL=on → vex −11% conflicts, wall noise.
3. **SAT_VIVIFY_SORT (kissat literal-count ordering) is a loser everywhere
   measured**: ibm 368k→625k conf, bp4 slightly worse, vex 1230→1530s,
   oski40 1343s→TIMEOUT. Knob kept default-off. (Kissat pairs sorting with
   candidate sorting + prefix trail reuse; in isolation it just reorders the
   walk against arena order.)
4. vex budget saturation: SAT_VIVIFY_ARMED_TICKS 300M does NOT help vex
   further (1227s vs 1230s; strengthenings 2x but merges frozen).

## Ranked next steps

### 1. vex wall margin (1720s in-gate is 80s from the wire)
vex flipped twice today but at 1661s/1720s — any suite-wide slowdown unfips
it. Cheapest insurance: proof-IO cost (vex writes ~8GB DRAT in-gate; binary
DRAT was measured pointless on div-mitern but never on vex where the proof is
30x bigger). Also the congruence-merge stall (18.4k frozen while kissat
reaches 183k): the closure finds no NEW gate patterns after round ~3 — kissat
keeps discovering because vivify/BVE create NEW ternaries that hash as gates;
check whether our gate extraction sees post-ALE strengthened clauses.

### 2. oski20 (kissat 617s; ours TIMEOUT both arms, 3.68M conf @1751s w/ sort)
Same family as oski40 (which now solves 1380s). oski20-ale (no sort) was
never screened standalone — screen it; if ~1400s it may flip with any
suite-wide margin gain.

### 3. Conflict-density transfer to booth/Bubble/fixedbandwidth
These have 0 congruence merges → the whole armed bundle is inert there. The
ALE mechanism itself is not congruence-dependent — a different arming signal
(e.g., vivify-yield: keep ALE+budget active while strengthenings/round > X)
could extend it without touching fragile cells. Needs a dry-run-style signal
measured on those cells first.

### 4. Housekeeping / traps (additions)
- `pkill -f <pattern>` matches YOUR OWN shell if the pattern appears in the
  command line — use `pkill -f "pat[t]ern"` (exit 144 = self-SIGTERM).
- drat-trim on 7.5GB proofs needs ~530s idle, >1750s under load → in-gate
  `checker-timeout` on vex/sqrt-mitern170 is the benign class; `FAIL` is real.
- feature_ablation's final verification phase runs AFTER all solver cells
  (0 sat-solver processes but drat-trim alive) — don't declare a run stalled.
- `inprocess_rounds` in JSON_STATS is hardcoded 0 (stats.rs:742) — still unwired.
- The A/B preflight warns about the agent's own monitor shells (command lines
  contain "sat-solver") — cosmetic, but kill stray monitors before launching.

## Where the evidence lives
- Winning gate: `log/abtest-cand-vs-base-2026-07-14-14-02-51` + launch log
  `log/abtest-vivifyale2-launch.log` (gate PASS output in session log).
- Rejected first shape: `log/abtest-cand-vs-base-2026-07-14-11-02-34` (LOSE
  65 vs 66, unscoped ALE) + `log/abtest-vivifyale-launch.log` — including the
  symmetric oski40 verify=FAIL that exposed the proof bug.
- Proof-bug repro + fix validation: scratchpad `oskiproof/` (base config,
  NOT VERIFIED at line 4,924,124), `oski-fixed/` (fixed, byte-identical
  trajectory), `oski-fixed-dratlong.log` (s VERIFIED 528s). Scratchpad dies on
  reboot; the numbers are in this note and the commit message.


==============================================================================
### SOURCE: plan/next-steps-armed-collapse-2026-07-13.md
==============================================================================

# Next steps after the armed-collapse-bundle promotion (2026-07-13 night, e5bd1f9)

Context for a fresh session. State as of this writing:

- Medium baseline: **64/100 @ e5bd1f9**, both-solved conflicts 53,406,201, PAR-2
  149,169.6 (conflicts-tier win over 906e7cc lineage: −49,103, PAR-2 −437.8).
  Kissat 4.0.4 reference: 74/100 (`log/kissat-medium-20260705-203444`). Gap ≈ 10.
- Promoted at `e5bd1f9` (one bundle, all armed-formula-only, every knob
  off-switchable):
  - **SAT_ELIM_GATES_EXT** (new): equivalence + ITE gate detection in BVE,
    kissat gates.c order (eq → AND/OR → ITE), fires ONLY in armed mid-search
    rounds (`inprocess_aggressive`) — root elimination untouched everywhere.
  - **SAT_VIVIFY_ARMED** (new): armed formulas vivify every inprocess round
    (learned candidates included), bypassing the 6M-conflict delay that starved
    BMC cells of ALL vivification (vex never vivified once pre-bundle).
  - Default flips: SAT_CONGRUENCE_WORKLIST=on, SAT_ELIM_ARMED_BOUNDS=on,
    SAT_CONGRUENCE_ARMED_MIN_MERGES=32, SAT_CHRONO_PRODUCTIVE_DELTA=100.
- Gate evidence: `log/abtest-cand-vs-base-2026-07-13-20-23-49` (PASS, WIN;
  launch log `log/abtest-elimgatesext-launch.log`). 64==64 identical solved
  sets; only armed cells diverge: bp4_CSO −154,585 conf, ibm +78,346,
  931621d9 +22,989, 6s299b685 +4,147. Wall: ibm 412→165s, bp4 321→265s.

## Load-bearing discoveries

1. **The bundle tames chrono delta=100 on ibm-2004.** The 2026-07-12 rejection
   (ibm derailed 390k→1.34M conflicts at delta=100) does NOT hold once the
   collapse flywheel runs: ibm reaches 145,158 congruence merges (vs ~20k),
   17.3k gate eliminations, 20k vivify strengthenings, and then delta=100 gives
   SAT 133s/370k standalone (vs 250s/981k at delta=1000 — bundle at default
   delta is a conflicts LOSER on ibm; delta=100 is load-bearing for the win).
2. **Vivify never ran on BMC cells pre-bundle**: `should_vivify_inprocess_round`
   skipped ALL vivification below 6M conflicts when learned-vivify was
   formula-active; vex peaks ~1M conflicts in 1800s. Any low-conflict-rate cell
   silently forfeited vivify. (kissat vivifies 322k clauses on vex.)
3. **Kissat's vex profile** (`kissat -s -v`, 167s): congruent matched 183k
   (62% ITE), eliminated 49%, substituted 30%, vivified 322k, 30 probings.
   Backbone units ≈ 0 and transitive reductions ≈ single digits — **backbone
   and transitive-reduction ports are dead ends for the vex gap; do not build.**
4. **Vex's remaining wall is props-per-conflict, not propagation speed**: ours
   3.9M props/s vs kissat 6.2M (1.6x), but 1,860 props/conflict vs kissat 580
   (3.2x) at 78 decisions/conflict vs 35. Raw prop-speed work (CSR watchers,
   tagged binaries) buys at most 1.6x; the conflict-density mechanism (what
   makes kissat conflict every 580 props on a 26k-deep trail) is the real gap.
5. **Ext gates + armed vivify fire but do not convert vex**: 1315s vs 1311s
   bundle-only (identical). Mid-search elimination 2.8x (+14k vars), conflict
   trajectories differ, wall identical. oski's merges finally grow (65,297 vs
   frozen 58,416; 8.7k ITE gate elims) but still TIMEOUT at 1750s.

## Negative results this session (measured — do not re-run blind)

1. vex bundle+delta100: 1444s (worse than 1315s at delta1000). The 07-12
   calibration "delta=100 solves vex ~1000s" does not transfer to the bundle.
2. vex bundle+ext+vivify == bundle-only standalone (1315 vs 1311s).
3. div-mitern172 and sqrt-mitern171 have 0 congruence merges → never arm →
   entire bundle inert there (verified byte-identical conflicts in-gate).
4. Kissat backbone/transitive-reduction: ~zero yield on vex (see 3 above).
5. An early 600s screen suggested +28% conflict rate for ext+vivify; the full
   1750s paired runs show parity — short-window screens on armed cells mislead.

## Ranked next steps

### 1. Conflict density on vex/oski (the +1 class: vex, oski×2, g2, goldcrest)
props/conflict 1860 vs 580. Mechanisms kissat has that plausibly matter here:
- Its **focused-mode restart cadence** on deep trails (interval floor 1 vs our
  50, per-mode EMAs — bead 2nr cluster, previously LOSErs globally but never
  re-tested armed-only under the bundle).
- **Trail reuse on restart** (rejected globally 07-12; also never re-tested
  under the bundle — the ibm/delta lesson says bundle context can flip verdicts).
- Sticky-trail chrono variants: kissat additionally reuses the trail ON
  CONFLICT via `kissat_backtrack_propagate_and_flush_trail` semantics.
Armed-only knobs + single-cell screens (vex/oski standalone) before any A/B.

### 2. Vivify yield (282→15.8k strengthenings happened, but kissat gets 322k)
Our vivify machinery deep-clones arena+watchers per round
(`with_temporary_assumptions`, bead 3yw) and lacks conflict-analysis-based
strengthening. A kissat-parity vivify (in-place, trail-reuse between
candidates, analyze-on-conflict strengthening, tier budgets 3:3:1) is a
multi-session rewrite with a known soundness minefield (the redundancy-delete
path was empirically unsound before — see vivify_round comment). High upside:
it feeds gates to the congruence closure (new ternaries → new ITE patterns),
which is what keeps kissat's per-closure yield at ~17k merges on vex.

### 3. TT406/TT492/lockchart class (kissat 41s on TT406!)
Untouched by this promotion (0 merges → never arm). Needs the elim-yield
arming signal (SAT_ELIM_PRODUCTIVE_MIN_PCT, knob exists, inert) made honest —
with SAT_ELIM_GATES_EXT + armed bounds now default, mid-search elimination on
TT-class may actually eliminate (the 07-12 lucky-shuffle objection was that
rounds eliminated ~nothing; re-measure TT406 standalone with
SAT_ELIM_PRODUCTIVE_MIN_PCT=40 before dismissing).

### 4. Housekeeping / traps (additions)
- `setsid` FORKS: `$!` is the dead parent. Watch `pgrep -f` output, not `$!`.
- Armed-cell trajectory counts reproduce EXACTLY between standalone screens and
  in-gate runs (dltm 102,141; 6s299 10,887) — screens are trustworthy on
  conflicts, not on wall.
- The off-switch replay of the pre-bundle baseline:
  `SAT_CONGRUENCE_WORKLIST=off SAT_ELIM_ARMED_BOUNDS=off
  SAT_CONGRUENCE_ARMED_MIN_MERGES=0 SAT_ELIM_GATES_EXT=off
  SAT_VIVIFY_ARMED=off SAT_CHRONO_PRODUCTIVE_DELTA=1000` (verified byte-exact
  on dltm).

## Where the evidence lives
- Gate: `log/abtest-cand-vs-base-2026-07-13-20-23-49` + launch log
  `log/abtest-elimgatesext-launch.log`.
- Bead: `SAT-playground-2a7` comment dated 2026-07-13 (session 3).
- Kissat vex verbose profile + all standalone screens: scratchpad (gone after
  reboot); key numbers preserved in the e5bd1f9 commit message and this note.


==============================================================================
### SOURCE: plan/next-steps-giant-arena-2026-07-13.md
==============================================================================

# Next steps after the giant-arena promotion (2026-07-13 evening, 906e7cc)

Context for a fresh session. State as of this writing:

- Medium baseline: **64/100 @ 906e7cc**. Kissat 4.0.4 reference: 74/100
  (`log/kissat-medium-20260705-203444`). Gap ≈ 10.
- Promoted at `906e7cc`: **giant-arena parse + lean giant construction**
  (default-on; `SAT_GIANT_ARENA_PARSE=off` = legacy nested parse + full
  allocations). ee5 (11.normalised, 53.9M vars / 145.1M clauses) flipped
  UNKNOWN → SAT 227s in-gate: it was never search-hard (469 conflicts), just
  memory-unfit (25.0GB VmPeak vs the 16GB cap). The diet: parse directly into
  arena words (the `Vec<Vec<i32>>` parse peaked 8.6GB alone), skip
  occurs/n_occ/dirty/binary_dedup_seen/lbd_seen/binary_implications headers
  (~4.4GB of allocations giant-light never uses), exact-capacity watch lists,
  streaming model verify, and an honest giant-path preflight estimate.
  Post-diet ee5: VmPeak 14.36GB.
- Gate evidence: `log/abtest-cand-vs-base-2026-07-13-15-05-01` (PASS, WIN 64
  vs 63; ee5 is the ONLY divergent cell; both-solved conflicts differ by
  exactly ee5's own 469; PAR-2 148,509.9 vs 152,187.5). Launch log:
  `log/abtest-giantarena-launch.log`.

## Load-bearing facts

1. **The memory-fit campaign is now COMPLETE.** All four "normalised" giants
   solve: 18 (SAT 110s), 00fd8ac/2 (SAT ~80s, conf=209), 83aa/1 (SAT ~100s,
   conf=259), ee5/11 (SAT 227s, conf=469). No remaining medium cell is
   UNKNOWN for memory reasons. Every remaining gap cell is a
   search-capability problem.
2. The giants are propagation-heavy, conflict-trivial (200-500 conflicts,
   100-400M props). Their solves are dominated by parse + one long
   propagation fixpoint; the giant-arena parse also made 83aa/00fd8ac ~20%
   faster at half the RSS with byte-identical trajectories.
3. `binary_dedup_seen` is a dead field (allocated, resized, never read) — on
   ALL instances. Left allocated for non-giants to keep them byte-identical;
   a future cleanup could drop it everywhere (0.43GB on giants, pennies
   elsewhere).
4. Trap avoided (worth remembering): `Vec<Vec<T>>` per-literal structures
   cost 24B/header + ~16B malloc overhead per non-empty list. On 108M-slot
   literal-indexed structures that is ~2.6GB before any payload. The
   `BinaryImplications::Flat` variant exists but is unused; the watcher CSR
   rewrite (bead ck8 endgame) remains parked with its conflict-order-parity
   analysis in plan/next-steps-worklist-congruence-2026-07-12.md.

## Where the remaining ~10 cells are (from the 2a7 bead + prior notes)

All search-capability, in rough order of prior evidence:

1. **VexRiscv/oski/g2/goldcrest (BMC/miter cascade)** — VexRiscv solved
   standalone once (1372s, needs <~1000s for in-gate). Cheap armed-cascade
   levers exhausted; next mechanisms per the 07-13 note: transitive reduction
   of the binary implication graph, backbone pass, vivify tiers, and the
   congruence closure gate-pattern gap (kissat 183k merges vs our 19k on vex).
2. **Timetable492 / lockchart / bp4_TCO structured-SAT** — TT406-class
   trajectory kicks are forbidden (lucky-shuffle class); needs real mid-search
   BVE strength (kissat bound escalation) made honest.
3. **booth×2, Bubble, fixedbandwidth conflict-volume cells** — kissat needs
   6.5-14M conflicts at 11-29k conf/s; we are ~10x slower in conflict density
   on these. Chrono (delta=1000) closed part; the rest is inprocessing-driven
   formula collapse, same bundle as (1).

## Housekeeping

- The A/B arm syntax reminder: commas for multiple envs; empty arm spec is
  valid (`--arm 'base:'`).
- sqrt-mitern170 checker-timeout: still the benign symmetric verify artifact.
- rbsat-v1375 solved in BOTH arms this run (1265s/1244s) — the coin-flip cell
  landed heads twice; keep treating ±1 swings involving it as noise.
- Never `cargo build --release` while an ablation is live (this session
  rebuilt only before launch).

## Where the evidence lives

- Gate: `log/abtest-cand-vs-base-2026-07-13-15-05-01` + launch log
  `log/abtest-giantarena-launch.log`.
- Bead: `SAT-playground-2a7` comment dated 2026-07-13 (session 2).
- Standalone validation numbers (scratchpad, gone after reboot) are preserved
  in the 906e7cc commit message.


==============================================================================
### SOURCE: plan/next-steps-preflight-factor-2026-07-13.md
==============================================================================

# Next steps after the preflight promotion + factor groundwork (2026-07-13, 15911aa)

Context for a fresh session. State as of this writing:

- Medium baseline: **63/100 @ 15911aa** (rbsat-v1375 remains the ±1 coin-flip cell;
  it timed out in BOTH arms of this session's gate). Kissat 4.0.4 reference: 74/100
  (`log/kissat-medium-20260705-203444`). Gap ≈ 11.
- Promoted at `15911aa`: **simp-aware memory preflight** (default-on;
  `SAT_PREFLIGHT_SIMP_AWARE=off` = byte-exact legacy estimator). 83aa
  (29.3M vars / 78.8M clauses) flipped UNKNOWN → SAT 100s in-gate: the old
  estimator charged the occurrence entries + inline-abstraction migration
  transient that the giant-light profile never allocates, and priced the
  migration reloc map at usize (stale since the 02e5d00 u32 diet). Estimated
  14,732MB vs threshold 14,400MB; TRUE peak 12.7GB VmSize / 9.8GB RSS.
- Gate evidence: `log/abtest-cand-vs-base-2026-07-13-10-47-36` (PASS, WIN
  solved tier 63 vs 62, PAR-2 152,271.8 vs 156,109.0, zero trajectory diffs on
  shared solved cells — the candidate only changes whether 83aa runs).
- Also landed (default-off, inert): `SAT_FACTOR_INPROCESS` (mid-search BVA with
  fresh-var growth — `grow_variables()` + `VmtfQueue::grow`),
  `SAT_INPROCESS_ROUNDS` (armed proberounds loop), `SAT_ELIM_ARMED_EFFORT_PCT`.

## Load-bearing discoveries

1. **vmtf FocusedOnly is ON in the default profile** (`apply_focused_stable_defaults`).
   Any `vmtf_queue.is_some()` guard silently disables a feature EVERYWHERE. The
   factor knob produced literally zero effect across three screen rounds until
   the queue got a `grow()` method. Audit any future fresh-var or var-indexed
   feature for this trap.
2. **Mid-search factor must use the armed eliminate bound (starts 0), not the
   frontend's mature bound 16** — kissat factors on ANY positive clause
   reduction mid-search. With bound 16 it never fires on the gap cells.
3. **drat-trim proof-line numbers are offset by the CNF clause count** —
   "proof line 389730" on a 389,661-clause formula is proof.out line 69.
   Deletion warnings ("deleted clause does not occur") on armed-bundle proofs
   are pre-existing (reproduce with factor off) and benign (s VERIFIED).
4. **The 16GB "OOM giants" must be re-audited against the preflight, not
   assumed infeasible**: 83aa was solvable all along. ee5 (54M vars, est
   19.9GB) is genuinely over — its +1 needs the arena/watcher architectural
   diet (Vec-of-Vec watcher headers alone ≈ 2.6GB there).

## Negative results this session (measured — do not re-run blind)

1. **Factor on VexRiscv is a LOSS**: 1613s standalone with factor vs ~1240s
   without (same load), despite 6.9k fresh vars / 55k product clauses removed
   in round 1. Clause compression does not convert on the BMC cell.
2. **oski20/40 still TIMEOUT with the full working bundle** (worklist +
   armed-BVE + min-merges 32 + factor): 80k+ product clauses factored, 7
   armed rounds, no solve. The eliminate→congruence→factor cascade alone is
   not the missing piece for oski.
3. **SAT_INPROCESS_ROUNDS=2** (armed proberounds): VexRiscv UNKNOWN at 1702s
   vs control UNSAT 1462s in the same wave — extra pass cost, no payoff.
4. **SAT_ELIM_ARMED_EFFORT_PCT=20**: VexRiscv 1679s vs control 1462s. Worse.
5. **Armed min-merges 8 vs 32**: 1496s vs 1462s — noise, keep 32.
6. **Kissat-scale sweep budgets (depth 3 / 8192 vars / 32768 clauses) are
   PATHOLOGICAL on 400k+-var formulas**: a single armed round runs for HOURS
   (SAT_LIMIT_WALL_SEC is only checked between conflicts, never inside an
   inprocess round), and the config doubled div-mitern172's wall (300s vs
   150s). Our per-seed sweep architecture cannot absorb kissat's budgets; it
   would need per-round tick budgeting first.
7. **VexRiscv standalone times are load-sensitive ±20%**: 1240s (2 concurrent)
   vs 1462s (8 concurrent) for the SAME config. Never compare screens across
   different host loads; pair configs within one wave.

## Ranked next steps

### 1. ee5 memory architecture diet (the next pure-fit +1, kissat solves it in 137s)
True need ≈ 23GB vs 16GB cap. Big pieces: flat CSR watcher layout (kills the
2×54M Vec headers ≈ 2.6GB + allocator slack), u8 phase/bool packing, and the
occurrence-index-free giant path. This is the "big architectural change"
lever; 83aa proved the payoff class is real.

### 2. VexRiscv/oski: the cascade is stalled — different mechanism needed
All cheap armed-cascade levers are exhausted. What kissat still has that we
don't, in likely-impact order for these cells: transitive reduction of the
binary implication graph (every probe round), the backbone pass
(kissat_binary_clauses_backbone, runs twice per round), and vivify tiers with
per-tier budgets. Consider also that kissat's congruence closure reaches 183k
merges on vex vs our 19k — the worklist closure may still be missing gate
patterns (e.g. definitions through XOR chains) rather than budget.

### 3. Factor: keep default-off; possible salvage angles
It provably works and its proofs verify. Salvage candidates: fire only in
round 1 (the big collapse) and not later rounds; or gate on SAT-looking
formulas (MVRoundRobin-class) rather than BMC. Needs a target cell where it
converts before another A/B is worth it.

### 4. Housekeeping / traps (additions to the standing list)
- SAT_LIMIT_WALL_SEC is not honored inside a long inprocess round — screens
  MUST wrap with external `timeout` (screen_run.sh now does).
- The A/B launch log's per-cell lines show identical conflict counts for
  trajectory-identical arms — a cheap sanity check that a "should be inert"
  candidate really is.
- 83aa now occupies a core for ~100s in-gate; wall-clock-sensitive cells
  (CONGRUENCE_ITER_MAX_SECONDS) could in principle notice the scheduling
  change; this gate showed zero conflict diffs, so it did not.

## Where the evidence lives
- Gate: `log/abtest-cand-vs-base-2026-07-13-10-47-36` + launch log
  `log/abtest-preflight-simp-aware-launch.log`.
- Bead: `SAT-playground-2a7` comment dated 2026-07-13 (full numbers).
- Screens (scratchpad, gone after reboot): vex-c0..c9, oski*-c7/c8/c10,
  giant-83aa-{probe,true,capped-fixed}, proofcheck-* — key numbers preserved
  in the bead comment and the 15911aa commit message.


==============================================================================
### SOURCE: plan/next-steps-worklist-congruence-2026-07-12.md
==============================================================================

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

## LATE-SESSION MILESTONE: VexRiscv solved UNSAT standalone (first time ever)

Third knob, same session: **`SAT_CONGRUENCE_ARMED_MIN_MERGES`** (default 0 =
inert). On ARMED formulas, mid-search re-closures use this lower dry-run
threshold instead of the shipped 3000 — the fragile-cell protection rationale
does not apply to a formula whose root closure already cleared the productivity
bar, and the 3000 threshold was measurably blocking the eliminate→congruence
feedback loop (oski merges frozen at 58,416 across all v2 rounds).

With the full bundle (`SAT_CONGRUENCE_WORKLIST=on SAT_ELIM_ARMED_BOUNDS=on
SAT_CONGRUENCE_ARMED_MIN_MERGES=32`, idle, single core, proof off):

- **VexRiscv: `s UNSATISFIABLE` in 1371.9s, 2.77M conflicts** — a kissat-only
  gap cell never solved by any iteration before (kissat: 169s). Mid-search
  cascade: merges 18,360 → 19,287, eliminations +6k over v2; small increments,
  but they convert.
- oski: 2.28M conflicts/1500s (+26% rate over v2, merges now growing) — no solve.
- ibm canary: SAT, another trajectory roll (1.0M conflicts; base 292k / wl 681k).
- goldcrest: 0 merges, untouched as designed.

**Why no A/B**: the in-gate flip line is ≲1000s standalone (32-way contention
≈1.8x); 1372s ≈ 2470s in-gate → still TIMEOUT, while ibm's conflicts-tier roll
worsens. Promoting this bundle needs VexRiscv (or oski/g2) under ~1000s
standalone first. A proof-ON rerun + drat-trim verification was launched at
session end (scratchpad `vexproof.status`) to certify the milestone.

## Ranked next steps

### 0. Push the armed cascade under the in-gate line (direct continuation)
VexRiscv needs −30% standalone. Knob sweeps worth screening (cheap, standalone):
armed threshold 32 → 8; armed-BVE effort 10% → 20%; and the real lever,
per-round ELS substitution + vivify on armed formulas between eliminate rounds
(kissat runs substitute twice per probe round). Then the factor step (below).

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


==============================================================================
### SOURCE: plan/next-steps-chrono-productive-2026-07-12.md
==============================================================================

# Next steps after the chrono-productive promotion (2026-07-12, 689f080)

Context for a fresh session. State as of this writing:

- Medium baseline: **62-63/100 @ 689f080** (rbsat-v1375 is still the ±1 coin-flip cell:
  solves at ~1745-1795s or times out, arm-symmetric noise — it landed TIMEOUT in BOTH
  arms of the promotion A/B). Kissat 4.0.4 reference: **74/100** fresh matched run
  (`log/kissat-medium-20260705-203444`). Gap ≈ 11-12 cells.
- Promoted at `689f080`: **SAT_CHRONO_PRODUCTIVE_DELTA=1000** default-on — a kissat
  `chronolevels` analog (learn.c: backjump discarding > N levels → chronological
  backtrack 1 level), applied ONLY on congruence-root-productive formulas (≥1000
  applied root merges, the exact signal that arms `inprocess_aggressive` since
  c579bfe). Implemented in `maybe_arm_congruence_productive_search()` (main.rs).
- Gate evidence: `log/abtest-cand-vs-base-2026-07-12-10-48-37` (PASS): 62==62
  identical solved sets, both-solved conflicts 53,454,576 vs 53,552,717 (−98,141),
  PAR-2 155,681.7 vs 155,820.8. **ibm-2004 is the ONLY trajectory-changed cell**
  (390k → 292k conflicts, −25%, 428→409s).
- Also landed in 689f080: congruence_merges/and_gates/ite_gates now emitted in
  JSON_STATS (were declared but never written — cost an hour of confusion);
  `SAT_ELIM_PRODUCTIVE_MIN_PCT` groundwork (default 0 = inert, see negative results).

## The load-bearing discovery (redirects the throughput campaign)

Same-host 240s head-to-head on VexRiscv (kissat solves it UNSAT in 169s standalone):

| metric | ours (base) | kissat |
|---|---|---|
| decisions/s | 343k | 368k |
| props/s | ~1.1M | 6.1M |
| decisions per conflict | 767 | 35 |
| conflicts/s | 335 | 10,526 |

**Decision throughput is at parity; the gap is conflict DENSITY (22x), not raw
speed.** The prior "14x search-rate" framing conflated the two. Two mechanisms feed
kissat's density: (a) chronolevels=100 preserves the deep trail (now partially closed
by 689f080), and (b) mid-search inprocessing keeps collapsing the formula — kissat's
VexRiscv run: 49% eliminated + 30% substituted + 28% congruent across **30 probings /
13 eliminations / 57 vivifications / 53 factorizations**, all mid-search. Our root
congruence reaches a syntactic fixpoint at 22.9k merges and stalls; kissat cascades to
183k matched because each closure re-runs cheaply between eliminations (worklist,
not whole-formula re-extraction).

Contention fact for planning: the 32-way gate costs ~1.8x wall vs idle. **An in-gate
cell flip needs ≲1000s standalone solve time** (VexRiscv @ delta=100 solved ~1000s
standalone and still timed out in-gate, twice).

## Negative results this session (measured — do not re-run blind)

1. **delta=100 (exact kissat parity), armed cells**: round-1 A/B
   `log/abtest-cand-vs-base-2026-07-12-07-50-47` LOSE 62 vs 63 — ibm-2004 derailed
   (390k → 1.34M conflicts, 442→735s) and rbsat noise-flipped (base squeaked in at
   1794s/1800s; rbsat has 0 congruence merges → its flip was pure clock noise).
   VexRiscv solved standalone (~1000s, complete 8.4GB DRAT) but NOT in-gate. Delta
   sweep on the (VexRiscv, ibm) pair: 100 → vex ~1000s / ibm 1.34M conf; 300 → vex
   1442s / ibm 1.09M; **1000 → vex 1578s / ibm 292k (only config that improves ibm;
   fires 453 vs 13k times)**. Scratchpad JSONs are gone after reboot; numbers here
   and in the 689f080 commit message are the record.
2. **SAT_RESTART_REUSE_TRAIL on armed cells**: worse everywhere — VexRiscv
   delta=100+reuse TIMEOUT (vs ~1000s without), ibm worse at every delta tested
   (best ibm+reuse 913k conf vs 292k at plain delta=1000). Knob exists, default off;
   leave it off.
3. **Elimination-yield arming (SAT_ELIM_PRODUCTIVE_MIN_PCT=40)**: arming
   `inprocess_aggressive` (early doubling cadence + mid-search `eliminate(true)`)
   on root-BVE-yield ≥40% **solves Timetable406 standalone in 728s** (baseline
   TIMEOUT in-gate twice; kissat 41s) — BUT full-suite screen shows 19 cells arm,
   including solved cells, and **mp1-Nb7T46 derails 35s → >900s TIMEOUT (measured)**.
   Pass ablation (probe/sweep/vivify off): mid-search `eliminate(true)` alone BOTH
   solves TT406 AND derails mp1, with ~0 actual elimination in both (+105 vars on
   TT406). The effect is the round's trajectory kick (branch-queue rebuild), i.e. a
   lucky shuffle — the exact class the development rules forbid promoting. Threshold
   can't separate (mp1 48.4% vs TT406 49.9% vs TT492 48.7%). Knob landed default-0
   (inert) with unit tests. Full-suite root-elim yields: Timetables 48-50%, g2 81.8%,
   goldcrest 54%, booth×3 ≈48-50%, Bubble 40.8%, sudoku-N30 51.1% (solved UNSAT
   1267s — thin margin, arms!), mp1 48.4%; fragile cells all below 35% (oddball
   20.5%, Kakuro 30.5%, bp4 19-30%, velev 8.4%, rbsat/sted2/lockchart ~0%).
4. **Timetable492 does not solve standalone even when armed** (1750s wall, niced).
   TT406 is the only Timetable within reach of a trajectory kick.
5. **Perf profiling is unavailable** (kernel.perf_event_paranoid=4, no passwordless
   sudo). The solver's own JSON_STATS + SAT_STATS_HOT counters were sufficient for
   everything above; `kissat -s [-v]` on the same instance is the reference column.

## Ranked next steps

### 1. Worklist congruence closure (top pick — makes re-closures cheap)
Our `try_congruence` re-extracts ALL gates (1.02M on VexRiscv, 4-8s) every round and
stops at the syntactic fixpoint (22.9k merges). Kissat rehashes only gates whose
inputs merged, so it can afford to re-run the closure in EVERY probe round as
eliminations expose new congruences (→183k cumulative on VexRiscv). Port that:
maintain gate index keyed by inputs; after ELS substitution, re-normalize/rehash only
affected gates. Payoff: turns the armed-cell inprocess rounds into real formula
collapse; direct attack on VexRiscv/oski/g2/goldcrest (oski20 reached 1541 conf/s
under delta=100 but didn't finish — it needs the formula to keep shrinking).

### 2. Real mid-search BVE strength (bead 5b2.3.35) — would make elim-arming honest
Mid-search `eliminate(true)` currently re-runs at frontend bounds (grow=0, clslim=20)
and eliminates ~nothing (+105 vars on TT406, ~0 on mp1). Kissat: bound 0→16 geometric
escalation, clslim 100, occlim 2000, multi-round; on TT406 that's 67% of vars over 4
mid-search eliminations (+15k factored). If mid-search rounds ACTUALLY eliminate,
the TT406 win stops being a lucky shuffle and elim-yield arming (knob already in,
screen data above) becomes promotable on mechanism evidence. Gate it on the same
productivity dry-run pattern: escalate bounds only while the previous round's yield
was real.

### 3. Mid-search factor (inprocessing factor)
Kissat runs factor at the end of every probe round (53 factorizations on VexRiscv,
14 on TT406, 15k vars). Ours is frontend-only (≤10^4 vars — never fires on the
217k-297k-var Timetables). Needs mid-search fresh-var growth (resize var-indexed
arrays) — the known BVA follow-up from the a402efd promotion notes.

### 4. Proof/IO + hot-loop cost shaving (margin for in-gate flips)
VexRiscv writes 8.4GB text DRAT during its ~1000-1600s solve; 32 concurrent writers
amplify this in-gate. Binary DRAT (~2-3x smaller, cheaper formatting; drat-trim
auto-detects) is a contained ProofLog change. Hot-loop parity items measured but
unexploited: binary-edge scan does a random load into a 48-byte BinaryClause struct
per edge (kissat touches only values[]); `mark_binary_clause_used` writes metadata
per binary propagation; kissat's `c->searched` replacement-search cache is
irrelevant for BMC (avg clause len 4.6) but the binary-edge metadata is not. These
are trajectory-neutral speedups (mode/restart/reduce are conflict/tick-based; only
CONGRUENCE_ITER_MAX_SECONDS=300 is wall-clock) — worth a bundle when a cell sits
within ~20% of the in-gate line.

### 5. Housekeeping / traps
- The A/B arm syntax uses **commas** for multiple envs; a space-separated spec
  silently kills every cand cell (UNKNOWN_rc2 at 0s).
- **Never `cargo build --release` while a feature_ablation run is live** — later
  cells would exec the new binary mid-A/B. Use
  `CARGO_TARGET_DIR=<scratch> cargo build --release` for side experiments (this
  session's isolated-binary pattern), and run them niced on the free cores
  (36 total, gate pins 0-31); timings under load are meaningless, trajectories fine.
- check_promotion_gate's `running_solver_processes_detected` FAIL can be a false
  positive from your own shell wrappers whose command line contains "sat-solver" —
  kill the stray shell and re-run the gate.
- sqrt-mitern170 checker-timeout: still the benign symmetric verify artifact.
- sted2_0x1e3-216 solved at 1628s and sudoku-N30 at 1267s are the thinnest-margin
  solved cells; any wall-cost regression shows up there first.
- rbsat ±1: ignore in analysis, but the gate is mechanical — if it alone decides a
  LOSE, re-run the full A/B rather than arguing with the gate.

## Where the evidence lives
- Gap bead: `SAT-playground-2a7` (2026-07-12 comments: head-to-head numbers, delta
  sweep, elim-arming post-mortem, full negative-results list).
- Promotion A/B: `log/abtest-cand-vs-base-2026-07-12-10-48-37` (+ launch log
  `log/abtest-chronoprod1000-launch.log`); rejected round-1:
  `log/abtest-cand-vs-base-2026-07-12-07-50-47` (`log/abtest-chronoprod-launch.log`).
- Kissat reference stats: re-derivable via
  `benchmarks/reference-solvers/kissat-latest/build/kissat -s <cnf>` (VexRiscv 169s,
  TT406 41s were measured fresh this session on this host).
- Commit message of `689f080` carries the full calibration table.


==============================================================================
### SOURCE: plan/next-steps-congruence-gap-2026-07-12.md
==============================================================================

# Next steps after the congruence-inprocess promotion (2026-07-12, c579bfe)

Context for a fresh session. State as of this writing:

- Medium baseline: **62-63/100** (63 nominal; rbsat-v1375 is a 1745s/1800s coin-flip cell).
  Kissat 4.0.4 reference: **74/100** fresh run (`log/kissat-medium-20260705-203444`),
  80 on the older reference run. Gap ≈ 11-16 cells.
- Promoted at `c579bfe`: SAT_CONGRUENCE + SAT_CONGRUENCE_XOR default-on; congruence runs
  first in every inprocess round (kissat probe.c order); all-or-nothing dry-run merge
  threshold (3000, env `SAT_CONGRUENCE_MIN_MERGES`; 10M-clause cap); root-productive
  formulas (≥1000 root merges) switch to early doubling inprocess cadence (first round
  10k conflicts) + mid-search BVE rounds.
- Gate evidence: `log/abtest-cand-vs-base-2026-07-12-00-03-56` (PASS, 62==62 identical
  solved sets, both-solved conflicts −2.0%, ibm-2004 −74%). Negative control (no
  threshold): `log/abtest-cand-vs-base-2026-07-11-21-59-04` (59 vs 63 LOSE — every lost
  cell was a SAT formula rewritten for sub-threshold merges).

## The one transferable pattern

**Formula-rewriting passes must dry-run a productivity signal on the untouched formula
and bail edit-free below threshold.** Fragile SAT cells die from zero-payoff rewrites
(Timetables: 34k hidden binaries / 0 merges; Kakuro: pure-binary ELS rewrites). This is
what converted a −4-solved regression into a conflicts-tier win. Apply it to any future
inprocessing/preprocessing candidate.

## Ranked next-step ideas

### 1. Raw propagation/search throughput (biggest single lever, hardest)
Measured on VexRiscv: ours 717 conflicts/s vs kissat 10k/s (14x), ~2-4M props/s vs
kissat's ~10x more; 3250 props/conflict, 400 decisions/conflict, deep BMC trails
(level ~3864, trail 100-300k). The congruence×eliminate interleave is now default-armed
for miters/BMC — throughput is the remaining blocker on VexRiscv/oski/goldcrest/g2 (and
booth×2, Bubble, fixedbandwidth are pure conflict-volume/throughput cells: kissat needs
6.5-14M conflicts at 11-29k conf/s on them).
Concrete angles (mostly open beads under "Hot-path throughput and memory layout"):
- Watch-list layout: blocking literals hit rate, compact watcher structs, arena
  cache-locality after GC; bead 5b2.7 (literal-indexed i8 values) was a null result —
  don't re-run without a new mechanism.
- Profile a BMC cell end-to-end (`/analyzesat` skill; perf) — where do the 650M
  props/200k conflicts actually go? Suspect: huge watch lists on deep trails, arena
  fragmentation after 47M root resolvents (bead 5b2.3.23: no GC during eliminate).
- Measure props/s on both-solved cells vs kissat to size the global gap precisely.

### 2. Push the aggressive-inprocess mechanism further on productive formulas
Now that trajectory damage is gated off, the interleave itself can get stronger:
- **Substitute/ELS as a first-class round step** on productive formulas (kissat runs
  substitute twice per probe round). Currently ELS only fires inside congruence/sweep.
- **Mid-search factor** (inprocessing factor — kissat runs factor at the end of every
  probe round; ours is frontend-only, ≤10^4 vars). Needs fresh-var growth mid-search
  (resize var-indexed arrays). Memory note says this was the planned BVA follow-up.
- **Congruence fixpoint efficiency**: the 64-round whole-formula re-extraction loop
  spends 46s finding ~20 merges/round in the tail (VexRiscv). A kissat-style worklist
  (rehash only gates whose inputs merged) would cut round cost ~10x and allow higher
  cadence.
- **Sweep effort scaling**: our sweep env budgets are fixed (256 vars/1024 clauses/
  depth 2, 512 seeds); kissat scales to 8192 vars/32768 clauses/depth 3 on success and
  spends 10% of search ticks. Sweep found ~0 equivalences on the gap cells while
  kissat's found hundreds — the budget, not the algorithm, is the difference.
- **BVE strength on productive formulas**: our grow=0/clslim=20 vs kissat bound=16/
  clslim=100/occlim=2000 multi-round (open bead 5b2.3.35). Gating a stronger BVE on the
  same productivity signal avoids the trajectory-shuffle that killed prior attempts.

### 3. Structured-SAT search cells (Timetable×2, lockchart-group1, bp4_TCO — 4 cells)
Kissat wins Timetable_C_406 in 41s via 67% mid-search elimination + factor + walk
rephasing, only 170k conflicts. Our timetables are now protected (0 merges → untouched),
so attacking these needs a different productivity signal than congruence merges —
e.g., elimination-yield dry-run: try a bounded BVE probe at the first inprocess round;
if it would eliminate >X% of vars, enable aggressive elimination+factor for the rest of
the run. Walk/rephase was characterized as not-the-answer standalone (WalkSAT bead note,
reverted); kissat's walk is a rephasing assist, not a solver.

### 4. Giants (83aa/ee5 infeasible >16GB, pj2008 search-slow)
Memory note sat-medium-oom-memory-rewrite: the remaining +1 is a behavior-preserving
usize→u32 refactor of original_clause_ids/decision_level/etc (~0.6GB) so the reloc map
fits 00fd8ac — already PROVED solvable (121s@18GB). Mechanical, well-documented,
trajectory-safe. Probably the cheapest remaining +1 solved if it holds under the gate.
(pj2008 is fixed-for-OOM but search-bound; 83aa/ee5 need >>16GB, skip.)

### 5. Housekeeping / known traps
- The medium A/B arm syntax uses **commas**: `--arm 'cand:SAT_X=on,SAT_Y=on'`.
  Space-separated env specs silently create one invalid var → every cand cell dies
  with UNKNOWN_rc2 at 0s (two aborted runs learned this).
- `checker-timeout` on sqrt-mitern170's huge proof is a known benign verify artifact
  (symmetric across arms).
- rbsat-v1375 solves at ~1745s — treat ±1 solved swings involving it as noise.
- Restart/mode/phase constant parity levers (bead 2nr items a-f) are ALL exhausted
  LOSErs on medium; do not re-litigate without new mechanism evidence.
- Solved-count is a razor-sharp local optimum: wins come from capability additions
  whose edits are productivity-gated, never from trajectory perturbation.

## Where the evidence lives
- Gap bead: `SAT-playground-2a7` (fully updated 2026-07-12).
- Kissat verbose stats on gap cells: scratchpad (gone after reboot) but reproducible:
  `benchmarks/reference-solvers/kissat-latest/build/kissat -s -v <cnf>`.
- Dry merge counts per cell: in the 2a7 notes + c579bfe commit message.
- This session's A/B logs: `log/abtest-congrinproc-launch{,2,3}.log`,
  `log/abtest-congrinproc-v2-launch.log`, dirs `log/abtest-cand-vs-base-2026-07-11-*`
  and `log/abtest-cand-vs-base-2026-07-12-00-03-56`.
