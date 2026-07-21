# SAT_PHASE_DELTA session — 2026-07-21 (incremental phase-prefix capture)

## The finding (read this first)

`capture_target_phase` / `capture_best_phase` walked the **entire trail** on
every new-max-trail event, and `maybe_capture_phase_prefix` fires on every
conflict-free propagation fixpoint in stable mode — i.e. on nearly **every
decision** during deep conflict-free dives. Cost per capture: 2 random DRAM
touches per trail entry (assignment[var] read + phase[var] write), ×2 arrays
(target + best). On shallow-trail cells this is invisible (why 8 wall-diet
sessions never saw it); on deep-trail cells it is O(trail) per decision —
quadratic-flavored — and dominates everything:

- **pj2008 (8.6M vars, avg level 22k, max 89.5k)**: legacy walked
  **131.6e9 trail entries** in 250s of search — search did only 4008
  conflicts / 38.1M props (153k props/s). That IS the search phase.
- Decomposition that found it: 360s run = parse 4.7 + preprocess 42.9 +
  search 251.0; search_ticks only 2.65/prop (propagation NOT the sink);
  234µs per decision-cycle unexplained → read the fixpoint path.

## The change (solver/12-kissat-inprocessing)

Low-water-mark incremental capture (`SAT_PHASE_DELTA`, default on; `off` =
legacy full walk byte-for-byte):

- New fields `target_capture_low` / `best_capture_low`: trail positions below
  the mark are captured-and-unchanged since the last capture. Captures walk
  `trail[low..]` only, then set `low = trail.len()`.
- Soundness invariant: a var at a position below the low-water mark has stayed
  continuously assigned with the same value since it was captured. Every
  trail-shrink site lowers the mark:
  - `backtrack()` (both paths): `low = min(low, new_trail_len)` — the chrono
    compaction reads/writes only positions >= new_trail_len.
  - `end_temporary_assumptions`: `low = min(low, guard.start_trail)`.
  - Unassignment happens ONLY at those sites (verified: 3 sites total).
- Reset sites restore full-walk semantics: `reset_target_phase()` (array
  cleared) sets target low = 0; the rephase 'B'-slot `best_assigned = 0`
  reset sets best low = 0 (stale prefix values must be overwritten like the
  legacy walk would).
- Stats: `phase_capture_entries` counts walked entries (the traffic meter
  that proves attribution). Strip it (with `*_sec`, `seconds_stable`,
  `seconds_focused`, `max_rss_mb`) in cross-arm identity byte-compares.

## Measured evidence (pre-gate)

- pj2008 300s screen (same binary, on vs off): conflicts 142,929 vs 4,008
  (**35.7x**), props 1.41G vs 38.1M (**37x**), props/s 5.62M — ABOVE kissat's
  5.1M on this instance; capture entries 78.9M vs 131.6e9 (1668x).
- vex @300k conflicts: elapsed 244.4s vs 290.1s (**wall −15.7%**), captures
  4.4M vs 30.8e9. ibm full solve: 116.0s vs 117.6s (−1.4%).
- Identity: stripped stats JSON byte-identical on ibm (full SAT solve) and
  vex @300k; fuzz test `phase_capture_delta_is_byte_identical_to_legacy_full_walk`
  asserts equal trajectories + equal target/best/saved phase arrays; 659 unit
  tests, smoke 9/9.

## Pre-gate prediction (recorded 2026-07-21 before results)

Trajectory-identical wall diet ⇒ conflicts EXACT tie on tied solved cells;
solved >= 69 expected. Upside candidates: pj2008 (timeout, now searches ~35x
deeper on the SAME trajectory; kissat solves it SAT at 1165s), oisc-subrv
(7M vars, kissat-timeout), and wall-margin relief on the deep-trail BMC class
(vex −15%, oski15/sted2/rbsat/TT/lockchart-g2 class). Loss risk: load-noise
only (rbsat 5.4s margin) — the diet cuts wall on exactly that class, so risk
is lower than any prior diet gate.

## Gate

- Run: `log/abtest-cand-vs-base-2026-07-21-08-17-47`
  (cand:SAT_PHASE_DELTA=on vs base:SAT_PHASE_DELTA=off, launched 08:17).
- Formal baseline TSV (69-lineage, unchanged):
  `log/abtest-cand-vs-base-2026-07-20-12-03-06/cand/results.tsv`.
- Gate cmd:
  `python3 tools/check_promotion_gate.py --multiseed --candidate log/abtest-cand-vs-base-2026-07-21-08-17-47/cand/results.tsv --baseline log/abtest-cand-vs-base-2026-07-20-12-03-06/cand/results.tsv --timeout 1800 --memory-mb 16000`
- RESULT: **PASS (WIN)** — cand 68 vs base 67, conflicts EXACT tie over all
  67 both-solved cells, PAR-2 140180.2 vs 142661.2 (−2481), correctness
  clean. Cell story: rbsat-v1375 SAVED by cand (SAT 1749s, base arm lost it
  to load); oski15a01b20 TIMEOUT in BOTH arms (load; 107.7s-margin lottery
  cell). vs the 69-lineage TSV the formal check reads 68v69 (FAIL) but both
  "lost" cells are load lotteries the identical-legacy base arm also lost —
  same signature as the promoted closure-diet 68v67 gate.
- Idle-box confirmation (post-gate): oski15a01b20 with the promoted config =
  **UNSAT 1400.9s at EXACTLY 2,663,684 conflicts** (identical trajectory,
  291s faster than the 1692.3s lineage run; margin 108s → 399s). The
  68-not-69 is entirely gate-load, the lineage is intact and strengthened.
- Biggest tied-cell wall gains in-gate: 6s299 −339.8s, VDW −119.1s,
  oski15b40 −92.0s, TT492 −70.9s, VexRiscv −68.7s; total −630s/67 cells.
- pj2008/oisc: still TIMEOUT (deeper search alone insufficient this draw).

## Session provenance

- Bead: SAT-playground-pow.
- The var-giant scoping analysis (kept for reference): cells with >=7M
  declared vars are pj2008 (TO), oisc (TO), 18.normalised (solves at ROOT,
  conf=0 — any first-conflict-latched mechanism is byte-identical for it),
  plus the >=20M lean giants. A "deep-arm above max-solved-conflicts
  (7.87M, cell 59-129706)" free-lottery scoping was analyzed: only
  high-conflict-rate timeout cells (density UNSAT cluster) would arm; TT
  lottery cells never reach 8M conflicts; parked as a future zero-risk shape.
