# Session notes: congruence indexing diet promotion — 68 → 69 solved (2026-07-19)

Continuation of the 2026-07-18 flywheel/decomposition session (see
`plan/next-steps-flywheel-decomposition-2026-07-18.md`). State at end:
medium baseline **69/100** @ this commit (rbsat-v1375 now IN). Kissat
reference: 74/100. Gap ≈ 5.

## PROMOTED: SAT_CONGRUENCE_FASTIDX (default ON)

Gate `log/abtest-cand-vs-base-2026-07-18-22-33-59` (launch
`log/abtest-fastidx-launch.log`): **PASS, WIN — solved 69 vs 68**
(+rbsat-v1375c111739g SAT 1750.9s verify=ok, base TIMEOUT at 1800.0s),
both-solved conflicts EXACT tie (zero mismatching cells — trajectory
identity at 100-cell scale), PAR-2 139,085.0 vs 140,949.4 (−1,864), zero
contradictions, zero correctness failures. `check_promotion_gate` PASS
(after killing stale prior-session watcher shells — the documented
running_solver_processes trap, hit again).

### The mechanism

Wall-decomposition checkpoints (new env-gated `SAT_TRACE_TIMING=1`) showed
ROOT gate-congruence closure invisible to every existing timer
(`preprocess_sec` starts AFTER it): **ibm 45.7s of a 147s solve (31%)**;
vex 12s, oski40 8.4s, oski20 ~8s, bp4 0.5s. Per closure round (ibm, 8
rounds): extract_gates 2.34s + closure 0.66s + apply ~1.4s — with rounds
2-7 re-deriving ~871k gates to find <100 new merges each.

The diet (all trajectory-identical, verified by per-round merge counts and
full-run byte-equal conflicts):
- **FxHash** (`src/fxhash.rs`, hand-inlined rustc-hash algorithm — no new
  crate, Cargo.toml untouched) for the extraction membership sets and the
  closure matching tables (1.3M `(kind, inputs)` Vec-keyed hashes/round).
  Iteration-order safety: outcomes were already byte-reproducible across
  std's randomly-seeded SipHash processes, so a fixed-seed hasher varies
  strictly within the tolerated envelope; the one iterated map (XOR
  `families`) demonstrably does not leak order into results.
- **Flat clause pool** in extraction (one literal arena + end offsets
  instead of per-clause `Vec` allocs + a full `Vec<Vec>` copy).
- **Capacity reservations** on the never-iterated closure table and gate vec.
- `SAT_CONGRUENCE_FASTIDX=off` = the pre-diet implementation VERBATIM
  (legacy fns restored from git) as the A/B baseline arm. The off-arm keeps
  Fx in the closure tables (proven trajectory-neutral), so the A/B slightly
  UNDERSTATES the candidate edge — conservative.

Measured idle: ibm 152.1s (off) → 137.5s (on), −9.6%, conflicts/merges
byte-equal (346,627 / 145,049). Suite: the win concentrates on
congruence-armed cells (ibm −52s-class, oski40 −23s, TT492 −52s in-gate,
sted2 −31s) — and on the wall-lottery margin cells, which is where the
rbsat flip came from (identical trajectory, 49s under the wire vs base 0.05s
over).

### Honest caveats

- rbsat-v1375 is a documented wall-lottery cell; the +1 is a wall-margin
  flip (trajectory identical, base missed by 0.05s). The PAR-2 and
  conflicts-tier evidence is the durable part; rbsat may wobble in future
  gates — but every future gate now inherits ~15s more margin on it.
- The conflicts tier is an EXACT tie by construction; nothing about search
  changed.

## Also in this commit (from the same two-session arc)

- `SAT_TRACE_TIMING=1` wall checkpoints (parse/frontend/Solver::new/
  solve.root_propagate/pair_abs_gauss_els/congruence_root/search_start/
  model steps) — the instrument that found the 31%. Keep.
- `SAT_DEBUG_CONGRUENCE=1` now also prints per-step round timings
  (extract_binaries/els/extract_gates/closure).
- `SAT_ELIM_UNARMED_FLYWHEEL` groundwork (default OFF, prior commit
  4bf2de4) — unchanged.

## Ranked next steps

1. **Deeper congruence diet** — the identity-safe ceiling here is another
   ~20-30s suite-wide (closure key clones via prehash buckets, apply-path
   allocs). The BIG version (incremental per-clause gate cache) is blocked
   on extraction's dependence on incidental clause-literal order (propagation
   swaps mutate it, including via PTR_FAST unsafe writes) — exact identity
   would need hot-loop bookkeeping. A canonicalized (sorted-lit) extraction
   would enable caching but is a one-time full-suite reroll: consider it
   only WITH the cache in the same gate so the reroll buys the wall.
2. **vex root eliminate = 28.3s** (13.3→41.6 in the checkpoint run) — the
   next biggest single non-search chunk measured; same diet playbook
   (decompose with SAT_TRACE_TIMING + targeted counters first).
3. **Flywheel ensemble port** (see 2026-07-18 note) — unchanged.
4. **TT-class stabilizer**, **pj2008/goldcrest class measurement** —
   unchanged from the 2026-07-18 note.

## Where the evidence lives

- Winning gate: `log/abtest-cand-vs-base-2026-07-18-22-33-59` + launch log
  `log/abtest-fastidx-launch.log`; formal check output in the commit message.
- Idle screens/decompositions: scratchpad (dies on reboot) — all
  decision-relevant numbers are in this note and the 2026-07-18 note.
- Bead: `SAT-playground-2a7`.
