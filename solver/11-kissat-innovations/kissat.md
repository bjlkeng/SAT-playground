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
- It has a central maintenance scheduler with real rephase support and default root-safe guarded
  stable/focused switching through `SAT_MODE_SWITCH_INTERVAL=50000`. The
  `SAT_MODE_SWITCH_POLICY=stale-stable` guard only enters focused mode after stable search stops
  reaching deeper trails, and focused mode now has a short dwell cap before returning to stable.
  Reorder, probing, and inprocessing elimination hooks are still no-ops.
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

- Uses stable heap search by default.
- Has an opt-in focused recent-conflict queue through `SAT_SEARCH_MODE=focused`; analyzed variables
  move to the queue front in focused mode instead of being activity-bumped.
- Has default root-safe stable/focused switching through `SAT_MODE_SWITCH_INTERVAL=50000`, with a
  progress-sensitive `stale-stable` guard and a bounded focused dwell cap.
- Stable decisions can use target/best rephase state; focused decisions still use saved phase.
- Relevant code: `src/main.rs:248`, `src/main.rs:1281`, `src/main.rs:1364`.

Kissat:

- Has two search modes.
- Stable mode uses a score heap.
- Focused mode uses a linked decision queue and picks the last enqueued unassigned variable.
- In focused mode, analyzed variables are moved to the front of the queue instead of score-bumped.
- It can switch modes on conflict/tick limits.
- Relevant code: `decide.c:126`, `bump.c:103`, `mode.c:19`, `mode.c:150`.

Why this matters:

- Solver 11 can now test Kissat-like search-mode transitions, but the measurements show focused
  mode remains path-sensitive. Guarded 50k switching improved the local profiling set, while more
  frequent or unbounded focused phases caused large regressions.

### 3. Restart policy

Solver 11:

- Still supports the older fixed Luby restart budget with `SAT_RESTART_MODE=luby` and
  `restart_unit = 100`.
- Has an opt-in focused-style glue restart scaffold via `SAT_RESTART_MODE=glue-ema`, using fast and
  slow learned-clause glue EMAs.
- Uses stable reluctant doubling restarts by default; in focused mode this policy uses the existing
  glue-EMA restart signal. Use `SAT_RESTART_MODE=luby` for the previous Luby restart ablation.
- Guarded mode switching now combines with reluctant restarts as the default policy. The measured
  configuration uses `SAT_RESTART_MODE=reluctant`, `SAT_MODE_SWITCH_POLICY=stale-stable`, and
  `SAT_MODE_SWITCH_INTERVAL=50000`.
- Restarts use Kissat-style trail reuse by default, keeping the prefix selected by activity in
  stable mode or focused recency stamp in focused mode. The default cap is
  `SAT_RESTART_REUSE_CAP=8`; use `SAT_RESTART_REUSE=off` to disable reuse or
  `SAT_RESTART_REUSE_CAP=0` for uncapped reuse.
- Relevant code: `src/main.rs:121`, `src/main.rs:3189`, `src/main.rs:3242`,
  `src/main.rs:3487`.

Kissat:

- Focused mode restarts compare fast and slow glue EMAs after a restart interval.
- Stable mode uses reluctant doubling restarting.
- Restarts can reuse a prefix of the trail instead of always backtracking to level 0.
- Relevant code: `restart.c:14`, `restart.c:39`, `restart.c:53`, `restart.c:112`.

Why this matters:

- Solver 11 now computes learned-clause glue and uses Kissat-like stable reluctant restart, trail
  reuse, root rephase, and guarded mode-switch signals by default, while focused-specific behavior
  remains bounded because focused mode is still path-sensitive.
- Always backtracking to level 0 can throw away useful trail prefixes, but reuse is also
  path-sensitive: capped stable reuse improves the profile set, while uncapped/coarse reuse can
  regress badly.

### 4. Learned-clause metadata and reduction

Solver 11:

- Learned clauses now store glue/LBD, a small used counter, a tier, and reserved reason/shrunken/
  vivify flags alongside the existing floating clause activity. They still do not store a searched
  literal position.
- Glue/tier/used-based reduction is the default, with the old activity reducer available through
  `SAT_REDUCE_MODE=activity`. Reduction-pressure counters record low-yield passes, still-over-budget
  passes, and optional cooldown skips.
- Relevant code: `src/main.rs:34`, `src/main.rs:780`, `src/main.rs:3868`, `src/main.rs:3943`.

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

- The core glue/tier metadata is now present, which is why glue-tiered reduction and glue-EMA
  restart experiments can be measured. Remaining Kissat features still need searched-position
  metadata, reason-side bumping, trail reuse, and mode-aware scheduling.

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
- Computes glue/LBD for learned-clause policy, has global chronological backtracking
  (`SAT_CHRONO_LEVELS`, default `100`), and defaults to one-hop unlimited reason-side bumping
  (`SAT_REASON_SIDE_BUMP_MODE=one-hop SAT_REASON_SIDE_BUMP_LIMIT=unlimited`). It still lacks
  failed-literal special handling, shrink, and eager subsumption of recent learned clauses.
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

- Stores `saved_phase`, `target_phase`, and `best_phase` vectors.
- Saved phase is updated on every enqueue. Stable decisions prefer target phase when set, then saved
  phase; focused decisions still follow the existing saved-phase behavior.
- Stable-mode backtracking records the deepest assignment snapshot as best phase while rephase is
  enabled. Periodic root rephase is on by default with `SAT_REPHASE_INTERVAL=10000`, and scheduled
  local-search walking is also enabled by default after the 2026-05-13 follow-up confirmation. The
  active source cycle is best, walk, inverted, best, walk, original. Use `SAT_REPHASE_INTERVAL=0`
  to disable scheduled rephase, or `SAT_WALK=off` to keep rephase while disabling the walking
  source.
- Relevant code: `src/main.rs` phase vectors, `run_rephase`, `run_walk_phase`, and
  `decision_phase`.

Kissat:

- Stores saved, target, and best phase vectors.
- Stable-mode backtracking records target/best assignments when the trail reaches new heights.
- Rephase events copy best, walking, inverted, best, walking, original phases in a schedule.
- Decisions prefer target then saved then initial phase, with focused-mode phase overrides.
- Relevant code: `phases.h:6`, `backtrack.c:38`, `decide.c:155`, `rephase.c:32`,
  `rephase.c:86`, `rephase.c:109`.

Why this matters:

- Solver 11 now uses Kissat-style polarity perturbation by default. The first measured interval
  rescues the known `random_v355_s3` restart-reuse regression, but it also makes one UNSAT proof
  much harder to check, so disabling rephase remains an important ablation.
- The walking source is implemented and now default-on with the measured deterministic cap
  (`SAT_WALK_STEPS=100`, `SAT_WALK_RANDOM_PERCENT=0`). Larger or random walks were too
  path-sensitive, and `SAT_WALK_INITIAL=1` remains opt-in.

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

- Runs a basic all-true/all-false lucky SAT shortcut after root propagation and one-shot
  preprocessing.
- Does not yet try ordered failed-literal probing before search.

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
  formula. Done as `SAT_LUCKY=shortcut`, default on, implemented by checking the complete
  all-true/all-false candidate model against live clauses after preprocessing.
- After binary implication watches exist, add bounded lucky probing with an effort cap and counters
  for SAT shortcut, units learned, conflicts, and time.
- Ensure a lucky SAT after preprocessing still runs the existing model-extension path for eliminated
  variables.

Why this matters:

- This is not a general performance feature, but it is a cheap way to solve or simplify easy
  structured instances before committing to a search trajectory.

### 12. Warmup phase seeding

Solver 11:

- Initializes phases from `SAT_BRANCH_MODE` and normally relies on phase saving during real search.
- A root-only warmup pass now runs by default after preprocessing. It makes bounded decisions with
  the current branching heuristic, propagates, saves phases through normal enqueue, then backtracks
  to root without updating target/best phase snapshots. Use `SAT_WARMUP=off` to disable it.
- The pass is bounded by `SAT_WARMUP_DECISIONS`, defaulting to the variable count when warmup is on.

Kissat:

- Optional `kissat_warmup` temporarily runs normal decisions and propagation beyond conflicts,
  saving phases through the normal assignment path.
- It then backtracks to level 0 without updating phases, leaving only the seeded saved phases.
- Relevant code: `walk.c:961`, `warmup.c:9`.

Implementation status for solver 11:

- Implemented with counters for warmups, warmup decisions, conflicts, assignments, and elapsed
  time. It is default-on after the 2026-05-13 follow-up confirmation run.
- Current warmup uses the normal propagator repeatedly and stops after the first decision-level
  conflict; it does not yet fully match Kissat's `propagate_beyond_conflicts` behavior of
  continuing every remaining watch after a conflict on the same literal.
- Measured separately because warmup can either help phase selection or waste time on formulas where
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

- Landed as an opt-in mode behind `SAT_SEARCH_MODE=focused`; default search still starts in stable
  heap mode.
- The stable hot path still uses the existing activity heap, variable activity bumps, and decay.
- Focused mode maintains a linked recent-conflict decision queue. Branching pops the queue front,
  analyzed variables are moved to the front instead of score-bumped, and backtracked variables are
  restored to the focused queue without rebuilding the heap.
- The normal branch heap remains populated in focused mode for invariants and future mode-switching
  work, but focused decisions read from the focused queue.
- Automatic guarded mode switching is covered by the later Phase 7 status notes.

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
- Default uses the full best, walk, inverted, best, walk, original schedule after the 2026-05-13
  follow-up confirmation. The walking source can still be disabled with `SAT_WALK=off`.

Dependencies:

- Search mode and limit scheduler.

### H. Add chronological backtracking and reason-side bumping

Goal:

- Match two smaller but search-path-relevant Kissat behaviors in conflict handling.

Status on 2026-05-12:

- Implemented chronological backtracking as a global `SAT_CHRONO_LEVELS=<levels>` threshold,
  defaulting to Kissat's `100`; `SAT_CHRONO_LEVELS=off` disables it for ablations.
- Implemented reason-side bump modes via `SAT_REASON_SIDE_BUMP_MODE=off|traversal|one-hop`.
  Learned-clause variables remain bumped even when extra reason-side bumping is off.
- Current default after the 2026-05-12 follow-up request:
  `SAT_REASON_SIDE_BUMP_MODE=one-hop SAT_REASON_SIDE_BUMP_LIMIT=unlimited`. The previous
  no-extra-reason-side default is available with `SAT_REASON_SIDE_BUMP_MODE=off`, and the old
  full-UIP-traversal behavior remains available with
  `SAT_REASON_SIDE_BUMP_MODE=traversal SAT_REASON_SIDE_BUMP_LIMIT=unlimited`.
- `SAT_CHRONO_LEVELS=100` was neutral on top of the accepted reason-side cap, while aggressive
  thresholds regressed the profile set. It is now the built-in global default after the follow-up
  request to match Kissat's default threshold.

Implementation items:

- Add a `chronolevels` threshold. If the jump would skip more than that many levels, backtrack only
  one level chronologically. Done as `SAT_CHRONO_LEVELS`.
- Add optional reason-side bumping with a measured limit, guarded by decision-rate style counters.
  Done as `SAT_REASON_SIDE_BUMP_MODE`, `SAT_REASON_SIDE_BUMP_LIMIT`, and `reason_side` trace
  counters. The one-hop mode inspects only immediate reasons of final learned-clause literals.
- Add tests for chronological backtracking threshold behavior. Done.

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

Implementation status:

- Root-only warmup is implemented and default-on; use `SAT_WARMUP=off` for ablations.
- Decide and propagate using the current heuristic, save phases, then backtrack without overwriting
  them.
- Track warmup decisions, conflicts, assigned trail size, and elapsed time. Propagations are still
  counted in the solver-wide propagation counter.

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

Status on 2026-05-12:

- Stable-only implementation landed behind `SAT_REORDER=stable-weight`; default remains off.
- The pass scans live original clauses after preprocessing, computes polarity-aware literal weights
  with binary clauses at `1` and each larger clause halved per extra literal, then adds
  `max(pos, neg) + 2*min(pos, neg)` into stable heap activity. It rebuilds the stable branch heap
  and is wired into the existing root `Reorder` maintenance hook. This legacy pre-search mode still
  skips focused-mode queue ordering.
- Delayed mode-aware implementation landed as `SAT_REORDER=kissat`; after the 2026-05-12 follow-up
  request this is now the default. It skips pre-search reorder, starts at `SAT_REORDER_INIT`
  conflicts, repeats with linearly growing `SAT_REORDER_INTERVAL` windows, skips satisfied clauses,
  caps effective clause size with `SAT_REORDER_MAX_CLAUSE_SIZE`, rescales stable heap scores before
  adding raw weights, and reorders the focused queue by weight with existing recency as the
  tie-breaker.
- Validation: `cargo test` passed 104 tests, default smoke passed 9/9
  (`log/2026-05-12-20-38-11`), default invariant smoke passed 9/9
  (`log/2026-05-12-20-38-22`), and opt-in reorder invariant smoke passed 9/9
  (`log/2026-05-12-20-51-26`).
- Default-off profiling after the implementation solved 8/11 with PAR-2 `794.914`
  (`log/bench-11-kissat-innovations-2026-05-12-20-41-47`), consistent with the existing default
  profile envelope.
- Full opt-in profiling rejected this as a default policy: `SAT_REORDER=stable-weight` solved the
  same 8/11 but worsened PAR-2 to `940.312`
  (`log/bench-11-kissat-innovations-2026-05-12-21-07-19`). Major regressions were
  `feistel_b64_k32_r22` `7.137s -> 20.349s`, `feistel_b64_k52_r17` `5.210s -> 51.128s`,
  `feistel_b64_k57_r18` `2.212s -> 42.423s`, timetable `28.566s -> 48.914s`, and `mp1`
  `3.529s -> 27.559s`; only `random_v285_s2` and `random_v355_s3` improved slightly. Direct `k32`
  traces showed the pass itself was cheap (`0.088ms` for `8111` clauses / `29701` literals /
  `1200` boosted variables), but the search path worsened from `187452` conflicts and `32638486`
  propagations to `506505` conflicts and `94834877` propagations.
- Focused mode should not be affected by this implementation. The hook intentionally skips when
  `search_mode != Stable`; a focused opt-in trace showed `reorder=1/0/1` and scanned no clauses.
  A useful focused reorder would need a separate focused-queue/stamp policy and its own validation.
- Delayed `SAT_REORDER=kissat` validation passed 109 unit tests, default smoke
  (`log/2026-05-12-22-26-19`), default invariant smoke (`log/2026-05-12-22-26-23`), and forced
  delayed reorder invariant smoke (`log/2026-05-12-22-26-31`). Full profiling solved 8/11 but
  regressed PAR-2 to `917.395` (`log/bench-11-kissat-innovations-2026-05-12-22-11-59`). This was
  better than pre-search `stable-weight` at `940.312`, but still much worse than default-off
  `794.914`. A traced `k32` run fired 11 stable reorder passes, each around `0.16ms`, and still
  grew to `616120` conflicts / `107241644` propagations. Less frequent `k32` schedules
  (`50000/50000`, `100000/100000`) and focused-start reorder (`SAT_SEARCH_MODE=focused`) remained
  bad at `56.30s`, `26.91s`, and `103.17s` respectively.
- Conclusion: clause-weight reorder is implemented as a useful opt-in diagnostic foundation, but is
  rejected as a default and should not be the next optimization target until more structural
  inprocessing, such as binary transitive reduction or scheduled probing, changes the search
  substrate.
- Follow-up with `SAT_FULL_BSR=off` changed that conclusion for the profiling set. No-reorder with
  full BSR disabled solved only 7/11 with PAR-2 `1072.183`
  (`log/bench-11-kissat-innovations-2026-05-12-22-33-58`). The pre-search
  `SAT_REORDER=stable-weight` mode solved 9/11 with PAR-2 `614.511`
  (`log/bench-11-kissat-innovations-2026-05-12-22-44-31`), recovering `k52`
  (`TIMEOUT -> 2.205s`) and timetable (`TIMEOUT -> 10.333s`) at the cost of a large `mp1`
  regression (`1.925s -> 28.296s`). Delayed `SAT_REORDER=kissat` solved 9/11 with PAR-2
  `797.994` (`log/bench-11-kissat-innovations-2026-05-12-22-51-24`). A traced
  no-full-BSR `stable-weight` `k52` run showed reorder cost only `0.114ms`, then solved in
  `2.180s` with `108475` conflicts. This means reorder is harmful after full BSR but can substitute
  for some missing structural guidance when full BSR is off. Do not default this from the small
  profiling set alone; validate on a larger slice first. Per the 2026-05-12 follow-up request,
  full BSR is now default-off and delayed `SAT_REORDER=kissat` is now the default; pre-search
  `stable-weight` remains an explicit experiment.

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
  search trajectory and excessive reduce-DB pressure. The default policy starts in stable mode and
  only enters focused mode through a guarded, bounded root-safe switch.
- Root-safe mode switching is now enabled by default through `SAT_MODE_SWITCH_INTERVAL=50000` /
  `SAT_MAINT_MODE_SWITCH_INTERVAL`, with `SAT_MODE_SWITCH_INTERVAL=0` available to disable it for
  ablations.
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
- Add focused glue-EMA restart behind `SAT_RESTART_MODE` and Kissat-style trail reuse behind
  `SAT_RESTART_REUSE`.
- Add stable reluctant restart after trail reuse behavior is understood.
- Add target and best phase arrays, then rephase scheduling through the Phase 3 scheduler.
- Add warmup as a phase-seeding pass once phase arrays exist.
- Add chronological backtracking and reason-side bumping after restart and mode behavior are
  measurable. Done as guarded controls; the current default is one-hop unlimited reason-side
  bumping after follow-up validation.

Why after search modes:

- Kissat's restart and rephase logic is mode-sensitive. Implementing it before focused/stable
  abstractions would hard-code the current MiniSat-like path and make mode switching harder.

Unlocks:

- More faithful Kissat search behavior before adding expensive inprocessing.

Status:

- The first Phase 7 slice landed on 2026-05-10 behind explicit environment flags. Solver 11 now has
  `SAT_RESTART_MODE=glue-ema` with fast/slow learned-glue EMAs and trace counters for
  `glue_fast`, `glue_slow`, and `glue_restarts`.
- The same slice added reduction-pressure observability and an opt-in low-yield reduction throttle
  via `SAT_REDUCE_LOW_YIELD_COOLDOWN=<conflicts>`. This records low-yield reduce passes,
  still-over-budget passes, cooldown skips, and the last reduce pass's live/candidate/delete counts.
- Validation: `cargo test` passed 76 tests, default smoke passed 9/9, focused glue-EMA invariant
  smoke passed 9/9, default profiling solved 6/11 with PAR-2 `1328.185`, and focused glue-EMA plus
  cooldown solved 2/11 with PAR-2 `2185.711`.
- Direct ablation on `feistel_b64_k57_r18`: focused Luby plus cooldown solved in `28.092s`,
  focused glue-EMA without cooldown solved in `13.519s`, and focused glue-EMA plus cooldown solved
  in `9.355s`. Stable default still solved faster at `4.034s`.
- Interpretation: glue-EMA restarts and reducer throttling help focused mode substantially on some
  trajectories, but they are not sufficient to make focused mode a default.
- Restart trail reuse landed on 2026-05-11 and is now the default. The implemented mode mirrors
  Kissat's decision-prefix rule: keep decision levels while each kept decision variable has a better
  activity score (stable mode) or recency stamp (focused mode) than the next decision candidate.
- The coarse reuse policies were rejected and removed: stable `quarter` took `57.124s` and stable
  `half` took `53.417s` on `feistel_b64_k57_r18`, versus `4.452s` with reuse off and `1.599s` with
  uncapped Kissat reuse.
- Full profiling: reuse off after the code change solved 6/11 with PAR-2 `1332.547`; uncapped
  stable Kissat reuse solved 6/11 with PAR-2 `1301.718`; capped stable reuse with
  `SAT_RESTART_REUSE_CAP=8` solved 6/11 with PAR-2 `1246.545`.
- The cap-8 profile improved `feistel_b64_k32_r22` (`27.60s` -> `0.59s`),
  `feistel_b64_k52_r17` (`18.42s` -> `4.28s`), `feistel_b64_k57_r18` (`4.85s` -> `2.43s`), and
  solved the timetable instance (`TIMEOUT` -> `15.58s`), but regressed `random_v355_s3`
  (`53.35s` -> `TIMEOUT`). This tradeoff is accepted for the default because the profile-set PAR-2
  improved materially; mode switching/rephase or a reuse guard should address the path sensitivity.
- Follow-up cleanup removed the rejected coarse modes and made capped Kissat reuse the default.
  Fresh validation: `cargo test` passed 79 tests, default smoke passed 9/9, reuse-off invariant
  smoke passed 9/9, default invariant smoke passed 9/9, and the default profiling run solved 6/11
  with PAR-2 `1247.040` (`log/bench-11-kissat-innovations-2026-05-11-07-33-52`).
- Target/best phase tracking and rephase scheduling landed on 2026-05-11. Rephase is stable-mode
  only and defaults to `SAT_REPHASE_INTERVAL=10000`; `SAT_MAINT_REPHASE_INTERVAL=<conflicts>` is
  accepted as an alias and `SAT_REPHASE_INTERVAL=0` disables it. Decisions prefer active target
  phase over saved phase; best phase is a deepest-trail snapshot used as one rephase source.
- Validation: `cargo test` passed 84 tests, default smoke with rephase interval 10000 passed 9/9,
  smoke with rephase interval 50000 passed 9/9 before the later default-policy change, and
  `SAT_REPHASE_INTERVAL=0 SAT_CHECK_INVARIANTS=1` smoke passed 9/9.
- Default no-rephase profiling after the implementation solved 6/11 with PAR-2 `1246.720`, matching
  the previous default cap-8 profile and showing no obvious default hot-path regression.
- `SAT_REPHASE_INTERVAL=10000` is now the aggressive default. In the targeted interval sweep it
  solved the rescued random instance but substantially regressed two Feistel paths, so this should
  be treated as an intentional policy step rather than a fully tuned value.
- `SAT_REPHASE_INTERVAL=50000` solved 7/11 by runtime with PAR-2 `1018.540`, rescuing
  `random_v355_s3` (`TIMEOUT` -> `1.17s`) while keeping the Feistel wins close to baseline.
  However, it regressed `random_v292_s4` (`14.85s` -> `26.85s`) and its DRAT proof check did not
  finish after 14 minutes, so the full-profile log has a verification failure for that row caused by
  manually stopping the checker. Proof and search side effects still need a guard or a better
  walking/source schedule.
- Stable reluctant restarting and root-safe mode-switch scaffolding landed on 2026-05-12. The new
  restart mode is selected with `SAT_RESTART_MODE=reluctant`; it uses Kissat-style reluctant
  doubling in stable mode with `SAT_RELUCTANT_INTERVAL=1024` and `SAT_RELUCTANT_LIMIT=1048576`
  defaults, and falls back to the existing glue-EMA trigger in focused mode. Mode switching is
  selected with `SAT_MODE_SWITCH_INTERVAL=<conflicts>` or `SAT_MAINT_MODE_SWITCH_INTERVAL`; it runs
  through the root maintenance scheduler, backtracks to root, rebuilds the heap/queue for the new
  mode, and grows later switch intervals as `N * count * log10(count + 9)^4`.
- Validation: `cargo test` passed 88 tests; default smoke passed 9/9 with proof checking; reluctant
  restart plus `SAT_MODE_SWITCH_INTERVAL=1 SAT_CHECK_INVARIANTS=1` smoke passed 9/9; focused-start
  reluctant/mode-switch invariant smoke also passed 9/9.
- Fresh 120s profiling baseline after the implementation solved 8/11 with PAR-2 `898.091`
  (`log/bench-11-kissat-innovations-2026-05-12-07-24-48`). `SAT_RESTART_MODE=reluctant` solved the
  same 8/11 with PAR-2 `872.650`
  (`log/bench-11-kissat-innovations-2026-05-12-07-44-59`), a small `2.8%` improvement that is below
  the usual default-policy keep threshold. Main win: `feistel_b64_k52_r17` `82.612s -> 24.079s`.
  Main regressions: `feistel_b64_k32_r22` `44.988s -> 69.005s`, Timetable `10.780s -> 20.833s`,
  and `mp1` `5.019s -> 8.076s`.
- Initial raw mode-switch experiments were rejected as default policies. `SAT_RESTART_MODE=reluctant
  SAT_MODE_SWITCH_INTERVAL=1000` solved only 4/11 with PAR-2 `1736.112`
  (`log/bench-11-kissat-innovations-2026-05-12-07-59-37`). `SAT_MODE_SWITCH_INTERVAL=50000` solved
  8/11 by runtime with PAR-2 `998.225`
  (`log/bench-11-kissat-innovations-2026-05-12-08-14-53`) but regressed Timetable to `109.144s`,
  slowed both random UNSAT rows, and required manually stopping `drat-trim` after about 14 minutes
  on `random_v292_s4`.
- Follow-up guarded mode switching added `SAT_MODE_SWITCH_POLICY=stale-stable`, focused dwell caps,
  and focused-only low-yield reducer cooldown. The measured default policy now uses reluctant
  restarts with stale-stable mode switching at interval `50000`; it solved 8/11 with PAR-2
  `849.514` on the same profiling set
  (`log/bench-11-kissat-innovations-2026-05-12-11-10-40`), improving the same-turn default anchor
  by about `6.1%`. Use `SAT_RESTART_MODE=luby SAT_MODE_SWITCH_INTERVAL=0` for the previous
  default-search ablation.
- Chronological backtracking and reason-side bump controls landed on 2026-05-12. Chronological
  backtracking now defaults globally to `SAT_CHRONO_LEVELS=100`; aggressive thresholds regressed,
  and `SAT_CHRONO_LEVELS=off` remains available for ablations. The first accepted reason-side
  policy was `SAT_REASON_SIDE_BUMP_LIMIT=0`, which preserved learned-clause variable bumping while
  suppressing extra reason-side bump flooding. The final no-proof profiling confirmation before
  the chrono default flip solved 8/11 with PAR-2 `817.875`
  (`log/bench-11-kissat-innovations-2026-05-12-13-38-18`), versus the no-proof legacy/unlimited
  baseline PAR-2 `848.544`.
- Follow-up validation after enabling global `SAT_CHRONO_LEVELS=100`: `cargo test` passed 95 tests,
  default smoke passed 9/9 (`log/2026-05-12-14-30-26`), invariant smoke passed 9/9
  (`log/2026-05-12-14-30-39`), and the no-proof profiling confirmation solved 8/11 with PAR-2
  `816.106` (`log/bench-11-kissat-innovations-2026-05-12-14-30-48`).
- Follow-up one-hop reason-side bump mode landed on 2026-05-12. It is enabled with
  `SAT_REASON_SIDE_BUMP_MODE=one-hop` and uses `SAT_REASON_SIDE_BUMP_LIMIT` as an absolute
  per-conflict side-variable cap. It now defaults to unlimited one-hop bumping. Validation:
  `cargo test` passed 96 tests, default smoke passed 9/9 (`log/2026-05-12-14-57-14`), one-hop
  cap `10` invariant smoke passed 9/9 (`log/2026-05-12-14-57-28`), and legacy traversal invariant
  smoke passed 9/9 (`log/2026-05-12-14-57-36`).
- Bounded one-hop caps were rejected as defaults. Cap `10` regressed the first three Feistel
  profile rows (`42.431s`, `14.987s`, `2.242s`) versus the prior default (`27.78s`, `11.67s`,
  `0.44s`); cap `1` improved `feistel_b64_k52_r17` (`0.964s`) but regressed
  `feistel_b64_k32_r22` (`51.903s`) and `feistel_b64_k57_r18` (`29.504s`). Keep bounded caps as
  targeted experiment knobs.
- One-hop unlimited was run to completion after a follow-up request. It solved 8/11 with PAR-2
  `794.645` (`log/bench-11-kissat-innovations-2026-05-12-15-41-28`) versus the post-change default
  anchor PAR-2 `816.217`. This is a real but sub-threshold `2.6%` profile-set improvement; it is
  now the default per follow-up request. Main wins: `feistel_b64_k32_r22`, `feistel_b64_k52_r17`,
  `mp1-Nb7T46`, `random_v285_s2`, and `random_v292_s4`; main regressions: `feistel_b64_k57_r18`,
  Timetable, and `random_v355_s3`.
- Final no-env validation after making one-hop unlimited the built-in default: `cargo test` passed
  96 tests, default smoke passed 9/9 (`log/2026-05-12-16-02-25`), invariant smoke passed 9/9
  (`log/2026-05-12-16-02-34`), and the no-proof profiling confirmation solved 8/11 with PAR-2
  `794.641` (`log/bench-11-kissat-innovations-2026-05-12-16-02-42`).
- Warmup and walking phase sources landed on 2026-05-13. `SAT_WARMUP` now defaults on and performs
  bounded pre-search phase seeding before backtracking through a no-target/best-snapshot path.
  `SAT_WALK` now defaults on and enables a bounded WalkSAT-style source for stable rephases.
  `SAT_WALK_INITIAL=1` remains opt-in and runs the same source once before CDCL search. The
  measured walking defaults are `SAT_WALK_STEPS=100` and `SAT_WALK_RANDOM_PERCENT=0`.
- Validation after the conflict-stop fix and walk tuning: `cargo test` passed 113 tests; default
  smoke passed 9/9 (`log/2026-05-13-00-55-02`); smoke with
  `SAT_WARMUP=1 SAT_WARMUP_DECISIONS=32 SAT_WALK_INITIAL=1 SAT_WALK=1
  SAT_CHECK_INVARIANTS=1` passed 9/9 (`log/2026-05-13-00-55-10`).
- No-new-flag no-proof profiling before the default flip solved 9/11 with PAR-2 `793.182`
  (`log/bench-11-kissat-innovations-2026-05-12-23-36-05`). Warmup alone was neutral/slightly
  negative at PAR-2 `795.985`. Final `SAT_WALK=1` solved 9/11 with PAR-2 `667.472`
  (`log/bench-11-kissat-innovations-2026-05-13-00-32-50`), a `15.9%` improvement from search-path
  changes. Combined `SAT_WARMUP=1 SAT_WALK=1` was only `1.516s` better than walk-only and below the
  3% keep threshold. A follow-up rerun of the combined policy solved 9/11 with PAR-2 `667.671`
  (`log/bench-11-kissat-innovations-2026-05-13-10-52-55`), confirming the number before making the
  policy default. The no-env default-policy confirmation after the flip solved 9/11 with PAR-2
  `665.670` (`log/bench-11-kissat-innovations-2026-05-13-11-14-32`).
- Default-policy validation after the flip passed 113 unit tests and the 9/9 smoke suite with UNSAT
  proofs verified (`log/2026-05-13-11-14-17`).
- Important tuning rejects: `SAT_WALK_INITIAL=1` lost the timetable solve, added a third timeout,
  and ended at PAR-2 `962.377`; the pre-tuning variable-scaled rephase walk regressed to PAR-2
  `846.795`; keeping the old 1% random rate with the 100-step cap was worse than deterministic
  walking (`727.151` versus `662.263` in the tuning run). Warmup remains separately ablatable, but
  the confirmed warmup-plus-walk interaction is now the default.

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

Status on 2026-05-12:

- Basic lucky SAT shortcut landed as `SAT_LUCKY=shortcut`, default on. `SAT_LUCKY=off` disables it
  for ablations.
- The shortcut runs after root propagation and one-shot preprocessing, before CDCL search. It checks
  all-true and then all-false candidate models against all live original and learned clauses, then
  routes SAT through the existing model-extension snapshot path.
- Validation: `cargo test` passed 100 tests, default smoke passed 9/9
  (`log/2026-05-12-16-36-04`), invariant smoke passed 9/9 (`log/2026-05-12-16-36-13`), and a
  traced all-positive smoke instance reported `lucky=1/1/1/0` and returned model `1 2 3` before
  CDCL decisions.
- Profiling: default shortcut solved 8/11 with PAR-2 `794.191`
  (`log/bench-11-kissat-innovations-2026-05-12-16-36-29`); `SAT_LUCKY=off` solved 8/11 with PAR-2
  `793.828` (`log/bench-11-kissat-innovations-2026-05-12-16-44-02`). The `0.363s` delta is noise,
  so this is kept as a low-risk foundation for later probing rather than a profile-set speed win.
- Clause-weight reordering landed as a foundation. Full-BSR-on profiling rejected both the
  pre-search stable mode (`SAT_REORDER=stable-weight`) and delayed mode-aware mode
  (`SAT_REORDER=kissat`) as defaults, but the no-full-BSR follow-up changed the requested default:
  full BSR is now off by default and `SAT_REORDER=kissat` is now default-on. The stronger
  `SAT_FULL_BSR=off SAT_REORDER=stable-weight` combination remains an explicit larger-benchmark
  validation candidate.

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
| F glue restarts/trail reuse | 7 | Glue-EMA, capped Kissat-style trail reuse, dynamic reuse-progress guard, stable reluctant restarts, and guarded mode switching landed; reluctant/mode switching are now default. |
| G target/best/rephase | 7 | Target/best rephase is default at interval 10000; walking source is default-on with tuned defaults. |
| H chronological backtracking/reason bump | 7 | Best measured after restart changes. |
| I eager subsumption | 5 | Learned-clause lifecycle feature. |
| J inprocessing scheduler | 3, 8, 9 | Scheduler first, actual passes later. |
| K BVE scoring | 8 | Needs search relevance from Phase 6. |
| L representation follow-ups | 1, 5 | Accessors early; sparse layout only if needed. |
| M lucky | 8 | Simple shortcut can be earlier, full probing needs Phase 2. |
| N warmup | 7 | Implemented and default-on after the warmup+walk follow-up confirmation. |
| O reorder | 3, 6, 8 | Hook in scheduler, then stable/focused implementations. Root-safe mode-switch hook landed separately. |
| P transitive reduction | 9 | Needs binary graph and scheduler. |
| Q on-the-fly strengthening | 1, 5 | Diagnostics early, mutation later. |
| R forward subsumption | 8 | Needs dense occurrence lifecycle. |
| S fast BVE | 8 | Needs dense mode and binary special cases. |
| T vivification | 9 | Needs glue tiers, scheduler, dense mode. |
| U gates/congruence/equivalence | 9, 10 | Binary equivalence first; gates/congruence later. |
| V dense/sparse mode | 4 | Foundational for repeated simplification. |
| W GC instrumentation | 0, 5 | Measure early; optimize after reduction pressure is real. |
| X factorization/BVA | 10 | Late-stage structural feature. |

## Remaining Non-Inprocessing Search Gaps

Before adding more formula-modifying simplification, the remaining Kissat gaps are mostly search
policy and trajectory controls:

1. Dynamic restart-reuse guard: capped trail reuse helps the profiling set, but it is still
   path-sensitive. Add a guard that temporarily disables reuse when a reused-restart window is short
   and learned-clause glue gets worse, then validate against the default and reuse-disabled
   ablations.
2. More Kissat-exact focused decisions: focused mode has the queue and guarded dwell cap, but
   focused-only remains weak. Missing pieces include focused-specific phase overrides and better
   coupling between queue order, reduction pressure, and restart timing.
3. More faithful warmup: solver 11 stops warmup at the first decision-level conflict, while Kissat
   can propagate beyond conflicts to seed more phases.
4. Rephase/restart guards: scheduled rephase and walking are default-on, but they still need guards
   for proof-heavy UNSAT paths and cases where rephase increases conflicts/restarts.
5. Search-path conflict-analysis details: failed-literal handling at decision level 1 and
   special handling for conflict clauses with one current-level literal remain missing. Clause
   shrinking and eager learned subsumption are also related, but cross into formula modification and
   should wait until this search-control pass is measured.

Status on 2026-05-13:

- Item 1 landed as `SAT_RESTART_REUSE_GUARD=progress`, default-on. The guard observes each restart
  window's learned-clause average glue. If the previous restart reused a nonzero trail prefix, the
  next restart arrives within `SAT_RESTART_REUSE_GUARD_MIN_CONFLICTS` conflicts, and the window's
  average glue worsened by `SAT_RESTART_REUSE_GUARD_GLUE_MARGIN`, reuse is disabled until
  `SAT_RESTART_REUSE_GUARD_COOLDOWN` more conflicts have passed. The defaults are `128`
  conflicts, `1024` cooldown conflicts, and a `1.05` glue margin.
- Validation: `cargo test` passed 116 tests, and the default smoke suite passed 9/9 with UNSAT
  proof checking (`log/2026-05-13-18-42-51`).
- No-proof profiling on `benchmarks/profiling`, 120s timeout and 16 GB memory: default guard-on
  solved 9/11 with PAR-2 `629.226`
  (`log/bench-11-kissat-innovations-2026-05-13-18-15-12`); the same code with
  `SAT_RESTART_REUSE_GUARD=off` solved 9/11 with PAR-2 `669.566`
  (`log/bench-11-kissat-innovations-2026-05-13-18-22-21`); and `SAT_RESTART_REUSE=off` solved 9/11
  with PAR-2 `722.942` (`log/bench-11-kissat-innovations-2026-05-13-18-33-10`). The guard improved
  the local profile by `40.340s` versus guard-off reuse, mostly by moving `feistel_b64_k52_r17`
  from `80.721s` to `40.977s`; reuse plus the guard improved by `93.716s` versus no reuse.
- A traced default `feistel_b64_k52_r17` run reported `restart_reuse_guard=436/2/1`, so the guard
  did fire on the instance that carries the win: 436 guard checks, 2 skipped reuse attempts, and 1
  cooldown window.

## First Concrete Milestones

1. Land Phase 0 and Phase 1 together if the patch remains manageable: diagnostics, metadata
   accessors, tagged reasons, glue computation, and no policy changes.
2. Land Phase 2 as a standalone propagation refactor: binary implication watches plus proof/model
   iteration support. This should get the heaviest unit-test coverage.
3. Land Phase 3 and Phase 4 as infrastructure with no major new algorithms: scheduler hooks,
   root-maintenance transition, dense/sparse mode, reusable occurrence lists.
4. Land Phase 5 and Phase 6 incrementally: glue-tiered reduction as the default with activity
   fallback, and focused queue behind a flag before automatic mode switching.
5. Finish the remaining Phase 7 policy pieces before expanding focused behavior: guards for harmful
   restart reuse or proof-heavy rephase paths. Stable reluctant restarting, root-safe mode-switch
   hooks, the progress-sensitive mode-switch guard, warmup, and walking phase sources are now
   implemented; warmup plus deterministic scheduled walking is now the default policy.
6. Only then start Phase 8 visible algorithmic work: lucky shortcut/probing, clause-weight reorder,
   Kissat-style BVE scoring, fast BVE, and bounded forward subsumption.
