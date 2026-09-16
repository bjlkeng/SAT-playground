# RL inprocessing/restart scheduler for solver 13 — build plan (v2, 2026-09-11)

Companion to `plan/rl-scheduler-base-comparison.md` (2026-08), which chose
kissat as the substrate. Solver 13 is that substrate in Rust: a faithful port
at exact counter parity with kissat 4.0.4 (313 v 312 solved on the paired
400-run). This document supersedes the "patch kissat in C" section of the
earlier note. v1 of this document (2026-09-10) and three review passes were
consolidated into this version; §12 records the fresh-eyes review.

**Goal.** Replace the *timing* layer of solver 13 (when to fire probe /
eliminate / reduce / rephase / reorder / mode-switch, how aggressively to
restart, and later how much budget each pass gets) with a small CPU MLP
consulted once every X ticks. Every mechanism stays byte-identical. Train
offline from logged and fork-branched trajectories first, on-policy second,
gate on the standard suite, and judge on the held-out 2026 track.

---

## 1. Epoch clock

Decisions happen only at **epoch boundaries**: every `X` search ticks
(`statistics.search_ticks`), checked at the head of the if-chain in the
search loop (`src/search.rs:254-267`) where the fire predicates are polled.
Between boundaries the solver runs the stock mechanism with the parameters
the last decision set.

Ticks, not conflicts or wall: ticks are deterministic and load-independent
(a trajectory replays exactly), cost-proportional (every action is paid in
ticks), and kissat already schedules mode switching on them (`mode.rs`).

**Calibration (tier-1 probes, 2026-09-10, quiet host, `-s --conflicts=30000`)**

| cell (`benchmarks/discriminating`) | conflicts | search_ticks | probing_ticks | process s | ticks/s | search ticks/conflict |
|---|--:|--:|--:|--:|--:|--:|
| aaai10 pathways-17-step20 | 30003 | 1.70e8 | 5.1e7 | 6.4 | ~35 M | 5.6 k |
| SC25_Timetable_C_392 | 30000 | 1.60e8 | 4.7e7 | 10.9 | ~19 M | 5.3 k |
| brocard_problem_large (UNSAT at 6780) | 6780 | 1.64e9 | 1.0e8 | 88.7 | ~20 M | 242 k |

Stock cadence on the two typical cells: probe every 6-7.5 k conflicts,
eliminate every 15 k, reduce every 2.5 k, rephase every 5 k, mode switch
every 3-6 k, restart every 24-35. Probing ticks are 25-30 % of search ticks.

Consequences: two cadences. **Observation epoch X_o = 2^23 (8.4 M ticks
≈ 0.25-0.4 s)**: a log row and a feature snapshot, ~4500-7000 per 1800 s
run. **Decision epoch X_d = 2^27 (≈ 4-6 s, ~25 k conflicts on a typical
cell, spanning ~3 stock probe fires)**: the policy acts every 16th
observation, ~300-450 decisions per run. The decision epoch **is** the
segment: a fork child's deviation is exactly one decision differing, and
the deployed net re-decides at the same cadence, so training and
inference match with no separate hold parameter (decision 2026-09-14).
The net's input carries deltas over the last 1, 4 and 16 observation
epochs so short and long trends are both visible. Sweep
X_d ∈ {2^26, 2^27, 2^28} on tier 2. Ticks per conflict spans 50× across
cells, so a typical cell sees ~1500 conflicts per epoch and brocard ~35;
ticks/conflict is an input feature and the random-mode priors are defined
over multipliers, not fire events, for this reason. `SAT_POLICY_EPOCH_TICKS=<X_o>,<X_d>`
sets both.

What the epoch does not decide: the per-conflict restart *test* (fires
every ~30 conflicts) and the inside of a pass. Inprocessing that outlives an
epoch runs to completion under its own tick budget; its cost is charged to
the action that fired it.

---

## 2. Action space

### 2.1 Stage 1: relative interval multipliers + restart margin

Stock keeps computing its next interval at every fire (`update_conflict_limit!`
math untouched), but the `scaled` delta is **stored** and the deadline is
recomputed every epoch:

```
eff_limit[T] = last_fire_conflicts[T] + m[T] * stock_delta[T]
```

`m = 1` everywhere is byte-identical stock. Multipliers do **not** stack:
each epoch overwrites `m`, and the base is the delta stock computed at the
last fire. Stock's n·log n growth of the base with the fire count stays on;
it is part of the reference the policy deviates from, and the net sees the
base (§3.2).

| knob | menu | notes |
|---|---|---|
| probe, eliminate, reduce, rephase, reorder | {0, 0.5, 1, 2, 4} | 0 = fire at the next safe point; eliminate keeps stock's "variables changed" test; reorder keeps its `level == 0` test |
| mode switch | {0.5, 1, 2} | ticks delta in stable, conflicts delta in focused |
| restart margin | {0.5, 1, 2} on `restartmargin` (5 / 10 / 20 %) | focused only; stable keeps reluctant doubling |
| sweep effort | {0 = skip, 0.5, 1, 2} on the sweep permille effort | promoted from stage 2: sweep is the dominant probe-bundle cost (10 % effort floor); stock `DELAYING(sweep)` stays ANDed in stage 1 |

Restart margin and mode switching are learned **together** in stage 1
(decision 2026-09-11): with rephase they jointly shape the stable/focused
rhythm, and learning one without the other makes the policy compensate for
a schedule it cannot move.

**Masking.** rephase and restart-margin are no-ops in focused/stable
respectively; reorder runs in both modes at the default `reorder=2` and
is stable-only at `reorder=1`. Invalid actions are
masked in the policy head (forced to the stock value) so exploration and
entropy are not wasted on them.

**`m = 0` is one-shot.** With a decision epoch of ~25 k conflicts, a
multiplier of 0 held for the whole epoch would refire a timer dozens of
times (reduce every few hundred conflicts would thrash the clause DB).
So `m = 0` means "fire once at the next safe point, then behave as `m =
1` until the next decision"; the other multipliers are rates and hold for
the epoch. A floor of `0.1 × stock_delta` on any effective interval
remains as a rail (never binds for a sane policy).

### 2.2 Stage 2: per-pass effort and two structural knobs

| knob | menu | applied where |
|---|---|---|
| effort per pass: sweep, vivify, eliminate, backbone, factor, forward, transitive, walk | {0 = skip, 0.5, 1, 2} | `set_effort_limit!` multiplies the permille effort; stock `mineffort` clamp kept |
| elimination bound | {hold, stock, escalate} | `set_next_elimination_bound` (stock: 0→1→2→4→8→16 when a round completes) |
| reduce fraction | {0.5, 1} on `reducefraction` (75 %) | `reduce` |
| delay counters | on / off (`SAT_POLICY_DELAYS`) | see below |

Delay counters (`delays.congruence/sweep/vivifyirr/bumpreasons`) are
kissat's self-throttle: a pass that yields nothing gets its delay bumped by
one and is skipped for that many rounds; a yield halves it. Stage 1 leaves
them on. Stage 2 evaluates both on and off, since with effort under policy
control the delays are a second scheduler underneath.

### 2.3 Main logic (stage 1 + 2 together)

```
struct Action {
    interval_mult:   [f32; 6],   // probe, eliminate, reduce, rephase, reorder, mode   (stage 1)
    restart_margin:  f32,        //                                                     (stage 1)
    effort_mult:     [f32; 8],   // sweep is stage 1; the other seven are stage 2
    elim_bound:      Hold | Stock | Escalate,
    reduce_fraction: f32,
}
STOCK = all multipliers 1, elim_bound Stock

loop:                                             # search.rs
    conflict = search_propagate()
    if conflict:        analyze(conflict); continue
    if unassigned == 0: return SAT
    if policy.on and search_ticks >= next_obs:                      # every X_o
        log_row(raw counters, averages, limits, delays, act, stock_would_fire[])
        push_snapshot(); next_obs += X_o
        if search_ticks >= next_decision:                            # every X_d = 4 X_o (and at D0)
            obs = observe(last 4 snapshots + static)                 # §3
            act = match mode { Stock => STOCK, Random => sample(), Net => mlp(obs) }
            if branch_due(): fork_children(one knob varied)          # §5.3
            next_decision += X_d                                     # X_d = 16 X_o
    if reducing():          reduce()
    elif switching_mode():  switch_search_mode()
    elif restarting():      restart()
    elif reordering():      reorder()
    elif rephasing():       rephase()
    elif probing():         probe()
    elif eliminating():     eliminate()
    else:                   decide()

fn due(T):                                        # `conflicts` reads `search_ticks` for mode in stable
    if !policy.on:                return conflicts >= limits[T]           # stock
    if act.interval_mult[T] == 0:                                        # one-shot: fire, then revert to 1
        if conflicts >= last_fire[T] + floor[T] { act.interval_mult[T] = 1; return true }
        return false
    return conflicts >= last_fire[T] + max(floor[T], act.interval_mult[T] * stock_delta[T])

reducing()       = due(reduce)
rephasing()      = stable && due(rephase)
reordering()     = reorder enabled for this mode && level == 0 && due(reorder)
probing()        = enabled.probe && last.reduce != conflicts && due(probe)
eliminating()    = enabled.eliminate && irredundant > 0 && last.reduce != conflicts
                   && due(eliminate) && (vars_eliminated or vars_subsumed changed)
switching_mode() = options.stable == 1 && due(mode)

after T fires:                                    # stock math unchanged
    scaled = update_conflict_limit! math
    limits[T] = conflicts + scaled;  stock_delta[T] = scaled;  last_fire[T] = conflicts

restarting():
    if stable: return reluctant_triggered()
    margin = (100 + restartmargin * act.restart_margin) / 100
    return fast_glue >= margin * slow_glue

set_effort_limit!(pass):                          # stage 2
    m = act.effort_mult[pass]
    if m == 0: skip this round
    limit = start + reference * effort_permille * m / 1000
set_next_elimination_bound(): Stock => stock rule | Hold => keep | Escalate => double (capped)
reduce(): fraction = reducefraction * act.reduce_fraction
```

Invariants: with `policy.on == false` no added line executes; with
`policy.on` and `act == STOCK` every predicate equals the stock value
(`last_fire + 1 × stock_delta == limits[T]`). Both are checked by
`tools/parity.py` 20/20 at 100 k conflicts and by a determinism test that
replays a seeded random run to identical counters.

### 2.4 Pre-search: two decision points and a free probe of the instance

The pre-search phase in `search()` is: lucky (early) → one preprocessing
round (probe bundle: congruence, backbone, sweep, factor, plus fast
elimination) → lucky (late) → classify → `init_limits`, which arms the
timers at their `*init` intervals (probe 100, eliminate 500, reduce /
rephase / mode 1000 conflicts). The first probe and eliminate rounds — the
largest formula changes in most runs — therefore fire inside the first one
or two observation epochs, before the first X_d decision.

Two things follow.

**D0 — a decision at search start (stage 1).** `init_conflict_limit` is
treated like a fire: it stores `stock_delta[T] = *init` and `last_fire = 0`,
so the relative multipliers apply to the initial intervals exactly as to
later ones. The policy takes its first decision at `search_ticks = 0`
(before the first `decide()`), choosing interval multipliers, restart
margin, mode-switch multiplier, and sweep effort for the opening phase from
the actor-tier static features plus the **preprocessing yields**. Nothing
else changes: preprocessing and lucky themselves stay stock in stage 1.

**Preprocessing as a free probe (conditioning).** By D0 the solver has
already run every inprocessing engine once and tried four lucky
assignments; their outcomes are the strongest cheap "how does this
instance respond" signal available and cost nothing extra. Actor-tier
features at D0 include: vars/clauses removed by preprocessing (absolute
and fraction), units and equivalences found, backbone units, congruence
gates matched and merges, sweep equivalences/units and whether sweep was
budget-limited, fast-elimination eliminated vars, factor extractions,
preprocessing ticks by pass, lucky outcome per pattern (solved / how far
the forward or backward propagation got before a conflict, as a fraction
of vars), and the warmup assigned fraction. These are SATzilla's probing
features, obtained from the solver's own work rather than a separate
probe.

**D−1 — controlling preprocessing itself (stage 2).** After parse and
before lucky/preprocess only pure static features exist. A stage-2 head
chooses `preprocessrounds` and applies the stage-2 effort multipliers to
the preprocessing round's passes (same `set_effort_limit!` users). Data
comes from fork-at-parse: children with different rounds/effort from the
identical parsed state, the cheapest counterfactual in the design because
no search prefix is spent. Lucky stays stock throughout (a few
propagations, often decisive).

**D−1 changes the state D0 sees.** D0's strongest inputs are the
preprocessing yields, and those depend on what D−1 chose; skipping
preprocessing would remove them entirely. Handled three ways:

1. **Menu {1, 2} first, no skip.** One stock round is budget-bounded and
   rarely hurts, and it is what produces the free probe. `0` enters the
   menu only if fork-at-parse data shows cells where skipping wins.
2. **D0 observes the D−1 action** (rounds taken, effort multipliers, and
   preprocessing ticks by pass), so yields are interpreted relative to the
   effort that produced them rather than as absolute instance properties.
3. **D−1's value is measured with the downstream policy fixed.** Its
   fork-at-parse children all run the same D0/stage-1 policy (stock during
   dataset 1, the learned policy later), so the D−1 advantage integrates
   over what D0 will do with the resulting state, which is the quantity
   that matters. D0 is trained first and frozen when D−1 data is collected;
   if D−1 is adopted, D0 is refit once on traces that include D−1
   variation.

**What `classify` is.** In kissat 4.0.4 (`classify.rs`) it computes two
booleans after preprocessing: `small` (clauses ≤ `smallclauses` = 100 k)
and `bigbig` (binary-clause fraction ≥ `bigbigfraction` = 99 %). In this
version only `bigbig` is consumed: it enables *reason jumping* in binary
assignment (`assign.rs:202`), where a literal implied by a binary clause
whose other literal was itself binary-implied takes that clause's reason,
shortening implication chains on almost-all-binary formulas. `small` is
computed and unused. So classify is a two-bit static classifier feeding
one mechanism, not a scheduling decision; the two bits are actor-tier
features and the mechanism stays as is.

---

## 3. Observation vector

All O(1) per epoch. Counts as `log1p`, sizes as ratios, every cumulative
value paired with its delta since the last epoch. Normalization
(mean/std) is fitted on the stock traces and baked into the weights file.

### 3.1 Dynamic, global

- conflicts, decisions, propagations, search_ticks: cumulative and per-epoch
  rates (conflicts/tick is the search-efficiency signal); ticks/conflict.
- active vars, irredundant / binary / redundant-by-tier clause counts,
  learned since last reduce; arena size, garbage fraction.
- root units this epoch; trail size, assigned fraction, level.
- `averages[stable]`: fast_glue, slow_glue, level, size, trail,
  decision_rate; fast/slow glue ratio.
- restarts this epoch, reused-trail fraction; mode flag, ticks since last
  switch, switch count.
- ticks split: search v probing v eliminating (cumulative and recent).
- learned-clause quality this epoch: glue histogram (8 bins), size mean,
  tier split, max level, backjump depth mean.
- **horizon**: fraction of budget consumed (§3.4).

At D0 and in the first decision epochs the 4- and 16-snapshot deltas do
not exist yet; they are zero-filled with a validity flag per window.

### 3.2 Per timer (so the relative encoding is well-defined)

For each of the six timers: `log(stock_delta[T])`, progress
`(conflicts - last_fire[T]) / stock_delta[T]`, fire count, and stock's
"would fire now" flag.

### 3.3 Per pass (recency; the heart of the problem)

For congruence, substitute, backbone, vivify (irr/tier1/tier2), sweep,
transitive, factor, eliminate, forward/subsume, reduce, rephase, walk:
epochs and ticks since last run; times run; yield at last run; cost (ticks)
at last run; yield/cost; EMA of yield over the last k runs; delay counter
state; current elimination bound.

### 3.4 Work clock, horizon feature, and determinism

**The work clock.** "Ticks" in this document means a single deterministic
work counter `W = statistics.ticks + k_res × eliminate_resolutions`, not
`search_ticks`. `statistics.ticks` is kissat's never-printed all-propagation
counter (search, probing, backbone, beyond, and the dense propagation
inside eliminate all add to it); `search_ticks` and `probing_ticks` are
subsets and eliminate's resolutions are counted separately with no tick
equivalent. `k_res` is one constant fitted from the stock traces (wall
per row regressed on the work kinds). W is what `SAT_LIMIT_TICKS` limits,
what `B_cell` is measured in, and what the terminal `t` is. Using
`search_ticks` alone would have made elimination free in both the budget
and the reward. The epoch clocks (X_o, X_d) stay on `search_ticks`, which
is fine: they only need to be deterministic.

Training uses `W / B_cell`, where `B_cell` is the work-clock reading at
which the paired stock run hit its wall point (known post hoc from the
log), so the feature means "fraction of budget consumed" exactly. At inference the same
quantity is estimated as elapsed wall / wall limit, where the harness
passes the limit as `SAT_WALL_LIMIT` (the binary has no other way to know
it); under `SAT_LIMIT_TICKS` runs it is ticks / limit. This is the **only**
non-deterministic input; ticks/s varies 2× across cells and more under a
32-way gate, so a fixed ticks/s constant would miscalibrate the horizon at
gate time. Parity, replay, and paired-difference tests run with the wall
estimate replaced by the tick fraction (`SAT_POLICY_HORIZON=ticks:<B>`).

### 3.5 Static features: two tiers

- **Actor tier (solver-computed, cheap, once after preprocessing):** vars,
  clauses, clauses/vars, binary/ternary fraction, clause-length histogram,
  occurrence-degree moments, literal balance, Horn fraction, kissat's
  `classification.{small,bigbig}`, and the D0 preprocessing/lucky yield
  features of §2.4.
- **Critic-only tier (offline script, joined at training):** anything
  expensive or unavailable at solve time — VIG modularity, treewidth
  estimates, the SAT/UNSAT label, backbone size from a reference run, stock
  solve time. The actor never sees these; the critic and analyses may.

### 3.6 Cheap instance analysis for the actor (one pass over the formula)

Everything here is linear or near-linear in the literal count, computed
once after preprocessing next to `classify()` over *active* variables and
non-garbage clauses only (eliminated/substituted variables are gone), and costs about a second per
100 M literals — parse already dominates on the giant cells. Grouped by
what they are meant to tell the policy. Prefer *response* features (how
the instance reacts to work: D0 yields, early dynamics) over *identity*
features (sizes, fingerprints): identity is what overfits to families, and
an ablation with the identity group masked is part of the evaluation.

**Shape (identity).** vars, clauses, literals, clauses/vars, clause-length
histogram (1, 2, 3, 4-8, 9-32, 33+) and moments, giant-clause presence and
share of literals in the longest 1 % of clauses (Kakuro's parse/giant-
clause bound), parse-time duplicates and tautologies removed.

**Occurrence and polarity (identity, cheap, well-attested in SATzilla).**
per-variable occurrence degree: mean, variance, max, entropy, fraction of
vars with ≤ 2 occurrences (near-singletons) and the fraction of literals
that are pure; polarity balance per variable (|pos − neg| / (pos + neg))
mean and histogram; per-clause positive-literal fraction distribution;
Horn and reverse-Horn clause fractions. Degree *entropy* is the cheapest
regularity test: crafted/combinatorial instances have near-uniform degrees,
industrial ones heavy tails.

**Binary implication graph (BIG).** binary fraction (the one feature this
repo's bands already leaned on: dive2 band membership, `bigbig`),
fraction of vars touched by a binary clause, BIG degree distribution,
number and size distribution of SCCs and the largest SCC (kissat's
substitute already computes SCCs; count them), roots/leaves ratio, and an
implication-depth/reach estimate from a bounded BFS sample (table below).
At-most-one cliques over negative literals are detectable in the same
pass and count exactly-one/cardinality structure cheaply.

**Locality (identity, nearly free, strong).** For each clause, the span
`max var − min var` normalized by vars: mean, median, and fraction of
clauses with span < 1 % of vars. Encoders emit variables in order, so
Tseitin/BMC/planning instances are highly local and random or shuffled
ones are not; this correlates with community structure at zero graph
cost. Also the fraction of clauses whose variables are consecutive.
**Caveat: locality is destroyed by variable renaming**, so it is the one
feature group that is not shuffle-invariant. CLAUDE.md forbids promoting
input-order luck; a policy that leans on locality must be validated on
shuffled copies (§8), and locality is masked in the shuffle ablation.

**Gate / definition structure (response, from work already done).** After
preprocessing: AND/XOR/ITE/definition gate counts and the fraction of
vars that are gate outputs (a Tseitin-ness score), equivalences found by
congruence and substitute, kitten solved/unsat/unknown counts and kitten
ticks (how expensive definition extraction was), backbone units, sweep
equivalences/units and whether sweep hit its budget, fast-elimination
counts if `fastel` is on. A gate-output fraction near 1 says circuit /
miter; near 0 says constraint encoding.

**Local-search probe (response, optional, budgeted).** kissat has
`walkinitially` (off by default) and `walkeffort`: a short initial walk
reports the minimum unsatisfied-clause count reached and how fast it
fell. SATzilla's local-search features were among its most predictive of
SAT-likelihood; here they cost one bounded walk. Treat as a candidate to
measure, not a default: it changes stock behaviour (turns on a pass), so
it goes in only if the fork-at-parse counterfactual says the information
is worth its ticks.

**Early-search fingerprint (response, free).** From the first two
observation epochs: propagations per decision, ticks per propagation
(a cache-miss proxy: watch-list bytes touched per propagated literal),
conflicts per decision, learned-clause glue and size distributions, mean
backjump depth, restart count, and the trail size at first conflict.
These are already logged dynamics, but as a D0+2 snapshot they
characterize the instance's response before any policy deviation.

**Availability at D0 (before the first decision, cost relative to parse).**

| group | cost | at D0? |
|---|---|---|
| shape, occurrence/polarity, Horn, locality, BIG degrees, binary fraction | one O(L) pass over the arena, ~0.2-0.5× parse, no allocation | yes |
| SCC count / sizes | already computed by substitute in the preprocessing round | yes, free |
| implication-depth/reach BFS from a sample of literals (32 random with binary occurrences + 32 of highest BIG degree; sample size set by the mean's standard error, not fixed at 64) | O(sample × BIG) worst case; capped at ~1 M edge visits | yes, bounded |
| at-most-one cliques over negative binaries | greedy clique growth, can blow up on dense BIGs; capped | bounded, defer |
| congruence gate census (`congruent_gates_{ands,xors,ites}`, `congruent_matched_*`, `congruent_equivalences`, `congruent_arity_*`) | congruence scans the whole formula for gates in order to merge them, so its counts are a full census, unlike eliminate's per-variable gate counts | yes, free |
| backbone: candidates scheduled / tried / units, ticks, budget-hit flag | budgeted (`backboneeffort`, `mineffort` reference in preprocessing); candidates are all active literals, previously refuted ones deprioritized; propagation over binaries only | yes, free (the pass runs anyway) |
| sweep: variables swept, environment sizes, equivalences / units, kitten sat/unsat/unknown, budget-hit flag | budgeted; environments are depth-2 clause neighbourhoods of low-occurrence variables copied into kitten | yes, free |
| eliminate-side gate counts (AND/XOR/ITE/definition extraction) | only run inside `eliminate`; `fastel` is off by default | no — first eliminate is at 500 conflicts, i.e. observation epoch 1-2 |
| initial walk (`walkinitially`) | one budgeted walk; changes stock behaviour | opt-in only |
| early-search fingerprint | needs 1-2 observation epochs of search | no — D0 + 2 |

So the D0 actor vector is: the one-pass shape/occurrence/locality/BIG
groups, SCCs, congruence/backbone/sweep/kitten yields, and a bounded BFS
depth. Gate-extraction counts arrive at the first eliminate and the early
fingerprint at D0 + 2; the decision at X_d = 2^25 (four observation epochs
in) sees both.

**Critic-only, not for the actor.** file compression ratio (a regularity
proxy: the CNFs are already xz), header comment fingerprints (generator
names leak the family), VIG modularity, treewidth estimates, the
SAT/UNSAT label, stock solve time. Useful for analysis and variance
reduction; never inputs to a deployed policy.

---

## 4. Reward

Per episode (one run of one cell), return = terminal + dense, always
**paired** with the stock run on the same cell and seed
(`R_policy - R_stock`).

- **Terminal, PAR-2 shaped:** solved → `2 - t/B` (in [1, 2]); unsolved →
  `0`. This is normalized PAR-2 (unsolved costs 2B), and it puts a gap of at
  least 1 between the slowest solve and any non-solve, matching the gate's
  solved-first lexicographic metric. v1's `1 - t/B` made a 0.99 B solve
  worth ~0.01 more than unsolved — wrong incentive.
- **Censoring:** unsolved is a right-censored observation `t > B`, not a
  zero-information tie; the critic predicts log time-to-solve with a
  survival loss.
- **Dense cost:** `-Σ_k w_k · Δwork_k / X` per epoch over work kinds
  (search ticks, probing ticks, kitten ticks, sweep/vivify/backbone/
  factor/transitive ticks, walk flips, **and eliminate resolutions** —
  eliminate has no tick counter, its effort is `eliminate_resolutions`,
  so a tick-only cost would charge elimination nothing). The weights
  `w_k` are wall-calibrated from the logs (wall per row is recorded),
  because a unit of work is not the same wall cost in every pass and the
  gate is wall. Without this a policy could shift work into cheap-unit
  passes.
- **Progress proxies** (shrinkage, root units, glue trend) are critic
  inputs and pretraining targets only, never selection criteria (hackable).
- **Discount** per tick with a horizon of about the budget (offline RL
  side line only; the ranking main line needs none).

Larger-than-budget instances: (1) timeout-band runs use 3600 s, 2× the
gate, so cells stock cannot solve at 1800 s but can at 3600 s produce
terminal signal and the policy learns budget as a feature; (2) censoring;
(3) dense cost still applies on cells nobody solves.

---

## 5. Data collection

Runs are the cost; row width is free (1.6 TB free; ~35 MB per run raw,
20-40 GB for the dataset compressed). The logger dumps raw state every
epoch and the feature vector is chosen offline.

### 5.1 What every row carries

All ~200 `statistics` counters as raw u64 (deltas derived offline), both
`averages[]`, every limit, delay counters, elimination bound, tier glue
limits, mode, level, trail, unassigned, arena/garbage, the per-epoch
learned-clause histogram, the stock counterfactual (deadline and
would-fire per timer), the action taken, RNG state, and the monotonic wall
clock. **Counter audit:** the port compiles out ~40 kissat METRIC counters
as no-ops (literals learned/minimized/shrunken/bumped/deduced,
definitions checked/extracted, walk decisions, target/best saved,
rescaled, compacted, flushed). Re-enable the ones the observation needs;
they are pure increments and cannot change search.

### 5.2 Stock traces

Stock solver 13 with logging on sat-comp-2025 (400), and on sat-comp-2026
(400, holdout, evaluation only). Uses: normalization, the static-feature
runtime predictor (critic baseline and feature sanity check), behaviour
cloning targets, and `B_cell`.

### 5.3 Perturbed runs and fork branching

Three flavours, each run recording its flavour and free knob groups:

1. **Branch-off via `fork()`.** Solver 13 is single-threaded (`libc` is
   already a dependency; the xz decompressor child in `file.rs` is reaped
   before search). At a branch decision the process forks: the parent
   continues under the parent policy (stock in round 0, the current net
   in later rounds); each branch point picks **one knob** and forks one
   child per alternative menu entry of that knob (4 children for a 5-entry
   knob, 2 for a 3-entry one), each holding its entry for one decision
   epoch (the segment), then returning control to the parent policy, and
   logging to `<log>.<branch>.<k>`. Forking the whole menu of one knob
   yields a complete ordering for that head (≈ 10 pairwise labels from 4
   children) instead of one pair per head.

   **One parent per cell.** Parents are deterministic — stock in round 0
   and a deterministic net later — so several parents of one cell would
   run identical prefixes. A single parent carries all branch points
   (~16, uniform over 0-80 % of the budget plus timer-due states in round
   0; chosen by ensemble disagreement × stakes later, §6.1c). Concurrency is bounded by a semaphore:
   the parent **blocks** at a branch point until a child slot frees. That
   costs wall but not work-clock ticks, and nothing here is measured in
   wall, so it is free. (Supersedes "4 children at 3 branch points per
   parent × 5 parents", 2026-09-15.) The prefix is computed once per cell, and
   **several children from the identical state** give a direct Q(s, a)
   comparison (e.g. four children with m ∈ {0, 0.5, 2, 4} on one timer).
   Branch points: drawn uniformly over 0-80 % of the budget, a few per
   run. Proofs off. **Children stop on a tick budget, not a wall clock**:
   `B_cell` from the paired stock run (or 2× it on the timeout band).
   Children inherit the parent's CPU affinity and contend with it for the
   core, which distorts wall but not ticks, so a tick budget makes every
   child's episode deterministic and load-independent. Children also
   redirect stdout/stderr to their own files so the harness never sees a
   second `s` line. Costs: children are live processes (`--jobs` =
   parents × (1 + children)); copy-on-write pages diverge as the arena is
   written, so memory is budgeted per child as a full process.
2. **Segmented sticky.** Segment lengths geometric with mean ~5 decision epochs (~80 observation epochs); at
   each segment start, with probability 0.5 the action is STOCK, else each
   knob is drawn from a categorical centred on 1 whose spread is the run's
   temperature (near-stock: ±1 menu step; wild: uniform). `m = 0` gets a
   small probability (~0.05) so wild runs do not fire every timer every
   few epochs regardless of the cell's cadence.
3. **Near-stock jitter.** Per-epoch resampling at low temperature.

i.i.d. per-epoch noise averages back to stock and teaches little; the
sticky and branch-off flavours produce the coherent counterfactuals
("probe twice as often for 50 epochs from this state") offline RL needs.

### 5.4 Run mix and stratified walls

Per cell (round 0): **1 branch-off parent with ~16 branch points (~60
children)**, 2 segmented, 1 jitter — the same child budget as the earlier
5 × 3 × 4 layout minus four redundant prefixes, with the wild flavours
trimmed because only the offline-RL side line uses them. Same on the
timeout band. (Stock-at-other-seeds runs were dropped earlier: the paired
difference is exact in ticks.) **All flavours use the paired stock run's seed**, so every
paired difference is on the same deal; seed robustness is tested at
evaluation via shuffled copies (§8), not by mixing seeds into the training
signal. Rebalance after the
first pass by measuring which flavour yields the most advantage variance.

**Every perturbed run — parent and child — stops on a per-cell tick
budget, never a wall clock.** `B_cell` is read off the stock trace: the
tick count the stock run had reached at `clamp(3 × stock solve time,
120 s, 1800 s)` of its own wall (whole suite), or at 3600 s (timeout
band). That stratifies the budget by difficulty (cells stock solves in
< 5 s produce almost no epochs; hopeless cells only censored rows) while
keeping every episode deterministic and load-independent. A generous
wall cap (2× the expected wall) exists only as a safety net and a hit on
it is logged as an anomaly, not a result. Requires `SAT_LIMIT_TICKS`
(§7).

| run set | cells | wall | passes | host time (32 cores) |
|---|--:|--:|--:|--:|
| stock traces 2025 + 2026 | 800 | 1800 s | 1 each | ~10 h |
| stock traces, timeout band at the band's budget (defines `B_cell` there) | ~90 | 3600 s | 1 | ~3 h |
| round 0: perturbation, 2025 train split | ~300 | stratified work budgets | 1 parent + ~60 children + 3 wild runs per cell | ~15-25 h |
| round 0: perturbation, timeout band | ~90 | 3600 s-equivalent work | same | ~30 h |
| DAgger rounds 1-3 (branch-off only, net as parent) | ~390 | same work budgets | 1 parent + ~60 children per cell, concentrated on the top knobs | ~1 day each |
| candidate gates + holdout | 100 / 400 | 1800 s | per candidate | 2-6 h each |

### 5.5 Stage-2 data

A second, cheaper set on the same machinery. Dataset 1 randomizes stage-1
knobs with stage-2 at stock. Dataset 2 randomizes stage-2 knobs (effort,
elimination bound, reduce fraction, delays on/off) with stage 1 at stock or
at the learned stage-1 policy. Stage-2 credit is local — an effort
multiplier's effect shows in that pass's yield per tick — so the design is
**fork-at-fire**: when a pass is about to run, fork children with effort
{0, 0.5, 1, 2} and compare. The stage-2 head trains on dataset 2; heads
share the trunk.

---

## 6. Learning

### 6.1 Baselines ladder (must be beaten, in order)

Cheap analyses on the perturbation data before any net is trained:

1. **Stock.**
2. **Best global constant:** for each knob, does a constant `m ≠ 1` win on
   average across cells? (A global retune of kissat's intervals; if
   eliminate at 2× beats stock, the RL policy must beat *that*.) **Needs no
   new code** — the knobs are CLI options (§7 item 5b) — so it runs as
   step 0 of the sequencing, before any policy engineering, and bounds the
   headroom of the whole project for one multi-arm sweep of host time.
3. **Per-instance constant from static features:** algorithm-configuration
   style, one decision at t = 0 from the actor-tier static features.
4. **Epoch policy, simple models first** (decided 2026-09-14). The
   ranking formulation makes each knob an independent small problem
   (state → score per menu entry, groups = sibling sets), so per-knob
   logistic regression and per-knob gradient-boosted trees (`rank:pairwise`,
   eight models, tens of thousands of rows × ~150 features, seconds each)
   are trained on the same labels before the net. At this data size trees
   often beat a small MLP; the net's only structural edge is the shared
   trunk. Whichever wins on the validation split is the deploy target — a
   tree forward pass in Rust is trivial, deterministic, and has no float
   question. The tree/linear models also give feature importance for
   pruning the observation vector.
5. **Shared-trunk net** (§6.4). Its headroom over (4) is what justifies
   it; if (3) or (4) captures most of the gain, ship that.

### 6.1b Data reality and knob triage

~390 cells × ~16 branch points ≈ 6 k labeled states per round, i.e.
**~750 states per knob per round** across eight stage-1 knobs, with ~150
features. That is thin: enough for per-knob linear/tree rankers, marginal
for a net, and the reason simple models come first (§6.1 step 4). Two
consequences:

- **Triage the knobs.** Step 0 (constant sweeps) and round 0 give each
  knob's effect size (how often and by how much a non-stock entry beats
  stock). Rounds 1-3 concentrate branch points on the top 3-4 knobs; the
  rest stay at stock (`m = 1`) and are revisited only if the top knobs
  plateau. Eight heads trained on 750 states each is worse than three
  heads on 2 000.
- **Branch points are the cheap lever.** With one parent per cell, an
  extra branch point costs only its children (~half a run each), no
  prefix; 16 → 32 points doubles per-knob data for ~2× child cost.

### 6.1c Active branching (explore/exploit for fork children; 2026-09-15)

Children are pure exploration — the parent already exploits by running
the current policy, and it must stay unmodified because its continuation
is the reference every child is compared against. So the question is
where a child buys the most information.

- **Signal = epistemic uncertainty × stakes.** Uncertainty is *ensemble
  disagreement*, not softmax entropy: train ~10 per-knob rankers on
  bootstrap resamples of the labels (seconds each for trees/linear) and
  measure the variance across them of the score gap between each entry
  and stock. A flat softmax can mean "entries are truly equal" (no point
  exploring) or "no data here" (explore); disagreement isolates the
  second. Stakes is 1 if that knob's timer would fire within the coming
  decision epoch (under stock or the policy) and small otherwise — a
  multiplier that changes no fire in the window teaches nothing.
- **Branch rule.** At each decision the parent scores every knob; it
  branches when the score is in the top quantile seen so far in the run
  (online quantile tuned to ~16 points per run). 25 % of branch points
  stay uniformly random for coverage and as a control that active
  selection helps.
- **Adaptive child count.** Fork stock plus every entry whose
  ensemble-mean score is within a band of the best; confidently bad
  entries are not re-tested. A contested pair costs 2 children, a
  wide-open knob the full menu; saved children become more branch
  points.
- **Round 0** has no model: uniform + timer-due branch points, knobs
  weighted by step-0 effect sizes. It is the burn-in that makes the
  ensemble possible.

### 6.2 Learners, in order

1. **Behaviour cloning** of stock on the stock traces. Under the relative
   encoding the target is the constant "all ones" (§6.4), so this is an
   initialization plus a plumbing check, not a learning problem. Success
   = policy-on-STOCK passes parity and the cloned net at working margin
   deviates on ~0 % of decisions.
2. **Fork-sibling ranking** (first real learner; adopted over advantage
   regression 2026-09-13): at each branch point the K children plus the
   parent's own continuation are outcomes from one state differing on one
   knob. Train that knob's head with a pairwise logistic loss so the
   better sibling scores higher; scale-free across cells. Act greedily
   with the margin over stock (§6.4). **All-censored sibling sets** (no
   sibling solved within budget — the common case on the timeout band)
   carry no terminal ordering and are **dropped from the policy loss**;
   they are not labeled by progress proxies, which are hackable. This is
   exactly why the band runs at 2× the gate budget: labels there exist
   only where some sibling solves. Ties among solved siblings are broken
   by wall-calibrated cost (§4).
3. **Offline RL** (IQL or AWR) on all flavours as a side experiment,
   using the sticky segments for multi-step credit. Checkpoints selected
   on the validation split (§8), never on training loss. The main line
   has no critic; the static-feature runtime predictor is a diagnostic.
4. **On-policy rounds by DAgger-with-fork** (decided 2026-09-13; PPO is
   the fallback only if fork labels stop improving while outcomes show
   headroom). Each round: the current policy runs as the parent, children
   fork at branch points and try the alternatives (stock's included),
   holding the deviation for a segment; the winners become ranking labels
   on states the deployed policy actually visits. Three rounds fit the
   budget. Branch points are chosen **actively** by ensemble disagreement ×
   timer-due stakes, with adaptive child counts (§6.1c).
   Why not PPO: ~10 passes give ≤ 8 updates, its exploration is
   uninformed sampling, and it cannot use exact replay or same-state
   counterfactuals — the two things this setting has that most RL
   settings lack. Cost of the DAgger choice: one-step-improvement labels
   and a few thousand labeled states per pass, mitigated by segment-held
   deviations and iteration.

   The loop, plainly: round 0's parent is stock (policy off), branch
   points uniform over the run plus one before each stock timer is due,
   and the first net is a behaviour clone of stock trained on round-0
   labels. Each later round: run the current net as parent, fork 4
   children at each of 3 branch points, each holding one deviation for
   one decision epoch then returning control to the net, run each to its tick budget,
   label by outcome ranking, add to *all* previous rounds' data (nothing
   is discarded — the aggregation is the point), retrain, repeat. Round
   cost ≈ 18 k children ≈ a day of host at stratified walls; levers are
   children per point, points per run, tick budget. Stop when the
   parent's own action wins at a stable fraction of branch points, the
   validation tick-PAR-2 plateaus, and successive nets disagree on few
   held-out states; expect most gain in round 1, budget three rounds.
5. **Ablations** by action group to locate the headroom.

### 6.3 Action persistence at inference

Resolved 2026-09-14 by making the decision epoch the segment (§1): a
child holds its deviation for exactly one decision epoch (X_d = 2^27),
and the deployed net re-decides once per decision epoch. Training and
inference cadences are identical by construction; the only persistence
device left is the switch margin over stock (§6.4).

### 6.4 Network and inference

MLP with a shared trunk and one head per knob. Input ~150 floats: static
block (~30, D0), per-timer block (6 × {log stock base, progress, fire
count, would-fire}), per-pass recency block (~12 × 5), global dynamics
(~30) as deltas over the last 1, 4 and 16 observation epochs (multi-scale
trend). Trunk 150 → 128 → 64, ReLU. Heads: five 5-way interval heads, mode 3-way,
restart margin 3-way, sweep effort 4-way (~30 scores); stage-2 heads
later. ~30 k multiply-adds per decision at 2^25 ticks: negligible;
measure once. Children at a branch point vary **one knob** from the
parent's action so each ranking label lands on exactly one head.

**Invariants across rounds.** The action menu is frozen before round 0
(labels are over menu entries); the raw-counter logging is complete, so
features and architecture may change later and old rounds are re-derived
at training time.

**First net = stock, enforced four ways.** (1) Initialize each head's
bias so the stock entry starts with a large positive score; (2) clone
stock on the stock traces (trivial under the relative encoding — the
target is "all ones" — which is why that encoding was chosen; it still
fits normalization and shows the trunk real states); (3) at deployment,
act on a non-stock entry only when its score beats the stock entry by a
margin τ (`SAT_POLICY_MARGIN`; with a pairwise-logistic ranker the scores
are log-odds, so τ = 1 means "≈ 73 % chance the alternative wins";
sweep τ ∈ {0.5, 1, 2} on tier 2), the safety knob at the gate; (4) verify: τ = ∞ must pass parity, and the initial net at
working τ must deviate on ~0 % of validation decisions. Trained in PyTorch, exported as a flat
little-endian f32 file (shapes, normalization, weights), loaded via
`SAT_POLICY=<file>`; hand-rolled forward pass in Rust, no new crates. Build
stays generic x86-64 (no FMA contraction differences between hosts; Rust
does not auto-contract, keep it that way).

---

## 7. Engineering plan in solver 13

Additive and off by default; policy-off stays at exact parity.

1. `src/policy.rs`: `Policy` state (epoch counter, snapshots, per-timer
   `stock_delta`/`last_fire`, action, mode = stock | random | net, branch
   schedule, logger); `epoch_due`, `observe`, `decide`, `log_row`,
   `fork_children`.
2. Chokepoints, each a two-line `if policy.on` branch: the six fire
   predicates, the `restartmargin` read; stage 2: `set_effort_limit!`,
   `set_next_elimination_bound`, `reducefraction`, `delaying`.
3. Search-loop hook at the head of the if-chain.
4. Actor-tier static features next to `classify()`; dumped in the log
   header.
5. Counter audit: re-enable the METRIC counters the observation needs.
   **Keep them out of the `-s` block** (or behind a flag): `parity.py`
   compares every `-s` counter and phase line against kissat, so a new
   printed counter would break the oracle.
5b. `run.sh`: forward `SAT_EXTRA_ARGS` to the binary (one line). All the
   interval/effort knobs (`probeint`, `eliminateint`, `reduceint`,
   `rephaseint`, `reorderint`, `modeint`, `restartmargin`, `sweepeffort`,
   `vivifyeffort`, ...) are already kissat CLI options, which makes the
   constant-multiplier baseline (§6.1 step 2) a zero-code experiment
   through `feature_ablation.py --arm 'x:SAT_EXTRA_ARGS=--eliminateint=1000'`.
6. Env: `SAT_POLICY=stock|random|<file>`, `SAT_POLICY_LOG`,
   `SAT_POLICY_EPOCH_TICKS`, `SAT_POLICY_TEMP`, `SAT_POLICY_SEED`,
   `SAT_POLICY_HOLD`, `SAT_POLICY_MARGIN`, `SAT_POLICY_BRANCH`,
   `SAT_POLICY_BRANCH_ACTIONS`, `SAT_POLICY_DELAYS`, `SAT_POLICY_HORIZON`,
   `SAT_WALL_LIMIT`, `SAT_LIMIT_TICKS`. `run.sh` forwards the environment
   already.
6b. **`SAT_LIMIT_TICKS`** (new; the solver has only `--conflicts` and
   `--decisions`): a `limited.ticks` alongside them, checked where the
   conflict limit is and inside the inprocessing effort loops,
   terminating with `s UNKNOWN` when the work clock W (§3.4) reaches the
   limit. Needed by every perturbed run (§5.4) and by the
   deterministic candidate comparison (§8).
6c. Fork hygiene and safety: flush stdout/stderr and the log writer
   before `fork()` (else buffered output is duplicated); the child reopens
   its own log and stdout files; the RNG state is deliberately inherited
   (same deal). `fork_children` refuses when a proof file is open (two
   writers on one DRAT file corrupt it); the collector runs proofs off.
   **Sibling agreement is a free correctness oracle**: every child and
   parent of one branch point that finishes must report the same
   SAT/UNSAT; the collector asserts it and validates every child's SAT
   model. A disagreement is a solver bug and stops the run.
6e. `observe()` and `log_row()` must be pure with respect to solver
   state: no RNG draws, no allocation that changes arena layout, no
   counter increments that feed heuristics. The policy-on-STOCK parity
   test is what catches a violation.
6d. The stock trace logs peak RSS at exit (`getrusage`); the collector
   uses it to cap children per parent by cell memory, since the harness
   does not record it today.
7. **Collection harness** `tools/rl_collect.py` (new): `feature_ablation.py`
   does not know about fork children. Needs process-group accounting, jobs
   = parents × (1 + children), per-child memory caps, per-cell stratified
   walls, seed/flavour bookkeeping, and a manifest per pass. Offline:
   `tools/rl_features.py` (critic-tier static features), `tools/rl_dataset.py`
   (log → parquet with deltas, pairing, censoring), and the training code
   under `tools/rl/`.
8. Tests: MLP forward matches a PyTorch reference on a fixed weights file;
   smoke test unchanged; parity 20/20 policy-off and policy-on-STOCK;
   seeded random-mode replay to identical counters; a fork test that a
   child with `act == STOCK` matches the parent's counters; `SAT_LIMIT_TICKS`
   terminates deterministically at the same counters on repeated runs.

Realistic cost of step 1-8: 2-3 sessions, not 1.

---

## 8. Evaluation discipline

- **Splits.** Every medium-suite cell is inside sat-comp-2025 (100/100
  overlap), so the standard gate is a *training-set* gate. Use: (a) a
  family-stratified validation split of 2025 (~100 cells held out of all
  fitting and used for checkpoint selection); (b) the medium gate as the
  in-distribution promotion check; (c) sat-comp-2026 as the true holdout,
  consulted at most once per promoted candidate so it does not become a
  second training set (the solver 12 lesson: 296 v 292 on 2025, 160 v 197
  on 2026).
- **Gate arms.** Every gate carries three arms: stock (policy off),
  policy-on with `act == STOCK` (measures logging/inference overhead and
  the wall-horizon nondeterminism alone), and the candidate.
- **Tick-budgeted, deterministic candidate selection.** Since the policy
  is trained on ticks, compare candidates first under a per-cell tick
  budget (`SAT_LIMIT_TICKS=B_cell`) rather than a wall clock: the result
  is exactly reproducible, immune to load and deal noise, and can run at
  full parallelism without the contention artefacts that make wall
  TIMEOUTs untrustworthy under a 32-way sweep. Report solved count and
  tick-PAR-2. The wall gate stays the only promotion evidence; this is
  for choosing what to gate.
- **Power.** ±2 solved is noise on 100 cells, so a +3 policy on 400 is
  invisible on the medium suite. RL candidates are checked on the full
  400 (train + validation, reported separately) at 1800 s, ~4-6 h, the
  same cost as the acceptance runs. The medium gate remains the repo's
  formal promotion gate (CLAUDE.md); for this project it is a necessary
  check, and the 400-cell wall run plus the tick-deterministic comparison
  are the decision evidence.
- **Shuffled copies.** Evaluate promoted candidates on variable-renamed,
  clause-shuffled copies of the validation split; a win that disappears
  under shuffle is input-order luck (CLAUDE.md rule), and locality is the
  feature most exposed to it.
- **Per-family reporting** (critic-tier family label) on every
  comparison, so a net gain that is one family's win and another's loss
  is visible.
- Lexicographic solved → conflicts → PAR-2 with the trade rules
  (wall-coin v capability cells); tier discipline; zero-tolerance
  correctness (scheduling is proof-neutral, smoke + drat-trim spot check
  anyway).

---

## 9. Risks and open questions

- **Sample cost.** Roughly a week of the 32-core host end to end: stock
  traces (~half a day), round 0 (~2 days), three DAgger rounds (~3 days),
  gates and holdout (~1 day). If the ranking learner does not beat
  baseline (2)/(3) on the validation split after round 1, narrow the
  target (eliminate cadence or restart regime alone).
- **Kissat's equilibrium.** Perturbation data will mostly say deviating
  hurts; that bounds the headroom, and KL-to-stock keeps the policy close
  where the data says stock is right.
- **Reward hacking on proxies**; wall-calibrated tick weights address the
  cheap-tick variant.
- **Horizon nondeterminism** is confined to one feature; if it proves
  costly at the gate, fall back to a tick budget from a per-host ticks/s
  constant and accept the miscalibration.
- **Fork memory** on giant cells (Kakuro 490 MB CNFs, brocard): budget
  children as full processes and cap children per parent by cell size.
- **Generalization / family fitting** via static features: the split
  discipline is the check; also run an ablation with static features
  masked.
- **X for the giant-cell regime**: fixed X first; adaptive X (scaled by
  ticks/conflict measured in the first epochs) only if fixed X fails.
- Open: hold length `k` at inference (§6.3); `k = 1` at X_d = 2^25 first.
- D−1 (preprocess rounds/effort) is **skipped for now** (decision
  2026-09-12); the design in §2.4 stays on record for later.

---

## 10. Sequencing and rough cost

| step | deliverable | cost | bead |
|---|---|---|---|
| A′ | ticks per cell in every `feature_ablation.py` results TSV (tier-2 axis becomes deterministic for all future A/Bs) | small | `SAT-playground-p9m.4` |
| 0 | `SAT_EXTRA_ARGS` passthrough; constant-multiplier sweeps on the existing CLI knobs (baseline 2); headroom estimate | 1 line + 2-3 multi-arm sweeps | `SAT-playground-p9m.5` |
| A | `policy.rs`, chokepoints, epoch hook, static features, logger, fork mode, counter audit, `SAT_LIMIT_TICKS`, `SAT_WALL_LIMIT`; parity 20/20 both ways | 2-3 sessions | `SAT-playground-p9m.6` |
| B | `rl_collect.py`; stock traces on 2025 + 2026 (+ band at 3600 s); X_d sweep on tier 2; normalization, runtime predictor, `B_cell`, peak RSS | 1-2 suite passes | `SAT-playground-p9m.7` |
| C | round 0: perturbation dataset (stratified tick budgets, fork branching, timeout band) | ~2 days of host | `SAT-playground-p9m.8` |
| D | baselines ladder (2)-(3); cloned policy passes parity / ~0 % deviation | 1 tick-deterministic run | `SAT-playground-p9m.9` |
| E | ranking policy from round 0; DAgger rounds 1-3; validation-split selection; 400-cell wall check; medium gate; 2026 holdout once | ~3 days + 2-3 gates | `SAT-playground-p9m.10` |
| F | stage-2 dataset + heads; ablations; promotion note | 2-4 gates | `SAT-playground-p9m.11` |

**Beads (2026-09-15).** The whole plan is in the tracker under the root
epic `SAT-playground-p9m`: one child epic per step above, one task per
deliverable, one bead per long host run. `SAT-playground-p9m.1` holds the
evaluation rules of §8 and is linked from every gate bead.
`SAT-playground-p9m.2` is the chore that reconciles this document's stale
numbers (2^25 v 2^27, 4 × 3 branching, `SAT_POLICY_HOLD`) with the
decisions in §15. `SAT-playground-p9m.3` is the parking lot for the ideas
§9 and §13 defer. Decisions the plan leaves to data are
`SAT-playground-p9m.12` to `.19` (X_d, menu freeze, knob triage, deploy
target, go/no-go after round 1, margin, delay counters, D−1). `bd show <id>`
gives the details; `bd ready` lists what can start. This document stays the
source of truth; every bead points back to its section.

---

## 11. Decision log (from the 2026-09-10/11 review passes)

- Relative multipliers over absolute fire flags for stage 1; the net sees
  the stock base so the encoding is fully expressive.
- Restarts: per-conflict test stays stock; margin regime knob kept in
  stage 1 (per-family sign seen in solver 12 history). ("First to drop"
  superseded 2026-09-11: learned together with mode switching.)
- Reward: PAR-2-shaped, paired, censored; 3600 s on the timeout band.
- Perturbation: three flavours; fork branching replaces prefix re-runs;
  X/4 logging dropped.
- Stage 2 is a second dataset on the same machinery, fork-at-fire.
- Delay counters on in stage 1, evaluated both ways in stage 2.
- 2026-09-11: restart margin and mode switching both learned in stage 1;
  sweep effort promoted to stage 1; stock-other-seed runs replaced by
  branch-off; observation epoch 2^23 with decision epoch 2^25; pre-search
  handled as D0 (conditioning + initial-interval multipliers, stage 1) and
  D−1 (preprocess control, stage 2, fork-at-parse data).
- 2026-09-16 (step 0, stage 1 done; `SAT-playground-p9m.5.2/.5.3/.5.5`):
  constant sweeps of the eight stage-1 knobs on the medium suite at 1800 s
  (18 arms, no proofs, frozen binary; solver README "Step-0 constant-knob
  sweeps"). Best global constant: `reorderint=20000` beats stock on
  solved, tick and wall (76 v 73, 0.983×, 0.949×); `reorderint=5000` and
  `eliminateint=1000` solve 74 at even tick PAR-2 (noise); the other
  fourteen constants lose cells, so stock is near a local optimum and
  baseline 2 is "reorderint 20000, pending the 400-cell check". Joint
  per-cell oracle
  over all 17 constants: 81 v 73 solved, tick PAR-2 0.678× — the upper
  bound for the policy, inflated by chaotic sensitivity (each knob beats
  stock on 25-41 cells and loses on 33-42). Knob triage order from the
  per-knob oracle gain: reduceint, modeint, reorderint, eliminateint,
  probeint, rephaseint, restartmargin, then sweepeffort far last (8 %).
  kissat 4.0.4 has no `reducefraction`: the §2.2 reduce-fraction knob is
  `reducelow`/`reducehigh` (500/900 ‰), scaled together.
- 2026-09-16 (step 0, stage 2 done; `SAT-playground-p9m.5.4`): the eight
  stage-2 knobs, 16 arms, same setup. Effort knobs perturb less than
  intervals (beats/loses 11-40 v 12-38 per cell) and three constants beat
  stock on every metric: forwardeffort 200 (75 v 72, tick 0.962×),
  backboneeffort 10 (74, 0.957×), eliminateeffort 50 (73, 0.990×); the
  reduce fraction halved keeps 72 solved at tick 0.953×. Per-knob oracle
  ranking for stage 2: reducefrac, walkeffort, vivifyeffort, then
  transitive, backbone, forward, factor, eliminateeffort (2 %). Joint
  oracle 81 v 72, tick 0.690×. Two stock arms 12 h apart solved 73 and 72:
  one wall-limit cell is the solved-count noise floor on 100 cells.

---

## 12. Fresh-eyes review (2026-09-11): fixes folded in, and gaps still open

**Folded into v2 above**

1. **Reward incentive was wrong.** `1 - t/B` with unsolved = 0 valued a
   last-second solve ~0.01 above a non-solve; the gate is solved-first.
   Now `2 - t/B` (normalized PAR-2), gap ≥ 1. (§4)
2. **Actor/critic feature confusion.** v1 said static features are computed
   offline and joined at training — but the *actor* needs them at solve
   time. Split into an actor tier the solver computes and a critic-only
   tier. (§3.5)
3. **Tick ≠ wall across passes.** Dense cost is now wall-calibrated per
   tick kind, otherwise a policy can hide work in cheap-tick passes. (§4)
4. **Horizon feature calibration.** A fixed ticks/s constant is off by 2×
   across cells and more under gate load. Train on the exact
   fraction-of-budget from the paired stock run's wall-hit tick count;
   infer from elapsed wall; keep a tick-only mode for parity/replay. (§3.4)
5. **Medium suite is inside the training suite** (100/100). Added a
   family-stratified 2025 validation split; 2026 consulted once per
   promoted candidate. (§8)
6. **`reorder` is a seventh timer** the action inventory missed (both
   modes at the default `reorder=2`). Added. (§2.1)
7. **~40 METRIC counters are compiled out** in the port (learned/minimized/
   shrunken literals, definitions extracted, ...). Audit and re-enable
   what the observation needs. (§5.1, §7)
8. **Wild random mode fires timers at a cell-independent rate** if `m = 0`
   is sampled per epoch; capped its probability, and the menu is over
   multipliers so the cadence scales with the cell. (§5.3)
9. **`m = 0` every epoch thrashes** (reduce every epoch). Added the
   0.1 × stock_delta floor as a rail. (§2.1)
10. **Invalid actions** (rephase in focused, restart margin in stable,
    reorder in focused) wasted exploration; masked. (§2.1)
11. **Uniform 600 s wall wasted runs** on trivial and hopeless cells;
    stratified per-cell walls. (§5.4)
12. **No cheap baselines.** Added the ladder: global constant retune and
    per-instance static selection must be beaten before the epoch policy
    is worth its complexity. (§6.1)
13. **Fork enables a simpler first learner** than IQL: per-branch-point
    advantage regression with counterfactual labels. (§6.2)
14. **Train/inference action-persistence mismatch** (segments in data,
    per-epoch flips at inference). Added a minimum hold length. (§6.3)
15. **Harness gap.** `feature_ablation.py` cannot account for fork
    children, stratified walls, or flavour bookkeeping; a dedicated
    collector is now a deliverable. (§7)
16. **Gate needs an overhead-control arm** (policy-on, STOCK action). (§8)
17. **Step A was underestimated**; 2-3 sessions. (§10)

**Resolved 2026-09-11** (see §11): restart + mode both in stage 1; sweep
effort promoted to stage 1; stock-other-seed runs → branch-off; X_o = 2^23
with X_d = 2^25; pre-search as D0/D−1 (§2.4).

**Still open**

- **Children per parent v branch points per parent** trade coverage of
  actions against coverage of states; start 4 × 3 and measure.
- **Log volume at fork scale**: 5 parents × 12 children per cell adds
  ~60 logs per cell, tens of GB compressed. The one-time converter to the
  compact per-decision feature table (a few MB per run) processes one run
  at a time and is resumable; the compact tables fit in memory for
  training. Not a design item.

---

## Appendix A. Mechanism notes behind the feature groups (2026-09-12 discussion)

- **SCCs.** On the binary implication graph (each binary clause gives
  two implication edges), literals in one strongly connected component
  imply each other and are equivalent; substitute replaces them. Tarjan's
  algorithm is one linear walk and substitute already runs it in the
  preprocessing round; counting non-trivial components and their sizes is
  a few lines inside that walk. A component holding both x and ¬x is root
  unsatisfiability.
- **Congruence closure.** Pattern-matches clauses into AND / XOR / ITE
  gates (Tseitin shapes), hashes gates by type and inputs, merges outputs
  of identical gates, and repeats to a fixpoint because merges create new
  identical gates. Counters: gates found and matched per type,
  equivalences, units, summed arity. Because it scans the whole formula
  to find merge candidates, its counts are a full gate census.
- **Backbone.** Budgeted (`backboneeffort`; `mineffort` reference in
  preprocessing). Candidates are all active literals, both polarities, in
  index order, with previously refuted literals deprioritized. For each
  candidate it assumes the negation and propagates over binary clauses
  only; a conflict makes the literal a unit. Free features: scheduled,
  tried, units, ticks, budget-hit.
- **Sweep and kitten.** Kitten is a small self-contained CDCL solver
  embedded in kissat, used only for sub-queries. Sweep picks a start
  variable (both polarities occurring, under an occurrence cap, cheapest
  first), grows an environment by walking clauses to depth 2 (256 vars /
  1024 clauses, escalating to 3 / 8192 / 32768), copies those clauses into
  kitten, solves once; UNSAT → root conflict/units from the core; SAT →
  the model partitions environment literals into equivalence and backbone
  candidates. It refines by assumption queries (assume ¬lit; assume the
  pair differs, both ways); an UNSAT answer proves the property and its
  core becomes the DRAT lines, a SAT answer splits the partition. The
  main formula only gains the derived units and equivalence binaries.
- **Implication depth / reach.** BFS along binary implications from a
  sampled literal; depth is the longest forced chain, reach the number of
  literals forced. Sample literals with binary occurrences, half random
  and half highest-degree, sized by the standard error of the mean, with a
  global edge-visit cap. Deep/long-reach graphs are where failed-literal
  probing, transitive reduction and vivification pay; a reach set holding
  x and ¬x is a failed literal.
- **Conflict-graph cliques (at-most-one).** A binary clause (a ∨ b) makes
  ¬a and ¬b mutually exclusive; a clique of mutually exclusive literals
  (any polarity mix) is an at-most-one constraint. "Pick one value"
  encodings emit them in bulk (planning, timetabling, colouring, Kakuro,
  pigeonhole). Greedy clique growth with a cap; count and size
  distribution.
- **Gate-aware elimination.** BVE resolves all clauses with x against all
  with ¬x. If x is a gate output, gate-clause × gate-clause resolvents are
  tautological, so only gate × non-gate resolvents are needed and x fits
  under the elimination bound. Kissat finds AND/XOR/ITE syntactically and
  general definitions via kitten (is x a function of its neighbours? core
  → definition). Eliminate's own gate counters cover only variables it
  tried, first available after the 500-conflict eliminate.
- **Initial walk.** `walkinitially` runs kissat's ProbSAT-style local
  search once before search under `walkeffort`: flip variables of
  unsatisfied clauses, biased toward low-break flips, keep the best
  assignment as saved phases. Probe outputs: minimum unsatisfied count
  and its decay rate. Changes starting phases, hence opt-in.
- **classify.** Two booleans after preprocessing: `small` (≤ 100 k
  clauses, unused in 4.0.4) and `bigbig` (≥ 99 % binary), the latter
  enabling reason jumping in binary assignment (`assign.rs:202`). Both are
  actor-tier features; the mechanism stays.

---

## 13. Second fresh-eyes review (2026-09-12)

**Errors and gaps fixed in the body**

1. **Fork children on wall clocks were wrong.** Children inherit the
   parent's affinity mask and contend with it, distorting wall; a wall
   timeout would have made child episodes load-dependent. Children now
   stop on `B_cell` tick budgets and redirect their stdout. (§5.3)
2. **`B_cell` was undefined on the timeout band**: the paired stock run
   there is at 3600 s, which had no logged stock trace. Added the band's
   stock trace at its own budget. (§5.4)
3. **Mixed seeds in the segmented/jitter flavours** would have made the
   paired difference a different deal. Single paired seed everywhere;
   deal robustness moves to evaluation via shuffled copies. (§5.4, §8)
4. **Locality is not shuffle-invariant** (variable renaming destroys it),
   and the repo forbids input-order wins. Flagged, masked in the shuffle
   ablation. (§3.6)
5. **New counters would break the parity oracle** if printed in `-s`.
   Keep them out of the block. (§7)
6. **Baseline 2 needs no code.** Every interval/effort knob is already a
   kissat CLI option; only `run.sh` lacks an argument passthrough. It is
   now step 0: a headroom estimate for the price of a few sweeps before
   any policy engineering. (§6.1, §7, §10)
7. **The 100-cell gate is underpowered for this project** (±2 noise
   against gains that may be +3 on 400). RL candidates are checked on the
   full 400 with train/validation reported separately. (§8)
8. **Candidate selection was going to use wall runs**, which are noisy
   and load-sensitive. Added tick-budgeted deterministic comparison
   (`SAT_LIMIT_TICKS`) for choosing what to gate. (§8)

**Ideas to make it stronger (not yet in the body; decide)**

- **DAgger-style on-policy data via fork** — adopted 2026-09-13, see
  §6.2 step 4.
- **Ranking loss among fork siblings.** Only the ordering of the K
  children from one state matters; a pairwise logistic (or listwise) loss
  on sibling outcomes is scale-free across cells, where absolute
  advantages differ by orders of magnitude. Use it for the
  fork-advantage learner instead of regression.
- **Curriculum** — rejected 2026-09-14: one round, all data at once.
- **Interpretability gate** — adopted 2026-09-14 as §6.1 step 4:
  per-knob linear and boosted-tree rankers before the net, feature
  importance for pruning.
- **Shuffled copies as training augmentation** — parked 2026-09-14 as a
  later experiment if extra margin is needed.
- **Runtime rail.** If the policy-on run shows zero conflicts over many
  decision epochs while stock's trace at the same ticks had thousands,
  fall back to STOCK for the rest of the run. A rail, not a heuristic;
  probably never fires, cheap insurance at the gate.
- **Ticks in the gate TSVs — decided 2026-09-12, do it.**
  `feature_ablation.py` records wall only; adding search/probing ticks per
  cell (from `-s`) to every results TSV makes every future A/B
  deterministic on its tier-2 axis for free, RL or not. Goes into step A.

**Beyond the current purview**

- **Per-instance static selection is a product on its own.** Baseline 3
  (one decision at D0 from actor-tier features) is the classic
  algorithm-selection / configuration result and needs none of the epoch
  machinery. If step 0 and baseline 3 capture most of the headroom, the
  epoch policy becomes a research follow-up rather than the deliverable.
- **Learned choices *within* passes** (which variable sweep starts from,
  which candidates backbone tries first, which clauses vivify picks) are
  noted and **rejected for this plan**: the candidate space is hundreds of
  thousands of items with almost no signal per item, and the only form
  with a chance is a linear scoring rule replacing a fixed ordering. A
  research question with unclear payoff, outside scope.
- **The same logger makes solver 13 an instrument.** Epoch-level traces
  of stock kissat on 800 instances with wall per row are a dataset for
  questions unrelated to RL: where wall goes per family, which passes are
  barren on which structures, how ticks/s (memory-boundness) varies with
  formula shape. Worth publishing on the site.
- **Restart-policy learning at conflict granularity** (the part explicitly
  left out) has a literature of its own (Haim & Walsh 2009 restart
  strategy selection; bandit-based restart/branching switches in
  MapleSAT-era solvers and Kissat_MAB). If the epoch policy's restart
  margin head shows a large effect, a per-conflict learned test is the
  follow-up.

---

## 14. Third fresh-eyes review (2026-09-13): fixes folded in

1. **No tick limit exists in the solver** (only `--conflicts` and
   `--decisions`), yet §5 and §8 depended on one. Added `SAT_LIMIT_TICKS`
   as an engineering item with its own test. (§7)
2. **Eliminate has no tick counter** — its effort is
   `eliminate_resolutions` — so the "tick-weighted" dense cost would have
   charged elimination nothing. Cost is now over work kinds including
   resolutions, wall-calibrated. (§4)
3. **Walls were still in the collection design** (stratified walls for
   perturbed runs) although children had moved to tick budgets. Every
   perturbed run now stops on a per-cell tick budget read off the stock
   trace; wall is a safety cap only. (§5.4)
4. **All-censored sibling sets** had no stated rule and would have been
   silently labeled by proxies. Dropped from the policy loss; the band's
   2× budget exists precisely to create solved siblings. (§6.2)
5. **Ranking loss adopted formally** in place of advantage regression;
   the body already assumed one-knob children and per-head labels. (§6.2)
6. **The solver cannot know the wall limit** for the horizon feature;
   `SAT_WALL_LIMIT` is passed by the harness. (§3.4, §7)
7. **`reorder` default is 2 (both modes)**, not stable-only; masking text
   and review item corrected. (§2.1, §12)
8. **Fork with an open proof file** would corrupt the DRAT; refused. (§7)
9. **Child memory budgeting had no data**: the harness records no peak
   RSS; the stock trace now logs it. (§7)
10. **Stale text** from earlier encodings (behaviour cloning "on fire
    decisions", a "fixed sample of 64", single-cadence pseudo-code,
    "parent continues as stock" in DAgger rounds, offline-RL discount
    presented as general) aligned with the current design. (§2.3, §3.6,
    §5.3, §6.2, §4)
11. **Cost and sequencing** updated for DAgger rounds; the end-to-end
    host budget is about a week. (§5.4, §9, §10)
12. **Gate roles** clarified: the medium gate stays the repo's formal
    promotion gate; the 400-cell wall run and the tick-deterministic
    comparison are this project's decision evidence. (§8)

Open after this pass (updated 2026-09-15): none of the earlier items;
the runtime rail is **dropped** — one-shot `m = 0` plus the interval
floor cover the thrash case it was meant for. Resolved: 4 × 3 branching; converter is a resumable one-time
pass; hold length eliminated by making X_d the segment (2^27); curriculum
rejected; simple per-knob rankers adopted before the net; shuffle
augmentation parked.

---

## 15. Fourth fresh-eyes review (2026-09-15): implementer's pass

**Errors fixed**

1. **`m = 0` held for a 2^27-tick decision epoch would refire a timer
   dozens of times** (the floor is 0.1 × stock delta, the epoch ~25 k
   conflicts). `m = 0` is now one-shot: fire once, revert to 1 for the
   rest of the epoch. (§2.1, §2.3)
2. **`search_ticks` is the wrong clock for budgets and reward.**
   Eliminate's dense propagation lands only in the never-printed
   `statistics.ticks`, and its resolutions have no tick equivalent, so a
   `search_ticks` budget and terminal `t` made elimination free. Defined
   the work clock `W = statistics.ticks + k_res × eliminate_resolutions`
   and used it for `SAT_LIMIT_TICKS`, `B_cell`, and `t`. (§3.4, §7)
3. **Five parents per cell were redundant.** Parents are deterministic,
   so all five ran the same prefix. One parent carries all branch points
   and blocks on a semaphore when too many children are alive — free,
   because nothing is measured in wall. (§5.3, §5.4)

**Gaps filled**

4. **Which children at a branch point** was unspecified. One knob, its
   whole menu: a complete ordering for one head per branch point. (§5.3)
5. **Per-head data is thin** (~750 labeled states per knob per round).
   Added knob triage — step 0 and round 0 rank knobs by effect size,
   later rounds concentrate on the top 3-4 — and noted that branch points,
   not parents, are the cheap lever now. (§6.1b)
6. **Sibling SAT/UNSAT agreement** is a free correctness oracle the
   collector now asserts. (§7)
7. **Fork hygiene** (flush before fork, reopen files, inherit RNG) and
   the purity requirement on `observe()`/`log_row()` were unstated. (§7)
8. **Margin τ had no units**; with a pairwise-logistic ranker it is
   log-odds. (§6.4)
9. **Missing-window handling at D0** (zero-fill + validity flag) and
   **active-variables-only** static features. (§3)
10. **Round-0 mix** rebalanced to 1 parent + ~60 children + 3 wild runs;
    wild flavours trimmed since only the offline-RL side line uses them.
    (§5.4)
11. **Runtime rail dropped**; one-shot `m = 0` plus the floor cover it.

Nothing else open. Ready to break into tasks: step A′ (ticks in TSVs),
step 0 (constant sweeps), step A (solver plumbing), step B (collector +
stock traces), then rounds.
