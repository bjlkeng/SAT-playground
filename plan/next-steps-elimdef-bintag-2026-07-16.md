# Next steps after elim-def groundwork + binary-edge-tag promotion (2026-07-16)

Context for a fresh session. State as of this writing:

- Medium baseline: **68/100** in this session's final gate (rbsat-v1375, the
  documented ±1 coin-flip cell, landed IN for BOTH arms — strict superset of
  the 67/100 lineage, zero losses; treat the 68th as coin-flip until it
  repeats). Kissat 4.0.4 reference: 74/100
  (`log/kissat-medium-20260705-203444`). Gap ≈ 6-7.
- **PROMOTED 6633bc7: SAT_BINARY_EDGE_TAG default on** — binary-edge deleted
  tag. The binary hot loop no longer does a random 48-byte `BinaryClause` load
  per edge (deleted flag = tag bit 1<<31 in `BinaryEdge::clause_id`,
  maintained at BOTH `deleted = true` sites: `mark_binary_clause_deleted_for_
  clause` and the GC NO_RELOC drop) and no longer writes dead usage metadata
  (`used_count`/`last_used_conflict` — no functional readers; analysis-side
  marking kept). Off-switch `SAT_BINARY_EDGE_TAG=off` = byte-for-byte legacy
  path.
  Gate `log/abtest-cand-vs-base-2026-07-16-02-17-11`: PASS, WIN — 68==68
  solved, both-solved conflicts IDENTICAL 65,324,524 on every pair (the
  trajectory-neutrality proof held across all 100 cells), PAR-2 140,997.7 vs
  141,058.4. Idle paired walls: ibm −5%, oski40 −4.6%, vex −0.4%.
- **COMMITTED e7d149a (default-OFF groundwork): SAT_ELIM_DEF** — kitten-based
  semantic definition extraction in armed BVE (kissat definition.c port) +
  budgeted kitten (`solve_budgeted`) + a **kitten clausal-core soundness fix**
  (compute_core now expands learned clauses through recorded derivation
  antecedents; the old current-reasons-only walk produced non-refuting cores).
  Sweep is unaffected (consumes proof_lemmas, not cores), but ANY future
  core consumer needs this fix. Gate `log/abtest-cand-vs-base-2026-07-15-20-
  35-28`: LOSE 66 vs 67 → stays default-off.

## The elim-def story (measured; do not re-run blind)

Definition extraction WORKS mechanically: oski20's 3.5GB proof with 2,218
definition eliminations is drat-trim VERIFIED; oski20 solved standalone in
every def variant (1254-1561s) while its paired base TIMED OUT (>1750s).
The conflicts tier would have won (−164k both-solved; bp4 −158k, DLTM −148k,
sqrt-mitern170 −118k, Pancake −97k; ibm +366k roll). It is NOT promotable
because:

1. **oski40 is the counter-cell**: base solves ~989s idle; every def variant
   is slower (1358s best) and lost it in-gate. Root cause measured, twice:
   - First: re-check wall (947k kitten checks/run — fixed by the per-var
     occurrence memo, checks → 186k, kitten ticks → 2M total = negligible).
   - Then: **densification** — definition resolvents doubled the live arena,
     tripled learned-clause literals, 5.6x search ticks (+700s wall for −14%
     conflicts). A per-resolvent parent-length cap kills the bloat but also
     the yield (1,643 → 231 eliminations), and even 231 eliminations roll
     oski40's trajectory +589k conflicts (ibm-class variance).
2. The armed scope cannot hold oski20 and oski40 simultaneously — same
   family, opposite responses. This is the single-cell-variance wall from the
   worklist note, again.
3. Density class: definitions FOUND at 99% of checks (Bubble 19,245 found)
   but almost none convert under the resolvent bound (307 eliminated); no
   flip. TT492/lockchart: 0-found class (protected by the 20k-check adaptive
   cutoff). vex: 0-found, byte-identical (protected).

Salvage angles if revisited: kissat's `definitioncores` core REFINEMENT
(shuffle + re-solve to shrink cores → smaller gates → resolvents fit the
bound → conversion rate up — the main structural difference left vs kissat),
forward subsumption of resolvents during elimination, and mid-search factor
to recompress after definition collapse. All three attack the densification
directly instead of capping it.

## Session traps (additions)

- Kitten cores were WRONG for 6 weeks (sweep never consumed them) — when a
  new consumer reads sub-solver output, validate the output itself first
  (the wrong-SAT screens cost a full A/B).
- `SAT_STATS_HOT=1` + SAT_STATS_JSON produced truncated .err JSON in screens
  (only a factor line); the plain-JSON screens were fine. Not debugged.
- `pkill -f 'pat[t]ern'` still self-matches if the LITERAL bracket pattern
  appears in your own command line's launch args — kill by PID instead.
- vex ignored SAT_LIMIT_WALL_SEC=240 for 8+ min (wall checked between
  conflicts only; long parse + inprocess rounds) — known, but it also holds
  for SHORT wall probes.
- feature_ablation keeps only results.tsv per arm; per-cell JSONs live in the
  tmp dir and are cleaned — extract per-cell stats DURING the run or re-screen.

## Ranked next steps

### 1. lockchart-group1 (kissat 1336s SAT — profile NOW CAPTURED, first time)
`kissat -s` profile (this session): 396k conflicts, 39 dec/conf, 4.43G props
@ 3.3M props/s, eliminated 16%, factored 1,531, 18 rephases, congruence 0.
It is a raw-propagation cell (11k props/conflict); our 1750s screens reach
only ~265k conflicts. The wire needs either ~2x propagation throughput on
binary/long mixed scans (CSR watcher endgame, bead ck8, parity analysis in
the worklist note) or the rephase schedule finding the model earlier (our
walk/rephase machinery exists; lockchart decision-arms at 36.2 — check
whether rephases actually fire there and what best_permille reaches).

### 2. More trajectory-identical wall diet (the bintag pattern, repeatable)
The gate cannot lose on these (identical trajectories) and the wire cells
bank every second. Next candidates from the chrono-productive note's list:
watch-list blocker-hit locality, `c->searched`-style replacement caching
(irrelevant for BMC but cheap), arena prefetch tuning (exists: bead 5b2.8.1),
and dropping the dead `binary_dedup_seen` allocation everywhere (0.43GB on
giants — byte-identical trajectories, less RSS). Bundle several, prove
conflicts-identical on 5 cells, gate once.

### 3. oski20 margin (solves 1254-1561s standalone w/ def; 1430-1500s w/o)
Any further suite-wide speedup may flip it in-gate even without elim_def.
It sits with vex/rbsat/sted2 in the wire-cell set that motivates play #2.

### 4. Density class: elim-def core refinement (the honest continuation)
See salvage angles above — refinement is what kissat actually does that we
skipped, and the conversion-rate numbers (99% found / 1.6% converted) say
the cores are too big, exactly what refinement fixes.

## Where the evidence lives

- Bintag gate: `log/abtest-cand-vs-base-2026-07-16-02-17-11` + launch log
  `log/abtest-bintag-launch.log`; formal check output in the 6633bc7 commit.
- Elim-def gate (LOSE, documented): `log/abtest-cand-vs-base-2026-07-15-20-
  35-28` + `log/abtest-elimdef-launch.log`.
- Kissat lockchart profile: scratchpad `screens/kissat-lockchart.out` (dies on
  reboot — key numbers preserved above).
- Bead: `SAT-playground-2a7` (running gap-analysis log).
