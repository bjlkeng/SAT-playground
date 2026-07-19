# Session notes: conflict-rate decomposition + unarmed eliminate flywheel groundwork (2026-07-18, PM)

State at session start and end: medium baseline **67-68/100** @ 5397939
(watchpool lineage; TT492 in). Kissat 4.0.4 reference: 74/100. **No gate was
spent** — the one implemented candidate was screened to a predicted gate FAIL
(conflicts-tier regression with no solved flip) before launch, per the
narrowing discipline. Committed: `SAT_ELIM_UNARMED_FLYWHEEL` default-OFF
groundwork (default path byte-identical by construction + sudoku 200k identity
screen; 653 tests, 9/9 smoke).

## THE headline measurement: props/s parity — the rate gap is formula size

Conflict-limited (SAT_LIMIT_CONFLICTS=200k/1M/2M) paired screens against
kissat 4.0.4 on the two conflict-rate gap cells, decomposing
conflicts/s = (props/s) x 1/(props/conflict):

- **g2, 200k→1M window**: ours 2,817 conf/s at 3.7M props/s and 1,310
  props/conflict; kissat 5,506 conf/s at 3.6M props/s and 645 props/conflict.
  **Propagation THROUGHPUT is at parity. The 2x (later 2.8x) rate gap is
  props/conflict**, i.e. formula size.
- **lockchart-group1, 200k conflicts**: ours 734.5s, kissat 691.4s (−6%);
  props/s 4.4M vs 4.4M — parity again; ours does +27% props/conflict. The
  "2.6x lockchart wall" in the previous notes is the walk-dominated SOLVE
  economics, not propagation speed.
- Mechanism behind kissat's g2 acceleration: its inprocessing collapses the
  irredundant DB 888k → 44.8k clauses (8% vars active) within the first 200k
  conflicts (eliminate rounds at ~500·n·log²n cadence with bound escalation,
  feeding on probe/sweep/substitute), and search accelerates 3.1k → 6.9k+
  conf/s as the DB shrinks. Ours does root preprocessing once (503k clauses)
  and NEVER inprocesses unarmed formulas before 1M conflicts
  (`inprocess_interval_conflicts = 1_000_000`), so the DB freezes and the rate
  decays.

**Consequences for the ranked list in the elimbounds note**: #1 "propagation
throughput on conflict-rate-bound cells" is DEAD as stated — there is no
per-visit hot-loop gap to close (measured at three conflict budgets on two
cells). #2 CSR watchers may still pay as a cache diet on the wall-lottery
cells, but NOT as a conflict-rate lever on g2/lockchart.

## Committed groundwork: SAT_ELIM_UNARMED_FLYWHEEL (default OFF)

Never-armed formulas past the first unarmed inprocess point (1M conflicts) run
bounded mid-search eliminate rounds every 100k conflicts with kissat-parity
COMPLETE-round bound escalation (0→1→2→4→8→16; `set_next_elimination_bound`)
on a bound counter SEPARATE from `armed_bve_bound`, with the extended gate
detectors (eq/AND-OR/ITE) active during flywheel rounds. Guards: density class
(dec/conf <= 3) excluded (protects yield-arm candidates QG7/Pancake where
escalation measured toxic), deep-phase excluded, armed excluded, two dry
rounds at max bound stop the schedule permanently. Every cell finishing
< 1M conflicts is byte-identical by construction.

Measured on g2 (2M-conflict paired screens):
- Baseline 786.8s; flywheel+gates **692.6s (−12% wall), window 1M→2M rate
  2,902 vs 2,279 conf/s (+27%)**, eliminations 416,892 → 451,944 (gates
  contributed 32.7k of the +35k).
- Escalation WITHOUT the gate detectors is worthless: +6.3k vars, clause DB
  larger, wall −1.6% (noise). Escalation with zero-yield-only rule (shipped
  armed default) stalls at bound 0 forever — same Bubble pattern as the
  elimbounds note.
- The decision-arm probe variant reached **90.7% vars eliminated** (462,120 of
  509,531 — beyond kissat's 86%) — depth is NOT the residual gap. The clause
  COUNT stays ~480k vs kissat's 45k: our eliminations reduce active vars but
  do not remove clause mass; kissat's collapse rides on the surrounding
  subsume/substitute/vivify ensemble cleaning the resolvent mass. That
  ensemble port (fast-cadence full rounds for the flywheel class) is the open
  follow-up, but see the gate-EV analysis before spending anything on it.

## Why NO gate was spent (pre-registered decision)

The flywheel's both-solved reroll surface is exactly TWO cells (the only
solved cells that exceed 1M conflicts unarmed, dec/conf > 3, not deep-phase):
- 59-129706 (SAT): knob-on re-solves but 574s/7.87M conf → 868s/11.03M conf
  on this seed (**+3.16M conflicts**).
- lockchart-group2 (SAT): 413s/1.239M → 421s/1.261M (+22k).

So the A/B starts ~3.2M in the red on the conflicts tier and can only pass by
flipping a timeout cell. Full-budget (1850s) knob-on screens of every
plausible flip candidate: **g2 no-flip** (22 flywheel rounds, dry-stopped at
3.15M conf; kissat itself needs 1758.9s of 1800 — g2 is effectively out of
reach for ANY single-session change), **pj2008 and goldcrest never reach 1M
conflicts** in budget (flywheel inert — they need a different lever),
**lockchart-group3 is density-class** (dec/conf 1.48, excluded). No flip ⇒
gate would FAIL lexicographically ⇒ not run. The knob stays default-off with
its evidence recorded.

## Other measurements this session (do not re-run blind)

- **Kissat gap set is 12 cells** (kissat-solves ∩ our-timeouts, vs watchpool
  base arm): Bubble 354s, oski20 617s, fixedbandwidth-eq-37 576s, TT406 41s,
  TT492 1052s (we have it — trade), bp4_TCO_CSO_IXA_LP_ZR 1287s, pj2008
  1165s, goldcrest 1234s, booth_wallace 1371s, booth_dadda 1389s,
  lockchart-group1 1687s, g2 1758.9s. Kissat's OWN wall on g2/lockchart-g1 is
  within 2-7% of the limit — those two are not realistic flips; the honest
  flip pool is Bubble/fixedbandwidth/TT406/oski20/booth.
- **Bubble sweep is NOT broken**: default sweep finds 85 equivalences at 2M
  conflicts (kissat: 100). The elimbounds note's "our closure finds ZERO" is
  congruence merges (0, kissat ~0 too). Bubble's kissat edge remains
  elimination/substitution ECONOMICS, already measured dead as bound knobs.
- **g2 sweep finds 66,460 equivalences** at the 1M unarmed round (kissat:
  2,090!) and applies them via ELS — yet the baseline still decelerates
  (window 2,279 conf/s after the round vs 2,817 before). Sweep+ELS alone does
  not stop the DB-size decay; whatever those merges free is not clause mass.
  Worth understanding before porting more of the kissat ensemble.
- Our root preprocessing on g2 is comparable-to-better than kissat's start
  state (503k clauses vs kissat's 888k at search start), and our per-prop
  chrono reason-scan cost is kissat parity (kissat_assignment_level scans
  reason clauses for long-clause propagations too; only binaries skip it).

## Traps (additions)

- TIMEOUT rows in the ablation TSVs carry zero conflicts/decisions — the
  UNKNOWN-cell class analysis (dec/conf guards etc.) needs standalone screens,
  not the TSV.
- `timeout <s> env ... sat-solver` kills the process before the stats JSON is
  emitted — use SAT_LIMIT_CONFLICTS (or in-solver wall limits) when the
  end-state stats matter.
- kissat progress lines: conflicts is $10, the $(NF-3) column is IRREDUNDANT
  CLAUSES (not variables) — the 888k→446k drops on g2 are clause counts.

## Ranked next steps (re-ranked on this session's evidence)

1. **Flywheel ensemble port** (the only mechanism-validated rate lever):
   fast-cadence vivify/subsume/sweep alongside the flywheel eliminate for the
   same guarded class, targeting clause-MASS removal (the 480k vs 45k
   residual). Success criterion before any gate: a both-solved >1M-conflict
   cell (59-129706) must not regress by more than the class wins elsewhere —
   otherwise the conflicts tier blocks the gate exactly as measured here.
2. **TT406 recovery** remains the cheapest +1 IF a TT-class stabilizer is
   found first (kissat solves it in 41s via early rephase/walk; our
   decision-arm fires at 200k+ conflicts at the earliest). Blocked on the
   stabilizer per the inlinebin/elimbounds traps — do not reroll blind.
3. **pj2008/goldcrest**: sub-200k conflicts in 1800s at props/s parity means
   they need either preprocessing collapse (their DB is huge from parse) or a
   memory-locality diet (CSR) — measure which before writing code.
4. **CSR watchers**: demoted from #2 to a wall-tier (PAR-2) diet play; the
   conflict-rate premise is dead (props/s parity).

## Where the evidence lives

- All screens: scratchpad `screens/` (dies on reboot) — every
  decision-relevant number is in this note.
- Kissat 200k/1M profiles for g2 + 200k for lockchart-group1:
  `g2-kissat.out`, `g2-kissat-1m.out`, `lock-kissat.out` in the same
  scratchpad; key numbers preserved above.
- Baseline for any future gate: `log/abtest-watchpool-vs-base-2026-07-18-01-51-12`.
