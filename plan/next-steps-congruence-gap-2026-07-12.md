# Next steps after the congruence-inprocess promotion (2026-07-12, c579bfe)

Context for a fresh session. State as of this writing:

- Medium baseline: **62-63/100** (63 nominal; rbsat-v1375 is a 1745s/1800s coin-flip cell).
  Kissat 4.0.4 reference: **74/100** fresh run (`log/kissat-medium-20260705-203444`),
  80 on the older reference run. Gap ≈ 11-16 cells.
- Promoted at `c579bfe`: SAT_CONGRUENCE + SAT_CONGRUENCE_XOR default-on; congruence runs
  first in every inprocess round (kissat probe.c order); all-or-nothing dry-run merge
  threshold (3000, env `SAT_CONGRUENCE_MIN_MERGES`; 10M-clause cap); root-productive
  formulas (≥1000 root merges) switch to early doubling inprocess cadence (first round
  10k conflicts) + mid-search BVE rounds.
- Gate evidence: `log/abtest-cand-vs-base-2026-07-12-00-03-56` (PASS, 62==62 identical
  solved sets, both-solved conflicts −2.0%, ibm-2004 −74%). Negative control (no
  threshold): `log/abtest-cand-vs-base-2026-07-11-21-59-04` (59 vs 63 LOSE — every lost
  cell was a SAT formula rewritten for sub-threshold merges).

## The one transferable pattern

**Formula-rewriting passes must dry-run a productivity signal on the untouched formula
and bail edit-free below threshold.** Fragile SAT cells die from zero-payoff rewrites
(Timetables: 34k hidden binaries / 0 merges; Kakuro: pure-binary ELS rewrites). This is
what converted a −4-solved regression into a conflicts-tier win. Apply it to any future
inprocessing/preprocessing candidate.

## Ranked next-step ideas

### 1. Raw propagation/search throughput (biggest single lever, hardest)
Measured on VexRiscv: ours 717 conflicts/s vs kissat 10k/s (14x), ~2-4M props/s vs
kissat's ~10x more; 3250 props/conflict, 400 decisions/conflict, deep BMC trails
(level ~3864, trail 100-300k). The congruence×eliminate interleave is now default-armed
for miters/BMC — throughput is the remaining blocker on VexRiscv/oski/goldcrest/g2 (and
booth×2, Bubble, fixedbandwidth are pure conflict-volume/throughput cells: kissat needs
6.5-14M conflicts at 11-29k conf/s on them).
Concrete angles (mostly open beads under "Hot-path throughput and memory layout"):
- Watch-list layout: blocking literals hit rate, compact watcher structs, arena
  cache-locality after GC; bead 5b2.7 (literal-indexed i8 values) was a null result —
  don't re-run without a new mechanism.
- Profile a BMC cell end-to-end (`/analyzesat` skill; perf) — where do the 650M
  props/200k conflicts actually go? Suspect: huge watch lists on deep trails, arena
  fragmentation after 47M root resolvents (bead 5b2.3.23: no GC during eliminate).
- Measure props/s on both-solved cells vs kissat to size the global gap precisely.

### 2. Push the aggressive-inprocess mechanism further on productive formulas
Now that trajectory damage is gated off, the interleave itself can get stronger:
- **Substitute/ELS as a first-class round step** on productive formulas (kissat runs
  substitute twice per probe round). Currently ELS only fires inside congruence/sweep.
- **Mid-search factor** (inprocessing factor — kissat runs factor at the end of every
  probe round; ours is frontend-only, ≤10^4 vars). Needs fresh-var growth mid-search
  (resize var-indexed arrays). Memory note says this was the planned BVA follow-up.
- **Congruence fixpoint efficiency**: the 64-round whole-formula re-extraction loop
  spends 46s finding ~20 merges/round in the tail (VexRiscv). A kissat-style worklist
  (rehash only gates whose inputs merged) would cut round cost ~10x and allow higher
  cadence.
- **Sweep effort scaling**: our sweep env budgets are fixed (256 vars/1024 clauses/
  depth 2, 512 seeds); kissat scales to 8192 vars/32768 clauses/depth 3 on success and
  spends 10% of search ticks. Sweep found ~0 equivalences on the gap cells while
  kissat's found hundreds — the budget, not the algorithm, is the difference.
- **BVE strength on productive formulas**: our grow=0/clslim=20 vs kissat bound=16/
  clslim=100/occlim=2000 multi-round (open bead 5b2.3.35). Gating a stronger BVE on the
  same productivity signal avoids the trajectory-shuffle that killed prior attempts.

### 3. Structured-SAT search cells (Timetable×2, lockchart-group1, bp4_TCO — 4 cells)
Kissat wins Timetable_C_406 in 41s via 67% mid-search elimination + factor + walk
rephasing, only 170k conflicts. Our timetables are now protected (0 merges → untouched),
so attacking these needs a different productivity signal than congruence merges —
e.g., elimination-yield dry-run: try a bounded BVE probe at the first inprocess round;
if it would eliminate >X% of vars, enable aggressive elimination+factor for the rest of
the run. Walk/rephase was characterized as not-the-answer standalone (WalkSAT bead note,
reverted); kissat's walk is a rephasing assist, not a solver.

### 4. Giants (83aa/ee5 infeasible >16GB, pj2008 search-slow)
Memory note sat-medium-oom-memory-rewrite: the remaining +1 is a behavior-preserving
usize→u32 refactor of original_clause_ids/decision_level/etc (~0.6GB) so the reloc map
fits 00fd8ac — already PROVED solvable (121s@18GB). Mechanical, well-documented,
trajectory-safe. Probably the cheapest remaining +1 solved if it holds under the gate.
(pj2008 is fixed-for-OOM but search-bound; 83aa/ee5 need >>16GB, skip.)

### 5. Housekeeping / known traps
- The medium A/B arm syntax uses **commas**: `--arm 'cand:SAT_X=on,SAT_Y=on'`.
  Space-separated env specs silently create one invalid var → every cand cell dies
  with UNKNOWN_rc2 at 0s (two aborted runs learned this).
- `checker-timeout` on sqrt-mitern170's huge proof is a known benign verify artifact
  (symmetric across arms).
- rbsat-v1375 solves at ~1745s — treat ±1 solved swings involving it as noise.
- Restart/mode/phase constant parity levers (bead 2nr items a-f) are ALL exhausted
  LOSErs on medium; do not re-litigate without new mechanism evidence.
- Solved-count is a razor-sharp local optimum: wins come from capability additions
  whose edits are productivity-gated, never from trajectory perturbation.

## Where the evidence lives
- Gap bead: `SAT-playground-2a7` (fully updated 2026-07-12).
- Kissat verbose stats on gap cells: scratchpad (gone after reboot) but reproducible:
  `benchmarks/reference-solvers/kissat-latest/build/kissat -s -v <cnf>`.
- Dry merge counts per cell: in the 2a7 notes + c579bfe commit message.
- This session's A/B logs: `log/abtest-congrinproc-launch{,2,3}.log`,
  `log/abtest-congrinproc-v2-launch.log`, dirs `log/abtest-cand-vs-base-2026-07-11-*`
  and `log/abtest-cand-vs-base-2026-07-12-00-03-56`.
