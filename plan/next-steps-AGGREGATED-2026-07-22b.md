# AGGREGATED next-steps plan — 2026-07-22b (supersedes next-steps-AGGREGATED-2026-07-22.md)

One-file plan for the next clear context. Folds the 2026-07-22 evening session
(endgame-cadence arc + two decisive negative results + kissat rate-gap census)
on top of the morning tseitin aggregate. Where this contradicts an older
`plan/next-steps-*.md`, THIS file wins.

GATE IN FLIGHT AT WRITE TIME: `log/abtest-cand-vs-base-2026-07-22-22-15-39`
(cand default = SAT_ENDGAME@4M + gauss var-recycling + arming stats, vs
base:SAT_ENDGAME=off). Expected tie-or-rare-win (see below). RESULT: see the
addendum section at the bottom — filled in when the gate lands.

## TL;DR — what changed this session

1. **NEGATIVE (permanent): no more XOR/parity targets in the suite.** All 29
   timeout instances scanned with a corrected stream-tokenizing parser for (a)
   closed odd Tseitin components and (b) GF(2)-inconsistent XOR subsystems with
   provenance certificates: ZERO hits. (An earlier line-based parser produced
   11 false positives — bp4/booth/SC25/pj2008 etc. all artifacts.) The
   tseitin_n188 win pattern has no remaining targets here.
2. **NEGATIVE (measured, permanent under .rs-only rules): tseitin_grid_n400
   stays unsolved.** Even with definition-variable recycling (proof max-var
   1.1M -> 319,601; implemented, tested, kept), backward drat-trim needs
   >2046s idle on the 14.63M-lemma proof (forward parse alone >910s) vs the
   1800s checker cap. Emission breakdown (new SAT_DEBUG_GAUSS counters):
   main=7.62M/shift=2.54M/append=3.82M — the s=2 resolution-cascade
   intermediates are an architectural floor (~48/step minimum, analyzed
   exhaustively: snake order, eq-splitting, fusions all >= 52/step). Getting
   under the ~6M-lemma budget needs a fundamentally different derivation or a
   checker change (forward mode/LRAT — harness change, not .rs).
3. **SAT_ENDGAME implemented (default on, trigger 4M armed conflicts) — the
   reroll-free scoped-firing shape.** Provably cannot touch any solved cell:
   armed-solved conflict census (exact-deterministic, digit-for-digit TSV
   reproduction): sqrt170 3,889,649 (max), tt492 3,729,575, pancake 2,890,977;
   every >=4M-conflict solved cell (59-129706 7.87M, rbsat 6.26M, vdw462
   5.84M, sted2 4.40M) verified UNARMED. New stats:
   `inprocess_armed_at_conflict`, `endgame_at_conflict`.
4. **THE BIG FINDING: kissat solves SC25_Timetable_C_406 in ~31s** (SAT, 170k
   conflicts, 18 vivifications, 4 walks, restart interval 23). TT406 is NOT a
   universal timeout — it's a cadence-capability gap. And the endgame cadence
   FIXES it: with the endgame bundle active from arming (trigger=1 test),
   **TT406 solves: SAT 1632s / 1.63M conflicts** (baseline: timeout, 5.16M
   conflicts by 1800s, restart interval 235 vs kissat 23).
5. **But the win is locked behind a lottery re-deal:** endgame-from-arming
   KILLS the three armed solved cells (sqrt170 TIMEOUT — pinned-10k inprocess
   slows dense refutation ~40%; pancake TIMEOUT at 2.97M vs 2.89M needed —
   agonizingly close; tt492 SAT-lottery rerolled away) and does NOT flip
   tt495/tt496. Net -3/+1. At the safe 4M trigger the TT cells get only
   ~300k conflicts of runway — measured no-flip; clqcl40 got a FULL 3.9M
   runway (decision-armed, latch at ~905s) — no flip either.
6. **Kissat rate-gap census on the gate-deciding margin cells** (300s windows):
   rbsat-v1375 3.1x conflicts/s gap (ours 4.4k vs 13.6k; 199 ticks/prop!),
   sted2 2.4x, oski15a01b20s 4.9x (props/s 1.62M vs 7.0M!!). This contradicts
   the ibm-class parity (flywheel session) — the dense/margin cells have a
   REAL rate gap. The propagate loop is already fully dieted (blockers,
   prefetch, inline-bin, pool — verified in source); the driver is learned-DB
   size / scan volume (kissat's reduce discipline) which is
   trajectory-coupled. No allocation-shred remains >~1s/300s (walk ~2%,
   vivify Vec allocs ~1s, proof I/O few s).

## Current lineage state

- HEAD: 687a366 + uncommitted validated bundle (gauss recycling + endgame@4M +
  arming stats), gate in flight. Medium baseline 70/100
  (`log/abtest-cand-vs-base-2026-07-22-14-52-12/cand/results.tsv`).
- If the gate PASSED (tie or win): commit the bundle. If it somehow failed:
  revert everything, keep this plan + the scratchpad evidence.

## RANKED PLAN for next session

1. **The TT-class re-deal, done right (the only measured +1 on the table).**
   The endgame cadence solves TT406 (1632s idle-ish load) but any from-arming
   deployment rerolls tt492/vex/oski15x2 (decision-armed solved). Paths, in
   order of promise:
   (a) **Decompose the bundle on TT406 vs the victims.** Which knob solves
       TT406 — flat-50k rephase, restart floor 10, or pinned-10k inprocess?
       Which kills sqrt170/pancake (bet: pinned inprocess — 300 vivify rounds
       with arena clones) vs tt492 (pure lottery)? A subset that solves TT406
       while sparing the dense miters shrinks the reroll set to the
       decision-armed four (tt492/vex/oski15x2). 4 cells x 3 subsets x 1800s
       — an evening of compute, no gate needed until a subset wins.
   (b) **Re-luck bundle:** if a subset solves TT406 (+maybe 495/496) and the
       measured reroll set holds (vex/oski40 have fat margins; oski15b20s
       1792.7s and tt492 1468.5s are the coin-flips), a deliberate bundle
       gate: expected +1 to +3 vs risk of losing 1-2 banked lottery cells.
       Do the idle-measure FIRST (searched-law: never gate blind rerolls).
   (c) Restart-cadence-only change (SAT_RESTART_ARMED_FLOOR exists, default
       off, "inert" — kissat restarts 10x denser on TT406): smallest possible
       re-deal; test on all 6 decision-armed cells idle first.
2. **Dense-margin-cell rate gap (rbsat 3.1x, oski15 4.9x)** — the biggest
   headroom number found in months, but the lever is learned-DB discipline
   (kissat reduce keeps far fewer clauses => shorter watch lists => fewer
   ticks/prop), which rerolls every trajectory. Ideas that DON'T reroll:
   none known. Ideas that reroll: kissat-parity reduce tiers (a full re-luck
   campaign — bundle with 1b?). Measure first: instrument kept-clause counts
   + ticks/prop on rbsat under kissat's tier limits in an offline A/B.
3. **Inprocessing cadence for never-armed slow cells** (goldcrest 474 conf/s,
   lockchart 330 — never reach any trigger; kissat inprocesses time-based).
   Unchanged from prior aggregate; now sharpened: any time/tick-based cadence
   must be scoped off the solved set (the endgame census machinery +
   `inprocess_armed_at_conflict` stat make that audit cheap now).
4. **Giant memory diet** (pj2008 RSS 10.4GB vs kissat 1.4GB; BVE emitted 71M
   proof adds + 74.7M dels = 1.7GB DRAT written+discarded in 150s!) — the
   proof-churn during preprocessing is also a WALL cost on every big cell.
   Two shreds worth measuring: buffer/batch the BVE deletion emission, and
   the occurrence-list peak during preprocessing (the 9.5GB spike).
5. **Carried forward from the 21c/22 aggregates (unstarted kissat gaps,
   still valid):**
   (a) Elimination depth: kissat 72-88% var elimination on circuit miters vs
       our 43-56% (gap-read 2026-07-21). Bound escalation and rounds=2 are
       measured-dead (elimbounds session) — the remaining delta is in
       substitution/definition-extraction interplay, not raw bounds.
   (b) SAT-sweeping productivity: kissat kitten does 90k-18M solves/run
       extracting backbones+equivalences; our sweep finds 0-826 facts.
       First known defect: `sweep_round` restarts its 512-seed scan at var 1
       every round (no persistent cursor). Bead SAT-playground-5b2.3.39 area.
   (c) Tiered vivification port + probing/HBR parity (21c #4/#5), unstarted.
   All three are trajectory-coupled (reroll the >=1M-conflict solved cells);
   scope with the armed-census machinery or bundle with a measured re-deal.
6. **Grid n400 / XOR arc: STAYS CLOSED even under the 2x checker budget.**
   The checker budget is now 2x the solver limit (3600s) — feature_ablation.py
   `_verify_result` + bench.sh (commit e46f7a4) — but the definitive uncapped
   idle measurement killed the arc anyway: backward drat-trim on the recycled
   14.63M-lemma proof ran **>4754s CPU without finishing** (killed; single
   thread, idle 36-core host). That exceeds even the relaxed 3600s cap before
   any gate-load inflation. Earlier ">2046s" was a kill point, not a
   completion — the true verify time is at least 2.6x the old cap and >1.3x
   the new one. Conclusion unchanged in kind, strengthened in degree: the +1
   needs a proof ~<=8M lemmas (architecture floor analysis says this class
   bottoms at ~12M) or a forward-mode/LRAT checker change. Next realistic
   lever if ever wanted: make the harness use drat-trim `-f` (forward mode)
   for proofs above a size threshold — the 22s-generated proof would verify
   in minutes.

## Standing traps (carried forward + new this session)

- `results.tsv` written only at run END — monitor per-cell lines in the launch
  log; completion = "DONE ->".
- checker-timeout on UNSAT = gate correctness FAIL; backward drat-trim budget
  ~<=6M lemmas under the 1800s cap (~8-25k lemmas/s; grid measured the hard
  way AGAIN this session: 14.6M lemmas >2046s idle even at 320k max-var).
- NEW: **conflict counts are EXACTLY deterministic across load** (8/8 armcheck
  cells reproduced the in-gate TSV digit-for-digit idle) — conflict-count
  triggers scoped above the solved set are provably reroll-free; wall/tick
  triggers are NOT (load-dependent).
- NEW: armed solved set + exact conflicts (2026-07-22): tt492 3,729,575 /
  sqrt170 3,889,649 / pancake 2,890,977 (armed); 59-129706, rbsat-v1375,
  vdw462, sted2, bp4_BC012 unarmed. Any armed-scoped trigger must clear
  3,889,649. Census via `SAT_ENDGAME=off` runs + `inprocess_armed_at_conflict`.
- NEW: rbsat945/vdw663/st659 (dense timeouts) never arm at all — the yield
  composite skips them (deep-phase or yield); don't assume dense => armed.
- NEW: pinned 10k-conflict inprocess cadence SLOWS dense refutation cells ~40%
  (sqrt170: 2.37M conflicts reached vs 3.89M needed) — vivify rounds with
  arena clones every 10k conflicts are expensive; never apply to yield-armed
  dense cells pre-solve.
- SAT_STATS_JSON needs `=on`; SAT_LIMIT_WALL_SEC for windows; perf unusable
  (perf_event_paranoid=4); no `cargo build` while a gate runs; watch cwd (the
  run.sh-not-found trap hit again); heredoc scratch writes flake — use the
  Write tool; stray sat-solver orphans — `pgrep -a sat-solver` before gates.
- 32x16GB preflight-warns vs 502GB RAM; cap not reservation.
- Rebuilding target/release DURING single-instance validation runs is safe
  (inode swap), but never during a gate.

## solver12's capability edge (protect in rerolls)

xor_op x2, oddball_80_5, Kakuro-easy-132, MVRoundRobin_n16_d10, case1,
tseitin_n188_d3 (SAT_TSEITIN). Kissat cannot solve these in 600s uncontended.
NOTE: TT406 is NOT on this list — kissat solves it in 31s; WE are the ones
missing it (cadence gap, plan #1).

## Where the evidence lives

- This session: `plan/next-steps-AGGREGATED-2026-07-22b.md` (this file);
  scratchpad session-notes + per-cell outputs (tt406/tt495/tt496/sqrt170/
  pancake/tt492/rbsat945/vdw663/clqcl40/st659/oski15/rbsat/sted2/pj/grid/n188:
  windows, armchecks, endgame variants, kissat comparisons) — scratchpad is
  session-scoped; the durable numbers are all in this file.
- Gate: `log/abtest-cand-vs-base-2026-07-22-22-15-39` + launch log
  `log/abtest-endgame-launch-2026-07-22.log`.
- Morning tseitin session: `plan/next-steps-tseitin-2026-07-22.md`, gate
  `log/abtest-cand-vs-base-2026-07-22-14-52-12`.

## ADDENDUM — gate result (2026-07-23 00:30)

- `promotion_gate=PASS`. cand 68/100 vs base 68/100, conflicts on both-solved
  cells EXACT tie (62,041,959 identical — trajectory-identity held perfectly,
  as engineered), PAR-2 138,509.9 vs 138,990.9 (−481, timing noise;
  verdict "win" via PAR-2 tie-break only). Zero contradictions, zero
  correctness failures.
- Both arms dropped the SAME two cells vs the 70/100 lineage TSV:
  oski15a01b20s (lineage 1792.686s of 1800 — the documented 7.3s-margin
  lottery) and rbsat-v1375 (lineage 1747.4s, 52.6s margin). Identical in both
  arms => load lottery, NOT the candidate. The reroll-luck law's prediction
  held exactly. **The lineage baseline for future A/Bs remains the 70/100
  TSV: `log/abtest-cand-vs-base-2026-07-22-14-52-12/cand/results.tsv`.**
- COMMITTED as validated-neutral groundwork (user's standing preference for
  gate-validated foundational changes with documented +1 setups): the
  SAT_ENDGAME machinery + armed-census + instrumentation set up plan #1
  (TT-class re-deal); the gauss recycling sets up any future proof-size
  reduction on the grid arc; no metric claim is made for this commit.
