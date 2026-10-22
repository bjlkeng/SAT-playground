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

**Where it stands (2026-10-20; run S.8, bead `1v2.10`).** Against the C
kissat 4.0.4 in the acceptance setting (the 400 cells of 2025, 3600 s,
16 GB, 16 + 16 pinned cores, no proofs, `tools/run_kissat_full.sh`),
solver 13 with every default on (the structure passes, the parity engine
and the arithmetic pass; commit 2928093, frozen copy sha256
e3b035b91b1eecf4) solves **344 v 313**, PAR-2 **555,303 v 783,917
(0.708x)**, no contradiction, both-solved wall geomean 0.894x over 282
cells. Logs `log/kissat-s8-20261020-070218` v
`log/solver13-s8-20261020-070218`; per family, with wall and tick PAR-2,
in `log/s8-families-2026-10-20.txt`. Solver 13 alone solves 33 cells:
31 that the structure pass and the parity engine settle in seconds
(roundrobin 11, clqcl 6, php 4, tseitin 4, clique-coloring 3, mchess_20,
a multiplier miter, xor_op_n36) and two search cells
(BubbleVsPancakeSort_8_6 UNSAT at 3118 s, SC25_Timetable_C_496 SAT at
1976 s). Kissat alone solves 2: ITC2021_Late_10 and
lockchart-group3-L15-K29, the wall-coin cell of the earlier runs. Every
answer is checked: 313 match the paired run of 2026-10-19/20, which ran
this configuration with a proof for every answer (289 gated, 24 where the
checker timed out), and the other 31 were rerun with proofs
(`log/abtest-s8-verify-2026-10-20-15-39-45`: the same answers, 30 gated,
1 checker timeout). The slowest cells are satisfiable ones where the
structure pass's added units move the search, as a different seed would
(SC25_Timetable_C_406 38 → 454 s, sted2 178 → 2095 s; the passes
themselves cost about 5 s and 1.3 s there). One finding: the parity
engine counts the proof's table in its memory limit (`fits` in
`src/gauss.rs`), so with a proof file the engine can stay off where a run
without one turns it on. On dislog_a14_x14_n24 the engine runs briefly
without a proof (solved at 517 s) and never starts with one (1782 s). Of
the 344 solved cells, 339 do the same work with and without a proof;
three are cells the structure pass solves at once, where the proof adds a
little work, and the other two are the parity cells dislog and
sum_of_3_cubes_37_bits_87.

**Work clock (plan step A′, 2026-09-15).** The binary prints one extra
line at every exit, after the `[ resources ]` section:

```
c workclock ticks=… search_ticks=… probing_ticks=… backbone_ticks=… transitive_ticks=… factor_ticks=… substitute_ticks=… kitten_ticks=… eliminate_resolutions=… forward_steps=… walk_steps=… flipped=… conflicts=… decisions=… propagations=… k_res=7 work=…
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
- **`k_res` = 7 since step B.5 (2026-09-18).** Fitted from the 2025
  stock traces: the wall of each observation epoch regressed on the work
  of each kind spent in it gives 29.7 ns per tick and 206 ns per
  resolution (ratio 6.9; 8.1 with the tick kinds split), see "RL
  scheduler step B" below. The provisional value was 11 (2026-09-15,
  from a `--profile=2` run of the 20 discriminating cells: eliminate
  wall per resolution over search ticks per second, median 11.5, range
  3.3 to 22). The constant is `K_RES` in `src/statistics.rs`; the harness
  reads it from the line and never hard-codes it. Changing `k_res`
  changes only `work` (never a trajectory), so old results are re-priced
  offline from `ticks` and `eliminate_resolutions`; the 2025 traces and
  the 2026-09-05 acceptance runs printed `k_res=11`, and the RL tables
  (`tools/rl_cells.py`) recompute W at 7 from the logged counters.
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
| `SAT_POLICY_BRANCH` | `D:knob[,D:knob...]` | fork mode (A.8): at decision D (0 = D0, each D at most once) fork one child per alternative entry of the knob's menu (probe, eliminate, reduce, rephase, reorder, mode, margin, sweep), or, for `D:regime`, one child per regime of `SAT_POLICY_BRANCH_REGIMES`; needs `SAT_POLICY_LOG` and `SAT_LIMIT_TICKS` (children stop on the inherited budget); refused with a proof or `-o` file |
| `SAT_POLICY_BRANCH_ACTIONS` | `knob=e\|e[;knob=e\|e]` | restrict the entries forked per knob (menu values such as `0\|0.5\|2\|4`); the parent's own entry is allowed here (a stock child is the fork test) |
| `SAT_POLICY_BRANCH_JOBS` | 1..64 | live children per parent (default 4); the parent blocks at a branch point until a child exits (wall, not ticks) |
| `SAT_POLICY_BRANCH_HOLD` | 1..1024 | decision epochs a child keeps its entry in force (default 1: the branch epoch only); at each of its next K−1 decisions the child puts the same entry back, masked for the mode of that moment, whatever the parent's policy would choose, then returns to the parent's policy; recorded as `hold` in the child's header |
| `SAT_POLICY_BRANCH_REGIMES` | `regime[@hold][,regime[@hold]...]` | the children of a regime point (`SAT_POLICY_BRANCH=D:regime`, plan §11 2026-10-03): one child per item, in order (at most 32). A regime runs in place of kissat's own choice from the branch decision for its hold (decision epochs; default `SAT_POLICY_BRANCH_HOLD`), then kissat's alternation resumes: `focused` / `stable` (that mode only), `stay` (the mode of the moment, no switch), `sat` (kissat's `--sat` dials: target 2, restartint 50), `eager` / `lazy` (half / twice the restart margin and a quarter / four times the reluctant period), `reroll` (kissat unchanged with the solver's random stream re-seeded: the luck control). A held mode ends with one switch due at once, and the stable stint a focused block hands over to gets the tick budget of the last stable stint kissat sized itself, not one as long as the block. The child's header reads `"knob":"regime"`, `entry` is its index in the list and `regime` its name; its output prints the four options when the regime starts and when it ends |
| `SAT_POLICY_EPOCH_TICKS` | `X_o[,X_d]` | observation and decision epochs in `search_ticks`; default `8388608,134217728` (2^23, 2^27); `X_d` defaults to 16 × `X_o` and must be an integer multiple of it (decisions are checked at observation boundaries) |
| `SAT_POLICY_SEED` | integer | the policy's own generator seed (default 0); never touches the solver's `--seed` stream |
| `SAT_POLICY_TEMP` | number > 0 | spread of the random menus around stock: weight exp(−distance/temp), so 0.2 is almost always stock and 100 is uniform (default 1.0) |
| `SAT_POLICY_SEGMENT` | number ≥ 1 | mean sticky segment length in decision epochs (default 5) |
| `SAT_POLICY_LOG` | path | raw-state log, one row per observation epoch plus a terminal row (A.5, below); with `SAT_POLICY` unset it turns the policy on in stock mode, i.e. a stock trace with logging; an unwritable path, or one that aliases the CNF, the proof, a `-o` output, a standard stream or a wrapper-reserved file, is a usage error |
| `SAT_POLICY_LOG_RESERVED` | paths, one per line | extra files the log may not alias; set by `run.sh` for its capture and result files and its redirected stdout/stderr |

## The structure pass (plan §11, 2026-10-07; epic `SAT-playground-1v2`)

Not in kissat. Before the solver sees the formula, a pre-solve pass looks
for structure that search handles badly and settles what it can in
seconds. Each pass is a kissat-style option; a pass is off until its
measurement promotes it, and stays a switch after. **Promoted 2026-10-10:
`structsym` and `structparity` are on by default** (the combined 2025 run
298 v 282 and the single 2026 holdout 212 v 203, below); **2026-10-13:
`structcount`** (309 v 306 on 2025 with the parity pass's flip step;
not seen on the holdout); **2026-10-20: `structarith`** (313 v 312 on
2025, the two multiplier miters; not seen on the holdout; "The linear
arithmetic pass" below). The stock kissat path is `--structsym=0
--structparity=0 --structcount=0 --structarith=0` (and `--gauss=0`, the
parity engine below), which `tools/parity.py` passes unless
told `--structure`, and the test helpers' `run` too
(`tests/common/mod.rs`, `STOCK_PATH`). A promoted configuration is
exactly the measured one:

**Second seed (2026-10-14).** The shipped set (4ac5870) against stock,
both with `--seed=1` (`log/abtest-struct-seed1-400-2026-10-14-08-19-45`,
`log/struct-seed1-400-report-2026-10-14.txt`; 400 cells, 1800 s, every
answer checked, no contradiction): **313 v 284 (+34 / −5)**, wall PAR-2
0.783×, tick PAR-2 0.757×, validation 76 v 71. Seed 0 gave 311 v 282.
30 of the 34 gains are cells the passes refute in under a minute in
both seeds and stock never solves (Tseitin 4, pigeonhole variants 4,
clique colouring 4, RoundRobin 6, clqcl 6, MVRoundRobin 4,
`mchess_20`, `xor_op_n40_d3`); the other four are three satisfiable
cells that come and go with the seed and one unsatisfiable cell at
1793 s. The five losses are satisfiable cells where the search took a
longer path (cut at 1.06-7× stock's work: three `bp4`,
`sum_of_3_cubes_37`, `HCP-446-105`). Where both solve, the passes leave
the search exactly as it was on 166 of 279 cells (same conflicts and
decisions; the work differs only by the passes' own, median 2.4M, at
most 1.02G): spending the budget without a find does not move the
search. On the other 113 they derived something and the search went
another way, and the work summed over those cells and the rest is
0.93× on satisfiable and 0.91× on unsatisfiable cells. The ship arm
also reran an earlier arm with the same binary and seed (the base arm
of `log/abtest-struct-walk-400-seed1-2026-10-13-23-29-21`): one cell
differs (`16_16_and_wallace...` at 1793 s against a timeout), the work
is identical on all 312 cells both solve, the wall ratio median is
0.994. So wall noise moves one cell where the seed moves 31, and the
structure set's gain, +29 in both seeds, is a deterministic one.

| option | pass | status |
|---|---|---|
| `--structsym=1` | symmetry: interchangeable rows and fixing (`src/structsym.rs`) | **default on** since 2026-10-10; measured on the 400 cells and the holdout (below) |
| `--structparity=1` | parity: closed XOR systems fixed by cycle flips, and the flip symmetries of any formula (`src/structparity.rs`) | **default on** since 2026-10-10; the flip step since 2026-10-13 (below) |
| `--structcount=1` | counting: demand clauses against pairwise at-most-one groups, a Hall violation proved by cutting planes (`src/structcount.rs`); runs last, within `structcountticks` | **default on** since 2026-10-13 (below); sequential-counter and decision-diagram counts are step two |
| `--structcountticks=N` | the counting pass's own work limit in thousands (default 20K, 20M), counted from where the other passes stopped | since 2026-10-13 |
| `--structarith=1` | linear arithmetic: a miter of two multipliers over shared partial products refuted by summing their adder cells (`src/structarith.rs`); runs after symmetry, before counting, within `structarithticks` | **default on** since 2026-10-20 (313 v 312; below, "The linear arithmetic pass") |
| `--structarithticks=N` | the arithmetic pass's own work limit in thousands (default 20K, 20M), counted from where the earlier passes stopped | since 2026-10-19 |
| `--structcsp=1` | one-hot CSP decoding, SAT only | bead S.4, not yet |
| `--structsymext=1` | general automorphisms through an external tool (measurement only) | bead S.2 step two, not yet |
| `--structclauses=N` | the pass buffers at most N clauses (default 10M); past that it is off for the run | |
| `--structticks=N` | work budget of the passes in thousands of literal visits (default 1M, 10-100 s depending on the formula) | |
| `--structmore=N` | the symmetry pass's budget (thousands) once a matrix has been verified: the detection in progress and its fixings run to this limit, once (default 2M); a later detection starts only within `structticks` | since 2026-10-12 |

With every option off nothing runs: the parser's literals go straight to
the solver as before, the proof path is the stock one and
`tools/parity.py --corpus default` stays at 9 of 9. With a pass on, the
parser buffers the formula (`src/structure.rs`), the passes derive
clauses (units, binaries, at most the empty clause) and the solver
receives the derived clauses first and the formula after, so it solves
F' = F plus derived; a model of F' is a model of F, and the search's DRAT
proof is a proof of F'.

**Proofs.** The pass writes a VeriPB 3 proof next to the DRAT path:
`<proof>.pbp` over the input's clause numbering (clause i is constraint i,
duplicates and tautologies included), and `<proof>.derived.cnf` with the
derived clauses. A fixed literal is logged as Satsuma logs orbitopal
fixing: one `red (l* v ~l_i) : swap` step per other literal of the clause
with the row swap as witness, then `rup l*`, then every propagated unit as
`rup`; the empty clause ends the proof with `conclusion UNSAT`, otherwise
the proof ends with `output EQUISATISFIABLE FILE` naming F'. The gate
(`tools/proof_gate.py`, used by `feature_ablation.py` and
`validate_solver_result.py`) checks stage 1 with VeriPB (both formulas
handed over as OPB, since VeriPB 3 reads CNF input but not CNF output
files) and stage 2 with drat-trim on F'; a proof that ends in the empty
clause needs stage 1 only. `tools/setup_checkers.sh` builds VeriPB into
`tools/checkers/VeriPB`; `tools/bench.sh` uses the gate when the sidecars
are present. `tools/smoke_test.sh` checks a proof with plain drat-trim
against the input when the run left no sidecar, and through the gate
when it did (edited 2026-10-10 for the promotion, with the owner's
permission); its four UNSAT cells now report "verified by the gate".
Solver-side tests: `tests/structure.rs` (off means off, a pigeonhole
refutation through the gate, a tampered proof rejected, an unchecked
extension rejected, the pass's work in the work clock, a model of the
input on a satisfiable formula). The gate also runs a negative control:
the same proof against the extended formula plus one underived clause
must be rejected, which shows VeriPB compared the output file.

**Preview on the 400 cells (2026-10-08;
`log/abtest-struct-sym-400-2026-10-07-23-24-29`,
`log/struct-sym-400-report-2026-10-07.txt`).** `--structsym=1` against
the same binary with the pass off, 1800 s, 14 GB, 32 shared pinned
cores, one seed, proofs through the gate, on the pre-review frozen
binary (sha256 63476afc; solved counts valid, its tick numbers not,
since that binary kept the pass's work out of the work clock): no failed
row, no contradiction, 47 answers unchecked at the checker's budget (24
on the pass arm, 23 on stock). Solved **294 v 282 (+18 / −6)**, wall
PAR-2 0.923×; validation split 74 v 70 (+5/−1). Gained, all with checked
proofs: the eight plain RoundRobin cells (0.5-1.7 s), clique-coloring 4
of 4 (1.4-6.5 s), rphp 3 of 3 (2-9 s), fphp (0.1 s), clqcl_30_7_6
(1011 s, the pass plus search) and one timetable SAT cell. Lost six SAT
cells that stock solves in 200-1400 s (two argumentation, sum-of-cubes,
reconf, sted, one anon-pair): fixed literals change the search path on
satisfiable cells, the same trajectory luck the regime experiment
measured. **Run of record** on the committed binary (`6b4bec6`,
`log/abtest-struct-sym-400b-2026-10-08-08-44-21`,
`log/struct-sym-400b-report-2026-10-08.txt`, 2026-10-08 08:44 to
17:31, same setting): **294 v 282 (+18 / −6)**, wall PAR-2 0.924×,
tick PAR-2 0.903× with the pass's work in the clock; train 213 v 206
(+12/−5), validation 74 v 70 (+5/−1); the same 18 gains and 6 losses as
the preview, cell for cell; every pass-arm answer verified (24 at the
checker's budget on each arm), no failed row, no contradiction. That is
the decision evidence for step one of the symmetry pass (plan §8: the
400-cell run, per family, every answer checked); the 2026 holdout stays
unspent until the other passes are in.

**Combined passes on the 400 cells (2026-10-09;
`log/abtest-struct-both-400-2026-10-09-06-25-50`,
`log/struct-both-400-report-2026-10-09.txt`; `--structparity=1
--structsym=1` against the same frozen binary with both off, the
setting of the runs of record).** Solved **298 v 282 (+22 / −6)**, wall
PAR-2 0.893×, tick PAR-2 0.882×; train 216 v 206 (+15/−5), validation
75 v 70 (+6/−1). The 22 gains are exactly the union of the two passes'
gains (the 18 of the symmetry run and the 4 Tseitin cells), all with
checked proofs; the 6 losses are the same SAT cells as in the symmetry
run. Every pass-arm answer verified (22 at the checker's budget, 24 on
stock), no failed row, no contradiction. One cell, `st_890_86_9_572`,
shows rc-15 on both arms: a debugging `kill` of this session hit the
sweep's two processes for it (stock never solves that cell, so the
counts stand). Where both arms solve in 10 s or more the wall ratio's
median is 1.015 and its p90 1.80: the symmetry pass's detection budget
costs up to about 15 s on cells it does not crack, and fixed literals
move the search on others.

**Holdout (2026-10-10; `log/abtest-struct-holdout-2026-2026-10-09-15-51-57`,
`log/struct-holdout-2026-report-2026-10-09.txt`).** The single
consultation of `sat-comp-2026` for the candidate set `--structsym=1
--structparity=1` (plan §8): both passes against the same frozen binary
with both off, 3600 s, 14 GB, 32 shared pinned cores, one seed, every
answer through the gate; nothing in this project had seen these 400
cells. Solved **212 v 203 (+16 / −7)**, wall PAR-2 0.961×, tick PAR-2
0.959×; no failed row, no contradiction, every pass-arm answer verified
(3 at the checker's budget on each arm). Ten of the gains are the
structural kind, UNSAT in seconds on families the 2025 suite does not
have: `chnl` 4 of 6 (channel routing, 2-6 s), `php_sudoku` 2 (0 s),
`DSC125` (12 s), `exam_75_70` (19 s), plus `clqcl_25_7_6` (10 s) and
`rphp_p25_r25` (1 s), which share generators with 2025. The other six
gains and all seven losses are satisfiable cells solved late by one
arm or the other (450-3600 s), the trajectory luck of the 2025 runs
again, netting −1 on the SAT side. So the structure classes transfer:
what the passes decide, they decide on unseen families, and what they
cost is the same pattern as before. Cost noted for a follow-up: on tiny
cells (the `medium` family, stock 0.4-1.1 s) the symmetry pass spends
its whole detection budget, 14-27 s, and finds nothing; the budget
should be capped relative to the formula's size. Compare solver 12's
holdout (296 v 292 on 2025, 160 v 197 on 2026): this candidate set's
2025 gain (+22/−6) carries to 2026 (+16/−7) in kind and in size.

**Known cost after the promotion (2026-10-10; bead
SAT-playground-1v2.8).** Six 2026 cells of the `Medium-result` family
(3,896 variables, 2.9M binary clauses) take 13-27 s against 0.4-1.1 s
stock: the symmetry pass's first six anchors each cost a full
refinement over the 2.9M clauses (160M work each), find no matrix, and
exhaust the budget. A budget relative to the formula's size does not
separate them from the gains: `clqcl_30`, `cliquecoloring_n26` and
`clqcl_25` need 990-1000M of the 1000M budget on far smaller formulas
(the pass work of every structural gain, 2025 and 2026, measured
2026-10-10: php_sudoku 7-10M, the Tseitin cells 0.02-96M, RoundRobin
69-246M, rphp 238-502M, chnl 287-688M, DSC125 706M, exam 970M). The
cost was larger than the tiny cells showed: on 2025, 62 of the 275
cells both arms solve had the pass spend its whole budget for nothing,
+7,185 s of wall against stock, and single cells went from 8.6 s to
116 s (`case19`), 5.9 s to 91 s (`veer_axi`), 2.3 s to 54 s
(`6s299b685`). A stock probe before the pass would have recovered
about 430 s of that; the refinement below recovered most of it.

**Split-driven refinement (2026-10-10;
`log/abtest-struct-refine-400-2026-10-10-14-27-48`,
`log/struct-refine-400-report-2026-10-10.txt`; bead
SAT-playground-1v2.8).** The symmetry pass's colour refinement was the
naive kind: every round re-hashed every clause and every literal, and
ran until the class count stopped changing, so individualizing one
anchor in a circuit cost 20-36 rounds of the whole formula (184M work
on `veer_axi` for a split of 60 classes). Now a round re-hashes only
the clauses with a literal whose class split and the literals with a
clause whose class split (or whose negation split), labels are stable
(a class keeps its label until it splits; fresh labels in hash order,
so they stay canonical and comparable across the refinements of two
anchors), and the base colouring is computed once per detection. Two
more cuts: a class whose negation class was individualized is skipped
(its candidate sets are the negations), and a candidate's rows are
checked after the first six anchor refinements, not after all of them.
Same fixed point; on the gains the old refinement finished, the derived
clauses are byte-identical (19 of 31) or the same count in another
order; where the old budget cut it off, detection now finishes.
Paired run against the promoted binary, both with the shipped
defaults, 1800 s, every answer through the gate: **299 v 297 (+3 /
−1)**, wall PAR-2 0.973×, tick PAR-2 0.978×, validation 76 v 75, no
contradiction. Gains `clqcl_50_6_5` (3 s) and `clqcl_40_6_5` (42 s),
two of the clqcl cells beyond 30 vertices the step-one detector could
not afford, and one late SAT cell; the loss is `booth_dadda`, UNSAT at
1785 s on the old arm. `clqcl_30` goes from 1015 s to 1.8 s,
`cliquecoloring_n26` from 6.8 s to 1.2 s (W 1010M to 167M). Against
stock: full-budget cells among the both-solved 62 → 31, their wall
excess +7,185 → +4,723 s, the cells stock solves under 10 s cost +416 →
+76 s (p90 +34 → +3 s), `case19` 116 → 45 s, `veer_axi` 91 → 20 s,
`6s299b685` 54 → 5 s, `x-epic` 42 → 1 s; where both arms solve, the
wall ratio new/old has median 0.98, p10 0.63, p90 1.04. What stays
expensive is a dense formula (the `Medium-result` cells, 2.9M binary
clauses): one anchor there genuinely splits tens of thousands of
classes, so a refinement touches the whole formula, and a candidate
with many rows costs one refinement per row until the budget ends;
the budget (1000M; the gains now need at most 420M) is the remaining
knob, to be measured on its own.

**Symmetry step two (2026-10-11;
`log/abtest-struct-sym2-400-2026-10-11-12-23-05`,
`log/struct-sym2-400-report-2026-10-11.txt`; bead SAT-playground-1v2.3).**
The targets were the seven unsolved 2025 cells the census had shown
symmetry fixing decides: the four `MVRoundRobin` cells and three `clqcl`
cells. MVRoundRobin needed three things, each general (the method text
above): a cell of anchor images sorted into rows by a second anchor
(its side flip puts both sides of a game on another day into one cell);
pairing by any shared clause and the links to the anchors, with a
one-to-one guess among entries refinement cannot tell apart and a retry
with refinement first when a guessed matrix fails; and the transport
fixing, which derives "game k plays on a day ≤ k" as a clause where no
clause lies in one column. With those ten clauses the stock search
refutes `MVRoundRobin_n14_d10_v2` in 0.05 s. Paired run against the
shipped binary (451cd66), both with the shipped defaults, 1800 s, every
answer through the gate: **302 v 300 (+4 / −2)**, wall PAR-2 0.981×,
tick PAR-2 0.969×; the four gains are exactly the four MVRoundRobin
cells (3, 4, 13 and 30 s; old arm timeouts), the two losses UNSAT cells
at 1742 s on the old arm (`booth_dadda`, `oddball_29`); no structural
gain of the runs of record lost; validation 75 v 75 (wall 1.011×, tick
1.019×, no cell changed). Where both arms solve, the wall ratio has
median 1.00 and p90 1.31, the tail being the `oski` family's search
swinging both ways (±6-19 G work); on the 78 cells the old arm solves
under 10 s the new one costs +10 s in all (p90 +0.1 s), the price of the
extra candidates (RoundRobin_n16_d13's detection 10M → 23M). The three
`clqcl` cells were not covered by this step: the pass fixed 73 literals
on `clqcl_30_11_10`, search did not finish, and a 20× budget changed
nothing. The reading written here on 2026-10-11, that they need the
row-and-column (double-lex) breaking Satsuma used, was wrong; the cause
was a defect in how a matrix keeps its rows (next paragraph).

**The clqcl cells (2026-10-12;
`log/abtest-struct-clqcl-400-2026-10-12-09-36-43`,
`log/struct-clqcl-400-report-2026-10-12.txt`; bead SAT-playground-1v2.3).**
A clique-colouring formula has a matrix whose rows are the vertices,
and two vertex rows share an entry, their edge variable. The columns of
a matrix are aligned to its first row: the shared entry of rows i and j
sits, in row j, in the column of the first row's entry it is the image
of. After the first vertex was fixed, that row was no longer symmetric,
and the rows were re-checked against the second row with the old
alignment; no consistent swap exists that way, so the whole vertex
matrix was dropped after one fixing, and only the small position and
colour matrices went on. Now the other rows are re-aligned as images of
the new reference under the exchange composed through the first row,
and verified as before. With the vertex matrix kept, the pass alone
refutes `clqcl_30_9_8` and `clqcl_30_11_10` (380 and 497 derived
clauses, 1-2 s). `clqcl_100_6_5` then needed only budget: its 100-row
candidate costs 1.6G of work to detect, over the 1G limit. A second
tier, `--structmore` (default 2M K): when a matrix has been verified,
the detection in progress and its fixings may run to that limit, once;
a later detection starts only within `structticks`, so a cell with no
matrix pays nothing more, and the repeated detections of a cell that
has one (`MVRoundRobin_n16_d10_v3`, 27 s) are not stretched. Paired run
against the shipped binary (241b2bf), both with the shipped defaults,
1800 s, every answer through the gate: **306 v 303 (+4 / −1)**, wall
PAR-2 0.977×, tick PAR-2 0.976×, validation 76 v 75. Gains: the three
cells (`clqcl_30_9_8` 1.3 s, `clqcl_30_11_10` 2.3 s, `clqcl_100_6_5`
14.8 s; timeouts before) and one satisfiable cell at 1753 s; the loss
is `booth_dadda`, UNSAT at 1753 s on the old arm, the cell that has
gone either way in each of the last three runs. The whole family costs
less: `clqcl_30_7_6` 7.8 → 0.7 s (W 554M → 92M), `clqcl_50` 880M →
197M, `clqcl_40` 523M → 129M, `cliquecoloring_n26` 271M → 64M. Every
other structural gain has the same work as before (MVRoundRobin,
RoundRobin, rphp, Tseitin, php: identical W). Where both arms solve,
the wall ratio has median 1.000, p90 1.15, p10 0.93, and the work summed
over those cells is 0.9993×. With this the default configuration solves
every one of the 26 cells that stock leaves unsolved and the census
found decided by symmetry breaking (`benchmarks/rl/structure_2025.tsv`,
column `sym_decided`: round robin 12, clqcl 6, php 4, clique-coloring
3, Tseitin 1; checked against this run's results). Two cells end in
an allocation failure under the 14 GB limit of these runs
(`pj2002_k500`, `17.normalised`) on every arm, the passes off included
(`log/abtest-struct-both-400-2026-10-09-06-25-50/stock`); they count as
unsolved.

**The parity pass, step one (2026-10-08).** An XOR constraint over k
variables is the 2^(k-1) clauses that forbid one parity (k up to 8, the
census detector). When a set of such constraints is closed (its
variables occur nowhere else) and every variable sits in exactly two
constraints, the constraints are the vertices and the variables the
edges of a graph, and flipping every variable on a cycle keeps the
parity at every vertex: a symmetry that maps each flipped variable to
its own negation, so any of them can be fixed by one redundance step
with the flip as witness. The pass takes a breadth-first spanning forest
(short cycles, small witnesses), fixes every non-tree edge true, then
unit propagation settles the tree from the leaves up and an odd total
parity falsifies the root. This is Tseitin's formula on any graph and
the symmetry pass cannot see it (no two literals are exchanged).
Measured with the gate: all five Tseitin cells, `tseitin_n188_d3`
0.0 s, `grid_n12` 0.0 s, `grid_n250` 2 s, `d3_n100000` 1.5 s (proof 31
MB, checked in 15 s), `grid_n400` 13 s (proof 1.2 GB, 160k redundance
steps, checked in 293 s). **Run of record on the 400 cells**
(2026-10-09; `log/abtest-struct-par-400-2026-10-08-20-54-17`,
`log/struct-par-400-report-2026-10-08.txt`; the setting of the
symmetry run, binary of commit `2b61e4f`): **286 v 282 (+4 / −0)**,
wall PAR-2 0.971×, tick PAR-2 0.985×; the four gains are exactly the
four unsolved Tseitin cells, all with checked proofs; no loss; on the
225 cells both arms solve in 10 s or more the wall ratio's median is
1.001 (the pass costs nothing where it finds nothing); every pass-arm
answer verified (23 at the checker's budget on each arm), no
contradiction. Not covered: XOR systems that mix with other clauses
(par32, grs, 16_2, xor-op), which need general elimination with a
certified sum (bead S.1 step two: Gocht and Nordström's PB
certification, or DRAT with extension variables).

**The counting pass, step one (2026-10-09).** A clause of two or more
literals demands at least one of them; a set of literals pairwise
forbidden by binary clauses allows at most one. For every literal of a
demand clause the pass takes the largest such group through it that
holds no other literal of that clause (a hole for a pigeon, the other
square of a domino), two-colours the demand clauses by shared literals,
and in each colour looks for a set of demand clauses that reaches fewer
groups than its size (an alternating search from a clause a maximum
matching cannot place). The refutation is pure cutting planes over the
input clause ids: each group's at-most-one constraint is built one
literal at a time from its pair clauses (sum, multiply, divide), then
the demand clauses and the group constraints are added; every literal
cancels against its negation and what is left cannot reach the
right-hand side. Refutes the pigeonhole formulas with a proof the gate
checks (php12 0.0 s). The rook cells (decoded 2026-10-14, scratch
decoder in the session notes): an n×n board (n = 51, 56, 52) with
pairwise at-most-one clauses on every row and column, and a decision
diagram of one-directional nodes `n -> (x ? hi : lo)` (135-179K of them,
one board variable per level, rows visited mostly in descending order
but interleaved) whose root, a unit clause, admits exactly n+1 true
board variables; `rook-52-0-1` also asks for a rook in every column.
n+1 rooks on n rows: refuted by counting. The proof is the hard part.
Each node's meaning, "n implies at least q of the board variables
below it are true", has to be derived from its two children, and that
is a case split on the node's variable, which cutting planes does in
about q steps per node (one step lowers the variable's coefficient by
one; a sum of the two child constraints rounded once cannot do it for
q ≥ 3, and an MIR cut of it gains no more), about 3.5M
`pol` lines and 4.6G term operations for VeriPB on `rook-51-0-0`; a
proof by contradiction (`pbc`) would be three steps but must spell out
each node's constraint, about 175M terms. The one-line derivation noted
in bead S.3 on 2026-10-13 assumed a fact (a node implies its weaker
child) that the one-directional encoding does not give. No cell in
the 2026 suite is a rook, ramsey or chess puzzle, so this is parked
until the owner decides it is worth building for three 2025 cells.
What was written here on 2026-10-09,
that `mchess_20` has at-most-one groups in sequential-counter form and
stays open, was wrong: the cell is plain pairwise (398 demand clauses,
2154 negative binaries, no auxiliary variable), and the pass refutes it
in 0.01 s (81K work) with a proof the gate checks. It had not been run
again after the review fixes of that week.

**The counting pass, placement (2026-10-12).** The pass ran between the
parity and the symmetry pass on their shared limit. On a circuit that
is wrong twice: every gate looks like a demand clause with groups, the
pass spends whatever it is given (`veer_axi`: the whole 1G, nothing
found), and the symmetry pass then has nothing left (its 65,950 units
on that cell were lost). It now runs last, when the other passes did
not refute, within a limit of its own (`--structcountticks`, 20M,
counted from where they stopped): it refutes from the input clauses
alone, so what the earlier passes derived does not matter to it, and
every measured path of theirs is unchanged. It also refutes when no
proof file is asked for (it used to decline then; the other passes
never did). Alone, the pass also refutes `harder-fphp` (0.9M), the
plain RoundRobin cells (5.6M on n15) and `MVRoundRobin_n14` (817M, over
the limit; the symmetry pass has those first anyway).
Measured in the three-arm run described under the parity pass's step
two (count against flips: the same binary with and without this pass):
**309 v 305 (+4 / −0)**: `mchess_20` in 0.1 s (2M work), and the three
near-timeout cells that the flips arm ran slowly. Against the shipped
binary 309 v 306 (+3 / −0), wall PAR-2 0.977×, tick PAR-2 0.964×,
every answer checked. Where the pass finds nothing it costs its limit:
+14.6M work at the median, +20M at most. Promoted: `structcount` is on
by default since 2026-10-13; the new default reproduces the count arm's
work to the unit on the seven cells checked (`mchess_20` 2310197,
`xor_op_n36_d3` 55341145, `veer_axi` 243393370, ...).

**The parity pass, step two: flip symmetries (2026-10-12; bead
SAT-playground-1v2.2).** Negating every variable of a set S maps the
formula onto itself when, for every set V of variables that clauses are
written over, the sign patterns of those clauses are closed under
flipping S inside V. The flips that keep one such group of patterns are
a linear space over GF(2), so the sets S are the solutions of a sparse
linear system with one unknown per variable: a variable in a clause
that is alone over its variable set cannot be flipped; the two
variables of `a xor b` substituted for a variable must be flipped
together; a Tseitin vertex must be met an even number of times. The
pass groups the clauses by variable set (a sort by a hash of the set;
only clauses holding a variable that occurs equally often in both
signs), finds each group's flips (the candidates are the differences
from one pattern; when more than half of all patterns are present, the
absent ones are looked at instead), turns them into equations, and
eliminates, shortest equation first. That leaves free variables, and
for each free variable f one flip that holds f and no other free
variable; `red f` with that flip as witness fixes f (to true; either
value is sound), and since the flip leaves the other free variables
alone, every one of them is fixed. The algebra only proposes: each flip is checked against every
group it touches before it is logged. Variables an earlier step or
pass already fixed are left out; the units are propagated by the
symmetry pass or the solver. A formula whose variables were each
replaced by the XOR of two fresh ones (`xor_op_n36_d3`: 1260 blocks,
all clauses of 6 literals, no XOR constraint for step one to see)
becomes the formula it was built from, the sparse ordering principle,
which search refutes in 0.2 s. Step one is unchanged and runs first;
what it refutes never reaches step two.
Measured in a three-arm run on the 400 cells (2026-10-12/13;
`log/abtest-struct-flips-400-2026-10-12-21-46-59`, reports
`log/struct-flips-400-report-2026-10-12.txt` and
`log/struct-flips-400-report-count-v-flips-2026-10-12.txt`; base = the
shipped binary 7a76a71, flips = the candidate's defaults, count = the
same with the counting pass on; 1800 s, every answer through the gate,
no contradiction): flips against base **305 v 306 (+2 / −3)**, wall
PAR-2 0.995×, tick PAR-2 0.990×. The gains are `xor_op_n36_d3` (0.6 s)
and `xor_op_n40_d3` (0.9 s); `xor_op_n38_d3` goes from 1492 s to 1 s.
The three losses are not the step's. `bp4_BC012_CSO_FPBEQ_FPBLE_ZR`
(1750 s on base), `16_16_and_wallace_origin_and_default_mapped` (1730 s)
and `ncc_none_2_17_4_3` (1799.6 s) need the same work in every arm: the
count arm, the same binary and the same search, solves all three at
base's work plus the passes' 1-30M. This arm's processes ran them at
76%, 83% and 99% of base's speed and were cut at 1800 s. Speed between
arms that run side by side differs that much per cell (cells all arms
solve at equal work: wall ratio p10 0.76, p90 1.10; work reached on the
89 cells no arm solves: median 1.001), so cells that need 1716-1800 s
fall on either side. The step fires on 25 cells: the three xor_op
cells, twelve `oski` cells (69-77 flips each), four hwmc step and
transition cells (53-5612), two `6s299b685` cells, `SAT_dat.k100`
(8611 flips), `summle`, and the two `ramsey_4_4` cells (one flip of all
153 and 171 variables, the swap of the two colours; still unsolved). The
21 of them that base solves all stay solved. Their search moves: the
twelve oski cells took 15% more work in sum, between 0.77× and 1.88×
per cell. The run fixed the free variables to false. Fixed to true
instead (the same cells one per core on an idle host, deterministic
work, `log/struct-flips-polarity-2026-10-13.tsv`) the twelve took
1.00× base's work in sum, between 0.48× and 1.88× per cell; true was
the lighter value on 8 of the 12 and the heavier on 4, a geometric
mean of 0.86× false's work with a standard error that leaves it short
of significant (t about 1.4): mostly this family moving under any
change, with a lean toward true. True is also the value search tries
first by default and what step one fixes its edges to, so the step
ships fixing to true. The value can only matter where a flip is found,
so the 400-cell picture with true is the count arm's on 375 cells (the
shipped binary reproduces that arm's work to the unit where no flip
fires) plus these runs on the 23 fire cells that are not ramsey: all
21 that base solves are solved (most work `oski15a01b15s` 34.7G,
against 39.9G with false), the three xor_op cells in under a second,
proofs through the gate. Where the step finds nothing it costs its
scans: the work ratio where both arms solve has median 1.0001. Review:
seven Codex rounds; five found one thing each, all of one kind (work
done past the work limit: a check after an early `continue`, a sort
charged after it ran, and the like), after which the step's accounting
was rewritten under one rule (no piece of work starts unless the limit
covers it) and two rounds were clean. A fuzz of 3 × 600 small formulas
with planted parity structure against the stock path, models checked
and every proof through the gate, had no mismatch.

**The symmetry pass, step one.** Colour refinement on the clause-literal
graph (literals, because competition cells carry flipped polarities and
a symmetry may map x to not-y) gives classes of literals that look alike.
Individualizing one literal of a class and refining again splits the
class into candidate anchor sets, one anchor per row; the row of an
anchor is the set of literals whose colour under its individualization
differs from their colour under the other anchors' (with four or more
other anchors, a literal shared by two rows, an edge variable, is allowed
to agree with one of them). Cells correspond across rows by colour;
entries inside a cell are paired by a binary clause between the rows
when that is unique, else by the entries sharing any clause with it,
told apart by their clauses with the anchors (rows 0 and i exchanged),
else by one more refinement per entry of row 0 with anchor 0 and the
entry individualized. Every row swap is then applied to every clause
(the input's and the clauses this pass derived) and checked against the
clause set, so the refinement only guesses and a wrong symmetry cannot
get through. Candidate anchor sets (step two, 2026-10-11): a second
symmetry can put several literals of one row into the cell of the
anchor's images (a round robin's side flip puts both sides of the same
game on another day there); the cell holds (R−1)·v members, v per row,
and with the anchor and a member individualized that member's v−1
row-mates lie in cells of at most v−1 while the other rows' members lie
in cells of a multiple of R−2, so the factorization that fits sorts the
cell into rows and the candidate takes one member per row. Fixing (step
two): over every matrix, the active clause whose *transported* form is
shortest, where transporting carries every entry of the clause onto the
lowest row it touches (that row's entries in the clause's columns, plus
the literals outside the matrix); a clause in one column, one entry per
row, transports to a unit, a game's "plays on some day on some side"
clause to "plays on day 1" and later "on day 1 or 2", and so on. The
derivation is the same for both: a `red` step per moved entry with the
row exchange as witness, `rup` for the transported clause, and the
steps deleted (`del id`) so no later witness has to map them. A unit is
assigned and propagated; a longer clause joins the symmetry checks, so
the rows it breaks drop out at the next verification. A transported
form an earlier derived clause subsumes is skipped. Every matrix keeps
the rows that still swap (verified again): the columns are aligned to
the matrix's first row, and when that row is no longer symmetric the
other rows are re-aligned as images of the new reference under the
exchange composed through the first row (2026-10-12; without this the
vertex rows of a clique colouring, which share their edge variables,
were dropped after the first vertex was fixed). When nothing
transports, detection runs once more. Measured 2026-10-07 on
the cells the census had marked, every answer through the gate: php 4
of 4 (0.1-5 s), clique-coloring 4 of 4 (1-7 s), the eight plain
RoundRobin cells (0.5-1.5 s), homer11 (0.1 s), rphp 3 of 3. Not yet:
the six clqcl cells (the 100-vertex ones spend the budget in detection;
a refinement that only recolours the touched part of the partition is
the follow-up), the four MVRoundRobin cells (no interchangeable rows
found; Satsuma's proofs for them were rejected by VeriPB too), ramsey
(Johnson structure, not rows), the Tseitin cells (literal-flip
symmetries, which the parity pass covers instead).

## The local-search share (2026-10-13; bead `SAT-playground-1v2.9`)

Not in kissat. Two options, both off by default; with both at 0 the walk
is kissat's own (the binary reproduces the shipped one's work to the
unit, and `tools/parity.py` stays exact):

| option | what it does |
|---|---|
| `--walkshare=N` | local search may use N per mille of the work clock: a walk runs past kissat's own limit until the walks so far have used that share. A walk step is counted as 1/8 tick into `statistics.ticks`, so tick PAR-2 sees the walking; the tick limit ends a walk |
| `--walkstall=N` | a walk ends when, since its best assignment, it has made N times the flips it took to reach it (never before 65,536 flips) |

**Why.** kissat walks at some rephases and gives a walk 5% of the
search ticks since the last inprocessing, in walk steps that cost about
an eighth of a tick: half a million steps, a few times in a run. Of the
94 cells the default configuration leaves unsolved, the same walker
given real time solves eight satisfiable ones (probe of 2026-10-12, 300
s each, every model checked,
`log/walkshare-proto-2026-10-12/probe-unsolved-300s.tsv`): with
`--walkeffort=100000` `Ptn-7824-b19` in 0.8 s (one walk of 154M steps),
`mp1-Nb7T45` 12 s, `lockchart-group2` L18 24 s, `ER_400_20_7` apx_1 86 s
and apx_2 246 s, `reconf10_70_queen14_2` 162 s, `oddball_57_5` 287 s;
with `--walkeffort=1000000` lockchart L19_3 28 s. (`walkinitially` is
declared in kissat 4.0.4 and never read, there or here.) As constants
those settings make the solver a local-search solver and are no
candidate; the share gives the walker a bounded part of every run.

**First results** (four free cores during a sweep, so a solve counts and
a wall time does not): with `--walkshare=100` seven of the eight solve
within 600 s (`Ptn-7824-b19` 3 s, `mp1-Nb7T45` 75 s, `ER_400` 116 and
253 s, `oddball_57_5` 291 s, `reconf10_70` 332 s, lockchart L19_3 518
s). Over the 27 satisfiable-looking unsolved cells at 1800 s,
`--walkshare=100 --walkstall=16` solves 8 and `--walkshare=200
--walkstall=16` 6, and only two cells are in both lists: which cell
falls is partly luck. Cost on 40 solved cells in deterministic work,
against the same binary with the share off: 24 unsatisfiable cells sum
1.059× (median 1.010) with share 100 and stall 16, 1.105× without the
stall rule; 16 satisfiable cells 1.195×; the cells far from 1 (one at
2.9×) spend 1-8% of their work walking, so they are searches that took
another path, not walk cost
(`log/walkshare-proto-2026-10-12/screen-results.tsv`,
`screen2-results.tsv`).

**What the long walk hands back is the point.** A walk ends by making
its best assignment the saved phases, and the search goes on from
there. A variant that kept the long walk for finding a model only (at
the end it handed back what the walk had at kissat's own limit, so the
search was the stock search to the tick) solved 2 of 12 target cells in
1800 s on an idle host (`Ptn-7824-b19` 7 s, `ER_400` apx_2 1777 s) where
the plain share had solved 8 of the same 12. Rejected
(`walk-v2-side-effect-free.patch`, `screen3-results.tsv` in the same
directory): the gain is the exchange between the two searches, not a
lucky walk, and it comes with every cell's search taking another path.

**Paired run** (2026-10-13; `log/abtest-struct-walk-400-2026-10-13-11-35-06`,
`log/struct-walk-400-report-2026-10-13.txt`; base = the shipped binary
4ac5870, 400 cells, 1800 s, seed 0, every answer through the gate, no
contradiction):

| arm | solved | SAT / UNSAT | +/− v base | wall PAR-2 | tick PAR-2 | validation |
|---|---:|---:|---:|---:|---:|---:|
| base | 311 | 136 / 175 | | 1.000× | 1.000× | 77 |
| `--walkshare=100 --walkstall=16` | **322** | 148 / 174 | **+17 / −6** | 0.926× | 0.947× | 80 |
| `--walkshare=50 --walkstall=16` | 317 | 145 / 172 | +17 / −11 | 0.946× | 0.949× | 81 |

With the 10% share, 16 of the 17 gains are satisfiable cells: five of
the eight probe cells (`Ptn-7824-b19` 14 s, `oddball_57_5` 449 s,
`mp1-Nb7T45` 967 s, lockchart L18 1031 s and L19_3 1057 s) and eleven
others the probe had not named (`HCP-529-420` 188 s, `oddball_53_5`,
`sum_of_3_cubes_37_bits_87`, `bp4_TCO_IXA_FPBLE_ZR`, the `fermat`
factoring cell, two `lockchart-group1` cells, a timetable, `sted2`,
`at-least-two-vmpc_28`, `bp4_BC012_AM`); the 17th is an unsatisfiable
cell at 1749 s. Of the six losses, the two unsatisfiable ones and one
satisfiable one are near-timeout cells (base 1773-1793 s; the arm was
cut below base's work, so slow on the wall), and three satisfiable ones
took a longer path (cut at 1.6-9.9× base's work: `SC25_Timetable_C_406`,
`SC25_Timetable_C_492`, `oddball_56_5`). Where both solve, satisfiable
cells: wall ratio median 0.90 (p10 0.42, p90 4.06), work summed 0.97×;
unsatisfiable cells: wall median 1.01, p90 1.50, work summed 1.06× (the
walking, mostly). The 5% share gains as many and loses more (four of its
eleven losses unsatisfiable, all near-timeout); nine cells are gained by
both settings.

**The replicate does not agree** (2026-10-14; seed 1 for both arms,
`log/abtest-struct-walk-400-seed1-2026-10-13-23-29-21`,
`log/struct-walk-400-seed1-report-2026-10-14.txt`): the 10% share
against base **311 v 312 (+13 / −14)**, wall PAR-2 1.007×, tick PAR-2
1.023×, every answer checked. Twelve of the 14 losses are satisfiable,
eight of them longer paths (cut at 1.8-14× base's work); only three
cells are gained in both seeds (lockchart L18, `sum_of_3_cubes_37`,
`bp4_TCO_IXA_FPBLE_ZR`), and none is lost in both. The reason is in the
base arms: the shipped binary itself, seed 0 against seed 1, solves 311
v 312 but differs on 31 cells, 27 of them satisfiable; 23 of the 31 are
solved by one seed well before the limit (median 578 s) and not at all
by the other, so this is the seed, not wall noise. With seed 1 it
solves four of the eight probe cells on its own (`Ptn-7824-b19`,
`mp1-Nb7T45`, `ER_400` apx_2, `reconf10_70_queen14_2`): those were cells
seed 0 happened to miss, not cells kissat cannot reach. Over both seeds
the stock binary solves 327 cells at least once and the share 329; 13
cells only the share reaches (oddball tto 52, 53, 57, the lockchart
group-2 L18 and L19 and group-1 L200, `at-least-two-vmpc_28`, ...) and
11 only stock; over the two seeds the share solves 633 v 623, +5 a
seed, which is inside what the seed alone moves. So the share changes
which satisfiable cells fall more than how many, it costs 2-6% work on
unsatisfiable cells, and most of its seed-0 lead was the luck of one
seed. **Not promoted**; the options
stay in the tree, off. What would move this is a way to spend
satisfiable-side luck better (more seeds per run, or a schedule that
diversifies), measured over at least two seeds. Tests:
`tests/walkshare.rs` (the share off is the stock trajectory; on, a walk
is longer and its steps are in the clock exactly; a work limit set half
way into a walk ends the run there; a stalled walk ends early). Review:
three Codex rounds (the stall rule's floor counted kissat's flip
counter, which advances twice a step; the limit test stopped before any
walk; the stall interval was counted from the walk's start), then clean.

## The parity engine (2026-10-15; bead `SAT-playground-1v2.7`)

Not in kissat. XOR reasoning inside search (`src/gauss.rs`), the way
CryptoMiniSat does it (Han and Jiang, "When Boolean Satisfiability Meets
Gaussian Elimination in a Simplex Way", 2012). An XOR constraint says an
odd (or even) number of its variables are true. Clauses handle a long
XOR badly: a clause propagates only when one literal is left, and the
sum of two XORs has no short clause form. The parity pass settles closed
XOR systems before search; this engine works on XORs mixed with other
clauses, during search. Options (the engine on by default since 2026-10-18, after the paired
run with lazy reasons below; a promoted default is exactly the measured
configuration):

| option | what it does |
|---|---|
| `--gauss=1` | the engine; **default on since 2026-10-18** (the paired run with lazy reasons, below); `--gauss=0` is the stock path |
| `--gaussyield=N` | at every window of 67M ticks of its work, the engine's propagations and conflicts per million ticks since the build must reach N, else it stops for the run (default 20; par32-2 runs at about 560, a cell where it only burns work under 1; 0 for no guard) |
| `--gausslimit=N` | the matrix may hold N million bits, rows times columns, plus the proof's table of row sets (default 256); a larger system leaves the engine off |
| `--gausseffort=N` | the engine may use N per mille of the search ticks (plus a start allowance); past that it stops for the run (default 0, no cap: a cell the engine solves takes more engine work than search work) |

**How it works.** The XOR constraints the parity pass extracts from the
input (groups of 2^(k-1) clauses over one variable set, k up to 8) are
rows of a matrix over GF(2): one bit per variable plus the parity. The
matrix is built when the search starts, after preprocessing, which runs
as it would without the engine: a variable preprocessing removed
(eliminated or substituted) is pivoted first and its row dropped, so
the rows left are over the solver's variables and still sums of input
XORs; a variable fixed at level zero is a column assigned for good. (An
earlier build froze every XOR variable before preprocessing; on
`jkkk-one-one`, 5,595 frozen variables crippled preprocessing before
the engine had run.) A variable that is in at most two XORs, in no
other clause of the input, and in no clause of the solver outside its
XORs' variable sets is a helper of an encoding (the chain variables of
a long XOR); the reduction pivots on helpers too and drops their rows,
so the matrix holds the long XORs the chains encode. The third
condition is checked on the solver's clauses at the build: a chain's
end that stands in for a substituted error bit carries the error bit's
clauses and is no helper (judged by counts instead, such a variable
was dropped and a 65-sample case lost every row). Every other variable,
including one that occurs only in XOR clauses, keeps its row: a
variable the solver decides on must be in the matrix for the
propagation to be complete (dropping them all cost par32-2 most of the
engine's effect). The matrix is kept
in reduced row echelon form: each row has a basic variable that occurs
in no other row. A row watches its basic variable and one other
unassigned one. After clause propagation reaches a fixpoint the engine
reads the new part of the trail, all of it before it inspects a row (a
row judged on part of the trail once assigned a variable the clauses
had already assigned, which overran the trail). A row whose other watch
was assigned moves it, or propagates its basic variable, or is
satisfied or a conflict; a row whose basic variable was assigned takes a
new basic variable among its unassigned ones and adds itself to every
other row holding it (the Jordan step), which keeps the propagation
complete. Backtracking undoes nothing in the matrix; the engine's
per-variable bits are cleared as the solver unassigns, and after
inprocessing (probe, eliminate) it re-reads every row. A propagation or
conflict puts the row's clause (every literal false but the propagated
one) into the arena as a redundant clause, the reason or the conflict
for analyze; a row of one variable is a unit, assigned at level zero. A
zero row of parity one at the reduction refutes the formula.

**What the solver gives up while the engine is on.** From the build on
its variables are frozen (never eliminated, never substituted), so the
columns stay live, and there is no compaction (which renumbers
variables; the first bug found: fixed variables merged into one
column). Its work goes into the work clock as `gauss_ticks`. **Guards:**
the yield guard above (on `Carry_Bits_Fast_19` the engine made one
propagation in 491M ticks; on `jkkk-one-one` 22K in 34G ticks and the
run took 144 s against 4 s; both stop within 67M ticks now), the matrix
size cap, and the effort cap as an option. A cell the engine stops on
early still takes another path afterwards (the clauses it added, the
few propagations), which on a satisfiable cell is luck either way
(`jkkk-one-one`: 4, 21 and 277 s for the stock solver's seeds 0-2).

**Measured so far** (X.1, no proof yet): `par32-2` (parity learning,
3176 variables; never solved by the stock solver, CryptoMiniSat 5.8
takes 7 s) is **satisfiable in 0.7 s** with 4,518 conflicts, the model
checked against every clause; the matrix is 513 rows after 2,111 helper
rows are dropped. `16_2` (a cipher key search) is not an XOR cell:
neither this engine nor CryptoMiniSat solves it in 15 minutes. Small
parity-learning formulas (`tests/gauss.rs`) fall in 1-20 conflicts
where the stock solver passes 20,000. A fixed basis of derived XORs
written as chain clauses, without an engine, did not solve par32-2 in
15 minutes: the re-pivoting in search is what matters. Fuzz: 4,600
random formulas mixing XORs of 2-5 variables with clauses (up to 160
variables), gauss on against off, every model checked, no mismatch.
Review: six Codex rounds (the forward SCC walk of substitution had to
skip frozen variables too; the engine stays off when a proof is
written until X.2; the reduction and the batch poll the work limit and
charge their scans; a row with no unassigned variable to watch keeps a
watch on the one assigned at the highest level, so a backtrack frees it
with any other; assignments kissat keeps across a chronological
backtrack are read again), the sixth clean.

**Paired run** (2026-10-16; `log/abtest-struct-gauss-400-2026-10-16-01-47-56`,
`log/struct-gauss-400-report-2026-10-16.txt`; one frozen binary, commit
e8b0bad, `--gauss=1` against `--gauss=0` at seeds 0 and 1, 400 cells,
1800 s, every answer through the gate, no contradiction; the engine
builds a matrix on 107 of the 400 cells, 46 of them over equivalences
only):

| arm | solved | +/− v its base | wall PAR-2 | tick PAR-2 | both seeds |
|---|---:|---:|---:|---:|---:|
| base, seed 0 | 309 | | 1.000× | 1.000× | |
| gauss, seed 0 | 310 | +5 / −4 | 0.997× | 0.982× | |
| base, seed 1 | 311 | | 0.974× | 0.978× | 620 |
| gauss, seed 1 | 311 | +4 / −4 | 0.984× | 0.981× | 621 |

The one gain that holds on both seeds is `par32-2` (12 and 34 s, never
solved by stock). The other gains and losses are satisfiable cells on
other paths (timetables, ER, HCP; `SC25_Timetable_C_496` is gained on
one seed and lost on the other) and two multiplier cells at 1714 and
1740 s, near-timeout noise. One loss repeats: `bivium-39-200`
(unsatisfiable, a cipher, 462 XORs): 707 and 838 s for stock, a
timeout and 1785 s with the engine. `jkkk-one-one` (satisfiable, 4,630
XORs) took 197 and 217 s against 5 and 26 s, inside the stock solver's
own spread across seeds (4, 21, 277 s), but twice on the wrong side.
On the 107 cells the engine touches, where both arms solve, the work
summed is 1.05× and 1.03× (medians 1.02 and 1.00); on the other 293
cells the search is identical (the work differs by the scan for XORs,
0.01% in the median). **Not promoted**: the option stays off. Why bivium
is slower (a diagnostic run at seed 1 on an idle host, no proof
written): the engine reasons there, 2.7M conflicts against 16.0M for
stock, but it makes 16.8M propagations and every one puts a reason
clause of a row's length into the arena, so the search's work per
conflict is five times stock's and the run takes 842 s against 760;
with the proof written, as in the paired run, every one of those
clauses is also a line in the stream, and the run takes 1785 s. So the
losses that are not noise come from eager reason clauses, which
CryptoMiniSat avoids by making a reason only when conflict analysis
asks for it (lazy reasons; what blocks that here is the proof, see
below). What it would take: lazy reasons with a proof that covers
them, no matrix for the 46 cells with equivalences only (kissat's
substitution handles those), and a second cell the engine cracks. The
engine's proofs cost two checker timeouts more than stock's over the
run (24 v 22 at seed 1).

**Lazy reasons** (X.4, 2026-10-17). A propagation of the engine is
recorded as its row: the literal's assignment carries the row index and
a flag (`Assigned::GAUSS`), at the highest level of the row's other
variables, and no clause exists until conflict analysis asks for the
reason. Then the clause is built from the row, proved, put into the
arena and the record points at it. This is sound because a row that
propagated has every variable assigned, and the Jordan step only adds
rows into rows that hold the new, unassigned pivot: the row is the same
when the clause is built, as long as the literal is still assigned (its
level is the highest of the row's, so a backtrack that frees any of
them frees it). Every reader of a reason as a clause (first-UIP
deduction, minimization, shrinking, reason-side bumping) goes through
one helper, which takes either side of the variable: minimization walks
the false literals of a clause. Readers that only protect or remap
reason clauses (reduce's marking, collection) skip the tag. Before the
engine stops for the run every tagged reason on the trail becomes a
clause (its rows would no longer be kept, and compaction may run again),
and so before kissat's analysis of a conflict at level one, which learns
the decision's negation as a unit as soon as a clause holds it, on the
strength of the whole level's propagation having reached a conflict:
the proof checker replays that propagation from clauses, so every
propagation of the engine on the level needs its clause (found by the
gate on a parity-learning formula: a unit with no derivation). Units
(every other variable fixed) and conflicts are built at once. The exit
line counts `lazy` and `materialized`. Measured: par32-2 2-7 s across
seeds (the eager engine 4-8 s); bivium at seed 1 on an idle host, no
proof written: **578 s against 842 s eager and 760 s stock**, with 3.5M
of 27.6M propagations made into clauses (13%) and 146K conflicts
against 2.7M eager and 16.0M stock. Tests: `tests/gauss.rs` (few
clauses made and the gate accepts; the engine stopped mid-search by its
yield guard with tagged reasons on the trail, and the same under
chronological backtracking, through the gate); the proof fuzz, now with
parity-learning formulas and the stop options, 600 formulas on the
release build and 120 on the debug build with its assertions, every
UNSAT through the gate; the whole test suite on both builds.

**Paired run with lazy reasons** (2026-10-17;
`log/abtest-struct-lazy-400-2026-10-17-08-31-04`,
`log/struct-lazy-400-report-2026-10-17.txt`; one frozen binary, commit
762d4c3, `--gauss=1` against `--gauss=0` at seeds 0 and 1, 400 cells,
1800 s, every answer through the gate, no contradiction; the base arms
are the stock trajectory to the unit, identical in work to the eager
run's base arms on every cell both solve):

| arm | solved | +/− v its base | wall PAR-2 | tick PAR-2 | both seeds |
|---|---:|---:|---:|---:|---:|
| base, seed 0 | 308 | | 1.000× | 1.000× | |
| **gauss, seed 0** | 311 | +4 / −1 | 0.981× | 0.969× | |
| base, seed 1 | 311 | | 0.967× | 0.981× | 619 |
| **gauss, seed 1** | 315 | +6 / −2 | 0.950× | 0.943× | 626 |

Two gains hold on both seeds: `par32-2` (11 and 4 s) and
`Circuit_multiplier29` (1372 and 520 s), neither ever solved by stock;
`16_2` (the cipher cell CryptoMiniSat did not solve in 15 minutes)
falls on seed 1 at 1118 s. The other gains are satisfiable cells on
other paths and two multiplier cells at the timeout's edge. **No
unsatisfiable cell is lost on either seed.** The three losses are
satisfiable cells on other paths (two timetables, where the engine
builds a matrix of equivalences and can reason nothing; fermat, where
the yield guard stops it within 0.1 s): coin flips, with the engine
flipping the coin. bivium, the one repeated loss of the eager engine,
is 887 and 889 s against 716 and 848 s for stock (1800 and 1785 s
eager), with its proof written on a loaded host; on an idle host with
no proof it is 578 s against 760. On the 107 cells where the engine
builds a matrix the work where both arms solve is 1.04× summed on both
seeds (medians 1.03 and 1.00); the other 293 cells are searched
identically. Checker timeouts: 24 and 23 against 22 and 22, the
engine's proofs on cells like bivium being slow to check (its 3.6 GB
stream took 71 minutes, past the harness's budget: an unverified
answer, not a failure). **Promoted** (2026-10-18): `gauss` is on by
default; the stock path is `--gauss=0` with the struct options at 0.
Left for a later candidate: no matrix for cells with equivalences only
(46 of the 107), and the checking cost.

**Proof** (X.2, `src/gaussproof.rs`). The structure pass's VeriPB
stream stays open through the search and the engine writes into it, so
the gate's first stage proves every clause the engine added from the
input alone and its second stage (drat-trim) sees those clauses as part
of the extended formula. Each input XOR becomes two linear inequalities
"sum of its literals − 2 × (carries) ≥ parity" and "≤" through a chain
of fresh xor/and variables (seven `red` clauses, three `rup` and two
`pol` lines a link; the parity of the chain's end from the XOR's
clauses level by level, three `rup` lines a clause). A row is the sum
of the input XORs in its set, derived (two balanced `pol` sums) when a
clause is first needed from it after the set changed; a row addition
itself costs nothing in the proof. A clause of a row is one `pol` line:
the row's "≤" side plus the clause's literal axioms (summed first among
themselves), divided by two and doubled, plus the "≥" side; the odd
right-hand side is what the division cuts, and what is left is the
clause. Replaced row forms are deleted. The derived clauses are
streamed to `<proof>.derived.cnf` (the header patched at exit) and the
stream is finished at every exit. Checked on a toy with VeriPB 3.0.2
before the code (equality checks that the derived constraints are the
clauses), then by `tests/gauss.rs` (an UNSAT found in search with the
engine's reasons in it, a refutation at the build, a satisfiable run
with a proof path) and a fuzz of 600 random mixed formulas with proofs:
272 UNSAT answers through the gate, 328 models checked, no failure.
A proof step that cannot be written ends the run with an error, as
the DRAT writer's does (a reviewer's case: a file size limit left an
UNSAT answer whose DRAT depended on clauses the dropped sidecars no
longer proved); the table of row sets counts with the matrix against
`gausslimit`. Review: two Codex rounds, the second clean.

**Cost of checking.** A parity-learning formula of 3,250 variables
refuted in 0.4 s with 914 engine clauses: 113K proof lines, the gate
3.3 s. par32-2 itself (satisfiable, 6 s with the proof written, 80K
engine clauses): 836 MB of proof, VeriPB 64 s, 92% of it in constraint
additions (the row sums; written as a left-to-right chain they took 31 s
on the small case, a balanced tree 3 s). Keeping a row's form
incrementally (two short lines per addition) cut the file to 72 MB but
the check ran past eight minutes: the forms grow terms that never
cancel. So an UNSAT answer from a run that leaned on the engine for
long is checked at ten times the solve time; the harness reports those
as checker timeouts (unverified, not failures). Only 64 of the 913
engine clauses of the small case are in drat-trim's core: deriving a
clause only when conflict analysis touches it would cut the proof
tenfold, but a clause kissat later rewrites (vivify, collection) would
then have no derivation, so every clause is proved when it is made.

## Next: netlist recovery (2026-10-19; epic `SAT-playground-skz`)

What is left after every pass and the engine, read off the run of record
(`log/abtest-struct-lazy-400-2026-10-17-08-31-04`, gauss arms, seeds 0
and 1), and the measured facts behind the next line of work. The ideas
themselves are in the epic's beads.

**The 72 cells unsolved on both seeds**, by kind: about 40 industrial or
arithmetic (16×16 multiplier miters 3, `multiplier_15/16bits__miter` 2,
`div_miter_lec`, `hhyp_cec`, sorter miters 2, oisc 3, bp4 2, timetable 5,
argumentation 5, sudoku 2, et-tc 2, singletons); 12 crafted combinatorics
with no known short certificate (ramsey 6, vdw, cfi-rigid, 6-colouring,
baseballcover, connm-csp, dubois50); about 20 with a known structural
answer not built (rooks 3, oddball 6 satisfiable, lockchart 9, rbsat,
sum-of-cubes); two that abort at 16 GB (`pj2002_k500`, anon-normalised 17;
the C kissat aborts on both too, exit 134 in the September paired run).
48 of the 72 carry pairwise at-most-one groups, but those are CSP-shaped
exactly-one blocks (sudoku 90K groups, timetables 31-34K, lockchart,
rbsat, cfi, argumentation); they propagate completely already, so native
at-most-one would buy nothing there. The census has no detector for
encoded at-most-k with k ≥ 2 (sequential counters, totalizers, adder
popcounts, decision diagrams); the file headers are normalised, so that
needs a detector run (bead `skz.5`).

**The arithmetic cells sit at the edge.** Of the 12 `16_16` miters, 7-9
are solved at **608-1707 s** and 3-5 time out depending on the seed;
`BubbleVsPancakeSort` 7_6 and 8_4 take 257-670 s, 8_6 and 9_4 time out;
`sqrt-mitern` 96-624 s; `oisc-subrv-sll-nested-8` 387 s, the three
`and-nested` cells time out. A 2× search speedup would convert about four
cells; a certified linear argument (after the partial products a
multiplier is linear; bead `skz.3`) would take the family to seconds.
The 2026 suite has two `16_16` booth-vs-and miters, `Circuit_multiplier36`,
three `BubbleVsPancakeSort`, two `DivS`, a sqrt miter and `bp5` by file
name, so the family recurs; it was not analysed further.

**Seed switching does not harvest the luck** (scratch analysis of the
same run). Single seeds solve 311 / 315, the union of the two is 328, and
27 of the 30 one-seed-only cells are satisfiable. But a run that uses
seed A from scratch for T seconds and seed B for the rest solves
**299-306 at every T from 300 to 1500 s**, below either seed alone,
because the one-seed-only solves are late (medians 785 and 1118 s) and
need most of the budget. The 13-cell gap is real; only a second core
(parallel track) reaches it, not a schedule. This closes the open
question of the local-search share ("spend satisfiable-side luck
better").

**BMC-shaped cells.** About 30 of the 2025 cells are bounded unrollings
of hardware designs (period in the census: VexRiscv 11881, veer_axi
226432, x-epic 30399 and 30862, sat-dat 15562). Nearly all are solved;
the slow unsatisfiable ones are `valves-gates-1-k617` (1533-1689 s; 985K
variables, about 1.6K per frame), `g2-T49` (1207-1393 s), `SAT_dat.k100`
(778-963 s), `oski15a01b06s_opt` (608-1150 s), `x-epic_a19-p16_step`,
`dijkstra_step` and `VexRiscv_step` (284-566 s); unsolved:
`g2-hwmcc15deep-oski15a10b10s-k20`, the three `oisc-and-nested` program
unrollings, `pj2002_k500` (memory). An IC3 probe on de-unrolled copies
is bead `skz.7`.

**Recovery census, the multiplier slice (2026-10-19; bead `skz.1`;
`tools/netlist_census.py`, `benchmarks/rl/netlist_2025_multipliers.tsv`).**
The tool reads a CNF back as a circuit: gates by the definition test (a
variable its clauses determine as a function of a few others; parity
clause sets define every variable in them, so the direction is settled
forward — a gate is accepted once its inputs are settled and none of
its clauses is used by an accepted gate; a deadlock means a primary
input), structural hashing, half and full adders by truth tables over
2- and 3-input cuts (so an adder built from ANDs is found as well as
one written with XOR clauses), weights by a union-find with offsets,
the operand grid from the adder leaves, and the two cones of the miter
from the compared variables. Checked on toy miters first (two 4- and
6-bit multipliers with different adder trees, XOR3 gates or 2-input
gates only, variables renumbered or not): 12 of 12 inputs, one network,
the 6×6 grid, consistent weights. Three things the real cells taught
the tool: control signals fan out to hundreds of clauses (an
occurrence cap of 64 lost the Booth encoders, so the odd `b` bits came
back as XORs of the encoder outputs); two real gates can imply a third
"definition" between them (`37 = 36 ? 14 : 13` and `109 = 36 ? x : 14`
give `14 = 36 ? 37 : 109`), which is why direction has to be settled
globally; and an XOR gate is reversible, so with renumbered variables
an input can only be told from an output by fan-out.

On the 14 multiplier cells (gate coverage 97-99 % everywhere; `grid` is
the AND partial-product grid; a side is one compared cone, `explained`
the share of its gates inside recovered adder cells, `cpa` the sum bits
of a final adder found by shape):

| cell | fa | ha | weight conflicts | grid | side A | side B |
|---|---:|---:|---:|---|---|---|
| `multiplier_16bits__miter_22` | 445 | 503 | 0 | 16×16, 255 pp | 1.00 (391 cells) | 1.00 (412 cells) |
| `multiplier_15bits__miter_23` | 387 | 441 | 0 | 15×15, 224 pp | 1.00 (374) | 1.00 (398) |
| `16_16_and_wallace_origin_and_default_mapped_ultra_bit27` | 383 | 467 | 0 | 16×16, 243 pp | 0.95 (446) | 0.93 (404) |
| `16_16_default_mapped_ultra_and_and_dadda_mapped_bit28` | 385 | 410 | 0 | 16×16, 244 pp | 0.92 (411) | 0.90 (390) |
| booth-vs-and, `origin` Booth side (5 cells) | 285-295 | 339-385 | 10-12 | 16×16, 217-250 pp | Booth 0.50-0.53 (239-260) | AND 0.82-0.87 (387-437) |
| booth-vs-and, `mapped` Booth side (4 cells) | 241-246 | 290-328 | 0-2 | 16×16, 230-247 pp | Booth 0.22-0.23 (113-119) | AND 0.91-0.95 (418-455) |
| `16_16_booth_dadda_mapped_and_booth_wallace_mapped` | 148 | 205 | 10 | none (Booth both sides) | — | — |

**Reading.** The two `multiplier_Nbits` miters (both-seed timeouts) are
two multipliers over one shared 16×16 grid, every gate of both cones
inside an adder cell, weights consistent: the linear pass as designed
(bead `skz.3`) refutes them with no gap to close. The two AND-vs-AND
`16_16` cells (608-1707 s today) are the same except for 5-10 % of each
cone, which is the **final carry-propagate adder**: a parallel-prefix
adder, not a ripple one — the half adders give propagate and generate,
the unexplained AND trees are the prefix network, the last XORs the sum
bits. That block needs its own step in the pass (the carry into each
bit proven equal to the ripple carry by a SAT sub-call logged as `rup`,
then the adder is virtual full adders; AMulet's adder substitution in
certificate form). On the Booth-vs-AND cells the AND side is 82-95 %
explained (the rest is the same prefix adder) and the Booth side's gap
is its partial-product logic (digit encoder and row muxes), which is
step two (`skz.4`); the mapped Booth trees also lose about half their
adders to a cell shape the matcher does not know yet. A single-bit
miter's cone holds the carry chain but not the lower sum bits, so the
bit-equality argument will define them as extension variables. The
weight conflicts on the Booth cells are rejected cells (an XOR and an
AND over the same two encoder signals look like a half adder).

**The certificate, checked on a toy (2026-10-19; bead `skz.2`;
`tools/arith_toy_proof.py`, `log/arith-toy-proof-*`).** Two n×n
multipliers over the same AND partial products, an array one against a
column-wise one, the miter on one output bit, and a VeriPB 3 proof
written the way the pass would write it: per adder cell the two sides of
`la + lb (+ lc) = s + 2c` as `pbc` blocks with a case split in `rup`
steps (nine for a full adder, two or three for a half adder); per side
one weighted `pol` sum (the telescoping: every internal signal cancels,
the partial products and the output bits remain); the two sides added so
the shared partial products cancel; then per bit from the top, weaken
the lower bits and divide by the bit's weight for the two equality
clauses, and cancel the bit from the running constraints (a bit one side
lacks, such as a dropped top carry, is proven zero the same way); one
case split against the miter's compare. VeriPB accepts the proofs for
4-bit miters in both gate styles (XOR3 and majority clauses; 2-input
gates only), the 8-bit one, and the **16×16 one at the real cells' size:
2,593 variables, 8,613 clauses, 480 cells, a 10,284-line (300 KB) proof
checked in 0.03 s**, against 608-1707 s of search on the real `16_16`
cells. A tampered proof (one adder's inequality off by one) is rejected
each time. Three things learned writing it: moving `−s − 2c` to literal
form adds 3 to the right-hand side whatever the number of inputs (a
half adder's inequality off by one made every bit clause trivial); a
`pbc` block consumes one constraint id for the negated constraint and
one per subproof line; and the last step needs one case split, since
four binary clauses with no unit do not propagate to conflict.

**Recovery census, the whole 2025 suite (2026-10-19;
`benchmarks/rl/netlist_2025.tsv` (a copy of the run under `log/netlist-census-2026-10-19/`), the family table from
`tools/netlist_census.py report`).** 400 cells, 24 workers, about an
hour; cells over 400K variables are skipped (`too-big`, 79 cells, among
them `oisc`, `pj`, `oski`, the big `lockchart` and `sudoku` ones), 92
cells have no gate structure (`no-gates`), and 9 cells over the gate cap
got the gate pass only. The tool went through seven
Codex rounds (17 findings, every one a metric or robustness defect in
the measurement: a cell-wise weight check, aliases through buffers and
inverters, the two sides of a miter found by walking from its unit
through and/or gates to the compared signals with the compare cells
told from adders by their inputs' cones, a non-overlapping cell cover,
word grids only with distinct bit positions and no weight conflict,
completion-order writes, degenerate toy tampers). What the suite looks
like as circuits:

- **Circuits with adder networks (50 cells or more): 67 cells** —
  multiplier-16x16 12, bp 12 of 20, itc99 7, div-sqrt-miter 4,
  hwmc-step 3, sat03-nw 3, grs 3, iscas 3, three singletons, mp1 2,
  mult-miter-bits 2, bits-fast 2, circuit-multiplier 2, and one each of
  reconf, parity, arith-xor-misc, sum-of-cubes, factoring, lec-mult,
  jkkk, aprove, hwmcc-bob. **Word multiplier grids (8×8 or wider): 19
  cells** — the 9 AND-array sides of the `16_16` cells, both
  `multiplier_Nbits`, sat03-nw 3, bits-fast 2, and one each of
  arith-xor-misc, hwmc-step and lec-mult.
- **No gate structure at all** (`no-gates` or coverage under 5 %): the
  combinatorial and CSP families — roundrobin, timetable, sudoku, arles,
  ramsey, php, rooks, lockchart, oddball, kakuro, vdw, cfi, rbsat, frb,
  et-tc, itc2021, clqcl. For those the netlist route is the wrong tool;
  the clause-level detectors (bead `skz.5`) are the route.
- **Two-cone miters:** 102 cells have two compared cones, 12 of them
  with both cones 90 %+ inside adder cells (`16_16` 2, `multiplier_Nbits`
  2, sat03-nw 3, bits-fast 2, and one each of grs, sum-of-cubes and
  lec-mult).

On the **72 cells unsolved on both seeds** the netlist reading is:
`multiplier_16bits__miter_22` and `multiplier_15bits__miter_23` — two
multipliers over one shared word grid, both cones 100 % adder cells,
the linear pass as designed; `16_16_booth_dadda_origin_and_and_dadda_*`
(2 cells) — AND side 0.88-0.94, Booth side 0.50, step two;
`16_16_booth_dadda_mapped_and_booth_wallace_mapped` — both sides Booth,
0.28-0.30; `fermat` (factoring) — 99 % gates, **3,508 full adders**, no
operand grid (one input word squared and compared, not a miter: a
number-theory cell, not a linear one); `sum_of_3_cubes_37` — 7,315 full
adders (cubes are multipliers), not a miter; `div_miter_lec__2` — 92 %
gates, 215 full, 498 half and 35 carry-in-one adders, but the two
compared cones are 0-6 % explained: a divider's subtract-and-select
loop is not an adder tree; `bp4` (4 cells) — 33-43 % gates, 183-258 full
adders and **138-155 half adders with a constant carry-in** (the
subtractor shape; the names `FPBEQ`, `FPBLE`, `ZR` read like
floating-point compare units), sides 11-15 % explained;
`BubbleVsPancakeSort` 8_6 and 9_4 — 88-91 % gates and **no adders at
all** (comparators and muxes); `oddball` — no gate structure (0-0.2 %),
so the counting backend must come from clause-level detection; the
rest are the CSP and combinatorial families above, or too big for the
Python pass.

**Reading for the decision (bead `skz.11`).** The linear pass's step one
has exactly two both-seed-unsolved targets on 2025 (`multiplier_Nbits`)
plus the two AND-vs-AND `16_16` cells it would take from 10-28 minutes to
seconds, and the holdout has two `16_16` miters and `k2mul` by name.
Step two (Booth) adds the five `origin` and four `mapped` Booth-vs-AND
cells (three of them both-seed-unsolved) and the Booth-vs-Booth one.
Beyond multipliers the arithmetic cells need other backends: factoring
and cubes are number theory, the divider needs the restoring-division
argument, `bp4` needs subtractor and compare blocks, and the sorters
need comparator-network reasoning. The counting families (oddball,
timetables, rooks) have no circuit to recover.

**Cardinality encoding census (2026-10-19/20; bead `skz.5`;
`tools/card_census.py`, `benchmarks/rl/card_2025_unsolved72.tsv`, the run
`log/card-census-2026-10-19/card_72_final5.tsv`).** Detectors for counts
with k ≥ 2: Sinz counters and totalizers (merge nodes whose staircase of
clauses is complete, linked into trees through their output vectors),
comparator networks, counter decision diagrams (the bound is the range of
totals a walk from the asserted root accepts) and direct subset encodings;
popcount adder networks are reported as candidates only (gate recovery can
orient a full adder backwards, so their inputs are not reliable). A
bound comes from forward clauses only (a unit -r_m or overflow clauses)
and **is proven before it counts**: solver 13 must refute the structure's
own clauses plus the opposite count (at least k + 1 of the inputs), else
the structure is reported free. On the 72 both-seed-unsolved cells (first
3M clauses of each file, 16 workers on a quiet host, the proofs by the
frozen snapshot binary, 60 s each) that is **153,385 bounds proven, none
refuted, 3 unknown** (the three rook diagrams below). Only counts with
2 ≤ k ≤ n − 2 are counted (at most one and single clauses are not), an
input read twice makes a weighted sum (not counted), and an input with
its negation in one count cancels. 77 toy checks (`python3
tools/card_census.py selftest`); 15 Codex rounds on the tool, the bound
check added after four findings in a row of the same kind (a false
bound from a capped merge). Real counts sit in 8 cells: the five
**timetables** (850-994 Sinz counters each, at most 2-5 over 6-7 or 50-58
inputs, 19 % of the clauses), **ITC2021** (1,475 totalizers, all at most
2, 1,440 of them over 57 inputs; 58 %), `baseballcover` (15 Sinz counters
over the same 323 inputs: at least 12, at least 25 and at most 25; read
on the first 3M clauses) and `dubois50 mis-99` (at most 99 of 150). The three **rooks** carry one counter diagram each
(132-176K nodes over 2,599-3,134 squares, 76 % of the clauses), but the
check of their bound times out at 60 s: refuting "the diagram plus n + 2
rooks" is the counting argument CDCL cannot make, which is what makes the
cells hard (the exact n + 1 comes from the decoding of 2026-10-14). The
two **et-tc** cells hold one diagram each (511 levels, width 102 / 120,
written both ways) whose walk accepts every total, so no bound is read.
**oddball** has no counter at all (two cells read in full by a separate
script; the cap shows only 6-16 % of the six big files): every implication
of `oddball_51_5` sits in one of 19,128 big OR definitions (2 to 12,000
inputs; the 4,701-literal clauses are their long halves), the equal number
of coins on the two pans is a pairing (coin i on the left pan opposite
coin j on the right, each coin in at most one pair per weighing through a
commander-style at-most-one), and its guarded "at least 2 of 4" families
are vacuous (each guard occurs elsewhere only in clauses a unit already
satisfies). Small guarded "2 of 4" families also show in bp4 (120-291 per
cell), `oddball_29_4` (2,088), vdw and the miters: threshold gates, not
counts. Nothing in ramsey, lockchart, argumentation, sudoku, rbsat, cfi,
the colouring and the oisc cells. **Reading for the counting backend
(bead `skz.6`):** the counts in the unsolved cells are written as Sinz
counters and totalizers, encodings that already propagate completely, so
a native at-most-k engine would save work there but add no reasoning
power; oddball, the cell family this was mainly for, has no count; the
rooks stay the one case where counting decides a cell, and their obstacle
is the certificate (bead `1v2.4`).

## The linear arithmetic pass (2026-10-19; bead `SAT-playground-skz.3`)

Not in kissat. `--structarith=1` (**default on since 2026-10-20**, after
the paired run at the end of this section; `src/structarith.rs`,
tests in `tests/structarith.rs`). After the partial products a multiplier
is linear: every adder cell says `a + b (+ c) = s + 2 carry` over the
integers, so two adder trees summing the same weighted bits give the same
integer, whatever their shape, and their output bits are equal bit by
bit. Resolution has no short proof of that; cutting planes has one a page
long, and the structure pass already writes cutting planes.

**What the pass does.** It reads the buffered formula back as a circuit
(gates by the definition test with the direction settled forward, the way
`tools/netlist_census.py` does it), merges equal gates, finds half and
full adders by truth tables over 2- and 3-input cuts, finds the two cones
the formula compares (from its unit clauses, through the and/or gates of
the compare), and gives every cell a weight (a sum keeps its inputs'
weight, a carry doubles it). It then checks, as integers, that each
cone's cells summed with their weights cancel every internal signal and
leave only partial products both cones read the same way, plus the
cone's output bits. Each cell has two readings (`a + b = s + 2c`, and with
every literal complemented `~a + ~b + 1 = ~s + 2 ~c`); the pass picks
them together so that every signal is the same literal where it is made
and where it is read. When the check closes, it derives bit by bit from
the top the two clauses "bit of A = bit of B" (or "this bit is zero" for
a bit one side lacks) and hands them to the solver; unit propagation then
finishes. The carries that leave the compared bit's cone (above it) are
one block, proven equal on both sides and cancelled before the bit steps.
When the check does not close (a gate in a cone outside the cells, such as
Booth partial-product logic), the pass derives nothing and the search
runs as without it. Carry networks and lone gates are handled by the
final-adder step below (2026-10-21).

**Proof** (checked on toys first, `tools/arith_toy_proof.py`): merged
gates proven equal to their representative by contradiction with a case
split over the gate's inputs (plain unit propagation cannot do it for two
copies of an xor); per cell the two sides of its equation as `pbc` blocks,
each with a full case split over the cell's inputs (under a full
assignment every gate of the cell propagates forward, whatever the
encoding); per cone one weighted `pol` sum; the two cones added so the
partial products cancel; the top block and then each bit by weakening the
lower bits and dividing by the weight.

**Three correctness bugs found before anything was committed**, each now
a regression test that fails on the build that had it
(`tests/structarith.rs`) and each caught by the fuzz below on that build:
(1) the two sides' constants were not compared, so a miter where only one
side folds in a `+1` (satisfiable: the products differ) was answered
UNSAT; (2) the leaf comparison was keyed by literal, so a side reading one
partial product twice matched a side reading it once, a wrong UNSAT again
(found by Codex); the check now compares each side's whole leaf sum as a
linear form, constants included, which covers both; (3) the cell
subproofs were short case splits that relied on unit propagation running
backwards through the cell's gates, which an adder with the right truth
table but an unusual carry cone (`(a ∧ b) ∨ ((((a ∨ b) ∧ c) ⊕ b) ⊕ b)`)
does not allow: VeriPB rejected the proof (found by Codex). Without a
proof file the first two would have been wrong answers with nothing to
catch them, which is why the pass is fuzzed against the stock answer.

**Fuzz** (`tools/arith_fuzz.py`). Random miters of two 3-6 bit
multipliers over shared AND partial products, random shapes (array or
column-wise, reduction orders, a constant one folded into a random pair,
complemented half adders, XOR3 and majority clauses, 2-input gates, the
unusual carry) and on a quarter of them a fault that makes the sides
differ (a constant on one side, a partial product dropped, duplicated,
moved a column, inverted). Each is solved with the pass off, on, and on
with a proof file: the three answers must agree, every model is checked,
every UNSAT proof from a run where the pass fired goes through the gate.
On the final build: 1,500 miters (`log/arith-fuzz-2026-10-19b.txt`), all
agree, the pass fired on 609 UNSAT ones, every proof accepted; an earlier
1,500 without the new shapes, the same. The same seeds on the build before
the fixes: 45 proofs rejected and 6 wrong UNSAT answers in the first 300.

**Measured so far.** The two cells of the 72 unsolved on both seeds that
the recovery census marked as two multipliers over one shared grid,
`multiplier_16bits__miter_22` and `multiplier_15bits__miter_23` (both
timeouts at 1800 s in the run of record), are **UNSAT in 0.17 and 0.16 s**
with the shipped defaults and the pass on (44 and 46 derived clauses,
work 2.0M and 1.7M, proofs of 583 and 560 KB through the two-stage gate in
half a second). The twelve `16_16` cells decline, as the census predicted:
on the Booth-vs-AND ones the weights do not form one network (the Booth
partial-product logic) or no consistent cell reading exists; on the
AND-vs-AND `ultra` cell side B reads 14 signals side A does not (the
prefix adder's carries). Seventeen hand-built toy miters through the
gate: array against column-wise and column-wise against column-wise
multipliers of 4-8 bits, XOR3 and majority clauses or 2-input gates only,
variables renumbered, NAND partial products, a whole-word compare,
columns shared by both sides (9-20 merged gates in the cones, xors
included), a constant one folded into a different pair on each side (two
constant-one cells), the unusual carry. Writing the proof also found two
errors the gate caught at once: the merged-gate equivalence as a plain
`rup` (fine for and gates, not for xors) and the right-hand sides of the
constant-one cell (ge is 3 − 1, le is k + 1). A sweep of the pass alone
over all 400 cells with the committed build (work clock capped at 30M,
`log/structarith-fire-2026-10-19d/fire.tsv`) fires on exactly those two
cells and on nothing else, with no panic; on the 342 cells where it runs
and finds nothing it costs a median 19.4M work (at most 20.1M, about a
second), the price of its limit; the other 56 cells are over the
structure stage's 10M-clause limit. Review: nine Codex rounds (round 1
found the leaf and the encoding bugs above; rounds 2-8 the work limit and
its accounting: polls by work done, live counting, affordability before
allocation, the gate cap, checked sums; the ninth, on the commit, clean).

**Paired run and promotion (2026-10-20).** The 400 cells of 2025, seed 0,
1800 s, 32 jobs, the frozen build of 0782bf5, the pass on against off
(`log/abtest-struct-arith-400-2026-10-19-21-28-07`, launch log
`log/struct-arith-400-launch-2026-10-19b.txt`): **313 v 312 solved**,
tick PAR-2 0.976x, wall PAR-2 0.986x, promotion gate PASS, no
contradiction. The gain is the two multiplier miters
(`multiplier_16bits__miter_22`, `multiplier_15bits__miter_23`: a timeout
against UNSAT in 0 s, both proofs checked). All 310 cells both arms
solved have the same conflict counts, so a declining pass changes no
search, and one seed is enough. Three other cells flipped on the same
search by wall-clock luck near the limit: `oddball_29_4` +1 (solved at
1791 s), `dislog_a14_x14_n24` −1 (base solved at 1800 s) and
`16_16_booth_wallace` bit 28 −1 (base 1476 s; the pass costs 0.04 s
there). The census and the reviews shared the host, and wall times on
identical searches moved by up to half. When the pass declines it costs
0 to 20.0M work (median 7.8M; nothing on the 40 cells over the clause
limit and the 41 solved before it runs) and at most about a second
(`xor_op_n38_d3` 0.79 → 1.72 s at `--conflicts=1`). Per family:
mult-miter-bits 0 → 2, oddball 11 → 12, multiplier-16x16 8 → 7 and
singletons 9 → 8 (booth_wallace and dislog, the luck above), the other
91 families equal (284 each). With the pass off the stock path is
unchanged: `tools/parity.py` on its 9 corpus cells and on three 2025
cells the pass fires or declines on (the 16-bit miter, booth_wallace,
xor_op_n38; 50,000 conflicts) matches the C kissat on all 80 counters.
No second seed was run: the pass changes no search, so on seed 1 it
refutes the same two miters before search and leaves every other search
as it is (CLAUDE.md asks for a second seed when a candidate moves every
cell's search).

## The final-adder step (2026-10-21; bead `SAT-playground-skz.12`)

Step one needed every gate of a cone to be an adder cell. The two
AND-against-AND 16x16 cells are not like that: both compare one output
bit of two multipliers whose lower sum bits were removed as dead logic,
and both end in a carry network. `16_16_and_wallace_origin_and_default_
mapped_ultra_bit27` has a Wallace tree, a carry-skip adder (two chains
over shared propagate signals, the carry handed over complemented by a nor
chain) and an xor tree over the top column on one side, and on the other
a prefix network of and/nor gates over generate/propagate half adders
written as xor + nor pairs. `16_16_default_mapped_ultra_and_and_dadda_
mapped_bit28` adds a mapped Dadda tree where 14 columns' half-adder
carries are written over two *other* partial products of the same four
inputs (`a_i b_j . a_k b_l = a_i b_l . a_k b_j`, the mapper sharing
product nodes) and the sums as `nor(carry, nor(pp, pp))`. In S.8 these
took 1582 s and 793 s (kissat 1616 and 742 s). Now both are refuted in
**0.12 s**, with proofs of 35,607 and 28,625 lines the gate checks;
`multiplier_1[56]bits__miter` still fire as before; nothing else on the
400 cells fires (`log/structarith-fire-2026-10-21b`, no panic, the
declining cost unchanged: median 19.8M work against the 20M cap, as the
sweep of 2026-10-19 had 19.4M).

How it works, in the order the pass runs it (`src/structarith.rs`):

- **Lone sums and carries.** A gate with a sum's function over a cut and
  no carry beside it (a top-column adder whose carry was dead logic), or
  a carry's function with no sum (a ripple chain whose sum bits were),
  becomes a cell with a *fresh* partner: a variable of the proof alone,
  defined by its clauses with `red` under the witness that sets it (the
  solver never sees it; VeriPB accepts that, checked on a toy first; the
  numbers continue past the formula's and the parity engine, which takes
  the proof stream over when the pass leaves work for search, takes its
  own from there: `Derived::last_var`). A
  lone sum reads each leaf as its producer writes it and may hang one
  unweighted leaf on the others' weight (the network's carry); a lone
  carry needs every leaf weighted, reads a written leaf as written and an
  unwritten one plain, or takes the whole flipped reading when every leaf
  disagrees, and is skipped on a mix. A gate strictly inside another
  gate's possible carry cell, read by that path alone, is no cell of its
  own (a mapped majority holds an and and a nor over the same two inputs).
  Partial products no cell reads (a column whose sum bit is dead, the top
  column's xor tree) get their weight from the operand grid: `pp(a, b)`
  sits at `exp(a, b') + exp(a', b) - exp(a', b')`.
- **Product cells.** For an uncovered gate with four primary inputs under
  it, the three pairings into two products are tried; a sum and a carry
  that read through the same pair make a cell whose proof splits over the
  four inputs (a full assignment of the two products alone does not
  propagate the gates). A carry some other gate reads is preferred (the
  sum is built from the carry and a private nor over the pair), the
  nearest by variable number among those.
- **The chain and the lemma.** A leaf no cell of the side produces, a
  gate over gates the other side does not read, is the carry network's
  output, one per side at most and read by one cell (the lemma cancels
  one read; a sum reading the carry twice is a wrong circuit, left to
  search). A virtual ripple chain of fresh cells adds
  up everything the side's cells leave over at or below that carry's
  column (the output and the cells over the carry aside), with the two's
  complement of the side's carry-in-one constants as fixed-one variables
  (`red 1 x >= 1 : x -> 1`), so the sums are off by a multiple of
  2^(K+1), which the top block absorbs. Kitten (the port's sub-solver,
  antecedents tracked) proves the carry equal to the chain's bit at that
  column, over **the side's whole cone** restated gate by gate over its
  inputs plus the fresh definitions: the network alone, or with the cells
  it reads, was not enough on the mapped Dadda cell, whose resynthesised
  network leans on facts about the tree (signals never both one) that only
  the tree's gates give. With the whole cone kitten's search was erratic
  (0.2M ticks one run, 1.6 billion another) until the clauses went in
  circuit order, inputs first: kitten decides the variables it met last
  first, and deciding primary inputs first just simulates the tree. Now
  the lemmas take 0.07M-0.6M ticks and 130-445 learned clauses. If the
  plain lemma fails, the complement is tried; a nor chain hands the carry
  over complemented, and the top cell (a lone sum of the pass's own) then
  reads it so. The proof: every gate of the cone as `rup` lines (a full
  assignment of a gate's inputs propagates its output), kitten's core
  learned clauses in their order, the lemma's two clauses, and those two
  cancel the carry against the chain's bit in the weighted sums; the fixed
  ones become constants by the literal axiom one way and the unit the
  other.
- **Readings.** The rule that every reader of a leaf reads it the same way
  is gone (the linear forms carry a negated reading as `1 - x`; only a
  produced signal must be read as written). The real cells' readings are
  settled first, the virtual cells read the final literals, and a side
  that reads its leaves mostly negated is flipped whole when it shares no
  cell with the other side (the two readings of a component are both
  valid, and the sides must agree on the shared products). `find_adders`
  pairs a sum with the carry some other gate reads, nearest by variable
  number (two circuits over shared products each have their own; a
  mapped xor is `or(and(a,~b), and(~a,b))`, and those and-nots are not
  carries), and a side keeps one cell per leaf set (a merged sum gate
  would otherwise bring the other circuit's cell along). A dead-end cell
  is dropped only when no gate reads its outputs (the generate/propagate
  half adders feed a network, not a cell).

Tests (`tests/structarith.rs`): Kogge-Stone against Brent-Kung,
Sklansky against ripple, pruned to the compared bit or not, generates
and propagates plain or inverted, a ripple with dead sums against a
prefix adder, every proof through the gate, one of them with every
default on (the parity engine continues the proof) and the same work
clock on three runs; a tampered chain lemma rejected by the gate; a
prefix network with a wrong carry, and a sum reading the carry twice,
left satisfiable. The lemma's clauses, the fixed ones and the product
grid go in sorted order: kitten numbers variables as it meets them and
its decisions follow, so a hash order there made the work clock vary
from run to run (review, 2026-10-21). The review took six rounds and
found three false-UNSAT cases on toys it built (a carry read twice by
the compared sum; a fixed one alone at the chain's top column, see
above; the pass's and the parity engine's proof variables numbered from
the same start), the hash orders, kitten ignoring the live stop flag
(now solved in pieces of a million ticks), a quadratic traversal
without a budget check and an overflow in the constant sum; each has a
test or a fuzz shape now. One more came from re-deriving the argument:
the complemented lemma was proven over the top cell's old carry
definition and written over the new one, so its learned clauses could
have mentioned a variable the proof defines differently; the cell is
now changed first and the lemma proven over what the proof writes. `tools/arith_fuzz.py` now draws two-row reductions with
ripple, Kogge-Stone, Brent-Kung, Sklansky or carry-skip final adders,
regrouped product carries, random gate polarities and dead-logic
pruning: 3,000 miters on two seeds all agree with the stock solver, the
pass fired with a checked proof on 1,156 (`log/arith-fuzz-2026-10-21*.txt`).
On the toys the pass fires on about half the unsatisfiable miters of the
new shapes; the rest decline for toy reasons (a pruned 5-bit cone has
too few cells; the compare detection needs cones of 16 gates) or are
left for later (a network read at two weights, Booth: bead `skz.4`).
Debug switches: `SAT_STRUCT_DEBUG_GATES` (the gates no cell covers, with
weights), `SAT_STRUCT_DEBUG_CELLS=all|v,..`, `SAT_STRUCT_DEBUG_VIRT=v`
(trace one gate's virtual-cell attempt), `SAT_STRUCT_DEBUG_LEMMA=path`
(the chain's columns, every cell, kitten's clauses and model on failure).

**Paired run (2026-10-21).** The 400 cells of 2025, seed 0, 1800 s,
16 GB, 32 pinned cores, the frozen binary
`solver/13-j2-final-adder-snapshot` (sha256 `e57e0a8e507378b7`) with
the pass off (`--structarith=0`) against on, every answer checked
(`log/abtest-struct-final-adder-400-2026-10-21-15-19-20`):

| arm | solved | PAR-2 | tick PAR-2 |
|---|---|---|---|
| pass off | 311 | 411,558 | 1.101e13 |
| pass on | 313 | 400,978 | 1.062e13 (0.965x) |

No contradiction. The two miters go from the 1800 s timeout to 0.35 s
and 0.18 s; the two AND-against-AND cells from 1663 s and 865 s to 0.1 s
each (under this run's 1800 s limit the pass-off arm solves them too,
so they do not show in the count). Of the 312 cells both arms solve,
310 have the same conflict count, the other two being those; the two
status swaps are wall luck on an unchanged search: `oski15a01b15s_opt`
was killed with the pass off at 96 % of the work the pass-on arm
needed (33.2G of 34.7G), and `16_16_booth_dadda_mapped_and_and_wallace_
mapped_bit28` was killed with the pass on at 82 % of what the pass-off
arm needed (53.8G of 65.5G). The two cells that run out of the 16 GB
are the same in both arms. The pass's own cost on the 396 cells where it
declines is at most the 20M-work cap (the cells differ in W by that
much), about 0.1 % of the suite's tick PAR-2.

## Booth, step two (2026-10-22; bead `SAT-playground-skz.4`, in progress)

Nine of the ten Booth cells of 2025 compare a radix-4 Booth multiplier
with an AND-array one (the tenth is Booth against Booth); five have the
Booth side in the generator's `origin` form, four in its `mapped` form.
Their partial products are not shared with the AND side, so the pass's
"same leaves" argument does not apply. What a Booth row adds is still
linear in products: digit `d = -2 b_{2j+1} + b_{2j} + b_{2j-1}`, row
`a * d * 4^j`, and summed over the digits (with the correction row
`a * b_15 * 2^16` for an unsigned multiplier) the rows telescope to
`a * b`. The design, checked on a toy first (`tools/booth_toy_proof.py`:
VeriPB accepts the derivation, a tampered coefficient is rejected): a
row's bits are *input leaves* (functions of five inputs: two bits of `a`
and the digit's three bits of `b`), the digit's bits are the row's
*selector set* (under each of their eight assignments every bit of the
row is a constant or a literal of one `a` bit), and the row's weighted
sum is proven equal to its linear form over products by a `pbc` with a
case split over the selector set: per case the bits' values and the
products' values are `rup` clauses, a weighted `pol` sum with literal
axioms gives the identity plus a big-M multiple of the case's falsified
literals, saturation gives the case's clause, and the eight resolve to
the empty clause.

What is in the solver after the first session (all of it behind the
existing option, no behaviour change on the cells the pass fired on:
the four cells of 2026-10-21 fire with checked proofs and the same work
clock): the input leaves and their truth tables; the selector sets and
the groups (on `16_16_booth_dadda_origin_and_and_dadda_origin_bit28`
the groups are exactly the nine digits); the two sides compared as
polynomials over the inputs instead of as leaf sets (the old
comparison is the special case where every leaf is a product); the
netlist's circuit-order judgement ignoring variables with several
candidate definitions (two row muxes over one select "define" their
shared `a` bit, which turned the odd `b` bits into gates); the digit
encoder's xor+and (a half adder by function over two bare inputs that
feeds no cell) dropped; a fused-leaf cell matcher (the `origin`
generator folds the row bit's xnor into the adder reading it: the sum
and carry over the cut are no adder's, but `sum + 2 carry` is a sum of
leaves one of which is the xor of two cut signals, which becomes a
fresh variable); weights that flow only through produced signals (a
Booth sign bit is read at two columns, and the product `a_i b_15` sits
at column 15 on the AND side and 16 in the correction row), the sides
tied through the compared signals inside the lone-gate rounds; a side
flipped whole when its real cells' leaf polynomial is the negative of
the other side's; the lemmas with an allowance of ten times the pass
budget (the `and_dadda_origin` carry-skip lemma needs about 19M ticks);
and the chain pool taking what the network reads that no cell produces
(products by the grid, row bits by their row's template shifted along
the operand).

Where it stands: on that cell the AND side's lemma passes and the two
sides agree on every monomial but nineteen: the ones above the compared
bit (multiples of `2^(K+1)`, which the block step can absorb once that
path is exercised) and four at the lowest columns, the digit-0 row's
bits 0 and 1 and the `+neg` handling, which the generator fuses into
the final adder's and-not gates, so they are not signals the chain can
sum. Open, in order: define the missing low row bits fresh from the
row's template (the lower `a` input fixed to zero) and let the lemma
verify them; write the group derivations into the proof; the `mapped`
Booth trees (67 of some 250 cells found, a third shape); Booth against
Booth (fresh products on both sides). Debug: `SAT_STRUCT_DEBUG_NOCHAIN=1`
prints both sides' polynomials and their difference.

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
- **Overhead check (A.11, 2026-09-17).** What the plumbing costs when it
  does nothing: two arms on the 100 cells of `sat-comp-2025-medium`,
  1800 s, 16 GB, 32 pinned cores (16 + 16), no proofs, frozen copy of the
  committed binary (`305c22188e38d5dc`), launched by
  `tools/rl_stepA_overhead.sh` on an otherwise idle host (13:05 to 14:43):
  `base` (policy off) against `policylog` (`SAT_POLICY=stock`, a raw-state
  log per cell into the run's scratch, so the static pass, `observe()` at
  every observation epoch and the 981-column rows all ran, plus
  `SAT_WALL_LIMIT=1800` for the wall horizon). Run dir
  `log/abtest-rl-stepA11-overhead-2026-09-17-13-05-09` (per-arm
  `results.tsv`, `report.txt` from `tools/rl_stepA_overhead_report.py`).
  Both arms solved 72/100 with the same 300,310,928 conflicts on their
  solved cells, no contradiction and no cell that flipped between solved
  and timeout. The work clock W is identical on all 72 cells both arms
  solved (the parity claim, now on the whole medium suite); on the 28
  cells neither solved the W reached before the kill is 0.997x
  (wall-dependent). Wall: PAR-2 126,370 v 126,308 (1.0005x); the per-cell
  ratio policylog / base over the 72 both-solved cells has geomean 0.993,
  median 1.000, and 0.999 over the 46 cells base took at least 60 s; the
  extremes (0.65 and 1.18) are sub-second cells. That is inside the +-2 %
  run-to-run wall noise the 2026-09-15 A/A run showed, so logging,
  observation and the static pass cost nothing measurable and no
  profiling bead is opened. The row width (7.8 KB every 2^23 search
  ticks) is what the collector will pay in disk: about 60 MB per 1800 s
  trace.
- `tools/parity.py --solver-env KEY=VALUE` (repeatable) sets environment
  for the solver-13 run only, e.g. `--solver-env SAT_POLICY=stock` for the
  policy-on-STOCK check; kissat never sees it.
- `Cargo.toml` gives the `test` profile `opt-level = 2` (debug assertions
  kept): the integration tests run the binary on formulas with tens of
  thousands of conflicts, a minute per test file unoptimized and about ten
  seconds at level 2. `build.sh` and the release profile are unchanged.

**RL scheduler step B: the collector, the stock traces and the data tables
(2026-09-17/18; plan §5, §7 item 7, §8, §10 step B; beads
`SAT-playground-p9m.7.*`).** The tooling that turns the step-A solver into
a dataset. Everything lives in `tools/` at the repo root; the offline
tools that need numpy and pyarrow run in a venv
(`tools/rl/requirements.txt`: `uv venv ~/.cache/sat13-rl/venv --python
3.12 && uv pip install --python ~/.cache/sat13-rl/venv/bin/python -r
tools/rl/requirements.txt`), the collector and the family table need
only the system `python3`.

- **`tools/rl_collect.py` (B.1), the collection harness.** One
  invocation runs one *pass* over a job table: a job is one solver run
  on one cell with a flavour (`stock`, `fork`, `random`, `jitter`), the
  solver seed, a work-clock budget (`SAT_LIMIT_TICKS`) or a wall limit,
  and extra policy environment (`SAT_POLICY_BRANCH=...` for fork
  parents). `stock --suite X` builds the table itself; `table jobs.tsv`
  runs one built from the per-cell budgets (round 0); `resume DIR`
  continues a stopped pass (jobs with a record are skipped); `status
  DIR` prints progress. A pass is `log/rl-<name>-<timestamp>/` with a
  frozen copy of the binary (`bin/sat-solver`, sha256 in
  `manifest.json`, so a rebuild during the pass changes nothing), the
  policy log of every run (`logs/<stem>.<tag>.log`, children as
  `.b<D>.<k>`), each run's stdout and stderr (`out/`), one JSON record
  per finished job (`cells/`, the resume key) and `results.tsv`.
  Process accounting: every job is its own session and process group,
  pinned to as many cores as it has processes (a fork parent with 4
  live children takes 5 of the 32 slots, CLAUDE.md's cap), under
  `ulimit -v` and `timeout -k 30`; a SIGTERM to the collector SIGTERMs
  every live group (each solver prints its `c workclock` line and seals
  its log), SIGKILLs what is left after the grace, refuses to record the
  killed jobs so a resume reruns them, and checks nothing survived.
  Correctness: every SAT answer, parent or child, is checked with
  `tools/verify_sat.py` before the scratch CNF is deleted; parent and
  children of one branch point must agree on SAT v UNSAT; `--oracle
  results.csv` adds the statuses of earlier runs. A disagreement, or a
  model checker that could not finish, drains the pass (nothing new is
  admitted, live jobs finish, exit 3), and a resumed pass keeps that
  failure. A wall-limited job records TIMEOUT; a budgeted job that hits
  its safety wall cap records an *anomaly*, not a result, and so does
  each child the cap ended; a fork parent whose children kept the group
  alive past the cap keeps its sealed answer (checked as usual) with the
  cap noted as an anomaly of the job.
  Checked on `benchmarks/discriminating`: a stock pass (3 cells, 20 s),
  a table pass with a fork parent (6 children over two branch points,
  all SAT, all models verified, siblings agree), a random and a jitter
  run (`log/rl-t-stock-2026-09-17-22-56-04`,
  `log/rl-t-table-2026-09-17-22-56-21`), and two kill tests: a SIGTERM
  12 s into a fork parent with 4 live children left no process, no
  record and every log sealed with reason `signal`
  (`log/rl-t-kill2-2026-09-17-22-58-59`, `log/rl-t-kill3-2026-09-17-22-59-32`).
- **`tools/rl_features.py` (B.8), family labels and critic-tier static
  features.** `families` writes `benchmarks/rl/families.tsv`: one row
  per cell of sat-comp-2025 and sat-comp-2026 with a family label from
  an ordered rule table in the script (130 families over the 800 cells;
  the competition manifests carry no family column, so the rules are
  the metadata; the name-token heuristic of `rl_sweep_report.py` is the
  fallback and is used by none of the 800), the family size, and
  `also_in`: **8 files are in both suites under the same hash**
  (`16_16_booth_dadda_mapped_and_and_wallace_origin_bit28`,
  `bp5_CSO`, `case10`, `case19`, `homer11`, `oski15a01b15s_opt`,
  `rphp_p25_r25`, `xor_op_n40_d3`), so the 2026 holdout is not disjoint
  from 2025. `static` computes, per cell and cached per cell under
  `log/rl-features-cache/`, what the actor never sees: the `p` line,
  exact clause and literal counts, compressed and uncompressed size
  (`xz --list`, no decompression) and their ratio, a fingerprint of the
  comment lines before the `p` line, and two estimates on a 200 k-clause
  reservoir sample of the variable-interaction graph: modularity of a
  label-propagation partition and a min-degree treewidth bound over the
  4000 highest-degree nodes, capped at 400 (the fill-in past that costs
  minutes for a number that only says "dense"). Output
  `benchmarks/rl/static_features.tsv`. The actor-tier features (140
  values, step A.7) come from the log footers through the converter.
- **`tools/rl_dataset.py` (B.7), logs to parquet.** `convert <pass>
  [--stock <stock pass>]` parses each policy log with numpy straight
  from its bytes and writes `<pass>/dataset/rows/<stem>.<tag>.parquet`
  (the parent's rows and its children's: keys, every work-kind counter
  and its delta since the previous row, the raw state, the 251-entry
  observation vector, the action in force, the action chosen at a
  decision row, the net scores, the stock counterfactual flags, and with
  `--stock` the stock run's counters at the same tick-grid position,
  refused when the two runs' `X_o` differ) and
  `<pass>/dataset/runs.parquet` (one row per run: outcome, censoring,
  budgets, anomaly, peak RSS, the footer's 140 static features as
  `s_<group>_<name>`, family). Three facts of the log layout, checked on
  real logs after the Codex review (2026-09-18), fix the row semantics:
  the solver writes a boundary row and *then* decides, so `is_decision`
  sits on the row whose observation chose the action and `act_taken_*`
  on that row is the next row's action in force; a fork child's first
  row is the parent's row `parent_rows − 1` (the child's epoch counter
  is already one ahead), which is therefore the child's predecessor
  (zero first deltas, `d_cpu_ns` zero by definition since CPU time
  restarts at fork); and pairing with the stock trace is by
  `search_ticks // X_o`, not by the observation count, because one
  search step can cross several grid boundaries and the solver then
  writes one row (brocard has five such rows). `work` is recomputed at
  one `k_res` for every log in the pass (`--k-res`, default 7;
  `work_logged` keeps the printed value), so k=11 traces pair with k=7
  runs without a fake saving; and because every observation entry
  derived from the work clock (the log of work, the search, probing and
  eliminate fractions, the per-pass costs and their windowed deltas, the
  preprocessing work) moves with k, a log written at another k gets
  all its `obs_*` columns rebuilt by the reference implementation
  (`solver/13-kissat-rs/tools/policy_obs.py`) on rows whose work is
  re-priced, a child's after its parent's prefix rows. Checked: at the
  logged k the rebuild reproduces the logged vectors bit for bit (0 of
  545 k values differ on a stock trace, 0 on a fork child replayed
  after 17 parent rows, 2147 without them); at k=7 the Kakuro-easy-115
  search fraction becomes 0.5303 from the logged 0.4399, the value the
  k=7 binary computes. The rebuild costs about 1.3 s per 2000 rows. The
  collector's failure flags travel into the runs table (`failed`) and
  every fitting script skips flagged runs. `MANIFEST.json` keys each job
  on its log, the collector record, the child logs, the family table,
  the stock pairing and the k_res, so a rerun skips only what is
  unchanged. `selftest` converts a synthetic fixture with a parent, a
  child and a stock trace with a skipped grid position and checks every
  delta, label, flag and pairing. About 5.6 MB for a 16 k-row fork
  family.
- **`tools/rl_split.py` (B.11), the validation split.**
  `benchmarks/rl/split_2025.tsv` labels every 2025 cell `train` (293),
  `val` (99) or `shared` (the 8 files also in 2026: never fitted on,
  never used for selection). Deterministic (seed 20260917), stratified
  by family and, inside a family, by the status of the 2026-09-05
  acceptance run (families under 4 cells share one stratum): train has
  SAT 39 % / UNSAT 39 % / TIMEOUT 22 %, val 40 / 39 / 19. Training code
  calls `rl_split.assert_training_only(stems)`, which raises on any
  val, shared or unknown cell.
- **`tools/rl_fit_work.py` (B.5), `tools/rl_cells.py` (B.6),
  `tools/rl_normalize.py` (B.9), `tools/rl_xd_sweep.py` (B.10)** read
  the converted stock pass; their results are in the paragraph below.

**Step B results (2026-09-18).**

- **Stock traces, sat-comp-2025 (B.2).** `python3 tools/rl_collect.py
  stock --suite sat-comp-2025 --name stock2025 --timeout 1800 --jobs 32
  --mem-mb 16000` with the 2026-09-05 acceptance results of both solver
  13 and kissat as status oracles, frozen binary `305c22188e38d5dc`
  (tree f3759bc), idle host, 2026-09-17 23:00 to 2026-09-18 01:41. Run
  dir `log/rl-stock2025-2026-09-17-23-00-35`. 400/400 jobs: 140 SAT,
  143 UNSAT (283 solved), 115 TIMEOUT, and 2 memory aborts under the 16
  GB ulimit (`pj2002_k500`, `17.normalised`, the two cells the
  acceptance run recorded UNKNOWN as well). Zero correctness failures:
  every SAT model verified, every answer agrees with the oracle, no
  anomaly, all 400 logs sealed with their footer and static block;
  953,224 rows, 7.0 GB of logs. Against the acceptance run's 3600 s,
  30 cells solved there time out at 1800 s (17 SAT, 13 UNSAT): the
  rescuable part of the timeout band, whose 115 cells are listed in
  `benchmarks/rl/band_2025.txt` for the 3600 s pass (B.4). Peak RSS:
  median 87 MB, 10 cells above 4 GB, max 13.6 GB (`oisc-subrv-*` and
  the Kakuro cells), which is what caps fork children per parent.
- **Stock traces, sat-comp-2026 (B.3), evaluation only.** The same
  command on the holdout suite with the same frozen k=11 binary, the
  kissat 4.0.4 run of 2026-08-29 (3600 s) as the status oracle and
  `--on-contradiction continue`, 2026-09-18 02:04 to 05:58. Run dir
  `log/rl-stock2026-2026-09-18-02-04-37`. 400/400: 107 SAT, 83 UNSAT
  (190 solved), 210 TIMEOUT; zero correctness failures (every SAT model
  verified, all 190 answers agree with kissat, every log sealed); wall
  PAR-2 821,530; 12 GB of logs; 7 cells kissat solved at 3600 s time out
  at 1800 s. No fitting script reads this directory: it is the paired
  stock reference for the holdout and a feature-range check. Codex
  review builds and single-core conversions ran on the idle cores during
  it, a load of at most a few processes.
- **k_res and the wall weights (B.5, `tools/rl_fit_work.py`,
  `benchmarks/rl/work_fit.json`).** 952,463 observation-epoch rows;
  fitted on the 693,459 rows of the 257 training cells that have rows,
  validation rows as the check; nonnegative least squares without an
  intercept, the top 1 % of |residual| trimmed once. Aggregate fit
  (wall per epoch on ticks, resolutions, kitten ticks, walk steps,
  forward steps): 29.7 ns per tick, 206 ns per resolution, 52 ns per
  kitten tick, 1.5 ns per walk step, 21 ns per forward step, so
  **k_res = 6.9**; train R² 0.72, |residual| / wall 0.33 on train and
  0.32 on validation (a per-epoch wall is noisy: median |residual| 57 ms
  on a 310 ms epoch). With the tick kinds split: search 29.8 ns (81 % of
  all wall), vivify 23, kitten 33, transitive 43, factor 65, substitute
  254, resolution 242 (ratio 8.1 to a search tick), walk 1.4, forward
  19; `probing_ticks`, `backbone_ticks` and `dense_ticks` get weight 0
  because `probing_ticks` is an umbrella containing the probe sub-passes
  (the sub-kinds sum to 114 % of `ticks`) and the other two are tiny; R²
  0.85. Worst families by |residual| / wall: scpc 0.91, x9 0.74,
  crypto-cipher 0.73, gensys 0.70, kakuro 0.68 (median residual +334 ms:
  parse-bound giants), oisc 0.56. **Decision: `K_RES` = 7** (between
  the two fits). The tick-limit tests pass unchanged; the frozen k=11
  binary of the traces is kept for the remaining stock passes (a stock
  pass has no budget, so K changes nothing it does), and every table
  recomputes W at 7.
- **The timeout band at 3600 s (B.4).** The 115 cells stock did not
  solve at 1800 s (`benchmarks/rl/band_2025.txt`; the 2 memory aborts
  left out) rerun with the same command and frozen binary at 3600 s,
  2026-09-18 06:20 to 09:53, run dir `log/rl-band2025-2026-09-18-06-20-44`.
  115/115: 17 SAT, 12 UNSAT (29 solved, walls 1546 to 3400 s), 86
  TIMEOUT; zero correctness failures (every model verified, all 29
  answers agree with the 2026-09-05 acceptance run at 3600 s; one cell
  that run solved UNSAT timed out here, the noise near the limit); 8.6
  GB of logs; peak RSS median 149 MB, max 6.1 GB. So 29 band cells have
  terminal signal at twice the gate's budget, and 86 produce only
  censored rows.
- **Per-cell table (B.6, `tools/rl_cells.py`,
  `benchmarks/rl/cells_2025.tsv`).** Family, split, seed, status,
  `failed`, stock time, W at exit (k = 7), conflicts, rows, decisions at
  X_d = 2^27 (median 70 per run; 97 cells take fewer than 5), W per
  second (median 3.0e7, range 1.1e7 to 6.4e7 on solved cells), B_cell
  (median 2.3e10, max 1.0e11) and peak RSS. `band` marks the 117 cells
  stock did not solve (the 115 timeouts plus the 2 memory aborts); for
  the 115 with a band record the band columns carry the 3600 s status,
  time, W and `B_cell_band` = W at 3600 s (or W × 3600 / t for the 29
  that solved): median 1.08e11, range 2.4e10 to 1.8e11. Rebuilt with
  `--stock STOCK --band BAND`; a run the collector flagged gets no budget.
- **Normalization and the runtime predictor (B.9,
  `tools/rl_normalize.py`).** `benchmarks/rl/obs_norm_2025.json`: mean
  and std of the 251 observation entries over the 693,723 boundary rows
  of the 293 training cells (layout hash `cd89e6212810b8c7`); 17 entries
  are constant on stock traces (`s_valid`, `s_len_hist0`,
  `s_fast_eliminated_per_var`, the lucky outcome and level flags) and get
  std 1. A stock-biased fixture net carrying them loads in the solver at
  `SAT_POLICY_MARGIN=inf` (the acceptance check: layout hash and array
  lengths validated). The predictor: ridge on 119 standardized static
  features (113 actor-tier from the footers, 6 critic-tier) for log10
  stock time on the solved cells, strength by 5-fold CV inside train:
  validation RMSE 1.19 decades against 1.30 for the constant (R² 0.17;
  train 1.06 v 1.32); the largest weights are compression ratio (+0.44),
  log clauses (+0.40), the lucky-level fraction, the BFS failed fraction,
  modularity and the Horn fraction (each about 0.2). A logistic
  solves-within-1800 s classifier reaches 0.77 on validation against
  0.71 for the majority class. Static features carry weak signal; the
  critic's variance reduction has to come from the dynamics.
  `benchmarks/rl/runtime_predictor.json`. The runs table carries the
  collector's failure flags (`premature`, `crash`, `contradiction`,
  `failed`); every fitting script skips flagged runs, and the 2025 pass
  has none (its two memory aborts are honest resource stops).
- **The X_d sweep (B.10, `tools/rl_xd_sweep.py`; decision bead
  `SAT-playground-p9m.12`).** Two measurements, no wall in either. (1)
  From the stock traces alone (392 cells: the 8 shared with the 2026
  holdout are left out of every tuning number): a decision epoch at X_d
  = 2^27 holds, on the median cell, 0.38 probe fires, 0.12 eliminate,
  3.6 reduce, 0.38 rephase, 0.25 reorder and 0.52 mode switches (half
  that at 2^26, double at 2^28), and a run takes a median 69 decisions
  (138 at 2^26, 35 at 2^28; 115 of the 392 cells take fewer than 10 at
  2^27). Decisions are counted as the distinct positions the boundary
  rows occupy on the decision grid, D0 included, which equals the
  solver's own decision counter on all 392 cells at the logged X_d. The
  plan's "about 3 probe fires per epoch" described the two calibration
  cells, not the median: at every X_d an interval multiplier on probe
  or eliminate mostly shifts the next fire, and reduce is the one timer
  that fires several times per epoch. (2) A fork pass on the 20
  discriminating cells, one parent per cell and X_d, branch points for
  the probe, reduce and mode menus at about 20/40/60 % of the run's own
  decisions, budgets min(B_cell, 2e9) at k=7 on the k=7 binary
  (`9defe8586fcdeadf`), tick-deterministic
  (`log/rl-xdsweep2-2026-09-18-05-59-06`, 21 min, 60 parents, 422
  children, all siblings agree, every SAT model verified; a first pass
  with the branch points scaled by B_cell instead of the run's work,
  `log/rl-xdsweep-2026-09-18-01-43-23`, gave the same picture). A child
  that holds one knob entry for one decision epoch and then returns to
  stock diverges from its parent in 88 to 98 % of cases (no-ops: 19 of
  158 at 2^26, 9 of 144 at 2^27, 3 of 120 at 2^28); among the diverged
  pairs whose parent solved, the child loses the solve within the budget
  in 38 %, 35 % and 36 % of cases (28 of 74, 23 of 65, 17 of 47) and
  wins one in 4, 3 and 4; among the pairs where both solved, the work
  clock at the end differs by a median 11 %, 22 % and 7 % and a 90th
  percentile 1.2× at all three epochs (about half of the diverged pairs
  end at the budget on both sides, where the work is equal by
  construction and the conflict counts differ by 2 % at the median, so
  the all-pairs figure the report prints understates the deviation
  unevenly across epochs). So neither the outcome effect nor the size
  of a one-epoch deviation separates the three epochs; only the number
  of labelled states per run does, halving with each doubling.
  Recommendation, recorded in the decision bead: freeze X_d = 2^27, the
  default.
- **Critic-tier static features (B.8).** `benchmarks/rl/static_features.tsv`,
  800 rows, no errors, 34 minutes on 4 cores (the slowest cell 570 s,
  `17.normalised`). Compression ratio 0.006 to 0.40 (median 0.095),
  modularity −0.50 to 0.97 (median 0.48), treewidth bound 1 to 400 with
  278 cells at the cap; no cell has a comment line before its `p` line
  (the benchmark database strips them), so the header fingerprint is one
  constant value and carries nothing.

**RL scheduler steps C and D (2026-09-18; plan §5.3-5.4, §6.1-6.4, §10).**
Decisions first: X_d stays 2^27 and the stage-1 menu is frozen as plan
§2.1 with nothing pruned (plan §11, beads `SAT-playground-p9m.12/.13`).

- **Round-0 schedule (C.1, `tools/rl_round0.py`,
  `benchmarks/rl/round0_jobs.tsv` + `.schedule.tsv`).** One stock parent
  per training cell carries every branch point; a point forks the whole
  menu of one knob (4 children for a 5-entry knob, 2 for mode and the
  margin, 3 for sweep effort), each child holding its entry for one
  decision epoch and then running stock to the cell's work budget. The
  points come from the cell's stock trace: half are "timer-due" (the
  branched knob's timer fires within the coming epoch, so the 2× and 4×
  entries act), half uniform over the window; knobs are drawn with the
  step-0 per-knob oracle gains as weights (reduce 16.0 ... sweep 8.4);
  rephase is only placed in stable mode and the margin only in focused
  mode, where the solver would not mask them. Hard-cell mix (owner's
  choice, costed in plan §11): cells solved in 60-1800 s get decisions / 8
  points (4-24) at B_cell; the 85 train band cells run only at
  B_cell_band, 16 points on the 19 stock solves at 3600 s and 8 late
  points (30-90 %) on the 66 timeouts; cells under 60 s at most 3 points;
  one segmented (temperature 1, mean segment 5 epochs) and one jitter
  run (temperature 0.5) per cell. 28 of the 293 training cells have no
  decision at all (solved before the first observation epoch) and get
  nothing. Totals: 264 cells, 2274 points (1097 timer-due), 7808
  children (per cell: fast median 8, slow 31, band solvers 54, band
  timeouts 28), plus 528 wild runs; points per knob reduce 368, mode
  356, eliminate 321, probe 333, reorder 315, rephase 204, margin 199,
  sweep 178. Projected 3185 core-hours: the plan's "about 2 days" assumed
  16 uniform points on every cell, and a child costs the budget left
  after its branch point (about 0.8 B_cell on a solved cell, since the
  parent ends at a third of it). Two-cell rehearsal through the
  collector: `log/rl-round0test-2026-09-18-17-14-42`, 6 jobs, all
  siblings agree, every model verified, converted with `--stock` pairing.
- **Round 0 (C.2 + C.3 in one pass), done 2026-09-22.**
  `python3 tools/rl_collect.py table benchmarks/rl/round0_jobs.tsv --suite
  sat-comp-2025 --name round0 --jobs 28 --mem-mb 16000 --mem-total-mb
  420000 --oracle <the two 2026-09-05 acceptance results>`, started
  2026-09-18 17:23, run dir `log/rl-round0-2026-09-18-17-23-27`
  (collector pid 2511078, its stdout in
  `log/round0-collector-2026-09-18.out`), frozen binary
  `9defe8586fcdeadf` (k_res 7, the X_d-sweep binary), 792 jobs, 6 live
  children per parent (`SAT_POLICY_BRANCH_JOBS=6`, so a fork job holds 7
  of the 28 slots and four parents run at once), cores 0-13 and 18-31.
  Projected 5.5 days at 28 slots; it took 3 d 15 h (ended 2026-09-22
  08:14), 181 GB of logs. **Result:** 792/792 jobs, sibling agreement
  100 % (all 264 parents agree with every child), 7808 children expected
  and forked, none missing, every child ended on its own budget or
  answer (2193 SAT, every model verified; 3179 UNSAT; 2436 UNKNOWN at
  the budget); parents 98 SAT / 99 UNSAT / 67 UNKNOWN; wild runs 160
  SAT / 192 UNSAT / 175 UNKNOWN. Zero correctness failures. One anomaly:
  the jitter run of `oisc-subrv-and-nested-12` hit its wall cap (2× the
  stock wall + 600 s) at 70 % of its budget, running at 250 ns per work
  unit against stock's 87 on that memory-bound 4.7 GB cell under the
  28-process load (28 % of its ticks in kitten); the wild runs' caps are
  tight on such cells, the fork parents' caps carry a (1 + waves) factor
  and none fired. Conversion (C.4) by `log/round0-convert-2026-09-22.sh`:
  the band cells paired with the band pass, the rest with the stock pass.
- **Round-0 dataset (C.4, 2026-09-23; `tools/rl_round0_report.py`,
  `benchmarks/rl/round0_children.tsv` one row per child with its sibling
  set and label, `round0_knobs.tsv` the effect sizes).** Conversion:
  `log/round0-convert-2026-09-22.sh` (12 minutes for 181 GB of logs,
  7.6 GB of parquet, 8600 runs; band cells paired with the band pass).
  2274 sibling sets: 1763 labeled (some member solved), 511 all-censored,
  of which 494 are on the 66 band timeouts (63 of their 1804 children
  solved). Per knob, the share of ordered pairs where a one-epoch
  deviation moved the outcome or the total work by over 1 %: reduce 87,
  margin 83, rephase 75, mode 69, probe 66, eliminate 60, reorder 48,
  sweep 9 %; children beat the parent's continuation on 20-35 % of
  ordered pairs and lose on 25-55 % (621 solves lost, 75 gained); rescue
  rates on parent-unsolved sets: probe 7.6 %, reduce 5.3 %, rephase 4.1
  %, reorder 3.0 %, eliminate 2.7 %, mode 2.2 %, margin 2.0 %, sweep 0.
  The delay entries act at timer-due points and are mostly no-ops at
  uniform ones (probe 2×: 90 % v 37 % moved), which is what the
  schedule's due half was for. Flavours: a segmented or jitter run
  changes the outcome of 62-72 % of runs (work moved by a median 0.11-
  0.16 log ratio) against 49 % (0.025) for a fork child at similar cost
  per run; the fork children are the same-state counterfactuals the
  ranking learner needs, the wild runs a side line. Plan §11 (2026-09-23)
  has the triage reading.
- **Per-knob rankers (E.1, 2026-09-24; `tools/rl/rank.py`,
  `benchmarks/rl/round0_rankers.tsv`, `round0_importance.tsv`).** On the
  round-0 sibling sets, the state being the parent's observation at the
  branch decision (cached as `dataset/branch_states.npz`), 5-fold
  cross-validation grouped by cell inside the training split; per knob
  the stock-always reference, a bias-only entry order, per-entry linear
  rankers at five L2 strengths and xgboost rank:pairwise at three sizes
  (trained on the same ordered pairs as the linear model, one two-item
  query per pair; a tied score gets half credit for every model, as in
  the stock-always reference). Held-out pairwise accuracy, best of each
  family:

  | knob | sets / pairs | stock-always | bias-only | linear | xgboost |
  |---|---:|---:|---:|---:|---:|
  | probe | 253 / 1713 | 53.2 | 58.2 | 54.8 | 54.4 |
  | eliminate | 208 / 1351 | 55.5 | 55.8 | 50.6 | 52.8 |
  | reduce | 274 / 2162 | 55.6 | 55.2 | 55.1 | 54.8 |
  | rephase | 151 / 1026 | 53.1 | 49.3 | 51.9 | 51.9 |
  | **reorder** | 201 / 1247 | 53.2 | 48.2 | **57.4** | **56.8** |
  | mode | 220 / 566 | 56.0 | 53.9 | 55.3 | 52.5 |
  | margin | 141 / 368 | 59.2 | 60.6 | 54.6 | 62.8 |

  Sweep effort has 18 labeled sets and is skipped. Another fold draw
  (the first run, before the folds were tied to the seed and the knob
  alone) moved single numbers by up to three points, which is the noise
  of 100-cell folds; reorder is the one knob above both references in
  every draw. Acting greedily at margin 1 on held-out states, the linear
  rankers move on 0-6 states per knob except reorder (99 states at
  L2 = 0.01: 24 better, 26 worse); the trees never clear the margin. The
  bootstrap ensembles (10 members per knob, in the run's
  `dataset/rankers_ensemble.npz`) disagree on an alternative's gap over
  stock by about as much as the gap. The pooled importance is flat (top
  input 1 % of the weight). **At round-0 size the outcome of a one-epoch
  deviation is not predictable from the state**, except a small signal on
  reorder; plan §11 (2026-09-24) lists the options for the go/no-go. The
  round-1 parent, `benchmarks/rl/round0_rankers.net.bin` (sha256
  `0c8dfd57c2211ddf...`), was exported by an earlier linear-only run in
  which every knob's chosen strength was L2 = 1.0; it is kept as the file
  round 1 ran, and the next round's parent is re-exported from rounds 0
  and 1 together.
- **Round 1 (E.5, started 2026-09-24; `tools/rl_round1.py`).** The plan's
  DAgger loop: the round-0 linear rankers exported as the parent
  (`tools/rl/rank.py --export`, `benchmarks/rl/round0_rankers.net.bin`,
  kind-1 heads on the standardized input, 38 KB; it loads, round-trips
  bit-exact and at margin 0.5 moves off stock on 5 of 17 decisions of a
  test cell, 0 at margin 1). Cells that yield labels only
  (`benchmarks/rl/round1_cells.tsv`, 157): the 118 training cells stock
  solves in 60-1800 s at B_cell, the 19 band solvers at B_cell_band, the
  12 band timeouts a **7200 s stock probe** solved
  (`log/rl-band7200-2026-09-24-10-01-29`: 17 of the 86 band timeouts
  solve at twice the band budget, 12 train and 5 validation, walls
  3302-6955 s; 69 do not) at 1.5× the probe's work at the solve, and the 8
  remaining timeouts a round-0 child or wild run solved at 1.5× the band
  budget with late points. The 52 timeouts nothing ever solved leave
  fork collection: a child costs the budget left after its fork, so a
  longer budget doubles the cost there for no label. Steps: the parent's
  dry run on every cell (`benchmarks/rl/round1_dryrun_jobs.tsv`,
  `log/rl-r1dry-2026-09-24-16-44-58`, 32 slots, about 93 core-hours),
  its conversion, the active schedule (per decision and knob the
  round-0 ensembles' disagreement, the spread across members of each
  alternative's gap over stock, times the stakes, 1 when the knob's
  timer fires in the coming epoch else 0.2; three quarters of a cell's
  points are active and shared equally among the knobs, each taking its
  highest-scoring free decisions, since the ensembles' regularization
  differs and raw disagreement is not comparable across knobs, and the
  last quarter is random; knobs reorder, reduce, probe, rephase, mode;
  twice round 0's points per cell), then the fork pass with the same
  parent at 28 slots. The dry run finished 2026-09-24 18:57 (157/157: 58
  SAT, 73 UNSAT, 26 at the budget, no failure). The first schedule at
  twice round 0's points projected 9.1 days and was held by the queue's
  5-day guard; with the point targets trimmed (slow cells decisions/7 in
  8-32, band solvers 20, probe-solved 12, rescuable 16) the pass is 154
  cells, 2333 points (reduce 499, mode 491, probe 484, reorder 476,
  rephase 383), 8350 children, 3601 core-hours, **6.3 projected days at
  28 slots** (round 0 ran at 0.65 of its projection, so about 4 real
  days). Launched 2026-09-24 19:01: `log/rl-round1-2026-09-24-19-01-06`,
  frozen binary `9defe8586fcdeadf`, the parent's weights checksum carried
  in every job's environment. The committed `benchmarks/rl/round1_jobs.tsv`
  is the table the pass runs (checked against the run's own copy); the
  round-0 ensemble it was drawn from was regenerated afterwards (folds
  and bootstrap on separate streams), so a fresh draw differs in its
  points, and the launched table is the record. **Done 2026-09-28
  00:14 (3 d 5 h)**: 154/154 parents (55 SAT, 73 UNSAT, 26 at the
  budget), 8350 children (2067 SAT, every model verified; 4048 UNSAT;
  2235 at the budget), zero failures, zero anomalies. Labels
  (`tools/rl_round0_report.py` against the round-1 schedule;
  `benchmarks/rl/round1_children.tsv`, `round1_knobs.tsv`): 2333 sibling
  sets, 1882 labeled (81 %, against 78 % in round 0; the rescuable
  timeouts 68 of 128, the probe-solved 128 of 144). A child beats the
  parent on 27-40 % of ordered pairs and loses on 36-47 %, as in round 0;
  rescues probe 76 of 456 parent-unsolved sets, reduce 57 of 536,
  rephase 46 of 372, reorder 43 of 476, mode 32 of 242. **The active
  selection worked as a selector of informative states**: a deviation
  moved the outcome on 83 % of the ordered pairs at actively chosen
  points against 55 % at the random quarter. The parent left stock at
  1-2 % of the branch points (rephase at 0.5 on 43 sets, the rest a
  handful), so round 1's states are close to stock's. **Retraining on
  rounds 0 + 1** (`tools/rl/rank.py` on both passes;
  `benchmarks/rl/round01_rankers.tsv`, `round01_importance.tsv`,
  `round01_rankers.net.bin`; 16,158 children, 4607 branch-point states,
  the same cell-grouped 5-fold cross-validation): held-out pairwise
  accuracy, stock-always / bias-only / best linear / best xgboost:

  | knob | sets, pairs (round 0 → 0 + 1) | stock-always | bias-only | linear | xgboost |
  |---|---:|---:|---:|---:|---:|
  | probe | 253 → 641, 4499 | 51.4 | 53.9 | 53.5 | 53.7 |
  | reduce | 274 → 664, 5201 | 54.1 | 55.2 | 54.4 | 54.8 |
  | rephase | 151 → 438, 3009 | 52.2 | 51.3 | 53.3 | 51.7 |
  | reorder | 201 → 535, 3439 | 53.1 | 52.6 | 53.8 | 54.8 |
  | mode | 220 → 579, 1497 | 53.6 | 53.2 | 54.6 | 55.4 |

  Eliminate, margin and sweep had no round-1 points and are unchanged.
  **With 2.5-3× the labels, no state-based ranker beats the state-free
  references beyond noise on any knob, and reorder's round-0 lead (57.4
  v 53.2) shrank to 53.8 v 53.1.** The only ranker acting at margin 1 on
  more than a handful of held-out states is rephase at the weakest
  regularization (122 states: 56 better, 44 worse, not significant).
  Plan §11 (2026-09-28) records this as the §9 decision point.
- **Hold-length pilot (E.6, `SAT-playground-p9m.10.14`, 2026-09-28; `SAT_POLICY_BRANCH_HOLD`,
  `tools/rl_hold_pilot.py`, `benchmarks/rl/holdpilot_jobs.tsv`).** After
  round 1, the one plan option the data supports is a deviation held
  for several epochs. Fork mode gained `SAT_POLICY_BRANCH_HOLD=K`
  (environment table above; `tests/policy_fork.rs`: a hold-3 child keeps
  a 2× reduce entry for the branch row and three epochs of rows, a
  hold-1 child for the branch epoch only, the parents are identical, bad
  values exit 1; the full suite and the smoke test pass on the rebuilt
  binary `9e11c2b6e9f65485`). The pilot: 20 training cells, one per
  family (30-263 decisions, B_cell ≤ 6e10), two nearby branch points per
  knob for probe, reduce and mode at 30/34, 50/54 and 70/74 % of the
  stock run's decisions, the same points at holds 1, 4 and 16: 60 fork
  jobs, 1200 children. Readout per hold: effect size (moved share, beat
  and lose rates, work-ratio spread) and the agreement of the label
  between the two nearby points of one cell and knob, printed with its
  chance level (the same agreement after re-pairing the second point's
  children across the cells of the same knob, 2000 re-pairings, which
  keeps how often the entry helps or hurts at that hold) and the excess
  over it; a hold that hurts more often raises raw agreement by itself,
  so the excess is the readout. The report's last section separates the
  entry's consistency from the parent's (every child is compared with
  one parent continuation, a single draw): the same agreement between
  different entries and between entries on opposite sides of stock
  shows the parent's part; the entry's own part is the ordering of
  sibling pairs (a solve beats a timeout, else the lower work by more
  than 1 %) compared between the two points, against relabelings of the
  entries within the cell.
  **Result (2026-09-28, `log/rl-holdpilot-2026-09-28-14-21-33`, 14:21
  to 19:06, zero failures and anomalies, every parent solved;
  `log/holdpilot-report-2026-09-28.txt`):**

  | hold | moved | better / worse | \|log W ratio\| median | same entry, same sign: excess over chance | different entries | sibling pair order (the entry's own) |
  |---:|---:|---:|---:|---:|---:|---:|
  | 1 | 67 % | 33 / 34 % | 0.026 | +22 | +24 | −1 (p 0.53) |
  | 4 | 86 % | 41 / 46 % | 0.051 | +18 | +17 | +3 (p 0.28) |
  | 16 | 89 % | 36 / 52 % | 0.058 | +25 | +16 | **+9 (p 0.03)** |

  The raw agreement is the parent's luck: entries on opposite sides of
  stock agree as often as the same entry (+24 / +18 / +14), and per cell
  the mean log ratio of all children runs from −0.43 (jkkk, an unlucky
  parent) to +0.54 (ER_400, a lucky one) at hold 16. Free of that, a
  deviation held for one or four epochs has no repeatable preference
  among the entries, and one held for sixteen epochs has a small one
  (59 v 50 % of sibling pairs ordered the same way at the two points;
  probe +5, reduce +11, mode +17; runs under 72 decisions +6, runs of 72
  or more +11). Synthetic checks (independent labels, a per-cell offset,
  a per-entry direction, a skewed sign balance) give the expected four
  signatures. Reading, the design caveat (the two points' held windows
  overlap at hold 16), options and the recommendation: plan §11,
  2026-09-28.
- **Hold-length pilot 2, longer holds with the points a hold apart (E.6b,
  `SAT-playground-p9m.10.16`, 2026-09-28; `benchmarks/rl/holdpilot2_jobs.tsv`).**
  Owner's option (a) after the first pilot. `tools/rl_hold_pilot.py make
  --design apart`: the 28 training cells stock solves with 150+
  decisions (153-524, 8-30 min, at most four per family, 18 families),
  the first point per knob at 20 / 30 / 40 % of the decisions (probe,
  reduce, mode) and the second point one hold plus four epochs later, so
  the held windows never overlap, at holds 16, 32 and 64: 84 fork jobs,
  20 children each, seven live children per parent on 32 slots. The
  first pilot's recipe (`--design nearby`, the default) still reproduces
  its committed table byte for byte. Same report, same readout (the
  sibling pair order agreement). The 400-cell check of baseline 2 that
  was running (`SAT-playground-p9m.10.15`) was stopped for it after
  thirty minutes, no results.
  **Result (`log/rl-holdpilot2-2026-09-28-23-35-39`, 2026-09-28 23:35 to
  2026-09-29 22:58 with a host reboot in between, resumed with
  `rl_collect.py resume`; 1680 children, zero failures and anomalies;
  `log/holdpilot2-report-2026-09-29.txt`):**

  | hold | moved | better / worse | unsolved children | same entry, same sign: excess over chance | opposite-side entries | sibling pair order (the entry's own) |
  |---:|---:|---:|---:|---:|---:|---:|
  | 16 | 95 % | 37 / 57 % | 113 / 560 | +17 | +16 | +3 (p 0.25) |
  | 32 | 95 % | 39 / 55 % | 111 / 560 | +12 | +9 | +7 (p 0.04) |
  | 64 | 93 % | 36 / 56 % | 113 / 560 | +14 | +9 | +6 (p 0.05) |

  With the windows apart, the entry's own consistency is 53-57 % against
  50 % by chance at every hold (the first pilot's hold-16 figure of 59 %
  had overlapping windows), far under the 65 % bar; the raw agreement is
  the parent's luck again. The only pocket is hold 64 on runs under 266
  decisions (+16, p < 0.001), a quarter to a half of the run, the
  per-instance-constant regime baseline 3 covers. A held entry of any
  length mostly hurts: 55-57 % lose and one child in five fails to
  solve within the 3× budget. **The epoch-policy line closes on two
  pilots; baseline 2 goes through its gates** (plan §11, 2026-09-29).
- **Within-run feedback at regime scale (check 1 of the regime-selection
  idea, 2026-09-30; `tools/rl_early_signal.py`, `log/early-signal-2026-09-30.txt`).**
  On pilot 2's children: the child's state k epochs after the fork minus
  the parent's row nearest in work, under cell-grouped CV, predicts
  whether the child ends worse than its parent no better than the
  majority rate (k 2-16, holds 16-64), and which of two siblings wins at
  50 % or less at hold 16, 53-55 % at hold 32 and 60 % at hold 64 read
  two epochs in (per knob at k = 8: reduce 59-61 % at holds 32 and 64,
  mode 66 % at hold 64 on 96 pairs, probe 47-50 %). Early units are the
  one strong signal (27 % worse v 59 %) and rare. Weak feedback for a
  regime selector to beat kissat's blind alternation; plan §11,
  2026-09-30.
- **Regime experiment (E.14, `SAT-playground-p9m.10.17`, 2026-10-03;
  `SAT_POLICY_BRANCH_REGIMES`, `tools/rl_regime.py`, `benchmarks/rl/regime_jobs.tsv`).**
  The question one level above timing: kissat alternates two search
  regimes blindly, about half the ticks each (48-53 % stable on the
  pilot-2 cells, 49-142 switches per run, a stint of 1-4 decision
  epochs); would another regime, chosen per instance, have paid? Fork
  mode gained regime points (environment table above): a child runs one
  regime in place of kissat's own choice for its hold and then hands
  back to kissat's alternation with normal-size stints. `mode.rs` is not
  edited: a held mode acts through `policy::effective_limit` and the
  switch is kissat's own; the dial regimes set four options and put them
  back. `tests/policy_fork.rs` has five regime tests (the mode held row
  by row and the size of the handover stint, a regime at decision 0, the
  options in force and restored, answers agreeing, bad settings); the
  suite (102 tests, release and debug builds), the smoke test and both
  parity runs (20 of 20, policy off and policy-on stock) pass on the
  binary with regimes. The pass: 95 training cells
  (46 slow solves, 19 band cells that solve at the band budget, 30 that
  do not), one point per cell at 30 % of the run, 14 children: four
  regimes for one kissat stint (the owner's design), five for a block of
  a quarter of the run, five rerolls (kissat unchanged on another random
  stream: the luck control). The report compares the best regime per
  cell with the best of as many rerolls, with a within-cell relabeling p
  value; design and rule in plan §11, 2026-10-03.
  **Result (`log/rl-regime-2026-10-03-14-19-53`, 2026-10-03 14:19 to
  2026-10-04 15:23, 1330 children, zero failures and anomalies;
  `log/regime-report-2026-10-04.txt`).** A child's fair baseline is a
  reroll, not its parent (parents were picked by outcome): stock solves
  64 of 95, one reroll 56.0, the best of five rerolls or the parent 71.

  | window | cells | solved, regime oracle v luck oracle | tick PAR-2 ratio |
  |---|---|---:|---:|
  | one stint | all 95 | 74 v 70.4 (p 0.06) | 0.918 (p 0.025) |
  | one stint | 65 stock solves | 65 v 65 | 1.07 slow (p 0.98), 0.98 band (p 0.39) |
  | one stint | 30 stock does not | 9 v 5.4 (p 0.06) | 0.876 (p 0.013) |
  | block | all 95 | 74 v 71 (p 0.15) | 0.965 (p 0.22) |
  | block | 65 stock solves | 65 v 65 | 1.01 slow (p 0.65), 1.09 band (p 0.89) |
  | block | 30 stock does not | 9 v 6 (p 0.15) | 0.932 (p 0.14) |

  No alpha on the cells stock solves. On the cells it does not, the
  matched-count gaps close when pooled: 13 of 31 unsolved parents are
  rescued by some regime child (nine children), 11.7 by nine children
  drawn by relabeling (p 0.22), and per child a regime rescues as often
  as a reroll (11.5 v 11.0 %). As constants against a reroll, `focused`
  for a quarter of the run loses 6 solved, `stable` for a quarter is the
  best arm (+5, p 0.06), `eager` for one stint +4 (p 0.13). A hint, not
  alpha; what is established is that focused-only stretches hurt and
  that trajectory luck is large. Options and the reading: plan §11,
  2026-10-04.
- **Baseline 2 on all 400 cells (`SAT-playground-p9m.10.15`, 2026-10-05;
  `log/abtest-rl-b2-400-2026-10-04-17-52-54`, `log/b2-400-report-2026-10-04.txt`).**
  `reorderint=20000` against stock at 1800 s, 16 GB, 32 shared pinned
  cores, one seed, proofs checked, idle host: no failed row, no
  contradiction (44 UNSAT proofs left unchecked at the checker's budget,
  on 24 cells, with no disagreement between the arms on any).

  | cells | solved, stock v 20000 | +solved / −solved | wall PAR-2 | tick PAR-2 |
  |---|---:|---:|---:|---:|
  | all 400 | 281 v 288 | +18 / −11 | 0.967× | 0.982× |
  | 293 train | 206 v 209 | +12 / −9 | 0.987× | 1.005× |
  | 99 validation | 70 v 73 | +5 / −2 | 0.927× | 0.936× |

  The sign holds everywhere and matches the medium suite and the
  tick-budgeted validation run, but it does not clear the noise: 29
  cells change side, 18 for and 11 against (sign test p 0.13), and the
  median work ratio on the 270 cells both solve is 1.000. A changed
  constant rerolls every cell, and the regime experiment showed how
  large that luck is, so this is a candidate retune one seed cannot
  confirm. Next for a promotion: both arms on several scrambled copies
  (plan §8); the 2026 holdout stays unspent. Plan §11, 2026-10-05.
  `python3 tools/rl_sweep_report.py log/abtest-rl-b2-400-2026-10-04-17-52-54
  --baseline stock --split benchmarks/rl/split_2025.tsv`.
- **Structure census of the 400 cells (`SAT-playground-p9m.20`, 2026-10-06;
  `tools/rl_structure.py`, `benchmarks/rl/structure_2025.tsv`,
  `log/structure-report-2026-10-06.txt`, `log/structure-verify-2026-10-06.tsv`).**
  What kind of structure each instance carries, from three sources: cheap
  detectors on the CNF (XOR constraints, at-most-one and exactly-one
  groups, time-step unrolling, uniform-random shape; the first 3M clauses
  on 79 big cells), the solver's own gate census from the stock pass, and
  Satsuma 1.4 (the symmetry preprocessor of the SAT Competition 2026
  winner, built from source) in `fix` mode on 378 cells (18 skipped over
  60 MB compressed, 4 timed out at 120 s). Costs: the detectors take a
  median 2.5 s per cell (p90 30 s, max 241 s, Python), Satsuma a median
  0.6 s (p90 20 s, max 105 s). The stock pass at 1800 s solves 283 of the
  400; 115 time out.

  | structure | all 400 | of the 115 unsolved | unsolved families |
  |---|---:|---:|---|
  | any symmetry (generators or interchangeable rows) | 173 | 65 | roundrobin 12, lockchart 11, clqcl 6, ramsey 6, timetable 5 |
  | interchangeable rows (orbitopes) | 55 | 33 | roundrobin 12, clqcl 6, php 4, clique-coloring 4, ramsey 4 |
  | decided by symmetry fixing alone | 37 | 26 | roundrobin 12, clqcl 6, php 4, clique-coloring 3, tseitin 1 |
  | at-most-one / exactly-one groups | 131 | 50 | roundrobin, clqcl, clique-coloring, php, timetable, argumentation |
  | XOR constraints (10 or more) | 130 | 27 | bp 4, tseitin 4, multiplier-16x16 3, oisc 3 |
  | half or more of the clauses in XOR constraints | 23 | 7 | tseitin 4, grs 1, parity 1, one singleton |
  | nearly every clause in an XOR constraint (98 %+) | 5 | 4 | tseitin 4 |
  | gates on 30 % of the variables | 235 | 62 | oddball 8, roundrobin 8, multipliers, bp |
  | time-step unrolling (copies of a clause shape at one period) | 53 | 10 | mult-miter-bits 2, tseitin 2, xor-op 2 |
  | nothing recognised | 30 | 7 | chess-puzzles 3, baseballcover, connm-csp, dubois50-mis |

  **The finding.** Symmetry fixing alone decides 37 cells in 0.0 to 4.7 s
  each, 26 of them cells we time out on at 1800 s, and 25 of the 37 are
  still unsolved by both solver 13 and the C kissat at 3600 s in the
  third paired run. VeriPB 3 checks Satsuma's proofs (the `verify`
  command; Satsuma writes VeriPB 2, the rewrite only renames and drops
  deletions, which cannot make a refutation unsound) and verifies 33 of
  the 37, 22 of them unsolved cells (roundrobin 8, clqcl 6, php 4,
  clique-coloring 3, tseitin 1). The four `MVRoundRobin` proofs are
  rejected at a redundance step; the 2026 winner solved these with the
  same tool, but they stay claimed, not verified, here. On top of that,
  all five Tseitin cells (four unsolved) are pure XOR systems with every
  variable in exactly two constraints, so adding every constraint gives
  0 = 1: UNSAT in under 7 s of Python with no search at all. Together
  that is 25 verified unsolved cells (22 by symmetry, 3 more by one XOR
  sum) that a seconds-long rewrite decides, against the +7 of the best
  search-side candidate. All are crafted UNSAT families (pigeonhole,
  clique-colouring, round-robin, Tseitin), which have no short resolution
  proofs: these answers cannot come with a DRAT proof, so the correctness
  gate would have to accept SR or VeriPB proofs for them. The 2026 holdout
  has none of the family names, so a gain there is not implied.

  **Patterns across families.** The 47 one- and two-cell families almost
  all share a signature with a larger family: the arithmetic singletons
  (aprove, jkkk, manthey, spg, lec-mult, linked-list, valves, brocard,
  circuit-multiplier) look like multiplier-16x16 / bp / sort-equivalence
  (XOR gates inside a circuit); battleship, crafted-n, dimacs-coloring and
  mod4block look like break-scheduling / timetable / sudoku (at-most-one
  groups with symmetry); goldcrest like bp and oisc; the hardware
  singletons like oddball / pj / iscas / itc99; rbsat, frb, sgi, snw and
  the planning cell like case / crusti-g2io (exactly-one groups). Twelve
  cells have a signature no larger family shares (the two goldcrest and
  the two mult-miter-bits cells, uniqinv40, myciel6, g2-T49, em_11,
  gensys, shuffling-sat04, velev-pipe-sat, one xits-miter). The unsolved
  "nothing recognised" cells are three rook puzzles, baseballcover,
  connm-csp, dubois50-mis and Ptn-7824. Plan §11, 2026-10-06.
- **LymphoSAT's specialists on our cells (2026-10-06; `tools/lymphosat_probe.py`,
  `log/lymphosat-2026-10-06/` with the registry JSON, `matrix.tsv`,
  `claims.tsv`, `report.txt`).** LymphoSAT (SAT Competition 2026) is 126
  family specialists, each a detector plus a method for one problem
  family, with kissat as the fallback; the registry page publishes every
  specialist's source. 116 of the 119 C++ ones compiled (two need data
  files that were not published, one needs an AVX-512 extension this
  host lacks) and ran with the 7 Python ones on the 382 cells under 60 MB
  compressed, 10 s and 8 GB each: 46,986 runs, 13,776 timeouts, 307
  crashes, 950 answers. Every answer on a cell our stock pass did not
  solve was re-run with a proof file and checked: SAT models against
  every clause, UNSAT proofs with VeriPB.

  *What the detectors look for* that our census did not: finite-domain
  CSPs in direct encoding (one-hot or binary-coded value blocks plus
  negative binary clauses as forbidden pairs), graphs recovered from the
  binary clauses and solved as graphs (colouring, cliques, matchings,
  tours, independent sets, treewidth), Hall-type counting arguments with
  cutting-planes proofs (pigeonhole variants, clique-colouring, rooks,
  mutilated chessboard), bit-blasted arithmetic read back from unit
  clauses and comparator or adder gadgets and solved with number theory
  (factoring, discrete log, subset sum, sums of cubes), Tseitin or
  lookup-table circuits rebuilt and simulated instead of searched,
  complete truth-table blocks turned into GF(2) or GF(3) equations,
  cardinality encodings (Sinz counters, totalizers) decoded back to
  their bounds, generator fingerprints (the header's variable count as a
  formula of the parameters, exact clause counts, regenerating the
  expected clause stream), random-formula signatures (ratio, balanced
  occurrences, a planted solution in the first clauses' signs), ordering
  and transitivity blocks, time-step planning encodings, and stored
  witnesses (a 13,000-term discrepancy sequence, known tours, schedules).
  The `c detected` lines are not reliable (several specialists print one
  on every input), so only an answer counts.

  *Soundness.* Three specialists answered cells with a known status
  wrongly (random-planted-solution 44 of 75 answers, station-repacking
  4 of 16, phnf 1 of 14) and are dropped from every count; LymphoSAT
  itself only routes to a specialist whose detector fires, so this is
  what running them outside their family looks like. Seventeen more are
  general solvers in disguise (a from-scratch CDCL or local search that
  answered 15 or more cells over 8 or more families, all easy).

  *What matched.* A specialist for the same problem (the tool carries
  the name map) answered cells in: oddball (10 of 20, eight of them our
  timeouts, SAT models in 2-4 s), clqcl 6 of 6, clique-coloring 4 of 4,
  php 4 of 4, roundrobin 4 of 12 (the MV cells, by the pigeonhole
  specialist), tseitin 3 of 5, xor-op 3 of 3, ramsey 2 of 6,
  chess-puzzles 4 of 4, kakuro 4 of 4, frb 2 of 2, battleship, em, sted,
  gp (graceful production, 4 of 7), bits-fast, reconf, et-tc, ncc,
  sum-of-cubes, ktf, fsf, factoring, and the singletons dislog, 16_2 (a
  Simon cipher) and Ptn-7824 (Pythagorean triples). The lock-chart
  specialist answered none of our 12 lockchart cells. No targeted answer at all for bp 20, multiplier-16x16 12,
  oski 12, timetable 12, crusti-g2io 9, itc99 9, hwmc-step-transition 8,
  sort-equivalence 7, pj 6, div-sqrt-miter 5, grs 5, scpc 5, sudoku 5
  and the rest of the industrial, hardware and arithmetic cells, where
  LymphoSAT is plain kissat.

  *The count.* 41 of our 115 unsolved cells got a verified answer from a
  specialist within 10 s: oddball 8 (SAT), clqcl 6, clique-coloring 4,
  MVRoundRobin 4 (this settles the four Satsuma proofs VeriPB rejected),
  php 4, tseitin 2, xor-op 2, ramsey 2, 16_2, Ptn-7824, fermat,
  Kakuro-132, frb80 and mchess_20 from a specialist for the same problem
  (38), plus lockchart 2 and mp1-Nb7T45 from the general local search
  that other specialists carry as a fallback (a search result, not a
  structural one). The three rook cells are claimed only (the proof is
  a bare assertion). With the census (22 by symmetry fixing, 4 Tseitin
  by one XOR sum) the union is **48 of the 115 unsolved cells decided by
  a seconds-long structural method with a checked answer** (51 counting
  the three fallback-search cells); 23 come only from LymphoSAT's
  structure, 10 only from the census (eight plain RoundRobin cells its
  pigeonhole detector does not recognise, two Tseitin cells too big for
  10 s). All are crafted families; the SAT side (oddball,
  puzzles, ciphers) comes from decoding the encoding and solving the
  original problem, the UNSAT side from counting, parity and symmetry
  proofs that have no short DRAT form. Plan §11, 2026-10-06.
- **Training scaffold and the stock clone (D.2, `tools/rl/`).** PyTorch
  2.14 CPU in the RL venv (`tools/rl/requirements.txt` has the index
  line). `data.py` loads the decision rows of a converted pass as a
  float32 matrix in the normalization's entry order (train, val or all,
  never the 8 shared cells; `assert_training_only` on train), `model.py`
  is the net (251 → 128 → 64 ReLU trunk, eight heads, head kind 0 on the
  trunk or 1 linear on the standardized input; export through the
  solver's `policy_net.write_net`, import with `from_file`, the
  pairwise-logistic loss, `choose` and the mode mask as the solver
  applies them), `clone.py` trains the clone and `roundtrip.py` checks
  an exported file against the solver. Clone: stock bias 3, five epochs
  of Adam on the 43,518 decision rows of the 264 training cells with
  decisions (46 s to load, seconds to train; `log/rl-stepD-clone-2026-09-18.log`).
  Loss 0.0499 → 0.0000; the exported net (`benchmarks/rl/clone_stock.net.bin`,
  173 KB) picks stock on **100.000 % of decision rows** at margins 0,
  0.5, 1 and 2 on both the training rows and the 14,559 validation rows,
  every head; stock's lead over the best alternative is at least 7.7
  log-odds (median 15). Round trip: the solver run with the clone on a
  discriminating cell at a 3e9 budget took 18 decisions, 0 off stock,
  and its logged `net_*` scores equal the pure-Python reference **bit
  for bit** on all 18 decision rows (PyTorch in float64 on the same
  float32 weights within 7e-16 relative); a row's `net_*` columns hold
  the scores of the *last* decision, so the scores of the decision made
  from row j are read from row j + 1.
- **Collector arms and the N-arm report (`tools/rl_collect.py`,
  `tools/rl_arms.py`).** Two more job flavours: `off` runs the plain
  solver (no policy, no log; the record comes from the `s` and
  `c workclock` lines, so its `rows`/`footer` columns are empty and the
  status counts do not treat that as a cut log) and `net` runs a weights
  file named by `SAT_POLICY=` in the job's env. `rl_arms.py make` writes
  an N-arm table on per-cell budgets (`--split val`, B_cell or
  `--band-budget`), `report` prints solved and tick PAR-2 per arm and per
  family and the per-cell W comparison against the first arm. One-cell
  rehearsal `log/rl-armstest-2026-09-18-17-24-46`: off, stock and clone
  identical (W 334274015, SAT, 2 decisions, 0 off stock).
- **Baseline 3, the per-instance constant (D.1, `tools/rl_baseline3.py`,
  `benchmarks/rl/baseline3.json`).** For each of the 25 step-0 constants
  whose option acts after preprocessing (the 7 sweep, backbone-effort and
  factor-effort arms are left out: kissat's preprocessing runs
  congruence, backbone, sweep, substitute, factor and fast elimination,
  and the features exist only after it, so a D0 choice could not
  reproduce those arms' sweep outcomes; `--all-arms` puts them back as a
  diagnostic) a ridge regression of log(tick cost at k_res 7 / stock) on
  the 113 actor-tier static features (the block the solver computes
  itself; the critic-tier estimates are offline-only and stay out unless
  `--critic` asks for a diagnostic), fitted on the 78 training cells of
  the medium suite; the selector takes the arm with the lowest predicted ratio when
  it beats stock by a margin, else stock; ridge strength and margin
  chosen by leave-one-out over the 78 cells with the realized sweep
  costs, solved count first, then tick PAR-2 (the feature hygiene is
  refitted inside every fold, so a held-out cell shapes nothing about
  its own prediction). On train the per-cell
  oracle is 62 v 55 solved at 0.698× and the best global constant
  (reorderint 20000) 58 v 55 at 0.986×. LOO picked ridge 1000 with a 0.2
  margin (0.909×, 58 v 55, 38 of 78 cells moved off stock; the same
  choices with the 7 preprocessing-time arms offered, none of which it
  picked): at that
  strength each arm's regression is close to its intercept, so the
  selector is mostly the constant with the best mean log ratio on the
  training cells (reducefrachalf) plus a few others (on the validation
  split, 65 of 99 cells move: reducefrachalf 42, walkeffort100 14,
  vivifyeffort50 6, restartmargin5 2, rephaseint500 1). The honest check,
  the 19 validation medium cells realized from the sweeps: **16 v 16
  solved, tick PAR-2 1.07× stock** (reorderint 20000 there: 16 v 16,
  0.97×); with the critic-tier features added the same procedure lost
  two solves there (14 v 16, 1.19×). The static features carry no
  per-instance signal at this data size (largest weights about 0.01);
  the LOO figure is a selection over 48 grid points on the same 78 cells
  and does not transfer. One constant per cell, not one per knob: only
  single-constant arms were measured in step 0. The validation-split run
  (`benchmarks/rl/baseline3_jobs.tsv`: the 65 moved cells with their
  constant, and all 99 cells with reorderint 20000 and with
  reducefrachalf, at B_cell, the plain solver with the constant on its
  command line through the collector's new `SAT_EXTRA_ARGS` passthrough,
  comma-separated options; stock from the D.3 pass) is queued behind
  D.3 on the same four cores by `log/b3-waiter-2026-09-18.sh`;
  `rl_baseline3.py report` prints solved and tick PAR-2 v stock per
  family when it lands.
- **D.3 done (2026-09-19).** `log/rl-d3clone-2026-09-18-17-26-05`: the
  99 validation cells × {off = the plain solver, stock = policy-on-STOCK
  with logging, clone at margin 1} at `SAT_LIMIT_TICKS = B_cell`, frozen
  binary `9defe8586fcdeadf`, 4 slots on cores 14-17 next to round 0
  (tick-budgeted, so the load does not touch the result). **Identical W
  and identical result on all 99 cells for all three arms** (70 solved
  each, tick PAR-2 3.6697e12 on every arm); no failure, no anomaly. The
  plumbing (log, `observe()`, the net's forward pass, margin and mask)
  is trajectory-neutral before any real learner
  (`python3 tools/rl_arms.py report log/rl-d3clone-2026-09-18-17-26-05`).
- **Baseline 3 on the validation split (D.1 result, 2026-09-22).**
  `log/rl-b3-2026-09-19-08-41-11` (263 jobs, no failure, no anomaly),
  stock taken from the D.3 pass, 99 cells at B_cell, tick-deterministic:

  | arm | solved | tick PAR-2 v stock |
  |---|---:|---:|
  | stock | 70 | 1.000 |
  | baseline 3: the per-instance constant (65 cells moved) | 69 | 1.008 |
  | its modal constant, reducefrachalf on every cell | 70 | 0.974 |
  | **baseline 2: reorderint 20000 on every cell** | **72** | **0.946** |

  The per-instance choice is worse than the single constant it mostly
  picks (cheaper on 16 of the 65 moved cells, dearer on 27; solved +2,
  −3), so static features capture none of the step-0 headroom at this
  data size. The best global constant of step 0 beats stock on the
  validation split as it did on the medium suite (+2 solved, −5.4 % tick
  PAR-2; per family +1 on oddball, lockchart, hcp, kakuro and
  at-least-two, −1 on ncc, circuit-multiplier and timetable): **the
  epoch policy has to beat reorderint 20000, not stock.**
  `~/.cache/sat13-rl/venv/bin/python tools/rl_baseline3.py report --d3
  log/rl-d3clone-2026-09-18-17-26-05 --run log/rl-b3-2026-09-19-08-41-11`.

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
