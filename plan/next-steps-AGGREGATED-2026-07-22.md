# AGGREGATED next-steps plan — 2026-07-22 (supersedes next-steps-AGGREGATED-2026-07-21c.md)

One-file plan for the next clear context. Folds the 2026-07-22 **closed-Tseitin
ER-refutation session** (bead SAT-playground-kk8) on top of the 2026-07-21c
aggregate (gap-read + inprocessing deep-dive). Where this contradicts an older
`plan/next-steps-*.md`, THIS file wins. Session detail:
`plan/next-steps-tseitin-2026-07-22.md`.

## TL;DR — what changed this session

1. **NEW CAPABILITY: closed-Tseitin component refutation with
   extension-variable DRAT proofs** (`src/gauss.rs`, default-on behind
   `SAT_TSEITIN`). Detects closed Tseitin XOR components (every var in exactly
   2 equations, union-find + odd charge) and emits a width-bounded
   extended-resolution summation proof (prefix-accumulator chains, pointer
   parking, deletion lines).
2. **tseitin_n188_d3: TIMEOUT → UNSAT in ~32 s** (universal timeout before —
   kissat 4.0.4 can't solve it either). Proof 4.71 M lemmas, drat-trim
   VERIFIED in 187 s idle. Expected **+1 solved** (top lexicographic metric).
3. **tseitin_grid_n12_m12: 5.65 s → ~1.5 s** (proof 1.26 M → 10.7 k clauses).
4. **tseitin_grid_n400_m400 stays a timeout BY CHOICE**: the engine proves it
   in 22 s (14.6 M lemmas) but backward drat-trim cannot verify that within
   the harness's 1800 s cap, and `checker-timeout` on UNSAT = gate
   correctness FAILURE. Capped via `TSEITIN_MAX_COMPONENT=20k` /
   `TSEITIN_MAX_EMIT=6M`; the cell keeps byte-identical baseline behavior.
5. Gate: `log/abtest-cand-vs-base-2026-07-22-14-52-12` (cand default vs
   `base:SAT_TSEITIN=off`): **PASS, WIN 70 vs 68 solved** — flips IN:
   tseitin_n188_d3 (engineered, UNSAT 44 s vs TIMEOUT; proof verified
   in-harness under load) + oski15a01b20s_opt (documented load-lottery
   flipper, UNSAT 1792.7 s — 7 s margin, NOT caused by the change).
   Both-solved conflicts EXACT tie on all 68 cells (0 differing) —
   trajectory-identity held perfectly. PAR-2 134611.0 vs 139761.9 (−5151).

## Current lineage state

- HEAD: the tseitin promotion commit on top of `81342c2` (SAT_TSEITIN
  default-on). **Medium baseline is now 70/100.**
- Lineage baseline TSV for the next A/B:
  `log/abtest-cand-vs-base-2026-07-22-14-52-12/cand/results.tsv` (70/100).
  NOTE it embeds the oski15 lottery luck (1792.7 s of 1800) — treat any
  future oski15 flip-out as load noise first.
- Reroll-luck law still in force: 68/69-lineage embeds banked wall-lottery
  luck; global trajectory rerolls are −EV. The tseitin engine is the model
  scoped shape: fires ONLY on cells that were universal timeouts; every other
  cell byte-identical.

## RANKED PLAN for next session

1. **Finish the Tseitin arc: crack tseitin_grid_n400_m400 verification
   (+1 solved sitting on the table).** The proof EXISTS (22 s generation,
   14.6 M lemmas, valid); only backward drat-trim's rate blocks it. Options,
   in order of promise: (a) shrink the proof ~3x — needs a structurally
   different derivation (per-row boundary-parity extension vars ≈ 50
   clauses/vertex ≈ 8 M lemmas; still marginal), current architecture is at
   its 2^(w-1) floor (91/step); (b) profile drat-trim's 3x rate degradation
   on the grid vs n188 (1.1 M vars incl. fresh + 1.27 M live originals early
   in the proof) and find a proof shape that avoids it (e.g. delete originals
   sooner, reorder components); (c) if the harness verification command is
   ever fair game (NOT this goal's rules — .rs only), forward mode `-f` or
   LRAT would verify in minutes.
2. **Inprocessing capability arc (unchanged from 21c, still the big
   structural gap):** (a) make SAT sweeping kitten-productive — solver12
   sweep finds 0–826 facts vs kissat's 90 k–18 M kitten solves; note
   `sweep_round` restarts its 512-seed scan at var 1 every round (no
   persistent cursor) — an obvious first defect; (b) tick/time-budget the
   inprocessing cadence — goldcrest (474 conf/s) and lockchart (330 conf/s)
   NEVER reach the 1 M-conflict trigger in 1800 s (zero inprocessing);
   (c) deepen gate-aware elimination + equivalence substitution (kissat
   72–88 % elim vs our 43–56 % on circuit cells). CAUTION: all three reroll
   the ≥1 M-conflict solved cells (rbsat 6.2 M/1749 s, sted2 4.4 M/1652 s,
   TT492 3.7 M/1468 s live in the danger zone) — the last two capability
   rerolls FAILED gates (SAT_SEARCHED, twice). Scope so already-solved cells
   stay byte-identical, or bundle with a re-luck plan.
3. **Other XOR/parity-shaped universal timeouts?** The tseitin win pattern
   (find cells NOBODY solves where solver12 has/can-build unique capability,
   scoped to fire only there) is the cheapest +1 shape. Candidates to
   examine: rphp5_050/085 (relativized pigeonhole — needs symmetry/counting,
   no DRAT-cheap proof known), ramsey/clqcl (combinatorial, hard),
   tseitin_n188-class cells in other suites. Also: does any timeout cell
   have a large closed-Tseitin SUBcomponent (odd charge) even if not pure?
   Detection already handles mixed formulas (only needs the XOR subsystem).
4. **10th wall diet** (the 9-for-9 promotable shape) — no candidate sink
   identified this session; would need fresh profiling. The remaining known
   sink: giant memory diet (pj2008 RSS 10.4 GB vs kissat 1.4 GB;
   clause_abstraction/binary_id/occurs hogs) — trajectory-safe, helps the
   16 GB cap and oski15-class OOM flips.
5. **Tiered vivification port / probing+HBR** (kissat parity, from 21c #4/#5)
   — unchanged, unstarted.

## Standing traps (carried forward + new)

- `results.tsv` written only at run END — monitor from the launch log
  per-cell lines (`  [arm] instance/s0 RESULT ...`); completion = "DONE ->".
- checker-timeout / missing proof on an UNSAT cell = gate correctness FAIL
  (compare_bench.correctness_failures) — an unverifiable proof is WORSE than
  a timeout. Budget ~≤6 M lemmas for backward drat-trim under the 1800 s cap
  (~8–25 k lemmas/s measured).
- drat-trim facts (new, load-bearing): RAT pivot = FIRST literal of the
  emitted line (it sorts afterward); deletions match on sorted literals;
  duplicate additions coexist as separate copies; backward mode ignores
  deletion of (pseudo) unit clauses; `checkRAT` scans the whole watch space
  per RAT lemma but only CORE lemmas get checked (n188: 796 of ~120 k def
  clauses in core).
- `combine_rows` derives nothing for variable-disjoint operands — any
  summation derivation must keep operands connected, or use the
  disjoint-direct-RUP emission (valid because falsifying a sum-row clause
  assigns all vars of both operands).
- extract_xors output is now sorted by min member clause index (HashMap
  iteration order was nondeterministic → proof size/time varied per run).
- SAT_STATS_JSON needs `=on`; SAT_LIMIT_WALL_SEC for window measurements;
  perf unusable (perf_event_paranoid=4); dcg blocks `rm -rf` and $HOME
  redirects; no `cargo build --release` while a gate runs; heredoc scratch
  writes flake (use the Write tool); watch cwd — `cargo build` from the
  wrong dir silently leaves a stale binary (bit twice this session).
- 32-way at 16 GB/job preflight-warns vs 502 GB RAM; cap not reservation.
- Stray long-running sat-solver processes from dead sessions: check
  `pgrep -a sat-solver` before gates (killed an 18 h orphan this session).

## solver12's capability edge (protect in rerolls)

xor_op ×2 (Gauss + pair-abs), oddball_80_5, Kakuro-easy-132,
MVRoundRobin_n16_d10, case1 — kissat cannot solve any in 600 s uncontended.
NOW ALSO: tseitin_n188_d3 (+ n12 speedup) via SAT_TSEITIN. Do not trade away.

## Where the evidence lives

- This session: `plan/next-steps-tseitin-2026-07-22.md`, bead
  SAT-playground-kk8, gate `log/abtest-cand-vs-base-2026-07-22-14-52-12`.
- Gap-read / inprocessing deep dive: `plan/gap-read-2026-07-21.md`,
  `log/gap-read-2026-07-21/deepdive/COMPARISON.txt`.
- Prior aggregate (superseded but valid provenance):
  `plan/next-steps-AGGREGATED-2026-07-21c.md`.
- Beads: SAT-playground-kk8 (tseitin, this session),
  SAT-playground-5b2.3.50 (cadence redesign — plan #2b),
  SAT-playground-5b2.3.39 (congruence — feeds plan #2c).
