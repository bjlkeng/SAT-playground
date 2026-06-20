# Benchmark Workflows

## Downloading SAT Competition 2025 Benchmarks

Download from `https://benchmark-database.de/?track=main_2025&context=cnf`:

```bash
cd benchmarks
wget -O track_main_2025.uri "https://benchmark-database.de/?track=main_2025&context=cnf"
wget --content-disposition -i track_main_2025.uri
```

Competition scoring is PAR-2: runtime for solved instances plus two times the
timeout for each unsolved instance. Lower is better.

SAT Competition 2025 main-track facts:

- Timeout: 5000 seconds.
- Memory limit: 30 GB RAM.
- Hardware: 8-core Xeon class competition host.
- Winner: kissat-sc2024, PAR-2 2788, 306/400 solved.
- Proof formats: DRAT to LRAT via drat-trim to cake_lpr; DRAT to GRAT via
  gratgen to gratchk; or VeriPB to cake_pb_cnf.

## Concurrent Benchmarking

Routine profiling runs may overlap with other agents if combined solver/bench
CPU usage stays below half the machine. On this AMD Ryzen 5 5600 host, use four
busy cores as the routine upper bound.

- Each `tools/bench.sh` run is single-instance-at-a-time and usually occupies
  roughly one core.
- Before starting, check:

  ```bash
  ps aux --sort=-%cpu | grep -E 'sat-solver|kissat|minisat'
  ```

- Hold off if launching yours would push total solver/bench cores to four or
  more.
- For tight A/B measurements, use quiet cores even if the routine threshold
  would allow overlap.

## Full Or Medium SAT Competition Benchmark Runs

For `benchmarks/sat-comp-2025/` or `benchmarks/sat-comp-2025-medium/`, use a
one-shot cron job so the run survives if the agent session ends.

Use `tools/run_bench_reference.sh` for reference solvers. For an in-repo solver,
use the same cron pattern but invoke `tools/bench.sh` directly and create custom
running/done sentinels if needed.

After the job starts, immediately remove the one-shot cron line so it cannot
re-fire.

### One-Shot Cron Pattern

```bash
# 1. Pick a time about 2 minutes from now.
date '+%M %H %d %m'

# 2. Append one one-shot entry while preserving the existing crontab.
EXISTING=$(crontab -l 2>/dev/null)
echo "$EXISTING
19 21 11 04 * /bin/bash /home/bojji/code/SAT-playground/tools/run_bench_reference.sh -t 1800 -m 16384 -d /home/bojji/code/SAT-playground/benchmarks/sat-comp-2025-medium" | crontab -

# 3. Verify it started after cron fires.
cat log/bench_reference_RUNNING

# 4. Remove the one-shot entry immediately.
crontab -l | grep -v 'run_bench_reference' | crontab -

# 5. Monitor progress.
tail -f log/bench_reference_*.log
wc -l log/bench-kissat-latest-*/results.csv
```

### `run_bench_reference.sh` Flags

- `-t <seconds>`: per-instance timeout, default 1800.
- `-m <MB>`: memory limit, default 16384.
- `-d <path>`: benchmark directory, default `benchmarks/sat-comp-2025`.
- Positional solver names default to `kissat-latest kissat-sc2024 minisat`.

### Progress Files

- `log/bench_reference_RUNNING`: exists while benchmark is active.
- `log/bench_reference_DONE`: created on completion.
- `log/bench-<solver>-<timestamp>/results.csv`: result rows.

### Stopping A Run

Kill both wrappers and solver binaries; children can outlive the parent.

```bash
pkill -f 'bench_reference'; pkill -f 'run_bench_reference'
pkill -f 'kissat.*\.cnf'; pkill -f 'minisat.*\.cnf'
ps aux | grep -E 'kissat|minisat|bench_reference' | grep -v grep
```

## Reference Solver Baselines

See `benchmarks/REFERENCE_SOLVERS.md` for pinned reference solver details,
baseline provenance, and refresh rules.
