# Deeper Findings: Why `SAT_REDUCE=lbd-tiered` Fails mp1

## Question

Does the current opt-in LBD-tiered reducer regress because of LBD computation overhead, reason-LBD metadata, reduction schedule, or learned-clause deletion semantics?

## Answer

The reducer fails because its deletion semantics perturb search heavily. LBD computation and reason-side metadata are not enough to explain the failure.

The decisive mp1 evidence:
- Default: SAT in 45.384s, 425,229 conflicts.
- Reason-LBD only: SAT in 46.867s, same 425,229 conflicts.
- LBD-tiered: `UNKNOWN` at 296.225s, 2,435,758 conflicts, 237 reductions, 2,245,127 learned clauses collected, 58 GCs.
- LBD-tiered without reason-LBD: `UNKNOWN` at 296.142s, 2,623,810 conflicts, 249 reductions.
- Delayed first reduce: `UNKNOWN` at 296.200s.
- Slow interval: `UNKNOWN` at 296.459s.

The reason-LBD-only row has identical conflict/decision/propagation counts to default and only a 3.2% propagation-speed cost. The reducer rows multiply conflicts by 5.5x to 6.2x and still do not solve.

## Work x Speed

For mp1:
- `D_lbd_tiered`: wall 6.527x, work 5.728x, speed 1.192x, net 6.828x.
- `I_lbd_tiered_no_reason`: wall 6.525x, work 6.170x, speed 1.140x, net 7.037x.
- `H_lbd_tiered_slow_interval`: wall 6.532x, work 5.718x, speed 1.146x, net 6.555x.

For REGRandom:
- `D_lbd_tiered`: wall 1.662x, work 1.700x, speed 0.997x.
- `H_lbd_tiered_slow_interval`: wall 1.148x, work 0.918x, speed 1.209x.

So REGRandom can be partially helped by less frequent reduction, but mp1 cannot. That means schedule knobs are insufficient.

## Trace Diagnosis

Short mp1 traces used:

```text
SAT_TRACE_SEARCH_INTERVAL=100000 SAT_LIMIT_WALL_SEC=120
```

Default trace:
- 100k conflicts at 9.426s, 348,830 decisions, 67,389,209 propagations.
- 400k conflicts at 43.524s, 1,333,745 decisions, 272,783,474 propagations.
- SAT at 425,229 conflicts and 46.723 search seconds.

Tiered trace:
- 100k conflicts at 9.898s, same 348,830 decisions and 67,389,209 propagations, but already 28 reduce-DB calls and only 47,142 live learned clauses.
- 400k conflicts at 46.771s, 1,332,102 decisions, 288,610,443 propagations, 71 reduce-DB calls, 125,171 live learned clauses.
- 900k conflicts at 109.897s, 2,804,720 decisions, 633,945,766 propagations, 122 reduce-DB calls.
- `UNKNOWN` at the 120s trace limit with 977,288 conflicts, 129 reductions, 768,304 learned clauses collected, and 32 GCs.

The two runs are initially aligned, then diverge after repeated reductions. Default solves just after the 400k snapshot; tiered is already on a different path and continues for more than twice as many conflicts before the 120s trace cutoff.

## Code-Level Gap

Local `reduce_db_lbd_tiered`:

```text
solver/11-kissat-port/src/main.rs:6032
```

The local reducer sorts candidates and then deletes until `projected_lits <= learned_lit_budget`. Its budget starts at 2,000 learned literals plus 300 times the square root of reduction count, with a separate hard guard. This makes deletion pressure depend on total learned literal volume, not a fraction of the current reducible set.

Kissat reference:

```text
https://github.com/arminbiere/kissat/blob/master/src/reduce.c
```

Kissat computes a target fraction from `reducehigh`, `reducelow`, and reduction count, then deletes the first `target` candidates after sorting reducibles. The reference also protects clauses based on `used` and tier, and updates the next reduce limit with a square-root schedule, but it does not use a learned-literal budget as the normal scheduled deletion target.

This matters because mp1 shows the local reducer deleting millions of learned clauses while still retaining a bad set for the CDCL trajectory. The no-reason and schedule probes make this a deletion-semantics issue.

## Why Existing Beads Are Sufficient

`SAT-playground-qmz` already describes the fraction-based deletion gap. This run adds current-HEAD evidence and stronger failing rows. Creating a duplicate bead would split the evidence. `SAT-playground-z70` and `SAT-playground-5b2.2.44` should remain blocked or second-order until `qmz` lands and is retested.

## Rejected Hypotheses

Reason-LBD recomputation:
- Refuted for root cause by `I_lbd_tiered_no_reason`, which still returns `UNKNOWN` on mp1.

First reduction too early:
- Refuted as a complete fix by `G_lbd_tiered_delayed`, which still returns `UNKNOWN` on mp1.

Too frequent later reductions:
- Refuted as a complete fix by `H_lbd_tiered_slow_interval`, which still returns `UNKNOWN` on mp1 even with only 42 reductions.

Pure per-event overhead:
- Refuted by work ratios. The failure is dominated by conflict growth; speed ratios are secondary.

## Next Implementation Slice

Implement `SAT-playground-qmz` in the smallest possible form:

1. Add `SAT_REDUCE_LOW` and `SAT_REDUCE_HIGH` config defaults matching Kissat-style 50/90 percent.
2. In scheduled `reduce_db_lbd_tiered`, compute target deletion count as a fraction of sorted candidates.
3. Keep the hard learned-literal budget only for emergency deletion pressure.
4. Preserve reason-pinning, binary/locked protection, and used/tier keep rules.
5. Add unit tests for low/high fraction behavior and emergency behavior.
6. Rerun:
   - smoke suite
   - mp1 target
   - REGRandom target
   - full profiling suite with `SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_REDUCE=lbd-tiered`

The acceptance gate should explicitly fail if mp1 remains `UNKNOWN` where default solves.
