# Solver 13 — kissat-rs

Faithful Rust reimplementation of kissat 4.0.4 (reference:
`benchmarks/reference-solvers/kissat-latest/`, built `gcc -O3 -DNDEBUG`).

- Plan and acceptance criteria: `plan/solver13-port-plan.md`
- Binding port conventions: `CONVENTIONS.md`
- Faithfulness oracle: `tools/parity.py` — diffs the deterministic `-s`
  statistics counters against the reference binary at fixed `--conflicts`
  limits. A faithful port matches exactly.

Goal: on `benchmarks/sat-comp-2025` (400 instances, 3600 s / 16 GB / 32
cores), solved count and PAR-2 within 2% of kissat 4.0.4 in a fresh paired
run, with all kissat features implemented.

**Work clock (plan step A′, 2026-09-15).** The binary prints one extra
line at every exit, after the `[ resources ]` section:

```
c workclock ticks=… search_ticks=… probing_ticks=… backbone_ticks=… transitive_ticks=… factor_ticks=… substitute_ticks=… kitten_ticks=… eliminate_resolutions=… forward_steps=… walk_steps=… flipped=… conflicts=… decisions=… propagations=… k_res=11 work=…
```

- `work` is the work clock W = `ticks` + `k_res` × `eliminate_resolutions`
  (plan §3.4): the deterministic unit the harness prices cells in (tick
  PAR-2, CLAUDE.md "Evaluation") and the unit `SAT_LIMIT_TICKS` limits
  (plan step A.1, below). `ticks` is kissat's all-propagation counter, which kissat
  itself never prints. The other keys are the work kinds the RL reward
  weights (plan §4). `vivify_ticks` is not on the line: kissat counts it
  only in a METRICS build (since the step-A counter audit the port fills
  it as a never-printed field for the RL log); vivify's propagation is
  inside `probing_ticks` either way.
- Printed on every exit path kissat prints statistics on: the normal exit
  and the signal handler, so a run killed by the harness `timeout`
  (SIGTERM) still reports the work it consumed. Not printed under `-q`. It
  sits outside the `-s` block, so `tools/parity.py` never sees it, and it
  has no `name:` token, so the parity regex cannot match it.
- **Checked against the C (2026-09-15).** A kissat 4.0.4 built with
  `./configure --statistics` prints the STATISTIC-tier counters (`ticks`,
  `flipped`) in its `-s` block. On the 20 discriminating cells at
  `--conflicts=100000`, all 15 counters on our line equal that build's rows
  on 20/20 cells, and that build's 80 default counters equal ours, so it is
  the same trajectory. `tools/parity.py --conflicts 100000` against the
  reference binary: 20/20 with the line in place. The statistics build was
  a scratch copy of `benchmarks/reference-solvers/kissat-latest` (about a
  minute to build); the reference build is untouched.
- **`k_res` is provisional (= 11).** From the same 20-cell run with
  `--profile=2`: eliminate wall time per resolution, converted to search-tick
  units with each cell's own search ticks per second, over the 13 cells
  with at least 0.05 s in both phases: median 11.5, geomean 10.8, range 3.3
  (circuit) to 22 (SCPC). Ticks per second itself ranged 3.1e7 to 6.7e7
  across cells. The eliminate time includes forward subsumption and
  definition extraction, so `k_res` also charges those to the resolution
  count. Plan step B refits it from the stock traces. The constant is
  `K_RES` in `src/statistics.rs`; the harness reads it from the line and
  never hard-codes it. Changing `k_res` changes only `work`, so old results
  can be re-priced offline from `ticks` and `eliminate_resolutions`.
- Harness: `tools/feature_ablation.py` records `ticks`,
  `eliminate_resolutions` and `work` per cell and prints tick PAR-2 (a
  solved cell costs its W, an unsolved cell twice the W it reached before
  the kill; that unsolved part of a wall-limited run moves with host load
  and is printed separately). `tools/run_kissat_full.sh` records the same
  plus `search_ticks` and `probing_ticks` (both arms now run with `-s`; the
  C arm records NA, never 0, for `ticks` and `work`), and
  `tools/compare_full_runs.py` prints tick PAR-2 for solver-13 pairs. Since
  2026-09-15 `feature_ablation.py` calls this binary the kissat way
  (`--seed=N`, the arm's `SAT_EXTRA_ARGS` split into options, the CNF, and
  the proof file only when verifying); before that it passed the output
  directory as the proof path and every solver-13 cell exited 1.
- Checking by hand: `timeout 5 sat-solver x.cnf | grep workclock` shows
  nothing and the shell reports `Terminated` (the `timeout` signal takes
  the pipeline down with it); redirect to a file instead. The harness paths
  (a Python pipe, `$(...)`) are unaffected and were checked on TIMEOUT cells.
- **Acceptance A/A run (2026-09-15, k_res = 11 binary):**
  `feature_ablation.py --arm cand: --arm base: --suite benchmarks/discriminating
  --seeds 1 --jobs 28 --timeout 300` (`log/abtest-cand-vs-base-2026-09-15-15-17-20`):
  20/20 solved in both arms, and `ticks`, `eliminate_resolutions`, `work` and
  `conflicts` identical on all 20 cells, so tick PAR-2 is 4.398e10 in both
  arms (ratio 1.0000) while wall PAR-2 differs by 2% (1405.5 v 1380.5 s,
  timing noise). No NA cell. Two UNSAT proofs hit the checker's 2x-timeout
  budget (`verified=checker-timeout`, a checker budget, not a failure). A
  first pass with the earlier k_res = 20 binary
  (`log/abtest-cand-vs-base-2026-09-15-15-02-57`) gave the same 20/20 identity.

**`SAT_EXTRA_ARGS` (plan step 0, 2026-09-15).** `run.sh` expands
`SAT_EXTRA_ARGS` (word-split) as kissat options before the CNF in both of
its paths, e.g. `SAT_EXTRA_ARGS='--eliminateint=1000' bash run.sh x.cnf out/`.
Unset or empty adds no words, so the command is unchanged. Every interval and
effort knob is a kissat option, so this is the whole constant-knob experiment
of the RL plan (§7 step 5b). `tools/feature_ablation.py` forwards an arm's
`SAT_EXTRA_ARGS` to the binary itself, so `--arm 'x:SAT_EXTRA_ARGS=--probeint=50'`
works without the wrapper; `tools/parity.py` calls the binary directly and is
unaffected. Checked 2026-09-15 on SCPC-500-1 at `--conflicts=300000`:
eliminations 6 (stock) v 4 (`--eliminateint=1000`) v 8 (`--eliminateint=250`);
unset v empty give identical `s` and `c workclock` lines; smoke test 9/9.

**RL scheduler plumbing, step A (2026-09-16; plan §7, beads
`SAT-playground-p9m.6.*`).** The solver can now run under an external
scheduler that moves kissat's timing decisions while every mechanism stays
byte-identical. Done (2026-09-16/17): the work-clock limit (A.1), the
policy module with its epoch clock and action (A.2), the stage-1
chokepoints (A.3), random and jitter modes with a replay test (A.4), the
raw-state logger (A.5), the counter audit (A.6), the static features
(A.7), fork mode (A.8), `observe()` (A.9) and the net loader (A.10); the
overhead check (A.11) is recorded below. Everything is off by default: with no
`SAT_POLICY*` and no `SAT_LIMIT_TICKS` in the environment the only added
work on the search path is one bool test per chokepoint.

Recipes (every variable is in the table below):

```bash
B=solver/13-kissat-rs/target/release/sat-solver
# a stock trace with logging (the collector's stock arm)
SAT_POLICY_LOG=run.log $B x.cnf
# a random-mode logged run: sticky segments, seed 7, tick budget 2^31
SAT_POLICY=random SAT_POLICY_SEED=7 SAT_LIMIT_TICKS=2147483648 SAT_POLICY_LOG=run.log $B x.cnf
# a fork run: at decisions 3 and 9 branch the reduce and mode menus, 4 live children
SAT_POLICY_LOG=run.log SAT_POLICY_BRANCH=3:reduce,9:mode SAT_POLICY_BRANCH_JOBS=4 SAT_LIMIT_TICKS=2147483648 $B x.cnf
# a learned policy at margin 1, with the wall budget the harness knows
SAT_POLICY=net.bin SAT_POLICY_MARGIN=1 SAT_WALL_LIMIT=1800 $B x.cnf
# read a log, recompute its observation vectors, make a fixture net from its layout
python3 solver/13-kissat-rs/tools/policy_log.py run.log --tail 3
python3 solver/13-kissat-rs/tools/policy_obs.py --check run.log
python3 solver/13-kissat-rs/tools/rl/policy_net.py --fixture run.log net.bin --stock-bias 1000
```

Environment (read by the binary itself, so `run.sh`, the harness and
`parity.py --solver-env` all reach it):

| variable | values | meaning |
|---|---|---|
| `SAT_LIMIT_TICKS` | integer | stop with `s UNKNOWN` (exit 0) once the work clock W = `ticks` + `k_res` × `eliminate_resolutions` reaches this; unset or empty = no limit; `0` is a real (zero) limit |
| `SAT_POLICY` | `stock`, `random`, `jitter`, or a file path | policy on with the stock action every epoch (the overhead arm), segmented sticky random actions, per-decision jitter, or the learned policy in a weights file (A.10, below); unset or empty = off |
| `SAT_POLICY_MARGIN` | number ≥ 0 or `inf` | net mode only: a non-stock entry is taken only when its score beats the stock entry's by this much (log-odds; default 1; `inf` = always stock, which must pass parity) |
| `SAT_POLICY_HORIZON` | `ticks:<B>` | the horizon feature (fraction of budget used) as work / B; the deterministic form for parity, replay and fork children; when unset a `SAT_LIMIT_TICKS` run uses work / limit, else `SAT_WALL_LIMIT` |
| `SAT_WALL_LIMIT` | seconds > 0 | the wall budget the harness runs the cell under, for the horizon feature at inference (elapsed wall / limit; the one non-deterministic input); no effect on when the solver stops |
| `SAT_POLICY_BRANCH` | `D:knob[,D:knob...]` | fork mode (A.8): at decision D (0 = D0, each D at most once) fork one child per alternative entry of the knob's menu (probe, eliminate, reduce, rephase, reorder, mode, margin, sweep); needs `SAT_POLICY_LOG` and `SAT_LIMIT_TICKS` (children stop on the inherited budget); refused with a proof or `-o` file |
| `SAT_POLICY_BRANCH_ACTIONS` | `knob=e\|e[;knob=e\|e]` | restrict the entries forked per knob (menu values such as `0\|0.5\|2\|4`); the parent's own entry is allowed here (a stock child is the fork test) |
| `SAT_POLICY_BRANCH_JOBS` | 1..64 | live children per parent (default 4); the parent blocks at a branch point until a child exits (wall, not ticks) |
| `SAT_POLICY_EPOCH_TICKS` | `X_o[,X_d]` | observation and decision epochs in `search_ticks`; default `8388608,134217728` (2^23, 2^27); `X_d` defaults to 16 × `X_o` and must be an integer multiple of it (decisions are checked at observation boundaries) |
| `SAT_POLICY_SEED` | integer | the policy's own generator seed (default 0); never touches the solver's `--seed` stream |
| `SAT_POLICY_TEMP` | number > 0 | spread of the random menus around stock: weight exp(−distance/temp), so 0.2 is almost always stock and 100 is uniform (default 1.0) |
| `SAT_POLICY_SEGMENT` | number ≥ 1 | mean sticky segment length in decision epochs (default 5) |
| `SAT_POLICY_LOG` | path | raw-state log, one row per observation epoch plus a terminal row (A.5, below); with `SAT_POLICY` unset it turns the policy on in stock mode, i.e. a stock trace with logging; an unwritable path, or one that aliases the CNF, the proof, a `-o` output, a standard stream or a wrapper-reserved file, is a usage error |
| `SAT_POLICY_LOG_RESERVED` | paths, one per line | extra files the log may not alias; set by `run.sh` for its capture and result files and its redirected stdout/stderr |

- **Work-clock limit.** `limited.ticks` / `limits.ticks` sit next to
  kissat's conflict and decision limits. The check lives in
  `terminate::terminated`, which is what every inprocessing effort loop and
  the search loop already poll for external termination; on a hit it raises
  the same flag `--time` (SIGALRM) does, so the current pass winds down
  exactly as it does on a signal and the search loop stops. A run also
  refuses to start searching when preprocessing alone spent the budget,
  and the lucky passes (which kissat's own conflict and decision limits
  never bound) poll the budget before every assumption and unwind to level
  0 on a hit, so a cell lucky would solve outright is still `s UNKNOWN`
  under a budget smaller than lucky's work. Tests (`tests/limit_ticks.rs`):
  two runs at the same limit stop at identical `-s` counters and
  `c workclock` line; on the random 3-SAT fixture (several probe and two
  eliminate rounds) limits at 10/25/50/75 % of the full run overrun by
  under 10 %; on a lucky-solved cell of 100 k clause pairs limits of 0, 100
  and 50 000 all stop with `s UNKNOWN` within a tenth of the full work; a
  non-binding limit and an empty value are trajectory-identical to unset;
  `0` gives `s UNKNOWN` before search; a non-integer is a usage error
  (exit 1).
- **Policy core (`src/policy.rs`).** Per timer (probe, eliminate, reduce,
  rephase, reorder, mode) the module keeps `last_fire` and `stock_delta`,
  recorded at every `INIT_CONFLICT_LIMIT` / `UPDATE_CONFLICT_LIMIT` and at
  both mode-limit updates (the mode timer is on conflicts in focused mode
  and on search ticks in stable mode, as kissat keeps it). Each fire
  predicate compares against `last_fire + max(0.1 × stock_delta, m ×
  stock_delta)` instead of `limits[T]`; `m = 1` reproduces `limits[T]`
  exactly, `m = 0` is one-shot (fires at `last_fire + 0.1 × stock_delta`,
  then reverts to 1 at that fire). The restart margin multiplies
  `restartmargin` in focused mode; the sweep effort multiplies the delta of
  sweep's `SET_EFFORT_LIMIT` and `0` skips the round after kissat's own
  delay test has run. The hook sits at the head of the search-loop
  if-chain: every `X_o` a row (logger pending), every `X_d` a decision, and
  one decision D0 before the first `decide()`. Decisions are masked to
  stock where a knob cannot act (rephase in focused mode, the margin in
  stable mode, reorder per `reorder`, mode switching only with
  `stable=1`, disabled passes). Unit tests in `policy.rs` check the
  `last_fire + stock_delta == limits[T]` invariant after `init_limits` and
  after every timer's real update macro, the floor, the one-shot, masking,
  the epoch arithmetic and the sampling distribution.
- **Random modes.** `random`: geometric segments (mean `SAT_POLICY_SEGMENT`
  decision epochs); at each segment start the action is stock with
  probability ½, else every knob is drawn from a categorical centred on
  its stock entry at the run's temperature, with the one-shot `m = 0`
  entry capped at 5 %. `jitter`: a fresh draw every decision. Both use the
  policy's own LCG (same algorithm as `random.rs`, separate state).
  `tests/policy.rs`: policy-on-STOCK is trajectory-identical to policy-off
  on php9 under `--conflicts` and on the random 3-SAT cell at default and at
  tiny epochs (hundreds of rows, dozens of decisions); a seeded random or
  jitter run replays to identical counters, a different seed differs, and
  wild random runs (temperature 100) still answer SAT/UNSAT correctly;
  every bad setting is a usage error.
- **Parity (`tools/parity.py --conflicts 100000`, 20 discriminating cells
  v the reference kissat).** 20/20 policy-off and 20/20 policy-on-STOCK (`--solver-env SAT_POLICY=stock`) on the committed binary (sha256 `d91f33850ab1e0a1`, 2026-09-16, 4 pinned cores per run); the same pair also passed on the pre-audit binary and after each Codex review fix (four pairs in all, every one 20/20).
- **Counter audit (A.6).** `Statistics` is now declared from one field list
  (`statistics_fields!`) with `Statistics::NAMES` / `values()` for the
  logger. 64 kissat METRIC counters are re-enabled as never-printed fields
  at exactly their C `INC`/`ADD` sites: `ands_extracted`, `arena_enlarged`,
  `arena_garbage` (bytes, unsigned wrap as in C), `arena_resized`,
  `arena_shrunken`, `backbone_implied`, `backbone_probes`,
  `backbone_propagations`, `backbone_rounds`, `best_saved`, `compacted`,
  `definitions_checked`, `definitions_extracted`, `defragmentations`,
  `dense_garbage_collections`, `dense_propagations`, `dense_ticks`,
  `duplicated`, `equivalences_extracted`, `flushed`, `focused_decisions`,
  `focused_modes`, `focused_propagations`, `focused_restarts`,
  `focused_ticks`, `forward_subsumptions`, `garbage_collections`,
  `gates_checked`, `gates_extracted`, `if_then_else_extracted`,
  `initial_decisions`, `literals_bumped`, `literals_deduced`,
  `literals_learned`, `literals_minimized`, `literals_minshrunken`,
  `literals_shrunken`, `moved`, `probing_propagations`, `rephased_best`,
  `rephased_inverted`, `rephased_original`, `rephased_walking`, `rescaled`,
  `saved_decisions`, `score_decisions`, `search_propagations`, `sparse_gcs`,
  `stable_decisions`, `stable_modes`, `stable_propagations`,
  `stable_restarts`, `stable_ticks`, `target_decisions`, `target_saved`,
  `transitive_probes`, `transitive_propagations`, `transitive_reduced`,
  `transitive_reductions`, `transitive_units`, `vectors_defrags_needed`,
  `vectors_enlarged`, `walk_decisions`, `weakened`. The METRICS-only flags
  `backbone_computing` and `vivifying` are back on `Solver` so
  `proprobe.rs` attributes probing propagations to backbone and, for the
  first time in the port, fills the STATISTIC fields `vivify_ticks` and
  `vivify_propagations`; that block does **not** add to `backbone_ticks`,
  which is a printed counter whose reference value comes from
  `backbone.rs` alone. Not re-enabled: `allocated_*` (malloc accounting
  the port has no equivalent of), `extensions` and `walk_previous` (no
  `INC` site in kissat 4.0.4). Every `GET (metric)` message site keeps
  printing `u64::MAX` (no count) as the reference build does, which
  `parity.py --phases` relies on. Nothing reads a re-enabled counter, so
  they cannot move the trajectory; parity on the audited binary:
  20/20 both ways (the runs above are on the audited, committed binary). Wall cost of the extra increments: paired timing of the pre-audit v audited binary on the 20 discriminating cells at `--conflicts=100000` (identical `c workclock` line per cell, 3 alternating reps each, medians, the two binaries pinned to two idle physical cores while the parity runs and the review sat on other cores): geomean 1.0045, per cell 0.984-1.021 with both signs (`log/pair_time-stepA-audit-2026-09-16.log`), inside the ±2 % run-to-run wall noise the 2026-09-15 A/A run showed. The formal overhead measurement of logging and observe() is bead A.11.
- **Logger (A.5, `src/policy_log.rs`).** `SAT_POLICY_LOG=path` writes
  one row per observation epoch (D0 first) and one terminal row at exit,
  then a sentinel row and a footer. One file, self-describing: line 1 is
  `SAT13POLICYLOG 1`, line 2 a JSON header (format, solver, `k_res`, the
  CNF path, pid, the policy settings, all 158 options, the column names,
  a `kinds` string with one `u`/`f` per column, `row_bytes`), written
  before parsing so that a kill at any later point still yields a
  complete file; then fixed-width rows of little-endian u64 (an `f`
  column is an f64 stored as its bit pattern), a row whose first word is
  2^64−1, and a JSON footer (result, exit code, reason `solve`, `signal`
  or `output-error` for a run whose `-o` file could not be written,
  rows, wall and CPU nanoseconds, peak RSS from `getrusage`,
  work, conflicts, the `static` object of A.7, the `branches` list and
  `branch` block of A.8). 981 columns, 7848 bytes per row (691 before the
  A.7-A.10 columns `row_boundary`, `horizon*`, `obs_*`, `net_*`): every
  `Statistics` counter by name plus the two 128-bin clause-use glue
  histograms (`used_f_glue*`, `used_s_glue*`), both averages blocks, every limit and the
  three limit flags, the four delay counters, the elimination bound, the
  tier glue limits, the search state (mode, level, trail, propagate
  cursor, unassigned, active, arena and watch-stack sizes, best/target
  assigned, reluctant state, `last.*`), the per-epoch learned-clause
  histogram (8 glue bins, count, summed size and glue; fed from
  `update_learned` under `policy.on` and reset after each row), per
  timer `last_fire`, `stock_delta`, fires, the effective limit and
  stock's would-fire flag, the action in force (`act_*`) and the action
  as decided at the last decision (`dec_*`, the training label: a
  consumed one-shot reads 1 in `act_*` but keeps its 0 in `dec_*`; a row
  is written at a boundary before that boundary's decision, so it
  describes the epoch that just ended and the decision that governed
  it), the policy RNG state and the segment counter, plus `work`, the monotonic wall clock and the
  process CPU clock per row. The footer is also written from the signal
  handler, so a `timeout` kill leaves a complete file; SIGINT, SIGTERM and
  SIGALRM are blocked while a row or the footer is being written, so a
  signal can never land between the logger being taken out of the solver
  and put back (checked six times per test run with a SIGTERM into a
  fine-grained log). Because the handler may interrupt the solver inside
  `malloc`, the row and footer path allocates nothing once the header
  exists: column names are formatted only for the header, the counters
  are visited in place, the row buffer keeps its capacity and the footer
  is formatted into a preallocated string, and a write error is kept as
  its OS error code (no text); a kill during parsing or preprocessing
  therefore still ends with a terminal row (the work and peak RSS so far)
  and a footer. On the normal path the footer is sealed only after every
  required output is out (proof closed, `s` line, model, `-o` file), so a
  kill during output still ends the file as `signal`/`UNKNOWN`, never as
  a successful record of an incomplete run. A log path that names the
  CNF, the proof or the
  `-o` output (by string, by resolved path with symlinks followed by
  hand so a dangling link to a future proof file still counts, or by
  inode), or that is the file behind the process's stdin, stdout or
  stderr (a CNF read through `< input.cnf` has no positional path), is
  refused with exit 1, so the log can never truncate the input, share the
  proof or clobber redirected output. `run.sh` hands the binary its own
  files (`solver_stdout.tmp`, `proof.out`, `model.txt`, `status.txt`,
  `result.json`) and the files behind the wrapper's redirected stdout or
  stderr (a caller's `> out.txt`) in `SAT_POLICY_LOG_RESERVED`, one path
  per line, and the binary refuses those with the same normalization and
  alias rules; it cannot see them by itself because the wrapper's stdout
  is a pipe, and a wrapper-side copy of the rules is how review rounds
  found trimming and hard-link gaps. In the signal handler the log is
  sealed before any diagnostic
  print, since the interrupted code may hold the stdout lock, and the
  logger itself never prints from that path. A write
  or flush failure (a full disk, `/dev/full`) stops the log, prints one
  `c warning: policy log ... incomplete` line and withholds the footer,
  which is how a reader learns the trace is unusable; the answer and the
  exit code are unaffected. Size: at the
  default 2^23-tick observation epoch a cell runs about 4 rows per
  second, so an 1800 s trace is about 40 MB (the plan's 35 MB estimate
  was generous). `tools/policy_log.py` reads it (`read(path)` returns
  header, rows, footer; the CLI prints a tail). `tests/policy_log.rs`:
  logging on v off is trajectory-identical; the terminal row equals the
  run's `-s` counters and its `c workclock` work; the per-epoch learned
  counts sum to `clauses_learned`; the D0 row carries the tick limit
  when one is set; a log alone runs the stock policy; an unwritable path
  exits 1. Parity with logging on (`--solver-env SAT_POLICY=stock --solver-env SAT_POLICY_LOG=...`, 20 discriminating cells, `--conflicts 100000`): 20/20 on the final search-path binary of this change set (sha256 `77dd6360a5150d46`), and 20/20 on each of the five earlier binaries of the review rounds; the row writer touches no solver state, so logging on is trajectory-identical to logging off (also checked per run by `tests/policy_log.rs`).
- **Static features (A.7, `src/policy_static.rs`).** One pass over the
  active variables and the non-garbage irredundant clauses, run once after
  preprocessing and `classify()` when the policy is on, plus a snapshot of
  the yields kissat's own passes already produced. 140 values in 15 named
  groups: identity groups `shape` (sizes, clause-length histogram, the
  share of literals in the longest 1 % of clauses), `occurrence`
  (degree moments and entropy, near-singletons, pure literals, polarity
  balance, Horn and reverse-Horn fractions), `big` (binary fraction and
  the implication graph's degrees, roots, leaves), `locality` (clause
  span over the variable index space: mean, median, small-span and
  consecutive fractions; the one group that variable renaming destroys),
  `classify`; response groups `scc` (the non-trivial SCCs substitute's
  Tarjan walk found, from a pure hook in substitute.rs), `bfs` (a bounded
  breadth-first implication walk from the 32 highest-degree literals plus
  random ones until the mean reach's standard error is under 10 %, at
  most 256, capped at 1 M edge visits: depth, reach, failed-literal
  fraction), `gates` (the congruence census), `backbone`, `sweep`,
  `kitten`, `fastel` (with budget-hit flags from hooks in backbone.rs and
  sweep.rs), `lucky` (outcome, level fraction and conflicts per pattern,
  from hooks in lucky.rs), `warmup`, `preprocess` (vars and clauses over
  the original counts, units, ticks by pass). The header carries the
  schema (`static_schema`: group, kind, feature names) so a tool can mask
  a group; the values are in the footer (`static`), which the signal
  handler also writes. Cost is wall only (no tick counter is charged; the
  work clock is unchanged): Kakuro-easy-112 0.50 s against a 4.13 s
  parse, brocard 0.14 s against 1.37 s, SCPC-500-1 0.5 ms (2026-09-16).
  The BFS sample draws from a generator seeded from `SAT_POLICY_SEED`,
  never from `solver.random`.
- **`observe()` (A.9, `src/policy_obs.rs`).** The policy's input: 251
  floats in five blocks, computed at every observation boundary before
  the row and the decision and logged in the row as `obs_*` columns
  (also on the terminal row, from the state then). Static block (63: a
  subset of the static features, counts as log1p, fractions raw, plus a
  validity flag); global block (45: log counts, ratios, the averages of
  the current mode, the per-epoch learned-clause histogram, the
  elimination bound, and the horizon with its validity flag); delta block
  (3 × 17: the same dynamics over the last 1, 4 and 16 observation
  epochs from a ring of boundary snapshots, zero with a validity flag
  until enough exist); timer block (6 × 4: log stock delta, progress to
  the stock deadline, fires, would-fire); pass block (12 × 5: for
  congruence, substitute, backbone, vivify, sweep, transitive, factor,
  eliminate, forward, reduce, rephase, walk: never-ran flag, epochs since
  the last run, times run, yield and cost at the last run, from a
  recency table updated at each boundary from the counter deltas).
  Normalization is not applied here; it is in the weights file. Pure: no
  generator, no solver container, nothing allocated after the first
  call. Every input is in the row, the header (horizon mode and budget)
  or the footer (static block), so `tools/policy_obs.py --check run.log`
  recomputes every row's vector in float64 and compares it to the logged
  float32 values: 0 mismatches on tick-limit, wall and explicit horizons
  and in random mode (`tests/policy_obs.rs`; a fork child's log needs its
  parent's rows replayed first, which the converter will do). The
  horizon is `SAT_POLICY_HORIZON=ticks:B` (work / B), else work / the
  `SAT_LIMIT_TICKS` limit, else elapsed wall / `SAT_WALL_LIMIT`, else 0
  with the flag 0; the wall reading is taken once per boundary and shared
  with the row's `wall_ns`, which is what makes the wall form recomputable.
- **Net (A.10, `src/policy_net.rs`, `tools/rl/policy_net.py`).**
  `SAT_POLICY=<file>` loads a flat little-endian file: magic
  `SAT13POLICYNET`, format 1, the input count and an FNV-1a hash of the
  observation names (a file trained on another layout is refused with
  both hashes in the message), hidden sizes, eight heads (probe,
  eliminate, reduce, rephase, reorder: 5-way; mode and restart margin:
  3-way; sweep: 4-way; each with a kind, size and stock index checked
  against the menus), then f32 arrays: mean, std, W1, b1, W2, b2 and per
  head W, b. Head kinds: 0 = on the trunk, 1 = linear on the normalized
  input, 2 = reserved for tree rankers (refused for now, plan step E.3).
  The forward pass is hand-rolled, f64 accumulation in index order, no
  FMA, so `tools/rl/policy_net.py --forward` (pure Python, float64) gives
  the same bits: 16 cases over both head kinds match exactly in the unit
  test. The same script also evaluates the net in PyTorch (float64 on the
  same float32 weights, so only the summation order differs) when torch
  is importable, and the test then requires agreement within 1e-6
  relative: with PyTorch 2.14.0+cpu in a scratch venv (`uv venv` +
  `uv pip install --index-url https://download.pytorch.org/whl/cpu
  torch`, put first on `PATH` for `cargo test`) the largest relative
  difference over the 16 cases was 1.6e-14 (2026-09-17). Cost measured
  once: 50 µs per forward pass at 251 → 128 → 64 (test profile), which
  at one decision per 2^27 search ticks (seconds) is nothing. Acting: per head the best entry is taken only when its score
  beats stock by `SAT_POLICY_MARGIN`, then the action is masked as
  usual; the row logs the 35 scores (`net_*`) and `net_deviations`, the
  header the file, sizes, hash and margin, and the exit prints one
  `c policy net: N decisions, M not stock` line. The weights file is one
  more file the log may not alias (it is reserved before the log is
  created, so `SAT_POLICY=net.bin SAT_POLICY_LOG=net.bin` exits 1 with
  the model intact), and the per-epoch learned-clause features are reset
  at every boundary by the epoch hook, not by the row writer, so a net
  decides the same with or without a log (both from Codex review round
  1, 2026-09-17). `tests/policy_net.rs`: a
  stock-biased fixture at margins 1, 0.25 and `inf` is trajectory-identical
  to policy-off; a fixture that prefers other entries at margin 0 moves
  the trajectory, replays to identical counters and still answers
  correctly; a name that is not a file, a non-weights file, a truncated
  file and bad margins exit 1.
- **Fork mode (A.8, `src/policy_fork.rs`).** `SAT_POLICY_BRANCH=D:knob,...`
  forks, when the parent takes decision D, one child per alternative entry
  of that knob's menu (or the entries listed in
  `SAT_POLICY_BRANCH_ACTIONS`). A child holds its entry for that decision
  epoch, then continues under the parent's policy with the inherited RNG
  state, stops on the inherited `SAT_LIMIT_TICKS`, and writes
  `<log>.b<D>.<k>` (its log; the header and footer carry a `branch` block:
  parent pid and log, decision, boundary epoch, knob, entry, index, parent
  rows, whether masking turned the entry back into the parent's action),
  `<log>.b<D>.<k>.out` and `.err` (its streams, redirected with dup2
  before anything else), so the harness sees one `s` line. Its first row
  is the branch state under its own decision (`row_boundary` 0, the
  parent already pushed that snapshot); the parent's log footer lists the
  children (`branches`: decision, knob, entry, index, pid). At most
  `SAT_POLICY_BRANCH_JOBS` children live per parent; the parent blocks at
  a branch point until one exits, reaps all at exit and prints
  `c policy branch: P points, F children forked, R reaped, A abnormal, U
  points not reached`; a parent killed by a signal SIGTERMs its live
  children from the handler. Hygiene: stdout, stderr and the log are
  flushed before every fork; the child forgets the parent's log handle
  (shared file offset), opens its own with the same alias rules, and a
  child that cannot redirect or open its log `_exit`s 1; a proof file or
  a `-o` output refuses fork mode at start (every child would write it).
  Every file a child of the schedule could create (the whole menu per
  point, log, `.out`, `.err`) is checked at start against the CNF, the
  proof, the output, the weights file, the wrapper's reserved files, the
  parent's log, the standard streams and each other (string, resolved
  path with symlinks followed, inode), and again right before each fork
  (an alias then skips that child with a warning); a decision listed
  twice in the schedule is refused, since the file names carry only the
  decision and the child index. SIGINT, SIGTERM and SIGALRM are blocked
  from before `fork()` until the parent has registered the child and the
  child has detached from the parent's files and forgotten its siblings
  (the mask is inherited), so a signal in that window cannot run the
  child's inherited handler against the parent's log or leave the parent
  an untracked child; the parent's pid is captured before the fork
  (Codex review round 1, 2026-09-17). Round 2 added: the alias check
  runs over the whole schedule at once (a later point's file linked to an
  earlier point's is refused), the footer's branch record is completed
  while signals are still blocked (a kill between two forks could leave
  `"branches":[...,]`), fork mode requires `SAT_LIMIT_TICKS` (children
  inherit neither kissat's `--time` alarm nor any wall clock), and a
  parent whose `--time` alarm fires or that is told to terminate SIGTERMs
  its live children before waiting for them (`tests/policy_fork.rs`: a
  `--time=1` parent with four live children exits within seconds with
  every child gone and every log sealed). Round 3: the whole epoch
  boundary (recency update, observation, row, snapshot, accumulator
  reset) runs with the handled signals blocked, so a kill between the row
  and the snapshot cannot leave a terminal row built from the old history
  with the epoch's learned clauses counted twice. Round 4: the signal
  guard is taken before the pre-fork flush of the parent's log (and the
  logger's flush blocks on its own), so a kill during that flush cannot
  re-enter the writer and duplicate bytes.
  `tests/policy_fork.rs`: a stock child (`SAT_POLICY_BRANCH_ACTIONS=probe=1`)
  has the parent's `-s` counters at the same tick limit, the parent's
  stdout has one `s` line and its log no duplicated rows; the default
  menus give 4 + 2 children for `1:reduce,3:mode`, all answering SAT like
  the parent with most leaving its trajectory; a proof file, a missing
  log and bad settings exit 1; a SIGTERM to a parent with 8 live children
  leaves no child alive within seconds, every log sealed.
- **Parity after A.7-A.10 (2026-09-17, committed binary sha256
  `305c22188e38d5dc`, 20 discriminating cells, `--conflicts 100000`, one
  pinned core per configuration, the three run side by side):** 20/20
  policy off; 20/20 policy-on-STOCK with logging (`--solver-env
  SAT_POLICY=stock --solver-env SAT_POLICY_LOG=...`, so the static pass,
  `observe()` at every boundary and the 981-column rows all ran); 20/20
  with the stock-biased fixture net at an infinite margin
  (`--solver-env SAT_POLICY=net.bin --solver-env SAT_POLICY_MARGIN=inf`
  plus logging, so the forward pass ran at every decision). The same
  three runs passed 20/20 on the binary of every Codex review round
  (`e9d29932e2ccd509` before the review and four more in between). Logs:
  `log/rl-stepA/parity20-{off,stock-log,net}-2026-09-17.log` (before the
  review) and `parity20-final-{off,stock-log,net}-2026-09-17.log`.
- `tools/parity.py --solver-env KEY=VALUE` (repeatable) sets environment
  for the solver-13 run only, e.g. `--solver-env SAT_POLICY=stock` for the
  policy-on-STOCK check; kissat never sees it.
- `Cargo.toml` gives the `test` profile `opt-level = 2` (debug assertions
  kept): the integration tests run the binary on formulas with tens of
  thousands of conflicts, a minute per test file unoptimized and about ten
  seconds at level 2. `build.sh` and the release profile are unchanged.

**Step-0 constant-knob sweeps: headroom (2026-09-16).** Baseline 2 of the
RL plan's ladder (§6.1): for each knob the scheduler will move, does one
constant other than stock win on average, and how much is there to gain if
every cell got its best constant? Stage 1 (the intervals, the restart
margin, sweep effort) is done; stage 2 (per-pass effort, reduce fraction)
is running and gets its own note. Setup: `tools/rl_step0_sweeps.sh`,
18 arms × the 100 cells of `benchmarks/sat-comp-2025-medium`, 1800 s,
16 GB, 32 pinned cores, one seed, no proofs (the cross-arm SAT/UNSAT
check is the oracle), frozen binary of tree d8d41eb (sha256
`be2103771bcddf30`), 2026-09-15 16:54 to 2026-09-16 05:21 on an otherwise
idle host. Run dir `log/abtest-rl-step0-stage1-2026-09-15-16-54-40`
(per-arm `results.tsv`; `report/report.txt` and `report/per_family.tsv`
from `tools/rl_sweep_report.py`). No failed rows, no contradictions.

Per arm (stock 73/100, wall PAR-2 124 812 s, tick PAR-2 3.69e12 W):

| arm | solved | tick PAR-2 v stock | wall PAR-2 v stock | +solved / −solved |
|---|---:|---:|---:|---:|
| reorderint 20000 (stock 10000) | **76** | **0.983** | **0.949** | +5 / −2 |
| reorderint 5000 | 74 | 1.000 | 0.956 | +4 / −3 |
| eliminateint 1000 (stock 500) | 74 | 1.002 | 0.980 | +3 / −2 |
| sweep off (`--sweep=0`) | 72 | 1.050 | 1.018 | +1 / −2 |
| modeint 2000 (stock 1000) | 71 | 1.051 | 1.036 | +3 / −5 |
| probeint 50 (stock 100) | 71 | 1.070 | 1.058 | +4 / −6 |
| sweepeffort 200 / 50 (stock 100) | 71 / 71 | 1.022 / 1.053 | 1.037 / 1.051 | +1 / −3 |
| probeint 200 | 70 | 1.088 | 1.072 | +3 / −6 |
| rephaseint 500 / 2000 (stock 1000) | 70 / 70 | 1.091 / 1.124 | 1.050 / 1.085 | +3 / −6 |
| reduceint 500 / 2000 (stock 1000) | 69 / 69 | 1.114 / 1.064 | 1.064 / 1.089 | +2 / −6, +3 / −7 |
| modeint 500 | 69 | 1.070 | 1.061 | +2 / −6 |
| restartmargin 20 / 5 (stock 10) | 69 / 68 | 1.117 / 1.103 | 1.062 / 1.107 | +2 / −6, +2 / −7 |
| eliminateint 250 | 67 | 1.152 | 1.118 | +2 / −8 |

- **Best global constant.** `reorderint=20000` beats stock on all three
  metrics. `reorderint=5000` and `eliminateint=1000` each solve one cell
  more than stock at even tick PAR-2 (1.000× and 1.002×), which is inside
  the noise. The other fourteen constants lose solved cells. ±2 solved is
  noise on 100 cells (CLAUDE.md), so even the +3 of reorderint 20000 needs
  the 400-cell check before it means anything. Nothing here says "retune
  kissat"; it says stock's constants are near a local optimum for this
  suite, which is what a global retune of a mature solver should find.
- **Per-cell sensitivity is large and two-sided.** For every knob, a
  non-stock constant beats stock (solves what stock does not, or W lower
  by > 5 %) on 25-41 of the 100 cells and loses on 33-42 — nearly
  symmetric. Half or double of one interval changes the trajectory the way
  a different seed would, so the per-cell oracle below is inflated by that
  chaos: it is the maximum over 2-3 noisy draws per cell, not a promise.
- **Headroom (upper bounds).** Every cell at its best constant of one knob
  (that knob's oracle): tick PAR-2 gain 8-16 %, +2 to +6 solved. Every
  cell at its best arm over all 17 constants (the joint oracle): **81 v 73
  solved, tick PAR-2 0.678×, wall PAR-2 0.696×.** That is the bound the
  epoch policy must be measured against, and the two-sidedness above
  means the reachable part is smaller: a policy can only cash in the
  fraction of this sensitivity that is predictable from the observation
  vector.
- **Knob ranking for triage** (plan §6.1b; per-knob oracle tick gain,
  solved gain in brackets): reduceint 16.0 % [+4], modeint 15.9 % [+5],
  reorderint 15.5 % [+6], eliminateint 14.4 % [+5], probeint 14.3 % [+4],
  rephaseint 12.9 % [+4], restartmargin 12.4 % [+3], sweepeffort 8.4 %
  [+2]. Sweep effort is clearly last (the pass is throttled by its own
  delay counter already); the other seven are within a few points of each
  other, so round 0 should weight them by this list and the top 3-4 for
  rounds 1-3 are reorderint, modeint, eliminateint and reduceint, subject
  to round-0 effect sizes.
- **Per family** (first name token; full table in the TSV). `bp` (8
  cells, stock 6): reduceint 500 solves one more (7/8 at 0.79× the work),
  five constants tie stock at 0.92-0.95×, the rest lose 1-2 cells. `sc`
  (7, stock 5): reorderint 20000 +1 (0.63×), eliminateint 1000 and
  reorderint 5000 tie, the other constants −1. `kakuro` (3, stock 2):
  twelve of the seventeen constants solve the third cell at 0.13-0.52× the
  work — a cell stock is simply unlucky on. `roundrobin` (1, stock 0):
  seven constants solve it. `rbsat` (2, stock 1): rephaseint 2000 +1,
  eleven constants −1. `xor`, `tseitin`, `g` (5 cells): most constants
  cost a cell; reorderint 20000 and eliminateint 1000 hold stock's count
  on all three. The families where constants win are the ones where the
  win looks like perturbation rather than tuning — the same lesson as the
  two-sided counts.

**Stage 2 (per-pass effort, reduce fraction; 2026-09-16).** Same setup,
16 arms, 2026-09-16 05:21 to 16:10, run dir
`log/abtest-rl-step0-stage2-2026-09-16-05-21-33`; no failed rows, no
contradictions. This stage's stock arm solved 72 where stage 1's solved
73: two identical stock runs differ by one wall-limit cell, which is the
noise floor for the solved count. Per arm (stock 72/100, wall PAR-2
126 654 s, tick PAR-2 3.74e12 W; stock efforts in ‰: vivify 100,
eliminate 100, backbone 20, factor 50, forward 100, transitive 20,
walk 50; reduce fraction reducelow/reducehigh 500/900):

| arm | solved | tick PAR-2 v stock | wall PAR-2 v stock | +solved / −solved |
|---|---:|---:|---:|---:|
| forwardeffort 200 | **75** | **0.962** | **0.946** | +3 / −0 |
| backboneeffort 10 | **74** | **0.957** | **0.947** | +3 / −1 |
| eliminateeffort 50 | 73 | 0.990 | 0.980 | +1 / −0 |
| forwardeffort 50 | 73 | 0.990 | 0.975 | +1 / −0 |
| walkeffort 25 | 73 | 1.017 | 0.993 | +5 / −4 |
| reduce fraction halved (250/450) | 72 | **0.953** | 0.984 | +5 / −5 |
| walkeffort 100 | 72 | 0.993 | 0.986 | +4 / −4 |
| vivifyeffort 50 | 72 | 0.992 | 0.996 | +2 / −2 |
| transitiveeffort 40 | 72 | 1.014 | 0.978 | +2 / −2 |
| backboneeffort 40 | 72 | 1.022 | 1.019 | +1 / −1 |
| vivifyeffort 200 | 71 | 1.057 | 0.999 | +4 / −5 |
| eliminateeffort 200 | 71 | 1.015 | 1.018 | +0 / −1 |
| factoreffort 100 / 25 | 71 / 71 | 1.009 / 1.045 | 0.999 / 1.031 | +1 / −2, +2 / −3 |
| transitiveeffort 10 | 70 | 1.049 | 1.044 | +1 / −3 |

- **Effort knobs perturb the trajectory less than intervals** (an effort
  limit only truncates a pass): the per-cell beats/loses counts are 11-40
  v 12-38, against 25-41 v 33-42 for the intervals, and the low-count
  knobs show a consistent direction rather than chaos. Three constants
  beat stock on every metric: double forward-subsumption effort
  (forwardeffort 200: +3/−0), half backbone effort (backboneeffort 10:
  +3/−1) and half eliminate effort (eliminateeffort 50: +1/−0); halving
  the reduce fraction keeps the solved count and cuts tick PAR-2 by 5 %.
  All within ±2-3 solved on 100 cells, so each needs the 400-cell check
  before it is a retune; but the direction (spend less in backbone and
  eliminate, more in forward subsumption) is the first real tuning signal
  of step 0.
- **Headroom.** Per-knob oracle tick gain: reducefrac 18.5 % [+5],
  walkeffort 16.0 % [+6], vivifyeffort 14.2 % [+4], transitiveeffort
  8.2 % [+2], backboneeffort 8.1 % [+3], forwardeffort 7.0 % [+3],
  factoreffort 6.7 % [+2], eliminateeffort 2.3 % [+1]. Joint oracle over
  the 15 constants: 81 v 72 solved, tick PAR-2 0.690×, wall 0.706×. The
  top of this ranking (reduce fraction, walk, vivify) is again the
  chaos-prone end — the knobs with the highest beats *and* loses counts —
  while the knobs with a clear best constant (forward, backbone) rank
  low. For stage 2 (plan §2.2) both matter: the ranking says where a
  per-cell choice has room, the global constants say where the menu's
  centre should move.
- **Per family.** `kakuro`'s third cell is solved by 11 of 15 constants
  (0.18-0.56× the work), as in stage 1. `sc` (7, stock 5): factoreffort
  25 and forwardeffort 200 +1, eliminateeffort 200, reducefrac half and
  walkeffort 100 −1. `bp` (8, stock 6): backboneeffort 10 +1; factor,
  transitive 10 and both walk efforts −1. `lockchart` (3, stock 1):
  reducefrac half and walkeffort 100 +1. `xor` (3, stock 1): both vivify
  efforts and walkeffort 100 −1.

Status (2026-09-04): all engines ported; counter parity exact at
`--conflicts=100000` on the 20 discriminating cells + 14 medium cells and on
full brocard runs; wall ratio v kissat at parity: 19-cell quiet screen geomean
0.9996, 18-cell loaded screen 0.994 (per-cell 0.94-1.06; Kakuro and
VanDerWaerden 1.06 are the outliers). **Third paired run (generic build, 2026-09-05/06): solver 13 AHEAD —
solved 313 v kissat 312, PAR-2 785,779 v 791,180 (0.993x), both-solved
wall geomean 0.9875x over 284 cells, zero contradictions** (logs
`log/kissat-full-accept3-20260905-164711` v
`log/solver13-full-accept3-20260905-164713`; details below). **Phase-8
acceptance run PASSED 2026-09-04** (paired 400x2 @ 3600 s / 16 GB / 16+16 pinned physical cores,
no proofs, `tools/run_kissat_full.sh`; logs
`log/kissat-full-accept-20260904-072748` v
`log/solver13-full-accept-20260904-072750`, report via
`tools/compare_full_runs.py`): solved **312 v kissat 313** (floor 306.7),
PAR-2 **792,489 v 786,872 = 1.0071x** (ceiling 1.02x), **zero SAT/UNSAT
contradictions**; 284 both-solved cells wall geomean **1.013x** (158,884 v
156,921 s). The one lost cell, lockchart-group3-L15-K29-p4, is a 53 s
wall-coin (kissat UNSAT at 3546.6 s); the other two wall-band cells
(frb80-14-1 3396 s, bp4_LPI_FPBEQ_ZR 3071 s) held. Both arms abort on
memory (exit 134) on pj2002_k500 and 17.normalised. Residual by family:
Kakuro 1.15-1.22x (4 cells; 490 MB CNFs, parse/giant-clause bound),
REGRandom 1.15x, crusti 1.11x; the `N.normalised` family runs 0.81-0.94x
(faster than the C). Measured results
(all tier-1 probes, NOT acceptance evidence):

- 2026-08-30 `tools/smoke_test.sh`: 9/9 PASS — valid SAT models,
  drat-trim-verified UNSAT proofs, default options
  (log/2026-08-30-23-04-12).
- 2026-08-30 parity, smoke corpus (9 CNFs): exact 80-counter `-s` match vs
  reference kissat under `--plain --no-lucky` and `--plain`.
- 2026-08-30 parity, `benchmarks/discriminating` (20 xz instances, real
  SAT-comp cells), `--conflicts=10000 --plain --no-lucky`: **18/20 at exact
  80-counter parity** — identical conflicts/decisions/propagations/ticks/
  restarts over 10k conflicts of real search. The other 2 cells hit the
  harness 600 s cap in BOTH binaries (no divergence observed; scratchpad
  disc_parity.log of session 2026-08-30).
- 2026-09-03 sweep-substitute divergence found and fixed: kissat's
  `substitute_connected_clauses` new_size>2 path ends in a `q--` that
  decrements a *shadowed* inner lits cursor, not the outer watch pointer, so
  the reference keeps a stale occurrence of the substituted clause in the old
  literal's list (later garbage-collected via dense propagation). Our port
  had implemented the intended move semantics; now matches the C behavior
  (see PORT NOTE in `src/sweep.rs`). Isolated via SWEEP_DEBUG watch-list
  hash dumps + per-ref tracing on
  `benchmarks/discriminating/*brocard_problem_large.cnf.xz`.
- 2026-09-03 parity, brocard_problem_large **full default-config run to
  completion** (no limits): both `s UNSATISFIABLE`, all 80 `-s` counters
  exact including probing_ticks 100764057 (was +5 drift pre-fix), ~150 s of
  real search with 3 sweeps, full inprocessing.
- 2026-09-03 parity, `benchmarks/discriminating` (20 xz instances),
  **full default config** `--conflicts=10000`: **20/20 at exact 80-counter
  parity** (statuses match; includes 2 SAT and 2 UNSAT full solves within
  the limit). All inprocessing engines active. Command:
  `python3 solver/13-kissat-rs/tools/parity.py --conflicts 10000
  --timeout 900 benchmarks/discriminating/*.xz`.

- 2026-09-03 parity, `benchmarks/discriminating` (20 xz instances), **full
  default config `--conflicts=100000`**: **20/20 at exact 80-counter
  parity** — 10x the previous horizon; multiple full solves inside the
  limit (battleship SAT, Kakuro SAT, REGRandom UNSAT, brocard UNSAT).

Performance notes (tier-1, brocard full default runs, quiet-ish host):

- 2026-09-03 wall gap vs reference: ~8.5% slower overall (87.4 v 94.8 s
  totals; search +6%, probe/simplify/vivify/sweep +20-25%, parse 1.19x).
  Earlier `--profile=4` phase ratios (decide 10x, lucky 7x, parse 4x) were
  measurement artifacts of the old `process_time()` reading and parsing
  /proc/self/stat per profile START/STOP; resources.rs now uses libc
  getrusage/gettimeofday exactly like the C. With the honest clock the
  `--profile=4` totals differ by ~3.5% and counters remain exact.
- 2026-09-03 REJECTED: software-prefetch of the next watched clause in
  `propagate_literal` (solver12 bead 5b2.8.1 pattern). Paired simultaneous
  brocard A/B: 99.50 s with prefetch v 96.25 s without (+3.4% regression),
  counters identical. solver13's 2-word interleaved watch layout does not
  benefit; do not re-add without new evidence.
- 2026-09-03 **profiler unlocked** (perf_event_paranoid=1, perf + valgrind
  present) and the propagation gap closed structurally, all measured with
  the same protocol: simultaneous pinned-core brocard full default runs,
  candidate v previous step v reference kissat, 80-counter parity checked on
  every arm (exact throughout). Session start on this protocol: 109.5 s v
  kissat 96.4 s (**+13.7%**).
  1. `#[inline(always)]` on the fast-assign chain (`assign`,
     `fast_binary_assign`, `fast_assign_reference`, `assignment_level`,
     `push_vectors`, `push_blocking_watch`, `delay_watching_large`,
     `watch_large_delayed`). perf showed C's `kissat_search_propagate` as one
     73.7% frame while ours split into `search_propagate` 52.8% +
     out-of-line `assign` 12.8% + `push_vectors` 10.6%; the C are all header
     `static inline`. 109.53 → 102.60 s (**−6.3%**), kissat 96.36 s alongside.
  2. `struct assigned` repacked to 16 bytes (`internal.rs`: the five bools as
     bits of one `flags` word, `repr(C)`, compile-time size guard). Five
     plain bools made it 20 bytes — a 25% larger var-indexed array on the
     hottest random-access path. 102.14 → 100.39 s (**−1.7%**), kissat 95.17 s.
  3. `sort_literals_inline`/`move_smallest_literal_to_front`
     `#[inline(always)]` (C static inline; ours was a separate 0.6% symbol),
     `watch_large_clauses` walked by word offset with unchecked reads, and
     `backtrack_without_updating_phases` loops with unchecked trail/assigned
     indexing. 102.00 → 99.76 s (**−2.2%**), kissat 95.80 s.
  4. PUSH_ARRAY ported unchecked (`resize.rs` keeps `trail` capacity at
     `size`, `assign` writes without the Vec grow check) plus unchecked
     indexing in `move_smallest_literal_to_front`: 100.83 → 100.39 s
     (−0.4%, within run noise; kept for structural fidelity), kissat 96.35 s.
  5. `substitute_clauses` literal loop read unchecked (it carried +73% of
     the C's branches): 100.38 → 99.17 s (**−1.2%**), kissat 95.52 s.
  6. Unchecked `*propagate++` trail read and `WATCHES (not_lit)` lookups in
     the propagation path and the assign prefetch: 100.00 → 99.97 s (no
     measurable change; kept — it is the C's shape), kissat 95.25 s.
  Net: 109.5 → 99.2 s on the same deal, gap **+13.7% → +3.8%**.
- 2026-09-03 **brocard was the memory-bound best case.** Paired step-5 v
  kissat at `--conflicts=100000` on other discriminating cells (identical
  conflict counts, statuses match): circuit 4.35 v 3.24 s (**1.34x**),
  Timetable_C_392 18.9 v 14.7 s (1.29x), Kakuro 92.7 v 71.9 s (1.29x),
  REGRandom 6.5 v 5.4 s (1.19x), battleship 0.38 v 0.29 s. perf on those
  cells: every engine 15-90% slower with the same trajectory — the crate's
  checked indexing (+27% branches over the C) exposed once misses stop
  hiding it. Two universal fixes, each paired on brocard + circuit +
  Timetable with parity exact:
  7. `ClauseRef`/`ClauseMut`/`arena.clause()` unchecked in release
     (debug-asserted), like `kissat_dereference_clause` under NDEBUG:
     circuit 4.31 → 4.19 s, Timetable 20.82 → 19.48 s, brocard 100.41 →
     98.39 s (C 3.26 / 15.63 / 96.35).
  8. `src/uvec.rs`: `UVec<T>`, a Vec newtype whose `[]` is unchecked in
     release (range slicing stays checked); values, marks, assigned, flags,
     links, watches, trail, frames, the three phase arrays and the shared
     watch stack switched to it — ~800 index sites at once with no call-site
     edits. circuit 4.20 → 3.92 s (**1.20x** C), Timetable 20.18 → 17.07 s
     (**1.18x**), brocard 99.42 → 96.80 s (**1.000x**, C 96.83).
  9. Loop-local base pointers (arena / values / watch stack) inside
     `propagate_literal` only — the C's `ward *const arena`, `value *const
     values` locals — with `fast_assign` still taking `&mut Solver` (the
     earlier rejected variant also threaded the pointers through assign).
     4-way paired with step 8 and kissat: circuit 1.191x → 1.164x, brocard
     1.015x → 1.009x, Timetable unchanged. Kept.
  10. kitten's per-var/per-lit arrays (`vars`, `links`, `marks`, `values`,
     `failed`, `phases`, `import`, `watches`) on `UVec`: Timetable 1.187x →
     1.138x, circuit 1.164x → 1.152x, brocard 1.009x → 1.007x.
  11. `Heap` arrays (`stack`, `score`, `pos`) on `UVec` and the heap
     operations `#[inline(always)]` (C: `inlineheap.h` static inline, folded
     into `kissat_next_decision_variable`; ours were three separate symbols
     carrying 1.9k branch samples v the C's 458 on circuit). Paired 3-cell
     run: Timetable 1.230x → 1.118x (**−9%**), circuit 1.172x → 1.160x,
     brocard 1.014x → 1.001x.
  12. kitten: `klauses` on `UVec` and `#[inline(always)]` on the helpers
     kitten.c has as static inline (watch_klause, assign,
     propagate_literal, propagate, move_to_front, unassign, the klause
     accessors). Timetable 1.140x → 1.126x, brocard 1.016x → 1.007x,
     circuit flat.
  13. The remaining Solver stacks (analyzed, levels, minimize, poisoned,
     promote, removable, shrinkable, clause, shadow, delayed, etrail, units,
     sorter) on `UVec`: circuit 1.160x → 1.142x, SCPC-500-14 1.203x →
     1.180x, Timetable/brocard flat (paired 4-cell run, parity exact).
  `parity.py --conflicts 100000` (20 discriminating cells, full default
  config): 20/20 exact on the step-5, step-10, step-11 and step-13 (HEAD
  4b0ba3f) binaries; every
  step verified 80-counter exact on brocard + circuit + Timetable (+ SCPC
  from step 13 on).
- 2026-09-04 wider paired check, step-11 v kissat, 10 `sat-comp-2025-medium`
  cells at `--conflicts=100000` (all UNKNOWN at the limit, identical conflict
  counts): ratios 1.137-1.201, i.e. **~1.17x on search-bound cells**;
  brocard-class memory-bound cells sit at 1.00-1.01x. On SCPC-500-14
  `perf stat`: instructions +6.6%, branches +14%, L1-icache misses 2.9x the
  C's (30.6M v 10.7M), dcache misses equal; per-function instruction counts
  put `search_propagate` EQUAL to the C (84557 v 84305 samples) — the
  residual is the analyze cluster (+8%), kitten (+25%), sparse collect, and
  front-end pressure.
- 2026-09-04 REJECTED: `#[inline(never)]` on deduce_first_uip_clause /
  bump_analyzed / shrink_clause / minimize_clause / learn_clause to mirror
  kissat's no-LTO translation-unit boundaries (the icache profile put 30% of
  RS misses in the fully-inlined `analyze`). Same-core `perf stat` on SCPC:
  icache misses 27.5M → 28.8M (no reduction), cycles −1.3%; paired 4-cell
  run circuit −0.8%, SCPC −1.5%, Timetable +1%, brocard flat — a wash, so
  not kept. The icache excess is not from analyze's inlining.
  Whole-program `perf stat` at that point: cycles +3.4%, instructions
  +15.6% (253.8G v 219.6G), branches +27% (49.1G v 38.7G), L1/LLC misses
  equal — the residual is instruction overhead hiding under memory latency,
  not extra misses.
- 2026-09-04 **the icache/fmt residual found and fixed: eager verbose-message
  formatting.** `print::extremely_verbose/very_verbose/verbose/phase` take
  `impl Display`, and ~160 call sites built the message with `format!(...)`
  BEFORE the verbosity check — so every restart (`restarting`), every
  `kimits::delaying` and every inprocessing phase formatted floats and
  malloc'd a String at verbosity 0 (the C's `kissat_extremely_verbose` is a
  macro that tests verbosity first). perf: `float_to_decimal_common_shortest`
  0.75% of circuit cycles, `format_inner`+`malloc` 5.5% of the L1-icache
  misses. Fix: `format!` → `format_args!` inside those calls (arguments are
  still evaluated, formatting is not; `fmt::Arguments` is `Display`), and
  `very_verbose_if_not_bumpreasons` takes `impl Display`. Paired 3-rep runs
  (s15/s16 v kissat v step-13, pinned, idle siblings): SCPC-500-14 5.03 →
  4.51 s (C 4.32: **1.16x → 1.044x**), circuit 3.58 → 3.26 s (C 3.17:
  **1.13x → 1.03x**), Timetable 16.9 → 16.4 s (C 15.3: 1.11x → 1.07x),
  brocard 93.4 v 91.9 s (1.017x); counters exact on all four. SCPC `perf
  stat`: icache misses 27.9M → 20.8M (C 10.0M), instructions +3.0%, branches
  +6.4%, cycles +6.6% (C reference).
- 2026-09-04 kitten `import_literal` `#[inline(always)]` (C static, inlined
  into `kitten_clause_with_id_and_exception`; ours was an out-of-line call
  with the +0x0/+0x7 prologue visible in the profile) and `enlarge_external/
  enlarge_internal` `#[cold] #[inline(never)]`. 4-way paired: Timetable
  −1.8%, circuit +1.2%, SCPC +0.9% — a wash, kept for the C's shape. Also
  inlining new_reference/new_original_klause/export_literal was no better.
- 2026-09-04 Timetable phase split (`--profile=2`, both binaries; note
  `--profile=4` doubles the runtime of BOTH arms and hides the gap): search
  0.983x, simplify 1.104x (eliminate **1.161x**, +0.57 s of the +0.88 s
  total), probe 1.068x (sweep 1.12x, factor 1.11x). The remaining excess is
  in elimination (kitten definition extraction, `inlined_connect_clause`
  2x — its cost sits on the `*end != INVALID` watch-slot read in
  `push_vectors`, same instruction the C pays for), not in search.
- 2026-09-04 wider paired check (step-17b = HEAD 7d46c4c, 14
  `sat-comp-2025-medium` cells, `--conflicts=100000`, 7 pairs at a time on
  physical cores with idle siblings; scratchpad `wideout/`): see the plan
  handoff for the table — ratios 0.99-1.06 on the 11 search-bound cells,
  counters exact everywhere.
- 2026-09-04 **Kakuro family (post-acceptance item 1).** Paired
  `--profile=2` on Kakuro-easy-112 (490 MB CNF, 18.8M irredundant clauses):
  total 1.152x — probe 1.20x (congruence 1.24x, vivify 1.24x, sweep 1.23x,
  walking 1.20x), preprocess 1.24x, search 1.07x. `perf stat`: instructions
  +10.6%, LLC misses +9%, memory-stall cycles +14%; per function
  `watch_large_clauses` 74.4 v 53.5 G cycles at only +7% instructions (its
  cost is the DRAM miss on the watch-stack slot read in `push_vectors`),
  `extract_gates` +52% instructions (`init_xor_gate_extraction` alone 26 G:
  per-literal `arena.clause(ref).lit(i)` + checked `Vec` counter indexing),
  `walk` +56% (the same push in `connect_large_counters`).
  Fixes, each paired (RS new v kissat v RS previous, pinned, counters exact
  on Kakuro + Timetable + circuit + SCPC):
  14. `vector::PushCursor` — the watch-stack base pointer, length, capacity
     and `usable` decrement hoisted across a clause loop with the generic
     `push_vectors` as the slow path (enlarge relocates the stack, so the
     cursor re-syncs after it). Through `&mut Solver` LLVM reloaded all four
     around every store. Used by `watch_large_clauses`,
     `connect_irredundant_large_clauses`/`inlined_connect_clause` and walk's
     `connect_large_counters`. Kakuro 84.8 → 82.6 s (`watch_large_clauses`
     74.4 → 64.1 G cycles), circuit −1.8%.
  15. Congruence XOR/ITE counting passes walk `clause.lits()` slices with
     `UVec` counters (C pointer walks): −6 G instructions, −0.9% cycles;
     `extract_gates` cycles now equal to the C's congruence cluster.
  16. `move_smallest_literal_to_front` best-update in select form (the C's
     cmov chain): a wash — LLVM still branches; kept for shape.
  17. Verbose-format laziness (above) had already taken restart/delay cost.
  State: Kakuro **1.152x → 1.072x** (78.2 v 73.0 s, `--profile=2` split:
  congruence 1.13x +2.1 s, preprocess 1.14x +1.4 s, vivify 1.09x +1.1 s,
  walking 1.15x +0.8 s, search 1.05x); Timetable **1.026x** (eliminate 1.04x,
  sweep 1.07x, factor 1.09x, backbone 1.17x). The residual in the push
  loops is memory-stall time on identical accesses (layout verified: the
  `[vectors] enlarged`/defrag phase lines match the C's line for line) that
  no code-shape change so far recovers; the no-sort experiment showed the
  literal sort itself is ~2.5 s of Kakuro's 80 s in both arms.
- 2026-09-04 **THE LOAD-SENSITIVITY LAW (REGRandom, factor).** The
  acceptance run's REGRandom 1.15x is not reproducible on a quiet host: paired
  quiet runs put step-21 at **0.98x** kissat (5.10 v 5.19 s), but with 28
  background kissat Kakuro runs on other cores (the acceptance run is a
  32-way load) it is **1.10-1.14x**, core-swap and 4-way layouts agreeing.
  `perf stat` under load: identical offcore requests, L2 misses, LLC loads
  and GHz; front-end fine (RS has fewer undelivered uops and 3x fewer branch
  misses); but `cycle_activity.stalls_l3_miss` 8.98 v 7.27 G and instructions
  +13% (38.4 v 34.1 G). Same misses, less overlap: the extra instructions
  between misses fill the OOO window, so each miss costs more once latency
  rises under load. **Screen perf candidates under load** (28 background
  `kissat -n -q Kakuro.cnf` on cores 8-35, the pair on 2/4) — quiet paired
  timing understates the metric-relevant gap. Instruction count in
  miss-heavy loops is the lever, not miss count.
  18. factor `next_factor`/`factorize_next`: the `for j in 0..size {
     clause(ref).lit(j) }` and `for wi in begin..end { stack[wi] }` loops
     rewritten as slice walks (addr2line attribution had Range::next + lt +
     unchecked_add at ~21% of factor's retired instructions). Instructions
     38.4 → **33.9 G (C 34.1)**; REGRandom **0.92x quiet, 0.96x under load**
     (was 1.12x loaded). Counters exact.
  19. The same two rewrites applied crate-wide by a compile-checked
     transform (scratch `slicify.py`/`slicify2.py`: rewrite every syntactic
     match, build, revert the sites the borrow checker rejects — bodies that
     mutate the arena or call `&mut Solver` methods — and repeat): 55
     literal loops in 19 files + 17 watch-range loops in 7 files. circuit
     −1.8%, SCPC −2.2%, Timetable −0.8%, Kakuro/REGRandom flat; counters
     exact on all five cells; `parity.py --conflicts 100000` on the 20
     discriminating cells: **20/20** for both the literal-loop (09fb200) and
     the watch-range (3843da7) transform binaries.
- 2026-09-04 **loaded screen of the current tree (3843da7)**: 14 wide
  medium cells + circuit/SCPC/Timetable/REGRandom, RS v kissat paired on
  cores 2/4 under 28 background kissat Kakuro runs (scratch
  `loadscreen.sh`, `loadout24/`): case7 1.016, clqcl_50 1.017, crusti 1.046,
  DLTM 0.982, oddball_24 1.006, QG7 1.007, ramsey 1.000, reconf10 0.982,
  RoundRobin 1.036, sudoku 1.029, tseitin_grid 0.981, VanDerWaerden 1.037,
  velev 1.041, xor_op 1.039, circuit 1.006, SCPC 1.043, Timetable 1.032,
  REGRandom 0.987 — geomean ≈ **1.02x under load**, no cell above 1.05x.
  Kakuro quiet 1.077x (instructions 327.5 v 306.4 G, cycles 292 v 271 G).
- 2026-09-04 REJECTED: unchecked `counts[lit]` in vivify's `count_literal`
  and unchecked `dst[pos]` in `radix_scatter` (addr2line attribution had
  them at 19% / 8% of their functions' instructions). Instructions −2 G on
  Kakuro but wall +0.5% Kakuro / +1.2% Timetable, each variant alone also
  slower (vivify-only +3.4%, radix-only +2.3% on a 4-way Timetable run) —
  a code-layout effect; not kept.
- 2026-09-04 **crusti (1.046x loaded) → factor's `schedule_factorization`
  scan, and the `Flags` struct.** crusti's gap is factor again (4.18 v
  3.27 s, 1.28x; instructions 14.5 v 11.5 G) and the inlined-function
  attribution put 44% of factor's cycles in `schedule_factorization`'s
  `for idx in vars { if flags[idx].active ... }` scan (factor.rs:170-171).
  Our `Flags` was 10 bytes (ten bools + a u8) v the C's 2-byte bitfield
  struct, so every var-indexed scan (factor rounds, backbone, sweep,
  eliminate scheduling) streamed 5x the cache lines.
  20. `Flags` packed into a `#[repr(transparent)] u16` with `active()` /
     `set_active(v)` / `factor()` / `factor_or(bits)` / `factor_and(bits)`
     accessors (bit order = the C declaration order; compile-time size
     guard); 124 access sites rewritten by regex over the receiver shapes
     `flags[..]`, `f`, `flags`, `pivot_flags`, zero compile errors. Quiet
     3-rep: crusti 15.92 → 15.58 s (C 14.97, **1.063 → 1.040x**), REGRandom
     5.67 → 5.58 s (C 5.75), circuit 3.22 → 3.27 s (+1.6%, layout), Timetable
     flat, Kakuro +1%; counters exact on all five; 20-cell parity below.
- 2026-09-04 **LAYOUT DIVERGENCE FOUND AND FIXED (invisible to the counters):
  two `SET_END_OF_WATCHES` ports set `watches[lit].end` directly.** The C
  macro is `kissat_resize_vector`: it also memsets the freed tail to
  INVALID and adds it to `vectors.usable`. Without the poison the next push
  into that list saw an occupied slot and relocated the whole vector
  (doubling it, leaving holes), and with `usable` undercounted the defrag
  never triggered. Found via crusti's page faults (119k v the C's 53k) →
  `/usr/bin/time` maxrss **320 MB v 74 MB** → massif (portable build; the
  native one SIGILLs valgrind) putting 512 MB in `enlarge_stack` under
  factor's `new_binary_clause` → `-v` logs: RS `[vectors] enlarged` 2^23 →
  2^27 during factorization-1 where the C stays at 2^23, then `[defrag]
  freed 71M usable 98%` v `5.6M 82%`. Sites: `factor::eagerly_remove_watch`
  (factor.rs:735) and `sweep::substitute_connected_clauses` (sweep.rs:1027);
  every other SET_END_OF_WATCHES port already used `vector::resize_vector`
  (the `watch.c` `end -= 1/2` decrements are faithful as written). After
  the fix (a118aa9): crusti maxrss 72 MB, faults 55k, and the
  `[vectors]`/`[defrag]`/`[arena]` sequences identical to the C on crusti,
  REGRandom, Timetable, circuit, SCPC, velev, sudoku and Kakuro; counters
  exact on all; `parity.py --conflicts 100000` on the 20 discriminating
  cells: **20/20** (a118aa9 + packed Flags). All 80 `-s` counters had been
  exact throughout — the bug only changed memory layout, RSS and wall.
  **New oracle: `parity.py --phases`** runs both binaries with `-v` and
  diffs the bracketed phase lines (numeric tokens at 1e-5 relative
  tolerance, report rows/options skipped); the pre-fix binary fails crusti
  at phase line 73 (`[vectors] enlarged to 2^24` where the C prints the
  factorization summary). Run it alongside the counter check for any
  change touching vectors, watches, arena growth or clause allocation.
  Remaining cosmetic `-v` differences: `format_count` prints `1000` where
  the C prints `1e3`, and `{}` floats print full expansions where the C
  uses `%g` (values identical).
- 2026-09-04 loaded screen of the layout-fix + packed-Flags tree
  (a118aa9/83236e6; 28 background kissat Kakuro runs, pairs on cores 2/4):
  case7 1.031, clqcl 1.017, DLTM 0.989, oddball 1.003, QG7 1.016, ramsey
  1.013, reconf10 0.979, RoundRobin 1.030, sudoku 1.026, tseitin_grid 1.000,
  velev 1.006, xor_op 1.037, circuit 1.015, REGRandom 0.982; clean re-runs
  (2 reps): **crusti 1.017** (was 1.046 before the layout fix),
  VanDerWaerden 1.046, SCPC 1.044, Timetable 1.039. Geomean ≈ 1.02x; the
  residual under load is now the search-bound cells at ~1.04x (SCPC,
  Timetable, VanDerWaerden), i.e. the analyze cluster's +8% instructions
  and propagation's extra loads, not inprocessing.
- 2026-09-04 **search-bound cells → propagate's delayed re-watching.** SCPC
  instructions were +17% v the C (`search_propagate` +14%, attribution:
  `push_vectors` 17% of it — `Vec::push`'s grow check on every append even
  though the capacity mirror guarantees room, plus the per-push reloads).
  21. `Vectors::push_unchecked` for the `*stack->end++ = e` cases of
     `push_vectors`, and `watch_large_delayed` pushing through a hoisted
     `PushCursor`. 3-rep paired: **SCPC 1.045x → 0.99x**, circuit 0.995x,
     Timetable 1.026x; instructions −2.8%; counters exact. Under load: SCPC
     0.994, xor_op 1.013, Timetable 1.027.
- 2026-09-04 REJECTED (reverted, 1ee63cd): `generate_resolvents` walking the
  `WClause` literals by raw pointer (the C's tmp-clause shape). Timetable
  instructions −2% and wall a wash, but **case7 +11% and RoundRobin +10%**
  (quiet 5-arm paired, both reps) — the elimination-heavy small cells lose
  badly; not understood (likely codegen of the enum + pointer pair), not
  kept. Lesson: a wash on one cell is not evidence; the quiet 19-cell screen
  is the gate for structural rewrites.
- 2026-09-04 VanDerWaerden (1.05x): fewer instructions than the C (65.3 v
  67.6 G) but +5% cycles from **+25% branch misses** (543M v 435M), 39% of
  factor's misses on the inner literal loop's exit in `next_factor` — the
  same loop as the C's; a predictor/layout effect. Combining the
  FACTOR|NOUNTED test (b699908) changed nothing (kept for shape).
- 2026-09-04 **loaded screen, step-30 tree**: geomean **0.994x** over 18
  cells (crusti 0.999, SCPC 0.991, circuit 0.988, REGRandom 0.939, DLTM
  0.957; worst VanDerWaerden 1.051, sudoku 1.028, Timetable 1.022).
- 2026-09-04 **QUIET SCREEN, final tree (1ee63cd = the step-30 source)**: 19
  cells, each cell run as two simultaneous RS/C pairs with the cores
  swapped (scratch `quietscreen.sh`, `quietout3/`), ratio of the two-run
  means: clqcl 1.006, case7 0.985, crusti 0.993, oddball 0.992, QG7 0.977,
  DLTM 0.971, RoundRobin 1.012, ramsey 0.971, reconf10 0.986, tseitin_grid
  0.990, VanDerWaerden 1.059, sudoku 1.024, circuit 1.000, xor_op 1.013,
  velev 1.009, SCPC 0.993, REGRandom 0.935, Timetable 1.023, Kakuro 1.062 —
  **geomean 0.9996**. Loaded screen of the same source (above): **0.994**.
  `parity.py --phases --conflicts 100000` on the 20 discriminating cells
  with this binary: **20/20** counters and phase lines.
  Outliers left: Kakuro 1.06 (memory-bound giant; push-loop stall time),
  VanDerWaerden 1.06 (branch misses in factor's inner loop), sudoku/
  Timetable 1.02. A first (invalid) quiet screen had the two legs of each
  pair sharing a core mid-run and read 1.10 — check `ps -o psr` before
  trusting a screen.
- 2026-09-05 **THE BUILD FLAG: `-C target-cpu=native` costs 8-11% on this
  crate.** The second paired 400-instance run
  (`log/kissat-full-accept2-20260905-074218` v
  `log/solver13-full-accept2-20260905-074220`, parity tree 2a2bd42, native
  build) came back WORSE than the first: 311 v 313 solved, PAR-2 1.0202x,
  both-solved wall geomean **1.047x** (first run 1.013x), while the kissat
  arm reproduced the first run to 0.07%. Per-cell join: the RS arm was
  uniformly +3.4-4.2% slower on every cell above 10 s and unchanged below.
  Quiet full runs confirmed it (bp4_CSO_AM_IXA_LP 140 v 131 s, crafted 117
  v 109 s, identical conflicts), and a 14-binary bisect put every
  intermediate step binary at 128-130 s but the frozen binary at 138 s —
  same source as step 30. The difference was the build: the step binaries
  came from `cargo build --release` with `RUSTFLAGS` unset (generic
  x86-64), the frozen ones from `build.sh` (`target-cpu=native`, 797 zmm
  instructions). Controlled rebuilds of HEAD: crafted generic 106.2 /
  x86-64-v3 108.0 / native 114.9 s (C 105.5); SCPC 4.28 / 4.30 / 4.75 (C
  4.33); x86-64-v2 +6% on crafted. `build.sh` now builds generic (the
  reference kissat is also generic `-O3`). Every screen number in this
  README was measured on generic binaries; both full runs used native ones.
  Third paired run launched with the generic binary
  (`~/.cache/sat13-accept3/sat-solver`).
- 2026-09-06 **THIRD PAIRED 400-RUN (generic binary
  `~/.cache/sat13-accept3/sat-solver`, sha256 0bf3e08a30981590, tree
  2a2bd42 + generic build.sh 5e758d5)**: solved **313 v 312** (we convert
  lockchart-group3-L15-K29-p4 UNSAT at 3595 s where kissat times out — the
  same cell we lost as a wall-coin in run 1; nothing lost), PAR-2 **785,779
  v 791,180 = 0.9932x**, both-solved wall geomean **0.9875x** (284 cells,
  155,779 v 157,575 s), zero SAT/UNSAT contradictions, the two memory-abort
  cells identical in both arms. Wall-band cells: frb80-14-1 3279 v 3424 s,
  bp4_LPI_FPBEQ_ZR 3047 v 3118 s (both ours faster). Residual families:
  Kakuro 1.09-1.15x (memory-bound push loops), oddball_67 1.12x; the
  `N.normalised` family 0.80-0.91x, tseitin_grid 0.90x. This is the
  acceptance-quality evidence for the wall-parity claim: run 1 (native
  build) 1.013x, run 2 (native) 1.047x, run 3 (generic) 0.9875x.
- 2026-09-03 REJECTED: kissat's FAST_ASSIGN shape — hoisting raw base
  pointers of arena/assigned/values/watch-stack into `propagate_literal`
  locals and threading `values`/`assigned` through `fast_assign` exactly as
  `fastassign.h` does. Static solver-field reloads in the loop did drop
  (111 → 91) but the paired run was 104.62 s v 102.60 s inline-only (+2%)
  and `perf stat` showed +1.06% instructions and +2.9% LLC misses: LLVM's
  codegen with the separate raw pointers is worse (more spills), so the C
  idiom does not transfer. Do not re-add without new evidence.
