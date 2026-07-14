# Next steps after the vivify-ALE promotion (2026-07-14, this commit)

Context for a fresh session. State as of this writing:

- Medium baseline: **67/100** (was 64 @ e5bd1f9), both-solved conflicts
  53,963,337, PAR-2 144,705.7. Kissat 4.0.4 reference: 74/100
  (`log/kissat-medium-20260705-203444`). Gap ≈ 7.
- Promoted (default-on):
  - **SAT_VIVIFY_ALE** — asymmetric literal elimination in `vivify_round`:
    strengthen a candidate even when the negated-prefix assumption walk ends
    WITHOUT a conflict, dropping the literals implied FALSE along the way
    (kissat vivify.c parity). Scoped in code to ARMED (`inprocess_aggressive`)
    formulas — unscoped ALE measurably rolled two non-armed solved SAT cells
    (sted2_0x1e3-216, 59-129706) into timeouts via the originals-schedule
    vivify rounds (first A/B, LOSE 65 vs 66,
    `log/abtest-cand-vs-base-2026-07-14-11-02-34`).
  - **SAT_VIVIFY_ARMED_TICKS=300000000** — armed-only per-round vivify budget
    replacing the permille clamp (cap was 100M). With ALE raising per-attempt
    yield, the cap was binding on the BMC cascade cells.
  - **Congruence worklist XOR-cancellation proof fix** (congruence.rs) — see
    "The correctness bug" below. Trajectory-neutral; REQUIRED for any promotion
    that makes oski-class cells solve.
- Gate evidence: `log/abtest-cand-vs-base-2026-07-14-14-02-51` (PASS, WIN
  **67 vs 64**, zero correctness failures; launch log
  `log/abtest-vivifyale2-launch.log`). Cand-only flips, no base-only cells:
  - **VexRiscv UNSAT 1720s** (first ever in-gate; kissat 232s) — flipped in
    BOTH A/Bs run today (1661s in the first).
  - **oski15a01b40s UNSAT 1380s, verify=ok** (first ever; kissat 543s).
  - rbsat-v1375 SAT 1739s (the known ±1 coin-flip cell; byte-identical 6.26M
    conflicts in every run, pure wall noise — do not attribute).
  - Divergent both-solved cells only 3: ibm −23k, bp4_CSO +22k, DLTM +558k
    (the DLTM armed roll is priced by the +3 tier-1 win).

## The mechanism (why ALE works here)

Vivify walk: assume ¬l for each candidate literal under propagation. Old code
only strengthened when the walk CONFLICTED (prefix-shrink). Literals implied
FALSE mid-walk (`FALSE => continue`) were tracked but never removed —
`vivify_removed_literals` was literally always 0. Kissat removes them (ALE).
The replayed `keep` is RUP: propagating ¬keep re-derives every dropped literal
false and the original (still present) clause supplies the conflict. No clause
is ever deleted (the redundant-delete path remains excluded — historically
unsound, div-mitern UNSAT→SAT).

Measured effect (standalone, idle):
- vex: 1696s/3.44M conf (base) → 1230s/2.83M (ALE); strengthenings 14k → 102k.
- oski40: TIMEOUT (never solved) → 1343s (ALE) → **1026s/2.51M @ 300M ticks**;
  strengthenings 237k; props/conflict 626 ≈ kissat's 580 (was 1860 pre-campaign).
- ibm canary: 370k → 368k (ALE) → 347k (ALE+300M). bp4_CSO proof VERIFIED.
- vex saturates at ~1230s regardless of budget (merges frozen 18.4k) — the
  remaining vex gap (1720s in-gate vs kissat 232s) is NOT vivify budget.

## The correctness bug (latent in e5bd1f9 and earlier)

oski40 solving in-gate exposed it: BOTH arms' UNSAT proofs were REJECTED by
drat-trim (`verify=FAIL`, RAT check failed on a merge equivalence binary,
`468056 340750 0`). Root cause: the worklist closure's XOR **cancellation**
(`a ⊕ a` removed during gate renormalization) dropped the cancelled variables
from the DRAT parity ladder. The gate's ORIGINAL clauses still range over the
cancelled vars, so the equivalence binaries need a case split on each — not
unit-derivable → non-RUP. Shipped with ce42829/e5bd1f9; oski never solved
before, so the proof stream was never checked end-to-end.

Fix (congruence.rs): accumulate cancelled vars per gate across renormalization
passes (`acc_cancelled`), carry them into every XOR merge chain (union of both
gates' histories + shared key inputs; table stores the rep's snapshot), emit
the ladder over ALL chain vars (was `l-1`), filter chain vars equal to
var(p)/var(q) (pinned by the RUP assumption; enumerating them creates
tautology holes), cap pathological unions (>12 → drop merge). Validation:
- 2 new unit tests (collapsed + keyed-union paths); 614 total pass.
- ibm byte-identical (369,887), oski40 byte-identical (3,556,063) — proof-only.
- oski40 full 7.5GB proof: **s VERIFIED in 528s** standalone
  (`oski-fixed-dratlong.log`, scratchpad), and verify=ok IN-GATE.

## Negative results this session (measured — do not re-run blind)

1. **SAT_ELIM_PRODUCTIVE_MIN_PCT=40 is dead under the bundle**: mp1 derail
   persists (27s → 586s, 336k → 5.4M conf, and the round machinery ran with
   armed vivify), AND TT406 no longer solves standalone (1751s TIMEOUT — the
   pre-bundle 728s "solve" was the lucky-shuffle trajectory, gone under the
   armed-collapse bundle). TT492 also TIMEOUT at pct40.
2. **Armed restart knobs are non-winners** (new default-off knobs kept as
   groundwork): SAT_RESTART_ARMED_FLOOR=1 → vex −27% conflicts (2.50M) but
   only 1530s; +REUSE_TRAIL_ARMED → TIMEOUT (worse); +MARGIN=1.10 → 1518s.
   Global SAT_RESTART_REUSE_TRAIL=on → vex −11% conflicts, wall noise.
3. **SAT_VIVIFY_SORT (kissat literal-count ordering) is a loser everywhere
   measured**: ibm 368k→625k conf, bp4 slightly worse, vex 1230→1530s,
   oski40 1343s→TIMEOUT. Knob kept default-off. (Kissat pairs sorting with
   candidate sorting + prefix trail reuse; in isolation it just reorders the
   walk against arena order.)
4. vex budget saturation: SAT_VIVIFY_ARMED_TICKS 300M does NOT help vex
   further (1227s vs 1230s; strengthenings 2x but merges frozen).

## Ranked next steps

### 1. vex wall margin (1720s in-gate is 80s from the wire)
vex flipped twice today but at 1661s/1720s — any suite-wide slowdown unfips
it. Cheapest insurance: proof-IO cost (vex writes ~8GB DRAT in-gate; binary
DRAT was measured pointless on div-mitern but never on vex where the proof is
30x bigger). Also the congruence-merge stall (18.4k frozen while kissat
reaches 183k): the closure finds no NEW gate patterns after round ~3 — kissat
keeps discovering because vivify/BVE create NEW ternaries that hash as gates;
check whether our gate extraction sees post-ALE strengthened clauses.

### 2. oski20 (kissat 617s; ours TIMEOUT both arms, 3.68M conf @1751s w/ sort)
Same family as oski40 (which now solves 1380s). oski20-ale (no sort) was
never screened standalone — screen it; if ~1400s it may flip with any
suite-wide margin gain.

### 3. Conflict-density transfer to booth/Bubble/fixedbandwidth
These have 0 congruence merges → the whole armed bundle is inert there. The
ALE mechanism itself is not congruence-dependent — a different arming signal
(e.g., vivify-yield: keep ALE+budget active while strengthenings/round > X)
could extend it without touching fragile cells. Needs a dry-run-style signal
measured on those cells first.

### 4. Housekeeping / traps (additions)
- `pkill -f <pattern>` matches YOUR OWN shell if the pattern appears in the
  command line — use `pkill -f "pat[t]ern"` (exit 144 = self-SIGTERM).
- drat-trim on 7.5GB proofs needs ~530s idle, >1750s under load → in-gate
  `checker-timeout` on vex/sqrt-mitern170 is the benign class; `FAIL` is real.
- feature_ablation's final verification phase runs AFTER all solver cells
  (0 sat-solver processes but drat-trim alive) — don't declare a run stalled.
- `inprocess_rounds` in JSON_STATS is hardcoded 0 (stats.rs:742) — still unwired.
- The A/B preflight warns about the agent's own monitor shells (command lines
  contain "sat-solver") — cosmetic, but kill stray monitors before launching.

## Where the evidence lives
- Winning gate: `log/abtest-cand-vs-base-2026-07-14-14-02-51` + launch log
  `log/abtest-vivifyale2-launch.log` (gate PASS output in session log).
- Rejected first shape: `log/abtest-cand-vs-base-2026-07-14-11-02-34` (LOSE
  65 vs 66, unscoped ALE) + `log/abtest-vivifyale-launch.log` — including the
  symmetric oski40 verify=FAIL that exposed the proof bug.
- Proof-bug repro + fix validation: scratchpad `oskiproof/` (base config,
  NOT VERIFIED at line 4,924,124), `oski-fixed/` (fixed, byte-identical
  trajectory), `oski-fixed-dratlong.log` (s VERIFIED 528s). Scratchpad dies on
  reboot; the numbers are in this note and the commit message.
