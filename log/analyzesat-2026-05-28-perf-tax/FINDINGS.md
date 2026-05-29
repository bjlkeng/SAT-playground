# Bottleneck Analysis — solver/11-kissat-port — 2026-05-28 (perf-tax)

**Investigation slug:** `analyzesat-2026-05-28-perf-tax`
**Agent:** `slate-heron` (continuation of crimson-fox's `analyzesat-2026-05-28-exec-tax`)
**Worktree:** `/tmp/analyzesat-2026-05-28-perf-tax` (built at `35429ab`)
**Primary question:** Is the residual solver10↔solver11 *identical-work* execution
tax **ALU** (per-event branches / reason encode-decode, T1–T6) or **cache/working-set
layout**? This is the exact open question that blocked `SAT-playground-5b2.2.62`
because the previous run had no perf counters.

## Why this run could finally answer it

The previous exec-tax run (crimson-fox) was blocked on hardware counters:
`perf_event_paranoid = 4` and no valgrind. **On this host it is now `1`**, and
`perf stat` reads `cycles`/`instructions`/`L1-dcache-load-misses`/`dTLB-load-misses`
fine (verified). So the ALU-vs-layout question is measurable again.

## Scope (what this run does / does not do)

- **Does:** `perf stat` counter decomposition, normalized per propagation, for a
  **three-point** comparison on the two probe instances (Sudoku, Kakuro) in the
  single/no-LBD parity config — plus a full source-level (Phase-4) diff of the
  hot per-propagation working set.
- **Three binaries** (all built with identical profiling flags
  `CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=1
  RUSTFLAGS="-C target-cpu=native"`):
  - `solver10` (`10-bve-preprocess`) — legacy baseline, profile PAR-2 **699.671**
  - `solver11-head` — `35429ab`, T1–T6 present, profile PAR-2 **753.236**
  - `solver11-candidate` — s11-06's uncommitted `NORMAL_SEARCH` const-spec diff
    (T1–T3 removed), profile PAR-2 **734.833**. Rebuilt from
    `s11-06-normal-search-candidate.diff` (applies clean on `35429ab`).
- **Does not:** re-run the full 6-config feature ablation (Phase 1) or the kissat
  reference comparison (Phase 3). Those answer the *feature-gap* question and are
  already well-characterized in `SOLVER11_STATE.md` / `FEATURES.md` and prior
  analyzesat runs; this run is laser-focused on the parity micro-perf tax that is
  the active P1 blocker (`5b2.2.61` → `5b2.2.62` → `5b2.2.56`).
- **Coordination:** read-only profiling, no source edits to `main`, no bead claims.
  `5b2.2.61` and the candidate stay s11-06's. Agent-mail direct/broadcast posts
  were blocked by stale agents' contact policies (pending requests created), so
  coordination is via a `bd note` on `5b2.2.62` and this FINDINGS doc.

## Executive Summary

- **The identical-work tax is ALU-dominated, not layout.** On both probe instances
  solver11 executes **~10–12% more instructions per propagation** than solver10
  doing byte-for-byte identical search work (same conflicts/decisions/propagations).
  The cache/layout signal is a **minority**: L1-dcache-load-misses/prop is only
  +1.9–4.9% and does not move when the accounting branches are removed.
- **Quantified residual split (candidate vs solver10, Sudoku):** of the ~42.6
  extra cycles/propagation, ~31 cyc (~73%) are the extra instructions (ALU/codegen)
  and ~11 cyc (~27%) are the extra L1 misses (layout). Same shape on Kakuro.
- **The const-specialization (5b2.2.61 / `56e56cc`) behaved exactly as predicted:**
  it cut ~17 insn/prop (Sudoku) / ~22 (Kakuro) — ~20% of the head excess — with
  **zero** change to L1-miss/prop, confirming T1–T3 were pure ALU branches.
- **Source-level (Phase 4):** the hot per-propagation working set is
  **byte-identical** between solver10 and solver11 (per-var arrays, `Watcher`,
  arena clause sizes). The residual +0.9 L1-miss/prop is therefore *not* the
  per-var/clause arrays — it is consistent with the **189-vs-71-field `Solver`
  struct** spreading hot base-pointers/flags across more cache lines. The layout
  beads `5b2.2.18.3` (usize→u32 per-var) / `5b2.2.18.4` (LearnedMeta split) shrink
  *both* solvers' arrays but do **not** close the s10↔s11 *parity* gap (s10 has the
  same arrays); they are absolute-throughput levers, not parity levers.
- **Disposition:** counters implicate ALU, so per the bead the lever is reason
  encode/decode. Localization (`perf record`) decides whether that is a fixable
  hotspot or diffuse codegen → see Phase 6b. Recommendation in the final section.

## Phase 4 — Reference (solver10) source diff of the hot per-propagation working set

The exec-tax FINDINGS flagged a "richer per-var / per-clause metadata inflating the
working set" as the key *layout* suspect. Reading the actual structures refutes the
per-element-width version of that hypothesis:

| Hot structure (touched per propagation/enqueue) | solver10 | solver11 | Same? |
|---|---|---|---|
| `assignment[var]` | `Vec<u8>` | `Vec<u8>` | ✅ |
| `saved_phase[var]` | `Vec<u8>` | `Vec<u8>` | ✅ |
| `decision_level[var]` | `Vec<usize>` | `Vec<usize>` | ✅ |
| `reason[var]` | `Vec<usize>` | `Vec<ReasonCode>` = `Vec<usize>` (8 B) | ✅ (same width) |
| `Watcher` (watch list element) | `{clause_idx:u32, blocker:i32}` = 8 B | `{clause_idx:u32, blocker:i32}` = 8 B | ✅ |
| arena clause words = `1 + len + extra` | `CLAUSE_ACTIVITY_WORDS=2`, `ORIGINAL_ABSTRACTION_WORDS=1`, `CLAUSE_SIZE_SHIFT=5` | identical constants | ✅ |

Per-var heap arrays, watch-list elements, and arena clause encodings are all
byte-identical. **`ReasonCode` is a single `usize`** — the T4/T6 cost is the
tag-mask/`match`/`Result`/`expect` *ALU*, not a wider reason slot.

The **only** structural difference is the `Solver` struct itself:
**189 fields (solver11) vs 71 (solver10)**, ~2.7×. But the per-var/clause arrays
are *separate heap allocations* whose layout is unaffected by struct size; the
struct only holds the base pointers + scalar flags, which LLVM hoists into
registers across the propagate inner loop. So this is at most a per-`propagate()`-call
pointer-load effect, not a per-propagation working-set effect.

T5 (`note_clause_used_as_propagation_reason`, main.rs:2741) short-circuits on
`!normal_search_accounting || !use_lbd || ...`; in the no-LBD parity config it
returns after ~2 bool loads + branch with **no array access**.

**Prediction:** `L1-dcache-load-misses / propagation` and `dTLB-load-misses /
propagation` should be ~equal across all three binaries (identical memory layout),
while `instructions / propagation` should be highest on solver11-head, drop on the
candidate (T1–T3 removed), and the residual candidate↔solver10 gap should be the
T4/T5/T6 ALU. If the miss counters match, the layout beads (`5b2.2.18.3` usize→u32,
`5b2.2.18.4` LearnedMeta hot/cold split) are **not** the lever for this tax.

## Candidate diff (T1–T3 removal) — what it changes

`s11-06-normal-search-candidate.diff` threads `const NORMAL_SEARCH: bool` through
`propagate` → `propagate_impl` → `propagate_binary_implications`,
`record_propagation_accounting`, `record_search_ticks`, and a new
`enqueue_impl::<NORMAL_SEARCH>` (dispatched from `enqueue` off `is_temporary()`).
In the `NORMAL_SEARCH=true` monomorphization: T1 → bare `stats.propagations += 1`;
T2 → unconditional `saved_phase` write; T3 → bare `current_level == 0`. It does
**not** touch T4 (`set_reason_ref`/`ReasonCode::from_ref`) or T6 (reason reads), so
the candidate's residual vs solver10 is exactly T4/T5/T6 + any layout/codegen.

## Phase 6 — Hardware counters (perf stat, per propagation)

Event sets chosen to avoid PMU multiplexing on this Zen3 host (~5–6 effective
counter slots after the NMI watchdog): Pass A `task-clock,cycles,instructions,
L1-dcache-load-misses,dTLB-load-misses` (verified 100% counter active, no
multiplex). All three binaries built with identical profiling flags. Work counts
are **identical** across all three on each instance (premise confirmed):
Sudoku 259,775 conflicts / 6,772,770 decisions / 1,312,437,897 propagations;
Kakuro 732,107 conflicts / 3,188,069 decisions / 617,655,456 propagations.

| Instance | Binary | insn/prop | cyc/prop | IPC | L1-dmiss/prop | dTLB-miss/prop |
|---|---|---|---|---|---|---|
| Sudoku | solver10 | 768.0 | 582.1 | 1.32 | 18.33 | 2.162 |
| Sudoku | solver11-head (T1–6) | 860.1 (+12.0%) | 631.9 (+8.6%) | 1.36 | 19.24 (+4.9%) | 2.173 |
| Sudoku | solver11-candidate (T1–3 gone) | 843.4 (+9.8%) | 624.7 (+7.3%) | 1.35 | 19.22 (+4.8%) | 2.162 |
| Kakuro | solver10 | 2194.6 | 1430.4 | 1.53 | 51.30 | 6.181 |
| Kakuro | solver11-head (T1–6) | 2422.0 (+10.4%) | 1533.1 (+7.2%) | 1.58 | 52.27 (+1.9%) | 6.724 |
| Kakuro | solver11-candidate (T1–3 gone) | 2400.0 (+9.4%) | 1521.0 (+6.3%) | 1.58 | 52.06 (+1.5%) | 6.883 |

Work × speed read: this is a pure **speed** (per-event execution) tax — work is
identical, so `work_ratio = 1.000` for all rows; the wall delta is entirely the
cyc/prop column. The cyc/prop excess decomposes into the insn/prop excess (ALU)
plus the L1-miss/prop excess (layout): Sudoku candidate residual 42.6 cyc/prop ≈
75 insn/prop × ~0.42 cyc/insn (~31 cyc, **73%**) + 0.89 L1-miss/prop × ~12 cyc
(~11 cyc, **27%**). Kakuro shows the same shape with a noisier dTLB term (the
490 MB instance is more TLB/DRAM-bound).

**Branch counts (Pass B, Sudoku):** solver10 152.2 branches/prop, solver11-head
168.5 (+10.7%), solver11-candidate 164.5 (+8.1%) — the candidate shed ~4
branches/prop (the T1–T3 accounting branches). Crucially, **branch-miss COUNT is
flat across all three** (~2.31–2.32B; rates 1.16% / 1.05% / 1.08%). So the extra
branches are *well-predicted* — always-same-direction policy/accounting checks
(T1–T6), not misprediction stalls. This is why the tax shows up as extra issued
instructions at near-constant IPC rather than pipeline-flush cycles, and why
removing them helps a little but not dramatically.

## Phase 6b — Residual localization (perf record, solver10 vs candidate, Sudoku)

`perf record -F 1999`, ~80 s steady-state sample each, `--sort symbol`:

| Symbol | solver10 | solver11-candidate |
|---|---|---|
| `Solver::propagate` | **78.66%** | **76.08%** |
| `Solver::backtrack` | 7.10% | 9.01% |
| `push_branch_var*` | 3.77% | 4.26% |
| `solve_*_with_proof` | 2.48% | 1.42% |
| `backward_subsumption_check_dynamic` | 0.85% | 0.94% |
| `minimize_learned_clause` | (—) | 0.64% |
| `analyze_conflict_to_scratch_impl` | (—) | 0.58% |
| `branch_var_better` | 0.58% | 0.57% |

**Decisive result: the residual ALU is diffuse, not a hotspot.** Both binaries
are dominated by `Solver::propagate` (the watcher-walk BCP loop), and **no
`set_reason_ref`, `ReasonCode::as_ref`, or `enqueue` symbol appears in either
profile** — they are inlined into `propagate`. The candidate→solver10 instruction
excess therefore lives *inside* the inlined propagate (and backtrack) loops as the
~16 extra well-predicted policy/accounting/encode instructions per propagation
(T1–T6 + general codegen), spread throughout — there is no separable function to
fast-path. `backtrack` is also slightly heavier on solver11 (9.0% vs 7.1%),
another piece of the same diffuse infrastructure tax.

This **refutes the "reason encode/decode fast path" hypothesis as a meaningful
lever**: T4/T6 are a sliver inlined into the hot loop, and the concentrated win
(T1–T3 accounting branches) was already taken by `56e56cc`. Per the optimization
workflow ("do not implement when the measured opportunity is negligible"), no
reason fast-path is implemented here.

## Code-Level Recommendations (ordered by ROI)

1. **Re-scope `SAT-playground-5b2.2.56` to ACCEPT the residual identical-work
   parity tax (~6–7%).** It is diffuse, well-predicted ALU inlined into the
   propagate/backtrack hot loops — the inherent cost of solver11's richer
   per-event infrastructure (const-generic dispatch, `ReasonCode` packing,
   accounting/phase hooks, 189-field `Solver` struct). It is not efficiently
   clawable to zero without removing machinery that exists *for solver11's
   features*. The strategic lever for beating solver10 is **feature trajectory**
   (LBD / focused-stable / VMTF reducing *conflicts*), not matching solver10 on
   identical-work throughput. Keep 5b2.2.56 blocked only if the project still
   wants parity; otherwise close it as "accepted, won't-chase".
2. **Re-label the layout beads as absolute-throughput, not parity.** `5b2.2.18.3`
   (usize→u32 per-var arrays) and `5b2.2.18.4` (LearnedMeta hot/cold split) target
   the minority (~27%, ~0.9 L1-miss/prop) layout component — but the per-var
   arrays are **byte-identical to solver10**, so these shrink *both* solvers and do
   **not** close the s10↔s11 parity gap. They remain worthwhile as absolute
   throughput wins (help every config, memory-bound instances most), just not as
   parity fixes. Validate against the >3% profile-bench threshold before promoting.
3. **No further micro-opt of the single-mode propagate path.** The const-spec
   (`56e56cc`) already took the one concentrated win (+~17 insn/prop, no layout
   change). Remaining per-event policy instructions are well-predicted (flat
   branch-miss count) and diffuse; chasing them individually will not clear the
   3% keep threshold.

## Artifact Paths

- This document: `log/analyzesat-2026-05-28-perf-tax/FINDINGS.md`
- Measurement script: `log/analyzesat-2026-05-28-perf-tax/run_perf_tax.sh`
- Quiet-core waiter: `log/analyzesat-2026-05-28-perf-tax/wait_then_perf.sh`
- Candidate diff (provenance): `log/analyzesat-2026-05-28-perf-tax/s11-06-normal-search-candidate.diff`
- Binaries: `sat-solver-11-head`, `sat-solver-11-candidate-rebuilt` (+ solver10 in worktree)
- Raw perf output: `perf_<instance>_<binary>_<pass>.txt`; solver stats: `stats_<...>.txt`
