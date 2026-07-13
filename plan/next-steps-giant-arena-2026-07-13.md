# Next steps after the giant-arena promotion (2026-07-13 evening, 906e7cc)

Context for a fresh session. State as of this writing:

- Medium baseline: **64/100 @ 906e7cc**. Kissat 4.0.4 reference: 74/100
  (`log/kissat-medium-20260705-203444`). Gap ≈ 10.
- Promoted at `906e7cc`: **giant-arena parse + lean giant construction**
  (default-on; `SAT_GIANT_ARENA_PARSE=off` = legacy nested parse + full
  allocations). ee5 (11.normalised, 53.9M vars / 145.1M clauses) flipped
  UNKNOWN → SAT 227s in-gate: it was never search-hard (469 conflicts), just
  memory-unfit (25.0GB VmPeak vs the 16GB cap). The diet: parse directly into
  arena words (the `Vec<Vec<i32>>` parse peaked 8.6GB alone), skip
  occurs/n_occ/dirty/binary_dedup_seen/lbd_seen/binary_implications headers
  (~4.4GB of allocations giant-light never uses), exact-capacity watch lists,
  streaming model verify, and an honest giant-path preflight estimate.
  Post-diet ee5: VmPeak 14.36GB.
- Gate evidence: `log/abtest-cand-vs-base-2026-07-13-15-05-01` (PASS, WIN 64
  vs 63; ee5 is the ONLY divergent cell; both-solved conflicts differ by
  exactly ee5's own 469; PAR-2 148,509.9 vs 152,187.5). Launch log:
  `log/abtest-giantarena-launch.log`.

## Load-bearing facts

1. **The memory-fit campaign is now COMPLETE.** All four "normalised" giants
   solve: 18 (SAT 110s), 00fd8ac/2 (SAT ~80s, conf=209), 83aa/1 (SAT ~100s,
   conf=259), ee5/11 (SAT 227s, conf=469). No remaining medium cell is
   UNKNOWN for memory reasons. Every remaining gap cell is a
   search-capability problem.
2. The giants are propagation-heavy, conflict-trivial (200-500 conflicts,
   100-400M props). Their solves are dominated by parse + one long
   propagation fixpoint; the giant-arena parse also made 83aa/00fd8ac ~20%
   faster at half the RSS with byte-identical trajectories.
3. `binary_dedup_seen` is a dead field (allocated, resized, never read) — on
   ALL instances. Left allocated for non-giants to keep them byte-identical;
   a future cleanup could drop it everywhere (0.43GB on giants, pennies
   elsewhere).
4. Trap avoided (worth remembering): `Vec<Vec<T>>` per-literal structures
   cost 24B/header + ~16B malloc overhead per non-empty list. On 108M-slot
   literal-indexed structures that is ~2.6GB before any payload. The
   `BinaryImplications::Flat` variant exists but is unused; the watcher CSR
   rewrite (bead ck8 endgame) remains parked with its conflict-order-parity
   analysis in plan/next-steps-worklist-congruence-2026-07-12.md.

## Where the remaining ~10 cells are (from the 2a7 bead + prior notes)

All search-capability, in rough order of prior evidence:

1. **VexRiscv/oski/g2/goldcrest (BMC/miter cascade)** — VexRiscv solved
   standalone once (1372s, needs <~1000s for in-gate). Cheap armed-cascade
   levers exhausted; next mechanisms per the 07-13 note: transitive reduction
   of the binary implication graph, backbone pass, vivify tiers, and the
   congruence closure gate-pattern gap (kissat 183k merges vs our 19k on vex).
2. **Timetable492 / lockchart / bp4_TCO structured-SAT** — TT406-class
   trajectory kicks are forbidden (lucky-shuffle class); needs real mid-search
   BVE strength (kissat bound escalation) made honest.
3. **booth×2, Bubble, fixedbandwidth conflict-volume cells** — kissat needs
   6.5-14M conflicts at 11-29k conf/s; we are ~10x slower in conflict density
   on these. Chrono (delta=1000) closed part; the rest is inprocessing-driven
   formula collapse, same bundle as (1).

## Housekeeping

- The A/B arm syntax reminder: commas for multiple envs; empty arm spec is
  valid (`--arm 'base:'`).
- sqrt-mitern170 checker-timeout: still the benign symmetric verify artifact.
- rbsat-v1375 solved in BOTH arms this run (1265s/1244s) — the coin-flip cell
  landed heads twice; keep treating ±1 swings involving it as noise.
- Never `cargo build --release` while an ablation is live (this session
  rebuilt only before launch).

## Where the evidence lives

- Gate: `log/abtest-cand-vs-base-2026-07-13-15-05-01` + launch log
  `log/abtest-giantarena-launch.log`.
- Bead: `SAT-playground-2a7` comment dated 2026-07-13 (session 2).
- Standalone validation numbers (scratchpad, gone after reboot) are preserved
  in the 906e7cc commit message.
