# Deep dive: chrono / binary_fast / kissat-ema / target_phase on the fstab_lbdtier default

Follow-up to sweep-2 (user-directed): did these four features actually *trigger*, what's the
mechanism behind their sweep-2 verdicts, and is any regression a **code-level inefficiency**
(suspected: the binary fast path) rather than a search effect?

Method: re-ran each config with `SAT_STATS_JSON=on` capturing trigger + work counters
(conflicts, propagations, decisions, restarts-by-type, chrono_attempts/used/rejected, phase_*_used,
binary_props/stale_skips, mode_switches) on representative profile20 instances, plus static reads
of the relevant code paths. Data: `probe.tsv` (runner `probe.sh`). Instances: `case9` (the sweep's
universal canary), `6s299b685_Iter30` (long SAT search — gives a clean work×speed split),
`battleship` (lucky-solved in 0.09 s → uninformative, all configs identical).

## TL;DR per feature

| feature | triggered? | mechanism | verdict |
|---|---|---|---|
| **kissat-ema** | **NO — dead env var** | `effective_restart_policy()` hardcodes ema in focused / reluctant in stable under FocusedStable; `SAT_RESTART` is ignored | byte-identical to default; "no-op" is literal |
| **chrono** | bimodal | fires only when backjump `delta > chrono_max_delta=100`; never on case9 (rejected 1.24M/1.24M), fires on 6s299b685 and is **catastrophic** (60× conflicts) | inert or harmful; never helps here |
| **binary_fast** | YES | **3× faster per-propagation** but drives a divergent, ~4× longer search trajectory | regress = trajectory change, **NOT a code inefficiency** |
| **target_phase** | instance-dependent | inert on short searches; on case9 genuinely engages (976k target decisions) and makes it **+25% harder** | real (mild) search degradation where it engages |

## Detail

### kissat-ema (and reluctant) — never applied
`src/main.rs:4677 effective_restart_policy()`:
```
if search_mode_policy == FocusedStable { Focused => KissatEma, Stable => Reluctant } else { restart_policy }
```
The promoted default **is** FocusedStable, so `self.restart_policy` (the `SAT_RESTART` env) is
**dead code** — focused mode already uses ema, stable already uses reluctant. Probe confirms: on
6s299b685, ema vs default differ in **zero** counters (byte-identical: 5097 conflicts, 27.27 M props,
identical restart breakdown). So sweep-2's "ema neutral / reluctant neutral / every ema-combo ≈ its
partner" was not "the restart didn't help" — **the flag was silently ignored.** (`SAT_RESTART` is
only live under single-mode search.)

### chrono — gated out on most instances, harmful when it fires
`src/main.rs:5326`: chrono backtrack is taken only when `delta = (current_level-1) - assertion_level
> chrono_max_delta` (default **100**) AND the learned clause still asserts at the chrono level.
- **case9:** `chrono_attempts=1,237,781`, `chrono_used=0`, `chrono_rejected_delta_small=1,237,781` —
  every backjump was ≤100 levels, so chrono was rejected every time → trace **identical** to default
  (same 1.24 M conflicts, same time). The sweep's +55 PAR-2 was noise on the other instances.
- **6s299b685:** chrono fires and is **catastrophic**: conflicts 5,097 → **310,424** (≈60×), time
  93.5 s → 129.7 s. Keeping deep partial assignments after a conflict sends search into a vastly
  longer trajectory on this instance.
Net: chrono is bimodal — invisible where backjumps are short, damaging where they're long. It never
helps on profile20.

### binary_fast — the code-inefficiency hypothesis is REFUTED
Suspicion was double-propagation or fast-path overhead. Code + data both say no:
- **No double work.** `attach_clause` (`src/main.rs:3621`) routes len-2 clauses to the
  `binary_implications` edge structure **only**, NOT the watch lists (`2 if binary_fast_path =>
  register_binary_clause`). `register_binary_clause` / `try_binary_id_for_clause` are O(1) (map
  lookup `binary_id_by_clause`), not scans. Binaries are propagated once, via edges.
- **The fast path is genuinely faster.** 6s299b685 work×speed split (binary_fast vs default):
  - propagation throughput **0.9 vs 0.3 Mprop/s ≈ 3× faster per op** ✓ (the optimization works)
  - but conflicts **2.54×**, propagations **3.95×**, decisions **8.6×** → it explores a *different,
    much longer* search path. Net time +21% despite the 3× speedup.
- On **case9** the divergent trajectory simply doesn't terminate within budget: default 68 s →
  binary_fast **TIMEOUT** (>320 s). That single flip is most of its +2614 sweep PAR-2.
Conclusion: binary_fast's regression is a **search-trajectory divergence**, not an implementation
inefficiency. Reordering which binary implications are discovered first changes conflict analysis and
phase saving, which compounds into a longer search. There is no hot-path waste to fix.

### target_phase — engages only on long searches, then mildly hurts
- **6s299b685:** zero differing counters vs default — the instance is solved before target-phase
  materially engages (it's a short, easy SAT search relative to its time).
- **case9:** genuinely engages — `phase_target_used=975,926`, `phase_saved_used` collapses
  838k → 7.9k (target phase displaces saved phase). Result: conflicts 1.24 M → 1.55 M (**+25%**),
  time 68 s → 91 s (**+34%**). Assigning toward the best partial assignment is a real, different
  search policy here — and on this suite it costs more than it saves (it *swapped* Pancake↔REGRandom
  in the sweep at fixed total). Not overhead; a genuine (unhelpful-here) policy change.

## Implications / recommendations

1. **`SAT_RESTART` is misleading on the default.** It silently no-ops under FocusedStable. Either
   (a) document that ema/reluctant are already the focused/stable defaults and `SAT_RESTART` only
   affects single-mode, or (b) make `effective_restart_policy` honor an explicit `SAT_RESTART`
   override even in focused-stable so the knob is testable. This is the one genuine *defect-class*
   finding — not a perf bug, but a dead-knob that wasted 4 sweep configs (ema, reluctant, and the
   ema_* combos all measured "no change" for this reason).
2. **No code-level fix is warranted for binary_fast / chrono / target_phase.** Their regressions are
   real search-policy effects on a default that's already well-tuned for this suite — not
   inefficiencies. binary_fast in particular is *faster per op*; "fixing" it would mean changing
   search policy, not code.
3. The sweep-2 conclusion stands: the `fstab_lbdtier` default is a robust local optimum; these four
   features don't improve it (and two of the four never even ran as intended).

Provenance: `probe.tsv`, `probe.sh`. Code anchors: `effective_restart_policy` main.rs:4677;
chrono decision main.rs:5326; `attach_clause` binary arm main.rs:3621; binary edge propagation
`propagate_binary_implications` main.rs:4259.

---

## UPDATE (2026-06-01): seed-distribution sweep — corrects the single-run claims above

The analysis above (and sweep-2) rested on **n=1 seed**, which cannot distinguish "systematically
worse" from "unlucky draw." Re-ran default vs each feature across **n=5 seeds × 13 default-solved
instances** at 300s (`SAT_SEED` varies internal random decisions; conflicts are contention-immune).
11/13 instances yielded data (sudoku + REGRandom timed out under all configs/seeds — censored
equally, no bias). Primary metric: per-instance **P(feature>default)** conflict stochastic dominance
(0.50 = lottery) + aggregate PAR-2/seed with the default's own seed-spread as the noise floor.
Data: `seedsweep_results.tsv`, `seedsweep_analysis.txt`. Driver `seedsweep.py`, analyzer
`seedsweep_analyze.py`.

**Default's own seed-spread: PAR-2/seed mean 2500 ± 186** (range 2154–2706). That ±186 is the bar
any feature delta must clear.

| feature | Δ PAR-2 (vs ±186) | P(f>d) mean | better:worse | verdict |
|---|---|---|---|---|
| **chrono** | **+56** (noise) | 0.54 | 1:1 (9/11 byte-identical) | **genuinely neutral**: inert on ~all instances (backjumps rarely exceed delta=100), one explosion (6s299b685 5.7× conflicts) the suite dilutes |
| **target_phase** | +248 (~1.3σ) | **0.44** (<0.5!) | **5:3** | **roughly neutral / mildly net-negative** — helps more instances than it hurts on conflicts (Kakuro 0.19×, SCPC/sqrt171/mp1 better); only case9's 2.53× blowup drags the aggregate positive. Earlier "real degradation" framing was too harsh. |
| **binary_fast** | **+570 (~3σ, real)** | **0.64** | 4:6 | **systematically worse search distribution** — confirmed (matches the n=8 single-instance P=0.62). Shifted up + **3× variance** (stdev 597 vs 186). case9 worse on all 5 seeds (P=1.00). NOT random, NOT uniformly bad (helps 4/11: Kakuro 0.39×, circuit 0.82×). |

### Corrections to the record
- **binary_fast is real-but-modest worse search, not "random bad luck"** (P=0.64, case9 P=1.00) — this
  answers the user's challenge: per-op speed is irrelevant; the *search distribution* is genuinely
  shifted up. BUT the sweep-2 "+2614 PAR-2" was inflated ~4× by case9 tipping into a hard 600s
  timeout; the intrinsic 300s search penalty is **+570**. Both "it's fine, just unlucky" and
  "+2614 systematic" were wrong; truth is "+570, ~3σ, directional, variance-amplified by the metric."
- **target_phase was over-condemned.** P=0.44 (<0.5) means it improves the *median* instance's
  conflicts; it's only marginally net-negative in aggregate and entirely due to case9. Not a clean
  regressor.
- **chrono confirmed neutral** with the precise mechanism: bimodal (inert | rare explosion), net within noise.
- **case9 is the universal pathology** — every feature's worst case. It is the fragile instance that
  any trajectory perturbation breaks, and it dominates aggregate verdicts on this suite. A
  case9-free suite would flip target_phase net-positive and roughly halve binary_fast's penalty.
