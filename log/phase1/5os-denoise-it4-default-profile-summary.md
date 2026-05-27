# SAT-playground-5os: De-noise it4 default-profile PAR-2 improvement

## Question

The `SAT-playground-it4` kept patch only changes `SAT_CLAUSE_MIN=inblock`
behavior, but its final default-profile benchmark looked much faster:

- Before it4: `log/bench-11-kissat-port-2026-05-26-20-35-51/results.csv`
  - 10/10 solved, PAR-2 `918.259`
- After it4: `log/phase1/it4-frame-used-inblock-only-profile-after/results.csv`
  - 10/10 solved, PAR-2 `746.415`

This bead re-ran the same default profile with `SAT_STATS_JSON=on` to determine
whether the speedup was reproducible and explained by work counters.

## New paired run

- Current run: `log/phase1/nextbeads-2026-05-27-before/results.csv`
- Stats: `log/phase1/nextbeads-2026-05-27-before/stats.jsonl`
- Solver code: current `f166fd3` has the same solver code as the it4 after-run;
  the intervening commit only added Beads follow-up records.
- Config hash: `146977fbb156cfb0` in both JSON-stats runs.

Result:

| Run | Solved | PAR-2 |
| --- | ---: | ---: |
| Pre-it4 default | 10/10 | `918.259` |
| it4 after-run | 10/10 | `746.415` |
| Current same-code rerun | 10/10 | `841.149` |

`tools/compare_bench.py` reported no status changes and no status regressions
for all comparisons. The current same-code rerun is `+94.734s` slower than the
it4 after-run, but still `-77.110s` faster than the pre-it4 run.

## Counter check

Comparing
`log/phase1/it4-frame-used-inblock-only-profile-after/stats.jsonl` to
`log/phase1/nextbeads-2026-05-27-before/stats.jsonl`:

- `config_hash` matched on every row.
- `conflicts`, `decisions`, `propagations`, `restarts`,
  `learned_clauses_final`, and `learned_lits_final` matched on every row.
- Statuses matched on every row.

The timing deltas therefore came from execution/runtime variance, not a changed
search trajectory:

| Instance | it4 after | current rerun | Delta |
| --- | ---: | ---: | ---: |
| sudoku-N30-12 | `182.740` | `230.001` | `+47.261` |
| 6s299b685_Iter30 | `15.963` | `17.610` | `+1.647` |
| REGRandom-K4-L1-Seed40 | `56.885` | `59.132` | `+2.247` |
| mp1-Nb7T46 | `42.426` | `46.533` | `+4.107` |
| Kakuro-easy-112-ext | `209.923` | `240.669` | `+30.746` |
| SCPC-500-13 | `13.687` | `13.813` | `+0.126` |
| velev-pipe-sat-1.0-b7 | `65.818` | `72.971` | `+7.153` |
| brocard_problem_large | `8.658` | `8.910` | `+0.252` |
| battleship-16-31-sat | `22.874` | `23.319` | `+0.445` |
| case9 | `127.441` | `128.191` | `+0.750` |

## Conclusion

Treat the apparent `918.259 -> 746.415` default-profile improvement as
single-run timing noise. The it4 patch remains useful as an inblock/shrink
prerequisite, but it should not be cited as a default recursive-limited speedup
unless a future paired run changes work counters or repeatedly reproduces the
same timing effect.
