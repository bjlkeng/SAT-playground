# Solver Optimization Workflow

Only run this when the user explicitly asks to optimize a solver or evaluate a
specific performance idea. Do not automatically start this loop after ordinary
implementation work.

## Decision Metric

The decision target is the lexicographic profile20 metric:

1. solved instances across seed runs;
2. total conflicts on tied solved cells;
3. aggregate PAR-2 as supplemental tie-break only.

Use `benchmarks/profile20/README.md` for suite provenance. Single-instance and
single-seed runs are iteration aids, not keep/promote evidence.

## Standard Driver

For solver 11 feature ablations, use `tools/feature_ablation.py`. It handles the
config matrix, same-binary `SAT_*` toggles, `SAT_SEED`, and `requires` deps.

### A/B a candidate against baseline (preferred when iterating)

Compare a candidate feature against baseline in ONE command so both arms start
together and share the pinned cores — no host drift, socket, or thermal bias
between arms:

```bash
python3 tools/feature_ablation.py --arm 'cand:SAT_NEWFEAT=on' --arm 'base:'
```

Repeat `--arm 'tag:ENV'` for N-way (empty env after `:` = solver default; a bare
`CONFIG_MAP` tag such as `solver10` uses its registered config, so you can add the
floor as an arm). Every `(arm, instance, seed)` run is interleaved into one
free-core pool, so both arms run in the same wall-clock window and each spreads
evenly across all cores. The run writes one gate-compatible `results.tsv` per arm,
then prints an inline solved -> conflicts -> PAR-2 verdict and the exact
`check_promotion_gate.py --multiseed` command to confirm the decision.

Defaults are sized for a large multi-core host (36 cores / ~500 GB): `--jobs 32`
(physical cores 0-31), `--mem-mb 16000`, `--timeout 1800` (30 min), `--seeds 10`.
On a constrained host (e.g. ~62 GiB) scale down, e.g. `--jobs 5 --mem-mb 11500`.

### Single-config primitives

The `--arm` run above composes these single-config seedgates; run one directly
when you only need one config measured.

For quiet-host iteration, a common screen is five jobs by five seeds:

```bash
python3 tools/feature_ablation.py --seedgate --configs <tag> \
  --seeds 5 --jobs 5 --mem-mb 11500
```

For an unregistered feature:

```bash
python3 tools/feature_ablation.py --seedgate \
  --env "SAT_NEWFEAT=on" --tag newfeat \
  --seeds 5 --jobs 5 --mem-mb 11500
```

The authoritative keep/turn-on/promote run is N=10:

```bash
python3 tools/feature_ablation.py --seedgate --configs <tag> --seeds 10
python3 tools/check_promotion_gate.py --multiseed ...
```

Before launching a parallel sweep, check for competing solver/bench processes
and ask the user before proceeding:

```bash
ps aux --sort=-%cpu | grep -E 'sat-solver|kissat|feature_ablation|bench'
```

Cap memory so `jobs * mem` fits RAM. On this 62 GiB host, five jobs should use
about `--mem-mb 11500`; the 14000 MB default times five can overcommit.

## Long Seedgate Runs

Long `--seedgate` jobs can run for hours. After preflight and user approval:

- Launch detached or via the cron pattern in `benchmarks/BENCHMARK_WORKFLOWS.md`.
- Record the run directory and driver PID.
- Report roughly hourly while it runs. `feature_ablation.py --seedgate` writes
  `results.tsv` only at the end; mid-run progress comes from live processes and
  `_work/<idx>` scratch dirs.
- Close with comparative analysis: gate output, solve-rate deltas, seed-fragile
  rows, conflicts, PAR-2, and keep/promote recommendation.

## Iteration Loop

1. Pick a fast target instance only for iteration speed. The decision remains
   profile20 aggregate evidence.
2. Capture baseline and candidate together with one `tools/feature_ablation.py
   --arm` A/B run (see Standard Driver). For a quick single-solver PAR-2 snapshot:

   ```bash
   bash tools/bench.sh -j 4 -d benchmarks/profile20 solver/NN-name
   ```

3. Profile before coding. Use `perf stat`, `perf record`, and `perf report` when
   available. For symbols in release builds:

   ```bash
   CARGO_PROFILE_RELEASE_STRIP=false \
   CARGO_PROFILE_RELEASE_DEBUG=1 \
   RUSTFLAGS="-C target-cpu=native" \
   cargo build --release
   ```

4. Check opportunity size before coding simplification ideas: duplicate clauses,
   pure literals, candidate variables, binary subsumption hits, SSR opportunities,
   and similar measured openings.
5. Make one focused change at a time.
6. Use fast single-instance or single-seed runs only as smoke signals while
   coding.
7. Keep a change only after it wins the multiseed lexicographic gate beyond
   seed noise and passes correctness checks. The `--arm` A/B prints this
   solved -> conflicts -> PAR-2 verdict inline; confirm with
   `check_promotion_gate.py --multiseed`.
8. Revert changes that lose, tie only through noise, or trigger correctness
   failures.
9. Stop long losers early when finished rows have already lost more than the
   remaining rows can plausibly recover.
10. Tune promising algorithmic features empirically; more simplification or more
    search machinery can damage CDCL trajectory.
11. Retest previously rejected micro-optimizations only when profiler evidence
    suggests the context changed.
12. Document successful improvements in the solver README with benchmark log
    paths, profile paths, machine metadata, and measured impact. Also record
    important rejected attempts so future loops do not repeat them.

## Diagnostic Stats To Capture

For every instance analysis, capture enough data to identify the bottleneck:

- pre/post-preprocessing variables, clauses, and literals;
- preprocessing time;
- eliminated variables, resolvents, subsumed clauses, strengthened literals, and
  root assignments when available;
- final result and runtime;
- conflicts, decisions, propagations, restarts, learned-clause count, reduce-DB
  calls, timeout/error status;
- propagation throughput, conflict-analysis time, simplification/proof I/O time
  when measured;
- exact config flags, seeds, order/literal-order choices, preprocessing toggles,
  profile paths, and log paths.

## Scientific Deep Dives

Use `/analyzesat` for the full bottleneck workflow: multi-config ablation, work
times speed decomposition, reference-source diff, trajectory trace analysis,
FINDINGS/DEEPER_FINDINGS artifacts, and bead creation.
