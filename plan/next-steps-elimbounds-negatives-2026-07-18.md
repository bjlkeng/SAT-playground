# Session notes: elimination-bound / rounds / lit-values / congruence-learned / unarmed-elim screens — SEVEN MEASURED NEGATIVES (2026-07-18)

State at session start and end: medium baseline **67-68/100** @ 6199fb2/8018782
(watchpool promotion; TT492 in, rbsat/TT406 the lottery cells). Kissat 4.0.4
reference: 74/100. Gap ≈ 6-7. **No A/B was spent this session** — every
candidate died in standalone screens first (the narrowing discipline worked;
six mechanisms measured dead for the price of zero gates).

Committed this session (default-off groundwork + instrumentation, default path
proven byte-identical on ibm 100k-conflict identity screen — conflicts/
decisions/props/ticks exactly equal, 652 tests + 9/9 smoke in both knob modes):

- **Elimination rejection instrumentation** (always-on counters, trace-gated
  print): `elim_round attempted= eliminated_total= reject_count_bound=
  reject_clslim= reject_defcap= reject_budget= bve_grow= clslim=` line under
  `SAT_TRACE_PREPROCESS_DETAILS=1` at the end of every `eliminate()` call.
  This is how every finding below was measured; keep it.
- `SAT_ELIM_BOUND_COMPLETE` (yield-armed scope) and
  `SAT_ELIM_BOUND_COMPLETE_DECISION` (decision-armed scope): kissat
  `set_next_elimination_bound` parity — escalate the armed BVE bound after
  every COMPLETE round (effort budget not exhausted) instead of the shipped
  zero-yield-only rule. Both DEFAULT OFF (measured losers, below).
- `SAT_ELIM_ARMED_CLSLIM` (armed-round resolvent length limit override,
  default 0 = shipped clslim 20): measured ~inert on Bubble (+3 eliminations).
- `SAT_ELIM_DEF_NOCAP` (drop the definition parent-length cap, kissat parity):
  measured useless — the count bound rejects the same vars instead.
- `SAT_CONGRUENCE_LEARNED` (gate extraction also scans learned clauses ≤ 3
  lits): measured byte-identical on vex — dead hypothesis, see below.

REVERTED (implemented, screened, removed from the tree): `SAT_LIT_VALUES`
lit-indexed value mirror — see negative #5; the off-path branch in `lit_value`
(the hottest function) was not worth carrying for a measured-negative knob.

## The six negatives (do not re-run blind)

### 1. Bound escalation does not flip the density class
Mechanism find (real, kept in the knob docs): kissat escalates its elimination
bound 0→1→2→4→8→16 after every complete round regardless of yield
(`set_next_elimination_bound` + `try_to_eliminate_all_variables_again`); our
armed schedule escalates only after ZERO-yield rounds. On Bubble that stalls
our bound at 0 forever — armed rounds go +72/+3/+1/+0 eliminations while
~1,430 candidate vars/round are rejected on the resolvent-count bound
(reject_count_bound ≈ 96% of rejections; clslim ≈ 1%). With escalation on,
elimination depth goes 43% → 60-61%… and the cell still does not flip:
Bubble UNKNOWN @1790s in every combo (escalation alone / +clslim100 /
+elim_def+nocap all plateau ~2,088-2,142 of 3,492 vars, ~15M conflicts;
kissat refutes at 6.5M with 72% eliminated + 9% substituted + 100 sweep
equivalences). booth_wallace same shape (no flip, 13.2M conflicts @1790s).
Kissat's Bubble refutation edge is substitution/sweep machinery (314+100
vars) our closure finds ZERO gates for — architecture, not a bound knob.
Definition conversions specifically: comboC (def-nocap) proved the parent-
length cap was NOT the def blocker — count-bound rejects grew by exactly the
old defcap count (+909 ≈ 910). Definitions do not convert on Bubble under
any cap setting; consistent with the 07-16 defcores negative.

### 2. Yield-armed escalation is a conflicts-tier LOSER on the solved cells
QG7 paired: 2,001,134 vs 1,972,662 conflicts (+1.4%), wall 736 vs 716s.
Pancake paired: 5,681,692 vs 2,890,977 conflicts (+96%!). The deeper
elimination densifies (resolvents = added clauses) and derails the
refutation trajectories that the vivify-yield arming had improved. Do not
put SAT_ELIM_BOUND_COMPLETE=on through a gate.

### 3. Decision-armed escalation trades TT406 for TT492 (the class trade, again)
TT406: base UNKNOWN @5.33M conf → cand SAT @1.99M (recovered!). TT492: base
SAT 1098s @3.73M → cand UNKNOWN @4.48M (lost). Exactly the
inlinebin-note prediction: the decision-armed Timetable class holds 2-3
solvable cells and rerolls trade them. Net 0. SAT_ELIM_BOUND_COMPLETE_DECISION
stays off; only worth revisiting with a class stabilizer.

### 4. SAT_INPROCESS_ROUNDS=2 cannot hold oski20 and oski40 simultaneously
The one tantalizing number of the session: **oski20 1278s vs base 1581s
(−19% wall) at near-identical conflicts** (2.671M vs 2.664M, +0.3%) — a
mechanistic formula-shrink win, would likely flip oski20 in-gate. But paired
screens on the other wire cells kill it: oski40 941s → 1340s (+42%, would
lose its in-gate solve), TT492 SAT 1098s → UNKNOWN (lost), ibm 158s → 178s
(+13% wall, conflicts −93k). Same-family opposite response = the documented
single-cell-variance wall (elimdef session); no honest scoping signal
separates oski20 from oski40. vex: conflicts −245k but wall +90s (+7%).

### 5. Lit-indexed value array (kissat values.h parity) is a wall LOSER here
Full implementation (mirror maintained at every assignment write site,
identity proven byte-equal on ibm/Bubble/TT406/lockchart, 652 tests knob-on):
paired idle walls on identical trajectories — lockchart +5.7%, ibm +1.5%,
Bubble/TT406 neutral. The var-indexed `assignment` array (n bytes) beats the
lit-indexed layout (2n bytes) on cache footprint for exactly the big-arena
cells the diet was meant to help; LLVM already compiles the legacy sign-branch
chain branchlessly. REVERTED from the tree. Do not re-implement without a
changed memory-hierarchy story.

### 6. Learned-clause gate extraction changes NOTHING on vex
`SAT_CONGRUENCE_LEARNED=on` (extraction scans learned ≤3-lit clauses): vex
conflicts byte-identical 2,975,066, merges identical 18,360; the extra
learned-clause gates (+78 AND, +268 ITE patterns found) produce zero new
merges. The vex merge freeze (18.4k vs kissat 183k) is NOT input starvation.
oski20 rolled (+228k conf, −85s wall — lottery). Knob kept default-off for
provenance; the freeze mechanism remains unexplained — kissat's closure/
substitution creates genuinely different circuit structure, or counts merges
differently. Next honest step there: diff kissat closure.c merge accounting
vs ours on a small miter.

### 7. SAT_ELIM_UNARMED (mid-search eliminate for unarmed formulas) — marginal + taxed
Implemented after the g2 profile below suggested elimination starvation; the
knob extends the armed-round eliminate gate to unarmed inprocess rounds
(default OFF, committed). Measured:
- The starvation theory was WRONG at root: g2's ROOT BVE already eliminates
  416,892 of 509,532 vars (82%; kissat 88%). The 4 unarmed mid-search rounds
  added only +6,076 vars (1.2%). g2 stays UNKNOWN at 4.08M conflicts/1790s;
  the residual gap is conflict RATE (kissat 11.36M conflicts at 8.9k/s vs our
  2.3k/s) — propagation throughput, not elimination.
- Safety is real: ALL FIVE fragile canaries byte-identical conflicts (mp1
  336,333 / velev 782,238 / 544707 241,644 / case9 431,668 / sudoku 612,825)
  — the deep-phase guard plus zero-yield rounds leave trajectories exactly
  intact.
- Cost is real: identical trajectories but sudoku +9.3% wall, velev +4.7% —
  fruitless eliminate rounds pay the occurrence-index rebuild every 2k
  conflicts. Any revival needs kissat's variables-based skip (do not re-run
  until eliminable candidates change) or zero-yield backoff.
Not gate-worthy: no flip anywhere, PAR-2 tax on unarmed solved cells.

## Other measurements this session

- **lockchart walk-effort**: base(50‰) no longer solves at 300k conflicts
  (2112s idle — the old 270k solve is confirmed GONE since the inline-bin
  reroll, as the watchpool note recorded). SAT_WALK_EFFORT=25 SOLVES at
  260,576 conflicts / 2613s idle (walk #2 finds the model, 221.7M steps) —
  but 2613s idle ≈ ~3000s in-gate ≫ the wire; it is a lottery reroll, not an
  economics win, and it rerolls all decision-armed cells. effort=6 does not
  solve. The 2.6x-wall problem stands.
- **kissat g2 profile (NEW — the never-analyzed gap cell)**: UNSAT 1273s,
  11.36M conflicts, **eliminated 88% of variables (448,961)**, congruence
  ~zero (771 matched — consistent with our 0 merges), substituted 2%, sweep
  54% of kitten solves. g2's kissat mechanism is massive repeated elimination
  + sweep. Our g2 NEVER ARMS (0 merges, and BMC dec/conf won't decision-arm)
  → zero mid-search elimination ever runs there. The structural delta:
  kissat inprocesses EVERY formula on its interval schedule; we only
  inprocess armed cells. Broad unconditional inprocessing has documented
  toxicity (mp1 27s→600s under forced rounds), so the play is a
  low-aggression universal eliminate cadence (kissat-parity intervals,
  NOT the aggressive bundle) — screen mp1/544707/case9/velev first.
- Kissat Bubble reference numbers (fresh, idle): UNSAT 295s, 6.53M conflicts,
  eliminated 2,506 (72%), substituted 314 (9%), sweep_equivalences 100,
  eliminate_resolutions only 1.27M, 22 elimination rounds.

## Ranked next steps (updated)

1. **Propagation throughput on the conflict-rate-bound cells** — now the
   single best-evidenced gap: g2 2.3k vs kissat 8.9k conflicts/s (2.8x, and
   MEASURED this session with elimination depth equalized at 82-88%),
   lockchart 2.6x, pj2008/goldcrest same class. The remaining structural
   deltas vs kissat proplit.h are small per-visit costs; perf is blocked on
   this host (perf_event_paranoid=4), so the next session should use
   /analyzesat's ablation-based decomposition or targeted counter
   instrumentation (per-visit loads) before writing any code. SAT_ELIM_UNARMED
   was the last cheap knob; see negative #7.
2. **CSR/merged long-clause watcher layout** (unchanged from watchpool note)
   — the remaining big wall lever for oski20 (1430-1581s idle, needs
   10-20% in-gate) and the propagation-bound cells (lockchart/goldcrest/
   pj2008). Multi-session; trajectory-parity minefield documented in the
   inlinebin note.
3. **Density class substitution machinery** — kissat's 9% substituted + 100
   sweep equivalences on Bubble vs our zero. Requires understanding WHY our
   sweep/ELS find nothing there (sweep runs? finds nothing? never runs?).
   Instrument first.
4. **vex merge-freeze mechanism** — see negative #6; closure.c accounting
   diff on a small miter.
5. **TT-class stabilizer** — before any knob that rerolls decision-armed
   cells, find what makes the TT406/TT492 walk lottery converge (warmup was
   measured destructive; effort=25 lockchart lottery suggests walk-count
   sensitivity). Until then, treat every decision-armed reroll as −EV
   because TT492 is currently IN.

## Traps (additions this session)

- The elim_round trace counters are CUMULATIVE across rounds (per-run), not
  per-round — diff consecutive lines for per-round yields.
- A conflict-limited (SAT_LIMIT_CONFLICTS) run that would have solved within
  the limit still reports `s UNKNOWN` if the limit hits first — lockchart
  base "no solve at 300k" means the solve point moved past 300k, not that
  search broke.
- 3-arm simultaneous feature_ablation runs change the contention profile vs
  the standard 2-arm gate; wall-lottery cells (sted2, rbsat) will not
  reproduce. Stick to 2-arm gates.
- The `/usr/bin/time -v` + `SAT_STATS_JSON=1` combo puts the stats JSON in
  .err; `result.json` does NOT carry conflicts (schema is contract-only).

## Where the evidence lives

- All screens: scratchpad `screens/` (dies on reboot) — every
  decision-relevant number is in this note.
- Kissat reference profiles: `bubble-kissat.out`, `g2-kissat.out` in the same
  scratchpad; key numbers preserved above.
- No gate was run; no gate logs to cite. Baseline remains
  `log/abtest-watchpool-vs-base-2026-07-18-01-51-12` (watchpool gate 3).
