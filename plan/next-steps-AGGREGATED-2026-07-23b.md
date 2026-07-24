# AGGREGATED next-steps plan — 2026-07-23b (supersedes next-steps-AGGREGATED-2026-07-23.md)

One-file plan for the next clear context. Folds the 2026-07-23 evening
endgame-delta-band session on top of the morning (endgame-rf) aggregate.
Where this contradicts an older `plan/next-steps-*.md`, THIS file wins.

## TL;DR — what happened this session

**SAT_ENDGAME_REPHASE_DELTA band-scoped 48k candidate: TT496 FIRST-EVER
solve (917s idle / 1097s in-gate / 2.57M conflicts; kissat 4.0.4 times out
at 1800s on BOTH TT495 and TT496 — measured this session). Gate 2 verdict:
PASS — WIN 70 v 68 (cand +TT496 capability, +sted2 favorable wall coin;
rbsat-v1375 AND bp4_TCO timed out in BOTH arms this deal — gate 1's 71s
had embedded two extra coin wins in both arms). PAR-2 −4631.8, zero
contradictions, zero correctness failures. PROMOTED as
ENDGAME_REPHASE_DELTA_DEC=48_000 default.**

Session mechanics (5 probe rounds, 2 full gates):

1. **TT492 armed_at = 200,057 — IDENTICAL to TT406's.** A MIN_ARMED band
   can never separate them (plan #1a closed). TT492 is DEAD at 1800s across
   ALL 12 rephase deltas tested (25/42/44/46/47/48/49/51/52/60/75/100k) —
   its old SAT draw existed only on the pre-rf trajectory. Accept and move on.
2. **Delta sweep law (TT class = pure cadence lottery):** TT496 solves ONLY
   at d48k (917s) and d49k (1777s — useless margin) out of 12 deltas. TT406
   solves at every delta but with 3x conflict scatter (351k@30k … 3.48M@44k;
   NO smoothness, adjacent deltas differ 3x). Fine-tuning deltas is drawing
   lottery tickets; only fat-margin draws (>600s) are promotable.
3. **Gate 1 (unbanded d48k): formal FAIL, and the failure decomposition
   became the fix.** cand 71 = base 71 (cand +TT496, −oski15 wall-coin at
   1685s base margin 115s), tier-2 conflicts +411,555: yield-armed rerolls
   +349k (sqrt170 +166k, oddball24 +123k, sqrt171 +72k…) + TT406 +316k
   − TT395 −254k. PAR-2 was BETTER (−433). Zero correctness failures.
4. **The band-scoping fix (the session's mechanism find):** the tuned delta
   now applies only to DECISION-ARMED cells (`armed_at <
   SAT_ENDGAME_DELTA_SPLIT`, default 500k); yield-armed (~800k) cells keep
   the legacy flat 50k. Verified DIGIT-EXACT six ways (round 5): sqrt170 →
   base 3,917,743 and div172 → base 1,221,342 under env=48000 (yield
   protection); TT395/406/496 → exact d48k trajectories (passthrough);
   TT406 no-env → shipped 593,251 (default equivalence). Banded diff vs
   base = THREE cells only: TT395 −254,253, TT406 +315,799, TT496 +flip;
   net +61,546 at an instance tie.
5. **Gate-2 outcome logic (decided before launch):** TT496's +1 stands
   unless an unfavorable wall-coin split cancels it (thin identical-
   trajectory cells: rbsat-v1375 ~1746/1772s!, oski15 ~1600-1700s, bp4_TCO
   ~1700/1747s, sted2 ~1545/1558s, vex ~1490s). At a tie, tier-2 loses by
   62k (TT406's draw minus TT395's credit) → judged-trade territory.

## Current state

- HEAD: the banded-delta promotion commit (on top of f75e26b). Medium
  baseline: **70/100** (see lineage TSV below).
- Solver12 endgame surface now: SAT_ENDGAME (on), SAT_ENDGAME_TRIGGER (1),
  SAT_ENDGAME_PARTS ("rf"), SAT_ENDGAME_MIN_ARMED (100k),
  **SAT_ENDGAME_REPHASE_DELTA (decision-armed flat delta; const default
  ENDGAME_REPHASE_DELTA_DEC), SAT_ENDGAME_DELTA_SPLIT (500k),
  ENDGAME_REPHASE_DELTA (50k, yield-armed legacy)**.
- Gate artifacts: gate 1 unbanded `log/abtest-cand-vs-base-2026-07-23-17-47-22`
  (launch log `log/abtest-endgame-delta48k-launch-2026-07-23.log`), gate 2
  banded `log/abtest-cand-vs-base-2026-07-23-21-23-54` (launch log
  `log/abtest-endgame-delta48k-banded-launch-2026-07-23.log`).
- **New lineage TSV for future A/Bs:**
  `log/abtest-cand-vs-base-2026-07-23-21-23-54/cand/results.tsv` (70/100).
  Composition vs the morning 70-lineage: +TT496 (1097s deterministic),
  −rbsat-v1375 (pure wall coin: identical 6.26M-conflict trajectory took
  1716s in the morning deal, >1800s in this one). oski15 (1657s), sted2
  (1551s), TT406 (290s), sudoku (~1100s) all IN. bp4_TCO/TT492/TT495 out.

## RANKED PLAN for next session

1. **rbsat-v1375 re-entry / dense wall-coin hardening (NEW #1).** rbsat is
   now the lineage's lost cell — identical trajectory (6.26M conflicts,
   deterministic), wall 1716s-1800s+ depending on load. It does NOT need
   new capability, just ~5-10% wall. Two angles: (a) the rate gap below
   (#3 of the morning plan): rbsat runs 3.1x fewer conflicts/s than kissat
   — ANY throughput win banks it and oski15 (1657s) and sted2 (1551s) for
   good; a 10th wall-diet in the established series (watch-pool /
   closure-diet / round-diet / elim-scratch / hotloop-ptr lineage) is the
   proven gate-safe shape (conflicts EXACT tie, wall down). (b) accept the
   coin — but note THREE cells now sit in the 1550-1750s band; one more
   wall diet turns up to 3 coins into solid solves.
2. **TT495: verified NOBODY solves it** (we timeout at 13 deltas, kissat
   times out at 1800s). Next lever is not cadence: needs capability (e.g.
   walk/rephase interplay or inprocessing depth). Low priority standalone;
   revisit only with a new mechanism.
3. **Dense-margin-cell rate gap (rbsat 3.1x, oski15 4.9x conflicts/s vs
   kissat)** — biggest headroom number known; lever is learned-DB
   discipline (kissat reduce keeps fewer clauses -> shorter watch lists).
   Any change rerolls every >=1M-conflict trajectory: bundle with a
   deliberate re-luck campaign; use arming-time scoping where possible.
   Measure first offline: kept-clause counts + ticks/prop on rbsat under
   kissat tier limits (no gate). NOTE this is also the fix for the
   wall-coin fragility that decided both gates today — rbsat at
   1746/1772s of 1800 is one bad scheduler day from −1.
4. **Inprocessing cadence for never-armed slow cells** (goldcrest 474
   conf/s, lockchart 330 — never reach any trigger; kissat inprocesses
   time-based). Must spare sudoku-N30 + bp5 (never-armed but solving).
5. **Giant memory diet** (pj2008 RSS 10.4GB vs kissat 1.4GB; BVE emits
   1.7GB DRAT written+discarded in 150s) — proof-churn batching +
   occurrence-list peak. Unstarted, carried.
6. **Carried kissat gaps (unstarted):** elimination depth (72-88% vs our
   43-56% on circuit miters), SAT-sweeping productivity (known defect:
   `sweep_round` restarts its 512-seed scan at var 1 every round — bead
   SAT-playground-5b2.3.39), tiered vivification + probing/HBR parity.
7. **Grid n400 / XOR arc: CLOSED both checker modes — DO NOT REVISIT.**

## Standing traps (carried + new this session)

- `results.tsv` written only at run END — monitor per-cell `[cand]/[base]`
  lines in the launch log instead.
- **A `pgrep -f feature_ablation` inside a monitoring loop matches ITSELF
  (the pattern is in the loop's own command line). Use
  `ps aux | grep "[f]eature_ablation.py"` or match the python binary.**
- vex UNSAT checker-timeout is historical/symmetric load-lottery, NOT a
  gate failure (it verified CLEAN in both 2026-07-23 evening gates).
- Conflict counts are EXACTLY deterministic across load; wall is not.
  Digit-exact identity checks (round-5 pattern: yield-protection +
  passthrough + default-equivalence) are cheap and conclusive — use them
  for every scoped-reroll change.
- Wall-coin cells decide gates at tied trajectories: rbsat-v1375
  (1746/1772s), oski15a01b20s (1607-1700s), bp4_TCO_CSO_ZR (1698/1747s),
  sted2 (1545/1558s), vex (~1490s). Any of these can split either way per
  gate run. Tier-1 margins under ~120s are load noise, not capability.
- Arming times (candidate env, idle, re-confirmed): instantly-armed (=1):
  vex, oski15x2. ~200k: TT406 (200,057), TT492 (200,057 — IDENTICAL to
  TT406), TT395 (200,191), TT496 (200,013). ~800k: sqrt170 (800,269),
  sqrt171 (801,312), pancake (800,317), QG7 (801,822), aaai10 (800,445),
  oddball24 (807,695), div172 (800,438).
- SAT_STATS_JSON=on emits to STDERR; timed-out runs emit NO stats JSON.
  For arming stats on timeout cells use SAT_LIMIT_CONFLICTS (~400k works).
- No `cargo build` while a gate runs; single-instance rebuilds are safe
  (inode swap). `pgrep -a sat-solver` before gates. Heredoc scratch writes
  flake — use the Write tool.
- The A/B launcher's `cd` matters: cd to repo root before launching.

## solver12's capability edge (protect in rerolls)

xor_op x2, oddball_80_5, Kakuro-easy-132, MVRoundRobin_n16_d10, case1,
tseitin_n188_d3 (SAT_TSEITIN), SC25_Timetable_C_406 (endgame rf),
**SC25_Timetable_C_496 (endgame banded d48k — kissat CANNOT at 1800s;
unique capability, 1097s in-gate / 703s margin, deterministic)**.

## Where the evidence lives

- Probes (session scratchpad, gone next boot; durable numbers in this
  file): round1 = TT492/495/496 x {42k,25k} + kissat refs + TT492
  armed_at; round2 = delta grid 30-48k; round3 = d48k blast-radius (8
  cells); round4 = fine grid 44-52k; round5 = band identity (6 digit-exact).
- Gates: see Current state above.
- Prior aggregate: `plan/next-steps-AGGREGATED-2026-07-23.md` (morning rf
  session), `plan/next-steps-AGGREGATED-2026-07-22b.md`.
