# Tseitin ER-refutation session — 2026-07-22 (bead SAT-playground-kk8)

## What was built

A scalable **closed-Tseitin component refutation engine** in
`solver/12-kissat-inprocessing/src/gauss.rs`, wired into `try_gauss_refute`
ahead of the old resolution-only Gaussian path, default-on behind
`SAT_TSEITIN` (off = byte-identical old behavior).

Motivation: the medium suite has three pure Tseitin cells. `tseitin_n188_d3`
(188-node 3-regular expander) and `tseitin_grid_n400_m400` (160k-vertex grid)
are UNIVERSAL timeouts — kissat 4.0.4 fails both too. Offline GF(2) check
(scratch xorcheck.py) proved both are closed Tseitin systems (every var in
exactly 2 XOR equations), single component, odd charge ⇒ UNSAT. The only
barrier was emitting a *feasible DRAT proof*: the old proof path materializes
2^(w-1) clauses per XOR row (width cap 24) and its `min_degree_order` is
quadratic in rows — on the grid it burns the whole 1800 s before search
starts; on n188 the elimination fill-in (cutwidth ~30) exceeds the cap after
minutes of churn.

## Mechanism (the parts that were hard to get right)

1. **Detection** (`find_odd_closed_tseitin_component`): var→eq occurrence
   (≤2), union-find over equations, rhs-parity charge per component; odd +
   closed + eligible ⇒ UNSAT witness. Linear, ~0.5 s on the grid.
2. **Extension variables in DRAT**: fresh `z ↔ a⊕b` = 4 clauses, RAT on the
   first literal. drat-trim uses the FIRST literal as RAT pivot and sorts
   clauses afterward — emit pivot-first, order: (-z a b)(-z -a -b)(z -a b)
   (z a -b). Fresh-var namespace starts at num_vars+1 (pair_abs buffers its
   proof in memory and only commits on success, so no collision).
3. **Summation with bounded row width**: sum all component equations in a
   **greedy min-cut-growth connected order** (pick the frontier equation
   minimizing introduced−cancelled vars; ties by index; on a row-major grid
   this reproduces row-major order). Connectivity matters for soundness:
   `combine_rows` derives the sum row's clauses only when operands share a
   variable. For genuinely disjoint sums, every clause of A⊕B is directly RUP
   when both operands' full clause sets are present (falsifying a C-clause
   assigns all vars of A and B and violates one side's parity) — emitted
   wholesale (`do_combine` disjoint branch).
4. **Prefix-accumulator chains** (the key structure): cut vars are compressed
   into chains ordered by ascending next-use; `z_j = z_{j-1} ⊕ x_j`. A live
   chain occupies P as `{z_end}` (untouched) or the pointer pair
   `{z_end, z_base}`. Consumption = shift the base pointer FORWARD with one
   3-var def-row combine per var — **no unwinding ever** (v1's Belady-paired
   unwind trees blew up: deep nests + sibling exposure ⇒ width explosion).
5. **Pointer parking**: dormant partially-consumed chains cost 2 slots; a
   fresh `z' = z_end⊕z_base` parks them at 1 slot; unpark replays the same
   def row. Without parking the n188 expander fragments into ~11 live chains
   and blows the width cap.
6. **Deletion lines**: the derivation is linear, so spent clauses (old P rows,
   consumed def rows, summed axioms, intermediate resolvents) are deleted
   immediately. drat-trim matches deletions on sorted literals; duplicate
   additions coexist as separate copies, so deleting our copy never kills a
   live original. Cut n188 verification 417 s → 187 s.

## Measured results (idle host)

| cell | before | after |
|------|--------|-------|
| tseitin_n188_d3 | TIMEOUT 1800 s (kissat: TIMEOUT) | **UNSAT 32 s**, 4.71 M-lemma proof, drat-trim **s VERIFIED** 187 s |
| tseitin_grid_n12_m12 | UNSAT 5.65 s (1.26 M-clause proof) | **UNSAT ~1.5 s** (10.7 k-clause proof, verify 0.1 s) |
| tseitin_grid_n400_m400 | TIMEOUT (1800 s inside old gauss path) | declines in 0.5 s, same TIMEOUT (see below) |

Width/emission tuning that mattered: raw width target 2 (`SAT_TSEITIN_COMPRESS`,
default `TSEITIN_COMPRESS_TARGET=2`) — at 6 the grid emitted 120 M+ clauses
(2^width per combine), at 2 it emits 14.6 M; P width cap 14.

## The grid-n400 verification wall (why it stays unsolved)

The engine PROVES the grid in 22 s (14.6 M lemmas) — but backward drat-trim
cannot verify 14.6 M lemmas over a 1.27 M-clause formula within the harness's
1800 s checker cap (measured >1800 s; n188's 4.7 M verifies at ~25 k/s). A
`checker-timeout` on an UNSAT cell is a promotion-gate CORRECTNESS FAILURE
(see compare_bench.correctness_failures), i.e. worse than not answering. So
the engine caps itself: `TSEITIN_MAX_COMPONENT=20_000` equations and
`TSEITIN_MAX_EMIT=6_000_000` lemmas. Grid keeps byte-identical baseline
behavior. Future paths to +1 more solved:
- 3x proof-size reduction won't come from tuning (91 clauses/step ≈ the
  2^(w-1) architecture floor at w≈5-6); needs a structurally different
  derivation (per-row boundary-parity variables ≈ 50/step, still ~8 M) or an
  LRAT/forward-checking harness change (not allowed: goal restricted changes
  to .rs).
- drat-trim's backward-mode rate on the grid degraded ~3x vs n188
  (14.6 M in >1800 s vs 4.7 M in 187 s) — large var space (1.1 M with fresh
  vars) + 1.27 M live originals early in the proof. Understanding/fixing that
  rate is the other half of the problem.

## Gate

Launched 2026-07-22 14:52: `log/abtest-cand-vs-base-2026-07-22-14-52-12`,
arms `cand:` (default, engine on) vs `base:SAT_TSEITIN=off`, 32 cores/16 GB/
1800 s. Expected: cand +1 solved (n188), n12 ~4 s PAR-2 margin, every other
cell trajectory-identical (the engine only fires on ≥90 %-XOR-coverage
formulas whose component detection succeeds — the three tseitin cells).
The extract_xors HashMap-order determinism fix (sort groups by min clause
index) affects only proof-construction order in the never-completing old path
elsewhere; no search trajectory touches it.

## Validation done

- 22 gauss unit tests incl. new: detection (odd cycle/even cycle/open chain/
  3-occurrence/multi-component), drat-trim-verified proofs (odd cycle, 6x6
  grid, 4x20 wide grid, 30-node expander multigraph).
- Full `cargo test --release` green; smoke test 9/9 (proofs drat-verified).
- Both target instances end-to-end + drat-trim verified (n188 187 s idle).
- SAT_TSEITIN=off arm confirmed to reproduce baseline behavior.
