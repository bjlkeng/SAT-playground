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
- **Benchmark UNKNOWN is a failure for solver work:** `UNKNOWN` means the solver produced neither
  `SATISFIABLE` nor `UNSATISFIABLE` within the run contract. Treat every new or retained
  `UNKNOWN` on a benchmark/profile row that the baseline solves as a failed experiment, even if
  aggregate PAR-2 happens to improve. Do not describe UNKNOWN churn as harmless.
- **SAT/UNSAT/UNKNOWN result errors are correctness failures:** if a solver run reports the wrong
  status, returns a SAT model that does not satisfy the original CNF, fails to write or validate an
  UNSAT proof, or returns `UNKNOWN` for a row that should produce SAT/UNSAT, stop and debug the
  issue to root cause. Capture the exact repro command/log, analyze the failing code path, and fix
  the correctness issue before continuing promotion, tuning, or downstream work. Do not paper over
  a result error by changing it to `UNKNOWN`, suppressing validation, quarantining flags, or
  rerouting the requested path unless the user explicitly asks for that mitigation.
- **Debug every UNKNOWN before continuing:** when an experiment produces `UNKNOWN`, stop promotion
  work for that path, debug the cause, and fix the actual issue or revert the experiment. Do not
  hide an UNKNOWN-producing path by silently normalizing, quarantining, or rerouting requested
  feature flags unless the user explicitly asks for that mitigation. Rerun the exact affected
  configuration and do not leave the task as complete until that configuration no longer produces
  `UNKNOWN` on the relevant baseline-solved rows.
- **Solver 11 default/fast promotion gate:** before promoting any solver 11 default or fast
  profile change, run a clean solver 10 comparison on the same benchmark set and pass
  `python3 tools/check_solver11_promotion.py --solver10 <solver10-results.csv> --previous <prior-solver11-results.csv> --candidate <candidate-results.csv> --timeout <seconds> --memory-mb <MB>`.
  The gate must record process sanity, matching instance/timeout sets, machine metadata, and the
  explicit decision when a candidate improves prior solver 11 but still loses to solver 10.
- **Do not promote one-instance overfit guards:** if a solver 11 default/profile improvement is
  dominated by one benchmark row, one formula-family classifier, or DIMACS input-order behavior,
  treat it as a diagnostic until a mechanism-level fix explains the win. Do not hard-code guards
  for a single instance shape as a default promotion without shuffled-order validation, a broader
  benchmark sample, and a written revert/rollback plan. Prefer fixing the underlying mechanism
  (for example watch selection, preprocessing budget, or search policy) over preserving a lucky
  trajectory from clause or literal order. Use the shuffle-sensitivity workflow
  (`python3 tools/shuffle_sensitivity.py --instances <cnf...> --seeds <seed-list>`) to record
  per-seed status, runtime, conflicts, decisions, and propagations before treating
  input-order-sensitive wins as promotion evidence.
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
2. **Baseline**: Run `bash tools/bench.sh -d benchmarks/profiling solver/NN-name` and record PAR-2. The profiling-suite default is 300 seconds per instance. For a one-instance target, also run `bash tools/bench.sh -t <seconds> -m 16384 -d <one-instance-dir> solver/NN-name` and record the target-instance runtime. If comparing to MiniSat-style simplification, also capture useful reference variants such as `minisat -no-pre`, `minisat -no-elim`, and full `minisat`.
3. **Profile first**: Use a profiler such as `perf stat` / `perf record` / `perf report` on the target instance before changing code. Use the profile to choose the next implementation slice. If the release binary is stripped and symbols are needed, rebuild for profiling with `CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=1 RUSTFLAGS="-C target-cpu=native" cargo build --release`.
4. **Check opportunity size before coding**: For simplification ideas, do a quick measurement pass over the target formula before implementing. Examples: count duplicate clauses, pure literals, candidate variables, binary subsumption hits, or self-subsuming-resolution opportunities. Do not implement an idea when the measured opportunity is negligible.
5. **Report diagnostic solver stats for every instance analysis**: Capture enough run data to identify whether the bottleneck is simplification, propagation, conflict learning, or search-path sensitivity. At minimum report pre/post-preprocessing variables, clauses, and literal counts; preprocessing time; eliminated variables, resolvents, subsumed clauses, strengthened literals, and root assignments when available; final result and runtime; conflicts, decisions, propagations, restarts, learned-clause count, reduce-DB calls, and any timeout/error status. When profiling, include propagation time or propagation throughput, conflict-analysis time if measured, simplification/proof I/O time if visible, and the profile/log paths used. When testing sensitivity, report the exact mode flags, seed/order/literal-order choices, preprocessing toggles, and per-instance deltas so the cause is not guessed from PAR-2 alone.
6. **Iterate** (at least 10 attempts unless the user asked for a narrower experiment): Make one change at a time, benchmark it against the current accepted baseline, and keep it only if it improves the target metric by more than `3%` without introducing or retaining any `UNKNOWN`/timeout/error row that the accepted baseline solves. Recalculate the accepted baseline and keep threshold after every kept change. Revert changes that improve PAR-2 by `3%` or less, worsen aggregate PAR-2, fail correctness checks, introduce a new unsolved/UNKNOWN row, or only shift time into a slow new implementation.
7. **Stop losers early when safe**: For long target-instance runs, if an experiment has already passed the `>3%` keep cutoff without finishing, stop the run, mark the attempt rejected, and revert it instead of waiting for the full timeout.
8. **Tune algorithmic features carefully**: When testing a focused solver idea such as bounded variable elimination, run small parameter sweeps around the first good result. More simplification can damage CDCL search even when preprocessing is cheap, so validate caps and cost thresholds empirically rather than assuming monotonic improvement.
9. **Retest interactions**: A previously rejected micro-optimization can become worthwhile after a later kept change changes the profile. Retest it only when profiler evidence or timing data suggests the interaction could now clear the `>3%` threshold.
10. **Optimize the new feature if needed**: If a feature is conceptually promising but too slow, profile the modified solver and iterate on that feature's implementation. Keep it only after the optimized version clears the `>3%` improvement threshold.
11. **Correctness checks**: Add or update focused tests when practical before changing solver behavior. Run `cargo test` and the full smoke suite after every kept solver change, and rerun them after reverting failed experiments if the revert touched solver logic.
12. **Record**: Document every *successful* improvement and its PAR-2 or target-instance runtime impact in the solver's `README.md`, including machine environment metadata, benchmark log paths, profile paths, and the profiler evidence that motivated it. Also document important rejected attempts with their measured runtime or reason for skipping so future loops do not repeat them blindly.

### Scientific bottleneck workflow

See the `/analyzesat` skill for the full scientific bottleneck workflow — multi-config ablation,
work × speed decomposition, reference-source diff, trajectory-trace chaos analysis, and the
debugging-regressions / reference-gap-closing procedures that used to live here.

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
- Read `benchmarks/profiling/README.md` for the current 10-instance profile suite (selected from SAT Competition 2025 medium-track, each <300 s on solver 10); the previous 6-instance set is preserved at `benchmarks/profiling/legacy/`
- Check `benchmarks/profiling/`, `benchmarks/crypto/`, `benchmarks/random-3sat/`, and the generator tools under `tools/` for the current benchmark inputs
- Use `tools/bench.sh`, `tools/bench_reference.sh`, and the latest `log/bench-*` directories to see the current benchmark workflow and outputs

## Common Pitfalls

- Forgetting the trailing `0` on `v` lines
- Printing `v` lines when the result is UNSAT
- Not handling empty clauses (immediately UNSAT)
- Not handling unit clauses at the top level
- Off-by-one on variable indexing (DIMACS is 1-based)
- Exceeding 4096 characters on a single `v` line

<!-- BEGIN BEADS CODEX SETUP: generated by bd setup codex -->
## Beads Issue Tracker

Use Beads (`bd`) for durable task tracking in repositories that include it. Use the `beads` skill at `.agents/skills/beads/SKILL.md` (project install) or `~/.agents/skills/beads/SKILL.md` (global install) for Beads workflow guidance, then use the `bd` CLI for issue operations.

### Quick Reference

```bash
bd ready                # Find available work
bd show <id>            # View issue details
bd update <id> --claim  # Claim work
bd close <id>           # Complete work
bd prime                # Refresh Beads context
```

### Beads Viewer (`bv`)

`bv` is installed at `~/.local/bin/bv` as a terminal sidecar for browsing and analyzing Beads. It reads `.beads/beads.jsonl`, so refresh that export after changing beads:

```bash
bd export -o .beads/beads.jsonl
```

For agent/automation use, do **not** run bare `bv` because it opens the interactive TUI. Use robot commands instead:

```bash
bv --robot-help       # list robot commands and contracts
bv --robot-triage     # full triage JSON with top picks, blockers, health
bv --robot-next       # single highest-priority recommendation
bv --robot-plan       # dependency-respecting execution tracks
bv --robot-insights   # graph metrics, bottlenecks, cycles, critical paths
```

Use `bd` as the source of truth for creating, updating, closing, and linking beads. Use `bv` for read-only triage, graph insight, and planning output. Do not hand-edit `.beads/beads.jsonl`; regenerate it with `bd export` when needed.

### Rules

- Use `bd` for all task tracking; do not create markdown TODO lists.
- Run `bd prime` when Beads context is missing or stale.
- Keep persistent project memory in Beads via `bd remember`; do not create ad hoc memory files.
<!-- END BEADS CODEX SETUP -->

## Agent Mail

### Installation

Clone the repo, set up and install with uv in a python 3.14 venv (install uv if you don't have it already), and then run `scripts/automatically_detect_all_installed_coding_agents_and_install_mcp_agent_mail_in_all.sh`. This will automatically set things up for your various installed coding agent tools and start the MCP server on port 8765. If you want to run the MCP server again in the future, simply run `scripts/run_server_with_token.sh`:

```bash
# Install uv (if you don't have it already)
curl -LsSf https://astral.sh/uv/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"

# Clone the repo
git clone https://github.com/Dicklesworthstone/mcp_agent_mail
cd mcp_agent_mail

# Create a Python 3.14 virtual environment and install dependencies
# Note: If you have an older uv version, run `uv self update` first
uv python install 3.14
uv venv -p 3.14
source .venv/bin/activate
uv sync

# Detect installed coding agents, integrate, and start the MCP server on port 8765
scripts/automatically_detect_all_installed_coding_agents_and_install_mcp_agent_mail_in_all.sh

# Later, to run the MCP server again with the same token
scripts/run_server_with_token.sh

# Change port after installation
uv run python -m mcp_agent_mail.cli config set-port 9000
```

Now, simply launch Codex-CLI or Claude Code or other agent tools in other consoles; they should have the mail tool available.

### MCP Agent Mail: coordination for multi-agent workflows

What it is
- A mail-like layer that lets coding agents coordinate asynchronously via MCP tools and resources.
- Provides identities, inbox/outbox, searchable threads, and advisory file reservations, with human-auditable artifacts in Git.

Why it's useful here
- The hot file for any active solver iteration is `solver/NN-name/src/main.rs` — multiple agents will want to edit it concurrently. Agent Mail lets them coordinate via Beads claims and thread messages instead of locking each other out with exclusive file reservations.
- Keeps cross-agent chatter out of the token budget by storing messages in a per-project archive.
- Fast reads via `resource://inbox/...` and `resource://thread/...`.

#### Coordination workflow (single repo, multiple agents on the same hot file)

This repo runs at most a few agents in parallel on the same solver iteration. The rules below are designed so two agents can both work on `solver/NN-name/src/main.rs` without blocking each other, while still avoiding lost work and racy commits.

1) Register and discover what others are doing
   - `ensure_project(human_key="/home/bojji/code/SAT-playground")`, then `register_agent` with a unique `agent_name`. Set `AGENT_NAME` in your shell so the pre-commit guard knows who you are.
   - Before picking a bead, list active claims (`bd ready`, `bd list --status in_progress`) and read the shared coordination thread (default `thread_id="coord"`) via `resource://thread/coord?...` to see what other agents have announced.
   - Pick a bead whose scope does **not** overlap the regions other agents have already announced. Prefer beads that touch different functions, modules, or solver iterations than the active claims.

2) Always work in a git worktree, never the main checkout
   - Create one worktree per agent under `/tmp/sat-worktrees/<agent>` on a fresh branch:
     ```bash
     git fetch origin
     git worktree add /tmp/sat-worktrees/<agent> -b agent/<agent>/<bead-id> origin/main
     cd /tmp/sat-worktrees/<agent>
     ```
   - All edits, builds, smoke tests, and benchmarks for the claimed bead happen inside that worktree.
   - When the bead is closed and merged to `main`, tear down the worktree: `git worktree remove /tmp/sat-worktrees/<agent>` and delete the branch.

3) Don't reserve the hot file — claim a bead and announce intent instead
   - Do **not** call `file_reservation_paths(..., exclusive=true)` on `solver/**/src/main.rs`. Exclusive reservations on the hot file defeat the point of parallel agents.
   - Instead: `bd update <id> --claim`, then `send_message(thread_id="coord", subject="claim <id>", body="bead <id>: <one-line scope> — touching <functions/regions> in solver/NN-name/src/main.rs, worktree /tmp/sat-worktrees/<agent>")`.
   - File reservations are still appropriate for **less-contended paths** (e.g. `docs/**`, `tools/**`, `tests/cnf/**`, `benchmarks/profiling/**`) when an edit will span many files there. Reserve those, not `src/main.rs`.

4) Pre-announce every commit with a short objection window
   - Before `git commit`, send `send_message(thread_id="coord", subject="commit <bead-id>", body="files: <list> · regions: <fns/lines> · summary: <one line> · validation: <what you ran> · objections within 2 min")`.
   - Wait ~2 minutes (`fetch_inbox` + `acknowledge_message` for any replies). If no objections, commit and push.
   - If another agent replies "I'm about to push overlapping hunks for bead Y", let them push first; you rebase and re-validate (step 5).
   - The 2-minute window is tunable — extend it for risky changes (cross-cutting refactors, new solver iteration scaffolding), keep it tight for narrow bead-scoped edits.

5) If someone beats you to a commit, rebase and re-run the bead's validation
   - `git fetch origin && git rebase origin/main` inside your worktree. Resolve conflicts (don't `--skip` or `--abort` away their work).
   - **Re-run whatever validation the bead requires**, in full, on the rebased state:
     - Correctness/fix beads → `bash tools/smoke_test.sh solver/NN-name` (+ `cargo test` if the bead touches tests).
     - Perf/optimization beads → smoke test **plus** `bash tools/bench.sh -d benchmarks/profiling solver/NN-name` so the measured delta is against the new baseline, not the pre-rebase one.
     - Promotion/default-change beads → the full gate described in CLAUDE.md's solver-11 promotion section, with the candidate re-measured on top of the new `main`.
   - If the rebase changes the baseline materially, **re-take the baseline** before reporting the experiment's delta — numbers from a stale baseline are meaningless.
   - Re-announce (step 4) with the updated commit-of-base SHA in the body, then push.

6) Closing out
   - After merge: `bd close <id>` with a summary, send a final `send_message(thread_id="coord", subject="closed <id>", body="merged at <sha>")`, and remove the worktree.

#### Granular vs macro tools
- Granular (use these in this workflow): `register_agent`, `send_message`, `fetch_inbox`, `acknowledge_message`, `fetch_topic`, `file_reservation_paths` (only for non-hot paths).
- Macros (handy when you don't need fine control): `macro_start_session`, `macro_prepare_thread`, `macro_contact_handshake`. Skip `macro_file_reservation_cycle` for the hot file — it implies exclusive reservation semantics we explicitly don't want there.

#### Common pitfalls
- **Treating `src/main.rs` as exclusive**: don't reserve it. Coordinate via Beads claims + the `coord` thread.
- **Editing in the main checkout while another agent is active**: always use `/tmp/sat-worktrees/<agent>`. Cross-agent edits to the same working tree will corrupt each other's state.
- **Skipping the pre-announce window**: silent commits race. A 2-minute window costs almost nothing and catches nearly all collisions.
- **Pushing after a rebase without re-running validation**: the bead's evidence belongs to the rebased commit, not the pre-rebase one. Re-run the full suite the bead requires, including baseline re-take for perf work.
- **Leaving worktrees behind**: stale `/tmp/sat-worktrees/<agent>` directories pile up and confuse future runs. Remove on bead close.
- **"from_agent not registered"**: always `register_agent` in the correct `project_key` first.
- **"FILE_RESERVATION_CONFLICT"** on non-hot paths: adjust patterns, wait for expiry, or use a non-exclusive reservation.
- **Auth errors**: if JWT+JWKS is enabled, include a bearer token with a `kid` that matches server JWKS; static bearer is used only when JWT is disabled.
