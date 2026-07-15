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
