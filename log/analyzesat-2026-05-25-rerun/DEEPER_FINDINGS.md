# AnalyzeSAT Deeper Findings - 2026-05-25 Rerun

## Finding 1: `SAT_LUCKY` Needs an Adaptive Gate Before Default Promotion

Current code enables lucky by default:

- `solver/11-kissat-port/src/config.rs:625` sets `lucky: true`
- `solver/11-kissat-port/src/config.rs:738-742` keeps lucky on for default/fast
- `solver/11-kissat-port/src/main.rs:6704-6705` runs lucky after preprocessing
- `solver/11-kissat-port/src/main.rs:3585-3608` tries six polarity patterns plus
  bounded local repair

The implementation is correct enough to solve the intended battleship case, but
the profiling-suite evidence does not support default-on use yet:

- `SAT_LUCKY=on`: 10/10 solved, PAR-2 `799.250`
- `SAT_LUCKY=off`: 10/10 solved, PAR-2 `759.720`
- lucky attempts: `70`
- lucky solves: `1`
- net default-on cost: `+39.530s`

The current pass is all-or-nothing. A failed lucky run still performs seven
attempts per instance, including full live-clause satisfaction checks and, for
small formulas, local repair. On this suite, those failed attempts cost more
than the one saved search.

Recommended code change:

1. Keep the implementation, but do not run it unconditionally in default/fast.
2. Add an opportunity gate before `try_lucky_assignment` in `solve_with_proof`.
3. Gate on features that match the observed win shape, such as small SAT-leaning
   formulas with no eliminated variables, low original literal count, and a
   cheap pre-pass that predicts most clauses are satisfied by all-false or
   all-true.
4. Record skip reasons and hit-rate counters so future benchmark runs can tune
   the gate without guessing.

The immediate safe option is to demote default/fast `SAT_LUCKY` back to opt-in
until such a gate exists. The battleship win is real, but the default profile is
currently worse on the 10-instance profiling suite.

## Finding 2: Standalone `SAT_RESTART=kissat-ema` Still Misses Kissat's Execution Model

The current single-mode EMA path can be selected through
`effective_restart_policy`:

- `solver/11-kissat-port/src/main.rs:4100-4108`

But two key Kissat execution details are still missing from standalone EMA:

- focused restart interval growth is only updated in focused/stable mode
  (`main.rs:4366-4371`)
- trail reuse is disabled outside focused/stable (`main.rs:4856-4864`), so
  standalone EMA restarts backtrack to root through `perform_restart_if_pending`
  (`main.rs:4898-4929`)

Kissat's reference implementation does the opposite in focused mode:

- `benchmarks/reference-solvers/kissat-latest/src/restart.c:39-50` grows the
  focused restart limit after restarts
- `restart.c:69-83` computes focused trail reuse from link stamps
- `restart.c:86-110` applies restart trail reuse when enabled
- `restart.c:112-127` backtracks to the reused level and then refreshes the
  focused restart limit
- `options.h:124-127` enables restarts and restart trail reuse by default
- `options.h:180-187` uses a larger SAT restart interval in SAT builds

The data matches this code-level gap. On `mp1`, standalone EMA starts doing many
more shallow restarts immediately:

| Trace point | Default | EMA |
| --- | ---: | ---: |
| decisions at 20k conflicts | 88,888 | 246,624 |
| propagations at 20k conflicts | 12,883,322 | 10,213,716 |
| restarts at 20k conflicts | 68 | 274 |
| decisions/conflict at 20k | 4.444 | 12.331 |
| restarts/1000 conflicts at 20k | 3.400 | 13.700 |

Final `mp1` status:

- default: SAT in `46.758s`, `425,229` conflicts
- standalone EMA: UNKNOWN at `296.636s`, `2,743,691` conflicts

`case9` shows the same class of failure with lower magnitude:

- default: SAT in `127.501s`, `4,186,969` conflicts
- standalone EMA: UNKNOWN at `295.341s`, `8,733,291` conflicts

Recommended code change:

Do not promote standalone `SAT_RESTART=kissat-ema`. Either remove it from
promotion candidates or make it a thin alias for the focused/stable execution
model that includes the matching branch order, restart interval update, and
trail reuse behavior. Parameter changes are weaker than fixing this execution
model mismatch.

## Finding 3: LBD Metadata Is Not the Current Root Cause

`B_lbd_metadata` exactly matched `A_default` on aggregate work:

- conflicts: `7,583,967` in both configs
- decisions: `26,711,523` in both configs
- propagations: `3,187,270,047` in both configs
- restarts: `14,689` in both configs

The wall time differed (`764.328s` vs `799.250s`), but the identical work
counters mean LBD bookkeeping alone did not change the CDCL trajectory in this
rerun. The hard failure appears only when LBD is paired with standalone EMA
restart policy.

## Finding 4: No Phase-Boundary Chaos Evidence In This Rerun

The `mp1` trace diverges at the first 20k-conflict checkpoint. That makes this a
deterministic execution-model regression, not a late single-decision sensitivity
case. The next useful work is code-level restart integration, not broad knob
tuning.

## Bead Recommendations

Existing related beads:

- `SAT-playground-ide`: implemented default-on lucky assignment and is now
  closed; this rerun adds evidence that default-on needs a gate or demotion.
- `SAT-playground-5b2.2.58`: previous AnalyzeSAT restart/LBD finding; this
  rerun confirms the hard failure still exists at `a03645a`.

Create or update work items:

1. Create a new follow-up bead linked to `SAT-playground-ide`:
   `Gate or demote default-on SAT_LUCKY`.
2. Add a note to `SAT-playground-5b2.2.58` with the `mp1`/`case9` rerun
   evidence and reference Kissat comparison.

## Reproduction

From the branch root:

```bash
bash log/analyzesat-2026-05-25-rerun/run_ablation.sh
python3 log/analyzesat-2026-05-25-rerun/analysis.py
bash log/analyzesat-2026-05-25-rerun/run_reference_failures.sh
```

The ablation script intentionally exits when a candidate produces a
baseline-solved `UNKNOWN`.
