# Session notes: elim-scratch BVE wall diet promotion — 68 vs 67 gate win (2026-07-19)

Continuation of the wall-diet arc (fastidx → this). State at end: promoted
**56a0bb5** `SAT_ELIM_SCRATCH` (default ON). Gate
`log/abtest-cand-vs-base-2026-07-19-09-23-57` (launch
`log/abtest-elimscratch-launch.log`): **PASS, WIN — solved 68 vs 67**
(+rbsat-v1375 SAT 1765.0s, base TIMEOUT), both-solved conflicts EXACT tie
(67 cells, zero mismatches), PAR-2 140,952.1 vs 142,926.7 (−1,974.6), zero
contradictions/correctness failures. checker-timeout on sqrt-mitern170 and vex
arm-symmetric (drat-trim verify window, benign). This is the 5th consecutive
trajectory-identical wall-diet gate winner (fastidx, watchpool, hotloop,
bintag, elim-scratch).

Note the absolute counts (68/67) sit below the previous gate's 69/68 — that is
the contention lottery (sted2/TT492-class cells wobble per gate); the
candidate-vs-baseline comparison inside ONE simultaneous gate is the valid
metric, and rbsat held for cand precisely because the diet banks wall margin.

## The mechanism (measured, not guessed)

New `SAT_TRACE_ELIM=1` decomposition of vex root eliminate (27.7s of wall):

- BVE (`try_eliminate_var`, 833k calls) = 22.5s; BSR = 0.7s; occ build = 0.14s.
- Inside BVE: apply = 17.7s (resolvent insertion `add` = 12.0s, proof snapshot
  2.1s, remove 1.7s, deferred proof deletions 1.7s), resolve = 2.3s,
  partition = 0.6s, gate detect = 0.02s.
- Insertion sub-decomposition: norm/proof/arena/attach/index/enq ≈ evenly
  spread 1.5-3s each — death by per-resolvent allocations + scattered writes,
  no single structural chunk.
- **Kissat spends ~24.7s in eliminate on vex too** (200k-conflict run, -s
  profile) — eliminate cost is at parity; the honest play was an overhead
  diet, NOT an architectural port. (Kissat's dense-mode/no-watcher elimination
  remains a possible future arc but rerolls trajectories.)

## What the diet does (all identity-safe)

1. `try_eliminate_var` → wrapper + `_inner` with six persistent Solver
   workspaces (pos/neg partition, resolvent lits/ranges, proof-del
   lits/ranges); `occurs[var]` iterated by index instead of `.cloned()`.
2. Proof-deletion snapshot flattened (one reused flat buffer, same DRAT bytes).
3. `normalize_original_clause_into` + `norm_scratch`: resolvent insertion is
   allocation-free in steady state; `add_normalized_original_clause` takes
   `&[i32]`.
4. `backward_subsumption_check` relation-marks buffer + stamp persist across
   calls (legacy: fresh Vec + zeroing 2·vars u32 ≈ 5.8MB per BSR entry; stamp
   equality semantics unaffected, wraparound clears as before).
5. `find_merges_closure`: skip gate-inputs write-back clone when renormalize
   returned identical inputs (~1 clone/gate/round on fixpoint rounds 2-7).
6. `SAT_ELIM_SCRATCH=off` = pre-diet allocating implementations VERBATIM
   (legacy fns restored from git) — the fair simultaneous A/B arm.

Idle effect: vex non-search wall −7s (search_start 40.9 → 33.9s); in-gate vex
1611.9s vs 1616.5s. The win concentrates as margin on wall-lottery cells —
which is where the rbsat flip came from.

## Identity evidence (the recipe, again)

- 100k-conflict screens on vex/ibm/oski40/bubble: stdout stats + preprocess
  counters (including mid-search ARMED elim rounds at conflicts
  20992/41788/82060 on vex) byte-equal cand vs base, and off-arm byte-equal to
  the pre-change binary.
- Full gate: 67 both-solved cells, conflicts EXACT tie, zero mismatches.
- 650 unit tests + smoke (9/9, drat-trim verified) at every step.

## Instrumentation added (keep)

- `SAT_TRACE_ELIM=1`: eliminate wall decomposition (occ_build/bsr/bve/gather +
  BVE sub-steps + insertion sub-steps). Timing tokens are `Option<Instant>` —
  branch-only when off. IMPORTANT heisenberg note: with tracing ON the hot
  sub-timers inflate `add` ~2x; use ratios, not absolutes, at the finest level.

## Ranked next steps (delta to the 2026-07-19 aggregate plan)

1. Wall-diet arc continues to pay: next measured chunks are eliminate `other`
   ≈1.2s/round, per-round `vec![false; vars]` flag allocs in `eliminate()`
   (mid-search armed rounds), and the congruence merge-application
   ~1.4s/round (second try_els + gates dealloc churn) — plus the dry-run plan
   reuse idea (round 0 re-extracts what the dry run just computed; reusable
   when extract_binaries+els made no edits — must prove edit-free first).
2. Everything else unchanged from `plan/next-steps-AGGREGATED-2026-07-19.md`
   (density ensemble #3, canonicalization+cache #2, TT406 stabilizer #5,
   pj2008/bp4 measurement #6).

## Where the evidence lives

- Gate: `log/abtest-cand-vs-base-2026-07-19-09-23-57` + launch log
  `log/abtest-elimscratch-launch.log`; formal check output in the commit.
- Decompositions: this note (scratchpad dies on reboot).
- Bead: `SAT-playground-2a7` (comment added).
