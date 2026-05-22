# Learned Clause Initial Use Count

## Bead

`SAT-playground-ajc` - Fix: high-LBD learned clauses get lower initial `used_recently` than Kissat.

## Change

Solver 11 previously initialized newly learned clauses with tier-dependent recent-use protection:

```text
LBD <= 6: used_recently = MAX_USED_RECENTLY
LBD > 6:  used_recently = 1
```

Kissat initializes every learned clause with its maximum `used` value, regardless of glue. Solver
11 now mirrors that behavior with its current smaller counter range by setting every newly learned
clause to `MAX_USED_RECENTLY` when LBD metadata is initialized.

The tier classification itself is unchanged:

```text
LBD <= 2: tier1
LBD <= 6: tier2
LBD > 6:  tier3
```

Only the initial retention counter changed. Later reduce-DB passes still age the counter down and
can delete high-LBD clauses after they become unused.

## Example

With decision levels arranged so a learned clause has LBD 7:

```text
before: tier3, used_recently = 1
after:  tier3, used_recently = 3
```

That gives the new high-LBD clause the same initial retention window as a new low-LBD clause. The
reduction policy, not the initial tier, decides when the clause has aged enough to be evictable.

## Fresh-Eyes Review

Reviewed the learn path, LBD metadata side table, reduce-DB candidate collection, recent-use aging,
reason-use marking, docs, and plan text after the edit.

Findings and handling:

- The code-side change is intentionally scoped to `initialize_learnt_lbd`; reduce-DB aging and
  `mark_learned_clause_recent` remain unchanged for the later reduction-policy beads.
- The original roadmap text still described the old `tier3 -> 1` initialization, so the plan was
  updated to avoid giving future agents contradictory guidance.
- A first invariant smoke command was run in parallel with standard smoke and both selected the
  same timestamped log directory; the invariant run had one bogus missing-output failure from that
  log collision. The invariant smoke was rerun by itself and passed.

## Validation

Commands:

```bash
cargo fmt --check
cargo test test_learned_clause_initial_used_recently_is_max_for_all_lbd_tiers -- --nocapture
cargo test reduce_db -- --nocapture
cargo test
bash tools/smoke_test.sh solver/11-kissat-port
SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port
SAT_USE_LBD=on SAT_REDUCE=lbd-tiered bash tools/smoke_test.sh solver/11-kissat-port
SAT_USE_LBD=on SAT_LBD_UPDATE_REASONS=on SAT_RESTART=kissat-ema \
  SAT_REDUCE=lbd-tiered SAT_PHASE=target-then-saved SAT_BINARY_FAST=on \
  SAT_SEARCH_MODE=focused-stable SAT_CLAUSE_MIN=basic SAT_VMTF=on \
  bash tools/smoke_test.sh solver/11-kissat-port
bash tools/bench.sh -m 16384 -d benchmarks/profiling \
  --log-dir log/1.14-learned-used-init/profile-default-300 solver/11-kissat-port
```

Results:

```text
cargo fmt --check: passed
focused unit test: passed
reduce_db tests: 6/6 passed
cargo test: 251/251 passed
standard smoke: 9/9 passed
invariant smoke rerun: 9/9 passed
LBD-tiered smoke: 9/9 passed
advanced focused/VMTF smoke: 9/9 passed
profile default: 11/11 solved, PAR-2 631.833
```

Profile artifact:

- `log/1.14-learned-used-init/profile-default-300/results.csv`
- `log/1.14-learned-used-init/profile-default-300/summary.log`

The default profile result is between the two immediately preceding default-profile results:

```text
log/1.14-target-phase-preserve/profile-default-300:       PAR-2 629.384
log/1.14-ema-reset/profile-default-300-rerun:             PAR-2 634.365
log/1.14-learned-used-init/profile-default-300:           PAR-2 631.833
```

This confirms no default-profile regression. A direct performance improvement is not expected in
default mode because `SAT_USE_LBD` and `SAT_REDUCE=lbd-tiered` are still opt-in.
