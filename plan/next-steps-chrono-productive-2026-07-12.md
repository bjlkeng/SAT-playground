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
