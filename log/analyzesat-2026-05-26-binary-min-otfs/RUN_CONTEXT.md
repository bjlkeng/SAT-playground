# AnalyzeSAT Run Context

- target: `solver/11-kissat-port`
- focus: binary implication fast path, clause minimization, and learned-clause OTFS
- benchmark directory: `benchmarks/profiling`
- timeout: 300s
- solver wall limit: 295s
- memory: 16384 MB
- started: 2026-05-26T03:11:23-04:00
- commit: `9143376`
- branch: `side-01-analyzesat-20260526-071123`

This run intentionally avoids retesting the just-covered default lucky pass and unsupported
single-mode Kissat EMA path. The different-place hypothesis is that propagation/minimization
or learned-clause deletion experiments may still hide actionable performance/correctness gaps.
