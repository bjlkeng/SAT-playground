# Choosing a base solver for an ML/RL inprocessing scheduler

**Purpose.** We want to replace the *decision layer* of a CDCL SAT solver —
when to fire inprocessing, restarts, clause-DB cleanup, rephasing/walking,
and how much budget each pass gets — with a learned (RL) policy, while
keeping the *mechanisms* (propagation, conflict analysis, the passes
themselves) fixed. This report gives plain-language pseudo-code of both
candidates, an inventory of the decision points a policy would own, and a
recommendation. Details that don't affect the scheduling question are
omitted on purpose.

Context that matters for the choice (measured on this host, 3600 s /
16 GB / 32 cores):

| | sat-comp-2025 (our tuning suite) | sat-comp-2026 (holdout) |
|---|:--:|:--:|
| solver12 | 296/400 | 160/400 |
| kissat 4.0.4 | 292/400 | 197/400 |

solver12's lead is 2025-fitted (engineered family engines + hand-tuned
bands); kissat's *general-purpose* search+inprocessing is stronger out of
distribution. An RL scheduler is precisely an attempt to win generality —
that asymmetry weighs heavily below.

---

## 1. The shared skeleton

Both solvers are CDCL and share this shape. Everything marked `DECIDE` is
a scheduling decision a policy could own; everything else is mechanism.

```
solve(formula):
    preprocess(formula)                        # one-shot simplification
    loop:
        conflict = propagate()
        if conflict:
            if at root level: return UNSAT
            learn clause, backjump             # mechanism (1-UIP etc.)
            DECIDE: restart now?               # fires every conflict
            DECIDE: reduce (clean) learned DB now?
            DECIDE: run inprocessing now?      # the big one
        else:
            if all variables assigned: return SAT
            DECIDE: switch search mode?        # focused <-> stable
            DECIDE: rephase / local-search walk now?
            pick decision variable & polarity  # mechanism (scores, phases)
```

The two solvers differ almost entirely in **how the DECIDE lines are
implemented** and in **how strong the mechanisms behind them are**.

---

## 2. kissat: pseudo-code of the decision layer

kissat's philosophy: *run everything, everywhere, on simple global
schedules; control cost with budgets, not with eligibility tests.*

```
# --- inprocessing: two bundles on conflict-count timers ---------------
after each conflict:
    if conflicts > probe_limit:                # timer grows ~ N*log(N)
        probe():                               # one bundle, fixed order:
            repeat while active vars keep dropping:
                congruence -> substitute -> backbone -> vivify
                -> sweep -> substitute -> transitive -> factor
        probe_limit = conflicts + next_interval()
    if conflicts > eliminate_limit:
        eliminate()                            # BVE + subsumption,
                                               # bound escalates 0,1,2,4,8,16
        eliminate_limit = conflicts + next_interval()

# every pass inside a bundle is BUDGETED, not gated:
pass_budget = pass_effort_permille * (search work since last bundle)
              floored at a minimum                     # e.g. sweep = 10%

# per-pass self-throttle:
if pass yielded ~nothing: delay it (skip next k bundles, k grows +1)
else:                     halve its delay

# --- restarts ---------------------------------------------------------
focused mode:  restart when fast_glue_EMA > margin * slow_glue_EMA
               (checked every conflict; interval floor ~1; fires every
               ~30-50 conflicts on hard instances; reuses trail)
stable mode:   reluctant doubling (Luby-like), much rarer restarts
mode switch:   alternate focused/stable on a tick/conflict schedule

# --- clause DB cleanup -------------------------------------------------
when conflicts > reduce_limit:
    keep clauses by tier (glue/usage); drop a fraction of the rest
    reduce_limit = conflicts + next_interval()

# --- rephase / walk ----------------------------------------------------
in stable mode, on its own conflict timer:
    rephase (flip saved phases by a rotating policy)
    occasionally run a local-search walk, budgeted by ticks
```

**The whole decision layer is ~6 timers + ~10 per-pass budgets + ~10
delay counters + 1 restart inequality.** No per-instance eligibility
logic: a barren pass runs, wastes its (bounded) budget once, and gets
delayed. Everything compounds because each pass sees the formula the
previous passes already shrank.

---

## 3. solver12: pseudo-code of the decision layer

solver12's philosophy (evolved over 29 tuning sessions): *default to a
conservative baseline byte-identically; unlock aggressive behavior only
where a structural test ("latch"/"band") proves the instance is in a
family that measurably benefits.*

```
# --- preprocessing extras (before search) ------------------------------
run structure detectors; if one matches, SOLVE OUTRIGHT or transform:
    PHP/counting refuter (pigeonhole-shaped cores, mchess class)
    sweepcount (exactly-one cover imbalance)
    gauss/XOR extraction, tseitin cycles
# these are proof-emitting engines, not schedulers - keep either way

one-shot simp (BVE, subsumption, ELS, probe, transitive, congruence)
DECIDE-once (structural bands, thresholds hand-fitted on 2025 suite):
    dive-restart latch?      (clause-mass collapse + binary fraction)
    dive2 miter band?        (=> kissat-parity restarts + elim escalation
                              + trail reuse + strict chrono, this band only)
    congruence-productive?   (root merges => aggressive inprocess cadence)
    inline binary tags?      (only if the formula will never be edited)

# --- inprocessing ------------------------------------------------------
after each conflict:
    if conflicts > inprocess_limit OR tick_cadence_due():
        inprocess_round():                     # same pass order as kissat:
            probe -> transitive -> congruence -> ELS -> vivify
            -> sweep -> eliminate
        BUT most passes are GATED per instance:
            vivify-deduce   only if armed late  (>=500k conflicts arming)
            sweep escalation only if yield latch fired (>=1000 equivs)
            mid-search eliminate only if ARMED (aggressive latch)
            elim bound escalation only inside dive2 band
        inprocess_limit += interval            # 2k flat; armed: 10k doubling
    ARMING: one-way latches flipped by pass yields (congruence merges,
    vivify yield, sweep equivalences) switch the instance from the
    conservative cadence to the aggressive one, permanently.

# --- restarts ----------------------------------------------------------
default: kissat-style glue EMAs but TAMER constants (interval floor
         ~50+log, margin 1.20) => ~10x fewer restarts than kissat
dive/dive2 bands: kissat-parity constants (floor 2, margin 1.10)
                  + trail reuse, for latched instances only

# --- clause DB cleanup --------------------------------------------------
reduce on learned-budget triggers; kissat-parity keep-fraction,
but the aggressive fraction only inside the late-armed band

# --- rephase / walk -----------------------------------------------------
never-armed instances: rephase/walk unlocked only after 1M..500k
conflicts (the "endgame" latch); armed instances: separate policy;
walk gives up after 16 stalled walks
```

**The decision layer is ~20+ interacting one-way latches, ~10 structural
bands with fitted numeric boundaries, plus the same timers kissat has.**
The 2026 holdout showed these guards *decline cleanly* on unseen input
(they don't misfire), but the conservative baseline they fall back to is
weaker than kissat's always-on regime.

---

## 4. Key differences that matter for an RL scheduler

| dimension | kissat | solver12 |
|---|---|---|
| decision surface | small, uniform: timers + budgets + delays + 1 restart test | large, heterogeneous: same timers PLUS ~20 hand-fitted latches/bands |
| decision semantics | *continuous knobs* (intervals, permille budgets, delay counts) — natural RL action space | mostly *discrete one-way latches* — fires once, irreversible, hard credit assignment |
| mechanism strength (general) | stronger: holdout 197/400; passes compound via immediate substitution fixpoints | weaker general core: holdout 160/400; strength is family engines + fitted bands |
| what a policy would replace | the schedule of already-good machinery → headroom is real and attributable | first must *unwind* the latch ecology to expose clean decisions; bands and policy will fight |
| observation features | must be added (C, stats exist internally; `kissat -s` shows the vocabulary) | already exported: rich per-pass stats JSON (yields, ticks, EMAs, arming events) — feature vector nearly free |
| hook engineering | C; few chokepoints (`kissat_probing()`, `SET_EFFORT_LIMIT`, `DELAYING`, restart test, reduce test) — small patched surface | Rust; we own it; easy inference embedding, but chokepoints are scattered across the latch code |
| determinism / training | deterministic per seed; cheap `--seed`-style variation via options | deterministic; per-cell trajectories heavily documented (helps debugging, biases toward 2025 lore) |
| proof/correctness harness | proofs optional (reference builds run without) | full DRAT emission + drat-trim + model-audit tooling in-tree |
| known trap | its heuristics co-evolved: changing the schedule perturbs a tuned equilibrium (still, its equilibrium is robust globally) | its equilibrium is 2025-fitted; policy trained against our bench would inherit the same overfit pressure unless trained/evaluated on held-out suites |

---

## 5. Where the RL policy would plug in (action inventory)

The same abstract interface fits both solvers:

```
observation (per decision tick, cheap to compute):
    conflicts, decisions, propagations, restarts since last fire
    glue/level EMAs (fast & slow), trail size, assigned fraction
    learned-DB size & tier mix, live vars/clauses, binary fraction
    per-pass recent yields (merges, equivalences, strengthened, eliminated)
    per-pass recent costs (ticks spent / budget)
    time-in-run, phase (focused/stable)

actions:
    fire {probe-bundle | eliminate | reduce | rephase | walk} now? (5 bools
        or next-interval integers)
    per-pass budget multipliers (continuous, ~10 dims)
    restart aggressiveness (margin/floor, continuous) or restart-now bool
    mode switch focused/stable

reward:
    terminal: solved (+ PAR-2 shaping); dense options: eliminated vars,
    formula shrinkage, conflict rate, EMA trends
```

- In **kissat** this maps 1:1 onto existing code: every action replaces a
  `UPDATE_CONFLICT_LIMIT` / `SET_EFFORT_LIMIT` / `DELAYING` / restart-test
  call site. Roughly a dozen small patch points, mechanisms untouched.
- In **solver12** the timers map the same way, but the latch/band layer
  overlaps the policy's job. To avoid two controllers fighting, the bands
  would be stripped or frozen open, which forfeits the tuned 2025 bank —
  i.e. solver12's main measured advantage disappears the moment the
  experiment starts.

---

## 6. Recommendation

**Use kissat as the base for the RL scheduler experiment.**

1. **The upside lives there.** RL over scheduling only pays if the
   underlying passes are worth scheduling. Kissat's mechanisms are the
   strongest available and its generalization is proven (197 v 160 on
   the unseen 2026 track). A perfect scheduler on solver12 still sits on
   the weaker general core.
2. **The decision layer is already policy-shaped.** Kissat's schedule is
   a handful of continuous knobs behind uniform chokepoints — a clean,
   small action space with no hidden hand-coded competitor. Solver12's
   latch ecology is itself a hand-crafted policy; replacing it means
   demolition before construction, and its fitted bands are exactly the
   overfit surface we're trying to escape.
3. **Attribution stays clean.** With mechanisms frozen and only ~12 call
   sites patched, any solve-count delta is attributable to the policy.
   In solver12, policy effects would be confounded with band interactions
   we spent 29 sessions mapping.

**What to carry over from solver12 regardless of base:**
- The **front-end engines** (PHP/counting, sweepcount, gauss, tseitin) as
  a pre-solver: measured +16 on 2025 and +6 on unseen 2026 (php_sudoku
  solved on first contact). They are orthogonal to the scheduling
  question and bolt onto any back-end.
- The **evaluation discipline**: train/tune on one suite, gate on the
  held-out one (`benchmarks/sat-comp-2026` is downloaded and manifested);
  PAR-2 + solved-count lexicographic; proof verification on.
- The **stats vocabulary** for the observation vector (solver12's stats
  JSON enumerates the useful signals; kissat needs an equivalent
  exporter, which is mechanical).

**When solver12 would be the right base instead:** if the project values
staying in Rust end-to-end (inference embedding, memory safety, our
debug/audit tooling) above starting from the stronger baseline — or if
the RL scope is *narrower than full scheduling* (e.g. only learning the
arming decisions themselves, where solver12's latch points are natural
labels). For the stated goal — replacing inprocessing/restart/cleanup
decisions wholesale — kissat is the better substrate.

---

## 7. What would need to change in kissat (concrete work plan)

Verified against the pinned source in
`benchmarks/reference-solvers/kissat-latest/src` (MIT-licensed,
dependency-free C, `./configure && make`). The decision layer really is
as small as claimed; here is the full patch inventory.

### 7.1 The decision chokepoints to intercept (~15 call sites)

Every scheduling decision already flows through a named boolean or macro
— the patch is "consult the policy here, fall back to stock":

| decision | today | file |
|---|---|---|
| fire the probe bundle? | `kissat_probing()` — conflict timer | `probe.c` |
| fire eliminate? | `kissat_eliminating()` — conflict timer + change tests | `eliminate.c` |
| fire reduce (DB cleanup)? | `kissat_reducing()` — conflict timer | `reduce.c` |
| restart now? | `kissat_restarting()` — glue-EMA test / reluctant | `restart.c` |
| rephase now? | `kissat_rephasing()` — conflict timer | `rephase.c` |
| switch focused/stable? | `kissat_switching_search_mode()` — tick timer | `mode.c` |
| next interval after a fire | `UPDATE_CONFLICT_LIMIT` macro (N·logN growth) | `kimits.h` |
| per-pass tick budget | `SET_EFFORT_LIMIT` macro — 8 users: sweep, vivify, eliminate, backbone, factor, forward-subsumption, transitive, walk | `kimits.h` + each pass |
| per-pass skip/throttle | `DELAYING` / `BUMP_DELAY` / `REDUCE_DELAY` counters | `kimits.c` |
| elimination depth | `set_next_elimination_bound` (0→1→2→4→8→16) | `eliminate.c` |

Concrete change: add `policy.c/h` with one entry point per row
(`policy_fire(solver, WHAT)`, `policy_budget(solver, PASS)`,
`policy_interval(...)`, `policy_delay(...)`), each returning the stock
answer when the policy is disabled (env/option `--policy=<weights file>`),
so a no-policy build is byte-identical stock kissat — that gives the
clean A/B baseline for every experiment. Estimated ~300–600 lines of
glue; zero changes to the mechanisms themselves.

### 7.2 Observation exporter (~300 lines, new)

The raw signals already exist in `statistics.h` (per-pass counters:
eliminated, sweep equivalences/units, vivify strengthened, kitten/search
ticks, …) and `averages.h` (fast/slow glue, level, trail EMAs). Needed:

- a `policy_observe(solver)` function packing ~40 floats: global state
  (conflicts, ticks, live vars/clauses, binary fraction, trail/assigned
  levels, mode), EMAs, and **per-pass recency features** — yield and cost
  since each pass's *last* firing (requires a small snapshot struct
  stamped at each fire; the counters themselves already exist);
- instance-static features computed once at parse (size, binary
  fraction, etc. — the same vocabulary solver12's bands read, minus the
  fitted thresholds);
- a decision-log mode (`--policy-log=<file>`): one line per decision with
  observation, action taken, and outcome deltas — this is the offline
  dataset for imitation-bootstrapping the policy from stock kissat's own
  schedule before any RL.

### 7.3 Inference embedding (~200 lines, new)

Keep kissat dependency-free: a hand-rolled MLP/linear-policy forward
pass in C (weights loaded from the `--policy` file). No ONNX runtime.
Latency discipline: `kissat_restarting` is consulted **every conflict**,
so either the policy for restarts stays a cheap thresholded score
refreshed every N conflicts, or the per-conflict path keeps the stock
EMA test and the policy only sets its margin/floor. All other decisions
fire at ≥ thousands-of-conflicts granularity — a small net is free there.

### 7.4 Safety rails (small but mandatory)

- Clamp every policy-chosen budget/interval to [min, max] so a
  degenerate policy cannot starve search or hang a pass (kissat's
  budgets already bound pass work — keep the clamps around the policy).
- Scheduling is proof-neutral (it changes *when*, never *what* is
  derived), so DRAT soundness is untouched — no new correctness surface.
- Keep `sweeprand`/phase RNG seeding exposed for reproducible rollouts.

### 7.5 Training harness (outside kissat, mostly exists)

- Episode runner = the bench tooling we already use
  (`feature_ablation`-style pinned-core parallel runs); reward =
  solved + PAR-2 shaping; truncated episodes via conflict/wall caps.
- Train/tune on `sat-comp-2025`, **gate on `sat-comp-2026`** (downloaded,
  manifested) — the holdout discipline this project just validated the
  hard way.
- Optional dense reward from the decision log (formula shrinkage, EMA
  trends) for pretraining; terminal reward for fine-tuning.
- Bolt the solver12 front-end engines on as a pre-solver in the runner
  (portfolio style, +16/+6 measured) so the RL work targets the search
  scheduling problem, not the structured families.

### 7.6 Suggested sequencing

1. Fork the pinned `kissat-latest`, add `policy.c` chokepoints with
   stock fallback; verify byte-identical behavior policy-off (A/B on the
   medium suite).
2. Add observation exporter + decision log; record stock-kissat traces
   on sat-comp-2025.
3. Imitation-learn the stock schedule (sanity: cloned policy ≈ stock
   solve count). This validates the whole plumbing before RL.
4. RL fine-tune (start with budgets/intervals only — continuous,
   low-risk; add fire-decisions and restart aggressiveness after).
5. Every candidate policy gates on sat-comp-2026 before being believed.
