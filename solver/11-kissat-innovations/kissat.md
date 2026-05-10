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

- This can regress before branch modes and restart policy are updated, so gate it with an env flag
  such as `SAT_REDUCE_MODE=activity|glue-tiered`.

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
- Validation: `cargo test` passed 56 tests; normal smoke passed 9/9; invariant smoke with
  `SAT_CHECK_INVARIANTS=1` passed 9/9.
- Profiling overhead check on `benchmarks/profiling`, timeout 120s, memory 16 GB:
  pre-change log `log/bench-11-kissat-innovations-2026-05-09-22-21-25`, PAR-2 `1101.501`,
  solved 7/11; representation log `log/bench-11-kissat-innovations-2026-05-09-22-56-02`, PAR-2
  `1101.544`, solved 7/11. Same solved/timeout split; PAR-2 delta was `+0.043s`.

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

### Phase 5. Learned-clause lifecycle policy

Primary roadmap items:

- C. Glue-tiered reduction
- I. Eager subsumption of recent learned clauses
- Q. On-the-fly strengthening implementation, after diagnostics

Build first:

- Replace the activity-only reducer with a glue/used/tier reducer behind `SAT_REDUCE_MODE`.
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
4. Land Phase 5 and Phase 6 behind flags: glue-tiered reduction and focused queue, still without
   automatic mode switching.
5. Only then start Phase 8 visible algorithmic work: lucky shortcut/probing, clause-weight reorder,
   Kissat-style BVE scoring, fast BVE, and bounded forward subsumption.
