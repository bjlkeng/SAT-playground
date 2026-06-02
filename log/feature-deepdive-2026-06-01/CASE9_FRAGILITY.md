# Why case9 is fragile — and what it reveals about profile20 verdicts (2026-06-02)

case9 dominated nearly every aggregate verdict in sweep-2 and the seed-sweep (it's every feature's
worst case). User asked: why is this one instance so perturbation-fragile? Read-only investigation
(structure + seed/component isolation, n=5 seeds, 300s).

## What case9 is
- 500 variables, **4502 clauses of which 4101 (91%) are binary**, SAT. Competition "case" family
  (typically circuit/hardware equivalence — gate encodings produce dense binary-implication graphs).
- Near-**regular** implication graph: every variable occurs in ~20 binary clauses (degree 16–24, mean
  20.4), **no high-degree hubs / decision spine**.
- Despite only 500 vars it needs **1.2M–4.2M conflicts** to solve — a deep, narrow solution that
  search must navigate by getting many decisions right in order; any wrong turn cascades through the
  dense binary graph into a long conflict.

## The key discovery: case9 is fragile to the SEED, not to features
Solve rate on case9, n=5 seeds, 300s:

| config | case9 solved | note |
|---|---|---|
| **old single-mode default** (pre-promotion) | **5/5** | 4,186,969 conflicts EVERY seed (deterministic), ~128s |
| solver-10 (selection.csv, 3 runs) | 3/3 | 107/121/211s |
| **new fstab_lbdtier default** | **1/5** | only seed 0 (68s); seeds 1–4 timeout |
| target_phase | 2/5 | |
| binary_fast | 2/5 | |
| chrono | 1/5 | = default (chrono inert here) |

**Consequence:** all prior per-feature case9 claims ("binary_fast loses case9", "target_phase blows
up case9 2.53×") were comparing **single seed-0 draws of a coin-flip instance**. They are seed-noise,
not feature effects. Any sweep verdict that hinged on case9 is unreliable at n=1.

## Component isolation — what made case9 fragile (5 seeds each)
| config | case9 solved |
|---|---|
| old single-mode | 5/5 |
| lbd-tiered reduce ALONE (single-mode) | **0/5** |
| focused-stable, NO lbd-tiered | 3/5 |
| full fstab_lbdtier default | 1/5 |

**Root cause = lbd-tiered reduction.** Alone it makes case9 0/5 — the tiered clause-deletion schedule
discards binary-derived learned clauses that case9's dense binary search needs to retain.
focused-stable's randomized restarts/rephasing add the seed-dependence (deterministic single-mode →
seed-varying). The combination that *won* the promotion (lbd-tiered, which cracked 3 hard instances)
**silently broke a previously rock-solid easy instance.**

## A real (latent) regression in the shipped default
- old single-mode default: case9 **5/5**, deterministic, ~128s
- new fstab_lbdtier default: case9 **1/5**
The default promotion **traded case9 robustness for the aggregate-PAR-2 win**. It was not *wrong*
under the aggregate-PAR-2 policy (at seed-0/600s the new default solves case9 in 68s, faster than
old), but the **single-seed promotion methodology could not see the robustness loss.**

## The bigger methodological finding: profile20 solve-counts are seed-0 artifacts
Of the 13 "default-solved" instances, **3 are seed-fragile** at 300s:
- case9: 1/5 seeds
- sudoku-N30-12: 0/5 seeds (only "solved" in selection via seed-0 / the 1800s budget)
- REGRandom-K4-L1-Seed40: 0/5 seeds

So profile20's headline "default solves 13/20" is **seed-0-specific**; at other seeds the default
solves fewer, and the ±noise band used in sweeps is understated for these instances. This affects the
reliability of any profile20 verdict — including the original promotion's per-instance breakdown.

## Recommendations
1. **Don't promote target_phase** (already concluded) — its case9 "edge" is coin-flip noise.
2. **Record the case9 robustness regression** against the default. Consider whether lbd-tiered's
   reduction should protect binary-derived clauses (kissat keeps a binary tier) — that could recover
   case9 without losing the 3 hard cracks. Worth a focused experiment, NOT a default revert (the
   aggregate win is real).
3. **Methodology fix for future sweeps:** report per-instance solve-rate across ≥3 seeds, not a single
   seed, so seed-fragile instances are flagged and don't silently drive verdicts. Single-seed
   per-instance deltas on this suite are unreliable for case9/sudoku/REGRandom.

Provenance: structural analysis of /tmp decompressed case9; seed/component isolation runs (5 seeds);
seedsweep_results.tsv. Code: lbd-tiered reduction reduce_db (main.rs:5235), effective restart policy
(main.rs:4677).
