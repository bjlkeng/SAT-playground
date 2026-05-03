# CLAUDE.md — SAT-playground Development Guide

## Project Overview

This repo builds Boolean SAT solvers iteratively in Rust, one directory per iteration, all conforming to the SAT Competition 2025 interface. Each iteration adds a technique on top of the previous one.

## Build & Run

```bash
# Build any iteration
cd solver/NN-name && bash build.sh        # runs: cargo build --release

# Run on a CNF instance
bash run.sh path/to/instance.cnf /tmp/proof_dir

# Run benchmarks
bash tools/bench.sh solver/NN-name
```

## Website / Docs Workflow

The repo has a static benchmark site under `docs/`, deployed at:

- **Live page:** `https://bjlkeng.io/SAT-playground/`
- **Main page source:** `docs/index.html`
- **Generated benchmark data:** `docs/data/medium-par2.json`
- **Generated README chart asset:** `docs/assets/medium-cumulative.svg`
- **Solver detail pages:** `docs/solvers/01-naive-dpll.html`, `docs/solvers/02-cdcl.html`
- **Shared solver-page stylesheet:** `docs/solver-pages.css`

### Current site conventions

- The hero title is **“Fun with Boolean SAT”**
- The intro paragraph should:
  - include one sentence describing Boolean SAT with a link to the Wikipedia page
  - say the goal is to understand SAT solvers more deeply
  - mention that all code in the repo was generated with AI coding tools
- The benchmark language on the site currently refers to:
  - **100 randomly selected instances from the SAT Competition 2025 main-track benchmark set**
  - local benchmark limits of **1800 seconds** and **16 GB RAM**
  - SAT Competition 2025 output/proof format via the official output page
- The main page theme is now light grey / white with blue and red accents
- Fixed-width / monospace text is intentionally kept for labels, legends, and benchmark metadata
- The main page section formerly called “Benchmark Logs” is now **“Solver Information”**
- The small machine footnote currently describes this host:
  - AMD Ryzen 5 5600 (6 cores / 12 threads)
  - 62 GiB RAM reported by `free -h`

### Site data generation

Regenerate the site payload and README chart with:

```bash
python3 tools/build_site_data.py
```

That script is the canonical source for:

- latest medium-run selection per solver
- benchmark metadata shown in the site notes
- `infoUrl` / `sourceUrl` used by the Solver Information cards
- the static SVG chart embedded at the top of `README.md`

### Current benchmark/site assumptions

- The site uses the **latest available** run per solver matching:
  - `100` instances
  - `1800s` timeout
- Repo solvers currently included in the site:
  - `01-naive-dpll`
  - `02-cdcl`
- `03-bcp` is skipped automatically until a matching medium run exists
- The cumulative chart is interactive:
  - hover with no selection shows an all-solvers overview
  - clicking a line or legend card locks focus to that solver
  - solver-focused hover shows per-instance details and rank on that instance
  - the tooltip disappears when the mouse leaves the chart area

### Solver information links

- Repo solvers (`01`, `02`) should link from the main page to the local detail pages under `docs/solvers/`
- Reference solvers should link to their appropriate external or vendored source page via `infoUrl`
- The `solver source` buttons in the cards should point to GitHub directory **tree** URLs, not `blob` URLs

### Updating solver detail pages

When adding or revising local solver pages:

- Keep them as simple static HTML pages under `docs/solvers/`
- Include:
  - a brief description of the technique(s) implemented
  - links to Wikipedia / papers where appropriate
  - high-level pseudocode
  - a short “code-level optimization diffs” section derived from the solver README
- `01-naive-dpll` and `02-cdcl` already have pages and can be used as the style/template baseline

### README integration

`README.md` now links to the live site and embeds `docs/assets/medium-cumulative.svg` near the top.

If the benchmark sample, chart styling, or site framing changes materially, update all three together:

1. `docs/index.html`
2. `tools/build_site_data.py`
3. `README.md`

### Verification for site changes

At minimum:

```bash
python3 tools/build_site_data.py

python3 - <<'PY'
from pathlib import Path
html = Path('docs/index.html').read_text()
start = html.index('<script>') + len('<script>')
end = html.index('</script>', start)
Path('/tmp/sat-playground-site.js').write_text(html[start:end].strip() + '\n')
PY
node --check /tmp/sat-playground-site.js
```

### Tracking benchmark logs used by the site

If the user wants the exact benchmark logs committed for the site:

- track the exact `summary.log` and `results.csv` used to generate the charts
- `log/` is ignored, so use `git add -f ...` for those files

Those tracked log files are the provenance for the plotted solver runs.

## Solver Interface Contract (SAT Competition 2025)

Every iteration MUST provide `build.sh` and `run.sh` at its top level:

- **`build.sh`**: No arguments. Builds the solver binary.
- **`run.sh <cnf_path> <output_dir>`**: Runs the solver. Prints to stdout. Writes `proof.out` to `<output_dir>` when UNSAT.

### Required stdout format

```
s SATISFIABLE
v 1 -2 3 0
```

or

```
s UNSATISFIABLE
```

or

```
s UNKNOWN
```

Rules:
- Exactly one `s` line per run
- `v` lines only when SAT — space-separated literals, terminated by `0`, max 4096 chars/line
- `c` comment lines are allowed anywhere
- Partial assignments are fine as long as every clause is satisfied

### UNSAT proofs

Write DRAT proof to `<output_dir>/proof.out`. This is required from every iteration.

## Code Conventions

- **Language:** Rust (each iteration is its own Cargo project)
- **Binary name:** `sat-solver` (consistent across iterations for tooling)
- **No external SAT solver dependencies** — the point is to build from scratch
- **Allowed crates:** standard utility crates (clap, anyhow, etc.) are fine; no SAT/SMT libraries
- **Each iteration directory is self-contained** — copy-and-modify from the previous iteration, don't use workspace dependencies between iterations
- **Test with small hand-crafted CNF files first**, then graduate to competition benchmarks

## Development Rules

- **Use red-green TDD for solver changes** — add or update a failing test first when practical, then implement until it passes before moving on.
- **Run smoke tests after every change** to a solver: `bash tools/smoke_test.sh solver/NN-name`
- **Only commit solver changes that pass the smoke test** (all 8 tests green). If a test fails, fix the solver before committing.
- **Never modify `tools/smoke_test.sh`** unless the user explicitly asks for changes to it.
- **Always commit and push** when the user asks — don't skip the push step.
- **Discord automatic task notices:** For future background ACP/subagent tasks in Discord, suppress automatic `Background task done/failed` channel notices by setting the task notify policy to `silent` once the run/task id exists.
- **Discord notifications:** When reporting background task completion or failures in Discord, @mention bjlkeng as `<@817490773179760662>` so Discord shows a badge notification.

## Iteration Workflow

When creating a new iteration:

1. Copy the previous iteration directory: `cp -r solver/NN-prev/ solver/MM-name/`
2. Update `Cargo.toml` package name
3. Add or update tests first when practical so the change follows a red-green TDD loop
4. Implement the new technique
5. Add unit tests for the new feature
6. Run `bash tools/smoke_test.sh solver/MM-name` — all 8 tests must pass
7. Run against benchmarks and record results in the iteration's `README.md`
8. Ensure `build.sh` and `run.sh` still work

## Testing

```bash
# Unit tests within an iteration
cd solver/NN-name && cargo test

# Smoke test — runs all 8 test instances (4 SAT + 4 UNSAT)
bash tools/smoke_test.sh solver/NN-name
```

### Smoke Test Suite

Located in `tests/cnf/`, these are small hand-crafted instances that run in under a second:

**SAT instances** (`tests/cnf/sat/`):
- `unit.cnf` — single unit clause (trivial)
- `two_clause.cnf` — 2 vars, 2 clauses
- `three_sat.cnf` — 5 vars, 6 clauses (small 3-SAT)
- `all_positive.cnf` — 3 vars, all positive literals

**UNSAT instances** (`tests/cnf/unsat/`):
- `contradiction.cnf` — x AND NOT x
- `empty_clause.cnf` — contains an empty clause
- `pigeonhole_3_2.cnf` — 3 pigeons, 2 holes (classic)
- `chain_unsat.cnf` — implication chain forcing contradiction

The smoke test script (`tools/smoke_test.sh`) builds the solver, runs all instances, checks the `s` line, and verifies SAT assignments satisfy the formula.

## DIMACS CNF Format Reference

```
c optional comment
p cnf <num_vars> <num_clauses>
<lit> <lit> ... 0        ← each line is one clause
```

- Variables: positive integers `1..num_vars`
- Literals: variable or its negation (e.g., `3` or `-3`)
- Clause: list of literals terminated by `0`
- No clause may contain both `x` and `-x`

## Benchmarks

Download from: `https://benchmark-database.de/?track=main_2025&context=cnf`

```bash
cd benchmarks
wget -O track_main_2025.uri "https://benchmark-database.de/?track=main_2025&context=cnf"
wget --content-disposition -i track_main_2025.uri
```

Competition scoring is PAR-2: sum of runtimes for solved instances + 2 × 5000s for each unsolved instance. Lower is better.

## Key SAT Competition 2025 Facts

- **Main Track:** 5000s timeout, 30 GB RAM, 8-core Xeon, PAR-2 scoring
- **Winner:** kissat-sc2024 (PAR-2: 2788, 306/400 solved)
- **Proof formats:** DRAT → LRAT (via drat-trim) → cake_lpr; or DRAT → GRAT (via gratgen) → gratchk; or VeriPB → cake_pb_cnf
- **Benchmark source:** https://benchmark-database.de/?track=main_2025&context=cnf

## Code-Level Optimization Workflow

**Only run this when the user explicitly asks for it.** Do not automatically optimize after implementing a solver.

After implementing a new solver iteration, this optimization loop squeezes out performance. Most
passes should be non-algorithmic, but if the user explicitly asks to optimize using a named solver
idea or benchmark gap, it is acceptable to test that focused feature as part of the loop.

### Procedure

1. **Pick a target**: If the user names a reference solver or competition gap, choose one concrete competition instance where the reference is faster than the repo solver but still short enough for repeated iteration. Create a one-instance benchmark directory for it and keep using that target until the user redirects.
2. **Baseline**: Run `bash tools/bench.sh -t 120 -d benchmarks/profiling solver/NN-name` and record PAR-2. For a one-instance target, also run `bash tools/bench.sh -t <seconds> -m 16384 -d <one-instance-dir> solver/NN-name` and record the target-instance runtime. If comparing to MiniSat-style simplification, also capture useful reference variants such as `minisat -no-pre`, `minisat -no-elim`, and full `minisat`.
3. **Profile first**: Use a profiler such as `perf stat` / `perf record` / `perf report` on the target instance before changing code. Use the profile to choose the next implementation slice. If the release binary is stripped and symbols are needed, rebuild for profiling with `CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=1 RUSTFLAGS="-C target-cpu=native" cargo build --release`.
4. **Check opportunity size before coding**: For simplification ideas, do a quick measurement pass over the target formula before implementing. Examples: count duplicate clauses, pure literals, candidate variables, binary subsumption hits, or self-subsuming-resolution opportunities. Do not implement an idea when the measured opportunity is negligible.
5. **Iterate** (at least 10 attempts unless the user asked for a narrower experiment): Make one change at a time, benchmark it against the current accepted baseline, and keep it only if it improves the target metric by more than `3%`. Recalculate the accepted baseline and keep threshold after every kept change. Revert changes that improve by `3%` or less, regress, fail correctness checks, or only shift time into a slow new implementation.
6. **Stop losers early when safe**: For long target-instance runs, if an experiment has already passed the `>3%` keep cutoff without finishing, stop the run, mark the attempt rejected, and revert it instead of waiting for the full timeout.
7. **Tune algorithmic features carefully**: When testing a focused solver idea such as bounded variable elimination, run small parameter sweeps around the first good result. More simplification can damage CDCL search even when preprocessing is cheap, so validate caps and cost thresholds empirically rather than assuming monotonic improvement.
8. **Retest interactions**: A previously rejected micro-optimization can become worthwhile after a later kept change changes the profile. Retest it only when profiler evidence or timing data suggests the interaction could now clear the `>3%` threshold.
9. **Optimize the new feature if needed**: If a feature is conceptually promising but too slow, profile the modified solver and iterate on that feature's implementation. Keep it only after the optimized version clears the `>3%` improvement threshold.
10. **Correctness checks**: Add or update focused tests when practical before changing solver behavior. Run `cargo test` and the full smoke suite after every kept solver change, and rerun them after reverting failed experiments if the revert touched solver logic.
11. **Record**: Document every *successful* improvement and its PAR-2 or target-instance runtime impact in the solver's `README.md`, including machine environment metadata, benchmark log paths, profile paths, and the profiler evidence that motivated it. Also document important rejected attempts with their measured runtime or reason for skipping so future loops do not repeat them blindly.

### Standard Optimizations (apply to every solver)

**Cargo.toml release profile** (always include):
```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
overflow-checks = false
```

**build.sh** (always include):
```bash
[[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

## Running Full/Medium SAT Competition 2025 Benchmarks

When running benchmarks against `benchmarks/sat-comp-2025/` or `benchmarks/sat-comp-2025-medium/`, **always use a cron job** so the run survives if the Claude session ends. Use `tools/run_bench_reference.sh` as the wrapper (it logs output and manages sentinel files).

Important distinction:

- `tools/run_bench_reference.sh` / `tools/bench_reference.sh` are only for the reference solvers
- For an in-repo solver such as `solver/03-bcp`, use the **same one-shot cron pattern** but invoke `tools/bench.sh` directly and create a custom running/done sentinel if needed
- After the job starts, immediately remove the one-shot cron line so it cannot re-fire

### One-shot cron pattern (preserves existing crontab)

```bash
# 1. Pick a time ~2 min from now
date '+%M %H %d %m'   # e.g. "17 21 11 04"

# 2. Append one-shot entry (MUST preserve existing crontab)
EXISTING=$(crontab -l 2>/dev/null)
echo "$EXISTING
19 21 11 04 * /bin/bash /home/bojji/code/SAT-playground/tools/run_bench_reference.sh -t 1800 -m 16384 -d /home/bojji/code/SAT-playground/benchmarks/sat-comp-2025-medium" | crontab -

# 3. Verify it started (wait ~2 min, check sentinel)
cat log/bench_reference_RUNNING

# 4. IMMEDIATELY clean up the cron entry so it doesn't re-fire
crontab -l | grep -v 'run_bench_reference' | crontab -

# 5. Monitor progress
tail -f log/bench_reference_*.log
# Or check instance counts:
wc -l log/bench-kissat-latest-*/results.csv
```

### Key flags for run_bench_reference.sh

- `-t <seconds>` — per-instance timeout (default: 1800)
- `-m <MB>` — memory limit (default: 16384)
- `-d <path>` — benchmark directory (default: benchmarks/sat-comp-2025)
- Positional args: solver names (default: kissat-latest kissat-sc2024 minisat)

### Monitor progress

- `log/bench_reference_RUNNING` — exists while benchmark is active (contains PID and start time)
- `log/bench_reference_DONE` — created on completion (contains log file path)
- Results CSVs: `log/bench-<solver>-<timestamp>/results.csv`

### Important: kill ALL solver processes when stopping

```bash
pkill -f 'bench_reference'; pkill -f 'run_bench_reference'
pkill -f 'kissat.*\.cnf'; pkill -f 'minisat.*\.cnf'
# Verify: ps aux | grep -E 'kissat|minisat|bench_reference' | grep -v grep
```

Processes spawned by the script can outlive the parent if only the wrapper is killed. Always kill the solver binaries directly too.

## Status Reporting

When the user asks for status (e.g. "status?", "how's it going?", "what's running?"):

1. **CPU usage**: Run `ps aux --sort=-%cpu | head -20` and report SAT solver processes (sat-solver, minisat, kissat, etc.) with their CPU%, runtime, and instance name
2. **Running solvers**: Run `pgrep -a sat-solver; pgrep -a minisat; pgrep -a kissat` to identify active solver processes
3. **Benchmark progress**: Find the most recent active benchmark log:
   - Check `log/bench_reference_RUNNING` for reference solver runs
   - Find the latest `log/bench-*` or `log/bench_reference_*` directory/file
   - Report how many instances are solved vs total, and current instance being worked on
   - Always include a detailed completed-result breakdown for running benchmarks: SAT count, UNSAT count, timeout count, error count, and any other result statuses present in `results.csv`
   - Use `tail` on the log file or `wc -l` on `results.csv` to get progress counts

## Finding Current State

Do not treat this file as the source of truth for the repo's exact current solver lineup, benchmark
state, or iteration status. Instead:

- List the currently available solver iterations with `ls solver`
- Read `solver/NN-name/README.md` for each iteration's actual scope, validation status, and latest recorded benchmark notes
- Inspect `solver/NN-name/src/main.rs` and `Cargo.toml` for the real implementation and package identity
- Use `git status --short` and `git log --oneline -- solver/NN-name` to see local changes and recent history for a solver
- Check `benchmarks/reference-solvers/` for the current vendored reference solvers
- Check `tools/checkers/` and `tools/setup_checkers.sh` for the currently configured proof checkers
- Check `benchmarks/profiling/`, `benchmarks/crypto/`, `benchmarks/random-3sat/`, and the generator tools under `tools/` for the current benchmark inputs
- Use `tools/bench.sh`, `tools/bench_reference.sh`, and the latest `log/bench-*` directories to see the current benchmark workflow and outputs

## Common Pitfalls

- Forgetting the trailing `0` on `v` lines
- Printing `v` lines when the result is UNSAT
- Not handling empty clauses (immediately UNSAT)
- Not handling unit clauses at the top level
- Off-by-one on variable indexing (DIMACS is 1-based)
- Exceeding 4096 characters on a single `v` line
