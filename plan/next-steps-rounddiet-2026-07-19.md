# Session notes: SAT_ROUND_DIET per-round inprocessing overhead diet — 69v69 PAR-2 gate win (2026-07-19 evening)

Continuation of the wall-diet arc (bintag → hotloop → watchpool → fastidx →
elim-scratch → THIS). State at end: promoted **74eeaf0** `SAT_ROUND_DIET`
(default ON). Gate `log/abtest-cand-vs-base-2026-07-19-16-01-57` (launch
`log/abtest-rounddiet-launch.log`): **PASS, WIN — solved 69 vs 69 (tie),
both-solved conflicts EXACT tie (70,963,533 both arms, all 100 cells),
PAR-2 138,593.7 vs 138,999.4 (−405.6)**, zero contradictions, zero
correctness failures. `check_promotion_gate` PASS. This is the 6th
consecutive trajectory-identical wall-diet gate winner.

**Both arms at 69 this gate** — oski20 (1693.8s cand / 1712.5s base) and
rbsat (1690.6s cand / 1791.1s base) were IN for both arms; the 69-lineage is
confirmed under this gate's contention profile.

## The mechanism (plan items 1a/1b/1c from the 2026-07-19b aggregate)

One knob, three identity-safe components; `SAT_ROUND_DIET=off` replays the
pre-diet allocating implementations verbatim (the fair simultaneous A/B arm):

1. **eliminate() round workspace persistence** (`simp::ElimRoundWs`): the
   subsumption queue, touched/BSR worklists + flags, candidate BinaryHeap and
   heap version stamps persist across rounds instead of three
   `vec![_; vars]` fills + heap regrowth per root/armed round. Identity
   argument: flag ⟺ worklist-membership invariant restores all-false on every
   exit; queue drained on every exit (`clear_subsumption_queue_marks`); heap
   cleared at entry; carried version stamps only ever compare same-var heap
   entries, where relative order (older < newer) is preserved.
2. **try_els allocation diet**: drop the `original_clause_ids.clone()`
   (multi-MB defensive copy per call, 2 calls per congruence round — the
   collection loop is read-only, verified), persistent active/binaries
   scratch, and a flat-CSR implication graph
   (`els::compute_representatives_csr`) replacing the `Vec<Vec<u32>>` with
   2·vars headers + per-node heap blocks. Per-node edge order equals legacy
   push order (both passes scan the binaries stream in order) → identical
   Tarjan traversal, unit-tested against legacy (`csr_variant_matches_legacy`).
3. **Congruence round-0 dry-run plan reuse**, self-guarded at runtime:
   reuse fires only when `added_bins == 0` (extract-binaries' only mutation
   site is the install loop) AND `els_substituted_vars` unchanged (try_els
   bumps it before its first mutation) — then the formula is byte-identical
   to what the dry run saw and extraction+closure are pure, so the dry-run
   plan IS what recomputation would produce. NOTE measured on ibm: it never
   fired there (round 0 keeps finding 500-900 new hidden binaries per armed
   invocation) — this component is nearly free but also nearly inert;
   don't credit it for the win.

Giant (>20M var) `turn_off_elim` path frees the persistent workspaces
before the GC relocation transient (same rationale + scoping as the
cd8f1b5 OOM reclaims).

## Measured effects

- Idle screens (100k conflicts, concurrent so timing directional): vex
  −3.5s, oski40 −4.1s, ibm −0.4s; trajectories byte-equal.
- In-gate wall-lottery margins banked: **sted2 −187.1s (1590.8s)**,
  **rbsat −100.5s (1690.6 vs 1791.1 — base was 9s from timeout)**,
  aaai10-planning −108.8s, SCPC-500-14 −64.9s, **TT492 −40.9s (1483.8s)**,
  **oski40 −22.5s (1237.8s)**, **oski20 −18.7s (1693.8s)**, vex +3.8s (noise).
  A few SAT cells slower (bp4_CSO_IXA +183.6s — SAT-lottery noise under
  identical conflicts); aggregate PAR-2 −405.6.
- Every future gate inherits these margins; oski20 now ~106s inside the wire
  in this contention profile (was flip-target at ~60s needed).

## Identity evidence (the recipe, 6th time)

- 100k-conflict screens on vex/ibm/oski40/bubble: stdout + stderr
  (SAT_TRACE_PREPROCESS_DETAILS counters incl. mid-search armed elim rounds)
  + full SAT_STATS_JSON (volatile fields stripped: *_sec, seconds_*,
  max_rss_mb, shas, config_hash, feature_maturity) byte-equal across
  cand / SAT_ROUND_DIET=off / pre-change (3df404c) binaries.
- Gate: conflicts EXACT tie over all 100 cells (69 solved each).
- 651 unit tests (+ new els CSR-vs-legacy test) + smoke 9/9 drat-trim clean.

## Ranked next steps (delta to plan/next-steps-AGGREGATED-2026-07-19b.md)

1. **Wall-diet arc continues**: remaining measured chunks —
   `compute_representatives_csr` still allocates 5 flat arrays per call
   (disc/low/on_stack/comp_min/repr — could persist as workspace);
   eliminate `other` attribution now has a `heap_build` sub-timer in
   SAT_TRACE_ELIM (unmeasured this session — run vex trace to see if the
   per-round heap rebuild is chunky enough for the persistent-schedule
   algorithmic change kissat uses); congruence gates-Vec dealloc churn
   (871k inputs Vecs/round) still unaddressed (needs flat gate arena
   through find_merges_closure — medium refactor, identity-safe).
2. **Round-0 dry-run reuse is near-inert on ibm** — before extending it
   (e.g. caching across armed invocations), measure how often round-0 is
   edit-free on vex/oski (SAT_DEBUG_CONGRUENCE + grep "reusing dry-run").
3. Everything else unchanged from the 2026-07-19b aggregate (density
   ensemble #3, canonicalization+cache #2, TT406 stabilizer #5,
   pj2008/bp4 measurement #6).

## Standing traps confirmed again

- feature_ablation setup phase runs ~2 min single-threaded before the
  [abtest] line appears — not hung.
- drat-trim verify tail after last solver exit: ~35 min this gate (vex
  proofs in both arms).

## Where the evidence lives

- Gate: `log/abtest-cand-vs-base-2026-07-19-16-01-57` + launch log
  `log/abtest-rounddiet-launch.log`; formal check output in commit 74eeaf0.
- Identity screens: scratchpad (dies on reboot) — decision-relevant numbers
  are all in this note.
- Bead: `SAT-playground-2a7`.
