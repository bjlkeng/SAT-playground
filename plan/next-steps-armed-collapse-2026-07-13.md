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
