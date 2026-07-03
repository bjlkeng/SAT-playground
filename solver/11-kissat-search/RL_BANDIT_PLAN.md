# Contextual Bandit / RL Search Controller for Solver 11 — Design Plan

**Status:** plan, rev 3 — second review + round-2 deep research integrated · **Date:** 2026-06-11 · **Target:** `solver/11-kissat-search`
**Objective metric:** the repo's lexicographic metric — (1) solved, (2) total conflicts on ties,
(3) PAR-2 — over `benchmarks/profile20`, N=10 seeds, per `CLAUDE.md`.

---

## 1. Executive summary

Build a **contextual multi-armed bandit controller** that makes coarse-grained search-policy
decisions at **restart / rephase / reduce / mode boundaries** inside solver 11, in three phases:

1. **Phase 1 — online non-contextual bandit (UCB/Thompson), no training needed.** Replicate the
   Kissat_MAB recipe (SAT Competition winner 2021, 2022, 2025 lineage): at restart boundaries
   select the branching system; at rephase points select the rephase strategy. Pure online
   learning per instance, O(1) overhead, deterministic per `SAT_SEED`.
2. **Phase 2 — contextual bandit trained offline from logged solving trajectories.** Add a
   `SAT_TRAJ_LOG` instrumentation flag, collect exploration trajectories on a training pool
   *disjoint from profile20*, train a linear policy (LinUCB/LinTS) and a tiny MLP offline in
   Python, export weights, run pure-Rust inference at boundary points only.
3. **Phase 3 (optional, gated on Phase 2 results)** — wider action spaces (restart margin, reduce
   aggressiveness, mode duration, CHB arm) and/or a one-shot per-instance prior network.

The literature is unambiguous on the shape of the winning design (§3): decisions at
restart-or-coarser granularity, dense LBD/search-efficiency surrogate rewards (never wall-clock),
all per-decision bookkeeping O(1), NN inference either once-per-instance or at ≤ per-restart
frequency. Contextual bandits at this granularity are explicitly flagged as *promising and
unexplored* by the SoCS 2025 MAB-for-SAT survey — this plan is novel-but-derisked territory.

Every keep/promote decision goes through the existing gate:
`tools/feature_ablation.py --seedgate` (N=10) → `tools/check_promotion_gate.py --multiseed`,
with the solver-10 floor and shuffled-order validation for anything that smells input-order-lucky.

Rev 3 integrates a second research round (§3.1): the exact deployed reward formulas of the
SC2024/25 MAB lineage (Kissat_MAB-DC, CoReward — verified against source), short-horizon bandit
evidence that challenges UCB-by-default (H21), formal OPE-confounding limits that demote offline
evaluation to screening-only (§8), clause-management findings that constrain reward choice
(H20/H22), and deployed safe-fallback precedents (Kissat-Pred → H23, Conservative UCB → H19).

---

## 2. What solver 11 decides today (decision-point map)

Current default profile (`config.rs:809-826`): focused-stable search + tick mode-switching +
LBD-tiered reduction + VMTF in focused mode + LBD + lucky. All file:line refs below are to
`solver/11-kissat-search/src/`.

| Decision point | Where | Current policy | Bandit-controllable? |
|---|---|---|---|
| Variable selection | `pick_branch_lit` main.rs:5105 | VMTF queue (focused, main.rs:3463) / VSIDS heap (stable) — tied to mode | **Yes — Phase 1 arm** (decouple heuristic from mode at restart boundary) |
| Phase selection | `pick_branch_phase` main.rs:5010 | best→target→saved cascade + focused phase cycling | Phase 3 (arms = cascade variants) |
| Restart trigger | `note_conflict` main.rs:4964; EMA logic main.rs:4934, 4906-4920 | focused: fast/slow LBD-EMA margin; stable: reluctant doubling (main.rs:4881) | **Yes — Phase 2/3** (margin arm, or MLR-style contextual trigger) |
| Restart execution + trail reuse | `perform_restart_if_pending` main.rs:5469 | optional trail-reuse flags | **Primary bandit hook point** (period boundary) |
| Mode switching | `maybe_switch_search_mode` main.rs:4777 | fixed schedule: focused interval `nlogpown(conflicts,4)`, stable = prior focused ticks | Phase 3 (extend-vs-switch arm) |
| Rephase strategy | main.rs:5190 / `apply_rephase` main.rs:5248 | fixed cycle best→inverted→original on stable restarts — behind `SAT_REPHASE`, **off by default** (see A2) | **Yes — Phase 1 arm** (4-arm precedent: Kissat-MAB-rephasing, SC2022) |
| Reduce schedule + tiers | `should_reduce_db` main.rs:5680; `reduce_db_lbd_tiered` main.rs:6479 | sqrt-scheduled; tier cuts at 50th/90th glue-use percentiles | Phase 3 (DynamicSAT evidence: clause-DB knobs gave the biggest DAC wins) |
| Chrono backtrack | main.rs:4860 | `SAT_CHRONO` off by default | Phase 3 arm |
| Random decisions | main.rs:5051-5073 | fixed interval, focused only | feature input, not an arm initially |

**Observable state already maintained** (free features): conflicts/decisions/propagations +
per-mode splits, `restart_fast_lbd`/`restart_slow_lbd` and level EMAs (main.rs:1217-1221), trail
length, decision level, learned-clause count/literals, glue-use histograms per mode
(`focused_glue_recent`/`stable_glue_recent`, tier limits main.rs:1259-1260), mode-switch and
restart counters, phase-capture counters (`stats.rs:36-198`). Seeded determinism exists:
LCG `next_random_word` main.rs:5056, seeded by `SAT_SEED`.

**Kissat reference deltas worth knowing** (from `benchmarks/reference-solvers/kissat-latest/src/`):
kissat itself has **no bandit** — all scheduling is deterministic formulas (mode.c:69-110,
restart.c:14-51, reduce.c UPDATE_CONFLICT_LIMIT). Its adaptivity is EMAs + histogram-derived tier
limits (tiers.c:6-61) + a 6-cycle rephase schedule incl. walk-derived phases (rephase.c:86-89).
The bandit lineage (Kissat_MAB → AE-Kissat-MAB) is a *fork family layered on top of* kissat —
which is exactly what we are doing to solver 11.

---

## 3. Literature foundations (what is proven, what failed)

Full survey was done 2026-06-09 (web). Key results, grouped by decision type:

**Branching-heuristic selection (strongest precedent).** Kissat_MAB (Cherif/Habet/Terrioux,
CP 2021 + SAT Comp 2021 **winner**): UCB1 at each restart picks branching ∈ {VSIDS, CHB}; reward
per restart period `X_t = log2(decisions_t) / decidedVars_t`; both score systems kept warm across
switches. Lineage kept winning: Kissat_MAB-HyWalk (SC2022 winner), KissatMabProp (SC2023 ranks
2-4), **AE-Kissat-MAB (SC2025 main-track winner, 327/400)**. The underlying per-variable
heuristics CHB/LRB (Liang et al. 2016) are themselves ERWA bandits with reward = clause-learning
participation; LRB's GLR study (SAT 2017) validated "global learning rate correlates with solve
speed" — the canonical surrogate-validation methodology we copy in E0.

**Restart control.** MLR (Liang et al., SAT 2018): online per-instance *linear regression* on the
last 3 LBDs (+ pairwise products) predicting next-LBD; restart when prediction is bad. MapleSSV
(SC2021): UCB over {uniform, linear, Luby, geometric} restart policies, reward
`Δconflicts/avgLBD`, **discounted counts α=0.95** for non-stationarity. Kissat-Adaptive-Restart
(SC2022): UCB between kissat's two restart modes. RL-Reset (Li et al. 2024): restart-vs-reset
2-arm; **Thompson sampling consistently beat UCB**; huge family-specific win (Satcoin 500/500 vs
13), modest comp-wide.

**Rephasing.** Kissat-MAB-rephasing (SC2022): 4-arm UCB over {best, inverted, original, walk}
replacing the fixed cycle; reward = Kissat_MAB-style metrics per rephase period. Successors
Kissat_MAB_CoRephase / CoReward (SC2024/25).

**Dynamic configuration.** DynamicSAT (Shi et al., CP 2025): UCB over per-parameter moves in
kissat-4.0.0, triggered when formula divergence (clause adds+removes) ≥ 30%, reward sampled every
~100 decisions = `U − avgLBD`; **all 10 variants beat baseline; +3.6% solved mean, best
+5.3% solved / −19% PAR-2**; biggest wins from clause-DB parameters; reward read too often
(d=10) *hurt*. Code: github.com/cure-lab/DynamicSAT.

**NN guidance.** NeuroCore (SAT 2019): GNN unsat-core predictions periodically **overwrite the
activity array** (+10-11% solved, GPU, async). NeuroBack (ICLR 2024): graph-transformer backbone
phase prediction run **once per instance on CPU** → +5.2% solved in kissat. RLAF (2025): one-shot
GNN per-variable weights/polarities trained by policy-gradient with **reward = −solver cost**;
>2× speedups, beats supervised imitation. Graph-Q-SAT (NeurIPS 2020): per-decision GNN-DQN cuts
iterations 2-4× but **does not win wall-clock** — model evaluation is the bottleneck; benefit
concentrates in the first ~10 decisions.

**Cross-cutting negative results / constraints:**
- Per-decision NN inference has never won wall-clock in a competitive solver. Tractable
  frequencies: per-restart (bandits), per-10⁴-conflicts async (NeuroCore), once-per-instance (NN).
- Rewards are non-stationary (the formula changes between pulls): use discounted counts, sliding
  windows, or periodic resets — vanilla lifetime UCB over-commits.
- Init-only signals get overwritten by solver dynamics (Eriksson 2026 negative result) — inject
  into persistent state, not just initial order.
- Learned policies often produce family-lottery wins (RL-Reset/Satcoin, Nejati/crypto) — exactly
  what this repo's multi-seed aggregate gate and no-overfit-guards rule exist to catch.
- Deployed winners are deterministic given a seed (UCB argmax; DynamicSAT fixes seeds); Thompson
  must be driven by the solver's seeded PRNG.
- **No published offline-RL/decision-transformer-from-CDCL-trace work exists** — Phase 2's
  offline-contextual-bandit-from-logged-trajectories is a genuine open niche; nearest neighbors
  are RLAF (episodic policy gradient) and Nejati CP 2020 (offline ranking for D&C splitting).

### 3.1 Round-2 deep-dive findings (2026-06-10; raw payloads: `log/deepresearch-rl-bandit-2026-06-10-salvage.md`)

**Exact deployed reward formulas** (the round-1 unknowns, now resolved from primary sources):

- **Kissat_MAB-DC (SC2024, NUDT):** reward `r_t = log2(decisions_t) / log2(conflicts_t)` since
  the latest restart — a decisions-to-conflicts ratio, chosen after the authors found "the bigger
  decisions over conflicts, the more efficient the branching heuristic." Beat Kissat_MAB, HyWalk,
  and MAB_Conflict+ on 800 SC22/23 instances, advantage **concentrated on satisfiable
  instances**. (Their "2nd place SC2024" claim is overstated vs official results — 144 solved,
  PAR-2 2740 — treat rankings in solver descriptions skeptically.) Their stated motivation: the
  standard `log2(decisions)/decidedVars` family is "not well-aligned with CDCL, biasing arm
  selection on hard instances" — a direct published critique of our pre-registered R1.
- **Kissat_CoRephase_CoReward (SC2025, NUDT)** — the "three targeted dimensions" reward,
  **verified 4× against the authors' repo** (`src/rephase.c:394-455`,
  github.com/2317891476/Kissat_CoRephase_CoReward):
  `reward = 0.4·conflict_rate + 0.1·avg_lbd + 0.5·unsigned_num`, where
  `conflict_rate = log2(decisions)·log2(conflicts)/decidedVars²`, `avg_lbd = 1/median_LBD` (live
  redundant clauses), `unsigned_num` = an inverse-remaining/eliminated-variables term (paper says
  `1/(1+remain_vars)`, code computes `1/(vars−active+1)` — they differ in direction). Three
  implementation quirks worth knowing: (a) each component is normalized by a **folded z-score**
  `|x−μ|/σ` with hard-coded offline-fit constants — the comment says "z-score" but the code
  discards the sign, so the reward favors *atypical* periods in either direction (design choice
  or bug; see H24); (b) per-arm value uses EMA `α = 0.7^age` — inverted recency (long-idle arms
  keep stale estimates); (c) UCB1 exploration constant is **0.01**, orders of magnitude below
  textbook √2 — the deployed-winner lineage explores far less than theory suggests. Arms are
  rephase *triples* {BWO, BWI, BWC, BWF}; one pull per 3 rephase events — pulls are scarce.
  Note the 0.5-weighted variable-elimination component is convergent with our R6.
- **AE_kissat2025_MAB (SC2025 winner lineage):** replaces the fixed UCB constant with
  `adaptive_c = mabc/(momentum × (stable_restarts+1))`, momentum ×1.1/×0.9 against a
  window-10 average gain; authors report ~3% PAR-2 on SC2024.
- **Kissat-CURE (SC2025):** kissat 4.0.2 + DynamicSAT-style MAB config tuning + **cold-restart
  arms** — forget branching order / forget phases / forget clauses (from arXiv:2404.16387).
  **Kissat_MAB_ESA:** no reward change (BVE-ordering tweak only). **Kissat-Pred (SC2025):**
  embedded *quantized decision trees* over linear-time graph features predict SAT/UNSAT +
  difficulty; the conservative variant adjusts only restart intervals and mode-switch periods and
  **falls back to defaults when the prediction is UNKNOWN** — a deployed in-run-classifier +
  safe-fallback precedent (H23).

**Short-horizon / non-stationary bandit theory (changes our default-index thinking):**

- At **h≈70 pulls, k=3 arms** — close to our pulls-per-episode regime — ε-greedy/ε-decreasing
  (+ simple regression oracles) **beat UCB1, tuned UCB, PHE, and Thompson**; UCB1 is
  hypersensitive to its exploration constant at short horizons; **UCBT** (parameter-free,
  variance-based Student-t bound) ≈ optimally-tuned UCB1 (IEEE CoG 2020, arXiv:2102.05263).
  Combined with CoReward's deployed c=0.01: confidence-bound exploration at textbook strength is
  probably wrong for this setting (H21).
- Garivier & Moulines (ALT 2011) parameter recipes: discount `γ = 1−¼√(Υ_T/T)`, window
  `τ = 2√(T·logT/Υ_T)`; D-UCB/SW-UCB significantly beat Exp3.S under abrupt change (the right
  model for mode/phase transitions) — softmax/adversarial bandits are the wrong default here.
  Discounted Thompson (arXiv:2305.10718, one γ on Gaussian posteriors) empirically beats both.
- **Meta-learned priors across instances** have a formal recipe: MTSS (AISTATS 2022) /
  Meta-Thompson hierarchical Bayes — learn per-arm priors across instances offline, start each
  episode warm. This is H14 with published machinery.
- **Conservative Bandits (ICML 2016):** anytime constraint "cumulative reward ≥ (1−α)× the
  default arm's," enforced by playing the default whenever a budget lower-bound goes negative;
  the safety price is only additive O(K/(αμ₀)) and provably unavoidable. Caveat: the *anytime*
  constraint suppresses early exploration — expensive at <100 pulls/episode (H19).

**OPE formal limits (demotes Phase 2 offline evaluation to screening, on proof not just policy):**

- Replay/IPS unbiasedness requires i.i.d. contexts + randomized logging (Li et al., WSDM 2011) —
  a CDCL run violates i.i.d. **outright**: the arm at restart k shapes the context at k+1. Also
  ~K·T logged events per T replay steps, and no concentration guarantee when evaluating an
  *adaptive* (learning) policy — repeat and average replays.
- FQE/DR are provably biased under unobserved confounders **with memory** — and the hidden
  solver state (heap order, clause DB composition) persists across restarts; error can be Ω(H)
  even with infinite data (Kausik et al. arXiv:2211.16583; Bruns-Smith & Zhou arXiv:2302.00662).
  Mitigations that survive: sensitivity-sweep acceptance tests, pessimistic value lower bounds,
  and warm-starting online learning with robust bounds (H14's offline-prior shape).
- Logs collected by an *adaptive* policy additionally break IPS/DR asymptotic normality; valid
  inference needs an exploration floor `e_t ≥ C·t^(−α)`, α < ½, plus stabilized weights (Zhan et
  al., KDD 2021). Uniform-random logging sidesteps all of it — keep `explore` as the only
  training-data mode. Log exact propensities at decision time; never re-estimate post hoc
  (calibration failures produce up to 50% OPE error from 0.06 propensity error — Raghu et al.,
  ICML 2018). For the estimator itself, SWITCH-DR dominates IPS/DR/DM at small samples (Wang et
  al., ICML 2017).

**Clause-management / reward-validity findings:**

- **CrystalBall (Soos):** clause-usefulness labels from DRAT ("used >5× in next 10k conflicts");
  LBD≤3 clauses used ~30× vs ~6× for LBD≥4 — yet the **hand-crafted deletion heuristic still
  beat the full ML system end-to-end**. ML-for-clause-deletion has never beaten hand-crafted in
  a competitive solver (also: AITP 2020 RL deletion — preliminary only; its episodic reward
  `200 − op×10⁻⁷` with op = propagation clause-accesses, and its observation "op highly
  correlated with wall-clock," directly support our R7/tick-normalization).
- **arXiv:2602.20829 (Feb 2026):** on arithmetic-circuit families (multipliers), **LBD collapses
  to clause length** and carries no usage information — LBD-based rewards (R2/R3/R5, DynamicSAT's
  U−avgLBD) are *uninformative on whole families* (H20). Their LBD-free usage counter (+1 per
  BCP/analysis use, −1 every T=4096 conflicts, delete a `0.90 − 0.40/log10(r+9)` fraction of
  zero-score clauses) beats kissat 4.0.2: 60/60 vs 56/60 on multipliers (PAR-2 −40%), 301 vs 297
  on SC22 main — rule-based, no ML.
- CaDiCaL 1.9.4 negative result: treating glue-shrink-during-strengthening as clause "use"
  caused a regression, reverted in 1.9.5 — plausible usefulness proxies can be net-negative.
- Biere-group adaptive machinery details (SC2024/25 descriptions): tier1/tier2 = glue at
  50th/90th percentile of *used* clauses, computed per mode but **only focused-mode limits used
  in both modes**; per-tier vivification ticks budgets with rollover to the next tier; SC2024
  kissat caps reason-side bumping by ticks *conditioned on decision rate* — each a hand-tuned
  feedback rule a contextual policy could subsume. Kissat was frozen 2024→2025 (effort went to
  porting kissat techniques into CaDiCaL, 46→64 KLOC).

---

## 4. Decisions to drive (action spaces, ranked)

Defined relative to the current default profile. Every arm set includes the current default
behavior as an arm, so the bandit's worst *asymptotic* case is "always pick default" — the
residual downside is bounded exploration cost, capped by the cold-start guard, default-arm
pseudo-counts, and discounting (§10.4).

### Phase 1 (online bandit, no training)
- **A1 — Branching system, chosen per restart**:
  arms = `{vmtf, vsids}`. Requires decoupling heuristic from mode and keeping both warm: VMTF
  stamps and VSIDS activities are *already both updated* on conflicts (main.rs:3448, 4522) in
  focused-stable mode; verify and enforce both-warm when `SAT_BANDIT≠off`. Direct Kissat_MAB
  analog (they used VSIDS vs CHB; we start with the two systems we already have).
  **Caution — novel pairings:** in the default profile the branching system is coupled to mode,
  and restart/phase policies auto-resolve from mode too (`effective_restart_policy`
  main.rs:4683-4692, `effective_phase_policy` main.rs:4694-4709). A mode-free A1 arm therefore
  creates pairings that have never been measured (e.g. VMTF under stable-mode reluctant
  restarts). Start with **per-mode arm sets** — focused: `{vmtf (default), vsids}`, stable:
  `{vsids (default)}` — and only open the stable-side arm after the focused arm proves itself in
  E5. Novel pairings are search space to explore deliberately, not free wins.
- **A2 — Rephase strategy, chosen at each rephase point** (stable restarts):
  arms = `{none, best, inverted, original}` (+ `walk` if/when local search lands).
  **Correction vs rev 1:** `SAT_REPHASE` is **off** in the current default profile (config.rs:693
  default; the profile block config.rs:809-826 does not enable it), so this action space is
  really *bandit-gated rephasing vs default-off*. The `none` arm IS current default behavior and
  must be in the arm set; the gate comparison is against a no-rephase baseline, and any A2 win is
  partly "rephasing helps at all," not purely "the bandit chose well" — E5's fixed-arm ablation
  (always-best, always-inverted, …) separates the two. Replaces the fixed cycle at
  main.rs:5249-5254 when a non-`none` arm fires.

### Phase 2 (contextual, offline-trained)
- Same arms as Phase 1, plus:
- **A3 — Focused restart margin**, re-chosen each period: arms = margin multiplier
  `{0.95, 1.0 (default), 1.1, 1.25}` applied to the EMA restart test (main.rs:4906-4912).
- **A4 — Reduce interval scale**, chosen at each reduce: arms = `{0.5×, 1× (default), 2×}` on the
  next `reduce_db_limit` (main.rs:5750-5759). DynamicSAT's biggest wins were clause-DB knobs.

### Phase 3 (gated on Phase 2 evidence)
- **A5 — Mode-duration arm**: at each mode switch point, `{switch now (default), extend 1×, extend 3×}`.
- **A6 — CHB implementation** as a third branching arm `{vmtf, vsids, chb}` — ERWA per-variable
  scores, cheap, the exact SC-winner arm set.
- **A7 — Tier-cut percentile arms** for LBD-tiered reduce `{(50,90) default, (30,80), (70,95)}`.
- **A8 — Chrono on/off arm** per period.

**Explicitly rejected:** per-decision variable selection by NN (Graph-Q-SAT wall-clock negative);
per-conflict model evaluation of any kind (only O(1) accumulator updates allowed per conflict).

---

## 5. Context features (the "contextual" part)

All features must be O(1)-maintained or already tracked. Computed/normalized **only at boundary
points**. Three blocks, ~40 dims total:

**Static (computed once at parse; SATzilla-lite):**
`log(num_vars)`, `log(num_clauses)`, clause/var ratio, fraction binary / ternary / long clauses,
fraction horn clauses, fraction positive literals, post-preprocessing deltas of the above.

**Run-state (cumulative, at boundary):**
`log1p(conflicts)`, `log1p(decisions)`, `log1p(propagations)`, GLR = conflicts/decisions,
propagations/conflict, `restart_fast_lbd`, `restart_slow_lbd`, fast/slow ratio, level EMAs +
ratio, trail_len/num_vars, root-fixed fraction (level-0 assignments/num_vars),
max-trail-fraction ever reached + its recent trend (cheap SAT-likelihood proxy, H16),
live_learned/original clause ratio, learned-literal growth rate, mode one-hot, conflicts- and
ticks-in-current-mode, mode_switches, restarts, reduce_db_calls, current tier1/tier2 cuts.

**Period-delta (since last boundary; reset each period):**
Δconflicts, Δdecisions, Δpropagations, period mean/var learned LBD, period mean backjump length,
period mean trail at conflict, distinct-vars-decided (for the Kissat_MAB reward), period GLR,
fraction of conflicts at decision level ≤ 5.

**Bandit-self features:** per-arm pull counts and discounted mean rewards (lets a contextual
policy meta-reason about its own exploration state).

Normalization: `log1p` where noted, then fixed affine `(x−μ)/σ` with constants computed from the
training corpus and **baked into the exported weights file** — no runtime adaptivity, keeps
inference deterministic and train/serve identical. **Clip** each normalized feature to the
training-corpus envelope (±4σ). If features clip persistently (an out-of-distribution run —
inevitable on long runs, since cumulative counters drift past anything seen at the 600 s
collection horizon), the controller falls back to the online-UCB index or the default arm rather
than trusting extrapolated weights; clip rates are logged and reported in every eval.

---

## 6. Reward function

### 6.1 Per-period surrogate (dense, drives the bandit)

The repo metric is lexicographic solved→conflicts, so the ideal dense reward is "search progress
per conflict spent." Progress is unobservable; the literature's proven surrogates are all
LBD/efficiency proxies. **Candidates to log and validate (E0) before committing:**

| id | reward for period t | precedent |
|---|---|---|
| R1 | `log2(decisions_t) / decidedVars_t` | Kissat_MAB (SC21 winner) |
| R2 | `Δconflicts_t / avgLBD_t` | MapleSSV |
| R3 | `U − avgLBD_t` (U fixed cap) | DynamicSAT |
| R4 | period GLR = `Δconflicts_t / Δdecisions_t` | GLR study (Liang 2017) |
| R5 | `−mean(learned LBD_t)` z-scored against the run's own slow EMA | MLR-flavored |
| R6 | root-fixed-vars delta + learned-binary delta (proof-progress proxy) | novel here; convergent with CoReward's 0.5-weighted variable-elimination term (SC2025, §3.1) |
| R7 | E0-winner recomputed per `Δticks` instead of per conflict (tick-normalized efficiency) | guards arm-cost asymmetry (H11); AITP 2020 used op-count (≈ticks) episodic reward, "op highly correlated with wall-clock" |
| R8 | `log2(Δdecisions_t) / log2(Δconflicts_t)` | Kissat_MAB-DC (SC2024); beat the R1 family on 800 SC22/23 instances, esp. on SAT instances |

Two design rules from the deployed-reward deep-dive (§3.1): use **signed** normalization (the
CoReward folded `|x−μ|/σ` rewards atypicality in either direction — replicate only deliberately,
as H24); and **report E0 correlations per benchmark family** — LBD-family candidates (R2/R3/R5)
are known-uninformative on arithmetic-circuit-like families where LBD degrades to clause length
(H20), so a headline reward must be family-robust or blended with a non-LBD component (R6/R8).

**E0 (surrogate validation, run before any bandit ships):** instrument trajectory logging on the
*current default* (no behavior change), run the training pool × 5 seeds, and measure Spearman
correlation between each candidate's prefix-mean and the final outcome rank (solved-at-budget,
then total conflicts). **Correlate within-instance only** — across seeds (and later across
exploration draws), never pooled across instances. Pooled correlation mostly measures instance
hardness (easy instances have low LBD *and* solve fast) and would anoint any hardness proxy as a
"good reward"; report the per-instance correlation distribution, not one pooled number. Pick the
best-correlated; pre-register R1 as the default choice (strongest competition pedigree) and
require a challenger to beat it clearly. **Reward-hacking monitor:** re-run this analysis on the
*deployed* policy's trajectories whenever a policy ships — a policy that raises the surrogate
without raising within-instance outcomes has gamed the reward, and the surrogate's validity under
the new trajectory distribution must be re-established before further training rounds.

All candidates are deterministic per (config, seed) and contention-immune — consistent with the
repo's conflicts-over-PAR2 philosophy. **Wall-clock is never a reward input.**

### 6.2 Non-stationarity handling

Discounted statistics (MapleSSV-style): arm counts and reward sums decay by α=0.95 per pull
(sweep α ∈ {0.9, 0.95, 0.99, 1.0} in E2). Optionally reset bandit state at mode switches
(kissat's EMA-reset precedent, main.rs:4813-4850 already resets EMAs there). The E2 index-policy
matrix must also include the short-horizon-favored alternatives (§3.1, H21): UCBT
(variance-based, parameter-free), ε-greedy/ε-decreasing (ε ≈ 0.03-0.11 per the short-horizon
study), discounted Thompson, AE-style adaptive exploration constant, and a UCB-c sweep extending
down to the deployed winner's 0.01 — not just textbook √2-scale values.

### 6.3 Terminal reward (offline training only)

For offline policy learning, augment per-period rewards with a terminal signal aligned to the
lexicographic metric: `+B` if solved within budget else 0, and `−log(total_conflicts)` scaled to
the per-period reward range; censored (timeout) episodes record the conflicts actually reached at
the budget plus a censoring flag (a lower bound on true cost — do not fabricate a value). Start with pure per-period reward (bandit assumption: periods independent); terminal-aware
credit assignment is hypothesis H4, not the baseline.

### 6.4 Measurement windows — the self-confounding trap

Several arms influence **how long their own measurement window is**: a restart-margin arm (A3)
directly controls restart frequency, and even the branching arm changes the LBD stream that
drives the EMA restart trigger. Comparing raw per-period rewards across arms then confounds "arm
was good" with "arm made the period short." Rules:

- Reward candidates must be **window-length-insensitive** before they are compared across arms.
  As defined in §6.1 they are not all so: R2's and R6's numerators are raw period sums (by design
  in MapleSSV, which deliberately rewards throughput), and R1's `log2(decisions)` dampens but
  does not remove length sensitivity. E0 therefore evaluates each candidate in two forms — as
  defined, and per-conflict-normalized (e.g. R2′ = `1/avgLBD_t`, R6′ = deltas per 1k conflicts) —
  and any candidate driving variable-length windows must be the normalized form, or be scored
  under fixed blocks (H17).
- Each decision point gets its **own** window: branch arm = restart→restart, rephase arm =
  rephase→rephase, reduce arm = reduce→reduce, margin arm = fixed conflict blocks (below). A
  rephase arm scored on a single restart period is mis-measured — its effect spans several
  restarts.
- Periods that span a mode switch are **closed at the switch** and the partial period is dropped
  from arm statistics (the mode change, not the arm, dominates that delta).
- **Contingency (H17):** if E2/E3 arm comparisons look unstable, decouple measurement from
  control entirely — score over fixed 1000-conflict blocks regardless of when decisions fire,
  attributing each block to the arm(s) active during it.

---

## 7. Trajectory data generation

### 7.1 Instrumentation (`SAT_TRAJ_LOG=<path>`)

New flag; when set, the solver appends **JSONL**: one `header` record (instance hash, static
features, full config, seed, arm definitions), one `period` record per boundary (period index,
boundary type, **raw counters — not the engineered feature vector** (features are derived offline
in `build_dataset.py`, so feature re-engineering never forces a re-collection), action taken,
**behavior-policy propensity**, all reward candidates R1-R7 incl. Δticks, cumulative counters),
one `final` record (status, conflicts, ticks, proof bytes, wall time). Buffered writes, flush at period boundaries only; zero cost when unset.
Rough volume: ≤ a few thousand periods × ~1 KB ≈ ≤ 5 MB/run — fine uncompressed in `log/`.

### 7.2 Behavior policy for collection

`SAT_BANDIT=explore`: choose arms **uniformly at random** from the solver's seeded LCG →
propensity = 1/K exactly, logged at decision time (never re-estimated post hoc — §3.1's
calibration result). Collect a smaller slice under `SAT_BANDIT=ucb` (Phase 1 policy) for
offline-evaluation realism only — adaptive-policy logs are formally unusable for unbiased OPE
without exploration floors (§8, step 3), so uniform logs are the only *training* data.

### 7.3 Instance pool — **train/test hygiene**

- **Training pool:** ~100-150 instances sampled from `benchmarks/sat-comp-2025-medium` (plus
  generator families `benchmarks/random-3sat/`, `benchmarks/crypto/` for diversity), filtered to
  "default solves in 30-600 s" **plus** ~25% unsolved-at-600 s headroom instances. **Explicitly
  exclude all 20 profile20 instances and anything sharing a family/provenance row with them**
  (check `benchmarks/profile20/selection.csv`). Record the pool manifest in
  `log/traj-pool-manifest.csv`.
- **Evaluation:** profile20 stays held-out; it is only ever touched by the seedgate.
- Split train/validation **by benchmark family**, never by instance or row (competition sets
  contain many near-duplicate variants from one generator/family — instance-level splits leak;
  rows within a run are trivially correlated).
- **Reweight by episode:** a 600 s timeout run emits orders of magnitude more period rows than a
  30 s solve, so unweighted training optimizes behavior on hopeless runs. Weight rows by
  `1/periods(episode)` so each run counts equally, and report trained-policy metrics with and
  without the censored-episode slice to confirm the policy isn't dominated by timeout dynamics.
- **Per-run reward normalization (offline only, H12):** LBD scales differ by an order of
  magnitude across instances, so pooled regression on raw rewards mostly learns instance
  identity. Z-score (or rank-transform) rewards **within each episode** before offline training.
  The online bandit needs no normalization — it only ever compares arms within one run.

### 7.4 Budget (realistic)

Collection run = pool(≈120) × seeds(5) × avg runtime ≈350 s (600 s cap) ≈ 58 core-hours per
exploration config → **~15 wall-hours at 4 jobs**. Run detached via the one-shot cron pattern
(CLAUDE.md), hourly status, respecting the 4-core concurrency bar. Start with a 40-instance ×
3-seed pilot (~4 h) to shake out the format before the full run. Because periods log raw counters
(§7.1), feature re-engineering needs **no** re-collection; only arm/action-space changes or newly
needed raw counters force one. Version the log schema (`traj_schema: 1`).

---

## 8. Training pipeline (`tools/bandit/`)

```
tools/bandit/
  collect_trajectories.py   # orchestrates runs (taskset cores, mem caps, pool manifest, seeds)
  build_dataset.py          # JSONL → npz; family-level split; feature derivation from raw counters; normalization constants
  train_policy.py           # models: ridge-per-arm (LinUCB/LinTS), tiny MLP Q(s,·)
  eval_offline.py           # OPE: IPS / SNIPS / doubly-robust, CIs; vs logged UCB + uniform
  export_weights.py         # weights → flat f32 .bin + .json metadata (feature order, μ/σ, arms, sha)
```

1. **Dataset:** rows = (context x, action a, propensity p, rewards r₁..r₇, episode id, terminal).
2. **Models (in order):**
   - *Ridge regression per arm* → LinUCB/LinTS parameters. Interpretable; coefficients tell us
     *which* context features matter (itself a deliverable — they may suggest a hand-coded rule).
   - *Tiny MLP* Q(x,·): 40 → 64 → 64 → K, ReLU, ~7k params, trained with weighted regression on
     the chosen reward (importance weights from propensities) or doubly-robust objective.
3. **Offline policy evaluation gate — screening only, with known formal limits (§3.1):** the
   learned policy must beat both the uniform behavior policy and the simulated-UCB policy on
   estimated reward with a bootstrap CI excluding zero **before** any solver wall-time is spent
   on it (cheap kill-switch for bad models). But OPE here is *provably* only a screen, not a
   verdict: contexts inside a run are not i.i.d. (the arm at restart k shapes the context at
   k+1 — replay unbiasedness fails), and the unlogged solver state is a confounder *with memory*
   (FQE/DR bias can be Ω(horizon) even with infinite data). Practice rules: estimate per-period
   reward effects (valid-ish) rather than whole-episode values (invalid); use SWITCH-DR as the
   estimator; stratify within-instance (instance identity is an *observed* global confounder —
   exploit that); train/evaluate ONLY on uniform-random (`explore`) logs — if logs from an
   adaptive policy are ever used, they need an exploration floor `e_t ≥ C·t^(−α)`, α < ½, plus
   stabilized weights, so just don't. The seedgate remains the only decision metric.
4. **Export:** little-endian f32 blob + JSON sidecar. Loaded via `SAT_BANDIT_WEIGHTS=<path>`;
   once a model is promoted, bake it into the binary via `include_bytes!` (build.rs) so the
   default needs no runtime file. The sidecar sha goes into `SAT_STATS_JSON` output for
   provenance.
5. Python deps: numpy + scikit-learn for linear; PyTorch (CPU) only if the MLP earns its keep.
   Everything under `tools/bandit/`, no new Rust deps.

---

## 9. Model architecture

### 9.1 Linear (default candidate)
Per-arm ridge weights, d≈40, K≤4 per decision type. Inference = K dot products at a restart
boundary (~ns). **LinTS** exploration driven by the solver LCG (deterministic per seed); LinUCB
as the deterministic alternative. This is the architecture to beat — the literature (MLR,
Kissat_MAB) says linear-at-coarse-granularity is already competitive.

### 9.2 Tiny MLP
`f32` MLP 40 → 64 → 64 → K (~7k params), fixed-size arrays, no heap allocation, no SIMD needed.
Worst-case eval frequency is bounded by the minimum restart interval (~50 conflicts): at
10⁴-10⁵ conflicts/s that is potentially **hundreds to ~2k evals/s**, not "a few" — but at ~14k
FLOPs/eval that is still < 0.01% of a core's throughput; confirm empirically in E7 rather than by
estimate. Pure-Rust forward pass in `src/bandit.rs` (~50 lines), no inference crate.
Deterministic (no dropout at inference; fixed weight blob).

### 9.3 Temporal context without recurrence
No RNN. Temporal signal enters via two-timescale EMA features (fast/slow) of the key period
stats — same trick kissat uses for restarts — keeping the model stateless and the
train/serve gap zero.

### 9.4 Explicitly deferred
GNN over the formula (NeuroBack/RLAF-style one-shot prior): highest ceiling, but needs a GPU
training pipeline, a graph encoder in Rust or an external preprocessing step, and a labeled/RL
training corpus. Revisit only if Phase 2 plateaus and the gap analysis points at
instance-level (not state-level) signal. Noted as H6/H7 below.

---

## 10. Solver integration

### 10.1 New module `src/bandit.rs`

```rust
pub(crate) struct BanditController {
    decisions: Vec<DecisionPoint>,   // one per enabled action space (A1, A2, ...)
    model: PolicyModel,              // Ucb { c, alpha } | Thompson | Linear { W, mu, sigma } | Mlp { ... }
    period: PeriodAccumulators,      // Δconflicts, Δdecisions, decided-var bitset/count, LBD sum/sq, ...
    // all randomness via &mut solver.random_state (existing seeded LCG)
}
```

API: `on_conflict(&mut self, lbd, backjump, trail_len)` — O(1) accumulator bumps, **no model
eval**; `select(&mut self, dp: DecisionPoint, ctx: &Features) -> Arm` at boundaries;
`close_period(&mut self, dp) -> Reward` computes the surrogate and updates online stats.

### 10.2 Hook points (exact)

| Hook | Location | Action |
|---|---|---|
| Conflict accounting | `note_conflict` main.rs:4964 (or next to `stats.conflicts += 1` main.rs:7804) | `on_conflict(...)` |
| Restart boundary | `perform_restart_if_pending` main.rs:5469 | close period → reward → select A1 (branching) / A3 (margin) for next period |
| Branching dispatch | `vmtf_branching_active` main.rs:3421 / `pick_branch_lit` main.rs:5105 | consult bandit-selected arm instead of mode-only rule |
| Rephase | main.rs:5190 → `apply_rephase` main.rs:5248 | select A2 arm instead of fixed cycle |
| Reduce | `should_reduce_db` main.rs:5680 / `schedule_next_lbd_reduce_db` main.rs:5750 | A4 interval-scale arm |
| Mode switch | `maybe_switch_search_mode` main.rs:4777 | A5 arm (Phase 3); also optional bandit-state reset |
| Trajectory log | same boundaries | emit JSONL when `SAT_TRAJ_LOG` set |

**Both-warm requirement (A1):** when `SAT_BANDIT≠off`, ensure conflict analysis always updates
*both* VSIDS activities (main.rs:4522) and VMTF stamps (main.rs:3448) regardless of the active
arm, and that heap/queue rebuilds on arm switch reuse the existing reorder machinery
(`maybe_reorder_branching` main.rs:4836, mode-switch refresh main.rs:4813-4850). Memory cost: nil
(both structures already exist); time cost: one extra stamp move per analyzed var when the VSIDS
arm is active — measure in E7.

### 10.3 Config flags (repo convention)

```
SAT_BANDIT=off|ucb|thompson|linucb|lints|mlp|explore|roundrobin   (default off)
#   explore   = uniform-random arms from the seeded LCG (collection behavior policy AND
#               E1's seeded-random control — same thing, propensity 1/K)
#   roundrobin = deterministic arm rotation (E1's second non-learning control)
SAT_BANDIT_ARMS=branch,rephase[,restart-margin,reduce,mode]
SAT_BANDIT_UCB_C=<f64>          # exploration constant (Kissat_MAB used small c; sweep)
SAT_BANDIT_DISCOUNT=<f64>       # 0.95 default, 1.0 = vanilla
SAT_BANDIT_WEIGHTS=<path>       # linucb/lints/mlp; default = baked-in blob
SAT_TRAJ_LOG=<path>             # trajectory JSONL; independent of SAT_BANDIT
```

All registered in `config.rs` + `CONFIG_SCHEMA.csv`, ablatable via
`feature_ablation.py --env "SAT_BANDIT=ucb ..." --tag ...`.

### 10.4 Invariants

- **Determinism per (config, seed):** every stochastic draw (Thompson, explore, tie-breaks) comes
  from the existing seeded LCG; UCB ties break by fixed arm index; f32 inference is bit-stable.
  Add a unit test: same (instance, seed) twice → identical pull sequence and identical conflicts.
- **Correctness untouched:** the bandit only chooses *which existing heuristic runs*; DRAT
  emission, model validity, and proof paths are unaffected by construction. Full smoke suite +
  `cargo test` after every change (red-green TDD: write the determinism + period-accounting tests
  first).
- **Overhead budget:** < 1% search ticks with `SAT_BANDIT=ucb` (measured in E7); per-conflict
  work is a handful of integer/f64 adds.
- **Cold-start guard (H10):** the bandit stays dormant (default arms) until `conflicts ≥ 10k`
  (sweepable). Easy instances finish within a handful of restarts — the bandit cannot converge
  there and pure exploration can only hurt; expect the easy-10 to be neutral at best. Initialize
  arm statistics with prior pseudo-counts on the default arm so early pulls aren't uniform noise.
- **Switch hysteresis:** minimum dwell ≥ 2 periods per arm bounds heap/queue rebuild churn;
  rebuild lazily on the first decision of the new period, not at selection time.
- **Factored credit assignment:** with multiple decision points enabled, each runs its own
  independent bandit updated from its own window (semi-bandit). This is biased when arms
  interact — E5's per-space ablation exists precisely to expose that; if the combined config
  underperforms the sum of its parts, add the other spaces' active arms to each bandit's
  *context* rather than coupling the bandits.
- **Stats:** extend `stats.rs` with per-arm pulls/mean-reward; surface in `SAT_STATS_JSON`.

---

## 11. Experiments & validation plan

All keep/promote decisions: N=10 seedgate + `check_promotion_gate.py --multiseed`
(lexicographic solved→conflicts→PAR-2, solver-10 floor). 5×5 (`--jobs 5 --seeds 5`) for
iteration only. Long runs detached + hourly status per CLAUDE.md.

| id | Question | Setup | Decision rule |
|---|---|---|---|
| **E0** | Which surrogate reward predicts real outcomes? | Default solver + `SAT_TRAJ_LOG`, training pool × 5 seeds; Spearman(prefix-mean Rᵢ, outcome rank), **within-instance only**, reported **per benchmark family** (§6.1, H20) | Highest within-instance correlation wins, and the winner must be family-robust (no family where it carries zero signal); R1 pre-registered default, R8 the strongest challenger |
| **E1** | Does Phase-1 UCB (A1+A2) beat the default? | `SAT_BANDIT=ucb` vs default vs **seeded-random arms** vs **round-robin arms**, seedgate N=10, timeout 900 | UCB must beat default *and* both non-learning controls (Kissat_MAB's own CP 2021 controls). Beating default but not random ⇒ diversification, not learning (H9) — still shippable, but ship the cheaper policy. Go/no-go for the program |
| **E2** | Index policy & non-stationarity | ucb (c-sweep incl. **0.01**) vs thompson vs discounted-thompson vs **UCBT** vs **ε-greedy/ε-decreasing** vs MOSS × discount {0.9,0.95,0.99,1.0}; + adaptive-c (AE-style); + racing/commit-after-trial (H15) | Best-of on N=10; priors: Thompson ≥ UCB (RL-Reset) but ε/UCBT may beat both at our pull counts (H21) |
| **E3** | Does *context* help online? | LinTS (online, no offline training) vs best E2 config | Gate; also inspect learned coefficients |
| **E4** | Does *offline training* help? | Phase-2 frozen policy vs hybrid (offline prior + online update) vs best online | OPE gate first (§8, step 3), then seedgate |
| **E5** | Action-space ablation | each of A1/A2/A3/A4 alone vs combined | Per-space lexicographic delta; drop spaces that add nothing (fewer arms = faster online learning) |
| **E6** | Overfit / generalization | (a) fresh 30-instance sample from sat-comp-2025-medium not in train pool; (b) `tools/shuffle_sensitivity.py` on any instance the bandit newly wins | Wins must survive reshuffle + fresh sample, else treat as lottery (CLAUDE.md no-overfit rule) |
| **E7** | Overhead isolation | `SAT_BANDIT=ucb` forced to always pick the default arm vs `SAT_BANDIT=off` | ticks delta < 1%, conflicts identical (else the controller itself perturbs search — find out why) |
| **E8** | Promotion | winner vs current default vs solver-10 floor, full gate | Standard promotion bead |

**Diagnostics to record per experiment** (CLAUDE.md step 5): per-instance solve-rate X/N,
median conflicts, P(candidate>default) dominance, arm-pull distributions per instance (did the
bandit actually switch, or collapse to one arm?), per-arm reward traces. A bandit that collapses
to the default arm everywhere and matches default performance is a *correct null result*, not a
failure — it bounds the cost of the machinery.

**Seed-fragile rows** (case9, sudoku-N30-12, REGRandom-K4-L1-Seed40): report but never let a
single-seed flip drive a verdict.

---

## 12. Alternative methods & hypotheses

- **H1 (null):** non-contextual UCB captures most of the available win; context adds little at
  restart granularity. *Test:* E3 vs E2. If true, ship Phase 1 and stop — that is the SC-winner
  configuration and a fine outcome.
- **H2 (granularity):** restart boundaries are the right frequency. *Alternative:* DynamicSAT's
  divergence trigger (act only when the formula has changed ≥ θ%) — fewer, better-timed pulls.
  *Test:* re-run best E5 config with divergence-triggered selection, θ ∈ {10, 30, 50}%.
- **H3 (cheapest contextual win):** an MLR-style *online per-instance linear regressor* on recent
  LBDs predicting next-period LBD, used as a restart trigger — no offline training at all.
  *Test:* implement as a restart-policy variant (`SAT_RESTART=mlr`, fitting the existing enum)
  rather than a bandit mode; compare against offline-trained A3.
- **H4 (credit assignment):** periods are not independent (reduce policy now changes future
  periods), so a bandit is mis-specified; fitted-Q iteration over the period-MDP could do better.
  *Risk:* offline RL instability on ~10⁵ transitions — and round-2 theory (§3.1) shows FQE is
  *provably* biased here (hidden solver state = confounder with memory; bias up to Ω(horizon)).
  *Test:* only if E4 shows the frozen policy underperforming its OPE estimate (the signature of
  horizon effects); use BC-regularized, pessimistic/sensitivity-bounded FQI variants only, and
  let the seedgate alone judge the result — FQE numbers are direction, never evidence.
- **H5 (arm-set expansion):** CHB as a third branching arm replicates the exact SC-winner setup.
  *Test:* Phase 3, after E5 confirms A1 is a live action space.
- **H6 (per-instance prior):** a per-instance *config classifier* — run all K fixed-arm configs
  on the training pool, label each instance with its lexicographic winner, train a classifier on
  parse-time static features, predict the initial arm/prior at solve start (SATzilla-lite,
  one inference, no GNN). *Test:* against uniform prior in E4's hybrid setting.
- **H7 (one-shot GNN prior):** NeuroBack/RLAF-style phase or score initialization. Highest
  ceiling, highest cost; deferred until H1-H6 are resolved and only if instance-level signal
  (H6 working) justifies a bigger model.
- **H8 (reward shaping):** R6 (proof-progress proxy: root-fixed + learned-binary deltas) is novel
  — if E0 shows it correlating better than LBD surrogates, it becomes the headline reward and is
  worth writing up regardless of solver outcome.
- **H9 (diversification, not learning):** mere arm *alternation* — not reward-driven selection —
  may explain most of any win (mixing strategies alone is known to help; this is why Kissat_MAB's
  CP 2021 paper ran random and single-switch controls). *Test:* E1's seeded-random and
  round-robin controls. *Contingency:* if random ≈ UCB > default, ship the simpler stochastic
  alternation and drop the reward machinery — equal performance at lower complexity wins.
- **H10 (value concentrates on long runs):** easy instances give the bandit only a handful of
  pulls — no time to learn; expect neutral-at-best on profile20's easy-10 and all upside on the
  hard-10. *Test:* the easy/hard split already reported at every gate. *Contingency:* the
  cold-start dormancy guard (§10.4); if the easy-10 regress beyond noise, raise the dormancy
  threshold before concluding the bandit failed.
- **H11 (arm-cost asymmetry):** conflict-denominated rewards are blind to per-conflict tick cost;
  an arm can win conflicts-per-period while losing wall-clock — the binary_fast lesson inverted
  (there: cheaper ticks, more search; here: better search, costlier ticks). *Test:* Δticks logged
  per period from M1; E0 scores R7 alongside R1-R6. *Contingency:* if arms differ > 10% in
  ticks/conflict, the reward must be tick-normalized (R7 becomes mandatory, not optional).
- **H12 (reward-scale heterogeneity):** pooled offline training on raw rewards learns instance
  identity, not policy value. *Test:* train with and without per-episode normalization (§7.3);
  the normalized model should win on the family-held-out validation split.
- **H13 (distill to rule):** if the linear policy's value is carried by 2-3 features (ridge
  coefficients + permutation importance), a hand-coded threshold rule may capture the win with
  zero model surface. *Test:* implement the rule, run the same gate. *Contingency:* prefer the
  rule whenever it is within noise of the model — simpler, trivially deterministic, no weights
  provenance to manage.
- **H14 (Bayesian warm start — a-priori best Phase-2 config):** frozen offline weights ignore
  instance idiosyncrasy; pure online learning wastes early pulls. LinTS with an offline-learned
  prior (mean + precision from the corpus) updated online per instance gets both. *Test:* E4's
  hybrid arm vs frozen-offline and pure-online. *Round-2 grounding:* this is exactly the
  meta-learned-prior recipe of MTSS (AISTATS 2022) / Meta-Thompson — hierarchical Bayes pooling
  across tasks (here: instances/families) into per-arm priors; and warm-starting online learning
  with offline-derived robust bounds is the one offline-RL mechanism that survives the
  confounding critiques (§3.1).
- **H15 (racing beats bandits):** commit-after-trial may beat continual selection — race all arms
  round-robin over the first conflict blocks, then lock the per-instance winner for the rest of
  the run. Simpler, lower variance, immune to non-stationarity in the locked phase. *Test:* one
  extra config in E2.
- **H16 (SAT/UNSAT asymmetry):** arms plausibly help differently by instance status (rephasing
  aids satisfiable instances; clause-DB management aids refutations). The max-trail-fraction
  trend feature (§5) is a cheap online SAT-likelihood proxy a contextual policy can condition on.
  *Test:* E0 trajectories checked for reward-vs-final-status interaction; if strong, the feature
  earns its slot and per-status arm priors become a Phase-3 option.
- **H17 (measurement-window decoupling):** if per-period rewards prove unstable, score arms over
  fixed 1000-conflict blocks decoupled from decision boundaries (§6.4).
- **H18 (variance inflation at the gate):** an adaptive controller may *widen* seed-spread (it
  branches internally on early-run noise), leaving N=10 underpowered even when the mean improves.
  *Test:* compare per-instance conflict variance, candidate vs default, in the E1 TSVs.
  *Contingency:* escalate deciding sweeps to N=20 seeds for bandit configs (cost is linear)
  rather than shipping or rejecting on an underpowered gate.
- **H19 (conservative-exploration guard):** a formal champion-challenger gate exists —
  Conservative UCB (ICML 2016) plays the default arm whenever a lower confidence bound on the
  cumulative-reward budget vs `(1−α)×default` goes negative; the safety price is additive and
  provably unavoidable. *But* the anytime constraint suppresses early exploration, which is
  exactly where our few-pull episodes live. *Test:* implement as an optional guard in E2; compare
  against the soft guards (dormancy + pseudo-counts + hysteresis). *Contingency:* prefer the soft
  guards unless E1 shows easy-10 regressions that dormancy alone cannot fix.
- **H20 (reward family-robustness):** LBD-family rewards (R2/R3/R5) are *provably uninformative*
  on families where LBD degrades to clause length (arithmetic circuits/multipliers —
  arXiv:2602.20829). *Test:* E0 reports per-family correlation; any family with ~zero signal
  disqualifies a pure-LBD headline reward. *Contingency:* blend a non-LBD component (R6/R8 — note
  CoReward weights its variable-elimination term highest at 0.5), or adopt the usage-counter
  signal (+1 per BCP/analysis use, −1 per T conflicts) as the clause-quality input.
- **H21 (short-horizon index policy):** confidence-bound exploration at textbook strength is
  likely wrong at our pulls-per-episode: at h≈70 pulls ε-greedy/ε-decreasing beat UCB1/Thompson,
  UCB1 is hypersensitive to its constant, and the deployed SC2025 winner-lineage runs UCB with
  c=0.01. *Test:* the expanded E2 matrix (UCBT, ε-policies, c-sweep to 0.01, discounted-TS,
  adaptive-c). *Contingency:* if ε/UCBT win, the bandit is effectively "mostly-exploit with a
  trickle of exploration" — simplify the shipped controller accordingly.
- **H22 (clause-usefulness arm design):** if/when clause-DB arms (A4/A7) are pursued, the arm set
  should include the *rule-based usage-counter* policy (proven: +4 solved on SC22 main over
  kissat 4.0.2; 60/60 multipliers), not just LBD-tier variants — and NOT a learned per-clause
  predictor: ML-for-deletion has never beaten hand-crafted end-to-end (CrystalBall negative
  result; AITP 2020 preliminary-only; CaDiCaL 1.9.4 usefulness-proxy regression). The bandit
  picks *between* rule-based policies; it does not score clauses.
- **H23 (embedded in-run SAT/UNSAT classifier — the H6/H13/H16 deployment shape):** Kissat-Pred
  (SC2025) ships quantized offline-trained decision trees over linear-time graph features,
  predicts SAT/UNSAT + difficulty, adjusts only restart intervals and mode-switch periods, and
  falls back to defaults on UNKNOWN. That is: distilled trees (H13), per-instance prior (H6),
  SAT/UNSAT conditioning (H16), and a safe fallback — all in one deployed precedent. *Test:* if
  H6's classifier works, deploy it Kissat-Pred-style (trees, conservative surface set,
  UNKNOWN→default) before considering anything heavier.
- **H24 (atypicality bonus):** CoReward's folded `|x−μ|/σ` normalization rewards *atypical*
  periods in either direction — possibly an accidental exploration bonus that nonetheless ships
  in a competitive solver. *Test:* in E0/E2, compare signed vs folded normalization of the chosen
  reward; if folded wins, that is evidence the bandit benefits from novelty-seeking, not just
  reward-seeking (connects to count-based exploration bonuses). Default remains signed.

### Contingency decision tree

- **E0 finds no surrogate with usable within-instance correlation** → stop; do not build a bandit
  on a reward that cannot be validated. Pivot to H6 (per-instance config classifier — needs only
  episode-level labels) or H3 (MLR-style restart regression — needs only the LBD stream).
- **E1: UCB ≥ default and ≥ both controls** → proceed to Phase 2 as planned.
- **E1: random ≈ UCB > default** → H9 holds; ship stochastic alternation, skip contextual arm
  selection, and revisit context only for *gating* (when to alternate), not arm choice.
- **E1: all bandit variants ≤ default with clean overhead (E7)** → the action spaces are wrong,
  not the machinery: swap in A3/A4 (restart-margin / reduce-scale — the DynamicSAT-validated
  knobs) before abandoning. If those also fail, archive with findings; the M1 instrumentation and
  the E0 reward study remain assets for any future learned-control work.
- **E3/E4: context ≤ non-contextual online** → H1 null confirmed; keep the Phase-1 winner,
  document the negative result in the README, do not start Phase-3 NN work.
- **E4: OPE predicts a win, seedgate disagrees** → the i.i.d.-context assumption is broken
  (horizon effects): this is the specific trigger for H4 (fitted-Q), not a reason to retrain the
  same model harder.
- **Any stage: a win rides on 1-2 seed-fragile rows or fails E6's shuffle test** → lottery per
  CLAUDE.md; reject the config but keep its trajectories (still valid training data).
- **Easy-10 regress beyond noise at any gate** → raise cold-start dormancy (H10) and re-gate;
  if dormancy cannot fix it, escalate to the Conservative-UCB hard guard (H19) before any other
  tuning.
- **E0: the winning reward has a zero-signal family (H20)** → do not ship a pure-LBD reward;
  blend R6/R8 or the usage-counter signal and re-run E0 before starting bandit work.
- **E2: ε-greedy/UCBT beat UCB and Thompson (H21)** → our pulls-per-episode are below the
  confidence-bound regime; simplify to mostly-exploit + trickle exploration, and re-weigh H15
  (racing) which thrives in exactly this regime.

---

## 13. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Reward hacking — policy maximizes surrogate, not solving | E0 pre-validation; E1/E4 gates are on the *real* lexicographic metric, never the surrogate |
| Overfit to profile20 | profile20 never in training pool; E6 fresh-sample + shuffle validation; no per-instance guards (CLAUDE.md rule) |
| Nondeterminism breaking conflicts-tiebreak | All randomness via seeded LCG; determinism unit test; if ever violated, fall back to repeated PAR-2 runs and say so (CLAUDE.md) |
| Controller overhead / both-warm cost | E7 isolation experiment with a hard <1% ticks budget |
| Arm-switch state churn (heap/queue rebuilds each restart) | Reuse existing reorder machinery; measure rebuild cost in E7; if hot, rebuild lazily on first decision of the period |
| Trajectory collection compute (~60 core-h/config) | Pilot first (40×3); detached cron + hourly status; pool budgeted at 600 s timeout |
| Non-stationarity defeating the bandit | Discounted counts (E2 sweep), optional mode-switch resets, divergence trigger (H2) |
| Complexity creep in main.rs (14.8k lines) | All controller logic in `src/bandit.rs`; hooks are single-line calls; TDD on the module |
| Family-lottery wins masquerading as progress | Aggregate-only gates; per-family breakdown reported in every experiment summary |
| Arm-cost asymmetry (conflict-blind reward) | Δticks logged per period from M1; R7 in E0; tick-normalize if arms differ > 10% in ticks/conflict (H11) |
| OOD contexts on long runs (collection horizon 600 s < gate 900 s) | Feature clipping ±4σ + fallback to online-UCB/default arm (§5); clip rates logged and reviewed per eval |
| Bandit widens seed variance → underpowered N=10 gate | Per-instance conflict-variance comparison in E1; escalate deciding sweeps to N=20 (H18) |
| Easy-instance regression from exploration noise | Cold-start dormancy + default-arm pseudo-counts + dwell hysteresis (§10.4, H10); Conservative-UCB hard guard as escalation (H19) |
| Headline reward uninformative on LBD-degenerate families | E0 per-family correlation reporting; blend non-LBD components (R6/R8) or usage-counter signal (H20) |
| Wrong index policy for few-pull episodes (UCB over-explores or C mistuned) | E2 includes UCBT/ε-policies/c=0.01; deployed-winner priors favor near-greedy (H21) |

---

## 14. Milestones (suggested beads)

1. **M1 — Instrumentation:** `SAT_TRAJ_LOG` + period accumulators + R1-R7 logging (incl. Δticks
   per period) + schema v1; smoke + determinism tests. (No behavior change; small, safe first
   commit.)
2. **M2 — E0:** trajectory collection on training pool (pilot → full), surrogate validation
   notebook, pick the reward. Deliverable: `log/traj-e0-*/FINDINGS.md`.
3. **M3 — Phase 1 bandit:** `src/bandit.rs` UCB/Thompson + A1 (both-warm branching, per-mode arm
   sets) + A2 (rephase arms incl. `none`) + the non-learning control policies (seeded-random,
   round-robin — same arm machinery, trivial) + cold-start/hysteresis guards + flags + tests.
   E7 overhead check.
4. **M4 — E1/E2:** seedgate runs, verdict on online bandit. **Go/no-go for Phase 2.**
5. **M5 — Offline pipeline:** `tools/bandit/` (dataset, ridge/LinTS, OPE, export) + Rust weight
   loading + LinTS/MLP inference.
6. **M6 — E3/E4/E5:** contextual + offline-trained evaluations; action-space ablation.
7. **M7 — E6/E8:** generalization checks, promotion gate, README + FEATURES.csv documentation,
   archive trajectory manifests.

Each milestone ends with: smoke test green, `cargo test` green, results recorded in the solver
README per repo convention.

---

## 15. Key references

- Cherif, Habet, Terrioux — *Kissat_MAB: Combining VSIDS and CHB through UCB* — CP 2021 / SAT Comp 2021 winner.
- Liang, Ganesh, Poupart, Czarnecki — *CHB* (AAAI 2016), *LRB* (SAT 2016), *GLR study* (SAT 2017/IJCAI 2018).
- Liang, Oh, Mathew, Thomas, Li, Ganesh — *MLR: ML-Based Restart Policy* — SAT 2018.
- Nejati, Chowdhury, Ganesh — *MapleSSV* (UCB restart-policy selection, discounted) — SC 2021.
- Li et al. — *RL-based Reset Policy for CDCL* — arXiv:2404.03753 (2024). Thompson > UCB.
- Chen et al. — *Kissat-MAB-rephasing* — SAT Comp 2022 proceedings.
- Shi et al. — *DynamicSAT: Dynamic Configuration Tuning for SAT Solving* — CP 2025. github.com/cure-lab/DynamicSAT.
- Xie, Liu, Li — *MAB Algorithms for SAT: A Survey* — SoCS 2025. (Contextual bandits flagged as open.)
- Selsam, Bjørner — *NeuroCore* — SAT 2019. Han — *Glue-variable prediction* — arXiv:2007.02559.
- Wang et al. — *NeuroBack* — ICLR 2024 (one-shot CPU phase prior, +5.2% solved in kissat).
- Tönshoff, Grohe — *RLAF: one-shot GNN guidance trained by policy gradient on solver cost* — arXiv:2505.16053 (2025).
- Kurin et al. — *Graph-Q-SAT* — NeurIPS 2020 (per-decision GNN: iterations ↓, wall-clock ✗).
- Eriksson et al. — *Learning to Rank Initial Branching Order* — 2026 (init-only signal gets overwritten).
- Biedenkapp et al. — *DAC* — ECAI 2020; Adriaensen et al. — JAIR 2022.
- AE-Kissat-MAB — SAT Competition 2025 main-track winner (MAB lineage, 327/400).

Round-2 additions (raw verified payloads in `log/deepresearch-rl-bandit-2026-06-10-salvage.md`):

- Liu, Zhang, Sun — *Kissat_MAB-DC* — SC2024 proceedings (reward `log2(dec)/log2(conf)`); journal version 2025 (IEEE).
- Chen, Zhang, Liu, Sun, Li — *Kissat_MAB_CoRephase / Kissat_CoRephase_CoReward* — SC2025 proceedings pp. 13-14; code github.com/2317891476/Kissat_CoRephase_CoReward (reward verified against src/rephase.c).
- *Kissat-CURE*, *Kissat-Pred*, *AE_kissat2025_MAB* — SC2025 proceedings (TU Wien repositum, DOI 10.34726/10379).
- Oh — *Between SAT and UNSAT* — SAT 2015 (origin of SAT/UNSAT-mode; H16's premise).
- Garivier, Moulines — *D-UCB / SW-UCB* — ALT 2011 (γ, τ recipes; beats Exp3.S on abrupt change).
- Wu, Shariff, Lattimore, Szepesvári — *Conservative Bandits* — ICML 2016 (H19's guard).
- *Regression Oracles and Exploration Strategies for Short-Horizon MABs* — IEEE CoG 2020, arXiv:2102.05263 (H21).
- Hsieh et al. — *Discounted Thompson Sampling* — arXiv:2305.10718; MTSS — AISTATS 2022 (H14 priors).
- Li, Chu, Langford, Wang — *replay OPE* — WSDM 2011; Wang, Agarwal, Dudík — *SWITCH* — ICML 2017; Zhan, Hadad, Athey, Wager — KDD 2021 (adaptive-log OPE); Raghu et al. — ICML 2018 (propensity calibration); Kausik et al. — arXiv:2211.16583 + Bruns-Smith, Zhou — arXiv:2302.00662 (confounded-OPE impossibility/robustness).
- Soos — *CrystalBall* — 2019 (clause-usefulness labels; ML-lost-to-heuristic negative result).
- *Rethinking Clause Management for CDCL SAT Solvers* — arXiv:2602.20829 (2026; LBD→length degeneration, usage-counter policy; H20/H22).
- Biere et al. — CaDiCaL/Kissat SC2024 + SC2025 descriptions (tier percentiles, vivify ticks budgets, decision-rate-conditioned bumping caps); *CaDiCaL 2.0* — CAV 2024.
- Shavit, Hoos — *Revisiting SATzilla Features in 2024* — SAT 2024 (parse-time feature refresh for H6).
- Jamali — SFU PhD thesis 2021 (age/RU-counter vs activity deletion; centrality bumping).
