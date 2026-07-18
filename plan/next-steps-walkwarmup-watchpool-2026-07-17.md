# Session notes: walk-warmup screens (NEGATIVE) + watch-pool campaign (2026-07-17)

State at session start: medium baseline **68/100** @ 2ed8e27 (watch-inline-binary
promotion). Kissat 4.0.4 reference: 74/100. Gap ≈ 6.

## SAT_WALK_WARMUP: measured NEGATIVE — do not re-screen blind

Implemented kissat warmup.c parity as `SAT_WALK_WARMUP` (default OFF, code in
2ed8e27+this session's commit): before every rephase walk, complete the root
assignment by repeated decide + propagate-beyond-conflicts (re-calling
`propagate()` until fixpoint, conflicts skipped without analysis), assignments
eagerly save phases (temporary accounting mode with `update_phase: true`, so no
search stats/ticks/arming-signal pollution), then `backtrack(0)` which leaves
saved phases in place (kissat backtrack_without_updating_phases semantics).
643 unit tests + smoke pass; ibm canary byte-identical (walk_warmups=0 —
root-armed cells cannot reach the walk path, rephase gate is yield/decision-armed
only).

Paired standalone screens (warm vs base simultaneous, idle host, 2026-07-17):

- **TT492 (the class's current +1): LOST** — base solves SAT 1289s / 3.73M
  conf; warm TIMEOUT 1750s @ 4.24M conf.
- **TT406 (hoped recovery): NOT recovered** — base TIMEOUT 4.77M conf, warm
  TIMEOUT 5.43M conf (worse trajectory).
- **C_395: regressed** — base SAT 184s / 719,772 conf; warm SAT 411s /
  1,574,747 conf (+855k). Warm walked 8/8 improved vs 5/5 — walker "improves"
  more but the search trajectory is worse.
- **lockchart-group1: solve LOST** — base solves at 270,451 conf (~2600s idle);
  warm hits the 300k-conflict limit at 1835s WITHOUT solving (walker improved
  2x but never found the model). Note warm reached 300k conf in 1835s vs base
  ~2600s to 270k — warmup runs cheaper per conflict here, but the model-finding
  walk is destroyed.
- **Bubble: inert** — walks=0 in BOTH arms (the yield-armed density class never
  reaches a walk slot in-budget); conflict-count delta is wall-cutoff noise.

Verdict: warmup's wholesale saved-phase overwrite (every var, every walk)
destroys the collapse+walk phase-evolution lottery that the decision-armed
Timetable class and lockchart depend on. Kissat wins these cells WITH warmup
on, but transplanting warmup alone into our schedule is strictly negative.
The knob stays default-off groundwork. No A/B was spent.

## Next: flat watcher pool (ck8 endgame, trajectory-identical wall diet)

Design decisions (locked in after reading kissat vectors.c/proplit.h):
- All watch lists in ONE contiguous arena, per-literal {start, len, cap};
  push appends in place or relocates the list to the arena end (order
  preserved, holes reclaimed by rebuild/defrag at the existing GC sites).
- Do NOT port kissat_delay_watching_large (deferred watch pushes): deferred
  arrival changes list-order evolution vs legacy immediate push → full-suite
  trajectory reroll. Push immediately, exactly like legacy Vec::push.
- Hot loop iterates by absolute indices, re-deriving the arena base pointer
  after any push (pool realloc moves memory; offsets stay valid). Monomorphize
  the pool/nested storage choice like PTR_FAST does (const-generic dispatch),
  cold sites go through small accessor methods.
- Off-switch (SAT_WATCH_POOL=off) = the legacy Vec<Vec<Watcher>> path,
  byte-for-byte.
- Identity proof: SAT_LIMIT_CONFLICTS screens, byte-compare
  conflicts/decisions/props/ticks on ibm/Bubble/TT406/sudoku/lockchart
  (never wall-limit screens — arms stop at different points).

Target cells: oski20 (solves 1430-1500s idle, needs ~10-20% in-gate margin =
the nearest +1), lockchart/goldcrest/pj2008 (propagation-bound), PAR-2 tier
suite-wide (the bintag/hotloop precedent: gate cannot lose on identical
trajectories).

## Watch-pool implementation + screens (measured 2026-07-17 evening)

Implemented as described. Key deltas from the design sketch:
- Cold sites unified through Solver watch_* dispatch helpers + a free
  `rewrite_all_watch_lists(watchers, pool, active, f)` (field-level split
  borrows for closures reading `self.arena`).
- Hot loop: local `wat_get!/wat_set!/wat_finish!` macros over a
  (pstart, plen) window; pool access via `get_unchecked` THROUGH `self`
  (no held raw pointers — the PTR_FAST arena-window UB shape). The
  bounds-checked first version cost +4% wall on cache-resident Bubble
  (triplicate-confirmed); the unchecked version is NEUTRAL there
  (33.1-33.2s both arms x3). Do not regress this: the pool fields being
  self-fields (vs the legacy mem::take LOCAL Vec) is why checked indexing
  was not free.
- Pool tests: push-order-across-growth vs a reference Vec<Vec>, swap_remove/
  truncate/defragment parity, exact-counts CSR layout. 646 tests total.

Identity screens (SAT_LIMIT_CONFLICTS, paired, byte-compare
conflicts/decisions/propagations/search_ticks): ibm 100k, Bubble 400k,
TT406 400k, TT492 200k, lockchart 100k — ALL byte-identical. Walls on the
identical trajectories (idle): ibm −5.0%, lockchart −1.4%, TT492 −1%,
Bubble/TT406 neutral. RSS slightly higher (holes + eager caps): ibm +5%,
TT406 +12% — watch for 16GB-margin cells in the gate (none of the wire
cells sit near the memory cap since the u32/giant diets).

## Gate 1 (SoA metadata): FAIL by timing noise — and the diagnosis

`log/abtest-watchpool-vs-base-2026-07-17-20-01-35`: solved 67==67 (identical
sets; TT492 solved BOTH arms 1643/1601s; rbsat+TT406 timeout both), both-solved
conflicts IDENTICAL 62,041,959 (100-cell trajectory identity held), PAR-2
143,409.5 vs 143,354.0 → formal gate FAIL by +55.5 (0.04%, driver-flagged
"timing noise"). Per-cell walls: wins on big-arena cells (6s299 −66s, oski15
−57s, sqrt-mitern −37s, SCPC −31s, ee5 −18s) offset by a SYSTEMATIC ~2-3%
loss on mid-size cells (QG7 +30s, sudoku +28s, TT492 +42s, div-mitern +16s,
Pancake +14s; vex +76s and sted2 +71s are the wall-lottery cells).

Diagnosis: pool metadata was three parallel arrays (starts/lens/caps) → every
literal visit touched 2-3 metadata cache lines vs the legacy Vec header's one.
Fixed by packing per-list metadata into ONE 16-byte `WatchMeta{start,len,cap}`
(fewer bytes than the legacy 24-byte Vec header). Gate 2 relaunched with the
AoS layout after re-verifying identity + walls.

## Gate 2 (AoS): LOSE 65 vs 67 — the fragmentation mechanism

`log/abtest-watchpool-vs-base-2026-07-17-23-10-08`: trajectory identity held
again on all pairs, and the AoS fix flipped the mid-size regressions into wins
(sudoku 1296 vs 1363, 5dbe7b 546 vs 636, 5e933a 122 vs 178, oski15 1295 vs
1328, TT492 1734 vs 1775, SCPC 123 vs 163). But the pool arm LOST two wire
cells to the wall: **vex TIMEOUT vs base 1678s** (pool slower on vex in BOTH
gates: +76s then >+122s — mechanistic, not lottery) and **rbsat TIMEOUT vs
base 1680s** (the coin-flip cell).

vex mechanism found: `with_temporary_assumptions` (per-pattern deep clone,
bead 3yw) cloned the ENTIRE pool arena including relocation holes + slack
capacity (`Vec::clone` on the legacy nested lists copies len, not capacity),
and NOTHING defragmented the pool on non-giant cells — rewrite-heavy armed
cells fragment without bound, so every vivify-round clone copied a ballooning
buffer. Fixes: (a) `clone_tight()` in the guard (copy live entries only;
restore also swaps the tight layout back in), (b) `maybe_defragment()` (waste
≥ data/4) hooked after all four GC-adjacent watcher rewrite passes. Both
layout-only → trajectory-neutral. 647 tests.

Fix validation (paired, idle): vex byte-identical 2,975,066 conflicts, pool
1084s vs base 1131s (−4.1% — was +7% SLOWER pre-fix); ibm identity intact,
−3.8% wall. vex RSS 5.0GB vs 4.0GB (holes between defrags; far from the cap).

## Gate 3 (AoS + defrag/tight-clone): PASS, WIN → PROMOTED default-on

`log/abtest-watchpool-vs-base-2026-07-18-01-51-12` (launch
`log/abtest-watchpool3-launch.log`): solved 67==67, both-solved conflicts
IDENTICAL on all 66 pairs (zero divergent cells — three full gates of
100-cell trajectory identity now on record), PAR-2 144,826.1 vs 145,116.8
(−290.7). check_promotion_gate: PASS (after TaskStop'ing the monitor shell —
the standard running_solver_processes false positive). Solved-set swap inside
the wall-lottery class: cand-only **TT492 SAT 1607s** (base TIMEOUT), base-only
VanDerWaerden 1541s (pool 1508-vs-1428 in gate 2; consistently ~+80s there —
the one repeatable pool-slower cell alongside QG7 +76s; both priced into the
PAR-2 win). Wire cells all landed IN for the pool arm: vex 1590s (−61s),
rbsat 1785s, sted2 1701s (−19s). Both-solved wall −1.4%; biggest wins sudoku
−74s, booth −71s, reconf −64s, vex −61s, Kakuro −61s, sqrt-mitern −47s.

Promoted: `SAT_WATCH_POOL` default ON (off = legacy byte-for-byte). Tests
converted to storage-agnostic watch_* helpers; 647 tests + 9/9 smoke pass in
BOTH modes.

## Traps (this campaign)

- A flat-pool port has TWO perf cliffs the legacy Vec<Vec> never had:
  metadata SoA (2-3 cache lines/visit — use one packed 16B struct) and
  unbounded fragmentation + full-arena clones (defrag triggers + tight
  clones). Idle small-cell screens catch neither; per-cell gate wall diffs
  found both.
- The temporary-assumption guard is the hidden hot path for any storage the
  armed vivify machinery clones per pattern (bead 3yw): clone cost scales
  with LAYOUT size, not live size, unless made tight explicitly.
- Bounds-checked indexing through self-fields in the propagate loop is NOT
  free even when branch-predicted: the legacy mem::take local kept ptr/len in
  registers across &mut self calls; get_unchecked through self recovers it.
