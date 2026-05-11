# Kissat Comparison Notes

This note compares solver `11-kissat-innovations` against the vendored Kissat source at
`benchmarks/reference-solvers/kissat-latest`.

Reference inspected:

- Kissat version: `4.0.4`
- local git revision: `8af8e56` (`bumped VERSION`)
- solver 11 is currently a fork of solver `10-bve-preprocess`

The main conclusion is that solver 11 already has a MiniSat-style CDCL core plus one-shot
MiniSat-like BVE/BSR preprocessing. Kissat's largest differences are in search organization,
metadata, propagation representation, learned-clause management, and scheduled inprocessing. The
BVE code is only one part of the gap.

## High-Level Algorithm Differences

### 1. Search loop and scheduled work

Solver 11:

- Runs root propagation, then one-shot `eliminate(true, proof_log)`, then disables simplification.
- Main loop order is conflict analysis, pending restart, root simplify, reduce DB, decide.
- There is no search-mode switch, rephase event, probing event, inprocessing BVE event, reorder
  event, or termination/limit scheduler.
- Relevant code: `src/main.rs:1988`, `src/main.rs:2024`, `src/main.rs:2100`.

Kissat:

- Tries lucky assignments before and after preprocessing.
- Runs preprocessing when configured, classifies the formula, initializes limits, then repeatedly
  checks scheduled maintenance in a fixed order: reduce, switch search mode, restart, reorder,
  rephase, probe, eliminate, then decide.
- Relevant code: `search.c:179`, `search.c:192`, `search.c:204`.

Why this matters:

- Solver 11 can only simplify before search. Kissat continually changes the search state and formula
  after enough conflicts/ticks.
- Some Kissat ideas, especially rephase, vivify, transitive reduction, and repeated BVE, require a
  scheduler and root-level transition path rather than another one-shot preprocessor.

### 2. Stable/focused search modes

Solver 11:

- Uses one global EVSIDS-like variable heap for all decisions.
- Every analyzed variable is bumped in that heap.
- Branch phase is only saved phase, initialized by `SAT_BRANCH_MODE`.
- Relevant code: `src/main.rs:248`, `src/main.rs:1281`, `src/main.rs:1364`.

Kissat:

- Has two search modes.
- Stable mode uses a score heap.
- Focused mode uses a linked decision queue and picks the last enqueued unassigned variable.
- In focused mode, analyzed variables are moved to the front of the queue instead of score-bumped.
- It can switch modes on conflict/tick limits.
- Relevant code: `decide.c:126`, `bump.c:103`, `mode.c:19`, `mode.c:150`.

Why this matters:

- Solver 11 is closer to MiniSat than Kissat here. Kissat's focused mode is a fundamentally different
  branching policy, not just different decay constants.

### 3. Restart policy

Solver 11:

- Uses a fixed Luby restart budget with `restart_unit = 100`.
- Restarts always backtrack to level 0.
- Relevant code: `src/main.rs:264`, `src/main.rs:1333`, `src/main.rs:1346`,
  `src/main.rs:1425`.

Kissat:

- Focused mode restarts compare fast and slow glue EMAs after a restart interval.
- Stable mode uses reluctant doubling restarting.
- Restarts can reuse a prefix of the trail instead of always backtracking to level 0.
- Relevant code: `restart.c:14`, `restart.c:39`, `restart.c:53`, `restart.c:112`.

Why this matters:

- Solver 11 does not compute glue/LBD, so it cannot currently implement Kissat's restart trigger.
- Always backtracking to level 0 also throws away useful trail prefixes that Kissat intentionally
  preserves.

### 4. Learned-clause metadata and reduction

Solver 11:

- Learned clauses have floating clause activity but no glue/LBD, used counter, tier, reason bit, or
  searched literal position.
- Reduction sorts by binary status and clause activity, deletes about half of non-binary unlocked
  clauses plus low-activity clauses.
- Relevant code: `src/main.rs:260`, `src/main.rs:1470`, `src/main.rs:1734`.

Kissat:

- Clause headers contain glue, garbage, redundant, reason, shrunken, subsume, swept, vivify, used,
  searched, and size fields.
- Learned clause usefulness is based primarily on glue/tier and used counts, not floating clause
  activity.
- Reduction recomputes tier limits from glue usage statistics, ranks reducibles by size/glue, uses a
  changing reduction fraction, and sparse-collects garbage.
- Relevant code: `clause.h:12`, `clause.h:17`, `reduce.c:37`, `reduce.c:102`,
  `tiers.c:6`.

Why this matters:

- Kissat's reduce/restart/rephase/vivify machinery all depends on glue and tier metadata that solver
  11 does not store.

### 5. Propagation and binary clauses

Solver 11:

- Stores every clause, including binary learned clauses, in the same arena format.
- Watchers are uniform `{ clause_idx, blocker }` records.
- Propagation scans large clauses from literal position 2 every time.
- Binary implications go through the same clause path as larger clauses.
- Relevant code: `src/main.rs:225`, `src/main.rs:1151`, `src/main.rs:1234`,
  `src/main.rs:1470`.

Kissat:

- Binary clauses are represented directly as tagged binary watches, with no arena clause.
- Large clauses use a two-watch representation with blocking literal plus arena reference.
- Large clauses cache a `searched` position to avoid repeatedly scanning from the beginning.
- Large watch insertion can be delayed until the current watch list scan finishes.
- Relevant code: `watch.h:20`, `watch.h:59`, `proplit.h:71`, `proplit.h:82`,
  `proplit.h:103`, `proplit.h:114`, `proplit.h:137`.

Why this matters:

- Propagation is usually the dominant runtime. Solver 11 is missing one of Kissat's most important
  low-level representations: binary watches as an implication graph with binary reasons.

### 6. Conflict analysis, learning, and backtracking

Solver 11:

- Implements first-UIP analysis, skips reason literal position 0 in the MiniSat-compatible mode, and
  does basic/deep clause minimization.
- Always jumps to the highest non-current decision level in the learned clause.
- Does not compute glue/LBD, chronological backtracking, failed-literal special handling, shrink,
  eager subsumption of recent learned clauses, or reason-side bumping.
- Relevant code: `src/main.rs:1824`, `src/main.rs:1860`, `src/main.rs:1910`,
  `src/main.rs:1928`, `src/main.rs:2089`.

Kissat:

- Handles conflict clauses that already have one literal on the conflict level.
- Has failed-literal analysis at decision level 1.
- Deduces first-UIP clauses, sorts by levels, minimizes, optionally shrinks, bumps reason-side
  literals, learns with glue, and optionally eagerly subsumes recently learned clauses.
- Uses chronological backtracking when jumping over too many levels is considered inefficient.
- Relevant code: `analyze.c:527`, `analyze.c:545`, `analyze.c:549`, `analyze.c:552`,
  `analyze.c:560`, `analyze.c:566`, `learn.c:16`, `learn.c:70`, `learn.c:190`.

Why this matters:

- Solver 11's analyzer is correct and compact, but it is missing several Kissat behaviors that can
  drastically change the search path.

### 7. Phase selection and rephasing

Solver 11:

- Stores only one `saved_phase` vector.
- Saved phase is updated on every enqueue, and decisions use saved phase directly.
- There is no target phase, best phase, periodic rephase, inverted/original schedule, or local-search
  phase source.
- Relevant code: `src/main.rs:232`, `src/main.rs:1120`, `src/main.rs:1364`.

Kissat:

- Stores saved, target, and best phase vectors.
- Stable-mode backtracking records target/best assignments when the trail reaches new heights.
- Rephase events copy best, walking, inverted, best, walking, original phases in a schedule.
- Decisions prefer target then saved then initial phase, with focused-mode phase overrides.
- Relevant code: `phases.h:6`, `backtrack.c:38`, `decide.c:155`, `rephase.c:32`,
  `rephase.c:86`, `rephase.c:109`.

Why this matters:

- Solver 11 can get stuck in one saved-phase trajectory. Kissat deliberately perturbs phases and
  preserves promising partial assignments.

### 8. Inprocessing and preprocessing

Solver 11:

- Preprocessing is a MiniSat-style one-shot pass over original clauses: canonical clause insertion,
  full BSR, BVE, model extension, and cleanup.
- It clears occurrence data and sets `use_simplification = false` after preprocessing.
- There is no probing, backbone, congruence, substitution, SAT sweeping, transitive reduction,
  vivification, factorization, or repeated elimination during search.
- Relevant code: `src/simp.rs:920`, `src/simp.rs:951`, `src/simp.rs:1026`.

Kissat:

- Initial preprocessing runs root propagation, sparse collection, initial probing, and optional fast
  elimination.
- Probing/inprocessing can run during search and includes congruence, substitution, backbone,
  vivification, sweeping, transitive reduction, and factorization.
- BVE is scheduled later by conflict limits and progress bounds, not only before search.
- Relevant code: `preprocess.c:29`, `preprocess.c:71`, `probe.c:16`, `probe.c:26`,
  `eliminate.c:21`.

Why this matters:

- Solver 11's preprocessing quality can be good on some instances, but it cannot exploit information
  learned during search or periodically clean the formula the way Kissat does.

### 9. Variable elimination scoring

Solver 11:

- Elimination heap score is `positive_occurrences * negative_occurrences`.
- Bounds are MiniSat-style `grow = 0` and `clause_limit = 20`.
- Relevant code: `src/simp.rs:28`, `src/simp.rs:941`, `src/simp.rs:1006`.

Kissat:

- Elimination score uses capped occurrence product minus occurrence sum, then adds search relevancy
  from stable scores or focused queue stamps.
- Elimination is also bounded by effort, occurrence limits, rounds, and variable progress limits.
- Relevant code: `eliminate.c:41`, `options.h:41`, `options.h:44`, `options.h:45`,
  `options.h:47`.

Why this matters:

- Kissat's elimination schedule is connected to search relevance. Solver 11 may spend effort on
  cheap variables that are irrelevant to the active search path, or miss variables whose elimination
  is useful despite a similar occurrence product.

### 10. Formula representation and clause lifecycle

Solver 11:

- Uses `Vec<u32>` packed words with a small custom header and separate vectors for original and
  learned clause ids.
- Deleted clauses are marked in the header, and garbage collection copies all live clauses when
  deleted words exceed a threshold.
- Relevant code: `src/main.rs:217`, `src/main.rs:345`, `src/main.rs:1543`.

Kissat:

- Uses a clause struct inside an arena, with explicit lifecycle flags and arena references.
- Keeps `first_reducible` / `last_irredundant` positions and can sparse collect from a later arena
  offset.
- Binary clauses live in watch lists, not the arena.
- Relevant code: `internal.h:111`, `clause.h:12`, `reduce.c:153`.

Why this matters:

- Solver 11 can keep working with the packed arena, but adding Kissat-like behavior will require
  storing more metadata per learned clause.

## Fresh Second-Pass Findings

These are the additional Kissat ideas that stood out after reading beyond the obvious CDCL loop.
Most of them are not isolated tricks; they depend on root-level scheduling, binary implication
watches, and richer clause metadata.

### 11. Lucky assignment and failed-literal probing

Solver 11:

- Goes straight into root propagation, one-shot simplification, and normal CDCL search.
- Does not try all-true/all-false shortcuts or ordered failed-literal probing before search.

Kissat:

- Calls `kissat_lucky` before and after preprocessing.
- First checks whether there are no all-negative or no all-positive irredundant clauses. If so, it
  can assign all active variables true or false and return SAT.
- If that fails, it tries four cheap probing passes: forward false, forward true, backward false,
  and backward true over import order.
- Conflicts at decision level 1 become failed literals and can learn root units; deeper conflicts
  trigger a complementary assignment check before continuing.
- Relevant code: `search.c:185`, `search.c:189`, `lucky.c:1`, `lucky.c:307`.

Implementation idea for solver 11:

- Add a very small first experiment for no-all-negative/no-all-positive on the active simplified
  formula. This is low risk and can be tested without changing the CDCL core.
- After binary implication watches exist, add bounded lucky probing with an effort cap and counters
  for SAT shortcut, units learned, conflicts, and time.
- Ensure a lucky SAT after preprocessing still runs the existing model-extension path for eliminated
  variables.

Why this matters:

- This is not a general performance feature, but it is a cheap way to solve or simplify easy
  structured instances before committing to a search trajectory.

### 12. Warmup phase seeding

Solver 11:

- Initializes phases from `SAT_BRANCH_MODE` and then relies entirely on phase saving during real
  search.

Kissat:

- Optional `kissat_warmup` temporarily runs normal decisions and propagation beyond conflicts,
  saving phases through the normal assignment path.
- It then backtracks to level 0 without updating phases, leaving only the seeded saved phases.
- Relevant code: `walk.c:961`, `warmup.c:9`.

Implementation idea for solver 11:

- Add `SAT_WARMUP=1` as a root-only pass after preprocessing and root propagation.
- Run decisions using the current heuristic, propagate, ignore learned clauses at first, save phases,
  and backtrack without overwriting those phases.
- Measure separately because warmup can either help phase selection or waste time on formulas where
  preprocessing already gives good phases.

Why this matters:

- It is a small, self-contained way to test whether solver 11's saved-phase trajectory is too cold
  relative to Kissat.

### 13. Root-level decision reordering by clause weights

Solver 11:

- The stable heap is shaped only by conflict bumps and decay. The focused queue does not exist yet.
- Clause structure at root does not directly reorder decisions after simplification.

Kissat:

- Periodically reorders at decision level 0.
- Computes literal weights from irredundant clauses and binary clauses. Clause size 2 starts at
  weight `1`, and each larger size halves the weight.
- Variable weight is `max(pos, neg) + 2 * min(pos, neg)`.
- Stable mode rescales scores and adds the weight to the score heap. Focused mode sorts active
  variables by weight and moves them through the decision queue.
- Relevant code: `search.c:210`, `reorder.c:14`, `reorder.c:198`.

Implementation idea for solver 11:

- Add a stable-mode-only version first: compute root clause weights and add them into the existing
  heap behind `SAT_REORDER=clause-weight`.
- Reuse the current clause arena and watcher lists; after binary watch splitting, count binary
  implications through the binary watch path.
- Later, use the same weight computation to feed focused queue order.

Why this matters:

- This is a direct bridge between formula structure and branching. It may be cheaper to implement
  than full focused mode but still moves solver 11 closer to Kissat's search initialization.

### 14. Binary implication graph cleanup

Solver 11:

- Has no separate binary implication graph, so binary simplifications must scan normal clauses.

Kissat:

- Treats binary clauses as first-class watches and uses them for transitive reduction.
- `kissat_transitive_reduction` probes reachability in the binary implication graph, removes
  transitive binary clauses, and can learn units when both a literal and its negation are reachable.
- The pass is root-level and tick-limited.
- Relevant code: `probe.c:39`, `transitive.c:299`.

Implementation idea for solver 11:

- Do not implement this before item D splits binary clauses into direct implication watches.
- After that split, add a bounded binary transitive reduction pass that only touches redundant or
  all binary clauses under a feature flag.
- Track removed binary clauses, learned units, ticks, and proof output for each removed clause.

Why this matters:

- Kissat's binary implication graph is used as a data structure for propagation and as an
  inprocessing substrate. Solver 11 currently lacks both benefits.

### 15. On-the-fly strengthening and subsumption during analysis

Solver 11:

- Learns the first-UIP clause, optionally minimizes it, and adds it as a new clause.
- It does not modify the reason/conflict clause during analysis except through normal learning.

Kissat:

- `deduce.c` marks redundant reason clauses as used, recomputes/promotes glue, and tracks
  antecedent and resolvent sizes.
- If on-the-fly strengthening is enabled and the resolvent shrinks the antecedent, it strengthens
  the reason clause and can subsume the conflict clause.
- Relevant code: `deduce.c:14`, `deduce.c:149`, `strengthen.c:142`.

Implementation idea for solver 11:

- After glue metadata is available, add counters in analysis for antecedent size and resolvent size.
- Start with diagnostics only: report how often a reason could be strengthened.
- Then implement strengthening for non-binary learned clauses first, with DRAT deletion/addition
  carefully tested.

Why this matters:

- This is a Kissat feature that improves clauses while analysis already has the relevant marks in
  cache. It is higher proof-risk than the search scheduling changes.

### 16. Forward subsumption, hyper-unary detection, and duplicate binary cleanup

Solver 11:

- BSR/BVE preprocessing exists, but there is no standalone bounded forward-subsumption pass exposed
  to the CDCL/inprocessing scheduler.

Kissat:

- `forward.c` uses occurrence lists to find subsumed or strengthenable clauses with bounds on
  occurrence counts and candidate sizes.
- It handles duplicate binary clauses, binary-large strengthening, large-large strengthening, and
  hyper-unary situations that produce units.
- Relevant code: `forward.c`, `fastel.c:822`.

Implementation idea for solver 11:

- Extract the reusable parts of `simp.rs` occurrence-list logic into a bounded forward-subsumption
  pass that can run at root before and later during search.
- Add a cheap duplicate-binary cleanup once binary clauses have their own representation.
- Record skipped candidates by occurrence limit, size limit, and proof mode.

Why this matters:

- Solver 11 already has some simplification code. This is a practical way to reuse it for more
  Kissat-like inprocessing without starting from congruence or vivification.

### 17. Fast variable elimination as a separate low-effort path

Solver 11:

- Has one MiniSat-like elimination path with full BVE/BSR settings.

Kissat:

- Has optional `kissat_fast_variable_elimination` in preprocessing. It enters dense mode, uses
  tighter occurrence and clause-size limits, handles binary-binary and binary-large resolvents
  quickly, and exits back to sparse watches.
- Relevant code: `preprocess.c:74`, `fastel.c:822`.

Implementation idea for solver 11:

- Add a fast-BVE mode with much stricter caps than the current BVE: low occurrence limit, low clause
  size limit, and explicit time/tick budget.
- Treat it as a preprocessing option before replacing the existing BVE.
- Compare full BVE, fast BVE, and both together because more elimination is not always better for
  CDCL search.

Why this matters:

- This gives a smaller measured target than a full Kissat elimination rewrite and may reveal whether
  solver 11's current BVE is over-spending on low-value variables.

### 18. Vivification scheduling details

Solver 11:

- Does not vivify clauses.

Kissat:

- Schedules vivification candidates by tier: irredundant clauses, low-glue learned clauses, then
  broader learned tiers.
- Counts literal occurrences, ranks candidates by size/count/vivify history, and sorts literals in
  candidates before probing.
- During vivification, conflicts or implied literals can subsume or shrink the candidate. Glue is
  recomputed/promoted for affected learned clauses.
- Relevant code: `probe.c:36`, `vivify.c:1395`.

Implementation idea for solver 11:

- Do not begin with full vivification. First implement candidate selection and report how many
  candidates would be tried by tier and budget.
- After glue/tier metadata and root probing are in place, add irredundant-clause vivification only.
- Learned-clause vivification should come after tiered reduction, otherwise the selection policy is
  not Kissat-like.

Why this matters:

- Vivification is a major Kissat inprocessing technique, but its value depends heavily on candidate
  scheduling. The scheduling can be ported and measured before the expensive propagation logic.

### 19. Gate extraction, congruence, and substitution

Solver 11:

- Does not detect gates or equivalent literals.

Kissat:

- During elimination, `kissat_find_gates` extracts equivalence, AND, ITE, and definition gates for a
  variable.
- Probing can run congruence closure/substitution over detected gates using dense occurrence data.
- Relevant code: `gates.c:34`, `ands.c`, `equivalences.c`, `ifthenelse.c`,
  `congruence.c:4578`.

Implementation idea for solver 11:

- Long term: add gate-aware elimination/substitution.
- Near term: start with equivalence detection from binary implications because it has the smallest
  dependency surface after binary watch splitting.
- Defer full AND/ITE/congruence closure until dense occurrence mode and proof logging for
  substitutions are mature.

Why this matters:

- These are high-impact structural simplifications on encoded hardware/planning formulas, but they
  are not first-wave features for solver 11.

### 20. Dense mode for simplification and sparse mode for search

Solver 11:

- Occurrence lists are built inside simplification and then discarded.
- Search uses watchers only, with no explicit mode boundary.

Kissat:

- Enters dense mode for simplification passes by flushing large watches and building full occurrence
  lists.
- Dense root propagation can mark satisfied clauses garbage and find units through occurrence lists.
- It resumes sparse mode by reconnecting watchers and resetting propagation state.
- Relevant code: `dense.c:99`, `dense.c:199`, `propdense.c`, `eliminate.c:587`.

Implementation idea for solver 11:

- Add an explicit `enter_simplification_mode` / `resume_search_mode` boundary before adding repeated
  inprocessing.
- Reuse occurrence-list allocation across BVE, forward subsumption, vivification candidate
  selection, and future gate detection.
- Keep the first version preprocessing-only; make it robust before allowing mid-search transitions.

Why this matters:

- Kissat's heavy inprocessing is practical because it has deliberate representation transitions.
  Solver 11's current one-off occurrence lists will become brittle if every pass rebuilds its own
  view of the formula.

### 21. Clause collection and arena discipline

Solver 11:

- Marks deleted clauses and occasionally rebuilds the packed arena by copying all live clauses.

Kissat:

- Separates irredundant and reducible regions with `first_reducible` / `last_irredundant`.
- Can sparse-collect garbage from later arena offsets and compact inactive variables/literals when
  simplification removes variables.
- Relevant code: `collect.c`, `compact.c`, `reduce.c:153`.

Implementation idea for solver 11:

- Do not rewrite the arena now, but add metadata/accessor boundaries so later sparse collection is
  possible.
- If learned reduction starts deleting many more clauses, add counters for copied words and GC pause
  time before deciding whether to port Kissat-style sparse collection.

Why this matters:

- This is an implementation multiplier rather than an algorithm by itself. It matters once solver 11
  starts doing repeated reduction, vivification, and inprocessing.

### 22. Bounded variable addition / factorization

Solver 11:

- Does not introduce new variables during simplification.

Kissat:

- `kissat_factor` performs bounded factorization / variable addition using watch counts and optional
  structural scoring.
- It is scheduled as a probing/inprocessing pass, not as a core CDCL operation.
- Relevant code: `probe.c:41`, `factor.c:1087`.

Implementation idea for solver 11:

- Treat this as a late-stage feature only after proof handling, dense mode, and gate-aware
  substitution are solid.
- Record it as an idea, but do not prioritize it for the first solver-11 experiments.

Why this matters:

- It is part of Kissat's broader simplification ecosystem, but the dependency and proof complexity
  are too high for the first wave.

## Proposed Implementation Roadmap

The first group below is ordered by dependency and expected leverage. Items M-X are second-pass
additions from the fresh Kissat read; their dependency notes should drive exact scheduling. Each
change should be implemented behind environment flags at first, measured on a fixed target set, and
kept only if it improves correctness or performance under the repo optimization workflow.

### A. Add Kissat-facing diagnostics first

Goal:

- Make solver 11 able to report the metrics needed to compare against Kissat behavior before changing
  algorithms.

Implementation items:

- Add optional trace fields for learned clause size, glue, backtrack level, restart reason, reduction
  reason, branch mode, phase source, and propagation throughput.
- Extend existing `SAT_TRACE_SEARCH_INTERVAL` output to include current mode, average glue once
  available, binary/large watch counts, live redundant clauses, and deleted garbage words.
- Keep this as diagnostics only; no behavior change.

Validation:

- `cargo test`
- `bash tools/smoke_test.sh solver/11-kissat-innovations`
- One small benchmark run comparing trace counters against Kissat on the same instances.

### B. Store glue/LBD and used counters for learned clauses

Goal:

- Introduce the metadata needed by Kissat-style restart, reduce, tiering, and vivification.

Implementation items:

- During conflict analysis, collect distinct non-zero decision levels in the learned clause and
  compute glue/LBD.
- Extend the learned-clause extra data to store glue and `used`, while preserving clause activity
  until the old reducer is replaced.
- Initialize learned clauses with computed glue/LBD and `used = MAX_USED`; binary learned clauses
  use the same distinct-decision-level LBD calculation instead of a separate hard-coded value.
- Add unit tests that build known implication graphs and assert learned clause glue.

Notes:

- This should come before changing restart or reduce policy.
- Implementation status: landed for diagnostics/future policy use. Learned clauses now carry one
  extra packed metadata word after activity containing glue, used, tier, and spare lifecycle flags.
  Existing activity-based reduction and restart behavior are unchanged.
- Validation: `cargo test` passed 53 tests; normal smoke passed 9/9; invariant smoke with
  `SAT_CHECK_INVARIANTS=1` passed 9/9.
- Profiling overhead check on `benchmarks/profiling`, timeout 120s, memory 16 GB:
  pre-metadata log `log/bench-11-kissat-innovations-2026-05-09-21-09-49`, PAR-2 `1104.865`,
  solved 7/11; metadata log `log/bench-11-kissat-innovations-2026-05-09-21-26-34`, PAR-2
  `1104.276`, solved 7/11. Same solved/timeout split; runtime delta is within normal benchmark
  noise.

### C. Replace activity-based learned reduction with tiered glue reduction

Goal:

- Move learned-clause retention toward Kissat.

Implementation items:

- Add tier thresholds with defaults equivalent to Kissat: tier1 `2`, tier2 `6`.
- Track used counts and decrement them during reduction scans.
- Keep binary clauses, locked clauses, tier1 clauses with nonzero used, and tier2 clauses with high
  used.
- Rank reducibles by Kissat-like usefulness: larger size and larger glue should be less useful.
- Use a dynamic reduction fraction moving from high toward low, similar to Kissat's `reducehigh` /
  `reducelow` policy.
- Add tests for reducer keep/delete decisions by glue, used, binary, and locked status.

Risk:

- This can regress before branch modes and restart policy are updated, so keep
  `SAT_REDUCE_MODE=activity` as an explicit fallback while glue-tiered reduction is the default.

Implementation status:

- Landed as the solver-11 default. Use `SAT_REDUCE_MODE=activity` to run the previous activity
  reducer.
- Learned clauses are classified by the stored glue metadata: tier 1 is glue `<= 2`, tier 2 is glue
  `<= 6`, and tier 3 is everything higher.
- Binary learned clauses, locked clauses, and tier-1 clauses are protected.
- Normal glue-mode reducibles are non-binary unlocked clauses with tier `>= 3` or `used == 0`.
- Reducibles are ordered worst first by higher tier, higher glue, lower used count, larger size,
  lower activity, then lower arena id.
- A reduction deletes at least half of normal reducibles, but also respects budget pressure: it aims
  the live learned DB back toward `3/4 * reduce_db_limit`. If normal reducibles cannot reach that
  target, used tier-2 clauses become pressure candidates. Kept learned clauses have `used`
  decremented by one, so the grace is temporary.

Validation:

- `cargo test`: 70 passed.
- Default glue-tiered smoke: 9/9 passed, log `log/2026-05-10-21-08-46`.
- Activity fallback smoke: 9/9 passed, log `log/2026-05-10-21-08-55`.
- Glue-tiered invariant smoke: 9/9 passed, log `log/2026-05-10-20-41-12`.
- Profiling activity fallback: 5/11 solved, PAR-2 `1559.855`, log
  `log/bench-11-kissat-innovations-2026-05-10-19-29-34`.
- Profiling glue-tiered/default: 6/11 solved, PAR-2 `1328.592`, log
  `log/bench-11-kissat-innovations-2026-05-10-20-17-39`.
- Main wins: `feistel_b64_k32_r22` improved from `90.071s` to `27.015s`, and
  `random_v355_s3` changed from timeout to SAT in `53.410s`.
- Main regression: `random_v292_s4` slowed from `8.776s` to `19.809s`; focused trace shows this is
  a search-path regression, not just local reducer overhead.

Rejected/adjusted variant:

- A primary-only glue reducer that deleted half of the immediately eligible clauses caused repeated
  over-budget reductions. On `random_v292_s4`, activity mode solved in `8.557s` with `991`
  reductions, while the first glue cut took over `21s` with about `75k` reductions.
- The retained budget-pressure variant reduced that churn to `3261` reductions in a direct trace,
  while keeping the full profiling-set PAR-2 gain.

### D. Split binary clauses into a fast implication path

Goal:

- Match Kissat's most important propagation representation.

Implementation items:

- Represent binary clauses as direct implication watches rather than arena clauses.
- Store binary reasons separately from arena clause reasons, for example with a tagged `Reason`.
- Make propagation handle binary watches before large watches.
- Preserve proof logging and smoke UNSAT proof correctness.
- Keep original binary clauses visible to proof/model logic or add a binary-clause iterator where
  needed.

Notes:

- This is a larger refactor but likely high leverage because propagation dominates runtime.
- It is also a prerequisite for Kissat-like probing, transitive reduction, backbone, and binary
  reason jumping.

### E. Add focused mode decision queue

Goal:

- Implement Kissat's focused search behavior without removing the existing stable heap path.

Implementation items:

- Add a decision queue with links/stamps for active variables.
- Add `SearchMode::{Focused, Stable}`.
- In focused mode, choose the last queued unassigned variable.
- In focused mode, bump analyzed variables by moving them to the front of the queue in stamp order.
- In stable mode, keep the current heap/EVSIDS behavior.
- Add tests for queue ordering, backtrack reinsertion, and analyzed-variable movement.

Notes:

- This can be introduced as `SAT_SEARCH_MODE=stable|focused`.
- Do not add mode switching until each single mode is correct and measured.

Implementation status:

- Landed as an opt-in mode behind `SAT_SEARCH_MODE=focused`; default remains stable heap search.
- The stable hot path still uses the existing activity heap, variable activity bumps, and decay.
- Focused mode maintains a linked recent-conflict decision queue. Branching pops the queue front,
  analyzed variables are moved to the front instead of score-bumped, and backtracked variables are
  restored to the focused queue without rebuilding the heap.
- The normal branch heap remains populated in focused mode for invariants and future mode-switching
  work, but focused decisions read from the focused queue.
- No automatic mode switching, focused restart policy, or trail reuse is implemented yet.

Validation:

- `cargo test`: 73 passed.
- Default stable smoke: 9/9 passed, log `log/2026-05-10-21-31-59`.
- Focused invariant smoke: 9/9 passed, log `log/2026-05-10-21-32-04`.
- Default stable profiling: 6/11 solved, PAR-2 `1329.605`, log
  `log/bench-11-kissat-innovations-2026-05-10-21-32-20`.
- Focused profiling: 2/11 solved, PAR-2 `2288.046`, log
  `log/bench-11-kissat-innovations-2026-05-10-21-52-31`.
- Stable default stayed aligned with the previous glue-default profile, so the infrastructure did
  not show an obvious default hot-path regression.

Analysis:

- Focused-only is a negative result at this stage. It lost several stable solves, including
  `feistel_b64_k32_r22`, `feistel_b64_k52_r17`, `random_v285_s2`, and `random_v292_s4`.
- Direct no-proof trace on `feistel_b64_k57_r18`: stable solved in `3.992s` with `243188`
  conflicts, `304380` decisions, `18090356` propagations, `90` reduce passes, and `112.844ms`
  reduction time.
- The same instance in focused mode solved in `61.275s` with `933578` conflicts, `1062833`
  decisions, `81748523` propagations, `191417` reduce passes, and `32077.625ms` reduction time.
- The immediate issue is search trajectory plus learned-DB pressure. Focused mode should remain an
  explicit experiment until glue-EMA restarts/trail reuse or a reduction throttle exists.

### F. Add glue EMA restart policy and trail reuse

Goal:

- Replace the MiniSat/Luby-only restart behavior with Kissat-like restarts.

Implementation items:

- Add fast and slow EMAs for learned glue.
- Focused mode: after a conflict interval, restart if `fast_glue >= slow_glue * margin`.
- Stable mode: add reluctant doubling restart sequence.
- Implement restart trail reuse by choosing the deepest prefix whose decision variables remain better
  than the next decision candidate.
- Keep the old Luby policy as `SAT_RESTART_MODE=luby`.

Dependencies:

- Glue metadata from item B.
- Focused queue from item E for focused trail reuse.

### G. Add target/best phases and rephase schedule

Goal:

- Move phase behavior toward Kissat and reduce search-path stickiness.

Implementation items:

- Add `best_phase` and `target_phase` vectors alongside `saved_phase`.
- During stable-mode backtracking, save target/best phase snapshots when the assigned trail reaches a
  new high-water mark.
- Decision phase order: target, then saved, then initial phase.
- Add rephase events in stable mode using this schedule: best, walk, inverted, best, walk, original.
- Start without full local search: implement best, inverted, and original first; make walk a no-op or
  disabled placeholder until a local-search implementation exists.

Dependencies:

- Search mode and limit scheduler.

### H. Add chronological backtracking and reason-side bumping

Goal:

- Match two smaller but search-path-relevant Kissat behaviors in conflict handling.

Implementation items:

- Add a `chronolevels` threshold. If the jump would skip more than that many levels, backtrack only
  one level chronologically.
- Add optional reason-side bumping with a measured limit, guarded by decision-rate style counters.
- Add tests for chronological backtracking threshold behavior.

Dependencies:

- Glue/analysis metrics are helpful but not strictly required.

### I. Add eager subsumption of recently learned clauses

Goal:

- Implement a bounded Kissat feature with small scope.

Implementation items:

- Keep a ring/list of the last four long learned clauses.
- When a new learned clause is added, check whether it subsumes any recent larger learned clause.
- Mark subsumed clauses deleted and let normal GC clean them.
- Gate with `SAT_EAGER_SUBSUME_LAST=N`.

Notes:

- This is a lower-risk standalone experiment after learned-clause metadata is in place.

### J. Reintroduce inprocessing after search starts

Goal:

- Stop treating simplification as a one-shot preprocessing-only phase.

Implementation items:

- Add an inprocessing scheduler with conflict limits.
- First candidate: rerun a bounded version of existing BVE/BSR at root after enough conflicts.
- Second candidate: add vivification for selected irredundant and low-glue learned clauses.
- Third candidate: add binary transitive reduction after binary watch splitting.

Dependencies:

- Root transition path that backtracks, propagates, flushes root units, and resumes search cleanly.
- Binary implication representation for transitive reduction.

### K. Align BVE scoring with Kissat

Goal:

- Make existing solver-11 BVE choose variables more like Kissat.

Implementation items:

- Change BVE heap key from `pos * neg` to capped `(pos * neg - pos - neg) + search_relevancy`.
- Add occurrence cap and effort limits.
- In stable mode, use variable activity as relevancy. In focused mode, use queue stamp.
- Add diagnostic counters for skipped variables by occurrence cap, clause limit, grow limit, and
  effort limit.

Dependencies:

- Stable/focused data structures.

### L. Consider formula representation follow-ups

Goal:

- Make later Kissat ports less awkward.

Implementation items:

- Keep the Rust packed arena for now, but add helper accessors for learned metadata instead of
  scattering bit/extra-word layout logic.
- Add `searched` position for long clauses to avoid restarting every replacement search at literal
  position 2.
- Add explicit `garbage`, `reason`, `redundant`, and `shrunken` semantics if they simplify tiered
  reduction and vivification.

Notes:

- Do this only when a concrete feature needs the metadata. A full clause-layout rewrite before the
  algorithmic experiments would be too high risk.

### M. Add lucky SAT shortcuts and bounded lucky probing

Goal:

- Capture Kissat's cheap pre-search wins without disturbing normal CDCL behavior.

Implementation items:

- Implement no-all-negative/no-all-positive checks first.
- Add an optional four-pass lucky probe after binary implication watches exist.
- Route any SAT shortcut through model extension and existing output validation.

Dependencies:

- Basic version: current clause iterators.
- Full version: binary implication representation and probing propagation.

### N. Add warmup phase seeding

Goal:

- Seed `saved_phase` before real search begins.

Implementation items:

- Add a root-only warmup pass behind `SAT_WARMUP=1`.
- Decide and propagate using the current heuristic, save phases, then backtrack without overwriting
  them.
- Track warmup decisions, propagations, and elapsed time.

### O. Add root-level clause-weight decision reordering

Goal:

- Feed root formula structure into branching order the way Kissat does.

Implementation items:

- Compute literal and variable weights from irredundant clauses and binary clauses.
- Stable mode: rescale/add weights to the existing heap.
- Focused mode later: sort and move variables through the focused queue.

Dependencies:

- Stable-only version can happen early.
- Focused integration depends on item E.

### P. Add binary transitive reduction

Goal:

- Use the binary implication graph as an inprocessing target, not only a propagation fast path.

Implementation items:

- Probe reachability under a literal's negation.
- Remove transitive binary clauses with proof deletions.
- Learn root units when both a literal and its negation are implied.

Dependencies:

- Binary watch splitting from item D.
- Root-level inprocessing scheduler from item J.

### Q. Add on-the-fly strengthening diagnostics, then implementation

Goal:

- Measure and then port Kissat's conflict-time clause strengthening.

Implementation items:

- Add analysis counters for antecedent size, resolvent size, and possible shrink opportunities.
- Implement strengthening for non-binary learned clauses after proof handling is clear.
- Add tests that check both the learned clause and proof log.

Dependencies:

- Glue/used metadata from item B.
- Mature proof deletion/addition handling for modified clauses.

### R. Extract bounded forward subsumption as reusable inprocessing

Goal:

- Turn the simplifier's occurrence-list logic into a repeated root-level pass.

Implementation items:

- Add bounded forward subsumption and strengthening over occurrence lists.
- Include duplicate binary cleanup and hyper-unary unit detection.
- Gate by occurrence and clause-size limits.

Dependencies:

- Dense/simplification mode boundary from item V is preferred before repeated use.

### S. Add fast-BVE mode

Goal:

- Test a stricter, cheaper elimination pass separate from full MiniSat-style BVE.

Implementation items:

- Add low occurrence and clause-size caps.
- Special-case binary-binary and binary-large resolvents.
- Compare `none`, `fast`, `full`, and `fast+full` preprocessing modes.

### T. Port vivification scheduling before full vivification

Goal:

- Understand candidate volume and selection quality before implementing expensive probing.

Implementation items:

- Rank candidates by irredundant/learned tier, size, occurrence counts, and previous vivify marker.
- Report candidate counts and estimated tick budget.
- Implement irredundant-only vivification before learned-clause vivification.

Dependencies:

- Glue tiers from item C.
- Probing/root inprocessing from item J.

### U. Start structural simplification with binary equivalences

Goal:

- Introduce the lowest-risk part of Kissat's gate/congruence family.

Implementation items:

- Detect equivalent literals from pairs of binary implications.
- Substitute representatives at root with proof logging.
- Defer AND/ITE/congruence closure until dense mode and substitution are well tested.

Dependencies:

- Binary implication representation.
- Root-level formula rewrite support.

### V. Add explicit dense/sparse representation transitions

Goal:

- Make repeated simplification passes share one robust occurrence-list lifecycle.

Implementation items:

- Add `enter_simplification_mode` and `resume_search_mode`.
- Rebuild or flush watches at the boundary.
- Share occurrence lists across BVE, forward subsumption, vivification candidate selection, and
  future gate detection.

Dependencies:

- This should precede broad mid-search inprocessing.

### W. Instrument clause GC and consider sparse collection later

Goal:

- Keep arena cost visible before copying Kissat's collection machinery.

Implementation items:

- Add counters for deleted clauses, deleted words, copied words, and GC time.
- Add learned/original region metadata only if reduction and vivification make full GC too costly.

### X. Track factorization / bounded variable addition as late-stage work

Goal:

- Keep Kissat's variable-addition technique on the roadmap without pulling it into the first wave.

Implementation items:

- Revisit after dense mode, proof logging for substitutions, and gate-aware simplification exist.
- Benchmark only on formulas where Kissat's structural simplification visibly beats solver 11.

## Infrastructure-First Implementation Order

The first implementation pass should not chase the easiest visible features. Lucky assignment,
warmup, and stable-only reordering are attractive because they are small, but they do not create the
foundation needed by most Kissat-style changes. The central dependencies are:

- richer clause/reason metadata for glue, tiers, binary reasons, and proof-safe modification;
- a real binary implication representation;
- a root-level maintenance scheduler;
- explicit search-mode and simplification-mode transitions;
- a reusable occurrence-list/dense view for repeated formula rewriting.

The order below prioritizes infrastructure that other features will build on. Each phase should land
with behavior either unchanged or guarded by an environment flag, plus unit tests and the solver 11
smoke suite.

### Phase 0. Baseline observability and invariants

Primary roadmap items:

- A. Kissat-facing diagnostics
- W. Clause-GC instrumentation

Build first:

- Add counters for learned clause size, glue placeholder, backtrack level, propagation throughput,
  live/deleted clauses, deleted words, copied GC words, proof bytes, preprocessing time, and search
  maintenance time.
- Add internal consistency checks that can be enabled in tests: watcher consistency, reason
  consistency, learned/original clause lists, and proof/model-extension stack shape.
- Keep the solver behavior unchanged.

Why first:

- The following phases are representation-heavy. Without counters and invariants, it will be too
  easy to mistake a search-path change for an implementation win or miss a slow proof/GC regression.

Unlocks:

- Safe measurement for every later phase.

Status:

- Implemented on 2026-05-09.
- Added behavior-preserving counters for learned clause size distribution, deletion/shrink words,
  GC copied/reclaimed words and time, simplify/reduce timing, preprocessing/search timing, and proof
  clause/byte counts.
- Added `SAT_CHECK_INVARIANTS=1` for expensive consistency checks over trail/reasons, live clause
  lists, watchers, branch heap positions, occurrence lists when present, clause abstractions, and
  model-extension stack shape.
- Extended `SAT_TRACE_PREPROCESS` and `SAT_TRACE_SEARCH_INTERVAL` output with the new counters.
- Validation: `cargo test` passed 50 tests; normal smoke passed 9/9; invariant smoke with
  `SAT_CHECK_INVARIANTS=1` passed 9/9.
- Profiling overhead check on `benchmarks/profiling`, timeout 120s, memory 16 GB:
  baseline log `log/bench-11-kissat-innovations-2026-05-09-17-38-22`, PAR-2 `1100.087`, solved
  7/11; instrumented log `log/bench-11-kissat-innovations-2026-05-09-17-55-01`, PAR-2 `1098.830`,
  solved 7/11. Same solved/timeout split; runtime delta is within normal benchmark noise.

### Phase 1. Clause metadata and tagged reasons

Primary roadmap items:

- B. Glue/LBD and used counters
- L. Formula representation follow-ups
- Q. On-the-fly strengthening diagnostics only

Build first:

- Add explicit accessor functions for clause metadata instead of scattering packed-word layout
  knowledge.
- Add learned-clause fields for `glue`, `used`, `tier`, `searched`, `reason`, and `shrunken` where
  practical, even if some are initially diagnostics-only.
- Replace raw optional clause ids in assignments with a tagged reason type:
  `Decision`, `Unit`, `Clause(id)`, and eventually `Binary(lit)`.
- Compute glue/LBD during analysis and record it, but keep the existing reducer and restart policy
  until later phases.
- Add diagnostics for on-the-fly strengthening opportunities without modifying clauses.

Why before binary watches:

- Binary clauses need a different reason representation. Doing tagged reasons first reduces the
  blast radius of the binary propagation refactor.

Unlocks:

- C. Glue-tiered reduction
- F. Glue EMA restarts
- T. Vivification tiering
- Q. Actual on-the-fly strengthening
- D. Binary reason support

### Phase 2. Binary implication watches and propagation split

Primary roadmap item:

- D. Split binary clauses into a fast implication path

Build first:

- Store binary clauses as direct implication watches and keep large clauses in the arena.
- Propagate binary implications before large watched clauses.
- Teach analysis, reason lookup, proof emission, model checking, clause iteration, and deletion
  accounting about binary reasons.
- Preserve a full iterator over all logical clauses, including binary clauses, for validation and
  proof/model logic.

Why this early:

- This is the most fundamental Kissat representation gap. It is also the prerequisite for probing,
  transitive reduction, binary equivalence detection, duplicate-binary cleanup, and faster BVE
  special cases.

Status:

- Representation setup landed as the first slice: solver 11 now keeps a compact binary-clause
  mirror table with `lit0`, `lit1`, and packed metadata, plus live original/learned binary id
  lists and an arena-offset bridge. Existing arena clauses and generic watchers still drive
  propagation.
- Direct per-literal binary implication lists were intentionally left for the propagation split.
  An earlier trial maintained unused implication lists and showed avoidable overhead on the profile
  bench.
- Propagation split first slice landed on 2026-05-10. Binary clauses still remain in the arena and
  still have normal watchers, so proof logging, clause iteration, conflict analysis, and reason
  references continue to use `ReasonRef::Clause(arena_idx)`. The new fast path recognizes binary
  clauses inside the existing watcher schedule and handles the implication/conflict case without
  running the long-clause replacement scan. `SAT_BINARY_FAST_PATH=0` disables this for comparison.
- A more aggressive direct binary-first queue is implemented as the default experiment. It keeps arena
  binary clauses as proof/reason sources, builds per-literal binary implication lists keyed by the
  falsified literal, and processes those lists before scanning the normal watcher list. After the
  2026-05-10 fix, aggressive mode no longer attaches live binary clauses to normal watcher lists;
  this removed the duplicate representation that made direct-first do binary work twice.
- Aggressive mode is the default again for this development branch. It is still high-risk because
  direct-first changes propagation/conflict ordering enough to hurt the standard profile bench, but
  keeping it default forces follow-up work to fix the direct implication path instead of leaving it
  as a side experiment. Use `SAT_BINARY_PROP_MODE=generic`, `watcher`, or `aggressive` to compare
  modes; the older `SAT_BINARY_FAST_PATH=0/1` still maps to `generic`/`watcher` when
  `SAT_BINARY_PROP_MODE` is unset.
- Representation validation: `cargo test` passed 56 tests; normal smoke passed 9/9; invariant
  smoke with `SAT_CHECK_INVARIANTS=1` passed 9/9.
- Profiling overhead check on `benchmarks/profiling`, timeout 120s, memory 16 GB:
  pre-change log `log/bench-11-kissat-innovations-2026-05-09-22-21-25`, PAR-2 `1101.501`,
  solved 7/11; representation log `log/bench-11-kissat-innovations-2026-05-09-22-56-02`, PAR-2
  `1101.544`, solved 7/11. Same solved/timeout split; PAR-2 delta was `+0.043s`.
- Propagation fast-path benchmark on `benchmarks/profiling`, timeout 120s, memory 16 GB:
  pre-change log `log/bench-11-kissat-innovations-2026-05-09-23-46-49`, PAR-2 `1099.164`, solved
  7/11; fast-path log `log/bench-11-kissat-innovations-2026-05-10-00-53-26`, PAR-2 `1098.463`,
  solved 7/11. Same solved/timeout split; PAR-2 delta was `-0.701s`. A disabled-fast-path
  comparison log `log/bench-11-kissat-innovations-2026-05-10-00-36-51` scored PAR-2 `1109.380`,
  which appears to be benchmark/search noise rather than a structural change because the enabled
  fast path preserved the same solved set and was closer to the pre-change aggregate.
- Fast-path validation: `cargo test` passed 58 tests; normal smoke passed 9/9; invariant smoke with
  `SAT_CHECK_INVARIANTS=1` passed 9/9.
- Direct-first representation validation after the duplicate-watch fix: `cargo test` passed 60
  tests; normal smoke passed 9/9; invariant smoke with `SAT_CHECK_INVARIANTS=1` passed 9/9.
- Circuit-focused aggressive check on 2026-05-10: a binary-heavy pair
  (`multiplier_16bits__miter_22`, 66.8% binary clauses; `velev-pipe-o-uns-1.1-6`, 88.3% binary)
  timed out in both previous generic/watcher runs and the new aggressive run. Aggressive log
  `log/bench-11-kissat-innovations-2026-05-10-08-41-35`, timeout 120s, solved 0/2, PAR-2
  `480.000`.
- Smaller circuit check on 2026-05-10: three 36-46% binary hardware/sqrt instances timed out in
  previous generic/watcher runs and in the aggressive run. Aggressive log
  `log/bench-11-kissat-innovations-2026-05-10-08-45-39`, timeout 60s, solved 0/3, PAR-2 `360.000`.
  Initial fixed-time traces showed the aggressive path did less work because binary clauses were
  represented in both direct implication lists and normal watcher lists. After removing the normal
  binary watchers in aggressive mode, the smaller 60e396 circuit improved from about `750k`
  conflicts in 25s to `800k`, closer to watcher mode at `810k` and generic mode at `820k`.
- Post-fix perf counters on the 60e396 circuit, 20s wall clock with `SAT_PROOF=0`, show the
  duplicate work is gone at the instruction level: generic `168.2B` instructions / `32.17B`
  branches, watcher `167.2B` / `32.03B`, aggressive `165.3B` / `31.52B`. Aggressive still has
  worse cache behavior (`30.32%` cache-miss rate versus watcher `28.48%`), and sampled profile
  `/tmp/perf-post-60e-aggressive.report` still has `Solver::propagate` as the dominant cost
  (`73.16%` cycles) with direct binary implication work only visible as an inlined subpath.
- Post-fix fixed-time traces on `velev-pipe-o-uns-1.1-6` show the remaining issue is search-path
  sensitivity, not duplicate watcher work: generic/watcher reached `415k` conflicts by about
  `26.9s` search time and learned one root unit, while aggressive reached `380k` conflicts, learned
  no root unit, and had much worse learned-clause glue (`51.79` average versus `38.49`).
- Standard profile bench after the duplicate-watch fix but with aggressive as the default regressed
  badly: log `log/bench-11-kissat-innovations-2026-05-10-09-15-37`, timeout 120s, memory 16 GB,
  solved 6/11, PAR-2 `1379.890`. The profile run lost `mp1-Nb7T46` to timeout and regressed
  `feistel_b64_k32_r22` from the older watcher-fast `15.386s` to `92.276s`.
- Restoring watcher-fast in a diagnostic run recovered the profile bench: log
  `log/bench-11-kissat-innovations-2026-05-10-09-37-56`, timeout 120s, memory 16 GB, solved 7/11,
  PAR-2 `1093.005`, versus the previous watcher-fast log
  `log/bench-11-kissat-innovations-2026-05-10-00-53-26`, solved 7/11, PAR-2 `1098.463`.
  This confirmed the regression comes from aggressive direct-first propagation rather than the
  binary representation setup itself. The temporary watcher-fast restore validated with `cargo
  test`, normal smoke, and invariant smoke before aggressive was made default again.
- The debug workflow that found the cache issue is now captured in `CLAUDE.md` under
  "Debugging Optimization Regressions": compare fixed-time mode traces, separate preprocessing from
  search with delayed `perf stat`, normalize cache/TLB counters by propagation or conflict count,
  sample the suspected event with `perf record -e cache-misses`, inspect source with
  `perf annotate`, and record both microarchitectural effects and CDCL search-path changes.
- Follow-up cache fix on 2026-05-10: aggressive direct binary implications now carry compact
  binary ids and successful implications use `ReasonRef::binary(false_lit)`, so the common
  non-conflicting binary implication path checks compact binary metadata instead of reading arena
  headers/literals and rotating the arena reason clause. Conflict analysis, deep/basic
  minimization, and invariants now understand binary reasons directly; arena binary clauses remain
  available for proof/conflict materialization and clause iteration.
- Local cache/TLB result: on 60e396, aggressive dTLB misses dropped from `6.61M` to `5.16M` and
  L1-dcache-load misses from `4.14B` to `4.08B` over the same 20s wall-clock perf run. On
  `velev-pipe-o-uns-1.1-6`, search-only delayed perf counters improved from `208.98B`
  instructions / `40.36B` branches / `5.50B` L1 misses / `1.25B` dTLB misses to `194.86B` /
  `36.50B` / `5.16B` / `1.04B`. The source-level cache-miss sample
  `/tmp/perf-cachemiss-60e-aggressive-binaryreason.report` no longer attributes
  `clause_is_deleted` misses to `propagate_binary_implications`; remaining arena header misses come
  from the normal watcher scan.
- Macro result: the cache fix is not enough to make aggressive mode a performance win yet.
  Aggressive profile log
  `log/bench-11-kissat-innovations-2026-05-10-10-39-25`, timeout 120s, memory 16 GB, solved 5/11,
  PAR-2 `1562.713`. It improved some instances (`feistel_b64_k52_r17` `18.404s` -> `11.559s`,
  `feistel_b64_k57_r18` `6.229s` -> `1.774s`, `random_v292_s4` `17.688s` -> `8.936s`) but kept
  the severe `feistel_b64_k32_r22` regression (`92.355s`) and newly timed out the timetable
  instance. Conclusion: keep aggressive as the default for this development branch, but treat it as
  an unfinished foundation. The next work needs ordering/search-path compatibility before direct
  binary-first propagation should be considered a performance improvement.
- Fresh low-noise aggressive-path pass on 2026-05-10: fixed-time `perf stat` runs compared clean
  `HEAD` against a count-array experiment on identical search traces with `SAT_PROOF=0` and
  `SAT_BINARY_PROP_MODE=aggressive`. The profiler showed the direct binary path still paying a
  random outer-`Vec` metadata load for every propagated literal; on 60e396 the pre-change
  `/tmp/sat-aggressive-analysis/perf-cache-aggressive.data` cache sample put `5.92%` local
  cache-miss weight on the binary implication `Vec::len`/empty test. Adding a contiguous
  `binary_implication_counts: Vec<u32>` makes the empty-list test a dense array load and only
  touches the per-literal `Vec` for non-empty lists.
- Count-array result on 60e396, same 700k-conflict trace and same 20s perf window after a 2s delay:
  baseline clean worktree `167.22B` instructions / `30.85B` branches / `74.86B` L1 loads /
  `4.088B` L1 misses / `5.38M` dTLB misses / `1.878B` cache misses; count-array build `166.13B`
  instructions / `30.65B` branches / `73.27B` L1 loads / `4.051B` L1 misses / `4.56M` dTLB
  misses / `1.823B` cache misses. This is a low-noise improvement in the target access pattern,
  not a claim about the instance solve trajectory.
- Count-array result on `velev-pipe-o-uns-1.1-6`, same 300k-conflict trace and same 20s search-only
  perf window after a 9s delay: baseline `155.12B` instructions / `29.09B` branches / `57.88B`
  L1 loads / `4.188B` L1 misses / `3.204B` cache misses; count-array build `146.89B`
  instructions / `27.64B` branches / `53.65B` L1 loads / `3.962B` L1 misses / `3.127B` cache
  misses. dTLB misses were essentially unchanged on this larger instance (`749.37M` -> `749.93M`).
- Rejected in the same pass: skipping normal watcher `mem::take` for empty watcher lists was mixed
  and moved dTLB/instruction counters the wrong way; moving `clause_idx` out of `BinaryImplication`
  shrank implication entries but worsened instructions/branches/L1/cache counters, likely because
  the larger `BinaryClause` table and changed codegen outweighed the stride reduction; replacing
  the binary id bounds check with unchecked indexing also worsened the robust counters. Next
  promising non-search direction is a larger representation change that removes the per-literal
  `Vec<Vec<_>>` metadata/allocation pattern entirely, such as a flat static implication segment for
  original binaries plus a tiny dynamic side structure for rare learned binary clauses.
- Second low-noise aggressive-path pass on 2026-05-10 found no additional small default change worth
  keeping. Current `HEAD` on 60e396, same fixed 700k-conflict trace and 20s perf window after a 2s
  delay, measured `165.43B` instructions / `30.53B` branches / `73.23B` L1 loads / `4.044B` L1
  misses / `4.67M` dTLB misses / `1.801B` cache misses. Cache sampling
  `/tmp/sat-aggressive-analysis/perf-cache-current-60e.data` still put most cache misses in the
  normal watcher scan (`Solver::propagate` `83.95%`, deleted-clause arena header checks `16.80%`);
  the direct binary implication loop was only `4.72%` of sampled cache misses.
- Rejected in that pass: removing the release deleted-binary check from direct implications and
  shrinking implications to two words helped high-binary Velev cache/TLB counters but worsened 60e
  instruction/branch/dTLB counters (`167.89B` instructions, `31.82B` branches, `5.29M` dTLB
  misses). Keeping the 12-byte implication shape while skipping the release deleted check was also
  worse on 60e (`168.70B` instructions, `31.94B` branches, `4.160B` L1 misses). A dense
  `watch_counts` array for normal watcher-list emptiness was clearly bad on 60e (`174.14B`
  instructions, `4.264B` L1 misses, `5.65M` dTLB misses). Reordering the watcher-fast binary branch
  to check `clause_len == 2` first reduced some miss counts but reached fewer conflicts in the same
  window and raised instruction/L1-load counts.
- Takeaway: after the count-array improvement, the aggressive direct-binary path is not the main
  cache-miss source on the sampled circuit target. The next worthwhile non-search work is not
  another small hot-loop tweak; it is either a flat static binary-implication representation tested
  across a broader binary-density sample, or a normal watcher data-structure cleanup aimed at arena
  header locality and deleted-watcher churn.
- Deeper representation experiment on 2026-05-10 split those two directions. Clause mixes used:
  60e396 has `45.7%` binary clauses after input parsing, Velev `velev-pipe-o-uns-1.1-6` has
  `88.3%` binary clauses, and `circuit_48in64out_with_800gates...` has no binary clauses and is a
  cleaner normal-watcher target. A prototype `SAT_BINARY_STATIC_SEGMENTS=1` builds flat original
  binary implication segments after preprocessing and keeps learned binaries in the existing dynamic
  per-literal lists. It preserves the exact search trace on the measured targets. On Velev it reached
  200k conflicts in `9.791s` vs baseline `10.529s` (`~7.0%` faster) while search-window counters
  moved from `126.83B` instructions / `3.409B` L1 misses / `624.9M` dTLB misses /
  `2.647B` cache misses to `135.64B` instructions / `3.348B` L1 misses / `579.0M` dTLB misses /
  `2.472B` cache misses. Cache profiles show the dynamic binary implication loop dropping from
  `12.4%` of sampled cache misses to roughly `3.7%` in the static path
  (`/tmp/sat-aggressive-analysis/perf-cache-baseline-velev.data` and
  `/tmp/sat-aggressive-analysis/perf-cache-static-velev.data`).
- Static binary segments were mildly positive but less decisive on mixed/no-binary circuits: 60e396
  reached the same 700k-conflict trace in `20.927s` vs `21.527s` baseline, but with higher
  instruction/branch/L1-load counts; the no-binary circuit reached 250k conflicts in `16.199s` vs
  `16.404s`, also with mixed cache counters. Interpretation: flat static original binaries are
  worth implementing as a real default or auto-enabled feature, but the implementation should avoid
  extra empty-segment work on formulas with few or no original binaries and should be validated on a
  broader binary-density sample.
- Watcher-storage cleanup was less convincing. Stable eager detach after preprocessing preserved
  60e396's search trace but only improved 700k-conflict time from `21.527s` to `21.258s` while
  increasing instruction/branch/cache-reference counts. On the no-binary circuit it improved 250k
  conflicts from `16.404s` to `16.072s` and cut dTLB misses from `120.7M` to `62.5M`, but total
  cache misses were roughly flat. A bulk `SAT_COMPACT_WATCHERS_AFTER_REDUCE=1` pass preserved the
  search trace and avoided per-clause `Vec::remove`; it reached 250k conflicts in `16.151s` on the
  no-binary circuit and 700k in `21.306s` on 60e396. This confirms deleted watcher tombstones hurt
  locality, but the simple cleanup policies are below the normal keep threshold. Revisit with a more
  structural watcher layout change rather than eager detach as-is.
- Production static-binary implementation on 2026-05-10: `SAT_BINARY_STATIC_SEGMENTS` now defaults
  to `auto`, with explicit `on`/`off` overrides. Auto mode enables only in aggressive propagation
  when the post-preprocessing formula has at least `64` live original binary clauses and either at
  least `20%` original-binary density or at least `4096` live original binaries. Static segments
  are rebuilt from live original binaries after preprocessing/simplification; learned binaries stay
  in the existing dynamic lists. The trace line now reports `original_binary`, `learned_binary`,
  `static_binary`, and `static_implications`.
- Production before/after fixed-window checks used the default solver before this change and the new
  auto mode after it. Velev enabled static segments (`261940` original binaries, `523880` static
  implications) and preserved the exact search trace: 200k conflicts moved from `10.690s` to
  `9.793s`; search-window counters moved from `127.65B` instructions / `3.467B` L1 misses /
  `626.8M` dTLB misses / `2.677B` cache misses to `134.48B` instructions / `3.340B` L1 misses /
  `637.3M` dTLB misses / `2.522B` cache misses. Cache profile
  `/tmp/sat-aggressive-analysis/perf-cache-prod-after-velev.data` shows the dynamic binary
  implication cache-miss subpath gone; the static binary path is only about `3.8%` combined sampled
  cache-miss weight, versus `12.4%` for the old dynamic path in
  `/tmp/sat-aggressive-analysis/perf-cache-baseline-velev.data`.
- Guard checks stayed disabled as intended: 60e396 had only `653` live original binaries after
  preprocessing and `static_binary=false`, reaching the same 700k-conflict trace in `21.168s` vs
  `21.295s` before, with mixed counters; the no-binary circuit acquired only `378` original binaries
  after preprocessing and also stayed disabled, reaching 250k conflicts in `16.108s` vs `16.181s`
  before. Validation: `cargo test` passed 61 tests and smoke passed 9/9 with DRAT checking, log
  `log/2026-05-10-16-04-06`; forced static segments with `SAT_CHECK_INVARIANTS=1` also passed
  smoke 9/9, log `log/2026-05-10-16-05-06`.

Unlocks:

- M. Full lucky probing
- P. Binary transitive reduction
- R. Duplicate binary cleanup and binary-large strengthening
- S. Fast binary-heavy elimination
- U. Binary equivalence detection

### Phase 3. Root-level maintenance scheduler and state transitions

Primary roadmap items:

- J. Reintroduce inprocessing after search starts
- O. Root-level reordering hook
- G. Rephase hook

Build first:

- Add a central maintenance scheduler with conflict/tick limits for reduce, restart, reorder,
  rephase, probe, and eliminate.
- Add a reliable `return_to_root_for_maintenance` path: backtrack, propagate root units, handle
  inconsistency, run one scheduled action, then resume search.
- Initially wire most actions as no-ops or diagnostics-only events.
- Keep the existing Luby restart and one-shot preprocessing behavior until the new hooks are proven.

Why before inprocessing:

- Kissat-style simplification is scheduled work. Adding individual root passes without a scheduler
  will create one-off paths that later need to be removed.

Unlocks:

- O. Clause-weight reordering
- G. Rephase schedule
- J. Repeated BVE/BSR
- P. Transitive reduction
- T. Vivification
- U. Equivalence substitution

Status:

- Initial scaffolding landed on 2026-05-10 without enabling any real new inprocessing action.
- The no-conflict search branch now goes through `run_search_maintenance`, preserving the existing
  order: pending Luby restart, root simplify, learned-clause reduction, then decision.
- Added `return_to_root_for_maintenance` for future root-only actions. It backtracks to level 0,
  propagates root assignments, detects root inconsistency, runs one action, and resumes search.
- Added disabled-by-default diagnostic intervals for root-level reorder, rephase, probe, and
  eliminate hooks (`SAT_MAINT_REORDER_INTERVAL`, `SAT_MAINT_REPHASE_INTERVAL`,
  `SAT_MAINT_PROBE_INTERVAL`, `SAT_MAINT_ELIMINATE_INTERVAL`). These hooks are currently no-ops and
  exist to validate the scheduler/transition path before real algorithms are attached.
- Added maintenance counters and trace fields for action counts and elapsed maintenance time.
- Validation: `cargo test` passed 64 tests; smoke passed 9/9 with DRAT checking, log
  `log/2026-05-10-17-20-27`.
- Fresh behavior check on `benchmarks/profiling`, timeout 120s, memory 16 GB: pre-change baseline
  `log/bench-11-kissat-innovations-2026-05-10-17-01-46` solved 5/11 with PAR-2 `1561.642`;
  post-change `log/bench-11-kissat-innovations-2026-05-10-17-20-43` solved 5/11 with PAR-2
  `1559.767`. Same solved/timeout split; the measured delta is noise-level.

### Phase 4. Dense/sparse simplification mode and rewrite boundary

Primary roadmap item:

- V. Explicit dense/sparse representation transitions

Build first:

- Add `enter_simplification_mode` and `resume_search_mode` boundaries.
- Build reusable occurrence lists once per simplification window.
- Define the legal operations while in dense mode: root propagation, clause deletion, clause
  strengthening, unit enqueue, variable elimination, substitution, and proof logging.
- Generalize solver 10's model-extension stack so repeated simplification can extend models in the
  right order.
- Reconnect watchers and reset propagation state when returning to sparse search mode.

Why before more simplifiers:

- Forward subsumption, fast BVE, vivification candidate selection, gates, congruence, and
  substitution all need occurrence data. They should share one lifecycle rather than rebuilding
  incompatible ad hoc structures.

Unlocks:

- R. Forward subsumption
- S. Fast BVE
- T. Vivification candidate scheduling and propagation
- U. Gate/equivalence substitution
- X. Later factorization/BVA

Status:

- Initial boundary landed on 2026-05-10 as a wrapper around the existing upfront MiniSat-style
  `eliminate(true, proof_log)` path, not as a replacement for it.
- Added `FormulaMode::{SparseSearch, DenseSimplification}` and explicit
  `enter_simplification_mode` / `resume_search_mode_after_simplification` hooks.
- `enter_simplification_mode` now owns occurrence-list construction for the simplification window.
  `resume_search_mode_after_simplification` clears occurrence metadata, optionally turns off the
  one-shot simplifier, rebuilds the branch queue, and runs the same post-preprocessing GC path as
  before.
- Added dense-boundary counters and `SAT_TRACE_PREPROCESS` fields for dense entries, resumes,
  occurrence-build time, and resume time.
- No repeated inprocessing action is enabled yet; this only names and tests the representation
  transition that future scheduled passes will use.
- Validation: `cargo test` passed 67 tests; normal smoke passed 9/9 with DRAT checking, log
  `log/2026-05-10-18-34-16`; invariant smoke with `SAT_CHECK_INVARIANTS=1` passed 9/9, log
  `log/2026-05-10-18-53-12`.
- Fresh behavior check on `benchmarks/profiling`, timeout 120s, memory 16 GB: pre-change baseline
  `log/bench-11-kissat-innovations-2026-05-10-18-15-21` solved 5/11 with PAR-2 `1559.776`;
  post-change `log/bench-11-kissat-innovations-2026-05-10-18-34-31` solved 5/11 with PAR-2
  `1559.650`. Same solved/timeout split; all solved-instance timing deltas were below `0.1s`.

### Phase 5. Learned-clause lifecycle policy

Primary roadmap items:

- C. Glue-tiered reduction
- I. Eager subsumption of recent learned clauses
- Q. On-the-fly strengthening implementation, after diagnostics

Build first:

- Replace the activity-only reducer with a default glue/used/tier reducer and keep the activity
  reducer available via `SAT_REDUCE_MODE=activity`.
- Promote/recompute glue when reason clauses are used.
- Add eager subsumption over the last few learned clauses.
- Implement on-the-fly strengthening only after proof additions/deletions for modified clauses are
  covered by tests.
- Use the Phase 0 GC counters to decide whether sparse collection needs to move earlier.

Why after metadata and scheduler:

- Reduction depends on Phase 1 metadata, and repeated reduce events should run through the Phase 3
  scheduler.

Unlocks:

- F. Glue-based restarts
- T. Learned-clause vivification
- More accurate clause database pressure for inprocessing limits

### Phase 6. Search-mode abstraction and decision infrastructure

Primary roadmap items:

- E. Focused mode decision queue
- O. Focused integration for clause-weight reordering
- K. Search-relevant BVE scoring

Build first:

- Add `SearchMode::{Stable, Focused}` and isolate decision selection behind a mode-aware API.
- Keep the existing heap as stable mode.
- Add focused queue links/stamps and analyzed-variable move-to-front behavior.
- Add a mode-independent way to ask for variable search relevance: stable score or focused queue
  stamp.
- Test stable and focused modes independently before adding automatic switching.

Status:

- The basic stable/focused decision abstraction is implemented and tested.
- Focused-only currently performs much worse than stable on the profiling set, mainly due to a worse
  search trajectory and excessive reduce-DB pressure. Keep stable as default.
- The search-relevance API for BVE/reorder is not implemented yet; focused queue stamps/recency can
  be exposed when those features are built.

Why after reasons and scheduler:

- Focused mode changes analysis bumping, decision order, restart behavior, and BVE scoring. It needs
  clear assignment/reason metadata and a scheduler for later mode switches.

Unlocks:

- F. Focused restart/trail reuse
- K. Kissat-like BVE scoring
- O. Focused reorder
- Mode switching

### Phase 7. Restart, backtracking, and phase system

Primary roadmap items:

- F. Glue EMA restart policy and trail reuse
- G. Target/best phases and rephase schedule
- H. Chronological backtracking and reason-side bumping
- N. Warmup phase seeding

Build first:

- Add fast/slow glue EMAs using Phase 1 glue data.
- Add stable reluctant restart and focused glue-EMA restart behind `SAT_RESTART_MODE`.
- Add target and best phase arrays, then rephase scheduling through the Phase 3 scheduler.
- Add warmup as a phase-seeding pass once phase arrays exist.
- Add chronological backtracking and reason-side bumping after restart and mode behavior are
  measurable.

Why after search modes:

- Kissat's restart and rephase logic is mode-sensitive. Implementing it before focused/stable
  abstractions would hard-code the current MiniSat-like path and make mode switching harder.

Unlocks:

- More faithful Kissat search behavior before adding expensive inprocessing.

### Phase 8. First simplification features on the new infrastructure

Primary roadmap items:

- M. Lucky SAT shortcut and bounded lucky probing
- O. Clause-weight reordering
- K. BVE scoring alignment
- R. Bounded forward subsumption
- S. Fast BVE

Build first:

- Add no-all-positive/no-all-negative lucky shortcut first; add full four-pass lucky probing only
  after binary propagation is stable.
- Add stable-mode clause-weight reordering, then focused-mode reordering.
- Update BVE scoring to use capped occurrence product plus search relevance.
- Add fast BVE and bounded forward subsumption in dense mode.

Why here:

- These are the first visible algorithmic wins that use the major foundations without requiring full
  vivification, congruence, or gate substitution.

Unlocks:

- A measured simplification/search baseline before adding deeper probing passes.

### Phase 9. Probing and structural inprocessing

Primary roadmap items:

- P. Binary transitive reduction
- T. Vivification scheduling and then vivification
- U. Binary equivalence substitution

Build first:

- Add candidate-only vivification scheduling and report counts/ticks before mutating clauses.
- Add binary transitive reduction with strict tick limits and proof deletion tests.
- Add binary equivalence detection/substitution as the first structural substitution pass.
- Add irredundant-only vivification before learned-clause vivification.

Why after Phases 2-8:

- These passes combine binary implication traversal, dense occurrence data, root scheduling, proof
  mutation, and glue tiers. They should not be first-wave changes.

Unlocks:

- Gate-aware simplification and broader Kissat-style probing.

### Phase 10. Heavy structural simplification

Primary roadmap items:

- U. Full gate extraction, congruence, and substitution
- X. Factorization / bounded variable addition

Build first:

- Extend binary equivalence substitution into AND/ITE/definition gate extraction.
- Add congruence closure only after substitution proof/model-extension behavior is routine.
- Consider factorization/BVA only for benchmark families where Kissat's structural passes show a
  clear gap and the earlier infrastructure is stable.

Why last:

- These features have the highest proof/model-extension complexity and are least useful without the
  dense/sparse mode, scheduler, binary graph, and rewrite boundary.

## Roadmap Item Placement

| Item | Primary phase | Notes |
| --- | --- | --- |
| A diagnostics | 0 | First non-behavioral baseline. |
| B glue/LBD/used | 1 | Store metadata before changing policies. |
| C glue-tiered reduce | 5 | Needs metadata and scheduled reduction. |
| D binary implication path | 2 | Core representation dependency. |
| E focused queue | 6 | Needs decision API isolation. |
| F glue restarts/trail reuse | 7 | Needs glue, modes, scheduler. |
| G target/best/rephase | 7 | Needs scheduler and phase arrays. |
| H chronological backtracking/reason bump | 7 | Best measured after restart changes. |
| I eager subsumption | 5 | Learned-clause lifecycle feature. |
| J inprocessing scheduler | 3, 8, 9 | Scheduler first, actual passes later. |
| K BVE scoring | 8 | Needs search relevance from Phase 6. |
| L representation follow-ups | 1, 5 | Accessors early; sparse layout only if needed. |
| M lucky | 8 | Simple shortcut can be earlier, full probing needs Phase 2. |
| N warmup | 7 | More coherent after phase arrays exist. |
| O reorder | 3, 6, 8 | Hook in scheduler, then stable/focused implementations. |
| P transitive reduction | 9 | Needs binary graph and scheduler. |
| Q on-the-fly strengthening | 1, 5 | Diagnostics early, mutation later. |
| R forward subsumption | 8 | Needs dense occurrence lifecycle. |
| S fast BVE | 8 | Needs dense mode and binary special cases. |
| T vivification | 9 | Needs glue tiers, scheduler, dense mode. |
| U gates/congruence/equivalence | 9, 10 | Binary equivalence first; gates/congruence later. |
| V dense/sparse mode | 4 | Foundational for repeated simplification. |
| W GC instrumentation | 0, 5 | Measure early; optimize after reduction pressure is real. |
| X factorization/BVA | 10 | Late-stage structural feature. |

## First Concrete Milestones

1. Land Phase 0 and Phase 1 together if the patch remains manageable: diagnostics, metadata
   accessors, tagged reasons, glue computation, and no policy changes.
2. Land Phase 2 as a standalone propagation refactor: binary implication watches plus proof/model
   iteration support. This should get the heaviest unit-test coverage.
3. Land Phase 3 and Phase 4 as infrastructure with no major new algorithms: scheduler hooks,
   root-maintenance transition, dense/sparse mode, reusable occurrence lists.
4. Land Phase 5 and Phase 6 incrementally: glue-tiered reduction as the default with activity
   fallback, and focused queue behind a flag, still without automatic mode switching.
5. Only then start Phase 8 visible algorithmic work: lucky shortcut/probing, clause-weight reorder,
   Kissat-style BVE scoring, fast BVE, and bounded forward subsumption.
