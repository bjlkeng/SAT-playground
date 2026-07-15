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
