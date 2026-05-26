# Deeper Findings - Binary Fast and Clause Minimization

## 1. Recursive Minimization Is a Status-Safety Dependency on Sudoku

The first different-place test was deliberately simple: keep the current default
search/preprocessing path, but turn off learned-clause minimization.

Result:

- default recursive: UNSAT in `207.092s`
- `SAT_CLAUSE_MIN=off`: `UNKNOWN` at `295.880s`

This is not just a small local primitive cost. The solver learns materially worse
clauses:

- final learned literals: `29,406,410` -> `65,973,772`
- max conflict-clause buffer: `1,698` -> `7,068`
- proof-added literals: `49,620,264` -> `85,226,560`
- restarts: `617` -> `764`

The measured wall ratio (`1.429`) matches the work-speed product (`1.427`), so
the cause is not hidden I/O or GC. It is the direct work-shape effect of weaker
learned clauses plus slower per-propagation throughput.

## 2. The Binary-Fast Default Is Confounded by an Unsafe Config Rewrite

`SAT_BINARY_FAST=on` currently rewrites `SAT_CLAUSE_MIN` to off when the user did
not set minimization explicitly:

```rust
if self.binary_fast_path && !clause_min_explicit {
    self.clause_min_mode = ClauseMinMode::Off;
}
```

That was probably conservative when binary reasons were new, but it now creates
a misleading benchmark mode:

- `SAT_BINARY_FAST=on`: `UNKNOWN` at `295.937s`
- `SAT_BINARY_FAST=on SAT_CLAUSE_MIN=recursive-limited`: UNSAT in `256.316s`

So `SAT_BINARY_FAST=on` is currently measuring two things at once:

1. the binary implication adjacency path
2. the loss of recursive learned-clause minimization

This makes binary-fast performance artifacts hard to interpret and can convert a
baseline-solved row into `UNKNOWN` without the user asking to disable
minimization.

Recommended fix:

- Remove the implicit minimization-off rewrite now that binary-reason-aware
  minimization exists, or
- reject `SAT_BINARY_FAST=on` without explicit `SAT_CLAUSE_MIN` until a safe
  default is chosen, or
- make the default explicit in profiles and benchmark gates so `SAT_BINARY_FAST`
  no longer silently means "binary-fast plus no minimization."

The first option is likely the best default for correctness/performance hygiene:
preserve recursive minimization unless a user explicitly asks for a no-min
ablation.

## 3. Binary-Fast Has a Real Execution-Cost Problem After the Config Confound Is Removed

With recursive minimization restored, binary-fast solves Sudoku but remains
slower:

- default recursive: `207.092s`
- binary-fast recursive: `256.316s`

The work/speed split is useful:

- conflicts improve: `259,775` -> `230,401` (`0.887x`)
- propagations improve: `1.312B` -> `1.151B` (`0.877x`)
- learned literals improve: `29.4M` -> `25.9M` (`0.882x`)
- propagation throughput worsens: `6.47M/s` -> `4.57M/s`

This validates the existing binary-fast throughput beads rather than replacing
them. The next binary-fast work should be execution/layout work, not search
trajectory tuning. The new bead `SAT-playground-a0f` is linked to
`SAT-playground-5b2.2.18.18`; the other binary hot/cold and deletion-check order
beads are also relevant.

## 4. The Kissat Depth Alignment Bead Looks Safe on This Critical Row

Existing bead `SAT-playground-k25` proposes changing the recursive minimization
depth cap from `1_000_000` to Kissat's `1000`.

The concern after this run was whether Sudoku needs deep recursion. The answer
from the targeted run is no:

- default depth `1_000_000`: UNSAT in `207.092s`
- depth `1000`: UNSAT in `207.935s`
- conflicts, propagations, learned-lits, max clause buffer, and proof-added
  literals are identical

This is direct supporting evidence for `SAT-playground-k25`, not a new issue.

## 5. Feature Ledger Drift Can Mislead Future Analysis

The code and README now agree that `SAT_LUCKY` is opt-in:

- `src/config.rs:628` default `lucky=false`
- `src/config.rs:741-745` default/fast leave lucky false
- README says default/fast profiles keep lucky off

But `FEATURES.csv` and `FEATURES.md` still list `SAT_LUCKY` promoted for
`default|fast` and cite the old lucky-on artifact. This does not affect solver
behavior, but it does affect `SAT_CONFIG_DUMP`, human triage, and future
AnalyzeSAT preflight reads. New bead: `SAT-playground-otk`.

## Rejected or Deferred Work

- Full binary/minimization/OTFS matrix: stopped after `SAT_CLAUSE_MIN=off`
  produced a baseline-solved `UNKNOWN`. Continuing the matrix would violate the
  repo's UNKNOWN policy.
- OTFS conclusions: not reached in this run because the earlier minimization
  boundary failed first. OTFS should be retested only after the binary-fast
  minimization config confound is fixed.
- Hardware counters: blocked by host perf settings.
