# AnalyzeSAT Run Context

- Date: 2026-05-26
- Worktree: `/tmp/analyzesat-20260526-083748`
- Branch: `side-01-analyzesat-20260526-083748`
- Base commit: `6d76772 SAT-playground-k25: align minimize depth limit`
- Target solver: `solver/11-kissat-port`
- Focus: decision-layer behavior, specifically `SAT_BRANCH_MODE`, phase policy, and guarded chronological backtracking.

This run intentionally avoids the previous clause-minimization / binary-fast / OTFS and lucky-assignment threads.
