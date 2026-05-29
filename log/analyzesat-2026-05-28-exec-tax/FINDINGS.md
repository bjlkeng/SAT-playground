# Bottleneck Analysis (source-level, partial) — solver/11-kissat-port — 2026-05-28

## Scope note — why this is a partial analyzesat

This run was requested while the host was under heavy contention and the prime
target is actively owned by another agent, so the bench-heavy phases were
**deliberately not run**:

- **Phase 1 (ablation matrix) / Phase 3 (reference live comparison): skipped.**
  Agent `s11-06` had a full profiling-suite baseline bench running on this
  6-core/12-thread host. A concurrent multi-config ablation would contaminate
  both their timings and mine, defeating the scientific purpose.
- **Phase 6 (hardware counters): blocked.** `/proc/sys/kernel/perf_event_paranoid = 4`
  (disallows user-space CPU event access), so `perf stat` / `perf record` are
  unavailable to any process on this host. `valgrind`/`cachegrind` (which would
  bypass `perf_event` via simulation) is **not installed**. There is currently
  no way to obtain cache/TLB-miss data on this machine — getting it requires a
  host with `CAP_PERFMON`/lower paranoid, or installing valgrind.
- **Ownership:** `SAT-playground-5b2.2.61` ("Profile and cut solver11
  identical-work execution tax") is claimed `in_progress` by `s11-06`. This
  document is **collaborative input for that bead**, not a competing claim. No
  source was edited; no bead was claimed.

What this run *does* deliver: a static, source-level diff of solver 10's
`propagate`/`enqueue` against solver 11's `propagate_impl`/`enqueue`, enumerating
every per-event overhead solver 11 adds on **identical work**, ranked by call
frequency and anchored to s11-06's already-measured throughput gap. This is the
Phase-4 (reference-diff) methodology applied to the solver10↔solver11 pair
instead of kissat↔solver11.

## Executive Summary

- s11-06 established the gap is pure **execution throughput on identical work**:
  Sudoku same 259,775 conflicts / 6,772,770 decisions / 1,312,437,897
  propagations, but solver10 search 169.256s @ 7.754M props/s vs solver11
  178.718s @ 7.344M props/s (**−5.3% props/s, +9.46s**). Kakuro same 732,107
  conflicts / 617,655,456 propagations, 3.795M → 3.737M props/s (−1.5%).
- The solver11 propagate/enqueue path adds **five distinct unconditional
  per-event overheads** that solver10 lacks, none of which are compiled out in
  the single/no-LBD parity config (`HOT_STATS=false, MODE_TICKS=false,
  BINARY_FAST=false`). They are individually tiny (≈1–3 cycles) but multiply by
  the per-event counts (propagations ≈ 1.3B on Sudoku).
- The largest *structural* suspect — richer per-variable / per-clause metadata
  inflating the working set touched per propagation (a cache effect) — is the
  most plausible single cause of a memory-bound props/s drop, but is
  **unconfirmable on this host** (no perf, no valgrind). Flagged as the key open
  measurement.
- s11-06's first experiment (`#[inline(always)]` on the accounting helpers)
  correctly failed: inlining does not *remove* the `accounting_mode` branches,
  it just inlines them. The right lever is **const-generic specialization** of
  the NormalSearch path (same pattern as the existing `HOT_STATS`/`MODE_TICKS`/
  `BINARY_FAST` monomorphization), which compiles the branches out entirely.

## Per-event overhead inventory (solver11 vs solver10, identical-work parity config)

Call-frequency legend (Sudoku): **P** ≈ 1.31B propagations (per trail literal
dequeued), **E** ≈ enqueues (implied literals, a large fraction of P), **C** =
conflicts (259,775 — analysis path, contributes to "search" time but not props/s).

| ID | Site (solver11) | What solver11 does per event | solver10 equivalent | Freq | Compiled out in parity cfg? |
|----|-----------------|------------------------------|---------------------|------|------------------------------|
| T1 | `record_propagation_accounting` (main.rs:4147), called in `propagate_impl` per trail literal (4254) | load `self.accounting_mode`, `is_temporary()` `matches!` discriminant, branch, then `+=1` | bare `self.stats.propagations += 1` (s10 main.rs:1460) | P | **No** — not const-gated |
| T2 | `enqueue` phase save (main.rs:4170) | `if self.accounting_mode.update_phase()` (enum load + match + branch) then conditional write | unconditional `self.saved_phase[var] = target_value` (s10:1431) | E | No |
| T3 | `enqueue` root-trail count (main.rs:4176) | `if current_level == 0 && !self.accounting_mode.is_temporary()` (extra enum load + match) | `if current_level == 0` (s10:1435) | E | No (cheap; gated behind level==0) |
| T4 | `set_reason_ref` (main.rs:3643) → `ReasonCode::from_ref(reason).expect(...)` (main.rs:775) | match ReasonRef variant + overflow compare + bit-or + `Result` wrap + `expect` discriminant branch | bare `self.reason[var] = reason` usize store (s10:1433) | E | No |
| T5 | `note_clause_used_as_propagation_reason` (main.rs:2741), called per enqueue in `propagate_impl` (4332, 4402) | short-circuits on `!normal_search_accounting \|\| !use_lbd` → ~2 bool field loads + branch | nothing | E | No (early-returns, but the loads+branch remain) |
| T6 | every reason *read* via `ReasonCode::as_ref_unchecked()` (main.rs:812 → `as_ref` 796) | NONE compare + tag mask + match + `Result` + `expect` | raw usize compare/use | C (analysis) | No |

Notes:
- **T-ruled-out:** `propagate_binary_implications::<_,_,false>` (main.rs:4219) early-returns `None` and is `#[inline(always)]`, so with `BINARY_FAST=false` it fully compiles away — *not* a tax in the parity config. `record_search_ticks::<false>` (4156) likewise compiles to nothing when `MODE_TICKS=false`. The `HOT_STATS`-gated `watch_scans`/`binary_props`/etc. increments compile out when `hot_stats=false`. These were verified by reading the const-generic guards, not assumed.
- The `propagate()` dispatch `match (hot_stats, mode_use_ticks, binary_fast_path)` (main.rs:4197) runs **once per `propagate()` call**, not per event — negligible.

## Why inlining failed and const-specialization is the lever

s11-06 tried `#[inline(always)]` on `record_propagation_accounting` + the
`SearchAccountingMode` helpers and measured a tiny *regression* (PAR-2 389.844 →
390.611 on the 2-row probe). That is expected: the helpers were almost certainly
already inlined by LTO, and `#[inline(always)]` can mildly perturb codegen. The
branches (T1/T2/T3) survive inlining because they test a **runtime** field
(`self.accounting_mode`).

The existing code already demonstrates the correct pattern: `propagate_impl`,
`propagate_binary_implications`, and `record_search_ticks` are monomorphized over
`const HOT_STATS/MODE_TICKS/BINARY_FAST`, so their gated work *compiles out* when
false. `accounting_mode` is **not** in that scheme — it is read as a runtime enum
on every propagation/enqueue even though it is `NormalSearch` for ~100% of search
(it is only `TemporaryAssumption{..}` during the bounded lucky/probe windows).

**Highest-ROI experiment (for s11-06):** add a `const NORMAL_SEARCH: bool`
parameter to `propagate_impl` (and thread it to `record_propagation_accounting`
and `enqueue`, or introduce `enqueue_normal`/`enqueue_temporary` variants).
Dispatch it from `propagate()` off `!self.accounting_mode.is_temporary()`. In the
`NORMAL_SEARCH=true` monomorphization:
- T1 becomes the bare `self.stats.propagations += 1` (the `is_temporary` branch is `const false`).
- T2 becomes the unconditional `saved_phase` write (`update_phase()` is `const true`).
- T3 becomes the bare `current_level == 0` test.
This removes T1–T3 from the hot path in the common case with zero behavior change
(the `TemporaryAssumption` path keeps the runtime checks via the
`NORMAL_SEARCH=false` monomorphization). It is the concrete realization of the
"const-specialize normal-vs-temporary propagation accounting" idea s11-06 already
noted as their next step.

**Second experiment:** T4/T6 — the `ReasonCode` encode/decode. For `Clause`
reasons (the overwhelming majority) `from_ref` is `CLAUSE_TAG(=0) | clause_idx`
i.e. an identity bit-or wrapped in `Result`+`expect`. Consider a
`set_reason_clause(var, clause_idx)` fast path that skips the enum and the
`Result` for the common Clause case, and a `reason_clause_unchecked` read that
skips `as_ref`'s NONE-compare + tag-match when the caller already knows the slot
is a clause. Measure separately from experiment 1.

## Working-set / cache hypothesis (KEY OPEN QUESTION — unmeasurable here)

A −5.3% drop in *props/s* on memory-bound watcher walking is the classic
signature of a larger per-propagation working set (more cache/TLB misses), not of
a few extra ALU branches. solver11 carries per-variable and per-clause state that
solver10 does not (LBD seen/stamp arrays, learned-clause metadata, the packed
`ReasonCode` vs raw usize, focused/stable bookkeeping fields) — allocated and
laid out even when the *features* are off. If any of these arrays are co-resident
in the cache lines touched during `propagate_impl`'s assignment/decision-level/
reason reads, every propagation pays extra misses.

This cannot be confirmed on this host: `perf` is blocked (`perf_event_paranoid=4`)
and `valgrind`/`cachegrind` is not installed. **To resolve it, run on a host with
`CAP_PERFMON` (or `perf_event_paranoid<=1`)**:
```
perf stat -e cycles,instructions,L1-dcache-loads,L1-dcache-load-misses,dTLB-load-misses \
  ./target/release/sat-solver sudoku.cnf /tmp/proof
```
normalized per propagation, solver10 vs solver11 single/no-LBD. If
`L1-dcache-load-misses / propagation` is materially higher on solver11, the fix
is layout (split hot/cold per-var fields; e.g. beads 5b2.2.18.3 `usize→u32`
per-var shrink and 5b2.2.18.4 LearnedMeta hot/cold split target exactly this) —
not the per-event branch removal above. If misses/prop match, the tax is the
T1–T6 ALU overhead and experiment 1 should recover most of the +9.46s.

## Code-Level Recommendations (ordered by ROI)

1. **`propagate_impl` / `enqueue`: const-specialize the NormalSearch path**
   (removes T1, T2, T3). main.rs:4147 / 4170 / 4176 / 4254. Highest ROI, zero
   behavior change, directly addresses the identical-work tax. → feeds
   `SAT-playground-5b2.2.61`.
2. **`set_reason_ref` / reason reads: Clause fast path** (T4, T6). main.rs:3643 /
   775 / 812. Skip the `Result`+`expect` and enum match for the common Clause
   case. Measure independently of (1).
3. **Confirm or refute the cache hypothesis on a perf-capable host** before
   investing further; it determines whether the remaining gap is layout (beads
   5b2.2.18.3 / 5b2.2.18.4) or ALU.

## Rejected / ruled-out

- `propagate_binary_implications`, `record_search_ticks`, and the `HOT_STATS`
  stat increments are **not** taxes in the parity config — all compile out via
  existing const generics (verified by reading the guards).
- `#[inline(always)]` on the accounting helpers (s11-06's first attempt) — does
  not remove the runtime `accounting_mode` branches; measured a tiny regression.

## Artifact Paths

- This document: `log/analyzesat-2026-05-28-exec-tax/FINDINGS.md`
- Empirical anchor (s11-06's identical-work probe): see `SAT-playground-5b2.2.61`
  bead description — solver10 vs solver11 single/no-LBD Sudoku + Kakuro probes.
- No new benchmark/perf artifacts produced (host contended; perf+valgrind
  unavailable).
