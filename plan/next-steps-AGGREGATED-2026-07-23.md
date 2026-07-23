# AGGREGATED next-steps plan — 2026-07-23 (supersedes next-steps-AGGREGATED-2026-07-22b.md)

One-file plan for the next clear context. Folds the 2026-07-23 endgame-rf
promotion session on top of the 22b aggregate. Where this contradicts an older
`plan/next-steps-*.md`, THIS file wins.

## TL;DR — what changed this session (PROMOTED f75e26b)

**SAT_ENDGAME late-armed rf cadence is now the default. TT406 first-ever
solve. Gate WIN 70 v 69.** The 22b plan's ranked #1 (TT-class re-deal via
bundle decomposition) executed exactly as scoped, in one session:

1. **Decomposition (SAT_ENDGAME_PARTS, idle, from-arming):** flat-50k rephase
   (`r`) CARRIES the TT406 flip (r alone: SAT 904s/3.15M conf); restart floor
   (`f`) alone does NOT (timeout); together (`rf`): **SAT 155s idle / 212s
   in-gate / 593,251 conflicts** (baseline: 5.16M conflicts and still timeout).
   Pinned-10k inprocess (`i`) is BOTH unnecessary for the flip AND the killer
   of the dense victims (sqrt170 ~40% slowdown) — now default-off.
2. **The arming-time discriminator (the session's key mechanism find):**
   every reroll victim arms INSTANTLY (`inprocess_armed_at_conflict == 1`:
   vex, oski15a01b20s/40s — and they carry banked lottery wins), while every
   winner/survivor arms LATE (TT406/TT395 decision-armed at ~200k; sqrt170/
   171, QG7, Pancake, aaai10 yield-armed at ~800k). SAT_ENDGAME_MIN_ARMED
   (default 100k) latches the endgame only for late-armed cells; instantly
   armed cells stay bit-identical (vex verified EXACT: 2,975,066 conflicts,
   idle and in-gate).
3. **Late-armed rf survivors, all measured idle:** TT395 173s (lineage 205s),
   sqrt171 333s (400s), pancake 387s (453s), aaai10 611s (908s), QG7 722s
   (995s), sqrt170 747s/3.92M conf (1166s/3.89M). Most FASTER under rf.
4. **The judged trade: −TT492 (+TT406).** TT492 is TT-family and therefore
   late-armed — the rf reroll kills its banked SAT draw (timeout idle AND
   in-gate; baseline had it at 1434–1468s of 1800, a ~350s-margin coin-flip).
   Traded for TT406 at 212s deterministic with ~1590s margin. Formal gate:
   cand 70/100 v base 69/100, zero contradictions, zero correctness failures,
   PAR-2 −3305.9. Judged explicitly per the session goal: good trade.
5. **tt495/tt496 do NOT flip under rf** (timeout in unscoped rf probes).
   BubbleVsPancake also still times out (yield-class, if it arms at all).

## Current lineage state

- HEAD: f75e26b. Medium baseline **70/100** — the NEW lineage TSV for future
  A/Bs: `log/abtest-cand-vs-base-2026-07-23-11-35-01/cand/results.tsv`
  (composition vs old 70-lineage: +TT406, −TT492; oski15a01b20s solved this
  deal at 1607s — still THE coin-flip cell; rbsat-v1375 1716s, the other one).
- Solver12 endgame surface: SAT_ENDGAME (on), SAT_ENDGAME_TRIGGER (1 = at
  arming), SAT_ENDGAME_PARTS (default "rf"), SAT_ENDGAME_MIN_ARMED (100k).
  Stats: `inprocess_armed_at_conflict`, `endgame_at_conflict`.

## RANKED PLAN for next session

1. **TT492 re-entry (the obvious +1 target).** Its SAT draw exists on the
   shipped (pre-rf) trajectory; late-armed scoping can't spare it (it arms at
   ~200k like TT406). Paths:
   (a) Measure its exact armed_at + endgame behavior (idle, candidate
       defaults + SAT_STATS_JSON, 1800s — it will timeout; the stats only
       need the arming point). If it arms meaningfully LATER than TT406
       (e.g. >250k), a MIN_ARMED band (100k..X) could separate them — check
       TT395 (200,191) stays inside.
   (b) Longer rf runway: TT406 solved at 593k conflicts total; TT492 needed
       3.73M shipped. If rf compresses its search similarly it may solve
       under a DIFFERENT draw — the current in-gate timeout says the first
       rf draw missed. A rephase-cycle variant (delta 42k = kissat's TT
       measured walk interval, or 25k) is a cheap idle sweep on TT492+TT406
       +TT495+TT496 before any gate.
   (c) Accept and move on — TT406's margin is structural, TT492's never was.
2. **TT495/TT496 (two more TT-family timeouts).** Kissat solves them? NOT
   verified this session — measure kissat 4.0.4 on both (600s idle) first.
   If kissat solves them, the gap is again cadence-shaped; try the rephase
   -delta sweep from 1(b) on them simultaneously.
3. **Dense-margin-cell rate gap (rbsat 3.1x, oski15 4.9x conflicts/s vs
   kissat)** — biggest headroom number known; lever is learned-DB discipline
   (kissat reduce keeps fewer clauses -> shorter watch lists). Any change
   rerolls every >=1M-conflict trajectory: bundle it with a deliberate
   re-luck campaign, and use the arming-time scoping trick where possible.
   Measure first: kept-clause counts + ticks/prop on rbsat under kissat tier
   limits, offline A/B (no gate).
4. **Inprocessing cadence for never-armed slow cells** (goldcrest 474 conf/s,
   lockchart 330 — never reach any trigger; kissat inprocesses time-based).
   The endgame census machinery + `inprocess_armed_at_conflict` make the
   solved-set audit cheap. NOTE: sudoku-N30 and bp5 also never arm (measured
   this session: armed_at=0) yet solve — any never-armed cadence must spare
   them (sudoku 1132-1165s margin is real but not fat).
5. **Giant memory diet** (pj2008 RSS 10.4GB vs kissat 1.4GB; BVE emits 1.7GB
   DRAT written+discarded in 150s) — proof-churn batching + occurrence-list
   peak. Unstarted, carried from 22b.
6. **Carried kissat gaps (unstarted, all trajectory-coupled):** elimination
   depth (72-88% vs our 43-56% on circuit miters — substitution/definition
   interplay, bounds are dead), SAT-sweeping productivity (kitten 90k-18M
   solves/run vs our 0-826 facts; known defect: `sweep_round` restarts its
   512-seed scan at var 1 every round — bead SAT-playground-5b2.3.39), tiered
   vivification + probing/HBR parity. Scope any of these with the arming-time
   discriminator + armed-census machinery.
7. **Grid n400 / XOR arc: CLOSED both checker modes — DO NOT REVISIT** (22b
   #6 has the full post-mortem; forward AND backward drat-trim exceed even
   the 2x/3600s budget on the 14.63M-lemma proof; derivation floor ~12M).

## Standing traps (carried + new)

- `results.tsv` written only at run END — monitor per-cell lines in the
  launch log.
- **vex UNSAT is verify=checker-timeout in EVERY gate (all four arms across
  2026-07-22/23 gates) and always has been — it is symmetric load-lottery,
  NOT a correctness failure; check_promotion_gate passes it. Do not panic.**
- checker-timeout budget: drat-trim gets 2x solver wall (3600s); backward
  budget ~<=6M lemmas under load.
- Conflict counts are EXACTLY deterministic across load; wall is not.
  Conflict-count triggers scoped by arming time are provably reroll-free for
  the protected set (vex reproduced digit-for-digit under the new default).
- Arming times measured 2026-07-23 (candidate env, idle): instantly-armed
  (=1): vex, oski15a01b20s, oski15a01b40s. ~200k: TT406 (200,057), TT395
  (200,191), TT492 (inferred, TT-family). ~800k: sqrt170 (800,269), sqrt171
  (801,312), pancake (800,317), QG7 (801,822), aaai10 (800,445). Never
  (armed_at=0): sudoku-N30, bp5, TT392/TT393 (too few conflicts).
- Pinned 10k-conflict inprocess measured toxic on dense refutation cells
  (~40% slowdown) — that's why part 'i' is default-off; don't resurrect it
  without new evidence.
- SAT_STATS_JSON needs `=on`; timed-out runs emit NO stats JSON (tt492 probe
  taught this again — don't infer arming class from a missing stat).
- No `cargo build` while a gate runs; single-instance rebuilds are safe
  (inode swap). `pgrep -a sat-solver` before gates. Heredoc scratch writes
  flake — use the Write tool. 32x16GB preflight-warns vs 502GB RAM; cap not
  reservation.
- The A/B launcher's `cd` matters: run.sh resolves relative to the solver
  dir (the run.sh-not-found trap hit AGAIN this session in a compound
  background command — cd inside each subshell).

## solver12's capability edge (protect in rerolls)

xor_op x2, oddball_80_5, Kakuro-easy-132, MVRoundRobin_n16_d10, case1,
tseitin_n188_d3 (SAT_TSEITIN), **SC25_Timetable_C_406 (endgame rf — kissat
does this one too, in 31s; ours is 212s in-gate)**.

## Where the evidence lives

- Gate: `log/abtest-cand-vs-base-2026-07-23-11-35-01` + launch log
  `log/abtest-endgame-rf-launch-2026-07-23.log`.
- Probes (session-scoped scratchpad, gone next boot; durable numbers all in
  this file): round1 = TT406 r/f/rf + sqrt170/pancake rf; round2 = 12-cell
  victim/bonus sweep (the arming-time table); round3 = vex identity + TT406
  regression; default-equivalence = TT406 + TT393 exact.
- Prior aggregate: `plan/next-steps-AGGREGATED-2026-07-22b.md`.
