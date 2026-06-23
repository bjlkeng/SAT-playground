# Superior Plan: Evolve `solver/10-bve-subsume` Toward Kissat-Class Performance

This plan synthesizes the strongest parts of the uploaded plans into a single implementation specification. It keeps the conservative, correctness-first scaffolding from `kissat_style_sat_solver_plan.md` and combines it with the more concrete performance roadmap, benchmark discipline, milestones, and risk register from `PLAN.md`.

The intended output is a beads/DAG task graph for coding agents. Each task should be implemented as a small, measurable change. The project should not rewrite solver 10 from scratch.

## Core stance

Keep the existing solver architecture:

- Rust word arena.
- Watched literals.
- Current proof stream and DRAT workflow.
- Existing BVE, BSR, occurrence-list machinery.
- Existing model-extension support.
- Existing binary entry contract: `run.sh cnf out_dir`.

Add Kissat-style search and inprocessing features incrementally, behind narrow flags, with correctness and benchmark gates after every change.

## Chosen naming convention

Use one directory name everywhere:

```text
solver/11-kissat-search
```

Several drafts used `solver/11-kissat-style`; do not mix both names. All scripts, smoke tests, logs, and benchmark commands in this synthesized plan use `solver/11-kissat-search`.

---

# 1. Goals, non-goals, and global acceptance criteria

## 1.1 Goals

1. Reach Kissat-class wall-clock performance on `benchmarks/profiling/`, the fixed discriminating set, and eventually the full SC-2025-style 100-instance set.
2. Preserve SAT assignment output against the original input CNF, not merely the simplified residual formula.
3. Preserve DRAT-correct UNSAT proof generation end-to-end.
4. Keep changes incremental enough that a bad search trajectory, proof bug, watcher bug, or model-reconstruction bug can be isolated quickly.
5. Produce enough instrumentation that performance decisions are based on counters, not vibes.
6. Ship user-meaningful profiles with clear behavior:
   - `baseline`: solver-10-equivalent compatibility (search=Safe, preprocess=Off).
   - `default`: safe default candidate composed of Validated search + Conservative preprocess; subsumes the older `search-conservative` / `inprocess-conservative` intents.
   - `fast`: target post-Phase-2 profile composed of Strong search + GateAware preprocess; subsumes the older `search-strong` / `inprocess-gate-aware` intents.
   - `experimental`: unvalidated combinations, never used for acceptance.
7. Produce benchmark logs and config replay files that allow independent reruns of promoted defaults.
8. Maintain separate proof-off and proof-on scorecards so proof-throughput regressions do not hide behind proof-off wins.

## 1.1a User-facing deliverables

At the end of each phase, produce:

```text
solver/11-kissat-search/README.md:
  - supported profiles
  - exact run.sh contract
  - proof behavior
  - SAT model guarantee
  - known disabled experimental features
  - example commands for each profile
  - expected files in out_dir for SAT, UNSAT, UNKNOWN, and parse failure
  - exit-code/status-file behavior
  - proof-on/proof-off examples
  - config replay example

log/<phase>/summary.md:
  - promoted profile
  - config replay file
  - solved count and PAR-2 vs baseline
  - proof-off scorecard
  - proof-on scorecard where applicable
  - lost solved instances, if any
  - proof/model validation status
```

## 1.2 Non-goals for this plan

- Parallel or portfolio solving.
- IPASIR/library packaging.
- Full incremental SAT under assumptions. Keep design-compatible where cheap, but do not implement it now.
- Kissat allocator tricks such as NUMA-aware allocation or huge pages.
- A full embedded secondary solver equivalent to Kissat's kitten. The optional sweep appendix uses a bounded internal DPLL-style helper only if milestone triage proves it pays for itself.

## 1.3 Global correctness gate

Every accepted `solver-behavior`, `config-contract`, or `performance-claim` task must satisfy this gate. `docs-only` and `tooling-only` tasks use the task-class gates in section 0.7, unless they change solver behavior, result parsing, or validation semantics.

```bash
cd solver/11-kissat-search
cargo test

cd ../..
bash tools/smoke_test.sh solver/11-kissat-search
SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-search
```

Acceptance:

```text
- no SAT/UNSAT mismatch
- SAT assignment satisfies the original input CNF
- UNSAT smoke proofs verify through drat-trim
- no panic under invariant-check mode
- no eliminated variable re-enters the decision heap or occurrence scheduler
- no locked learned clause is deleted
```

## 1.4 Global performance gate

After task 0.5 has created generated iteration sets, every `solver-behavior` or `performance-claim` task must run at least the iteration smoke set:

```bash
bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/smoke-plus solver/11-kissat-search
```

Before task 0.5 exists, solver-behavior/config-contract tasks 0.0 through 0.4 use this reduced gate instead:

```bash
cd solver/11-kissat-search
cargo test

cd ../..
bash tools/smoke_test.sh solver/11-kissat-search
SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-search
bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-search
```

Task 0.5 is the boundary after which `benchmarks/iteration/*` is mandatory.

Search changes must also run:

```bash
bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/search-core solver/11-kissat-search
```

Simplification/inprocessing changes must also run:

```bash
bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/preprocess-core solver/11-kissat-search
```

Milestones must run:

```bash
bash tools/bench.sh -t 300 -m 16384 \
  -d benchmarks/discriminating solver/11-kissat-search

bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-search

bash tools/bench.sh -t 300 -m 16384 \
  -d benchmarks/iteration/holdout solver/11-kissat-search

bash tools/bench.sh -t 600 -m 16384 \
  -d benchmarks/profiling/minisat-simp-five solver/11-kissat-search
```

End-of-phase runs must use the full set:

```bash
bash tools/bench.sh -t 1800 -m 16384 \
  -d benchmarks/sat-comp-2025 solver/11-kissat-search
```

## 1.5 Noise discipline

SAT search is noisy. Single wall-clock measurements are not reliable enough.

Performance acceptance rules:

```text
Promotion gate (statistical):
  - PAR-2 delta is computed paired per instance.
  - compare_bench.py reports the bootstrap 90% CI on the paired PAR-2 delta using 10_000 resamples.
  - Promotion requires: CI upper bound < 0 (improvement is significant) OR a named counter improvement that a follow-up task is expected to consume.
  - Rejection requires: CI lower bound > 0 (regression is significant) AND no offsetting counter improvement.
  - Indeterminate (CI straddles 0): acceptable only as a prerequisite task; not promotable to a default.

Per-instance robustness:
  - For search trajectory changes, run paired median of 3 on search-core (single seed; vary only solver binary).
  - For very small deltas, additionally run multi-seed A/B (SAT_SEED ∈ {0, 1, 2}) to estimate seed-vs-binary variance.
  - Per-instance minimum is allowed only for exploratory triage, never for acceptance.
- Per-instance ±10% noise is normal.
- Per-instance −50% wins are obvious and should be investigated for reusable structure.
- Never accept silent correctness regression for speed.
- Never promote a default if it loses a previously solved regression-guard instance unless milestone triage explicitly accepts that tradeoff.
- Report category PAR-2 for search-core, preprocess-core, regression-guards, proof-heavy UNSAT, and SAT-heavy trajectory instances.
- Report per-category bootstrap CIs; categories with sample size < 10 are reported with a "low-sample" annotation and cannot be the *sole* promotion evidence.
- A feature may be profile-specific when it wins one category and loses another; do not force it into baseline.
- Any newly solved hard instance must be checked for reusable structure before using it as promotion evidence.
- If a prerequisite change regresses alone but a dependent change is expected to pay for it, benchmark the pair before rejecting.
- If still regressing, revert and write a short rejection note into the beads task so the same experiment is not repeated blindly.
- The old `max(3%, two-run noise band)` heuristic is deprecated; bootstrap CIs supersede it. Cite the heuristic only when explaining historic decisions.
```

Benchmark environment sanity:

```text
- tools/bench.sh records whether CPU frequency governor, thermal throttling status, and core pinning are known.
- tools/bench.sh supports optional `--pin-core <n>` and records the pinning decision.
- tools/bench.sh runs a tiny calibration instance before benchmark timing and records calibration time.
- compare_bench.py warns if calibration time differs by more than 10% between before/after runs.
- compare_bench.py warns if the machine/environment block differs between paired runs.
```

Flaky-instance policy:

```text
- Add benchmarks/iteration/FLAKY.csv.
- An instance may be quarantined only after median-of-5 variance exceeds the measured noise band on an otherwise stable machine.
- Quarantined instances stay in stress runs but cannot be used as promotion evidence.
- Removing an instance from FLAKY.csv requires a written note and a stable median-of-5 rerun.
```

---

# 2. Code-level invariants to preserve everywhere

Keep these invariants visible while implementing every task.

```text
Assignment/trail:
  - every assigned variable appears exactly once on trail
  - decision_level[var] matches the trail frame
  - root assignments have level 0
  - propagate_head never points past trail length
  - temporary assumptions always backtrack to root before returning to normal search
  - temporary assumptions do not update saved/target/best phase unless explicitly requested
  - temporary assumptions do not enter VMTF/heap decision statistics

Reasons:
  - unassigned variables have NO_REASON
  - decision variables have NO_REASON
  - propagated variables have either a live clause reason or encoded binary reason
  - locked learned clauses cannot be deleted
  - binary reason expansion during conflict analysis is exactly the corresponding binary clause
  - no formula edit may delete a live reason clause unless it first installs a valid replacement reason or backtracks/removes that assignment safely at level 0
  - root-level reasons are treated as live reasons for deletion and GC purposes
  - deletion and GC decisions consult a freshly built ReasonPinSet rather than ad-hoc reason checks

Watchers:
  - long live clauses have two valid watches
  - binary clauses in binary_implications are also represented for proof/model/debug or have an explicit reason/proof path
  - stale deleted watchers are tolerated only if propagation checks deletion before clause use
  - in-place clause rewriting must either detach watchers first or mark watch lists dirty and make propagation skip impossible watchers
  - do not mix strict detach and dirty-watch strategies in the same mutator without an explicit contract

Clause arena:
  - header size matches literal count and extra words
  - learned clauses have activity/meta words as expected
  - original clauses have abstraction metadata only when the current mode expects it
  - LBD metadata starts in a side table; any later arena migration is one-time, atomic, and benchmark-justified
  - later metadata should use side tables unless there is a measured reason to touch arena layout again
  - GC rewrites every clause reference: watchers, reasons, learned ids, original ids, occurrence refs, and proof/model side structures
  - debug/invariant mode stores a generation for every allocated clause slot and validates every ClauseRef before dereference
  - any ClauseRef copied into side structures must be registered in a rewrite visitor or marked debug-only

Scratch / allocation:
  - propagation, conflict analysis, decision selection, and reduce-db candidate scans do not allocate in steady state
  - temporary vectors are owned by Solver scratch fields and reused
  - formula-edit transactions may allocate, but budgeted inprocessing passes must account for that cost
  - any new hot loop must state whether it allocates and which scratch buffers it uses
  - allocation-heavy prototypes stay experimental until allocation counters are flat on search-core

Occurrence lists:
  - occurrence refs are u32 unless a measured overflow risk appears
  - dirty lists are cleaned before exact scans
  - n_occ is updated on add/delete/strengthen/substitute
  - eliminated variables do not re-enter occurrence scheduling

Proof:
  - every learned clause used for UNSAT proof is logged
  - every generated resolvent needed for BVE is logged
  - every strengthened clause is logged before the old clause is deleted
  - deletion logging exists before inprocessing transformations rely on removed-clause proof context
  - proof buffering stays enabled; increase buffer capacity only after measuring proof throughput

SAT model:
  - final assignment satisfies original input CNF
  - every satisfiability-preserving deletion that can break direct SAT assignment needs an extension entry or must remain disabled for SAT-output runs
  - extension stack is applied in reverse order
  - equivalent-literal substitution records enough information to reconstruct the eliminated literal value
  - BCE remains disabled until model reconstruction is proven
  - every extension entry records its approximate literal memory cost
  - extension replay has an invariant-check mode that verifies each replay step satisfies the clauses it restores
  - transformations may be disabled by extension memory budget even if they are proof-correct

Decision eligibility:
  - eliminated/frozen variables are never decision candidates
  - heap/VMTF insertion paths check decision_var[v]
  - chronological backtracking must not reinsert or reassign non-decision variables incorrectly

Cache and layout invariants for hot paths:
  Propagation watcher entries:
    - watcher entry size <= 16 bytes (target: 8 bytes: blocker + ClauseRef)
    - watcher list is contiguous Vec<Watcher>, not Vec<Box<Watcher>>
    - blocker literal is the first field so it lives at the start of the cache line
    - if blocker miss rate exceeds 30% in profiling, revisit blocker selection before adding new propagation work

  Binary implication adjacency:
    - BinaryEdge size <= 8 bytes (target: 8 bytes: implied i32 + BinaryClauseId u32)
    - in Flat representation, edges for one literal are contiguous; offsets array fits in L2
    - propagation walks at most one cache line per binary edge in the hot path

  Clause arena access:
    - clause header (size + LBD/tier metadata) fits in 8 bytes (one word)
    - first two literals (the watched positions) follow the header so the first cache line load covers header + watchers + 1-2 more literals
    - learned-meta side tables are L2-resident for typical learned-clause counts

  Allocation rules for performance-claim tasks:
    - allocation count per 1M propagations must be reported in JSON_STATS as hot_allocations_estimated
    - any new hot path that allocates in steady state is rejected unless the task explicitly justifies it with a counter delta
```

---

# 3. Section 0 — Baseline, scaffolding, and test infrastructure

Execution order matters. The fork must happen before any extraction, configuration work, benchmark setup, or source audit. Task sections follow dependency order except for the intentionally named `0.0a`, which runs after 0.5 because it depends on benchmark tooling.

## 0.0 Fork to a new solver directory

### Goal

Create an identity copy of solver 10 that can be A/B tested against the unchanged baseline.

### Implementation

```bash
cp -R solver/10-bve-subsume solver/11-kissat-search
```

Update `solver/11-kissat-search/Cargo.toml`:

```toml
[package]
name = "sat-solver-11-kissat-search"

[[bin]]
name = "sat-solver"
path = "src/main.rs"
```

Update any local `build.sh` / `run.sh` references while preserving the binary contract:

```text
run.sh <cnf-path> <out-dir>
```

Keep release profile settings identical to solver 10. Do not relax optimization settings.

Add:

```bash
tools/run_solver11_baseline.sh
tools/status_compare.py
tools/validate_solver_result.py
```

`tools/validate_solver_result.py` validates one solver output directory:

```text
inputs:
  - --cnf <path>
  - --out-dir <path>
  - --expected-status SAT|UNSAT|UNKNOWN|PARSE_ERROR|any
  - --proof-policy off|drat
  - --require-json-stats on|off

checks:
  - status file exists and matches documented schema
  - SAT model satisfies the original CNF
  - UNSAT proof exists and verifies when proof policy is drat
  - UNKNOWN has no finalized proof and no model
  - ParseError has no model or finalized proof
  - JSON_STATS required fields exist when requested
```

`tools/status_compare.py` is a minimal pre-0.5 helper: it reads two existing `results.csv` files, compares instance/status pairs, reports solved-count deltas if the columns exist, and has no dependency on `benchmarks/iteration/baseline.csv` or `tools/compare_bench.py`.

Script contents:

```bash
#!/usr/bin/env bash
set -euo pipefail

cd solver/11-kissat-search
cargo test

cargo build --release

cd ../..
bash tools/smoke_test.sh solver/11-kissat-search
mkdir -p log/baseline-lock

bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/10-bve-subsume \
  --log-dir log/baseline-lock/solver10

bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/profiling solver/11-kissat-search \
  --log-dir log/baseline-lock/solver11

python3 tools/status_compare.py \
  --before log/baseline-lock/solver10/results.csv \
  --after log/baseline-lock/solver11/results.csv \
  > solver/11-kissat-search/BASELINE_LOCK.raw.txt

{
  echo "solver10_dir=solver/10-bve-subsume"
  echo "solver11_dir=solver/11-kissat-search"
  echo "date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "rustc=$(rustc --version 2>/dev/null || true)"
  echo "cargo=$(cargo --version 2>/dev/null || true)"
  echo "uname=$(uname -a)"
  echo "solver10_binary_sha256=$(sha256sum solver/10-bve-subsume/target/release/sat-solver 2>/dev/null | awk '{print $1}')"
  echo "solver11_binary_sha256=$(sha256sum solver/11-kissat-search/target/release/sat-solver 2>/dev/null | awk '{print $1}')"
  echo "env_SAT=$(env | sort | grep '^SAT_' || true)"
  echo "solver10_log=log/baseline-lock/solver10/results.csv"
  echo "solver11_log=log/baseline-lock/solver11/results.csv"
} >> solver/11-kissat-search/BASELINE_LOCK.raw.txt
```

### Tests

```bash
cd solver/11-kissat-search
cargo build
cargo test
cd ../..
bash tools/smoke_test.sh solver/11-kissat-search
```

### Acceptance

```text
- copied solver matches solver 10 status counts on smoke and profiling before any algorithmic change
- solver11 baseline outputs validate through tools/validate_solver_result.py on all smoke instances
- BASELINE_LOCK.raw.txt records solver10 vs solver11 status counts, solved count if available, rustc/cargo version, binary sha256, command log path, environment SAT_* variables, and benchmark log paths
- no Phase 1 work starts until BASELINE_LOCK.raw.txt exists
- after 0.5 creates compare_bench.py and benchmarks/iteration/baseline.csv, regenerate a richer BASELINE_LOCK.txt from the same raw logs
```

### Dependency

None.

---

## 0.0b Thin-slice vertical: fork → LBD-only → smoke → measure

### Goal

Before completing the rest of section 0, validate that the planned pipeline (config, stats, smoke, validator, compare_bench, golden tests) works end-to-end on a tiny but real solver change.

### Dependencies

0.0.

### Scope

- The minimum SolverConfig needed for one feature flag (`SAT_USE_LBD=on/off`).
- Hand-written shim versions of `tools/smoke_test.sh`, `tools/bench.sh`, and `tools/validate_solver_result.py` (no full schemas, no JSON stats, no replay).
- A real but minimal LBD implementation: `compute_lbd_from_lits` + side-table storage. No reduction policy change.
- 5 hand-picked instances: 1 small SAT, 1 small UNSAT, 1 medium SAT, 1 K4-like, 1 Timetable-like.

### Acceptance

```text
- SAT_USE_LBD=off matches solver-10 status counts on all 5 instances.
- SAT_USE_LBD=on matches status counts on all 5 instances; LBD counters are nonzero.
- compare_bench.py shim runs and reports per-instance delta.
- one obvious bug in the planned pipeline is found and recorded in log/0.0b/findings.md, or the file says "no surprises."
```

### Rationale (kept in this section for execution clarity)

```text
- The full section 0 takes ~10 tasks before any solver-algorithm work. A pipeline flaw discovered in task 1.3 is expensive to unwind.
- This thin slice validates the *shape* of the pipeline cheaply, then hardens after.
- It also produces the first real "solver-11 is better than solver-10" data point for one feature, which calibrates the team's noise band before the full bench harness is built.
- A subsequent reproducibility CI gate (see 0.9) will assert bitwise-identical artifacts from this slice forward.
```

### What this task is NOT

- It does not freeze any decision made in 0.1–0.9.
- It does not promote any default.
- Its shim tooling is replaced (not extended) by 0.3a, 0.4, 0.5.
- The minimal LBD implementation is re-derived in task 1.1 once the full reason/propagation scaffold exists; the 0.0b version may be discarded or kept solely as a regression fixture.

---

## 0.1 Architecture boundary and ownership map

### Goal

Prevent solver 11 from becoming a larger `main.rs` monolith while preserving solver 10 behavior.

### Additional architecture rule: capability-based mutation (introduced incrementally)

Module extraction is not enough. New subsystems must receive the narrowest capability object that can perform their job, not unrestricted `&mut Solver`. However, capability types are introduced **incrementally**, when the first task that needs the seam is implemented. Pre-declaring six capability wrappers during the fork stage forces a large refactor before any algorithmic change requires it.

Until a task introduces the first capability seam, the rule is simpler:

```text
- No `pub fn` in a new pass module (1.x or 2.x) takes unrestricted `&mut Solver` if the function does not need it.
- The first task that introduces a capability type (likely 1.0a, with `TemporaryAssumptionCtx`) sets the pattern: a private newtype wrapping `&mut Solver`, exposing only the subset of methods needed by callers.
- Subsequent passes follow the same pattern, but capabilities are added in the task that first needs them, not pre-declared in 0.1.
- The "no unrestricted &mut Solver" rule applies only to new pass modules; existing `Solver::*` methods inherited from solver-10 are exempt until a task explicitly refactors them.
- capability structs expose methods only; their fields remain private.
- pass modules may not store capability objects beyond a single call frame.
- no public method on a capability may return `&mut Solver`.
- `validate_solver11_plan.py` scans new `src/*` pass modules for `pub fn .*&mut Solver`.
- SOLVER11_STATE.md includes an `unrestricted_mut_solver_exceptions` table with expiry task IDs (the table starts empty and is populated as capabilities are added).
- After 2.1a, all destructive formula edits must go through whatever capability `InprocessCtx`-equivalent exists at that point (the 2.1a task defines the exact type).
```

Expected capability roll-out order (informative, not normative):

```text
1.0a  introduces TemporaryAssumptionCtx
1.6   introduces a propagation-level capability if binary fast path needs it
2.0   introduces InprocessCtx
2.1   introduces ProofCtx (proof finalization/temp lifecycle)
2.1a  introduces ModelCtx (extension-entry append)
2.6+  occurrence-list capability when BVE scheduling refactor needs it
```

Each of the above is a normal task that adds tests, an SOLVER11_STATE.md row, and a migration of the call sites it touches. None of them are required before task 1.0.

### Implementation

Before algorithmic changes, create module boundaries in two stages.

Stage A is mandatory before Phase 1 and should be small:

```text
src/config.rs       SolverConfig, parsing, validation, profile/default selection
src/stats.rs        counters, timers, JSON/trace output
src/lit.rs          literal/variable encoding helpers
src/limits.rs       conflict/propagation/tick/wall-clock limit checks
src/output.rs       status/model/proof-path contract and temp-proof lifecycle
src/check.rs        invariant checks and original-CNF model verification
```

Stage B is incremental and should happen only when a task needs the boundary:

```text
src/arena.rs        clause arena, ClauseRef, clause header/meta helpers
src/trail.rs        assignment, decision levels, reasons, trail frames
src/watch.rs        long watchers, binary implications, propagation contracts
src/proof.rs        DRAT/proof buffering and proof event API
src/model.rs        extension-stack replay and original-CNF assignment repair
src/branch.rs       heap/VMTF/rephase phase-selection machinery
src/search.rs       CDCL search loop, restart/reduce/mode policy glue
src/simp.rs         existing BVE/BSR/occurrence machinery
src/inprocess.rs    scheduler and pass orchestration only
```

Extraction rules:

```text
- each extraction patch is behavior-preserving
- no new feature flag changes behavior during extraction
- no algorithmic tuning in extraction patches
- every module exposes narrow methods rather than public fields
- every module documents the invariants it owns
- do not move an entire subsystem merely to satisfy the map; move code when the next task needs the seam
- add façade methods first, then move internals later
- every extraction patch must pass a solver-10-vs-solver-11 status comparison on smoke-plus
```

Minimum ownership contracts:

```text
Trail owns:
  - assignment values
  - decision levels
  - trail frames
  - reason references

Watch owns:
  - long watcher mutation
  - binary implication adjacency
  - propagation conflict representation

Arena owns:
  - clause layout
  - learned/original metadata
  - GC reference rewriting entry points
  - debug generation checks for ClauseRef and ClauseHandle

Proof owns:
  - DRAT add/delete output
  - proof temp-file lifecycle
  - proof counters

Model owns:
  - original-CNF model check
  - extension-stack replay

Capability ownership (applies once each capability is introduced; see roll-out order above):
  - `PropagationCtx`, when introduced, owns no allocation-heavy buffers and must not allocate in steady-state propagation.
  - `InprocessCtx`, when introduced, is the only capability allowed to commit FormulaEditTxn after 2.1a.
  - `ProofCtx`, when introduced, is the only capability allowed to finalize or rename proof files.
  - `ModelCtx`, when introduced, is the only capability allowed to append destructive-edit extension entries.
  - Until each capability lands, the equivalent rule applies to the existing `Solver::*` methods that perform that role, and the introducing task migrates the call sites in the same patch.
```

### Tests

```bash
cd solver/11-kissat-search
cargo test
cd ../..
bash tools/smoke_test.sh solver/11-kissat-search
```

### Acceptance

```text
- behavior matches baseline before and after extraction
- no feature default changes
- no new public cross-module mutable access to trail/watch/arena/proof internals
- SAT_CHECK_INVARIANTS=on catches stale ClauseRef/BinaryClauseId use after a forced GC/rebuild test
```

Debug handle tests:

```text
test_debug_clause_ref_generation_rejects_deleted_slot
test_gc_rewrite_visitor_updates_all_registered_clause_refs
test_binary_clause_id_generation_rejects_deleted_binary_in_debug
```

### Dependency

0.0.

---

## 0.2 Source map and baseline audit

### Goal

Give coding agents the current solver map and force a one-time audit of line numbers and entry points before edits.

### Implementation

Create:

```text
solver/11-kissat-search/SOLVER11_STATE.md
```

It should record:

```text
Known source files:
  - src/main.rs: arena, watchers, propagate, conflict analysis, restart, reduce_db, simplify, search loop, DRAT logging
  - src/simp.rs: occurrence lists, backward subsumption, BVE, model extension

Entry points to audit and update after fork:
  - Solver::new
  - Solver::solve_with_proof
  - Solver::propagate
  - Solver::analyze_conflict_to_scratch
  - Solver::reduce_db
  - Solver::eliminate or simp::eliminate

Known missing features at baseline:
  - glue/LBD
  - chronological backtracking
  - best/target phases
  - focused/stable mode switching
  - VMTF queue
  - rephasing
  - vivification
  - failed literal probing
  - HBR
  - equivalent literal substitution
  - transitive reduction
  - gate extraction
  - walking local search
  - DRAT deletions
```

### Acceptance

```text
Line numbers and function names in SOLVER11_STATE.md match the forked solver.
```

### Dependency

0.0.

---

## 0.3 Single configuration object, feature flags, profiles, and validation

### Goal

Avoid scattered `std::env::var` calls in hot paths. All feature selection should go through one parsed config object, with reproducible profile defaults and strict validation for invalid combinations.

### Implementation

Add:

```rust
struct SolverConfig {
    // Reproducibility / profile selection.
    profile: SolverProfile,
    proof_policy: ProofPolicy,
    feature_statuses: Vec<FeatureStatus>,
    config_dump: bool,
    config_out: Option<PathBuf>,
    config_replay: Option<PathBuf>,
    config_replay_allow_overrides: bool,
    strict_config: bool,
    run_label: Option<String>,

    // Output / diagnostics.
    stats_json: bool,
    trace_full: bool,
    check_invariants: bool,
    deterministic_seed: u64,

    // Internal limits.
    conflict_limit: Option<u64>,
    propagation_limit: Option<u64>,
    tick_limit: Option<u64>,
    wall_limit_sec: Option<f64>,
    rss_limit_mb: Option<u64>,
    learned_lit_limit: Option<u64>,
    binary_clause_limit: Option<u64>,
    extension_bytes_limit: Option<u64>,
    proof_bytes_limit: Option<u64>,

    // Phase 1 search.
    use_lbd: bool,
    update_reason_lbd: bool,
    restart_policy: RestartPolicy,
    reduce_policy: ReducePolicy,
    phase_policy: PhasePolicy,
    search_mode_policy: SearchModePolicy,
    chrono_backtrack: bool,
    binary_fast_path: bool,
    clause_min_mode: ClauseMinMode,
    vmtf: bool,
    rephase: bool,

    // Phase 1 search tuning limits. These fields are listed here so references in this plan are explicit.
    // A task may not use one until it also adds parser, CONFIG_SCHEMA.csv, dump, replay, README/internal docs, and config_hash coverage.
    minimize_depth_limit: u32,
    chrono_max_delta: usize,
    mode_init_conflicts: u64,
    mode_interval_scale: f64,
    rephase_init_conflicts: u64,

    // Solver-10 compatibility and existing preprocessing controls.
    simplification: bool,
    bve: bool,
    full_bsr: bool,

    // Phase 2 simplification.
    inprocess: bool,
    vivify: bool,
    probe: bool,
    hbr: bool,
    transitive: bool,
    forward_subsume: bool,
    gate_extract: bool,
    gate_bve: bool,
    rcheck: bool,

    // Phase 2 pass budgets. A task may not reference one until it also adds parser, CONFIG_SCHEMA.csv, dump, replay, README/internal docs, and config_hash coverage.
    inprocess_interval_conflicts: u64,
    inprocess_max_rounds: u64,
    vivify_ticks_budget: u64,
    vivify_max_clause_len: usize,
    probe_ticks_budget: u64,
    eliminate_ticks_budget: u64,
    transitive_max_depth: u32,
    transitive_ticks_per_source: u64,
    transitive_max_removed_per_round: u64,
    rcheck_ticks_budget: u64,
}
```

Enums:

```rust
enum SolverProfile {
    Baseline,       // solver-10-equivalent: Safe search × Off preprocess
    Default,        // Validated Phase 1 search × Conservative preprocess
    Fast,           // Strong Phase 1 search × GateAware preprocess (target after Phase 2)
    Experimental,   // any combination of validated and unvalidated features
}

struct ProfileAxes {
    search: SearchAxis,        // Safe | Validated | Strong
    preprocess: PreprocessAxis, // Off | Conservative | GateAware
}

// Legacy profile name mapping (kept only for cross-reference with older log
// artifacts; new code must use the SolverProfile + ProfileAxes pair):
//   search-conservative  -> Default, search=Validated, preprocess=Off
//   search-strong        -> Default, search=Strong,    preprocess=Off
//   inprocess-conservative -> Default, search=Validated, preprocess=Conservative
//   inprocess-gate-aware -> Fast,    search=Strong,    preprocess=GateAware

enum ProofPolicy { Off, Drat, Lrat }

enum FeatureMaturity {
    ParkingLot,
    Experimental,
    SmokeSafe,
    OracleSafe,
    ProofValidated,
    DiscriminatingValidated,
    FullSetValidated,
}

struct FeatureStatus {
    name: &'static str,
    enabled: bool,
    maturity: FeatureMaturity,
    proof_validated: bool,
    model_validated: bool,
    full_set_validated: bool,
    validation_artifact: Option<PathBuf>,
}

enum RestartPolicy { LegacyLuby, KissatEma, Reluctant }
// MiniSat-style restart removed: it is never used or benchmarked by the Phase 1 candidate matrix
// (legacy-luby vs kissat-ema vs reluctant). If a task later motivates it, reintroduce in a focused experiment.

enum ReducePolicy { LegacyActivity, Activity, LbdTiered }

enum PhasePolicy { Legacy, Saved, TargetThenSaved, BestThenTargetThenSaved }
// `Negative` (always-false) is a degenerate case of Saved with uninitialized phase; not exposed.
// `Kissat` was an alias for TargetThenSaved; removed to keep one canonical name per behavior.

enum SearchModePolicy { Single, FocusedStable }
enum ClauseMinMode { Off, Basic, RecursiveLimited, InBlockShrink }
```

Environment variables:

```text
SAT_STATS_JSON=on/off
SAT_TRACE_FULL=on/off
SAT_CHECK_INVARIANTS=on/off
SAT_SEED=<u64>
SAT_PROFILE=baseline/default/fast/experimental
SAT_SEARCH_AXIS=safe/validated/strong          # optional axis override
SAT_PREPROCESS_AXIS=off/conservative/gate-aware # optional axis override
SAT_PROOF=off/drat/lrat
SAT_CONFIG_DUMP=on/off
SAT_CONFIG_OUT=<path>
SAT_CONFIG_REPLAY=<path>
SAT_CONFIG_REPLAY_ALLOW_OVERRIDES=on/off
SAT_STRICT_CONFIG=on/off
SAT_RUN_LABEL=<freeform-label>
SAT_LIMIT_CONFLICTS=<u64>
SAT_LIMIT_PROPAGATIONS=<u64>
SAT_LIMIT_TICKS=<u64>
SAT_LIMIT_WALL_SEC=<float>
SAT_LIMIT_RSS_MB=<u64>
SAT_LIMIT_LEARNED_LITS=<u64>
SAT_LIMIT_BINARY_CLAUSES=<u64>
SAT_LIMIT_EXTENSION_BYTES=<u64>
SAT_LIMIT_PROOF_BYTES=<u64>

SAT_USE_LBD=on/off
SAT_LBD_UPDATE_REASONS=on/off
SAT_RESTART=legacy-luby/kissat-ema/reluctant
SAT_REDUCE=legacy/activity/lbd-tiered
SAT_PHASE=legacy/saved/target-then-saved/best-then-target-then-saved
SAT_SEARCH_MODE=single/focused-stable
SAT_CHRONO=on/off
SAT_BINARY_FAST=on/off
SAT_CLAUSE_MIN=off/basic/recursive-limited/inblock
SAT_VMTF=on/off
SAT_REPHASE=on/off
SAT_MINIMIZE_DEPTH_LIMIT=<u32>
SAT_CHRONO_MAX_DELTA=<usize>
SAT_MODE_INIT_CONFLICTS=<u64>
SAT_MODE_INTERVAL_SCALE=<float>
SAT_REPHASE_INIT_CONFLICTS=<u64>

SAT_INPROCESS=on/off
SAT_VIVIFY=on/off
SAT_PROBE=on/off
SAT_HBR=on/off
SAT_TRANSITIVE=on/off
SAT_FORWARD_SUBSUME=on/off
SAT_GATE_EXTRACT=on/off
SAT_GATE_BVE=on/off
SAT_RCHECK=on/off
SAT_INPROCESS_INTERVAL_CONFLICTS=<u64>
SAT_INPROCESS_MAX_ROUNDS=<u64>
SAT_VIVIFY_TICKS=<u64>
SAT_VIVIFY_MAX_CLAUSE_LEN=<usize>
SAT_PROBE_TICKS=<u64>
SAT_ELIMINATE_TICKS=<u64>
SAT_TRANSITIVE_MAX_DEPTH=<u32>
SAT_TRANSITIVE_TICKS_PER_SOURCE=<u64>
SAT_TRANSITIVE_MAX_REMOVED_PER_ROUND=<u64>
SAT_RCHECK_TICKS=<u64>
```

Legacy compatibility variables:

```text
SAT_SIMPLIFICATION=on/off   # maps to existing solver-10 simplification default controls
SAT_BVE=on/off              # maps to existing preprocessing BVE enablement
SAT_FULL_BSR=on/off         # maps to existing full-BSR behavior
```

Compatibility rules:

```text
- legacy variables are parsed only in config.rs
- legacy variables are converted into explicit SolverConfig fields before solving starts
- legacy variables are included in config_hash and SAT_CONFIG_DUMP
- no legacy std::env::var call may remain in search, propagation, simplification, or proof hot paths
- if a legacy variable conflicts with a new explicit SAT_* flag, SAT_STRICT_CONFIG=on rejects the run
- SAT_ELIMINATE_INPROCESS is not accepted as a legacy alias; use SAT_INPROCESS plus explicit BVE flags
```

Config validation:

```text
- fail fast on unknown `SAT_*` variables when SAT_STRICT_CONFIG=on
- fail fast on invalid enum values
- normalize profile defaults before explicit per-flag overrides
- when SAT_CONFIG_REPLAY is set, load the effective config from that file
- by default, SAT_CONFIG_REPLAY rejects env overrides other than:
    * SAT_CONFIG_OUT
    * SAT_RUN_LABEL
    * SAT_STATS_JSON
    * SAT_TRACE_FULL
    * SAT_LIMIT_WALL_SEC
    * SAT_LIMIT_RSS_MB
- allow arbitrary replay overrides only with SAT_CONFIG_REPLAY_ALLOW_OVERRIDES=on, and record `replay_overridden=true` plus the override list in JSON_STATS
- when SAT_CONFIG_OUT is set, write the effective config and config_hash before parsing CNF
- replay files must include schema_version, solver version, profile, proof policy, seed, limits, every config field, all feature flags, and per-feature maturity records
- any task that introduces a config field must add:
    * SolverConfig field
    * env var parser row
    * CONFIG_SCHEMA.csv row
    * SAT_CONFIG_DUMP output
    * replay serialization/deserialization
    * README or hidden/internal documentation status
    * config_hash coverage
- reject contradictory combinations unless explicitly marked experimental
- print effective config when SAT_CONFIG_DUMP=on
- include config_hash, profile, proof policy, and per-feature maturity summary in JSON stats
```

Invalid combinations to reject by default:

```text
- SAT_REDUCE=lbd-tiered with SAT_USE_LBD=off
- SAT_RESTART=kissat-ema with SAT_USE_LBD=off
- SAT_VMTF=on with SAT_SEARCH_MODE=single
- SAT_HBR=on with SAT_PROBE=off
- SAT_GATE_BVE=on with SAT_GATE_EXTRACT=off
- SAT_PHASE=target, SAT_PHASE=best, SAT_PHASE=negative, and SAT_PHASE=kissat are invalid aliases; use explicit target-then-saved or best-then-target-then-saved
- SAT_RESTART=minisat is no longer accepted; use legacy-luby, kissat-ema, or reluctant
```

Named profile intent (4 named profiles, composed from two axes):

```text
baseline (search=Safe, preprocess=Off):
  - solver-10-equivalent: no Phase 1/2 features enabled
  - kept indefinitely for A/B comparison and the "I want predictable behavior" use case

default (search=Validated, preprocess=Conservative):
  - search axis: every Phase 1 feature that passed smoke + oracle + proof + discriminating + regression-guards
  - preprocess axis: vivification + probing only (no gate-aware BVE, no HBR)
  - this is what `sat-solver` runs with no flags
  - subsumes the previous "search-conservative" and "inprocess-conservative" intents

fast (search=Strong, preprocess=GateAware):
  - search axis: default plus VMTF, rephase, recursive-limited minimization, target+best phase
  - preprocess axis: default plus HBR, gate extraction, gate-aware BVE, transitive reduction
  - target profile after Phase 2 completes
  - subsumes the previous "search-strong" and "inprocess-gate-aware" intents

experimental:
  - any combination of validated and unvalidated features
  - never used for promotion acceptance unless explicitly stated
```

Composition rule:

```text
- SAT_PROFILE=fast implies fast.search AND fast.preprocess axes.
- SAT_SEARCH_AXIS / SAT_PREPROCESS_AXIS may override the composed default.
- For example, `SAT_PROFILE=fast SAT_PREPROCESS_AXIS=conservative` is permitted via explicit axes;
  it is not a named profile and so is reported with profile=experimental in JSON_STATS but
  records the override pair.
- A run with axes not matching any named profile reports the explicit axis tuple.
```

Parking-lot flags:

```text
Do not add SAT_WALK, SAT_SWEEP, SAT_ELS, or SAT_BCE to SolverConfig until the corresponding Appendix A feature is explicitly unparked by milestone triage.
Treat those names as the parking-lot denylist: they may appear in PLAN.md only as forbidden or future-after-unpark flags, not as active config variables.
If unparked, add them in the same task that adds tests, maturity ledger entries, CONFIG_SCHEMA.csv rows, FEATURES.csv rows, and profile rules.
```

Promotion sequence:

```text
1. promote feature to experimental
2. promote into the `default` profile's search and/or preprocess axis after discriminating + regression-guards
3. mark feature ProofValidated only after proof-on validation
4. promote into `fast` only after `default` shows a stable win and the feature earns its preprocess/search axis seat
5. promote to a baseline-candidate replay artifact only after end-of-phase full-set validation
6. promote baseline-candidate to baseline only after the default-profile hardening gate
```

Candidate replay rule:

```text
- candidate runs use SAT_PROFILE=experimental until promotion
- promoted profiles (baseline/default/fast) are generated from validated SAT_CONFIG_REPLAY files, not hand-copied env snippets
- every candidate config must include SAT_PROOF, SAT_SEED, SAT_CONFIG_OUT, and concrete values for every feature flag in scope
- axis-override runs (SAT_SEARCH_AXIS / SAT_PREPROCESS_AXIS) record the explicit axis pair in JSON_STATS so promotion comparison is unambiguous
```

Default-profile hardening gate:

```text
- README examples run successfully through config replay
- tools/ci_solver11_fast.sh passes
- tools/ci_solver11_matrix.sh passes
- tools/ci_solver11_proof_model.sh passes
- full-set proof-off and proof-on scorecards are both attached
- regression-guards have no lost solved instances unless accepted in milestone triage
- SAT model validation passes on SAT-heavy smoke-plus and discriminating SAT instances
- UNSAT proof verification passes on proof-heavy UNSAT set
- memory limits and extension/proof budgets have no unexplained UNKNOWN spikes
- compare_bench.py report includes no unchecked UNSAT proofs and no SAT model-check failures
- FEATURES.csv rows for every baseline-enabled feature have validation artifacts
```

Per-feature maturity ledger:

```text
solver/11-kissat-search/FEATURES.md:
  - one row per feature flag
  - current maturity
  - proof/model validation state
  - last validation artifact path
  - promoted profiles that enable it
  - risk IDs
```

Machine-readable maturity ledger:

```text
solver/11-kissat-search/FEATURES.csv:
  - feature_flag
  - config_field
  - maturity
  - proof_validated
  - model_validated
  - full_set_validated
  - promoted_profiles
  - validation_artifact
  - risk_ids
  - last_changed_task
```

Single source of truth: the `SolverConfig` struct is annotated with a `#[config_field(...)]` attribute (proc macro or `build.rs` code generation) that captures: env variable, allowed values, per-profile defaults, conflicts/requires constraints, is_feature_flag, is_legacy_alias, introduced_task, and short description. Similarly, `enum SolverFeature` variants carry `#[feature(maturity=..., proof_validated=..., ...)]` attributes.

At build time, codegen emits:

```text
target/generated/CONFIG_SCHEMA.csv        # for tooling and CI
target/generated/config_env_parser.rs     # env parsing, no hand-written std::env::var
target/generated/config_dump.rs           # SAT_CONFIG_DUMP output
target/generated/config_replay.rs         # serialize/deserialize replay files
target/generated/README_config_table.md   # included into README via a one-line include
target/generated/FEATURES.csv             # derived from SolverFeature variants
```

The generated `CONFIG_SCHEMA.csv` and `FEATURES.csv` retain the same columns documented elsewhere in this plan (env_var, config_field, type, allowed_values, per-profile defaults, is_feature_flag, is_limit, is_legacy_alias, conflicts_with, requires, introduced_task, documented_in_readme; and for FEATURES.csv: feature_flag, config_field, maturity, proof_validated, model_validated, full_set_validated, promoted_profiles, validation_artifact, risk_ids, last_changed_task).

Rules:

```text
- SolverConfig (with #[config_field]) and SolverFeature (with #[feature]) are the only hand-edited config/feature artifacts.
- Adding a config field requires only: one struct field, one #[config_field(...)] attribute, one CONFIG_SCHEMA.csv golden-diff review.
- Generated CSVs are checked into the tree and refreshed by `cargo run -p solver11-codegen`. A CI step regenerates and fails on drift.
- FEATURES.md is human-facing commentary and may be generated or manually maintained.
- SAT_CONFIG_DUMP includes the maturity row for every enabled feature.
- A promoted profile that enables a feature missing a `validation_artifact` attribute fails `cargo build` at codegen time, not at CI.
- `validate_solver11_plan.py` shrinks to: dependency-graph sanity, parking-lot denylist check, and PLAN.md/README.md cross-reference. Schema-vs-struct checks are unnecessary because schema is derived.
- README profile examples are validated against generated CONFIG_SCHEMA.csv defaults plus explicit overrides at CI time.
```

### Acceptance

```text
All new features default off until validated.
Defaults are changed one at a time only after milestone/profile promotion.
Feature maturity is tracked per feature, separately from performance profile selection.
No feature may enter the `default` or `fast` profile (or any non-baseline named axis combination) unless FEATURES.csv names the validation artifact.
No feature may be marked ProofValidated unless proof-on smoke and at least one proof-heavy benchmark set passed.
No feature may be marked FullSetValidated unless full-set run artifacts are linked.
No hot path calls std::env::var directly.
Every benchmark log records profile, proof policy, effective config, config_hash, seed, solver git SHA if available, rustc version if available, and solver binary mtime.
Invalid configs fail before parsing the CNF.
Limit behavior is split:
- global process limits such as wall time, RSS hard cap, conflict limit, propagation limit, and fatal proof write failure return UNKNOWN cleanly unless a valid SAT/UNSAT result was already derived
- optional pass budgets such as vivification ticks, BVE transaction memory, proof-byte soft budget, binary-clause soft budget, and extension-byte soft budget abort or disable the triggering pass and continue search when correctness allows
- only SAT_CONFIG hard limits marked `fatal=true` may force UNKNOWN
```

### Dependency

0.0, 0.1.

## 0.3a Minimal status and result-file schema

### Goal

Define the minimal output shape that smoke tests, bench.sh, status_compare.py, validate_solver_result.py, and compare_bench.py will parse.

### Dependencies

0.0, 0.3.

### Implementation

Document the pre-proof minimal contract:

```text
- exact status strings: SAT, UNSAT, UNKNOWN, PARSE_ERROR
- status file path inside out_dir
- result JSON path inside out_dir: out_dir/result.json
- model file path inside out_dir for SAT
- proof file path inside out_dir for UNSAT when SAT_PROOF=drat
- stderr JSON_STATS prefix when enabled
- exit-code mapping for SAT, UNSAT, UNKNOWN, and PARSE_ERROR
```

`out_dir/result.json` minimum fields:

```text
schema_version
status
exit_code
termination_reason
unknown_reason
status_file
model_file
proof_file
proof_completeness
model_check_result
proof_check_result
config_hash
profile
proof_policy
stats_json_seen
```

Add:

```rust
enum SolveStatus { Sat, Unsat, Unknown, ParseError }
```

Rules:

```text
- tools/status_compare.py, tools/validate_solver_result.py, and tools/compare_bench.py (and their `sat-bench` successors) must consume this schema.
- `result.json` is mandatory for every normally-exited solver run from 0.3a onward. Tooling no longer falls back to legacy status-file-only parsing.
- The solver may continue to emit the legacy status file for one phase as a compatibility output; benchmark tooling treats absent `result.json` from a normally-exited process as an output-contract failure.
- The legacy reader path is removed from benchmark tooling at the end of Phase 1.
- The fuller OutputContract in 0.8 may add proof/model finalization checks but may not change the status strings without updating benchmark tooling and README tests.
```

### Acceptance

```text
- smoke_test.sh and bench.sh parse only the documented status/result schema
- malformed or missing status files from a normally exited process are reported as harness/output-contract failures, not solver UNKNOWN
- README and tools agree on exact output paths
```

## 0.4 Statistics and trace output

### Goal

Provide both machine-readable summary stats and human-readable deep trace counters.

### Implementation

Add a final machine-readable line to stderr when enabled:

```bash
SAT_STATS_JSON=on
```

Format:

```text
c JSON_STATS {...}
```

Build the JSON manually with `eprintln!`; do not add `serde_json`. Use a tiny local JSON writer with escaping tests. Do not concatenate raw strings into JSON. If dependency policy later permits `serde_json`, it may replace the local writer because stats output is not a hot path.

Minimum fields:

```text
schema_version
solver
solver_git_sha
rustc_version
binary_sha256
input_path
input_sha256
input_compressed_sha256
input_size_bytes
manifest_selection_version
manifest_expected_status
profile
proof_policy
config_hash
replay_overridden
replay_override_vars
feature_maturity_summary
seed
run_label
result
exit_code
status_file_status
termination_reason
unknown_reason
limit_hit
parse_error_kind
model_check_result
proof_check_result
proof_checker
proof_checker_version
elapsed_sec
parse_sec
preprocess_sec
search_sec
proof_sec

vars
original_clauses_initial
original_lits_initial
original_clauses_after_preprocess
original_lits_after_preprocess
learned_clauses_final
learned_lits_final

conflicts
decisions
propagations
restarts
reductions
gc_count
deleted_clauses
deleted_words

avg_decision_level
max_decision_level
avg_lbd
lbd_1
lbd_2
lbd_3_5
lbd_6_10
lbd_gt_10

binary_props
long_props
watch_scans
watch_stale_skips
watch_blocker_hits
watch_clause_loads

phase_saved_used
phase_target_used
phase_best_used
phase_initial_used
random_decisions

pre_bve_eliminated_vars
pre_bve_resolvents
pre_bsr_subsumed
pre_bsr_strengthened
pre_occ_clean_scans
pre_occ_clean_removed

inprocess_rounds
vivify_attempts
vivify_strengthened
vivify_subsumed
vivify_removed_literals
probe_attempts
probe_failed_literals
probe_units
hbr_added_binary
gate_equivalences_detected
transitive_removed
rcheck_attempts
rcheck_implied
proof_added_clauses
proof_deleted_clauses
proof_added_literals
proof_deleted_literals
proof_flushes
proof_bytes_written
max_rss_mb
rss_limit_mb
learned_lit_limit
binary_clause_limit
binary_clauses_final
binary_clauses_deleted
binary_implication_edges_final
extension_bytes_limit
proof_bytes_limit
proof_temp_created
proof_temp_deleted
proof_finalized
proof_incomplete
proof_write_errors
proof_state
proof_completeness
output_contract_state
extension_entries
extension_lits
extension_bytes_estimated
model_replay_steps
model_replay_conflicts
scratch_reuses
scratch_grows
hot_allocations_estimated
max_clause_buffer_len
max_txn_buffer_lits
```

Allowed solver-produced `termination_reason` values:

```text
sat
unsat
parse-error
conflict-limit
propagation-limit
tick-limit
wall-limit
rss-limit
learned-lit-limit
binary-clause-limit
extension-budget-fatal
proof-write-failure
internal-error
panic-cleanup
```

Harness-only result classification:

```text
- `external-timeout` is assigned by tools/bench.sh when the process is killed by the harness timeout.
- solver-produced JSON_STATS is optional for external-timeout.
- out_dir/result.json may be absent for external-timeout.
- missing result.json is a harness timeout fact only when bench.sh recorded the kill.
- missing result.json from a normally exited process is an output-contract failure.
```

Add `SAT_TRACE_FULL=on` for detailed human trace. Include:

```text
glue_sum, glue_max, glue_count
learned_size_sum, learned_size_max
glucose_restarts, luby_restarts, reluctant_restarts
chrono_backtracks, non_chrono_backtracks, chrono_skipped_levels
vivified_clauses, vivified_strengthened, vivified_subsumed, vivified_ticks
probe_failed_lits, probe_units, probe_ticks
transitive_removed, gate_equivs_found, appendix_equiv_substitutions
inprocess_runs, inprocess_ticks
phase_save_target, phase_save_best, rephases
decisions_focused, decisions_stable
mode_switches, seconds_focused, seconds_stable
learned_kept_tier1, learned_kept_tier2, learned_kept_tier3, learned_collected
decision_heap_pops, decision_heap_stale_pops, decision_heap_inserts
```

### Tests

Extend existing conflict-budget/restart tests to assert that new counters are zero before any conflict and move correctly after `note_conflict()`.

### Acceptance

```text
SAT_STATS_JSON=off and SAT_TRACE_FULL=off preserve existing behavior.
All tests pass with SAT_STATS_JSON=on.
Trace output for at least one K4-like and one Timetable-like instance is attached to every milestone benchmark log. JSON writer escaping tests cover labels, paths, and invalid enum diagnostics.
Benchmark tooling treats missing `termination_reason`, `input_sha256`, `config_hash`, or `output_contract_state` as a report warning, and treats SAT without `model_check_result=pass` as a correctness failure.
```

### Dependency

0.3, 0.3a.

---

## 0.5 Benchmark sets and comparison tooling

### Goal

Use both a fixed discriminating set and automatically selected iteration sets. The fixed set gives stable go/no-go continuity. The generated sets separate search, preprocessing, regression guards, and stress cases. Paired comparison tooling prevents cherry-picking and makes benchmark deltas reproducible.

### Implementation: fixed discriminating set

Create:

```text
benchmarks/discriminating/
benchmarks/discriminating/README.md
benchmarks/discriminating/MANIFEST.csv
```

Commit symlinks for at least these 20 instances, or the closest exact filenames available in the repo:

| # | Instance | Kissat reference | Solver reference | Likely root cause |
|---:|---|---:|---:|---|
| 1 | `battleship-16-31-sat` | SAT 0.18s | SAT 169.47s | phase / decision quality |
| 2 | `REGRandom-K4-L1-Seed40` | UNSAT 2.36s | TIMEOUT | preprocessing + LBD-tiered retention |
| 3 | `circuit_48in64out_with_800gates...` | SAT 7.24s | TIMEOUT | gate-aware BVE / phase |
| 4 | `mp1-Nb7T46` | SAT 8.69s | TIMEOUT | learned-clause quality |
| 5 | `544707...nw.shuffled-as.sat03-1671` | SAT 9.11s | SAT 105.5s | phase / restarts |
| 6 | `SC25_Timetable_C_392...` | SAT 10.35s | SAT 72.8s | search / restart strategy |
| 7 | `83aa...1.normalised` | SAT 17.39s | SAT 66.6s | search throughput |
| 8 | `SC25_Timetable_C_406...` | SAT 27.64s | SAT 772.8s | search / phase saving |
| 9 | `DLTM_twitter845_79_19` | SAT 29.12s | SAT 362.6s | search / phase saving |
| 10 | `div-mitern172` | UNSAT 30.5s | UNSAT 657.9s | clause DB / LBD |
| 11 | `ee5f...11.normalised` | SAT 32.24s | SAT 166.0s | search throughput |
| 12 | `Kakuro-easy-112-ext.xml.hg_7` | SAT 42.2s | SAT 240.6s | preprocessing throughput |
| 13 | `SCPC-500-1` | UNSAT 42.6s | UNSAT 270.8s | clause DB / LBD |
| 14 | `aaai10-planning-...-step20` | UNSAT 60.5s | UNSAT 381.7s | clause DB / LBD |
| 15 | `bp4_CSO_IXA_ZR.normalised` | SAT 62.6s | TIMEOUT | preprocessing |
| 16 | `sqrt-mitern171` | UNSAT 63.4s | UNSAT 464.0s | clause DB |
| 17 | `brocard_problem_large` | UNSAT 66.4s | UNSAT 479.9s | preprocessing residual |
| 18 | `case9` | SAT 78.9s | SAT 215.1s | search |
| 19 | `bp4_CSO_AM_IXA_LP.normalised` | UNSAT 83.9s | TIMEOUT | preprocessing |
| 20 | `1-TC-256-K-63` | SAT 101.8s | TIMEOUT | search trajectory |

Run with `-t 300` during iteration and `-t 600` at end-of-phase if needed.

First: establish a local Kissat reference baseline. Without this, "Kissat-class performance" is unfalsifiable because Kissat times measured on different CPUs/governors/memory commonly drift 20-60% versus local runs.

```text
tools/build_reference_solvers.sh
  - clones Kissat at a pinned commit and MiniSat at a pinned commit
  - builds release binaries
  - records build flags, commit SHAs, and binary sha256
  - writes log/reference-baselines/{kissat,minisat}/{commit.txt,binary.sha256}

tools/run_reference_baseline.sh
  - runs Kissat and MiniSat on benchmarks/profiling, benchmarks/discriminating, and benchmarks/sat-comp-2025
  - same machine, same governor, same wall/memory limits as solver-11 runs
  - writes log/reference-baselines/{kissat,minisat}/{profiling,discriminating,sat-comp-2025}/results.csv
  - runs the calibration instance from tools/bench.sh so we can detect cross-run thermal drift
```

Local-baseline rules:

```text
- The local Kissat baseline is the reference for "Kissat-class performance," not the external published table.
- The external Kissat times in discriminating/MANIFEST.csv become `kissat_reference_external_time` (informational); a new `kissat_reference_local_time` column is the authority.
- compare_bench.py reports "% of Kissat local" alongside "% of solver-10," so milestone gates can target a real number.
- The reference baseline is refreshed only when the benchmark machine changes; the SHA-pinned solvers stay constant.
- If reference solver building fails (toolchain mismatch, etc.), the project may proceed using the external table, but every promotion writeup must flag the missing local-baseline measurement.
```

`benchmarks/discriminating/MANIFEST.csv` fields:

```text
selection_version
logical_name
path
sha256
compressed_sha256
size_bytes
expected_status
expected_status_source
family
root_cause_tag
kissat_reference_external_time   # from the published table; informational
kissat_reference_local_time      # local-machine measurement; authoritative when present
solver10_reference_time
notes
```

Rules:

```text
- symlinks are allowed, but every symlink target must have a manifest row
- `tools/select_iter_bench.py --dry-run` verifies manifest checksums
- benchmark reports print manifest selection_version
- if the closest available filename is not exact, MANIFEST.csv must say so in notes
- expected_status_source must identify whether status came from solver 10, Kissat, MiniSat, brute force, or proof/model validation
```

### Implementation: generated iteration sets

Tooling consolidation: instead of separate Python scripts (`status_compare.py`, `validate_solver_result.py`, `select_iter_bench.py`, `compare_bench.py`, `extract_hot_instances.py`, `validate_solver11_plan.py`), provide a single Rust binary in the workspace:

```text
tools/sat-bench/                  # Cargo crate in the workspace
  src/main.rs
  subcommands:
    sat-bench status-compare      # was status_compare.py
    sat-bench validate-result     # was validate_solver_result.py
    sat-bench select-iter         # was select_iter_bench.py
    sat-bench compare             # was compare_bench.py
    sat-bench extract-hot         # was extract_hot_instances.py
    sat-bench validate-plan       # was validate_solver11_plan.py
    sat-bench profile             # was the script-only parts of profile_solver11.sh
  binary is sha256-pinned in BASELINE_LOCK.txt
```

Shell scripts (`tools/bench.sh`, `tools/smoke_test.sh`, `tools/ci_*.sh`, `tools/profile_solver11.sh`) remain shell — they orchestrate processes and don't carry stateful logic.

Add:

```text
tools/sat-bench/                  # Rust workspace crate (see above)
tools/run_matrix.sh
benchmarks/REFERENCE_SOLVERS.md
```

Rationale for consolidation:

```text
- one tool, one type system, one test suite for benchmark/validation logic
- no Python virtualenv on CI machines
- compare_bench runs on 100+ instances 5–10× faster (matters at end-of-phase)
- the binary can sha256-pin into BASELINE_LOCK.txt for reproducibility
```

Transition policy:

```text
- During Phase 1, Python scripts and `sat-bench` subcommands may coexist; the Python scripts call into the same data formats so output is interchangeable.
- The Python scripts are removed at the end of Phase 1 once each subcommand has a passing self-test and CI uses the Rust binary exclusively.
- Until then, references in this plan to `python3 tools/<script>.py` are read as "either the Python script or the matching `sat-bench` subcommand".
```

Inputs:

```text
log/bench-10-bve-preprocess-*/results.csv
log/bench-minisat-*/results.csv
log/bench-kissat-*/results.csv
benchmarks/profiling
benchmarks/profiling/minisat-simp-five
benchmarks/discriminating
```

`benchmarks/REFERENCE_SOLVERS.md` records:

```text
solver_name
version_string
git_sha_or_release
build_command
run_command_template
proof_policy
timeout
memory_limit
date_utc
machine_id_or_environment_block
```

Missing reference policy:

```text
- if Kissat/MiniSat logs are absent, select_iter_bench.py may still create smoke-plus and regression-guards from solver10 plus smoke data
- rows without a reference solver time must set kissat_time/minisat_time to NA and reference_status_source accordingly
- generated benchmark sets must mark `selection_confidence=low|medium|high`
- profile promotion cannot rely on low-confidence rows alone
```

Outputs:

```text
benchmarks/iteration/smoke-plus
benchmarks/iteration/search-core
benchmarks/iteration/preprocess-core
benchmarks/iteration/regression-guards
benchmarks/iteration/stress
benchmarks/iteration/holdout
benchmarks/iteration/killer-tests       # 10 hand-picked instances exercising common bug classes
benchmarks/iteration/baseline.csv
benchmarks/iteration/FLAKY.csv
```

`benchmarks/iteration/killer-tests` selection rule:

```text
- one instance per common solver bug class:
  * UIP off-by-one
  * learned-clause re-watcher staleness
  * BVE model-reconstruction failure
  * vivification wrong strengthening
  * DRAT deletion misorder
  * binary-clause GC reference drift
  * chrono backtrack reason corruption
  * gate-extraction false positive
  * extension-stack out-of-order replay
  * proof-buffering write-error mid-finalize
- run as a hard CI gate (any failure blocks merge)
- maintained as historical regressions; new entries require milestone-triage signoff
```

Selection policy:

```text
smoke-plus:
  - all smoke tests
  - a few small SAT and UNSAT cases that exercise proof/model output

search-core:
  - solver 10 and Kissat/MiniSat produce similar residual size but solver 10 is slower in search
  - include Timetable-like SAT instances
  - include bp4 / 1-TC style instances if present

preprocess-core:
  - solver 10 spends most time in preprocessing
  - include K4 / Kakuro-like instances
  - include occurrence-list-heavy cases

regression-guards:
  - instances where solver 10 is already faster than MiniSat/Kissat
  - include mp1-like cases
  - include cases previously regressed by scheduling or simplification changes

stress:
  - solver 10 timeout but Kissat solved
  - proof-heavy UNSAT cases
  - large occurrence-list / full-BSR cases

holdout:
  - not used for per-task tuning
  - used only at milestone/profile promotion
  - sampled from the same broad distributions as search-core and preprocess-core
  - refreshed only after a written benchmark-set update note
```

`baseline.csv` fields:

```text
instance
expected_status
proof_policy
solver10_time
kissat_time
minisat_time
solver10_conflicts
solver10_decisions
solver10_propagations
solver10_preprocess_time
solver10_search_time
category
reason_for_selection
residual_vars_after_preprocess
residual_clauses_after_preprocess
residual_lits_after_preprocess
proof_required
model_required
category_weight
holdout_bucket
benchmark_family
selection_version
selection_confidence
reference_solver_versions
```

`tools/compare_bench.py` inputs:

```text
--before log/<prior>/results.csv
--after log/<this>/results.csv
--baseline benchmarks/iteration/baseline.csv
--timeout <seconds>
```

Before comparison, `compare_bench.py` consumes validation summaries produced by `tools/validate_solver_result.py`. Timing wins from invalid results are ignored and reported as correctness failures.

Validation summary format:

```text
log/<run>/validation.jsonl
  one JSON object per instance:
    instance
    out_dir
    status
    expected_status
    model_check_result
    proof_check_result
    proof_checker
    proof_checker_exit_status
    output_contract_state
    validation_error
```

`compare_bench.py` must prefer validation.jsonl over revalidating outputs when it exists, but must warn if validation.jsonl is missing, incomplete, or stale relative to results.csv.

Outputs:

```text
PAR-2 before/after
PAR-2 proof-off before/after
PAR-2 proof-on before/after
solved-count before/after
status regressions
newly solved instances
new timeouts
per-category PAR-2
per-instance deltas
counter deltas when JSON_STATS exists
proof throughput deltas when SAT_PROOF=drat
paired speedup distribution
bootstrap confidence interval for PAR-2 delta
bootstrap confidence interval per category
seed-vs-binary variance estimate when multi-seed runs exist
promotion verdict: significant_improvement | significant_regression | indeterminate
machine/environment block
top 10 wins
top 10 regressions
instances requiring manual review
flaky instance warnings
calibration drift warnings
environment mismatch warnings
```

Acceptance for comparison tooling:

```text
- status regressions are highlighted before speedups
- SAT assignment/proof failures dominate the report
- missing JSON_STATS does not crash the tool
- output is stable and machine-readable enough for agents to paste into beads notes
- benchmark reports include CPU model, core count, governor if available, OS, rustc version, binary sha256, command line, timeout, memory limit, SAT_SEED, and config_hash
- profile promotion requires discriminating plus holdout, not discriminating alone
- proof-on promotion requires a separate proof-heavy UNSAT report
- proof-off promotion must not imply proof-on certification
- in SAT_PROOF=drat runs, every UNSAT result must record proof path, proof byte size, proof checker command, proof checker version, checker exit status, and proof_completeness=Complete
- compare_bench.py reports UNSAT proof missing, unchecked, checker-timeout, and checker-failed as correctness failures before timing deltas
- compare_bench.py separates solver UNKNOWN, harness external-timeout, malformed-output failure, and infrastructure failure
```

### Tests

```bash
find benchmarks/discriminating -name '*.cnf*' | wc -l
python3 tools/select_iter_bench.py --check-manifest benchmarks/discriminating/MANIFEST.csv
python3 tools/select_iter_bench.py --dry-run
python3 tools/validate_solver_result.py --self-test
python3 tools/compare_bench.py --self-test
```

### Acceptance

```text
Fixed discriminating set has at least 12 valid symlinks and preferably all 20.
Generated iteration sets are non-empty.
Every later task runs smoke-plus plus the relevant core set.
Comparison reports preserve correctness failures and status regressions ahead of timing deltas.
Quarantined flaky instances are still reported in stress runs but excluded from promotion evidence.
```

### Dependency

0.0, 0.3a, 0.4.

## 0.5a Profiling and hot-path observability

### Goal

Make performance regressions diagnosable before Phase 1 changes the search loop.

### Dependencies

0.4, 0.5.

### Implementation

Add:

```text
tools/profile_solver11.sh
tools/extract_hot_instances.py
log/profiles/README.md
```

`tools/profile_solver11.sh` should:

```text
- run one selected instance under the chosen SAT_PROFILE and SAT_CONFIG_REPLAY
- capture JSON_STATS
- capture wall time, user/sys time, max RSS, binary sha256, config_hash, SAT_SEED
- use `perf stat` when available
- use `perf record` or flamegraph tooling when available
- degrade cleanly when profiler tools are unavailable
- never change solver behavior or feature flags implicitly
```

Profiler output layout:

```text
log/profiles/<date>-<instance>-<config_hash>/
  command.txt
  env.txt
  stats.jsonl
  perf-stat.txt
  perf-record.txt or unavailable.txt
  notes.md
```

Hot-path task rule:

```text
Any task claiming propagation, watch-list, occurrence-list, proof-throughput, or allocation-speed improvement must attach either:
  - a profiler artifact, or
  - a written note explaining why counters alone were sufficient.
```

### Acceptance

```text
- profiler script works on at least one SAT and one UNSAT instance
- unavailable profiler tools do not fail CI
- config_hash and binary sha256 are recorded
- profiling artifacts are linked from milestone triage when used for performance decisions
```

### Dependency

0.4, 0.5.

## 0.0a Rich baseline comparison after benchmark tooling exists

### Goal

Upgrade the raw identity lock into a full paired benchmark report after 0.5 has created `tools/compare_bench.py` and `benchmarks/iteration/baseline.csv`.

### Dependencies

0.0, 0.5.

### Implementation

```bash
python3 tools/compare_bench.py \
  --before log/baseline-lock/solver10/results.csv \
  --after log/baseline-lock/solver11/results.csv \
  --baseline benchmarks/iteration/baseline.csv \
  --timeout 120 \
  > solver/11-kissat-search/BASELINE_LOCK.txt
```

### Acceptance

```text
- BASELINE_LOCK.txt and BASELINE_LOCK.raw.txt both exist
- rich comparison reports status regressions, solved count, PAR-2, per-category PAR-2, config hash if available, rustc version, binary sha256, and benchmark command lines
- any mismatch between raw and rich status counts blocks Phase 1
```

### Dependency

0.0, 0.5.

---

## 0.6 Brute-force, metamorphic, and differential oracle test harness

### Goal

Catch SAT model reconstruction bugs, simplification correctness bugs, order-dependence, parser edge cases, watcher corruption, and formula-rewrite bugs before they hit benchmarks.

### Implementation

Add brute-force utilities under:

```text
src/tests/bruteforce.rs
```

or inside `#[cfg(test)] mod tests`.

Generate all or sampled small CNFs within:

```text
n_vars <= 4
n_clauses <= 7
clause_len <= 3
```

For each formula:

1. Solve by brute force.
2. Solve with the solver.
3. If SAT, verify returned assignment satisfies the original, unmodified CNF.
4. If UNSAT, verify solver says UNSAT. Proof verification remains covered by smoke/bench scripts.

Run variants:

```text
SAT_SIMPLIFICATION=off
SAT_BVE=off
SAT_FULL_BSR=off
SAT_FULL_BSR=on
SAT_USE_LBD=on/off
SAT_BINARY_FAST=on/off
future: SAT_VIVIFY=on
future: SAT_PROBE=on
future: SAT_HBR=on
future after Appendix unpark: SAT_ELS=on
future after Appendix unpark: SAT_BCE=on
future: SAT_INPROCESS=on with SAT_BVE=on
```

Add metamorphic tests under:

```text
src/tests/metamorphic.rs
```

For each generated CNF, derive equivalent variants:

```text
- permute variable ids
- permute clause order
- permute literal order inside clauses
- duplicate random clauses
- duplicate random literals inside clauses
- add tautological clauses
- add subsumed clauses
- add blocked-looking but not blocked clauses
- randomly flip all literals through a consistent variable polarity map
```

Expected:

```text
- status is identical across variants
- if SAT, mapped assignment satisfies the original formula
- if UNSAT and proof enabled, drat-trim verifies where proof generation is expected
```

Add parser normalization differential tests:

```text
- parse CNF into Vec<Vec<i32>>
- write normalized DIMACS with sorted clauses, sorted literals, no duplicate literals, and a dense variable map when requested
- solve original and normalized CNFs under the same config
- require identical status
- if SAT, require both assignments satisfy their own original inputs and the normalized assignment maps back correctly
- if UNSAT with proof enabled, verify proof for each emitted input independently
```

Parser fuzz variants:

```text
- comments before and after p-line
- multiple spaces and tabs
- clauses split across many lines
- trailing zeros and duplicate zeros
- duplicate literals
- tautological clauses
- huge but in-range variable ids
- out-of-range variable ids
- missing terminal zero
- empty files
- compressed inputs supported by solver 10
```

Add randomized differential tests:

```text
n_vars <= 20
n_clauses <= 80
clause_len <= 5
fixed seed from SAT_SEED
budgeted to keep cargo test fast
```

When external reference solvers are present in the repo/toolchain, compare status against them.
When not present:

```text
- use brute force for very small formulas
- use a tiny independent DPLL oracle for sampled formulas up to the randomized differential limits
- use solver 10 only as a secondary regression reference, not as the correctness oracle
```

Add a deterministic shrinker:

```text
- remove clauses while preserving failure
- remove literals while preserving failure
- remap variables densely
- minimize enabled feature set that reproduces the failure
- print seed, minimized CNF, config hash, and failing transformation/edit index
```

Add a formula-edit replay harness placeholder:

```text
- define the independent Vec<Vec<i32>> replay model
- define the debug-log serialization format envelope
- add self-tests for replaying synthetic Add/Delete/Strengthen events
- do not require real FormulaEditTxn logs until task 2.1a/2.1b
```

### Acceptance

```text
Oracle passes before enabling any simplification feature by default.
Any simplification feature that changes formula satisfiability representation adds an oracle variant.
Every formula-rewrite feature adds at least one metamorphic test that exercises duplicate, tautology, and shuffled-clause cases.
Randomized tests print the failing seed and minimized formula when practical.
Randomized tests do not treat solver 10 agreement as sufficient correctness evidence except for pure baseline-regression checks.
Formula-edit replay harness self-tests pass in 0.6.
Full FormulaEditTxn replay against real solver edits is required by 2.1a/2.1b before any destructive simplification feature can be promoted.
Parser normalization differentials pass before benchmark tooling is used for profile promotion.
```

### Dependency

0.3, 0.3a.

## 0.7 Universal beads node template

Every beads task should follow this structure:

```text
Task ID:
Sort key:
Task class:
Goal:
Dependencies:
Scope:
Non-goals:
Code-level changes:
Touched files:
New public APIs:
Capability exceptions:
Proof/model obligations:
Unit tests:
Benchmark gate:
Counters to inspect:
Promotion/default rule:
Rejection note if reverted:
Risk IDs:
Feature flags touched:
Profile impact:
Validation artifact paths:
Run artifact directory:
  - log/tasks/<task-id>-<slug>/
Required artifacts:
  - command.txt
  - env.txt
  - config.json or config.replay
  - results.csv when benchmarks run
  - validation.jsonl when solver outputs are validated
  - compare.md when before/after comparison is required
```

This avoids half-implemented features with no test surface.

Allowed task classes:

```text
docs-only:
  - PLAN.md, README.md, FEATURES.md commentary only
  - gate: validate_solver11_plan.py

tooling-only:
  - benchmark/report/validator scripts with no solver behavior change
  - gate: script self-tests plus affected smoke/bench parser tests

config-contract:
  - SolverConfig, CONFIG_SCHEMA.csv, FEATURES.csv, replay parsing
  - gate: cargo test, config validation tests, smoke tests

solver-behavior:
  - search, propagation, proof, model, parser, inprocessing, formula edits
  - gate: full correctness gate plus relevant benchmark gate

performance-claim:
  - any task claiming speed, allocation, proof-throughput, or propagation improvement
  - gate: solver-behavior gate plus compare_bench.py report and profiler artifact or counter justification
```

Task ordering rule:

```text
- dependencies, not lexical task IDs, define execution order
- suffix IDs such as 0.0a, 1.0a, 2.1a, and 2.16a are valid
- gaps such as 2.13 -> 2.16 are valid only when the DAG summary explains the reserved/parked tasks
- validate_solver11_plan.py must reject a task whose dependencies are not represented in the DAG summary
- validate_solver11_plan.py must not sort task IDs lexically to infer execution order
- validate_solver11_plan.py rejects a task whose declared class does not match touched files and required gates
```

---

## 0.8 Parser, output, proof-temp, and resource-limit contract

### Goal

Make the external solver behavior precise and stable before adding search and inprocessing complexity.

### Implementation

Document and test:

```text
Input:
  - accepts plain `.cnf` and any compression formats already supported by solver 10
  - rejects malformed DIMACS with a clear diagnostic
  - tolerates comments, blank lines, trailing whitespace, and split clauses
  - rejects variable ids beyond declared bounds unless solver 10 intentionally accepts them

Output:
  - writes a status file or stdout/stderr status exactly as solver 10 currently expects
  - writes out_dir/result.json with the structured result contract after 0.3a
  - SAT output contains an assignment for the original input variables
  - UNSAT output keeps only a completed proof
  - UNKNOWN output never leaves a proof that looks complete

Limits:
  - support internal conflict, propagation, tick, and wall-clock soft limits
  - support RSS, learned-literal, binary-clause, extension-bytes, and proof-bytes hard limits
  - return UNKNOWN cleanly when a solve-ending internal limit fires
  - abort optional inprocessing passes rather than the whole solve when a pass budget is exhausted and correctness allows
  - flush stats on SAT/UNSAT/UNKNOWN

Budget classes:
  - `SolveLimit`: ends the solve with UNKNOWN if exceeded before SAT/UNSAT
  - `PassBudget`: aborts the current optional pass and resumes search
  - `EditBudget`: aborts the current FormulaEditTxn before mutation
  - `EmergencyMemoryLimit`: may force UNKNOWN if continuing would risk corrupt output or process failure

Proof temp lifecycle:
  - write proof to a temporary path first
  - atomically rename only after UNSAT proof completion
  - delete proof temp on SAT, UNKNOWN, panic-cleanup path where possible, or parse failure
```

Add a typed result/output contract:

```rust
enum SolveStatus {
    Sat,
    Unsat,
    Unknown,
    ParseError,
}

enum ProofCompleteness {
    NotRequested,
    Complete,
    Incomplete,
}

struct OutputContract {
    status: SolveStatus,
    proof_completeness: ProofCompleteness,
    model_written: bool,
    proof_written: bool,
    stats_written: bool,
    result_json_written: bool,
    output_contract_state: OutputContractState,
}
```

Add fixed golden contract tests:

```text
solver/11-kissat-search/testdata/golden/
  - sat_tiny.cnf
  - unsat_empty_clause.cnf
  - empty_formula.cnf
  - malformed_missing_zero.cnf
  - malformed_var_out_of_bounds.cnf
  - split_clause.cnf
  - tautology.cnf
```

Each golden case records:

```text
expected_status
expected_exit_code
expected_status_file
expected_model_check
expected_proof_presence
expected_stats_required_fields
```

Golden test rules:

```text
- Golden tests compare normalized output, not raw wall-clock fields.
- Proof contents are not snapshot-compared, but proof existence, completeness state, finalization, and drat-trim verification are checked where applicable.
- README example commands must be backed by one golden or smoke test.
```

Allowed combinations:

```text
SAT:
  - model_written=true
  - proof_written=false
  - proof_completeness=NotRequested or Incomplete

UNSAT with SAT_PROOF=drat:
  - model_written=false
  - proof_written=true
  - proof_completeness=Complete

UNSAT with SAT_PROOF=off:
  - model_written=false
  - proof_written=false
  - proof_completeness=NotRequested

UNKNOWN:
  - model_written=false
  - proof_written=false
  - proof_completeness=NotRequested or Incomplete

ParseError:
  - model_written=false
  - proof_written=false
  - stats_written=true if stats were requested
```

Rules:

```text
- benchmark tooling rejects UNSAT + SAT_PROOF=drat unless proof_completeness=Complete
- benchmark tooling rejects SAT unless the original-CNF model checker passed
- tools/validate_solver_result.py is the shared correctness validator for smoke tests, golden tests, benchmark tooling, CI, and manual reproductions
- result.json is the preferred source for output-contract parsing; legacy status files remain compatibility input
- a finalized proof path is created only through OutputContract finalization
- proof temp cleanup is best-effort on panic, but normal SAT/UNKNOWN/ParseError paths must remove temp proof deterministically
```

Add config:

```rust
conflict_limit: Option<u64>,
propagation_limit: Option<u64>,
tick_limit: Option<u64>,
wall_limit_sec: Option<f64>,
rss_limit_mb: Option<u64>,
learned_lit_limit: Option<u64>,
binary_clause_limit: Option<u64>,
extension_bytes_limit: Option<u64>,
proof_bytes_limit: Option<u64>,
```

Environment variables:

```text
SAT_LIMIT_CONFLICTS=<u64>
SAT_LIMIT_PROPAGATIONS=<u64>
SAT_LIMIT_TICKS=<u64>
SAT_LIMIT_WALL_SEC=<float>
SAT_LIMIT_RSS_MB=<u64>
SAT_LIMIT_LEARNED_LITS=<u64>
SAT_LIMIT_BINARY_CLAUSES=<u64>
SAT_LIMIT_EXTENSION_BYTES=<u64>
SAT_LIMIT_PROOF_BYTES=<u64>
```

### Tests

```text
test_golden_sat_tiny_output_contract
test_golden_unsat_proof_contract
test_golden_unknown_limit_contract
test_golden_parse_error_contract
test_empty_formula_sat
test_empty_clause_unsat
test_tautological_clause_parse
test_split_clause_parse
test_malformed_dimacs_rejected
test_unknown_limit_flushes_stats
test_sat_deletes_temp_proof
test_unsat_renames_completed_proof
test_output_contract_rejects_unsat_with_incomplete_proof
test_output_contract_rejects_sat_without_model_check
test_output_contract_unknown_never_finalizes_proof
test_profile_baseline_matches_solver10_feature_defaults
test_profile_search_conservative_enables_only_documented_features
test_profile_inprocess_conservative_enables_only_documented_features
test_readme_profile_examples_have_matching_config_hashes
```

### Acceptance

```text
- no malformed proof file is left behind after SAT or UNKNOWN
- all parse failures are deterministic and non-panicking
- original run.sh contract remains intact
- every README profile example has a corresponding config replay smoke test
- baseline profile remains solver-10-equivalent until explicit baseline promotion
```

### Dependency

0.0, 0.3, 0.3a, 0.4.

## 0.9 Local CI and feature-interaction matrix

### Goal

Give coding agents one authoritative fast gate and one explicit feature-combination matrix.

### Implementation

Add:

```bash
tools/ci_solver11_fast.sh
tools/ci_solver11_matrix.sh
tools/ci_solver11_proof_model.sh
tools/validate_solver11_plan.py
```

`tools/ci_solver11_fast.sh` must run:

```bash
cd solver/11-kissat-search
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
python3 ../../tools/validate_solver11_plan.py ../../solver/11-kissat-search/PLAN.md

cd ../..
python3 -m py_compile tools/status_compare.py tools/validate_solver_result.py tools/select_iter_bench.py tools/compare_bench.py  # legacy Python compile-check; remove after sat-bench cutover
cargo run -p sat-bench --release -- validate-plan solver/11-kissat-search/PLAN.md
if command -v shellcheck >/dev/null 2>&1; then
  shellcheck tools/*.sh solver/11-kissat-search/*.sh
fi
bash tools/smoke_test.sh solver/11-kissat-search
SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-search
bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/smoke-plus solver/11-kissat-search
bash tools/ci_reproducibility.sh solver/11-kissat-search
```

`tools/ci_reproducibility.sh` must verify, for at least 5 instances (one each: trivial SAT, trivial UNSAT, medium SAT, medium UNSAT, proof-on UNSAT):

```text
- Two runs of the same binary with SAT_SEED=0 and the same SAT_CONFIG_REPLAY produce:
    * identical status
    * identical conflicts, decisions, propagations, restarts in JSON_STATS
    * identical search trace under SAT_TRACE_FULL=on (modulo non-deterministic fields like wall time)
    * bitwise-identical proof file (DRAT or LRAT) when SAT_PROOF is on
- A run with SAT_SEED=0 and another with SAT_SEED=1 produce identical status but at least one differing counter (sanity check that the seed is actually used).
- The reproducibility check is skipped if the build is debug; release-mode binaries are required.
- Any reproducibility failure blocks merge — there is no profile or maturity workaround.
- The gate is active from 0.0b onward.
```

Allowed sources of nondeterminism (must be documented explicitly):

```text
- wall_time, parse_time, search_time, max_rss_mb in JSON_STATS (excluded from byte-comparison)
- binary_sha256, config_hash (not nondeterminism; recorded once)
- nothing else
```

`tools/ci_solver11_matrix.sh` must run a bounded config matrix:

```text
baseline
SAT_USE_LBD=on SAT_REDUCE=legacy SAT_RESTART=legacy-luby
SAT_USE_LBD=on SAT_REDUCE=lbd-tiered SAT_RESTART=legacy-luby
SAT_USE_LBD=on SAT_REDUCE=lbd-tiered SAT_RESTART=kissat-ema
SAT_BINARY_FAST=on
SAT_INPROCESS=on SAT_VIVIFY=off SAT_PROBE=off SAT_HBR=off
```

`tools/ci_solver11_proof_model.sh` must run:

```text
baseline proof-on UNSAT smoke
default proof-on UNSAT smoke (search=Validated, preprocess=Conservative)
default proof-on UNSAT smoke with SAT_SEARCH_AXIS=strong (axis override)
fast proof-on UNSAT smoke (search=Strong, preprocess=GateAware)
fast proof-on UNSAT smoke with SAT_PREPROCESS_AXIS=conservative (axis override)
SAT model output check on SAT-heavy smoke-plus
SAT model extension replay check on every enabled destructive simplification variant
generated small UNSAT proof fuzz with drat-trim
generated small SAT model fuzz with original-CNF model verification
UNKNOWN limit path with proof temp cleanup
proof-checker unavailable path is reported as infrastructure failure, not proof success
LRAT proof-on smoke for every profile that produces UNSAT
LRAT vs DRAT cross-check on the same UNSAT instance: same status, both verify
LRAT proof-checker throughput recorded in JSON_STATS as proof_check_sec
```

`tools/validate_solver11_plan.py` must check:

```text
- missing or duplicate task IDs
- unknown dependencies
- task IDs missing from the DAG summary
- DAG edges pointing to missing tasks
- accidental lexical-order assumptions for suffix task IDs
- parking-lot features appearing in main profile candidates (denylist check)
- active SAT_* variables mentioned in PLAN.md or README.md but absent from the codegen-derived CONFIG_SCHEMA.csv (cross-reference only; schema authority is the SolverConfig struct)
- README profile examples whose effective config does not match generated CONFIG_SCHEMA.csv defaults plus explicit overrides
- task classes inconsistent with touched files and required gates
- risk-area tasks without risk IDs
- the codegen-derived CONFIG_SCHEMA.csv and FEATURES.csv files are checked-in and current (CI runs `cargo run -p solver11-codegen` and diffs)
```

Schema-vs-struct, feature-vs-CSV, and promoted-profile-vs-validation-artifact checks are now enforced at `cargo build` time by the codegen macros (see 0.3), not by this validator.

### Acceptance

```text
- every beads node says whether ci_solver11_fast and/or ci_solver11_matrix was run
- Python tooling compiles under python3
- shellcheck warnings are either fixed or recorded as intentional when shellcheck is available
- every beads node touching proof, model, formula edits, binary clauses, or inprocessing says whether ci_solver11_proof_model was run
- matrix failures are reported before benchmark speedups
- matrix output includes config_hash for every row
- validate_solver11_plan.py catches missing task IDs, duplicate task IDs, unknown dependencies, DAG/lexical-order errors, parking-lot features appearing in main profile candidates, SAT_* variables mentioned in PLAN.md/README.md but absent from the codegen-derived CONFIG_SCHEMA.csv, README examples drifting from generated defaults, codegen-derived CSVs not refreshed, task-class/gate mismatches, and risk-area tasks without risk IDs (schema/feature/profile-validation checks are enforced at cargo build by codegen macros)
```

### Dependency

0.0, 0.3, 0.3a, 0.4, 0.5, 0.6, 0.8.

---

# 4. Phase 1 — Search, decision, phase, restart, and learned-clause logic

Phase 1 comes first because many observed gaps are search-core gaps. Preprocessing alone will not close them.

The safe priority order is:

```text
1. reason/propagation scaffold for binary IDs, binary reasons, and conflict representation
2. temporary-assumption context shared with later vivification, probing, RCheck, and diagnostics
3. decision heap cleanup and decision eligibility hardening
4. LBD metadata
5. LBD-tiered learned-clause reduction
6. EMA restart
7. saved/target/best phase selection
8. binary implication fast path
9. core search candidate benchmark
10. focused/stable mode + reluctant restarts
11. VMTF queue
12. clause minimization and reason-side bumping
13. rephasing hook
14. advanced search candidate benchmark
15. guarded chronological backtracking
16. throughput pass
```

Task numbering remains stable for downstream references. Implement 1.0a immediately after the 1.0 scaffold, and implement the heap-cleanup work in 1.7 immediately after that if it can be done without depending on phase-selection internals.

Do not start with chronological backtracking, BCE, ELS, or large inprocessing. They are valuable but easy to get subtly wrong.

---

## 1.0 Reason and propagation scaffold

### Goal

Create the shared reason/conflict representation needed by LBD, binary fast path, HBR, transitive reduction, and proof/model debug paths.

### Dependencies

0.1, 0.3, 0.6.

### Code-level changes

Add:

```rust
#[derive(Copy, Clone)]
struct BinaryClauseId(u32);

enum ReasonRef {
    None,
    Clause(ClauseRef),
    Binary(BinaryClauseId),
}

enum Conflict {
    Clause(ClauseRef),
    Binary(BinaryClauseId),
    RootUnit,
}
```

Rules:

```text
- conflict analysis expands reasons through one helper
- all solver code reads and writes reasons through `ReasonRef` or a single `ReasonCode` newtype; do not mix raw `usize`, `ClauseRef`, and tagged binary IDs at call sites
- any compact tagged encoding is private to `trail.rs`
- GC and binary-clause deletion rewrite reasons through one `rewrite_reason_ref` helper
- proof/model/debug code can recover the actual reason clause literals
- binary fast path may remain disabled; this task only creates representation
- no search trajectory change is expected
```

### Tests

```text
test_reason_none_for_decision
test_reason_clause_expands_lits
test_reason_binary_expands_lits
test_conflict_binary_expands_lits
test_reason_code_roundtrip_clause
test_reason_code_roundtrip_binary
test_reason_code_rejects_invalid_tag_or_overflow
test_gc_rewrites_reason_ref
test_legacy_reason_path_unchanged_when_binary_fast_off
```

### Benchmark gate

Same as LBD metadata: search-core should be nearly identical except for representation overhead.

---

## 1.0a Temporary-assumption context

### Goal

Centralize temporary root-level assumptions so vivification, probing, HBR, RCheck, BCE diagnostics, and optional sweep cannot accidentally mutate normal search state.

### Dependencies

1.0, 0.3, 0.6.

### Code-level changes

Add:

```rust
enum SearchAccountingMode {
    NormalSearch,
    TemporaryAssumption {
        update_phase: bool,
        update_branch_stats: bool,
        update_restart_stats: bool,
        count_as_decision: bool,
    },
}

struct TemporaryAssumptionOptions {
    update_phase: bool,
    update_branch_stats: bool,
    update_restart_stats: bool,
    count_as_decision: bool,
}

struct TemporaryAssumptionGuard {
    start_trail: usize,
    start_level: usize,
    saved_accounting_mode: SearchAccountingMode,
}

struct TemporaryAssumptionCtx<'a> { /* private */ }
```

API:

```rust
fn with_temporary_assumptions<R>(
    &mut self,
    opts: TemporaryAssumptionOptions,
    f: impl FnOnce(&mut TemporaryAssumptionCtx<'_>) -> R,
) -> R;

impl<'a> TemporaryAssumptionCtx<'a> {
    fn enqueue(&mut self, lit: i32) -> EnqueueResult;
    fn propagate_budgeted(&mut self, budget: &mut Budget) -> Option<Conflict>;
}
```

Low-level `begin_temporary_assumptions` / `end_temporary_assumptions` may exist only as private trail/search internals. Pass code must use the closure API so early returns cannot leak temporary state.

Rules:

```text
- temporary assumptions always start and end at root level
- temporary propagation uses separate counters
- temporary assignments never update saved/target/best phase unless options.update_phase=true
- temporary assignments never update VMTF, heap activity, restart averages, normal decision counters, or best/target prefix tracking unless the corresponding option explicitly permits it
- closure exit restores trail, level, accounting mode, and propagate_head even on early return
- debug mode asserts restoration after every closure invocation
```

### Tests

```text
test_temp_assumption_guard_restores_root
test_temp_assumption_does_not_update_saved_phase
test_temp_assumption_does_not_update_target_or_best_phase
test_temp_assumption_does_not_bump_vmtf_or_heap_stats
test_temp_assumption_does_not_update_restart_ema
test_temp_assumption_conflict_restores_propagate_head
test_temp_assumption_closure_restores_on_early_return
```

### Benchmark gate

No speed requirement. Search-core with no temporary-assumption features enabled should be identical except for zero counters.

---

## 1.1 Learned-clause LBD / glue metadata

### Goal

Compute and store true LBD/glue for every learned clause and every conflict without changing behavior when disabled.

### Dependencies

1.0, 0.3, 0.4.

### Code-level changes

Add fields:

```rust
lbd_seen: Vec<u32>,
lbd_stamp: u32,
last_conflict_lbd: u16,
sum_lbd: u64,
num_lbd: u64,
lbd_hist_1: u64,
lbd_hist_2: u64,
lbd_hist_3_5: u64,
lbd_hist_6_10: u64,
lbd_hist_gt_10: u64,
```

Helper:

```rust
#[inline]
fn compute_lbd_from_lits(&mut self, lits: &[i32]) -> u16 {
    self.lbd_stamp = self.lbd_stamp.wrapping_add(1);
    if self.lbd_stamp == 0 {
        self.lbd_seen.fill(0);
        self.lbd_stamp = 1;
    }

    let mut count: u32 = 0;
    for &lit in lits {
        let v = var(lit);
        let lvl = self.decision_level[v] as usize;
        if self.lbd_seen[lvl] != self.lbd_stamp {
            self.lbd_seen[lvl] = self.lbd_stamp;
            count += 1;
        }
    }
    count.min(u16::MAX as u32) as u16
}
```

Size `lbd_seen` by max decision level if cheap; otherwise `n_vars + 1` is acceptable initially.

Initial learned-clause metadata layout:

Use a side table first, keyed by a stable `LearnedId`, not by arena offset:

```rust
struct LearnedId(u32);

struct LearnedMeta {
    lbd: u16,
    tier: u8,
    used_recently: u8,
    removable: bool,
    vivified: bool,
}

learned_meta: Vec<LearnedMeta>;
learned_clause_ids: Vec<LearnedId>;
```

The clause arena may store `LearnedId` if there is already a safe spare metadata slot. Do not change arena word layout in the first LBD task unless the source audit proves it is harmless.

Optional later arena packing, only after measurement:

```rust
const LEARNT_EXTRA_ACTIVITY_WORDS: usize = 2;
const LEARNT_EXTRA_META_WORDS: usize = 1;
```

Pack the optional meta word:

```text
bits 0..15   glue / lbd
bits 16..23  tier
bit 24       used_recently flag/counter low bit or used flag
bit 25       removable flag
bit 26       vivified flag
bits 27..31  reserved
```

If `used_recently` needs values 0..3, allocate two bits or store it in a side byte/side table. Do not silently truncate it.

Helpers:

```rust
fn learned_id_for_clause(clause_idx: usize) -> LearnedId;
fn set_learnt_lbd(clause_idx: usize, lbd: u16);
fn learnt_lbd(clause_idx: usize) -> u16;
fn set_learnt_tier(clause_idx: usize, tier: u8);
fn learnt_used_recently(clause_idx: usize) -> u8;
fn set_learnt_used_recently(clause_idx: usize, value: u8);
```

Do not use a `HashMap` keyed by arena offset. It will be too slow and will add noisy memory effects. Do not require an arena-layout migration merely to compute LBD.

Important correction:

```text
Do not hard-code all binary learned clauses to glue=1.
A binary clause spanning two decision levels has true LBD=2.
Binary clauses should be protected during reduction regardless of true LBD.
```

### Tests

```text
test_lbd_single_level_clause_is_1
test_lbd_binary_across_two_levels_is_2
test_lbd_ignores_duplicate_decision_levels
test_lbd_stamp_wrap_clears_seen
test_lbd_stored_and_read_from_learned_clause
test_lbd_metadata_does_not_touch_original_clause_layout
```

### Benchmark gate

```bash
SAT_USE_LBD=on SAT_REDUCE=legacy SAT_RESTART=legacy-luby \
  bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/search-core solver/11-kissat-search
```

Expected: nearly identical search path except for small memory/layout effects. Heavy regression means metadata placement is too invasive.

---

## 1.2 LBD-aware conflict analysis updates

### Goal

Make learned-clause quality available to restart and reduction policies.

### Dependencies

1.1.

### Code-level changes

During conflict analysis, after producing the asserting clause:

1. Compute LBD.
2. Store LBD.
3. Update LBD stats.
4. Optionally improve LBD of learned reason clauses participating in analysis.

```rust
fn maybe_improve_lbd(&mut self, clause_idx: usize, new_lbd: u16) {
    if !self.clause_is_learnt(clause_idx) { return; }
    let old = self.learnt_lbd(clause_idx);
    if new_lbd < old {
        self.set_learnt_lbd(clause_idx, new_lbd);
        self.stats.lbd_improved += 1;
    }
}
```

Gate reason updates behind:

```bash
SAT_LBD_UPDATE_REASONS=on/off
```

Default off until LBD-tiered reduction is stable.

### Tests

```text
test_lbd_improvement_only_lowers
test_original_clause_lbd_not_touched
test_reason_clause_lbd_update_preserves_activity
test_analyze_stores_last_conflict_lbd
```

### Benchmark gate

Run search-core with reason updates both off and on. Expect low/no impact before reduction/restart uses LBD.

---

## 1.3 LBD-tiered learned-clause reduction

### Goal

Replace activity-only learned-clause deletion with a glue/LBD-tiered policy while preserving binary and locked clauses.

### Dependencies

1.1, 1.2.

### Code-level changes

Constants:

```rust
const TIER1_MAX_GLUE: u16 = 2;
const TIER2_MAX_GLUE: u16 = 6;
const MAX_USED_RECENTLY: u8 = 3;
```

Tier classification:

```rust
fn classify_learnt_clause(&mut self, clause_idx: usize) {
    let lbd = self.learnt_lbd(clause_idx);
    let tier = if lbd <= 2 { 0 } else if lbd <= 6 { 1 } else { 2 };
    self.set_learnt_tier(clause_idx, tier);
}
```

Clause lifecycle:

```text
On learned:
  - compute LBD
  - classify tier
  - initialize used_recently = MAX_USED_RECENTLY for every tier, matching Kissat's
    maximum initial learned-clause retention before reduce-DB aging
  - initialize activity using current conflict activity scheme

On clause used as propagation reason:
  - if learned, set used_recently = max(used_recently, 1)
  - optionally improve LBD only when SAT_LBD_UPDATE_REASONS=on
  - reclassify if LBD improved

On reduction:
  - never delete locked clauses
  - decrement used_recently for protected non-locked tier2/tier3 clauses
  - collect tier3 first
  - collect tier2 only if unused and above learned budget
  - protect tier1 under normal memory pressure
  - under emergency memory pressure, tier1 clauses may be demoted only if:
      * not binary
      * not unit
      * not locked
      * not used_recently
      * older than a configured conflict age
      * learned_lits exceeds the hard learned-lit budget

On GC:
  - preserve learned clause meta
  - rewrite all ClauseRef/ReasonRef/BinaryClauseId-dependent structures
```

Never delete:

```text
- unit clauses
- binary clauses
- locked reason clauses
- deleted clauses
- low-LBD clauses protected by recent use policy, except explicit emergency demotion above
```

Reason lock API:

```rust
struct ReasonPinSet {
    pinned_clauses: Vec<ClauseRef>,
    pinned_binaries: Vec<BinaryClauseId>,
    generation: u64,
}

fn rebuild_reason_pinset(&mut self) -> ReasonPinSet;
fn clause_is_reason_pinned(&self, pins: &ReasonPinSet, cref: ClauseRef) -> bool;
fn binary_is_reason_pinned(&self, pins: &ReasonPinSet, bid: BinaryClauseId) -> bool;
```

Rules:

```text
- reduce_db builds one ReasonPinSet before candidate collection.
- GC builds one ReasonPinSet before moving or deleting clause storage.
- FormulaEditTxn preflight rejects deletion of any pinned reason unless the plan supplies a replacement reason or root-level reassignment plan.
- invariant mode checks that every non-decision propagated assignment's reason appears in the pinset.
```

Candidate ordering should use integer keys, not float comparisons:

```rust
struct ReduceCand {
    clause_idx: usize,
    lbd: u16,
    size: usize,
    used_recently: u8,
    activity_rank: u32,
}
```

Worse clauses first:

```text
higher LBD
larger size
not used recently
lower activity / older age
```

Use a comparator first. If sorting becomes hot, replace with bucketing by tier and size.

Reduction schedule:

```text
base = 2_000
factor = 300
next_limit = base + factor * sqrt(reductions)
```

This is tunable; do not treat it as sacred.

Add a learned-budget target in addition to the time schedule:

```text
reduce when conflicts >= next_reduce_conflict OR learned_lits > learned_lit_budget
target deletion is budget-driven, not a fixed percentage:
  - if learned_lits <= budget: skip
  - else delete worst candidates until under budget or candidates exhausted
budget grows with conflicts and shrinks only through explicit experiments
hard budget triggers emergency demotion before process memory explodes
```

### Tests

```text
test_reason_pinset_contains_all_clause_reasons
test_reason_pinset_contains_all_binary_reasons
test_reduce_db_consults_reason_pinset
test_gc_preserves_reason_pinned_clauses
test_reduce_never_deletes_binary
test_reduce_never_deletes_unit
test_reduce_never_deletes_reason_clause
test_reduce_db_protects_glue_one_clauses
test_reduce_db_protects_tier2_with_used_recently
test_reduce_db_drops_high_glue_unused_large_clauses_first
test_reduce_updates_live_learned_counts
test_reduce_deleted_watchers_are_skipped_by_propagation
test_reason_use_marks_learned_clause_recent
test_lbd_improvement_reclassifies_clause_tier
test_reduce_respects_learned_lit_budget
test_reduce_emergency_can_demote_old_unused_tier1
test_reduce_emergency_never_deletes_locked_binary_or_unit
test_gc_preserves_learned_meta_after_reduction
```

### Benchmark gate

Run:

```bash
SAT_USE_LBD=on SAT_REDUCE=lbd-tiered SAT_RESTART=legacy-luby \
  bash tools/bench.sh -t 300 -m 16384 \
  -d benchmarks/discriminating solver/11-kissat-search
```

Track:

```text
learned_kept_tier1
learned_kept_tier2
learned_kept_tier3
learned_collected
avg_lbd
learned_lits_final
propagations/sec
```


---

## 1.3a Clause database budget and GC policy

### Goal

Make learned-clause deletion, arena garbage, watcher staleness, and GC scheduling measurable before binary fast path and search-throughput tuning.

### Dependencies

1.3, 0.4, 0.5a.

### Code-level changes

Add counters:

```text
arena_words_live
arena_words_garbage
arena_garbage_ratio
learned_words_live
original_words_live
watchers_live
watchers_stale
gc_reason
gc_words_reclaimed
gc_refs_rewritten
```

GC scheduling:

```text
- trigger GC when arena_garbage_ratio exceeds threshold after reduce_db
- trigger GC when stale watcher count exceeds threshold and propagation counters show cost
- never GC while decision level > 0 unless a task explicitly proves reason/trail safety
- GC must rebuild or rewrite every registered ClauseRef/BinaryClauseId-dependent structure
- GC report records reason: learned-reduction, arena-fragmentation, watcher-staleness, emergency-memory
```

### Tests

```text
test_gc_not_run_above_root_level
test_gc_reclaims_deleted_learned_words
test_gc_rewrites_all_registered_refs
test_gc_preserves_original_clause_model_check_refs
test_gc_reason_recorded_in_stats
```

### Benchmark gate

Run search-core and preprocess-core. Accept if memory/stale-watch counters improve without status regressions; no wall-clock win is required.

---

## 1.4 EMA restart policy

### Goal

Replace purely Luby conflict limits with an LBD EMA restart option. Keep legacy policies for A/B testing.

### Dependencies

1.1.

### Code-level changes

```rust
struct MovingAverage {
    value: f64,
    initialized: bool,
    alpha: f64,
}

impl MovingAverage {
    fn update(&mut self, x: f64) {
        if !self.initialized {
            self.value = x;
            self.initialized = true;
        } else {
            self.value += self.alpha * (x - self.value);
        }
    }
}
```

Fields:

```rust
restart_fast_lbd: MovingAverage, // alpha 1/32
restart_slow_lbd: MovingAverage, // alpha 1/4096
restart_fast_level: MovingAverage,
restart_slow_level: MovingAverage,
restart_min_conflicts: u64,       // start around 50
restart_next_check_conflict: u64,
restart_margin: f64,             // start around 1.20; tune 1.10..1.40
restart_conflicts_since_last: u64,
```

Restart condition:

```rust
fn should_restart(&self) -> bool {
    if self.current_decision_level() == 0 { return false; }
    if self.restart_conflicts_since_last < self.restart_min_conflicts { return false; }

    match self.config.restart_policy {
        RestartPolicy::LegacyLuby => self.legacy_luby_restart_due(),
        RestartPolicy::KissatEma => {
            self.restart_fast_lbd.initialized
                && self.restart_slow_lbd.initialized
                && self.restart_fast_lbd.value > self.restart_slow_lbd.value * self.restart_margin
        }
        RestartPolicy::Reluctant => self.reluctant_restart_due(),
    }
}
```

First implementation backtracks to level 0. Trail reuse is a separate experiment.

### Tests

```text
test_no_restart_at_level_zero
test_lbd_ema_fast_reacts_faster_than_slow
test_restart_triggers_when_fast_exceeds_slow_by_margin
test_restart_blocked_during_min_interval
test_restart_policy_legacy_unchanged_when_selected
test_restart_backtracks_and_preserves_root_units
```

### Benchmark gate

Run median of 3 on search-core:

```bash
SAT_USE_LBD=on SAT_RESTART=kissat-ema SAT_REDUCE=lbd-tiered \
  bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/search-core solver/11-kissat-search
```

Inspect:

```text
conflicts
decisions
propagations
restarts
avg_lbd
avg_decision_level
time
```

---

## 1.5 Saved, target, and best phase selection

### Goal

Improve SAT search trajectory using saved phase, target phase, and best phase.

### Dependencies

1.1, 0.3.

### Code-level changes

Use existing `saved_phase`. Add:

```rust
target_phase: Vec<u8>,
best_phase: Vec<u8>,
original_phase: Vec<u8>,
target_assigned: usize,
best_assigned: usize,
phase_ticks: u64,
```

Encoding:

```rust
const VALUE_UNASSIGNED: u8 = 0;
const VALUE_FALSE: u8 = 1;
const VALUE_TRUE: u8 = 2;
```

Selection:

```rust
#[inline]
fn pick_branch_phase(&mut self, var: usize) -> bool {
    match self.config.phase_policy {
        PhasePolicy::Legacy => self.saved_or_default(var),
        PhasePolicy::Saved => self.saved_phase[var] == VALUE_TRUE,
        PhasePolicy::TargetThenSaved => {
            if self.target_phase[var] != VALUE_UNASSIGNED {
                self.stats.phase_target_used += 1;
                return self.target_phase[var] == VALUE_TRUE;
            }
            if self.saved_phase[var] != VALUE_UNASSIGNED {
                self.stats.phase_saved_used += 1;
                return self.saved_phase[var] == VALUE_TRUE;
            }
            self.stats.phase_initial_used += 1;
            false
        }
        PhasePolicy::BestThenTargetThenSaved => {
            if self.best_phase[var] != VALUE_UNASSIGNED {
                self.stats.phase_best_used += 1;
                return self.best_phase[var] == VALUE_TRUE;
            }
            if self.target_phase[var] != VALUE_UNASSIGNED {
                self.stats.phase_target_used += 1;
                return self.target_phase[var] == VALUE_TRUE;
            }
            if self.saved_phase[var] != VALUE_UNASSIGNED {
                self.stats.phase_saved_used += 1;
                return self.saved_phase[var] == VALUE_TRUE;
            }
            self.stats.phase_initial_used += 1;
            false
        }
    }
}
```

Update saved phase on assignment. Update target/best phase only when a new deepest unconflicted prefix is reached, not on every assignment.

On restart:

```text
- reset target_assigned for per-restart target phase
- keep best_assigned for full-solve best phase
```

### Tests

```text
test_phase_falls_back_to_initial
test_saved_phase_used_when_no_target
test_target_phase_precedes_saved
test_best_phase_precedes_target_when_policy_selected
test_target_phase_captured_at_new_deep_prefix
test_target_phase_reset_on_restart
test_best_phase_only_grows_monotonically
test_phase_saving_survives_backtrack
```

### Benchmark gate

Run:

```bash
SAT_PHASE=saved
SAT_PHASE=target-then-saved
SAT_PHASE=best-then-target-then-saved
```

Track:

```text
phase_target_used
phase_best_used
phase_saved_used
conflicts
decisions
propagations
```

Promote only if search-core improves or if it enables later focused/stable gains.

---

## 1.6 Binary implication fast path

### Goal

Make binary propagation cheaper and reduce long-clause watch traffic without losing proof/model/debug traceability.

### Dependencies

1.0, 1.1, 0.6.

### Code-level changes

Add:

```rust
// BinaryClauseId is introduced in 1.0.
struct BinaryClause {
    a: i32,
    b: i32,
    redundant: bool,
    deleted: bool,
    proof_logged: bool,
    origin: BinaryOrigin,
    used_count: u32,
    last_used_conflict: u64,
}

enum BinaryOrigin {
    Original,
    LearnedConflict,
    Hbr,
    Transitive,
    Gate,
}

#[derive(Copy, Clone)]
struct BinaryEdge {
    implied: i32,
    clause_id: BinaryClauseId,
}

binary_clauses: Vec<BinaryClause>,
binary_implications: BinaryImplications, // indexed by assigned-true antecedent literal
binary_dedup_seen: Vec<u32>,
binary_dedup_stamp: u32,
```

Storage abstraction:

```rust
enum BinaryImplications {
    Nested(Vec<Vec<BinaryEdge>>),
    Flat {
        edges: Vec<BinaryEdge>,
        offsets: Vec<u32>,
        dirty: bool,
    },
}

impl BinaryImplications {
    fn edges_for(&self, lit: i32) -> &[BinaryEdge];
    fn add_edge(&mut self, antecedent: i32, edge: BinaryEdge);
    fn mark_deleted(&mut self, id: BinaryClauseId);
    fn rebuild_flat_if_needed(&mut self);
}
```

For binary clause `(a ∨ b)`:

```text
binary implication ¬a -> b
binary implication ¬b -> a
```

Propagation:

```rust
fn propagate_lit(&mut self, lit: i32) -> Option<Conflict> {
    for edge in self.binary_implications.edges_for(lit).iter().copied() {
        if self.binary_clauses[edge.clause_id.0 as usize].deleted {
            self.stats.binary_stale_skips += 1;
            continue;
        }
        match self.value(edge.implied) {
            TRUE => {}
            FALSE => return Some(Conflict::Binary(edge.clause_id)),
            UNASSIGNED => self.assign(edge.implied, ReasonCode::from_binary(edge.clause_id)),
        }
    }

    let long_watch_idx = lit_index(neg(lit));
    self.propagate_long_watch_list(long_watch_idx)
}
```

Reason encoding may be compact, but only behind a typed wrapper:

```rust
struct ReasonCode(usize);

impl ReasonCode {
    fn none() -> Self;
    fn from_clause(r: ClauseRef) -> Self;
    fn from_binary(id: BinaryClauseId) -> Self;
    fn decode(self) -> ReasonRef;
}
```

Conflict analysis expands binary reasons as:

```text
ReasonCode::decode(reason) must return ReasonRef::Binary(id), then binary_clauses[id] expands as the two-literal clause
```

Rules:

```text
- binary clauses must still be represented for proof/model/debug or through an explicit proof path
- generated binary clauses must be deduplicated before insertion
- deleted binary clauses remain in binary_clauses until binary-implication rebuild or GC
- original and conflict-learned binaries are protected from reduce-db
- generated redundant binaries are subject to binary_clause_limit and may be deleted only through FormulaEditTxn/proof deletion
- binary usage is counted when used as propagation reason or conflict reason
- GC rewrites all references that may depend on BinaryClauseId or ReasonRef
- never use the same indexing-rule name for long watched clauses and binary implication adjacency
```

### Tests

```text
test_binary_propagates_implied_literal
test_binary_conflict_detected
test_binary_reason_expands_in_analyze
test_binary_original_clause_preserved_for_model_check
test_binary_fast_and_legacy_same_result_on_small_oracle
test_binary_fast_path_sets_assignment
test_binary_generated_clause_has_proof_id
test_binary_delete_marks_edge_stale_until_rebuild
test_binary_dedup_prevents_duplicate_hbr_edge
test_generated_redundant_binary_deleted_through_formula_edit
test_original_binary_not_deleted_by_binary_budget
test_binary_usage_counter_updates_on_reason
```

### Benchmark gate

```bash
SAT_BINARY_FAST=on \
  bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/search-core solver/11-kissat-search
```

Track:

```text
binary_props
long_props
watch_scans
watch_clause_loads
propagations/sec
binary_stale_skips
```

If runtime regresses, inspect cloning/allocation of watch vectors. Propagation must scan by index and avoid per-literal allocation.

## 1.7 Decision heap cleanup

### Goal

Reduce stale heap work and make decision selection predictable before adding phase, VMTF, or focused/stable modes.

### Dependencies

1.0, 0.3.

### Code-level changes

Add helpers:

```rust
fn heap_contains_var(&self, var: usize) -> bool;
fn heap_remove_assigned_top(&mut self);
fn heap_reinsert_unassigned_decision_var(&mut self, var: usize);
fn push_branch_var_if_decision(&mut self, var: usize);
```

Rules:

```text
When variable becomes assigned:
  - lazy removal is fine
  - skip assigned variables at heap top

When backtracking:
  - reinsert unassigned decision vars whose heap position is BRANCH_NOT_IN_HEAP

When eliminated/frozen:
  - mark non-decision
  - never reinsert

When activity bumped:
  - if in heap, percolate up
  - if not assigned and decision_var, insert

Determinism:
  - heap ties break by variable id unless a profile explicitly enables randomized decisions
  - VMTF ties break by stamp, then variable id
  - activity rescale preserves relative order except where values are exactly tied
  - SAT_SEED controls all randomized decisions and rephase randomness
  - config_hash records every randomized heuristic knob
```

### Tests

```text
test_eliminated_var_not_reinserted
test_assigned_heap_top_skipped
test_backtrack_reinserts_unassigned_decision_var
test_activity_bump_percolates
test_heap_push_respects_decision_var
test_heap_tie_break_is_deterministic
test_activity_rescale_preserves_order
test_same_seed_reproduces_decision_prefix_on_small_formula
test_different_seed_changes_only_randomized_policy
```

### Benchmark gate

Mostly counters:

```text
decision_heap_pops
decision_heap_stale_pops
decision_heap_inserts
```

No measurable speedup required, but stale pop rate should not worsen.

---

## 1.8 Core search default candidate milestone

### Goal

Test combinations, not just isolated features. Some policies only make sense together.

### Dependencies

1.1 through 1.7.

### Candidate configurations

Conservative:

```bash
SAT_PROFILE=experimental
SAT_PROOF=drat
SAT_SEED=0
SAT_USE_LBD=on
SAT_LBD_UPDATE_REASONS=off
SAT_RESTART=kissat-ema
SAT_REDUCE=lbd-tiered
SAT_PHASE=saved
SAT_BINARY_FAST=off
SAT_CHRONO=off
SAT_SEARCH_MODE=single
SAT_CLAUSE_MIN=off
SAT_VMTF=off
SAT_REPHASE=off
SAT_CONFIG_OUT=log/phase1/candidate-conservative.config
```

Strong:

```bash
SAT_PROFILE=experimental
SAT_PROOF=drat
SAT_SEED=0
SAT_USE_LBD=on
SAT_LBD_UPDATE_REASONS=on
SAT_RESTART=kissat-ema
SAT_REDUCE=lbd-tiered
SAT_PHASE=target-then-saved
SAT_BINARY_FAST=on
SAT_CHRONO=off
SAT_SEARCH_MODE=single
SAT_CLAUSE_MIN=off
SAT_VMTF=off
SAT_REPHASE=off
SAT_CONFIG_OUT=log/phase1/candidate-strong.config
```

Exploratory:

```bash
SAT_PROFILE=experimental
SAT_PROOF=drat
SAT_SEED=0
SAT_USE_LBD=on
SAT_LBD_UPDATE_REASONS=on
SAT_RESTART=kissat-ema
SAT_REDUCE=lbd-tiered
SAT_PHASE=target-then-saved
SAT_BINARY_FAST=on
SAT_CHRONO=off
SAT_SEARCH_MODE=single
SAT_CLAUSE_MIN=off
SAT_VMTF=off
SAT_REPHASE=off
SAT_CONFIG_OUT=log/phase1/candidate-exploratory.config
```

### Promotion rule

Promote to default only if:

```text
- smoke proof passes
- brute-force oracle passes relevant variants
- profiling has no status regressions
- search-core improves by median time, solved count, or robust counters
- regression-guards do not lose previously solved instances
```

Default promotion should target the existing performance profiles, not the global baseline all at once:

```text
baseline (search=Safe, preprocess=Off):
  - remains solver-10-equivalent until the end of Phase 1

default (search=Validated, preprocess=Conservative):
  - enables validated Phase 1 search features that passed smoke, oracle, proof-on, discriminating, and regression-guard checks
  - Phase 1 promotion targets default.search; preprocess axis stays Conservative throughout Phase 1

fast (search=Strong, preprocess=GateAware):
  - enables validated Phase 1 search features plus stronger knobs that improve the discriminating set without lost solved regressions
  - Phase 1 promotion seeds fast.search; the preprocess axis is finalized in Phase 2

experimental:
  - may enable risky features, but never used for acceptance unless explicitly stated
```

Proof validation is a separate certification axis, not a performance profile.

Promotion sequence:

```text
1. promote feature to experimental
2. promote into the `default` profile's search axis after discriminating + regression-guards
3. mark feature ProofValidated only after proof-on validation
4. promote into `fast`'s search axis only if the feature wins on the discriminating set without losing solved regression-guards
5. promote to a baseline-candidate replay artifact only after end-of-phase full-set validation
6. promote baseline-candidate to baseline only after the default-profile hardening gate
```

Candidate replay rule:

```text
- candidate runs use SAT_PROFILE=experimental until promotion
- promoted profiles are generated from validated SAT_CONFIG_REPLAY files, not hand-copied env snippets
- every candidate config must include SAT_PROOF, SAT_SEED, SAT_CONFIG_OUT, and concrete values for every feature flag in scope
```

---

## 1.9 Focused/stable mode scaffold and reluctant restarts

### Goal

Wire mode state cleanly before changing the decision heuristic. Use focused mode for aggressive restarts and stable mode for slower, more persistent search.

### Dependencies

1.4, 1.5, 1.7.

### Code-level changes

Add:

```rust
enum SearchMode { Focused, Stable }

search_mode: SearchMode,
mode_start_conflicts: u64,
mode_start_decisions: u64,
mode_switches: u64,
mode_switch_at_conflicts: u64,
mode_interval: u64,
```

Initial policy:

```text
Focused:
  - existing heap initially
  - saved phase or target/saved policy
  - EMA restart

Stable:
  - existing heap initially
  - target/best phase policy
  - reluctant restart schedule
```

Reluctant sequence:

```rust
struct Reluctant { u: u64, v: u64 }
```

Verify prefix:

```text
1, 1, 2, 1, 1, 2, 4, ...
```

Mode switching:

```text
start focused
switch after mode_init = 2_000 conflicts
then alternate with interval scaled by sqrt(mode_switches + 1) * mode_init
on switch, drain restart_pending so the new mode starts clean
on switch to stable, refresh the VSIDS heap from current variable activity
```

### Tests

```text
test_mode_starts_focused_default
test_mode_switch_after_budget
test_mode_switch_back_preserves_heap
test_mode_switch_resets_restart_pending
test_mode_stats_count_switches
test_reluctant_sequence_matches_expected_prefix
```

### Benchmark gate

Expected by itself: flat or small improvement. Accept if correctness holds and overhead is negligible. Do not promote as default until VMTF/rephase experiments have been tested.

---

## 1.10 VMTF focused-mode decision queue

### Goal

Add Kissat-like Variable-Move-To-Front decision behavior in focused mode while keeping the VSIDS heap in stable mode.

### Dependencies

1.9.

### Code-level changes

New module:

```text
src/branch.rs
```

Data structure:

```rust
struct VmtfQueue {
    next: Vec<u32>,
    prev: Vec<u32>,
    head: u32,
    search: u32,
    stamp: Vec<u64>,
    stamp_counter: u64,
}
```

After conflict analysis:

```text
for each analyzed variable:
  stamp[v] = stamp_counter
  stamp_counter += 1
  queue.move_to_front(v)
```

Decision:

```text
pick_branch_lit_focused walks from search toward head looking for the most recently stamped unassigned decision var
remember search position
never pick assigned, eliminated, or non-decision vars
phase selection still goes through pick_branch_phase
```

### Tests

```text
test_vmtf_recently_analyzed_variable_pops_first
test_vmtf_search_pointer_resets_on_relevant_backtrack
test_vmtf_does_not_pick_assigned_variables
test_vmtf_does_not_pick_eliminated_variables
test_vmtf_tie_break_is_deterministic
```

### Benchmark gate

Run focused/stable candidate with and without VMTF. Accept if discriminating PAR-2 does not regress beyond noise and shuffled SAT/search-throughput instances improve.

---

## 1.11 Clause minimization, in-block shrink, and reason-side bumping

### Goal

Improve learned clause quality without adding expensive recursive work by default.

### Dependencies

1.1, 1.3, 1.4.

### Code-level changes

Add modes:

```rust
enum ClauseMinMode {
    Off,
    Basic,
    RecursiveLimited,
    InBlockShrink,
}
```

Basic minimization:

```text
Drop a literal only if its reason literals are already seen or assigned at level 0.
Never drop the UIP/asserting literal.
Never drop a decision literal.
```

Recursive-limited:

```rust
fn lit_redundant(&mut self, lit: i32, depth: u32) -> bool {
    if depth > self.config.minimize_depth_limit { return false; }
    let r = self.reason[var(lit)];
    if r == NO_REASON { return false; }

    for q in self.reason_lits_except_first(r) {
        if self.level(q) == 0 { continue; }
        if !self.seen[var(q)] && !self.lit_redundant(q, depth + 1) {
            return false;
        }
    }
    true
}
```

In-block shrink:

```text
After UIP, remove a literal only if its same-level reason chain is covered by already-included literals.
Use per-level/per-var stamps, not repeated vector clears.
```

Reason-side bumping:

```text
During UIP DFS, also push reason-side parent variables into scratch_bumped_vars when level > 0.
```

### Tests

```text
test_basic_minimization_removes_level_zero_reason_literal
test_minimization_does_not_remove_decision_literal
test_recursive_limit_prevents_unbounded_walk
test_minimized_clause_still_asserting
test_shrink_removes_block_covered_literal
test_shrink_leaves_uip_at_pos_zero
test_conflict_analysis_tracks_intermediate_reason_variables_for_activity
```

### Benchmark gate

Track:

```text
avg_lbd
avg_learned_size
conflicts
propagations
analyze_time if available
```

Keep recursive minimization off by default unless learned-size gains justify analyze overhead.

---

## 1.12 Rephasing hook

### Goal

Diversify stable-mode phase selection, especially for SAT-heavy instances.

### Dependencies

1.5, 1.9.

### Code-level changes

Fields:

```rust
rephase_index: u8,
rephase_at_conflicts: u64,
rephase_conflicts: u64,
original_phase: Vec<u8>,
```

Cycle:

```text
best -> inverted -> original
```

Initial implementation:

```text
best: copy best_phase into saved_phase
inverted: flip saved_phase
original: restore original_phase
```

Only run on stable-mode restart when schedule fires.

### Tests

```text
test_rephase_best_writes_best_phase_into_saved
test_rephase_inverted_flips_all_saved_phases
test_rephase_original_restores_original_phase
test_rephase_advances_index_on_each_call
test_rephase_cycle_excludes_walk_by_default
```

### Benchmark gate

Expected small gains on SAT instances. Keep off if noise or regression dominates.


---

## 1.12a Advanced search candidate milestone

### Goal

Evaluate the composed post-core search stack after focused/stable mode, VMTF, clause minimization, and rephasing exist.

### Dependencies

1.8, selected 1.9, 1.10, 1.11, 1.12.

### Candidate configurations

Focused/VMTF:

```bash
SAT_PROFILE=experimental
SAT_PROOF=drat
SAT_SEED=0
SAT_USE_LBD=on
SAT_LBD_UPDATE_REASONS=on
SAT_RESTART=kissat-ema
SAT_REDUCE=lbd-tiered
SAT_PHASE=target-then-saved
SAT_BINARY_FAST=on
SAT_SEARCH_MODE=focused-stable
SAT_CLAUSE_MIN=basic
SAT_VMTF=on
SAT_REPHASE=off
SAT_CHRONO=off
SAT_CONFIG_OUT=log/phase1/candidate-focused-vmtf.config
```

Stable-SAT exploratory:

```bash
SAT_PROFILE=experimental
SAT_PROOF=drat
SAT_SEED=0
SAT_USE_LBD=on
SAT_LBD_UPDATE_REASONS=on
SAT_RESTART=kissat-ema
SAT_REDUCE=lbd-tiered
SAT_PHASE=best-then-target-then-saved
SAT_BINARY_FAST=on
SAT_SEARCH_MODE=focused-stable
SAT_CLAUSE_MIN=recursive-limited
SAT_VMTF=on
SAT_REPHASE=on
SAT_CHRONO=off
SAT_CONFIG_OUT=log/phase1/candidate-stable-sat.config
```

### Promotion rule

Do not replace the 1.8 core profile unless advanced candidates beat it on search-core, discriminating, and regression-guards without proof/model failures.

---

## 1.13 Guarded chronological backtracking

### Goal

Experiment with chronological backtracking without risking trail/reason corruption.

### Skip rule (added before scoping work)

```text
- Skip this task if the Phase 1 candidate composed in 1.12a already meets the Phase 1 PAR-2 target on search-core AND discriminating AND regression-guards.
- Chrono backtracking is high-risk for correctness and often unnecessary; the plan should let agents skip it cleanly when its motivation is absent.
- Record the skip decision (with reference to the 1.12a candidate report) in `log/phase1/triage.md`.
```

### Dependencies

1.1, 1.7, 1.11.

### Code-level changes

Use a conservative implementation first:

```rust
fn choose_backtrack_level(&self, assertion_level: usize, learnt_clause: &[i32]) -> usize {
    if !self.config.chrono_backtrack { return assertion_level; }

    let current = self.current_decision_level();
    if current == 0 { return 0; }
    if assertion_level >= current {
        return assertion_level;
    }

    let delta = current - assertion_level;
    if delta > self.config.chrono_max_delta {
        return assertion_level;
    }

    let chrono_level = current - 1;
    if self.learnt_clause_asserts_at_level(learnt_clause, chrono_level) {
        chrono_level
    } else {
        assertion_level
    }
}
```

Do not start with trail splicing or arbitrary `enqueue_at_level` unless the conservative version shows value. If a later implementation adds `enqueue_at_level`, it must come with deep invariant tests and oracle coverage.

Stats:

```text
chrono_attempts
chrono_used
chrono_rejected_not_asserting
chrono_rejected_delta_too_large
chrono_skipped_levels
```

### Tests

```text
test_chrono_off_uses_assertion_level
test_chrono_rejects_large_delta
test_chrono_allows_small_delta_when_asserting
test_chrono_rejects_non_asserting_level
test_chrono_backtrack_preserves_reason_invariant
test_chrono_root_conflict_unchanged
test_chrono_does_not_break_smoke_unsat_proof
```

### Benchmark gate

Run only after LBD/restart/reduction/phase are stable:

```bash
SAT_CHRONO=on SAT_USE_LBD=on SAT_RESTART=kissat-ema SAT_REDUCE=lbd-tiered
```

Accept only if search-core improves or one hard class improves without proof/model regressions.

---

## 1.14 Search-throughput pass

### Goal

Lift propagation rate after the higher-level search policy is stable.

### Dependencies

1.3a, 1.6, 1.9.

### Subtasks

Each is a separate beads node:

```text
1.14a Inline-blocker propagation specialization:
  - skip deleted-clause and length checks before blocker compare when safe

1.14b Watch list compaction:
  - when a watcher misses its blocker, swap-remove and continue without extra bookkeeping
  - prove compatibility with deleted-watch skipping

1.14c Binary implication storage tuning:
  - switch BinaryImplications from Nested to Flat only through the storage abstraction
  - measure Nested vs Flat adjacency representation
  - avoid per-propagation allocation
  - require no propagation/HBR/transitive/proof code changes outside BinaryImplications

1.14d f32 clause activity experiment:
  - only after LBD metadata layout is stable
  - revalidate; do not assume older neutral result still holds
```

### Tests

Existing propagation tests plus:

```text
test_binary_clause_fast_path_sets_assignment
test_watch_compaction_preserves_propagation_result
test_deleted_watcher_skipped_after_compaction
```

### Benchmark gate

Track:

```text
propagations/sec
watch_scans
watch_clause_loads
watch_blocker_hits
binary_props
long_props
```

The target is aggregate propagation-rate improvement on propagation-heavy search instances. Do not merge micro-optimizations that only look faster in isolation but regress K4/Kakuro or Timetable classes.

---

## 1.15 Phase 1 gate and promotion

Run:

```bash
bash tools/bench.sh -t 300 -m 16384 \
  -d benchmarks/discriminating solver/11-kissat-search

bash tools/bench.sh -t 1800 -m 16384 \
  -d benchmarks/sat-comp-2025 solver/11-kissat-search
```

Target direction:

```text
- SC-2025 PAR-2 moves meaningfully below the solver-09/solver-10 baseline.
- discriminating PAR-2 improves at least 25% by the end of core LBD/restart/reduction/phase/binary work, or counters explain why a later inprocessing phase is needed.
- no lost solved instances in regression-guards.
```

Only after this gate should Phase 2 inprocessing features become serious default candidates.

---

# 5. Phase 2 — Clause simplification, rewriting, and formula modification

Absolute rule:

```text
All formula modifications happen at decision level 0 unless the task explicitly implements temporary assumptions and backtracks cleanly before returning to normal search.
```

Proof logging and model reconstruction are first-class requirements, not optional cleanup.

The safe priority order is:

```text
1. inprocessing scheduler scaffold
2. proof deletion support and transformation audit
3. compaction/watch rebuild and GC rewrite policy
4. formula-edit transaction layer
5. model-extension replay contract
6. vivification
7. failed literal probing
8. HBR
9. BVE scheduling/cost upgrade
10. gate extraction
11. local implied-clause/RUP checker scaffold
12. gate-aware BVE
13. BSR/subsumption hot-path and forward sweep
14. transitive reduction over binary implication graph
15. full RCheck as diagnostic/guarded helper only
16. Phase 2 infrastructure certification gate
17. Phase 2 feature retirement and candidate pruning
18. Phase 2 candidate gate
```

ELS, BCE, walking local search, and optional sweep are outside the main Phase 2 target path. Keep their design notes, but implement them only after milestone triage identifies them as the next bottleneck.

---

## 2.0 Inprocessing scheduler scaffold

### Goal

Move from one-shot preprocessing to a controlled, budgeted, measurable inprocessing loop between search epochs.

### Dependencies

0.3, 0.4, 0.6, Phase 1 core gate.

### Code-level changes

Add:

```rust
struct InprocessLimits {
    next_conflict: u64,
    interval_conflicts: u64,
    max_rounds: u64,
    prop_budget: u64,
    clause_budget: usize,
    tick_budget: u64,
}

struct InprocessStats {
    rounds: u64,
    skipped_not_level_zero: u64,
    skipped_budget: u64,
    ticks: u64,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum InprocessPassId {
    RootCleanup,
    VivifyOriginal,
    ProbeFailedLiterals,
    Hbr,
    GateExtract,
    GateAwareBve,
    Bve,
    BsrForward,
    TransitiveReduction,
    RCheck,
}

impl InprocessPassId {
    fn name(self) -> &'static str;
    fn enabled(self, config: &SolverConfig) -> bool;
    fn should_run(self, solver: &Solver, ctl: &InprocessController) -> bool;
}

struct InprocessPassResult {
    ticks: u64,
    clauses_added: u64,
    clauses_deleted: u64,
    literals_removed: u64,
    units_added: u64,
    binaries_added: u64,
    vars_eliminated: u64,
    conflicts_saved_proxy: i64,
    exhausted_budget: bool,
    proof_bytes_added: u64,
    extension_bytes_added: u64,
    watch_rebuild_ticks: u64,
    shadow_value_score: i64,
}

struct InprocessController {
    round: u64,
    global_tick_budget: u64,
    next_eligible_round: Vec<u64>,
    deterministic_order: Vec<InprocessPassId>,
}
```

Internal inprocessing outcomes are distinct from final user-visible `SolveStatus`:

```rust
enum InprocessOutcome {
    NoResult,
    Unsat,
    AbortPassBudget,
    AbortSolve(SolveLimitReason),
}

enum InprocessPassError {
    BudgetExhausted,
    SolveLimit(SolveLimitReason),
}
```

Search loop insertion:

```rust
if self.config.inprocess
   && self.current_decision_level() == 0
   && self.should_inprocess()
{
    match self.inprocess_round() {
        InprocessOutcome::Unsat => return SolveStatus::Unsat,
        InprocessOutcome::AbortSolve(reason) => return SolveStatus::Unknown,
        InprocessOutcome::NoResult | InprocessOutcome::AbortPassBudget => {}
    }
}
```

Round skeleton:

```rust
fn inprocess_round(&mut self) -> InprocessOutcome {
    debug_assert_eq!(self.current_decision_level(), 0);

    if self.propagate_root_units_or_return_unsat() == SolveStatus::Unsat {
        return InprocessOutcome::Unsat;
    }

    for pass_id in self.inprocess_controller.choose_pass_order(&self.config).iter().copied() {
        if self.inprocess_budget.exhausted() { break; }

        let result = match self.run_inprocess_pass(pass_id) {
            Ok(result) => result,
            Err(InprocessPassError::BudgetExhausted) => return InprocessOutcome::AbortPassBudget,
            Err(InprocessPassError::SolveLimit(reason)) => return InprocessOutcome::AbortSolve(reason),
        };
        self.inprocess_controller.record_result(pass_id, result);

        if self.propagate_root_units_or_return_unsat() == SolveStatus::Unsat {
            return InprocessOutcome::Unsat;
        }
    }

    self.rebuild_branch_heap_after_simplification();
    self.garbage_collect_if_needed();

    InprocessOutcome::NoResult
}
```

Initial fixed order:

```text
1. cheap root propagation cleanup
2. original-clause vivification
3. failed-literal probing with HBR collection
4. gate extraction
5. gate-aware BVE
6. existing BVE/BSR budgeted cleanup
7. transitive reduction
8. ELS only after Appendix unpark triage and explicit validation
```

Do not implement adaptive scoring in 2.0. Add it later as task 2.16a only after Phase 2 candidate gates have enough counter data.

Initial scheduler rule:

```text
- use static InprocessPassId dispatch first
- do not use Box<dyn InprocessPass> in the initial scheduler
- no pass selection path may allocate during a normal inprocessing round
- a later trait-object plugin layer requires a benchmark-justified task and allocation counters
```

Shadow ROI rule:

```text
- deterministic scheduling remains fixed in 2.0
- every pass records shadow_value_score using a stable formula
- shadow_value_score is reported but never used for scheduling before 2.16a
- 2.16a may change scheduling only after comparing shadow predictions with actual benchmark outcomes
```

Future adaptive scoring:

```text
positive value:
  - root units added
  - binaries added and later used
  - literals removed
  - clauses deleted
  - variables eliminated
  - residual formula shrink

negative value:
  - ticks spent
  - proof bytes added
  - watch rebuild cost
  - later search-core regression
  - extension bytes added
  - binary clauses added but never used
```

Rules:

```text
- all passes run at decision level 0
- every pass has a tick/propagation/clause budget
- after each pass, root propagation runs before the next pass
- pass order is deterministic for SAT_SEED
- pass ROI changes frequency and budget, not correctness behavior, and is disabled until task 2.16a
- a disabled pass must be a true no-op
```

### Tests

```text
test_inprocess_not_called_above_level_zero
test_inprocess_interval_triggers
test_inprocess_budget_skip
test_inprocess_noop_has_no_behavior_change
test_inprocess_controller_runs_root_propagation_after_each_pass
test_inprocess_pass_order_is_deterministic_for_seed
test_inprocess_rebuilds_branch_heap_after_simplification
```

### Benchmark gate

```bash
SAT_INPROCESS=on SAT_VIVIFY=off SAT_PROBE=off SAT_HBR=off   bash tools/bench.sh -t 120 -m 16384   -d benchmarks/iteration/search-core solver/11-kissat-search
```

Expected: no behavior change except tiny scheduling overhead.

## 2.1 Proof logging upgrade: deletions and transformation audit

### Goal

Make proof logging explicit enough for inprocessing transformations before those transformations delete or rewrite clauses.

### Dependencies

2.0.

### Code-level changes

Extend `ProofLog`:

```rust
fn record_add_clause(&mut self, lits: &[i32]);
fn record_delete_clause(&mut self, lits: &[i32]);
fn finish_unsat(&mut self);
fn finish_sat_cleanup(&mut self);
```

DRAT deletion format:

```text
d lit1 lit2 ... 0
```

LRAT additions (new lines, parallel to DRAT lines):

```text
<id> a <lits> 0 <chain-of-cids> 0    # addition with explicit hints
<id> d <cids> 0                       # deletion of listed clause ids
```

LRAT integration rules:

```text
- ProofLog assigns a monotone clause ID to every learned/added clause from the learning epoch onward.
- LRAT chain hints are produced inline by conflict analysis (analysis already walks reasons; collect their IDs into a scratch vec).
- DRAT and LRAT emission share the same buffered-output path; only the formatter differs.
- A proof writes either DRAT or LRAT, never both; the checker (`drat-trim` vs `lrat-check`/`cake_lrat`) is auto-selected from the proof path suffix (`.drat`, `.lrat`).
- BVE/vivification/HBR/transitive each emit explicit hint chains for LRAT; lacking hints, fall back to DRAT for that profile (recorded in JSON_STATS as `proof_format_fallback=drat`).
- For research/SAT-competition-style verification, LRAT is preferred: it has formally verified checkers (`cake_lrat`) and is typically 3-10x faster to check than DRAT.
- DRAT remains supported as compatibility for tools that do not consume LRAT.
```

Add counters:

```text
proof_added_clauses
proof_deleted_clauses
proof_added_literals
proof_deleted_literals
proof_flushes
proof_bytes_written
proof_temp_created
proof_temp_deleted
proof_finalized
proof_incomplete
proof_write_errors
proof_format        # "drat" or "lrat"
proof_check_sec     # checker wall time when verification ran
proof_format_fallback  # if a pass could not emit hints and we fell back to DRAT
```

Transformation audit:

```text
For each simplification/inprocessing mutator, specify:
  - clauses added
  - clauses deleted
  - whether old clauses are still needed for proof context
  - model-extension entry, if SAT assignment can be affected
  - watcher update strategy
  - occurrence-list update strategy
```

Create:

```text
solver/11-kissat-search/PROOF_OBLIGATIONS.md
```

Required matrix columns:

```text
transformation
task_id
formula_edit_kind
proof_adds_required
proof_deletes_required
rup_or_rat_check_required_in_invariant_mode
model_extension_required
allowed_when_SAT_PROOF_off
allowed_when_SAT_PROOF_drat
validation_tests
```

Initial rows:

```text
BVE resolvent generation
BVE input deletion
Vivification strengthening
Vivification subsumption deletion
Failed literal unit
HBR generated binary
Transitive binary deletion
Gate-aware BVE resolvent envelope
BSR strengthening
Forward subsumption deletion
RCheck implied-clause skip
```

Rules:

```text
- no formula-rewrite task is accepted unless PROOF_OBLIGATIONS.md has a row for it
- validate_solver11_plan.py rejects formula-edit tasks without a proof-obligation row
- invariant mode runs a local RUP/RAT precheck where the matrix requires it
- proof-off success is not proof-on certification
```

Proof completeness states:

```rust
enum ProofState {
    Disabled,
    TempOpen,
    Incomplete,
    Finalized,
}
```

Rules:

```text
- SAT result must end in Disabled or Incomplete with temp proof removed
- UNKNOWN must end in Incomplete with temp proof removed
- UNSAT must end in Finalized after final empty clause and atomic rename
- any proof write error forces UNKNOWN or process failure with proof marked incomplete
```

### Tests

```text
test_proof_add_clause_format
test_proof_delete_clause_format
test_sat_removes_temp_proof
test_unsat_final_empty_clause_written
test_drat_records_deletions_for_bve_inputs
test_drat_proof_verifies_after_inprocessing
test_proof_obligation_matrix_has_rows_for_enabled_formula_edits
```

### Benchmark gate

Proof-off and proof-on comparison on one Timetable-like SAT case and one K4-like UNSAT case. Correctness is mandatory; performance is observed but not a gate unless catastrophic.

---

## 2.2 Compaction and watch rebuild policy

### Goal

Avoid stale watcher and arena-reference bugs as formula rewrites become common.

### Dependencies

2.1.

### Code-level changes

Add a clear mutator contract:

```text
For in-place clause strengthening or substitution:
  preferred first strategy: detach watchers before rewriting, then reattach
  alternative strategy: mark dirty and make propagation skip impossible watchers
  do not mix strategies within the same transformation family
```

Add helpers:

```rust
fn detach_clause_for_rewrite(&mut self, clause_idx: usize);
fn reattach_clause_after_rewrite(&mut self, clause_idx: usize);
fn mark_watch_lists_dirty_for_clause(&mut self, clause_idx: usize);
fn rebuild_watchers_after_inprocess(&mut self);
fn rebuild_binary_implications_after_inprocess(&mut self);
fn validate_watch_invariants(&self);
```

GC must build a fresh ReasonPinSet before deciding what can be moved, deleted, or compacted.

GC must rewrite:

```text
watchers
binary_implications if they store clause refs
reasons
learned clause refs
original clause refs
occurrence refs
extension stack refs if any
proof audit refs if any
```

### Tests

```text
test_rewrite_detaches_and_reattaches_watchers
test_dirty_watcher_skipped_safely
test_gc_rewrites_clause_refs_after_inprocess
test_binary_implications_rebuilt_after_clause_deletion
```

### Benchmark gate

No speed requirement. Invariant mode must pass on smoke-plus and preprocess-core.

---

## 2.1a Formula-edit transaction layer

### Goal

Make every formula mutation update proof, model reconstruction, watchers, occurrences, reasons, and stats through one auditable path.

### Dependencies

2.1, 2.2.

### Code-level changes

Add:

```rust
enum FormulaEditKind {
    AddClause,
    DeleteClause,
    StrengthenClause,
    SubstituteLiterals,
    AddRootUnit,
    EliminateVariable,
    AddBinary,
    DeleteBinary,
}

struct FormulaEditPlan {
    txn_id: u64,
    kind: FormulaEditKind,
    added: Vec<Vec<i32>>,
    deleted: Vec<Vec<i32>>,
    strengthened: Vec<(Vec<i32>, Vec<i32>)>, // old, new
    model_effect: ModelEffect,
    extension_entries: Vec<ExtensionEntry>,
    touched_clauses: Vec<ClauseRef>,
    touched_vars: Vec<usize>,
    proof_required: bool,
    model_extension_required: bool,
    estimated_bytes: usize,
}

enum ModelEffect {
    ModelPreserving {
        reason: ModelPreservingReason,
    },
    NeedsExtension {
        entries: Vec<ExtensionEntry>,
    },
    ForbiddenForSatOutput {
        reason: &'static str,
    },
}

enum ModelPreservingReason {
    AddedClauseOnly,
    StrengthenedToSubset,
    DeletedTautology,
    DeletedSubsumedClause,
    DeletedSemanticallyImpliedClause,
    DeletedRedundantBinaryWithCheckedAlternateImplication,
}

struct FormulaEditPreflight {
    plan: FormulaEditPlan,
    proof_add_buffer: Vec<Vec<i32>>,
    proof_delete_buffer: Vec<Vec<i32>>,
    debug_replay_record: Option<FormulaEditReplayRecord>,
    watcher_strategy: WatcherMutationStrategy,
}

enum AbortedEdit {
    Plan(FormulaEditPlan),
    Preflight(FormulaEditPreflight),
}
```

`FormulaEditTxn` remains the task/layer name, but the code-level handles are deliberately split into pure planning and owned preflight. Do not implement a single mutable transaction object that can both plan and mutate.

Model-extension journal:

```rust
enum ExtensionEntry {
    EliminatedVar {
        var: usize,
        positive_occurs: Vec<Vec<i32>>,
        negative_occurs: Vec<Vec<i32>>,
    },
    GateEliminatedVar {
        eliminated_lit: i32,
        gate: GateDef,
        restored_clauses: Vec<Vec<i32>>,
    },
    StrengthenedClause {
        old_clause: Vec<i32>,
        new_clause: Vec<i32>,
    },
    DeletedSatisfiedClause {
        clause: Vec<i32>,
        witness_lit: i32,
    },
}
```

Rules:

```text
- every destructive edit must choose a ModelEffect before mutation
- ModelPreserving edits must include a checkable reason
- NeedsExtension edits must install extension entries before mutation
- FormulaEditPlan.extension_entries is derived from ModelEffect::NeedsExtension entries during preflight; no pass may populate it independently
- ForbiddenForSatOutput edits are rejected when SAT model output is required
- extension entries are replayed in exact reverse commit order
- each entry has an estimated byte cost before commit
- invariant mode validates each replay step against the clauses it restores
- invariant mode validates ModelPreserving reasons against the debug replay formula when practical
- Appendix features may add ExtensionEntry variants only in the task that unparks them
- no pass may use `DeletedSatisfiedClause` unless it can name a witness literal under the reconstructed assignment
```

API:

```rust
fn begin_formula_edit(&mut self, kind: FormulaEditKind) -> FormulaEditPlan;
fn preflight_formula_edit(&mut self, plan: FormulaEditPlan) -> Result<FormulaEditPreflight, EditPreflightError>;
fn commit_formula_edit(&mut self, preflight: FormulaEditPreflight) -> EditResult;
fn abort_formula_edit(&mut self, plan_or_preflight: impl Into<AbortedEdit>);
```

Commit order:

```text
1. verify current decision level is 0 unless the edit is explicitly temporary-assumption-safe
2. check all deleted/strengthened clauses are still live
3. build a fresh ReasonPinSet and reject deletion of any pinned reason unless the plan supplies a replacement reason or root-level reassignment plan
4. estimate transaction memory and reject if extension/proof/txn memory budgets would be exceeded
5. build proof additions/deletions and validate they are serializable before mutation
6. build debug replay record before mutation when SAT_CHECK_INVARIANTS=on
7. freeze FormulaEditPreflight; no pass-owned buffers or borrowed clause slices may be used after this point
8. install model-extension entries before destructive deletion
9. detach or mark watchers according to the chosen mutator strategy
10. update occurrence lists and n_occ
11. update binary implication adjacency if binary clauses changed
12. rebuild branch eligibility for touched variables
13. validate invariants in SAT_CHECK_INVARIANTS mode
14. write buffered proof records
15. append FormulaEditReplayRecord in debug mode
16. commit stats counters
```

Rules:

```text
- FormulaEditPlan construction is pure: it may inspect formula state but may not mutate proof, watchers, occurrence lists, binary implications, trail, branch structures, or extension stack.
- FormulaEditPreflight owns all literals needed for proof/model/debug replay; it must not borrow clause memory that can be invalidated by commit.
- simplification passes must not call proof.record_* directly after this layer exists
- simplification passes must not mark clauses deleted directly after this layer exists
- direct low-level mutation is allowed only inside commit_formula_edit
- proof lines are buffered inside the transaction until preflight and invariant checks pass
- abort must leave proof, watchers, occurrences, binary implication adjacency, model extension, and stats unchanged
- a failed proof serialization preflight aborts before mutation
- an EditBudget failure aborts the transaction before mutation and returns control to the caller
- optional inprocessing callers must treat EditBudget failure as pass exhaustion, not solver failure
- optional inprocessing pass-budget exhaustion returns InprocessOutcome::AbortPassBudget, not final SolveStatus::Unknown
- a failed proof write after destructive mutation is a fatal solver error; proof temp file must be marked incomplete, result must not be reported as proof-complete UNSAT
- every committed formula edit in invariant mode appends a replayable edit record
- transaction memory counts toward `max_txn_buffer_lits` and `extension_bytes_estimated`
```

### Tests

```text
test_formula_edit_debug_log_replays_against_vec_formula_model
test_formula_edit_replay_shrinker_minimizes_failing_edit_prefix
test_formula_edit_plan_is_pure_and_does_not_mutate_formula
test_formula_edit_preflight_owns_proof_model_and_debug_literals
test_formula_edit_add_clause_logs_proof_and_occurrence
test_formula_edit_delete_clause_updates_watch_and_occurrence
test_formula_edit_strengthen_clause_adds_new_before_delete_old
test_formula_edit_requires_extension_for_sat_affecting_deletion
test_formula_edit_allows_model_preserving_strengthening_without_extension
test_formula_edit_allows_subsumed_clause_delete_without_extension
test_formula_edit_rejects_unclassified_destructive_delete
test_formula_edit_forbidden_for_sat_output_blocks_candidate
test_formula_edit_abort_leaves_formula_unchanged
test_formula_edit_abort_does_not_write_proof_records
test_formula_edit_preflight_fails_before_mutation
test_formula_edit_proof_serialization_preflight_before_mutation
test_formula_edit_commit_rejects_non_root_mutation
test_formula_edit_rejects_deleting_live_reason_without_replacement
test_formula_edit_allows_reason_deletion_after_replacement_reason
test_formula_edit_emits_replay_record
test_formula_edit_replay_record_reconstructs_occurrence_counts
test_extension_journal_replays_in_reverse_commit_order
test_extension_journal_rejects_missing_entry_for_destructive_delete
test_extension_journal_estimates_bytes_before_commit
test_extension_journal_gate_entry_restores_output_value
```

### Benchmark gate

No speed requirement. Invariant mode must pass smoke-plus and preprocess-core.


---

## 2.1b Model-extension replay contract

### Goal

Make SAT assignment reconstruction independently testable before destructive simplification features rely on it.

### Dependencies

2.1a, 0.6.

### Code-level changes

Create:

```text
src/model_ext.rs
```

Add:

```rust
struct ExtensionJournal {
    entries: Vec<ExtensionEntry>,
    estimated_bytes: usize,
}

enum ExtensionReplayResult {
    Pass,
    Conflict { entry_index: usize, clause: Vec<i32> },
    BudgetExceeded,
}

fn replay_extension_journal_reverse(
    &mut self,
    assignment: &mut [u8],
    mode: ReplayCheckMode,
) -> ExtensionReplayResult;
```

Rules:

```text
- extension replay is independent from proof logging
- replay checks the original restored clauses in invariant mode
- replay failure forces SAT result rejection, not UNKNOWN success
- extension byte budget can disable candidate edits before mutation
- extension replay cost is reported in JSON_STATS
```

### Tests

```text
test_extension_replay_reverse_order
test_extension_replay_reconstructs_bve_eliminated_var
test_extension_replay_reconstructs_gate_eliminated_var
test_extension_replay_detects_unsatisfied_restored_clause
test_extension_budget_rejects_edit_before_mutation
test_model_check_uses_original_cnf_not_residual_formula
```

### Benchmark gate

No speed promotion. SAT-heavy smoke-plus must pass with invariant replay enabled.


## 2.3 Vivification / asymmetric clause strengthening

### Goal

Strengthen clauses by temporary root-level assumptions and propagation.

### Dependencies

2.0, 2.1, 2.1a, 2.1b, 2.2, 1.0a.

### Scope

Start with original clauses. Then add learned tier-1/tier-2 clauses after proof and watcher behavior is stable.

### Code-level changes

New module:

```text
src/vivify.rs
```

Candidate filters:

```text
- skip deleted clauses
- skip length < 3
- skip clauses above max_vivify_clause_len initially, e.g. 100
- skip clauses touched too recently
- prioritize low LBD learned clauses after learned vivification is enabled
- prioritize original clauses with high occurrence leverage
```

Practical bounded algorithm:

```rust
fn vivify_clause(&mut self, clause_idx: usize, budget: &mut Budget) -> VivifyResult {
    if self.clause_deleted(clause_idx) { return VivifyResult::None; }
    if self.clause_len(clause_idx) < 3 { return VivifyResult::None; }
    if budget.exhausted() { return VivifyResult::Budget; }

    self.backtrack_to_root();
    self.vivify_scratch.clear();
    self.vivify_scratch.extend_from_clause(clause_idx);
    self.order_vivify_literals();

    // Try assuming negations of selected literals and propagate.
    // On conflict or implication, derive a strengthened clause.

    self.backtrack_to_root();
    self.apply_strengthening_with_proof(clause_idx, strengthened_lits)
}
```

Proof:

```text
When replacing C with C':
  independently verify C' is RUP/AT under the current formula before mutation in check mode
  record_add_clause(C') through formula-edit transaction
  record_delete_clause(C) when deletion is emitted/enabled
  update watchers and occurrences through formula-edit transaction
```

When a clause becomes a unit:

```text
enqueue at root
record proof addition
propagate root units
```

### Tests

```text
test_vivify_strengthens_clause_with_binary_implication
test_vivify_shrinks_known_strengthenable_clause
test_vivify_detects_subsumed_clause_and_deletes_it
test_vivify_derives_unit
test_vivify_no_change_when_not_implied
test_vivify_updates_occurrence_lists
test_vivify_proof_adds_strengthened_clause
test_vivify_does_not_affect_assignments_after_run
test_vivify_model_satisfies_original_formula
test_vivify_respects_ticks_budget
test_vivification_check_mode_verifies_strengthening_before_mutation
test_vivification_uses_formula_edit_transaction
```

### Benchmark gate

```bash
SAT_INPROCESS=on SAT_VIVIFY=on SAT_PROBE=off SAT_HBR=off \
  bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/preprocess-core solver/11-kissat-search
```

Track:

```text
vivify_attempts
vivify_strengthened
vivify_subsumed
vivify_removed_literals
vivify_props
search_conflicts_after_vivify
preprocess_time
```

Default on only if it pays for itself on search-core/preprocess-core or improves full profiling solved count/PAR-2.

---

## 2.4 Failed-literal probing

### Goal

Detect forced assignments and cheap implications at root level without polluting normal search statistics, phase memories, or branch queues.

### Dependencies

2.0, 2.1, 2.1a, 2.1b, 2.2, 1.0a.

### Code-level changes

New module:

```text
src/probe.rs
```

Candidate order:

```text
- unassigned decision vars with high activity
- vars with small occurrence product
- vars touched by recent conflicts
- dense occurrence candidates when seeking failed literals
```

Temporary-assumption contract:

```text
- probes use the 1.0a `with_temporary_assumptions` API
- probes start and end at root level
- temporary assumptions do not update saved/target/best phase
- temporary assumptions do not bump VMTF or heap decision statistics
- temporary propagation has separate counters
- root units discovered by probing are committed through the formula-edit transaction layer
```

Algorithm:

```rust
fn probe_literal(&mut self, lit: i32, budget: &mut Budget) -> ProbeResult {
    self.backtrack_to_root();

    let failed = self.with_temporary_assumptions(
        TemporaryAssumptionOptions::probe(),
        |ctx| ctx.enqueue(lit).is_conflict()
            || ctx.propagate_budgeted(budget).is_some(),
    );

    // Optionally collect implications for HBR inside the closure before restoration.

    if failed {
        self.add_root_unit_with_proof(neg(lit));
        ProbeResult::FailedLiteral(neg(lit))
    } else {
        ProbeResult::NoConflict
    }
}
```

Budget:

```text
probe_ticks_budget = 200_000 initially
```

### Tests

```text
test_flp_derives_known_unit_on_crafted_instance
test_probe_backtracks_to_root
test_probe_respects_ticks_budget
test_probe_proof_records_derived_units
test_probe_does_not_corrupt_saved_phase_or_trail
test_probe_does_not_update_target_or_best_phase
test_probe_does_not_bump_vmtf_or_decision_heap_as_real_decision
test_probe_root_unit_uses_formula_edit_transaction
```

### Benchmark gate

```bash
SAT_INPROCESS=on SAT_PROBE=on SAT_VIVIFY=on SAT_HBR=off
```

Track:

```text
probe_attempts
probe_failed_lits
probe_units
probe_ticks
root_units_added
```

## 2.5 Hyper-binary resolution during probing

### Goal

Generate useful binary clauses from probing implications and long-clause reasons.

### Dependencies

2.4, 1.6, 2.1a.

### Code-level changes

During probe of literal `l`, when propagation implies `m`:

```text
If m's reason is a long clause and all but one of the other literals were set by the probe context:
  learn binary (¬l ∨ m)
```

Store generated binaries in a deduplicated buffer and commit after the temporary probe context is backtracked to root.

Commit generated binaries through the formula-edit transaction layer. Each generated binary receives a stable `BinaryClauseId`, proof state, and binary implication edges. Duplicate HBR edges must be discarded before proof logging.

Proof:

```text
record_add_clause(binary) through formula-edit transaction before binary insertion
```

Data structures:

```rust
hbr_seen: Vec<u32>,
hbr_stamp: u32,
hbr_added_buffer: Vec<(i32, i32)>,
```

### Tests

```text
test_hbr_adds_binary_clause
test_hbr_adds_unit_clause
test_hbr_emits_binary_when_only_probe_lit_in_reason
test_hbr_does_not_add_duplicate_binary
test_hbr_binary_clause_propagates
test_hbr_proof_records_binary
test_hbr_generated_binary_has_stable_clause_id
test_hbr_commit_uses_formula_edit_transaction
```

### Benchmark gate

Track:

```text
hbr_added_binary
binary_props
long_props
search_conflicts
watch_list_lengths
```

---

## 2.6 BVE cost model and inprocessing scheduling upgrade

### Goal

Reuse existing BVE/BSR machinery under budgeted recurring inprocessing.

### Dependencies

2.0, 2.1, 2.1a, 2.1b, 2.2.

### Code-level changes

Split existing elimination into:

```rust
pub(super) fn eliminate(...) -> EliminateResult;
pub(super) fn inprocess_eliminate(budget: Budget, limits: InprocessLimits) -> EliminateResult;
```

Formula mutation rule:

```text
- generated resolvents, deleted original clauses, strengthened clauses, and model-extension entries go through FormulaEditTxn
- direct occurrence/watch/proof mutation inside BVE is not allowed after 2.1a
```

Budget:

```text
eliminate_ticks_budget = 500_000 initially
count occurrence scans, resolvent attempts, strengthening checks, propagation steps
```

Cost model:

```text
- exact product pos.len() * neg.len() only after dirty occurrence cleanup
- tautological resolvents do not count as growth
- non-gate BVE keeps bve_grow=0 initially
- variables touched by recent probing/HBR get priority
- never eliminate variables required for model output without extension entry
```

### Tests

```text
test_inprocess_eliminate_respects_tick_budget
test_inprocess_eliminate_runs_on_second_call
test_bve_cost_skips_expensive_var
test_bve_tautological_resolvent_not_counted
test_bve_elimination_adds_resolvents
test_bve_model_extension_satisfies_original
test_extension_stack_replay_stepwise_check
test_extension_stack_memory_budget_disables_bve_candidate
test_bve_proof_logs_resolvents
test_inprocess_eliminate_preserves_heap_decision_flags
```

### Benchmark gate

Run preprocess-core. Watch K4/Kakuro-like cases and large occurrence-list/full-BSR cases.

---

## 2.7 Reserved: Equivalent literal substitution is parked

ELS is outside the main Phase 2 target path. Keep the design in Appendix A.3 and implement it only after milestone triage identifies binary-SCC equivalence substitution as the next bottleneck. Do not expose `SAT_ELS` or `SolverConfig::els` before that unpark task.

---

## 2.8 Transitive reduction over binary implication graph

### Goal

Remove redundant binary clauses after HBR and selected binary-producing passes to shrink watch lists and improve propagation.

### Dependencies

2.5, 2.1a, 2.1b, 2.2.

### Code-level changes

New module:

```text
src/transitive.rs
```

For implication edge `a -> b`, if there is an alternate path `a -> c -> ... -> b` within depth/tick budget, remove direct binary `(¬a ∨ b)`.

Budget:

```text
max_depth
max_ticks_per_source
max_removed_per_round
```

Proof:

```text
record_delete_clause(¬a ∨ b) through FormulaEditTxn and mark the BinaryClauseId deleted
```

### Tests

```text
test_transitive_removes_direct_binary_when_indirect_path_exists
test_transitive_does_not_remove_only_path
test_transitive_respects_depth_budget
test_transitive_updates_binary_implications
test_transitive_marks_binary_clause_id_deleted
test_transitive_proof_delete_uses_formula_edit_transaction
```

### Benchmark gate

Track binary implication-list sizes and propagation rate. Accept if watch traffic improves without correctness regression.

---

## 2.9 BSR/subsumption hot-path pass and forward sweep

### Goal

Improve known preprocessing hot spots after BVE and gate-aware BVE have shaped the formula, without repeating unmeasured “obvious” optimizations.

### Dependencies

2.6, 2.11.

### Keep existing proven principles

```text
- u32 occurrence refs
- manual dirty-list compaction
- inline original abstractions only on large formulas
- sorted relation only under proven gate
- lazy membership cleanup only under proven large-inline path
- strict detach where needed
```

### Add measured changes only

1. Clause-local literal stamps for candidate scans.
2. Driver-literal ordering by occurrence length.
3. Budgeted occurrence cleaning per round.
4. Optional arena-offset ordering only if benchmark proves it.
5. Forward subsumption sweep sorted by clause length, budgeted.

Helper:

```rust
fn subsumes_driver_candidate_fast(
    &mut self,
    driver_idx: usize,
    candidate_idx: usize,
    allowed_missing: usize,
) -> SubsumptionOutcome
```

Use abstraction first:

```text
if driver_abstraction & !candidate_abstraction != 0:
    reject
```

Then choose sorted two-pointer path or stamp path based on canonical sorting gate.

### Tests

```text
test_subsumption_exact
test_subsumption_strengthen_one_literal
test_subsumption_rejects_abstraction_mismatch
test_sorted_and_unsorted_relation_agree
test_strengthen_updates_occurrence_counts
test_forward_subsumption_drops_long_clause_subsumed_by_short
test_forward_subsumption_respects_budget
```

### Benchmark gate

Preprocess-core first. Do not merge any hot-path optimization unless it wins K4/Kakuro or at least does not regress them.

---

## 2.10 Gate extraction

### Goal

Detect simple gate patterns and store them as hints for gate-aware BVE before broad ELS rewrites can obscure circuit structure.

### Dependencies

2.2, 2.6.

### Code-level changes

New module:

```text
src/gates.rs
```

Detect:

```text
AND gate:
  (¬v ∨ a1), ..., (¬v ∨ ak), (v ∨ ¬a1 ∨ ... ∨ ¬ak)
  record v ↔ AND(a1,...,ak)

ITE gate:
  detect standard 3/4-clause ITE patterns conservatively

Equivalence:
  two binaries v ↔ a
```

Store in side table:

```rust
enum GateDef {
    And { output_lit: i32, inputs: Vec<i32> },
    Ite { output_lit: i32, cond: i32, then_lit: i32, else_lit: i32 },
    Equiv { a: i32, b: i32 },
}
```

No semantic change yet.

### Tests

```text
test_gate_detects_simple_and_gate
test_gate_detects_negated_output_and_gate
test_gate_detects_ite_pattern
test_gate_model_replay_preserves_output_polarity
test_gate_detects_equivalence_pattern
test_gate_table_ignores_incomplete_pattern
```

### Benchmark gate

No semantic change. Discriminating PAR-2 should be noise only. Track extraction time and gate counts.


---

## 2.10a Local implied-clause and RUP checker scaffold

### Goal

Provide the minimal checker needed by gate-aware BVE envelope validation without turning RCheck into a broad simplification feature yet.

### Dependencies

2.3, 2.4, 2.6, 1.0a.

### Code-level changes

Add:

```rust
struct RCheckWitness {
    assumptions: Vec<i32>,
    conflict: Conflict,
}

enum ImpliedCheckResult {
    Implied(RCheckWitness),
    NotImplied,
    BudgetExhausted,
}

fn check_clause_implied_for_envelope(
    &mut self,
    lits: &[i32],
    budget: &mut Budget,
) -> ImpliedCheckResult;
```

Rules:

```text
- uses `with_temporary_assumptions` and temporary accounting only
- always backtracks to root before returning
- never updates saved/target/best phase
- never updates VMTF/heap/restart statistics
- returns BudgetExhausted as unsafe for deletion
- may be used only for invariant-mode validation and gate-aware BVE envelope witnesses until 2.12
```

### Tests

```text
test_local_implied_check_detects_implied_clause
test_local_implied_check_budget_exhaustion_is_not_implied
test_local_implied_check_restores_root_trail
test_local_implied_check_does_not_update_search_stats
```

### Benchmark gate

No promotion based on this task. It is an infrastructure dependency.

---

## 2.11 Gate-aware BVE

### Goal

Use gate definitions to avoid BVE cross-product blowups on circuit-encoded variables.

### Dependencies

2.10, 2.10a, 2.6, 2.1a, 2.1b.

### Code-level changes

Add reusable envelope type:

```rust
struct EliminationEnvelope {
    var: usize,
    cleaned_occurrences: bool,
    gate_clauses: Vec<ClauseRef>,
    consumer_clauses: Vec<ClauseRef>,
    generated_resolvents: Vec<Vec<i32>>,
    skipped_resolvents: Vec<SkippedResolvent>,
    model_extension: ExtensionEntry,
}

enum SkippedResolvent {
    Tautological { left: ClauseRef, right: ClauseRef },
    Subsumed { resolvent: Vec<i32>, subsumer: ClauseRef },
    Implied { resolvent: Vec<i32>, witness: RCheckWitness },
}
```

Rules:

```text
- Gate-aware BVE must construct and validate an EliminationEnvelope before deleting any occurrence of the eliminated variable.
- Every skipped resolvent must have a SkippedResolvent witness.
- In invariant mode, every Implied witness is rechecked using RCheck or local RUP before commit.
- Later Appendix features that eliminate or substitute variables must reuse EliminationEnvelope or define a stricter replacement.
```

In `try_eliminate_var`, when `var` has a recorded gate definition:

```text
- first run a complete occurrence-envelope check:
    * every occurrence of var is either part of the gate definition,
      a consumer clause whose resolvents are generated,
      or a clause whose skipped resolvents are tautological/subsumed/implied with proof justification
    * dirty occurrence lists must be cleaned before this check
    * incomplete, duplicate, tautological, or stale gate definitions force fallback to ordinary BVE or skip
- generate the complete proof-sufficient resolvent set for the verified envelope
- target resolvent count should be O(inputs.len()) only when the envelope check proves that non-gate cross-products are unnecessary
- keep bve_grow=0 for non-gate eliminations initially
- proof logs all generated resolvents and deletions through FormulaEditTxn
- model extension records eliminated variable relation before destructive deletion
```

### Tests

```text
test_elimination_envelope_requires_clean_occurrences
test_elimination_envelope_rejects_unwitnessed_skipped_resolvent
test_elimination_envelope_rechecks_implied_witness_in_invariant_mode
test_gate_aware_bve_eliminates_and_gate_with_zero_grow
test_gate_aware_bve_does_not_use_incomplete_gate
test_gate_aware_bve_falls_back_on_extra_non_gate_occurrence
test_gate_aware_bve_generates_all_required_consumer_resolvents
test_gate_aware_bve_skipped_resolvent_must_be_tautological_or_proved
test_gate_aware_bve_cleans_dirty_occurrences_before_envelope_check
test_gate_aware_bve_model_extension_satisfies_original
test_gate_aware_bve_proof_records_resolvents
test_gate_aware_bve_uses_formula_edit_transaction
```

### Benchmark gate

Especially watch circuit and bp4 instances. This should be a meaningful preprocess-core win if gate patterns are present.

---

## 2.12 RCheck implied-clause checks

### Goal

Implement conservative implied-clause checks for use in simplification, without breaking proof/model semantics or polluting search state.

### Dependencies

2.10a, 2.3, 2.4, 2.6, 1.0a.

### Code-level changes

```rust
fn clause_implied_by_current_formula(&mut self, lits: &[i32], budget: usize) -> bool {
    self.backtrack_to_root();

    let mut budget = Budget::from_ticks(budget as u64);
    self.with_temporary_assumptions(
        TemporaryAssumptionOptions::rcheck(),
        |ctx| {
            let mut implied = false;

            for &lit in lits {
                if budget.exhausted() { break; }

                if ctx.enqueue(neg(lit)).is_conflict() {
                    implied = true;
                    break;
                }

                if ctx.propagate_budgeted(&mut budget).is_some() {
                    implied = true;
                    break;
                }
            }

            if !implied && !budget.exhausted() {
                implied = ctx.propagate_budgeted(&mut budget).is_some();
            }

            implied && !budget.exhausted()
        }
    )
}
```

Temporary-assumption rules:

```text
- use the 1.0a `with_temporary_assumptions` API
- do not update saved/target/best phase
- do not update VMTF or heap decision statistics
- do not update restart averages
- do not count assumptions as normal decisions
- return false on budget exhaustion
```

Safe first uses:

```text
- skip adding redundant generated strengthening clauses
- skip vivification candidates already implied
```

Do not initially use:

```text
- skipping BVE resolvents required for elimination proof/model correctness
```

### Tests

```text
test_rcheck_detects_implied_clause
test_rcheck_rejects_non_implied_clause
test_rcheck_backtracks_to_root
test_rcheck_budget_exhaustion_returns_false
test_rcheck_does_not_update_search_phase_or_restart_stats
```

### Benchmark gate

Track:

```text
rcheck_attempts
rcheck_implied
rcheck_props
resolvents_skipped
```

Default off until proof behavior is fully verified.

## 2.13 Reserved: BCE is parked

Blocked Clause Elimination is outside the main Phase 2 target path. Keep the design in Appendix A.4 and implement it only after milestone triage proves model reconstruction and proof deletion are boringly reliable. Do not expose `SAT_BCE` or `SolverConfig::bce` before that unpark task.

---

## 2.14 Phase 2 infrastructure certification gate

### Goal

Certify that proof deletion, watch rebuild, FormulaEditTxn, model-extension replay, and inprocessing scheduling are stable before evaluating broad feature mixes.

### Dependencies

2.0, 2.1, 2.1a, 2.1b, 2.2, 0.6, 0.9.

### Required validation

```bash
bash tools/ci_solver11_fast.sh
bash tools/ci_solver11_matrix.sh
bash tools/ci_solver11_proof_model.sh
SAT_CHECK_INVARIANTS=on bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/preprocess-core solver/11-kissat-search
```

### Acceptance

```text
- FormulaEditTxn replay passes on generated formulas
- proof temp lifecycle passes SAT, UNSAT, UNKNOWN, and parse-failure cases
- model-extension replay passes every enabled destructive-edit variant
- no direct formula-rewrite pass calls proof.record_* outside FormulaEditTxn
- no direct formula-rewrite pass marks clauses deleted outside FormulaEditTxn
- PROOF_OBLIGATIONS.md covers every enabled formula-edit kind
```

---

## 2.15 Phase 2 feature retirement and candidate pruning

### Goal

Remove or park weak Phase 2 features before composing final candidate profiles.

### Dependencies

2.3 through 2.12, 2.14.

### Acceptance

```text
- each Phase 2 feature has a keep/tune/park decision
- each kept feature has at least one named benchmark-family win or proof/model necessity
- any feature with only noise-level gains remains experimental
- candidate profiles contain only kept features
- parking decisions are recorded in log/phase2/triage.md
- parked implemented features have either:
    * code retained with tests and FEATURES.csv maturity=ParkingLot, or
    * code deleted with a rollback note and archived validation artifact
- no parked feature remains in CONFIG_SCHEMA.csv as a promoted-profile default
```

---

## 2.16 Inprocessing candidate configurations and Phase 2 gate

### Dependencies

2.15.

Candidate A: conservative simplification

```bash
SAT_PROFILE=experimental
SAT_PROOF=drat
SAT_SEED=0
SAT_INPROCESS=on
SAT_VIVIFY=on
SAT_PROBE=off
SAT_HBR=off
SAT_GATE_EXTRACT=off
SAT_GATE_BVE=off
SAT_TRANSITIVE=off
SAT_FORWARD_SUBSUME=off
SAT_RCHECK=off
SAT_CONFIG_OUT=log/phase2/candidate-a.config
```

Candidate B: probing/HBR

```bash
SAT_PROFILE=experimental
SAT_PROOF=drat
SAT_SEED=0
SAT_INPROCESS=on
SAT_VIVIFY=on
SAT_PROBE=on
SAT_HBR=on
SAT_GATE_EXTRACT=off
SAT_GATE_BVE=off
SAT_TRANSITIVE=off
SAT_FORWARD_SUBSUME=off
SAT_RCHECK=off
SAT_CONFIG_OUT=log/phase2/candidate-b.config
```

Candidate C: gate-aware preprocessing

```bash
SAT_PROFILE=experimental
SAT_PROOF=drat
SAT_SEED=0
SAT_INPROCESS=on
SAT_VIVIFY=on
SAT_PROBE=on
SAT_HBR=on
SAT_GATE_EXTRACT=on
SAT_GATE_BVE=on
SAT_TRANSITIVE=off
SAT_FORWARD_SUBSUME=off
SAT_RCHECK=off
SAT_CONFIG_OUT=log/phase2/candidate-c.config
```

Candidate rule:

```text
- candidate configs must use SAT_PROFILE=experimental until promotion
- candidate configs must be complete enough that `env -i` plus the listed SAT_* variables reproduces the run
- if 2.15 keeps transitive reduction, forward subsumption, or RCheck, create additional named candidate replay files with concrete `on`/`off` values rather than editing these snippets in place
- every candidate must produce a SAT_CONFIG_OUT replay file
- candidate comparison must use SAT_CONFIG_REPLAY, not hand-retyped env snippets, for promotion reruns
```

Candidate D is removed from the main Phase 2 gate. Aggressive ELS/BCE mixes belong in Appendix A after explicit triage.

Promotion rule:

```text
Default on only if:
  - brute-force oracle passes all enabled simplification variants
  - smoke proofs verify
  - tools/ci_solver11_proof_model.sh passes for the promoted profile
  - SAT assignments satisfy original formula
  - profiling solved count or PAR-2 improves
  - search-core does not regress badly from simplification perturbation
  - regression-guards preserve previously solved instances
```

Profile-specific promotion:

```text
Candidate A can promote into `default.preprocess=Conservative` only after its replay file passes promotion gates.
Candidate B can promote into `default.preprocess=Conservative` only if HBR proof/model behavior is verified and its replay file passes promotion gates.
Candidate C can promote into `fast.preprocess=GateAware` before becoming baseline only after its replay file passes promotion gates.
Appendix features never join baseline until they have their own proof/model/full-set promotion gate.
```

Phase 2 full gate:

```bash
bash tools/bench.sh -t 1800 -m 16384 \
  -d benchmarks/sat-comp-2025 solver/11-kissat-search
```

Target direction:

```text
- approach Kissat-class performance on the discriminating set
- reduce SC-2025 PAR-2 toward 110_000 or better, subject to measured noise
- if ELS/BCE/walking/optional sweep do not earn their place through milestone triage, leave them parked rather than forcing implementation
```

## 2.16a Adaptive inprocessing ROI scheduler

### Goal

Adapt inprocessing pass frequency and budget only after deterministic scheduling has produced enough counter data to justify it.

### Dependencies

2.16 plus milestone triage showing deterministic scheduling overhead or poor pass ordering.

### Scope

This is not part of the initial Phase 2 path. It may use pass credits, last_ticks, and last_value fields only after vivification, probing/HBR, BVE scheduling, gate extraction, and BSR/subsumption have stable counters.

### Acceptance

```text
- adaptive scheduling changes frequency and budget, not correctness behavior
- deterministic mode remains selectable and reproducible
- config_hash records whether adaptive scheduling is enabled
- benchmark deltas are reported separately from pass implementation deltas
```

---

# 6. Milestones

The numbers below are directional targets, not absolute promises. Use them to catch obvious stagnation or regressions.

| Milestone | After tasks | Target full-set PAR-2 direction | Target discriminating direction |
|---|---|---:|---:|
| Baseline | 0.x | reference | reference |
| LBD + tiered reduce + DB/GC policy | 1.1–1.3a | meaningful drop | roughly −10% |
| EMA + phase + binary | 1.4–1.8 | larger drop | roughly −20% to −25% |
| Mode/VMTF/minimize/rephase + advanced candidates | 1.9–1.12a | SAT-side gains | roughly −25% to −35% |
| Guarded chrono + throughput | 1.13–1.15 | class-specific gains | no regression, selective wins |
| Vivification + FLP + HBR | 2.0–2.5 | major UNSAT/preprocess gains | roughly −40% to −45% |
| BVE scheduling + gate extraction + local checker + gate-aware BVE | 2.6, 2.10–2.11 | targeted circuit/preprocess shrink | roughly −45% to −55% |
| BSR/subsumption + selected binary rewrites | 2.9, 2.8, selected 2.12 | close remaining preprocess gaps | within striking distance of Kissat on targeted classes |
| Default hardening | after 1.15 or 2.16 baseline-candidate | no new speed target | no correctness, replay, proof/model, or documentation gaps |

If these targets do not move, inspect counters before adding more features.

Milestone triage requirement:

```text
After every milestone, write `log/<milestone>/triage.md` with:
  - top 10 regressions
  - top 10 wins
  - lost solved instances
  - newly solved instances
  - status/proof/model failures
  - dominant bottleneck category per hard instance:
      search-trajectory
      propagation-throughput
      learned-clause-quality
      restart/phase-policy
      preprocessing-shrink
      occurrence-list-cost
      proof-throughput
      memory/GC
      benchmark-noise
  - recommended next two beads tasks
  - tasks to defer or delete
  - explicit keep/tune/revert decision for every feature added since the previous milestone
  - profile changes accepted or rejected
  - config flags to remove from candidate profiles
  - holdout result summary
  - whether observed PAR-2 delta exceeds measured noise/confidence band
```

Do not start a new major feature family after a milestone miss until triage identifies why the previous family missed and makes a keep/tune/revert decision for each feature in the missed milestone.

If a feature remains experimental for two milestones without a named benchmark-family win, remove it from the active DAG and move it to Appendix A.

Feature retirement rule:

```text
When a feature is parked after implementation:
  - remove it from non-experimental profile defaults
  - mark FEATURES.csv maturity=ParkingLot
  - keep or remove the code based on maintenance cost:
      * keep code only if tests still run and the feature remains useful for diagnostics
      * delete code if it creates capability, proof, model, or config complexity
  - remove runnable env snippets from README
  - keep a replay artifact showing why it was parked
  - add a reactivation condition naming the benchmark family or counter signal needed
```

---

# 7. Dependency-respecting DAG summary

```text
0.0 fork baseline and BASELINE_LOCK.raw.txt
  -> 0.0b thin-slice vertical: minimal LBD + shim tooling, 5-instance smoke
  -> 0.1 architecture boundary and ownership map
  -> 0.2 source map
  -> 0.3 config flags, profiles, validation, proof policy, replay, and limits
  -> 0.3a minimal status and result-file schema
  -> 0.4 stats/trace
  -> 0.5 benchmark sets and comparison tooling
  -> 0.5a profiling and hot-path observability
  -> 0.0a rich baseline comparison after benchmark tooling exists
  -> 0.6 brute-force/metamorphic/differential oracle and formula-edit replay placeholder
  -> 0.8 parser/output/proof-temp/limit contract
  -> 0.9 local CI and feature-interaction matrix
  -> non-task default-profile hardening gate before baseline promotion

Phase 1:
0.1 + 0.3 + 0.6 -> 1.0 reason/propagation scaffold
1.0 + 0.3 + 0.6 -> 1.0a temporary-assumption context
1.0 + 0.3 -> 1.7 decision heap cleanup
1.0 + 0.3 + 0.4 -> 1.1 LBD metadata
1.1 -> 1.2 LBD-aware analysis updates
1.1 + 1.2 -> 1.3 LBD-tiered reduce
1.3 -> 1.3a clause database budget and GC policy
1.1 -> 1.4 EMA restart
1.7 + 1.1 + 0.3 -> 1.5 saved/target/best phase
1.0 + 1.1 + 1.3a + 0.6 -> 1.6 binary fast path
1.1..1.7 -> 1.8 core search default candidate
1.4 + 1.5 + 1.7 -> 1.9 focused/stable + reluctant
1.9 -> 1.10 VMTF queue
1.1 + 1.3 + 1.4 -> 1.11 minimization/shrink/bumping
1.5 + 1.9 -> 1.12 rephase hook
1.8 + selected 1.9 + 1.10 + 1.11 + 1.12 -> 1.12a advanced search candidate, optional
1.1 + 1.7 + 1.11 -> 1.13 guarded chrono
1.3a + 1.6 + 1.9 -> 1.14 throughput pass
1.8 + selected 1.9..1.14 + optional 1.12a -> 1.15 Phase 1 gate

Phase 2:
1.15 + 0.6 -> 2.0 deterministic inprocess scheduler
2.0 -> 2.1 proof deletion/audit and proof completeness state
2.1 -> 2.2 compaction/watch rebuild and GC rewrite policy
2.1 + 2.2 -> 2.1a formula-edit transaction layer
2.1a + 0.6 -> 2.1b model-extension replay contract
2.0 + 2.1 + 2.1a + 2.1b + 2.2 + 1.0a -> 2.3 vivification
2.0 + 2.1 + 2.1a + 2.1b + 2.2 + 1.0a -> 2.4 failed-literal probing
2.4 + 1.6 + 2.1a -> 2.5 HBR
2.0 + 2.1 + 2.1a + 2.1b + 2.2 -> 2.6 BVE scheduling
2.2 + 2.6 -> 2.10 gate extraction
2.3 + 2.4 + 2.6 + 1.0a -> 2.10a local implied-clause/RUP checker scaffold
2.10 + 2.10a + 2.6 + 2.1a + 2.1b -> 2.11 gate-aware BVE
2.6 + 2.11 -> 2.9 BSR/subsumption hot path
2.5 + 2.1a + 2.1b + 2.2 -> 2.8 transitive reduction
2.10a + 2.3 + 2.4 + 2.6 + 1.0a -> 2.12 full RCheck
Appendix A.1 walking rephaser -> parking lot, outside main DAG
Appendix A.2 optional sweep -> parking lot, outside main DAG
Appendix A.3 equivalent literal substitution -> parking lot, outside main DAG
Appendix A.4 BCE diagnostic -> parking lot, outside main DAG
2.0 + 2.1 + 2.1a + 2.1b + 2.2 + 0.6 + 0.9 -> 2.14 Phase 2 infrastructure certification
2.3 + 2.4 + 2.5 + 2.6 + 2.8 + 2.9 + 2.10 + 2.11 + selected 2.12 + 2.14 -> 2.15 feature retirement and candidate pruning
2.15 -> 2.16 Phase 2 gate
2.16 + milestone triage -> 2.16a adaptive inprocessing ROI scheduler, optional
```

# 8. How to run every time

After implementing a beads node:

```bash
bash tools/ci_solver11_fast.sh
```

When a task changes interactions among search, proof, or inprocessing features, also run:

```bash
bash tools/ci_solver11_matrix.sh
```

The expanded manual equivalent is:

```bash
cd solver/11-kissat-search
cargo test

cd ../..
bash tools/smoke_test.sh solver/11-kissat-search
SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-search
bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/smoke-plus solver/11-kissat-search
```

For search nodes:

```bash
bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/search-core solver/11-kissat-search \
  --log-dir log/tasks/<task-id>-<slug>/search-core
```

For simplification nodes:

```bash
bash tools/bench.sh -t 120 -m 16384 \
  -d benchmarks/iteration/preprocess-core solver/11-kissat-search \
  --log-dir log/tasks/<task-id>-<slug>/preprocess-core
```

For milestone nodes:

```bash
bash tools/bench.sh -t 300 -m 16384 \
  -d benchmarks/discriminating solver/11-kissat-search
```

Compare benchmark runs:

```bash
python3 tools/compare_bench.py \
  --before log/<prior>/results.csv \
  --after log/<this>/results.csv \
  --baseline benchmarks/iteration/baseline.csv \
  --timeout 120
```

End of phase:

```bash
bash tools/bench.sh -t 1800 -m 16384 \
  -d benchmarks/sat-comp-2025 solver/11-kissat-search
```

---

## 8.1 Risk register, sentinel tests, and rollback triggers

Every beads task that touches a risk area must name the applicable risk IDs.

| Risk ID | Risk | Sentinel tests / counters | Rollback trigger | Mitigation |
|---|---|---|---|---|
| R1 | SAT model reconstruction failure | brute-force oracle, metamorphic tests, original-CNF model check | any SAT assignment fails original CNF | disable transformation or add extension entry |
| R2 | UNSAT proof invalidity | smoke proofs, generated UNSAT fuzz, proof add/delete unit tests | any proof verification failure | revert or keep feature proof-off only |
| R3 | stale watcher reads deleted/rewritten clause | invariant mode, watcher rebuild tests, propagation differential tests | panic, wrong status, or stale watcher counter explosion | use formula-edit transaction and rebuild watchers |
| R4 | reason corruption after GC or binary deletion | reason expansion tests, conflict-analysis oracle, GC reference rewrite tests | reason points to deleted/moved clause | centralize ReasonRef rewrite |
| R5 | non-decision variable re-enters heap/VMTF | heap/VMTF decision eligibility tests | eliminated/frozen var selected | single decision_var gate on all insertion paths |
| R6 | benchmark overfit | regression-guards, category PAR-2, paired benchmark report | lost solved instance or category regression beyond noise | keep feature off by default or profile-specific |
| R7 | memory blowup | learned_lits_final, original_lits_after_preprocess, RSS if available | memory or extension stack grows faster than solved-count/PAR-2 benefit | add budget/GC threshold, extension-memory cap, or revert |
| R10 | binary-clause explosion | binary_clauses_final, binary_implication_edges_final, binary_stale_skips | HBR/transitive/gate passes add many binaries without propagation/search benefit | add binary budget, stronger dedup, or keep HBR/transitive off |
| R11 | extension-stack blowup | extension_entries, extension_bytes_estimated, model_replay_steps | model reconstruction dominates SAT output or memory | disable destructive simplification by extension budget |
| R12 | proof-size blowup | proof_bytes_written, proof_added_literals, proof_deleted_literals | proof-on profile regresses while proof-off wins | keep feature proof-off/experimental or adjust proof logging/buffering |
| R8 | nondeterminism masks regressions | fixed SAT_SEED, config_hash, repeated runs, ci_reproducibility.sh | same config produces different status or large unexplained deltas | isolate randomness and log seed |
| R9 | proof throughput dominates runtime | proof_sec, proof_bytes_written, proof_flushes | proof-on benchmarks collapse while proof-off is fine | adjust buffering only after measurement |
| R13 | proof-checker version drift | proof_checker, proof_checker_version in JSON_STATS; checker SHA pinned in BASELINE_LOCK.txt | proof verification disagrees across `drat-trim`/`lrat-check` versions, or local-vs-CI checker output diverges | pin checker binaries (SHA + commit) and record `proof_checker_version` on every proof-on run; refuse to promote when CI cannot reproduce the local checker verdict |

Rollback note template:

```text
Task:
Risk IDs:
Observed failure/regression:
Counters:
Instances:
Decision:
Follow-up idea:
```

---

# Appendix A. Parking-lot experiments

The following experiments are intentionally minimal — design notes only. Their goal is to preserve the intent and the unpark trigger, not to pre-bake the implementation. When milestone triage unparks one of these, the responsible task rewrites the design with full implementation detail (algorithm, data structures, tests, proof obligations, model reconstruction) at that time, with the benefit of having seen Phase 2's actual counter data.

## Appendix A.1 Walking local-search rephaser

- **Intent**: WalkSAT-style local search over original clauses to escape SAT-trajectory ruts.
- **Unpark trigger**: Phase 2 SAT-heavy categories (Timetable/battleship-class) still trail local Kissat by ≥30% with phase-diversification counters showing low best_phase coverage.
- **Risk**: SAT-only; can starve UNSAT progress if scheduled poorly.
- **Dependencies on unpark**: 1.12, 0.6, plus a new task in section 2.x that re-derives algorithm, data structures, tests, proof obligation row, and model reconstruction.

## Appendix A.2 Optional bounded sweep

- **Intent**: bounded DPLL on small variable cones to detect backbones/equivalences without embedding a second solver.
- **Unpark trigger**: Phase 2 leaves backbone-rich instances (e.g., bp4-class) ≥3× slower than local Kissat with probe-failed-lits saturated.
- **Risk**: cone expansion can blow up; bounded version may not pay for itself.
- **Dependencies on unpark**: independent of Appendix A.3 unless triage decides ELS is the right vehicle.

## Appendix A.3 Equivalent literal substitution (ELS)

- **Intent**: detect binary-implication SCCs and substitute representative literals.
- **Unpark trigger**: binary implication graph shows ≥10% SCC density after HBR/transitive on the discriminating set, AND simplification residual size is the dominant cost.
- **Risk**: model reconstruction is subtle; requires a new ExtensionEntry variant (e.g., `EquivalentLiteral { removed_lit, representative_lit, polarity }`) and oracle coverage; can interact badly with gate extraction if run first.
- **Dependencies on unpark**: 2.5, 2.1, 2.1a, 2.2, 0.6.

## Appendix A.4 Blocked Clause Elimination (BCE)

- **Intent**: delete clauses blocked by a literal whose resolvents are tautological.
- **Unpark trigger**: profile evidence that gate-aware BVE leaves ≥5% of original clauses removable by BCE on circuit-heavy instances, and FormulaEditTxn + extension replay have ≥1 month of clean milestone runs.
- **Risk**: SAT model reconstruction does not always preserve original-clause satisfaction; needs a `BlockedClause { clause, blocking_lit }` ExtensionEntry variant with witness-flip semantics and oracle coverage. BCE must stay disabled for SAT-output runs until reconstruction is proven.
- **Dependencies on unpark**: 2.1, 2.1a, 2.2, 0.6.

## Unpark process

1. Milestone triage names the parked feature and the counters justifying the unpark.
2. A new task in section 2.x is opened; it re-derives the design (algorithm, structures, tests, proof obligation row, model reconstruction).
3. The new task is not allowed to copy designs from this appendix verbatim; counter data must shape the design.
4. The unparked feature begins at maturity=Experimental and follows the standard promotion sequence.
5. The Appendix entry above is then rewritten to point at the new task, not the other way around.

---

# 9. Rationale index

For background decisions, rejected alternatives, and original-plan synthesis notes, see:

```text
docs/solver11-plan-rationale.md
```

The following sections were moved there:

```text
- What each original plan contributed
- Clearly good changes in this synthesis
- Debatable choices
- Pushbacks
```

Only sections 1 through 8, Appendix A, and section 13 define executable planning rules. The moved rationale is non-authoritative unless this PLAN.md references a rule directly.

---

# 13. Revised high-level sequence and deviation policy

The revised project sequence is:

```text
0. fork baseline, write BASELINE_LOCK.raw.txt, then generate BASELINE_LOCK.txt after benchmark tooling exists
1. extract architecture boundaries without behavior change
2. harden config, config replay, stats, parser/output, typed output contracts, limits, local CI, and benchmark comparison
3. create benchmark manifests, proof-off/proof-on scorecards, profiling scripts, and hot-path observability
4. add brute-force + metamorphic + differential testing plus formula-edit replay harness placeholder
5. create reason/binary-clause scaffold and closure-based temporary-assumption context
6. fix heap/decision eligibility and deterministic tie-breaking
7. implement LBD, learned DB policy, clause database/GC policy, EMA restarts, and phase policies
8. enable binary fast path using stable binary IDs
9. evaluate Phase 1 profiles
10. add proof deletions, proof completeness state, proof-obligation matrix, watch/GC rebuild policy, debug generation checks, ModelEffect classification, and two-phase buffered/preflighted formula-edit transactions
11. define the model-extension journal and replay contract before destructive simplification
12. add vivification + probing/HBR
13. add gate extraction, local implied-clause/RUP checking, and soundness-checked gate-aware BVE before any ELS experiment
14. improve BSR/subsumption, transitive reduction, RCheck, and selected binary rewrites
15. certify Phase 2 infrastructure, retire weak features, and compare replayable candidate profiles
16. keep ELS, BCE, walking, and sweep parked unless triage proves they are the next bottleneck
```

The plan is a strong starting point, not a prison.

Allowed deviations:

```text
- Reorder tasks if dependencies are fake and the new order has a written reason.
- Skip optional tasks that do not pay for themselves.
- Add a missing Kissat/CaDiCaL-style feature when a benchmark class reveals a gap.
- Pair prerequisite tasks with dependent tasks when the prerequisite is not expected to win alone.
- Update benchmark sets when new logs reveal better representatives.
```

Required when deviating:

```text
- update the DAG
- update feature flags and task dependencies
- add or update tests
- record why the deviation was made
- preserve proof/model/watcher invariants
- update profile-promotion and risk-register notes when applicable
```

End of synthesized plan.
