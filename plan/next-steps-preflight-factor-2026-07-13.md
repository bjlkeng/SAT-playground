# Next steps after the preflight promotion + factor groundwork (2026-07-13, 15911aa)

Context for a fresh session. State as of this writing:

- Medium baseline: **63/100 @ 15911aa** (rbsat-v1375 remains the ±1 coin-flip cell;
  it timed out in BOTH arms of this session's gate). Kissat 4.0.4 reference: 74/100
  (`log/kissat-medium-20260705-203444`). Gap ≈ 11.
- Promoted at `15911aa`: **simp-aware memory preflight** (default-on;
  `SAT_PREFLIGHT_SIMP_AWARE=off` = byte-exact legacy estimator). 83aa
  (29.3M vars / 78.8M clauses) flipped UNKNOWN → SAT 100s in-gate: the old
  estimator charged the occurrence entries + inline-abstraction migration
  transient that the giant-light profile never allocates, and priced the
  migration reloc map at usize (stale since the 02e5d00 u32 diet). Estimated
  14,732MB vs threshold 14,400MB; TRUE peak 12.7GB VmSize / 9.8GB RSS.
- Gate evidence: `log/abtest-cand-vs-base-2026-07-13-10-47-36` (PASS, WIN
  solved tier 63 vs 62, PAR-2 152,271.8 vs 156,109.0, zero trajectory diffs on
  shared solved cells — the candidate only changes whether 83aa runs).
- Also landed (default-off, inert): `SAT_FACTOR_INPROCESS` (mid-search BVA with
  fresh-var growth — `grow_variables()` + `VmtfQueue::grow`),
  `SAT_INPROCESS_ROUNDS` (armed proberounds loop), `SAT_ELIM_ARMED_EFFORT_PCT`.

## Load-bearing discoveries

1. **vmtf FocusedOnly is ON in the default profile** (`apply_focused_stable_defaults`).
   Any `vmtf_queue.is_some()` guard silently disables a feature EVERYWHERE. The
   factor knob produced literally zero effect across three screen rounds until
   the queue got a `grow()` method. Audit any future fresh-var or var-indexed
   feature for this trap.
2. **Mid-search factor must use the armed eliminate bound (starts 0), not the
   frontend's mature bound 16** — kissat factors on ANY positive clause
   reduction mid-search. With bound 16 it never fires on the gap cells.
3. **drat-trim proof-line numbers are offset by the CNF clause count** —
   "proof line 389730" on a 389,661-clause formula is proof.out line 69.
   Deletion warnings ("deleted clause does not occur") on armed-bundle proofs
   are pre-existing (reproduce with factor off) and benign (s VERIFIED).
4. **The 16GB "OOM giants" must be re-audited against the preflight, not
   assumed infeasible**: 83aa was solvable all along. ee5 (54M vars, est
   19.9GB) is genuinely over — its +1 needs the arena/watcher architectural
   diet (Vec-of-Vec watcher headers alone ≈ 2.6GB there).

## Negative results this session (measured — do not re-run blind)

1. **Factor on VexRiscv is a LOSS**: 1613s standalone with factor vs ~1240s
   without (same load), despite 6.9k fresh vars / 55k product clauses removed
   in round 1. Clause compression does not convert on the BMC cell.
2. **oski20/40 still TIMEOUT with the full working bundle** (worklist +
   armed-BVE + min-merges 32 + factor): 80k+ product clauses factored, 7
   armed rounds, no solve. The eliminate→congruence→factor cascade alone is
   not the missing piece for oski.
3. **SAT_INPROCESS_ROUNDS=2** (armed proberounds): VexRiscv UNKNOWN at 1702s
   vs control UNSAT 1462s in the same wave — extra pass cost, no payoff.
4. **SAT_ELIM_ARMED_EFFORT_PCT=20**: VexRiscv 1679s vs control 1462s. Worse.
5. **Armed min-merges 8 vs 32**: 1496s vs 1462s — noise, keep 32.
6. **Kissat-scale sweep budgets (depth 3 / 8192 vars / 32768 clauses) are
   PATHOLOGICAL on 400k+-var formulas**: a single armed round runs for HOURS
   (SAT_LIMIT_WALL_SEC is only checked between conflicts, never inside an
   inprocess round), and the config doubled div-mitern172's wall (300s vs
   150s). Our per-seed sweep architecture cannot absorb kissat's budgets; it
   would need per-round tick budgeting first.
7. **VexRiscv standalone times are load-sensitive ±20%**: 1240s (2 concurrent)
   vs 1462s (8 concurrent) for the SAME config. Never compare screens across
   different host loads; pair configs within one wave.

## Ranked next steps

### 1. ee5 memory architecture diet (the next pure-fit +1, kissat solves it in 137s)
True need ≈ 23GB vs 16GB cap. Big pieces: flat CSR watcher layout (kills the
2×54M Vec headers ≈ 2.6GB + allocator slack), u8 phase/bool packing, and the
occurrence-index-free giant path. This is the "big architectural change"
lever; 83aa proved the payoff class is real.

### 2. VexRiscv/oski: the cascade is stalled — different mechanism needed
All cheap armed-cascade levers are exhausted. What kissat still has that we
don't, in likely-impact order for these cells: transitive reduction of the
binary implication graph (every probe round), the backbone pass
(kissat_binary_clauses_backbone, runs twice per round), and vivify tiers with
per-tier budgets. Consider also that kissat's congruence closure reaches 183k
merges on vex vs our 19k — the worklist closure may still be missing gate
patterns (e.g. definitions through XOR chains) rather than budget.

### 3. Factor: keep default-off; possible salvage angles
It provably works and its proofs verify. Salvage candidates: fire only in
round 1 (the big collapse) and not later rounds; or gate on SAT-looking
formulas (MVRoundRobin-class) rather than BMC. Needs a target cell where it
converts before another A/B is worth it.

### 4. Housekeeping / traps (additions to the standing list)
- SAT_LIMIT_WALL_SEC is not honored inside a long inprocess round — screens
  MUST wrap with external `timeout` (screen_run.sh now does).
- The A/B launch log's per-cell lines show identical conflict counts for
  trajectory-identical arms — a cheap sanity check that a "should be inert"
  candidate really is.
- 83aa now occupies a core for ~100s in-gate; wall-clock-sensitive cells
  (CONGRUENCE_ITER_MAX_SECONDS) could in principle notice the scheduling
  change; this gate showed zero conflict diffs, so it did not.

## Where the evidence lives
- Gate: `log/abtest-cand-vs-base-2026-07-13-10-47-36` + launch log
  `log/abtest-preflight-simp-aware-launch.log`.
- Bead: `SAT-playground-2a7` comment dated 2026-07-13 (full numbers).
- Screens (scratchpad, gone after reboot): vex-c0..c9, oski*-c7/c8/c10,
  giant-83aa-{probe,true,capped-fixed}, proofcheck-* — key numbers preserved
  in the bead comment and the 15911aa commit message.
