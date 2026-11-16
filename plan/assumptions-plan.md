# Assumptions: one solver, asked again and again

Plan for epic `SAT-playground-bp3` (branch `assumptions`). Written
2026-11-15 as step 1 (bead `bp3.2`), before any solver code. Line numbers
are those of `solver/13-kissat-rs/src` at the branch point (main `5451c7b`).

## 1. What this is

Today the solver answers one question per run: is this formula satisfiable?

With **assumptions** the same solver can be asked many questions about one
formula: "is it satisfiable if these few variables have these values, for
now?" It answers, takes the values back, and keeps everything it learned for
the next question.

An answer under assumptions is one of three:

- **SAT**, with a model that satisfies the formula and every assumption;
- **UNSAT**, with a **failed core**: a subset of the assumptions that cannot
  all hold together with the formula;
- **UNKNOWN**, when a limit was reached.

The first user is the control split (epic `d96.8`). Today it loads the whole
formula into a fresh solver for every case. Measured before this plan
(`log/as-prior-2026-11-15/NOTE.txt` in the main checkout): of 384 cases on
the four 2025 oisc cells, 352 fall in a few decisions with no fresh solver
at all. Section 9 has the numbers.

## 2. A worked example

Formula, three clauses:

    (not a or x)        a forces x
    (not b or not x or y)   b and x force y
    (not c or not y)    c and y cannot both hold

Question: is it satisfiable assuming `a, b, d, c`, in that order?

| step | what the solver does | what follows |
|---|---|---|
| level 1 | decides `a` | `x` by the first clause |
| level 2 | decides `b` | `y` by the second, then `not c` by the third |
| level 3 | decides `d` | nothing |
| level 4 | wants `c`, but `c` is already false | the answer is UNSAT |

To find the core it walks back from `not c`: its reason holds `y`; the
reason of `y` holds `b` and `x`; the reason of `x` holds `a`. So the core is
`a, b, c`. `d` is not in it.

The proof gains one clause, `(not a or not b or not c)`. A checker confirms
it by setting `a, b, c` and propagating to a conflict. Checked with drat-trim
on 2026-11-15: the formula plus the units `a, b, c` verifies; with the wrong
core `a, c` it is rejected.

Asked next "assuming `a, b`", the same solver answers SAT, with `x`, `y` true
and `c` false. Nothing was loaded again.

## 3. The calls

A new module, `src/assume.rs`. Every call takes DIMACS numbers; internal
numbers never leave the solver (compaction changes them, section 5).

| call | what it does |
|---|---|
| `freeze(var)` | from now on no pass may remove or rename this variable. Counted: two freezes need two melts. Refused if the variable is already eliminated or substituted. A variable the formula never used is made an ordinary free variable first. |
| `melt(var)` | take one freeze back. |
| `assume(lit)` | add a literal to the next solve's list. Freezes its variable for as long as it is in the list. |
| `solve()` | answer under the list: 10 SAT, 20 UNSAT, 0 unknown. The list is used once and is empty afterwards. With an empty list this is the plain solve. |
| `failed(place)` | after 20: is the assumption at this place of the list in the core? Asked by place, never by literal, so a literal given twice is not ambiguous. |
| `value(lit)` | after 10: the model (the existing `internal::value`). |
| `limit(conflicts, work)` | limits for the next solve only (section 6). |
| `add(lit)` | a clause between solves: step 4, not in the first build. A clause on a variable that was eliminated needs that variable's old clauses back. kissat's own option `incremental` keeps all of them on the extension stack (`src/eliminate.rs:321`, `src/substitute.rs:290`; off by default, then only half are kept), so step 4 turns it on. |

The caller also says, before the formula is read, which of two uses it wants:

- **cases of one question** (the split, cube and conquer): the cases cover
  every assignment, and the real question is whether the formula is
  satisfiable. Any pass that keeps satisfiability may run.
- **true assumptions**: "is the formula satisfiable with these values?" Then
  no pass may cut off a model. Symmetry breaking does exactly that, so it is
  off (section 8).

## 4. The search

**Deciding.** Assumption `i` of the list belongs to level `i`. The search
loop's last branch, where a free decision is made, first looks whether
`level` is below the number of assumptions. If so the next assumption is
handled instead:

- it is free: it is decided, at the next level;
- it is already true: the level is opened with nothing on it (an **empty
  level**), so level `i` still belongs to assumption `i`. kitten does the
  same (`src/kitten.rs:1117`);
- it is false: the solve ends with 20 and the core is worked out.

With no assumptions the number is zero and this is one comparison per
decision. Nothing else on the search path changes.

**All variables assigned.** The loop tests "no variable left" before it
decides. With assumptions still to handle that is not yet a model: in
`(not a or not b)` under `a, b`, level 1 decides `a`, propagation sets `b`
false, and every variable has a value. So that branch also looks at the
assumptions first: each one left is true (an empty level) or false (failed).

**Backjumps and restarts.** A backjump may land below the assumption levels;
the loop then decides the missing assumptions again before any free
decision. A restart keeps the assumption levels: trail reuse
(`src/restart.rs:83`, `:101`) starts counting above them, and when the
search is still inside them it does nothing.

**The failed core.** When assumption `a` is found false, every decision on
the trail is an assumption (the search is still inside the assumption
levels). The analysis marks `a`'s variable and walks the trail backwards.
A marked literal with a reason marks the reason's other literals above
level 0; a marked decision is an assumption and goes into the core, by its
level. This is MiniSat's `analyzeFinal`. Special cases:

- `a` is false at level 0: the core is `a` alone, and the unit `not a` is in
  the proof already;
- the list holds `a` and later `not a`: `not a` is found false, its reason
  is the decision `a`, and the core is the pair;
- the same literal twice: the second is true at its turn, gets an empty
  level and is never in the core;
- the formula itself is unsatisfiable: 20 with an empty core, for this and
  every later solve.

**Code that reads a level's decision.** An empty level has none
(`INVALID`):

| place | what changes |
|---|---|
| trail reuse, `src/restart.rs:89`, `:107` | never reads a level at or below the assumption levels |
| compaction, `src/compact.rs:294` | skips an empty level |
| level-one analysis, `src/analyze.rs:416` | no change: a conflict at level 1 means level 1 has literals, so it is not empty |
| vivification, `src/vivify.rs:1170` | no change: it works on its own levels, from level 0 |

**Code that takes level 1 for the first free decision.**

- The level-one analysis (`src/analyze.rs:581`) learns the negation of the
  level-1 decision as a unit. With an assumption there the unit is still
  implied by the formula alone (the assumption by itself led to a conflict),
  so it stays as it is. The next turn of the loop then finds that assumption
  false at level 0, and the core is that one assumption.
- The random-decision trigger (`src/decide.rs:132`) starts a random sequence
  only at level 1. Under assumptions free decisions start higher, so no
  random sequence starts in such a solve. Left so in the first build (it
  costs nothing on the plain path); written down here so it is not a
  surprise.

**Counting.** An assumption is not a choice, so it is not counted in
`decisions` (kissat's own `internal_assume` does not count either). The
assumption solves get their own exit line (section 10).

## 5. What a later solve keeps and what it starts again

Before each solve the solver goes back to level 0. After that:

| state | first solve (today) | a later solve |
|---|---|---|
| clauses, learned clauses, root units | built | kept |
| the trail above level 0, the levels | empty | undone before the solve |
| the model of the last answer | none | gone (the solver is at level 0 again) |
| extension stack (how to give eliminated variables a value) | built by elimination | kept; its "done" flag is cleared, and each SAT answer extends again (`internal.rs:799` extends once today) |
| variable scores, the queue and its stamps | started | kept |
| saved phases | started | kept |
| target and best phases | started | kept, but their measure starts from zero. The measure is how many variables were assigned without a conflict when the phases were saved. It was taken under other assumptions, and after a SAT answer it is "all of them", which nothing could beat. |
| the averages (glue, level, size, trail, decision rate) | started once | kept |
| the mode (focused or stable) and its switch limits | focused first | kept: a solve starts in the mode the last one ended in. `stop_search` clears the mode flag, so it is remembered there and set again between solves. It has to be: a variable that becomes free goes back into the mode's own structure (the score heap in stable mode, the queue in focused mode). |
| restart, reduce, rephase, reorder limits | set by `init_limits` | kept |
| elimination and probing limits | set by `init_limits` | kept |
| the elimination bound | set to zero | kept |
| the delays (reason bumping, congruence, sweeping, vivification) | zero | kept |
| the tiers | set once | kept |
| the random generator | the seed | kept: the stream goes on |
| statistics and the work clock | zero | add up; `searches` counts the solves |
| conflict, decision and work limits | set by the caller, cleared at the end | set again before each solve (section 6) |
| the stop flag | cleared at the end | cleared at the end; a second flag remembers the stop (section 6) |
| the proof | opened by the caller | stays open; lines add up |
| "the formula is unsatisfiable" | false | kept for good |

So a later solve does not call `init_limits` again (it asserts a first
search, `src/kimits.rs:295`) and does not set the generator back to the seed
(`src/search.rs:69`). `start_search` gets a second, short path for a later
solve; the first solve's path is untouched.

**Compaction** renumbers the internal variables in the middle of a search
and maps every fixed variable to one internal literal. So the assumption
list is kept in DIMACS numbers and mapped through `import_` whenever it is
read, as the control decisions already are (`src/control.rs:103`). The core
is reported as places in the caller's list.

## 6. Limits and stops

Today `stop_search` clears the conflict, decision and work limits and the
stop flag. With one search that is harmless. With many it would mean
`SAT_LIMIT_TICKS` and `--conflicts` end with the first solve, and a `--time`
alarm in one solve is forgotten by the next.

- **The run's limits** (`SAT_LIMIT_TICKS`, `--conflicts`, `--decisions`)
  are remembered as absolute numbers when the first solve starts. Before
  every solve the limit is set again to the smaller of the run's limit and
  the call's own. When the run's limit is reached the run is over: every
  later solve answers 0 at once.
- **A call's limits** (`limit(conflicts, work)`) hold for one solve. A solve
  that ends at its limit leaves the solver usable: the next solve goes back
  to level 0 and starts.
- **A stop** (the `--time` alarm; a parent solver's stop) sets the old flag
  and a new one beside it that is never cleared. `solve()` looks at the new
  one first and answers 0 at once. A work limit also raises the old flag
  (`src/terminate.rs:103`), which is why the old flag alone cannot tell a
  stop from a limit.
- A signal (SIGINT, SIGTERM) prints the statistics and ends the process, as
  today.

## 7. The proof

Assumptions are decisions, not clauses. So every learned clause and every
unit in the proof is implied by the formula alone, as today, and the proof
files stay what they are: the DRAT file, and beside it the VeriPB file when a
structure pass or an engine wrote one.

Each UNSAT answer under assumptions adds one line: the clause that negates
the core. It is a propagation consequence of the lines before it (set the
core, propagate, reach the conflict), as long as every reason on the way is a
clause in the proof. A reason the parity or count engine keeps lazily is
written as a clause first, as conflict analysis already does
(`gauss::assigned_with_reason`). A core that holds a literal and its negation
needs no line (the clause would be a tautology).

**Checking one answer.** The claim is "formula plus core is unsatisfiable".
The check: the formula plus one unit clause per core literal, against the
proof written so far plus the empty clause. The solver flushes the proof at
each answer and says how many bytes it holds, so the checker can cut it
there.

When a VeriPB file exists it is checked against the formula as it is: its
lines are numbered after the input clauses, and added units would shift
every number. The units join only at the last stage (drat-trim on the
formula, the derived clauses and the units). `tools/proof_gate.py` gets an
option for this in this step, and a test that a wrong core is rejected.

This check is sound only while no proof line depends on a value of an
assumption variable. That is the freeze rule again: an assumption variable is
never eliminated and never replaced by an equivalent one.

## 8. Every pass and engine

"Safe" means: with assumptions present it can run as it is. "Freeze" means:
safe once it skips frozen variables. "Off" means: it does not run in a solve
that has assumptions. The column "first build" is step 2; "later" is what
step 3 turns on, one pass at a time, each with its own fuzz run.

| pass | removes or renames variables | can answer SAT by itself | first build | later, cases of one question | later, true assumptions |
|---|---|---|---|---|---|
| elimination | yes | no | off | freeze | freeze |
| fast elimination (first preprocessing) | yes | no | off | freeze | freeze |
| substitution | yes | no | off | freeze | freeze |
| sweeping | only through substitution | no | off | freeze | freeze |
| congruence | only through substitution | no | off | freeze | freeze |
| factor | adds variables, removes none | no | off | safe; new variables get numbers above the caller's | the same |
| vivification | no | no | safe | safe | safe |
| backbone | no | no | safe | safe | safe |
| transitive reduction | no | no | safe | safe | safe |
| the first preprocessing as a whole | yes | no | only in a first solve without assumptions | once, before the first solve, assumption variables frozen | the same |
| reduce, restart, rephase, reorder, mode switch | no | no | safe | safe | safe |
| kissat's walk | no | no (it only sets phases) | safe | safe | safe |
| compaction | renumbers inside | no | safe (section 5) | safe | safe |
| model extension | — | — | again at each SAT answer | the same | the same |
| lucky | no | **yes**, from level 0, ignoring assumptions | off | on only if it keeps to the assumptions or its model is checked against them | the same |
| free-choice walk (`found`, `turn`, `rephase_slot`) | no | **yes**, ignoring assumptions | a solver that has it refuses assumptions | on only with its model checked against the assumptions | the same |
| structure stage: symmetry (`structsym`, `structsymext`), the point step (`structpoints`) | no | no | off | may run (they keep satisfiability) | **off**: they cut off models |
| structure stage: the other passes | add helper variables | no | off | may run | each pass's proof is read first: a redundance step may only define a new variable, never restrict an input one |
| parity engine (`gauss`) | freezes its own columns | no | a solver that has it refuses assumptions | on, lazy reasons written before the core analysis | the same |
| count engine (`card`) | no | no | a solver that has it refuses assumptions | on, the same rule | the same |
| RL policy, its log, fork mode | — | — | refused together with assumptions | refused | refused |

Three notes on the first build:

- The two engines and the free-choice walk exist only when the structure
  stage found something for them. A solver that has one refuses `assume`
  with an error. An earlier draft had them "stand aside" for the length of a
  solve (the parity engine through `gauss.suspended`, the count engine
  through a new switch). That code would have had no user and no test: the
  test mode runs without the structure stage, and so do the split's own
  solvers. A refusal can be tested, and is. Step 3 replaces it by the real
  thing.

- The passes marked off are off in a solve that has assumptions. A solve
  with an empty list is the plain solve and runs everything, with one
  exception: the first preprocessing runs only if that solve is the very
  first. (It was written for a formula with no learned clauses. Whether it
  can run after assumption solves is for step 5, which needs it after the
  split.)
- The freeze is real from the first build on, because a plain solve between
  two assumption solves does eliminate. The checks are the ones that already
  protect the parity engine's columns: `src/eliminate.rs:432`,
  `src/substitute.rs:75`, `:106`, `:205`, `src/fastel.rs:924`, `:973`. The
  frozen mark is a spare bit of the variable's flags (`src/flags.rs`: 11 of
  16 bits are used), so it moves with the variable in a compaction.

## 9. The split in two stages (step 5)

Measured from outside, 2026-11-15, the frozen `13-j22` build, 768 runs
(`log/as-prior-2026-11-15/NOTE.txt`):

| cell | cases | fall with the clean-up off | need the clean-up | no answer in 3G | split's work today | two stages (estimate) |
|---|---|---|---|---|---|---|
| sll-8 | 64 | 60 | 4 | 0 | 4.90G | 1.10G |
| and-11 | 96 | 85 | 10 | 1 | 17.38G | 8.47G |
| and-12 | 128 | 116 | 11 | 1 | 23.64G | 8.52G |
| and-15 (96 of its cases) | 96 | 91 | 1 | 4 | 32.19G | 14.74G |

One easy case, sll-8 with its six bits at the value 5: today a fresh solver
pays 49.9M to load the formula and 36.3M to refute it. With the passes off
it falls in 16.0M after one decision and two conflicts.

- **Stage 1.** Every case is a solve under the case's bits, with a small work
  limit (one load's worth covers every easy case measured). No clean-up,
  learned clauses kept from case to case.
- **Stage 2.** A case stage 1 leaves open goes to a fresh solver with the
  bits as unit clauses, as today.
- A core with fewer bits than the case refutes every case that shares those
  values; they are skipped.
- The estimates are from a stand-in (bits as unit clauses, passes off). Step
  5 measures the real thing first, case by case, before the proof work.

**Which solver answers stage 1.** Not the main solver, but one **case
solver** that the split keeps for all its cases: a second solver, loaded
once, asked under assumptions. Decided 2026-11-15, before step 5's code, for
these reasons:

- *The main search stays what it is today.* When a split gives up (a case is
  a search, or the budget ends), the main search starts afterwards. If the
  cases had run in the main solver, that search would start with other
  scores, other phases, learned clauses and no first preprocessing. With a
  case solver the main solver is untouched until the split is over, so
  whatever the split does not decide is searched exactly as today. The runs
  of step 8 then differ from the current split only where the split itself
  differs.
- *The proof is simple.* Under assumptions the case solver's learned clauses
  follow from the formula alone, so its proof lines go into the main proof as
  they are, with no bits added. Only added lines are copied (its deletions
  are dropped), so nothing the main solver still needs is ever deleted.
- *The cost is one load*, paid by the first case, instead of one per case
  (sll-8: 49.9M of an estimated 1.10G).
- *What is given up:* the memory of the second solver stays, as today. It
  would stay anyway while hard cases go to a fresh solver.

A case's proof in stage 1, in the worked example's terms: the case solver
writes whatever it learned, then the clause that negates the failed core. If
the core is the whole case, that clause is the case's own clause, the one
the joins need. If the core is smaller, the case's clause is added after it
(it contains the core's clause, so a checker accepts it at once).

## 10. The test driver

The crate builds only a binary, so the tests in `tests/` can only run it.
The binary gets a test mode, `--inccnf=1`, that reads the **incremental CNF
format** (the one CaDiCaL and iLingeling read):

    p inccnf
    1 -2 0
    2 3 0
    a 1 2 0
    a -3 0
    a 0

The header has no counts. Clauses come first; then every line that starts
with `a` is one question: solve under these literals. `a 0` asks with no
assumptions. (CaDiCaL's own binary treats the `a` lines as the cases of one
question and stops at the first satisfiable one. Here each line is its own
question and all are answered.)

For each question the binary prints one block:

    q 1 UNSATISFIABLE
    f 1 2 0
    p 37
    q 2 SATISFIABLE
    v 1 -2 -3 0
    p 37
    q 3 UNKNOWN
    p 52

- `q`, the question's number, the answer;
- after SATISFIABLE, `v` lines: the model, as the witness is printed today;
- after UNSATISFIABLE, one `f` line: the failed core, in the order of the
  question's literals (`f 0` when the formula itself is unsatisfiable);
- `p`: the bytes of the DRAT file so far, when a proof is written.

No `s` line is printed in this mode, so nothing can take the output for a
competition answer. Limits per question: `--assumeconflicts=N` and
`--assumeticks=N` (thousands of work units); the run's own limits are the
usual ones. One exit line, outside the `-s` block and absent when the mode is
off: `c assume solves=... sat=... unsat=... unknown=... decided=...
empty=... core=...`.

## 11. The oracle: `tools/assume_fuzz.py`

It makes small formulas and lists of questions, runs the binary in test
mode, and checks every answer without trusting the branch's code:

- **Formulas**: random 3-SAT near the threshold, and small structured ones.
- **Questions** made to break a freeze: assumption variables that are gate
  outputs, equivalent to another variable, pure, fixed by a unit, or absent
  from the formula; the same literal twice; a literal with its negation; an
  empty list; a question after one that ended at its limit.
- **The right answer**: by brute force when the variables are few. Otherwise
  from solvers the branch cannot touch, each run fresh with the assumptions
  as unit clauses: the frozen main build, the C kissat and CaDiCaL 2.1.3.
  They must agree with each other, or the case is reported as an oracle
  failure, not a solver failure.
- **A model** is checked against every clause and every assumption.
- **A core** must be a subset of the question's literals, and the formula
  plus the core must be unsatisfiable (by the same oracles).
- **A proof**: every UNSAT answer through `tools/proof_gate.py` with the
  core as units and the proof cut at the answer's `p` line.

Before it is used on the branch's solver the harness is validated twice.
First on today's solver, one fresh run per question with the assumptions as
unit clauses. Then against deliberately wrong answers, each of which it must
catch: a wrong model, a model that breaks an assumption, a core that is
satisfiable with the formula, a core that is not a subset, a broken proof.

## 12. What is refused

- Assumptions together with the RL policy, its log or fork mode (one search
  per run is built into the epoch clock and the log): an error at the start.
- Assuming or freezing a variable that is already eliminated or substituted:
  an error, until step 4 can bring its clauses back.
- `--inccnf=1` together with a structure pass, an engine, the free-choice
  walk or the control options: the driver turns them off and says so.
- `assume` in a solver that has the parity engine, the count engine or the
  free-choice walk: an error, until step 3.

## 13. Off means off

With no assumptions the solver must do what main's solver does, step for
step. The baselines of step 0 (`benchmarks/assumptions/`) are the measure:
after every step, `tools/as_baseline.py` runs the branch build over the same
cells and `compare` must find every cell identical: the answer, the `-s`
counters, the work-clock line, the output with times taken out, and the
sha256 of each proof file. `solver/13-kissat-rs/tools/parity.py --corpus
default --phases` must still match the C kissat on the stock path. Speed is
checked with `tools/as_pairtime.py` against the noise floor of step 0.

## 14. Order of work

| step | bead | what |
|---|---|---|
| 0 | `bp3.1` | branch, worktree, baselines |
| 1 | `bp3.2` | this plan, CaDiCaL, the proof gate's option, the fuzz harness |
| 2 | `bp3.3` | assumptions in the search, the freeze, the test driver |
| 3 | `bp3.4` | the passes back on, one at a time |
| 4 | `bp3.5` | adding clauses between solves |
| 5 | `bp3.6` | the split in two stages, measured first |
| 6 | `bp3.7` | the correctness gate |
| 7 | `bp3.8` | the performance gate |
| 8 | `bp3.9` | the runs |
| 9 | `bp3.10` | the owner's decision: merge or roll back |

The split needs steps 0, 1, 2 and 5. Steps 3 and 4 serve later uses.
