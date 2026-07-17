# Next steps after the hotloop-diet promotion + defcores groundwork (2026-07-16 evening)

Context for a fresh session. State as of this writing:

- Medium baseline: **67-68/100** (67 in this session's gate — rbsat-v1375, the
  documented ±1 coin-flip cell, timed out in BOTH arms; the 68-lineage from
  ce8ad28 is otherwise intact). Kissat 4.0.4 reference: 74/100
  (`log/kissat-medium-20260705-203444`). Gap ≈ 6-7.
- **PROMOTED d23e454: SAT_HOTLOOP_PTR default on** — pointer hot-loop diet
  (third instance of the repeatable trajectory-identical pattern after bintag
  6633bc7): binary-edge slice hoisted out of the per-edge loop (the `&mut self`
  calls were blocking LLVM from hoisting 3 dependent loads + a bounds check per
  edge), long-clause literals through a once-validated unchecked window,
  search-tick accounting batched into one add per scan, dead
  `binary_dedup_seen`/`binary_dedup_stamp` allocation removed (0.43GB on
  giants). Off-switch `SAT_HOTLOOP_PTR=off` = byte-for-byte legacy loop.
  Gate `log/abtest-cand-vs-base-2026-07-16-18-24-39` (launch
  `log/abtest-hotloop-launch.log`): PASS, WIN — 67==67, both-solved conflicts
  IDENTICAL 59,066,634 on every pair, PAR-2 144,148.2 vs 144,404.4 (−256).
  In-gate wire-cell walls: **oski40 1351s vs 1425s (−5.2%)** at identical
  conflicts — contention amplifies the memory-traffic savings; idle paired
  screens showed only −1% (ibm) / −2.9% (TT406) / ~0 (small-formula cells).
  Do NOT judge this diet class by idle screens; the in-gate PAR-2 is the signal.
- **GROUNDWORK (inert): SAT_ELIM_DEF_CORES=2** — kissat `definitioncores`
  refinement parity inside `detect_kitten_definition` (fresh kitten over core
  clauses only, deterministic splitmix shuffle of var numbering + clause order,
  10x budget re-solve, ABORT-parity on exhaustion, ticks charged to the
  eliminate budget). Active only under default-off SAT_ELIM_DEF.

## Measured this session (do not re-run blind)

1. **Core refinement does NOT fix the density-class conversion rate.**
   At 2M conflicts, cores=1 vs cores=2 trajectories and eliminations are
   IDENTICAL: oski40 87,574 found / 230 eliminated both ways (2,295/87,576
   refine solves shrank a core; zero conversion flips); Bubble 2,818 found /
   1 eliminated, **0 cores shrunk**. Our antecedent-expanded round-1 cores are
   already near-minimal. The elimdef-bintag note's salvage hypothesis ("cores
   too big, refinement fixes it") is DEAD. The conversion bottleneck is the
   resolvent/occurrence bound with already-minimal gates — if elim-def is
   revisited, instrument WHICH bound rejects (resolvent count vs clslim vs
   occlim) per found definition, and compare against kissat's exact
   elimination accounting (kissat clslim=100 vs our 20 is the most suspicious
   delta; ELIM_DEF_MAX_ENV_CLAUSES=64 vs kissat occlim=2000 next).
2. **lockchart-group1 SOLVED standalone, first time ever**: `s SATISFIABLE`
   at **270,451 conflicts** (kissat needs 396k — our trajectory is BETTER on
   conflicts), model validated against all 3,410,378 clauses. Default config;
   decision-arm bundle did the work (factor 2,075 fresh vars, 7 rephases,
   2 walks, walk_improved=2). Probe: SAT_LIMIT_CONFLICTS=300000, niced free
   cores under full A/B load (wall meaningless). The cell is
   **conflict-rate-bound**: idle screens historically reach ~265k conflicts in
   1750s; the solve point is ~270k; in-gate both arms still TIMEOUT. The flip
   needs ~1.15x+ in-gate conflict rate (11k props/conflict cell).
3. Hot-loop identity screens: wall-limit cutoffs are USELESS for identity
   comparison (arms stop at different trajectory points) — use
   SAT_LIMIT_CONFLICTS. 400k-conflict paired runs gave byte-equal
   conflicts/decisions/propagations/search_ticks on ibm/Bubble/booth/
   div-mitern/TT406.
4. Kissat propagation deltas NOT taken (trajectory-rolling, do not port into
   a diet bundle): `c->searched` replacement-search cache and the
   lits[0]^lits[1]^not_lit no-swap layout — both change clause literal order
   that conflict analysis iterates (bump order → coin flip).

## Ranked next steps

### 1. lockchart-group1 in-gate flip (the nearest +1, now proven solvable)
The solve point is ~270k conflicts; we make ~265k idle in 1750s and less
in-gate. Two attack lines, both valid:
(a) more wall diet on the propagation path (this session banked ~5% in-gate;
the remaining big item is the CSR/merged watcher layout, bead ck8, with the
conflict-order-parity minefield documented in the worklist note), and
(b) walk-effort economics — the walker improved twice and found the model,
but 444.6M walk steps dominate the wall budget; measure the wall split
(walk vs search vs factor) and whether SAT_WALK_EFFORT can be cut without
losing the model-finding walk (risky: the walk IS the mechanism). Idle wall
datum now measured: 2598s to the 270k-conflict solve → ~2.6x needed for the
in-gate flip; that is CSR-watcher-scale (ck8), not diet-scale.

### 2. oski20 margin (unchanged)
Still TIMEOUT in both arms this gate. oski40 in-gate went 1425→1351s on the
diet; oski20 solves 1430-1500s standalone idle. Each further in-gate percent
matters; the CSR watcher endgame (ck8) is the remaining big lever, shared
with play #1.

### 3. Density class: instrument the elimination bound rejection
Refinement is dead (above). The honest next question is WHY minimal-gate
definitions still fail the resolvent bound on Bubble/booth (2,818 found → 1
eliminated at 2M conflicts) while kissat eliminates 72-77% of vars there.
Add per-rejection counters (resolvent-count-exceeded / clslim-exceeded /
occlim-capped) to try_eliminate_var's definition path, screen Bubble once,
and compare against kissat -s elimination stats on the same cell.

### 4. Housekeeping / traps (additions)
- Identity screens: SAT_LIMIT_CONFLICTS, never SAT_LIMIT_WALL_SEC (see #3
  above in Measured).
- The A/B launch-log per-cell `conf=` fields make in-gate trajectory-identity
  checks free — grep before running any formal gate.
- `bd comment` needs the FULL bead id (SAT-playground-2a7), not the suffix.
- Free-core probes during a 32-way gate: trajectories/stats exact, wall
  meaningless — perfect for conflict-limited probes; label them as such.

## Where the evidence lives

- Gate: `log/abtest-cand-vs-base-2026-07-16-18-24-39` + launch log
  `log/abtest-hotloop-launch.log`; formal check output in the d23e454 commit.
- Identity screens, defcores screens, lockchart probe: scratchpad (dies on
  reboot) — all decision-relevant numbers are in this note, the d23e454
  commit message, and bead 2a7's comment log.
- Bead: `SAT-playground-2a7` (running gap-analysis log).
