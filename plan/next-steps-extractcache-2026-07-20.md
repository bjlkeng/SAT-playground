# Extract-cache promotion (2026-07-20 session)

## Outcome

**PROMOTED: `SAT_EXTRACT_CACHE` (default on) — wall-diet arc win #8, commit
c8228aa.** Gate `log/abtest-cand-vs-base-2026-07-20-12-03-06` (launch log
`log/abtest-extractcache-launch.log`), arms `cand:` vs
`base:SAT_EXTRACT_CACHE=off`: **PASS, WIN — solved 69 vs 68 (oski20 FLIP:
cand UNSAT 1692.3s vs base TIMEOUT), both-solved conflicts EXACT tie (delta
0 over all 68 both-solved cells), PAR-2 138,391.4 vs 140,258.6 (−1,867.2)**.
`check_promotion_gate` formal PASS; zero contradictions, zero correctness
failures. **Lineage cell count is a REALIZED 69 again** (oski20 back in;
last gate it was out on a contention coin-flip).

In-gate wall margins (base minus cand, conflicts identical): 5e933a +42.2s,
3746303c +41.1s, vex +34.0s (1494.2s), oski40 +22.5s (1237.8s), 663bb565
+21.4s. Wobbles (not mechanism — conflicts identical): rbsat −65.8s (cand
1794.6s, **margin now 5.4s — thinnest ever, treat as coin-flip**), sted2
−63.4s (1636.4s), 31e843c5 −53.7s.

## The change (solver/12-kissat-inprocessing)

One knob `SAT_EXTRACT_CACHE` (default on; `off` = shipped
`extract_gates_for_congruence_flat` verbatim, the fair A/B arm):

1. **Per-clause AND/ITE gate cache** (`congruence::ExtractCache`), scoped to
   ONE `try_congruence` invocation. Each large clause's AND/ITE gates are
   spliced from the previous round's extraction unless
   `vars(clause) ∩ touched ≠ ∅`. `touched` = full var sets of **len<=3**
   clauses installed (`els_install_original_clause` hook), deleted
   (`delete_clause_for_simplify` hook), or newly extraction-dead (assigned/
   eliminated var since last extraction, detected in the scan via a precise
   `nonlive_delta` was-dead-before test).
2. **The len<=3 scoping is the load-bearing insight**: AND gates depend on
   neighbor BINARIES, ITE gates on neighbor TERNARIES, and the only len-4
   consumer (XOR families) is recomputed fresh every extraction; a len>=4
   clause carries only its own gates, keyed by its own cid. Without the
   len<=3 scope, ELS rewrites of long BMC clauses invalidated ~69% of
   clauses per round (reuse 364k/1.17M); with it, reuse = 1.15-1.17M/1.17M
   (98-99.9%).
3. **Persistent index workspaces** (binaries/ternary sets, large pool,
   learned-short list) cleared not reallocated, and `pair_thirds` as a flat
   **tail-appended** chain through one Vec (iteration order == shipped
   per-pair Vec push order; head-insertion would reverse candidate order and
   reroll — and per-key bucket Vecs are the known closure-diet trap).
4. Wrapper `try_congruence` → `try_congruence_inner` disarms the hooks on
   every return path; cache invalidated wholesale at every entry (outside
   edits unhooked); giant turn-off path releases the cache (simp.rs,
   ElsCsrWs precedent). No GC runs inside the invocation window (verified);
   arena cids are stable within it.
5. `SAT_EXTRACT_CACHE_VERIFY=1`: runs the shipped full extraction after
   every cached one and asserts the gate streams identical. Full-ibm run:
   27/27 extractions identical. `SAT_TRACE_EXTRACT=1`: per-extraction
   scan/and/ite/xor timing + reuse counts (works in both paths).

## Why it's identity-safe (the argument, reusable)

In-place literal reorders only happen via root-propagation watch swaps, and
only in clauses containing a newly assigned var — such clauses are
extraction-dead afterwards and enter `touched` through the newly-dead path.
ELS replaces rewritten clauses positionally and appends new clauses at the
end, so untouched clauses keep their scan order, and representative-is-
first-seen closure semantics are preserved. Splice therefore equals
recompute exactly; verified at full formula scale by the VERIFY mode and by
byte-identical stripped stats across cand-on / cand-off / pre-change
binaries on ibm (full SAT), vex @300k conflicts, bubble @1.5M conflicts.

## Measured basis (SAT_TRACE_EXTRACT, ibm)

- Shipped per-round extraction split: scan/index 0.79s, AND 0.085s, ITE
  0.50s, XOR 0.09s (visible 1.46s of the 2.06s step; the remainder is
  alloc/drop churn incl. per-key pair_thirds Vecs).
- ibm baseline extraction total: 19.0s + 4.0s dry-run of 131s wall; rounds
  2-7 re-extract a formula whose gate count moves <1% (871-878k gates).
- Cached rounds: 0.70s visible (scan 0.46 + and 0.12 + ite 0.035 + xor
  0.08). Standalone paired screens: ibm −8.8% (130.2 vs 142.7s), vex −4.5%
  (280.9 vs 294.2s), bubble neutral.

## Notes / residue

- Dry-run-only invocations pay a small pool-population overhead (gates
  written twice on the recompute path) — priced into the gate, net win.
- The cache is WITHIN-invocation only. Cross-invocation caching stays
  blocked on lit-order sensitivity (search moves lits between invocations);
  canonicalization (sort clause lits) would unblock it AND make extraction
  order-insensitive, but is a full-suite reroll — that remains the separate
  deliberate-reroll play (aggregated plan #1 alternative).
- rbsat margin is now 5.4s. Do NOT build anything on rbsat being in.

## Where the evidence lives

- Gate: `log/abtest-cand-vs-base-2026-07-20-12-03-06` + launch log
  `log/abtest-extractcache-launch.log`; formal check output in session log.
- Commit: c8228aa. Baseline TSV for the NEXT A/B:
  `log/abtest-cand-vs-base-2026-07-20-12-03-06/cand/results.tsv` (69/100,
  oski20 IN at 107.7s margin, rbsat IN at 5.4s margin).
- Screens: scratchpad (dies on reboot); all decision-relevant numbers above.
